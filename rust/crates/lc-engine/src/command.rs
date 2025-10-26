use std::collections::{HashMap, VecDeque};

use crate::{
    ocf, ActionProcedure, ActionUpdate, CommandDirection, DefinitionId, ObjectId, ObjectStatus,
    ObjectUpdate, Vector2, CATEGORY_STATIC_BACK, CATEGORY_STRUCTURE, CATEGORY_VEHICLE, FULL_CON,
    LINE_CONNECT_POWER_INPUT, OWNER_NONE,
};
use serde::{Deserialize, Serialize};

/// Maximum number of commands that may be queued for an object.
pub const MAX_COMMAND_STACK: usize = 35;
const LINEKIT_DEFINITION: &str = "LNKT";
const ACQUIRE_REQUEST_INTERVAL: u32 = 50;

#[derive(Debug, Clone)]
pub struct CommandObjectSnapshot {
    pub id: ObjectId,
    pub definition_id: DefinitionId,
    pub position: Vector2,
    pub status: ObjectStatus,
    pub destroyed: bool,
    pub category: i32,
    pub container: Option<ObjectId>,
    pub action_target: Option<ObjectId>,
    pub action_procedure: ActionProcedure,
    pub command_direction: CommandDirection,
    pub construction: i32,
    pub owner: i32,
    pub crew_member: bool,
    pub selected: bool,
    pub alive: bool,
    pub contents: Vec<ObjectId>,
    pub line_connect: u32,
    pub ocf: u32,
    pub collectible: bool,
}

impl CommandObjectSnapshot {
    pub fn is_active(&self) -> bool {
        !self.destroyed && self.status.is_active() && self.alive
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::ocf;

    fn snapshot_with_id(id: u64) -> CommandObjectSnapshot {
        CommandObjectSnapshot {
            id: ObjectId::new(id),
            definition_id: format!("DEF{id}"),
            position: Vector2::ZERO,
            status: ObjectStatus::Normal,
            destroyed: false,
            category: 0,
            container: None,
            action_target: None,
            action_procedure: ActionProcedure::Undefined,
            command_direction: CommandDirection::Stop,
            construction: 0,
            owner: OWNER_NONE,
            crew_member: false,
            selected: false,
            alive: true,
            contents: Vec::new(),
            line_connect: 0,
            ocf: ocf::AVAILABLE,
            collectible: false,
        }
    }

    #[test]
    fn follow_completes_for_unselected_crew() {
        let follower_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut follower = snapshot_with_id(follower_id.as_u64());
        follower.crew_member = true;
        follower.owner = 42;
        follower.selected = false;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(20, 0);
        target.crew_member = true;

        let mut objects = HashMap::new();
        objects.insert(follower.id, follower.clone());
        objects.insert(target.id, target);

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: follower.position,
            object: objects.get(&follower_id).expect("follower present"),
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = FollowState::from_request(
            &CommandRequest::new(CommandId::Follow).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
    }

    #[test]
    fn follow_requests_move_when_out_of_range() {
        let follower_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);

        let mut follower = snapshot_with_id(follower_id.as_u64());
        follower.crew_member = true;
        follower.owner = 1;
        follower.selected = true;
        follower.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 0);
        target.crew_member = true;

        let mut objects = HashMap::new();
        objects.insert(follower.id, follower.clone());
        objects.insert(target.id, target);

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: follower.position,
            object: objects.get(&follower_id).expect("follower present"),
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = FollowState::from_request(
            &CommandRequest::new(CommandId::Follow).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.update.is_some(),
            "follower should receive a stop update before moving"
        );
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, Some(target_id));
            }
            other => panic!("expected move request, got {:?}", other),
        }
    }

    #[test]
    fn attack_completes_when_target_not_crew() {
        let attacker_id = ObjectId::new(7);
        let target_id = ObjectId::new(8);

        let attacker = snapshot_with_id(attacker_id.as_u64());

        let mut target = snapshot_with_id(target_id.as_u64());
        target.crew_member = false;

        let mut objects = HashMap::new();
        objects.insert(attacker.id, attacker.clone());
        objects.insert(target.id, target);

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: attacker.position,
            object: objects.get(&attacker_id).expect("attacker present"),
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = AttackState::from_request(
            &CommandRequest::new(CommandId::Attack).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
    }

    #[test]
    fn attack_requests_move_when_out_of_range() {
        let attacker_id = ObjectId::new(30);
        let target_id = ObjectId::new(40);

        let mut attacker = snapshot_with_id(attacker_id.as_u64());
        attacker.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.crew_member = true;
        target.position = Vector2::new(200, -50);

        let mut objects = HashMap::new();
        objects.insert(attacker.id, attacker.clone());
        objects.insert(target.id, target);

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: attacker.position,
            object: objects.get(&attacker_id).expect("attacker present"),
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = AttackState::from_request(
            &CommandRequest::new(CommandId::Attack).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.update.is_some(),
            "attacker should stop before chasing target"
        );
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, Some(target_id));
            }
            other => panic!("expected move request, got {:?}", other),
        }
    }

    #[test]
    fn build_queues_activate_for_internal_vehicle() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.construction = FULL_CON;
        target.category = CATEGORY_VEHICLE;
        target.container = Some(builder_id);

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = BuildState::from_request(
            &CommandRequest::new(CommandId::Build).with_target(Some(target_id)),
        )
        .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Activate);
                assert_eq!(request.target, Some(target_id));
            }
            other => panic!("expected activate request, got {:?}", other),
        }
    }

    #[test]
    fn build_queues_energy_for_structures_needing_power() {
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);

        let builder = snapshot_with_id(builder_id.as_u64());

        let mut target = snapshot_with_id(target_id.as_u64());
        target.construction = FULL_CON;
        target.line_connect = LINE_CONNECT_POWER_INPUT;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            structures_need_energy: true,
        };

        let mut state = BuildState::from_request(
            &CommandRequest::new(CommandId::Build).with_target(Some(target_id)),
        )
        .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Energy);
                assert_eq!(request.target, Some(target_id));
            }
            other => panic!("expected energy request, got {:?}", other),
        }
    }

    #[test]
    fn acquire_completes_when_inventory_contains_item() {
        let builder_id = ObjectId::new(1);
        let item_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(builder_id);

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
    }

    #[test]
    fn acquire_requests_move_for_nearby_item() {
        let builder_id = ObjectId::new(10);
        let item_id = ObjectId::new(20);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.position = Vector2::new(100, 0);
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, Some(item_id));
            }
            other => panic!("expected move request, got {:?}", other),
        }
    }

    #[test]
    fn acquire_transfers_item_from_shared_container() {
        let builder_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.container = Some(container_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(0, 0);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.construction = FULL_CON;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.container = Some(container_id);

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(container.id, container);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 42,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);

        match &result.events[0] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(update.container, Some(Some(builder_id)));
                assert_eq!(update.position, Some(builder_snapshot.position));
            }
        }
    }

    #[test]
    fn acquire_enters_container_when_adjacent() {
        let builder_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.position = Vector2::new(0, 0);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(4, 0);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.construction = FULL_CON;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.container = Some(container_id);
        item.position = container.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(container.id, container);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 100,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            structures_need_energy: false,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.is_empty());
        let update = result.update.expect("builder update");
        assert_eq!(update.container, Some(Some(container_id)));
        assert_eq!(update.position, Some(Vector2::new(4, 0)));
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CommandEvent {
    ApplyObjectUpdate {
        object_id: ObjectId,
        update: ObjectUpdate,
    },
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
    pub events: Vec<CommandEvent>,
}

