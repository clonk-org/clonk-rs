//! The `ObjectCom*` per-procedure helpers (C4ObjectCom.cpp) and the
//! object commands they submit.

use super::*;

impl Engine {
    /// The `PSF_ContainedControlUpdate` (`~ContainedUpdate`) notification for
    /// Jump'n'Run control (C4Script.h:74; C4Object.cpp:3256-3262,3300-3304).
    pub(in crate::direct_com) fn contained_control_update(
        &mut self,
        index: usize,
        com: u8,
        controller: i32,
    ) -> Result<(), EngineError> {
        if com & (COM_SINGLE | COM_DOUBLE) != 0 {
            return Ok(());
        }
        let Some(player) = self.players.get(&controller) else {
            return Ok(());
        };
        if !player.control.control_style {
            return Ok(());
        }
        let pressed = player.control.pressed_coms;
        let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        else {
            return Ok(());
        };
        let clonk_ref = compat::object_reference_value(self.objects[index].id);
        let args = [
            clonk_ref,
            Value::Int(coms_to_com_dir(pressed).to_script_value()),
            Value::Bool(pressed & (1 << COM_DIG) != 0),
            Value::Bool(pressed & (1 << COM_THROW) != 0),
        ];
        self.contained_call(container_index, "ContainedUpdate", &args)?;
        Ok(())
    }

    /// `C4Object::CallControl` (C4Object.cpp:3307-3325): the `Control{Com}`
    /// script override, C4Value-truthy, plus the Jump'n'Run ControlUpdate
    /// notification.
    pub(in crate::direct_com) fn object_call_control(
        &mut self,
        index: usize,
        controller: i32,
        com: u8,
        clonk_arg: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        let function = format!("Control{}", com_name_raw(com));
        let args: Vec<Value> = clonk_arg
            .map(|id| vec![compat::object_reference_value(id)])
            .into_iter()
            .flatten()
            .collect();
        let value = self.contained_call(index, &function, &args)?;
        let result = compat::value_raw_truthy(&value);
        // ControlUpdate for Jump'n'Run control (:3313-3323).
        let (control_style, pressed) = self
            .players
            .get(&controller)
            .map(|player| (player.control.control_style, player.control.pressed_coms))
            .unwrap_or((false, 0));
        if control_style {
            let first = clonk_arg
                .map(compat::object_reference_value)
                .unwrap_or_else(|| compat::object_reference_value(self.objects[index].id));
            let args = [
                first,
                Value::Int(coms_to_com_dir(pressed).to_script_value()),
                Value::Bool(pressed & (1 << COM_DIG) != 0),
                Value::Bool(pressed & (1 << COM_THROW) != 0),
                Value::Bool(pressed & (1 << COM_SPECIAL) != 0),
                Value::Bool(pressed & (1 << COM_SPECIAL2) != 0),
            ];
            self.contained_call(index, "ControlUpdate", &args)?;
        }
        Ok(result)
    }

    /// Fail-safe object script call used by the control chain: script
    /// errors log and the tick continues (C4AulExec fail-safe execution,
    /// C4AulExec.cpp:1318-1342). Missing functions return Nil like `Call`
    /// with the `~` prefix.
    pub(in crate::direct_com) fn contained_call(
        &mut self,
        index: usize,
        function: &str,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        if self.objects[index].destroyed
            || self.objects[index].state.status == crate::ObjectStatus::Deleted
        {
            return Ok(Value::Nil);
        }
        self.contained_call_unchecked(index, function, args)
    }

    pub(in crate::direct_com) fn contained_call_unchecked(
        &mut self,
        index: usize,
        function: &str,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        let Some(definition) = self.definitions.get(&definition_id) else {
            return Ok(Value::Nil);
        };
        // C4Object::Call receives an already linked C4AulFunc pointer. A
        // missing failsafe callback returns nil before C4AulExec allocates a
        // context (C4AulExec.cpp:1318-1342; C4ObjectCom.cpp:48-61).
        if !definition.script.has_function(function) {
            return Ok(Value::Nil);
        }
        let library = definition.shared_action_library_handle();
        let object_id = self.objects[index].id;
        Ok(tolerate_script_error(self.call_movement_object_function(
            index,
            function,
            args,
            &library,
            object_id,
            &definition_id,
        ))?
        .unwrap_or(Value::Nil))
    }

    /// `Contained{Com}` is invoked through the `C4AulFunc *sf` captured by
    /// C4Object::ContainedControl, not through C4Object::Call. Preserve that
    /// exact function and raw receiver; ContainedUpdate and ordinary
    /// Control* calls use `contained_call` above and retain their Status gate
    /// (C4Object.cpp:3237-3255,3297-3302; C4AulExec.cpp:1610-1625).
    pub(in crate::direct_com) fn contained_direct_callback(
        &mut self,
        index: Option<usize>,
        definition_id: &DefinitionId,
        callback: &ScriptCallbackTarget,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        let result = match index {
            Some(index) => self.call_direct_object_callback_from_definition(
                index,
                definition_id,
                callback,
                args.to_vec(),
            ),
            None => self.call_direct_definition_callback(definition_id, callback, args.to_vec()),
        };
        Ok(tolerate_script_error(result)?.unwrap_or(Value::Nil))
    }

    pub(in crate::direct_com) fn object_script_callback(
        &self,
        index: usize,
        function: &str,
    ) -> Option<ScriptCallbackTarget> {
        let definition = self
            .definitions
            .get(&self.objects.get(index)?.definition_id)?;
        let resolution = definition.script.resolve_function(function, false)?;
        Some(ScriptCallbackTarget::linked(function, resolution))
    }

    pub(in crate::direct_com) fn object_has_function(&self, index: usize, function: &str) -> bool {
        self.definitions
            .get(&self.objects[index].definition_id)
            .map(|definition| definition.script.has_function(function))
            .unwrap_or(false)
    }

    /// `DrawCommandQuery`'s function-presence and `Method=` filter
    /// (C4ScriptHost.cpp:95-118; C4Object.cpp:2938-2951). C4Aul functions
    /// default to `All`; an unknown Method value also falls back to `All`
    /// (C4AulLink.cpp:200; C4AulParse.cpp:355-367).
    pub(in crate::direct_com) fn object_control_command_is_visible(
        &self,
        index: usize,
        controller: i32,
        function: &str,
    ) -> bool {
        let Some(control_style) = self
            .players
            .get(&controller)
            .map(|player| player.control.control_style)
        else {
            return false;
        };
        let Some(function) = self
            .definitions
            .get(&self.objects[index].definition_id)
            .and_then(|definition| definition.script.functions().get(function))
        else {
            return false;
        };
        let method = function.description.as_deref().and_then(|description| {
            description.split('|').find_map(|segment| {
                let (key, value) = segment.split_once('=')?;
                key.trim()
                    .eq_ignore_ascii_case("Method")
                    .then(|| value.trim())
            })
        });
        match method {
            Some(method) if method.eq_ignore_ascii_case("None") => false,
            Some(method) if method.eq_ignore_ascii_case("Classic") => !control_style,
            Some(method) if method.eq_ignore_ascii_case("JumpAndRun") => control_style,
            _ => true,
        }
    }

    pub(in crate::direct_com) fn object_procedure(&self, index: usize) -> ActionProcedure {
        let Some(definition) = self.definitions.get(&self.objects[index].definition_id) else {
            return ActionProcedure::Undefined;
        };
        let library = definition.action_library();
        let action = &self.objects[index].state.action;
        if library.is_idle_state(action) {
            return ActionProcedure::Undefined;
        }
        library.procedure_for_entry(&action.name, action.act_map_index)
    }

    // ---- Contents shifting (C4Object.cpp:5751-5797) -----------------------

