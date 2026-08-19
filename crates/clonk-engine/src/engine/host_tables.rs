//! `impl Engine` — host definition tables, world contexts and the scenario script.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub(crate) fn definition_metadata_table(
        &self,
    ) -> Rc<HashMap<DefinitionId, DefinitionMetadata>> {
        #[cfg(test)]
        DEFINITION_METADATA_TABLE_READS.with(|count| count.set(count.get().saturating_add(1)));
        let mut cache = self.definition_metadata_cache.borrow_mut();
        if let Some(table) = cache.as_ref() {
            return Rc::clone(table);
        }
        let table: Rc<HashMap<DefinitionId, DefinitionMetadata>> = Rc::new(
            self.definitions
                .iter()
                .map(|(id, definition)| {
                    (
                        id.clone(),
                        DefinitionMetadata {
                            name: definition.name().to_string(),
                            portrait_names: definition
                                .portrait_graphics_names()
                                .map(str::to_string)
                                .collect(),
                            category: definition.category(),
                            border_bound: definition.border_bound(),
                            contact_function_calls: definition.contact_function_calls(),
                            blit_mode: definition.blit_mode(),
                            ocf_base: definition.ocf_base(),
                            crew_member: definition.is_crew(),
                            crew_member_value: definition.crew_member_value(),
                            silent_commands: definition.silent_commands(),
                            vehicle_control: definition.vehicle_control(),
                            action_library: definition.action_library().clone().into(),
                            control_transfer_callback: definition.control_transfer_callback(),
                            action_graphics: definition.action_graphics().clone(),
                            value: definition.value(),
                            allow_picture_stack: definition.allow_picture_stack(),
                            mass: definition.mass(),
                            float_line: definition.float_line(),
                            no_component_mass: definition.no_component_mass(),
                            constructable: definition.is_constructable(),
                            shape: definition.shape_rect(),
                            placement: definition.placement(),
                            growth: definition.growth(),
                            construction_offset: definition.construction_offset(),
                            basement: definition.basement(),
                            physical: *definition.physical(),
                            components: definition
                                .components()
                                .iter()
                                .map(|component| {
                                    (component.id.as_str().to_string(), component.count)
                                })
                                .collect(),
                            collection_limit: definition.collection_limit(),
                            grab_put_get: definition.grab_put_get(),
                            line_connect: definition.line_connect(),
                            stretch_growth: definition.stretch_growth(),
                            rotateable: definition.rotateable(),
                            line: definition.line(),
                            vertices: definition.shape_vertices().to_vec(),
                            contact_density: Some(definition.contact_density()),
                            clonk_name_newlines: definition
                                .clonk_names()
                                .map(|names| names.bytes().filter(|&b| b == b'\n').count() as i32),
                            fire: compat::DefinitionFireMetadata {
                                def_core_values: compat::DefCoreValueStore::from_definition(
                                    definition,
                                )
                                .into(),
                                fire_top: definition.fire_top(),
                                smoke_rate: definition.smoke_rate(),
                                lift_top: definition.lift_top(),
                                blast_incinerate: definition.blast_incinerate(),
                                burn_turn_to: definition.burn_turn_to().map(str::to_string),
                                incomplete_activity: definition.incomplete_activity(),
                                no_burn_decay: definition.no_burn_decay(),
                                no_burn_damage: definition.no_burn_damage(),
                                contact_incinerate: definition.contact_incinerate(),
                                contain_blast: definition.contain_blast(),
                                closed_container: definition.closed_container(),
                                no_horizontal_move: definition.no_horizontal_move(),
                                grab: definition.grab(),
                                no_push_enter: definition.no_push_enter(),
                                no_get: definition.no_get(),
                                oversize: definition.oversize(),
                                collection_rect: definition.collection_rect(),
                                fragile: definition.fragile(),
                                projectile: definition.projectile(),
                                entrance_rect: definition.entrance_rect(),
                                rotated_entrance: definition.rotated_entrance,
                                attract_lightning: definition.attract_lightning,
                                no_fight: definition.no_fight,
                            },
                        },
                    )
                })
                .collect(),
        );
        *cache = Some(Rc::clone(&table));
        table
    }

    pub(crate) fn command_definition_snapshot_table(
        &self,
    ) -> Rc<HashMap<DefinitionId, CommandDefinitionSnapshot>> {
        let mut cache = self.command_definition_snapshot_cache.borrow_mut();
        if let Some(table) = cache.as_ref() {
            return Rc::clone(table);
        }
        let table = Rc::new(
            self.definitions
                .iter()
                .map(|(id, definition)| {
                    let chop_action =
                        definition
                            .action_library()
                            .specs()
                            .iter()
                            .find_map(|(name, spec)| {
                                spec.procedure
                                    .as_deref()
                                    .filter(|procedure| {
                                        ActionProcedure::from_name(procedure)
                                            == ActionProcedure::Chop
                                    })
                                    .map(|_| name.clone())
                            });
                    (
                        id.clone(),
                        CommandDefinitionSnapshot {
                            value: definition.value(),
                            shape: definition.shape_rect(),
                            category: definition.category(),
                            construction_offset: definition.construction_offset(),
                            collection_limit: definition.collection_limit(),
                            collection_rect: definition.collection_rect(),
                            fragile: definition.fragile(),
                            projectile: definition.projectile(),
                            can_chop: chop_action.is_some(),
                            chop_action,
                            constructable: definition.is_constructable(),
                            grab: definition.grab(),
                            grab_put_get: definition.grab_put_get(),
                            no_get: definition.no_get(),
                        },
                    )
                })
                .collect(),
        );
        *cache = Some(Rc::clone(&table));
        table
    }

    pub(crate) fn host_definition_tables(&self) -> Rc<compat::HostDefinitionTables> {
        let mut cache = self.host_definition_tables_cache.borrow_mut();
        if let Some(tables) = cache.as_ref() {
            return Rc::clone(tables);
        }
        let tables = Rc::new(compat::HostDefinitionTables::new(
            self.definitions
                .iter()
                .filter(|(_, definition)| definition.color_by_owner())
                .map(|(id, _)| id.clone())
                .collect(),
            self.definitions
                .iter()
                .filter(|(_, definition)| definition.base_auto_sell())
                .map(|(id, _)| id.clone())
                .collect(),
            self.definitions
                .iter()
                .filter(|(_, definition)| definition.rebuyable())
                .map(|(id, _)| id.clone())
                .collect(),
            self.definitions
                .iter()
                .filter(|(_, definition)| definition.no_sell() != 0)
                .map(|(id, _)| id.clone())
                .collect(),
            self.definitions
                .iter()
                .filter_map(|(id, definition)| {
                    definition
                        .description()
                        .map(|description| (id.clone(), description.to_string()))
                })
                .collect(),
            self.definitions
                .iter()
                .filter_map(|(id, definition)| {
                    definition
                        .rank_names()
                        .map(|names| (id.clone(), names.clone()))
                })
                .collect(),
            self.definitions
                .iter()
                .filter_map(|(id, definition)| {
                    definition.rank_base().map(|base| (id.clone(), base))
                })
                .collect(),
            self.definitions
                .iter()
                .map(|(id, definition)| (id.clone(), definition.script_arc()))
                .collect(),
            self.script_link_sources
                .iter()
                .filter_map(|source| match source {
                    ScriptLinkSource::Script { name, script, .. } => {
                        Some((name.clone(), Arc::clone(script)))
                    }
                    ScriptLinkSource::Definition(_) | ScriptLinkSource::Scenario => None,
                })
                .collect(),
            self.standard_names.clone(),
            self.definitions
                .iter()
                .filter_map(|(id, definition)| {
                    definition
                        .clonk_names()
                        .map(|names| (id.as_str().to_string(), names.to_string()))
                })
                .collect(),
            self.reloadable_definition_ids(),
        ));
        *cache = Some(Rc::clone(&tables));
        tables
    }

    pub(crate) fn invalidate_host_definition_tables(&self) {
        self.host_definition_tables_cache.borrow_mut().take();
    }

    fn solid_mask_metadata_table(&self) -> Rc<HashMap<DefinitionId, HostSolidMaskMetadata>> {
        let mut cache = self.solid_mask_metadata_cache.borrow_mut();
        if let Some(table) = cache.as_ref() {
            return Rc::clone(table);
        }
        let table = Rc::new(
            self.definitions
                .iter()
                .map(|(id, definition)| {
                    let image = |image: &DefinitionSpriteImage| {
                        HostSolidMaskImage::new(
                            image.width(),
                            image.height(),
                            image.solid_mask_source_pixels(),
                        )
                    };
                    let named_images = definition
                        .sprite_variant_keys()
                        .into_iter()
                        .filter_map(|name| {
                            definition.sprite_image_variant(Some(&name)).map(|sprite| {
                                (clonk_resources::material::c4_name_key(&name), image(sprite))
                            })
                        })
                        .collect();
                    (
                        id.clone(),
                        HostSolidMaskMetadata::new(
                            definition.shape_rect(),
                            definition.solid_mask(),
                            definition.rotated_solid_masks(),
                            definition.sprite_image().map(image),
                            named_images,
                        ),
                    )
                })
                .collect(),
        );
        *cache = Some(Rc::clone(&table));
        table
    }

    fn host_world_object(
        definitions: &rustc_hash::FxHashMap<DefinitionId, Definition>,
        object: &Object,
    ) -> HostWorldObject {
        Self::host_world_object_with_snapshot(
            definitions,
            object,
            Rc::new(object.script_state_snapshot()),
        )
    }

    fn host_world_object_with_snapshot(
        definitions: &rustc_hash::FxHashMap<DefinitionId, Definition>,
        object: &Object,
        state_snapshot: Rc<ObjectState>,
    ) -> HostWorldObject {
        Self::host_world_object_projection(definitions, object).with_full_state(state_snapshot)
    }

    fn host_world_object_projection(
        definitions: &rustc_hash::FxHashMap<DefinitionId, Definition>,
        object: &Object,
    ) -> HostWorldObject {
        #[cfg(test)]
        HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let definition = definitions.get(&object.definition_id);
        let procedure = definition
            .and_then(|definition| {
                definition.action_library().procedure_name_for_entry(
                    &object.state.action.name,
                    object.state.action.act_map_index,
                )
            })
            .map(str::to_string);
        // World objects expose the cached mask like C++ obj->OCF
        // (FindObject criteria, host functions).
        let ocf = object.state.ocf;
        HostWorldObject::with_category(
            object.id,
            object.definition_id.clone(),
            object.state.status,
            object.state.action.name.clone(),
            object.state.action.target,
            object.state.action.target2,
            procedure,
            object.state.owner,
            object.state.category,
            object.state.energy,
            object.state.construction,
            object.state.damage,
            object.state.position,
            object.state.velocity,
            object.state.rotation,
            object.state.vertices.clone(),
            object.state.action.data,
            object.state.action.time,
            object.state.action.phase,
            object.state.container,
            object.state.draw_transform,
        )
        .with_action_index(object.state.action.act_map_index)
        .with_unsorted(object.unsorted)
        .with_fixed_motion(object.fixed_position, object.fixed_velocity)
        .with_compiler_fields(
            object.motion_x,
            object.motion_y,
            object.last_attach_movement_frame,
            object.compiler_cache.clone(),
        )
        .with_fixed_rotation(object.fixed_rotation)
        .with_rotation_velocity(object.rotation_velocity)
        .with_own_vertices(object.own_shape_vertices.is_some())
        .with_move_to_range(definition.map_or(0, Definition::move_to_range))
        .with_pathfinder(definition.map_or(0, Definition::pathfinder))
        .with_no_transfer_zones(definition.map_or(0, Definition::no_transfer_zones))
        .with_no_push_enter(definition.map_or(0, Definition::no_push_enter))
        .with_contact_density(object.state.contact_density)
        .with_direction(object.state.direction.to_script_value())
        .with_selected(object.state.selected)
        .with_crew_disabled(object.state.crew_disabled)
        .with_contents(object.state.contents.clone())
        .with_alive(object.state.alive)
        .with_need_energy(object.state.need_energy)
        .with_collectible(definition.is_some_and(Definition::is_collectible))
        .with_collection_available_ignoring_delay(definition.is_some_and(|definition| {
            definition.collection_ocf_enabled(&object.state, object.state.contents.len(), 0)
        }))
        .with_collection_enabled(
            definition
                .is_some_and(|definition| definition.collection_ocf_enabled(&object.state, 0, 0)),
        )
        .with_no_collect_delay(object.state.no_collect_delay)
        .with_collection_limit(definition.map_or(0, Definition::collection_limit))
        .with_in_liquid(object.state.in_liquid)
        .with_ocf(ocf)
        .with_commands(object.commands.command_views())
        .with_command_stack(object.commands.snapshot())
        .with_compiled_mass(object.compiled_mass)
        .with_material_contents(object.material_contents.clone())
        .with_last_energy_loss_cause(object.last_energy_loss_cause)
    }

    pub(crate) fn host_retained_contents_count(
        world: &HostWorldContext,
        contents: &[ObjectId],
    ) -> usize {
        contents
            .iter()
            .filter(|content_id| {
                world
                    .get(**content_id)
                    .is_some_and(|object| object.is_present())
            })
            .count()
    }

    /// Materialize one callback object from an engine that is synchronously
    /// paused inside script execution.
    ///
    /// # Safety
    ///
    /// `source` must be the stable address of the originating `Engine`, its
    /// object-index cache must match `objects_generation`, and object storage
    /// may not change shape until the host context is dropped. A currently
    /// mutably borrowed object is seeded before entering script, so this
    /// function is never asked to dereference that entry.
    unsafe fn lazy_host_world_object(
        source: *const (),
        id: ObjectId,
    ) -> Option<(usize, HostWorldObject)> {
        let engine = source.cast::<Self>();
        // SAFETY: guaranteed by the provider contract above. Accessing the
        // index-cache field does not touch any exclusively borrowed object.
        let cache = unsafe { &*std::ptr::addr_of!((*engine).object_index_cache) }.borrow();
        let index = cache.1.get(&id).copied()?;
        drop(cache);
        // SAFETY: object-vector shape is frozen for the synchronous call and
        // the requested entry is not an outstanding exclusive seed.
        let objects = unsafe { &*std::ptr::addr_of!((*engine).objects) };
        let object = unsafe { &*objects.as_ptr().add(index) };
        if object.id != id {
            return None;
        }
        let definitions = unsafe { &*std::ptr::addr_of!((*engine).definitions) };
        Some((index, Self::host_world_object(definitions, object)))
    }

    /// Test the scalar fields read by legacy C4Game::FindObject without
    /// cloning the candidate's complete callback state.
    ///
    /// # Safety
    ///
    /// The `lazy_host_world_object` contract applies. Seeded/exclusively
    /// borrowed objects are resolved from the callback-local object store
    /// before this provider is called.
    unsafe fn lazy_host_world_object_matches(
        source: *const (),
        id: ObjectId,
        params: &compat::FindObjectParams,
    ) -> Option<bool> {
        let engine = source.cast::<Self>();
        // SAFETY: guaranteed by the provider contract above.
        let cache = unsafe { &*std::ptr::addr_of!((*engine).object_index_cache) }.borrow();
        let index = cache.1.get(&id).copied()?;
        drop(cache);
        // SAFETY: object storage is frozen and callback-local seeds prevent
        // this path from reading an outstanding exclusive object borrow.
        let objects = unsafe { &*std::ptr::addr_of!((*engine).objects) };
        let object = unsafe { &*objects.as_ptr().add(index) };
        (object.id == id).then(|| params.matches_engine_object(object))
    }

    /// Test a scalar C4FindObject criterion tree against the paused engine
    /// object without cloning its callback state.
    ///
    /// # Safety
    ///
    /// The `lazy_host_world_object` contract applies. Callback-local seeds
    /// are resolved before this provider is consulted.
    unsafe fn lazy_host_world_find_condition_matches(
        source: *const (),
        id: ObjectId,
        condition: &compat::FindCondition,
    ) -> Option<bool> {
        let engine = source.cast::<Self>();
        // SAFETY: guaranteed by the provider contract above.
        let cache = unsafe { &*std::ptr::addr_of!((*engine).object_index_cache) }.borrow();
        let index = cache.1.get(&id).copied()?;
        drop(cache);
        // SAFETY: object storage is frozen and callback-local seeds prevent
        // this path from reading an outstanding exclusive object borrow.
        let objects = unsafe { &*std::ptr::addr_of!((*engine).objects) };
        let object = unsafe { &*objects.as_ptr().add(index) };
        if object.id != id {
            return None;
        }
        let matches = condition.matches_engine_object(object)?;
        Some(object.state.status.is_active() && matches)
    }

    /// Fill every not-yet-seeded object in storage order.
    ///
    /// # Safety
    ///
    /// The `lazy_host_world_object` contract applies. In particular,
    /// `excluded` contains every object held through an outstanding exclusive
    /// borrow, and those indices are skipped before their storage is read.
    unsafe fn lazy_host_world_objects(
        source: *const (),
        excluded: &HashSet<usize>,
    ) -> Vec<(usize, HostWorldObject)> {
        let engine = source.cast::<Self>();
        let objects = unsafe { &*std::ptr::addr_of!((*engine).objects) };
        let definitions = unsafe { &*std::ptr::addr_of!((*engine).definitions) };
        let mut result = Vec::with_capacity(objects.len().saturating_sub(excluded.len()));
        for index in 0..objects.len() {
            if excluded.contains(&index) {
                continue;
            }
            // SAFETY: skipped indices are the only entries that may be
            // exclusively borrowed by the callback wrapper.
            let object = unsafe { &*objects.as_ptr().add(index) };
            result.push((index, Self::host_world_object(definitions, object)));
        }
        result
    }

    /// Select the paused-engine objects whose C4Object::ClearPointers fields
    /// may name `target`, without cloning their complete ObjectState. The
    /// caller resolves callback-local/exclusively borrowed entries from its
    /// own object store.
    ///
    /// # Safety
    ///
    /// The `lazy_host_world_object` contract applies. Every index in
    /// `excluded` is skipped before dereferencing object storage.
    unsafe fn lazy_host_world_pointer_referrers(
        source: *const (),
        target: ObjectId,
        excluded: &HashSet<usize>,
    ) -> Vec<(usize, ObjectId)> {
        let engine = source.cast::<Self>();
        let objects = unsafe { &*std::ptr::addr_of!((*engine).objects) };
        let target_number = i32::try_from(target.as_u64()).ok();
        let mut result = Vec::new();
        for index in 0..objects.len() {
            if excluded.contains(&index) {
                continue;
            }
            // SAFETY: skipped indices are the only entries that may be
            // exclusively borrowed by the callback wrapper.
            let object = unsafe { &*objects.as_ptr().add(index) };
            let references_target = object.state.action.target == Some(target)
                || object.state.action.target2 == Some(target)
                || object.state.layer == Some(target)
                || object.commands.command_views().iter().any(|command| {
                    command.target == Some(target) || command.target2 == Some(target)
                })
                || target_number.is_some_and(|target| {
                    object
                        .state
                        .effects
                        .iter()
                        .any(|effect| effect.command_target == Some(target))
                });
            if references_target {
                result.push((index, object.id));
            }
        }
        result
    }

    /// Select paused-engine objects whose persistent C4Values name `target`.
    /// Kept separate from ClearPointers because inactive objects retain
    /// ordinary script references.
    unsafe fn lazy_host_world_script_value_referrers(
        source: *const (),
        target: ObjectId,
        excluded: &HashSet<usize>,
    ) -> Vec<(usize, ObjectId)> {
        let engine = source.cast::<Self>();
        let objects = unsafe { &*std::ptr::addr_of!((*engine).objects) };
        let mut result = Vec::new();
        for index in 0..objects.len() {
            if excluded.contains(&index) {
                continue;
            }
            // SAFETY: skipped indices are the only entries that may be
            // exclusively borrowed by the callback wrapper.
            let object = unsafe { &*objects.as_ptr().add(index) };
            let references_target = object
                .state
                .local_vars
                .values()
                .any(|value| value.contains_object_reference(target.as_u64()))
                || object
                    .state
                    .effects
                    .iter()
                    .any(|effect| effect.contains_object_reference(target.as_u64()));
            if references_target {
                result.push((index, object.id));
            }
        }
        result
    }

    /// Project one C4Player into callback-local state on its first value
    /// query. Numeric validity and indexed order are seeded separately, so
    /// callbacks that do not inspect player data clone none of it.
    ///
    /// # Safety
    ///
    /// The engine is synchronously paused and its player map remains stable
    /// until the host context is dropped.
    unsafe fn lazy_host_world_player(source: *const (), id: i32) -> Option<PlayerState> {
        let engine = source.cast::<Self>();
        let players = unsafe { &*std::ptr::addr_of!((*engine).players) };
        players.get(&id).map(|player| {
            #[cfg(test)]
            HOST_WORLD_PLAYER_STATE_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
            player.to_state()
        })
    }

    /// Clone the landscape shell only when a callback first invokes a terrain
    /// host API.
    ///
    /// # Safety
    ///
    /// The engine is synchronously paused and its landscape is not mutably
    /// accessed until the callback outcome is replayed.
    unsafe fn lazy_host_world_landscape(source: *const ()) -> Option<Landscape> {
        #[cfg(test)]
        HOST_WORLD_LANDSCAPE_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let engine = source.cast::<Self>();
        unsafe { &*std::ptr::addr_of!((*engine).landscape) }.clone()
    }

    /// Borrow the landscape for read-only host queries. `GBackSolid` and the
    /// rest of C4Wrappers.h:66-92 read single pixels; copying the whole map to
    /// answer one was the single largest cost in `advance_tick` on real
    /// content. Terrain *writes* still go through
    /// [`Self::lazy_host_world_landscape`] and its private copy.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::lazy_host_world_landscape`], which already
    /// requires the source landscape to stay put and unmutated for the
    /// lifetime of every context carrying this provider.
    unsafe fn lazy_host_world_landscape_borrow(source: *const ()) -> Option<*const Landscape> {
        let engine = source.cast::<Self>();
        unsafe { &*std::ptr::addr_of!((*engine).landscape) }
            .as_ref()
            .map(std::ptr::from_ref)
    }

    /// Borrow `Game.Objects.Sectors` for a callback's read-only bounded-find
    /// walk. C++ queries the live sector lists directly
    /// (oracle-src-pinned src/C4FindObject.cpp:315-355); the callback-local
    /// context switches to an owned map before previewing any mutation.
    ///
    /// # Safety
    ///
    /// Same synchronous source-lifetime contract as
    /// [`Self::lazy_host_world_landscape_borrow`].
    unsafe fn lazy_host_world_sector_map_borrow(source: *const ()) -> Option<*const SectorMap> {
        let engine = source.cast::<Self>();
        unsafe { &*std::ptr::addr_of!((*engine).sectors) }
            .as_ref()
            .map(std::ptr::from_ref)
    }

    /// Report the landscape extent without copying the shell. Sector sizing is
    /// the only caller that used to force `lazy_host_world_landscape` for two
    /// integers, once per script call that reaches `FindObjects`.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::lazy_host_world_landscape`].
    unsafe fn lazy_host_world_landscape_dimensions(source: *const ()) -> Option<(i32, i32)> {
        let engine = source.cast::<Self>();
        unsafe { &*std::ptr::addr_of!((*engine).landscape) }
            .as_ref()
            .map(crate::compat::landscape_extent)
    }

    /// Snapshot `Game.Objects` from First -> Next only when a host API needs
    /// ordering. Seeded objects may be exclusively borrowed by the callback,
    /// so their copied status is authoritative and their source entry is not
    /// dereferenced.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::lazy_host_world_object`].
    unsafe fn lazy_host_world_master_order(
        source: *const (),
        seeded_statuses: &HashMap<ObjectId, ObjectStatus>,
        excluded: &HashSet<usize>,
    ) -> Vec<ObjectId> {
        #[cfg(test)]
        HOST_WORLD_MASTER_ORDER_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let engine = source.cast::<Self>();
        let objects = unsafe { &*std::ptr::addr_of!((*engine).objects) };
        let exec_list = unsafe { &*std::ptr::addr_of!((*engine).exec_list) };
        let index_cache = unsafe { &*std::ptr::addr_of!((*engine).object_index_cache) }.borrow();
        let mut master_order = Vec::with_capacity(exec_list.len());
        master_order.extend(exec_list.iter().rev().copied().filter(|id| {
            if let Some(status) = seeded_statuses.get(id) {
                return *status != ObjectStatus::Inactive;
            }
            let cached_index = index_cache
                .1
                .get(id)
                .copied()
                .filter(|&index| index < objects.len() && !excluded.contains(&index));
            let cached_index = cached_index.filter(|&index| {
                // SAFETY: the index is in bounds and not among the entries
                // held through an outstanding callback-local `&mut`.
                unsafe { &*objects.as_ptr().add(index) }.id == *id
            });
            // The provider contract keeps the cache current. Retain an
            // identity-checked fallback for diagnostics/tests that inject a
            // stale cache, matching `find_object_index`'s fail-safe behavior.
            let index = cached_index.or_else(|| {
                (0..objects.len()).find(|index| {
                    if excluded.contains(index) {
                        return false;
                    }
                    // SAFETY: checked in bounds and excluded callback-local
                    // mutable entries before constructing this shared view.
                    unsafe { &*objects.as_ptr().add(*index) }.id == *id
                })
            });
            let Some(index) = index else {
                return seeded_statuses
                    .get(id)
                    .is_some_and(|status| *status != ObjectStatus::Inactive);
            };
            if excluded.contains(&index) {
                return seeded_statuses
                    .get(id)
                    .is_some_and(|status| *status != ObjectStatus::Inactive);
            }
            // SAFETY: object-vector shape is frozen and exclusively borrowed
            // entries were rejected above.
            let object = unsafe { &*objects.as_ptr().add(index) };
            #[cfg(test)]
            HOST_WORLD_MASTER_ORDER_SOURCE_STATUS_READS.with(|count| count.set(count.get() + 1));
            object.state.status != ObjectStatus::Inactive
        }));
        master_order
    }

    pub(crate) fn note_solid_mask_host_state_changed(&self) {
        self.solid_mask_host_state_generation
            .set(self.solid_mask_host_state_generation.get().wrapping_add(1));
    }

    fn host_solid_mask_state(&self) -> SolidMaskHostStateCache {
        let generation = self.solid_mask_host_state_generation.get();
        if let Some(cached) = self
            .solid_mask_host_state_cache
            .borrow()
            .as_ref()
            .filter(|cached| cached.generation == generation)
        {
            return cached.clone();
        }

        let mut bakes = Vec::new();
        let mut instance_sequences = HashMap::new();
        for object in &self.objects {
            #[cfg(test)]
            HOST_SOLID_MASK_STATE_OBJECT_VISITS.with(|count| count.set(count.get() + 1));
            if let Some(bake) = &object.solid_mask_bake {
                bakes.push((object.id, bake.clone()));
            }
            if let Some(sequence) = object.solid_mask_instance_sequence {
                instance_sequences.insert(object.id, sequence);
            }
        }
        let cached = SolidMaskHostStateCache {
            generation,
            bakes: Rc::new(bakes),
            instance_sequences: Rc::new(instance_sequences),
            next_instance_sequence: self.solid_mask_staging.next_solid_mask_instance_sequence,
        };
        *self.solid_mask_host_state_cache.borrow_mut() = Some(cached.clone());
        cached
    }

    /// Build the shared/static portion of a script host context without
    /// materializing every object's mutable script state or cloning the
    /// landscape shell. Movement can finish this lazily on first contact.
    fn host_world_context_base(&self) -> HostWorldContext {
        #[cfg(test)]
        HOST_WORLD_CONTEXT_BASE_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        self.record_effect_dispatch(|stats| stats.context_base_materializations += 1);
        let definition_metadata = self.definition_metadata_table();
        let host_definition_tables = self.host_definition_tables();
        let reloadable_definitions = Rc::clone(&host_definition_tables.reloadable_definitions);
        let solid_mask_metadata = self.solid_mask_metadata_table();
        let transfer_zones = self.transfer_zones.states();
        let player_order = self.player_ids_in_order();
        let local_players: Vec<i32> = self.local_players.as_ref().map_or_else(
            || player_order.clone(),
            |players| players.iter().copied().collect(),
        );
        let solid_mask_state = self.host_solid_mask_state();
        let sky_adjustment = self
            .sky
            .as_ref()
            .map(SkyState::adjustment)
            .unwrap_or_default();
        let sky_fade = self.sky.as_ref().map_or_else(
            || {
                let settings = SkySettings::default();
                [settings.fade_top, settings.fade_bottom]
            },
            |sky| [sky.settings().fade_top, sky.settings().fade_bottom],
        );
        let mut world = HostWorldContext::with_landscape_shared(
            std::iter::empty(),
            None,
            definition_metadata,
            Rc::clone(&self.scenario_values),
            Rc::clone(&self.default_rank_names),
            transfer_zones,
            HashMap::new(),
            HashMap::new(),
            self.next_object_id,
            self.team_home_base_rule,
        )
        .with_player_fow_view_objects(
            self.players
                .values()
                .map(|player| (player.id(), player.fow_view_objects().iter().copied())),
        )
        .with_game_time(self.game_time)
        .with_needed_material_strings(Rc::clone(&self.needed_material_strings))
        .with_object_no_dig_resource_string(Rc::clone(&self.object_no_dig_resource_string))
        .with_construction_check_strings(Rc::clone(&self.construction_check_strings))
        .with_control_key_names(Rc::clone(&self.control_key_names))
        .with_solid_mask_metadata(solid_mask_metadata)
        .with_shared_solid_mask_bakes(Rc::clone(&solid_mask_state.bakes))
        .with_solid_mask_instance_sequences(
            solid_mask_state.instance_sequences.as_ref().clone(),
            solid_mask_state.next_instance_sequence,
        )
        .with_scenario_sections(
            self.scenario_sections
                .values()
                .map(|section| section.name.as_str()),
        )
        .with_teams(Rc::clone(&self.team_state.teams))
        .with_team_runtime_options(self.team_state.team_configuration, self.league_game)
        .with_game_tick_delay(
            Rc::clone(&self.game_tick_delay_ms),
            Rc::clone(&self.game_tick_delay_revision),
        )
        .with_league_progress_data(
            Rc::clone(&self.league_name),
            Rc::clone(&self.player_info_league_progress_data),
        )
        .with_player_info_ids(self.players.values().map(Player::player_info_id))
        .with_league_scores(Rc::clone(&self.player_info_league_scores))
        .with_movement_solid_masks(self.ocf_solid_mask_overlay())
        .with_definition_order(Rc::clone(&self.runtime_definition_order))
        .with_definition_tables(
            host_definition_tables,
            self.base_auto_sell_enabled,
            self.host_crew_info_state(),
        )
        .with_shared_particle_defs(self.particle_system.shared_def_names())
        .with_shared_particle_reloads(
            self.particle_system.shared_reloadable_def_names(),
            Rc::clone(&self.host_requests.particle_reload_requests),
        )
        .with_shared_particle_reload_io_success(
            self.particle_system.shared_reloadable_def_io_success(),
        )
        .with_definition_reloads(
            reloadable_definitions,
            Rc::clone(&self.host_requests.definition_reload_requests),
        )
        .with_crew_ranks(Rc::clone(&self.crew_ranks))
        .with_crew_infos(Rc::clone(&self.crew_object_infos))
        .with_crew_info_links(Rc::clone(&self.crew_info_links))
        .with_materials(Some(self.materials_shared()))
        .with_scenario_script(
            self.scenario_script
                .as_ref()
                .map(ScenarioScript::script_arc),
        )
        .with_network_game(self.network_game)
        .with_network_control_mode(self.network_control_mode)
        .with_control_sync_mode(self.control_sync_mode())
        .with_edit_cursor_target(self.edit_cursor_target)
        .with_pause_game_requests(
            self.replay_control,
            Rc::clone(&self.host_requests.pause_game_requests),
        )
        .with_network_target_fps_requests(Rc::clone(
            &self.host_requests.network_target_fps_requests,
        ))
        .with_viewport_presentation_requests(
            self.replay_control,
            Rc::clone(&self.host_requests.viewport_presentation_requests),
        )
        .with_film_viewport_available(self.film_viewport_available)
        .with_smoke_level(self.bubble_smoke_level())
        .with_fire_particles_loaded(self.particle_system.is_fire_particle_loaded())
        .with_max_players(self.max_players.unwrap_or_default())
        .with_fair_crew_parameters(self.use_fair_crew, self.fair_crew_strength)
        .with_fair_crew_physical_cache(Rc::clone(&self.fair_crew_physical_cache))
        .with_control_host(
            self.control_host,
            Rc::clone(&self.host_requests.player_info_updates),
        )
        .with_live_player_order(player_order)
        .with_local_players(local_players)
        .with_shared_physical_viewport_players(Rc::clone(&self.physical_viewport_players))
        .with_active_message_board_input(self.active_message_board_input.clone())
        .with_mission_access(Rc::clone(&self.mission_access.inner))
        .with_scoreboard(Rc::clone(&self.scoreboard))
        .with_scoreboard_presentations(Rc::clone(&self.scoreboard_presentations))
        .with_scenario_script_counter(self.scenario_script_counter)
        .with_pathfinder_settings(
            self.pathfinder_level,
            self.pathfinder_transfer_zones_enabled,
        )
        .with_pathfinder_debug_sink(Rc::clone(&self.pathfinder_debug))
        .with_command_settings(
            self.frame,
            self.base_buy_enabled,
            self.base_sell_enabled,
            self.base_reject_entrance_enabled,
            self.base_extinguish_enabled,
        )
        .with_structures_need_energy(self.structures_need_energy)
        .with_flag_removeable(self.flag_removeable)
        .with_sky_adjustment(sky_adjustment)
        .with_sky_fade(sky_fade[0], sky_fade[1]);
        if self.solid_mask_staging.defer_solid_mask_updates {
            if let Some(preview) = self.solid_mask_staging.deferred_host_raster_preview.clone() {
                world.apply_host_raster_preview(preview);
            }
        }
        world
    }

    pub(crate) fn host_world_context(&self) -> HostWorldContext {
        self.record_effect_dispatch(|stats| stats.world_context_builds += 1);
        // Freeze the generation-stamped id lookup before a callback may hold
        // one object through `&mut`. Script-side spawns/removals are staged and
        // cannot change this vector until the context has been dropped.
        if let Some(first) = self.objects.first() {
            let _ = self.find_object_index(first.id);
        }
        // SAFETY: every use of this private context is synchronous. The
        // callback wrappers drop it before replaying any operation that may
        // move object storage or replace the authoritative landscape.
        let provider = unsafe {
            LazyHostWorldProvider::new(
                std::ptr::from_ref(self).cast(),
                Self::lazy_host_world_object,
                Self::lazy_host_world_objects,
                Self::lazy_host_world_landscape,
            )
            .with_pointer_referrers(Self::lazy_host_world_pointer_referrers)
            .with_script_value_referrers(Self::lazy_host_world_script_value_referrers)
            // `exec_list` stores C++ Game.Objects reversed for Last -> Prev
            // execution. APIs such as FindBase walk the forward list, but
            // most callbacks never inspect it, so snapshot it on first use.
            .with_master_order(Self::lazy_host_world_master_order)
            .with_player(Self::lazy_host_world_player)
            .with_landscape_dimensions(Self::lazy_host_world_landscape_dimensions)
            .with_landscape_borrow(Self::lazy_host_world_landscape_borrow)
            .with_sector_map_borrow(Self::lazy_host_world_sector_map_borrow)
            .with_legacy_find_object(Self::lazy_host_world_object_matches)
            .with_find_condition(Self::lazy_host_world_find_condition_matches)
        };
        self.host_world_context_base()
            .with_lazy_world_provider(provider)
    }

    pub(crate) fn host_world_context_for_object(&self, index: usize) -> HostWorldContext {
        self.record_effect_dispatch(|stats| stats.object_state_snapshots += 1);
        let state_snapshot = Rc::new(self.objects[index].script_state_snapshot());
        self.host_world_context_for_object_with_snapshot(index, state_snapshot)
    }

    pub(crate) fn host_world_context_for_object_with_snapshot(
        &self,
        index: usize,
        state_snapshot: Rc<ObjectState>,
    ) -> HostWorldContext {
        self.host_world_context().with_seeded_object(
            index,
            Self::host_world_object_with_snapshot(
                &self.definitions,
                &self.objects[index],
                state_snapshot,
            ),
        )
    }

    /// C4Game::NewObject links a fresh object into Game.Objects before its
    /// Construction/Initialize/effect callbacks. Rust still owns that object
    /// as a local until those phases finish, so seed both lookup storage and
    /// the forward master-list position into their callback world.
    pub(crate) fn host_world_context_for_pending_object(
        &self,
        object: &Object,
        exec_position: usize,
    ) -> HostWorldContext {
        let mut world = self.host_world_context();
        let mut master_order = world.master_object_ids().to_vec();
        if !master_order.contains(&object.id) {
            let master_position = master_order
                .len()
                .saturating_sub(exec_position.min(master_order.len()));
            master_order.insert(master_position, object.id);
        }
        world = world.with_master_order(master_order);
        world.seed_object(
            self.objects.len(),
            Self::host_world_object(&self.definitions, object),
        );
        world
    }

    /// The shared definition-script table host contexts carry (nested
    /// obj->Method resolution; see host_world_context).
    pub(crate) fn definition_script_table(&self) -> HashMap<DefinitionId, Arc<ScriptEngine>> {
        self.definitions
            .iter()
            .map(|(id, definition)| (id.clone(), definition.script_arc()))
            .collect()
    }

    pub fn clear_scenario_script(&mut self) {
        self.scenario_script = None;
        for definition in self.definitions.values_mut() {
            definition.set_game_script_name("Script.c");
        }
        for source in &mut self.script_link_sources {
            if let ScriptLinkSource::Script { script, .. } = source {
                Arc::make_mut(script).set_game_script_name("Script.c");
            }
        }
        self.script_link_sources
            .retain(|source| !matches!(source, ScriptLinkSource::Scenario));
    }

    /// Enters C4GUI's shared in-game phase for scoreboard presentation.
    /// Ordered script requests produced while startup/save loading was
    /// exclusive are discarded, matching C4Scoreboard::DoDlgShow's GUI guard.
    pub fn begin_scoreboard_presentation_capture(&mut self) {
        self.scoreboard_presentations
            .borrow_mut()
            .begin_runtime_capture();
    }

    /// Returns the current live scoreboard model without consuming any
    /// ordered dialog-lifecycle requests.
    pub fn scoreboard_snapshot(&self) -> ScoreboardState {
        self.scoreboard.borrow().clone()
    }

    /// Runtime C4Scoreboard::SetCell invalidation generation. Sorting changes
    /// row order without invalidating native geometry, so it deliberately
    /// does not advance this value.
    pub fn scoreboard_layout_revision(&self) -> u64 {
        self.scoreboard_presentations.borrow().layout_revision()
    }

    /// Drains ordered runtime `DoScoreboardShow` presentation requests.
    /// Synchronous input/menu callbacks may execute script between ticks, so
    /// the app must also consume this queue before scoreboard input/render.
    pub fn take_scoreboard_presentations(&mut self) -> Vec<ScoreboardPresentationRequest> {
        self.scoreboard_presentations.borrow_mut().drain()
    }

    pub fn install_scenario_script(
        &mut self,
        name: impl Into<String>,
        source: &str,
    ) -> Result<Vec<ObjectId>, EngineError> {
        self.install_scenario_script_with_convention(name, source, false)
    }

    /// Like [`install_scenario_script`], with the callback convention:
    /// `c4_args = true` for real (legacy) content — C++ passes no
    /// synthetic state argument and has no fixture Step calls.
    pub fn install_scenario_script_with_convention(
        &mut self,
        name: impl Into<String>,
        source: &str,
        c4_args: bool,
    ) -> Result<Vec<ObjectId>, EngineError> {
        self.load_scenario_script_with_convention(name, source, c4_args)?;
        self.initialize_scenario_script()
    }

    /// `C4Console::EditScript` (`C4Console.cpp:1335-1342`) — commit an edited
    /// scenario script body and relink the whole tree.
    ///
    /// Two details a from-scratch version loses:
    ///
    /// - **No `Initialize`.** C++ only replaces the host's `Data` and relinks;
    ///   the scenario is already running, so re-running `Initialize` here would
    ///   recreate its objects. That is why this uses the load-without-init path
    ///   rather than [`Self::install_scenario_script`].
    /// - **The relink is unconditional.** `Game.ScriptEngine.ReLink(&Game.Defs)`
    ///   sits *outside* the `#ifdef _WIN32`, so it runs even where the dialog
    ///   never opened and even when the user cancelled — see
    ///   [`Self::relink_after_component_edit`] for that arm.
    ///
    /// Refusing the edit in a network game is the caller's gate
    /// (`if (Game.Network.isEnabled()) return;`, `:1336`), modelled by
    /// [`crate::developer_components::component_editor_available`].
    pub fn apply_scenario_script_edit(
        &mut self,
        name: impl Into<String>,
        source: &str,
    ) -> Result<(), EngineError> {
        self.load_scenario_script_with_convention(name, source, true)?;
        self.relink_scripts()
    }

    /// The relink `C4Console::EditScript` performs when the editor changed
    /// nothing — cancelled, or on a build where the dialog does not exist.
    /// C++ still runs it (`C4Console.cpp:1341`).
    pub fn relink_after_component_edit(&mut self) -> Result<(), EngineError> {
        self.relink_scripts()
    }

    /// Loads and registers a scenario script without calling Initialize.
    /// C++ performs this before loading the scenario-local System.c4g and
    /// linking, while Initialize runs later from InitGame.
    pub fn load_scenario_script_with_convention(
        &mut self,
        name: impl Into<String>,
        source: &str,
        c4_args: bool,
    ) -> Result<(), EngineError> {
        let name = name.into();
        let mut script = ScenarioScript::from_source(name, source)?;
        script.c4_args = c4_args;
        let game_script_name = script.script.script_name().to_owned();
        for definition in self.definitions.values_mut() {
            definition.set_game_script_name(game_script_name.clone());
        }
        for source in &mut self.script_link_sources {
            if let ScriptLinkSource::Script { script, .. } = source {
                Arc::make_mut(script).set_game_script_name(game_script_name.clone());
            }
        }
        // C4Aul's preparser installs Ref (non-Hold) static-constant strings
        // before its later function-body parse registers held operands.
        if let Err(diagnostic) = clonk_script::register_global_declarations_with_strings(
            script.base_script.var_decls(),
            &self.script_globals,
            Some(&self.script_global_consts),
            &self.script_string_registrations,
        ) {
            tracing::warn!(
                script = %script.name,
                %diagnostic,
                "scenario static-constant link diagnostic; continuing like C++"
            );
        }
        {
            let host = Arc::make_mut(&mut script.script);
            host.set_global_variables(self.script_globals.clone());
            host.set_global_slots(self.script_global_slots.clone());
            host.set_global_constants(self.script_global_consts.clone());
            host.set_string_registrations_deferred(self.script_string_registrations.clone());
            host.adopt_statics_into_globals();
        }
        let scenario_globals: Vec<(String, clonk_script::Function)> = script
            .script
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect();
        let scenario_global_order = script
            .script
            .global_function_names_in_link_order()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if !self
            .script_link_sources
            .iter()
            .any(|source| matches!(source, ScriptLinkSource::Scenario))
        {
            self.script_link_sources.push(ScriptLinkSource::Scenario);
        }
        // The scenario script's `global func`s are engine-global like any
        // other script's (C4AulScriptEngine owns AA_GLOBAL functions from
        // EVERY linked script): GoldRush's FxStayThere*/DoInitialize live
        // there and must resolve from def scripts and effect callbacks.
        let mut functions: rustc_hash::FxHashMap<String, clonk_script::Function> = self
            .global_script_functions
            .as_deref()
            .cloned()
            .unwrap_or_default();
        let mut changed = false;
        for (name, mut function) in scenario_globals {
            if let Some(previous) = functions.remove(&name) {
                function.push_overload(previous);
            }
            Arc::make_mut(&mut script.script).link_global_access_function(&name, function.clone());
            functions.insert(name, function);
            changed = true;
        }
        if changed {
            let table = Some(Arc::new(functions));
            let mut function_order = self.global_script_function_order.clone();
            function_order.extend(scenario_global_order);
            self.distribute_global_script_functions(table, function_order);
            self.definition_metadata_cache.borrow_mut().take();
            self.solid_mask_metadata_cache.borrow_mut().take();
        }
        script.set_global_functions(self.global_script_functions.clone());
        self.scenario_script = Some(script);
        Ok(())
    }

    /// C4Game::InitGame's per-definition `~InitializeDef` pass. Definitions
    /// run in numeric C4ID order after Objects.txt has been denumerated and
    /// before environment placement (C4Game.cpp:112, 2505-2520). Calls are
    /// fail-safe, but host side effects made before an error still commit.
    pub(crate) fn initialize_definition_scripts(&mut self) -> Result<Vec<ObjectId>, EngineError> {
        let definition_ids = Rc::clone(&self.runtime_definition_order);
        let mut created = Vec::new();
        for definition_id in definition_ids.iter() {
            let Some((script_name, script)) = self
                .definitions
                .get(definition_id)
                .filter(|definition| definition.script.has_function("InitializeDef"))
                .map(|definition| (definition.id.clone(), definition.script_arc()))
            else {
                continue;
            };

            let world = self.host_world_context();
            let (_value, _args, batch, audio_state, rng, script_error) =
                ScenarioScript::call_value_for_script(
                    &script_name,
                    &script,
                    Some(definition_id.clone()),
                    "InitializeDef",
                    &[Value::Nil],
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
            created.extend(self.apply_scenario_batch(batch)?);
            if let Some(error) = script_error {
                if !matches!(error, EngineError::Script { .. }) {
                    return Err(error);
                }
            }
        }
        Ok(created)
    }

    pub fn initialize_scenario_script(&mut self) -> Result<Vec<ObjectId>, EngineError> {
        self.finalize_legacy_object_links()?;
        self.initialize_scenario_script_after_restored_object_links()
    }

    /// Invoke the scenario constructor after InitGameFinal's restored-object
    /// link phase was already forced, including the zero-player case.
    pub fn initialize_scenario_script_after_restored_object_links(
        &mut self,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let Some(c4_args) = self.scenario_script.as_ref().map(|script| script.c4_args) else {
            return Ok(Vec::new());
        };
        let snapshot = self.snapshot();
        let world = self.host_world_context();
        // The `random` Initialize argument is the command-DSL fixture
        // convention — real content (c4_args) burns no synced draw
        // (C++ passes no such argument).
        let random = if c4_args { 0 } else { self.next_random_i32() };
        let rng_state = self.rng.clone();
        let global_effects = self.global_effects.clone();
        let physics = self.physics;
        let environment = self.environment;
        let audio_registry = self.audio_registry.clone();
        let particle_defs = self.particle_system.def_names();
        let definition_scripts = self.definition_script_table();
        let definition_metadata_table = self.definition_metadata_table();
        let definition_order = Rc::clone(&self.runtime_definition_order);
        let network_game = self.network_game;
        let next_object_id = self.next_object_id;
        let scenario_script_counter = self.scenario_script_counter;
        let scoreboard = Rc::clone(&self.scoreboard);
        let materials = self.materials_shared();
        let Some(script) = self.scenario_script.as_mut() else {
            return Ok(Vec::new());
        };
        let (batch, audio_state, new_rng, script_error) = script.initialize(
            &snapshot,
            world,
            scoreboard,
            materials,
            rng_state,
            random,
            &global_effects,
            physics,
            environment,
            audio_registry,
            particle_defs,
            definition_scripts,
            definition_metadata_table,
            definition_order,
            network_game,
            next_object_id,
            scenario_script_counter,
        )?;
        // Initialize is a game call: a script error logs and the scenario
        // still runs WITH its script installed (C++ fail-safe exec,
        // C4AulExec.cpp:1318-1342). Host mutations made before the error
        // already happened and therefore apply before the error is logged.
        self.rng = new_rng;
        self.audio_registry = audio_state;
        let created = self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            tolerate_script_error::<()>(Err(error))?;
        }
        self.game_over_triggered = false;
        self.game_evaluated = false;
        Ok(created)
    }

    pub(crate) fn broadcast_scenario_function(
        &mut self,
        function: &str,
        extra_args: Vec<Value>,
    ) -> Result<(), EngineError> {
        // C4GameScriptHost::GRBroadcast (C4ScriptHost.cpp:234-249): every
        // live object with a C4D_Goal|C4D_Rule|C4D_Environment category bit
        // is called FIRST ("call objects first - scenario script might
        // overwrite hostility, etc."), results discarded; the scenario
        // script runs after. Object-call errors log-and-continue
        // (fPassError defaults false).
        const BROADCAST_MASK: i32 = (1 << 5) | (1 << 6) | (1 << 19);
        // Game.Objects is the active master list and GRBroadcast walks it
        // First -> Next. `exec_list` stores that list reversed. Snapshot
        // identities, not storage indices: callbacks may append objects or
        // otherwise mutate the list, while later snapshotted objects must be
        // resolved and tested against their live state at their turn.
        let broadcast_targets: Vec<ObjectId> = self
            .exec_list
            .iter()
            .rev()
            .copied()
            // Rust retains an inactive object's old exec-list ledger slot;
            // C++ has already moved that link to InactiveObjects.
            .filter(|&id| {
                self.find_object_index(id)
                    .is_some_and(|index| self.objects[index].state.status != ObjectStatus::Inactive)
            })
            .collect();
        for object_id in broadcast_targets {
            let Some(index) = self.find_object_index(object_id) else {
                continue;
            };
            // C++ reads both fields at the current link. An earlier callback
            // may remove this object, demote it, or promote a previously
            // ineligible later object into the broadcast mask.
            if self.objects[index].destroyed
                || !self.objects[index].state.status.is_active()
                || self.objects[index].state.category & BROADCAST_MASK == 0
            {
                continue;
            }
            let definition_id = self.objects[index].definition_id.clone();
            // A missing function is no error (GetSFunc miss → C4Value()).
            let has_function = self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.has_function(function))
                .unwrap_or(false);
            if !has_function {
                continue;
            }
            let _ = tolerate_script_error(self.call_object_function(
                index,
                function,
                extra_args.clone(),
            ))?;
        }

        self.call_scenario_script_function(function, extra_args)
    }

    /// Runs a function on the SCENARIO SCRIPT ONLY (Game.Script.Call) —
    /// the direct-call half of GRBroadcast, also used by the Script%d
    /// counter sections (C4GameScriptHost::Execute, C4ScriptHost.cpp:
    /// 222-232).
    #[doc(hidden)]
    pub fn call_scenario_script_function(
        &mut self,
        function: &str,
        mut extra_args: Vec<Value>,
    ) -> Result<(), EngineError> {
        if self.scenario_script.is_none() {
            return Ok(());
        }
        let snapshot = self.snapshot();
        let world = self.host_world_context();
        let c4_args = self
            .scenario_script
            .as_ref()
            .map(|script| script.c4_args)
            .unwrap_or(false);
        let mut args = Vec::with_capacity(extra_args.len() + 1);
        // GRBroadcast passes the C++ argument list as-is (e.g.
        // PSF_InitializePlayer starts with the player number,
        // C4Player.cpp:769-775); the state proplist is fixture-only.
        if !c4_args {
            args.push(build_scenario_state_value(&snapshot));
        }
        args.append(&mut extra_args);
        let rng_state = self.rng.clone();
        let env_frame = self.frame;
        let global_effects = self.global_effects.clone();
        let physics = self.physics;
        let environment = self.environment;
        let audio_state = self.audio_registry.clone();
        let particle_defs = self.particle_system.def_names();
        let definition_scripts = self.definition_script_table();
        let definition_metadata_for_call = self.definition_metadata_table();
        let definition_order = Rc::clone(&self.runtime_definition_order);
        let network_game = self.network_game;
        let engine_next_object_id = self.next_object_id;
        let scenario_script_counter = self.scenario_script_counter;
        let scoreboard = Rc::clone(&self.scoreboard);
        let materials = self.materials_shared();
        let script = match self.scenario_script.as_mut() {
            Some(script) if script.has_function(function) => script,
            Some(_) => return Ok(()),
            None => unreachable!("scenario script must be present"),
        };
        let (batch, audio_state, new_rng, script_error) = script.call_raw(
            function,
            args,
            &snapshot,
            world,
            scoreboard,
            materials,
            rng_state,
            env_frame,
            &global_effects,
            physics,
            environment,
            audio_state,
            particle_defs,
            definition_scripts,
            definition_metadata_for_call,
            definition_order,
            network_game,
            engine_next_object_id,
            scenario_script_counter,
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        // Partial side effects fold BEFORE the error surfaces: C++
        // mutates live state as the script runs — GoldRush's Script1
        // creates the intro Talker before any later line can fail.
        let _ = self.apply_scenario_batch(batch)?;
        match script_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn scenario_local_function_names(&self) -> HashSet<String> {
        self.scenario_script
            .as_ref()
            .map(ScenarioScript::local_function_names)
            .unwrap_or_default()
    }

    pub(crate) fn run_post_init_map_callbacks(
        &mut self,
        fallback: &map_creator_s2::PostInitMapCallbacks,
    ) -> Result<(), EngineError> {
        let array_count = self
            .live_post_init_map_callbacks()
            .unwrap_or(fallback)
            .array_count();
        for array in 0..array_count {
            let size = self
                .live_post_init_map_callbacks()
                .unwrap_or(fallback)
                .array_size(array);
            for index in (0..size).rev() {
                let invocation = self
                    .live_post_init_map_callbacks()
                    .unwrap_or(fallback)
                    .invocation_at(array, index);
                let Some((function, args)) = invocation else {
                    continue;
                };
                let args = args.into_iter().map(Value::Int).collect();
                let _ = tolerate_script_error(self.call_scenario_script_function(&function, args))?;
            }
        }
        Ok(())
    }

    fn live_post_init_map_callbacks(&self) -> Option<&map_creator_s2::PostInitMapCallbacks> {
        self.landscape
            .as_ref()
            .and_then(Landscape::raster_state)
            .and_then(crate::landscape::LandscapeRasterState::map_creator)
            .map(map_creator_s2::MapCreatorS2State::callbacks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_host_contexts_reuse_unchanged_solid_mask_state() {
        let mut engine = Engine::new();
        engine
            .register_script_definition("Plain", "Plain", "func Noop() { return 0; }")
            .expect("plain definition registers");
        for _ in 0..2 {
            engine
                .spawn_object(SpawnConfig::new("Plain"))
                .expect("plain object spawns");
        }

        HOST_SOLID_MASK_STATE_OBJECT_VISITS.with(|count| count.set(0));
        let _ = engine.host_world_context();
        let _ = engine.host_world_context();

        assert_eq!(HOST_SOLID_MASK_STATE_OBJECT_VISITS.with(Cell::get), 2);
    }

    #[test]
    fn callback_contexts_share_unchanged_solid_mask_bakes() {
        let mut engine = Engine::new();
        engine
            .register_script_definition("Plain", "Plain", "func Noop() { return 0; }")
            .expect("plain definition registers");
        let object = engine
            .spawn_object(SpawnConfig::new("Plain"))
            .expect("plain object spawns");
        let index = engine
            .find_object_index(object)
            .expect("plain object exists");

        HOST_SOLID_MASK_BAKE_VECTOR_CLONES.with(|count| count.set(0));
        engine
            .call_object_function(index, "Noop", Vec::new())
            .expect("first callback succeeds");
        engine
            .call_object_function(index, "Noop", Vec::new())
            .expect("second callback succeeds");

        assert_eq!(HOST_SOLID_MASK_BAKE_VECTOR_CLONES.with(Cell::get), 0);
    }
}