impl CommandStepResult {
    pub fn running(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Running,
            operations: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn completed(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Completed,
            operations: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn failed(update: Option<ObjectUpdate>) -> Self {
        Self {
            update,
            status: CommandStatus::Failed,
            operations: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn with_operations(mut self, operations: Vec<CommandOperation>) -> Self {
        self.operations = operations;
        self
    }
}

fn c4id_to_definition_string(id: i32) -> Option<DefinitionId> {
    if id == 0 {
        return None;
    }
    if (0..=9999).contains(&id) {
        return Some(format!("{id:04}"));
    }
    let bytes = id.to_le_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == 0 {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

fn definition_id_to_c4id(definition: &str) -> Option<i32> {
    if definition.is_empty() {
        return None;
    }
    if definition.chars().all(|ch| ch.is_ascii_digit()) && definition.len() <= 4 {
        return definition.parse::<i32>().ok();
    }
    let mut bytes = [0u8; 4];
    for (idx, ch) in definition.chars().take(4).enumerate() {
        bytes[idx] = ch as u8;
    }
    Some(i32::from_le_bytes(bytes))
}

fn command_data_to_definition_id(data: &CommandData) -> Option<DefinitionId> {
    match data {
        CommandData::Integer(value) => c4id_to_definition_string(*value),
        CommandData::Text(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

#[derive(Clone)]
pub struct CommandRuntimeContext<'a> {
    pub frame: u64,
    pub position: Vector2,
    pub object: &'a CommandObjectSnapshot,
    pub objects: &'a HashMap<ObjectId, CommandObjectSnapshot>,
    pub structures_need_energy: bool,
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
            let mut operations = Vec::new();
            if target_snapshot.container.is_some()
                && (target_snapshot.category & CATEGORY_VEHICLE) != 0
            {
                operations.push(CommandOperation::PushFront(
                    CommandRequest::new(CommandId::Activate).with_target(Some(self.target)),
                ));
            }

            if ctx.structures_need_energy
                && (target_snapshot.line_connect & LINE_CONNECT_POWER_INPUT) != 0
            {
                operations.push(CommandOperation::PushFront(
                    CommandRequest::new(CommandId::Energy).with_target(Some(self.target)),
                ));
            }

            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            return CommandStepResult::completed(Some(update)).with_operations(operations);
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
struct FollowState {
    target: ObjectId,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
}

impl FollowState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
        })
    }

    fn should_issue_move(&mut self, frame: u64) -> bool {
        const MOVE_COOLDOWN: u64 = 12;
        match self.last_move_order {
            Some(last) if frame.saturating_sub(last) < MOVE_COOLDOWN => false,
            _ => {
                self.last_move_order = Some(frame);
                true
            }
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

        let follower = ctx.object;

        if follower.crew_member && follower.owner != OWNER_NONE && !follower.selected {
            let update = if follower.command_direction != CommandDirection::Stop {
                Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
            } else {
                None
            };
            return CommandStepResult::completed(update);
        }

        let target = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::failed(Some(update));
            }
        };

        if !target.is_active() {
            let update = if follower.command_direction != CommandDirection::Stop {
                Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
            } else {
                None
            };
            return CommandStepResult::completed(update);
        }

        if follower.id == target.id {
            return CommandStepResult::completed(None);
        }

        if follower.container != target.container {
            if follower.crew_member {
                let update = if follower.command_direction != CommandDirection::Stop {
                    Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
                } else {
                    None
                };
                return CommandStepResult::completed(update);
            }
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            return CommandStepResult::failed(Some(update));
        }

        const FOLLOW_RANGE: i32 = 6;
        let dx = target.position.x - ctx.position.x;
        let dy = target.position.y - ctx.position.y;
        if dx.abs() <= FOLLOW_RANGE && dy.abs() <= FOLLOW_RANGE {
            if follower.command_direction != target.command_direction {
                let update = ObjectUpdate::new().with_command_direction(target.command_direction);
                return CommandStepResult::running(Some(update));
            }
            return CommandStepResult::running(None);
        }

        let update = if follower.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        };
        let mut result = CommandStepResult::running(update);
        if self.should_issue_move(ctx.frame) {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(self.target))
                .with_update_interval(10);
            result = result.with_operations(vec![CommandOperation::PushFront(request)]);
        }
        result
    }
}

#[derive(Debug, Clone)]
struct AttackState {
    target: ObjectId,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
}

impl AttackState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
        })
    }

    fn should_issue_move(&mut self, frame: u64) -> bool {
        const MOVE_COOLDOWN: u64 = 8;
        match self.last_move_order {
            Some(last) if frame.saturating_sub(last) < MOVE_COOLDOWN => false,
            _ => {
                self.last_move_order = Some(frame);
                true
            }
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

        let attacker = ctx.object;
        let target = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::failed(Some(update));
            }
        };

        if !target.is_active() {
            let update = if attacker.command_direction != CommandDirection::Stop {
                Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
            } else {
                None
            };
            return CommandStepResult::completed(update);
        }

        if !target.crew_member {
            let update = if attacker.command_direction != CommandDirection::Stop {
                Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
            } else {
                None
            };
            return CommandStepResult::completed(update);
        }

        const ATTACK_RANGE: i32 = 12;
        let dx = target.position.x - ctx.position.x;
        let dy = target.position.y - ctx.position.y;
        if dx.abs() <= ATTACK_RANGE && dy.abs() <= ATTACK_RANGE {
            if attacker.command_direction != CommandDirection::Stop {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::running(Some(update));
            }
            return CommandStepResult::running(None);
        }

        let update = if attacker.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        };
        let mut result = CommandStepResult::running(update);
        if self.should_issue_move(ctx.frame) {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(self.target))
                .with_update_interval(10);
            result = result.with_operations(vec![CommandOperation::PushFront(request)]);
        }
        result
    }
}

