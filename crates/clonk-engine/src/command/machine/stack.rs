//! `command` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandStackSnapshot {
    pub(in crate::command) commands: Vec<CommandSnapshot>,
    /// Runtime-only monotonic allocator state. Never rewound by an in-memory
    /// callback Restore; persisted restores safely restart with fresh ids.
    #[serde(skip)]
    next_instance_id: u64,
    #[serde(skip)]
    detached_grab_attempts: Vec<DetachedGrabAttempt>,
    /// Runtime-only iExec-retained Get bodies. This crosses an in-memory
    /// callback Restore but is deliberately absent from persisted saves.
    #[serde(skip)]
    detached_get_attempts: Vec<DetachedGetAttempt>,
    /// Runtime-only iExec-retained Put bodies. This crosses an in-memory
    /// callback Restore but is deliberately absent from persisted saves.
    #[serde(skip)]
    detached_put_attempts: Vec<DetachedPutAttempt>,
}

impl PartialEq for CommandStackSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.commands == other.commands
            && self.detached_grab_attempts == other.detached_grab_attempts
            && self.detached_get_attempts == other.detached_get_attempts
            && self.detached_put_attempts == other.detached_put_attempts
    }
}

impl CommandStackSnapshot {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// C++ CommandName strings for the persisted stack, top first
    /// (FnGetCommand walks `Command->Next`, C4Script.cpp:918-945).
    pub fn command_names(&self) -> Vec<String> {
        self.commands
            .iter()
            .map(|command| {
                command
                    .state
                    .id()
                    .map(CommandId::to_name)
                    .unwrap_or("None")
                    .to_string()
            })
            .collect()
    }

    /// FnGetCommand element views from the persisted request + live
    /// state overrides — restored stacks keep their elements.
    pub fn command_views(&self) -> Vec<CommandView> {
        self.commands
            .iter()
            .map(|command| {
                CommandView::from_entry(
                    command
                        .state
                        .id()
                        .map(CommandId::to_name)
                        .unwrap_or("None")
                        .to_string(),
                    command.request.as_ref(),
                    &command.state,
                    command.finished.is_some(),
                )
            })
            .collect()
    }

    pub(crate) fn legacy_save_commands(&self) -> Vec<LegacyCommandSave> {
        self.commands
            .iter()
            .map(|command| {
                let view = CommandView::from_entry(
                    command
                        .state
                        .id()
                        .map(CommandId::to_name)
                        .unwrap_or("None")
                        .to_string(),
                    command.request.as_ref(),
                    &command.state,
                    command.finished.is_some(),
                );
                legacy_command_save(
                    view,
                    command.request.as_ref(),
                    &command.state,
                    command.update_interval.unwrap_or_else(|| {
                        command.request.as_ref().map_or_else(
                            || {
                                i32::try_from(command.state.legacy_update_interval())
                                    .unwrap_or(i32::MAX)
                            },
                            |request| request.update_interval,
                        )
                    }),
                    command.evaluated,
                    command.path_checked,
                    command.finished.is_some(),
                    command.failures,
                    command.retries,
                    command.permit,
                    command.mode,
                    command.legacy_evaluated_word,
                    command.legacy_path_checked_word,
                    command.legacy_finished_word,
                    command.legacy_text.as_deref(),
                )
            })
            .collect()
    }

