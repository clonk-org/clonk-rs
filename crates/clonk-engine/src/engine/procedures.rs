//! `impl Engine` — dig, bridge, build, action, attach, pull, push and fight.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub(crate) fn apply_lift_to_target(
        &mut self,
        lifter_idx: usize,
        command_direction: CommandDirection,
        action_target: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        let lifter_id = self.objects[lifter_idx].id;
        let target_id = match action_target {
            Some(id) => id,
            None => return Ok(false),
        };
        let target_idx = match self.find_object_index(target_id) {
            Some(idx) => idx,
            None => return Ok(false),
        };
        if self.objects[target_idx].destroyed
            || self.objects[target_idx].state.status == ObjectStatus::Deleted
            || self.objects[target_idx].state.container.is_some()
            || !self
                .definitions
                .contains_key(&self.objects[target_idx].definition_id)
        {
            return Ok(false);
        }

        let gravity = self.physics.gravity_as_c4fixed();
        let desired_velocity = match command_direction {
            CommandDirection::Up => -itofix(2),
            CommandDirection::Stop => -gravity,
            CommandDirection::Down => itofix(2),
            _ => C4Fixed::ZERO,
        };
        // C4Object::Lift divides the constant 0.5 force by the LIVE object
        // mass, including construction, OwnMass and (normally) contents.
        let lift_force = math::fixed100(50) * 100 / self.effective_object_mass(target_idx).max(1);

        {
            let object = &mut self.objects[target_idx];
            // C4Object::Lift pre-mobilization (C4Object.cpp:1841-1845):
            // zero dirs, snap fix to the pixel position, mobilize.
            if !object.state.mobile {
                object.fixed_velocity = FixedVec2::ZERO;
                object.fixed_position = FixedVec2::new(
                    itofix(object.state.position.x),
                    itofix(object.state.position.y),
                );
                object.state.mobile = true;
            }
            math::towards(&mut object.fixed_velocity.y, desired_velocity, lift_force);
            object.refresh_velocity_from_fixed();
        }

        // Lift's stuck probe is unthrottled. The exact -GravAccel hold is
        // the sole bypass, even for noncanonical ComDir values that happen
        // to request the same raw fixed velocity.
        if desired_velocity != -gravity {
            let position = self.objects[target_idx].state.position;
            let contacted = self
                .object_contact_check_at(target_idx, position)?
                .is_some_and(|contact| contact.is_contact());
            if contacted {
                // Contact callbacks above may have changed or removed the
                // target. C++ queues the object message before ~Stuck.
                if let Some(target_idx) = self.find_object_index(target_id).filter(|&index| {
                    !self.objects[index].destroyed
                        && self.objects[index].state.status != ObjectStatus::Deleted
                }) {
                    let object = &self.objects[target_idx];
                    let name = object
                        .state
                        .custom_name
                        .clone()
                        .or_else(|| {
                            self.crew_object_infos
                                .get(&target_id)
                                .map(|info| info.name.clone())
                        })
                        .or_else(|| {
                            self.definitions
                                .get(&object.definition_id)
                                .map(|definition| definition.name().to_string())
                        })
                        .unwrap_or_else(|| object.definition_id.clone());
                    self.messages
                        .add_message(MessageSpec::target(format!("{name} is stuck!"), target_id));
                    let callback_definition_id = self.objects[target_idx].definition_id.clone();
                    if let Some(action_library) = self
                        .definitions
                        .get(&callback_definition_id)
                        .map(Definition::shared_action_library_handle)
                    {
                        let _ = tolerate_script_error(self.call_movement_object_function(
                            target_idx,
                            "Stuck",
                            &[],
                            &action_library,
                            target_id,
                            &callback_definition_id,
                        ))?;
                    }
                }
            }
        }

        // Re-read the lifter's live action/definition after target Contact
        // and Stuck callbacks. LiftTop is level-triggered on the lifter and
        // runs before the caller's trailing DoGravity.
        if let Some(lifter_idx) = self.find_object_index(lifter_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) {
            let object = &self.objects[lifter_idx];
            let live_target = object.state.action.target;
            let definition_id = object.definition_id.clone();
            let lift_top = self
                .definitions
                .get(&definition_id)
                .map(Definition::lift_top)
                .unwrap_or(0);
            let should_call = lift_top != 0
                && object.state.command_direction == CommandDirection::Up
                && live_target
                    .and_then(|id| self.find_object_index(id))
                    .is_some_and(|index| {
                        self.objects[index].state.position.y
                            <= object.state.position.y.wrapping_add(lift_top)
                    });
            if should_call {
                if let Some(action_library) = self
                    .definitions
                    .get(&definition_id)
                    .map(Definition::shared_action_library_handle)
                {
                    let _ = tolerate_script_error(self.call_movement_object_function(
                        lifter_idx,
                        "LiftTop",
                        &[],
                        &action_library,
                        lifter_id,
                        &definition_id,
                    ))?;
                }
            }
        }

        Ok(true)
    }

    pub(crate) fn apply_dig_procedure(&mut self, idx: usize, definition_id: &DefinitionId) {
        let (action_name, action_index, predicted, requested, object_id, construction, shape_rect) = {
            let object = &self.objects[idx];
            (
                object.state.action.name.clone(),
                object.state.action.act_map_index,
                Vector2::new(
                    math::fixtoi(object.fixed_position.x + object.fixed_velocity.x),
                    math::fixtoi(object.fixed_position.y + object.fixed_velocity.y),
                ),
                object.state.action.data != 0,
                object.id,
                object.state.construction,
                object.current_shape_rect(),
            )
        };

        let Some(action_library) = self
            .definitions
            .get(definition_id)
            .map(Definition::action_library)
        else {
            return;
        };
        if action_library.is_idle_entry(&action_name, action_index) {
            return;
        }
        let Some(dig_free_value) = action_library.dig_free_for_entry(&action_name, action_index)
        else {
            return;
        };

        if dig_free_value <= 0 {
            return;
        }

        if dig_free_value == 1 {
            let Some(shape_rect) = shape_rect else {
                return;
            };
            self.execute_dig_rect_operation(
                Vector2::new(
                    predicted.x.saturating_add(shape_rect.x),
                    predicted.y.saturating_add(shape_rect.y),
                ),
                shape_rect.width,
                shape_rect.height,
                requested,
                Some(object_id),
            );
        } else {
            let mut radius = dig_free_value;
            if construction < FULL_CON {
                radius = radius * 6 * construction / 5 / FULL_CON;
            }
            self.execute_dig_circle_operation(
                Vector2::new(predicted.x, predicted.y.saturating_sub(1)),
                radius,
                requested,
                Some(object_id),
            );
        }
    }

    pub(crate) fn dig_column(
        materials: &MaterialSet,
        landscape: &mut Landscape,
        column: i32,
        target_height: i32,
    ) -> Option<(MaterialId, i32)> {
        let width = landscape.width() as i32;
        if column < 0 || width == 0 || column >= width {
            return None;
        }

        if materials.is_empty() {
            landscape.ensure_surface_at_least(column, target_height);
            return None;
        }

        let previous_height = landscape.surface_height(column).unwrap_or(0);
        let Some(material_id) = landscape.solid_material_at(column) else {
            return None;
        };
        let Some(material) = materials.get_by_id(material_id) else {
            return None;
        };
        if !material.dig_free() {
            return None;
        }
        let clamped_target = target_height.max(0);
        let desired_target = if clamped_target <= previous_height {
            let one_beyond = clamped_target.saturating_add(1);
            if one_beyond <= previous_height {
                return None;
            }
            previous_height.saturating_add(1)
        } else {
            clamped_target
        };

        landscape.ensure_surface_at_least(column, desired_target);
        let new_height = landscape.surface_height(column).unwrap_or(previous_height);
        let removed = new_height.saturating_sub(previous_height);
        if removed <= 0 {
            return None;
        }

        Some((material_id, removed))
    }

    pub(crate) fn apply_bridge_procedure(
        &mut self,
        idx: usize,
        command_direction: CommandDirection,
        definition_id: &DefinitionId,
    ) -> Result<bool, EngineError> {
        let parameters = BridgeParameters::from_action_data(self.objects[idx].state.action.data);
        let action_time = self.objects[idx].state.action.time;

        if action_time >= parameters.duration {
            // ObjectActionStand runs inside DoBridge, before DFA_BRIDGE can
            // OR in CNAT_Bottom for this frame (C4Object.cpp:4584,
            // 4996-5006). Preserve only the earlier UprightAttach arm.
            let upright_attach = self.objects[idx].upright_t_attach;
            self.objects[idx].frame_t_attach = upright_attach;
            self.objects[idx].state.t_attach = upright_attach;
            let _ = self.object_action_stand(idx, definition_id)?;
            return Ok(false);
        }

        let Some(step_interval) = parameters.step_interval(command_direction) else {
            return Ok(true);
        };
        // DoBridge overrides the action-data flag for vertical walls and
        // diagonal roofs; only the wall-Up arm may move the Clonk
        // (C4Object.cpp:4587-4592).
        let move_clonk = parameters.move_clonk
            && !(parameters.wall
                && matches!(
                    command_direction,
                    CommandDirection::Left
                        | CommandDirection::Right
                        | CommandDirection::UpLeft
                        | CommandDirection::UpRight
                ));

        if step_interval == 0 || action_time % step_interval != 0 {
            return Ok(true);
        }

        let (base_position, shape_width, shape_height, facing) = {
            let object = &self.objects[idx];
            let shape = object.current_shape_rect();
            (
                object.state.position,
                shape.map(|rect| rect.width).unwrap_or(0),
                shape.map(|rect| rect.height).unwrap_or(0),
                object.state.direction,
            )
        };
        let mut clonk_x = base_position.x;
        let mut clonk_y = base_position.y;
        let mut target_x = base_position.x;
        let mut target_y = base_position.y + shape_height / 2;
        let delta_time = if move_clonk {
            0
        } else {
            action_time / step_interval
        };

        // The target formulas are the DoBridge switch verbatim
        // (C4Object.cpp:4605-4630). Keeping all arms here prevents the
        // Tutorial-2 UpLeft path from inheriting the old x-only shortcut.
        if parameters.wall {
            match command_direction {
                CommandDirection::Left => {
                    target_x -= shape_width / 2;
                    target_y -= delta_time;
                }
                CommandDirection::Right => {
                    target_x += shape_width / 2;
                    target_y -= delta_time;
                }
                CommandDirection::Up => {
                    let x0 = if move_clonk {
                        -3
                    } else {
                        (parameters.duration / step_interval) / -2
                    };
                    let direction = if facing == Direction::Right { 1 } else { -1 };
                    target_x += (x0 + delta_time) * direction;
                    clonk_x += direction;
                    target_y -= shape_height + 3;
                }
                CommandDirection::UpLeft => {
                    target_x -= -4 + delta_time;
                    target_y += -shape_height - 7 + delta_time;
                }
                CommandDirection::UpRight => {
                    target_x += -4 + delta_time;
                    target_y += -shape_height - 7 + delta_time;
                }
                _ => return Ok(true),
            }
        } else {
            match command_direction {
                CommandDirection::Left => {
                    target_x += -2 - delta_time;
                    clonk_x -= 1;
                }
                CommandDirection::Right => {
                    target_x += 2 + delta_time;
                    clonk_x += 1;
                }
                CommandDirection::Up => {
                    let stationary = i32::from(!move_clonk);
                    target_x += (-shape_width / 2
                        + (shape_width - 1) * i32::from(facing == Direction::Right))
                        * stationary;
                    target_y += -delta_time - i32::from(move_clonk);
                    clonk_y -= 1;
                }
                CommandDirection::UpLeft => {
                    target_x += -5 - delta_time + i32::from(move_clonk) * 3;
                    target_y += 2 - delta_time - i32::from(move_clonk) * 3;
                    clonk_x -= 1;
                    clonk_y -= 1;
                }
                CommandDirection::UpRight => {
                    target_x += 5 + delta_time - i32::from(move_clonk) * 2;
                    target_y += 2 - delta_time - i32::from(move_clonk) * 3;
                    clonk_x += 1;
                    clonk_y -= 1;
                }
                _ => return Ok(true),
            }
        }

        // Shape.CheckContact(cx, cy-1) runs before drawing. A blocked moving
        // bridge rewrites the remaining bridge as stationary and immediately
        // retries at Action.Time 0 (C4Object.cpp:4631-4646).
        if move_clonk && self.object_shape_contacts_at(idx, Vector2::new(clonk_x, clonk_y - 1)) {
            let mut remaining = parameters.duration.saturating_sub(action_time);
            let mut retry_time = 0;
            if parameters.wall && command_direction == CommandDirection::Up {
                retry_time = remaining;
                remaining = remaining.saturating_mul(2).min(0xffff);
            }
            let material = parameters
                .material
                .map(|material| material.index() as i32)
                .unwrap_or(-1);
            let object = &mut self.objects[idx];
            object.state.action.time = retry_time;
            object.state.action.data =
                encode_bridge_action_data(remaining, false, parameters.wall, material);
            return self.apply_bridge_procedure(idx, command_direction, definition_id);
        }

        if let Some(material) = parameters.material {
            self.draw_material_rect(material, target_x - 2, target_y, 4, 3);
        }

        if move_clonk {
            // C4Object::MovePosition removes/re-puts the solid mask with rider
            // backup, adds integer deltas to the true fixed coordinates, and
            // updates sectors (C4Movement.cpp:547-556).
            let attachments = self.remove_solid_mask_for_movement(idx);
            let object = &mut self.objects[idx];
            object.state.position = Vector2::new(clonk_x, clonk_y);
            object.fixed_position.x += itofix(clonk_x - base_position.x);
            object.fixed_position.y += itofix(clonk_y - base_position.y);
            self.update_sector_for_index(idx);
            self.update_solid_mask(idx);
            self.restore_solid_mask_attachments(idx, attachments);
        }

        Ok(true)
    }

    /// C4Landscape::DrawMaterialRect (C4Landscape.cpp:1064-1072): direct
    /// Surface8 writes, with density first and DigFree as the equal-density
    /// tie-break. SetPix preserves the destination IFT bit. Unlike script
    /// DrawMaterialQuad this deliberately does not enter PrepareChange.
    pub(crate) fn draw_material_rect(
        &mut self,
        material: MaterialId,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        let Some(bridge) = self.materials.get_by_id(material) else {
            return;
        };
        let bridge_density = bridge.density();
        let bridge_dig_free = i32::from(bridge.dig_free());
        let materials = &self.materials;
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };

        for target_y in y..y.saturating_add(height) {
            for target_x in x..x.saturating_add(width) {
                let current_density = landscape.density_at(target_x, target_y, materials);
                let current_dig_free = landscape
                    .border_material_at(target_x, target_y)
                    .and_then(|current| materials.get_by_id(current))
                    .map(|current| i32::from(current.dig_free()))
                    // MatDigFree(MNone) is 1 (C4Wrappers.h:153-157).
                    .unwrap_or(1);
                if bridge_density > current_density
                    || (bridge_density == current_density && bridge_dig_free <= current_dig_free)
                {
                    let _ = landscape.insert_material_pix(target_x, target_y, material);
                }
            }
        }
    }

    pub(crate) fn apply_build_procedure(&mut self, idx: usize) -> Result<bool, EngineError> {
        let builder_id = self.objects[idx].id;
        let category = self.objects[idx].state.category;
        let is_structure = (category & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK)) != 0;

        let target_id = match self.objects[idx].state.action.target {
            Some(id) => id,
            None => {
                if is_structure {
                    return Ok(true);
                }
                let _ = self.object_com_stop_build(builder_id)?;
                return Ok(false);
            }
        };

        let target_idx = match self.find_object_index(target_id) {
            Some(index) if index != idx => index,
            _ => {
                let _ = self.object_com_stop_build(builder_id)?;
                return Ok(false);
            }
        };

        // An internal construction is supported only while its live
        // container is itself building and has power (C4Object.cpp:
        // 5016-5020). This bare return deliberately precedes both the area
        // and completed-target checks and does not stop the builder.
        if let Some(container_id) = self.objects[target_idx].state.container {
            let supported = self
                .find_object_index(container_id)
                .is_some_and(|container_idx| {
                    let container = &self.objects[container_idx];
                    !container.state.need_energy
                        && self.definitions.get(&container.definition_id).is_some_and(
                            |definition| {
                                definition.action_library().procedure_for_entry(
                                    &container.state.action.name,
                                    container.state.action.act_map_index,
                                ) == ActionProcedure::Build
                            },
                        )
                });
            if !supported {
                return Ok(false);
            }
        }

        // C++ tests the builder's integer position against the target's
        // current C4Shape, with inclusive Inside bounds. A definition-less
        // shape is the native zero rectangle, not the expanded sector area.
        let builder_position = self.objects[idx].state.position;
        let target_position = self.objects[target_idx].state.position;
        let shape = self.objects[target_idx]
            .current_shape_rect()
            .unwrap_or_default();
        let dx =
            i64::from(builder_position.x) - (i64::from(target_position.x) + i64::from(shape.x));
        let dy =
            i64::from(builder_position.y) - (i64::from(target_position.y) + i64::from(shape.y));
        let in_target_area = dx >= 0
            && dx <= i64::from(shape.width)
            && dy >= -16
            && dy <= i64::from(shape.height) + 16;
        if !in_target_area {
            let _ = self.object_com_stop_build(builder_id)?;
            return Ok(false);
        }

        if !self.objects[target_idx].has_nonzero_status()
            || self.objects[target_idx].state.construction >= FULL_CON
        {
            // Target::Build returns false once full. The common failure tail
            // stops first and then may SetCommand(Exit) on an internal target.
            self.finish_failed_build(builder_id)?;
            return Ok(false);
        }

        // DFA_BUILD chooses the external/internal level before entering
        // Target::Build. GetCustomComponents may recontain the target, but
        // that side effect only changes the next tick's level.
        let level = if self.objects[target_idx].state.container.is_some() {
            1
        } else {
            10
        };
        let target_definition_id = self.objects[target_idx].definition_id.clone();
        let need_material = self.construction_needs_material
            || (self.objects[target_idx].state.category
                & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK))
                == 0;
        let required_components = if need_material {
            self.build_required_components(&target_definition_id, builder_id)?
        } else {
            Vec::new()
        };

        let missing_component = if need_material {
            self.ensure_build_components(idx, target_idx, &required_components)?
        } else {
            None
        };
        if let Some((missing_component, missing_count)) = missing_component {
            // C4Object::Build lets the builder override missing-material
            // handling after the component-grab pass. Script errors are
            // fail-safe/falsy like C4Object::Call (C4Object.cpp:1734-1748).
            let handled = tolerate_script_error(self.call_object_function(
                idx,
                "BuildNeedsMaterial",
                vec![
                    Value::C4Id(missing_component.clone()),
                    Value::Int(missing_count),
                ],
            ))?
            .is_some_and(|value| compat::value_raw_truthy(&value));

            if !handled {
                // The callback may have changed the builder's controller,
                // OCF, commands, or definition, so re-resolve its live state.
                if let Some(builder_idx) = self.find_object_index(builder_id).filter(|&index| {
                    !self.objects[index].destroyed
                        && !matches!(self.objects[index].state.status, ObjectStatus::Deleted)
                }) {
                    if self.objects[builder_idx].state.ocf & ocf::CREW_MEMBER != 0 {
                        // AddCommand(Acquire) is a retrying SilentSub in front
                        // of the retained Build order.
                        let acquire = CommandRequest::new(CommandId::Acquire)
                            .with_data(CommandData::Text(missing_component))
                            .with_update_interval(50)
                            .with_retries(1)
                            .with_mode(CommandMode::SilentSub);
                        self.objects[builder_idx]
                            .apply_command_operations([CommandOperation::PushFront(acquire)]);
                    }

                    // C4Object::GetNeededMatStr runs after the callback and
                    // may itself call GetCustomComponents with the builder.
                    // Evaluate the existing compatibility host in the
                    // builder's context so it sees the same live state.
                    let expression = format!("GetNeededMatStr(Object({target_id}))");
                    let text = tolerate_script_error(self.direct_exec_on_object(
                        builder_idx,
                        &expression,
                        "Build:GetNeededMatStr",
                    ))?
                    .and_then(|value| match value {
                        Value::String(text) => Some(text.into_string()),
                        _ => None,
                    });
                    if let Some(text) = text {
                        if let Some(builder_idx) =
                            self.find_object_index(builder_id).filter(|&index| {
                                !self.objects[index].destroyed
                                    && !matches!(
                                        self.objects[index].state.status,
                                        ObjectStatus::Deleted
                                    )
                            })
                        {
                            let controller = self.objects[builder_idx].state.controller;
                            self.messages.add_message(
                                MessageSpec::target(text, builder_id)
                                    .with_player((controller != OWNER_NONE).then_some(controller))
                                    .with_offset(Vector2::new(-1, -1)),
                            );
                        }
                    }
                }
            }

            // BuildNeedsMaterial may itself complete, remove, or retarget the
            // construction. Re-read the native failure split before stopping.
            self.finish_failed_build(builder_id)?;
            return Ok(false);
        }

        // Component-removal callbacks may have changed either object's live
        // definition or physicals. C++ reads GetPhysical and Def->Mass only
        // after both material passes (C4Object.cpp:1751-1763).
        let Some(builder_idx) = self.find_object_index(builder_id) else {
            return Ok(false);
        };
        let mut build_speed = self.object_physical(builder_idx).can_construct;
        if build_speed == 0 {
            self.finish_failed_build(builder_id)?;
            return Ok(false);
        }
        if build_speed <= 1 {
            build_speed = 100;
        }

        let Some(target_idx) = self.find_object_index(target_id) else {
            self.finish_failed_build(builder_id)?;
            return Ok(false);
        };
        let target_mass = self
            .definitions
            .get(&self.objects[target_idx].definition_id)
            .map(Definition::mass)
            .unwrap_or(100);
        let target_mass = if target_mass == 0 { 1 } else { target_mass };
        let delta = (i64::from(level) * i64::from(build_speed) * 150) / i64::from(target_mass);
        let delta = delta.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let current_construction = self.objects[target_idx].state.construction;
        let desired_construction = current_construction
            .saturating_add(delta)
            .clamp(0, FULL_CON);
        let crossed_full_con = current_construction < FULL_CON && desired_construction >= FULL_CON;
        let refresh_components = !need_material
            && docon_refreshes_construction(current_construction, desired_construction);
        let gained_components = refresh_components.then(|| {
            let definition_components = self
                .definitions
                .get(&self.objects[target_idx].definition_id)
                .map(|definition| {
                    definition
                        .components()
                        .iter()
                        .map(|component| (component.id.clone(), component.count))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            docon_component_counts(
                &self.objects[target_idx].state.components,
                &definition_components,
                desired_construction,
                delta,
            )
        });
        self.objects[target_idx].set_construction(desired_construction);
        if let Some(components) = gained_components {
            self.objects[target_idx].state.components = components;
        }
        self.refresh_object_ocf(target_idx);
        self.update_sector_for_index(target_idx);
        self.update_solid_mask(target_idx);

        // C4Object::DoCon dispatches Completion and then Initialize after
        // the bottom-preserving shape adjustment on the first FullCon
        // crossing (C4Object.cpp:1498-1503). Completion may delete the
        // target, in which case C++ suppresses Initialize.
        if crossed_full_con {
            if let Some(completion_idx) = self
                .find_object_index(target_id)
                .filter(|&index| self.object_survives_creation(index))
            {
                let _ = tolerate_script_error(self.call_object_function(
                    completion_idx,
                    "Completion",
                    Vec::new(),
                ))?;
            }
            if let Some(initialize_idx) = self
                .find_object_index(target_id)
                .filter(|&index| self.object_survives_creation(index))
            {
                let _ = tolerate_script_error(self.call_object_function(
                    initialize_idx,
                    "Initialize",
                    Vec::new(),
                ))?;
            }
        }

        // BuildTurnTo is read from the live definition only after DoCon's
        // Completion/Initialize callbacks, and runs on every successful
        // Build tick (C4Object.cpp:1765-1769).
        let build_turn_to = self
            .find_object_index(target_id)
            .and_then(|index| self.definitions.get(&self.objects[index].definition_id))
            .and_then(|definition| definition.build_turn_to().map(str::to_owned));
        if let (Some(target_idx), Some(build_turn_to)) =
            (self.find_object_index(target_id), build_turn_to)
        {
            let _ = self.change_object_def_live(target_idx, &build_turn_to)?;
        }
        if let Some(target_idx) = self.find_object_index(target_id) {
            self.objects[target_idx].state.damage = 0;
        }

        // Unlike most procedure attachment bits, DFA_BUILD adds Bottom only
        // after Target::Build succeeds (C4Object.cpp:5052-5055). All guard
        // and Build-failure returns retain only the pre-switch base bits.
        if let Some(builder_idx) = self.find_object_index(builder_id) {
            let builder = &mut self.objects[builder_idx];
            builder.frame_t_attach |= CNAT_BOTTOM;
            builder.state.t_attach = builder.frame_t_attach;
        }

        Ok(true)
    }

    /// `C4Def::GetComponents(..., pObjInstance=nullptr, pBuilder)` for
    /// `C4Object::Build`. The definition callback has no object `this`, but
    /// receives the builder and commits synchronous host side effects before
    /// the material scan. Only an array overrides the DefCore component list.
    pub(crate) fn build_required_components(
        &mut self,
        definition_id: &DefinitionId,
        builder_id: ObjectId,
    ) -> Result<Vec<DefinitionComponent>, EngineError> {
        let Some((script, static_components, has_custom_components)) =
            self.definitions.get(definition_id).map(|definition| {
                (
                    definition.script_arc(),
                    definition.components().to_vec(),
                    definition.has_function("GetCustomComponents"),
                )
            })
        else {
            return Ok(Vec::new());
        };
        if !has_custom_components {
            return Ok(static_components);
        }

        let world = self.host_world_context();
        let (value, _args, batch, audio_state, rng, script_error) =
            ScenarioScript::call_value_for_script(
                definition_id,
                &script,
                Some(definition_id.clone()),
                "GetCustomComponents",
                &[object_reference_value(builder_id)],
                world,
                self.rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            tolerate_script_error::<()>(Err(error))?;
            return Ok(static_components);
        }

        let Some(Value::Array(values)) = value else {
            return Ok(static_components);
        };
        Ok(compat::component_list_from_custom_array(&values)
            .into_iter()
            .map(|(id, count)| DefinitionComponent { id, count })
            .collect())
    }

    /// The ordinary (non-forced) ObjectComStop used by DFA_BUILD
    /// (C4ObjectCom.cpp:239-245). Re-resolve after Idle because its callbacks
    /// may remove the builder or change the definition used by Walk.
    fn object_com_stop_build(&mut self, builder_id: ObjectId) -> Result<bool, EngineError> {
        let Some(_) = self.find_object_index(builder_id).filter(|&index| {
            !self.objects[index].destroyed
                && !matches!(self.objects[index].state.status, ObjectStatus::Deleted)
        }) else {
            return Ok(false);
        };
        self.object_com_stop_live(builder_id)
    }

    /// Shared false return from `C4Object::Build` (C4Object.cpp:5033-5051).
    /// The complete/missing decision uses the live pre-stop Action.Target;
    /// internal Exit uses the post-stop target because action callbacks may
    /// clear, replace, remove, or recontain it.
    fn finish_failed_build(&mut self, builder_id: ObjectId) -> Result<(), EngineError> {
        let complete_or_missing = self
            .find_object_index(builder_id)
            .and_then(|builder_idx| self.objects[builder_idx].state.action.target)
            .is_none_or(|target_id| {
                self.find_object_index(target_id).is_none_or(|target_idx| {
                    self.objects[target_idx].state.construction >= FULL_CON
                })
            });

        let _ = self.object_com_stop_build(builder_id)?;

        if !complete_or_missing {
            return Ok(());
        }
        let Some(target_idx) = self
            .find_object_index(builder_id)
            .and_then(|builder_idx| self.objects[builder_idx].state.action.target)
            .and_then(|target_id| self.find_object_index(target_id))
            .filter(|&target_idx| self.objects[target_idx].state.container == Some(builder_id))
        else {
            return Ok(());
        };
        self.set_plain_exit_command(target_idx)
    }

    /// DFA_CHOP (C4Object.cpp:5202-5221): every Tick3 asks the current
    /// target to Chop; C4Object::Chop applies +10 damage only on Tick10
    /// (C4Object.cpp:1775-1782). The target must remain chop-capable and
    /// cover the chopper's current position, then the chopper faces it.
    pub(crate) fn apply_chop_procedure(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
    ) -> Result<bool, EngineError> {
        let Some(target_id) = self.objects[idx].state.action.target else {
            let _ = self.object_action_stand(idx, definition_id)?;
            return Ok(false);
        };
        let Some(target_idx) = self.find_object_index(target_id) else {
            let _ = self.object_action_stand(idx, definition_id)?;
            return Ok(false);
        };
        let target_can_be_chopped = {
            let target = &self.objects[target_idx];
            target.has_nonzero_status()
                && target.state.container.is_none()
                && self.definitions.contains_key(&target.definition_id)
                && target.state.ocf & crate::ocf::CHOP != 0
        };
        if !target_can_be_chopped {
            let _ = self.object_action_stand(idx, definition_id)?;
            return Ok(false);
        }

        if self.frame.is_multiple_of(3) && self.frame.is_multiple_of(10) {
            let caused_by = self.objects[idx].state.owner;
            self.change_object_damage(target_idx, 10, C4FX_CALL_DMG_CHOP, caused_by)?;
        }

        // Damage callbacks may remove or otherwise invalidate the target;
        // C++ rechecks Action.Target immediately after Chop returns.
        let Some(target_id) = self.objects[idx].state.action.target else {
            let _ = self.object_action_stand(idx, definition_id)?;
            return Ok(false);
        };
        let Some(target_idx) = self.find_object_index(target_id) else {
            let _ = self.object_action_stand(idx, definition_id)?;
            return Ok(false);
        };
        let chopper_position = self.objects[idx].state.position;
        let target = &self.objects[target_idx];
        let target_is_at_chopper = target.has_nonzero_status()
            && target.state.container.is_none()
            && target.state.ocf & crate::ocf::CHOP != 0
            && self
                .object_shape_rect(target)
                .contains_point(chopper_position.x, chopper_position.y);
        if !target_is_at_chopper {
            let _ = self.object_action_stand(idx, definition_id)?;
            return Ok(false);
        }

        let direction = if chopper_position.x > target.state.position.x {
            Direction::Left
        } else {
            Direction::Right
        };
        self.set_exec_action_direction(idx, definition_id, direction)?;
        Ok(true)
    }

    pub(crate) fn ensure_build_components(
        &mut self,
        builder_idx: usize,
        target_idx: usize,
        required: &[DefinitionComponent],
    ) -> Result<Option<(DefinitionId, i32)>, EngineError> {
        if required.is_empty() {
            return Ok(None);
        }

        // Target::Build makes two independent passes: at most one full-con
        // object of every missing component from the builder, then at most
        // one more from the construction's container (C4Object.cpp:1694-1723).
        for component in required {
            let inserted = self.objects[target_idx]
                .state
                .components
                .get(&component.id)
                .unwrap_or(0);

            if inserted < component.count {
                if let Some(material_id) =
                    self.first_eligible_build_component(builder_idx, &component.id)
                {
                    self.credit_build_component(target_idx, &component.id, inserted);
                    self.consume_build_component(builder_idx, material_id)?;
                }
            }
        }

        for component in required {
            let inserted = self.objects[target_idx]
                .state
                .components
                .get(&component.id)
                .unwrap_or(0);

            if inserted < component.count {
                if let Some((container_idx, material_id)) =
                    self.first_build_component_from_container_of(target_idx, &component.id)
                {
                    self.credit_build_component(target_idx, &component.id, inserted);
                    self.consume_build_component(container_idx, material_id)?;
                }
            }
        }

        Ok(required.iter().find_map(|component| {
            (component.count != 0)
                .then(|| {
                    let inserted = self.objects[target_idx]
                        .state
                        .components
                        .get(&component.id)
                        .unwrap_or(0);
                    let inserted_percent =
                        i64::from(inserted).saturating_mul(100) / i64::from(component.count);
                    let construction_percent =
                        i64::from(self.objects[target_idx].state.construction).saturating_mul(100)
                            / i64::from(FULL_CON);
                    (inserted_percent < construction_percent)
                        .then(|| (component.id.clone(), component.count.wrapping_sub(inserted)))
                })
                .flatten()
        }))
    }

    /// C4ObjectList::Find(id) returns the first matching live content. Build
    /// tests only that one for OnFire/OCF_FullCon; an ineligible head blocks
    /// later same-ID entries for this pass (C4Object.cpp:1697-1709).
    fn first_eligible_build_component(
        &self,
        container_index: usize,
        component_id: &DefinitionId,
    ) -> Option<ObjectId> {
        if container_index >= self.objects.len() {
            return None;
        }
        for &object_id in &self.objects[container_index].state.contents {
            let Some(child_index) = self.find_object_index(object_id) else {
                continue;
            };
            let child = &self.objects[child_index];
            if child.definition_id != *component_id
                || child.destroyed
                || matches!(child.state.status, ObjectStatus::Deleted)
            {
                continue;
            }
            return (!child.state.on_fire && child.state.ocf & ocf::FULL_CON != 0)
                .then_some(object_id);
        }
        None
    }

    fn first_build_component_from_container_of(
        &self,
        object_index: usize,
        component_id: &DefinitionId,
    ) -> Option<(usize, ObjectId)> {
        if object_index >= self.objects.len() {
            return None;
        }
        let container_id = match self.objects[object_index].state.container {
            Some(id) => id,
            None => return None,
        };
        let container_index = self.find_object_index(container_id)?;
        self.first_eligible_build_component(container_index, component_id)
            .map(|material_id| (container_index, material_id))
    }

    fn credit_build_component(
        &mut self,
        target_idx: usize,
        component_id: &DefinitionId,
        previous_count: i32,
    ) {
        let target_state = &mut self.objects[target_idx].state;
        if !target_state.components.contains(component_id) {
            target_state.component_order.push(component_id.clone());
        }
        target_state
            .components
            .set(component_id.clone(), previous_count.wrapping_add(1));
    }

    /// Build credits the component, unlinks it from the parent list, then
    /// calls native AssignRemoval while the child's Contained pointer still
    /// names that parent (C4Object.cpp:1705-1709,1717-1721).
    fn consume_build_component(
        &mut self,
        container_index: usize,
        material_id: ObjectId,
    ) -> Result<(), EngineError> {
        if self.objects.get(container_index).is_none() {
            return Ok(());
        }
        if self.find_object_index(material_id).is_none() {
            return Ok(());
        }
        self.objects[container_index]
            .state
            .contents
            .retain(|&id| id != material_id);
        let _ = self.assign_object_removal(material_id)?;
        Ok(())
    }

    /// `ReduceLineSegments` (C4Object.cpp:4683-4694): remove the first
    /// redundant bend whose surrounding vertices have a clear direct path.
    /// The alternate pass skips two vertices and removes both of them.
    fn reduce_line_segments(
        landscape: Option<&Landscape>,
        vertices: &mut Vec<ObjectVertex>,
        alternate: bool,
    ) -> bool {
        let skip = 2 + usize::from(alternate);
        let redundant = (0..vertices.len().saturating_sub(skip)).find(|&index| {
            let from = Vector2::new(vertices[index].x, vertices[index].y);
            let to = Vector2::new(vertices[index + skip].x, vertices[index + skip].y);
            landscape
                .map(|landscape| landscape.path_is_clear(from, to))
                .unwrap_or(true)
        });
        let Some(index) = redundant else {
            return false;
        };
        if alternate {
            vertices.remove(index + 2);
        }
        vertices.remove(index + 1);
        true
    }

    /// The per-pixel `ForLine` walk behind C++ `PathFree`, including its
    /// canonical low-to-high major-axis traversal (C4Landscape.cpp:1670-1722).
    fn line_first_collision(
        landscape: Option<&Landscape>,
        start: Vector2,
        end: Vector2,
        ignore_vehicle: bool,
    ) -> Option<Vector2> {
        let landscape = landscape?;
        let (mut x1, mut y1) = (i64::from(start.x), i64::from(start.y));
        let (mut x2, mut y2) = (i64::from(end.x), i64::from(end.y));
        let blocked = |x: i64, y: i64| {
            let (x, y) = (x as i32, y as i32);
            let solid = if ignore_vehicle {
                landscape.is_solid_ignoring_vehicle_at(x, y)
            } else {
                landscape.is_solid_at(x, y)
            };
            solid.then_some(Vector2::new(x, y))
        };

        if (x2 - x1).abs() < (y2 - y1).abs() {
            if y1 > y2 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let x_increment = if x2 > x1 { 1 } else { -1 };
            let dy = y2 - y1;
            let dx = (x2 - x1).abs();
            let mut decision = 2 * dx - dy;
            let advance_both = 2 * (dx - dy);
            let advance_y = 2 * dx;
            let mut x = x1;
            if let Some(hit) = blocked(x, y1) {
                return Some(hit);
            }
            for y in (y1 + 1)..=y2 {
                if decision >= 0 {
                    x += x_increment;
                    decision += advance_both;
                } else {
                    decision += advance_y;
                }
                if let Some(hit) = blocked(x, y) {
                    return Some(hit);
                }
            }
        } else {
            if x1 > x2 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let y_increment = if y2 > y1 { 1 } else { -1 };
            let dx = x2 - x1;
            let dy = (y2 - y1).abs();
            let mut decision = 2 * dy - dx;
            let advance_both = 2 * (dy - dx);
            let advance_x = 2 * dy;
            let mut y = y1;
            if let Some(hit) = blocked(x1, y) {
                return Some(hit);
            }
            for x in (x1 + 1)..=x2 {
                if decision >= 0 {
                    y += y_increment;
                    decision += advance_both;
                } else {
                    decision += advance_x;
                }
                if let Some(hit) = blocked(x, y) {
                    return Some(hit);
                }
            }
        }
        None
    }

    /// `C4Shape::LineConnect` (C4Shape.cpp:273-326): move one endpoint of
    /// a wrapping line, inserting the first viable bend around the first
    /// solid pixel when the direct path to its neighbour is blocked.
    pub(crate) fn line_connect_endpoint(
        landscape: Option<&Landscape>,
        vertices: &mut Vec<ObjectVertex>,
        target: Vector2,
        endpoint: usize,
        direction: isize,
    ) -> bool {
        const MAX_VERTEX_COUNT: usize = 30; // C4D_MaxVertex

        if vertices.len() < 2 {
            return false;
        }
        let Some(neighbour) = endpoint.checked_add_signed(direction) else {
            return false;
        };
        if neighbour >= vertices.len() {
            return false;
        }
        if (vertices[endpoint].x, vertices[endpoint].y) == (target.x, target.y) {
            return true;
        }

        let neighbour_point = Vector2::new(vertices[neighbour].x, vertices[neighbour].y);
        let collision = Self::line_first_collision(landscape, target, neighbour_point, false);
        let Some(collision) = collision else {
            vertices[endpoint].x = target.x;
            vertices[endpoint].y = target.y;
            return true;
        };

        let path_is_clear =
            |from, to| Self::line_first_collision(landscape, from, to, false).is_none();
        let bend = [4, 8, 12].into_iter().find_map(|range| {
            let half = range / 2;
            [collision.x - half, collision.x + half]
                .into_iter()
                .find_map(|x| {
                    [collision.y - half, collision.y + half]
                        .into_iter()
                        .map(|y| Vector2::new(x, y))
                        .find(|&candidate| {
                            path_is_clear(candidate, target)
                                && path_is_clear(candidate, neighbour_point)
                        })
                })
        });

        let old_endpoint = Vector2::new(vertices[endpoint].x, vertices[endpoint].y);
        let path_is_clear_ignoring_vehicle =
            |from, to| Self::line_first_collision(landscape, from, to, true).is_none();
        let Some(bend) = bend.or_else(|| {
            (path_is_clear_ignoring_vehicle(old_endpoint, target)
                && path_is_clear_ignoring_vehicle(old_endpoint, neighbour_point))
            .then_some(old_endpoint)
        }) else {
            return false;
        };
        if vertices.len() >= MAX_VERTEX_COUNT {
            return false;
        }

        if direction > 0 {
            vertices.insert(endpoint + 1, ObjectVertex::new(bend.x, bend.y));
            vertices[endpoint].x = target.x;
            vertices[endpoint].y = target.y;
        } else {
            vertices.insert(endpoint, ObjectVertex::new(bend.x, bend.y));
            vertices[endpoint + 1].x = target.x;
            vertices[endpoint + 1].y = target.y;
        }
        true
    }

    /// DFA_CONNECT (C4Object.cpp:5363-5447): a Line object's first
    /// vertex tracks Action.Target and its last vertex Action.Target2 —
    /// C4D_Line_Vertex (8) connects to the target's own vertex (index
    /// from the numbered Local[2]/Local[3], both 0 for CHBM), any other
    /// line type to the target's bottom center (x, y + Shape.Hgt/4).
    /// LineIntersect=1 assigns the ABSOLUTE point directly; a missing or
    /// incomplete target fires LineBreak(true) and removes the line.
    /// Returns false when the object removed itself.
    pub(crate) fn exec_connect_line(&mut self, idx: usize) -> Result<bool, EngineError> {
        let object_id = self.objects[idx].id;
        let definition_id = self.objects[idx].definition_id.clone();
        let Some(definition) = self.definitions.get(&definition_id) else {
            return Ok(true);
        };
        if definition.line() == 0 {
            return Ok(true);
        }
        let action_name = self.objects[idx].state.action.name.clone();
        let action_index = self.objects[idx].state.action.act_map_index;
        if definition
            .action_library()
            .procedure_for_entry(&action_name, action_index)
            != ActionProcedure::Connect
        {
            return Ok(true);
        }
        let line_vertex = definition.line() == 8; // C4D_Line_Vertex
        let line_intersect = definition.line_intersect();
        let vertex_indices = ["__local_2", "__local_3"].map(|name| {
            self.objects[idx]
                .state
                .local_vars
                .get(name)
                .and_then(Value::as_c4_int)
                .unwrap_or(0)
        });

        let mut broke = false;
        let mut points = [None, None];
        for (slot, target_id) in [
            (0usize, self.objects[idx].state.action.target),
            (1usize, self.objects[idx].state.action.target2),
        ] {
            let resolved = target_id
                .and_then(|id| self.find_object_index(id))
                .filter(|&target_idx| self.objects[target_idx].state.construction >= FULL_CON);
            match resolved {
                None => broke = true,
                Some(target_idx) => {
                    let target = &self.objects[target_idx];
                    let point = if line_vertex {
                        // C4Shape::GetVertexX/Y return zero for every index
                        // outside the active vertex range.
                        let (vertex_x, vertex_y) = usize::try_from(vertex_indices[slot])
                            .ok()
                            .and_then(|index| target.state.vertices.get(index))
                            .map(|vertex| (vertex.x, vertex.y))
                            .unwrap_or((0, 0));
                        Vector2::new(
                            target.state.position.x + vertex_x,
                            target.state.position.y + vertex_y,
                        )
                    } else {
                        let height = target
                            .current_shape_rect()
                            .map(|rect| rect.height)
                            .unwrap_or(0);
                        Vector2::new(
                            target.state.position.x,
                            target.state.position.y + height / 4,
                        )
                    };
                    points[slot] = Some(point);
                }
            }
        }

        if broke {
            // Call(PSF_LineBreak, {true}) then AssignRemoval
            // (C4Object.cpp:5371-5375); the call is fail-safe like every
            // engine callback.
            if self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.has_function("LineBreak"))
                .unwrap_or(false)
            {
                tolerate_script_error(self.call_object_function(
                    idx,
                    "LineBreak",
                    vec![Value::Bool(true)],
                ))?;
            }
            let _ = self.assign_object_removal(object_id)?;
            return Ok(false);
        }

        let reduce_segments = self.frame.is_multiple_of(35);
        let alternate_reduction = self.frame.is_multiple_of(2);
        let movement_broke = {
            let landscape = self.landscape.as_ref();
            let object = &mut self.objects[idx];
            if object.state.vertices.is_empty() {
                return Ok(true);
            }
            let mut movement_broke = false;
            if line_intersect == 1 {
                if let Some(point) = points[0] {
                    object.state.vertices[0].x = point.x;
                    object.state.vertices[0].y = point.y;
                }
                if let Some(point) = points[1] {
                    let last = object.state.vertices.len() - 1;
                    object.state.vertices[last].x = point.x;
                    object.state.vertices[last].y = point.y;
                }
            } else {
                if let Some(point) = points[0] {
                    movement_broke |= !Self::line_connect_endpoint(
                        landscape,
                        &mut object.state.vertices,
                        point,
                        0,
                        1,
                    );
                }
                if let Some(point) = points[1] {
                    let last = object.state.vertices.len() - 1;
                    movement_broke |= !Self::line_connect_endpoint(
                        landscape,
                        &mut object.state.vertices,
                        point,
                        last,
                        -1,
                    );
                }
            }
            // ExecAction's CONNECT branch prunes at most one redundant run on
            // !Tick35, alternating one- and two-bend skips with !Tick2
            // (C4Object.cpp:5443-5445).
            if !movement_broke && reduce_segments {
                Self::reduce_line_segments(
                    landscape,
                    &mut object.state.vertices,
                    alternate_reduction,
                );
            }
            movement_broke
        };
        if movement_broke {
            if self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.has_function("LineBreak"))
                .unwrap_or(false)
            {
                tolerate_script_error(self.call_object_function(idx, "LineBreak", Vec::new()))?;
            }
            let _ = self.assign_object_removal(object_id)?;
            return Ok(false);
        }
        Ok(true)
    }

    /// `C4ActionDef::FlipDir` of the action a world object currently holds.
    /// Idle objects answer the C++ zero (C4Object.cpp:412-415).
    pub(crate) fn object_action_flip_dir(&self, idx: usize) -> i32 {
        self.action_entry_flip_dir(idx, &self.objects[idx].state.action)
    }

    /// `C4ActionDef::FlipDir` of an arbitrary ActMap entry of the object's
    /// definition — SetAction's guard compares the outgoing entry's value
    /// against the incoming one (C4Object.cpp:4183-4184).
    pub(crate) fn action_entry_flip_dir(&self, idx: usize, action: &ActionState) -> i32 {
        self.definitions
            .get(&self.objects[idx].definition_id)
            .map_or(0, |definition| {
                definition
                    .action_library()
                    .flip_dir_for_entry(&action.name, action.act_map_index)
            })
    }

    /// `C4Object::UpdateFlipDir` (C4Object.cpp:410-442) for a world object.
    pub(crate) fn update_object_flip_dir(&mut self, idx: usize) {
        let flip_dir = self.object_action_flip_dir(idx);
        self.objects[idx].state.update_flip_dir(flip_dir);
    }

    /// `C4Object::SetDir`'s trailing assignment for a world object. Every C++
    /// facing change goes through SetDir — `Action.Dir` is assigned directly
    /// nowhere else at runtime — so every engine-side direction write has to
    /// keep the mirror coherent too.
    pub(crate) fn write_object_direction(&mut self, idx: usize, direction: Direction) {
        let flip_dir = self.object_action_flip_dir(idx);
        self.objects[idx].state.write_direction(direction, flip_dir);
    }

    /// C4Object::SetDir as reached from an internal ExecAction procedure
    /// (C4Object.cpp:4248-4265): run the current action's TurnAction through
    /// SetActionByName on a facing change, then assign the requested direction
    /// even if that transition failed or changed actions. The shared SetDir
    /// entry rejects idle and out-of-range directions first, including signed
    /// zero/negative Directions values (C4Object.cpp:4237-4240).
    pub(crate) fn set_exec_action_direction(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        direction: Direction,
    ) -> Result<(), EngineError> {
        let object_id = self.objects[idx].id;
        let Some((current_direction, turn_action)) =
            self.definitions.get(definition_id).and_then(|definition| {
                let library = definition.action_library();
                let action = &self.objects[idx].state.action;
                let raw_direction = direction.to_script_value();
                (!library.is_idle_state(action)
                    && raw_direction >= 0
                    && raw_direction
                        < library.directions_for_entry(&action.name, action.act_map_index))
                .then(|| {
                    (
                        self.objects[idx].state.direction,
                        library
                            .turn_action_for_entry(&action.name, action.act_map_index)
                            .map(str::to_string),
                    )
                })
            })
        else {
            return Ok(());
        };
        if direction != current_direction {
            if let Some(turn_action) = turn_action {
                self.action_with_calls(idx, definition_id, &turn_action)?;
            }
        }
        if let Some(idx) = self.find_object_index(object_id) {
            self.write_object_direction(idx, direction);
        }
        Ok(())
    }

    /// The two independent raw-xdir phase/`SetDir` tests shared by DFA_PUSH
    /// and DFA_PULL (C4Object.cpp:5103-5108,5186-5194). Each branch latches
    /// phase advance before its TurnAction callback; a callback may mutate
    /// xdir, the action, or the definition, so the positive test then uses the
    /// live object and may replace that latch.
    fn set_exec_direction_from_xdir_live(
        &mut self,
        object_id: ObjectId,
        mut phase_advance: i32,
    ) -> Result<(Option<usize>, i32), EngineError> {
        if let Some(idx) = self.find_object_index(object_id) {
            if self.objects[idx].fixed_velocity.x < C4Fixed::ZERO {
                phase_advance = -math::fixtoi(self.objects[idx].fixed_velocity.x * 10);
                let definition_id = self.objects[idx].definition_id.clone();
                self.set_exec_action_direction(idx, &definition_id, Direction::Left)?;
            }
        }
        if let Some(idx) = self.find_object_index(object_id) {
            if self.objects[idx].fixed_velocity.x > C4Fixed::ZERO {
                phase_advance = math::fixtoi(self.objects[idx].fixed_velocity.x * 10);
                let definition_id = self.objects[idx].definition_id.clone();
                self.set_exec_action_direction(idx, &definition_id, Direction::Right)?;
            }
        }
        Ok((self.find_object_index(object_id), phase_advance))
    }

    /// Exact `C4Object::SetDir` gate for direct command/native paths
    /// (C4Object.cpp:4235-4253). Unlike the legacy ExecAction helper above,
    /// this rejects idle/out-of-range directions and uses non-forced
    /// SetActionByName for TurnAction.
    pub(crate) fn set_command_action_direction(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        direction: Direction,
    ) -> Result<(), EngineError> {
        let object_id = self.objects[idx].id;
        let Some((current_direction, turn_action)) =
            self.definitions.get(definition_id).and_then(|definition| {
                let library = definition.action_library();
                let action = &self.objects[idx].state.action;
                let raw_direction = direction.to_script_value();
                (!library.is_idle_state(action)
                    && raw_direction >= 0
                    && raw_direction
                        < library.directions_for_entry(&action.name, action.act_map_index))
                .then(|| {
                    (
                        self.objects[idx].state.direction,
                        library
                            .turn_action_for_entry(&action.name, action.act_map_index)
                            .map(str::to_string),
                    )
                })
            })
        else {
            return Ok(());
        };
        if direction != current_direction {
            if let Some(turn_action) = turn_action {
                self.action_with_calls(idx, definition_id, &turn_action)?;
            }
        }
        if let Some(idx) = self.find_object_index(object_id) {
            self.write_object_direction(idx, direction);
        }
        Ok(())
    }

    /// Explicit `SetActionByName(..., fForce=true)` transition: applies the
    /// named action even through NoOtherAction, refreshes OCF, resyncs the
    /// fixed coords, then synchronously runs StartCall while force suppresses
    /// EndCall/AbortCall (C4ActionCallbacks.h:24-34). C++ ObjectAction helpers
    /// use the ordinary non-forced twin below.
    #[doc(hidden)]
    pub fn force_action_with_calls(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        name: &str,
    ) -> Result<bool, EngineError> {
        self.action_with_optional_target_and_calls(idx, definition_id, name, None, true)
    }

    /// Ordinary non-forced SetActionByName transition, including its
    /// synchronous StartCall/AbortCall sequence.
    pub(crate) fn action_with_calls(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        name: &str,
    ) -> Result<bool, EngineError> {
        self.action_with_optional_target_and_calls(idx, definition_id, name, None, false)
    }

    /// Ordinary target-bearing `SetActionByName`, used by ObjectActionPush
    /// and the other C++ ObjectAction helpers that do not pass `fForce`.
    pub(crate) fn action_with_target_and_calls(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        name: &str,
        target: ObjectId,
    ) -> Result<bool, EngineError> {
        self.action_with_optional_target_and_calls(idx, definition_id, name, Some(target), false)
    }

    fn action_with_optional_target_and_calls(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        name: &str,
        target: Option<ObjectId>,
        force: bool,
    ) -> Result<bool, EngineError> {
        let builtin_idle = action::is_builtin_idle_name(name);
        let name = if builtin_idle { "Idle" } else { name };
        let Some((library, incomplete_activity)) =
            self.definitions.get(definition_id).map(|definition| {
                (
                    definition.shared_action_library_handle(),
                    definition.incomplete_activity(),
                )
            })
        else {
            return Ok(false);
        };
        // SetActionByName("Idle"/"ActIdle") selects the built-in slot before scanning
        // ActMap (C4Object.cpp:4214-4215). ActionState's
        // library-aware update already models that slot; do not reject it
        // merely because a definition's map starts at Walk.
        if !action::is_builtin_idle_name(name) && !library.contains(name) {
            return Ok(false);
        }
        let previous = self.objects[idx].state.action.clone();
        let previous_index = previous
            .act_map_index
            .or_else(|| library.named_action_index(&previous.name));
        let requested_index = (!builtin_idle)
            .then(|| library.named_action_index(name))
            .flatten();
        let requested_action_changed = previous.name != name || previous_index != requested_index;
        let active_action_allowed =
            self.objects[idx].state.construction >= FULL_CON || incomplete_activity;
        let object_id = self.objects[idx].id;
        let original_definition_id = self.objects[idx].definition_id.clone();
        let update = ActionUpdate {
            name: Some(name.to_string()),
            phase: Some(0),
            ticks: Some(0),
            force,
            data: None,
            // SetAction assigns a target before SetOCF and callbacks when one
            // is supplied (C4Object.cpp:4148-4178). None preserves the old
            // target; the Idle/ActIdle name sentinel discards supplied
            // targets before SetAction (C4Object.cpp:4225-4227).
            target: (!builtin_idle).then_some(target).flatten().map(Some),
            target2: None,
            callbacks_dispatched: false,
            action_sound_dispatched: false,
            action_sound_selection: None,
        };
        let result = {
            let object = &mut self.objects[idx];
            object.state.action.apply_update_with_library_and_activity(
                &update,
                &library,
                active_action_allowed,
            )
        };
        if !matches!(result, ActionUpdateResult::Applied) {
            return Ok(false);
        }

        // SetAction refreshes the mirror only when the FlipDir VALUE differs
        // between the old and the new action — two actions sharing a FlipDir
        // keep the transform untouched (C4Object.cpp:4183-4184). It runs after
        // UpdateActionFace and before SetOCF.
        let previous_flip_dir = library.flip_dir_for_entry(&previous.name, previous.act_map_index);
        let current_flip_dir = self.objects[idx].state.action_flip_dir(&library);
        if previous_flip_dir != current_flip_dir {
            self.objects[idx].state.update_flip_dir(current_flip_dir);
        }

        let current_action = self.objects[idx].state.action.clone();
        self.objects[idx].record_action_sound_transition(
            &previous,
            &current_action,
            &library,
            requested_action_changed,
        );
        self.dispatch_pending_action_sounds(idx, false);

        // SetOCF and fixed-position resync both precede StartCall in C++
        // (C4Object.cpp:4165-4178).
        self.refresh_object_ocf(idx);
        {
            let object = &mut self.objects[idx];
            object.fixed_position =
                FixedVec2::from_ints(object.state.position.x, object.state.position.y);
        }

        if !library.is_idle_state(&current_action) {
            self.invoke_action_callback(
                idx,
                ActionCallbackKind::Start,
                &current_action.name,
                current_action.act_map_index,
                None,
                None,
                None,
                None,
            )?;
        }

        // A StartCall that removes the object or changes its definition stops
        // the remaining callback sequence (C4Object.cpp:4178-4198).
        let callback_target_survived = self.objects.get(idx).is_some_and(|object| {
            object.id == object_id
                && object.definition_id == original_definition_id
                && !object.destroyed
                && !matches!(object.state.status, ObjectStatus::Deleted)
        });
        if callback_target_survived && !force && !library.is_idle_state(&previous) {
            self.invoke_action_callback(
                idx,
                ActionCallbackKind::Abort,
                &previous.name,
                previous.act_map_index,
                None,
                None,
                Some(previous.phase),
                None,
            )?;
        }

        Ok(true)
    }

    /// C4Object::ContactAction (C4Object.cpp:4307-4520): the hardcoded
    /// per-procedure contact transitions, dispatched on the frame's
    /// accumulated t_contact bits right after DoMovement
    /// (C4Movement.cpp:463-467). ObjectAction* helpers per
    /// C4ObjectCom.cpp:34-232.
    #[doc(hidden)]
    pub fn exec_contact_action(
        &mut self,
        idx: usize,
        t_contact: u32,
        _definition_id: &DefinitionId,
    ) -> Result<(), EngineError> {
        let Some(object_id) = self.objects.get(idx).map(|object| object.id) else {
            return Ok(());
        };
        // Direct/test callers pass the accumulated DoMovement contacts just
        // like C4Movement's `t_contact = iContacts` immediately before this
        // call. From here on ContactAction must re-read the live member after
        // every nested action callback (C4Movement.cpp:467-471;
        // C4Object.cpp:4319-4569).
        self.objects[idx].frame_t_contact = t_contact;
        // ContactAction resolves physicals before even its idle-action gate;
        // Def, Action and OCF are read only after that callback returns
        // (C4Object.cpp:4324-4330).
        let physical = self.object_physical(idx);
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(());
        };
        let live_definition_id = self.objects[idx].definition_id.clone();
        let Some(definition) = self.definitions.get(&live_definition_id) else {
            return Ok(());
        };
        let library = definition.shared_action_library_handle();
        let action = self.objects[idx].state.action.clone();
        let action_name = action.name.clone();
        if library.is_idle_state(&action) {
            return Ok(());
        }
        let procedure = library.procedure_for_entry(&action_name, action.act_map_index);
        let action_disabled = library.disables_object_for_entry(&action_name, action.act_map_index);
        let can_scale = physical.can_scale != 0;
        let can_hangle = physical.can_hangle != 0;

        let com_dir_like = |com: CommandDirection, sample: CommandDirection| -> bool {
            // ComDirLike (C4ObjectCom.cpp:922-928): the two COMD ring
            // neighbours count as "like".
            let com = com.to_script_value();
            let sample = sample.to_script_value();
            com == sample || com % 8 + 1 == sample || com == sample % 8 + 1
        };

        // Hit Bottom (C4Object.cpp:4332-4380). Only iProcedure and
        // fDisabled above are stack locals; t_contact, OCF, Action.Dir and
        // Action.ComDir remain live object fields throughout ContactAction.
        if self
            .find_object_index(object_id)
            .is_some_and(|idx| self.objects[idx].frame_t_contact & CNAT_BOTTOM != 0)
        {
            match procedure {
                ActionProcedure::Flight => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    if self.objects[idx].fixed_velocity.y >= C4Fixed::ZERO {
                        // FlatHit / HardHit / Walk
                        if self.objects[idx].state.ocf & crate::ocf::HIT_SPEED4 != 0
                            || action_disabled
                        {
                            let direction = self.objects[idx].state.direction;
                            let definition_id = self.objects[idx].definition_id.clone();
                            if self.object_action_flat(idx, &definition_id, direction)? {
                                return Ok(());
                            }
                        }
                        let Some(idx) = self.find_object_index(object_id) else {
                            return Ok(());
                        };
                        if self.objects[idx].state.ocf & crate::ocf::HIT_SPEED3 != 0 {
                            let definition_id = self.objects[idx].definition_id.clone();
                            if self.action_with_calls(idx, &definition_id, "KneelDown")? {
                                if let Some(idx) = self.find_object_index(object_id) {
                                    let object = &mut self.objects[idx];
                                    object.fixed_velocity = FixedVec2::ZERO;
                                    object.state.velocity = Vector2::ZERO;
                                }
                                return Ok(());
                            }
                        }
                        // Walk keeping horizontal momentum
                        // (C4Object.cpp:4330-4338).
                        let Some(idx) = self.find_object_index(object_id) else {
                            return Ok(());
                        };
                        let last_xdir = self.objects[idx].fixed_velocity.x;
                        let definition_id = self.objects[idx].definition_id.clone();
                        if self.action_with_calls(idx, &definition_id, "Walk")? {
                            if let Some(idx) = self.find_object_index(object_id) {
                                let object = &mut self.objects[idx];
                                object.fixed_velocity = FixedVec2::new(last_xdir, C4Fixed::ZERO);
                                object.state.velocity = object.velocity_pixels();
                            }
                        }
                        return Ok(());
                    }
                }
                ActionProcedure::Scale => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    let com_dir = self.objects[idx].state.command_direction;
                    let definition_id = self.objects[idx].definition_id.clone();
                    if !com_dir_like(com_dir, CommandDirection::Down) {
                        let _ = self.object_action_corner_scale(idx, &definition_id, procedure)?;
                        return Ok(());
                    }
                    self.object_action_stand(idx, &definition_id)?;
                    return Ok(());
                }
                ActionProcedure::Dig => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    match self.objects[idx].state.command_direction {
                        CommandDirection::DownLeft => {
                            self.objects[idx].state.command_direction = CommandDirection::Left;
                        }
                        CommandDirection::DownRight => {
                            self.objects[idx].state.command_direction = CommandDirection::Right;
                        }
                        _ => {
                            let definition_id = self.objects[idx].definition_id.clone();
                            self.object_com_stop_dig(idx, &definition_id)?;
                            return Ok(());
                        }
                    }
                }
                ActionProcedure::Swim => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    let above_liquid = {
                        let position = self.objects[idx].state.position;
                        self.landscape
                            .as_ref()
                            .map(|landscape| landscape.is_liquid_at(position.x, position.y - 1))
                            .unwrap_or(false)
                    };
                    if !above_liquid {
                        let definition_id = self.objects[idx].definition_id.clone();
                        let _ = self.object_action_corner_scale(idx, &definition_id, procedure)?;
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        // Hit Ceiling (C4Object.cpp:4382-4421).
        if self
            .find_object_index(object_id)
            .is_some_and(|idx| self.objects[idx].frame_t_contact & CNAT_TOP != 0)
        {
            match procedure {
                ActionProcedure::Walk => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    let definition_id = self.objects[idx].definition_id.clone();
                    self.object_action_stand(idx, &definition_id)?;
                    return Ok(());
                }
                ActionProcedure::Scale => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    let com_dir = self.objects[idx].state.command_direction;
                    if com_dir_like(com_dir, CommandDirection::Up) {
                        if can_hangle {
                            let new_dir = if self.objects[idx].state.direction == Direction::Left {
                                Direction::Right
                            } else {
                                Direction::Left
                            };
                            let definition_id = self.objects[idx].definition_id.clone();
                            self.object_action_hangle(idx, &definition_id, new_dir)?;
                            return Ok(());
                        }
                        self.objects[idx].state.command_direction = CommandDirection::Stop;
                    }
                }
                ActionProcedure::Flight => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    let direction = self.objects[idx].state.direction;
                    let definition_id = self.objects[idx].definition_id.clone();
                    if self.objects[idx].state.ocf & crate::ocf::HIT_SPEED3 != 0 || action_disabled
                    {
                        self.object_action_tumble(
                            idx,
                            &definition_id,
                            direction,
                            C4Fixed::ZERO,
                            C4Fixed::ZERO,
                        )?;
                    } else if can_hangle {
                        self.object_action_hangle(idx, &definition_id, direction)?;
                        return Ok(());
                    }
                }
                ActionProcedure::Dig => {
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    let definition_id = self.objects[idx].definition_id.clone();
                    self.object_com_stop_dig(idx, &definition_id)?;
                    return Ok(());
                }
                ActionProcedure::Hang => {
                    if let Some(idx) = self.find_object_index(object_id) {
                        self.objects[idx].state.command_direction = CommandDirection::Stop;
                    }
                }
                _ => {}
            }
        }

        // Hit Left / Right Walls (C4Object.cpp:4406-4520)
        for (cnat, wall_direction) in [(CNAT_LEFT, Direction::Left), (CNAT_RIGHT, Direction::Right)]
        {
            let Some(idx) = self.find_object_index(object_id) else {
                return Ok(());
            };
            if self.objects[idx].frame_t_contact & cnat == 0 {
                continue;
            }
            let tumble_x = contact_action_wall_tumble_x(cnat);
            let toward = if wall_direction == Direction::Left {
                CommandDirection::Left
            } else {
                CommandDirection::Right
            };
            let away = if wall_direction == Direction::Left {
                CommandDirection::Right
            } else {
                CommandDirection::Left
            };
            match procedure {
                ActionProcedure::Flight => {
                    let definition_id = self.objects[idx].definition_id.clone();
                    if self.objects[idx].state.ocf & crate::ocf::HIT_SPEED3 != 0 || action_disabled
                    {
                        self.object_action_tumble(
                            idx,
                            &definition_id,
                            wall_direction,
                            tumble_x,
                            C4Fixed::ZERO,
                        )?;
                    } else if can_scale {
                        self.object_action_scale(idx, &definition_id, wall_direction)?;
                        return Ok(());
                    }
                }
                ActionProcedure::Walk => {
                    let com_dir = self.objects[idx].state.command_direction;
                    if com_dir_like(com_dir, toward) {
                        if can_scale {
                            let definition_id = self.objects[idx].definition_id.clone();
                            self.object_action_scale(idx, &definition_id, wall_direction)?;
                            return Ok(());
                        }
                        self.objects[idx].state.command_direction = CommandDirection::Stop;
                    }
                    let Some(idx) = self.find_object_index(object_id) else {
                        return Ok(());
                    };
                    if com_dir_like(self.objects[idx].state.command_direction, away) {
                        // Slide off (C4Object.cpp:4437/4491).
                        let xdir = self.objects[idx].fixed_velocity.x / 2;
                        let ydir = self.objects[idx].fixed_velocity.y;
                        let _ = self.object_action_jump(idx, xdir, ydir, false)?;
                    }
                    return Ok(());
                }
                ActionProcedure::Swim => {
                    let com_dir = self.objects[idx].state.command_direction;
                    if com_dir_like(com_dir, toward) && can_scale {
                        let definition_id = self.objects[idx].definition_id.clone();
                        self.object_action_scale(idx, &definition_id, wall_direction)?;
                        return Ok(());
                    }
                    let definition_id = self.objects[idx].definition_id.clone();
                    let _ = self.object_action_corner_scale(idx, &definition_id, procedure)?;
                    return Ok(());
                }
                ActionProcedure::Hang => {
                    if can_scale {
                        let definition_id = self.objects[idx].definition_id.clone();
                        if self.object_action_scale(idx, &definition_id, wall_direction)? {
                            return Ok(());
                        }
                    }
                    if let Some(idx) = self.find_object_index(object_id) {
                        self.objects[idx].state.command_direction = CommandDirection::Stop;
                    }
                    return Ok(());
                }
                ActionProcedure::Dig => {
                    let definition_id = self.objects[idx].definition_id.clone();
                    self.object_com_stop_dig(idx, &definition_id)?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Flight stuck: enforce slide free (C4Object.cpp:4524-4546).
        if matches!(procedure, ActionProcedure::Flight) {
            let Some(idx) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let velocity = self.objects[idx].fixed_velocity;
            if !velocity.y.is_nonzero() {
                let allow_down = i32::from(self.objects[idx].frame_t_contact & CNAT_BOTTOM == 0);
                if self.objects[idx].frame_t_contact & CNAT_RIGHT != 0 {
                    let position = self.objects[idx].state.position;
                    self.force_object_position(
                        idx,
                        Vector2::new(position.x - 1, position.y + allow_down),
                    );
                    let object = &mut self.objects[idx];
                    object.fixed_velocity = FixedVec2::ZERO;
                    object.state.velocity = Vector2::ZERO;
                }
                let Some(idx) = self.find_object_index(object_id) else {
                    return Ok(());
                };
                if self.objects[idx].frame_t_contact & CNAT_LEFT != 0 {
                    let position = self.objects[idx].state.position;
                    self.force_object_position(
                        idx,
                        Vector2::new(position.x + 1, position.y + allow_down),
                    );
                    let object = &mut self.objects[idx];
                    object.fixed_velocity = FixedVec2::ZERO;
                    object.state.velocity = Vector2::ZERO;
                }
            }
            let Some(idx) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let velocity = self.objects[idx].fixed_velocity;
            if !velocity.x.is_nonzero() && self.objects[idx].frame_t_contact & CNAT_TOP != 0 {
                let position = self.objects[idx].state.position;
                self.force_object_position(idx, Vector2::new(position.x, position.y + 1));
                let object = &mut self.objects[idx];
                object.fixed_velocity = FixedVec2::ZERO;
                object.state.velocity = Vector2::ZERO;
            }
        }
        Ok(())
    }

    /// C4Object::ForcePosition (C4Movement.cpp:531-539): fix always
    /// resyncs; movement zeroed by the callers that need it.
    #[doc(hidden)]
    pub fn force_object_position(&mut self, idx: usize, target: Vector2) {
        let position_changed = self.objects[idx].state.position != target;
        let object = &mut self.objects[idx];
        object.fixed_position = FixedVec2::from_ints(target.x, target.y);
        object.state.position = target;
        self.update_sector_for_index(idx);
        if position_changed {
            self.update_solid_mask(idx);
        }
    }

    /// ObjectActionStand (C4ObjectCom.cpp:41-46): set ComDir Stop, then use
    /// ordinary SetActionByName("Walk") and zero both dirs only when that
    /// transition succeeds. Action callbacks may remove or redefine the
    /// object, so every post-callback write resolves its stable id again.
    pub(crate) fn object_action_stand_live(
        &mut self,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        self.objects[idx].state.command_direction = CommandDirection::Stop;
        let definition_id = self.objects[idx].definition_id.clone();
        if !self.action_with_calls(idx, &definition_id, "Walk")? {
            return Ok(false);
        }
        if let Some(idx) = self.find_object_index(object_id) {
            let object = &mut self.objects[idx];
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
        }
        Ok(true)
    }

    fn object_action_stand(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
    ) -> Result<bool, EngineError> {
        self.objects[idx].state.command_direction = CommandDirection::Stop;
        if self.action_with_calls(idx, definition_id, "Walk")? {
            let object = &mut self.objects[idx];
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
            return Ok(true);
        }
        Ok(false)
    }

    /// `ObjectComStop` (C4ObjectCom.cpp:239-245): enter ActIdle first,
    /// then stand in Walk when that action exists. Both transitions are
    /// ordinary and the Walk lookup uses the live post-Idle definition.
    pub(crate) fn object_com_stop_live(
        &mut self,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let definition_id = self.objects[idx].definition_id.clone();
        let _ = self.action_with_calls(idx, &definition_id, "Idle")?;
        self.object_action_stand_live(object_id)
    }

    /// `ObjectComBuild` (C4ObjectCom.cpp:690-697): the target must remain
    /// valid and the builder must be in ActIdle or DFA_WALK; the ordinary,
    /// non-forced Build transition runs all action callbacks synchronously.
    pub(crate) fn object_com_build_live(
        &mut self,
        object_id: ObjectId,
        target_id: ObjectId,
    ) -> Result<bool, EngineError> {
        if self.find_object_index(target_id).is_none() {
            return Ok(false);
        }
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let definition_id = self.objects[idx].definition_id.clone();
        let can_build = self
            .definitions
            .get(&definition_id)
            .is_some_and(|definition| {
                let actions = definition.action_library();
                let action = &self.objects[idx].state.action;
                actions.is_idle_state(action)
                    || actions.procedure_for_entry(&action.name, action.act_map_index)
                        == ActionProcedure::Walk
            });
        if !can_build {
            return Ok(false);
        }
        self.action_with_target_and_calls(idx, &definition_id, "Build", target_id)
    }

    pub(crate) fn object_com_stop_action(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
    ) -> Result<bool, EngineError> {
        self.action_with_calls(idx, definition_id, "Idle")?;
        self.object_action_stand(idx, definition_id)
    }

    /// ObjectActionFlat (C4ObjectCom.cpp:96-102): "FlatUp", dirs zeroed.
    fn object_action_flat(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        direction: Direction,
    ) -> Result<bool, EngineError> {
        if self.action_with_calls(idx, definition_id, "FlatUp")? {
            let object = &mut self.objects[idx];
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
            // Native calls SetDir here, which runs the new action's TurnAction
            // (C4Object.cpp:4243-4248); the bare mirror write skipped it
            // (clonk-org/clonk-rs#1130).
            self.set_exec_action_direction(idx, definition_id, direction)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// ObjectActionScale (C4ObjectCom.cpp:104-110).
    fn object_action_scale(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        direction: Direction,
    ) -> Result<bool, EngineError> {
        if self.action_with_calls(idx, definition_id, "Scale")? {
            let object = &mut self.objects[idx];
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
            // Native calls SetDir here, which runs the new action's TurnAction
            // (C4Object.cpp:4243-4248); the bare mirror write skipped it
            // (clonk-org/clonk-rs#1130).
            self.set_exec_action_direction(idx, definition_id, direction)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// ObjectActionHangle (C4ObjectCom.cpp:112-118).
    fn object_action_hangle(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        direction: Direction,
    ) -> Result<bool, EngineError> {
        if self.action_with_calls(idx, definition_id, "Hangle")? {
            let object = &mut self.objects[idx];
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
            // Native calls SetDir here, which runs the new action's TurnAction
            // (C4Object.cpp:4243-4248); the bare mirror write skipped it
            // (clonk-org/clonk-rs#1130).
            self.set_exec_action_direction(idx, definition_id, direction)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// ObjectActionTumble (C4ObjectCom.cpp:74-80).
    pub(crate) fn object_action_tumble(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        direction: Direction,
        xdir: C4Fixed,
        ydir: C4Fixed,
    ) -> Result<bool, EngineError> {
        if self.action_with_calls(idx, definition_id, "Tumble")? {
            // SetDir, not the trailing assignment: the new action's TurnAction
            // runs on a facing change (clonk-org/clonk-rs#1130).
            self.set_exec_action_direction(idx, definition_id, direction)?;
            let object = &mut self.objects[idx];
            object.fixed_velocity = FixedVec2::new(xdir, ydir);
            object.state.velocity = object.velocity_pixels();
            return Ok(true);
        }
        Ok(false)
    }

    /// ObjectComStopDig (C4ObjectCom.cpp:776-784): Stand + clear a Dig
    /// command at the stack top.
    pub(crate) fn object_com_stop_dig(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
    ) -> Result<(), EngineError> {
        self.object_action_stand(idx, definition_id)?;
        let object = &mut self.objects[idx];
        if object.commands.front_command_name() == Some("Dig") {
            object.commands.clear_front();
        }
        Ok(())
    }

    /// C4Object::ContactCheck (C4Movement.cpp:166-182): perform the shape
    /// probe first, then synchronously dispatch the contacted directions in
    /// Left/Right/Top/Bottom order. A truthy callback stops dispatch for this
    /// probe; engine callback errors are fail-safe.
    fn object_contact_check_at(
        &mut self,
        idx: usize,
        position: Vector2,
    ) -> Result<Option<ShapeContact>, EngineError> {
        let mut contact = {
            let Some(object) = self.objects.get(idx) else {
                return Ok(None);
            };
            let Some(landscape) = self.landscape.as_ref() else {
                return Ok(None);
            };
            let masks = self.live_movement_solid_masks();
            let contact_density = object.state.contact_density;
            shape_contact_check(
                &object.state.vertices,
                position,
                landscape,
                &self.materials,
                &masks,
                Some(object.id),
                contact_density,
            )
        };
        if let Some(object) = self.objects.get_mut(idx) {
            object.latch_shape_contact(&contact);
        }

        self.dispatch_contact_callbacks(idx, MovementContactDispatch::ShapeProbe)?;
        if let Some(object) = self.objects.get(idx) {
            contact.contact_cnat = object.frame_shape_contact_cnat;
            if object.frame_shape_contact_count == 0 {
                contact.contact_count = 0;
            }
        }
        Ok(Some(contact))
    }

    /// The callback half of `C4Object::ContactCheck` (C4Movement.cpp:
    /// 166-183): contacted directions run left/right/top/bottom and a truthy
    /// result stops the remaining calls. Both movement probes and Stabilize's
    /// temporary upright probe use this same path.
    pub(crate) fn dispatch_contact_callbacks(
        &mut self,
        idx: usize,
        dispatch: MovementContactDispatch,
    ) -> Result<(), EngineError> {
        let Some(object_id) = self.objects.get(idx).map(|object| object.id) else {
            return Ok(());
        };
        let (directions, shape_probe) = match dispatch {
            MovementContactDispatch::ShapeProbe => {
                ([CNAT_LEFT, CNAT_RIGHT, CNAT_TOP, CNAT_BOTTOM], true)
            }
            MovementContactDispatch::Direct(cnat) => ([cnat, 0, 0, 0], false),
        };
        for cnat in directions {
            if cnat == CNAT_NONE {
                continue;
            }
            let Some(idx) = self.find_object_index(object_id) else {
                break;
            };
            if self.objects[idx].state.status == ObjectStatus::Deleted {
                break;
            }
            if shape_probe && self.objects[idx].frame_shape_contact_cnat & cnat == 0 {
                continue;
            }
            #[cfg(test)]
            crate::engine_movement::record_movement_contact_invocation();
            let Some(function_name) = contact_callback_name(cnat) else {
                continue;
            };
            let Some((callback_definition_id, action_library, has_function)) =
                self.objects.get(idx).and_then(|object| {
                    self.definitions
                        .get(&object.definition_id)
                        .map(|definition| {
                            (
                                object.definition_id.clone(),
                                definition.shared_action_library_handle(),
                                definition.has_function(function_name),
                            )
                        })
                })
            else {
                break;
            };
            if !has_function {
                continue;
            }
            let contact_calls = self
                .definitions
                .get(&callback_definition_id)
                .map(|definition| definition.contact_function_calls())
                .unwrap_or(false);
            if !contact_calls {
                continue;
            }
            match tolerate_script_error(self.call_movement_object_function(
                idx,
                function_name,
                &[],
                &action_library,
                object_id,
                &callback_definition_id,
            ))? {
                Some(value) if value.as_bool() => break,
                Some(_) | None => {}
            }
        }
        Ok(())
    }

    /// ObjectActionCornerScale (C4ObjectCom.cpp:167-217): probe a free
    /// spot up-and-sideways, then KneelUp (Walk fallback) with the fixed
    /// coords shifted.
    #[doc(hidden)]
    pub fn object_action_corner_scale(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        procedure: ActionProcedure,
    ) -> Result<bool, EngineError> {
        const CORNER_RANGE: i32 = ATTACH_RANGE + 2;
        let corner_okay =
            |engine: &mut Engine, range_x: i32, range_y: i32| -> Result<bool, EngineError> {
                let (position, direction) = {
                    let object = &engine.objects[idx];
                    (object.state.position, object.state.direction)
                };
                let cty = position.y - range_y;
                let ctx = if direction == Direction::Left {
                    position.x - range_x
                } else {
                    position.x + range_x
                };
                // C4Object::ContactCheck reads the live landscape on every
                // corner candidate. A Contact* callback from the previous
                // probe can change another object's mask before this one.
                Ok(engine
                    .object_contact_check_at(idx, Vector2::new(ctx, cty))?
                    .is_some_and(|contact| contact.contact_cnat == 0))
            };
        let (range_x, range_y) = if matches!(procedure, ActionProcedure::Scale) {
            // Scaling: range max to min (CheckCornerScale).
            let mut found = None;
            'outer: for range_x in (1..=CORNER_RANGE).rev() {
                for range_y in (1..=CORNER_RANGE).rev() {
                    if corner_okay(self, range_x, range_y)? {
                        found = Some((range_x, range_y));
                        break 'outer;
                    }
                }
            }
            match found {
                Some(ranges) => ranges,
                None => return Ok(false),
            }
        } else {
            // Swimming: range min to max.
            let mut range = 2;
            while !corner_okay(self, range, range)? {
                range += 1;
                if range > CORNER_RANGE {
                    return Ok(false);
                }
            }
            (range, range)
        };
        if !self.action_with_calls(idx, definition_id, "KneelUp")? {
            self.action_with_calls(idx, definition_id, "Walk")?;
        }
        let object = &mut self.objects[idx];
        object.fixed_velocity = FixedVec2::ZERO;
        object.state.velocity = Vector2::ZERO;
        if object.state.direction == Direction::Left {
            object.fixed_position.x -= itofix(range_x);
        } else {
            object.fixed_position.x += itofix(range_x);
        }
        object.fixed_position.y -= itofix(range_y);
        object.state.position = Vector2::new(
            fixtoi(object.fixed_position.x),
            fixtoi(object.fixed_position.y),
        );
        self.update_sector_for_index(idx);
        Ok(true)
    }

    /// C++'s shared `GrabLost` helper: notify the current action target,
    /// then clear commands above the first PushTo in the pusher's live stack.
    /// The callback may replace that stack, so the clear must follow it.
    fn grab_lost(&mut self, pusher_id: ObjectId) -> Result<(), EngineError> {
        let Some(target_id) = self
            .find_object_index(pusher_id)
            .and_then(|index| self.objects[index].state.action.target)
        else {
            return Ok(());
        };
        let Some(target_idx) = self.find_object_index(target_id) else {
            return Ok(());
        };

        let _ =
            tolerate_script_error(self.call_object_function(target_idx, "GrabLost", Vec::new()))?;
        if let Some(pusher_idx) = self.find_object_index(pusher_id) {
            self.objects[pusher_idx].commands.clear_to_first_push_to();
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn apply_no_attach_action(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        library: &ActionLibrary,
    ) -> Result<(), EngineError> {
        if idx >= self.objects.len() {
            return Ok(());
        }

        let object_id = self.objects[idx].id;
        let initial_action = self.objects[idx].state.action.clone();
        if library.is_idle_state(&initial_action) {
            // Inactive objects: simple mobile natural gravity
            // (C4Object.cpp:4299-4303) — DoGravity's free-fall branch
            // still skips StaticBack (:4662).
            let gravity = self.physics.gravity_as_c4fixed();
            let object = &mut self.objects[idx];
            if object.state.category & CATEGORY_STATIC_BACK == 0 {
                object.fixed_velocity.y += gravity;
                object.refresh_velocity_from_fixed();
            }
            object.state.mobile = true;
            return Ok(());
        }

        let procedure =
            library.procedure_for_entry(&initial_action.name, initial_action.act_map_index);
        let command_direction = self.objects[idx].state.command_direction;
        let direction = self.objects[idx].state.direction;
        let scaling_upward = matches!(
            command_direction,
            CommandDirection::Up | CommandDirection::UpLeft | CommandDirection::UpRight
        ) || (command_direction == CommandDirection::Left
            && direction == Direction::Left)
            || (command_direction == CommandDirection::Right && direction == Direction::Right);
        if matches!(procedure, ActionProcedure::Scale) && scaling_upward {
            let corner_scaled =
                self.object_action_corner_scale(idx, definition_id, ActionProcedure::Scale)?;
            if corner_scaled {
                // Scaling upward tries the corner transition before the generic
                // Jump fallback (C4Object.cpp:4282-4289).
                return Ok(());
            }
        }

        // A stopped scaler that loses its wall first tries to jump away from
        // it. A failed SetAction falls through to the generic jump below, so
        // OnActionJump can run twice in that narrow case (C4Object.cpp:
        // 4290-4299).
        if matches!(procedure, ActionProcedure::Scale) {
            let stopped_scale = self.find_object_index(object_id).and_then(|live_idx| {
                let state = &self.objects[live_idx].state;
                (state.command_direction == CommandDirection::Stop).then_some((
                    live_idx,
                    if state.direction == Direction::Left {
                        itofix(1)
                    } else {
                        itofix(-1)
                    },
                ))
            });
            if let Some((live_idx, xdir)) = stopped_scale {
                if self.object_action_jump(live_idx, xdir, C4Fixed::ZERO, false)? {
                    return Ok(());
                }
            }
        }

        // Pushing off an attachment first notifies the vehicle and restores
        // the PushTo command before falling through to ObjectActionJump
        // (C4Object.cpp:4302-4306).
        if matches!(procedure, ActionProcedure::Push) {
            self.grab_lost(object_id)?;
        } else if matches!(procedure, ActionProcedure::Fight) {
            // Losing the ground during a fight records the opponent's live
            // Controller for a later fall death. This is an unconditional
            // direct assignment in C++ (including NO_OWNER), not the guarded
            // DoEnergy kill-trace update (C4Object.cpp:4304-4305).
            let opponent_controller = initial_action
                .target
                .and_then(|target_id| self.find_object_index(target_id))
                .map(|target_idx| self.objects[target_idx].state.controller);
            if let Some(controller) = opponent_controller {
                self.objects[idx].last_energy_loss_cause = controller;
            }
        }

        // GrabLost and corner-probe callbacks are synchronous. The ensuing
        // ObjectActionJump resolves the pusher's live definition and action.
        let Some(idx) = self.find_object_index(object_id).filter(|&index| {
            !self.objects[index].destroyed
                && !matches!(self.objects[index].state.status, ObjectStatus::Deleted)
        }) else {
            return Ok(());
        };
        let launch = self.objects[idx].fixed_velocity;
        let _ = self.object_action_jump(idx, launch.x, launch.y, false)?;
        Ok(())
    }

    /// The status-gated tail shared by DFA_ATTACH's two lost-target arms:
    /// ordinary SetAction(ActIdle), then AttachTargetLost on the same live
    /// object. The action transition may be denied or remove the object; its
    /// result is deliberately ignored (C4Object.cpp:5317-5325, 5341-5349).
    fn notify_attach_target_lost(&mut self, object_id: ObjectId) -> Result<(), EngineError> {
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(());
        };
        if self.objects[idx].state.status == ObjectStatus::Deleted {
            return Ok(());
        }
        let definition_id = self.objects[idx].definition_id.clone();
        let _ = tolerate_script_error(self.action_with_calls(idx, &definition_id, "Idle"))?;
        if let Some(idx) = self.find_object_index(object_id) {
            let _ = tolerate_script_error(self.call_object_function(
                idx,
                "AttachTargetLost",
                Vec::new(),
            ))?;
        }
        Ok(())
    }

    pub(crate) fn apply_attach_procedure(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
    ) -> Result<bool, EngineError> {
        let object_id = self.objects[idx].id;
        let Some(target_id) = self.objects[idx].state.action.target else {
            self.notify_attach_target_lost(object_id)?;
            return Ok(false);
        };

        let Some(target_idx) = self.find_object_index(target_id).filter(|&target_idx| {
            !self.objects[target_idx].destroyed
                && self.objects[target_idx].state.status != ObjectStatus::Deleted
        }) else {
            // AssignRemoval normally clears Action.Target before ExecAction.
            // Keep malformed/dangling fixture state equivalent to that null
            // pointer before dispatching the native lost-target sequence.
            if let Some(idx) = self.find_object_index(object_id) {
                if self.objects[idx].state.action.target == Some(target_id) {
                    self.objects[idx].state.action.target = None;
                    self.objects[idx].compiler_cache.action_target1 = 0;
                }
            }
            self.notify_attach_target_lost(object_id)?;
            return Ok(false);
        };

        let target_is_incomplete = self.objects[target_idx].state.ocf & ocf::FULL_CON == 0;
        let target_allows_incomplete_activity = self
            .definitions
            .get(&self.objects[target_idx].definition_id)
            .is_some_and(Definition::incomplete_activity);
        if target_is_incomplete && !target_allows_incomplete_activity {
            let _ = tolerate_script_error(self.action_with_calls(idx, definition_id, "Idle"))?;
            return Ok(false);
        }

        let target_container = self.objects[target_idx].state.container;
        let previous_container = self.objects[idx].state.container;
        if previous_container != target_container {
            match target_container {
                Some(container_id) => {
                    // Enter's bool result is intentionally ignored. A veto or
                    // callback re-entry does not stop DFA_ATTACH from using
                    // the live Action.Target below.
                    let _ = self.try_object_enter(object_id, container_id)?;
                }
                None => {
                    // This exact arm is Exit(x, y, r): preserve the current
                    // rotation, zero all dirs, and run Ejection/Departure.
                    let _ = self.exit_object_at_current_transform(object_id)?;
                }
            }

            let Some(live_idx) = self.find_object_index(object_id) else {
                return Ok(false);
            };
            if self.objects[live_idx].state.action.target.is_none() {
                self.notify_attach_target_lost(object_id)?;
                return Ok(false);
            }
        }

        // Enter/Exit callbacks may retarget, ChangeDef, change Action.Data,
        // or replace either shape. Native DFA_ATTACH rereads all of those
        // live fields, but deliberately does not repeat its completeness or
        // containment checks (C4Object.cpp:5353-5359).
        let Some(live_idx) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let Some(live_target_id) = self.objects[live_idx].state.action.target else {
            self.notify_attach_target_lost(object_id)?;
            return Ok(false);
        };
        let Some(live_target_idx) = self
            .find_object_index(live_target_id)
            .filter(|&target_idx| {
                !self.objects[target_idx].destroyed
                    && self.objects[target_idx].state.status != ObjectStatus::Deleted
            })
        else {
            if self.objects[live_idx].state.action.target == Some(live_target_id) {
                self.objects[live_idx].state.action.target = None;
                self.objects[live_idx].compiler_cache.action_target1 = 0;
            }
            self.notify_attach_target_lost(object_id)?;
            return Ok(false);
        };

        let target_position = self.objects[live_target_idx].state.position;
        let target_vertices = self.objects[live_target_idx].state.vertices.clone();
        let object_vertices = self.objects[live_idx].state.vertices.clone();
        let action_data = self.objects[live_idx].state.action.data as u32;
        let self_vertex_index = ((action_data >> 8) & 0xFF) as usize;
        let target_vertex_index = (action_data & 0xFF) as usize;

        let self_offset = object_vertices
            .get(self_vertex_index)
            .map(|vertex| Vector2::new(vertex.x, vertex.y))
            .unwrap_or(Vector2::ZERO);
        let target_offset = target_vertices
            .get(target_vertex_index)
            .map(|vertex| Vector2::new(vertex.x, vertex.y))
            .unwrap_or(Vector2::ZERO);
        let new_position = Vector2::new(
            target_position
                .x
                .saturating_add(target_offset.x)
                .saturating_sub(self_offset.x),
            target_position
                .y
                .saturating_add(target_offset.y)
                .saturating_sub(self_offset.y),
        );

        if std::env::var("LC_ATTDBG").is_ok() && (21..=22).contains(&self.frame) {
            eprintln!(
                "ATTDBG f{} obj={} tgt_pos={:?} tgt_off={:?} self_off={:?} data={:#x} -> {:?}",
                self.frame,
                object_id.as_u64(),
                target_position,
                target_offset,
                self_offset,
                action_data,
                new_position
            );
        }
        self.force_object_position(live_idx, new_position);
        let object = &mut self.objects[live_idx];
        object.fixed_velocity.x = C4Fixed::ZERO;
        object.fixed_velocity.y = C4Fixed::ZERO;
        object.state.velocity.x = 0;
        object.state.velocity.y = 0;

        Ok(true)
    }

    /// DFA_PULL with a nonzero Walk physical (C4Object.cpp:5099-5170):
    /// pull-position math, target force via `C4Object::Push`, ComDir
    /// transfer onto walking/pulling targets, pulling-range check with
    /// GrabLost and target loss, own move toward the pulling position. The
    /// controller transfer (C4Object.cpp:5157).
    fn apply_pull_procedure_physical(
        &mut self,
        idx: usize,
        target_idx: usize,
        command_direction: CommandDirection,
        _definition_id: &DefinitionId,
        physical: PhysicalInfo,
        phase_advance: &mut Option<i32>,
    ) -> Result<bool, EngineError> {
        let puller_id = self.objects[idx].id;
        let position = self.objects[idx].state.position;
        let target_position = self.objects[target_idx].state.position;
        let own_width = self.objects[idx]
            .current_shape_rect()
            .map(|rect| rect.width)
            .unwrap_or(0);
        let target_rect = self.objects[target_idx]
            .current_shape_rect()
            .unwrap_or_default();
        // Pulling positions (C4Object.cpp:5110-5118).
        let pull_distance = target_rect.width / 2 + own_width / 2;
        let mut target_x = position.x;
        let mut pull_x = target_position.x;
        match command_direction {
            CommandDirection::Right => {
                target_x = target_position.x + pull_distance;
                pull_x = position.x - pull_distance;
            }
            CommandDirection::Left => {
                target_x = target_position.x - pull_distance;
                pull_x = position.x + pull_distance;
            }
            _ => {}
        }
        // Target pulling force (C4Object.cpp:5120-5126).
        let walk = math::val_by_physical(280, physical.walk);
        let movement = match command_direction {
            CommandDirection::Right => walk,
            CommandDirection::Left => -walk,
            _ => C4Fixed::ZERO,
        };
        let txdir = movement + walk * (pull_x - target_position.x).clamp(-10, 10) / 10;
        // Push object (C4Object.cpp:5129-5132).
        if !self.push_object(
            target_idx,
            txdir,
            math::val_by_physical(250, physical.push),
            false,
        )? {
            self.stop_action_delay_command_live(puller_id)?;
            return Ok(false);
        }
        // Successful target force transfers attribution before the train
        // and range checks, even when the pull is lost below
        // (C4Object.cpp:5157).
        self.objects[target_idx].state.controller = self.objects[idx].state.controller;
        // Train pulling: ComDir transfer (C4Object.cpp:5137-5143).
        let target_procedure = self
            .definitions
            .get(&self.objects[target_idx].definition_id)
            .map(|definition| {
                definition.action_library().procedure_for_entry(
                    &self.objects[target_idx].state.action.name,
                    self.objects[target_idx].state.action.act_map_index,
                )
            })
            .unwrap_or_default();
        if matches!(
            target_procedure,
            ActionProcedure::Walk | ActionProcedure::Pull
        ) {
            let mut transfer = CommandDirection::Stop;
            if txdir < C4Fixed::ZERO {
                transfer = CommandDirection::Left;
            }
            if txdir > C4Fixed::ZERO {
                transfer = CommandDirection::Right;
            }
            self.objects[target_idx].state.command_direction = transfer;
        }
        // Pulling range (C4Object.cpp:5146-5161).
        let push_distance = (own_width / 2 - 8).max(0);
        let push_range = push_distance + 20;
        let sax = target_position.x + target_rect.x;
        let say = target_position.y + target_rect.y;
        let sawdt = target_rect.width;
        let sahgt = target_rect.height;
        if !(-push_range..=sawdt - 1 + push_range).contains(&(position.x - sax))
            || !(-push_range..=sahgt - 1 + push_range).contains(&(position.y - say))
        {
            // Stop and queue the delay before notifying the actor's live
            // target. GrabLost may trim that new Wait back to PushTo and may
            // retarget the puller; C++ clears the resulting live target last
            // (C4Object.cpp:5177-5181).
            self.stop_action_delay_command_live(puller_id)?;
            self.grab_lost(puller_id)?;
            if let Some(puller_idx) = self.find_object_index(puller_id) {
                self.objects[puller_idx].state.action.target = None;
                self.objects[puller_idx].compiler_cache.action_target1 = 0;
            }
            return Ok(false);
        }
        // Move to pulling position (C4Object.cpp:5164), facing by xdir,
        // grounded.
        let xdir = movement + walk * (target_x - position.x).clamp(-10, 10) / 10;
        let physics = self.physics;
        self.objects[idx].fixed_velocity.x = xdir;
        let (live_idx, advance) = self.set_exec_direction_from_xdir_live(puller_id, 0)?;
        *phase_advance = Some(advance);
        let Some(idx) = live_idx else {
            return Ok(false);
        };
        let object = &mut self.objects[idx];
        object.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut object.fixed_velocity);
        object.refresh_velocity_from_fixed();
        // Attachment (C4Object.cpp:5194-5196): after SetDir, so the
        // TurnAction StartCall above still observed the pre-write register.
        object.frame_t_attach |= CNAT_BOTTOM;
        object.state.t_attach = object.frame_t_attach;
        Ok(true)
    }

    pub(crate) fn apply_pull_procedure(
        &mut self,
        idx: usize,
        command_direction: CommandDirection,
        movement_profile: MovementProfile,
        definition_id: &DefinitionId,
        physical: PhysicalInfo,
        phase_advance: &mut Option<i32>,
    ) -> Result<bool, EngineError> {
        let puller_id = self.objects[idx].id;
        let Some(target_id) = self.objects[idx].state.action.target else {
            self.stop_action_delay_command_live(puller_id)?;
            return Ok(false);
        };

        let Some(target_idx) = self.find_object_index(target_id) else {
            self.stop_action_delay_command_live(puller_id)?;
            return Ok(false);
        };

        if target_idx == idx {
            self.stop_action_delay_command_live(puller_id)?;
            return Ok(false);
        }

        let puller_container = self.objects[idx].state.container;
        if puller_container == Some(target_id) {
            self.stop_action_delay_command_live(puller_id)?;
            return Ok(false);
        }

        let target_removed = {
            let target = &self.objects[target_idx];
            target.destroyed || matches!(target.state.status, ObjectStatus::Deleted)
        };
        if target_removed {
            self.stop_action_delay_command_live(puller_id)?;
            return Ok(false);
        }

        if self.objects[target_idx].state.container.is_some() {
            self.stop_action_delay_command_live(puller_id)?;
            return Ok(false);
        }

        if physical.walk != 0 {
            return self.apply_pull_procedure_physical(
                idx,
                target_idx,
                command_direction,
                definition_id,
                physical,
                phase_advance,
            );
        }

        let walk_speed = movement_profile.walk_speed.max(0);
        let walk_accel = movement_profile.walk_acceleration.max(0);

        let puller_position = self.objects[idx].state.position;
        let target_position = self.objects[target_idx].state.position;

        let (puller_half_width, _) = Self::object_half_extents(&self.objects[idx]);
        let (target_half_width, target_half_height) =
            Self::object_half_extents(&self.objects[target_idx]);
        let pull_distance = puller_half_width.saturating_add(target_half_width);

        let horizontal_input = Self::horizontal_input_sign(command_direction);
        let base_velocity = horizontal_input.saturating_mul(walk_speed);

        let desired_puller_x = if horizontal_input == 0 {
            puller_position.x
        } else {
            target_position
                .x
                .saturating_add(horizontal_input.saturating_mul(pull_distance))
        };
        let desired_target_x = if horizontal_input == 0 {
            target_position.x
        } else {
            puller_position
                .x
                .saturating_sub(horizontal_input.saturating_mul(pull_distance))
        };

        let desired_target_velocity = Self::desired_pull_velocity(
            target_position.x,
            desired_target_x,
            base_velocity,
            walk_speed,
        );
        let desired_puller_velocity = Self::desired_pull_velocity(
            puller_position.x,
            desired_puller_x,
            base_velocity,
            walk_speed,
        );

        // Mirror C++ pull range tolerance: stay close enough to the target to keep the rope taut.
        let range_extension = puller_half_width.saturating_sub(8).max(0) + 20;
        let horizontal_gap_limit = target_half_width.saturating_add(range_extension);
        let vertical_gap_limit = target_half_height.saturating_add(range_extension);

        let horizontal_gap = (puller_position.x as i64 - target_position.x as i64).abs() as i32;
        let vertical_gap = (puller_position.y as i64 - target_position.y as i64).abs() as i32;
        if horizontal_gap > horizontal_gap_limit || vertical_gap > vertical_gap_limit {
            self.stop_action_delay_command_live(puller_id)?;
            self.grab_lost(puller_id)?;
            if let Some(puller_idx) = self.find_object_index(puller_id) {
                self.objects[puller_idx].state.action.target = None;
                self.objects[puller_idx].compiler_cache.action_target1 = 0;
            }
            return Ok(false);
        }

        let speed_limit = walk_speed.saturating_mul(2).max(walk_speed);
        let physics = self.physics;

        match idx.cmp(&target_idx) {
            std::cmp::Ordering::Less => {
                let (first, second) = self.objects.split_at_mut(target_idx);
                let puller = &mut first[idx];
                let target = &mut second[0];
                Self::update_pull_pair(
                    puller,
                    target,
                    desired_puller_velocity,
                    desired_target_velocity,
                    speed_limit,
                    walk_accel,
                    physics,
                );
            }
            std::cmp::Ordering::Greater => {
                let (first, second) = self.objects.split_at_mut(idx);
                let target = &mut first[target_idx];
                let puller = &mut second[0];
                Self::update_pull_pair(
                    puller,
                    target,
                    desired_puller_velocity,
                    desired_target_velocity,
                    speed_limit,
                    walk_accel,
                    physics,
                );
            }
            std::cmp::Ordering::Equal => {
                // Should not happen because we guard earlier, but keep the action safe.
                self.stop_action_delay_command_live(puller_id)?;
                return Ok(false);
            }
        }

        let (live_idx, advance) = self.set_exec_direction_from_xdir_live(puller_id, 0)?;
        *phase_advance = Some(advance);
        let Some(idx) = live_idx else {
            return Ok(false);
        };
        // Attachment (C4Object.cpp:5194-5196).
        let object = &mut self.objects[idx];
        object.frame_t_attach |= CNAT_BOTTOM;
        object.state.t_attach = object.frame_t_attach;

        Ok(true)
    }

    pub(crate) fn apply_fight_procedure(
        &mut self,
        idx: usize,
        physical: PhysicalInfo,
    ) -> Result<bool, EngineError> {
        let fighter_id = self.objects[idx].id;
        let Some(target_id) = self.objects[idx].state.action.target else {
            let _ = self.object_action_stand_live(fighter_id)?;
            return Ok(false);
        };

        let Some(target_idx) = self.find_object_index(target_id) else {
            let _ = self.object_action_stand_live(fighter_id)?;
            return Ok(false);
        };

        if target_idx == idx {
            let _ = self.object_action_stand_live(fighter_id)?;
            return Ok(false);
        }

        let target_definition_id = self.objects[target_idx].definition_id.clone();
        let target_action_name = self.objects[target_idx].state.action.name.clone();
        let target_action_index = self.objects[target_idx].state.action.act_map_index;
        let target_procedure = self
            .definitions
            .get(&target_definition_id)
            .map(|definition| {
                definition
                    .action_library()
                    .procedure_for_entry(&target_action_name, target_action_index)
            })
            .unwrap_or_default();
        if !matches!(target_procedure, ActionProcedure::Fight) {
            let _ = self.object_action_stand_live(fighter_id)?;
            return Ok(false);
        }

        let fighter_container = self.objects[idx].state.container;
        let target_container = self.objects[target_idx].state.container;
        let fighter_container_is_closed = fighter_container
            .and_then(|container_id| self.find_object_index(container_id))
            .is_some_and(|container_idx| !self.objects[container_idx].state.entrance_status);
        let target_container_is_closed = target_container
            .and_then(|container_id| self.find_object_index(container_id))
            .is_some_and(|container_idx| !self.objects[container_idx].state.entrance_status);
        if fighter_container != target_container
            && (fighter_container_is_closed || target_container_is_closed)
        {
            let _ = self.object_action_stand_live(fighter_id)?;
            return Ok(false);
        }

        let fighter_position = self.objects[idx].state.position;
        let target_position = self.objects[target_idx].state.position;

        // Physical training (C4Object.cpp:5214-5216): Tick5 trains Fight.
        if self.frame.is_multiple_of(5) {
            self.train_physical(idx, "Fight", 1, C4_MAX_PHYSICAL);
        }

        // Direction (C4Object.cpp:5241-5243): these are independent tests,
        // not an assignment from velocity. Equal x calls SetDir zero times;
        // after a right-facing TurnAction, the left test reads the live actor
        // and Action.Target again.
        if target_position.x > fighter_position.x {
            let definition_id = self.objects[idx].definition_id.clone();
            self.set_exec_action_direction(idx, &definition_id, Direction::Right)?;
        }
        let Some(idx) = self.find_object_index(fighter_id) else {
            return Ok(false);
        };
        let Some(target_id) = self.objects[idx].state.action.target else {
            return Ok(false);
        };
        let Some(target_idx) = self.find_object_index(target_id) else {
            return Ok(false);
        };
        if self.objects[target_idx].state.position.x < self.objects[idx].state.position.x {
            let definition_id = self.objects[idx].definition_id.clone();
            self.set_exec_action_direction(idx, &definition_id, Direction::Left)?;
        }
        let Some(idx) = self.find_object_index(fighter_id) else {
            return Ok(false);
        };
        let Some(target_id) = self.objects[idx].state.action.target else {
            return Ok(false);
        };
        let Some(target_idx) = self.find_object_index(target_id) else {
            return Ok(false);
        };
        let fighter_position = self.objects[idx].state.position;
        let target_position = self.objects[target_idx].state.position;
        let direction = self.objects[idx].state.direction;

        // Position (C4Object.cpp:5244-5251): stand beside the target at half
        // its shape width + 2, approach with the Walk physical:
        // lLimit = ValByPhysical(95, Walk), Towards(xdir, ±lLimit, lLimit).
        let target_half_width = self.objects[target_idx]
            .current_shape_rect()
            .unwrap_or_default()
            .width
            / 2;
        let mut approach_x = fighter_position.x;
        if direction == Direction::Left {
            approach_x = target_position.x + target_half_width + 2;
        }
        if direction == Direction::Right {
            approach_x = target_position.x - target_half_width - 2;
        }
        let limit = math::val_by_physical(95, physical.walk);
        let physics = self.physics;
        let fighter = &mut self.objects[idx];
        let mut xdir = fighter.fixed_velocity.x;
        match fighter_position.x.cmp(&approach_x) {
            std::cmp::Ordering::Equal => math::towards(&mut xdir, C4Fixed::ZERO, limit),
            std::cmp::Ordering::Less => math::towards(&mut xdir, limit, limit),
            std::cmp::Ordering::Greater => math::towards(&mut xdir, -limit, limit),
        }
        fighter.fixed_velocity.x = xdir;

        // Distance check (C4Object.cpp:5229-5234): own shape width bounds.
        let threshold = self.objects[idx]
            .current_shape_rect()
            .unwrap_or_default()
            .width;
        if (fighter_position.x - target_position.x).abs() > threshold
            || (fighter_position.y - target_position.y).abs() > threshold
        {
            let _ = self.object_action_stand_live(fighter_id)?;
            return Ok(false);
        }

        // Other (C4Object.cpp:5235-5238): grounded fighting and Tick35
        // experience after every validity check above has succeeded. The
        // attachment lands before the experience call, whose promotion
        // callback can read Action.t_attach.
        let fighter_id = self.objects[idx].id;
        let fighter = &mut self.objects[idx];
        fighter.frame_t_attach |= CNAT_BOTTOM;
        fighter.state.t_attach = fighter.frame_t_attach;
        fighter.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut fighter.fixed_velocity);
        fighter.refresh_velocity_from_fixed();
        if self.frame.is_multiple_of(35) {
            self.do_object_experience(fighter_id, 2);
        }

        Ok(true)
    }

    /// `C4Object::Push` (C4Object.cpp:1758-1808): grab/containment checks,
    /// force scaled against the target mass, facing from the current motion,
    /// xdir worked toward the push speed (close-enough-set), straightening
    /// for upright pushes, and the final Tick35 stuck probe.
    /// The LIVE object mass (C4Object::UpdateMass, C4Object.cpp:497-505).
    /// Objects.txt compiles the cached Mass word verbatim; only native
    /// UpdateMass paths replace it with the derived own-plus-contents value.
    pub(crate) fn valid_compiled_object_mass(&self, index: usize) -> Option<i32> {
        fn valid(engine: &Engine, index: usize, visiting: &mut HashSet<ObjectId>) -> bool {
            let object = &engine.objects[index];
            let retained_contents = object
                .state
                .contents
                .iter()
                .copied()
                .filter(|content| {
                    engine
                        .find_object_index(*content)
                        .is_some_and(|index| engine.objects[index].has_nonzero_status())
                })
                .collect::<Vec<_>>();
            if object.compiled_mass.is_none()
                || object.compiled_mass_contents.len() != retained_contents.len()
                || !object
                    .compiled_mass_contents
                    .iter()
                    .all(|id| retained_contents.contains(id))
            {
                return false;
            }
            if !visiting.insert(object.id) {
                return true;
            }
            let valid = retained_contents.iter().all(|content| {
                engine
                    .find_object_index(*content)
                    .is_some_and(|index| valid(engine, index, visiting))
            });
            visiting.remove(&object.id);
            valid
        }

        valid(self, index, &mut HashSet::new()).then_some(self.objects[index].compiled_mass?)
    }

    pub(crate) fn effective_object_mass(&self, index: usize) -> i32 {
        fn inner(
            engine: &Engine,
            index: usize,
            is_root: bool,
            visiting: &mut HashSet<ObjectId>,
        ) -> i32 {
            let object = &engine.objects[index];
            if let Some(mass) = engine.valid_compiled_object_mass(index) {
                return mass;
            }
            if !visiting.insert(object.id) {
                return 1;
            }
            let (def_mass, no_component_mass) = engine
                .definitions
                .get(&object.definition_id)
                .map(|definition| (definition.mass(), definition.no_component_mass()))
                .unwrap_or((0, false));
            // (Def->Mass + OwnMass) * Con / FullCon (C4Object.cpp:499) —
            // OwnMass carries script SetMass overrides (the ArrowPack
            // family: SetMass(GetMass(item)*PackCount), ArrowPack.c4d).
            let mut mass = ((def_mass + object.state.own_mass)
                .saturating_mul(object.state.construction)
                / FULL_CON)
                .max(1);
            if !no_component_mass {
                for content in &object.state.contents {
                    if let Some(content_idx) = engine
                        .find_object_index(*content)
                        .filter(|&index| engine.objects[index].has_nonzero_status())
                    {
                        let m = inner(engine, content_idx, false, visiting);
                        if is_root
                            && object.state.contents.len() > 20
                            && std::env::var("LC_MASSDBG").is_ok()
                        {
                            eprintln!(
                                "MASSDBG {} {}",
                                engine.objects[content_idx].definition_id.as_str(),
                                m
                            );
                        }
                        mass += m;
                    }
                }
            }
            visiting.remove(&object.id);
            mass
        }
        inner(self, index, true, &mut HashSet::new())
    }

    fn push_object(
        &mut self,
        target_idx: usize,
        txdir: C4Fixed,
        dforce: C4Fixed,
        straighten: bool,
    ) -> Result<bool, EngineError> {
        {
            let target = &self.objects[target_idx];
            if !target.has_nonzero_status() || target.state.container.is_some() {
                return Ok(false);
            }
        }
        if self.object_ocf_at_index(target_idx) & ocf::GRAB == 0 {
            return Ok(false);
        }
        // dforce divides by the LIVE Mass incl. contents (C4Object::Push
        // C4Object.cpp:1770 uses this->Mass; UpdateMass :497-505).
        let live_mass = self.effective_object_mass(target_idx);
        let (grab, mass) = self
            .definitions
            .get(&self.objects[target_idx].definition_id)
            .map(|definition| (definition.grab(), live_mass))
            .unwrap_or((0, 0));
        // Grabbing okay, no pushing (C4Object.cpp:1763).
        if grab == 2 {
            return Ok(true);
        }
        // General pushing force vs. object mass (C4Object.cpp:1770).
        let dforce = dforce * 100 / mass.max(1);
        let target_id = self.objects[target_idx].id;
        let target_definition_id = self.objects[target_idx].definition_id.clone();
        let direction = {
            let target = &mut self.objects[target_idx];
            // Mobilization check - pre-mobilization zero
            // (C4Object.cpp:1765-1768): a resting target starts from clean
            // dirs and pixel-snapped fix.
            if !target.state.mobile {
                target.fixed_velocity = FixedVec2::ZERO;
                target.fixed_position = FixedVec2::new(
                    itofix(target.state.position.x),
                    itofix(target.state.position.y),
                );
            }
            // SetDir reads the raw pre-force xdir. End this borrow before the
            // callback-capable TurnAction transition.
            if target.fixed_velocity.x < C4Fixed::ZERO {
                Some(Direction::Left)
            } else if target.fixed_velocity.x > C4Fixed::ZERO {
                Some(Direction::Right)
            } else {
                None
            }
        };
        if let Some(direction) = direction {
            self.set_command_action_direction(target_idx, &target_definition_id, direction)?;
        }
        let Some(target_idx) = self.find_object_index(target_id) else {
            return Ok(false);
        };
        let target = &mut self.objects[target_idx];
        // Work towards txdir (C4Object.cpp:1775-1783).
        let mut xdir = target.fixed_velocity.x;
        math::towards(&mut xdir, txdir, dforce);
        if std::env::var("LC_PULLDBG").is_ok() && (19..=22).contains(&self.frame) {
            eprintln!(
                "PULLDBG f{} target={} {} -> {} (txdir {} dforce {})",
                self.frame,
                target.id.as_u64(),
                target.fixed_velocity.x.val(),
                xdir.val(),
                txdir.val(),
                dforce.val()
            );
        }
        target.fixed_velocity.x = xdir;
        // Straighten (C4Object.cpp:1785-1794); the normalized rotation maps
        // back to the C++ signed range.
        if straighten {
            let rotation = target.state.rotation;
            let signed = if rotation > 180 {
                rotation - 360
            } else {
                rotation
            };
            if (-math::STABLE_RANGE..=math::STABLE_RANGE).contains(&signed) {
                target.rotation_velocity = C4Fixed::ZERO;
            } else if signed > 0 {
                if target.rotation_velocity > -math::ROTATE_ACCEL {
                    target.rotation_velocity -= dforce;
                }
            } else if target.rotation_velocity < math::ROTATE_ACCEL {
                target.rotation_velocity += dforce;
            }
        }
        // Mobilization check (C4Object.cpp:1797): any nonzero dir after the
        // force application mobilizes the target.
        if target.fixed_velocity.x.is_nonzero()
            || target.fixed_velocity.y.is_nonzero()
            || target.rotation_velocity.is_nonzero()
        {
            target.state.mobile = true;
        }
        target.refresh_velocity_from_fixed();

        // Stuck check (C4Object.cpp:1801-1807): gate on the requested raw
        // fixed speed, not on the velocity reached above. ContactCheck
        // refreshes t_contact and runs directional Contact callbacks before
        // the target message and fail-safe Stuck callback.
        let no_horizontal_move = self
            .definitions
            .get(&self.objects[target_idx].definition_id)
            .map(Definition::no_horizontal_move)
            .unwrap_or(0);
        if self.frame.is_multiple_of(35) && txdir.is_nonzero() && no_horizontal_move == 0 {
            let target_id = self.objects[target_idx].id;
            let position = self.objects[target_idx].state.position;
            let contacted = self
                .object_contact_check_at(target_idx, position)?
                .is_some_and(|contact| contact.is_contact());
            if contacted {
                if let Some(target_idx) = self.find_object_index(target_id).filter(|&index| {
                    !self.objects[index].destroyed
                        && self.objects[index].state.status != ObjectStatus::Deleted
                }) {
                    let object = &self.objects[target_idx];
                    let name = object
                        .state
                        .custom_name
                        .clone()
                        .or_else(|| {
                            self.crew_object_infos
                                .get(&target_id)
                                .map(|info| info.name.clone())
                        })
                        .or_else(|| {
                            self.definitions
                                .get(&object.definition_id)
                                .map(|definition| definition.name().to_string())
                        })
                        .unwrap_or_else(|| object.definition_id.clone());
                    self.messages
                        .add_message(MessageSpec::target(format!("{name} is stuck!"), target_id));
                    let _ = tolerate_script_error(self.call_object_function(
                        target_idx,
                        "Stuck",
                        Vec::new(),
                    ))?;
                }
            }
        }
        Ok(true)
    }

    /// DFA_PUSH with a nonzero Walk physical (C4Object.cpp:5040-5097):
    /// target force `ValByPhysical(250, Push)` toward `±ValByPhysical(280,
    /// Walk)` per ComDir, got-hold area check with the GrabLost callback,
    /// follow xdir at the full walk limit, and controller transfer
    /// (C4Object.cpp:5082).
    fn apply_push_procedure_physical(
        &mut self,
        idx: usize,
        target_idx: usize,
        command_direction: CommandDirection,
        definition_id: &DefinitionId,
        physical: PhysicalInfo,
        phase_advance: &mut Option<i32>,
    ) -> Result<bool, EngineError> {
        let limit = math::val_by_physical(280, physical.walk);
        // ComDir → target speed and straightening (C4Object.cpp:5049-5057).
        let (txdir, straighten) = match command_direction {
            CommandDirection::Left | CommandDirection::DownLeft => (-limit, false),
            CommandDirection::UpLeft => (-limit, true),
            CommandDirection::Right | CommandDirection::DownRight => (limit, false),
            CommandDirection::UpRight => (limit, true),
            CommandDirection::Up => (C4Fixed::ZERO, true),
            CommandDirection::Stop | CommandDirection::Down => (C4Fixed::ZERO, false),
            _ => (C4Fixed::ZERO, false),
        };
        // Push object (C4Object.cpp:5059-5062).
        if !self.push_object(
            target_idx,
            txdir,
            math::val_by_physical(250, physical.push),
            straighten,
        )? {
            self.stop_action_delay_command(idx, definition_id)?;
            return Ok(false);
        }
        // C++ copies attribution immediately after Push succeeds, before a
        // later got-hold failure can stop the action (C4Object.cpp:5082).
        self.objects[target_idx].state.controller = self.objects[idx].state.controller;
        // Got-hold check (C4Object.cpp:5066-5080).
        let own_width = self.objects[idx]
            .current_shape_rect()
            .map(|rect| rect.width)
            .unwrap_or(0);
        let push_distance = (own_width / 2 - 8).max(0);
        let push_range = push_distance + 10;
        let target_position = self.objects[target_idx].state.position;
        let target_rect = self.objects[target_idx]
            .current_shape_rect()
            .unwrap_or_default();
        let mut sax = target_position.x + target_rect.x;
        let say = target_position.y + target_rect.y;
        let mut sawdt = target_rect.width;
        let sahgt = target_rect.height;
        let position = self.objects[idx].state.position;
        if !(-push_range..=sawdt - 1 + push_range).contains(&(position.x - sax))
            || !(-push_range..=sahgt - 1 + push_range).contains(&(position.y - say))
        {
            let pusher_id = self.objects[idx].id;
            self.stop_action_delay_command(idx, definition_id)?;
            self.grab_lost(pusher_id)?;
            return Ok(false);
        }
        // Vertical follow (C4Object.cpp:5083).
        if position.y - push_distance > say + sahgt && txdir != C4Fixed::ZERO {
            if txdir > C4Fixed::ZERO {
                sax += sawdt / 2;
            }
            sawdt /= 2;
        }
        // Horizontal follow with the full xdir reset (C4Object.cpp:5085-5087).
        let target_x = position
            .x
            .max(sax - push_distance)
            .min(sax + sawdt - 1 + push_distance);
        let physics = self.physics;
        let mut xdir = C4Fixed::ZERO;
        if position.x < target_x {
            xdir = limit;
        }
        if position.x > target_x {
            xdir = -limit;
        }
        self.objects[idx].fixed_velocity.x = xdir;
        // SetDir by raw xdir (C4Object.cpp:5103-5108), grounded (5110).
        let pusher_id = self.objects[idx].id;
        let (live_idx, advance) = self.set_exec_direction_from_xdir_live(pusher_id, 1)?;
        *phase_advance = Some(advance);
        let Some(idx) = live_idx else {
            return Ok(false);
        };
        let object = &mut self.objects[idx];
        object.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut object.fixed_velocity);
        object.refresh_velocity_from_fixed();
        // Attachment (C4Object.cpp:5110-5112): after SetDir, so the
        // TurnAction StartCall above still observed the pre-write register.
        object.frame_t_attach |= CNAT_BOTTOM;
        object.state.t_attach = object.frame_t_attach;
        Ok(true)
    }

    /// `StopActionDelayCommand` (C4Object.cpp:4677-4681): every failed
    /// DFA_PUSH first runs ObjectComStop (Idle, then stand in Walk) and adds
    /// a silent 50-frame Wait to the top of the command stack.
    pub(crate) fn stop_action_delay_command(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
    ) -> Result<(), EngineError> {
        self.object_com_stop_action(idx, definition_id)?;
        let wait = CommandRequest::new(CommandId::Wait)
            .with_update_interval(50)
            .with_mode(CommandMode::SilentSub);
        let _ = self.objects[idx].commands.push_front(wait);
        Ok(())
    }

    /// Stable-id form used when target-side callbacks may have changed the
    /// object list or the actor's definition before the stop runs.
    fn stop_action_delay_command_live(&mut self, object_id: ObjectId) -> Result<(), EngineError> {
        let _ = self.object_com_stop_live(object_id)?;
        let wait = CommandRequest::new(CommandId::Wait)
            .with_update_interval(50)
            .with_mode(CommandMode::SilentSub);
        if let Some(idx) = self.find_object_index(object_id) {
            let _ = self.objects[idx].commands.push_front(wait);
        }
        Ok(())
    }

    pub(crate) fn apply_push_procedure(
        &mut self,
        idx: usize,
        command_direction: CommandDirection,
        movement_profile: MovementProfile,
        definition_id: &DefinitionId,
        physical: PhysicalInfo,
        phase_advance: &mut Option<i32>,
    ) -> Result<bool, EngineError> {
        let pusher_id = self.objects[idx].id;
        let Some(target_id) = self.objects[idx].state.action.target else {
            self.stop_action_delay_command(idx, definition_id)?;
            return Ok(false);
        };
        let Some(target_idx) = self.find_object_index(target_id) else {
            self.stop_action_delay_command(idx, definition_id)?;
            return Ok(false);
        };
        if target_idx == idx {
            self.stop_action_delay_command(idx, definition_id)?;
            return Ok(false);
        }
        let target_removed = {
            let target = &self.objects[target_idx];
            target.destroyed || matches!(target.state.status, ObjectStatus::Deleted)
        };
        if target_removed {
            self.stop_action_delay_command(idx, definition_id)?;
            return Ok(false);
        }
        if self.objects[idx].state.container == Some(target_id)
            || self.objects[target_idx].state.container.is_some()
        {
            self.stop_action_delay_command(idx, definition_id)?;
            return Ok(false);
        }

        if physical.walk != 0 {
            return self.apply_push_procedure_physical(
                idx,
                target_idx,
                command_direction,
                definition_id,
                physical,
                phase_advance,
            );
        }

        let push_speed = movement_profile.walk_speed.max(0);
        let push_accel = movement_profile.walk_acceleration.max(0);
        let straighten = matches!(
            command_direction,
            CommandDirection::Up | CommandDirection::UpLeft | CommandDirection::UpRight
        );
        let desired_target_velocity = match command_direction {
            CommandDirection::Left | CommandDirection::DownLeft | CommandDirection::UpLeft => {
                -push_speed
            }
            CommandDirection::Right | CommandDirection::DownRight | CommandDirection::UpRight => {
                push_speed
            }
            _ => 0,
        };

        let physics = self.physics;

        if idx < target_idx {
            let (first, rest) = self.objects.split_at_mut(target_idx);
            let pusher = &mut first[idx];
            let target = &mut rest[0];
            Self::update_push_pair(
                pusher,
                target,
                desired_target_velocity,
                push_speed,
                push_accel,
                straighten,
                physics,
            );
        } else {
            let (first, rest) = self.objects.split_at_mut(idx);
            let target = &mut first[target_idx];
            let pusher = &mut rest[0];
            Self::update_push_pair(
                pusher,
                target,
                desired_target_velocity,
                push_speed,
                push_accel,
                straighten,
                physics,
            );
        }

        let (live_idx, advance) = self.set_exec_direction_from_xdir_live(pusher_id, 1)?;
        *phase_advance = Some(advance);
        let Some(idx) = live_idx else {
            return Ok(false);
        };
        // Attachment (C4Object.cpp:5110-5112).
        let object = &mut self.objects[idx];
        object.frame_t_attach |= CNAT_BOTTOM;
        object.state.t_attach = object.frame_t_attach;

        Ok(true)
    }

    fn update_pull_pair(
        puller: &mut Object,
        target: &mut Object,
        desired_puller_velocity: i32,
        desired_target_velocity: i32,
        speed_limit: i32,
        acceleration: i32,
        physics: PhysicsSettings,
    ) {
        let accel = itofix(acceleration.max(0));
        // DFA_PULL moves the target through C4Object::Push, so the same
        // pre-mobilization zero + mobilization check apply
        // (C4Object.cpp:1765-1768,1797).
        if !target.state.mobile {
            target.fixed_velocity = FixedVec2::ZERO;
            target.fixed_position = FixedVec2::new(
                itofix(target.state.position.x),
                itofix(target.state.position.y),
            );
        }
        let new_target_velocity = step_fixed_toward(
            target.fixed_velocity.x,
            itofix(desired_target_velocity),
            accel,
        );
        target.fixed_velocity.x = clamp_fixed_to_limit(new_target_velocity, speed_limit);
        physics.clamp_fixed_velocity(&mut target.fixed_velocity);
        if target.fixed_velocity.x.is_nonzero()
            || target.fixed_velocity.y.is_nonzero()
            || target.rotation_velocity.is_nonzero()
        {
            target.state.mobile = true;
        }
        target.refresh_velocity_from_fixed();
        target.state.controller = puller.state.controller;

        let new_puller_velocity = step_fixed_toward(
            puller.fixed_velocity.x,
            itofix(desired_puller_velocity),
            accel,
        );
        puller.fixed_velocity.x = clamp_fixed_to_limit(new_puller_velocity, speed_limit);
        puller.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut puller.fixed_velocity);
        puller.refresh_velocity_from_fixed();
    }

    fn update_push_pair(
        pusher: &mut Object,
        target: &mut Object,
        desired_target_velocity: i32,
        push_speed: i32,
        push_accel: i32,
        straighten: bool,
        physics: PhysicsSettings,
    ) {
        let push_accel = push_accel.max(0);
        let push_accel_fixed = itofix(push_accel);
        // C4Object::Push pre-mobilization zero (C4Object.cpp:1765-1768).
        if !target.state.mobile {
            target.fixed_velocity = FixedVec2::ZERO;
            target.fixed_position = FixedVec2::new(
                itofix(target.state.position.x),
                itofix(target.state.position.y),
            );
        }
        let new_target_velocity = step_fixed_toward(
            target.fixed_velocity.x,
            itofix(desired_target_velocity),
            push_accel_fixed,
        );
        target.fixed_velocity.x = clamp_fixed_to_limit(new_target_velocity, push_speed);
        if straighten && push_accel > 0 {
            target.fixed_velocity.y =
                decelerate_fixed_toward_zero(target.fixed_velocity.y, push_accel_fixed);
        }
        physics.clamp_fixed_velocity(&mut target.fixed_velocity);
        // Mobilization check (C4Object.cpp:1797).
        if target.fixed_velocity.x.is_nonzero()
            || target.fixed_velocity.y.is_nonzero()
            || target.rotation_velocity.is_nonzero()
        {
            target.state.mobile = true;
        }
        target.refresh_velocity_from_fixed();
        target.state.controller = pusher.state.controller;

        let mut desired_pusher_velocity = desired_target_velocity;
        if desired_pusher_velocity == 0 {
            let delta = target.state.position.x - pusher.state.position.x;
            let threshold = push_speed.max(1);
            if delta > threshold {
                desired_pusher_velocity = push_speed;
            } else if delta < -threshold {
                desired_pusher_velocity = -push_speed;
            }
        }

        let new_pusher_velocity = step_fixed_toward(
            pusher.fixed_velocity.x,
            itofix(desired_pusher_velocity),
            push_accel_fixed,
        );
        pusher.fixed_velocity.x = clamp_fixed_to_limit(new_pusher_velocity, push_speed);
        pusher.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut pusher.fixed_velocity);
        pusher.refresh_velocity_from_fixed();
    }

    fn desired_pull_velocity(
        current_position: i32,
        desired_position: i32,
        base_velocity: i32,
        walk_speed: i32,
    ) -> i32 {
        let delta = desired_position.saturating_sub(current_position);
        let correction = delta.clamp(-10, 10) / 10;
        base_velocity + walk_speed.saturating_mul(correction)
    }

    fn object_half_extents(object: &Object) -> (i32, i32) {
        if object.state.vertices.is_empty() {
            // Without explicit vertex data fall back to a generous default so pull spacing stays stable.
            return (10, 10);
        }

        let mut min_x = object.state.vertices[0].x;
        let mut max_x = min_x;
        let mut min_y = object.state.vertices[0].y;
        let mut max_y = min_y;
        for vertex in &object.state.vertices {
            if vertex.x < min_x {
                min_x = vertex.x;
            }
            if vertex.x > max_x {
                max_x = vertex.x;
            }
            if vertex.y < min_y {
                min_y = vertex.y;
            }
            if vertex.y > max_y {
                max_y = vertex.y;
            }
        }

        let width = max_x.saturating_sub(min_x);
        let height = max_y.saturating_sub(min_y);
        let half_width = if width <= 0 { 10 } else { (width + 1) / 2 };
        let half_height = if height <= 0 { 10 } else { (height + 1) / 2 };
        (half_width, half_height)
    }

    fn horizontal_input_sign(command_direction: CommandDirection) -> i32 {
        match command_direction {
            CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => -1,
            CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => 1,
            _ => 0,
        }
    }

    /// `C4GameObjects::CrossCheck` reverse area check
    /// (C4GameObjects.cpp:140-197), run once per frame after object    /// execution: every frame an OCF_Alive victim takes OCF_HitSpeed2 hits
    /// from C4D_Object projectiles inside its shape; on Tick3 frames
    /// collection runs (OCF_Collection vs OCF_Carryable, Collection rect).
    /// Candidates are deduplicated per victim like the C++ Marker. Pass 1
    /// (Tick5 fight / Tick35 contact incineration) and pass 3 (contained
    /// fight) still need the hostility and fire models.
    #[doc(hidden)]
    pub fn cross_check(&mut self, frame: u64) -> Result<(), EngineError> {
        self.cross_check_at_object_pass(frame)?;
        self.cross_check_reverse_area_pass(frame)?;
        self.cross_check_contained_pass(frame)
    }

    /// CrossCheck pass 3: Contained check (C4GameObjects.cpp:199-230). On
    /// Tick10 frames, contained FightReady objects fight hostile FightReady
    /// company sharing their container — directly, with no RejectFight veto.
    fn cross_check_contained_pass(&mut self, frame: u64) -> Result<(), EngineError> {
        if !frame.is_multiple_of(10) {
            return Ok(());
        }
        let focf = crate::ocf::FIGHT_READY;
        let tocf = crate::ocf::FIGHT_READY;
        let object_ids = self
            .execution
            .exec_list
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        'outer: for obj1_id in object_ids {
            let Some(idx) = self.find_object_index(obj1_id) else {
                continue;
            };
            let container = {
                let obj1 = &self.objects[idx];
                if obj1.destroyed || !obj1.state.status.is_active() {
                    continue;
                }
                match obj1.state.container {
                    Some(container) => container,
                    None => continue,
                }
            };
            if self.object_ocf_at_index(idx) & focf == 0 {
                continue;
            }
            let obj1_layer = self.objects[idx].state.layer;
            let contents = match self
                .find_object_index(container)
                .map(|container_idx| self.objects[container_idx].state.contents.clone())
            {
                Some(contents) => contents,
                None => continue,
            };
            for obj2_id in contents {
                if obj2_id == obj1_id {
                    continue;
                }
                // Object indices are stable mid-tick (the Vec only appends
                // until the end-of-tick retain), so obj1'"'"'s index from the
                // outer loop stays valid; C++ re-checks the FLAGS after
                // callbacks, not the identity (C4GameObjects.cpp:186-192).
                let Some(obj2_idx) = self.find_object_index(obj2_id) else {
                    continue;
                };
                {
                    let obj2 = &self.objects[obj2_idx];
                    if !obj2.has_nonzero_status()
                        || obj2.state.container.is_none()
                        || obj2.state.layer != obj1_layer
                    {
                        continue;
                    }
                }
                if self.object_ocf_at_index(obj2_idx) & tocf == 0 {
                    continue;
                }
                let ocf1 = self.object_ocf_at_index(idx);
                // Fight (C4GameObjects.cpp:218-227)
                if ocf1 & crate::ocf::FIGHT_READY != 0 {
                    let owner1 = self.objects[idx].state.owner;
                    let owner2 = self.objects[obj2_idx].state.owner;
                    if self.players_hostile(owner1, owner2) {
                        self.object_action_fight(obj1_id, obj2_id);
                        self.object_action_fight(obj2_id, obj1_id);
                        // obj1 might have been tampered with
                        let Some(idx) = self.find_object_index(obj1_id) else {
                            continue 'outer;
                        };
                        let obj1 = &self.objects[idx];
                        if obj1.destroyed
                            || !obj1.state.status.is_active()
                            || obj1.state.container.is_some()
                            || self.object_ocf_at_index(idx) & focf == 0
                        {
                            continue 'outer;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// CrossCheck pass 1: AtObject check (C4GameObjects.cpp:97-138). On Tick5
    /// frames FightReady objects standing at a hostile FightReady object    /// start fighting both ways after the RejectFight callbacks. The Tick35
    /// contact-incineration arm (OCF_OnFire vs OCF_Inflammable with the
    /// `!Random(ContactIncinerate)` draw) still needs the fire model — no
    /// Rust object ever carries OCF_OnFire yet, so the C++ stream consumes no
    /// draws for it either.
    fn cross_check_at_object_pass(&mut self, frame: u64) -> Result<(), EngineError> {
        let tick5 = frame.is_multiple_of(5);
        let tick35 = frame.is_multiple_of(35);
        let mut focf = crate::ocf::NONE;
        let mut tocf = crate::ocf::NONE;
        if tick5 {
            focf |= crate::ocf::FIGHT_READY;
            tocf |= crate::ocf::FIGHT_READY;
        }
        // Very low level: Incineration (C4GameObjects.cpp:106-110)
        if tick35 {
            focf |= crate::ocf::ON_FIRE;
            tocf |= crate::ocf::INFLAMMABLE;
        }
        if focf == 0 || tocf == 0 {
            return Ok(());
        }
        let object_ids = self
            .execution
            .exec_list
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        for obj1_id in object_ids {
            let Some(idx) = self.find_object_index(obj1_id) else {
                continue;
            };
            {
                let obj1 = &self.objects[idx];
                if obj1.destroyed
                    || !obj1.state.status.is_active()
                    || obj1.state.container.is_some()
                {
                    continue;
                }
            }
            let ocf1 = self.object_ocf_at_index(idx);
            if ocf1 & focf == 0 {
                continue;
            }
            let position = self.objects[idx].state.position;
            let Some((obj2_idx, obj2_id, ocf2)) = self.at_object(position, tocf, Some(obj1_id))
            else {
                continue;
            };
            // Incineration (C4GameObjects.cpp:120-125): the Random draw runs
            // whenever the OCF pair matches, regardless of its outcome.
            if ocf1 & crate::ocf::ON_FIRE != 0 && ocf2 & crate::ocf::INFLAMMABLE != 0 {
                let contact_incinerate = self
                    .definitions
                    .get(&self.objects[obj2_idx].definition_id)
                    .map(|definition| definition.contact_incinerate())
                    .unwrap_or(0);
                if self.rng.random(contact_incinerate) == 0 {
                    // GetFireCausePlr: the fire effect's CausedBy, NO_OWNER
                    // unless it is a valid player.
                    let cause = self.objects[idx].state.fire_caused_by;
                    let cause = if self.players.contains_key(&cause) {
                        cause
                    } else {
                        OWNER_NONE
                    };
                    let _ = self.incinerate_object(obj2_idx, cause, false, Some(obj1_id))?;
                    continue;
                }
            }
            // Fight (C4GameObjects.cpp:126-136)
            if ocf1 & crate::ocf::FIGHT_READY != 0 && ocf2 & crate::ocf::FIGHT_READY != 0 {
                let owner1 = self.objects[idx].state.owner;
                let owner2 = self.objects[obj2_idx].state.owner;
                if !self.players_hostile(owner1, owner2) {
                    continue;
                }
                // RejectFight callbacks (C4GameObjects.cpp:131-132)
                let reject1 = self.call_object_function(
                    idx,
                    "RejectFight",
                    vec![object_reference_value(obj2_id)],
                )?;
                if reject1.as_bool() {
                    continue;
                }
                let Some(obj2_idx) = self.find_object_index(obj2_id) else {
                    continue;
                };
                let reject2 = self.call_object_function(
                    obj2_idx,
                    "RejectFight",
                    vec![object_reference_value(obj1_id)],
                )?;
                if reject2.as_bool() {
                    continue;
                }
                self.object_action_fight(obj1_id, obj2_id);
                self.object_action_fight(obj2_id, obj1_id);
            }
        }
        Ok(())
    }

    /// `ObjectActionFight` (C4ObjectCom.cpp:157-160):
    /// `SetActionByName("Fight", target)`.
    pub(crate) fn object_action_fight(&mut self, object_id: ObjectId, target_id: ObjectId) {
        let Some(idx) = self.find_object_index(object_id) else {
            return;
        };
        let definition_id = self.objects[idx].definition_id.clone();
        let Some((library, incomplete_activity)) =
            self.definitions.get(&definition_id).map(|definition| {
                (
                    definition.shared_action_library_handle(),
                    definition.incomplete_activity(),
                )
            })
        else {
            return;
        };
        if !library.contains("Fight") {
            return;
        }
        let previous = self.objects[idx].state.action.clone();
        let previous_index = previous
            .act_map_index
            .or_else(|| library.named_action_index(&previous.name));
        let requested_index = library.named_action_index("Fight");
        let requested_action_changed =
            previous.name != "Fight" || previous_index != requested_index;
        let active_action_allowed =
            self.objects[idx].state.construction >= FULL_CON || incomplete_activity;
        let update = ActionUpdate {
            name: Some("Fight".to_string()),
            phase: Some(0),
            ticks: Some(0),
            force: false,
            data: None,
            target: Some(Some(target_id)),
            target2: Some(None),
            callbacks_dispatched: false,
            action_sound_dispatched: false,
            action_sound_selection: None,
        };
        let object = &mut self.objects[idx];
        let result = object.state.action.apply_update_with_library_and_activity(
            &update,
            &library,
            active_action_allowed,
        );
        // SetAction fix resync (C4Object.cpp:4144) — only past the
        // NoOtherAction early returns.
        if update.name.is_some() && matches!(result, ActionUpdateResult::Applied) {
            object.fixed_position =
                FixedVec2::from_ints(object.state.position.x, object.state.position.y);
        }
        let changed = matches!(result, ActionUpdateResult::Applied)
            && (previous.name != object.state.action.name
                || previous.act_map_index != object.state.action.act_map_index);
        if changed {
            let previous_flip_dir =
                library.flip_dir_for_entry(&previous.name, previous.act_map_index);
            object.record_action_event_with_sound_stop(
                previous,
                ActionTransitionKind::Forced,
                &library,
                requested_action_changed,
            );
            // SetAction's FlipDir refresh, guarded on the value changing
            // (C4Object.cpp:4183-4184).
            if previous_flip_dir != self.object_action_flip_dir(idx) {
                self.update_object_flip_dir(idx);
            }
        }
        if changed {
            self.dispatch_pending_action_sounds(idx, false);
        }
    }

    /// `C4PlayerList::Hostile` (C4PlayerList.cpp:82-92): false for missing or
    /// identical players; one-way declarations count both ways.
    pub(crate) fn players_hostile(&self, player1: i32, player2: i32) -> bool {
        let (Some(plr1), Some(plr2)) = (self.players.get(&player1), self.players.get(&player2))
        else {
            return false;
        };
        if plr1.id() == plr2.id() {
            return false;
        }
        plr1.is_hostile_towards(plr2.id()) || plr2.is_hostile_towards(plr1.id())
    }

    /// CrossCheck pass 2: reverse area check (C4GameObjects.cpp:140-197).
    fn cross_check_reverse_area_pass(&mut self, frame: u64) -> Result<(), EngineError> {
        let tick3 = frame.is_multiple_of(3);
        let mut focf = crate::ocf::ALIVE;
        let mut tocf = crate::ocf::HIT_SPEED2;
        if tick3 {
            focf |= crate::ocf::COLLECTION;
            tocf |= crate::ocf::CARRYABLE;
        }
        let object_ids = self
            .execution
            .exec_list
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        'outer: for obj1_id in object_ids {
            let Some(idx) = self.find_object_index(obj1_id) else {
                continue;
            };
            {
                let obj1 = &self.objects[idx];
                if obj1.destroyed
                    || !obj1.state.status.is_active()
                    || obj1.state.container.is_some()
                {
                    continue;
                }
            }
            if self.object_ocf_at_index(idx) & focf == 0 {
                continue;
            }
            let obj1_layer = self.objects[idx].state.layer;
            // obj1->Area: candidates from the sector lists under the shape
            let collector_shape_rect = self.object_shape_rect(&self.objects[idx]);
            let candidate_ids = self
                .sectors
                .as_ref()
                .map(|sectors| {
                    let area = sectors.area(collector_shape_rect);
                    sectors.object_ids_in_area(&area)
                })
                .unwrap_or_else(|| self.objects.iter().map(|object| object.id).collect());
            // handle collision only once (Marker, C4GameObjects.cpp:163-165)
            let mut marker: HashSet<ObjectId> = HashSet::new();
            for candidate_id in candidate_ids {
                // Object indices are stable mid-tick (the Vec only appends
                // until the end-of-tick retain), so obj1'"'"'s index from the
                // outer loop stays valid; C++ re-checks the FLAGS after
                // callbacks, not the identity (C4GameObjects.cpp:186-192).
                let Some(candidate_idx) = self.find_object_index(candidate_id) else {
                    continue;
                };
                if candidate_idx == idx {
                    continue;
                }
                {
                    let candidate = &self.objects[candidate_idx];
                    if candidate.destroyed
                        || !candidate.state.status.is_active()
                        || candidate.state.container.is_some()
                        || candidate.state.layer != obj1_layer
                    {
                        continue;
                    }
                }
                let ocf2 = self.object_ocf_at_index(candidate_idx);
                if ocf2 & tocf == 0 {
                    continue;
                }
                let obj1_position = self.objects[idx].state.position;
                let candidate_position = self.objects[candidate_idx].state.position;
                let dx = candidate_position.x - obj1_position.x;
                let dy = candidate_position.y - obj1_position.y;
                // Inside(obj2->x - (obj1->x + Shape.x), 0, Shape.Wdt - 1).
                // This is the raw LIVE Shape, without Area's addtop expansion;
                // re-read it for each candidate because an earlier callback may
                // change obj1's shape (C4GameObjects.cpp:159-160).
                let shape_rect = self.objects[idx].current_shape_rect();
                if let Some(shape) = shape_rect {
                    if !shape.contains_offset(dx, dy) {
                        continue;
                    }
                } else {
                    let (half_w, half_h) = Self::object_half_extents(&self.objects[idx]);
                    if dx < -half_w || dx >= half_w || dy < -half_h || dy >= half_h {
                        continue;
                    }
                }
                if !marker.insert(candidate_id) {
                    continue;
                }
                let ocf1 = self.object_ocf_at_index(idx);
                // Hit (C4GameObjects.cpp:167-184)
                if ocf2 & crate::ocf::HIT_SPEED2 != 0
                    && ocf1 & crate::ocf::ALIVE != 0
                    && self.objects[candidate_idx].state.category & CATEGORY_OBJECT != 0
                {
                    let by_value = object_reference_value(candidate_id);
                    let query =
                        self.call_object_function(idx, "QueryCatchBlow", vec![by_value.clone()])?;
                    if !query.as_bool() {
                        // "realistic" hit energy (C4GameObjects.cpp:171-173)
                        let v1 = self.objects[idx].fixed_velocity;
                        let v2 = self.objects[candidate_idx].fixed_velocity;
                        let dx_dir = v2.x - v1.x;
                        let dy_dir = v2.y - v1.y;
                        // "realistic" hit energy uses the LIVE Mass
                        // (C4GameObjects.cpp:171; UpdateMass includes
                        // contents).
                        let candidate_mass = self.effective_object_mass(candidate_idx);
                        let hit_energy =
                            fixtoi((dx_dir * dx_dir + dy_dir * dy_dir) * candidate_mass / 5);
                        // reduced to 1/3rd, but never dropped to zero by it
                        let hit_energy = (hit_energy / 3).max(i32::from(hit_energy != 0));
                        self.change_object_energy(
                            idx,
                            -(hit_energy / 5),
                            C4FX_CALL_ENG_OBJ_HIT,
                            self.objects[candidate_idx].state.controller,
                        )?;
                        // tmass = max(obj1->Mass, 50) with the LIVE mass
                        // (C4GameObjects.cpp:174).
                        let tmass = self.effective_object_mass(idx).max(50);
                        let candidate_velocity = self.objects[candidate_idx].fixed_velocity;
                        // fling unless airborne off-Tick3 (C4GameObjects.cpp:176)
                        let definition_id = self.objects[idx].definition_id.clone();
                        let procedure = self
                            .definitions
                            .get(&definition_id)
                            .map(|definition| {
                                definition.action_library().procedure_for_entry(
                                    &self.objects[idx].state.action.name,
                                    self.objects[idx].state.action.act_map_index,
                                )
                            })
                            .unwrap_or_default();
                        let has_action = !self.objects[idx].state.action.name.is_empty();
                        if tick3 || (has_action && procedure != ActionProcedure::Flight) {
                            let txdir = C4Fixed::from_raw(
                                candidate_velocity.x.val().wrapping_mul(50) / tmass,
                            );
                            let tydir = C4Fixed::from_raw(
                                -(candidate_velocity.y.val() / 2).abs().wrapping_mul(50) / tmass,
                            );
                            let caused_by = self.objects[candidate_idx].state.controller;
                            self.fling_object(idx, txdir, tydir, caused_by);
                        }
                        let _ = self.call_object_function(
                            idx,
                            "CatchBlow",
                            vec![Value::Int(-(hit_energy / 5)), by_value],
                        )?;
                        // obj1 might have been tampered with
                        let Some(idx) = self.find_object_index(obj1_id) else {
                            continue 'outer;
                        };
                        let obj1 = &self.objects[idx];
                        if obj1.destroyed
                            || !obj1.state.status.is_active()
                            || obj1.state.container.is_some()
                            || self.object_ocf_at_index(idx) & focf == 0
                        {
                            continue 'outer;
                        }
                        continue;
                    }
                }
                // Collection (C4GameObjects.cpp:185-194)
                // QueryCatchBlow may have changed either object's OCF,
                // position, or the collector's definition. C++ reads every
                // one of these live after that callback
                // (C4GameObjects.cpp:167-190).
                let live_ocf1 = self.object_ocf_at_index(idx);
                let live_ocf2 = self.object_ocf_at_index(candidate_idx);
                let live_obj1_position = self.objects[idx].state.position;
                let live_candidate_position = self.objects[candidate_idx].state.position;
                let live_dx = live_candidate_position.x - live_obj1_position.x;
                let live_dy = live_candidate_position.y - live_obj1_position.y;
                let collection_rect = self
                    .definitions
                    .get(&self.objects[idx].definition_id)
                    .and_then(|definition| definition.collection_rect());
                if live_ocf1 & crate::ocf::COLLECTION != 0 && live_ocf2 & crate::ocf::CARRYABLE != 0
                {
                    let Some(collection_rect) = collection_rect.filter(|rect| rect.is_positive())
                    else {
                        continue;
                    };
                    if !collection_rect.contains_offset(live_dx, live_dy) {
                        continue;
                    }
                    // C4Object::Collect rejects FLAG/FlyBase before Enter
                    // when cached C4RULE_FlagRemoveable is off. Keep this
                    // ahead of RejectEntrance and every mutation/callback
                    // (C4Object.cpp:5693-5700).
                    if compat::flag_collection_blocked(
                        self.objects[candidate_idx].definition_id.as_str(),
                        self.objects[candidate_idx].state.action.name.as_str(),
                        self.flag_removeable,
                    ) {
                        continue;
                    }
                    if coach_debug_id() == Some(candidate_id.as_u64())
                        || coach_debug_id() == Some(obj1_id.as_u64())
                    {
                        crate::rng::rng_trace_line(
                            self.rng.trace_index,
                            &format!(
                                "XCOLLECT collector={} ({:?}) at {:?} takes {} at {:?}",
                                obj1_id.as_u64(),
                                self.objects[idx].definition_id,
                                live_obj1_position,
                                candidate_id.as_u64(),
                                live_candidate_position
                            ),
                        );
                    }
                    // C4Object::Collect runs the full Enter-and-tail path:
                    // both vetoes, fCopyMotion=false callbacks, attach
                    // cancellation, Collection/Hit and the final conditional
                    // CopyMotion (C4Object.cpp:5693-5715).
                    let _ = self.try_object_collect(candidate_id, obj1_id)?;
                    // obj1 might have been tampered with
                    let Some(idx) = self.find_object_index(obj1_id) else {
                        continue 'outer;
                    };
                    let obj1 = &self.objects[idx];
                    if obj1.destroyed
                        || !obj1.state.status.is_active()
                        || obj1.state.container.is_some()
                        || self.object_ocf_at_index(idx) & focf == 0
                    {
                        continue 'outer;
                    }
                }
            }
        }
        Ok(())
    }
}
