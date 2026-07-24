//! `command` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandMode {
    SilentSub,
    Base,
    SilentBase,
    Sub,
    Unknown(i32),
}

impl CommandMode {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::SilentSub),
            1 => Some(Self::Base),
            2 => Some(Self::SilentBase),
            3 => Some(Self::Sub),
            value => Some(Self::Unknown(value)),
        }
    }

    pub(crate) const fn to_i32(self) -> i32 {
        match self {
            Self::SilentSub => 0,
            Self::Base => 1,
            Self::SilentBase => 2,
            Self::Sub => 3,
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandData {
    Integer(i32),
    Text(String),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRequest {
    pub id: CommandId,
    pub target: Option<ObjectId>,
    pub target2: Option<ObjectId>,
    pub tx: Option<i32>,
    /// Exact tagged C4Command::Tx. Integer/C4ID mirrors remain above/below
    /// for the many command implementations that consume those projections;
    /// Call forwards this complete value verbatim to script.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_value: Option<clonk_script::Value>,
    /// C4Command::Tx is a tagged C4Value. Most commands use its integer
    /// payload, but Call must preserve a C4ID tag for script parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_definition: Option<DefinitionId>,
    pub ty: Option<i32>,
    pub data: CommandData,
    /// Raw signed `C4Command::UpdateInterval`. Native accepts negative
    /// values from script and compiled saves; `Execute` decrements only
    /// strictly positive values, so non-positive words remain verbatim.
    pub update_interval: i32,
    /// C4Command::Evaluated as initialized by C4Object::AddCommand's
    /// fInitEvaluation parameter. Pathfinder waypoints are the only
    /// commands created with evaluation already complete.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub evaluated: bool,
    pub retries: i32,
    pub mode: CommandMode,
}

impl CommandRequest {
    pub fn new(id: CommandId) -> Self {
        Self {
            id,
            target: None,
            target2: None,
            tx: None,
            tx_value: None,
            tx_definition: None,
            ty: None,
            data: CommandData::None,
            update_interval: 0,
            evaluated: false,
            retries: 0,
            mode: CommandMode::SilentSub,
        }
    }

    pub fn with_target(mut self, target: Option<ObjectId>) -> Self {
        self.target = target;
        self
    }

    pub fn with_target2(mut self, target: Option<ObjectId>) -> Self {
        self.target2 = target;
        self
    }

    pub fn with_tx(mut self, tx: Option<i32>) -> Self {
        self.tx = tx;
        self.tx_value = tx.map(clonk_script::Value::Int);
        self.tx_definition = None;
        self
    }

    pub fn with_tx_definition(mut self, definition_id: DefinitionId) -> Self {
        self.tx = definition_id_to_c4id(&definition_id);
        self.tx_value = Some(clonk_script::Value::C4Id(definition_id.clone()));
        self.tx_definition = Some(definition_id);
        self
    }

    pub fn with_tx_value(mut self, value: clonk_script::Value) -> Self {
        self.tx = match &value {
            clonk_script::Value::Int(value) => Some(*value),
            _ => None,
        };
        self.tx_definition = match &value {
            clonk_script::Value::C4Id(definition) => Some(definition.clone()),
            _ => None,
        };
        self.tx_value = Some(value);
        self
    }

    pub fn with_ty(mut self, ty: Option<i32>) -> Self {
        self.ty = ty;
        self
    }

    pub fn with_data(mut self, data: CommandData) -> Self {
        self.data = data;
        self
    }

    pub fn with_update_interval(mut self, interval: i32) -> Self {
        self.update_interval = interval;
        self
    }

    pub fn with_evaluated(mut self, evaluated: bool) -> Self {
        self.evaluated = evaluated;
        self
    }

    pub fn with_retries(mut self, retries: i32) -> Self {
        self.retries = retries;
        self
    }

    pub fn with_mode(mut self, mode: CommandMode) -> Self {
        self.mode = mode;
        self
    }
}

/// Typed command helpers retain several historical positive counters that
/// predate the shared raw `ActiveCommand::update_interval`. They are not the
/// persisted C4Command word; convert explicitly so a negative raw interval
/// never wraps to a huge unsigned duration.
pub(in crate::command) const fn positive_helper_interval(interval: i32) -> u32 {
    if interval > 0 {
        interval as u32
    } else {
        0
    }
}

pub(in crate::command) const fn positive_helper_interval_or_one(interval: i32) -> u32 {
    let interval = positive_helper_interval(interval);
    if interval == 0 {
        1
    } else {
        interval
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CommandOperation {
    Clear,
    PushFront(CommandRequest),
    PushBack(CommandRequest),
    /// FnFinishCommand (C4Script.cpp:947-957): mark the index-th stack
    /// entry finished (success) or bump its failure counter.
    Finish {
        index: i32,
        success: bool,
    },
    /// C4Object::SetCommand's entry decrement (C4Object.cpp:3941-3942):
    /// every SetCommand counts an armed NoCollectDelay down by one. It
    /// travels with the command ops so its order against Clear/Push is
    /// preserved through the staged script outcomes.
    DecrementNoCollectDelay,
    /// ObjectComDrop's post-Exit assignment. This is an ordered command
    /// operation because a synchronous ExecuteCommand may run after an
    /// earlier SetCommand decrement in the same script call.
    SetNoCollectDelay {
        value: i32,
        ocf: u32,
    },
    /// Replace the staged command stack after a synchronous
    /// `ExecuteCommand` host call. The command's mutable evaluation state
    /// must cross the copy-in/copy-out script boundary with the stack.
    Restore(CommandStackSnapshot),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MenuRequestKind {
    Context {
        target: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<Vector2>,
    },
    /// ActivateMenu(C4MN_Activate) with the calling object's current
    /// container as the implicit target (C4Command::Take).
    Activate,
    /// ActivateMenu(C4MN_Activate, ..., Target2) with an explicit target
    /// container (C4Command::Activate's first branch).
    ActivateTarget {
        container: ObjectId,
    },
    Get {
        container: ObjectId,
    },
    /// ActivateMenu(C4MN_Contents) (C4Command::Get Data=2,
    /// C4Command.cpp:1129-1135).
    Contents {
        container: ObjectId,
    },
    /// C4Command::Construct with no definition opens the owning crew's
    /// C4MN_Construction menu (C4Command.cpp:1690-1703).
    Construction,
    /// ActivateMenu(C4MN_Buy) with the base container as target
    /// (ContainedControl COM_Up, C4Object.cpp:3269-3274; ActivateMenu,
    /// C4Object.cpp:1919-1930).
    Buy {
        base: ObjectId,
    },
    /// ActivateMenu(C4MN_Sell) (ContainedControl COM_Dig,
    /// C4Object.cpp:3275-3280; ActivateMenu, C4Object.cpp:1932-1943).
    Sell {
        base: ObjectId,
    },
    /// ShowInfo(target) -> ActivateMenu(C4MN_Info) on the calling object
    /// (C4Script.cpp:3332-3336; C4Object.cpp:2008-2027).
    Info {
        target: ObjectId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuRequest {
    pub crew_id: ObjectId,
    pub owner: i32,
    pub kind: MenuRequestKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CommandEvent {
    /// `C4Command::MoveTo` writes both knobs on the game-global pathfinder
    /// immediately before every obstructed-path Find. Later script GetPath
    /// calls reuse the pair, even when this Find fails (C4Command.cpp:
    /// 239-244; C4Script.cpp:5040).
    SetPathFinderSettings {
        level: i32,
        transfer_zones_enabled: bool,
    },
    /// Presentation copy of the rays retained by native's game-global
    /// pathfinder after the most recent obstructed MoveTo search.
    SetPathFinderDebug {
        snapshot: PathfinderDebugSnapshot,
    },
    /// Resolve the actor's first scripted fair-crew projection at the exact
    /// native GetPhysical call, then resume this same C4Command::Execute.
    /// Build and Construct evaluate GetPhysical twice in their capability
    /// guards; `reads` preserves the second, post-hook backing selection.
    ResolveCommandPhysical {
        object_id: ObjectId,
        reads: u8,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// MoveTo's FlightControl calls ordinary SetActionByName("Fly"). Its
    /// callbacks must finish before a walking command runs JumpControl
    /// against the callback-mutated live object.
    MoveToFlightControlTakeoff {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    ApplyObjectUpdate {
        object_id: ObjectId,
        update: ObjectUpdate,
    },
    /// Run C4Object::Enter as one ordered operation so the engine can make
    /// the container link visible before Collection2 and Entrance callbacks
    /// execute (C4Object.cpp:1598-1630).
    EnterObject {
        object_id: ObjectId,
        container_id: ObjectId,
    },
    /// C4Command::GetTryEnter must observe both Enter vetoes and may need
    /// to put away an existing inventory object before retrying the SAME
    /// target/count (C4Command.cpp:1092-1126).
    GetObject {
        actor_id: ObjectId,
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Run Buy's callbackful GetValue preflight and, once contained, the
    /// complete C4Player::Buy loop against live state. Dynamic pricing may
    /// mutate stock, wealth, commands, or containment before Buy decides
    /// whether to push Enter (C4Command.cpp:1987-2041).
    EvaluateBuy {
        actor_id: ObjectId,
        base_id: ObjectId,
        definition_id: DefinitionId,
        buyer: i32,
        payer: i32,
        count: i32,
    },
    /// Run SellFromBase and the complete recursive C4Player::Sell2Home
    /// loop against live state. CalcValue/SellTo/Sale callbacks can mutate
    /// the base, player, objects, or command stack between count iterations
    /// (C4Command.cpp:2040-2080; C4ObjectCom.cpp:959-988).
    EvaluateSell {
        actor_id: ObjectId,
        base_id: ObjectId,
        definition_id: DefinitionId,
        preferred: Option<ObjectId>,
        count: i32,
    },
    /// Run ObjectComPut as one live ordered operation. Enter rejection,
    /// collection callbacks, and helper failure must resolve against the
    /// exact Put command that emitted the attempt (C4ObjectCom.cpp:591-622;
    /// C4Command.cpp:1439-1503).
    ObjectComPut {
        actor_id: ObjectId,
        target_id: ObjectId,
        object_id: ObjectId,
        /// Put's internal Ty flag: a target grabbed solely for this command
        /// is released after a successful helper call.
        ungrab_on_success: bool,
        /// Identity of the native Put whose callbackful helper is in flight.
        /// Persisted legacy events rebind zero before their first callback.
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Throw/Drop call ObjectComPutTake inline and unconditionally finish
    /// only after its callbackful put or menu attempt returns. Keep this
    /// separate from ObjectComPut so a nested PutTake cannot consume an
    /// outer Put command's pending result (C4ObjectCom.cpp:700-721;
    /// C4Command.cpp:966-979,1036-1049).
    ObjectComPutTake {
        actor_id: ObjectId,
        target_id: ObjectId,
        requested_item: Option<ObjectId>,
        command: CommandId,
        /// Identity of the native C4Command instance whose helper is in
        /// flight. Zero is reserved for deserialized legacy events.
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// ObjectComThrow -> ObjectActionThrow is one ordered operation: the
    /// action transition must succeed before Random(360) and C4Object::Exit
    /// run (C4ObjectCom.cpp:120-137).
    ThrowObject {
        actor_id: ObjectId,
        object_id: ObjectId,
        /// Targeted throws finish only on helper success; ordinary outside
        /// throws finish after the helper regardless of its boolean.
        complete_command_on_success: bool,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Run ObjectComDrop as one live ordered operation. Exit callbacks must
    /// observe the final fixed motion before the dropper's NoCollectDelay
    /// and trailing ObjectComUnGrab (C4ObjectCom.cpp:640-676).
    ObjectComDrop {
        actor_id: ObjectId,
        object_id: ObjectId,
        /// Identity of the native C4Command instance whose helper is in
        /// flight. Zero is reserved for deserialized legacy events.
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// C4CMD_UnGrab executes the callbackful ObjectComUnGrab, then writes
    /// ComDir Stop and finishes unconditionally (C4Command.cpp:903-908).
    ObjectComUnGrabCommand {
        actor_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Execute C4Command::Jump against the live object. ObjectComJump may run
    /// the object's OnActionJump hook synchronously, so it cannot be reduced
    /// to a pure snapshot update (C4Command.cpp:1056-1067;
    /// C4ObjectCom.cpp:48-61,280-307).
    ObjectComJump {
        object_id: ObjectId,
        tx: i32,
    },
    /// C4Command::Dig calls the callbackful ObjectComDig before writing
    /// Dig2Object and steering. Its failure message and action rejection
    /// must therefore resolve against live state (C4Command.cpp:468-484;
    /// C4ObjectCom.cpp:353-361).
    ObjectComDig {
        actor_id: ObjectId,
        dig_out_material: bool,
        direction: Option<CommandDirection>,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// C4Command::Exit's collection-area branch calls ObjectComJump
    /// directly and only finishes Exit after the callbackful helper returns
    /// (C4Command.cpp:643-649). Keep this separate from the Jump-command
    /// event, which must not finish a callback-installed C4CMD_Jump.
    ObjectComExitJump {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Run C4Command::Exit's live C4Object::Exit call, including ordered
    /// Ejection/Departure callbacks, before its optional collection-area
    /// ObjectComJump and final Finish(true). A field-only containment update
    /// would skip both callbacks and finish the command too early
    /// (C4Command.cpp:624-653; C4Object.cpp:1513-1545).
    CommandExitObject {
        object_id: ObjectId,
        previous_container: ObjectId,
        position: Vector2,
        jump_after: bool,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Nested C4Command::Exit delegates to C4Object::Enter and calls
    /// Finish(true) only after that complete callbackful operation returns.
    CommandExitIntoParent {
        object_id: ObjectId,
        container_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// DFA_BUILD Exit runs callbackful ObjectComStop and then resumes the
    /// same C4Command::Exit invocation against callback-mutated live state.
    ObjectComStopExit {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// C4Command::MoveTo must run the callbackful, ordinary ObjectComStop
    /// before its range/idle/procedure continuation. The continuation is
    /// retained on the live MoveTo state so the engine can resume it in the
    /// same Execute without decrementing UpdateInterval a second time.
    ObjectComStopMoveTo {
        object_id: ObjectId,
    },
    /// Build's Dig arm runs callbackful `ObjectComStop` and resumes the
    /// same C4Command::Build invocation afterward (C4Command.cpp:872-899).
    ObjectComStopBuild {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Chop's active work-procedure branch runs callbackful ObjectComStop
    /// and then returns from Execute without any post-stop command body.
    ObjectComStopChop {
        object_id: ObjectId,
    },
    /// Construct stops BUILD/CHOP/DIG through callbackful ObjectComStop and
    /// continues the same Execute into push/site/script handling.
    ObjectComStopConstruct {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Run `ObjectComBuild` against live action state. Ordinary reached-site
    /// builds stop first; the legacy internal-structure arm does not.
    ObjectComBuild {
        object_id: ObjectId,
        target_id: ObjectId,
        stop_first: bool,
    },
    /// C4Command::Throw runs ordinary ObjectComStop synchronously when the
    /// actor starts in DFA_DIG, then continues the same native command with
    /// callback-mutated object/command state (C4Command.cpp:910-914).
    ObjectComStopThrow {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// C4Command::Drop has the same callbackful ObjectComStop prelude when
    /// it starts in DFA_DIG, then continues the same native command against
    /// callback-mutated state (C4Command.cpp:988-989).
    ObjectComStopDrop {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Put stops DFA_DIG through ordinary callbackful ObjectComStop, then
    /// continues the same Execute against post-callback action/containment
    /// state before it reads pGrabbing or GetPhysical (C4Command.cpp:1438).
    ObjectComStopPut {
        object_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// A targeted Throw at its launch position calls C4Object::SetDir before
    /// writing ComDir=Stop and invoking ObjectComThrow. SetDir may run the
    /// current action's TurnAction callbacks, so the remaining Throw body
    /// must resume against live state (C4Command.cpp:948-955).
    ObjectComSetDirThrow {
        object_id: ObjectId,
        direction: Direction,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// Run C4Command::Grab's live sequence. Build/chop/dig stopping may
    /// change the subsequent At result, while scale/hangle let-go and the
    /// target's RejectGrabbed callback must finish before ObjectComGrab
    /// (C4Command.cpp:667-716).
    AttemptGrab {
        actor_id: ObjectId,
        target_id: ObjectId,
    },
    /// Assign a fresh command stack to another object. C4CMD_Activate uses
    /// `Target->SetCommand(C4CMD_Exit)` with the actor's controller, while
    /// push-target Enter assigns the vehicle's Enter without changing its
    /// controller (C4Command.cpp:594-597,1335-1362).
    SetObjectCommand {
        object_id: ObjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        controller: Option<i32>,
        request: CommandRequest,
    },
    ControlCommandAcquire {
        caller: ObjectId,
        target: Option<ObjectId>,
        range_x: i32,
        range_y: i32,
        ignore_container: Option<ObjectId>,
        definition_id: DefinitionId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// `~ControlCommandConstruction(Target,Tx,Ty,Target2,Data)` runs after
    /// site resolution and before conkit/range/construction checks. Its
    /// integer result uses the same 0/1/2/3 contract as Acquire.
    ControlCommandConstruction {
        caller: ObjectId,
        target: Option<ObjectId>,
        site: Vector2,
        target2: Option<ObjectId>,
        definition_id: DefinitionId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// ConstructionCheck rejected Construct's site: emit the localized
    /// `GameMsgObject(..., cObj, FRed)` feedback before the appended
    /// FailureFeedback event (C4Command.cpp:1797-1801;
    /// C4Landscape.cpp:2131-2163).
    ConstructionCheckRejected {
        actor_id: ObjectId,
        definition_id: DefinitionId,
        failure: ConstructionCheckFailure,
    },
    /// Construct's validated native creation tail: create the Con=1
    /// object, consume the conkit, then resume this exact command to
    /// Finish(true) and add Build in the same Execute.
    SpawnConstruction {
        actor_id: ObjectId,
        definition_id: DefinitionId,
        owner: i32,
        position: Vector2,
        kit_id: ObjectId,
        #[serde(skip)]
        command_instance_id: u64,
    },
    SpawnObject {
        definition_id: DefinitionId,
        owner: i32,
        position: Vector2,
        container: Option<ObjectId>,
        #[serde(default)]
        construction: Option<i32>,
    },
    /// Atomically create a CONNECT line with both live endpoints. A plain
    /// SpawnObject would execute once with null targets and break before a
    /// later update could attach it (CreateLine, C4Command.cpp:2244,2285-2289).
    CreateLine {
        definition_id: DefinitionId,
        owner: i32,
        from: ObjectId,
        to: ObjectId,
    },
    /// C4Command::Transfer invokes the definition host's cached
    /// `SFn_ControlTransfer` pointer directly. Unlike C4Object::Call this
    /// deliberately has no receiver-Status gate, and a missing/false
    /// callback completes the Transfer successfully (C4Command.cpp:
    /// 1931-1942; C4ScriptHost.cpp:178-189).
    ControlTransfer {
        object_id: ObjectId,
        caller: ObjectId,
        /// Exact tagged C4Command::Tx forwarded by `f->Exec`.
        tx_value: clonk_script::Value,
        ty: i32,
        /// Identity of the native Transfer whose direct callback is in
        /// flight. Zero is reserved for direct fixtures/legacy events.
        #[serde(skip)]
        command_instance_id: u64,
    },
    CallObjectFunction {
        object_id: ObjectId,
        function: String,
        caller: ObjectId,
        tx: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tx_value: Option<clonk_script::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tx_definition: Option<DefinitionId>,
        ty: Option<i32>,
        target2: Option<ObjectId>,
        #[serde(default)]
        on_result: Option<CallResultAction>,
    },
    /// Run C4Object::ActivateEntrance rather than directly dispatching the
    /// same-named script callback. The native method applies the hostile-base
    /// and current OCF_Entrance gates before calling ~ActivateEntrance
    /// (C4Object.cpp:1654-1670).
    ActivateEntrance {
        object_id: ObjectId,
        caller: ObjectId,
        #[serde(default)]
        on_result: Option<CallResultAction>,
        #[serde(skip)]
        command_instance_id: u64,
    },
    /// An ordered `Finish(true)` that occurs before the same command later
    /// reaches another callback or a final failure. Most successful finishes
    /// use the retained command-stack tail; this event preserves the rare
    /// mixed Construct success-before-failure ordering.
    NativeCommandSuccess {
        object_id: ObjectId,
        command: CommandId,
    },
    /// C4Command::Fail's mode/retry-gated tail. The engine executes this
    /// synchronously after the command handler's own events and before
    /// `~ControlCommandFinished`, because CallFailed may replace the stack
    /// (C4Command.cpp:2139-2242,2428-2439).
    FailureFeedback {
        actor_id: ObjectId,
        feedback: CommandFailureFeedback,
    },
    AdjustPlayerHomeBaseMaterial {
        player_id: i32,
        definition_id: DefinitionId,
        delta: i32,
    },
    AdjustPlayerWealth {
        player_id: i32,
        delta: i32,
    },
    /// ObjectComDrop's post-exit `cObj->NoCollectDelay = 2` plus the
    /// immediate `SetOCF()` on the DROPPER (C4ObjectCom.cpp:668-671).
    ArmNoCollectDelay {
        object_id: ObjectId,
    },
    OpenMenu(MenuRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallResultAction {
    CompleteCommandOnFalse {
        command: CommandId,
    },
    CompleteCommandOnTrue {
        command: CommandId,
    },
    /// Legacy queued-event form retained for save compatibility. Native
    /// entrance activation now uses ResolveExitActivation so newly emitted
    /// events resolve the exact in-flight Exit synchronously.
    FailCommandOnFalse {
        command: CommandId,
    },
    /// Resolve only the Exit which emitted a native ActivateEntrance event.
    /// Callback-side command replacement must not inherit its result.
    ResolveExitActivation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandFailureReason {
    CannotBuild,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandStepResult {
    pub update: Option<ObjectUpdate>,
    pub status: CommandStatus,
    pub operations: Vec<CommandOperation>,
    pub events: Vec<CommandEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<CommandFailureReason>,
}

impl CommandStepResult {
    pub fn running(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Running,
            operations: Vec::new(),
            events: Vec::new(),
            failure_reason: None,
        }
    }

    pub fn completed(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Completed,
            operations: Vec::new(),
            events: Vec::new(),
            failure_reason: None,
        }
    }

    pub fn failed(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Failed,
            operations: Vec::new(),
            events: Vec::new(),
            failure_reason: None,
        }
    }

    pub fn with_operations(mut self, operations: Vec<CommandOperation>) -> Self {
        self.operations = operations;
        self
    }

    pub fn with_events(mut self, events: Vec<CommandEvent>) -> Self {
        self.events = events;
        self
    }

    pub fn with_failure_reason(mut self, reason: CommandFailureReason) -> Self {
        self.failure_reason = Some(reason);
        self
    }
}

pub(in crate::command) fn resolve_command_physical(
    object_id: ObjectId,
    reads: u8,
    update: Option<ObjectUpdate>,
) -> CommandStepResult {
    CommandStepResult::running(update).with_events(vec![CommandEvent::ResolveCommandPhysical {
        object_id,
        reads,
        command_instance_id: 0,
    }])
}

pub(in crate::command) fn stamp_command_event_instances(events: &mut [CommandEvent], instance_id: u64) {
    for event in events {
        let event_instance_id = match event {
            CommandEvent::ResolveCommandPhysical {
                command_instance_id,
                ..
            }
            | CommandEvent::MoveToFlightControlTakeoff {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComPut {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComPutTake {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComDrop {
                command_instance_id,
                ..
            }
            | CommandEvent::ThrowObject {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComUnGrabCommand {
                command_instance_id,
                ..
            }
            | CommandEvent::CommandExitObject {
                command_instance_id,
                ..
            }
            | CommandEvent::CommandExitIntoParent {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComStopExit {
                command_instance_id,
                ..
            }
            | CommandEvent::ActivateEntrance {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComDig {
                command_instance_id,
                ..
            }
            | CommandEvent::GetObject {
                command_instance_id,
                ..
            }
            | CommandEvent::ControlCommandAcquire {
                command_instance_id,
                ..
            }
            | CommandEvent::ControlCommandConstruction {
                command_instance_id,
                ..
            }
            | CommandEvent::SpawnConstruction {
                command_instance_id,
                ..
            }
            | CommandEvent::ControlTransfer {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComStopDrop {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComStopPut {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComStopConstruct {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComStopBuild {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComStopThrow {
                command_instance_id,
                ..
            }
            | CommandEvent::ObjectComSetDirThrow {
                command_instance_id,
                ..
            } => command_instance_id,
            _ => continue,
        };
        if *event_instance_id == 0 {
            *event_instance_id = instance_id;
        }
    }
}

fn c4id_to_definition_string(id: i32) -> Option<DefinitionId> {
    let raw = id as u32 as usize;
    (raw != 0).then(|| clonk_script::c4_id_from_raw(raw))
}

pub(crate) fn definition_id_to_c4id(definition: &str) -> Option<i32> {
    let raw = clonk_script::c4_id_raw(definition);
    (raw != 0).then_some(raw as u32 as i32)
}

pub(in crate::command) fn command_data_to_definition_id(data: &CommandData) -> Option<DefinitionId> {
    match data {
        CommandData::Integer(value) => c4id_to_definition_string(*value),
        CommandData::Text(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}
