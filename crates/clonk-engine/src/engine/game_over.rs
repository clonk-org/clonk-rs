//! `impl Engine` — game-over checks, objective evaluation and mission handoff.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub(crate) fn check_game_over(&mut self) -> Result<(), EngineError> {
        if self.game_over_triggered || !self.players_registered {
            return Ok(());
        }

        let mut should_trigger = !self.has_active_players();

        if !should_trigger && self.should_evaluate_objectives() && self.objectives_met() {
            should_trigger = true;
        }

        if should_trigger {
            self.request_game_over()?;
        }

        Ok(())
    }

    fn has_active_players(&self) -> bool {
        self.players.values().any(|player| {
            !matches!(
                player.status(),
                PlayerStatus::Eliminated | PlayerStatus::Surrendered
            ) && !player.surrendered()
        })
    }

    fn should_evaluate_objectives(&self) -> bool {
        !self.objectives.is_empty() && self.objective_check_counter == 0
    }

    fn objectives_met(&self) -> bool {
        if self.objectives.is_empty() {
            return false;
        }

        let mut game_over_valid = false;
        let mut game_over = true;

        if !self.objectives.create_objects.is_empty() {
            let mut condition_valid = false;
            let mut condition_true = true;
            for objective in &self.objectives.create_objects {
                if objective.count <= 0 {
                    continue;
                }
                condition_valid = true;
                let target_id = objective.definition.as_str();
                let current = self
                    .objects
                    .iter()
                    .filter(|object| object.definition_id.as_str() == target_id)
                    .filter(|object| object.state.status.is_active())
                    .filter(|object| object.state.construction >= FULL_CON)
                    .count() as i32;
                if current < objective.count {
                    condition_true = false;
                }
            }
            if condition_valid {
                game_over_valid = true;
                if !condition_true {
                    game_over = false;
                }
            }
        }

        if !self.objectives.clear_objects.is_empty() {
            let mut condition_valid = false;
            let mut condition_true = true;
            for objective in &self.objectives.clear_objects {
                condition_valid = true;
                let limit = objective.count.max(0);
                let target_id = objective.definition.as_str();
                let alive_only = self
                    .definitions
                    .get(target_id)
                    .map(|definition| definition.category() & CATEGORY_LIVING != 0)
                    .unwrap_or(false);
                let count = self
                    .objects
                    .iter()
                    .filter(|object| object.definition_id.as_str() == target_id)
                    .filter(|object| object.state.status.is_active())
                    .filter(|object| !alive_only || object.state.alive)
                    .count() as i32;
                if count > limit {
                    condition_true = false;
                }
            }
            if condition_valid {
                game_over_valid = true;
                if !condition_true {
                    game_over = false;
                }
            }
        }

        if !self.objectives.clear_materials.is_empty() {
            let mut condition_valid = false;
            let mut condition_true = true;
            if let Some(landscape) = self.landscape.as_ref() {
                for objective in &self.objectives.clear_materials {
                    if let Some(material_id) = self.materials.id_of(&objective.material) {
                        condition_valid = true;
                        let limit = i64::from(objective.count.max(0));
                        let total = self.count_material_pixels(landscape, material_id);
                        if total > limit {
                            condition_true = false;
                        }
                    }
                }
            }
            if condition_valid {
                game_over_valid = true;
                if !condition_true {
                    game_over = false;
                }
            }
        }

        game_over_valid && game_over
    }

    fn count_material_pixels(&self, landscape: &Landscape, material_id: MaterialId) -> i64 {
        let mut total: i64 = 0;

        for x in 0..landscape.width() {
            if landscape.solid_material_at(x as i32) == Some(material_id) {
                let height = landscape
                    .surface()
                    .get(x as usize)
                    .copied()
                    .unwrap_or_default()
                    .max(0);
                total += i64::from(height);
            }
        }

        for column in landscape.liquids() {
            for segment in column.segments() {
                if segment.material == Some(material_id) {
                    let span = i64::from(segment.bottom) - i64::from(segment.top);
                    if span > 0 {
                        total += span;
                    }
                }
            }
        }

        total
    }

    pub(crate) fn request_game_over(&mut self) -> Result<bool, EngineError> {
        if self.game_over_triggered {
            return Ok(false);
        }
        self.game_over_triggered = true;
        tolerate_script_error(self.broadcast_scenario_function("OnGameOver", Vec::new()))?;
        // C4Game::DoGameOver marks winners only after OnGameOver, so any
        // player eliminated by that callback is not promoted (C4Game.cpp:
        // 3659-3670).
        for player in self.players.values_mut() {
            if !matches!(
                player.status(),
                PlayerStatus::Eliminated | PlayerStatus::Surrendered
            ) && !player.surrendered()
            {
                player.mark_won();
            }
        }
        Ok(true)
    }

    pub(crate) fn evaluate_game(&mut self) -> Result<(), EngineError> {
        if self.game_evaluated {
            return Ok(());
        }

        // Cooperative games award every player the average of all positive
        // ValueGain values; C4Player::Evaluate selects the player's own gain
        // instead when the converted scenario goal list is melee.
        let average_value_gain = if self.players.is_empty() {
            0
        } else {
            let sum = self.players.values().fold(0_i32, |sum, player| {
                sum.wrapping_add(player.value_gain().max(0))
            });
            sum / i32::try_from(self.players.len()).unwrap_or(i32::MAX)
        };

        let player_numbers = self.player_ids_in_order();
        for number in player_numbers {
            let evaluated = self.evaluate_player(number, average_value_gain)?;
            let Some((player_info_id, total_playing_time, score_old, score_new)) = evaluated else {
                continue;
            };
            let league_progress_data = self
                .player_info_league_progress_data
                .get(&player_info_id)
                .cloned()
                .flatten();
            if let Some(result) = self
                .round_results
                .players
                .iter_mut()
                .find(|result| result.player_info_id == player_info_id)
            {
                result.total_playing_time = total_playing_time;
                result.score_old = score_old;
                result.score_new = Some(score_new);
                result.league_progress_data = league_progress_data;
            } else {
                self.round_results.players.push(RoundResultsPlayerState {
                    player_info_id,
                    total_playing_time,
                    score_old,
                    score_new: Some(score_new),
                    league_progress_data,
                    ..RoundResultsPlayerState::default()
                });
            }
        }

        let (goals, fulfilled_goals) = self.evaluate_round_goals()?;
        self.round_results.goal_counts = goals.iter().cloned().map(|goal| (goal, 1)).collect();
        self.round_results.goals = goals;
        self.round_results.fulfilled_goals = fulfilled_goals;
        // C4RoundResults::EvaluateGame writes playing time after all goal
        // callbacks, whose scripts may mutate Game.Time.
        self.round_results.playing_time_seconds = self.game_time as u32;
        self.game_evaluated = true;
        Ok(())
    }

    #[doc(hidden)]
    pub fn evaluate_round_goals(
        &mut self,
    ) -> Result<(Vec<DefinitionId>, Vec<DefinitionId>), EngineError> {
        let first_local_player = self
            .player_ids_in_order()
            .into_iter()
            .find(|player| {
                self.local_players
                    .as_ref()
                    .is_none_or(|local| local.contains(player))
            })
            .unwrap_or(OWNER_NONE);
        self.evaluate_goals_for_player(first_local_player)
    }

    pub(crate) fn evaluate_goals_for_player(
        &mut self,
        player: i32,
    ) -> Result<(Vec<DefinitionId>, Vec<DefinitionId>), EngineError> {
        let rivalry = self.exec_list.iter().rev().any(|&object_id| {
            let Some(index) = self.find_object_index(object_id) else {
                return false;
            };
            let object = &self.objects[index];
            !object.destroyed
                && object.state.status.is_active()
                && object.definition_id.as_str() == "RVLR"
        });

        let mut goals = Vec::new();
        let mut fulfilled = Vec::new();
        // C4ObjectList::GetListID uses a 500-entry temporary ID table.
        for goal_index in 0..500 {
            // GetListID rebuilds its unique-ID list on every call. Goal
            // callbacks can remove objects, so the cnt-th ID must be found
            // again from the current master list (C4ObjectList.cpp:58-78).
            let goal = {
                let mut seen = HashSet::new();
                self.exec_list
                    .iter()
                    .rev()
                    .filter_map(|&object_id| {
                        let index = self.find_object_index(object_id)?;
                        let object = &self.objects[index];
                        if object.destroyed || !object.state.status.is_active() {
                            return None;
                        }
                        let definition = self.definitions.get(&object.definition_id)?;
                        (definition.category() & CATEGORY_GOAL != 0
                            && seen.insert(object.definition_id.clone()))
                        .then(|| object.definition_id.clone())
                    })
                    .nth(goal_index)
            };
            let Some(goal) = goal else {
                break;
            };

            // C4ObjectList::Find re-resolves the first live instance for
            // every distinct goal ID; an earlier callback may have removed
            // the instance that originally contributed the ID.
            let target = self.exec_list.iter().rev().find_map(|&object_id| {
                let index = self.find_object_index(object_id)?;
                let object = &self.objects[index];
                (!object.destroyed
                    && object.state.status.is_active()
                    && object.definition_id == goal)
                    .then_some(index)
            });
            let is_fulfilled = if let Some(index) = target {
                let (function, args) = if rivalry {
                    ("IsFulfilledforPlr", vec![Value::Int(player)])
                } else {
                    ("IsFulfilled", Vec::new())
                };
                tolerate_script_error(self.call_object_function(index, function, args))?
                    .is_some_and(|value| compat::value_raw_truthy(&value))
            } else {
                false
            };
            goals.push(goal.clone());
            if is_fulfilled {
                fulfilled.push(goal);
            }
        }
        Ok((goals, fulfilled))
    }

    /// First active object with this definition in C++ master-list order.
    /// Player goal/rule menu commands re-resolve the object at click time.
    pub fn first_active_object_for_definition(&self, definition: &str) -> Option<ObjectId> {
        self.exec_list.iter().rev().find_map(|&object_id| {
            let index = self.find_object_index(object_id)?;
            let object = &self.objects[index];
            (!object.destroyed
                && object.state.status.is_active()
                && object.definition_id == definition)
                .then_some(object_id)
        })
    }

    pub fn next_mission(&self) -> &NextMissionState {
        &self.next_mission
    }

    /// Raw `C4NetworkRestartInfos::Infos::What` mask retained for the future
    /// app/network restart handoff. Bits outside the known 0x1/0x2 flags are
    /// observable by design because the C++ script host stores them verbatim.
    pub const fn restart_restore_info_mask(&self) -> i32 {
        self.restart_restore_info_mask
    }

    pub(crate) fn apply_next_mission_commands(
        &mut self,
        commands: impl IntoIterator<Item = NextMissionCommand>,
    ) {
        for command in commands {
            match command {
                NextMissionCommand::Set {
                    path,
                    text,
                    description,
                } => {
                    self.next_mission = NextMissionState {
                        path,
                        text,
                        description,
                    };
                }
                NextMissionCommand::Clear => {
                    self.next_mission.path.clear();
                    self.next_mission.text.clear();
                    // FnSetNextMission deliberately leaves NextMissionDesc
                    // untouched when clearing (C4Script.cpp:6055-6061).
                }
            }
        }
    }

    #[doc(hidden)]
    pub fn apply_scenario_batch(
        &mut self,
        batch: ScenarioBatch,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let solid_mask_operations = batch.solid_mask_operations.0.clone();
        let host_raster_preview = batch.host_raster_preview.0.clone();
        let was_deferred = self.defer_solid_mask_updates;
        let mut outermost =
            self.stage_host_solid_mask_operations(solid_mask_operations, host_raster_preview);
        let result = self.apply_scenario_batch_inner(batch);
        outermost |= !was_deferred && self.defer_solid_mask_updates;
        self.finish_host_solid_mask_operations(outermost, result)
    }

    fn apply_scenario_batch_inner(
        &mut self,
        batch: ScenarioBatch,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let ScenarioBatch {
            spawns,
            other_objects,
            global_effects,
            environment,
            physics,
            landscape_ops,
            solid_mask_operations: _,
            host_raster_preview: _,
            landscape,
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
        } = batch;

        if !player_commands.is_empty() {
            self.apply_player_commands(player_commands)?;
        }
        self.pending_object_order_commands
            .extend(object_order_commands);
        self.apply_next_mission_commands(next_mission_commands);

        if !landscape_ops.is_empty() {
            self.apply_landscape_operations(landscape_ops);
        }

        if let Some(delta) = environment {
            self.apply_environment_delta(&delta);
        }
        if let Some(delta) = physics {
            self.apply_physics_delta(delta);
        }
        if !global_effects.is_empty() {
            self.apply_global_effect_commands(&global_effects);
        }
        if !landscape.is_empty() {
            let mut landscape_slot = self.landscape.take();
            if let Some(landscape_ref) = landscape_slot.as_mut() {
                for command in landscape {
                    command.apply(landscape_ref);
                }
            }
            self.landscape = landscape_slot;
            // C++ landscape drawing never touches the mass-mover set —
            // movers pinned to changed pixels die on their next Execute
            // (C4MassMover.cpp:119).
        }
        self.apply_particle_commands(particles);
        if !audio.is_empty() {
            self.pending_audio.extend(audio);
        }
        if !messages.is_empty() {
            for command in messages {
                self.messages.apply_command(command);
            }
        }

        // Pre-scan spawns to find maximum explicit ID and reserve ID space
        // This prevents conflicts between auto-assigned IDs (from earlier objects like crew)
        // and explicit IDs (from scenario Objects.txt)
        let max_explicit_id = spawns
            .iter()
            .filter_map(|spawn| spawn.id)
            .map(|id| id.as_u64())
            .max();

        if let Some(max_id) = max_explicit_id {
            // Reserve ID space: ensure next_object_id is beyond all explicit IDs
            if max_id >= self.next_object_id {
                self.next_object_id = max_id + 1;
            }
        }

        let mut created = Vec::with_capacity(spawns.len());
        for spawn in spawns {
            match self.spawn_object(spawn) {
                Ok(id) => created.push(id),
                // CreateObject resolves the id with C4Id2Def and yields
                // nullptr for unknown definitions — never an error
                // (Drachenfels' Initialize creates `_EAI` before its def
                // loads). Mirrors the process_spawn_queue tolerance.
                Err(EngineError::UnknownDefinition(definition)) => {
                    tracing::warn!(%definition, "scenario spawn names an unknown definition; skipped");
                }
                Err(error) => return Err(error),
            }
        }
        // Transfer zones fold AFTER the spawns: C4Game::NewObject adds the
        // object to Game.Objects BEFORE its creation callbacks fire
        // (C4Game.cpp:1115-1131), so a SetTransferZone recorded during the
        // scenario Initialize (C4Script.cpp:3145-3149) always found its
        // owner live — including owners this very batch creates.
        if !transfer_zones.is_empty() {
            self.apply_transfer_zone_commands(transfer_zones)?;
        }
        // Nested-call outcomes fold AFTER the spawns: scripts arrow-call
        // objects they just created (C++ creates them live mid-call), so
        // outcomes may target this batch's fresh ids.
        self.apply_nested_object_outcomes(other_objects)?;
        if let Some(go) = script_go {
            self.scenario_script_go = go;
        }
        if let Some(counter) = script_counter {
            self.scenario_script_counter = counter;
        }
        if trigger_game_over {
            self.request_game_over()?;
        }
        Ok(created)
    }

}