#[derive(Debug, Clone)]
struct AcquireState {
    definition_id: DefinitionId,
    ignore_container: Option<ObjectId>,
    range_x: i32,
    range_y: i32,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    candidate: Option<ObjectId>,
}

impl AcquireState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let definition_id =
            command_data_to_definition_id(&request.data).ok_or(CommandError::Unsupported)?;
        let raw_range_x = request.tx.unwrap_or(0);
        let raw_range_y = request.ty.unwrap_or(0);
        let range_x = if raw_range_x == 0 {
            500
        } else {
            raw_range_x.abs()
        };
        let range_y = if raw_range_y == 0 {
            250
        } else {
            raw_range_y.abs()
        };
        Ok(Self {
            definition_id,
            ignore_container: request.target2,
            range_x,
            range_y,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            candidate: None,
        })
    }

    fn update_to_stop(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
    }

    fn should_issue_move(&mut self, frame: u64) -> bool {
        const MOVE_COOLDOWN: u64 = 12;
        match self.last_move_order {
            Some(last) if frame.saturating_sub(last) < MOVE_COOLDOWN => false,
            _ => {
                self.last_move_order = Some(frame);
                true
            }
        }
    }

    fn candidate_is_valid(
        &self,
        candidate: &CommandObjectSnapshot,
        ctx: &CommandRuntimeContext<'_>,
    ) -> bool {
        if candidate.destroyed || !candidate.status.is_active() || !candidate.alive {
            return false;
        }
        if candidate.definition_id != self.definition_id {
            return false;
        }
        if candidate.id == ctx.object.id {
            return false;
        }
        if candidate.ocf & ocf::AVAILABLE == 0 {
            return false;
        }
        if candidate.construction < FULL_CON {
            return false;
        }
        if let Some(ignore) = self.ignore_container {
            if candidate.container == Some(ignore) {
                return false;
            }
        }
        if let Some(container) = candidate.container {
            if container == ctx.object.id {
                return false;
            }
        } else if !candidate.collectible {
            return false;
        }
        true
    }

    fn find_candidate(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        let mut best: Option<(ObjectId, i32)> = None;
        for (id, snapshot) in ctx.objects.iter() {
            if !self.candidate_is_valid(snapshot, ctx) {
                continue;
            }
            let dx = snapshot.position.x - ctx.position.x;
            let dy = snapshot.position.y - ctx.position.y;
            if dx.abs() > self.range_x || dy.abs() > self.range_y {
                continue;
            }
            let distance = dx.abs() + dy.abs();
            if best.map_or(true, |(_, best_dist)| distance < best_dist) {
                best = Some((*id, distance));
            }
        }
        best.map(|(id, _)| id)
    }

    fn handle_container_candidate(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        candidate_id: ObjectId,
        container_id: ObjectId,
    ) -> CommandStepResult {
        let base_update = self.update_to_stop(ctx);
        let builder_container = ctx.object.container;

        if builder_container == Some(container_id) {
            let mut result = CommandStepResult::running(base_update.clone());
            let mut transfer_update = ObjectUpdate::new();
            transfer_update.container = Some(Some(ctx.object.id));
            transfer_update.position = Some(ctx.position);
            transfer_update.velocity = Some(Vector2::ZERO);
            result.events.push(CommandEvent::ApplyObjectUpdate {
                object_id: candidate_id,
                update: transfer_update,
            });
            return result;
        }

        let Some(container_snapshot) = ctx.resolve(container_id) else {
            return CommandStepResult::running(base_update);
        };

        if builder_container.is_none() {
            let dx = container_snapshot.position.x - ctx.position.x;
            let dy = container_snapshot.position.y - ctx.position.y;
            const CONTAINER_APPROACH_RANGE: i32 = 12;
            if dx.abs() <= CONTAINER_APPROACH_RANGE && dy.abs() <= CONTAINER_APPROACH_RANGE {
                let mut update = base_update.clone().unwrap_or_default();
                update.container = Some(Some(container_id));
                update.position = Some(container_snapshot.position);
                update.velocity = Some(Vector2::ZERO);
                if update.command_direction.is_none() {
                    update.command_direction = Some(CommandDirection::Stop);
                }
                return CommandStepResult::running(Some(update));
            }
        }

        if self.should_issue_move(ctx.frame) {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(container_id))
                .with_update_interval(10);
            return CommandStepResult::running(base_update)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        CommandStepResult::running(base_update)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let has_item = ctx
            .object
            .contents
            .iter()
            .filter_map(|id| ctx.resolve(*id))
            .any(|snapshot| snapshot.definition_id == self.definition_id);

        if has_item {
            return CommandStepResult::completed(None);
        }

        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        if let Some(candidate_id) = self.candidate {
            let candidate = ctx.resolve(candidate_id);
            if candidate
                .filter(|snapshot| self.candidate_is_valid(snapshot, ctx))
                .is_none()
            {
                self.candidate = None;
            }
        }

        if self.candidate.is_none() {
            self.candidate = self.find_candidate(ctx);
        }

        let Some(candidate_id) = self.candidate else {
            return CommandStepResult::running(self.update_to_stop(ctx));
        };

        let Some(candidate) = ctx.resolve(candidate_id) else {
            self.candidate = None;
            return CommandStepResult::running(self.update_to_stop(ctx));
        };

        let dx = candidate.position.x - ctx.position.x;
        let dy = candidate.position.y - ctx.position.y;
        const PICKUP_RANGE: i32 = 12;
        if dx.abs() <= PICKUP_RANGE && dy.abs() <= PICKUP_RANGE {
            if let Some(container_id) = candidate.container {
                if container_id != ctx.object.id {
                    return self.handle_container_candidate(ctx, candidate_id, container_id);
                }
            }
            return CommandStepResult::running(self.update_to_stop(ctx));
        }

        if self.should_issue_move(ctx.frame) {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(candidate_id))
                .with_update_interval(10);
            let operations = vec![CommandOperation::PushFront(request)];
            return CommandStepResult::running(self.update_to_stop(ctx))
                .with_operations(operations);
        }

        CommandStepResult::running(self.update_to_stop(ctx))
    }
}

