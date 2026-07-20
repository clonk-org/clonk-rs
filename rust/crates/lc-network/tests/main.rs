// Keep the network integration suite in one harness. Each integration target
// links the complete engine/network dependency graph, while modules preserve
// the existing test isolation and namespacing inside a single executable.
mod client_player_resource;
mod exact_game_reference;
mod host_initial_resources;
mod host_resource_core;
mod initial_network_dynamic;
mod initial_network_metadata;
mod initial_network_parameters;
mod legacy_control_cpp_diff;
mod live_network_dynamic;
mod local_resource_resolution;
mod resource_file_store;
mod resource_transfer_backend;
mod startup_game_advertiser;
mod startup_game_search;
mod synchronized_tick;

// These two tests exercise private packet/catalog internals. Keep one shared
// copy at the harness root because resource_catalog uses `crate::resource_packet`.
#[allow(dead_code)]
#[path = "../src/resource_catalog.rs"]
mod resource_catalog;
#[path = "resource_catalog.rs"]
mod resource_catalog_tests;
#[allow(dead_code)]
#[path = "../src/resource_packet.rs"]
mod resource_packet;
#[path = "resource_packet_codec.rs"]
mod resource_packet_codec_tests;
