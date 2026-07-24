use std::collections::{HashMap, HashSet, VecDeque};

use crate::math::{self, FixedVec2};
use crate::pathfinder::{PathFinder, PathfinderDebugSnapshot};
use crate::transfer::{TransferZone, TransferZoneTable};
use crate::{
    minimum_con_activation_denied, ocf, ActionProcedure, ActionUpdate, CommandDirection,
    DefinitionId, DefinitionRect, Direction, ObjectId, ObjectStatus, ObjectUpdate, PlayerStatus,
    Vector2, CATEGORY_SORT_LIMIT, CATEGORY_STATIC_BACK, CATEGORY_STRUCTURE, CATEGORY_VEHICLE,
    FULL_CON, LINE_CONNECT_POWER_INPUT, OWNER_NONE,
};
use clonk_resources::PhysicalInfo;
use serde::{Deserialize, Serialize};

/// Maximum number of commands that may be queued for an object.
pub const MAX_COMMAND_STACK: usize = 35;
const LINEKIT_DEFINITION: &str = "LNKT";
const POWERLINE_DEFINITION: &str = "PWRL";
const SOURCE_PIPE_DEFINITION: &str = "SPIP";
const DRAIN_PIPE_DEFINITION: &str = "DPIP";
const CONNECT_ACTION: &str = "Connect";
const CONKIT_DEFINITION: &str = "CNKT";
const ACQUIRE_REQUEST_INTERVAL: i32 = 50;
const COMMAND_FLAG_ENTER_PUSH_TARGET: i32 = 0b10;
const COMMAND_FLAG_MOVE_TO_NO_POS_ADJUST: i32 = 0b1;
const COMMAND_FLAG_MOVE_TO_PUSH_TARGET: i32 = 0b10;
const DIG_MOVE_TO_RANGE_DEFAULT: i32 = 5;
const DIG_OUT_POSITION_RANGE: i32 = 15;
const DIG_DIRECTION_RANGE: i32 = 1;
const PUSH_TO_RANGE: i32 = 10;

#[derive(Debug, Clone)]
pub struct CommandObjectSnapshot {
    pub id: ObjectId,
    /// Forward C++ `Game.Objects` list position. The engine stores that
    /// master list reversed for execution, while command searches such as
    /// Acquire use the forward order to break equal-distance ties.
    pub master_list_order: usize,
    pub definition_id: DefinitionId,
    pub position: Vector2,
    /// Authoritative C4Object fix_x/fix_y used by fixed-point command paths.
    pub fixed_position: FixedVec2,
    /// Authoritative C4Object xdir/ydir used by momentum-aware steering.
    pub fixed_velocity: FixedVec2,
    /// Raw DefCore MoveToRange; positive values override the default five.
    pub move_to_range: i32,
    /// Raw DefCore Pathfinder; nonzero enables MoveTo path search for
    /// non-crew objects and is clamped to [1,10] by C4PathFinder::SetLevel.
    pub pathfinder: i32,
    /// DefCore NoTransferZones; suppresses transfer-zone edges for this
    /// object's MoveTo path search.
    pub no_transfer_zones: i32,
    /// DefCore NoPushEnter; any nonzero value makes C4Command::Enter fail
    /// before its already-contained and geometry checks.
    pub no_push_enter: i32,
    pub status: ObjectStatus,
    pub destroyed: bool,
    pub category: i32,
    pub container: Option<ObjectId>,
    pub action_name: String,
    /// Whether Action.Act is ActIdle (or the inactive ActHold slot), as
    /// opposed to a real ActMap entry that happens to be named "Idle".
    pub action_idle: bool,
    /// Whether the current physical ActMap entry has ObjectDisabled set.
    /// Built-in ActIdle/ActHold slots remain false regardless of name.
    pub action_disabled: bool,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub action_procedure: ActionProcedure,
    pub command_direction: CommandDirection,
    pub construction: i32,
    /// Facing (C4Object Action.Dir) for ComDir-less jump direction.
    pub direction: Direction,
    /// The resolved GetPhysical view (temporary→info→definition).
    pub physical: PhysicalInfo,
    /// True only when reading `physical` would perform the first scripted
    /// fair-crew cache fill. Command handlers suspend at their exact native
    /// GetPhysical seam instead of observing this non-callback placeholder.
    pub physical_deferred: bool,
    pub owner: i32,
    /// C4Object::Controller, used when commands arm work on a target.
    pub controller: i32,
    /// C4Object::Base: the player whose home-base material and wealth this
    /// object brokers, independently of the object's owner.
    pub base: i32,
    pub crew_member: bool,
    pub selected: bool,
    pub alive: bool,
    /// Live C4Object::NeedEnergy marker used by C4Command::Energy's
    /// already-supplied completion check.
    pub need_energy: bool,
    /// Raw C4Object::OnFire, which Acquire reads independently of cached OCF.
    pub on_fire: bool,
    pub contents: Vec<ObjectId>,
    /// Every linked command, top first. `FindObjectByCommand` scans the full
    /// stack rather than only the executing entry (C4Game.cpp:3764-3784).
    pub commands: Vec<CommandView>,
    pub line_connect: u32,
    pub ocf: u32,
    /// C4Object::EntranceStatus, consulted separately from OCF_Entrance by
    /// C4Command::Enter (C4Command.cpp:600-609).
    pub entrance_status: bool,
    pub collectible: bool,
    /// Live vertex-contact bits (C4Object::t_contact equivalent; CNAT_*).
    pub contact: u32,
    /// Frames in the current action (C4Object Action.Time) for the
    /// "not if just started" let-go contact check (C4Command.cpp:348,362).
    pub action_time: i32,
    /// Current shape top (C4Object Shape.y) for the top-free scans
    /// (C4Command.cpp:1867).
    pub shape_top: i32,
    /// Raw current C4Shape::Hgt. Unlike [`Self::shape`], this is not expanded
    /// to the eighteen-pixel `At`/`Height` action area; ballistic Throw uses
    /// the shape field verbatim (C4Command.cpp:942).
    pub shape_height: i32,
    /// The absolute (position-applied) shape rect for `C4Object::At`
    /// point-in-shape tests (C4Object.cpp At(), used by C4Command::Enter
    /// :587-588 and Grab :690-691).
    pub shape: DefinitionRect,
    /// Absolute DefCore entrance area when OCF_Entrance is currently set.
    /// C4Command::Exit uses its center/bottom for top-level ejection
    /// (C4Command.cpp:624-645).
    pub entrance: Option<DefinitionRect>,
}

impl CommandObjectSnapshot {
    /// `C4Object::At(ctx, cty)` without the OCF mask: point in shape.
    pub fn at_point(&self, x: i32, y: i32) -> bool {
        self.shape.contains_point(x, y)
    }

    /// Raw absolute `C4Object::Shape` used by `Game.OverlapObject`.
    /// `shape` normally carries the eighteen-pixel `At` expansion, so undo
    /// that expansion when the snapshot fields identify it. Hand-built
    /// fixtures and vertex fallbacks keep their explicit rectangle.
    fn raw_shape_rect(&self) -> DefinitionRect {
        let add_top = (18 - self.shape_height).max(0);
        let expanded_top = self
            .position
            .y
            .saturating_add(self.shape_top)
            .saturating_sub(add_top);
        let expanded_height = self.shape_height.saturating_add(add_top);
        if self.shape.y == expanded_top && self.shape.height == expanded_height {
            DefinitionRect::new(
                self.shape.x,
                self.position.y.saturating_add(self.shape_top),
                self.shape.width,
                self.shape_height,
            )
        } else {
            self.shape
        }
    }
}

impl CommandObjectSnapshot {
    /// C++ APIs that explicitly test `C4Object::Status` accept every
    /// nonzero value, including C4OS_INACTIVE. Raw pointers are a separate
    /// concern: a detached command may retain one even after Status reaches
    /// zero, until an actual ClearPointers walk reaches that command.
    pub fn has_nonzero_status(&self) -> bool {
        !self.destroyed && self.status != ObjectStatus::Deleted
    }

    /// C4Object::Status without C4Object::Alive. Structures and ordinary
    /// items are active command targets even though they are not living.
    pub fn is_status_active(&self) -> bool {
        self.has_nonzero_status() && self.status.is_active()
    }

