//! `impl Engine` — materials, landscape, sectors and the game-settings accessors.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub fn set_materials(&mut self, materials: MaterialSet) {
        self.materials = materials;
        let capacity = self.materials.len();
        for object in &mut self.objects {
            object.ensure_material_capacity(capacity);
        }
        if let Some(landscape) = self.landscape.as_mut() {
            let default = self.materials.default_ground_material();
            landscape.set_default_solid_material(default);
        }
    }

    pub fn materials(&self) -> &MaterialSet {
        &self.materials
    }

    pub fn materials_mut(&mut self) -> &mut MaterialSet {
        &mut self.materials
    }

    pub(crate) fn materials_shared(&self) -> Rc<MaterialSet> {
        let mut cache = self.materials_shared.borrow_mut();
        match cache.as_ref() {
            Some(shared) => Rc::clone(shared),
            None => {
                let shared = Rc::new(self.materials.clone());
                *cache = Some(Rc::clone(&shared));
                shared
            }
        }
    }

    pub fn configure_materials_from_library(&mut self, library: &clonk_resources::MaterialLibrary) {
        self.materials_shared.borrow_mut().take();
        self.materials = MaterialSet::from_resource_library(library);
        let capacity = self.materials.len();
        for object in &mut self.objects {
            object.ensure_material_capacity(capacity);
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// `Game.Script.Counter`, displayed by the developer console and used as
    /// the next `Script%d` section number by `C4GameScriptHost::Execute`.
    pub fn scenario_script_counter(&self) -> i32 {
        self.scenario_script_counter
    }

    pub fn is_game_over(&self) -> bool {
        self.game_over_triggered
    }

    /// Executes the `C4Game::DoGameOver` half of a synchronized control.
    /// In particular, a successful `VoteEnd(VT_Kick, ClientIDUnknown)` ends
    /// replay control without pretending that the unknown client is a
    /// removable player owner.
    #[doc(hidden)]
    pub fn request_game_over_from_control(&mut self) -> Result<bool, EngineError> {
        self.request_game_over()
    }

    pub fn game_time(&self) -> i32 {
        self.game_time
    }

    /// `C4Game::Sec1Timer`: consume the per-frame TimeGo latch and advance
    /// the real game clock at most once (`C4Game.cpp:1755-1759`).
    pub fn sec1_timer(&mut self) {
        if std::mem::take(&mut self.time_go) {
            self.game_time = self.game_time.wrapping_add(1);
        }
    }

    pub(crate) fn assign_player_info_id(&mut self, requested: i32) -> i32 {
        if requested == 0 {
            self.last_player_info_id = self.last_player_info_id.wrapping_add(1);
            self.last_player_info_id
        } else {
            self.last_player_info_id = self.last_player_info_id.max(requested);
            requested
        }
    }

    /// Exact live `C4PlayerInfoList::iLastPlayerID` value.
    #[doc(hidden)]
    pub fn last_player_info_id(&self) -> i32 {
        self.last_player_info_id
    }

    /// Install the exact persisted `C4PlayerInfoList::iLastPlayerID` after
    /// replay PlayerInfo rows have been projected into the engine.
    #[doc(hidden)]
    pub fn set_last_player_info_id(&mut self, last_player_info_id: i32) {
        self.last_player_info_id = last_player_info_id;
    }

    pub fn configure_objectives(&mut self, objectives: ScenarioObjectives) {
        self.objectives = objectives;
        self.objective_check_counter = GAME_OVER_CHECK_INTERVAL.saturating_sub(1);
    }

    #[doc(hidden)]
    pub fn set_needed_material_resource_strings(
        &mut self,
        need: impl Into<String>,
        none: impl Into<String>,
    ) {
        self.needed_material_strings = Rc::new(NeededMaterialStrings::new(need, none));
    }

    #[doc(hidden)]
    pub fn set_object_no_dig_resource_string(&mut self, template: impl Into<String>) {
        self.object_no_dig_resource_string = Rc::new(template.into());
    }

    #[doc(hidden)]
    pub fn set_construction_check_resource_strings(
        &mut self,
        undefined: impl Into<String>,
        no_construction: impl Into<String>,
        no_room: impl Into<String>,
        no_level: impl Into<String>,
        no_other: impl Into<String>,
    ) {
        self.construction_check_strings = Rc::new(ConstructionCheckStrings::new(
            undefined,
            no_construction,
            no_room,
            no_level,
            no_other,
        ));
    }

    #[doc(hidden)]
    pub fn set_default_rank_names(&mut self, names: Vec<String>) {
        self.default_rank_names = Rc::new(names);
    }

    pub fn set_landscape(&mut self, mut landscape: Landscape) {
        let default = self.materials.default_ground_material();
        if default.is_some() {
            landscape.set_default_solid_material(default);
        }
        // UpdatePixMaps (C4Landscape.cpp:2832-2839): resolve the grid's
        // texmap material names into engine ids now that both exist.
        let materials = &self.materials;
        landscape.resolve_grid_materials(|name| materials.id_of(name));
        // MVehic (C4Game::InitMaterialTexture, C4Game.cpp:1669): the
        // closed-border material for GetPix's MCVehic reads.
        landscape.set_vehicle_material(materials.id_of("Vehicle"));
        self.landscape = Some(landscape);
        self.reset_sectors_from_landscape();
    }

    pub fn blast_circle(
        &mut self,
        center: Vector2,
        radius: i32,
        controller: Option<i32>,
    ) -> Option<BlastResult> {
        if radius < 0 {
            return None;
        }
        let has_pixel_grid = self.landscape.as_ref()?.pixel_grid().is_some();
        let result = if has_pixel_grid {
            self.blast_raster_circle(center, radius)
        } else {
            let result = {
                let landscape = self.landscape.as_mut()?;
                landscape.blast_circle(center, radius, &self.materials)
            };
            if !result.shift_candidates.is_empty() {
                self.apply_blast_shifts(radius, &result);
            }
            // Column-only fixture worlds cannot interleave their approximate
            // clears with C++'s per-pixel instability probes. Retain the old
            // post-pass for that synthetic fallback only.
            for ycnt in -radius..=radius {
                let remaining =
                    i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
                let lwdt = (remaining as f64).sqrt() as i32;
                let dpy = center.y + ycnt;
                for xcnt in -lwdt..lwdt + i32::from(lwdt == 0) {
                    self.check_instability_range(center.x + xcnt, dpy);
                }
            }
            result
        };
        // The evaluate loop keys on the PRE-blast BlastMatCount, not on
        // what was removed (C4Landscape.cpp:1065-1079).
        if !result.pixel_count_by_material.is_empty() {
            self.process_blast_reactions(center, controller, &result);
        }
        Some(result)
    }

    /// C4Landscape::BlastFree / BlastFreePix on the authoritative Surface8
    /// plane (C4Landscape.cpp:941-960,1022-1063): count the complete circle
    /// first, then revisit every pixel in the same order. BlastShiftTo draws
    /// once PER source pixel, writes while retaining IFT, BlastFree clears
    /// based on the original material, and instability is probed immediately.
    fn blast_raster_circle(&mut self, center: Vector2, radius: i32) -> BlastResult {
        let mut result = BlastResult::default();
        for ycnt in -radius..=radius {
            let remaining =
                i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
            let lwdt = (remaining as f64).sqrt() as i32;
            let y = center.y + ycnt;
            for xcnt in -lwdt..lwdt + i32::from(lwdt == 0) {
                let x = center.x + xcnt;
                if let Some(material) = self
                    .landscape
                    .as_ref()
                    .and_then(|landscape| landscape.border_material_at(x, y))
                {
                    *result.pixel_count_by_material.entry(material).or_insert(0) += 1;
                }
            }
        }

        let blast_size = compute_blast_size(radius);
        let grade = compute_blast_grade(radius);
        let shift_threshold = (blast_size * grade) / 6;
        let mut changed_columns = HashSet::new();
        for ycnt in -radius..=radius {
            let remaining =
                i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
            let lwdt = (remaining as f64).sqrt() as i32;
            let y = center.y + ycnt;
            for xcnt in -lwdt..lwdt + i32::from(lwdt == 0) {
                let x = center.x + xcnt;
                let material = self
                    .landscape
                    .as_ref()
                    .and_then(|landscape| landscape.border_material_at(x, y));
                if let Some(material) = material {
                    let (blast_free, shift_spec, shift_target) = self
                        .materials
                        .get_by_id(material)
                        .map(|entry| {
                            (
                                entry.blast_free(),
                                entry.blast_shift_to_spec().map(str::to_owned),
                                entry.blast_shift_to_target(),
                            )
                        })
                        .unwrap_or((false, None, None));
                    let shift_byte =
                        shift_spec
                            .as_deref()
                            .zip(shift_target)
                            .and_then(|(spec, fallback)| {
                                self.landscape.as_ref().and_then(|landscape| {
                                    landscape.crossmapped_material_texture_byte(
                                        spec,
                                        material,
                                        &self.materials,
                                        fallback,
                                    )
                                })
                            });
                    if let Some(shift_byte) = shift_byte {
                        let material_count = result
                            .pixel_count_by_material
                            .get(&material)
                            .copied()
                            .unwrap_or(0);
                        if i64::from(self.rng.random(material_count)) < shift_threshold {
                            let shifted = self.landscape.as_mut().is_some_and(|landscape| {
                                landscape.insert_material_texture_pix(x, y, shift_byte)
                            });
                            if shifted {
                                changed_columns.insert(x);
                            }
                        }
                    }
                    if blast_free
                        && self
                            .landscape
                            .as_mut()
                            .is_some_and(|landscape| landscape.clear_pix(x, y))
                    {
                        *result.removed_by_material.entry(material).or_insert(0) += 1;
                        changed_columns.insert(x);
                    }
                }
                self.check_instability_range(x, y);
            }
        }

        if let Some((width, _)) = self.landscape.as_ref().and_then(Landscape::grid_dimensions) {
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
        result
    }

    pub fn clear_landscape(&mut self) {
        self.landscape = None;
        self.sectors = None;
        self.pxs_system.clear();
    }

    pub(crate) fn load_scenario_landscape_systems(
        &mut self,
        systems: &scenario::ScenarioLandscapeSystems,
        clear_missing: bool,
    ) {
        match &systems.pxs {
            Some(pxs) => self.pxs_system = pxs.clone(),
            None if clear_missing => self.pxs_system.clear(),
            None => {}
        }
        match &systems.mass_movers {
            Some(mass_movers) => self.mass_movers = mass_movers.clone(),
            None if clear_missing => self.mass_movers.clear(),
            None => {}
        }
    }

    pub fn landscape(&self) -> Option<&Landscape> {
        self.landscape.as_ref()
    }

    /// `C4EditCursor::ApplyToolPicker` (`C4EditCursor.cpp:698-731`) at a
    /// landscape pixel: Static samples the retained map through `MapZoom`,
    /// Exact reads the live material and IFT bit, and anything unresolved is
    /// sky. Any other mode picks nothing.
    pub fn developer_tool_pick(
        &self,
        x: i32,
        y: i32,
    ) -> Option<crate::developer_landscape::ToolPick> {
        use crate::developer_landscape as tool;
        let landscape = self.landscape.as_ref()?;
        match landscape.mode() {
            crate::landscape::LANDSCAPE_MODE_STATIC => {
                let state = self.developer_landscape_tool_state()?;
                let raster = landscape.raster_state()?;
                let (map_x, map_y) = tool::static_pick_map_coordinates(x, y, raster.map_zoom())?;
                let map = raster.map()?;
                let byte = usize::try_from(map_y)
                    .ok()
                    .zip(usize::try_from(map_x).ok())
                    .filter(|(row, column)| {
                        *row < map.height as usize && *column < map.width as usize
                    })
                    .and_then(|(row, column)| {
                        map.indices.get(row * map.width as usize + column).copied()
                    })
                    // Off-map coordinates read as sky, like an empty map byte.
                    .unwrap_or(0);
                Some(tool::static_tool_pick(state.texmap(), byte))
            }
            crate::landscape::LANDSCAPE_MODE_EXACT => {
                let material = landscape
                    .border_material_at(x, y)
                    .and_then(|id| self.materials.get_by_id(id))
                    .map(|material| material.name().to_owned());
                Some(tool::exact_tool_pick(
                    material.as_deref(),
                    landscape.is_ift_at(x, y),
                ))
            }
            _ => None,
        }
    }

    /// The read-only landscape-tool state the developer console populates its
    /// material/texture controls from (`C4ToolsDlg.cpp:482-508,796-940`).
    /// `None` when no landscape exists, which is when C++ disables the controls.
    pub fn developer_landscape_tool_state(
        &self,
    ) -> Option<crate::developer_landscape::DeveloperLandscapeToolState> {
        let landscape = self.landscape.as_ref()?;
        let raster = landscape.raster_state();
        let texmap = raster.map(|state| state.texmap());
        Some(crate::developer_landscape::DeveloperLandscapeToolState {
            mode: landscape.mode(),
            map_zoom: raster.map(|state| state.map_zoom()),
            has_map: raster.is_some_and(|state| state.map().is_some()),
            material_map_names: self
                .materials
                .materials()
                .iter()
                .map(|material| material.name().to_owned())
                .collect(),
            material_names: texmap
                .map(|texmap| texmap.material_names.clone())
                .unwrap_or_default(),
            texture_names: texmap
                .map(|texmap| texmap.texture_names.clone())
                .unwrap_or_default(),
            texture_inventory: texmap
                .map(|texmap| texmap.texture_inventory.clone())
                .unwrap_or_default(),
        })
    }

    pub(crate) fn reset_sectors_from_landscape(&mut self) {
        let Some(landscape) = self.landscape.as_ref() else {
            self.sectors = None;
            return;
        };
        let width = saturating_u64_to_i32(u64::from(landscape.width()));
        let height = landscape.estimated_height();
        self.sectors = Some(SectorMap::new(width, height));
        self.rebuild_sectors();
    }

    pub(crate) fn rebuild_sectors(&mut self) {
        #[cfg(test)]
        SECTOR_FULL_REBUILDS.with(|count| count.set(count.get().saturating_add(1)));
        // C4LSectors::Add receives the main-list order. `exec_list` stores
        // that list reversed, so rebuild each sector front-to-back here;
        // SetObjectOrder's UpdatePosResort must change area traversal too
        // (C4GameObjects.cpp:739-769).
        let mut seen = HashSet::with_capacity(self.objects.len());
        let mut records = Vec::with_capacity(self.objects.len());
        for &id in self.exec_list.iter().rev() {
            let Some(index) = self.find_object_index(id) else {
                continue;
            };
            seen.insert(id);
            if let Some(record) = self.sector_record_for_object(&self.objects[index]) {
                records.push(record);
            }
        }
        records.extend(
            self.objects
                .iter()
                .filter(|object| seen.insert(object.id))
                .filter_map(|object| self.sector_record_for_object(object)),
        );
        if let Some(sectors) = self.sectors.as_mut() {
            sectors.rebuild(records);
        }
    }

    /// `C4GameObjects::UpdatePosResort`: remove and re-add exactly one
    /// object's sector links against the current master list. A full rebuild
    /// is observably different when unrelated sector pairs intentionally
    /// retain an older order after native SortByCategory.
    pub(crate) fn update_pos_resort(&mut self, object_id: ObjectId) {
        let record = self
            .find_object_index(object_id)
            .and_then(|index| self.sector_record_for_object(&self.objects[index]));
        let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        let Some(sectors) = self.sectors.as_mut() else {
            return;
        };
        sectors.remove(object_id);
        sectors.set_master_order(master_order);
        if let Some(record) = record {
            sectors.add(record);
        }
    }

    pub(crate) fn update_sector_for_index(&mut self, index: usize) {
        let Some(object) = self.objects.get(index) else {
            return;
        };
        let object_id = object.id;
        let record = self.sector_record_for_object(object);
        let Some(sectors) = self.sectors.as_mut() else {
            return;
        };
        match record {
            Some(record) => sectors.update(record),
            None => sectors.remove(object_id),
        }
    }

    fn sector_record_for_object(&self, object: &Object) -> Option<SectorObject> {
        if object.destroyed || !object.state.status.is_active() {
            return None;
        }
        let position = object.state.position;
        let shape_rect = self.object_shape_rect(object);
        Some(SectorObject {
            id: object.id,
            position,
            shape_rect,
        })
    }

    pub(crate) fn object_shape_rect(&self, object: &Object) -> DefinitionRect {
        let position = object.state.position;
        let mut rect = object
            .current_shape_rect()
            .map(|rect| {
                DefinitionRect::new(
                    position.x.saturating_add(rect.x),
                    position.y.saturating_add(rect.y),
                    rect.width,
                    rect.height,
                )
            })
            .or_else(|| vertex_bounds_rect(position, &object.state.vertices))
            .unwrap_or_else(|| DefinitionRect::new(position.x, position.y, 1, 1));
        // C4Object::Top/Height and At expand short construction shapes
        // upward to an 18-pixel action area (C4Object.h:340-344;
        // C4Object.cpp:1124-1140). C4LArea::Set uses those same accessors,
        // so sector ObjectShapes membership must include the expansion too.
        let add_top = (18 - rect.height).max(0);
        rect.y = rect.y.saturating_sub(add_top);
        rect.height = rect.height.saturating_add(add_top);
        rect
    }

    pub(crate) fn object_entrance_area(&self, object: &Object) -> Option<DefinitionRect> {
        if object.state.ocf & crate::ocf::ENTRANCE == 0 {
            return None;
        }
        let entrance = self
            .definitions
            .get(&object.definition_id)?
            .entrance_rect()?;
        Some(DefinitionRect::new(
            object.state.position.x.saturating_add(entrance.x),
            object.state.position.y.saturating_add(entrance.y),
            entrance.width,
            entrance.height,
        ))
    }

    #[doc(hidden)]
    pub fn at_object(
        &self,
        point: Vector2,
        mask: u32,
        exclude: Option<ObjectId>,
    ) -> Option<(usize, ObjectId, u32)> {
        let candidate_ids = self
            .sectors
            .as_ref()
            // C4GameObjects::ObjectsAt returns this sector's ObjectShapes,
            // not its center-point Objects list (C4GameObjects.cpp:87-90).
            .map(|sectors| sectors.shape_ids_at(point.x, point.y).to_vec())
            .unwrap_or_else(|| self.objects.iter().map(|object| object.id).collect());
        // Preserve the outer Option: Some(None) is a valid layerless exclude
        // and only matches candidates whose pLayer is likewise null.
        let exclude_layer = exclude.and_then(|id| {
            self.find_object_index(id)
                .map(|index| self.objects[index].state.layer)
        });
        for candidate_id in candidate_ids {
            if exclude == Some(candidate_id) {
                continue;
            }
            let Some(candidate_idx) = self.find_object_index(candidate_id) else {
                continue;
            };
            let candidate = &self.objects[candidate_idx];
            if exclude_layer.is_some_and(|layer| candidate.state.layer != layer) {
                continue;
            }
            if candidate.destroyed
                || !candidate.state.status.is_active()
                || candidate.state.container.is_some()
            {
                continue;
            }
            let candidate_ocf = self.object_ocf_at_index(candidate_idx);
            if candidate_ocf & (mask | crate::ocf::EXCLUSIVE) == 0 {
                continue;
            }
            if !self
                .object_shape_rect(candidate)
                .contains_point(point.x, point.y)
            {
                continue;
            }
            // GetOCFForPos (C4Object.cpp:1146-1160): the returned mask
            // keeps Entrance/Collection only inside their def areas.
            let candidate_ocf = self.object_ocf_for_pos(candidate_idx, point);
            if candidate_ocf & mask != 0 {
                return Some((candidate_idx, candidate_id, candidate_ocf));
            }
            return None;
        }
        None
    }

    /// `C4Object::GetOCFForPos` (C4Object.cpp:1146-1160): the cached mask
    /// with OCF_Entrance/OCF_Collection verified against the def's
    /// Entrance/Collection areas at the probe point.
    pub(crate) fn object_ocf_for_pos(&self, index: usize, point: Vector2) -> u32 {
        let object = &self.objects[index];
        let mut rocf = object.state.ocf;
        if rocf & (crate::ocf::ENTRANCE | crate::ocf::COLLECTION) == 0 {
            return rocf;
        }
        let definition = self.definitions.get(&object.definition_id);
        let position = object.state.position;
        let inside_area = |rect: Option<DefinitionRect>| {
            rect.is_some_and(|rect| {
                let dx = point.x - (position.x + rect.x);
                let dy = point.y - (position.y + rect.y);
                (0..rect.width).contains(&dx) && (0..rect.height).contains(&dy)
            })
        };
        // Verify entrance area (C4Object.cpp:1149-1153)
        if rocf & crate::ocf::ENTRANCE != 0
            && !inside_area(definition.and_then(|definition| definition.entrance_rect()))
        {
            rocf &= !crate::ocf::ENTRANCE;
        }
        // Verify collection area (C4Object.cpp:1154-1158)
        if rocf & crate::ocf::COLLECTION != 0
            && !inside_area(definition.and_then(|definition| definition.collection_rect()))
        {
            rocf &= !crate::ocf::COLLECTION;
        }
        rocf
    }

    pub fn find_path(
        &self,
        from: Vector2,
        to: Vector2,
        level: i32,
        transfer_zones_enabled: bool,
    ) -> Option<pathfinder::Path> {
        let landscape = self.landscape.as_ref()?;
        let zones = self.transfer_zones.states();
        let mut finder = PathFinder::new(landscape, &zones);
        finder.set_level(level);
        finder.enable_transfer_zones(transfer_zones_enabled);
        let path = finder.find(from, to);
        *self.pathfinder_debug.borrow_mut() = finder.debug_snapshot().clone();
        path
    }

    pub fn physics(&self) -> PhysicsSettings {
        self.physics
    }

    /// Register a particle definition, mirroring `C4ParticleDef::Load`
    /// (C4Particles.cpp:118-192). `gfx_length` is the number of animation
    /// phases in the graphics, `aspect` the native facet width/height ratio —
    /// both derived from Graphics.png at load time in C++. This legacy/manual
    /// seam registers no frontend graphics payload but still validates all
    /// named init, exec, collision, and draw procedures.
    pub fn register_particle_definition(
        &mut self,
        core: particles::ParticleDefCore,
        gfx_length: i32,
        aspect: f32,
    ) -> Result<(), particles::ParticleDefError> {
        self.particle_system.register_def(core, gfx_length, aspect)
    }

    /// Register one fully decoded particle resource, retaining its RGBA image
    /// and normalized source-facet metadata for the frontend render catalog.
    pub fn register_particle_resource(
        &mut self,
        resource: &clonk_resources::ParticleDefinition,
    ) -> Result<(), particles::ParticleDefError> {
        self.particle_system.register_resource(resource)
    }

    /// Loaded particle definitions in native linked-list order. The returned
    /// catalog is immutable; resource-backed entries expose graphics while
    /// manually registered simulation-only entries have `graphics == None`.
    pub fn particle_render_catalog(&self) -> &[particles::ParticleDef] {
        self.particle_system.definitions()
    }

    pub fn particle_system(&self) -> &particles::ParticleSystem {
        &self.particle_system
    }

    /// Set process-local `Config.Graphics.SmokeLevel`. Def-based particles
    /// consume the configured value directly; synchronized BubbleOut object
    /// creation uses the fixed legacy cap instead.
    pub fn set_smoke_level(&mut self, smoke_level: i32) {
        self.particle_system.smoke_level = smoke_level;
    }

    /// Set process-local `Config.Graphics.FireParticles`. C++ folds it into
    /// `SetDefParticles` (C4Particles.cpp:483-489), leaving pFire1/pFire2
    /// null, so switching it off silences the automatic fire emitter without
    /// touching script-created `Fire`/`Fire2` particles or the fire facet.
    pub fn set_fire_particles(&mut self, enabled: bool) {
        self.particle_system.fire_particles = enabled;
    }

    pub(crate) fn bubble_smoke_level(&self) -> i32 {
        if self.network_game || self.recording_active {
            SYNC_SMOKE_LEVEL
        } else {
            self.particle_system.smoke_level
        }
    }

    pub fn set_physics(&mut self, mut physics: PhysicsSettings) {
        physics.reconcile_raw_gravity();
        self.physics = physics;
        for object in &mut self.objects {
            object.clamp_velocity(&self.physics);
        }
    }

    pub fn set_network_game(&mut self, network_game: bool) {
        self.network_game = network_game;
    }

    /// Set the process-local CM_Network gate independently from the preserved
    /// IsNetwork game parameter (notably for ChangeToLocal).
    #[doc(hidden)]
    pub fn set_network_control_mode(&mut self, network_control_mode: bool) {
        self.network_control_mode = network_control_mode;
    }

    pub(crate) fn control_sync_mode(&self) -> bool {
        self.network_control_mode || self.replay_control || self.recording_active
    }

    /// Set the process-local recording gate used by GetSmokeLevel. The app
    /// pre-arms it before initialization when it will attach a recorder.
    pub fn set_recording_active(&mut self, recording_active: bool) {
        self.recording_active = recording_active;
    }

    pub fn set_max_players(&mut self, max_players: i32) {
        self.max_players = Some(max_players);
    }

    pub fn max_players(&self) -> Option<i32> {
        self.max_players
    }

    /// Freeze `Game.Parameters.StartupPlayerCount` at the native startup
    /// boundary. Repeated calls retain the first value exactly, including 0.
    pub fn freeze_startup_player_count(&mut self, startup_player_count: i32) -> i32 {
        *self
            .startup_player_count
            .get_or_insert(startup_player_count)
    }

    pub fn startup_player_count(&self) -> Option<i32> {
        self.startup_player_count
    }

    /// Updates `Game.Parameters.UseFairCrew`. Native parameter assignment by
    /// itself does not invalidate already-derived definition physicals; the
    /// synchronized control and definition-list paths do that explicitly.
    pub fn set_use_fair_crew(&mut self, use_fair_crew: bool) {
        self.use_fair_crew = use_fair_crew;
    }

    pub fn use_fair_crew(&self) -> bool {
        self.use_fair_crew
    }

    /// Updates `Game.Parameters.FairCrewStrength` for the next cache fill.
    /// The default is `Config.General.DefCrewStrength`.
    pub fn set_fair_crew_strength(&mut self, fair_crew_strength: i32) {
        self.fair_crew_strength = fair_crew_strength;
    }

    pub fn fair_crew_strength(&self) -> i32 {
        self.fair_crew_strength
    }

    /// `C4DefList::Synchronize` / runtime `C4CVT_FairCrew`: preserve the
    /// shared cache allocation so already-copied host worlds enter the same
    /// empty epoch.
    #[doc(hidden)]
    pub fn clear_fair_crew_physicals(&mut self) {
        self.fair_crew_physical_cache.borrow_mut().clear();
    }

    pub fn set_fair_crew_forced(&mut self, fair_crew_forced: bool) {
        self.fair_crew_forced = fair_crew_forced;
    }

    pub fn fair_crew_forced(&self) -> bool {
        self.fair_crew_forced
    }

    pub fn set_allow_debug(&mut self, allow_debug: bool) {
        self.allow_debug = allow_debug;
    }

    pub fn allow_debug(&self) -> bool {
        self.allow_debug
    }

    #[doc(hidden)]
    pub fn set_debug_mode(&mut self, debug_mode: bool) {
        self.debug_mode = debug_mode;
        // `DebugLog` reaches the message board and developer console only while
        // the round has debug mode on (C4Game.cpp:447-454).
        clonk_core::log_target::set_debug_mode_presentation(debug_mode);
    }

    pub fn debug_mode(&self) -> bool {
        self.debug_mode
    }

    /// `C4ControlSet::Execute(C4CVT_DisableDebug)`: unlike every other
    /// mutating Set type, this is intentionally not restricted to the host.
    pub fn disable_debug(&mut self) {
        if self.debug_mode {
            self.message_board_commands
                .retain(|command| clonk_script::c4_string_bytes(&command.name) != b"speed");
        }
        self.debug_mode = false;
        self.allow_debug = false;
        clonk_core::log_target::set_debug_mode_presentation(false);
    }

    /// Install the live C4GameControl rate without disturbing ControlTick or
    /// the absolute FrameCounter phase.
    pub fn set_control_rate(&mut self, control_rate: i32) {
        self.control_rate = control_rate.clamp(
            NetworkControlTiming::MIN_CONTROL_RATE,
            NetworkControlTiming::MAX_CONTROL_RATE,
        );
    }

    pub fn control_rate(&self) -> i32 {
        self.control_rate
    }

    /// Current process-local delay between simulation ticks. SetGameSpeed
    /// stores integer-truncated `1000 / fps` milliseconds here.
    pub fn game_tick_delay_ms(&self) -> u64 {
        self.game_tick_delay_ms.get()
    }

    /// Changes on every successful SetGameSpeed, even when the delay is
    /// unchanged, so the embedding scheduler can mirror ResetTimer.
    #[doc(hidden)]
    pub fn game_tick_delay_revision(&self) -> u64 {
        self.game_tick_delay_revision.get()
    }

    pub fn set_league_game(&mut self, league_game: bool) {
        self.league_game = league_game;
        if league_game {
            self.team_state.team_configuration.allow_team_switch = false;
        }
    }

    /// Install exact `Game.Parameters.League` bytes. The native progress
    /// data API gates on this name, not on LeagueAddress/`isLeague()`.
    pub fn set_league_name(&mut self, league_name: Vec<u8>) {
        self.league_name = Rc::new(legacy_c_string_bytes(league_name));
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
    }

    /// Replace the engine's projection of all retained `Game.PlayerInfos`
    /// league progress buffers. Key presence represents a known info row;
    /// the nested option preserves null versus allocated-empty StdStrBuf.
    pub fn replace_player_info_league_progress_data(
        &mut self,
        entries: impl IntoIterator<Item = (i32, Option<Vec<u8>>)>,
    ) {
        let mut progress_data = entries
            .into_iter()
            .filter(|(id, _)| *id != 0)
            .map(|(id, data)| (id, data.map(legacy_c_string_bytes)))
            .collect::<BTreeMap<_, _>>();
        if !self.league_name.is_empty() {
            for player in self.players.values() {
                let id = player.player_info_id();
                if id != 0 {
                    progress_data.entry(id).or_insert(None);
                }
            }
        }
        if let Some(maximum_id) = progress_data.keys().next_back().copied() {
            self.last_player_info_id = self.last_player_info_id.max(maximum_id);
        }
        self.player_info_league_progress_data = Rc::new(progress_data);
    }

    /// Replace the retained `C4PlayerInfo::iLeagueScore` projection. Score
    /// zero is the serialized default and is stored sparsely; PlayerInfo row
    /// existence remains authoritative in the progress/active-player maps.
    pub fn replace_player_info_league_scores(
        &mut self,
        entries: impl IntoIterator<Item = (i32, i32)>,
    ) {
        let mut scores = BTreeMap::new();
        for (id, score) in entries.into_iter().filter(|(id, _)| *id != 0) {
            // GetPlayerInfoByID resolves the first row when malformed input
            // contains duplicate IDs; preserve that storage-order rule.
            scores.entry(id).or_insert(score);
        }
        if let Some(maximum_id) = scores.keys().next_back().copied() {
            self.last_player_info_id = self.last_player_info_id.max(maximum_id);
        }
        scores.retain(|_, score| *score != 0);
        self.player_info_league_scores = Rc::new(scores);
    }

    /// Applies the league server's final evaluation to persistent round
    /// results without overwriting live PlayerInfo's pre-round score/rank.
    pub fn evaluate_league_round_results(
        &mut self,
        success: bool,
        result_message: Vec<u8>,
        players: impl IntoIterator<Item = LeagueRoundResultUpdate>,
    ) {
        self.round_results
            .evaluate_league(success, result_message, players);
    }

    /// Applies a network failure verdict without replacing an earlier,
    /// more-specific result.
    pub fn evaluate_network_round_results(
        &mut self,
        result: RoundResultsNetworkResult,
        result_message: Option<Vec<u8>>,
    ) {
        self.round_results.evaluate_network(result, result_message);
    }

    /// Close the process-local query input and build its synchronized answer
    /// packet. This is the `MarkMessageBoardQueryAnswered` step performed by
    /// C4ChatInputDialog before the queued control executes.
    pub fn prepare_message_board_answer_control(
        &mut self,
        answer: LegacyCString,
        by_client: i32,
    ) -> Option<MessageBoardAnswerControlData> {
        let input = self.active_message_board_input.take()?;
        let marked = self
            .players
            .get_mut(&input.player)?
            .mark_message_board_query_answered(input.target);
        if !marked {
            return None;
        }

        let answer = if input.uppercase {
            let bytes = answer
                .as_bytes()
                .iter()
                .map(|&byte| match byte {
                    b'a'..=b'z' => byte - b'a' + b'A',
                    0xe4 => 0xc4,
                    0xf6 => 0xd6,
                    0xfc => 0xdc,
                    other => other,
                })
                .collect();
            LegacyCString::from_bytes(bytes)
                .expect("capitalizing a NUL-free message-board answer cannot add NUL")
        } else {
            answer
        };
        let object = match input.target {
            Some(target) => i32::try_from(target.as_u64()).ok()?,
            None => 0,
        };
        Some(MessageBoardAnswerControlData {
            object,
            answer,
            player: input.player,
            by_client,
        })
    }
}
