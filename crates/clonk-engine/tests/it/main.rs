// One integration-test binary for the whole crate: each former tests/*.rs file
// is a module here, so an engine edit costs one test-crate compile and one link
// instead of one per file. Nextest runs each #[test] in a separate process.
// Explicitly batched real-scenario subcases share immutable preparation within
// one process, but every subcase instantiates a fresh Engine.
mod support;

#[cfg(all(
    feature = "engine-it-sharded",
    not(any(feature = "engine-it-shard-1", feature = "engine-it-shard-2",)),
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
// bird_flight offsets shard 2's use beside the unit and parity suites in CI.
shard_modules!(
    "engine-it-shard-1",
    action_attach,
    action_build,
    action_procedure,
    activate_entrance_native,
    bird_flight,
    burning_clonk_fire_particles,
    component_natives,
    component_order,
    construction_check_feedback,
    creation_owner_strictness,
    dev_feedback_replay,
    effect_negotiation,
    eke_pistol,
    eke_uzi_action_sound,
    elevator_motion_oracle,
    engine_snapshots,
    far_worlds_arctic_harpoon_drop,
    gamma,
    get_inventory,
    get_material_color,
    get_material_val,
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
    blobby_soccer_effect_call,
    body_declarations,
    dragon_rock_audio,
    effect_check_conversion,
    effect_command_target_context,
    eke_flamethrower_particles,
    eke_gped_remote_control,
    eke_missile_guidance,
    eke_missile_schedule,
    far_worlds_arctic_kayak,
    far_worlds_deep_airlock,
    far_worlds_deep_construction,
    far_worlds_deep_lorry_acquire,
    far_worlds_jungle_amulet,
    flight_movement,
    get_act_map_val,
    get_entrance,
    global_call,
    harpoonrace_worldgen,
    hazard_crosshair,
    hazard_death_relaunch,
    legacy_scenario_loading,
    literal_zero_strictness,
    manifest_definitions,
    mars_base_order_menu,
    mars_folder_material_landscape,
    mars_menu_override_drift,
    mars_base_research_exit,
    mars_material_unit_entrance,
    mars_oxygen,
    message_board_queries,
    optional_int_strictness,
    real_tutorial01_virtual_play,
    real_tutorial03_production,
    real_tutorial04_virtual_play,
    real_tutorial05_route,
    real_tutorial06_virtual_play,
    real_tutorial07_virtual_play,
    real_tutorial09_virtual_play,
    real_tutorial10_virtual_play,
    real_tutorial_campaign,
    reference_parameters,
    script_counter,
    script_goto,
    set_builtin,
    set_vertex,
    spawn_container_order,
    surplus_host_args,
    test_action_callback_return_value,
    test_local_var_initialization,
    test_object_id_reservation,
    test_transitive_includes,
    typed_arrow_caller_strictness,
    virtual_player_harness,
    walk_movement,
    weather_audio,
);