    pub fn is_active(&self) -> bool {
        self.is_status_active() && self.alive
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPlayerSnapshot {
    pub status: PlayerStatus,
    pub surrendered: bool,
    pub wealth: i32,
    pub home_base_material: HashMap<DefinitionId, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub home_base_material_entries: Vec<(DefinitionId, i32)>,
    #[serde(default)]
    pub knowledge: Vec<DefinitionId>,
    /// One-way hostility declarations. C4PlayerList::Hostile treats a
    /// declaration from either player as hostility in both directions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hostile_to: Vec<i32>,
}

impl CommandPlayerSnapshot {
    pub fn is_active(&self) -> bool {
        matches!(self.status, PlayerStatus::Active) && !self.surrendered
    }

    pub fn material_count(&self, definition_id: &str) -> i32 {
        if self.home_base_material_entries.is_empty() {
            return self
                .home_base_material
                .get(definition_id)
                .copied()
                .map(|count| i32::try_from(count).unwrap_or(i32::MAX))
                .unwrap_or(0);
        }
        self.home_base_material_entries
            .iter()
            .find(|(id, _)| id == definition_id)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    pub fn knows(&self, definition_id: &DefinitionId) -> bool {
        self.knowledge.iter().any(|entry| entry == definition_id)
    }

    pub fn is_hostile_towards(&self, player: i32) -> bool {
        self.hostile_to.contains(&player)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandDefinitionSnapshot {
    pub value: i32,
    /// Raw definition shape used by construction-site search/checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<DefinitionRect>,
    /// Raw DefCore category used by `Game.OverlapObject` construction vetoes.
    #[serde(default)]
    pub category: i32,
    /// DefCore `ConSizeOff`, subtracted from shape height by ConstructionCheck.
    #[serde(default)]
    pub construction_offset: i32,
    /// Raw signed DefCore `CollectionLimit`; zero alone means unlimited.
    #[serde(default, skip_serializing_if = "crate::i32_is_zero")]
    pub collection_limit: i32,
    /// DefCore collection area relative to the target object's position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_rect: Option<DefinitionRect>,
    /// DefCore `Fragile`; any nonzero source value disables Put's outdoor
    /// throw-in path for this item definition.
    #[serde(default)]
    pub fragile: bool,
    /// Raw DefCore `Projectile`; Attack selects the first contents item
    /// whose definition carries any nonzero value.
    #[serde(default)]
    pub projectile: i32,
    #[serde(default)]
    pub can_chop: bool,
    #[serde(default)]
    pub chop_action: Option<String>,
    #[serde(default)]
    pub constructable: bool,
    /// DefCore `Grab` (0 none, 1 grab+push, 2 grab-only) for the
    /// pushing let-go checks (C4Command.cpp:260, :565).
    #[serde(default)]
    pub grab: i32,
    /// DefCore `GrabPutGet`; bit 2 (`C4D_Grab_Get`) permits Get to take
    /// contents through a pushed container (C4Command.cpp:1226-1238).
    #[serde(default)]
    pub grab_put_get: i32,
    /// DefCore `NoGet`; contained objects with this flag remain inaccessible
    /// to C4Command::Get (C4Command.cpp:1209-1211).
    #[serde(default)]
    pub no_get: bool,
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
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::Follow),
            2 => Some(Self::MoveTo),
            3 => Some(Self::Enter),
            4 => Some(Self::Exit),
            5 => Some(Self::Grab),
            6 => Some(Self::Build),
            7 => Some(Self::Throw),
            8 => Some(Self::Chop),
            9 => Some(Self::UnGrab),
            10 => Some(Self::Jump),
            11 => Some(Self::Wait),
            12 => Some(Self::Get),
            13 => Some(Self::Put),
            14 => Some(Self::Drop),
            15 => Some(Self::Dig),
            16 => Some(Self::Activate),
            17 => Some(Self::PushTo),
            18 => Some(Self::Construct),
            19 => Some(Self::Transfer),
            20 => Some(Self::Attack),
            21 => Some(Self::Context),
            22 => Some(Self::Buy),
            23 => Some(Self::Sell),
            24 => Some(Self::Acquire),
            25 => Some(Self::Energy),
            26 => Some(Self::Retry),
            27 => Some(Self::Home),
            28 => Some(Self::Call),
            29 => Some(Self::Take),
            30 => Some(Self::Take2),
            _ => None,
        }
    }

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

    /// `C4Command::GetExpGain` (C4Command.cpp:2441-2496): successful
    /// native command finishes add this many points to the attached
    /// `C4ObjectInfo::ControlCount`.
    pub(crate) const fn experience_gain(self) -> i32 {
        match self {
            Self::Wait | Self::Transfer | Self::Retry | Self::Call => 0,
            Self::Acquire | Self::Home => 2,
            Self::Chop | Self::Build | Self::Construct | Self::Energy => 5,
            Self::Attack => 15,
            Self::Follow
            | Self::MoveTo
            | Self::Enter
            | Self::Exit
            | Self::Grab
            | Self::Throw
            | Self::UnGrab
            | Self::Jump
            | Self::Get
            | Self::Put
            | Self::Drop
            | Self::Dig
            | Self::Activate
            | Self::PushTo
            | Self::Context
            | Self::Buy
            | Self::Sell
            | Self::Take
            | Self::Take2 => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::transfer::TransferZoneRect;
    use once_cell::sync::Lazy;

    static EMPTY_TRANSFER_ZONES: Lazy<TransferZoneTable> = Lazy::new(TransferZoneTable::default);

    use crate::ocf;

    // Bodies live in byte-verbatim contiguous parts so the module — and
    // every test id it exports — stays exactly as it was.
    include!("command/tests/part_01.rs");
    include!("command/tests/part_02.rs");
    include!("command/tests/part_03.rs");
    include!("command/tests/part_04.rs");
    include!("command/tests/part_05.rs");
    include!("command/tests/part_06.rs");
    include!("command/tests/part_07.rs");
}

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
const fn positive_helper_interval(interval: i32) -> u32 {
    if interval > 0 {
        interval as u32
    } else {
        0
    }
}

const fn positive_helper_interval_or_one(interval: i32) -> u32 {
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

fn resolve_command_physical(
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

fn stamp_command_event_instances(events: &mut [CommandEvent], instance_id: u64) {
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

fn command_data_to_definition_id(data: &CommandData) -> Option<DefinitionId> {
    match data {
        CommandData::Integer(value) => c4id_to_definition_string(*value),
        CommandData::Text(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

#[derive(Clone)]
pub struct CommandRuntimeContext<'a> {
    /// The synced game RNG (C4Random) — command AI draws (Get's
    /// Random(15)/Random(2), C4Command.cpp:1272-1290) advance the
    /// lockstep ledger. None in unit fixtures that don't pin draws.
    pub rng: Option<&'a std::cell::RefCell<crate::LcgRng>>,
    pub frame: u64,
    pub position: Vector2,
    /// Landscape probes for the walking controls (GBackSolid/PathFree in
    /// C4Command JumpControl/FlightControl, C4Command.cpp:1816-1920).
    pub landscape: Option<&'a crate::Landscape>,
    pub object: &'a CommandObjectSnapshot,
    pub objects: &'a HashMap<ObjectId, CommandObjectSnapshot>,
    pub players: &'a HashMap<i32, CommandPlayerSnapshot>,
    pub definitions: &'a HashMap<DefinitionId, CommandDefinitionSnapshot>,
    pub structures_need_energy: bool,
    pub base_buy_enabled: bool,
    pub base_sell_enabled: bool,
    pub transfer_zones: &'a TransferZoneTable,
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

    /// C4PlayerList::Hostile: missing or identical players are friendly;
    /// either player's one-way declaration makes the pair hostile.
    pub fn players_hostile(&self, first: i32, second: i32) -> bool {
        if first == second {
            return false;
        }
        let (Some(first_player), Some(second_player)) = (self.player(first), self.player(second))
        else {
            return false;
        };
        first_player.is_hostile_towards(second) || second_player.is_hostile_towards(first)
    }

    pub fn definition(&self, id: &str) -> Option<&CommandDefinitionSnapshot> {
        self.definitions.get(id)
    }

    pub fn transfer_zone(&self, owner: ObjectId) -> Option<&TransferZone> {
        self.transfer_zones.get(owner)
    }
}

#[derive(Debug)]
pub enum CommandError {
    StackFull,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSnapshot {
    /// Runtime identity of the C4Command node. Native pointer identity is
    /// not savegame state; persisted restores assign a fresh nonzero id.
    #[serde(skip)]
    instance_id: u64,
    state: CommandState,
    mode: CommandMode,
    retries: i32,
    failures: i32,
    /// Generic C4Command::Evaluated flag. Command-specific state retains
    /// the special InitEvaluation transitions; this field covers every
    /// ordinary command after its first Execute.
    #[serde(default)]
    evaluated: bool,
    /// Generic C4Command::PathChecked flag for command kinds that do not
    /// otherwise retain the pathfinder latch in their typed state.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    path_checked: bool,
    /// C4Command::Permit is normally zero, but compiled saves accept and
    /// re-emit arbitrary signed values.
    #[serde(default, skip_serializing_if = "crate::i32_is_zero")]
    permit: i32,
    /// Raw compiled boolean words. Native treats these as booleans at
    /// runtime but writes the original signed integer until the semantic
    /// value changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_evaluated_word: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_path_checked_word: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_finished_word: Option<i32>,
    /// Exact C4Command::Text from a compiled save. Non-Call commands may
    /// carry otherwise-unused text which still needs to survive a resave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_text: Option<String>,
    /// C4Command::UpdateInterval after the most recent Execute. Kept on
    /// the stack entry because it is shared by every command kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    update_interval: Option<i32>,
    /// The creating request — the base of the FnGetCommand element view
    /// (persisted so restored stacks keep their elements; pre-existing
    /// saves without it degrade to name-only views).
    #[serde(default)]
    request: Option<CommandRequest>,
    /// C4Command::Finished. Normally transient until
    /// C4Object::ExecuteCommand fires the callback and clears the front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished: Option<CommandStatus>,
}

impl PartialEq for CommandSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
            && self.mode == other.mode
            && self.retries == other.retries
            && self.failures == other.failures
            && self.evaluated == other.evaluated
            && self.path_checked == other.path_checked
            && self.permit == other.permit
            && self.legacy_evaluated_word == other.legacy_evaluated_word
            && self.legacy_path_checked_word == other.legacy_path_checked_word
            && self.legacy_finished_word == other.legacy_finished_word
            && self.legacy_text == other.legacy_text
            && self.update_interval == other.update_interval
            && self.request == other.request
            && self.finished == other.finished
    }
}

impl CommandSnapshot {
    fn new(entry: &ActiveCommand) -> Self {
        Self {
            instance_id: entry.instance_id,
            state: entry.state.clone(),
            mode: entry.mode,
            retries: entry.retries,
            failures: entry.failures,
            evaluated: entry.evaluated,
            path_checked: entry.path_checked,
            permit: entry.permit,
            legacy_evaluated_word: entry.legacy_evaluated_word,
            legacy_path_checked_word: entry.legacy_path_checked_word,
            legacy_finished_word: entry.legacy_finished_word,
            legacy_text: entry.legacy_text.clone(),
            update_interval: Some(entry.update_interval),
            request: entry.request.clone(),
            finished: entry.finished,
        }
    }
}

/// The FnGetCommand element view of one stack entry (C4Script.cpp:926-945):
/// name, Target, Tx, Ty, Target2, Data — the LIVE C4Command fields. The
/// creating CommandRequest is the base; states whose C++ counterpart
/// rewrites the fields (MoveTo's target absorption C4Command.cpp:1637,
/// PushTo's coordinate adjustment :1645-1652, Acquire's 500/250 range
/// defaults :1666-1670, Construct's found site :1757-1766) override it with
/// their live values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandView {
    pub name: String,
    pub target: Option<ObjectId>,
    pub tx: Option<i32>,
    /// Exact tagged Tx for Call/GetCommand/save projection. Older snapshots
    /// reconstruct it from the integer/C4ID mirrors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_value: Option<clonk_script::Value>,
    pub tx_definition: Option<DefinitionId>,
    pub ty: Option<i32>,
    pub target2: Option<ObjectId>,
    pub data: CommandData,
    /// Independent C4Command::Data word when `data` carries Call's function
    /// Text. Native Call stores both fields; script APIs normally seed zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_data: Option<i32>,
    /// Whether C4Command::Finished is set while the entry remains linked.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub finished: bool,
}

/// Complete C4Command::CompileFunc projection used by Objects.txt saves.
/// Unlike CommandView, this includes native scheduler and retry state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyCommandSave {
    pub(crate) view: CommandView,
    pub(crate) update_interval: i32,
    pub(crate) evaluated: i32,
    pub(crate) path_checked: i32,
    pub(crate) finished: i32,
    pub(crate) failures: i32,
    pub(crate) retries: i32,
    pub(crate) permit: i32,
    pub(crate) base_mode: i32,
    pub(crate) text: String,
}

/// The failed command fields frozen before its script feedback can mutate
/// or clear the live stack. Command-specific feedback (notably CallFailed)
/// must use these exact live values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandFailureFeedback {
    pub command: CommandView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CommandFailureReason>,
}

impl CommandView {
    fn from_entry(
        name: String,
        request: Option<&CommandRequest>,
        state: &CommandState,
        finished: bool,
    ) -> Self {
        let mut view = Self {
            name,
            target: request.and_then(|request| request.target),
            tx: request.and_then(|request| request.tx),
            tx_value: request.and_then(|request| {
                request.tx_value.clone().or_else(|| {
                    request
                        .tx_definition
                        .as_ref()
                        .map(|value| clonk_script::Value::C4Id(value.clone()))
                        .or_else(|| request.tx.map(clonk_script::Value::Int))
                })
            }),
            tx_definition: request.and_then(|request| request.tx_definition.clone()),
            ty: request.and_then(|request| request.ty),
            target2: request.and_then(|request| request.target2),
            data: request
                .map(|request| request.data.clone())
                .unwrap_or(CommandData::None),
            legacy_data: request
                .filter(|request| request.id == CommandId::Call)
                .map(|_| 0),
            finished,
        };
        state.apply_live_overrides(&mut view);
        view
    }
}

fn legacy_command_save(
    view: CommandView,
    request: Option<&CommandRequest>,
    state: &CommandState,
    update_interval: i32,
    evaluated: bool,
    path_checked: bool,
    finished: bool,
    failures: i32,
    retries: i32,
    permit: i32,
    mode: CommandMode,
    legacy_evaluated_word: Option<i32>,
    legacy_path_checked_word: Option<i32>,
    legacy_finished_word: Option<i32>,
    legacy_text: Option<&str>,
) -> LegacyCommandSave {
    let text = match (&view.data, state.id()) {
        (CommandData::Text(text), Some(CommandId::Call)) => text.clone(),
        _ => String::new(),
    };
    let evaluated = state.legacy_evaluated(evaluated);
    let path_checked = state.legacy_path_checked(path_checked);
    LegacyCommandSave {
        view,
        update_interval,
        evaluated: legacy_bool_word(legacy_evaluated_word, evaluated),
        path_checked: legacy_bool_word(legacy_path_checked_word, path_checked),
        finished: legacy_bool_word(legacy_finished_word, finished),
        failures,
        retries,
        permit,
        base_mode: mode.to_i32(),
        text: legacy_text.map(str::to_owned).unwrap_or_else(|| {
            request
                .filter(|request| request.id == CommandId::Call)
                .and_then(|request| match &request.data {
                    CommandData::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or(text)
        }),
    }
}

fn legacy_bool_word(raw: Option<i32>, semantic: bool) -> i32 {
    raw.filter(|raw| (*raw != 0) == semantic)
        .unwrap_or(i32::from(semantic))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandStackSnapshot {
    commands: Vec<CommandSnapshot>,
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
    entries: VecDeque<ActiveCommand>,
    /// Nonzero identity allocator for native-command lifetime matching.
    next_instance_id: u64,
    detached_grab_attempts: Vec<DetachedGrabAttempt>,
    /// GetTryEnter crosses several callback boundaries. Preserve the exact
    /// executing Get if one of those callbacks unlinks it from the object.
    detached_get_attempts: VecDeque<DetachedGetAttempt>,
    /// A callback may ClearCommands/SetCommand while ObjectComStop is
    /// running. Native keeps the executing C4Command alive through its
    /// iExec guard, so retain that detached MoveTo until the same-Execute
    /// continuation consumes it.
    detached_move_to_stops: VecDeque<MoveToState>,
    /// FlightControl's ordinary Fly action can run callbacks which unlink
    /// the executing MoveTo before its walking-only JumpControl tail.
    detached_move_to_flights: VecDeque<DetachedMoveToFlight>,
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
    detached_throw_preludes: VecDeque<DetachedThrowPrelude>,
    /// Drop has the same retained ObjectComStop boundary when it starts in
    /// DFA_DIG. Keep a detached body alive if callbacks replace the stack.
    detached_drop_preludes: VecDeque<DetachedDropPrelude>,
    /// Put's callbackful ObjectComPut may unlink the executing command.
    /// Retain its failure/base context until the helper returns.
    detached_put_attempts: VecDeque<DetachedPutAttempt>,
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
struct DetachedGrabAttempt {
    target: ObjectId,
    target_retained: bool,
}

#[derive(Debug, Clone)]
struct DetachedThrowPrelude {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
struct DetachedMoveToFlight {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
struct DetachedBuildStop {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
struct DetachedExitPrelude {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
struct DetachedDropPrelude {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone, PartialEq)]
struct DetachedPutAttempt {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
struct DetachedPutStop {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
struct DetachedConstructCommand {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone, PartialEq)]
struct DetachedGetAttempt {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone)]
struct DetachedPhysicalCommand {
    entry: ActiveCommand,
    base_chain: Vec<DetachedCommandBase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DetachedCommandBase {
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

    fn pop_front(&mut self) -> Option<ActiveCommand> {
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

    fn apply_result_operations(&mut self, result: &mut CommandStepResult) {
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
    fn record_failure_at(&mut self, index: usize) -> Option<CommandFailureFeedback> {
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

    fn record_detached_failure(
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

// C4Command.cpp:31-36 movement-control constants.
const LET_GO_RANGE1: i32 = 7;
const LET_GO_RANGE2: i32 = 30;
const LET_GO_HANGLE_ANGLE: i32 = 110;
const JUMP_ANGLE: i32 = 35;
const JUMP_LOW_ANGLE: i32 = 80;
const JUMP_ANGLE_RANGE: i32 = 10;
const JUMP_HIGH_ANGLE: i32 = 0;
const FLIGHT_ANGLE_RANGE: i32 = 60;
const PATH_RANGE: i32 = 20;
const MAX_PATH_RANGE: i32 = 1_000;

fn inside(value: i32, lo: i32, hi: i32) -> bool {
    value >= lo && value <= hi
}

fn bound_by(value: i32, lo: i32, hi: i32) -> i32 {
    // C++ BoundBy preserves the supplied bound order, including the
    // inverted bounds produced by negative transfer-zone extents.
    if value < lo {
        lo
    } else if value > hi {
        hi
    } else {
        value
    }
}

/// `Angle` (C4Math.cpp:33-45): 0 = up, 90 = right, clockwise; float
/// atan2 truncated like the C++ `static_cast<int>`.
fn c4_angle(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dy = (y1 - y2).abs() as f32;
    let dx = (x1 - x2).abs() as f32;
    let angle =
        (180.0_f64 * f64::from(dy.atan2(dx)) * f64::from(std::f32::consts::FRAC_1_PI)) as i32;
    if x2 > x1 {
        if y2 < y1 {
            90 - angle
        } else {
            90 + angle
        }
    } else if y2 < y1 {
        270 + angle
    } else {
        270 - angle
    }
}

/// `Distance` (C4Math.cpp:22-31): integer sqrt with the double-step
/// correction.
pub(crate) fn c4_distance(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dx = i64::from(x1) - i64::from(x2);
    let dy = i64::from(y1) - i64::from(y2);
    let d2 = dx * dx + dy * dy;
    let mut dist = (d2 as f64).sqrt() as i64;
    if dist * dist < d2 {
        dist += 1;
    }
    if dist * dist > d2 {
        dist -= 1;
    }
    dist as i32
}

/// The global `PathFree` probe used before C4PathFinder
/// (C4Command.cpp:235; C4Landscape.cpp:1683-1738,2052-2055).
fn command_path_free(
    landscape: &crate::Landscape,
    mut x1: i32,
    mut y1: i32,
    mut x2: i32,
    mut y2: i32,
) -> bool {
    if (x2 - x1).abs() < (y2 - y1).abs() {
        if y1 > y2 {
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut y1, &mut y2);
        }
        let xincr = if x2 > x1 { 1 } else { -1 };
        let dy = y2 - y1;
        let dx = (x2 - x1).abs();
        let mut d = 2 * dx - dy;
        let aincr = 2 * (dx - dy);
        let bincr = 2 * dx;
        let mut x = x1;
        if landscape.is_solid_at(x, y1) {
            return false;
        }
        for y in (y1 + 1)..=y2 {
            if d >= 0 {
                x += xincr;
                d += aincr;
            } else {
                d += bincr;
            }
            if landscape.is_solid_at(x, y) {
                return false;
            }
        }
    } else {
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut y1, &mut y2);
        }
        let yincr = if y2 > y1 { 1 } else { -1 };
        let dx = x2 - x1;
        let dy = (y2 - y1).abs();
        let mut d = 2 * dy - dx;
        let aincr = 2 * (dy - dx);
        let bincr = 2 * dy;
        let mut y = y1;
        if landscape.is_solid_at(x1, y) {
            return false;
        }
        for x in (x1 + 1)..=x2 {
            if d >= 0 {
                y += yincr;
                d += aincr;
            } else {
                d += bincr;
            }
            if landscape.is_solid_at(x, y) {
                return false;
            }
        }
    }
    true
}

/// `AdjustSolidOffset` (C4Command.cpp:126-143): move a pathfinder waypoint
/// away from nearby solid pixels by half the moving object's shape.
fn adjust_solid_offset(
    landscape: &crate::Landscape,
    x: &mut i32,
    y: &mut i32,
    x_offset: i32,
    y_offset: i32,
) -> bool {
    if landscape.is_solid_at(*x, *y) {
        return false;
    }
    for offset in 1..y_offset {
        if landscape.is_solid_at(*x, *y + offset) && !landscape.is_solid_at(*x, *y - offset) {
            *y -= 1;
        }
        if landscape.is_solid_at(*x, *y - offset) && !landscape.is_solid_at(*x, *y + offset) {
            *y += 1;
        }
    }
    for offset in 1..x_offset {
        if landscape.is_solid_at(*x + offset, *y) && !landscape.is_solid_at(*x - offset, *y) {
            *x -= 1;
        }
        if landscape.is_solid_at(*x - offset, *y) && !landscape.is_solid_at(*x + offset, *y) {
            *x += 1;
        }
    }
    true
}

/// `ObjectComLetGo` (C4ObjectCom.cpp:310-314) as an object update:
/// ObjectActionJump(itofix(xdirf), Fix0) — the hardcoded Jump action plus
/// the launch velocity (the fixed-velocity delta apply arms mobility,
/// matching Mobile=1 in ObjectActionJump). Any pending ComDir steer from
/// the same Execute rides along.
fn let_go_update(steer: Option<CommandDirection>, xdirf: i32) -> ObjectUpdate {
    let mut update = ObjectUpdate::new();
    if let Some(direction) = steer {
        update = update.with_command_direction(direction);
    }
    let mut update = update.with_action_update(
        ActionUpdate::default()
            .with_name("Jump")
            .with_phase(0)
            .with_ticks(0)
            .with_force(true),
    );
    update.fixed_velocity = Some(FixedVec2::new(
        math::itofix(xdirf),
        crate::C4Fixed::from_raw(0),
    ));
    update
}

/// `SolidOnWhichSide` (C4Command.cpp:147-156).
fn solid_on_which_side(landscape: &crate::Landscape, x: i32, y: i32) -> i32 {
    for cx in 1..10 {
        for cy in 0..10 {
            if landscape.is_solid_at(x - cx, y - cy) || landscape.is_solid_at(x - cx, y + cy) {
                return -1;
            }
            if landscape.is_solid_at(x + cx, y - cy) || landscape.is_solid_at(x + cx, y + cy) {
                return 1;
            }
        }
    }
    0
}

/// `AdjustMoveToTarget` (C4Command.cpp:94-114): raise above solid, then
/// (walking) drop to the bottom of free space and lift half a shape
/// above near-ground solid.
fn adjust_move_to_target(
    landscape: &crate::Landscape,
    x: &mut i32,
    y: &mut i32,
    free_move: bool,
    shape_height: i32,
) {
    let mut iy = *y;
    while iy >= 0 && landscape.is_solid_at(*x, iy) {
        iy -= 1;
    }
    if iy >= 0 {
        *y = iy;
    }
    if !free_move {
        if !landscape.is_semi_solid_at(*x, *y) {
            let back_hgt = landscape.estimated_height();
            let mut iy = *y;
            while iy < back_hgt && !landscape.is_semi_solid_at(*x, iy + 1) {
                iy += 1;
            }
            if iy < back_hgt {
                *y = iy;
            }
        }
        if (landscape.is_solid_at(*x, *y + 1) || landscape.is_solid_at(*x, *y + 5))
            && !landscape.is_semi_solid_at(*x, *y - shape_height / 2)
        {
            *y -= shape_height / 2;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MoveToStopContinuation {
    /// MoveTo's cx/cy, including the final-waypoint push/pull substitution,
    /// are captured before ObjectComStop callbacks execute.
    position: Vector2,
    target: Vector2,
    /// The definition MoveToRange/default-five value is likewise captured;
    /// a live post-stop crew still overrides it from its current shape.
    target_range: i32,
    next_is_move_to: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum MoveToPhysicalContinuation {
    InitEvaluation {
        x: i32,
        y: i32,
    },
    Float {
        fixed_dx: crate::C4Fixed,
        fixed_dy: crate::C4Fixed,
    },
    FlightControl {
        target: Vector2,
        from_walk: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MoveToFlightContinuation {
    target: Vector2,
    jump_after_takeoff: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MoveToState {
    target: Option<ObjectId>,
    tx: Option<i32>,
    ty: Option<i32>,
    /// C4CMD_MoveTo Data flags (C4CMD_MoveTo_NoPosAdjust/PushTarget,
    /// C4Command.h:68-69).
    #[serde(default)]
    data: i32,
    /// C4Command::Evaluated — false until the InitEvaluation Execute has
    /// absorbed Target and adjusted Tx/Ty, except for pathfinder waypoints
    /// created with fInitEvaluation=false (C4Command.cpp:189-209,1625-1643).
    #[serde(default)]
    evaluated: bool,
    /// C4Command::PathChecked suppresses repeated path searches until the
    /// next Tick35 recheck (C4Command.cpp:230-255).
    #[serde(default)]
    path_checked: bool,
    update_interval: u32,
    tolerance: i32,
    last_direction: CommandDirection,
    /// Same-Execute continuation staged while the engine performs the live
    /// ObjectComStop (Idle then Walk with callbacks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stop_continuation: Option<MoveToStopContinuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    physical_continuation: Option<MoveToPhysicalContinuation>,
    /// Same-Execute tail after FlightControl's callbackful Fly transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    flight_continuation: Option<MoveToFlightContinuation>,
    /// Runtime handoff to the engine-owned `Game.PathFinder` settings.
    /// The event is consumed in the same Execute and is not save state.
    #[serde(skip)]
    pathfinder_settings_update: Option<(i32, bool)>,
    /// Runtime-only viewport handoff for `C4PathFinder::Draw`.
    #[serde(skip)]
    pathfinder_debug_update: Option<PathfinderDebugSnapshot>,
}

impl MoveToState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            target: request.target,
            tx: request.tx,
            ty: request.ty,
            data: match request.data {
                CommandData::Integer(value) => value,
                _ => 0,
            },
            evaluated: request.evaluated,
            path_checked: false,
            update_interval: positive_helper_interval(request.update_interval),
            tolerance: 5,
            last_direction: CommandDirection::Stop,
            stop_continuation: None,
            physical_continuation: None,
            flight_continuation: None,
            pathfinder_settings_update: None,
            pathfinder_debug_update: None,
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

    /// C4CMD_MoveTo InitEvaluation (C4Command.cpp:1634-1643): absorb the
    /// Target position into Tx/Ty once (Target clears, :1637) and ground
    /// the destination via AdjustMoveToTarget unless Data carries
    /// C4CMD_MoveTo_NoPosAdjust (:1640, C4Command.h:68). FreeMoveTo
    /// accepts any spot for floaters and CanFly physicals (:116-124).
    fn init_evaluation(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<CommandStepResult> {
        if let Some(target) = self.target.take() {
            if let Some(position) = ctx.resolve_position(target) {
                self.tx = Some(self.tx.unwrap_or(0) + position.x);
                self.ty = Some(self.ty.unwrap_or(0) + position.y);
            }
        }
        // C4Value null Tx reads as integer zero and Ty is a zero-initialized
        // integer. Native always writes the numeric Tx back after evaluation,
        // so omitted/partial coordinates still reach FreeMoveTo/GetPhysical.
        let tx = self.tx.unwrap_or(0);
        let ty = self.ty.unwrap_or(0);
        self.tx = Some(tx);
        self.ty = Some(ty);
        if self.data & COMMAND_FLAG_MOVE_TO_NO_POS_ADJUST == 0 {
            if let Some(landscape) = ctx.landscape {
                let free_move = if ctx.object.action_procedure == ActionProcedure::Float {
                    true
                } else if ctx.object.physical_deferred {
                    self.physical_continuation =
                        Some(MoveToPhysicalContinuation::InitEvaluation { x: tx, y: ty });
                    return Some(resolve_command_physical(ctx.object.id, 1, None));
                } else {
                    ctx.object.physical.can_fly != 0
                };
                let (mut x, mut y) = (tx, ty);
                adjust_move_to_target(
                    landscape,
                    &mut x,
                    &mut y,
                    free_move,
                    ctx.object.shape.height,
                );
                self.tx = Some(x);
                self.ty = Some(y);
            }
        }
        None
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        self.step_with_waypoint(ctx, false)
    }

    fn step_with_waypoint(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        next_is_move_to: bool,
    ) -> CommandStepResult {
        // The initial-evaluation Execute consumes the frame without
        // moving (`if (InitEvaluation()) return;`, C4Command.cpp:1555).
        if !self.evaluated {
            self.evaluated = true;
            return self
                .init_evaluation(ctx)
                .unwrap_or_else(|| CommandStepResult::running(None));
        }

        // C4Command::MoveTo leaves any container before pathfinding or
        // steering. The default AddCommand mode is SilentSub
        // (C4Command.cpp:213-217; C4Object.h:221-225).
        if ctx.object.container.is_some() {
            let exit = CommandRequest::new(CommandId::Exit)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(exit)]);
        }
        let target = match self.resolve_target_position(ctx) {
            Some(position) => position,
            None => return CommandStepResult::failed(None),
        };

        // C4Command::MoveTo path phase (C4Command.cpp:225-255): crew and
        // definitions with a nonzero Pathfinder participate; SetLevel
        // clamps the raw DefCore value to [1,10] (C4PathFinder.cpp:557-560).
        if (ctx.object.ocf & ocf::CREW_MEMBER != 0 || ctx.object.pathfinder != 0)
            && !self.path_checked
            && c4_distance(ctx.position.x, ctx.position.y, target.x, target.y) < MAX_PATH_RANGE
            && !(inside(ctx.position.x - target.x, -PATH_RANGE, PATH_RANGE)
                && inside(ctx.position.y - target.y, -PATH_RANGE, PATH_RANGE))
        {
            if let Some(landscape) = ctx.landscape {
                let direct_free = command_path_free(
                    landscape,
                    ctx.position.x,
                    ctx.position.y,
                    target.x,
                    target.y,
                );
                if direct_free {
                    self.path_checked = true;
                } else {
                    let level = ctx.object.pathfinder.clamp(1, 10);
                    let transfer_zones_enabled = ctx.object.no_transfer_zones == 0;
                    self.pathfinder_settings_update = Some((level, transfer_zones_enabled));
                    let transfer_zones = ctx.transfer_zones.states();
                    let mut finder = PathFinder::new(landscape, &transfer_zones);
                    finder.set_level(level);
                    finder.enable_transfer_zones(transfer_zones_enabled);
                    let path = finder.find(ctx.position, target);
                    self.pathfinder_debug_update = Some(finder.debug_snapshot().clone());
                    match path {
                        Some(path) if path.waypoints.len() > 2 => {
                            let waypoint_count = path.waypoints.len();
                            let mut operations = Vec::with_capacity(waypoint_count - 2);
                            for waypoint in
                                path.waypoints.into_iter().skip(1).take(waypoint_count - 2)
                            {
                                let request =
                                    if let Some(transfer_target) = waypoint.transfer_target {
                                        CommandRequest::new(CommandId::Transfer)
                                            .with_target(Some(transfer_target))
                                            .with_tx(Some(waypoint.x))
                                            .with_ty(Some(waypoint.y))
                                            .with_evaluated(true)
                                            .with_mode(CommandMode::SilentSub)
                                    } else {
                                        let (mut x, mut y) = (waypoint.x, waypoint.y);
                                        adjust_solid_offset(
                                            landscape,
                                            &mut x,
                                            &mut y,
                                            ctx.object.shape.width / 2,
                                            ctx.object.shape.height / 2,
                                        );
                                        CommandRequest::new(CommandId::MoveTo)
                                            .with_tx(Some(x))
                                            .with_ty(Some(y))
                                            .with_data(CommandData::Integer(self.data))
                                            .with_update_interval(25)
                                            .with_evaluated(true)
                                            .with_mode(CommandMode::SilentSub)
                                    };
                                operations.push(CommandOperation::PushFront(request));
                            }
                            return CommandStepResult::running(None).with_operations(operations);
                        }
                        Some(_) => return CommandStepResult::running(None),
                        None => {
                            self.path_checked = true;
                            return CommandStepResult::running(None);
                        }
                    }
                }
            }
        }
        if ctx.frame.is_multiple_of(35) {
            self.path_checked = false;
        }

        // Pushing grab-only or pushing not desired: let go
        // (C4Command.cpp:257-265) — UnGrab sub-command, and the command
        // re-evaluates because vehicle control might have blocked the
        // evaluation (:263).
        if ctx.object.action_procedure == ActionProcedure::Push {
            if let Some(action_target) = ctx.object.action_target {
                let grab_only = ctx
                    .resolve(action_target)
                    .and_then(|snapshot| ctx.definition(snapshot.definition_id.as_str()))
                    .is_some_and(|definition| definition.grab == 2);
                if grab_only || self.data & COMMAND_FLAG_MOVE_TO_PUSH_TARGET == 0 {
                    self.evaluated = false;
                    let request = CommandRequest::new(CommandId::UnGrab)
                        .with_update_interval(50)
                        .with_mode(CommandMode::SilentSub);
                    return CommandStepResult::running(None)
                        .with_operations(vec![CommandOperation::PushFront(request)]);
                }
            }
        }

        // Push/pull movers measure from the pushed vehicle only on the final
        // waypoint; intermediate MoveTos steer the clonk itself
        // (C4Command.cpp:218-220,271-277).
        let mut position = ctx.position;
        if !next_is_move_to
            && matches!(
                ctx.object.action_procedure,
                ActionProcedure::Push | ActionProcedure::Pull
            )
        {
            if let Some(vehicle) = ctx
                .object
                .action_target
                .and_then(|id| ctx.resolve_position(id))
            {
                position = vehicle;
            }
        }

        let target_range = if ctx.object.move_to_range > 0 {
            ctx.object.move_to_range
        } else {
            self.tolerance
        };

        // The four work procedures synchronously run ordinary
        // ObjectComStop before the target/idle/procedure checks. Keep the
        // pre-callback geometry here; the engine resumes this exact command
        // with a fresh live action snapshot after Idle/Walk callbacks.
        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Chop
                | ActionProcedure::Build
                | ActionProcedure::Dig
                | ActionProcedure::Bridge
        ) {
            self.stop_continuation = Some(MoveToStopContinuation {
                position,
                target,
                target_range,
                next_is_move_to,
            });
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopMoveTo {
                    object_id: ctx.object.id,
                },
            ]);
        }

        self.step_after_procedure(ctx, position, target, target_range, next_is_move_to, false)
    }

    fn resume_after_stop(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(continuation) = self.stop_continuation.take() else {
            return CommandStepResult::running(None);
        };
        self.step_after_procedure(
            ctx,
            continuation.position,
            continuation.target,
            continuation.target_range,
            continuation.next_is_move_to,
            true,
        )
    }

    /// MoveTo's post-ObjectComStop half. `force_steer` is required because
    /// the stop has just written ComDir=Stop; a same-direction value cached
    /// by this command must still be re-applied in this Execute.
    fn step_after_procedure(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        position: Vector2,
        target: Vector2,
        base_target_range: i32,
        next_is_move_to: bool,
        force_steer: bool,
    ) -> CommandStepResult {
        let dx = target.x - position.x;
        let dy = target.y - position.y;
        // Crew use their live post-stop shape width rather than the global
        // MoveToRange: `iTargetRange = Shape.Wdt / 5` (:286-292).
        let target_range = if ctx.object.ocf & ocf::CREW_MEMBER != 0 {
            ctx.object.shape.width / 5
        } else {
            base_target_range
        };
        let (range_factor_side, range_factor_top, range_factor_bottom) = if next_is_move_to
            && ctx.object.ocf & ocf::CREW_MEMBER != 0
            && ctx.object.action_procedure != ActionProcedure::Scale
        {
            (3, 3, 2)
        } else {
            (1, 1, 1)
        };
        let offset_x = position.x - target.x;
        let offset_y = position.y - target.y;
        if inside(
            offset_x,
            -range_factor_side * target_range,
            range_factor_side * target_range,
        ) && inside(
            offset_y,
            -range_factor_bottom * target_range,
            range_factor_top * target_range,
        ) {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            self.last_direction = CommandDirection::Stop;
            return CommandStepResult::completed(Some(update));
        }

        // Action.Act <= ActIdle is tested after arrival, so an idle object
        // already in range succeeds while an out-of-range one fails.
        if ctx.object.action_idle {
            return CommandStepResult::failed(None);
        }

        let float_steering = ctx.object.action_procedure == ActionProcedure::Float;
        let direction = match ctx.object.action_procedure {
            // DFA_WALK is horizontal-only. When x is already in range C++
            // does not assign ComDir at all, regardless of vertical error.
            ActionProcedure::Walk => {
                if dx > target_range {
                    Some(CommandDirection::Right)
                } else if dx < -target_range {
                    Some(CommandDirection::Left)
                } else {
                    None
                }
            }
            // DFA_SWIM (C4Command.cpp:370-382): Tick2 frames (Game.iTick2
            // != 0 — odd FrameCounter) steer horizontally with the target
            // range; !Tick2 frames steer vertically toward Ty with no
            // range. ComDir is left alone when no condition hits.
            ActionProcedure::Swim => {
                if !ctx.frame.is_multiple_of(2) {
                    if dx > target_range {
                        Some(CommandDirection::Right)
                    } else if dx < -target_range {
                        Some(CommandDirection::Left)
                    } else {
                        None
                    }
                } else if dy > 0 {
                    Some(CommandDirection::Down)
                } else if dy < 0 {
                    Some(CommandDirection::Up)
                } else {
                    None
                }
            }
            // DFA_SCALE (C4Command.cpp:335-338): vertical steering only —
            // cy > Ty + range climbs Up, cy < Ty - range slides Down.
            ActionProcedure::Scale => {
                if dy < -target_range {
                    Some(CommandDirection::Up)
                } else if dy > target_range {
                    Some(CommandDirection::Down)
                } else {
                    None
                }
            }
            // DFA_FLIGHT (C4Command.cpp:414-417): no ComDir steering —
            // only FlightControl runs (below).
            ActionProcedure::Flight => None,
            // DFA_PUSH/DFA_PULL (C4Command.cpp:329-333): horizontal
            // steering only, measured from the vehicle position above.
            ActionProcedure::Push | ActionProcedure::Pull => {
                if dx > target_range {
                    Some(CommandDirection::Right)
                } else if dx < -target_range {
                    Some(CommandDirection::Left)
                } else {
                    None
                }
            }
            // DFA_HANGLE (C4Command.cpp:384-387): horizontal steering
            // only; the angle-based drop follows below.
            ActionProcedure::Hang => {
                if dx > target_range {
                    Some(CommandDirection::Right)
                } else if dx < -target_range {
                    Some(CommandDirection::Left)
                } else {
                    None
                }
            }
            // DFA_FLOAT (C4Command.cpp:393-410): normalize the fixed-point
            // target vector to Physical.Float, subtract current momentum,
            // then choose the closest of the eight control directions.
            ActionProcedure::Float => {
                let fixed_dx = math::itofix(target.x) - ctx.object.fixed_position.x;
                let fixed_dy = math::itofix(target.y) - ctx.object.fixed_position.y;
                if ctx.object.physical_deferred {
                    self.physical_continuation =
                        Some(MoveToPhysicalContinuation::Float { fixed_dx, fixed_dy });
                    return resolve_command_physical(ctx.object.id, 1, None);
                }
                Some(Self::float_control_direction(
                    ctx,
                    ctx.object.physical,
                    fixed_dx,
                    fixed_dy,
                ))
            }
            // C++ has no default procedure arm: NONE and every other
            // unmatched procedure leave ComDir untouched.
            _ => None,
        };

        // The C++ Float arm writes ComDir every execution. In particular,
        // COMD_None must stop momentum correction even when this new command
        // has not observed the object's pre-existing ComDir.
        let steer = direction.filter(|direction| {
            force_steer || float_steering || *direction != ctx.object.command_direction
        });
        if let Some(direction) = direction {
            self.last_direction = direction;
        }

        // DFA_SCALE let-go control (C4Command.cpp:339-368): jump off the
        // wall toward the target or on wall contact; the C++ `return`
        // ends this Execute with the command still pending.
        if ctx.object.action_procedure == ActionProcedure::Scale {
            if let Some(xdirf) = self.scale_let_go(ctx, position, target) {
                return CommandStepResult::running(Some(let_go_update(steer, xdirf)));
            }
        }

        // DFA_HANGLE let-go control (C4Command.cpp:388-390): drop off the
        // ceiling once the target angle leaves the hangling sector.
        if ctx.object.action_procedure == ActionProcedure::Hang
            && c4_angle(position.x, position.y, target.x, target.y).abs() > LET_GO_HANGLE_ANGLE
        {
            return CommandStepResult::running(Some(let_go_update(steer, 0)));
        }

        // DFA_WALK movement controls, after the ComDir steering
        // (C4Command::Execute MoveTo, C4Command.cpp:316-326):
        // FlightControl never short-circuits (it returns false even after
        // taking off, :1816-1849); JumpControl returning true ends the
        // Execute for this tick. DFA_FLIGHT runs FlightControl alone
        // (:414-417).
        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Walk | ActionProcedure::Flight
        ) && ctx.object.physical_deferred
        {
            self.physical_continuation = Some(MoveToPhysicalContinuation::FlightControl {
                target,
                from_walk: ctx.object.action_procedure == ActionProcedure::Walk,
            });
            let update =
                steer.map(|direction| ObjectUpdate::new().with_command_direction(direction));
            return resolve_command_physical(ctx.object.id, 1, update);
        }
        let from_walk = ctx.object.action_procedure == ActionProcedure::Walk;
        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Walk | ActionProcedure::Flight
        ) && self.flight_control_takes_off(ctx, target, ctx.object.physical)
        {
            self.flight_continuation = Some(MoveToFlightContinuation {
                target,
                jump_after_takeoff: from_walk,
            });
            let update =
                steer.map(|direction| ObjectUpdate::new().with_command_direction(direction));
            return CommandStepResult::running(update).with_events(vec![
                CommandEvent::MoveToFlightControlTakeoff {
                    object_id: ctx.object.id,
                    command_instance_id: 0,
                },
            ]);
        }

        let jump_operations = from_walk.then(|| self.jump_control(ctx, target)).flatten();
        if jump_operations.is_some() {
            let mut update = ObjectUpdate::new();
            if let Some(direction) = steer {
                update = update.with_command_direction(direction);
            }
            let mut result = CommandStepResult::running(Some(update));
            if let Some(operations) = jump_operations {
                result = result.with_operations(operations);
            }
            return result;
        }

        match steer {
            None => CommandStepResult::running(None),
            Some(direction) => {
                let update = ObjectUpdate::new().with_command_direction(direction);
                CommandStepResult::running(Some(update))
            }
        }
    }

    fn float_control_direction(
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
        mut fixed_dx: crate::C4Fixed,
        mut fixed_dy: crate::C4Fixed,
    ) -> CommandDirection {
        let scale = math::fixed100(physical.float) / fixed_dx.abs().max(fixed_dy.abs());
        fixed_dx *= scale;
        fixed_dy *= scale;
        fixed_dx -= ctx.object.fixed_velocity.x;
        fixed_dy -= ctx.object.fixed_velocity.y;
        if fixed_dx.abs() + fixed_dy.abs() < math::fixed100(20) {
            CommandDirection::Stop
        } else if fixed_dy.abs() * 3 < fixed_dx {
            CommandDirection::Right
        } else if fixed_dy.abs() * 3 < -fixed_dx {
            CommandDirection::Left
        } else if fixed_dx.abs() * 3 < fixed_dy {
            CommandDirection::Down
        } else if fixed_dx.abs() * 3 < -fixed_dy {
            CommandDirection::Up
        } else if fixed_dx > crate::C4Fixed::ZERO && fixed_dy > crate::C4Fixed::ZERO {
            CommandDirection::DownRight
        } else if fixed_dx < crate::C4Fixed::ZERO && fixed_dy > crate::C4Fixed::ZERO {
            CommandDirection::DownLeft
        } else if fixed_dx > crate::C4Fixed::ZERO && fixed_dy < crate::C4Fixed::ZERO {
            CommandDirection::UpRight
        } else {
            CommandDirection::UpLeft
        }
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        let Some(continuation) = self.physical_continuation.take() else {
            return CommandStepResult::running(None);
        };
        match continuation {
            MoveToPhysicalContinuation::InitEvaluation { x, y } => {
                let (mut x, mut y) = (x, y);
                if let Some(landscape) = ctx.landscape {
                    adjust_move_to_target(
                        landscape,
                        &mut x,
                        &mut y,
                        physical.can_fly != 0,
                        ctx.object.shape.height,
                    );
                }
                self.tx = Some(x);
                self.ty = Some(y);
                CommandStepResult::running(None)
            }
            MoveToPhysicalContinuation::Float { fixed_dx, fixed_dy } => {
                let direction = Self::float_control_direction(ctx, physical, fixed_dx, fixed_dy);
                self.last_direction = direction;
                CommandStepResult::running(Some(
                    ObjectUpdate::new().with_command_direction(direction),
                ))
            }
            MoveToPhysicalContinuation::FlightControl { target, from_walk } => {
                if self.flight_control_takes_off(ctx, target, physical) {
                    self.flight_continuation = Some(MoveToFlightContinuation {
                        target,
                        jump_after_takeoff: from_walk,
                    });
                    return CommandStepResult::running(None).with_events(vec![
                        CommandEvent::MoveToFlightControlTakeoff {
                            object_id: ctx.object.id,
                            command_instance_id: 0,
                        },
                    ]);
                }
                let jump_operations = from_walk.then(|| self.jump_control(ctx, target)).flatten();
                CommandStepResult::running(None)
                    .with_operations(jump_operations.unwrap_or_default())
            }
        }
    }

    fn resume_after_flight_control(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
    ) -> CommandStepResult {
        let Some(continuation) = self.flight_continuation.take() else {
            return CommandStepResult::running(None);
        };
        let operations = continuation
            .jump_after_takeoff
            .then(|| self.jump_control(ctx, continuation.target))
            .flatten()
            .unwrap_or_default();
        CommandStepResult::running(None).with_operations(operations)
    }

    /// The DFA_SCALE let-go decision (C4Command.cpp:339-368): jump away
    /// from the wall (xdir sign opposite the scaling side) when the
    /// target lies off the wall beyond LetGoRange1 within LetGoRange2
    /// vertically, or on any contact once the action is 3+ frames old.
    fn scale_let_go(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        position: Vector2,
        target: Vector2,
    ) -> Option<i32> {
        let (cx, cy) = (position.x, position.y);
        let contact_let_go = ctx.object.action_time > 2 && ctx.object.contact != 0;
        match ctx.object.direction {
            Direction::Left => (target.x > cx + LET_GO_RANGE1
                && inside(cy - target.y, -LET_GO_RANGE2, LET_GO_RANGE2)
                || contact_let_go)
                .then_some(1),
            Direction::Right => (target.x < cx - LET_GO_RANGE1
                && inside(cy - target.y, -LET_GO_RANGE2, LET_GO_RANGE2)
                || contact_let_go)
                .then_some(-1),
            _ => None,
        }
    }

    /// `C4Command::FlightControl` (C4Command.cpp:1816-1849): CanFly crew or
    /// Pathfinder definitions walking toward a distant target within ±60°
    /// of straight up take off unless the current ActMap entry is Disabled.
    /// This predicate reports whether native calls SetActionByName("Fly");
    /// FlightControl itself always returns false, so WALK later resumes
    /// JumpControl after that callbackful transition.
    fn flight_control_takes_off(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        target: Vector2,
        physical: PhysicalInfo,
    ) -> bool {
        if physical.can_fly == 0 {
            return false;
        }
        if ctx.object.ocf & crate::ocf::CREW_MEMBER == 0 && ctx.object.pathfinder == 0 {
            return false;
        }
        if ctx.object.action_disabled {
            return false;
        }
        let Some(landscape) = ctx.landscape else {
            return false;
        };
        let (cx, cy) = (ctx.position.x, ctx.position.y);
        let mut angle = c4_angle(cx, cy, target.x, target.y);
        while angle > 180 {
            angle -= 360;
        }
        if !inside(angle, -FLIGHT_ANGLE_RANGE, FLIGHT_ANGLE_RANGE) {
            return false;
        }
        if c4_distance(cx, cy, target.x, target.y) <= 30 {
            return false;
        }
        let mut top_free = 0;
        while top_free < 50 && !landscape.is_solid_at(cx, cy + ctx.object.shape_top - top_free) {
            top_free += 1;
        }
        if top_free < 15 {
            return false;
        }
        true
    }

    /// `C4Command::JumpControl` (C4Command.cpp:1851-1920): the three
    /// walking-jump triggers for crew or Pathfinder definitions.
    fn jump_control(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        target: Vector2,
    ) -> Option<Vec<CommandOperation>> {
        if ctx.object.ocf & crate::ocf::CREW_MEMBER == 0 && ctx.object.pathfinder == 0 {
            return None;
        }
        let landscape = ctx.landscape?;
        let (cx, cy) = (ctx.position.x, ctx.position.y);
        let (tx, ty) = (target.x, target.y);
        let mut angle = c4_angle(cx, cy, tx, ty);
        while angle > 180 {
            angle -= 360;
        }

        let jump_request = || {
            CommandOperation::PushFront(
                CommandRequest::new(CommandId::Jump)
                    .with_tx(Some(tx))
                    .with_ty(Some(ty)),
            )
        };

        // Diagonal free jump (:1861-1872).
        if (inside(angle - JUMP_ANGLE, -JUMP_ANGLE_RANGE, JUMP_ANGLE_RANGE)
            || inside(angle + JUMP_ANGLE, -JUMP_ANGLE_RANGE, JUMP_ANGLE_RANGE))
            && landscape.path_is_clear(Vector2::new(cx, cy), Vector2::new(tx, ty))
            && c4_distance(cx, cy, tx, ty) > 30
        {
            let mut top_free = 0;
            while top_free < 50 && !landscape.is_solid_at(cx, cy + ctx.object.shape_top - top_free)
            {
                top_free += 1;
            }
            if top_free >= 15 {
                return Some(vec![jump_request()]);
            }
        }

        // High-angle side move + jump (:1874-1893).
        if inside(
            angle - JUMP_HIGH_ANGLE,
            -3 * JUMP_ANGLE_RANGE,
            3 * JUMP_ANGLE_RANGE,
        ) && inside(cy - ty, 10, 40)
        {
            let side = solid_on_which_side(landscape, tx, ty);
            let dist = 5 * (cy - ty).abs() / 6;
            let mut side_x = cx - dist * side;
            let mut side_y = cy;
            adjust_move_to_target(landscape, &mut side_x, &mut side_y, false, 0);
            if inside(side_y - cy, -20, 20)
                && landscape.path_is_clear(Vector2::new(side_x, side_y), Vector2::new(tx, ty))
            {
                return Some(vec![
                    jump_request(),
                    CommandOperation::PushFront(
                        CommandRequest::new(CommandId::MoveTo)
                            .with_tx(Some(side_x))
                            .with_ty(Some(side_y))
                            .with_update_interval(50),
                    ),
                ]);
            }
        }

        // Low side contact jump (:1896-1908).
        let low_range = 5;
        if ctx.object.contact & crate::CNAT_RIGHT != 0
            && inside(
                angle - JUMP_LOW_ANGLE,
                -low_range * JUMP_ANGLE_RANGE,
                low_range * JUMP_ANGLE_RANGE,
            )
        {
            return Some(vec![jump_request()]);
        }
        if ctx.object.contact & crate::CNAT_LEFT != 0
            && inside(
                angle + JUMP_LOW_ANGLE,
                -low_range * JUMP_ANGLE_RANGE,
                low_range * JUMP_ANGLE_RANGE,
            )
        {
            return Some(vec![jump_request()]);
        }

        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EnterState {
    target: Option<ObjectId>,
    push_target: bool,
    update_interval: u32,
}

impl EnterState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let push_target = matches!(
            request.data,
            CommandData::Integer(flags) if flags & COMMAND_FLAG_ENTER_PUSH_TARGET != 0
        );
        Ok(Self {
            target: request.target,
            push_target,
            update_interval: positive_helper_interval_or_one(request.update_interval),
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(target) = self.target else {
            return CommandStepResult::failed(None);
        };
        let Some(target_snapshot) = ctx.resolve(target) else {
            return CommandStepResult::failed(None);
        };

        if ctx.object.no_push_enter != 0 {
            return CommandStepResult::failed(None);
        }

        if ctx.object.container == Some(target) {
            return CommandStepResult::completed(None);
        }

        let pushed_target = (ctx.object.action_procedure == ActionProcedure::Push)
            .then_some(ctx.object.action_target)
            .flatten();
        if let Some(pushed_id) = pushed_target {
            let grab_only = ctx
                .resolve(pushed_id)
                .and_then(|snapshot| ctx.definition(snapshot.definition_id.as_str()))
                .is_some_and(|definition| definition.grab == 2);
            if grab_only || !self.push_target || pushed_id == target {
                let ungrab = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
                return CommandStepResult::running(None)
                    .with_operations(vec![CommandOperation::PushFront(ungrab)]);
            }
        }

        let position = pushed_target
            .and_then(|id| ctx.resolve_position(id))
            .unwrap_or(ctx.position);
        // Target->At(cx, cy, OCF_Entrance) first tests the target shape and
        // then narrows the returned OCF to Def->Entrance. The actor itself
        // must be outside every container; At likewise rejects a contained
        // target (C4Command.cpp:586-588; C4Object.cpp:1133-1155).
        let entrance_area = (target_snapshot.ocf & ocf::ENTRANCE != 0)
            .then_some(target_snapshot.entrance)
            .flatten();
        let in_entrance_range = target_snapshot.has_nonzero_status()
            && ctx.object.container.is_none()
            && target_snapshot.container.is_none()
            && target_snapshot.at_point(position.x, position.y)
            && entrance_area
                .is_some_and(|entrance| entrance.contains_point(position.x, position.y));
        if in_entrance_range {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            if let Some(pushed_id) = pushed_target {
                let event = CommandEvent::SetObjectCommand {
                    object_id: pushed_id,
                    controller: None,
                    request: CommandRequest::new(CommandId::Enter)
                        .with_target(Some(target))
                        .with_mode(CommandMode::Base),
                };
                return CommandStepResult::completed(Some(update)).with_events(vec![event]);
            }
            if !target_snapshot.entrance_status {
                let event = CommandEvent::ActivateEntrance {
                    object_id: target,
                    caller: ctx.object.id,
                    on_result: None,
                    command_instance_id: 0,
                };
                return CommandStepResult::running(Some(update)).with_events(vec![event]);
            }
            let event = CommandEvent::EnterObject {
                object_id: ctx.object.id,
                container_id: target,
            };
            return CommandStepResult::completed(Some(update)).with_events(vec![event]);
        }

        // GetEntranceArea is a method-level Status gate independent of the
        // raw Target pointer. A detached command may still hold Target after
        // Status reaches zero, but C++ leaves Enter pending without adding a
        // MoveTo in that case (C4Object.cpp:2074-2093).
        if !target_snapshot.has_nonzero_status() {
            return CommandStepResult::running(None);
        }

        let mut result = CommandStepResult::running(None);
        // Move to the entrance with the push flag carried through:
        // (Data & C4CMD_Enter_PushTarget) ? C4CMD_MoveTo_PushTarget
        // : 0 (C4Command.cpp:615).
        // GetEntranceArea returns the Def->Entrance rectangle when the OCF
        // is present, otherwise a zero-sized area at the object center. The
        // child has no Target: its explicit coordinates are fixed until
        // Enter reissues the command (C4Object.cpp:2074-2093).
        let destination = entrance_area
            .map(|entrance| {
                Vector2::new(
                    entrance.x.saturating_add(entrance.width / 2),
                    entrance.y.saturating_add(entrance.height / 2),
                )
            })
            .unwrap_or(target_snapshot.position);
        let mut request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(destination.x))
            .with_ty(Some(destination.y))
            .with_update_interval(50);
        if self.push_target {
            request = request.with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET));
        }
        result = result.with_operations(vec![CommandOperation::PushFront(request)]);
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ExitState {
    update_interval: u32,
    /// C4Command::Evaluated. An ordinary Exit spends its first Execute in
    /// InitEvaluation, where it cancels DFA_ATTACH and returns before Exit.
    #[serde(default)]
    evaluated: bool,
    #[serde(default, skip_serializing_if = "crate::u32_is_zero")]
    activation_pending: u32,
    /// ObjectComStop is a synchronous callback boundary for DFA_BUILD.
    /// Resume the same native Exit body afterward without spending another
    /// command Execute/update interval.
    #[serde(default)]
    stop_continuation: bool,
}

impl ExitState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            update_interval: positive_helper_interval_or_one(request.update_interval),
            evaluated: request.evaluated,
            activation_pending: 0,
            stop_continuation: false,
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        // InitEvaluation consumes this Execute even when there is no attach
        // action to cancel (C4Command.cpp:1554-1555,1654-1657).
        if !self.evaluated {
            self.evaluated = true;
            let events = (ctx.object.action_procedure == ActionProcedure::Attach)
                .then(|| CommandEvent::ApplyObjectUpdate {
                    object_id: ctx.object.id,
                    // ObjectComCancelAttach uses ordinary SetAction(ActIdle),
                    // including its synchronous AbortCall (:769-773).
                    update: ObjectUpdate::new().with_action_update(
                        ActionUpdate::default().with_name("Idle").with_force(false),
                    ),
                })
                .into_iter()
                .collect();
            return CommandStepResult::running(None).with_events(events);
        }
        if ctx.object.container.is_none() {
            return CommandStepResult::completed(None);
        }
        if ctx.object.action_procedure == ActionProcedure::Build && !self.stop_continuation {
            self.stop_continuation = true;
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopExit {
                    object_id: ctx.object.id,
                    command_instance_id: 0,
                },
            ]);
        }
        self.step_after_stop(ctx)
    }

    fn resume_after_stop(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        self.stop_continuation = false;
        self.step_after_stop(ctx)
    }

    fn step_after_stop(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(container_id) = ctx.object.container else {
            return CommandStepResult::completed(None);
        };

        let container_snapshot = match ctx.resolve(container_id) {
            // C4Command::Exit only needs the live Contained pointer;
            // structures are nonliving but valid containers.
            Some(snapshot) => snapshot,
            None => return CommandStepResult::completed(None),
        };

        // A closed entrance is not an ejection point. C++ asks the
        // container to open and leaves this Exit command pending; a false
        // ActivateEntrance result fails it (C4Command.cpp:644-650).
        if !container_snapshot.entrance_status {
            self.activation_pending = self.activation_pending.saturating_add(1);
            let event = CommandEvent::ActivateEntrance {
                object_id: container_id,
                caller: ctx.object.id,
                on_result: Some(CallResultAction::ResolveExitActivation),
                command_instance_id: 0,
            };
            return CommandStepResult::running(None).with_events(vec![event]);
        }

        let retained_parent = container_snapshot
            .container
            .filter(|parent_id| ctx.resolve(*parent_id).is_some());
        if let Some(parent_id) = retained_parent {
            // Native calls C4Object::Enter and ignores its boolean before
            // finishing Exit. Keep this as one live event: RejectEntrance,
            // the old-container Exit callbacks, the target Status gate and
            // Collection2/Entrance may all change the final containment
            // (C4Command.cpp:629-632; C4Object.cpp:1566-1636).
            CommandStepResult::running(None).with_events(vec![
                CommandEvent::CommandExitIntoParent {
                    object_id: ctx.object.id,
                    container_id: parent_id,
                    command_instance_id: 0,
                },
            ])
        } else {
            let entrance_position = if container_snapshot.entrance_status
                && container_snapshot.ocf & ocf::ENTRANCE != 0
                && container_snapshot.has_nonzero_status()
            {
                container_snapshot.entrance.map(|entrance| {
                    Vector2::new(
                        entrance.x.saturating_add(entrance.width / 2),
                        entrance
                            .y
                            .saturating_add(entrance.height)
                            .saturating_add(ctx.object.shape_top)
                            .saturating_sub(1),
                    )
                })
            } else {
                None
            };
            let (position, jump_after) = if let Some(position) = entrance_position {
                (position, false)
            } else if let Some(collection) = ctx
                .definition(container_snapshot.definition_id.as_str())
                .and_then(|definition| definition.collection_rect)
                .filter(|collection| ctx.object.collectible && collection.width != 0)
            {
                // Def->Collection is container-local. C++ deliberately uses
                // the container x (not Collection.x/center) and one pixel
                // above its top before invoking ObjectComJump (:643-649).
                let position = Vector2::new(
                    container_snapshot.position.x,
                    container_snapshot
                        .position
                        .y
                        .saturating_add(collection.y)
                        .saturating_sub(1),
                );
                (position, true)
            } else {
                // Plain C4CMD_Exit keeps the contained object's own x/y;
                // those can differ from the container after Enter with
                // fCopyMotion=false or direct repositioning.
                (ctx.position, false)
            };
            CommandStepResult::running(None).with_events(vec![CommandEvent::CommandExitObject {
                object_id: ctx.object.id,
                previous_container: container_id,
                position,
                jump_after,
                command_instance_id: 0,
            }])
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BuildState {
    target: ObjectId,
    site: Option<Vector2>,
    approach_horizontal: i32,
    approach_vertical: i32,
    /// Same-Execute continuation staged while Dig runs live ObjectComStop.
    #[serde(default)]
    stop_continuation: bool,
    #[serde(default)]
    physical_pending: bool,
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
            approach_horizontal: 9,
            approach_vertical: 20,
            stop_continuation: false,
            physical_pending: false,
        })
    }

    fn target_position(&self, ctx: &CommandRuntimeContext<'_>) -> Option<Vector2> {
        // C4Command::Build approaches Target itself; unlike Construct, its
        // Tx/Ty fields are not a construction-site override
        // (C4Command.cpp:823-899).
        ctx.resolve_position(self.target)
    }

    fn object_has_command(
        object: &CommandObjectSnapshot,
        command: CommandId,
        target: ObjectId,
    ) -> bool {
        object
            .commands
            .iter()
            .any(|entry| entry.name == command.to_name() && entry.target == Some(target))
    }

    fn object_contains_linekit(
        object: &CommandObjectSnapshot,
        ctx: &CommandRuntimeContext<'_>,
    ) -> bool {
        object.contents.iter().any(|id| {
            ctx.resolve(*id).is_some_and(|item| {
                item.has_nonzero_status()
                    && item.ocf != 0
                    && item.definition_id == LINEKIT_DEFINITION
            })
        })
    }

    fn should_queue_energy(&self, ctx: &CommandRuntimeContext<'_>) -> bool {
        if ctx
            .objects
            .values()
            .filter(|object| !object.destroyed && object.status.is_active())
            .any(|object| Self::object_has_command(object, CommandId::Energy, self.target))
        {
            return false;
        }
        if Self::object_contains_linekit(ctx.object, ctx) {
            return true;
        }
        !ctx.objects
            .values()
            .filter(|object| !object.destroyed && object.status.is_active())
            .any(|object| {
                Self::object_has_command(object, CommandId::Build, self.target)
                    && Self::object_contains_linekit(object, ctx)
            })
    }

    fn start_build(&self, ctx: &CommandRuntimeContext<'_>, stop_first: bool) -> CommandStepResult {
        CommandStepResult::running(None).with_events(vec![CommandEvent::ObjectComBuild {
            object_id: ctx.object.id,
            target_id: self.target,
            stop_first,
        }])
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if ctx.resolve(self.target).is_none() {
            return CommandStepResult::failed(None);
        }
        if ctx.object.physical_deferred {
            self.physical_pending = true;
            return resolve_command_physical(ctx.object.id, 2, None);
        }
        self.step_after_physical(ctx, ctx.object.physical)
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        if !std::mem::take(&mut self.physical_pending) {
            return CommandStepResult::running(None);
        }
        self.step_after_physical(ctx, physical)
    }

    fn step_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        let builder = ctx.object;
        let Some(target_snapshot) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(None);
        };

        // C4Object::GetPhysical always resolves a physical set for an extant
        // object. Only exact zero loses construction ability; negative raw
        // values remain truthy in C++ (C4Command.cpp:831-836).
        if physical.can_construct == 0 {
            return CommandStepResult::failed(None)
                .with_failure_reason(CommandFailureReason::CannotBuild);
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
                && self.should_queue_energy(ctx)
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
            return CommandStepResult::running(None);
        }

        if builder.action_procedure == ActionProcedure::Push {
            let request = CommandRequest::new(CommandId::UnGrab)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        if builder.action_procedure == ActionProcedure::Dig {
            self.stop_continuation = true;
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopBuild {
                    object_id: builder.id,
                    command_instance_id: 0,
                },
            ]);
        }

        self.step_after_dig(ctx)
    }

    fn resume_after_stop(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if !std::mem::take(&mut self.stop_continuation) {
            return CommandStepResult::running(None);
        }
        self.step_after_dig(ctx)
    }

    fn step_after_dig(&self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let builder = ctx.object;
        let Some(target_snapshot) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(None);
        };

        // Structures and StaticBack objects use only the legacy internal
        // target path, regardless of proximity or shared outer container.
        if builder.category & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK) != 0 {
            return if target_snapshot.container == Some(builder.id) {
                self.start_build(ctx, false)
            } else {
                CommandStepResult::failed(None)
            };
        }

        let target_position = match self.target_position(ctx) {
            Some(position) => position,
            None => {
                return CommandStepResult::failed(None);
            }
        };

        let same_container =
            target_snapshot.container.is_some() && builder.container == target_snapshot.container;
        let at_target = target_snapshot.container.is_none()
            && target_snapshot.has_nonzero_status()
            && !target_snapshot.definition_id.is_empty()
            && target_snapshot.ocf != 0
            && target_snapshot.at_point(ctx.position.x, ctx.position.y)
            && builder.action_procedure == ActionProcedure::Walk;

        if same_container || at_target {
            return self.start_build(ctx, true);
        }

        let request = target_snapshot.container.map_or_else(
            || {
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(target_position.x))
                    .with_ty(Some(target_position.y))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub)
            },
            |container| {
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(container))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub)
            },
        );
        CommandStepResult::running(None).with_operations(vec![CommandOperation::PushFront(request)])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ConstructState {
    target: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target2: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    definition_id: Option<DefinitionId>,
    site: Option<Vector2>,
    update_interval: u32,
    spawn_requested: bool,
    construction_id: Option<ObjectId>,
    #[serde(default)]
    script_pending: bool,
    #[serde(default)]
    script_invoked: bool,
    #[serde(default)]
    script_result: Option<AcquireScriptResult>,
    #[serde(default)]
    physical_pending: bool,
    #[serde(default)]
    stop_continuation: bool,
    /// Native iMoveToRange local captured before stop/script callbacks in
    /// the current Execute. A new ordinary Execute overwrites it.
    #[serde(skip)]
    execute_move_to_range: Option<i32>,
}

/// A `ConstructionCheck` reject with the branch C++ reports through
/// `GameMsgObject(..., pByObj, FRed)` when a caller object is present
/// (C4Landscape.cpp:2131-2163). `Blocked` retains the overlapping object so
/// message emission can resolve its live display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstructionCheckFailure {
    /// `!ndef->Constructable` -> IDS_OBJ_NOCON.
    NotConstructable,
    /// Too much solid inside the body rect -> IDS_OBJ_NOROOM.
    NoRoom,
    /// Missing solid support strip below -> IDS_OBJ_NOLEVEL.
    NoLevel,
    /// `Game.OverlapObject` hit -> IDS_OBJ_NOOTHER with the blocker's name.
    Blocked(ObjectId),
}

/// Side-effect-free core of `ConstructionCheck` (C4Landscape.cpp:2125-2169).
///
/// Command execution and local construction-drag previews must use the same
/// terrain/support/object-overlap predicate. The caller supplies the overlap
/// lookup because command execution reads its frozen command snapshots while
/// the public preview API reads the live object list. `None` means the site
/// is legal; callers with a C++ `pByObj` turn the failure into the localized
/// red feedback message.
pub(crate) fn construction_check<F>(
    constructable: bool,
    shape: Option<DefinitionRect>,
    construction_offset: i32,
    category: i32,
    site: Vector2,
    landscape: Option<&crate::Landscape>,
    overlaps: F,
) -> Option<ConstructionCheckFailure>
where
    F: FnOnce(i32, i32, i32, i32, i32) -> Option<ObjectId>,
{
    if !constructable {
        return Some(ConstructionCheckFailure::NotConstructable);
    }

    let (width, height) = shape
        .map(|shape| (shape.width, shape.height))
        .unwrap_or((0, 0));
    let effective_height = height - construction_offset;
    let left = site.x - width / 2;
    let top = site.y - effective_height;

    let Some(landscape) = landscape else {
        return None;
    };

    let solid_count = (top..site.y)
        .flat_map(|y| (left..left + width).map(move |x| (x, y)))
        .filter(|&(x, y)| landscape.is_solid_at(x, y))
        .count()
        .min(i32::MAX as usize) as i32;
    if solid_count > width * effective_height / 20 {
        return Some(ConstructionCheckFailure::NoRoom);
    }
    let support_count = (site.y..site.y + 5)
        .flat_map(|y| (left..left + width).map(move |x| (x, y)))
        .filter(|&(x, y)| landscape.is_solid_at(x, y))
        .count()
        .min(i32::MAX as usize) as i32;
    if support_count < width * 2 {
        return Some(ConstructionCheckFailure::NoLevel);
    }

    overlaps(left, top, width, effective_height, category).map(ConstructionCheckFailure::Blocked)
}

impl ConstructState {
    fn from_request(request: &CommandRequest) -> Self {
        let definition_id = command_data_to_definition_id(&request.data);
        let site = Some(Vector2::new(
            request.tx.unwrap_or(0),
            request.ty.unwrap_or(0),
        ));
        Self {
            target: request.target,
            target2: request.target2,
            definition_id,
            site,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            spawn_requested: false,
            construction_id: None,
            script_pending: false,
            script_invoked: false,
            script_result: None,
            physical_pending: false,
            stop_continuation: false,
            execute_move_to_range: None,
        }
    }

    fn builder_has_conkit(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        ctx.object.contents.iter().copied().find(|id| {
            ctx.resolve(*id)
                .map(|snapshot| {
                    snapshot.definition_id == CONKIT_DEFINITION && snapshot.has_nonzero_status()
                })
                .unwrap_or(false)
        })
    }

    fn at_site(&self, ctx: &CommandRuntimeContext<'_>, site: Vector2) -> bool {
        const APPROACH_VERTICAL: i32 = 20;
        let approach_horizontal = self.execute_move_to_range.unwrap_or({
            if ctx.object.move_to_range > 0 {
                ctx.object.move_to_range
            } else {
                5
            }
        });
        let dx = site.x - ctx.position.x;
        let dy = site.y - ctx.position.y;
        dx.abs() <= approach_horizontal && dy.abs() <= APPROACH_VERTICAL
    }

    fn find_command(object: &CommandObjectSnapshot, command: CommandId) -> Option<&CommandView> {
        object
            .commands
            .iter()
            .find(|entry| entry.name == command.to_name())
    }

    fn overlaps_construction_rect(
        ctx: &CommandRuntimeContext<'_>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        category: i32,
    ) -> bool {
        Self::overlapping_construction_object(ctx, x, y, width, height, category).is_some()
    }

    fn overlapping_construction_object(
        ctx: &CommandRuntimeContext<'_>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        category: i32,
    ) -> Option<ObjectId> {
        ctx.objects
            .values()
            .filter(|object| {
                let shape = object.raw_shape_rect();
                object.is_status_active()
                    && object.container.is_none()
                    && object.category & category & CATEGORY_SORT_LIMIT != 0
                    && x < shape.x + shape.width
                    && shape.x < x + width
                    && y < shape.y + shape.height
                    && shape.y < y + height
            })
            // Game.OverlapObject walks the sector object lists in master
            // order; the frozen snapshots preserve that order key.
            .min_by_key(|object| (object.master_list_order, object.id))
            .map(|object| object.id)
    }

    fn find_construction_site(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        definition: &CommandDefinitionSnapshot,
    ) -> Option<Vector2> {
        let landscape = ctx.landscape?;
        let (width, height) = definition
            .shape
            .map(|shape| (shape.width, shape.height))
            .unwrap_or((0, 0));
        landscape
            .find_con_site_spot(
                ctx.position.x,
                ctx.position.y,
                width,
                height,
                20,
                |x, y, width, height| {
                    Self::overlaps_construction_rect(ctx, x, y, width, height, definition.category)
                },
            )
            .map(|(x, y)| Vector2::new(x, y))
    }

    fn construction_check(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        definition: &CommandDefinitionSnapshot,
        site: Vector2,
    ) -> Option<ConstructionCheckFailure> {
        construction_check(
            definition.constructable,
            definition.shape,
            definition.construction_offset,
            definition.category,
            site,
            ctx.landscape,
            |left, top, width, height, category| {
                Self::overlapping_construction_object(ctx, left, top, width, height, category)
            },
        )
    }

    fn find_spawned_construction(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        definition_id: &str,
        site: Vector2,
    ) -> Option<ObjectId> {
        ctx.objects
            .values()
            .filter(|snapshot| {
                snapshot.id != ctx.object.id
                    && snapshot.is_status_active()
                    && snapshot.definition_id == definition_id
                    && snapshot.owner == ctx.object.owner
                    && snapshot.construction < FULL_CON
                    && snapshot.container.is_none()
            })
            .filter(|snapshot| {
                let dx = (snapshot.position.x - site.x).abs();
                let dy = (snapshot.position.y - site.y).abs();
                dx <= 4 && dy <= 4
            })
            // CreateObjectConstruction inserts the newborn before the
            // existing same-definition cluster in C++ Game.Objects. The
            // asynchronous Rust recovery therefore takes the first matching
            // master-list entry, never HashMap iteration order.
            .min_by_key(|snapshot| (snapshot.master_list_order, snapshot.id))
            .map(|snapshot| snapshot.id)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if ctx.object.physical_deferred {
            self.physical_pending = true;
            return resolve_command_physical(ctx.object.id, 2, None);
        }
        self.step_after_physical(ctx, ctx.object.physical)
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        if !std::mem::take(&mut self.physical_pending) {
            return CommandStepResult::running(None);
        }
        self.step_after_physical(ctx, physical)
    }

    fn step_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        // C4Command::Construct applies the physical capability gate before
        // both the menu-opening Data=0 path and definition validation.
        if physical.can_construct == 0 {
            return CommandStepResult::failed(None);
        }

        let Some(definition_id) = self.definition_id.clone() else {
            return CommandStepResult::completed(None).with_events(vec![CommandEvent::OpenMenu(
                MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind: MenuRequestKind::Construction,
                },
            )]);
        };
        self.execute_move_to_range = Some(if ctx.object.move_to_range > 0 {
            ctx.object.move_to_range
        } else {
            5
        });

        if let Some(target_id) = self.target {
            if let Some(target) = ctx.resolve(target_id) {
                let mut operations = Vec::new();
                let adopted_build = Self::find_command(target, CommandId::Build);
                if let Some(build) = adopted_build {
                    operations.push(CommandOperation::PushFront(
                        CommandRequest::new(CommandId::Build)
                            .with_target(build.target)
                            .with_mode(CommandMode::SilentSub),
                    ));
                }

                if Self::find_command(target, CommandId::Construct).is_none() {
                    let result = CommandStepResult::failed(None).with_operations(operations);
                    return if adopted_build.is_some() {
                        // Native first calls Finish(true) after adopting the
                        // target's Build, then may call Finish(false) when
                        // that target no longer has Construct (:1714-1725).
                        result.with_events(vec![CommandEvent::NativeCommandSuccess {
                            object_id: ctx.object.id,
                            command: CommandId::Construct,
                        }])
                    } else {
                        result
                    };
                }

                let site = self.site.unwrap_or(Vector2::ZERO);
                let request = if site != Vector2::ZERO && !self.at_site(ctx, site) {
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(site.x))
                        .with_ty(Some(site.y))
                        .with_update_interval(50)
                        .with_mode(CommandMode::SilentSub)
                } else {
                    CommandRequest::new(CommandId::Wait)
                        .with_update_interval(10)
                        .with_mode(CommandMode::SilentSub)
                };
                operations.push(CommandOperation::PushFront(request));
                return if adopted_build.is_some() {
                    CommandStepResult::completed(None).with_operations(operations)
                } else {
                    CommandStepResult::running(None).with_operations(operations)
                };
            }
            // C4Game::ClearObjectPtrs turns a removed helper target into the
            // ordinary primary-construction path.
            self.target = None;
        }

        let owner = ctx.object.owner;
        let definition = match ctx.definition(&definition_id) {
            Some(definition) => definition,
            None => return CommandStepResult::failed(None),
        };

        if ctx
            .player(owner)
            .is_some_and(|player| !player.knows(&definition_id))
        {
            return CommandStepResult::failed(None);
        }

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Build | ActionProcedure::Chop | ActionProcedure::Dig
        ) {
            self.stop_continuation = true;
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopConstruct {
                    object_id: ctx.object.id,
                    command_instance_id: 0,
                },
            ]);
        }

        self.step_after_stop(ctx)
    }

    fn resume_after_stop(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if !std::mem::take(&mut self.stop_continuation) {
            return CommandStepResult::running(None);
        }
        self.step_after_stop(ctx)
    }

    fn step_after_stop(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(definition_id) = self.definition_id.clone() else {
            return CommandStepResult::failed(None);
        };
        let Some(definition) = ctx.definition(&definition_id) else {
            return CommandStepResult::failed(None);
        };
        let owner = ctx.object.owner;

        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target.is_some()
        {
            let request = CommandRequest::new(CommandId::UnGrab)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        let mut site = self.site.unwrap_or(Vector2::ZERO);
        if site == Vector2::ZERO {
            let Some(found) = self.find_construction_site(ctx, definition) else {
                return CommandStepResult::failed(None);
            };
            site = found;
            self.site = Some(site);
        }

        self.step_after_site(ctx)
    }

    fn step_after_site(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(definition_id) = self.definition_id.clone() else {
            return CommandStepResult::failed(None);
        };
        let Some(definition) = ctx.definition(&definition_id) else {
            return CommandStepResult::failed(None);
        };
        let site = self.site.unwrap_or(Vector2::ZERO);
        let owner = ctx.object.owner;

        if !self.spawn_requested {
            if self.script_pending {
                let Some(script_result) = self.script_result.take() else {
                    return CommandStepResult::running(None);
                };
                self.script_pending = false;
                match script_result {
                    AcquireScriptResult::Handled => {
                        self.script_invoked = false;
                        return CommandStepResult::running(None);
                    }
                    AcquireScriptResult::Complete => {
                        self.script_invoked = false;
                        return CommandStepResult::completed(None);
                    }
                    AcquireScriptResult::Failed => {
                        self.script_invoked = false;
                        return CommandStepResult::failed(None);
                    }
                    AcquireScriptResult::Continue => {
                        // Continue the same evaluated Construct pass below.
                    }
                }
            }

            if !self.script_invoked {
                self.script_pending = true;
                self.script_invoked = true;
                return CommandStepResult::running(None).with_events(vec![
                    CommandEvent::ControlCommandConstruction {
                        caller: ctx.object.id,
                        target: self.target,
                        site,
                        target2: self.target2,
                        definition_id: definition_id.clone(),
                        command_instance_id: 0,
                    },
                ]);
            }

            let kit_id = match self.builder_has_conkit(ctx) {
                Some(id) => id,
                None => {
                    self.script_invoked = false;
                    if let Some(c4id) = definition_id_to_c4id(CONKIT_DEFINITION) {
                        let request = CommandRequest::new(CommandId::Acquire)
                            .with_data(CommandData::Integer(c4id))
                            .with_update_interval(ACQUIRE_REQUEST_INTERVAL)
                            .with_retries(5)
                            .with_mode(CommandMode::Sub);
                        return CommandStepResult::running(None)
                            .with_operations(vec![CommandOperation::PushFront(request)]);
                    }
                    return CommandStepResult::failed(None);
                }
            };

            if !self.at_site(ctx, site) {
                self.script_invoked = false;
                let request = CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(site.x))
                    .with_ty(Some(site.y))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
                return CommandStepResult::running(None)
                    .with_operations(vec![CommandOperation::PushFront(request)]);
            }

            if let Some(failure) = self.construction_check(ctx, definition, site) {
                // ConstructionCheck's red feedback precedes C4Command::Fail
                // (C4Command.cpp:1797-1801; C4Landscape.cpp:2131-2163).
                return CommandStepResult::failed(None).with_events(vec![
                    CommandEvent::ConstructionCheckRejected {
                        actor_id: ctx.object.id,
                        definition_id: definition_id.clone(),
                        failure,
                    },
                ]);
            }

            self.spawn_requested = true;
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::SpawnConstruction {
                    actor_id: ctx.object.id,
                    definition_id: definition_id.clone(),
                    owner,
                    position: site,
                    kit_id,
                    command_instance_id: 0,
                },
            ]);
        }

        if self.construction_id.is_none() {
            if let Some(construction_id) = self.find_spawned_construction(ctx, &definition_id, site)
            {
                self.construction_id = Some(construction_id);
            } else {
                return CommandStepResult::running(None);
            }
        }

        let construction_id = self.construction_id.expect("construction id present");
        let mut operations = Vec::new();
        operations.push(CommandOperation::PushFront(
            CommandRequest::new(CommandId::Build)
                .with_target(Some(construction_id))
                .with_mode(CommandMode::SilentSub),
        ));

        CommandStepResult::completed(None).with_operations(operations)
    }

    fn resume_after_script(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        result: AcquireScriptResult,
    ) -> CommandStepResult {
        if !self.script_pending {
            return CommandStepResult::running(None);
        }
        self.script_result = Some(result);
        self.step_after_site(ctx)
    }

    fn resume_after_spawn(
        &mut self,
        _ctx: &CommandRuntimeContext<'_>,
        construction_id: Option<ObjectId>,
    ) -> CommandStepResult {
        if !self.spawn_requested || self.construction_id.is_some() {
            return CommandStepResult::running(None);
        }
        // Native continues even when CreateObjectConstruction returned null:
        // the kit is consumed, Construct finishes successfully and it still
        // attempts AddCommand(Build, nullptr). The typed Build request may be
        // rejected later, but this exact Construct must not remain pending.
        self.spawn_requested = false;
        self.construction_id = construction_id;
        CommandStepResult::completed(None).with_operations(vec![CommandOperation::PushFront(
            CommandRequest::new(CommandId::Build)
                // Required-target command states use object zero for a
                // native null pointer. This queues the Build so it fails
                // on its next Execute, suppressing Construct's finished
                // callback in the same way as AddCommand(Build,nullptr).
                .with_target(Some(construction_id.unwrap_or(ObjectId::new(0))))
                .with_mode(CommandMode::SilentSub),
        )])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TransferState {
    target: ObjectId,
    tx: Option<i32>,
    /// Exact tagged C4Command::Tx. The integer/C4ID mirrors remain for
    /// compatibility with older snapshots and GetCommand projections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tx_value: Option<clonk_script::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tx_definition: Option<DefinitionId>,
    ty: Option<i32>,
}

impl TransferState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            tx: request.tx,
            tx_value: request.tx_value.clone(),
            tx_definition: request.tx_definition.clone(),
            ty: request.ty,
        })
    }

    fn effective_tx_value(&self) -> clonk_script::Value {
        self.tx_value
            .clone()
            .or_else(|| {
                self.tx_definition
                    .as_ref()
                    .map(|value| clonk_script::Value::C4Id(value.clone()))
            })
            .or_else(|| self.tx.map(clonk_script::Value::Int))
            // C4Object::AddCommand's ordinary overload supplies C4VInt(0)
            // when no explicit Tx was given (C4Object.h:221-226).
            .unwrap_or(clonk_script::Value::Int(0))
    }

    fn within_zone(&self, ctx: &CommandRuntimeContext<'_>, zone: &TransferZone) -> bool {
        let left = zone.x - 5;
        let right = zone.x + zone.width - 1 + 5;
        let x = ctx.position.x;
        x >= left && x <= right
    }

    /// `C4TransferZone::GetEntryPoint` (C4TransferZone.cpp:139-180): clamp
    /// to the adjacent perimeter, search both directions for a free pixel,
    /// then ground side entries with AdjustMoveToTarget.
    fn entry_point(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        zone: &TransferZone,
        actor_pos: Vector2,
    ) -> Option<Vector2> {
        let inside_x = actor_pos.x >= zone.x && actor_pos.x < zone.x + zone.width;
        let inside_y = actor_pos.y >= zone.y && actor_pos.y < zone.y + zone.height;
        let mut target_x = actor_pos.x;
        let target_y = actor_pos.y;

        if inside_x && inside_y {
            if actor_pos.x < zone.x + zone.width / 2 {
                target_x = zone.x - 1;
            } else {
                target_x = zone.x + zone.width;
            }
        }

        let mut x = bound_by(target_x, zone.x - 1, zone.x + zone.width);
        let mut y = bound_by(target_y, zone.y - 1, zone.y + zone.height);
        let (mut x1, mut y1, mut x2, mut y2) = (x, y, x, y);
        let (mut x_incr1, mut y_incr1) = (0, -1);
        let (mut x_incr2, mut y_incr2) = (0, 1);
        let solid = |x: i32, y: i32| {
            ctx.landscape
                .is_some_and(|landscape| landscape.is_solid_at(x, y))
        };
        let mut found = false;
        for _ in 0..2 * zone.width + 2 * zone.height {
            if !solid(x1, y1) {
                x = x1;
                y = y1;
                found = true;
                break;
            }
            if !solid(x2, y2) {
                x = x2;
                y = y2;
                found = true;
                break;
            }
            x1 += x_incr1;
            y1 += y_incr1;
            x2 += x_incr2;
            y2 += y_incr2;
            if y1 < zone.y - 1 {
                y1 = zone.y - 1;
                x_incr1 = 1;
                y_incr1 = 0;
            }
            if x1 > zone.x + zone.width {
                x1 = zone.x + zone.width;
                x_incr1 = 0;
                y_incr1 = 1;
            }
            if y1 > zone.y + zone.height {
                y1 = zone.y + zone.height;
                x_incr1 = -1;
                y_incr1 = 0;
            }
            if x1 < zone.x - 1 {
                x1 = zone.x - 1;
                x_incr1 = 0;
                y_incr1 = -1;
            }
            if y2 < zone.y - 1 {
                y2 = zone.y - 1;
                x_incr2 = -1;
                y_incr2 = 0;
            }
            if x2 > zone.x + zone.width {
                x2 = zone.x + zone.width;
                x_incr2 = 0;
                y_incr2 = -1;
            }
            if y2 > zone.y + zone.height {
                y2 = zone.y + zone.height;
                x_incr2 = 1;
                y_incr2 = 0;
            }
            if x2 < zone.x - 1 {
                x2 = zone.x - 1;
                x_incr2 = 0;
                y_incr2 = 1;
            }
        }
        if !found {
            return None;
        }
        if !(zone.x..zone.x + zone.width).contains(&x) {
            if let Some(landscape) = ctx.landscape {
                adjust_move_to_target(landscape, &mut x, &mut y, false, 20);
            }
        }
        Some(Vector2::new(x, y))
    }

    fn should_call_script(frame: u64) -> bool {
        frame.is_multiple_of(5)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let target_id = self.target;
        if ctx.resolve(target_id).is_none() {
            return CommandStepResult::failed(None);
        }
        let Some(zone) = ctx.transfer_zone(self.target) else {
            return CommandStepResult::failed(None);
        };

        if !self.within_zone(ctx, zone) {
            let Some(entry) = self.entry_point(ctx, zone, ctx.position) else {
                return CommandStepResult::failed(None);
            };
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(entry.x))
                .with_ty(Some(entry.y))
                .with_update_interval(25);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        if Self::should_call_script(ctx.frame) {
            let event = CommandEvent::ControlTransfer {
                object_id: self.target,
                caller: ctx.object.id,
                tx_value: self.effective_tx_value(),
                ty: self.ty.unwrap_or(0),
                command_instance_id: 0,
            };
            return CommandStepResult::running(None).with_events(vec![event]);
        }

        CommandStepResult::running(None)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ChopState {
    target: ObjectId,
    update_interval: u32,
    #[serde(default)]
    physical_pending: bool,
}

impl ChopState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            physical_pending: false,
        })
    }

    fn at_target_stop_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
    }

    fn chop_action_name(&self, ctx: &CommandRuntimeContext<'_>) -> Option<String> {
        ctx.definition(&ctx.object.definition_id)
            .and_then(|definition| definition.chop_action.clone())
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if ctx.resolve(self.target).is_none() {
            return CommandStepResult::failed(None);
        }
        if ctx.object.physical_deferred {
            self.physical_pending = true;
            return resolve_command_physical(ctx.object.id, 1, None);
        }
        self.step_after_physical(ctx, ctx.object.physical)
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        if !std::mem::take(&mut self.physical_pending) {
            return CommandStepResult::running(None);
        }
        self.step_after_physical(ctx, physical)
    }

    fn step_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        if physical.can_chop == 0 {
            return CommandStepResult::failed(None);
        }

        let target_snapshot = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => {
                return CommandStepResult::failed(None);
            }
        };

        if target_snapshot.ocf & ocf::CHOP == 0 {
            return CommandStepResult::completed(None);
        }

        if ctx.object.action_procedure == ActionProcedure::Chop
            && ctx.object.action_target == Some(self.target)
        {
            return CommandStepResult::running(None);
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            let request = CommandRequest::new(CommandId::UnGrab)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            let mut result = CommandStepResult::running(None);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Chop | ActionProcedure::Build | ActionProcedure::Dig
        ) {
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopChop {
                    object_id: ctx.object.id,
                },
            ]);
        }

        let dx = target_snapshot.position.x - ctx.position.x;

        const MIN_HORIZONTAL_RANGE: i32 = 4;
        const MAX_HORIZONTAL_RANGE: i32 = 9;
        const MOVE_AWAY_HORIZONTAL_RANGE: i32 = 5;

        let at_target = target_snapshot.has_nonzero_status()
            && ctx.object.container.is_none()
            && target_snapshot.container.is_none()
            && target_snapshot.at_point(ctx.position.x, ctx.position.y)
            && dx.abs() >= MIN_HORIZONTAL_RANGE
            && dx.abs() <= MAX_HORIZONTAL_RANGE;

        if at_target {
            let update = self.at_target_stop_update(ctx);
            if ctx.object.action_procedure != ActionProcedure::Walk {
                return CommandStepResult::running(update);
            }

            let action_name = self
                .chop_action_name(ctx)
                .unwrap_or_else(|| "Chop".to_string());
            let action_update = ActionUpdate::default()
                .with_name(action_name)
                .with_target(Some(self.target))
                .with_phase(0)
                .with_ticks(0)
                .with_force(false);
            let update = update.unwrap_or_default().with_action_update(action_update);
            return CommandStepResult::running(Some(update));
        }

        let mut result = CommandStepResult::running(None);

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

        if dx.abs() < MOVE_AWAY_HORIZONTAL_RANGE {
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

        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DigState {
    target: Vector2,
    update_interval: u32,
    dig_out_material: bool,
    ungrab_requested: bool,
    exit_requested: bool,
    /// Transient live ObjectComDig boundary. Command snapshots taken by an
    /// action callback retain it, while persisted saves never observe the
    /// synchronous event in flight.
    #[serde(skip)]
    start_pending: bool,
}

impl DigState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        // Tx is a C4Value and Ty is an integer; omitted command arguments
        // read as zero rather than making Dig structurally invalid.
        let tx = request.tx.unwrap_or(0);
        let ty = request.ty.unwrap_or(0);
        let dig_out_material = match &request.data {
            CommandData::Integer(value) => *value != 0,
            CommandData::Text(text) => !text.is_empty(),
            CommandData::None => false,
        };
        Ok(Self {
            target: Vector2::new(tx, ty),
            update_interval: positive_helper_interval_or_one(request.update_interval),
            dig_out_material,
            ungrab_requested: false,
            exit_requested: false,
            start_pending: false,
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
            let update = update.unwrap_or_default();
            Some(update.with_command_direction(CommandDirection::Stop))
        }
    }

    fn apply_idle(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        update: Option<ObjectUpdate>,
    ) -> Option<ObjectUpdate> {
        let update = self.ensure_stop(ctx, update).unwrap_or_default();
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

    fn adjusted_target(&self, ctx: &CommandRuntimeContext<'_>) -> Vector2 {
        Vector2::new(self.target.x, self.target.y + ctx.object.shape_top + 3)
    }

    fn move_to_range(&self, ctx: &CommandRuntimeContext<'_>) -> i32 {
        if ctx.object.move_to_range > 0 {
            ctx.object.move_to_range
        } else {
            DIG_MOVE_TO_RANGE_DEFAULT
        }
    }

    fn desired_direction(&self, position: Vector2, target: Vector2) -> Option<CommandDirection> {
        let mut direction = None;

        // C4Command::Dig (C4Command.cpp:478-484) deliberately uses seven
        // independent writes. In particular, the DownLeft guard compares
        // against `tx - DigRange`, and the chain never assigns plain Up.
        if position.x < target.x - DIG_DIRECTION_RANGE {
            direction = Some(CommandDirection::Right);
        }
        if position.x > target.x + DIG_DIRECTION_RANGE {
            direction = Some(CommandDirection::Left);
        }
        if position.y < target.y - DIG_DIRECTION_RANGE {
            direction = Some(CommandDirection::Down);
        }
        if position.x < target.x - DIG_DIRECTION_RANGE
            && position.y < target.y - DIG_DIRECTION_RANGE
        {
            direction = Some(CommandDirection::DownRight);
        }
        if position.x > target.x - DIG_DIRECTION_RANGE
            && position.y < target.y - DIG_DIRECTION_RANGE
        {
            direction = Some(CommandDirection::DownLeft);
        }
        if position.x < target.x - DIG_DIRECTION_RANGE
            && position.y > target.y + DIG_DIRECTION_RANGE
        {
            direction = Some(CommandDirection::UpRight);
        }
        if position.x > target.x + DIG_DIRECTION_RANGE
            && position.y > target.y + DIG_DIRECTION_RANGE
        {
            direction = Some(CommandDirection::UpLeft);
        }

        direction
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        // C++ snapshots the adjusted bottom-center target before any helper
        // call in this evaluation, but recomputes it from live Shape.y on
        // every later execution.
        let target = self.adjusted_target(ctx);
        let mut pending_update: Option<ObjectUpdate> = None;

        if ctx.object.action_procedure == ActionProcedure::Push {
            pending_update = self.ensure_stop(ctx, pending_update);
            if !self.ungrab_requested {
                self.ungrab_requested = true;
                let mut result = CommandStepResult::running(pending_update.clone());
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
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
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(pending_update);
        }
        self.exit_requested = false;

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
            let xdirf = if ctx.object.direction == Direction::Left {
                1
            } else {
                -1
            };
            pending_update = Some(let_go_update(None, xdirf));
        }

        let move_to_range = self.move_to_range(ctx);
        let dx = target.x - ctx.position.x;
        let dy = target.y - ctx.position.y;
        if dx.abs() <= move_to_range && dy.abs() <= move_to_range {
            let update = self.apply_idle(ctx, pending_update);
            return CommandStepResult::completed(update);
        }

        if ctx.object.action_procedure != ActionProcedure::Dig {
            if ctx.object.action_procedure != ActionProcedure::Walk {
                return CommandStepResult::running(pending_update);
            }
            if self.start_pending {
                return CommandStepResult::running(pending_update);
            }
            self.start_pending = true;
            return CommandStepResult::running(pending_update).with_events(vec![
                CommandEvent::ObjectComDig {
                    actor_id: ctx.object.id,
                    dig_out_material: self.dig_out_material,
                    direction: self.desired_direction(ctx.position, target),
                    command_instance_id: 0,
                },
            ]);
        }

        if self.dig_out_material {
            let update = pending_update.unwrap_or_default().with_action_data(1);
            pending_update = Some(update);
        }

        if let Some(direction) = self.desired_direction(ctx.position, target) {
            if ctx.object.command_direction != direction {
                let mut update = pending_update.unwrap_or_default();
                update = update.with_command_direction(direction);
                pending_update = Some(update);
            }
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
    #[serde(default, skip_serializing_if = "crate::is_false")]
    reject_pending: bool,
    #[serde(default, skip_serializing_if = "crate::is_false")]
    target_cleared: bool,
}

impl GrabState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        // C4Object::AddCommand accepts a null target and leaves the pointer
        // check to C4Command::Grab. Object IDs start above zero, so retain a
        // reserved ID internally while the request keeps the script-visible
        // Target=nil field (C4Object.cpp:3914-3919; C4Command.cpp:667-687).
        let target = request.target.unwrap_or_else(|| ObjectId::new(0));
        Ok(Self {
            target,
            offset_x: request.tx.unwrap_or(0),
            offset_y: request.ty.unwrap_or(0),
            update_interval: positive_helper_interval_or_one(request.update_interval),
            reject_pending: false,
            target_cleared: request.target.is_none(),
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let target = (!self.target_cleared).then_some(self.target);
        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target == target
        {
            return CommandStepResult::completed(None);
        }

        // C++ stops BUILD/CHOP first and then DIG before consulting Target
        // or adding UnGrab/MoveTo. The live event must therefore run even
        // when a callback-cleared target is no longer resolvable here.
        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Build | ActionProcedure::Chop | ActionProcedure::Dig
        ) {
            self.reject_pending = true;
            return CommandStepResult::running(None).with_events(vec![CommandEvent::AttemptGrab {
                actor_id: ctx.object.id,
                target_id: self.target,
            }]);
        }

        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target != target
        {
            let request = CommandRequest::new(CommandId::UnGrab)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            let mut result = CommandStepResult::running(None);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        if self.target_cleared {
            return CommandStepResult::failed(None);
        }

        let target_snapshot = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(None),
        };

        let approach_position = Vector2::new(
            target_snapshot.position.x + self.offset_x,
            target_snapshot.position.y + self.offset_y,
        );

        // "At target object: grab": point-in-shape like C4Command::Grab
        // (Target->At(cObj->x, cObj->y, ocf), C4Command.cpp:689-691).
        let can_grab_here = ctx.object.container.is_none()
            && target_snapshot.has_nonzero_status()
            && target_snapshot.container.is_none()
            && target_snapshot.at_point(ctx.position.x, ctx.position.y)
            && (target_snapshot.ocf & ocf::ALL) != 0;

        if can_grab_here {
            // Stop/Push must not be staged ahead of RejectGrabbed. The live
            // event also performs ObjectComStop and Scale/Hangle's let-go.
            self.reject_pending = true;
            return CommandStepResult::running(None).with_events(vec![CommandEvent::AttemptGrab {
                actor_id: ctx.object.id,
                target_id: self.target,
            }]);
        }

        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(approach_position.x))
            .with_ty(Some(approach_position.y))
            .with_update_interval(50);
        let mut result = CommandStepResult::running(None);
        result.operations.push(CommandOperation::PushFront(request));
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ActivateState {
    target: Option<ObjectId>,
    container: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    remaining: i32,
    update_interval: u32,
    exit_requested: bool,
    enter_requested: bool,
}

impl ActivateState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let definition_id = command_data_to_definition_id(&request.data);
        let remaining = if definition_id.is_some() {
            request.tx.unwrap_or(1).max(1)
        } else {
            1
        };
        Ok(Self {
            target: request.target,
            container: request.target2,
            definition_id,
            remaining,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            exit_requested: false,
            enter_requested: false,
        })
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.action_procedure != ActionProcedure::Dig {
            return None;
        }
        let mut update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
        let action_update = ActionUpdate::default()
            .with_name("Idle")
            .with_force(true)
            .with_phase(0)
            .with_ticks(0);
        update = update.with_action_update(action_update);
        Some(update)
    }

    fn resolve_container(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        if self
            .container
            .is_some_and(|container_id| ctx.resolve(container_id).is_none())
        {
            self.container = None;
        }
        if self.container.is_none() {
            if let Some(target_id) = self.target {
                if let Some(snapshot) = ctx.resolve(target_id) {
                    self.container = snapshot.container;
                }
            }
        }
        self.container
    }

    fn check_minimum_con(&self, target: &CommandObjectSnapshot) -> bool {
        !minimum_con_activation_denied(target.category, target.construction)
    }

    fn find_release_candidate(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        container_id: ObjectId,
        released: &HashSet<ObjectId>,
    ) -> Option<ObjectId> {
        let definition_id = self.definition_id.as_ref()?;
        let container = ctx.resolve(container_id)?;
        container.contents.iter().copied().find(|object_id| {
            !released.contains(object_id)
                && ctx.resolve(*object_id).is_some_and(|snapshot| {
                    snapshot.has_nonzero_status()
                        && snapshot.definition_id == *definition_id
                        && snapshot.container == Some(container_id)
                        && snapshot
                            .commands
                            .first()
                            .is_none_or(|command| command.name != CommandId::Exit.to_name())
                })
        })
    }

    fn release_targets(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        container_id: ObjectId,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        let mut result = CommandStepResult::completed(update);
        let mut released = HashSet::new();

        while self.remaining > 0 {
            let target_id = match self.target {
                Some(target_id) => target_id,
                None => {
                    let Some(target_id) = self.find_release_candidate(ctx, container_id, &released)
                    else {
                        result.status = CommandStatus::Failed;
                        return result;
                    };
                    self.target = Some(target_id);
                    target_id
                }
            };
            let Some(target) = ctx.resolve(target_id) else {
                result.status = CommandStatus::Failed;
                return result;
            };
            if target.container != Some(container_id) || !self.check_minimum_con(target) {
                result.status = CommandStatus::Failed;
                return result;
            }

            result.events.push(CommandEvent::SetObjectCommand {
                object_id: target_id,
                controller: Some(ctx.object.controller),
                request: CommandRequest::new(CommandId::Exit).with_mode(CommandMode::Base),
            });
            released.insert(target_id);
            self.target = None;
            self.remaining -= 1;
        }

        result
    }

    fn request_exit(&mut self, update: Option<ObjectUpdate>) -> CommandStepResult {
        if self.exit_requested {
            CommandStepResult::running(update)
        } else {
            self.exit_requested = true;
            let mut result = CommandStepResult::running(update);
            let request = CommandRequest::new(CommandId::Exit)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            result
        }
    }

    fn request_enter(
        &mut self,
        container_id: ObjectId,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        if self.enter_requested {
            CommandStepResult::running(update)
        } else {
            self.enter_requested = true;
            let mut result = CommandStepResult::running(update);
            let request = CommandRequest::new(CommandId::Enter)
                .with_target(Some(container_id))
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            result
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.target.is_none() && self.definition_id.is_none() {
            let Some(container) = self.container else {
                return CommandStepResult::failed(None);
            };
            if ctx.resolve(container).is_none() {
                return CommandStepResult::failed(None);
            }
            return CommandStepResult::completed(None).with_events(vec![CommandEvent::OpenMenu(
                MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.controller,
                    kind: MenuRequestKind::ActivateTarget { container },
                },
            )]);
        }

        if let Some(target_id) = self.target {
            let Some(target) = ctx.resolve(target_id) else {
                return CommandStepResult::failed(None);
            };
            if target.container.is_none() {
                return CommandStepResult::completed(None);
            }
        }

        let Some(container_id) = self.resolve_container(ctx) else {
            return CommandStepResult::failed(None);
        };

        let update = self.prepare_update(ctx);

        if ctx.object.container.is_none() {
            self.exit_requested = false;
        }
        if ctx.object.container == Some(container_id) {
            self.enter_requested = false;
        }

        if ctx.object.id == container_id || ctx.object.container == Some(container_id) {
            return self.release_targets(ctx, container_id, update);
        }

        // C++ resolves a Data/type-based target only inside the container.
        // Keeping it unset while navigating also ensures that a candidate
        // which starts exiting before entry is filtered from the live scan.
        if let Some(target_id) = self.target {
            let target_snapshot = match ctx.resolve(target_id) {
                Some(snapshot) => snapshot,
                None => return CommandStepResult::failed(update),
            };

            if target_snapshot.container.is_none() {
                return CommandStepResult::completed(update);
            }

            if target_snapshot.container != Some(container_id) {
                return CommandStepResult::failed(update);
            }

            if !self.check_minimum_con(target_snapshot) {
                return CommandStepResult::failed(update);
            }
        }

        if let Some(current_container) = ctx.object.container {
            if current_container != container_id {
                return self.request_exit(update);
            }
        }

        if let Some(container_snapshot) = ctx.resolve(container_id) {
            if container_snapshot.ocf & ocf::ENTRANCE != 0 {
                return self.request_enter(container_id, update);
            }
        }

        CommandStepResult::failed(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PushToState {
    target: Option<ObjectId>,
    container: Option<ObjectId>,
    tx: Option<i32>,
    ty: Option<i32>,
    update_interval: u32,
    /// Ordinary AddCommand calls leave PushTo pending InitEvaluation. Older
    /// snapshots predate this state and therefore retain their old, already
    /// executing behavior when the field is absent.
    #[serde(default)]
    evaluation_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    physical_continuation: Option<Vector2>,
}

impl PushToState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            target: request.target,
            container: request.target2,
            tx: request.tx,
            ty: request.ty,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            evaluation_pending: !request.evaluated,
            physical_continuation: None,
        })
    }

    fn destination(&self) -> Vector2 {
        // C4Command::Tx is a C4Value and Ty is an integer; both read as
        // zero when AddCommand left their slots empty.
        Vector2::new(self.tx.unwrap_or(0), self.ty.unwrap_or(0))
    }

    /// C4CMD_PushTo InitEvaluation (C4Command.cpp:1645-1652): ground the
    /// destination once, using the actor's live FreeMoveTo/shape state, and
    /// consume this Execute before the PushTo handler runs.
    fn init_evaluation(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<CommandStepResult> {
        if !self.evaluation_pending {
            return None;
        }
        self.evaluation_pending = false;

        let destination = self.destination();
        let (mut x, mut y) = (destination.x, destination.y);
        if let Some(landscape) = ctx.landscape {
            let free_move = if ctx.object.action_procedure == ActionProcedure::Float {
                true
            } else if ctx.object.physical_deferred {
                self.physical_continuation = Some(destination);
                return Some(resolve_command_physical(ctx.object.id, 1, None));
            } else {
                ctx.object.physical.can_fly != 0
            };
            adjust_move_to_target(
                landscape,
                &mut x,
                &mut y,
                free_move,
                ctx.object.shape_height,
            );
        }
        self.tx = Some(x);
        self.ty = Some(y);
        Some(CommandStepResult::running(None))
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        let Some(destination) = self.physical_continuation.take() else {
            return CommandStepResult::running(None);
        };
        let (mut x, mut y) = (destination.x, destination.y);
        if let Some(landscape) = ctx.landscape {
            adjust_move_to_target(
                landscape,
                &mut x,
                &mut y,
                physical.can_fly != 0,
                ctx.object.shape_height,
            );
        }
        self.tx = Some(x);
        self.ty = Some(y);
        CommandStepResult::running(None)
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if !matches!(
            ctx.object.action_procedure,
            ActionProcedure::Build | ActionProcedure::Chop | ActionProcedure::Dig
        ) {
            return None;
        }
        let idle_action = ActionUpdate::default()
            .with_name("Idle")
            .with_force(true)
            .with_phase(0)
            .with_ticks(0);
        Some(
            ObjectUpdate::new()
                .with_command_direction(CommandDirection::Stop)
                .with_action_update(idle_action),
        )
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(target) = self.target else {
            return CommandStepResult::failed(None);
        };
        let target_snapshot = match ctx.resolve(target) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(None),
        };

        if self.container == Some(target) {
            return CommandStepResult::failed(None);
        }

        if let Some(destination) = self.container {
            if target_snapshot.container == Some(destination) {
                return CommandStepResult::completed(None);
            }
        } else {
            let destination = self.destination();
            let dx = target_snapshot.position.x - destination.x;
            let dy = target_snapshot.position.y - destination.y;
            if dx.abs() <= PUSH_TO_RANGE && dy.abs() <= PUSH_TO_RANGE {
                let completion_update =
                    ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                let mut result = CommandStepResult::completed(Some(completion_update));
                let mut operations = Vec::new();
                let ungrab_request =
                    CommandRequest::new(CommandId::UnGrab).with_mode(CommandMode::SilentSub);
                operations.push(CommandOperation::PushFront(ungrab_request));
                let wait_request = CommandRequest::new(CommandId::Wait)
                    .with_update_interval(10)
                    .with_mode(CommandMode::SilentSub);
                operations.push(CommandOperation::PushFront(wait_request));
                result = result.with_operations(operations);
                return result;
            }
        }

        let update = self.prepare_update(ctx);

        if let Some(target_container) = target_snapshot.container {
            if Some(target_container) != self.container {
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Activate)
                    .with_target(Some(target))
                    .with_update_interval(40)
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
        }

        let pushing_target = ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target == Some(target);

        if !pushing_target {
            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::Grab)
                .with_target(Some(target))
                .with_update_interval(40)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        if let Some(destination) = self.container {
            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::Enter)
                .with_target(Some(destination))
                .with_update_interval(40)
                .with_mode(CommandMode::SilentSub)
                .with_data(CommandData::Integer(COMMAND_FLAG_ENTER_PUSH_TARGET));
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        let destination = self.destination();
        let mut result = CommandStepResult::running(update.clone());
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(destination.x))
            .with_ty(Some(destination.y))
            .with_update_interval(40)
            .with_mode(CommandMode::SilentSub)
            .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET));
        result.operations.push(CommandOperation::PushFront(request));
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct UnGrabState {
    update_interval: u32,
    #[serde(default)]
    completion_pending: bool,
}

