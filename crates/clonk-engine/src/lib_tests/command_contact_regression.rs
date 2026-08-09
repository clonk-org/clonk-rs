use super::*;
use crate::landscape::PixelGrid;
use std::collections::HashMap;

#[test]
fn pixel_less_landscape_does_not_invent_column_surface_contact() {
    // C4Object::ContactCheck samples the current shape against landscape
    // pixels (C4Movement.cpp:165-181); C4Object::DoMovement consumes that
    // contact result (C4Movement.cpp:231). The C++ oracle has no
    // per-column surface snap for a pixel-less landscape.
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(20, 5));

    let mut definition =
        Definition::from_script("FALL", "Falling fixture", "").expect("definition compiles");
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("FALL").with_position(Vector2::new(3, 8)))
        .expect("object spawns");

    engine
        .apply_object_update(
            object,
            ObjectUpdate::new()
                .with_position(Vector2::new(3, 8))
                .with_velocity(Vector2::new(0, 3)),
        )
        .expect("object update applies");

    let index = engine.find_object_index(object).expect("object exists");
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
    engine
        .register_script_definition("SYNC", "Sync", "")
        .expect("definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("SYNC").with_position(Vector2::new(10, 20)))
        .expect("object spawns");
    let index = engine.find_object_index(object).expect("object exists");
    let fractional = crate::math::C4Fixed::from_raw(itofix(10).val().wrapping_add(1));
    engine.objects[index].fixed_position.x = fractional;

    engine
        .execute_synchronize_control(false, false)
        .expect("synchronization succeeds");
    assert_eq!(engine.objects[index].fixed_position.x, fractional);

    engine
        .execute_synchronize_control(false, true)
        .expect("synchronization with clearance succeeds");
    assert_eq!(engine.objects[index].fixed_position.x, itofix(10));
}

#[test]
fn synchronize_control_checkpoints_live_player_and_crew_time_but_not_replays() {
    let mut engine = Engine::new();
    engine.game_time = 10;
    engine
        .register_player(
            PlayerConfig::new(3, "Local")
                .with_player_info_id(17)
                .with_total_playing_time(40),
        )
        .expect("player registers");
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
    engine
        .register_player(
            PlayerConfig::new(4, "Script")
                .with_player_info_id(18)
                .with_total_playing_time(70),
        )
        .expect("script player registers");
    engine
        .player_mut(4)
        .expect("script player exists")
        .set_script_player(true);
    engine
        .register_player(
            PlayerConfig::new(5, "Eliminated")
                .with_player_info_id(19)
                .with_total_playing_time(80)
                .with_status(PlayerStatus::Eliminated),
        )
        .expect("eliminated player registers");
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

    engine
        .execute_synchronize_control(true, false)
        .expect("player-file synchronization succeeds");
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
    engine
        .execute_synchronize_control(true, false)
        .expect("replay synchronization succeeds");
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

    let mut definition = Definition::from_script("TILT", "Tilt", "").expect("definition compiles");
    definition.set_rotateable(1);
    definition.set_shape_vertices(vec![ObjectVertex {
        x: 0,
        y: 1,
        cnat: CNAT_BOTTOM,
        friction: 100,
    }]);
    engine
        .register_definition(definition)
        .expect("definition registers");

    let object_id = engine
        .spawn_object(
            SpawnConfig::new("TILT")
                .with_position(Vector2::new(50, 50))
                .with_rotation(5),
        )
        .expect("object spawns");
    let index = engine.find_object_index(object_id).expect("object exists");
    engine.objects[index].frame_t_contact = CNAT_LEFT;

    engine
        .stabilize_object(index, &[])
        .expect("stabilize callback succeeds");

    assert_eq!(engine.objects[index].state.rotation, 0);
    assert_eq!(engine.objects[index].frame_t_contact, CNAT_NONE);
}

