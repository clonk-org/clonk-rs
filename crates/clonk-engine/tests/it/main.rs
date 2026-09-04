// One integration-test binary for the whole crate: each former tests/*.rs file
// is a module here, so an engine edit costs one test-crate compile and one link
// instead of one per file. Nextest runs each #[test] in a separate process.
// Explicitly batched real-scenario subcases share immutable preparation within
// one process, but every subcase instantiates a fresh Engine.

// Match the shipped app's allocator: route tests exercise the same
// allocation-heavy script/snapshot paths without changing simulation state.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod support;

#[cfg(all(
    feature = "engine-it-sharded",
    not(any(
        feature = "engine-it-shard-1",
        feature = "engine-it-shard-2",
        feature = "engine-it-shard-3",
    )),
))]
compile_error!("engine-it-sharded requires an engine-it-shard-N selector");

macro_rules! shard_modules {
    ($selector:literal, $($module:ident),+ $(,)?) => {
        $(
            #[cfg(any(
                not(feature = "engine-it-sharded"),
                feature = $selector,
            ))]
            mod $module;
        )+
    };
}

// Keep cross-module helper bundles atomic. Shard 1 owns recording-host tests;
// the Tutorial 04 and 07 routes balance the other shards' exclusive routes,
// while those shards split the long Queron and sky-lighting inventory cases.
shard_modules!(
    "engine-it-shard-1",
    action_attach,
    action_build,
    airbike_hold_to_steer,
    airbike_pilot_control,
    airbike_pilot_dismount,
    action_procedure,
    activate_entrance_native,
    burning_clonk_fire_particles,
    catapult_payload_launch,
    clonk_party_remake,
    column_landscape_reachability,
    component_natives,
    component_order,
    construction_check_feedback,
    creation_owner_strictness,
    deep_sea_volcano_profile,
    dev_feedback_replay,
    effect_dispatch_profile,
    effect_negotiation,
    eke_pistol,
    eke_uzi_action_sound,
    elevator_motion_oracle,
    engine_snapshots,
    environment_placement_profile,
    far_worlds_arctic_harpoon_drop,
    gamma,
    gidl_race_probe,
    get_inventory,
    get_material_color,
    get_material_val,
    global_add_effect_scaling,
    goldrush_scenario_batches,
    hangle_movement,
    harpoonrace_reload,
    hazard_inventory,
    hazard_squat_aim,
    is_newgfx,
    load_scenario_section,
    object_visibility,
    overlay_effect_timer_persistence,
    path_free2,
    real_clonk_hangle,
    real_scenario_harness,
    real_tutorial02_balloon_platform,
    real_tutorial02_virtual_play,
    real_tutorial03_virtual_play,
    real_tutorial04_construction,
    real_tutorial04_virtual_play,
    real_tutorial07_virtual_play,
    real_tutorial08_virtual_play,
    set_color,
    set_landscape_pixel,
    set_mat_adjust,
    set_material_color,
    set_picture,
    set_plr_show_command,
    sim_flight,
    swim_movement,
    test_construction_callback,
    test_get_id,
    test_include_resolution,
    test_math_functions,
    unused_overlay_id,
);

shard_modules!(
    "engine-it-shard-2",
    bird_flight,
    blobby_soccer_effect_call,
    body_declarations,
    effect_command_target_context,
    eke_flamethrower_particles,
    eke_gped_remote_control,
    eke_missile_schedule,
    far_worlds_arctic_kayak,
    far_worlds_jungle_amulet,
    get_act_map_val,
    get_entrance,
    global_call,
    goldwipfcaves_breath,
    hazard_death_relaunch,
    literal_zero_strictness,
    manifest_definitions,
    mars_folder_material_landscape,
    mars_base_research_exit,
    mars_material_unit_entrance,
    mars_oxygen,
    queron_relaunch_cycle,
    real_tutorial03_production,
    real_tutorial10_virtual_play,
    reference_parameters,
    sailboat_hull_solid_mask,
    script_counter,
    script_goto,
    set_builtin,
    set_vertex,
    surplus_host_args,
    test_action_callback_return_value,
    test_local_var_initialization,
    test_object_id_reservation,
    test_transitive_includes,
    typed_arrow_caller_strictness,
    walk_movement,
);

shard_modules!(
    "engine-it-shard-3",
    dragon_rock_audio,
    dragon_rock_cage,
    effect_check_conversion,
    eke_missile_guidance,
    far_worlds_deep_airlock,
    far_worlds_deep_construction,
    far_worlds_deep_lorry_acquire,
    wagon_grab_put_get,
    flight_movement,
    harpoonrace_worldgen,
    hazard_crosshair,
    legacy_scenario_loading,
    mars_base_order_menu,
    mars_menu_override_drift,
    message_board_queries,
    optional_int_strictness,
    scenario_save_fuzz,
    real_tutorial01_virtual_play,
    real_tutorial05_route,
    real_tutorial06_virtual_play,
    real_tutorial09_virtual_play,
    real_tutorial_campaign,
    scenario_activation_profile,
    script_lookup_profile,
    script_execution_profile,
    skies_of_fire_activation,
    sky_lighting_is_static,
    snapshot_section_profile,
    spawn_container_order,
    virtual_player_harness,
    weather_audio,
    western_sack_pickup,
);
