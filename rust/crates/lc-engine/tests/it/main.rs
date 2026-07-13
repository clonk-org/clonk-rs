// One integration-test binary for the whole crate: each former tests/*.rs
// file is a module here, so an engine edit costs one test-crate compile and
// one link instead of one per file. nextest still runs every #[test]
// separately (process-per-test), so isolation and parallelism are unchanged.
mod support;

mod action_attach;
mod action_build;
mod action_procedure;
mod component_natives;
mod component_order;
mod dev_feedback_replay;
mod dragon_rock_audio;
mod elevator_motion_oracle;
mod engine_snapshots;
mod far_worlds_arctic_kayak;
mod far_worlds_deep_airlock;
mod flight_movement;
mod gamma;
mod get_inventory;
mod hangle_movement;
mod legacy_scenario_loading;
mod manifest_definitions;
mod object_visibility;
mod real_clonk_hangle;
mod real_scenario_harness;
mod real_tutorial01_virtual_play;
mod real_tutorial02_balloon_platform;
mod real_tutorial02_virtual_play;
mod real_tutorial03_production;
mod real_tutorial03_virtual_play;
mod real_tutorial04_construction;
mod real_tutorial04_virtual_play;
mod real_tutorial05_route;
mod real_tutorial06_virtual_play;
mod real_tutorial07_virtual_play;
mod real_tutorial08_virtual_play;
mod real_tutorial09_virtual_play;
mod real_tutorial10_virtual_play;
mod real_tutorial_campaign;
mod script_counter;
mod script_goto;
mod set_picture;
mod sim_flight;
mod swim_movement;
mod test_action_callback_return_value;
mod test_construction_callback;
mod test_get_id;
mod test_include_resolution;
mod test_local_var_initialization;
mod test_math_functions;
mod test_object_id_reservation;
mod test_transitive_includes;
mod virtual_player_harness;
mod walk_movement;
mod weather_audio;