    /// `C4Object::ShiftContents` (C4Object.cpp:5751-5775): walk First->Next
    /// (or Last->Prev with `shift_back`) for the first present item the
    /// current front cannot concat-picture with, using the full definition,
    /// color, graphics, name, and overlay rules; select it via
    /// DirectComContents.
    pub(in crate::direct_com) fn object_shift_contents(
        &mut self,
        index: usize,
        shift_back: bool,
        do_calls: bool,
    ) -> Result<bool, EngineError> {
        let contents = self.objects[index].state.contents.clone();
        let present_contents: Vec<ObjectId> = contents
            .into_iter()
            .filter(|candidate_id| {
                self.find_object_index(*candidate_id)
                    .is_some_and(|candidate| self.objects[candidate].has_nonzero_status())
            })
            .collect();
        let Some(front_id) = present_contents.first().copied() else {
            return Ok(false);
        };
        let Some(front) = self.object_snapshot(front_id) else {
            return Ok(false);
        };
        let mut candidates: Vec<ObjectId> = present_contents[1..].to_vec();
        if shift_back {
            candidates.reverse();
        }
        for candidate_id in candidates {
            let Some(candidate) = self.object_snapshot(candidate_id) else {
                continue;
            };
            if !self.can_concat_picture_with(&front, &candidate) {
                // Object different: shift to this (C4Object.cpp:5768).
                self.object_direct_com_contents(index, candidate_id, do_calls)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `C4Object::DirectComContents` (C4Object.cpp:5777-5797): the
    /// ~ControlContents veto, the cyclic rotation to the front, and the
    /// ~Selection callback whose falsy return plays the Grab sound. The
    /// context-menu refill (:5792-5795) is app-side presentation.
    pub(in crate::direct_com) fn object_direct_com_contents(
        &mut self,
        index: usize,
        target_id: ObjectId,
        do_calls: bool,
    ) -> Result<(), EngineError> {
        // Safety: present and contained in this object (:5780). Both
        // Status=1 (Normal) and Status=2 (Inactive) are truthy in C++.
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(());
        };
        if !self.objects[target_index].has_nonzero_status()
            || self.objects[target_index].state.container != Some(self.objects[index].id)
        {
            return Ok(());
        }
        // Desired object already at front? (:5782)
        let front = self.objects[index]
            .state
            .contents
            .iter()
            .copied()
            .find(|candidate_id| {
                self.find_object_index(*candidate_id)
                    .is_some_and(|candidate| self.objects[candidate].has_nonzero_status())
            });
        if front == Some(target_id) {
            return Ok(());
        }
        // Select object via script? (:5784-5786)
        let target_definition = self.objects[target_index].definition_id.clone();
        if do_calls {
            let veto = self.contained_call(
                index,
                "ControlContents",
                &[Value::C4Id(target_definition.as_str().to_string())],
            )?;
            if compat::value_raw_truthy(&veto) {
                return Ok(());
            }
        }
        // Default action: the cyclic relink (C4ObjectList::ShiftContents,
        // C4ObjectList.cpp:815-833) — a no-op if the id left the list.
        let contents = &mut self.objects[index].state.contents;
        let Some(position) = contents.iter().position(|id| *id == target_id) else {
            return Ok(());
        };
        contents.rotate_left(position);
        // Selection sound (:5790): falsy ~Selection(container) on the new
        // front plays "Grab" at the container.
        if do_calls {
            let container_ref = compat::object_reference_value(self.objects[index].id);
            let selected = self.contained_call(target_index, "Selection", &[container_ref])?;
            if !compat::value_raw_truthy(&selected) {
                let container_id = self.objects[index].id;
                self.emit_audio_command(crate::AudioCommand::PlaySound {
                    name: "Grab".to_string(),
                    target: Some(container_id),
                    volume: 100,
                    looped: false,
                    multiple: false,
                    custom_falloff: None,
                    target_position: None,
                });
            }
        }
        Ok(())
    }

    // ---- ObjectCom* helpers (C4ObjectCom.cpp) -----------------------------

    /// `ObjectComMovement` (C4ObjectCom.cpp:220-237).
    pub(in crate::direct_com) fn object_com_movement(
        &mut self,
        index: usize,
        com_dir: CommandDirection,
    ) -> Result<(), EngineError> {
        self.objects[index].state.command_direction = com_dir;
        let owner = self.objects[index].state.owner;
        let self_id = self.objects[index].id;
        // Selected crew follows the moving cursor (:224).
        self.player_object_command(owner, CommandId::Follow, Some(self_id), 0, 0)?;
        // Direct turnaround if standing still (:226-235).
        let procedure = self.object_procedure(index);
        if self.objects[index].fixed_velocity.x.val() == 0
            && matches!(procedure, ActionProcedure::Walk | ActionProcedure::Hang)
        {
            // Native calls `cObj->SetDir(...)` here, not a bare assignment:
            // SetDir runs the current action's TurnAction through
            // SetActionByName before writing the facing, and rejects idle or
            // out-of-range directions first (C4Object.cpp:4237-4253). Going
            // through the trailing assignment alone left the object in its old
            // action (clonk-org/clonk-rs#1124).
            let turn = match com_dir {
                CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
                    Some(Direction::Left)
                }
                CommandDirection::Right
                | CommandDirection::UpRight
                | CommandDirection::DownRight => Some(Direction::Right),
                _ => None,
            };
            if let Some(direction) = turn {
                let definition_id = self.objects[index].definition_id.clone();
                self.set_exec_action_direction(index, &definition_id, direction)?;
            }
        }
        Ok(())
    }

    /// `ObjectComStop` (C4ObjectCom.cpp:239-245): cease action, then stand.
    pub(in crate::direct_com) fn object_com_stop(
        &mut self,
        index: usize,
    ) -> Result<bool, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        self.object_com_stop_action(index, &definition_id)
    }

    /// C4Command::Grab's direct ObjectComStop call. Unlike the older shared
    /// engine helper, C++ uses ordinary SetActionByName transitions here,
    /// so a current NoOtherAction may block Idle and Walk.
    pub(in crate::direct_com) fn object_com_stop_for_grab(
        &mut self,
        index: usize,
    ) -> Result<bool, EngineError> {
        let actor_id = self.objects[index].id;
        let definition_id = self.objects[index].definition_id.clone();
        let _ = self.action_with_calls(index, &definition_id, "Idle")?;

        let Some(index) = self.find_object_index(actor_id) else {
            return Ok(false);
        };
        self.objects[index].state.command_direction = CommandDirection::Stop;
        let definition_id = self.objects[index].definition_id.clone();
        if !self.action_with_calls(index, &definition_id, "Walk")? {
            return Ok(false);
        }
        if let Some(index) = self.find_object_index(actor_id) {
            let object = &mut self.objects[index];
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
        }
        Ok(true)
    }

    /// `ObjectComUp` (C4ObjectCom.cpp:335-351): entrance first, then jump.
    pub(in crate::direct_com) fn object_com_up(
        &mut self,
        index: usize,
    ) -> Result<bool, EngineError> {
        let position = self.objects[index].state.position;
        let self_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        if let Some((_, target_id, target_ocf)) =
            self.at_object(position, ocf::ENTRANCE, Some(self_id))
        {
            if target_ocf & ocf::ENTRANCE != 0 {
                return self.player_object_command(owner, CommandId::Enter, Some(target_id), 0, 0);
            }
        }
        if self.object_procedure(index) == ActionProcedure::Walk {
            return self.player_object_command(owner, CommandId::Jump, None, 0, 0);
        }
        Ok(false)
    }

    /// `ObjectComDig` (C4ObjectCom.cpp:353-362): CanDig gate + Dig action,
    /// with the native localized object message on either failure path.
    pub(crate) fn object_com_dig(&mut self, index: usize) -> Result<bool, EngineError> {
        let actor_id = self.objects[index].id;
        let physical = self.object_physical(index);
        let definition_id = self.objects[index].definition_id.clone();
        if physical.can_dig == 0 || !self.action_with_calls(index, &definition_id, "Dig")? {
            let name = self.object_message_name(actor_id);
            let text = self.object_no_dig_resource_string.replacen("%s", &name, 1);
            self.game_msg_object(actor_id, text);
            return Ok(false);
        }
        // ObjectActionDig resets the Dig2Object request (:143).
        self.objects[index].state.action.data = 0;
        Ok(true)
    }

    /// First nonzero-Status entry returned by `Contents.GetObject()`.
    pub(in crate::direct_com) fn first_live_content_id(&self, index: usize) -> Option<ObjectId> {
        self.objects[index]
            .state
            .contents
            .iter()
            .copied()
            .find(|object_id| {
                self.find_object_index(*object_id).is_some_and(|index| {
                    !self.objects[index].destroyed
                        && self.objects[index].state.status != crate::ObjectStatus::Deleted
                })
            })
    }

