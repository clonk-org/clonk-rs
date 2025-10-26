use std::collections::{HashMap, VecDeque};

use crate::{
    ActionProcedure, ActionUpdate, CommandDirection, ObjectId, ObjectStatus, ObjectUpdate, Vector2,
    CATEGORY_STATIC_BACK, CATEGORY_STRUCTURE, FULL_CON,
};

/// Maximum number of commands that may be queued for an object.
pub const MAX_COMMAND_STACK: usize = 35;

#[derive(Debug, Clone)]
pub struct CommandObjectSnapshot {
    pub id: ObjectId,
    pub position: Vector2,
    pub status: ObjectStatus,
    pub destroyed: bool,
    pub category: i32,
    pub container: Option<ObjectId>,
    pub action_target: Option<ObjectId>,
    pub action_procedure: ActionProcedure,
    pub command_direction: CommandDirection,
    pub construction: i32,
}

impl CommandObjectSnapshot {
    pub fn is_active(&self) -> bool {
        !self.destroyed && self.status.is_active()
    }
}

/// Identifiers that map to the classic C4 command constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CommandId {
    Follow = 1,
    MoveTo = 2,
    Enter = 3,
    Exit = 4,
    Grab = 5,
    Build = 6,
    Throw = 7,
    Chop = 8,
    UnGrab = 9,
    Jump = 10,
    Wait = 11,
    Get = 12,
    Put = 13,
    Drop = 14,
    Dig = 15,
    Activate = 16,
    PushTo = 17,
    Construct = 18,
    Transfer = 19,
    Attack = 20,
    Context = 21,
    Buy = 22,
    Sell = 23,
    Acquire = 24,
    Energy = 25,
    Retry = 26,
    Home = 27,
    Call = 28,
    Take = 29,
    Take2 = 30,
}

impl CommandId {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Follow" => Some(Self::Follow),
            "MoveTo" => Some(Self::MoveTo),
            "Enter" => Some(Self::Enter),
            "Exit" => Some(Self::Exit),
            "Grab" => Some(Self::Grab),
            "Build" => Some(Self::Build),
            "Throw" => Some(Self::Throw),
            "Chop" => Some(Self::Chop),
            "UnGrab" => Some(Self::UnGrab),
            "Jump" => Some(Self::Jump),
            "Wait" => Some(Self::Wait),
            "Get" => Some(Self::Get),
            "Put" => Some(Self::Put),
            "Drop" => Some(Self::Drop),
            "Dig" => Some(Self::Dig),
            "Activate" => Some(Self::Activate),
            "PushTo" => Some(Self::PushTo),
            "Construct" => Some(Self::Construct),
            "Transfer" => Some(Self::Transfer),
            "Attack" => Some(Self::Attack),
            "Context" => Some(Self::Context),
            "Buy" => Some(Self::Buy),
            "Sell" => Some(Self::Sell),
            "Acquire" => Some(Self::Acquire),
            "Energy" => Some(Self::Energy),
            "Retry" => Some(Self::Retry),
            "Home" => Some(Self::Home),
            "Call" => Some(Self::Call),
            "Take" => Some(Self::Take),
            "Take2" => Some(Self::Take2),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn to_name(self) -> &'static str {
        match self {
            Self::Follow => "Follow",
            Self::MoveTo => "MoveTo",
            Self::Enter => "Enter",
            Self::Exit => "Exit",
            Self::Grab => "Grab",
            Self::Build => "Build",
            Self::Throw => "Throw",
            Self::Chop => "Chop",
            Self::UnGrab => "UnGrab",
            Self::Jump => "Jump",
            Self::Wait => "Wait",
            Self::Get => "Get",
            Self::Put => "Put",
            Self::Drop => "Drop",
            Self::Dig => "Dig",
            Self::Activate => "Activate",
            Self::PushTo => "PushTo",
            Self::Construct => "Construct",
            Self::Transfer => "Transfer",
            Self::Attack => "Attack",
            Self::Context => "Context",
            Self::Buy => "Buy",
            Self::Sell => "Sell",
            Self::Acquire => "Acquire",
            Self::Energy => "Energy",
            Self::Retry => "Retry",
            Self::Home => "Home",
            Self::Call => "Call",
            Self::Take => "Take",
            Self::Take2 => "Take2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMode {
    SilentSub,
    Base,
    SilentBase,
    Sub,
}

impl CommandMode {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::SilentSub),
            1 => Some(Self::Base),
            2 => Some(Self::SilentBase),
            3 => Some(Self::Sub),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandData {
    Integer(i32),
    Text(String),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub id: CommandId,
    pub target: Option<ObjectId>,
    pub target2: Option<ObjectId>,
    pub tx: Option<i32>,
    pub ty: Option<i32>,
    pub data: CommandData,
    pub update_interval: u32,
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
            ty: None,
            data: CommandData::None,
            update_interval: 0,
            retries: 0,
            mode: CommandMode::Base,
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

    pub fn with_update_interval(mut self, interval: u32) -> Self {
        self.update_interval = interval;
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

#[derive(Debug, Clone)]
pub enum CommandOperation {
    Clear,
    PushFront(CommandRequest),
    PushBack(CommandRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct CommandStepResult {
    pub update: Option<ObjectUpdate>,
    pub status: CommandStatus,
    pub operations: Vec<CommandOperation>,
}

impl CommandStepResult {
    pub fn running(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Running,
            operations: Vec::new(),
        }
    }

    pub fn completed(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Completed,
            operations: Vec::new(),
        }
    }

    pub fn failed(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Failed,
            operations: Vec::new(),
        }
    }

    pub fn with_operations(mut self, operations: Vec<CommandOperation>) -> Self {
        self.operations = operations;
        self
    }
}

#[derive(Clone)]
pub struct CommandRuntimeContext<'a> {
    pub frame: u64,
    pub position: Vector2,
    pub object: &'a CommandObjectSnapshot,
    pub objects: &'a HashMap<ObjectId, CommandObjectSnapshot>,
}

impl<'a> CommandRuntimeContext<'a> {
    pub fn resolve(&self, id: ObjectId) -> Option<&CommandObjectSnapshot> {
        self.objects.get(&id)
    }

