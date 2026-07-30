//! `impl Engine` — snapshot capture/restore, effects, sections and crew info.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

fn effect_callback_needs_owned_snapshot(effects: &[EffectState], event: &EffectEvent) -> bool {
    matches!(event.kind, EffectEventKind::Stopped(_))
        && !effects
            .iter()
            .any(|effect| effect.number == event.effect.number)
}

impl Engine {
    pub fn snapshot(&self) -> SimulationSnapshot {
        let mut objects = Vec::with_capacity(self.objects.len());
        for object in &self.objects {
            let library = self
                .definitions
                .get(&object.definition_id)
                .map(|definition| definition.action_library());
            objects.push(object.snapshot(library));
        }
        objects.sort_by_key(|object| object.id);
        let mut particles: Vec<_> = self
            .particles
            .iter()
            .map(ActiveParticle::snapshot)
            .collect();
        particles.extend(self.pxs_system.iter_slots().map(|(chunk, slot, pixel)| {
            pxs_snapshot(
                pixel,
                &self.materials,
                Some((chunk * pxs::PXS_CHUNK_SIZE + slot) as u32),
            )
        }));
        particles.extend(
            self.particle_system
                .particles()
                .iter()
                .map(system_particle_snapshot),
        );
        let crew_selection = self.crew_selection_states();
        let crew_roles = self.crew_roles.clone();
        let mut known_crew_owners: Vec<_> = self.known_crew_owners.iter().cloned().collect();
        known_crew_owners.sort_unstable();
        let mut eliminated_crew_owners: Vec<_> =
            self.eliminated_crew_owners.iter().cloned().collect();
        eliminated_crew_owners.sort_unstable();
        let ambient_temperature = self.environment.ambient_temperature(self.frame);
        let sky_color = self.environment.resolved_sky_color(ambient_temperature);
        let has_fog_player = self.players.values().any(Player::fog_of_war);
        let environment = EnvironmentFrame {
            settings: self.environment,
            wind_force: self.environment.wind_force(self.frame),
            ambient_temperature,
            precipitation: self.environment.precipitation(),
            sky_color: Some(sky_color),
            gamma: self.gamma,
            fow_color: if has_fog_player {
                self.scenario_values.fow_color()
            } else {
                0
            },
            fow_resolution: if has_fog_player {
                self.scenario_values.fow_resolution()
            } else {
                DEFAULT_FOW_RESOLUTION
            },
        };
        let sky_snapshot = self.sky.as_ref().map(SkyState::snapshot);
        let weather_events = self.weather_events.clone();
        let mut player_order = self.player_ids_in_order();
        let player_states: Vec<_> = player_order
            .iter()
            .filter_map(|number| self.players.get(number))
            .map(|player| {
                let mut state = player.to_state();
                let owner = player.id();
                state.crew.retain(|id| {
                    self.find_object_index(*id).is_some_and(|index| {
                        let object = &self.objects[index];
                        !object.destroyed && object.state.status != ObjectStatus::Deleted
                    })
                });
                state.cursor = self
                    .crew_selection
                    .get(&owner)
                    .and_then(|selection| selection.cursor())
                    .or(state.cursor);
                if self.eliminated_crew_owners.contains(&owner) {
                    state.status_value = Some(state.exact_status_value());
                    state.eliminated_value = 1;
                    state.status = PlayerStatus::Eliminated;
                } else if state.status == PlayerStatus::Eliminated {
                    state.status = PlayerStatus::Active;
                    state.status_value = None;
                    state.eliminated_value = 0;
                }
                if state.viewports.is_empty() {
                    let focus_id = state
                        .view_cursor
                        .or(state.cursor)
                        .or_else(|| state.crew.first().copied())
                        .or_else(|| {
                            self.objects
                                .iter()
                                .find(|object| object.state.owner == owner)
                                .map(|object| object.id)
                        })
                        .or_else(|| self.objects.first().map(|object| object.id));
                    let mut center = Vector2::ZERO;
                    if let Some(focus) =
                        focus_id.and_then(|id| self.objects.iter().find(|object| object.id == id))
                    {
                        center = focus.state.position;
                        state
                            .viewports
                            .push(PlayerViewport::new(center).with_focus(Some(focus.id)));
                    } else {
                        state.viewports.push(PlayerViewport::new(center));
                    }
                }
                state
            })
            .collect();
        let local_players = player_order
            .iter()
            .copied()
            .filter(|number| {
                self.local_players
                    .as_ref()
                    .is_none_or(|players| players.contains(number))
            })
            .collect();
        let synthetic_owner_start = player_order.len();
        player_order.extend(
            self.known_crew_owners
                .iter()
                .copied()
                .chain(self.eliminated_crew_owners.iter().copied())
                .filter(|owner| !self.players.contains_key(owner)),
        );
        player_order[synthetic_owner_start..].sort_unstable();
        player_order.dedup();
        let mut hud_players = Vec::with_capacity(player_order.len());
        for owner in player_order {
            let mut crew = self.crew_members(owner);
            // The COMPARATOR surface: the bridge std::sort's its HUD crew
            // ascending before export (RustEngineBridge.cpp:1381), so the
            // rust snapshot mirrors that normalization. Engine-internal
            // crew order stays newest-first.
            crew.sort_unstable_by_key(|id| id.as_u64());
            let focus = self
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor());
            let eliminated = self.eliminated_crew_owners.contains(&owner);
            let (wealth, score) = self
                .players
                .get(&owner)
                .map(|player| (player.wealth(), player.points()))
                .unwrap_or((0, 0));
            hud_players.push(HudPlayerSnapshot {
                owner,
                crew,
                focus,
                eliminated,
                wealth,
                score,
            });
        }
        let fow_players = self
            .players
            .iter()
            .filter(|(_, player)| player.fog_of_war())
            .map(|(&id, player)| {
                (
                    id,
                    FogOfWarPlayerFrame {
                        view_objects: player.fow_view_objects().to_vec(),
                        view_target: player.raw_view_target(),
                    },
                )
            })
            .collect();
        let definition_categories = self
            .definitions
            .iter()
            .map(|(id, definition)| (id.clone(), definition.category()))
            .collect();
        let definition_closed_containers = if has_fog_player {
            self.definitions
                .iter()
                .filter_map(|(id, definition)| {
                    let closed = definition.closed_container();
                    (closed != 0).then(|| (id.clone(), closed))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let definition_lines = self
            .definitions
            .iter()
            .filter(|(_, definition)| definition.line() != 0 || definition.line_intersect() != 0)
            .map(|(id, definition)| {
                (
                    id.clone(),
                    DefinitionLineMetadata {
                        line: definition.line(),
                        line_intersect: definition.line_intersect(),
                    },
                )
            })
            .collect();
        let message_snapshots = self.messages.snapshot();
        let transfer_zones = self.transfer_zones.states();
        let mut pathfinder_debug = self.pathfinder_debug.borrow().clone();
        let used_transfer_zones = pathfinder_debug
            .zones
            .iter()
            .filter(|zone| zone.used)
            .map(|zone| zone.owner)
            .collect::<HashSet<_>>();
        // C4PathFinder retains the last ray graph, but Draw walks the live
        // global transfer-zone list. Preserve last-search `Used` markers by
        // owner while reflecting subsequent Set/ClearTransferZone calls.
        pathfinder_debug.zones = transfer_zones
            .iter()
            .map(|zone| PathfinderDebugZone {
                owner: zone.owner,
                x: zone.x,
                y: zone.y,
                width: zone.width,
                height: zone.height,
                used: used_transfer_zones.contains(&zone.owner),
            })
            .collect();
        SimulationSnapshot {
            frame: self.frame,
            game_time: self.game_time,
            game_over: self.game_over_triggered,
            round_results: self.round_results.clone(),
            league_name: self.league_name.as_ref().clone(),
            player_info_league_progress_data: self
                .player_info_league_progress_data
                .as_ref()
                .clone(),
            player_info_league_scores: self.player_info_league_scores.as_ref().clone(),
            physics: Some(self.physics),
            objects,
            render_order: self.exec_list.clone(),
            environment,
            sky: sky_snapshot,
            weather_events,
            global_effects: self.global_effects.clone(),
            script_globals: self.capture_script_globals(),
            particles,
            players: player_states,
            fow_players,
            crew_selection,
            crew_roles,
            known_crew_owners,
            eliminated_crew_owners,
            landscape: self.landscape.clone(),
            rng: self.rng.clone(),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: hud_players,
                messages: message_snapshots,
                scoreboard: self.scoreboard.borrow().clone(),
                scoreboard_presentations: Vec::new(),
                local_players,
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories,
            definition_closed_containers,
            definition_lines,
            transfer_zones,
            pathfinder_debug,
            menu_requests: Vec::new(),
            audio: Vec::new(),
        }
    }

    /// `C4ControlSyncCheck::Set` (C4Control.cpp:445-458): the per-frame
    /// determinism digest. `Random3` is the Rnd3 ring pointer (`FRndPtr3`),
    /// `AllCrewPosX` sums `fixtoi(fix_x, 100)` (centipixels) over the
    /// players' crew lists (C4Control.cpp:460-467), `SectShapeSum` counts the
    /// sector shape lists (C4Sector.cpp:197-203). `MassMoverIndex` remains a
    /// signature hash until the mass-mover gets C++ `CreatePtr` slots.
    pub fn sync_check(&self, by_client: i32) -> SyncCheckPacket {
        let frame = saturating_u64_to_i32(self.frame);
        let crew_positions_sum: i64 = self
            .players
            .values()
            .flat_map(|player| player.crew().iter())
            .filter_map(|id| self.find_object_index(*id))
            .map(|index| i64::from(fixtoi_prec(self.objects[index].fixed_position.x, 100)))
            .sum();
        let pxs_count = i32::try_from(self.pxs_system.execute_count()).unwrap_or(i32::MAX);
        // MassMover.CreatePtr (C4Control.cpp:454)
        let mass_mover_index = self.mass_movers.create_ptr();
        let object_count = i32::try_from(
            self.objects
                .iter()
                .filter(|object| !object.destroyed && object.state.status.is_active())
                .count(),
        )
        .unwrap_or(i32::MAX);
        let object_enumeration_index = saturating_u64_to_i32(self.next_object_id.saturating_sub(1));
        let sector_shape_sum = self
            .sectors
            .as_ref()
            .map(|sectors| i32::try_from(sectors.shape_sum()).unwrap_or(i32::MAX))
            .unwrap_or(0);

        SyncCheckPacket {
            frame,
            control_tick: self.control_tick,
            random3: self.rng.rnd3_ptr(),
            random_count: self.rng.count,
            crew_positions_sum: saturating_i64_to_i32(crew_positions_sum),
            pxs_count,
            mass_mover_index,
            object_count,
            object_enumeration_index,
            sector_shape_sum,
            by_client,
        }
    }

    /// `C4GameControl::Ticks` (C4GameControl.cpp:326-332): advance
    /// ControlTick every ControlRate frames and request a sync check every
    /// SyncRate frames (C4SyncCheckRate = 100, C4GameControl.h:38).
    pub(crate) fn control_ticks(&mut self) {
        if self.frame.is_multiple_of(self.control_rate.max(1) as u64) {
            self.control_tick += 1;
        }
        if self.frame.is_multiple_of(self.sync_rate.max(1) as u64) {
            self.do_sync = true;
        }
    }

    /// `C4GameControl::DoSyncCheck` (C4GameControl.cpp:441-468), run at the
    /// end of the frame (C4Game.cpp:829): build the digest once per DoSync,
    /// keep it in the local queue (the network layer exchanges packets and
    /// feeds foreign ones to `register_remote_sync_check`), drop old entries.
    pub(crate) fn do_sync_check(&mut self) {
        if !self.do_sync {
            return;
        }
        self.do_sync = false;
        let packet = self.sync_check(0);
        if self.get_sync_check(packet.frame).is_none() {
            self.sync_checks.push(packet);
        }
        self.remove_old_sync_checks();
    }

    pub(crate) fn capture_script_globals(&self) -> ScriptGlobalState {
        let numbered = self
            .script_global_slots
            .borrow()
            .iter()
            .map(|(&index, cell)| (index, cell.borrow().clone()))
            .collect();
        let named = self
            .script_globals
            .borrow()
            .iter()
            .map(|(name, cell)| (name.clone(), cell.borrow().clone()))
            .collect();
        ScriptGlobalState { numbered, named }
    }

    pub(crate) fn script_global_name_order(&self) -> Vec<String> {
        self.script_globals.borrow().keys().cloned().collect()
    }

    pub(crate) fn script_string_registration_order(&self) -> Vec<String> {
        clonk_script::c4_string_registration_order(&self.script_string_registrations)
    }

    /// C4AulScriptEngine::DenumerateVariablePointers after every object has
    /// loaded (C4Aul.cpp:506-520; C4Game.cpp:2492): restore numbered slots,
    /// then map saved GlobalNamed values onto the declarations registered by
    /// the fresh engine. Obsolete saved names are dropped and new declarations
    /// stay nil, matching C4ValueMapData::SetNameList.
    pub(crate) fn restore_script_globals(&mut self, state: &ScriptGlobalState) {
        let object_numbers: HashSet<u64> = self
            .objects
            .iter()
            .filter(|object| object.state.status != ObjectStatus::Deleted)
            .map(|object| object.id.as_u64())
            .collect();

        let mut numbered = self.script_global_slots.borrow_mut();
        numbered.clear();
        for (&index, value) in &state.numbered {
            // Serialized C4ValueList positions are always within MaxSize;
            // ignore impossible keyed JSON input rather than creating an
            // unreachable runtime slot.
            if !(0..1_000_000).contains(&index) {
                continue;
            }
            let value = denumerate_script_value(value, &object_numbers);
            clonk_script::register_c4_value_strings(&self.script_string_registrations, &value);
            numbered.insert(index, clonk_script::value_cell(value));
        }
        drop(numbered);

        let named_cells: HashMap<String, clonk_script::ValueCell> = self
            .script_globals
            .borrow()
            .iter()
            .map(|(name, cell)| (name.clone(), cell.clone()))
            .collect();
        for cell in named_cells.values() {
            *cell.borrow_mut() = Value::Nil;
        }
        for (name, value) in &state.named {
            if let Some(cell) = named_cells.get(name) {
                let value = denumerate_script_value(value, &object_numbers);
                clonk_script::register_c4_value_strings(&self.script_string_registrations, &value);
                *cell.borrow_mut() = value;
            }
        }
    }

    /// `C4GameControl::GetSyncCheck` (C4GameControl.cpp:493-506).
    pub fn get_sync_check(&self, frame: i32) -> Option<&SyncCheckPacket> {
        self.sync_checks.iter().find(|check| check.frame == frame)
    }

    /// `C4GameControl::RemoveOldSyncChecks` (C4GameControl.cpp:508-522):
    /// drop checks older than `FrameCounter - C4SyncCheckMaxKeep` (50).
    fn remove_old_sync_checks(&mut self) {
        let cutoff = saturating_u64_to_i32(self.frame) - 50;
        self.sync_checks.retain(|check| check.frame >= cutoff);
    }

    /// `C4ControlSyncCheck::Execute` (C4Control.cpp:469-525) for a sync check
    /// received from another client: compare against the local digest for the
    /// same frame, or queue it until that frame's local check exists. Returns
    /// false on synchronization loss.
    pub fn register_remote_sync_check(&mut self, packet: SyncCheckPacket) -> bool {
        let is_replay = self.replay_control;
        let Some(local) = self.get_sync_check(packet.frame) else {
            self.sync_checks.push(packet);
            return true;
        };
        if is_replay {
            local.matches_replay(&packet)
        } else {
            local.matches(&packet)
        }
    }

    /// C4GameObjects::RemoveSolidMasks around landscape persistence, applied
    /// to a clone so the running world's masks and their buffers stay put.
    /// Rust's exec list is the reverse of C++'s master list, hence `rev()`
    /// reproduces the C4GameObjects First->Next walk (C4GameObjects.cpp:
    /// 296-303). Each removal also runs C4SolidMask's global Last->Prev
    /// overlap repair against cloned buffers. That list includes a mask
    /// retained by runtime deactivation even though its owner is no longer
    /// in the active object list.
    pub(crate) fn landscape_without_solid_masks(&self) -> Option<Landscape> {
        let mut landscape = self.landscape.clone()?;
        let Some(vehicle) = landscape.grid_vehicle_byte() else {
            return Some(landscape);
        };
        let mut bakes = self
            .objects
            .iter()
            .map(|object| object.solid_mask_bake.clone())
            .collect::<Vec<_>>();
        for &id in self.exec_list.iter().rev() {
            let Some(index) = self.find_object_index(id) else {
                continue;
            };
            let object = &self.objects[index];
            if object.state.status != ObjectStatus::Normal {
                continue;
            }
            let Some(removed) = bakes[index].take() else {
                continue;
            };
            removed.restore_background(&mut landscape, vehicle);

            let mut overlapping = bakes
                .iter()
                .enumerate()
                .filter_map(|(other_index, other)| {
                    other.as_ref().and_then(|other| {
                        other
                            .overlaps(&removed)
                            .then_some((other_index, other.instance_sequence))
                    })
                })
                .collect::<Vec<_>>();
            overlapping.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.1));
            for (other_index, _) in overlapping {
                if let Some(other) = bakes[other_index].as_mut() {
                    other.reput_after_removal(&removed, &mut landscape, vehicle);
                }
            }
        }
        Some(landscape)
    }

    /// C4GameObjects::PutSolidMasks after loading a persisted landscape.
    fn put_all_solid_masks(&mut self) {
        let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        for id in master_order {
            let Some(index) = self.find_object_index(id) else {
                continue;
            };
            if self.objects[index].state.status == ObjectStatus::Normal {
                self.put_solid_mask(index);
            }
        }
    }

    /// Snapshot used by C4GameSaveNetwork(false). Its preceding Synchronize
    /// call passes `fSavePlayerFiles=false`, so C4Player::LocalSync does not add
    /// the current Game.Time-GameJoinTime stint to the profile counter.
    pub(crate) fn capture_state_for_network_save(&self) -> EngineState {
        let mut state = self.capture_state();
        for saved in &mut state.players {
            if let Some(player) = self.players.get(&saved.id) {
                saved.total_playing_time = player.total_playing_time();
            }
        }
        state
    }

