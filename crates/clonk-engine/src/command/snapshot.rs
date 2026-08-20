//! `command` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

/// The per-frame `ObjectId -> CommandObjectSnapshot` table. It is only ever
/// probed by key: every consumer that ranks its contents sorts on an explicit
/// total order (`master_list_order` then `ObjectId`), so the fixed-seed hasher
/// changes nothing but the cost of a probe.
pub type CommandObjectSnapshots = rustc_hash::FxHashMap<ObjectId, CommandObjectSnapshot>;

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
    pub(crate) fn raw_shape_rect(&self) -> DefinitionRect {
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