    pub fn resolve_position(&self, id: ObjectId) -> Option<Vector2> {
        if id == self.object.id {
            return Some(self.position);
        }
        self.resolve(id).map(|snapshot| snapshot.position)
    }
}

#[derive(Debug)]
pub enum CommandError {
    StackFull,
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct CommandStack {
    entries: VecDeque<ActiveCommand>,
}

impl CommandStack {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn push_front(&mut self, request: CommandRequest) -> Result<(), CommandError> {
        if self.entries.len() >= MAX_COMMAND_STACK {
            return Err(CommandError::StackFull);
        }
        let command = ActiveCommand::from_request(request)?;
        self.entries.push_front(command);
        Ok(())
    }

    pub fn push_back(&mut self, request: CommandRequest) -> Result<(), CommandError> {
        if self.entries.len() >= MAX_COMMAND_STACK {
            return Err(CommandError::StackFull);
        }
        let command = ActiveCommand::from_request(request)?;
        self.entries.push_back(command);
        Ok(())
    }

    pub fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<CommandStepResult> {
        let mut result = {
            let front = self.entries.front_mut()?;
            front.step(ctx)
        };
        match result.status {
            CommandStatus::Completed | CommandStatus::Failed => {
                self.entries.pop_front();
            }
            CommandStatus::Running => {}
        }
        for operation in std::mem::take(&mut result.operations) {
            match operation {
                CommandOperation::Clear => self.entries.clear(),
                CommandOperation::PushFront(request) => {
                    let _ = self.push_front(request);
                }
                CommandOperation::PushBack(request) => {
                    let _ = self.push_back(request);
                }
            }
        }
        Some(result)
    }
}

#[derive(Debug, Clone)]
struct MoveToState {
    target: Option<ObjectId>,
    tx: Option<i32>,
    ty: Option<i32>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    tolerance: i32,
    last_direction: CommandDirection,
    arrived_frames: u32,
}

impl MoveToState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            target: request.target,
            tx: request.tx,
            ty: request.ty,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            tolerance: 5,
            last_direction: CommandDirection::Stop,
            arrived_frames: 0,
        }
    }

    fn resolve_target_position(&self, ctx: &CommandRuntimeContext<'_>) -> Option<Vector2> {
        if let Some(target) = self.target {
            return ctx.resolve_position(target);
        }
        match (self.tx, self.ty) {
            (Some(x), Some(y)) => Some(Vector2::new(x, y)),
            _ => None,
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let target = match self.resolve_target_position(ctx) {
            Some(position) => position,
            None => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::failed(Some(update));
            }
        };

        let dx = target.x - ctx.position.x;
        let dy = target.y - ctx.position.y;
        if dx.abs() <= self.tolerance && dy.abs() <= self.tolerance {
            self.arrived_frames += 1;
        } else {
            self.arrived_frames = 0;
        }

        if self.arrived_frames >= 2 {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            self.last_direction = CommandDirection::Stop;
            return CommandStepResult::completed(Some(update));
        }

        let direction = if dx > self.tolerance {
            CommandDirection::Right
        } else if dx < -self.tolerance {
            CommandDirection::Left
        } else if dy > self.tolerance {
            CommandDirection::Down
        } else if dy < -self.tolerance {
            CommandDirection::Up
        } else {
            CommandDirection::Stop
        };

        if direction == self.last_direction {
            return CommandStepResult::running(None);
        }

        self.last_direction = direction;
        let update = ObjectUpdate::new().with_command_direction(direction);
        CommandStepResult::running(Some(update))
    }
}

