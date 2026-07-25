//! `impl Engine` — the frame tick, object updates and callback outcomes.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub(crate) fn advance_tick(&mut self) -> Result<(), EngineError> {
        // The previous frame's C4Landscape::Draw ran DoRelights before its
        // blit. Start the new simulation frame after that presentation
        // boundary so fresh SetLandscapePixel writes may survive again.
        if let Some(landscape) = &mut self.landscape {
            landscape.finish_surface32_draw();
        }
        self.exec_cursor = None;
        self.frame += 1;
        // C4Game::Ticks only arms the external one-second timer; it does not
        // increment Game.Time itself (C4Game.cpp:1899-1913).
        self.time_go = true;
        self.objective_check_counter =
            (self.objective_check_counter + 1) % GAME_OVER_CHECK_INTERVAL;
        let frame = self.frame;
        // C4GameControl::Ticks runs with the frame advance (C4Game.cpp:801)
        self.control_ticks();
        if std::env::var("LC_RUST_RNG_TRACE").is_ok() {
            crate::rng::rng_trace_frame_marker(frame);
        }
        // The per-tick scenario Step (and its `random` argument DRAW) is a
        // JSON-fixture convention: C++ never calls Step on scenario
        // scripts, and the draw would shift the synced stream every frame.
        let fixture_scenario_step = self
            .scenario_script
            .as_ref()
            .map(|script| !script.c4_args)
            .unwrap_or(false);
        if fixture_scenario_step {
            let snapshot = self.snapshot();
            let world = self.host_world_context();
            let random = self.next_random_i32();
            let rng_state = self.rng.clone();
            let environment = self.environment;
            let global_effects = self.global_effects.clone();
            let particle_defs = self.particle_system.def_names();
            let definition_metadata_table = self.definition_metadata_table();
            let definition_order = Rc::clone(&self.runtime_definition_order);
            let network_game = self.network_game;
            let engine_next_object_id = self.next_object_id;
            let scenario_script_counter = self.scenario_script_counter;
            let scoreboard = Rc::clone(&self.scoreboard);
            let materials = self.materials_shared();
            let (batch, audio_state, new_rng) = {
                let definition_scripts = self.definition_script_table();
                let script = self
                    .scenario_script
                    .as_mut()
                    .expect("scenario script must be present");
                script.step(
                    &snapshot,
                    world,
                    scoreboard,
                    materials,
                    rng_state,
                    random,
                    frame,
                    &global_effects,
                    self.physics,
                    environment,
                    self.audio_registry.clone(),
                    particle_defs,
                    definition_scripts,
                    definition_metadata_table.clone(),
                    definition_order,
                    network_game,
                    engine_next_object_id,
                    scenario_script_counter,
                )?
            };
            self.rng = new_rng;
            self.audio_registry = audio_state;
            self.apply_scenario_batch(batch)?;
        }
        let mut spawn_requests = Vec::new();
        let selected_objects: HashSet<_> = self
            .objects
            .iter()
            .filter(|object| object.state.selected)
            .map(|object| object.id)
            .collect();
        let solid_mask_indices = self.active_solid_mask_indices();

        let master_list_indices = self
            .exec_list
            .iter()
            .rev()
            .copied()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect::<HashMap<_, _>>();
        let command_snapshot_object_count = self.objects.len();
        let mut command_snapshots: HashMap<ObjectId, CommandObjectSnapshot> =
            HashMap::with_capacity(command_snapshot_object_count);
        for fallback_order in 0..command_snapshot_object_count {
            let physical = self.object_physical_without_fair_fill(fallback_order);
            let object = &self.objects[fallback_order];
            let (
                procedure,
                line_connect,
                collectible,
                move_to_range,
                pathfinder,
                no_transfer_zones,
                no_push_enter,
                action_idle,
                action_disabled,
            ) = self
                .definitions
                .get(&object.definition_id)
                .map(|definition| {
                    let procedure = definition.action_library().procedure_for_entry(
                        &object.state.action.name,
                        object.state.action.act_map_index,
                    );
                    (
                        procedure,
                        definition.line_connect(),
                        definition.is_collectible(),
                        definition.move_to_range(),
                        definition.pathfinder(),
                        definition.no_transfer_zones(),
                        definition.no_push_enter(),
                        definition
                            .action_library()
                            .is_idle_state(&object.state.action),
                        definition.action_library().disables_object_for_entry(
                            &object.state.action.name,
                            object.state.action.act_map_index,
                        ),
                    )
                })
                .unwrap_or((
                    ActionProcedure::default(),
                    OCF_NORMAL,
                    false,
                    0,
                    0,
                    0,
                    0,
                    true,
                    false,
                ));
            // ExecuteCommand reads the CACHED obj->OCF (C4Command.cpp uses
            // Target->OCF etc. straight off the objects).
            let ocf = object.state.ocf;
            command_snapshots.insert(
                object.id,
                CommandObjectSnapshot {
                    id: object.id,
                    master_list_order: master_list_indices
                        .get(&object.id)
                        .copied()
                        .unwrap_or_else(|| self.exec_list.len().saturating_add(fallback_order)),
                    definition_id: object.definition_id.clone(),
                    position: object.state.position,
                    fixed_position: object.fixed_position,
                    fixed_velocity: object.fixed_velocity,
                    move_to_range,
                    pathfinder,
                    no_transfer_zones,
                    no_push_enter,
                    // C4Object::t_contact from the previous movement frame.
                    contact: object.frame_t_contact,
                    action_time: object.state.action.time,
                    shape_top: object.current_shape_rect().map(|rect| rect.y).unwrap_or(0),
                    shape_height: object
                        .current_shape_rect()
                        .map(|rect| rect.height)
                        .unwrap_or(0),
                    shape: self.object_shape_rect(object),
                    entrance: self.object_entrance_area(object),
                    status: object.state.status,
                    destroyed: object.destroyed,
                    category: object.state.category,
                    container: object.state.container,
                    action_name: object.state.action.name.clone(),
                    action_idle,
                    action_disabled,
                    action_target: object.state.action.target,
                    action_target2: object.state.action.target2,
                    action_procedure: procedure,
                    command_direction: object.state.command_direction,
                    construction: object.state.construction,
                    direction: object.state.direction,
                    physical,
                    physical_deferred: false,
                    owner: object.state.owner,
                    controller: object.state.controller,
                    base: object.state.base,
                    crew_member: object.state.crew_member,
                    selected: selected_objects.contains(&object.id),
                    alive: object.state.alive,
                    need_energy: object.state.need_energy,
                    on_fire: object.state.on_fire,
                    contents: object.state.contents.clone(),
                    commands: object.commands.command_views(),
                    line_connect,
                    ocf,
                    entrance_status: object.state.entrance_status,
                    collectible,
                },
            );
        }

        let player_snapshots: HashMap<i32, CommandPlayerSnapshot> = self
            .players
            .iter()
            .map(|(&id, player)| {
                (
                    id,
                    CommandPlayerSnapshot {
                        status: player.status(),
                        surrendered: player.surrendered(),
                        wealth: player.wealth(),
                        home_base_material: player.home_base_material().clone(),
                        home_base_material_entries: player.home_base_material_entries().to_vec(),
                        knowledge: player.knowledge().cloned().collect(),
                        hostile_to: self
                            .players
                            .keys()
                            .copied()
                            .filter(|opponent| player.is_hostile_towards(*opponent))
                            .collect(),
                    },
                )
            })
            .collect();

        let definition_snapshots = self.command_definition_snapshot_table();

        // The synced RNG rides along for command-AI draws (C4Command Get's
        // Random calls); swapped in only around step_command_stack so the
        // effect-callback paths below keep using self.rng directly.
        let command_rng = std::cell::RefCell::new(LcgRng::default());
        // C4Game::ExecObjects iterates the main list from the BACK
        // (BeginLast, C4Game.cpp:1582); `exec_list` IS that list kept
        // reversed (see the field docs and `insert_into_exec_list`).
        // Prune ids whose objects were removed since the last frame, then
        // map to indices. Any live object missing from the list would be
        // a missed insertion site — execute it last and say so loudly.
        let mut exec_list = std::mem::take(&mut self.exec_list);
        exec_list.retain(|&id| self.find_object_index(id).is_some());
        self.exec_list = exec_list;
        let mut exec_order: Vec<usize> = self
            .exec_list
            .iter()
            .filter_map(|&id| self.find_object_index(id))
            .collect();
        let listed: HashSet<usize> = exec_order.iter().copied().collect();
        for idx in 0..self.objects.len() {
            let object = &self.objects[idx];
            // Inactive objects belong only to C4GameObjects::InactiveObjects;
            // deleted/destroyed objects must never be repaired back into the
            // executable main list after ResortUnsorted removed their link.
            if !object.destroyed && object.state.status.is_active() && !listed.contains(&idx) {
                tracing::warn!(
                    object = object.id.as_u64(),
                    "active object missing from exec_list; appending"
                );
                self.insert_exec_link(self.exec_list.len(), object.id);
                exec_order.push(idx);
            }
        }
        // Command scans walk `Game.Objects` from First to Next, while
        // `exec_list` is that master list reversed. Refresh the ephemeral
        // rank after pruning/fixing/appending so HashMap iteration can never
        // decide an equal-distance command target.
        Self::refresh_command_master_list_order(&self.exec_list, &mut command_snapshots);
        let mut command_snapshot_exec_insert_generation = self.exec_list_insert_generation;
        if let Some(traced) = coach_debug_id().filter(|_| (1..=300).contains(&frame)) {
            if let Some(idx) = self.find_object_index(ObjectId::new(traced)) {
                let object = &self.objects[idx];
                crate::rng::rng_trace_line(&format!(
                    "RCOACH f{frame} x={} fix_x={} xdir={} y={} fix_y={} ydir={} mobile={} t_attach={} act={} ph={} comdir={:?} dir={:?} liq={} tgt={} tgt2={}",
                    object.state.position.x,
                    object.fixed_position.x.val(),
                    object.fixed_velocity.x.val(),
                    object.state.position.y,
                    object.fixed_position.y.val(),
                    object.fixed_velocity.y.val(),
                    object.state.mobile,
                    object.frame_t_attach,
                    object.state.action.name,
                    object.state.action.phase,
                    object.state.command_direction,
                    object.state.direction,
                    object.state.in_liquid,
                    object.state.action.target.map(|id| id.as_u64()).unwrap_or(0),
                    object.state.action.target2.map(|id| id.as_u64()).unwrap_or(0)
                ));
                let commands = object.commands.snapshot().command_names();
                if !commands.is_empty() {
                    crate::rng::rng_trace_line(&format!("RCMD f{frame} {}", commands.join(",")));
                }
            }
        }
        // LC_EXECDBG=<frame>: dump the full exec order for that frame
        // (bare LC_EXECDBG keeps the legacy f1-2 intro-window dump).
        if let Ok(raw) = std::env::var("LC_EXECDBG") {
            let requested: Option<u64> = raw.trim().parse().ok();
            let hit = match requested {
                Some(target) => frame == target,
                None => (1..=2).contains(&frame),
            };
            if hit {
                for &idx in &exec_order {
                    let id = self.objects[idx].id.as_u64();
                    if requested.is_some() || (1449..=1460).contains(&id) {
                        crate::rng::rng_trace_line(&format!(
                            "REXEC f{frame} {id} {}",
                            self.objects[idx].definition_id.as_str()
                        ));
                    }
                }
            }
        }
        self.exec_cursor = Some(0);
        let mut previous_exec_object = None;
        while self
            .exec_cursor
            .is_some_and(|cursor| cursor < self.exec_list.len())
        {
            // Native removal paths can leave through many early-continue
            // arms. Clear their layer pointers before the next object gets
            // a callback-visible world snapshot; the post-loop sweep handles
            // a removal by the final object.
            if previous_exec_object.take().is_some_and(|id| {
                self.find_object_index(id).is_some_and(|index| {
                    self.objects[index].destroyed
                        || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
                })
            }) {
                self.clear_destroyed_object_layers();
            }
            if command_snapshot_exec_insert_generation != self.exec_list_insert_generation {
                Self::refresh_command_master_list_order(&self.exec_list, &mut command_snapshots);
                command_snapshot_exec_insert_generation = self.exec_list_insert_generation;
            }
            let cursor = self.exec_cursor.unwrap_or_default();
            let current_id = self.exec_list[cursor];
            previous_exec_object = Some(current_id);
            self.exec_cursor = Some(cursor + 1);
            let Some(idx) = self.find_object_index(current_id) else {
                continue;
            };
            // UpdateOCF runs first in C4Object::Execute (C4Object.cpp:1058).
            self.refresh_object_ocf(idx);
            let build_target = (self.objects[idx].commands.front_command_name() == Some("Build"))
                .then(|| {
                    self.objects[idx]
                        .commands
                        .command_views()
                        .first()
                        .and_then(|command| command.target)
                })
                .flatten();
            if let Some(target_id) = build_target {
                let live_target = self
                    .find_object_index(target_id)
                    .map(|target_index| self.live_command_snapshot(target_index));
                let completing = live_target
                    .as_ref()
                    .is_some_and(|target| target.construction >= FULL_CON);
                if let Some(target) = live_target {
                    command_snapshots.insert(target_id, target);
                }
                if completing {
                    // Completion scans every live command stack and each
                    // co-builder's current contents. Foreign callbacks can
                    // mutate either after that object's own Execute, so
                    // refresh the full table where C++ calls FindObjectByCommand.
                    let live_snapshots = (0..self.objects.len())
                        .map(|index| {
                            let snapshot = self.live_command_snapshot(index);
                            (snapshot.id, snapshot)
                        })
                        .collect::<Vec<_>>();
                    command_snapshots.extend(live_snapshots);
                }
            }
            let Some(idx) = self.find_object_index(current_id) else {
                continue;
            };
            // C++ command handlers read the live object and command lists.
            // Refresh the executing object after any earlier object in this
            // frame may have changed it; completed objects are likewise
            // written back below for later command scans.
            let mut actor_snapshot = self.live_command_snapshot(idx);
            actor_snapshot.physical_deferred = self.object_physical_will_fill_fair_cache(idx);
            command_snapshots.insert(current_id, actor_snapshot);
            let (mut definition_id, mut action_library) = self.object_definition_context(idx)?;
            let previous_action_state = self.objects[idx].state.action.clone();
            let previous_action_name = previous_action_state.name.clone();
            let mut landscape_slot = self.landscape.take();
            let command_gravity = self.physics.gravity_as_c4fixed();
            let (
                queued_spawns,
                queue_destroy,
                queue_events,
                container_updates,
                command_events,
                queue_definition_changed,
                queue_change_def_reinsert,
                (
                    object_id,
                    previous_owner,
                    previous_crew,
                    new_owner,
                    new_crew,
                    previous_status,
                    new_status,
                ),
            ) = {
                let object = &mut self.objects[idx];
                let object_id = object.id;
                let current_position = object.state.position;
                let builder_snapshot = command_snapshots
                    .get(&object_id)
                    .expect("command snapshot exists");
                command_rng.replace(std::mem::take(&mut self.rng));
                let command_context = CommandRuntimeContext {
                    rng: Some(&command_rng),
                    frame: self.frame,
                    position: current_position,
                    landscape: landscape_slot.as_ref(),
                    object: builder_snapshot,
                    objects: &command_snapshots,
                    players: &player_snapshots,
                    definitions: definition_snapshots.as_ref(),
                    structures_need_energy: self.structures_need_energy,
                    base_buy_enabled: self.base_buy_enabled,
                    base_sell_enabled: self.base_sell_enabled,
                    transfer_zones: &self.transfer_zones,
                };
                let step_result = object.step_command_stack(command_context, command_gravity);
                self.rng = command_rng.take();
                if let Some(result) = step_result {
                    if result.update.is_some() || !result.events.is_empty() {
                        let update = result.update.unwrap_or_default();
                        let mut queued = QueuedCommand::immediate(update);
                        if !result.events.is_empty() {
                            queued = queued.with_events(result.events.clone());
                        }
                        object.command_queue.push_front(queued);
                    }
                }
                let previous_owner = object.state.owner;
                let previous_crew = object.state.crew_member;
                let previous_status = object.state.status;
                let outcome = object.execute_command_queue(
                    &self.physics,
                    &self.materials,
                    landscape_slot.as_mut(),
                    &action_library,
                    &self.definitions,
                    &self.players,
                );
                let new_owner = object.state.owner;
                let new_crew = object.state.crew_member;
                let new_status = object.state.status;
                (
                    outcome.spawns,
                    outcome.destroy,
                    outcome.effect_events,
                    outcome.container_updates,
                    outcome.command_events,
                    outcome.definition_changed,
                    outcome.change_def_reinsert,
                    (
                        object.id,
                        previous_owner,
                        previous_crew,
                        new_owner,
                        new_crew,
                        previous_status,
                        new_status,
                    ),
                )
            };
            self.landscape = landscape_slot;
            self.update_inactive_list_for_status_change(object_id, previous_status, new_status);
            self.update_selection_for_state_change(
                object_id,
                previous_owner,
                previous_crew,
                new_owner,
                new_crew,
            );

            if queue_definition_changed {
                self.update_sector_for_index(idx);
                self.update_solid_mask(idx);
                self.refresh_object_ocf(idx);
            }

            for update in container_updates {
                self.apply_container_change(update.object_id, update.previous, update.new, false)?;
            }
            if queue_change_def_reinsert {
                self.reinsert_change_def_contents_link(object_id)?;
            }

            // GetFairCrewPhysical is ordinary script. Its callback can
            // mutate any object which a later ExecObjects command will read,
            // so the frame-wide structural table from before that callback
            // is no longer valid even though only the actor's captured
            // physical value feeds the retained continuation.
            let mut resolved_command_physical = false;
            for event in command_events {
                resolved_command_physical |= self.apply_command_event(event)?;
            }
            if resolved_command_physical {
                command_snapshots = (0..self.objects.len())
                    .map(|index| {
                        let snapshot = self.live_command_snapshot(index);
                        (snapshot.id, snapshot)
                    })
                    .collect();
                Self::refresh_command_master_list_order(&self.exec_list, &mut command_snapshots);
                command_snapshot_exec_insert_generation = self.exec_list_insert_generation;
            }

            if !queue_events.is_empty() {
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
                        .get(&definition_id)
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
                        queue_events,
                        global_view,
                        &mut self.environment,
                        self.physics,
                        self.frame,
                        world.clone(),
                        self.audio_registry.clone(),
                    )?
                };
                let was_deferred = self.solid_mask_staging.defer_solid_mask_updates;
                let mut outermost = self.stage_host_solid_mask_operations(
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
                        self.apply_container_change(
                            object_id,
                            previous_container,
                            new_container,
                            false,
                        )?;
                    }
                    if effect_change_def_reinsert.unwrap_or(false) {
                        self.reinsert_change_def_contents_link(object_id)?;
                    }
                    Ok(())
                })();
                outermost |= !was_deferred && self.solid_mask_staging.defer_solid_mask_updates;
                self.finish_host_solid_mask_operations(outermost, fold_result)?;
            }

            self.finish_object_command_execution(object_id)?;
            let Some(idx) = self.find_object_index(object_id) else {
                continue;
            };
            // ExecuteCommand and its synchronous callbacks run before every
            // later C4Object::Execute stage. ChangeDef therefore changes the
            // live Def/ActMap used by ExecAction, movement, effects and Timer
            // in this same frame — including a swap from queue effects or
            // ControlCommandFinished rather than the direct queued delta.
            if !queued_spawns.is_empty() {
                spawn_requests.extend(queued_spawns);
            }

            if queue_destroy || self.objects[idx].destroyed {
                continue;
            }

            if !self.objects[idx].state.status.is_active() {
                continue;
            }

            dbg_stage(&self.objects[idx], "POSTCMD");
            // ExecAction captures pAction before procedure steering. SetDir
            // may replace the live action through TurnAction, but C++ keeps
            // this entry for phase advance through the end of ExecAction.
            let exec_action_source = self.objects[idx].state.action.name.clone();
            let exec_action_index = self.objects[idx].state.action.act_map_index;
            let (exec_action_definition_id, exec_action_library) =
                self.object_definition_context(idx)?;
            let mut exec_action_physical = None;
            let exec_action_returned_early =
                self.apply_physics_at_index_inner(idx, Some(&mut exec_action_physical))?;
            if self.objects[idx].destroyed {
                continue;
            }
            dbg_stage(&self.objects[idx], "POSTACT");

            // Phase advance runs at the END of ExecAction
            // (C4Object.cpp:5440-5465) — AFTER the procedure steering
            // updated xdir/ydir, so WALK/SCALE/HANGLE advances read THIS
            // frame's velocity (the old pre-steer order lagged BISO's
            // walk phase one step behind on every acceleration frame).
            // An early `return` inside ExecAction (the free-fall swim
            // exit) skips it entirely; movement below still runs.
            let mut allow_deleted_phase_end_start = false;
            if !exec_action_returned_early {
                let physical_for_advance = exec_action_physical
                    .unwrap_or_else(|| self.object_physical_without_fair_fill(idx));
                let mut advance_outcome = {
                    let object = &mut self.objects[idx];
                    // iPhaseAdvance (C4Object.cpp:4696): WALK fixtoi(|xdir|*10)
                    // (:4787-4789), SCALE fixtoi(|ydir|*14) (:4830-4832),
                    // HANGLE fixtoi(|xdir|*10) (:4867-4869), PUSH the same
                    // for nonzero xdir while retaining the default 1 at
                    // rest (:5106-5108), PULL the same with a zero baseline
                    // (:5189-5192), SWIM
                    // fixtoi(swimlimit*10) with the PHYSICAL limit, not the
                    // velocity (:5010-ish "iPhaseAdvance = fixtoi(lLimit*10)"),
                    // DIG fixtoi(diglimit*40) (:4894-4895); everything else 1.
                    let phase_advance = match exec_action_library
                        .procedure_for_entry(&exec_action_source, exec_action_index)
                    {
                        ActionProcedure::Walk | ActionProcedure::Hang => {
                            math::fixtoi(object.fixed_velocity.x.abs() * 10)
                        }
                        ActionProcedure::Push if object.fixed_velocity.x.is_nonzero() => {
                            math::fixtoi(object.fixed_velocity.x.abs() * 10)
                        }
                        ActionProcedure::Push => 1,
                        ActionProcedure::Pull => math::fixtoi(object.fixed_velocity.x.abs() * 10),
                        ActionProcedure::Scale => math::fixtoi(object.fixed_velocity.y.abs() * 14),
                        ActionProcedure::Swim => {
                            math::fixtoi(math::val_by_physical(160, physical_for_advance.swim) * 10)
                        }
                        ActionProcedure::Dig
                            if object.state.command_direction == CommandDirection::Stop =>
                        {
                            0
                        }
                        ActionProcedure::Dig => {
                            math::fixtoi(math::val_by_physical(125, physical_for_advance.dig) * 40)
                        }
                        _ => 1,
                    };
                    exec_action_library.advance_state_from_entry_by(
                        &mut object.state.action,
                        &exec_action_source,
                        exec_action_index,
                        phase_advance,
                        false,
                    )
                };

                if let Some(event) = advance_outcome.phase_event.take() {
                    if let Some(callback) = exec_action_library
                        .phase_callback_for_entry(&event.action, event.act_map_index)
                    {
                        // C++ runs the PhaseCall AFTER `Phase += Step` but
                        // BEFORE the length-wrap SetAction
                        // (C4Object.cpp:5448-5462): the callback sees the
                        // POST-advance phase under the OLD action. The
                        // event snapshot carries exactly that pair; the
                        // rest of the state is live.
                        let mut state_snapshot = self.objects[idx].script_state_snapshot();
                        state_snapshot.action.name = event.live_action;
                        state_snapshot.action.act_map_index = event.live_act_map_index;
                        state_snapshot.action.phase = event.phase;
                        self.invoke_action_callback(
                            idx,
                            ActionCallbackKind::Phase,
                            &event.action,
                            event.act_map_index,
                            Some(callback),
                            Some(state_snapshot),
                            None,
                            Some(&exec_action_definition_id),
                        )?;
                    }
                }

                // Only after PhaseCall returns does C++ compare the LIVE
                // phase against the stale pAction Length and call ordinary
                // SetAction(NextAction). Callback-side SetPhase/SetAction can
                // therefore suppress or alter this transition.
                let transition_previous_state = self.objects[idx].state.action.clone();
                if let Some(phase_end) = advance_outcome.phase_end.take() {
                    let (current_definition_id, current_action_library) =
                        self.object_definition_context(idx)?;
                    // PhaseCall is synchronous and may have changed both
                    // Con and Def. The ensuing SetAction(NextAction) reads
                    // that LIVE pair before applying the incomplete-object
                    // ActIdle coercion (C4Object.cpp:4127-4130,5480-5485).
                    let active_action_allowed = self.objects[idx].state.construction >= FULL_CON
                        || self
                            .definitions
                            .get(&current_definition_id)
                            .is_some_and(Definition::incomplete_activity);
                    advance_outcome.wrapped = exec_action_library
                        .finish_phase_end_against_with_activity(
                            &mut self.objects[idx].state.action,
                            &phase_end,
                            &current_action_library,
                            active_action_allowed,
                        );
                }

                // A successful SetAction always resyncs fixed coords; Hold
                // and NoOtherAction rejection leave them untouched.
                if advance_outcome.wrapped {
                    {
                        let object = &mut self.objects[idx];
                        object.fixed_position =
                            FixedVec2::from_ints(object.state.position.x, object.state.position.y);
                        object.record_action_event(
                            transition_previous_state,
                            ActionTransitionKind::Natural,
                        );
                        // A PhaseCall may have assigned removal already. C++
                        // nevertheless executes the stale pAction phase-end
                        // SetAction, including its new action's StartCall; the
                        // callback dispatcher notices Status=0 only afterward
                        // and suppresses the remaining EndCall.
                        allow_deleted_phase_end_start = object.destroyed
                            || matches!(object.state.status, ObjectStatus::Deleted);
                    }
                    // C4Object::SetAction refreshes OCF after selecting the
                    // actual (possibly incomplete-coerced) action and before
                    // StartCall/EndCall (C4Object.cpp:4165-4183).
                    self.refresh_object_ocf(idx);
                }
            }

            // ExecAction's action transitions fire their StartCall/EndCall
            // INSIDE SetAction (C4Object.cpp:4160-4185), i.e. BEFORE
            // ExecMovement (C4Object::Execute order, :1074/:1079). A
            // StartCall that SetActions again (the coach's Driving ->
            // "Drive2") must take its fix_x/fix_y resync (:4154-4155) at
            // the PRE-movement pixel — draining only after movement let
            // the snap eat the sub-pixel remainder DoMovement just built.
            // Transitions recorded during movement/effects still drain at
            // the post-movement call below.
            self.trigger_action_callbacks_impl(
                idx,
                Some(previous_action_name.clone()),
                allow_deleted_phase_end_start,
            )?;
            if self.objects[idx].destroyed {
                continue;
            }
            (definition_id, action_library) = self.object_definition_context(idx)?;
            dbg_stage(&self.objects[idx], "PREMOVE");

            // C4Object::ExecMovement (C4Movement.cpp:553-616): contained
            // objects copy the container's motion (:556-561), C4D_StaticBack
            // never moves (:564, a MASK test unlike Init's equality), only
            // Mobile objects run DoMovement (:567), and a resting object
            // re-mobilizes with zeroed dirs and pixel-snapped fixed coords
            // on the Tick10 pulse (:576-587; counters advance before
            // objects execute, C4Game.cpp:1888).
            let exec_movement_contained = self.objects[idx].state.container.is_some();
            let exec_movement_static_back =
                self.objects[idx].state.category & CATEGORY_STATIC_BACK != 0;
            if exec_movement_contained {
                self.copy_motion_from_container(idx);
            } else if !exec_movement_static_back {
                if self.objects[idx].state.mobile {
                    // DoMovement itself owns the mask lifecycle: DigFree and
                    // pre-motion contacts see the put mask, the first
                    // DoMotion removes it, and the tail always re-puts it.
                    let _movement_outcome = self.exec_mobile_object_movement(
                        idx,
                        &action_library,
                        &definition_id,
                        &solid_mask_indices,
                    )?;
                } else {
                    // Static objects stabilize every frame
                    // (C4Movement.cpp:579).
                    self.stabilize_object(idx, &solid_mask_indices)?;
                    if frame.is_multiple_of(10) {
                        // Gravity mobilization (C4Movement.cpp:581-586).
                        let object = &mut self.objects[idx];
                        object.fixed_velocity = FixedVec2::ZERO;
                        object.state.velocity = Vector2::ZERO;
                        object.rotation_velocity = C4Fixed::ZERO;
                        object.fixed_position = FixedVec2::new(
                            itofix(object.state.position.x),
                            itofix(object.state.position.y),
                        );
                        object.fixed_rotation = itofix(object.state.rotation);
                        object.state.mobile = true;
                    }

                    // C4Object::ExecMovement applies this raw assignment after
                    // its static leg too (C4Movement.cpp:611-612).
                    let non_rotateable = self
                        .definitions
                        .get(&self.objects[idx].definition_id)
                        .is_some_and(|definition| definition.rotateable() == 0);
                    if non_rotateable {
                        self.objects[idx].state.rotation = 0;
                    }
                }
            }

            // C4Object::ExecMovement removes ordinary objects whose origin
            // crosses the unbounded landscape sides or bottom, before effects
            // and life execute (src/C4Movement.cpp:598-617).
            let crossed_landscape_bounds = !exec_movement_contained
                && !exec_movement_static_back
                && self.object_should_be_removed_out_of_bounds(idx);
            if crossed_landscape_bounds {
                // Rust defers SetAction callbacks; C++ ran movement-induced
                // Start/Abort calls inline before reaching this predicate.
                // Drain them first, then re-check because a callback may
                // move, save, attach, or delete the object.
                self.trigger_action_callbacks(idx, Some(previous_action_name.clone()))?;
                if self.objects[idx].destroyed
                    || matches!(self.objects[idx].state.status, ObjectStatus::Deleted)
                {
                    continue;
                }
                self.update_sector_for_index(idx);
                if self.object_should_be_removed_out_of_bounds(idx) {
                    self.assign_out_of_bounds_removal(idx)?;
                    continue;
                }
            }

            dbg_stage(&self.objects[idx], "POSTMOVE");
            self.apply_landscape_at_index(idx);
            // Masks follow every state change this frame
            // (C4Object::UpdateSolidMask fires from UpdatePos/Enter/
            // Exit/DoCon; the end-of-exec update covers the net state).
            self.update_solid_mask(idx);
            self.update_sector_for_index(idx);
            // Script effect timers execute HERE in C++ — pEffects->Execute
            // follows ExecAction and ExecMovement inside C4Object::Execute
            // (C4Object.cpp:1069-1090): an action set by a timer callback
            // gets its first PhaseDelay increment the NEXT frame, and the
            // callbacks read POST-movement state. Script effects (low
            // priority) run before the internal fire (priority 100 —
            // C4Effect execution is priority-ordered).
            // C4Effect::Execute walks the LIVE list one node at a time. Do
            // not pre-advance or snapshot the suffix: callbacks may kill a
            // later node or insert a new upper node that must execute in
            // this same frame (C4Effect.cpp:319-363).
            let mut effect_cursor = None;
            loop {
                if self.objects[idx].destroyed || !self.objects[idx].state.status.is_active() {
                    break;
                }
                let Some((next_cursor, timer_event)) =
                    self.objects[idx].advance_effect_frame_cursor(effect_cursor)
                else {
                    break;
                };
                effect_cursor = Some(next_cursor);
                let Some(event) = timer_event else {
                    continue;
                };
                definition_id = self.objects[idx].definition_id.clone();

                // The engine-internal fire timer (FnFxFireTimer,
                // C4Effect.cpp:643-658 → C4Object::ExecFire) executes at
                // its live-list position unless a script overload shadows
                // the engine function (C4Script.cpp:6995).
                let native_fire = event.effect.name == C4FX_FIRE
                    && !self.effect_has_script_callback(&event.effect, &definition_id, "Timer");
                if !native_fire {
                    self.dispatch_object_effect_events(idx, &definition_id, vec![event])?;
                    continue;
                }

                let entry = event.effect;
                if !entry.start_dispatched {
                    // Runtime constructors dispatch native FxFireStart in
                    // their Started event. Loaded effects also mark Start as
                    // complete. Never reinterpret persistent Fire EffectVars
                    // (Mode/CausedBy/Blasted/Incinerating) as constructor rVals.
                    if let Some(pending) = self.objects[idx]
                        .state
                        .effects
                        .iter_mut()
                        .find(|effect| effect.number == entry.number)
                    {
                        pending.start_dispatched = true;
                    }
                }
                let stop_events = self.exec_object_fire(idx, frame, entry.number);
                definition_id = self.objects[idx].definition_id.clone();
                if !stop_events.is_empty()
                    && !self.objects[idx].destroyed
                    && self.objects[idx].state.status.is_active()
                {
                    self.dispatch_object_effect_events(idx, &definition_id, stop_events)?;
                }
            }
            if self.objects[idx].destroyed || !self.objects[idx].state.status.is_active() {
                // pEffects->Execute may remove the object; C++ returns
                // before ExecLife/ExecBase/Timer (C4Object.cpp:1087-1090).
                continue;
            }
            // ExecLife runs after the fire effect (C4Object.cpp:1074-1080)
            self.exec_object_life(idx, frame)?;
            // ExecBase runs after ExecLife (C4Object.cpp:1082-1083)
            self.exec_object_base(idx, frame)?;
            if self.objects[idx].destroyed || !self.objects[idx].state.status.is_active() {
                continue;
            }
            definition_id = self.objects[idx].definition_id.clone();

            // Def TimerCall (C4Object::Execute, C4Object.cpp:1085-1091):
            // Timer++ every Execute; reaching Def->Timer resets the counter
            // and Execs Def->TimerCall. C++ resolves the name at link time
            // (missing -> nullptr, no call); the Exec itself is fail-safe.
            if self.objects[idx].state.status.is_active() && !self.objects[idx].destroyed {
                if std::env::var("LC_RUST_RNG_TRACE").is_ok()
                    && self.objects[idx].id.as_u64() == 569
                    && (16..=18).contains(&self.frame)
                {
                    tracing::warn!(
                        frame = self.frame,
                        action = %self.objects[idx].state.action.name,
                        phase = self.objects[idx].state.action.phase,
                        in_liquid = self.objects[idx].state.in_liquid,
                        "RSNKTIMER"
                    );
                }
                let timer_call = self.definitions.get(&definition_id).and_then(|definition| {
                    let interval = definition.timer().max(1);
                    let object_timer = &mut self.objects[idx].state.timer;
                    *object_timer += 1;
                    (*object_timer >= interval)
                        .then(|| {
                            *object_timer = 0;
                            definition.timer_callback()
                        })
                        .flatten()
                });
                if let Some(callback) = timer_call {
                    tolerate_script_error(self.call_object_callback(idx, &callback, Vec::new()))?;
                }
                if self.objects[idx].destroyed
                    || matches!(self.objects[idx].state.status, ObjectStatus::Deleted)
                {
                    continue;
                }
            }
            (definition_id, action_library) = self.object_definition_context(idx)?;

            // C4Object::Execute advances its active menu immediately after
            // TimerCall (C4Object.cpp:1085-1093; C4Menu.cpp:990-1000).
            if let Some(menu) = self.objects[idx]
                .state
                .menu
                .as_mut()
                .filter(|menu| menu.text_progressing)
            {
                let _ = menu.set_text_progress(1, true);
            }

            let object_id = self.objects[idx].id;
            // Step is the command-DSL fixture callback; real content has no
            // Step function (call_step would return an empty batch) — skip
            // the world snapshot, the state clone and the random draw it
            // would never consume.
            let definition_has_step = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?
                .has_step;
            let command = if definition_has_step {
                let state_snapshot = Rc::new(self.objects[idx].script_state_snapshot());
                let random = self.next_random_i32();

                let rng_state = self.rng.clone();
                let (command, audio_state, new_rng, next_object_id) = {
                    let definition = self
                        .definitions
                        .get(&definition_id)
                        .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                    let world = self.host_world_context_for_object_with_snapshot(
                        idx,
                        Rc::clone(&state_snapshot),
                    );
                    definition.call_step(
                        state_snapshot.as_ref(),
                        object_id,
                        frame,
                        random,
                        rng_state,
                        &self.global_effects,
                        self.physics,
                        self.environment,
                        world,
                        self.game_over_triggered,
                        self.audio_registry.clone(),
                    )?
                };
                self.rng = new_rng;
                self.sync_next_object_id(next_object_id);
                self.audio_registry = audio_state;
                command
            } else {
                CommandBatch::default()
            };

            let CommandBatch {
                delta,
                spawns,
                destroy,
                commands,
                command_ops,
                effects,
                other_objects,
                global_effects,
                environment,
                physics,
                landscape_ops,
                solid_mask_operations,
                host_raster_preview: command_host_raster_preview,
                particles,
                transfer_zones,
                audio,
                messages,
                player_commands,
                object_order_commands,
                next_mission_commands,
                trigger_game_over,
                script_go,
                script_counter,
            } = command;

            let was_deferred = self.solid_mask_staging.defer_solid_mask_updates;
            let mut outermost = self.stage_host_solid_mask_operations(
                solid_mask_operations,
                command_host_raster_preview,
            );
            let command_fold_result = (|| -> Result<(), EngineError> {
                let change_def = delta.change_def.clone();
                let change_def_reinsert = delta.change_def_reinsert;

                let action_library = change_def
                    .as_deref()
                    .and_then(|new_def| {
                        self.apply_change_object_def(idx, new_def);
                        self.shared_action_library_for(&self.objects[idx].definition_id)
                    })
                    .unwrap_or_else(|| action_library.clone());

                if let Some(go) = script_go {
                    self.scenario_script_go = go;
                }
                if let Some(counter) = script_counter {
                    self.scenario_script_counter = counter;
                }
                if trigger_game_over {
                    self.request_game_over()?;
                }

                if !player_commands.is_empty() {
                    self.apply_player_commands(player_commands)?;
                }
                self.pending_object_order_commands
                    .extend(object_order_commands);
                self.apply_next_mission_commands(next_mission_commands);

                if !landscape_ops.is_empty() {
                    self.apply_landscape_operations(landscape_ops);
                }

                if let Some(update) = environment {
                    self.apply_environment_delta(&update);
                }
                if let Some(delta) = physics {
                    self.apply_physics_delta(delta);
                }

                let mut effect_events = Vec::new();
                if !messages.is_empty() {
                    for command in messages {
                        self.messages.apply_command(command);
                    }
                }
                let (
                    object_id,
                    previous_owner,
                    previous_crew,
                    new_owner,
                    new_crew,
                    previous_status,
                    new_status,
                    container_change,
                ) = {
                    let object = &mut self.objects[idx];
                    let previous_owner = object.state.owner;
                    let previous_crew = object.state.crew_member;
                    let previous_status = object.state.status;
                    let mut container_change = None;
                    let callbacks_dispatched = delta
                        .action
                        .as_ref()
                        .map(|action| action.callbacks_dispatched)
                        .unwrap_or(false);
                    let delta_outcome = object.apply_delta(&delta, &action_library);
                    if let Some(change) = delta_outcome.action_change {
                        if !callbacks_dispatched {
                            object
                                .record_action_event(change.previous, ActionTransitionKind::Forced);
                        }
                    }
                    if let Some(change) = delta_outcome.container_change {
                        container_change = Some(change);
                    }
                    let mut applied = object.apply_effect_commands(&effects);
                    effect_events.append(&mut applied);
                    if !matches!(
                        action_library.procedure_for_entry(
                            &object.state.action.name,
                            object.state.action.act_map_index,
                        ),
                        ActionProcedure::Flight
                    ) {
                        object.clamp_velocity(&self.physics);
                    }
                    if destroy {
                        effect_events.extend(object.mark_destroyed());
                    }
                    if !command_ops.is_empty() {
                        object.apply_command_operations(command_ops);
                    }
                    if !commands.is_empty() {
                        object.enqueue_commands(commands);
                    }
                    (
                        object.id,
                        previous_owner,
                        previous_crew,
                        object.state.owner,
                        object.state.crew_member,
                        previous_status,
                        object.state.status,
                        container_change,
                    )
                };
                self.update_inactive_list_for_status_change(object_id, previous_status, new_status);
                self.update_sector_for_index(idx);
                if !audio.is_empty() {
                    self.pending_audio.extend(audio);
                }
                self.update_selection_for_state_change(
                    object_id,
                    previous_owner,
                    previous_crew,
                    new_owner,
                    new_crew,
                );
                if let Some((previous_container, new_container)) = container_change {
                    self.apply_container_change(
                        object_id,
                        previous_container,
                        new_container,
                        false,
                    )?;
                }
                if change_def_reinsert {
                    self.reinsert_change_def_contents_link(object_id)?;
                }
                if change_def.is_some() {
                    self.update_solid_mask(idx);
                    self.refresh_object_ocf(idx);
                }

                self.apply_particle_commands(particles);
                if !transfer_zones.is_empty() {
                    self.apply_transfer_zone_commands(transfer_zones)?;
                }

                if !global_effects.is_empty() {
                    self.apply_global_effect_commands(&global_effects);
                }

                if !effect_events.is_empty() {
                    let previous_container = self.objects[idx].state.container;
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
                            .get(&definition_id)
                            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                        let definitions_ref = &self.definitions;
                        let global_view = self.global_effects.clone();
                        let rng_state = self.rng.clone();
                        let object = &mut self.objects[idx];
                        Self::run_effect_events_for_object(
                            definition,
                            definitions_ref,
                            self.game_over_triggered,
                            rng_state,
                            object_id,
                            object,
                            effect_events,
                            global_view,
                            &mut self.environment,
                            self.physics,
                            self.frame,
                            world.clone(),
                            self.audio_registry.clone(),
                        )?
                    };
                    self.stage_host_solid_mask_operations(
                        effect_solid_mask_operations,
                        effect_host_raster_preview,
                    );
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
                    if !player_commands.is_empty() {
                        self.apply_player_commands(player_commands)?;
                    }
                    self.pending_object_order_commands
                        .extend(object_order_commands);
                    self.apply_next_mission_commands(next_mission_commands);
                    if !landscape_ops.is_empty() {
                        self.apply_landscape_operations(landscape_ops);
                    }
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
                    let new_container = self.objects[idx].state.container;
                    if !global_cmds.is_empty() {
                        self.apply_global_effect_commands(&global_cmds);
                    }
                    self.apply_particle_commands(emitted_particles);
                    if previous_container != new_container {
                        self.apply_container_change(
                            object_id,
                            previous_container,
                            new_container,
                            false,
                        )?;
                    }
                    if effect_change_def_reinsert.unwrap_or(false) {
                        self.reinsert_change_def_contents_link(object_id)?;
                    }
                }
                self.update_sector_for_index(idx);

                self.apply_nested_object_outcomes(other_objects)?;

                Ok(())
            })();
            outermost |= !was_deferred && self.solid_mask_staging.defer_solid_mask_updates;
            self.finish_host_solid_mask_operations(outermost, command_fold_result)?;

            self.trigger_action_callbacks(idx, Some(previous_action_name))?;
            self.update_sector_for_index(idx);

            if self.objects[idx].destroyed {
                continue;
            }

            self.apply_landscape_at_index(idx);
            self.update_sector_for_index(idx);
            let (
                procedure,
                line_connect,
                collectible,
                move_to_range,
                pathfinder,
                no_transfer_zones,
                no_push_enter,
                action_idle,
                action_disabled,
            ) = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .map(|definition| {
                    (
                        definition.action_library().procedure_for_entry(
                            &self.objects[idx].state.action.name,
                            self.objects[idx].state.action.act_map_index,
                        ),
                        definition.line_connect(),
                        definition.is_collectible(),
                        definition.move_to_range(),
                        definition.pathfinder(),
                        definition.no_transfer_zones(),
                        definition.no_push_enter(),
                        definition
                            .action_library()
                            .is_idle_state(&self.objects[idx].state.action),
                        definition.action_library().disables_object_for_entry(
                            &self.objects[idx].state.action.name,
                            self.objects[idx].state.action.act_map_index,
                        ),
                    )
                })
                .unwrap_or((
                    action_library.procedure_for_entry(
                        &self.objects[idx].state.action.name,
                        self.objects[idx].state.action.act_map_index,
                    ),
                    OCF_NORMAL,
                    false,
                    0,
                    0,
                    0,
                    0,
                    action_library.is_idle_state(&self.objects[idx].state.action),
                    action_library.disables_object_for_entry(
                        &self.objects[idx].state.action.name,
                        self.objects[idx].state.action.act_map_index,
                    ),
                ));
            // ExecuteCommand reads the CACHED obj->OCF (refreshed at this
            // object's Execute-start, C4Object.cpp:1058).
            let ocf = self.objects[idx].state.ocf;
            let master_list_order = self
                .exec_list
                .iter()
                .rev()
                .position(|&id| id == object_id)
                .unwrap_or_else(|| self.exec_list.len().saturating_add(idx));
            command_snapshots.insert(
                object_id,
                CommandObjectSnapshot {
                    id: object_id,
                    master_list_order,
                    definition_id: self.objects[idx].definition_id.clone(),
                    position: self.objects[idx].state.position,
                    fixed_position: self.objects[idx].fixed_position,
                    fixed_velocity: self.objects[idx].fixed_velocity,
                    move_to_range,
                    pathfinder,
                    no_transfer_zones,
                    no_push_enter,
                    contact: self.objects[idx].frame_t_contact,
                    action_time: self.objects[idx].state.action.time,
                    shape_top: self.objects[idx]
                        .current_shape_rect()
                        .map(|rect| rect.y)
                        .unwrap_or(0),
                    shape_height: self.objects[idx]
                        .current_shape_rect()
                        .map(|rect| rect.height)
                        .unwrap_or(0),
                    shape: self.object_shape_rect(&self.objects[idx]),
                    entrance: self.object_entrance_area(&self.objects[idx]),
                    status: self.objects[idx].state.status,
                    destroyed: self.objects[idx].destroyed,
                    category: self.objects[idx].state.category,
                    container: self.objects[idx].state.container,
                    action_name: self.objects[idx].state.action.name.clone(),
                    action_idle,
                    action_disabled,
                    action_target: self.objects[idx].state.action.target,
                    action_target2: self.objects[idx].state.action.target2,
                    action_procedure: procedure,
                    command_direction: self.objects[idx].state.command_direction,
                    construction: self.objects[idx].state.construction,
                    direction: self.objects[idx].state.direction,
                    physical: self.object_physical_without_fair_fill(idx),
                    physical_deferred: false,
                    owner: self.objects[idx].state.owner,
                    controller: self.objects[idx].state.controller,
                    base: self.objects[idx].state.base,
                    crew_member: self.objects[idx].state.crew_member,
                    selected: self.objects[idx].state.selected,
                    alive: self.objects[idx].state.alive,
                    need_energy: self.objects[idx].state.need_energy,
                    on_fire: self.objects[idx].state.on_fire,
                    contents: self.objects[idx].state.contents.clone(),
                    commands: self.objects[idx].commands.command_views(),
                    line_connect,
                    ocf,
                    entrance_status: self.objects[idx].state.entrance_status,
                    collectible,
                },
            );
            spawn_requests.extend(spawns);
        }
        self.exec_cursor = None;

        // CreateObject is synchronous inside C4Object::Execute. Rust defers
        // materialization only so a freshly linked object cannot enter the
        // current execution-list walk; once that walk closes it must exist
        // for CrossCheck, world systems, and especially Players.Execute's
        // Tick35 CrewCnt snapshot.
        self.process_spawn_queue(spawn_requests)?;

        // AssignRemoval calls Game.ClearPointers before returning. Native
        // engine-owned removal paths do not pass through the synchronous
        // script host, so clear their pLayer references at the last common
        // seam before this frame's CrossCheck.
        self.clear_destroyed_object_layers();

        // C4GameObjects::CrossCheck runs once per frame after object        // execution (C4Game.cpp ExecObjects → Objects.CrossCheck()).
        self.cross_check(frame)?;
        self.execute_object_order_commands();

        for index in 0..self.objects.len() {
            if self.objects[index].destroyed {
                // AssignRemoval destroys the mask (C4Object.cpp:5647).
                self.remove_solid_mask(index);
            }
        }
        self.detach_destroyed_objects()?;
        let object_count_before_retain = self.objects.len();
        self.objects.retain(|object| !object.destroyed);
        if self.objects.len() != object_count_before_retain {
            self.note_objects_changed();
        }
        self.rebuild_sectors();
        let alive: HashSet<_> = self.objects.iter().map(|object| object.id).collect();
        // C4Game::Execute phase order (C4Game.cpp:810-822): ExecObjects
        // runs FIRST; then GlobalEffects, PXS, Particles, MassMover,
        // Weather, Landscape, Players, Messages, Script. Objects observe
        // the PREVIOUS frame's weather/PXS state and their RNG draws
        // precede every world-system draw within the frame.
        self.tick_global_effects()?;
        self.tick_pxs();
        self.tick_particles();
        self.tick_mass_movers();
        self.weather_events.clear();
        if let Some(points) = self.environment.advance_frame(&mut self.rng, frame) {
            let _ = self.gamma.set_ramp(1, points);
        }
        self.tick_weather_wind_audio(frame);
        self.tick_weather_events(frame)?;
        self.apply_landscape_temperature_conversions();
        if let Some(sky) = &mut self.sky {
            sky.advance(&self.environment);
        }
        // Players.Execute completes each player serially in list order:
        // counts/view/control/menu, Tick35 work, then delays.
        self.tick_player_systems()?;
        self.messages.tick(&alive);
        // C4GameScriptHost::Execute (C4ScriptHost.cpp:222-232): while
        // Game.Script.Go, every 10th frame calls Script%d with the counter
        // post-incrementing — the timed intro/movie sections. Runs AFTER
        // ExecObjects and Messages.Execute in the C++ frame
        // (C4Game.cpp:810-822): effects it adds first execute NEXT frame
        // (the intro Divinity markers compare at t=0 on their add frame).
        if self.scenario_script_go && frame.is_multiple_of(10) && self.scenario_script.is_some() {
            let section = format!("Script{}", self.scenario_script_counter);
            self.scenario_script_counter += 1;
            tolerate_script_error(self.call_scenario_script_function(&section, Vec::new()))?;
        }
        self.transfer_zones.retain_existing(&alive);
        self.prune_selection();
        // C4Game::UpdateRules follows Script.Execute and refreshes only on
        // Tick255 (plus frame one) (C4Game.cpp:845,4038-4047).
        if frame == 1 || frame.is_multiple_of(255) {
            self.refresh_structures_snow_in_rule();
            self.refresh_flag_removeable_rule();
        }
        self.refresh_elimination_state();
        self.check_game_over()?;
        // Control.DoSyncCheck() closes the frame (C4Game.cpp:829)
        self.do_sync_check();
        // C4Game::Execute evaluates only after the synchronized frame closes
        // (C4Game.cpp:845-854).
        if self.game_over_triggered && !self.game_evaluated {
            self.evaluate_game()?;
        }
        Ok(())
    }

    /// `C4Object::SetAction`'s ActMap-sound pair (C4Object.cpp:4149-4152,
    /// 4186-4190): leaving a numeric action slot stops that slot's `Sound=`
    /// and entering one starts it as an object-attached loop at volume 100.
    ///
    /// C++ emits both inside `SetAction`; the port reconciles the frame's
    /// resulting selection instead, because the Rust action state is mutated
    /// from a dozen sites (script `SetAction`, the ExecAction `NextAction`
    /// wrap, command/procedure/economy forcing, `ChangeDef`) and a hook per
    /// site would leak a stuck loop the moment one was missed. Sound is
    /// client-local presentation, never synchronized or save-persisted, so
    /// deriving it from observed state costs no determinism. The one residual
    /// is an A->B->A round trip completed inside a single frame, which C++
    /// would stop and restart within that same 1/36s.
    fn reconcile_action_sounds(&mut self) {
        for index in 0..self.objects.len() {
            let object = &self.objects[index];
            // A removed object's looping instances are halted by
            // DetachObjectSounds (C4SoundSystem::ClearPointers), which C++
            // reaches from removal rather than from SetAction. Drop the
            // tracking without emitting a redundant stop of our own.
            if object.destroyed || !object.state.status.is_active() {
                self.objects[index].active_action_sound = None;
                continue;
            }
            let desired = self
                .definitions
                .get(&object.definition_id)
                .map(Definition::action_library)
                .and_then(|library| library.spec_for_state(&object.state.action))
                .and_then(|spec| spec.sound.clone());
            if desired == object.active_action_sound {
                continue;
            }
            let id = object.id;
            if let Some(previous) = self.objects[index].active_action_sound.take() {
                self.pending_audio.push(AudioCommand::StopSound {
                    name: previous,
                    target: Some(id),
                });
            }
            if let Some(sound) = desired {
                self.pending_audio.push(AudioCommand::PlaySound {
                    name: sound.clone(),
                    target: Some(id),
                    volume: 100,
                    looped: true,
                    // `StartSoundEffect` goes straight to `NewInstance`
                    // (C4SoundSystem.cpp:54-58). The IsSoundPlaying gate is
                    // FnSound's alone (C4Script.cpp:2317-2319), so the action
                    // sound must not inherit it.
                    multiple: true,
                    custom_falloff: None,
                });
                self.objects[index].active_action_sound = Some(sound);
            }
        }
    }

    pub(crate) fn drain_tick_presentation(&mut self) -> TickPresentation {
        self.reconcile_action_sounds();
        let scoreboard_presentations = self.take_scoreboard_presentations();
        let menu_requests = self.pending_menu_requests.drain(..).collect();
        for command in &self.pending_audio {
            match command {
                AudioCommand::PlaySound {
                    target: Some(target),
                    ..
                }
                | AudioCommand::PlaySpeech {
                    target: Some(target),
                    ..
                }
                | AudioCommand::SetSoundVolume {
                    target: Some(target),
                    ..
                } => self.audio_registry.note_attached_sound(*target),
                AudioCommand::DetachObjectSounds { target, .. } => {
                    self.audio_registry.note_detached_sounds(*target);
                }
                _ => {}
            }
        }
        let audio = self.pending_audio.drain(..).collect();
        TickPresentation {
            scoreboard_presentations,
            menu_requests,
            audio,
        }
    }

    pub fn object_snapshot(&self, id: ObjectId) -> Option<ObjectSnapshot> {
        let object = &self.objects[self.find_object_index(id)?];
        let library = self
            .definitions
            .get(&object.definition_id)
            .map(|definition| definition.action_library());
        Some(object.snapshot(library))
    }

    /// `C4ObjectList::ObjectCount(C4ID_None, C4D_All)`: count only objects
    /// whose native `Status` is still set.
    pub fn active_object_count(&self) -> usize {
        self.objects
            .iter()
            .filter(|object| !object.destroyed)
            .count()
    }

    /// Lowest-ID object with this definition, matching the ordering exposed
    /// by [`Engine::snapshot`] without constructing that snapshot.
    pub fn first_object_for_definition(&self, definition: &str) -> Option<ObjectId> {
        self.objects
            .iter()
            .filter(|object| object.definition_id == definition)
            .map(|object| object.id)
            .min()
    }

    pub fn object_count_for_definition(&self, definition: &str) -> usize {
        self.objects
            .iter()
            .filter(|object| object.definition_id == definition)
            .count()
    }

    pub fn object_count_for_definition_in_container(
        &self,
        definition: &str,
        container: ObjectId,
    ) -> usize {
        self.objects
            .iter()
            .filter(|object| {
                object.definition_id == definition && object.state.container == Some(container)
            })
            .count()
    }

    /// Exact text used by `C4MouseControl` for an object's help caption:
    /// `C4Object::GetName()`, followed by the definition description when it
    /// exists (src/C4MouseControl.cpp:1134-1142).
    pub fn object_help_caption(&self, id: ObjectId) -> Option<String> {
        let object = self.objects.iter().find(|object| object.id == id)?;
        let definition = self.definitions.get(&object.definition_id)?;
        let name = object
            .state
            .custom_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .or_else(|| {
                self.crew_object_infos
                    .get(&id)
                    .map(|info| info.name.as_str())
            })
            .unwrap_or_else(|| definition.name());
        Some(match definition.description() {
            Some(description) => format!("{name}: {description}"),
            None => name.to_string(),
        })
    }

    pub fn apply_object_update(
        &mut self,
        id: ObjectId,
        update: ObjectUpdate,
    ) -> Result<(), EngineError> {
        let landscape = self.landscape.clone();
        let index = self
            .objects
            .iter()
            .position(|object| object.id == id)
            .ok_or(EngineError::UnknownObject(id))?;
        let previous_status = self.objects[index].state.status;

        let ObjectUpdate {
            custom_name,
            layer,
            compiler_cache,
            visibility,
            blit_mode,
            picture_rect: update_picture_rect,
            position,
            velocity,
            fixed_velocity,
            fixed_velocity_x,
            fixed_velocity_y,
            mobile,
            t_attach,
            rotation,
            rotation_velocity,
            energy,
            host_energy_death_checked,
            breath,
            energy_loss_cause,
            fire,
            fire_flag,
            construction,
            construction_via_docon,
            construction_preserves_fixed_position,
            resolved_docon_position,
            resolved_docon_fixed_position,
            damage,
            magic_energy,
            magic_capacity,
            direction,
            command_direction,
            action,
            status,
            owner,
            base,
            controller,
            own_mass,
            crew_member,
            plr_view_range,
            crew_status_change,
            info_rank,
            info_link,
            crew_disabled,
            solid_mask_override: update_solid_mask,
            solid_mask_instance_sequence,
            change_def,
            change_def_reinsert,
            alive,
            container,
            live_vertices,
            shape_vertices,
            contact_density,
            vertices,
            graphics_overlays,
            base_graphics: update_base_graphics,
            components,
            component_order,
            physicals,
            entrance_status: update_entrance_status,
            color: update_color,
            color_modulation: update_color_modulation,
            shape_override: update_shape_override,
            menu: update_menu,
            ..
        } = update;
        let mut update_menu = update_menu;
        if let Some(Some(menu)) = update_menu.as_mut() {
            if menu.runtime_id == 0 {
                menu.runtime_id = crate::direct_com::next_object_menu_runtime_id();
            }
        }
        if let Some(sequence) = solid_mask_instance_sequence {
            self.solid_mask_staging.next_solid_mask_instance_sequence = self
                .solid_mask_staging
                .next_solid_mask_instance_sequence
                .max(
                    sequence
                        .checked_add(1)
                        .expect("C4SolidMask instance sequence overflow"),
                );
        }
        let fow_range_changed = plr_view_range.is_some();

        let definition_id = self.objects[index].definition_id.clone();
        let previous_action_name = self.objects[index].state.action.name.clone();
        let action_library = self
            .shared_action_library_for(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;

        let mut energy_died = false;
        let mut solid_mask_refresh = rotation.is_some()
            || change_def.is_some()
            || construction.is_some()
            || solid_mask_instance_sequence.is_some();
        // FnChangeDef swaps INLINE (C4Object.cpp:1205-1231): apply it
        // BEFORE the staged fields so an action write in the same update
        // resolves against the NEW def's ActMap.
        let action_library = change_def
            .as_ref()
            .and_then(|new_def| {
                self.apply_change_object_def(index, new_def);
                self.shared_action_library_for(&self.objects[index].definition_id)
            })
            .unwrap_or(action_library);
        let (object_id, previous_owner, previous_crew, new_owner, new_crew, container_change) = {
            let object = &mut self.objects[index];
            let previous_owner = object.state.owner;
            let previous_crew = object.state.crew_member;
            let previous_container = object.state.container;
            let mut container_change = None;

            if let Some(custom_name) = custom_name {
                object.state.custom_name = custom_name;
            }
            if let Some(layer) = layer {
                object.state.layer = layer;
            }
            if let Some(compiler_cache) = compiler_cache {
                object.compiler_cache = compiler_cache;
            }
            if let Some(visibility) = visibility {
                object.state.visibility = visibility;
            }
            if let Some(blit_mode) = blit_mode {
                object.state.blit_mode = blit_mode;
            }
            if let Some(picture_rect) = update_picture_rect {
                object.state.picture_rect = picture_rect;
            }
            if let Some(position) = position {
                object.set_position(position);
            }
            if let Some(physicals) = physicals {
                object.state.info_physical = physicals.info;
                object.state.temporary_physical = physicals.temporary;
                object.state.physical_changes = physicals.changes;
            }
            if let Some(velocity) = velocity {
                object.set_velocity(velocity);
            }
            if let Some(fixed_velocity) = fixed_velocity {
                object.fixed_velocity = fixed_velocity;
                object.state.velocity = object.velocity_pixels();
            }
            // Component dir writes land on the TRUE fixed velocity — the
            // untouched component keeps its sub-pixel value, and the write
            // mobilizes like FnSetXDir/FnSetYDir (`pObj->Mobile = 1`,
            // C4Script.cpp:697-732). Dropping these here lost script
            // SetXDir(0)s whose INT mirror didn't change (|xdir| < 0.5 —
            // the GoldRush fish's 0.4 drift surviving TurnLeft, f100 wall).
            if let Some(x) = fixed_velocity_x {
                object.fixed_velocity.x = x;
                object.state.velocity = object.velocity_pixels();
                object.state.mobile = true;
            }
            if let Some(y) = fixed_velocity_y {
                object.fixed_velocity.y = y;
                object.state.velocity = object.velocity_pixels();
                object.state.mobile = true;
            }
            if let Some(rotation) = rotation {
                let previous_rect = object.current_shape_rect();
                let previous_construction = object.state.construction;
                object.state.rotation = rotation.rem_euclid(360);
                object.fixed_rotation = itofix(object.state.rotation);
                object.refresh_shape_after_state_change(
                    previous_construction,
                    previous_rect,
                    false,
                );
            }
            if let Some(rotation_velocity) = rotation_velocity {
                object.rotation_velocity = rotation_velocity;
            }
            // Apply an explicit native-helper result after all velocity
            // writes that may mobilize the object.
            if let Some(mobile) = mobile {
                object.state.mobile = mobile;
            }
            if let Some(t_attach) = t_attach {
                object.state.t_attach = t_attach;
                object.frame_t_attach = t_attach;
            }
            if let Some(contact_density) = contact_density {
                object.state.contact_density = contact_density;
            }
            if let Some(cause) = energy_loss_cause {
                // Kill-trace mark BEFORE the energy write
                // (C4Object.cpp:1351-1361) so AssignDeath credits it.
                object.last_energy_loss_cause = cause;
            }
            if let Some(energy) = energy {
                // AssignDeath below when a nonzero energy reaches 0
                // (C4Object::DoEnergy, C4Object.cpp:1363) — host DoEnergy
                // folds arrive here.
                energy_died = !host_energy_death_checked
                    && object.state.alive
                    && object.state.energy != 0
                    && energy == 0;
                object.state.energy = energy;
            }
            if let Some(breath) = breath {
                object.state.breath = breath;
            }
            if let Some(damage) = damage {
                object.state.damage = damage.max(0);
            }
            if let Some((fire_caused_by, fire_phase)) = fire {
                // Staged incinerate outcome — draw + Incineration callback
                // already ran mid-call (fxFireStart core,
                // C4Effect.cpp:632-638); OnFire is a SetOCF-owned bit,
                // refreshed with the rest of the fold below.
                object.state.on_fire = true;
                object.state.fire_caused_by = fire_caused_by;
                object.state.fire_phase = fire_phase;
            }
            if let Some(flag) = fire_flag {
                object.state.on_fire = flag;
            }
            if let Some(magic_energy) = magic_energy {
                object.state.magic_energy = magic_energy.max(0);
            }
            if let Some(magic_capacity) = magic_capacity {
                object.state.magic_capacity = magic_capacity.max(0);
            }
            if let Some(direction) = direction {
                object.state.direction = direction;
            }
            if let Some(command_direction) = command_direction {
                object.state.command_direction = command_direction;
            }
            if let Some(action) = action {
                let previous_action = object.state.action.clone();
                let requested_name_change = action.name.is_some();
                let result = object
                    .state
                    .action
                    .apply_update_with_library(&action, &action_library);
                // C4Object::SetAction resyncs the fixed coords to the
                // integer position once past its early returns
                // (C4Object.cpp:4144).
                if action.name.is_some() && matches!(result, ActionUpdateResult::Applied) {
                    object.fixed_position =
                        FixedVec2::from_ints(object.state.position.x, object.state.position.y);
                }
                if matches!(result, ActionUpdateResult::Applied)
                    && (requested_name_change
                        || object.state.action.name != previous_action.name
                        || object.state.action.act_map_index != previous_action.act_map_index)
                    && !action.callbacks_dispatched
                {
                    object.record_action_event(previous_action, ActionTransitionKind::Forced);
                }
            } else {
                object.state.action.reconcile_with_library(&action_library);
            }
            if let Some(owner) = owner {
                object.state.owner = owner;
                // SetOwner "automatically updates controller"
                // (C4Object.cpp:5499-5500); an explicit SetController in
                // the same batch still wins below.
                object.state.controller = controller.unwrap_or(owner);
            } else if let Some(controller) = controller {
                object.state.controller = controller;
            }
            if let Some(base) = base {
                object.state.base = base;
            }
            if let Some(own_mass) = own_mass {
                object.state.own_mass = own_mass;
                object.compiled_mass = None;
            }
            if let Some(crew_member) = crew_member {
                object.state.crew_member = crew_member;
            }
            if let Some(plr_view_range) = plr_view_range {
                object.state.plr_view_range = plr_view_range;
            }
            if let Some(crew_disabled) = crew_disabled {
                object.state.crew_disabled = crew_disabled;
            }
            if let Some(rect) = update_solid_mask {
                object.state.solid_mask_override = Some(rect);
                object.solid_mask_instance_sequence = None;
                solid_mask_refresh = true;
            }
            if let Some(alive) = alive {
                object.state.alive = alive;
            }
            if let Some(entrance_status) = update_entrance_status {
                object.state.entrance_status = entrance_status;
            }
            let menu_written = update_menu.is_some();
            if let Some(menu) = update_menu {
                object.state.menu = menu;
            }
            if let Some(color) = update_color {
                object.state.color = color;
            }
            if let Some(color_modulation) = update_color_modulation {
                object.state.color_modulation = color_modulation;
            }
            if let Some(status) = status {
                object.apply_status(status);
            }
            if let Some(container_update) = container {
                if object.state.container != container_update {
                    object.state.container = container_update;
                    container_change = Some((previous_container, object.state.container));
                    // C4Object::Enter/Exit force-close the moving object's
                    // menu (CloseMenu(true), C4Object.cpp:1555/:1594) —
                    // unless this update carries its own (already correctly
                    // ordered) menu write.
                    if !menu_written {
                        object.state.menu = None;
                    }
                }
            }
            if let Some(vertices) = vertices {
                object.set_owned_shape_vertices(vertices);
            }
            if let Some(construction) = construction {
                let fixed_position = object.fixed_position;
                if construction_via_docon {
                    object.set_construction_from_docon(construction);
                } else {
                    object.set_construction(construction);
                }
                if construction_via_docon && construction_preserves_fixed_position {
                    object.fixed_position = fixed_position;
                }
            }
            if let Some(position) = resolved_docon_position {
                object.state.position = position;
            }
            if let Some(position) = resolved_docon_fixed_position {
                object.fixed_position = position;
            }
            if let Some(vertices) = live_vertices.as_ref() {
                object.set_live_shape_vertices(vertices.clone());
            }
            if let Some(vertices) = shape_vertices.as_ref() {
                object.set_shape_vertex_buffer(vertices.clone());
            }
            if let Some(overlays) = graphics_overlays {
                object.state.graphics_overlays = overlays;
            }
            if let Some(base_graphics) = update_base_graphics {
                if object.state.base_graphics != base_graphics {
                    object.state.base_graphics = base_graphics;
                    object.solid_mask_instance_sequence = None;
                    solid_mask_refresh = true;
                }
            }
            if let Some(sequence) = solid_mask_instance_sequence {
                object.solid_mask_instance_sequence = Some(sequence);
            }
            if let Some(components) = components {
                object.state.component_order = normalized_component_order(
                    &components,
                    component_order
                        .clone()
                        .unwrap_or_else(|| object.state.component_order.clone()),
                    &[],
                );
                object.state.components = components;
            } else if let Some(component_order) = component_order {
                object.state.component_order =
                    normalized_component_order(&object.state.components, component_order, &[]);
            }

            object.clamp_velocity(&self.physics);

            if !object.state.vertices.is_empty() {
                if let Some(landscape) = landscape.as_ref() {
                    let resolution =
                        landscape.resolve_collision(object.state.position, object.state.velocity);
                    if resolution.collided {
                        object.apply_collision_resolution(&resolution);
                        if let Some(material_id) = resolution.material {
                            if let Some(material) = self.materials.get_by_id(material_id) {
                                object.apply_material_interaction(material);
                            }
                        }
                    }
                }
            }

            (
                object.id,
                previous_owner,
                previous_crew,
                object.state.owner,
                object.state.crew_member,
                container_change,
            )
        };

        let current_status = self.objects[index].state.status;
        self.update_inactive_list_for_status_change(object_id, previous_status, current_status);
        if fow_range_changed {
            self.actualize_object_fow_view_range(object_id);
        } else if previous_owner != new_owner {
            self.actualize_object_fow_after_owner_change(object_id, new_owner);
        } else if current_status == ObjectStatus::Deleted {
            self.remove_object_from_fow_view_lists(object_id);
        }

        if solid_mask_refresh {
            // SetSolidMask and SetGraphics reflow the bake immediately
            // (C4Object.cpp:3792 and UpdateGraphics at :381-402).
            self.update_solid_mask(index);
        }
        self.apply_info_update(object_id, info_rank, info_link);
        self.update_sector_for_index(index);
        if energy_died {
            self.assign_death(index, false)?;
        }
        self.update_selection_for_state_change(
            object_id,
            previous_owner,
            if crew_status_change {
                new_crew
            } else {
                previous_crew
            },
            new_owner,
            new_crew,
        );
        if let Some((previous_container, new_container)) = container_change {
            self.apply_container_change(object_id, previous_container, new_container, false)?;
        }
        if change_def_reinsert {
            self.reinsert_change_def_contents_link(object_id)?;
        }
        // Host callbacks collapse several ordered native calls into one
        // ObjectUpdate. SetR/DoCon/Enter/StatusActivate stage an explicit
        // clear, while a later SetShape replaces it with the final rect.
        // Apply that ordering token only after every native shape refresh so
        // `Enter(); SetShape(...)` and its inverse match C++.
        if let Some(shape_override) = update_shape_override {
            let object = &mut self.objects[index];
            object.state.shape_override = shape_override;
            match shape_override {
                Some(rect) => object.shape_rect = Some(rect),
                None => {
                    object.refresh_shape_geometry();
                    // The ordering token may represent an earlier SetR,
                    // Enter, DoCon or activation. Script vertex edits after
                    // that native UpdateShape are the final C++ state and
                    // must survive this deferred refresh.
                    if let Some(vertices) = live_vertices.as_ref() {
                        object.set_live_shape_vertices(vertices.clone());
                    }
                    if let Some(vertices) = shape_vertices.as_ref() {
                        object.set_shape_vertex_buffer(vertices.clone());
                    }
                }
            }
        }
        // Host-driven changes are SetOCF events (SetAlive C4Object.h:361,
        // DoCon C4Object.cpp:1417, status C4Object.cpp:4139).
        self.refresh_object_ocf(index);
        self.trigger_action_callbacks(index, Some(previous_action_name))?;
        self.update_sector_for_index(index);
        if self.objects[index].destroyed
            || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
        {
            self.detach_destroyed_objects()?;
            self.update_sector_for_index(index);
        }
        self.refresh_elimination_state();
        self.check_game_over()?;

        Ok(())
    }

    fn apply_info_update(
        &mut self,
        object_id: ObjectId,
        rank_update: Option<Option<i32>>,
        link_update: Option<Option<CrewInfoLink>>,
    ) {
        if let Some(rank) = rank_update {
            match rank {
                Some(rank) => {
                    Rc::make_mut(&mut self.crew_ranks).insert(object_id.as_u64(), rank);
                    if let Some(info) =
                        Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id)
                    {
                        info.rank = rank;
                    }
                }
                None => {
                    Rc::make_mut(&mut self.crew_ranks).remove(&object_id.as_u64());
                    Rc::make_mut(&mut self.crew_object_infos).remove(&object_id);
                }
            }
        }
        if let Some(link) = link_update {
            match link {
                Some(link) => {
                    Rc::make_mut(&mut self.crew_info_links).insert(object_id, link);
                }
                None => {
                    Rc::make_mut(&mut self.crew_info_links).remove(&object_id);
                }
            }
        }
    }

    pub(crate) fn trigger_action_callbacks(
        &mut self,
        index: usize,
        previous_action: Option<String>,
    ) -> Result<(), EngineError> {
        self.trigger_action_callbacks_impl(index, previous_action, false)
    }

    fn trigger_action_callbacks_impl(
        &mut self,
        index: usize,
        previous_action: Option<String>,
        allow_deleted_initial_start: bool,
    ) -> Result<(), EngineError> {
        if self.objects[index].destroyed && !allow_deleted_initial_start {
            return Ok(());
        }

        let mut needs_start = previous_action.is_none();

        // C++ has NO deferred callback queue (SetAction runs its calls
        // inline) — an unbounded drain here is always a rust-side artifact
        // (the coach Driving loop drained forever against stale state).
        let mut drained = 0u32;
        while let Some(event) = self.objects[index].pending_action_events.pop_front() {
            drained += 1;
            if drained > 32 {
                tracing::warn!(
                    object = self.objects[index].id.as_u64(),
                    "action-callback drain backstop hit; dropping queued transitions"
                );
                self.objects[index].pending_action_events.clear();
                break;
            }
            let current_action = self.objects[index].state.action.clone();
            let callback_definition = self.objects[index].definition_id.clone();
            self.invoke_action_callback(
                index,
                ActionCallbackKind::Start,
                &current_action.name,
                current_action.act_map_index,
                None,
                None,
                None,
                None,
            )?;
            if self.objects[index].destroyed
                || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
                || self.objects[index].definition_id != callback_definition
            {
                self.objects[index].pending_action_events.clear();
                return Ok(());
            }

            let callback_kind = match event.kind {
                ActionTransitionKind::Natural => ActionCallbackKind::End,
                ActionTransitionKind::Forced => ActionCallbackKind::Abort,
            };
            self.invoke_action_callback(
                index,
                callback_kind,
                &event.previous_action.name,
                event.previous_action.act_map_index,
                None,
                None,
                matches!(event.kind, ActionTransitionKind::Forced)
                    .then_some(event.previous_action.phase),
                None,
            )?;
            if self.objects[index].destroyed
                || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
                || self.objects[index].definition_id != callback_definition
            {
                self.objects[index].pending_action_events.clear();
                return Ok(());
            }

            needs_start = false;
        }

        if needs_start {
            let current_action = self.objects[index].state.action.clone();
            self.invoke_action_callback(
                index,
                ActionCallbackKind::Start,
                &current_action.name,
                current_action.act_map_index,
                None,
                None,
                None,
                None,
            )?;
        }

        Ok(())
    }

    pub(crate) fn invoke_action_callback(
        &mut self,
        index: usize,
        kind: ActionCallbackKind,
        action_name: &str,
        action_index: Option<u32>,
        callback_override: Option<ScriptCallbackTarget>,
        state_override: Option<ObjectState>,
        abort_phase: Option<i32>,
        callback_definition_override: Option<&str>,
    ) -> Result<(), EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        let callback_definition_id = callback_definition_override.unwrap_or(&definition_id);
        let object_definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = self
            .shared_action_library_for(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let callback_definition = self
            .definitions
            .get(callback_definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(callback_definition_id.to_string()))?;

        let callback = match callback_override {
            Some(callback) => Some(callback),
            None => match kind {
                ActionCallbackKind::Start => callback_definition
                    .action_library()
                    .start_callback_for_entry(action_name, action_index),
                ActionCallbackKind::End => callback_definition
                    .action_library()
                    .end_callback_for_entry(action_name, action_index),
                ActionCallbackKind::Phase => callback_definition
                    .action_library()
                    .phase_callback_for_entry(action_name, action_index),
                ActionCallbackKind::Abort => callback_definition
                    .action_library()
                    .abort_callback_for_entry(action_name, action_index),
            },
        };

        let Some(callback) = callback else {
            return Ok(());
        };
        let function = callback.function_name();

        tracing::debug!(
            definition = %callback_definition_id,
            object_definition = %definition_id,
            function,
            ?kind,
            action = action_name,
            object = self.objects[index].id.as_u64(),
            "action callback"
        );
        let object_id = self.objects[index].id;
        let (state_snapshot, world) = match state_override {
            Some(state) => (
                Rc::new(state),
                // An explicit action-state override can intentionally differ
                // from the object's current live state. Preserve the existing
                // host-world view in that case.
                self.host_world_context_for_object(index),
            ),
            None => {
                let state = Rc::new(self.objects[index].script_state_snapshot());
                let world =
                    self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state));
                (state, world)
            }
        };
        let definitions_ref = &self.definitions;
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let callback = callback_definition.call_action_callback(
            object_definition,
            &callback,
            kind,
            state_snapshot.as_ref(),
            object_id,
            action_name,
            abort_phase,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let callback = callback.map_err(|error| {
            self.apply_script_error_recovery(
                error,
                index,
                &action_library,
                object_id,
                &definition_id,
                true,
            )
        });
        // StartCall/EndCall/PhaseCall/AbortCall are engine-initiated game
        // calls: a script error logs and the action proceeds (C++ fail-safe
        // exec, C4AulExec.cpp:1318-1342).
        let Some((outcome, audio_state, new_rng)) = tolerate_script_error(callback)? else {
            return Ok(());
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;

        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            object_id,
            &definition_id,
        )
    }

    /// Apply the pre-error mutations a failed outer call carried before
    /// handing the error on (C4AulExec.cpp:1318-1342: the error aborts the
    /// call, but C++ already mutated the live objects — nothing rolls
    /// back). Errors without a payload pass through untouched.
    pub(crate) fn apply_script_error_recovery(
        &mut self,
        error: EngineError,
        index: usize,
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
        clamp_velocity: bool,
    ) -> EngineError {
        let (definition, function, source, recovery) = match error {
            EngineError::Script {
                definition,
                function,
                source,
                recovery: Some(recovery),
            } => (definition, function, source, recovery),
            other => return other,
        };
        let ScriptCallRecovery {
            outcome,
            audio,
            rng,
        } = *recovery;
        self.rng = rng;
        self.audio_registry = audio;
        if let Err(apply_error) = self.apply_callback_outcome(
            index,
            outcome,
            action_library,
            object_id,
            definition_id,
            clamp_velocity,
        ) {
            return apply_error;
        }
        EngineError::Script {
            definition,
            function,
            source,
            recovery: None,
        }
    }

    pub(crate) fn apply_action_callback_outcome(
        &mut self,
        index: usize,
        outcome: compat::EffectContextOutcome,
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
    ) -> Result<(), EngineError> {
        self.apply_callback_outcome(
            index,
            outcome,
            action_library,
            object_id,
            definition_id,
            true,
        )
    }

    pub(crate) fn apply_callback_outcome(
        &mut self,
        index: usize,
        outcome: compat::EffectContextOutcome,
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
        clamp_velocity: bool,
    ) -> Result<(), EngineError> {
        let solid_mask_operations = outcome.solid_mask_operations.clone();
        let host_raster_preview = outcome.host_raster_preview.clone();
        // The outcome's outer-object, spawn, and foreign-object channels do
        // not preserve the interleaving of synchronous UpdateSolidMask
        // calls. Apply every non-mask state change first while suppressing
        // their channel-local mask folds, then replay the captured C++ call
        // order against the now-materialized objects.
        let was_deferred = self.solid_mask_staging.defer_solid_mask_updates;
        let mut outermost =
            self.stage_host_solid_mask_operations(solid_mask_operations, host_raster_preview);
        let result = self.apply_callback_outcome_inner(
            index,
            outcome,
            action_library,
            object_id,
            definition_id,
            clamp_velocity,
        );
        outermost |= !was_deferred && self.solid_mask_staging.defer_solid_mask_updates;
        self.finish_host_solid_mask_operations(outermost, result)
    }

    fn apply_callback_outcome_inner(
        &mut self,
        index: usize,
        outcome: compat::EffectContextOutcome,
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
        clamp_velocity: bool,
    ) -> Result<(), EngineError> {
        let compat::EffectContextOutcome {
            object: object_effects,
            global: global_effects,
            object_update,
            mut object_commands,
            command_operations,
            command_events,
            destroy_object,
            other_objects,
            environment,
            physics,
            spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: _,
            host_raster_preview: _,
            particles,
            transfer_zones,
            messages,
            player_commands,
            object_order_commands,
            next_mission_commands,
            menu_requests,
            audio: outcome_audio,
            trigger_game_over,
            script_go,
            script_counter,
            next_object_id,
            context_locals,
        } = outcome;
        let refresh_ocf = object_update
            .as_ref()
            .is_some_and(ObjectUpdate::refreshes_ocf_like_cpp);
        let ocf_override = object_update
            .as_ref()
            .and_then(|update| update.ocf_override);
        let info_rank_update = object_update.as_ref().and_then(|update| update.info_rank);
        let info_link_update = object_update.as_ref().and_then(|update| update.info_link);
        let crew_status_change = object_update
            .as_ref()
            .is_some_and(|update| update.crew_status_change);
        let fow_range_changed = object_update
            .as_ref()
            .is_some_and(|update| update.plr_view_range.is_some());
        let final_shape_override = object_update
            .as_ref()
            .and_then(|update| update.shape_override);
        let final_live_vertices = object_update
            .as_ref()
            .and_then(|update| update.live_vertices.clone());
        let final_shape_vertices = object_update
            .as_ref()
            .and_then(|update| update.shape_vertices.clone());

        if !host_landscape_ops.is_empty() {
            self.apply_landscape_operations(host_landscape_ops);
        }

        // FnExecuteCommand executes one command synchronously
        // (C4Script.cpp:922-929). Its object/player/spawn events therefore
        // become visible before this host callback returns, not on the next
        // simulation tick.
        for event in &command_events {
            self.apply_command_event(event.clone())?;
        }
        // Other callback consumers still use the historical zero-delay
        // event queue. This path applied the same events synchronously, so
        // remove only those pure carrier commands before they reach the
        // object's next-tick queue.
        object_commands.retain(|command| {
            !(command.delay == 0
                && command.update.is_empty()
                && command.effects.is_empty()
                && !command.events.is_empty()
                && command
                    .events
                    .iter()
                    .all(|event| command_events.contains(event))
                && !command.destroy
                && command.spawns.is_empty()
                && command.landscape.is_empty()
                && command.particles.is_empty())
        });

        if !player_commands.is_empty() {
            self.apply_player_commands(player_commands)?;
        }
        self.pending_object_order_commands
            .extend(object_order_commands);
        self.apply_next_mission_commands(next_mission_commands);

        if let Some(update) = environment {
            self.apply_environment_delta(&update);
        }
        if let Some(delta) = physics {
            self.apply_physics_delta(delta);
        }

        self.sync_next_object_id(next_object_id);
        if !spawns.is_empty() {
            self.process_spawn_queue(spawns)?;
        }
        self.apply_particle_commands(particles);
        if !transfer_zones.is_empty() {
            self.apply_transfer_zone_commands(transfer_zones)?;
        }

        if !outcome_audio.events.is_empty() {
            self.pending_audio.extend(outcome_audio.events);
        }
        if !messages.is_empty() {
            for command in messages {
                self.messages.apply_command(command);
            }
        }
        for request in menu_requests {
            let MenuRequest {
                crew_id,
                owner,
                kind,
            } = request;
            let Some(crew_index) = self.find_object_index(crew_id) else {
                continue;
            };
            match kind {
                MenuRequestKind::Activate => {
                    self.apply_container_menu_request(MenuRequest {
                        crew_id,
                        owner,
                        kind: MenuRequestKind::Activate,
                    })?;
                }
                MenuRequestKind::ActivateTarget { container } => {
                    self.apply_container_menu_request(MenuRequest {
                        crew_id,
                        owner,
                        kind: MenuRequestKind::ActivateTarget { container },
                    })?;
                }
                MenuRequestKind::Construction => {
                    self.open_construction_menu(crew_index)?;
                }
                MenuRequestKind::Buy { base } => {
                    if let Some(base_index) = self.find_object_index(base) {
                        self.open_base_buy_menu(crew_index, base_index)?;
                    }
                }
                MenuRequestKind::Sell { base } => {
                    if let Some(base_index) = self.find_object_index(base) {
                        self.open_base_sell_menu(crew_index, base_index)?;
                    }
                }
                MenuRequestKind::Get { container } => {
                    self.apply_container_menu_request(MenuRequest {
                        crew_id,
                        owner,
                        kind: MenuRequestKind::Get { container },
                    })?;
                }
                MenuRequestKind::Contents { container } => {
                    self.apply_container_menu_request(MenuRequest {
                        crew_id,
                        owner,
                        kind: MenuRequestKind::Contents { container },
                    })?;
                }
                MenuRequestKind::Info { target } => {
                    if let Some(target_index) = self.find_object_index(target) {
                        self.open_object_info_menu(crew_index, target_index)?;
                    }
                }
                MenuRequestKind::Context { target, position } => {
                    if let Some(target_index) = self.find_object_index(target) {
                        self.open_context_menu(crew_index, target_index, false, position)?;
                    }
                }
                kind => self.pending_menu_requests.push(MenuRequest {
                    crew_id,
                    owner,
                    kind,
                }),
            }
        }

        if let Some(go) = script_go {
            self.scenario_script_go = go;
        }
        if let Some(counter) = script_counter {
            self.scenario_script_counter = counter;
        }
        if trigger_game_over {
            self.request_game_over()?;
        }

        let mut effect_events = Vec::new();
        let mut energy_died = false;
        let mut container_changes = Vec::new();

        let mut command_operations = command_operations;

        let (previous_owner, previous_crew_member, previous_base_graphics, previous_status) = {
            let object = &self.objects[index];
            (
                object.state.owner,
                object.state.crew_member,
                object.state.base_graphics.clone(),
                object.state.status,
            )
        };
        let solid_mask_changed = destroy_object
            || object_update.as_ref().is_some_and(|update| {
                update.change_def.is_some()
                    || update.solid_mask_override.is_some()
                    || update.position.is_some()
                    || update.rotation.is_some()
                    || update.construction.is_some()
            });
        let mut change_def_reinsert = object_update
            .as_ref()
            .is_some_and(|update| update.change_def_reinsert);

        // FnChangeDef swaps the definition INLINE at the call site
        // (C4Object::ChangeDef, C4Object.cpp:1205-1231, incl. the
        // SetAction(ActIdle) pre-reset :1214) — the staged writes that
        // follow it (the horse Death's SetAction("Dead")) must resolve
        // against the NEW def's ActMap, so the swap applies BEFORE the
        // delta and the action library is re-resolved (the f147 wall:
        // cpp "Dead" vs rust "Idle" fallback against the old library).
        let changed_library = object_update
            .as_ref()
            .and_then(|update| update.change_def.clone())
            .and_then(|new_def| {
                self.apply_change_object_def(index, &new_def);
                self.shared_action_library_for(&self.objects[index].definition_id)
            });
        let action_library = changed_library.as_deref().unwrap_or(action_library);

        {
            let object = &mut self.objects[index];

            // Effect-callback VM finals (C4Effect.cpp:129: the callback ran
            // in this object's own context) apply first; host-command
            // updates below may override.
            if let Some(locals) = context_locals {
                object.state.local_vars = locals;
            }

            if let Some(update) = object_update {
                let callbacks_dispatched = update
                    .action
                    .as_ref()
                    .map(|action| action.callbacks_dispatched)
                    .unwrap_or(false);
                let delta: ObjectDelta = update.into();
                let outcome = object.apply_delta(&delta, action_library);
                energy_died |= outcome.energy_died;
                if let Some(change) = outcome.action_change {
                    if !callbacks_dispatched {
                        object.record_action_event(change.previous, ActionTransitionKind::Forced);
                    }
                }
                if let Some(change) = outcome.container_change {
                    container_changes.push(change);
                }
            }

            if !command_operations.is_empty() {
                let operations: Vec<_> = std::mem::take(&mut command_operations);
                object.apply_command_operations(operations);
            }

            if !object_commands.is_empty() {
                object.enqueue_commands(object_commands);
            }

            if !object_effects.is_empty() {
                let mut applied = object.apply_effect_commands(&object_effects);
                effect_events.append(&mut applied);
            }

            // Exact AssignRemoval callbacks have already stopped effects
            // synchronously and emit no-callback removals. Fold those before
            // the status write so mark_destroyed cannot stop them twice.
            if destroy_object {
                object.retired_info_physical = object.state.info_physical;
                object.state.info_physical = None;
                effect_events.extend(object.mark_destroyed());
            }

            if clamp_velocity
                && !matches!(
                    action_library.procedure_for_entry(
                        &object.state.action.name,
                        object.state.action.act_map_index,
                    ),
                    ActionProcedure::Flight
                )
            {
                object.clamp_velocity(&self.physics);
            }
        }
        self.apply_info_update(object_id, info_rank_update, info_link_update);
        self.update_sector_for_index(index);

        if energy_died {
            // C4Object::DoEnergy kills synchronously when a nonzero
            // energy reaches 0
            // (oracle-src-pinned src/C4Object.cpp:1372-1393).
            self.assign_death(index, false)?;
        }

        let (new_owner, new_crew_member) = {
            let object = &self.objects[index];
            (object.state.owner, object.state.crew_member)
        };

        if previous_owner != new_owner || previous_crew_member != new_crew_member {
            self.update_selection_for_state_change(
                object_id,
                previous_owner,
                if crew_status_change {
                    new_crew_member
                } else {
                    previous_crew_member
                },
                new_owner,
                new_crew_member,
            );
        }
        let current_status = self.objects[index].state.status;
        if fow_range_changed {
            self.actualize_object_fow_view_range(object_id);
        } else if previous_owner != new_owner {
            self.actualize_object_fow_after_owner_change(object_id, new_owner);
        } else if current_status == ObjectStatus::Deleted {
            self.remove_object_from_fow_view_lists(object_id);
        }

        if !global_effects.is_empty() {
            self.apply_global_effect_commands(&global_effects);
        }

        let mut effect_solid_mask_changed = false;
        if !effect_events.is_empty() {
            let previous_container = self.objects[index].state.container;
            let definition = self
                .definitions
                .get(definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.to_string()))?;
            let definitions_ref = &self.definitions;
            let global_view = self.global_effects.clone();
            let rng_state = self.rng.clone();
            let world = self.host_world_context_for_object(index);
            let object = &mut self.objects[index];
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
                nested_effect_solid_mask_operations,
                nested_effect_host_raster_preview,
                nested_effect_solid_mask_changed,
                _nested_effect_action_callbacks_dispatched,
                nested_effect_change_def_reinsert,
                effect_next_object_id,
                triggered_game_over,
                effect_script_go,
                effect_script_counter,
                audio_state,
                new_rng,
            ) = Self::run_effect_events_for_object(
                definition,
                definitions_ref,
                self.game_over_triggered,
                rng_state,
                object_id,
                object,
                effect_events,
                global_view,
                &mut self.environment,
                self.physics,
                self.frame,
                world.clone(),
                self.audio_registry.clone(),
            )?;
            self.stage_host_solid_mask_operations(
                nested_effect_solid_mask_operations,
                nested_effect_host_raster_preview,
            );
            self.rng = new_rng;
            self.audio_registry = audio_state;
            effect_solid_mask_changed |= nested_effect_solid_mask_changed;
            if let Some(marker) = nested_effect_change_def_reinsert {
                change_def_reinsert = marker;
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
            let new_container = self.objects[index].state.container;
            if previous_container != new_container {
                container_changes.push((previous_container, new_container));
            }
        }
        self.update_sector_for_index(index);

        for (previous, new) in container_changes {
            self.apply_container_change(object_id, previous, new, false)?;
        }
        if change_def_reinsert {
            self.reinsert_change_def_contents_link(object_id)?;
        }

        if solid_mask_changed
            || effect_solid_mask_changed
            || self.objects[index].state.base_graphics != previous_base_graphics
        {
            // SetSolidMask and SetGraphics both remove, recreate and re-put
            // the active solid mask immediately (C4Object.cpp:3809-3818,
            // :381-402).
            self.update_solid_mask(index);
        }

        self.apply_nested_object_outcomes(other_objects)?;

        // Only the matching C++ host operations refresh OCF. In particular,
        // a no-op Contact callback must preserve Execute's pre-movement
        // HitSpeed cache through the later Splash gate
        // (C4Movement.cpp:166-182,449-456).
        if refresh_ocf || energy_died {
            self.refresh_object_ocf(index);
        }
        if let Some(ocf) = ocf_override {
            self.objects[index].state.ocf = ocf;
        }

        self.update_inactive_list_for_status_change(
            object_id,
            previous_status,
            self.objects[index].state.status,
        );
        if let Some(shape_override) = final_shape_override {
            let object = &mut self.objects[index];
            object.state.shape_override = shape_override;
            match shape_override {
                Some(rect) => object.shape_rect = Some(rect),
                None => {
                    object.refresh_shape_geometry();
                    if let Some(vertices) = final_live_vertices {
                        object.set_live_shape_vertices(vertices);
                    }
                    if let Some(vertices) = final_shape_vertices {
                        object.set_shape_vertex_buffer(vertices);
                    }
                }
            }
            self.update_sector_for_index(index);
        }

        Ok(())
    }

    /// Applies mutations nested script calls (Find_Func/GameCall reentrancy)
    /// made to objects other than the outer call's `this`, in first-call
    /// order. C++ mutates live state mid-call; the copy-in/copy-out model
    /// commits them here, after the outer object's own update.
    pub(crate) fn apply_nested_object_outcomes(
        &mut self,
        outcomes: Vec<compat::NestedObjectOutcome>,
    ) -> Result<(), EngineError> {
        let _ = self.apply_nested_object_outcomes_retaining_missing(outcomes)?;
        Ok(())
    }

    /// Applies every outcome whose target is live and returns only outcomes
    /// for not-yet-materialized targets. Spawn queues use the returned tail
    /// to bridge the gap between C++'s synchronous NewObject insertion and
    /// Rust's deferred SpawnConfig materialization.
    pub(crate) fn apply_nested_object_outcomes_retaining_missing(
        &mut self,
        outcomes: Vec<compat::NestedObjectOutcome>,
    ) -> Result<Vec<compat::NestedObjectOutcome>, EngineError> {
        let was_deferred = self.solid_mask_staging.defer_solid_mask_updates;
        let result = self.apply_nested_object_outcomes_retaining_missing_inner(outcomes);
        let outermost = !was_deferred && self.solid_mask_staging.defer_solid_mask_updates;
        self.finish_host_solid_mask_operations(outermost, result)
    }

    fn apply_nested_object_outcomes_retaining_missing_inner(
        &mut self,
        outcomes: Vec<compat::NestedObjectOutcome>,
    ) -> Result<Vec<compat::NestedObjectOutcome>, EngineError> {
        let mut retained = Vec::new();
        for mut outcome in outcomes {
            if !outcome.contents_orders.is_empty() {
                debug_assert!(
                    outcome.update.is_none()
                        && outcome.effects.is_empty()
                        && outcome.commands.is_empty()
                        && outcome.command_operations.is_empty()
                        && !outcome.destroy
                        && outcome.assign_death.is_none(),
                    "contents-order carriers must not contain object mutations"
                );
                let pending =
                    self.apply_host_contents_orders(std::mem::take(&mut outcome.contents_orders));
                if !pending.is_empty() {
                    outcome.object_id = pending[0].container;
                    outcome.contents_orders = pending;
                    retained.push(outcome);
                }
                continue;
            }
            let requested_death = outcome.assign_death;
            let Some(index) = self.find_object_index(outcome.object_id) else {
                retained.push(outcome);
                continue;
            };
            let object_id = outcome.object_id;
            let definition_id = self.objects[index].definition_id.clone();
            let action_library = self
                .shared_action_library_for(&definition_id)
                .unwrap_or_default();

            let mut effect_events = Vec::new();
            let mut container_changes = Vec::new();
            let (previous_owner, previous_crew_member, previous_base_graphics, previous_status) = {
                let object = &self.objects[index];
                (
                    object.state.owner,
                    object.state.crew_member,
                    object.state.base_graphics.clone(),
                    object.state.status,
                )
            };
            let solid_mask_changed = outcome.update.as_ref().is_some_and(|update| {
                update.change_def.is_some()
                    || update.solid_mask_override.is_some()
                    || update.position.is_some()
                    || update.rotation.is_some()
                    || update.construction.is_some()
            });
            let mut change_def_reinsert = outcome
                .update
                .as_ref()
                .is_some_and(|update| update.change_def_reinsert);
            let refresh_ocf = outcome
                .update
                .as_ref()
                .is_some_and(ObjectUpdate::refreshes_ocf_like_cpp);
            let ocf_override = outcome
                .update
                .as_ref()
                .and_then(|update| update.ocf_override);
            let info_rank_update = outcome.update.as_ref().and_then(|update| update.info_rank);
            let info_link_update = outcome.update.as_ref().and_then(|update| update.info_link);
            let crew_status_change = outcome
                .update
                .as_ref()
                .is_some_and(|update| update.crew_status_change);
            let fow_range_changed = outcome
                .update
                .as_ref()
                .is_some_and(|update| update.plr_view_range.is_some());
            let final_shape_override = outcome
                .update
                .as_ref()
                .and_then(|update| update.shape_override);
            let final_live_vertices = outcome
                .update
                .as_ref()
                .and_then(|update| update.live_vertices.clone());
            let final_shape_vertices = outcome
                .update
                .as_ref()
                .and_then(|update| update.shape_vertices.clone());
            let mut energy_died = false;
            let mut delayed_docon_state = None;
            // FnChangeDef swaps INLINE (C4Object.cpp:1205-1231): apply the
            // def change BEFORE the staged delta so a following
            // SetAction resolves against the NEW ActMap.
            let action_library = outcome
                .update
                .as_ref()
                .and_then(|update| update.change_def.clone())
                .and_then(|new_def| {
                    self.apply_change_object_def(index, &new_def);
                    self.shared_action_library_for(&self.objects[index].definition_id)
                })
                .unwrap_or(action_library);
            {
                let object = &mut self.objects[index];
                if let Some(mut update) = outcome.update {
                    // A nested Enter/Exit followed by DoCon must copy the
                    // container motion first and only then bottom-adjust the
                    // new construction shape. The copy-in/out scope carries
                    // both writes in one update, so delay just this ordered
                    // DoCon fold until after apply_container_change below.
                    if update.construction_via_docon && update.container.is_some() {
                        delayed_docon_state = update.construction.take().map(|construction| {
                            (
                                construction,
                                update.resolved_docon_position.take(),
                                update.resolved_docon_fixed_position.take(),
                            )
                        });
                        update.construction_via_docon = false;
                        update.construction_preserves_fixed_position = false;
                    }
                    let callbacks_dispatched = update
                        .action
                        .as_ref()
                        .map(|action| action.callbacks_dispatched)
                        .unwrap_or(false);
                    let delta: ObjectDelta = update.into();
                    let apply_outcome = object.apply_delta(&delta, &action_library);
                    energy_died = apply_outcome.energy_died;
                    if let Some(change) = apply_outcome.action_change {
                        if !callbacks_dispatched {
                            object
                                .record_action_event(change.previous, ActionTransitionKind::Forced);
                        }
                    }
                    if let Some(change) = apply_outcome.container_change {
                        container_changes.push(change);
                    }
                }
                if !outcome.command_operations.is_empty() {
                    object.apply_command_operations(outcome.command_operations);
                }
                if !outcome.commands.is_empty() {
                    object.enqueue_commands(outcome.commands);
                }
                if !outcome.effects.is_empty() {
                    let mut applied = object.apply_effect_commands(&outcome.effects);
                    effect_events.append(&mut applied);
                }
                // Effect commands happened synchronously before the
                // AssignRemoval status write. In particular, its exact host
                // path already ran Fx*Stop and carries no-callback removals;
                // fold those before mark_destroyed so it cannot emit a
                // second deferred Stop for the same effects.
                if outcome.destroy {
                    object.retired_info_physical = object.state.info_physical;
                    object.state.info_physical = None;
                    effect_events.extend(object.mark_destroyed());
                }
            }
            self.apply_info_update(object_id, info_rank_update, info_link_update);
            self.update_sector_for_index(index);
            if energy_died {
                // C4Object::DoEnergy kills synchronously when a nonzero
                // energy reaches 0 (C4Object.cpp:1363) — foreign writes
                // (Punch, DoEnergy on a named target) included.
                self.assign_death(index, false)?;
            }

            let (new_owner, new_crew_member) = {
                let object = &self.objects[index];
                (object.state.owner, object.state.crew_member)
            };
            if previous_owner != new_owner || previous_crew_member != new_crew_member {
                self.update_selection_for_state_change(
                    object_id,
                    previous_owner,
                    if crew_status_change {
                        new_crew_member
                    } else {
                        previous_crew_member
                    },
                    new_owner,
                    new_crew_member,
                );
            }
            let current_status = self.objects[index].state.status;
            if fow_range_changed {
                self.actualize_object_fow_view_range(object_id);
            } else if previous_owner != new_owner {
                self.actualize_object_fow_after_owner_change(object_id, new_owner);
            } else if current_status == ObjectStatus::Deleted {
                self.remove_object_from_fow_view_lists(object_id);
            }

            let mut effect_solid_mask_changed = false;
            if !effect_events.is_empty() {
                let previous_container = self.objects[index].state.container;
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                let definitions_ref = &self.definitions;
                let global_view = self.global_effects.clone();
                let rng_state = self.rng.clone();
                let world = self.host_world_context_for_object(index);
                let object = &mut self.objects[index];
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
                    nested_effect_solid_mask_operations,
                    nested_effect_host_raster_preview,
                    nested_effect_solid_mask_changed,
                    _nested_effect_action_callbacks_dispatched,
                    nested_effect_change_def_reinsert,
                    effect_next_object_id,
                    triggered_game_over,
                    effect_script_go,
                    effect_script_counter,
                    audio_state,
                    new_rng,
                ) = Self::run_effect_events_for_object(
                    definition,
                    definitions_ref,
                    self.game_over_triggered,
                    rng_state,
                    object_id,
                    object,
                    effect_events,
                    global_view,
                    &mut self.environment,
                    self.physics,
                    self.frame,
                    world.clone(),
                    self.audio_registry.clone(),
                )?;
                self.stage_host_solid_mask_operations(
                    nested_effect_solid_mask_operations,
                    nested_effect_host_raster_preview,
                );
                self.rng = new_rng;
                self.audio_registry = audio_state;
                effect_solid_mask_changed |= nested_effect_solid_mask_changed;
                if let Some(marker) = nested_effect_change_def_reinsert {
                    change_def_reinsert = marker;
                }
                self.sync_next_object_id(effect_next_object_id);
                if !effect_spawns.is_empty() {
                    self.process_spawn_queue(effect_spawns)?;
                }
                if !effect_transfer_zones.is_empty() {
                    self.apply_transfer_zone_commands(effect_transfer_zones)?;
                }
                if !effect_other_objects.is_empty() {
                    retained.extend(
                        self.apply_nested_object_outcomes_retaining_missing(effect_other_objects)?,
                    );
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
                let new_container = self.objects[index].state.container;
                if previous_container != new_container {
                    container_changes.push((previous_container, new_container));
                }
            }
            self.update_sector_for_index(index);

            for (previous, new) in container_changes {
                self.apply_container_change(object_id, previous, new, false)?;
            }
            if change_def_reinsert {
                self.reinsert_change_def_contents_link(object_id)?;
            }
            if let Some((construction, resolved_position, resolved_fixed_position)) =
                delayed_docon_state
            {
                if let Some(position) = resolved_position {
                    self.objects[index].compiled_mass = None;
                    self.objects[index].state.construction = construction.max(0);
                    self.objects[index].refresh_shape_geometry();
                    self.objects[index].state.position = position;
                    if let Some(fixed_position) = resolved_fixed_position {
                        self.objects[index].fixed_position = fixed_position;
                    }
                } else {
                    let stale_fixed = self.objects[index].fixed_position;
                    self.objects[index].set_construction_from_docon(construction);
                    self.objects[index].fixed_position = stale_fixed;
                }
                self.refresh_object_ocf(index);
                self.update_sector_for_index(index);
            }
            if solid_mask_changed
                || effect_solid_mask_changed
                || self.objects[index].state.base_graphics != previous_base_graphics
            {
                self.update_solid_mask(index);
            }
            if let Some(forced) = requested_death {
                if let Some(death_index) = self.find_object_index(object_id) {
                    self.assign_death(death_index, forced)?;
                }
            }
            if refresh_ocf || energy_died {
                if let Some(refresh_index) = self.find_object_index(object_id) {
                    self.refresh_object_ocf(refresh_index);
                }
            }
            if let Some(ocf) = ocf_override {
                if let Some(override_index) = self.find_object_index(object_id) {
                    self.objects[override_index].state.ocf = ocf;
                }
            }
            if let Some(status_index) = self.find_object_index(object_id) {
                let current_status = self.objects[status_index].state.status;
                self.update_inactive_list_for_status_change(
                    object_id,
                    previous_status,
                    current_status,
                );
                if let Some(shape_override) = final_shape_override {
                    let object = &mut self.objects[status_index];
                    object.state.shape_override = shape_override;
                    match shape_override {
                        Some(rect) => object.shape_rect = Some(rect),
                        None => {
                            object.refresh_shape_geometry();
                            if let Some(vertices) = final_live_vertices {
                                object.set_live_shape_vertices(vertices);
                            }
                            if let Some(vertices) = final_shape_vertices {
                                object.set_shape_vertex_buffer(vertices);
                            }
                        }
                    }
                    self.update_sector_for_index(status_index);
                }
            }
        }
        Ok(retained)
    }

    /// Install callback-final raw contents order after the ordinary
    /// outer/spawn/nested copy-out has established every child's final
    /// `Contained` pointer. C++ performed these link mutations immediately;
    /// this late list-only correction is the copy-in/copy-out equivalent.
    ///
    /// Membership is expected to agree with the ordinary container deltas.
    /// Keep any independently materialized valid child as a fail-safe tail
    /// rather than losing it if a synthetic command batch mixed host calls
    /// with deferred spawn commands.
    fn apply_host_contents_orders(
        &mut self,
        orders: Vec<compat::HostContentsOrder>,
    ) -> Vec<compat::HostContentsOrder> {
        let mut pending = Vec::new();
        for order in orders {
            let Some(container_index) = self.find_object_index(order.container) else {
                pending.push(order);
                continue;
            };
            let mut contents = Vec::with_capacity(order.contents.len());
            let mut missing_child = false;
            for &child in &order.contents {
                let Some(child_index) = self.find_object_index(child) else {
                    missing_child = true;
                    continue;
                };
                let child = &self.objects[child_index];
                if !child.destroyed
                    && child.state.status != ObjectStatus::Deleted
                    && child.state.container == Some(order.container)
                    && !contents.contains(&child.id)
                {
                    contents.push(child.id);
                }
            }

            let current = self.objects[container_index].state.contents.clone();
            for child_id in current {
                let Some(child_index) = self.find_object_index(child_id) else {
                    continue;
                };
                let child = &self.objects[child_index];
                if !child.destroyed
                    && child.state.status != ObjectStatus::Deleted
                    && child.state.container == Some(order.container)
                    && !contents.contains(&child_id)
                {
                    contents.push(child_id);
                }
            }
            self.objects[container_index].state.contents = contents;
            if missing_child {
                pending.push(order);
            }
        }
        pending
    }

    pub fn queue_object_command(
        &mut self,
        id: ObjectId,
        command: QueuedCommand,
    ) -> Result<(), EngineError> {
        self.queue_object_commands(id, std::iter::once(command))
    }

    pub fn queue_object_commands<I>(&mut self, id: ObjectId, commands: I) -> Result<(), EngineError>
    where
        I: IntoIterator<Item = QueuedCommand>,
    {
        let object = self
            .objects
            .iter_mut()
            .find(|object| object.id == id)
            .ok_or(EngineError::UnknownObject(id))?;
        object.enqueue_commands(commands);
        Ok(())
    }
}
