//! `impl Engine` — dig/blast/shake operations, particles and transfer zones.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

#[cfg(test)]
std::thread_local! {
    pub(crate) static MATERIAL_INCINERATE_PROBES: std::cell::RefCell<Vec<(i32, i32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

impl Engine {
    pub(crate) fn process_dig_material_conversions(&mut self, idx: usize, requested: bool) {
        if idx >= self.objects.len() || self.materials.is_empty() {
            return;
        }

        self.objects[idx].ensure_material_capacity(self.materials.len());
        let casts = self
            .materials
            .iter()
            .filter_map(|material| {
                Some((
                    material.id(),
                    material.dig_to_object_name()?.to_string(),
                    material.dig_to_object_ratio()?,
                    material.dig_to_object_on_request_only(),
                ))
            })
            .collect::<Vec<_>>();

        // C4Object::DigOutMaterialCast walks materials inline. Each
        // CreateObject lifecycle may move, reshape, relayer, or add contents
        // to the digger before the next material is considered; reset only
        // the current bucket after that lifecycle returns.
        for (material, definition_id, ratio, on_request_only) in casts {
            if ratio == 0 || (on_request_only && !requested) {
                continue;
            }
            let current = self.objects[idx].material_content(material);
            if current == 0 || current < ratio {
                continue;
            }
            let (creator, spawn_position, layer) = {
                let object = &self.objects[idx];
                let position = object.state.position;
                let bottom = object.current_shape_rect().map_or(position.y, |shape| {
                    position
                        .y
                        .saturating_add(shape.y)
                        .saturating_add(shape.height)
                });
                (
                    object.id,
                    Vector2::new(position.x, bottom),
                    object.state.layer,
                )
            };
            let rotation = self.rng.random(360);
            if self.definitions.contains_key(&definition_id) {
                let mut config = SpawnConfig::new(definition_id)
                    .with_position(spawn_position)
                    .with_owner(OWNER_NONE)
                    .with_rotation(rotation);
                if let Some(layer) = layer {
                    config = config.with_layer(layer);
                }
                let _ = self.spawn_object_with_initial_lifecycle(config, Some(creator));
            }
            self.objects[idx].set_material_content(material, 0);
        }
    }

    #[doc(hidden)]
    pub fn apply_landscape_operations(&mut self, operations: Vec<LandscapeOperation>) {
        if operations.is_empty() {
            return;
        }
        for operation in operations {
            if self.solid_mask_staging.defer_solid_mask_updates {
                // Host landscape calls share the chronological mask stream
                // and replay there after copy-out channels materialize.
                continue;
            }
            match operation {
                LandscapeOperation::DigCircle {
                    center,
                    radius,
                    requested,
                    by_object,
                } => self.execute_dig_circle_operation(center, radius, requested, by_object),
                LandscapeOperation::DigCirclePreviewed { center, radius } => {
                    self.execute_dig_circle_pixels_only(center, radius)
                }
                LandscapeOperation::DigRect {
                    origin,
                    width,
                    height,
                    requested,
                    by_object,
                } => self.execute_dig_rect_operation(origin, width, height, requested, by_object),
                LandscapeOperation::DigRectPreviewed {
                    origin,
                    width,
                    height,
                } => self.execute_dig_rect_pixels_only(origin, width, height),
                LandscapeOperation::ClearRect {
                    origin,
                    width,
                    height,
                } => self.execute_clear_rect_operation(origin, width, height, None),
                LandscapeOperation::ClearRectDensity {
                    origin,
                    width,
                    height,
                    density,
                } => self.execute_clear_rect_operation(origin, width, height, Some(density)),
                LandscapeOperation::PrepareConstructionTerrain {
                    center_x,
                    bottom_y,
                    width,
                    height,
                    basement,
                } => self.prepare_construction_terrain(center_x, bottom_y, width, height, basement),
                LandscapeOperation::DrawMaterialQuad {
                    material_texture,
                    vertices,
                    ift,
                } => {
                    let _ = self.draw_material_quad(&material_texture, vertices, ift);
                }
                LandscapeOperation::DrawMatChunks {
                    origin,
                    width,
                    height,
                    count_x,
                    count_y,
                    material,
                    byte,
                    map_seed,
                    random_offsets,
                    texmap,
                } => {
                    let _ = self.draw_material_chunks(
                        origin,
                        width,
                        height,
                        count_x,
                        count_y,
                        &material,
                        byte,
                        map_seed,
                        &random_offsets,
                        texmap,
                    );
                }
                LandscapeOperation::DrawVolcanoBranch {
                    from,
                    to,
                    size,
                    material_byte,
                } => {
                    if let Some(landscape) = &mut self.landscape {
                        let _ = landscape.draw_volcano_branch(from, to, size, material_byte);
                    }
                }
                LandscapeOperation::DrawMap {
                    origin,
                    bitmap,
                    map_width,
                    map_height,
                    texmap,
                    map_creator,
                } => {
                    let _ = self.draw_indexed_map(origin, &bitmap, map_width, map_height, texmap);
                    if let Some(map_creator) = map_creator {
                        let _ = self.replace_runtime_map_creator(map_creator.0);
                    }
                }
                LandscapeOperation::DrawDefMap {
                    origin,
                    bitmap,
                    map_width,
                    map_height,
                    texmap,
                    map_creator,
                } => {
                    let _ = self.draw_indexed_map(origin, &bitmap, map_width, map_height, texmap);
                    let _ = self.replace_runtime_map_creator(map_creator.0);
                }
                LandscapeOperation::SyncRuntimeTexMap { texmap } => {
                    let _ = self.replace_runtime_texmap(texmap);
                }
                LandscapeOperation::SetTextureIndex {
                    texmap,
                    old_index,
                    new_index,
                } => {
                    let _ = self.apply_runtime_texture_index_move(texmap, old_index, new_index);
                }
                LandscapeOperation::RemoveUnusedTexMapEntries { .. } => {
                    let _ = self.remove_unused_runtime_texmap_entries();
                }
                LandscapeOperation::BlastCircle {
                    center,
                    radius,
                    controller,
                } => self.execute_blast_circle_operation(center, radius, controller),
                LandscapeOperation::BlastCirclePreviewed {
                    center,
                    radius,
                    replay,
                } => self.execute_blast_replay(center, radius, replay),
                LandscapeOperation::ShakeCircle { center, radius } => {
                    self.execute_shake_circle_operation(center, radius)
                }
                LandscapeOperation::GammaRamp { index, points } => {
                    // The host already applied SetGamma's silent valid-index
                    // gate. Retain it here for robust operation replay.
                    self.gamma.set_ramp(index, points);
                }
                LandscapeOperation::SkyAdjust {
                    modulation,
                    back_color,
                } => {
                    // FnSetSkyAdjust -> C4Sky::SetModulation
                    // (C4Sky.cpp:238-244).
                    self.sky
                        .get_or_insert_with(|| SkyState::new(SkySettings::default()))
                        .apply_modulation(modulation, back_color);
                }
                LandscapeOperation::MatAdjust { modulation } => {
                    // FnSetMatAdjust/FnSetMaterialColor ->
                    // C4Landscape::SetModulation (C4Script.cpp:4451-4465,
                    // 4626-4630; C4Landscape.h:200-205).
                    if let Some(landscape) = &mut self.landscape {
                        landscape.set_modulation(modulation);
                    }
                }
                LandscapeOperation::SetLandscapePixel { position, color } => {
                    // FnSetLandscapePixel writes only the visible 32-bit
                    // surface; Surface8/material queries stay untouched.
                    if let Some(landscape) = &mut self.landscape {
                        let _ = landscape.set_surface32_pixel(position.x, position.y, color);
                    }
                }
                LandscapeOperation::SetLandscapePixels { writes } => {
                    // Adjacent FnSetLandscapePixel calls share one storage
                    // transaction, while the per-write revision/token and
                    // dirty-record sequence remains ordered in the landscape.
                    if let Some(landscape) = &mut self.landscape {
                        landscape.set_surface32_pixels(&writes);
                    }
                }
                LandscapeOperation::SkyParallax {
                    mode,
                    par_x,
                    par_y,
                    xdir,
                    ydir,
                    x,
                    y,
                } => {
                    // FnSetSkyParallax (C4Script.cpp:4955-4970) mutates
                    // Game.Landscape.Sky; a world without a configured
                    // sky has nothing to scroll.
                    if let Some(sky) = &mut self.sky {
                        sky.apply_parallax(mode, par_x, par_y, xdir, ydir, x, y);
                    }
                }
                LandscapeOperation::ExtractMaterialAmount {
                    material,
                    position,
                    amount,
                } => {
                    // FnExtractMaterialAmount (C4Script.cpp:2264-2273):
                    // rerun the REAL loop the host fn simulated.
                    if let Some(material_id) =
                        usize::try_from(material).ok().and_then(MaterialId::new)
                    {
                        for _ in 0..amount {
                            if self.landscape_material(position.x, position.y) != Some(material_id)
                            {
                                break;
                            }
                            if self.extract_material(position.x, position.y) != Some(material_id) {
                                break;
                            }
                        }
                    }
                }
                LandscapeOperation::ExtractLiquid { position } => {
                    // FnExtractLiquid already performed its synchronous
                    // GBackLiquid/material query in the host call. Fold the
                    // corresponding real C4Landscape::ExtractMaterial now
                    // (C4Script.cpp:2194-2199).
                    let _ = self.extract_material(position.x, position.y);
                }
                LandscapeOperation::InsertMaterial {
                    material,
                    position,
                    velocity,
                } => {
                    // FnInsertMaterial → C4Landscape::InsertMaterial
                    // (C4Script.cpp:2207-2211) — the full port (slide
                    // re-creation as PXS, reactions, thrust).
                    if let Some(material_id) =
                        usize::try_from(material).ok().and_then(MaterialId::new)
                    {
                        self.insert_material(
                            material_id,
                            position.x,
                            position.y,
                            velocity.x,
                            velocity.y,
                        );
                    }
                }
                LandscapeOperation::CastPxs {
                    material,
                    position,
                    velocities,
                } => {
                    // FnCastPXS already consumed the synced r2/r1 draws
                    // while the VM call was live; preserve their order as
                    // PXS slots are allocated (C4PXS.cpp:309-321). Create
                    // rejects an invalid material before New can allocate a
                    // chunk (C4PXS.cpp:207-215).
                    if self.materials.get_by_id(material).is_none() {
                        continue;
                    }
                    for velocity in velocities {
                        self.pxs_system.create(
                            &self.materials,
                            material,
                            math::itofix(position.x),
                            math::itofix(position.y),
                            velocity.x,
                            velocity.y,
                        );
                    }
                }
            }
        }
    }

    /// C4Landscape::DigFreePix (C4Landscape.cpp:936-944): clear DigFree
    /// materials, then CheckInstabilityRange at the probed pixel — ALWAYS,
    /// even when nothing clears. Returns the material like the C++ (grid
    /// worlds only; `None` without a plane).
    pub(crate) fn dig_free_pix(&mut self, tx: i32, ty: i32) -> Option<MaterialId> {
        let mat = {
            let materials = &self.materials;
            self.landscape
                .as_mut()
                .and_then(|landscape| landscape.dig_free_pix(tx, ty, materials))
        };
        self.check_instability_range(tx, ty);
        mat
    }

    /// `C4Landscape::DigFreeMat` (C4Landscape.cpp:1012-1021): only pixels
    /// of the requested material enter `DigFreePix`; there is no material
    /// accounting or dig-out cast for this rectangle helper.
    pub(crate) fn dig_free_material_rect(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        material: MaterialId,
    ) {
        if self.materials.get_by_id(material).is_none() || width <= 0 || height <= 0 {
            return;
        }
        let x_end = origin.x.saturating_add(width);
        let y_end = origin.y.saturating_add(height);
        let mut changed_min = None::<i32>;
        let mut changed_max = None::<i32>;
        for x in origin.x..x_end {
            for y in origin.y..y_end {
                let matches = self
                    .landscape
                    .as_ref()
                    .and_then(|landscape| landscape.dig_free_pixel_material_at(x, y))
                    == Some(material);
                if matches {
                    let _ = self.dig_free_pix(x, y);
                    let changed = self
                        .landscape
                        .as_ref()
                        .and_then(|landscape| landscape.material_at(x, y))
                        != Some(material);
                    if changed {
                        changed_min = Some(changed_min.map_or(x, |current| current.min(x)));
                        changed_max = Some(changed_max.map_or(x, |current| current.max(x)));
                    }
                }
            }
        }
        if let (Some(changed_min), Some(changed_max), Some(landscape)) =
            (changed_min, changed_max, self.landscape.as_mut())
        {
            let start = changed_min.max(0) as usize;
            let end = changed_max
                .saturating_add(1)
                .max(0)
                .min(landscape.width() as i32) as usize;
            landscape.refresh_raster_columns(start..end);
        }
    }

    /// C4Landscape::DigFreeSinglePix (C4Landscape.h:236-240): DigFreePix
    /// (with its instability probe) only when the pixel is denser than its
    /// neighbour toward (dx, dy).
    fn dig_free_single_pix(&mut self, x: i32, y: i32, dx: i32, dy: i32) {
        let denser = self
            .landscape
            .as_ref()
            .map(|landscape| {
                landscape.density_at(x, y, &self.materials)
                    > landscape.density_at(x + dx, y + dy, &self.materials)
            })
            .unwrap_or(false);
        if denser {
            let _ = self.dig_free_pix(x, y);
        }
    }

    pub(crate) fn execute_dig_circle_operation(
        &mut self,
        center: Vector2,
        radius: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    ) {
        self.execute_dig_circle_operation_inner(center, radius, Some((requested, by_object)));
    }

    fn execute_dig_circle_pixels_only(&mut self, center: Vector2, radius: i32) {
        self.execute_dig_circle_operation_inner(center, radius, None);
    }

    fn execute_dig_circle_operation_inner(
        &mut self,
        center: Vector2,
        radius: i32,
        material_accounting: Option<(bool, Option<ObjectId>)>,
    ) {
        if radius <= 0 {
            return;
        }
        let Some(landscape) = self.landscape.as_ref() else {
            return;
        };
        let mut removal_counts: HashMap<MaterialId, i32> = HashMap::new();
        if landscape.pixel_grid().is_some() {
            // C4Landscape::DigFree (C4Landscape.cpp:980-1001), including
            // the single-pixel edge clears and C++'s reuse of the LAST
            // row's line width for the bottom edge.
            let mut line_width = 0;
            for ycnt in -radius..radius {
                let remaining =
                    i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
                line_width = (remaining as f64).sqrt() as i32;
                let line_y = center.y + ycnt;
                let extend = i32::from(line_width == 0);
                for xcnt in -line_width..line_width + extend {
                    if let Some(material_id) = self.dig_free_pix(center.x + xcnt, line_y) {
                        *removal_counts.entry(material_id).or_insert(0) += 1;
                    }
                }
                self.dig_free_single_pix(center.x - line_width - 1, line_y, -1, 0);
                self.dig_free_single_pix(center.x + line_width + extend, line_y, 1, 0);
            }
            self.dig_free_single_pix(center.x, center.y - radius - 1, 0, -1);
            let extend = i32::from(line_width == 0);
            for xcnt in -line_width..line_width + extend {
                self.dig_free_single_pix(center.x + xcnt, center.y + radius, 0, 1);
            }
        } else {
            let materials = self.materials.clone();
            let Some(landscape) = self.landscape.as_mut() else {
                return;
            };
            let width = landscape.width() as i32;
            let radius_sq = i64::from(radius) * i64::from(radius);
            for dx in -radius..=radius {
                let column = center.x.saturating_add(dx);
                if column < 0 || column >= width {
                    continue;
                }
                let dx_sq = i64::from(dx) * i64::from(dx);
                if dx_sq > radius_sq {
                    continue;
                }
                let remaining = radius_sq - dx_sq;
                if remaining < 0 {
                    continue;
                }
                let vertical = (remaining as f64).sqrt().floor() as i32;
                let target = center.y.saturating_add(vertical);
                if let Some((material_id, removed)) =
                    Self::dig_column(&materials, landscape, column, target)
                {
                    removal_counts
                        .entry(material_id)
                        .and_modify(|value| *value = value.saturating_add(removed))
                        .or_insert(removed);
                }
            }
        }
        if let Some((requested, by_object)) = material_accounting {
            self.apply_dig_removal_counts(removal_counts, requested, by_object);
        }
    }

    pub(crate) fn execute_dig_rect_operation(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    ) {
        self.execute_dig_rect_operation_inner(origin, width, height, Some((requested, by_object)));
    }

    fn execute_dig_rect_pixels_only(&mut self, origin: Vector2, width: i32, height: i32) {
        self.execute_dig_rect_operation_inner(origin, width, height, None);
    }

    fn execute_dig_rect_operation_inner(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        material_accounting: Option<(bool, Option<ObjectId>)>,
    ) {
        if width <= 0 || height <= 0 {
            return;
        }
        let Some(landscape) = self.landscape.as_ref() else {
            return;
        };
        let mut removal_counts: HashMap<MaterialId, i32> = HashMap::new();
        if landscape.pixel_grid().is_some() {
            // C4Landscape::DigFreeRect (C4Landscape.cpp:1003-1014):
            // per-pixel DigFreePix; EVERY valid-material pixel counts
            // toward the digger's material contents, dug free or not.
            for cx in origin.x..origin.x.saturating_add(width) {
                for cy in origin.y..origin.y.saturating_add(height) {
                    if let Some(material_id) = self.dig_free_pix(cx, cy) {
                        *removal_counts.entry(material_id).or_insert(0) += 1;
                    }
                }
            }
        } else {
            let materials = self.materials.clone();
            let Some(landscape) = self.landscape.as_mut() else {
                return;
            };
            let landscape_width = landscape.width() as i32;
            let bottom = origin.y.saturating_add(height);
            for offset in 0..width {
                let column = origin.x.saturating_add(offset);
                if column < 0 || column >= landscape_width {
                    continue;
                }
                if let Some((material_id, removed)) =
                    Self::dig_column(&materials, landscape, column, bottom)
                {
                    removal_counts
                        .entry(material_id)
                        .and_modify(|value| *value = value.saturating_add(removed))
                        .or_insert(removed);
                }
            }
        }
        if let Some((requested, by_object)) = material_accounting {
            self.apply_dig_removal_counts(removal_counts, requested, by_object);
        }
    }

    /// `Landscape::ClearRect/ClearRectDensity` (FnFreeRect, C4Script.cpp:
    /// 3119-3125): clear outright without dug-out material accounting or PXS.
    fn execute_clear_rect_operation(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        density: Option<i32>,
    ) {
        if height <= 0 {
            return;
        }
        if density.is_none()
            && self
                .landscape
                .as_ref()
                .is_some_and(|landscape| landscape.pixel_grid().is_some())
        {
            let bounds = landscape::RasterChangeRect::new(origin.x, origin.y, width, height);
            let _ = self.landscape_solid_mask_transaction(bounds, |landscape| {
                landscape.clear_rect_pixels(bounds);
            });
            return;
        }
        let materials = self.materials.clone();
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };
        let landscape_height = landscape.estimated_height();
        for row in origin.y..origin.y.saturating_add(height) {
            Self::mutate_clear_rect_landscape_row(
                landscape,
                &materials,
                origin.x,
                row,
                width,
                density,
                landscape_height,
            );
        }
    }

    /// One source row of C4Landscape::ClearRect/ClearRectDensity. The caller
    /// owns row ordering because FnFreeRect interleaves each completed row
    /// with `if (Rnd3()) Rnd3()` before script execution resumes.
    pub(crate) fn mutate_clear_rect_landscape_row(
        landscape: &mut Landscape,
        materials: &MaterialSet,
        x: i32,
        y: i32,
        width: i32,
        density: Option<i32>,
        landscape_height: i32,
    ) {
        let density_range = density.map(|density| match density {
            C4M_VEHICLE => (C4M_VEHICLE, 1000),
            C4M_SOLID => (C4M_SOLID, C4M_VEHICLE - 1),
            C4M_SEMI_SOLID => (C4M_SEMI_SOLID, C4M_SOLID - 1),
            0 => (0, C4M_SEMI_SOLID - 1),
            density => (density, density),
        });
        if landscape.pixel_grid().is_some() {
            // ClearPix has no DigFree gate. Density selection uses the exact
            // inclusive Pix2Dens band before each in-bounds write.
            for column in x..x.saturating_add(width) {
                let matches = density_range.is_none_or(|(minimum, maximum)| {
                    (minimum..=maximum).contains(&landscape.density_at(column, y, materials))
                });
                if matches {
                    landscape.clear_pix(column, y);
                }
            }
            return;
        }

        // Column-only fixtures retain no arbitrary pixel plane, but they can
        // faithfully clear liquid segments and the exposed solid surface.
        // ClearPix ignores DigFree, so do not route this through dig_column.
        if y < 0 || y >= landscape_height {
            return;
        }
        let landscape_width = landscape.width() as i32;
        for column in x..x.saturating_add(width) {
            if column < 0 || column >= landscape_width {
                continue;
            }
            let matches = density_range.is_none_or(|(minimum, maximum)| {
                (minimum..=maximum).contains(&landscape.density_at(column, y, materials))
            });
            if !matches {
                continue;
            }
            if landscape.is_liquid_at(column, y) {
                landscape.remove_liquid_at(column, y);
            } else if landscape.is_solid_at(column, y) {
                landscape.ensure_surface_at_least(column, y.saturating_add(1));
            }
        }
    }

    fn apply_dig_removal_counts(
        &mut self,
        removal_counts: HashMap<MaterialId, i32>,
        requested: bool,
        by_object: Option<ObjectId>,
    ) {
        let Some(object_id) = by_object else {
            return;
        };
        let Some(object_index) = self.find_object_index(object_id) else {
            return;
        };
        if self.materials.is_empty() {
            return;
        }
        {
            let object = &mut self.objects[object_index];
            object.ensure_material_capacity(self.materials.len());
            for (material_id, removed) in &removal_counts {
                object.add_material_content(*material_id, *removed);
            }
        }
        // DigFree/DigFreeRect add contents on every call, then run the cast
        // check only on !Tick5 — including calls that removed no new pixels
        // (C4Landscape.cpp:982,996).
        if self.frame.is_multiple_of(5) {
            self.process_dig_material_conversions(object_index, requested);
        }
    }

    fn execute_blast_circle_operation(
        &mut self,
        center: Vector2,
        radius: i32,
        controller: Option<i32>,
    ) {
        if radius < 0 {
            return;
        }
        let _ = self.blast_circle(center, radius, controller);
    }

    fn execute_blast_replay(&mut self, center: Vector2, radius: i32, replay: BlastReplay) {
        match replay.pixels {
            BlastPixelReplay::Raster {
                steps,
                pixel_count_by_material,
            } => {
                let mut result = BlastResult {
                    pixel_count_by_material,
                    ..BlastResult::default()
                };
                let mut changed_columns = HashSet::new();
                for step in steps {
                    if let Some(byte) = step.shift_byte {
                        if self.landscape.as_mut().is_some_and(|landscape| {
                            landscape.insert_material_texture_pix(
                                step.position.x,
                                step.position.y,
                                byte,
                            )
                        }) {
                            changed_columns.insert(step.position.x);
                        }
                    }
                    if step.clear
                        && self.landscape.as_mut().is_some_and(|landscape| {
                            landscape.clear_pix(step.position.x, step.position.y)
                        })
                    {
                        if let Some(material) = step.original_material {
                            *result.removed_by_material.entry(material).or_insert(0) += 1;
                        }
                        changed_columns.insert(step.position.x);
                    }
                    self.check_instability_range(step.position.x, step.position.y);
                }
                if let Some((width, _)) =
                    self.landscape.as_ref().and_then(Landscape::grid_dimensions)
                {
                    let start = center.x.saturating_sub(radius).clamp(0, width) as usize;
                    let end = center
                        .x
                        .saturating_add(radius)
                        .saturating_add(1)
                        .clamp(0, width) as usize;
                    if let Some(landscape) = self.landscape.as_mut() {
                        landscape.refresh_raster_columns(start..end);
                        for x in start..end {
                            let x = x as i32;
                            if changed_columns.contains(&x) {
                                if let Some(surface) = landscape.surface_height(x) {
                                    result.affected_columns.push((x, surface));
                                }
                            }
                        }
                    }
                }
            }
            BlastPixelReplay::Column { shift_decisions } => {
                let result = self
                    .landscape
                    .as_mut()
                    .map(|landscape| landscape.blast_circle(center, radius, &self.materials))
                    .unwrap_or_default();
                if let Some(landscape) = self.landscape.as_mut() {
                    for (candidate, should_shift) in
                        result.shift_candidates.iter().zip(shift_decisions)
                    {
                        if should_shift && candidate.apply_column_shift && candidate.column >= 0 {
                            landscape.set_solid_material(
                                candidate.column as u32,
                                Some(candidate.target),
                            );
                        }
                    }
                }
                for ycnt in -radius..=radius {
                    let remaining =
                        i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
                    let line_width = (remaining.max(0) as f64).sqrt() as i32;
                    let y = center.y.saturating_add(ycnt);
                    for xcnt in -line_width..line_width + i32::from(line_width == 0) {
                        self.check_instability_range(center.x.saturating_add(xcnt), y);
                    }
                }
            }
        }
    }

    #[doc(hidden)]
    pub fn execute_shake_circle_operation(&mut self, center: Vector2, radius: i32) {
        if radius <= 0 {
            return;
        }
        if self
            .landscape
            .as_ref()
            .is_some_and(|landscape| landscape.pixel_grid().is_some())
        {
            let mut cleared_solid_pixels = Vec::new();
            // C4Landscape::ShakeFree walks top-to-bottom in this exact
            // circle order. ShakeFreePix clears each DigFree pixel, creates
            // its zero-velocity PXS, then probes instability; non-DigFree
            // material is left in place (C4Landscape.cpp:928-938,999-1010).
            for ycnt in (-radius..radius).rev() {
                let remaining =
                    i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
                let line_width = (remaining as f64).sqrt() as i32;
                let y = center.y + ycnt;
                for xcnt in -line_width..line_width + i32::from(line_width == 0) {
                    let x = center.x + xcnt;
                    let material = {
                        let materials = &self.materials;
                        self.landscape
                            .as_mut()
                            .and_then(|landscape| landscape.dig_free_pix(x, y, materials))
                    };
                    if let Some(material) = material.filter(|material| {
                        self.materials
                            .get_by_id(*material)
                            .is_some_and(|material| material.dig_free())
                    }) {
                        if self
                            .materials
                            .get_by_id(material)
                            .is_some_and(|material| material.is_solid())
                        {
                            cleared_solid_pixels.push(Vector2::new(x, y));
                        }
                        self.pxs_system.create(
                            &self.materials,
                            material,
                            itofix(x),
                            itofix(y),
                            C4Fixed::ZERO,
                            C4Fixed::ZERO,
                        );
                    }
                    self.check_instability_range(x, y);
                }
            }

            let dislodged = self
                .landscape
                .as_ref()
                .map(|landscape| {
                    landscape.shake_free_fragments(&cleared_solid_pixels, &self.materials)
                })
                .unwrap_or_default();
            for (position, material) in dislodged {
                let cleared = self.landscape.as_mut().and_then(|landscape| {
                    landscape.dig_free_pix(position.x, position.y, &self.materials)
                });
                if cleared == Some(material) {
                    self.pxs_system.create(
                        &self.materials,
                        material,
                        itofix(position.x),
                        itofix(position.y),
                        C4Fixed::ZERO,
                        C4Fixed::ZERO,
                    );
                    self.check_instability_range(position.x, position.y);
                }
            }
            return;
        }
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };
        if self.materials.is_empty() {
            return;
        }
        let width = landscape.width() as i32;
        if width <= 0 {
            return;
        }
        let radius_sq = i64::from(radius) * i64::from(radius);
        for dx in -radius..=radius {
            let column = center.x.saturating_add(dx);
            if column < 0 || column >= width {
                continue;
            }
            let dx_sq = i64::from(dx) * i64::from(dx);
            if dx_sq > radius_sq {
                continue;
            }
            let remaining = radius_sq - dx_sq;
            if remaining < 0 {
                continue;
            }
            let vertical = (remaining as f64).sqrt().floor() as i32;
            let mut target_height = center.y.saturating_add(vertical);
            let previous_height = match landscape.surface_height(column) {
                Some(height) => height,
                None => continue,
            };
            if target_height <= previous_height {
                let distance = previous_height.saturating_sub(target_height);
                if distance > radius {
                    continue;
                }
                target_height = previous_height.saturating_add(1);
            }
            let Some((material_id, removed)) =
                Self::dig_column(&self.materials, landscape, column, target_height)
            else {
                continue;
            };
            if removed <= 0 {
                continue;
            }
            let count = match usize::try_from(removed) {
                Ok(count) if count > 0 => count,
                _ => continue,
            };
            // Freed pixels become zero-velocity PXS at their integer
            // positions, like DigFreePix → PXS.Create (C4Landscape.cpp:947-954).
            for offset in 0..count {
                self.pxs_system.create(
                    &self.materials,
                    material_id,
                    itofix(column),
                    itofix(previous_height.saturating_add(offset as i32)),
                    C4Fixed::ZERO,
                    C4Fixed::ZERO,
                );
            }
        }
        // C4Landscape::ShakeFree's per-pixel ShakeFreePix ends in
        // CheckInstabilityRange (C4Landscape.cpp:946-956) for EVERY pixel
        // of the circle, walked top row LAST (`ycnt = rad - 1; ycnt >=
        // -rad`, :1021-1027). The column shake above cannot interleave the
        // probes, so they run as a post-pass in the C++ scan order.
        for ycnt in (-radius..radius).rev() {
            let remaining =
                i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
            let lwdt = (remaining as f64).sqrt() as i32;
            let dpy = center.y + ycnt;
            for xcnt in -lwdt..lwdt + i32::from(lwdt == 0) {
                self.check_instability_range(center.x + xcnt, dpy);
            }
        }
    }

    /// C4Landscape::BlastFree evaluate loop (C4Landscape.cpp:1065-1079):
    /// materials in INDEX order, gated on the pre-blast BlastMatCount;
    /// within one material BlastCastObjects runs BEFORE PXS.Cast, and each
    /// object is created inline between its draws and the next.
    pub(crate) fn process_blast_reactions(
        &mut self,
        center: Vector2,
        controller: Option<i32>,
        result: &BlastResult,
    ) {
        let material_casts: Vec<(MaterialId, Option<String>, Option<i32>, Option<i32>)> = self
            .materials
            .iter()
            .map(|material| {
                (
                    material.id(),
                    material.blast_to_object_name().map(str::to_string),
                    material.blast_to_object_ratio(),
                    material.blast_to_pxs_ratio(),
                )
            })
            .collect();

        for (material_id, object_name, object_ratio, pxs_ratio) in material_casts {
            let count = result
                .pixel_count_by_material
                .get(&material_id)
                .copied()
                .unwrap_or(0);
            // `if (BlastMatCount[cnt])` (C4Landscape.cpp:1067)
            if count == 0 {
                continue;
            }

            // Blast2Object → C4Game::BlastCastObjects (C4Game.cpp:1723-1735):
            // per object 4 draws in argument-evaluation order — rdir =
            // itofix(Random(3) + 1), ydir = FIXED10(Random(61) - 40),
            // xdir = FIXED10(Random(61) - 30), angle = Random(360) — then
            // CreateObject(id, nullptr, NO_OWNER, tx, ty, …, iByPlayer).
            // The draws happen BEFORE C4Id2Def, so an unloaded definition
            // still consumes them (C4Game.cpp:1142-1148).
            if let (Some(definition_id), Some(ratio)) = (object_name, object_ratio) {
                if ratio != 0 {
                    let num = count / ratio;
                    for _ in 0..num {
                        let rdir = itofix(self.rng.random(3) + 1);
                        let ydir = fixed10(self.rng.random(61) - 40);
                        let xdir = fixed10(self.rng.random(61) - 30);
                        let rotation = self.rng.random(360);
                        let config = SpawnConfig::new(definition_id.clone())
                            .with_position(center)
                            .with_rotation(rotation)
                            .with_fixed_velocity(FixedVec2::new(xdir, ydir))
                            .with_rotation_velocity(rdir)
                            .with_owner(OWNER_NONE)
                            .with_controller(controller.unwrap_or(OWNER_NONE));
                        // Unknown definition: C4Id2Def → nullptr, no object.
                        let _ = self.spawn_object(config);
                    }
                }
            }

            // Blast2PXSRatio → PXS.Cast(mat, BlastMatCount/ratio, tx, ty, 60)
            // (C4Landscape.cpp:1075-1078)
            if let Some(ratio) = pxs_ratio {
                if ratio != 0 {
                    self.pxs_system.cast(
                        &self.materials,
                        &mut self.rng,
                        material_id,
                        count / ratio,
                        center.x,
                        center.y,
                        60,
                    );
                }
            }
        }
    }

    pub(crate) fn apply_blast_shifts(&mut self, radius: i32, result: &BlastResult) {
        if result.shift_candidates.is_empty() {
            return;
        }
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };

        let threshold = (compute_blast_size(radius) * compute_blast_grade(radius)) / 6;

        for candidate in &result.shift_candidates {
            let total_pixels = match result.pixel_count_by_material.get(&candidate.material) {
                Some(value) => *value,
                None => continue,
            };
            if total_pixels <= 0 {
                continue;
            }
            let pixel_count = candidate.pixel_count.max(0);
            if pixel_count <= 0 {
                continue;
            }
            // BlastFreePix evaluates every BlastShiftTo source pixel and
            // calls Random(BlastMatCount[mat]) unconditionally, even after
            // an earlier pixel shifted successfully (C4Landscape.cpp:941-960).
            let mut should_shift = false;
            for _ in 0..pixel_count {
                if i64::from(self.rng.random(total_pixels)) < threshold {
                    should_shift = true;
                }
            }

            if !should_shift || !candidate.apply_column_shift {
                continue;
            }
            if candidate.column < 0 {
                continue;
            }
            let column = candidate.column as u32;
            landscape.set_solid_material(column, Some(candidate.target));
        }
    }

    /// Object origin for the particle Attach offset (C4Particles.cpp:404-408
    /// subtracts the target object's position when the def has Attach set).
    fn particle_attach_origin(&self, layer: &ParticleLayer) -> Option<(i32, i32)> {
        match layer {
            ParticleLayer::Global => None,
            ParticleLayer::ObjectFront(id) | ParticleLayer::ObjectBack(id) => {
                self.find_object_index(*id).map(|index| {
                    let object = &self.objects[index];
                    (object.fixed_position.int_x(), object.fixed_position.int_y())
                })
            }
        }
    }

    pub(crate) fn apply_particle_commands(&mut self, commands: Vec<ParticleCommand>) {
        if commands.is_empty() {
            return;
        }
        for command in commands {
            match command {
                ParticleCommand::Create(config) => {
                    // Def-based path: full C4ParticleSystem::Create semantics.
                    // Def-less names keep the legacy fixture particle.
                    if self
                        .particle_system
                        .get_def(&config.definition_id)
                        .is_some()
                    {
                        let attach_origin = self.particle_attach_origin(&config.layer);
                        self.particle_system.create(
                            &config.definition_id.clone(),
                            config.position.x,
                            config.position.y,
                            config.velocity.x,
                            config.velocity.y,
                            config.parameter_a,
                            config.parameter_b,
                            config.layer,
                            attach_origin,
                        );
                    } else {
                        self.particles.push(ActiveParticle::from_config(config));
                    }
                }
                ParticleCommand::Cast {
                    definition_id,
                    amount,
                    x,
                    y,
                    level,
                    a0,
                    b0,
                    a1,
                    b1,
                    layer,
                } => {
                    let attach_origin = self.particle_attach_origin(&layer);
                    self.particle_system.cast(
                        &definition_id,
                        amount,
                        x,
                        y,
                        level,
                        a0,
                        b0,
                        a1,
                        b1,
                        layer,
                        attach_origin,
                    );
                }
                ParticleCommand::Push {
                    definition_id,
                    dxdir,
                    dydir,
                } => {
                    self.particle_system
                        .push(definition_id.as_deref(), dxdir, dydir);
                }
                ParticleCommand::ObjectFire(emission) => {
                    // "special effects only if loaded" (C4Effect.cpp:660-661):
                    // no fire defs, no draws off the SafeRandom stream.
                    if self.particle_system.is_fire_particle_loaded() {
                        self.particle_system.create_object_fire(&emission);
                    }
                }
                ParticleCommand::Clear {
                    definition_id,
                    scope,
                } => {
                    self.particle_system
                        .remove(definition_id.as_deref(), &scope);
                    let definition = definition_id.as_deref();
                    match scope {
                        ParticleScope::Global => {
                            self.particles.retain(|particle| {
                                if !matches!(particle.snapshot.layer, ParticleLayer::Global) {
                                    return true;
                                }
                                match definition {
                                    Some(def) => particle.snapshot.definition_id != def,
                                    None => false,
                                }
                            });
                        }
                        ParticleScope::Object(target) => {
                            self.particles.retain(|particle| {
                                let matches_layer = match particle.snapshot.layer {
                                    ParticleLayer::ObjectFront(id)
                                    | ParticleLayer::ObjectBack(id) => id == target,
                                    ParticleLayer::Global => false,
                                };
                                if !matches_layer {
                                    return true;
                                }
                                match definition {
                                    Some(def) => particle.snapshot.definition_id != def,
                                    None => false,
                                }
                            });
                        }
                    }
                }
            }
        }
    }

    /// `C4PXSSystem::Execute` (C4PXS.cpp:218-240): visit chunks in order,
    /// freeing each one only when its turn begins, then run every live PXS in
    /// slot order, IN PLACE. The slot stays occupied while its PXS executes,
    /// so a PXS created inside a reaction can never be handed the executing
    /// slot (New(), C4PXS.cpp:195-202).
    #[doc(hidden)]
    pub fn tick_pxs(&mut self) {
        // C4PXSSystem::Execute resets Count before visiting slots. Count is
        // incremented after every live slot's Execute, even if that call
        // deactivates the pixel (C4PXS.cpp:218-240).
        self.pxs_system.begin_execute();
        let mut inspected = 0_usize;
        for chunk in 0..pxs::PXS_MAX_CHUNK {
            self.pxs_system.free_empty_chunk(chunk);
            let mut cursor = chunk * pxs::PXS_CHUNK_SIZE;
            let end = cursor + pxs::PXS_CHUNK_SIZE;
            while let Some(index) = self.pxs_system.next_live_slot(cursor) {
                if index >= end {
                    break;
                }
                let slot = index % pxs::PXS_CHUNK_SIZE;
                cursor = index + 1;
                inspected += 1;
                let Some(pixel) = self.pxs_system.peek_slot(chunk, slot) else {
                    continue;
                };
                match self.execute_pxs_lifecycle(pixel) {
                    Ok(updated) => self.pxs_system.put_slot(chunk, slot, updated),
                    Err(deactivated) => {
                        self.pxs_system.deactivate_slot(chunk, slot, deactivated);
                    }
                }
                self.pxs_system.note_executed();
            }
        }
        self.pxs_system.note_inspected_slots(inspected);
    }

    /// `GBackMat` (C4Wrappers.h:164-167): the PXS step loop and reaction
    /// lookups read materials through the GetPix border rules — a closed
    /// side/bottom answers the Vehicle material, not sky.
    pub(crate) fn landscape_material(&self, x: i32, y: i32) -> Option<MaterialId> {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.border_material_at(x, y))
    }

    /// `MatValid` for a raw `C4PXS::Mat` (C4Wrappers.h:100-103): inside
    /// `[0, Material.Num - 1]`. A slot may legitimately hold something else
    /// between the write that put it there and the Execute guard that reads
    /// it, so this answers `None` rather than refusing to represent it.
    pub(crate) fn pxs_material_id(&self, mat: pxs::PxsMaterial) -> Option<MaterialId> {
        mat.id()
            .filter(|id| self.materials.get_by_id(*id).is_some())
    }

    fn pxs_material(&self, mat: pxs::PxsMaterial) -> Option<&crate::material::Material> {
        mat.id().and_then(|id| self.materials.get_by_id(id))
    }

    /// `C4PXS::Execute` (C4PXS.cpp:28-127). Returns the surviving PXS, or
    /// `None` when it deactivates.
    pub(crate) fn execute_pxs(&mut self, pixel: pxs::Pxs) -> Option<pxs::Pxs> {
        self.execute_pxs_lifecycle(pixel).ok()
    }

    /// Preserve the final mutated payload on the deactivation side while the
    /// public differential helper retains its historical `Option` shape.
    fn execute_pxs_lifecycle(&mut self, mut pixel: pxs::Pxs) -> Result<pxs::Pxs, pxs::Pxs> {
        // Frame first: this runs once per live PXS, and `env::var` takes a
        // global lock and allocates before it can answer.
        if (17..=19).contains(&self.frame) && std::env::var("LC_RUST_RNG_TRACE").is_ok() {
            crate::rng::rng_trace_line(
                self.rng.trace_index,
                &format!(
                    "PXS {} {} {} {} {} {}",
                    pixel.mat.raw(),
                    fixtoi_prec(pixel.x, 256),
                    fixtoi_prec(pixel.y, 256),
                    fixtoi_prec(pixel.xdir, 256),
                    fixtoi_prec(pixel.ydir, 256),
                    self.frame
                ),
            );
        }
        // Safety: MatValid(Mat) (C4PXS.cpp:46-50; C4Wrappers.h:100-103). A
        // raw index Load or a script reaction stored survives in the slot
        // until exactly here.
        if self.pxs_material(pixel.mat).is_none() {
            return Err(pixel);
        }
        // Out of bounds (C4PXS.cpp:45-49)
        let (back_wdt, back_hgt) = self
            .landscape
            .as_ref()
            .map(|landscape| (landscape.width() as i32, landscape.estimated_height()))
            .unwrap_or((0, 0));
        if pixel.x < C4Fixed::ZERO
            || pixel.x >= itofix(back_wdt)
            || pixel.y < itofix(-10)
            || pixel.y >= itofix(back_hgt)
        {
            return Err(pixel);
        }
        // Material conversion: meePXSPos check before movement (C4PXS.cpp:51-57)
        let mut ix = fixtoi(pixel.x);
        let mut iy = fixtoi(pixel.y);
        let inmat = self.landscape_material(ix, iy);
        let reaction = self.materials.reaction_for_event(
            self.pxs_material_id(pixel.mat),
            inmat,
            MaterialInteractionEvent::PxsPos,
        );
        if !matches!(reaction.kind, MaterialReactionKind::None) {
            // C++ passes nullptr for pfPosChanged at the PXSPos event; the
            // landscape position equals the PXS position here (C4PXS.cpp:55).
            let (ls_x, ls_y) = (ix, iy);
            let mut pos_changed = false;
            if self.execute_pxs_reaction(
                reaction,
                &mut ix,
                &mut iy,
                ls_x,
                ls_y,
                &mut pixel,
                inmat,
                MaterialInteractionEvent::PxsPos,
                &mut pos_changed,
            ) {
                return Err(pixel);
            }
        }
        // `Mat` is passed by reference to the PXSPos reaction. Both
        // mrfConvert and mrfScript may replace it, and C++ reads the density
        // and WindDrift from that replacement for this same tick
        // (C4PXS.cpp:59-80; C4Material.cpp:643-649, 822-832).
        let Some(material) = self.pxs_material(pixel.mat) else {
            return Err(pixel);
        };
        let density = material.density();
        let wind_drift_param = material.wind_drift();
        // Gravity (C4PXS.cpp:60)
        pixel.ydir += self.physics.gravity_as_c4fixed();
        // Free fall: wind drift with synced jitter (C4PXS.cpp:62-74). The
        // Random(1200) draws are unconditional in free fall; WindDrift only
        // scales the result.
        let below_density = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.density_at(ix, iy + 1, &self.materials))
            .unwrap_or(0);
        if below_density < density {
            let wind = self.wind_at(ix, iy);
            let txdir = itofix_prec(wind, 15) + fixed256(self.rng.random(1200) - 600);
            let tydir = fixed256(self.rng.random(1200) - 600);
            let wind_drift = (wind_drift_param - 20).max(0);
            // WindDrift_Factor = itofix(1, 800) (C4PXS.cpp:26)
            let factor = itofix_prec(1, 800);
            pixel.xdir += (txdir - pixel.xdir) * wind_drift * factor;
            pixel.ydir += (tydir - pixel.ydir) * wind_drift * factor;
        }
        // Target position (C4PXS.cpp:76-81)
        let ctcox = pixel.x + pixel.xdir;
        let ctcoy = pixel.y + pixel.ydir;
        let ito_x = fixtoi(ctcox);
        let ito_y = fixtoi(ctcoy);
        // In bounds + free path → move (C4PXS.cpp:83-89)
        // Inside<int32_t>(iToX, 0, GBackWdt - 1) / (iToY, 0, GBackHgt - 1)
        if ito_x >= 0
            && ito_x < back_wdt
            && ito_y >= 0
            && ito_y < back_hgt
            && self
                .landscape
                .as_ref()
                .map(|landscape| landscape.path_free(ix, iy, ito_x, ito_y, &self.materials))
                .unwrap_or(false)
        {
            pixel.x = ctcox;
            pixel.y = ctcoy;
            return Ok(pixel);
        }
        // Step toward the target (C4PXS.cpp:91-117), do-while
        loop {
            let in_x = ix + (ito_x - ix).signum();
            let in_y = iy + (ito_y - iy).signum();
            let inmat = self.landscape_material(in_x, in_y);
            let reaction = self.materials.reaction_for_event(
                self.pxs_material_id(pixel.mat),
                inmat,
                MaterialInteractionEvent::PxsMove,
            );
            if !matches!(reaction.kind, MaterialReactionKind::None) {
                let mut pos_changed = false;
                if self.execute_pxs_reaction(
                    reaction,
                    &mut ix,
                    &mut iy,
                    in_x,
                    in_y,
                    &mut pixel,
                    inmat,
                    MaterialInteractionEvent::PxsMove,
                    &mut pos_changed,
                ) {
                    // destructive contact
                    return Err(pixel);
                }
                if pos_changed {
                    // speed or position changed: stop moving for now
                    pixel.x = itofix(ix);
                    pixel.y = itofix(iy);
                    return Ok(pixel);
                }
                // reaction did nothing — continue movement
            }
            ix = in_x;
            iy = in_y;
            if ix == ito_x && iy == ito_y {
                break;
            }
        }
        // No contact: free movement (C4PXS.cpp:119-120)
        pixel.x = ctcox;
        pixel.y = ctcoy;
        Ok(pixel)
    }

    /// Reaction proc dispatch for the PXS events, mirroring the mrf*
    /// functions (C4Material.cpp:626-798). Returns true when the PXS dies
    /// (the C++ procs' return value).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_pxs_reaction(
        &mut self,
        reaction: MaterialReaction,
        x: &mut i32,
        y: &mut i32,
        ls_x: i32,
        ls_y: i32,
        pixel: &mut pxs::Pxs,
        ls_mat: Option<MaterialId>,
        event: MaterialInteractionEvent,
        pos_changed: &mut bool,
    ) -> bool {
        // Every reaction below reads the PXS material as a live map entry.
        // An earlier reaction in this same Execute may have written a raw
        // index back (C4Material.cpp:822); C++ then indexes its map out of
        // range, so answer the guard's verdict — deactivate — instead.
        let Some(pxs_mat) = self.pxs_material_id(pixel.mat) else {
            return true;
        };
        // mrfUserCheck (C4Material.cpp:612-625): user-defined reactions do
        // the splash/slide check up front, gated on CheckSlide; the ExecMask
        // gate was applied when the reaction table was built.
        if reaction.user_defined
            && reaction.insertion_check
            && event == MaterialInteractionEvent::PxsMove
            && !self.mrf_insert_check(
                x,
                y,
                &mut pixel.xdir,
                &mut pixel.ydir,
                pxs_mat,
                ls_mat,
                pos_changed,
            )
        {
            return false;
        }
        let user_defined = reaction.user_defined;
        match reaction.kind {
            MaterialReactionKind::None => false,
            // mrfConvert (C4Material.cpp:626-661)
            MaterialReactionKind::Convert { target, depth } => {
                if event != MaterialInteractionEvent::PxsPos && !user_defined {
                    // hardcoded InMatConvert has no collision proc; USER
                    // conversions also convert upon hitting materials
                    // (C4Material.cpp:629-634)
                    return false;
                }
                // Check depth (C4Material.cpp:638-650)
                let depth = depth.unwrap_or(0);
                if depth != 0 && self.landscape_material(*x, *y - depth) != ls_mat {
                    return false;
                }
                match target.filter(|id| self.materials.get_by_id(*id).is_some()) {
                    Some(target) => {
                        pixel.mat = target.into();
                        pixel.xdir = C4Fixed::ZERO;
                        pixel.ydir = C4Fixed::ZERO;
                        *pos_changed = true;
                        false
                    }
                    // Convert failure (target not loaded or sky): kill pix
                    None => true,
                }
            }
            // mrfPoof (C4Material.cpp:663-689)
            MaterialReactionKind::Poof => {
                // the body's insert check is `!fUserDefined`-gated
                // (C4Material.cpp:685-688); user entries did mrfUserCheck
                if !user_defined
                    && event == MaterialInteractionEvent::PxsMove
                    && !self.mrf_insert_check(
                        x,
                        y,
                        &mut pixel.xdir,
                        &mut pixel.ydir,
                        pxs_mat,
                        ls_mat,
                        pos_changed,
                    )
                {
                    // either splash or slide prevented interaction
                    return false;
                }
                // Always kill both landscape and PXS mat — a real
                // ExtractMaterial (C4Material.cpp:682) including its
                // CheckInstabilityRange (C4Landscape.cpp:1154).
                let _ = self.extract_material(ls_x, ls_y);
                if self.rng.rnd3() == 0 {
                    self.spawn_smoke(*x, *y, 3);
                }
                // !Rnd3() → "Pshshsh". Reuse the existing synchronized
                // draw as the presentation gate; emitting the command draws
                // no additional randomness.
                if self.rng.rnd3() == 0 {
                    self.emit_audio_command(AudioCommand::PlaySoundAt {
                        name: "Pshshsh".to_string(),
                        position: Vector2::new(*x, *y),
                    });
                }
                true
            }
            // mrfCorrode (C4Material.cpp:691-745)
            MaterialReactionKind::Corrode {
                corrosive_strength,
                corrode_resistance,
                corrosion_probability,
            } => {
                if event != MaterialInteractionEvent::PxsMove {
                    // No corrosion before movement (C4Material.cpp:696-698)
                    return false;
                }
                // `!fUserDefined`-gated body check (C4Material.cpp:719-722)
                if !user_defined
                    && !self.mrf_insert_check(
                        x,
                        y,
                        &mut pixel.xdir,
                        &mut pixel.ydir,
                        pxs_mat,
                        ls_mat,
                        pos_changed,
                    )
                {
                    return false;
                }
                let corroded = evaluate_corrosion(
                    corrosive_strength,
                    corrode_resistance,
                    corrosion_probability,
                    &mut self.rng,
                );
                if corroded {
                    // ClearBackPix (= ClearPix) IN PLACE, then
                    // CheckInstabilityRange at that exact pixel
                    // (C4Material.cpp:731-733).
                    let cleared = self
                        .landscape
                        .as_mut()
                        .map(|landscape| landscape.clear_pix(ls_x, ls_y))
                        .unwrap_or(false);
                    if !cleared {
                        // column-model fixture worlds keep the column removal
                        if let Some(landscape) = self.landscape.as_mut() {
                            let _ = landscape.extract_material_at(ls_x, ls_y);
                        }
                    }
                    self.check_instability_range(ls_x, ls_y);
                    // effect draws (C4Material.cpp:734-735): 1/5 smoke with a
                    // Random(3) size component, then the 1/20 sound draw
                    if self.rng.random(5) == 0 {
                        let level = 3 + self.rng.random(3);
                        self.spawn_smoke(*x, *y, level);
                    }
                    if self.rng.random(20) == 0 {
                        self.emit_audio_command(AudioCommand::PlaySoundAt {
                            name: "Corrode".to_string(),
                            position: Vector2::new(*x, *y),
                        });
                    }
                } else {
                    // Else: dead. C++ routes through the full InsertMaterial
                    // slide/reaction/thrust path (C4Material.cpp:737-740).
                    let _ = self.insert_material(pxs_mat, *x, *y, 0, 0);
                }
                true
            }
            // mrfIncinerate (C4Material.cpp:747-771)
            MaterialReactionKind::Incinerate => {
                if event == MaterialInteractionEvent::PxsMove
                    && !self.mrf_insert_check(
                        x,
                        y,
                        &mut pixel.xdir,
                        &mut pixel.ydir,
                        pxs_mat,
                        ls_mat,
                        pos_changed,
                    )
                {
                    return false;
                }
                #[cfg(test)]
                MATERIAL_INCINERATE_PROBES.with(|probes| {
                    probes.borrow_mut().push((*x, *y));
                });
                let can_incinerate = self
                    .landscape
                    .as_ref()
                    .map(|landscape| landscape.can_incinerate(*x, *y, &self.materials))
                    .unwrap_or(false);
                if can_incinerate && self.spawn_fire_at(*x, *y) {
                    return true;
                }
                if event == MaterialInteractionEvent::PxsMove {
                    // Else: dead. C++ routes through the full InsertMaterial
                    // slide/reaction/thrust path (C4Material.cpp:765-767).
                    let _ = self.insert_material(pxs_mat, *x, *y, 0, 0);
                    return true;
                }
                false
            }
            // mrfScript (C4Material.cpp:800-835): mrfUserCheck already ran
            // in the prologue (Script entries are always user-defined); a
            // missing/unresolvable function is a no-op (null pScriptFunc).
            // X/Y/XDir/YDir/PxsMat go in by reference (GetRef pars at slots
            // 0/1/4/5/6, :814-815); a truthy return kills the PXS.
            MaterialReactionKind::Script { func } => {
                let Some(function) = self
                    .materials
                    .script_reaction_name(func)
                    .map(str::to_string)
                else {
                    return false;
                };
                // fixtoi(fDir, 100) (C4Material.cpp:812-813)
                let xdir1 = math::fixtoi_prec(pixel.xdir, 100);
                let ydir1 = math::fixtoi_prec(pixel.ydir, 100);
                let args = [
                    Value::Int(*x),
                    Value::Int(*y),
                    Value::Int(ls_x),
                    Value::Int(ls_y),
                    Value::Int(xdir1),
                    Value::Int(ydir1),
                    Value::Int(pixel.mat.raw()),
                    Value::Int(
                        ls_mat.map(|id| id.index() as i32).unwrap_or(-1), // MNone
                    ),
                    Value::Int(event.index() as i32),
                ];
                let Some((value, finals)) = self.call_material_reaction_script(&function, &args)
                else {
                    return false;
                };
                // `if (pScriptFunc->Exec(...)) return true;` — raw C4Value
                // truthiness kills the PXS (C4Material.cpp:818)
                if !matches!(value, Value::Nil | Value::Int(0) | Value::Bool(false)) {
                    return true;
                }
                // Write back parameters (C4Material.cpp:822-832).
                let final_int =
                    |index: usize| finals.get(index).and_then(Value::as_c4_int).unwrap_or(0);
                // iPxsMat writes back UNCONDITIONALLY (C4Material.cpp:822),
                // whatever the reaction returned. The slot keeps that raw
                // index; the MatValid guard is what rejects an invalid one.
                pixel.mat = pxs::PxsMaterial::from_raw(final_int(6));
                let (x2, y2) = (final_int(0), final_int(1));
                let (xdir2, ydir2) = (final_int(4), final_int(5));
                if *x != x2 || *y != y2 || xdir1 != xdir2 || ydir1 != ydir2 {
                    // changes to pos/speed detected
                    *pos_changed = true;
                    *x = x2;
                    *y = y2;
                    pixel.xdir = math::fixed100(xdir2);
                    pixel.ydir = math::fixed100(ydir2);
                }
                false
            }
            // mrfInsert (C4Material.cpp:773-798)
            MaterialReactionKind::Insert => {
                if event != MaterialInteractionEvent::PxsMove {
                    return false;
                }
                // `!fUserDefined`-gated body check (C4Material.cpp:783-787)
                if !user_defined
                    && !self.mrf_insert_check(
                        x,
                        y,
                        &mut pixel.xdir,
                        &mut pixel.ydir,
                        pxs_mat,
                        ls_mat,
                        pos_changed,
                    )
                {
                    // continue existing
                    return false;
                }
                // Else: dead. Insert material here (C4Material.cpp:789 →
                // C4Landscape::InsertMaterial full port)
                let (ix, iy, mat) = (*x, *y, pxs_mat);
                self.insert_material(mat, ix, iy, 0, 0);
                true
            }
        }
    }

    /// One mass-move reaction (meeMassMove) through the engine, so reactions
    /// that need engine state can run: `Type=Script` functions (mrfScript,
    /// C4MassMover.cpp:163-167: xdir=ydir=Fix0, pfPosChanged=nullptr — the
    /// by-ref write-backs land in discarded locals; only a truthy return
    /// matters, consuming the material), and Incinerate's FLAM object query
    /// and creation (C4Landscape.cpp:1417-1427). Other builtin kinds delegate
    /// to the MaterialSet path unchanged.
    pub(crate) fn execute_mass_move_reaction(
        &mut self,
        pxs_material: MaterialId,
        pxs_x: i32,
        pxs_y: i32,
        landscape_x: i32,
        landscape_y: i32,
    ) -> material::MaterialReactionExecution {
        let landscape_material = self
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.border_material_at(landscape_x, landscape_y));
        let reaction = self.materials.reaction_for_event(
            Some(pxs_material),
            landscape_material,
            MaterialInteractionEvent::MassMove,
        );
        if let MaterialReactionKind::Script { func } = reaction.kind {
            let Some(function) = self
                .materials
                .script_reaction_name(func)
                .map(str::to_string)
            else {
                return material::MaterialReactionExecution::Unhandled;
            };
            let args = [
                Value::Int(pxs_x),
                Value::Int(pxs_y),
                Value::Int(landscape_x),
                Value::Int(landscape_y),
                Value::Int(0),
                Value::Int(0),
                Value::Int(pxs_material.index() as i32),
                Value::Int(
                    landscape_material.map(|id| id.index() as i32).unwrap_or(-1), // MNone
                ),
                Value::Int(MaterialInteractionEvent::MassMove.index() as i32),
            ];
            return match self.call_material_reaction_script(&function, &args) {
                Some((value, _finals))
                    if !matches!(value, Value::Nil | Value::Int(0) | Value::Bool(false)) =>
                {
                    material::MaterialReactionExecution::Consumed
                }
                _ => material::MaterialReactionExecution::Unhandled,
            };
        }
        if reaction.kind == MaterialReactionKind::Incinerate {
            return if self.spawn_fire_at(pxs_x, pxs_y) {
                material::MaterialReactionExecution::Consumed
            } else {
                material::MaterialReactionExecution::Unhandled
            };
        }
        let mut instability_probes = Vec::new();
        let mut smoke_request = None;
        let mut sound_request = None;
        let result = {
            let Some(landscape) = self.landscape.as_mut() else {
                return material::MaterialReactionExecution::Unhandled;
            };
            self.materials.execute_mass_move_reaction_with_smoke(
                landscape,
                pxs_material,
                pxs_x,
                pxs_y,
                landscape_x,
                landscape_y,
                &mut self.rng,
                &mut instability_probes,
                &mut smoke_request,
                &mut sound_request,
            )
        };
        // The CheckInstabilityRange half of each ExtractMaterial the
        // reaction ran (C4Landscape.cpp:1154).
        for (probe_x, probe_y) in instability_probes {
            self.check_instability_range(probe_x, probe_y);
        }
        if let Some(request) = smoke_request {
            self.spawn_smoke(request.x, request.y, request.level);
        }
        if let Some(request) = sound_request {
            self.emit_audio_command(AudioCommand::PlaySoundAt {
                name: request.name.to_string(),
                position: Vector2::new(request.x, request.y),
            });
        }
        result
    }

    /// Runs a `Type=Script` material reaction function (mrfScript,
    /// C4Material.cpp:800-835). C++ resolves and retains an SFunc from
    /// Game.ScriptEngine, so only linked `global func` entries qualify and
    /// execution must keep the declaring host's local-helper scope. Returns
    /// the raw return value, `None` when the function is unresolvable
    /// (null `pScriptFunc` → the reaction is a no-op) — script ERRORS return
    /// `Some(Value::Nil)` semantics via the fail-safe exec.
    fn call_material_reaction_script(
        &mut self,
        function: &str,
        args: &[Value],
    ) -> Option<(Value, Vec<Value>)> {
        if function.is_empty()
            || !self
                .global_script_functions
                .as_deref()
                .is_some_and(|functions| functions.contains_key(function))
        {
            return None;
        }
        let world = self.host_world_context();
        let (script, resolution) = world.resolve_engine_global_script(function)?;
        let rng = self.rng.clone();
        let (value, finals, batch, audio_state, rng, script_error) =
            ScenarioScript::execute_value_for_script(
                "Game.ScriptEngine",
                None,
                function,
                args,
                world,
                rng,
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
                || script.call_resolved_with_ref_args(&resolution, true, args),
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        if let Err(error) = self.apply_scenario_batch(batch) {
            tracing::warn!(%error, "material reaction script batch failed to apply");
        }
        // Ordinary raw callback errors were already logged by
        // `call_value_for_script`.
        let _ = script_error;
        Some((value.unwrap_or(Value::Nil), finals))
    }

    /// `Smoke()` (C4Effect.cpp:859-865): create a "Smoke" particle if the def
    /// is loaded. (The FXS1 object fallback for missing particle defs is not
    /// ported.) `level/2` is integer division like the C++ call.
    pub(crate) fn spawn_smoke(&mut self, x: i32, y: i32, level: i32) {
        self.particle_system.create(
            "Smoke",
            x as f32,
            y as f32 - (level / 2) as f32,
            0.0,
            0.0,
            level as f32,
            0,
            ParticleLayer::Global,
            None,
        );
    }

    /// `mrfInsertCheck` (C4Material.cpp:567-610): splash/slide preamble run by
    /// the default Poof/Corrode/Incinerate/Insert reactions on the PXS-move
    /// event. Returns true when insertion may proceed; false keeps the PXS
    /// alive (splashed or sliding). Mutates pos/speed like the C++ by-ref
    /// parameters.
    #[allow(clippy::too_many_arguments)]
    /// C4Landscape::InsertMaterial (C4Landscape.cpp:1158-1223): select the
    /// scenario's primitive or push-pull destination path, then run the
    /// FindMatSlide loop, reaction below (meePXSPos), and dead-material write
    /// with the insert-thrust recursion.
    pub(crate) fn insert_material(
        &mut self,
        mut mat: MaterialId,
        tx: i32,
        ty: i32,
        vx: i32,
        vy: i32,
    ) -> bool {
        if (15..=19).contains(&self.frame) && std::env::var("LC_RUST_RNG_TRACE").is_ok() {
            crate::rng::rng_trace_line(
                self.rng.trace_index,
                &format!("INSMAT {} {tx} {ty} {vx} {vy} {}", mat.index(), self.frame),
            );
        }
        let Some(material) = self.materials.get_by_id(mat) else {
            return false;
        };
        let mdens = material.density();
        if mdens == 0 {
            return true;
        }
        let max_slide = material.max_slide();
        let instable = material.instable();
        let landscape_push_pull = self.scenario_values.landscape_push_pull();
        let Some(destination) = self.landscape.as_ref().and_then(|landscape| {
            landscape.insert_material_destination(
                tx,
                ty,
                mdens,
                landscape_push_pull,
                max_slide,
                instable,
                &self.materials,
            )
        }) else {
            return false;
        };
        let (mut tx, mut ty) = match destination {
            landscape::InsertMaterialDestination::Column => {
                return self
                    .landscape
                    .as_mut()
                    .is_some_and(|landscape| landscape.insert_material_at(tx, ty, mat));
            }
            landscape::InsertMaterialDestination::Grid { x, y } => (x, y),
        };
        let density_at = |engine: &Self, x: i32, y: i32| -> i32 {
            engine
                .landscape
                .as_ref()
                .map(|landscape| landscape.density_at(x, y, &engine.materials))
                .unwrap_or(0)
        };
        // Try slide: while a slide position exists and the pixel below is
        // free, the material continues as PXS (C4Landscape.cpp:1178-1183)
        loop {
            let slid = self
                .landscape
                .as_ref()
                .map(|landscape| {
                    let (mut sx, mut sy) = (tx, ty);
                    let ok = landscape.find_mat_slide(
                        &mut sx,
                        &mut sy,
                        1,
                        mdens,
                        max_slide,
                        &self.materials,
                    );
                    (ok, sx, sy)
                })
                .unwrap_or((false, tx, ty));
            if !slid.0 {
                break;
            }
            tx = slid.1;
            ty = slid.2;
            if density_at(self, tx, ty + 1) < mdens {
                self.pxs_system.create(
                    &self.materials,
                    mat,
                    itofix(tx),
                    itofix(ty),
                    fixed10(vx),
                    fixed10(vy),
                );
                return true;
            }
        }
        // Try the reaction in gravity direction. The preceding slide remains
        // hardcoded downward, but C++ probes `ty + Sign(GravAccel)` here
        // (C4Landscape.cpp:1185-1193).
        let reaction_y = ty + self.physics.gravity_as_c4fixed().val().signum();
        let reaction_mat = self.landscape_material(tx, reaction_y);
        let reaction = self.materials.reaction_for_event(
            Some(mat),
            reaction_mat,
            MaterialInteractionEvent::PxsPos,
        );
        if !matches!(reaction.kind, MaterialReactionKind::None) || reaction.user_defined {
            let mut probe = pxs::Pxs {
                mat: mat.into(),
                x: itofix(tx),
                y: itofix(ty),
                xdir: fixed10(vx),
                ydir: fixed10(vy),
            };
            let mut pos_changed = false;
            let (mut rx, mut ry) = (tx, ty);
            if self.execute_pxs_reaction(
                reaction,
                &mut rx,
                &mut ry,
                tx,
                reaction_y,
                &mut probe,
                reaction_mat,
                MaterialInteractionEvent::PxsPos,
                &mut pos_changed,
            ) {
                // the material to be inserted killed itself in some
                // material reaction in gravity direction
                return true;
            }
            // InsertMaterial passes tx, ty and mat by reference. A false
            // reaction result keeps the material alive with every write-back
            // applied before the dead-pixel SetPix (C4Landscape.cpp:1198-1218).
            tx = rx;
            ty = ry;
            // mrfScript may have written a raw index back. C++ carries it
            // into the insert and indexes its material map with it; keep the
            // last representable one rather than repeating that read.
            mat = self.pxs_material_id(probe.mat).unwrap_or(mat);
        }
        // Insert dead material, keeping the current pixel's IFT. C++ only
        // captures and re-inserts the displaced material when the runtime
        // LandscapeInsertThrust flag is enabled (C4Landscape.cpp:1197-1206).
        let old_mat = if self.landscape_insert_thrust {
            self.landscape_material(tx, ty)
        } else {
            None
        };
        if let Some(landscape) = self.landscape.as_mut() {
            landscape.insert_material_pix(tx, ty, mat);
        }
        if let Some(old_mat) = old_mat {
            self.insert_material(old_mat, tx, ty.wrapping_sub(1), 0, 0);
        }
        true
    }

    #[doc(hidden)]
    pub fn mrf_insert_check(
        &mut self,
        x: &mut i32,
        y: &mut i32,
        xdir: &mut C4Fixed,
        ydir: &mut C4Fixed,
        pxs_mat: MaterialId,
        ls_mat: Option<MaterialId>,
        pos_changed: &mut bool,
    ) -> bool {
        // always manipulating pos/speed here (C4Material.cpp:570)
        *pos_changed = true;
        let Some(material) = self.materials.get_by_id(pxs_mat) else {
            return true;
        };
        let splash_rate = material.splash_rate();
        let incindiary = material.incindiary();
        let density = material.density();
        let max_slide = material.max_slide();

        // Rough contact? May splash (C4Material.cpp:572-579)
        if *ydir > itofix(1) && splash_rate != 0 && self.rng.random(splash_rate) == 0 {
            *ydir = -*ydir / 8;
            *xdir = *xdir / 8 + fixed100(self.rng.random(200) - 100);
            if ydir.is_nonzero() {
                return false;
            }
        }

        // Contact: Stop (C4Material.cpp:581-582)
        *ydir = C4Fixed::ZERO;

        // Incindiary mats smoke on contact even before doing their slide
        // (C4Material.cpp:584-586). Rnd3 is consumed as the call argument.
        if incindiary != 0 && self.rng.random(25) == 0 {
            let level = 4 + self.rng.rnd3();
            self.spawn_smoke(*x, *y, level);
        }

        // Move by mat path/slide (C4Material.cpp:588-607)
        let gravity_sign = self.physics.gravity_as_c4fixed().val().signum();
        let (mut slide_x, mut slide_y) = (*x, *y);
        let found_slide = self
            .landscape
            .as_ref()
            .map(|landscape| {
                landscape.find_mat_slide(
                    &mut slide_x,
                    &mut slide_y,
                    gravity_sign,
                    density,
                    max_slide,
                    &self.materials,
                )
            })
            .unwrap_or(false);
        if found_slide {
            if Some(pxs_mat) == ls_mat {
                *x = slide_x;
                *y = slide_y;
                *xdir = C4Fixed::ZERO;
                return false;
            }
            // Accelerate into the direction (C4Material.cpp:597)
            *xdir = C4Fixed::from_raw(
                (xdir.val().wrapping_mul(10) + itofix((slide_x - *x).signum()).val()) / 11,
            ) + fixed10(self.rng.random(5) - 2);
            // Slide target in range? Move there directly. (C4Material.cpp:599-604)
            if (*x - slide_x).abs() <= fixtoi(*xdir).abs() {
                *x = slide_x;
                *y = slide_y;
                if *ydir <= C4Fixed::ZERO {
                    *xdir = C4Fixed::ZERO;
                }
            }
            // Continue existance
            return false;
        }
        // insertion OK
        true
    }

    pub(crate) fn spawn_fire_at(&mut self, x: i32, y: i32) -> bool {
        if !self
            .landscape
            .as_ref()
            .map(|landscape| landscape.can_incinerate(x, y, &self.materials))
            .unwrap_or(false)
        {
            return false;
        }

        if !self.definitions.contains_key(FIRE_DEFINITION_ID) {
            return false;
        }

        let left = x.saturating_sub(4);
        let right = left.saturating_add(8);
        let top = y.saturating_sub(1);
        let bottom = top.saturating_add(20);

        let has_existing = self.objects.iter().any(|object| {
            if object.destroyed {
                return false;
            }
            if object.definition_id != FIRE_DEFINITION_ID {
                return false;
            }
            if !object.state.status.is_active() {
                return false;
            }
            let pos = object.state.position;
            pos.x >= left && pos.x < right && pos.y >= top && pos.y < bottom
        });

        if has_existing {
            return false;
        }

        match self.spawn_object_with_initial_lifecycle(
            SpawnConfig::new(FIRE_DEFINITION_ID).with_position(Vector2::new(x, y)),
            None,
        ) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => false,
        }
    }

    #[doc(hidden)]
    pub fn apply_transfer_zone_commands(
        &mut self,
        commands: Vec<TransferZoneCommand>,
    ) -> Result<(), EngineError> {
        for command in commands {
            match command {
                TransferZoneCommand::Set { owner, rect } => {
                    // A zone whose owner vanished before the deferred apply
                    // cannot exist in C++ (C4TransferZones entries die with
                    // their object) — drop it instead of aborting the batch.
                    match self.set_transfer_zone(owner, rect) {
                        Ok(()) => {}
                        Err(EngineError::UnknownObject(missing)) => {
                            tracing::warn!(
                                owner = missing.as_u64(),
                                "transfer zone owner vanished before the deferred apply; dropped"
                            );
                        }
                        Err(error) => return Err(error),
                    }
                }
                TransferZoneCommand::Clear { owner } => {
                    self.transfer_zones.clear(owner);
                }
            }
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn set_transfer_zone(
        &mut self,
        owner: ObjectId,
        rect: TransferZoneRect,
    ) -> Result<(), EngineError> {
        if !self.objects.iter().any(|object| object.id == owner) {
            return Err(EngineError::UnknownObject(owner));
        }
        self.transfer_zones.set(owner, rect);
        Ok(())
    }

    pub(crate) fn tick_particles(&mut self) {
        // Legacy def-less fixture particles (additive command-DSL path).
        if !self.particles.is_empty() {
            for particle in &mut self.particles {
                particle.tick();
            }
            self.particles.retain(|particle| !particle.is_expired());
        }
        // C4ParticleSystem exec: each object's Back then Front list
        // (C4Object.cpp:1071-1072), then GlobalParticles (C4Game.cpp:814).
        if self.particle_system.particles().is_empty() {
            return;
        }
        let gravity = self.physics.gravity_as_c4fixed();
        let frame_counter = self.frame as i32;
        let wind_force = self.environment.wind_force(self.frame);
        // GBackWdt/GBackHgt; the Rust landscape is a height-map model, so the
        // world height is the estimated extent rather than a pixel-map height.
        let (back_wdt, back_hgt) = self
            .landscape
            .as_ref()
            .map(|landscape| (landscape.width() as i32, landscape.estimated_height()))
            .unwrap_or((0, 0));
        let attached_ids: HashSet<ObjectId> = self
            .particle_system
            .particles()
            .iter()
            .filter_map(|particle| match particle.layer {
                ParticleLayer::ObjectFront(id) | ParticleLayer::ObjectBack(id) => Some(id),
                ParticleLayer::Global => None,
            })
            .collect();
        let targets: Vec<(ObjectId, particles::ParticleTarget)> = self
            .objects
            .iter()
            .filter(|object| attached_ids.contains(&object.id))
            .map(|object| {
                (
                    object.id,
                    particles::ParticleTarget {
                        x: object.fixed_position.int_x(),
                        y: object.fixed_position.int_y(),
                        xdir: object.fixed_velocity.x,
                        ydir: object.fixed_velocity.y,
                    },
                )
            })
            .collect();
        let landscape = self.landscape.as_ref();
        let solid = move |x: i32, y: i32| {
            landscape
                .map(|landscape| landscape.is_solid_at(x, y))
                .unwrap_or(false)
        };
        // GBackWind (C4Wrappers.h:189-192): IFT/tunnel-background pixels
        // suppress wind for both standard drift and smoke particles.
        let wind = move |x: i32, y: i32| {
            if landscape.is_some_and(|landscape| landscape.is_ift_at(x, y)) {
                0
            } else {
                wind_force
            }
        };
        let env = particles::ParticleEnv {
            gravity,
            frame_counter,
            back_wdt,
            back_hgt,
            solid: &solid,
            wind: &wind,
        };
        let mut system = std::mem::take(&mut self.particle_system);
        for (id, target) in targets {
            system.exec_layer(&ParticleLayer::ObjectBack(id), Some(target), &env);
            system.exec_layer(&ParticleLayer::ObjectFront(id), Some(target), &env);
        }
        system.exec_layer(&ParticleLayer::Global, None, &env);
        self.particle_system = system;
    }

    /// `pGlobalEffects->Execute(nullptr)` (C4Game.cpp:830-831): the global
    /// effect list executes right after ExecObjects — C4Effect::Execute
    /// (C4Effect.cpp:319-363) advances every live effect's iTime and fires
    /// `Fx*Timer(nil, iNumber, iTime)` on elapsed intervals. Callback
    /// outcomes fold exactly like the object timer batch.
    pub(crate) fn tick_global_effects(&mut self) -> Result<(), EngineError> {
        let mut cursor = None;
        while let Some((next_cursor, timer_event)) =
            advance_effect_frame_cursor(&mut self.global_effects, cursor)
        {
            cursor = Some(next_cursor);
            let Some(event) = timer_event else {
                continue;
            };

            self.record_effect_dispatch(|stats| stats.global_timer_events += 1);
            self.dispatch_global_effect_events(vec![event])?;
        }
        Ok(())
    }

    /// Runs one already-selected batch from the global effect list and folds
    /// every callback side channel back into the live engine.
    pub(crate) fn dispatch_global_effect_events(
        &mut self,
        events: Vec<EffectEvent>,
    ) -> Result<(), EngineError> {
        let world = self.host_world_context();
        let rng_state = self.rng.clone();
        let mut global_effects = std::mem::take(&mut self.global_effects);
        let outcome = Self::run_effect_events_for_global(
            self.game_over_triggered,
            rng_state,
            events,
            &mut global_effects,
            &mut self.environment,
            self.physics,
            self.frame,
            world,
            self.audio_registry.clone(),
        );
        self.global_effects = global_effects;
        let GlobalEffectRunOutcome {
            particles,
            physics_delta,
            audio_events,
            messages,
            player_commands,
            object_order_commands,
            next_mission_commands,
            landscape_ops,
            solid_mask_operations,
            host_raster_preview,
            transfer_zones,
            spawns,
            other_objects,
            object_lists,
            next_object_id,
            game_over,
            script_go,
            script_counter,
            audio_state,
            rng,
        } = outcome?;
        let was_deferred = self.solid_mask_staging.defer_solid_mask_updates;
        let mut outermost =
            self.stage_host_solid_mask_operations(solid_mask_operations, host_raster_preview);
        let fold_result = (|| -> Result<(), EngineError> {
            self.rng = rng;
            self.audio_registry = audio_state;
            self.sync_next_object_id(next_object_id);
            if !spawns.is_empty() {
                self.process_spawn_queue(spawns)?;
            }
            if !transfer_zones.is_empty() {
                self.apply_transfer_zone_commands(transfer_zones)?;
            }
            if !other_objects.is_empty() {
                self.apply_nested_object_outcomes(other_objects)?;
            }
            if let Some(preview) = object_lists {
                self.install_effect_object_lists(preview);
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
                self.emit_audio_commands(audio_events);
            }
            for command in messages {
                self.messages.apply_command(command);
            }
            if let Some(go) = script_go {
                self.scenario_script_go = go;
            }
            if let Some(counter) = script_counter {
                self.scenario_script_counter = counter;
            }
            if game_over {
                self.request_game_over()?;
            }
            if !physics_delta.is_empty() {
                self.apply_physics_delta(physics_delta);
            }
            self.apply_particle_commands(particles);
            Ok(())
        })();
        outermost |= !was_deferred && self.solid_mask_staging.defer_solid_mask_updates;
        self.finish_host_solid_mask_operations(outermost, fold_result)
    }

    /// `C4Effect::ClearAll(nullptr, C4FxCall_RemoveClear)` for the global
    /// effect list (C4Game.cpp:4202-4208; C4Effect.cpp:407-425). The traversal
    /// is captured tail-first, but each node is resolved live when its turn is
    /// reached. Effects created by a Stop callback are therefore outside this
    /// clear, while the original nodes stay linked dead for the next Execute.
    pub(crate) fn clear_global_effects_for_scenario_section(&mut self) -> Result<(), EngineError> {
        let events = self
            .global_effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .rev()
            .cloned()
            .map(|effect| EffectEvent::stopped(effect, EffectStopReason::Cleared))
            .collect::<Vec<_>>();
        self.dispatch_global_effect_events(events)
    }

    /// Executes deferred Fx* events of the GLOBAL effect list — the
    /// nil-object analog of [`Self::run_effect_events_for_object`]:
    /// callbacks receive nil as the affected object (C4Effect::Execute
    /// passes pObj=nullptr, C4Effect.cpp:345) and resolve against the
    /// command target's def script, the command-id def script, or the
    /// engine-global function table (C4Effect::DoCall, C4Effect.cpp:
    /// 439-456).
    #[allow(clippy::too_many_arguments)]
    fn run_effect_events_for_global(
        game_over_triggered: bool,
        mut rng: LcgRng,
        events: Vec<EffectEvent>,
        global_effects: &mut Vec<EffectState>,
        environment: &mut EnvironmentSettings,
        physics: PhysicsSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<GlobalEffectRunOutcome, EngineError> {
        let mut world = world;
        let mut pending_spawns: Vec<SpawnConfig> = Vec::new();
        let mut queue: VecDeque<EffectEvent> = VecDeque::from(events);
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
        let mut pending_other_objects = Vec::new();
        let mut pending_object_lists = None;
        let mut pending_solid_mask_operations = Vec::new();
        let mut game_over_requested = false;
        let mut script_go_requested: Option<bool> = None;
        let mut script_counter_requested: Option<i32> = None;
        // Anchors whose temp remove/readd bracket was already queued (a
        // re-popped anchor event must not expand again); see the object
        // runner's temp_wrapped_stopped.
        let mut temp_wrapped_stopped: HashSet<i32> = HashSet::new();

        while let Some(mut event) = queue.pop_front() {
            if matches!(event.kind, EffectEventKind::Timer)
                && !global_effects
                    .iter()
                    .any(|effect| effect.number == event.effect.number && effect.priority != 0)
            {
                continue;
            }
            // C4Effect::Kill (C4Effect.cpp:365-405): the real removal is
            // bracketed by temp-deactivating all upper effects
            // (C4Effect.cpp:370-374) and reactivating them after the Stop
            // (C4Effect.cpp:404); priority-1 victims skip the bracket
            // (C4Effect.cpp:477).
            if matches!(
                event.kind,
                EffectEventKind::Stopped(EffectStopReason::Removed)
            ) && event.effect.priority != 1
                && !temp_wrapped_stopped.contains(&event.effect.number)
            {
                let uppers = upper_effects_of(global_effects, &event.effect);
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
            match event.kind {
                EffectEventKind::TempRemoved => {
                    let Some(effect) = global_effects
                        .iter_mut()
                        .find(|effect| effect.number == event.effect.number)
                    else {
                        continue;
                    };
                    // This recursive TempRemoveUpperEffects frame was entered
                    // while the node was active. A higher Stop callback may
                    // kill it before the frame resumes, but C++ still applies
                    // FlipActive (zero remains zero) and dispatches FxStop
                    // (C4Effect.cpp:480-490).
                    effect.priority = -effect.priority;
                    event.effect = effect.clone();
                    if event.effect.priority == 1 {
                        continue;
                    }
                }
                EffectEventKind::TempReadded => {
                    let Some(effect) = global_effects
                        .iter_mut()
                        .find(|effect| effect.number == event.effect.number && effect.priority < 0)
                    else {
                        continue;
                    };
                    effect.priority = -effect.priority;
                    event.effect = effect.clone();
                }
                _ => {}
            }
            let clear_all_stop = matches!(
                event.kind,
                EffectEventKind::Stopped(EffectStopReason::Cleared)
            );
            if clear_all_stop {
                // ClearAll reaches this node only after recursively processing
                // its original successor. Re-read it now: an upper Stop may
                // have removed, renamed, or otherwise replaced the node while
                // the recursion unwound (C4Effect.cpp:407-425).
                let Some(effect) = global_effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number && effect.priority != 0)
                else {
                    continue;
                };
                event.effect = effect.clone();
                effect.priority = 0;
            }
            // C++ runs Fx* callbacks with fPassErrors=false (fail-safe
            // exec); RNG/audio restore from the pre-call backups on the
            // error path like the object runner.
            let rng_backup = rng.clone();
            let audio_backup = current_audio.clone();
            let mut timer_kill = false;
            let mut stop_denied = false;
            let call_result = match event.kind {
                EffectEventKind::Timer => dispatch_global_effect_callback(
                    &event.effect,
                    "Timer",
                    "FxTimer",
                    vec![Value::Int(event.effect.timer)],
                    rng,
                    global_effects,
                    current_physics,
                    current_environment,
                    frame,
                    world.clone(),
                    game_over_triggered,
                    current_audio,
                )
                .map(|(outcome, audio_state, new_rng, timer_result)| {
                    // C4Effect::Execute (C4Effect.cpp:342-357): Fx*Timer
                    // returning C4Fx_Execute_Kill (-1, C4Effects.h:40)
                    // kills the effect; an elapsed interval with NO
                    // timer function kills too (:355-357).
                    timer_kill = timer_result
                        .as_ref()
                        .is_none_or(|value| compat::value_as_i32(value) == -1);
                    (outcome, audio_state, new_rng)
                }),
                EffectEventKind::Stopped(reason) => dispatch_global_effect_callback(
                    &event.effect,
                    "Stop",
                    "FxStop",
                    effect_stop_reason_value(reason).map_or_else(Vec::new, |value| vec![value]),
                    rng,
                    global_effects,
                    current_physics,
                    current_environment,
                    frame,
                    world.clone(),
                    game_over_triggered,
                    current_audio,
                )
                .map(|(outcome, audio_state, new_rng, stop_result)| {
                    // C4Fx_Stop_Deny (-1, C4Effects.h:42): the effect
                    // refuses its removal and recovers
                    // (C4Effect.cpp:389-396).
                    stop_denied = matches!(
                        reason,
                        EffectStopReason::Removed | EffectStopReason::Cleared
                    ) && stop_result
                        .as_ref()
                        .is_some_and(|value| compat::value_as_i32(value) == -1);
                    (outcome, audio_state, new_rng)
                }),
                EffectEventKind::TempRemoved => dispatch_global_effect_callback(
                    &event.effect,
                    "Stop",
                    "FxStop",
                    vec![Value::Int(1), Value::Bool(true)],
                    rng,
                    global_effects,
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
                EffectEventKind::TempReadded => dispatch_global_effect_callback(
                    &event.effect,
                    "Start",
                    "FxStart",
                    vec![Value::Int(1)],
                    rng,
                    global_effects,
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
                // Started runs synchronously inside FnAddEffect for global
                // effects; Check/AddTo (the priority check chain) are not
                // generated for the global list (documented residual).
                _ => continue,
            };
            let (event_outcome, audio_state, new_rng) = match call_result {
                Ok(value) => value,
                Err(EngineError::Script {
                    definition,
                    function,
                    source,
                    recovery: _,
                }) => {
                    tracing::error!(
                        %definition,
                        function,
                        error = %source,
                        "script error in global effect callback; continuing like the C++ fail-safe exec"
                    );
                    log_runtime_call_frames(&definition, source.call_frames());
                    rng = rng_backup;
                    current_audio = audio_backup;
                    let _ = world.take_effect_spawn_previews();
                    continue;
                }
                Err(other) => return Err(other),
            };
            rng = new_rng;
            current_audio = audio_state;
            if stop_denied
                && !global_effects
                    .iter()
                    .any(|effect| effect.number == event.effect.number)
            {
                // Preserve EffectVar writes from a Stop denial even for
                // legacy deferred removals that unlinked before dispatch.
                insert_effect_into_stack(global_effects, event.effect.clone());
            }
            let compat::EffectContextOutcome {
                global: global_effect_commands,
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
                spawns,
                next_object_id,
                other_objects: event_other_objects,
                object_lists: event_object_lists,
                ..
            } = event_outcome;

            if let Some(preview) = event_host_raster_preview {
                world.apply_host_raster_preview(preview);
            } else {
                world.preview_solid_mask_operations(&event_solid_mask_operations);
            }
            pending_solid_mask_operations.extend(event_solid_mask_operations);

            for command in &event_transfer_zones {
                world.preview_transfer_zone_command(command);
            }
            pending_transfer_zones.extend(event_transfer_zones);

            let spawn_previews = world.take_effect_spawn_previews();
            world.seed_pending_objects(spawn_previews);
            if !spawns.is_empty() {
                pending_spawns.extend(spawns);
            }
            if !event_other_objects.is_empty() {
                for nested in &event_other_objects {
                    if let Some(update) = nested.update.as_ref() {
                        world.preview_object_update(nested.object_id, update);
                    }
                    if nested.destroy {
                        world.preview_object_destroyed(nested.object_id);
                    }
                    for order in &nested.contents_orders {
                        world.preview_contents_order(
                            order.container,
                            &order.contents,
                            &order.link_generations,
                        );
                    }
                }
                pending_other_objects.extend(event_other_objects);
            }
            if let Some(preview) = event_object_lists {
                world.install_effect_object_lists(preview.clone());
                pending_object_lists = Some(preview);
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
            if !global_effect_commands.is_empty() {
                apply_effect_commands_to_stack(global_effects, &global_effect_commands);
            }
            if stop_denied {
                if let Some(effect) = global_effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number)
                {
                    effect.priority = event.effect.priority;
                } else {
                    insert_effect_into_stack(global_effects, event.effect.clone());
                }
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
            if timer_kill {
                if let Some(effect) = global_effects
                    .iter_mut()
                    .find(|effect| effect.number == event.effect.number && effect.priority != 0)
                {
                    let stopped = effect.clone();
                    effect.priority = 0;
                    queue.push_front(EffectEvent::stopped(stopped, EffectStopReason::Removed));
                }
            }
        }

        *environment = current_environment;
        let next_object_id = world.next_object_id();
        let host_raster_preview =
            (!pending_solid_mask_operations.is_empty()).then(|| world.host_raster_preview());
        Ok(GlobalEffectRunOutcome {
            particles: pending_particles,
            physics_delta: accumulated_physics,
            audio_events: pending_audio,
            messages: pending_messages,
            player_commands: pending_player_commands,
            object_order_commands: pending_object_order_commands,
            next_mission_commands: pending_next_mission_commands,
            landscape_ops: pending_landscape_ops,
            solid_mask_operations: pending_solid_mask_operations,
            host_raster_preview,
            transfer_zones: pending_transfer_zones,
            spawns: pending_spawns,
            other_objects: pending_other_objects,
            object_lists: pending_object_lists,
            next_object_id,
            game_over: game_over_requested,
            script_go: script_go_requested,
            script_counter: script_counter_requested,
            audio_state: current_audio,
            rng,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landscape::{Landscape, PixelGrid};

    #[test]
    fn dig_free_material_rect_uses_exact_pix2mat_for_prefilter() {
        // DigFreeMat compares GetMat exactly before calling DigFreePix
        // (C4Landscape.cpp:1012-1019); GetMat is Pix2Mat[GetPix]
        // (C4Landscape.h:173-176). An unresolved Surface8 slot must not use
        // the column approximation and therefore must not reach DigFreePix's
        // instability probe (C4Landscape.cpp:918-925).
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Earth]\nName=Earth\nDensity=100\nDigFree=1\nInstable=1\n",
        )
        .expect("material parses");
        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        let earth = engine.materials.id_of("Earth").expect("Earth exists");

        let grid = PixelGrid::new(
            1,
            1,
            vec![1],
            vec![0, 100],
            vec![None, None],
            vec![None, None],
        );
        let mut landscape = Landscape::flat_with_material(1, 0, Some(earth));
        landscape.set_world_height(1);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.material_at(0, 0)),
            Some(earth),
            "the general lookup deliberately has a column fallback",
        );

        crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES.with(|probes| probes.borrow_mut().clear());
        engine.dig_free_material_rect(Vector2::new(0, 0), 1, 1, earth);

        assert_eq!(
            engine
                .landscape()
                .and_then(Landscape::pixel_grid)
                .and_then(|grid| grid.byte_at(0, 0)),
            Some(1),
        );
        crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES.with(|probes| {
            assert!(
                probes.borrow().is_empty(),
                "unmatched pixels are not probed"
            );
        });
        assert_eq!(engine.mass_movers.live_movers(), 0);
    }

    #[test]
    fn dig_free_material_rect_rejects_material_outside_loaded_map() {
        // DigFreeMat's outer MatValid gate precedes the rectangle walk
        // (C4Landscape.cpp:1012-1019). Even if a stale Pix2Mat slot carries
        // that numeric id, an id outside Game.Material.Num cannot reach
        // DigFreePix or its instability probe.
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Earth]\nName=Earth\nDensity=100\nInstable=1\n",
        )
        .expect("material parses");
        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&library);
        let invalid = crate::material::MaterialId::new(99).expect("id fits");

        let grid = PixelGrid::new(
            1,
            1,
            vec![1],
            vec![0, 100],
            vec![None, Some("Ghost".to_string())],
            vec![None, None],
        );
        let mut landscape = Landscape::flat(1, 0);
        landscape.set_world_height(1);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);
        engine
            .landscape
            .as_mut()
            .expect("landscape exists")
            .resolve_grid_materials(|name| (name == "Ghost").then_some(invalid));
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.dig_free_pixel_material_at(0, 0)),
            Some(invalid),
            "fixture carries a stale exact Pix2Mat id",
        );

        crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES.with(|probes| probes.borrow_mut().clear());
        engine.dig_free_material_rect(Vector2::new(0, 0), 1, 1, invalid);

        crate::mass_mover::MASS_MOVER_INSTABILITY_PROBES.with(|probes| {
            assert!(
                probes.borrow().is_empty(),
                "invalid materials skip the walk"
            );
        });
        assert_eq!(engine.mass_movers.live_movers(), 0);
    }
}