    pub(in crate::direct_com) fn dig_double_physical_backing(
        &mut self,
        index: usize,
    ) -> DigDoublePhysicalBacking {
        let object_id = self.objects[index].id;
        if self.objects[index].state.temporary_physical.is_some() {
            DigDoublePhysicalBacking::Temporary
        } else if self.crew_object_infos.contains_key(&object_id)
            || self.objects[index].state.info_physical.is_some()
            || self.objects[index].state.crew_member
        {
            let linked_info = self.crew_object_infos.contains_key(&object_id);
            let physical = if linked_info {
                self.object_physical(index)
            } else {
                self.objects[index]
                    .state
                    .info_physical
                    .or_else(|| {
                        self.definitions
                            .get(&self.objects[index].definition_id)
                            .map(|definition| *definition.physical())
                    })
                    .unwrap_or_default()
            };
            if linked_info && self.use_fair_crew() {
                DigDoublePhysicalBacking::FairCrew(physical)
            } else {
                DigDoublePhysicalBacking::Info(physical)
            }
        } else {
            DigDoublePhysicalBacking::Definition(self.objects[index].definition_id.clone())
        }
    }

    pub(in crate::direct_com) fn physical_from_dig_double_backing(
        &self,
        index: usize,
        backing: &DigDoublePhysicalBacking,
    ) -> PhysicalInfo {
        match backing {
            DigDoublePhysicalBacking::Temporary => self.objects[index]
                .state
                .temporary_physical
                .unwrap_or_default(),
            DigDoublePhysicalBacking::FairCrew(physical) => *physical,
            DigDoublePhysicalBacking::Info(initial) => self.objects[index]
                .state
                .info_physical
                .or(self.objects[index].retired_info_physical)
                .unwrap_or(*initial),
            DigDoublePhysicalBacking::Definition(definition_id) => self
                .definitions
                .get(definition_id)
                .map(|definition| *definition.physical())
                .unwrap_or_default(),
        }
    }

    /// `ObjectComDigDouble` (C4ObjectCom.cpp:531-571) — "activation":
    /// contents Activate, linekit construction, chop, line pickup, then own
    /// Activate.
    pub(in crate::direct_com) fn object_com_dig_double(
        &mut self,
        index: usize,
    ) -> Result<(), EngineError> {
        let self_id = self.objects[index].id;
        let physical_backing = self.dig_double_physical_backing(index);
        // Contents activation — first contents object only (:537-539).
        if let Some(contents_id) = self.first_live_content_id(index) {
            if let Some(contents_index) = self.find_object_index(contents_id) {
                let clonk_ref = compat::object_reference_value(self_id);
                let value = self.contained_call(contents_index, "Activate", &[clonk_ref])?;
                if compat::value_raw_truthy(&value) {
                    return Ok(());
                }
            }
        }

        let Some(index) = self.find_object_index(self_id) else {
            return Ok(());
        };
        // Re-read the first content after Activate. A leading LNKT always
        // consumes DigDouble even when line construction fails (:542-547).
        let first_contents = self.first_live_content_id(index);
        if first_contents.is_some_and(|contents_id| {
            self.find_object_index(contents_id)
                .is_some_and(|contents_index| self.objects[contents_index].definition_id == "LNKT")
        }) {
            let _ = self.object_com_line_construction(index)?;
            return Ok(());
        }

        // Chop (:549-558).
        let physical = self.physical_from_dig_double_backing(index, &physical_backing);
        if physical.can_chop != 0 && self.object_procedure(index) != ActionProcedure::Swim {
            let position = self.objects[index].state.position;
            if let Some((_, target_id, target_ocf)) =
                self.at_object(position, ocf::CHOP, Some(self_id))
            {
                if target_ocf & ocf::CHOP != 0 {
                    let owner = self.objects[index].state.owner;
                    self.player_object_command(owner, CommandId::Chop, Some(target_id), 0, 0)?;
                    return Ok(());
                }
            }
        }

        // Empty-hand line pickup follows Chop and has an outer physical/
        // structure precheck before the helper repeats its live gate
        // (C4ObjectCom.cpp:559-567).
        if self
            .physical_from_dig_double_backing(index, &physical_backing)
            .can_construct
            != 0
            && self.first_live_content_id(index).is_none()
        {
            let position = self.objects[index].state.position;
            if self
                .at_object(position, ocf::LINE_CONSTRUCT, Some(self_id))
                .is_some_and(|(_, _, object_ocf)| object_ocf & ocf::LINE_CONSTRUCT != 0)
                && self.object_com_line_construction(index)?
            {
                return Ok(());
            }
        }

        // Own activation call (:569-570).
        let self_ref = compat::object_reference_value(self_id);
        if let Some(index) = self.find_object_index(self_id) {
            self.contained_call(index, "Activate", &[self_ref])?;
        }
        Ok(())
    }

    /// First C++ master-list object whose live `Connect` action targets the
    /// supplied endpoint (`C4Game::FindObject`, C4Game.cpp:1391-1419).
    pub(in crate::direct_com) fn find_connect_line_index(
        &self,
        target: ObjectId,
        definition_id: Option<&str>,
    ) -> Option<usize> {
        self.execution.exec_list.iter().rev().find_map(|object_id| {
            let index = self.find_object_index(*object_id)?;
            let object = &self.objects[index];
            (!object.destroyed
                && object.state.status.is_active()
                && self.object_ocf_at_index(index) != 0
                && definition_id.is_none_or(|id| object.definition_id == id)
                && object.state.action.name == "Connect"
                && (object.state.action.target == Some(target)
                    || object.state.action.target2 == Some(target)))
            .then_some(index)
        })
    }

    pub(in crate::direct_com) fn play_line_construction_sound(
        &mut self,
        name: &str,
        clonk_id: ObjectId,
    ) {
        self.emit_audio_command(crate::AudioCommand::PlaySound {
            name: name.to_owned(),
            target: Some(clonk_id),
            volume: 100,
            looped: false,
            multiple: false,
            custom_falloff: None,
            target_position: None,
        });
    }

