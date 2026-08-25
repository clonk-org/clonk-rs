//! `impl Engine` — live movement stepping and per-index physics.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MovementProbeTrace {
    pub(crate) position: Vector2,
    pub(crate) object_position: Vector2,
    pub(crate) fixed_position: FixedVec2,
    pub(crate) rotation: i32,
    pub(crate) fixed_rotation: C4Fixed,
    pub(crate) fixed_velocity: FixedVec2,
    pub(crate) rotation_velocity: C4Fixed,
    pub(crate) motion_x: i32,
    pub(crate) motion_y: i32,
    pub(crate) result: bool,
    pub(crate) t_contact: u32,
    pub(crate) contact_count: i32,
    pub(crate) contact_cnat: u32,
    pub(crate) vertex_contacts: Vec<u32>,
    pub(crate) random_count: i32,
    pub(crate) random_hold: u32,
}

#[cfg(test)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MovementParityTrace {
    pub(crate) probes: Vec<MovementProbeTrace>,
    pub(crate) contact_invocations: usize,
    pub(crate) update_pos_calls: usize,
    pub(crate) pre_contact_action_t_contact: Option<u32>,
}

#[cfg(test)]
thread_local! {
    static MOVEMENT_PARITY_TRACE: std::cell::RefCell<Option<MovementParityTrace>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn begin_movement_parity_trace() {
    MOVEMENT_PARITY_TRACE.with(|trace| {
        debug_assert!(trace.borrow().is_none());
        *trace.borrow_mut() = Some(MovementParityTrace::default());
    });
}

#[cfg(test)]
fn take_movement_parity_trace() -> MovementParityTrace {
    MOVEMENT_PARITY_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("movement parity trace was enabled")
    })
}

#[cfg(test)]
pub(crate) fn record_movement_contact_invocation() {
    MOVEMENT_PARITY_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.contact_invocations += 1;
        }
    });
}

#[cfg(test)]
fn record_movement_update_pos() {
    MOVEMENT_PARITY_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.update_pos_calls += 1;
        }
    });
}

#[cfg(test)]
fn record_pre_contact_action_t_contact(t_contact: u32) {
    MOVEMENT_PARITY_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.pre_contact_action_t_contact = Some(t_contact);
        }
    });
}

impl Engine {
    fn movement_live_config_at(&self, object_id: ObjectId) -> Option<MovementLiveConfig> {
        let index = self.find_object_index(object_id)?;
        Some(movement_live_config_for(
            &self.objects[index],
            &self.definitions,
            self.layer_movement_bounds_for(index),
        ))
    }