    pub fn capture_state(&self) -> EngineState {
        let objects = self
            .objects
            .iter()
            .map(|object| {
                let library = self
                    .definitions
                    .get(&object.definition_id)
                    .map(|definition| definition.action_library());
                PersistedObject {
                    snapshot: object.snapshot(library),
                    compiled_mass: self
                        .find_object_index(object.id)
                        .and_then(|index| self.valid_compiled_object_mass(index)),
                    command_queue: object.command_queue.iter().cloned().collect(),
                    command_stack: object.commands.snapshot(),
                    motion_x: object.motion_x,
                    motion_y: object.motion_y,
                    compiler_cache: object.compiler_cache.clone(),
                    last_attach_movement_frame: object.last_attach_movement_frame,
                    no_collect_delay: object.state.no_collect_delay,
                    shape_attach: object.state.shape_attach,
                    entrance_status: object.state.entrance_status,
                    crew_disabled: object.state.crew_disabled,
                    solid_mask_override: object.state.solid_mask_override,
                    shape_vertices: (!object
                        .state
                        .shape_vertices
                        .is_canonical_for(&object.state.vertices))
                    .then(|| object.state.shape_vertices.clone()),
                }
            })
            .collect();

        let crew_selection = self.crew_selection_states();

        let crew_roles = self
            .crew_roles
            .iter()
            .map(|(&owner, roles)| (owner, roles.clone()))
            .collect();

        let mut known_crew_owners: Vec<_> = self.known_crew_owners.iter().cloned().collect();
        known_crew_owners.sort_unstable();
        let mut eliminated_crew_owners: Vec<_> =
            self.eliminated_crew_owners.iter().cloned().collect();
        eliminated_crew_owners.sort_unstable();
        let mut particles: Vec<_> = self
            .particles
            .iter()
            .map(ActiveParticle::snapshot)
            .collect();
        particles.extend(self.pxs_system.iter_slots().map(|(chunk, slot, pixel)| {
            pxs_snapshot(
                pixel,
                &self.materials,
                Some((chunk * pxs::PXS_CHUNK_SIZE + slot) as u32),
            )
        }));
        let player_order = self.player_ids_in_order();
        let players: Vec<_> = player_order
            .iter()
            .map(|number| {
                let player = self
                    .players
                    .get(number)
                    .expect("projected player order contains only live players");
                let mut state = player.to_state();
                if !state.evaluated {
                    let current_stint = self.game_time.wrapping_sub(player.game_join_time());
                    state.total_playing_time = state.total_playing_time.wrapping_add(current_stint);
                }
                // C4Player::ViewTarget is NO-SAVE. Preserve ViewMode and the
                // last ViewX/ViewY center, but never smuggle the transient
                // target through PlayerViewport::focus.
                state.prepare_for_save();
                state
            })
            .collect();
        let joined_player_info_ids = players
            .iter()
            .map(|player| player.player_info_id)
            .filter(|id| *id != 0)
            .collect::<HashSet<_>>();
        let saved_player_info_league_progress_data = self
            .player_info_league_progress_data
            .iter()
            .filter(|(id, _)| joined_player_info_ids.contains(id))
            .map(|(&id, data)| (id, Some(data.clone().unwrap_or_default())))
            .collect();
        let saved_player_info_league_scores = self
            .player_info_league_scores
            .iter()
            .filter(|(id, score)| joined_player_info_ids.contains(id) && **score != 0)
            .map(|(&id, &score)| (id, score))
            .collect();
        let mut round_results = self.round_results.clone();
        round_results.prepare_for_save();

        EngineState {
            frame: self.frame,
            game_time: self.game_time,
            max_players: self.max_players,
            startup_player_count: self.startup_player_count,
            league_name: Some(self.league_name.as_ref().clone()),
            player_info_league_progress_data: Some(saved_player_info_league_progress_data),
            player_info_league_scores: Some(saved_player_info_league_scores),
            use_fair_crew: self.use_fair_crew,
            fair_crew_strength: self.fair_crew_strength,
            fair_crew_forced: Some(self.fair_crew_forced),
            allow_debug: Some(self.allow_debug),
            control_rate: Some(self.control_rate),
            message_board_commands: self.message_board_commands.clone(),
            physics: self.physics,
            environment: self.environment,
            gamma: self.gamma,
            play_list: self.audio_registry.music_playlist().map(str::to_owned),
            music_level: self.audio_registry.music_level(),
            next_object_id: self.next_object_id,
            landscape: self.landscape_without_solid_masks(),
            solid_masks_removed_from_landscape: true,
            scenario_values: Some(self.scenario_values.as_ref().clone()),
            base_reject_entrance_enabled: Some(self.base_reject_entrance_enabled),
            objects,
            object_order: self.exec_list.clone(),
            inactive_object_order: self.inactive_exec_list.clone(),
            particles,
            players,
            player_crew_rosters_authoritative: true,
            last_player_info_id: self.last_player_info_id,
            forced_control_style: self.forced_control_style,
            forced_auto_context_menu: self.forced_auto_context_menu,
            teams: self.team_state.teams.as_ref().clone(),
            team_configuration: Some(self.team_state.team_configuration),
            team_last_team_id: self.team_state.team_last_team_id,
            team_max_script_players: self.team_state.team_max_script_players,
            team_script_player_names: self.team_state.team_script_player_names.clone(),
            team_random_team_count: self.team_state.team_random_team_count,
            crew_selection,
            crew_roles,
            crew_info_rosters: self.crew_rosters.clone(),
            crew_info_order: self.crew_info_order.clone(),
            crew_object_infos: self.crew_object_infos.as_ref().clone(),
            crew_info_links: self.crew_info_links.as_ref().clone(),
            global_effects: self.global_effects.clone(),
            script_globals: self.capture_script_globals(),
            known_crew_owners,
            eliminated_crew_owners,
            transfer_zones: self.transfer_zones.states(),
            messages: self.messages.persisted(),
            pending_menu_requests: self.pending_menu_requests.clone(),
            next_mission: self.next_mission.clone(),
            scoreboard: self.scoreboard.borrow().clone(),
            game_over: self.game_over_triggered,
            round_results,
            landscape_insert_thrust: self.landscape_insert_thrust,
            structures_snow_in: self.structures_snow_in,
            flag_removeable: self.flag_removeable,
            mass_movers: self.mass_movers.clone(),
            sky: self.sky.as_ref().map(SkyState::snapshot),
            rng: self.rng.clone(),
        }
    }

    pub fn restore_state(&mut self, state: &EngineState) -> Result<(), EngineError> {
        // C4Def::pFairCrewPhysical is derived runtime state. A restored game
        // must lazily rebuild it from the restored parameters and RNG epoch.
        self.clear_fair_crew_physicals();
        self.active_message_board_input = None;
        self.host_requests.pending_game_goal_menu_requests.clear();
        self.host_requests
            .network_target_fps_requests
            .borrow_mut()
            .clear();
        self.host_requests
            .viewport_presentation_requests
            .borrow_mut()
            .clear();
        self.film_viewport_available = false;
        self.host_requests
            .player_info_league_progress_updates
            .clear();
        for object in &state.objects {
            if !self
                .definitions
                .contains_key(&object.snapshot.definition_id)
            {
                return Err(EngineError::UnknownDefinition(
                    object.snapshot.definition_id.clone(),
                ));
            }
        }
        // Game.Input is not part of EngineState. A pre-load direct-removal
        // request must not fire against players from the restored game.
        self.host_requests.pending_remove_player_controls.clear();
        // Sound instances are presentation state and are not serialized by
        // C4SoundSystem. Loading a game starts without channels or object
        // bindings from the discarded world.
        self.audio_registry.clear_sound_instances();
        self.pending_audio.retain(|command| {
            matches!(
                command,
                AudioCommand::PlayMusic { .. }
                    | AudioCommand::StopMusic
                    | AudioCommand::SetMusicLevel { .. }
                    | AudioCommand::SetMusicPlaylist { .. }
            )
        });

        self.frame = state.frame;
        self.game_time = state.game_time;
        if let Some(max_players) = state.max_players {
            self.max_players = Some(max_players);
        }
        if let Some(startup_player_count) = state.startup_player_count {
            self.startup_player_count = Some(startup_player_count);
        }
        if let Some(league_name) = &state.league_name {
            self.league_name = Rc::new(legacy_c_string_bytes(league_name.clone()));
        }
        if let Some(progress_data) = &state.player_info_league_progress_data {
            self.player_info_league_progress_data = Rc::new(
                progress_data
                    .iter()
                    .filter(|(id, _)| **id != 0)
                    .map(|(&id, data)| {
                        (
                            id,
                            Some(legacy_c_string_bytes(data.clone().unwrap_or_default())),
                        )
                    })
                    .collect(),
            );
        }
        if let Some(scores) = &state.player_info_league_scores {
            self.player_info_league_scores = Rc::new(
                scores
                    .iter()
                    .filter(|(id, score)| **id != 0 && **score != 0)
                    .map(|(&id, &score)| (id, score))
                    .collect(),
            );
        }
        self.use_fair_crew = state.use_fair_crew;
        self.fair_crew_strength = state.fair_crew_strength;
        if let Some(fair_crew_forced) = state.fair_crew_forced {
            self.fair_crew_forced = fair_crew_forced;
        }
        if let Some(allow_debug) = state.allow_debug {
            self.allow_debug = allow_debug;
        }
        if let Some(control_rate) = state.control_rate {
            self.set_control_rate(control_rate);
        }
        self.message_board_commands = state.message_board_commands.clone();
        self.debug_mode = false;
        // Round setup and clear both start from debug output disabled
        // (C4Game.cpp:640-652).
        clonk_core::log_target::set_debug_mode_presentation(false);
        self.edit_cursor_target = None;
        self.time_go = false;
        let mut physics = state.physics;
        physics.reconcile_raw_gravity();
        self.physics = physics;
        self.environment = state.environment;
        self.environment.refresh_runtime_fields();
        self.gamma = state.gamma;
        let music_playlist = state.play_list.clone();
        self.audio_registry
            .restore_music_playlist(music_playlist.clone());
        self.pending_audio.push(AudioCommand::SetMusicPlaylist {
            playlist: music_playlist,
            restart: false,
        });
        let music_level = self.audio_registry.restore_music_level(state.music_level);
        self.pending_audio
            .push(AudioCommand::SetMusicLevel { level: music_level });
        self.landscape_insert_thrust = state.landscape_insert_thrust;
        self.structures_snow_in = state.structures_snow_in;
        self.flag_removeable = state.flag_removeable;
        self.landscape = state.landscape.clone();
        if let Some(values) = &state.scenario_values {
            self.scenario_values = Rc::new(values.clone());
        }
        if let Some(enabled) = state.base_reject_entrance_enabled {
            self.base_reject_entrance_enabled = enabled;
        }
        // C4MassMoverSet::Load semantics (C4MassMover.cpp:204-217): the
        // saved slots restore verbatim; nothing is re-derived from the
        // landscape.
        self.mass_movers = state.mass_movers.clone();
        if self.landscape.is_none() {
            self.mass_movers.clear();
        }
        // C4Sky::CompileFunc's load half (C4Sky.cpp:248-251); the savegame
        // Init keeps the loaded scroll state (`if (!fSavegame)` reset
        // gate, C4Sky.cpp:77-80). Legacy states without a sky keep the
        // scenario-provided one.
        if let Some(frame) = &state.sky {
            self.sky = Some(SkyState::from_frame(frame));
        }
        self.rng = state.rng.clone();
        self.objects.clear();
        self.exec_list.clear();
        self.inactive_exec_list.clear();
        self.pending_object_order_commands.clear();
        self.resort_any_object = false;
        self.pending_legacy_object_infos.clear();
        self.exec_cursor = None;
        self.note_objects_changed();
        self.crew_rosters = state.crew_info_rosters.clone();
        // Older Rust states predate C4ObjectInfo::WasInAction. Active or
        // already-dead entries prove participation and can be repaired.
        for roster in self.crew_rosters.values_mut() {
            for info in roster {
                info.was_in_action |= info.in_action || info.has_died;
            }
        }
        self.crew_info_order = state.crew_info_order.clone();
        for (&player_id, roster) in &self.crew_rosters {
            let order = self.crew_info_order.entry(player_id).or_default();
            order.retain(|index| *index < roster.len());
            let mut seen = HashSet::new();
            order.retain(|index| seen.insert(*index));
            order.extend((0..roster.len()).filter(|index| seen.insert(*index)));
        }
        self.crew_info_order
            .retain(|player_id, _| self.crew_rosters.contains_key(player_id));
        self.crew_object_infos = Rc::new(state.crew_object_infos.clone());
        self.crew_info_links = Rc::new(state.crew_info_links.clone());
        // Older Rust states already persisted these fields in the roster,
        // but their duplicated live Info projection predates them. The C++
        // object pointer and roster node are one structure, so reconcile the
        // new projection from its linked authoritative node on load.
        for (&object_id, &link) in self.crew_info_links.iter() {
            let Some(entry) = self
                .crew_rosters
                .get(&link.player_id)
                .and_then(|roster| roster.get(link.roster_index))
            else {
                continue;
            };
            if let Some(info) = Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id) {
                info.death_message = entry.death_message.clone();
                info.core = entry.core.clone();
                info.rank_name = entry.rank_name.clone();
                info.participation = entry.participation;
                info.rounds = entry.rounds;
                info.total_playing_time = entry.total_playing_time;
                info.birthday = entry.birthday;
                info.age = entry.age;
                info.in_action_time = entry.in_action_time;
                info.extra_data = entry.extra_data.clone();
                info.portraits = entry.portraits.clone();
            }
        }
        self.crew_info_control_counts.clear();
        self.crew_ranks = Rc::new(
            state
                .crew_object_infos
                .iter()
                .map(|(object, info)| (object.as_u64(), info.rank))
                .collect(),
        );
        self.global_effects = state.global_effects.clone();
        self.particles.clear();
        self.pxs_system.clear();
        self.particle_system.clear_particles();
        for snapshot in &state.particles {
            if snapshot.definition_id.starts_with("material/pxs/") && snapshot.parameter_b >= 0 {
                if let Some(material) = MaterialId::new(snapshot.parameter_b as usize) {
                    // raw C4Fixed state when present (lossless save/load);
                    // float projections only for legacy snapshots
                    let [x, y, xdir, ydir] = snapshot.pxs_fixed.unwrap_or([
                        math::ftofix(snapshot.position.x).val(),
                        math::ftofix(snapshot.position.y).val(),
                        math::ftofix(snapshot.velocity.x).val(),
                        math::ftofix(snapshot.velocity.y).val(),
                    ]);
                    let pixel = pxs::Pxs {
                        mat: material,
                        x: C4Fixed::from_raw(x),
                        y: C4Fixed::from_raw(y),
                        xdir: C4Fixed::from_raw(xdir),
                        ydir: C4Fixed::from_raw(ydir),
                    };
                    // Saved slot position: C4PXSSystem::Load keeps the
                    // chunk layout verbatim (C4PXS.cpp:383-397). Legacy
                    // snapshots without one fall back to New()-style fill.
                    match snapshot.pxs_slot {
                        Some(index) => {
                            let index = index as usize;
                            self.pxs_system.create_at(
                                index / pxs::PXS_CHUNK_SIZE,
                                index % pxs::PXS_CHUNK_SIZE,
                                pixel,
                            );
                        }
                        None => {
                            self.pxs_system
                                .create(material, pixel.x, pixel.y, pixel.xdir, pixel.ydir);
                        }
                    }
                }
                continue;
            }
            if self
                .particle_system
                .get_def(&snapshot.definition_id)
                .is_some()
            {
                self.particle_system.restore_particle(particles::Particle {
                    def_name: snapshot.definition_id.clone(),
                    x: snapshot.position.x,
                    y: snapshot.position.y,
                    xdir: snapshot.velocity.x,
                    ydir: snapshot.velocity.y,
                    life: snapshot.life,
                    a: snapshot.parameter_a,
                    b: snapshot.parameter_b,
                    layer: snapshot.layer.clone(),
                });
                continue;
            }
            self.particles
                .push(ActiveParticle::from_snapshot(snapshot.clone()));
        }
        self.transfer_zones = TransferZoneTable::from_states(&state.transfer_zones);
        self.messages.restore(state.messages.clone());
        self.pending_menu_requests = state.pending_menu_requests.clone();
        self.crew_selection = state
            .crew_selection
            .iter()
            .map(|(&owner, selection)| (owner, CrewSelection::from(selection.clone())))
            .collect();

        let mut container_assignments = Vec::new();
        for persisted in &state.objects {
            let snapshot = &persisted.snapshot;
            let (shape_template, definition_blit_mode, definition_component_order) = {
                let definition =
                    self.definitions
                        .get(&snapshot.definition_id)
                        .ok_or_else(|| {
                            EngineError::UnknownDefinition(snapshot.definition_id.clone())
                        })?;
                (
                    ObjectShapeTemplate::new(
                        definition.shape_vertices().to_vec(),
                        definition.shape_rect(),
                        definition.fire_top(),
                        definition.stretch_growth(),
                        definition.rotateable(),
                    )
                    .with_line(definition.line()),
                    definition.blit_mode(),
                    definition
                        .components()
                        .iter()
                        .map(|component| component.id.clone())
                        .collect::<Vec<_>>(),
                )
            };
            let mut object = Object::new(
                snapshot.id,
                snapshot.definition_id.clone(),
                ObjectState {
                    view_energy: 0,
                    custom_name: snapshot.custom_name.clone(),
                    script_fixed_position: None,
                    script_fixed_velocity: None,
                    script_rotation_velocity: snapshot.rotation_velocity,
                    script_fixed_rotation: snapshot.fixed_rotation,
                    position: snapshot.position,
                    velocity: snapshot.velocity,
                    // Objects.txt `Rotation` loads verbatim (C4Object.cpp:
                    // 2769); only the SetR host function normalizes.
                    rotation: snapshot.rotation,
                    shape_attach: persisted.shape_attach,
                    t_attach: 0,
                    no_collect_delay: persisted.no_collect_delay,
                    // C4Object::CompileFunc persists Base verbatim
                    // (C4Object.cpp:2776); owner validation is a separate
                    // post-load pass (C4Object.cpp:3157-3162).
                    base: snapshot.base,
                    energy: snapshot.energy,
                    need_energy: snapshot.need_energy,
                    construction: snapshot.construction,
                    damage: snapshot.damage,
                    magic_energy: snapshot.magic_energy,
                    magic_capacity: snapshot.magic_capacity,
                    action: snapshot.action.clone(),
                    direction: snapshot.direction,
                    command_direction: snapshot.command_direction,
                    effects: snapshot.effects.clone(),
                    vertices: snapshot.vertices.clone(),
                    shape_vertices: persisted
                        .shape_vertices
                        .clone()
                        .unwrap_or_else(|| ShapeVertexBuffer::from_active(&snapshot.vertices)),
                    contact_density: snapshot.contact_density,
                    container: None,
                    layer: snapshot.layer,
                    visibility: snapshot.visibility,
                    blit_mode: if snapshot.blit_mode == 0 {
                        definition_blit_mode
                    } else {
                        snapshot.blit_mode
                    },
                    picture_rect: snapshot.picture_rect,
                    contents: Vec::new(),
                    contents_link_generation: 0,
                    components: snapshot.components.clone(),
                    component_order: normalized_component_order(
                        &snapshot.components,
                        snapshot.component_order.clone(),
                        &definition_component_order,
                    ),
                    status: snapshot.status,
                    owner: snapshot.owner,
                    controller: snapshot.controller,
                    category: snapshot.category,
                    crew_member: snapshot.crew_member,
                    plr_view_range: snapshot.plr_view_range,
                    selected: snapshot.selected,
                    crew_disabled: persisted.crew_disabled,
                    alive: snapshot.alive,
                    base_graphics: snapshot.base_graphics.clone(),
                    graphics_overlays: snapshot.graphics_overlays.clone(),
                    draw_transform: snapshot.draw_transform,
                    local_vars: snapshot.local_vars.clone(),
                    in_liquid: snapshot.in_liquid,
                    mobile: snapshot.mobile,
                    solid_mask_override: persisted.solid_mask_override,
                    timer: snapshot.timer,
                    own_mass: snapshot.own_mass,
                    on_fire: snapshot.on_fire,
                    fire_phase: snapshot.fire_phase,
                    fire_caused_by: snapshot.fire_caused_by,
                    info_physical: snapshot.info_physical,
                    temporary_physical: snapshot.temporary_physical,
                    physical_changes: snapshot.physical_changes.clone(),
                    breath: snapshot.breath,
                    entrance_status: persisted.entrance_status,
                    menu: None,
                    color: snapshot.color,
                    color_modulation: snapshot.color_modulation,
                    shape_override: snapshot.current_shape,
                    ocf: OCF_NORMAL,
                },
                shape_template,
                snapshot.own_vertices.clone(),
            );
            object.compiled_mass = persisted.compiled_mass;
            if let Some(rect) = snapshot.current_shape {
                object.shape_rect = Some(rect);
            }
            if let Some(fire_top) = snapshot.current_fire_top {
                object.shape_fire_top = fire_top;
            }
            object.motion_x = persisted.motion_x;
            object.motion_y = persisted.motion_y;
            object.compiler_cache = persisted.compiler_cache.clone();
            object.last_attach_movement_frame = persisted.last_attach_movement_frame;
            // Restore authoritative sub-pixel state when the snapshot carried it
            // (whole-pixel objects fall back to the `itofix` set by `Object::new`).
            if let Some(fixed_position) = snapshot.fixed_position {
                object.fixed_position = fixed_position;
                object.state.position = object.position_pixels();
            }
            if let Some(fixed_velocity) = snapshot.fixed_velocity {
                object.fixed_velocity = fixed_velocity;
                object.state.velocity = object.velocity_pixels();
            }
            if let Some(rotation_velocity) = snapshot.rotation_velocity {
                object.rotation_velocity = rotation_velocity;
            }
            if let Some(fixed_rotation) = snapshot.fixed_rotation {
                object.fixed_rotation = fixed_rotation;
            }
            object.last_energy_loss_cause = snapshot.last_energy_loss_cause;
            object.command_queue = VecDeque::from(persisted.command_queue.clone());
            object
                .commands
                .restore_from_snapshot(&persisted.command_stack);
            let restored_id = object.id;
            self.objects.push(object);
            self.note_objects_changed();
            // State restores rebuild the list verbatim like a compiled
            // load (C4ObjectList::CompileFunc, C4ObjectList.cpp:508-530).
            self.insert_into_exec_list(restored_id, true);
            if snapshot.status == ObjectStatus::Inactive {
                self.insert_into_inactive_list(restored_id, true);
            }
            if let Some(container) = snapshot.container {
                container_assignments.push((snapshot.id, container));
            }
        }
        // C4ValueMapData compiles LocalNamed through a temporary saved name
        // list, then C4Object::CompileFunc switches it to the freshly linked
        // definition list and copies matching values by name
        // (C4ValueMap.cpp:163-195,236-293; C4Object.cpp:2815,2858-2865).
        // Numbered Local slots use our reserved keys and are independent.
        let definition_local_names: HashMap<DefinitionId, HashSet<String>> = self
            .definitions
            .iter()
            .map(|(id, definition)| {
                (
                    id.clone(),
                    definition
                        .script
                        .local_variable_names()
                        .map(str::to_string)
                        .collect(),
                )
            })
            .collect();