#[test]
fn command_snapshot_keeps_definition_command_policies() {
    // Commands read these policies from cObj->Def at execution, so the
    // Rust frame snapshot must preserve the raw engine definition values
    // rather than infer them from crew OCF.
    let mut engine = Engine::with_seed(0);
    let mut definition =
        Definition::from_script("ROUT", "Router", "").expect("definition compiles");
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
    engine
        .register_definition(definition)
        .expect("definition registers");
    let object_id = engine
        .spawn_object(SpawnConfig::new("ROUT").with_action(ActionState::new("Route")))
        .expect("object spawns");
    let index = engine.find_object_index(object_id).expect("object exists");

    let snapshot = engine.live_command_snapshot(index);

    assert_eq!(snapshot.pathfinder, -4);
    assert_eq!(snapshot.no_transfer_zones, -3);
    assert_eq!(snapshot.no_push_enter, -2);
    assert!(snapshot.action_disabled);
    assert_eq!(snapshot.ocf & ocf::CREW_MEMBER, 0);
}

#[test]
fn lazy_host_world_call_object_materializes_only_on_world_access() {
    let mut engine = Engine::with_seed(0);
    let mut landscape =
        Landscape::with_default_material(100, vec![100; 100], None).expect("query landscape");
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

    engine
        .register_script_definition("FILL", "Filler", "#strict\n")
        .expect("filler registers");
    let mut caller = Definition::from_script(
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
    )
    .expect("caller compiles");
    caller.set_c4_callback_convention(true);
    engine
        .register_definition(caller)
        .expect("caller registers");
    for x in 0..64 {
        engine
            .spawn_object(SpawnConfig::new("FILL").with_position(Vector2::new(x % 100, 10)))
            .expect("filler spawns");
    }
    let caller = engine
        .spawn_object(SpawnConfig::new("LAZY").with_position(Vector2::new(50, 50)))
        .expect("caller spawns");
    let caller_index = engine.find_object_index(caller).expect("caller exists");

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
        engine.objects.len(),
        "enumeration fills one complete object view exactly once"
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
    let mover = Definition::from_script(
        "MOVE",
        "Unmasked mover",
        r#"#strict
func Move() { SetPosition(20, 30); return(0); }
"#,
    )
    .expect("mover compiles");
    engine.register_definition(mover).expect("mover registers");
    let mover = engine
        .spawn_object(SpawnConfig::new("MOVE").with_position(Vector2::new(10, 10)))
        .expect("mover spawns");
    let mover_index = engine.find_object_index(mover).expect("mover exists");

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
    engine
        .register_script_definition("ORDR", "Order", "")
        .expect("definition registers");
    let ids = (0..3)
        .map(|_| {
            engine
                .spawn_object(SpawnConfig::new("ORDR"))
                .expect("object spawns")
        })
        .collect::<Vec<_>>();
    for id in &ids {
        assert!(engine.find_object_index(*id).is_some());
    }

    engine.objects.swap(1, 2);
    let inactive = ids[1];
    engine
        .objects
        .iter_mut()
        .find(|object| object.id == inactive)
        .expect("inactive object remains stored")
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
fn lazy_host_world_action_callback_seeds_only_caller() {
    let mut engine = Engine::with_seed(0);
    engine
        .register_script_definition("FILL", "Filler", "#strict\n")
        .expect("filler registers");
    let mut actor = Definition::from_script(
        "ACTR",
        "Action caller",
        "#strict\nlocal phase_calls; protected func OnPhase() { phase_calls++; return(0); }",
    )
    .expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        Some("Swim".to_owned()),
        HashMap::from([(
            "Swim".to_owned(),
            ActionSpec::default().with_phase_call("OnPhase"),
        )]),
    );
    engine.register_definition(actor).expect("actor registers");
    for x in 0..64 {
        engine
            .spawn_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)))
            .expect("filler spawns");
    }
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_action(ActionState::new("Swim")))
        .expect("actor spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    let action_index = engine.objects[actor_index].state.action.act_map_index;

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    engine
        .invoke_action_callback(
            actor_index,
            ActionCallbackKind::Phase,
            "Swim",
            action_index,
            None,
            None,
            None,
            None,
        )
        .expect("phase callback succeeds");
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
        Definition::from_script("TARG", "Target", "#strict\n").expect("target compiles"),
        Definition::from_script("FILL", "Filler", "#strict\n").expect("filler compiles"),
    ] {
        engine
            .register_definition(definition)
            .expect("definition registers");
    }
    let mut searcher = Definition::from_script(
        "SRCH",
        "Searcher",
        "#strict\n\
             protected func FindTarget() { return(FindObject(TARG)); }\n\
             protected func FindOwned() { return(FindObjectOwner(TARG, 3)); }\n",
    )
    .expect("searcher compiles");
    searcher.set_c4_callback_convention(true);
    engine
        .register_definition(searcher)
        .expect("searcher registers");
    for player in [2, 3] {
        engine
            .register_player(PlayerConfig::new(player, format!("Player {player}")))
            .expect("player registers");
    }

    let older_target = engine
        .spawn_object(SpawnConfig::new("TARG").with_owner(3))
        .expect("older target spawns");
    for x in 0..64 {
        engine
            .spawn_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)))
            .expect("filler spawns");
    }
    let newer_target = engine
        .spawn_object(SpawnConfig::new("TARG").with_owner(2))
        .expect("newer target spawns");
    for x in 64..128 {
        engine
            .spawn_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)))
            .expect("filler spawns");
    }
    let searcher = engine
        .spawn_object(SpawnConfig::new("SRCH"))
        .expect("searcher spawns");
    let searcher_index = engine.find_object_index(searcher).expect("searcher exists");

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
fn lazy_host_world_global_effect_without_world_access_copies_nothing() {
    use std::sync::Mutex;

    let calls = Arc::new(Mutex::new(0usize));
    let mut hooks = DebuggerHooks::new();
    {
        let calls = Arc::clone(&calls);
        hooks.set_on_call(move |name, _| {
            if name == "FxLazyTimer" {
                *calls.lock().expect("call counter") += 1;
            }
        });
    }
    let mut definition = Definition::from_script(
        "GFXL",
        "Global lazy effect",
        "#strict\nfunc FxLazyTimer(object target, int number, int time) { return(0); }",
    )
    .expect("effect definition compiles");
    definition.set_c4_callback_convention(true);
    definition.set_debugger_hooks(hooks);
    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(definition)
        .expect("effect definition registers");
    engine
        .register_script_definition("FILL", "Filler", "#strict\n")
        .expect("filler registers");
    for x in 0..64 {
        engine
            .spawn_object(SpawnConfig::new("FILL").with_position(Vector2::new(x, 10)))
            .expect("filler spawns");
    }
    let mut effect = EffectState::new("Lazy")
        .with_interval(1)
        .with_command_id(Some("GFXL"));
    effect.number = 1;
    engine.global_effects.push(effect);

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    engine.tick_global_effects().expect("global effect ticks");
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
    let mut landscape =
        Landscape::with_default_material(100, vec![100; 100], None).expect("contact landscape");
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

    engine
        .register_script_definition("FILL", "Filler", "#strict\n")
        .expect("filler registers");
    let mut swimmer = Definition::from_script(
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
    )
    .expect("swimmer compiles");
    swimmer.set_c4_callback_convention(true);
    swimmer.set_contact_function_calls(true);
    swimmer.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
    swimmer.set_shape_vertices(vec![ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT)]);
    engine
        .register_definition(swimmer)
        .expect("swimmer registers");

    for x in 0..128 {
        engine
            .spawn_object(SpawnConfig::new("FILL").with_position(Vector2::new(x % 100, 10)))
            .expect("filler spawns");
    }
    let swimmer = engine
        .spawn_object(
            SpawnConfig::new("SWIM")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(1, 0))
                .with_mobile(true),
        )
        .expect("swimmer spawns");
    let swimmer_index = engine.find_object_index(swimmer).expect("swimmer exists");
    let definition_id = engine.objects[swimmer_index].definition_id.clone();
    let action_library = engine
        .definitions
        .get(&definition_id)
        .expect("swimmer definition exists")
        .action_library()
        .clone();
    let solid_mask_indices = engine.active_solid_mask_indices();

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    engine
        .exec_object_movement(
            swimmer_index,
            &action_library,
            &definition_id,
            &solid_mask_indices,
        )
        .expect("free movement executes");
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

    engine
        .landscape
        .as_mut()
        .expect("landscape exists")
        .grid_write_byte(54, 50, 1);
    // Cross one free pixel first so DoMotion mutably reborrows the
    // non-mover slices before the same movement reaches ContactRight.
    engine.objects[swimmer_index].set_fixed_velocity(FixedVec2::from_ints(2, 0));
    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    engine
        .exec_object_movement(
            swimmer_index,
            &action_library,
            &definition_id,
            &solid_mask_indices,
        )
        .expect("contact movement executes");
    assert_eq!(
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
        engine.objects.len(),
        "the first real Contact* call materializes one complete world"
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
    let mut landscape =
        Landscape::with_default_material(200, vec![200; 200], None).expect("contact landscape");
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

    let mut walker_definition =
        Definition::from_script("WALK", "Walker", "").expect("walker compiles");
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
    engine
        .register_definition(walker_definition)
        .expect("walker registers");

    let walker = engine
        .spawn_object(
            SpawnConfig::new("WALK")
                .with_position(Vector2::new(100, 100))
                .with_velocity(Vector2::new(-1, 0))
                .with_action(ActionState::new("Walk"))
                .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                .with_crew_member(true)
                .with_alive(true)
                .with_mobile(true),
        )
        .expect("walker spawns");
    let walker_index = engine.find_object_index(walker).expect("walker exists");
    engine.refresh_object_ocf(walker_index);
    assert_ne!(
        engine.objects[walker_index].state.ocf & ocf::CREW_MEMBER,
        0,
        "the fixture participates in C4Command JumpControl"
    );
    let solid_mask_indices = engine.active_solid_mask_indices();
    let definition_id = engine.objects[walker_index].definition_id.clone();
    let action_library = engine
        .definitions
        .get(&definition_id)
        .expect("walker definition exists")
        .action_library()
        .clone();

    engine
        .exec_object_movement(
            walker_index,
            &action_library,
            &definition_id,
            &solid_mask_indices,
        )
        .expect("rejected movement step executes");

    assert_eq!(
        engine.live_command_snapshot(walker_index).contact & CNAT_LEFT,
        CNAT_LEFT,
        "the rejected candidate step latches C4Object::t_contact"
    );

    engine.objects[walker_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(60))
                .with_ty(Some(93))
                // C4CMD_MoveTo_NoPosAdjust keeps the low-side target.
                .with_data(CommandData::Integer(1)),
        )
        .expect("MoveTo command queues");
    engine
        .execute_object_command_now(walker)
        .expect("MoveTo evaluates");
    engine
        .execute_object_command_now(walker)
        .expect("MoveTo reads previous-frame t_contact");

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
    engine
        .register_script_definition("SECT", "Sector", "")
        .expect("definition registers");
    engine.set_landscape(Landscape::flat(100, 100));
    for x in 0..8 {
        engine
            .spawn_object(SpawnConfig::new("SECT").with_position(Vector2::new(x * 8, 10)))
            .expect("object spawns");
    }

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    let context = engine.host_world_context();
    let found = context
        .object_sector_ids_in_rect(DefinitionRect::new(0, 0, 100, 100))
        .expect("a landscape-backed context has a sector map");

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
    engine
        .register_script_definition("SECT", "Sector", "")
        .expect("definition registers");
    engine.set_landscape(Landscape::flat(400, 100));
    let object = engine
        .spawn_object(SpawnConfig::new("SECT").with_position(Vector2::new(10, 10)))
        .expect("object spawns");

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
    engine
        .register_definition(
            Definition::from_script(
                "SECT",
                "Sector mover",
                r#"#strict
public func MoveAndFind()
{
    SetPosition(310, 10);
    return GetLength(FindObjects([C4FO_InRect, 300, 0, 50, 100]));
}
"#,
            )
            .expect("mover compiles"),
        )
        .expect("mover registers");
    let object = engine
        .spawn_object(SpawnConfig::new("SECT").with_position(Vector2::new(10, 10)))
        .expect("mover spawns");
    let index = engine.find_object_index(object).expect("mover exists");

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
    let mut definition = Definition::from_script(
        "ROTR",
        "Sector rotator",
        r#"#strict
public func RotateAndFind()
{
    SetR(90);
    return GetLength(FindObjects([C4FO_AtRect, 260, 100, 1, 1]));
}
"#,
    )
    .expect("rotator compiles");
    definition.set_rotateable(1);
    definition.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    engine
        .register_definition(definition)
        .expect("rotator registers");
    let object = engine
        .spawn_object(SpawnConfig::new("ROTR").with_position(Vector2::new(200, 100)))
        .expect("rotator spawns");
    let index = engine.find_object_index(object).expect("rotator exists");

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
    let mut replacement = Definition::from_script("NEW1", "Wide replacement", "#strict\n")
        .expect("replacement compiles");
    replacement.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    engine
        .register_definition(replacement)
        .expect("replacement registers");
    let mut original = Definition::from_script(
        "OLD1",
        "Narrow original",
        r#"#strict
public func ChangeAndFind()
{
    ChangeDef(NEW1);
    return GetLength(FindObjects([C4FO_AtRect, 130, 100, 1, 1]));
}
"#,
    )
    .expect("original compiles");
    original.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    engine
        .register_definition(original)
        .expect("original registers");
    let object = engine
        .spawn_object(SpawnConfig::new("OLD1").with_position(Vector2::new(200, 100)))
        .expect("original spawns");
    let index = engine.find_object_index(object).expect("original exists");

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
    let mut definition = Definition::from_script(
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
    )
    .expect("grower compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 80)));
    engine
        .register_definition(definition)
        .expect("grower registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("GROW")
                .with_position(Vector2::new(200, 100))
                .with_construction(FULL_CON / 2),
        )
        .expect("grower spawns");
    let index = engine.find_object_index(object).expect("grower exists");

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
    let mut grower = Definition::from_script(
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
    )
    .expect("grower compiles");
    grower.set_c4_callback_convention(true);
    grower.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 80)));
    engine
        .register_definition(grower)
        .expect("grower registers");
    engine
        .register_script_definition("ITEM", "Contained item", "#strict\n")
        .expect("item registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("GROW")
                .with_position(Vector2::new(200, 100))
                .with_construction(FULL_CON / 4),
        )
        .expect("grower spawns");
    engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(object))
        .expect("contained item spawns");
    let index = engine.find_object_index(object).expect("grower exists");

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
    let mut container = Definition::from_script(
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
    )
    .expect("container compiles");
    container.set_c4_callback_convention(true);
    engine
        .register_definition(container)
        .expect("container registers");
    engine
        .register_definition(
            Definition::from_script(
                "ITEM",
                "Exiting item",
                r#"#strict
public func Leave()
{
    return Exit(this(), 310, 10);
}
"#,
            )
            .expect("item compiles"),
        )
        .expect("item registers");
    let container = engine
        .spawn_object(SpawnConfig::new("CONT"))
        .expect("container spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(container))
        .expect("item spawns contained");
    let item_index = engine.find_object_index(item).expect("item exists");

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
    let mut container = Definition::from_script(
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
    )
    .expect("container compiles");
    container.set_c4_callback_convention(true);
    engine
        .register_definition(container)
        .expect("container registers");
    engine
        .register_definition(
            Definition::from_script(
                "ITEM",
                "Entering item",
                r#"#strict
public func Go(object container)
{
    return Enter(container);
}
"#,
            )
            .expect("item compiles"),
        )
        .expect("item registers");
    let container = engine
        .spawn_object(SpawnConfig::new("CONT").with_position(Vector2::new(10, 10)))
        .expect("container spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_position(Vector2::new(310, 10)))
        .expect("item spawns outside");
    let item_index = engine.find_object_index(item).expect("item exists");

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
    let mut definition = Definition::from_script(
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
    )
    .expect("vertex editor compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    engine
        .register_definition(definition)
        .expect("vertex editor registers");
    let object = engine
        .spawn_object(SpawnConfig::new("VRTX").with_position(Vector2::new(200, 10)))
        .expect("vertex editor spawns");
    let index = engine
        .find_object_index(object)
        .expect("vertex editor exists");

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
    let mut collector = Definition::from_script(
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
    )
    .expect("collector compiles");
    collector.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
    engine
        .register_definition(collector)
        .expect("collector registers");
    let mut item =
        Definition::from_script("ITEM", "Collected item", "#strict\n").expect("item compiles");
    item.set_collectible(true);
    engine.register_definition(item).expect("item registers");
    let collector = engine
        .spawn_object(SpawnConfig::new("COLL").with_position(Vector2::new(10, 10)))
        .expect("collector spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_position(Vector2::new(310, 10)))
        .expect("item spawns");
    let collector_index = engine
        .find_object_index(collector)
        .expect("collector exists");

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
    let mut definition = Definition::from_script(
        "ACTV",
        "Sector activator",
        r#"#strict
public func ActivateAndFind()
{
    SetObjectStatus(C4OS_NORMAL);
    return [GetLength(FindObjects([C4FO_AtRect, 130, 5, 1, 1])), GetObjWidth()];
}
"#,
    )
    .expect("activator compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(-80, 0, 80, 10)));
    engine
        .register_definition(definition)
        .expect("activator registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("ACTV")
                .with_position(Vector2::new(200, 10))
                .with_shape_rect(DefinitionRect::new(0, 0, 1, 1))
                .with_status(ObjectStatus::Inactive),
        )
        .expect("inactive object spawns");
    let index = engine
        .find_object_index(object)
        .expect("inactive object exists");

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
    engine
        .register_script_definition("ORDR", "Ordered candidate", "#strict\n")
        .expect("candidate definition registers");
    engine
        .register_definition(
            Definition::from_script(
                "MOVR",
                "Sector mover",
                r#"#strict
public func MoveAndFindOrdered()
{
    SetPosition(310, 10);
    return FindObjects([C4FO_InRect, 0, 0, 50, 100]);
}
"#,
            )
            .expect("mover compiles"),
        )
        .expect("mover registers");
    let older = engine
        .spawn_object(SpawnConfig::new("ORDR").with_position(Vector2::new(10, 10)))
        .expect("older candidate spawns");
    let newer = engine
        .spawn_object(SpawnConfig::new("ORDR").with_position(Vector2::new(20, 10)))
        .expect("newer candidate spawns");
    for x in 100..116 {
        engine
            .spawn_object(SpawnConfig::new("ORDR").with_position(Vector2::new(x, 10)))
            .expect("unrelated candidate spawns");
    }
    let mover = engine
        .spawn_object(SpawnConfig::new("MOVR").with_position(Vector2::new(210, 10)))
        .expect("mover spawns");

    let older_index = engine.find_object_index(older).expect("older exists");
    let newer_index = engine.find_object_index(newer).expect("newer exists");
    engine.objects[older_index].state.category = CATEGORY_OBJECT;
    engine.objects[newer_index].state.category = CATEGORY_STRUCTURE;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SortByCategory);
    engine.execute_object_order_commands();

    HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
    let mover_index = engine.find_object_index(mover).expect("mover exists");
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
        3,
        "only the caller and two returned candidates materialize; the first sector mutation clones the entry map instead of every object"
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
    engine
        .register_script_definition("ORDR", "Ordered candidate", "#strict\n")
        .expect("candidate definition registers");
    engine
        .register_script_definition("MOVR", "Foreign mover", "#strict\n")
        .expect("mover definition registers");
    engine
        .register_script_definition("STAT", "Foreign status target", "#strict\n")
        .expect("status definition registers");
    for id in ["CHG1", "CHG2"] {
        engine
            .register_script_definition(id, id, "#strict\n")
            .expect("change definition registers");
    }
    engine
        .register_script_definition("DEAD", "Foreign removal target", "#strict\n")
        .expect("removal definition registers");
    let mut observer = Definition::from_script(
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
    )
    .expect("effect observer compiles");
    observer.set_c4_callback_convention(true);
    engine
        .register_definition(observer)
        .expect("effect observer registers");
    let older = engine
        .spawn_object(SpawnConfig::new("ORDR").with_position(Vector2::new(10, 10)))
        .expect("older candidate spawns");
    let newer = engine
        .spawn_object(SpawnConfig::new("ORDR").with_position(Vector2::new(20, 10)))
        .expect("newer candidate spawns");
    let mover = engine
        .spawn_object(SpawnConfig::new("MOVR").with_position(Vector2::new(210, 10)))
        .expect("foreign mover spawns");
    let status_target = engine
        .spawn_object(SpawnConfig::new("STAT").with_position(Vector2::new(260, 10)))
        .expect("foreign status target spawns");
    let change_target = engine
        .spawn_object(SpawnConfig::new("CHG1").with_position(Vector2::new(270, 10)))
        .expect("foreign change target spawns");
    let removal_target = engine
        .spawn_object(SpawnConfig::new("DEAD").with_position(Vector2::new(280, 10)))
        .expect("foreign removal target spawns");
    let observer = engine
        .spawn_object(SpawnConfig::new("FXOR").with_position(Vector2::new(210, 20)))
        .expect("effect observer spawns");

    let older_index = engine.find_object_index(older).expect("older exists");
    let newer_index = engine.find_object_index(newer).expect("newer exists");
    engine.objects[older_index].state.category = CATEGORY_OBJECT;
    engine.objects[newer_index].state.category = CATEGORY_STRUCTURE;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SortByCategory);
    engine.execute_object_order_commands();

    let observer_index = engine.find_object_index(observer).expect("observer exists");
    engine
        .call_object_function(
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
        )
        .expect("effects arm");
    let observer_index = engine
        .find_object_index(observer)
        .expect("observer remains");
    let move_effect = engine.objects[observer_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "Move")
        .cloned()
        .expect("move effect exists");
    let observe_effect = engine.objects[observer_index]
        .state
        .effects
        .iter()
        .find(|effect| effect.name == "Observe")
        .cloned()
        .expect("observe effect exists");
    let definition_id = engine.objects[observer_index].definition_id.clone();

    engine
        .dispatch_object_effect_events(
            observer_index,
            &definition_id,
            vec![
                EffectEvent::timer(move_effect),
                EffectEvent::timer(observe_effect),
            ],
        )
        .expect("effect batch executes");

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
        engine
            .register_definition(Definition::from_script(id, id, "").expect("definition compiles"))
            .expect("definition registers");
    }
    engine.set_landscape(Landscape::flat(400, 200));

    // Spread objects across sector boundaries and interleave the two
    // definitions so an ordering change cannot hide behind a stable grouping.
    let mut spawned = Vec::new();
    for index in 0..24 {
        let id = if index % 2 == 0 { "SCTA" } else { "SCTB" };
        let object = engine
            .spawn_object(
                SpawnConfig::new(id).with_position(Vector2::new(9 + index * 15, 20 + index % 5)),
            )
            .expect("object spawns");
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
    let whole = context
        .object_sector_ids_in_rect(DefinitionRect::new(0, 0, 400, 200))
        .expect("a landscape-backed context has a sector map");
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
    let window = context
        .object_sector_ids_in_rect(DefinitionRect::new(100, 0, 120, 200))
        .expect("sector map present");
    assert_eq!(
        window,
        expect(&SECTOR_GROUPS[2..5]),
        "a partial rect preserves master-list order within the covered sectors"
    );

    // Per-sector lists are the shape FindObject consumes; freeze the grouping
    // as well as the flattening, since only the latter is order-insensitive.
    let lists = context
        .object_sector_id_lists_in_rect(DefinitionRect::new(0, 0, 400, 200))
        .expect("sector map present");
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
    let mut landscape =
        Landscape::with_default_material(100, vec![100; 100], None).expect("query landscape");
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
    engine
        .landscape
        .as_mut()
        .expect("landscape exists")
        .grid_write_byte(10, 20, 1);

    let mut prober = Definition::from_script(
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
    )
    .expect("prober compiles");
    prober.set_c4_callback_convention(true);
    engine
        .register_definition(prober)
        .expect("prober registers");
    let prober = engine
        .spawn_object(SpawnConfig::new("PROB").with_position(Vector2::new(50, 50)))
        .expect("prober spawns");
    let prober_index = engine.find_object_index(prober).expect("prober exists");

    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    engine
        .call_object_function(prober_index, "Probe", Vec::new())
        .expect("terrain probe succeeds");
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