    fn dispatch_live_movement_contact(
        &mut self,
        object_id: ObjectId,
        dispatch: MovementContactDispatch,
    ) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        self.dispatch_contact_callbacks(index, dispatch)
    }

    fn refresh_live_movement_solid_masks(&self, solid_masks: &mut Vec<SolidMaskRect>) {
        *solid_masks = self.live_movement_solid_masks();
    }

    fn apply_live_movement_side_bounds(
        &mut self,
        object_id: ObjectId,
        target_x: &mut i32,
        solid_masks: &mut Vec<SolidMaskRect>,
    ) -> Result<(), EngineError> {
        let layer_contacts = self
            .movement_live_config_at(object_id)
            .and_then(|live| {
                let layer = live.layer_bounds?;
                if layer.border_bound & C4D_BORDER_LAYER == 0
                    || (!live.action_is_idle
                        && matches!(live.action_procedure, ActionProcedure::Attach))
                {
                    return None;
                }
                let index = self.find_object_index(object_id)?;
                let object = &self.objects[index];
                let shape_x = object
                    .current_shape_rect()
                    .map(|shape| shape.x)
                    .unwrap_or(0);
                let (low, high) = if object.state.category & CATEGORY_STATIC_BACK != 0 {
                    (
                        layer.position.x + layer.shape_rect.x,
                        layer.position.x + layer.shape_rect.x + layer.shape_rect.width,
                    )
                } else {
                    (
                        layer.position.x + layer.shape_rect.x - shape_x,
                        layer.position.x + layer.shape_rect.x + layer.shape_rect.width + shape_x,
                    )
                };
                Some(target_bounds(target_x, low, high, CNAT_LEFT, CNAT_RIGHT))
            })
            .unwrap_or([None, None]);
        for cnat in layer_contacts.into_iter().flatten() {
            let Some(index) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let object = &mut self.objects[index];
            object.fixed_velocity.x = C4Fixed::ZERO;
            object.refresh_velocity_from_fixed();
            self.dispatch_live_movement_contact(object_id, MovementContactDispatch::Direct(cnat))?;
            self.refresh_live_movement_solid_masks(solid_masks);
        }

        if self
            .movement_live_config_at(object_id)
            .is_none_or(|live| live.border_bound & C4D_BORDER_SIDES == 0)
        {
            return Ok(());
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        let shape_x = self.objects[index]
            .current_shape_rect()
            .map(|shape| shape.x)
            .unwrap_or(0);
        let width = self
            .landscape
            .as_ref()
            .map(|landscape| i32::try_from(landscape.width()).unwrap_or(i32::MAX))
            .unwrap_or(0);
        for cnat in target_bounds(
            target_x,
            -shape_x,
            width.saturating_add(shape_x),
            CNAT_LEFT,
            CNAT_RIGHT,
        )
        .into_iter()
        .flatten()
        {
            let Some(index) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let object = &mut self.objects[index];
            object.fixed_velocity.x = C4Fixed::ZERO;
            object.refresh_velocity_from_fixed();
            self.dispatch_live_movement_contact(object_id, MovementContactDispatch::Direct(cnat))?;
            self.refresh_live_movement_solid_masks(solid_masks);
        }
        Ok(())
    }

    pub(crate) fn apply_live_movement_vertical_bounds(
        &mut self,
        object_id: ObjectId,
        target_y: &mut i32,
        solid_masks: &mut Vec<SolidMaskRect>,
    ) -> Result<(), EngineError> {
        let layer_contacts = self
            .movement_live_config_at(object_id)
            .and_then(|live| {
                let layer = live.layer_bounds?;
                if layer.border_bound & C4D_BORDER_LAYER == 0
                    || (!live.action_is_idle
                        && matches!(live.action_procedure, ActionProcedure::Attach))
                {
                    return None;
                }
                let index = self.find_object_index(object_id)?;
                let object = &self.objects[index];
                let shape_y = object
                    .current_shape_rect()
                    .map(|shape| shape.y)
                    .unwrap_or(0);
                let (low, high) = if object.state.category & CATEGORY_STATIC_BACK != 0 {
                    (
                        layer.position.y + layer.shape_rect.y,
                        layer.position.y + layer.shape_rect.y + layer.shape_rect.height,
                    )
                } else {
                    (
                        layer.position.y + layer.shape_rect.y - shape_y,
                        layer.position.y + layer.shape_rect.y + layer.shape_rect.height + shape_y,
                    )
                };
                Some(target_bounds(target_y, low, high, CNAT_TOP, CNAT_BOTTOM))
            })
            .unwrap_or([None, None]);
        for cnat in layer_contacts.into_iter().flatten() {
            let Some(index) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let object = &mut self.objects[index];
            object.fixed_velocity.y = C4Fixed::ZERO;
            object.refresh_velocity_from_fixed();
            self.dispatch_live_movement_contact(object_id, MovementContactDispatch::Direct(cnat))?;
            self.refresh_live_movement_solid_masks(solid_masks);
        }

        if self
            .movement_live_config_at(object_id)
            .is_some_and(|live| live.border_bound & C4D_BORDER_TOP != 0)
        {
            let Some(index) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let shape_y = self.objects[index]
                .current_shape_rect()
                .map(|shape| shape.y)
                .unwrap_or(0);
            for cnat in target_bounds(target_y, -shape_y, 1_000_000, CNAT_TOP, CNAT_BOTTOM)
                .into_iter()
                .flatten()
            {
                let Some(index) = self.find_object_index(object_id) else {
                    return Ok(());
                };
                let object = &mut self.objects[index];
                object.fixed_velocity.y = C4Fixed::ZERO;
                object.refresh_velocity_from_fixed();
                self.dispatch_live_movement_contact(
                    object_id,
                    MovementContactDispatch::Direct(cnat),
                )?;
                self.refresh_live_movement_solid_masks(solid_masks);
            }
        }
        if self
            .movement_live_config_at(object_id)
            .is_some_and(|live| live.border_bound & C4D_BORDER_BOTTOM != 0)
        {
            let Some(index) = self.find_object_index(object_id) else {
                return Ok(());
            };
            let shape_y = self.objects[index]
                .current_shape_rect()
                .map(|shape| shape.y)
                .unwrap_or(0);
            let bottom = self
                .landscape
                .as_ref()
                .map(Landscape::estimated_height)
                .unwrap_or(0)
                .saturating_add(shape_y);
            for cnat in target_bounds(target_y, -1_000_000, bottom, CNAT_TOP, CNAT_BOTTOM)
                .into_iter()
                .flatten()
            {
                let Some(index) = self.find_object_index(object_id) else {
                    return Ok(());
                };
                let object = &mut self.objects[index];
                object.fixed_velocity.y = C4Fixed::ZERO;
                object.refresh_velocity_from_fixed();
                self.dispatch_live_movement_contact(
                    object_id,
                    MovementContactDispatch::Direct(cnat),
                )?;
                self.refresh_live_movement_solid_masks(solid_masks);
            }
        }
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].refresh_velocity_from_fixed();
        }
        Ok(())
    }

    fn probe_live_movement_contact(
        &mut self,
        object_id: ObjectId,
        candidate: Vector2,
        solid_masks: &mut Vec<SolidMaskRect>,
        solid_mask_removed: bool,
    ) -> Result<bool, EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let contact = {
            let object = &self.objects[index];
            let Some(landscape) = self.landscape.as_ref() else {
                return Ok(false);
            };
            shape_contact_check(
                &object.state.vertices,
                candidate,
                landscape,
                &self.materials,
                solid_masks.as_slice(),
                solid_mask_removed.then_some(object_id),
                object.state.contact_density,
            )
        };
        let contacted = contact.is_contact();
        self.objects[index].latch_shape_contact(&contact);
        if contacted {
            self.dispatch_live_movement_contact(object_id, MovementContactDispatch::ShapeProbe)?;
            self.refresh_live_movement_solid_masks(solid_masks);
        }
        let result = contacted
            && self
                .find_object_index(object_id)
                .is_some_and(|index| self.objects[index].frame_shape_contact_count != 0);
        #[cfg(test)]
        if let Some(object) = self
            .find_object_index(object_id)
            .and_then(|index| self.objects.get(index))
        {
            MOVEMENT_PARITY_TRACE.with(|trace| {
                if let Some(trace) = trace.borrow_mut().as_mut() {
                    trace.probes.push(MovementProbeTrace {
                        position: candidate,
                        object_position: object.state.position,
                        fixed_position: object.fixed_position,
                        rotation: object.state.rotation,
                        fixed_rotation: object.fixed_rotation,
                        fixed_velocity: object.fixed_velocity,
                        rotation_velocity: object.rotation_velocity,
                        motion_x: object.motion_x,
                        motion_y: object.motion_y,
                        result,
                        t_contact: object.frame_t_contact,
                        contact_count: object.frame_shape_contact_count,
                        contact_cnat: object.frame_shape_contact_cnat,
                        vertex_contacts: object.frame_vertex_contacts.clone(),
                        random_count: self.rng.count,
                        random_hold: self.rng.hold,
                    });
                }
            });
        }
        Ok(result)
    }

    fn begin_live_object_motion(
        &mut self,
        object_id: ObjectId,
        solid_mask_removed: &mut bool,
        mask_attachments: &mut Option<SolidMaskAttachmentBackup>,
    ) {
        if let Some(index) = self.find_object_index(object_id) {
            if let Some(backup) = self.remove_solid_mask_for_movement(index) {
                *mask_attachments = Some(backup);
            }
        }
        *solid_mask_removed = true;
    }

    fn advance_live_attached_position(
        &mut self,
        object_id: ObjectId,
        solid_masks: &mut Vec<SolidMaskRect>,
        initial_solid_mask_removed: bool,
        mask_attachments: &mut Option<SolidMaskAttachmentBackup>,
    ) -> Result<MovementStepOutcome, EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(MovementStepOutcome::default());
        };
        {
            let object = &mut self.objects[index];
            object.fixed_position += object.fixed_velocity;
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(MovementStepOutcome::default());
        };
        let mut target_x = fixtoi(self.objects[index].fixed_position.x);
        let mut target_y = fixtoi(self.objects[index].fixed_position.y);
        self.apply_live_movement_side_bounds(object_id, &mut target_x, solid_masks)?;
        self.apply_live_movement_vertical_bounds(object_id, &mut target_y, solid_masks)?;

        let mut no_attach = false;
        let mut any_contact = false;
        let mut contact_cnat = CNAT_NONE;
        let mut solid_mask_removed = initial_solid_mask_removed;
        let mut first_step = true;
        while first_step
            || self.find_object_index(object_id).is_some_and(|index| {
                let position = self.objects[index].state.position;
                position.x != target_x || position.y != target_y
            })
        {
            first_step = false;
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            let original = {
                let position = self.objects[index].state.position;
                Vector2::new(
                    position.x + sign_i32(target_x - position.x),
                    position.y + sign_i32(target_y - position.y),
                )
            };
            let mut candidate = original;
            let attached = {
                let object = &mut self.objects[index];
                let Some(landscape) = self.landscape.as_ref() else {
                    break;
                };
                shape_attach(
                    &object.state.vertices,
                    &mut candidate,
                    object.movement_attach(),
                    landscape,
                    &self.materials,
                    solid_masks.as_slice(),
                    solid_mask_removed.then_some(object_id),
                    object.state.contact_density,
                    &mut object.state.shape_attach,
                )
            };
            if !attached {
                no_attach = true;
            }

            let contacted = self.probe_live_movement_contact(
                object_id,
                candidate,
                solid_masks,
                solid_mask_removed,
            )?;
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            if contacted {
                any_contact = true;
                contact_cnat |= self.objects[index].frame_t_contact;
                let object = &mut self.objects[index];
                object.fixed_position =
                    FixedVec2::from_ints(object.state.position.x, object.state.position.y);
                // C4Movement.cpp:363-368 applies these attachment
                // overrides after ContactCheck, even when the contact aborts
                // the step.
                if candidate.x != original.x {
                    object.fixed_velocity.x = C4Fixed::ZERO;
                }
                if candidate.y != original.y {
                    object.fixed_velocity.y = C4Fixed::ZERO;
                }
                break;
            }

            let override_x = candidate.x != original.x;
            let override_y = candidate.y != original.y;
            self.begin_live_object_motion(object_id, &mut solid_mask_removed, mask_attachments);
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            let object = &mut self.objects[index];
            object.motion_x = object
                .motion_x
                .saturating_add(candidate.x - object.state.position.x);
            object.motion_y = object
                .motion_y
                .saturating_add(candidate.y - object.state.position.y);
            object.state.position = candidate;
            if override_x {
                target_x = object.state.position.x;
                object.fixed_velocity.x = C4Fixed::ZERO;
                object.fixed_position.x = itofix(object.state.position.x);
            }
            if override_y {
                target_y = object.state.position.y;
                object.fixed_velocity.y = C4Fixed::ZERO;
                object.fixed_position.y = itofix(object.state.position.y);
            }
        }
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].refresh_velocity_from_fixed();
        }
        Ok(MovementStepOutcome {
            no_attach,
            redirect_yr: false,
            any_contact,
            contact_cnat,
            solid_mask_removed,
        })
    }

    fn advance_live_position_per_pixel(
        &mut self,
        object_id: ObjectId,
        solid_masks: &mut Vec<SolidMaskRect>,
        mask_attachments: &mut Option<SolidMaskAttachmentBackup>,
    ) -> Result<MovementStepOutcome, EngineError> {
        if self.landscape.is_none() {
            let Some(index) = self.find_object_index(object_id) else {
                return Ok(MovementStepOutcome::default());
            };
            let object = &mut self.objects[index];
            let previous_position = object.state.position;
            object.advance_fixed_position();
            object.motion_x = object
                .motion_x
                .saturating_add(object.state.position.x - previous_position.x);
            object.motion_y = object
                .motion_y
                .saturating_add(object.state.position.y - previous_position.y);
            return Ok(MovementStepOutcome {
                solid_mask_removed: object.state.position != previous_position,
                ..MovementStepOutcome::default()
            });
        }

        let Some(index) = self.find_object_index(object_id) else {
            return Ok(MovementStepOutcome::default());
        };
        if self.objects[index].movement_attach() != CNAT_NONE {
            return self.advance_live_attached_position(
                object_id,
                solid_masks,
                false,
                mask_attachments,
            );
        }

        let mut outcome = MovementStepOutcome::default();
        let mut solid_mask_removed = false;
        {
            let object = &mut self.objects[index];
            object.fixed_position.x += object.fixed_velocity.x;
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(outcome);
        };
        let mut target_x = fixtoi(self.objects[index].fixed_position.x);
        self.apply_live_movement_side_bounds(object_id, &mut target_x, solid_masks)?;
        while self
            .find_object_index(object_id)
            .is_some_and(|index| self.objects[index].state.position.x != target_x)
        {
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            let position = self.objects[index].state.position;
            let next_x = position.x + sign_i32(target_x - position.x);
            let candidate = Vector2::new(next_x, position.y);
            let contacted = self.probe_live_movement_contact(
                object_id,
                candidate,
                solid_masks,
                solid_mask_removed,
            )?;
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            if contacted {
                outcome.any_contact = true;
                outcome.contact_cnat |= self.objects[index].frame_t_contact;
                let object = &mut self.objects[index];
                object.fixed_position.x = itofix(object.state.position.x);
                redirect_force(
                    &mut object.fixed_velocity.x,
                    &mut object.fixed_velocity.y,
                    -1,
                );
                let friction = object.live_contact_first_friction();
                apply_contact_friction(&mut object.fixed_velocity.y, friction);
                break;
            }
            self.begin_live_object_motion(object_id, &mut solid_mask_removed, mask_attachments);
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            let object = &mut self.objects[index];
            object.motion_x = object
                .motion_x
                .saturating_add(next_x - object.state.position.x);
            object.state.position.x = next_x;
        }

        let Some(index) = self.find_object_index(object_id) else {
            outcome.solid_mask_removed = solid_mask_removed;
            return Ok(outcome);
        };
        {
            let object = &mut self.objects[index];
            object.fixed_position.y += object.fixed_velocity.y;
        }
        let Some(index) = self.find_object_index(object_id) else {
            outcome.solid_mask_removed = solid_mask_removed;
            return Ok(outcome);
        };
        let mut target_y = fixtoi(self.objects[index].fixed_position.y);
        self.apply_live_movement_vertical_bounds(object_id, &mut target_y, solid_masks)?;
        while self
            .find_object_index(object_id)
            .is_some_and(|index| self.objects[index].state.position.y != target_y)
        {
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            let position = self.objects[index].state.position;
            let next_y = position.y + sign_i32(target_y - position.y);
            let candidate = Vector2::new(position.x, next_y);
            let contacted = self.probe_live_movement_contact(
                object_id,
                candidate,
                solid_masks,
                solid_mask_removed,
            )?;
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            if contacted {
                outcome.any_contact = true;
                outcome.contact_cnat |= self.objects[index].frame_t_contact;
                let object = &mut self.objects[index];
                object.fixed_position.y = itofix(object.state.position.y);
                let friction = object.live_contact_first_friction();
                apply_contact_friction(&mut object.fixed_velocity.x, friction);
                if !object.live_contact_has_vertex_cnat(CNAT_LEFT) {
                    redirect_force(
                        &mut object.fixed_velocity.y,
                        &mut object.fixed_velocity.x,
                        -1,
                    );
                } else if !object.live_contact_has_vertex_cnat(CNAT_RIGHT) {
                    redirect_force(
                        &mut object.fixed_velocity.y,
                        &mut object.fixed_velocity.x,
                        1,
                    );
                } else {
                    if object.state.ocf & crate::ocf::ROTATE != 0
                        && object.frame_shape_contact_count == 1
                        && !object.state.alive
                    {
                        let weight = object.live_contact_first_weight();
                        redirect_force(
                            &mut object.fixed_velocity.y,
                            &mut object.rotation_velocity,
                            -weight,
                        );
                        outcome.redirect_yr = true;
                    }
                    object.fixed_velocity.y = C4Fixed::ZERO;
                }
                break;
            }
            self.begin_live_object_motion(object_id, &mut solid_mask_removed, mask_attachments);
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            let object = &mut self.objects[index];
            object.motion_y = object
                .motion_y
                .saturating_add(next_y - object.state.position.y);
            object.state.position.y = next_y;
        }

        outcome.solid_mask_removed = solid_mask_removed;
        if self
            .find_object_index(object_id)
            .is_some_and(|index| self.objects[index].movement_attach() != CNAT_NONE)
        {
            let attached = self.advance_live_attached_position(
                object_id,
                solid_masks,
                solid_mask_removed,
                mask_attachments,
            )?;
            outcome.no_attach |= attached.no_attach;
            outcome.redirect_yr |= attached.redirect_yr;
            outcome.any_contact |= attached.any_contact;
            outcome.contact_cnat |= attached.contact_cnat;
            outcome.solid_mask_removed |= attached.solid_mask_removed;
        }
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].refresh_velocity_from_fixed();
        }
        Ok(outcome)
    }

    /// Test-only entry to the exact `DoUnattachedMovement` translation stage.
    /// The oracle stops before rotation and `ContactAction`, so using the full
    /// `exec_object_movement` here would compare state from later C++ stages.
    #[cfg(test)]
    pub(crate) fn parity_advance_live_position_per_pixel(
        &mut self,
        index: usize,
    ) -> Result<(MovementStepOutcome, MovementParityTrace), EngineError> {
        let object_id = self.objects[index].id;
        self.objects[index].motion_x = 0;
        self.objects[index].motion_y = 0;
        let mut solid_masks = self.live_movement_solid_masks();
        let mut mask_attachments = None;
        begin_movement_parity_trace();
        let outcome = self.advance_live_position_per_pixel(
            object_id,
            &mut solid_masks,
            &mut mask_attachments,
        );
        let trace = take_movement_parity_trace();
        outcome.map(|outcome| (outcome, trace))
    }

    /// Test-only entry to the exact rotation block that follows unattached
    /// translation in `C4Object::DoMovement` (C4Movement.cpp:372-436).
    #[cfg(test)]
    pub(crate) fn parity_advance_live_rotation(
        &mut self,
        index: usize,
        redirect_yr: bool,
    ) -> Result<((bool, u32, bool), MovementParityTrace), EngineError> {
        let object_id = self.objects[index].id;
        let mut solid_masks = self.live_movement_solid_masks();
        begin_movement_parity_trace();
        let outcome =
            self.advance_live_rotation(object_id, &mut solid_masks, false, redirect_yr, false);
        let trace = take_movement_parity_trace();
        outcome.map(|outcome| (outcome, trace))
    }

    #[cfg(test)]
    pub(crate) fn parity_exec_object_movement(
        &mut self,
        index: usize,
        action_library: &ActionLibrary,
        definition_id: &DefinitionId,
        solid_mask_indices: &[usize],
    ) -> Result<(ExecMovementOutcome, MovementParityTrace), EngineError> {
        begin_movement_parity_trace();
        let outcome =
            self.exec_object_movement(index, action_library, definition_id, solid_mask_indices);
        let trace = take_movement_parity_trace();
        outcome.map(|outcome| (outcome, trace))
    }

    pub(crate) fn advance_live_rotation(
        &mut self,
        object_id: ObjectId,
        solid_masks: &mut Vec<SolidMaskRect>,
        no_attach: bool,
        redirect_yr: bool,
        solid_mask_removed: bool,
    ) -> Result<(bool, u32, bool), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok((false, CNAT_NONE, false));
        };
        if self.objects[index].state.ocf & crate::ocf::ROTATE == 0
            || !self.objects[index].rotation_velocity.is_nonzero()
        {
            return Ok((false, CNAT_NONE, false));
        }
        {
            let object = &mut self.objects[index];
            object.fixed_rotation += object.rotation_velocity * 5;
        }
        let rotateable = self
            .movement_live_config_at(object_id)
            .map(|live| live.rotateable)
            .unwrap_or(0);
        let Some(index) = self.find_object_index(object_id) else {
            return Ok((false, CNAT_NONE, false));
        };
        {
            let object = &mut self.objects[index];
            if rotateable > 1 {
                let limit = itofix(rotateable);
                if object.fixed_rotation > limit {
                    object.fixed_rotation = limit;
                    object.rotation_velocity = C4Fixed::ZERO;
                }
                if object.fixed_rotation < -limit {
                    object.fixed_rotation = -limit;
                    object.rotation_velocity = C4Fixed::ZERO;
                }
            }
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok((false, CNAT_NONE, false));
        };
        let target_rotation = fixtoi(self.objects[index].fixed_rotation);
        let mut any_contact = false;
        let mut contact_cnat = CNAT_NONE;
        let mut turned = false;

        if self.landscape.is_some() {
            while self
                .find_object_index(object_id)
                .is_some_and(|index| self.objects[index].state.rotation != target_rotation)
            {
                let Some(index) = self.find_object_index(object_id) else {
                    break;
                };
                let (
                    previous_rotation,
                    previous_vertices,
                    previous_shape_vertices,
                    previous_shape_rect,
                    previous_fire_top,
                    previous_shape_override,
                    previous_vertex_contacts,
                    previous_shape_contact_cnat,
                    previous_shape_contact_count,
                    previous_attach,
                    previous_contact_density,
                ) = {
                    let object = &self.objects[index];
                    (
                        object.state.rotation,
                        object.state.vertices.clone(),
                        object.state.shape_vertices.clone(),
                        object.shape_rect,
                        object.shape_fire_top,
                        object.state.shape_override,
                        object.frame_vertex_contacts.clone(),
                        object.frame_shape_contact_cnat,
                        object.frame_shape_contact_count,
                        object.state.shape_attach,
                        object.state.contact_density,
                    )
                };
                let shape_updated = {
                    let object = &mut self.objects[index];
                    object.state.rotation += sign_i32(target_rotation - object.state.rotation);
                    let shape_updated = object.shape_template.line == 0;
                    if shape_updated {
                        object.refresh_shape_geometry();
                    }
                    shape_updated
                };
                if shape_updated {
                    // UpdateShape calls UpdatePos before Shape.Attach and
                    // ContactCheck for every attempted degree
                    // (C4Object.cpp:322-344; C4Movement.cpp:397-411).
                    self.update_sector_for_index(index);
                    #[cfg(test)]
                    record_movement_update_pos();
                }

                let Some(index) = self.find_object_index(object_id) else {
                    break;
                };
                let mut candidate_position = self.objects[index].state.position;
                let attach = self.objects[index].movement_attach();
                if attach != CNAT_NONE && !no_attach {
                    let object = &mut self.objects[index];
                    let Some(landscape) = self.landscape.as_ref() else {
                        break;
                    };
                    shape_attach(
                        &object.state.vertices,
                        &mut candidate_position,
                        attach,
                        landscape,
                        &self.materials,
                        solid_masks.as_slice(),
                        solid_mask_removed.then_some(object_id),
                        object.state.contact_density,
                        &mut object.state.shape_attach,
                    );
                }

                let contacted = self.probe_live_movement_contact(
                    object_id,
                    candidate_position,
                    solid_masks,
                    solid_mask_removed,
                )?;
                let Some(index) = self.find_object_index(object_id) else {
                    break;
                };
                if contacted {
                    any_contact = true;
                    contact_cnat |= self.objects[index].frame_t_contact;
                    let contact_count = self.objects[index].frame_shape_contact_count;
                    let object = &mut self.objects[index];
                    let owns_shape_vertices = object.own_shape_vertices.is_some();
                    object.state.rotation = previous_rotation;
                    object.state.vertices = previous_vertices;
                    object.state.shape_vertices = previous_shape_vertices;
                    object.own_shape_vertices = owns_shape_vertices
                        .then(|| object.state.shape_vertices.own_original_vertices());
                    object.shape_rect = previous_shape_rect;
                    object.shape_fire_top = previous_fire_top;
                    object.state.shape_override = previous_shape_override;
                    object.frame_vertex_contacts = previous_vertex_contacts;
                    object.frame_shape_contact_cnat = previous_shape_contact_cnat;
                    object.frame_shape_contact_count = previous_shape_contact_count;
                    object.state.shape_attach = previous_attach;
                    object.state.contact_density = previous_contact_density;
                    object.fixed_rotation = itofix(previous_rotation);
                    if contact_count == 1 && !redirect_yr {
                        redirect_force(
                            &mut object.rotation_velocity,
                            &mut object.fixed_velocity.y,
                            -1,
                        );
                    }
                    object.rotation_velocity = C4Fixed::ZERO;
                    object.refresh_velocity_from_fixed();
                    self.update_sector_for_index(index);
                    #[cfg(test)]
                    record_movement_update_pos();
                    break;
                }
                if let Some(index) = self.find_object_index(object_id) {
                    self.objects[index].state.position = candidate_position;
                    turned = true;
                }
            }
        } else if let Some(index) = self.find_object_index(object_id) {
            let object = &mut self.objects[index];
            let changed = object.state.rotation != target_rotation;
            object.state.rotation = target_rotation;
            if changed && object.shape_template.line == 0 {
                object.refresh_shape_geometry();
            }
            turned = changed;
        }

        if let Some(index) = self.find_object_index(object_id) {
            let object = &mut self.objects[index];
            let half_circle = itofix(FIX_HALF_CIRCLE);
            let full_circle = itofix(FIX_FULL_CIRCLE);
            if object.fixed_rotation < -half_circle {
                object.fixed_rotation += full_circle;
                object.state.rotation = fixtoi(object.fixed_rotation);
            }
            if object.fixed_rotation > half_circle {
                object.fixed_rotation -= full_circle;
                object.state.rotation = fixtoi(object.fixed_rotation);
            }
        }
        Ok((any_contact, contact_cnat, turned))
    }

    /// Complete mobile leg of C4Object::ExecMovement. AssignRemoval from a
    /// DoMovement callback does not unwind the native stack: demobilization,
    /// Stabilize and the raw non-rotateable assignment still run before
    /// C4Object::Execute observes Status=0 (oracle-src-pinned
    /// src/C4Movement.cpp:558-620; src/C4Object.cpp:1082-1094).
    #[doc(hidden)]
    pub fn exec_mobile_object_movement(
        &mut self,
        idx: usize,
        action_library: &ActionLibrary,
        definition_id: &DefinitionId,
        solid_mask_indices: &[usize],
    ) -> Result<ExecMovementOutcome, EngineError> {
        let object_id = self.objects[idx].id;
        let outcome =
            self.exec_object_movement(idx, action_library, definition_id, solid_mask_indices)?;
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(outcome);
        };

        // Same-frame friction/contact zeroing demobilizes immediately
        // (C4Movement.cpp:592-593).
        let object = &mut self.objects[idx];
        if !object.fixed_velocity.x.is_nonzero()
            && !object.fixed_velocity.y.is_nonzero()
            && !object.rotation_velocity.is_nonzero()
        {
            object.state.mobile = false;
        }
        // Stabilize while not rotating, including a Status=0 tombstone still
        // resident on this synchronous stack (C4Movement.cpp:594-595).
        if !self.objects[idx].rotation_velocity.is_nonzero() {
            self.stabilize_object(idx, solid_mask_indices)?;
        }
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(outcome);
        };
        // This is a raw r assignment: fix_r, rdir, Shape and OCF stay live.
        let non_rotateable = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .is_some_and(|definition| definition.rotateable() == 0);
        if non_rotateable {
            self.objects[idx].state.rotation = 0;
        }
        Ok(outcome)
    }

    /// The DoMovement portion of C4Object::ExecMovement plus the
    /// tail C++ runs inside it: the InLiquid update
    /// (C4Movement.cpp:443-460), ContactAction/NoAttachAction dispatch
    /// (:463-470) and the Hit* calls (:472-478). Reports both whether the
    /// object survived and whether positional integration reached DoMotion;
    /// only the latter restores a moving mask's attachment backup
    /// (C4Movement.cpp:121-126,443-445).
    #[doc(hidden)]
    pub fn exec_object_movement(
        &mut self,
        idx: usize,
        _action_library: &ActionLibrary,
        definition_id: &DefinitionId,
        _solid_mask_indices: &[usize],
    ) -> Result<ExecMovementOutcome, EngineError> {
        // C4Object::DoMovement resets the displacement cache before any
        // restriction, collision probe, or DoMotion call.
        self.objects[idx].motion_x = 0;
        self.objects[idx].motion_y = 0;
        // C4Object::DoMovement applies Def->NoHorizontalMove before DigFree
        // predicts its target and before the old dirs for Hit* are captured
        // (C4Movement.cpp:224-251).
        let no_horizontal_move = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .map(Definition::no_horizontal_move)
            .unwrap_or(0);
        if no_horizontal_move != 0 {
            let object = &mut self.objects[idx];
            object.fixed_velocity.x = C4Fixed::ZERO;
            object.refresh_velocity_from_fixed();
        }
        self.apply_dig_procedure(idx, definition_id);
        // C++ captures ix0/iy0 after DigFree and before any translation or
        // rotation. Its final UpdatePos is selected from this exact pair
        // after all movement callbacks have completed (C4Movement.cpp:247,
        // 480-491).
        let entry_position = self.objects[idx].state.position;
        // DoMovement snapshots post-action dirs for Hit* arguments but gates
        // the callbacks with the already-cached OCF field; command/action
        // mutations may have refreshed that cache without making its clock
        // identical to the dirs (C4Movement.cpp:250-252,477-483).
        let old_movement_velocity = self.objects[idx].fixed_velocity;
        let old_movement_hit_flags = self.objects[idx].state.ocf;
        let object_id = self.objects[idx].id;
        let mut solid_masks = self.live_movement_solid_masks();
        let mut mask_attachments = None;
        let mut movement_outcome = self.advance_live_position_per_pixel(
            object_id,
            &mut solid_masks,
            &mut mask_attachments,
        )?;
        let (rotation_contact, rotation_cnat, turned) = self.advance_live_rotation(
            object_id,
            &mut solid_masks,
            movement_outcome.no_attach,
            movement_outcome.redirect_yr,
            movement_outcome.solid_mask_removed,
        )?;
        movement_outcome.any_contact |= rotation_contact;
        movement_outcome.contact_cnat |= rotation_cnat;
        let did_motion = movement_outcome.solid_mask_removed;
        // DoMovement's unconditional UpdateSolidMask(true) tail precedes
        // InLiquid, ContactAction, NoAttachAction, and Hit callbacks
        // (C4Movement.cpp:443-478). With no DoMotion this performs the real
        // remove(no backup)+put cycle; after motion it re-puts at the final
        // position and translates the riders captured by the first removal.
        let Some(idx) = self.find_object_index(object_id) else {
            return Ok(ExecMovementOutcome { alive: false });
        };
        self.update_solid_mask(idx);
        self.restore_solid_mask_attachments(idx, did_motion.then_some(mask_attachments).flatten());
        // C4Object::InLiquid update, inline in DoMovement after
        // integration and BEFORE ContactAction/NoAttachAction
        // (C4Movement.cpp:443-460): IsInLiquidCheck probes
        // GBackLiquid(x, y + Float*Con/FullCon - 1)
        // (C4Object.cpp:5632-5635); entering liquid clears fNoAttach
        // (:452). DoMovement never runs contained or C4D_StaticBack
        // (C4Movement.cpp:553-575; the outer ExecMovement gate has already
        // selected this DoMovement invocation). A callback changing
        // Contained/category does not retroactively skip this tail.
        // The entry Splash (:450-451, OCF_HitSpeed2 && Mass>3) draws from
        // the SYNCHRONIZED Random() stream — the bubble draws first, then
        // the extracted-material cast (C4Object.cpp:6093-6110) — so its
        // draw count and order are determinism-critical: never skip it,
        // reorder its draws, or move them onto an unsynced stream.
        let probe = {
            let state = &self.objects[idx].state;
            let float_line = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .map(|definition| definition.float_line)
                .unwrap_or(0);
            Vector2::new(
                state.position.x,
                crate::engine_splash::liquid_probe_y(
                    state.position.y,
                    float_line,
                    state.construction,
                ),
            )
        };
        let wet = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.is_liquid_at(probe.x, probe.y))
            .unwrap_or(false);
        let state = &self.objects[idx].state;
        if crate::engine_splash::entered_liquid(wet, state.in_liquid) {
            // Entry splash (C4Movement.cpp:450-453): fast + heavy
            // objects splash — synced RNG draws + FXU1 bubbles.
            let object_mass = self.effective_object_mass(idx);
            let state = &self.objects[idx].state;
            let should_splash =
                crate::engine_splash::should_splash(wet, state.in_liquid, state.ocf, object_mass);
            let (splash_x, splash_y, splash_amt) = {
                let shape = self.objects[idx].current_shape_rect();
                let area = shape
                    .map(|rect| crate::engine_splash::splash_amount(rect.width, rect.height))
                    .unwrap_or(0);
                (state.position.x, state.position.y + 1, area)
            };
            if should_splash {
                self.splash(splash_x, splash_y, splash_amt)?;
            }
            let state = &mut self.objects[idx].state;
            state.in_liquid = true;
            movement_outcome.no_attach = false;
        } else if !wet && self.objects[idx].state.in_liquid {
            self.objects[idx].state.in_liquid = false;
        }
        // Contact Action, then Attachment Loss Action, then the Hit
        // script calls (C4Movement.cpp:463-478).
        if movement_outcome.any_contact {
            let Some(idx) = self.find_object_index(object_id) else {
                return Ok(ExecMovementOutcome { alive: false });
            };
            // The most recent rotation/contact probe remains visible through
            // UpdateSolidMask and InLiquid/Splash. C++ restores accumulated
            // iContacts to t_contact only immediately before ContactAction
            // (C4Movement.cpp:443-470).
            #[cfg(test)]
            record_pre_contact_action_t_contact(self.objects[idx].frame_t_contact);
            self.objects[idx].frame_t_contact = movement_outcome.contact_cnat;
            let definition_id = self.objects[idx].definition_id.clone();
            self.exec_contact_action(idx, movement_outcome.contact_cnat, &definition_id)?;
        }
        if movement_outcome.no_attach {
            let Some(idx) = self.find_object_index(object_id) else {
                return Ok(ExecMovementOutcome { alive: false });
            };
            let definition_id = self.objects[idx].definition_id.clone();
            if let Some(action_library) = self
                .definitions
                .get(&definition_id)
                .map(Definition::shared_action_library_handle)
            {
                self.apply_no_attach_action(idx, &definition_id, &action_library)?;
            }
        }
        if movement_outcome.any_contact {
            self.invoke_movement_hit_callbacks(
                old_movement_velocity,
                old_movement_hit_flags,
                object_id,
            )?;
        }
        // C4Movement's final graphics/position tail runs after Hit*. Any
        // accepted rotation degree makes fTurned sticky and therefore calls
        // UpdateFace(true): rebuild the LIVE definition shape, UpdatePos, and
        // perform the second solid-mask remove/re-put. Without rotation,
        // UpdatePos runs only when the entry integer position differs
        // (C4Movement.cpp:398-429,443-491; C4Object.cpp:322-376).
        if let Some(idx) = self.find_object_index(object_id) {
            if turned {
                let shape_updated = self.objects[idx].shape_template.line == 0;
                self.objects[idx].refresh_shape_geometry();
                if shape_updated {
                    self.update_sector_for_index(idx);
                }
                self.update_solid_mask(idx);
            } else if self.objects[idx].state.position != entry_position {
                self.update_sector_for_index(idx);
            }
        }
        let alive = self.find_object_index(object_id).is_some_and(|idx| {
            !self.objects[idx].destroyed
                && !matches!(self.objects[idx].state.status, ObjectStatus::Deleted)
        });
        Ok(ExecMovementOutcome { alive })
    }

    /// `DoGravity(this)` as used by the ExecAction idle and insufficient
    /// action-energy returns (C4Object.cpp:4644-4664, 4708-4712, 4747-4752).
    /// This is the raw native operation: no procedure gravity mask, terminal
    /// clamp, steering, phase work, or generic DoEnergy side effects.
    fn apply_do_gravity_at_index(&mut self, idx: usize) {
        if idx >= self.objects.len() {
            return;
        }
        let float_line = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .map(|definition| definition.float_line)
            .unwrap_or(0);
        let floats = self.objects[idx].state.in_liquid && float_line != 0;
        let surfaced = if floats {
            let state = &self.objects[idx].state;
            let probe_y = state.position.y - 1
                + float_line
                    .saturating_mul(state.construction)
                    .checked_div(FULL_CON)
                    .unwrap_or(0)
                - 1;
            self.landscape
                .as_ref()
                .map(|landscape| !landscape.is_liquid_at(state.position.x, probe_y))
                .unwrap_or(true)
        } else {
            false
        };
        let gravity = self.physics.gravity_as_c4fixed();
        let object = &mut self.objects[idx];
        if floats {
            object.fixed_velocity.y -= math::FLOAT_ACCEL;
            let min_rise = C4Fixed::from_raw(-10 * math::FLOAT_ACCEL.val());
            if object.fixed_velocity.y < min_rise {
                object.fixed_velocity.y = min_rise;
            }
            let friction = math::FLOAT_FRICTION;
            if object.fixed_velocity.x < -friction {
                object.fixed_velocity.x += friction;
            } else if object.fixed_velocity.x > friction {
                object.fixed_velocity.x -= friction;
            }
            if object.rotation_velocity < -friction {
                object.rotation_velocity += friction;
            } else if object.rotation_velocity > friction {
                object.rotation_velocity -= friction;
            }
            if surfaced && object.fixed_velocity.y < C4Fixed::ZERO {
                object.fixed_velocity.y = C4Fixed::ZERO;
            }
        } else if object.state.category & CATEGORY_STATIC_BACK == 0 {
            object.fixed_velocity.y += gravity;
        }
        object.refresh_velocity_from_fixed();
    }

    #[doc(hidden)]
    pub fn apply_physics_at_index(&mut self, idx: usize) -> Result<bool, EngineError> {
        self.apply_physics_at_index_inner(idx, None, None)
    }

    pub(crate) fn apply_physics_at_index_inner(
        &mut self,
        mut idx: usize,
        captured_physical: Option<&mut Option<PhysicalInfo>>,
        mut captured_phase_advance: Option<&mut Option<i32>>,
    ) -> Result<bool, EngineError> {
        if idx >= self.objects.len() {
            return Ok(false);
        }
        // Action.t_attach resets each ExecAction (C4Object.cpp:4692);
        // early returns below leave it CNAT_None for this frame's
        // movement.
        self.objects[idx].frame_t_attach = CNAT_NONE;
        // Upright attachment check precedes the idle, incomplete, and
        // action-energy gates.
        // (C4Object.cpp:4698-4705). Preserve its Action.t_attach bit even
        // when either later gate returns before procedure attachment latches.
        {
            let upright_attach = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .map(|definition| definition.upright_attach())
                .unwrap_or(0);
            let object = &mut self.objects[idx];
            object.upright_t_attach = 0;
            object.swim_exit_this_frame = false;
            if !object.state.mobile && upright_attach != 0 {
                let rotation = object.state.rotation;
                let signed = if rotation > 180 {
                    rotation - 360
                } else {
                    rotation
                };
                if (-math::STABLE_RANGE..=math::STABLE_RANGE).contains(&signed) {
                    object.upright_t_attach = upright_attach as u32;
                    object.state.mobile = true;
                }
            }
            object.frame_t_attach = object.upright_t_attach;
            object.state.t_attach = object.frame_t_attach;
        }

        let (is_idle_action, incomplete_activity, energy_usage) = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .map(|definition| {
                let action = &self.objects[idx].state.action;
                let library = definition.action_library();
                (
                    library.is_idle_state(action),
                    definition.incomplete_activity(),
                    library.energy_usage_for_entry(&action.name, action.act_map_index),
                )
            })
            .unwrap_or((true, false, 0));

        // Native idle actions return before C4Object::GetPhysical
        // (C4Object.cpp:4718-4723). Besides preserving hook/RNG laziness,
        // this keeps their gravity-only path out of the phase tail.
        if is_idle_action {
            if self.objects[idx].state.mobile {
                self.apply_do_gravity_at_index(idx);
            }
            return Ok(true);
        }

        // A real action on an object without OCF_FullCon is reset through
        // ordinary SetAction(ActIdle) unless its definition opts into
        // IncompleteActivity (C4Object.cpp:4725-4729). SetAction supplies the
        // synchronous Abort callback, OCF refresh and fixed-position resync;
        // ExecAction returns even when NoOtherAction rejects the transition.
        if self.objects[idx].state.ocf & crate::ocf::FULL_CON == 0 && !incomplete_activity {
            let definition_id = self.objects[idx].definition_id.clone();
            let _ = tolerate_script_error(self.action_with_calls(idx, &definition_id, "Idle"))?;
            return Ok(true);
        }

        // C++ retains pAction first, then resolves pPhysical exactly once
        // before the action-energy gate and every script-capable procedure
        // (C4Object.cpp:4731-4733). Keep the pAction-derived metadata and
        // copied physical together across callbacks below.
        let action_definition_id = self.objects[idx].definition_id.clone();
        let (
            procedure,
            movement_profile,
            mut gravity_component,
            is_idle,
            action_attach,
            action_disabled,
            in_liquid_action,
        ) = {
            let gravity = self.physics.gravity_as_c4fixed();
            if let Some(definition) = self.definitions.get(&action_definition_id) {
                let object = &self.objects[idx];
                let library = definition.action_library();
                let procedure = library.procedure_for_entry(
                    &object.state.action.name,
                    object.state.action.act_map_index,
                );
                (
                    procedure,
                    definition.movement_profile(),
                    procedure.gravity_component_fixed(gravity),
                    library.is_idle_state(&object.state.action),
                    library.attach_for_entry(
                        &object.state.action.name,
                        object.state.action.act_map_index,
                    ),
                    library.disables_object_for_entry(
                        &object.state.action.name,
                        object.state.action.act_map_index,
                    ),
                    library
                        .in_liquid_action_for_entry(
                            &object.state.action.name,
                            object.state.action.act_map_index,
                        )
                        .map(str::to_string),
                )
            } else {
                let procedure = ActionProcedure::default();
                (
                    procedure,
                    MovementProfile::default(),
                    procedure.gravity_component_fixed(gravity),
                    true,
                    0,
                    false,
                    None,
                )
            }
        };
        let physical = self.object_physical(idx);
        if let Some(slot) = captured_physical {
            *slot = Some(physical);
        }

        // C4ActionDef::EnergyUsage is a signed, nonzero gate on every real
        // ActMap action while C4RULE_StructuresNeedEnergy is active
        // (C4Object.cpp:4738-4753). It runs before Action.Time++,
        // InLiquidAction, steering and phase advance. Insufficient power is
        // the native idle return: mark NeedEnergy, apply raw gravity only to
        // Mobile objects, and skip all remaining action work.
        if self.structures_need_energy && energy_usage != 0 {
            if energy_usage <= self.objects[idx].state.energy {
                let object = &mut self.objects[idx];
                object.state.energy = object.state.energy.wrapping_sub(energy_usage);
                object.state.need_energy = false;
            } else {
                let mobile = {
                    let object = &mut self.objects[idx];
                    object.state.need_energy = true;
                    object.state.mobile
                };
                if mobile {
                    self.apply_do_gravity_at_index(idx);
                }
                return Ok(true);
            }
        }
        // Native increments the LIVE Action.Time exactly once after the
        // energy gate and before InLiquidAction/procedure dispatch
        // (C4Object.cpp:4755-4756). GetPhysical may have replaced the action,
        // so this deliberately does not write through the retained pAction.
        self.objects[idx].state.action.time = self.objects[idx].state.action.time.wrapping_add(1);
        // InLiquidAction check (C4Object.cpp:4749-4753): an InLiquid
        // object whose action declares one switches THROUGH
        // SetActionByName (Abort+Start calls, fix resync) and returns
        // early — steering and the phase advance skip; movement runs.
        if self.objects[idx].state.in_liquid {
            if let Some(target) = in_liquid_action {
                let definition_id = self.objects[idx].definition_id.clone();
                self.action_with_calls(idx, &definition_id, &target)?;
                return Ok(true);
            }
        }
        if idx >= self.objects.len() {
            return Ok(false);
        }
        let mut swim_walk_transition = false;

        let mut definition_id = self.objects[idx].definition_id.clone();
        let command_direction = self.objects[idx].state.command_direction;
        let action_target = self.objects[idx].state.action.target;

        // Latch this frame's Action.t_attach from the PRE-wrap action
        // (C4Object.cpp:4692 + per-procedure assignments): the phase-wrap
        // SetAction at ExecAction's end must not retroactively attach
        // this frame's movement.
        self.objects[idx].frame_t_attach = procedure_t_attach(
            procedure,
            is_idle,
            self.objects[idx].state.direction,
            action_attach,
            self.objects[idx].upright_t_attach,
        );
        // Mirror into the script-visible state: FnAdjustWalkRotation
        // reads Action.t_attach (C4Script.cpp:5444).
        self.objects[idx].state.t_attach = self.objects[idx].frame_t_attach;

        // Once an object is controllable again, an older attack no longer
        // owns a later environmental death (C4Object.cpp:4771-4776). Flight,
        // swimming, disabled actions, and burning deliberately retain the
        // trace so throws, drowning, and fire deaths keep their attacker.
        if !is_idle
            && !action_disabled
            && !matches!(procedure, ActionProcedure::Flight | ActionProcedure::Swim)
            && !self.objects[idx].state.on_fire
        {
            self.objects[idx].last_energy_loss_cause = OWNER_NONE;
        }

        // DFA_CONNECT is procedure work, so it follows the action-energy,
        // Action.Time, InLiquidAction, attachment, and attribution steps
        // (C4Object.cpp:4738-4776, 5341-5420). Broken targets fire LineBreak
        // and remove the line, skipping the rest of this object's exec.
        if matches!(procedure, ActionProcedure::Connect) && !self.exec_connect_line(idx)? {
            return Ok(true);
        }

        // DFA_DIG must stay attached to solid ground. C++ performs this
        // independent CNAT_Bottom probe before assigning dig velocity and
        // stops digging immediately when it fails (C4Object.cpp:4906-4911).
        // Synthetic fixture worlds without a landscape keep their historical
        // movement path; a real game always has GBack available here.
        if matches!(procedure, ActionProcedure::Dig) && self.landscape.is_some() {
            let solid_masks = self.live_movement_solid_masks();
            let attachment = self.landscape.as_ref().map(|landscape| {
                let object = &self.objects[idx];
                let mut sample_position = object.state.position;
                let mut record = object.state.shape_attach;
                let contact_density = object.state.contact_density;
                let attached = shape_attach(
                    &object.state.vertices,
                    &mut sample_position,
                    CNAT_BOTTOM,
                    landscape,
                    &self.materials,
                    &solid_masks,
                    Some(object.id),
                    contact_density,
                    &mut record,
                );
                (attached, record)
            });
            if let Some((attached, record)) = attachment {
                self.objects[idx].state.shape_attach = record;
                if !attached {
                    self.object_com_stop_dig(idx, &definition_id)?;
                    return Ok(true);
                }
            }
        }

        if matches!(procedure, ActionProcedure::Bridge)
            && !self.apply_bridge_procedure(idx, command_direction, &definition_id)?
        {
            // `if (!DoBridge(this)) return;` skips the phase tail
            // (C4Object.cpp:4998-4999).
            return Ok(true);
        }

        if matches!(procedure, ActionProcedure::Build) && !self.apply_build_procedure(idx)? {
            return Ok(true);
        }

        if matches!(procedure, ActionProcedure::Chop)
            && !self.apply_chop_procedure(idx, &definition_id)?
        {
            return Ok(true);
        }

        // A false helper result is a native `return` from ExecAction. Signal
        // the outer object loop to skip the captured action's phase tail.
        if matches!(procedure, ActionProcedure::Fight)
            && !self.apply_fight_procedure(idx, physical)?
        {
            return Ok(true);
        }

        if matches!(procedure, ActionProcedure::Attach)
            && !self.apply_attach_procedure(idx, &definition_id)?
        {
            return Ok(true);
        }

        let mut procedure_phase_advance = None;
        let mut push_handled = false;
        if matches!(procedure, ActionProcedure::Push) {
            if !self.apply_push_procedure(
                idx,
                command_direction,
                movement_profile,
                &definition_id,
                physical,
                &mut procedure_phase_advance,
            )? {
                return Ok(true);
            }
            push_handled = true;
        }

        let mut pull_handled = false;
        if matches!(procedure, ActionProcedure::Pull) {
            if !self.apply_pull_procedure(
                idx,
                command_direction,
                movement_profile,
                &definition_id,
                physical,
                &mut procedure_phase_advance,
            )? {
                return Ok(true);
            }
            pull_handled = true;
        }
        if let Some(slot) = captured_phase_advance.as_mut() {
            **slot = procedure_phase_advance;
        }

        // DFA_LIFT applies the target force and all target callbacks before
        // the lifter's trailing DoGravity (C4Object.cpp:5266-5289). Keep this
        // ahead of the generic gravity block below so LiftTop observes the
        // pre-gravity lifter state.
        if matches!(procedure, ActionProcedure::Lift) {
            let lifter_id = self.objects[idx].id;
            if !self.apply_lift_to_target(idx, command_direction, action_target)? {
                // C++ uses ordinary SetAction(ActIdle), including its
                // Start/Abort calls and NoOtherAction gate, then returns.
                let _ = self.action_with_calls(idx, &definition_id, "Idle")?;
                return Ok(true);
            }
            let Some(live_idx) = self.find_object_index(lifter_id) else {
                return Ok(true);
            };
            idx = live_idx;
            if self.objects[idx].destroyed
                || self.objects[idx].state.status == ObjectStatus::Deleted
            {
                return Ok(true);
            }
            // LiftTop may ChangeDef the lifter. DoGravity reads the live Def
            // (notably its Float line) and live GravAccel after that callback.
            definition_id = self.objects[idx].definition_id.clone();
            gravity_component = self.physics.gravity_as_c4fixed();
        }

        // DFA_FLIGHT contained fall-out (C4Object.cpp:4893-4900): on
        // Tick10, stop into Walk and add the delayed Wait, then immediately
        // replace that stack through plain SetCommand(Exit). Keep executing
        // this frame under the captured FLIGHT procedure so gravity/mobile
        // still run after the callbacks, just as the stale pAction does.
        if matches!(procedure, ActionProcedure::Flight)
            && self.frame.is_multiple_of(10)
            && self.objects[idx].state.container.is_some()
        {
            let object_id = self.objects[idx].id;
            self.stop_action_delay_command(idx, &definition_id)?;
            let Some(live_idx) = self.find_object_index(object_id) else {
                return Ok(true);
            };
            idx = live_idx;
            self.set_plain_exit_command(idx)?;
            let Some(live_idx) = self.find_object_index(object_id) else {
                return Ok(true);
            };
            idx = live_idx;
        }

        let mut exec_set_direction = None;
        {
            // At-limit physical training before the ComDir movement: Scale
            // Tick5 (C4Object.cpp:4810-4812), Hangle Tick5 (:4844-4846),
            // Swim Tick10 (:4924-4926).
            match procedure {
                ActionProcedure::Scale if physical.scale != 0 && self.frame.is_multiple_of(5) => {
                    let ydir = self.objects[idx].fixed_velocity.y;
                    if ydir.abs() == math::val_by_physical(200, physical.scale) {
                        self.train_physical(idx, "Scale", 1, C4_MAX_PHYSICAL);
                    }
                }
                ActionProcedure::Hang if physical.hangle != 0 && self.frame.is_multiple_of(5) => {
                    let xdir = self.objects[idx].fixed_velocity.x;
                    if xdir.abs() == math::val_by_physical(160, physical.hangle) {
                        self.train_physical(idx, "Hangle", 1, C4_MAX_PHYSICAL);
                    }
                }
                ActionProcedure::Swim if physical.swim != 0 && self.frame.is_multiple_of(10) => {
                    let xdir = self.objects[idx].fixed_velocity.x;
                    if xdir.abs() == math::val_by_physical(160, physical.swim) {
                        self.train_physical(idx, "Swim", 1, C4_MAX_PHYSICAL);
                    }
                }
                _ => {}
            }
            // DFA_SWIM liquid probes (C4Object.cpp:4946-4960): below =
            // GBackLiquid(x, y + 1 + Float*Con/FullCon - 1), surface =
            // GBackLiquid(x, y - 1 + Float*Con/FullCon - 1); position is
            // stable through the steering, so probe before the borrow.
            // Fixture worlds have no landscape at all — skip the liquid
            // checks entirely there (a real game always has one).
            let swim_probes_available = self.landscape.is_some();
            let (swim_below_wet, swim_surface_wet) = {
                let state = &self.objects[idx].state;
                let float_line = self
                    .definitions
                    .get(&self.objects[idx].definition_id)
                    .map(|definition| definition.float_line)
                    .unwrap_or(0);
                let offset = float_line
                    .saturating_mul(state.construction)
                    .checked_div(FULL_CON)
                    .unwrap_or(0);
                let probe = |y: i32| {
                    self.landscape
                        .as_ref()
                        .map(|landscape| landscape.is_liquid_at(state.position.x, y))
                        .unwrap_or(false)
                };
                (
                    probe(state.position.y + offset),
                    probe(state.position.y + offset - 2),
                )
            };
            let walk_rotation = if matches!(procedure, ActionProcedure::Walk) {
                self.definitions
                    .get(&definition_id)
                    .map(|definition| definition.walk_rotation_seed(&self.objects[idx].state))
            } else {
                None
            };
            let walk_landscape = self.landscape.as_ref();
            // `lLimit = FIXED100(pPhysical->Float)` comes off the LIVE
            // physical with no zero special case, so Eke's airbike parks dead
            // still when `SetPhysical("Float", 0, 2)` dismounts its pilot
            // (EkeReloaded.c4d/Weapons.c4d/Airbike.c4d/Script.c:78-83,452-461).
            let native_float_bounds = self.uses_native_float_bounds(idx, physical.float);
            let object = &mut self.objects[idx];
            // DFA_SWIM and DFA_FLOAT never apply gravity (no DoGravity call,
            // C4Object.cpp:4920-4970/:5268-5287); the legacy halved-gravity
            // path remains only for explicitly opted-in FLOAT fixtures.
            let physical_skips_gravity = match procedure {
                ActionProcedure::Swim => physical.swim != 0,
                ActionProcedure::Float => native_float_bounds,
                _ => false,
            };
            // The C++ ExecAction default case (custom/DFA_NONE procedures,
            // C4Object.cpp:5426-5437): with an ActMap Attach the dirs are
            // zeroed and the object mobilized INSTEAD of gravity; without
            // one it just falls.
            let default_case_attach = !is_idle
                && action_attach != 0
                && matches!(
                    procedure,
                    ActionProcedure::Undefined | ActionProcedure::Other
                );
            // DoGravity (C4Object.cpp:4644-4664): the free-fall branch
            // skips C4D_StaticBack categories (:4662), and idle objects
            // only probe gravity while Mobile (C4Object.cpp:4708-4712) —
            // the Tick10 pulse in ExecMovement is what re-arms them.
            let gravity_gated_off = object.state.category & CATEGORY_STATIC_BACK != 0
                || (is_idle && !object.state.mobile)
                || default_case_attach;
            // DoGravity's float branch (C4Object.cpp:4644-4661): InLiquid
            // objects with a Def->Float line RISE instead of falling —
            // ydir -= FloatAccel clamped to FloatAccel*-10, xdir/rdir decay
            // by FloatFriction, and a float-line probe out of liquid zeroes
            // negative ydir (surface equilibrium). Free-fall is the ELSE.
            let float_line = self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.float_line)
                .unwrap_or(0);
            let floats = object.state.in_liquid && float_line != 0;
            if floats && !physical_skips_gravity && !gravity_gated_off {
                object.fixed_velocity.y -= math::FLOAT_ACCEL;
                let min_rise = C4Fixed::from_raw(-10 * math::FLOAT_ACCEL.val());
                if object.fixed_velocity.y < min_rise {
                    object.fixed_velocity.y = min_rise;
                }
                let friction = math::FLOAT_FRICTION;
                if object.fixed_velocity.x < -friction {
                    object.fixed_velocity.x += friction;
                } else if object.fixed_velocity.x > friction {
                    object.fixed_velocity.x -= friction;
                }
                if object.rotation_velocity < -friction {
                    object.rotation_velocity += friction;
                } else if object.rotation_velocity > friction {
                    object.rotation_velocity -= friction;
                }
                let probe_y = object.state.position.y - 1
                    + float_line
                        .saturating_mul(object.state.construction)
                        .checked_div(FULL_CON)
                        .unwrap_or(0)
                    - 1;
                let surfaced = self
                    .landscape
                    .as_ref()
                    .map(|landscape| !landscape.is_liquid_at(object.state.position.x, probe_y))
                    .unwrap_or(true);
                if surfaced && object.fixed_velocity.y < C4Fixed::ZERO {
                    object.fixed_velocity.y = C4Fixed::ZERO;
                }
            } else if !physical_skips_gravity && !gravity_gated_off {
                object.fixed_velocity.y += gravity_component;
            }
            if default_case_attach {
                object.fixed_velocity = FixedVec2::ZERO;
            }
            // Every C++ ExecAction procedure case ends in `Mobile = 1`
            // except DFA_LIFT (the TARGET mobilizes via Lift),
            // DFA_ATTACH, DFA_CONNECT and the no-Attach default case
            // (C4Object.cpp:4791-5437).
            let procedure_mobilizes = default_case_attach
                || matches!(
                    procedure,
                    ActionProcedure::Walk
                        | ActionProcedure::Kneel
                        | ActionProcedure::Scale
                        | ActionProcedure::Hang
                        | ActionProcedure::Flight
                        | ActionProcedure::Dig
                        | ActionProcedure::Swim
                        | ActionProcedure::Throw
                        | ActionProcedure::Bridge
                        | ActionProcedure::Build
                        | ActionProcedure::Push
                        | ActionProcedure::Pull
                        | ActionProcedure::Chop
                        | ActionProcedure::Fight
                        | ActionProcedure::Float
                );
            if procedure_mobilizes {
                object.state.mobile = true;
            }
            // C++ wind reaches only PXS and particles via GBackWind
            // (C4Wrappers.h:189-192) — object motion never reads it.
            if procedure.locks_vertical_velocity() {
                object.fixed_velocity.y = C4Fixed::ZERO;
            }
            let mut pending_direction = None;
            match procedure {
                ActionProcedure::Float if native_float_bounds => {
                    apply_float_physical_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        math::fixed100(physical.float),
                    );
                }
                ActionProcedure::Float => {
                    apply_float_command_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        movement_profile,
                    );
                }
                // After the contained Tick10 arm above, DFA_FLIGHT's
                // remaining steering is gravity + Mobile only
                // (C4Object.cpp:4893-4904): ComDir never steers a flier.
                ActionProcedure::Flight => {}
                ActionProcedure::Swim => {
                    if physical.swim != 0 {
                        pending_direction = apply_swim_physical_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            math::val_by_physical(160, physical.swim),
                        );
                    } else {
                        apply_swim_command_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            movement_profile,
                            gravity_component,
                        );
                    }
                    // Out-of-liquid checks (C4Object.cpp:4946-4960): a
                    // swimmer whose InLiquid dropped either paddles back
                    // down (liquid just below the float line) or free-falls
                    // into Walk (ObjectActionWalk, C4ObjectCom.cpp:34-39 —
                    // the early `return` skips the rest of ExecAction).
                    // The surface bound zeroes upward ydir once the float
                    // line clears the liquid.
                    if swim_probes_available {
                        if !object.state.in_liquid {
                            if swim_below_wet {
                                object.fixed_velocity.y = math::SWIM_ACCEL;
                            } else {
                                swim_walk_transition = true;
                            }
                        }
                        if !swim_walk_transition
                            && !swim_surface_wet
                            && object.fixed_velocity.y < C4Fixed::ZERO
                        {
                            object.fixed_velocity.y = C4Fixed::ZERO;
                        }
                    }
                }
                ActionProcedure::Walk => {
                    if physical.walk != 0 {
                        apply_walk_physical_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            math::val_by_physical(280, physical.walk),
                        );
                    } else {
                        apply_walk_command_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            movement_profile,
                        );
                    }
                    if let Some(seed) = walk_rotation {
                        let live_attach_vtx_x = usize::try_from(seed.attach.vtx)
                            .ok()
                            .and_then(|vtx| object.state.vertices.get(vtx))
                            .map(|vertex| vertex.x)
                            .unwrap_or(0);
                        // Internal DFA_WALK gate (C4Object.cpp:4817-4821):
                        // unlike FnAdjustWalkRotation, this does not inspect
                        // t_attach. Its false branch always stops rotation.
                        object.rotation_velocity = if seed.rotateable != 0
                            && seed.attach.mat_valid
                            && (object.fixed_velocity.x != C4Fixed::ZERO
                                || seed.def_attach_vtx_x != 0)
                        {
                            calculate_walk_rotation_velocity(
                                object.state.rotation,
                                seed.attach,
                                seed.def_attach_vtx_x,
                                live_attach_vtx_x,
                                20,
                                20,
                                100,
                                |x, y| {
                                    walk_landscape
                                        .map(|landscape| landscape.is_solid_at(x, y))
                                        .unwrap_or(false)
                                },
                            )
                        } else {
                            C4Fixed::ZERO
                        };
                    }
                }
                ActionProcedure::Scale => {
                    if physical.scale != 0 {
                        apply_scale_physical_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            math::val_by_physical(200, physical.scale),
                            object.state.direction,
                        );
                    } else {
                        apply_scale_command_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            movement_profile,
                            object.state.direction,
                        );
                    }
                }
                ActionProcedure::Hang => {
                    pending_direction = if physical.hangle != 0 {
                        apply_hangle_physical_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            math::val_by_physical(160, physical.hangle),
                            object.state.direction,
                        )
                    } else {
                        apply_hangle_command_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            movement_profile,
                            object.state.direction,
                        )
                    };
                }
                ActionProcedure::Dig => {
                    pending_direction = if physical.dig != 0 {
                        apply_dig_physical_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            math::val_by_physical(125, physical.dig),
                            object.state.direction,
                        )
                    } else {
                        apply_dig_command_movement(
                            &mut object.fixed_velocity,
                            command_direction,
                            movement_profile,
                            object.state.direction,
                        )
                    };
                }
                ActionProcedure::Push => {
                    if !push_handled {
                        // If push was not handled earlier (shouldn't happen), ensure velocities stay zeroed.
                        object.fixed_velocity = FixedVec2::ZERO;
                    }
                }
                ActionProcedure::Pull if !pull_handled => {
                    object.fixed_velocity = FixedVec2::ZERO;
                }
                _ => {}
            }
            match procedure {
                ActionProcedure::Bridge
                | ActionProcedure::Build
                | ActionProcedure::Attach
                | ActionProcedure::Throw
                | ActionProcedure::Connect
                | ActionProcedure::Chop => {
                    object.fixed_velocity = FixedVec2::ZERO;
                }
                _ => {}
            }
            // DFA_FLIGHT is only DoGravity + Mobile in C++, and DFA_FLOAT
            // applies its own `FIXED100(pPhysical->Float)` bounds
            // (C4Object.cpp:4893-4904,5291-5310). Neither is subject to the
            // synthetic PhysicsSettings terminal-speed bounds. Keep those
            // bounds for FLOAT fixtures that explicitly use the additive
            // MovementProfile convenience path.
            let native_float = matches!(procedure, ActionProcedure::Float) && native_float_bounds;
            if !matches!(procedure, ActionProcedure::Flight | ActionProcedure::Lift)
                && !native_float
            {
                self.physics
                    .clamp_fixed_velocity(&mut object.fixed_velocity);
            }
            object.refresh_velocity_from_fixed();
            match procedure {
                // WALK/HANGLE/DIG/SWIM all call C4Object::SetDir from the
                // raw C4Fixed xdir sign (C4Object.cpp:4802-4805,
                // 4886-4887, 4933, 4980-4981). Defer until after this borrow
                // so TurnAction runs through the full SetAction path.
                ActionProcedure::Walk
                | ActionProcedure::Hang
                | ActionProcedure::Dig
                | ActionProcedure::Swim => {
                    exec_set_direction = pending_direction.or_else(|| {
                        if object.fixed_velocity.x < C4Fixed::ZERO {
                            Some(Direction::Left)
                        } else if object.fixed_velocity.x > C4Fixed::ZERO {
                            Some(Direction::Right)
                        } else {
                            None
                        }
                    });
                }
                ActionProcedure::Bridge => {
                    // DFA_BRIDGE faces along its horizontal ComDir after
                    // DoBridge succeeds (C4Object.cpp:5000-5004).
                    exec_set_direction = match command_direction {
                        CommandDirection::Left | CommandDirection::UpLeft => Some(Direction::Left),
                        CommandDirection::Right | CommandDirection::UpRight => {
                            Some(Direction::Right)
                        }
                        _ => None,
                    };
                    object.state.mobile = true;
                }
                _ => {}
            }
        }

        if let Some(direction) = exec_set_direction {
            self.set_exec_action_direction(idx, &definition_id, direction)?;
        }

        // Free-fall swim exit: ObjectActionWalk (SetActionByName("Walk"),
        // xdir = ydir = 0, C4ObjectCom.cpp:34-39); the DFA_SWIM case
        // `return`s so the phase advance is skipped this frame
        // (C4Object.cpp:4956).
        if swim_walk_transition {
            if self.action_with_calls(idx, &definition_id, "Walk")? {
                let object = &mut self.objects[idx];
                object.fixed_velocity = FixedVec2::ZERO;
                object.state.velocity = Vector2::ZERO;
            }
            self.objects[idx].swim_exit_this_frame = true;
            return Ok(true);
        }
        Ok(false)
    }
}