impl UnGrabState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            update_interval: positive_helper_interval_or_one(request.update_interval),
            completion_pending: false,
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        self.completion_pending = true;
        CommandStepResult::running(None).with_events(vec![CommandEvent::ObjectComUnGrabCommand {
            actor_id: ctx.object.id,
            command_instance_id: 0,
        }])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct JumpState {
    tx: Option<i32>,
    evaluated: bool,
}

/// ObjectComJump's fixed launch calculation (C4ObjectCom.cpp:284-296).
/// The live engine owns the WALK gate, script callback, and action change.
pub(crate) fn object_com_jump_launch(
    construction: i32,
    physical: PhysicalInfo,
    command_direction: CommandDirection,
    direction: Direction,
) -> FixedVec2 {
    let con_scale = math::itofix_prec(construction, FULL_CON);
    let physical_walk = math::val_by_physical(280, physical.walk) * con_scale;
    let physical_jump = math::val_by_physical(1000, physical.jump) * con_scale;
    let txdir = match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft => -physical_walk,
        CommandDirection::Right | CommandDirection::UpRight => physical_walk,
        _ => match direction {
            Direction::Left => -physical_walk,
            Direction::Right => physical_walk,
            _ => crate::C4Fixed::ZERO,
        },
    };
    FixedVec2::new(txdir, -physical_jump)
}

