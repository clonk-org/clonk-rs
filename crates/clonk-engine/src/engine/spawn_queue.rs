//! `impl Engine` — the object spawn queue.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

/// The trailing queue holds one exact `Game.Objects` list per creation phase
/// that mutated it, oldest first: Construction, Initialize, then the deferred
/// effect batch. A single final list cannot describe more than one phase.
type SpawnSingleOutcome = (
    ObjectId,
    Vec<SpawnConfig>,
    Vec<compat::NestedObjectOutcome>,
    VecDeque<compat::EffectObjectListPreview>,
);

impl Engine {
    fn spawn_single(&mut self, config: SpawnConfig) -> Result<SpawnSingleOutcome, EngineError> {
        self.spawn_single_inner(config, None)
    }

    pub(crate) fn spawn_single_inner(
        &mut self,
        config: SpawnConfig,
        initial_info_physical: Option<PhysicalInfo>,
    ) -> Result<SpawnSingleOutcome, EngineError> {
        let SpawnConfig {
            id: explicit_id,
            definition_id,
            custom_name,
            position,
            velocity,
            motion_x,
            motion_y,
            compiler_cache,
            fixed_velocity,
            rotation,
            energy,
            damage,
            need_energy,
            magic_energy,
            construction,
            action,
            action_sound_dispatched,
            action_sound_selection,
            direction,
            command_direction,
            effects,
            temporary_physical,
            physical_changes,
            breath,
            vertices,
            shape_vertices: saved_shape_vertices,
            owns_shape_vertices,
            shape_rect: saved_shape_rect,
            contact_density,
            shape_fire_top,
            shape_attach,
            components,
            component_order,
            owner,
            controller,
            crew_member,
            crew_disabled,
            plr_view_range,
            selected,
            status,
            container,
            layer,
            visibility,
            blit_mode,
            picture_rect,
            color,
            color_modulation,
            base_graphics,
            graphics_overlays,
            draw_transform,
            alive,
            category,
            in_liquid,
            entrance_status,
            fixed_position,
            fixed_rotation,
            rotation_velocity,
            mobile,
            timer,
            own_mass,
            compiled_mass,
            on_fire,
            fire_phase,
            fire_caused_by,
            last_attach_movement_frame,
            last_energy_loss_cause,
            no_collect_delay,
            base,
            compiled_ocf: _compiled_ocf,
            command_stack,
            local_vars,
            loaded,
            native_compiled_object_defaults,
            solid_mask,
            solid_mask_instance_sequence,
            position_adjusted,
            initialized,
        } = config;
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

        let (
            action_library,
            definition_category,
            default_action_state,
            definition_vertices,
            definition_vertex_slots,
            definition_shape_rect,
            definition_fire_top,
            definition_stretch_growth,
            definition_oversize,
            definition_rotateable,
            definition_line,
            definition_blit_mode,
            definition_color_by_owner,
            definition_components,
            definition_contact_density,
            definition_incomplete_activity,
        ) = {
            let definition_ref = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            (
                definition_ref.action_library().clone(),
                definition_ref.category(),
                definition_ref.default_action_state(),
                definition_ref.shape_vertices().to_vec(),
                definition_ref.shape_vertex_buffer().clone(),
                definition_ref.shape_rect(),
                definition_ref.fire_top(),
                definition_ref.stretch_growth(),
                definition_ref.oversize(),
                definition_ref.rotateable(),
                definition_ref.line(),
                definition_ref.blit_mode(),
                definition_ref.color_by_owner(),
                definition_ref.components().to_vec(),
                definition_ref.contact_density(),
                definition_ref.incomplete_activity(),
            )
        };
        // Objects.txt compiles Con/Size verbatim. Fresh non-Oversize
        // objects retain NewObject's FullCon ceiling; Oversize definitions
        // retain construction-scaled values above 100 percent.
        let construction = if loaded {
            construction
        } else if definition_oversize {
            construction.max(0)
        } else {
            construction.clamp(0, FULL_CON)
        };
        let mut initial_action = match action {
            Some(state) => state,
            // C4Action::Default leaves the compiled Name buffer empty and
            // numeric Act at ActIdle. CompileFunc still attempts the empty
            // SetActionByName lookup, which fails without changing either.
            None if loaded => ActionState::new(String::new()),
            None => default_action_state,
        };
        let loaded_action_resolved = loaded
            && (crate::action::is_builtin_idle_name(&initial_action.name)
                || action_library.contains(&initial_action.name));
        if loaded {
            initial_action.restore_loaded_with_library(
                &action_library,
                construction >= FULL_CON || definition_incomplete_activity,
            );
        } else {
            initial_action.reconcile_with_library(&action_library);
        }
        // Def->CrewMember is an object capability (and drives
        // OCF_CrewMember), not membership in any C4Player::Crew list.
        // Ordinary CreateObject passes no C4ObjectInfo and starts outside
        // every roster; ready crew and explicit fixture restores opt in.
        let initial_crew_member = crew_member.unwrap_or(false);

        let id = match explicit_id {
            Some(explicit) => {
                if self.objects.iter().any(|object| object.id == explicit) {
                    return Err(EngineError::DuplicateObjectId(explicit));
                }
                let raw = explicit.as_u64();
                if raw >= self.next_object_id {
                    self.next_object_id = raw + 1;
                }
                explicit
            }
            None => self.next_object_id(),
        };
        // Native C4Object::CompileFunc records start Category at zero; only
        // DefCore loading repairs a missing sort category (C4Object.cpp:2741;
        // C4Def.cpp:226-232). Programmatic loaded fixtures retain their
        // historical definition fallback unless they opt into that compiler
        // default explicitly.
        let initial_category = category
            .map(|value| {
                if loaded {
                    value
                } else {
                    normalize_category(value, definition_category)
                }
            })
            .unwrap_or(if native_compiled_object_defaults {
                0
            } else {
                definition_category
            });
        let initial_alive = alive.unwrap_or(!loaded && initial_category & CATEGORY_LIVING != 0);
        let definition_physical = self
            .definitions
            .get(&definition_id)
            .map(|definition| *definition.physical())
            .unwrap_or_default();
        let initial_permanent_physical = match initial_info_physical {
            // Loaded objects compile their saved Energy/Breath and never run
            // C4Object::Init. In particular, restore must not eagerly refill
            // the definition fair-crew cache that was just invalidated.
            Some(_) if self.use_fair_crew && !loaded => {
                let retained_definition_id = self
                    .crew_object_infos
                    .get(&id)
                    .map(|info| info.definition_id.clone());
                // C4Object::GetPhysical uses Info->pDef when available and
                // falls back to the object's current Def when that retained
                // pointer cannot be resolved after a definition transition.
                let info_definition_id = retained_definition_id
                    .filter(|id| self.definitions.contains_key(id))
                    .unwrap_or_else(|| definition_id.clone());
                let projection_source =
                    self.definitions.get(&info_definition_id).map(|definition| {
                        (
                            *definition.physical(),
                            definition.rank_base().unwrap_or(1_000),
                            definition.script_arc(),
                        )
                    });
                match projection_source {
                    Some((physical, rank_base, script)) => self.fill_fair_crew_projection(
                        info_definition_id,
                        physical,
                        rank_base,
                        script,
                    ),
                    None => fair_crew_physical_cached(
                        definition_physical,
                        self.fair_crew_strength,
                        1_000,
                        &info_definition_id,
                        &self.fair_crew_physical_cache,
                    ),
                }
            }
            Some(physical) => physical,
            None => definition_physical,
        };

        // Init: an explicit controller wins, else the owner
        // (C4Object.cpp:162). Loaded objects skip Init and keep the
        // compiled value (default NO_OWNER, C4Object.cpp:2739).
        let initial_controller = controller
            .filter(|value| *value > OWNER_NONE)
            .unwrap_or(if loaded { OWNER_NONE } else { owner });
        // C4Object::Init resolves ColorByOwner from the live player color;
        // loaded objects instead compile Color/ColorDw verbatim after Clear
        // (C4Object.cpp:201-204,2733-2787).
        let initial_color = color.unwrap_or_else(|| {
            (!loaded && definition_color_by_owner)
                .then(|| self.players.get(&owner))
                .flatten()
                .and_then(|player| player.to_state().color)
                .map(|color| {
                    u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                })
                .unwrap_or(0)
        });
        let owns_vertices = !vertices.is_empty();
        let shape_template = ObjectShapeTemplate::new(
            definition_vertices.clone(),
            definition_shape_rect,
            definition_fire_top,
            definition_stretch_growth,
            definition_rotateable,
        );
        let shape_template = shape_template.with_line(definition_line);
        // C4Game::NewObject runs DoCon(iCon, fInitial=true) on every
        // freshly CREATED object; the straight-con bottom y-adjust keeps
        // the con-0 bottom — the given y — fixed while the shape grows:
        // the final center is y - (Shape.Hgt + Shape.y) at the spawn
        // construction (C4Object.cpp:1401-1470). Loaded objects keep
        // their saved center (C4GameObjects::Load never re-cons).
        // DoCon's initial bottom adjust moves the INT y only — C++ never
        // touches fix_y here, leaving y and fixtoi(fix_y) split until a
        // movement re-syncs them (rule objects keep the split forever).
        let given_position = position;
        let position = if loaded || position_adjusted {
            position
        } else {
            Vector2::new(
                position.x,
                docon_initial_center_y(
                    shape_template.rect,
                    shape_template.stretch_growth,
                    definition_line,
                    construction,
                    position.y,
                ),
            )
        };
        let docon_split_fixed = (!loaded && !position_adjusted && position.y != given_position.y)
            .then(|| FixedVec2::from_ints(given_position.x, given_position.y));
        // Objects.txt vertices are the CURRENT effective shape serialized
        // by C4Shape::CompileFunc (C4Shape.cpp:495-515) — already Con/
        // rotation-transformed, loaded VERBATIM. Future UpdateShape
        // recomputes from the def like C++ (no vertex ownership).
        // Only a raw-buffer sidecar denotes an actual Objects.txt shape.
        // `loaded=true` is also used by programmatic fixtures to skip Init;
        // those retain the historical definition-shape fallback.
        let (initial_vertices, own_shape_vertices) = if loaded && saved_shape_vertices.is_some() {
            (
                saved_shape_vertices
                    .as_ref()
                    .map(ShapeVertexBuffer::active_vec)
                    .unwrap_or_default(),
                owns_shape_vertices.unwrap_or(false).then(|| {
                    saved_shape_vertices
                        .as_ref()
                        .map(ShapeVertexBuffer::own_original_vertices)
                        .unwrap_or_default()
                }),
            )
        } else if loaded && owns_vertices {
            (vertices.clone(), None)
        } else {
            let shape_base_vertices = if owns_vertices {
                vertices.clone()
            } else {
                definition_vertices
            };
            let initial_vertices = if definition_line != 0 {
                // C4Object::UpdateShape returns immediately for line
                // definitions, so their Init-time Def->Shape survives raw
                // Con=0 and is visible to Construction unchanged.
                shape_base_vertices.clone()
            } else {
                transformed_shape_vertices(
                    &shape_base_vertices,
                    construction,
                    shape_template.stretch_growth,
                    shape_template.rotateable,
                    rotation,
                )
            };
            (
                initial_vertices,
                owns_vertices.then_some(shape_base_vertices),
            )
        };
        let initial_shape_vertices = if loaded {
            saved_shape_vertices
                .unwrap_or_else(|| ShapeVertexBuffer::from_active(&initial_vertices))
        } else if owns_vertices {
            ShapeVertexBuffer::from_active(&initial_vertices)
        } else {
            // C4Object::Init begins with a whole-struct `Shape = Def->Shape`
            // (C4Object.cpp:201-207). UpdateFace(true) then rewrites only the
            // active prefix for Con/rotation, leaving dormant slots intact.
            let mut slots = definition_vertex_slots;
            slots.replace_active(&initial_vertices);
            slots
        };

        let (initial_components, initial_component_order) = match components {
            Some(components) => {
                let definition_order = definition_components
                    .iter()
                    .map(|component| component.id.clone())
                    .collect::<Vec<_>>();
                let order = normalized_component_order(
                    &components,
                    component_order.unwrap_or_default(),
                    &definition_order,
                );
                (components, order)
            }
            None if loaded => (ComponentList::new(), Vec::new()),
            None => {
                let order = definition_components
                    .iter()
                    .map(|component| component.id.clone())
                    .collect();
                // Appended, not merged: a definition may name an ID twice with
                // independent counts and C4IDList keeps both entries
                // (`C4IDList.cpp:33-36`).
                let mut components = ComponentList::new();
                for component in &definition_components {
                    components.push(
                        component.id.clone(),
                        fresh_definition_component_count(component.count, construction),
                    );
                }
                (components, order)
            }
        };

        let mut object = Object::new(
            id,
            definition_id.clone(),
            ObjectState {
                view_energy: 0,
                custom_name,
                script_fixed_position: None,
                script_fixed_velocity: None,
                script_rotation_velocity: rotation_velocity,
                script_fixed_rotation: None,
                position,
                velocity,
                // C4Object::Init stores its nr argument verbatim; script-level
                // SetRotation is the separate path that normalizes to 0..359.
                rotation,
                shape_attach: shape_attach.unwrap_or_default(),
                t_attach: 0,
                no_collect_delay: no_collect_delay.unwrap_or(0),
                base: base.unwrap_or(OWNER_NONE),
                // C4Object::Init assigns Info before resolving Energy and
                // Breath. Ready crew therefore use their persistent physical
                // set, or its fair-crew projection, before Construction.
                energy: energy.unwrap_or(if native_compiled_object_defaults {
                    0
                } else if initial_alive {
                    initial_permanent_physical.energy
                } else {
                    0
                }),
                need_energy: need_energy.unwrap_or(false),
                damage: damage.unwrap_or(0),
                // MagicEnergy compiles verbatim, default 0
                // (C4Object.cpp:2768 / the C4Object ctor, :97).
                magic_energy: magic_energy.unwrap_or(0),
                magic_capacity: 0,
                construction,
                action: initial_action,
                direction,
                command_direction,
                effects: Vec::new(),
                vertices: initial_vertices,
                shape_vertices: initial_shape_vertices,
                // Fresh Init copies Def->Shape. Loaded objects compile the
                // embedded shape from a Clear() default of C4M_Solid.
                contact_density: contact_density.unwrap_or(if loaded {
                    CONTACT_DENSITY_SOLID
                } else {
                    definition_contact_density
                }),
                container: None,
                layer,
                visibility: visibility.unwrap_or(0),
                blit_mode: blit_mode
                    .filter(|mode| *mode != 0)
                    .unwrap_or(definition_blit_mode),
                contents: Vec::new(),
                contents_link_generation: 0,
                // C4Object::Init copies Def->Component and immediately
                // ComponentConCutoff-scales it to Con; NewObject's initial
                // DoCon then gains the same floor-scaled counts
                // (C4Object.cpp:197-199,519-526,1428-1464). Loaded objects
                // bypass Init and compile their saved list verbatim (:2811).
                components: initial_components,
                component_order: initial_component_order,
                status: status.unwrap_or_default(),
                owner,
                controller: initial_controller,
                category: initial_category,
                crew_member: initial_crew_member,
                plr_view_range: plr_view_range.unwrap_or(0),
                selected: selected.unwrap_or(false),
                crew_disabled: crew_disabled.unwrap_or(false),
                // C4Object::Init sets Alive only for C4D_Living categories
                // (C4Object.cpp:191); loaded objects compile it with default
                // false (C4Object.cpp:2756).
                alive: initial_alive,
                base_graphics,
                graphics_overlays,
                draw_transform,
                local_vars: local_vars.into(),
                in_liquid: in_liquid.unwrap_or(false),
                mobile: false,
                solid_mask_override: solid_mask,
                timer: timer.unwrap_or(0),
                own_mass: own_mass.unwrap_or(0),
                on_fire: on_fire.unwrap_or(false),
                fire_phase: fire_phase.unwrap_or(0),
                fire_caused_by: fire_caused_by.unwrap_or(OWNER_NONE),
                info_physical: initial_info_physical,
                temporary_physical,
                physical_changes,
                breath: breath.unwrap_or(if native_compiled_object_defaults {
                    0
                } else {
                    initial_permanent_physical.breath
                }),
                entrance_status: entrance_status.unwrap_or(false),
                menu: None,
                color: initial_color,
                color_modulation: color_modulation.unwrap_or(0),
                picture_rect: picture_rect.unwrap_or_default(),
                shape_override: None,
                ocf: OCF_NORMAL,
            },
            shape_template,
            own_shape_vertices,
        );
        if action_sound_dispatched {
            if let Some(selection) = action_sound_selection {
                object.active_action_sound = selection;
            }
            object.action_sound_initialized = true;
        }
        if native_compiled_object_defaults {
            // C4Object::Clear seeds Mass=0 and CompileFunc overwrites it only
            // when the naming is present.
            object.compiled_mass = Some(compiled_mass.unwrap_or(0));
        } else if let Some(compiled_mass) = compiled_mass {
            object.compiled_mass = Some(compiled_mass);
        }
        object.motion_x = motion_x;
        object.motion_y = motion_y;
        object.compiler_cache = compiler_cache;
        object.last_attach_movement_frame = last_attach_movement_frame.unwrap_or(-1);
        object.last_energy_loss_cause = last_energy_loss_cause.unwrap_or(OWNER_NONE);
        if let Some(snapshot) = command_stack.as_ref() {
            object.commands.restore_from_snapshot(snapshot);
        }
        if let Some(rect) = saved_shape_rect {
            object.shape_rect = Some(rect);
            object.state.shape_override = Some(rect);
        }
        if let Some(fire_top) = shape_fire_top {
            object.shape_fire_top = fire_top;
        }
        object.solid_mask_instance_sequence = solid_mask_instance_sequence;
        if !loaded {
            // C4Object::Init checks the copied object rect against the base
            // graphics before Construction/Initialize and before the first
            // possible mask put (C4Object.cpp:206-211). Keep a distinct
            // override only when an explicit rect or the clamp changed it.
            let explicit_mask = object.state.solid_mask_override.is_some();
            let raw_mask = object.state.solid_mask_override.or_else(|| {
                self.definitions
                    .get(&object.definition_id)
                    .and_then(Definition::solid_mask)
            });
            if let Some(raw_mask) = raw_mask {
                if let Some(checked) = self.checked_solid_mask_rect_for_object(&object, raw_mask) {
                    if explicit_mask || checked != raw_mask {
                        object.state.solid_mask_override = Some(checked);
                    }
                }
            }
        }
        // C4Object::Clear initializes fix_r to zero. Objects.txt compiles
        // Rotation and FixR independently, so an absent FixR must not inherit
        // the serialized integer Rotation until SyncClearance synchronizes it.
        if loaded && fixed_rotation.is_none() {
            object.fixed_rotation = C4Fixed::ZERO;
        }
        // Saved XDir/YDir are C4Fixed, not whole pixels
        // (C4Object.cpp:2765-2766): restore the exact sub-pixel velocity
        // and let the int mirror follow fixtoi.
        if let Some(fixed) = fixed_velocity {
            object.set_fixed_velocity(fixed);
            object.state.velocity = object.velocity_pixels();
        }
        // Saved sub-pixel position/rotation (FixX/FixY/FixR,
        // C4Object.cpp:2762-2764) override the itofix seeds; C++ keeps the
        // integer X/Y/Rotation independent — no back-projection.
        // FixX/FixY compile before the load-time SetActionByName. Every
        // successful lookup (including Idle and an incomplete-object
        // coercion) reaches SetAction's fixed-position resync; only a failed
        // lookup retains the serialized subpixel pair (C4Object.cpp:
        // 2867-2877,4165-4170).
        let saved_fixed_position = fixed_position.filter(|_| !loaded_action_resolved);
        if let Some(fixed) = saved_fixed_position.or(docon_split_fixed) {
            object.fixed_position = fixed;
        }
        if let Some(fixed) = fixed_rotation {
            object.fixed_rotation = fixed;
        }
        if let Some(rdir) = rotation_velocity {
            object.rotation_velocity = rdir;
        }
        // C4GameObjects::Load zeroes StaticBack motion after Objects.txt
        // load (C4GameObjects.cpp:600-604); rdir and fix stay untouched.
        // C++ compiles the object Category VERBATIM (no sort-bit
        // normalization at load), so the bit test uses the raw value.
        if loaded && initial_category & CATEGORY_STATIC_BACK != 0 {
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
        }
        // Initial mobility (C4Object.cpp:183-185): a fresh spawn with any
        // nonzero dir is Mobile unless Category == C4D_StaticBack — the C++
        // check is an EQUALITY test on the whole category, not a bitmask.
        // Loaded objects bypass Init and keep the serialized flag
        // (default false, C4Object.cpp:2772) via the explicit override.
        object.state.mobile = mobile.unwrap_or(
            !loaded
                && initial_category != CATEGORY_STATIC_BACK
                && (object.fixed_velocity.x.is_nonzero()
                    || object.fixed_velocity.y.is_nonzero()
                    || object.rotation_velocity.is_nonzero()),
        );
        object.ensure_material_capacity(self.materials.len());
        // C4Game::NewObject links the original definition/category into the
        // main list before Construction/Initialize run. A callback ChangeDef
        // only marks that existing link Unsorted; it must not use the final
        // definition (or append position) until a later ResortUnsorted.
        let initial_exec_position = self.exec_insert_position(
            Some(id),
            loaded,
            false,
            definition_line != 0,
            initial_category,
            &definition_id,
            None,
        );
        let mut container_changes = Vec::new();
        let mut change_def_reinsert = false;
        if let Some(container_id) = container {
            object.state.container = Some(container_id);
            container_changes.push((None, Some(container_id), false));
        }

        let mut effect_events = Vec::new();
        if loaded {
            // C4Effect::CompileFunc reconstructs the linked list without
            // invoking Fx*Start callbacks. The serialized order is already
            // the live order and must survive byte-for-byte reload semantics.
            object.state.effects = effects;
            // Old-style/bare OnFire saves with no effect list receive the
            // callback-suppressed native Fire node during CompileFunc
            // (C4Object.cpp:2878-2881).
            if object.state.on_fire && object.state.effects.is_empty() {
                let mut fire = EffectState::new(C4FX_FIRE)
                    // With fDoCalls=false the native constructor returns
                    // before assigning its requested priority. The linked
                    // compatibility node remains dead until the first walk.
                    .with_priority(0)
                    .with_interval(C4FX_FIRE_TIMER_INTERVAL);
                fire.number = 1;
                fire.start_dispatched = true;
                object.state.effects.push(fire);
            }
        } else if !effects.is_empty() {
            let commands: Vec<_> = effects.into_iter().map(EffectCommand::add).collect();
            let mut initial_events = object.apply_effect_commands(&commands);
            effect_events.append(&mut initial_events);
        }

        object.clamp_velocity(&self.physics);

        let mut additional_spawns = Vec::new();
        // A callback may synchronously initialize an object that is still a
        // pending SpawnConfig in this creation batch. C++ already has that
        // object in Game.Objects (C4Game.cpp:1121-1138); Rust must retain the
        // nested outcome until the queue materializes its target instead of
        // dropping it as an unknown live object.
        let mut pending_nested_outcomes = Vec::new();
        let mut pending_effect_object_lists: VecDeque<compat::EffectObjectListPreview> =
            VecDeque::new();
        let mut deferred_transfer_zones: Vec<TransferZoneCommand> = Vec::new();
        // C++ Init runs SetOCF before Objects.Add and before Construction
        // (C4Game.cpp:1115-1126; C4Object.cpp:198-217). Compute against the
        // existing world without making the newborn or its not-yet-put mask
        // visible to object queries.
        self.refresh_pending_object_ocf(&mut object, false);
        // Initialize/Construction may legally remove the object
        // (RemoveObject in a placer script, e.g. the grass distributor):
        // the object spawns and immediately ends Deleted like C++.
        let mut destroy_requested = false;
        // The ordinary post-insertion StartCall is only for an object whose
        // creation phases did not already execute SetAction synchronously.
        let mut creation_action_callbacks_dispatched = false;
        // C4Object::DoCon refreshes OCF before PSF_Initialize. Native calls
        // made by Initialize may deliberately leave that cache stale relative
        // to later raw writes, so retain its final host-side value across the
        // deferred materialization refresh (C4Object.cpp:1428-1511).
        let mut initialize_ocf_override = None;

        // Call Construction() before Initialize()
        // Construction() initializes local variables that may be used in Initialize() or action callbacks
        // Loaded objects (Objects.txt / savegame) skip both: C4GameObjects::Load
        // (C4GameObjects.cpp:535-618) never fires construction callbacks.
        if !loaded
            && !initialized
            && self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.has_construction)
                .unwrap_or(false)
        {
            let rng_state = self.rng.clone();
            let (
                CommandBatch {
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
                    solid_mask_operations: construction_solid_mask_operations,
                    host_raster_preview: construction_host_raster_preview,
                    particles,
                    transfer_zones,
                    audio,
                    messages,
                    player_commands,
                    object_order_commands,
                    next_mission_commands,
                    object_lists: phase_object_lists,
                    trigger_game_over,
                    script_go,
                    script_counter,
                },
                audio_state,
                new_rng,
                next_object_id,
                construction_error,
            ) = {
                let world =
                    self.host_world_context_for_pending_object(&object, initial_exec_position);
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .expect("definition must exist");
                definition.call_construction(
                    &object.state,
                    id,
                    rng_state,
                    &self.global_effects,
                    self.physics,
                    self.environment,
                    self.frame,
                    world,
                    self.game_over_triggered,
                    self.audio_registry.clone(),
                )?
            };
            self.stage_host_solid_mask_operations(
                construction_solid_mask_operations,
                construction_host_raster_preview,
            );
            // Fail-safe game call: a script error logs and the object
            // spawns WITH the callback's pre-error effects — C4AulExec
            // aborts the call but rolls nothing back
            // (C4AulExec.cpp:1318-1342), so pre-error creations persist
            // and their burned enumeration numbers stay consumed
            // (C4Game.cpp:1119).
            if let Some(error) = construction_error {
                tolerate_script_error::<()>(Err(error))?;
            }
            self.rng = new_rng;
            self.sync_next_object_id(next_object_id);
            self.audio_registry = audio_state;
            if let Some(go) = script_go {
                self.scenario_script_go = go;
            }
            if let Some(counter) = script_counter {
                self.scenario_script_counter = counter;
            }
            if trigger_game_over {
                self.request_game_over()?;
            }
            if let Some(update) = environment {
                self.apply_environment_delta(&update);
            }
            if let Some(delta) = physics {
                self.apply_physics_delta(delta);
            }
            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }
            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }
            self.pending_object_order_commands
                .extend(object_order_commands);
            pending_effect_object_lists.extend(phase_object_lists);
            self.apply_next_mission_commands(next_mission_commands);
            if destroy {
                destroy_requested = true;
            }
            let change_def = delta.change_def.clone();
            let callback_change_def_reinsert = delta.change_def_reinsert;
            let host_container_change = delta.host_container_change;
            let callback_action_library = if let Some(new_def) = change_def.as_deref() {
                let definition = self
                    .definitions
                    .get(new_def)
                    .ok_or_else(|| EngineError::UnknownDefinition(new_def.to_string()))?;
                let owner_color = self
                    .players
                    .get(&object.state.owner)
                    .and_then(Player::color)
                    .map(|color| {
                        u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                    });
                Self::apply_change_object_def_to_object(
                    &mut object,
                    new_def,
                    definition,
                    self.materials.len(),
                    owner_color,
                );
                definition.action_library().clone()
            } else {
                self.definitions
                    .get(&object.definition_id)
                    .map(|definition| definition.action_library().clone())
                    .unwrap_or_else(|| action_library.clone())
            };
            if change_def.is_some() {
                change_def_reinsert = callback_change_def_reinsert;
            }
            let callbacks_dispatched = delta
                .action
                .as_ref()
                .is_some_and(|action| action.callbacks_dispatched);
            creation_action_callbacks_dispatched |= callbacks_dispatched;
            let outcome = object.apply_delta(&delta, &callback_action_library);
            if change_def.is_some() {
                if let Some(current_definition) = self.definitions.get(&object.definition_id) {
                    object.state.ocf = current_definition.compute_ocf(&object.state);
                }
            }
            // A pending object is seeded into the callback world, so
            // SetAction can run its Start/Abort calls synchronously just as
            // it does after insertion. Only legacy/non-host action writes
            // still need an engine-side deferred transition.
            if let Some(change) = outcome.action_change {
                if !callbacks_dispatched {
                    object.record_action_event(
                        change.previous,
                        ActionTransitionKind::Forced,
                        &callback_action_library,
                    );
                }
            }
            if let Some(change) = outcome.container_change {
                container_changes.push((change.0, change.1, host_container_change));
            }
            let mut applied = object.apply_effect_commands(&effects);
            effect_events.append(&mut applied);
            self.apply_particle_commands(particles);
            // The object joins self.objects only after the callbacks, but
            // C++ adds it to Game.Objects BEFORE Construction/Initialize
            // fire (C4Game.cpp:1115-1131) — its own SetTransferZone must
            // land, so the commands defer to right after the push.
            deferred_transfer_zones.extend(transfer_zones);
            if !global_effects.is_empty() {
                self.apply_global_effect_commands(&global_effects);
            }
            object.clamp_velocity(&self.physics);
            if !command_ops.is_empty() {
                object.apply_command_operations(command_ops);
            }
            if !commands.is_empty() {
                object.enqueue_commands(commands);
            }
            additional_spawns.extend(spawns);
            pending_nested_outcomes
                .extend(self.apply_nested_object_outcomes_retaining_missing(other_objects)?);
            if !audio.is_empty() {
                self.emit_audio_commands(audio);
            }
            if !messages.is_empty() {
                for command in messages {
                    self.messages.apply_command(command);
                }
            }
        }

        if !loaded && !initialized && !destroy_requested {
            // NewObject's initial DoCon calls SetOCF after Objects.Add and
            // Construction, but before UpdateFace puts the completed mask
            // and before Completion/Initialize (C4Object.cpp:1428-1511).
            self.refresh_pending_object_ocf(&mut object, true);
            let has_initialize = self
                .definitions
                .get(&object.definition_id)
                .is_some_and(|definition| definition.has_initialize);
            if has_initialize || !effect_events.is_empty() {
                // Only callbacks that still run before materialization need
                // a private raster preview. With no such observer, the
                // ordinary post-insertion put below is the same C++ state
                // transition without an extra COW landscape copy.
                self.stage_pending_spawn_solid_mask(&mut object);
            }
        }

        let initialize_definition_id = object.definition_id.clone();
        if !loaded
            && !initialized
            && !destroy_requested
            && self
                .definitions
                .get(&initialize_definition_id)
                .map(|definition| definition.has_initialize)
                .unwrap_or(false)
        {
            // The `random` Initialize argument is the command-DSL fixture
            // convention — real content (c4 callback args) burns no synced
            // draw (C4Object.cpp:4154-4182 passes none).
            let c4_convention = self
                .definitions
                .get(&initialize_definition_id)
                .map(|definition| definition.c4_callback_args)
                .unwrap_or(false);
            let random = if c4_convention {
                0
            } else {
                self.next_random_i32()
            };
            let rng_state = self.rng.clone();
            let (
                CommandBatch {
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
                    solid_mask_operations: initialize_solid_mask_operations,
                    host_raster_preview: initialize_host_raster_preview,
                    particles,
                    transfer_zones,
                    audio,
                    messages,
                    player_commands,
                    object_order_commands,
                    next_mission_commands,
                    object_lists: phase_object_lists,
                    trigger_game_over,
                    script_go,
                    script_counter,
                },
                audio_state,
                new_rng,
                next_object_id,
                initialize_error,
            ) = {
                // Keep the world snapshot phase-local: retaining its COW
                // landscape while folding Construction would split dirty
                // generations. Replay only the ordered zone overlay that
                // C++ kept live between the two callbacks.
                let mut world =
                    self.host_world_context_for_pending_object(&object, initial_exec_position);
                for command in &deferred_transfer_zones {
                    world.preview_transfer_zone_command(command);
                }
                let definition = self
                    .definitions
                    .get(&initialize_definition_id)
                    .expect("definition must exist");
                definition.call_initialize(
                    &object.state,
                    id,
                    random,
                    rng_state,
                    &self.global_effects,
                    self.physics,
                    self.environment,
                    self.frame,
                    world,
                    self.game_over_triggered,
                    self.audio_registry.clone(),
                )?
            };
            let initialize_host_ocf_override = delta.ocf_override();
            self.stage_host_solid_mask_operations(
                initialize_solid_mask_operations,
                initialize_host_raster_preview,
            );
            // Fail-safe game call: a script error logs and the object
            // spawns WITH the callback's pre-error effects — C4AulExec
            // aborts the call but rolls nothing back
            // (C4AulExec.cpp:1318-1342), so pre-error creations persist
            // and their burned enumeration numbers stay consumed
            // (C4Game.cpp:1119).
            if let Some(error) = initialize_error {
                tolerate_script_error::<()>(Err(error))?;
            }
            self.rng = new_rng;
            self.sync_next_object_id(next_object_id);
            self.audio_registry = audio_state;
            if let Some(go) = script_go {
                self.scenario_script_go = go;
            }
            if let Some(counter) = script_counter {
                self.scenario_script_counter = counter;
            }
            if trigger_game_over {
                self.request_game_over()?;
            }
            if let Some(update) = environment {
                self.apply_environment_delta(&update);
            }
            if let Some(delta) = physics {
                self.apply_physics_delta(delta);
            }
            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }
            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }
            self.pending_object_order_commands
                .extend(object_order_commands);
            pending_effect_object_lists.extend(phase_object_lists);
            self.apply_next_mission_commands(next_mission_commands);
            if destroy {
                destroy_requested = true;
            }
            let change_def = delta.change_def.clone();
            let callback_change_def_reinsert = delta.change_def_reinsert;
            let host_container_change = delta.host_container_change;
            let callback_action_library = if let Some(new_def) = change_def.as_deref() {
                let definition = self
                    .definitions
                    .get(new_def)
                    .ok_or_else(|| EngineError::UnknownDefinition(new_def.to_string()))?;
                let owner_color = self
                    .players
                    .get(&object.state.owner)
                    .and_then(Player::color)
                    .map(|color| {
                        u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                    });
                Self::apply_change_object_def_to_object(
                    &mut object,
                    new_def,
                    definition,
                    self.materials.len(),
                    owner_color,
                );
                definition.action_library().clone()
            } else {
                self.definitions
                    .get(&object.definition_id)
                    .map(|definition| definition.action_library().clone())
                    .unwrap_or_else(|| action_library.clone())
            };
            if change_def.is_some() {
                change_def_reinsert = callback_change_def_reinsert;
            }
            let callbacks_dispatched = delta
                .action
                .as_ref()
                .is_some_and(|action| action.callbacks_dispatched);
            creation_action_callbacks_dispatched |= callbacks_dispatched;
            let outcome = object.apply_delta(&delta, &callback_action_library);
            if change_def.is_some() {
                if let Some(current_definition) = self.definitions.get(&object.definition_id) {
                    object.state.ocf = current_definition.compute_ocf(&object.state);
                }
            }
            if let Some(ocf) = initialize_host_ocf_override {
                object.state.ocf = ocf;
            }
            initialize_ocf_override = Some(object.state.ocf);
            // See the Construction fold above: callback-world SetAction has
            // already completed its synchronous Start/Abort sequence.
            if let Some(change) = outcome.action_change {
                if !callbacks_dispatched {
                    object.record_action_event(
                        change.previous,
                        ActionTransitionKind::Forced,
                        &callback_action_library,
                    );
                }
            }
            if let Some(change) = outcome.container_change {
                container_changes.push((change.0, change.1, host_container_change));
            }
            let mut applied = object.apply_effect_commands(&effects);
            effect_events.append(&mut applied);
            self.apply_particle_commands(particles);
            // The object joins self.objects only after the callbacks, but
            // C++ adds it to Game.Objects BEFORE Construction/Initialize
            // fire (C4Game.cpp:1115-1131) — its own SetTransferZone must
            // land, so the commands defer to right after the push.
            deferred_transfer_zones.extend(transfer_zones);
            if !global_effects.is_empty() {
                self.apply_global_effect_commands(&global_effects);
            }
            object.clamp_velocity(&self.physics);
            if !command_ops.is_empty() {
                object.apply_command_operations(command_ops);
            }
            if !commands.is_empty() {
                object.enqueue_commands(commands);
            }
            additional_spawns = spawns;
            pending_nested_outcomes
                .extend(self.apply_nested_object_outcomes_retaining_missing(other_objects)?);
            if !audio.is_empty() {
                self.emit_audio_commands(audio);
            }
            if !messages.is_empty() {
                for command in messages {
                    self.messages.apply_command(command);
                }
            }
        }

        if !destroy_requested && !effect_events.is_empty() {
            let mut world =
                self.host_world_context_for_pending_object(&object, initial_exec_position);
            for command in &deferred_transfer_zones {
                world.preview_transfer_zone_command(command);
            }
            let effect_definition_id = object.definition_id.clone();
            let definition = self
                .definitions
                .get(&effect_definition_id)
                .expect("definition must exist");
            let definitions_ref = &self.definitions;
            let global_view = self.global_effects.clone();
            let previous_container = object.state.container;
            let rng_state = self.rng.clone();
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
                effect_object_lists,
                effect_solid_mask_operations,
                effect_host_raster_preview,
                _effect_solid_mask_changed,
                effect_action_callbacks_dispatched,
                effect_change_def_reinsert,
                effect_host_container_change,
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
                id,
                &mut object,
                effect_events,
                global_view,
                &mut self.environment,
                self.physics,
                self.frame,
                world,
                self.audio_registry.clone(),
            )?;
            self.stage_host_solid_mask_operations(
                effect_solid_mask_operations,
                effect_host_raster_preview,
            );
            self.rng = new_rng;
            self.audio_registry = audio_state;
            creation_action_callbacks_dispatched |= effect_action_callbacks_dispatched;
            if let Some(marker) = effect_change_def_reinsert {
                change_def_reinsert = marker;
            }
            self.sync_next_object_id(effect_next_object_id);
            // Creation callbacks run with the new object already linked in
            // C++, but Rust inserts it after this local effect batch. Keep
            // all effect-produced zone commands ordered with the other
            // creation commands and fold them immediately after insertion.
            deferred_transfer_zones.extend(effect_transfer_zones);
            additional_spawns.extend(effect_spawns);
            pending_nested_outcomes
                .extend(self.apply_nested_object_outcomes_retaining_missing(effect_other_objects)?);
            pending_effect_object_lists.extend(effect_object_lists);
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
                self.emit_audio_commands(audio_events);
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
            if previous_container != object.state.container {
                container_changes.push((
                    previous_container,
                    object.state.container,
                    effect_host_container_change,
                ));
            }
            if initialize_ocf_override.is_some() {
                initialize_ocf_override = Some(object.state.ocf);
            }
        }

        // No spawn-time landscape resolution AT ALL: C4Game::NewObject
        // places the object exactly where Init+DoCon computed it
        // (C4Game.cpp:1085-1127) — contacts resolve in movement. Loaded
        // objects keep their Objects.txt position verbatim likewise. The
        // GoldRush wagon (CreateObject(COAC,28,270) -> center 250, 20px
        // above the road) FLOATS at first; snapping it to the surface
        // displaced it and its 30+ contents (the (0,-20) live class).
        let new_id = object.id;
        self.objects.push(object);
        self.note_objects_changed();
        self.insert_exec_link(initial_exec_position.min(self.exec_list.len()), new_id);
        if self
            .find_object_index(new_id)
            .is_some_and(|index| self.objects[index].state.status == ObjectStatus::Inactive)
        {
            self.insert_into_inactive_list(new_id, loaded);
        }
        // Legacy Rust SpawnConfig carries a membership bit directly. Fold
        // that one-time creation intent into the authoritative player list;
        // steady-state refresh must never recreate a link removed by death
        // or SetCrewStatus.
        if initial_crew_member
            && self
                .find_object_index(new_id)
                .is_some_and(|index| self.objects[index].state.status != ObjectStatus::Deleted)
            && self.players.contains_key(&owner)
        {
            let mut roster = self
                .players
                .get(&owner)
                .map(|player| player.crew().to_vec())
                .unwrap_or_default();
            if !roster.contains(&new_id) {
                let position = self.crew_insert_position(&roster, new_id);
                roster.insert(position.min(roster.len()), new_id);
                if let Some(player) = self.players.get_mut(&owner) {
                    player.set_crew(roster);
                }
            }
        }
        // Deferred SetTransferZone commands from the creation callbacks —
        // C++ ran them live with the object already in Game.Objects.
        if !deferred_transfer_zones.is_empty() {
            self.apply_transfer_zone_commands(deferred_transfer_zones)?;
        }
        let index = self.objects.len() - 1;
        self.update_sector_for_index(index);
        // C4GameObjects::Load moves Status=Inactive rows out of the main
        // object list before its UpdateFaces pass, so those loaded objects
        // have no C4SolidMask instance until StatusActivate calls
        // UpdateFace(true). Runtime deactivation is deliberately different:
        // it leaves an already-put mask intact.
        let loaded_inactive = loaded && self.objects[index].state.status == ObjectStatus::Inactive;
        if !loaded_inactive && !destroy_requested {
            self.stage_materialized_spawn_solid_mask(index);
            self.update_solid_mask(index);
        }
        for (previous, new, host_executed) in container_changes {
            if host_executed {
                self.apply_host_container_link_change(id, previous, new)?;
            } else {
                self.apply_container_change(id, previous, new, loaded)?;
            }
        }
        if change_def_reinsert {
            self.reinsert_change_def_contents_link(id)?;
        }
        if loaded {
            // Loaded Contained placement is denumeration, not Enter — the
            // compiled controller stays (C4Object.cpp:2739), no
            // C4Object.cpp:1582 transfer.
            self.objects[index].state.controller = initial_controller;
        }
        if destroy_requested {
            self.objects[index].mark_destroyed();
            self.clear_destroyed_object_layers();
        }
        self.actualize_object_fow_view_range(id);
        self.dispatch_pending_action_sounds(index, false);
        if loaded {
            // Objects.txt restores Action directly; C4GameObjects::Load only
            // refreshes faces and OCF afterwards, so no SetAction Sound= loop
            // is created for the restored slot (C4GameObjects.cpp:575-675;
            // C4Object.cpp:4159-4163). Mark it observed so the presentation
            // reconciliation pass cannot invent that missing transition.
            self.objects[index].action_sound_initialized = true;
        } else {
            self.initialize_action_sound(index, false);
        }
        self.refresh_object_ocf(index);
        if let Some(ocf) = initialize_ocf_override {
            self.objects[index].state.ocf = ocf;
        }
        // Loaded objects restore their action WITHOUT callbacks. Native
        // host creation marked `initialized` already ran every SetAction
        // Start/Abort callback synchronously before this materialization;
        // replaying it here double-fired Construction-selected actions.
        if !loaded && !initialized {
            let previous_action = creation_action_callbacks_dispatched
                .then(|| self.objects[index].state.action.name.clone());
            self.trigger_action_callbacks(index, previous_action)?;
        }
        self.update_sector_for_index(index);
        Ok((
            id,
            additional_spawns,
            pending_nested_outcomes,
            pending_effect_object_lists,
        ))
    }

    /// BubbleOut (C4Effect.cpp:847-857): a bubble only from semi-solid
    /// (submerged) spots, capped by GetSmokeLevel — fixed at 150 in sync
    /// mode, otherwise `Config.Graphics.SmokeLevel`.
    pub(crate) fn bubble_out(&mut self, tx: i32, ty: i32) -> Result<(), EngineError> {
        let semi_solid = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.is_semi_solid_at(tx, ty))
            .unwrap_or(false);
        if !semi_solid {
            return Ok(());
        }
        let bubble_count = self
            .objects
            .iter()
            .filter(|object| {
                object.definition_id.as_str() == "FXU1"
                    && object.state.status.is_active()
                    && !object.destroyed
            })
            .count();
        if bubble_cap_reached(bubble_count, self.bubble_smoke_level()) {
            return Ok(());
        }
        let config = SpawnConfig::new("FXU1")
            .with_position(Vector2::new(tx, ty))
            .with_owner(-1);
        let _ = self.process_spawn_queue(vec![config])?;
        Ok(())
    }

    /// Splash (C4Effect.cpp:800-835): bubbles + liquid PXS on entering
    /// water fast. The Random draws are synced — order matters.
    pub(crate) fn splash(&mut self, tx: i32, ty: i32, amt: i32) -> Result<(), EngineError> {
        crate::engine_splash::run_splash(self, tx, ty, amt)?;
        // Splash sounds are presentation-only.
        Ok(())
    }

    pub(crate) fn process_spawn_queue(
        &mut self,
        queue: Vec<SpawnConfig>,
    ) -> Result<Vec<ObjectId>, EngineError> {
        self.process_spawn_queue_with_outcomes(queue, Vec::new())
    }

    #[doc(hidden)]
    pub fn process_spawn_queue_with_outcomes(
        &mut self,
        queue: Vec<SpawnConfig>,
        nested_outcomes: Vec<compat::NestedObjectOutcome>,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let was_deferred = self.solid_mask_staging.defer_solid_mask_updates;
        let result =
            self.process_spawn_queue_with_outcomes_inner(queue, nested_outcomes, VecDeque::new());
        let outermost = !was_deferred && self.solid_mask_staging.defer_solid_mask_updates;
        self.finish_host_solid_mask_operations(outermost, result)
    }

    fn effect_object_lists_are_materialized(
        &self,
        preview: &compat::EffectObjectListPreview,
    ) -> bool {
        preview
            .master_order
            .iter()
            .chain(&preview.inactive_order)
            .all(|id| self.find_object_index(*id).is_some())
    }

    fn install_materialized_effect_object_lists(
        &mut self,
        pending: &mut VecDeque<compat::EffectObjectListPreview>,
    ) {
        while pending
            .front()
            .is_some_and(|preview| self.effect_object_lists_are_materialized(preview))
        {
            let preview = pending
                .pop_front()
                .expect("the materialized effect-list preview must still be queued");
            self.install_effect_object_lists(preview);
        }
    }

    /// Apply every held same-call `Enter` whose container has materialized,
    /// repeating until no further link resolves so a chain of freshly created
    /// containers binds in one pass.
    pub(crate) fn apply_materialized_deferred_enters(
        &mut self,
        deferred_enters: &mut Vec<(ObjectId, ObjectId)>,
    ) -> Result<(), EngineError> {
        while let Some(position) = deferred_enters
            .iter()
            .position(|(_, container)| self.find_object_index(*container).is_some())
        {
            let (object_id, container) = deferred_enters.remove(position);
            if self.find_object_index(object_id).is_none() {
                continue;
            }
            self.apply_container_change(object_id, None, Some(container), false)?;
        }
        Ok(())
    }

    pub(crate) fn process_spawn_queue_with_outcomes_inner(
        &mut self,
        queue: Vec<SpawnConfig>,
        nested_outcomes: Vec<compat::NestedObjectOutcome>,
        mut pending_object_lists: VecDeque<compat::EffectObjectListPreview>,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let mut pending: VecDeque<_> = queue.into_iter().collect();
        let mut created = Vec::new();
        // C4Object::Enter binds two objects that already exist, because
        // FnCreateObject hands back a live C4Object (C4Object.cpp:1560-1620;
        // C4Script.cpp FnCreateObject). One call may therefore create its
        // content before the container it enters, as Hazard's
        // Arena_RelaunchClonk does. Materialize in creation order and hold
        // such a link until the queued container exists.
        let mut deferred_enters: Vec<(ObjectId, ObjectId)> = Vec::new();
        // Live targets must commit before the first pending object's
        // Initialize. Only targets represented by an unmaterialized
        // SpawnConfig remain deferred.
        let mut nested_outcomes =
            self.apply_nested_object_outcomes_retaining_missing(nested_outcomes)?;
        self.install_materialized_effect_object_lists(&mut pending_object_lists);
        while let Some(mut config) = pending.pop_front() {
            if let (Some(object_id), Some(container)) = (config.id, config.container) {
                if self.find_object_index(container).is_none()
                    && pending.iter().any(|queued| queued.id == Some(container))
                {
                    config.container = None;
                    deferred_enters.push((object_id, container));
                }
            }
            // C++ CreateObject with an unknown id is C4Id2Def -> nullptr
            // (C4Script.cpp FnCreateObject): the call yields nil, never an
            // error, so unknown spawns are skipped rather than fatal.
            let (id, additional, additional_outcomes, object_lists) =
                match self.spawn_single(config) {
                    Ok(result) => result,
                    Err(EngineError::UnknownDefinition(definition)) => {
                        tracing::warn!(
                            definition,
                            "skipping spawn of unknown definition like C++ CreateObject"
                        );
                        continue;
                    }
                    Err(other) => return Err(other),
                };
            created.push(id);
            nested_outcomes.extend(additional_outcomes);
            // The just-created object is now a live target. Flush its
            // retained outcomes before the next queued object's callbacks,
            // preserving C++ NewObject's synchronous visibility.
            nested_outcomes =
                self.apply_nested_object_outcomes_retaining_missing(nested_outcomes)?;
            pending_object_lists.extend(object_lists);
            // NewObject does not return to its caller until every nested
            // CreateObject has completed (C4Game.cpp:1085-1142). Process the
            // current object's callback-produced creations before advancing
            // to an unrelated pre-populated queue member.
            for spawn in additional.into_iter().rev() {
                pending.push_front(spawn);
            }
            self.apply_materialized_deferred_enters(&mut deferred_enters)?;
            self.install_materialized_effect_object_lists(&mut pending_object_lists);
        }
        assert!(
            pending_object_lists.is_empty(),
            "effect-list previews may only reference objects from their creation batch"
        );
        // Any remainder belongs to a same-call object removed before its
        // SpawnConfig materialized; preserve the prior silent-miss behavior.
        // A held Enter whose container never materialized shares that fate:
        // C4Object::Enter on a removed container does not happen either.
        Ok(created)
    }
}