        // C4Object::DenumeratePointers runs only after every object has loaded
        // and recursively resolves both numbered Local and LocalNamed values
        // (C4Object.cpp:2914-2924; C4Value.cpp:684-713). Inactive objects are
        // valid targets through Game.Objects.InactiveObjects; deleted ones are
        // not part of either lookup list.
        let object_numbers: HashSet<u64> = self
            .objects
            .iter()
            .filter(|object| object.state.status != ObjectStatus::Deleted)
            .map(|object| object.id.as_u64())
            .collect();
        let object_definition_ids = self
            .objects
            .iter()
            .filter(|object| object.state.status != ObjectStatus::Deleted)
            .map(|object| (object.id.as_u64(), object.definition_id.clone()))
            .collect::<HashMap<_, _>>();
        container_assignments.retain(|(object, container)| {
            object_numbers.contains(&object.as_u64())
                && object_numbers.contains(&container.as_u64())
        });
        for object in &mut self.objects {
            denumerate_object_reference(&mut object.state.action.target, &object_numbers);
            denumerate_object_reference(&mut object.state.action.target2, &object_numbers);
            denumerate_object_reference(&mut object.state.layer, &object_numbers);
            if let Some(names) = definition_local_names.get(&object.definition_id) {
                object
                    .state
                    .local_vars
                    .retain(|name, _| name.starts_with("__local_") || names.contains(name));
            }
            for value in object.state.local_vars.values_mut() {
                *value = denumerate_script_value(value, &object_numbers);
            }
            object
                .commands
                .denumerate_object_references(&object_numbers);
            for effect in &mut object.state.effects {
                denumerate_loaded_effect(effect, &object_numbers, &object_definition_ids);
            }
        }
        for effect in &mut self.global_effects {
            denumerate_loaded_effect(effect, &object_numbers, &object_definition_ids);
        }
        // Backward compatibility for states written before ObjectSnapshot
        // carried C4Object::Select: project the legacy per-player list onto
        // the now-authoritative object bits. New states serialize both views
        // consistently, so this OR is idempotent.
        for (&owner, selection) in &state.crew_selection {
            for &id in &selection.selected {
                let roster_membership = state
                    .players
                    .iter()
                    .find(|player| player.id == owner)
                    .filter(|_| state.player_crew_rosters_authoritative)
                    .map(|player| player.crew.contains(&id));
                if let Some(object) = self.objects.iter_mut().find(|object| {
                    object.id == id
                        && roster_membership
                            .unwrap_or(object.state.owner == owner && object.state.crew_member)
                        && !object.destroyed
                        && object.state.status != ObjectStatus::Deleted
                }) {
                    object.state.selected = true;
                }
            }
        }
        if !state.object_order.is_empty() {
            let live: HashSet<ObjectId> = self.exec_list.iter().copied().collect();
            let mut seen = HashSet::with_capacity(live.len());
            let mut restored_order: Vec<ObjectId> = state
                .object_order
                .iter()
                .copied()
                .filter(|id| live.contains(id) && seen.insert(*id))
                .collect();
            restored_order.extend(self.exec_list.iter().copied().filter(|id| seen.insert(*id)));
            self.exec_list = restored_order;
        }
        if !state.inactive_object_order.is_empty() {
            let live: HashSet<ObjectId> = self.inactive_exec_list.iter().copied().collect();
            let mut seen = HashSet::with_capacity(live.len());
            let mut restored_order = state
                .inactive_object_order
                .iter()
                .copied()
                .filter(|id| live.contains(id) && seen.insert(*id))
                .collect::<Vec<_>>();
            restored_order.extend(
                self.inactive_exec_list
                    .iter()
                    .copied()
                    .filter(|id| seen.insert(*id)),
            );
            self.inactive_exec_list = restored_order;
        }
        self.reset_sectors_from_landscape();

        for (object_id, container) in container_assignments {
            self.apply_container_change(object_id, None, Some(container), true)?;
            // Restores denumerate Contained without running Enter — the
            // snapshot controller stays authoritative (no C4Object.cpp:1582
            // transfer on load).
            if let (Some(index), Some(snapshot)) = (
                self.find_object_index(object_id),
                state
                    .objects
                    .iter()
                    .find(|persisted| persisted.snapshot.id == object_id)
                    .map(|persisted| &persisted.snapshot),
            ) {
                self.objects[index].state.controller = snapshot.controller;
            }
        }

        self.restore_script_globals(&state.script_globals);

        // Objects.Load updates faces (and thus masks) before SetOCF and
        // FixObjectOrder (C4GameObjects.cpp:657-663). Snapshot projections
        // and legacy Rust states carry a live baked landscape, so only clean
        // capture_state/section states take this re-put path.
        if state.solid_masks_removed_from_landscape {
            self.put_all_solid_masks();
        }

        // C++ recomputes OCF on load rather than persisting it
        // (C4Object.cpp:2863, savegame SetOCF).
        for index in 0..self.objects.len() {
            self.refresh_object_ocf(index);
        }

        // Restoring a snapshot re-enters C4GameObjects::Load, whose
        // misc-updates pass runs UpdateFlipDir once per still-active object —
        // "for old objects.txt with no flipdir defined"
        // (C4GameObjects.cpp:665-674). Runtime CreateObject deliberately does
        // not get this, so it belongs here and not in the spawn path.
        for index in 0..self.objects.len() {
            if self.objects[index].state.status.is_active() {
                self.update_object_flip_dir(index);
            }
        }

        self.crew_roles = state
            .crew_roles
            .iter()
            .map(|(&owner, roles)| {
                let mut filtered = HashMap::new();
                let roster = state
                    .players
                    .iter()
                    .find(|player| player.id == owner)
                    .filter(|_| state.player_crew_rosters_authoritative)
                    .map(|player| player.crew.as_slice());
                for (&object_id, role) in roles {
                    if let Some(object) = self.objects.iter().find(|object| object.id == object_id)
                    {
                        if roster
                            .map(|roster| roster.contains(&object_id))
                            .unwrap_or(object.state.owner == owner && object.state.crew_member)
                            && !object.destroyed
                            && object.state.status != ObjectStatus::Deleted
                        {
                            filtered.insert(object_id, role.clone());
                        }
                    }
                }
                (owner, filtered)
            })
            .filter(|(_, roles)| !roles.is_empty())
            .collect();

        self.players.clear();
        self.player_order.clear();
        for mut player_state in state.players.iter().cloned() {
            // C4Player::DenumeratePointers resolves Cursor/ViewCursor,
            // Captain, and every message-board callback object, then
            // rebuilds Crew only after the object table is complete
            // (C4Player.cpp:1789-1796). ViewTarget is NO-SAVE; viewport
            // focus remains the presentation projection only.
            denumerate_object_reference(&mut player_state.cursor, &object_numbers);
            denumerate_object_reference(&mut player_state.view_cursor, &object_numbers);
            denumerate_object_reference(&mut player_state.captain, &object_numbers);
            for query in &mut player_state.message_board_queries {
                denumerate_object_reference(&mut query.target, &object_numbers);
            }
            for viewport in &mut player_state.viewports {
                denumerate_object_reference(&mut viewport.focus, &object_numbers);
            }
            player_state.restore_runtime_view();
            player_state
                .crew
                .retain(|id| object_numbers.contains(&id.as_u64()));
            let player = Player::from_state(player_state);
            let number = player.id();
            self.player_order.push(number);
            self.players.insert(number, player);
        }
        if !state.player_crew_rosters_authoritative {
            let mut player_ids: Vec<_> = self.players.keys().copied().collect();
            player_ids.sort_unstable();
            for player_id in player_ids {
                self.bootstrap_player_crew_from_union(player_id);
            }
        }
        for player in self.players.values_mut() {
            player.set_game_join_time(self.game_time);
        }
        if !self.league_name.is_empty() {
            for player_info_id in self
                .players
                .values()
                .map(Player::player_info_id)
                .filter(|id| *id != 0)
            {
                Rc::make_mut(&mut self.player_info_league_progress_data)
                    .entry(player_info_id)
                    .or_insert(None);
            }
        }
        self.last_player_info_id = state
            .last_player_info_id
            .max(
                state
                    .players
                    .iter()
                    .map(|player| player.player_info_id)
                    .chain(
                        state
                            .round_results
                            .players
                            .iter()
                            .map(|player| player.player_info_id),
                    )
                    .max()
                    .unwrap_or(0),
            )
            .max(
                self.player_info_league_progress_data
                    .keys()
                    .next_back()
                    .copied()
                    .unwrap_or(0),
            )
            .max(
                self.player_info_league_scores
                    .keys()
                    .next_back()
                    .copied()
                    .unwrap_or(0),
            )
            .max(0);
        self.forced_control_style = state.forced_control_style;
        self.forced_auto_context_menu = state.forced_auto_context_menu;
        self.team_state.teams = Rc::new(state.teams.clone());
        if let Some(configuration) = state.team_configuration {
            self.set_team_configuration(configuration);
        }
        self.team_state.team_last_team_id = state
            .team_last_team_id
            .max(state.teams.iter().map(|team| team.id).max().unwrap_or(0));
        self.team_state.team_max_script_players = state.team_max_script_players;
        self.team_state.team_script_player_names = state.team_script_player_names.clone();
        self.team_state.team_random_team_count = state.team_random_team_count;
        self.recheck_runtime_team_memberships();
        self.players_registered = !self.players.is_empty();
        self.next_mission = state.next_mission.clone();
        *self.scoreboard.borrow_mut() = state.scoreboard.clone();
        self.game_over_triggered = state.game_over;
        self.game_evaluated = false;
        self.round_results = state.round_results.clone();

        self.known_crew_owners = state.known_crew_owners.iter().cloned().collect();
        self.eliminated_crew_owners = state.eliminated_crew_owners.iter().cloned().collect();

        let highest_id = self
            .objects
            .iter()
            .map(|object| object.id.as_u64())
            .max()
            .unwrap_or(0);
        self.next_object_id = state.next_object_id.max(highest_id + 1);

        self.prune_roles();
        self.prune_selection();
        self.sync_all_player_cursors();
        self.refresh_elimination_state();