impl JumpState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            tx: request.tx,
            evaluated: false,
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.evaluated {
            return CommandStepResult::completed(None);
        }
        self.evaluated = true;
        CommandStepResult::running(None).with_events(vec![CommandEvent::ObjectComJump {
            object_id: ctx.object.id,
            // C4Command::Tx is an integer value; absent/nil and explicit zero
            // are the same sentinel for C4Command::Jump (:1058-1063).
            tx: self.tx.unwrap_or(0),
        }])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WaitState {
    /// Effective signed interval chosen from Data, Tx, then UpdateInterval.
    /// The active command installs Data/Tx overrides during InitEvaluation;
    /// negative values remain negative and therefore skip the countdown.
    remaining: Option<i32>,
    #[serde(default)]
    evaluation_pending: bool,
    /// Data and Tx overwrite UpdateInterval after its ordinary pre-evaluation
    /// decrement; a plain UpdateInterval must not be reset on that frame.
    #[serde(default)]
    evaluation_overrides_interval: bool,
}

impl WaitState {
    fn from_request(request: &CommandRequest) -> Self {
        // C4CMD_Wait InitEvaluation (C4Command.cpp:1659-1663): a nonzero
        // Data overrides the update interval, else a nonzero Tx does.
        let (interval, evaluation_overrides_interval) = match request.data {
            CommandData::Integer(data) if data != 0 => (data, true),
            _ => match request.tx {
                Some(tx) if tx != 0 => (tx, true),
                _ => (request.update_interval, false),
            },
        };
        let remaining = (interval != 0).then_some(interval);
        Self {
            remaining,
            evaluation_pending: !request.evaluated,
            evaluation_overrides_interval,
        }
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
        CommandStepResult::running(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PutState {
    container: ObjectId,
    requested_item: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    #[serde(default)]
    remaining_count: i32,
    update_interval: u32,
    /// C4Command::Put reuses Ty as a reminder to let go after it grabbed a
    /// GrabPut target itself.
    #[serde(default)]
    put_ty: i32,
    /// A live ObjectComPut event is being resolved synchronously.
    #[serde(default)]
    put_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    physical_continuation: Option<PutPhysicalContinuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stop_continuation: Option<PutStopContinuation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct PutPhysicalContinuation {
    item_id: ObjectId,
    target_position: Vector2,
    p_grabbing: Option<ObjectId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct PutStopContinuation {
    item_id: ObjectId,
}

impl PutState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let container = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            container,
            requested_item: request.target2,
            definition_id: command_data_to_definition_id(&request.data),
            remaining_count: request.tx.unwrap_or(0),
            update_interval: positive_helper_interval_or_one(request.update_interval),
            put_ty: request.ty.unwrap_or(0),
            put_pending: false,
            physical_continuation: None,
            stop_continuation: None,
        })
    }

    fn resolve_item<'a>(
        &mut self,
        ctx: &'a CommandRuntimeContext<'a>,
    ) -> Result<Option<(ObjectId, &'a CommandObjectSnapshot)>, ()> {
        if let Some(item_id) = self.requested_item {
            if let Some(snapshot) = ctx.resolve(item_id) {
                return Ok(Some((item_id, snapshot)));
            }
            self.requested_item = None;
        }

        if let Some(definition_id) = &self.definition_id {
            for object_id in &ctx.object.contents {
                if let Some(snapshot) = ctx
                    .resolve(*object_id)
                    .filter(|snapshot| snapshot.has_nonzero_status())
                {
                    if &snapshot.definition_id == definition_id {
                        self.requested_item = Some(*object_id);
                        return Ok(Some((*object_id, snapshot)));
                    }
                }
            }
            // A requested definition is a hard requirement. C++ finishes
            // unsuccessfully instead of falling back to arbitrary contents.
            return Err(());
        }

        if let Some((object_id, snapshot)) = ctx.object.contents.iter().find_map(|object_id| {
            ctx.resolve(*object_id)
                .filter(|snapshot| snapshot.has_nonzero_status())
                .map(|snapshot| (*object_id, snapshot))
        }) {
            self.requested_item = Some(object_id);
            return Ok(Some((object_id, snapshot)));
        }

        Ok(None)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        self.step_with_gravity(ctx, crate::PhysicsSettings::default().gravity_as_c4fixed())
    }

