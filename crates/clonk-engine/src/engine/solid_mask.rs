//! `impl Engine` — solid masks, OCF, contents links and object entry.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

#[cfg(test)]
thread_local! {
    static SOLID_MASK_MOVEMENT_CANDIDATE_VISITS: Cell<usize> = const { Cell::new(0) };
}

impl Engine {
    fn solid_mask_pixels_for_object(
        &self,
        object: &Object,
        mask: DefinitionTargetRect,
    ) -> SolidMaskPixels {
        let (graphics_definition, graphics_name) = object
            .state
            .base_graphics
            .as_ref()
            .map(|graphics| {
                (
                    graphics.definition.as_str(),
                    graphics.graphics_name.as_deref(),
                )
            })
            .unwrap_or((object.definition_id.as_str(), None));
        self.definitions
            .get(graphics_definition)
            .map(|definition| definition.solid_mask_pixels_for_rect(mask, graphics_name))
            .unwrap_or(SolidMaskPixels::OutOfBounds)
    }

    /// The object's effective solid mask spec when eligible
    /// (C4Object::UpdateSolidMask gates: mask enabled, FullCon, not
    /// contained, no rotation, C4Object.cpp:5652-5656).
    pub(crate) fn solid_mask_spec(&self, index: usize) -> Option<SolidMaskSpec> {
        let object = self.objects.get(index)?;
        self.solid_mask_spec_for_object(object)
    }

    fn solid_mask_spec_for_object(&self, object: &Object) -> Option<SolidMaskSpec> {
        if object.destroyed
            || matches!(object.state.status, ObjectStatus::Deleted)
            || object.state.container.is_some()
            || object.state.construction < FULL_CON
        {
            return None;
        }
        let definition = self.definitions.get(&object.definition_id)?;
        // Rotation only blocks the mask without Def->RotatedSolidmasks
        // (`if (!r || Def->RotatedSolidmasks)`, C4Object.cpp:5655).
        if object.state.rotation != 0 && !definition.rotated_solid_masks() {
            return None;
        }
        let mask = match object.state.solid_mask_override {
            Some(rect) if rect.width <= 0 || rect.height <= 0 => return None,
            Some(rect) => rect,
            None => definition.solid_mask()?,
        };
        let pixels = match self.solid_mask_pixels_for_object(object, mask) {
            SolidMaskPixels::OutOfBounds => return None,
            SolidMaskPixels::Rectangle => None,
            SolidMaskPixels::Alpha(pixels) => Some(Arc::clone(&pixels)),
        };
        let shape = definition.shape_rect().unwrap_or_default();
        Some(SolidMaskSpec {
            mask,
            pixels,
            shape_x: shape.x,
            shape_y: shape.y,
            rotation: object.state.rotation,
        })
    }

    /// C4SolidMask::Put (regular, unrotated, C4SolidMask.cpp:24-107):
    /// clip the mask to the landscape, save the background bytes, write
    /// MCVehic. No-op when already put or ineligible.
    pub(crate) fn put_solid_mask(&mut self, index: usize) {
        if self.solid_mask_staging.defer_solid_mask_updates {
            return;
        }
        if index >= self.objects.len()
            || self.objects[index].solid_mask_bake.is_some()
            || self.objects[index].solid_mask_empty_put
        {
            return;
        }
        let Some(spec) = self.solid_mask_spec(index) else {
            // UpdateSolidMask deletes pSolidMaskData whenever its eligibility
            // gates fail (C4Object.cpp:5688-5690).
            self.objects[index].solid_mask_instance_sequence = None;
            return;
        };
        self.note_solid_mask_host_state_changed();
        let instance_sequence = match self.objects[index].solid_mask_instance_sequence {
            Some(sequence) => {
                self.solid_mask_staging.next_solid_mask_instance_sequence = self
                    .solid_mask_staging
                    .next_solid_mask_instance_sequence
                    .max(
                        sequence
                            .checked_add(1)
                            .expect("C4SolidMask instance sequence overflow"),
                    );
                sequence
            }
            None => {
                let sequence = self.solid_mask_staging.next_solid_mask_instance_sequence;
                self.solid_mask_staging.next_solid_mask_instance_sequence = self
                    .solid_mask_staging
                    .next_solid_mask_instance_sequence
                    .checked_add(1)
                    .expect("C4SolidMask instance sequence overflow");
                self.objects[index].solid_mask_instance_sequence = Some(sequence);
                sequence
            }
        };
        let position = self.objects[index].state.position;
        self.put_solid_mask_with_spec(index, spec, position, instance_sequence);
    }

