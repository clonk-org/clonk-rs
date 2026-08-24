//! `impl Engine` — landscape debug probes and the execution-order lists.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub fn debug_landscape_byte(&self, x: i32, y: i32) -> Option<u8> {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.pixel_grid())
            .and_then(|grid| grid.byte_at(x, y))
    }

    /// Debug helper: the raw pixel plane (width, height, bytes).
    pub fn debug_landscape_plane(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.pixel_grid())
            .map(|grid| (grid.width(), grid.height(), grid.bytes().to_vec()))
    }

    pub fn debug_landscape_is_liquid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .map(|landscape| landscape.is_liquid_at(x, y))
            .unwrap_or(false)
    }

    pub fn debug_landscape_is_solid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .map(|landscape| landscape.is_solid_at(x, y))
            .unwrap_or(false)
    }

    /// Debug helper: a definition's physical + an action's procedure.
    pub fn debug_definition_physical(
        &self,
        id: &str,
        action: &str,
    ) -> Option<(i32, Option<String>)> {
        self.definitions.get(&DefinitionId::from(id)).map(|def| {
            (
                def.physical().float,
                def.action_library()
                    .procedure_name_for_action(action)
                    .map(|s| s.to_string()),
            )
        })
    }

    /// Debug helper: is `name` a global script function?
    pub fn debug_global_has_function(&self, name: &str) -> bool {
        self.global_script_functions
            .as_ref()
            .map(|functions| functions.contains_key(name))
            .unwrap_or(false)
    }

    /// Debug/test helper: ids of objects whose solid mask is active for
    /// movement.
    pub fn debug_solid_mask_ids(&self) -> Vec<u64> {
        let indices: Vec<usize> = (0..self.objects.len()).collect();
        self.solid_masks_for_movement(&indices)
            .iter()
            .map(|mask| mask.object_id.as_u64())
            .collect()
    }

    /// Debug/test helper: the per-object SolidMask override of `id`
    /// (outer None = object missing; inner None = definition default).
    pub fn debug_solid_mask_override(&self, id: u64) -> Option<Option<(i32, i32, i32, i32)>> {
        self.objects
            .iter()
            .find(|object| object.id.as_u64() == id)
            .map(|object| {
                object
                    .state
                    .solid_mask_override
                    .map(|rect| (rect.x, rect.y, rect.width, rect.height))
            })
    }

    /// Debug/test helper: saved background bytes under an active solid mask.
    pub fn debug_solid_mask_buffer(&self, id: u64) -> Option<Vec<u8>> {
        self.objects
            .iter()
            .find(|object| object.id.as_u64() == id)
            .and_then(|object| object.solid_mask_bake.as_ref())
            .map(|bake| bake.buffer.clone())
    }

    /// Debug/test helper: C4SolidMask::MaskPut state, including a fully
    /// clipped put that has no raster buffer.
    pub fn debug_solid_mask_is_put(&self, id: u64) -> Option<bool> {
        self.objects
            .iter()
            .find(|object| object.id.as_u64() == id)
            .map(|object| object.solid_mask_bake.is_some() || object.solid_mask_empty_put)
    }

    /// The force-close/RejectContents lifecycle shared by the internal
    /// Activate/Get/Contents menus (C4Object.cpp:1884-1959).
    pub(crate) fn apply_container_menu_request(
        &mut self,
        request: MenuRequest,
    ) -> Result<(), EngineError> {
        let reused_menu_identity = self
            .find_object_index(request.crew_id)
            .and_then(|index| self.objects[index].state.menu.as_ref())
            .map(|menu| menu.internal_refill_token)
            .filter(|identity| *identity != 0);
        let _ = self.close_object_menu(request.crew_id, true)?;
        let (container, identification) = match &request.kind {
            MenuRequestKind::Activate => (
                self.find_object_index(request.crew_id)
                    .and_then(|index| self.objects[index].state.container),
                6,
            ),
            MenuRequestKind::ActivateTarget { container } => (Some(*container), 6),
            MenuRequestKind::Get { container } => (Some(*container), 13),
            MenuRequestKind::Contents { container } => (Some(*container), 18),
            _ => return Ok(()),
        };
        let Some(container) = container else {
            return Ok(());
        };
        let Some(container_index) = self.find_object_index(container) else {
            return Ok(());
        };

        let rejected = tolerate_script_error(self.call_object_function(
            container_index,
            "RejectContents",
            Vec::new(),
        ))?
        .is_some_and(|value| compat::value_raw_truthy(&value));
        if rejected
            || self.find_object_index(request.crew_id).is_none()
            || self.find_object_index(container).is_none()
        {
            return Ok(());
        }
        let Some(crew_index) = self.find_object_index(request.crew_id) else {
            return Ok(());
        };
        let Some(container_index) = self.find_object_index(container) else {
            return Ok(());
        };
        match identification {
            6 => {
                self.set_activate_menu(crew_index, container_index, false, reused_menu_identity)?
            }
            13 | 18 => {
                self.set_container_contents_menu(
                    crew_index,
                    container_index,
                    identification,
                    false,
                    reused_menu_identity,
                )?;
            }
            _ => unreachable!("known internal object-menu id"),
        }
        Ok(())
    }

    /// C4Object::CloseMenu (C4Object.cpp:2033-2041) for the engine-side
    /// control paths that run outside a script scope (the host-fn twin is
    /// compat::close_object_menu). Force skips the MenuQueryCancel query
    /// (C4Menu::TryClose, C4Menu.cpp:317-320); a soft close of a USER menu
    /// asks MenuQueryCancel(Selection, ParentObject) on the command object
    /// first (C4ObjectMenu::IsCloseDenied, C4ObjectMenu.cpp:57-76) — a
    /// truthy answer keeps the menu and fails the close. A menu initialized
    /// with CB_Scenario uses the scenario script even though a cleared
    /// CB_Object target is also represented by a missing command pointer.
    pub(crate) fn close_object_menu(
        &mut self,
        object_id: ObjectId,
        force: bool,
    ) -> Result<bool, EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(true);
        };
        let Some(menu) = self.objects[index].state.menu.clone() else {
            return Ok(true); // no menu -> close OK (C4Object.cpp:2035)
        };
        if !force && menu.user_menu && compat::begin_menu_close_query(object_id) {
            // Missing handler = silent miss (the "~" in PSF_MenuQueryCancel,
            // C4GameScript.h); callee errors fall back to close-OK like the
            // C++ fail-safe Call.
            let pars = vec![
                Value::Int(menu.selection),
                object_reference_value(object_id),
            ];
            let denied = if menu.scenario_callbacks {
                self.call_scenario_script_value("MenuQueryCancel", &pars)
                    .map(|value| value.is_some_and(|value| value.as_bool()))
            } else if let Some(command_object) = menu.command_object {
                let command_index =
                    self.find_object_index(command_object)
                        .filter(|&command_index| {
                            self.definitions
                                .get(&self.objects[command_index].definition_id)
                                .is_some_and(|definition| {
                                    definition.has_function("MenuQueryCancel")
                                })
                        });
                match command_index {
                    Some(command_index) => tolerate_script_error(self.call_object_function(
                        command_index,
                        "MenuQueryCancel",
                        pars,
                    ))
                    .map(|value| value.is_some_and(|value| value.as_bool())),
                    None => Ok(false),
                }
            } else {
                Ok(false)
            };
            compat::end_menu_close_query(object_id);
            let denied = denied?;
            if denied {
                return Ok(false);
            }
        }
        // delete Menu (C4Object.cpp:2038) — dropping the state is the
        // close in this model.
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].state.menu = None;
        }
        Ok(true)
    }

    /// The local player's cursor-owned script menu. Kept out of snapshots
    /// and saves like C++'s runtime-only C4ObjectMenu; frontends may read
    /// this live state for presentation while controls stay in InCom.
    pub fn cursor_object_menu(&self, owner: i32) -> Option<(ObjectId, &ObjectMenuState)> {
        let cursor = self.crew_cursor(owner)?;
        self.objects
            .iter()
            .find(|object| object.id == cursor)?
            .state
            .menu
            .as_ref()
            .map(|menu| (cursor, menu))
    }

    /// Whether any runtime C4ObjectMenu is registered as a shown C4GUI
    /// dialog. CreateMenu may target an arbitrary object, not only a player's
    /// current cursor, so presentation ownership must use the global scan.
    pub fn has_active_object_menu(&self) -> bool {
        self.objects
            .iter()
            .any(|object| object.state.menu.is_some())
    }

    /// Resolve `C4MenuItem::IsDragElement` for an item in the owner's live
    /// cursor menu (C4Menu.cpp:128-133).
    ///
    /// Callers should resolve this once on button-down and again after the
    /// drag threshold so menu/cursor/item changes cannot start a stale drag.
    pub fn object_menu_construction_drag(
        &self,
        owner: i32,
        index: usize,
    ) -> Option<ObjectMenuConstructionDrag> {
        let (menu_object_id, menu) = self.cursor_object_menu(owner)?;
        let item = menu.items.get(index)?;
        let definition = self.definitions.get(&item.item_id)?;
        if !definition.is_constructable() {
            return None;
        }
        let definition_c4id = command::definition_id_to_c4id(&item.item_id)?;
        Some(ObjectMenuConstructionDrag {
            menu_object_id,
            definition_id: item.item_id.clone(),
            definition_c4id,
        })
    }

    /// Read-only `ConstructionCheck` for local drag feedback. Synchronized
    /// Construct execution invokes the same core predicate again. The C++
    /// preview passes no `pByObj`, so the failure branch is discarded
    /// without feedback (C4MouseControl.cpp:1098).
    pub fn construction_site_valid(&self, definition_id: &str, site: Vector2) -> bool {
        let Some(definition) = self.definitions.get(definition_id) else {
            return false;
        };
        command::construction_check(
            definition.is_constructable(),
            definition.shape_rect(),
            definition.construction_offset(),
            definition.category(),
            site,
            self.landscape.as_ref(),
            |left, top, width, height, category| {
                Self::placement_overlapping_object(
                    &self.objects,
                    left,
                    top,
                    width,
                    height,
                    category,
                )
            },
        )
        .is_none()
    }

    /// `C4MouseControl::UpdateFogOfWar` plus `C4Player::FoWIsVisible` for a
    /// construction-drag world point (C4MouseControl.cpp:1266-1277;
    /// C4Player.cpp:1959-1993).
    pub fn construction_site_visible(&self, owner: i32, site: Vector2) -> bool {
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        if site.x < 0
            || site.y < 0
            || site.x >= landscape.width() as i32
            || site.y >= landscape.estimated_height()
        {
            return false;
        }
        let Some(player) = self.players.get(&owner) else {
            return false;
        };
        if !player.fog_of_war() {
            return true;
        }

        let closed_to_fog = |object: &Object| {
            object
                .state
                .container
                .and_then(|container| self.find_object_index(container))
                .and_then(|index| self.definitions.get(&self.objects[index].definition_id))
                .is_some_and(|definition| definition.closed_container() == 1)
        };
        let check_object = |object: &Object, range: i32, seen: &mut bool| {
            if closed_to_fog(object)
                || command::c4_distance(
                    object.state.position.x,
                    object.state.position.y,
                    site.x,
                    site.y,
                ) as u32
                    >= range.unsigned_abs()
            {
                return true;
            }
            if range < 0 {
                // A nonzero ColorMod alpha is a faded generator: it paints
                // darkness but does not block FoWIsVisible.
                object.state.color_modulation & 0xff00_0000 != 0
            } else {
                *seen = true;
                true
            }
        };

        let mut seen = false;
        let mut last_view_object = None;
        for &object_id in player.fow_view_objects() {
            last_view_object = Some(object_id);
            let Some(index) = self.find_object_index(object_id) else {
                continue;
            };
            let object = &self.objects[index];
            if !check_object(object, object.state.plr_view_range, &mut seen) {
                return false;
            }
        }

        if player.raw_view_mode() == PLAYER_VIEW_MODE_TARGET {
            if let Some(target_id) = player
                .raw_view_target()
                .filter(|target| Some(*target) != last_view_object)
            {
                if let Some(target_index) = self.find_object_index(target_id) {
                    let target = &self.objects[target_index];
                    let mut range = target.state.plr_view_range;
                    if range == 0 {
                        range = player
                            .cursor()
                            .and_then(|cursor| self.find_object_index(cursor))
                            .map_or(0, |index| self.objects[index].state.plr_view_range);
                    }
                    if range == 0 {
                        range = 500;
                    }
                    if !check_object(target, range, &mut seen) {
                        return false;
                    }
                }
            }
        }
        seen
    }

    /// Live callback controller used by C4ObjectMenu::GetControllingPlayer.
    pub fn object_controller(&self, object: ObjectId) -> Option<i32> {
        self.find_object_index(object)
            .map(|index| self.objects[index].state.controller)
    }

    /// Debug/test helper: the object's script menu state (outer None =
    /// object missing; inner None = no menu open).
    pub fn debug_object_menu(&self, id: u64) -> Option<Option<ObjectMenuState>> {
        self.objects
            .iter()
            .find(|object| object.id.as_u64() == id)
            .map(|object| object.state.menu.clone())
    }

    /// Debug/test helper: a clone of the synced RNG for ledger-position
    /// assertions.
    pub fn debug_rng_clone(&self) -> crate::rng::LcgRng {
        self.rng.clone()
    }

    /// Debug helper: (id, definition) rows at or above `min_id`, sorted —
    /// the creation-order forensics feed for the numbering-skew epic.
    pub fn spawn_dump_from(&self, min_id: u64) -> Vec<(u64, String)> {
        let mut rows: Vec<(u64, String)> = self
            .objects
            .iter()
            .filter(|object| object.id.as_u64() >= min_id)
            .map(|object| (object.id.as_u64(), object.definition_id.clone()))
            .collect();
        rows.sort();
        rows
    }

    #[doc(hidden)]
    pub fn debug_exec_order(&self) -> Vec<ObjectId> {
        self.exec_list.clone()
    }

    /// C4ObjectList::Add stMain insertion (C4ObjectList.cpp:110-216) kept
    /// in EXEC order (the C++ list reversed — see the `exec_list` docs).
    /// - Line defs skip sorting (`fUnsorted`, :148): forward-list end =
    ///   exec HEAD (a new line executes first, like today's beams).
    /// - Pass 1 (:150-162, skipped for C4D_StaticBack "to allow
    ///   multiobject outside structure"): insert before the forward-FIRST
    ///   live link with the same sorted category AND the same def = exec
    ///   right AFTER the def cluster's last-executing member.
    /// - Pass 2 (:164-173): insert before the forward-first live link
    ///   with sorted category <= own = exec at the END of the category
    ///   bracket; with no such link the object lands at the forward end =
    ///   exec head.
    /// Loaded objects bypass Add: Objects.txt is saved back-to-front and
    /// re-added with stReverse (C4ObjectList.cpp:508-530), so a loaded
    /// object's exec position is its file position — plain append.
    /// C4GameObjects::FixObjectOrder (C4GameObjects.cpp:773-830, called
    /// after Objects.txt load :663): normalizes each object's sort
    /// category to exactly one bit (lowest set; missing -> StaticBack,
    /// persisted into Category) and re-establishes the master-list
    /// category order. The master list is category-DESCENDING front to
    /// back and ExecObjects walks it in reverse (C4Game.cpp:1582), so the
    /// exec view sorts category-ASCENDING; the bubble pass preserves
    /// relative order within a bracket (Objects.txt file order). Loaded
    /// entries append in file order and are bracket-sorted here. This is a
    /// load/restore repair only: C++ never normalizes runtime categories or
    /// silently relocates ChangeDef's Unsorted link on a later frame.
    pub(crate) fn fix_exec_list_order(&mut self) {
        if std::env::var("LC_EXECDBG").is_ok() {
            crate::rng::rng_trace_line(&format!("FIXORDER len={}", self.exec_list.len()));
        }
        let mut keyed: Vec<(i32, usize, ObjectId)> = Vec::with_capacity(self.exec_list.len());
        for (position, &id) in self.exec_list.iter().enumerate() {
            let Some(index) = self.find_object_index(id) else {
                continue;
            };
            let object = &self.objects[index];
            // Unsorted/dead links are holes in FixObjectOrder; inactive
            // objects belong to C4GameObjects::InactiveObjects and likewise
            // cannot participate in the main-list repair.
            if object.destroyed || !object.state.status.is_active() || object.unsorted {
                continue;
            }
            let raw = self.objects[index].state.category;
            let masked = raw & CATEGORY_SORT_LIMIT;
            let sort_bit = if masked == 0 {
                self.objects[index].state.category = raw + 1;
                1
            } else {
                let lowest = masked & masked.wrapping_neg();
                if lowest != masked {
                    self.objects[index].state.category = (raw & !CATEGORY_SORT_LIMIT) | lowest;
                }
                lowest
            };
            keyed.push((sort_bit, position, id));
        }
        let positions = keyed
            .iter()
            .map(|&(_, position, _)| position)
            .collect::<Vec<_>>();
        keyed.sort_by_key(|&(sort_bit, position, _)| (sort_bit, position));
        let sorted_ids = keyed.into_iter().map(|(_, _, id)| id).collect::<Vec<_>>();
        for (position, id) in positions.into_iter().zip(sorted_ids) {
            self.exec_list[position] = id;
        }
        let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        if let Some(sectors) = self.sectors.as_mut() {
            sectors.set_master_order(master_order);
        }
    }

    /// C4GameObjects::ExecuteResorts (C4GameObjects.cpp:874-886). Requests
    /// are pushed at the head of ResortProc, hence newest-first. The Rust
    /// list is the C++ main list reversed, so Before/After are reversed too.
    #[doc(hidden)]
    pub fn execute_object_order_commands(&mut self) {
        if self.pending_object_order_commands.is_empty() && !self.resort_any_object {
            return;
        }
        let commands = std::mem::take(&mut self.pending_object_order_commands);

        let sort_all = commands
            .iter()
            .any(|command| matches!(command, ObjectOrderCommand::SortByCategory));
        let resort_objects: HashSet<ObjectId> = commands
            .iter()
            .filter_map(|command| match command {
                ObjectOrderCommand::ResortObject(object) => Some(*object),
                _ => None,
            })
            .collect();
        self.resort_any_object |= !resort_objects.is_empty()
            || commands
                .iter()
                .any(|command| matches!(command, ObjectOrderCommand::ResortUnsortedSweep));

        // C4Object::Resort sets both the object's Unsorted bit and the one
        // global `fResortAnyObject` trigger. Once that trigger fires,
        // ResortUnsorted scans *every* previously flagged object — notably
        // ChangeDef objects, which set Unsorted without setting the trigger.
        for object in &resort_objects {
            if let Some(index) = self.find_object_index(*object) {
                self.objects[index].unsorted = true;
            }
        }

        if sort_all {
            self.sort_exec_list_by_category();
        }

        if self.resort_any_object {
            self.resort_all_unsorted();
            self.resort_any_object = false;
        }

        // C4ObjResort nodes are pushed at the list head: newest first.
        for command in commands.into_iter().rev() {
            match command {
                ObjectOrderCommand::SetRelative {
                    relative_to,
                    object,
                    after,
                } => {
                    if self.execute_relative_object_order_command(relative_to, object, after) {
                        self.update_pos_resort(object);
                    }
                }
                ObjectOrderCommand::OrderFuncAll { order, category } => {
                    self.execute_order_function_all(&order, category);
                }
                ObjectOrderCommand::OrderFuncObject { order, object } => {
                    if self.execute_order_function_object(&order, object) {
                        self.update_pos_resort(object);
                    }
                }
                ObjectOrderCommand::ResortObject(_)
                | ObjectOrderCommand::ResortUnsortedSweep
                | ObjectOrderCommand::SortByCategory => {}
            }
        }

        // ExecuteResorts saves each node's old Next pointer before invoking
        // it. A comparator that prepends another C4ObjResort therefore puts
        // it outside that traversal, and the final ResortProc=null discards
        // it. Native Resort flags are a separate game-level trigger and stay
        // armed for the next post-CrossCheck sweep.
        self.pending_object_order_commands.retain(|command| {
            matches!(
                command,
                ObjectOrderCommand::ResortObject(_)
                    | ObjectOrderCommand::ResortUnsortedSweep
                    | ObjectOrderCommand::SortByCategory
            )
        });
    }

    /// Invoke one C4ObjResort OrderFunc through the script host captured at
    /// queue time. C4AulParSet supplies two object values with no `this`
    /// object; ordinary script errors, nil, and non-integer results compare
    /// as zero. Host side effects and the synchronized RNG position commit
    /// before the caller performs the next comparison.
    fn call_object_order_function(
        &mut self,
        order: &ObjectOrderFunction,
        first: ObjectId,
        second: ObjectId,
    ) -> i32 {
        let args = [
            object_reference_value(first),
            object_reference_value(second),
        ];
        // C4LSectors retains its own physical list order until an object is
        // re-added. Native SortByCategory refreshes only its rank oracle, so
        // this comparator needs the exact callback-entry sector snapshot.
        // Keep ordinary host contexts lazy to avoid a full-map clone on every
        // unrelated script call.
        let world = self
            .host_world_context()
            .with_sector_map(self.sectors.clone());
        let Some((_, _, script)) = world.script_for_host_identity(order.host_identity) else {
            tracing::warn!(
                script = %order.script_name,
                function = %order.function,
                "queued object-order function lost its retained script host"
            );
            return 0;
        };
        let global_effects = self.global_effects.clone();
        let (value, _finals, mut batch, audio, rng, script_error) =
            ScenarioScript::execute_value_for_script(
                &order.script_name,
                order.definition_context.clone(),
                &order.function,
                &args,
                world,
                self.rng.clone(),
                self.frame,
                &global_effects,
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
                || {
                    script.call_resolved_with_ref_args(
                        &order.resolution,
                        order.engine_global,
                        &args,
                    )
                },
            );
        self.rng = rng;
        self.audio_registry = audio;

        // C4Object::Resort writes Unsorted immediately and separately arms
        // Game.fResortAnyObject. Host outcomes otherwise retain only the
        // deferred command, so materialize that write around the copied
        // batch and convert the surviving command to an arm-only trigger.
        // The second pass covers CreateObject()+Resort(new_object) from one
        // comparator call: the explicit id exists only after batch apply.
        let mut reentrant_resorts = Vec::new();
        let mut synchronous_category_sort = false;
        batch.object_order_commands.retain_mut(|command| {
            if matches!(command, ObjectOrderCommand::SortByCategory) {
                synchronous_category_sort = true;
                return false;
            }
            if let ObjectOrderCommand::ResortObject(object) = command {
                reentrant_resorts.push(*object);
                *command = ObjectOrderCommand::ResortUnsortedSweep;
            }
            true
        });
        for &object in &reentrant_resorts {
            if let Some(index) = self.find_object_index(object) {
                self.objects[index].unsorted = true;
            }
        }

        let mut failed = false;
        if let Err(error) = self.apply_scenario_batch(batch) {
            failed = true;
            tracing::warn!(%error, "object-order script batch failed to apply");
        }
        if synchronous_category_sort {
            // Global Resort() has no ResortProc node in C++; it sorts the
            // main list before the script call returns. Host outcomes keep
            // mutation channels separately, so exact cross-channel call
            // chronology is unavailable here. Applying the whole batch and
            // sorting at its boundary nevertheless preserves the visible
            // post-call state for the next comparator and never defers the
            // native sort into a later sweep.
            self.sort_exec_list_by_category();
        }
        for object in reentrant_resorts {
            if let Some(index) = self.find_object_index(object) {
                self.objects[index].unsorted = true;
            }
        }
        if script_error.is_some() {
            failed = true;
        }

        if failed {
            0
        } else {
            value.as_ref().and_then(Value::as_c4_int).unwrap_or(0)
        }
    }

    /// Object stored in a physical C++ master-list link. `exec_list` is the
    /// reverse representation, so master index zero maps to its final slot.
    fn object_order_master_id(&self, master_index: usize) -> Option<ObjectId> {
        let reverse_offset = master_index.checked_add(1)?;
        let exec_index = self.exec_list.len().checked_sub(reverse_offset)?;
        self.exec_list.get(exec_index).copied()
    }

    fn object_order_master_active(&self, master_index: usize) -> bool {
        self.object_order_master_id(master_index)
            .and_then(|id| self.find_object_index(id))
            .is_some_and(|index| {
                let object = &self.objects[index];
                !object.destroyed && object.state.status == ObjectStatus::Normal
            })
    }

    fn object_order_master_status(&self, master_index: usize) -> Option<ObjectStatus> {
        self.object_order_master_id(master_index)
            .and_then(|id| self.find_object_index(id))
            .map(|index| self.objects[index].state.status)
    }

    fn object_order_master_category(&self, master_index: usize) -> i32 {
        self.object_order_master_id(master_index)
            .and_then(|id| self.find_object_index(id))
            .map(|index| self.objects[index].state.category)
            .unwrap_or(0)
    }

    fn swap_object_order_master_links(&mut self, first: usize, second: usize) -> bool {
        let Some(first_offset) = first.checked_add(1) else {
            return false;
        };
        let Some(second_offset) = second.checked_add(1) else {
            return false;
        };
        let Some(first_exec) = self.exec_list.len().checked_sub(first_offset) else {
            return false;
        };
        let Some(second_exec) = self.exec_list.len().checked_sub(second_offset) else {
            return false;
        };
        if first_exec >= self.exec_list.len() || second_exec >= self.exec_list.len() {
            return false;
        }
        self.exec_list.swap(first_exec, second_exec);
        // Swapping payloads changes Game.Objects immediately. Existing
        // physical sector vectors stay untouched until UpdatePosResort, but
        // any object added or re-added by a later comparator must rank
        // against this new live master order.
        let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        if let Some(sectors) = self.sectors.as_mut() {
            sectors.set_master_order(master_order);
        }
        true
    }

    /// C4ObjResort::Execute's raw-category walk. It starts at master Last,
    /// processes requested sort bits from 1 through 16, and carries the
    /// First-side boundary into the next category. The extent loop
    /// intentionally tests the moving link's category without its Status —
    /// matching the legacy `pLnk`/`pNextLnk` typo.
    fn execute_order_function_all(&mut self, order: &ObjectOrderFunction, category: i32) {
        let mut cursor = self.exec_list.len().checked_sub(1);
        let mut bit = 1;
        while bit < CATEGORY_SORT_LIMIT {
            if category & bit == 0 {
                bit <<= 1;
                continue;
            }

            loop {
                let Some(position) = cursor else {
                    // The C++ loop leaves a null cursor here (and would
                    // dereference it if another requested category follows).
                    // A fail-safe return preserves every completed sort.
                    return;
                };
                if self.object_order_master_active(position)
                    && self.object_order_master_category(position) & bit != 0
                {
                    break;
                }
                cursor = position.checked_sub(1);
            }

            let last = cursor.expect("matching master-list link established");
            let fixed_last_is_inactive = !self.object_order_master_active(last);
            let mut next = Some(last);
            while let Some(position) = next {
                // C4OS_INACTIVE lives in C4GameObjects::InactiveObjects and
                // has no physical link in Game.Objects. Rust retains a slot
                // in exec_list for save/restore bookkeeping, so it must be
                // transparent here regardless of its stale raw category.
                if self.object_order_master_status(position) == Some(ObjectStatus::Inactive) {
                    next = position.checked_sub(1);
                    continue;
                }
                if !fixed_last_is_inactive && self.object_order_master_category(position) & bit == 0
                {
                    break;
                }
                next = position.checked_sub(1);
            }

            let mut first = next.map_or(0, |position| position + 1);
            while first <= last && !self.object_order_master_active(first) {
                first += 1;
            }
            if first <= last {
                self.sort_object_order_span(order, first, last);
            }

            cursor = next;
            if cursor.is_none() {
                return;
            }
            bit <<= 1;
        }
    }

    /// C4ObjResort::Sort. The physical master-list span stays fixed while
    /// object payloads bubble from Last toward First. Comparisons therefore
    /// run in the exact legacy order; only a negative integer swaps.
    fn sort_object_order_span(&mut self, order: &ObjectOrderFunction, first: usize, last: usize) {
        let first_backup = first;
        let mut first = first;
        while first != last {
            let mut current = last;
            let mut new_first = last;
            while current != first {
                let Some(mut previous) = current.checked_sub(1) else {
                    break;
                };
                while !self.object_order_master_active(previous) {
                    let Some(earlier) = previous.checked_sub(1) else {
                        break;
                    };
                    previous = earlier;
                }
                if !self.object_order_master_active(previous) {
                    break;
                }
                let Some(current_object) = self.object_order_master_id(current) else {
                    current = previous;
                    continue;
                };
                let Some(previous_object) = self.object_order_master_id(previous) else {
                    current = previous;
                    continue;
                };
                if self.call_object_order_function(order, current_object, previous_object) < 0
                    && self.swap_object_order_master_links(current, previous)
                {
                    for position in [current, previous] {
                        if let Some(index) = self
                            .object_order_master_id(position)
                            .and_then(|id| self.find_object_index(id))
                        {
                            self.objects[index].unsorted = true;
                        }
                    }
                    new_first = current;
                }
                current = previous;
            }
            first = new_first;
        }

        // UpdatePosResort is delayed until every comparison is complete and
        // scans the original physical span in final master-forward order.
        for position in first_backup..=last {
            let Some(object_id) = self.object_order_master_id(position) else {
                continue;
            };
            let should_resort = self.find_object_index(object_id).is_some_and(|index| {
                let object = &mut self.objects[index];
                if !object.destroyed
                    && object.state.status == ObjectStatus::Normal
                    && object.unsorted
                {
                    object.unsorted = false;
                    true
                } else {
                    false
                }
            });
            if should_resort {
                self.update_pos_resort(object_id);
            }
        }
    }

    /// C4ObjResort::SortObject. Try the master-forward direction first; only
    /// if it records no negative comparison does the legacy code walk back
    /// toward First. Zero keeps scanning, positive stops, and category
    /// compatibility uses the full raw category intersection.
    fn execute_order_function_object(
        &mut self,
        order: &ObjectOrderFunction,
        object: ObjectId,
    ) -> bool {
        let Some(object_index) = self.find_object_index(object) else {
            return false;
        };
        if self.objects[object_index].destroyed
            || self.objects[object_index].state.status != ObjectStatus::Normal
            || self.objects[object_index].unsorted
        {
            return false;
        }
        let Some(origin) = self
            .exec_list
            .iter()
            .rev()
            .position(|candidate| *candidate == object)
        else {
            return false;
        };

        let mut move_after = None;
        let mut position = origin + 1;
        while position < self.exec_list.len() {
            let Some(candidate) = self.object_order_master_id(position) else {
                break;
            };
            let Some(candidate_index) = self.find_object_index(candidate) else {
                position += 1;
                continue;
            };
            if self.objects[candidate_index].destroyed
                || self.objects[candidate_index].state.status != ObjectStatus::Normal
            {
                position += 1;
                continue;
            }
            let Some(current_object_index) = self.find_object_index(object) else {
                break;
            };
            if self.objects[candidate_index].state.category
                & self.objects[current_object_index].state.category
                == 0
            {
                break;
            }
            let result = self.call_object_order_function(order, candidate, object);
            if result > 0 {
                break;
            }
            if result < 0 {
                move_after = Some(candidate);
            }
            position += 1;
        }

        if let Some(relative_to) = move_after {
            return self.move_object_order_relative(object, relative_to, true);
        }

        let mut move_before = None;
        let mut position = origin;
        while let Some(previous) = position.checked_sub(1) {
            position = previous;
            let Some(candidate) = self.object_order_master_id(position) else {
                continue;
            };
            let Some(candidate_index) = self.find_object_index(candidate) else {
                continue;
            };
            if self.objects[candidate_index].destroyed
                || self.objects[candidate_index].state.status != ObjectStatus::Normal
            {
                continue;
            }
            let Some(current_object_index) = self.find_object_index(object) else {
                break;
            };
            if self.objects[candidate_index].state.category
                & self.objects[current_object_index].state.category
                == 0
            {
                break;
            }
            let result = self.call_object_order_function(order, object, candidate);
            if result > 0 {
                break;
            }
            if result < 0 {
                move_before = Some(candidate);
            }
        }

        move_before
            .is_some_and(|relative_to| self.move_object_order_relative(object, relative_to, false))
    }

    /// Move one existing payload directly before/after another in C++
    /// master order. In the reversed exec representation those insertion
    /// sides are inverted.
    fn move_object_order_relative(
        &mut self,
        object: ObjectId,
        relative_to: ObjectId,
        after: bool,
    ) -> bool {
        if object == relative_to {
            return false;
        }

        // Inactive objects belong to a separate C++ list. Preserve their
        // unified Rust ledger slots while performing the remove/insert over
        // the logical Game.Objects links (including retained Deleted links).
        let mut inactive_slots = Vec::new();
        let mut logical_links = Vec::with_capacity(self.exec_list.len());
        for (position, &id) in self.exec_list.iter().enumerate() {
            let inactive = self
                .find_object_index(id)
                .is_some_and(|index| self.objects[index].state.status == ObjectStatus::Inactive);
            if inactive {
                inactive_slots.push((position, id));
            } else {
                logical_links.push(id);
            }
        }

        let Some(object_position) = logical_links.iter().position(|id| *id == object) else {
            return false;
        };
        if !logical_links.contains(&relative_to) {
            return false;
        }
        logical_links.remove(object_position);
        let Some(relative_position) = logical_links.iter().position(|id| *id == relative_to) else {
            return false;
        };
        let insert_at = if after {
            // Master AFTER is exec BEFORE.
            relative_position
        } else {
            // Master BEFORE is exec AFTER.
            relative_position + 1
        };
        logical_links.insert(insert_at, object);
        for (position, inactive) in inactive_slots {
            logical_links.insert(position.min(logical_links.len()), inactive);
        }
        self.exec_list = logical_links;
        true
    }

    /// C4GameObjects::ResortUnsorted, scanning main-list First -> Next.
    /// `exec_list` is the reverse view; each object is cleared immediately
    /// before its Add, while later flagged peers remain invisible to both
    /// insertion scans.
    pub(crate) fn resort_all_unsorted(&mut self) {
        let resort_order = self
            .exec_list
            .iter()
            .rev()
            .copied()
            .filter(|object| {
                self.find_object_index(*object).is_some_and(|index| {
                    let object = &self.objects[index];
                    object.unsorted && object.state.status != ObjectStatus::Inactive
                })
            })
            .collect::<Vec<_>>();
        let mut unprocessed_resorts = resort_order.iter().copied().collect::<HashSet<_>>();
        for object in resort_order {
            unprocessed_resorts.remove(&object);
            if let Some(index) = self.find_object_index(object) {
                self.objects[index].unsorted = false;
            }
            self.resort_object(object, &unprocessed_resorts);
            self.update_pos_resort(object);
        }
    }

    fn sort_exec_list_by_category(&mut self) {
        let mut keyed = self
            .exec_list
            .iter()
            .enumerate()
            .filter_map(|(position, &id)| {
                let index = self.find_object_index(id)?;
                // C4GameObjects keeps inactive objects in a separate list;
                // preserve their physical slots in Rust's unified view.
                if self.objects[index].state.status == ObjectStatus::Inactive {
                    return None;
                }
                let category = self.objects[index].state.category & CATEGORY_SORT_LIMIT;
                Some((category, position, id))
            })
            .collect::<Vec<_>>();
        let positions = keyed
            .iter()
            .map(|&(_, position, _)| position)
            .collect::<Vec<_>>();
        keyed.sort_by_key(|&(category, position, _)| (category, position));
        for (position, id) in positions
            .into_iter()
            .zip(keyed.into_iter().map(|(_, _, id)| id))
        {
            self.exec_list[position] = id;
        }
        let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        if let Some(sectors) = self.sectors.as_mut() {
            sectors.set_master_order(master_order);
        }
    }

    pub(crate) fn resort_object(&mut self, object: ObjectId, still_unsorted: &HashSet<ObjectId>) {
        let Some(object_index) = self.find_object_index(object) else {
            return;
        };
        if self.objects[object_index].state.status == ObjectStatus::Inactive {
            return;
        }
        let Some(position) = self.exec_list.iter().position(|&id| id == object) else {
            return;
        };
        self.exec_list.remove(position);
        if self.objects[object_index].destroyed
            || self.objects[object_index].state.status == ObjectStatus::Deleted
        {
            let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
            if let Some(sectors) = self.sectors.as_mut() {
                sectors.set_master_order(master_order);
            }
            return;
        }
        self.insert_into_exec_list_ignoring(object, false, Some(still_unsorted));
    }

    fn execute_relative_object_order_command(
        &mut self,
        relative_to: ObjectId,
        object: ObjectId,
        after: bool,
    ) -> bool {
        let Some(object_index) = self.find_object_index(object) else {
            return false;
        };
        let Some(relative_index) = self.find_object_index(relative_to) else {
            return false;
        };
        if self.objects[object_index].destroyed
            || !self.objects[object_index].state.status.is_active()
            || self.objects[object_index].unsorted
            || self.objects[relative_index].destroyed
            || !self.objects[relative_index].state.status.is_active()
        {
            return false;
        }
        let object_category = self.objects[object_index].state.category & CATEGORY_SORT_LIMIT;
        let relative_category = self.objects[relative_index].state.category & CATEGORY_SORT_LIMIT;
        // C4GameObjects::OrderObjectBefore/After protect category sorting
        // with opposite one-sided comparisons (C4GameObjects.cpp:749-769).
        if (!after && object_category < relative_category)
            || (after && object_category > relative_category)
        {
            return false;
        }
        let logical_links = self
            .exec_list
            .iter()
            .copied()
            .filter(|id| {
                self.find_object_index(*id)
                    .is_none_or(|index| self.objects[index].state.status != ObjectStatus::Inactive)
            })
            .collect::<Vec<_>>();
        let Some(object_position) = logical_links.iter().position(|&id| id == object) else {
            return false;
        };
        let Some(relative_position) = logical_links.iter().position(|&id| id == relative_to) else {
            return false;
        };

        // C4ObjectList's wrappers report success when the requested relation
        // is already satisfied; C4GameObjects still calls UpdatePosResort on
        // the target in that case. Compare the logical main list with Rust's
        // inactive-only ledger slots projected out.
        let already_satisfied = if after {
            // Main-list AFTER is exec-list BEFORE.
            object_position < relative_position
        } else {
            // Main-list BEFORE is exec-list AFTER.
            object_position > relative_position
        };
        if !already_satisfied && !self.move_object_order_relative(object, relative_to, after) {
            return false;
        }
        true
    }

    pub(crate) fn insert_exec_link(&mut self, position: usize, id: ObjectId) {
        self.exec_list.insert(position, id);
        self.exec_list_insert_generation = self.exec_list_insert_generation.wrapping_add(1);
        let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        if let Some(sectors) = self.sectors.as_mut() {
            sectors.set_master_order(master_order);
        }
        if let Some(cursor) = self.exec_cursor {
            if position < cursor {
                self.exec_cursor = Some(cursor + 1);
            }
        }
    }

    /// Install the exact `Game.Objects` order captured at the end of one
    /// synchronous effect callback batch. `exec_list` stores that list in
    /// reverse and also retains inactive-object ledger slots; keep those
    /// private slots while replacing the logical main-list links.
    pub(crate) fn install_effect_object_lists(&mut self, preview: compat::EffectObjectListPreview) {
        let compat::EffectObjectListPreview {
            master_order,
            inactive_order,
            sectors,
        } = preview;
        let next_exec = self
            .exec_cursor
            .and_then(|cursor| self.exec_list.get(cursor).copied());
        let previous_cursor = self.exec_cursor;
        let retained_ledger_slots = self
            .exec_list
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, id)| {
                self.find_object_index(*id).is_some_and(|index| {
                    let object = &self.objects[index];
                    object.destroyed || object.state.status != ObjectStatus::Normal
                })
            })
            .collect::<Vec<_>>();
        let mut seen = HashSet::with_capacity(master_order.len());
        let mut exact = master_order
            .iter()
            .rev()
            .copied()
            .filter(|id| {
                seen.insert(*id)
                    && self.find_object_index(*id).is_some_and(|index| {
                        let object = &self.objects[index];
                        !object.destroyed && object.state.status == ObjectStatus::Normal
                    })
            })
            .collect::<Vec<_>>();
        for (position, id) in retained_ledger_slots {
            exact.insert(position.min(exact.len()), id);
        }
        if self.exec_list != exact {
            self.exec_list = exact;
            self.exec_list_insert_generation = self.exec_list_insert_generation.wrapping_add(1);
            self.exec_cursor = next_exec
                .and_then(|id| self.exec_list.iter().position(|candidate| *candidate == id))
                .or_else(|| previous_cursor.map(|cursor| cursor.min(self.exec_list.len())));
        }
        let inactive_exec_list = inactive_order
            .into_iter()
            .rev()
            .filter(|id| {
                self.find_object_index(*id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.destroyed && object.state.status == ObjectStatus::Inactive
                })
            })
            .collect();
        self.inactive_exec_list = inactive_exec_list;
        if let Some(mut sectors) = sectors {
            sectors.set_master_order(master_order);
            self.sectors = Some(sectors);
        } else if let Some(sectors) = self.sectors.as_mut() {
            sectors.set_master_order(master_order);
        }
    }

    /// C4ObjectList::Add(stMain) for `Game.Objects.InactiveObjects`, stored
    /// in the same reversed representation as `exec_list`.
    pub(crate) fn insert_into_inactive_list(&mut self, id: ObjectId, loaded: bool) {
        self.inactive_exec_list.retain(|other| *other != id);
        if loaded {
            self.inactive_exec_list.push(id);
            return;
        }
        let Some(index) = self.find_object_index(id) else {
            return;
        };
        let is_line = |engine: &Self, idx: usize| {
            engine
                .definitions
                .get(&engine.objects[idx].definition_id)
                .map(|definition| definition.line() != 0)
                .unwrap_or(false)
        };
        if is_line(self, index) {
            self.inactive_exec_list.insert(0, id);
            return;
        }
        let category = self.objects[index].state.category;
        let sort_category = category & CATEGORY_SORT_LIMIT;
        let definition_id = self.objects[index].definition_id.clone();
        if category & CATEGORY_STATIC_BACK == 0 {
            if let Some(position) = self.inactive_exec_list.iter().rposition(|&other| {
                self.find_object_index(other).is_some_and(|other_index| {
                    let object = &self.objects[other_index];
                    !object.destroyed
                        && object.state.status == ObjectStatus::Inactive
                        && object.state.category & CATEGORY_SORT_LIMIT == sort_category
                        && object.definition_id == definition_id
                })
            }) {
                self.inactive_exec_list.insert(position + 1, id);
                return;
            }
        }
        let bracket_position = self.inactive_exec_list.iter().rposition(|&other| {
            self.find_object_index(other).is_some_and(|other_index| {
                let object = &self.objects[other_index];
                !object.destroyed
                    && object.state.status == ObjectStatus::Inactive
                    && object.state.category & CATEGORY_SORT_LIMIT <= sort_category
            })
        });
        match bracket_position {
            Some(position) => self.inactive_exec_list.insert(position + 1, id),
            None => self.inactive_exec_list.insert(0, id),
        }
    }

    pub(crate) fn update_inactive_list_for_status_change(
        &mut self,
        id: ObjectId,
        previous: ObjectStatus,
        current: ObjectStatus,
    ) {
        if previous == current {
            return;
        }
        if previous == ObjectStatus::Inactive {
            self.inactive_exec_list.retain(|other| *other != id);
        }
        if previous == ObjectStatus::Inactive && current == ObjectStatus::Normal {
            if let Some(index) = self.find_object_index(id) {
                self.objects[index].refresh_shape_geometry();
                self.update_sector_for_index(index);
            }
            // StatusActivate calls Game.Objects.Add(this), so it receives a
            // fresh stMain position rather than recovering its old slot.
            if let Some(position) = self.exec_list.iter().position(|other| *other == id) {
                self.exec_list.remove(position);
                if let Some(cursor) = self.exec_cursor {
                    if position < cursor {
                        self.exec_cursor = Some(cursor - 1);
                    }
                }
            }
            self.insert_into_exec_list(id, false);
        }
        if current == ObjectStatus::Inactive {
            self.insert_into_inactive_list(id, false);
        }
    }

    /// Repair imported/legacy state that predates the dedicated inactive
    /// ordering ledger. Normal runtime transitions update the list eagerly;
    /// this fallback only inserts missing live inactive objects.
    pub(crate) fn reconcile_inactive_list(&mut self) {
        let previous = std::mem::take(&mut self.inactive_exec_list);
        self.inactive_exec_list = previous
            .into_iter()
            .filter(|&id| {
                self.find_object_index(id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.destroyed && object.state.status == ObjectStatus::Inactive
                })
            })
            .collect();
        let missing = self
            .objects
            .iter()
            .filter(|object| {
                !object.destroyed
                    && object.state.status == ObjectStatus::Inactive
                    && !self.inactive_exec_list.contains(&object.id)
            })
            .map(|object| object.id)
            .collect::<Vec<_>>();
        for id in missing {
            self.insert_into_inactive_list(id, false);
        }
    }

    pub(crate) fn insert_into_exec_list(&mut self, id: ObjectId, loaded: bool) {
        self.insert_into_exec_list_ignoring(id, loaded, None);
    }

    fn insert_into_exec_list_ignoring(
        &mut self,
        id: ObjectId,
        loaded: bool,
        ignored: Option<&HashSet<ObjectId>>,
    ) {
        let Some(index) = self.find_object_index(id) else {
            return;
        };
        let object = &self.objects[index];
        let is_line = self
            .definitions
            .get(&object.definition_id)
            .is_some_and(|definition| definition.line() != 0);
        let position = self.exec_insert_position(
            Some(id),
            loaded,
            object.unsorted,
            is_line,
            object.state.category,
            &object.definition_id,
            ignored,
        );
        self.insert_exec_link(position, id);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn exec_insert_position(
        &self,
        id: Option<ObjectId>,
        loaded: bool,
        unsorted: bool,
        is_line: bool,
        category: i32,
        definition_id: &str,
        ignored: Option<&HashSet<ObjectId>>,
    ) -> usize {
        if loaded {
            return self.exec_list.len();
        }
        if is_line || unsorted {
            return 0;
        }
        let sort_category = category & CATEGORY_SORT_LIMIT;
        let master_order = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        // The scans consider live sorted links only (Status && !Unsorted,
        // :156,:168). Runtime removals are pruned separately.
        let live_index = |engine: &Self, other: ObjectId| {
            (id != Some(other)
                && ignored.is_none_or(|ignored| !ignored.contains(&other))
                && engine.find_object_index(other).is_some_and(|index| {
                    let object = &engine.objects[index];
                    !object.destroyed && object.state.status.is_active() && !object.unsorted
                }))
            .then(|| engine.find_object_index(other))
            .flatten()
            .filter(|&other_index| {
                let object = &engine.objects[other_index];
                !object.destroyed && object.state.status == ObjectStatus::Normal
            })
        };
        let mut predecessor = None;
        let mut found_cluster = false;
        if category & CATEGORY_STATIC_BACK == 0 {
            for (position, &other) in master_order.iter().enumerate() {
                let Some(other_index) = live_index(self, other) else {
                    continue;
                };
                let matches = {
                    let object = &self.objects[other_index];
                    object.state.category & CATEGORY_SORT_LIMIT == sort_category
                        && object.definition_id == definition_id
                };
                if matches {
                    found_cluster = true;
                    break;
                }
                predecessor = Some(position);
            }
        }
        if !found_cluster {
            predecessor = None;
            for (position, &other) in master_order.iter().enumerate() {
                let Some(other_index) = live_index(self, other) else {
                    continue;
                };
                if self.objects[other_index].state.category & CATEGORY_SORT_LIMIT <= sort_category {
                    break;
                }
                predecessor = Some(position);
            }
        }
        // C++ inserts at cPrev->Next, not immediately before the qualifying
        // live link. Dead/Unsorted links skipped after cPrev therefore stay
        // after the newcomer. `exec_list` is the reverse of this master view.
        let master_position = predecessor.map_or(0, |position| position + 1);
        master_order.len() - master_position
    }

    #[doc(hidden)]
    pub fn find_object_index(&self, id: ObjectId) -> Option<usize> {
        let generation = self.objects_generation.get();
        {
            let cache = self.object_index_cache.borrow();
            if cache.0 == generation {
                match cache.1.get(&id).copied() {
                    // Identity-checked: a stale hit (missed generation bump)
                    // falls through to the rebuild instead of resolving the
                    // wrong object.
                    Some(index) if self.objects.get(index).map(|object| object.id) == Some(id) => {
                        return Some(index);
                    }
                    Some(_) => {}
                    None => return None,
                }
            }
        }
        let mut cache = self.object_index_cache.borrow_mut();
        cache.0 = generation;
        cache.1.clear();
        cache.1.extend(
            self.objects
                .iter()
                .enumerate()
                .map(|(i, object)| (object.id, i)),
        );
        cache.1.get(&id).copied()
    }

    pub(crate) fn note_objects_changed(&self) {
        self.objects_generation
            .set(self.objects_generation.get().wrapping_add(1));
        self.note_solid_mask_host_state_changed();
    }

    pub(crate) fn layer_movement_bounds_for(&self, index: usize) -> Option<LayerMovementBounds> {
        let layer_id = self.objects.get(index)?.state.layer?;
        let layer = self.objects.iter().find(|object| object.id == layer_id)?;
        let definition = self.definitions.get(&layer.definition_id)?;
        Some(LayerMovementBounds {
            position: layer.state.position,
            shape_rect: layer.current_shape_rect()?,
            border_bound: definition.border_bound(),
        })
    }

    /// Whether masks bake into the plane (a pixel grid with a Vehicle
    /// slot exists). Without it the rect overlay below stays in force.
    pub(crate) fn solid_mask_grid_mode(&self) -> bool {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.grid_vehicle_byte())
            .is_some()
    }

    /// C4SolidMask copies alpha from `pForObject->GetGraphics()->GetBitmap()`
    /// (C4SolidMask.cpp:400-412), not necessarily the owning definition's
    /// default sprite. Mask geometry still belongs to the owning definition.
    pub(crate) fn checked_solid_mask_rect_for_object(
        &self,
        object: &Object,
        mask: DefinitionTargetRect,
    ) -> Option<DefinitionTargetRect> {
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
        let definition = self.definitions.get(graphics_definition)?;
        match definition.sprite_image_variant(graphics_name) {
            Some(image) => Some(mask.checked_for_solid_mask_bitmap(
                i32::try_from(image.width).unwrap_or(i32::MAX),
                i32::try_from(image.height).unwrap_or(i32::MAX),
            )),
            None if graphics_name.is_none() => Some(mask),
            None => None,
        }
    }
}