#[derive(Debug, Clone)]
struct BuildState {
    target: ObjectId,
    site: Option<Vector2>,
    last_move_order: Option<u64>,
    approach_horizontal: i32,
    approach_vertical: i32,
}

impl BuildState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        let site = match (request.tx, request.ty) {
            (Some(x), Some(y)) => Some(Vector2::new(x, y)),
            _ => None,
        };
        Ok(Self {
            target,
            site,
            last_move_order: None,
            approach_horizontal: 9,
            approach_vertical: 20,
        })
    }

    fn target_position(&self, ctx: &CommandRuntimeContext<'_>) -> Option<Vector2> {
        if let Some(site) = self.site {
            return Some(site);
        }
        ctx.resolve_position(self.target)
    }

    fn should_issue_move(&mut self, frame: u64) -> bool {
        const MOVE_ORDER_COOLDOWN: u64 = 12;
        match self.last_move_order {
            Some(last) if frame.saturating_sub(last) < MOVE_ORDER_COOLDOWN => false,
            _ => {
                self.last_move_order = Some(frame);
                true
            }
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let builder = ctx.object;
        let Some(target_snapshot) = ctx.resolve(self.target) else {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            return CommandStepResult::failed(Some(update));
        };

        if !target_snapshot.is_active() {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            return CommandStepResult::failed(Some(update));
        }

        if target_snapshot.construction >= FULL_CON {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            return CommandStepResult::completed(Some(update));
        }

        let builder_actively_building = builder.action_procedure == ActionProcedure::Build
            && builder.action_target == Some(self.target);
        if builder_actively_building {
            if builder.command_direction != CommandDirection::Stop {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::running(Some(update));
            }
            return CommandStepResult::running(None);
        }

        let target_position = match self.target_position(ctx) {
            Some(position) => position,
            None => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::failed(Some(update));
            }
        };

        let same_container = builder.container == target_snapshot.container;
        let builder_inside_target = builder.container == Some(self.target);
        let target_inside_builder = target_snapshot.container == Some(builder.id);

        let close_enough = if same_container || builder_inside_target || target_inside_builder {
            true
        } else {
            let dx = target_position.x - ctx.position.x;
            let dy = target_position.y - ctx.position.y;
            dx.abs() <= self.approach_horizontal && dy.abs() <= self.approach_vertical
        };

        if close_enough {
            let action_update = ActionUpdate::default()
                .with_name("Build")
                .with_target(Some(self.target))
                .with_force(true)
                .with_phase(0)
                .with_ticks(0);
            let update = ObjectUpdate::new()
                .with_action_update(action_update)
                .with_command_direction(CommandDirection::Stop);
            return CommandStepResult::running(Some(update));
        }

        let is_structure = (builder.category & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK)) != 0;
        if is_structure && !close_enough {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            return CommandStepResult::failed(Some(update));
        }

        let mut operations = Vec::new();
        if !is_structure && self.should_issue_move(ctx.frame) {
            let mut request = CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(target_position.x))
                .with_ty(Some(target_position.y));
            if same_container {
                request = request.with_target(target_snapshot.container);
            }
            operations.push(CommandOperation::PushFront(request));
        }

        CommandStepResult::running(None).with_operations(operations)
    }
}

#[derive(Debug, Clone)]
enum CommandState {
    MoveTo(MoveToState),
    Build(BuildState),
    Unsupported,
}

#[derive(Debug, Clone)]
struct ActiveCommand {
    state: CommandState,
}

impl ActiveCommand {
    fn from_request(request: CommandRequest) -> Result<Self, CommandError> {
        let state = match request.id {
            CommandId::MoveTo => CommandState::MoveTo(MoveToState::from_request(&request)),
            CommandId::Build => CommandState::Build(BuildState::from_request(&request)?),
            _ => CommandState::Unsupported,
        };

        if matches!(state, CommandState::Unsupported) {
            return Err(CommandError::Unsupported);
        }

        Ok(Self { state })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        match &mut self.state {
            CommandState::MoveTo(state) => state.step(ctx),
            CommandState::Build(state) => state.step(ctx),
            CommandState::Unsupported => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                CommandStepResult::failed(Some(update))
            }
        }
    }
}