    fn step_with_gravity(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
    ) -> CommandStepResult {
        if self.put_pending {
            return CommandStepResult::running(None);
        }

        let container_snapshot = match ctx.resolve(self.container) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(None),
        };

        let (item_id, item_snapshot) = match self.resolve_item(ctx) {
            Ok(Some(value)) => value,
            Ok(None) => return CommandStepResult::completed(None),
            Err(()) => return CommandStepResult::failed(None),
        };

        if item_snapshot.container == Some(self.container) {
            if self.remaining_count > 1 {
                self.requested_item = None;
                self.remaining_count -= 1;
                return CommandStepResult::running(None);
            }
            return CommandStepResult::completed(None);
        }

        if item_snapshot.container != Some(ctx.object.id) {
            // A nearby uncontained object with impact speed is still in
            // flight. The C++ command waits instead of chasing it.
            if item_snapshot.container.is_none()
                && c4_distance(
                    ctx.position.x,
                    ctx.position.y,
                    item_snapshot.position.x,
                    item_snapshot.position.y,
                ) < 80
                && item_snapshot.ocf & ocf::HIT_SPEED1 != 0
            {
                return CommandStepResult::running(None);
            }

            let request = CommandRequest::new(CommandId::Get)
                .with_target(Some(item_id))
                .with_update_interval(40)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        // A contained target cannot receive a Put. This precedes the DIG
        // stop and all navigation branches (C4Command.cpp:1431-1436).
        if container_snapshot.container.is_some() {
            return CommandStepResult::failed(None);
        }

        if ctx.object.action_procedure == ActionProcedure::Dig {
            self.stop_continuation = Some(PutStopContinuation { item_id });
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopPut {
                    object_id: ctx.object.id,
                    command_instance_id: 0,
                },
            ]);
        }

        self.step_after_stop(ctx, gravity, item_id)
    }

    fn resume_after_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
    ) -> CommandStepResult {
        let Some(continuation) = self.stop_continuation.take() else {
            return CommandStepResult::running(None);
        };
        self.step_after_stop(ctx, gravity, continuation.item_id)
    }

    fn step_after_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        item_id: ObjectId,
    ) -> CommandStepResult {
        let Some(container_snapshot) = ctx.resolve(self.container) else {
            return CommandStepResult::failed(None);
        };
        let Some(item_snapshot) = ctx.resolve(item_id) else {
            return CommandStepResult::failed(None);
        };
        let p_grabbing = (ctx.object.action_procedure == ActionProcedure::Push)
            .then_some(ctx.object.action_target)
            .flatten();

        if p_grabbing.is_some() && p_grabbing != Some(self.container) {
            let request = CommandRequest::new(CommandId::UnGrab)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        if ctx.object.container == Some(self.container) {
            self.put_pending = true;
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComPut {
                    actor_id: ctx.object.id,
                    target_id: self.container,
                    object_id: item_id,
                    ungrab_on_success: false,
                    command_instance_id: 0,
                },
            ]);
        }

        if ctx.object.container.is_some() {
            let request = CommandRequest::new(CommandId::Exit)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        let target_definition = ctx.definition(&container_snapshot.definition_id);
        let item_is_fragile = ctx
            .definition(&item_snapshot.definition_id)
            .is_some_and(|definition| definition.fragile);
        if container_snapshot.ocf & ocf::COLLECTION != 0
            && !item_is_fragile
            && p_grabbing != Some(self.container)
        {
            if let Some(collection) =
                target_definition.and_then(|definition| definition.collection_rect)
            {
                let target_position = Vector2::new(
                    container_snapshot.position.x + collection.x + collection.width / 2,
                    container_snapshot.position.y + collection.y + collection.height / 2,
                );
                if ctx.object.physical_deferred {
                    self.physical_continuation = Some(PutPhysicalContinuation {
                        item_id,
                        target_position,
                        p_grabbing,
                    });
                    return resolve_command_physical(ctx.object.id, 1, None);
                }
                return self.step_after_throw_physical(
                    ctx,
                    gravity,
                    ctx.object.physical,
                    item_id,
                    target_position,
                    p_grabbing,
                    None,
                );
            }
        }

        self.step_after_throw_attempt(ctx, item_id, p_grabbing, None)
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        let Some(continuation) = self.physical_continuation.take() else {
            return CommandStepResult::running(None);
        };
        self.step_after_throw_physical(
            ctx,
            gravity,
            physical,
            continuation.item_id,
            continuation.target_position,
            continuation.p_grabbing,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn step_after_throw_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        physical: PhysicalInfo,
        item_id: ObjectId,
        target_position: Vector2,
        p_grabbing: Option<ObjectId>,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        let Some(container_snapshot) = ctx.resolve(self.container) else {
            return CommandStepResult::failed(update);
        };
        let throw_force = math::val_by_physical(400, physical.throw);
        let target_distance = c4_distance(
            ctx.position.x,
            ctx.position.y,
            container_snapshot.position.x,
            container_snapshot.position.y,
        );
        let throwing_position_found = ctx.landscape.is_some_and(|landscape| {
            [1, -1].into_iter().any(|direction| {
                landscape
                    .find_throwing_position(
                        target_position,
                        FixedVec2::new(throw_force * direction, -throw_force),
                        ctx.object.shape_height,
                        gravity,
                    )
                    .is_some_and(|position| {
                        c4_distance(position.x, position.y, ctx.position.x, ctx.position.y)
                            < target_distance
                    })
            })
        });
        if throwing_position_found {
            let request = CommandRequest::new(CommandId::Throw)
                .with_target(Some(item_id))
                .with_tx(Some(target_position.x))
                .with_ty(Some(target_position.y))
                .with_update_interval(5)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(update)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }
        self.step_after_throw_attempt(ctx, item_id, p_grabbing, update)
    }

    fn step_after_throw_attempt(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        item_id: ObjectId,
        p_grabbing: Option<ObjectId>,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        let Some(container_snapshot) = ctx.resolve(self.container) else {
            return CommandStepResult::failed(update);
        };
        let target_definition = ctx.definition(&container_snapshot.definition_id);
        if target_definition
            .is_some_and(|definition| definition.grab_put_get & crate::GRAB_PUT_GET_PUT != 0)
        {
            if p_grabbing == Some(self.container) {
                self.put_pending = true;
                return CommandStepResult::running(update).with_events(vec![
                    CommandEvent::ObjectComPut {
                        actor_id: ctx.object.id,
                        target_id: self.container,
                        object_id: item_id,
                        ungrab_on_success: self.put_ty != 0,
                        command_instance_id: 0,
                    },
                ]);
            }

            self.put_ty = 1;
            let request = CommandRequest::new(CommandId::Grab)
                .with_target(Some(self.container))
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(update)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        if container_snapshot.ocf & ocf::ENTRANCE != 0 {
            let request = CommandRequest::new(CommandId::Enter)
                .with_target(Some(self.container))
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(update)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        CommandStepResult::running(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DropState {
    requested_item: Option<ObjectId>,
    target_position: Option<Vector2>,
    update_interval: u32,
    /// Legacy save fields from the former delegated-Put approximation.
    /// ObjectComPutTake is now executed inline, but retaining these fields
    /// keeps old command snapshots readable without a migration.
    #[serde(default)]
    delegated_put: bool,
    #[serde(default)]
    delegated_container: Option<ObjectId>,
    /// ObjectComDrop is running between the helper call and Finish(true).
    /// This identifies the exact command if callbacks push new entries.
    #[serde(default)]
    completion_pending: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    continuations: Vec<DropContinuation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DropContinuation {
    AfterObjectComStop,
}

impl DropState {
    fn from_request(request: &CommandRequest) -> Self {
        let tx = request.tx.unwrap_or(0);
        let ty = request.ty.unwrap_or(0);
        let target_position = (tx != 0 || ty != 0).then_some(Vector2::new(tx, ty));
        Self {
            requested_item: request.target,
            target_position,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            delegated_put: false,
            delegated_container: None,
            completion_pending: false,
            continuations: Vec::new(),
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
        }

        ctx.object.contents.iter().find_map(|object_id| {
            ctx.resolve(*object_id)
                .filter(|snapshot| snapshot.has_nonzero_status())
                .map(|snapshot| (*object_id, snapshot))
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if ctx.object.action_procedure == ActionProcedure::Dig {
            self.continuations
                .push(DropContinuation::AfterObjectComStop);
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopDrop {
                    object_id: ctx.object.id,
                    command_instance_id: 0,
                },
            ]);
        }

        self.step_after_object_com_stop(ctx)
    }

    fn resume_after_prelude(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        match self.continuations.pop() {
            Some(DropContinuation::AfterObjectComStop) => self.step_after_object_com_stop(ctx),
            None => CommandStepResult::running(None),
        }
    }

    fn step_after_object_com_stop(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        // Plain outside Drop preserves Action.ComDir. Only ObjectComStop's
        // ObjectActionStand and the targeted at-position branch stop it
        // (C4Command.cpp:988-1049).
        let mut update = None;

        // C4Command::Drop tests the raw Target link before dereferencing it.
        // An executing command may have detached from the linked stack before
        // AssignRemoval runs, so ClearPointers cannot null that retained
        // status-zero pointer (C4Command.cpp:998-1010).
        if let Some(item_id) = self
            .requested_item
            .filter(|item_id| !ctx.object.contents.contains(item_id))
        {
            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::Get)
                .with_target(Some(item_id))
                .with_update_interval(40)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        let item = self.resolve_item(ctx);
        // Older snapshots may already have a delegated Put child in front
        // of this Drop. Once that child moved the item to its recorded
        // target, preserve the old one-shot completion instead of fetching
        // the item back and attempting a second inline PutTake.
        if self.delegated_put
            && self.delegated_container.is_some()
            && item.is_some_and(|(_, item)| item.container == self.delegated_container)
        {
            return CommandStepResult::completed(update);
        }

        if let Some((item_id, _item_snapshot)) = item {
            if !ctx.object.contents.contains(&item_id) {
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Get)
                    .with_target(Some(item_id))
                    .with_update_interval(40)
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
        }

        if let Some(position) = self.target_position {
            // C4Command::Drop handles target coordinates before the
            // contained/pushing Put branches. A pusher always lets go first,
            // even when already inside the target range.
            if ctx.object.action_procedure == ActionProcedure::Push {
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }

            const DROP_RANGE_DEFAULT: i32 = 5;
            const DROP_RANGE_VERTICAL: i32 = 15;
            let drop_range = if ctx.object.move_to_range > 0 {
                ctx.object.move_to_range
            } else {
                DROP_RANGE_DEFAULT
            };
            let dx = position.x - ctx.position.x;
            let dy = position.y - ctx.position.y;

            if dx.abs() > drop_range || dy.abs() > DROP_RANGE_VERTICAL {
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(position.x))
                    .with_ty(Some(position.y))
                    .with_update_interval(20)
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            if ctx.object.command_direction != CommandDirection::Stop {
                update = Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop));
            }
        } else if let Some(container_id) = ctx.object.container {
            self.completion_pending = true;
            return CommandStepResult::running(update).with_events(vec![
                CommandEvent::ObjectComPutTake {
                    actor_id: ctx.object.id,
                    target_id: container_id,
                    requested_item: self.requested_item,
                    command: CommandId::Drop,
                    command_instance_id: 0,
                },
            ]);
        } else if ctx.object.action_procedure == ActionProcedure::Push {
            let Some(container_id) = ctx.object.action_target else {
                return CommandStepResult::completed(update);
            };
            self.completion_pending = true;
            return CommandStepResult::running(update).with_events(vec![
                CommandEvent::ObjectComPutTake {
                    actor_id: ctx.object.id,
                    target_id: container_id,
                    requested_item: self.requested_item,
                    command: CommandId::Drop,
                    command_instance_id: 0,
                },
            ]);
        }

        let Some((item_id, _)) = item else {
            return CommandStepResult::completed(update);
        };

        // C++ calls Finish(true) only after ObjectComDrop (including all
        // Exit/UnGrab callbacks) returns. The live event marks this Drop
        // finished afterward if callbacks left that exact command alive.
        self.completion_pending = true;
        CommandStepResult::running(update).with_events(vec![CommandEvent::ObjectComDrop {
            actor_id: ctx.object.id,
            object_id: item_id,
            command_instance_id: 0,
        }])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GetState {
    target: Option<ObjectId>,
    fallback_container: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    #[serde(default)]
    menu_identification: Option<i32>,
    /// Count-equivalent state retained for compatibility with older saves,
    /// which normalized C4Command::Tx values <= 1 to one.
    remaining: i32,
    /// Live raw C4Command::Tx get-count forwarded to side-move Jump. Older
    /// serialized multi-count states reconstruct it from `remaining`.
    #[serde(default)]
    jump_tx: i32,
    /// C4Command::Ty is an integer (default zero) and is copied to the
    /// side-move Jump even though Jump itself only consumes Tx.
    #[serde(default)]
    jump_ty: i32,
    update_interval: u32,
    #[serde(default, skip_serializing_if = "crate::is_false")]
    enter_pending: bool,
}

impl GetState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let menu_identification = match (&request.data, request.target) {
            (CommandData::Integer(1), Some(_)) => Some(13),
            (CommandData::Integer(2), Some(_)) => Some(18),
            _ => None,
        };
        let definition_id = menu_identification
            .is_none()
            .then(|| command_data_to_definition_id(&request.data))
            .flatten();
        if request.target.is_none() && definition_id.is_none() {
            return Err(CommandError::Unsupported);
        }
        let jump_tx = request.tx.unwrap_or(0);
        let remaining = jump_tx.max(1);
        Ok(Self {
            target: request.target,
            fallback_container: request.target2,
            definition_id,
            menu_identification,
            remaining,
            jump_tx,
            jump_ty: request.ty.unwrap_or(0),
            update_interval: positive_helper_interval_or_one(request.update_interval),
            enter_pending: false,
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
        let mut update = update.unwrap_or_default();
        update.command_direction = Some(CommandDirection::Stop);
        Some(update)
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.action_procedure != ActionProcedure::Dig {
            return None;
        }
        let mut update = self.ensure_stop(ctx, None).unwrap_or_default();
        let action_update = ActionUpdate::default()
            .with_name("Idle")
            .with_force(true)
            .with_phase(0)
            .with_ticks(0);
        update = update.with_action_update(action_update);
        Some(update)
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
                if item_snapshot.has_nonzero_status()
                    && item_snapshot.definition_id == definition_id
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
        stop_here: bool,
    ) -> CommandStepResult {
        let update = if stop_here {
            self.ensure_stop(ctx, update)
        } else {
            update
        };
        self.enter_pending = true;
        // Do not decrement `remaining` here. C++ only observes a successful
        // collection on the NEXT Get evaluation (Target->Contained == cObj,
        // C4Command.cpp:1154-1165). A RejectCollect/PutAway retry therefore
        // retains both Target and Tx exactly.
        CommandStepResult::running(update).with_events(vec![CommandEvent::GetObject {
            actor_id: ctx.object.id,
            object_id: target_id,
            command_instance_id: 0,
        }])
    }

    fn handle_container_target(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_id: ObjectId,
        target_snapshot: &CommandObjectSnapshot,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        let Some(container_id) = target_snapshot.container else {
            return CommandStepResult::failed(update);
        };

        if ctx
            .definition(target_snapshot.definition_id.as_str())
            .is_some_and(|definition| definition.no_get)
        {
            return CommandStepResult::running(update);
        }

        if ctx.object.container == Some(container_id) {
            return self.transfer_to_actor(ctx, target_id, update, false);
        }

        if let Some(current_container) = ctx.object.container {
            if current_container != container_id {
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Exit)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
        }

        let Some(container_snapshot) = ctx.resolve(container_id) else {
            return CommandStepResult::failed(update);
        };

        let grab_get = ctx
            .definition(container_snapshot.definition_id.as_str())
            .is_some_and(|definition| definition.grab_put_get & crate::GRAB_PUT_GET_GET != 0);
        if grab_get {
            if ctx.object.action_procedure == ActionProcedure::Push
                && ctx.object.action_target == Some(container_id)
            {
                return self.transfer_to_actor(ctx, target_id, update, false);
            }

            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::Grab)
                .with_target(Some(container_id))
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        if container_snapshot.ocf & ocf::ENTRANCE != 0 {
            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::Enter)
                .with_target(Some(container_id))
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        CommandStepResult::failed(update)
    }

    fn dig_out_position(landscape: &crate::Landscape, target_position: Vector2) -> Option<Vector2> {
        let mut staging_position =
            landscape.find_closest_free(target_position, -120, 120, -1, -1)?;
        if let Some(good_angle_position) =
            landscape.find_closest_free(target_position, -140, 140, -40, 40)
        {
            let closest_distance = math::integer_distance(
                target_position.x,
                target_position.y,
                staging_position.x,
                staging_position.y,
            );
            let good_angle_distance = math::integer_distance(
                target_position.x,
                target_position.y,
                good_angle_position.x,
                good_angle_position.y,
            );
            if good_angle_distance < 10 * closest_distance {
                staging_position = good_angle_position;
            }
        }
        Some(staging_position)
    }

    fn handle_in_solid_target(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_snapshot: &CommandObjectSnapshot,
    ) -> CommandStepResult {
        if ctx.object.container.is_some() {
            let mut result = CommandStepResult::running(None);
            let request = CommandRequest::new(CommandId::Exit)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        let Some(landscape) = ctx.landscape else {
            return CommandStepResult::failed(None);
        };
        let target_position = target_snapshot.position;
        let Some(staging_position) = Self::dig_out_position(landscape, target_position) else {
            return CommandStepResult::failed(None);
        };

        let dx = staging_position.x - ctx.position.x;
        let dy = staging_position.y - ctx.position.y;
        if dx.abs() > DIG_OUT_POSITION_RANGE || dy.abs() > DIG_OUT_POSITION_RANGE {
            let mut result = CommandStepResult::running(None);
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(staging_position.x))
                .with_ty(Some(staging_position.y))
                .with_update_interval(50);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        let mut result = CommandStepResult::running(None);
        let request = CommandRequest::new(CommandId::Dig)
            .with_tx(Some(target_position.x))
            .with_ty(Some(target_position.y + 4))
            .with_update_interval(50)
            .with_mode(CommandMode::SilentSub);
        result.operations.push(CommandOperation::PushFront(request));
        result
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if let (Some(identification), Some(container)) = (self.menu_identification, self.target) {
            let kind = if identification == 18 {
                MenuRequestKind::Contents { container }
            } else {
                MenuRequestKind::Get { container }
            };
            return CommandStepResult::completed(None).with_events(vec![CommandEvent::OpenMenu(
                MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind,
                },
            )]);
        }

        let update = self.prepare_update(ctx);

        let target_id = match self.resolve_target(ctx) {
            Some(id) => id,
            None => return CommandStepResult::failed(None),
        };

        let Some(target_snapshot) = ctx.resolve(target_id) else {
            return CommandStepResult::failed(None);
        };

        if !target_snapshot.collectible {
            return CommandStepResult::failed(None);
        }

        if target_snapshot.id == ctx.object.id {
            return CommandStepResult::failed(None);
        }

        if target_snapshot.container == Some(ctx.object.id) {
            if self.remaining > 1 {
                self.remaining -= 1;
                if self.jump_tx > 1 {
                    self.jump_tx -= 1;
                }
                self.target = None;
                return CommandStepResult::running(None);
            }
            return CommandStepResult::completed(self.ensure_stop(ctx, None));
        }

        if ctx.object.action_procedure == ActionProcedure::Push {
            if let Some(container_id) = target_snapshot.container {
                if ctx.object.action_target != Some(container_id) {
                    let mut result = CommandStepResult::running(update.clone());
                    let request = CommandRequest::new(CommandId::UnGrab)
                        .with_update_interval(50)
                        .with_mode(CommandMode::SilentSub);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
            } else {
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
        }

        if target_snapshot.container.is_some() {
            return self.handle_container_target(ctx, target_id, target_snapshot, update);
        }

        if target_snapshot.ocf & ocf::IN_SOLID != 0 {
            return self.handle_in_solid_target(ctx, target_snapshot);
        }

        if ctx.object.container.is_some() {
            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::Exit)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        let dx = target_snapshot.position.x - ctx.position.x;
        let dy = target_snapshot.position.y - ctx.position.y;
        const PICKUP_RANGE: i32 = 12;
        if dx.abs() <= PICKUP_RANGE && dy.abs() <= PICKUP_RANGE {
            return self.transfer_to_actor(ctx, target_id, update, true);
        }

        // C4Command::Get outside pursuit (C4Command.cpp:1267-1290).
        let mut result = CommandStepResult::running(update);
        let tx = target_snapshot.position.x;
        let ty = target_snapshot.position.y;

        // Target in jumping range above the clonk: try the side-move jump
        // (C4Command.cpp:1272-1287) — Random(2) picks the side (a synced
        // ledger draw).
        let above = ctx.position.y - ty;
        if (-10..=10).contains(&(ctx.position.x - tx)) && (30..=50).contains(&above) {
            if let Some(rng) = ctx.rng {
                let side = if rng.borrow_mut().random(2) != 0 {
                    -1
                } else {
                    1
                };
                let side_x = ctx.position.x + side * above;
                let path_clear = ctx.landscape.is_none_or(|landscape| {
                    landscape
                        .path_is_clear(Vector2::new(side_x, ctx.position.y), Vector2::new(tx, ty))
                });
                if path_clear {
                    let jump_tx = if self.jump_tx == 0 && self.remaining > 1 {
                        self.remaining
                    } else {
                        self.jump_tx
                    };
                    result.operations.push(CommandOperation::PushFront(
                        CommandRequest::new(CommandId::Jump)
                            .with_tx(Some(jump_tx))
                            .with_ty(Some(self.jump_ty)),
                    ));
                    let collection_limit = ctx
                        .definition(ctx.object.definition_id.as_str())
                        .map_or(0, |definition| definition.collection_limit);
                    let contents_count = ctx
                        .object
                        .contents
                        .iter()
                        .filter(|object_id| {
                            ctx.resolve(**object_id)
                                .is_some_and(CommandObjectSnapshot::has_nonzero_status)
                        })
                        .count();
                    let collection_limit_reached =
                        crate::collection_limit_reached(collection_limit, contents_count);
                    if collection_limit_reached {
                        result
                            .operations
                            .push(CommandOperation::PushFront(CommandRequest::new(
                                CommandId::Drop,
                            )));
                    }
                    result.operations.push(CommandOperation::PushFront(
                        CommandRequest::new(CommandId::MoveTo)
                            .with_tx(Some(side_x))
                            .with_ty(Some(ctx.position.y))
                            .with_update_interval(50),
                    ));
                }
            }
        }

        // Move to target with the random pickup offset (C4Command.cpp:1290):
        // MoveTo(Target->x + Random(15) - 7, Target->y, 25).
        let offset = ctx
            .rng
            .map(|rng| rng.borrow_mut().random(15) - 7)
            .unwrap_or(0);
        result.operations.push(CommandOperation::PushFront(
            CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(tx + offset))
                .with_ty(Some(ty))
                .with_update_interval(25),
        ));
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RetryState {
    remaining: u32,
}

impl RetryState {
    fn from_request(request: &CommandRequest) -> Self {
        Self {
            remaining: positive_helper_interval_or_one(request.update_interval),
        }
    }

    fn step(&mut self, _ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        CommandStepResult::running(None)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FollowState {
    target: Option<ObjectId>,
    update_interval: u32,
}

impl FollowState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            target: request.target,
            update_interval: positive_helper_interval_or_one(request.update_interval),
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let follower = ctx.object;

        if follower.crew_member && follower.owner != OWNER_NONE && !follower.selected {
            return CommandStepResult::completed(None);
        }

        let Some(target_id) = self.target else {
            return CommandStepResult::failed(None);
        };
        let target = match ctx.resolve(target_id) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(None),
        };

        if follower.container != target.container {
            if !follower.crew_member {
                return CommandStepResult::failed(None);
            }
            let request = if follower.container.is_some() {
                CommandRequest::new(CommandId::Exit)
            } else {
                CommandRequest::new(CommandId::Enter).with_target(target.container)
            }
            .with_update_interval(50);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        if target.action_procedure == ActionProcedure::Push {
            if follower.action_procedure == ActionProcedure::Push
                && follower.action_target == target.action_target
            {
                let update = (follower.command_direction != target.command_direction)
                    .then(|| ObjectUpdate::new().with_command_direction(target.command_direction));
                return CommandStepResult::running(update);
            }
            let request = CommandRequest::new(CommandId::Grab).with_target(target.action_target);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        } else if follower.action_procedure == ActionProcedure::Push {
            return CommandStepResult::running(None).with_operations(vec![
                CommandOperation::PushFront(CommandRequest::new(CommandId::UnGrab)),
            ]);
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

        let mut result = CommandStepResult::running(None);
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(target.position.x))
            .with_ty(Some(target.position.y))
            .with_update_interval(10);
        result = result.with_operations(vec![CommandOperation::PushFront(request)]);
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ThrowState {
    target: Option<ObjectId>,
    tx: Option<i32>,
    ty: Option<i32>,
    update_interval: u32,
    #[serde(default)]
    put_take_pending: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    continuations: Vec<ThrowContinuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    physical_continuation: Option<ThrowPhysicalContinuation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ThrowContinuation {
    AfterObjectComStop,
    AfterSetDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ThrowPhysicalContinuation {
    target_position: Vector2,
    preferred_direction: i32,
    horizontal_range: i32,
}

impl ThrowState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            target: request.target,
            tx: request.tx,
            ty: request.ty,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            put_take_pending: false,
            continuations: Vec::new(),
            physical_continuation: None,
        })
    }

    fn throw_position(&self) -> Option<Vector2> {
        let position = Vector2::new(self.tx.unwrap_or(0), self.ty.unwrap_or(0));
        (position != Vector2::ZERO).then_some(position)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        self.step_with_gravity(ctx, crate::PhysicsSettings::default().gravity_as_c4fixed())
    }

    fn step_with_gravity(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
    ) -> CommandStepResult {
        if ctx.object.action_procedure == ActionProcedure::Dig {
            // ObjectComStop is two callbackful SetAction operations (Idle,
            // then Walk), not a reducible Walk update. Resume this same
            // Throw only after both calls have returned.
            self.continuations
                .push(ThrowContinuation::AfterObjectComStop);
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComStopThrow {
                    object_id: ctx.object.id,
                    command_instance_id: 0,
                },
            ]);
        }

        self.step_after_object_com_stop(ctx, gravity)
    }

    fn step_after_object_com_stop(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
    ) -> CommandStepResult {
        if let Some(target_id) = self.target {
            if !ctx.object.contents.contains(&target_id) {
                let get_request = CommandRequest::new(CommandId::Get)
                    .with_target(Some(target_id))
                    .with_update_interval(40)
                    .with_mode(CommandMode::SilentSub);
                let mut result = CommandStepResult::running(None);
                result
                    .operations
                    .push(CommandOperation::PushFront(get_request));
                return result;
            }
        }

        let target_position = self.throw_position();
        const THROW_HORIZONTAL_RANGE_DEFAULT: i32 = 5;
        let horizontal_range = if ctx.object.move_to_range > 0 {
            ctx.object.move_to_range
        } else {
            THROW_HORIZONTAL_RANGE_DEFAULT
        };
        if ctx.object.action_procedure == ActionProcedure::Push && target_position.is_some() {
            let request = CommandRequest::new(CommandId::UnGrab)
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            let mut result = CommandStepResult::running(None);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        if let Some(target_position) = target_position {
            let preferred_direction = if ctx.position.x > target_position.x {
                -1
            } else {
                1
            };
            if ctx.object.physical_deferred {
                self.physical_continuation = Some(ThrowPhysicalContinuation {
                    target_position,
                    preferred_direction,
                    horizontal_range,
                });
                return resolve_command_physical(ctx.object.id, 1, None);
            }
            return self.step_after_physical(
                ctx,
                gravity,
                ctx.object.physical,
                target_position,
                preferred_direction,
                horizontal_range,
            );
        }

        // An untargeted Throw while contained is an inline put/take operation,
        // not the outside Throw action (C4Command.cpp:966-970).
        if target_position.is_none() {
            if let Some(container_id) = ctx.object.container {
                self.put_take_pending = true;
                return CommandStepResult::running(None).with_events(vec![
                    CommandEvent::ObjectComPutTake {
                        actor_id: ctx.object.id,
                        target_id: container_id,
                        requested_item: self.target,
                        command: CommandId::Throw,
                        command_instance_id: 0,
                    },
                ]);
            }
        }

        // Untargeted Throw while pushing is the grabbed-object twin of the
        // contained branch above: ObjectComPutTake uses Action.Target and the
        // command finishes without ungrabbing (C4Command.cpp:973-979).
        if target_position.is_none() && ctx.object.action_procedure == ActionProcedure::Push {
            let Some(container_id) = ctx.object.action_target else {
                return CommandStepResult::completed(None);
            };
            self.put_take_pending = true;
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::ObjectComPutTake {
                    actor_id: ctx.object.id,
                    target_id: container_id,
                    requested_item: self.target,
                    command: CommandId::Throw,
                    command_instance_id: 0,
                },
            ]);
        }

        self.step_object_com_throw(ctx, false)
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        let Some(continuation) = self.physical_continuation.take() else {
            return CommandStepResult::running(None);
        };
        self.step_after_physical(
            ctx,
            gravity,
            physical,
            continuation.target_position,
            continuation.preferred_direction,
            continuation.horizontal_range,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn step_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        physical: PhysicalInfo,
        target_position: Vector2,
        preferred_direction: i32,
        horizontal_range: i32,
    ) -> CommandStepResult {
        let throw_force = math::val_by_physical(400, physical.throw);
        let throwing_position = ctx.landscape.and_then(|landscape| {
            [preferred_direction, -preferred_direction]
                .into_iter()
                .find_map(|direction| {
                    landscape.find_throwing_position(
                        target_position,
                        FixedVec2::new(throw_force * direction, -throw_force),
                        ctx.object.shape_height,
                        gravity,
                    )
                })
        });
        let Some(throwing_position) = throwing_position else {
            return CommandStepResult::failed(None);
        };

        const THROW_VERTICAL_RANGE: i32 = 15;
        let dx = throwing_position.x - ctx.position.x;
        let dy = throwing_position.y - ctx.position.y;
        if dx.abs() > horizontal_range || dy.abs() > THROW_VERTICAL_RANGE {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(throwing_position.x))
                .with_ty(Some(throwing_position.y))
                .with_update_interval(20)
                .with_mode(CommandMode::SilentSub);
            let mut result = CommandStepResult::running(None);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        let direction = if target_position.x > ctx.position.x {
            Direction::Right
        } else {
            Direction::Left
        };
        self.continuations.push(ThrowContinuation::AfterSetDir);
        CommandStepResult::running(None).with_events(vec![CommandEvent::ObjectComSetDirThrow {
            object_id: ctx.object.id,
            direction,
            command_instance_id: 0,
        }])
    }

    fn resume_after_prelude(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
    ) -> CommandStepResult {
        match self.continuations.pop() {
            Some(ThrowContinuation::AfterObjectComStop) => {
                self.step_after_object_com_stop(ctx, gravity)
            }
            Some(ThrowContinuation::AfterSetDir) => self.step_object_com_throw(ctx, true),
            None => CommandStepResult::running(None),
        }
    }

    fn step_object_com_throw(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        targeted: bool,
    ) -> CommandStepResult {
        let update =
            targeted.then(|| ObjectUpdate::new().with_command_direction(CommandDirection::Stop));

        // ObjectActionThrow changes the Clonk action, then immediately exits
        // the selected (or first) content. Keep that ordered operation in the
        // engine: SetAction may reject the transition, and only a successful
        // transition consumes Random(360) (C4ObjectCom.cpp:120-137).
        // Target was verified before a possible SetDir callback. Keep that
        // explicit pointer even if the callback moved it out of Contents;
        // ClearPointers may instead have nulled it, in which case native
        // ObjectComThrow falls back to the current first content.
        let item_id = self.target.or_else(|| {
            ctx.object.contents.iter().copied().find(|id| {
                ctx.resolve(*id)
                    .is_some_and(CommandObjectSnapshot::has_nonzero_status)
            })
        });
        let Some(object_id) = item_id else {
            return if targeted {
                CommandStepResult::running(update)
            } else {
                CommandStepResult::completed(None)
            };
        };
        if ctx.object.action_procedure != ActionProcedure::Walk {
            return if targeted {
                CommandStepResult::running(update)
            } else {
                CommandStepResult::completed(None)
            };
        }
        let event = CommandEvent::ThrowObject {
            actor_id: ctx.object.id,
            object_id,
            complete_command_on_success: targeted,
            command_instance_id: 0,
        };
        CommandStepResult::running(update).with_events(vec![event])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AttackState {
    target: ObjectId,
    update_interval: u32,
}

impl AttackState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            update_interval: positive_helper_interval_or_one(request.update_interval),
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(target) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(None);
        };

        if target.ocf & ocf::CREW_MEMBER == 0 {
            return CommandStepResult::completed(None);
        }

        if let Some(projectile) = ctx.object.contents.iter().copied().find(|id| {
            ctx.resolve(*id)
                .and_then(|object| ctx.definition(&object.definition_id))
                .is_some_and(|definition| definition.projectile != 0)
        }) {
            let request = CommandRequest::new(CommandId::Throw)
                .with_target(Some(projectile))
                .with_tx(Some(target.position.x))
                .with_ty(Some(target.position.y))
                .with_update_interval(2);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        if ctx.object.container != target.container {
            let request = if ctx.object.container.is_some() {
                CommandRequest::new(CommandId::Exit).with_update_interval(10)
            } else {
                CommandRequest::new(CommandId::Enter)
                    .with_target(target.container)
                    .with_update_interval(10)
            };
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(target.position.x))
            .with_ty(Some(target.position.y))
            .with_update_interval(10);
        CommandStepResult::running(None).with_operations(vec![CommandOperation::PushFront(request)])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CallState {
    target: Option<ObjectId>,
    function: String,
    tx: Option<i32>,
    /// Exact C4Value payload. `tx`/`tx_definition` are compatibility
    /// projections for older snapshots and non-Call command consumers. An
    /// absent field means an older snapshot and is reconstructed from them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tx_value: Option<clonk_script::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tx_definition: Option<DefinitionId>,
    ty: Option<i32>,
    target2: Option<ObjectId>,
    /// C4Command::Data is independent of Call's `Text` function name.
    /// Script-created calls initialize it to zero; Objects.txt restores it.
    #[serde(default)]
    legacy_data: i32,
    executed: bool,
}

impl CallState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let function = match &request.data {
            CommandData::Text(text) => text.clone(),
            CommandData::Integer(_) | CommandData::None => String::new(),
        };
        Ok(Self {
            target: request.target,
            function,
            tx: request.tx,
            tx_value: request.tx_value.clone(),
            tx_definition: request.tx_definition.clone(),
            ty: request.ty,
            target2: request.target2,
            legacy_data: 0,
            executed: false,
        })
    }

    fn effective_tx_value(&self) -> clonk_script::Value {
        self.tx_value
            .clone()
            .or_else(|| {
                self.tx_definition
                    .as_ref()
                    .map(|value| clonk_script::Value::C4Id(value.clone()))
            })
            .or_else(|| self.tx.map(clonk_script::Value::Int))
            .unwrap_or(clonk_script::Value::Nil)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.executed {
            return CommandStepResult::completed(None);
        }

        self.executed = true;
        if self.function.is_empty() {
            return CommandStepResult::failed(None);
        }
        let Some(target) = self.target else {
            return CommandStepResult::failed(None);
        };
        if ctx.resolve(target).is_none() {
            return CommandStepResult::failed(None);
        }
        let event = CommandEvent::CallObjectFunction {
            object_id: target,
            function: self.function.clone(),
            caller: ctx.object.id,
            tx: self.tx,
            tx_value: Some(self.effective_tx_value()),
            tx_definition: self.tx_definition.clone(),
            ty: self.ty,
            target2: self.target2,
            on_result: None,
        };

        CommandStepResult::completed(None).with_events(vec![event])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ContextState {
    target: ObjectId,
    position: Option<Vector2>,
    executed: bool,
}

impl ContextState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target2.ok_or(CommandError::Unsupported)?;
        let position = match (request.tx, request.ty) {
            (Some(x), Some(y)) if x != 0 && y != 0 => Some(Vector2::new(x, y)),
            _ => None,
        };
        Ok(Self {
            target,
            position,
            executed: false,
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.executed {
            return CommandStepResult::completed(None);
        }
        self.executed = true;

        let Some(target_snapshot) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(None);
        };

        let mut update = None;
        if ctx.object.command_direction != CommandDirection::Stop {
            update = Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop));
        }

        let mut events = Vec::new();
        if ctx.object.owner != OWNER_NONE {
            events.push(CommandEvent::OpenMenu(MenuRequest {
                crew_id: ctx.object.id,
                owner: ctx.object.owner,
                kind: MenuRequestKind::Context {
                    target: self.target,
                    position: self.position,
                },
            }));
        }

        CommandStepResult::completed(update).with_events(events)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TakeState {
    executed: bool,
}

impl TakeState {
    fn from_request(_request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self { executed: false })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.executed {
            return CommandStepResult::completed(None);
        }
        self.executed = true;

        let event = CommandEvent::OpenMenu(MenuRequest {
            crew_id: ctx.object.id,
            owner: ctx.object.controller,
            kind: MenuRequestKind::Activate,
        });
        CommandStepResult::completed(None).with_events(vec![event])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Take2State {
    executed: bool,
}

impl Take2State {
    fn from_request(_request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self { executed: false })
    }

    fn update_to_stop(ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.executed {
            return CommandStepResult::completed(None);
        }
        self.executed = true;

        let Some(container_id) = ctx.object.container else {
            return CommandStepResult::completed(None);
        };
        let Some(container) = ctx.resolve(container_id) else {
            return CommandStepResult::completed(None);
        };

        let update = Self::update_to_stop(ctx);

        if ctx.object.owner == OWNER_NONE {
            return CommandStepResult::completed(update);
        }

        let event = CommandEvent::OpenMenu(MenuRequest {
            crew_id: ctx.object.id,
            owner: ctx.object.owner,
            kind: MenuRequestKind::Get {
                container: container_id,
            },
        });
        CommandStepResult::completed(update).with_events(vec![event])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub enum AcquireScriptResult {
    Continue,
    Handled,
    Complete,
    Failed,
}

impl AcquireScriptResult {
    pub(crate) fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Continue),
            1 => Some(Self::Handled),
            2 => Some(Self::Complete),
            3 => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AcquireState {
    target: Option<ObjectId>,
    definition_id: DefinitionId,
    ignore_container: Option<ObjectId>,
    range_x: i32,
    range_y: i32,
    update_interval: u32,
    buy_requested: bool,
    last_buy_request: Option<u64>,
    #[serde(default)]
    script_pending: bool,
    #[serde(default)]
    script_invoked: bool,
    #[serde(default)]
    script_result: Option<AcquireScriptResult>,
    /// C4CMD_Acquire defaults its ranges on an evaluation-only Execute.
    #[serde(default)]
    evaluation_pending: bool,
}

impl AcquireState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let definition_id = command_data_to_definition_id(&request.data).unwrap_or_default();
        // Fold the eventual InitEvaluation values into private state now;
        // evaluation_pending still keeps them out of the live command view
        // and prevents command work until the native evaluation Execute.
        let raw_range_x = request.tx.unwrap_or(0);
        let raw_range_y = request.ty.unwrap_or(0);
        let range_x = if !request.evaluated && raw_range_x == 0 {
            500
        } else {
            raw_range_x
        };
        let range_y = if !request.evaluated && raw_range_y == 0 {
            250
        } else {
            raw_range_y
        };
        Ok(Self {
            target: request.target,
            definition_id,
            ignore_container: request.target2,
            range_x,
            range_y,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            buy_requested: false,
            last_buy_request: None,
            script_pending: false,
            script_invoked: false,
            script_result: None,
            evaluation_pending: !request.evaluated,
        })
    }

    /// C4CMD_Acquire InitEvaluation (C4Command.cpp:1666-1670): only a ZERO
    /// range becomes 500/250. Negative ranges retain their sign and match no
    /// candidates in the later Inside checks.
    fn init_evaluation(&mut self) -> bool {
        if !self.evaluation_pending {
            return false;
        }
        self.evaluation_pending = false;
        if self.range_x == 0 {
            self.range_x = 500;
        }
        if self.range_y == 0 {
            self.range_y = 250;
        }
        true
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

    fn candidate_is_valid(
        &self,
        candidate: &CommandObjectSnapshot,
        ctx: &CommandRuntimeContext<'_>,
    ) -> bool {
        if candidate.destroyed || !candidate.status.is_active() {
            return false;
        }
        if candidate.definition_id != self.definition_id {
            return false;
        }
        if candidate.ocf & ocf::AVAILABLE == 0 {
            return false;
        }
        if candidate.ocf & ocf::FULL_CON == 0 {
            return false;
        }
        if let Some(ignore) = self.ignore_container {
            if candidate.container == Some(ignore) {
                return false;
            }
        }
        if candidate.on_fire {
            return false;
        }
        if ctx.objects.values().any(|pipe| {
            pipe.is_status_active()
                && pipe.ocf != 0
                && matches!(
                    pipe.definition_id.as_str(),
                    SOURCE_PIPE_DEFINITION | DRAIN_PIPE_DEFINITION
                )
                && pipe.action_name == CONNECT_ACTION
                && (pipe.action_target == Some(candidate.id)
                    || pipe.action_target2 == Some(candidate.id))
        }) {
            return false;
        }
        true
    }

    fn find_candidate(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        let mut best: Option<(ObjectId, i64, usize)> = None;
        for snapshot in ctx.objects.values() {
            if !self.candidate_is_valid(snapshot, ctx) {
                continue;
            }
            let dx = i64::from(snapshot.position.x) - i64::from(ctx.position.x);
            let dy = i64::from(snapshot.position.y) - i64::from(ctx.position.y);
            if dx.abs() > i64::from(self.range_x) || dy.abs() > i64::from(self.range_y) {
                continue;
            }
            let distance = dx * dx + dy * dy;
            if best.is_none_or(|(best_id, best_distance, best_order)| {
                (distance, snapshot.master_list_order, snapshot.id)
                    < (best_distance, best_order, best_id)
            }) {
                best = Some((snapshot.id, distance, snapshot.master_list_order));
            }
        }
        best.map(|(id, _, _)| id)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.definition_id.is_empty() {
            return CommandStepResult::failed(None);
        }

        let has_item = ctx
            .object
            .contents
            .iter()
            .filter_map(|id| ctx.resolve(*id))
            .any(|snapshot| {
                snapshot.has_nonzero_status() && snapshot.definition_id == self.definition_id
            });

        if has_item {
            self.script_pending = false;
            self.script_invoked = false;
            self.script_result = None;
            return CommandStepResult::completed(None);
        }

        if self.script_pending {
            if let Some(result) = self.script_result.take() {
                self.script_pending = false;
                match result {
                    AcquireScriptResult::Handled => {
                        self.script_invoked = false;
                        return CommandStepResult::running(None);
                    }
                    AcquireScriptResult::Complete => {
                        self.script_invoked = false;
                        return CommandStepResult::completed(None);
                    }
                    AcquireScriptResult::Failed => {
                        self.script_invoked = false;
                        return CommandStepResult::failed(None);
                    }
                    AcquireScriptResult::Continue => {
                        // proceed with default logic below
                    }
                }
            } else {
                return CommandStepResult::running(None);
            }
        }

        if !self.script_invoked {
            self.script_pending = true;
            self.script_invoked = true;
            let event = CommandEvent::ControlCommandAcquire {
                caller: ctx.object.id,
                target: self.target,
                range_x: self.range_x,
                range_y: self.range_y,
                ignore_container: self.ignore_container,
                definition_id: self.definition_id.clone(),
                command_instance_id: 0,
            };
            return CommandStepResult::running(None).with_events(vec![event]);
        }

        let Some(candidate_id) = self.find_candidate(ctx) else {
            self.maybe_reset_buy(ctx.frame);
            self.script_invoked = false;
            let mut result = CommandStepResult::running(None);
            if let Some(operation) = self.request_buy(ctx.frame) {
                result.operations.push(operation);
            }
            return result;
        };

        self.buy_requested = false;
        self.last_buy_request = None;
        self.script_invoked = false;
        let mut result = CommandStepResult::running(None);
        let request = CommandRequest::new(CommandId::Get)
            .with_target(Some(candidate_id))
            .with_update_interval(40)
            .with_mode(CommandMode::SilentSub);
        result.operations.push(CommandOperation::PushFront(request));
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SellState {
    definition_id: DefinitionId,
    target: Option<ObjectId>,
    preferred: Option<ObjectId>,
    remaining: i32,
    update_interval: u32,
    last_enter_request: Option<u64>,
    #[serde(default)]
    evaluation_pending: bool,
}

impl SellState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        // Data==0 is the internal "open C4MN_Sell" command
        // (C4Command.cpp:2052-2057); a nonzero ID performs a sale.
        let definition_id = command_data_to_definition_id(&request.data).unwrap_or_default();
        Ok(Self {
            definition_id,
            target: request.target,
            preferred: request.target2,
            remaining: request.tx.unwrap_or(0),
            update_interval: positive_helper_interval_or_one(request.update_interval),
            last_enter_request: None,
            evaluation_pending: false,
        })
    }

    fn should_issue_enter(&self, frame: u64) -> bool {
        const ENTER_COOLDOWN: u64 = 12;
        match self.last_enter_request {
            Some(last) => frame.saturating_sub(last) >= ENTER_COOLDOWN,
            None => true,
        }
    }

    fn resolve_base(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        if let Some(target) = self.target {
            // An explicit C4Command target bypasses FindBase. Data==0 opens
            // the menu before Base/hostility validation.
            return ctx.resolve(target).map(|_| target);
        }

        let owner = ctx.object.owner;
        let target = ctx
            .objects
            .values()
            .filter(|snapshot| snapshot.is_status_active() && snapshot.base == owner)
            .min_by_key(|snapshot| {
                (
                    c4_distance(
                        snapshot.position.x,
                        snapshot.position.y,
                        ctx.position.x,
                        ctx.position.y,
                    ),
                    snapshot.master_list_order,
                    snapshot.id,
                )
            })
            .map(|snapshot| snapshot.id);
        self.target = target;
        target
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.evaluation_pending {
            return CommandStepResult::running(None);
        }

        // C4Command::Sell applies the global gate before target resolution
        // and before its internal menu command.
        if !ctx.base_sell_enabled {
            return CommandStepResult::failed(None);
        }
        let Some(base_id) = self.resolve_base(ctx) else {
            return CommandStepResult::failed(None);
        };

        if self.definition_id.is_empty() {
            return CommandStepResult::completed(None).with_events(vec![CommandEvent::OpenMenu(
                MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind: MenuRequestKind::Sell { base: base_id },
                },
            )]);
        }

        let base_snapshot = match ctx.resolve(base_id) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(None),
        };

        let base_owner = base_snapshot.base;
        if ctx.player(base_owner).is_none() || ctx.players_hostile(ctx.object.owner, base_owner) {
            return CommandStepResult::failed(None);
        }

        if ctx.object.container != Some(base_id) {
            let mut result = CommandStepResult::running(None);
            if self.should_issue_enter(ctx.frame) {
                self.last_enter_request = Some(ctx.frame);
                let request = CommandRequest::new(CommandId::Enter)
                    .with_target(Some(base_id))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
                result.operations.push(CommandOperation::PushFront(request));
            }
            return result;
        }
        self.last_enter_request = None;

        self.evaluation_pending = true;
        CommandStepResult::running(None).with_events(vec![CommandEvent::EvaluateSell {
            actor_id: ctx.object.id,
            base_id,
            definition_id: self.definition_id.clone(),
            preferred: self.preferred,
            count: self.remaining,
        }])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BuyState {
    definition_id: DefinitionId,
    target: Option<ObjectId>,
    update_interval: u32,
    #[serde(default)]
    remaining_count: i32,
    #[serde(default)]
    evaluation_pending: bool,
}

impl BuyState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        // Data==0 is the internal "open C4MN_Buy" command
        // (C4Command.cpp:1999-2004); a nonzero ID performs a purchase.
        let definition_id = command_data_to_definition_id(&request.data).unwrap_or_default();
        Ok(Self {
            definition_id,
            target: request.target,
            update_interval: positive_helper_interval_or_one(request.update_interval),
            remaining_count: request.tx.unwrap_or(0),
            evaluation_pending: false,
        })
    }

    fn resolve_base(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        if let Some(target) = self.target {
            // Explicit C4Command targets bypass FindFriendlyBase. Data==0
            // opens the menu before Base/hostility validation.
            return ctx.resolve(target).map(|_| target);
        }

        let buyer_owner = ctx.object.owner;
        let target = ctx
            .objects
            .values()
            .filter(|snapshot| {
                snapshot.is_status_active()
                    && ctx.player(snapshot.base).is_some()
                    && !ctx.players_hostile(buyer_owner, snapshot.base)
            })
            .min_by_key(|snapshot| {
                (
                    c4_distance(
                        snapshot.position.x,
                        snapshot.position.y,
                        ctx.position.x,
                        ctx.position.y,
                    ),
                    snapshot.master_list_order,
                    snapshot.id,
                )
            })
            .map(|snapshot| snapshot.id);
        self.target = target;
        target
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.evaluation_pending {
            return CommandStepResult::running(None);
        }

        // C4Command::Buy applies the global gate to every path, including
        // explicit targets and menu commands.
        if !ctx.base_buy_enabled {
            return CommandStepResult::failed(None);
        }
        let Some(base_id) = self.resolve_base(ctx) else {
            return CommandStepResult::failed(None);
        };

        if self.definition_id.is_empty() {
            return CommandStepResult::completed(None).with_events(vec![CommandEvent::OpenMenu(
                MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind: MenuRequestKind::Buy { base: base_id },
                },
            )]);
        }

        let base_snapshot = match ctx.resolve(base_id) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(None),
        };

        let base_owner = base_snapshot.base;
        if base_owner == OWNER_NONE {
            return CommandStepResult::failed(None);
        }

        let base_player = match ctx.player(base_owner) {
            Some(player) => player,
            _ => return CommandStepResult::failed(None),
        };

        let buyer_owner = ctx.object.owner;
        if ctx.players_hostile(buyer_owner, base_owner) {
            return CommandStepResult::failed(None);
        }

        if ctx.definition(&self.definition_id).is_none() {
            return CommandStepResult::failed(None);
        }

        let available = base_player.material_count(&self.definition_id);
        if available <= 0 {
            return CommandStepResult::failed(None);
        }

        self.evaluation_pending = true;
        CommandStepResult::running(None).with_events(vec![CommandEvent::EvaluateBuy {
            actor_id: ctx.object.id,
            base_id,
            definition_id: self.definition_id.clone(),
            buyer: buyer_owner,
            payer: base_owner,
            count: self.remaining_count,
        }])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct HomeState {
    target: Option<ObjectId>,
    update_interval: u32,
}

impl HomeState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        Ok(Self {
            target: request.target,
            update_interval: positive_helper_interval_or_one(request.update_interval),
        })
    }

    fn is_base(snapshot: &CommandObjectSnapshot, owner: i32) -> bool {
        snapshot.is_status_active() && snapshot.base == owner
    }

    fn is_home(&self, ctx: &CommandRuntimeContext<'_>) -> bool {
        match ctx.object.container {
            Some(container_id) => ctx
                .resolve(container_id)
                .map(|snapshot| snapshot.base == ctx.object.owner)
                .unwrap_or(false),
            None => false,
        }
    }

    fn resolve_base(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        let owner = ctx.object.owner;
        if let Some(target_id) = self.target {
            // Explicit C4Command targets bypass FindBase entirely.
            return ctx.resolve(target_id).map(|_| target_id);
        }

        ctx.objects
            .values()
            .filter(|snapshot| Self::is_base(snapshot, owner))
            .min_by_key(|snapshot| {
                (
                    c4_distance(
                        snapshot.position.x,
                        snapshot.position.y,
                        ctx.position.x,
                        ctx.position.y,
                    ),
                    snapshot.master_list_order,
                    snapshot.id,
                )
            })
            .map(|snapshot| snapshot.id)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.is_home(ctx) {
            return CommandStepResult::completed(None);
        }

        let base_id = match self.resolve_base(ctx) {
            Some(id) => {
                self.target = Some(id);
                id
            }
            None => return CommandStepResult::failed(None),
        };

        let request = CommandRequest::new(CommandId::Enter).with_target(Some(base_id));
        CommandStepResult::running(None).with_operations(vec![CommandOperation::PushFront(request)])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EnergyState {
    target: ObjectId,
    #[serde(default)]
    source: Option<ObjectId>,
    #[serde(default)]
    linekit: Option<ObjectId>,
    #[serde(default)]
    line: Option<ObjectId>,
    #[serde(default)]
    line_spawn_requested: bool,
    #[serde(default)]
    acquire_requested: bool,
}

impl EnergyState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            source: request.target2,
            linekit: None,
            line: None,
            line_spawn_requested: false,
            acquire_requested: false,
        })
    }

    fn builder_linekit(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        ctx.object
            .contents
            .iter()
            .filter_map(|id| ctx.resolve(*id))
            .find(|snapshot| {
                snapshot.has_nonzero_status() && snapshot.definition_id == LINEKIT_DEFINITION
            })
            .map(|snapshot| snapshot.id)
    }

    fn target_has_power_line(&self, ctx: &CommandRuntimeContext<'_>, target_id: ObjectId) -> bool {
        ctx.objects.values().any(|snapshot| {
            snapshot.definition_id == POWERLINE_DEFINITION
                && snapshot.is_status_active()
                && snapshot.ocf != 0
                && !snapshot.action_idle
                && snapshot.action_name == CONNECT_ACTION
                && (snapshot.action_target == Some(target_id)
                    || snapshot.action_target2 == Some(target_id))
        })
    }

    fn carried_line(
        &self,
        ctx: &CommandRuntimeContext<'_>,
    ) -> Option<(ObjectId, ObjectId, Option<ObjectId>)> {
        for kit_id in &ctx.object.contents {
            let Some(kit) = ctx.resolve(*kit_id) else {
                continue;
            };
            if !kit.has_nonzero_status() || kit.definition_id != LINEKIT_DEFINITION {
                continue;
            }

            let Some(line) = ctx
                .objects
                .values()
                .filter(|snapshot| {
                    snapshot.definition_id == POWERLINE_DEFINITION
                        && snapshot.is_status_active()
                        && snapshot.ocf != 0
                        && !snapshot.action_idle
                        && snapshot.action_name == CONNECT_ACTION
                        && (snapshot.action_target == Some(*kit_id)
                            || snapshot.action_target2 == Some(*kit_id))
                })
                .min_by_key(|snapshot| (snapshot.master_list_order, snapshot.id))
            else {
                continue;
            };
            let far_endpoint = if line.action_target == Some(*kit_id) {
                line.action_target2
            } else {
                line.action_target
            };
            return Some((*kit_id, line.id, far_endpoint));
        }
        None
    }

    fn resolve_source(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_id: ObjectId,
    ) -> Option<ObjectId> {
        if let Some(source) = self.source {
            return ctx.resolve(source).map(|_| source);
        }
        let target = ctx.resolve(target_id)?;
        let source = ctx
            .objects
            .values()
            .filter(|snapshot| {
                snapshot.id != target_id
                    && snapshot.is_status_active()
                    && snapshot.ocf & ocf::POWER_SUPPLY != 0
            })
            .min_by_key(|snapshot| {
                let dx = i64::from(snapshot.position.x - target.position.x);
                let dy = i64::from(snapshot.position.y - target.position.y);
                (dx * dx + dy * dy, snapshot.master_list_order, snapshot.id)
            })?
            .id;
        self.source = Some(source);
        Some(source)
    }

    fn spawned_line(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        source: &CommandObjectSnapshot,
        linekit_id: ObjectId,
    ) -> Option<ObjectId> {
        ctx.objects
            .values()
            .filter(|snapshot| {
                snapshot.definition_id == POWERLINE_DEFINITION
                    && snapshot.is_status_active()
                    && snapshot.owner == ctx.object.owner
                    && snapshot.action_target == Some(source.id)
                    && snapshot.action_target2 == Some(linekit_id)
            })
            // Line definitions append at C++ Objects.Last, so the line just
            // returned by CreateLine has the greatest forward master rank.
            // Include both endpoints so a later line from the same supply
            // cannot be mistaken for this command's synchronous result.
            .max_by_key(|snapshot| (snapshot.master_list_order, snapshot.id))
            .map(|snapshot| snapshot.id)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let target_id = self.target;
        let Some(target_snapshot) = ctx.resolve(target_id) else {
            return CommandStepResult::failed(None);
        };

        if (target_snapshot.line_connect & LINE_CONNECT_POWER_INPUT) == 0 {
            return CommandStepResult::failed(None);
        }
        if !ctx.structures_need_energy
            || (!target_snapshot.need_energy && self.target_has_power_line(ctx, target_id))
        {
            return CommandStepResult::completed(None);
        }

        let Some(source_id) = self.resolve_source(ctx, target_id) else {
            return CommandStepResult::failed(None);
        };
        let Some(source_snapshot) = ctx.resolve(source_id) else {
            return CommandStepResult::failed(None);
        };
        if c4_distance(
            ctx.position.x,
            ctx.position.y,
            source_snapshot.position.x,
            source_snapshot.position.y,
        ) > 650
        {
            return CommandStepResult::failed(None);
        }
        if (source_snapshot.line_connect & crate::LINE_CONNECT_POWER_OUTPUT) == 0 {
            return CommandStepResult::failed(None);
        }

        let linekit_id = self
            .linekit
            .filter(|id| {
                ctx.object.contents.contains(id)
                    && ctx
                        .resolve(*id)
                        .is_some_and(CommandObjectSnapshot::has_nonzero_status)
            })
            .or_else(|| self.builder_linekit(ctx));
        let Some(mut linekit_id) = linekit_id else {
            if self.acquire_requested {
                return CommandStepResult::running(None);
            }

            let mut operations = Vec::new();
            if let Some(c4id) = definition_id_to_c4id(LINEKIT_DEFINITION) {
                let request = CommandRequest::new(CommandId::Acquire)
                    .with_data(CommandData::Integer(c4id))
                    .with_update_interval(ACQUIRE_REQUEST_INTERVAL);
                operations.push(CommandOperation::PushFront(request));
                self.acquire_requested = true;
            }
            return CommandStepResult::running(None).with_operations(operations);
        };
        self.linekit = Some(linekit_id);
        self.acquire_requested = false;

        let mut connection_source_id = Some(source_id);
        if let Some((connected_kit_id, line_id, far_endpoint)) = self.carried_line(ctx) {
            linekit_id = connected_kit_id;
            self.linekit = Some(connected_kit_id);
            self.line = Some(line_id);
            self.source = far_endpoint;
            self.line_spawn_requested = false;
            connection_source_id = far_endpoint;
        } else {
            // C++ keeps pLine local and repeats the carried-kit scan on every
            // Energy execution. Retain state only while waiting for the
            // deferred CreateLine event to materialize.
            self.line = None;
            if self.line_spawn_requested {
                if self
                    .spawned_line(ctx, source_snapshot, linekit_id)
                    .is_none()
                {
                    return CommandStepResult::running(None);
                }
                self.line_spawn_requested = false;
            }
        }

        if self.line.is_none() {
            if !source_snapshot.has_nonzero_status()
                || source_snapshot.container.is_some()
                || source_snapshot.ocf & ocf::ALL == 0
                || !source_snapshot.at_point(ctx.position.x, ctx.position.y)
            {
                let request = CommandRequest::new(CommandId::MoveTo)
                    .with_target(Some(source_id))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub);
                return CommandStepResult::running(None)
                    .with_operations(vec![CommandOperation::PushFront(request)]);
            }

            self.line_spawn_requested = true;
            return CommandStepResult::running(None).with_events(vec![CommandEvent::CreateLine {
                definition_id: POWERLINE_DEFINITION.into(),
                owner: ctx.object.owner,
                from: source_id,
                to: linekit_id,
            }]);
        }

        if !target_snapshot.has_nonzero_status()
            || target_snapshot.container.is_some()
            || target_snapshot.ocf & ocf::ALL == 0
            || !target_snapshot.at_point(ctx.position.x, ctx.position.y)
        {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(target_id))
                .with_update_interval(50)
                .with_mode(CommandMode::SilentSub);
            return CommandStepResult::running(None)
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        let line_id = self.line.expect("line is present");
        let mut action_update = ActionUpdate::default()
            .with_name(CONNECT_ACTION)
            .with_force(true)
            .with_target2(Some(target_id));
        if let Some(connection_source_id) = connection_source_id {
            action_update = action_update.with_target(Some(connection_source_id));
        }
        let line_update = ObjectUpdate::new().with_action_update(action_update);
        let linekit_update = ObjectUpdate::new()
            .clear_container()
            .with_status(ObjectStatus::Deleted)
            .with_alive(false);
        let update = (ctx.object.command_direction != CommandDirection::Stop)
            .then(|| ObjectUpdate::new().with_command_direction(CommandDirection::Stop));
        CommandStepResult::completed(update).with_events(vec![
            CommandEvent::ApplyObjectUpdate {
                object_id: line_id,
                update: line_update,
            },
            CommandEvent::ApplyObjectUpdate {
                object_id: linekit_id,
                update: linekit_update,
            },
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum CommandState {
    Follow(FollowState),
    MoveTo(MoveToState),
    Enter(EnterState),
    Exit(ExitState),
    Build(BuildState),
    Construct(ConstructState),
    Transfer(TransferState),
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
    Activate(ActivateState),
    PushTo(PushToState),
    Retry(RetryState),
    Attack(AttackState),
    Call(CallState),
    Context(ContextState),
    Buy(BuyState),
    Sell(SellState),
    Take(TakeState),
    Take2(Take2State),
    Acquire(AcquireState),
    Home(HomeState),
    Energy(EnergyState),
    /// A recognized command whose request lacked fields required by the
    /// typed Rust handler. Native C4Command still links such a node and lets
    /// its handler fail, so retain the command identity and creating request.
    Malformed(CommandId),
    Unsupported,
}

impl CommandState {
    fn id(&self) -> Option<CommandId> {
        match self {
            CommandState::Follow(_) => Some(CommandId::Follow),
            CommandState::MoveTo(_) => Some(CommandId::MoveTo),
            CommandState::Enter(_) => Some(CommandId::Enter),
            CommandState::Exit(_) => Some(CommandId::Exit),
            CommandState::Build(_) => Some(CommandId::Build),
            CommandState::Construct(_) => Some(CommandId::Construct),
            CommandState::Transfer(_) => Some(CommandId::Transfer),
            CommandState::Chop(_) => Some(CommandId::Chop),
            CommandState::Grab(_) => Some(CommandId::Grab),
            CommandState::Throw(_) => Some(CommandId::Throw),
            CommandState::UnGrab(_) => Some(CommandId::UnGrab),
            CommandState::Jump(_) => Some(CommandId::Jump),
            CommandState::Wait(_) => Some(CommandId::Wait),
            CommandState::Put(_) => Some(CommandId::Put),
            CommandState::Drop(_) => Some(CommandId::Drop),
            CommandState::Get(_) => Some(CommandId::Get),
            CommandState::Dig(_) => Some(CommandId::Dig),
            CommandState::Activate(_) => Some(CommandId::Activate),
            CommandState::PushTo(_) => Some(CommandId::PushTo),
            CommandState::Retry(_) => Some(CommandId::Retry),
            CommandState::Attack(_) => Some(CommandId::Attack),
            CommandState::Call(_) => Some(CommandId::Call),
            CommandState::Context(_) => Some(CommandId::Context),
            CommandState::Buy(_) => Some(CommandId::Buy),
            CommandState::Sell(_) => Some(CommandId::Sell),
            CommandState::Take(_) => Some(CommandId::Take),
            CommandState::Take2(_) => Some(CommandId::Take2),
            CommandState::Acquire(_) => Some(CommandId::Acquire),
            CommandState::Home(_) => Some(CommandId::Home),
            CommandState::Energy(_) => Some(CommandId::Energy),
            CommandState::Malformed(id) => Some(*id),
            CommandState::Unsupported => None,
        }
    }

    fn has_physical_continuation(&self) -> bool {
        match self {
            CommandState::MoveTo(state) => state.physical_continuation.is_some(),
            CommandState::Build(state) => state.physical_pending,
            CommandState::Construct(state) => state.physical_pending,
            CommandState::Chop(state) => state.physical_pending,
            CommandState::Throw(state) => state.physical_continuation.is_some(),
            CommandState::Put(state) => state.physical_continuation.is_some(),
            CommandState::PushTo(state) => state.physical_continuation.is_some(),
            _ => false,
        }
    }

    fn resume_after_physical(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        physical: PhysicalInfo,
    ) -> CommandStepResult {
        match self {
            CommandState::MoveTo(state) => state.resume_after_physical(ctx, physical),
            CommandState::Build(state) => state.resume_after_physical(ctx, physical),
            CommandState::Construct(state) => state.resume_after_physical(ctx, physical),
            CommandState::Chop(state) => state.resume_after_physical(ctx, physical),
            CommandState::Throw(state) => state.resume_after_physical(ctx, gravity, physical),
            CommandState::Put(state) => state.resume_after_physical(ctx, gravity, physical),
            CommandState::PushTo(state) => state.resume_after_physical(ctx, physical),
            _ => CommandStepResult::running(None),
        }
    }

    fn legacy_evaluated(&self, generic: bool) -> bool {
        match self {
            CommandState::MoveTo(state) => state.evaluated,
            CommandState::Exit(state) => state.evaluated,
            CommandState::PushTo(state) => !state.evaluation_pending,
            CommandState::Wait(state) => !state.evaluation_pending,
            CommandState::Acquire(state) => !state.evaluation_pending,
            _ => generic,
        }
    }

    fn legacy_path_checked(&self, generic: bool) -> bool {
        match self {
            CommandState::MoveTo(state) => state.path_checked,
            _ => generic,
        }
    }

    fn restore_legacy_evaluation(&mut self, evaluated: bool, path_checked: bool) {
        match self {
            CommandState::MoveTo(state) => {
                state.evaluated = evaluated;
                state.path_checked = path_checked;
            }
            CommandState::Exit(state) => state.evaluated = evaluated,
            CommandState::PushTo(state) => state.evaluation_pending = !evaluated,
            CommandState::Wait(state) => state.evaluation_pending = !evaluated,
            CommandState::Acquire(state) => state.evaluation_pending = !evaluated,
            _ => {}
        }
    }

    /// Child-command latches are meaningful only while that child sits
    /// above its parent. Whenever the parent itself executes, the child has
    /// completed, failed, or could not be pushed (for example StackFull), so
    /// retrying must start from an unlatched state like native C4Command.
    fn clear_child_command_latches(&mut self) {
        match self {
            CommandState::Dig(state) => {
                state.ungrab_requested = false;
                state.exit_requested = false;
            }
            CommandState::Activate(state) => {
                state.exit_requested = false;
                state.enter_requested = false;
            }
            CommandState::Acquire(state) => {
                state.buy_requested = false;
                state.last_buy_request = None;
            }
            CommandState::Energy(state) => {
                state.acquire_requested = false;
            }
            _ => {}
        }
    }

    /// Live C4Command-field overrides for the FnGetCommand view
    /// (C4Script.cpp:926-945 reads the LIVE fields): only the states
    /// whose C++ counterpart rewrites Target/Tx/Ty after Set do so —
    /// MoveTo's InitEvaluation absorption/adjust (C4Command.cpp:
    /// 1634-1643), Acquire's 500/250 range defaults (:1666-1670) and
    /// Construct's found-site write (:1757-1766), plus Put's resolved
    /// Target2, remaining Tx count, and internal Ty reminder flag
    /// (:1384-1504).
    fn apply_live_overrides(&self, view: &mut CommandView) {
        match self {
            CommandState::MoveTo(state) => {
                view.target = state.target;
                view.tx = state.tx;
                view.ty = state.ty;
            }
            CommandState::PushTo(state) => {
                view.tx = state.tx;
                view.ty = state.ty;
            }
            CommandState::Acquire(state) if !state.evaluation_pending => {
                view.target = state.target;
                view.tx = Some(state.range_x);
                view.ty = Some(state.range_y);
                view.target2 = state.ignore_container;
            }
            CommandState::Construct(state) => {
                view.target = state.target;
                view.target2 = state.target2;
                if let Some(site) = state.site {
                    view.tx = Some(site.x);
                    view.ty = Some(site.y);
                }
            }
            CommandState::Transfer(state) => {
                view.tx = state.tx;
                view.tx_value = Some(state.effective_tx_value());
                view.tx_definition = state.tx_definition.clone();
            }
            CommandState::Put(state) => {
                view.tx = (state.remaining_count != 0).then_some(state.remaining_count);
                view.target2 = state.requested_item;
                if state.put_ty != 0 {
                    view.ty = Some(state.put_ty);
                }
            }
            CommandState::Buy(state) => {
                view.target = state.target;
                view.tx = (state.remaining_count != 0).then_some(state.remaining_count);
            }
            CommandState::Sell(state) => {
                view.target = state.target;
                view.target2 = state.preferred;
                view.tx = (state.remaining != 0).then_some(state.remaining);
            }
            CommandState::Call(state) => {
                view.target = state.target;
                view.tx = state.tx;
                view.tx_value = Some(state.effective_tx_value());
                view.tx_definition = state.tx_definition.clone();
                view.target2 = state.target2;
                view.legacy_data = Some(state.legacy_data);
            }
            _ => {}
        }
    }

    /// Clear nullable state copies of C4Command's object fields as part of
    /// the post-load pointer pass. Required state IDs remain harmless when
    /// absent because command execution resolves them through the same live
    /// object table before use; the creating request above is the canonical
    /// FnGetCommand field view.
    fn denumerate_object_references(
        &mut self,
        object_numbers: &HashSet<u64>,
        denumerate_values: bool,
    ) {
        match self {
            CommandState::Follow(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
            }
            CommandState::MoveTo(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
            }
            CommandState::Enter(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
            }
            CommandState::Transfer(state) if denumerate_values => {
                if let Some(value) = &mut state.tx_value {
                    *value = crate::denumerate_script_value(value, object_numbers);
                }
            }
            CommandState::Construct(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.target2, object_numbers);
                denumerate_object_reference(&mut state.construction_id, object_numbers);
            }
            CommandState::Activate(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.container, object_numbers);
            }
            CommandState::PushTo(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.container, object_numbers);
            }
            CommandState::Put(state) => {
                denumerate_object_reference(&mut state.requested_item, object_numbers);
            }
            CommandState::Drop(state) => {
                denumerate_object_reference(&mut state.requested_item, object_numbers);
                denumerate_object_reference(&mut state.delegated_container, object_numbers);
            }
            CommandState::Get(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.fallback_container, object_numbers);
            }
            CommandState::Throw(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
            }
            CommandState::Call(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.target2, object_numbers);
                if denumerate_values {
                    if let Some(value) = &mut state.tx_value {
                        *value = crate::denumerate_script_value(value, object_numbers);
                    }
                }
            }
            CommandState::Acquire(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.ignore_container, object_numbers);
            }
            CommandState::Sell(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.preferred, object_numbers);
            }
            CommandState::Buy(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
            }
            CommandState::Home(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
            }
            CommandState::Energy(state) => {
                denumerate_object_reference(&mut state.source, object_numbers);
                denumerate_object_reference(&mut state.linekit, object_numbers);
                denumerate_object_reference(&mut state.line, object_numbers);
            }
            _ => {}
        }
    }

    fn clear_object_reference(&mut self, removed: ObjectId) -> bool {
        let clear =
            |reference: &mut Option<ObjectId>| clear_matching_object_reference(reference, removed);
        match self {
            CommandState::Follow(state) => clear(&mut state.target),
            CommandState::MoveTo(state) => clear(&mut state.target),
            CommandState::Enter(state) => clear(&mut state.target),
            CommandState::Build(state) => {
                clear_required_object_reference(&mut state.target, removed)
            }
            CommandState::Transfer(state) => {
                let tx_changed = state
                    .tx_value
                    .as_mut()
                    .is_some_and(|value| clear_value_object_reference(value, removed));
                clear_required_object_reference(&mut state.target, removed) | tx_changed
            }
            CommandState::Chop(state) => {
                clear_required_object_reference(&mut state.target, removed)
            }
            CommandState::Grab(state) if state.target == removed => {
                let changed = !state.target_cleared;
                state.target_cleared = true;
                changed
            }
            CommandState::Construct(state) => {
                clear(&mut state.target)
                    | clear(&mut state.target2)
                    | clear(&mut state.construction_id)
            }
            CommandState::Activate(state) => clear(&mut state.target) | clear(&mut state.container),
            CommandState::PushTo(state) => clear(&mut state.target) | clear(&mut state.container),
            CommandState::Put(state) => {
                let mut changed = clear_required_object_reference(&mut state.container, removed)
                    | clear(&mut state.requested_item);
                if let Some(continuation) = &mut state.stop_continuation {
                    changed |= clear_required_object_reference(&mut continuation.item_id, removed);
                }
                if let Some(continuation) = &mut state.physical_continuation {
                    changed |= clear_required_object_reference(&mut continuation.item_id, removed);
                    changed |= clear(&mut continuation.p_grabbing);
                }
                changed
            }
            CommandState::Drop(state) => {
                clear(&mut state.requested_item) | clear(&mut state.delegated_container)
            }
            CommandState::Get(state) => {
                clear(&mut state.target) | clear(&mut state.fallback_container)
            }
            CommandState::Throw(state) => clear(&mut state.target),
            CommandState::Attack(state) => {
                clear_required_object_reference(&mut state.target, removed)
            }
            CommandState::Call(state) => {
                let tx_changed = state
                    .tx_value
                    .as_mut()
                    .is_some_and(|value| clear_value_object_reference(value, removed));
                clear(&mut state.target) | clear(&mut state.target2) | tx_changed
            }
            CommandState::Acquire(state) => {
                clear(&mut state.target) | clear(&mut state.ignore_container)
            }
            CommandState::Sell(state) => clear(&mut state.target) | clear(&mut state.preferred),
            CommandState::Buy(state) => clear(&mut state.target),
            CommandState::Home(state) => clear(&mut state.target),
            CommandState::Energy(state) => {
                clear_required_object_reference(&mut state.target, removed)
                    | clear(&mut state.source)
                    | clear(&mut state.linekit)
                    | clear(&mut state.line)
            }
            CommandState::Context(state) => {
                clear_required_object_reference(&mut state.target, removed)
            }
            _ => false,
        }
    }

    /// Remaining interval stored by snapshots written before the counter
    /// moved to ActiveCommand. Most old states only kept the original
    /// polling interval; MoveTo, Wait and Retry actually counted it down.
    fn legacy_update_interval(&self) -> u32 {
        match self {
            CommandState::MoveTo(state) => state.update_interval,
            CommandState::Enter(state) => state.update_interval,
            CommandState::Exit(state) => state.update_interval,
            CommandState::Construct(state) => state.update_interval,
            CommandState::Chop(state) => state.update_interval,
            CommandState::Grab(state) => state.update_interval,
            CommandState::Throw(state) => state.update_interval,
            CommandState::UnGrab(state) => state.update_interval,
            CommandState::Wait(state) => positive_helper_interval(state.remaining.unwrap_or(0)),
            CommandState::Put(state) => state.update_interval,
            CommandState::Drop(state) => state.update_interval,
            CommandState::Get(state) => state.update_interval,
            CommandState::Dig(state) => state.update_interval,
            CommandState::Activate(state) => state.update_interval,
            CommandState::PushTo(state) => state.update_interval,
            CommandState::Retry(state) => state.remaining,
            CommandState::Follow(state) => state.update_interval,
            CommandState::Attack(state) => state.update_interval,
            CommandState::Buy(state) => state.update_interval,
            CommandState::Sell(state) => state.update_interval,
            CommandState::Acquire(state) => state.update_interval,
            CommandState::Home(state) => state.update_interval,
            _ => 0,
        }
    }
}

