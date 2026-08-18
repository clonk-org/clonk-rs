//! `impl Engine` — command result dispatch, completion and failure.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    fn resolve_command_buy_attempt(
        &mut self,
        actor_id: ObjectId,
        base_id: ObjectId,
        definition_id: &str,
        succeeded: bool,
    ) -> Result<(), EngineError> {
        let resolution = self.find_object_index(actor_id).and_then(|index| {
            self.objects[index]
                .commands
                .resolve_pending_buy(base_id, definition_id, succeeded)
        });
        if let Some(feedback) = resolution.and_then(|result| result.feedback) {
            self.execute_command_failure_feedback(actor_id, feedback, None)?;
        }
        Ok(())
    }

    fn resolve_command_sell_attempt(
        &mut self,
        actor_id: ObjectId,
        base_id: ObjectId,
        definition_id: &str,
        succeeded: bool,
    ) -> Result<(), EngineError> {
        let resolution = self.find_object_index(actor_id).and_then(|index| {
            self.objects[index]
                .commands
                .resolve_pending_sell(base_id, definition_id, succeeded)
        });
        if let Some(feedback) = resolution.and_then(|result| result.feedback) {
            self.execute_command_failure_feedback(actor_id, feedback, None)?;
        }
        Ok(())
    }

    pub(crate) fn command_sell_candidate(
        &self,
        actor_id: ObjectId,
        base_id: ObjectId,
        definition_id: &str,
        preferred: Option<ObjectId>,
    ) -> Option<(i32, ObjectId)> {
        if !self.base_sell_enabled {
            return None;
        }
        let actor_index = self.find_object_index(actor_id)?;
        let base_index = self.find_object_index(base_id)?;
        let seller = self.objects[actor_index].state.owner;
        let base_owner = self.objects[base_index].state.base;
        if !self.players.contains_key(&seller)
            || self.players.get(&base_owner).is_none_or(|player| {
                matches!(
                    player.status(),
                    PlayerStatus::Eliminated | PlayerStatus::Surrendered
                ) || player.surrendered()
            })
            || self.players_hostile(seller, base_owner)
        {
            return None;
        }

        // Target2 is preferred exactly once and need not match Data. Only a
        // stale/noncontained target falls back to Contents.Find(Data).
        let candidate = preferred
            .filter(|candidate| {
                self.find_object_index(*candidate).is_some_and(|index| {
                    let object = &self.objects[index];
                    object.has_nonzero_status() && object.state.container == Some(base_id)
                })
            })
            .or_else(|| {
                self.objects[base_index]
                    .state
                    .contents
                    .iter()
                    .copied()
                    .find(|candidate| {
                        self.find_object_index(*candidate).is_some_and(|index| {
                            let object = &self.objects[index];
                            object.has_nonzero_status()
                                && object.definition_id == definition_id
                                && object.state.container == Some(base_id)
                        })
                    })
            })?;
        let candidate_index = self.find_object_index(candidate)?;
        let candidate_definition = &self.objects[candidate_index].definition_id;
        if self
            .definitions
            .get(candidate_definition)
            .is_none_or(|definition| definition.no_sell() != 0)
        {
            return None;
        }
        Some((base_owner, candidate))
    }

    /// Bind a deserialized command event to the fresh runtime identity of
    /// the command which emitted it before any live callback can replace the
    /// object's command stack. Runtime-produced events already carry a
    /// nonzero identity and pass through unchanged.
    fn resolve_command_event_instance_id(
        &self,
        object_id: ObjectId,
        kind: CommandEventInstanceKind,
        command_instance_id: u64,
    ) -> u64 {
        self.find_object_index(object_id)
            .map_or(command_instance_id, |index| {
                self.objects[index]
                    .commands
                    .resolve_event_instance_id(kind, command_instance_id)
            })
    }

    fn resolve_call_result_instance_id(
        &self,
        caller: ObjectId,
        action: &CallResultAction,
        command_instance_id: u64,
    ) -> u64 {
        let kind = match action {
            CallResultAction::CompleteCommandOnFalse { command }
            | CallResultAction::CompleteCommandOnTrue { command }
            | CallResultAction::FailCommandOnFalse { command } => {
                CommandEventInstanceKind::Exact(*command)
            }
            CallResultAction::ResolveExitActivation => CommandEventInstanceKind::ExitActivation,
        };
        self.resolve_command_event_instance_id(caller, kind, command_instance_id)
    }

    /// Apply one command event and report whether it (including any retained
    /// stop/prelude continuation it resumes) crossed a callbackful physical
    /// read. ExecObjects uses this signal to invalidate its outer snapshots.
    pub(crate) fn apply_command_event(&mut self, event: CommandEvent) -> Result<bool, EngineError> {
        let mut resolved_command_physical = false;
        match event {
            CommandEvent::SetPathFinderSettings {
                level,
                transfer_zones_enabled,
            } => {
                self.pathfinder_level = level.clamp(1, 10);
                self.pathfinder_transfer_zones_enabled = transfer_zones_enabled;
            }
            CommandEvent::SetPathFinderDebug { snapshot } => {
                *self.pathfinder_debug.borrow_mut() = snapshot;
            }
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                self.apply_object_update(object_id, update)?;
            }
            CommandEvent::ResolveCommandPhysical {
                object_id,
                reads,
                command_instance_id,
            } => {
                resolved_command_physical = true;
                // Bind the native command pointer before GetPhysical enters
                // GetFairCrewPhysical: that callback may replace the visible
                // stack, while the outer iExec body must still resume.
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Physical,
                    command_instance_id,
                );
                let mut physical = None;
                for _ in 0..reads {
                    let Some(index) = self.find_object_index(object_id) else {
                        break;
                    };
                    // C++ occasionally spells one logical gate as two
                    // GetPhysical calls. Execute every read in order; only
                    // the first missing FairCrew projection invokes script,
                    // and the final returned pointer feeds the continuation.
                    physical = Some(self.object_physical(index));
                }
                if let Some(physical) = physical {
                    resolved_command_physical |= self.resume_command_after_physical(
                        object_id,
                        command_instance_id,
                        physical,
                    )?;
                }
            }
            CommandEvent::MoveToFlightControlTakeoff {
                object_id,
                command_instance_id,
            } => {
                // Fly's callbacks can mutate any object a later ExecObjects
                // command will scan. Rebuild the frame-wide snapshot table
                // even when the retained JumpControl emits no physical read.
                resolved_command_physical = true;
                // Bind the retained MoveTo before Fly's Start/Abort calls:
                // either callback may replace the visible command stack.
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::MoveToFlightControl,
                    command_instance_id,
                );
                if let Some(index) = self.find_object_index(object_id) {
                    let definition_id = self.objects[index].definition_id.clone();
                    // FlightControl deliberately ignores SetActionByName's
                    // result before returning to the procedure-specific tail.
                    let _ = self.action_with_calls(index, &definition_id, "Fly")?;
                }
                resolved_command_physical |=
                    self.resume_move_to_after_flight(object_id, command_instance_id)?;
            }
            CommandEvent::EnterObject {
                object_id,
                container_id,
            } => {
                // C4Command::Enter ignores C4Object::Enter's boolean and
                // finishes successfully after making the attempt
                // (C4Command.cpp:600-605).
                let _ = self.try_object_enter(object_id, container_id)?;
            }
            CommandEvent::GetObject {
                actor_id,
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::Get,
                    command_instance_id,
                );
                let (disposition, message) =
                    match self.try_get_object_enter(actor_id, object_id, command_instance_id)? {
                        GetEnterOutcome::Entered | GetEnterOutcome::Retry => {
                            (GetAttemptDisposition::Continue, None)
                        }
                        GetEnterOutcome::Completed => (GetAttemptDisposition::Complete, None),
                        GetEnterOutcome::MinimumConstructionDenied(message) => {
                            (GetAttemptDisposition::Fail, Some(message))
                        }
                        GetEnterOutcome::Failed => (GetAttemptDisposition::Fail, None),
                    };
                if let Some(actor_index) = self.find_object_index(actor_id) {
                    let resolution = self.objects[actor_index]
                        .commands
                        .resolve_get_attempt(command_instance_id, disposition);
                    if let Some(feedback) = resolution.and_then(|result| result.feedback) {
                        self.execute_command_failure_feedback(actor_id, feedback, message)?;
                    }
                }
            }
            CommandEvent::EvaluateBuy {
                actor_id,
                base_id,
                definition_id,
                buyer,
                payer,
                count,
            } => {
                // The command preflight deliberately prices for the BUYER,
                // while C4Player::Buy below prices each item for the paying
                // base owner. This allied-base mismatch is native behavior.
                let price = tolerate_script_error(self.call_command_buy_value(
                    actor_id,
                    &definition_id,
                    base_id,
                    buyer,
                ))?
                .flatten()
                .unwrap_or(0);
                let payer_wealth = self.players.get(&payer).map(|player| player.wealth());
                if payer_wealth.is_none_or(|wealth| price > wealth) {
                    self.resolve_command_buy_attempt(actor_id, base_id, &definition_id, false)?;
                    return Ok(resolved_command_physical);
                }

                let contained = self
                    .find_object_index(actor_id)
                    .is_some_and(|index| self.objects[index].state.container == Some(base_id));
                if !contained {
                    if let Some(actor_index) = self.find_object_index(actor_id) {
                        self.objects[actor_index]
                            .commands
                            .defer_pending_buy_for_enter(base_id, &definition_id);
                        let _ = self.objects[actor_index].commands.push_front(
                            CommandRequest::new(CommandId::Enter)
                                .with_target(Some(base_id))
                                .with_update_interval(50)
                                .with_mode(CommandMode::SilentSub),
                        );
                    }
                    return Ok(resolved_command_physical);
                }

                let purchase_count = self
                    .find_object_index(actor_id)
                    .and_then(|index| {
                        self.objects[index]
                            .commands
                            .normalize_pending_buy_count(base_id, &definition_id)
                    })
                    .unwrap_or_else(|| count.max(1));

                for _ in 0..purchase_count {
                    // Buy2Base repeats these gates for every item, allowing
                    // earlier Purchase callbacks to invalidate later buys.
                    let live_parties = self
                        .find_object_index(actor_id)
                        .zip(self.find_object_index(base_id))
                        .map(|(actor_index, base_index)| {
                            (
                                self.objects[actor_index].state.owner,
                                self.objects[base_index].state.base,
                            )
                        });
                    let Some((live_buyer, live_payer)) = live_parties else {
                        self.resolve_command_buy_attempt(actor_id, base_id, &definition_id, false)?;
                        return Ok(resolved_command_physical);
                    };
                    if !self.base_buy_enabled
                        || !self.players.contains_key(&live_buyer)
                        || !self.players.contains_key(&live_payer)
                        || self.players_hostile(live_buyer, live_payer)
                    {
                        self.resolve_command_buy_attempt(actor_id, base_id, &definition_id, false)?;
                        return Ok(resolved_command_physical);
                    }

                    let bought = tolerate_script_error(self.call_command_buy_item(
                        actor_id,
                        &definition_id,
                        live_buyer,
                        live_payer,
                        base_id,
                    ))?
                    .unwrap_or(false);
                    if !bought {
                        self.resolve_command_buy_attempt(actor_id, base_id, &definition_id, false)?;
                        return Ok(resolved_command_physical);
                    }
                    if let Some(actor_index) = self.find_object_index(actor_id) {
                        self.objects[actor_index]
                            .commands
                            .record_pending_buy_success(base_id, &definition_id);
                    }
                }
                self.resolve_command_buy_attempt(actor_id, base_id, &definition_id, true)?;
            }
            CommandEvent::EvaluateSell {
                actor_id,
                base_id,
                definition_id,
                preferred,
                count,
            } => {
                let sale_count = self
                    .find_object_index(actor_id)
                    .and_then(|index| {
                        self.objects[index]
                            .commands
                            .normalize_pending_sell_count(base_id, &definition_id)
                    })
                    .unwrap_or_else(|| count.max(1));
                let mut preferred = preferred;

                for _ in 0..sale_count {
                    // SellFromBase repeats every gate and selection against
                    // live state. Earlier successful sales are not rolled
                    // back when a later callback makes the next one fail.
                    let Some((base_owner, candidate)) =
                        self.command_sell_candidate(actor_id, base_id, &definition_id, preferred)
                    else {
                        self.resolve_command_sell_attempt(
                            actor_id,
                            base_id,
                            &definition_id,
                            false,
                        )?;
                        return Ok(resolved_command_physical);
                    };
                    let sold = tolerate_script_error(
                        self.sell_object_to_home(actor_id, candidate, base_owner),
                    )?
                    .unwrap_or(false);
                    if !sold {
                        self.resolve_command_sell_attempt(
                            actor_id,
                            base_id,
                            &definition_id,
                            false,
                        )?;
                        return Ok(resolved_command_physical);
                    }
                    preferred = None;
                    if let Some(actor_index) = self.find_object_index(actor_id) {
                        self.objects[actor_index]
                            .commands
                            .record_pending_sell_success(base_id, &definition_id);
                    }
                }
                self.resolve_command_sell_attempt(actor_id, base_id, &definition_id, true)?;
            }
            CommandEvent::ObjectComPut {
                actor_id,
                target_id,
                object_id,
                ungrab_on_success,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::Put,
                    command_instance_id,
                );
                let succeeded = self.try_object_com_put(actor_id, target_id, object_id)?;
                if succeeded && ungrab_on_success {
                    if let Some(actor_index) = self.find_object_index(actor_id) {
                        // Put's Ty cleanup uses AddCommand(UnGrab) with the
                        // default zero interval, after ObjectComPut callbacks.
                        let _ = self.objects[actor_index]
                            .commands
                            .push_front(CommandRequest::new(CommandId::UnGrab));
                    }
                }
                let feedback = self.find_object_index(actor_id).and_then(|index| {
                    self.objects[index]
                        .commands
                        .resolve_put_attempt(command_instance_id, succeeded)
                });
                if let Some(feedback) = feedback {
                    self.execute_command_failure_feedback(actor_id, feedback, None)?;
                }
            }
            CommandEvent::ObjectComPutTake {
                actor_id,
                target_id,
                requested_item,
                command,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::PutTake(command),
                    command_instance_id,
                );
                let result = self.try_object_com_put_take(actor_id, target_id, requested_item)?;
                if let Some(actor_index) = self.find_object_index(actor_id) {
                    match result {
                        ObjectComPutTakeOutcome::Finished => match command {
                            CommandId::Throw => {
                                self.objects[actor_index]
                                    .commands
                                    .finish_pending_throw(command_instance_id);
                            }
                            CommandId::Drop => {
                                self.objects[actor_index]
                                    .commands
                                    .finish_pending_drop(command_instance_id);
                            }
                            _ => {
                                debug_assert!(false, "ObjectComPutTake must come from Throw/Drop")
                            }
                        },
                        ObjectComPutTakeOutcome::NeedsGet(item_id) => {
                            if self.objects[actor_index]
                                .commands
                                .clear_pending_put_take(command, command_instance_id)
                            {
                                let _ = self.objects[actor_index].commands.push_front(
                                    CommandRequest::new(CommandId::Get)
                                        .with_target(Some(item_id))
                                        .with_update_interval(40)
                                        .with_mode(CommandMode::SilentSub),
                                );
                            }
                        }
                    }
                }
            }
            CommandEvent::ThrowObject {
                actor_id,
                object_id,
                complete_command_on_success,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::Exact(CommandId::Throw),
                    command_instance_id,
                );
                let success = self.try_object_action_throw(actor_id, object_id)?;
                if success || !complete_command_on_success {
                    if let Some(actor_index) = self.find_object_index(actor_id) {
                        self.objects[actor_index]
                            .commands
                            .finish_command_instance(CommandId::Throw, command_instance_id);
                    }
                }
            }
            CommandEvent::ObjectComDrop {
                actor_id,
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::PutTake(CommandId::Drop),
                    command_instance_id,
                );
                let _ = self.object_com_drop(actor_id, object_id)?;
                if let Some(actor_index) = self.find_object_index(actor_id) {
                    self.objects[actor_index]
                        .commands
                        .finish_pending_drop(command_instance_id);
                }
            }
            CommandEvent::ObjectComUnGrabCommand {
                actor_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::Exact(CommandId::UnGrab),
                    command_instance_id,
                );
                if let Some(actor_index) = self.find_object_index(actor_id) {
                    let _ = self.object_com_ungrab(actor_index)?;
                }
                if let Some(actor_index) = self.find_object_index(actor_id) {
                    self.objects[actor_index].state.command_direction = CommandDirection::Stop;
                    self.objects[actor_index]
                        .commands
                        .finish_command_instance(CommandId::UnGrab, command_instance_id);
                }
            }
            CommandEvent::ObjectComJump { object_id, tx } => {
                self.execute_jump_command(object_id, tx)?;
            }
            CommandEvent::ObjectComDig {
                actor_id,
                dig_out_material,
                direction,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::Dig,
                    command_instance_id,
                );
                let succeeded = match self.find_object_index(actor_id) {
                    Some(index) => self.object_com_dig(index)?,
                    None => false,
                };

                // These writes follow ObjectComDig and all SetAction calls
                // synchronously in C4Command::Dig. They use the target and
                // position captured before the callbackful helper.
                if succeeded {
                    if let Some(index) = self.find_object_index(actor_id) {
                        if dig_out_material {
                            self.objects[index].state.action.data = 1;
                        }
                        if let Some(direction) = direction {
                            self.objects[index].state.command_direction = direction;
                        }
                    }
                }

                let feedback = self.find_object_index(actor_id).and_then(|index| {
                    self.objects[index]
                        .commands
                        .resolve_dig_attempt(command_instance_id, succeeded)
                });
                if let Some(feedback) = feedback {
                    // Dig's C4Command::Fail branch deliberately contributes
                    // no second text; ObjectComDig already emitted NODIG.
                    self.execute_command_failure_feedback(actor_id, feedback, None)?;
                }
            }
            CommandEvent::ObjectComExitJump {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Exact(CommandId::Exit),
                    command_instance_id,
                );
                if let Some(index) = self.find_object_index(object_id) {
                    let _ = self.object_com_jump(index)?;
                }
                if let Some(index) = self.find_object_index(object_id) {
                    self.objects[index]
                        .commands
                        .finish_command_instance(CommandId::Exit, command_instance_id);
                }
            }
            CommandEvent::CommandExitObject {
                object_id,
                previous_container,
                position,
                jump_after,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Exact(CommandId::Exit),
                    command_instance_id,
                );
                let _ = self.exit_object_at_position_with_zero_motion(
                    object_id,
                    previous_container,
                    position,
                    0,
                )?;
                if jump_after {
                    if let Some(index) = self.find_object_index(object_id) {
                        let _ = self.object_com_jump(index)?;
                    }
                }
                if let Some(index) = self.find_object_index(object_id) {
                    self.objects[index]
                        .commands
                        .finish_command_instance(CommandId::Exit, command_instance_id);
                }
            }
            CommandEvent::CommandExitIntoParent {
                object_id,
                container_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Exact(CommandId::Exit),
                    command_instance_id,
                );
                let _ = self.try_object_enter(object_id, container_id)?;
                if let Some(index) = self.find_object_index(object_id) {
                    self.objects[index]
                        .commands
                        .finish_command_instance(CommandId::Exit, command_instance_id);
                }
            }
            CommandEvent::ObjectComStopExit {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Exit),
                    command_instance_id,
                );
                let _ = self.object_com_stop_live(object_id)?;
                resolved_command_physical |=
                    self.resume_exit_after_stop(object_id, command_instance_id)?;
            }
            CommandEvent::ObjectComStopMoveTo { object_id } => {
                let _ = self.object_com_stop_live(object_id)?;
                resolved_command_physical |= self.resume_move_to_after_stop(object_id)?;
            }
            CommandEvent::ObjectComStopBuild {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Build),
                    command_instance_id,
                );
                let _ = self.object_com_stop_live(object_id)?;
                resolved_command_physical |=
                    self.resume_build_after_stop(object_id, command_instance_id)?;
            }
            CommandEvent::ObjectComStopChop { object_id } => {
                let _ = self.object_com_stop_live(object_id)?;
            }
            CommandEvent::ObjectComStopConstruct {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Construct),
                    command_instance_id,
                );
                let _ = self.object_com_stop_live(object_id)?;
                resolved_command_physical |=
                    self.resume_construct_after_stop(object_id, command_instance_id)?;
            }
            CommandEvent::ObjectComStopPut {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Put),
                    command_instance_id,
                );
                let _ = self.object_com_stop_live(object_id)?;
                resolved_command_physical |=
                    self.resume_put_after_stop(object_id, command_instance_id)?;
            }
            CommandEvent::ObjectComBuild {
                object_id,
                target_id,
                stop_first,
            } => {
                if stop_first {
                    let _ = self.object_com_stop_live(object_id)?;
                }
                let _ = self.object_com_build_live(object_id, target_id)?;
            }
            CommandEvent::ObjectComStopThrow {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Throw),
                    command_instance_id,
                );
                let _ = self.object_com_stop_live(object_id)?;
                resolved_command_physical |=
                    self.resume_throw_after_prelude(object_id, command_instance_id)?;
            }
            CommandEvent::ObjectComStopDrop {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Drop),
                    command_instance_id,
                );
                let _ = self.object_com_stop_live(object_id)?;
                resolved_command_physical |=
                    self.resume_drop_after_prelude(object_id, command_instance_id)?;
            }
            CommandEvent::ObjectComSetDirThrow {
                object_id,
                direction,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Throw),
                    command_instance_id,
                );
                if let Some(index) = self.find_object_index(object_id) {
                    let definition_id = self.objects[index].definition_id.clone();
                    self.set_command_action_direction(index, &definition_id, direction)?;
                }
                resolved_command_physical |=
                    self.resume_throw_after_prelude(object_id, command_instance_id)?;
            }
            CommandEvent::AttemptGrab {
                actor_id,
                target_id,
            } => {
                self.execute_grab_command(actor_id, target_id)?;
                let feedback = self
                    .find_object_index(actor_id)
                    .and_then(|index| self.objects[index].commands.take_failure_feedback());
                if let Some(feedback) = feedback {
                    self.execute_command_failure_feedback(actor_id, feedback, None)?;
                }
            }
            CommandEvent::SetObjectCommand {
                object_id,
                controller,
                request,
            } => {
                if let Some(index) = self.find_object_index(object_id) {
                    if let Some(controller) = controller {
                        self.objects[index].state.controller = controller;
                    }
                    self.set_object_command(index, request, false)?;
                }
            }
            CommandEvent::ControlCommandAcquire {
                caller,
                target,
                range_x,
                range_y,
                ignore_container,
                definition_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    caller,
                    CommandEventInstanceKind::Script(CommandId::Acquire),
                    command_instance_id,
                );
                let result = self.call_control_command_acquire(
                    caller,
                    target,
                    range_x,
                    range_y,
                    ignore_container,
                    &definition_id,
                )?;
                self.set_acquire_script_result(caller, command_instance_id, result)?;
            }
            CommandEvent::ControlCommandConstruction {
                caller,
                target,
                site,
                target2,
                definition_id,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    caller,
                    CommandEventInstanceKind::Script(CommandId::Construct),
                    command_instance_id,
                );
                let result = self.call_control_command_construction(
                    caller,
                    target,
                    site,
                    target2,
                    &definition_id,
                )?;
                let caller_present = self
                    .find_object_index(caller)
                    .is_some_and(|index| self.objects[index].state.status != ObjectStatus::Deleted);
                if caller_present {
                    resolved_command_physical |=
                        self.resume_construct_after_script(caller, command_instance_id, result)?;
                }
            }
            CommandEvent::ConstructionCheckRejected {
                actor_id,
                definition_id,
                failure,
            } => {
                self.register_construction_check_failure_message(actor_id, &definition_id, failure);
            }
            CommandEvent::SpawnConstruction {
                actor_id,
                definition_id,
                owner,
                position,
                kit_id,
                command_instance_id,
            } => {
                // Runtime events are already stamped. A restored zero token
                // must be rebound before Construction or conkit Destruction
                // callbacks can replace the visible command stack.
                let command_instance_id = self.resolve_command_event_instance_id(
                    actor_id,
                    CommandEventInstanceKind::ConstructSpawn,
                    command_instance_id,
                );
                let (width, height, basement) = self
                    .definitions
                    .get(&definition_id)
                    .map(|definition| {
                        let (width, height) = definition
                            .shape_rect()
                            .map(|shape| (shape.width, shape.height))
                            .unwrap_or_default();
                        (width, height, definition.basement())
                    })
                    .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;

                // C4Game::CreateObjectConstruction(..., 1, true) prepares
                // terrain first, then runs the complete NewObject lifecycle
                // synchronously with a null creator.
                self.prepare_construction_terrain(position.x, position.y, width, height, basement);
                let construction_id = self.spawn_object_with_initial_lifecycle(
                    SpawnConfig::new(definition_id)
                        .with_position(position)
                        .with_owner(owner)
                        .with_construction(1),
                    None,
                )?;

                // Native consumes the retained kit even if creation failed
                // or a creation callback removed the new object.
                let _ = self.assign_object_removal(kit_id)?;
                resolved_command_physical |= self.resume_construct_after_spawn(
                    actor_id,
                    command_instance_id,
                    construction_id,
                )?;
            }
            CommandEvent::SpawnObject {
                definition_id,
                owner,
                position,
                container,
                construction,
            } => {
                if !self.definitions.contains_key(definition_id.as_str()) {
                    return Err(EngineError::UnknownDefinition(definition_id));
                }
                let mut config = SpawnConfig::new(definition_id)
                    .with_position(position)
                    .with_owner(owner);
                if let Some(value) = construction {
                    config = config.with_construction(value);
                }
                if let Some(container_id) = container {
                    config = config.with_container(container_id);
                }
                let _ = self.spawn_object(config)?;
            }
            CommandEvent::CreateLine {
                definition_id,
                owner,
                from,
                to,
            } => {
                let _ = self.create_line_object(&definition_id, owner, from, to)?;
            }
            CommandEvent::ControlTransfer {
                object_id,
                caller,
                tx_value,
                ty,
                command_instance_id,
            } => {
                let command_instance_id = self.resolve_command_event_instance_id(
                    caller,
                    CommandEventInstanceKind::Exact(CommandId::Transfer),
                    command_instance_id,
                );
                let Some(index) = self.find_object_index(object_id) else {
                    return Err(EngineError::UnknownObject(object_id));
                };
                let handled = self.call_control_transfer(index, caller, tx_value, ty)?;
                if !handled {
                    if let Some(caller_index) = self.find_object_index(caller) {
                        self.objects[caller_index]
                            .commands
                            .finish_command_instance(CommandId::Transfer, command_instance_id);
                    }
                }
            }
            CommandEvent::CallObjectFunction {
                object_id,
                function,
                caller,
                tx,
                tx_value,
                tx_definition,
                ty,
                target2,
                on_result,
            } => {
                let command_instance_id = on_result.as_ref().map_or(0, |action| {
                    self.resolve_call_result_instance_id(caller, action, 0)
                });
                let Some(index) = self.find_object_index(object_id) else {
                    return Err(EngineError::UnknownObject(object_id));
                };
                let legacy_transfer = function == "ControlTransfer"
                    && matches!(
                        &on_result,
                        Some(CallResultAction::CompleteCommandOnFalse {
                            command: CommandId::Transfer
                        })
                    );
                if legacy_transfer {
                    // Saves from before the dedicated ControlTransfer event
                    // retain the old generic-call shape. Rebind its command
                    // identity above, but execute the cached definition
                    // function with native's Status bypass and getBool
                    // return conversion just like a newly emitted event.
                    let tx_value = tx_value
                        .or_else(|| tx_definition.map(Value::C4Id))
                        .or_else(|| tx.map(Value::Int))
                        .unwrap_or(Value::Nil);
                    let handled =
                        self.call_control_transfer(index, caller, tx_value, ty.unwrap_or(0))?;
                    self.apply_call_result(
                        on_result.expect("legacy Transfer has a result action"),
                        caller,
                        handled,
                        command_instance_id,
                    )?;
                    return Ok(resolved_command_physical);
                }
                let mut args = Vec::new();
                args.push(object_reference_value(caller));
                args.push(
                    tx_value
                        .or_else(|| {
                            tx_definition
                                .map(Value::C4Id)
                                .or_else(|| tx.map(Value::Int))
                        })
                        .unwrap_or(Value::Nil),
                );
                let ty_value = Value::Int(ty.unwrap_or(0));
                args.push(ty_value);
                let target2_value = target2.map(object_reference_value).unwrap_or(Value::Nil);
                args.push(target2_value);
                let value = self.call_object_function(index, &function, args)?;
                if let Some(action) = on_result {
                    self.apply_call_result(action, caller, value.as_bool(), command_instance_id)?;
                }
            }
            CommandEvent::ActivateEntrance {
                object_id,
                caller,
                on_result,
                command_instance_id,
            } => {
                let command_instance_id =
                    on_result.as_ref().map_or(command_instance_id, |action| {
                        self.resolve_call_result_instance_id(caller, action, command_instance_id)
                    });
                let detached_feedback =
                    matches!(&on_result, Some(CallResultAction::ResolveExitActivation))
                        .then(|| {
                            self.find_object_index(caller).and_then(|index| {
                                self.objects[index]
                                    .commands
                                    .pending_exit_activation_failure_feedback(command_instance_id)
                            })
                        })
                        .flatten();
                let result = self.activate_object_entrance(object_id, caller)?;
                match on_result {
                    Some(CallResultAction::ResolveExitActivation) => {
                        self.resolve_exit_activation_result(
                            caller,
                            result,
                            command_instance_id,
                            detached_feedback,
                        )?;
                    }
                    Some(action) => {
                        self.apply_call_result(action, caller, result, command_instance_id)?
                    }
                    None => {}
                }
            }
            CommandEvent::NativeCommandSuccess { object_id, command } => {
                self.count_crew_info_control(object_id, command.experience_gain());
            }
            CommandEvent::FailureFeedback { actor_id, feedback } => {
                self.execute_command_failure_feedback(actor_id, feedback, None)?;
            }
            CommandEvent::OpenMenu(request) => {
                let Some(crew_index) = self.find_object_index(request.crew_id) else {
                    return Ok(resolved_command_physical);
                };
                match request.kind {
                    MenuRequestKind::Activate => {
                        self.apply_container_menu_request(MenuRequest {
                            crew_id: request.crew_id,
                            owner: request.owner,
                            kind: MenuRequestKind::Activate,
                        })?;
                    }
                    MenuRequestKind::ActivateTarget { container } => {
                        self.apply_container_menu_request(MenuRequest {
                            crew_id: request.crew_id,
                            owner: request.owner,
                            kind: MenuRequestKind::ActivateTarget { container },
                        })?;
                    }
                    MenuRequestKind::Construction => {
                        self.open_construction_menu(crew_index)?;
                    }
                    MenuRequestKind::Buy { base } => {
                        if let Some(base_index) = self.find_object_index(base) {
                            self.open_base_buy_menu(crew_index, base_index)?;
                        }
                    }
                    MenuRequestKind::Sell { base } => {
                        if let Some(base_index) = self.find_object_index(base) {
                            self.open_base_sell_menu(crew_index, base_index)?;
                        }
                    }
                    MenuRequestKind::Get { container } => {
                        self.apply_container_menu_request(MenuRequest {
                            crew_id: request.crew_id,
                            owner: request.owner,
                            kind: MenuRequestKind::Get { container },
                        })?;
                    }
                    MenuRequestKind::Contents { container } => {
                        self.apply_container_menu_request(MenuRequest {
                            crew_id: request.crew_id,
                            owner: request.owner,
                            kind: MenuRequestKind::Contents { container },
                        })?;
                    }
                    MenuRequestKind::Info { target } => {
                        if let Some(target_index) = self.find_object_index(target) {
                            self.open_object_info_menu(crew_index, target_index)?;
                        }
                    }
                    MenuRequestKind::Context { target, position } => {
                        if let Some(target_index) = self.find_object_index(target) {
                            self.open_context_menu(crew_index, target_index, false, position)?;
                        }
                    }
                    kind => self.pending_menu_requests.push(MenuRequest {
                        crew_id: request.crew_id,
                        owner: request.owner,
                        kind,
                    }),
                }
            }
            CommandEvent::AdjustPlayerHomeBaseMaterial {
                player_id,
                definition_id,
                delta,
            } => {
                let _ = self.adjust_player_home_base_material(player_id, definition_id, delta)?;
            }
            CommandEvent::AdjustPlayerWealth { player_id, delta } => {
                let _ = self.adjust_player_wealth(player_id, delta)?;
            }
            CommandEvent::ArmNoCollectDelay { object_id } => {
                // ObjectComDrop (C4ObjectCom.cpp:668-671): NoCollectDelay = 2
                // on the dropper, then its SetOCF so OCF_Collection clears
                // immediately (SetOCF gate, C4Object.cpp:598).
                if let Some(index) = self.find_object_index(object_id) {
                    self.objects[index].state.no_collect_delay = 2;
                    self.refresh_object_ocf(index);
                }
            }
        }
        Ok(resolved_command_physical)
    }

    fn dispatch_control_command_finished(
        &mut self,
        object_id: ObjectId,
        command: command::CommandView,
    ) -> Result<(), EngineError> {
        let args = vec![
            Value::String(command.name.clone().into()),
            command
                .target
                .map(object_reference_value)
                .unwrap_or(Value::Nil),
            command
                .tx_value
                .clone()
                .or_else(|| command.tx_definition.clone().map(Value::C4Id))
                .or_else(|| command.tx.map(Value::Int))
                .unwrap_or(Value::Nil),
            Value::Int(command.ty.unwrap_or(0)),
            command
                .target2
                .map(object_reference_value)
                .unwrap_or(Value::Nil),
            match command.legacy_data {
                Some(value) => compat::command_data_any_value(value),
                None => match &command.data {
                    CommandData::Integer(value) => compat::command_data_any_value(*value),
                    CommandData::Text(value) => Value::String(value.clone().into()),
                    CommandData::None => Value::Nil,
                },
            },
        ];
        if let Some(index) = self
            .find_object_index(object_id)
            .filter(|&index| self.objects[index].state.status != ObjectStatus::Deleted)
        {
            let _ = tolerate_script_error(self.call_object_function(
                index,
                "ControlCommandFinished",
                args,
            ))?;
        }
        Ok(())
    }

    pub(crate) fn finish_object_command_execution(
        &mut self,
        object_id: ObjectId,
    ) -> Result<(), EngineError> {
        let successful_finishes = self
            .find_object_index(object_id)
            .map(|index| self.objects[index].commands.take_successful_finishes())
            .unwrap_or_default();
        for command in successful_finishes {
            self.count_crew_info_control(object_id, command.experience_gain());
        }
        let finished = self
            .find_object_index(object_id)
            .and_then(|index| self.objects[index].commands.finished_front_view());
        if let Some(command) = finished {
            self.dispatch_control_command_finished(object_id, command)?;
        }
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].commands.clear_finished_fronts();
        }
        Ok(())
    }

    /// `ConstructionCheck`'s `GameMsgObject(..., pByObj, FRed)` feedback for
    /// the Construct command's site rejection (C4Landscape.cpp:2131-2163;
    /// C4Command.cpp:1797-1801). FRed resolves through the C4.PAL entry
    /// FColors[FRed]=47 (C4GameMessage.cpp:280-282; C4Surface.cpp:1304).
    fn register_construction_check_failure_message(
        &mut self,
        actor_id: ObjectId,
        definition_id: &str,
        failure: command::ConstructionCheckFailure,
    ) {
        if self.find_object_index(actor_id).is_none() {
            return;
        }
        let strings = Rc::clone(&self.construction_check_strings);
        let text = match failure {
            command::ConstructionCheckFailure::NotConstructable => {
                let definition_name = self
                    .definitions
                    .get(definition_id)
                    .map(|definition| definition.name().to_string())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| definition_id.to_string());
                strings.format_not_constructable(&definition_name)
            }
            command::ConstructionCheckFailure::NoRoom => strings.no_room.clone(),
            command::ConstructionCheckFailure::NoLevel => strings.no_level.clone(),
            command::ConstructionCheckFailure::Blocked(blocker) => {
                strings.format_blocked(&self.object_message_name(blocker))
            }
        };
        self.messages.add_message(crate::message::MessageSpec {
            kind: crate::message::MessageKind::Target,
            text,
            target: Some(actor_id),
            player: None,
            offset: Vector2::ZERO,
            color: CONSTRUCTION_CHECK_MESSAGE_COLOR,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        });
    }

    /// C4Command::Fail's mode-gated ExecFail tail. CommandStack decides the
    /// mode/base/retry gate while the failed command is still linked; this
    /// live half performs callbacks before the common ComDir stop and before
    /// ControlCommandFinished (C4Command.cpp:2139-2242,2428-2439).
    pub(crate) fn execute_command_failure_feedback(
        &mut self,
        actor_id: ObjectId,
        feedback: CommandFailureFeedback,
        fail_message: Option<String>,
    ) -> Result<(), EngineError> {
        let mut fail_message = fail_message;
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(());
        };
        if fail_message.is_none()
            && feedback.reason == Some(command::CommandFailureReason::CannotBuild)
        {
            let actor = &self.objects[actor_index];
            let name = actor
                .state
                .custom_name
                .clone()
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    self.crew_object_infos
                        .get(&actor_id)
                        .map(|info| info.name.clone())
                })
                .or_else(|| {
                    self.definitions
                        .get(&actor.definition_id)
                        .map(|definition| definition.name().to_string())
                })
                .unwrap_or_else(|| actor.definition_id.clone());
            fail_message = Some(format!("{name} can't build."));
        }
        // C++ reads the cached OCF at Fail entry. Inactive objects still have
        // a nonzero Status and remain eligible for the later common tail.
        if self.objects[actor_index].state.ocf & ocf::CREW_MEMBER == 0 {
            return Ok(());
        }

        let command = feedback.command;
        match command.name.as_str() {
            "Call" => {
                if let (Some(target), CommandData::Text(text)) = (command.target, &command.data) {
                    if !text.is_empty() {
                        if let Some(target_index) = self.find_object_index(target) {
                            let args = vec![
                                object_reference_value(actor_id),
                                command
                                    .tx_value
                                    .clone()
                                    .or_else(|| command.tx_definition.clone().map(Value::C4Id))
                                    .or_else(|| command.tx.map(Value::Int))
                                    .unwrap_or(Value::Nil),
                                Value::Int(command.ty.unwrap_or(0)),
                                command
                                    .target2
                                    .map(object_reference_value)
                                    .unwrap_or(Value::Nil),
                            ];
                            let function = format!("{text}Failed");
                            let handled = tolerate_script_error(self.call_object_function(
                                target_index,
                                &function,
                                args,
                            ))?
                            .is_some_and(|value| value.as_bool());
                            // C++ reads the raw value union through _getInt;
                            // raw truthiness preserves nonzero IDs/pointers.
                            if handled {
                                return Ok(());
                            }
                        }
                    }
                }
            }
            "Build" => {
                if let Some(target) = command.target {
                    let (component, count) =
                        self.find_object_index(target)
                            .and_then(|target_index| {
                                let state = &self.objects[target_index].state;
                                state.component_order.first().map(|id| {
                                    (Some(id.clone()), state.components.get(id).unwrap_or(0))
                                })
                            })
                            .unwrap_or_default();
                    if let Some(builder_index) = self.find_object_index(actor_id) {
                        // A truthy result suppresses only the generated
                        // material message in C++; sound/Stop still follow.
                        let handled = tolerate_script_error(self.call_object_function(
                            builder_index,
                            "BuildNeedsMaterial",
                            vec![
                                component.map(Value::C4Id).unwrap_or(Value::Nil),
                                Value::Int(count),
                            ],
                        ))?
                        .is_some_and(|value| value.as_bool());
                        if !handled && fail_message.is_none() {
                            // Message construction is not presentation-only:
                            // GetNeededMatStr may synchronously invoke the
                            // target definition's GetCustomComponents.
                            let expression = format!("GetNeededMatStr(Object({target}))");
                            fail_message = tolerate_script_error(self.direct_exec_on_object(
                                builder_index,
                                &expression,
                                "CommandFail:GetNeededMatStr",
                            ))?
                            .and_then(|value| match value {
                                Value::String(text) => Some(text.into_string()),
                                _ => None,
                            });
                        }
                    }
                }
            }
            _ => {}
        }

        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(());
        };
        if self.objects[actor_index].state.status == ObjectStatus::Deleted {
            return Ok(());
        }
        let silent_commands = self
            .definitions
            .get(&self.objects[actor_index].definition_id)
            .is_some_and(Definition::silent_commands);
        if silent_commands {
            return Ok(());
        }
        if let Some(text) = fail_message {
            self.messages.apply_command(MessageCommand::Append {
                spec: MessageSpec {
                    kind: message::MessageKind::Target,
                    text,
                    target: Some(actor_id),
                    player: None,
                    offset: Vector2::ZERO,
                    color: 0xffff_ffff,
                    flags: 0,
                    width: None,
                    decoration: None,
                    frame_decoration: None,
                    portrait: None,
                },
                no_duplicates: true,
            });
        }
        self.objects[actor_index].state.command_direction = CommandDirection::Stop;
        Ok(())
    }

    /// C4Object::ActivateEntrance (C4Object.cpp:1654-1670). This is a native
    /// gate, not a generic call to the same-named script function: command
    /// Enter/Exit must reject hostile bases first and then require the
    /// receiver's current cached OCF_Entrance.
    pub(crate) fn activate_object_entrance(
        &mut self,
        object_id: ObjectId,
        caller: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let Some(caller_index) = self.find_object_index(caller) else {
            return Ok(false);
        };
        // ActivateEntrance is invoked through retained raw pointers. It
        // reads the caller's Controller and the receiver's Base/Owner/OCF
        // before its eventual C4Object::Call applies the Status gate, so a
        // status-zero tombstone can still produce the hostile-base message
        // (C4Object.cpp:1654-1670; C4Object.cpp:2224-2227).
        let by_player = self.objects[caller_index].state.controller;
        let (base, owner) = {
            let object = &self.objects[index];
            (object.state.base, object.state.owner)
        };

        if self.base_reject_entrance_enabled && self.players_hostile(by_player, base) {
            if let Some(owner_name) = self
                .players
                .get(&owner)
                .map(|player| player.name().to_string())
            {
                self.messages.add_message(MessageSpec {
                    kind: message::MessageKind::Target,
                    text: format!("{owner_name} hostile.|No entrance!"),
                    target: Some(object_id),
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
            return Ok(false);
        }

        if self.object_ocf_at_index(index) & ocf::ENTRANCE == 0 {
            return Ok(false);
        }

        let value = tolerate_script_error(self.call_object_function(
            index,
            "ActivateEntrance",
            vec![object_reference_value(caller)],
        ))?;
        // Native uses C4Value::operator bool here (`if (Call(...))`), not
        // `getBool`; preserve full raw-union truthiness for noncanonical
        // C4VBool payloads.
        Ok(value.is_some_and(|value| value.as_bool()))
    }

    /// C4Command::Transfer's direct cached-function dispatch. Native reads
    /// `Def->Script.SFn_ControlTransfer` and invokes that exact script body
    /// with `f->Exec(Target, ...)`; it neither routes through C4Object::Call
    /// nor rejects a Status-zero receiver (C4Command.cpp:1931-1942).
    pub(crate) fn call_control_transfer(
        &mut self,
        index: usize,
        caller: ObjectId,
        tx_value: Value,
        ty: i32,
    ) -> Result<bool, EngineError> {
        let callback = {
            let object = self
                .objects
                .get(index)
                .ok_or_else(|| EngineError::UnknownObject(ObjectId::new(u64::MAX)))?;
            let definition = self
                .definitions
                .get(&object.definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(object.definition_id.clone()))?;
            let Some(callback) = definition.control_transfer_callback() else {
                return Ok(false);
            };
            callback
        };
        let value = tolerate_script_error(self.call_direct_object_callback(
            index,
            &callback,
            vec![object_reference_value(caller), tx_value, Value::Int(ty)],
        ))?;
        Ok(value.is_some_and(|value| {
            value
                .c4_bool_raw()
                .map_or_else(|| value.as_bool(), |raw| raw != 0)
        }))
    }

    fn apply_call_result(
        &mut self,
        action: CallResultAction,
        caller: ObjectId,
        result: bool,
        command_instance_id: u64,
    ) -> Result<(), EngineError> {
        match action {
            CallResultAction::CompleteCommandOnFalse { command } => {
                if !result {
                    let Some(index) = self.find_object_index(caller) else {
                        return Err(EngineError::UnknownObject(caller));
                    };
                    self.objects[index]
                        .commands
                        .finish_command_instance(command, command_instance_id);
                }
            }
            CallResultAction::CompleteCommandOnTrue { command } => {
                if result {
                    let Some(index) = self.find_object_index(caller) else {
                        return Err(EngineError::UnknownObject(caller));
                    };
                    self.objects[index]
                        .commands
                        .finish_command_instance(command, command_instance_id);
                }
            }
            CallResultAction::FailCommandOnFalse { command } => {
                if !result {
                    let Some(index) = self.find_object_index(caller) else {
                        return Err(EngineError::UnknownObject(caller));
                    };
                    self.objects[index]
                        .commands
                        .fail_command_instance(command, command_instance_id);
                }
            }
            CallResultAction::ResolveExitActivation => {
                self.resolve_exit_activation_result(caller, result, command_instance_id, None)?;
            }
        }
        Ok(())
    }

    fn resolve_exit_activation_result(
        &mut self,
        caller: ObjectId,
        result: bool,
        command_instance_id: u64,
        detached_feedback: Option<CommandFailureFeedback>,
    ) -> Result<(), EngineError> {
        let resolution = self.find_object_index(caller).and_then(|index| {
            self.objects[index]
                .commands
                .resolve_exit_activation(result, command_instance_id)
        });
        let feedback = match resolution {
            Some(resolution) => resolution.feedback,
            None if !result => detached_feedback,
            None => None,
        };
        if let Some(feedback) = feedback {
            self.execute_command_failure_feedback(caller, feedback, None)?;
        }
        Ok(())
    }

    pub(crate) fn set_acquire_script_result(
        &mut self,
        object_id: ObjectId,
        command_instance_id: u64,
        result: AcquireScriptResult,
    ) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        if let Some(object) = self.objects.get_mut(index) {
            // Native checks `!cObj->Status` after the callback and returns
            // before interpreting result 2 for a deleted caller.
            if object.state.status != ObjectStatus::Deleted {
                let _ = object
                    .commands
                    .resolve_acquire_script_result(command_instance_id, result);
            }
        }
        Ok(())
    }

    fn call_control_command_acquire(
        &mut self,
        caller: ObjectId,
        target: Option<ObjectId>,
        range_x: i32,
        range_y: i32,
        ignore_container: Option<ObjectId>,
        definition_id: &str,
    ) -> Result<AcquireScriptResult, EngineError> {
        let Some(index) = self.find_object_index(caller) else {
            return Ok(AcquireScriptResult::Continue);
        };
        let mut args = Vec::new();
        args.push(target.map(object_reference_value).unwrap_or(Value::Nil));
        args.push(Value::Int(range_x));
        args.push(Value::Int(range_y));
        args.push(
            ignore_container
                .map(object_reference_value)
                .unwrap_or(Value::Nil),
        );
        let definition_value = definition_id_to_c4id(definition_id)
            .map(Value::Int)
            .unwrap_or_else(|| Value::String(definition_id.to_string().into()));
        args.push(definition_value);

        let value = self.call_object_function(index, "~ControlCommandAcquire", args)?;
        let code = match value {
            Value::Int(code) => Some(code),
            Value::Bool(flag) => Some(if flag { 1 } else { 0 }),
            _ => None,
        };

        Ok(match code.and_then(AcquireScriptResult::from_code) {
            Some(result) => result,
            None => AcquireScriptResult::Continue,
        })
    }

    fn call_control_command_construction(
        &mut self,
        caller: ObjectId,
        target: Option<ObjectId>,
        site: Vector2,
        target2: Option<ObjectId>,
        definition_id: &str,
    ) -> Result<AcquireScriptResult, EngineError> {
        let Some(index) = self.find_object_index(caller) else {
            return Ok(AcquireScriptResult::Continue);
        };
        let args = vec![
            target.map(object_reference_value).unwrap_or(Value::Nil),
            Value::Int(site.x),
            Value::Int(site.y),
            target2.map(object_reference_value).unwrap_or(Value::Nil),
            definition_id_to_c4id(definition_id)
                .map(Value::Int)
                .unwrap_or_else(|| Value::String(definition_id.to_string().into())),
        ];
        let value = self.call_object_function(index, "~ControlCommandConstruction", args)?;
        let code = match value {
            Value::Int(code) => Some(code),
            Value::Bool(flag) => Some(if flag { 1 } else { 0 }),
            _ => None,
        };
        Ok(code
            .and_then(AcquireScriptResult::from_code)
            .unwrap_or(AcquireScriptResult::Continue))
    }

    /// C4Game::ClearObjectPtrs fallback for engine-owned removal paths.
    /// Script RemoveObject stages the same layer and transfer-zone clears
    /// synchronously in compat; this idempotent sweep also covers direct
    /// status updates and queued/native destruction after their callbacks.
    pub(crate) fn clear_destroyed_object_layers(&mut self) {
        let destroyed = self
            .objects
            .iter()
            .filter(|object| {
                object.destroyed || matches!(object.state.status, ObjectStatus::Deleted)
            })
            .map(|object| object.id)
            .collect::<HashSet<_>>();
        if destroyed.is_empty() {
            return;
        }
        for object_id in &destroyed {
            self.transfer_zones.clear(*object_id);
        }
        for object in &mut self.objects {
            if object
                .state
                .layer
                .is_some_and(|layer| destroyed.contains(&layer))
            {
                object.state.layer = None;
                object.compiler_cache.layer = 0;
            }
        }
    }

    fn detach_audio_for_object(&mut self, target: ObjectId, position: Vector2) {
        if self.pending_audio.iter().any(|command| {
            matches!(
                command,
                AudioCommand::PlaySound {
                    target: Some(event_target),
                    ..
                }
                | AudioCommand::PlaySpeech {
                    target: Some(event_target),
                    ..
                }
                | AudioCommand::SetSoundVolume {
                    target: Some(event_target),
                    ..
                } if *event_target == target
            )
        }) {
            self.audio_registry.note_attached_sound(target);
        }
        self.audio_registry.detach_object_sounds(target, position);
        for event in self.audio_registry.take_events() {
            let duplicate_detach = matches!(
                &event,
                AudioCommand::DetachObjectSounds {
                    target: event_target,
                    ..
                } if *event_target == target
            ) && self.pending_audio.iter().any(|pending| {
                matches!(
                    pending,
                    AudioCommand::DetachObjectSounds {
                        target: pending_target,
                        ..
                    } if *pending_target == target
                )
            });
            if !duplicate_detach {
                self.pending_audio.push(event);
            }
        }
    }

    pub(crate) fn detach_destroyed_objects(&mut self) -> Result<(), EngineError> {
        self.clear_destroyed_object_layers();
        let mut updates = Vec::new();
        let destroyed_objects: Vec<(ObjectId, Vector2)> = self
            .objects
            .iter()
            .filter(|object| {
                object.destroyed || matches!(object.state.status, ObjectStatus::Deleted)
            })
            .map(|object| (object.id, object.state.position))
            .collect();
        let destroyed: Vec<ObjectId> = destroyed_objects
            .iter()
            .map(|(object, _)| *object)
            .collect();
        for (object, position) in destroyed_objects {
            self.detach_audio_for_object(object, position);
        }
        for object_id in &destroyed {
            self.clear_effect_command_target(*object_id);
        }
        for object in &self.objects {
            if (object.destroyed || matches!(object.state.status, ObjectStatus::Deleted))
                && object.state.container.is_some()
            {
                updates.push((object.id, object.state.container));
            }

            if object.destroyed || matches!(object.state.status, ObjectStatus::Deleted) {
                for child in &object.state.contents {
                    updates.push((*child, Some(object.id)));
                }
            }
        }

        for (object_id, previous) in updates {
            self.apply_container_change(object_id, previous, None, false)?;
        }

        for object in &mut self.objects {
            if object.destroyed || matches!(object.state.status, ObjectStatus::Deleted) {
                object.state.contents.clear();
            }
        }

        if !destroyed.is_empty() {
            // C4Object::Clear retires and nulls Info before Game.ClearPointers.
            for object_id in &destroyed {
                if let Some(link) = self.crew_info_links.get(object_id).copied() {
                    if let Some(info) = self
                        .crew_rosters
                        .get_mut(&link.player_id)
                        .and_then(|roster| roster.get_mut(link.roster_index))
                    {
                        if info.in_action {
                            info.total_playing_time = info
                                .total_playing_time
                                .wrapping_add(self.game_time.wrapping_sub(info.in_action_time));
                            info.in_action = false;
                        }
                    }
                }
                Rc::make_mut(&mut self.crew_info_links).remove(object_id);
                Rc::make_mut(&mut self.crew_object_infos).remove(object_id);
                Rc::make_mut(&mut self.crew_ranks).remove(&object_id.as_u64());
            }

            // C4Player::ClearPointers runs synchronously during object
            // removal: Cursor/ViewCursor/ViewTarget must never retain the
            // dead pointer (C4Player.cpp:55-73). Cursor-mode view then falls
            // back from ViewCursor to Cursor; target mode keeps its last
            // center with a null ViewTarget until the next input.
            let active: HashSet<ObjectId> = self
                .objects
                .iter()
                .filter(|object| {
                    !object.destroyed && !matches!(object.state.status, ObjectStatus::Deleted)
                })
                .map(|object| object.id)
                .collect();
            let owners = self.player_ids_in_order();
            for object in &destroyed {
                // C4MessageInput::ClearPointers closes a script type-in whose
                // callback object is being removed (C4MessageInput.cpp:737-742).
                // Clear the process-local dialog as part of this ordinary
                // detach path as well as the explicit host-command path.
                if self
                    .active_message_board_input
                    .as_ref()
                    .is_some_and(|input| input.target == Some(*object))
                {
                    self.active_message_board_input = None;
                }
                for owner in &owners {
                    let removed_cursor = self.crew_cursor(*owner) == Some(*object);
                    if removed_cursor {
                        if let Some(selection) = self.crew_selection.get_mut(owner) {
                            selection.set_cursor(None);
                        }
                    }
                    if let Some(player) = self.players.get_mut(owner) {
                        player.clear_object_pointers(*object);
                    }
                    self.remove_from_roles(*owner, *object);
                    if removed_cursor {
                        self.player_adjust_cursor_command(*owner)?;
                    }
                }
            }
            self.crew_selection.retain(|_, selection| {
                selection.prune(&active);
                !selection.is_empty()
            });
            self.sync_all_player_cursors();
        }

        Ok(())
    }

    /// Engine-side fallback for removals that do not originate in a live
    /// script host call. The synchronous host sweep performs the same
    /// mutation at RemoveObject call time; this catches native/status paths
    /// before destroyed objects are detached and global effects execute.
    fn clear_effect_command_target(&mut self, target: ObjectId) {
        let Ok(target) = i32::try_from(target.as_u64()) else {
            return;
        };
        for object in &mut self.objects {
            for effect in &mut object.state.effects {
                if effect.command_target == Some(target) {
                    effect.priority = 0;
                    effect.command_target = None;
                }
            }
        }
        for effect in &mut self.global_effects {
            if effect.command_target == Some(target) {
                effect.priority = 0;
                effect.command_target = None;
            }
        }
    }

    /// Synchronous `FirstRef->Set0(); Game.ClearPointers(this)` tail of
    /// AssignRemoval (C4Object.cpp:302-304; C4Game.cpp:1018-1031).
    pub(crate) fn clear_object_references_for_removal(
        &mut self,
        target: ObjectId,
    ) -> Result<(), EngineError> {
        if let Some(position) = self
            .find_object_index(target)
            .map(|index| self.objects[index].state.position)
        {
            self.detach_audio_for_object(target, position);
        }
        let remaining_numbers = self
            .objects
            .iter()
            .filter(|object| object.id != target)
            .map(|object| object.id.as_u64())
            .collect::<HashSet<_>>();

        for object in &mut self.objects {
            if object.state.action.target == Some(target) {
                object.state.action.target = None;
                object.compiler_cache.action_target1 = 0;
            }
            if object.state.action.target2 == Some(target) {
                object.state.action.target2 = None;
                object.compiler_cache.action_target2 = 0;
            }
            if object.state.layer == Some(target) {
                object.state.layer = None;
                object.compiler_cache.layer = 0;
            }
            object.commands.clear_object_reference(target);
            for value in object.state.local_vars.values_mut() {
                *value = denumerate_script_value(value, &remaining_numbers);
            }
            for effect in &mut object.state.effects {
                denumerate_effect(effect, &remaining_numbers);
            }
            object
                .state
                .graphics_overlays
                .retain(|overlay| overlay.overlay_object != Some(target));
            if let Some(menu) = object.state.menu.as_mut() {
                if menu.command_object == Some(target) {
                    menu.command_object = None;
                }
                if menu.refill_object == Some(target) {
                    menu.refill_object = None;
                }
                menu.identification =
                    denumerate_script_value(&menu.identification, &remaining_numbers);
                for item in &mut menu.items {
                    if item.picture_object == Some(target) {
                        item.picture_object = None;
                    }
                    if matches!(
                        item.image,
                        ObjectMenuImage::Object { object }
                            | ObjectMenuImage::ObjectRank { object }
                            if object == target
                    ) {
                        item.image = ObjectMenuImage::None;
                    }
                }
            }
        }
        for effect in &mut self.global_effects {
            denumerate_effect(effect, &remaining_numbers);
        }

        let named_cells = self
            .script_globals
            .borrow()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let constant_cells = self
            .script_global_consts
            .borrow()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let numbered_cells = self
            .script_global_slots
            .borrow()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for cell in named_cells
            .into_iter()
            .chain(constant_cells)
            .chain(numbered_cells)
        {
            let value = cell.borrow().clone();
            *cell.borrow_mut() = denumerate_script_value(&value, &remaining_numbers);
        }

        self.messages.clear_for_object(target);
        if self
            .active_message_board_input
            .as_ref()
            .is_some_and(|input| input.target == Some(target))
        {
            self.active_message_board_input = None;
        }
        self.transfer_zones.clear(target);
        self.clear_effect_command_target(target);

        let active = self
            .objects
            .iter()
            .filter(|object| {
                object.id != target
                    && !object.destroyed
                    && object.state.status != ObjectStatus::Deleted
            })
            .map(|object| object.id)
            .collect::<HashSet<_>>();
        let owners = self.player_ids_in_order();
        for owner in owners {
            let removed_cursor = self.crew_cursor(owner) == Some(target);
            if let Some(selection) = self.crew_selection.get_mut(&owner) {
                selection.prune(&active);
                if removed_cursor {
                    selection.set_cursor(None);
                }
            }
            if let Some(player) = self.players.get_mut(&owner) {
                player.clear_object_pointers(target);
            }
            self.remove_from_roles(owner, target);
            if removed_cursor {
                self.player_adjust_cursor_command(owner)?;
            }
        }
        self.crew_selection
            .retain(|_, selection| !selection.is_empty());
        self.sync_all_player_cursors();
        if let Some(index) = self.find_object_index(target) {
            self.objects[index].state.crew_member = false;
        }
        Ok(())
    }

    pub(crate) fn apply_global_effect_commands(&mut self, commands: &[EffectCommand]) {
        apply_effect_commands_to_stack(&mut self.global_effects, commands);
    }
}