    /// Raster half of C4SolidMask::Put. Host-operation replay supplies the
    /// call-time spec and position because later outcome channels may have
    /// already advanced the object's final state.
    fn put_solid_mask_with_spec(
        &mut self,
        index: usize,
        spec: SolidMaskSpec,
        position: Vector2,
        instance_sequence: u64,
    ) {
        self.note_solid_mask_host_state_changed();
        if index >= self.objects.len()
            || self.objects[index].solid_mask_bake.is_some()
            || self.objects[index].solid_mask_empty_put
        {
            return;
        }
        let Some(vehicle) = self
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.grid_vehicle_byte())
        else {
            return;
        };
        let Some((grid_width, grid_height)) = self
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.grid_dimensions())
        else {
            return;
        };
        if spec.rotation != 0 {
            self.put_solid_mask_rotated(
                index,
                vehicle,
                spec,
                position,
                (grid_width, grid_height),
                instance_sequence,
            );
            return;
        }
        let SolidMaskSpec {
            mask,
            pixels,
            shape_x,
            shape_y,
            ..
        } = spec;
        let ox = position.x + shape_x + mask.target_x;
        let oy = position.y + shape_y + mask.target_y;
        let mut rect_x = ox;
        let mut tx = 0;
        if rect_x < 0 {
            tx = -rect_x;
            rect_x = 0;
        }
        let mut rect_y = oy;
        let mut ty = 0;
        if rect_y < 0 {
            ty = -rect_y;
            rect_y = 0;
        }
        let width = (ox + mask.width).min(grid_width) - rect_x;
        let height = (oy + mask.height).min(grid_height) - rect_y;
        if width <= 0 || height <= 0 {
            // Native stores MaskPut=true even when Wdt/Hgt are zero or
            // negative; the raster loops are empty, but attachment restore
            // still belongs to this successful regular Put.
            self.objects[index].solid_mask_empty_put = true;
            return;
        }
        let mut bake = SolidMaskBake {
            instance_sequence,
            x: rect_x,
            y: rect_y,
            width,
            height,
            tx,
            ty,
            mask_width: mask.width,
            pixels,
            buffer: vec![vehicle; (width * height) as usize],
            rotated: None,
        };
        let landscape = self.landscape.as_mut().expect("grid mode checked");
        let mut buffer = std::mem::take(&mut bake.buffer);
        let writes = (0..height).flat_map(|cy| {
            let bake = &bake;
            (0..width).filter_map(move |cx| {
                bake.mask_set(tx + cx, ty + cy)
                    .then_some(crate::landscape::MaskWrite::set(
                        rect_x + cx,
                        rect_y + cy,
                        vehicle,
                        (cy * width + cx) as usize,
                    ))
            })
        });
        landscape.grid_write_mask_bytes(writes, |result, _view| {
            // Regular put stores the pixel even when it is already MCVehic
            // (C4SolidMask.cpp:92-96) — it just will not be used for restore.
            buffer[result.tag] = result.old.unwrap_or(0);
        });
        bake.buffer = buffer;
        self.objects[index].solid_mask_bake = Some(bake);
    }

    /// The rotated branch of C4SolidMask::Put (C4SolidMask.cpp:108-174):
    /// clip the MatBuffPitch square around the rotated extent to the
    /// landscape, then inverse-rotate every buffer cell back into the
    /// mask with the C4Fixed matrix accumulation. Reached only with
    /// Def->RotatedSolidmasks (C4Object.cpp:5655).
    fn put_solid_mask_rotated(
        &mut self,
        index: usize,
        vehicle: u8,
        spec: SolidMaskSpec,
        position: Vector2,
        (grid_width, grid_height): (i32, i32),
        instance_sequence: u64,
    ) {
        let SolidMaskSpec {
            mask,
            pixels,
            shape_x,
            shape_y,
            rotation,
        } = spec;
        // MatBuffPitch = int(sqrt(Wdt^2+Hgt^2)) + 1 (ctor,
        // C4SolidMask.cpp:415): f64 sqrt of an exact integer is correctly
        // rounded on both sides, and `as i32` truncates like the C++
        // static_cast.
        let mat_buff_pitch =
            f64::from(mask.width * mask.width + mask.height * mask.height).sqrt() as i32 + 1;
        // Rotation matrix for -MaskPutRotation (C4SolidMask.cpp:111-112).
        let negated = itofix(-rotation);
        let ma1 = negated.cos_deg();
        let ma2 = -negated.sin_deg();
        let mb1 = negated.sin_deg();
        let mb2 = negated.cos_deg();
        // Upper-left corner of the landscape copy rect
        // (C4SolidMask.cpp:114-117): rotate the mask center, then back
        // off half the enlarged square.
        let center_x = shape_x + mask.target_x + mask.width / 2;
        let center_y = shape_y + mask.target_y + mask.height / 2;
        let xstart = position.x + fixtoi(ma1 * itofix(center_x) - ma2 * itofix(center_y))
            - mat_buff_pitch / 2;
        let ystart = position.y + fixtoi(-mb1 * itofix(center_x) + mb2 * itofix(center_y))
            - mat_buff_pitch / 2;
        // Store put rect (C4SolidMask.cpp:119-128).
        let mut rect_x = xstart;
        let mut tx = 0;
        if rect_x < 0 {
            tx = -rect_x;
            rect_x = 0;
        }
        let mut rect_y = ystart;
        let mut ty = 0;
        if rect_y < 0 {
            ty = -rect_y;
            rect_y = 0;
        }
        let width = (xstart + mat_buff_pitch).min(grid_width) - rect_x;
        let height = (ystart + mat_buff_pitch).min(grid_height) - rect_y;
        if width <= 0 || height <= 0 {
            self.objects[index].solid_mask_empty_put = true;
            return;
        }
        let mut bake = SolidMaskBake {
            instance_sequence,
            x: rect_x,
            y: rect_y,
            width,
            height,
            tx,
            ty,
            mask_width: mask.width,
            pixels,
            buffer: vec![vehicle; (width * height) as usize],
            rotated: Some(RotatedBake {
                rotation,
                mat_buff_pitch,
                mask_height: mask.height,
            }),
        };
        // Go through the clipping rect with the EXACT C4Fixed matrix
        // accumulation (C4SolidMask.cpp:130-173). x0/y0 are integer
        // fixed values, so every product and running sum below is an
        // exact multiple of the matrix entries — bit-identical to C++.
        let x0 = itofix(tx - mat_buff_pitch / 2);
        let y0 = itofix(ty - mat_buff_pitch / 2);
        let landscape = self.landscape.as_mut().expect("grid mode checked");
        let mut writes = Vec::new();
        let mut ya = y0 * ma2;
        let mut yb = y0 * mb2;
        for cy in 0..height {
            let mut xa = x0 * ma1;
            let mut xb = x0 * mb1;
            for cx in 0..width {
                // Position in the solidmask buffer (C4SolidMask.cpp:147-148).
                let mask_x = fixtoi(xa + ya) + mask.width / 2;
                let mask_y = fixtoi(xb + yb) + mask.height / 2;
                if mask_x >= 0
                    && mask_y >= 0
                    && mask_x < mask.width
                    && mask_y < mask.height
                    && bake.mask_pixel(mask_x, mask_y)
                {
                    writes.push(crate::landscape::MaskWrite::set(
                        rect_x + cx,
                        rect_y + cy,
                        vehicle,
                        (cy * width + cx) as usize,
                    ));
                }
                // Cells the rotated mask misses keep the MCVehic marker
                // the buffer was initialized with (C4SolidMask.cpp:165-167).
                xa += ma1;
                xb += mb1;
            }
            ya += ma2;
            yb += mb2;
        }
        landscape.grid_write_mask_bytes(writes, |result, _view| {
            // Rotated put also stores an already-MCVehic pixel
            // (C4SolidMask.cpp:156-160).
            bake.buffer[result.tag] = result.old.unwrap_or(0);
        });
        self.objects[index].solid_mask_bake = Some(bake);
    }

    /// Ordinary C4SolidMask::Remove callers do not carry attached objects
    /// (SetRotation/DoCon/Enter/Exit/destruction all pass false in C++).
    #[doc(hidden)]
    pub fn remove_solid_mask(&mut self, index: usize) {
        if self.solid_mask_staging.defer_solid_mask_updates {
            return;
        }
        self.remove_solid_mask_impl(index, true, false);
        if index < self.objects.len() && self.solid_mask_spec(index).is_none() {
            self.objects[index].solid_mask_instance_sequence = None;
        }
    }

    /// C4Object::DoMotion removes its mask with fBackupAttachment=true
    /// (C4Movement.cpp:121-126); the later Put restores these riders.
    pub(crate) fn remove_solid_mask_for_movement(
        &mut self,
        index: usize,
    ) -> Option<SolidMaskAttachmentBackup> {
        self.remove_solid_mask_impl(index, true, true)
    }

    /// C4SolidMask::Remove (C4SolidMask.cpp:233-305): restore the saved
    /// bytes (only where the pixel is STILL MCVehic — landscape changes
    /// under the mask win), re-put overlapping masks, then optionally
    /// remember objects attached to this exact mask.
    fn remove_solid_mask_impl(
        &mut self,
        index: usize,
        cause_instability: bool,
        backup_attachments: bool,
    ) -> Option<SolidMaskAttachmentBackup> {
        if index >= self.objects.len() {
            return None;
        }
        if self.objects[index].solid_mask_bake.is_some()
            || self.objects[index].solid_mask_empty_put
            || self.objects[index].solid_mask_instance_sequence.is_some()
        {
            self.note_solid_mask_host_state_changed();
        }
        let Some(landscape) = self.landscape.as_mut() else {
            self.objects[index].solid_mask_bake.take();
            self.objects[index].solid_mask_empty_put = false;
            return None;
        };
        let definitions = &self.definitions;
        let materials = &self.materials;
        let mass_movers = &mut self.mass_movers;
        let sectors = self.sectors.as_ref();
        let (before, tail) = self.objects.split_at_mut(index);
        let (mover, after) = tail.split_first_mut().expect("index checked");
        Self::remove_solid_mask_from_fields(
            mover,
            before,
            after,
            definitions,
            materials,
            mass_movers,
            landscape,
            sectors,
            cause_instability,
            backup_attachments,
        )
    }

    /// Field-split C4SolidMask::Remove used both by ordinary lifecycle
    /// calls and by DoMotion's one-shot pre-move removal. Keeping the mover
    /// separate lets the per-pixel walker remove its bake without borrowing
    /// the whole Engine from inside the movement callback.
    #[allow(clippy::too_many_arguments)]
    fn remove_solid_mask_from_fields(
        mover: &mut Object,
        before: &mut [Object],
        after: &mut [Object],
        definitions: &rustc_hash::FxHashMap<DefinitionId, Definition>,
        materials: &MaterialSet,
        mass_movers: &mut MassMoverSet,
        landscape: &mut Landscape,
        sectors: Option<&SectorMap>,
        cause_instability: bool,
        backup_attachments: bool,
    ) -> Option<SolidMaskAttachmentBackup> {
        let instance_sequence = mover.solid_mask_instance_sequence;
        let empty_put = std::mem::take(&mut mover.solid_mask_empty_put);
        let Some(bake) = mover.solid_mask_bake.take() else {
            return (empty_put && backup_attachments).then(|| SolidMaskAttachmentBackup {
                instance_sequence,
                removal_position: mover.state.position,
                object_ids: Vec::new(),
            });
        };
        let vehicle = landscape.grid_vehicle_byte()?;
        let writes = (0..bake.height).flat_map(|cy| {
            let bake = &bake;
            (0..bake.width).filter_map(move |cx| {
                let saved = bake.buffer[(cy * bake.width + cx) as usize];
                (saved != vehicle).then_some(crate::landscape::MaskWrite::replace(
                    bake.x + cx,
                    bake.y + cy,
                    vehicle,
                    saved,
                    (),
                ))
            })
        });
        landscape.grid_write_mask_bytes(writes, |result, view| {
            // C++ probes every mask-used pixel when requested, whether or not
            // the restore happened (C4SolidMask.cpp:244-257).
            if cause_instability {
                mass_movers
                    .check_instability_range_for_landscape(view, materials, result.x, result.y);
            }
        });
        // Re-put overlapping masks: doubled MCVehic pixels were just
        // removed inside the freed rect. C++ walks the live instance list
        // Last->Prev, i.e. newest construction first (C4SolidMask.cpp:
        // 263-274,395-400).
        let mut overlapping_masks = before
            .iter_mut()
            .chain(after.iter_mut())
            .filter(|other| {
                other
                    .solid_mask_bake
                    .as_ref()
                    .is_some_and(|other_bake| other_bake.overlaps(&bake))
            })
            .collect::<Vec<_>>();
        overlapping_masks.sort_unstable_by(|left, right| {
            let left = left
                .solid_mask_bake
                .as_ref()
                .map_or(0, |bake| bake.instance_sequence);
            let right = right
                .solid_mask_bake
                .as_ref()
                .map_or(0, |bake| bake.instance_sequence);
            right.cmp(&left)
        });
        for other in overlapping_masks {
            let Some(other_bake) = other.solid_mask_bake.as_mut() else {
                continue;
            };
            other_bake.reput_after_removal(&bake, landscape, vehicle);
        }

        if !backup_attachments {
            return None;
        }
        // C4SolidMask::Remove enumerates every ObjectShapes link from a
        // C4LArea expanded one pixel around MaskPutRect. This is sector
        // row-major order (outside last), with each sector list retaining
        // main-list order; it is neither the backing object-vec order nor a
        // deduplicated find (C4SolidMask.cpp:282-305; C4Sector.cpp:264-277).
        let candidate_ids: Vec<ObjectId> = sectors.map_or_else(
            || {
                before
                    .iter()
                    .chain(after.iter())
                    .map(|object| object.id)
                    .collect::<Vec<_>>()
            },
            |sectors| {
                let area = sectors.area(DefinitionRect::new(
                    bake.x.saturating_sub(1),
                    bake.y.saturating_sub(1),
                    bake.width.saturating_add(2),
                    bake.height.saturating_add(2),
                ));
                sectors
                    .shape_id_lists_in_area(&area)
                    .into_iter()
                    .flatten()
                    .collect()
            },
        );
        let candidates = before
            .iter()
            .chain(after.iter())
            .map(|object| (object.id, object))
            .collect::<HashMap<_, _>>();
        let object_ids = candidate_ids
            .into_iter()
            // The lookup retains every repeated sector link in the input;
            // only the id-to-object resolution is indexed.
            .filter_map(|candidate_id| candidates.get(&candidate_id).copied())
            .filter(|object| {
                if object.state.status != ObjectStatus::Normal
                    || object.state.category & (CATEGORY_STATIC_BACK | CATEGORY_STRUCTURE) != 0
                    || object.state.container.is_some()
                    || (object.state.category & CATEGORY_VEHICLE != 0
                        && object.state.ocf & ocf::GRAB == 0)
                {
                    return false;
                }
                let Some(definition) = definitions.get(&object.definition_id) else {
                    return false;
                };
                definition
                    .action_library()
                    .is_idle_state(&object.state.action)
                    || definition.action_library().procedure_for_entry(
                        &object.state.action.name,
                        object.state.action.act_map_index,
                    ) != ActionProcedure::Float
            })
            .filter(|object| {
                !shape_contact_check(
                    &object.state.vertices,
                    object.state.position,
                    landscape,
                    materials,
                    &[],
                    None,
                    object.state.contact_density,
                )
                .is_contact()
            })
            .filter(|object| Self::object_contacts_solid_mask_bake(object, &bake, vehicle))
            .map(|object| object.id)
            .collect();
        Some(SolidMaskAttachmentBackup {
            instance_sequence,
            removal_position: mover.state.position,
            object_ids,
        })
    }

    /// The solid-mask half of `C4GameObjects::Synchronize`: first remove all
    /// put masks with `fCauseInstability=false`, preserving each Remove's
    /// overlapping-mask re-put chain, then call UpdateSolidMask for every
    /// active object in the same post-resort master-list order.
    pub(crate) fn synchronize_solid_masks(&mut self) {
        let master_order = self
            .execution
            .exec_list
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        for &id in &master_order {
            let Some(index) = self.find_object_index(id) else {
                continue;
            };
            if self.objects[index].destroyed || !self.objects[index].state.status.is_active() {
                continue;
            }
            if self.objects[index].solid_mask_bake.is_some()
                || self.objects[index].solid_mask_empty_put
            {
                self.remove_solid_mask_impl(index, false, false);
            }
        }
        for id in master_order {
            let Some(index) = self.find_object_index(id) else {
                continue;
            };
            if self.objects[index].destroyed || !self.objects[index].state.status.is_active() {
                continue;
            }
            self.update_solid_mask(index);
        }
    }

    /// C4Object::IsMoveableBySolidMask (C4Object.h:434-440).
    fn object_is_moveable_by_solid_mask(&self, index: usize) -> bool {
        let Some(object) = self.objects.get(index) else {
            return false;
        };
        if object.state.status != ObjectStatus::Normal
            || object.state.category & (CATEGORY_STATIC_BACK | CATEGORY_STRUCTURE) != 0
            || object.state.container.is_some()
            || (object.state.category & CATEGORY_VEHICLE != 0 && object.state.ocf & ocf::GRAB == 0)
        {
            return false;
        }
        let Some(definition) = self.definitions.get(&object.definition_id) else {
            return false;
        };
        definition
            .action_library()
            .is_idle_state(&object.state.action)
            || definition
                .action_library()
                .procedure_for_entry(&object.state.action.name, object.state.action.act_map_index)
                != ActionProcedure::Float
    }

    /// Raw C4Shape::CheckContact: no Contact* script callbacks. During mask
    /// backup the mover is already removed; during restore it has been put at
    /// its new position (C4SolidMask.cpp:286,185).
    pub(crate) fn object_shape_contacts_at(&self, index: usize, position: Vector2) -> bool {
        let Some(object) = self.objects.get(index) else {
            return true;
        };
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        let indices: Vec<usize> = (0..self.objects.len()).collect();
        let masks = self.solid_masks_for_movement(&indices);
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
        .is_contact()
    }

    fn object_contacts_solid_mask_bake(object: &Object, bake: &SolidMaskBake, vehicle: u8) -> bool {
        // C4SolidMask::DensityProvider returns C4M_Solid (50) for a mask
        // pixel, and C4Shape::GetVertexContact compares it inclusively to
        // the candidate shape's live ContactDensity.
        if CONTACT_DENSITY_SOLID < object.state.contact_density {
            return false;
        }
        let check_mask = object.frame_t_attach | CNAT_BOTTOM;
        object.state.vertices.iter().any(|vertex| {
            if vertex.cnat & CNAT_NO_COLLISION != 0 {
                return false;
            }
            let x = object.state.position.x + vertex.x;
            let y = object.state.position.y + vertex.y;
            (check_mask & CNAT_CENTER != 0 && bake.provides_attachment_density_at(vehicle, x, y))
                || (check_mask & CNAT_LEFT != 0
                    && bake.provides_attachment_density_at(vehicle, x - 1, y))
                || (check_mask & CNAT_RIGHT != 0
                    && bake.provides_attachment_density_at(vehicle, x + 1, y))
                || (check_mask & CNAT_TOP != 0
                    && bake.provides_attachment_density_at(vehicle, x, y - 1))
                || (check_mask & CNAT_BOTTOM != 0
                    && bake.provides_attachment_density_at(vehicle, x, y + 1))
        })
    }

    /// C4SolidMask::Put(..., fRestoreAttachment=true), including the
    /// destination contact probe, once-per-frame guard, and MovePosition's
    /// recursive mask lifecycle for stacked carriers (C4SolidMask.cpp:178-195;
    /// C4Movement.cpp:547-556).
    pub(crate) fn restore_solid_mask_attachments(
        &mut self,
        mover_index: usize,
        backup: Option<SolidMaskAttachmentBackup>,
    ) {
        let Some(backup) = backup else {
            return;
        };
        let Some(mover) = self.objects.get(mover_index) else {
            return;
        };
        // Attached riders are owned by one concrete C4SolidMaskData object.
        // SetSolidMask, ChangeDef and graphics changes delete that instance;
        // a later Put of the replacement must not inherit the old list.
        if backup.instance_sequence.is_none()
            || mover.solid_mask_instance_sequence != backup.instance_sequence
        {
            return;
        }
        // C++ restores only from Put; a removed/ineligible mover clears the
        // backup without translating anything. A fully clipped regular Put
        // still sets MaskPut despite having no raster bake.
        if mover.solid_mask_bake.is_none() && !mover.solid_mask_empty_put {
            return;
        }
        let dx = mover.state.position.x - backup.removal_position.x;
        let dy = mover.state.position.y - backup.removal_position.y;
        if dx == 0 && dy == 0 {
            return;
        }
        let frame = self.frame as i32;

        for object_id in backup.object_ids {
            let Some(index) = self.find_object_index(object_id) else {
                continue;
            };
            if !self.object_is_moveable_by_solid_mask(index) {
                continue;
            }
            let old_position = self.objects[index].state.position;
            let new_position = Vector2::new(old_position.x + dx, old_position.y + dy);
            if self.object_shape_contacts_at(index, new_position)
                || self.objects[index].last_attach_movement_frame == frame
            {
                continue;
            }

            self.objects[index].last_attach_movement_frame = frame;
            let nested_backup = self.remove_solid_mask_for_movement(index);
            {
                let object = &mut self.objects[index];
                object.state.position = new_position;
                object.fixed_position.x += itofix(dx);
                object.fixed_position.y += itofix(dy);
            }
            self.update_sector_for_index(index);
            self.update_solid_mask(index);
            self.restore_solid_mask_attachments(index, nested_backup);
        }
    }

    /// C4Object::UpdateSolidMask (C4Object.cpp:5644-5670): remove, then
    /// re-put when (still) eligible.
    #[doc(hidden)]
    pub fn update_solid_mask(&mut self, index: usize) {
        if self.solid_mask_staging.defer_solid_mask_updates {
            return;
        }
        if self.objects.get(index).is_some_and(|object| {
            object.solid_mask_bake.is_some()
                || object.solid_mask_empty_put
                || object.solid_mask_instance_sequence.is_some()
        }) || self.solid_mask_spec(index).is_some()
        {
            self.note_solid_mask_host_state_changed();
        }
        self.remove_solid_mask(index);
        self.put_solid_mask(index);
    }

    /// Apply the exact synchronous UpdateSolidMask history captured by a
    /// callback host. Each Put denotes the complete remove/re-put call; its
    /// call-time geometry is intentionally independent of the object's final
    /// copy-out state.
    fn replay_host_solid_mask_operations(&mut self, operations: Vec<HostSolidMaskOperation>) {
        for operation in operations {
            if let HostSolidMaskOperation::Landscape { operation } = operation {
                self.apply_landscape_operations(vec![operation]);
                continue;
            }
            let object_id = match &operation {
                HostSolidMaskOperation::Remove { object_id }
                | HostSolidMaskOperation::Put { object_id, .. } => *object_id,
                HostSolidMaskOperation::Landscape { .. } => unreachable!(),
            };
            let Some(index) = self.find_object_index(object_id) else {
                continue;
            };

            self.remove_solid_mask_impl(index, true, false);
            match operation {
                HostSolidMaskOperation::Remove { .. } => {
                    self.objects[index].solid_mask_instance_sequence = None;
                }
                HostSolidMaskOperation::Put {
                    spec,
                    position,
                    instance_sequence,
                    ..
                } => {
                    self.objects[index].solid_mask_instance_sequence = Some(instance_sequence);
                    self.solid_mask_staging.next_solid_mask_instance_sequence = self
                        .solid_mask_staging
                        .next_solid_mask_instance_sequence
                        .max(
                            instance_sequence
                                .checked_add(1)
                                .expect("C4SolidMask instance sequence overflow"),
                        );
                    self.put_solid_mask_with_spec(index, spec, position, instance_sequence);
                }
                HostSolidMaskOperation::Landscape { .. } => unreachable!(),
            }
        }
    }

    /// Join a mask-operation stream to the active chronological fold. The
    /// outermost participant owns replay; nested participants only append.
    pub(crate) fn stage_host_solid_mask_operations(
        &mut self,
        operations: Vec<HostSolidMaskOperation>,
        host_raster_preview: Option<compat::HostRasterPreview>,
    ) -> bool {
        if operations.is_empty() {
            debug_assert!(host_raster_preview.is_none());
            return false;
        }
        let outermost = !self.solid_mask_staging.defer_solid_mask_updates;
        if outermost {
            debug_assert!(self
                .solid_mask_staging
                .deferred_solid_mask_operations
                .is_empty());
            self.solid_mask_staging.defer_solid_mask_updates = true;
        }
        self.solid_mask_staging
            .deferred_solid_mask_operations
            .extend(operations);
        if host_raster_preview.is_some() {
            self.solid_mask_staging.deferred_host_raster_preview = host_raster_preview;
        }
        outermost
    }

    /// Initial DoCon's UpdateFace(true) puts a completed newborn's mask
    /// before Completion/Initialize, while Rust still owns that object as a
    /// local SpawnConfig materialization. Add the put to the chronological
    /// host stream now so those callbacks query the live raster; replay waits
    /// until the object has joined `self.objects` (C4Object.cpp:1428-1511,
    /// 5655-5690).
    pub(crate) fn stage_pending_spawn_solid_mask(&mut self, object: &mut Object) {
        let Some(spec) = self.solid_mask_spec_for_object(object) else {
            return;
        };
        self.solid_mask_staging.next_solid_mask_instance_sequence = self
            .solid_mask_staging
            .deferred_solid_mask_operations
            .iter()
            .filter_map(|operation| match operation {
                HostSolidMaskOperation::Put {
                    instance_sequence, ..
                } => instance_sequence.checked_add(1),
                HostSolidMaskOperation::Remove { .. }
                | HostSolidMaskOperation::Landscape { .. } => None,
            })
            .fold(
                self.solid_mask_staging.next_solid_mask_instance_sequence,
                u64::max,
            );
        let instance_sequence = object.solid_mask_instance_sequence.unwrap_or_else(|| {
            let sequence = self.solid_mask_staging.next_solid_mask_instance_sequence;
            self.solid_mask_staging.next_solid_mask_instance_sequence = self
                .solid_mask_staging
                .next_solid_mask_instance_sequence
                .checked_add(1)
                .expect("C4SolidMask instance sequence overflow");
            sequence
        });
        object.solid_mask_instance_sequence = Some(instance_sequence);
        self.solid_mask_staging.next_solid_mask_instance_sequence = self
            .solid_mask_staging
            .next_solid_mask_instance_sequence
            .max(
                instance_sequence
                    .checked_add(1)
                    .expect("C4SolidMask instance sequence overflow"),
            );
        self.note_solid_mask_host_state_changed();
        let operations = vec![HostSolidMaskOperation::Put {
            object_id: object.id,
            spec,
            position: object.state.position,
            instance_sequence,
        }];
        let mut world = self.host_world_context();
        world.preview_solid_mask_operations(&operations);
        let preview = world.host_raster_preview();
        self.stage_host_solid_mask_operations(operations, Some(preview));
    }

    /// A materialized spawn normally gets its first mask at the final
    /// UpdateSolidMask below. When a creation callback opened a deferred
    /// chronological fold using only foreign-object operations, that normal
    /// put is suppressed and must join the stream explicitly.
    pub(crate) fn stage_materialized_spawn_solid_mask(&mut self, index: usize) {
        if !self.solid_mask_staging.defer_solid_mask_updates
            || self.objects[index].solid_mask_bake.is_some()
            || self.objects[index].solid_mask_empty_put
            || self
                .solid_mask_staging
                .deferred_solid_mask_operations
                .iter()
                .any(|operation| {
                    matches!(operation,
                    HostSolidMaskOperation::Put { object_id, .. }
                    | HostSolidMaskOperation::Remove { object_id }
                    if *object_id == self.objects[index].id)
                })
        {
            return;
        }
        let Some(spec) = self.solid_mask_spec(index) else {
            return;
        };
        self.solid_mask_staging.next_solid_mask_instance_sequence = self
            .solid_mask_staging
            .deferred_solid_mask_operations
            .iter()
            .filter_map(|operation| match operation {
                HostSolidMaskOperation::Put {
                    instance_sequence, ..
                } => instance_sequence.checked_add(1),
                HostSolidMaskOperation::Remove { .. }
                | HostSolidMaskOperation::Landscape { .. } => None,
            })
            .fold(
                self.solid_mask_staging.next_solid_mask_instance_sequence,
                u64::max,
            );
        let instance_sequence = self.objects[index]
            .solid_mask_instance_sequence
            .unwrap_or_else(|| {
                let sequence = self.solid_mask_staging.next_solid_mask_instance_sequence;
                self.solid_mask_staging.next_solid_mask_instance_sequence = self
                    .solid_mask_staging
                    .next_solid_mask_instance_sequence
                    .checked_add(1)
                    .expect("C4SolidMask instance sequence overflow");
                self.objects[index].solid_mask_instance_sequence = Some(sequence);
                sequence
            });
        self.solid_mask_staging.next_solid_mask_instance_sequence = self
            .solid_mask_staging
            .next_solid_mask_instance_sequence
            .max(
                instance_sequence
                    .checked_add(1)
                    .expect("C4SolidMask instance sequence overflow"),
            );
        let operation = HostSolidMaskOperation::Put {
            object_id: self.objects[index].id,
            spec,
            position: self.objects[index].state.position,
            instance_sequence,
        };
        let mut world = self.host_world_context();
        world.preview_solid_mask_operations(std::slice::from_ref(&operation));
        self.solid_mask_staging.deferred_host_raster_preview = Some(world.host_raster_preview());
        self.solid_mask_staging
            .deferred_solid_mask_operations
            .push(operation);
    }

    /// Close a chronological fold and replay only when this caller opened
    /// the outermost scope. Replay precedes propagation of either success or
    /// failure because C++ retains mutations made before a script error.
    pub(crate) fn finish_host_solid_mask_operations<T>(
        &mut self,
        outermost: bool,
        result: Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if !outermost {
            return result;
        }
        self.solid_mask_staging.defer_solid_mask_updates = false;
        self.solid_mask_staging.deferred_host_raster_preview = None;
        let operations =
            std::mem::take(&mut self.solid_mask_staging.deferred_solid_mask_operations);
        self.replay_host_solid_mask_operations(operations);
        result
    }

    /// The rect overlay a live movement step checks against: the candidate
    /// scan feeding [`Self::solid_masks_for_movement`]. Movement derived this
    /// pair inline in three places; naming it keeps the candidate set and its
    /// only consumer together.
    pub(crate) fn live_movement_solid_masks(&self) -> Vec<SolidMaskRect> {
        // Ask the grid question before deriving candidates, not after:
        // solid_masks_for_movement answers an empty overlay for a grid world
        // (:966-969) whatever the scan finds, and that scan walks every object
        // against the definition table. Reading the same predicate in the same
        // `&self` borrow keeps the result identical.
        if self.solid_mask_grid_mode() {
            return Vec::new();
        }
        self.solid_masks_for_movement(&self.active_solid_mask_indices())
    }

    pub(crate) fn solid_masks_for_movement(
        &self,
        candidate_indices: &[usize],
    ) -> Vec<SolidMaskRect> {
        // Grid worlds bake masks into the plane (put_solid_mask) — the
        // rect overlay would double-apply.
        if self.solid_mask_grid_mode() {
            return Vec::new();
        }
        let mut masks = Vec::new();
        for &index in candidate_indices {
            #[cfg(test)]
            SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(|count| count.set(count.get() + 1));
            let Some(object) = self.objects.get(index) else {
                continue;
            };
            // Rotation blocks the overlay even with RotatedSolidmasks:
            // this rect model cannot express a rotated mask, and grid
            // worlds (all real content) take the bake path above.
            if object.destroyed
                || matches!(object.state.status, ObjectStatus::Deleted)
                || object.state.container.is_some()
                || object.state.construction < FULL_CON
                || object.state.rotation != 0
            {
                continue;
            }
            let Some(definition) = self.definitions.get(&object.definition_id) else {
                continue;
            };
            // The per-object SolidMask override wins (C4Object::SolidMask;
            // Objects.txt SolidMask= / FnSetSolidMask): a zero-area rect
            // means the mask is OFF (opened gates).
            let mask = match object.state.solid_mask_override {
                Some(rect) if rect.width <= 0 || rect.height <= 0 => continue,
                Some(rect) => rect,
                None => match definition.solid_mask() {
                    Some(mask) => mask,
                    None => continue,
                },
            };
            // The pixel decode follows the EFFECTIVE rect: an Objects.txt
            // override reads its own sprite region (C4SolidMask::Put uses
            // the object's SolidMask, not the def's).
            let mask_pixels = match self.solid_mask_pixels_for_object(object, mask) {
                SolidMaskPixels::OutOfBounds => continue,
                SolidMaskPixels::Rectangle => None,
                SolidMaskPixels::Alpha(pixels) => Some(Arc::clone(&pixels)),
            };
            let shape_offset = definition
                .shape_rect()
                .map(|shape| Vector2::new(shape.x, shape.y))
                .unwrap_or(Vector2::ZERO);
            let position = object.position_pixels();
            masks.push(SolidMaskRect {
                object_id: object.id,
                x: position.x + shape_offset.x + mask.target_x,
                y: position.y + shape_offset.y + mask.target_y,
                width: mask.width,
                height: mask.height,
                pixels: mask_pixels,
            });
        }
        masks
    }

    fn is_container_cycle(&self, object_id: ObjectId, container_id: ObjectId) -> bool {
        let mut current = Some(container_id);
        while let Some(id) = current {
            if id == object_id {
                return true;
            }
            current = self
                .objects
                .iter()
                .find(|object| object.id == id)
                .and_then(|object| object.state.container);
        }
        false
    }

    /// The cached mask, like every C++ reader (CrossCheck, FindObject
    /// criteria, host functions all consume `obj->OCF`).
    #[doc(hidden)]
    pub fn object_ocf_at_index(&self, index: usize) -> u32 {
        self.objects[index].state.ocf
    }

    /// The SetOCF/UpdateOCF analogue: recompute and store the cache.
    #[doc(hidden)]
    pub fn refresh_object_ocf(&mut self, index: usize) {
        // Both SetOCF and UpdateOCF update InMat before touching OCF
        // (C4Object.cpp:545-548,687-690). Contained objects inherit the
        // container's CACHE, not a fresh landscape sample; any nonzero
        // ClosedContainer shields them.
        let (container, position) = {
            let object = &self.objects[index];
            (object.state.container, object.state.position)
        };
        let in_mat = if let Some(container_id) = container {
            self.find_object_index(container_id)
                .and_then(|container_index| {
                    let container = &self.objects[container_index];
                    let closed = self
                        .definitions
                        .get(&container.definition_id)
                        .is_some_and(|definition| definition.closed_container() != 0);
                    (!closed).then_some(container.in_mat).flatten()
                })
        } else {
            self.landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(position.x, position.y))
        };
        self.objects[index].in_mat = in_mat;
        let ocf = self.compute_object_ocf(index);
        self.objects[index].state.ocf = ocf;
    }

    /// SetOCF for a NewObject value that Rust is still materializing outside
    /// `self.objects`. `linked` distinguishes Init (before Objects.Add) from
    /// the initial DoCon (after Objects.Add); neither phase has put the
    /// newborn's solid mask yet (C4Game.cpp:1115-1131; C4Object.cpp:198-217,
    /// 1428-1450).
    pub(crate) fn refresh_pending_object_ocf(&self, object: &mut Object, linked: bool) {
        object.in_mat = if let Some(container_id) = object.state.container {
            self.find_object_index(container_id)
                .and_then(|container_index| {
                    let container = &self.objects[container_index];
                    let closed = self
                        .definitions
                        .get(&container.definition_id)
                        .is_some_and(|definition| definition.closed_container() != 0);
                    (!closed).then_some(container.in_mat).flatten()
                })
        } else {
            self.landscape.as_ref().and_then(|landscape| {
                landscape.material_at(object.state.position.x, object.state.position.y)
            })
        };
        let contents_count = self.retained_contents_count(&object.state.contents);
        object.state.ocf = self.compute_object_ocf_for(object, contents_count, Some(linked));
    }

    pub(crate) fn retained_contents_count(&self, contents: &[ObjectId]) -> usize {
        contents
            .iter()
            .filter(|object_id| {
                self.find_object_index(**object_id)
                    .is_some_and(|index| self.objects[index].has_nonzero_status())
            })
            .count()
    }

    fn compute_object_ocf(&self, index: usize) -> u32 {
        let object = &self.objects[index];
        let contents_count = self.retained_contents_count(&object.state.contents);
        self.compute_object_ocf_for(object, contents_count, None)
    }

    fn compute_object_ocf_for(
        &self,
        object: &Object,
        contents_count: usize,
        pending_linked: Option<bool>,
    ) -> u32 {
        let definition = self.definitions.get(&object.definition_id);
        let mut ocf = definition
            .map(|definition| {
                definition.compute_ocf_with_contents_count(&object.state, contents_count)
            })
            .unwrap_or_else(|| {
                crate::ocf::compute(
                    OCF_NORMAL,
                    object.state.crew_member,
                    object.state.alive,
                    object.state.status,
                    object.state.container.is_some(),
                    object.state.construction,
                    object.state.category,
                )
            });
        // HitSpeeds from the fixed speed |xdir| + |ydir| (SetOCF,
        // C4Object.cpp:588-592)
        ocf |= movement_hit_speed_flags(object.fixed_velocity);
        // OCF_Chop: Chopable, StaticBack (excludes felled trees), and no
        // exclusive object blocking the center — the
        // Game.Objects.AtObject(x, y, OCF_Exclusive) probe (SetOCF,
        // C4Object.cpp:570-575).
        let pending_self_blocks_center = pending_linked == Some(true)
            && object.state.status.is_active()
            && object.state.container.is_none()
            && object.state.ocf & crate::ocf::EXCLUSIVE != 0
            && self
                .object_shape_rect(object)
                .contains_point(object.state.position.x, object.state.position.y);
        if definition.is_some_and(|definition| definition.is_chopable())
            && object.state.category & CATEGORY_STATIC_BACK != 0
            && !pending_self_blocks_center
            && self
                .at_object(object.state.position, crate::ocf::EXCLUSIVE, None)
                .is_none()
        {
            ocf |= crate::ocf::CHOP;
        }
        // The landscape probes GBackSolid/GBackSemiSolid see baked
        // C4SolidMask MCVehic pixels; rect-model fixture worlds join the
        // mask overlay here (grid worlds bake, and the overlay is empty).
        // Without a landscape everything is air, like C++ sky borders.
        let landscape = self.landscape.as_ref();
        let mut solid_masks = landscape
            .map(|_| self.ocf_solid_mask_overlay())
            .unwrap_or_default();
        if pending_linked.is_some() {
            // Init has not joined Game.Objects; initial DoCon has joined the
            // list but runs SetOCF before UpdateFace can put its new mask.
            solid_masks.retain(|mask| mask.object_id != object.id);
        }
        let masked = |x: i32, y: i32| solid_masks.iter().any(|mask| mask.contains(x, y));
        let solid = |x: i32, y: i32| landscape.is_some_and(|l| l.is_solid_at(x, y)) || masked(x, y);
        let semi_solid =
            |x: i32, y: i32| landscape.is_some_and(|l| l.is_semi_solid_at(x, y)) || masked(x, y);
        let x = object.state.position.x;
        let y = object.state.position.y;
        if object.state.container.is_none() {
            // OCF_InSolid (SetOCF, C4Object.cpp:637-640)
            if solid(x, y) {
                ocf |= crate::ocf::IN_SOLID;
            }
            // OCF_InFree (SetOCF, C4Object.cpp:641-644)
            if !semi_solid(x, y - 1) {
                ocf |= crate::ocf::IN_FREE;
            }
        }
        // OCF_Available (SetOCF, C4Object.cpp:645-648): reachable through
        // the container (Grab_Get or the container's cached OCF_Entrance),
        // and not buried — free above, or a thin non-solid cover with
        // clearance eight pixels up.
        let container_open =
            match object.state.container {
                None => true,
                Some(container_id) => {
                    self.find_object_index(container_id)
                        .is_some_and(|container_idx| {
                            let container = &self.objects[container_idx];
                            self.definitions.get(&container.definition_id).is_some_and(
                                |definition| definition.grab_put_get() & GRAB_PUT_GET_GET != 0,
                            ) || container.state.ocf & crate::ocf::ENTRANCE != 0
                        })
                }
            };
        if container_open && (!semi_solid(x, y - 1) || (!solid(x, y - 1) && !semi_solid(x, y - 8)))
        {
            ocf |= crate::ocf::AVAILABLE;
        }
        ocf
    }

    /// The solid-mask overlay for the SetOCF landscape probes: grid worlds
    /// bake masks into the pixel plane (empty overlay), rect fixture
    /// worlds carry them as rects like the movement checks.
    pub(crate) fn ocf_solid_mask_overlay(&self) -> Vec<SolidMaskRect> {
        if self.solid_mask_grid_mode() {
            return Vec::new();
        }
        let indices = self.active_solid_mask_indices();
        self.solid_masks_for_movement(&indices)
    }

    fn object_has_ocf(&self, index: usize, mask: u32) -> bool {
        self.object_ocf_at_index(index) & mask != 0
    }

    pub(crate) fn find_nearby_object_with_mask<F>(
        &self,
        origin_id: ObjectId,
        origin_pos: Vector2,
        mask: u32,
        radius: i32,
        mut filter: F,
    ) -> Option<(usize, ObjectId)>
    where
        F: FnMut(&Object) -> bool,
    {
        if radius <= 0 {
            return None;
        }
        let radius_sq = i64::from(radius) * i64::from(radius);
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.id != origin_id)
            .filter_map(|(index, object)| {
                if !self.object_has_ocf(index, mask) || !filter(object) {
                    return None;
                }
                let dx = i64::from(object.state.position.x - origin_pos.x);
                let dy = i64::from(object.state.position.y - origin_pos.y);
                let distance_sq = dx * dx + dy * dy;
                if distance_sq <= radius_sq {
                    Some((index, object.id, distance_sq))
                } else {
                    None
                }
            })
            .min_by_key(|(_, _, distance_sq)| *distance_sq)
            .map(|(index, id, _)| (index, id))
    }

    /// C4ObjectList::Add stContents insertion (C4ObjectList.cpp:110-176,
    /// reached from C4Object::Enter, C4Object.cpp:1587), forward order —
    /// index 0 is the C++ list head `First` (= `Contents(0)`):
    /// - line defs skip sorting (`fUnsorted`, :148) and append at the tail;
    /// - pass 1 (:150-162, skipped for C4D_StaticBack): insert before the
    ///   forward-first live entry with the same sorted category AND the
    ///   same def — the same-id cluster;
    /// - pass 2 (:164-173): insert before the forward-first live entry
    ///   whose (Category & C4D_SortLimit) <= the entering object's; with
    ///   no such entry the object appends at the tail.
    /// Unsorted objects (including ChangeDef between the swap and a later
    /// global resort sweep) append, and sorted insertions ignore unsorted
    /// peers in both scans.
    pub(crate) fn contents_insert_position(
        &self,
        container_index: usize,
        object_index: usize,
    ) -> usize {
        let object = &self.objects[object_index];
        self.contents_insert_position_for(
            container_index,
            object.state.category,
            &object.definition_id,
            object.unsorted,
        )
    }

    fn contents_insert_position_for(
        &self,
        container_index: usize,
        category: i32,
        definition_id: &str,
        unsorted: bool,
    ) -> usize {
        let contents = &self.objects[container_index].state.contents;
        let is_line = self
            .definitions
            .get(definition_id)
            .is_some_and(|definition| definition.line() != 0);
        if is_line || unsorted {
            return contents.len();
        }
        let sort_category = category & CATEGORY_SORT_LIMIT;
        let mut predecessor = None;
        let mut found_cluster = false;
        if category & CATEGORY_STATIC_BACK == 0 {
            for (position, &other) in contents.iter().enumerate() {
                let Some(other_index) = self.find_object_index(other) else {
                    continue;
                };
                let object = &self.objects[other_index];
                if object.destroyed
                    || object.state.status == ObjectStatus::Deleted
                    || object.unsorted
                {
                    continue;
                }
                if object.state.category & CATEGORY_SORT_LIMIT == sort_category
                    && object.definition_id == definition_id
                {
                    found_cluster = true;
                    break;
                }
                predecessor = Some(position);
            }
        }
        if !found_cluster {
            predecessor = None;
            for (position, &other) in contents.iter().enumerate() {
                let Some(other_index) = self.find_object_index(other) else {
                    continue;
                };
                let object = &self.objects[other_index];
                if object.destroyed
                    || object.state.status == ObjectStatus::Deleted
                    || object.unsorted
                {
                    continue;
                }
                if object.state.category & CATEGORY_SORT_LIMIT <= sort_category {
                    break;
                }
                predecessor = Some(position);
            }
        }
        predecessor.map_or(0, |position| position + 1)
    }

    pub(crate) fn track_contents_link_removal(&self, container_id: ObjectId, object_id: ObjectId) {
        let Some(container_index) = self.find_object_index(container_id) else {
            return;
        };
        let Some(position) = self.objects[container_index]
            .state
            .contents
            .iter()
            .position(|&child| child == object_id)
        else {
            return;
        };
        let generation = self
            .find_object_index(object_id)
            .map(|index| self.objects[index].state.contents_link_generation)
            .unwrap_or(0);
        let successor = self.objects[container_index]
            .state
            .contents
            .get(position + 1)
            .and_then(|&successor| {
                self.find_object_index(successor).map(|index| {
                    (
                        successor,
                        self.objects[index].state.contents_link_generation,
                    )
                })
            });
        crate::direct_com::track_internal_object_menu_link_removal(
            container_id,
            object_id,
            generation,
            successor,
        );
    }

    /// `loaded`: a compiled load rebuilds contents verbatim — C4ObjectList::
    /// DenumerateRead appends in saved order (Add stNone, C4ObjectList.cpp:
    /// 457-464) — while runtime entries sort in (Add stContents,
    /// C4Object.cpp:1587; see `contents_insert_position`).
    pub(crate) fn apply_container_change(
        &mut self,
        object_id: ObjectId,
        previous: Option<ObjectId>,
        new: Option<ObjectId>,
        loaded: bool,
    ) -> Result<(), EngineError> {
        self.apply_container_change_with_motion(object_id, previous, new, loaded, true)
    }

    /// Fold AssignRemoval's final Contained unlink without running Exit.
    /// Native removes the old Contents link and refreshes only the surviving
    /// parent; the deleted child keeps its cached OCF, mobility, liquid state,
    /// position, and motion (C4Object.cpp:284-306).
    pub(crate) fn apply_container_unlink_for_removal(
        &mut self,
        object_id: ObjectId,
        previous: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        if let Some(previous) = previous {
            if let Some(previous_index) = self.find_object_index(previous) {
                self.track_contents_link_removal(previous, object_id);
                self.objects[previous_index]
                    .state
                    .contents
                    .retain(|&child| child != object_id);
                self.refresh_object_ocf(previous_index);
            }
        }
        let object_index = self
            .find_object_index(object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        self.objects[object_index].state.container = None;
        self.objects[object_index].compiler_cache.contained = 0;
        Ok(())
    }

    /// Reconcile the authoritative Contents links for an Enter/Exit that the
    /// script host already executed synchronously. Its copied-out object
    /// fields contain the final native controller, motion, mobility, liquid,
    /// face, solid-mask, compiler-cache, and OCF state, so replaying ordinary
    /// [`Self::apply_container_change`] semantics here would move those side
    /// effects after later statements in the same callback.
    pub(crate) fn apply_host_container_link_change(
        &mut self,
        object_id: ObjectId,
        previous: Option<ObjectId>,
        new: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        if previous == new {
            return Ok(());
        }

        if let Some(previous) = previous {
            if let Some(previous_index) = self.find_object_index(previous) {
                self.track_contents_link_removal(previous, object_id);
                self.objects[previous_index]
                    .state
                    .contents
                    .retain(|&child| child != object_id);
            }
        }

        let object_index = self
            .find_object_index(object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        if let Some(container_id) = new {
            let container_index = self
                .find_object_index(container_id)
                .ok_or(EngineError::UnknownObject(container_id))?;
            if !self.objects[container_index]
                .state
                .contents
                .contains(&object_id)
            {
                let position = self.contents_insert_position(container_index, object_index);
                self.objects[container_index]
                    .state
                    .contents
                    .insert(position, object_id);
                let generation = &mut self.objects[object_index].state.contents_link_generation;
                *generation = generation.checked_add(1).unwrap_or(1);
            }
        }
        self.objects[object_index].state.container = new;
        Ok(())
    }

    /// Runtime `Enter` normally copies the new container's motion. Collect
    /// is the one C++ caller that passes `fCopyMotion=false`, keeping the
    /// entering object's exact position and fixed velocity through callbacks
    /// before its own post-Hit CopyMotion (C4Object.cpp:1598-1606,5698-5713).
    fn apply_container_change_with_motion(
        &mut self,
        object_id: ObjectId,
        previous: Option<ObjectId>,
        new: Option<ObjectId>,
        loaded: bool,
        copy_motion: bool,
    ) -> Result<(), EngineError> {
        if previous == new {
            return Ok(());
        }

        if let Some(prev_id) = previous {
            if let Some(prev_index) = self.find_object_index(prev_id) {
                if let Some(position) = self.objects[prev_index]
                    .state
                    .contents
                    .iter()
                    .position(|&child| child == object_id)
                {
                    let generation = self
                        .find_object_index(object_id)
                        .map(|index| self.objects[index].state.contents_link_generation)
                        .unwrap_or(0);
                    let successor = self.objects[prev_index]
                        .state
                        .contents
                        .get(position + 1)
                        .and_then(|&successor| {
                            self.find_object_index(successor).map(|index| {
                                (
                                    successor,
                                    self.objects[index].state.contents_link_generation,
                                )
                            })
                        });
                    crate::direct_com::track_internal_object_menu_link_removal(
                        prev_id, object_id, generation, successor,
                    );
                }
                let contents = &mut self.objects[prev_index].state.contents;
                contents.retain(|&child| child != object_id);
                // Exit refreshes the old container's OCF (Collection limit,
                // C4Object.cpp:1597).
                self.refresh_object_ocf(prev_index);
            }
        }

        let object_index = match self.find_object_index(object_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(object_id)),
        };

        match new {
            Some(container_id) => {
                if container_id == object_id {
                    return Err(EngineError::Container {
                        object: object_id,
                        detail: "object cannot contain itself".into(),
                    });
                }
                let container_index = match self.find_object_index(container_id) {
                    Some(index) => index,
                    None => return Err(EngineError::UnknownObject(container_id)),
                };
                let container = &self.objects[container_index];
                if container.destroyed || matches!(container.state.status, ObjectStatus::Deleted) {
                    return Err(EngineError::Container {
                        object: object_id,
                        detail: format!("container {} is destroyed", container_id),
                    });
                }
                if self.is_container_cycle(object_id, container_id) {
                    return Err(EngineError::Container {
                        object: object_id,
                        detail: format!("container {} would create a cycle", container_id),
                    });
                }

                if !self.objects[container_index]
                    .state
                    .contents
                    .contains(&object_id)
                {
                    let position = if loaded {
                        self.objects[container_index].state.contents.len()
                    } else {
                        self.contents_insert_position(container_index, object_index)
                    };
                    self.objects[container_index]
                        .state
                        .contents
                        .insert(position, object_id);
                    let generation = &mut self.objects[object_index].state.contents_link_generation;
                    *generation = generation.checked_add(1).unwrap_or(1);
                }

                if !loaded && previous.is_some() {
                    // Enter transfers through Exit first. Exit's literal-null
                    // assignment resets the raw enumeration cache; assigning
                    // the new typed pointer does not repopulate it.
                    self.objects[object_index].compiler_cache.contained = 0;
                }
                self.objects[object_index].state.container = Some(container_id);
                // "Assume that the new container controls this object, if
                // it cannot control itself (i.e.: Alive)" — projectile kill
                // tracing (C4Object::Enter, C4Object.cpp:1579-1582).
                let container_controller = self.objects[container_index].state.controller;
                let entering = &mut self.objects[object_index].state;
                if !(entering.alive && entering.category & CATEGORY_LIVING != 0) {
                    entering.controller = container_controller;
                }
                if !loaded {
                    // C4Object::Enter's runtime semantics (loads are
                    // denumeration): a TRANSFER exits first (`if (Contained)
                    // if (!Exit(x, y))`, C4Object.cpp:1579) and Exit
                    // mobilizes (`Mobile = 1; InLiquid = 0;`, :1540-1541);
                    // then fCopyMotion (default true, C4Object.h:313)
                    // copies the NEW container's motion IMMEDIATELY
                    // (:1598-1606; CopyMotion, C4Movement.cpp:523-534).
                    // The COLLECT path passes fCopyMotion=false and keeps
                    // its incoming position/velocity through this Enter
                    // (C4Object.cpp:5698).
                    // Enter installs Contained first, then removes the old
                    // solid mask before CopyMotion. Containment prevents
                    // UpdateSolidMask from putting it back at either site.
                    if copy_motion {
                        self.remove_solid_mask(object_index);
                    }
                    if previous.is_some() {
                        let entering = &mut self.objects[object_index].state;
                        entering.mobile = true;
                        entering.in_liquid = false;
                    }
                    if copy_motion {
                        let (container_position, container_velocity) = {
                            let container = &self.objects[container_index];
                            (container.state.position, container.fixed_velocity)
                        };
                        let object = &mut self.objects[object_index];
                        object.state.position = container_position;
                        object.fixed_position =
                            FixedVec2::from_ints(container_position.x, container_position.y);
                        object.fixed_velocity = container_velocity;
                        object.state.velocity = object.velocity_pixels();
                        self.update_sector_for_index(object_index);
                    }
                }
            }
            None => {
                self.objects[object_index].state.container = None;
                if !loaded {
                    self.objects[object_index].compiler_cache.contained = 0;
                }
                // C4Object::Exit resets InLiquid and mobilizes
                // (C4Object.cpp:1527-1528).
                self.objects[object_index].state.in_liquid = false;
                self.objects[object_index].state.mobile = true;
                // C4Object::Exit does NOT touch the master object list
                // (C4Object.cpp:1513-1545 only moves Contents) — the
                // exec position never changes on exit.
            }
        }
        // The moved object's own SetOCF (C4Object.cpp:1531,1570).
        self.refresh_object_ocf(object_index);
        // Enter always follows SetOCF with UpdateFace(true). With ordinary
        // fCopyMotion=true the mask was already removed before CopyMotion;
        // Collect's false form reaches the same removal only here, preserving
        // the C++ OCF-before-UpdateFace order (C4Object.cpp:1608-1621).
        if new.is_some() && !loaded {
            self.objects[object_index].refresh_shape_geometry();
            self.update_sector_for_index(object_index);
            if !copy_motion {
                self.remove_solid_mask(object_index);
            }
        }
        // C++ updates the entering object's OCF/face before it updates the
        // new container's mass and OCF (C4Object.cpp:1617-1624).
        if let Some(container_index) = new.and_then(|id| self.find_object_index(id)) {
            self.refresh_object_ocf(container_index);
        }
        if loaded {
            for container_id in previous.into_iter().chain(new) {
                if let Some(index) = self.find_object_index(container_id) {
                    self.objects[index].remember_compiled_mass_contents();
                }
            }
        }

        Ok(())
    }

    /// Fold a contents-link remove/add that cannot be represented by a final
    /// container delta when a callback exits and successfully re-enters the
    /// same parent. ChangeDef may supply an old-definition sort override;
    /// ordinary Enter uses the object's current key. Motion and object fields
    /// already contain the host's final, correctly ordered writes.
    pub(crate) fn reinsert_change_def_contents_link(
        &mut self,
        object_id: ObjectId,
    ) -> Result<(), EngineError> {
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        let sort_override = self.objects[object_index].change_def_contents_sort.take();
        let Some(container_id) = self.objects[object_index].state.container else {
            return Ok(());
        };
        // The initial silent Exit always mobilizes and clears InLiquid. A
        // final same-container relation would otherwise collapse that state
        // transition along with the container delta.
        self.objects[object_index].state.mobile = true;
        self.objects[object_index].state.in_liquid = false;
        let Some(container_index) = self.find_object_index(container_id) else {
            return Ok(());
        };
        if let Some(position) = self.objects[container_index]
            .state
            .contents
            .iter()
            .position(|&child| child == object_id)
        {
            let generation = self.objects[object_index].state.contents_link_generation;
            let successor = self.objects[container_index]
                .state
                .contents
                .get(position + 1)
                .and_then(|&successor| {
                    self.find_object_index(successor).map(|index| {
                        (
                            successor,
                            self.objects[index].state.contents_link_generation,
                        )
                    })
                });
            crate::direct_com::track_internal_object_menu_link_removal(
                container_id,
                object_id,
                generation,
                successor,
            );
        }
        self.objects[container_index]
            .state
            .contents
            .retain(|&child| child != object_id);
        let position = sort_override
            .filter(|sort| sort.container == container_id)
            .map(|sort| {
                self.contents_insert_position_for(
                    container_index,
                    sort.category,
                    &sort.definition_id,
                    sort.unsorted,
                )
            })
            .unwrap_or_else(|| self.contents_insert_position(container_index, object_index));
        self.objects[container_index]
            .state
            .contents
            .insert(position, object_id);
        let generation = &mut self.objects[object_index].state.contents_link_generation;
        *generation = generation.checked_add(1).unwrap_or(1);
        self.refresh_object_ocf(container_index);
        self.refresh_object_ocf(object_index);
        Ok(())
    }

    /// Engine-owned `C4Object::Exit(x, y)` at the object's current integer
    /// position. The containment change and motion reset are live before
    /// Ejection and Departure, and both callbacks are fail-safe.
    pub(crate) fn exit_object_at_current_position(
        &mut self,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let object = &self.objects[object_index];
        let Some(previous) = object.state.container else {
            return Ok(false);
        };
        let position = object.state.position;

        self.exit_object_at_position_with_zero_motion(object_id, previous, position, 0)
    }

    /// DFA_ATTACH's direct `Exit(x, y, r)`: unlike ordinary Enter's
    /// transfer `Exit(x, y)`, this preserves the current rotation while
    /// zeroing every motion component and dispatching the normal callbacks.
    pub(crate) fn exit_object_at_current_transform(
        &mut self,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let object = &self.objects[object_index];
        let Some(previous) = object.state.container else {
            return Ok(false);
        };
        let position = object.state.position;
        let rotation = object.state.rotation;

        self.exit_object_at_position_with_zero_motion(object_id, previous, position, rotation)
    }

    /// Engine-owned `C4Object::Exit(x, y, r, 0, 0, 0)`. The caller supplies
    /// the already-resolved previous container and rotation so the relation
    /// cannot drift between the precondition and the live unlink.
    pub(crate) fn exit_object_at_position_with_zero_motion(
        &mut self,
        object_id: ObjectId,
        previous: ObjectId,
        position: Vector2,
        rotation: i32,
    ) -> Result<bool, EngineError> {
        self.exit_object_at_position_with_full_motion(
            object_id,
            previous,
            position,
            rotation,
            FixedVec2::ZERO,
            C4Fixed::ZERO,
        )
    }

    /// Engine-owned `C4Object::Exit` with the caller-supplied full fixed
    /// motion. BoundsCheck still observes the object's pre-Exit motion; the
    /// requested motion is installed only after every bound/contact callback.
    pub(crate) fn exit_object_at_position_with_full_motion(
        &mut self,
        object_id: ObjectId,
        previous: ObjectId,
        position: Vector2,
        rotation: i32,
        velocity: FixedVec2,
        rotation_velocity: C4Fixed,
    ) -> Result<bool, EngineError> {
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        if self.objects[object_index].state.container != Some(previous) {
            return Ok(false);
        }

        // Raw first half of Exit: only the old container's list/OCF is
        // updated before BoundsCheck. The moving object's cached OCF, menu,
        // motion and liquid/mobile flags remain callback-visible until the
        // requested target has been clamped (C4Object.cpp:1519-1531).
        if let Some(previous_index) = self.find_object_index(previous) {
            self.track_contents_link_removal(previous, object_id);
            self.objects[previous_index]
                .state
                .contents
                .retain(|&child| child != object_id);
            self.refresh_object_ocf(previous_index);
        }
        if let Some(object_index) = self.find_object_index(object_id) {
            let object = &mut self.objects[object_index];
            object.state.container = None;
            object.compiler_cache.contained = 0;
        }

        let mut position = position;
        self.bounds_check_for_change_def_exit(object_id, &mut position)?;
        if let Some(object_index) = self.find_object_index(object_id) {
            let object = &mut self.objects[object_index];
            let previous_rect = object.current_shape_rect();
            let previous_construction = object.state.construction;
            object.set_position(position);
            object.state.rotation = rotation;
            object.fixed_rotation = itofix(rotation);
            object.fixed_velocity = velocity;
            object.state.velocity = object.velocity_pixels();
            object.rotation_velocity = rotation_velocity;
            object.state.mobile = true;
            object.state.in_liquid = false;
            object.state.menu = None;
            if object.shape_template.line == 0 {
                object.state.shape_override = None;
            }
            object.refresh_shape_after_state_change(previous_construction, previous_rect, false);
            self.update_solid_mask(object_index);
            self.update_sector_for_index(object_index);
            self.refresh_object_ocf(object_index);
        }

        if let Some(previous_index) = self.find_object_index(previous).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) {
            let _ = tolerate_script_error(self.call_object_function(
                previous_index,
                "Ejection",
                vec![object_reference_value(object_id)],
            ))?;
        }
        if let Some(object_index) = self.find_object_index(object_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) {
            let _ = tolerate_script_error(self.call_object_function(
                object_index,
                "Departure",
                vec![object_reference_value(previous)],
            ))?;
        }

        Ok(self
            .find_object_index(object_id)
            .is_some_and(|index| self.objects[index].state.container.is_none()))
    }

    /// ChangeDef's initial `Exit(0,0,0,0,0,0,false)`: the normal state and
    /// list updates, but no Ejection/Departure callbacks.
    fn apply_change_def_exit_target_bounds(
        &mut self,
        object_id: ObjectId,
        coordinate: &mut i32,
        low: i32,
        high: i32,
        low_cnat: u32,
        high_cnat: u32,
    ) -> Result<(), EngineError> {
        // C4Object::TargetBounds uses two independent `if`s. Inverted
        // bounds can therefore clamp/call at both ends; an `else if` would
        // observably lose the second Contact callback.
        if *coordinate < low {
            *coordinate = low;
            if let Some(index) = self.find_object_index(object_id) {
                let object = &mut self.objects[index];
                if low_cnat == CNAT_LEFT {
                    object.fixed_velocity.x = C4Fixed::ZERO;
                } else {
                    object.fixed_velocity.y = C4Fixed::ZERO;
                }
                object.state.velocity = object.velocity_pixels();
            }
            if let Some(index) = self.find_object_index(object_id) {
                self.dispatch_contact_callbacks(index, MovementContactDispatch::Direct(low_cnat))?;
            }
        }
        if *coordinate > high {
            *coordinate = high;
            if let Some(index) = self.find_object_index(object_id) {
                let object = &mut self.objects[index];
                if high_cnat == CNAT_RIGHT {
                    object.fixed_velocity.x = C4Fixed::ZERO;
                } else {
                    object.fixed_velocity.y = C4Fixed::ZERO;
                }
                object.state.velocity = object.velocity_pixels();
            }
            if let Some(index) = self.find_object_index(object_id) {
                self.dispatch_contact_callbacks(index, MovementContactDispatch::Direct(high_cnat))?;
            }
        }
        Ok(())
    }

    fn change_def_exit_layer_bounds(
        &self,
        object_id: ObjectId,
        horizontal: bool,
    ) -> Option<(i32, i32)> {
        let index = self.find_object_index(object_id)?;
        let object = self.objects.get(index)?;
        let definition = self.definitions.get(&object.definition_id)?;
        let layer = self.layer_movement_bounds_for(index)?;
        if layer.border_bound & C4D_BORDER_LAYER == 0 {
            return None;
        }
        let action_name = object.state.action.name.as_str();
        let procedure = definition
            .action_library()
            .procedure_for_entry(action_name, object.state.action.act_map_index);
        // C++'s numeric `Action.Act <= ActIdle` arm keeps None/Idle inside
        // layer bounds even if a synthetic Idle map names DFA_ATTACH.
        if !definition
            .action_library()
            .is_idle_state(&object.state.action)
            && matches!(procedure, ActionProcedure::Attach)
        {
            return None;
        }
        let shape = object.current_shape_rect().unwrap_or_default();
        let is_static = object.state.category & CATEGORY_STATIC_BACK != 0;
        let (layer_origin, layer_size, shape_offset) = if horizontal {
            (
                layer.position.x.saturating_add(layer.shape_rect.x),
                layer.shape_rect.width,
                shape.x,
            )
        } else {
            (
                layer.position.y.saturating_add(layer.shape_rect.y),
                layer.shape_rect.height,
                shape.y,
            )
        };
        let low = if is_static {
            layer_origin
        } else {
            layer_origin.saturating_sub(shape_offset)
        };
        let high = if is_static {
            layer_origin.saturating_add(layer_size)
        } else {
            layer_origin
                .saturating_add(layer_size)
                .saturating_add(shape_offset)
        };
        Some((low, high))
    }

    /// `Exit` calls BoundsCheck after unlinking containment but before it
    /// installs the requested position/motion. Each arm re-reads live state:
    /// a Contact callback may change Def, Shape, Layer or Action before the
    /// next arm runs (C4Movement.cpp:128-216).
    fn bounds_check_for_change_def_exit(
        &mut self,
        object_id: ObjectId,
        target: &mut Vector2,
    ) -> Result<(), EngineError> {
        if let Some((low, high)) = self.change_def_exit_layer_bounds(object_id, true) {
            self.apply_change_def_exit_target_bounds(
                object_id,
                &mut target.x,
                low,
                high,
                CNAT_LEFT,
                CNAT_RIGHT,
            )?;
        }

        let side_bounds = self.find_object_index(object_id).and_then(|index| {
            let object = self.objects.get(index)?;
            let definition = self.definitions.get(&object.definition_id)?;
            if definition.border_bound() & C4D_BORDER_SIDES == 0 {
                return None;
            }
            let shape_x = object
                .current_shape_rect()
                .map(|shape| shape.x)
                .unwrap_or(0);
            let width = self
                .landscape
                .as_ref()
                .and_then(|landscape| i32::try_from(landscape.width()).ok())?;
            Some((-shape_x, width.saturating_add(shape_x)))
        });
        if let Some((low, high)) = side_bounds {
            self.apply_change_def_exit_target_bounds(
                object_id,
                &mut target.x,
                low,
                high,
                CNAT_LEFT,
                CNAT_RIGHT,
            )?;
        }

        if let Some((low, high)) = self.change_def_exit_layer_bounds(object_id, false) {
            self.apply_change_def_exit_target_bounds(
                object_id,
                &mut target.y,
                low,
                high,
                CNAT_TOP,
                CNAT_BOTTOM,
            )?;
        }

        let top_bounds = self.find_object_index(object_id).and_then(|index| {
            let object = self.objects.get(index)?;
            let definition = self.definitions.get(&object.definition_id)?;
            if definition.border_bound() & C4D_BORDER_TOP == 0 {
                return None;
            }
            let shape_y = object
                .current_shape_rect()
                .map(|shape| shape.y)
                .unwrap_or(0);
            Some((-shape_y, 1_000_000))
        });
        if let Some((low, high)) = top_bounds {
            self.apply_change_def_exit_target_bounds(
                object_id,
                &mut target.y,
                low,
                high,
                CNAT_TOP,
                CNAT_BOTTOM,
            )?;
        }

        let bottom_bounds = self.find_object_index(object_id).and_then(|index| {
            let object = self.objects.get(index)?;
            let definition = self.definitions.get(&object.definition_id)?;
            if definition.border_bound() & C4D_BORDER_BOTTOM == 0 {
                return None;
            }
            let shape_y = object
                .current_shape_rect()
                .map(|shape| shape.y)
                .unwrap_or(0);
            let height = self
                .landscape
                .as_ref()
                .map(|landscape| landscape.estimated_height())?;
            Some((-1_000_000, height.saturating_add(shape_y)))
        });
        if let Some((low, high)) = bottom_bounds {
            self.apply_change_def_exit_target_bounds(
                object_id,
                &mut target.y,
                low,
                high,
                CNAT_TOP,
                CNAT_BOTTOM,
            )?;
        }
        Ok(())
    }

    pub(crate) fn exit_object_for_change_def(
        &mut self,
        object_id: ObjectId,
    ) -> Result<Option<ObjectId>, EngineError> {
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(None);
        };
        let Some(previous) = self.objects[object_index].state.container else {
            return Ok(None);
        };
        // Raw first half of Exit: remove the old contents link and update
        // only the parent. The moving object's own cached OCF, menu, Mobile
        // and InLiquid are intentionally stale during BoundsCheck Contact*
        // callbacks; Exit writes them only after BoundsCheck returns.
        if let Some(previous_index) = self.find_object_index(previous) {
            self.track_contents_link_removal(previous, object_id);
            self.objects[previous_index]
                .state
                .contents
                .retain(|&child| child != object_id);
            self.refresh_object_ocf(previous_index);
        }
        if let Some(object_index) = self.find_object_index(object_id) {
            let object = &mut self.objects[object_index];
            object.state.container = None;
            object.compiler_cache.contained = 0;
        }

        let mut target = Vector2::ZERO;
        self.bounds_check_for_change_def_exit(object_id, &mut target)?;
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(Some(previous));
        };
        {
            let object = &mut self.objects[object_index];
            let previous_rect = object.current_shape_rect();
            let previous_construction = object.state.construction;
            object.set_position(target);
            object.state.rotation = 0;
            object.fixed_rotation = C4Fixed::ZERO;
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
            object.rotation_velocity = C4Fixed::ZERO;
            object.state.mobile = true;
            object.state.in_liquid = false;
            object.state.menu = None;
            if object.shape_template.line == 0 {
                object.state.shape_override = None;
            }
            object.refresh_shape_after_state_change(previous_construction, previous_rect, false);
        }
        self.update_solid_mask(object_index);
        self.update_sector_for_index(object_index);
        self.refresh_object_ocf(object_index);
        Ok(Some(previous))
    }

    /// The callback tail shared by every engine-owned successful Enter.
    /// The new containment relation must already be live. Collection2 may
    /// move the entering object; Entrance then receives its current
    /// container (C4Object.cpp:1625-1630).
    fn run_object_enter_callbacks(
        &mut self,
        object_id: ObjectId,
        target_id: ObjectId,
    ) -> Result<(), EngineError> {
        if let Some(target_index) = self.find_object_index(target_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        }) {
            let _ = tolerate_script_error(self.call_object_function(
                target_index,
                "Collection2",
                vec![object_reference_value(object_id)],
            ))?;
        }
        let current_container = self
            .find_object_index(object_id)
            .and_then(|index| self.objects[index].state.container);
        let current_container_live = current_container.is_some_and(|container_id| {
            self.find_object_index(container_id).is_some_and(|index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != ObjectStatus::Deleted
            })
        });
        let target_live = self.find_object_index(target_id).is_some_and(|index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        });
        if current_container_live && target_live {
            if let (Some(object_index), Some(container_id)) =
                (self.find_object_index(object_id), current_container)
            {
                let _ = tolerate_script_error(self.call_object_function(
                    object_index,
                    "Entrance",
                    vec![object_reference_value(container_id)],
                ))?;
            }
        }
        Ok(())
    }

    /// `C4Object::Enter` for engine-owned callers such as C4CMD_Enter:
    /// reject before mutation, Exit a previous container, establish the new
    /// link, then call target Collection2 before the entering object's
    /// Entrance (C4Object.cpp:1566-1636). Script errors are fail-safe calls.
    pub(crate) fn try_object_enter(
        &mut self,
        object_id: ObjectId,
        target_id: ObjectId,
    ) -> Result<bool, EngineError> {
        self.try_object_enter_with_reject_collect(object_id, target_id, false)
            .map(|outcome| outcome == ObjectEnterOutcome::Entered)
    }

    /// The ordered core of C4Object::Enter. `query_reject_collect` is the
    /// non-null `pfRejectCollect` pointer used by Get/Put/Collect; ordinary
    /// C4CMD_Enter deliberately passes no pointer and therefore skips that
    /// collector veto (C4Object.cpp:1566-1591; C4Command.cpp:600-605).
    pub(crate) fn try_object_enter_with_reject_collect(
        &mut self,
        object_id: ObjectId,
        target_id: ObjectId,
        query_reject_collect: bool,
    ) -> Result<ObjectEnterOutcome, EngineError> {
        self.try_object_enter_with_reject_collect_and_calls(
            object_id,
            target_id,
            query_reject_collect,
            true,
        )
    }

    pub(crate) fn try_object_enter_with_reject_collect_and_calls(
        &mut self,
        object_id: ObjectId,
        target_id: ObjectId,
        query_reject_collect: bool,
        f_calls: bool,
    ) -> Result<ObjectEnterOutcome, EngineError> {
        self.try_object_enter_with_options(
            object_id,
            target_id,
            query_reject_collect,
            f_calls,
            true,
        )
    }

    fn try_object_enter_with_options(
        &mut self,
        object_id: ObjectId,
        target_id: ObjectId,
        query_reject_collect: bool,
        f_calls: bool,
        copy_motion: bool,
    ) -> Result<ObjectEnterOutcome, EngineError> {
        if object_id == target_id {
            return Ok(ObjectEnterOutcome::Failed);
        }
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(ObjectEnterOutcome::Failed);
        };
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(ObjectEnterOutcome::Failed);
        };

        // RejectEntrance belongs to the ENTERING object and runs before
        // cycle detection or Exit (C4Object.cpp:1575-1581).
        let rejected = if self.objects[object_index].destroyed
            || self.objects[object_index].state.status == ObjectStatus::Deleted
        {
            false
        } else {
            tolerate_script_error(self.call_object_function(
                object_index,
                "RejectEntrance",
                vec![object_reference_value(target_id)],
            ))?
            .is_some_and(|value| value.as_bool())
        };
        if rejected {
            return Ok(ObjectEnterOutcome::RejectedEntrance);
        }

        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(ObjectEnterOutcome::Removed);
        };
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(ObjectEnterOutcome::Failed);
        };
        let mut container = self.objects[target_index].state.container;
        let mut seen = HashSet::new();
        while let Some(container_id) = container {
            if container_id == object_id || !seen.insert(container_id) {
                return Ok(ObjectEnterOutcome::Failed);
            }
            container = self
                .find_object_index(container_id)
                .and_then(|index| self.objects[index].state.container);
        }

        if query_reject_collect {
            // C4Object::Enter queries the COLLECTOR after RejectEntrance
            // and cycle validation, with (entering definition, entering
            // object), before Exit or any containment mutation
            // (C4Object.cpp:1582-1591).
            let definition_id = self.objects[object_index].definition_id.clone();
            let rejected = if self.objects[target_index].destroyed
                || self.objects[target_index].state.status == ObjectStatus::Deleted
            {
                false
            } else {
                tolerate_script_error(self.call_object_function(
                    target_index,
                    "RejectCollect",
                    vec![
                        Value::C4Id(definition_id.as_str().to_string()),
                        object_reference_value(object_id),
                    ],
                ))?
                .is_some_and(|value| value.as_bool())
            };
            if rejected {
                return Ok(ObjectEnterOutcome::RejectedCollect);
            }
        }

        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(ObjectEnterOutcome::Removed);
        };

        // A transfer is an actual Exit first. Ejection precedes Departure;
        // either callback may re-enter, in which case Exit reports false and
        // the outer Enter aborts (C4Object.cpp:1592-1594, 1560-1563).
        let previous = self.objects[object_index].state.container;
        if previous.is_some() && !self.exit_object_at_current_position(object_id)? {
            return Ok(if self.find_object_index(object_id).is_some() {
                ObjectEnterOutcome::Failed
            } else {
                ObjectEnterOutcome::Removed
            });
        }

        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(ObjectEnterOutcome::Removed);
        };
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(ObjectEnterOutcome::Failed);
        };
        if self.objects[object_index].state.container.is_some()
            || self.objects[object_index].destroyed
            || self.objects[object_index].state.status == ObjectStatus::Deleted
            || self.objects[target_index].destroyed
            || self.objects[target_index].state.status == ObjectStatus::Deleted
        {
            return Ok(ObjectEnterOutcome::Failed);
        }

        // Forced CloseMenu runs after the final status gate and before the
        // new contents link (C4Object.cpp:1596).
        self.objects[object_index].state.menu = None;
        self.apply_container_change_with_motion(
            object_id,
            None,
            Some(target_id),
            false,
            copy_motion,
        )?;
        if f_calls {
            self.run_object_enter_callbacks(object_id, target_id)?;
        }

        // Collection2 and Entrance may both move the entering object. C++
        // re-reads Contained after the callbacks, still requires the original
        // pTarget to be live, and only then performs the synchronous base
        // auto-sale (C4Object.cpp:1625-1634).
        let target_live = self.find_object_index(target_id).is_some_and(|index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != ObjectStatus::Deleted
        });
        if self.base_auto_sell_enabled && target_live {
            let active_container = self
                .find_object_index(object_id)
                .and_then(|index| self.objects[index].state.container)
                .and_then(|container_id| self.find_object_index(container_id))
                .filter(|&index| {
                    !self.objects[index].destroyed
                        && self.objects[index].state.status != ObjectStatus::Deleted
                })
                .map(|index| (index, self.objects[index].state.base));
            if let Some((container_index, base_owner)) = active_container {
                if self.players.contains_key(&base_owner) {
                    self.auto_sell_base_contents(container_index, base_owner)?;
                }
            }
        }
        Ok(ObjectEnterOutcome::Entered)
    }

    /// `C4Object::Collect`: Enter with both vetoes and without its ordinary
    /// motion copy, then cancel ATTACH, notify the collector, dispatch live
    /// Hit thresholds in order, and only finally copy the current collector
    /// motion if callbacks left the item inside it (C4Object.cpp:5693-5715).
    pub(crate) fn try_object_collect(
        &mut self,
        object_id: ObjectId,
        collector_id: ObjectId,
    ) -> Result<bool, EngineError> {
        if self.try_object_enter_with_options(object_id, collector_id, true, true, false)?
            != ObjectEnterOutcome::Entered
        {
            return Ok(false);
        }

        let attached = self.find_object_index(object_id).is_some_and(|index| {
            let object = &self.objects[index];
            self.definitions
                .get(&object.definition_id)
                .is_some_and(|definition| {
                    definition.action_library().procedure_for_entry(
                        &object.state.action.name,
                        object.state.action.act_map_index,
                    ) == ActionProcedure::Attach
                })
        });
        if attached {
            if let Some(index) = self.find_object_index(object_id) {
                let definition_id = self.objects[index].definition_id.clone();
                let _ =
                    tolerate_script_error(self.action_with_calls(index, &definition_id, "Idle"))?;
            }
        }

        if let Some(index) = self
            .find_object_index(collector_id)
            .filter(|&index| self.objects[index].has_nonzero_status())
        {
            let _ = tolerate_script_error(self.call_object_function(
                index,
                "Collection",
                vec![object_reference_value(object_id)],
            ))?;
        }

        for (flag, callback) in [
            (ocf::HIT_SPEED1, "Hit"),
            (ocf::HIT_SPEED2, "Hit2"),
            (ocf::HIT_SPEED3, "Hit3"),
        ] {
            let Some(index) = self.find_object_index(object_id) else {
                break;
            };
            if !self.objects[index].has_nonzero_status()
                || self.objects[index].state.ocf & flag == 0
            {
                continue;
            }
            let _ = tolerate_script_error(self.call_object_function(index, callback, Vec::new()))?;
        }

        if let Some(index) = self
            .find_object_index(object_id)
            .filter(|&index| self.objects[index].state.container == Some(collector_id))
        {
            self.copy_motion_from_container(index);
        }
        Ok(true)
    }

    /// ObjectComPut's synchronous transfer. Unlike ordinary C4CMD_Enter,
    /// it supplies the RejectCollect pointer, then fires Put/Collection
    /// only after a successful Enter (C4ObjectCom.cpp:591-622).
    pub(crate) fn try_object_com_put(
        &mut self,
        actor_id: ObjectId,
        target_id: ObjectId,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(false);
        };
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(false);
        };
        if self.objects[actor_index].state.container != Some(target_id)
            && self
                .definitions
                .get(&self.objects[target_index].definition_id)
                .is_none_or(|definition| definition.grab_put_get() & GRAB_PUT_GET_PUT == 0)
        {
            let owner = self.objects[actor_index].state.owner;
            if self
                .players
                .get(&owner)
                .is_some_and(|player| player.control.last_com_down_double != 0)
            {
                return self.object_com_drop(actor_id, object_id);
            }
            return Ok(false);
        }
        if self.objects[target_index].state.ocf & ocf::FULL_CON == 0 {
            return Ok(false);
        }
        let collection_limit = self
            .definitions
            .get(&self.objects[target_index].definition_id)
            .map_or(0, Definition::collection_limit);
        let contents_count = self.objects[target_index]
            .state
            .contents
            .iter()
            .filter(|object_id| {
                self.find_object_index(**object_id)
                    .is_some_and(|index| self.objects[index].has_nonzero_status())
            })
            .count();
        if collection_limit_reached(collection_limit, contents_count) {
            return Ok(false);
        }

        if self.try_object_enter_with_reject_collect(object_id, target_id, true)?
            != ObjectEnterOutcome::Entered
        {
            return Ok(false);
        }
        if let Some(actor_index) = self.find_object_index(actor_id) {
            let _ =
                tolerate_script_error(self.call_object_function(actor_index, "Put", Vec::new()))?;
        }
        if let Some(target_index) = self.find_object_index(target_id) {
            let _ = tolerate_script_error(self.call_object_function(
                target_index,
                "Collection",
                vec![object_reference_value(object_id), Value::Bool(true)],
            ))?;
        }
        Ok(true)
    }

    /// ObjectComPutTake's inline put-or-menu operation. Throw/Drop ignore
    /// this helper's boolean, but all callbacks must finish before their
    /// original command is marked complete (C4ObjectCom.cpp:700-721).
    pub(crate) fn try_object_com_put_take(
        &mut self,
        actor_id: ObjectId,
        target_id: ObjectId,
        requested_item: Option<ObjectId>,
    ) -> Result<ObjectComPutTakeOutcome, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(ObjectComPutTakeOutcome::Finished);
        };
        if self.find_object_index(target_id).is_none() {
            return Ok(ObjectComPutTakeOutcome::Finished);
        }
        let (contents, container, controller, owner) = {
            let actor = &self.objects[actor_index];
            (
                actor.state.contents.clone(),
                actor.state.container,
                actor.state.controller,
                actor.state.owner,
            )
        };
        let item_id = match requested_item {
            Some(item_id) if contents.contains(&item_id) => Some(item_id),
            Some(item_id) if self.find_object_index(item_id).is_some() => {
                return Ok(ObjectComPutTakeOutcome::NeedsGet(item_id));
            }
            Some(_) | None => contents.into_iter().find(|item_id| {
                self.find_object_index(*item_id)
                    .is_some_and(|index| self.objects[index].has_nonzero_status())
            }),
        };

        if let Some(item_id) = item_id {
            let _ = self.try_object_com_put(actor_id, target_id, item_id)?;
            return Ok(ObjectComPutTakeOutcome::Finished);
        }

        let request = if container == Some(target_id) {
            Some(MenuRequest {
                crew_id: actor_id,
                owner: controller,
                kind: MenuRequestKind::Activate,
            })
        } else {
            let grab_get = self
                .find_object_index(target_id)
                .and_then(|index| self.definitions.get(&self.objects[index].definition_id))
                .is_some_and(|definition| definition.grab_put_get() & GRAB_PUT_GET_GET != 0);
            grab_get.then_some(MenuRequest {
                crew_id: actor_id,
                owner,
                kind: MenuRequestKind::Get {
                    container: target_id,
                },
            })
        };
        if let Some(request) = request {
            self.apply_container_menu_request(request)?;
        }
        Ok(ObjectComPutTakeOutcome::Finished)
    }

    /// C4Object::PutAwayUnusedObject, with the Tutorial04-critical direct
    /// put into the actor's containing HUT2 and the same command fallbacks
    /// for failed contained/outside puts (C4Object.cpp:5853-5891).
    pub(crate) fn put_away_unused_object(
        &mut self,
        actor_id: ObjectId,
        object_to_make_room_for: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(false);
        };
        let custom_selector = self
            .definitions
            .get(&self.objects[actor_index].definition_id)
            .is_some_and(|definition| definition.has_function("GetObject2Drop"));
        let unused = if custom_selector {
            let definition_id = self.objects[actor_index].definition_id.clone();
            let selected = tolerate_script_error(self.call_object_function(
                actor_index,
                "GetObject2Drop",
                vec![object_to_make_room_for
                    .map(object_reference_value)
                    .unwrap_or(Value::Nil)],
            ))?;
            selected
                .map(|value| {
                    value_to_object_reference(
                        definition_id.as_str(),
                        "GetObject2Drop",
                        "result",
                        value,
                    )
                })
                .transpose()?
                .flatten()
        } else {
            self.objects[actor_index].state.contents.last().copied()
        };
        let Some(unused) = unused else {
            return Ok(false);
        };

        let (procedure, action_target, contained) = {
            let actor = &self.objects[actor_index];
            let procedure = self
                .definitions
                .get(&actor.definition_id)
                .map(|definition| {
                    definition.action_library().procedure_for_entry(
                        &actor.state.action.name,
                        actor.state.action.act_map_index,
                    )
                })
                .unwrap_or_default();
            (procedure, actor.state.action.target, actor.state.container)
        };
        if procedure == ActionProcedure::Push {
            if let Some(target) = action_target {
                if self.try_object_com_put(actor_id, target, unused)? {
                    return Ok(true);
                }
            }
        }
        if let Some(container_id) = contained {
            if self.try_object_com_put(actor_id, container_id, unused)? {
                return Ok(true);
            }
            if let Some(actor_index) = self.find_object_index(actor_id) {
                self.objects[actor_index].apply_command_operations([
                    CommandOperation::PushFront(
                        CommandRequest::new(CommandId::Drop)
                            .with_target(Some(unused))
                            .with_mode(CommandMode::SilentSub),
                    ),
                    CommandOperation::PushFront(
                        CommandRequest::new(CommandId::Exit).with_mode(CommandMode::SilentSub),
                    ),
                ]);
                return Ok(true);
            }
            return Ok(false);
        }

        self.object_com_drop(actor_id, unused)
    }

    /// C4Command::GetTryEnter's outcome-sensitive wrapper. RejectCollect
    /// makes room and returns to the same live Get command; a successful
    /// contained pickup fires CLNK's Get callback (C4Command.cpp:1092-1126).
    pub(crate) fn try_get_object_enter(
        &mut self,
        actor_id: ObjectId,
        object_id: ObjectId,
        command_instance_id: u64,
    ) -> Result<GetEnterOutcome, EngineError> {
        let target_gate = self.find_object_index(object_id).map(|index| {
            let target = &self.objects[index];
            let name = target.state.custom_name.clone().unwrap_or_else(|| {
                self.definitions.get(&target.definition_id).map_or_else(
                    || target.definition_id.clone(),
                    |definition| definition.name().to_string(),
                )
            });
            (
                target.state.container,
                minimum_con_activation_denied(target.state.category, target.state.construction),
                name,
            )
        });

        if let Some((Some(container_id), minimum_con_denied, target_name)) = target_gate {
            // C4Command::GetTryEnter runs CheckMinimumCon before any script
            // callback or Enter attempt (C4Command.cpp:1092-1095,1295-1305).
            if minimum_con_denied {
                return Ok(GetEnterOutcome::MinimumConstructionDenied(format!(
                    "{target_name} not completed.|Activation denied."
                )));
            }

            // The target's current container may veto removing its contents.
            // This precedes the collection-limit and RejectCollect paths
            // (C4Command.cpp:1096-1098).
            if let Some(container_index) = self.find_object_index(container_id) {
                let rejected = tolerate_script_error(self.call_object_function(
                    container_index,
                    "RejectContents",
                    Vec::new(),
                ))?
                .is_some_and(|value| compat::value_raw_truthy(&value));
                if rejected {
                    return Ok(GetEnterOutcome::Failed);
                }
            }
        }

        // A full collector makes room before Target->Enter, so the desired
        // object's RejectEntrance/RejectCollect callbacks do not run on this
        // evaluation (C4Command.cpp:1099-1106).
        let collection_limit_reached = self.find_object_index(actor_id).is_some_and(|index| {
            let contents_count = self.objects[index]
                .state
                .contents
                .iter()
                .filter(|object_id| {
                    self.find_object_index(**object_id)
                        .is_some_and(|index| self.objects[index].has_nonzero_status())
                })
                .count();
            let collection_limit = self
                .definitions
                .get(&self.objects[index].definition_id)
                .map_or(0, Definition::collection_limit);
            crate::collection_limit_reached(collection_limit, contents_count)
        });
        if collection_limit_reached {
            let current_target = self.find_object_index(actor_id).and_then(|index| {
                self.objects[index]
                    .commands
                    .get_event_target_after_callback(command_instance_id, object_id)
            });
            return if self.put_away_unused_object(actor_id, current_target)? {
                Ok(GetEnterOutcome::Retry)
            } else {
                Ok(GetEnterOutcome::Failed)
            };
        }

        let was_contained = self
            .find_object_index(object_id)
            .is_some_and(|index| self.objects[index].state.container.is_some());
        let enter_outcome = self.try_object_enter_with_reject_collect(object_id, actor_id, true)?;
        let current_target = self.find_object_index(actor_id).and_then(|index| {
            self.objects[index]
                .commands
                .get_event_target_after_callback(command_instance_id, object_id)
        });
        if current_target.is_none() {
            return Ok(GetEnterOutcome::Completed);
        }
        match enter_outcome {
            ObjectEnterOutcome::Entered => {
                if self.find_object_index(object_id).is_none() {
                    return Ok(GetEnterOutcome::Completed);
                }
                if was_contained {
                    if let Some(actor_index) = self.find_object_index(actor_id) {
                        let _ = tolerate_script_error(self.call_object_function(
                            actor_index,
                            "Get",
                            vec![object_reference_value(object_id)],
                        ))?;
                    }
                }
                Ok(GetEnterOutcome::Entered)
            }
            ObjectEnterOutcome::RejectedCollect => {
                if self.put_away_unused_object(actor_id, current_target)? {
                    Ok(GetEnterOutcome::Retry)
                } else {
                    Ok(GetEnterOutcome::Failed)
                }
            }
            ObjectEnterOutcome::Removed => Ok(GetEnterOutcome::Completed),
            ObjectEnterOutcome::RejectedEntrance | ObjectEnterOutcome::Failed => {
                Ok(GetEnterOutcome::Failed)
            }
        }
    }

    /// ObjectActionThrow (C4ObjectCom.cpp:120-137): resolve the physical
    /// force/facing first, honor the ordinary SetAction gate, then consume
    /// exactly one synced rotation draw and perform C4Object::Exit.
    #[doc(hidden)]
    pub fn try_object_action_throw(
        &mut self,
        actor_id: ObjectId,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(false);
        };
        let definition_id = self.objects[actor_index].definition_id.clone();
        let current_procedure = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?
            .action_library()
            .procedure_for_entry(
                &self.objects[actor_index].state.action.name,
                self.objects[actor_index].state.action.act_map_index,
            );
        if current_procedure != ActionProcedure::Walk {
            return Ok(false);
        }
        // Force and direction precede SetAction in C++ and therefore cannot
        // be changed by Throw's StartCall/Walk's AbortCall.
        let throw_force = math::val_by_physical(400, self.object_physical(actor_index).throw);
        let direction = if self.objects[actor_index].state.direction == Direction::Left {
            -1
        } else {
            1
        };
        // ObjectActionThrow resolves SetActionByName against the live Def
        // after GetPhysical returns (C4ObjectCom.cpp:127-130).
        let live_definition_id = self.objects[actor_index].definition_id.clone();
        if !self.action_with_calls(actor_index, &live_definition_id, "Throw")? {
            return Ok(false);
        }

        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(true);
        };
        let position = self.objects[actor_index].state.position;
        let shape_top = self.objects[actor_index]
            .current_shape_rect()
            .map(|rect| rect.y)
            .unwrap_or(0);
        let rotation = self.rng.random(360);
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(true);
        };
        let Some(previous_container) = self.objects[object_index].state.container else {
            return Ok(true);
        };
        let _ = self.exit_object_at_position_with_full_motion(
            object_id,
            previous_container,
            Vector2::new(position.x, position.y + shape_top - 1),
            rotation,
            FixedVec2::new(throw_force * direction, -throw_force),
            throw_force * direction,
        )?;
        // ObjectActionThrow ignores Exit's boolean (including callback
        // re-entry) and reports success once SetAction succeeded.
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocf_overlay_skips_objects_without_solid_masks() {
        let mut engine = Engine::new();
        engine
            .register_script_definition("Plain", "Plain", "func Noop() { return 0; }")
            .expect("plain definition registers");
        for _ in 0..2 {
            engine
                .spawn_object(SpawnConfig::new("Plain"))
                .expect("plain object spawns");
        }

        SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(|count| count.set(0));
        SOLID_MASK_DEFINITION_LOOKUPS.with(|count| count.set(0));
        assert!(engine.ocf_solid_mask_overlay().is_empty());
        assert!(engine.ocf_solid_mask_overlay().is_empty());
        assert_eq!(SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(Cell::get), 0);
        assert_eq!(SOLID_MASK_DEFINITION_LOOKUPS.with(Cell::get), 0);
    }

    /// A grid world whose texmap carries a `Vehicle` default entry, which is
    /// what `Landscape::grid_vehicle_byte` reads to decide that masks are
    /// baked into the pixel plane.
    fn grid_world_engine() -> Engine {
        let mut engine = Engine::new();
        let mut texmap = crate::landscape::RuntimeTexMapState::default();
        texmap.set_default_material_entry("Vehicle", 2);
        let mut landscape =
            crate::landscape::Landscape::new(8, vec![8; 8]).expect("landscape builds");
        landscape.set_raster_state(crate::landscape::LandscapeRasterState::new(1, 0, texmap));
        engine.set_landscape(landscape);
        engine
    }

    #[test]
    fn grid_world_movement_skips_the_solid_mask_candidate_scan() {
        // A grid world bakes masks into the plane via put_solid_mask, so
        // solid_masks_for_movement returns an empty overlay whatever the
        // candidate scan found (:966-969) — and every real scenario is a grid
        // world. Deriving the candidates first walked every object against the
        // definition table to build a value its only consumer discards: on
        // Gold Rush that dead scan was ~23% of every simulation frame.
        let mut engine = grid_world_engine();
        assert!(
            engine.solid_mask_grid_mode(),
            "the fixture must be a grid world or this test pins nothing"
        );
        let mut masked =
            Definition::from_script("Masked", "Masked", "").expect("masked definition compiles");
        masked.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine
            .register_definition(masked)
            .expect("masked definition registers");
        engine
            .register_script_definition("Plain", "Plain", "func Noop() { return 0; }")
            .expect("plain definition registers");
        engine
            .spawn_object(SpawnConfig::new("Masked"))
            .expect("masked object spawns");
        for _ in 0..8 {
            engine
                .spawn_object(SpawnConfig::new("Plain"))
                .expect("plain object spawns");
        }

        SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(|count| count.set(0));
        SOLID_MASK_DEFINITION_LOOKUPS.with(|count| count.set(0));
        assert!(
            engine.live_movement_solid_masks().is_empty(),
            "a grid world applies masks through the plane, not the rect overlay"
        );
        assert_eq!(
            SOLID_MASK_DEFINITION_LOOKUPS.with(Cell::get),
            0,
            "the discarded candidate scan must not run at all"
        );
        assert_eq!(SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(Cell::get), 0);
    }

    #[test]
    fn stationary_object_execution_does_not_repaint_its_solid_mask() {
        // C4Object::Execute only reaches UpdateSolidMask through operations
        // that actually update mask-relevant state; an immobile object's
        // ExecMovement performs no mask update (C4Object.cpp:1082-1105;
        // C4Movement.cpp:558-590).
        let mut engine = grid_world_engine();
        engine
            .landscape
            .as_mut()
            .expect("grid landscape")
            .set_pixel_grid(crate::landscape::PixelGrid::new(
                8,
                8,
                vec![0; 64],
                vec![0; 128],
                vec![None; 128],
                vec![None; 128],
            ));
        let mut masked =
            Definition::from_script("Masked", "Masked", "").expect("masked definition compiles");
        masked.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine
            .register_definition(masked)
            .expect("masked definition registers");
        engine
            .spawn_object(SpawnConfig::new("Masked"))
            .expect("masked object spawns");
        let revision = engine
            .landscape()
            .and_then(crate::landscape::Landscape::pixel_grid)
            .expect("grid landscape")
            .revision();

        engine.advance_tick().expect("stationary frame advances");

        assert_eq!(
            engine
                .landscape()
                .and_then(crate::landscape::Landscape::pixel_grid)
                .expect("grid landscape")
                .revision(),
            revision,
            "stationary execution leaves the already-current mask untouched"
        );
    }

    #[test]
    fn grid_world_tick_does_not_scan_solid_mask_definitions_before_movement() {
        // C4Object::Execute dispatches each object's own movement directly;
        // there is no frame-global solid-mask candidate enumeration before
        // that dispatch (C4Object.cpp:1082-1105; C4Movement.cpp:553-616).
        // Grid worlds already bake masks into the landscape plane, so such an
        // enumeration would be discarded by every movement consumer.
        let mut engine = grid_world_engine();
        let mut masked =
            Definition::from_script("Masked", "Masked", "").expect("masked definition compiles");
        masked.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine
            .register_definition(masked)
            .expect("masked definition registers");
        engine
            .spawn_object(SpawnConfig::new("Masked"))
            .expect("masked object spawns");

        SOLID_MASK_DEFINITION_LOOKUPS.with(|count| count.set(0));
        engine.advance_tick().expect("stationary frame advances");

        assert_eq!(
            SOLID_MASK_DEFINITION_LOOKUPS.with(Cell::get),
            0,
            "grid movement must not derive a rect-overlay candidate list"
        );
    }

    #[test]
    fn one_solid_mask_put_acquires_surface8_cow_storage_once() {
        // C4SolidMask::Put is one uninterrupted row-major Surface8 raster
        // walk (C4SolidMask.cpp:79-101). Rust may acquire its COW planes once
        // for that walk; reacquiring them for every pixel is bookkeeping with
        // no C++ counterpart.
        let mut engine = grid_world_engine();
        engine
            .landscape
            .as_mut()
            .expect("grid landscape")
            .set_pixel_grid(crate::landscape::PixelGrid::new(
                8,
                8,
                vec![0; 64],
                vec![0; 128],
                vec![None; 128],
                vec![None; 128],
            ));
        let mut masked =
            Definition::from_script("Masked", "Masked", "").expect("masked definition compiles");
        masked.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 2, 2, 0, 0)));
        engine
            .register_definition(masked)
            .expect("masked definition registers");
        crate::landscape::MASK_WRITE_BATCH_ACTIVATIONS.with(|count| count.set(0));

        engine
            .spawn_object(SpawnConfig::new("Masked"))
            .expect("masked object spawns");

        assert_eq!(
            crate::landscape::MASK_WRITE_BATCH_ACTIVATIONS.with(Cell::get),
            1
        );
    }

    #[test]
    fn rect_world_movement_still_collects_the_solid_mask_overlay() {
        // The overlay model is the fixture path, and it must keep seeing
        // masks: the grid short-circuit may not swallow a world that has no
        // baked plane to fall back on.
        let mut engine = Engine::new();
        assert!(!engine.solid_mask_grid_mode(), "no landscape, no grid");
        let mut masked =
            Definition::from_script("Masked", "Masked", "").expect("masked definition compiles");
        masked.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine
            .register_definition(masked)
            .expect("masked definition registers");
        engine
            .spawn_object(SpawnConfig::new("Masked"))
            .expect("masked object spawns");

        assert_eq!(
            engine.live_movement_solid_masks().len(),
            1,
            "the rect overlay still reports the masked object"
        );
    }

    #[test]
    fn frozen_solid_mask_candidates_survive_a_grid_mode_change() {
        let mut engine = grid_world_engine();
        let mut masked =
            Definition::from_script("Masked", "Masked", "").expect("masked definition compiles");
        masked.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine
            .register_definition(masked)
            .expect("masked definition registers");
        engine
            .spawn_object(SpawnConfig::new("Masked"))
            .expect("masked object spawns");

        // Rust movement freezes fixture-overlay candidates before some
        // contact/action callbacks and consumes them afterwards. A mode
        // change across that seam must not erase an otherwise-live candidate.
        let candidates = engine.active_solid_mask_indices();
        engine.clear_landscape();

        assert_eq!(engine.solid_masks_for_movement(&candidates).len(), 1);
    }

    #[test]
    fn ocf_overlay_observes_direct_runtime_override() {
        let mut engine = Engine::new();
        engine
            .register_script_definition("Plain", "Plain", "func Noop() { return 0; }")
            .expect("plain definition registers");
        engine
            .spawn_object(SpawnConfig::new("Plain"))
            .expect("plain object spawns");
        assert!(engine.ocf_solid_mask_overlay().is_empty());

        engine.objects[0].state.solid_mask_override =
            Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0));
        SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(|count| count.set(0));

        assert_eq!(engine.ocf_solid_mask_overlay().len(), 1);
        assert_eq!(SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(Cell::get), 1);

        engine.objects[0].state.solid_mask_override =
            Some(DefinitionTargetRect::new(0, 0, 0, 0, 0, 0));
        assert!(engine.ocf_solid_mask_overlay().is_empty());
    }

    #[test]
    fn ocf_overlay_observes_reloaded_definition_mask() {
        let mut engine = Engine::new();
        engine
            .register_script_definition("Plain", "Plain", "func Noop() { return 0; }")
            .expect("plain definition registers");
        engine
            .spawn_object(SpawnConfig::new("Plain"))
            .expect("plain object spawns");
        assert!(engine.ocf_solid_mask_overlay().is_empty());

        assert!(engine.remove_definition("Plain"));
        let mut definition =
            Definition::from_script("Plain", "Plain", "").expect("definition compiles");
        definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine
            .register_definition(definition)
            .expect("replacement definition registers");

        SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(|count| count.set(0));
        assert_eq!(engine.ocf_solid_mask_overlay().len(), 1);
        assert_eq!(SOLID_MASK_MOVEMENT_CANDIDATE_VISITS.with(Cell::get), 1);
    }
}
