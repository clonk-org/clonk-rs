use std::collections::{HashMap, VecDeque};

use crate::{
    ocf, ActionProcedure, ActionUpdate, CommandDirection, DefinitionId, Direction, ObjectId,
    ObjectStatus, ObjectUpdate, PlayerStatus, Vector2, CATEGORY_STATIC_BACK, CATEGORY_STRUCTURE,
    CATEGORY_VEHICLE, FULL_CON, LINE_CONNECT_POWER_INPUT, OWNER_NONE,
};
use serde::{Deserialize, Serialize};

/// Maximum number of commands that may be queued for an object.
pub const MAX_COMMAND_STACK: usize = 35;
const LINEKIT_DEFINITION: &str = "LNKT";
const ACQUIRE_REQUEST_INTERVAL: u32 = 50;
const COMMAND_FLAG_ENTER_PUSH_TARGET: i32 = 0b10;
const DIG_MOVE_TO_RANGE_DEFAULT: i32 = 5;
const DIG_DIRECTION_RANGE: i32 = 1;

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

#[derive(Debug, Clone)]
pub struct CommandPlayerSnapshot {
    pub status: PlayerStatus,
    pub surrendered: bool,
    pub wealth: i32,
    pub home_base_material: HashMap<DefinitionId, u32>,
}

impl CommandPlayerSnapshot {
    pub fn is_active(&self) -> bool {
        matches!(self.status, PlayerStatus::Active) && !self.surrendered
    }

    pub fn material_count(&self, definition_id: &str) -> u32 {
        self.home_base_material
            .get(definition_id)
            .copied()
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDefinitionSnapshot {
    pub value: i32,
    #[serde(default)]
    pub can_chop: bool,
    #[serde(default)]
    pub chop_action: Option<String>,
}

/// Identifiers that map to the classic C4 command constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: follower.position,
            object: objects.get(&follower_id).expect("follower present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: follower.position,
            object: objects.get(&follower_id).expect("follower present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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
    fn wait_stops_dig_and_completes_after_interval() {
        let actor_id = ObjectId::new(50);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Dig;
        actor.command_direction = CommandDirection::Left;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Wait).with_update_interval(3);
        let mut state = WaitState::from_request(&request);

        let ctx0 = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let result0 = state.step(&ctx0);
        assert_eq!(result0.status, CommandStatus::Running);
        let update0 = result0
            .update
            .expect("wait should issue an update to stop digging");
        assert_eq!(update0.command_direction, Some(CommandDirection::Stop));
        let action_update = update0.action.expect("wait should reset the action");
        assert_eq!(action_update.name.as_deref(), Some("Idle"));

        let ctx1 = CommandRuntimeContext {
            frame: 1,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let result1 = state.step(&ctx1);
        assert_eq!(result1.status, CommandStatus::Running);

        let ctx2 = CommandRuntimeContext {
            frame: 2,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let result2 = state.step(&ctx2);
        assert_eq!(result2.status, CommandStatus::Completed);
    }

    #[test]
    fn get_transfers_item_when_in_range() {
        let actor_id = ObjectId::new(100);
        let target_id = ObjectId::new(200);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut item = snapshot_with_id(target_id.as_u64());
        item.position = Vector2::new(8, 0);
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);

        match &result.events[0] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, target_id);
                assert_eq!(update.container, Some(Some(actor_id)));
                assert_eq!(update.position, Some(actor_snapshot.position));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn get_requests_exit_when_actor_contained() {
        let actor_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let target_id = ObjectId::new(3);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(0, 0);

        let mut item = snapshot_with_id(target_id.as_u64());
        item.position = Vector2::new(20, 0);
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Exit);
            }
            other => panic!("expected exit request, got {:?}", other),
        }
    }