#[cfg(test)]
mod hasher_tests {
    use super::*;
    use std::hash::BuildHasher;

    /// The per-frame lookup tables are only ever probed by key: every consumer
    /// that ranks their contents sorts on an explicit total order (master-list
    /// rank then `ObjectId`). `RandomState` reseeds per process, so a
    /// simulation outcome that read iteration order would already desync
    /// between two runs of one seed. The engine hasher therefore carries no
    /// per-process seed, which is strictly more reproducible.
    #[test]
    fn per_frame_lookup_tables_hash_without_a_per_process_seed() {
        let id = ObjectId::new(7);
        let first = rustc_hash::FxBuildHasher;
        let second = rustc_hash::FxBuildHasher;
        assert_eq!(first.hash_one(id), second.hash_one(id));

        // `Engine::definitions` is keyed by `DefinitionId`, which the
        // per-tick `active_solid_mask_indices` probe hashes on every frame.
        let definition = DefinitionId::from("CLNK");
        assert_eq!(
            rustc_hash::FxBuildHasher.hash_one(&definition),
            rustc_hash::FxBuildHasher.hash_one(&definition)
        );
        assert_ne!(
            std::hash::RandomState::new().hash_one(&definition),
            std::hash::RandomState::new().hash_one(&definition)
        );

        let seeded = std::hash::RandomState::new();
        let reseeded = std::hash::RandomState::new();
        assert_ne!(seeded.hash_one(id), reseeded.hash_one(id));
    }
}