        self.fix_exec_list_order();
        self.rebuild_fow_view_objects();
        self.rebuild_sectors();
        Ok(())
    }

    /// Apply the no-callback half of savegame `RecreatePlayers`: runtime
    /// players whose current player-info/client entries were skipped never
    /// exist, but their saved objects remain and are orphaned by the later
    /// `ValidateOwners` pass.
    pub fn retain_restored_players(&mut self, retained_numbers: impl IntoIterator<Item = i32>) {
        let retained_numbers = retained_numbers.into_iter().collect::<HashSet<_>>();
        let removed_numbers = self
            .players
            .keys()
            .copied()
            .filter(|number| !retained_numbers.contains(number))
            .collect::<HashSet<_>>();

        self.players
            .retain(|number, _| retained_numbers.contains(number));
        self.player_order
            .retain(|number| retained_numbers.contains(number));
        self.pending_player_joins
            .retain(|number, _| retained_numbers.contains(number));
        self.crew_selection
            .retain(|number, _| retained_numbers.contains(number));
        self.crew_roles
            .retain(|number, _| retained_numbers.contains(number));
        self.crew_rosters
            .retain(|number, _| retained_numbers.contains(number));
        self.crew_info_order
            .retain(|number, _| retained_numbers.contains(number));
        self.known_crew_owners
            .retain(|number| retained_numbers.contains(number));
        self.eliminated_crew_owners
            .retain(|number| retained_numbers.contains(number));
        if let Some(local_players) = &mut self.local_players {
            local_players.retain(|number| retained_numbers.contains(number));
        }

        let removed_info_objects = self
            .crew_info_links
            .iter()
            .filter_map(|(object, link)| {
                removed_numbers.contains(&link.player_id).then_some(*object)
            })
            .collect::<HashSet<_>>();
        Rc::make_mut(&mut self.crew_info_links)
            .retain(|object, _| !removed_info_objects.contains(object));
        Rc::make_mut(&mut self.crew_object_infos)
            .retain(|object, _| !removed_info_objects.contains(object));
        Rc::make_mut(&mut self.crew_ranks)
            .retain(|object, _| !removed_info_objects.contains(&ObjectId::new(*object)));
        for object in &mut self.objects {
            if removed_info_objects.contains(&object.id) {
                object.state.info_physical = None;
            }
        }

        self.players_registered = !self.players.is_empty();
        self.validate_object_player_references();
        self.prune_roles();
        self.prune_selection();
        self.sync_all_player_cursors();
        self.refresh_elimination_state();
    }

    pub fn restore_snapshot(&mut self, snapshot: &SimulationSnapshot) -> Result<(), EngineError> {
        let state = EngineState::from_snapshot(snapshot);
        self.restore_state(&state)
    }

    /// Runs a batch of effect events for one object and folds every side
    /// channel back into the engine — the frame-loop half of
    /// `pEffects->Execute` (C4Object::Execute, C4Object.cpp:1069-1090).
    pub(crate) fn dispatch_object_effect_events(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        events: Vec<EffectEvent>,
    ) -> Result<(), EngineError> {
        let object_id = self.objects[idx].id;
        let previous_container = self.objects[idx].state.container;
        let global_view = self.global_effects.clone();
        let rng_state = self.rng.clone();
        let world = self.host_world_context_for_object(idx);
        let (
            global_cmds,
            emitted_particles,
            physics_delta,
            audio_events,
            event_messages,
            player_commands,
            object_order_commands,
            next_mission_commands,
            landscape_ops,
            effect_transfer_zones,
            effect_spawns,
            effect_other_objects,
            effect_solid_mask_operations,
            effect_host_raster_preview,
            effect_solid_mask_changed,
            _effect_action_callbacks_dispatched,
            effect_change_def_reinsert,
            effect_next_object_id,
            triggered_game_over,
            effect_script_go,
            effect_script_counter,
            audio_state,
            new_rng,
        ) = {
            let definition = self
                .definitions
                .get(definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            let definitions_ref = &self.definitions;
            let object = &mut self.objects[idx];
            Self::run_effect_events_for_object(
                definition,
                definitions_ref,
                self.game_over_triggered,
                rng_state,
                object_id,
                object,
                events,
                global_view,
                &mut self.environment,
                self.physics,
                self.frame,
                world.clone(),
                self.audio_registry.clone(),
            )?
        };
        let outermost = self.stage_host_solid_mask_operations(
            effect_solid_mask_operations,
            effect_host_raster_preview,
        );
        let fold_result = (|| -> Result<(), EngineError> {
            self.rng = new_rng;
            self.audio_registry = audio_state;
            if effect_solid_mask_changed {
                self.update_solid_mask(idx);
            }
            self.sync_next_object_id(effect_next_object_id);
            if !effect_spawns.is_empty() {
                self.process_spawn_queue(effect_spawns)?;
            }
            if !effect_transfer_zones.is_empty() {
                self.apply_transfer_zone_commands(effect_transfer_zones)?;
            }
            if !effect_other_objects.is_empty() {
                self.apply_nested_object_outcomes(effect_other_objects)?;
            }
            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }
            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }
            self.pending_object_order_commands
                .extend(object_order_commands);
            self.apply_next_mission_commands(next_mission_commands);
            if !audio_events.is_empty() {
                self.pending_audio.extend(audio_events);
            }
            if !event_messages.is_empty() {
                for command in event_messages {
                    self.messages.apply_command(command);
                }
            }
            if let Some(go) = effect_script_go {
                self.scenario_script_go = go;
            }
            if let Some(counter) = effect_script_counter {
                self.scenario_script_counter = counter;
            }
            if triggered_game_over {
                self.request_game_over()?;
            }
            if !physics_delta.is_empty() {
                self.apply_physics_delta(physics_delta);
            }
            if !global_cmds.is_empty() {
                self.apply_global_effect_commands(&global_cmds);
            }
            self.apply_particle_commands(emitted_particles);
            let new_container = self.objects[idx].state.container;
            if previous_container != new_container {
                self.apply_container_change(object_id, previous_container, new_container, false)?;
            }
            if effect_change_def_reinsert.unwrap_or(false) {
                self.reinsert_change_def_contents_link(object_id)?;
            }
            Ok(())
        })();
        self.finish_host_solid_mask_operations(outermost, fold_result)
    }

    pub(crate) fn run_effect_events_for_object(
        definition: &Definition,
        definitions: &HashMap<DefinitionId, Definition>,
        game_over_triggered: bool,
        mut rng: LcgRng,
        object_id: ObjectId,
        object: &mut Object,
        events: Vec<EffectEvent>,
        mut global_view: Vec<EffectState>,
        environment: &mut EnvironmentSettings,
        physics: PhysicsSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<
        (
            Vec<EffectCommand>,
            Vec<ParticleCommand>,
            PhysicsDelta,
            Vec<AudioCommand>,
            Vec<MessageCommand>,
            Vec<PlayerCommand>,
            Vec<ObjectOrderCommand>,
            Vec<NextMissionCommand>,
            Vec<LandscapeOperation>,
            Vec<TransferZoneCommand>,
            Vec<SpawnConfig>,
            Vec<compat::NestedObjectOutcome>,
            Vec<HostSolidMaskOperation>,
            Option<compat::HostRasterPreview>,
            bool,
            bool,
            Option<bool>,
            u64,
            bool,
            Option<bool>,
            Option<i32>,
            AudioRegistry,
            LcgRng,
        ),
        EngineError,
    > {
        if events.is_empty() {
            let next_object_id = world.next_object_id();
            return Ok((
                Vec::new(),
                Vec::new(),
                PhysicsDelta::default(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
                false,
                false,
                None,
                next_object_id,
                false,
                None,
                None,
                audio,
                rng,
            ));
        }

        // Spawns from one callback (CreateContents in an FxStart, the
        // GoldRush bandit equip) must not collide with the next callback's
        // ids: thread the allocator through the loop.
        let mut world = world;
        let mut pending_spawns: Vec<SpawnConfig> = Vec::new();
        let mut queue: VecDeque<EffectEvent> = VecDeque::from(events);
        let mut state_snapshot = object.script_state_snapshot();
        let mut global_commands = Vec::new();
        let mut current_environment = *environment;
        let mut current_physics = physics;
        let mut accumulated_physics = PhysicsDelta::default();
        let mut pending_particles = Vec::new();
        let mut pending_audio = Vec::new();
        let mut pending_messages = Vec::new();
        let mut current_audio = audio;
        let mut pending_player_commands = Vec::new();
        let mut pending_object_order_commands = Vec::new();
        let mut pending_next_mission_commands = Vec::new();
        let mut pending_landscape_ops = Vec::new();
        let mut pending_transfer_zones = Vec::new();
        // Nested-call mutations to OTHER objects (the copy-in/copy-out
        // model's deferred fold; C++ mutates live state mid-call): the
        // CALLER applies them via apply_nested_object_outcomes.
        let mut pending_other_objects = Vec::new();
        let mut pending_solid_mask_operations = Vec::new();
        let mut solid_mask_changed = false;
        let mut action_callbacks_dispatched = false;
        let mut change_def_reinsert = None;
        let mut game_over_requested = false;
        let mut script_go_requested: Option<bool> = None;
        let mut script_counter_requested: Option<i32> = None;
        // Pending-effect bookkeeping is keyed by the effect NUMBER — the
        // C++ identity (C4Effect.cpp:76-78); names may repeat.
        let mut checked_started: HashSet<i32> = HashSet::new();
        let mut denied_started: HashSet<i32> = HashSet::new();
        // C4Fx_Effect_Annul/AnnulCalls (C4Effect.cpp:287-291): pending
        // number -> (acceptor number, AnnulCalls temp-call request). The
        // LAST checker answering -2/-3 wins the merge.
        let mut annulled_started: HashMap<i32, (i32, bool)> = HashMap::new();
        // Anchors whose temp remove/readd bracket was already queued (a
        // re-popped anchor event must not expand again). Start and stop
        // anchors are tracked separately: numbers can be reused within one
        // run after a removal (max existing + 1, C4Effect.cpp:76-78).
        let mut temp_wrapped_started: HashSet<i32> = HashSet::new();
        let mut temp_wrapped_stopped: HashSet<i32> = HashSet::new();

        while let Some(mut event) = queue.pop_front() {
            // A command generated by an earlier callback may have killed a
            // queued callback target. Timer execution normally arrives one
            // live node at a time, but this guard also protects callers that
            // supply an explicit event batch.
            if matches!(event.kind, EffectEventKind::Timer)
                && !object
                    .state
                    .effects
                    .iter()
                    .any(|effect| effect.number == event.effect.number && effect.priority != 0)
            {
                continue;
            }
            // C4Effect::Check (C4Effect.cpp:97-116): before a new effect
            // validates, ask all effects with iPriority >= the new priority
            // via their Fx<Name>Effect callback — except for priority-1
            // effects, which are out of the priority call chain (:170).
            if matches!(event.kind, EffectEventKind::Started) {
                // Fx*Start already ran synchronously inside FnAddEffect
                // (priority-1 effects, C4Effect.cpp:96-152) — do not
                // dispatch it again.
                if event.effect.start_dispatched {
                    continue;
                }
                if denied_started.remove(&event.effect.number) {
                    continue;
                }
                if event.effect.priority != 1 && !checked_started.contains(&event.effect.number) {
                    checked_started.insert(event.effect.number);
                    // C4Effect::Check (C4Effect.cpp:278-282): every OTHER
                    // effect with iPriority >= the new priority is asked —
                    // the walk starts at the new effect's pNext, so only
                    // the effect itself is excluded, never same-name peers.
                    let checkers: Vec<EffectState> = state_snapshot
                        .effects
                        .iter()
                        .filter(|existing| {
                            existing.number != event.effect.number
                                && existing.priority != 0
                                && existing.priority >= event.effect.priority
                        })
                        .cloned()
                        .collect();
                    if !checkers.is_empty() {
                        let pending = event.effect.clone();
                        let constructor_values = event.constructor_values.clone();
                        queue.push_front(event);
                        for checker in checkers.into_iter().rev() {
                            queue.push_front(EffectEvent::check(
                                checker,
                                pending.clone(),
                                constructor_values.clone(),
                            ));
                        }
                        continue;
                    }
                }
                if let Some((acceptor_number, do_temp_calls)) =
                    annulled_started.remove(&event.effect.number)
                {
                    // Add-to-other-effect (C4Effect.cpp:295-313): the new
                    // effect stays dead — no Start, no Stop — and the
                    // acceptor's Fx*Add merges its parameters.
                    object.remove_effect_by_number(event.effect.number);
                    state_snapshot.effects = object.state.effects.clone();
                    if let Some(acceptor) = object
                        .state
                        .effects
                        .iter()
                        .find(|existing| existing.number == acceptor_number)
                        .cloned()
                    {
                        // AnnulCalls (C4Fx_Effect_AnnulCalls,
                        // C4Effects.h:38): the Add runs inside a temp
                        // remove/readd bracket of the effects above the
                        // ACCEPTOR (C4Effect.cpp:297-304).
                        let uppers = if do_temp_calls {
                            upper_effects_of(&object.state.effects, &acceptor)
                        } else {
                            Vec::new()
                        };
                        let mut sequence: Vec<EffectEvent> = uppers
                            .iter()
                            .rev()
                            .cloned()
                            .map(EffectEvent::temp_removed)
                            .collect();
                        sequence.push(EffectEvent::add_to(
                            acceptor,
                            event.effect.clone(),
                            event.constructor_values.clone(),
                        ));
                        sequence.extend(uppers.into_iter().map(EffectEvent::temp_readded));
                        for queued in sequence.into_iter().rev() {
                            queue.push_front(queued);
                        }
                    }
                    continue;
                }
                // C4Effect ctor (C4Effect.cpp:118-133): a validating
                // Fx*Start is bracketed by temp-deactivating all upper
                // effects (high to low) and reactivating them afterwards
                // (low to high) — only when the new effect HAS an Fx*Start
                // and is not priority 1 (`fRemoveUpper && pNext &&
                // pFnStart`, C4Effect.cpp:123). The bracket runs even when
                // the Start then denies (C4Effect.cpp:128-133).
                if event.effect.priority != 1
                    && !temp_wrapped_started.contains(&event.effect.number)
                {
                    let callback_name = format!("Fx{}Start", event.effect.name);
                    let has_start = event.effect.name == C4FX_FIRE
                        || resolve_effect_script_callback(&event.effect, &callback_name, &world)
                            .is_some();
                    if has_start {
                        let uppers = upper_effects_of(&object.state.effects, &event.effect);
                        if !uppers.is_empty() {
                            temp_wrapped_started.insert(event.effect.number);
                            let mut sequence: Vec<EffectEvent> = uppers
                                .iter()
                                .rev()
                                .cloned()
                                .map(EffectEvent::temp_removed)
                                .collect();
                            sequence.push(event);
                            sequence.extend(uppers.into_iter().map(EffectEvent::temp_readded));
                            for queued in sequence.into_iter().rev() {
                                queue.push_front(queued);
                            }
                            continue;
                        }
                    }
                }
                // The constructor has survived Check/annul negotiation and
                // is about to execute its one Start call. Mark the live node
                // before the callback so EffectVar updates cloned from its
                // host snapshot retain the completed-constructor state.
                event.effect.start_dispatched = true;
                if let Some(effect) = object
                    .state
                    .effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number)
                {
                    effect.start_dispatched = true;
                }
                state_snapshot.effects = object.state.effects.clone();
            }
            // C4Effect::Kill (C4Effect.cpp:365-405): the real removal is
            // bracketed by temp-deactivating all upper effects
            // (C4Effect.cpp:370-374) and reactivating them after the Stop
            // (C4Effect.cpp:404). Clear/destroy removals go through
            // ClearAll, which does NO temp calls (C4Effect.cpp:407-425);
            // priority-1 victims skip the bracket (C4Effect.cpp:477).
            if matches!(
                event.kind,
                EffectEventKind::Stopped(EffectStopReason::Removed)
            ) && event.effect.priority != 1
                && !temp_wrapped_stopped.contains(&event.effect.number)
            {
                let uppers = upper_effects_of(&object.state.effects, &event.effect);
                if !uppers.is_empty() {
                    temp_wrapped_stopped.insert(event.effect.number);
                    let mut sequence: Vec<EffectEvent> = uppers
                        .iter()
                        .rev()
                        .cloned()
                        .map(EffectEvent::temp_removed)
                        .collect();
                    sequence.push(event);
                    sequence.extend(uppers.into_iter().map(EffectEvent::temp_readded));
                    for queued in sequence.into_iter().rev() {
                        queue.push_front(queued);
                    }
                    continue;
                }
            }
            let clear_all_stop = matches!(
                event.kind,
                EffectEventKind::Stopped(
                    EffectStopReason::Cleared
                        | EffectStopReason::Death
                        | EffectStopReason::Destroyed
                )
            );
            let death_stop = matches!(
                event.kind,
                EffectEventKind::Stopped(EffectStopReason::Death)
            );
            if clear_all_stop {
                // ClearAll calls SetDead immediately before Fx*Stop, but
                // keeps the node linked so GetEffect(number, include-dead)
                // and the unique-number allocator still see it
                // (C4Effect.cpp:407-424,55-81).
                if let Some(effect) = object
                    .state
                    .effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number)
                {
                    effect.priority = 0;
                    state_snapshot.effects = object.state.effects.clone();
                }
            }

            // TempRemoveUpperEffects flips each live upper node BEFORE its
            // Stop callback; TempReadd walks the then-live suffix and flips
            // only still-inactive nodes before Start. Resolving by number at
            // dispatch time prevents a stale queued readd from resurrecting
            // an effect killed inside its temp Stop.
            match event.kind {
                EffectEventKind::TempRemoved => {
                    let Some(effect) =
                        object.state.effects.iter_mut().find(|effect| {
                            effect.number == event.effect.number && effect.priority > 0
                        })
                    else {
                        continue;
                    };
                    effect.priority = -effect.priority;
                    event.effect = effect.clone();
                    state_snapshot.effects = object.state.effects.clone();
                }
                EffectEventKind::TempReadded => {
                    let Some(effect) =
                        object.state.effects.iter_mut().find(|effect| {
                            effect.number == event.effect.number && effect.priority < 0
                        })
                    else {
                        continue;
                    };
                    effect.priority = -effect.priority;
                    event.effect = effect.clone();
                    state_snapshot.effects = object.state.effects.clone();
                }
                _ => {}
            }
            let owned_snapshot_for_call =
                effect_callback_needs_owned_snapshot(&state_snapshot.effects, &event).then(|| {
                    let mut snapshot = state_snapshot.clone();
                    // C4Effect::Kill/ClearAll keep the dead list node linked
                    // throughout Fx*Stop (C4Effect.cpp:365-424). EffectVar and
                    // GetEffect must therefore still resolve the victim even
                    // though the Rust command fold already removed it.
                    let mut linked = event.effect.clone();
                    if clear_all_stop {
                        linked.priority = 0;
                    }
                    insert_effect_into_stack(&mut snapshot.effects, linked);
                    snapshot
                });
            let snapshot_for_call = owned_snapshot_for_call.as_ref().unwrap_or(&state_snapshot);
            let dispatch_definition = resolve_effect_dispatch_definition(
                &event.effect,
                &world,
                definitions,
                Some((object_id, object.definition_id.as_str())),
                definitions.get(&object.definition_id).unwrap_or(definition),
            );
            // Engine-internal fire callbacks: when no script overload
            // shadows the engine function (AddFunc C4Script.cpp:6994-6996),
            // Fx*Stop clears the OnFire flag — real and temp removals alike
            // (FnFxFireStop, C4Effect.cpp:775-791) — and the temp-readd
            // Fx*Start re-arms it (FnFxFireStart iTemp arm,
            // C4Effect.cpp:563-565).
            if event.effect.name == C4FX_FIRE {
                let engine_native = |callback: &str| -> bool {
                    let callback_name = format!("Fx{C4FX_FIRE}{callback}");
                    resolve_effect_script_callback(&event.effect, &callback_name, &world).is_none()
                };
                match event.kind {
                    EffectEventKind::Stopped(_) | EffectEventKind::TempRemoved
                        if engine_native("Stop") =>
                    {
                        object.state.on_fire = false;
                    }
                    EffectEventKind::TempReadded if engine_native("Start") => {
                        object.state.on_fire = true;
                    }
                    _ => {}
                }
            }
            let mut timer_kill = false;
            let mut start_denied = false;
            let mut stop_denied = false;
            let mut add_denied = false;
            // C++ runs Fx* callbacks with fPassErrors=false: a script
            // error logs and the game continues (the erroring callback
            // yields nil). RNG/audio are restored from the pre-call
            // backups on the error path — the callback's partial outcome,
            // including its RNG draws, is dropped (documented seam: C++
            // keeps mutations made before the error).
            let rng_backup = rng.clone();
            let audio_backup = current_audio.clone();
            let call_result = match event.kind {
                EffectEventKind::Started => dispatch_definition
                    .call_effect_start(
                        Some((snapshot_for_call, object_id)),
                        &event.effect,
                        &event.constructor_values,
                        rng,
                        &global_view,
                        current_physics,
                        current_environment,
                        frame,
                        world.clone(),
                        game_over_triggered,
                        current_audio,
                    )
                    .map(|(outcome, audio_state, new_rng, start_result)| {
                        // C4Fx_Start_Deny (-1, C4Effects.h:43): the effect
                        // is marked dead before validating
                        // (C4Effect.cpp:128-131) and deleted without a
                        // Stop callback.
                        start_denied = start_result
                            .as_ref()
                            .is_some_and(|value| compat::value_as_i32(value) == -1);
                        (outcome, audio_state, new_rng)
                    }),
                EffectEventKind::Timer => dispatch_definition
                    .call_effect_timer(
                        Some((snapshot_for_call, object_id)),
                        &event.effect,
                        frame,
                        rng,
                        &global_view,
                        current_physics,
                        current_environment,
                        world.clone(),
                        game_over_triggered,
                        current_audio,
                    )
                    .map(|(outcome, audio_state, new_rng, timer_result)| {
                        // C4Effect::Execute (C4Effect.cpp:342-360): FxTimer
                        // returning C4Fx_Execute_Kill (-1, C4Effects.h:40)
                        // kills the effect; an elapsed interval with NO
                        // timer function kills too ("no timer function:
                        // mark dead after time elapsed" — the else arm at
                        // :358-360; the intro's Divinity markers die on
                        // their first exec in C++ as well).
                        timer_kill = timer_result
                            .as_ref()
                            .is_none_or(|value| compat::value_as_i32(value) == -1);
                        (outcome, audio_state, new_rng)
                    }),
                EffectEventKind::Stopped(reason) => dispatch_definition
                    .call_effect_stop(
                        Some((snapshot_for_call, object_id)),
                        &event.effect,
                        reason,
                        rng,
                        &global_view,
                        current_physics,
                        current_environment,
                        frame,
                        world.clone(),
                        game_over_triggered,
                        current_audio,
                    )
                    .map(|(outcome, audio_state, new_rng, stop_result)| {
                        // AssignDeath uses ClearAll(RemoveDeath), whose Stop
                        // callbacks may deny removal and revive the object
                        // (C4Object.cpp:1162-1170; C4Effect.cpp:407-424).
                        // Object-clear/destroy removals still cannot veto the
                        // object going away.
                        stop_denied = matches!(
                            reason,
                            EffectStopReason::Removed
                                | EffectStopReason::Cleared
                                | EffectStopReason::Death
                                | EffectStopReason::Destroyed
                        ) && stop_result
                            .as_ref()
                            .is_some_and(|value| compat::value_as_i32(value) == -1);
                        (outcome, audio_state, new_rng)
                    }),
                EffectEventKind::Check { ref pending } => dispatch_definition
                    .call_effect_effect(
                        Some((snapshot_for_call, object_id)),
                        &event.effect,
                        pending,
                        &event.constructor_values,
                        rng,
                        &global_view,
                        current_physics,
                        current_environment,
                        frame,
                        world.clone(),
                        game_over_triggered,
                        current_audio,
                    )
                    .map(|(outcome, audio_state, new_rng, check_result)| {
                        match check_result.as_ref().map(compat::value_as_i32) {
                            // C4Fx_Effect_Deny (-1, C4Effects.h:36) blocks
                            // the new effect entirely.
                            Some(-1) => {
                                denied_started.insert(pending.number);
                                annulled_started.remove(&pending.number);
                                object.remove_effect_by_number(pending.number);
                                state_snapshot.effects = object.state.effects.clone();
                                // The deny returns immediately from the
                                // checker walk (C4Effect.cpp:283-285) —
                                // checkers later in the chain are never
                                // asked.
                                queue.retain(|queued| match &queued.kind {
                                    EffectEventKind::Check {
                                        pending: queued_pending,
                                    } => queued_pending.number != pending.number,
                                    _ => true,
                                });
                            }
                            // C4Fx_Effect_Annul/AnnulCalls (-2/-3,
                            // C4Effects.h:37-38): this checker accepts the
                            // new effect; the walk continues and the LAST
                            // acceptor wins (C4Effect.cpp:287-291).
                            Some(-2) => {
                                annulled_started
                                    .insert(pending.number, (event.effect.number, false));
                            }
                            Some(-3) => {
                                annulled_started
                                    .insert(pending.number, (event.effect.number, true));
                            }
                            _ => {}
                        }
                        (outcome, audio_state, new_rng)
                    }),
                EffectEventKind::AddTo { ref pending } => dispatch_definition
                    .call_effect_add(
                        Some((snapshot_for_call, object_id)),
                        &event.effect,
                        pending,
                        &event.constructor_values,
                        rng,
                        &global_view,
                        current_physics,
                        current_environment,
                        frame,
                        world.clone(),
                        game_over_triggered,
                        current_audio,
                    )
                    .map(|(outcome, audio_state, new_rng, add_result)| {
                        // C4Fx_Start_Deny from Fx*Add kills the ACCEPTOR
                        // (C4Effect.cpp:306-309).
                        add_denied = add_result
                            .as_ref()
                            .is_some_and(|value| compat::value_as_i32(value) == -1);
                        (outcome, audio_state, new_rng)
                    }),
                EffectEventKind::TempRemoved => dispatch_definition
                    .call_effect_stop(
                        Some((snapshot_for_call, object_id)),
                        &event.effect,
                        EffectStopReason::Temp,
                        rng,
                        &global_view,
                        current_physics,
                        current_environment,
                        frame,
                        world.clone(),
                        game_over_triggered,
                        current_audio,
                    )
                    .map(|(outcome, audio_state, new_rng, _temp_result)| {
                        // The temp stop's result is ignored
                        // (C4Effect.cpp:489 does not check it).
                        (outcome, audio_state, new_rng)
                    }),
                EffectEventKind::TempReadded => dispatch_definition
                    .call_effect_temp_readd(
                        Some((snapshot_for_call, object_id)),
                        &event.effect,
                        rng,
                        &global_view,
                        current_physics,
                        current_environment,
                        frame,
                        world.clone(),
                        game_over_triggered,
                        current_audio,
                    )
                    .map(|(outcome, audio_state, new_rng, _temp_result)| {
                        // The temp readd's result is ignored
                        // (C4Effect.cpp:505 does not check it).
                        (outcome, audio_state, new_rng)
                    }),
            };
            let (outcome, audio_state, new_rng) = match call_result {
                Ok(value) => value,
                Err(EngineError::Script {
                    definition,
                    function,
                    source,
                    recovery: _,
                }) => {
                    tracing::debug!(
                        %definition,
                        function,
                        error = %source,
                        "script error in effect callback; continuing like the C++ fail-safe exec"
                    );
                    log_runtime_call_frames(&definition, source.call_frames());
                    rng = rng_backup;
                    current_audio = audio_backup;
                    continue;
                }
                Err(other) => return Err(other),
            };
            rng = new_rng;
            current_audio = audio_state;
            if start_denied {
                object.remove_effect_by_number(event.effect.number);
                state_snapshot.effects = object.state.effects.clone();
            }
            if stop_denied
                && !death_stop
                && !object
                    .state
                    .effects
                    .iter()
                    .any(|effect| effect.number == event.effect.number)
            {
                // Deferred command producers may have unlinked the victim
                // before Stop. Reinsert it before folding the callback's
                // EffectVar updates; timer Kill already has the exact dead
                // node linked and therefore skips this compatibility path.
                object.insert_effect(event.effect.clone());
                state_snapshot.effects = object.state.effects.clone();
            }
            if add_denied {
                // pAddToEffect->Kill (C4Effect.cpp:308): a full Kill with
                // the acceptor's own Stop callback.
                if let Some(removed) = object.remove_effect_by_number(event.effect.number) {
                    queue.push_back(EffectEvent::stopped(removed, EffectStopReason::Removed));
                }
                state_snapshot.effects = object.state.effects.clone();
            }

            let compat::EffectContextOutcome {
                object: object_effect_commands,
                global: mut global_effect_commands,
                object_update,
                object_commands,
                command_operations,
                destroy_object,
                environment: environment_update,
                physics: physics_update,
                landscape: host_landscape_ops,
                solid_mask_operations: event_solid_mask_operations,
                host_raster_preview: event_host_raster_preview,
                particles: mut emitted_particles,
                transfer_zones: event_transfer_zones,
                messages: event_messages,
                player_commands: effect_player_commands,
                object_order_commands: effect_object_order_commands,
                next_mission_commands: effect_next_mission_commands,
                audio: outcome_audio,
                trigger_game_over,
                script_go,
                script_counter,
                context_locals,
                spawns,
                next_object_id,
                other_objects: event_other_objects,
                ..
            } = outcome;

            if let Some(preview) = event_host_raster_preview {
                world.apply_host_raster_preview(preview);
            } else {
                world.preview_solid_mask_operations(&event_solid_mask_operations);
            }
            pending_solid_mask_operations.extend(event_solid_mask_operations);

            // Every effect callback receives a cloned host world. Replay
            // transfer-zone mutations into the threaded copy before the
            // next callback, while retaining their original order for the
            // authoritative Engine fold after this batch returns.
            for command in &event_transfer_zones {
                world.preview_transfer_zone_command(command);
            }
            pending_transfer_zones.extend(event_transfer_zones);

            // The callback ran in this object's own context
            // (C4Effect.cpp:129) — persist its local writes. VM finals
            // apply first; host-command updates below may override.
            if let Some(locals) = context_locals {
                object.state.local_vars = locals;
                state_snapshot = object.script_state_snapshot();
            }

            if !spawns.is_empty() {
                pending_spawns.extend(spawns);
            }
            let mut active_contents_order = None;
            if !event_other_objects.is_empty() {
                for nested in &event_other_objects {
                    if let Some(update) = nested.update.as_ref() {
                        world.preview_object_update(nested.object_id, update);
                    }
                    if nested.destroy {
                        world.preview_object_destroyed(nested.object_id);
                    }
                    for order in &nested.contents_orders {
                        world.preview_contents_order(order.container, &order.contents);
                        if order.container == object_id {
                            active_contents_order = Some(order.contents.clone());
                        }
                    }
                }
                pending_other_objects.extend(event_other_objects);
            }
            world = world.with_next_object_id(next_object_id);

            if !host_landscape_ops.is_empty() {
                pending_landscape_ops.extend(host_landscape_ops);
            }

            if !effect_player_commands.is_empty() {
                pending_player_commands.extend(effect_player_commands);
            }
            pending_object_order_commands.extend(effect_object_order_commands);
            pending_next_mission_commands.extend(effect_next_mission_commands);

            if let Some(update) = environment_update {
                update.apply(&mut current_environment);
            }
            if let Some(update) = physics_update {
                merge_physics_delta(&mut accumulated_physics, &update);
                update.apply(&mut current_physics);
            }

            if let Some(update) = object_update {
                // Later callbacks in this same deferred batch must see the
                // carrier's complete live update. C++ mutates the object in
                // place; in particular, consecutive DigFree callbacks share
                // MaterialContents rather than reseeding from the batch's
                // entry snapshot.
                world.preview_object_update(object_id, &update);
                solid_mask_changed |= update.change_def.is_some()
                    || update.solid_mask_override.is_some()
                    || update.base_graphics.is_some()
                    || update.position.is_some()
                    || update.construction.is_some()
                    || update.container.is_some()
                    || update.rotation.is_some();
                let mut delta = ObjectDelta::default();
                delta.merge_update(update);
                let definition_changed = delta.change_def.is_some();
                let callback_action_library = if let Some(new_def) = delta.change_def.as_deref() {
                    let new_definition = definitions
                        .get(new_def)
                        .ok_or_else(|| EngineError::UnknownDefinition(new_def.to_string()))?;
                    let material_capacity = object.material_contents.len();
                    Self::apply_change_object_def_to_object(
                        object,
                        new_def,
                        new_definition,
                        material_capacity,
                        None,
                    );
                    new_definition.action_library()
                } else {
                    definitions
                        .get(&object.definition_id)
                        .unwrap_or(definition)
                        .action_library()
                };
                if definition_changed {
                    change_def_reinsert = Some(delta.change_def_reinsert);
                }
                let callbacks_dispatched = delta
                    .action
                    .as_ref()
                    .map(|action| action.callbacks_dispatched)
                    .unwrap_or(false);
                action_callbacks_dispatched |= callbacks_dispatched;
                let outcome = object.apply_delta(&delta, callback_action_library);
                if definition_changed {
                    if let Some(current_definition) = definitions.get(&object.definition_id) {
                        let contents_count =
                            Self::host_retained_contents_count(&world, &object.state.contents);
                        object.state.ocf = current_definition
                            .compute_ocf_with_contents_count(&object.state, contents_count);
                    }
                }
                if let Some(change) = outcome.action_change {
                    if !callbacks_dispatched {
                        object.record_action_event(change.previous, ActionTransitionKind::Forced);
                    }
                }
                state_snapshot = object.script_state_snapshot();
            }

            if let Some(contents) = active_contents_order {
                object.state.contents = contents;
                state_snapshot = object.script_state_snapshot();
            }

            if !command_operations.is_empty() {
                object.apply_command_operations(command_operations);
            }

            if !object_commands.is_empty() {
                object.enqueue_commands(object_commands);
            }

            if !object_effect_commands.is_empty() {
                let mut generated = object.apply_effect_commands(&object_effect_commands);
                state_snapshot = object.script_state_snapshot();
                if !generated.is_empty() {
                    queue.extend(generated.drain(..));
                }
            }

            if destroy_object {
                // AssignRemoval already ran ClearAll synchronously inside
                // RemoveObject. Fold its no-callback effect removals before
                // marking the carrier destroyed, so mark_destroyed cannot
                // enqueue duplicate RemoveClear Stops against Deleted state.
                // This mirrors apply_callback_outcome's fold order.
                solid_mask_changed = true;
                object.retired_info_physical = object.state.info_physical;
                object.state.info_physical = None;
                let mut generated = object.mark_destroyed();
                if !generated.is_empty() {
                    queue.extend(generated.drain(..));
                }
            }

            if stop_denied
                && !death_stop
                && !object.destroyed
                && !matches!(object.state.status, ObjectStatus::Deleted)
            {
                // Kill restores iPriority on the same still-linked node
                // after Stop returns. Restore after folding EffectVar
                // updates so writes made while the node was dead survive.
                if let Some(effect) = object
                    .state
                    .effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number)
                {
                    effect.priority = event.effect.priority;
                } else {
                    object.insert_effect(event.effect.clone());
                }
                state_snapshot = object.script_state_snapshot();
            }

            if stop_denied
                && death_stop
                && !object.destroyed
                && !matches!(object.state.status, ObjectStatus::Deleted)
            {
                // ClearAll recovery restores only iPriority on the still
                // linked node. Preserve EffectVar writes made inside Stop,
                // and keep any callback-added effects at their newly
                // allocated numbers (C4Effect.cpp:413-422).
                if let Some(effect) = object
                    .state
                    .effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number)
                {
                    effect.priority = event.effect.priority;
                } else {
                    object.insert_effect(event.effect.clone());
                }
                state_snapshot = object.script_state_snapshot();
            }

            if !global_effect_commands.is_empty() {
                apply_effect_commands_to_stack(&mut global_view, &global_effect_commands);
                global_commands.append(&mut global_effect_commands);
            }

            if !emitted_particles.is_empty() {
                pending_particles.append(&mut emitted_particles);
            }

            if !outcome_audio.events.is_empty() {
                pending_audio.extend(outcome_audio.events);
            }
            if !event_messages.is_empty() {
                pending_messages.extend(event_messages);
            }

            if script_go.is_some() {
                script_go_requested = script_go;
            }
            if script_counter.is_some() {
                script_counter_requested = script_counter;
            }
            if trigger_game_over {
                game_over_requested = true;
            }

            if timer_kill
                && !object.destroyed
                && !matches!(object.state.status, ObjectStatus::Deleted)
            {
                // Execute calls Kill synchronously after the Timer callback
                // has committed all of its mutations. Kill marks this exact
                // node dead but leaves it linked; its Stop/temp bracket must
                // finish before the live cursor advances to the next node.
                if let Some(effect) = object
                    .state
                    .effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number && effect.priority != 0)
                {
                    let stopped = effect.clone();
                    effect.priority = 0;
                    state_snapshot.effects = object.state.effects.clone();
                    queue.push_front(EffectEvent::stopped(stopped, EffectStopReason::Removed));
                }
            }
        }

        *environment = current_environment;

        let next_object_id = world.next_object_id();
        let host_raster_preview =
            (!pending_solid_mask_operations.is_empty()).then(|| world.host_raster_preview());
        Ok((
            global_commands,
            pending_particles,
            accumulated_physics,
            pending_audio,
            pending_messages,
            pending_player_commands,
            pending_object_order_commands,
            pending_next_mission_commands,
            pending_landscape_ops,
            pending_transfer_zones,
            pending_spawns,
            pending_other_objects,
            pending_solid_mask_operations,
            host_raster_preview,
            solid_mask_changed,
            action_callbacks_dispatched,
            change_def_reinsert,
            next_object_id,
            game_over_requested,
            script_go_requested,
            script_counter_requested,
            current_audio,
            rng,
        ))
    }

    pub(crate) fn apply_physics_delta(&mut self, delta: PhysicsDelta) {
        if delta.is_empty() {
            return;
        }
        let mut physics = self.physics;
        delta.apply(&mut physics);
        self.set_physics(physics);
    }

    pub(crate) fn update_selection_for_state_change(
        &mut self,
        _object_id: ObjectId,
        _previous_owner: i32,
        _previous_crew_member: bool,
        _new_owner: i32,
        _new_crew_member: bool,
    ) {
        // Owner, StatusDeactivate(false), and an individual player's crew
        // membership changes do not implicitly clear C4Object::Select or any
        // player pointer. The concrete C++ operations that do clear pointers
        // (death, deletion, StatusDeactivate(true), GrabInfo) handle them at
        // their exact ordered call sites.
    }

    fn remove_from_selection(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(object) = self
            .objects
            .iter_mut()
            .find(|object| object.id == object_id)
        {
            object.state.selected = false;
        }
        if self.crew_cursor(owner) == Some(object_id) {
            let replacement = self.selected_crew(owner).last().copied();
            if let Some(selection) = self.crew_selection.get_mut(&owner) {
                selection.cursor = replacement;
            }
        }
        if self
            .crew_selection
            .get(&owner)
            .is_some_and(CrewSelection::is_empty)
        {
            self.crew_selection.remove(&owner);
        }
        self.sync_player_cursor(owner);
    }

    pub(crate) fn remove_from_roles(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(assignments) = self.crew_roles.get_mut(&owner) {
            assignments.remove(&object_id);
            if assignments.is_empty() {
                self.crew_roles.remove(&owner);
            }
        }
        self.sync_player_cursor(owner);
    }

    pub(crate) fn sync_player_cursor(&mut self, owner: i32) {
        if let Some(player) = self.players.get_mut(&owner) {
            let cursor = self
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor());
            player.set_cursor(cursor);
        }
    }

    pub(crate) fn sync_all_player_cursors(&mut self) {
        let owners: Vec<i32> = self.players.keys().copied().collect();
        for owner in owners {
            self.sync_player_cursor(owner);
        }
    }

    fn scenario_section_object_spawns(
        &self,
        section: &RuntimeScenarioSection,
    ) -> Result<Vec<scenario::ScenarioSpawn>, EngineError> {
        let group = if let Some(payload) = section.frozen_group.as_ref() {
            Some(
                clonk_resources::Group::from_raw_memory(
                    std::path::PathBuf::from(format!("Sect{}.c4g", section.name)),
                    payload.clone(),
                )
                .map_err(|error| EngineError::ScenarioSectionObjects {
                    section: section.name.clone(),
                    detail: error.to_string(),
                })?,
            )
        } else {
            section.source_group.clone()
        };
        let Some(group) = group else {
            // Unit/synthetic sections have no C4Group to reopen. Preserve
            // their explicit templates as the test-fixture equivalent of an
            // Objects.txt component.
            return Ok(section.initial_objects.clone());
        };
        if section.source_group.is_none() && !group.exists("Objects.txt") {
            // A landscape-only freeze of a synthetic section creates a temp
            // group without an object component. Real named sections copied
            // their original Objects.txt into that group; only fixtures need
            // to fall back to the explicit templates here.
            return Ok(section.initial_objects.clone());
        }

        let definition_ids = self
            .definitions
            .keys()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let retained_object_numbers = self
            .objects
            .iter()
            .filter(|object| object.state.status != ObjectStatus::Deleted)
            .map(|object| object.id.as_u64())
            .collect::<HashSet<_>>();
        scenario::collect_legacy_objects_with_definition_ids(
            &group,
            &definition_ids,
            &self.legacy_string_table,
            &retained_object_numbers,
        )
        .map_err(|error| EngineError::ScenarioSectionObjects {
            section: section.name.clone(),
            detail: error.to_string(),
        })
    }

    fn spawn_scenario_section_objects(
        &mut self,
        mut pending: Vec<scenario::ScenarioSpawn>,
    ) -> Result<(), EngineError> {
        // Inactive cross-section objects keep their enumeration numbers. If
        // an original section object reused one, C++'s loader resolves the
        // live inactive object first; dropping that one colliding template
        // entry is the safe equivalent until section renumeration is modeled.
        let retained_ids = self
            .objects
            .iter()
            .map(|object| object.id)
            .collect::<HashSet<_>>();
        pending.retain(|spawn| spawn.config.id.is_none_or(|id| !retained_ids.contains(&id)));
        let contents_specs = pending
            .iter()
            .filter(|spawn| !spawn.contents_handles.is_empty())
            .map(|spawn| (spawn.handle.clone(), spawn.contents_handles.clone()))
            .collect::<Vec<_>>();

        let max_explicit_id = pending
            .iter()
            .filter_map(|spawn| spawn.config.id)
            .map(ObjectId::as_u64)
            .max();
        if let Some(max_id) = max_explicit_id {
            self.next_object_id = self.next_object_id.max(max_id.saturating_add(1));
        }

        let mut handles = retained_ids
            .iter()
            .map(|id| (id.as_u64().to_string(), *id))
            .collect::<HashMap<String, ObjectId>>();
        while !pending.is_empty() {
            let ready = pending.iter().position(|spawn| {
                spawn
                    .container_handle
                    .as_ref()
                    .is_none_or(|handle| handles.contains_key(handle))
            });
            let index = match ready {
                Some(index) => index,
                None => {
                    // Missing/cyclic containers denumerate to null in the
                    // existing initial-scenario loader. Break one edge and
                    // let the same deterministic file-order loop continue.
                    pending[0].container_handle = None;
                    0
                }
            };
            let mut spawn = pending.remove(index);
            if let Some(container) = spawn
                .container_handle
                .as_ref()
                .and_then(|handle| handles.get(handle))
                .copied()
            {
                spawn.config = spawn.config.with_container(container);
            }
            let id = self.spawn_object(spawn.config)?;
            if let Some(handle) = spawn.handle {
                handles.insert(handle, id);
            }
        }
        let contents_orders = contents_specs
            .into_iter()
            .filter_map(|(parent, children)| {
                let parent = parent.and_then(|handle| handles.get(&handle).copied())?;
                let children = children
                    .iter()
                    .filter_map(|handle| handles.get(handle).copied())
                    .collect::<Vec<_>>();
                Some((parent, children))
            })
            .collect::<Vec<_>>();
        self.restore_legacy_contents_order(&contents_orders);
        self.finish_legacy_object_load();
        Ok(())
    }

    pub(crate) fn load_scenario_section(
        &mut self,
        name: &str,
        flags: i32,
        preserve_ids: Vec<ObjectId>,
    ) -> Result<bool, EngineError> {
        if !self.scenario_current_section_registered {
            // C4Game::LoadScenarioSection creates the implicit current/root
            // node before it even looks up the requested target. Its
            // constructor prepends the node to Game.pScenarioSections.
            let current = self.current_scenario_section.to_ascii_lowercase();
            if self.scenario_sections.contains_key(&current) {
                self.scenario_section_order
                    .retain(|section| section != &current);
                self.scenario_section_order.insert(0, current);
            }
            self.scenario_current_section_registered = true;
        }
        let key = name.to_ascii_lowercase();
        let Some(target) = self.scenario_sections.get(&key) else {
            return Ok(false);
        };

        // C4ScenarioSection::GetGroupfile can reopen an implicit scenario
        // root only while it is named Main. A resumed network save may bind
        // that implicit root to CurrentScenarioSection (for example Cave);
        // it has neither a child Filename nor a group it can reopen after
        // departure. EnsureTempStore, reached when either save flag is set,
        // gives it the temporary group represented by frozen_group.
        if target.source_is_scenario_root
            && !target.name.eq_ignore_ascii_case("main")
            && target.frozen_group.is_none()
        {
            return Ok(false);
        }

        let preserved = preserve_ids.into_iter().collect::<HashSet<_>>();
        let departing_pxs = self.pxs_system.clone();
        let departing_mass_movers = self.mass_movers.clone();
        let departing_key = self.current_scenario_section.to_ascii_lowercase();
        let changing_section = key != departing_key;
        // Objects.Save enumerates both active and inactive lists even though
        // a section file decompiles active non-player objects only. Capture
        // the enumerated wrappers for the frozen Objects.txt, then perform
        // native Denumerate immediately so surviving inactive objects keep
        // the same refreshed caches and normalized live pointers.
        let object_enumeration = (changing_section && flags & 2 != 0)
            .then(|| self.enumerate_object_compiler_caches_for_save());
        if let Some(enumeration) = object_enumeration.as_ref() {
            self.denumerate_object_compiler_caches_after_save(enumeration);
        }
        // Capture after Denumerate: preserved objects are restored from this
        // state later in the switch and must not resurrect an off-list live
        // pointer that native Denumerate just cleared. The refreshed number
        // caches survive Denumerate and remain available to Objects.txt.
        let mut state = self.capture_state();
        let saved_landscape_systems =
            (changing_section && flags & 1 != 0).then(|| scenario::ScenarioLandscapeSystems {
                pxs: self.pxs_system.to_c4b().map(|bytes| {
                    pxs::PxsSystem::from_c4b(&bytes)
                        .expect("an engine-produced PXS component must reload")
                }),
                mass_movers: self.mass_movers.to_c4b().map(|bytes| {
                    MassMoverSet::from_c4b(&bytes)
                        .expect("an engine-produced MassMover component must reload")
                }),
            });
        let mut saved_section_landscape =
            (changing_section && flags & 1 != 0).then(|| self.landscape_without_solid_masks());
        if changing_section && flags & 3 != 0 {
            let saved_objects = (flags & 2 != 0).then(|| {
                state
                    .objects
                    .iter()
                    .filter(|object| object.snapshot.status.is_active())
                    .filter(|object| !preserved.contains(&object.snapshot.id))
                    .filter(|object| !self.is_user_player_object_snapshot(&object.snapshot))
                    .cloned()
                    .collect::<Vec<_>>()
            });
            let saved_object_ids = saved_objects
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|object| object.snapshot.id)
                .collect::<HashSet<_>>();
            let saved_order = if flags & 2 != 0 {
                state
                    .object_order
                    .iter()
                    .copied()
                    .filter(|id| saved_object_ids.contains(id))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let mut current = self
                .scenario_sections
                .get(&departing_key)
                .cloned()
                .expect("departing section remains registered");
            current.modified = true;
            if flags & 1 != 0 {
                current.landscape_modified = true;
                // C4GameObjects::RemoveSolidMasks runs before a section
                // landscape is persisted. Keep the running raster intact and
                // retain the same native persistence clone used by root
                // saves, including linked runtime-inactive mask survivors.
                current.landscape = saved_section_landscape
                    .take()
                    .expect("landscape-save state captured above");
                if let Some(landscape) = current
                    .landscape
                    .as_mut()
                    .filter(|landscape| landscape.pixel_grid().is_some())
                {
                    // Reloading a section recreates pInitial from the full
                    // saved Surface8. Without this, the next SaveDiff would
                    // compare against the pre-departure scenario baseline.
                    landscape
                        .save_initial()
                        .expect("a present section pixel grid can seed pInitial");
                }
                current.exact_landscape = true;
                current.texmap_lookups.clear();
                current.resynthesize_static_map = false;
                current.s2_overload = None;
                current.map_creator = None;
                // A saved section landscape reloads as ExactLandscape;
                // C++ has no S2 creator or callback masks to replay.
                current.post_init_map_callbacks = map_creator_s2::PostInitMapCallbacks::default();
                if let Some(raster) = current
                    .landscape
                    .as_mut()
                    .and_then(Landscape::raster_state_mut)
                {
                    raster.set_map_creator(None);
                }
                current.scenario_values = self.scenario_values.as_ref().clone();
                current.base_reject_entrance_enabled = self.base_reject_entrance_enabled;
                current.base_extinguish_enabled = self.base_extinguish_enabled;
                current.environment = self.environment;
                current.landscape_systems = saved_landscape_systems
                    .clone()
                    .expect("landscape-save state captured above");
            }
            if let Some(objects) = saved_objects {
                current.objects_modified = true;
                current.saved_objects = Some(objects);
                current.saved_object_order = saved_order;
            }

            // Native LoadScenarioSection writes the departing categories to
            // C4ScenarioSection's temporary group at this exact boundary.
            // Preserve the packed image now; final C4GameSave merely copies
            // it and must not observe any subsequent live-state changes.
            let frozen_group = live_c4_save::freeze_scenario_section(
                self,
                &current,
                flags & 1 != 0,
                flags & 2 != 0,
            )
            .map_err(|error| EngineError::ScenarioSectionSave {
                section: self.current_scenario_section.clone(),
                detail: error.to_string(),
            })?;
            current.frozen_group = Some(frozen_group);
            // The persisted snapshots are only scratch input to Objects.Save.
            // Native deletes the departing live objects after producing the
            // temporary group; retaining these structured copies would keep
            // their C4String references registered outside the raw section.
            current.saved_objects = None;
            current.saved_object_order.clear();
            if flags & 1 != 0 {
                // C4Landscape::Save writes a changed retained Map.bmp before
                // the exact section is reloaded. Exact Init then owns no Map,
                // so clear only the post-freeze in-memory representation.
                if let Some(landscape) = current.landscape.as_mut() {
                    landscape.clear_retained_map();
                }
            }
            self.scenario_sections.insert(departing_key, current);
        }

        let target = self
            .scenario_sections
            .get(&key)
            .cloned()
            .expect("section presence checked above");
        let target_landscape_systems = target.landscape_systems.clone();
        let mut post_init_map_callbacks = map_creator_s2::PostInitMapCallbacks::default();
        let keep_map_creator = target.keep_map_creator;
        let retained = state
            .objects
            .iter()
            .filter(|object| preserved.contains(&object.snapshot.id))
            .cloned()
            .collect::<Vec<_>>();
        let retained_order = state
            .object_order
            .iter()
            .copied()
            .filter(|id| preserved.contains(id))
            .collect::<Vec<_>>();

        // C4Landscape::Init repairs an unset persistent MapSeed before its
        // first FixRandom. The draw is deliberately made on the pre-reset
        // game ledger; FixRandom immediately discards its resulting state.
        if state
            .landscape
            .as_ref()
            .is_some_and(|landscape| landscape.map_seed() == 0)
        {
            let map_seed = self.rng.random(3_133_700);
            if let Some(landscape) = state.landscape.as_mut() {
                landscape.set_map_seed(map_seed);
            }
        }

        // C4Landscape::Init brackets section landscape creation with two
        // unconditional FixRandom calls. RuntimeScenarioSection landscapes
        // are built eagerly with the persistent initial MapSeed, so the
        // prepared-landscape swap below is the corresponding creation span
        // (C4Landscape.cpp:564,579,735; C4Game.cpp:2642-2657).
        let persistent_landscape = state.landscape.as_ref().map(|landscape| {
            (
                landscape.map_seed(),
                landscape.modulation(),
                landscape.mode(),
                landscape
                    .raster_state()
                    .map(landscape::LandscapeRasterState::map_zoom),
                landscape.raster_state().map(|state| state.texmap().clone()),
                landscape
                    .raster_state()
                    .and_then(landscape::LandscapeRasterState::map_creator)
                    .cloned(),
            )
        });
        self.fix_random();
        state.rng = self.rng.clone();
        let mut landscape_loaded = target.landscape.is_some();
        let mut runtime_s2_handled = false;
        if let Some(spec) = target.s2_overload.as_ref() {
            let live_texmap = persistent_landscape
                .as_ref()
                .and_then(|(_, _, _, _, texmap, _)| texmap.clone())
                .or_else(|| {
                    target
                        .landscape
                        .as_ref()
                        .and_then(Landscape::raster_state)
                        .map(|state| state.texmap().clone())
                });
            if let Some(live_texmap) = live_texmap {
                runtime_s2_handled = true;
                let map_seed = persistent_landscape
                    .as_ref()
                    .map(|(seed, _, _, _, _, _)| *seed)
                    .or_else(|| target.landscape.as_ref().map(Landscape::map_seed))
                    .unwrap_or_default();
                let modulation = persistent_landscape
                    .as_ref()
                    .map(|(_, modulation, _, _, _, _)| *modulation)
                    .or_else(|| target.landscape.as_ref().map(Landscape::modulation))
                    .unwrap_or(0xffff_ffff);
                let previous_mode = persistent_landscape
                    .as_ref()
                    .map(|(_, _, mode, _, _, _)| *mode)
                    .unwrap_or(LANDSCAPE_MODE_UNDEFINED);
                let live_creator = persistent_landscape
                    .as_ref()
                    .and_then(|(_, _, _, _, _, creator)| creator.clone());
                let mut classifier = scenario::MapPixelClassifier::from_runtime_state(live_texmap);
                let creation = {
                    let mut script_algo = |rng: &mut LcgRng, function: &str, args: [i32; 4]| {
                        self.call_map_script_algorithm(rng, function, args)
                    };
                    map_creator_s2::create_s2_map_for_section_with_state_and_functions_with_script_algo(
                        live_creator,
                        &spec.source,
                        &mut classifier,
                        spec.map_width,
                        spec.map_height,
                        spec.map_player_extend,
                        spec.player_count,
                        &mut state.rng,
                        &spec.script_functions,
                        &mut script_algo,
                    )
                };
                // restore_state below installs the pending target section.
                // Keep the live Game.Script mutations made while its map was
                // rendered instead of restoring the pre-render snapshot.
                state.script_globals = self.capture_script_globals();
                let runtime_texmap = classifier.into_runtime_state();
                if let Some(bitmap) = creation.bitmap.as_ref() {
                    let map_zoom = spec.map_zoom.evaluate(&mut state.rng) as u32 as i32;
                    let mut landscape = scenario::classified_landscape(
                        bitmap,
                        &scenario::MapPixelClassifier::from_runtime_state(runtime_texmap),
                        map_zoom,
                        map_seed,
                    )
                    .map_err(|error| {
                        EngineError::InvalidScenarioSectionLandscape(error.to_string())
                    })?;
                    landscape.save_initial().map_err(|error| {
                        EngineError::InvalidScenarioSectionLandscape(error.to_string())
                    })?;
                    if let Some(diff) = spec.diff.as_ref() {
                        landscape.apply_diff(diff).map_err(|error| {
                            EngineError::InvalidScenarioSectionLandscape(error.to_string())
                        })?;
                    }
                    landscape.set_shade_materials(spec.shade_materials);
                    landscape.set_no_scan(spec.no_scan);
                    landscape.set_border_open(
                        spec.left_open,
                        spec.right_open,
                        spec.top_open,
                        spec.bottom_open,
                    );
                    if spec.auto_scan_side_open {
                        landscape.scan_side_open();
                    }
                    landscape.set_modulation(modulation);
                    let mut creator = creation.creator;
                    creator.set_callback_map_zoom(map_zoom);
                    let retained_creator = if keep_map_creator {
                        post_init_map_callbacks = creator.callback_state();
                        Some(creator)
                    } else {
                        None
                    };
                    landscape
                        .raster_state_mut()
                        .expect("classified section landscapes carry raster state")
                        .set_map_creator(retained_creator);
                    let mode =
                        if target.exact_landscape && previous_mode == LANDSCAPE_MODE_UNDEFINED {
                            LANDSCAPE_MODE_EXACT
                        } else {
                            LANDSCAPE_MODE_UNDEFINED
                        };
                    landscape.set_runtime_mode(mode);
                    state.landscape = Some(landscape);
                    landscape_loaded = true;
                } else {
                    landscape_loaded = false;
                    if let Some(landscape) = state.landscape.as_mut() {
                        if !landscape.replace_runtime_texmap_state(runtime_texmap.clone()) {
                            let mut raster =
                                landscape::LandscapeRasterState::new(1, map_seed, runtime_texmap);
                            raster.set_map_creator(Some(creation.creator));
                            landscape.set_raster_state(raster);
                        } else if let Some(raster) = landscape.raster_state_mut() {
                            raster.set_map_creator(Some(creation.creator));
                        }
                    }
                }
            }
        }
        if !runtime_s2_handled && landscape_loaded {
            state.landscape = target.landscape.clone();
            if let Some(landscape) = state.landscape.as_mut() {
                if let Some((
                    map_seed,
                    modulation,
                    previous_mode,
                    persistent_map_zoom,
                    texmap,
                    live_creator,
                )) = persistent_landscape.clone()
                {
                    landscape.set_map_seed(map_seed);
                    landscape.set_modulation(modulation);
                    if target.exact_landscape {
                        if let Some(raster) = landscape.raster_state_mut() {
                            raster.set_map_zoom(persistent_map_zoom.unwrap_or(0));
                        }
                    }
                    if let Some(texmap) = texmap {
                        if target.texmap_lookups.is_empty() {
                            landscape.replace_runtime_texmap_state_and_repaint(texmap);
                        } else {
                            landscape
                                .merge_runtime_texmap_for_section(texmap, &target.texmap_lookups);
                        }
                    }
                    if target.resynthesize_static_map {
                        landscape.resynthesize_retained_map_for_section();
                    }
                    let prepared_creator = landscape
                        .raster_state()
                        .and_then(landscape::LandscapeRasterState::map_creator)
                        .cloned()
                        .or_else(|| target.map_creator.clone());
                    let mut creator = if keep_map_creator {
                        match (live_creator, prepared_creator) {
                            (Some(mut live), Some(prepared)) => {
                                live.append_from(&prepared);
                                Some(live)
                            }
                            (Some(live), None) => Some(live),
                            (None, Some(prepared)) => Some(prepared),
                            (None, None) => None,
                        }
                    } else {
                        None
                    };
                    if let Some(creator) = creator.as_mut() {
                        if let Some(map_zoom) = landscape
                            .raster_state()
                            .map(landscape::LandscapeRasterState::map_zoom)
                        {
                            creator.set_callback_map_zoom(map_zoom);
                        }
                        post_init_map_callbacks = creator.callback_state();
                    }
                    if let Some(state) = landscape.raster_state_mut() {
                        state.set_map_creator(creator);
                    }
                    let mode =
                        if target.exact_landscape && previous_mode == LANDSCAPE_MODE_UNDEFINED {
                            LANDSCAPE_MODE_EXACT
                        } else {
                            LANDSCAPE_MODE_UNDEFINED
                        };
                    landscape.set_runtime_mode(mode);
                }
            }
        } else if !runtime_s2_handled {
            let mut prepared_creator = target.map_creator.clone();
            if let Some(landscape) = state.landscape.as_mut() {
                if !target.texmap_lookups.is_empty() {
                    if let Some(remap) =
                        landscape.replay_empty_section_texmap_lookups(&target.texmap_lookups)
                    {
                        if let Some(creator) = prepared_creator.as_mut() {
                            creator.remap_material_colors(&remap);
                        }
                    }
                }
                let live_creator = persistent_landscape
                    .as_ref()
                    .and_then(|(_, _, _, _, _, creator)| creator.clone());
                let creator = match (live_creator, prepared_creator) {
                    (Some(mut live), Some(prepared)) => {
                        live.append_from(&prepared);
                        Some(live)
                    }
                    (Some(live), None) => Some(live),
                    (None, Some(prepared)) => Some(prepared),
                    (None, None) => None,
                };
                if let Some(state) = landscape.raster_state_mut() {
                    state.set_map_creator(creator);
                }
            }
        }
        if let Some(landscape) = state.landscape.as_mut() {
            landscape.set_map_changed();
        }
        state.scenario_values = Some(target.scenario_values.clone());
        state.base_reject_entrance_enabled = Some(target.base_reject_entrance_enabled);
        state.environment = target.environment;
        state.global_effects.clear();
        state.particles.clear();
        state.transfer_zones.clear();
        state.mass_movers.clear();

        state.objects.clear();
        state.objects.extend(retained);
        state.object_order = retained_order;

        self.base_extinguish_enabled = target.base_extinguish_enabled;
        self.restore_state(&state)?;
        match target_landscape_systems.pxs {
            Some(mut pxs) => {
                // C4PXSSystem::Load clears chunks but deliberately leaves the
                // public per-Execute Count ledger unchanged.
                pxs.set_execute_count(departing_pxs.execute_count());
                self.pxs_system = pxs;
            }
            None if landscape_loaded => self.pxs_system.clear(),
            None => self.pxs_system = departing_pxs,
        }
        match target_landscape_systems.mass_movers {
            Some(mass_movers) => self.mass_movers = mass_movers,
            None if landscape_loaded => self.mass_movers.clear(),
            None => self.mass_movers = departing_mass_movers,
        }
        // Objects.Load follows Landscape.Init's second FixRandom. Keep the
        // same boundary before fresh section objects run Construction or
        // Initialize callbacks.
        if landscape_loaded {
            self.fix_random();
            let gravity = self.evaluate_scenario_gravity(target.gravity);
            let mut physics = self.physics;
            physics.set_script_gravity(gravity);
            self.set_physics(physics);
        }
        let target_objects = self.scenario_section_object_spawns(&target)?;
        self.spawn_scenario_section_objects(target_objects)?;
        if !target.no_initialize && landscape_loaded && keep_map_creator {
            self.run_post_init_map_callbacks(&post_init_map_callbacks)?;
        }
        self.current_scenario_section = target.name;
        self.last_scenario_section_flags = Some(flags);
        Ok(true)
    }

    /// C4Object::IsUserPlayerObject: section object saves exclude user crew
    /// and user flags, while script-player objects remain ordinary section
    /// state. Membership is read from the player's exact crew list rather
    /// than the definition's CrewMember capability bit.
    pub(crate) fn is_user_player_object_snapshot(&self, object: &ObjectSnapshot) -> bool {
        self.players.get(&object.owner).is_some_and(|player| {
            !player.is_script_player()
                && (object.definition_id.as_str() == "FLAG" || player.crew().contains(&object.id))
        })
    }

    fn is_script_player_object_snapshot(&self, object: &ObjectSnapshot) -> bool {
        self.players.get(&object.owner).is_some_and(|player| {
            player.is_script_player()
                && (object.definition_id.as_str() == "FLAG" || player.crew().contains(&object.id))
        })
    }

    fn apply_script_player_team(
        &mut self,
        player_id: i32,
        team: Option<i32>,
        generated_team: Option<TeamInfo>,
        color: Option<u32>,
        home_base_material_entries: Option<Vec<(DefinitionId, i32)>>,
        synchronize_hostility: bool,
    ) -> Result<(), EngineError> {
        if let Some(generated_team) = generated_team {
            self.team_state.team_last_team_id =
                self.team_state.team_last_team_id.max(generated_team.id);
            if !self
                .team_state
                .teams
                .iter()
                .any(|existing| existing.id == generated_team.id)
            {
                Rc::make_mut(&mut self.team_state.teams).push(generated_team);
            }
        }
        if !self.players.contains_key(&player_id) {
            return Ok(());
        }
        self.players
            .get_mut(&player_id)
            .expect("player presence checked above")
            .set_team(team);
        if let Some(color) = color {
            self.set_player_color(player_id, color)?;
        }
        self.recheck_runtime_team_memberships();
        if let Some(material) = home_base_material_entries {
            self.players
                .get_mut(&player_id)
                .expect("player remains present")
                .set_home_base_material_entries(material);
        }
        if synchronize_hostility {
            self.set_player_team_hostility(player_id);
        }
        Ok(())
    }

    /// `C4Player::SetTeamHostility`: a nonzero team makes the switching
    /// player mutually hostile to every player in another team and mutually
    /// allied with every teammate. The writes are silent; SetPlayerTeam's
    /// own callbacks bracket this operation in the synchronous host preview.
    pub(crate) fn set_player_team_hostility(&mut self, player_id: i32) {
        let Some(team) = self.players.get(&player_id).and_then(Player::team) else {
            return;
        };
        let mut relations = self
            .players
            .iter()
            .filter_map(|(&other_id, player)| {
                (other_id != player_id).then_some((other_id, player.team() != Some(team)))
            })
            .collect::<Vec<_>>();
        relations.sort_unstable_by_key(|(other_id, _)| *other_id);
        for (other_id, hostile) in relations {
            if let Some(player) = self.players.get_mut(&player_id) {
                player.set_hostile_towards(other_id, hostile);
            }
            if let Some(other) = self.players.get_mut(&other_id) {
                other.set_hostile_towards(player_id, hostile);
            }
        }
    }

    fn adjust_object_info_experience(
        &mut self,
        object_id: ObjectId,
        link: Option<CrewInfoLink>,
        change: i32,
    ) -> Option<CrewObjectInfo> {
        // The C4ObjectInfo belongs to the player's persistent CrewInfoList
        // independently of the live object pointer. Apply there first so a
        // later Retire/Grab in the same ordered stream retains the change.
        let roster_values = link.and_then(|link| {
            self.crew_rosters
                .get_mut(&link.player_id)
                .and_then(|roster| roster.get_mut(link.roster_index))
                .map(|entry| {
                    let mut info = CrewObjectInfo {
                        definition_id: DefinitionId::from(entry.id.as_str()),
                        name: entry.name.clone(),
                        death_message: entry.death_message.clone(),
                        core: entry.core.clone(),
                        rank: entry.rank,
                        rank_name: entry.rank_name.clone(),
                        experience: entry.experience,
                        participation: entry.participation,
                        rounds: entry.rounds,
                        death_count: entry.death_count,
                        total_playing_time: entry.total_playing_time,
                        birthday: entry.birthday,
                        age: entry.age,
                        in_action_time: entry.in_action_time,
                        extra_data: entry.extra_data.clone(),
                        portraits: entry.portraits.clone(),
                    };
                    let promoted = adjust_crew_experience(&mut info, change);
                    entry.rank = info.rank;
                    entry.experience = info.experience;
                    (info, promoted)
                })
        });

        let live_values = {
            let infos = Rc::make_mut(&mut self.crew_object_infos);
            infos.get_mut(&object_id).map(|info| {
                let promoted = if let Some((roster_info, promoted)) = roster_values.as_ref() {
                    info.rank = roster_info.rank;
                    info.rank_name = roster_info.rank_name.clone();
                    info.experience = roster_info.experience;
                    *promoted
                } else {
                    adjust_crew_experience(info, change)
                };
                (info.clone(), promoted)
            })
        };
        let final_values = live_values.as_ref().or(roster_values.as_ref());
        if let Some((info, _)) = final_values {
            Rc::make_mut(&mut self.crew_ranks).insert(object_id.as_u64(), info.rank);
        }
        final_values
            .filter(|(_, promoted)| *promoted)
            .map(|(info, _)| info.clone())
    }

    fn promotion_rank_name(&self, info: &CrewObjectInfo) -> Option<String> {
        match self
            .definitions
            .get(&info.definition_id)
            .and_then(Definition::rank_names)
        {
            Some(names) => usize::try_from(info.rank)
                .ok()
                .and_then(|rank| names.get(rank))
                .map(|name| name.into_owned()),
            None => {
                compat::default_rank_name(&self.default_rank_names, info.rank).map(str::to_owned)
            }
        }
    }

    fn set_object_info_rank_name(
        &mut self,
        object_id: ObjectId,
        link: Option<CrewInfoLink>,
        rank_name: String,
    ) {
        if let Some(link) = link {
            if let Some(entry) = self
                .crew_rosters
                .get_mut(&link.player_id)
                .and_then(|roster| roster.get_mut(link.roster_index))
            {
                entry.rank_name = rank_name.clone();
            }
        }
        if let Some(info) = Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id) {
            info.rank_name = rank_name;
        }
    }

    /// Native `C4Object::DoExperience`: mutate the persistent info first,
    /// then run the promotion-only physical and presentation half exactly
    /// once. Script `DoCrewExp` previews those effects in its host scope and
    /// deliberately replays only [`Self::adjust_object_info_experience`].
    pub(crate) fn do_object_experience(&mut self, object_id: ObjectId, change: i32) {
        let link = self.crew_info_links.get(&object_id).copied();
        let Some(mut info) = self.adjust_object_info_experience(object_id, link, change) else {
            return;
        };
        let rank_name = self.promotion_rank_name(&info);
        if let Some(rank_name) = rank_name.as_ref() {
            info.rank_name = rank_name.clone();
            self.set_object_info_rank_name(object_id, link, rank_name.clone());
        }

        let definition_physical = self
            .definitions
            .get(&info.definition_id)
            .map(|definition| *definition.physical())
            .unwrap_or_default();
        if let Some(index) = self.find_object_index(object_id) {
            let physical = self.objects[index]
                .state
                .info_physical
                .unwrap_or(definition_physical);
            self.objects[index].state.info_physical =
                Some(promotion_updated_physical(physical, info.rank, None));
        }

        // An exhausted custom rank table promotes silently and retains the
        // preceding stored rank name; only definitions without a custom table
        // fall back to the game-global rank names.
        let Some(rank_name) = rank_name else {
            return;
        };
        let object_name = self
            .find_object_index(object_id)
            .and_then(|index| self.objects[index].state.custom_name.clone())
            .unwrap_or(info.name);
        self.messages.add_message(MessageSpec {
            kind: message::MessageKind::Target,
            text: format!("{object_name} is promoted|to {rank_name}!"),
            target: Some(object_id),
            player: None,
            offset: Vector2::ZERO,
            color: 0xffff_ffff,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        });
        self.pending_audio.push(AudioCommand::PlaySound {
            name: "Trumpet".to_string(),
            target: Some(object_id),
            volume: 100,
            looped: false,
            multiple: false,
            custom_falloff: None,
        });
    }

    pub(crate) fn set_linked_crew_info_physical(
        &mut self,
        link: CrewInfoLink,
        physical: PhysicalInfo,
    ) {
        if let Some(entry) = self
            .crew_rosters
            .get_mut(&link.player_id)
            .and_then(|roster| roster.get_mut(link.roster_index))
        {
            entry.physical = physical;
        }
    }

    #[doc(hidden)]
    pub fn apply_player_commands(
        &mut self,
        commands: Vec<PlayerCommand>,
    ) -> Result<(), EngineError> {
        for command in commands {
            match command {
                PlayerCommand::SetDefinitionName {
                    definition_id,
                    name,
                } => {
                    if let Some(definition) = self.definitions.get_mut(&definition_id) {
                        definition.set_name(name);
                        self.definition_metadata_cache.borrow_mut().take();
                    }
                }
                PlayerCommand::SetCrewInfoName {
                    object_id,
                    link,
                    name,
                } => {
                    if let Some(link) = link {
                        if let Some(entry) = self
                            .crew_rosters
                            .get_mut(&link.player_id)
                            .and_then(|roster| roster.get_mut(link.roster_index))
                        {
                            entry.name = name.clone();
                        }
                    }
                    if let Some(info) =
                        Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id)
                    {
                        info.name = name;
                    }
                }
                PlayerCommand::SetCrewInfoPortrait {
                    object_id,
                    link,
                    portraits,
                } => {
                    if let Some(link) = link {
                        if let Some(entry) = self
                            .crew_rosters
                            .get_mut(&link.player_id)
                            .and_then(|roster| roster.get_mut(link.roster_index))
                        {
                            entry.portraits = portraits.clone();
                        }
                    }
                    if let Some(info) =
                        Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id)
                    {
                        info.portraits = portraits;
                    }
                }
                PlayerCommand::SetCrewExtraData {
                    object_id,
                    link,
                    name,
                    value,
                } => {
                    let write_slot = |slots: &mut Vec<(String, Value)>| match slots
                        .iter_mut()
                        .find(|(slot, _)| *slot == name)
                    {
                        Some((_, stored)) => *stored = value.clone(),
                        None => slots.push((name.clone(), value.clone())),
                    };
                    if let Some(link) = link {
                        if let Some(entry) = self
                            .crew_rosters
                            .get_mut(&link.player_id)
                            .and_then(|roster| roster.get_mut(link.roster_index))
                        {
                            write_slot(&mut entry.extra_data);
                        }
                    }
                    if let Some(info) =
                        Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id)
                    {
                        write_slot(&mut info.extra_data);
                    }
                }
                PlayerCommand::SetCrewInfoPhysical { link, physical } => {
                    self.set_linked_crew_info_physical(link, physical);
                }
                PlayerCommand::LoadScenarioSection {
                    name,
                    flags,
                    preserve_ids,
                } => {
                    let _ = self.load_scenario_section(&name, flags, preserve_ids)?;
                }
                PlayerCommand::AddEvaluationData {
                    player_info_id,
                    text,
                } => {
                    self.round_results
                        .add_custom_evaluation_string(&text, player_info_id);
                }
                PlayerCommand::HideSettlementScore { hide } => {
                    self.round_results.hide_settlement_score = hide;
                }
                PlayerCommand::SetLeaguePerformance {
                    score,
                    player_info_id,
                } => {
                    self.round_results
                        .set_league_performance(score, player_info_id);
                }
                PlayerCommand::SetLeagueProgressData {
                    player_info_id,
                    data,
                } => {
                    let data = data.map(legacy_c_string_bytes);
                    Rc::make_mut(&mut self.player_info_league_progress_data)
                        .insert(player_info_id, data.clone());
                    self.host_requests
                        .player_info_league_progress_updates
                        .push((player_info_id, data));
                }
                PlayerCommand::SetRestoreInfos { what } => {
                    self.restart_restore_info_mask = what;
                }
                PlayerCommand::SetMaxPlayer { max_players } => {
                    self.max_players = Some(max_players);
                }
                PlayerCommand::AddMessageBoardCommand { command } => {
                    let _ = self.add_message_board_command(command);
                }
                PlayerCommand::CallMessageBoard { player_id, query } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.call_message_board(query);
                    }
                }
                PlayerCommand::AbortMessageBoard { player_id, target } => {
                    if self
                        .active_message_board_input
                        .as_ref()
                        .is_some_and(|input| input.player == player_id && input.target == target)
                    {
                        self.active_message_board_input = None;
                    }
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.remove_message_board_query(target);
                    }
                }
                PlayerCommand::RemoveMessageBoardQuery { player_id, target } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.remove_message_board_query(target);
                    }
                }
                PlayerCommand::ActivateGameGoalMenu {
                    player_id,
                    open_menu,
                } => {
                    let (goals, fulfilled_goals) = self.evaluate_goals_for_player(player_id)?;
                    self.host_requests
                        .pending_game_goal_menu_requests
                        .push(GameGoalMenuRequest {
                            player: player_id,
                            goals,
                            fulfilled_goals,
                            open_menu: open_menu && !self.replay_control,
                        });
                }
                PlayerCommand::SetPlayerTeam {
                    player_id,
                    team,
                    generated_team,
                    color,
                    home_base_material_entries,
                    synchronize_hostility,
                } => {
                    self.apply_script_player_team(
                        player_id,
                        team,
                        generated_team,
                        color,
                        home_base_material_entries,
                        synchronize_hostility,
                    )?;
                }
                PlayerCommand::InitScenarioPlayer { player_id, team } => {
                    let _ = self.initialize_scenario_player(player_id, team)?;
                }
                PlayerCommand::SetCrewRosters { rosters } => {
                    for (player_id, crew) in rosters {
                        if let Some(player) = self.players.get_mut(&player_id) {
                            player.set_crew(crew);
                        }
                    }
                }
                PlayerCommand::RetireCrewInfo { object_id, link } => {
                    if let Some(entry) = self
                        .crew_rosters
                        .get_mut(&link.player_id)
                        .and_then(|roster| roster.get_mut(link.roster_index))
                    {
                        // C4ObjectInfo::Retire is idempotent and accrues only
                        // the current active stint.
                        if entry.in_action {
                            entry.total_playing_time = entry
                                .total_playing_time
                                .wrapping_add(self.game_time.wrapping_sub(entry.in_action_time));
                            entry.in_action = false;
                        }
                    }
                    if self.crew_info_links.get(&object_id) == Some(&link) {
                        Rc::make_mut(&mut self.crew_info_links).remove(&object_id);
                        Rc::make_mut(&mut self.crew_object_infos).remove(&object_id);
                        Rc::make_mut(&mut self.crew_ranks).remove(&object_id.as_u64());
                    }
                }
                PlayerCommand::AssignDeathCrewInfo { object_id, link } => {
                    let mut info_update = None;
                    if let Some(entry) = self
                        .crew_rosters
                        .get_mut(&link.player_id)
                        .and_then(|roster| roster.get_mut(link.roster_index))
                    {
                        entry.has_died = true;
                        entry.death_count = entry.death_count.wrapping_add(1);
                        if entry.in_action {
                            entry.total_playing_time = entry
                                .total_playing_time
                                .wrapping_add(self.game_time.wrapping_sub(entry.in_action_time));
                            entry.in_action = false;
                        }
                        info_update = Some((
                            entry.death_count,
                            entry.total_playing_time,
                            entry.in_action_time,
                            entry.age,
                        ));
                    }
                    if self.crew_info_links.get(&object_id) == Some(&link) {
                        if let (
                            Some(info),
                            Some((death_count, total_playing_time, in_action_time, age)),
                        ) = (
                            Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id),
                            info_update,
                        ) {
                            info.death_count = death_count;
                            info.total_playing_time = total_playing_time;
                            info.in_action_time = in_action_time;
                            info.age = age;
                        }
                    }
                }
                PlayerCommand::LinkCrewInfo {
                    object_id,
                    link,
                    mut info,
                    created_entry,
                    recruit,
                    has_died,
                } => {
                    let recruited_rank_name = recruit.then(|| {
                        self.recruit_rank_name(&info.definition_id, info.rank, &info.rank_name)
                    });
                    if let Some(link) = link {
                        let roster = self.crew_rosters.entry(link.player_id).or_default();
                        let mut created = false;
                        if let Some(entry) = created_entry {
                            if link.roster_index == roster.len() {
                                roster.push(entry);
                                created = true;
                            } else if link.roster_index < roster.len() {
                                roster[link.roster_index] = entry;
                            }
                        }
                        if created {
                            let order =
                                self.crew_info_order
                                    .entry(link.player_id)
                                    .or_insert_with(|| {
                                        (0..roster.len())
                                            .filter(|index| *index != link.roster_index)
                                            .collect()
                                    });
                            order.retain(|index| *index != link.roster_index);
                            order.insert(0, link.roster_index);
                            if let Some(player) = self.players.get_mut(&link.player_id) {
                                player.increment_crew_created();
                            }
                        }
                        if let Some(entry) = roster.get_mut(link.roster_index) {
                            entry.has_died = has_died;
                            if recruit && !entry.in_action {
                                entry.in_action = true;
                                entry.was_in_action = true;
                                entry.in_action_time = self.game_time;
                                if let Some(rank_name) = recruited_rank_name.as_ref() {
                                    entry.rank_name = rank_name.clone();
                                }
                            }
                            info.definition_id = DefinitionId::from(entry.id.as_str());
                            info.name = entry.name.clone();
                            info.death_message = entry.death_message.clone();
                            info.core = entry.core.clone();
                            info.rank = entry.rank;
                            info.rank_name = entry.rank_name.clone();
                            info.experience = entry.experience;
                            info.participation = entry.participation;
                            info.rounds = entry.rounds;
                            info.death_count = entry.death_count;
                            info.total_playing_time = entry.total_playing_time;
                            info.birthday = entry.birthday;
                            info.age = entry.age;
                            info.in_action_time = entry.in_action_time;
                            info.extra_data = entry.extra_data.clone();
                            info.portraits = entry.portraits.clone();
                        }
                        Rc::make_mut(&mut self.crew_info_links).insert(object_id, link);
                    } else {
                        Rc::make_mut(&mut self.crew_info_links).remove(&object_id);
                    }
                    Rc::make_mut(&mut self.crew_object_infos).insert(object_id, info.clone());
                    Rc::make_mut(&mut self.crew_ranks).insert(object_id.as_u64(), info.rank);
                }
                PlayerCommand::AdjustCrewExperience {
                    object_id,
                    link,
                    change,
                } => {
                    if let Some(info) = self.adjust_object_info_experience(object_id, link, change)
                    {
                        if let Some(rank_name) = self.promotion_rank_name(&info) {
                            self.set_object_info_rank_name(object_id, link, rank_name);
                        }
                    }
                }
                PlayerCommand::AdjustCrewControlCount { link, gain } => {
                    self.adjust_crew_info_control_count(link, gain);
                }
                PlayerCommand::AdjustHomeBaseMaterial {
                    player_id,
                    definition_id,
                    delta,
                } => {
                    self.adjust_player_home_base_material(player_id, definition_id, delta)?;
                }
                PlayerCommand::SyncHomeBaseMaterialToTeam { player_id } => {
                    self.sync_team_home_base_from_player(player_id);
                }
                PlayerCommand::AdjustHomeBaseProduction {
                    player_id,
                    definition_id,
                    delta,
                } => {
                    self.adjust_player_home_base_production(player_id, definition_id, delta)?;
                }
                PlayerCommand::GrantKnowledge {
                    player_id,
                    definition_id,
                } => {
                    self.grant_player_knowledge(player_id, definition_id)?;
                }
                PlayerCommand::RevokeKnowledge {
                    player_id,
                    definition_id,
                } => {
                    self.revoke_player_knowledge(player_id, &definition_id)?;
                }
                PlayerCommand::GrantMagic {
                    player_id,
                    definition_id,
                } => {
                    self.grant_player_magic(player_id, definition_id)?;
                }
                PlayerCommand::RevokeMagic {
                    player_id,
                    definition_id,
                } => {
                    self.revoke_player_magic(player_id, &definition_id)?;
                }
                PlayerCommand::SetCursor {
                    player_id,
                    object,
                    control,
                } => {
                    // FnSetCursor's callbacks and SelectCrew branch already
                    // ran synchronously inside the host context. This fold
                    // persists cursor/control metadata only; cursor-only
                    // calls must never imply C4Object::Select. Do not repeat
                    // FnSetCursor's object validation here: CreateObject is
                    // live synchronously in C++, while Rust materializes its
                    // already-validated SpawnConfig later in this outcome.
                    // Dragon Rock's Redefine3 sets the cursor to exactly such
                    // a fresh replacement before the spawn queue is folded.
                    let selection = self.crew_selection.entry(player_id).or_default();
                    selection.cursor = object;
                    if selection.is_empty() {
                        self.crew_selection.remove(&player_id);
                    }
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.set_cursor(object);
                        player.control = control;
                    }
                }
                PlayerCommand::SetViewCursor { player_id, object } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.set_view_cursor(object);
                    }
                }
                PlayerCommand::SetPlrView { player_id, object } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.set_view_target(object);
                    }
                }
                PlayerCommand::ClearObjectPointers { object } => {
                    if self
                        .active_message_board_input
                        .as_ref()
                        .is_some_and(|input| input.target == Some(object))
                    {
                        self.active_message_board_input = None;
                    }
                    let owners = self.player_ids_in_order();
                    for owner in owners {
                        let removed_cursor = self.crew_cursor(owner) == Some(object);
                        if removed_cursor {
                            if let Some(selection) = self.crew_selection.get_mut(&owner) {
                                selection.set_cursor(None);
                            }
                            if self
                                .crew_selection
                                .get(&owner)
                                .is_some_and(CrewSelection::is_empty)
                            {
                                self.crew_selection.remove(&owner);
                            }
                        }
                        if let Some(player) = self.players.get_mut(&owner) {
                            player.clear_object_pointers(object);
                        }
                        self.remove_from_roles(owner, object);
                        if removed_cursor {
                            self.player_adjust_cursor_command(owner)?;
                        }
                    }
                }
                PlayerCommand::ClearPlayerObjectPointersBeforeAdjust { player_id, object } => {
                    if self.crew_cursor(player_id) == Some(object) {
                        if let Some(selection) = self.crew_selection.get_mut(&player_id) {
                            selection.set_cursor(None);
                        }
                        if self
                            .crew_selection
                            .get(&player_id)
                            .is_some_and(CrewSelection::is_empty)
                        {
                            self.crew_selection.remove(&player_id);
                        }
                    }
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.clear_object_pointers_before_cursor_adjust(object);
                    }
                    self.remove_from_roles(player_id, object);
                }
                PlayerCommand::ClearPlayerObjectPointersAfterAdjust { player_id, object } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.clear_object_pointers_after_cursor_adjust(object);
                    }
                }
                PlayerCommand::ResetCursorView { player_id } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.reset_cursor_view();
                    }
                }
                PlayerCommand::UpdatePlayerView {
                    player_id,
                    position,
                } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.update_view(position);
                    }
                }
                PlayerCommand::ClearLastPlrCom { player_id } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.control.last_com = 0;
                        player.control.last_com_down_double = 0;
                    }
                }
                PlayerCommand::SetWealth {
                    player_id,
                    value,
                    show_change,
                } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.set_wealth(value);
                        if show_change {
                            player.arm_view_wealth();
                        }
                    }
                }
                PlayerCommand::AdjustPoints { player_id, delta } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.adjust_points(delta);
                    }
                }
                PlayerCommand::Eliminate { player_id } => {
                    if self
                        .players
                        .get_mut(&player_id)
                        .is_some_and(|player| player.eliminate())
                    {
                        self.eliminated_crew_owners.insert(player_id);
                    }
                }
                PlayerCommand::Surrender { player_id } => {
                    if self.players.contains_key(&player_id) {
                        self.set_player_surrendered(player_id, true)?;
                    }
                }
                PlayerCommand::Remove { player_id } => {
                    self.host_requests.pending_remove_player_controls.push(
                        RemovePlayerControlData {
                            player: player_id,
                            disconnected: false,
                            by_client: 0,
                        },
                    );
                }
                PlayerCommand::SetFogOfWar { player_id, enabled } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.set_fog_of_war(enabled);
                    }
                }
                PlayerCommand::SetShowControlPosition {
                    player_id,
                    position,
                } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.show_control_position = position;
                    }
                }
                PlayerCommand::SetShowControl { player_id, mask } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.show_control = mask;
                    }
                }
                PlayerCommand::SetShowCommand { player_id, command } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.set_flash_command(command);
                        self.show_commands_requests.request_enable();
                    }
                }
                PlayerCommand::SetHostility {
                    player_id,
                    opponent,
                    hostile,
                } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.set_hostile_towards(opponent, hostile);
                    }
                }
                // FnSetPlrExtraData (C4Script.cpp:4712-4730): update in
                // place, or append preserving the names-list order.
                PlayerCommand::SetExtraData {
                    player_id,
                    name,
                    value,
                } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        match player.extra_data.iter_mut().find(|(slot, _)| *slot == name) {
                            Some((_, stored)) => *stored = value,
                            None => player.extra_data.push((name, value)),
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn sync_team_home_base_for(&mut self, id: i32) {
        if !self.team_home_base_rule {
            return;
        }
        let (team, player_info_id) = match self.players.get(&id) {
            Some(player) => match player.team() {
                Some(team) => (team, player.player_info_id()),
                None => return,
            },
            None => return,
        };
        let captain_id = self
            .team_state
            .teams
            .iter()
            .find(|candidate| candidate.id == team)
            .and_then(|team| team.player_ids.first().copied())
            .and_then(|captain_info_id| {
                if captain_info_id == player_info_id {
                    return None;
                }
                self.players
                    .values()
                    .find(|player| player.player_info_id() == captain_info_id)
                    .map(Player::id)
            })
            .or_else(|| {
                let has_player_info_order = self
                    .team_state
                    .teams
                    .iter()
                    .find(|candidate| candidate.id == team)
                    .is_some_and(|team| !team.player_ids.is_empty());
                if has_player_info_order {
                    None
                } else {
                    self.runtime_team_members_in_order(team)
                        .into_iter()
                        .next()
                        .filter(|captain| *captain != id)
                }
            });
        let Some(material) = captain_id.and_then(|captain| {
            self.players
                .get(&captain)
                .map(|player| player.home_base_material_entries().to_vec())
        }) else {
            return;
        };
        if let Some(player) = self.players.get_mut(&id) {
            player.set_home_base_material_entries(material);
        }
    }

    /// `C4Player::SyncHomebaseMaterialToTeam`: a runtime mutation propagates
    /// the mutating player's complete ordered list, even when that player is
    /// not the team's first member (C4Player.cpp:2335-2349).
    pub(crate) fn sync_team_home_base_from_player(&mut self, id: i32) {
        if !self.team_home_base_rule {
            return;
        }
        let Some((team, material)) = self.players.get(&id).and_then(|player| {
            player
                .team()
                .map(|team| (team, player.home_base_material_entries().to_vec()))
        }) else {
            return;
        };
        for (&member_id, member) in &mut self.players {
            if member_id != id && member.team() == Some(team) {
                member.set_home_base_material_entries(material.clone());
            }
        }
    }

    fn runtime_team_order_id(player: &Player) -> i32 {
        let info_id = player.player_info_id();
        if info_id > 0 {
            info_id
        } else {
            player.id()
        }
    }

    /// Mirrors `C4Team::RecheckPlayers`: retain configured/persisted order,
    /// remove a live player-info ID that changed teams, then append newly
    /// observed IDs in PlayerInfo order. IDs without a current runtime
    /// player remain because `GetFirstActivePlayerID` skips them at lookup.
    pub(crate) fn recheck_runtime_team_memberships(&mut self) {
        let mut live = self
            .players
            .values()
            .map(|player| (Self::runtime_team_order_id(player), player.team()))
            .collect::<Vec<_>>();
        live.sort_unstable_by_key(|(info_id, _)| *info_id);
        let live_teams = live.iter().copied().collect::<HashMap<_, _>>();
        for team in Rc::make_mut(&mut self.team_state.teams) {
            let team_id = team.id;
            team.player_ids.retain(|info_id| {
                live_teams
                    .get(info_id)
                    .is_none_or(|runtime_team| *runtime_team == Some(team_id))
            });
            for (info_id, _) in live
                .iter()
                .copied()
                .filter(|(_, runtime_team)| *runtime_team == Some(team_id))
            {
                if !team.player_ids.contains(&info_id) {
                    team.player_ids.push(info_id);
                }
            }
        }
    }

    /// Resolves the ordered player-info membership to current runtime player
    /// numbers. These number spaces diverge after savegame recreation.
    pub(crate) fn runtime_team_members_in_order(&self, team_id: i32) -> Vec<i32> {
        let mut live = self
            .players
            .values()
            .filter(|player| player.team() == Some(team_id))
            .map(|player| (Self::runtime_team_order_id(player), player.id()))
            .collect::<Vec<_>>();
        live.sort_unstable();

        let mut ordered = Vec::with_capacity(live.len());
        if let Some(team) = self.team_state.teams.iter().find(|team| team.id == team_id) {
            for info_id in &team.player_ids {
                if let Some(index) = live.iter().position(|(id, _)| id == info_id) {
                    ordered.push(live.remove(index).1);
                }
            }
        }
        ordered.extend(live.into_iter().map(|(_, number)| number));
        ordered
    }

    pub(crate) fn sync_team_home_base_group(&mut self, team: i32) {
        if !self.team_home_base_rule {
            return;
        }
        let members = self.runtime_team_members_in_order(team);
        if members.len() <= 1 {
            return;
        }
        let leader_id = members[0];
        let material = match self.players.get(&leader_id) {
            Some(leader) => leader.home_base_material_entries().to_vec(),
            None => return,
        };
        for member_id in members.into_iter().skip(1) {
            if let Some(member) = self.players.get_mut(&member_id) {
                member.set_home_base_material_entries(material.clone());
            }
        }
    }

    pub(crate) fn prune_selection(&mut self) {
        self.prune_roles();
        let listed: HashSet<ObjectId> = self
            .players
            .values()
            .flat_map(|player| player.crew().iter().copied())
            .collect();
        for object in &mut self.objects {
            let member = if self.players.is_empty() {
                object.state.crew_member
            } else {
                listed.contains(&object.id)
                    || (!self.players.contains_key(&object.state.owner) && object.state.crew_member)
            };
            if !member || object.destroyed || object.state.status == ObjectStatus::Deleted {
                object.state.selected = false;
            }
        }

        // C4Player::Cursor is an enumerated object pointer, not restricted
        // to the Crew list: SetCursor deliberately installs helper objects
        // such as AIMR/SELR/CBMU while their owning clonk is deselected
        // (C4Script.cpp:2943-2963; C4Player.cpp:1745,1784-1792). Object
        // selection flags remain crew-only above, but prune Cursor only when
        // its object pointer is gone. Inactive objects remain valid pointers.
        let active: HashSet<ObjectId> = self
            .objects
            .iter()
            .filter(|object| !object.destroyed && object.state.status != ObjectStatus::Deleted)
            .map(|object| object.id)
            .collect();
        self.crew_selection.retain(|_, selection| {
            selection.prune(&active);
            !selection.is_empty()
        });
        self.sync_all_player_cursors();
    }

    pub(crate) fn prune_roles(&mut self) {
        if self.crew_roles.is_empty() {
            return;
        }

        let existing: HashSet<ObjectId> = self
            .objects
            .iter()
            .filter(|object| !object.destroyed && object.state.status != ObjectStatus::Deleted)
            .map(|object| object.id)
            .collect();
        let mut valid: HashMap<i32, HashSet<ObjectId>> = self
            .players
            .iter()
            .map(|(&player_id, player)| {
                (
                    player_id,
                    player
                        .crew()
                        .iter()
                        .copied()
                        .filter(|object| existing.contains(object))
                        .collect(),
                )
            })
            .collect();
        for object in &self.objects {
            if !self.players.contains_key(&object.state.owner)
                && object.state.crew_member
                && object.state.status.is_active()
                && !object.destroyed
            {
                valid
                    .entry(object.state.owner)
                    .or_default()
                    .insert(object.id);
            }
        }

        self.crew_roles.retain(|owner, assignments| {
            let roster = valid.get(owner);
            assignments
                .retain(|object_id, _| roster.is_some_and(|roster| roster.contains(object_id)));
            !assignments.is_empty()
        });
    }

    pub(crate) fn resolve_command_targets(
        &self,
        owner: i32,
        target: &CrewCommandTarget,
    ) -> Vec<ObjectId> {
        match target {
            CrewCommandTarget::Cursor => self.crew_cursor(owner).into_iter().collect(),
            CrewCommandTarget::Selection => self.selected_crew(owner),
            CrewCommandTarget::Role(role) => self
                .crew_roles
                .get(&owner)
                .map(|assignments| {
                    assignments
                        .iter()
                        .filter_map(|(&object_id, assigned)| {
                            if assigned == role {
                                Some(object_id)
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// One-time compatibility import for fixtures/old saves that carry only
    /// the Rust union membership bit. Once a Player exists its ordered Crew
    /// list is authoritative, so steady-state refresh never repeats this.
    pub(crate) fn bootstrap_player_crew_from_union(&mut self, player_id: i32) {
        let already_has_roster = self
            .players
            .get(&player_id)
            .is_some_and(|player| !player.crew().is_empty());
        if already_has_roster {
            return;
        }
        let crew = self
            .exec_list
            .iter()
            .rev()
            .copied()
            .filter(|object_id| {
                self.find_object_index(*object_id).is_some_and(|index| {
                    let object = &self.objects[index];
                    object.state.owner == player_id
                        && object.state.crew_member
                        && !object.destroyed
                        && object.state.status != ObjectStatus::Deleted
                })
            })
            .collect();
        if let Some(player) = self.players.get_mut(&player_id) {
            player.set_crew(crew);
        }
    }

    pub(crate) fn crew_insert_position(&self, roster: &[ObjectId], target: ObjectId) -> usize {
        let Some(target_index) = self.find_object_index(target) else {
            return roster.len();
        };
        let object = &self.objects[target_index];
        if self
            .definitions
            .get(&object.definition_id)
            .is_some_and(|definition| definition.line() != 0)
        {
            return roster.len();
        }
        let category = object.state.category;
        let sort_category = category & CATEGORY_SORT_LIMIT;
        if category & CATEGORY_STATIC_BACK == 0 {
            if let Some(position) = roster.iter().position(|other| {
                self.find_object_index(*other).is_some_and(|other_index| {
                    let other = &self.objects[other_index];
                    other.state.category & CATEGORY_SORT_LIMIT == sort_category
                        && other.definition_id == object.definition_id
                })
            }) {
                return position;
            }
        }
        roster
            .iter()
            .position(|other| {
                self.find_object_index(*other).is_some_and(|other_index| {
                    self.objects[other_index].state.category & CATEGORY_SORT_LIMIT <= sort_category
                })
            })
            .unwrap_or(roster.len())
    }

    /// Preserve the ordered C4Player::Crew lists as authoritative pointers.
    /// Inactive and dead objects remain linked exactly like C++; only a gone
    /// object pointer is pruned. The return value is raw CrewCnt>0 by player.
    pub(crate) fn refresh_elimination_state(&mut self) -> HashSet<i32> {
        let mut nonempty = HashSet::new();
        if self.objects.is_empty() && self.known_crew_owners.is_empty() && self.players.is_empty() {
            return nonempty;
        }

        if self.players.is_empty() {
            for object in &self.objects {
                if object.state.crew_member
                    && object.state.owner != OWNER_NONE
                    && !object.destroyed
                    && object.state.status != ObjectStatus::Deleted
                {
                    self.known_crew_owners.insert(object.state.owner);
                    if object.state.status.is_active() && object.state.alive {
                        nonempty.insert(object.state.owner);
                    }
                }
            }
            return nonempty;
        }

        let player_ids: Vec<i32> = self.players.keys().copied().collect();
        let mut listed = HashSet::new();
        for player_id in &player_ids {
            let existing = self
                .players
                .get(player_id)
                .map(|player| player.crew().to_vec())
                .unwrap_or_default();
            let mut seen = HashSet::new();
            let retained = existing
                .into_iter()
                .filter(|id| {
                    seen.insert(*id)
                        && self.find_object_index(*id).is_some_and(|index| {
                            let object = &self.objects[index];
                            !object.destroyed && object.state.status != ObjectStatus::Deleted
                        })
                })
                .collect::<Vec<_>>();
            listed.extend(retained.iter().copied());
            if !retained.is_empty() {
                self.known_crew_owners.insert(*player_id);
                nonempty.insert(*player_id);
            }
            if let Some(player) = self.players.get_mut(player_id) {
                player.set_crew(retained);
            }
        }

        // Rust retains a compatibility union bit for older snapshots and
        // host fixtures. Keep it derived from the live per-player links;
        // never use it to reconstruct a removed link during steady state.
        for object in &mut self.objects {
            if object.state.status != ObjectStatus::Deleted {
                if listed.contains(&object.id) {
                    object.state.crew_member = true;
                } else if self.players.contains_key(&object.state.owner) {
                    object.state.crew_member = false;
                }
            }
        }
        nonempty
    }

    /// C4Object::Stabilize (C4Movement.cpp:488-516): a tilt within
    /// ±StableRange (±10, C4Physics.h:23, normalized to ±180) snaps
    /// upright when the rotation-0 shape stands contact-free at the
    /// current position; any contact keeps the tilt. NoStabilize defs opt
    /// out (:491). The upright probe is the ordinary ContactCheck, including
    /// Contact* callback dispatch for ContactCalls definitions (:503).
    #[doc(hidden)]
    pub fn stabilize_object(
        &mut self,
        idx: usize,
        _solid_mask_indices: &[usize],
    ) -> Result<(), EngineError> {
        let rotation = self.objects[idx].state.rotation;
        // C++ repeatedly folds arbitrary saved/scripted angles into
        // [-180, 180] (C4Movement.cpp:493-494). `% 360` plus one boundary
        // adjustment is equivalent without a potentially huge loop.
        let mut signed = rotation % 360;
        if signed < -180 {
            signed += 360;
        } else if signed > 180 {
            signed -= 360;
        }
        if signed == 0 || !(-math::STABLE_RANGE..=math::STABLE_RANGE).contains(&signed) {
            return Ok(());
        }
        let no_stabilize = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .is_some_and(Definition::no_stabilize);
        // C4Shape::ContactDensity is live object state. C4Object::Stabilize
        // probes Shape.CheckContact after SetContactDensity mutations
        // (C4Movement.cpp:488-516; C4Shape.cpp:495-510).
        let contact_density = self.objects[idx].state.contact_density;
        if no_stabilize {
            return Ok(());
        }
        let upright_vertices = self.objects[idx].unrotated_shape_vertices();
        let original_vertices = self.objects[idx].state.vertices.clone();
        let original_shape_vertices = self.objects[idx].state.shape_vertices.clone();
        let original_shape_rect = self.objects[idx].shape_rect;
        let original_fire_top = self.objects[idx].shape_fire_top;
        let original_shape_override = self.objects[idx].state.shape_override;
        let original_vertex_contacts = self.objects[idx].frame_vertex_contacts.clone();
        let original_shape_contact_cnat = self.objects[idx].frame_shape_contact_cnat;
        let original_shape_contact_count = self.objects[idx].frame_shape_contact_count;
        let original_shape_attach = self.objects[idx].state.shape_attach;
        let original_contact_density = self.objects[idx].state.contact_density;
        let object_id = self.objects[idx].id;
        let position = self.objects[idx].state.position;
        // C++ temporarily writes r=0 and UpdateShape() before ContactCheck,
        // so the callback observes the upright rotation and vertices. fix_r
        // is left untouched unless stabilization succeeds (:498-514).
        self.objects[idx].state.rotation = 0;
        self.objects[idx].refresh_shape_geometry();
        self.update_sector_for_index(idx);
        debug_assert_eq!(self.objects[idx].state.vertices, upright_vertices);
        let contact = self
            .landscape
            .as_ref()
            .map(|landscape| {
                let solid_masks = self.solid_masks_for_movement(&self.active_solid_mask_indices());
                shape_contact_check(
                    &self.objects[idx].state.vertices,
                    position,
                    landscape,
                    &self.materials,
                    &solid_masks,
                    None,
                    contact_density,
                )
            })
            .unwrap_or_default();
        // ContactCheck always overwrites t_contact, including with zero,
        // before it dispatches Contact* callbacks (C4Movement.cpp:166-182).
        self.objects[idx].latch_shape_contact(&contact);
        if contact.is_contact() {
            self.dispatch_contact_callbacks(idx, MovementContactDispatch::ShapeProbe)?;
        }
        if let Some(index) = self.find_object_index(object_id) {
            if self.objects[index].frame_shape_contact_count != 0 {
                // ContactCheck rejected the trial: restore exactly Shape and
                // integer r. Callback changes to other fields (including
                // fix_r) remain live, matching C++'s two assignments (:505-508).
                let owns_shape_vertices = self.objects[index].own_shape_vertices.is_some();
                self.objects[index].state.vertices = original_vertices;
                self.objects[index].state.shape_vertices = original_shape_vertices;
                self.objects[index].own_shape_vertices = owns_shape_vertices.then(|| {
                    self.objects[index]
                        .state
                        .shape_vertices
                        .own_original_vertices()
                });
                self.objects[index].shape_rect = original_shape_rect;
                self.objects[index].shape_fire_top = original_fire_top;
                self.objects[index].state.shape_override = original_shape_override;
                self.objects[index].frame_vertex_contacts = original_vertex_contacts;
                self.objects[index].frame_shape_contact_cnat = original_shape_contact_cnat;
                self.objects[index].frame_shape_contact_count = original_shape_contact_count;
                self.objects[index].state.shape_attach = original_shape_attach;
                self.objects[index].state.contact_density = original_contact_density;
                self.objects[index].state.rotation = rotation;
            } else {
                // ContactCheck callbacks may have rebuilt Shape and changed r.
                // Native Stabilize commits that live r into fix_r, then runs
                // UpdateFace(true) (C4Movement.cpp:524-535).
                let live_rotation = self.objects[index].state.rotation;
                self.objects[index].fixed_rotation = itofix(live_rotation);
                let shape_updated = self.objects[index].shape_template.line == 0;
                self.objects[index].refresh_shape_geometry();
                if shape_updated {
                    self.update_sector_for_index(index);
                }
                self.update_solid_mask(index);
            }
        }
        Ok(())
    }

    /// C4Object::CopyMotion (C4Movement.cpp:518-529), run for contained
    /// objects by ExecMovement (:556-561): copy the container's integer
    /// position (resorting sectors on change), snap fix_x/fix_y to
    /// itofix(x/y) and copy the container's dirs.
    pub(crate) fn copy_motion_from_container(&mut self, idx: usize) {
        let Some(container_idx) = self.objects[idx]
            .state
            .container
            .and_then(|container_id| self.find_object_index(container_id))
        else {
            return;
        };
        let (position, fixed_velocity) = {
            let container = &self.objects[container_idx];
            (container.state.position, container.fixed_velocity)
        };
        let moved = {
            let object = &mut self.objects[idx];
            let moved = object.state.position != position;
            object.state.position = position;
            object.fixed_position = FixedVec2::new(itofix(position.x), itofix(position.y));
            object.fixed_velocity = fixed_velocity;
            object.state.velocity = object.velocity_pixels();
            moved
        };
        if moved {
            self.update_sector_for_index(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_callbacks_only_need_an_owned_snapshot_for_an_unlinked_stop_victim() {
        // C4Effect construction, Check, and Execute call Start/Effect/Add/Timer
        // against the live target and linked list (C4Effect.cpp:96-152,
        // 271-317,339-360). Kill/ClearAll alone keep a removed victim linked
        // through its Stop callback (:365-424).
        let mut effect = EffectState::new("Pulse");
        effect.number = 7;

        let timer = EffectEvent::timer(effect.clone());
        assert!(!effect_callback_needs_owned_snapshot(
            &[effect.clone()],
            &timer
        ));

        let stopped = EffectEvent::stopped(effect.clone(), EffectStopReason::Removed);
        assert!(!effect_callback_needs_owned_snapshot(
            &[effect.clone()],
            &stopped
        ));
        assert!(effect_callback_needs_owned_snapshot(&[], &stopped));
    }
}