fn denumerate_object_reference(reference: &mut Option<ObjectId>, object_numbers: &HashSet<u64>) {
    if reference.is_some_and(|id| !object_numbers.contains(&id.as_u64())) {
        *reference = None;
    }
}

fn clear_matching_object_reference(reference: &mut Option<ObjectId>, removed: ObjectId) -> bool {
    if *reference == Some(removed) {
        *reference = None;
        true
    } else {
        false
    }
}

/// Required typed-state fields mirror nullable native Target/Target2 slots.
/// Object id zero is reserved and resolves to no object, so it preserves the
/// command variant/continuation while representing C4Command::ClearPointers'
/// null write. The creating request is cleared separately for GetCommand.
fn clear_required_object_reference(reference: &mut ObjectId, removed: ObjectId) -> bool {
    if *reference == removed {
        *reference = ObjectId::new(0);
        true
    } else {
        false
    }
}

fn clear_value_object_reference(value: &mut clonk_script::Value, removed: ObjectId) -> bool {
    match value {
        clonk_script::Value::Object(id) if ObjectId::new(*id) == removed => {
            *value = clonk_script::Value::Nil;
            true
        }
        clonk_script::Value::Array(values) => values.iter_mut().fold(false, |changed, value| {
            clear_value_object_reference(value, removed) | changed
        }),
        clonk_script::Value::Proplist(entries) => {
            let previous = std::mem::take(entries);
            let mut changed = false;
            for (mut key, mut value) in previous {
                if matches!(&key, clonk_script::Value::Object(id) if ObjectId::new(*id) == removed)
                    || matches!(&value, clonk_script::Value::Object(id) if ObjectId::new(*id) == removed)
                {
                    changed = true;
                    continue;
                }
                changed |= clear_value_object_reference(&mut key, removed);
                changed |= clear_value_object_reference(&mut value, removed);
                entries.insert_key(key, value);
            }
            changed
        }
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ActiveCommand {
    #[serde(skip)]
    instance_id: u64,
    state: CommandState,
    mode: CommandMode,
    retries: i32,
    failures: i32,
    /// Generic C4Command::Evaluated flag for commands whose typed state does
    /// not otherwise need to retain the InitEvaluation latch.
    evaluated: bool,
    /// Generic C4Command::PathChecked flag for command kinds without a
    /// typed pathfinder state.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    path_checked: bool,
    /// Exact persisted C4Command::Permit word.
    #[serde(default, skip_serializing_if = "crate::i32_is_zero")]
    permit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_evaluated_word: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_path_checked_word: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_finished_word: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_text: Option<String>,
    /// C4Command::UpdateInterval: a per-front-execution lifetime, not a
    /// wall-clock polling cadence (C4Command.cpp:1545-1552).
    update_interval: i32,
    /// The creating request, the FnGetCommand element-view base
    /// (C4Script.cpp:926-945); persisted through CommandSnapshot so
    /// restored stacks keep their elements.
    request: Option<CommandRequest>,
    finished: Option<CommandStatus>,
}

impl ActiveCommand {
    fn from_request(request: CommandRequest) -> Result<Self, CommandError> {
        let state = (|| -> Result<CommandState, CommandError> {
            Ok(match request.id {
                CommandId::Follow => CommandState::Follow(FollowState::from_request(&request)?),
                CommandId::MoveTo => CommandState::MoveTo(MoveToState::from_request(&request)),
                CommandId::Enter => CommandState::Enter(EnterState::from_request(&request)?),
                CommandId::Exit => CommandState::Exit(ExitState::from_request(&request)?),
                CommandId::Build => CommandState::Build(BuildState::from_request(&request)?),
                CommandId::Construct => {
                    CommandState::Construct(ConstructState::from_request(&request))
                }
                CommandId::Transfer => {
                    CommandState::Transfer(TransferState::from_request(&request)?)
                }
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
                CommandId::Activate => {
                    CommandState::Activate(ActivateState::from_request(&request)?)
                }
                CommandId::PushTo => CommandState::PushTo(PushToState::from_request(&request)?),
                CommandId::Retry => CommandState::Retry(RetryState::from_request(&request)),
                CommandId::Attack => CommandState::Attack(AttackState::from_request(&request)?),
                CommandId::Call => CommandState::Call(CallState::from_request(&request)?),
                CommandId::Context => CommandState::Context(ContextState::from_request(&request)?),
                CommandId::Buy => CommandState::Buy(BuyState::from_request(&request)?),
                CommandId::Sell => CommandState::Sell(SellState::from_request(&request)?),
                CommandId::Take => CommandState::Take(TakeState::from_request(&request)?),
                CommandId::Take2 => CommandState::Take2(Take2State::from_request(&request)?),
                CommandId::Acquire => CommandState::Acquire(AcquireState::from_request(&request)?),
                CommandId::Home => CommandState::Home(HomeState::from_request(&request)?),
                CommandId::Energy => CommandState::Energy(EnergyState::from_request(&request)?),
                _ => CommandState::Malformed(request.id),
            })
        })();
        let state = match state {
            Ok(state) => state,
            Err(CommandError::Unsupported) => CommandState::Malformed(request.id),
            Err(error) => return Err(error),
        };

        if matches!(state, CommandState::Unsupported) {
            return Err(CommandError::Unsupported);
        }

        // C4Command::Execute decrements the raw interval before
        // InitEvaluation. Wait's Data/Tx override is installed only on that
        // evaluation frame below; folding it in here would lose one tick.
        let update_interval = request.update_interval;

        Ok(Self {
            instance_id: 0,
            state,
            mode: request.mode,
            retries: request.retries.max(0),
            failures: 0,
            evaluated: request.evaluated,
            path_checked: false,
            permit: 0,
            legacy_evaluated_word: None,
            legacy_path_checked_word: None,
            legacy_finished_word: None,
            legacy_text: None,
            update_interval,
            request: Some(request),
            finished: None,
        })
    }

    fn from_snapshot(snapshot: CommandSnapshot) -> Self {
        let CommandSnapshot {
            instance_id,
            state,
            mode,
            retries,
            failures,
            evaluated,
            path_checked,
            permit,
            legacy_evaluated_word,
            legacy_path_checked_word,
            legacy_finished_word,
            legacy_text,
            update_interval,
            request,
            finished,
        } = snapshot;
        let remaining = update_interval.unwrap_or_else(|| {
            if matches!(
                state,
                CommandState::MoveTo(_) | CommandState::Wait(_) | CommandState::Retry(_)
            ) {
                i32::try_from(state.legacy_update_interval()).unwrap_or(i32::MAX)
            } else {
                request.as_ref().map_or_else(
                    || i32::try_from(state.legacy_update_interval()).unwrap_or(i32::MAX),
                    |request| request.update_interval,
                )
            }
        });
        Self {
            instance_id,
            state,
            mode,
            retries,
            failures,
            evaluated,
            path_checked,
            permit,
            legacy_evaluated_word,
            legacy_path_checked_word,
            legacy_finished_word,
            legacy_text,
            update_interval: remaining,
            request,
            finished,
        }
    }

    fn id(&self) -> Option<CommandId> {
        self.state.id()
    }

    fn step(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        gravity: crate::C4Fixed,
        next_is_move_to: bool,
    ) -> CommandStepResult {
        self.state.clear_child_command_latches();

        if self.failures > 0 {
            if self.retries > 0 {
                self.failures = 0;
                self.retries -= 1;
                let request = CommandRequest::new(CommandId::Retry)
                    .with_update_interval(10)
                    .with_mode(CommandMode::SilentSub);
                let mut result = CommandStepResult::running(None);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            self.failures = 0;
            return CommandStepResult::failed(None);
        }

        // C4Command::Execute decrements this before InitEvaluation and the
        // handler. Expiry is ordinary success and performs no command work.
        if self.update_interval > 0 {
            self.update_interval -= 1;
            if self.update_interval == 0 {
                return CommandStepResult::completed(None);
            }
        }

        // C4Command::InitEvaluation sets this for every command, including
        // commands with no special evaluation body.
        self.evaluated = true;

        // C4Command::InitEvaluation runs after the interval decrement and
        // consumes the Execute without invoking the command handler.
        if let CommandState::PushTo(state) = &mut self.state {
            if let Some(mut result) = state.init_evaluation(ctx) {
                stamp_command_event_instances(&mut result.events, self.instance_id);
                return result;
            }
        }
        if let CommandState::Wait(state) = &mut self.state {
            if state.evaluation_pending {
                state.evaluation_pending = false;
                if state.evaluation_overrides_interval {
                    self.update_interval = state.remaining.unwrap_or(0);
                }
                return CommandStepResult::running(None);
            }
        }
        if let CommandState::Acquire(state) = &mut self.state {
            if state.init_evaluation() {
                return CommandStepResult::running(None);
            }
        }

        let mut result = match &mut self.state {
            CommandState::Follow(state) => state.step(ctx),
            CommandState::MoveTo(state) => {
                let mut result = state.step_with_waypoint(ctx, next_is_move_to);
                if let Some(snapshot) = state.pathfinder_debug_update.take() {
                    result
                        .events
                        .insert(0, CommandEvent::SetPathFinderDebug { snapshot });
                }
                if let Some((level, transfer_zones_enabled)) =
                    state.pathfinder_settings_update.take()
                {
                    result.events.insert(
                        0,
                        CommandEvent::SetPathFinderSettings {
                            level,
                            transfer_zones_enabled,
                        },
                    );
                }
                result
            }
            CommandState::Enter(state) => state.step(ctx),
            CommandState::Exit(state) => state.step(ctx),
            CommandState::Build(state) => state.step(ctx),
            CommandState::Construct(state) => state.step(ctx),
            CommandState::Transfer(state) => state.step(ctx),
            CommandState::Chop(state) => state.step(ctx),
            CommandState::Grab(state) => state.step(ctx),
            CommandState::Throw(state) => state.step_with_gravity(ctx, gravity),
            CommandState::UnGrab(state) => state.step(ctx),
            CommandState::Jump(state) => state.step(ctx),
            CommandState::Wait(state) => state.step(ctx),
            CommandState::Put(state) => state.step_with_gravity(ctx, gravity),
            CommandState::Drop(state) => state.step(ctx),
            CommandState::Get(state) => state.step(ctx),
            CommandState::Dig(state) => state.step(ctx),
            CommandState::Activate(state) => state.step(ctx),
            CommandState::PushTo(state) => state.step(ctx),
            CommandState::Retry(state) => state.step(ctx),
            CommandState::Attack(state) => state.step(ctx),
            CommandState::Call(state) => state.step(ctx),
            CommandState::Context(state) => state.step(ctx),
            CommandState::Buy(state) => state.step(ctx),
            CommandState::Sell(state) => state.step(ctx),
            CommandState::Take(state) => state.step(ctx),
            CommandState::Take2(state) => state.step(ctx),
            CommandState::Acquire(state) => state.step(ctx),
            CommandState::Home(state) => state.step(ctx),
            CommandState::Energy(state) => state.step(ctx),
            CommandState::Malformed(_) => CommandStepResult::failed(None),
            CommandState::Unsupported => CommandStepResult::failed(None),
        };
        stamp_command_event_instances(&mut result.events, self.instance_id);
        result
    }
}
