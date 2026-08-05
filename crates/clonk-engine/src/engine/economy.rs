//! `impl Engine` — buy/sell, base rules, fair crew, ChangeDef, weather, sync.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;
use crate::math::fixtof;
use crate::particles::ObjectFireEmission;

impl Engine {
    pub(crate) fn call_command_buy_value(
        &mut self,
        actor_id: ObjectId,
        item_definition: &str,
        base_id: ObjectId,
        buyer: i32,
    ) -> Result<Option<i32>, EngineError> {
        let idx = self
            .find_object_index(actor_id)
            .ok_or(EngineError::UnknownObject(actor_id))?;
        let (definition_id, state_snapshot) = {
            let object = &self.objects[idx];
            (
                object.definition_id.clone(),
                Rc::new(object.script_state_snapshot()),
            )
        };
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition.action_library().clone();
        let world =
            self.host_world_context_for_object_with_snapshot(idx, Rc::clone(&state_snapshot));
        let call = definition.command_buy_value(
            state_snapshot.as_ref(),
            actor_id,
            item_definition,
            base_id,
            buyer,
            self.rng.clone(),
            &self.global_effects.clone(),
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (price, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            Err(error) => {
                return Err(self.apply_script_error_recovery(
                    error,
                    idx,
                    &action_library,
                    actor_id,
                    &definition_id,
                    false,
                ));
            }
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_callback_outcome(
            idx,
            outcome,
            &action_library,
            actor_id,
            &definition_id,
            false,
        )?;
        Ok(price)
    }

    pub(crate) fn call_command_buy_item(
        &mut self,
        actor_id: ObjectId,
        item_definition: &str,
        buyer: i32,
        payer: i32,
        base_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let idx = self
            .find_object_index(actor_id)
            .ok_or(EngineError::UnknownObject(actor_id))?;
        let (definition_id, state_snapshot) = {
            let object = &self.objects[idx];
            (
                object.definition_id.clone(),
                Rc::new(object.script_state_snapshot()),
            )
        };
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition.action_library().clone();
        let world =
            self.host_world_context_for_object_with_snapshot(idx, Rc::clone(&state_snapshot));
        let call = definition.command_buy_item(
            state_snapshot.as_ref(),
            actor_id,
            item_definition,
            buyer,
            payer,
            base_id,
            self.rng.clone(),
            &self.global_effects.clone(),
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (bought, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            Err(error) => {
                return Err(self.apply_script_error_recovery(
                    error,
                    idx,
                    &action_library,
                    actor_id,
                    &definition_id,
                    false,
                ));
            }
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_callback_outcome(
            idx,
            outcome,
            &action_library,
            actor_id,
            &definition_id,
            false,
        )?;
        Ok(bought)
    }

    pub(crate) fn sell_object_to_home(
        &mut self,
        context_object: ObjectId,
        sold_object: ObjectId,
        base_owner: i32,
    ) -> Result<bool, EngineError> {
        let idx = self
            .find_object_index(context_object)
            .ok_or(EngineError::UnknownObject(context_object))?;
        let (definition_id, state_snapshot) = {
            let object = &self.objects[idx];
            (
                object.definition_id.clone(),
                Rc::new(object.script_state_snapshot()),
            )
        };
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition.action_library().clone();
        let world =
            self.host_world_context_for_object_with_snapshot(idx, Rc::clone(&state_snapshot));
        let call = definition.command_sell_item(
            state_snapshot.as_ref(),
            context_object,
            sold_object,
            base_owner,
            self.rng.clone(),
            &self.global_effects.clone(),
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (sold, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            Err(error) => {
                return Err(self.apply_script_error_recovery(
                    error,
                    idx,
                    &action_library,
                    context_object,
                    &definition_id,
                    false,
                ));
            }
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_callback_outcome(
            idx,
            outcome,
            &action_library,
            context_object,
            &definition_id,
            false,
        )?;
        Ok(sold)
    }

    /// `C4Object::Incinerate` (C4Object.cpp:1257-1268): construct the
    /// priority-100, interval-1 Fire effect through the same live C4Effect
    /// path as AddEffect. That path runs the higher/equal-priority Fx*Effect
    /// check chain, annul/merge and temporary upper-effect calls, then lets a
    /// global script FxFireStart override the native engine start.
    #[doc(hidden)]
    pub fn incinerate_object(
        &mut self,
        idx: usize,
        caused_by: i32,
        blasted: bool,
        incinerating: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        let (object_id, definition_id, state_snapshot) = {
            let object = self
                .objects
                .get(idx)
                .ok_or_else(|| EngineError::UnknownObject(ObjectId::new(u64::MAX)))?;
            (
                object.id,
                object.definition_id.clone(),
                Rc::new(object.script_state_snapshot()),
            )
        };
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition.action_library().clone();
        let world =
            self.host_world_context_for_object_with_snapshot(idx, Rc::clone(&state_snapshot));
        let call = definition.incinerate_object(
            state_snapshot.as_ref(),
            object_id,
            caused_by,
            blasted,
            incinerating,
            self.rng.clone(),
            &self.global_effects.clone(),
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (incinerated, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            Err(error) => {
                return Err(self.apply_script_error_recovery(
                    error,
                    idx,
                    &action_library,
                    object_id,
                    &definition_id,
                    false,
                ));
            }
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_callback_outcome(
            idx,
            outcome,
            &action_library,
            object_id,
            &definition_id,
            false,
        )?;
        Ok(incinerated)
    }

    /// `C4Object::ExecFire` (C4Object.cpp:766-810), run by the fire
    /// effect's timer (FnFxFireTimer, C4Effect.cpp:643-771), followed by
    /// that function's particle emitter. Returns the deferred Fx*Stop
    /// events of effects an extinguish killed. Still open: SmokeRate smoke
    /// (visual), and death/removal callbacks from the energy and damage
    /// changes.
    #[doc(hidden)]
    pub fn exec_object_fire(
        &mut self,
        idx: usize,
        frame: u64,
        fire_number: i32,
    ) -> Vec<EffectEvent> {
        if !self.objects[idx].state.on_fire {
            return Vec::new();
        }
        // C++ takes iTime as an argument, fixed for the whole call, so read
        // the effect's own clock before ExecFire can disturb the list.
        let effect_time = self.object_fire_effect_var(idx, fire_number, |effect| effect.timer);
        let mut stop_events = Vec::new();
        // FnFxFireTimer reads Var(1), validates only the local copy and
        // passes NO_OWNER into ExecFire when that player no longer exists.
        // The stored effect value remains available to scripts unchanged.
        let stored_caused_by = self.objects[idx]
            .state
            .effects
            .iter()
            .find(|effect| effect.number == fire_number)
            .and_then(|effect| effect.vars.get(1))
            .map(|value| match value {
                EffectVarValue::Int(value) => *value,
                EffectVarValue::Bool(value) => i32::from(*value),
                EffectVarValue::RawBool(value) => *value as u32 as i32,
                _ => 0,
            })
            .unwrap_or(self.objects[idx].state.fire_caused_by);
        let caused_by = if self.players.contains_key(&stored_caused_by) {
            stored_caused_by
        } else {
            OWNER_NONE
        };
        // Fire Phase (C4Object.cpp:769)
        {
            let object = &mut self.objects[idx];
            object.state.fire_phase = (object.state.fire_phase + 1) % MAX_FIRE_PHASE;
        }
        // Tick5 base extinguish precedes decay/damage/energy and does not
        // stop the rest of this ExecFire call (C4Object.cpp:772-777). The
        // direct container's Base only needs to name a currently linked
        // player; ownership, hostility and the burning object's Alive flag
        // are irrelevant.
        if frame.is_multiple_of(5)
            && self.base_extinguish_enabled
            && self.objects[idx].state.category & CATEGORY_LIVING != 0
        {
            let valid_container_base = self.objects[idx]
                .state
                .container
                .and_then(|container| self.find_object_index(container))
                .map(|container_idx| self.objects[container_idx].state.base)
                .is_some_and(|base| self.players.contains_key(&base));
            if valid_container_base {
                let (_, events) = self.extinguish_object(idx, fire_number);
                stop_events.extend(events);
            }
        }
        let (no_burn_decay, no_burn_damage) = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .map(|definition| (definition.no_burn_decay(), definition.no_burn_damage()))
            .unwrap_or((false, false));
        // Decay: DoCon(-100) every frame (C4Object.cpp:776-778); burned away
        // at zero construction (C4Object::DoCon removal)
        if !no_burn_decay {
            if let Err(error) = self.do_con(idx, -100) {
                tracing::error!(%error, "fire DoCon callback failed; continuing");
                crate::object::log_engine_error_call_frames(&error);
            }
        }
        // Damage: Tick10 DoDamage(+2) by fire (C4Object.cpp:780)
        if frame.is_multiple_of(10) && !no_burn_damage {
            if let Err(error) = self.change_object_damage(idx, 2, C4FX_CALL_DMG_FIRE, caused_by) {
                tracing::error!(%error, "fire damage callback failed; continuing");
                crate::object::log_engine_error_call_frames(&error);
            }
        }
        // Energy: Tick5 DoEnergy(-1) (C4Object.cpp:782)
        if frame.is_multiple_of(5) {
            if let Err(error) = self.change_object_energy(idx, -1, C4FX_CALL_ENG_FIRE, caused_by) {
                tracing::error!(%error, "fire energy callback failed; continuing");
                crate::object::log_engine_error_call_frames(&error);
            }
        }
        // Effects: SmokeRate smoke (C4Object.cpp:785-793). The cadence is
        // fully deterministic — frame, object number and def fields — so only
        // the particle it spawns is presentation.
        self.exec_object_fire_smoke(idx, frame);
        // Background effects: Tick5 over valid landscape material
        // (C4Object.cpp:791-806) — extinguish in extinguisher material, then
        // the unconditional Random(3) landscape-inflame draw.
        if frame.is_multiple_of(5) {
            let position = self.objects[idx].state.position;
            let material = self
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(position.x, position.y));
            if let Some(material_id) = material {
                let extinguisher = self
                    .materials
                    .get_by_id(material_id)
                    .map(|material| material.extinguisher() != 0)
                    .unwrap_or(false);
                if extinguisher {
                    // Extinguish(iFireNumber) kills THIS fire effect
                    // (C4Object.cpp:799-801); the Pshshsh sound is
                    // presentation-only.
                    let (_, events) = self.extinguish_object(idx, fire_number);
                    stop_events.extend(events);
                }
                // Inflame (C4Object.cpp:803-804)
                if self.rng.random(3) == 0 {
                    let _ = self.spawn_fire_at(position.x, position.y);
                }
            }
        }
        // FnFxFireTimer returns C4Fx_Execute_Kill once the flag is gone
        // (C4Effect.cpp:663-666) — belt and braces when something cleared
        // OnFire without killing the effect. C++ checks it before the
        // emitter, so a fire that just went out draws no particles.
        if !self.objects[idx].state.on_fire {
            if let Some(removed) = self.objects[idx].remove_effect_by_number(fire_number) {
                stop_events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
            }
            return stop_events;
        }
        self.exec_object_fire_particles(idx, fire_number, effect_time);
        stop_events
    }

    /// `C4Object::ExecFire`'s "Effects" arm (C4Object.cpp:785-793): a burning
    /// object trails smoke on a period derived from its live shape width and
    /// the definition's `SmokeRate`, or on every execution once it is moving
    /// faster than two pixels a frame. `Number * 7` staggers the phase so a
    /// row of identical burning objects does not puff in lockstep.
    fn exec_object_fire_smoke(&mut self, idx: usize, frame: u64) {
        let object = &self.objects[idx];
        let smoke_rate = self
            .definitions
            .get(&object.definition_id)
            .map_or(0, Definition::smoke_rate);
        // `if (smoke_rate)` — SmokeRate=0 opts out, and guards the divide.
        if smoke_rate == 0 {
            return;
        }
        let shape_width = object.current_shape_rect().unwrap_or_default().width;
        let smoke_level = 2 * shape_width / 3;
        let period = (50 * smoke_level / smoke_rate).max(3);
        let phase = (frame as i32).wrapping_add((object.id.as_u64() as i32).wrapping_mul(7));
        // `Abs(xdir) > 2` compares the raw fixed velocity against itofix(2)
        // (Fixed.h:185, the int32 spaceship wrapper).
        let fast = object.fixed_velocity.x.abs() > crate::math::itofix(2);
        // `%` matches C's truncated remainder; `period >= 3` keeps it safe.
        if phase % period == 0 || fast {
            let position = object.state.position;
            self.spawn_smoke(position.x, position.y, smoke_level);
        }
    }

    /// One `Fire` effect variable, read through the C4Value int conversion
    /// FnFxFireTimer applies to `FxFireVar*`.
    fn object_fire_effect_var(
        &self,
        idx: usize,
        fire_number: i32,
        pick: impl Fn(&EffectState) -> i32,
    ) -> i32 {
        self.objects[idx]
            .state
            .effects
            .iter()
            .find(|effect| effect.number == fire_number)
            .map(pick)
            .unwrap_or(0)
    }

    /// The particle half of `FnFxFireTimer` (C4Effect.cpp:660-769): the
    /// three gates, then a snapshot of the burning object handed to the
    /// particle system, which owns the unsynchronized `SafeRandom` stream
    /// the emitter draws from.
    fn exec_object_fire_particles(&mut self, idx: usize, fire_number: i32, effect_time: i32) {
        // special effects only if loaded (C4Effect.cpp:660-661)
        if !self.particle_system.is_fire_particle_loaded() {
            return;
        }
        // get fire mode (C4Effect.cpp:670-671); an unset EffectVar reads as
        // zero, which is no mode at all and takes the normal-fire arms.
        let fire_mode =
            self.object_fire_effect_var(idx, fire_number, |effect| match effect.vars.first() {
                Some(EffectVarValue::Int(value)) => *value,
                Some(EffectVarValue::Bool(value)) => i32::from(*value),
                Some(EffectVarValue::RawBool(value)) => *value as u32 as i32,
                _ => 0,
            });
        // special effects only each four frames, except for objects
        // (C4Effect.cpp:673-674) — the effect's own clock, not the frame.
        if effect_time % 4 != 0 && fire_mode != C4FX_FIRE_MODE_OBJECT {
            return;
        }
        // no gfx for contained (C4Effect.cpp:676-677)
        if self.objects[idx].state.container.is_some() {
            return;
        }
        let object = &self.objects[idx];
        let definition = self.definitions.get(&object.definition_id);
        let def_shape = object.shape_template.rect.unwrap_or_default();
        let shape = object.current_shape_rect().unwrap_or(def_shape);
        let emission = ObjectFireEmission {
            object: object.id,
            fire_mode,
            def_width: def_shape.width,
            def_height: def_shape.height,
            fire_top: definition.map_or(0, Definition::fire_top),
            con: object.state.construction,
            growth_type: definition.is_some_and(Definition::stretch_growth),
            x: object.state.position.x,
            y: object.state.position.y,
            shape_x: shape.x,
            shape_y: shape.y,
            shape_width: shape.width,
            shape_height: shape.height,
            rotation: object.state.rotation,
            rotateable: definition.is_some_and(|definition| definition.rotateable() != 0),
            xdir: fixtof(object.fixed_velocity.x),
            ydir: fixtof(object.fixed_velocity.y),
        };
        self.particle_system.create_object_fire(&emission);
    }

    /// `C4Object::Extinguish` (C4Object.cpp:1269-1301): a known fire number
    /// kills exactly that effect; zero kills every "*Fire*" effect while
    /// skipping engine-internal "Int*" names (C4Fx_AnyFire/C4Fx_Internal,
    /// C4Effects.h:154-155). The engine-internal FnFxFireStop clears the
    /// OnFire flag (C4Effect.cpp:787); the returned Stopped events carry
    /// the deferred Fx*Stop dispatch for script-visible effects.
    fn extinguish_object(&mut self, idx: usize, fire_number: i32) -> (bool, Vec<EffectEvent>) {
        let mut events = Vec::new();
        let mut killed = 0usize;
        loop {
            let target = self.objects[idx].state.effects.iter().find_map(|effect| {
                if fire_number != 0 {
                    (effect.number == fire_number).then_some(effect.number)
                } else {
                    (effect.name.contains("Fire") && !effect.name.starts_with("Int"))
                        .then_some(effect.number)
                }
            });
            let Some(number) = target else { break };
            if let Some(removed) = self.objects[idx].remove_effect_by_number(number) {
                killed += 1;
                if removed.name == C4FX_FIRE {
                    // engine FnFxFireStop (C4Effect.cpp:787)
                    self.objects[idx].state.on_fire = false;
                }
                events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
            }
            if fire_number != 0 {
                break;
            }
        }
        (killed > 0, events)
    }

    /// Whether `Fx<Name><Event>` resolves to a SCRIPT function for this
    /// effect (C4Effect::AssignCallbackFunctions, C4Effect.cpp:30-56:
    /// command-target script, command-id def script, else the global
    /// table; GetFuncRecursive finds script globals before the
    /// engine-registered C++ functions).
    pub(crate) fn effect_has_script_callback(
        &self,
        effect: &EffectState,
        _fallback_definition_id: &DefinitionId,
        event: &str,
    ) -> bool {
        let callback_name = format!("Fx{}{event}", effect.name);
        resolve_effect_script_callback(effect, &callback_name, &self.host_world_context()).is_some()
    }

    /// `C4Object::ExecLife` breathing block (C4Object.cpp:878-919), run
    /// after the fire effect like C++ (C4Object.cpp:1074-1080). Still open:
    /// the FXB1 bubble object (the synced `Random(5)` x-argument draw IS
    /// consumed), the DeepBreath callback's sound, and the corrosion/
    /// Complete non-initial `C4Object::DoCon` side effects
    /// (C4Object.cpp:1428-1516).
    pub(crate) fn do_con(&mut self, idx: usize, change: i32) -> Result<(), EngineError> {
        let Some(object_id) = self.objects.get(idx).map(|object| object.id) else {
            return Ok(());
        };
        let definition = self.definitions.get(&self.objects[idx].definition_id);
        let oversize = definition.is_some_and(Definition::oversize);
        let definition_components = definition
            .map(|definition| {
                definition
                    .components()
                    .iter()
                    .map(|component| (component.id.clone(), component.count))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let before = self.objects[idx].state.construction;
        let mut after = before.saturating_add(change).max(0);
        if !oversize {
            after = after.min(FULL_CON);
        }
        let previous_step = before / (FULL_CON / 100);
        let step_diff = after / (FULL_CON / 100) - previous_step;
        let was_full = before >= FULL_CON;
        let refresh = docon_refreshes_construction(before, after);
        let entry_y = self.objects[idx].state.position.y;
        let entry_shape = self.objects[idx].current_shape_rect();

        if refresh {
            self.objects[idx].compiled_mass = None;
        }
        self.objects[idx].state.construction = after;
        self.refresh_object_ocf(idx);
        if !refresh {
            return Ok(());
        }

        // A full solid mask is removed before UpdateFace mutates the shape.
        if was_full && after < FULL_CON {
            self.remove_solid_mask(idx);
        }
        let components = docon_component_counts(
            &self.objects[idx].state.components,
            &self.objects[idx].state.component_order,
            &definition_components,
            after,
            change,
        );
        self.objects[idx].state.components = components;
        if self.objects[idx].shape_template.line == 0 {
            // UpdateShape restores Def->Shape before applying construction;
            // an earlier SetShape override never survives DoCon.
            self.objects[idx].state.shape_override = None;
        }
        self.objects[idx].refresh_shape_geometry();
        self.update_sector_for_index(idx);
        self.update_solid_mask(idx);

        if after < FULL_CON {
            let incomplete_activity = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .is_some_and(Definition::incomplete_activity);
            if !incomplete_activity {
                while let Some(parent_index) = self.find_object_index(object_id) {
                    let Some(child) = self.objects[parent_index]
                        .state
                        .contents
                        .iter()
                        .copied()
                        .find(|child| {
                            self.find_object_index(*child)
                                .is_some_and(|index| self.objects[index].has_nonzero_status())
                        })
                    else {
                        break;
                    };
                    let destination = self.objects[parent_index].state.container;
                    let moved = if let Some(destination) = destination {
                        self.try_object_enter(child, destination)?
                    } else {
                        self.exit_object_at_current_position(child)?
                    };
                    if !moved {
                        // Exit/Enter callbacks may have removed or moved the
                        // head even when the requested transfer reported
                        // false. C++ re-reads Contents.GetObject() each pass.
                        let same_head =
                            self.find_object_index(object_id).and_then(|parent_index| {
                                self.objects[parent_index]
                                    .state
                                    .contents
                                    .iter()
                                    .copied()
                                    .find(|candidate| {
                                        self.find_object_index(*candidate).is_some_and(|index| {
                                            self.objects[index].has_nonzero_status()
                                        })
                                    })
                            }) == Some(child);
                        if same_head {
                            break;
                        }
                    }
                }
            }
            if let Some(parent_index) = self.find_object_index(object_id) {
                self.objects[parent_index].state.need_energy = false;
            }
        }

        if was_full {
            let idle = self.find_object_index(object_id).is_some_and(|index| {
                let object = &self.objects[index];
                object.state.construction < FULL_CON
                    && self
                        .definitions
                        .get(&object.definition_id)
                        .is_none_or(|definition| !definition.incomplete_activity())
            });
            if idle {
                if let Some(index) = self.find_object_index(object_id) {
                    let definition_id = self.objects[index].definition_id.clone();
                    let _ = tolerate_script_error(self.action_with_calls(
                        index,
                        &definition_id,
                        "Idle",
                    ))?;
                }
            }
        }

        // Ejection and action callbacks precede this position adjustment.
        if let Some(index) = self.find_object_index(object_id) {
            let definition_height = self
                .definitions
                .get(&self.objects[index].definition_id)
                .and_then(Definition::shape_rect)
                .map_or(0, |shape| shape.height);
            let current_position = self.objects[index].state.position;
            let current_shape = self.objects[index].current_shape_rect();
            let adjusted_y = docon_adjusted_position_y(
                entry_y,
                entry_shape,
                current_position.y,
                current_shape,
                self.objects[index].state.rotation,
                self.objects[index].state.category,
                previous_step,
                step_diff,
                definition_height,
            );
            if adjusted_y != current_position.y {
                // UpdatePos changes only integer coordinates; fixed_position
                // remains the value from before DoCon.
                self.objects[index].state.position.y = adjusted_y;
                self.update_sector_for_index(index);
                self.update_solid_mask(index);
            }
        }

        let crossed_full = !was_full
            && self
                .find_object_index(object_id)
                .is_some_and(|index| self.objects[index].state.construction >= FULL_CON);
        if crossed_full {
            if let Some(index) = self.find_object_index(object_id) {
                let _ = tolerate_script_error(self.call_object_function(
                    index,
                    "Completion",
                    Vec::new(),
                ))?;
            }
            if let Some(index) = self.find_object_index(object_id).filter(|&index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != ObjectStatus::Deleted
            }) {
                let _ = tolerate_script_error(self.call_object_function(
                    index,
                    "Initialize",
                    Vec::new(),
                ))?;
            }
        }

        if self
            .find_object_index(object_id)
            .is_some_and(|index| self.objects[index].state.construction <= 0)
        {
            let _ = self.assign_object_removal(object_id)?;
        }
        Ok(())
    }

    /// `C4Object::BuyEnergy` (C4Object.cpp:814-823): buy one hundred
    /// percent-points for the base object, charging its assigned player.
    fn buy_object_energy(&mut self, idx: usize) -> Result<bool, EngineError> {
        let base_player = self.objects[idx].state.base;
        // Native captures pPlr and returns before GetPhysical when Base does
        // not resolve (C4Object.cpp:816-817).
        if !self.players.contains_key(&base_player) {
            return Ok(false);
        }
        if self.object_physical(idx).energy == 0 {
            return Ok(false);
        }
        let Some(player) = self.players.get(&base_player) else {
            return Ok(false);
        };
        if matches!(
            player.status(),
            PlayerStatus::Eliminated | PlayerStatus::Surrendered
        ) || player.wealth() < self.base_regenerate_energy_price
        {
            return Ok(false);
        }
        self.adjust_player_wealth(base_player, -self.base_regenerate_energy_price)?;
        // GetFairCrewPhysical may have changed the object's owner. Native
        // evaluates this argument only when it reaches DoEnergy (:821).
        let owner = self.objects[idx].state.owner;
        self.change_object_energy(idx, 100, C4FX_CALL_ENG_BASE_REFRESH, owner)?;
        Ok(true)
    }

    /// All periodic arms of `C4Object::ExecLife` in native order
    /// (C4Object.cpp:825-967).
    pub(crate) fn exec_object_life(&mut self, idx: usize, frame: u64) -> Result<(), EngineError> {
        // Growth (C4Object.cpp:824-837): every Tick35, Def Growth on an
        // incomplete, unburning alive Living or StaticBack gains
        // DoCon(Growth*100).
        if frame.is_multiple_of(35) {
            let object = &self.objects[idx];
            let category = object.state.category;
            let eligible = !object.state.on_fire
                && ((category & CATEGORY_LIVING != 0 && object.state.alive)
                    || category & CATEGORY_STATIC_BACK != 0);
            let growth = self
                .definitions
                .get(&object.definition_id)
                .map(|definition| definition.growth())
                .unwrap_or(0);
            if eligible && growth != 0 && object.state.construction < FULL_CON {
                self.do_con(idx, growth * 100)?;
            }
        }

        // Energy reload in a friendly assigned base (C4Object.cpp:839-856).
        if frame.is_multiple_of(3) && self.objects[idx].state.alive {
            let recipient_owner = self.objects[idx].state.owner;
            let recipient_energy = self.objects[idx].state.energy;
            let eligible_container = self.objects[idx].state.container.and_then(|container_id| {
                let container_idx = self.find_object_index(container_id)?;
                let base_player = self.objects[container_idx].state.base;
                (self.players.contains_key(&base_player)
                    && !self.players_hostile(recipient_owner, base_player))
                .then_some(container_id)
            });
            // C++ reaches GetPhysical only after Contained, ValidPlr and
            // Hostile have all passed (:843-846).
            if eligible_container.is_some() {
                let recipient_max = self.object_physical(idx).energy;
                if recipient_energy < recipient_max && self.base_regenerate_energy_enabled {
                    if let Some(container_idx) = self.objects[idx]
                        .state
                        .container
                        .and_then(|container_id| self.find_object_index(container_id))
                    {
                        if self.objects[container_idx].state.energy <= 0 {
                            let _ = self.buy_object_energy(container_idx)?;
                        }
                        // BuyEnergy's DoEnergy callbacks can mutate either
                        // object's physicals or even its Contained pointer.
                        // C++ re-evaluates both before calculating transfer.
                        if let Some(current_container_idx) = self.objects[idx]
                            .state
                            .container
                            .and_then(|current| self.find_object_index(current))
                        {
                            let recipient_max_after_buy = self.object_physical(idx).energy;
                            let transfer = (2 * C4_MAX_PHYSICAL / 100)
                                .min(self.objects[current_container_idx].state.energy)
                                .min(recipient_max_after_buy - self.objects[idx].state.energy);
                            if transfer != 0 {
                                // The second GetPhysical may have changed
                                // Contained. Native dereferences the live
                                // pointer again for the donor DoEnergy call.
                                if let Some(debit_container_idx) = self.objects[idx]
                                    .state
                                    .container
                                    .and_then(|current| self.find_object_index(current))
                                {
                                    let debit_caused_by =
                                        self.objects[debit_container_idx].state.owner;
                                    self.change_object_energy_exact(
                                        debit_container_idx,
                                        -transfer,
                                        C4FX_CALL_ENG_BASE_REFRESH,
                                        debit_caused_by,
                                    )?;
                                    // `Contained->Owner` is evaluated again
                                    // for the second call after donor damage
                                    // callbacks.
                                    let credit_caused_by = self.objects[idx]
                                        .state
                                        .container
                                        .and_then(|current| self.find_object_index(current))
                                        .map(|current_idx| self.objects[current_idx].state.owner)
                                        .unwrap_or(OWNER_NONE);
                                    self.change_object_energy_exact(
                                        idx,
                                        transfer,
                                        C4FX_CALL_ENG_BASE_REFRESH,
                                        credit_caused_by,
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Magic reload uses the ENGINE-global DoMagicEnergy overload so the
        // No-Magic-Energy rule can veto the debit (C4Object.cpp:858-878).
        if frame.is_multiple_of(3) && self.objects[idx].state.alive {
            let recipient_owner = self.objects[idx].state.owner;
            let recipient_magic = self.objects[idx].state.magic_energy;
            let eligible_container = self.objects[idx].state.container.and_then(|container_id| {
                let container_idx = self.find_object_index(container_id)?;
                let container_owner = self.objects[container_idx].state.owner;
                (!self.players_hostile(recipient_owner, container_owner)).then_some(container_id)
            });
            // Native's Contained/Hostile gates precede GetPhysical()->Magic
            // (C4Object.cpp:859-862).
            if eligible_container.is_some() {
                let recipient_max = self.object_physical(idx).magic;
                if recipient_magic < recipient_max {
                    if let Some(container_idx) = self.objects[idx]
                        .state
                        .container
                        .and_then(|current| self.find_object_index(current))
                    {
                        const MAGIC_PHYSICAL_FACTOR: i32 = 1000;
                        // Native performs a second GetPhysical in the transfer
                        // expression and rereads both live energy words
                        // (C4Object.cpp:864).
                        let recipient_max_after_check = self.object_physical(idx).magic;
                        let transfer = (2 * MAGIC_PHYSICAL_FACTOR)
                            .min(self.objects[container_idx].state.magic_energy)
                            .min(recipient_max_after_check - self.objects[idx].state.magic_energy)
                            / MAGIC_PHYSICAL_FACTOR;
                        if transfer != 0 {
                            // The second GetPhysical may have changed
                            // Contained. Native resolves the donor again for
                            // the DoMagicEnergy debit.
                            if let Some(debit_container_id) =
                                self.objects[idx].state.container.and_then(|current| {
                                    self.find_object_index(current).map(|_| current)
                                })
                            {
                                let debited = self.call_engine_global_function(
                                    "DoMagicEnergy",
                                    &[
                                        Value::Int(-transfer),
                                        compat::object_reference_value(debit_container_id),
                                    ],
                                )?;
                                if compat::value_raw_truthy(&debited) {
                                    let recipient_id = self.objects[idx].id;
                                    let _ = self.call_engine_global_function(
                                        "DoMagicEnergy",
                                        &[
                                            Value::Int(transfer),
                                            compat::object_reference_value(recipient_id),
                                        ],
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Breathing is one arm, not an early return: later Tick10/Tick255
        // arms must run for NoBreath and nonliving objects too.
        let no_breath = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .map(|definition| definition.no_breath())
            .unwrap_or(false);
        if frame.is_multiple_of(5) && self.objects[idx].state.alive && !no_breath {
            let position = self.objects[idx].state.position;
            let shape_top = self.objects[idx]
                .current_shape_rect()
                .map(|rect| rect.y)
                .unwrap_or(0);
            let mouth_y = position.y + shape_top / 2;
            let vehicle_at_mouth = self.materials.id_of("Vehicle").is_some_and(|vehicle| {
                self.landscape
                    .as_ref()
                    .and_then(|landscape| landscape.material_at(position.x, mouth_y))
                    == Some(vehicle)
            });
            let mut breathe = if vehicle_at_mouth {
                true
            } else if self.object_physical(idx).breathe_water != 0 {
                // GetFairCrewPhysical runs before this material read. Use the
                // live position it may have changed, as native does (:891-893).
                let position = self.objects[idx].state.position;
                let water = self.materials.id_of("Water");
                water.is_some()
                    && self
                        .landscape
                        .as_ref()
                        .and_then(|landscape| landscape.material_at(position.x, position.y))
                        == water
            } else {
                let position = self.objects[idx].state.position;
                let shape_top = self.objects[idx]
                    .current_shape_rect()
                    .map(|rect| rect.y)
                    .unwrap_or(0);
                let mouth_y = position.y + shape_top / 2;
                !self
                    .landscape
                    .as_ref()
                    .map(|landscape| {
                        landscape.is_solid_at(position.x, mouth_y)
                            || landscape.is_liquid_at(position.x, mouth_y)
                    })
                    .unwrap_or(false)
            };
            // Native checks containment after the complete vehicle /
            // BreatheWater / semisolid chain (C4Object.cpp:899).
            if self.objects[idx].state.container.is_some() {
                breathe = true;
            }
            if !breathe {
                if self.objects[idx].state.breath > 0 {
                    let breath = &mut self.objects[idx].state.breath;
                    *breath = (*breath - 2 * C4_MAX_PHYSICAL / 100).max(0);
                } else {
                    let cause = self.objects[idx].last_energy_loss_cause;
                    self.change_object_energy(idx, -1, C4FX_CALL_ENG_ASPHYXIATION, cause)?;
                }
                let bubble_dx = self.rng.random(5) - 2;
                let (bubble_x, bubble_y) = {
                    let state = &self.objects[idx].state;
                    let shape_top = self
                        .definitions
                        .get(&self.objects[idx].definition_id)
                        .and_then(|definition| definition.shape_rect())
                        .map(|rect| rect.y)
                        .unwrap_or(0);
                    (
                        state.position.x + bubble_dx,
                        state.position.y + shape_top / 2,
                    )
                };
                if let Err(error) = self.bubble_out(bubble_x, bubble_y) {
                    tracing::debug!(%error, "BubbleOut failed; continuing");
                }
                self.train_physical(idx, "Breath", 2, C4_MAX_PHYSICAL);
            } else {
                let max_breath = self.object_physical(idx).breath;
                let take = max_breath - self.objects[idx].state.breath;
                if take > self.object_physical(idx).breath / 2 {
                    let _ = tolerate_script_error(self.call_object_function(
                        idx,
                        "DeepBreath",
                        Vec::new(),
                    ))?;
                }
                self.objects[idx].state.breath += take;
            }
        }

        // Corrosion reads the cached (normally pre-movement) InMat.
        if frame.is_multiple_of(10) && self.objects[idx].state.alive {
            let corrosive = self.objects[idx]
                .in_mat
                .and_then(|material| self.materials.get_by_id(material))
                .map(|material| material.corrosive())
                .unwrap_or(0);
            if corrosive != 0 && self.object_physical(idx).corrosion_resist == 0 {
                let live_corrosive = self.objects[idx]
                    .in_mat
                    .and_then(|material| self.materials.get_by_id(material))
                    .map(|material| material.corrosive())
                    .unwrap_or(0);
                let caused_by = self.objects[idx].last_energy_loss_cause;
                self.change_object_energy(
                    idx,
                    live_corrosive.wrapping_neg() / 15,
                    C4FX_CALL_ENG_CORROSION,
                    caused_by,
                )?;
            }
        }

        // Lava/material fire has no Alive gate and ignores the magnitude of
        // either property (C4Object.cpp:932-938).
        if frame.is_multiple_of(10) {
            let incindiary = self.objects[idx]
                .in_mat
                .and_then(|material| self.materials.get_by_id(material))
                .map(|material| material.incindiary())
                .unwrap_or(0);
            let contact_incinerate = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .map(|definition| definition.contact_incinerate())
                .unwrap_or(0);
            if incindiary != 0 && contact_incinerate != 0 {
                let caused_by = self.objects[idx].last_energy_loss_cause;
                let _ = self.incinerate_object(idx, caused_by, false, None)?;
            }
        }

        // Ordinary energy on nonliving structures drains unless a valid base
        // assignment protects it or the definition is an EnergyHolder.
        if frame.is_multiple_of(10) && self.objects[idx].state.energy != 0 {
            let object = &self.objects[idx];
            let valid_base = self.players.contains_key(&object.state.base);
            let nonliving = object.state.category & CATEGORY_LIVING == 0;
            let energy_holder =
                self.definitions
                    .get(&object.definition_id)
                    .is_some_and(|definition| {
                        definition.line_connect() & LINE_CONNECT_ENERGY_HOLDER != 0
                    });
            if nonliving && (!valid_base || !self.base_regenerate_energy_enabled) && !energy_holder
            {
                self.change_object_energy(idx, -1, C4FX_CALL_ENG_STRUCT, OWNER_NONE)?;
            }
        }

        // Five-playing-hour birthday age cache and presentation.
        if frame.is_multiple_of(255) && self.objects[idx].state.alive {
            let object_id = self.objects[idx].id;
            let link = self.crew_info_links.get(&object_id).copied();
            let mut changed = None;
            if let Some(link) = link {
                if let Some(info) = self
                    .crew_rosters
                    .get_mut(&link.player_id)
                    .and_then(|roster| roster.get_mut(link.roster_index))
                {
                    let playing = info
                        .total_playing_time
                        .wrapping_add(self.game_time.wrapping_sub(info.in_action_time));
                    let new_age = playing / 3600 / 5;
                    if info.age != new_age {
                        changed = Some((info.name.clone(), new_age));
                    }
                    info.age = new_age;
                    if let Some(live) =
                        Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id)
                    {
                        live.age = new_age;
                        live.total_playing_time = info.total_playing_time;
                        live.in_action_time = info.in_action_time;
                    }
                }
            } else if let Some(info) = Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id)
            {
                let playing = info
                    .total_playing_time
                    .wrapping_add(self.game_time.wrapping_sub(info.in_action_time));
                let new_age = playing / 3600 / 5;
                if info.age != new_age {
                    changed = Some((info.name.clone(), new_age));
                }
                info.age = new_age;
            }
            if let Some((info_name, age)) = changed {
                let object_name = self.objects[idx]
                    .state
                    .custom_name
                    .clone()
                    .unwrap_or(info_name);
                self.messages.add_message(MessageSpec {
                    kind: message::MessageKind::Target,
                    text: format!("{object_name} becomes {age}!|Happy birthday!"),
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
        }
        Ok(())
    }

    /// `C4Object::AutoSellContents` + `C4Player::Sell2Home` for the base's
    /// direct contents and their first-level contents (C4Object.cpp:970-995;
    /// C4Player.cpp:865-897).
    pub(crate) fn auto_sell_base_contents(
        &mut self,
        base_index: usize,
        base_owner: i32,
    ) -> Result<(), EngineError> {
        if !self.player_can_sell_objects(base_owner) {
            return Ok(());
        }
        let contents = self.objects[base_index].state.contents.clone();
        for outer_id in contents {
            let nested = self
                .find_object_index(outer_id)
                .filter(|&index| self.objects[index].has_nonzero_status())
                .map(|index| self.objects[index].state.contents.clone())
                .unwrap_or_default();
            for object_id in nested {
                if self.object_is_base_auto_sell(object_id)
                    && self.player_can_sell_objects(base_owner)
                {
                    let _ = self.exit_object_at_current_position(object_id)?;
                    let _ = self.sell_object_to_home(object_id, object_id, base_owner)?;
                }
            }
            if self.object_is_base_auto_sell(outer_id) && self.player_can_sell_objects(base_owner) {
                let _ = self.exit_object_at_current_position(outer_id)?;
                let _ = self.sell_object_to_home(outer_id, outer_id, base_owner)?;
            }
        }
        Ok(())
    }

    fn player_can_sell_objects(&self, player: i32) -> bool {
        self.players.get(&player).is_some_and(|player| {
            !matches!(
                player.status(),
                PlayerStatus::Eliminated | PlayerStatus::Surrendered
            ) && !player.surrendered()
        })
    }

    fn object_is_base_auto_sell(&self, object_id: ObjectId) -> bool {
        self.find_object_index(object_id).is_some_and(|index| {
            let object = &self.objects[index];
            object.has_nonzero_status()
                && object.state.ocf & ocf::CREW_MEMBER == 0
                && self
                    .definitions
                    .get(&object.definition_id)
                    .is_some_and(Definition::base_auto_sell)
        })
    }

    /// `C4Object::ExecBase` (C4Object.cpp:1000-1044): base assignment,
    /// auto-sale/lost-flag handling, and upright structure snow clearing.
    pub(crate) fn exec_object_base(&mut self, idx: usize, frame: u64) -> Result<(), EngineError> {
        // New base assignment by flag, no old base removal (:1005-1018).
        if frame.is_multiple_of(10) {
            let base = self.objects[idx].state.base;
            let can_be_base = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .is_some_and(|definition| definition.can_be_base());
            if can_be_base && !self.players.contains_key(&base) {
                // Contents.Find(C4ID_Flag) (:1007).
                let flag = self.objects[idx].state.contents.iter().copied().find(|id| {
                    self.find_object_index(*id).is_some_and(|flag_index| {
                        let flag = &self.objects[flag_index];
                        flag.has_nonzero_status() && flag.definition_id.as_str() == "FLAG"
                    })
                });
                if let Some(flag_id) = flag {
                    let candidate_owner = flag
                        .and_then(|id| self.find_object_index(id))
                        .map(|flag_index| self.objects[flag_index].state.owner)
                        .unwrap_or(OWNER_NONE);
                    if self.players.contains_key(&candidate_owner) && candidate_owner != base {
                        let base_id = self.objects[idx].id;
                        // Attach new flag: Exit + FlyBase on this (:1010-1011).
                        let _ = self.exit_object_at_position_with_zero_motion(
                            flag_id,
                            base_id,
                            Vector2::ZERO,
                            0,
                        )?;
                        if let Some(flag_index) = self.find_object_index(flag_id) {
                            let flag_definition = self.objects[flag_index].definition_id.clone();
                            self.action_with_target_and_calls(
                                flag_index,
                                &flag_definition,
                                "FlyBase",
                                base_id,
                            )?;
                        }
                        // Exit and FlyBase callbacks may change Owner. C++
                        // re-reads flag->Owner for both assignments below.
                        let flag_owner = self
                            .find_object_index(flag_id)
                            .map(|flag_index| self.objects[flag_index].state.owner)
                            .unwrap_or(candidate_owner);
                        // Assign new base and force-close every remaining
                        // contained object's menu (:1013-1017;
                        // C4ObjectList::CloseMenus, C4ObjectList.cpp:705-710).
                        if let Some(idx) = self.find_object_index(base_id) {
                            self.objects[idx].state.base = flag_owner;
                            let contents = self.objects[idx].state.contents.clone();
                            for content in contents {
                                self.close_object_menu(content, true)?;
                            }
                            self.pending_audio.push(AudioCommand::PlaySound {
                                name: "Trumpet".to_string(),
                                target: Some(base_id),
                                volume: 100,
                                looped: false,
                                multiple: false,
                                custom_falloff: None,
                            });
                            self.set_object_owner(base_id, flag_owner)?;
                        }
                    }
                }
            }
        }
        // Base execution (:1021-1031); AutoSellContents unported.
        if frame.is_multiple_of(35) {
            let idx = match self.objects.get(idx) {
                Some(object) if object.state.status.is_active() => idx,
                _ => return Ok(()),
            };
            let base = self.objects[idx].state.base;
            if self.players.contains_key(&base) {
                if self.base_auto_sell_enabled {
                    self.auto_sell_base_contents(idx, base)?;
                }
                // Lost flag? Game.FindObject(C4ID_Flag, ..., "FlyBase", this)
                // (:1027-1030).
                let self_id = self.objects[idx].id;
                let has_flag = self.objects.iter().any(|flag| {
                    flag.state.status.is_active()
                        && !flag.destroyed
                        && flag.definition_id.as_str() == "FLAG"
                        && flag.state.action.name == "FlyBase"
                        && flag.state.action.target == Some(self_id)
                });
                if !has_flag {
                    let contents = self.objects[idx].state.contents.clone();
                    self.objects[idx].state.base = OWNER_NONE;
                    for content in contents {
                        self.close_object_menu(content, true)?;
                    }
                }
            }

            // Environmental action (:1033-1044): unless the STSN rule is
            // active, upright structures dig Snow and FlyAshes out of their
            // current shape rectangle. This is independent of Base validity.
            let snow_rect = self.objects.get(idx).and_then(|object| {
                (object.state.category & CATEGORY_STRUCTURE != 0
                    && object.state.rotation == 0
                    && !self.structures_snow_in)
                    .then(|| {
                        object.current_shape_rect().map(|shape| {
                            (
                                Vector2::new(
                                    object.state.position.x.saturating_add(shape.x),
                                    object.state.position.y.saturating_add(shape.y),
                                ),
                                shape.width,
                                shape.height,
                            )
                        })
                    })
                    .flatten()
            });
            if let Some((origin, width, height)) = snow_rect {
                for material in ["Snow", "FlyAshes"] {
                    if let Some(material) = self.materials.id_of(material) {
                        self.dig_free_material_rect(origin, width, height, material);
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn refresh_structures_snow_in_rule(&mut self) {
        self.structures_snow_in = self.objects.iter().any(|object| {
            object.definition_id.as_str() == "STSN"
                && object.state.status.is_active()
                && !object.destroyed
        });
    }

    /// `C4Game::UpdateRules` caches C4RULE_FlagRemoveable from Def.Count
    /// for FGRV. Inactive and contained objects still contribute; only an
    /// assigned removal/deleted object no longer counts.
    pub(crate) const fn cached_flag_removeable_rule(&self) -> bool {
        self.flag_removeable
    }

    pub(crate) fn refresh_flag_removeable_rule(&mut self) {
        self.flag_removeable = self.objects.iter().any(|object| {
            object.definition_id.as_str() == "FGRV"
                && object.state.status != ObjectStatus::Deleted
                && !object.destroyed
        });
    }

    /// `GBackWind` (C4Wrappers.h:189-192): zero inside tunnel-background
    /// (IFT) pixels, else the weather wind.
    pub(crate) fn wind_at(&self, x: i32, y: i32) -> i32 {
        let in_tunnel = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.is_ift_at(x, y))
            .unwrap_or(false);
        if in_tunnel {
            0
        } else {
            self.environment.wind_force(self.frame)
        }
    }

    /// The definition's `[Physical]` section for the object (the
    /// `Def->Physical` fallback and the info-clone source).
    pub(crate) fn definition_physical(&self, idx: usize) -> PhysicalInfo {
        self.definitions
            .get(&self.objects[idx].definition_id)
            .map(|definition| *definition.physical())
            .unwrap_or_default()
    }

    /// The definition retained by `C4ObjectInfo::pDef`. Unlike the object's
    /// current `Def`, this source survives `ChangeDef`.
    pub(crate) fn info_definition_physical(&self, idx: usize) -> Option<PhysicalInfo> {
        let object = &self.objects[idx];
        let info = self.crew_object_infos.get(&object.id)?;
        self.definitions
            .get(&info.definition_id)
            .map(|definition| *definition.physical())
            .or_else(|| Some(self.definition_physical(idx)))
    }

    pub(crate) fn fill_fair_crew_projection(
        &mut self,
        definition_id: DefinitionId,
        definition_physical: PhysicalInfo,
        rank_base: i32,
        script: Arc<ScriptEngine>,
    ) -> PhysicalInfo {
        let (mut physical, rank) = match begin_fair_crew_projection(
            definition_physical,
            self.fair_crew_strength,
            rank_base,
            &definition_id,
            &self.fair_crew_physical_cache,
        ) {
            FairCrewProjectionStart::Cached(physical) => return physical,
            FairCrewProjectionStart::New { physical, rank } => (physical, rank),
        };
        if !script.has_function("GetFairCrewPhysical") {
            return physical;
        }

        for name in FAIR_CREW_PHYSICAL_NAMES {
            let current = physical.value_by_name(name).unwrap_or_default();
            let args = [Value::from(name), Value::Int(rank), Value::Int(current)];
            let world = self.host_world_context();
            let global_effects = self.global_effects.clone();
            let (value, final_args, batch, audio, rng, script_error) =
                compat::with_fair_crew_definition_context(
                    definition_id.clone(),
                    definition_physical,
                    || {
                        ScenarioScript::execute_value_for_script(
                            definition_id.as_str(),
                            Some(definition_id.clone()),
                            "GetFairCrewPhysical",
                            &args,
                            world,
                            self.rng.clone(),
                            self.frame,
                            &global_effects,
                            self.physics,
                            self.environment,
                            self.audio_registry.clone(),
                            self.game_over_triggered,
                            || script.call_with_ref_args("GetFairCrewPhysical", &args),
                        )
                    },
                );
            self.rng = rng;
            self.audio_registry = audio;
            // Native host operations happen before the callback returns and
            // before its reference value commits. Fold the copied host world
            // first so any re-entrant read sees the published partial cache.
            if let Err(error) = self.apply_scenario_batch(batch) {
                tracing::warn!(
                    definition = %definition_id,
                    field = name,
                    %error,
                    "fair-crew callback host batch failed to apply"
                );
            }
            if script_error.is_none() && value.as_ref().is_some_and(compat::value_raw_truthy) {
                let value = final_args
                    .get(2)
                    .and_then(Value::as_c4_int)
                    .unwrap_or_default();
                physical.set_by_name(name, value);
                self.fair_crew_physical_cache
                    .borrow_mut()
                    .insert(definition_id.clone(), physical);
            }
        }
        physical
    }

    fn fair_crew_info_physical(&mut self, idx: usize, raw: PhysicalInfo) -> PhysicalInfo {
        let object = &self.objects[idx];
        let retained_id = self
            .crew_object_infos
            .get(&object.id)
            .map(|info| info.definition_id.clone());
        // C4Object::GetPhysical falls back from a null Info->pDef to the
        // object's current Def. Rust represents that null pointer as a
        // retained id whose definition is no longer loaded.
        let definition_id = retained_id
            .filter(|id| self.definitions.contains_key(id))
            .unwrap_or_else(|| object.definition_id.clone());
        let Some(definition) = self.definitions.get(&definition_id) else {
            return fair_crew_physical_cached(
                raw,
                self.fair_crew_strength,
                1_000,
                &definition_id,
                &self.fair_crew_physical_cache,
            );
        };
        let rank_base = definition.rank_base().unwrap_or(1_000);
        let script = definition.script_arc();
        self.fill_fair_crew_projection(definition_id, raw, rank_base, script)
    }

    /// `C4Object::GetPhysical` (C4Object.cpp:2118-2134): the temporary set
    /// when temporary mode is on; otherwise an object carrying crew info
    /// resolves either its persistent info physicals or the definition's
    /// fair-crew projection cached by definition. Objects without info use
    /// the definition's `[Physical]` section.
    #[doc(hidden)]
    /// `C4Object::ViewEnergy`, the transient cursor-bar timer. It is live
    /// object state rather than snapshot state: C++ marks it `// NoSave //`
    /// and never synchronizes it (C4Object.h:145).
    pub fn object_view_energy(&self, idx: usize) -> i32 {
        self.objects[idx].state.view_energy
    }

    pub fn object_physical(&mut self, idx: usize) -> PhysicalInfo {
        if let Some(temporary) = self.objects[idx].state.temporary_physical {
            return temporary;
        }
        let definition = self.definition_physical(idx);
        match self.info_definition_physical(idx) {
            Some(info_definition) if self.use_fair_crew => {
                self.fair_crew_info_physical(idx, info_definition)
            }
            Some(info_definition) => self.objects[idx]
                .state
                .info_physical
                .unwrap_or(info_definition),
            None => definition,
        }
    }

    /// Command tables carry a physical field for every structural target,
    /// although production handlers only inspect the executing actor's field.
    /// Build those unused target fields without triggering a lazy definition
    /// callback; the actor is resolved explicitly at its native read seam.
    pub(crate) fn object_physical_without_fair_fill(&self, idx: usize) -> PhysicalInfo {
        if let Some(temporary) = self.objects[idx].state.temporary_physical {
            return temporary;
        }
        let definition = self.definition_physical(idx);
        match self.info_definition_physical(idx) {
            Some(info_definition) if self.use_fair_crew => {
                let object = &self.objects[idx];
                let definition_id = self
                    .crew_object_infos
                    .get(&object.id)
                    .map(|info| info.definition_id.clone())
                    .filter(|id| self.definitions.contains_key(id))
                    .unwrap_or_else(|| object.definition_id.clone());
                if let Some(physical) = self
                    .fair_crew_physical_cache
                    .borrow()
                    .get(&definition_id)
                    .copied()
                {
                    return physical;
                }
                let rank_base = self
                    .definitions
                    .get(&definition_id)
                    .and_then(Definition::rank_base)
                    .unwrap_or(1_000);
                fair_crew_physical(info_definition, self.fair_crew_strength, rank_base)
            }
            Some(info_definition) => self.objects[idx]
                .state
                .info_physical
                .unwrap_or(info_definition),
            None => definition,
        }
    }

    pub(crate) fn object_physical_will_fill_fair_cache(&self, idx: usize) -> bool {
        if !self.use_fair_crew || self.objects[idx].state.temporary_physical.is_some() {
            return false;
        }
        let object = &self.objects[idx];
        let Some(info) = self.crew_object_infos.get(&object.id) else {
            return false;
        };
        let definition_id = if self.definitions.contains_key(&info.definition_id) {
            &info.definition_id
        } else {
            &object.definition_id
        };
        !self
            .fair_crew_physical_cache
            .borrow()
            .contains_key(definition_id)
    }

    /// `C4Object::TrainPhysical` (C4Object.cpp:2136-2146): trains the
    /// temporary set when active — including the stacked previous values for
    /// the same physical (C4InfoCore.cpp:309-317) — and the info physicals
    /// when the object carries an actual info pointer, seeded lazily from
    /// `Info->pDef`. Returns false when the object has neither.
    #[doc(hidden)]
    pub fn train_physical(
        &mut self,
        idx: usize,
        name: &str,
        train_by: i32,
        max_train: i32,
    ) -> bool {
        let object_id = self.objects[idx].id;
        let info_definition_physical = self.info_definition_physical(idx);
        let (trained, info_writeback) = {
            let object = &mut self.objects[idx];
            let mut trained = false;
            if let Some(temporary) = object.state.temporary_physical.as_mut() {
                if let Some(value) = temporary.value_mut_by_name(name) {
                    PhysicalInfo::train_value(value, train_by, max_train);
                }
                for (_, previous) in object
                    .state
                    .physical_changes
                    .iter_mut()
                    .filter(|(changed, _)| changed.eq_ignore_ascii_case(name))
                {
                    PhysicalInfo::train_value(previous, train_by, max_train);
                }
                trained = true;
            }
            let mut info_writeback = None;
            if let Some(definition_physical) = info_definition_physical {
                let info = object
                    .state
                    .info_physical
                    .get_or_insert(definition_physical);
                if let Some(value) = info.value_mut_by_name(name) {
                    PhysicalInfo::train_value(value, train_by, max_train);
                }
                info_writeback = Some(*info);
                trained = true;
            }
            (trained, info_writeback)
        };
        if let (Some(link), Some(physical)) = (
            self.crew_info_links.get(&object_id).copied(),
            info_writeback,
        ) {
            self.set_linked_crew_info_physical(link, physical);
        }
        trained
    }

    /// `C4Effect::DoDamage` (C4Effect.cpp:312-322): walk the object's
    /// effects in list order; each `Fx<Name>Damage` callback (resolved via
    /// its command-target script) receives the running damage and its
    /// return REPLACES it (`getInt` — a nil return zeroes it); the walk
    /// stops once the damage reaches zero.
    fn call_effects_do_damage(
        &mut self,
        idx: usize,
        mut change: i32,
        cause: i32,
        caused_by: i32,
    ) -> Result<i32, EngineError> {
        let effects: Vec<EffectState> = self.objects[idx].state.effects.clone();
        if effects.is_empty() {
            return Ok(change);
        }
        let object_id = self.objects[idx].id;
        let host_definition_id = self.objects[idx].definition_id.clone();
        let mut first_effect = true;
        for effect in effects {
            // C4Effect::DoDamage is a do/while: even an initial zero visits
            // the head node once, then zero stops the suffix.
            if change == 0 && !first_effect {
                break;
            }
            first_effect = false;
            if effect.priority == 0 {
                continue;
            }
            let dispatch_id = effect
                .command_target
                .and_then(|target| self.find_object_index(ObjectId::new(target as u64)))
                .map(|target_idx| self.objects[target_idx].definition_id.clone())
                .or_else(|| {
                    effect
                        .command_id
                        .as_ref()
                        .filter(|id| self.definitions.contains_key(*id))
                        .cloned()
                })
                .unwrap_or_else(|| host_definition_id.clone());
            let world = self.host_world_context();
            let callback_name = format!("Fx{}Damage", effect.name);
            let has_callback =
                resolve_effect_script_callback(&effect, &callback_name, &world).is_some();
            if !has_callback {
                continue;
            }
            let action_library = self
                .definitions
                .get(&host_definition_id)
                .map(|definition| definition.action_library().clone())
                .unwrap_or_default();
            let state_snapshot = self.objects[idx].script_state_snapshot();
            let rng_state = self.rng.clone();
            let global_view = self.global_effects.clone();
            let Some(definition) = self.definitions.get(&dispatch_id) else {
                continue;
            };
            let callback = definition.call_effect_damage(
                Some((&state_snapshot, object_id)),
                &effect,
                change,
                cause,
                caused_by,
                rng_state,
                &global_view,
                self.physics,
                self.environment,
                self.frame,
                world,
                self.game_over_triggered,
                self.audio_registry.clone(),
            );
            let Some((outcome, audio_state, new_rng, result)) = tolerate_script_error(callback)?
            else {
                continue;
            };
            self.rng = new_rng;
            self.audio_registry = audio_state;
            self.apply_action_callback_outcome(
                idx,
                outcome,
                &action_library,
                object_id,
                &host_definition_id,
            )?;
            if let Some(value) = result.as_ref() {
                change = compat::value_as_i32(value);
            }
        }
        Ok(change)
    }

    /// `C4Object::DoDamage` (C4Object.cpp:1330-1343): NON-living things ask
    /// their effects first, the damage stat clamps at zero, and the Damage
    /// script callback fires with (change, causedBy).
    #[doc(hidden)]
    pub fn change_object_damage(
        &mut self,
        idx: usize,
        change: i32,
        cause: i32,
        caused_by: i32,
    ) -> Result<(), EngineError> {
        let change =
            if !self.objects[idx].state.alive && !self.objects[idx].state.effects.is_empty() {
                let modified = self.call_effects_do_damage(idx, change, cause, caused_by)?;
                if modified == 0 {
                    return Ok(());
                }
                modified
            } else {
                change
            };
        {
            let object = &mut self.objects[idx];
            object.state.damage = object.state.damage.saturating_add(change).max(0);
        }
        // Engine script call (C4Object.cpp:1342).
        let _ = tolerate_script_error(self.call_object_function(
            idx,
            "Damage",
            vec![Value::Int(change), Value::Int(caused_by)],
        ))?;
        Ok(())
    }

    /// Engine-side `C4Object::DoEnergy` slice (C4Object.cpp:1345-1365) in the
    /// engine's percent-point energy units: living things' effects get the
    /// Fx*Damage modification first (C4Object.cpp:1355-1359, zero aborts),
    /// then clamp between zero and the physical Energy ceiling (scaled from
    /// the 0..C4MaxPhysical range to percent points), track the last
    /// energy-loss cause (C4Object.cpp:1353), and assign death when an alive
    /// object's energy first reaches zero (C4Object.cpp:1363). A definition
    /// without Physical Energy has a zero ceiling and therefore clamps to
    /// zero as well.
    /// Engine-side `C4Object::DoEnergy` with fExact=false — every engine
    /// caller passes percent (fire/hit/asphyxiation/corrosion,
    /// C4Object.cpp:782/904/928, C4GameObjects.cpp:174): the change scales
    /// by C4MaxPhysical/100 BEFORE the effect DoDamage hook
    /// (C4Object.cpp:1347 precedes :1355).
    #[doc(hidden)]
    pub fn change_object_energy(
        &mut self,
        idx: usize,
        change: i32,
        cause: i32,
        caused_by: i32,
    ) -> Result<(), EngineError> {
        let change = change.saturating_mul(C4_MAX_PHYSICAL / 100);
        self.change_object_energy_raw(idx, change, cause, caused_by)
    }

    /// `DoEnergy(..., fExact=true)`: base transfers already carry raw
    /// physical units and therefore bypass the percent-to-physical scale.
    fn change_object_energy_exact(
        &mut self,
        idx: usize,
        change: i32,
        cause: i32,
        caused_by: i32,
    ) -> Result<(), EngineError> {
        self.change_object_energy_raw(idx, change, cause, caused_by)
    }

    fn change_object_energy_raw(
        &mut self,
        idx: usize,
        change: i32,
        cause: i32,
        caused_by: i32,
    ) -> Result<(), EngineError> {
        // C++ captures this before damage effects and GetPhysical callbacks
        // (C4Object.cpp:1375-1377).
        let was_zero = self.objects[idx].state.energy == 0;
        // Mark the damage-causing player first (C4Object.cpp:1351-1353).
        if change < 0 || cause == C4FX_CALL_ENG_OBJ_HIT {
            self.update_last_energy_loss_cause(idx, caused_by);
        }
        // Living things: ask effects for change first (C4Object.cpp:1355-1359).
        let change = if self.objects[idx].state.alive && !self.objects[idx].state.effects.is_empty()
        {
            let modified = self.call_effects_do_damage(idx, change, cause, caused_by)?;
            if modified == 0 {
                return Ok(());
            }
            modified
        } else {
            change
        };
        let max_energy = self.object_physical(idx).energy;
        {
            let object = &mut self.objects[idx];
            object.state.energy =
                bound_energy(object.state.energy.saturating_add(change), max_energy);
        }
        if self.objects[idx].state.alive && self.objects[idx].state.energy == 0 && !was_zero {
            let _ = tolerate_script_error(self.assign_death(idx, false))?;
        }
        Ok(())
    }

    /// `C4Object::UpdatLastEnergyLossCause` (C4Object.cpp:1369-1378):
    /// self-administered damage does not steal an already-tracked killer —
    /// only a DIFFERENT player (or an empty slot) updates the kill trace.
    fn update_last_energy_loss_cause(&mut self, idx: usize, new_cause_player: i32) {
        let object = &mut self.objects[idx];
        if new_cause_player != object.state.controller || object.last_energy_loss_cause < 0 {
            object.last_energy_loss_cause = new_cause_player;
        }
    }

    /// `C4Object::AssignDeath` core (C4Object.cpp:1137-1177): alive objects
    /// only; clear effects with the death reason and honor revival, set the
    /// "Dead" action, clear commands, eject contents at their stored positions,
    /// clean player crew/cursor/view pointers, then run the Death script callback
    /// with the death-causing player.
    #[doc(hidden)]
    pub fn assign_death(&mut self, idx: usize, forced: bool) -> Result<(), EngineError> {
        if !self.objects[idx].state.alive {
            return Ok(());
        }
        let death_causing_player = self.objects[idx].last_energy_loss_cause;
        self.objects[idx].state.alive = false;
        // C4Effect::ClearAll recurses into pNext first, so Stop callbacks run
        // from the highest list entry back to the lowest. A Stop may deny its
        // own death removal and set Alive again; ordinary AssignDeath then
        // aborts immediately (C4Object.cpp:1162-1170;
        // C4Effect.cpp:407-424).
        let object_id = self.objects[idx].id;
        let definition_id = self.objects[idx].definition_id.clone();
        // Keep the nodes linked while callbacks run, as C4Effect::ClearAll
        // does. The event loop marks each node dead immediately before its
        // Stop callback; linked dead nodes still reserve their effect number
        // for effects added by that callback.
        let mut effect_events = self.objects[idx]
            .state
            .effects
            .iter()
            .cloned()
            .map(|effect| EffectEvent::stopped(effect, EffectStopReason::Death))
            .collect::<Vec<_>>();
        effect_events.reverse();
        if !effect_events.is_empty() {
            self.dispatch_object_effect_events(idx, &definition_id, effect_events)?;
        }
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(());
        };
        if self.objects[idx].state.alive && !forced {
            return Ok(());
        }
        // Ordinary SetActionByName("Dead") (C4Object.cpp:1153): this does
        // not bypass NoOtherAction and runs Dead StartCall before the old
        // action's AbortCall synchronously.
        self.set_death_action_by_name(idx, "Dead")?;
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Values: Select=0; Alive=0 (C4Object.cpp:1154-1155). Forced
        // deaths must clear Alive again after a death effect revived it.
        self.objects[idx].state.selected = false;
        self.objects[idx].state.alive = false;
        // ClearCommands (C4Object.cpp:1157)
        self.objects[idx].command_queue.clear();
        self.objects[idx].commands.clear();
        // Info->HasDied=true; ++Info->DeathCount; Info->Retire(), but the
        // pointer remains on the dead object (C4Object.cpp:1185-1190).
        let mut info_update = None;
        if let Some(link) = self.crew_info_links.get(&object_id).copied() {
            if let Some(info) = self
                .crew_rosters
                .get_mut(&link.player_id)
                .and_then(|roster| roster.get_mut(link.roster_index))
            {
                info.has_died = true;
                info.death_count = info.death_count.wrapping_add(1);
                if info.in_action {
                    info.total_playing_time = info
                        .total_playing_time
                        .wrapping_add(self.game_time.wrapping_sub(info.in_action_time));
                    info.in_action = false;
                }
                info_update = Some((
                    info.death_count,
                    info.total_playing_time,
                    info.in_action_time,
                    info.age,
                ));
            }
        }
        if let Some((death_count, total_playing_time, in_action_time, age)) = info_update {
            if let Some(info) = Rc::make_mut(&mut self.crew_object_infos).get_mut(&object_id) {
                info.death_count = death_count;
                info.total_playing_time = total_playing_time;
                info.in_action_time = in_action_time;
                info.age = age;
            }
        }
        // Lose contents by repeatedly exiting the current live list head
        // (C4Object.cpp:1192). Ejection/Departure may mutate the remaining
        // contents, so taking or cloning the list once is observably wrong.
        loop {
            let Some(idx) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let content_id = self.objects[idx]
                .state
                .contents
                .iter()
                .copied()
                .find(|&content_id| {
                    self.find_object_index(content_id)
                        .is_some_and(|content_idx| {
                            let content = &self.objects[content_idx];
                            !content.destroyed && content.state.status != ObjectStatus::Deleted
                        })
                });
            let Some(content_id) = content_id else {
                break;
            };
            let _ = self.exit_object_at_current_position(content_id)?;
        }
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // C4Player::ClearPointers(this, true): remove from crew and clear a
        // matching cursor/view pointer, then choose a replacement cursor.
        let owner = self.objects[idx].state.owner;
        let owner_player_exists = self.players.contains_key(&owner);
        let removed_cursor = self
            .players
            .get(&owner)
            .is_some_and(|player| player.cursor() == Some(object_id));
        if self.crew_cursor(owner) == Some(object_id) {
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
            player.clear_object_pointers_before_cursor_adjust(object_id);
        }
        self.remove_from_roles(owner, object_id);
        let still_in_crew = self
            .players
            .values()
            .any(|player| player.crew().contains(&object_id));
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].state.crew_member = still_in_crew;
        }
        if removed_cursor {
            self.player_adjust_cursor_command(owner)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.clear_object_pointers_after_cursor_adjust(object_id);
        }
        // C++ retains a dead living object's range only while the owning
        // player's runtime FoWViewObjs still contains it.
        let retained_by_owner_fow = self.find_object_index(object_id).is_some_and(|idx| {
            let state = &self.objects[idx].state;
            owner_player_exists
                && state.category & CATEGORY_LIVING != 0
                && self
                    .players
                    .get(&owner)
                    .is_some_and(|player| player.has_fow_view_object(object_id))
        });
        if !retained_by_owner_fow {
            if let Some(idx) = self.find_object_index(object_id) {
                self.objects[idx].state.plr_view_range = 0;
            }
            self.actualize_object_fow_view_range(object_id);
        }
        // Engine script call (C4Object.cpp:1173)
        if let Some(idx) = self.find_object_index(object_id) {
            let _ = tolerate_script_error(self.call_object_function(
                idx,
                "Death",
                vec![Value::Int(death_causing_player)],
            ))?;
        }
        // AssignDeath refreshes OCF after the fail-safe Death callback;
        // that callback may itself revive the object.
        if let Some(idx) = self.find_object_index(object_id) {
            self.refresh_object_ocf(idx);
        }
        Ok(())
    }

    /// C4Object::ExecMovement's complete out-of-bounds predicate
    /// (src/C4Movement.cpp:598-617). Contained and StaticBack objects return
    /// before this tail. Bounded sides/bottom are exempt, live DFA_ATTACH
    /// targets follow their removed target one frame later, and parallax HUD
    /// objects use the asymmetric Local[0] viewport rules below.
    pub(crate) fn object_should_be_removed_out_of_bounds(&self, idx: usize) -> bool {
        let Some(object) = self.objects.get(idx) else {
            return false;
        };
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        let Some(definition) = self.definitions.get(&object.definition_id) else {
            return false;
        };
        let width = i32::try_from(landscape.width()).unwrap_or(i32::MAX);
        let height = landscape.estimated_height();
        let x = object.state.position.x;
        let y = object.state.position.y;
        let outside_unbounded_side =
            (x < 0 || x > width) && definition.border_bound() & C4D_BORDER_SIDES == 0;
        let outside_unbounded_bottom =
            y > height && definition.border_bound() & C4D_BORDER_BOTTOM == 0;
        if !outside_unbounded_side && !outside_unbounded_bottom {
            return false;
        }
        let attached_to_target = matches!(
            definition
                .action_library()
                .procedure_for_entry(&object.state.action.name, object.state.action.act_map_index,),
            ActionProcedure::Attach
        ) && object.state.action.target.is_some();
        if attached_to_target {
            return false;
        }

        if object.state.category & CATEGORY_PARALLAX == 0 {
            return true;
        }

        // C4D_Parallax objects normally survive outside the landscape so HUD
        // elements can be positioned in viewport coordinates. C++ still
        // removes them beyond the right/bottom, on the left when Local[0]
        // enables horizontal parallax, or farther than one landscape width
        // left when Local[0] is zero (C4Movement.cpp:606-612).
        if x > width || y > height {
            return true;
        }
        let horizontal_parallax = object
            .state
            .local_vars
            .get("__local_0")
            .is_some_and(compat::value_raw_truthy);
        if x < 0 && horizontal_parallax {
            return true;
        }
        !horizontal_parallax && x < -width
    }

    /// Engine-owned `C4Object::AssignRemoval(false)`.
    pub(crate) fn assign_object_removal(
        &mut self,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        self.assign_object_removal_with_contents(object_id, false)
    }

    /// Callbacks and effect cleanup run while the object is live, Status
    /// becomes Deleted before contents are handled, and the container link
    /// is removed last. `AssignRemoval(true)` exits direct contents at the
    /// removed object's position; the default recursively removes them
    /// instead (C4Object.cpp:240-309).
    pub(crate) fn assign_object_removal_with_contents(
        &mut self,
        object_id: ObjectId,
        exit_contents: bool,
    ) -> Result<bool, EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        if self.objects[index].destroyed
            || self.objects[index].state.status == ObjectStatus::Deleted
        {
            return Ok(false);
        }

        if let Some(container_index) = self.objects[index]
            .state
            .container
            .and_then(|container| self.find_object_index(container))
            .filter(|&container_index| {
                !self.objects[container_index].destroyed
                    && self.objects[container_index].state.status != ObjectStatus::Deleted
            })
        {
            let _ = tolerate_script_error(self.call_object_function(
                container_index,
                "ContentsDestruction",
                vec![object_reference_value(object_id)],
            ))?;
        }

        let Some(index) = self.find_object_index(object_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let _ = tolerate_script_error(self.call_object_function(index, "Destruction", Vec::new()))?;

        let Some(index) = self.find_object_index(object_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let definition_id = self.objects[index].definition_id.clone();
        // ClearAll recurses through the original list tail-to-head. Each
        // victim is marked dead but remains linked during FxStop, so earlier
        // callbacks can still inspect later effects (including a denied Stop
        // that restored its priority). Callback-added effects are outside
        // the captured traversal and receive no RemoveClear callback.
        let original_effects = self.objects[index]
            .state
            .effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .map(|effect| effect.number)
            .collect::<Vec<_>>();
        for number in original_effects.into_iter().rev() {
            let Some(index) = self.find_object_index(object_id).filter(|&index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != ObjectStatus::Deleted
            }) else {
                return Ok(true);
            };
            let Some(effect_index) = self.objects[index]
                .state
                .effects
                .iter()
                .position(|effect| effect.number == number && effect.priority != 0)
            else {
                continue;
            };
            let effect = self.objects[index].state.effects[effect_index].clone();
            self.objects[index].state.effects[effect_index].priority = 0;
            self.dispatch_object_effect_events(
                index,
                &definition_id,
                vec![EffectEvent::stopped(effect, EffectStopReason::Cleared)],
            )?;
        }

        let Some(index) = self.find_object_index(object_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        // `delete pEffects` follows ClearAll unconditionally: denied
        // originals and callback-added effects vanish without another Stop,
        // before particles and SetAction(Idle).
        self.objects[index].state.effects.clear();
        self.apply_particle_commands(vec![ParticleCommand::Clear {
            definition_id: None,
            scope: ParticleScope::Object(object_id),
        }]);
        let Some(index) = self.find_object_index(object_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let definition_id = self.objects[index].definition_id.clone();
        let _ = self.action_with_calls(index, &definition_id, "Idle")?;

        let Some(index) = self.find_object_index(object_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let previous_status = self.objects[index].state.status;
        let exit_position = self.objects[index].state.position;
        let _ = self.objects[index].mark_destroyed();
        self.update_inactive_list_for_status_change(
            object_id,
            previous_status,
            ObjectStatus::Deleted,
        );
        self.update_sector_for_index(index);

        // Status is already zero while each child is destroyed. As in C++,
        // remove the list link first but leave the child's Contained pointer
        // available to its Destruction callback until its own removal tail.
        loop {
            let child = self
                .find_object_index(object_id)
                .and_then(|index| self.objects[index].state.contents.first().copied());
            let Some(child) = child else { break };
            // Native's loop condition stops when First->Obj is null. A
            // missing Rust object-table entry is the denumerated equivalent;
            // do not spin forever on the retained list slot.
            if self.find_object_index(child).is_none() {
                break;
            }
            if exit_contents {
                let _ = self.exit_object_at_position_with_zero_motion(
                    child,
                    object_id,
                    exit_position,
                    0,
                )?;
                continue;
            }
            if let Some(index) = self.find_object_index(object_id) {
                self.track_contents_link_removal(object_id, child);
                self.objects[index]
                    .state
                    .contents
                    .retain(|&candidate| candidate != child);
            }
            if self.find_object_index(child).is_some() {
                let _ = self.assign_object_removal(child)?;
            }
        }

        let container = self
            .find_object_index(object_id)
            .and_then(|index| self.objects[index].state.container);
        if let Some(container) = container {
            if let Some(container_index) = self.find_object_index(container) {
                self.track_contents_link_removal(container, object_id);
                self.objects[container_index]
                    .state
                    .contents
                    .retain(|&child| child != object_id);
                self.refresh_object_ocf(container_index);
            }
            if let Some(index) = self.find_object_index(object_id) {
                let object = &mut self.objects[index];
                object.state.container = None;
                object.compiler_cache.contained = 0;
            }
        }

        // Info->Retire and Info=null happen before C4Value/Game pointer
        // clearing (C4Object.cpp:297-304).
        if let Some(link) = self.crew_info_links.get(&object_id).copied() {
            if let Some(info) = self
                .crew_rosters
                .get_mut(&link.player_id)
                .and_then(|roster| roster.get_mut(link.roster_index))
            {
                if info.in_action {
                    info.total_playing_time = info
                        .total_playing_time
                        .wrapping_add(self.game_time.wrapping_sub(info.in_action_time));
                    info.in_action = false;
                }
            }
        }
        Rc::make_mut(&mut self.crew_info_links).remove(&object_id);
        Rc::make_mut(&mut self.crew_object_infos).remove(&object_id);
        Rc::make_mut(&mut self.crew_ranks).remove(&object_id.as_u64());
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].retired_info_physical = self.objects[index].state.info_physical;
            self.objects[index].state.info_physical = None;
        }

        self.clear_object_references_for_removal(object_id)?;
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].command_queue.clear();
            self.objects[index].commands.clear();
            self.remove_solid_mask(index);
            self.update_sector_for_index(index);
        }
        self.note_objects_changed();
        Ok(true)
    }

    /// `AssignDeath(true); AssignRemoval()` from the movement tail
    /// (src/C4Movement.cpp:613-614). Rust's object store has no two-frame
    /// `RemovalDelay` tombstone yet, so the normal end-of-frame destroyed
    /// cleanup removes it after all synchronous callbacks have run.
    pub(crate) fn assign_out_of_bounds_removal(&mut self, idx: usize) -> Result<(), EngineError> {
        let Some(object_id) = self.objects.get(idx).map(|object| object.id) else {
            return Ok(());
        };
        tolerate_script_error(self.assign_death(idx, true))?;
        let _ = self.assign_object_removal(object_id)?;
        Ok(())
    }

    pub(crate) fn object_definition_context(
        &self,
        index: usize,
    ) -> Result<(DefinitionId, SharedActionLibrary), EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        let action_library = self
            .shared_action_library_for(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        Ok((definition_id, action_library))
    }

    pub(crate) fn shared_action_library_for(
        &self,
        definition_id: &str,
    ) -> Option<SharedActionLibrary> {
        self.definition_metadata_table()
            .get(definition_id)
            .map(|metadata| metadata.action_library.clone())
    }

    /// Definition-swap half of `C4Object::ChangeDef`. Script-host outcomes
    /// have already performed the synchronous Exit/RejectEntrance/Enter
    /// lifecycle; engine-owned callers use `change_object_def_live` below.
    pub(crate) fn apply_change_object_def(&mut self, idx: usize, new_def: &str) {
        let contents_count = self.retained_contents_count(&self.objects[idx].state.contents);
        let Some(definition) = self.definitions.get(new_def) else {
            return;
        };
        let material_capacity = self.materials.len();
        let owner_color = self
            .players
            .get(&self.objects[idx].state.owner)
            .and_then(Player::color)
            .map(|color| u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b));
        Self::apply_change_object_def_to_object(
            &mut self.objects[idx],
            new_def,
            definition,
            material_capacity,
            owner_color,
        );
        let ocf =
            definition.compute_ocf_with_contents_count(&self.objects[idx].state, contents_count);
        self.objects[idx].state.ocf = ocf;
    }

    pub(crate) fn apply_change_object_def_to_object(
        object: &mut Object,
        new_def: &str,
        definition: &Definition,
        material_capacity: usize,
        owner_color: Option<u32>,
    ) {
        let vertices = definition.shape_vertices().to_vec();
        let template = ObjectShapeTemplate::new(
            vertices.clone(),
            definition.shape_rect(),
            definition.fire_top(),
            definition.stretch_growth(),
            definition.rotateable(),
        )
        .with_line(definition.line());
        let blit_mode = definition.blit_mode();
        let rotateable = definition.rotateable();
        let previous_rect = object.current_shape_rect();
        let previous_construction = object.state.construction;
        object.compiled_mass = None;
        object.definition_id = new_def.to_string();
        object.solid_mask_instance_sequence = None;
        object.unsorted = true;
        object.state.base_graphics = None;
        // Category is an object field, initialized from Def only in Init.
        // C4Object::ChangeDef deliberately preserves it.
        // C4Object::ChangeDef follows the new definition's mode unless a
        // script explicitly marked the old mode custom (C4Object.cpp:1231).
        if object.state.blit_mode & 128 == 0 {
            object.state.blit_mode = blit_mode;
        }
        if !definition.color_by_owner() {
            object.state.color = 0;
        } else if object.state.color == 0 {
            if let Some(color) = owner_color {
                object.state.color = color;
            }
        }
        object.shape_template = template;
        if definition.line() == 0 {
            object.state.shape_override = None;
        }
        // SolidMask falls back to the NEW def default (C4Object.cpp:1213)
        object.state.solid_mask_override = None;
        // Non-rotateable defs reset rotation (C4Object.cpp:1211)
        if rotateable == 0 {
            object.state.rotation = 0;
            object.fixed_rotation = C4Fixed::ZERO;
            object.rotation_velocity = C4Fixed::ZERO;
        }
        // UpdateFace(true) rebuilds against the new template but preserves
        // fOwnVertices. Line definitions keep their independent live shape.
        object.refresh_shape_after_state_change(previous_construction, previous_rect, false);
        object.ensure_material_capacity(material_capacity);
        // ChangeDef finishes with SetOCF before returning to script. Keep the
        // raw helper callback-visible even in folds that do not own an
        // Engine index yet (Construction/Initialize/effect/contact paths).
        object.state.ocf = definition.compute_ocf(&object.state);
    }

    /// Full engine-owned C4Object::ChangeDef lifecycle. This is required by
    /// native BurnTurnTo; script-host changes have already executed the same
    /// callbacks synchronously and use `apply_change_object_def` at fold.
    pub(crate) fn change_object_def_live(
        &mut self,
        idx: usize,
        new_def: &str,
    ) -> Result<bool, EngineError> {
        if !self.definitions.contains_key(new_def) {
            return Ok(false);
        }
        let object_id = self.objects[idx].id;
        let previous_container = self.exit_object_for_change_def(object_id)?;

        if let Some(current_index) = self.find_object_index(object_id) {
            // BoundsCheck Contact* callbacks run before the outer
            // ChangeDef's SetAction and may themselves ChangeDef. C++ reads
            // `Def` only after Exit returns, so the action transition must
            // use the definition live now rather than the pre-Exit one.
            let old_definition_id = self.objects[current_index].definition_id.clone();
            // Ordinary old-definition SetAction(ActIdle), including callback
            // order. The subsequent raw swap enforces Idle even if
            // NoOtherAction rejected this transition.
            let _ = tolerate_script_error(self.action_with_calls(
                current_index,
                &old_definition_id,
                "Idle",
            ))?;
            if let Some(current_index) = self.find_object_index(object_id) {
                // The assignment is unconditional, even when SetAction
                // succeeded and its AbortCall selected another action.
                // Only Act is overwritten; callback-written Time/Data/Phase
                // and targets survive (C4Object.cpp:1217-1218).
                self.objects[current_index].state.action.name = "Idle".to_string();
                self.objects[current_index].state.action.act_map_index = None;
            }
        }

        let Some(current_index) = self.find_object_index(object_id) else {
            return Ok(true);
        };
        self.apply_change_object_def(current_index, new_def);
        self.update_solid_mask(current_index);
        self.update_sector_for_index(current_index);
        self.refresh_object_ocf(current_index);

        if let Some(container) = previous_container {
            let _ = self.try_object_enter_with_reject_collect_and_calls(
                object_id, container, false, false,
            )?;
        }
        Ok(true)
    }

    /// AssignDeath's ordinary SetActionByName("Dead") transition.
    fn set_death_action_by_name(&mut self, idx: usize, action: &str) -> Result<(), EngineError> {
        let definition_id = self.objects[idx].definition_id.clone();
        let Some(library) = self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.action_library().clone())
        else {
            return Ok(());
        };
        if !library.contains(action) {
            return Ok(());
        }
        let previous = self.objects[idx].state.action.clone();
        let update = ActionUpdate {
            name: Some(action.to_string()),
            phase: Some(0),
            ticks: Some(0),
            force: false,
            data: None,
            // SetAction assigns targets only when given
            // (C4Object.cpp:4122-4123).
            target: None,
            target2: None,
            callbacks_dispatched: false,
        };
        let object = &mut self.objects[idx];
        let result = object
            .state
            .action
            .apply_update_with_library(&update, &library);
        // SetAction fix resync (C4Object.cpp:4144) — only past the
        // NoOtherAction early returns.
        let applied = matches!(result, ActionUpdateResult::Applied);
        if update.name.is_some() && applied {
            object.fixed_position =
                FixedVec2::from_ints(object.state.position.x, object.state.position.y);
        }
        if applied {
            let previous_flip_dir = object.state.action_flip_dir(&library);
            object.record_action_event(previous, ActionTransitionKind::Forced);
            // SetAction's FlipDir refresh, guarded on the value changing and
            // ordered before SetOCF (C4Object.cpp:4182-4192).
            if previous_flip_dir != self.object_action_flip_dir(idx) {
                self.update_object_flip_dir(idx);
            }
            // SetAction calls SetOCF before StartCall/AbortCall
            // (C4Object.cpp:4141,4173).
            self.refresh_object_ocf(idx);
        }
        self.trigger_action_callbacks(idx, Some(action.to_string()))?;
        Ok(())
    }

    /// `C4Object::Fling` (C4Object.cpp:1639-1652) without fAddSpeed: trace the
    /// causing player, try the Tumble action, then Jump (ObjectActionTumble/Jump,
    /// C4ObjectCom.cpp:48-80), else set the velocity directly.
    pub(crate) fn fling_object(
        &mut self,
        idx: usize,
        txdir: C4Fixed,
        tydir: C4Fixed,
        caused_by: i32,
    ) {
        // C4Object::Fling attributes indirect kills before changing the
        // action: living objects update their kill trace; an uncontained
        // non-living object takes the causing player as Controller
        // (C4Object.cpp:1641-1642).
        if self.object_ocf_at_index(idx) & crate::ocf::ALIVE != 0 {
            self.update_last_energy_loss_cause(idx, caused_by);
        } else if self.objects[idx].state.container.is_none() {
            self.objects[idx].state.controller = caused_by;
        }
        let definition_id = self.objects[idx].definition_id.clone();
        let library = self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.action_library().clone());
        if let Some(library) = library {
            for action in ["Tumble", "Jump"] {
                if !library.contains(action) {
                    continue;
                }
                let previous = self.objects[idx].state.action.clone();
                let update = ActionUpdate {
                    name: Some(action.to_string()),
                    phase: Some(0),
                    ticks: Some(0),
                    force: false,
                    data: None,
                    // SetAction assigns targets only when given
                    // (C4Object.cpp:4122-4123).
                    target: None,
                    target2: None,
                    callbacks_dispatched: false,
                };
                let object = &mut self.objects[idx];
                let result = object
                    .state
                    .action
                    .apply_update_with_library(&update, &library);
                if matches!(result, ActionUpdateResult::Applied) {
                    if previous.name != object.state.action.name
                        || previous.act_map_index != object.state.action.act_map_index
                    {
                        object.record_action_event(previous, ActionTransitionKind::Forced);
                    }
                    // Tumble also turns the object (SetDir, C4ObjectCom.cpp:77)
                    if action == "Tumble" {
                        let direction = if txdir < C4Fixed::ZERO {
                            Direction::Right
                        } else {
                            Direction::Left
                        };
                        object
                            .state
                            .write_direction(direction, object.state.action_flip_dir(&library));
                    } else {
                        // ObjectActionJump mobilizes (C4ObjectCom.cpp:56);
                        // ObjectActionTumble does NOT (:74-79 — its
                        // DFA_FLIGHT procedure re-arms Mobile next
                        // ExecAction).
                        object.state.mobile = true;
                    }
                    object.fixed_velocity = FixedVec2::new(txdir, tydir);
                    object.refresh_velocity_from_fixed();
                    return;
                }
            }
        }
        let object = &mut self.objects[idx];
        object.fixed_velocity = FixedVec2::new(txdir, tydir);
        object.refresh_velocity_from_fixed();
        // The raw-velocity fallback mobilizes and unhooks the bottom attach
        // bit (C4Object.cpp:1647-1650). Keep both Rust mirrors in sync.
        object.state.mobile = true;
        object.state.t_attach &= !CNAT_BOTTOM;
        object.frame_t_attach &= !CNAT_BOTTOM;
    }

    pub(crate) fn apply_landscape_temperature_conversions(&mut self) {
        if self.materials.is_empty() {
            return;
        }
        let materials = &self.materials;
        let mass_movers = &mut self.mass_movers;
        if let Some(landscape) = self.landscape.as_mut() {
            landscape.apply_temperature_conversions_with(
                materials,
                self.environment.temperature,
                &mut |landscape, x, y| {
                    mass_movers.check_instability_range_for_landscape(landscape, materials, x, y);
                },
            );
        }
    }

    pub(crate) fn next_random_i32(&mut self) -> i32 {
        self.rng.random(i32::MAX)
    }

    pub(crate) fn next_object_id(&mut self) -> ObjectId {
        let id = self.next_object_id;
        self.next_object_id += 1;
        ObjectId::new(id)
    }

    /// Fold a script-world snapshot's id counter back into the engine.
    /// C4Game::NewObj mints strictly increasing numbers
    /// (`Number = ++ObjectEnumerationIndex`) — the allocator never
    /// rewinds within a session, so a snapshot counter that fell behind
    /// an interleaved engine-side spawn is stale and must be ignored.
    #[doc(hidden)]
    pub fn sync_next_object_id(&mut self, reported: u64) {
        self.next_object_id = self.next_object_id.max(reported);
    }

    /// C4Weather::Init's scenario evaluates (C4Weather.cpp:36-70): the
    /// synced-RNG init draws in exact order — Season, YearSpeed, Climate
    /// (100 - value - 50), Wind (= TargetWind), the NoInitialize-gated
    /// rain block (the gate Rain.Evaluate plus per-cloud
    /// Random(320)/Random(GBackWdt)/Rain.Evaluate and LaunchCloud),
    /// Lightning, then the Disasters
    /// (Meteorite/Volcano/Earthquake). Replaying these keeps the whole
    /// ledger aligned with C++ from frame 0.
    /// C4Landscape::ScenarioInit's Gravity draw (C4Landscape.cpp:66):
    /// `Gravity = FIXED100(Gravity.Evaluate()) / 5` — one synced ledger
    /// draw that precedes Weather.Init's evaluates.
    pub(crate) fn evaluate_scenario_gravity(
        &mut self,
        gravity: crate::scenario::LegacyC4SVal,
    ) -> i32 {
        gravity.evaluate(&mut self.rng)
    }

    #[doc(hidden)]
    pub fn apply_weather_init(
        &mut self,
        init: &crate::scenario::LegacyWeatherInit,
    ) -> Result<(), EngineError> {
        let season = init.season.evaluate(&mut self.rng);
        let year_speed = init.year_speed.evaluate(&mut self.rng);
        let climate = 100 - init.climate.evaluate(&mut self.rng) - 50;
        self.environment.set_legacy_wind_value(
            init.wind.std,
            init.wind.rnd,
            init.wind.min,
            init.wind.max,
        );
        let wind = init.wind.evaluate(&mut self.rng);
        // Evaluate already applies the scenario C4SVal bounds; C++ stores
        // those results directly without another hard-coded clamp.
        // These assignments precede LaunchCloud in C++ and are observable
        // from FXP1's synchronous Activate -> Movement callback through
        // GetWind/GetTemperature (C4Weather.cpp:40-48,55-58).
        self.environment.season = season;
        self.environment.season_min = init.season.min;
        self.environment.season_max = init.season.max;
        self.environment.year_speed = year_speed;
        self.environment.climate = climate;
        self.environment.temperature = climate;
        self.environment.wind = wind;
        self.environment.wind_target = wind;
        // These Rust-side precipitation fields previously arrived through
        // the eagerly installed scenario environment. Stage the same final
        // metadata here so pre-init callbacks see C4Weather::Default without
        // changing the established post-init EnvironmentSettings state.
        let rain_base = init.rain.base().clamp(-100, 100);
        self.environment.precipitation = rain_base;
        self.environment.precipitation_strength = rain_base;
        if !init.no_initialize {
            let rain = init.rain.evaluate(&mut self.rng);
            if rain != 0 {
                let width = self
                    .landscape
                    .as_ref()
                    .map(|landscape| landscape.width() as i32)
                    .unwrap_or(0);
                let clouds = (width / 500).min(5);
                for _ in 0..clouds.max(0) {
                    let cloud_width = width / 15 + self.rng.random(320);
                    let x = self.rng.random(width.max(1));
                    let strength = init.rain.evaluate(&mut self.rng);
                    let _ = self.launch_cloud(x, -1, cloud_width, strength, &init.precipitation)?;
                }
            }
            self.environment.precipitation = rain.clamp(0, 100) as u8 as i32;
        }
        let lightning = init.lightning.evaluate(&mut self.rng);
        let meteorite = init.meteorite.evaluate(&mut self.rng);
        let volcano = init.volcano.evaluate(&mut self.rng);
        let earthquake = init.earthquake.evaluate(&mut self.rng);

        self.environment.lightning = lightning;
        self.environment.meteorite = meteorite;
        self.environment.volcano = volcano;
        self.environment.earthquake = earthquake;
        // C++ assigns NoGamma only after every scenario-value evaluation and
        // cloud callback, immediately before SetSeasonGamma.
        self.environment.no_gamma = init.no_gamma;
        // C4Weather::Init calls SetSeasonGamma after all scenario weather
        // fields, including NoGamma, have been established (:65-69).
        self.refresh_season_gamma_control();
        Ok(())
    }

    /// C4Weather::LaunchCloud (C4Weather.cpp:205-215): resolve the
    /// precipitation material before object creation, create FXP1 with
    /// NO_OWNER at the requested point, and call Activate(mat,width,strength).
    /// The object remains alive even when Activate is missing or false.
    fn launch_cloud(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        strength: i32,
        precipitation: &str,
    ) -> Result<bool, EngineError> {
        const PRECIPITATION_DEFINITION: &str = "FXP1";
        let Some(material) = self.materials.id_of(precipitation) else {
            return Ok(false);
        };
        if !self.definitions.contains_key(PRECIPITATION_DEFINITION) {
            return Ok(false);
        }
        let cloud_id = self.spawn_object(
            SpawnConfig::new(PRECIPITATION_DEFINITION)
                .with_position(Vector2::new(x, y))
                .with_owner(OWNER_NONE),
        )?;
        let Some(index) = self.find_object_index(cloud_id) else {
            return Ok(false);
        };
        let value = tolerate_script_error(self.call_object_function(
            index,
            "Activate",
            vec![
                Value::Int(material.index() as i32),
                Value::Int(width),
                Value::Int(strength),
            ],
        ))?;
        Ok(value.as_ref().is_some_and(compat::value_raw_truthy))
    }

    /// Debug/test helper: (definition, action name, phase, position, fix)
    /// for one object id.
    #[allow(clippy::type_complexity)]
    pub fn debug_object_by_id(
        &self,
        id: u64,
    ) -> Option<(String, String, i32, Vector2, i32, i32, i32)> {
        self.objects
            .iter()
            .find(|object| object.id.as_u64() == id)
            .map(|object| {
                (
                    object.definition_id.clone(),
                    object.state.action.name.clone(),
                    object.state.action.phase,
                    object.state.position,
                    crate::math::fixtoi(object.fixed_position.y),
                    object.state.owner,
                    object.state.energy,
                )
            })
    }

    /// Debug/test helper: raw fixed-point motion state of an object —
    /// (xdir, ydir as 16.16 raw ints, mobile flag). Headless forensics for
    /// C4Fixed-resolution velocity questions (the snapshot only carries
    /// fixtoi pixels).
    pub fn debug_object_motion(&self, id: u64) -> Option<(i32, i32, bool)> {
        self.objects
            .iter()
            .find(|object| object.id.as_u64() == id)
            .map(|object| {
                (
                    object.fixed_velocity.x.val(),
                    object.fixed_velocity.y.val(),
                    object.state.mobile,
                )
            })
    }

    /// Debug/test helper: the action names + default of a definition's
    /// library.
    pub fn debug_action_library(&self, id: &str) -> Option<(String, Vec<String>)> {
        self.definitions.get(&DefinitionId::from(id)).map(|def| {
            let library = def.action_library();
            (
                library.default_action().to_string(),
                library.specs().keys().cloned().collect(),
            )
        })
    }

    /// `C4Game::SyncClearance` (`C4Game.cpp:3676-3680`): discard transient
    /// precision/contact state that the C++ save format does not preserve.
    fn game_sync_clearance(&mut self) {
        // PXS chunk consolidation is applied by `PxsSystem::sync_clearance`.
        // C4Game runs it before Objects.SyncClearance (C4Game.cpp:3676-3680).
        self.pxs_system.sync_clearance();
        for object in &mut self.objects {
            // C4GameObjects moves C4OS_INACTIVE rows to its separate
            // InactiveObjects list while loading. Objects.SyncClearance walks
            // only the main list, so dormant rows retain their saved fixed
            // state until StatusActivate re-adds them.
            if object.state.status == ObjectStatus::Inactive {
                continue;
            }
            // C4Object::SyncClearance (C4Object.cpp:3803-3823).
            object.state.t_attach = 0;
            object.frame_t_attach = 0;
            object.upright_t_attach = 0;
            object.frame_t_contact = 0;
            object.fixed_position = crate::math::FixedVec2 {
                x: itofix(object.state.position.x),
                y: itofix(object.state.position.y),
            };
            object.fixed_rotation = itofix(object.state.rotation);
            object.state.menu = None;
            object.material_contents.fill(0);
            if object.state.category & CATEGORY_STATIC_BACK != 0 {
                object.fixed_velocity = crate::math::FixedVec2::ZERO;
                object.state.velocity = Vector2::ZERO;
            }
        }
        // SetOCF is part of each C4Object::SyncClearance call. Recompute only
        // after every object has had its velocity cleared so cross-object OCF
        // probes observe the fully cleared state.
        for index in 0..self.objects.len() {
            if self.objects[index].state.status == ObjectStatus::Inactive {
                continue;
            }
            self.refresh_object_ocf(index);
        }
    }

    /// `C4Game::FixRandom`: reset the synced LCG to the game parameter seed
    /// and rebuild FRndBuf3. This leaves RandomCount at 500 and FRndPtr3 at
    /// zero (C4Game.cpp:3554-3558; C4Random.cpp:29-33).
    pub(crate) fn fix_random(&mut self) {
        self.rng = LcgRng::seed_from_u64(self.random_seed);
        self.rng.trace = std::env::var("LC_RUST_RNG_TRACE").is_ok();
    }

    /// Apply the mutable half of `C4PlayerList::SynchronizeLocalFiles` before
    /// the app writes profile groups: checkpoint player and active-crew play
    /// time, then refresh definition-owned rank fields.
    fn synchronize_local_player_file_state(&mut self) {
        let players = self
            .players
            .iter()
            .filter_map(|(&player_id, player)| {
                (!matches!(
                    player.status(),
                    PlayerStatus::Eliminated | PlayerStatus::Surrendered
                ) && !player.is_script_player())
                .then_some(player_id)
            })
            .collect::<HashSet<_>>();
        for player_id in &players {
            if let Some(player) = self.players.get_mut(player_id) {
                player.synchronize_playing_time(self.game_time);
            }
            if let Some(roster) = self.crew_rosters.get_mut(player_id) {
                for info in roster {
                    if info.in_action {
                        info.total_playing_time = info
                            .total_playing_time
                            .wrapping_add(self.game_time.wrapping_sub(info.in_action_time));
                        info.in_action_time = self.game_time;
                    }
                }
            }
        }
        let linked = self
            .crew_info_links
            .iter()
            .filter_map(|(&object_id, link)| {
                if !players.contains(&link.player_id) {
                    return None;
                }
                self.crew_rosters
                    .get(&link.player_id)
                    .and_then(|roster| roster.get(link.roster_index))
                    .map(|info| (object_id, info.total_playing_time, info.in_action_time))
            })
            .collect::<Vec<_>>();
        let live_infos = Rc::make_mut(&mut self.crew_object_infos);
        for (object_id, total_playing_time, in_action_time) in linked {
            if let Some(info) = live_infos.get_mut(&object_id) {
                info.total_playing_time = total_playing_time;
                info.in_action_time = in_action_time;
            }
        }
        self.refresh_crew_custom_ranks_for_save();
    }

    /// Checkpoint the mutable player and crew state written by
    /// `C4PlayerList::SynchronizeLocalFiles`. The application owns the
    /// physical `.c4p` groups and persists them immediately after this call.
    #[doc(hidden)]
    pub fn checkpoint_local_player_files_for_save(&mut self) {
        self.synchronize_local_player_file_state();
    }

    /// `C4ObjectInfoCore::Save(..., pDefs)` refreshes custom-rank fields
    /// immediately before every local crew-file write.
    fn refresh_crew_custom_ranks_for_save(&mut self) {
        let eligible_players = self
            .players
            .iter()
            .filter_map(|(&player_id, player)| {
                if matches!(
                    player.status(),
                    PlayerStatus::Eliminated | PlayerStatus::Surrendered
                ) || player.is_script_player()
                {
                    return None;
                }
                let remote = self
                    .local_players
                    .as_ref()
                    .is_some_and(|local| !local.contains(&player_id));
                let remote_save_blocked = remote
                    && (self.league_game
                        || self.max_players.is_some_and(|max_players| max_players <= 0));
                (!remote_save_blocked).then_some(player_id)
            })
            .collect::<HashSet<_>>();
        let mut saved_entries = HashSet::new();
        for player_id in &eligible_players {
            let Some(roster) = self.crew_rosters.get_mut(player_id) else {
                continue;
            };
            for (roster_index, info) in roster.iter_mut().enumerate() {
                let definition = self.definitions.get(&DefinitionId::from(info.id.as_str()));
                if definition.is_some_and(|definition| definition.temporary_crew != 0) {
                    continue;
                }
                saved_entries.insert((*player_id, roster_index));
                if let Some(definition) = definition {
                    update_custom_rank_fields(
                        &mut info.rank_name,
                        &mut info.core,
                        info.rank,
                        definition.rank_names(),
                        definition.rank_base(),
                    );
                }
            }
        }

        let linked = self
            .crew_info_links
            .iter()
            .filter_map(|(&object_id, &link)| {
                if !saved_entries.contains(&(link.player_id, link.roster_index)) {
                    return None;
                }
                self.crew_rosters
                    .get(&link.player_id)
                    .and_then(|roster| roster.get(link.roster_index))
                    .map(|entry| (object_id, entry.rank_name.clone(), entry.core.clone()))
            })
            .collect::<Vec<_>>();
        let live_infos = Rc::make_mut(&mut self.crew_object_infos);
        for (object_id, rank_name, core) in linked {
            if let Some(info) = live_infos.get_mut(&object_id) {
                info.rank_name = rank_name;
                info.core = core;
            }
        }
    }

    /// `C4Game::Synchronize` (`C4Game.cpp:3682-3715`). This deliberately
    /// does not perform `SyncClearance`; `C4ControlSynchronize::Execute`
    /// invokes clearance only when its `SyncClear` flag is set, and does so
    /// after synchronization (`C4Control.cpp:537-543`).
    fn game_synchronize_before_network(
        &mut self,
        save_player_files: bool,
    ) -> Result<(), EngineError> {
        // Objects.Synchronize resolves SetObjectOrder calls queued by
        // scenario/object initialization before InitPlayers (C4Game.cpp:3720;
        // C4GameObjects.cpp:250-260).
        self.resort_all_unsorted();
        self.execute_object_order_commands();
        self.fix_random();
        // Defs.Synchronize follows FixRandom in C4Game::Synchronize. Cache
        // refill stays lazy because its 21 hooks can consume synchronized RNG.
        self.clear_fair_crew_physicals();
        // C4Landscape::Synchronize resets the progressive material-scan
        // cursor before synchronized play resumes (C4Landscape.cpp:1662-1667).
        if let Some(landscape) = self.landscape.as_mut() {
            landscape.synchronize_temperature_scan();
        }
        // MassMover.Synchronize() (C4Game.cpp:3700): consolidate the slot
        // set and reset CreatePtr (C4MassMover.cpp:249-252).
        self.mass_movers.synchronize();
        // PXS.Synchronize() resets only the per-Execute Count ledger; live
        // chunk contents stay in place (C4PXS.cpp:401-404).
        self.pxs_system.synchronize();
        // Objects.Synchronize removes every put mask without causing
        // instability, then updates every active object's mask. Both passes
        // follow the C++ master list First -> Next; `exec_list` stores that
        // list reversed (C4GameObjects.cpp:254-261,296-311).
        self.synchronize_solid_masks();
        // The app owns the physical groups, but this deterministic state
        // checkpoint must precede its callback just like C4Player::LocalSync.
        if save_player_files && !self.replay_control {
            self.synchronize_local_player_file_state();
        }
        Ok(())
    }

    fn game_synchronize_after_network(&mut self) -> Result<(), EngineError> {
        // C4Game::Synchronize's tail: TransferZones.Synchronize()
        // broadcasts ~UpdateTransferZone to every active Game.Objects entry
        // AFTER the FixRandom re-fix (C4Game.cpp:3713-3714,3727-3729;
        // C4GameObjects.cpp:50-59; C4ObjectList.cpp:734-739). GoldRush's
        // placed cannon re-runs Initialize() here
        // (Cannon.c4d/Script.c:20-25) — SetAction Ready, SetDir(Random(2))
        // as the fresh ledger's first draw, and the GC4V crosshair as the
        // first created object.
        let ids: Vec<ObjectId> = self
            .objects
            .iter()
            .filter(|object| !object.destroyed && object.state.status.is_active())
            .map(|object| object.id)
            .collect();
        for id in ids {
            let Some(index) = self.find_object_index(id) else {
                continue;
            };
            let has_handler = self
                .definitions
                .get(&self.objects[index].definition_id)
                .map(|definition| definition.has_function("UpdateTransferZone"))
                .unwrap_or(false);
            if !has_handler {
                continue;
            }
            let _ = tolerate_script_error(
                self.call_object_function(index, "UpdateTransferZone", Vec::new())
                    .map(|_| ()),
            )?;
        }
        Ok(())
    }

    fn game_synchronize(&mut self, save_player_files: bool) -> Result<(), EngineError> {
        self.game_synchronize_before_network(save_player_files)?;
        self.game_synchronize_after_network()
    }

    /// Install the separately compiled C4ObjectList links once all numbered
    /// objects exist, preserving the exact saved Contents order.
    pub(crate) fn restore_legacy_contents_order(&mut self, orders: &[(ObjectId, Vec<ObjectId>)]) {
        self.restore_legacy_object_links(&[], orders);
    }

    /// Two-phase `Contained`/`Contents` denumeration for Objects.txt. The
    /// object graph is allowed to contain cycles; setting raw links after all
    /// objects exist avoids the runtime Enter path's sequential constraint.
    pub(crate) fn restore_legacy_object_links(
        &mut self,
        contained_links: &[(ObjectId, ObjectId)],
        orders: &[(ObjectId, Vec<ObjectId>)],
    ) {
        let active = self
            .objects
            .iter()
            .filter(|object| !object.destroyed && object.state.status != ObjectStatus::Deleted)
            .map(|object| object.id)
            .collect::<HashSet<_>>();

        for &(child, parent) in contained_links {
            if child != parent && active.contains(&child) && active.contains(&parent) {
                if let Some(child_index) = self.find_object_index(child) {
                    self.objects[child_index].state.container = Some(parent);
                }
            }
        }

        // C4ObjectList::DenumerateRead appends the saved links in their
        // compiled order. Its duplicate repair keeps the final occurrence.
        for (parent, children) in orders {
            let mut seen = HashSet::new();
            let mut normalized = children
                .iter()
                .rev()
                .copied()
                .filter(|child| *child != *parent && active.contains(child))
                .filter(|child| seen.insert(*child))
                .collect::<Vec<_>>();
            normalized.reverse();
            if let Some(parent_index) = self.find_object_index(*parent) {
                self.objects[parent_index].state.contents = normalized;
            }
        }

        // The saved Contents list is authoritative over a conflicting
        // Contained pointer. Writer-produced saves are consistent, but this
        // mirrors C4GameObjects::Load's repair pass for hand-edited files.
        for (parent, _) in orders {
            let Some(parent_index) = self.find_object_index(*parent) else {
                continue;
            };
            let children = self.objects[parent_index].state.contents.clone();
            for child in children {
                if let Some(child_index) = self.find_object_index(child) {
                    self.objects[child_index].state.container = Some(*parent);
                }
            }
        }

        // Conversely, a valid Contained pointer omitted from Contents is
        // appended after the compiled list (C4GameObjects.cpp:605-611).
        let contained = self
            .objects
            .iter()
            .filter(|object| active.contains(&object.id))
            .filter_map(|object| object.state.container.map(|parent| (object.id, parent)))
            .collect::<Vec<_>>();
        for (child, parent) in contained {
            if !active.contains(&parent) {
                if let Some(child_index) = self.find_object_index(child) {
                    self.objects[child_index].state.container = None;
                }
                continue;
            }
            if let Some(parent_index) = self.find_object_index(parent) {
                if !self.objects[parent_index].state.contents.contains(&child) {
                    self.objects[parent_index].state.contents.push(child);
                }
            }
        }
        for object in &mut self.objects {
            object.remember_compiled_mass_contents();
        }
    }

    /// Complete the pointer/list/sector half of `C4GameObjects::Load` before
    /// any `InitializeDef` or environment-placement callback can query
    /// objects. Objects.txt is execution-order and C++ rebuilds its main list
    /// with `stReverse`; rebuild sectors once at the equivalent seam.
    pub(crate) fn finish_legacy_object_load(&mut self) {
        // C4GameObjects::Load compiles every enumerated pointer as a number,
        // then calls C4Object::DenumeratePointers for the complete list before
        // any InitializeDef/scenario callback. Missing ActionTarget1/2 values
        // therefore become null and can never leak as dangling ObjectIds into
        // those callbacks (C4GameObjects.cpp:600-608; C4Object.cpp:2914-2937).
        let object_numbers = self
            .objects
            .iter()
            .map(|object| object.id.as_u64())
            .collect::<HashSet<_>>();
        let object_definition_ids = self
            .objects
            .iter()
            .map(|object| (object.id.as_u64(), object.definition_id.clone()))
            .collect::<HashMap<_, _>>();
        for object in &mut self.objects {
            denumerate_legacy_enumerated_object_reference(
                &mut object.state.action.target,
                &object_numbers,
            );
            denumerate_legacy_enumerated_object_reference(
                &mut object.state.action.target2,
                &object_numbers,
            );
            denumerate_legacy_enumerated_object_reference(&mut object.state.layer, &object_numbers);
            denumerate_legacy_enumerated_object_reference(
                &mut object.state.container,
                &object_numbers,
            );
            for value in object.state.local_vars.values_mut() {
                *value = denumerate_script_value(value, &object_numbers);
            }
            for effect in &mut object.state.effects {
                denumerate_loaded_effect(effect, &object_numbers, &object_definition_ids);
            }
            for overlay in &mut object.state.graphics_overlays {
                denumerate_legacy_enumerated_object_reference(
                    &mut overlay.overlay_object,
                    &object_numbers,
                );
            }
            object
                .commands
                .denumerate_object_references(&object_numbers);
        }
        let refreshed_ocf = self
            .objects
            .iter()
            .map(|object| {
                let contents_count = self.retained_contents_count(&object.state.contents);
                self.definitions
                    .get(&object.definition_id)
                    .map(|definition| {
                        definition.compute_ocf_with_contents_count(&object.state, contents_count)
                    })
                    .unwrap_or(OCF_NORMAL)
            })
            .collect::<Vec<_>>();
        for (object, ocf) in self.objects.iter_mut().zip(refreshed_ocf) {
            object.state.ocf = ocf;
        }
        self.rebuild_sectors();
        self.fix_exec_list_order();
        // C4GameObjects::Load's misc-updates loop runs after FixObjectOrder
        // and calls UpdateFlipDir once per loaded object — "for old
        // objects.txt with no flipdir defined" (C4GameObjects.cpp:665-674).
        // Runtime CreateObject deliberately never gets this pass.
        for index in 0..self.objects.len() {
            if self.objects[index].state.status.is_active() {
                self.update_object_flip_dir(index);
            }
        }
    }

    /// Execute the body of `C4ControlSynchronize` in C++ order: synchronize
    /// first, then optionally clear no-save state (`C4Control.cpp:537-543`).
    pub fn execute_synchronize_control(
        &mut self,
        save_player_files: bool,
        sync_clearance: bool,
    ) -> Result<(), EngineError> {
        self.execute_synchronize_control_before_network(save_player_files)?;
        self.execute_synchronize_control_after_network(sync_clearance)
    }

    /// Execute `C4Game::Synchronize` through the point immediately before
    /// `C4Network2::OnGameSynchronized`. A runtime network dynamic must be
    /// captured after this returns and before
    /// [`Self::execute_synchronize_control_after_network`] is called.
    #[doc(hidden)]
    pub fn execute_synchronize_control_before_network(
        &mut self,
        save_player_files: bool,
    ) -> Result<(), EngineError> {
        self.game_synchronize_before_network(save_player_files)
    }

    /// Finish `C4Game::Synchronize` after the network callback, then apply
    /// `C4ControlSynchronize::fSyncClearance` in native packet order.
    #[doc(hidden)]
    pub fn execute_synchronize_control_after_network(
        &mut self,
        sync_clearance: bool,
    ) -> Result<(), EngineError> {
        self.game_synchronize_after_network()?;
        if sync_clearance {
            self.game_sync_clearance();
        }
        Ok(())
    }

    /// C4Game::Init tail (C4Game.cpp:473-475): SyncClearance followed by
    /// Synchronize(false), after InitGame and before InitPlayers.
    #[doc(hidden)]
    pub fn game_start_synchronize(&mut self) -> Result<(), EngineError> {
        self.game_sync_clearance();
        self.game_synchronize(false)
    }

    /// Debug helper: does a definition's compiled script define `name`?
    pub fn debug_definition_has_function(&self, id: &str, name: &str) -> Option<bool> {
        self.definitions
            .get(&DefinitionId::from(id))
            .map(|definition| definition.has_function(name))
    }

    /// Applies the C4Def DefCore metadata onto a compiled definition —
    /// shared by `from_resource` and the legacy scenario loader so no
    /// core field is silently dropped (physicals/Float/Timer/Grab were).
    #[doc(hidden)]
    pub fn apply_resource_core(
        definition: &mut Definition,
        core: &clonk_resources::definition::DefCore,
    ) {
        definition.set_name(core.name.clone().unwrap_or_else(|| "Undefined".to_string()));
        definition.set_version(core.version);
        definition.require_defs = core.require_defs.clone();
        definition.set_crew_member_value(core.crew_member);
        definition.no_standard_crew = core.no_standard_crew;
        definition.set_silent_commands(core.silent_commands);
        definition.set_category(core.category);
        definition.max_user_select = core.max_user_select;
        definition.set_blit_mode(core.blit_mode);
        definition.set_color_by_owner(core.color_by_owner);
        definition.color_by_material = core.color_by_material.clone();
        definition.set_allow_picture_stack(core.allow_picture_stack);
        definition.set_graphics_scale(core.graphics_scale as f32 / 100.0);
        definition.set_value(core.value);
        definition.set_no_sell(core.no_sell);
        definition.set_rebuyable(core.rebuyable);
        definition.set_base_auto_sell(core.base_auto_sell);
        definition.set_mass(core.mass);
        definition.set_picture(core.picture.map(DefinitionPicture::from));
        definition.set_solid_mask(core.solid_mask.map(DefinitionTargetRect::from));
        definition.set_top_face(core.top_face.map(DefinitionTargetRect::from));
        definition.set_shape_rect(core.shape.map(DefinitionRect::from));
        definition.set_fire_top(core.fire_top);
        definition.set_lift_top(core.lift_top);
        definition.set_shape_vertex_slots(
            core.vertices.len(),
            &core
                .vertex_slots
                .iter()
                .map(|vertex| {
                    ObjectVertex::new(vertex.x, vertex.y)
                        .with_cnat(vertex.cnat)
                        .with_friction(vertex.friction)
                })
                .collect::<Vec<_>>(),
        );
        definition.set_contact_density(core.contact_density);
        definition.set_contact_function_calls(core.contact_function_calls);
        definition.set_collection_rect(core.collection.map(DefinitionRect::from));
        definition.set_collection_limit(core.collection_limit);
        definition.set_fragile(core.fragile);
        definition.set_projectile(core.projectile);
        definition.explosive = core.explosive;
        definition.set_entrance_rect(core.entrance.map(DefinitionRect::from));
        definition.set_rotated_entrance(core.rotated_entrance);
        definition.set_fire_properties(
            core.contact_incinerate,
            core.no_burn_decay,
            core.no_burn_damage,
        );
        definition.set_blast_incinerate(core.blast_incinerate);
        definition.set_contain_blast(core.contain_blast);
        definition.set_closed_container(core.closed_container);
        definition.set_no_horizontal_move(core.no_horizontal_move);
        definition.set_burn_turn_to(core.burn_turn_to.clone());
        definition.set_build_turn_to(core.build_turn_to.clone());
        definition.set_incomplete_activity(core.incomplete_activity);
        definition.set_no_breath(core.no_breath);
        definition.temporary_crew = core.temporary_crew;
        definition.smoke_rate = core.smoke_rate;
        definition.set_grab(core.grab);
        definition.set_move_to_range(core.move_to_range);
        definition.set_pathfinder(core.pathfinder);
        definition.set_no_transfer_zones(core.no_transfer_zones);
        definition.set_no_push_enter(core.no_push_enter);
        definition.drag_image_picture = core.drag_image_picture;
        definition.float_line = core.float_line;
        definition.set_line(core.line);
        definition.set_line_intersect(core.line_intersect);
        definition.set_physical(core.physical);
        definition.set_collectible(core.collectible);
        definition.set_no_get(core.no_get != 0);
        definition.set_grab_put_get(core.grab_put_get);
        definition.set_vehicle_control(core.vehicle_control);
        definition.set_constructable(core.constructable);
        definition.set_can_be_base(core.can_be_base);
        definition.set_construction_offset(core.con_size_off);
        definition.set_stretch_growth(core.stretch_growth);
        definition.set_oversize(core.oversize);
        definition.set_placement(core.placement);
        definition.set_growth(core.growth);
        definition.set_basement(core.basement);
        definition.set_rotateable(core.rotateable);
        definition.set_border_bound(core.border_bound);
        definition.set_upright_attach(core.upright_attach);
        definition.set_rotated_solid_masks(core.rotated_solid_masks);
        definition.set_auto_context_menu(core.auto_context_menu);
        definition.needed_gfx_mode = core.needed_gfx_mode;
        definition.set_no_component_mass(core.no_component_mass);
        definition.set_no_stabilize(core.no_stabilize);
        definition.hide_hud_bars = core.hide_hud_bars;
        definition.hide_hud_elements = core.hide_hud_elements;
        definition.set_timer(core.timer);
        definition.set_timer_call(core.timer_call.clone());
        if !core.components.is_empty() {
            definition.set_components(
                core.components
                    .iter()
                    .map(|component| DefinitionComponent {
                        id: component.id.clone(),
                        count: component.count,
                    })
                    .collect(),
            );
        }
        definition.set_line_connect(core.line_connect);
        definition.set_exclusive(core.exclusive);
        definition.set_edible(core.edible);
        definition.set_prey(core.prey);
        definition.set_attract_lightning(core.attract_lightning);
        definition.set_no_fight(core.no_fight);
        definition.set_chopable(core.chopable);
        definition.def_core_reflected_ints = core.reflected_ints.clone();
    }

    /// Exact callback-entry projection of every C4ObjectInfoList. The host
    /// consumes idle entries synchronously and allocates stable roster-index
    /// links for new infos before the callback outcome reaches Engine.
    pub(crate) fn host_crew_info_state(&self) -> compat::HostCrewInfoState {
        let mut state = compat::HostCrewInfoState::default();
        state.control_counts = self.crew_info_control_counts.clone();
        for (&number, roster) in &self.crew_rosters {
            state.next_indices.insert(number, roster.len());
            state.roster_names.insert(
                number,
                roster.iter().map(|info| info.name.clone()).collect(),
            );
            for (roster_index, info) in roster.iter().enumerate() {
                let link = CrewInfoLink {
                    player_id: number,
                    roster_index,
                };
                state.entries.insert(link, info.clone());
            }
            let fallback_order;
            let order = match self.crew_info_order.get(&number) {
                Some(order) => order.as_slice(),
                None => {
                    fallback_order = (0..roster.len()).collect::<Vec<_>>();
                    fallback_order.as_slice()
                }
            };
            for &roster_index in order {
                let Some(info) = roster.get(roster_index) else {
                    continue;
                };
                let link = CrewInfoLink {
                    player_id: number,
                    roster_index,
                };
                state.order.entry(number).or_default().push(link);
                if info.participation != 0
                    && !info.in_action
                    && !info.has_died
                    && self
                        .definitions
                        .contains_key(&DefinitionId::from(info.id.as_str()))
                {
                    state
                        .idle
                        .entry((number, info.id.clone()))
                        .or_default()
                        .push((link, info.clone()));
                }
            }
        }
        state
    }

    /// Compatibility seam for the scenario-load tail. ClonkNames now follow
    /// C4Def::IncludeDefinition inside [`Self::resolve_includes`], alongside
    /// rank metadata and in the same push-front include order.
    pub fn inherit_include_clonk_names(&mut self) {
        // The work is complete once resolve_includes returns.
    }

    /// Debug helper: a definition's shape rect.
    pub fn debug_definition_shape(&self, id: &str) -> Option<DefinitionRect> {
        self.definitions
            .get(&DefinitionId::from(id))
            .and_then(|definition| definition.shape_rect())
    }

    /// Debug helper: landscape solidity probe.
    /// Debug helper: an object's position in the exec vector.
    pub fn debug_object_vector_index(&self, id: u64) -> Option<usize> {
        self.objects
            .iter()
            .position(|object| object.id.as_u64() == id)
    }

    /// Debug helper: an action's ActMap Attach bits.
    pub fn debug_action_attach(&self, id: &str, action: &str) -> Option<u32> {
        self.definitions
            .get(&DefinitionId::from(id))
            .map(|def| def.action_library().attach_for_action(action))
    }

    /// Test helper: arm/disarm an object's InLiquid flag (fixtures
    /// without water need it so the DFA_SWIM out-of-liquid exit stays
    /// quiet).
    pub fn debug_set_in_liquid(&mut self, id: ObjectId, in_liquid: bool) {
        if let Some(idx) = self.find_object_index(id) {
            self.objects[idx].state.in_liquid = in_liquid;
        }
    }

    /// Debug helper: landscape liquid probe.
    /// Debug helper: raw density probe.
    pub fn debug_landscape_density(&self, x: i32, y: i32) -> Option<i32> {
        self.landscape
            .as_ref()
            .map(|landscape| landscape.density_at(x, y, &self.materials))
    }

    /// Debug helper: raw grid byte at a pixel.
    /// Debug helper: the resolved material NAME at a pixel.
    pub fn debug_landscape_material_name(&self, x: i32, y: i32) -> Option<String> {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.material_at(x, y))
            .and_then(|id| self.materials.get_by_id(id))
            .map(|material| material.name().to_string())
    }
}
