use super::*;
use crate::landscape::PixelGrid;
use std::collections::HashMap;

#[test]
fn synchronize_control_applies_clearance_only_when_requested() {
        // C4ControlSynchronize executes Game.Synchronize first and calls
        // Game.SyncClearance only when SyncClear is set. The latter alone
        // collapses fixed coordinates to integer object state (pristine
        // 9ffa0a5d src/C4Control.cpp:537-550;
        // src/C4Game.cpp:3679-3715; src/C4Object.cpp:3803-3815).
    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(
            Definition::from_script("SYNC", "Sync", "").expect("definition compiles"),
        )
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
            death_message: String::new(),
            core: CrewInfoCoreFields::default(),
            rank: 0,
            rank_name: "Clonk".to_string(),
            experience: 0,
            rounds: 0,
            physical: PhysicalInfo::default(),
            death_count: 0,
            total_playing_time: 7,
            birthday: 0,
            age: 0,
            participation: 1,
            in_action: true,
            was_in_action: true,
            in_action_time: 10,
            has_died: false,
            extra_data: Vec::new(),
            portraits: CrewPortraitState::default(),
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
        death_message: String::new(),
        core: CrewInfoCoreFields::default(),
        rank: 0,
        rank_name: "Clonk".to_string(),
        experience: 0,
        rounds: 0,
        physical: PhysicalInfo::default(),
        death_count: 0,
        total_playing_time: 17,
        birthday: 0,
        age: 0,
        participation: 1,
        in_action: true,
        was_in_action: true,
        in_action_time: 10,
        has_died: false,
        extra_data: Vec::new(),
        portraits: CrewPortraitState::default(),
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

    let mut definition =
        Definition::from_script("TILT", "Tilt", "").expect("definition compiles");
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
        .register_definition(
            Definition::from_script("FILL", "Filler", "#strict\n").expect("filler compiles"),
        )
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
        1,
            "terrain is cloned on its first actual query"
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
fn lazy_host_world_action_callback_seeds_only_caller() {
    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(
            Definition::from_script("FILL", "Filler", "#strict\n").expect("filler compiles"),
        )
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
        .register_definition(
            Definition::from_script("FILL", "Filler", "#strict\n").expect("filler compiles"),
        )
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
        .register_definition(
            Definition::from_script("FILL", "Filler", "#strict\n").expect("filler compiles"),
        )
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
        1,
            "Contact* clones terrain only when GBackSolid first queries it"
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
    // script call, which is the hot path while many flames are alive.
    let mut engine = Engine::with_seed(0);
    engine
        .register_definition(
            Definition::from_script("SECT", "Sector", "").expect("definition compiles"),
        )
        .expect("definition registers");
    engine.set_landscape(Landscape::flat(100, 100));
    for x in 0..8 {
        engine
            .spawn_object(SpawnConfig::new("SECT").with_position(Vector2::new(x * 8, 10)))
            .expect("object spawns");
    }

    HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(0));
    let context = engine.host_world_context();
    let found = context
        .object_sector_ids_in_rect(DefinitionRect::new(0, 0, 100, 100))
        .expect("a landscape-backed context has a sector map");

    assert_eq!(found.len(), 8, "every spawned object is inside the query rect");
    assert_eq!(
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(Cell::get),
        0,
        "sector sizing must not clone the landscape shell"
    );
}

#[test]
fn sector_query_ordering_is_frozen_across_rebuild_and_incremental_paths() {
    // FREEZE, not a new behavior claim. `C4LSectors` keeps its own physical
    // per-sector list order and refreshes only a rank oracle on SortByCategory
    // (oracle-src-pinned src/C4Sector.cpp:107-160), so a map rebuilt from the
    // current object set and a map mutated incrementally can legitimately
    // disagree. FindObject ordering is determinism-critical, so pin the exact
    // sequences the callback-local rebuild produces today. Any change to how
    // host contexts obtain their sector map must leave every assertion here
    // untouched.
    let mut engine = Engine::with_seed(0);
    for id in ["SCTA", "SCTB"] {
        engine
            .register_definition(
                Definition::from_script(id, id, "").expect("definition compiles"),
            )
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

    let context = engine.host_world_context();
    let whole = context
        .object_sector_ids_in_rect(DefinitionRect::new(0, 0, 400, 200))
        .expect("a landscape-backed context has a sector map");
    assert_eq!(
        whole,
        spawned,
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
    let expected_window: Vec<_> = spawned.iter().copied().skip(7).take(10).collect();
    assert_eq!(
        window, expected_window,
        "a partial rect preserves master-list order within the covered sectors"
    );

    // Per-sector lists are the shape FindObject consumes; freeze the grouping
    // as well as the flattening, since only the latter is order-insensitive.
    let lists = context
        .object_sector_id_lists_in_rect(DefinitionRect::new(0, 0, 400, 200))
        .expect("sector map present");
    assert_eq!(
        lists.iter().flatten().copied().collect::<Vec<_>>(),
        spawned,
        "per-sector lists flatten back to master-list order"
    );
    assert!(
        lists.iter().any(|list| list.len() > 1),
        "the fixture must actually populate a shared sector, or it freezes nothing"
    );
}