    /// Rebuild the live typed command stack from the fields emitted by
    /// `C4Command::CompileFunc`. Object pointers are still enumerated here;
    /// `Engine::finish_legacy_object_load` resolves them after all objects
    /// have materialized, just like C4GameObjects::Load.
    pub(crate) fn from_legacy_save_commands(
        commands: Vec<LegacyCommandSave>,
    ) -> Result<Self, CommandError> {
        let mut snapshots = Vec::with_capacity(commands.len());
        for command in commands {
            let id = CommandId::from_name(&command.view.name).ok_or(CommandError::Unsupported)?;
            let mode = CommandMode::from_i32(command.base_mode).ok_or(CommandError::Unsupported)?;
            let data = if id == CommandId::Call {
                CommandData::Text(command.text.clone())
            } else {
                command.view.data.clone()
            };
            let request = CommandRequest {
                id,
                target: command.view.target,
                target2: command.view.target2,
                tx: command.view.tx.or_else(|| {
                    command
                        .view
                        .tx_definition
                        .as_deref()
                        .and_then(definition_id_to_c4id)
                }),
                tx_value: command.view.tx_value.clone().or_else(|| {
                    command
                        .view
                        .tx_definition
                        .as_ref()
                        .map(|value| clonk_script::Value::C4Id(value.clone()))
                        .or_else(|| command.view.tx.map(clonk_script::Value::Int))
                }),
                tx_definition: command.view.tx_definition.clone(),
                ty: command.view.ty,
                data,
                update_interval: command.update_interval,
                evaluated: command.evaluated != 0,
                retries: command.retries,
                mode,
            };
            let mut active = ActiveCommand::from_request(request)?;
            if let CommandState::Call(state) = &mut active.state {
                state.legacy_data = command.view.legacy_data.unwrap_or({
                    match command.view.data {
                        CommandData::Integer(value) => value,
                        CommandData::Text(_) | CommandData::None => 0,
                    }
                });
            }
            active.retries = command.retries;
            active.failures = command.failures;
            active.evaluated = command.evaluated != 0;
            active.path_checked =
                !matches!(active.state, CommandState::MoveTo(_)) && command.path_checked != 0;
            active.permit = command.permit;
            active.legacy_evaluated_word =
                (!matches!(command.evaluated, 0 | 1)).then_some(command.evaluated);
            active.legacy_path_checked_word =
                (!matches!(command.path_checked, 0 | 1)).then_some(command.path_checked);
            active.legacy_finished_word =
                (!matches!(command.finished, 0 | 1)).then_some(command.finished);
            active.legacy_text =
                (id != CommandId::Call && !command.text.is_empty()).then_some(command.text);
            active.update_interval = command.update_interval;
            active.finished = (command.finished != 0).then_some(CommandStatus::Completed);
            active
                .state
                .restore_legacy_evaluation(command.evaluated != 0, command.path_checked != 0);
            snapshots.push(CommandSnapshot::new(&active));
        }
        Ok(Self {
            commands: snapshots,
            next_instance_id: 0,
            detached_grab_attempts: Vec::new(),
            detached_get_attempts: Vec::new(),
            detached_put_attempts: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct CommandStack {
    pub(in crate::command) entries: VecDeque<ActiveCommand>,
    /// Nonzero identity allocator for native-command lifetime matching.
    next_instance_id: u64,
    detached_grab_attempts: Vec<DetachedGrabAttempt>,
    /// GetTryEnter crosses several callback boundaries. Preserve the exact
    /// executing Get if one of those callbacks unlinks it from the object.
    pub(in crate::command) detached_get_attempts: VecDeque<DetachedGetAttempt>,
    /// A callback may ClearCommands/SetCommand while ObjectComStop is
    /// running. Native keeps the executing C4Command alive through its
    /// iExec guard, so retain that detached MoveTo until the same-Execute
    /// continuation consumes it.
    detached_move_to_stops: VecDeque<MoveToState>,
    /// FlightControl's ordinary Fly action can run callbacks which unlink
    /// the executing MoveTo before its walking-only JumpControl tail.
    pub(in crate::command) detached_move_to_flights: VecDeque<DetachedMoveToFlight>,
    /// Build has the same callback-detachment hazard while its Dig stop is
    /// in flight; retain that exact executing state through the continuation.
    detached_build_stops: VecDeque<DetachedBuildStop>,
    /// Exit's DFA_BUILD ObjectComStop must likewise retain the exact command
    /// (and its failure base chain) if a stop callback replaces the stack.
    detached_exit_preludes: VecDeque<DetachedExitPrelude>,
    /// Throw has two callback boundaries inside one C4Command::Execute:
    /// ObjectComStop for a digging actor and SetDir's TurnAction at a
    /// targeted launch point. A callback may unlink the command while its
    /// native body is still executing, so retain that body until the
    /// matching continuation event returns.
    pub(in crate::command) detached_throw_preludes: VecDeque<DetachedThrowPrelude>,
    /// Drop has the same retained ObjectComStop boundary when it starts in
    /// DFA_DIG. Keep a detached body alive if callbacks replace the stack.
    detached_drop_preludes: VecDeque<DetachedDropPrelude>,
    /// Put's callbackful ObjectComPut may unlink the executing command.
    /// Retain its failure/base context until the helper returns.
    pub(in crate::command) detached_put_attempts: VecDeque<DetachedPutAttempt>,
    /// Put's DFA_DIG ObjectComStop retains the full executing command and
    /// original base chain while callbacks may replace the visible stack.
    detached_put_stops: VecDeque<DetachedPutStop>,
    /// Construct can cross both ObjectComStop and its script overload after
    /// a physical hook has already detached the executing command.
    detached_construct_commands: VecDeque<DetachedConstructCommand>,
    /// GetPhysical's scripted fair-crew fill is a synchronous callback.
    /// ClearCommands/SetCommand may unlink the executing native command,
    /// whose iExec guard nevertheless keeps its post-callback body alive.
    detached_physical_commands: VecDeque<DetachedPhysicalCommand>,
    /// Live Grab callbacks resolve inside engine/compat event handling, so
    /// their failure feedback cannot travel on the original CommandEvent.
    /// Keep it transient and let that synchronous caller drain it before
    /// `ControlCommandFinished` runs.
    pending_failure_feedback: VecDeque<CommandFailureFeedback>,
    /// Native `C4Command::Finish(true)` calls awaiting the owning object's
    /// experience tail. This is separate from `Finished=Completed` because
    /// script `FinishCommand(true)` sets that flag without calling Finish.
    pending_successful_finishes: VecDeque<CommandId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GetAttemptDisposition {
    Continue,
    Complete,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GetAttemptResolution {
    pub feedback: Option<CommandFailureFeedback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuyAttemptResolution {
    pub feedback: Option<CommandFailureFeedback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SellAttemptResolution {
    pub feedback: Option<CommandFailureFeedback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExitActivationResolution {
    pub feedback: Option<CommandFailureFeedback>,
}

/// The live-command predicate that identifies the native C4Command retained
/// by a callbackful [`CommandEvent`]. Runtime instance ids are deliberately
/// omitted from persisted snapshots, so a restored zero-token event must be
/// rebound to the freshly allocated command identity before its callback can
/// replace the visible stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandEventInstanceKind {
    Exact(CommandId),
    Dig,
    Get,
    Put,
    PutTake(CommandId),
    Prelude(CommandId),
    ExitActivation,
    Script(CommandId),
    Physical,
    MoveToFlightControl,
    ConstructSpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::command) struct DetachedGrabAttempt {
    target: ObjectId,
    target_retained: bool,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedThrowPrelude {
    pub(in crate::command) entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedMoveToFlight {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedBuildStop {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedExitPrelude {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedDropPrelude {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::command) struct DetachedPutAttempt {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedPutStop {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedConstructCommand {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::command) struct DetachedGetAttempt {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
pub(in crate::command) struct DetachedPhysicalCommand {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::command) struct DetachedCommandBase {
    instance_id: u64,
    retries: i32,
    finished: Option<CommandStatus>,
}

impl From<&ActiveCommand> for DetachedCommandBase {
    fn from(entry: &ActiveCommand) -> Self {
        Self {
            instance_id: entry.instance_id,
            retries: entry.retries,
            finished: entry.finished,
        }
    }
}

impl Default for CommandStack {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandStack {
    // `mod command` is only `#[doc(hidden)] pub` as a test seam, so this `new`
    // is not real public API and needs no `Default`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next_instance_id: 1,
            detached_grab_attempts: Vec::new(),
            detached_get_attempts: VecDeque::new(),
            detached_move_to_stops: VecDeque::new(),
            detached_move_to_flights: VecDeque::new(),
            detached_build_stops: VecDeque::new(),
            detached_exit_preludes: VecDeque::new(),
            detached_throw_preludes: VecDeque::new(),
            detached_drop_preludes: VecDeque::new(),
            detached_put_attempts: VecDeque::new(),
            detached_put_stops: VecDeque::new(),
            detached_construct_commands: VecDeque::new(),
            detached_physical_commands: VecDeque::new(),
            pending_failure_feedback: VecDeque::new(),
            pending_successful_finishes: VecDeque::new(),
        }
    }

    fn record_native_success(&mut self, command: CommandId) {
        self.pending_successful_finishes.push_back(command);
    }

    /// Drain native successful finishes before `ControlCommandFinished`.
    /// The queue survives callback-driven stack replacement because the
    /// executing C++ command remains alive through its `iExec` guard.
    pub(crate) fn take_successful_finishes(&mut self) -> Vec<CommandId> {
        self.pending_successful_finishes.drain(..).collect()
    }

    /// Resolve the runtime identity of a command event before invoking its
    /// first callback. A nonzero token already names the exact native command
    /// and passes through unchanged. Zero is the persisted-event fallback:
    /// select the same pending state that the event's eventual continuation
    /// or finish method consumes, including an iExec-retained prelude that a
    /// callback has detached from the visible stack.
    pub(crate) fn resolve_event_instance_id(
        &self,
        kind: CommandEventInstanceKind,
        supplied: u64,
    ) -> u64 {
        if supplied != 0 {
            return supplied;
        }

        let attached = self.entries.iter().find(|entry| match kind {
            CommandEventInstanceKind::Exact(command) => entry.id() == Some(command),
            CommandEventInstanceKind::Dig => matches!(
                &entry.state,
                CommandState::Dig(state) if state.start_pending
            ),
            CommandEventInstanceKind::Get => matches!(
                &entry.state,
                CommandState::Get(state) if state.enter_pending
            ),
            CommandEventInstanceKind::Put => matches!(
                &entry.state,
                CommandState::Put(state) if state.put_pending
            ),
            CommandEventInstanceKind::PutTake(CommandId::Throw) => matches!(
                &entry.state,
                CommandState::Throw(state) if state.put_take_pending
            ),
            CommandEventInstanceKind::PutTake(CommandId::Drop) => matches!(
                &entry.state,
                CommandState::Drop(state) if state.completion_pending
            ),
            CommandEventInstanceKind::Prelude(CommandId::Exit) => matches!(
                &entry.state,
                CommandState::Exit(state) if state.stop_continuation
            ),
            CommandEventInstanceKind::Prelude(CommandId::Throw) => matches!(
                &entry.state,
                CommandState::Throw(state) if !state.continuations.is_empty()
            ),
            CommandEventInstanceKind::Prelude(CommandId::Drop) => matches!(
                &entry.state,
                CommandState::Drop(state) if !state.continuations.is_empty()
            ),
            CommandEventInstanceKind::Prelude(CommandId::Put) => matches!(
                &entry.state,
                CommandState::Put(state) if state.stop_continuation.is_some()
            ),
            CommandEventInstanceKind::Prelude(CommandId::Construct) => matches!(
                &entry.state,
                CommandState::Construct(state) if state.stop_continuation
            ),
            CommandEventInstanceKind::Prelude(CommandId::Build) => matches!(
                &entry.state,
                CommandState::Build(state) if state.stop_continuation
            ),
            CommandEventInstanceKind::ExitActivation => matches!(
                &entry.state,
                CommandState::Exit(state) if state.activation_pending != 0
            ),
            CommandEventInstanceKind::Script(CommandId::Acquire) => matches!(
                &entry.state,
                CommandState::Acquire(state) if state.script_pending
            ),
            CommandEventInstanceKind::Script(CommandId::Construct) => matches!(
                &entry.state,
                CommandState::Construct(state) if state.script_pending
            ),
            CommandEventInstanceKind::Physical => entry.state.has_physical_continuation(),
            CommandEventInstanceKind::MoveToFlightControl => matches!(
                &entry.state,
                CommandState::MoveTo(state) if state.flight_continuation.is_some()
            ),
            CommandEventInstanceKind::ConstructSpawn => matches!(
                &entry.state,
                CommandState::Construct(state)
                    if state.spawn_requested && state.construction_id.is_none()
            ),
            CommandEventInstanceKind::PutTake(_)
            | CommandEventInstanceKind::Prelude(_)
            | CommandEventInstanceKind::Script(_) => false,
        });
        if let Some(entry) = attached {
            return entry.instance_id;
        }

        match kind {
            CommandEventInstanceKind::Prelude(CommandId::Exit) => self
                .detached_exit_preludes
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Exit(state) if state.stop_continuation
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Prelude(CommandId::Throw) => self
                .detached_throw_preludes
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Throw(state) if !state.continuations.is_empty()
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Prelude(CommandId::Drop) => self
                .detached_drop_preludes
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Drop(state) if !state.continuations.is_empty()
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Prelude(CommandId::Put) => self
                .detached_put_stops
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Put(state) if state.stop_continuation.is_some()
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Prelude(CommandId::Construct) => self
                .detached_construct_commands
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Construct(state) if state.stop_continuation
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Prelude(CommandId::Build) => self
                .detached_build_stops
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Build(state) if state.stop_continuation
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::ExitActivation => self
                .detached_exit_preludes
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Exit(state) if state.activation_pending != 0
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Get => self
                .detached_get_attempts
                .iter()
                .rev()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Get(state) if state.enter_pending
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Physical => self
                .detached_physical_commands
                .iter()
                .find(|detached| detached.entry.state.has_physical_continuation())
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::MoveToFlightControl => self
                .detached_move_to_flights
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::MoveTo(state) if state.flight_continuation.is_some()
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Script(CommandId::Construct) => self
                .detached_construct_commands
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Construct(state) if state.script_pending
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::ConstructSpawn => self
                .detached_construct_commands
                .iter()
                .find(|detached| {
                    matches!(
                        &detached.entry.state,
                        CommandState::Construct(state)
                            if state.spawn_requested && state.construction_id.is_none()
                    )
                })
                .map(|detached| detached.entry.instance_id),
            CommandEventInstanceKind::Exact(_)
            | CommandEventInstanceKind::Dig
            | CommandEventInstanceKind::Put
            | CommandEventInstanceKind::PutTake(_)
            | CommandEventInstanceKind::Prelude(_)
            | CommandEventInstanceKind::Script(_) => None,
        }
        .unwrap_or(0)
    }

    fn pending_grab_attempt(entry: &ActiveCommand) -> Option<DetachedGrabAttempt> {
        let CommandState::Grab(state) = &entry.state else {
            return None;
        };
        if !state.reject_pending {
            return None;
        }
        let request_retained = entry
            .request
            .as_ref()
            .is_none_or(|request| request.target == Some(state.target));
        Some(DetachedGrabAttempt {
            target: state.target,
            target_retained: !state.target_cleared && request_retained,
        })
    }

    fn remember_detached_grab(&mut self, entry: &ActiveCommand) {
        if let Some(attempt) = Self::pending_grab_attempt(entry) {
            self.detached_grab_attempts.push(attempt);
        }
    }

    fn remember_detached_move_to_stop(&mut self, entry: &ActiveCommand) {
        if let CommandState::MoveTo(state) = &entry.state {
            if state.stop_continuation.is_some() {
                self.detached_move_to_stops.push_back(state.clone());
            }
        }
    }

    fn remember_detached_move_to_flight(&mut self, entry: &ActiveCommand) {
        if matches!(
            &entry.state,
            CommandState::MoveTo(state) if state.flight_continuation.is_some()
        ) {
            self.detached_move_to_flights
                .push_back(DetachedMoveToFlight {
                    entry: entry.clone(),
                    base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
                });
        }
    }

    fn remember_detached_build_stop(&mut self, entry: &ActiveCommand) {
        if let CommandState::Build(state) = &entry.state {
            if state.stop_continuation {
                self.detached_build_stops.push_back(DetachedBuildStop {
                    entry: entry.clone(),
                    base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
                });
            }
        }
    }

    fn remember_detached_exit_prelude(&mut self, entry: &ActiveCommand) {
        if matches!(
            &entry.state,
            CommandState::Exit(state)
                if state.stop_continuation || state.activation_pending != 0
        ) {
            self.detached_exit_preludes.push_back(DetachedExitPrelude {
                entry: entry.clone(),
                base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
            });
        }
    }

    fn remember_detached_throw_prelude(&mut self, entry: &ActiveCommand) {
        if let CommandState::Throw(state) = &entry.state {
            if !state.continuations.is_empty() {
                self.detached_throw_preludes
                    .push_back(DetachedThrowPrelude {
                        entry: entry.clone(),
                        base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
                    });
            }
        }
    }

    fn remember_detached_drop_prelude(&mut self, entry: &ActiveCommand) {
        if let CommandState::Drop(state) = &entry.state {
            if !state.continuations.is_empty() {
                self.detached_drop_preludes.push_back(DetachedDropPrelude {
                    entry: entry.clone(),
                    base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
                });
            }
        }
    }

    fn remember_detached_put_attempt(&mut self, entry: &ActiveCommand) {
        if matches!(&entry.state, CommandState::Put(state) if state.put_pending) {
            self.detached_put_attempts.push_back(DetachedPutAttempt {
                entry: entry.clone(),
                base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
            });
        }
    }

    fn remember_detached_put_stop(&mut self, entry: &ActiveCommand) {
        if matches!(
            &entry.state,
            CommandState::Put(state) if state.stop_continuation.is_some()
        ) {
            self.detached_put_stops.push_back(DetachedPutStop {
                entry: entry.clone(),
                base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
            });
        }
    }

    fn remember_detached_construct(&mut self, entry: &ActiveCommand) {
        if matches!(
            &entry.state,
            CommandState::Construct(state)
                if state.stop_continuation
                    || state.script_pending
                    || (state.spawn_requested && state.construction_id.is_none())
        ) {
            self.detached_construct_commands
                .push_back(DetachedConstructCommand {
                    entry: entry.clone(),
                    base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
                });
        }
    }

    fn remember_detached_get_attempt(&mut self, entry: &ActiveCommand) {
        if matches!(&entry.state, CommandState::Get(state) if state.enter_pending) {
            self.detached_get_attempts.push_back(DetachedGetAttempt {
                entry: entry.clone(),
                base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
            });
        }
    }

    fn remember_detached_physical(&mut self, entry: &ActiveCommand) {
        if entry.state.has_physical_continuation() {
            self.detached_physical_commands
                .push_back(DetachedPhysicalCommand {
                    entry: entry.clone(),
                    base_chain: self.entries.iter().map(DetachedCommandBase::from).collect(),
                });
        }
    }

    pub(in crate::command) fn pop_front(&mut self) -> Option<ActiveCommand> {
        let entry = self.entries.pop_front()?;
        self.remember_detached_grab(&entry);
        self.remember_detached_get_attempt(&entry);
        self.remember_detached_move_to_stop(&entry);
        self.remember_detached_move_to_flight(&entry);
        self.remember_detached_build_stop(&entry);
        self.remember_detached_exit_prelude(&entry);
        self.remember_detached_throw_prelude(&entry);
        self.remember_detached_drop_prelude(&entry);
        self.remember_detached_put_attempt(&entry);
        self.remember_detached_put_stop(&entry);
        self.remember_detached_construct(&entry);
        self.remember_detached_physical(&entry);
        Some(entry)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// C++ CommandName strings for the active stack, top first
    /// (FnGetCommand walks `Command->Next`, C4Script.cpp:918-945).
    pub fn command_names(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| {
                entry
                    .state
                    .id()
                    .map(CommandId::to_name)
                    .unwrap_or("None")
                    .to_string()
            })
            .collect()
    }

    /// FnGetCommand element views for the active stack, top first.
    pub fn command_views(&self) -> Vec<CommandView> {
        self.entries
            .iter()
            .map(|entry| {
                CommandView::from_entry(
                    entry
                        .state
                        .id()
                        .map(CommandId::to_name)
                        .unwrap_or("None")
                        .to_string(),
                    entry.request.as_ref(),
                    &entry.state,
                    entry.finished.is_some(),
                )
            })
            .collect()
    }

    pub(crate) fn legacy_save_commands(&self) -> Vec<LegacyCommandSave> {
        self.entries
            .iter()
            .map(|entry| {
                let view = CommandView::from_entry(
                    entry
                        .state
                        .id()
                        .map(CommandId::to_name)
                        .unwrap_or("None")
                        .to_string(),
                    entry.request.as_ref(),
                    &entry.state,
                    entry.finished.is_some(),
                );
                legacy_command_save(
                    view,
                    entry.request.as_ref(),
                    &entry.state,
                    entry.update_interval,
                    entry.evaluated,
                    entry.path_checked,
                    entry.finished.is_some(),
                    entry.failures,
                    entry.retries,
                    entry.permit,
                    entry.mode,
                    entry.legacy_evaluated_word,
                    entry.legacy_path_checked_word,
                    entry.legacy_finished_word,
                    entry.legacy_text.as_deref(),
                )
            })
            .collect()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The front command's kind name, if any (ObjectComStopDig's
    /// C4CMD_Dig check, C4ObjectCom.cpp:776-784).
    pub fn front_command_name(&self) -> Option<&'static str> {
        self.entries
            .front()
            .and_then(|entry| entry.state.id())
            .map(CommandId::to_name)
    }

    /// Drops the front command (ClearCommand on the stack top).
    pub fn clear_front(&mut self) {
        self.pop_front();
    }

    pub fn clear(&mut self) {
        let attempts = self
            .entries
            .iter()
            .rev()
            .filter_map(Self::pending_grab_attempt)
            .collect::<Vec<_>>();
        self.detached_grab_attempts.extend(attempts);
        let move_to_stops = self
            .entries
            .iter()
            .filter_map(|entry| match &entry.state {
                CommandState::MoveTo(state) if state.stop_continuation.is_some() => {
                    Some(state.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_move_to_stops.extend(move_to_stops);
        let move_to_flights = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    &entry.state,
                    CommandState::MoveTo(state) if state.flight_continuation.is_some()
                )
            })
            .map(|(index, entry)| DetachedMoveToFlight {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_move_to_flights.extend(move_to_flights);
        let build_stops = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                CommandState::Build(state) if state.stop_continuation => Some(DetachedBuildStop {
                    entry: entry.clone(),
                    base_chain: self
                        .entries
                        .iter()
                        .skip(index + 1)
                        .map(DetachedCommandBase::from)
                        .collect(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_build_stops.extend(build_stops);
        let exit_preludes = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                CommandState::Exit(state)
                    if state.stop_continuation || state.activation_pending != 0 =>
                {
                    Some(DetachedExitPrelude {
                        entry: entry.clone(),
                        base_chain: self
                            .entries
                            .iter()
                            .skip(index + 1)
                            .map(DetachedCommandBase::from)
                            .collect(),
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_exit_preludes.extend(exit_preludes);
        let throw_preludes = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                CommandState::Throw(state) if !state.continuations.is_empty() => {
                    Some(DetachedThrowPrelude {
                        entry: entry.clone(),
                        base_chain: self
                            .entries
                            .iter()
                            .skip(index + 1)
                            .map(DetachedCommandBase::from)
                            .collect(),
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_throw_preludes.extend(throw_preludes);
        let drop_preludes = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                CommandState::Drop(state) if !state.continuations.is_empty() => {
                    Some(DetachedDropPrelude {
                        entry: entry.clone(),
                        base_chain: self
                            .entries
                            .iter()
                            .skip(index + 1)
                            .map(DetachedCommandBase::from)
                            .collect(),
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_drop_preludes.extend(drop_preludes);
        let get_attempts = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(&entry.state, CommandState::Get(state) if state.enter_pending)
            })
            .map(|(index, entry)| DetachedGetAttempt {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_get_attempts.extend(get_attempts);
        let put_attempts = self
            .entries
            .iter()
            .enumerate()
            .filter(
                |(_, entry)| matches!(&entry.state, CommandState::Put(state) if state.put_pending),
            )
            .map(|(index, entry)| DetachedPutAttempt {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_put_attempts.extend(put_attempts);
        let put_stops = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    &entry.state,
                    CommandState::Put(state) if state.stop_continuation.is_some()
                )
            })
            .map(|(index, entry)| DetachedPutStop {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_put_stops.extend(put_stops);
        let construct_commands = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    &entry.state,
                    CommandState::Construct(state)
                        if state.stop_continuation
                            || state.script_pending
                            || (state.spawn_requested && state.construction_id.is_none())
                )
            })
            .map(|(index, entry)| DetachedConstructCommand {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_construct_commands.extend(construct_commands);
        let physical_commands = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.state.has_physical_continuation())
            .map(|(index, entry)| DetachedPhysicalCommand {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_physical_commands.extend(physical_commands);
        self.entries.clear();
    }

    /// A staged SetCommand that detaches an executing Grab must copy its
    /// frozen target-pointer state back as a snapshot. Replaying only the
    /// Clear operation later can invert its order with same-call removal.
    pub(crate) fn has_pending_grab_attempt(&self) -> bool {
        !self.detached_grab_attempts.is_empty()
            || self.entries.iter().any(|entry| {
                matches!(
                    &entry.state,
                    CommandState::Grab(state) if state.reject_pending
                )
            })
    }

    /// `GrabLost` clears through the predecessor of the first PushTo that
    /// has one, keeping that PushTo and its tail (C4Object.cpp:4262-4273).
    pub(crate) fn clear_to_first_push_to(&mut self) {
        let Some(count) = self
            .entries
            .iter()
            .skip(1)
            .position(|entry| entry.id() == Some(CommandId::PushTo))
            .map(|index| index + 1)
        else {
            return;
        };
        for _ in 0..count {
            self.pop_front();
        }
    }

    pub fn snapshot(&self) -> CommandStackSnapshot {
        CommandStackSnapshot {
            commands: self.entries.iter().map(CommandSnapshot::new).collect(),
            next_instance_id: self.next_instance_id,
            detached_grab_attempts: self.detached_grab_attempts.clone(),
            detached_get_attempts: self.detached_get_attempts.iter().cloned().collect(),
            detached_put_attempts: self.detached_put_attempts.iter().cloned().collect(),
        }
    }

    pub fn restore_from_snapshot(&mut self, snapshot: &CommandStackSnapshot) {
        // A callback-driven replacement detaches the currently executing
        // MoveTo, but native iExec keeps it alive until Execute returns. If
        // the incoming snapshot still contains a pending MoveTo, it is the
        // same retained command and must not also be queued as detached.
        let snapshot_retains_move_to = snapshot.commands.iter().any(|command| {
            matches!(
                &command.state,
                CommandState::MoveTo(state) if state.stop_continuation.is_some()
            )
        });
        if !snapshot_retains_move_to {
            let move_to_stops = self
                .entries
                .iter()
                .filter_map(|entry| match &entry.state {
                    CommandState::MoveTo(state) if state.stop_continuation.is_some() => {
                        Some(state.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            self.detached_move_to_stops.extend(move_to_stops);
        }
        let retained_move_to_flight_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| {
                matches!(
                    &command.state,
                    CommandState::MoveTo(state) if state.flight_continuation.is_some()
                )
                .then_some(command.instance_id)
            })
            .collect::<HashSet<_>>();
        let detached_move_to_flights = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    &entry.state,
                    CommandState::MoveTo(state) if state.flight_continuation.is_some()
                ) && !retained_move_to_flight_ids.contains(&entry.instance_id)
            })
            .map(|(index, entry)| DetachedMoveToFlight {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_move_to_flights
            .extend(detached_move_to_flights);
        let retained_build_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| {
                matches!(
                    &command.state,
                    CommandState::Build(state) if state.stop_continuation
                )
                .then_some(command.instance_id)
            })
            .collect::<HashSet<_>>();
        let build_stops = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    &entry.state,
                    CommandState::Build(state) if state.stop_continuation
                ) && !retained_build_ids.contains(&entry.instance_id)
            })
            .map(|(index, entry)| DetachedBuildStop {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_build_stops.extend(build_stops);
        let retained_exit_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| match &command.state {
                CommandState::Exit(state)
                    if state.stop_continuation || state.activation_pending != 0 =>
                {
                    Some(command.instance_id)
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let detached_exit_preludes = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                CommandState::Exit(state)
                    if (state.stop_continuation || state.activation_pending != 0)
                        && !retained_exit_ids.contains(&entry.instance_id) =>
                {
                    Some(DetachedExitPrelude {
                        entry: entry.clone(),
                        base_chain: self
                            .entries
                            .iter()
                            .skip(index + 1)
                            .map(DetachedCommandBase::from)
                            .collect(),
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_exit_preludes.extend(detached_exit_preludes);
        let retained_throw_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| match &command.state {
                CommandState::Throw(state) if !state.continuations.is_empty() => {
                    Some(command.instance_id)
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let detached_throw_preludes = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                CommandState::Throw(state)
                    if !state.continuations.is_empty()
                        && !retained_throw_ids.contains(&entry.instance_id) =>
                {
                    Some(DetachedThrowPrelude {
                        entry: entry.clone(),
                        base_chain: self
                            .entries
                            .iter()
                            .skip(index + 1)
                            .map(DetachedCommandBase::from)
                            .collect(),
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_throw_preludes.extend(detached_throw_preludes);
        let retained_drop_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| match &command.state {
                CommandState::Drop(state) if !state.continuations.is_empty() => {
                    Some(command.instance_id)
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let detached_drop_preludes = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                CommandState::Drop(state)
                    if !state.continuations.is_empty()
                        && !retained_drop_ids.contains(&entry.instance_id) =>
                {
                    Some(DetachedDropPrelude {
                        entry: entry.clone(),
                        base_chain: self
                            .entries
                            .iter()
                            .skip(index + 1)
                            .map(DetachedCommandBase::from)
                            .collect(),
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.detached_drop_preludes.extend(detached_drop_preludes);
        for attempt in &snapshot.detached_get_attempts {
            if !self
                .detached_get_attempts
                .iter()
                .any(|existing| existing.entry.instance_id == attempt.entry.instance_id)
            {
                self.detached_get_attempts.push_back(attempt.clone());
            }
        }
        let retained_get_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| {
                matches!(&command.state, CommandState::Get(state) if state.enter_pending)
                    .then_some(command.instance_id)
            })
            .chain(
                snapshot
                    .detached_get_attempts
                    .iter()
                    .map(|attempt| attempt.entry.instance_id),
            )
            .collect::<HashSet<_>>();
        let detached_get_attempts = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(&entry.state, CommandState::Get(state) if state.enter_pending)
                    && !retained_get_ids.contains(&entry.instance_id)
            })
            .map(|(index, entry)| DetachedGetAttempt {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_get_attempts.extend(detached_get_attempts);
        for attempt in &snapshot.detached_put_attempts {
            if !self
                .detached_put_attempts
                .iter()
                .any(|existing| existing.entry.instance_id == attempt.entry.instance_id)
            {
                self.detached_put_attempts.push_back(attempt.clone());
            }
        }
        let retained_put_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| {
                matches!(&command.state, CommandState::Put(state) if state.put_pending)
                    .then_some(command.instance_id)
            })
            .chain(
                snapshot
                    .detached_put_attempts
                    .iter()
                    .map(|attempt| attempt.entry.instance_id),
            )
            .collect::<HashSet<_>>();
        let detached_put_attempts = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(&entry.state, CommandState::Put(state) if state.put_pending)
                    && !retained_put_ids.contains(&entry.instance_id)
            })
            .map(|(index, entry)| DetachedPutAttempt {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_put_attempts.extend(detached_put_attempts);
        let retained_put_stop_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| {
                matches!(
                    &command.state,
                    CommandState::Put(state) if state.stop_continuation.is_some()
                )
                .then_some(command.instance_id)
            })
            .collect::<HashSet<_>>();
        let detached_put_stops = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    &entry.state,
                    CommandState::Put(state) if state.stop_continuation.is_some()
                ) && !retained_put_stop_ids.contains(&entry.instance_id)
            })
            .map(|(index, entry)| DetachedPutStop {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_put_stops.extend(detached_put_stops);
        let retained_construct_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| {
                matches!(
                    &command.state,
                    CommandState::Construct(state)
                        if state.stop_continuation
                            || state.script_pending
                            || (state.spawn_requested && state.construction_id.is_none())
                )
                .then_some(command.instance_id)
            })
            .collect::<HashSet<_>>();
        let detached_construct_commands = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(
                    &entry.state,
                    CommandState::Construct(state)
                        if state.stop_continuation
                            || state.script_pending
                            || (state.spawn_requested && state.construction_id.is_none())
                ) && !retained_construct_ids.contains(&entry.instance_id)
            })
            .map(|(index, entry)| DetachedConstructCommand {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_construct_commands
            .extend(detached_construct_commands);
        let retained_physical_ids = snapshot
            .commands
            .iter()
            .filter_map(|command| {
                command
                    .state
                    .has_physical_continuation()
                    .then_some(command.instance_id)
            })
            .collect::<HashSet<_>>();
        let detached_physical_commands = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.state.has_physical_continuation()
                    && !retained_physical_ids.contains(&entry.instance_id)
            })
            .map(|(index, entry)| DetachedPhysicalCommand {
                entry: entry.clone(),
                base_chain: self
                    .entries
                    .iter()
                    .skip(index + 1)
                    .map(DetachedCommandBase::from)
                    .collect(),
            })
            .collect::<Vec<_>>();
        self.detached_physical_commands
            .extend(detached_physical_commands);
        let highest_restored_id = snapshot
            .commands
            .iter()
            .map(|command| command.instance_id)
            .max()
            .unwrap_or(0);
        self.next_instance_id = self
            .next_instance_id
            .max(snapshot.next_instance_id)
            .max(highest_restored_id.saturating_add(1))
            .max(1);
        let mut entries: VecDeque<_> = snapshot
            .commands
            .iter()
            .cloned()
            .map(ActiveCommand::from_snapshot)
            .collect();
        for entry in &mut entries {
            if entry.instance_id == 0 {
                entry.instance_id = self.allocate_command_instance_id();
            }
        }
        self.entries = entries;
        self.detached_grab_attempts = snapshot.detached_grab_attempts.clone();
    }

    fn allocate_command_instance_id(&mut self) -> u64 {
        let id = self.next_instance_id.max(1);
        self.next_instance_id = id.checked_add(1).unwrap_or(1);
        id
    }

    /// C4Command::DenumeratePointers resolves the saved Target/Target2
    /// object numbers only after the complete object table has loaded
    /// (C4Command.cpp:2417-2421; C4Object.cpp:2914-2929).
    pub(crate) fn denumerate_object_references(&mut self, object_numbers: &HashSet<u64>) {
        for entry in &mut self.entries {
            if let Some(request) = &mut entry.request {
                denumerate_object_reference(&mut request.target, object_numbers);
                denumerate_object_reference(&mut request.target2, object_numbers);
                if let Some(value) = &mut request.tx_value {
                    *value = crate::denumerate_script_value(value, object_numbers);
                }
            }
            entry
                .state
                .denumerate_object_references(object_numbers, true);
        }
    }

    /// Save-time C4Command::EnumeratePointers/DenumeratePointers clears
    /// Target and Target2 wrappers which are outside Game.Objects, but Tx is
    /// an ordinary C4Value and is only compiled through ObjectNumber without
    /// mutating the live value.
    pub(crate) fn denumerate_compiled_pointer_fields(&mut self, object_numbers: &HashSet<u64>) {
        for entry in &mut self.entries {
            if let Some(request) = &mut entry.request {
                denumerate_object_reference(&mut request.target, object_numbers);
                denumerate_object_reference(&mut request.target2, object_numbers);
            }
            entry
                .state
                .denumerate_object_references(object_numbers, false);
        }
    }

    /// `C4Command::ClearPointers`: clear one removed object's references in
    /// both the serialized request fields and the live command state.
    pub(crate) fn clear_object_reference(&mut self, removed: ObjectId) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            if let Some(request) = &mut entry.request {
                changed |= clear_matching_object_reference(&mut request.target, removed);
                changed |= clear_matching_object_reference(&mut request.target2, removed);
                if let Some(value) = &mut request.tx_value {
                    changed |= clear_value_object_reference(value, removed);
                }
            }
            changed |= entry.state.clear_object_reference(removed);
        }
        // An iExec command detached by ClearCommands is no longer in
        // C4Object::Command, so the later ClearPointers walk does not reach
        // it (C4Object.cpp:2194-2205). Preserve those raw native pointers.
        changed
    }

    /// Execute the live front while retaining a finished entry for
    /// C4Object::ExecuteCommand's callback/clear tail.
    pub fn execute_front(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<CommandStepResult> {
        self.execute_front_with_gravity(ctx, crate::PhysicsSettings::default().gravity_as_c4fixed())
    }

    /// Engine execution supplies the live scenario gravity separately from
    /// the object/landscape command snapshots. Ballistic Throw and Put's
    /// throw-in preflight consume it; the public fixture seam above retains
    /// default-physics behavior.
    pub(crate) fn execute_front_with_gravity(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
    ) -> Option<CommandStepResult> {
        let next_is_move_to =
            self.entries.get(1).and_then(ActiveCommand::id) == Some(CommandId::MoveTo);
        let mut completed_command = None;
        let mut result = {
            let front = self.entries.front_mut()?;
            if front.finished.is_some() {
                return None;
            }
            let command = front.id();
            let result = front.step(ctx, gravity, next_is_move_to);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                front.finished = Some(result.status);
            }
            if result.status == CommandStatus::Completed {
                completed_command = command;
            }
            result
        };
        if let Some(command) = completed_command {
            self.record_native_success(command);
        }

        if result.status == CommandStatus::Failed {
            if let Some(mut feedback) = self.record_failure_at(0) {
                feedback.reason = result.failure_reason;
                // C4Command::Fail runs after the command handler has
                // returned, so preserve all handler-emitted event order.
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }

        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume the MoveTo whose live ObjectComStop event is in flight. This
    /// bypasses the ordinary front-step lifetime decrement: native C++ is
    /// still inside the same C4Command::Execute call. Action callbacks may
    /// have pushed another command above it, so locate the retained state
    /// rather than assuming it remains the stack front.
    pub(crate) fn execute_pending_move_to_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            matches!(
                &entry.state,
                CommandState::MoveTo(state) if state.stop_continuation.is_some()
            )
        });
        let mut result = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            let CommandState::MoveTo(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            result
        } else {
            // ClearCommands marks the executing native command for deletion
            // but does not interrupt its current MoveTo body. There is no
            // longer a live stack entry to finish; steering/child operations
            // still apply to the callback-installed stack.
            let mut state = self.detached_move_to_stops.pop_front()?;
            state.resume_after_stop(ctx)
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::MoveTo);
        }

        if result.status == CommandStatus::Failed {
            if let Some(index) = index {
                if let Some(feedback) = self.record_failure_at(index) {
                    result.events.push(CommandEvent::FailureFeedback {
                        actor_id: ctx.object.id,
                        feedback,
                    });
                }
            }
        }

        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Continue the exact MoveTo after FlightControl's ordinary Fly action
    /// and all of its callbacks. Only a WALK-origin continuation runs
    /// JumpControl; DFA_FLIGHT returns immediately after the action.
    pub(crate) fn execute_pending_move_to_flight(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        command_instance_id: u64,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::MoveTo(state) if state.flight_continuation.is_some()
                )
        });

        let (resumed_instance_id, mut result) = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            let resumed_instance_id = entry.instance_id;
            let CommandState::MoveTo(state) = &mut entry.state else {
                return None;
            };
            (resumed_instance_id, state.resume_after_flight_control(ctx))
        } else {
            let detached_index = self.detached_move_to_flights.iter().position(|detached| {
                (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                    && matches!(
                        &detached.entry.state,
                        CommandState::MoveTo(state) if state.flight_continuation.is_some()
                    )
            })?;
            let mut detached = self.detached_move_to_flights.remove(detached_index)?;
            let resumed_instance_id = detached.entry.instance_id;
            let CommandState::MoveTo(state) = &mut detached.entry.state else {
                return None;
            };
            (resumed_instance_id, state.resume_after_flight_control(ctx))
        };

        stamp_command_event_instances(&mut result.events, resumed_instance_id);
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume the Build whose Dig arm synchronously ran ObjectComStop.
    /// Callback-side ClearCommands may have detached the executing entry,
    /// but native C++ continues that same command object until Build returns.
    pub(crate) fn execute_pending_build_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        command_instance_id: u64,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Build(state) if state.stop_continuation
                )
        });
        let mut detached_failure = None;
        let resumed_instance_id;
        let mut result = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            resumed_instance_id = entry.instance_id;
            let CommandState::Build(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            result
        } else {
            let detached_index = self.detached_build_stops.iter().position(|detached| {
                (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                    && matches!(
                        &detached.entry.state,
                        CommandState::Build(state) if state.stop_continuation
                    )
            })?;
            let mut detached = self.detached_build_stops.remove(detached_index)?;
            resumed_instance_id = detached.entry.instance_id;
            let CommandState::Build(state) = &mut detached.entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx);
            if result.status == CommandStatus::Failed {
                detached.entry.finished = Some(CommandStatus::Failed);
                detached_failure = Some(detached);
            }
            result
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Build);
        }

        if result.status == CommandStatus::Failed {
            let feedback = if let Some(index) = index {
                self.record_failure_at(index)
            } else {
                detached_failure.as_ref().and_then(|detached| {
                    self.record_detached_failure(&detached.entry, &detached.base_chain)
                })
            };
            if let Some(mut feedback) = feedback {
                feedback.reason = result.failure_reason;
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }
        stamp_command_event_instances(&mut result.events, resumed_instance_id);
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume the exact Exit whose DFA_BUILD ObjectComStop just returned.
    /// The stop callback may have replaced the visible stack; native iExec
    /// still completes this retained command body against freshly read live
    /// containment and entrance state.
    pub(crate) fn execute_pending_exit_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        command_instance_id: u64,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Exit(state) if state.stop_continuation
                )
        });

        let mut detached_failure = None;
        let resumed_instance_id;
        let mut result = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            resumed_instance_id = entry.instance_id;
            let CommandState::Exit(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            result
        } else {
            let detached_index = self.detached_exit_preludes.iter().position(|detached| {
                (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                    && matches!(
                        &detached.entry.state,
                        CommandState::Exit(state) if state.stop_continuation
                    )
            })?;
            let mut detached = self.detached_exit_preludes.remove(detached_index)?;
            resumed_instance_id = detached.entry.instance_id;
            let CommandState::Exit(state) = &mut detached.entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx);
            if state.activation_pending != 0 {
                self.detached_exit_preludes.push_back(detached);
            } else if result.status == CommandStatus::Failed {
                detached_failure = Some((detached.entry, detached.base_chain));
            }
            result
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Exit);
        }
        if result.status == CommandStatus::Failed {
            let feedback = if let Some(index) = index {
                self.record_failure_at(index)
            } else if let Some((entry, base_chain)) = detached_failure.as_ref() {
                self.record_detached_failure(entry, base_chain)
            } else {
                None
            };
            if let Some(feedback) = feedback {
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }
        for event in &mut result.events {
            match event {
                CommandEvent::CommandExitObject {
                    command_instance_id: event_instance_id,
                    ..
                }
                | CommandEvent::CommandExitIntoParent {
                    command_instance_id: event_instance_id,
                    ..
                }
                | CommandEvent::ActivateEntrance {
                    command_instance_id: event_instance_id,
                    ..
                } if *event_instance_id == 0 => {
                    // A zero input token is the persisted-event fallback, not
                    // permission to leave the resumed event ambiguous. Pin
                    // the actual retained command so a callback-installed
                    // replacement Exit cannot be finished instead.
                    *event_instance_id = resumed_instance_id;
                }
                _ => {}
            }
        }
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume the exact Throw body suspended across ObjectComStop or
    /// SetDir. This is still the same native Execute call, so neither the
    /// command interval nor retry bookkeeping advances a second time.
    pub(crate) fn execute_pending_throw_prelude(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        command_instance_id: u64,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Throw(state) if !state.continuations.is_empty()
                )
        });

        let mut detached_failure = None;
        let resumed_instance_id;
        let mut result = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            resumed_instance_id = entry.instance_id;
            let CommandState::Throw(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_prelude(ctx, gravity);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            result
        } else {
            let detached_index = self.detached_throw_preludes.iter().position(|entry| {
                (command_instance_id == 0 || entry.entry.instance_id == command_instance_id)
                    && matches!(
                        &entry.entry.state,
                        CommandState::Throw(state) if !state.continuations.is_empty()
                    )
            })?;
            let mut detached = self.detached_throw_preludes.remove(detached_index)?;
            resumed_instance_id = detached.entry.instance_id;
            let CommandState::Throw(state) = &mut detached.entry.state else {
                return None;
            };
            let result = state.resume_after_prelude(ctx, gravity);
            if !state.continuations.is_empty() {
                self.detached_throw_preludes.push_back(detached);
            } else if result.status == CommandStatus::Failed {
                detached_failure = Some((detached.entry, detached.base_chain));
            }
            result
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Throw);
        }

        if result.status == CommandStatus::Failed {
            if let Some(index) = index {
                if let Some(feedback) = self.record_failure_at(index) {
                    result.events.push(CommandEvent::FailureFeedback {
                        actor_id: ctx.object.id,
                        feedback,
                    });
                }
            } else if let Some((entry, base_chain)) = detached_failure.as_ref() {
                if let Some(feedback) = self.record_detached_failure(entry, base_chain) {
                    result.events.push(CommandEvent::FailureFeedback {
                        actor_id: ctx.object.id,
                        feedback,
                    });
                }
            }
        }
        for event in &mut result.events {
            match event {
                CommandEvent::ObjectComPutTake {
                    command_instance_id: event_instance_id,
                    ..
                }
                | CommandEvent::ThrowObject {
                    command_instance_id: event_instance_id,
                    ..
                }
                | CommandEvent::ObjectComStopThrow {
                    command_instance_id: event_instance_id,
                    ..
                }
                | CommandEvent::ObjectComSetDirThrow {
                    command_instance_id: event_instance_id,
                    ..
                } if *event_instance_id == 0 => {
                    *event_instance_id = resumed_instance_id;
                }
                _ => {}
            }
        }
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume the exact Drop body suspended across its initial
    /// ObjectComStop. This is still the same native Execute call, so the
    /// command interval and retry bookkeeping must not advance again.
    pub(crate) fn execute_pending_drop_prelude(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        command_instance_id: u64,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Drop(state) if !state.continuations.is_empty()
                )
        });

        let mut detached_failure = None;
        let resumed_instance_id;
        let mut result = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            resumed_instance_id = entry.instance_id;
            let CommandState::Drop(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_prelude(ctx);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            result
        } else {
            let detached_index = self.detached_drop_preludes.iter().position(|entry| {
                (command_instance_id == 0 || entry.entry.instance_id == command_instance_id)
                    && matches!(
                        &entry.entry.state,
                        CommandState::Drop(state) if !state.continuations.is_empty()
                    )
            })?;
            let mut detached = self.detached_drop_preludes.remove(detached_index)?;
            resumed_instance_id = detached.entry.instance_id;
            let CommandState::Drop(state) = &mut detached.entry.state else {
                return None;
            };
            let result = state.resume_after_prelude(ctx);
            if !state.continuations.is_empty() {
                self.detached_drop_preludes.push_back(detached);
            } else if result.status == CommandStatus::Failed {
                detached_failure = Some((detached.entry, detached.base_chain));
            }
            result
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Drop);
        }

        if result.status == CommandStatus::Failed {
            if let Some(index) = index {
                if let Some(feedback) = self.record_failure_at(index) {
                    result.events.push(CommandEvent::FailureFeedback {
                        actor_id: ctx.object.id,
                        feedback,
                    });
                }
            } else if let Some((entry, base_chain)) = detached_failure.as_ref() {
                if let Some(feedback) = self.record_detached_failure(entry, base_chain) {
                    result.events.push(CommandEvent::FailureFeedback {
                        actor_id: ctx.object.id,
                        feedback,
                    });
                }
            }
        }
        for event in &mut result.events {
            match event {
                CommandEvent::ObjectComPutTake {
                    command_instance_id: event_instance_id,
                    ..
                }
                | CommandEvent::ObjectComDrop {
                    command_instance_id: event_instance_id,
                    ..
                } if *event_instance_id == 0 => {
                    *event_instance_id = resumed_instance_id;
                }
                _ => {}
            }
        }
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume Put after its DFA_DIG ObjectComStop. The callback may have
    /// detached the executing command; native iExec still continues its
    /// post-stop body against fresh pGrabbing, containment and physicals.
    pub(crate) fn execute_pending_put_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        command_instance_id: u64,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Put(state) if state.stop_continuation.is_some()
                )
        });

        let mut detached = None;
        let (resumed_instance_id, mut result) = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            let resumed_instance_id = entry.instance_id;
            let CommandState::Put(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx, gravity);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            (resumed_instance_id, result)
        } else {
            let detached_index = self.detached_put_stops.iter().position(|detached| {
                (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                    && matches!(
                        &detached.entry.state,
                        CommandState::Put(state) if state.stop_continuation.is_some()
                    )
            })?;
            let mut retained = self.detached_put_stops.remove(detached_index)?;
            let resumed_instance_id = retained.entry.instance_id;
            let CommandState::Put(state) = &mut retained.entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx, gravity);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                retained.entry.finished = Some(result.status);
            }
            detached = Some(retained);
            (resumed_instance_id, result)
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Put);
        }
        if result.status == CommandStatus::Failed {
            let feedback = if let Some(index) = index {
                self.record_failure_at(index)
            } else {
                detached.as_ref().and_then(|detached| {
                    self.record_detached_failure(&detached.entry, &detached.base_chain)
                })
            };
            if let Some(feedback) = feedback {
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }
        stamp_command_event_instances(&mut result.events, resumed_instance_id);

        if result.status == CommandStatus::Running {
            if let Some(detached) = detached {
                if detached.entry.state.has_physical_continuation() {
                    self.detached_physical_commands
                        .push_back(DetachedPhysicalCommand {
                            entry: detached.entry,
                            base_chain: detached.base_chain,
                        });
                } else if matches!(
                    &detached.entry.state,
                    CommandState::Put(state) if state.put_pending
                ) {
                    self.detached_put_attempts.push_back(DetachedPutAttempt {
                        entry: detached.entry,
                        base_chain: detached.base_chain,
                    });
                }
            }
        }

        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume Construct after callbackful ObjectComStop without repeating
    /// its physical/definition/knowledge gates or command interval.
    pub(crate) fn execute_pending_construct_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        command_instance_id: u64,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Construct(state) if state.stop_continuation
                )
        });

        let mut detached = None;
        let (resumed_instance_id, mut result) = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            let resumed_instance_id = entry.instance_id;
            let CommandState::Construct(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            (resumed_instance_id, result)
        } else {
            let detached_index = self
                .detached_construct_commands
                .iter()
                .position(|detached| {
                    (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                        && matches!(
                            &detached.entry.state,
                            CommandState::Construct(state) if state.stop_continuation
                        )
                })?;
            let mut retained = self.detached_construct_commands.remove(detached_index)?;
            let resumed_instance_id = retained.entry.instance_id;
            let CommandState::Construct(state) = &mut retained.entry.state else {
                return None;
            };
            let result = state.resume_after_stop(ctx);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                retained.entry.finished = Some(result.status);
            }
            detached = Some(retained);
            (resumed_instance_id, result)
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Construct);
        }
        if result.status == CommandStatus::Failed {
            let feedback = if let Some(index) = index {
                self.record_failure_at(index)
            } else {
                detached.as_ref().and_then(|detached| {
                    self.record_detached_failure(&detached.entry, &detached.base_chain)
                })
            };
            if let Some(feedback) = feedback {
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }
        stamp_command_event_instances(&mut result.events, resumed_instance_id);

        if result.status == CommandStatus::Running {
            if let Some(detached) = detached {
                if matches!(
                    &detached.entry.state,
                    CommandState::Construct(state)
                        if state.script_pending
                            || (state.spawn_requested && state.construction_id.is_none())
                ) {
                    self.detached_construct_commands.push_back(detached);
                }
            }
        }
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Continue the exact Construct immediately after its script overload
    /// returns. Result zero falls through to conkit/range/check/spawn in the
    /// same Execute; callback-detached commands retain their native base.
    pub(crate) fn execute_pending_construct_script(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        command_instance_id: u64,
        script_result: AcquireScriptResult,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Construct(state) if state.script_pending
                )
        });

        let mut detached = None;
        let (resumed_instance_id, mut result) = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            let resumed_instance_id = entry.instance_id;
            let CommandState::Construct(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_script(ctx, script_result);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            (resumed_instance_id, result)
        } else {
            let detached_index = self
                .detached_construct_commands
                .iter()
                .position(|detached| {
                    (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                        && matches!(
                            &detached.entry.state,
                            CommandState::Construct(state) if state.script_pending
                        )
                })?;
            let mut retained = self.detached_construct_commands.remove(detached_index)?;
            let resumed_instance_id = retained.entry.instance_id;
            let CommandState::Construct(state) = &mut retained.entry.state else {
                return None;
            };
            let result = state.resume_after_script(ctx, script_result);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                retained.entry.finished = Some(result.status);
            }
            detached = Some(retained);
            (resumed_instance_id, result)
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Construct);
        }
        if result.status == CommandStatus::Failed {
            let feedback = if let Some(index) = index {
                self.record_failure_at(index)
            } else {
                detached.as_ref().and_then(|detached| {
                    self.record_detached_failure(&detached.entry, &detached.base_chain)
                })
            };
            if let Some(feedback) = feedback {
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }
        stamp_command_event_instances(&mut result.events, resumed_instance_id);
        if result.status == CommandStatus::Running {
            if let Some(detached) = detached {
                if matches!(
                    &detached.entry.state,
                    CommandState::Construct(state)
                        if state.spawn_requested && state.construction_id.is_none()
                ) {
                    self.detached_construct_commands.push_back(detached);
                }
            }
        }
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Finish the exact Construct after its validated object was created and
    /// the conkit consumed, then add Build before returning from Execute.
    pub(crate) fn execute_pending_construct_spawn(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        command_instance_id: u64,
        construction_id: Option<ObjectId>,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Construct(state)
                        if state.spawn_requested && state.construction_id.is_none()
                )
        });

        let mut detached = None;
        let (resumed_instance_id, mut result) = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            let resumed_instance_id = entry.instance_id;
            let CommandState::Construct(state) = &mut entry.state else {
                return None;
            };
            let result = state.resume_after_spawn(ctx, construction_id);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            (resumed_instance_id, result)
        } else {
            let detached_index = self
                .detached_construct_commands
                .iter()
                .position(|detached| {
                    (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                        && matches!(
                            &detached.entry.state,
                            CommandState::Construct(state)
                                if state.spawn_requested && state.construction_id.is_none()
                        )
                })?;
            let mut retained = self.detached_construct_commands.remove(detached_index)?;
            let resumed_instance_id = retained.entry.instance_id;
            let CommandState::Construct(state) = &mut retained.entry.state else {
                return None;
            };
            let result = state.resume_after_spawn(ctx, construction_id);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                retained.entry.finished = Some(result.status);
            }
            detached = Some(retained);
            (resumed_instance_id, result)
        };

        if result.status == CommandStatus::Completed {
            self.record_native_success(CommandId::Construct);
        }
        if result.status == CommandStatus::Failed {
            let feedback = if let Some(index) = index {
                self.record_failure_at(index)
            } else {
                detached.as_ref().and_then(|detached| {
                    self.record_detached_failure(&detached.entry, &detached.base_chain)
                })
            };
            if let Some(feedback) = feedback {
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }
        stamp_command_event_instances(&mut result.events, resumed_instance_id);
        self.apply_result_operations(&mut result);
        Some(result)
    }

    /// Resume the exact native command suspended at a first fair-crew
    /// GetPhysical callback. The hook may have replaced the visible stack;
    /// in that case the retained iExec body still completes against fresh
    /// runtime snapshots without consuming another command interval.
    pub(crate) fn execute_pending_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        command_instance_id: u64,
        physical: PhysicalInfo,
    ) -> Option<CommandStepResult> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && entry.state.has_physical_continuation()
        });

        let mut detached = None;
        let (resumed_instance_id, command, mut result) = if let Some(index) = index {
            let entry = self.entries.get_mut(index)?;
            let resumed_instance_id = entry.instance_id;
            let command = entry.id();
            let result = entry.state.resume_after_physical(ctx, gravity, physical);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                entry.finished = Some(result.status);
            }
            (resumed_instance_id, command, result)
        } else {
            let detached_index = self
                .detached_physical_commands
                .iter()
                .position(|detached| {
                    (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                        && detached.entry.state.has_physical_continuation()
                })?;
            let mut retained = self.detached_physical_commands.remove(detached_index)?;
            let resumed_instance_id = retained.entry.instance_id;
            let command = retained.entry.id();
            let result = retained
                .entry
                .state
                .resume_after_physical(ctx, gravity, physical);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                retained.entry.finished = Some(result.status);
            }
            detached = Some(retained);
            (resumed_instance_id, command, result)
        };

        if result.status == CommandStatus::Completed {
            if let Some(command) = command {
                self.record_native_success(command);
            }
        }
        if result.status == CommandStatus::Failed {
            let feedback = if let Some(index) = index {
                self.record_failure_at(index)
            } else {
                detached.as_ref().and_then(|detached| {
                    self.record_detached_failure(&detached.entry, &detached.base_chain)
                })
            };
            if let Some(mut feedback) = feedback {
                feedback.reason = result.failure_reason;
                result.events.push(CommandEvent::FailureFeedback {
                    actor_id: ctx.object.id,
                    feedback,
                });
            }
        }
        stamp_command_event_instances(&mut result.events, resumed_instance_id);

        if result.status == CommandStatus::Running {
            if let Some(detached) = detached {
                if matches!(
                    &detached.entry.state,
                    CommandState::MoveTo(state) if state.flight_continuation.is_some()
                ) {
                    self.detached_move_to_flights
                        .push_back(DetachedMoveToFlight {
                            entry: detached.entry,
                            base_chain: detached.base_chain,
                        });
                } else if matches!(
                    &detached.entry.state,
                    CommandState::Throw(state) if !state.continuations.is_empty()
                ) {
                    self.detached_throw_preludes
                        .push_back(DetachedThrowPrelude {
                            entry: detached.entry,
                            base_chain: detached.base_chain,
                        });
                } else if matches!(
                    &detached.entry.state,
                    CommandState::Put(state) if state.put_pending
                ) {
                    self.detached_put_attempts.push_back(DetachedPutAttempt {
                        entry: detached.entry,
                        base_chain: detached.base_chain,
                    });
                } else if let CommandState::Build(state) = &detached.entry.state {
                    if state.stop_continuation {
                        self.detached_build_stops.push_back(DetachedBuildStop {
                            entry: detached.entry,
                            base_chain: detached.base_chain,
                        });
                    }
                } else if matches!(
                    &detached.entry.state,
                    CommandState::Construct(state)
                        if state.stop_continuation
                            || state.script_pending
                            || (state.spawn_requested && state.construction_id.is_none())
                ) {
                    self.detached_construct_commands
                        .push_back(DetachedConstructCommand {
                            entry: detached.entry,
                            base_chain: detached.base_chain,
                        });
                }
            }
        }

        self.apply_result_operations(&mut result);
        Some(result)
    }

    pub(in crate::command) fn apply_result_operations(&mut self, result: &mut CommandStepResult) {
        for operation in std::mem::take(&mut result.operations) {
            match operation {
                CommandOperation::Clear => self.clear(),
                CommandOperation::PushFront(request) => {
                    let _ = self.push_front(request);
                }
                CommandOperation::PushBack(request) => {
                    let _ = self.push_back(request);
                }
                CommandOperation::Finish { index, success } => {
                    self.finish_entry(index, success);
                }
                CommandOperation::DecrementNoCollectDelay => {}
                CommandOperation::SetNoCollectDelay { .. } => {}
                CommandOperation::Restore(snapshot) => self.restore_from_snapshot(&snapshot),
            }
        }
    }

    /// The finished command C4Object::ExecuteCommand exposes to
    /// `~ControlCommandFinished` before clearing it.
    pub fn finished_front_view(&self) -> Option<CommandView> {
        self.entries.front().and_then(|entry| {
            entry.finished.map(|_| {
                CommandView::from_entry(
                    entry
                        .state
                        .id()
                        .map(CommandId::to_name)
                        .unwrap_or("None")
                        .to_string(),
                    entry.request.as_ref(),
                    &entry.state,
                    entry.finished.is_some(),
                )
            })
        })
    }

    /// C4Object::ExecuteCommand clears every finished stack front after
    /// the callback, including finished commands uncovered by the first
    /// removal.
    pub fn clear_finished_fronts(&mut self) {
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.finished.is_some())
        {
            self.pop_front();
        }
    }

    pub fn push_front(&mut self, request: CommandRequest) -> Result<(), CommandError> {
        if self.entries.len() >= MAX_COMMAND_STACK {
            return Err(CommandError::StackFull);
        }
        let mut command = ActiveCommand::from_request(request)?;
        command.instance_id = self.allocate_command_instance_id();
        self.entries.push_front(command);
        Ok(())
    }

    pub fn push_back(&mut self, request: CommandRequest) -> Result<(), CommandError> {
        if self.entries.len() >= MAX_COMMAND_STACK {
            return Err(CommandError::StackFull);
        }
        let mut command = ActiveCommand::from_request(request)?;
        command.instance_id = self.allocate_command_instance_id();
        self.entries.push_back(command);
        Ok(())
    }

    pub fn complete_front_if(&mut self, id: CommandId) -> bool {
        if let Some(front) = self.entries.front() {
            if front.id() == Some(id) {
                self.pop_front();
                return true;
            }
        }
        false
    }

    /// Mark the matching front as successfully finished without clearing it.
    /// Live command events use this so `ControlCommandFinished` still sees
    /// the command after the event's synchronous callbacks return. Calling
    /// this means the in-flight native command reached `Finish(true)` even
    /// when a callback detached or pre-finished its stack entry.
    pub fn finish_front_if(&mut self, id: CommandId) -> bool {
        self.record_native_success(id);
        if let Some(front) = self.entries.front_mut() {
            if front.id() == Some(id) {
                front.finished = Some(CommandStatus::Completed);
                return true;
            }
        }
        false
    }

    /// Finish the exact native C4Command whose callbackful helper returned.
    /// A zero token is the compatibility fallback for persisted legacy
    /// events, which had no runtime pointer identity.
    pub(crate) fn finish_command_instance(
        &mut self,
        id: CommandId,
        command_instance_id: u64,
    ) -> bool {
        // The event itself retains the executing native command's lifetime;
        // the live stack entry is optional after callback-side replacement.
        self.record_native_success(id);
        let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.id() == Some(id)
                && (command_instance_id == 0 || entry.instance_id == command_instance_id)
        }) else {
            return false;
        };
        entry.finished = Some(CommandStatus::Completed);
        true
    }

    /// Finish the exact Drop whose live ObjectComDrop helper just returned.
    /// AddCommand may have pushed entries above it; SetCommand may have
    /// removed it entirely. The helper return still reaches native
    /// `Finish(true)`, so success is recorded independently of attachment.
    pub fn finish_pending_drop(&mut self, command_instance_id: u64) -> bool {
        self.record_native_success(CommandId::Drop);
        let Some(entry) = self.entries.iter_mut().find(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Drop(state) if state.completion_pending
                )
        }) else {
            return false;
        };
        let CommandState::Drop(state) = &mut entry.state else {
            unreachable!("the pending-drop predicate only matches Drop commands");
        };
        state.completion_pending = false;
        entry.finished = Some(CommandStatus::Completed);
        true
    }

    /// Resolve the live ObjectComDig boundary against the exact command
    /// instance which emitted it. A successful SetAction callback may unlink
    /// the command; both failure exits precede callbacks and remain attached.
    pub(crate) fn resolve_dig_attempt(
        &mut self,
        command_instance_id: u64,
        succeeded: bool,
    ) -> Option<CommandFailureFeedback> {
        if let Some(index) = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(&entry.state, CommandState::Dig(state) if state.start_pending)
        }) {
            if let CommandState::Dig(state) = &mut self.entries[index].state {
                state.start_pending = false;
            }
            if succeeded {
                return None;
            }
            self.entries[index].finished = Some(CommandStatus::Failed);
            return self.record_failure_at(index);
        }

        // Both false exits happen before SetAction begins callbacks, so a
        // missing exact instance can only be a callback-detached success.
        None
    }

    /// Finish the exact Throw whose live ObjectComPutTake helper returned.
    /// Callback-side AddCommand may have pushed another entry above it, and
    /// SetCommand may have removed it entirely. The helper return still
    /// reaches native `Finish(true)` in either case.
    pub(crate) fn finish_pending_throw(&mut self, command_instance_id: u64) -> bool {
        self.record_native_success(CommandId::Throw);
        let Some(entry) = self.entries.iter_mut().find(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Throw(state) if state.put_take_pending
                )
        }) else {
            return false;
        };
        let CommandState::Throw(state) = &mut entry.state else {
            unreachable!("the pending-throw predicate only matches Throw commands");
        };
        state.put_take_pending = false;
        entry.finished = Some(CommandStatus::Completed);
        true
    }

    /// A live requested item moved out of the actor after the command
    /// snapshot was built. Clear the in-flight PutTake marker so a Get child
    /// can run and the same original command can retry afterward.
    pub(crate) fn clear_pending_put_take(
        &mut self,
        command: CommandId,
        command_instance_id: u64,
    ) -> bool {
        let entry = match command {
            CommandId::Throw => self.entries.iter_mut().find(|entry| {
                (command_instance_id == 0 || entry.instance_id == command_instance_id)
                    && matches!(
                        &entry.state,
                        CommandState::Throw(state) if state.put_take_pending
                    )
            }),
            CommandId::Drop => self.entries.iter_mut().find(|entry| {
                (command_instance_id == 0 || entry.instance_id == command_instance_id)
                    && matches!(
                        &entry.state,
                        CommandState::Drop(state) if state.completion_pending
                    )
            }),
            _ => None,
        };
        let Some(entry) = entry else {
            return false;
        };
        match &mut entry.state {
            CommandState::Throw(state) => state.put_take_pending = false,
            CommandState::Drop(state) => state.completion_pending = false,
            _ => unreachable!("pending PutTake predicate matched another command"),
        }
        true
    }

    /// Apply the legacy failed-result adjustment to the exact command which
    /// emitted a callback event. A restored zero token keeps the historical
    /// first-same-kind fallback, but runtime events cannot increment a
    /// callback-installed replacement command.
    pub(crate) fn fail_command_instance(
        &mut self,
        id: CommandId,
        command_instance_id: u64,
    ) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.id() == Some(id)
                && (command_instance_id == 0 || entry.instance_id == command_instance_id)
        }) else {
            return false;
        };
        entry.failures = entry.failures.saturating_add(1);
        true
    }

    pub fn fail_front_if(&mut self, id: CommandId) -> bool {
        if let Some(front) = self.entries.front_mut() {
            if front.id() == Some(id) {
                front.failures = front.failures.saturating_add(1);
                return true;
            }
        }
        false
    }

    /// RejectGrabbed's truthy result calls Finish(false) immediately. A
    /// direct command has no unfinished command behind it and is made a
    /// SilentBase first so the expected veto does not report a failure
    /// (C4Command.cpp:697-703).
    /// Returns whether the marked command still retains its target pointer;
    /// `None` means callback-side command replacement removed the attempt.
    pub fn resolve_grab_attempt(&mut self, target: ObjectId, rejected: bool) -> Option<bool> {
        let Some(index) = self.entries.iter().position(|entry| {
            matches!(
                &entry.state,
                CommandState::Grab(state)
                    if state.target == target && state.reject_pending
            )
        }) else {
            if let Some(index) = self
                .detached_grab_attempts
                .iter()
                .rposition(|attempt| attempt.target == target)
            {
                return Some(self.detached_grab_attempts.remove(index).target_retained);
            }
            return None;
        };
        let state_retained = matches!(
            &self.entries[index].state,
            CommandState::Grab(state) if !state.target_cleared
        );
        let request_retained = self.entries[index]
            .request
            .as_ref()
            .is_none_or(|request| request.target == Some(target));
        let target_retained = state_retained && request_retained;
        if !rejected {
            if let CommandState::Grab(state) = &mut self.entries[index].state {
                state.reject_pending = false;
            }
            return Some(target_retained);
        }
        let direct = self
            .entries
            .iter()
            .skip(index + 1)
            .all(|entry| entry.finished.is_some());
        {
            let command = &mut self.entries[index];
            if direct {
                command.mode = CommandMode::SilentBase;
            }
            if let CommandState::Grab(state) = &mut command.state {
                state.reject_pending = false;
            }
            command.finished = Some(CommandStatus::Failed);
        }
        if let Some(feedback) = self.record_failure_at(index) {
            self.pending_failure_feedback.push_back(feedback);
        }
        Some(target_retained)
    }

    /// Tx/Ty belong to the exact Grab command that armed AttemptGrab. Read
    /// them before live callbacks can replace or detach that command.
    pub(crate) fn pending_grab_offsets(&self, target: ObjectId) -> Option<(i32, i32)> {
        self.entries.iter().find_map(|entry| {
            let CommandState::Grab(state) = &entry.state else {
                return None;
            };
            (state.target == target && state.reject_pending)
                .then_some((state.offset_x, state.offset_y))
        })
    }

    /// ObjectComStop callbacks run before C4Command::Grab's null-target
    /// check. If ClearPointers reached the executing command before a
    /// callback detached it, finish that exact Grab with ordinary failure
    /// semantics. A command detached first retains its raw C++ pointer even
    /// when the target's Status subsequently becomes zero.
    pub(crate) fn fail_pending_grab_if_target_cleared(&mut self, target: ObjectId) -> bool {
        let Some(index) = self.entries.iter().position(|entry| {
            matches!(
                &entry.state,
                CommandState::Grab(state)
                    if state.target == target && state.reject_pending
            )
        }) else {
            if let Some(index) = self
                .detached_grab_attempts
                .iter()
                .rposition(|attempt| attempt.target == target)
            {
                if self.detached_grab_attempts[index].target_retained {
                    return false;
                }
                self.detached_grab_attempts.remove(index);
                return true;
            }
            return false;
        };
        let state_retained = matches!(
            &self.entries[index].state,
            CommandState::Grab(state) if !state.target_cleared
        );
        let request_retained = self.entries[index]
            .request
            .as_ref()
            .is_none_or(|request| request.target == Some(target));
        if state_retained && request_retained {
            return false;
        }

        {
            let command = &mut self.entries[index];
            if let CommandState::Grab(state) = &mut command.state {
                state.reject_pending = false;
            }
            command.finished = Some(CommandStatus::Failed);
        }
        if let Some(feedback) = self.record_failure_at(index) {
            self.pending_failure_feedback.push_back(feedback);
        }
        true
    }

    /// Read the emitting Get's live Target after a callback may have run
    /// `Game.ClearPointers`. Detachment freezes the pointer state at that
    /// instant: an earlier clear remains null, while a later clear cannot
    /// reach the unlinked native iExec command.
    pub(crate) fn get_event_target_after_callback(
        &self,
        command_instance_id: u64,
        captured_target: ObjectId,
    ) -> Option<ObjectId> {
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.instance_id == command_instance_id)
        {
            if let CommandState::Get(state) = &entry.state {
                return state.target;
            }
        }
        if let Some(detached) = self
            .detached_get_attempts
            .iter()
            .find(|attempt| attempt.entry.instance_id == command_instance_id)
        {
            if let CommandState::Get(state) = &detached.entry.state {
                return state.target;
            }
        }
        Some(captured_target)
    }

    /// Resolve only the Get command which emitted the live GetObject event.
    /// Callback-side SetCommand may have removed it and installed another
    /// Get, which must not inherit the old attempt's result.
    pub(crate) fn resolve_get_attempt(
        &mut self,
        command_instance_id: u64,
        disposition: GetAttemptDisposition,
    ) -> Option<GetAttemptResolution> {
        if disposition == GetAttemptDisposition::Complete {
            // Get's callbackful collection attempt has returned to the
            // still-executing native command even if scripts replaced its
            // stack entry in the meantime.
            self.record_native_success(CommandId::Get);
        }
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(&entry.state, CommandState::Get(state) if state.enter_pending)
        });
        if let Some(index) = index {
            if let CommandState::Get(state) = &mut self.entries[index].state {
                state.enter_pending = false;
            }

            let mut feedback = None;
            match disposition {
                GetAttemptDisposition::Continue => {}
                GetAttemptDisposition::Complete => {
                    self.entries[index].finished = Some(CommandStatus::Completed);
                }
                GetAttemptDisposition::Fail => {
                    self.entries[index].finished = Some(CommandStatus::Failed);
                    feedback = self.record_failure_at(index);
                }
            }
            return Some(GetAttemptResolution { feedback });
        }

        let detached_index = if command_instance_id == 0 {
            self.detached_get_attempts.iter().rposition(|attempt| {
                matches!(&attempt.entry.state, CommandState::Get(state) if state.enter_pending)
            })
        } else {
            self.detached_get_attempts.iter().position(|attempt| {
                attempt.entry.instance_id == command_instance_id
                    && matches!(&attempt.entry.state, CommandState::Get(state) if state.enter_pending)
            })
        }?;
        let mut detached = self.detached_get_attempts.remove(detached_index)?;
        if let CommandState::Get(state) = &mut detached.entry.state {
            state.enter_pending = false;
        }
        let feedback = match disposition {
            GetAttemptDisposition::Continue => None,
            GetAttemptDisposition::Complete => {
                detached.entry.finished = Some(CommandStatus::Completed);
                None
            }
            GetAttemptDisposition::Fail => {
                detached.entry.finished = Some(CommandStatus::Failed);
                self.record_detached_failure(&detached.entry, &detached.base_chain)
            }
        };
        Some(GetAttemptResolution { feedback })
    }

    fn pending_buy_index(&self, base: ObjectId, definition_id: &str) -> Option<usize> {
        self.entries.iter().position(|entry| {
            matches!(
                &entry.state,
                CommandState::Buy(state)
                    if state.evaluation_pending
                        && state.target == Some(base)
                        && state.definition_id == definition_id
            )
        })
    }

    /// Buy's GetValue preflight completed and the actor must enter the base
    /// before the same command retries. Callback-installed replacement
    /// commands do not inherit the old evaluation's pending marker.
    pub(crate) fn defer_pending_buy_for_enter(
        &mut self,
        base: ObjectId,
        definition_id: &str,
    ) -> bool {
        let Some(index) = self.pending_buy_index(base, definition_id) else {
            return false;
        };
        if let CommandState::Buy(state) = &mut self.entries[index].state {
            state.evaluation_pending = false;
        }
        true
    }

    /// C4Command::Buy normalizes Tx only after its containment check. This
    /// happens before C4Player::Buy callbacks, so FnGetCommand observes the
    /// normalized live count during Purchase/Recruitment.
    pub(crate) fn normalize_pending_buy_count(
        &mut self,
        base: ObjectId,
        definition_id: &str,
    ) -> Option<i32> {
        let index = self.pending_buy_index(base, definition_id)?;
        let CommandState::Buy(state) = &mut self.entries[index].state else {
            unreachable!("the pending-buy predicate only matches Buy commands");
        };
        state.remaining_count = state.remaining_count.max(1);
        Some(state.remaining_count)
    }

    /// One synchronous C4Player::Buy iteration succeeded. Native decrements
    /// Tx after Purchase and Enter, leaving earlier purchases committed if a
    /// later iteration fails.
    pub(crate) fn record_pending_buy_success(
        &mut self,
        base: ObjectId,
        definition_id: &str,
    ) -> bool {
        let Some(index) = self.pending_buy_index(base, definition_id) else {
            return false;
        };
        let CommandState::Buy(state) = &mut self.entries[index].state else {
            unreachable!("the pending-buy predicate only matches Buy commands");
        };
        state.remaining_count = state.remaining_count.saturating_sub(1);
        true
    }

    /// Finish only the Buy command that emitted EvaluateBuy. Pricing and
    /// Purchase callbacks may have replaced the stack in the meantime.
    pub(crate) fn resolve_pending_buy(
        &mut self,
        base: ObjectId,
        definition_id: &str,
        succeeded: bool,
    ) -> Option<BuyAttemptResolution> {
        if succeeded {
            self.record_native_success(CommandId::Buy);
        }
        let index = self.pending_buy_index(base, definition_id)?;
        if let CommandState::Buy(state) = &mut self.entries[index].state {
            state.evaluation_pending = false;
        }
        self.entries[index].finished = Some(if succeeded {
            CommandStatus::Completed
        } else {
            CommandStatus::Failed
        });
        let feedback = (!succeeded)
            .then(|| self.record_failure_at(index))
            .flatten();
        Some(BuyAttemptResolution { feedback })
    }

    fn pending_sell_index(&self, base: ObjectId, definition_id: &str) -> Option<usize> {
        self.entries.iter().position(|entry| {
            matches!(
                &entry.state,
                CommandState::Sell(state)
                    if state.evaluation_pending
                        && (state.target == Some(base) || state.target.is_none())
                        && state.definition_id == definition_id
            )
        })
    }

    /// C4Command::Sell normalizes Tx only after containment succeeds and
    /// immediately before the synchronous SellFromBase loop.
    pub(crate) fn normalize_pending_sell_count(
        &mut self,
        base: ObjectId,
        definition_id: &str,
    ) -> Option<i32> {
        let index = self.pending_sell_index(base, definition_id)?;
        let CommandState::Sell(state) = &mut self.entries[index].state else {
            unreachable!("the pending-sell predicate only matches Sell commands");
        };
        state.remaining = state.remaining.max(1);
        Some(state.remaining)
    }

    /// One SellFromBase iteration succeeded. Target2 is preferred once,
    /// then C++ clears it before selecting the next item by Data.
    pub(crate) fn record_pending_sell_success(
        &mut self,
        base: ObjectId,
        definition_id: &str,
    ) -> bool {
        let Some(index) = self.pending_sell_index(base, definition_id) else {
            return false;
        };
        let CommandState::Sell(state) = &mut self.entries[index].state else {
            unreachable!("the pending-sell predicate only matches Sell commands");
        };
        state.preferred = None;
        state.remaining = state.remaining.saturating_sub(1);
        true
    }

    /// Finish only the Sell command which emitted EvaluateSell. Sale hooks
    /// may have replaced the command stack while prior sales stay committed.
    pub(crate) fn resolve_pending_sell(
        &mut self,
        base: ObjectId,
        definition_id: &str,
        succeeded: bool,
    ) -> Option<SellAttemptResolution> {
        if succeeded {
            self.record_native_success(CommandId::Sell);
        }
        let index = self.pending_sell_index(base, definition_id)?;
        if let CommandState::Sell(state) = &mut self.entries[index].state {
            state.evaluation_pending = false;
        }
        self.entries[index].finished = Some(if succeeded {
            CommandStatus::Completed
        } else {
            CommandStatus::Failed
        });
        let feedback = (!succeeded)
            .then(|| self.record_failure_at(index))
            .flatten();
        Some(SellAttemptResolution { feedback })
    }

    /// Resolve only the Put command which emitted ObjectComPut. Collection
    /// callbacks may have replaced or reordered the command stack while the
    /// helper ran, so a plain front-command check is insufficient.
    pub(crate) fn resolve_put_attempt(
        &mut self,
        command_instance_id: u64,
        succeeded: bool,
    ) -> Option<CommandFailureFeedback> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(&entry.state, CommandState::Put(state) if state.put_pending)
        });
        if let Some(index) = index {
            if let CommandState::Put(state) = &mut self.entries[index].state {
                state.put_pending = false;
            }
            if succeeded {
                return None;
            }

            self.entries[index].finished = Some(CommandStatus::Failed);
            return self.record_failure_at(index);
        }

        let detached_index = if command_instance_id == 0 {
            self.detached_put_attempts.iter().rposition(|attempt| {
                matches!(&attempt.entry.state, CommandState::Put(state) if state.put_pending)
            })
        } else {
            self.detached_put_attempts.iter().position(|attempt| {
                attempt.entry.instance_id == command_instance_id
                    && matches!(&attempt.entry.state, CommandState::Put(state) if state.put_pending)
            })
        }?;
        let mut detached = self.detached_put_attempts.remove(detached_index)?;
        if let CommandState::Put(state) = &mut detached.entry.state {
            state.put_pending = false;
        }
        if succeeded {
            return None;
        }
        detached.entry.finished = Some(CommandStatus::Failed);
        self.record_detached_failure(&detached.entry, &detached.base_chain)
    }

    /// Freeze the failure feedback of the Exit which emitted
    /// ActivateEntrance. A callback may detach that command before its false
    /// result arrives, but C++ still runs the old command's Fail tail.
    pub(crate) fn pending_exit_activation_failure_feedback(
        &self,
        command_instance_id: u64,
    ) -> Option<CommandFailureFeedback> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Exit(state) if state.activation_pending != 0
                )
        });
        if let Some(index) = index {
            let entry = &self.entries[index];
            let base = self
                .entries
                .iter()
                .skip(index + 1)
                .find(|entry| entry.finished.is_none());
            let execute_feedback = match entry.mode {
                CommandMode::SilentSub => base.is_none(),
                CommandMode::Sub => base.is_none_or(|entry| entry.retries == 0),
                CommandMode::Base => true,
                CommandMode::SilentBase | CommandMode::Unknown(_) => false,
            };
            return execute_feedback.then(|| CommandFailureFeedback {
                command: CommandView::from_entry(
                    entry
                        .state
                        .id()
                        .map(CommandId::to_name)
                        .unwrap_or("None")
                        .to_string(),
                    entry.request.as_ref(),
                    &entry.state,
                    entry.finished.is_some(),
                ),
                reason: None,
            });
        }
        let detached = self.detached_exit_preludes.iter().find(|detached| {
            (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                && matches!(
                    &detached.entry.state,
                    CommandState::Exit(state) if state.activation_pending != 0
                )
        })?;
        let base = detached
            .base_chain
            .iter()
            .find(|entry| entry.finished.is_none());
        let execute_feedback = match detached.entry.mode {
            CommandMode::SilentSub => base.is_none(),
            CommandMode::Sub => base.is_none_or(|entry| entry.retries == 0),
            CommandMode::Base => true,
            CommandMode::SilentBase | CommandMode::Unknown(_) => false,
        };
        execute_feedback.then(|| CommandFailureFeedback {
            command: CommandView::from_entry(
                detached
                    .entry
                    .state
                    .id()
                    .map(CommandId::to_name)
                    .unwrap_or("None")
                    .to_string(),
                detached.entry.request.as_ref(),
                &detached.entry.state,
                detached.entry.finished.is_some(),
            ),
            reason: None,
        })
    }

    /// Resolve one nested activation depth on the exact Exit which emitted
    /// it. Callback-installed replacement Exits have no pending depth and
    /// remain untouched (C4Command.cpp:644-650,1575-1582).
    pub(crate) fn resolve_exit_activation(
        &mut self,
        activated: bool,
        command_instance_id: u64,
    ) -> Option<ExitActivationResolution> {
        let index = self.entries.iter().position(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(
                    &entry.state,
                    CommandState::Exit(state) if state.activation_pending != 0
                )
        });
        if let Some(index) = index {
            if let CommandState::Exit(state) = &mut self.entries[index].state {
                state.activation_pending = state.activation_pending.saturating_sub(1);
            }
            if activated {
                return Some(ExitActivationResolution { feedback: None });
            }
            self.entries[index].finished = Some(CommandStatus::Failed);
            let feedback = self.record_failure_at(index);
            return Some(ExitActivationResolution { feedback });
        }
        let detached_index = self.detached_exit_preludes.iter().position(|detached| {
            (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                && matches!(
                    &detached.entry.state,
                    CommandState::Exit(state) if state.activation_pending != 0
                )
        })?;
        let mut detached = self.detached_exit_preludes.remove(detached_index)?;
        let CommandState::Exit(state) = &mut detached.entry.state else {
            unreachable!("detached Exit activation matched another command")
        };
        state.activation_pending = state.activation_pending.saturating_sub(1);
        if activated {
            if state.activation_pending != 0 {
                self.detached_exit_preludes.push_back(detached);
            }
            return Some(ExitActivationResolution { feedback: None });
        }
        detached.entry.finished = Some(CommandStatus::Failed);
        let feedback = self.record_detached_failure(&detached.entry, &detached.base_chain);
        Some(ExitActivationResolution { feedback })
    }

    pub(crate) fn resolve_acquire_script_result(
        &mut self,
        command_instance_id: u64,
        result: AcquireScriptResult,
    ) -> bool {
        if result == AcquireScriptResult::Complete {
            self.record_native_success(CommandId::Acquire);
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(&entry.state, CommandState::Acquire(state) if state.script_pending)
        }) else {
            return false;
        };
        let CommandState::Acquire(state) = &mut entry.state else {
            unreachable!("pending Acquire predicate matched another command");
        };
        if result == AcquireScriptResult::Complete {
            state.script_pending = false;
            state.script_invoked = false;
            state.script_result = None;
            entry.finished = Some(CommandStatus::Completed);
        } else {
            state.script_result = Some(result);
        }
        true
    }

    /// Legacy fixture seam for feeding the first Acquire without a native
    /// command identity. Engine callback resolution uses the exact-instance
    /// method above.
    #[doc(hidden)]
    pub fn set_acquire_script_result(&mut self, result: AcquireScriptResult) -> bool {
        for entry in &mut self.entries {
            if let CommandState::Acquire(state) = &mut entry.state {
                state.script_result = Some(result);
                return true;
            }
        }
        false
    }

    pub(crate) fn resolve_construct_script_result(
        &mut self,
        command_instance_id: u64,
        result: AcquireScriptResult,
    ) -> bool {
        if result == AcquireScriptResult::Complete {
            self.record_native_success(CommandId::Construct);
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            (command_instance_id == 0 || entry.instance_id == command_instance_id)
                && matches!(&entry.state, CommandState::Construct(state) if state.script_pending)
        }) {
            let CommandState::Construct(state) = &mut entry.state else {
                unreachable!("pending Construct predicate matched another command");
            };
            if result == AcquireScriptResult::Complete {
                state.script_pending = false;
                state.script_invoked = false;
                state.script_result = None;
                entry.finished = Some(CommandStatus::Completed);
            } else {
                state.script_result = Some(result);
            }
            return true;
        }
        let detached_index = self
            .detached_construct_commands
            .iter()
            .position(|detached| {
                (command_instance_id == 0 || detached.entry.instance_id == command_instance_id)
                    && matches!(
                        &detached.entry.state,
                        CommandState::Construct(state) if state.script_pending
                    )
            });
        if let Some(detached_index) = detached_index {
            let detached = &mut self.detached_construct_commands[detached_index];
            let CommandState::Construct(state) = &mut detached.entry.state else {
                unreachable!("detached Construct predicate matched another command");
            };
            state.script_result = Some(result);
            return true;
        }
        false
    }

    pub fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<CommandStepResult> {
        let result = self.execute_front(ctx);
        self.clear_finished_fronts();
        result
    }

    /// FnFinishCommand (C4Script.cpp:947-957): walk to the index-th
    /// command; success drops it (Finished commands complete on their
    /// next Execute — the stack model removes immediately), failure
    /// bumps the entry's failure counter.
    pub fn finish_entry_public(&mut self, index: i32, success: bool) -> bool {
        self.finish_entry(index, success)
    }

    fn finish_entry(&mut self, index: i32, success: bool) -> bool {
        if index < 0 {
            return false;
        }
        let index = index as usize;
        let Some(entry) = self.entries.get_mut(index) else {
            return false;
        };
        if success {
            entry.finished = Some(CommandStatus::Completed);
        } else {
            entry.failures = entry.failures.saturating_add(1);
        }
        true
    }

    /// Drain one feedback item produced by a live callback resolution.
    /// Callers must do this synchronously, before the finished-command
    /// callback can clear or replace the stack.
    pub(crate) fn take_failure_feedback(&mut self) -> Option<CommandFailureFeedback> {
        self.pending_failure_feedback.pop_front()
    }

    /// C4Command::Fail's exact BaseMode/GetBaseCommand gate. The failed
    /// entry must already have `Finished=true`, just as C++ Finish does
    /// before calling Fail (C4Command.cpp:1575-1582,2139-2174).
    pub(in crate::command) fn record_failure_at(
        &mut self,
        index: usize,
    ) -> Option<CommandFailureFeedback> {
        let mode = self.entries.get(index)?.mode;
        let base_index = self
            .entries
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, entry)| entry.finished.is_none())
            .map(|(base_index, _)| base_index);

        let execute_feedback = match mode {
            CommandMode::SilentSub => {
                if let Some(base_index) = base_index {
                    let base = &mut self.entries[base_index];
                    base.failures = base.failures.saturating_add(1);
                    false
                } else {
                    true
                }
            }
            CommandMode::Sub => {
                if let Some(base_index) = base_index {
                    let base = &mut self.entries[base_index];
                    base.failures = base.failures.saturating_add(1);
                    base.retries == 0
                } else {
                    true
                }
            }
            CommandMode::Base => true,
            CommandMode::SilentBase | CommandMode::Unknown(_) => false,
        };

        execute_feedback.then(|| {
            let entry = &self.entries[index];
            CommandFailureFeedback {
                command: CommandView::from_entry(
                    entry
                        .state
                        .id()
                        .map(CommandId::to_name)
                        .unwrap_or("None")
                        .to_string(),
                    entry.request.as_ref(),
                    &entry.state,
                    entry.finished.is_some(),
                ),
                reason: None,
            }
        })
    }

    /// Fail tail for an iExec command unlinked by ClearCommand(s). Preserve
    /// the original ordered `Next` chain: callback-installed replacements
    /// are not bases, and a newly finished original base must be skipped.
    fn increment_detached_base_failure(
        &mut self,
        base_chain: &[DetachedCommandBase],
    ) -> Option<bool> {
        for candidate in base_chain {
            if let Some(index) = self
                .entries
                .iter()
                .position(|entry| entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.entries[index];
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_exit_preludes
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_exit_preludes[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_build_stops
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_build_stops[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_move_to_flights
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_move_to_flights[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_throw_preludes
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_throw_preludes[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_drop_preludes
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_drop_preludes[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_put_attempts
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_put_attempts[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_put_stops
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_put_stops[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_construct_commands
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_construct_commands[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_get_attempts
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_get_attempts[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if let Some(index) = self
                .detached_physical_commands
                .iter()
                .position(|entry| entry.entry.instance_id == candidate.instance_id)
            {
                let base = &mut self.detached_physical_commands[index].entry;
                if base.finished.is_some() {
                    continue;
                }
                base.failures = base.failures.saturating_add(1);
                return Some(base.retries == 0);
            }
            if candidate.finished.is_none() {
                // The base was another executing command cleared from the
                // object list. Native iExec retains it even when this Rust
                // continuation has no full detached state for that command.
                return Some(candidate.retries == 0);
            }
        }
        None
    }

    pub(in crate::command) fn record_detached_failure(
        &mut self,
        entry: &ActiveCommand,
        base_chain: &[DetachedCommandBase],
    ) -> Option<CommandFailureFeedback> {
        let execute_feedback = match entry.mode {
            CommandMode::SilentSub => self.increment_detached_base_failure(base_chain).is_none(),
            CommandMode::Sub => self
                .increment_detached_base_failure(base_chain)
                .unwrap_or(true),
            CommandMode::Base => true,
            CommandMode::SilentBase | CommandMode::Unknown(_) => false,
        };
        execute_feedback.then(|| CommandFailureFeedback {
            command: CommandView::from_entry(
                entry
                    .state
                    .id()
                    .map(CommandId::to_name)
                    .unwrap_or("None")
                    .to_string(),
                entry.request.as_ref(),
                &entry.state,
                entry.finished.is_some(),
            ),
            reason: None,
        })
    }
}
