//! `impl Engine` — crew selection, context menus, object calls, controls and roles.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub fn crew_members(&self, owner: i32) -> Vec<ObjectId> {
        if let Some(player) = self.players.get(&owner) {
            return player
                .crew()
                .iter()
                .copied()
                .filter(|id| {
                    self.find_object_index(*id).is_some_and(|index| {
                        let object = &self.objects[index];
                        !object.destroyed && object.state.status != ObjectStatus::Deleted
                    })
                })
                .collect();
        }
        self.objects
            .iter()
            .filter(|object| {
                object.state.crew_member
                    && object.state.owner == owner
                    && object.state.status.is_active()
            })
            .map(|object| object.id)
            .collect()
    }

    pub fn eliminated_owners(&self) -> Vec<i32> {
        let mut eliminated: Vec<_> = self.eliminated_crew_owners.iter().cloned().collect();
        eliminated.sort_unstable();
        eliminated
    }

    pub fn is_owner_eliminated(&self, owner: i32) -> bool {
        self.eliminated_crew_owners.contains(&owner)
            || self.players.get(&owner).is_some_and(|player| {
                matches!(
                    player.status(),
                    PlayerStatus::Eliminated | PlayerStatus::Surrendered
                ) || player.surrendered()
            })
    }

    pub fn selected_crew(&self, owner: i32) -> Vec<ObjectId> {
        // GetCursor(index) scans C4Player::Crew in its stored list order
        // (C4Script.cpp:2905-2928). Object numbers are not an ordering key:
        // loaded games may assign arbitrary ids, so follow the player's
        // actual roster and only fall back to object insertion order in the
        // playerless engine fixtures.
        let roster = self
            .players
            .get(&owner)
            .map(|player| player.crew().to_vec())
            .unwrap_or_else(|| self.crew_members(owner));
        roster
            .into_iter()
            .filter(|id| {
                self.find_object_index(*id)
                    .is_some_and(|index| self.objects[index].state.selected)
            })
            .collect()
    }

    fn crew_selection_state(&self, owner: i32) -> CrewSelectionState {
        CrewSelectionState {
            selected: self.selected_crew(owner),
            cursor: self.crew_cursor(owner),
        }
    }

    pub(crate) fn crew_selection_states(&self) -> HashMap<i32, CrewSelectionState> {
        let mut owners: HashSet<i32> = self.crew_selection.keys().copied().collect();
        owners.extend(
            self.objects
                .iter()
                .filter(|object| object.state.selected)
                .map(|object| object.state.owner),
        );
        owners
            .into_iter()
            .filter_map(|owner| {
                let state = self.crew_selection_state(owner);
                (!state.selected.is_empty() || state.cursor.is_some()).then_some((owner, state))
            })
            .collect()
    }

    pub fn crew_cursor(&self, owner: i32) -> Option<ObjectId> {
        self.crew_selection
            .get(&owner)
            .and_then(|selection| selection.cursor())
    }

    pub fn select_crew<I>(&mut self, owner: i32, crew: I) -> Result<(), EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        let mut validated = Vec::new();
        for id in crew {
            let object = self
                .objects
                .iter()
                .find(|object| object.id == id)
                .ok_or(EngineError::UnknownObject(id))?;
            if let Some(player) = self.players.get(&owner) {
                if !player.crew().contains(&id) {
                    return Err(EngineError::CrewSelection {
                        owner,
                        detail: format!("object {} is not in this player's crew", id),
                    });
                }
            } else {
                if object.state.owner != owner {
                    return Err(EngineError::CrewSelection {
                        owner,
                        detail: format!("object {} is owned by {}", id, object.state.owner),
                    });
                }
                if !object.state.crew_member {
                    return Err(EngineError::CrewSelection {
                        owner,
                        detail: format!("object {} is not a crew member", id),
                    });
                }
            }
            if !object.state.status.is_active() {
                return Err(EngineError::CrewSelection {
                    owner,
                    detail: format!("object {} is not active", id),
                });
            }
            validated.push(id);
        }

        if validated.is_empty() {
            return Ok(());
        }

        for id in validated {
            if let Some(index) = self.find_object_index(id) {
                self.object_do_select(index, owner, false)?;
            }
        }
        // Crew selection flashes the select marks (C4Player::SelectCrew,
        // C4Player.cpp:1846 / SelectSingleByCursor, :1317).
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.select_flash = 30;
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
        }
        self.player_adjust_cursor_command(owner)?;
        Ok(())
    }

    pub fn deselect_crew<I>(&mut self, owner: i32, crew: I)
    where
        I: IntoIterator<Item = ObjectId>,
    {
        let roster = self
            .players
            .get(&owner)
            .map(|player| player.crew().to_vec())
            .unwrap_or_else(|| self.crew_members(owner));
        for id in crew {
            if let Some(index) = self.find_object_index(id) {
                if roster.contains(&id) {
                    let _ = self.object_un_select(index, owner, false);
                }
            }
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.select_flash = 30;
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
        }
        let _ = self.player_adjust_cursor_command(owner);
    }

    pub fn clear_crew_selection(&mut self, owner: i32) {
        let _ = self.player_unselect_crew(owner);
        self.crew_selection.remove(&owner);
        self.sync_player_cursor(owner);
    }

    pub fn set_crew_cursor(
        &mut self,
        owner: i32,
        cursor: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        match cursor {
            Some(id) => {
                let object = self
                    .objects
                    .iter()
                    .find(|object| object.id == id)
                    .ok_or(EngineError::UnknownObject(id))?;
                if let Some(player) = self.players.get(&owner) {
                    if !player.crew().contains(&id) {
                        return Err(EngineError::CrewSelection {
                            owner,
                            detail: format!("object {} is not in this player's crew", id),
                        });
                    }
                } else {
                    if object.state.owner != owner {
                        return Err(EngineError::CrewSelection {
                            owner,
                            detail: format!("object {} is owned by {}", id, object.state.owner),
                        });
                    }
                    if !object.state.crew_member {
                        return Err(EngineError::CrewSelection {
                            owner,
                            detail: format!("object {} is not a crew member", id),
                        });
                    }
                }
                if !object.state.status.is_active() {
                    return Err(EngineError::CrewSelection {
                        owner,
                        detail: format!("object {} is not active", id),
                    });
                }
                self.player_set_cursor(owner, Some(id), false, true)?;
            }
            None => {
                self.player_set_cursor(owner, None, false, true)?;
            }
        }

        self.sync_player_cursor(owner);
        Ok(())
    }

    pub fn ensure_cursor(&mut self, owner: i32) -> Result<(), EngineError> {
        if self.crew_cursor(owner).is_some() {
            return Ok(());
        }
        self.player_adjust_cursor_command(owner)
    }

    pub fn context_menu_entries(
        &mut self,
        object_id: ObjectId,
    ) -> Result<Vec<ContextMenuEntry>, EngineError> {
        let index = self
            .objects
            .iter()
            .position(|object| object.id == object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        let definition_id = self.objects[index].definition_id.clone();
        let state_snapshot = Rc::new(self.objects[index].script_state_snapshot());
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        // C4ObjectMenu::AddContextFunctions enumerates effective annotated
        // Context* functions separately from the Rust-port MenuEntries hook
        // (C4ObjectMenu.cpp:398-399,670-682). Keep a detached copy so their
        // conditions can run through the live engine after MenuEntries releases
        // the immutable definition borrow.
        let legacy_context_functions = definition.script_context_functions();
        let definitions_ref = &self.definitions;
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let (mut entries, audio_state, new_rng) = definition.call_menu_entries(
            state_snapshot.as_ref(),
            object_id,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;

        // C++ calls each Condition on the context target with the menu object
        // and annotation image id. The player-facing menu is built for its
        // cursor, so the target and menu object are this same crew object.
        for context in legacy_context_functions {
            let image = context.image.as_deref().unwrap_or("NONE");
            let enabled = match context.condition.as_deref() {
                Some(condition) => {
                    let value = self.call_object_function(
                        index,
                        condition,
                        vec![
                            compat::object_reference_value(object_id),
                            Value::C4Id(image.to_owned()),
                        ],
                    )?;
                    compat::value_raw_truthy(&value)
                }
                None => true,
            };
            if enabled {
                entries.push(ContextMenuEntry {
                    function: context.function,
                    label: context.label,
                    description: context.description,
                });
            }
        }
        Ok(entries)
    }

    pub fn menu_command(
        &mut self,
        crew_id: ObjectId,
        kind: MenuCommandKind,
        selection: MenuCommandSelection,
    ) -> Result<bool, EngineError> {
        let index = self
            .objects
            .iter()
            .position(|object| object.id == crew_id)
            .ok_or(EngineError::UnknownObject(crew_id))?;
        let definition_id = self.objects[index].definition_id.clone();
        let state_snapshot = Rc::new(self.objects[index].script_state_snapshot());
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let definitions_ref = &self.definitions;
        let action_library = definition.action_library().clone();
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let (handled, outcome, audio_state, new_rng) = definition.call_menu_command(
            state_snapshot.as_ref(),
            crew_id,
            kind,
            &selection,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            crew_id,
            &definition_id,
        )?;
        Ok(handled)
    }

    pub fn execute_context_menu(
        &mut self,
        object_id: ObjectId,
        function: &str,
    ) -> Result<bool, EngineError> {
        let index = self
            .objects
            .iter()
            .position(|object| object.id == object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        let definition_id = self.objects[index].definition_id.clone();
        let state_snapshot = Rc::new(self.objects[index].script_state_snapshot());
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let is_legacy_context = definition
            .script_context_functions()
            .iter()
            .any(|context| context.function == function);
        if is_legacy_context {
            // C4ObjectMenu installs
            // ProtectedCall(target, "Context*", menu_object): the selected
            // legacy function receives a live object reference, not the state
            // proplist used by the Rust-port MenuEntries callback convention
            // (C4ObjectMenu.cpp:650-665,678-680).
            let value = self.call_object_function(
                index,
                function,
                vec![compat::object_reference_value(object_id)],
            )?;
            return Ok(compat::value_raw_truthy(&value));
        }
        let definitions_ref = &self.definitions;
        let action_library = definition.action_library().clone();
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let (handled, outcome, audio_state, new_rng) = definition.call_menu_callback(
            state_snapshot.as_ref(),
            object_id,
            function,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            object_id,
            &definition_id,
        )?;
        Ok(handled)
    }

    #[doc(hidden)]
    pub fn call_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: Vec<Value>,
    ) -> Result<Value, EngineError> {
        self.call_object_callback(index, &ScriptCallbackTarget::unlinked(function), args)
    }

    pub(crate) fn call_object_callback(
        &mut self,
        index: usize,
        callback: &ScriptCallbackTarget,
        args: Vec<Value>,
    ) -> Result<Value, EngineError> {
        self.call_object_callback_with_status_gate(index, None, callback, args, true)
    }

    /// Execute an already resolved definition script function with the raw
    /// receiver semantics of `C4AulScriptFunc::Exec`. Only native call sites
    /// that bypass `C4Object::Call` may use this path.
    pub(crate) fn call_direct_object_callback(
        &mut self,
        index: usize,
        callback: &ScriptCallbackTarget,
        args: Vec<Value>,
    ) -> Result<Value, EngineError> {
        self.call_object_callback_with_status_gate(index, None, callback, args, false)
    }

    /// Direct `C4AulFunc::Exec` with independently retained function owner
    /// and receiver. The pinned body/helper/static lookup stays on
    /// `callback_definition_id`; `this`, object locals and metadata come
    /// from `index` (C4Object.cpp:3293-3298; C4AulExec.cpp:330-359).
    pub(crate) fn call_direct_object_callback_from_definition(
        &mut self,
        index: usize,
        callback_definition_id: &DefinitionId,
        callback: &ScriptCallbackTarget,
        args: Vec<Value>,
    ) -> Result<Value, EngineError> {
        self.call_object_callback_with_status_gate(
            index,
            Some(callback_definition_id),
            callback,
            args,
            false,
        )
    }

    /// Execute a captured definition function with `C4AulFunc::Exec`'s
    /// null-object receiver. `C4Object::ContainedControl` uses this after
    /// old-version hardcoded controls even when those controls moved the
    /// clonk out of its container: the pinned function still runs with its
    /// owner's definition context and `this == nil` (C4Object.cpp:3293-3298;
    /// C4AulExec.cpp:330-359).
    pub(crate) fn call_direct_definition_callback(
        &mut self,
        definition_id: &DefinitionId,
        callback: &ScriptCallbackTarget,
        args: Vec<Value>,
    ) -> Result<Value, EngineError> {
        let definition = self
            .definitions
            .get(definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let script = definition.script_arc();
        let world = self.host_world_context();
        let (value, _final_args, batch, audio_state, rng, script_error) =
            ScenarioScript::execute_value_for_script(
                definition_id,
                Some(definition_id.clone()),
                callback.function_name(),
                &args,
                world,
                self.rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
                || match callback.resolution() {
                    Some(resolution) => script.call_resolved_with_ref_args(
                        resolution,
                        resolution.scope == clonk_script::ScriptFunctionScope::Global,
                        &args,
                    ),
                    None => script.call_with_ref_args(callback.function_name(), &args),
                },
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            return Err(error);
        }
        Ok(value.unwrap_or(Value::Nil))
    }

    fn call_object_callback_with_status_gate(
        &mut self,
        index: usize,
        callback_definition_id: Option<&DefinitionId>,
        callback: &ScriptCallbackTarget,
        args: Vec<Value>,
        require_nonzero_status: bool,
    ) -> Result<Value, EngineError> {
        let (object_id, definition_id, state_snapshot) = {
            let object = self
                .objects
                .get(index)
                .ok_or_else(|| EngineError::UnknownObject(ObjectId::new(u64::MAX)))?;
            // C4Object::Call is a silent no-op once Status reaches zero
            // (C4Object.cpp:2224-2227). The tombstone remains addressable
            // for the rest of the current native call, but scripts cannot
            // be re-entered through it.
            if require_nonzero_status
                && (object.destroyed || object.state.status == ObjectStatus::Deleted)
            {
                return Ok(Value::Nil);
            }
            (
                object.id,
                object.definition_id.clone(),
                Rc::new(object.script_state_snapshot()),
            )
        };
        let object_definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let callback_definition = match callback_definition_id {
            Some(callback_definition_id) => self
                .definitions
                .get(callback_definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(callback_definition_id.clone()))?,
            None => object_definition,
        };
        let action_library = object_definition.action_library().clone();
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let call = callback_definition.call_object_callback(
            object_definition,
            state_snapshot.as_ref(),
            object_id,
            callback,
            &args,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (value, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            // Pre-error mutations persist (C++ mutated the live objects).
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
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            object_id,
            &definition_id,
        )?;
        Ok(value)
    }

    /// C4Object::MenuCommand (C4Object.cpp:3756-3760): DirectExec `source`
    /// in the object's own script context and fold the outcome like any
    /// object call. Error handling (fPassErrors=false — log and continue)
    /// is the caller's via `tolerate_script_error`.
    pub(crate) fn direct_exec_on_object(
        &mut self,
        index: usize,
        source: &str,
        label: &str,
    ) -> Result<Value, EngineError> {
        self.direct_exec_on_object_impl(index, source, label, None)
    }

    pub(crate) fn direct_exec_on_object_at_strict(
        &mut self,
        index: usize,
        source: &str,
        label: &str,
        strict_level: Option<u8>,
    ) -> Result<Value, EngineError> {
        self.direct_exec_on_object_impl(index, source, label, Some(strict_level))
    }

    fn direct_exec_on_object_impl(
        &mut self,
        index: usize,
        source: &str,
        label: &str,
        strict_level: Option<Option<u8>>,
    ) -> Result<Value, EngineError> {
        let (object_id, definition_id, state_snapshot) = {
            let object = self
                .objects
                .get(index)
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
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let call = match strict_level {
            Some(strict_level) => definition.direct_exec_object_expression_at_strict(
                state_snapshot.as_ref(),
                object_id,
                source,
                label,
                strict_level,
                rng_state,
                &global_view,
                self.physics,
                self.environment,
                self.frame,
                world,
                self.game_over_triggered,
                self.audio_registry.clone(),
            ),
            None => definition.direct_exec_object_expression(
                state_snapshot.as_ref(),
                object_id,
                source,
                label,
                rng_state,
                &global_view,
                self.physics,
                self.environment,
                self.frame,
                world,
                self.game_over_triggered,
                self.audio_registry.clone(),
            ),
        };
        let (value, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            // Pre-error mutations persist (C++ mutated the live objects).
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
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            object_id,
            &definition_id,
        )?;
        Ok(value)
    }

    /// The sim-observable core of a user Enter on the object's SCRIPT menu
    /// — C4Menu::Enter (C4Menu.cpp:498-523), reached in C++ via the queued
    /// COM_MenuEnter/COM_MenuEnterAll control (C4ObjectMenu::OnUserEnter,
    /// C4ObjectMenu.cpp:467-471; C4Object::Control -> Menu->Control,
    /// C4Object.cpp:3365-3367): the selected item's command string executes
    /// on the menu's command object (C4ObjectMenu::MenuCommand ->
    /// C4Object::MenuCommand DirectExec, C4ObjectMenu.cpp:505-527 /
    /// C4Object.cpp:3756-3760). The app-side menu UI must route its Enter
    /// here. What remains for object menus is draw-side, not input-side:
    /// native Contents/Get/Put, Activate and base-sell rows still resolve
    /// their symbol against the frame's snapshot instead of capturing it at
    /// refill as `C4ObjectMenu::RefillInternal` does, so such a row can draw
    /// blank for one frame (clonk-org/clonk-rs#364). Dialog input now receives
    /// the same resolved item icons as drawing, so `GetSymbolWidth` follows
    /// the facet's surface (`C4Menu.cpp:138`). No parity gate has a menu
    /// section, so the native-row lifetime gap is not machine-checked.
    pub fn menu_user_enter(
        &mut self,
        object_id: ObjectId,
        right: bool,
    ) -> Result<bool, EngineError> {
        const STYLE_INFO: i32 = 2; // C4MN_Style_Info (C4Menu.h:41)
        const STYLE_DIALOG: i32 = 3; // C4MN_Style_Dialog (C4Menu.h:42)
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let Some(menu) = self.objects[index].state.menu.clone() else {
            return Ok(false); // !IsActive (C4Menu.cpp:501)
        };
        if menu.style == STYLE_INFO {
            return Ok(false); // C4Menu.cpp:502
        }
        let item = usize::try_from(menu.selection)
            .ok()
            .and_then(|selection| menu.items.get(selection));
        let Some(item) = item else {
            // No selected item: dialogs try a soft close (TryClose(false,
            // true), C4Menu.cpp:508); every style reports true (:509).
            if menu.style == STYLE_DIALOG {
                self.close_object_menu(object_id, false)?;
            }
            return Ok(true);
        };
        // Copy the command to a buffer (the menu may clear); right enter
        // takes Command2 when set (C4Menu.cpp:512-514).
        let command = if right && !item.command2.is_empty() {
            item.command2.clone()
        } else {
            item.command.clone()
        };
        // Close if not permanent BEFORE the exec — Close(true) skips the
        // MenuQueryCancel query (C4Menu.cpp:517).
        if !menu.permanent {
            self.objects[index].state.menu = None;
        }
        // C4ObjectMenu::MenuCommand dispatches the copied expression either
        // on the command object or, for CB_Scenario, on Game.Script with a
        // nil object context (C4ObjectMenu.cpp:519-526). Both calls are
        // fail-safe (fPassErrors=false). C4Player::Execute performs the later
        // AutoContextMenu pass through execute_player_controls.
        if menu.scenario_callbacks {
            let strict_level = self
                .scenario_script
                .as_ref()
                .and_then(|scenario| scenario.base_script.strict_level());
            let _ = self.direct_exec_scenario_script(&command, "MenuCommand", strict_level)?;
        } else if let Some(command_object) = menu.command_object {
            if let Some(command_index) = self.find_object_index(command_object) {
                tolerate_script_error(self.direct_exec_on_object(
                    command_index,
                    &command,
                    "MenuCommand",
                ))?;
            }
        }
        // Internal permanent inventory/base menus are refill-driven. The
        // selected command may have changed stock or contents
        // synchronously, so rebuild immediately like C4ObjectMenu::Execute
        // (C4ObjectMenu.cpp:124-129, 207-326, 450-459).
        if !menu.user_menu
            && menu.permanent
            && matches!(
                &menu.identification,
                Value::Int(4) | Value::Int(5) | Value::Int(6) | Value::Int(13) | Value::Int(18)
            )
        {
            let indices = self.find_object_index(object_id).and_then(|crew_index| {
                menu.refill_object
                    .or(self.objects[crew_index].state.container)
                    .and_then(|base_id| self.find_object_index(base_id))
                    .map(|base_index| (crew_index, base_index))
            });
            if let Some((crew_index, base_index)) = indices {
                match menu.identification {
                    Value::Int(4) => self.open_base_buy_menu(crew_index, base_index)?,
                    Value::Int(5) => self.open_base_sell_menu(crew_index, base_index)?,
                    Value::Int(6) => self.open_activate_menu(crew_index, base_index)?,
                    Value::Int(13) => {
                        self.open_container_contents_menu(crew_index, base_index, 13)?;
                    }
                    Value::Int(18) => {
                        self.open_container_contents_menu(crew_index, base_index, 18)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(true)
    }

    pub(crate) fn call_movement_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: &[Value],
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
    ) -> Result<Value, EngineError> {
        let state_snapshot = Rc::new(
            self.objects
                .get(index)
                .ok_or_else(|| EngineError::UnknownObject(ObjectId::new(u64::MAX)))?
                .script_state_snapshot(),
        );
        let definition = self
            .definitions
            .get(definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.to_string()))?;
        let definitions_ref = &self.definitions;
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let call = definition.call_object_function(
            state_snapshot.as_ref(),
            object_id,
            function,
            args,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (value, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            // Pre-error mutations persist (C++ mutated the live objects).
            Err(error) => {
                return Err(self.apply_script_error_recovery(
                    error,
                    index,
                    action_library,
                    object_id,
                    definition_id,
                    false,
                ));
            }
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_callback_outcome(
            index,
            outcome,
            action_library,
            object_id,
            definition_id,
            false,
        )?;
        Ok(value)
    }

    pub(crate) fn invoke_movement_hit_callbacks(
        &mut self,
        old_velocity: FixedVec2,
        hit_speed_flags: u32,
        object_id: ObjectId,
    ) -> Result<(), EngineError> {
        let args = [
            Value::Int(fixtoi_prec(old_velocity.x, 100)),
            Value::Int(fixtoi_prec(old_velocity.y, 100)),
        ];
        for (flag, function) in [
            (crate::ocf::HIT_SPEED1, "Hit"),
            (crate::ocf::HIT_SPEED2, "Hit2"),
            (crate::ocf::HIT_SPEED3, "Hit3"),
        ] {
            if hit_speed_flags & flag == 0 {
                continue;
            }
            let Some(index) = self.find_object_index(object_id).filter(|&index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != ObjectStatus::Deleted
            }) else {
                break;
            };
            let definition_id = self.objects[index].definition_id.clone();
            let Some(action_library) = self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.action_library().clone())
            else {
                break;
            };
            // Engine-initiated lifecycle calls are fail-safe: a script
            // error in Hit/Hit2/Hit3 logs and the tick continues
            // (C4AulExec.cpp:1318-1342) — it must never kill the frame.
            let _ = tolerate_script_error(self.call_movement_object_function(
                index,
                function,
                &args,
                &action_library,
                object_id,
                &definition_id,
            ))?;
        }
        Ok(())
    }

    pub fn handle_control_command(
        &mut self,
        owner: i32,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Result<bool, EngineError> {
        let Some(function_name) = control_function_name(command, kind) else {
            return Ok(false);
        };

        self.ensure_cursor(owner)?;
        let Some(cursor) = self.crew_cursor(owner) else {
            return Ok(false);
        };

        let index = self
            .objects
            .iter()
            .position(|object| object.id == cursor)
            .ok_or(EngineError::UnknownObject(cursor))?;

        // C4Object::DirectCom (C4Object.cpp:3363-3367): a contained clonk
        // hands every non-Special com to its container and DirectCom
        // returns — the clonk's own Control<Com> is never consulted.
        if !matches!(command, ControlCommand::Special | ControlCommand::Special2) {
            if let Some(container) = self.objects[index].state.container {
                return self.contained_control(index, cursor, container, command, kind);
            }
        }

        let definition_id = self.objects[index].definition_id.clone();
        let state_snapshot = Rc::new(self.objects[index].script_state_snapshot());
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let definitions_ref = &self.definitions;
        let action_library = definition.action_library().clone();
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world =
            self.host_world_context_for_object_with_snapshot(index, Rc::clone(&state_snapshot));
        let call = definition.call_control(
            state_snapshot.as_ref(),
            cursor,
            &function_name,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        );
        let (handled, outcome, audio_state, new_rng) = match call {
            Ok(ok) => ok,
            // Pre-error mutations persist (C++ mutated the live objects).
            Err(error) => {
                return Err(self.apply_script_error_recovery(
                    error,
                    index,
                    &action_library,
                    cursor,
                    &definition_id,
                    true,
                ));
            }
        };
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            cursor,
            &definition_id,
        )?;
        Ok(handled)
    }

    /// C4Object::ContainedControl script dispatch (C4Object.cpp:3208-3306):
    /// the container's `Contained<Com>` function runs with the container as
    /// context and the clonk as parameter (`sf->Exec(Contained,
    /// {C4VObj(this)})`, :3221,3230) after the controller propagates
    /// (`Contained->Controller = Controller`, :3367). The com is consumed
    /// either way — DirectCom returns unconditionally. The hardcoded
    /// non-script fallbacks (COM_Down exit, COM_Throw command,
    /// COM_Up/COM_Dig base buy/sell, Take/Take2; :3243-3306) are not ported:
    /// each drives the container command AI (Exit/Throw/Buy/Sell), so they
    /// cannot land ahead of it (clonk-org/clonk-rs#334), and only a container
    /// whose script leaves the com unhandled reaches them at all.
    fn contained_control(
        &mut self,
        crew_index: usize,
        crew_id: ObjectId,
        container: ObjectId,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Result<bool, EngineError> {
        let Some(container_index) = self.find_object_index(container) else {
            return Ok(true);
        };
        let controller = self.objects[crew_index].state.controller;
        self.objects[container_index].state.controller = controller;
        let Some(function) = com_name(command, kind).map(|com| format!("Contained{com}")) else {
            return Ok(true);
        };
        let definition_id = self.objects[container_index].definition_id.clone();
        let action_library = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?
            .action_library()
            .clone();
        let args = [compat::object_reference_value(crew_id)];
        self.call_movement_object_function(
            container_index,
            &function,
            &args,
            &action_library,
            container,
            &definition_id,
        )?;
        Ok(true)
    }

    pub fn try_grab_nearby(&mut self, owner: i32) -> Result<bool, EngineError> {
        self.ensure_cursor(owner)?;
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(false);
        };
        let crew_index = match self.find_object_index(crew_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(crew_id)),
        };
        if !self.objects[crew_index].state.status.is_active() {
            return Ok(false);
        }
        let crew_position = self.objects[crew_index].state.position;
        let Some((_, target_id)) =
            self.find_nearby_object_with_mask(crew_id, crew_position, ocf::GRAB, 22, |object| {
                object.state.container.is_none() && object.state.status.is_active()
            })
        else {
            return Ok(false);
        };
        let update = ObjectUpdate::new()
            .with_container(crew_id)
            .with_position(crew_position)
            .with_velocity(Vector2::ZERO);
        self.apply_object_update(target_id, update)?;
        Ok(true)
    }

    pub fn try_drop_held_object(&mut self, owner: i32) -> Result<bool, EngineError> {
        self.ensure_cursor(owner)?;
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(false);
        };
        let crew_index = match self.find_object_index(crew_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(crew_id)),
        };
        if !self.objects[crew_index].state.status.is_active() {
            return Ok(false);
        }
        let Some(item_id) = self.objects[crew_index]
            .state
            .contents
            .iter()
            .copied()
            .find(|item_id| {
                self.find_object_index(*item_id)
                    .is_some_and(|index| self.objects[index].has_nonzero_status())
            })
        else {
            return Ok(false);
        };
        self.object_com_drop(crew_id, item_id)
    }

    pub fn try_enter_nearby(&mut self, owner: i32) -> Result<bool, EngineError> {
        self.ensure_cursor(owner)?;
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(false);
        };
        let crew_index = match self.find_object_index(crew_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(crew_id)),
        };
        let crew_state = &self.objects[crew_index].state;
        if crew_state.container.is_some() || !crew_state.status.is_active() {
            return Ok(false);
        }
        let crew_position = crew_state.position;
        let Some((target_index, target_id)) = self.find_nearby_object_with_mask(
            crew_id,
            crew_position,
            ocf::ENTRANCE,
            24,
            |object| object.state.status.is_active(),
        ) else {
            return Ok(false);
        };
        let target_position = self.objects[target_index].state.position;
        let update = ObjectUpdate::new()
            .with_container(target_id)
            .with_position(target_position)
            .with_velocity(Vector2::ZERO);
        self.apply_object_update(crew_id, update)?;
        Ok(true)
    }

    pub fn set_crew_role(
        &mut self,
        owner: i32,
        object_id: ObjectId,
        role: CrewRole,
    ) -> Result<(), EngineError> {
        if role.as_str().trim().is_empty() {
            return Err(EngineError::CrewRole {
                owner,
                detail: "role name must not be empty".to_string(),
            });
        }

        let object = self
            .objects
            .iter()
            .find(|object| object.id == object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        let valid_member = self
            .players
            .get(&owner)
            .map(|player| player.crew().contains(&object_id))
            .unwrap_or(object.state.owner == owner && object.state.crew_member);
        if !valid_member {
            return Err(EngineError::CrewRole {
                owner,
                detail: if object.state.owner != owner {
                    format!("object {} is owned by {}", object_id, object.state.owner)
                } else {
                    format!("object {} is not in this player's crew", object_id)
                },
            });
        }
        if object.destroyed || object.state.status == ObjectStatus::Deleted {
            return Err(EngineError::CrewRole {
                owner,
                detail: format!("object {} no longer exists", object_id),
            });
        }

        self.crew_roles
            .entry(owner)
            .or_default()
            .insert(object_id, role);
        Ok(())
    }

    pub fn crew_role(&self, owner: i32, object_id: ObjectId) -> Option<&CrewRole> {
        self.crew_roles
            .get(&owner)
            .and_then(|roles| roles.get(&object_id))
    }

    pub fn crew_role_assignments(&self, owner: i32) -> HashMap<ObjectId, CrewRole> {
        self.crew_roles.get(&owner).cloned().unwrap_or_default()
    }

    pub fn clear_crew_role(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(assignments) = self.crew_roles.get_mut(&owner) {
            assignments.remove(&object_id);
            if assignments.is_empty() {
                self.crew_roles.remove(&owner);
            }
        }
    }

    pub fn clear_roles_for_owner(&mut self, owner: i32) {
        self.crew_roles.remove(&owner);
    }

    pub fn apply_command(
        &mut self,
        owner: i32,
        target: CrewCommandTarget,
        update: ObjectUpdate,
    ) -> Result<(), EngineError> {
        self.prune_roles();
        let mut recipients = self.resolve_command_targets(owner, &target);
        if recipients.is_empty() {
            return Ok(());
        }

        let mut seen = HashSet::new();
        recipients.retain(|id| seen.insert(*id));
        if recipients.len() > 1 {
            let ordering: HashMap<_, _> = self
                .objects
                .iter()
                .enumerate()
                .map(|(index, object)| (object.id, index))
                .collect();
            recipients.sort_by_key(|id| ordering.get(id).copied().unwrap_or(usize::MAX));
        }
        for object_id in recipients {
            self.apply_object_update(object_id, update.clone())?;
        }
        Ok(())
    }
}
