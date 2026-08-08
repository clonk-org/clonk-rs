//! `impl Engine` — fog-of-war views, player assets and command resumption.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    #[doc(hidden)]
    pub fn tick_player_systems(&mut self) -> Result<(), EngineError> {
        if self.players.is_empty() {
            // Playerless engine fixtures predate runtime C4Player records but
            // still project crew-owner elimination. Preserve that import/test
            // compatibility on the same Tick35 boundary as the old path.
            if self.frame.is_multiple_of(35) {
                let crew_owners = self.refresh_elimination_state();
                let mut known = self.known_crew_owners.iter().copied().collect::<Vec<_>>();
                known.sort_unstable();
                for owner in known {
                    if !crew_owners.contains(&owner) {
                        self.eliminated_crew_owners.insert(owner);
                    }
                }
            }
            return Ok(());
        }

        let player_ids = self.player_ids_in_order();
        let mut retire_player = None;
        for id in player_ids {
            if self.execute_one_player(id, true)? && retire_player.is_none() {
                retire_player = Some(id);
            }
        }
        if let Some(player) = retire_player {
            self.retire_player(player)?;
        }
        Ok(())
    }

    pub(crate) fn remove_object_from_fow_view_lists(&mut self, object_id: ObjectId) {
        for player in self.players.values_mut() {
            player.remove_fow_view_object(object_id);
        }
    }

    /// `C4Object::PlrFoWActualize`: a nonzero range belongs to the valid
    /// owner's runtime FoW list, or to every current player when ownerless.
    /// These lists are intentionally absent from save data.
    pub(crate) fn actualize_object_fow_view_range(&mut self, object_id: ObjectId) {
        self.remove_object_from_fow_view_lists(object_id);
        let Some(index) = self.find_object_index(object_id) else {
            return;
        };
        let object = &self.objects[index];
        if object.destroyed
            || object.state.status == ObjectStatus::Deleted
            || object.state.plr_view_range == 0
        {
            return;
        }
        let owner = object.state.owner;
        if let Some(player) = self.players.get_mut(&owner) {
            player.add_fow_view_object(object_id);
        } else {
            for player in self.players.values_mut() {
                player.add_fow_view_object(object_id);
            }
        }
    }

    /// `C4Object::SetOwner` removes the old membership, but its NO_OWNER arm
    /// does not call `PlrFoWActualize` and therefore does not immediately add
    /// the object to every player.
    pub(crate) fn actualize_object_fow_after_owner_change(
        &mut self,
        object_id: ObjectId,
        new_owner: i32,
    ) {
        self.remove_object_from_fow_view_lists(object_id);
        if new_owner != OWNER_NONE {
            self.actualize_object_fow_view_range(object_id);
        }
    }

    pub(crate) fn actualize_ownerless_fow_objects_for_new_player(&mut self) {
        // C4Player::Init walks Game.Objects First -> Next. `exec_list` is the
        // reverse C++ master list, hence the reversed iterator here.
        let object_ids = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        for object_id in object_ids {
            let ownerless = self.find_object_index(object_id).is_some_and(|index| {
                let object = &self.objects[index];
                !object.destroyed
                    && object.state.status != ObjectStatus::Deleted
                    && object.state.owner == OWNER_NONE
                    && object.state.plr_view_range != 0
            });
            if ownerless {
                self.actualize_object_fow_view_range(object_id);
            }
        }
    }

    pub(crate) fn rebuild_fow_view_objects(&mut self) {
        for player in self.players.values_mut() {
            player.clear_fow_view_objects();
        }
        // C4ObjectList::AssignPlrViewRange walks Last -> Prev, which is the
        // stored order of the reverse-master `exec_list`.
        let object_ids = self.exec_list.clone();
        for object_id in object_ids {
            self.actualize_object_fow_view_range(object_id);
        }
    }

    fn decay_dead_fow_view_objects(&mut self, player_id: i32) {
        let view_objects = self
            .players
            .get(&player_id)
            .map(|player| player.fow_view_objects().to_vec())
            .unwrap_or_default();
        for object_id in view_objects {
            let Some(index) = self.find_object_index(object_id) else {
                if let Some(player) = self.players.get_mut(&player_id) {
                    player.remove_fow_view_object(object_id);
                }
                continue;
            };
            let object = &mut self.objects[index];
            if object.destroyed || object.state.status == ObjectStatus::Deleted {
                if let Some(player) = self.players.get_mut(&player_id) {
                    player.remove_fow_view_object(object_id);
                }
                continue;
            }
            if !object.state.alive && object.state.category & CATEGORY_LIVING != 0 {
                object.state.plr_view_range = object.state.plr_view_range.wrapping_sub(10);
                if object.state.plr_view_range <= 0 {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.remove_fow_view_object(object_id);
                    }
                }
            }
        }
    }

    /// One complete C4Player::Execute pass. The list caller invokes this in
    /// player-number order; FinalInit invokes it for just the joining/restored
    /// player and ignores the list-level retirement result.
    pub(crate) fn execute_one_player(
        &mut self,
        id: i32,
        check_elimination: bool,
    ) -> Result<bool, EngineError> {
        if self
            .players
            .get(&id)
            .is_none_or(|player| player.status() == PlayerStatus::Inactive)
        {
            return Ok(false);
        }

        // C4Player::UpdateCounts is the first Tick1 step. SelectCount remains
        // stale through callbacks later in this same Execute.
        self.refresh_player_select_count(id);
        let crew_nonempty = self
            .players
            .get(&id)
            .is_some_and(|player| !player.crew().is_empty());
        self.update_player_view(id);
        self.execute_player_control_and_menu(id)?;
        // C4Player::Execute decays dead living FoW targets after control/menu
        // and before Tick35 work, independently of whether FoW is enabled.
        self.decay_dead_fow_view_objects(id);

        let normal_status = self.players.get(&id).is_some_and(|player| {
            matches!(
                player.status(),
                PlayerStatus::Active | PlayerStatus::Eliminated | PlayerStatus::Surrendered
            )
        });
        if self.frame.is_multiple_of(35) && normal_status {
            let team = self.players.get(&id).and_then(Player::team);
            let valid_team_home_base = team.filter(|team| {
                self.team_home_base_rule
                    && self
                        .team_state
                        .teams
                        .iter()
                        .any(|candidate| candidate.id == *team)
            });
            let should_produce = match valid_team_home_base {
                Some(team) => self.runtime_team_members_in_order(team).first().copied() == Some(id),
                _ => true,
            };
            let team_update = self.players.get_mut(&id).and_then(|player| {
                (player.advance_home_base_production_as_leader(should_produce)
                    && valid_team_home_base.is_some())
                .then(|| {
                    player
                        .team()
                        .map(|team| (team, player.home_base_material_entries().to_vec()))
                })
                .flatten()
            });
            if let Some((team, material)) = team_update {
                for player in self
                    .players
                    .values_mut()
                    .filter(|player| player.team() == Some(team))
                {
                    player.set_home_base_material_entries(material.clone());
                }
            }
            self.update_player_asset_value(id)?;
            if check_elimination
                && !crew_nonempty
                && self.players.get(&id).is_some_and(|player| {
                    player.status() == PlayerStatus::Active && !player.no_elimination_check()
                })
            {
                self.eliminated_crew_owners.insert(id);
                if let Some(player) = self.players.get_mut(&id) {
                    player.eliminate();
                }
            }
        }

        // C4Player::ExecMsgBoardQueries runs after Tick35 production/value/
        // elimination work, and only for a normal local player. One global
        // C4ChatInputDialog serializes prompts across all local players.
        if self.frame.is_multiple_of(35) && normal_status {
            self.open_next_message_board_input(id);
        }

        Ok(self.finish_player_execute_delays(id))
    }

    fn open_next_message_board_input(&mut self, player_id: i32) {
        if self.active_message_board_input.is_some()
            || self
                .local_players
                .as_ref()
                .is_some_and(|players| !players.contains(&player_id))
            || self
                .players
                .get(&player_id)
                .is_some_and(Player::is_script_player)
        {
            return;
        }
        let query = self
            .players
            .get(&player_id)
            .and_then(|player| {
                player
                    .message_board_queries()
                    .iter()
                    .find(|query| !query.answered)
            })
            .cloned();
        if let Some(query) = query {
            self.active_message_board_input = Some(ActiveMessageBoardInput {
                player: player_id,
                target: query.target,
                prompt: query.prompt,
                uppercase: query.uppercase,
            });
        }
    }

    pub(crate) fn update_player_view(&mut self, player_id: i32) {
        let Some(player) = self.players.get(&player_id) else {
            return;
        };
        if matches!(player.status(), PlayerStatus::Inactive) {
            return;
        }
        let focus = player.resolved_view_object();
        let position = focus.and_then(|object_id| {
            self.objects
                .iter()
                .find(|object| {
                    object.id == object_id
                        && !object.destroyed
                        && !matches!(object.state.status, ObjectStatus::Deleted)
                })
                .map(|object| object.state.position)
        });
        if let Some(player) = self.players.get_mut(&player_id) {
            player.update_view(position);
        }
    }

    #[doc(hidden)]
    pub fn update_player_asset_values(&mut self) -> Result<(), EngineError> {
        if self.players.is_empty() {
            return Ok(());
        }

        let ids = self
            .players
            .iter()
            .filter_map(|(&id, player)| {
                matches!(
                    player.status(),
                    PlayerStatus::Active | PlayerStatus::Eliminated | PlayerStatus::Surrendered
                )
                .then_some(id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            self.update_player_asset_value(id)?;
        }
        Ok(())
    }

    fn call_player_asset_object_value(
        &mut self,
        object_id: ObjectId,
        player: i32,
    ) -> Result<i32, EngineError> {
        let index = self
            .find_object_index(object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        let (definition_id, construction, state_snapshot) = {
            let object = &self.objects[index];
            (
                object.definition_id.clone(),
                object.state.construction,
                Rc::new(object.script_state_snapshot()),
            )
        };
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        if !definition.has_function("CalcValue") && !definition.has_function("CalcDefValue") {
            return Ok(definition.value().wrapping_mul(construction) / FULL_CON);
        }
        let action_library = definition.action_library().clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let call = definition.player_asset_object_value(
            state_snapshot.as_ref(),
            object_id,
            player,
            self.rng.clone(),
            &self.global_effects.clone(),
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (value, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            Err(error) => {
                return Err(self.apply_script_error_recovery(
                    error,
                    index,
                    &action_library,
                    object_id,
                    &definition_id,
                    true,
                ));
            }
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_callback_outcome(
            index,
            outcome,
            &action_library,
            object_id,
            &definition_id,
            true,
        )?;
        Ok(value)
    }

    pub(crate) fn update_player_asset_value(&mut self, id: i32) -> Result<(), EngineError> {
        let Some(previous) = self
            .players
            .get_mut(&id)
            .map(Player::begin_asset_value_update)
        else {
            return Ok(());
        };
        let mut visited = HashSet::new();
        let mut next = self.exec_list.last().copied();
        while let Some(object_id) = next {
            visited.insert(object_id);
            let successors = self
                .exec_list
                .iter()
                .position(|candidate| *candidate == object_id)
                .map(|position| {
                    self.exec_list[..position]
                        .iter()
                        .rev()
                        .copied()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let owned = self.find_object_index(object_id).is_some_and(|index| {
                self.objects[index].state.status.is_active()
                    && self.objects[index].state.owner == id
            });
            if owned {
                if let Some(player) = self.players.get_mut(&id) {
                    player.count_owned_asset();
                }
                let object_value =
                    tolerate_script_error(self.call_player_asset_object_value(object_id, id))?
                        .unwrap_or(0);
                if let Some(player) = self.players.get_mut(&id) {
                    player.add_asset_value(object_value);
                }
            }
            next = if let Some(position) = self
                .exec_list
                .iter()
                .position(|candidate| *candidate == object_id)
            {
                self.exec_list[..position]
                    .iter()
                    .rev()
                    .copied()
                    .find(|candidate| !visited.contains(candidate))
            } else {
                successors.into_iter().find(|candidate| {
                    !visited.contains(candidate) && self.exec_list.contains(candidate)
                })
            };
        }
        if let Some(player) = self.players.get_mut(&id) {
            player.finish_asset_value_update(previous);
        }
        Ok(())
    }

    fn refresh_player_select_count(&mut self, id: i32) {
        let Some(crew) = self.players.get(&id).map(|player| player.crew().to_vec()) else {
            return;
        };
        let count = crew
            .iter()
            .filter(|member| {
                self.objects
                    .iter()
                    .find(|object| object.id == **member)
                    .is_some_and(|object| object.state.selected)
            })
            .count();
        if let Some(player) = self.players.get_mut(&id) {
            player.set_select_count(i32::try_from(count).unwrap_or(i32::MAX));
        }
    }

    #[doc(hidden)]
    pub fn trigger_lightning(&mut self, position: i32) -> Result<bool, EngineError> {
        self.launch_lightning_effect(position, 0, -20, 41, 5, 15, true)
    }

    /// `C4Weather::LaunchLightning`: creatorless FXL1 at the native
    /// CreateObject default position, followed by a fail-safe Activate call.
    /// The C++ function returns true unconditionally (C4Weather.cpp:153-165;
    /// C4Game.h:229-231).
    fn launch_lightning_effect(
        &mut self,
        x: i32,
        y: i32,
        xdir: i32,
        xrange: i32,
        ydir: i32,
        yrange: i32,
        gamma: bool,
    ) -> Result<bool, EngineError> {
        const LIGHTNING_DEFINITION: &str = "FXL1";
        if !self.definitions.contains_key(LIGHTNING_DEFINITION) {
            return Ok(true);
        }
        let config = SpawnConfig::new(LIGHTNING_DEFINITION).with_position(Vector2::new(50, 50));
        let lightning_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(true),
            Err(err) => return Err(err),
        };
        let Some(index) = self.find_object_index(lightning_id) else {
            return Ok(true);
        };
        let args = vec![
            Value::Int(x),
            Value::Int(y),
            Value::Int(xdir),
            Value::Int(xrange),
            Value::Int(ydir),
            Value::Int(yrange),
            Value::Bool(gamma),
        ];
        // C4Object::Call defaults to fPassError=false, so an Activate script
        // error is logged after preserving partial mutations and weather
        // execution continues (C4Object.h:240; C4Object.cpp:2224-2227).
        let _ = tolerate_script_error(self.call_object_function(index, "Activate", args))?;
        Ok(true)
    }

    /// Wind audio from `C4Weather::Execute` (C4Weather.cpp:94-104), after
    /// the Tick10 wind step and before any disaster RNG draws.
    pub(crate) fn tick_weather_wind_audio(&mut self, frame: u64) {
        if !frame.is_multiple_of(10) {
            return;
        }
        let volume = self.environment.wind.abs().saturating_sub(30) * 2;
        self.audio_registry.sound_level("Wind", None, volume);
        self.pending_audio.extend(self.audio_registry.take_events());
    }

    /// Disaster launch from `C4Weather::Execute` (C4Weather.cpp:104-148),
    /// run on Tick10 frames. The gate Random draws are unconditional — the
    /// configured levels only decide whether a launch follows — so the synced
    /// RNG stream advances identically whether or not disasters are enabled.
    #[doc(hidden)]
    pub fn tick_weather_events(&mut self, frame: u64) -> Result<(), EngineError> {
        if !frame.is_multiple_of(10) {
            return Ok(());
        }
        let width = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.width() as i32)
            .unwrap_or(0);
        let height = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.estimated_height())
            .unwrap_or(0);

        // Meteorite (C4Weather.cpp:106-120)
        if self.rng.random(60) == 0 && self.rng.random(100) < self.environment.meteorite {
            // force argument evaluation order (C4Weather.cpp:113-115)
            let r2 = self.rng.random(100 + 1);
            let r1 = self.rng.random(width);
            if self.trigger_meteorite(r1, r2)? {
                self.weather_events.push(WeatherEvent::Meteorite { x: r1 });
            }
        }
        // Lightning (C4Weather.cpp:122-127)
        if self.rng.random(35) == 0 && self.rng.random(100) < self.environment.lightning {
            let position = self.rng.random(width);
            if self.trigger_lightning(position)? {
                self.weather_events
                    .push(WeatherEvent::Lightning { position });
            }
        }
        // Earthquake (C4Weather.cpp:129-136)
        if self.rng.random(50) == 0 && self.rng.random(100) < self.environment.earthquake {
            // force argument evaluation order (C4Weather.cpp:132-134)
            let r2 = self.rng.random(height);
            let r1 = self.rng.random(width);
            if self.trigger_earthquake(r1, r2)? {
                self.weather_events
                    .push(WeatherEvent::Earthquake { x: r1, y: r2 });
            }
        }
        // Volcano (C4Weather.cpp:138-147)
        if self.rng.random(60) == 0 && self.rng.random(100) < self.environment.volcano {
            // force argument evaluation order (C4Weather.cpp:141-143)
            let r2 = self.rng.random(10);
            let r1 = self.rng.random(width);
            let size = (15 * height / 500 + r2).clamp(10, 60);
            if self.trigger_volcano(r1, height - 1, size)? {
                self.weather_events.push(WeatherEvent::Volcano {
                    x: r1,
                    y: height - 1,
                    size,
                });
            }
        }
        Ok(())
    }

    /// Meteor creation (C4Weather.cpp:110-119): "METO" at y=-20/ydir=0
    /// with an open top, or y=5/ydir=itofix(2) in a closed cave; xdir is
    /// itofix(r2-50)/10 and rdir is itofix(1)/5 in both cases.
    fn trigger_meteorite(&mut self, x: i32, r2: i32) -> Result<bool, EngineError> {
        const METEOR_DEFINITION: &str = "METO";
        if !self.definitions.contains_key(METEOR_DEFINITION) {
            return Ok(false);
        }
        let top_open = self
            .landscape
            .as_ref()
            .is_none_or(crate::landscape::Landscape::top_open);
        let (y, ydir) = if top_open {
            (-20, C4Fixed::ZERO)
        } else {
            (5, itofix(2))
        };
        let xdir = C4Fixed::from_raw(itofix(r2 - 50).val() / 10);
        let rdir = C4Fixed::from_raw(itofix(1).val() / 5);
        let config = SpawnConfig::new(METEOR_DEFINITION)
            .with_position(Vector2::new(x.max(0), y))
            .with_fixed_velocity(FixedVec2::new(xdir, ydir))
            .with_rotation_velocity(rdir);
        let meteor_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(false),
            Err(err) => return Err(err),
        };
        Ok(self.find_object_index(meteor_id).is_some())
    }

    /// `LaunchEarthquake` (C4Weather.cpp:196-203): FXQ1 + Activate().
    fn trigger_earthquake(&mut self, x: i32, y: i32) -> Result<bool, EngineError> {
        const EARTHQUAKE_DEFINITION: &str = "FXQ1";
        if !self.definitions.contains_key(EARTHQUAKE_DEFINITION) {
            return Ok(false);
        }
        let config =
            SpawnConfig::new(EARTHQUAKE_DEFINITION).with_position(Vector2::new(x.max(0), y.max(0)));
        let quake_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(false),
            Err(err) => return Err(err),
        };
        let Some(index) = self.find_object_index(quake_id) else {
            return Ok(false);
        };
        // Unlike lightning/volcano, LaunchEarthquake succeeds only when the
        // fail-safe Activate call returns truthy (C4Weather.cpp:196-203).
        let activated =
            tolerate_script_error(self.call_object_function(index, "Activate", Vec::new()))?;
        Ok(activated.is_some_and(|value| compat::value_raw_truthy(&value)))
    }

    /// `LaunchVolcano` (C4Weather.cpp:178-184): FXV1 + Activate(x, y, size,
    /// mat) with mat = Material "Lava" (C4Weather.cpp:144).
    fn trigger_volcano(&mut self, x: i32, y: i32, size: i32) -> Result<bool, EngineError> {
        const VOLCANO_DEFINITION: &str = "FXV1";
        if !self.definitions.contains_key(VOLCANO_DEFINITION) {
            return Ok(true);
        }
        // LaunchVolcano creates FXV1 at C4Game::CreateObject's native
        // default (50,50) and passes requested x/y only to Activate.
        let config = SpawnConfig::new(VOLCANO_DEFINITION).with_position(Vector2::new(50, 50));
        let volcano_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(true),
            Err(err) => return Err(err),
        };
        let Some(index) = self.find_object_index(volcano_id) else {
            return Ok(true);
        };
        let lava = self
            .materials
            .id_of("Lava")
            .map(|id| id.index() as i32)
            .unwrap_or(-1);
        let args = vec![
            Value::Int(x),
            Value::Int(y),
            Value::Int(size),
            Value::Int(lava),
        ];
        // Same fail-safe C4Object::Call contract as LaunchLightning.
        let _ = tolerate_script_error(self.call_object_function(index, "Activate", args))?;
        Ok(true)
    }

    pub(crate) fn active_solid_mask_indices(&self) -> Vec<usize> {
        let definitions_have_solid_masks = self
            .definitions
            .values()
            .any(|definition| definition.solid_mask().is_some());
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, object)| {
                if object
                    .state
                    .solid_mask_override
                    .is_some_and(|rect| rect.width > 0 && rect.height > 0)
                {
                    return true;
                }
                if !definitions_have_solid_masks {
                    return false;
                }
                #[cfg(test)]
                SOLID_MASK_DEFINITION_LOOKUPS.with(|count| count.set(count.get() + 1));
                self.definitions
                    .get(&object.definition_id)
                    .is_some_and(|definition| definition.solid_mask().is_some())
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub(crate) fn refresh_command_master_list_order(
        exec_list: &[ObjectId],
        command_snapshots: &mut CommandObjectSnapshots,
    ) {
        for snapshot in command_snapshots.values_mut() {
            snapshot.master_list_order = usize::MAX;
        }
        for (master_list_order, &id) in exec_list.iter().rev().enumerate() {
            if let Some(snapshot) = command_snapshots.get_mut(&id) {
                snapshot.master_list_order = master_list_order;
            }
        }
    }

    /// Build the live command view for an object inserted after the frame's
    /// bulk command snapshot. C++'s live ExecObjects iterator still runs
    /// ExecuteCommand for such newborn objects in the same frame.
    pub(crate) fn live_command_snapshot(&self, index: usize) -> CommandObjectSnapshot {
        let physical = self.object_physical_without_fair_fill(index);
        let object = &self.objects[index];
        let master_list_order = self
            .exec_list
            .iter()
            .rev()
            .position(|&id| id == object.id)
            .unwrap_or_else(|| self.exec_list.len().saturating_add(index));
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
                (
                    definition.action_library().procedure_for_entry(
                        &object.state.action.name,
                        object.state.action.act_map_index,
                    ),
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
        CommandObjectSnapshot {
            id: object.id,
            master_list_order,
            definition_id: object.definition_id.clone(),
            position: object.state.position,
            fixed_position: object.fixed_position,
            fixed_velocity: object.fixed_velocity,
            move_to_range,
            pathfinder,
            no_transfer_zones,
            no_push_enter,
            contact: object.frame_t_contact,
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
            action_time: object.state.action.time,
            command_direction: object.state.command_direction,
            construction: object.state.construction,
            direction: object.state.direction,
            physical,
            physical_deferred: false,
            owner: object.state.owner,
            controller: object.state.controller,
            base: object.state.base,
            crew_member: object.state.crew_member,
            selected: object.state.selected,
            alive: object.state.alive,
            need_energy: object.state.need_energy,
            on_fire: object.state.on_fire,
            contents: object.state.contents.clone(),
            commands: object.commands.command_views(),
            line_connect,
            ocf: object.state.ocf,
            entrance_status: object.state.entrance_status,
            collectible,
        }
    }

    /// The synchronous `C4Object::ExecuteCommand` path used by controls
    /// that issue and immediately execute an order (notably contained
    /// Throw). It shares the retained-front callback/clear tail with the
    /// ordinary object tick.
    pub(crate) fn execute_object_command_now(
        &mut self,
        object_id: ObjectId,
    ) -> Result<(), EngineError> {
        self.execute_object_command_now_inner(object_id, ImmediateCommandResume::Front)
            .map(|_| ())
    }

    /// Resume the post-ObjectComStop half of the retained MoveTo without a
    /// second UpdateInterval decrement. This is still the same native
    /// C4Command::Execute invocation; only the live callback boundary made
    /// the engine rebuild its command snapshot.
    pub(crate) fn resume_move_to_after_stop(
        &mut self,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(object_id, ImmediateCommandResume::MoveToAfterStop)
    }

    /// Resume MoveTo after FlightControl's ordinary SetActionByName("Fly")
    /// and callbacks. A WALK origin continues into JumpControl; DFA_FLIGHT
    /// only consumes the retained callback boundary.
    pub(crate) fn resume_move_to_after_flight(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::MoveToAfterFlight(command_instance_id),
        )
    }

    /// Resume Build after its Dig arm's live ObjectComStop without consuming
    /// another command interval or frame.
    pub(crate) fn resume_build_after_stop(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::BuildAfterStop(command_instance_id),
        )
    }

    pub(crate) fn resume_exit_after_stop(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::ExitAfterStop(command_instance_id),
        )
    }

    pub(crate) fn resume_throw_after_prelude(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::ThrowPrelude(command_instance_id),
        )
    }

    pub(crate) fn resume_drop_after_prelude(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::DropPrelude(command_instance_id),
        )
    }

    pub(crate) fn resume_put_after_stop(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::PutAfterStop(command_instance_id),
        )
    }

    pub(crate) fn resume_construct_after_stop(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::ConstructAfterStop(command_instance_id),
        )
    }

    pub(crate) fn resume_construct_after_script(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
        result: AcquireScriptResult,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::ConstructScript {
                command_instance_id,
                result,
            },
        )
    }

    /// Finish Construct after CreateObjectConstruction and the conkit's
    /// AssignRemoval have both completed. This is still the same native
    /// Execute invocation: Finish(true) and AddCommand(Build) happen before
    /// the event handler returns.
    pub(crate) fn resume_construct_after_spawn(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
        construction_id: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::ConstructSpawn {
                command_instance_id,
                construction_id,
            },
        )
    }

    /// Resume the exact command body after its first callbackful FairCrew
    /// physical read. This remains the same native `C4Command::Execute`
    /// invocation, so its update interval and InitEvaluation gates must not
    /// run a second time.
    pub(crate) fn resume_command_after_physical(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
        physical: PhysicalInfo,
    ) -> Result<bool, EngineError> {
        self.execute_object_command_now_inner(
            object_id,
            ImmediateCommandResume::Physical {
                command_instance_id,
                physical,
            },
        )
    }

    fn execute_object_command_now_inner(
        &mut self,
        object_id: ObjectId,
        resume: ImmediateCommandResume,
    ) -> Result<bool, EngineError> {
        let Some(initial_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let physical_deferred = !matches!(resume, ImmediateCommandResume::Physical { .. })
            && self.object_physical_will_fill_fair_cache(initial_index);

        // Structural command snapshots never trigger FairCrew promotion.
        // The exact handler branch that reaches GetPhysical emits a live
        // continuation event; that event rebuilds this table after the hook.
        let mut command_snapshots = CommandObjectSnapshots::with_capacity_and_hasher(
            self.objects.len(),
            Default::default(),
        );
        for index in 0..self.objects.len() {
            let id = self.objects[index].id;
            command_snapshots.insert(id, self.live_command_snapshot(index));
        }
        if let Some(snapshot) = command_snapshots.get_mut(&object_id) {
            match resume {
                ImmediateCommandResume::Physical { physical, .. } => {
                    // Keep the exact pointer value returned by the final
                    // native GetPhysical call even if its hook changed Def,
                    // Info, or the global FairCrew controls afterward.
                    snapshot.physical = physical;
                    snapshot.physical_deferred = false;
                }
                _ => snapshot.physical_deferred = physical_deferred,
            }
        }
        let player_snapshots = self
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
            .collect::<HashMap<_, _>>();
        let definition_snapshots = self.command_definition_snapshot_table();
        let transfer_zones = self.transfer_zones.clone();
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let Some(object_snapshot) = command_snapshots.get(&object_id) else {
            return Ok(false);
        };
        let command_rng = std::cell::RefCell::new(std::mem::take(&mut self.rng));
        let command_context = CommandRuntimeContext {
            rng: Some(&command_rng),
            frame: self.frame,
            position: object_snapshot.position,
            landscape: self.landscape.as_ref(),
            object: object_snapshot,
            objects: &command_snapshots,
            players: &player_snapshots,
            definitions: definition_snapshots.as_ref(),
            structures_need_energy: self.structures_need_energy,
            base_buy_enabled: self.base_buy_enabled,
            base_sell_enabled: self.base_sell_enabled,
            transfer_zones: &transfer_zones,
        };
        let command_gravity = self.physics.gravity_as_c4fixed();
        let result = match resume {
            ImmediateCommandResume::Front => {
                self.objects[index].step_command_stack(command_context, command_gravity)
            }
            ImmediateCommandResume::MoveToAfterStop => self.objects[index]
                .commands
                .execute_pending_move_to_stop(&command_context),
            ImmediateCommandResume::MoveToAfterFlight(command_instance_id) => self.objects[index]
                .commands
                .execute_pending_move_to_flight(&command_context, command_instance_id),
            ImmediateCommandResume::BuildAfterStop(command_instance_id) => self.objects[index]
                .commands
                .execute_pending_build_stop(&command_context, command_instance_id),
            ImmediateCommandResume::ExitAfterStop(command_instance_id) => self.objects[index]
                .commands
                .execute_pending_exit_stop(&command_context, command_instance_id),
            ImmediateCommandResume::ThrowPrelude(command_instance_id) => {
                self.objects[index].commands.execute_pending_throw_prelude(
                    &command_context,
                    command_gravity,
                    command_instance_id,
                )
            }
            ImmediateCommandResume::DropPrelude(command_instance_id) => self.objects[index]
                .commands
                .execute_pending_drop_prelude(&command_context, command_instance_id),
            ImmediateCommandResume::PutAfterStop(command_instance_id) => self.objects[index]
                .commands
                .execute_pending_put_stop(&command_context, command_gravity, command_instance_id),
            ImmediateCommandResume::ConstructAfterStop(command_instance_id) => self.objects[index]
                .commands
                .execute_pending_construct_stop(&command_context, command_instance_id),
            ImmediateCommandResume::ConstructScript {
                command_instance_id,
                result,
            } => self.objects[index]
                .commands
                .execute_pending_construct_script(&command_context, command_instance_id, result),
            ImmediateCommandResume::ConstructSpawn {
                command_instance_id,
                construction_id,
            } => self.objects[index]
                .commands
                .execute_pending_construct_spawn(
                    &command_context,
                    command_instance_id,
                    construction_id,
                ),
            ImmediateCommandResume::Physical {
                command_instance_id,
                physical,
            } => self.objects[index].commands.execute_pending_physical(
                &command_context,
                command_gravity,
                command_instance_id,
                physical,
            ),
        };
        self.rng = command_rng.into_inner();

        let mut resolved_command_physical = false;
        if let Some(result) = result {
            if let Some(update) = result.update {
                self.apply_object_update(object_id, update)?;
            }
            for event in result.events {
                resolved_command_physical |= self.apply_command_event(event)?;
            }
        }
        self.finish_object_command_execution(object_id)?;
        Ok(resolved_command_physical)
    }

    /// Advance one simulation frame and return its full presentation snapshot.
    pub fn tick(&mut self) -> Result<SimulationSnapshot, EngineError> {
        self.advance_tick()?;
        let mut snapshot = self.snapshot();
        let presentation = self.drain_tick_presentation();
        snapshot.hud.scoreboard_presentations = presentation.scoreboard_presentations;
        snapshot.menu_requests = presentation.menu_requests;
        snapshot.audio = presentation.audio;
        Ok(snapshot)
    }

    /// Advance one simulation frame and return only its transient presentation
    /// requests, without constructing the frame's full [`SimulationSnapshot`].
    ///
    /// Simulation and presentation-queue semantics are identical to
    /// [`Engine::tick`].
    pub fn tick_with_presentation(&mut self) -> Result<TickPresentation, EngineError> {
        self.advance_tick()?;
        Ok(self.drain_tick_presentation())
    }

    /// Advance one simulation frame without constructing its presentation
    /// snapshot. Presentation requests are still consumed and audio registry
    /// state is still updated exactly as if the caller had discarded the
    /// value returned by [`Engine::tick`].
    pub fn tick_without_snapshot(&mut self) -> Result<(), EngineError> {
        self.tick_with_presentation()?;
        Ok(())
    }
}