    #[test]
    fn get_requests_ungrab_when_pushing_other_target() {
        let actor_id = ObjectId::new(1);
        let pushed_id = ObjectId::new(2);
        let container_id = ObjectId::new(3);
        let target_id = ObjectId::new(4);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut pushed = snapshot_with_id(pushed_id.as_u64());
        pushed.position = Vector2::new(0, 0);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.ocf = ocf::AVAILABLE | ocf::GRAB;
        container.position = Vector2::new(10, 0);

        let mut item = snapshot_with_id(target_id.as_u64());
        item.container = Some(container_id);
        item.position = container.position;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(pushed.id, pushed);
        objects.insert(container.id, container);
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::UnGrab);
            }
            other => panic!("expected ungrab request, got {:?}", other),
        }
    }

    #[test]
    fn get_fails_for_non_collectible_target() {
        let actor_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;

        let mut item = snapshot_with_id(target_id.as_u64());
        item.position = Vector2::new(8, 0);
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = false;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn put_transfers_item_into_target_container() {
        let actor_id = ObjectId::new(600);
        let item_id = ObjectId::new(601);
        let container_id = ObjectId::new(602);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(50, 50);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        item.position = actor.position;

        let mut target_container = snapshot_with_id(container_id.as_u64());
        target_container.position = Vector2::new(54, 48);
        target_container.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(target_container.id, target_container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(container_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(update.container, Some(Some(container_id)));
                assert_eq!(update.position, Some(target_container.position));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn put_requests_exit_when_actor_in_other_container() {
        let actor_id = ObjectId::new(610);
        let item_id = ObjectId::new(611);
        let target_container_id = ObjectId::new(612);
        let current_container_id = ObjectId::new(613);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(current_container_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut target_container = snapshot_with_id(target_container_id.as_u64());
        target_container.position = Vector2::new(0, 0);

        let mut current_container = snapshot_with_id(current_container_id.as_u64());
        current_container.position = Vector2::new(-20, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(target_container.id, target_container);
        objects.insert(current_container.id, current_container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(target_container_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Exit);
            }
            other => panic!("expected exit request, got {:?}", other),
        }
    }

    #[test]
    fn put_requests_move_when_far_from_target() {
        let actor_id = ObjectId::new(620);
        let item_id = ObjectId::new(621);
        let container_id = ObjectId::new(622);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(80, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(container.id, container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(container_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.iter().any(|operation| match operation {
                CommandOperation::PushFront(request) => request.id == CommandId::MoveTo,
                _ => false,
            }),
            "put should request movement when far from container"
        );
    }

    #[test]
    fn drop_transfers_item_to_ground() {
        let actor_id = ObjectId::new(630);
        let item_id = ObjectId::new(631);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        item.position = actor.position;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = DropState::from_request(&CommandRequest::new(CommandId::Drop));
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(update.container, Some(None));
                assert_eq!(update.position, Some(actor.position));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn drop_requests_move_to_coordinates() {
        let actor_id = ObjectId::new(640);
        let item_id = ObjectId::new(641);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = DropState::from_request(
            &CommandRequest::new(CommandId::Drop)
                .with_tx(Some(120))
                .with_ty(Some(0)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.iter().any(|operation| match operation {
                CommandOperation::PushFront(request) => {
                    request.id == CommandId::MoveTo && request.tx == Some(120) && request.ty == Some(0)
                }
                _ => false,
            }),
            "drop should request movement towards target coordinates"
        );
    }

    #[test]
    fn drop_delegates_put_when_actor_contained() {
        let actor_id = ObjectId::new(650);
        let item_id = ObjectId::new(651);
        let container_id = ObjectId::new(652);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(0, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(container.id, container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = DropState::from_request(&CommandRequest::new(CommandId::Drop));
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Put);
                assert_eq!(request.target, Some(container_id));
                assert_eq!(request.target2, Some(item_id));
            }
            other => panic!("expected delegated put request, got {:?}", other),
        }
    }

    #[test]
    fn drop_delegates_put_when_pushing_target() {
        let actor_id = ObjectId::new(660);
        let item_id = ObjectId::new(661);
        let pushed_id = ObjectId::new(662);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(pushed_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);

        let mut pushed = snapshot_with_id(pushed_id.as_u64());
        pushed.position = Vector2::new(0, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(pushed.id, pushed);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = DropState::from_request(&CommandRequest::new(CommandId::Drop));
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Put);
                assert_eq!(request.target, Some(pushed_id));
                assert_eq!(request.target2, Some(item_id));
            }
            other => panic!("expected delegated put request, got {:?}", other),
        }
    }

    #[test]
    fn dig_requests_ungrab_when_pushing() {
        let actor_id = ObjectId::new(60);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.command_direction = CommandDirection::Right;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(15))
            .with_ty(Some(25));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.iter().any(|operation| match operation {
                CommandOperation::PushFront(request) => request.id == CommandId::UnGrab,
                _ => false,
            }),
            "dig should request ungrab when pushing"
        );
    }

    #[test]
    fn dig_requests_exit_when_contained() {
        let actor_id = ObjectId::new(61);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(ObjectId::new(99));

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(0))
            .with_ty(Some(0));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(
            result.operations.iter().any(|operation| match operation {
                CommandOperation::PushFront(request) => request.id == CommandId::Exit,
                _ => false,
            }),
            "dig should request exit when contained"
        );
    }

    #[test]
    fn dig_sets_dig_action_when_walking() {
        let actor_id = ObjectId::new(62);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.command_direction = CommandDirection::Stop;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(actor.position.x))
            .with_ty(Some(actor.position.y + 20))
            .with_data(CommandData::Integer(1));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("dig should issue an update");
        assert_eq!(
            update.command_direction,
            Some(CommandDirection::Down),
            "dig should direct the crew towards the target"
        );
        let action_update = update.action.expect("dig should start the dig action");
        assert_eq!(action_update.name.as_deref(), Some("Dig"));
        assert_eq!(action_update.data, Some(1));
    }

    #[test]
    fn dig_completes_when_within_move_range() {
        let actor_id = ObjectId::new(63);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Dig;
        actor.command_direction = CommandDirection::Left;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(actor.position.x))
            .with_ty(Some(actor.position.y));
        let mut state = DigState::from_request(&request).expect("state created");

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("dig should stop when done");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        let action_update = update.action.expect("dig should reset to idle");
        assert_eq!(action_update.name.as_deref(), Some("Idle"));
    }

    #[test]
    fn retry_command_waits_then_completes() {
        let actor = snapshot_with_id(60);
        let actor_id = actor.id;

        let mut objects = HashMap::new();
        objects.insert(actor_id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Retry).with_update_interval(3))
            .expect("retry command accepted");

        for frame in 0..2 {
            let ctx = CommandRuntimeContext {
                frame,
                position: actor.position,
                object: objects.get(&actor_id).expect("actor present"),
                objects: &objects,
                players: &players,
                definitions: &definitions,
                structures_need_energy: false,
                base_buy_enabled: true,
            };
            let result = stack.step(&ctx).expect("running result");
            assert_eq!(result.status, CommandStatus::Running);
            assert!(result.update.is_none());
        }

        let ctx = CommandRuntimeContext {
            frame: 2,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };
        let result = stack.step(&ctx).expect("completion result");
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
    }

    #[test]
    fn enter_enters_target_when_in_range() {
        let actor_id = ObjectId::new(30);
        let target_id = ObjectId::new(40);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(18, 16);
        target.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        target.category = CATEGORY_STRUCTURE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("enter should produce an update");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(update.container, Some(Some(target_id)));
        assert_eq!(update.position, Some(Vector2::new(18, 16)));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
        assert!(result.operations.is_empty());
    }

    #[test]
    fn enter_requests_move_when_far() {
        let actor_id = ObjectId::new(31);
        let target_id = ObjectId::new(41);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(120, 0);
        target.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        target.category = CATEGORY_STRUCTURE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 5,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result
            .update
            .expect("enter should stop actor before requesting movement");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(result.events.is_empty());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, Some(target_id));
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn grab_requests_move_when_far() {
        let actor_id = ObjectId::new(200);
        let target_id = ObjectId::new(300);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);
        actor.command_direction = CommandDirection::Left;
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(60, 0);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result
            .update
            .expect("grab should stop actor before requesting movement");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(result.events.is_empty());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(target.position.x));
                assert_eq!(request.ty, Some(target.position.y));
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn grab_starts_push_when_in_range() {
        let actor_id = ObjectId::new(310);
        let target_id = ObjectId::new(320);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        actor.command_direction = CommandDirection::Right;
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(14, 12);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 1,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        let update = result.update.expect("grab should update action");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        let action = update.action.expect("grab should set push action");
        assert_eq!(action.name.as_deref(), Some("Push"));
        assert_eq!(action.target, Some(Some(target_id)));
    }

    #[test]
    fn grab_completes_when_already_pushing_target() {
        let actor_id = ObjectId::new(330);
        let target_id = ObjectId::new(340);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 2,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn grab_requests_ungrab_when_pushing_other_target() {
        let actor_id = ObjectId::new(350);
        let target_id = ObjectId::new(360);
        let other_id = ObjectId::new(361);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(other_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(15, 0);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut other = snapshot_with_id(other_id.as_u64());
        other.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(other.id, other);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 3,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::UnGrab);
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn ungrab_sets_idle_and_completes() {
        let actor_id = ObjectId::new(370);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.command_direction = CommandDirection::Left;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 4,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = UnGrabState::from_request(&CommandRequest::new(CommandId::UnGrab));

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("ungrab should update actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        let action = update.action.expect("ungrab should reset action");
        assert_eq!(action.name.as_deref(), Some("Idle"));
        assert_eq!(action.target, Some(None));
    }

    #[test]
    fn ungrab_completes_without_update_when_not_pushing() {
        let actor_id = ObjectId::new(380);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.command_direction = CommandDirection::Stop;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 5,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = UnGrabState::from_request(&CommandRequest::new(CommandId::UnGrab));

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn jump_sets_direction_and_action_when_walking() {
        let actor_id = ObjectId::new(400);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 6,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = JumpState::from_request(
            &CommandRequest::new(CommandId::Jump).with_tx(Some(actor.position.x + 10)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("jump should update actor");
        assert_eq!(update.direction, Some(Direction::Right));
        let action = update.action.expect("jump should trigger action");
        assert_eq!(action.name.as_deref(), Some("Jump"));
    }

    #[test]
    fn jump_skips_action_when_not_walking() {
        let actor_id = ObjectId::new(401);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Hang;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 7,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = JumpState::from_request(
            &CommandRequest::new(CommandId::Jump).with_tx(Some(actor.position.x - 15)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("jump should update actor");
        assert_eq!(update.direction, Some(Direction::Left));
        assert!(
            update.action.is_none(),
            "jump should not change action when not walking"
        );
    }

    #[test]
    fn throw_requests_acquire_when_item_missing() {
        let actor_id = ObjectId::new(410);
        let target_id = ObjectId::new(420);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.contents.clear();

        let mut item = snapshot_with_id(target_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 32,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(target_id))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Acquire);
                assert_eq!(request.mode, CommandMode::Sub);
                match &request.data {
                    CommandData::Text(text) => assert_eq!(text, "STON"),
                    other => panic!("unexpected acquire data: {:?}", other),
                }
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn throw_pushes_move_to_target_when_out_of_range() {
        let actor_id = ObjectId::new(430);
        let target_id = ObjectId::new(440);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.contents = vec![target_id];

        let mut item = snapshot_with_id(target_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 48,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(target_id))
                .with_tx(Some(actor.position.x + 64))
                .with_ty(Some(actor.position.y))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(actor.position.x + 64));
                assert_eq!(request.ty, Some(actor.position.y));
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn throw_sets_throw_action_when_in_range() {
        let actor_id = ObjectId::new(450);
        let target_id = ObjectId::new(460);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 200);
        actor.contents = vec![target_id];

        let mut item = snapshot_with_id(target_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 52,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(target_id))
                .with_tx(Some(actor.position.x + 8))
                .with_ty(Some(actor.position.y))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("throw should update actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        let action = update.action.expect("throw should set action");
        assert_eq!(action.name.as_deref(), Some("Throw"));
        assert_eq!(action.target, Some(Some(target_id)));
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn throw_requests_ungrab_when_pushing() {
        let actor_id = ObjectId::new(470);
        let push_target_id = ObjectId::new(471);
        let item_id = ObjectId::new(472);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(push_target_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "STON".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut push_target = snapshot_with_id(push_target_id.as_u64());
        push_target.ocf = ocf::GRAB;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(push_target.id, push_target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 60,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(item_id))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::UnGrab);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn home_completes_when_already_in_owner_base() {
        let builder_id = ObjectId::new(510);
        let base_id = ObjectId::new(520);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 7;
        builder.container = Some(base_id);

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 7;
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        base.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state =
            HomeState::from_request(&CommandRequest::new(CommandId::Home)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn home_requests_enter_when_not_in_base() {
        let builder_id = ObjectId::new(530);
        let base_id = ObjectId::new(540);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 11;
        builder.position = Vector2::new(0, 0);

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 11;
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        base.position = Vector2::new(100, 0);
        base.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state =
            HomeState::from_request(&CommandRequest::new(CommandId::Home)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        if let Some(update) = &result.update {
            assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        }
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(base_id));
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn home_fails_when_no_base_available() {
        let builder_id = ObjectId::new(550);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 23;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state =
            HomeState::from_request(&CommandRequest::new(CommandId::Home)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
    }

    #[test]
    fn exit_completes_when_not_contained() {
        let actor_id = ObjectId::new(51);
        let actor = snapshot_with_id(actor_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn exit_moves_into_parent_container_when_nested() {
        let actor_id = ObjectId::new(60);
        let container_id = ObjectId::new(70);
        let parent_id = ObjectId::new(80);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.command_direction = CommandDirection::Right;

        let mut container = snapshot_with_id(container_id.as_u64());
        container.container = Some(parent_id);
        container.position = Vector2::new(12, 34);

        let mut parent = snapshot_with_id(parent_id.as_u64());
        parent.position = Vector2::new(100, -20);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);
        objects.insert(parent.id, parent.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 10,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("exit should update actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(update.container, Some(Some(parent_id)));
        assert_eq!(update.position, Some(parent.position));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
    }

    #[test]
    fn exit_leaves_container_when_no_parent() {
        let actor_id = ObjectId::new(90);
        let container_id = ObjectId::new(100);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.command_direction = CommandDirection::Left;

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(-40, 5);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 20,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("exit should update actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(update.container, Some(None));
        assert_eq!(update.position, Some(container.position));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
    }

    #[test]
    fn exit_stops_building_procedure() {
        let actor_id = ObjectId::new(110);
        let container_id = ObjectId::new(120);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.action_procedure = ActionProcedure::Build;

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(0, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 30,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("exit should update actor");
        let action = update.action.expect("exit should reset action");
        assert_eq!(action.name.as_deref(), Some("Idle"));
        assert!(action.force);
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

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: attacker.position,
            object: objects.get(&attacker_id).expect("attacker present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: attacker.position,
            object: objects.get(&attacker_id).expect("attacker present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: true,
            base_buy_enabled: true,
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
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 42,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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
            other => panic!("unexpected event: {:?}", other),
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
        container.ocf = ocf::AVAILABLE | ocf::ENTRANCE;

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
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 100,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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

    #[test]
    fn acquire_requests_buy_when_no_candidate() {
        let builder_id = ObjectId::new(10);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
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
                assert_eq!(request.id, CommandId::Buy);
                assert_eq!(request.update_interval, 100);
            }
            other => panic!("expected buy request, got {:?}", other),
        }

        let later_ctx = CommandRuntimeContext {
            frame: 10,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let second = state.step(&later_ctx);
        assert!(second.operations.is_empty());
    }

    #[test]
    fn acquire_exits_other_container_before_access() {
        let builder_id = ObjectId::new(1);
        let current_container_id = ObjectId::new(2);
        let target_container_id = ObjectId::new(3);
        let item_id = ObjectId::new(4);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.container = Some(current_container_id);

        let mut current_container = snapshot_with_id(current_container_id.as_u64());
        current_container.position = Vector2::new(5, 5);
        current_container.ocf = ocf::AVAILABLE | ocf::ENTRANCE;

        let mut target_container = snapshot_with_id(target_container_id.as_u64());
        target_container.position = Vector2::new(20, 0);
        target_container.ocf = ocf::AVAILABLE | ocf::ENTRANCE;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.construction = FULL_CON;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.container = Some(target_container_id);
        item.position = target_container.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(current_container.id, current_container.clone());
        objects.insert(target_container.id, target_container);
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        let update = result.update.expect("builder update");
        assert_eq!(update.container, Some(None));
        assert_eq!(update.position, Some(current_container.position));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
    }

    #[test]
    fn acquire_leaves_container_for_loose_item() {
        let builder_id = ObjectId::new(1);
        let current_container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.container = Some(current_container_id);

        let mut current_container = snapshot_with_id(current_container_id.as_u64());
        current_container.position = Vector2::new(5, 5);
        current_container.ocf = ocf::AVAILABLE | ocf::ENTRANCE;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.construction = FULL_CON;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.position = Vector2::new(30, 0);

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(current_container.id, current_container.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        let update = result.update.expect("builder update");
        assert_eq!(update.container, Some(None));
        assert_eq!(update.position, Some(current_container.position));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
    }

    #[test]
    fn acquire_attaches_to_grabbable_container() {
        let builder_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.position = Vector2::new(0, 0);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(6, 0);
        container.ocf = ocf::AVAILABLE | ocf::GRAB;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.construction = FULL_CON;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.container = Some(container_id);
        item.position = container.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(container.id, container.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        let update = result.update.expect("builder update");
        assert_eq!(update.container, Some(Some(container_id)));
        assert_eq!(update.position, Some(container.position));
    }

    #[test]
    fn command_stack_snapshot_preserves_acquire_state() {
        let builder_id = ObjectId::new(10);
        let item_id = ObjectId::new(11);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;
        builder.position = Vector2::new(0, 0);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.position = Vector2::new(50, 0);
        item.collectible = true;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
            )
            .expect("command enqueued");

        let ctx_initial = CommandRuntimeContext {
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let first_step = stack.step(&ctx_initial).expect("first step evaluates");
        assert_eq!(first_step.status, CommandStatus::Running);
        assert_eq!(stack.len(), 2, "move command should be queued");

        let snapshot = stack.snapshot();
        let encoded = serde_json::to_value(&snapshot).expect("serialize snapshot");
        let acquire_state = encoded["commands"]
            .as_array()
            .and_then(|commands| commands.iter().find_map(|entry| entry.get("Acquire")))
            .expect("acquire state present");
        let candidate = acquire_state["candidate"]
            .as_u64()
            .expect("candidate recorded");
        assert_eq!(candidate, item_id.as_u64());

        let ctx_followup = CommandRuntimeContext {
            frame: 25,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let original_second = stack.step(&ctx_followup).expect("second step evaluates");

        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&snapshot);
        let restored_second = restored
            .step(&ctx_followup)
            .expect("restored step evaluates");

        assert_eq!(original_second, restored_second);
    }

    #[test]
    fn buy_spawns_item_and_updates_player_state() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 42;
        builder.position = Vector2::new(10, 5);
        builder.command_direction = CommandDirection::Right;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 42;
        base.position = Vector2::new(20, 10);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());

        let mut home_base = HashMap::new();
        home_base.insert("WOOD".to_string(), 2);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: home_base,
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 25,
                can_chop: false,
                chop_action: None,
            },
        );

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = BuyState::from_request(
            &CommandRequest::new(CommandId::Buy).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.operations.len(), 0);
        assert!(result
            .update
            .as_ref()
            .and_then(|update| update.command_direction)
            .is_some());

        assert_eq!(result.events.len(), 3);
        match &result.events[0] {
            CommandEvent::AdjustPlayerHomeBaseMaterial {
                player_id,
                definition_id,
                delta,
            } => {
                assert_eq!(*player_id, 42);
                assert_eq!(definition_id, "WOOD");
                assert_eq!(*delta, -1);
            }
            event => panic!("unexpected event: {:?}", event),
        }

        match &result.events[1] {
            CommandEvent::AdjustPlayerWealth { player_id, delta } => {
                assert_eq!(*player_id, 42);
                assert_eq!(*delta, -25);
            }
            event => panic!("unexpected event: {:?}", event),
        }

        match &result.events[2] {
            CommandEvent::SpawnObject {
                definition_id,
                owner,
                position,
                container,
            } => {
                assert_eq!(definition_id, "WOOD");
                assert_eq!(*owner, 42);
                assert_eq!(*position, base.position);
                assert_eq!(*container, Some(base_id));
            }
            event => panic!("unexpected event: {:?}", event),
        }
    }

    #[test]
    fn buy_moves_toward_explicit_target() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 42;
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 0);
        target.category = CATEGORY_STRUCTURE;
        target.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        target.collectible = false;
        target.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(target_id);
        item.position = target.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 25,
                can_chop: false,
                chop_action: None,
            },
        );

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = BuyState::from_request(
            &CommandRequest::new(CommandId::Buy)
                .with_target(Some(target_id))
                .with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_some());
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
    fn buy_enters_explicit_target_when_adjacent() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 7;
        builder.position = Vector2::new(8, 0);
        builder.command_direction = CommandDirection::Left;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.category = CATEGORY_STRUCTURE;
        target.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        target.collectible = false;
        target.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(target_id);
        item.position = target.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            7,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 50,
                home_base_material: HashMap::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 5,
                can_chop: false,
                chop_action: None,
            },
        );

        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = BuyState::from_request(
            &CommandRequest::new(CommandId::Buy)
                .with_target(Some(target_id))
                .with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("builder update");
        assert_eq!(update.container, Some(Some(target_id)));
        assert_eq!(update.position, Some(Vector2::new(0, 0)));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
    }

    #[test]
    fn buy_collects_item_from_explicit_target() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 5;
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Stop;
        builder.container = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.category = CATEGORY_STRUCTURE;
        target.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        target.collectible = false;
        target.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(target_id);
        item.position = target.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            5,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 40,
                home_base_material: HashMap::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 15,
                can_chop: false,
                chop_action: None,
            },
        );

        let ctx = CommandRuntimeContext {
            frame: 10,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = BuyState::from_request(
            &CommandRequest::new(CommandId::Buy)
                .with_target(Some(target_id))
                .with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        if let Some(update) = result.update.as_ref() {
            assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        }

        assert_eq!(result.events.len(), 2);
        match &result.events[0] {
            CommandEvent::AdjustPlayerWealth { player_id, delta } => {
                assert_eq!(*player_id, 5);
                assert_eq!(*delta, -15);
            }
            other => panic!("unexpected event: {:?}", other),
        }

        match &result.events[1] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(update.container, Some(Some(builder_id)));
                assert_eq!(update.position, Some(builder.position));
                assert_eq!(update.velocity, Some(Vector2::ZERO));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn chop_sets_action_when_in_range() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(6, 0);
        builder.command_direction = CommandDirection::Right;
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition.clone(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);

        let update = result.update.expect("expected update");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
        let action_update = update.action.expect("action update");
        assert_eq!(action_update.name, Some("Chop".into()));
        assert_eq!(action_update.target, Some(Some(target_id)));
        assert!(action_update.force);
    }

    #[test]
    fn chop_requests_move_when_far() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(30, 0);
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(!result.operations.is_empty());
        match &result.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::MoveTo),
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn chop_requests_ungrab_when_pushing() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(30, 0);
        builder.action_procedure = ActionProcedure::Push;
        builder.action_target = Some(ObjectId::new(99));
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::UnGrab),
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn chop_completes_when_target_not_choppable() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(6, 0);
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: true,
                chop_action: Some("Chop".into()),
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
    }

    #[test]
    fn chop_fails_when_builder_cannot_chop() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(10, 0);
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            builder_definition,
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: Some("Chop".into()),
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
        };

        let mut state = ChopState::from_request(
            &CommandRequest::new(CommandId::Chop).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    SpawnObject {
        definition_id: DefinitionId,
        owner: i32,
        position: Vector2,
        container: Option<ObjectId>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    pub fn with_events(mut self, events: Vec<CommandEvent>) -> Self {
        self.events = events;
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
    pub players: &'a HashMap<i32, CommandPlayerSnapshot>,
    pub definitions: &'a HashMap<DefinitionId, CommandDefinitionSnapshot>,
    pub structures_need_energy: bool,
    pub base_buy_enabled: bool,
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

    pub fn player(&self, id: i32) -> Option<&CommandPlayerSnapshot> {
        self.players.get(&id)
    }

    pub fn definition(&self, id: &str) -> Option<&CommandDefinitionSnapshot> {
        self.definitions.get(id)
    }
}

#[derive(Debug)]
pub enum CommandError {
    StackFull,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CommandStackSnapshot {
    commands: Vec<CommandState>,
}

impl CommandStackSnapshot {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
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

    pub fn snapshot(&self) -> CommandStackSnapshot {
        CommandStackSnapshot {
            commands: self
                .entries
                .iter()
                .map(|entry| entry.state.clone())
                .collect(),
        }
    }

    pub fn restore_from_snapshot(&mut self, snapshot: &CommandStackSnapshot) {
        self.entries = snapshot
            .commands
            .iter()
            .cloned()
            .map(ActiveCommand::from_state)
            .collect();
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EnterState {
    target: ObjectId,
    #[allow(dead_code)]
    push_target: bool,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
}

impl EnterState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        let push_target = matches!(
            request.data,
            CommandData::Integer(flags) if flags & COMMAND_FLAG_ENTER_PUSH_TARGET != 0
        );
        Ok(Self {
            target,
            push_target,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
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

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let Some(target_snapshot) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        };

        if !target_snapshot.is_active() {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        if ctx.object.container == Some(self.target) {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        if target_snapshot.ocf & ocf::ENTRANCE == 0 {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        }

        if let Some(container) = ctx.object.container {
            if container != self.target && target_snapshot.container != Some(container) {
                return CommandStepResult::running(self.update_to_stop(ctx));
            }
        }

        const ENTRANCE_RANGE: i32 = 12;
        let dx = target_snapshot.position.x - ctx.position.x;
        let dy = target_snapshot.position.y - ctx.position.y;
        if dx.abs() <= ENTRANCE_RANGE && dy.abs() <= ENTRANCE_RANGE {
            let mut update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            update.container = Some(Some(self.target));
            update.position = Some(target_snapshot.position);
            update.velocity = Some(Vector2::ZERO);
            return CommandStepResult::completed(Some(update));
        }

        let mut result = CommandStepResult::running(self.update_to_stop(ctx));
        if self.should_issue_move(ctx.frame) {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(self.target))
                .with_update_interval(50);
            result = result.with_operations(vec![CommandOperation::PushFront(request)]);
        }
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ExitState {
    update_interval: u32,
    last_evaluated: Option<u64>,
}

impl ExitState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
        })
    }

    fn update_to_stop(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> ObjectUpdate {
        self.update_to_stop(ctx).unwrap_or_else(ObjectUpdate::new)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let Some(container_id) = ctx.object.container else {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        };

        let mut update = self.prepare_update(ctx);

        if ctx.object.action_procedure == ActionProcedure::Build {
            let action_update = ActionUpdate::default().with_name("Idle").with_force(true);
            update = update.with_action_update(action_update);
        }

        let container_snapshot = match ctx.resolve(container_id) {
            Some(snapshot) if snapshot.is_active() => snapshot,
            _ => {
                update.container = Some(None);
                update.position = Some(ctx.position);
                update.velocity = Some(Vector2::ZERO);
                return CommandStepResult::completed(Some(update));
            }
        };

        update.velocity = Some(Vector2::ZERO);

        if let Some(parent_id) = container_snapshot.container {
            update.container = Some(Some(parent_id));
            if let Some(parent_snapshot) = ctx.resolve(parent_id) {
                update.position = Some(parent_snapshot.position);
            } else {
                update.position = Some(container_snapshot.position);
            }
        } else {
            update.container = Some(None);
            update.position = Some(container_snapshot.position);
        }

        CommandStepResult::completed(Some(update))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ChopState {
    target: ObjectId,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    ungrab_requested: bool,
}

impl ChopState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            ungrab_requested: false,
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

    fn chop_action_name(&self, ctx: &CommandRuntimeContext<'_>) -> Option<String> {
        ctx.definition(&ctx.object.definition_id)
            .and_then(|definition| definition.chop_action.clone())
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        match ctx.definition(&ctx.object.definition_id) {
            Some(definition) if definition.can_chop => {}
            _ => {
                return CommandStepResult::failed(self.update_to_stop(ctx));
            }
        }

        let target_snapshot = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => {
                return CommandStepResult::failed(self.update_to_stop(ctx));
            }
        };

        if !target_snapshot.is_active() {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        }

        if target_snapshot.ocf & ocf::CHOP == 0 {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        if ctx.object.action_procedure == ActionProcedure::Chop
            && ctx.object.action_target == Some(self.target)
        {
            return CommandStepResult::running(self.update_to_stop(ctx));
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            if !self.ungrab_requested {
                self.ungrab_requested = true;
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                let mut result = CommandStepResult::running(self.update_to_stop(ctx));
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(self.update_to_stop(ctx));
        }
        self.ungrab_requested = false;

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Chop | ActionProcedure::Build | ActionProcedure::Dig
        ) {
            let mut update = self.update_to_stop(ctx).unwrap_or_else(ObjectUpdate::new);
            let idle_action = ActionUpdate::default().with_name("Idle").with_force(true);
            update = update.with_action_update(idle_action);
            return CommandStepResult::running(Some(update));
        }

        let dx = target_snapshot.position.x - ctx.position.x;
        let dy = target_snapshot.position.y - ctx.position.y;

        const MIN_HORIZONTAL_RANGE: i32 = 4;
        const MAX_HORIZONTAL_RANGE: i32 = 9;
        const MAX_VERTICAL_OFFSET: i32 = 12;

        let at_target = ctx.object.container.is_none()
            && target_snapshot.container.is_none()
            && dx.abs() >= MIN_HORIZONTAL_RANGE
            && dx.abs() <= MAX_HORIZONTAL_RANGE
            && dy.abs() <= MAX_VERTICAL_OFFSET;

        if at_target {
            let action_name = self
                .chop_action_name(ctx)
                .unwrap_or_else(|| "Chop".to_string());
            let action_update = ActionUpdate::default()
                .with_name(action_name)
                .with_target(Some(self.target))
                .with_phase(0)
                .with_ticks(0)
                .with_force(true);
            let update = ObjectUpdate::new()
                .with_action_update(action_update)
                .with_command_direction(CommandDirection::Stop)
                .with_velocity(Vector2::ZERO);
            return CommandStepResult::running(Some(update));
        }

        let mut result = CommandStepResult::running(self.update_to_stop(ctx));

        if self.should_issue_move(ctx.frame) {
            let approach_x = if ctx.position.x > target_snapshot.position.x {
                target_snapshot.position.x + 6
            } else {
                target_snapshot.position.x - 6
            };
            let mut operations = Vec::new();
            let approach_request = CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(approach_x))
                .with_ty(Some(target_snapshot.position.y))
                .with_update_interval(50);
            operations.push(CommandOperation::PushFront(approach_request));

            if dx.abs() < MIN_HORIZONTAL_RANGE {
                let move_away_x = if ctx.position.x > target_snapshot.position.x {
                    target_snapshot.position.x + 15
                } else {
                    target_snapshot.position.x - 15
                };
                let move_away_request = CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(move_away_x))
                    .with_ty(Some(target_snapshot.position.y))
                    .with_update_interval(50);
                operations.push(CommandOperation::PushFront(move_away_request));
            }

            result.operations.extend(operations);
        }

        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DigState {
    target: Vector2,
    update_interval: u32,
    last_evaluated: Option<u64>,
    dig_out_material: bool,
    ungrab_requested: bool,
    exit_requested: bool,
}

impl DigState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let tx = request.tx.ok_or(CommandError::Unsupported)?;
        let ty = request.ty.ok_or(CommandError::Unsupported)?;
        let dig_out_material = match &request.data {
            CommandData::Integer(value) => *value != 0,
            CommandData::Text(text) => !text.is_empty(),
            CommandData::None => false,
        };
        Ok(Self {
            target: Vector2::new(tx, ty),
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            dig_out_material,
            ungrab_requested: false,
            exit_requested: false,
        })
    }

    fn ensure_stop(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        update: Option<ObjectUpdate>,
    ) -> Option<ObjectUpdate> {
        if ctx.object.command_direction == CommandDirection::Stop {
            update
        } else {
            let update = update.unwrap_or_else(ObjectUpdate::new);
            Some(update.with_command_direction(CommandDirection::Stop))
        }
    }

    fn apply_idle(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        update: Option<ObjectUpdate>,
    ) -> Option<ObjectUpdate> {
        let update = self
            .ensure_stop(ctx, update)
            .unwrap_or_else(ObjectUpdate::new);
        Some(
            update.with_action_update(
                ActionUpdate::default()
                    .with_name("Idle")
                    .with_force(true)
                    .with_phase(0)
                    .with_ticks(0),
            ),
        )
    }

    fn move_to_range(&self, _ctx: &CommandRuntimeContext<'_>) -> i32 {
        DIG_MOVE_TO_RANGE_DEFAULT
    }

    fn desired_direction(&self, position: Vector2) -> CommandDirection {
        let mut direction = CommandDirection::Stop;

        if position.x < self.target.x - DIG_DIRECTION_RANGE {
            direction = CommandDirection::Right;
        } else if position.x > self.target.x + DIG_DIRECTION_RANGE {
            direction = CommandDirection::Left;
        }

        if position.y < self.target.y - DIG_DIRECTION_RANGE {
            direction = match direction {
                CommandDirection::Right => CommandDirection::DownRight,
                CommandDirection::Left => CommandDirection::DownLeft,
                _ => CommandDirection::Down,
            };
        } else if position.y > self.target.y + DIG_DIRECTION_RANGE {
            direction = match direction {
                CommandDirection::Right => CommandDirection::UpRight,
                CommandDirection::Left => CommandDirection::UpLeft,
                _ => CommandDirection::Up,
            };
        }

        direction
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let mut pending_update: Option<ObjectUpdate> = None;

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Build | ActionProcedure::Chop
        ) {
            pending_update = self.apply_idle(ctx, pending_update);
        }

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Hang | ActionProcedure::Scale
        ) {
            pending_update = self.apply_idle(ctx, pending_update);
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            pending_update = self.ensure_stop(ctx, pending_update);
            if !self.ungrab_requested {
                self.ungrab_requested = true;
                let mut result = CommandStepResult::running(pending_update.clone());
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(pending_update);
        }
        self.ungrab_requested = false;

        if ctx.object.container.is_some() {
            pending_update = self.ensure_stop(ctx, pending_update);
            if !self.exit_requested {
                self.exit_requested = true;
                let mut result = CommandStepResult::running(pending_update.clone());
                let request = CommandRequest::new(CommandId::Exit)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(pending_update);
        }
        self.exit_requested = false;

        let move_to_range = self.move_to_range(ctx);
        let dx = self.target.x - ctx.position.x;
        let dy = self.target.y - ctx.position.y;
        if dx.abs() <= move_to_range && dy.abs() <= move_to_range {
            let update = self.apply_idle(ctx, pending_update);
            return CommandStepResult::completed(update);
        }

        if ctx.object.action_procedure != ActionProcedure::Dig {
            if ctx.object.action_procedure != ActionProcedure::Walk {
                return CommandStepResult::running(pending_update);
            }
            let mut update = self
                .ensure_stop(ctx, pending_update)
                .unwrap_or_else(ObjectUpdate::new);
            let mut action_update = ActionUpdate::default()
                .with_name("Dig")
                .with_force(true)
                .with_phase(0)
                .with_ticks(0);
            if self.dig_out_material {
                action_update = action_update.with_data(1);
            }
            update = update.with_action_update(action_update);
            pending_update = Some(update);
        }

        let direction = self.desired_direction(ctx.position);
        if direction == CommandDirection::Stop {
            return CommandStepResult::running(pending_update);
        }

        if ctx.object.command_direction != direction {
            let mut update = pending_update.unwrap_or_else(ObjectUpdate::new);
            update = update.with_command_direction(direction);
            pending_update = Some(update);
        }

        CommandStepResult::running(pending_update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GrabState {
    target: ObjectId,
    offset_x: i32,
    offset_y: i32,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    ungrab_requested: bool,
}

impl GrabState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            offset_x: request.tx.unwrap_or(0),
            offset_y: request.ty.unwrap_or(0),
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            ungrab_requested: false,
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

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target == Some(self.target)
        {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        let target_snapshot = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::failed(Some(update));
            }
        };

        if !target_snapshot.is_active() {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            return CommandStepResult::failed(Some(update));
        }

        let mut pending_update = self.update_to_stop(ctx);

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Build
                | ActionProcedure::Chop
                | ActionProcedure::Dig
                | ActionProcedure::Hang
                | ActionProcedure::Scale
        ) {
            let idle_action = ActionUpdate::default().with_name("Idle").with_force(true);
            let update = pending_update.take().unwrap_or_else(ObjectUpdate::new);
            pending_update = Some(update.with_action_update(idle_action));
        }

        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target != Some(self.target)
        {
            if !self.ungrab_requested {
                self.ungrab_requested = true;
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                let mut result = CommandStepResult::running(pending_update);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(pending_update);
        }

        let approach_position = Vector2::new(
            target_snapshot.position.x + self.offset_x,
            target_snapshot.position.y + self.offset_y,
        );

        let dx = target_snapshot.position.x - ctx.position.x;
        let dy = target_snapshot.position.y - ctx.position.y;
        const GRAB_RANGE: i32 = 8;
        let can_grab_here = ctx.object.container.is_none()
            && dx.abs() <= GRAB_RANGE
            && dy.abs() <= GRAB_RANGE
            && (target_snapshot.ocf & ocf::GRAB) != 0;

        if can_grab_here {
            let mut update = pending_update.unwrap_or_else(ObjectUpdate::new);
            let action_update = ActionUpdate::default()
                .with_name("Push")
                .with_target(Some(self.target))
                .with_force(true)
                .with_phase(0)
                .with_ticks(0);
            update = update.with_action_update(action_update);
            return CommandStepResult::running(Some(update));
        }

        if self.should_issue_move(ctx.frame) {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(approach_position.x))
                .with_ty(Some(approach_position.y))
                .with_update_interval(50);
            let mut result = CommandStepResult::running(pending_update);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        CommandStepResult::running(pending_update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct UnGrabState {
    update_interval: u32,
    last_evaluated: Option<u64>,
}

impl UnGrabState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
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

        let mut needs_update = false;
        let mut update = ObjectUpdate::new();

        if ctx.object.command_direction != CommandDirection::Stop {
            update = update.with_command_direction(CommandDirection::Stop);
            needs_update = true;
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            let action_update = ActionUpdate::default()
                .with_name("Idle")
                .with_target(None)
                .with_force(true);
            update = update.with_action_update(action_update);
            needs_update = true;
        }

        if needs_update {
            CommandStepResult::completed(Some(update))
        } else {
            CommandStepResult::completed(None)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct JumpState {
    tx: Option<i32>,
    evaluated: bool,
}

impl JumpState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            tx: request.tx,
            evaluated: false,
        }
    }

    fn desired_direction(&self, ctx: &CommandRuntimeContext<'_>) -> Option<Direction> {
        let target_x = self.tx?;
        if target_x < ctx.position.x {
            Some(Direction::Left)
        } else if target_x > ctx.position.x {
            Some(Direction::Right)
        } else {
            None
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.evaluated {
            return CommandStepResult::completed(None);
        }
        self.evaluated = true;

        let mut update: Option<ObjectUpdate> = None;

        if let Some(direction) = self.desired_direction(ctx) {
            let mut object_update = update.unwrap_or_else(ObjectUpdate::new);
            object_update.direction = Some(direction);
            update = Some(object_update);
        }

        if ctx.object.action_procedure == ActionProcedure::Walk {
            let mut object_update = update.unwrap_or_else(ObjectUpdate::new);
            let action_update = ActionUpdate::default()
                .with_name("Jump")
                .with_phase(0)
                .with_ticks(0)
                .with_force(true);
            object_update = object_update.with_action_update(action_update);
            update = Some(object_update);
        }

        CommandStepResult::completed(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WaitState {
    remaining: Option<u32>,
}

impl WaitState {
    fn from_request(request: &CommandRequest) -> Self {
        let remaining = if request.update_interval == 0 {
            None
        } else {
            Some(request.update_interval)
        };
        Self { remaining }
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.action_procedure != ActionProcedure::Dig {
            return None;
        }
        let mut update = ObjectUpdate::new();
        if ctx.object.command_direction != CommandDirection::Stop {
            update = update.with_command_direction(CommandDirection::Stop);
        }
        let action_update = ActionUpdate::default().with_name("Idle").with_force(true);
        Some(update.with_action_update(action_update))
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let update = self.prepare_update(ctx);

        if let Some(remaining) = self.remaining.as_mut() {
            if *remaining == 0 {
                return CommandStepResult::completed(update);
            }
            *remaining -= 1;
            if *remaining == 0 {
                return CommandStepResult::completed(update);
            }
        }

        CommandStepResult::running(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PutState {
    container: ObjectId,
    requested_item: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    get_requested: bool,
    exit_requested: bool,
    ungrab_requested: bool,
}

impl PutState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let container = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            container,
            requested_item: request.target2,
            definition_id: command_data_to_definition_id(&request.data),
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            get_requested: false,
            exit_requested: false,
            ungrab_requested: false,
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

    fn resolve_item<'a>(
        &mut self,
        ctx: &'a CommandRuntimeContext<'a>,
    ) -> Option<(ObjectId, &'a CommandObjectSnapshot)> {
        if let Some(item_id) = self.requested_item {
            if let Some(snapshot) = ctx.resolve(item_id) {
                return Some((item_id, snapshot));
            }
            self.requested_item = None;
        }

        if let Some(definition_id) = &self.definition_id {
            for object_id in &ctx.object.contents {
                if let Some(snapshot) = ctx.resolve(*object_id) {
                    if &snapshot.definition_id == definition_id {
                        self.requested_item = Some(*object_id);
                        return Some((*object_id, snapshot));
                    }
                }
            }
        }

        if let Some(object_id) = ctx.object.contents.first().copied() {
            if let Some(snapshot) = ctx.resolve(object_id) {
                self.requested_item = Some(object_id);
                return Some((object_id, snapshot));
            }
        }

        None
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        if ctx.object.container.is_none() {
            self.exit_requested = false;
        }
        if ctx.object.action_procedure != ActionProcedure::Push {
            self.ungrab_requested = false;
        }

        let update = self.update_to_stop(ctx);

        let container_snapshot = match ctx.resolve(self.container) {
            Some(snapshot) if snapshot.is_active() => snapshot,
            _ => return CommandStepResult::failed(update),
        };

        let (item_id, item_snapshot) = match self.resolve_item(ctx) {
            Some(value) => value,
            None => return CommandStepResult::completed(update),
        };

        if item_snapshot.container == Some(self.container) {
            return CommandStepResult::completed(update);
        }

        if item_snapshot.destroyed {
            return CommandStepResult::failed(update);
        }

        if item_snapshot.container != Some(ctx.object.id) {
            if !self.get_requested {
                self.get_requested = true;
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Get)
                    .with_target(Some(item_id))
                    .with_update_interval(40)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update);
        }
        self.get_requested = false;

        if let Some(container_id) = ctx.object.container {
            if container_id != self.container {
                if !self.exit_requested {
                    self.exit_requested = true;
                    let mut result = CommandStepResult::running(update.clone());
                    let request = CommandRequest::new(CommandId::Exit)
                        .with_update_interval(50)
                        .with_mode(CommandMode::Sub);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
                return CommandStepResult::running(update);
            }
        }

        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target != Some(self.container)
        {
            if !self.ungrab_requested {
                self.ungrab_requested = true;
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update);
        } else if ctx.object.action_procedure != ActionProcedure::Push {
            self.ungrab_requested = false;
        }

        const PUT_RANGE_HORIZONTAL: i32 = 12;
        const PUT_RANGE_VERTICAL: i32 = 18;
        let dx = container_snapshot.position.x - ctx.position.x;
        let dy = container_snapshot.position.y - ctx.position.y;
        if dx.abs() > PUT_RANGE_HORIZONTAL || dy.abs() > PUT_RANGE_VERTICAL {
            if self.should_issue_move(ctx.frame) {
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::MoveTo)
                    .with_target(Some(self.container))
                    .with_update_interval(15)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update);
        }

        let mut item_update = ObjectUpdate::new();
        item_update.container = Some(Some(self.container));
        item_update.position = Some(container_snapshot.position);
        item_update.velocity = Some(Vector2::ZERO);

        CommandStepResult::completed(update).with_events(vec![CommandEvent::ApplyObjectUpdate {
            object_id: item_id,
            update: item_update,
        }])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DropState {
    requested_item: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    target_position: Option<Vector2>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    get_requested: bool,
    ungrab_requested: bool,
    delegated_put: bool,
    delegated_container: Option<ObjectId>,
}

impl DropState {
    fn from_request(request: &CommandRequest) -> Self {
        let has_coordinates = request.tx.is_some() || request.ty.is_some();
        let target_position = if has_coordinates {
            Some(Vector2::new(request.tx.unwrap_or(0), request.ty.unwrap_or(0)))
        } else {
            None
        };
        Self {
            requested_item: request.target,
            definition_id: command_data_to_definition_id(&request.data),
            target_position,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            get_requested: false,
            ungrab_requested: false,
            delegated_put: false,
            delegated_container: None,
        }
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

    fn resolve_item<'a>(
        &mut self,
        ctx: &'a CommandRuntimeContext<'a>,
    ) -> Option<(ObjectId, &'a CommandObjectSnapshot)> {
        if let Some(item_id) = self.requested_item {
            if let Some(snapshot) = ctx.resolve(item_id) {
                return Some((item_id, snapshot));
            }
            self.requested_item = None;
        }

        if let Some(definition_id) = &self.definition_id {
            for object_id in &ctx.object.contents {
                if let Some(snapshot) = ctx.resolve(*object_id) {
                    if &snapshot.definition_id == definition_id {
                        self.requested_item = Some(*object_id);
                        return Some((*object_id, snapshot));
                    }
                }
            }
        }

        if let Some(object_id) = ctx.object.contents.first().copied() {
            if let Some(snapshot) = ctx.resolve(object_id) {
                self.requested_item = Some(object_id);
                return Some((object_id, snapshot));
            }
        }

        None
    }

    fn delegate_put(
        &mut self,
        item_id: ObjectId,
        container_id: ObjectId,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        if self.delegated_put && self.delegated_container == Some(container_id) {
            return CommandStepResult::running(update);
        }
        self.delegated_put = true;
        self.delegated_container = Some(container_id);

        let mut request = CommandRequest::new(CommandId::Put)
            .with_target(Some(container_id))
            .with_target2(Some(item_id))
            .with_mode(CommandMode::Sub)
            .with_update_interval(15);

        if let Some(position) = self.target_position {
            request = request.with_tx(Some(position.x)).with_ty(Some(position.y));
        }

        let mut result = CommandStepResult::running(update);
        result.operations.push(CommandOperation::PushFront(request));
        result
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        if ctx.object.action_procedure != ActionProcedure::Push {
            self.ungrab_requested = false;
        }

        let update = self.update_to_stop(ctx);

        let (item_id, item_snapshot) = match self.resolve_item(ctx) {
            Some(value) => value,
            None => return CommandStepResult::completed(update),
        };

        if item_snapshot.destroyed {
            return CommandStepResult::failed(update);
        }

        if item_snapshot.container != Some(ctx.object.id) {
            if item_snapshot.container.is_none() {
                return CommandStepResult::completed(update);
            }
            if !self.get_requested {
                self.get_requested = true;
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Get)
                    .with_target(Some(item_id))
                    .with_update_interval(40)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update);
        }
        self.get_requested = false;

        if let Some(container_id) = ctx.object.container {
            return self.delegate_put(item_id, container_id, update);
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            if let Some(container_id) = ctx.object.action_target {
                return self.delegate_put(item_id, container_id, update);
            }
        }

        if let Some(position) = self.target_position {
            const DROP_RANGE_HORIZONTAL: i32 = 12;
            const DROP_RANGE_VERTICAL: i32 = 15;
            let dx = position.x - ctx.position.x;
            let dy = position.y - ctx.position.y;

            if dx.abs() > DROP_RANGE_HORIZONTAL || dy.abs() > DROP_RANGE_VERTICAL {
                if ctx.object.action_procedure == ActionProcedure::Push && !self.ungrab_requested {
                    self.ungrab_requested = true;
                    let mut result = CommandStepResult::running(update.clone());
                    let request = CommandRequest::new(CommandId::UnGrab)
                        .with_update_interval(50)
                        .with_mode(CommandMode::Sub);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }

                if self.should_issue_move(ctx.frame) {
                    let mut result = CommandStepResult::running(update.clone());
                    let request = CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(position.x))
                        .with_ty(Some(position.y))
                        .with_update_interval(20)
                        .with_mode(CommandMode::Sub);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
                return CommandStepResult::running(update);
            }
        } else if ctx.object.action_procedure == ActionProcedure::Push && !self.ungrab_requested {
            self.ungrab_requested = true;
            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::UnGrab)
                .with_update_interval(50)
                .with_mode(CommandMode::Sub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        let drop_position = self.target_position.unwrap_or(ctx.position);
        let mut item_update = ObjectUpdate::new();
        item_update.container = Some(None);
        item_update.position = Some(drop_position);
        item_update.velocity = Some(Vector2::ZERO);

        CommandStepResult::completed(update).with_events(vec![CommandEvent::ApplyObjectUpdate {
            object_id: item_id,
            update: item_update,
        }])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GetState {
    target: Option<ObjectId>,
    fallback_container: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    remaining: i32,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    exit_requested: bool,
    ungrab_requested: bool,
    grab_requested: bool,
    enter_requested: bool,
    dig_requested: bool,
}

impl GetState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let definition_id = command_data_to_definition_id(&request.data);
        if request.target.is_none() && definition_id.is_none() {
            return Err(CommandError::Unsupported);
        }
        let mut remaining = request.tx.unwrap_or(1);
        if remaining <= 0 {
            remaining = 1;
        }
        Ok(Self {
            target: request.target,
            fallback_container: request.target2,
            definition_id,
            remaining,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            exit_requested: false,
            ungrab_requested: false,
            grab_requested: false,
            enter_requested: false,
            dig_requested: false,
        })
    }

    fn ensure_stop(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        update: Option<ObjectUpdate>,
    ) -> Option<ObjectUpdate> {
        if ctx.object.command_direction == CommandDirection::Stop {
            return update;
        }
        let mut update = update.unwrap_or_else(ObjectUpdate::new);
        update.command_direction = Some(CommandDirection::Stop);
        Some(update)
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        let mut update = self.ensure_stop(ctx, None);
        if ctx.object.action_procedure == ActionProcedure::Dig {
            let mut object_update = update.unwrap_or_else(ObjectUpdate::new);
            let action_update = ActionUpdate::default()
                .with_name("Idle")
                .with_force(true)
                .with_phase(0)
                .with_ticks(0);
            object_update = object_update.with_action_update(action_update);
            update = Some(object_update);
        }
        update
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

    fn resolve_target(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        if let Some(target_id) = self.target {
            if ctx.resolve(target_id).is_some() {
                return Some(target_id);
            }
            self.target = None;
        }

        let (Some(container_id), Some(definition_id)) =
            (self.fallback_container, self.definition_id.clone())
        else {
            return None;
        };

        let container_snapshot = ctx.resolve(container_id)?;
        for item_id in &container_snapshot.contents {
            if let Some(item_snapshot) = ctx.resolve(*item_id) {
                if item_snapshot.is_active()
                    && item_snapshot.definition_id == definition_id
                    && item_snapshot.collectible
                    && item_snapshot.construction >= FULL_CON
                {
                    self.target = Some(*item_id);
                    return self.target;
                }
            }
        }

        None
    }

    fn transfer_to_actor(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_id: ObjectId,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        let update = self.ensure_stop(ctx, update);
        let mut transfer_update = ObjectUpdate::new();
        transfer_update.container = Some(Some(ctx.object.id));
        transfer_update.position = Some(ctx.position);
        transfer_update.velocity = Some(Vector2::ZERO);

        let events = vec![CommandEvent::ApplyObjectUpdate {
            object_id: target_id,
            update: transfer_update,
        }];

        if self.remaining > 1 {
            self.remaining -= 1;
            self.target = None;
            self.grab_requested = false;
            self.enter_requested = false;
            return CommandStepResult::running(update).with_events(events);
        }

        CommandStepResult::completed(update).with_events(events)
    }

    fn handle_container_target(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_id: ObjectId,
        target_snapshot: &CommandObjectSnapshot,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        let Some(container_id) = target_snapshot.container else {
            return CommandStepResult::failed(self.ensure_stop(ctx, update));
        };

        if ctx.object.container == Some(container_id) {
            self.enter_requested = false;
            self.grab_requested = false;
            return self.transfer_to_actor(ctx, target_id, update);
        }

        if let Some(current_container) = ctx.object.container {
            if current_container != container_id {
                if !self.exit_requested {
                    self.exit_requested = true;
                    let mut result =
                        CommandStepResult::running(self.ensure_stop(ctx, update.clone()));
                    let request = CommandRequest::new(CommandId::Exit)
                        .with_update_interval(50)
                        .with_mode(CommandMode::Sub);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
                return CommandStepResult::running(self.ensure_stop(ctx, update));
            }
        } else {
            self.exit_requested = false;
        }

        let Some(container_snapshot) = ctx.resolve(container_id) else {
            return CommandStepResult::failed(self.ensure_stop(ctx, update));
        };
        if !container_snapshot.is_active() {
            return CommandStepResult::failed(self.ensure_stop(ctx, update));
        }

        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target == Some(container_id)
        {
            self.grab_requested = false;
            return self.transfer_to_actor(ctx, target_id, update);
        }

        if container_snapshot.ocf & ocf::GRAB != 0 {
            if !self.grab_requested {
                self.grab_requested = true;
                let mut result = CommandStepResult::running(self.ensure_stop(ctx, update.clone()));
                let request = CommandRequest::new(CommandId::Grab)
                    .with_target(Some(container_id))
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(self.ensure_stop(ctx, update));
        }

        if container_snapshot.ocf & ocf::ENTRANCE != 0 {
            if !self.enter_requested {
                self.enter_requested = true;
                let mut result = CommandStepResult::running(self.ensure_stop(ctx, update.clone()));
                let request = CommandRequest::new(CommandId::Enter)
                    .with_target(Some(container_id))
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(self.ensure_stop(ctx, update));
        }

        CommandStepResult::failed(self.ensure_stop(ctx, update))
    }

    fn handle_in_solid_target(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_id: ObjectId,
        target_snapshot: &CommandObjectSnapshot,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        if ctx.object.container.is_some() {
            if !self.exit_requested {
                self.exit_requested = true;
                let mut result = CommandStepResult::running(self.ensure_stop(ctx, update.clone()));
                let request = CommandRequest::new(CommandId::Exit)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(self.ensure_stop(ctx, update));
        }

        let dx = target_snapshot.position.x - ctx.position.x;
        let dy = target_snapshot.position.y - ctx.position.y;
        if dx.abs() > DIG_MOVE_TO_RANGE_DEFAULT || dy.abs() > DIG_MOVE_TO_RANGE_DEFAULT {
            if self.should_issue_move(ctx.frame) {
                let mut result = CommandStepResult::running(self.ensure_stop(ctx, update.clone()));
                let request = CommandRequest::new(CommandId::MoveTo)
                    .with_target(Some(target_id))
                    .with_update_interval(10);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(self.ensure_stop(ctx, update));
        }

        if !self.dig_requested {
            self.dig_requested = true;
            let mut result = CommandStepResult::running(self.ensure_stop(ctx, update.clone()));
            let request = CommandRequest::new(CommandId::Dig)
                .with_tx(Some(target_snapshot.position.x))
                .with_ty(Some(target_snapshot.position.y + 4))
                .with_update_interval(50)
                .with_mode(CommandMode::Sub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        CommandStepResult::running(self.ensure_stop(ctx, update))
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        if ctx.object.container.is_none() {
            self.exit_requested = false;
        }
        if ctx.object.action_procedure != ActionProcedure::Push {
            self.ungrab_requested = false;
        }

        let update = self.prepare_update(ctx);

        let target_id = match self.resolve_target(ctx) {
            Some(id) => id,
            None => return CommandStepResult::failed(update),
        };

        let target_snapshot = match ctx.resolve(target_id) {
            Some(snapshot) if snapshot.is_active() => snapshot,
            _ => return CommandStepResult::failed(update),
        };

        if !target_snapshot.collectible || target_snapshot.construction < FULL_CON {
            return CommandStepResult::failed(update);
        }

        if target_snapshot.id == ctx.object.id {
            return CommandStepResult::failed(update);
        }

        if target_snapshot.container == Some(ctx.object.id) {
            if self.remaining > 1 {
                self.remaining -= 1;
                self.target = None;
                return CommandStepResult::running(update);
            }
            return CommandStepResult::completed(update);
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            if let Some(container_id) = target_snapshot.container {
                if ctx.object.action_target != Some(container_id) && !self.ungrab_requested {
                    self.ungrab_requested = true;
                    let mut result = CommandStepResult::running(update.clone());
                    let request = CommandRequest::new(CommandId::UnGrab)
                        .with_update_interval(50)
                        .with_mode(CommandMode::Sub);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
            } else if !self.ungrab_requested {
                self.ungrab_requested = true;
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
        } else {
            self.ungrab_requested = false;
        }

        if target_snapshot.container.is_some() {
            return self.handle_container_target(ctx, target_id, target_snapshot, update);
        }

        self.grab_requested = false;
        self.enter_requested = false;

        if target_snapshot.ocf & ocf::IN_SOLID != 0 {
            return self.handle_in_solid_target(ctx, target_id, target_snapshot, update);
        }
        self.dig_requested = false;

        if ctx.object.container.is_some() {
            if !self.exit_requested {
                self.exit_requested = true;
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Exit)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update);
        }

        let dx = target_snapshot.position.x - ctx.position.x;
        let dy = target_snapshot.position.y - ctx.position.y;
        const PICKUP_RANGE: i32 = 12;
        if dx.abs() <= PICKUP_RANGE && dy.abs() <= PICKUP_RANGE {
            return self.transfer_to_actor(ctx, target_id, update);
        }

        if self.should_issue_move(ctx.frame) {
            let mut result = CommandStepResult::running(update);
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(target_id))
                .with_update_interval(10);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        CommandStepResult::running(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RetryState {
    remaining: u32,
}

impl RetryState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            remaining: request.update_interval.max(1),
        }
    }

    fn step(&mut self, _ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.remaining == 0 {
            return CommandStepResult::completed(None);
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            CommandStepResult::completed(None)
        } else {
            CommandStepResult::running(None)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ThrowState {
    target: Option<ObjectId>,
    tx: Option<i32>,
    ty: Option<i32>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    acquire_requested: bool,
    ungrab_requested: bool,
}

impl ThrowState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            target: request.target,
            tx: request.tx,
            ty: request.ty,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            acquire_requested: false,
            ungrab_requested: false,
        })
    }

    fn update_to_stop(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
    }

    fn throw_position(&self) -> Option<Vector2> {
        match (self.tx, self.ty) {
            (Some(x), Some(y)) if x != 0 || y != 0 => Some(Vector2::new(x, y)),
            _ => None,
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

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let mut pending_update = self.update_to_stop(ctx);

        if ctx.object.action_procedure == ActionProcedure::Dig {
            let idle_action = ActionUpdate::default().with_name("Idle").with_force(true);
            let update = pending_update.take().unwrap_or_else(ObjectUpdate::new);
            pending_update = Some(update.with_action_update(idle_action));
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            if !self.ungrab_requested {
                self.ungrab_requested = true;
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                let mut result = CommandStepResult::running(pending_update);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(pending_update);
        }
        self.ungrab_requested = false;

        if let Some(target_id) = self.target {
            let mut has_item = false;
            for id in &ctx.object.contents {
                if *id == target_id {
                    has_item = true;
                    break;
                }
            }
            if !has_item {
                let target_snapshot = match ctx.resolve(target_id) {
                    Some(snapshot) => snapshot,
                    None => return CommandStepResult::failed(self.update_to_stop(ctx)),
                };
                if !target_snapshot.is_active() {
                    return CommandStepResult::failed(self.update_to_stop(ctx));
                }
                if !self.acquire_requested {
                    let acquire_request = CommandRequest::new(CommandId::Acquire)
                        .with_data(CommandData::Text(target_snapshot.definition_id.clone()))
                        .with_update_interval(ACQUIRE_REQUEST_INTERVAL)
                        .with_mode(CommandMode::Sub)
                        .with_tx(Some(500))
                        .with_ty(Some(250));
                    self.acquire_requested = true;
                    let mut result = CommandStepResult::running(pending_update);
                    result
                        .operations
                        .push(CommandOperation::PushFront(acquire_request));
                    return result;
                }
                return CommandStepResult::running(pending_update);
            }
        }
        self.acquire_requested = false;

        if let Some(position) = self.throw_position() {
            const THROW_HORIZONTAL_RANGE: i32 = 15;
            const THROW_VERTICAL_RANGE: i32 = 15;
            let dx = position.x - ctx.position.x;
            let dy = position.y - ctx.position.y;
            if dx.abs() > THROW_HORIZONTAL_RANGE || dy.abs() > THROW_VERTICAL_RANGE {
                if self.should_issue_move(ctx.frame) {
                    let request = CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(position.x))
                        .with_ty(Some(position.y))
                        .with_update_interval(20);
                    let mut result = CommandStepResult::running(pending_update);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
                return CommandStepResult::running(pending_update);
            }
        }

        let mut update = pending_update.unwrap_or_else(ObjectUpdate::new);
        update.command_direction = Some(CommandDirection::Stop);

        if let Some(position) = self.throw_position() {
            if position.x > ctx.position.x {
                update.direction = Some(Direction::Right);
            } else if position.x < ctx.position.x {
                update.direction = Some(Direction::Left);
            }
        } else if let Some(target_id) = self.target {
            if let Some(snapshot) = ctx.resolve(target_id) {
                if snapshot.position.x > ctx.position.x {
                    update.direction = Some(Direction::Right);
                } else if snapshot.position.x < ctx.position.x {
                    update.direction = Some(Direction::Left);
                }
            }
        }

        let mut action_update = ActionUpdate::default()
            .with_name("Throw")
            .with_force(true)
            .with_phase(0)
            .with_ticks(0);

        if let Some(target_id) = self.target {
            action_update = action_update.with_target(Some(target_id));
        }

        update = update.with_action_update(action_update);
        CommandStepResult::completed(Some(update))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AcquireState {
    definition_id: DefinitionId,
    ignore_container: Option<ObjectId>,
    range_x: i32,
    range_y: i32,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    candidate: Option<ObjectId>,
    buy_requested: bool,
    last_buy_request: Option<u64>,
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
            buy_requested: false,
            last_buy_request: None,
        })
    }

    fn update_to_stop(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
    }

    fn maybe_reset_buy(&mut self, frame: u64) {
        const BUY_RETRY_INTERVAL: u64 = 100;
        if !self.buy_requested {
            self.last_buy_request = None;
            return;
        }
        if let Some(last) = self.last_buy_request {
            if frame.saturating_sub(last) >= BUY_RETRY_INTERVAL {
                self.buy_requested = false;
                self.last_buy_request = None;
            }
        }
    }

    fn request_buy(&mut self, frame: u64) -> Option<CommandOperation> {
        if self.buy_requested {
            return None;
        }
        let Some(c4id) = definition_id_to_c4id(&self.definition_id) else {
            return None;
        };
        self.buy_requested = true;
        self.last_buy_request = Some(frame);
        let request = CommandRequest::new(CommandId::Buy)
            .with_data(CommandData::Integer(c4id))
            .with_update_interval(100)
            .with_mode(CommandMode::Sub);
        Some(CommandOperation::PushFront(request))
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

        let can_enter = (container_snapshot.ocf & ocf::ENTRANCE) != 0;
        let can_grab = (container_snapshot.ocf & ocf::GRAB) != 0;

        if builder_container.is_none() {
            let dx = container_snapshot.position.x - ctx.position.x;
            let dy = container_snapshot.position.y - ctx.position.y;
            const CONTAINER_APPROACH_RANGE: i32 = 12;
            if dx.abs() <= CONTAINER_APPROACH_RANGE && dy.abs() <= CONTAINER_APPROACH_RANGE {
                let mut update = base_update.clone().unwrap_or_default();
                if can_enter || can_grab {
                    update.container = Some(Some(container_id));
                }
                update.position = Some(container_snapshot.position);
                update.velocity = Some(Vector2::ZERO);
                if update.command_direction.is_none() {
                    update.command_direction = Some(CommandDirection::Stop);
                }
                return CommandStepResult::running(Some(update));
            }
        }

        if !can_enter && !can_grab {
            return CommandStepResult::running(base_update);
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

        if self.candidate.is_none() {
            self.maybe_reset_buy(ctx.frame);
            let mut result = CommandStepResult::running(self.update_to_stop(ctx));
            if let Some(operation) = self.request_buy(ctx.frame) {
                result.operations.push(operation);
            }
            return result;
        }

        let candidate_id = self.candidate.expect("candidate present");
        let Some(candidate) = ctx.resolve(candidate_id) else {
            self.candidate = None;
            return CommandStepResult::running(self.update_to_stop(ctx));
        };

        self.buy_requested = false;
        self.last_buy_request = None;

        if let Some(container_id) = candidate.container {
            if let Some(builder_container) = ctx.object.container {
                if builder_container != container_id {
                    let mut update = self.update_to_stop(ctx).unwrap_or_else(ObjectUpdate::new);
                    if let Some(snapshot) = ctx.resolve(builder_container) {
                        update.position = Some(snapshot.position);
                    } else {
                        update.position = Some(ctx.position);
                    }
                    update.velocity = Some(Vector2::ZERO);
                    update.container = Some(None);
                    return CommandStepResult::running(Some(update));
                }
            }
        }

        if candidate.container.is_none() {
            if let Some(builder_container) = ctx.object.container {
                let mut update = self.update_to_stop(ctx).unwrap_or_else(ObjectUpdate::new);
                if let Some(snapshot) = ctx.resolve(builder_container) {
                    update.position = Some(snapshot.position);
                } else {
                    update.position = Some(ctx.position);
                }
                update.velocity = Some(Vector2::ZERO);
                update.container = Some(None);
                return CommandStepResult::running(Some(update));
            }
        }

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BuyState {
    definition_id: DefinitionId,
    target: Option<ObjectId>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
}

impl BuyState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let definition_id =
            command_data_to_definition_id(&request.data).ok_or(CommandError::Unsupported)?;
        Ok(Self {
            definition_id,
            target: request.target,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
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

    fn try_purchase_from_explicit_target(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_id: ObjectId,
        target_snapshot: &CommandObjectSnapshot,
    ) -> Option<CommandStepResult> {
        if !target_snapshot.is_active() || target_snapshot.collectible {
            return None;
        }

        let update_to_stop = self.update_to_stop(ctx);

        if let Some(container_id) = ctx.object.container {
            if container_id != target_id {
                let mut update = update_to_stop.unwrap_or_else(ObjectUpdate::new);
                if let Some(snapshot) = ctx.resolve(container_id) {
                    update.position = Some(snapshot.position);
                } else {
                    update.position = Some(ctx.position);
                }
                update.velocity = Some(Vector2::ZERO);
                update.container = Some(None);
                return Some(CommandStepResult::running(Some(update)));
            }
        }

        let needs_container_change = ctx.object.container != Some(target_id);
        if needs_container_change {
            const APPROACH_RANGE: i32 = 12;
            let dx = target_snapshot.position.x - ctx.position.x;
            let dy = target_snapshot.position.y - ctx.position.y;
            if dx.abs() <= APPROACH_RANGE && dy.abs() <= APPROACH_RANGE {
                if (target_snapshot.ocf & (ocf::ENTRANCE | ocf::GRAB)) == 0 {
                    return None;
                }
                let mut update = update_to_stop.unwrap_or_else(ObjectUpdate::new);
                update.position = Some(target_snapshot.position);
                update.velocity = Some(Vector2::ZERO);
                update.container = Some(Some(target_id));
                return Some(CommandStepResult::running(Some(update)));
            }

            if self.should_issue_move(ctx.frame) {
                let request = CommandRequest::new(CommandId::MoveTo)
                    .with_target(Some(target_id))
                    .with_update_interval(10);
                let mut result = CommandStepResult::running(update_to_stop);
                result.operations.push(CommandOperation::PushFront(request));
                return Some(result);
            }

            return Some(CommandStepResult::running(update_to_stop));
        }

        let mut candidate = None;
        for item_id in &target_snapshot.contents {
            if let Some(item_snapshot) = ctx.resolve(*item_id) {
                if item_snapshot.is_active()
                    && item_snapshot.definition_id == self.definition_id
                    && item_snapshot.collectible
                    && item_snapshot.construction >= FULL_CON
                {
                    candidate = Some(*item_id);
                    break;
                }
            }
        }

        let item_id = match candidate {
            Some(id) => id,
            None => return None,
        };

        let buyer_owner = ctx.object.owner;
        let player = match ctx.player(buyer_owner) {
            Some(player) if player.is_active() => player,
            _ => {
                return Some(CommandStepResult::failed(update_to_stop));
            }
        };

        let price = ctx
            .definition(&self.definition_id)
            .map(|definition| definition.value.max(0))
            .unwrap_or(0);

        if price > player.wealth {
            return Some(CommandStepResult::failed(update_to_stop));
        }

        let mut events = Vec::new();
        if price != 0 {
            events.push(CommandEvent::AdjustPlayerWealth {
                player_id: buyer_owner,
                delta: -price,
            });
        }

        let mut transfer_update = ObjectUpdate::new();
        transfer_update.container = Some(Some(ctx.object.id));
        transfer_update.position = Some(ctx.position);
        transfer_update.velocity = Some(Vector2::ZERO);
        events.push(CommandEvent::ApplyObjectUpdate {
            object_id: item_id,
            update: transfer_update,
        });

        Some(CommandStepResult::completed(update_to_stop).with_events(events))
    }

    fn resolve_base(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        if let Some(target) = self.target {
            if let Some(snapshot) = ctx.resolve(target) {
                if snapshot.is_active() {
                    return Some(target);
                }
            }
        }

        let buyer_owner = ctx.object.owner;
        ctx.objects
            .values()
            .filter(|snapshot| {
                snapshot.is_active()
                    && snapshot.owner == buyer_owner
                    && (snapshot.category & CATEGORY_STRUCTURE) != 0
                    && (snapshot.ocf & ocf::ENTRANCE) != 0
                    && !snapshot.collectible
            })
            .min_by_key(|snapshot| {
                let dx = i64::from(snapshot.position.x - ctx.position.x);
                let dy = i64::from(snapshot.position.y - ctx.position.y);
                dx * dx + dy * dy
            })
            .map(|snapshot| snapshot.id)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < self.update_interval as u64 {
                if let Some(target_id) = self.target {
                    if let Some(target_snapshot) = ctx.resolve(target_id) {
                        if let Some(result) =
                            self.try_purchase_from_explicit_target(ctx, target_id, target_snapshot)
                        {
                            return result;
                        }
                    }
                }
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        if let Some(target_id) = self.target {
            if let Some(target_snapshot) = ctx.resolve(target_id) {
                if let Some(result) =
                    self.try_purchase_from_explicit_target(ctx, target_id, target_snapshot)
                {
                    return result;
                }
            }
        }

        let update = self.update_to_stop(ctx);

        if !ctx.base_buy_enabled {
            return CommandStepResult::failed(update);
        }

        let buyer_owner = ctx.object.owner;
        if buyer_owner == OWNER_NONE {
            return CommandStepResult::failed(update);
        }

        let base_id = match self.resolve_base(ctx) {
            Some(id) => id,
            None => return CommandStepResult::failed(update),
        };

        let base_snapshot = match ctx.resolve(base_id) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(update),
        };

        let base_owner = base_snapshot.owner;
        if base_owner == OWNER_NONE {
            return CommandStepResult::failed(update);
        }

        let base_player = match ctx.player(base_owner) {
            Some(player) if player.is_active() => player,
            _ => return CommandStepResult::failed(update),
        };

        let available = base_player.material_count(&self.definition_id);
        if available == 0 {
            return CommandStepResult::failed(update);
        }

        let price = ctx
            .definition(&self.definition_id)
            .map(|definition| definition.value.max(0))
            .unwrap_or(0);

        if price > base_player.wealth {
            return CommandStepResult::failed(update);
        }

        let mut events = Vec::new();
        events.push(CommandEvent::AdjustPlayerHomeBaseMaterial {
            player_id: base_owner,
            definition_id: self.definition_id.clone(),
            delta: -1,
        });
        if price != 0 {
            events.push(CommandEvent::AdjustPlayerWealth {
                player_id: base_owner,
                delta: -price,
            });
        }
        events.push(CommandEvent::SpawnObject {
            definition_id: self.definition_id.clone(),
            owner: buyer_owner,
            position: base_snapshot.position,
            container: Some(base_id),
        });

        CommandStepResult::completed(update).with_events(events)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct HomeState {
    target: Option<ObjectId>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_enter_request: Option<u64>,
}

impl HomeState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            target: request.target,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_enter_request: None,
        })
    }

    fn update_to_stop(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
    }

    fn is_base(snapshot: &CommandObjectSnapshot, owner: i32) -> bool {
        snapshot.is_active()
            && snapshot.owner == owner
            && (snapshot.category & CATEGORY_STRUCTURE) != 0
            && (snapshot.ocf & ocf::ENTRANCE) != 0
            && !snapshot.collectible
    }

    fn is_home(&self, ctx: &CommandRuntimeContext<'_>) -> bool {
        match ctx.object.container {
            Some(container_id) => ctx
                .resolve(container_id)
                .map(|snapshot| Self::is_base(snapshot, ctx.object.owner))
                .unwrap_or(false),
            None => false,
        }
    }

    fn resolve_base(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        let owner = ctx.object.owner;
        if let Some(target_id) = self.target {
            if let Some(snapshot) = ctx.resolve(target_id) {
                if Self::is_base(snapshot, owner) {
                    return Some(target_id);
                }
            }
        }

        ctx.objects
            .values()
            .filter(|snapshot| snapshot.id != ctx.object.id && Self::is_base(snapshot, owner))
            .min_by_key(|snapshot| {
                let dx = i64::from(snapshot.position.x - ctx.position.x);
                let dy = i64::from(snapshot.position.y - ctx.position.y);
                dx * dx + dy * dy
            })
            .map(|snapshot| snapshot.id)
    }

    fn should_issue_enter(&self, frame: u64) -> bool {
        const ENTER_COOLDOWN: u64 = 12;
        match self.last_enter_request {
            Some(last) => frame.saturating_sub(last) >= ENTER_COOLDOWN,
            None => true,
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.is_home(ctx) {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let base_id = match self.resolve_base(ctx) {
            Some(id) => {
                self.target = Some(id);
                id
            }
            None => return CommandStepResult::failed(self.update_to_stop(ctx)),
        };

        let update = self.update_to_stop(ctx);
        if self.should_issue_enter(ctx.frame) {
            self.last_enter_request = Some(ctx.frame);
            let request = CommandRequest::new(CommandId::Enter)
                .with_target(Some(base_id))
                .with_update_interval(25)
                .with_mode(CommandMode::Sub);
            return CommandStepResult::running(update)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        CommandStepResult::running(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum CommandState {
    Follow(FollowState),
    MoveTo(MoveToState),
    Enter(EnterState),
    Exit(ExitState),
    Build(BuildState),
    Chop(ChopState),
    Grab(GrabState),
    Throw(ThrowState),
    UnGrab(UnGrabState),
    Jump(JumpState),
    Wait(WaitState),
    Put(PutState),
    Drop(DropState),
    Get(GetState),
    Dig(DigState),
    Retry(RetryState),
    Attack(AttackState),
    Buy(BuyState),
    Acquire(AcquireState),
    Home(HomeState),
    Energy(EnergyState),
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ActiveCommand {
    state: CommandState,
}

impl ActiveCommand {
    fn from_request(request: CommandRequest) -> Result<Self, CommandError> {
        let state = match request.id {
            CommandId::Follow => CommandState::Follow(FollowState::from_request(&request)?),
            CommandId::MoveTo => CommandState::MoveTo(MoveToState::from_request(&request)),
            CommandId::Enter => CommandState::Enter(EnterState::from_request(&request)?),
            CommandId::Exit => CommandState::Exit(ExitState::from_request(&request)?),
            CommandId::Build => CommandState::Build(BuildState::from_request(&request)?),
            CommandId::Chop => CommandState::Chop(ChopState::from_request(&request)?),
            CommandId::Grab => CommandState::Grab(GrabState::from_request(&request)?),
            CommandId::Throw => CommandState::Throw(ThrowState::from_request(&request)?),
            CommandId::UnGrab => CommandState::UnGrab(UnGrabState::from_request(&request)),
            CommandId::Jump => CommandState::Jump(JumpState::from_request(&request)),
            CommandId::Wait => CommandState::Wait(WaitState::from_request(&request)),
            CommandId::Put => CommandState::Put(PutState::from_request(&request)?),
            CommandId::Drop => CommandState::Drop(DropState::from_request(&request)),
            CommandId::Get => CommandState::Get(GetState::from_request(&request)?),
            CommandId::Dig => CommandState::Dig(DigState::from_request(&request)?),
            CommandId::Retry => CommandState::Retry(RetryState::from_request(&request)),
            CommandId::Attack => CommandState::Attack(AttackState::from_request(&request)?),
            CommandId::Buy => CommandState::Buy(BuyState::from_request(&request)?),
            CommandId::Acquire => CommandState::Acquire(AcquireState::from_request(&request)?),
            CommandId::Home => CommandState::Home(HomeState::from_request(&request)?),
            CommandId::Energy => CommandState::Energy(EnergyState::from_request(&request)?),
            _ => CommandState::Unsupported,
        };

        if matches!(state, CommandState::Unsupported) {
            return Err(CommandError::Unsupported);
        }

        Ok(Self { state })
    }

    fn from_state(state: CommandState) -> Self {
        Self { state }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        match &mut self.state {
            CommandState::Follow(state) => state.step(ctx),
            CommandState::MoveTo(state) => state.step(ctx),
            CommandState::Enter(state) => state.step(ctx),
            CommandState::Exit(state) => state.step(ctx),
            CommandState::Build(state) => state.step(ctx),
            CommandState::Chop(state) => state.step(ctx),
            CommandState::Grab(state) => state.step(ctx),
            CommandState::Throw(state) => state.step(ctx),
            CommandState::UnGrab(state) => state.step(ctx),
            CommandState::Jump(state) => state.step(ctx),
            CommandState::Wait(state) => state.step(ctx),
            CommandState::Put(state) => state.step(ctx),
            CommandState::Drop(state) => state.step(ctx),
            CommandState::Get(state) => state.step(ctx),
            CommandState::Dig(state) => state.step(ctx),
            CommandState::Retry(state) => state.step(ctx),
            CommandState::Attack(state) => state.step(ctx),
            CommandState::Buy(state) => state.step(ctx),
            CommandState::Acquire(state) => state.step(ctx),
            CommandState::Home(state) => state.step(ctx),
            CommandState::Energy(state) => state.step(ctx),
            CommandState::Unsupported => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                CommandStepResult::failed(Some(update))
            }
        }
    }
}
