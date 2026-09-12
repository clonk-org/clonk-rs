//! Engine-backed callback worlds, constructed without fixture placeholders.
use super::*;

impl HostWorldContext {
    /// Seed the shared engine resources directly. Mutable previews still get
    /// fresh callback-local storage; lazy reads retain the paused-engine
    /// provider's synchronous lifetime and exclusive-borrow contract.
    pub(crate) fn from_engine(
        engine: &crate::Engine,
        solid_mask_metadata: Rc<HashMap<DefinitionId, HostSolidMaskMetadata>>,
        solid_mask_state: crate::SolidMaskHostStateCache,
        provider: LazyHostWorldProvider,
    ) -> Self {
        let tables = engine.host_definition_tables();
        let player_order = engine.player_ids_in_order();
        let local_players = engine.local_players.as_ref().map_or_else(
            || player_order.iter().copied().collect(),
            |players| players.iter().copied().collect(),
        );
        let player_states = player_order
            .iter()
            .copied()
            .map(|id| (id, OnceCell::new()))
            .collect();
        // Preserve all three sources of known info IDs, including progress
        // entries without a live player and zero-score entries.
        let player_info_ids = engine
            .player_info_league_progress_data
            .keys()
            .copied()
            .chain(engine.players.values().map(crate::Player::player_info_id))
            .filter(|id| *id != 0)
            .chain(
                engine
                    .player_info_league_scores
                    .keys()
                    .copied()
                    .filter(|id| *id > 0),
            )
            .collect();
        Self {
            object_store: RefCell::new(Rc::new(HostWorldObjectStore {
                objects: FxHashMap::default(),
                order: Vec::new(),
                indices: FxHashMap::default(),
                removed: FxHashSet::default(),
                order_dirty: false,
                complete: false,
            })),
            effect_spawn_previews: Rc::new(RefCell::new(Vec::new())),
            lazy_world: Some(provider),
            pending_instance_tokens: Rc::new(RefCell::new(HashMap::new())),
            next_pending_instance_token: Rc::new(Cell::new(1)),
            master_order: OnceCell::new(),
            inactive_order: Rc::new(engine.execution.inactive.iter().rev().copied().collect()),
            landscape: OnceCell::new(),
            scenario_values: Rc::clone(&engine.scenario_values),
            scenario_sections: Rc::new(
                engine
                    .scenario_section_state
                    .sections
                    .values()
                    .map(|section| section.name.to_ascii_lowercase())
                    .collect(),
            ),
            scenario_section_landscape_extents: Rc::new(
                engine
                    .scenario_section_state
                    .sections
                    .values()
                    .map(|section| {
                        (
                            section.name.to_ascii_lowercase(),
                            section
                                .landscape
                                .as_ref()
                                .map(crate::compat::landscape_extent),
                        )
                    })
                    .collect(),
            ),
            scenario_section_landscape_extent: None,
            scenario_section_switch_in_flight: engine.scenario_section_state.switch_in_flight,
            suspended_script_registrations: Some(Rc::clone(&engine.suspended_script_registrations)),
            movement_solid_masks: Rc::new(engine.ocf_solid_mask_overlay()),
            definitions: engine.definition_metadata_table(),
            solid_mask_metadata,
            solid_mask_bakes: Rc::clone(&solid_mask_state.bakes),
            solid_mask_instance_sequences: Rc::new(RefCell::new(
                solid_mask_state.instance_sequences.as_ref().clone(),
            )),
            next_solid_mask_instance_sequence: Rc::new(Cell::new(
                solid_mask_state.next_instance_sequence,
            )),
            color_by_owner_definitions: Rc::clone(&tables.color_by_owner),
            base_auto_sell_definitions: Rc::clone(&tables.base_auto_sell),
            rebuyable_definitions: Rc::clone(&tables.rebuyable),
            no_sell_definitions: Rc::clone(&tables.no_sell),
            definition_descriptions: Rc::clone(&tables.descriptions),
            definition_rank_names: Rc::clone(&tables.rank_names),
            default_rank_names: Rc::clone(&engine.default_rank_names),
            definition_rank_bases: Rc::clone(&tables.rank_bases),
            definition_order: Rc::clone(&engine.definition_order.runtime_order),
            sectors: RefCell::new(None),
            borrowed_sector_map_valid: Cell::new(provider.sector_map_borrow.is_some()),
            transfer_zones: Rc::new(engine.transfer_zones.states()),
            pathfinder_level: engine.pathfinder_level,
            pathfinder_transfer_zones_enabled: engine.pathfinder_transfer_zones_enabled,
            pathfinder_debug: Rc::clone(&engine.pathfinder_debug),
            local_players: Rc::new(local_players),
            physical_viewport_players: Rc::clone(&engine.physical_viewport_players),
            active_message_board_input: engine.active_message_board_input.clone(),
            player_order: Rc::new(player_order),
            player_info_ids: Rc::new(player_info_ids),
            player_states: Rc::new(player_states),
            player_fow_view_objects: Rc::new(
                engine
                    .players
                    .values()
                    .map(|player| {
                        (
                            player.id(),
                            player.fow_view_objects().iter().copied().collect(),
                        )
                    })
                    .collect(),
            ),
            control_key_names: Rc::clone(&engine.control_key_names),
            teams: Rc::clone(&engine.team_state.teams),
            crew_selection: Rc::new(HashMap::new()),
            next_object_id: engine.next_object_id,
            next_storage_index: engine.objects.len(),
            team_home_base_rule: engine.team_home_base_rule,
            shared_bases: engine.shared_bases,
            needed_material_strings: Rc::clone(&engine.needed_material_strings),
            construction_check_strings: Rc::clone(&engine.construction_check_strings),
            object_no_dig_resource_string: Rc::clone(&engine.object_no_dig_resource_string),
            league_game: engine.league_game,
            game_tick_delay_ms: Rc::clone(&engine.game_tick_delay_ms),
            game_tick_delay_revision: Rc::clone(&engine.game_tick_delay_revision),
            league_name: Rc::clone(&engine.league_name),
            player_info_league_progress_data: Rc::clone(&engine.player_info_league_progress_data),
            player_info_league_scores: Rc::new(
                engine
                    .player_info_league_scores
                    .iter()
                    .filter_map(|(&id, &score)| (id > 0 && score != 0).then_some((id, score)))
                    .collect(),
            ),
            team_configuration: engine.team_state.team_configuration,
            network_game: engine.network_game,
            network_control_mode: engine.network_control_mode,
            control_sync_mode: engine.control_sync_mode(),
            edit_cursor_target: engine.edit_cursor_target,
            replay_control: engine.replay_control,
            film_viewport_available: engine.film_viewport_available,
            pause_game_requests: Rc::clone(&engine.host_requests.pause_game_requests),
            network_target_fps_requests: Rc::clone(
                &engine.host_requests.network_target_fps_requests,
            ),
            viewport_presentation_requests: Rc::clone(
                &engine.host_requests.viewport_presentation_requests,
            ),
            smoke_level: engine.bubble_smoke_level(),
            fire_particles_loaded: engine.particle_system.is_fire_particle_loaded(),
            max_players: engine.max_players.unwrap_or_default(),
            use_fair_crew: engine.use_fair_crew,
            fair_crew_strength: engine.fair_crew_strength,
            fair_crew_physical_cache: Rc::clone(&engine.definition_order.fair_crew_physical_cache),
            control_host: engine.control_host,
            player_info_updates: Rc::clone(&engine.host_requests.player_info_updates),
            scenario_script_counter: engine.scenario_script_counter,
            structures_need_energy: engine.structures_need_energy,
            flag_removeable: engine.flag_removeable,
            standard_crew_names: tables.standard_crew_names.clone(),
            definition_crew_names: Rc::clone(&tables.definition_crew_names),
            crew_info_state: Rc::new(RefCell::new(engine.host_crew_info_state())),
            particle_defs: Some(engine.particle_system.shared_def_names()),
            reloadable_particle_defs: Some(engine.particle_system.shared_reloadable_def_names()),
            particle_reload_requests: Rc::clone(&engine.host_requests.particle_reload_requests),
            reloadable_particle_io_success: Some(
                engine.particle_system.shared_reloadable_def_io_success(),
            ),
            reloadable_definitions: Some(Rc::clone(&tables.reloadable_definitions)),
            definition_reload_requests: Rc::clone(&engine.host_requests.definition_reload_requests),
            definition_scripts: Rc::clone(&tables.scripts),
            ordered_definition_scripts: Rc::clone(&tables.ordered_scripts),
            reference_parameter_slots: Rc::clone(&tables.reference_parameter_slots),
            direct_call_function_names: Rc::clone(&tables.direct_call_function_names),
            linked_script_hosts: Rc::clone(&tables.linked_script_hosts),
            scenario_script: engine
                .scenario_script
                .as_ref()
                .map(crate::ScenarioScript::script_arc),
            crew_ranks: Rc::clone(&engine.crew_ranks),
            crew_infos: Rc::clone(&engine.crew_object_infos),
            crew_info_links: Rc::clone(&engine.crew_info_links),
            materials: Some(engine.materials_shared()),
            frame: engine.frame,
            game_time: engine.game_time,
            base_buy_enabled: engine.base_buy_enabled,
            base_sell_enabled: engine.base_sell_enabled,
            base_auto_sell_enabled: engine.base_auto_sell_enabled,
            base_reject_entrance_enabled: engine.base_reject_entrance_enabled,
            base_extinguish_enabled: engine.base_extinguish_enabled,
            sky_adjustment: engine
                .sky
                .as_ref()
                .map(crate::sky::SkyState::adjustment)
                .unwrap_or_default(),
            sky_fade: engine.sky.as_ref().map_or_else(default_sky_fade, |sky| {
                [sky.settings().fade_top, sky.settings().fade_bottom]
            }),
            mission_access: Rc::clone(&engine.mission_access.inner),
            scoreboard: Rc::clone(&engine.scoreboard),
            scoreboard_presentations: Rc::clone(&engine.scoreboard_presentations),
        }
    }
}