#[derive(Debug, Clone)]
struct EnergyState {
    target: ObjectId,
    acquire_requested: bool,
}

impl EnergyState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            acquire_requested: false,
        })
    }

    fn builder_has_linekit(&self, ctx: &CommandRuntimeContext<'_>) -> bool {
        ctx.object
            .contents
            .iter()
            .filter_map(|id| ctx.resolve(*id))
            .any(|snapshot| snapshot.definition_id == LINEKIT_DEFINITION)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let update_to_stop = || {
            if ctx.object.command_direction != CommandDirection::Stop {
                Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
            } else {
                None
            }
        };

        let Some(target_snapshot) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(update_to_stop());
        };

        if !target_snapshot.is_active() {
            return CommandStepResult::failed(update_to_stop());
        }

        if !ctx.structures_need_energy
            || (target_snapshot.line_connect & LINE_CONNECT_POWER_INPUT) == 0
        {
            return CommandStepResult::completed(None);
        }

        if self.builder_has_linekit(ctx) {
            return CommandStepResult::completed(None);
        }

        if self.acquire_requested {
            return CommandStepResult::running(update_to_stop());
        }

        let mut operations = Vec::new();
        if let Some(c4id) = definition_id_to_c4id(LINEKIT_DEFINITION) {
            let request = CommandRequest::new(CommandId::Acquire)
                .with_data(CommandData::Integer(c4id))
                .with_update_interval(ACQUIRE_REQUEST_INTERVAL);
            operations.push(CommandOperation::PushFront(request));
            self.acquire_requested = true;
        }

        CommandStepResult::running(update_to_stop()).with_operations(operations)
    }
}

