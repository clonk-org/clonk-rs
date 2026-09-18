//! Engine-backed callback worlds, constructed without fixture placeholders.
use super::*;

#[cfg(test)]
thread_local! {
    static SECTION_TABLE_BUILDS: Cell<usize> = const { Cell::new(0) };
}

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
        Self {
            object_store: RefCell::new(Rc::new(HostWorldObjectStore::reusable(false))),
            effect_spawn_previews: Rc::new(RefCell::new(Vec::new())),
            lazy_world: Some(provider),
            pending_instance_tokens: Rc::new(RefCell::new(HashMap::new())),
            next_pending_instance_token: Rc::new(Cell::new(1)),
            master_order: OnceCell::new(),
            inactive_order: Rc::new(engine.execution.inactive.iter().rev().copied().collect()),
            global_effects: Some(engine.global_effects.clone()),
            landscape: OnceCell::new(),
            scenario_values: Rc::clone(&engine.scenario_values),
            // SAFETY: section storage is frozen until this callback returns.
            scenario_sections: unsafe {
                CallbackSnapshot::deferred(provider.source, scenario_sections)
            },
            // SAFETY: section storage is frozen until this callback returns.
            scenario_section_landscape_extents: unsafe {
                CallbackSnapshot::deferred(provider.source, scenario_section_landscape_extents)
            },
            scenario_section_landscape_extent: None,
            scenario_section_switch_in_flight: engine.scenario_section_state.switch_in_flight,
            suspended_script_registrations: Some(Rc::clone(&engine.suspended_script_registrations)),
            movement_solid_masks: Rc::new(engine.ocf_solid_mask_overlay()),
            definitions: engine.definition_metadata_table(),
            solid_mask_metadata,
            solid_mask_bakes: Rc::clone(&solid_mask_state.bakes),
            solid_mask_instance_sequences: Rc::new(RefCell::new(Rc::clone(
                &solid_mask_state.instance_sequences,
            ))),
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
            // SAFETY: the provider's paused-engine lifetime covers this table.
            local_players: unsafe { CallbackSnapshot::deferred(provider.source, local_players) },
            physical_viewport_players: Rc::clone(&engine.physical_viewport_players),
            active_message_board_input: engine.active_message_board_input.clone(),
            // SAFETY: the provider's paused-engine lifetime covers this table.
            player_order: unsafe { CallbackSnapshot::deferred(provider.source, player_order) },
            // SAFETY: the provider's paused-engine lifetime covers this table.
            player_info_ids: unsafe {
                CallbackSnapshot::deferred(provider.source, player_info_ids)
            },
            // SAFETY: the provider's paused-engine lifetime covers this table.
            player_states: unsafe { CallbackSnapshot::deferred(provider.source, player_states) },
            // SAFETY: the provider's paused-engine lifetime covers this table.
            player_fow_view_objects: unsafe {
                CallbackSnapshot::deferred(provider.source, player_fow_view_objects)
            },
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
            // SAFETY: the provider's paused-engine lifetime covers this table.
            player_info_league_scores: unsafe {
                CallbackSnapshot::deferred(provider.source, player_info_league_scores)
            },
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

// These projections run only while the engine is paused in its synchronous
// callback. Borrow raw fields individually to avoid aliasing the active object.
unsafe fn player_order(source: *const ()) -> Vec<i32> {
    let engine = source.cast::<crate::Engine>();
    // SAFETY: both player fields are stable for the callback's lifetime.
    let players = unsafe { &*std::ptr::addr_of!((*engine).players) };
    let ledger = unsafe { &*std::ptr::addr_of!((*engine).player_order) };
    if ledger.len() == players.len() && ledger.iter().all(|id| players.contains_key(id)) {
        return ledger.clone();
    }
    let mut order = Vec::with_capacity(players.len());
    let mut seen = HashSet::with_capacity(players.len());
    order.extend(
        ledger
            .iter()
            .copied()
            .filter(|id| players.contains_key(id) && seen.insert(*id)),
    );
    let missing_start = order.len();
    order.extend(players.keys().copied().filter(|id| seen.insert(*id)));
    order[missing_start..].sort_unstable();
    order
}

unsafe fn local_players(source: *const ()) -> HashSet<i32> {
    let engine = source.cast::<crate::Engine>();
    // SAFETY: these fields cannot change during a synchronous callback.
    let local = unsafe { &*std::ptr::addr_of!((*engine).local_players) };
    let players = unsafe { &*std::ptr::addr_of!((*engine).players) };
    local.as_ref().map_or_else(
        || players.keys().copied().collect(),
        |local| local.iter().copied().collect(),
    )
}

unsafe fn player_states(source: *const ()) -> HashMap<i32, OnceCell<PlayerState>> {
    let engine = source.cast::<crate::Engine>();
    // SAFETY: player storage is frozen; states remain projected on demand.
    let players = unsafe { &*std::ptr::addr_of!((*engine).players) };
    players.keys().map(|&id| (id, OnceCell::new())).collect()
}

unsafe fn player_fow_view_objects(source: *const ()) -> HashMap<i32, HashSet<ObjectId>> {
    let engine = source.cast::<crate::Engine>();
    // SAFETY: player storage is frozen for the synchronous callback.
    let players = unsafe { &*std::ptr::addr_of!((*engine).players) };
    players
        .values()
        .map(|player| {
            (
                player.id(),
                player.fow_view_objects().iter().copied().collect(),
            )
        })
        .collect()
}

unsafe fn player_info_ids(source: *const ()) -> HashSet<i32> {
    let engine = source.cast::<crate::Engine>();
    // SAFETY: these three registry fields stay unchanged until copy-out.
    let progress = unsafe { &*std::ptr::addr_of!((*engine).player_info_league_progress_data) };
    let players = unsafe { &*std::ptr::addr_of!((*engine).players) };
    let scores = unsafe { &*std::ptr::addr_of!((*engine).player_info_league_scores) };
    // Retain departed players' info IDs and zero-score entries too.
    progress
        .keys()
        .copied()
        .chain(players.values().map(crate::Player::player_info_id))
        .filter(|id| *id != 0)
        .chain(scores.keys().copied().filter(|id| *id > 0))
        .collect()
}

unsafe fn player_info_league_scores(source: *const ()) -> BTreeMap<i32, i32> {
    let engine = source.cast::<crate::Engine>();
    // SAFETY: league scores are stable until callback copy-out.
    let scores = unsafe { &*std::ptr::addr_of!((*engine).player_info_league_scores) };
    scores
        .iter()
        .filter_map(|(&id, &score)| (id > 0 && score != 0).then_some((id, score)))
        .collect()
}

unsafe fn scenario_sections(source: *const ()) -> HashSet<String> {
    #[cfg(test)]
    SECTION_TABLE_BUILDS.with(|count| count.set(count.get() + 1));
    let engine = source.cast::<crate::Engine>();
    // SAFETY: section registry storage is frozen throughout this callback.
    let state = unsafe { &*std::ptr::addr_of!((*engine).scenario_section_state) };
    state
        .sections
        .values()
        .map(|section| section.name.to_ascii_lowercase())
        .collect()
}

unsafe fn scenario_section_landscape_extents(
    source: *const (),
) -> HashMap<String, Option<(i32, i32)>> {
    #[cfg(test)]
    SECTION_TABLE_BUILDS.with(|count| count.set(count.get() + 1));
    let engine = source.cast::<crate::Engine>();
    // SAFETY: section landscapes are not changed until the callback returns.
    let state = unsafe { &*std::ptr::addr_of!((*engine).scenario_section_state) };
    state
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
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callbacks_without_section_queries_leave_section_tables_unbuilt() {
        let mut engine = crate::Engine::new();
        engine
            .register_script_definition("TEST", "Test", "func Probe() { return 42; }")
            .unwrap();
        engine
            .spawn_object(crate::SpawnConfig::new("TEST"))
            .unwrap();
        SECTION_TABLE_BUILDS.with(|count| count.set(0));
        assert_eq!(
            engine.call_object_function(0, "Probe", Vec::new()).unwrap(),
            Value::Int(42)
        );
        assert_eq!(SECTION_TABLE_BUILDS.with(Cell::get), 0);
    }

    #[test]
    fn lazy_player_tables_preserve_order_retired_infos_and_callback_isolation() {
        let mut engine = crate::Engine::new();
        engine
            .register_player(crate::PlayerConfig::new(20, "First").with_player_info_id(7))
            .unwrap();
        engine
            .register_player(crate::PlayerConfig::new(10, "Second").with_player_info_id(8))
            .unwrap();
        // Exercise the supported fallback for fixtures with unledgered players.
        engine
            .players
            .insert(30, crate::Player::new(30, "Unledgered"));
        Rc::make_mut(&mut engine.player_info_league_scores).insert(91, 0);
        Rc::make_mut(&mut engine.player_info_league_scores).insert(92, 12);
        Rc::make_mut(&mut engine.player_info_league_progress_data).insert(93, None);
        let expected = engine.player_ids_in_order();
        {
            let world = engine.host_world_context();
            let clone = world.clone();
            assert_eq!(world.player_ids(), expected);
            assert_eq!(clone.player_ids(), expected);
            for id in [7, 8, 91, 92, 93] {
                assert!(world.player_info_id_known(id));
            }
            assert!(!world.player_info_id_known(0));
            assert_eq!(world.player_info_league_score(91), Some(0));
            assert_eq!(world.player_info_league_score(92), Some(12));
            let edited = world.with_league_scores(Rc::new(BTreeMap::from([(92, 24)])));
            assert_eq!(edited.player_info_league_score(92), Some(24));
            assert_eq!(clone.player_info_league_score(92), Some(12));
        }
        Rc::make_mut(&mut engine.player_info_league_scores).insert(92, 36);
        assert_eq!(
            engine.host_world_context().player_info_league_score(92),
            Some(36)
        );
    }
}