    pub(crate) fn object_message_name(&self, object_id: ObjectId) -> String {
        self.find_object_index(object_id)
            .map(|index| &self.objects[index])
            .and_then(|object| {
                object
                    .state
                    .custom_name
                    .clone()
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        self.crew_object_infos
                            .get(&object_id)
                            .map(|info| info.name.clone())
                    })
                    .or_else(|| {
                        self.definitions
                            .get(&object.definition_id)
                            .map(|definition| definition.name().to_owned())
                    })
            })
            .unwrap_or_default()
    }

    /// `GameMsgObject` after its caller resolves the active `LoadResStr` text.
    /// Ordering and target replacement are simulation-visible.
    pub(in crate::direct_com) fn game_msg_object(&mut self, target: ObjectId, text: String) {
        // C4GameMessageList::New replaces prior messages before its deleted
        // target guard, so a failed GameMsgObject still performs the clear.
        self.messages.clear_for_object(target);
        let target_live = self.find_object_index(target).is_some_and(|index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        });
        if !target_live {
            return;
        }
        self.messages.add_message(MessageSpec {
            kind: message::MessageKind::Target,
            text,
            target: Some(target),
            player: None,
            offset: Vector2::ZERO,
            color: 0xffff_ffff,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        });
    }

    /// `ObjectComLineConstruction` (C4ObjectCom.cpp:379-529): stand and
    /// physical gate, pickup without a kit, finish an attached line, or
    /// create a new one.
    pub(in crate::direct_com) fn object_com_line_construction(
        &mut self,
        clonk_index: usize,
    ) -> Result<bool, EngineError> {
        // ObjectComLineConstruction enters Stand even when the following
        // physical gate rejects construction (C4ObjectCom.cpp:384-390).
        let clonk_id = self.objects[clonk_index].id;
        let clonk_definition = self.objects[clonk_index].definition_id.clone();
        self.objects[clonk_index].state.command_direction = CommandDirection::Stop;
        if self.action_with_calls(clonk_index, &clonk_definition, "Walk")? {
            if let Some(clonk_index) = self.find_object_index(clonk_id) {
                let clonk = &mut self.objects[clonk_index];
                clonk.fixed_velocity = FixedVec2::ZERO;
                clonk.state.velocity = Vector2::ZERO;
            }
        }
        let Some(clonk_index) = self.find_object_index(clonk_id) else {
            return Ok(false);
        };
        if self.object_physical(clonk_index).can_construct == 0 {
            let clonk_name = self.object_message_name(clonk_id);
            self.game_msg_object(clonk_id, format!("{clonk_name} cannot create lines."));
            return Ok(false);
        }

        let position = self.objects[clonk_index].state.position;
        let linekit_id = self.objects[clonk_index]
            .state
            .contents
            .iter()
            .copied()
            .find(|linekit_id| {
                self.find_object_index(*linekit_id).is_some_and(|index| {
                    let linekit = &self.objects[index];
                    !linekit.destroyed
                        && linekit.state.status != crate::ObjectStatus::Deleted
                        && self.object_ocf_at_index(index) != 0
                        && linekit.definition_id == "LNKT"
                })
            });

        // Line pickup (:392-427).
        let Some(linekit_id) = linekit_id else {
            let collection_limit = self
                .definitions
                .get(&self.objects[clonk_index].definition_id)
                .map_or(0, crate::Definition::collection_limit);
            let contents_count = self.objects[clonk_index]
                .state
                .contents
                .iter()
                .filter(|object_id| {
                    self.find_object_index(**object_id).is_some_and(|index| {
                        !self.objects[index].destroyed
                            && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    })
                })
                .count();
            if crate::collection_limit_reached(collection_limit, contents_count) {
                return Ok(false);
            }

            let Some((_, structure_id, structure_ocf)) =
                self.at_object(position, ocf::LINE_CONSTRUCT, Some(clonk_id))
            else {
                return Ok(false);
            };
            if structure_ocf & ocf::LINE_CONSTRUCT == 0 {
                return Ok(false);
            }
            let Some(line_index) = self.find_connect_line_index(structure_id, None) else {
                return Ok(false);
            };
            let first = self.objects[line_index].state.action.target;
            let second = self.objects[line_index].state.action.target2;
            let endpoint_is_linekit = |engine: &Self, endpoint: Option<ObjectId>| {
                endpoint
                    .and_then(|endpoint| engine.find_object_index(endpoint))
                    .is_some_and(|index| engine.objects[index].definition_id == "LNKT")
            };
            if endpoint_is_linekit(self, first) || endpoint_is_linekit(self, second) {
                self.play_line_construction_sound("Error", clonk_id);
                let line_name = self.object_message_name(self.objects[line_index].id);
                self.game_msg_object(
                    clonk_id,
                    format!("{line_name} is not fixed at the other end."),
                );
                return Ok(false);
            }
            if !self.definitions.contains_key("LNKT") {
                return Ok(false);
            }

            let line_id = self.objects[line_index].id;
            let line_owner = self.objects[line_index].state.owner;
            let clonk_layer = self.objects[clonk_index].state.layer;
            let mut linekit_config = crate::SpawnConfig::new("LNKT")
                .with_position(Vector2::new(50, 50))
                .with_owner(line_owner);
            if let Some(layer) = clonk_layer {
                linekit_config = linekit_config.with_layer(layer);
            }
            let linekit_id =
                self.spawn_object_with_initial_lifecycle(linekit_config, Some(clonk_id))?;
            let Some(linekit_id) = linekit_id else {
                return Ok(false);
            };
            if self.try_object_enter_with_reject_collect(linekit_id, clonk_id, true)?
                != ObjectEnterOutcome::Entered
            {
                let _ = self.assign_object_removal(linekit_id)?;
                return Ok(false);
            }

            self.play_line_construction_sound("Connect", clonk_id);
            if let Some(line_index) = self.find_object_index(line_id) {
                if self.objects[line_index].state.action.target == Some(structure_id) {
                    self.objects[line_index].state.action.target = Some(linekit_id);
                }
                if self.objects[line_index].state.action.target2 == Some(structure_id) {
                    self.objects[line_index].state.action.target2 = Some(linekit_id);
                }
            }
            let line_name = self.object_message_name(line_id);
            let structure_name = self.object_message_name(structure_id);
            self.game_msg_object(
                structure_id,
                format!("{line_name} disconnected|from {structure_name}."),
            );
            return Ok(true);
        };
        let Some(linekit_index) = self.find_object_index(linekit_id) else {
            return Ok(false);
        };

        let active_line = self.find_connect_line_index(linekit_id, None);

        let Some((structure_index, structure_id, structure_ocf)) =
            self.at_object(position, ocf::LINE_CONSTRUCT, Some(clonk_id))
        else {
            self.play_line_construction_sound("Error", clonk_id);
            self.game_msg_object(
                clonk_id,
                if active_line.is_some() {
                    "Connection not possible.".to_owned()
                } else {
                    "Cannot create a new line here.".to_owned()
                },
            );
            return Ok(false);
        };
        if structure_ocf & ocf::LINE_CONSTRUCT == 0 {
            self.play_line_construction_sound("Error", clonk_id);
            self.game_msg_object(
                clonk_id,
                if active_line.is_some() {
                    "Connection not possible.".to_owned()
                } else {
                    "Cannot create a new line here.".to_owned()
                },
            );
            return Ok(false);
        }

        if let Some(line_index) = active_line {
            let first = self.objects[line_index].state.action.target;
            let second = self.objects[line_index].state.action.target2;
            if first == Some(structure_id) || second == Some(structure_id) {
                self.play_line_construction_sound("Connect", clonk_id);
                let line_id = self.objects[line_index].id;
                let line_name = self.object_message_name(line_id);
                self.game_msg_object(structure_id, format!("{line_name} disconnected."));
                let _ = self.assign_object_removal(line_id)?;
                return Ok(true);
            }

            let line_type = self
                .definitions
                .get(&self.objects[line_index].definition_id)
                .map(|definition| definition.line())
                .unwrap_or_default();
            let line_connect = self
                .definitions
                .get(&self.objects[structure_index].definition_id)
                .map(|definition| definition.line_connect())
                .unwrap_or_default();
            let connect_ok = match line_type {
                1 => {
                    line_connect
                        & (crate::LINE_CONNECT_POWER_INPUT | crate::LINE_CONNECT_POWER_OUTPUT)
                        != 0
                }
                2 => line_connect & crate::LINE_CONNECT_LIQUID_OUTPUT != 0,
                3 => line_connect & crate::LINE_CONNECT_LIQUID_INPUT != 0,
                _ => return Ok(false),
            };
            if !connect_ok {
                self.play_line_construction_sound("Error", clonk_id);
                let line_name = self.object_message_name(self.objects[line_index].id);
                let structure_name = self.object_message_name(structure_id);
                self.game_msg_object(
                    structure_id,
                    format!("{line_name} cannot be connected|to {structure_name}."),
                );
                return Ok(false);
            }

            self.play_line_construction_sound("Connect", clonk_id);
            if first == Some(linekit_id) {
                self.objects[line_index].state.action.target = Some(structure_id);
            }
            if second == Some(linekit_id) {
                self.objects[line_index].state.action.target2 = Some(structure_id);
            }
            // Bare Exit() uses the default zero position/motion. Its return
            // is ignored; AssignRemoval still follows even if a callback
            // re-enters the kit (C4ObjectCom.cpp:479-480).
            if let Some(previous) = self.objects[linekit_index].state.container {
                let _ = self.exit_object_at_position_with_zero_motion(
                    linekit_id,
                    previous,
                    Vector2::ZERO,
                    0,
                )?;
            }
            let _ = self.assign_object_removal(linekit_id)?;
            let line_name = self.object_message_name(self.objects[line_index].id);
            let structure_name = self.object_message_name(structure_id);
            self.game_msg_object(
                structure_id,
                format!("{line_name} conntected|to {structure_name}"),
            );
            return Ok(true);
        }

        let line_connect = self
            .definitions
            .get(&self.objects[structure_index].definition_id)
            .map(|definition| definition.line_connect())
            .unwrap_or_default();
        let has_connected_line = |engine: &Self, definition_id: &str| {
            engine
                .find_connect_line_index(structure_id, Some(definition_id))
                .is_some()
        };
        let line_definition = if line_connect & crate::LINE_CONNECT_LIQUID_PUMP != 0
            && !has_connected_line(self, "SPIP")
        {
            Some("SPIP")
        } else if line_connect & crate::LINE_CONNECT_LIQUID_OUTPUT != 0
            && !has_connected_line(self, "DPIP")
        {
            Some("DPIP")
        } else if line_connect & crate::LINE_CONNECT_POWER_OUTPUT != 0 {
            Some("PWRL")
        } else {
            None
        };
        let Some(line_definition) = line_definition else {
            self.play_line_construction_sound("Error", clonk_id);
            self.game_msg_object(clonk_id, "Cannot create a new line here.".to_owned());
            return Ok(false);
        };
        let owner = self.objects[clonk_index].state.owner;
        let created = self.create_line_object(line_definition, owner, structure_id, linekit_id)?;
        if let Some(line_id) = created {
            self.play_line_construction_sound("Connect", clonk_id);
            let line_name = self.object_message_name(line_id);
            self.game_msg_object(structure_id, format!("New|{line_name}."));
        }
        Ok(created.is_some())
    }

    /// `ObjectComDownDouble` (C4ObjectCom.cpp:573-589): build or grab what
    /// is at the object's position.
    pub(in crate::direct_com) fn object_com_down_double(
        &mut self,
        index: usize,
    ) -> Result<bool, EngineError> {
        let position = self.objects[index].state.position;
        let self_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        if let Some((_, target_id, target_ocf)) =
            self.at_object(position, ocf::CONSTRUCT | ocf::GRAB, Some(self_id))
        {
            if target_ocf & ocf::CONSTRUCT != 0 {
                self.player_object_command(owner, CommandId::Build, Some(target_id), 0, 0)?;
                return Ok(true);
            }
            if target_ocf & ocf::GRAB != 0 {
                self.player_object_command(owner, CommandId::Grab, Some(target_id), 0, 0)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ObjectComLetGo` (C4ObjectCom.cpp:310-314): jump off a wall/ceiling.
    pub(in crate::direct_com) fn object_com_let_go(
        &mut self,
        index: usize,
        xdirf: i32,
    ) -> Result<bool, EngineError> {
        self.object_action_jump(index, itofix(xdirf), crate::C4Fixed::from_raw(0), true)
    }

    /// `ObjectComGrab` (C4ObjectCom.cpp:247-259): ordinary, non-forced Push
    /// followed by the two ordered script notifications and the live
    /// controller hand-off between them.
    pub(in crate::direct_com) fn object_com_grab(
        &mut self,
        actor_id: ObjectId,
        target_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(false);
        };
        if self.object_procedure(actor_index) != ActionProcedure::Walk {
            return Ok(false);
        }
        let definition_id = self.objects[actor_index].definition_id.clone();
        if !self.action_with_target_and_calls(actor_index, &definition_id, "Push", target_id)? {
            return Ok(false);
        }

        // ObjectActionPush's Start/Abort callbacks precede the explicit
        // Grab callback. A removed actor cannot execute the latter.
        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let _ = tolerate_script_error(self.call_object_function(
            actor_index,
            "Grab",
            vec![compat::object_reference_value(target_id), Value::Bool(true)],
        ))?;

        // C++ checks both Status fields only after Grab. The callback may
        // remove either object or change the actor's Controller; propagate
        // the live post-callback value before calling Grabbed.
        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let Some(target_index) = self.find_object_index(target_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let controller = self.objects[actor_index].state.controller;
        self.objects[target_index].state.controller = controller;
        let _ = tolerate_script_error(self.call_object_function(
            target_index,
            "Grabbed",
            vec![compat::object_reference_value(actor_id), Value::Bool(true)],
        ))?;
        Ok(true)
    }

    /// C4Command::Grab's full live sequence (C4Command.cpp:667-716).
    /// ObjectComStop may run callbacks before the At test; ObjectComLetGo
    /// and RejectGrabbed likewise precede ObjectComGrab.
    pub(crate) fn execute_grab_command(
        &mut self,
        actor_id: ObjectId,
        target_id: ObjectId,
    ) -> Result<(), EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(());
        };
        if self.objects[actor_index].destroyed
            || self.objects[actor_index].state.status == crate::ObjectStatus::Deleted
        {
            return Ok(());
        }

        let (offset_x, offset_y) = self.objects[actor_index]
            .commands
            .pending_grab_offsets(target_id)
            .unwrap_or((0, 0));

        let mut stopped_for_grab = false;
        if matches!(
            self.object_procedure(actor_index),
            ActionProcedure::Build | ActionProcedure::Chop
        ) {
            stopped_for_grab = true;
            let _ = self.object_com_stop_for_grab(actor_index)?;
        }

        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(());
        };
        if self.object_procedure(actor_index) == ActionProcedure::Dig {
            stopped_for_grab = true;
            let _ = self.object_com_stop_for_grab(actor_index)?;
        }

        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(());
        };

        // ObjectComStop callbacks can install a Push action. C++ performs
        // this recheck before the null-target and At branches.
        if self.object_procedure(actor_index) == ActionProcedure::Push {
            let _ = self.objects[actor_index]
                .commands
                .resolve_grab_attempt(target_id, false);
            let _ = self.objects[actor_index].commands.push_front(
                CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub),
            );
            return Ok(());
        }

        if stopped_for_grab
            && self.objects[actor_index]
                .commands
                .fail_pending_grab_if_target_cleared(target_id)
        {
            return Ok(());
        }

        let target_at_actor = self
            .find_object_index(target_id)
            .filter(|&index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    && self.objects[index].state.container.is_none()
                    && self.objects[index].state.ocf & ocf::ALL != 0
            })
            .is_some_and(|target_index| {
                self.objects[actor_index].state.container.is_none()
                    && self
                        .object_shape_rect(&self.objects[target_index])
                        .contains_point(
                            self.objects[actor_index].state.position.x,
                            self.objects[actor_index].state.position.y,
                        )
            });

        if !target_at_actor {
            let target_retained = self.objects[actor_index]
                .commands
                .resolve_grab_attempt(target_id, false)
                .unwrap_or(true);
            if target_retained {
                if let Some(target_index) = self.find_object_index(target_id) {
                    let position = self.objects[target_index].state.position;
                    let _ = self.objects[actor_index].commands.push_front(
                        CommandRequest::new(CommandId::MoveTo)
                            .with_tx(Some(position.x.wrapping_add(offset_x)))
                            .with_ty(Some(position.y.wrapping_add(offset_y)))
                            .with_update_interval(50)
                            .with_mode(CommandMode::SilentSub),
                    );
                }
            }
            return Ok(());
        }

        if matches!(
            self.object_procedure(actor_index),
            ActionProcedure::Scale | ActionProcedure::Hang
        ) {
            let xdirf = if self.objects[actor_index].state.direction == Direction::Left {
                1
            } else {
                -1
            };
            let _ = self.object_com_let_go(actor_index, xdirf)?;
        }

        let rejected = match self.find_object_index(target_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) {
            Some(target_index) => tolerate_script_error(self.call_object_function(
                target_index,
                "RejectGrabbed",
                vec![compat::object_reference_value(actor_id)],
            ))?
            .is_some_and(|value| value.as_bool()),
            None => false,
        };

        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(());
        };
        let target_retained = self.objects[actor_index]
            .commands
            .resolve_grab_attempt(target_id, rejected)
            .unwrap_or(true);
        if rejected {
            return Ok(());
        }

        self.objects[actor_index].state.command_direction = CommandDirection::Stop;
        if target_retained {
            let _ = self.object_com_grab(actor_id, target_id)?;
        }
        Ok(())
    }

    /// `C4Command::Jump` followed by `ObjectComJump` (C4Command.cpp:
    /// 1056-1067; C4ObjectCom.cpp:280-307). This stays live because
    /// ObjectActionJump synchronously invokes the object's OnActionJump hook.
    pub(crate) fn execute_jump_command(
        &mut self,
        object_id: ObjectId,
        tx: i32,
    ) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Tx==0 is the C++ sentinel: do not reinterpret it as world x=0.
        if tx != 0 {
            let x = self.objects[index].state.position.x;
            let direction = if tx < x {
                Some(Direction::Left)
            } else if tx > x {
                Some(Direction::Right)
            } else {
                None
            };
            if let Some(direction) = direction {
                let definition_id = self.objects[index].definition_id.clone();
                self.set_command_action_direction(index, &definition_id, direction)?;
            }
        }
        let _ = self.object_com_jump(index)?;
        // C4Command::Jump calls Finish(true) only after ObjectComJump and its
        // synchronous OnActionJump callback return (C4Command.cpp:1064-1067).
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index]
                .commands
                .finish_front_if(CommandId::Jump);
        }
        Ok(())
    }

    /// `ObjectComJump` (C4ObjectCom.cpp:280-307): predict a deep-liquid
    /// landing from the shape's bottom vertex before falling back to the
    /// script-overridable regular jump.
    pub(crate) fn object_com_jump(&mut self, index: usize) -> Result<bool, EngineError> {
        if self.object_procedure(index) != ActionProcedure::Walk {
            return Ok(false);
        }
        // Native GetPhysical may run the lazy fair-crew fill before any of
        // GetCon, Action.ComDir, or Action.Dir are read (:286-294).
        let physical = self.object_physical(index);
        let launch = crate::command::object_com_jump_launch(
            self.objects[index].state.construction,
            physical,
            self.objects[index].state.command_direction,
            self.objects[index].state.direction,
        );
        // ObjectComJump reads pObj->Shape.ContactDensity, not Def->Shape
        // (C4ObjectCom.cpp:297-305). SetContactDensity therefore changes the
        // dive gate independently for every live object.
        let contact_density = self.objects[index].state.contact_density;
        if contact_density > 25
            && self.object_com_jump_hits_liquid(index, launch)
            && self.object_action_dive(index, launch.x, launch.y)?
        {
            return Ok(true);
        }
        self.object_action_jump(index, launch.x, launch.y, true)
    }

    /// `SimFlightHitsLiquid` (C4Movement.cpp:657-670), including the
    /// ten-frame escape when the bottom vertex already starts in water.
    pub(in crate::direct_com) fn object_com_jump_hits_liquid(
        &self,
        index: usize,
        launch: FixedVec2,
    ) -> bool {
        let Some(object) = self.objects.get(index) else {
            return false;
        };
        // Despite the name, C4Shape::GetBottomVertex selects the CNAT_Bottom
        // vertex with the smallest VtxY (C4Shape.cpp:445-455).
        let bottom = object
            .state
            .vertices
            .iter()
            .filter(|vertex| vertex.cnat & crate::CNAT_BOTTOM != 0)
            .min_by_key(|vertex| vertex.y);
        let mut position = object.fixed_position;
        if let Some(bottom) = bottom {
            position.x += bottom.x;
            position.y += bottom.y;
        }
        let mut velocity = launch;
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        let solid_masks = self.live_movement_solid_masks();
        let density_at =
            |x, y| crate::movement_density_at(landscape, &self.materials, &solid_masks, None, x, y);
        let width = landscape.width() as i32;
        let height = landscape.estimated_height();
        let gravity = self.physics.gravity_as_c4fixed();
        let liquid = |density| (25..50).contains(&density);

        if liquid(density_at(
            crate::math::fixtoi(position.x),
            crate::math::fixtoi(position.y),
        )) && !sim_flight_to_density(
            &mut position,
            &mut velocity,
            0,
            24,
            10,
            gravity,
            width,
            height,
            &density_at,
        ) {
            return false;
        }
        if !sim_flight_to_density(
            &mut position,
            &mut velocity,
            25,
            100,
            -1,
            gravity,
            width,
            height,
            &density_at,
        ) {
            return false;
        }
        let x = crate::math::fixtoi(position.x);
        let y = crate::math::fixtoi(position.y);
        liquid(density_at(x, y)) && liquid(density_at(x, y + 9))
    }

    /// `ObjectActionDive` (C4ObjectCom.cpp:63-72): unlike a regular jump,
    /// Dive has no OnActionJump callback.
    pub(in crate::direct_com) fn object_action_dive(
        &mut self,
        index: usize,
        xdir: crate::C4Fixed,
        ydir: crate::C4Fixed,
    ) -> Result<bool, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        if !self.action_with_calls(index, &definition_id, "Dive")? {
            return Ok(false);
        }
        let object = &mut self.objects[index];
        object.fixed_velocity = FixedVec2::new(xdir, ydir);
        object.state.velocity = Vector2::new(crate::math::fixtoi(xdir), crate::math::fixtoi(ydir));
        object.state.mobile = true;
        object.frame_t_attach &= !crate::CNAT_BOTTOM;
        object.state.t_attach &= !crate::CNAT_BOTTOM;
        Ok(true)
    }

    /// `ObjectActionJump` (C4ObjectCom.cpp:48-61): the scripted OnActionJump
    /// override, then the hardcoded Jump action with launch velocity.
    pub(crate) fn object_action_jump(
        &mut self,
        index: usize,
        xdir: crate::C4Fixed,
        ydir: crate::C4Fixed,
        by_com: bool,
    ) -> Result<bool, EngineError> {
        let args = [
            Value::Int(crate::math::fixtoi_prec(xdir, 100)),
            Value::Int(crate::math::fixtoi_prec(ydir, 100)),
            Value::Bool(by_com),
        ];
        let value = self.contained_call(index, "OnActionJump", &args)?;
        if compat::value_raw_truthy(&value) {
            return Ok(true);
        }
        let definition_id = self.objects[index].definition_id.clone();
        if !self.action_with_calls(index, &definition_id, "Jump")? {
            return Ok(false);
        }
        let object = &mut self.objects[index];
        object.fixed_velocity = FixedVec2::new(xdir, ydir);
        object.state.velocity = Vector2::new(crate::math::fixtoi(xdir), crate::math::fixtoi(ydir));
        object.state.mobile = true;
        // Unstick from ground: attach-values were already determined for
        // this frame (:58-59).
        object.frame_t_attach &= !crate::CNAT_BOTTOM;
        object.state.t_attach &= !crate::CNAT_BOTTOM;
        Ok(true)
    }

    /// `ObjectComEnter` for the pushed target (C4ObjectCom.cpp:316-333):
    /// the vehicle enters the entrance at its own position via a plain
    /// SetCommand.
    pub(in crate::direct_com) fn object_com_enter(
        &mut self,
        target_index: Option<usize>,
    ) -> Result<bool, EngineError> {
        let Some(target_index) = target_index else {
            return Ok(false);
        };
        if self
            .definitions
            .get(&self.objects[target_index].definition_id)
            .is_some_and(|definition| definition.no_push_enter() != 0)
        {
            return Ok(false);
        }
        let position = self.objects[target_index].state.position;
        let target_id = self.objects[target_index].id;
        if let Some((_, entrance_id, entrance_ocf)) =
            self.at_object(position, ocf::ENTRANCE, Some(target_id))
        {
            if entrance_ocf & ocf::ENTRANCE != 0 {
                self.set_object_command(
                    target_index,
                    CommandRequest::new(CommandId::Enter)
                        .with_target(Some(entrance_id))
                        .with_mode(CommandMode::Base),
                    false,
                )?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ObjectComDrop` (C4ObjectCom.cpp:640-676): calculate the live shape-
    /// relative exit and fixed launch, run the full Exit callback sequence,
    /// then arm collection delay and release any current Push action.
    pub(crate) fn object_com_drop(
        &mut self,
        actor_id: ObjectId,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(false);
        };
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };

        let throw_force = math::val_by_physical(400, self.object_physical(actor_index).throw);
        let procedure = self.object_procedure(actor_index);
        let command_direction = self.objects[actor_index].state.command_direction;
        let actor_xdir = self.objects[actor_index].fixed_velocity.x;
        let actor_position = self.objects[actor_index].state.position;
        let actor_shape = self.objects[actor_index]
            .current_shape_rect()
            .unwrap_or_default();
        let object_shape = self.objects[object_index]
            .current_shape_rect()
            .unwrap_or_default();

        let com_dir_like = |sample: CommandDirection| {
            let com = command_direction.to_script_value();
            let sample = sample.to_script_value();
            com == sample || com % 8 + 1 == sample || com == sample % 8 + 1
        };
        let hangling_or_swimming =
            matches!(procedure, ActionProcedure::Hang | ActionProcedure::Swim);
        let mut throw_direction = 0;
        let mut right = 0;
        let mut outpos_reduction = 1;
        if procedure != ActionProcedure::Scale {
            if com_dir_like(CommandDirection::Left) {
                throw_direction = -1;
                if actor_xdir < math::fixed10(15) && !hangling_or_swimming {
                    outpos_reduction -= 1;
                }
            }
            if com_dir_like(CommandDirection::Right) {
                throw_direction = 1;
                right = 1;
                if actor_xdir > -math::fixed10(15) && !hangling_or_swimming {
                    outpos_reduction -= 1;
                }
            }
        }

        let edge = actor_shape
            .x
            .wrapping_add(actor_shape.width.wrapping_mul(right));
        let exit_position = Vector2::new(
            actor_position.x.wrapping_add(
                edge.wrapping_mul(i32::from(throw_direction != 0))
                    .wrapping_mul(outpos_reduction),
            ),
            actor_position
                .y
                .wrapping_add(actor_shape.y)
                .wrapping_add(actor_shape.height)
                .wrapping_sub(object_shape.y.wrapping_add(object_shape.height)),
        );
        let exit_velocity = FixedVec2::new(throw_force * throw_direction, C4Fixed::ZERO);

        // ObjectComDrop intentionally ignores Exit's boolean: callback
        // re-entry still proceeds to NoCollectDelay and ObjectComUnGrab.
        let _ = self.exit_object_for_drop(object_id, exit_position, exit_velocity)?;

        if let Some(actor_index) = self.find_object_index(actor_id) {
            self.objects[actor_index].state.no_collect_delay = 2;
            self.refresh_object_ocf(actor_index);
        }
        if let Some(actor_index) = self.find_object_index(actor_id) {
            let _ = self.object_com_ungrab(actor_index)?;
        }
        Ok(true)
    }

    /// The `C4Object::Exit` slice used by ObjectComDrop. The old parent is
    /// refreshed before BoundsCheck while the moving object's own OCF/menu
    /// remain stale; requested motion is installed before Ejection and
    /// Departure (C4Object.cpp:1513-1563).
    pub(in crate::direct_com) fn exit_object_for_drop(
        &mut self,
        object_id: ObjectId,
        target: Vector2,
        velocity: FixedVec2,
    ) -> Result<bool, EngineError> {
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let Some(previous) = self.objects[object_index].state.container else {
            return Ok(false);
        };
        self.exit_object_at_position_with_full_motion(
            object_id,
            previous,
            target,
            0,
            velocity,
            C4Fixed::ZERO,
        )
    }

    /// `ObjectComUnGrab` (C4ObjectCom.cpp:261-278): stand up and release the
    /// grab with the Grab/Grabbed script notifications.
    pub(crate) fn object_com_ungrab(&mut self, index: usize) -> Result<bool, EngineError> {
        if self.object_procedure(index) != ActionProcedure::Push {
            return Ok(false);
        }
        let object_id = self.objects[index].id;
        let target = self.objects[index].state.action.target;
        if !self.object_action_stand_live(object_id)? {
            return Ok(false);
        }
        if !self.close_object_menu(object_id, false)? {
            return Ok(false);
        }
        if let Some(index) = self.find_object_index(object_id) {
            let target_ref = target
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil);
            self.contained_call(index, "Grab", &[target_ref, Value::Bool(false)])?;
            let actor_has_status = self
                .find_object_index(object_id)
                .is_some_and(|index| self.objects[index].has_nonzero_status());
            if actor_has_status {
                if let Some(target_index) = target
                    .and_then(|id| self.find_object_index(id))
                    .filter(|&index| self.objects[index].has_nonzero_status())
                {
                    let self_ref = compat::object_reference_value(object_id);
                    self.contained_call(target_index, "Grabbed", &[self_ref, Value::Bool(false)])?;
                }
            }
        }
        Ok(true)
    }

    // ---- Player command routing -------------------------------------------

    /// `PlayerObjectCommand` (C4ObjectCom.cpp:1013-1040) +
    /// `C4Player::ObjectCommand` (C4Player.cpp:1397-1443): route a control
    /// command to the selected crew (and always the cursor), with the
    /// classic down-double throw→drop conversion.
    #[doc(hidden)]
    pub fn player_object_command(
        &mut self,
        owner: i32,
        mut command: CommandId,
        target: Option<ObjectId>,
        tx: i32,
        ty: i32,
    ) -> Result<bool, EngineError> {
        let Some(player) = self.players.get_mut(&owner) else {
            return Ok(false);
        };
        // Adjust for old-style keyboard throw/drop control (:1018-1019).
        let ranged = matches!(command, CommandId::Throw | CommandId::Drop);
        if command == CommandId::Throw {
            let mut convert_to_drop = false;
            // Drop on down-down-throw (classic, :1024-1033).
            if player.control.last_com_down_double > 0 {
                convert_to_drop = true;
                player.control.last_com = i32::from(COM_DOWN | COM_DOUBLE);
                player.control.last_com_down_double = C4_DOUBLE_CLICK;
            }
            // Jump'n'Run: drop on combined Down+Throw (:1034-1035).
            if player.control.control_style && player.control.pressed_coms & (1 << COM_DOWN) != 0 {
                convert_to_drop = true;
            }
            if convert_to_drop {
                command = CommandId::Drop;
            }
        }
        let mode = if ranged {
            PlayerObjectCommandMode::Add
        } else {
            PlayerObjectCommandMode::Set
        };
        self.player_crew_object_command(owner, command, target, None, tx, ty, 0, mode, ranged)
    }

    /// `C4MouseControl::ButtonUpDragMoving`: issue one independent carryable
    /// Drop/Throw command per locally selected object. The first packet uses
    /// C4P_Command_Set and every later packet uses C4P_Command_Append, so each
    /// selected crew member handles every object in mouse-list order
    /// (C4MouseControl.cpp:1171-1201; C4Player.cpp:1397-1450).
    pub fn player_mouse_drag_objects<I>(
        &mut self,
        owner: i32,
        command: CommandId,
        objects: I,
        position: Vector2,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner)
            || !matches!(command, CommandId::Drop | CommandId::Throw)
        {
            return Ok(false);
        }
        let mut mode = PlayerObjectCommandMode::Set;
        let mut issued = false;
        for target in objects {
            self.player_crew_object_command(
                owner,
                command,
                Some(target),
                None,
                position.x,
                position.y,
                0,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Control-modified carryable drag onto an `OCF_Container`: each packet
    /// is `Put(Target=container, Target2=dragged object, X=Y=0)`. The first
    /// object replaces the crew command stack and the rest append in mouse
    /// selection order; Shift makes the first packet append as well
    /// (C4MouseControl.cpp:742-768,1171-1219).
    pub fn player_mouse_drag_put<I>(
        &mut self,
        owner: i32,
        objects: I,
        container: ObjectId,
        append_to_existing: bool,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let mut mode = if append_to_existing {
            PlayerObjectCommandMode::Append
        } else {
            PlayerObjectCommandMode::Set
        };
        let mut issued = false;
        for object in objects {
            self.player_crew_object_command(
                owner,
                CommandId::Put,
                Some(container),
                Some(object),
                0,
                0,
                0,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Issue ButtonUpDragMoving's vehicle commands. Every selected Grab=1
    /// object receives `PushTo(Target=vehicle, Target2=optional container)`
    /// at the release coordinates; the first packet is Set and later packets
    /// Append, while Shift makes the first packet Append too
    /// (C4MouseControl.cpp:1171-1227).
    pub fn player_mouse_drag_vehicles<I>(
        &mut self,
        owner: i32,
        vehicles: I,
        position: Vector2,
        put_target: Option<ObjectId>,
        append_to_existing: bool,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let mut mode = if append_to_existing {
            PlayerObjectCommandMode::Append
        } else {
            PlayerObjectCommandMode::Set
        };
        let mut issued = false;
        for vehicle in vehicles {
            self.player_crew_object_command(
                owner,
                CommandId::PushTo,
                Some(vehicle),
                put_target,
                position.x,
                position.y,
                0,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Mouse `C4CMD_Context`: unlike ordinary PlayerObjectCommand, the
    /// clicked object occupies Target2 while Target remains null, and Add
    /// mode does not apply the ±15 cursor range (C4MouseControl.cpp:
    /// 1253-1260; C4Player.cpp:1397-1451).
    pub fn player_context_command(
        &mut self,
        owner: i32,
        target: ObjectId,
    ) -> Result<bool, EngineError> {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        self.player_crew_object_command(
            owner,
            CommandId::Context,
            None,
            Some(target),
            0,
            0,
            0,
            PlayerObjectCommandMode::Add,
            false,
        )
    }

    /// `C4Player::ObjectCommand` (C4Player.cpp:1397-1443): apply to all
    /// selected crew in cursor range except the target, then always to the
    /// cursor. `ranged` mirrors C4P_Command_Add|C4P_Command_Range.
    pub(in crate::direct_com) fn player_crew_object_command(
        &mut self,
        owner: i32,
        command: CommandId,
        target: Option<ObjectId>,
        target2: Option<ObjectId>,
        tx: i32,
        ty: i32,
        data: i32,
        mode: PlayerObjectCommandMode,
        ranged: bool,
    ) -> Result<bool, EngineError> {
        if self.is_owner_eliminated(owner) {
            return Ok(false);
        }
        // C4Player::ObjectCommand clears ShowStartup before it commits the
        // selection toggle or dispatches commands to crew.
        if let Some(player) = self.players.get_mut(&owner) {
            player.hide_startup();
        }
        self.player_update_selection_toggle_status(owner)?;
        let cursor = self.crew_cursor(owner);
        let cursor_position = cursor
            .and_then(|id| self.find_object_index(id))
            .map(|index| self.objects[index].state.position);
        let selected = self.selected_crew(owner);
        let mut cursor_processed = false;
        for crew_id in selected {
            if Some(crew_id) == cursor {
                cursor_processed = true;
            }
            if Some(crew_id) == target {
                continue;
            }
            let Some(index) = self.find_object_index(crew_id) else {
                continue;
            };
            if !self.objects[index].has_nonzero_status() {
                continue;
            }
            if ranged {
                // C4P_Command_Range: within ±15 of the cursor (:1412).
                let Some(cursor_position) = cursor_position else {
                    continue;
                };
                let position = self.objects[index].state.position;
                if (position.x - cursor_position.x).abs() > 15
                    || (position.y - cursor_position.y).abs() > 15
                {
                    continue;
                }
            }
            self.object_command_to_obj(index, command, target, target2, tx, ty, data, mode, true)?;
        }
        // Always apply to cursor, even if it's not selected (:1436-1439).
        if let Some(cursor_id) = cursor {
            if !cursor_processed && Some(cursor_id) != target {
                if let Some(index) = self.find_object_index(cursor_id) {
                    if self.objects[index].has_nonzero_status() {
                        self.object_command_to_obj(
                            index, command, target, target2, tx, ty, data, mode, true,
                        )?;
                    }
                }
            }
        }
        Ok(true)
    }

    /// `C4Player::ObjectCommand2Obj` (C4Player.cpp:1445-1451): Add-mode
    /// commands push in front of the stack, Set-mode commands replace it.
    /// The Set path is `C4Object::SetCommand` with fControl
    /// (C4Object.cpp:3923-3981): clear, then the soft menu close, then the
    /// `ControlCommand` script overload before the hardcoded push.
    pub(in crate::direct_com) fn object_command_to_obj(
        &mut self,
        index: usize,
        command: CommandId,
        target: Option<ObjectId>,
        target2: Option<ObjectId>,
        tx: i32,
        ty: i32,
        data: i32,
        mode: PlayerObjectCommandMode,
        f_control: bool,
    ) -> Result<(), EngineError> {
        let request = CommandRequest::new(command)
            .with_target(target)
            .with_target2(target2)
            .with_tx((tx != 0).then_some(tx))
            .with_ty((ty != 0).then_some(ty))
            .with_data(CommandData::Integer(data))
            .with_mode(CommandMode::Base);
        match mode {
            PlayerObjectCommandMode::None => return Ok(()),
            PlayerObjectCommandMode::Add => {
                // C4P_Command_Add → AddCommand(..., fAppend=false): push front
                // without clearing (C4Command.cpp AddCommand semantics).
                self.objects[index]
                    .apply_command_operations([CommandOperation::PushFront(request)]);
                return Ok(());
            }
            PlayerObjectCommandMode::Append => {
                // C4P_Command_Append → AddCommand(..., fAppend=true): retain
                // the independent command sequence in list order.
                self.objects[index].apply_command_operations([CommandOperation::PushBack(request)]);
                return Ok(());
            }
            PlayerObjectCommandMode::Set => {}
        }
        self.set_object_command(index, request, f_control)
    }

    /// `C4Object::SetCommand` for a fully parsed request. Only menu closing
    /// and the command object's own ControlCommand overload are gated by
    /// `f_control`; contained/pushed vehicle overloads run for every entry
    /// point (C4Object.cpp:3939-3983).
    pub(crate) fn set_object_command(
        &mut self,
        index: usize,
        request: CommandRequest,
        f_control: bool,
    ) -> Result<(), EngineError> {
        // SetCommand: decrement NoCollectDelay (:3941-3942), then clear the
        // stack (:3943).
        self.objects[index].apply_command_operations([
            CommandOperation::DecrementNoCollectDelay,
            CommandOperation::Clear,
        ]);
        let object_id = self.objects[index].id;
        if f_control {
            // Close menu — soft: `if (!CloseMenu(false)) return;`
            // (C4Object.cpp:3944-3946). A MenuQueryCancel denial aborts the
            // SetCommand with the stack already cleared.
            if !self.close_object_menu(object_id, false)? {
                return Ok(());
            }
        }
        // The optional menu query may run script, so re-resolve the index.
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Script overload (:3935-3942): `ControlCommand(name, target, tx,
        // ty, target2, data)`.
        let tx = request
            .tx_definition
            .as_ref()
            .map(|id| Value::C4Id(id.as_str().to_string()))
            .or_else(|| request.tx.map(Value::Int))
            .unwrap_or(Value::Int(0));
        let data = match &request.data {
            CommandData::Integer(value) => *value,
            CommandData::Text(_) | CommandData::None => 0,
        };
        let args = [
            Value::String(request.id.to_name().to_string().into()),
            request
                .target
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil),
            tx,
            Value::Int(request.ty.unwrap_or(0)),
            request
                .target2
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil),
            Value::Int(data),
        ];
        if f_control {
            let overloaded = self
                .contained_call(index, "ControlCommand", &args)
                .map(|value| compat::value_raw_truthy(&value))?;
            if overloaded {
                return Ok(());
            }
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Inside vehicle control overload (:3947-3961): the container's
        // ControlCommand with the clonk appended in slot 7.
        if let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        {
            let inside = self
                .definitions
                .get(&self.objects[container_index].definition_id)
                .is_some_and(|definition| {
                    definition.vehicle_control() & crate::VEHICLE_CONTROL_INSIDE != 0
                });
            if inside {
                let controller = self.objects[index].state.controller;
                self.objects[container_index].state.controller = controller;
                let mut vehicle_args = args.to_vec();
                vehicle_args.push(compat::object_reference_value(object_id));
                let consumed = self
                    .contained_call(container_index, "ControlCommand", &vehicle_args)
                    .map(|value| compat::value_raw_truthy(&value))?;
                if consumed {
                    return Ok(());
                }
            }
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Outside vehicle control overload (:3962-3974): the pushed
        // target's ControlCommand, plain six args.
        if self.object_procedure(index) == ActionProcedure::Push {
            if let Some(target_index) = self.objects[index]
                .state
                .action
                .target
                .and_then(|id| self.find_object_index(id))
            {
                let outside = self
                    .definitions
                    .get(&self.objects[target_index].definition_id)
                    .is_some_and(|definition| {
                        definition.vehicle_control() & crate::VEHICLE_CONTROL_OUTSIDE != 0
                    });
                if outside {
                    let controller = self.objects[index].state.controller;
                    self.objects[target_index].state.controller = controller;
                    let consumed = self
                        .contained_call(target_index, "ControlCommand", &args)
                        .map(|value| compat::value_raw_truthy(&value))?;
                    if consumed {
                        return Ok(());
                    }
                }
            }
        }
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].apply_command_operations([CommandOperation::PushFront(request)]);
        }
        Ok(())
    }

    /// Native `SetCommand(C4CMD_Exit)` with the default `fControl=false`:
    /// clear and replace the stack without the menu/own-object control arms,
    /// while retaining the unconditional inside/outside vehicle overloads.
    pub(crate) fn set_plain_exit_command(&mut self, index: usize) -> Result<(), EngineError> {
        self.set_object_command(
            index,
            CommandRequest::new(CommandId::Exit).with_mode(CommandMode::Base),
            false,
        )
    }
}