#[derive(Debug, Clone)]
enum CommandState {
    Follow(FollowState),
    MoveTo(MoveToState),
    Build(BuildState),
    Attack(AttackState),
    Acquire(AcquireState),
    Energy(EnergyState),
    Unsupported,
}

#[derive(Debug, Clone)]
struct ActiveCommand {
    state: CommandState,
}

impl ActiveCommand {
    fn from_request(request: CommandRequest) -> Result<Self, CommandError> {
        let state = match request.id {
            CommandId::Follow => CommandState::Follow(FollowState::from_request(&request)?),
            CommandId::MoveTo => CommandState::MoveTo(MoveToState::from_request(&request)),
            CommandId::Build => CommandState::Build(BuildState::from_request(&request)?),
            CommandId::Attack => CommandState::Attack(AttackState::from_request(&request)?),
            CommandId::Acquire => CommandState::Acquire(AcquireState::from_request(&request)?),
            CommandId::Energy => CommandState::Energy(EnergyState::from_request(&request)?),
            _ => CommandState::Unsupported,
        };

        if matches!(state, CommandState::Unsupported) {
            return Err(CommandError::Unsupported);
        }

        Ok(Self { state })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        match &mut self.state {
            CommandState::Follow(state) => state.step(ctx),
            CommandState::MoveTo(state) => state.step(ctx),
            CommandState::Build(state) => state.step(ctx),
            CommandState::Attack(state) => state.step(ctx),
            CommandState::Acquire(state) => state.step(ctx),
            CommandState::Energy(state) => state.step(ctx),
            CommandState::Unsupported => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                CommandStepResult::failed(Some(update))
            }
        }
    }
}
