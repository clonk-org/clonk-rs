use std::collections::{HashMap, HashSet, VecDeque};

use crate::math::{self, FixedVec2};
use crate::pathfinder::PathFinder;
use crate::transfer::{TransferZone, TransferZoneTable};
use crate::{
    ocf, ActionProcedure, ActionUpdate, CommandDirection, DefinitionId, DefinitionRect, Direction,
    ObjectId, ObjectStatus, ObjectUpdate, PlayerStatus, Vector2, CATEGORY_OBJECT,
    CATEGORY_STATIC_BACK, CATEGORY_STRUCTURE, CATEGORY_VEHICLE, FULL_CON,
    LINE_CONNECT_POWER_INPUT, OWNER_NONE,
};
use lc_resources::PhysicalInfo;
use serde::{Deserialize, Serialize};

/// Maximum number of commands that may be queued for an object.
pub const MAX_COMMAND_STACK: usize = 35;
const LINEKIT_DEFINITION: &str = "LNKT";
const POWERLINE_DEFINITION: &str = "PWRL";
const CONKIT_DEFINITION: &str = "CNKT";
const ACQUIRE_REQUEST_INTERVAL: u32 = 50;
const COMMAND_FLAG_ENTER_PUSH_TARGET: i32 = 0b10;
const COMMAND_FLAG_MOVE_TO_NO_POS_ADJUST: i32 = 0b1;
const COMMAND_FLAG_MOVE_TO_PUSH_TARGET: i32 = 0b10;
const DIG_MOVE_TO_RANGE_DEFAULT: i32 = 5;
const DIG_DIRECTION_RANGE: i32 = 1;
const CATEGORY_SELECT_KNOWLEDGE: i32 = 1 << 10;
const PUSH_TO_RANGE: i32 = 10;

#[derive(Debug, Clone)]
pub struct CommandObjectSnapshot {
    pub id: ObjectId,
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
    pub status: ObjectStatus,
    pub destroyed: bool,
    pub category: i32,
    pub container: Option<ObjectId>,
    pub action_target: Option<ObjectId>,
    pub action_procedure: ActionProcedure,
    pub command_direction: CommandDirection,
    pub construction: i32,
    /// Facing (C4Object Action.Dir) for ComDir-less jump direction.
    pub direction: Direction,
    /// The resolved GetPhysical view (temporary→info→definition).
    pub physical: PhysicalInfo,
    pub owner: i32,
    /// C4Object::Controller, used when commands arm work on a target.
    pub controller: i32,
    /// C4Object::Base: the player whose home-base material and wealth this
    /// object brokers, independently of the object's owner.
    pub base: i32,
    pub crew_member: bool,
    pub selected: bool,
    pub alive: bool,
    pub contents: Vec<ObjectId>,
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
    pub action_time: u32,
    /// Current shape top (C4Object Shape.y) for the top-free scans
    /// (C4Command.cpp:1867).
    pub shape_top: i32,
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
}

impl CommandObjectSnapshot {
    /// C4Object::Status without C4Object::Alive. Structures and ordinary
    /// items are active command targets even though they are not living.
    pub fn is_status_active(&self) -> bool {
        !self.destroyed && self.status.is_active()
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDefinitionSnapshot {
    pub value: i32,
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

    use crate::transfer::TransferZoneRect;
    use once_cell::sync::Lazy;

    static EMPTY_TRANSFER_ZONES: Lazy<TransferZoneTable> = Lazy::new(TransferZoneTable::default);

    use crate::ocf;

    #[test]
    fn c4_angle_matches_cpp_axis_and_diagonal_boundaries() {
        // C4Math.cpp:33-45 computes atan2f first, then promotes the product
        // with the double literal 180.0 before truncating to int.
        assert_eq!(c4_angle(0, 0, 0, 10), 181);
        assert_eq!(c4_angle(0, 0, 0, -10), 359);
        assert_eq!(c4_angle(0, 0, 10, 10), 134);
    }

    #[test]
    fn c4_angle_inner_angle_matches_cpp_double_chain_exhaustively() {
        fn cpp_inner_angle(dx: i32, dy: i32) -> i32 {
            let radians = (dy as f32).atan2(dx as f32);
            (180.0_f64
                * f64::from(radians)
                * f64::from(std::f32::consts::FRAC_1_PI)) as i32
        }

        for dx in 0..=512 {
            for dy in 0..=512 {
                let folded = c4_angle(0, 0, dx, dy);
                let actual_inner = if dx > 0 { folded - 90 } else { 270 - folded };
                assert_eq!(actual_inner, cpp_inner_angle(dx, dy), "dx={dx}, dy={dy}");
            }
        }
    }

    fn snapshot_with_id(id: u64) -> CommandObjectSnapshot {
        CommandObjectSnapshot {
            contact: 0,
            action_time: 0,
            shape_top: 0,
            shape: DefinitionRect::new(-8, -10, 16, 20),
            entrance: None,
            id: ObjectId::new(id),
            definition_id: format!("DEF{id}"),
            position: Vector2::ZERO,
            fixed_position: FixedVec2::ZERO,
            fixed_velocity: FixedVec2::ZERO,
            move_to_range: 0,
            pathfinder: 0,
            no_transfer_zones: 0,
            status: ObjectStatus::Normal,
            destroyed: false,
            category: 0,
            container: None,
            action_target: None,
            action_procedure: ActionProcedure::Undefined,
            command_direction: CommandDirection::Stop,
            construction: 0,
            direction: Direction::Left,
            physical: PhysicalInfo::default(),
            owner: OWNER_NONE,
            controller: OWNER_NONE,
            base: OWNER_NONE,
            crew_member: false,
            selected: false,
            alive: true,
            contents: Vec::new(),
            line_connect: 0,
            ocf: ocf::AVAILABLE,
            entrance_status: false,
            collectible: false,
        }
    }

    fn walking_jumper(position: Vector2) -> CommandObjectSnapshot {
        let mut walker = snapshot_with_id(1);
        walker.position = position;
        walker.action_procedure = ActionProcedure::Walk;
        walker.crew_member = true;
        walker.ocf |= ocf::CREW_MEMBER;
        walker.shape_top = -10;
        walker
    }

    fn jump_ctx<'a>(
        walker: &'a CommandObjectSnapshot,
        objects: &'a HashMap<ObjectId, CommandObjectSnapshot>,
        players: &'a HashMap<i32, CommandPlayerSnapshot>,
        definitions: &'a HashMap<DefinitionId, CommandDefinitionSnapshot>,
        landscape: &'a crate::Landscape,
    ) -> CommandRuntimeContext<'a> {
        CommandRuntimeContext {
            landscape: Some(landscape),
            frame: 0,
            position: walker.position,
            object: walker,
            objects,
            players,
            definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        }
    }

    /// A MoveTo state past its InitEvaluation Execute with the raw Tx/Ty
    /// (the C++ equivalent of an Evaluated command): the movement-control
    /// geometry pins below run against these coordinates directly.
    fn evaluated_move_to(request: &CommandRequest) -> MoveToState {
        let mut state = MoveToState::from_request(request);
        state.evaluated = true;
        state
    }

    fn move_to_ctx_at_frame<'a>(
        object: &'a CommandObjectSnapshot,
        objects: &'a HashMap<ObjectId, CommandObjectSnapshot>,
        players: &'a HashMap<i32, CommandPlayerSnapshot>,
        definitions: &'a HashMap<DefinitionId, CommandDefinitionSnapshot>,
        frame: u64,
    ) -> CommandRuntimeContext<'a> {
        CommandRuntimeContext {
            landscape: None,
            frame,
            position: object.position,
            object,
            objects,
            players,
            definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        }
    }

    #[test]
    fn move_to_crew_uses_one_fifth_shape_width_as_target_range() {
        // Crew override the default five-pixel MoveToRange with
        // Shape.Wdt/5. Tutorial05's eight-pixel-wide CLNK at x=156 must
        // therefore walk toward the elevator's centering point x=160
        // instead of treating the four-pixel gap as arrived
        // (C4Command.cpp:286-306; Case.c4d/Script.c:171-220).
        let mut clonk = walking_jumper(Vector2::new(156, 100));
        clonk.shape = DefinitionRect::new(-4, -9, 8, 18);
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&clonk, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(160))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "four pixels exceeds CLNK Shape.Wdt/5 = 1"
        );
    }

    #[test]
    fn move_to_crew_pushes_pathfinder_waypoints_around_blocked_ground() {
        // A solid cave wall below the column surface blocks the direct line.
        // C4Command::MoveTo asks C4PathFinder for a route and pushes its
        // intermediate points as 25-frame MoveTo subcommands, preserving the
        // parent command's Data (C4Command.cpp:193-255).
        let mut landscape =
            crate::Landscape::with_default_material(100, vec![100; 100], None)
                .expect("cave landscape");
        landscape.set_world_height(100);
        let mut bytes = vec![0; 100 * 100];
        for y in 45..55 {
            for x in 45..47 {
                bytes[y * 100 + x] = 1;
            }
        }
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            100,
            100,
            bytes,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        let walker = walking_jumper(Vector2::new(10, 50));
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(90))
                .with_ty(Some(50))
                .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.operations,
            vec![
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(54))
                        .with_ty(Some(46))
                        .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET))
                        .with_update_interval(25)
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(47))
                        .with_ty(Some(44))
                        .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET))
                        .with_update_interval(25)
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
            ],
            "ObjectAddWaypoint applies the shape offset and pushes each deterministic intermediate waypoint with Data and interval 25 (C4Command.cpp:189-208; C4PathFinder.cpp:383-400)"
        );

        let CommandOperation::PushFront(nearest_waypoint) = &result.operations[1] else {
            panic!("nearest pathfinder operation must push MoveTo");
        };
        let mut waypoint_state = MoveToState::from_request(nearest_waypoint);
        let first = waypoint_state.step(&ctx);
        assert_eq!(
            first.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "an evaluated pathfinder waypoint steers on its first Execute"
        );
        assert_eq!(waypoint_state.update_interval, 24);
    }

    #[test]
    fn pathfinder_waypoint_skips_regrounding_after_solid_offset() {
        // ObjectAddWaypoint first nudges this point left from the ledge via
        // AdjustSolidOffset, then creates an already-evaluated MoveTo. The
        // waypoint must remain mid-air instead of AdjustMoveToTarget dropping
        // it onto the lower surface (C4Command.cpp:189-208,1628-1643).
        let mut surface = vec![110i32; 300];
        for column in surface.iter_mut().take(190).skip(150) {
            *column = 75;
        }
        let landscape =
            crate::Landscape::with_default_material(300, surface, None).expect("landscape");
        let mut walker = walking_jumper(Vector2::new(100, 100));
        walker.shape = DefinitionRect::new(-8, -10, 16, 20);
        let (mut x, mut y) = (149, 75);
        assert!(adjust_solid_offset(
            &landscape,
            &mut x,
            &mut y,
            walker.shape.width / 2,
            walker.shape.height / 2,
        ));
        assert_eq!((x, y), (142, 75));

        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(x))
            .with_ty(Some(y))
            .with_data(CommandData::Integer(0))
            .with_update_interval(25)
            .with_evaluated(true)
            .with_mode(CommandMode::SilentSub);
        let mut state = MoveToState::from_request(&request);

        let first = state.step(&ctx);

        assert_eq!((state.tx, state.ty), (Some(142), Some(75)));
        assert_eq!(
            first.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "the post-AdjustSolidOffset coordinate steers without an evaluation-only frame"
        );
    }

    #[test]
    fn move_to_skips_pathfinder_when_the_direct_path_is_free() {
        // Transfer zones are consulted only after the ordinary PathFree
        // probe reports solid terrain. A clear line that merely crosses a
        // zone must remain one direct MoveTo (C4Command.cpp:235-252).
        let landscape = crate::Landscape::flat(200, 100);
        let walker = walking_jumper(Vector2::new(20, 50));
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            ObjectId::new(9),
            TransferZoneRect {
                x: 80,
                y: 40,
                width: 20,
                height: 20,
            },
        );
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 1,
            position: walker.position,
            object: &walker,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &transfer_zones,
            rng: None,
        };
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(160))
                .with_ty(Some(50)),
        );

        let result = state.step(&ctx);

        assert!(
            result.operations.is_empty(),
            "a transfer zone alone does not trigger pathfinding"
        );
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );
    }

    #[test]
    fn definition_pathfinder_routes_noncrew_and_honors_transfer_zone_opt_out() {
        // C4Command::MoveTo enables path search for OCF_CrewMember OR a
        // nonzero Def->Pathfinder, passes the raw level through SetLevel,
        // and disables zones for Def->NoTransferZones
        // (C4Command.cpp:228-248; C4PathFinder.cpp:552-560).
        let mut landscape = crate::Landscape::with_default_material(100, vec![100; 100], None)
            .expect("split landscape");
        landscape.set_world_height(100);
        let mut bytes = vec![0; 100 * 100];
        for y in 0..100 {
            bytes[y * 100 + 49] = 1;
            bytes[y * 100 + 50] = 1;
        }
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            100,
            100,
            bytes,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        let mut mover = snapshot_with_id(1);
        mover.position = Vector2::new(10, 50);
        mover.action_procedure = ActionProcedure::Walk;
        mover.pathfinder = 27;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let zone_owner = ObjectId::new(9);
        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            zone_owner,
            TransferZoneRect {
                x: 45,
                y: 40,
                width: 10,
                height: 20,
            },
        );

        let enabled = {
            let ctx = CommandRuntimeContext {
                landscape: Some(&landscape),
                frame: 1,
                position: mover.position,
                object: &mover,
                objects: &objects,
                players: &players,
                definitions: &definitions,
                structures_need_energy: false,
                base_buy_enabled: true,
                base_sell_enabled: true,
                transfer_zones: &transfer_zones,
                rng: None,
            };
            evaluated_move_to(
                &CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(90))
                    .with_ty(Some(50)),
            )
            .step(&ctx)
        };
        assert_eq!(
            enabled.operations,
            vec![
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(58))
                        .with_ty(Some(89))
                        .with_data(CommandData::Integer(0))
                        .with_update_interval(25)
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
                CommandOperation::PushFront(
                    CommandRequest::new(CommandId::Transfer)
                        .with_target(Some(zone_owner))
                        .with_tx(Some(55))
                        .with_ty(Some(89))
                        .with_evaluated(true)
                        .with_mode(CommandMode::SilentSub),
                ),
            ],
            "the clamped non-crew search crosses the only available transfer edge"
        );

        mover.no_transfer_zones = 1;
        let disabled = {
            let ctx = CommandRuntimeContext {
                landscape: Some(&landscape),
                frame: 1,
                position: mover.position,
                object: &mover,
                objects: &objects,
                players: &players,
                definitions: &definitions,
                structures_need_energy: false,
                base_buy_enabled: true,
                base_sell_enabled: true,
                transfer_zones: &transfer_zones,
                rng: None,
            };
            evaluated_move_to(
                &CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(90))
                    .with_ty(Some(50)),
            )
            .step(&ctx)
        };
        assert_eq!(disabled.status, CommandStatus::Running);
        assert!(
            disabled.operations.is_empty(),
            "without transfer-zone edges, the full-height wall has no route"
        );
    }

    #[test]
    fn move_to_uses_positive_definition_range_for_noncrew_like_cpp() {
        // C4Command::MoveTo replaces the default five-pixel range only when
        // Def->MoveToRange is positive (C4Command.cpp:213-215); signed zero
        // and negative DefCore values retain the default.
        let mut mover = snapshot_with_id(1);
        mover.position = Vector2::new(100, 100);
        mover.fixed_position = FixedVec2::from_ints(100, 100);
        mover.move_to_range = 20;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(115))
            .with_ty(Some(100));
        let ctx = move_to_ctx_at_frame(&mover, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        assert_eq!(state.step(&ctx).status, CommandStatus::Completed);

        mover.move_to_range = -3;
        let ctx = move_to_ctx_at_frame(&mover, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );
    }

    // C4Command::MoveTo DFA_SWIM arm (C4Command.cpp:370-382): on Tick2
    // frames (Game.iTick2 != 0, i.e. odd FrameCounter) the swimmer steers
    // horizontally toward Tx (with target range); on !Tick2 frames it
    // steers vertically toward Ty with NO range (cy < Ty -> Down).
    #[test]
    fn move_to_swim_steers_horizontal_on_tick2_and_vertical_otherwise() {
        let mut swimmer = snapshot_with_id(1);
        swimmer.position = Vector2::new(100, 100);
        swimmer.action_procedure = ActionProcedure::Swim;
        swimmer.crew_member = true;
        swimmer.ocf |= ocf::CREW_MEMBER;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();

        // Target right and below: dx = 60, dy = 40.
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(160))
            .with_ty(Some(140));

        // Odd frame (iTick2 == 1): horizontal arm -> COMD_Right.
        let ctx = move_to_ctx_at_frame(&swimmer, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "Tick2 swim steering is horizontal (C4Command.cpp:372-376)"
        );

        // Even frame (iTick2 == 0): vertical arm -> COMD_Down (cy < Ty).
        let ctx = move_to_ctx_at_frame(&swimmer, &objects, &players, &definitions, 2);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Down),
            "!Tick2 swim steering is vertical (C4Command.cpp:377-381)"
        );
    }

    #[test]
    fn move_to_float_steers_against_fixed_momentum_like_cpp() {
        // DFA_FLOAT aims for a Float-physical velocity toward the target,
        // then steers from the fixed-point difference to current momentum
        // (C4Command.cpp:393-410). A floater already moving upward while its
        // target is due right therefore corrects DownRight, not merely Right.
        let mut floater = snapshot_with_id(1);
        floater.position = Vector2::new(100, 100);
        floater.fixed_position = FixedVec2::from_ints(100, 100);
        floater.fixed_velocity = FixedVec2::from_ints(0, -1);
        floater.action_procedure = ActionProcedure::Float;
        floater.physical.float = 100;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&floater, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::DownRight)
        );

        // At the desired rightward velocity the correction falls below
        // FIXED100(20), so this newly created command must explicitly clear
        // the object's pre-existing Right ComDir with COMD_None/Stop.
        floater.command_direction = CommandDirection::Right;
        floater.fixed_velocity = FixedVec2::from_ints(1, 0);
        let ctx = move_to_ctx_at_frame(&floater, &objects, &players, &definitions, 2);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );
        let result = state.step(&ctx);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop)
        );
    }

    #[test]
    fn move_to_float_finishes_immediately_inside_range_like_cpp() {
        // The target-reached branch precedes procedure steering and finishes
        // in this Execute (C4Command.cpp:286-307). Besides avoiding a second
        // arrival frame, this keeps DFA_FLOAT from normalizing a zero vector.
        let mut floater = snapshot_with_id(1);
        floater.position = Vector2::new(100, 100);
        floater.fixed_position = FixedVec2::from_ints(100, 100);
        floater.action_procedure = ActionProcedure::Float;
        floater.command_direction = CommandDirection::Right;
        floater.physical.float = 100;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&floater, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(100))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop)
        );
    }

    // C4Command::MoveTo DFA_SCALE arm (C4Command.cpp:335-338): vertical
    // steering only — cy > Ty + range heads Up, cy < Ty - range heads
    // Down (y grows downward).
    #[test]
    fn move_to_scale_steers_vertically() {
        let mut scaler = snapshot_with_id(1);
        scaler.position = Vector2::new(100, 100);
        scaler.action_procedure = ActionProcedure::Scale;
        scaler.crew_member = true;
        scaler.ocf |= ocf::CREW_MEMBER;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);

        // Target above and well to the right: DFA_SCALE ignores Tx for
        // steering (no horizontal branch in the arm) and heads Up. The
        // Dir_Left let-go stays quiet: |cy - Ty| = 60 > LetGoRange2 30.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(40)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Up),
            "scaling toward a higher target heads Up (C4Command.cpp:337)"
        );
    }

    // DFA_SCALE let-go control (C4Command.cpp:339-353): scaling with
    // Action.Dir DIR_Left and the target off the wall to the right
    // (Tx > cx + LetGoRange1 7, |cy - Ty| <= LetGoRange2 30) jumps off
    // with xdir +1 (ObjectComLetGo -> ObjectActionJump(itofix(+1), 0)).
    #[test]
    fn move_to_scale_lets_go_toward_target() {
        let mut scaler = snapshot_with_id(1);
        scaler.position = Vector2::new(100, 100);
        scaler.action_procedure = ActionProcedure::Scale;
        scaler.direction = Direction::Left;
        scaler.crew_member = true;
        scaler.ocf |= ocf::CREW_MEMBER;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);

        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(110)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("let-go update");
        let action = update.action.expect("jump action");
        assert_eq!(action.name.as_deref(), Some("Jump"));
        assert_eq!(
            update.fixed_velocity,
            Some(FixedVec2::new(
                math::itofix(1),
                crate::C4Fixed::from_raw(0)
            )),
            "let-go launches with xdir +1, ydir 0 (C4ObjectCom.cpp:310-314)"
        );
    }

    // The contact let-go (C4Command.cpp:347-352,361-366) only fires once
    // the scale action is 3+ frames old ("not if just started").
    #[test]
    fn move_to_scale_contact_let_go_respects_action_time() {
        let mut scaler = snapshot_with_id(1);
        scaler.position = Vector2::new(100, 100);
        scaler.action_procedure = ActionProcedure::Scale;
        scaler.direction = Direction::Right;
        scaler.contact = crate::CNAT_LEFT;
        scaler.crew_member = true;
        scaler.ocf |= ocf::CREW_MEMBER;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();

        // Target high above on this side: no target-direction let-go.
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(100))
            .with_ty(Some(20));

        // Action.Time == 2: too fresh, keep scaling.
        scaler.action_time = 2;
        let ctx = move_to_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        assert!(
            result
                .update
                .as_ref()
                .and_then(|update| update.action.as_ref())
                .is_none(),
            "Action.Time <= 2 must not let go (C4Command.cpp:348)"
        );

        // Action.Time == 3 with contact: let go against the facing (-1).
        scaler.action_time = 3;
        let ctx = move_to_ctx_at_frame(&scaler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(&request);
        let result = state.step(&ctx);
        let update = result.update.expect("let-go update");
        assert_eq!(
            update.action.and_then(|action| action.name),
            Some("Jump".into())
        );
        assert_eq!(
            update.fixed_velocity,
            Some(FixedVec2::new(
                math::itofix(-1),
                crate::C4Fixed::from_raw(0)
            )),
            "DIR_Right contact let-go jumps with xdir -1 (C4Command.cpp:365)"
        );
    }

    // C4Command::MoveTo DFA_HANGLE arm (C4Command.cpp:384-391):
    // horizontal steering; |Angle(cx,cy,Tx,Ty)| > LetGoHangleAngle 110
    // drops off the ceiling (ObjectComLetGo(0) — Jump with zero xdir).
    #[test]
    fn move_to_hangle_steers_horizontal_and_drops_past_angle() {
        let mut hangler = snapshot_with_id(1);
        hangler.position = Vector2::new(100, 100);
        hangler.action_procedure = ActionProcedure::Hang;
        hangler.crew_member = true;
        hangler.ocf |= ocf::CREW_MEMBER;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();

        // Target right, slightly below: Angle = 99 <= 110 keeps hangling;
        // steer Right. No vertical branch in the arm.
        let ctx = move_to_ctx_at_frame(&hangler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(160))
                .with_ty(Some(110)),
        );
        let result = state.step(&ctx);
        let update = result.update.expect("steer update");
        assert_eq!(update.command_direction, Some(CommandDirection::Right));
        assert!(update.action.is_none(), "within LetGoHangleAngle: no drop");

        // Target straight below: Angle = 180 > 110 -> ObjectComLetGo(0).
        let ctx = move_to_ctx_at_frame(&hangler, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(100))
                .with_ty(Some(160)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("drop update");
        assert_eq!(
            update.action.and_then(|action| action.name),
            Some("Jump".into())
        );
        assert_eq!(
            update.fixed_velocity,
            Some(FixedVec2::new(
                math::itofix(0),
                crate::C4Fixed::from_raw(0)
            )),
            "hangle drop has zero launch velocity (C4Command.cpp:390)"
        );
    }

    // C4Command::MoveTo DFA_FLIGHT arm (C4Command.cpp:414-417): no ComDir
    // steering at all — only FlightControl, which re-arms the Fly action
    // for a CanFly crew member with the target in the ±60° top sector.
    #[test]
    fn move_to_flight_runs_flight_control_without_steering() {
        let landscape = crate::Landscape::flat(300, 110);
        let mut flyer = snapshot_with_id(1);
        flyer.position = Vector2::new(100, 100);
        flyer.action_procedure = ActionProcedure::Flight;
        flyer.crew_member = true;
        flyer.ocf |= ocf::CREW_MEMBER;
        flyer.physical.can_fly = 1;
        flyer.shape_top = -10;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut ctx = move_to_ctx_at_frame(&flyer, &objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);

        // Target up and slightly right (angle 9, distance 70, sky above):
        // FlightControl takes off; the flight arm never assigns ComDir.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(110))
                .with_ty(Some(30)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("fly update");
        assert_eq!(
            update.command_direction, None,
            "DFA_FLIGHT never steers ComDir (C4Command.cpp:414-417)"
        );
        assert_eq!(
            update.action.and_then(|action| action.name),
            Some("Fly".into()),
            "FlightControl takes off (C4Command.cpp:1843)"
        );
    }

    // C4CMD_MoveTo InitEvaluation (C4Command.cpp:1634-1643): the first
    // Execute only evaluates (returns true — no movement that frame);
    // AdjustMoveToTarget grounds a mid-air target unless Data carries
    // C4CMD_MoveTo_NoPosAdjust (C4Command.h:68).
    #[test]
    fn move_to_init_evaluation_adjusts_target_unless_no_pos_adjust() {
        let landscape = crate::Landscape::flat(300, 110);
        // Standing walker: center y 100, feet on the 110 surface.
        let walker = walking_jumper(Vector2::new(100, 100));
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);

        // Mid-air target straight up: AdjustMoveToTarget drops it to the
        // bottom of free space (109) then lifts it Shape.Hgt/2 -> y 99,
        // one pixel off the walker's center — inside the crew range.
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(100))
            .with_ty(Some(50));
        assert!(
            !request.evaluated,
            "ordinary Enter/JumpControl/script MoveTos retain fInitEvaluation=true"
        );
        let mut state = MoveToState::from_request(&request); // unevaluated
        let first = state.step(&ctx);
        assert_eq!(first.status, CommandStatus::Running);
        assert!(
            first.update.is_none() && first.operations.is_empty(),
            "the evaluation Execute does nothing else (C4Command.cpp:1555)"
        );
        let mut ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        ctx.landscape = Some(&landscape);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert_eq!(
            second.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop),
            "the adjusted in-range target finishes immediately (C4Command.cpp:294-307)"
        );

        // NoPosAdjust keeps the raw (100,50): the walker steers Up.
        let request = request.with_data(CommandData::Integer(1));
        let mut state = MoveToState::from_request(&request); // unevaluated
        let mut ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        ctx.landscape = Some(&landscape);
        let _ = state.step(&ctx);
        let mut ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        ctx.landscape = Some(&landscape);
        let second = state.step(&ctx);
        assert_eq!(
            second.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Up),
            "NoPosAdjust leaves the mid-air target (C4Command.h:68)"
        );
    }

    // C4CMD_MoveTo InitEvaluation target absorption (C4Command.cpp:1637):
    // Tx/Ty become Target->x/y ONCE and Target clears — the destination
    // does not follow the target afterwards.
    #[test]
    fn move_to_absorbs_target_position_once() {
        let walker = walking_jumper(Vector2::new(100, 100));
        let target_id = ObjectId::new(9);
        let mut target = snapshot_with_id(9);
        target.position = Vector2::new(200, 100);
        let mut objects = HashMap::new();
        objects.insert(target_id, target);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let request = CommandRequest::new(CommandId::MoveTo).with_target(Some(target_id));
        let mut state = MoveToState::from_request(&request); // unevaluated
        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let _ = state.step(&ctx); // evaluation frame
        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        let result = state.step(&ctx);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "steers toward the absorbed (200,100)"
        );

        // Target teleports left; the command keeps heading for 200.
        objects.get_mut(&target_id).expect("target").position = Vector2::new(0, 100);
        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 3);
        let result = state.step(&ctx);
        assert_ne!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Left),
            "Tx/Ty were absorbed once — no live following (C4Command.cpp:1637)"
        );
    }

    #[test]
    fn move_to_update_interval_is_cpp_lifetime_not_step_throttle() {
        // C4Command::Execute decrements UpdateInterval as a lifetime, but
        // still executes MoveTo on every non-expiring frame
        // (C4Command.cpp:1545-1555).
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let request = CommandRequest::new(CommandId::MoveTo)
            .with_tx(Some(200))
            .with_ty(Some(100))
            .with_update_interval(4);
        let mut state = MoveToState::from_request(&request);

        let mut walker = walking_jumper(Vector2::new(100, 100));
        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 0);
        assert_eq!(state.step(&ctx).status, CommandStatus::Running);

        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );

        walker.position = Vector2::new(210, 100);
        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 2);
        let result = state.step(&ctx);
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Left),
            "MoveTo executes again on the next frame"
        );

        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 3);
        assert_eq!(state.step(&ctx).status, CommandStatus::Completed);
    }

    #[test]
    fn move_to_exits_a_container_before_steering() {
        // C4Command::MoveTo always delegates Exit before its path and
        // movement logic, which lets Build's automatic Acquire return from
        // a base with the component (C4Command.cpp:213-217).
        let container_id = ObjectId::new(9);
        let mut walker = walking_jumper(Vector2::new(100, 100));
        walker.container = Some(container_id);
        let objects = HashMap::from([(container_id, snapshot_with_id(9))]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.iter().any(|operation| matches!(
            operation,
            CommandOperation::PushFront(request)
                if request.id == CommandId::Exit
                    && request.mode == CommandMode::SilentSub
                    && request.update_interval == 50
        )));
        assert!(result.update.is_none(), "contained MoveTo does not steer");
    }

    // C4Command::MoveTo pushing (C4Command.cpp:257-265): without the
    // C4CMD_MoveTo_PushTarget Data flag (C4Command.h:69) — or against a
    // Grab=2 grab-only target — a pushing mover lets go (UnGrab sub-
    // command) and marks itself for re-evaluation.
    #[test]
    fn move_to_push_without_push_target_flag_ungrabs() {
        let vehicle_id = ObjectId::new(7);
        let mut vehicle = snapshot_with_id(7);
        vehicle.position = Vector2::new(95, 100);
        let mut pusher = walking_jumper(Vector2::new(100, 100));
        pusher.action_procedure = ActionProcedure::Push;
        pusher.action_target = Some(vehicle_id);
        let mut objects = HashMap::new();
        objects.insert(vehicle_id, vehicle);
        let players = HashMap::new();
        let mut definitions = HashMap::new();
        definitions.insert(
            "DEF7".to_string(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 1,
            },
        );

        // Data 0: pushing not desired -> UnGrab, still running.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100)),
        );
        let ctx = move_to_ctx_at_frame(&pusher, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        match result.operations.first() {
            Some(CommandOperation::PushFront(request)) => {
                assert_eq!(request.id, CommandId::UnGrab);
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("expected UnGrab, got {other:?}"),
        }
        assert!(
            !state.evaluated,
            "vehicle control may have blocked evaluation — re-evaluate (C4Command.cpp:263)"
        );
    }

    // With the PushTarget flag the mover keeps pushing: cx/cy come from
    // the pushed vehicle (C4Command.cpp:271-277) and the DFA_PUSH arm
    // steers horizontally only (:329-333).
    #[test]
    fn move_to_push_with_flag_steers_from_vehicle_position() {
        let vehicle_id = ObjectId::new(7);
        let mut vehicle = snapshot_with_id(7);
        vehicle.position = Vector2::new(95, 100);
        let mut pusher = walking_jumper(Vector2::new(100, 100));
        pusher.action_procedure = ActionProcedure::Push;
        pusher.action_target = Some(vehicle_id);
        let mut objects = HashMap::new();
        objects.insert(vehicle_id, vehicle);
        let players = HashMap::new();
        let mut definitions = HashMap::new();
        definitions.insert(
            "DEF7".to_string(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 1,
            },
        );

        // Target far below the vehicle's column: the vehicle position
        // override yields dx 0 and the push arm ignores dy entirely.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(95))
                .with_ty(Some(160))
                .with_data(CommandData::Integer(2)),
        );
        let ctx = move_to_ctx_at_frame(&pusher, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty(), "no UnGrab with PushTarget");
        assert_eq!(
            result.update.and_then(|update| update.command_direction),
            None,
            "DFA_PUSH steers horizontally from the vehicle position only"
        );

        // Grab-only target (Grab=2) lets go even with the flag.
        definitions.get_mut("DEF7").expect("def").grab = 2;
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(200))
                .with_ty(Some(100))
                .with_data(CommandData::Integer(2)),
        );
        let ctx = move_to_ctx_at_frame(&pusher, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        match result.operations.first() {
            Some(CommandOperation::PushFront(request)) => {
                assert_eq!(request.id, CommandId::UnGrab, "Grab=2 (C4Command.cpp:260)");
            }
            other => panic!("expected UnGrab, got {other:?}"),
        }
    }

    // C4Command::JumpControl trigger 1 (C4Command.cpp:1861-1872): target
    // in the ±(35±10)° diagonal, path free, farther than 30, 15px head
    // room -> a C4CMD_Jump goes on TOP of the MoveTo.
    #[test]
    fn move_to_diagonal_free_jump_like_cpp() {
        let landscape = crate::Landscape::flat(300, 110);
        let walker = walking_jumper(Vector2::new(100, 100));
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);

        // Angle(100,100 -> 140,43) = 90 - trunc(atan2(57,40)) = 36 — inside
        // 35±10; distance 70 > 30; sky above.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(43)),
        );
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1, "one jump op");
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Jump);
                assert_eq!((request.tx, request.ty), (Some(140), Some(43)));
            }
            other => panic!("expected jump, got {other:?}"),
        }
    }

    // Trigger 3 (C4Command.cpp:1896-1908): CNAT_RIGHT wall contact with
    // the target up the wall (angle ≈ ±80°) jumps without a path check.
    #[test]
    fn move_to_low_side_contact_jump_like_cpp() {
        let landscape = crate::Landscape::flat(300, 110);
        let mut walker = walking_jumper(Vector2::new(100, 100));
        walker.contact = crate::CNAT_RIGHT;
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);

        // Angle(100,100 -> 140,93) = 90 - trunc(atan2(7,40)) = 81; 81-80=1
        // inside ±50.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(140))
                .with_ty(Some(93)),
        );
        let result = state.step(&ctx);
        assert_eq!(
            result.operations.len(),
            1,
            "right-contact jump fires (left mirror uses angle+80)"
        );
        match &result.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::Jump),
            other => panic!("expected jump, got {other:?}"),
        }
    }

    // Trigger 2 (C4Command.cpp:1874-1893): target overhead on a ledge —
    // side-move first (pushed on top), then the jump.
    #[test]
    fn move_to_high_angle_side_move_jump_like_cpp() {
        // Ledge: surface 110 everywhere except a plateau (top 75) right of
        // the target.
        let mut surface = vec![110i32; 300];
        for column in surface.iter_mut().take(190).skip(150) {
            *column = 75;
        }
        let landscape =
            crate::Landscape::with_default_material(300, surface, None).expect("landscape");
        let walker = walking_jumper(Vector2::new(140, 100));
        let objects = HashMap::new();
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = jump_ctx(&walker, &objects, &players, &definitions, &landscape);

        // Target on the plateau edge: (148,72): angle = trunc(atan2(28,8))
        // = 74 -> 90-74 = 16?? — angle must sit within ±30 of straight up.
        // (148,72): dx 8, dy -28 -> angle 90-74=16 NOT <= 30? inside(16,-30,30) yes.
        // cy - ty = 28 inside 10..40. SolidOnWhichSide(148,72): plateau
        // solid at x>=150 -> +1 -> side point x = 140 - 23 = 117 (clear
        // ground), adjust drops it to the 110 surface (|dy|<=20 from 100
        // fails?) — pick ty=75 edge instead for a shallower drop.
        let mut state = evaluated_move_to(
            &CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(148))
                .with_ty(Some(72)),
        );
        let result = state.step(&ctx);
        assert_eq!(
            result.operations.len(),
            2,
            "side-move lands on top of the jump (AddCommand pushes front twice)"
        );
        match (&result.operations[0], &result.operations[1]) {
            (CommandOperation::PushFront(jump), CommandOperation::PushFront(side_move)) => {
                assert_eq!(jump.id, CommandId::Jump);
                assert_eq!(side_move.id, CommandId::MoveTo);
                assert_eq!(side_move.update_interval, 50);
            }
            other => panic!("expected jump + side move, got {other:?}"),
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
            landscape: None,
            frame: 0,
            position: follower.position,
            object: objects.get(&follower_id).expect("follower present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: follower.position,
            object: objects.get(&follower_id).expect("follower present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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

    // FnGetCommand serves the LIVE C4Command fields (C4Script.cpp:
    // 926-945) — the snapshot stack (which backs the world-context views
    // every frame) must carry the same elements, not nil.
    #[test]
    fn snapshot_command_views_expose_live_elements() {
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(140))
                    .with_ty(Some(90))
                    .with_data(CommandData::Integer(1)),
            )
            .expect("push");

        let snapshot = stack.snapshot();
        let views = snapshot.command_views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, "MoveTo");
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(140), Some(90)),
            "restored views keep Tx/Ty (C4Script.cpp:934-937)"
        );
        assert_eq!(views[0].data, CommandData::Integer(1));

        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&snapshot);
        let views = restored.command_views();
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(140), Some(90)),
            "a restored stack keeps its elements"
        );
    }

    // The element view follows the InitEvaluation rewrites: MoveTo's
    // Target folds into Tx/Ty and clears (C4Command.cpp:1637), so
    // GetCommand element 1 goes nil and element 2 reads the absorbed X.
    #[test]
    fn command_views_follow_move_to_target_absorption() {
        let walker = walking_jumper(Vector2::new(100, 100));
        let target_id = ObjectId::new(9);
        let mut target = snapshot_with_id(9);
        target.position = Vector2::new(200, 100);
        let mut objects = HashMap::new();
        objects.insert(target_id, target);
        let players = HashMap::new();
        let definitions = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_target(Some(target_id)),
            )
            .expect("push");

        let ctx = move_to_ctx_at_frame(&walker, &objects, &players, &definitions, 1);
        let _ = stack.step(&ctx); // InitEvaluation execute

        let views = stack.command_views();
        assert_eq!(views[0].target, None, "Target cleared (C4Command.cpp:1637)");
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(200), Some(100)),
            "Tx/Ty absorbed the target position"
        );

        // The same live values survive the snapshot round-trip.
        let views = stack.snapshot().command_views();
        assert_eq!(views[0].target, None);
        assert_eq!((views[0].tx, views[0].ty), (Some(200), Some(100)));
    }

    // Acquire's element view reads the defaulted 500/250 search range
    // (InitEvaluation, C4Command.cpp:1666-1670).
    #[test]
    fn command_views_show_acquire_range_defaults() {
        let mut stack = CommandStack::new();
        stack
            .push_front(
                CommandRequest::new(CommandId::Acquire)
                    .with_data(CommandData::Text("WOOD".into())),
            )
            .expect("push");

        let views = stack.command_views();
        assert_eq!(
            (views[0].tx, views[0].ty),
            (Some(500), Some(250)),
            "defaulted search range is live-visible (C4Command.cpp:1668-1669)"
        );
        assert_eq!(views[0].data, CommandData::Text("WOOD".into()));

        let views = stack.snapshot().command_views();
        assert_eq!((views[0].tx, views[0].ty), (Some(500), Some(250)));
    }

    #[test]
    fn wait_takes_its_duration_from_data_then_tx() {
        // C4CMD_Wait InitEvaluation (C4Command.cpp:1659-1663): a nonzero
        // Data overrides the update interval, else a nonzero Tx does. The
        // dragon waits via SetCommand(this(), "Wait", 0,0,0,0, 10) — data
        // slot 10, no interval (Fantasy.c4d Dragon.c4d Script.c:1649).
        let from_data = CommandRequest::new(CommandId::Wait).with_data(CommandData::Integer(10));
        assert_eq!(
            WaitState::from_request(&from_data).remaining,
            Some(10),
            "Data overrides the interval"
        );

        let from_tx = CommandRequest::new(CommandId::Wait).with_tx(Some(7));
        assert_eq!(
            WaitState::from_request(&from_tx).remaining,
            Some(7),
            "Tx is the fallback duration"
        );

        let from_interval = CommandRequest::new(CommandId::Wait)
            .with_update_interval(3)
            .with_data(CommandData::Integer(10));
        assert_eq!(
            WaitState::from_request(&from_interval).remaining,
            Some(10),
            "Data wins even when an interval is present"
        );
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
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 1,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let result1 = state.step(&ctx1);
        assert_eq!(result1.status, CommandStatus::Running);

        let ctx2 = CommandRuntimeContext {
            landscape: None,
            frame: 2,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let result2 = state.step(&ctx2);
        assert_eq!(result2.status, CommandStatus::Completed);
    }

    #[test]
    fn get_pursuit_moves_with_the_random_offset_like_cpp() {
        // C4Command::Get outside pursuit (C4Command.cpp:1288-1290): target
        // not in collection range and not in jump range -> AddCommand
        // MoveTo(Target->x + Random(15) - 7, Target->y, 25). The Random
        // draw advances the synced ledger.
        let actor_id = ObjectId::new(501);
        let target_id = ObjectId::new(502);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 100);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(300, 100);
        target.collectible = true;
        target.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor_id, actor);
        objects.insert(target_id, target);
        let players = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let rng = std::cell::RefCell::new(crate::LcgRng::seed_from_u64(7));
        let expected_offset = {
            let mut probe = rng.borrow().clone();
            probe.random(15) - 7
        };
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            rng: Some(&rng),
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");
        let result = state.step(&ctx);

        let move_to = result
            .operations
            .iter()
            .find_map(|operation| match operation {
                CommandOperation::PushFront(request) if request.id == CommandId::MoveTo => {
                    Some(request)
                }
                _ => None,
            })
            .expect("pursuit pushes MoveTo");
        assert_eq!(
            move_to.tx,
            Some(300 + expected_offset),
            "MoveTo x = Target->x + Random(15) - 7 (C4Command.cpp:1290)"
        );
        assert_eq!(move_to.ty, Some(100), "MoveTo y = Target->y");
        assert_eq!(move_to.update_interval, 25, "iUpdateInterval 25");
        assert_eq!(
            rng.borrow().count,
            crate::LcgRng::seed_from_u64(7).count + 1,
            "exactly one ledger draw"
        );
    }

    #[test]
    fn get_transfers_item_when_in_range() {
        // C4Command::Get only requires a live target pointer with OCF_Carryable;
        // nonliving items such as the Tutorial04 construction kit are valid
        // Get targets (C4Command.cpp:1129-1152,1206-1216).
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
        item.alive = false;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);

        match &result.events[0] {
            CommandEvent::GetObject {
                actor_id: event_actor,
                object_id,
            } => {
                assert_eq!(*event_actor, actor_id);
                assert_eq!(*object_id, target_id);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn get_resolves_nonliving_item_by_container_and_definition() {
        // C4Command::Get resolves Target2->Contents.Find(Data) without an
        // Alive check, then collects the carryable target from the actor's
        // container (C4Command.cpp:1138-1152,1206-1216).
        let actor_id = ObjectId::new(100);
        let container_id = ObjectId::new(200);
        let item_id = ObjectId::new(300);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "CNKT".into();
        item.container = Some(container_id);
        item.collectible = true;
        item.construction = FULL_CON;
        item.alive = false;

        let objects = HashMap::from([
            (actor_id, actor),
            (container_id, container),
            (item_id, item),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state = GetState::from_request(
            &CommandRequest::new(CommandId::Get)
                .with_target2(Some(container_id))
                .with_data(CommandData::Text("CNKT".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.iter().any(|event| matches!(
            event,
            CommandEvent::GetObject { actor_id: event_actor, object_id }
                if *event_actor == actor_id && *object_id == item_id
        )));
    }

    #[test]
    fn get_enters_nonliving_structure_for_contained_item() {
        // C4Command::Get treats a target container as an object with Status,
        // not as a living object. A HUT3 therefore remains a valid entrance
        // on the automatic construction-material route
        // (C4Command.cpp:1180-1217).
        let actor_id = ObjectId::new(101);
        let container_id = ObjectId::new(201);
        let item_id = ObjectId::new(301);
        let actor = snapshot_with_id(actor_id.as_u64());
        let mut container = snapshot_with_id(container_id.as_u64());
        container.alive = false;
        container.ocf = ocf::ENTRANCE;
        container.contents.push(item_id);
        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.alive = false;
        item.collectible = true;
        item.construction = FULL_CON;
        item.container = Some(container_id);
        let objects = HashMap::from([
            (actor_id, actor),
            (container_id, container),
            (item_id, item),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state =
            GetState::from_request(&CommandRequest::new(CommandId::Get).with_target(Some(item_id)))
                .expect("state created");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.iter().any(|operation| matches!(
            operation,
            CommandOperation::PushFront(request)
                if request.id == CommandId::Enter && request.target == Some(container_id)
        )));
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn put_transfers_item_into_nearby_target_container() {
        let actor_id = ObjectId::new(590);
        let item_id = ObjectId::new(591);
        let container_id = ObjectId::new(592);

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
        objects.insert(actor.id, actor);
        objects.insert(item.id, item);
        objects.insert(target_container.id, target_container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 0);
        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(container_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(update.container, Some(Some(container_id)));
                assert_eq!(update.position, Some(target_container.position));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn put_inside_target_container_ignores_center_distance() {
        let actor_id = ObjectId::new(600);
        let item_id = ObjectId::new(601);
        let container_id = ObjectId::new(602);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(50, 50);
        actor.container = Some(container_id);
        actor.contents = vec![item_id];

        let mut item = snapshot_with_id(item_id.as_u64());
        item.container = Some(actor_id);
        item.position = actor.position;

        let mut target_container = snapshot_with_id(container_id.as_u64());
        target_container.position = Vector2::new(54, 80);
        target_container.collectible = false;
        target_container.alive = false;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item);
        objects.insert(target_container.id, target_container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PutState::from_request(
            &CommandRequest::new(CommandId::Put).with_target(Some(container_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
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

        objects
            .get_mut(&item_id)
            .expect("item present")
            .container = Some(container_id);
        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .contents
            .clear();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 1);
        assert_eq!(state.step(&ctx).status, CommandStatus::Completed);
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = DropState::from_request(&CommandRequest::new(CommandId::Drop));
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 2);
        match &result.events[0] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(update.container, Some(None));
                assert_eq!(update.position, Some(actor.position));
            }
            other => panic!("unexpected event: {:?}", other),
        }
        // ObjectComDrop arms the DROPPER's NoCollectDelay after the exit
        // (C4ObjectCom.cpp:668-671).
        match &result.events[1] {
            CommandEvent::ArmNoCollectDelay { object_id } => {
                assert_eq!(*object_id, actor_id);
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                    request.id == CommandId::MoveTo
                        && request.tx == Some(120)
                        && request.ty == Some(0)
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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

        // C4Command::Drop calls ObjectComPutTake while pushing and then
        // immediately Finish(true); once the delegated Put has moved the item
        // into that target, Drop must not enqueue Get and pull it back out
        // (C4Command.cpp:998-1049, especially :1043-1048).
        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .contents
            .clear();
        objects
            .get_mut(&item_id)
            .expect("item present")
            .container = Some(pushed_id);
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 1);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
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
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                landscape: None,
                frame,
                position: actor.position,
                object: objects.get(&actor_id).expect("actor present"),
                objects: &objects,
                players: &players,
                definitions: &definitions,
                structures_need_energy: false,
                base_buy_enabled: true,

                base_sell_enabled: true,
                transfer_zones: &EMPTY_TRANSFER_ZONES,
                rng: None,
            };
            let result = stack.step(&ctx).expect("running result");
            assert_eq!(result.status, CommandStatus::Running);
            assert!(result.update.is_none());
        }

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 2,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
        // C4Command::Enter checks Target->At(cx, cy) — the actor point in
        // the target's absolute shape (C4Command.cpp:586-588).
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        target.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        target.entrance_status = true;
        target.category = CATEGORY_STRUCTURE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("enter should produce an update");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(
            update.container.is_none(),
            "C4Object::Enter is an ordered engine event, not a plain delta"
        );
        assert_eq!(result.events.len(), 1);
        assert_eq!(
            result.events[0],
            CommandEvent::EnterObject {
                object_id: actor_id,
                container_id: target_id,
            }
        );
        assert!(result.operations.is_empty());
    }

    #[test]
    fn enter_rechecks_an_opened_door_before_its_interval_expires() {
        // UpdateInterval is a command lifetime decremented before every
        // execution; it never throttles C4Command::Enter. After the first
        // call activates a closed door, the very next frame may enter it
        // (C4Command.cpp:545-616,1545-1555).
        let actor_id = ObjectId::new(32);
        let target_id = ObjectId::new(42);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(10, 10);
        let mut target = snapshot_with_id(target_id.as_u64());
        target.shape = DefinitionRect::new(0, 0, 20, 20);
        target.ocf = ocf::ENTRANCE;
        target.entrance_status = false;
        let mut objects = HashMap::from([(actor_id, actor), (target_id, target)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter)
                .with_target(Some(target_id))
                .with_update_interval(50),
        )
        .expect("state created");

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let first_ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 100);
        let first = state.step(&first_ctx);
        assert!(matches!(
            first.events.as_slice(),
            [CommandEvent::CallObjectFunction { function, .. }] if function == "ActivateEntrance"
        ));

        objects
            .get_mut(&target_id)
            .expect("target present")
            .entrance_status = true;
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let next_ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 101);
        let next = state.step(&next_ctx);
        assert_eq!(next.status, CommandStatus::Completed);
        assert_eq!(
            next.events,
            vec![CommandEvent::EnterObject {
                object_id: actor_id,
                container_id: target_id,
            }]
        );
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
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        target.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        target.category = CATEGORY_STRUCTURE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 5,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                assert_eq!(
                    request.data,
                    CommandData::None,
                    "no Enter_PushTarget: the MoveTo gets no PushTarget either"
                );
            }
            other => panic!("unexpected operation: {:?}", other),
        }

        // C4Command::Enter passes C4CMD_MoveTo_PushTarget through when
        // its own Data carries C4CMD_Enter_PushTarget (C4Command.cpp:615).
        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter)
                .with_target(Some(target_id))
                .with_data(CommandData::Integer(2)),
        )
        .expect("state created");
        let result = state.step(&ctx);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(
                    request.data,
                    CommandData::Integer(2),
                    "Enter_PushTarget -> MoveTo_PushTarget (C4Command.cpp:615)"
                );
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn enter_moves_toward_the_target_while_contained_elsewhere() {
        // Enter does not stall when the actor is in another structure. Its
        // far branch queues MoveTo(target entrance), and MoveTo then queues
        // Exit before steering (C4Command.cpp:586-615,213-217). Workshop
        // production relies on this after Acquire buys material in HUT3.
        let actor_id = ObjectId::new(51);
        let current_container_id = ObjectId::new(52);
        let target_id = ObjectId::new(53);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(20, 20);
        actor.container = Some(current_container_id);
        let current_container = snapshot_with_id(current_container_id.as_u64());
        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(120, 20);
        target.shape = DefinitionRect::new(110, 10, 20, 20);
        target.ocf = ocf::ENTRANCE;
        target.entrance_status = true;
        let objects = HashMap::from([
            (actor_id, actor.clone()),
            (current_container_id, current_container),
            (target_id, target),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = move_to_ctx_at_frame(&actor, &objects, &players, &definitions, 10);
        let mut state = EnterState::from_request(
            &CommandRequest::new(CommandId::Enter).with_target(Some(target_id)),
        )
        .expect("Enter state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        match result.operations.as_slice() {
            [CommandOperation::PushFront(request)] => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, Some(target_id));
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("expected MoveTo target entrance, got {other:?}"),
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
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
        // C4Command::Grab tests the actor point in the target's shape
        // (Target->At, C4Command.cpp:689-691).
        target.shape = DefinitionRect::new(target.position.x - 10, target.position.y - 10, 20, 20);
        target.ocf = ocf::GRAB | ocf::AVAILABLE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 1,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = GrabState::from_request(
            &CommandRequest::new(CommandId::Grab).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert!(
            result.update.is_none(),
            "RejectGrabbed must run before Stop or Push"
        );
        assert_eq!(
            result.events,
            vec![CommandEvent::AttemptGrab {
                actor_id,
                target_id,
            }]
        );
    }

    #[test]
    fn reject_grab_finishes_direct_command_as_silent_base() {
        let target = ObjectId::new(321);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;

        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands[0].mode, CommandMode::SilentBase);
        assert_eq!(
            snapshot.commands[0].finished,
            Some(CommandStatus::Failed)
        );
        assert_eq!(snapshot.commands[0].failures, 0);
    }

    #[test]
    fn reject_grab_propagates_sub_failure_to_first_unfinished_command() {
        let target = ObjectId::new(322);
        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Wait))
            .expect("base queues");
        stack
            .push_front(
                CommandRequest::new(CommandId::Grab)
                    .with_target(Some(target))
                    .with_mode(CommandMode::Sub),
            )
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;

        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands[0].mode, CommandMode::Sub);
        assert_eq!(
            snapshot.commands[0].finished,
            Some(CommandStatus::Failed)
        );
        assert_eq!(snapshot.commands[1].failures, 1);
    }

    #[test]
    fn reject_grab_resolves_marked_command_below_callback_added_front() {
        let target = ObjectId::new(323);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;
        stack
            .push_front(CommandRequest::new(CommandId::Wait))
            .expect("callback command queues");

        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands[0].state.id(), Some(CommandId::Wait));
        assert_eq!(snapshot.commands[0].finished, None);
        assert_eq!(snapshot.commands[1].state.id(), Some(CommandId::Grab));
        assert_eq!(
            snapshot.commands[1].finished,
            Some(CommandStatus::Failed)
        );
        assert_eq!(snapshot.commands[1].mode, CommandMode::SilentBase);
    }

    #[test]
    fn reject_grab_does_not_finish_same_target_replacement() {
        let target = ObjectId::new(324);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        let Some(CommandState::Grab(state)) =
            stack.entries.front_mut().map(|entry| &mut entry.state)
        else {
            panic!("Grab is front");
        };
        state.reject_pending = true;

        stack.clear();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("replacement Grab queues");
        assert_eq!(stack.resolve_grab_attempt(target, true), Some(true));
        assert_eq!(stack.snapshot().commands[0].finished, None);
    }

    #[test]
    fn detached_grab_preserves_clear_pointer_order() {
        fn marked_grab(target: ObjectId) -> CommandStack {
            let mut stack = CommandStack::new();
            stack
                .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
                .expect("Grab queues");
            let CommandState::Grab(state) = &mut stack.entries[0].state else {
                panic!("Grab is front");
            };
            state.reject_pending = true;
            stack
        }

        let target = ObjectId::new(326);
        let mut cleared_first = marked_grab(target);
        assert!(cleared_first.clear_object_reference(target));
        cleared_first.clear();
        cleared_first
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("replacement queues");
        assert_eq!(
            cleared_first.resolve_grab_attempt(target, false),
            Some(false)
        );
        assert_eq!(cleared_first.snapshot().commands[0].finished, None);

        let mut replaced_first = marked_grab(target);
        replaced_first.clear();
        replaced_first
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("replacement queues");
        assert!(replaced_first.clear_object_reference(target));
        assert_eq!(
            replaced_first.resolve_grab_attempt(target, false),
            Some(true)
        );
        assert_eq!(replaced_first.snapshot().commands[0].finished, None);
    }

    #[test]
    fn detached_same_target_attempts_resolve_lifo() {
        let target = ObjectId::new(327);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("outer Grab queues");
        let CommandState::Grab(outer) = &mut stack.entries[0].state else {
            panic!("outer Grab is front");
        };
        outer.reject_pending = true;
        outer.target_cleared = true;
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("nested Grab queues");
        let CommandState::Grab(nested) = &mut stack.entries[0].state else {
            panic!("nested Grab is front");
        };
        nested.reject_pending = true;

        stack.clear();
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(true));
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(false));
    }

    #[test]
    fn grab_attempt_tracks_cleared_target_without_legacy_request() {
        let target = ObjectId::new(325);
        let mut stack = CommandStack::new();
        stack
            .push_front(CommandRequest::new(CommandId::Grab).with_target(Some(target)))
            .expect("Grab queues");
        stack.entries[0].request = None;
        let CommandState::Grab(state) = &mut stack.entries[0].state else {
            panic!("Grab is front");
        };
        state.reject_pending = true;
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(true));

        let CommandState::Grab(state) = &mut stack.entries[0].state else {
            panic!("Grab is front");
        };
        state.reject_pending = true;
        assert!(stack.clear_object_reference(target));
        assert_eq!(stack.resolve_grab_attempt(target, false), Some(false));
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
            landscape: None,
            frame: 2,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 3,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn push_to_completes_when_target_in_destination_container() {
        let actor_id = ObjectId::new(400);
        let target_id = ObjectId::new(401);
        let destination_id = ObjectId::new(402);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.container = Some(destination_id);
        target.ocf |= ocf::GRAB;

        let destination = snapshot_with_id(destination_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(destination.id, destination);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_target2(Some(destination_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
    }

    #[test]
    fn push_to_requests_activate_when_target_contained_elsewhere() {
        let actor_id = ObjectId::new(410);
        let target_id = ObjectId::new(411);
        let container_id = ObjectId::new(412);

        let actor = snapshot_with_id(actor_id.as_u64());

        let mut target = snapshot_with_id(target_id.as_u64());
        target.container = Some(container_id);
        target.ocf |= ocf::GRAB;

        let container = snapshot_with_id(container_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(container.id, container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Activate);
                assert_eq!(request.target, Some(target_id));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected activate request, got {:?}", other),
        }
    }

    #[test]
    fn construct_without_definition_opens_menu_after_capability_gate() {
        let builder_id = ObjectId::new(1);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 42;
        builder.command_direction = CommandDirection::Right;
        builder.physical.can_construct = 1;
        let objects = HashMap::from([(builder_id, builder.clone())]);
        let players = HashMap::from([(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Eliminated,
                surrendered: false,
                wealth: 0,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        )]);
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            ConstructState::from_request(&CommandRequest::new(CommandId::Construct));
        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            [CommandEvent::OpenMenu(MenuRequest {
                crew_id: builder_id,
                owner: 42,
                kind: MenuRequestKind::Construction,
            })]
        );

        let mut incapable = builder.clone();
        incapable.physical.can_construct = 0;
        let incapable_ctx = CommandRuntimeContext {
            object: &incapable,
            ..ctx.clone()
        };
        let mut incapable_state =
            ConstructState::from_request(&CommandRequest::new(CommandId::Construct));
        let incapable_result = incapable_state.step(&incapable_ctx);
        assert_eq!(incapable_result.status, CommandStatus::Failed);
        assert!(incapable_result.events.is_empty());
    }

    #[test]
    fn construct_spawns_construction_and_queues_build() {
        let builder_id = ObjectId::new(1);
        let kit_id = ObjectId::new(2);
        let construction_definition = "STRT".to_string();

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(10, 0);
        builder.command_direction = CommandDirection::Right;
        builder.physical.can_construct = 1;
        builder.owner = 42;
        builder.contents.push(kit_id);

        let mut kit = snapshot_with_id(kit_id.as_u64());
        kit.definition_id = CONKIT_DEFINITION.into();
        kit.collectible = true;
        kit.construction = FULL_CON;
        kit.container = Some(builder_id);
        kit.position = builder.position;

        let mut objects = HashMap::new();
        objects.insert(builder_id, builder.clone());
        objects.insert(kit_id, kit);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            42,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: vec![construction_definition.clone()],
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            construction_definition.clone(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: true,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ConstructState::from_request(
            &CommandRequest::new(CommandId::Construct)
                .with_data(CommandData::Text(construction_definition.clone()))
                .with_tx(Some(10))
                .with_ty(Some(0)),
        );

        let first = state.step(&ctx);
        assert_eq!(first.status, CommandStatus::Running);
        assert_eq!(first.events.len(), 2);

        match &first.events[0] {
            CommandEvent::SpawnObject {
                definition_id,
                owner,
                position,
                container,
                construction,
            } => {
                assert_eq!(definition_id, &construction_definition);
                assert_eq!(*owner, 42);
                assert_eq!(*position, Vector2::new(10, 0));
                assert_eq!(*container, None);
                assert_eq!(*construction, Some(1));
            }
            other => panic!("unexpected event: {:?}", other),
        }

        match &first.events[1] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, kit_id);
                assert_eq!(update.container, Some(None));
                assert_eq!(update.status, Some(ObjectStatus::Deleted));
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let construction_id = ObjectId::new(3);
        let mut construction = snapshot_with_id(construction_id.as_u64());
        construction.definition_id = construction_definition.clone();
        construction.position = Vector2::new(10, 0);
        construction.owner = 42;
        construction.construction = 1;
        objects.insert(construction_id, construction);

        let mut updated_builder = builder.clone();
        updated_builder.contents.clear();
        objects.insert(builder_id, updated_builder);

        let ctx_after_spawn = CommandRuntimeContext {
            landscape: None,
            frame: 1,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let second = state.step(&ctx_after_spawn);
        assert_eq!(second.status, CommandStatus::Completed);
        assert_eq!(second.operations.len(), 1);
        match &second.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Build);
                assert_eq!(request.target, Some(construction_id));
                assert_eq!(request.tx, Some(10));
                assert_eq!(request.ty, Some(0));
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn construct_requests_acquire_when_missing_conkit() {
        let builder_id = ObjectId::new(5);
        let construction_definition = "STRT".to_string();

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Right;
        builder.physical.can_construct = 1;
        builder.owner = 7;

        let mut objects = HashMap::new();
        objects.insert(builder_id, builder.clone());

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            7,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: vec![construction_definition.clone()],
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            construction_definition.clone(),
            CommandDefinitionSnapshot {
                value: 0,
                can_chop: false,
                chop_action: None,
                constructable: true,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ConstructState::from_request(
            &CommandRequest::new(CommandId::Construct)
                .with_data(CommandData::Text(construction_definition))
                .with_tx(Some(8))
                .with_ty(Some(2)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.events.len(), 0);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Acquire);
                assert_eq!(request.mode, CommandMode::Sub);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn push_to_requests_grab_when_actor_not_pushing() {
        let actor_id = ObjectId::new(420);
        let target_id = ObjectId::new(421);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(30, 0);
        target.ocf |= ocf::GRAB;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Grab);
                assert_eq!(request.target, Some(target_id));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected grab request, got {:?}", other),
        }
    }

    #[test]
    fn push_to_requests_enter_when_destination_requires_container() {
        let actor_id = ObjectId::new(430);
        let target_id = ObjectId::new(431);
        let destination_id = ObjectId::new(432);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(60, 0);
        target.ocf |= ocf::GRAB;

        let destination = snapshot_with_id(destination_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);
        objects.insert(destination.id, destination);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 5,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_target2(Some(destination_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(destination_id));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected enter request, got {:?}", other),
        }
    }

    #[test]
    fn push_to_requests_move_to_target_position() {
        let actor_id = ObjectId::new(440);
        let target_id = ObjectId::new(441);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(80, 0);
        target.ocf |= ocf::GRAB;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 8,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_tx(Some(100))
                .with_ty(Some(0)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(100));
                assert_eq!(request.ty, Some(0));
                assert_eq!(request.update_interval, 40);
            }
            other => panic!("expected moveto request, got {:?}", other),
        }
    }

    #[test]
    fn push_to_completes_with_wait_and_ungrab_when_in_position() {
        let actor_id = ObjectId::new(450);
        let target_id = ObjectId::new(451);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.command_direction = CommandDirection::Right;
        actor.action_procedure = ActionProcedure::Push;
        actor.action_target = Some(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(95, 5);
        target.ocf |= ocf::GRAB;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 12,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = PushToState::from_request(
            &CommandRequest::new(CommandId::PushTo)
                .with_target(Some(target_id))
                .with_tx(Some(100))
                .with_ty(Some(0)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("push_to should stop actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(result.operations.len(), 2);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::UnGrab);
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("expected ungrab request, got {:?}", other),
        }
        match &result.operations[1] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Wait);
                assert_eq!(request.update_interval, 10);
            }
            other => panic!("expected wait request, got {:?}", other),
        }
    }

    #[test]
    fn ungrab_stands_in_walk_and_completes() {
        let actor_id = ObjectId::new(370);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Push;
        actor.command_direction = CommandDirection::Left;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 4,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = UnGrabState::from_request(&CommandRequest::new(CommandId::UnGrab));

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("ungrab should update actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        let action = update.action.expect("ungrab should reset action");
        assert_eq!(action.name.as_deref(), Some("Walk"));
        assert_eq!(
            action.target, None,
            "ObjectActionStand retains the former action target"
        );
        assert_eq!(update.velocity, Some(Vector2::ZERO));
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
            landscape: None,
            frame: 5,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = UnGrabState::from_request(&CommandRequest::new(CommandId::UnGrab));

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn jump_command_defers_targeted_jump_to_the_live_engine() {
        let actor_id = ObjectId::new(400);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = JumpState::from_request(
            &CommandRequest::new(CommandId::Jump).with_tx(Some(actor.position.x + 10)),
        );

        let result = state.step(&ctx);
        // C4Command::Jump calls ObjectComJump before Finish(true), so the
        // live event must run while the command is still active
        // (C4Command.cpp:1056-1067).
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComJump {
                object_id: actor_id,
                tx: actor.position.x + 10,
            }]
        );
    }

    #[test]
    fn jump_launches_with_con_scaled_walk_and_jump_physicals() {
        // ObjectComJump (C4ObjectCom.cpp:284-296): TXDir = ±ValByPhysical(280,
        // Walk)*Con/FullCon, ydir = -ValByPhysical(1000, Jump)*Con/FullCon,
        // applied with the Jump action (ObjectActionJump, :48-61).
        let actor_id = ObjectId::new(402);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.construction = FULL_CON;
        actor.physical = PhysicalInfo {
            walk: 35_000,
            jump: 40_000,
            ..PhysicalInfo::default()
        };

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            Direction::Right,
        );
        // Full Con: TXDir = +ValByPhysical(280, 35000) = raw 64225,
        // ydir = -ValByPhysical(1000, 40000) = raw -262144.
        assert_eq!(velocity.x.val(), 64225);
        assert_eq!(velocity.y.val(), -262144);

        // Half Con scales both (C4ObjectCom.cpp:287-288).
        let mut small = actor.clone();
        small.construction = FULL_CON / 2;
        let mut objects = HashMap::new();
        objects.insert(small.id, small.clone());
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
            position: small.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            Direction::Right,
        );
        assert_eq!(velocity.x.val(), 32112);
        assert_eq!(velocity.y.val(), -131072);
    }

    #[test]
    fn jump_with_multidirection_facing_has_zero_horizontal_launch() {
        // ObjectComJump initializes TXDir to zero and only changes it for a
        // left/right ComDir or exact DIR_Left/DIR_Right facing
        // (C4ObjectCom.cpp:284-296).
        let actor_id = ObjectId::new(403);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.construction = FULL_CON;
        actor.direction = Direction::from_raw(8);
        actor.command_direction = CommandDirection::Stop;
        actor.physical = PhysicalInfo {
            walk: 35_000,
            jump: 40_000,
            ..PhysicalInfo::default()
        };

        let objects = HashMap::from([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            ctx.object.direction,
        );
        assert_eq!(velocity.x, crate::C4Fixed::ZERO);
        assert!(velocity.y < crate::C4Fixed::ZERO);
    }

    #[test]
    fn jump_with_zero_physicals_still_overwrites_both_velocities() {
        // ObjectComJump always passes its calculated TXDir/iPhysicalJump to
        // ObjectActionJump (C4ObjectCom.cpp:284-307), which unconditionally
        // assigns both xdir and ydir (C4ObjectCom.cpp:48-61). Zero physicals
        // therefore stop any pre-existing motion rather than preserving it.
        let actor_id = ObjectId::new(404);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Walk;
        actor.construction = FULL_CON;
        actor.fixed_velocity = FixedVec2::new(crate::itofix(3), crate::itofix(-4));
        actor.physical = PhysicalInfo::default();

        let objects = HashMap::from([(actor.id, actor.clone())]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 6,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let velocity = object_com_jump_launch(
            ctx.object.construction,
            ctx.object.physical,
            ctx.object.command_direction,
            ctx.object.direction,
        );
        assert_eq!(
            velocity,
            FixedVec2::new(crate::C4Fixed::ZERO, crate::C4Fixed::ZERO)
        );
    }

    #[test]
    fn jump_command_defers_the_walk_gate_to_live_object_com_jump() {
        let actor_id = ObjectId::new(401);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.action_procedure = ActionProcedure::Hang;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 7,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = JumpState::from_request(
            &CommandRequest::new(CommandId::Jump).with_tx(Some(actor.position.x - 15)),
        );

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none());
        assert_eq!(
            result.events,
            vec![CommandEvent::ObjectComJump {
                object_id: actor_id,
                tx: actor.position.x - 15,
            }]
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
            landscape: None,
            frame: 32,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 48,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
        // C4Command::Throw faces/stops at the targeted position, then
        // ObjectActionThrow performs the action-gated exit atomically
        // (C4Command.cpp:950-957; C4ObjectCom.cpp:120-137).
        let actor_id = ObjectId::new(450);
        let target_id = ObjectId::new(460);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 200);
        actor.shape_top = -10;
        actor.direction = Direction::Right;
        actor.action_procedure = ActionProcedure::Walk;
        actor.physical.throw = 50_000;
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
        let rng = std::cell::RefCell::new(crate::LcgRng::seed_from_u64(7));
        let expected_rng = rng.borrow().clone();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 52,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: Some(&rng),
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
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("throw should update actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(update.action.is_none(), "the engine event gates SetAction");
        assert_eq!(update.direction, Some(Direction::Right));
        assert_eq!(result.events.len(), 1);
        let CommandEvent::ThrowObject {
            actor_id: event_actor,
            object_id,
            complete_command_on_success,
        } = &result.events[0]
        else {
            panic!("outdoor Throw must emit one atomic throw event")
        };
        assert_eq!(*event_actor, actor_id);
        assert_eq!(*object_id, target_id);
        assert!(*complete_command_on_success);
        assert_eq!(*rng.borrow(), expected_rng, "the event owns the RNG draw");
    }

    #[test]
    fn untargeted_throw_puts_into_pushed_target_without_ungrabbing() {
        // C4Command::Throw only ungrabs for a targeted-coordinate throw.
        // With no coordinates, DFA_PUSH instead calls ObjectComPutTake on
        // Action.Target and immediately finishes (C4Command.cpp:910-984,
        // especially :927-934 and :973-979).
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
            landscape: None,
            frame: 60,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ThrowState::from_request(
            &CommandRequest::new(CommandId::Throw)
                .with_target(Some(item_id))
                .with_update_interval(1),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        let CommandEvent::ApplyObjectUpdate { object_id, update } = &result.events[0] else {
            panic!("pushed-target Throw must transfer one item")
        };
        assert_eq!(*object_id, item_id);
        assert_eq!(update.container, Some(Some(push_target_id)));
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
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
        container.entrance_status = true;

        let mut parent = snapshot_with_id(parent_id.as_u64());
        parent.position = Vector2::new(100, -20);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);
        objects.insert(parent.id, parent.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 10,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
        container.entrance_status = true;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 20,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn exit_uses_open_entrance_bottom_for_top_level_ejection() {
        // C4Command::Exit places a top-level contained object at the
        // entrance center/bottom, adjusted by the exiting object's shape
        // top (C4Command.cpp:624-645). This is the HUT2/TFLN geometry in
        // Tutorial 4: dropping at the building center makes the flint hit
        // the ground hard enough to ignite.
        let actor_id = ObjectId::new(91);
        let container_id = ObjectId::new(101);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.shape_top = -3;

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(586, 245);
        container.alive = false;
        container.entrance_status = true;
        container.ocf |= ocf::ENTRANCE;
        container.entrance = Some(DefinitionRect::new(568, 253, 16, 17));

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 20,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit)).expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("exit should update actor");
        assert_eq!(update.container, Some(None));
        assert_eq!(update.position, Some(Vector2::new(576, 266)));
        assert_eq!(update.velocity, Some(Vector2::ZERO));
    }

    #[test]
    fn exit_activates_closed_entrance_and_remains_pending() {
        let actor_id = ObjectId::new(92);
        let container_id = ObjectId::new(102);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        actor.command_direction = CommandDirection::Right;

        let mut container = snapshot_with_id(container_id.as_u64());
        container.alive = false;
        container.entrance_status = false;
        container.ocf |= ocf::ENTRANCE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 20,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit)).expect("state created");
        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("closed-door Exit stops the actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(update.container.is_none(), "the actor remains contained");
        assert!(update.position.is_none(), "the actor is not ejected yet");
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::CallObjectFunction {
                object_id,
                function,
                caller,
                on_result,
                ..
            } => {
                assert_eq!(*object_id, container_id);
                assert_eq!(function, "ActivateEntrance");
                assert_eq!(*caller, actor_id);
                assert_eq!(
                    on_result,
                    &Some(CallResultAction::FailCommandOnFalse {
                        command: CommandId::Exit,
                    })
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let mut stack = CommandStack::new();
        stack
            .push_back(CommandRequest::new(CommandId::Exit))
            .expect("Exit command queues");
        assert!(stack.fail_front_if(CommandId::Exit));
        assert_eq!(stack.snapshot().commands[0].failures, 1);
        assert_eq!(
            stack.step(&ctx).expect("armed failure executes").status,
            CommandStatus::Failed,
            "ActivateEntrance=false follows C4Command::Finish(false)"
        );
    }

    #[test]
    fn exit_rechecks_an_opened_door_before_its_interval_expires() {
        // C4Command::Exit executes every frame; UpdateInterval is command
        // lifetime, not a polling delay (C4Command.cpp:624-650,1545-1555).
        // That matters for HUT3's nine-frame OpenDoor and forty-frame
        // DoorOpen window (Hut3.c4d/ActMap.txt:2-26).
        let actor_id = ObjectId::new(93);
        let container_id = ObjectId::new(103);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.container = Some(container_id);
        let mut container = snapshot_with_id(container_id.as_u64());
        container.entrance_status = false;
        container.ocf |= ocf::ENTRANCE;
        let mut objects = HashMap::from([(actor_id, actor), (container_id, container)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let mut state =
            ExitState::from_request(&CommandRequest::new(CommandId::Exit).with_update_interval(50))
                .expect("state created");

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let first_ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 100);
        let first = state.step(&first_ctx);
        assert!(matches!(
            first.events.as_slice(),
            [CommandEvent::CallObjectFunction { function, .. }] if function == "ActivateEntrance"
        ));

        objects
            .get_mut(&container_id)
            .expect("container present")
            .entrance_status = true;
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let next_ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 101);
        let next = state.step(&next_ctx);
        assert_eq!(next.status, CommandStatus::Completed);
        assert_eq!(next.update.and_then(|update| update.container), Some(None));
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
        container.entrance_status = true;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 30,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: attacker.position,
            object: objects.get(&attacker_id).expect("attacker present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
            landscape: None,
            frame: 0,
            position: attacker.position,
            object: objects.get(&attacker_id).expect("attacker present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn call_requires_function_name() {
        let request = CommandRequest::new(CommandId::Call).with_target(Some(ObjectId::new(99)));
        assert!(CallState::from_request(&request).is_err());
    }

    #[test]
    fn call_emits_event_and_completes() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let target2_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.command_direction = CommandDirection::Right;

        // C4Command::Call only requires a non-null Target pointer; it does
        // not require Alive (C4Command.cpp:2355-2365). Real targets include
        // nonliving structures such as Tutorial07's WRKS.
        let mut target = snapshot_with_id(target_id.as_u64());
        target.alive = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = CallState::from_request(
            &CommandRequest::new(CommandId::Call)
                .with_target(Some(target_id))
                .with_target2(Some(target2_id))
                .with_tx(Some(42))
                .with_ty(Some(7))
                .with_data(CommandData::Text("ControlCall".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        let update = result.update.expect("call should stop builder");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::CallObjectFunction {
                object_id,
                function,
                caller,
                tx,
                tx_definition,
                ty,
                target2,
                on_result,
            } => {
                assert_eq!(*object_id, target_id);
                assert_eq!(function, "ControlCall");
                assert_eq!(*caller, builder_id);
                assert_eq!(*tx, Some(42));
                assert!(tx_definition.is_none());
                assert_eq!(*ty, Some(7));
                assert_eq!(*target2, Some(target2_id));
                assert!(on_result.is_none());
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
    }

    #[test]
    fn context_requires_target_object() {
        let request = CommandRequest::new(CommandId::Context);
        assert!(ContextState::from_request(&request).is_err());
    }

    #[test]
    fn context_emits_menu_request() {
        let crew_id = ObjectId::new(77);
        let target_id = ObjectId::new(88);

        let mut crew = snapshot_with_id(crew_id.as_u64());
        crew.owner = 42;
        crew.command_direction = CommandDirection::Left;

        let target = snapshot_with_id(target_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(crew.id, crew.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: crew.position,
            object: objects.get(&crew_id).expect("crew present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ContextState::from_request(
            &CommandRequest::new(CommandId::Context)
                .with_target2(Some(target_id))
                .with_tx(Some(15))
                .with_ty(Some(25)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result.update.expect("context should stop crew");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::OpenMenu(request) => {
                assert_eq!(request.crew_id, crew_id);
                assert_eq!(request.owner, 42);
                match &request.kind {
                    MenuRequestKind::Context { target, position } => {
                        assert_eq!(*target, target_id);
                        assert_eq!(*position, Some(Vector2::new(15, 25)));
                    }
                    other => panic!("unexpected menu kind: {:?}", other),
                }
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
    }

    #[test]
    fn context_skips_menu_when_owner_none() {
        let crew_id = ObjectId::new(101);
        let target_id = ObjectId::new(202);

        let mut crew = snapshot_with_id(crew_id.as_u64());
        crew.owner = OWNER_NONE;
        crew.command_direction = CommandDirection::Right;

        let target = snapshot_with_id(target_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(crew.id, crew.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: crew.position,
            object: objects.get(&crew_id).expect("crew present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ContextState::from_request(
            &CommandRequest::new(CommandId::Context).with_target2(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_some());
        assert!(result.events.is_empty());
    }

    #[test]
    fn take_opens_activate_menu() {
        let crew_id = ObjectId::new(101);
        let container_id = ObjectId::new(102);

        let mut crew = snapshot_with_id(crew_id.as_u64());
        crew.owner = OWNER_NONE;
        crew.controller = 23;
        crew.command_direction = CommandDirection::Left;
        crew.container = Some(container_id);
        let container = snapshot_with_id(container_id.as_u64());

        let mut objects = HashMap::new();
        objects.insert(crew.id, crew.clone());
        objects.insert(container.id, container);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: crew.position,
            object: objects.get(&crew_id).expect("crew present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            TakeState::from_request(&CommandRequest::new(CommandId::Take)).expect("take state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::OpenMenu(request) => {
                assert_eq!(request.crew_id, crew_id);
                assert_eq!(request.owner, crew.controller);
                assert!(matches!(request.kind, MenuRequestKind::Activate));
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
        assert!(second.update.is_none());
    }

    #[test]
    fn take2_requires_container() {
        let crew_id = ObjectId::new(201);

        let mut crew = snapshot_with_id(crew_id.as_u64());
        crew.owner = 5;
        crew.command_direction = CommandDirection::Right;

        let mut objects = HashMap::new();
        objects.insert(crew.id, crew.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: crew.position,
            object: objects.get(&crew_id).expect("crew present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            Take2State::from_request(&CommandRequest::new(CommandId::Take2)).expect("take2 state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
        let update = result
            .update
            .expect("take2 should request crew to stop even on failure");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert!(result.events.is_empty());

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
    }

    #[test]
    fn take2_opens_get_menu_for_container() {
        let crew_id = ObjectId::new(301);
        let container_id = ObjectId::new(302);

        let mut crew = snapshot_with_id(crew_id.as_u64());
        crew.owner = 9;
        crew.command_direction = CommandDirection::Right;
        crew.container = Some(container_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.owner = 9;

        let mut objects = HashMap::new();
        objects.insert(crew.id, crew.clone());
        objects.insert(container.id, container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: crew.position,
            object: objects.get(&crew_id).expect("crew present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state =
            Take2State::from_request(&CommandRequest::new(CommandId::Take2)).expect("take2 state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        let update = result
            .update
            .expect("take2 should stop crew before opening menu");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::OpenMenu(request) => {
                assert_eq!(request.crew_id, crew_id);
                assert_eq!(request.owner, crew.owner);
                match &request.kind {
                    MenuRequestKind::Get { container } => assert_eq!(*container, container_id),
                    other => panic!("unexpected menu kind: {:?}", other),
                }
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
        assert!(second.events.is_empty());
    }

    #[test]
    fn transfer_requires_target() {
        let request = CommandRequest::new(CommandId::Transfer);
        assert!(TransferState::from_request(&request).is_err());
    }

    #[test]
    fn transfer_requests_move_when_outside_zone() {
        let actor_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 90);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 90);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            target_id,
            TransferZoneRect {
                x: 90,
                y: 80,
                width: 20,
                height: 20,
            },
        );

        let mut surface = vec![120; 200];
        surface[89] = 80;
        let landscape = crate::Landscape::new(200, surface).expect("transfer landscape");
        let ctx = CommandRuntimeContext {
            landscape: Some(&landscape),
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &transfer_zones,
            rng: None,
        };

        let mut state = TransferState::from_request(
            &CommandRequest::new(CommandId::Transfer).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.events.len(), 0);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.tx, Some(89));
                assert_eq!(request.ty, Some(69));
                assert_eq!(request.update_interval, 25);
            }
            other => panic!("unexpected operation: {:?}", other),
        }
    }

    #[test]
    fn transfer_emits_control_transfer_event() {
        let actor_id = ObjectId::new(100);
        let target_id = ObjectId::new(200);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(95, 0);
        actor.command_direction = CommandDirection::Right;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut transfer_zones = TransferZoneTable::default();
        transfer_zones.set(
            target_id,
            TransferZoneRect {
                x: 90,
                y: -10,
                width: 20,
                height: 40,
            },
        );

        let mut state = TransferState::from_request(
            &CommandRequest::new(CommandId::Transfer)
                .with_target(Some(target_id))
                .with_tx(Some(42))
                .with_ty(Some(-5)),
        )
        .expect("state created");

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 5,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &transfer_zones,
            rng: None,
        };

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        let update = result.update.expect("transfer should stop actor");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));
        assert_eq!(result.operations.len(), 0);
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::CallObjectFunction {
                object_id,
                function,
                caller,
                tx,
                tx_definition,
                ty,
                target2,
                on_result,
            } => {
                assert_eq!(*object_id, target_id);
                assert_eq!(function, "ControlTransfer");
                assert_eq!(*caller, actor_id);
                assert_eq!(*tx, Some(42));
                assert!(tx_definition.is_none());
                assert_eq!(*ty, Some(-5));
                assert!(target2.is_none());
                match on_result {
                    Some(CallResultAction::CompleteCommandOnFalse { command }) => {
                        assert_eq!(*command, CommandId::Transfer);
                    }
                    other => panic!("unexpected result action: {:?}", other),
                }
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let ctx_next = CommandRuntimeContext {
            landscape: None,
            frame: 6,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &transfer_zones,
            rng: None,
        };

        let follow_up = state.step(&ctx_next);
        assert_eq!(follow_up.status, CommandStatus::Running);
        assert!(follow_up.events.is_empty());
    }

    #[test]
    fn transfer_fails_without_zone() {
        let actor_id = ObjectId::new(123);
        let target_id = ObjectId::new(456);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(0, 0);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(10, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: objects.get(&actor_id).expect("actor present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = TransferState::from_request(
            &CommandRequest::new(CommandId::Transfer).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
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
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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

    // C4Command::Build (C4Command.cpp:823-899) accepts every extant target,
    // not only living objects. Its same-container arm at :887 is guarded by
    // Target->Contained, so two uncontained objects still have to approach.
    #[test]
    fn build_moves_to_uncontained_nonliving_structure() {
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(0, 0);
        builder.physical.can_construct = 1;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 0);
        target.status = ObjectStatus::Normal;
        target.alive = false;
        target.category = CATEGORY_STRUCTURE;
        target.construction = FULL_CON * 4 / 5;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(target.id, target);

        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = BuildState::from_request(
            &CommandRequest::new(CommandId::Build).with_target(Some(target_id)),
        )
        .expect("build state");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_none(), "must not build remotely");
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::MoveTo);
                assert_eq!(request.target, None);
                assert_eq!(request.tx, Some(100));
                assert_eq!(request.ty, Some(0));
            }
            other => panic!("expected MoveTo request, got {other:?}"),
        }
    }

    #[test]
    fn build_enters_the_container_of_an_internal_target() {
        // C4Command::Build ignores Tx/Ty and, when the incomplete target is
        // contained elsewhere, enters Target->Contained rather than walking
        // to the placeholder coordinates (C4Command.cpp:823-899). Workshop
        // passes explicit zero slots in AddCommand(...,"Build",pToBuild,...)
        // (Objects.c4d/Structures.c4d/Workshop.c4d/Script.c:76-91).
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);
        let workshop_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(8, 199);
        builder.physical.can_construct = 1;

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(150, 184);
        target.container = Some(workshop_id);
        target.alive = false;
        target.construction = FULL_CON / 100;

        let mut workshop = snapshot_with_id(workshop_id.as_u64());
        workshop.position = target.position;
        workshop.alive = false;
        workshop.category = CATEGORY_STRUCTURE;

        let objects = HashMap::from([
            (builder.id, builder.clone()),
            (target.id, target),
            (workshop.id, workshop),
        ]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state = BuildState::from_request(
            &CommandRequest::new(CommandId::Build)
                .with_target(Some(target_id))
                .with_tx(Some(0))
                .with_ty(Some(0)),
        )
        .expect("Build state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        match result.operations.as_slice() {
            [CommandOperation::PushFront(request)] => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(workshop_id));
                assert_eq!(request.update_interval, 50);
            }
            other => panic!("expected Enter workshop request, got {other:?}"),
        }
    }

    #[test]
    fn activate_explicit_container_opens_menu_before_movement_logic() {
        let actor_id = ObjectId::new(1);
        let container_id = ObjectId::new(2);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.owner = 17;
        actor.controller = 23;
        actor.command_direction = CommandDirection::Right;
        actor.action_procedure = ActionProcedure::Dig;
        let container = snapshot_with_id(container_id.as_u64());
        let objects = HashMap::from([(actor_id, actor.clone()), (container_id, container)]);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: &actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate).with_target2(Some(container_id)),
        )
        .expect("activate state");
        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.update.is_none());
        assert!(result.operations.is_empty());
        assert_eq!(
            result.events,
            [CommandEvent::OpenMenu(MenuRequest {
                crew_id: actor_id,
                owner: actor.controller,
                kind: MenuRequestKind::ActivateTarget {
                    container: container_id,
                },
            })]
        );

        let mut stale = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate)
                .with_target2(Some(ObjectId::new(999))),
        )
        .expect("stale activate state");
        let stale_result = stale.step(&ctx);
        assert_eq!(stale_result.status, CommandStatus::Failed);
        assert!(stale_result.events.is_empty());

        let mut deleted_container = snapshot_with_id(container_id.as_u64());
        deleted_container.status = ObjectStatus::Deleted;
        let deleted_objects =
            HashMap::from([(actor_id, actor.clone()), (container_id, deleted_container)]);
        let deleted_ctx = CommandRuntimeContext {
            objects: &deleted_objects,
            ..ctx
        };
        let mut deleted = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate).with_target2(Some(container_id)),
        )
        .expect("deleted-target activate state");
        let deleted_result = deleted.step(&deleted_ctx);
        assert_eq!(deleted_result.status, CommandStatus::Failed);
        assert!(deleted_result.events.is_empty());
    }

    #[test]
    fn activate_completes_when_target_outside_container() {
        let actor_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let actor = snapshot_with_id(actor_id.as_u64());

        let mut target = snapshot_with_id(target_id.as_u64());
        target.container = None;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: &actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        assert!(result.events.is_empty());
    }

    #[test]
    fn activate_requests_enter_when_actor_outside_container() {
        let actor_id = ObjectId::new(10);
        let container_id = ObjectId::new(20);
        let target_id = ObjectId::new(30);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.position = Vector2::new(100, 0);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(0, 0);
        container.ocf = ocf::ENTRANCE | ocf::AVAILABLE;
        container.contents.push(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = container.position;
        target.container = Some(container_id);
        target.collectible = true;
        target.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: &actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate)
                .with_target(Some(target_id))
                .with_update_interval(5),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(container_id));
            }
            other => panic!("expected enter request, got {:?}", other),
        }
    }

    #[test]
    fn activate_sets_exit_command_on_target_inside_container() {
        let actor_id = ObjectId::new(5);
        let container_id = ObjectId::new(6);
        let target_id = ObjectId::new(7);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.owner = 42;
        actor.controller = 23;
        actor.position = Vector2::new(15, 5);
        actor.container = Some(container_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(12, 4);
        container.contents.push(target_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = container.position;
        target.container = Some(container_id);
        target.collectible = true;
        target.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(container.id, container);
        objects.insert(target.id, target);

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: &actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = ActivateState::from_request(
            &CommandRequest::new(CommandId::Activate).with_target(Some(target_id)),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::SetObjectCommand {
                object_id,
                controller,
                request,
            } => {
                assert_eq!(*object_id, target_id);
                assert_eq!(*controller, 23);
                assert_eq!(request.id, CommandId::Exit);
            }
            other => panic!("unexpected event: {:?}", other),
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
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: true,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn energy_starts_a_power_line_at_the_nearby_supply() {
        // C4Command::Energy keeps running after it has a line kit: at the
        // supply it creates PWRL from the supply to that kit
        // (C4Command.cpp:2259-2289).
        let builder_id = ObjectId::new(10);
        let target_id = ObjectId::new(20);
        let supply_id = ObjectId::new(30);
        let linekit_id = ObjectId::new(40);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 1;
        builder.contents.push(linekit_id);

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(100, 0);
        target.line_connect = LINE_CONNECT_POWER_INPUT;

        let mut supply = snapshot_with_id(supply_id.as_u64());
        supply.definition_id = "POWR".into();
        supply.line_connect = crate::LINE_CONNECT_POWER_OUTPUT;
        supply.ocf |= ocf::POWER_SUPPLY;

        let mut linekit = snapshot_with_id(linekit_id.as_u64());
        linekit.definition_id = LINEKIT_DEFINITION.into();
        linekit.container = Some(builder_id);

        let mut objects = HashMap::new();
        objects.insert(target_id, target);
        objects.insert(supply_id, supply);
        objects.insert(linekit_id, linekit);
        let players = HashMap::new();
        let definitions = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: true,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };
        let mut state = EnergyState::from_request(
            &CommandRequest::new(CommandId::Energy).with_target(Some(target_id)),
        )
        .expect("energy state");

        let result = state.step(&ctx);

        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.iter().any(|event| matches!(
            event,
            CommandEvent::CreateLine {
                definition_id,
                owner: 1,
                from,
                to,
            } if definition_id == "PWRL"
                && *from == supply_id
                && *to == linekit_id
        )));
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
            landscape: None,
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn acquire_requests_get_for_candidate() {
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
        // Construction components are nonliving. C4Command::Acquire filters
        // by OCF_Available/full construction/fire, never OCF_Alive
        // (C4Command.cpp:2105-2132).
        item.alive = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script = state.step(&ctx);
        match script.events.first() {
            Some(CommandEvent::ControlCommandAcquire {
                caller,
                definition_id,
                ..
            }) => {
                assert_eq!(*caller, builder_id);
                assert_eq!(definition_id, "WOOD");
            }
            other => panic!("expected acquire control command, got {:?}", other),
        }
        assert!(script.operations.is_empty());

        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Get);
                assert_eq!(request.target, Some(item_id));
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("expected get request, got {:?}", other),
        }

        // C4CMD_Acquire InitEvaluation (C4Command.cpp:1666-1670) only
        // replaces a ZERO Tx/Ty with 500/250 — a negative range stays
        // negative and the Inside(cx-px, -Tx, +Tx) test (:2115-2116)
        // then matches nothing: the nearby item is NOT found and the
        // command falls through to Buy (:2136).
        assert_eq!(
            (state.range_x, state.range_y),
            (500, 250),
            "zero ranges default (C4Command.cpp:1668-1669)"
        );
        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire)
                .with_data(CommandData::Text("WOOD".into()))
                .with_tx(Some(-50))
                .with_ty(Some(-50)),
        )
        .expect("state created");
        assert_eq!(
            (state.range_x, state.range_y),
            (-50, -50),
            "negative ranges keep their sign (C4Command.cpp:1668 replaces only 0)"
        );
        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(
                    request.id,
                    CommandId::Buy,
                    "empty signed range finds no material -> Buy (C4Command.cpp:2136)"
                );
            }
            other => panic!("expected buy request, got {:?}", other),
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
            landscape: None,
            frame: 42,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script = state.step(&ctx);
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Get);
                assert_eq!(request.target, Some(item_id));
                assert_eq!(request.mode, CommandMode::SilentSub);
            }
            other => panic!("expected get request, got {:?}", other),
        }
        assert!(
            result.events.is_empty(),
            "acquire should delegate transfer to Get command"
        );
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
            landscape: None,
            frame: 100,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script = state.step(&ctx);
        dbg!(state.script_pending, script.events.len());
        state.script_result = Some(AcquireScriptResult::Continue);
        let result = state.step(&ctx);
        dbg!(state.script_pending, result.operations.len());
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.events.is_empty());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Get);
                assert_eq!(request.target, Some(item_id));
            }
            other => panic!("expected get request, got {:?}", other),
        }
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
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
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
            landscape: None,
            frame: 10,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let _ = state.step(&later_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let second = state.step(&later_ctx);
        assert!(second.operations.is_empty());
    }

    #[test]
    fn acquire_retries_buy_after_cooldown() {
        let builder_id = ObjectId::new(11);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let initial_ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let _ = state.step(&initial_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let initial = state.step(&initial_ctx);
        assert_eq!(initial.status, CommandStatus::Running);
        assert_eq!(initial.operations.len(), 1);
        match &initial.operations[0] {
            CommandOperation::PushFront(request) => assert_eq!(request.id, CommandId::Buy),
            other => panic!("expected initial buy request, got {:?}", other),
        }

        let mid_ctx = CommandRuntimeContext {
            landscape: None,
            frame: 60,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let _ = state.step(&mid_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let mid = state.step(&mid_ctx);
        assert_eq!(mid.status, CommandStatus::Running);
        assert!(
            mid.operations.is_empty(),
            "buy request should not repeat before cooldown elapses"
        );

        let retry_ctx = CommandRuntimeContext {
            landscape: None,
            frame: 150,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let _ = state.step(&retry_ctx);
        state.script_result = Some(AcquireScriptResult::Continue);
        let retry = state.step(&retry_ctx);
        assert_eq!(retry.status, CommandStatus::Running);
        let buy_requests: Vec<_> = retry
            .operations
            .iter()
            .filter_map(|operation| match operation {
                CommandOperation::PushFront(request) if request.id == CommandId::Buy => Some(()),
                _ => None,
            })
            .collect();
        assert_eq!(
            buy_requests.len(),
            1,
            "expected a single buy retry after cooldown elapsed"
        );
    }

    #[test]
    fn acquire_requests_get_when_in_other_container() {
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

        builder.position = current_container.position;

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
        objects.insert(builder.id, builder);
        objects.insert(current_container.id, current_container);
        objects.insert(target_container.id, target_container);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
            )
            .expect("command queued");

        let script = stack.step(&ctx).expect("script stage");
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        assert!(stack.set_acquire_script_result(AcquireScriptResult::Continue));
        let mut frame = ctx.frame + 1;
        let initial_len = stack.len();
        loop {
            let step_ctx = CommandRuntimeContext {
                landscape: None,
                frame,
                position: ctx.position,
                object: ctx.object,
                objects: ctx.objects,
                players: ctx.players,
                definitions: ctx.definitions,
                structures_need_energy: ctx.structures_need_energy,
                base_buy_enabled: ctx.base_buy_enabled,
                base_sell_enabled: ctx.base_sell_enabled,
                transfer_zones: ctx.transfer_zones,
                rng: None,
            };
            let step_result = stack.step(&step_ctx).expect("acquire evaluation");
            assert_eq!(step_result.status, CommandStatus::Running);
            if stack.len() > initial_len {
                break;
            }
            frame += 1;
            assert!(
                frame < 1000,
                "test timeout - no new command after {} frames",
                frame
            );
        }

        // Verify that a Get command was pushed to the stack
        assert_eq!(
            stack.len(),
            2,
            "expected Get command to be pushed onto stack"
        );
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

        builder.position = current_container.position;

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.construction = FULL_CON;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.collectible = true;
        item.position = Vector2::new(30, 0);

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder);
        objects.insert(current_container.id, current_container);
        objects.insert(item.id, item);

        let builder_snapshot = objects.get(&builder_id).expect("builder present");
        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
            )
            .expect("command queued");

        let script = stack.step(&ctx).expect("script stage");
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        assert!(stack.set_acquire_script_result(AcquireScriptResult::Continue));
        let mut frame = ctx.frame + 1;
        let initial_len = stack.len();
        loop {
            let step_ctx = CommandRuntimeContext {
                landscape: None,
                frame,
                position: ctx.position,
                object: ctx.object,
                objects: ctx.objects,
                players: ctx.players,
                definitions: ctx.definitions,
                structures_need_energy: ctx.structures_need_energy,
                base_buy_enabled: ctx.base_buy_enabled,
                base_sell_enabled: ctx.base_sell_enabled,
                transfer_zones: ctx.transfer_zones,
                rng: None,
            };
            let step_result = stack.step(&step_ctx).expect("acquire evaluation");
            assert_eq!(step_result.status, CommandStatus::Running);
            if stack.len() > initial_len {
                break;
            }
            frame += 1;
            assert!(
                frame < 1000,
                "test timeout - no new command after {} frames",
                frame
            );
        }

        // Verify that a Get command was pushed to the stack
        assert_eq!(
            stack.len(),
            2,
            "expected Get command to be pushed onto stack"
        );
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
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
            )
            .expect("command queued");

        let script = stack.step(&ctx).expect("script stage");
        assert!(matches!(
            script.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        assert!(stack.set_acquire_script_result(AcquireScriptResult::Continue));
        let initial_len = stack.len();
        let mut frame = ctx.frame + 1;
        loop {
            let step_ctx = CommandRuntimeContext {
                landscape: None,
                frame,
                position: ctx.position,
                object: ctx.object,
                objects: ctx.objects,
                players: ctx.players,
                definitions: ctx.definitions,
                structures_need_energy: ctx.structures_need_energy,
                base_buy_enabled: ctx.base_buy_enabled,
                base_sell_enabled: ctx.base_sell_enabled,
                transfer_zones: ctx.transfer_zones,
                rng: None,
            };
            let step_result = stack.step(&step_ctx).expect("acquire evaluation");
            assert_eq!(step_result.status, CommandStatus::Running);
            // `CommandStack::step` applies the command's operations to the stack
            // internally (it drains `result.operations`), so detect the requested
            // Get by the new front entry rather than by `result.operations` (which
            // is always empty here). The bounded frame guard keeps a never-pushed
            // regression a failure rather than an infinite hang.
            if stack.len() > initial_len {
                break;
            }
            frame += 1;
            assert!(
                frame < 1000,
                "test timeout - no Get command after {frame} frames"
            );
        }

        // Acquire should request a Get for the WOOD held inside the grabbable
        // container; verify the pushed sub-command targets the contained item.
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 2);
        match &snapshot.commands[0].state {
            CommandState::Get(state) => {
                assert_eq!(state.target, Some(item_id));
            }
            other => panic!("expected get command at front, got {other:?}"),
        }
        match &snapshot.commands[1].state {
            CommandState::Acquire(_) => {}
            other => panic!("expected acquire command beneath get, got {other:?}"),
        }
    }

    #[test]
    fn acquire_script_handled_skips_default_logic() {
        let builder_id = ObjectId::new(5);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let script_step = state.step(&ctx);
        assert!(matches!(
            script_step.events.first(),
            Some(CommandEvent::ControlCommandAcquire { .. })
        ));

        state.script_result = Some(AcquireScriptResult::Handled);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Running);
        assert!(second.operations.is_empty());
    }

    #[test]
    fn acquire_script_complete_finishes_command() {
        let builder_id = ObjectId::new(6);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Complete);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Completed);
    }

    #[test]
    fn acquire_script_failed_marks_command_failed() {
        let builder_id = ObjectId::new(7);
        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.ocf = ocf::AVAILABLE | ocf::ALIVE;
        builder.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: &builder,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = AcquireState::from_request(
            &CommandRequest::new(CommandId::Acquire).with_data(CommandData::Text("WOOD".into())),
        )
        .expect("state created");

        let _ = state.step(&ctx);
        state.script_result = Some(AcquireScriptResult::Failed);
        let second = state.step(&ctx);
        assert_eq!(second.status, CommandStatus::Failed);
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
            landscape: None,
            frame: 0,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let script_step = stack.step(&ctx_initial).expect("script step evaluates");
        assert_eq!(script_step.status, CommandStatus::Running);
        assert_eq!(
            stack.len(),
            1,
            "script phase should not enqueue additional commands"
        );
        assert!(
            matches!(
                script_step.events.first(),
                Some(CommandEvent::ControlCommandAcquire { .. })
            ),
            "expected control command event during first acquire evaluation"
        );

        assert!(
            stack.set_acquire_script_result(AcquireScriptResult::Continue),
            "script result should be stored on acquire state"
        );

        let second_step = stack.step(&ctx_initial).expect("second step evaluates");
        assert_eq!(second_step.status, CommandStatus::Running);
        assert_eq!(
            stack.len(),
            2,
            "move command should be queued after script phase"
        );

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 2);
        match &snapshot.commands[0].state {
            CommandState::Get(state) => {
                assert_eq!(
                    state.target,
                    Some(item_id),
                    "get command should target the acquire candidate"
                );
            }
            other => panic!("expected get command at front, got {:?}", other),
        }
        assert_eq!(snapshot.commands[0].mode, CommandMode::SilentSub);
        match &snapshot.commands[1].state {
            CommandState::Acquire(_) => {}
            other => panic!("expected acquire command second, got {:?}", other),
        }
        assert_eq!(snapshot.commands[1].mode, CommandMode::Base);

        let encoded = serde_json::to_value(&snapshot).expect("serialize snapshot");
        let acquire_entry = encoded["commands"]
            .as_array()
            .and_then(|commands| {
                commands
                    .iter()
                    .find(|entry| entry["state"].get("Acquire").is_some())
            })
            .expect("acquire state present");
        let acquire_state = &acquire_entry["state"]["Acquire"];
        let candidate = acquire_state["candidate"]
            .as_u64()
            .expect("candidate recorded");
        assert_eq!(candidate, item_id.as_u64());
        assert_eq!(snapshot.commands[0].failures, 0);

        let ctx_followup = CommandRuntimeContext {
            landscape: None,
            frame: 25,
            position: builder_snapshot.position,
            object: builder_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn command_stack_snapshot_preserves_buy_state() {
        let base_id = ObjectId::new(200);

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Buy)
                    .with_target(Some(base_id))
                    .with_data(CommandData::Text("WOOD".into()))
                    .with_update_interval(25),
            )
            .expect("buy command queued");

        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        match &snapshot.commands[0].state {
            CommandState::Buy(state) => {
                assert_eq!(state.target, Some(base_id));
                assert_eq!(state.update_interval, 25);
            }
            other => panic!("expected buy state, got {:?}", other),
        }
        let mut restored = CommandStack::new();
        restored.restore_from_snapshot(&snapshot);
        assert_eq!(restored.snapshot(), snapshot);
    }

    #[test]
    fn failing_subcommand_increments_base_failures_and_schedules_retry() {
        let actor_id = ObjectId::new(1);
        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;
        actor.position = Vector2::new(0, 0);

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor.position,
            object: &actor,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut stack = CommandStack::new();
        let wait_request = CommandRequest::new(CommandId::Wait)
            .with_update_interval(1)
            .with_retries(1);
        stack.push_back(wait_request).expect("wait command queued");
        stack
            .push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(ObjectId::new(999)))
                    .with_mode(CommandMode::SilentSub),
            )
            .expect("enter command queued");

        let initial_snapshot = stack.snapshot();
        assert_eq!(initial_snapshot.commands.len(), 2);
        match &initial_snapshot.commands[1].state {
            CommandState::Wait(_) => {
                assert_eq!(initial_snapshot.commands[1].retries, 1);
            }
            other => panic!("expected wait command as base, got {:?}", other),
        }

        let first = stack.step(&ctx).expect("enter should evaluate");
        assert_eq!(first.status, CommandStatus::Failed);
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(snapshot.commands[0].failures, 1);
        assert_eq!(snapshot.commands[0].retries, 1);

        let second = stack.step(&ctx).expect("wait should evaluate");
        assert_eq!(second.status, CommandStatus::Running);

        let post_snapshot = stack.snapshot();
        assert_eq!(post_snapshot.commands.len(), 2);
        match &post_snapshot.commands[1].state {
            CommandState::Wait(_) => {
                assert_eq!(post_snapshot.commands[1].failures, 0);
                assert_eq!(post_snapshot.commands[1].retries, 0);
            }
            other => panic!(
                "expected wait command after retry scheduling, got {:?}",
                other
            ),
        }
        match &post_snapshot.commands[0].state {
            CommandState::Retry(_) => {}
            other => panic!("expected retry command at front, got {:?}", other),
        }
    }

    #[test]
    fn command_stack_put_transfers_item_into_container() {
        let actor_id = ObjectId::new(20);
        let item_id = ObjectId::new(21);
        let container_id = ObjectId::new(22);

        let mut actor = snapshot_with_id(actor_id.as_u64());
        actor.ocf = ocf::AVAILABLE | ocf::ALIVE;
        actor.collectible = false;
        actor.position = Vector2::new(0, 0);
        actor.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "WOOD".into();
        item.position = actor.position;
        item.collectible = true;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.construction = FULL_CON;
        item.container = Some(actor_id);

        let mut container = snapshot_with_id(container_id.as_u64());
        container.position = Vector2::new(10, 0);
        container.collectible = false;
        container.category = CATEGORY_STRUCTURE;
        container.ocf = ocf::AVAILABLE | ocf::ENTRANCE;

        let mut objects = HashMap::new();
        objects.insert(actor.id, actor.clone());
        objects.insert(item.id, item.clone());
        objects.insert(container.id, container.clone());

        let players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let mut stack = CommandStack::new();
        stack
            .push_back(
                CommandRequest::new(CommandId::Put)
                    .with_target(Some(container_id))
                    .with_target2(Some(item_id)),
            )
            .expect("put enqueued");

        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: actor_snapshot.position,
            object: actor_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let result = stack.step(&ctx).expect("put evaluates");
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.operations.is_empty());
        assert_eq!(result.events.len(), 1);
        match &result.events[0] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(
                    update.container,
                    Some(Some(container_id)),
                    "item should enter destination container"
                );
                assert_eq!(update.position, Some(container.position));
            }
            other => panic!("unexpected put event: {:?}", other),
        }

        assert_eq!(stack.len(), 1, "Put finishes on the following execute");
        objects
            .get_mut(&item_id)
            .expect("item present")
            .container = Some(container_id);
        objects
            .get_mut(&actor_id)
            .expect("actor present")
            .contents
            .clear();
        let actor_snapshot = objects.get(&actor_id).expect("actor present");
        let ctx = move_to_ctx_at_frame(actor_snapshot, &objects, &players, &definitions, 1);
        let result = stack.step(&ctx).expect("Put observes transferred item");
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(stack.len(), 0);
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
        base.base = 42;
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
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 25,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                construction,
            } => {
                assert_eq!(definition_id, "WOOD");
                assert_eq!(*owner, 42);
                assert_eq!(*position, base.position);
                assert_eq!(*container, Some(base_id));
                assert_eq!(*construction, None);
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
        target.base = 42;
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
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 25,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn sell_without_definition_is_the_internal_menu_command() {
        // C4Command::Sell treats Data=0 as "open C4MN_Sell" rather than
        // rejecting the command (C4Command.cpp:2052-2057).
        let request = CommandRequest::new(CommandId::Sell);
        let state = SellState::from_request(&request).expect("menu command is valid");
        assert!(state.definition_id.is_empty());
    }

    #[test]
    fn sell_requests_enter_when_outside() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 7;
        builder.position = Vector2::new(-50, 0);
        builder.command_direction = CommandDirection::Left;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 7;
        base.base = 7;
        base.position = Vector2::new(0, 0);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;
        base.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "ORE1".into();
        item.collectible = true;
        item.container = Some(base_id);
        item.position = base.position;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());
        objects.insert(item.id, item);

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            7,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 100,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "ORE1".to_string(),
            CommandDefinitionSnapshot {
                value: 30,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = SellState::from_request(
            &CommandRequest::new(CommandId::Sell)
                .with_target(Some(base_id))
                .with_data(CommandData::Text("ORE1".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Running);
        assert!(result.update.is_some());
        assert_eq!(result.operations.len(), 1);
        match &result.operations[0] {
            CommandOperation::PushFront(request) => {
                assert_eq!(request.id, CommandId::Enter);
                assert_eq!(request.target, Some(base_id));
            }
            other => panic!("expected enter request, got {:?}", other),
        }
    }

    #[test]
    fn sell_completes_when_inside() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);
        let item_id = ObjectId::new(3);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 11;
        builder.container = Some(base_id);
        builder.position = Vector2::new(0, 0);
        builder.command_direction = CommandDirection::Right;

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 11;
        base.base = 11;
        base.position = Vector2::new(0, 0);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;
        base.contents.push(item_id);

        let mut item = snapshot_with_id(item_id.as_u64());
        item.definition_id = "ORE1".into();
        item.collectible = true;
        item.container = Some(base_id);
        item.position = base.position;
        item.ocf = ocf::AVAILABLE | ocf::FULL_CON;
        item.construction = FULL_CON;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());
        objects.insert(item.id, item.clone());

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            11,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 10,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "ORE1".to_string(),
            CommandDefinitionSnapshot {
                value: 15,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = SellState::from_request(
            &CommandRequest::new(CommandId::Sell)
                .with_target(Some(base_id))
                .with_data(CommandData::Text("ORE1".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.operations.is_empty());
        let update = result.update.expect("seller stops");
        assert_eq!(update.command_direction, Some(CommandDirection::Stop));

        assert_eq!(result.events.len(), 3);
        match &result.events[0] {
            CommandEvent::AdjustPlayerWealth { player_id, delta } => {
                assert_eq!(*player_id, 11);
                assert_eq!(*delta, 15);
            }
            event => panic!("unexpected event: {:?}", event),
        }

        match &result.events[1] {
            CommandEvent::AdjustPlayerHomeBaseMaterial {
                player_id,
                definition_id,
                delta,
            } => {
                assert_eq!(*player_id, 11);
                assert_eq!(definition_id, "ORE1");
                assert_eq!(*delta, 1);
            }
            event => panic!("unexpected event: {:?}", event),
        }

        match &result.events[2] {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                assert_eq!(*object_id, item_id);
                assert_eq!(update.status, Some(ObjectStatus::Deleted));
                assert_eq!(update.alive, Some(false));
                assert_eq!(update.container, Some(None));
            }
            event => panic!("unexpected event: {:?}", event),
        }

        let follow_up = state.step(&ctx);
        assert_eq!(follow_up.status, CommandStatus::Completed);
    }

    #[test]
    fn sell_fails_when_disabled() {
        let builder_id = ObjectId::new(1);
        let base_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.owner = 5;
        builder.container = Some(base_id);
        builder.position = Vector2::new(0, 0);

        let mut base = snapshot_with_id(base_id.as_u64());
        base.owner = 5;
        base.base = 5;
        base.position = Vector2::new(0, 0);
        base.category = CATEGORY_STRUCTURE;
        base.ocf = ocf::AVAILABLE | ocf::ENTRANCE;
        base.collectible = false;

        let mut objects = HashMap::new();
        objects.insert(builder.id, builder.clone());
        objects.insert(base.id, base.clone());

        let mut players: HashMap<i32, CommandPlayerSnapshot> = HashMap::new();
        players.insert(
            5,
            CommandPlayerSnapshot {
                status: PlayerStatus::Active,
                surrendered: false,
                wealth: 0,
                home_base_material: HashMap::new(),
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        );

        let definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: false,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
        };

        let mut state = SellState::from_request(
            &CommandRequest::new(CommandId::Sell)
                .with_target(Some(base_id))
                .with_data(CommandData::Text("ORE1".into())),
        )
        .expect("state created");

        let result = state.step(&ctx);
        assert_eq!(result.status, CommandStatus::Failed);
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
        target.base = 7;
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
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 5,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
        target.base = 5;
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
                home_base_material_entries: Vec::new(),
                knowledge: Vec::new(),
            },
        );

        let mut definitions: HashMap<DefinitionId, CommandDefinitionSnapshot> = HashMap::new();
        definitions.insert(
            "WOOD".to_string(),
            CommandDefinitionSnapshot {
                value: 15,
                can_chop: false,
                chop_action: None,
                constructable: false,
                grab: 0,
            },
        );

        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 10,
            position: builder.position,
            object: objects.get(&builder_id).expect("builder present"),
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    fn chop_sets_action_on_a_nonliving_tree_at_point_like_cpp() {
        // C4Command::Chop checks Target->At(cObj->x,cObj->y), plus the
        // horizontal 4..9 range, before starting ObjectComChop. The real
        // TRE2 shape reaches 28px vertically and trees are nonliving
        // objects (C4Command.cpp:778-812).
        let builder_id = ObjectId::new(1);
        let target_id = ObjectId::new(2);

        let mut builder = snapshot_with_id(builder_id.as_u64());
        builder.position = Vector2::new(6, 14);
        builder.command_direction = CommandDirection::Right;
        let builder_definition = builder.definition_id.clone();

        let mut target = snapshot_with_id(target_id.as_u64());
        target.position = Vector2::new(0, 0);
        target.shape = DefinitionRect::new(-20, -28, 40, 56);
        target.ocf = ocf::CHOP | ocf::AVAILABLE;
        target.alive = false;

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
                constructable: false,
                grab: 0,
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                constructable: false,
                grab: 0,
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                constructable: false,
                grab: 0,
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                constructable: false,
                grab: 0,
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
                constructable: false,
                grab: 0,
            },
        );

        let builder_entry = objects.get(&builder_id).expect("builder present");
        let ctx = CommandRuntimeContext {
            landscape: None,
            frame: 0,
            position: builder_entry.position,
            object: builder_entry,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: false,
            base_buy_enabled: true,

            base_sell_enabled: true,
            transfer_zones: &EMPTY_TRANSFER_ZONES,
            rng: None,
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
    /// C4Command::Tx is a tagged C4Value. Most commands use its integer
    /// payload, but Call must preserve a C4ID tag for script parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_definition: Option<DefinitionId>,
    pub ty: Option<i32>,
    pub data: CommandData,
    pub update_interval: u32,
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
            tx_definition: None,
            ty: None,
            data: CommandData::None,
            update_interval: 0,
            evaluated: false,
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
        self.tx_definition = None;
        self
    }

    pub fn with_tx_definition(mut self, definition_id: DefinitionId) -> Self {
        self.tx = definition_id_to_c4id(&definition_id);
        self.tx_definition = Some(definition_id);
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
    },
    /// ObjectComThrow -> ObjectActionThrow is one ordered operation: the
    /// action transition must succeed before Random(360) and C4Object::Exit
    /// run (C4ObjectCom.cpp:120-137).
    ThrowObject {
        actor_id: ObjectId,
        object_id: ObjectId,
        complete_command_on_success: bool,
    },
    /// Execute C4Command::Jump against the live object. ObjectComJump may run
    /// the object's OnActionJump hook synchronously, so it cannot be reduced
    /// to a pure snapshot update (C4Command.cpp:1056-1067;
    /// C4ObjectCom.cpp:48-61,280-307).
    ObjectComJump {
        object_id: ObjectId,
        tx: i32,
    },
    /// Run C4Command::Grab's live at-target sequence. Scale/hangle let-go
    /// and the target's RejectGrabbed callback must both finish before the
    /// actor enters Push (C4Command.cpp:689-706).
    AttemptGrab {
        actor_id: ObjectId,
        target_id: ObjectId,
    },
    /// Assign a fresh command stack to another object. C4CMD_Activate
    /// uses `Target->SetCommand(C4CMD_Exit)` rather than exiting the
    /// target inline (C4Command.cpp:1335-1362).
    SetObjectCommand {
        object_id: ObjectId,
        controller: i32,
        request: CommandRequest,
    },
    ControlCommandAcquire {
        caller: ObjectId,
        target: Option<ObjectId>,
        range_x: i32,
        range_y: i32,
        ignore_container: Option<ObjectId>,
        definition_id: DefinitionId,
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
    CallObjectFunction {
        object_id: ObjectId,
        function: String,
        caller: ObjectId,
        tx: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tx_definition: Option<DefinitionId>,
        ty: Option<i32>,
        target2: Option<ObjectId>,
        #[serde(default)]
        on_result: Option<CallResultAction>,
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
    CompleteCommandOnFalse { command: CommandId },
    CompleteCommandOnTrue { command: CommandId },
    /// C4Command::Finish(false): arm a command failure for its next
    /// Execute rather than completing it successfully.
    FailCommandOnFalse { command: CommandId },
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

pub(crate) fn definition_id_to_c4id(definition: &str) -> Option<i32> {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandSnapshot {
    state: CommandState,
    mode: CommandMode,
    retries: i32,
    failures: i32,
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

impl CommandSnapshot {
    fn new(entry: &ActiveCommand) -> Self {
        Self {
            state: entry.state.clone(),
            mode: entry.mode,
            retries: entry.retries,
            failures: entry.failures,
            request: entry.request.clone(),
            finished: entry.finished,
        }
    }
}

/// The FnGetCommand element view of one stack entry (C4Script.cpp:926-945):
/// name, Target, Tx, Ty, Target2, Data — the LIVE C4Command fields. The
/// creating CommandRequest is the base; states whose C++ counterpart
/// rewrites the fields (MoveTo's target absorption C4Command.cpp:1637,
/// Acquire's 500/250 range defaults :1666-1670, Construct's found site
/// :1757-1766) override it with their live values.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandView {
    pub name: String,
    pub target: Option<ObjectId>,
    pub tx: Option<i32>,
    pub tx_definition: Option<DefinitionId>,
    pub ty: Option<i32>,
    pub target2: Option<ObjectId>,
    pub data: CommandData,
}

impl CommandView {
    fn from_entry(name: String, request: Option<&CommandRequest>, state: &CommandState) -> Self {
        let mut view = Self {
            name,
            target: request.and_then(|request| request.target),
            tx: request.and_then(|request| request.tx),
            tx_definition: request.and_then(|request| request.tx_definition.clone()),
            ty: request.and_then(|request| request.ty),
            target2: request.and_then(|request| request.target2),
            data: request
                .map(|request| request.data.clone())
                .unwrap_or(CommandData::None),
        };
        state.apply_live_overrides(&mut view);
        view
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CommandStackSnapshot {
    commands: Vec<CommandSnapshot>,
    #[serde(skip)]
    detached_grab_attempts: Vec<DetachedGrabAttempt>,
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
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct CommandStack {
    entries: VecDeque<ActiveCommand>,
    detached_grab_attempts: Vec<DetachedGrabAttempt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DetachedGrabAttempt {
    target: ObjectId,
    target_retained: bool,
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
            detached_grab_attempts: Vec::new(),
        }
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
            .map_or(true, |request| request.target == Some(state.target));
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

    fn pop_front(&mut self) -> Option<ActiveCommand> {
        let entry = self.entries.pop_front()?;
        self.remember_detached_grab(&entry);
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
        self.entries.clear();
    }

    pub fn snapshot(&self) -> CommandStackSnapshot {
        CommandStackSnapshot {
            commands: self.entries.iter().map(CommandSnapshot::new).collect(),
            detached_grab_attempts: self.detached_grab_attempts.clone(),
        }
    }

    pub fn restore_from_snapshot(&mut self, snapshot: &CommandStackSnapshot) {
        self.entries = snapshot
            .commands
            .iter()
            .cloned()
            .map(ActiveCommand::from_snapshot)
            .collect();
        self.detached_grab_attempts = snapshot.detached_grab_attempts.clone();
    }

    /// C4Command::DenumeratePointers resolves the saved Target/Target2
    /// object numbers only after the complete object table has loaded
    /// (C4Command.cpp:2417-2421; C4Object.cpp:2914-2929).
    pub(crate) fn denumerate_object_references(&mut self, object_numbers: &HashSet<u64>) {
        for entry in &mut self.entries {
            if let Some(request) = &mut entry.request {
                denumerate_object_reference(&mut request.target, object_numbers);
                denumerate_object_reference(&mut request.target2, object_numbers);
            }
            entry.state.denumerate_object_references(object_numbers);
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
            }
            changed |= entry.state.clear_object_reference(removed);
        }
        changed
    }

    /// Execute the live front while retaining a finished entry for
    /// C4Object::ExecuteCommand's callback/clear tail.
    pub fn execute_front(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
    ) -> Option<CommandStepResult> {
        let (mode, mut result) = {
            let front = self.entries.front_mut()?;
            if front.finished.is_some() {
                return None;
            }
            let mode = front.mode;
            let result = front.step(ctx);
            if matches!(
                result.status,
                CommandStatus::Completed | CommandStatus::Failed
            ) {
                front.finished = Some(result.status);
            }
            (mode, result)
        };

        if result.status == CommandStatus::Failed {
            self.record_failure(mode);
        }

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
                CommandOperation::Restore(snapshot) => self.restore_from_snapshot(&snapshot),
            }
        }
        Some(result)
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
    /// the command after the event's synchronous callbacks return.
    pub fn finish_front_if(&mut self, id: CommandId) -> bool {
        if let Some(front) = self.entries.front_mut() {
            if front.id() == Some(id) {
                front.finished = Some(CommandStatus::Completed);
                return true;
            }
        }
        false
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
                return Some(
                    self.detached_grab_attempts
                        .remove(index)
                        .target_retained,
                );
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
            .map_or(true, |request| request.target == Some(target));
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
        let mode = {
            let command = &mut self.entries[index];
            if direct {
                command.mode = CommandMode::SilentBase;
            }
            if let CommandState::Grab(state) = &mut command.state {
                state.reject_pending = false;
            }
            command.finished = Some(CommandStatus::Failed);
            command.mode
        };
        if !direct && matches!(mode, CommandMode::SilentSub | CommandMode::Sub) {
            if let Some(base) = self
                .entries
                .iter_mut()
                .skip(index + 1)
                .find(|entry| entry.finished.is_none())
            {
                base.failures = base.failures.saturating_add(1);
            }
        }
        Some(target_retained)
    }

    pub fn set_acquire_script_result(&mut self, result: AcquireScriptResult) -> bool {
        for entry in &mut self.entries {
            if let CommandState::Acquire(state) = &mut entry.state {
                state.script_result = Some(result);
                return true;
            }
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
    pub fn finish_entry_public(&mut self, index: i32, success: bool) {
        self.finish_entry(index, success);
    }

    fn finish_entry(&mut self, index: i32, success: bool) {
        if index < 0 {
            return;
        }
        let index = index as usize;
        if index >= self.entries.len() {
            return;
        }
        if success {
            if let Some(entry) = self.entries.get_mut(index) {
                entry.finished = Some(CommandStatus::Completed);
            }
        } else if let Some(entry) = self.entries.get_mut(index) {
            entry.failures = entry.failures.saturating_add(1);
        }
    }

    fn record_failure(&mut self, mode: CommandMode) {
        match mode {
            CommandMode::SilentSub | CommandMode::Sub => {
                if let Some(base) = self
                    .entries
                    .iter_mut()
                    .find(|entry| matches!(entry.mode, CommandMode::Base | CommandMode::SilentBase))
                {
                    base.failures = base.failures.saturating_add(1);
                }
            }
            CommandMode::SilentBase => {}
            CommandMode::Base => {}
        }
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

/// `Angle` (C4Math.cpp:33-45): 0 = up, 90 = right, clockwise; float
/// atan2 truncated like the C++ `static_cast<int>`.
fn c4_angle(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dy = (y1 - y2).abs() as f32;
    let dx = (x1 - x2).abs() as f32;
    let angle = (180.0_f64
        * f64::from(dy.atan2(dx))
        * f64::from(std::f32::consts::FRAC_1_PI)) as i32;
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
fn c4_distance(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
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
        if landscape.is_solid_at(*x, *y + offset)
            && !landscape.is_solid_at(*x, *y - offset)
        {
            *y -= 1;
        }
        if landscape.is_solid_at(*x, *y - offset)
            && !landscape.is_solid_at(*x, *y + offset)
        {
            *y += 1;
        }
    }
    for offset in 1..x_offset {
        if landscape.is_solid_at(*x + offset, *y)
            && !landscape.is_solid_at(*x - offset, *y)
        {
            *x -= 1;
        }
        if landscape.is_solid_at(*x - offset, *y)
            && !landscape.is_solid_at(*x + offset, *y)
        {
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
            update_interval: request.update_interval,
            tolerance: 5,
            last_direction: CommandDirection::Stop,
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
    fn init_evaluation(&mut self, ctx: &CommandRuntimeContext<'_>) {
        if let Some(target) = self.target.take() {
            if let Some(position) = ctx.resolve_position(target) {
                self.tx = Some(self.tx.unwrap_or(0) + position.x);
                self.ty = Some(self.ty.unwrap_or(0) + position.y);
            }
        }
        if self.data & COMMAND_FLAG_MOVE_TO_NO_POS_ADJUST == 0 {
            if let (Some(landscape), Some(tx), Some(ty)) = (ctx.landscape, self.tx, self.ty) {
                let free_move = ctx.object.action_procedure == ActionProcedure::Float
                    || ctx.object.physical.can_fly != 0;
                let (mut x, mut y) = (tx, ty);
                adjust_move_to_target(landscape, &mut x, &mut y, free_move, ctx.object.shape.height);
                self.tx = Some(x);
                self.ty = Some(y);
            }
        }
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        // C4Command::Execute treats UpdateInterval as the command's
        // remaining lifetime, not as an evaluation throttle
        // (C4Command.cpp:1545-1555).
        if self.update_interval > 0 {
            self.update_interval -= 1;
            if self.update_interval == 0 {
                return CommandStepResult::completed(None);
            }
        }

        // The initial-evaluation Execute consumes the frame without
        // moving (`if (InitEvaluation()) return;`, C4Command.cpp:1555).
        if !self.evaluated {
            self.evaluated = true;
            self.init_evaluation(ctx);
            return CommandStepResult::running(None);
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
            None => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::failed(Some(update));
            }
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
                    let transfer_zones = ctx.transfer_zones.states();
                    let mut finder = PathFinder::new(landscape, &transfer_zones);
                    finder.set_level(ctx.object.pathfinder);
                    finder.enable_transfer_zones(ctx.object.no_transfer_zones == 0);
                    match finder.find(ctx.position, target) {
                        Some(path) if path.waypoints.len() > 2 => {
                            let waypoint_count = path.waypoints.len();
                            let mut operations = Vec::with_capacity(waypoint_count - 2);
                            for waypoint in path
                                .waypoints
                                .into_iter()
                                .skip(1)
                                .take(waypoint_count - 2)
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
                        Some(_) => self.path_checked = true,
                        None => {
                            self.path_checked = true;
                            return CommandStepResult::running(None);
                        }
                    }
                }
            }
        }
        if ctx.frame % 35 == 0 {
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

        // Push/pull movers measure from the pushed vehicle's position
        // (C4Command.cpp:271-277; the fWaypoint skip needs the pathfinder
        // waypoint stack, which is not ported yet).
        let mut position = ctx.position;
        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Push | ActionProcedure::Pull
        ) {
            if let Some(vehicle) = ctx
                .object
                .action_target
                .and_then(|id| ctx.resolve_position(id))
            {
                position = vehicle;
            }
        }

        let dx = target.x - position.x;
        let dy = target.y - position.y;
        // Crew use their shape width rather than the global MoveToRange:
        // `iTargetRange = Shape.Wdt / 5` (C4Command.cpp:286-292).
        let target_range = if ctx.object.ocf & ocf::CREW_MEMBER != 0 {
            ctx.object.shape.width / 5
        } else if ctx.object.move_to_range > 0 {
            ctx.object.move_to_range
        } else {
            self.tolerance
        };
        if dx.abs() <= target_range && dy.abs() <= target_range {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            self.last_direction = CommandDirection::Stop;
            return CommandStepResult::completed(Some(update));
        }

        let float_steering = ctx.object.action_procedure == ActionProcedure::Float;
        let direction = match ctx.object.action_procedure {
            // DFA_SWIM (C4Command.cpp:370-382): Tick2 frames (Game.iTick2
            // != 0 — odd FrameCounter) steer horizontally with the target
            // range; !Tick2 frames steer vertically toward Ty with no
            // range. ComDir is left alone when no condition hits.
            ActionProcedure::Swim => {
                if ctx.frame % 2 != 0 {
                    if dx > target_range {
                        CommandDirection::Right
                    } else if dx < -target_range {
                        CommandDirection::Left
                    } else {
                        self.last_direction
                    }
                } else if dy > 0 {
                    CommandDirection::Down
                } else if dy < 0 {
                    CommandDirection::Up
                } else {
                    self.last_direction
                }
            }
            // DFA_SCALE (C4Command.cpp:335-338): vertical steering only —
            // cy > Ty + range climbs Up, cy < Ty - range slides Down.
            ActionProcedure::Scale => {
                if dy < -target_range {
                    CommandDirection::Up
                } else if dy > target_range {
                    CommandDirection::Down
                } else {
                    self.last_direction
                }
            }
            // DFA_FLIGHT (C4Command.cpp:414-417): no ComDir steering —
            // only FlightControl runs (below).
            ActionProcedure::Flight => self.last_direction,
            // DFA_PUSH/DFA_PULL (C4Command.cpp:329-333): horizontal
            // steering only, measured from the vehicle position above.
            ActionProcedure::Push | ActionProcedure::Pull => {
                if dx > target_range {
                    CommandDirection::Right
                } else if dx < -target_range {
                    CommandDirection::Left
                } else {
                    self.last_direction
                }
            }
            // DFA_HANGLE (C4Command.cpp:384-387): horizontal steering
            // only; the angle-based drop follows below.
            ActionProcedure::Hang => {
                if dx > target_range {
                    CommandDirection::Right
                } else if dx < -target_range {
                    CommandDirection::Left
                } else {
                    self.last_direction
                }
            }
            // DFA_FLOAT (C4Command.cpp:393-410): normalize the fixed-point
            // target vector to Physical.Float, subtract current momentum,
            // then choose the closest of the eight control directions.
            ActionProcedure::Float => {
                let mut fixed_dx = math::itofix(target.x) - ctx.object.fixed_position.x;
                let mut fixed_dy = math::itofix(target.y) - ctx.object.fixed_position.y;
                let scale = math::fixed100(ctx.object.physical.float)
                    / fixed_dx.abs().max(fixed_dy.abs());
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
                } else if fixed_dx > crate::C4Fixed::ZERO
                    && fixed_dy > crate::C4Fixed::ZERO
                {
                    CommandDirection::DownRight
                } else if fixed_dx < crate::C4Fixed::ZERO
                    && fixed_dy > crate::C4Fixed::ZERO
                {
                    CommandDirection::DownLeft
                } else if fixed_dx > crate::C4Fixed::ZERO
                    && fixed_dy < crate::C4Fixed::ZERO
                {
                    CommandDirection::UpRight
                } else {
                    CommandDirection::UpLeft
                }
            }
            _ => {
                if dx > target_range {
                    CommandDirection::Right
                } else if dx < -target_range {
                    CommandDirection::Left
                } else if dy > target_range {
                    CommandDirection::Down
                } else if dy < -target_range {
                    CommandDirection::Up
                } else {
                    CommandDirection::Stop
                }
            }
        };

        // The C++ Float arm writes ComDir every execution. In particular,
        // COMD_None must stop momentum correction even when this new command
        // has not observed the object's pre-existing ComDir.
        let steer = if float_steering || direction != self.last_direction {
            self.last_direction = direction;
            Some(direction)
        } else {
            None
        };

        // DFA_SCALE let-go control (C4Command.cpp:339-368): jump off the
        // wall toward the target or on wall contact; the C++ `return`
        // ends this Execute with the command still pending.
        if ctx.object.action_procedure == ActionProcedure::Scale {
            if let Some(xdirf) = self.scale_let_go(ctx, target) {
                return CommandStepResult::running(Some(let_go_update(steer, xdirf)));
            }
        }

        // DFA_HANGLE let-go control (C4Command.cpp:388-390): drop off the
        // ceiling once the target angle leaves the hangling sector.
        if ctx.object.action_procedure == ActionProcedure::Hang
            && c4_angle(ctx.position.x, ctx.position.y, target.x, target.y).abs()
                > LET_GO_HANGLE_ANGLE
        {
            return CommandStepResult::running(Some(let_go_update(steer, 0)));
        }

        // DFA_WALK movement controls, after the ComDir steering
        // (C4Command::Execute MoveTo, C4Command.cpp:316-326):
        // FlightControl never short-circuits (it returns false even after
        // taking off, :1816-1849); JumpControl returning true ends the
        // Execute for this tick. DFA_FLIGHT runs FlightControl alone
        // (:414-417).
        let mut fly_update: Option<ActionUpdate> = None;
        let mut jump_operations: Option<Vec<CommandOperation>> = None;
        if ctx.object.action_procedure == ActionProcedure::Walk {
            fly_update = self.flight_control(ctx, target);
            jump_operations = self.jump_control(ctx, target);
        } else if ctx.object.action_procedure == ActionProcedure::Flight {
            fly_update = self.flight_control(ctx, target);
        }

        if fly_update.is_some() || jump_operations.is_some() {
            let mut update = ObjectUpdate::new();
            if let Some(direction) = steer {
                update = update.with_command_direction(direction);
            }
            if let Some(action) = fly_update {
                update = update.with_action_update(action);
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

    /// The DFA_SCALE let-go decision (C4Command.cpp:339-368): jump away
    /// from the wall (xdir sign opposite the scaling side) when the
    /// target lies off the wall beyond LetGoRange1 within LetGoRange2
    /// vertically, or on any contact once the action is 3+ frames old.
    fn scale_let_go(&self, ctx: &CommandRuntimeContext<'_>, target: Vector2) -> Option<i32> {
        let (cx, cy) = (ctx.position.x, ctx.position.y);
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

    /// `C4Command::FlightControl` (C4Command.cpp:1816-1849): CanFly crew
    /// walking toward a distant target within ±60° of straight up takes
    /// off (SetActionByName("Fly")); C++ always returns false, so the
    /// jump control still runs. The ActMap Disabled check (:1824-1828)
    /// is unmodeled (no Disabled flag in the snapshot).
    fn flight_control(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        target: Vector2,
    ) -> Option<ActionUpdate> {
        if ctx.object.physical.can_fly == 0 {
            return None;
        }
        if ctx.object.ocf & crate::ocf::CREW_MEMBER == 0 {
            return None;
        }
        let landscape = ctx.landscape?;
        let (cx, cy) = (ctx.position.x, ctx.position.y);
        let mut angle = c4_angle(cx, cy, target.x, target.y);
        while angle > 180 {
            angle -= 360;
        }
        if !inside(angle, -FLIGHT_ANGLE_RANGE, FLIGHT_ANGLE_RANGE) {
            return None;
        }
        if c4_distance(cx, cy, target.x, target.y) <= 30 {
            return None;
        }
        let mut top_free = 0;
        while top_free < 50 && !landscape.is_solid_at(cx, cy + ctx.object.shape_top - top_free) {
            top_free += 1;
        }
        if top_free < 15 {
            return None;
        }
        Some(ActionUpdate::default().with_name("Fly"))
    }

    /// `C4Command::JumpControl` (C4Command.cpp:1851-1920): the three
    /// walking-jump triggers. Gate: OCF_CrewMember (Def->Pathfinder has
    /// no DefCore parse yet — documented gap).
    fn jump_control(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        target: Vector2,
    ) -> Option<Vec<CommandOperation>> {
        if ctx.object.ocf & crate::ocf::CREW_MEMBER == 0 {
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
    target: ObjectId,
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
        let Some(target_snapshot) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        };

        // C4Command::Enter has no aliveness gate on the target — dead
        // structures are entered fine; only removal clears the pointer
        // (C4Command.cpp:545-560).
        if target_snapshot.destroyed || !target_snapshot.status.is_active() {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        if ctx.object.container == Some(self.target) {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        if target_snapshot.ocf & ocf::ENTRANCE == 0 {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        }

        // "If in entrance range": C4Command::Enter tests the clonk point
        // against the target's shape (Target->At(cx, cy, ocf),
        // C4Command.cpp:586-588).
        if target_snapshot.at_point(ctx.position.x, ctx.position.y) {
            let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
            if !target_snapshot.entrance_status {
                let event = CommandEvent::CallObjectFunction {
                    object_id: self.target,
                    function: "ActivateEntrance".into(),
                    caller: ctx.object.id,
                    tx: None,
                    tx_definition: None,
                    ty: None,
                    target2: None,
                    on_result: None,
                };
                return CommandStepResult::running(Some(update)).with_events(vec![event]);
            }
            let event = CommandEvent::EnterObject {
                object_id: ctx.object.id,
                container_id: self.target,
            };
            return CommandStepResult::completed(Some(update)).with_events(vec![event]);
        }

        let mut result = CommandStepResult::running(self.update_to_stop(ctx));
        if self.should_issue_move(ctx.frame) {
            // Move to the entrance with the push flag carried through:
            // (Data & C4CMD_Enter_PushTarget) ? C4CMD_MoveTo_PushTarget
            // : 0 (C4Command.cpp:615).
            let mut request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(self.target))
                .with_update_interval(50);
            if self.push_target {
                request =
                    request.with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET));
            }
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
        self.update_to_stop(ctx).unwrap_or_default()
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(container_id) = ctx.object.container else {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        };

        let mut update = self.prepare_update(ctx);

        if ctx.object.action_procedure == ActionProcedure::Build {
            let action_update = ActionUpdate::default().with_name("Idle").with_force(true);
            update = update.with_action_update(action_update);
        }

        let container_snapshot = match ctx.resolve(container_id) {
            // C4Command::Exit only needs the live Contained pointer;
            // structures are nonliving but valid containers.
            Some(snapshot) if snapshot.is_status_active() => snapshot,
            _ => {
                update.container = Some(None);
                update.position = Some(ctx.position);
                update.velocity = Some(Vector2::ZERO);
                return CommandStepResult::completed(Some(update));
            }
        };

        // A closed entrance is not an ejection point. C++ asks the
        // container to open and leaves this Exit command pending; a false
        // ActivateEntrance result fails it (C4Command.cpp:644-650).
        if !container_snapshot.entrance_status {
            let event = CommandEvent::CallObjectFunction {
                object_id: container_id,
                function: "ActivateEntrance".into(),
                caller: ctx.object.id,
                tx: None,
                tx_definition: None,
                ty: None,
                target2: None,
                on_result: Some(CallResultAction::FailCommandOnFalse {
                    command: CommandId::Exit,
                }),
            };
            return CommandStepResult::running(Some(update)).with_events(vec![event]);
        }

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
            update.position = if container_snapshot.entrance_status
                && container_snapshot.ocf & ocf::ENTRANCE != 0
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
            }
            .or(Some(container_snapshot.position));
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
        // C4Command::Build approaches Target itself; unlike Construct, its
        // Tx/Ty fields are not a construction-site override
        // (C4Command.cpp:823-899).
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

        if !target_snapshot.is_status_active() {
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

        let same_container =
            target_snapshot.container.is_some() && builder.container == target_snapshot.container;
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
            operations.push(CommandOperation::PushFront(request));
        }

        CommandStepResult::running(None).with_operations(operations)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ConstructState {
    target: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    definition_id: Option<DefinitionId>,
    site: Option<Vector2>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    acquire_requested: bool,
    exit_requested: bool,
    ungrab_requested: bool,
    spawn_requested: bool,
    construction_id: Option<ObjectId>,
}

impl ConstructState {
    fn from_request(request: &CommandRequest) -> Self {
        let definition_id = command_data_to_definition_id(&request.data);
        let site = match (request.tx, request.ty) {
            (Some(x), Some(y)) => Some(Vector2::new(x, y)),
            _ => None,
        };
        Self {
            target: request.target,
            definition_id,
            site,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            acquire_requested: false,
            exit_requested: false,
            ungrab_requested: false,
            spawn_requested: false,
            construction_id: None,
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

    fn builder_has_conkit(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        ctx.object.contents.iter().copied().find(|id| {
            ctx.resolve(*id)
                .map(|snapshot| snapshot.definition_id == CONKIT_DEFINITION && snapshot.is_active())
                .unwrap_or(false)
        })
    }

    fn at_site(&self, ctx: &CommandRuntimeContext<'_>, site: Vector2) -> bool {
        const APPROACH_HORIZONTAL: i32 = 9;
        const APPROACH_VERTICAL: i32 = 20;
        let dx = site.x - ctx.position.x;
        let dy = site.y - ctx.position.y;
        dx.abs() <= APPROACH_HORIZONTAL && dy.abs() <= APPROACH_VERTICAL
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
            .map(|snapshot| snapshot.id)
            .next()
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < self.update_interval as u64 {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        // C4Command::Construct applies the physical capability gate before
        // both the menu-opening Data=0 path and definition validation.
        if ctx.object.physical.can_construct == 0 {
            return CommandStepResult::failed(None);
        }

        let Some(definition_id) = self.definition_id.clone() else {
            return CommandStepResult::completed(None).with_events(vec![
                CommandEvent::OpenMenu(MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind: MenuRequestKind::Construction,
                }),
            ]);
        };

        let update_to_stop = self.update_to_stop(ctx);

        if self.target.is_some() {
            return CommandStepResult::failed(update_to_stop);
        }

        let owner = ctx.object.owner;
        if owner == OWNER_NONE {
            return CommandStepResult::failed(update_to_stop);
        }

        let definition = match ctx.definition(&definition_id) {
            Some(definition) => definition,
            None => return CommandStepResult::failed(update_to_stop),
        };

        if !definition.constructable {
            return CommandStepResult::failed(update_to_stop);
        }

        let player = match ctx.player(owner) {
            Some(player) if player.is_active() => player,
            _ => return CommandStepResult::failed(update_to_stop),
        };

        if !player.knows(&definition_id) {
            return CommandStepResult::failed(update_to_stop);
        }

        if ctx.object.container.is_some() {
            if !self.exit_requested {
                self.exit_requested = true;
                let mut result = CommandStepResult::running(update_to_stop.clone());
                let request = CommandRequest::new(CommandId::Exit)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update_to_stop);
        }
        self.exit_requested = false;

        if ctx.object.action_procedure == ActionProcedure::Push {
            if !self.ungrab_requested {
                self.ungrab_requested = true;
                let mut result = CommandStepResult::running(update_to_stop.clone());
                let request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update_to_stop);
        }
        self.ungrab_requested = false;

        let site = match self.site {
            Some(site) => site,
            None => return CommandStepResult::failed(update_to_stop),
        };

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Build | ActionProcedure::Chop | ActionProcedure::Dig
        ) {
            let mut update = update_to_stop.unwrap_or_default();
            let idle_action = ActionUpdate::default().with_name("Idle").with_force(true);
            update = update.with_action_update(idle_action);
            return CommandStepResult::running(Some(update));
        }

        if !self.spawn_requested {
            let kit_id = match self.builder_has_conkit(ctx) {
                Some(id) => id,
                None => {
                    if !self.acquire_requested {
                        if let Some(c4id) = definition_id_to_c4id(CONKIT_DEFINITION) {
                            let request = CommandRequest::new(CommandId::Acquire)
                                .with_data(CommandData::Integer(c4id))
                                .with_update_interval(ACQUIRE_REQUEST_INTERVAL)
                                .with_mode(CommandMode::Sub);
                            let mut result = CommandStepResult::running(update_to_stop.clone());
                            result.operations.push(CommandOperation::PushFront(request));
                            self.acquire_requested = true;
                            return result;
                        }
                        return CommandStepResult::failed(update_to_stop);
                    }
                    return CommandStepResult::running(update_to_stop);
                }
            };
            self.acquire_requested = false;

            if !self.at_site(ctx, site) {
                if self.should_issue_move(ctx.frame) {
                    let request = CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(site.x))
                        .with_ty(Some(site.y))
                        .with_update_interval(10);
                    let mut result = CommandStepResult::running(update_to_stop.clone());
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
                return CommandStepResult::running(update_to_stop);
            }

            let mut events = Vec::new();
            events.push(CommandEvent::SpawnObject {
                definition_id: definition_id.clone(),
                owner,
                position: site,
                container: None,
                construction: Some(1),
            });

            let mut kit_update = ObjectUpdate::new();
            kit_update.container = Some(None);
            kit_update.position = Some(ctx.position);
            kit_update.velocity = Some(Vector2::ZERO);
            kit_update.status = Some(ObjectStatus::Deleted);
            kit_update.alive = Some(false);
            events.push(CommandEvent::ApplyObjectUpdate {
                object_id: kit_id,
                update: kit_update,
            });

            self.spawn_requested = true;
            return CommandStepResult::running(update_to_stop).with_events(events);
        }

        if self.construction_id.is_none() {
            if let Some(construction_id) =
                self.find_spawned_construction(ctx, &definition_id, site)
            {
                self.construction_id = Some(construction_id);
            } else {
                return CommandStepResult::running(update_to_stop);
            }
        }

        let construction_id = self.construction_id.expect("construction id present");
        let mut operations = Vec::new();
        operations.push(CommandOperation::PushFront(
            CommandRequest::new(CommandId::Build)
                .with_target(Some(construction_id))
                .with_tx(Some(site.x))
                .with_ty(Some(site.y))
                .with_update_interval(50)
                .with_mode(CommandMode::Sub),
        ));

        CommandStepResult::completed(update_to_stop).with_operations(operations)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TransferState {
    target: ObjectId,
    tx: Option<i32>,
    ty: Option<i32>,
    last_move_order: Option<u64>,
    last_script_call: Option<u64>,
}

impl TransferState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            tx: request.tx,
            ty: request.ty,
            last_move_order: None,
            last_script_call: None,
        })
    }

    fn update_to_stop(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        if ctx.object.command_direction != CommandDirection::Stop {
            Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
        } else {
            None
        }
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

        let mut x = target_x.clamp(zone.x - 1, zone.x + zone.width);
        let mut y = target_y.clamp(zone.y - 1, zone.y + zone.height);
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

    fn should_call_script(&mut self, frame: u64) -> bool {
        if frame % 5 != 0 {
            return false;
        }
        if self.last_script_call == Some(frame) {
            return false;
        }
        self.last_script_call = Some(frame);
        true
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let Some(target_snapshot) = ctx.resolve(self.target) else {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        };
        if !target_snapshot.is_active() {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        }
        let Some(zone) = ctx.transfer_zone(self.target) else {
            return CommandStepResult::failed(self.update_to_stop(ctx));
        };

        let update = self.update_to_stop(ctx);

        if !self.within_zone(ctx, zone) {
            if self.should_issue_move(ctx.frame) {
                let Some(entry) = self.entry_point(ctx, zone, ctx.position) else {
                    return CommandStepResult::failed(update);
                };
                let request = CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(entry.x))
                    .with_ty(Some(entry.y))
                    .with_update_interval(25);
                return CommandStepResult::running(update)
                    .with_operations(vec![CommandOperation::PushFront(request)]);
            }
            return CommandStepResult::running(update);
        }

        if self.should_call_script(ctx.frame) {
            let event = CommandEvent::CallObjectFunction {
                object_id: self.target,
                function: "ControlTransfer".into(),
                caller: ctx.object.id,
                tx: self.tx,
                tx_definition: None,
                ty: self.ty,
                target2: None,
                on_result: Some(CallResultAction::CompleteCommandOnFalse {
                    command: CommandId::Transfer,
                }),
            };
            return CommandStepResult::running(update).with_events(vec![event]);
        }

        CommandStepResult::running(update)
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

        if !target_snapshot.is_status_active() {
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
            let mut update = self.update_to_stop(ctx).unwrap_or_default();
            let idle_action = ActionUpdate::default().with_name("Idle").with_force(true);
            update = update.with_action_update(idle_action);
            return CommandStepResult::running(Some(update));
        }

        let dx = target_snapshot.position.x - ctx.position.x;

        const MIN_HORIZONTAL_RANGE: i32 = 4;
        const MAX_HORIZONTAL_RANGE: i32 = 9;

        let at_target = ctx.object.container.is_none()
            && target_snapshot.container.is_none()
            && target_snapshot.at_point(ctx.position.x, ctx.position.y)
            && dx.abs() >= MIN_HORIZONTAL_RANGE
            && dx.abs() <= MAX_HORIZONTAL_RANGE;

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
            let mut update = self.ensure_stop(ctx, pending_update).unwrap_or_default();
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
            let mut update = pending_update.unwrap_or_default();
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
    #[serde(default, skip_serializing_if = "crate::is_false")]
    reject_pending: bool,
    #[serde(default, skip_serializing_if = "crate::is_false")]
    target_cleared: bool,
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
            reject_pending: false,
            target_cleared: false,
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

        let target = (!self.target_cleared).then_some(self.target);
        if ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target == target
        {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        if self.target_cleared {
            return CommandStepResult::failed(Some(
                ObjectUpdate::new().with_command_direction(CommandDirection::Stop),
            ));
        }

        let target_snapshot = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                return CommandStepResult::failed(Some(update));
            }
        };

        // C4Command::Grab checks only the target pointer — no aliveness
        // gate on the grabbed vehicle (C4Command.cpp:667-695).
        if target_snapshot.destroyed || !target_snapshot.status.is_active() {
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
            let update = pending_update.take().unwrap_or_default();
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

        // "At target object: grab": point-in-shape like C4Command::Grab
        // (Target->At(cObj->x, cObj->y, ocf), C4Command.cpp:689-691).
        let can_grab_here = ctx.object.container.is_none()
            && target_snapshot.at_point(ctx.position.x, ctx.position.y)
            && (target_snapshot.ocf & ocf::GRAB) != 0;

        if can_grab_here {
            // Stop/Push must not be staged ahead of RejectGrabbed. The live
            // event also performs Scale/Hangle's earlier let-go operation.
            self.reject_pending = true;
            return CommandStepResult::running(None).with_events(vec![
                CommandEvent::AttemptGrab {
                    actor_id: ctx.object.id,
                    target_id: self.target,
                },
            ]);
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
struct ActivateState {
    target: Option<ObjectId>,
    container: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    remaining: i32,
    update_interval: u32,
    last_evaluated: Option<u64>,
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
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            exit_requested: false,
            enter_requested: false,
        })
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        let mut update = None;
        if ctx.object.command_direction != CommandDirection::Stop {
            let mut object_update = ObjectUpdate::new();
            object_update.command_direction = Some(CommandDirection::Stop);
            update = Some(object_update);
        }
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

    fn resolve_container(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
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
        if (target.category & (CATEGORY_VEHICLE | CATEGORY_OBJECT)) == 0 {
            return true;
        }
        if (target.category & CATEGORY_SELECT_KNOWLEDGE) == 0 {
            return true;
        }
        target.construction >= FULL_CON
    }

    fn release_target(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        target_id: ObjectId,
        update: Option<ObjectUpdate>,
    ) -> CommandStepResult {
        let mut result = if self.remaining > 1 {
            self.remaining -= 1;
            if self.definition_id.is_some() {
                self.target = None;
            }
            CommandStepResult::running(update)
        } else {
            CommandStepResult::completed(update)
        };

        result.events.push(CommandEvent::SetObjectCommand {
            object_id: target_id,
            controller: ctx.object.controller,
            request: CommandRequest::new(CommandId::Exit),
        });

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
                .with_mode(CommandMode::Sub);
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
                .with_mode(CommandMode::Sub);
            result.operations.push(CommandOperation::PushFront(request));
            result
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

        if self.target.is_none() && self.definition_id.is_none() {
            let Some(container) = self.container else {
                return CommandStepResult::failed(None);
            };
            if !ctx.resolve(container).is_some_and(|target| {
                !target.destroyed && target.status != ObjectStatus::Deleted
            }) {
                return CommandStepResult::failed(None);
            }
            return CommandStepResult::completed(None).with_events(vec![
                CommandEvent::OpenMenu(MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.controller,
                    kind: MenuRequestKind::ActivateTarget { container },
                }),
            ]);
        }

        let update = self.prepare_update(ctx);

        if let Some(target_id) = self.target {
            match ctx.resolve(target_id) {
                Some(snapshot) if snapshot.container.is_none() => {
                    return CommandStepResult::completed(update);
                }
                Some(_) => {}
                None => return CommandStepResult::failed(update),
            }
        }

        let Some(container_id) = self.resolve_container(ctx) else {
            return CommandStepResult::failed(update);
        };

        if ctx.object.container.is_none() {
            self.exit_requested = false;
        }
        if ctx.object.container == Some(container_id) {
            self.enter_requested = false;
        }

        let target_id = if let Some(target_id) = self.target {
            target_id
        } else {
            let Some(definition_id) = self.definition_id.clone() else {
                return CommandStepResult::failed(update);
            };
            let Some(container_snapshot) = ctx.resolve(container_id) else {
                return CommandStepResult::failed(update);
            };
            let mut candidate = None;
            for object_id in &container_snapshot.contents {
                if let Some(snapshot) = ctx.resolve(*object_id) {
                    if snapshot.is_status_active()
                        && snapshot.definition_id == definition_id
                        && snapshot.container == Some(container_id)
                    {
                        candidate = Some(*object_id);
                        break;
                    }
                }
            }
            let Some(candidate_id) = candidate else {
                return CommandStepResult::failed(update);
            };
            self.target = Some(candidate_id);
            candidate_id
        };

        let target_snapshot = match ctx.resolve(target_id) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(update),
        };

        if !target_snapshot.is_status_active() {
            return CommandStepResult::failed(update);
        }

        if target_snapshot.container.is_none() {
            return CommandStepResult::completed(update);
        }

        if target_snapshot.container != Some(container_id) {
            return CommandStepResult::failed(update);
        }

        if !self.check_minimum_con(target_snapshot) {
            return CommandStepResult::failed(update);
        }

        if ctx.object.id == container_id || ctx.object.container == Some(container_id) {
            return self.release_target(ctx, target_id, update);
        }

        if let Some(current_container) = ctx.object.container {
            if current_container != container_id {
                return self.request_exit(update);
            }
        }

        if let Some(container_snapshot) = ctx.resolve(container_id) {
            if container_snapshot.destroyed || !container_snapshot.status.is_active() {
                return CommandStepResult::failed(update);
            }
            if container_snapshot.ocf & ocf::ENTRANCE != 0 {
                return self.request_enter(container_id, update);
            }
        }

        CommandStepResult::failed(update)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PushToState {
    target: ObjectId,
    container: Option<ObjectId>,
    tx: Option<i32>,
    ty: Option<i32>,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_move_order: Option<u64>,
    activate_requested: bool,
    grab_requested: bool,
    enter_requested: bool,
}

impl PushToState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        Ok(Self {
            target,
            container: request.target2,
            tx: request.tx,
            ty: request.ty,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            last_move_order: None,
            activate_requested: false,
            grab_requested: false,
            enter_requested: false,
        })
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        let mut update = None;
        if ctx.object.command_direction != CommandDirection::Stop {
            update = Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop));
        }

        if matches!(
            ctx.object.action_procedure,
            ActionProcedure::Build | ActionProcedure::Chop | ActionProcedure::Dig
        ) {
            let mut object_update = update.unwrap_or_else(ObjectUpdate::new);
            let idle_action = ActionUpdate::default()
                .with_name("Idle")
                .with_force(true)
                .with_phase(0)
                .with_ticks(0);
            object_update = object_update.with_action_update(idle_action);
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

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let update = self.prepare_update(ctx);

        let target_snapshot = match ctx.resolve(self.target) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(update),
        };

        if !target_snapshot.is_active() {
            return CommandStepResult::failed(update);
        }

        if self.container == Some(self.target) {
            return CommandStepResult::failed(update);
        }

        if let Some(destination) = self.container {
            if target_snapshot.container == Some(destination) {
                return CommandStepResult::completed(update);
            }
        } else if let (Some(tx), Some(ty)) = (self.tx, self.ty) {
            let dx = target_snapshot.position.x - tx;
            let dy = target_snapshot.position.y - ty;
            if dx.abs() <= PUSH_TO_RANGE && dy.abs() <= PUSH_TO_RANGE {
                let mut result = CommandStepResult::completed(update.clone());
                let mut operations = Vec::new();
                let ungrab_request = CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::Sub);
                operations.push(CommandOperation::PushFront(ungrab_request));
                let wait_request = CommandRequest::new(CommandId::Wait)
                    .with_update_interval(10)
                    .with_mode(CommandMode::Sub);
                operations.push(CommandOperation::PushFront(wait_request));
                result = result.with_operations(operations);
                return result;
            }
        }

        if let Some(target_container) = target_snapshot.container {
            if Some(target_container) != self.container {
                if !self.activate_requested {
                    self.activate_requested = true;
                    let mut result = CommandStepResult::running(update.clone());
                    let request = CommandRequest::new(CommandId::Activate)
                        .with_target(Some(self.target))
                        .with_update_interval(40)
                        .with_mode(CommandMode::Sub);
                    result.operations.push(CommandOperation::PushFront(request));
                    return result;
                }
                return CommandStepResult::running(update);
            }
        }
        self.activate_requested = false;

        let pushing_target = ctx.object.action_procedure == ActionProcedure::Push
            && ctx.object.action_target == Some(self.target);

        if !pushing_target {
            self.enter_requested = false;
            if !self.grab_requested {
                self.grab_requested = true;
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Grab)
                    .with_target(Some(self.target))
                    .with_update_interval(40)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update);
        }
        self.grab_requested = false;

        if let Some(destination) = self.container {
            if !self.enter_requested {
                self.enter_requested = true;
                let mut result = CommandStepResult::running(update.clone());
                let request = CommandRequest::new(CommandId::Enter)
                    .with_target(Some(destination))
                    .with_update_interval(40)
                    .with_mode(CommandMode::Sub)
                    .with_data(CommandData::Integer(COMMAND_FLAG_ENTER_PUSH_TARGET));
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            return CommandStepResult::running(update);
        }
        self.enter_requested = false;

        if self.tx.is_none() || self.ty.is_none() {
            return CommandStepResult::failed(update);
        }

        if self.should_issue_move(ctx.frame) {
            let mut result = CommandStepResult::running(update.clone());
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_tx(self.tx)
                .with_ty(self.ty)
                .with_update_interval(40)
                .with_mode(CommandMode::Sub)
                .with_data(CommandData::Integer(COMMAND_FLAG_MOVE_TO_PUSH_TARGET));
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        CommandStepResult::running(update)
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
            // ObjectComUnGrab calls ObjectActionStand: ComDir Stop, Walk,
            // and zero xdir/ydir. SetActionByName receives no target, so
            // the previous target is retained (C4ObjectCom.cpp:41-46,
            // 261-278; C4Object.cpp:4142-4143).
            let action_update = ActionUpdate::default().with_name("Walk").with_force(true);
            update = update
                .with_action_update(action_update)
                .with_velocity(Vector2::ZERO);
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
    remaining: Option<u32>,
}

impl WaitState {
    fn from_request(request: &CommandRequest) -> Self {
        // C4CMD_Wait InitEvaluation (C4Command.cpp:1659-1663): a nonzero
        // Data overrides the update interval, else a nonzero Tx does.
        let interval = match request.data {
            CommandData::Integer(data) if data != 0 => data.max(0) as u32,
            _ => match request.tx {
                Some(tx) if tx != 0 => tx.max(0) as u32,
                _ => request.update_interval,
            },
        };
        let remaining = (interval != 0).then_some(interval);
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
    #[serde(default)]
    remaining_count: i32,
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
            remaining_count: request.tx.unwrap_or(0),
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
            Some(snapshot) if snapshot.is_status_active() => snapshot,
            _ => return CommandStepResult::failed(update),
        };

        let (item_id, item_snapshot) = match self.resolve_item(ctx) {
            Some(value) => value,
            None => return CommandStepResult::completed(update),
        };

        if item_snapshot.container == Some(self.container) {
            if self.remaining_count > 1 {
                self.requested_item = None;
                self.remaining_count -= 1;
                return CommandStepResult::running(update);
            }
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

        // A Clonk already inside the target puts directly even when the
        // object centers exceed the outdoor range gate
        // (C4Command.cpp:1439-1447).
        if ctx.object.container != Some(self.container) {
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
        }

        let mut item_update = ObjectUpdate::new();
        item_update.container = Some(Some(self.container));
        item_update.position = Some(container_snapshot.position);
        item_update.velocity = Some(Vector2::ZERO);

        CommandStepResult::running(update).with_events(vec![CommandEvent::ApplyObjectUpdate {
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
            Some(Vector2::new(
                request.tx.unwrap_or(0),
                request.ty.unwrap_or(0),
            ))
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

        // C4Command::Drop's contained/pushing branches call
        // ObjectComPutTake and finish immediately. Once our delegated Put has
        // transferred the item to that same container, do not try to Get it
        // back into the actor (C4Command.cpp:1036-1049).
        if self.delegated_put && item_snapshot.container == self.delegated_container {
            return CommandStepResult::completed(update);
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

        // ObjectComDrop (C4ObjectCom.cpp:640-676): after the item's Exit
        // the dropper arms NoCollectDelay = 2 and refreshes its OCF so the
        // Collection bit is off before the next cross check.
        CommandStepResult::completed(update).with_events(vec![
            CommandEvent::ApplyObjectUpdate {
                object_id: item_id,
                update: item_update,
            },
            CommandEvent::ArmNoCollectDelay {
                object_id: ctx.object.id,
            },
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GetState {
    target: Option<ObjectId>,
    fallback_container: Option<ObjectId>,
    definition_id: Option<DefinitionId>,
    #[serde(default)]
    menu_identification: Option<i32>,
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
        let mut remaining = request.tx.unwrap_or(1);
        if remaining <= 0 {
            remaining = 1;
        }
        Ok(Self {
            target: request.target,
            fallback_container: request.target2,
            definition_id,
            menu_identification,
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
        let mut update = update.unwrap_or_default();
        update.command_direction = Some(CommandDirection::Stop);
        Some(update)
    }

    fn prepare_update(&self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
        let mut update = self.ensure_stop(ctx, None);
        if ctx.object.action_procedure == ActionProcedure::Dig {
            let mut object_update = update.unwrap_or_default();
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
                if item_snapshot.is_status_active()
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
        // Do not decrement `remaining` here. C++ only observes a successful
        // collection on the NEXT Get evaluation (Target->Contained == cObj,
        // C4Command.cpp:1154-1165). A RejectCollect/PutAway retry therefore
        // retains both Target and Tx exactly.
        CommandStepResult::running(update).with_events(vec![CommandEvent::GetObject {
            actor_id: ctx.object.id,
            object_id: target_id,
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
        if !container_snapshot.is_status_active() {
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
        if let (Some(identification), Some(container)) =
            (self.menu_identification, self.target)
        {
            let kind = if identification == 18 {
                MenuRequestKind::Contents { container }
            } else {
                MenuRequestKind::Get { container }
            };
            return CommandStepResult::completed(self.prepare_update(ctx)).with_events(vec![
                CommandEvent::OpenMenu(MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind,
                }),
            ]);
        }

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
            Some(snapshot) if snapshot.is_status_active() => snapshot,
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
                    result.operations.push(CommandOperation::PushFront(
                        CommandRequest::new(CommandId::Jump)
                            .with_tx(Some(tx))
                            .with_ty(Some(ty)),
                    ));
                    // (CollectionLimit drop, C4Command.cpp:1282-1284, is
                    // unmodeled — GoldRush clonks have no CollectionLimit.)
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

        let mut pending_update = None;

        if ctx.object.action_procedure == ActionProcedure::Dig {
            // C4Command::Throw first ObjectComStop's a digging Clonk back to
            // Walk (C4Command.cpp:912-913; C4ObjectCom.cpp:239-244).
            let walk_action = ActionUpdate::default().with_name("Walk").with_force(false);
            let update = self.update_to_stop(ctx).unwrap_or_default();
            pending_update = Some(
                update
                    .with_velocity(Vector2::ZERO)
                    .with_action_update(walk_action),
            );
        }

        if ctx.object.action_procedure == ActionProcedure::Push && self.throw_position().is_some() {
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

        // An untargeted Throw while contained is a put/take operation, not
        // the outside Throw action (C4Command.cpp:966-970). ObjectComPutTake
        // chooses the requested item or the actor's first content and enters
        // it into the containing object (C4ObjectCom.cpp:700-712).
        if let Some(container_id) = ctx.object.container {
            let item_id = self
                .target
                .filter(|target| ctx.object.contents.contains(target))
                .or_else(|| ctx.object.contents.first().copied());
            let events = item_id
                .map(|object_id| CommandEvent::ApplyObjectUpdate {
                    object_id,
                    update: ObjectUpdate::new().with_container(container_id),
                })
                .into_iter()
                .collect();
            return CommandStepResult::completed(pending_update).with_events(events);
        }

        // Untargeted Throw while pushing is the grabbed-object twin of the
        // contained branch above: ObjectComPutTake uses Action.Target and the
        // command finishes without ungrabbing (C4Command.cpp:973-979).
        if ctx.object.action_procedure == ActionProcedure::Push {
            let item_id = self
                .target
                .filter(|target| ctx.object.contents.contains(target))
                .or_else(|| ctx.object.contents.first().copied());
            let events = ctx
                .object
                .action_target
                .zip(item_id)
                .map(|(container_id, object_id)| CommandEvent::ApplyObjectUpdate {
                    object_id,
                    update: ObjectUpdate::new().with_container(container_id),
                })
                .into_iter()
                .collect();
            return CommandStepResult::completed(pending_update).with_events(events);
        }

        let mut update = pending_update.unwrap_or_default();

        if let Some(position) = self.throw_position() {
            update.command_direction = Some(CommandDirection::Stop);
            if position.x > ctx.position.x {
                update.direction = Some(Direction::Right);
            } else {
                update.direction = Some(Direction::Left);
            }
        }

        // ObjectActionThrow changes the Clonk action, then immediately exits
        // the selected (or first) content. Keep that ordered operation in the
        // engine: SetAction may reject the transition, and only a successful
        // transition consumes Random(360) (C4ObjectCom.cpp:120-137).
        let item_id = self
            .target
            .filter(|target| ctx.object.contents.contains(target))
            .or_else(|| ctx.object.contents.first().copied());
        let targeted = self.throw_position().is_some();
        let Some(object_id) = item_id else {
            return if targeted {
                CommandStepResult::running(Some(update))
            } else {
                CommandStepResult::completed(Some(update))
            };
        };
        if !matches!(
            ctx.object.action_procedure,
            ActionProcedure::Walk | ActionProcedure::Dig
        ) {
            return if targeted {
                CommandStepResult::running(Some(update))
            } else {
                CommandStepResult::completed(Some(update))
            };
        }
        let event = CommandEvent::ThrowObject {
            actor_id: ctx.object.id,
            object_id,
            complete_command_on_success: targeted,
        };
        let update = (!update.is_empty()).then_some(update);
        if targeted {
            CommandStepResult::running(update).with_events(vec![event])
        } else {
            CommandStepResult::completed(update).with_events(vec![event])
        }
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
struct CallState {
    target: ObjectId,
    function: String,
    tx: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tx_definition: Option<DefinitionId>,
    ty: Option<i32>,
    target2: Option<ObjectId>,
    executed: bool,
}

impl CallState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let target = request.target.ok_or(CommandError::Unsupported)?;
        let function = match &request.data {
            CommandData::Text(text) if !text.is_empty() => text.clone(),
            _ => return Err(CommandError::Unsupported),
        };
        Ok(Self {
            target,
            function,
            tx: request.tx,
            tx_definition: request.tx_definition.clone(),
            ty: request.ty,
            target2: request.target2,
            executed: false,
        })
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.executed {
            return CommandStepResult::completed(None);
        }

        let Some(target_snapshot) = ctx.resolve(self.target) else {
            self.executed = true;
            return CommandStepResult::failed(None);
        };

        if !target_snapshot.is_status_active() {
            self.executed = true;
            return CommandStepResult::failed(None);
        }

        self.executed = true;

        let mut update = None;
        if ctx.object.command_direction != CommandDirection::Stop {
            update = Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop));
        }

        let event = CommandEvent::CallObjectFunction {
            object_id: self.target,
            function: self.function.clone(),
            caller: ctx.object.id,
            tx: self.tx,
            tx_definition: self.tx_definition.clone(),
            ty: self.ty,
            target2: self.target2,
            on_result: None,
        };

        CommandStepResult::completed(update).with_events(vec![event])
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

        // C4Command::Context only requires a live Status target; buildings,
        // items, and synthetic crew can be non-Alive (C4Command.cpp:1076-
        // 1089; C4Object::ActivateMenu checks pTarget, not Alive).
        if !target_snapshot.is_status_active() {
            return CommandStepResult::failed(None);
        }

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

        let update = Self::update_to_stop(ctx);
        let Some(container_id) = ctx.object.container else {
            return CommandStepResult::failed(update);
        };
        let Some(container) = ctx.resolve(container_id) else {
            return CommandStepResult::failed(update);
        };
        if !container.is_active() {
            return CommandStepResult::failed(update);
        }

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
    last_evaluated: Option<u64>,
    candidate: Option<ObjectId>,
    buy_requested: bool,
    last_buy_request: Option<u64>,
    get_requested: bool,
    #[serde(default)]
    script_pending: bool,
    #[serde(default)]
    script_invoked: bool,
    #[serde(default)]
    script_result: Option<AcquireScriptResult>,
}

impl AcquireState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        let definition_id =
            command_data_to_definition_id(&request.data).ok_or(CommandError::Unsupported)?;
        // C4CMD_Acquire InitEvaluation Tx/Ty defaults (C4Command.cpp:
        // 1666-1670): only a ZERO range becomes 500/250 — the sign of a
        // nonzero range survives into the Inside(cx-px, -Tx, +Tx) match
        // (:2115-2116), where a negative range matches nothing.
        let raw_range_x = request.tx.unwrap_or(0);
        let raw_range_y = request.ty.unwrap_or(0);
        let range_x = if raw_range_x == 0 { 500 } else { raw_range_x };
        let range_y = if raw_range_y == 0 { 250 } else { raw_range_y };
        Ok(Self {
            target: request.target,
            definition_id,
            ignore_container: request.target2,
            range_x,
            range_y,
            update_interval: request.update_interval.max(1),
            last_evaluated: None,
            candidate: None,
            buy_requested: false,
            last_buy_request: None,
            get_requested: false,
            script_pending: false,
            script_invoked: false,
            script_result: None,
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
            if best.is_none_or(|(_, best_dist)| distance < best_dist) {
                best = Some((*id, distance));
            }
        }
        best.map(|(id, _)| id)
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        let has_item = ctx
            .object
            .contents
            .iter()
            .filter_map(|id| ctx.resolve(*id))
            .any(|snapshot| snapshot.definition_id == self.definition_id);

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
                        return CommandStepResult::running(self.update_to_stop(ctx));
                    }
                    AcquireScriptResult::Complete => {
                        self.script_invoked = false;
                        return CommandStepResult::completed(self.update_to_stop(ctx));
                    }
                    AcquireScriptResult::Failed => {
                        self.script_invoked = false;
                        return CommandStepResult::failed(self.update_to_stop(ctx));
                    }
                    AcquireScriptResult::Continue => {
                        // proceed with default logic below
                    }
                }
            } else {
                return CommandStepResult::running(self.update_to_stop(ctx));
            }
        }

        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
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
            };
            return CommandStepResult::running(self.update_to_stop(ctx)).with_events(vec![event]);
        }

        self.last_evaluated = Some(ctx.frame);

        if let Some(candidate_id) = self.candidate {
            let valid = ctx
                .resolve(candidate_id)
                .filter(|snapshot| self.candidate_is_valid(snapshot, ctx))
                .is_some();
            if !valid {
                self.candidate = None;
                self.get_requested = false;
            }
        }

        if self.candidate.is_none() {
            self.candidate = self.find_candidate(ctx);
        }

        if self.candidate.is_none() {
            self.get_requested = false;
            self.maybe_reset_buy(ctx.frame);
            self.script_invoked = false;
            let mut result = CommandStepResult::running(self.update_to_stop(ctx));
            if let Some(operation) = self.request_buy(ctx.frame) {
                result.operations.push(operation);
            }
            return result;
        }

        let candidate_id = self.candidate.expect("candidate present");
        if ctx.resolve(candidate_id).is_none() {
            self.candidate = None;
            self.get_requested = false;
            self.script_invoked = false;
            return CommandStepResult::running(self.update_to_stop(ctx));
        }

        self.buy_requested = false;
        self.last_buy_request = None;

        if !self.get_requested {
            self.get_requested = true;
            self.script_invoked = false;
            let mut result = CommandStepResult::running(self.update_to_stop(ctx));
            let request = CommandRequest::new(CommandId::Get)
                .with_target(Some(candidate_id))
                .with_update_interval(40)
                .with_mode(CommandMode::SilentSub);
            result.operations.push(CommandOperation::PushFront(request));
            return result;
        }

        self.script_invoked = false;
        CommandStepResult::running(self.update_to_stop(ctx))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SellState {
    definition_id: DefinitionId,
    target: Option<ObjectId>,
    preferred: Option<ObjectId>,
    remaining: i32,
    update_interval: u32,
    last_evaluated: Option<u64>,
    last_enter_request: Option<u64>,
}

impl SellState {
    fn from_request(request: &CommandRequest) -> Result<Self, CommandError> {
        // Data==0 is the internal "open C4MN_Sell" command
        // (C4Command.cpp:2052-2057); a nonzero ID performs a sale.
        let definition_id = command_data_to_definition_id(&request.data).unwrap_or_default();
        let remaining = request.tx.unwrap_or(1).max(1);
        Ok(Self {
            definition_id,
            target: request.target,
            preferred: request.target2,
            remaining,
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

    fn should_issue_enter(&self, frame: u64) -> bool {
        const ENTER_COOLDOWN: u64 = 12;
        match self.last_enter_request {
            Some(last) => frame.saturating_sub(last) >= ENTER_COOLDOWN,
            None => true,
        }
    }

    fn is_base(snapshot: &CommandObjectSnapshot, owner: i32) -> bool {
        snapshot.is_status_active()
            && snapshot.base == owner
            && (snapshot.category & CATEGORY_STRUCTURE) != 0
            && (snapshot.ocf & ocf::ENTRANCE) != 0
            && !snapshot.collectible
    }

    fn resolve_base(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        let owner = ctx.object.owner;
        if owner == OWNER_NONE {
            return None;
        }

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
            .map(|snapshot| {
                let id = snapshot.id;
                self.target = Some(id);
                id
            })
    }

    fn resolve_candidate(
        &mut self,
        ctx: &CommandRuntimeContext<'_>,
        base: &CommandObjectSnapshot,
    ) -> Option<ObjectId> {
        if let Some(candidate_id) = self.preferred {
            if let Some(snapshot) = ctx.resolve(candidate_id) {
                if snapshot.is_status_active()
                    && snapshot.container == Some(base.id)
                    && snapshot.definition_id == self.definition_id
                {
                    return Some(candidate_id);
                }
            }
            self.preferred = None;
        }

        for item_id in &base.contents {
            if let Some(snapshot) = ctx.resolve(*item_id) {
                if snapshot.is_status_active()
                    && snapshot.container == Some(base.id)
                    && snapshot.definition_id == self.definition_id
                {
                    self.preferred = Some(*item_id);
                    return Some(*item_id);
                }
            }
        }

        None
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.definition_id.is_empty() {
            let update = self.update_to_stop(ctx);
            if !ctx.base_sell_enabled {
                return CommandStepResult::failed(update);
            }
            let Some(base) = self.resolve_base(ctx) else {
                return CommandStepResult::failed(update);
            };
            return CommandStepResult::completed(update).with_events(vec![
                CommandEvent::OpenMenu(MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind: MenuRequestKind::Sell { base },
                }),
            ]);
        }

        if self.remaining <= 0 {
            return CommandStepResult::completed(self.update_to_stop(ctx));
        }

        let interval = self.update_interval as u64;
        if let Some(last) = self.last_evaluated {
            if ctx.frame.saturating_sub(last) < interval {
                return CommandStepResult::running(None);
            }
        }
        self.last_evaluated = Some(ctx.frame);

        let update_to_stop = self.update_to_stop(ctx);

        if !ctx.base_sell_enabled {
            return CommandStepResult::failed(update_to_stop);
        }

        let base_id = match self.resolve_base(ctx) {
            Some(id) => id,
            None => return CommandStepResult::failed(update_to_stop),
        };

        let base_snapshot = match ctx.resolve(base_id) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(update_to_stop),
        };

        let base_owner = base_snapshot.base;
        if base_owner == OWNER_NONE {
            return CommandStepResult::failed(update_to_stop);
        }

        if ctx.object.container != Some(base_id) {
            let mut result = CommandStepResult::running(update_to_stop);
            if self.should_issue_enter(ctx.frame) {
                self.last_enter_request = Some(ctx.frame);
                let request = CommandRequest::new(CommandId::Enter)
                    .with_target(Some(base_id))
                    .with_update_interval(25)
                    .with_mode(CommandMode::Sub);
                result.operations.push(CommandOperation::PushFront(request));
            }
            return result;
        }
        self.last_enter_request = None;

        let candidate_id = match self.resolve_candidate(ctx, base_snapshot) {
            Some(id) => id,
            None => return CommandStepResult::failed(update_to_stop),
        };

        let candidate_snapshot = match ctx.resolve(candidate_id) {
            Some(snapshot) => snapshot,
            None => return CommandStepResult::failed(update_to_stop),
        };

        if candidate_snapshot.definition_id != self.definition_id
            || candidate_snapshot.container != Some(base_id)
        {
            self.preferred = None;
            return CommandStepResult::failed(update_to_stop);
        }

        if !candidate_snapshot.contents.is_empty() {
            return CommandStepResult::failed(update_to_stop);
        }

        let value = ctx
            .definition(&self.definition_id)
            .map(|definition| definition.value.max(0))
            .unwrap_or(0);

        let mut events = Vec::new();
        if value != 0 {
            events.push(CommandEvent::AdjustPlayerWealth {
                player_id: base_owner,
                delta: value,
            });
        }
        events.push(CommandEvent::AdjustPlayerHomeBaseMaterial {
            player_id: base_owner,
            definition_id: self.definition_id.clone(),
            delta: 1,
        });

        let mut item_update = ObjectUpdate::new();
        item_update.container = Some(None);
        item_update.position = Some(base_snapshot.position);
        item_update.velocity = Some(Vector2::ZERO);
        item_update.status = Some(ObjectStatus::Deleted);
        item_update.alive = Some(false);
        item_update.command_direction = Some(CommandDirection::Stop);
        events.push(CommandEvent::ApplyObjectUpdate {
            object_id: candidate_id,
            update: item_update,
        });

        self.preferred = None;
        self.remaining = self.remaining.saturating_sub(1);

        if self.remaining == 0 {
            CommandStepResult::completed(self.update_to_stop(ctx)).with_events(events)
        } else {
            CommandStepResult::running(self.update_to_stop(ctx)).with_events(events)
        }
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
        // Data==0 is the internal "open C4MN_Buy" command
        // (C4Command.cpp:1999-2004); a nonzero ID performs a purchase.
        let definition_id = command_data_to_definition_id(&request.data).unwrap_or_default();
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
        if !target_snapshot.is_status_active() || target_snapshot.collectible {
            return None;
        }

        let update_to_stop = self.update_to_stop(ctx);

        if let Some(container_id) = ctx.object.container {
            if container_id != target_id {
                let mut update = update_to_stop.unwrap_or_default();
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
                let mut update = update_to_stop.unwrap_or_default();
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
                if item_snapshot.is_status_active()
                    && item_snapshot.definition_id == self.definition_id
                    && item_snapshot.collectible
                    && item_snapshot.construction >= FULL_CON
                {
                    candidate = Some(*item_id);
                    break;
                }
            }
        }

        let item_id = candidate?;

        let buyer_owner = ctx.object.owner;
        let base_owner = target_snapshot.base;
        let player = match ctx.player(base_owner) {
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
                player_id: base_owner,
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
                if snapshot.is_status_active() {
                    return Some(target);
                }
            }
        }

        let buyer_owner = ctx.object.owner;
        ctx.objects
            .values()
            .filter(|snapshot| {
                snapshot.is_status_active()
                    && snapshot.base == buyer_owner
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
        if self.definition_id.is_empty() {
            let update = self.update_to_stop(ctx);
            if !ctx.base_buy_enabled {
                return CommandStepResult::failed(update);
            }
            let Some(base) = self.resolve_base(ctx) else {
                return CommandStepResult::failed(update);
            };
            return CommandStepResult::completed(update).with_events(vec![
                CommandEvent::OpenMenu(MenuRequest {
                    crew_id: ctx.object.id,
                    owner: ctx.object.owner,
                    kind: MenuRequestKind::Buy { base },
                }),
            ]);
        }
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

        let base_owner = base_snapshot.base;
        if base_owner == OWNER_NONE {
            return CommandStepResult::failed(update);
        }

        let base_player = match ctx.player(base_owner) {
            Some(player) if player.is_active() => player,
            _ => return CommandStepResult::failed(update),
        };

        let available = base_player.material_count(&self.definition_id);
        if available <= 0 {
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
            construction: None,
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
            .find(|snapshot| snapshot.definition_id == LINEKIT_DEFINITION)
            .map(|snapshot| snapshot.id)
    }

    fn resolve_source(&mut self, ctx: &CommandRuntimeContext<'_>) -> Option<ObjectId> {
        if let Some(source) = self.source {
            return ctx.resolve(source).and_then(|snapshot| {
                (snapshot.is_status_active()
                    && snapshot.line_connect & crate::LINE_CONNECT_POWER_OUTPUT != 0)
                    .then_some(source)
            });
        }
        let target = ctx.resolve(self.target)?;
        let source = ctx
            .objects
            .values()
            .filter(|snapshot| {
                snapshot.id != self.target
                    && snapshot.is_status_active()
                    && snapshot.ocf & ocf::POWER_SUPPLY != 0
                    && snapshot.line_connect & crate::LINE_CONNECT_POWER_OUTPUT != 0
            })
            .min_by_key(|snapshot| {
                let dx = i64::from(snapshot.position.x - target.position.x);
                let dy = i64::from(snapshot.position.y - target.position.y);
                (dx * dx + dy * dy, snapshot.id)
            })?
            .id;
        self.source = Some(source);
        Some(source)
    }

    fn spawned_line(
        &self,
        ctx: &CommandRuntimeContext<'_>,
        source: &CommandObjectSnapshot,
    ) -> Option<ObjectId> {
        ctx.objects
            .values()
            .filter(|snapshot| {
                snapshot.definition_id == POWERLINE_DEFINITION
                    && snapshot.is_status_active()
                    && snapshot.owner == ctx.object.owner
                    && snapshot.action_target == Some(source.id)
            })
            // The just-created line follows older lines from the same
            // supply in C4ObjectList order; ObjectId is monotonic here.
            .max_by_key(|snapshot| snapshot.id)
            .map(|snapshot| snapshot.id)
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

        if !target_snapshot.is_status_active() {
            return CommandStepResult::failed(update_to_stop());
        }

        if !ctx.structures_need_energy {
            return CommandStepResult::completed(None);
        }
        if (target_snapshot.line_connect & LINE_CONNECT_POWER_INPUT) == 0 {
            return CommandStepResult::failed(update_to_stop());
        }

        let Some(source_id) = self.resolve_source(ctx) else {
            return CommandStepResult::failed(update_to_stop());
        };
        let Some(source_snapshot) = ctx.resolve(source_id) else {
            return CommandStepResult::failed(update_to_stop());
        };
        let dx = i64::from(ctx.position.x - source_snapshot.position.x);
        let dy = i64::from(ctx.position.y - source_snapshot.position.y);
        if dx * dx + dy * dy > 650_i64.pow(2) {
            return CommandStepResult::failed(update_to_stop());
        }

        let linekit_id = self
            .linekit
            .filter(|id| ctx.object.contents.contains(id))
            .or_else(|| self.builder_linekit(ctx));
        let Some(linekit_id) = linekit_id else {
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
            return CommandStepResult::running(update_to_stop()).with_operations(operations);
        };
        self.linekit = Some(linekit_id);
        self.acquire_requested = false;

        if self.line.is_none() {
            if self.line_spawn_requested {
                let Some(line_id) = self.spawned_line(ctx, source_snapshot) else {
                    return CommandStepResult::running(update_to_stop());
                };
                self.line = Some(line_id);
            } else {
                if !source_snapshot.at_point(ctx.position.x, ctx.position.y) {
                    let request = CommandRequest::new(CommandId::MoveTo)
                        .with_target(Some(source_id))
                        .with_update_interval(50)
                        .with_mode(CommandMode::Sub);
                    return CommandStepResult::running(update_to_stop())
                        .with_operations(vec![CommandOperation::PushFront(request)]);
                }

                self.line_spawn_requested = true;
                return CommandStepResult::running(update_to_stop()).with_events(vec![
                    CommandEvent::CreateLine {
                        definition_id: POWERLINE_DEFINITION.into(),
                        owner: ctx.object.owner,
                        from: source_id,
                        to: linekit_id,
                    },
                ]);
            }
        }

        if !target_snapshot.at_point(ctx.position.x, ctx.position.y) {
            let request = CommandRequest::new(CommandId::MoveTo)
                .with_target(Some(self.target))
                .with_update_interval(50)
                .with_mode(CommandMode::Sub);
            return CommandStepResult::running(update_to_stop())
                .with_operations(vec![CommandOperation::PushFront(request)]);
        }

        let line_id = self.line.expect("line is present");
        let line_update = ObjectUpdate::new().with_action_update(
            ActionUpdate::default()
                .with_name("Connect")
                .with_force(true)
                .with_target(Some(source_id))
                .with_target2(Some(self.target)),
        );
        let linekit_update = ObjectUpdate::new()
            .clear_container()
            .with_status(ObjectStatus::Deleted)
            .with_alive(false);
        CommandStepResult::completed(update_to_stop()).with_events(vec![
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
            CommandState::Unsupported => None,
        }
    }

    /// Live C4Command-field overrides for the FnGetCommand view
    /// (C4Script.cpp:926-945 reads the LIVE fields): only the states
    /// whose C++ counterpart rewrites Target/Tx/Ty after Set do so —
    /// MoveTo's InitEvaluation absorption/adjust (C4Command.cpp:
    /// 1634-1643), Acquire's 500/250 range defaults (:1666-1670) and
    /// Construct's found-site write (:1757-1766), plus Put's resolved
    /// Target2 and remaining Tx count (:1384-1418). Put's Ty reminder flag
    /// is unmodeled.
    fn apply_live_overrides(&self, view: &mut CommandView) {
        match self {
            CommandState::MoveTo(state) => {
                view.target = state.target;
                view.tx = state.tx;
                view.ty = state.ty;
            }
            CommandState::Acquire(state) => {
                view.target = state.target;
                view.tx = Some(state.range_x);
                view.ty = Some(state.range_y);
                view.target2 = state.ignore_container;
            }
            CommandState::Construct(state) => {
                if let Some(site) = state.site {
                    view.tx = Some(site.x);
                    view.ty = Some(site.y);
                }
            }
            CommandState::Put(state) => {
                view.tx = (state.remaining_count != 0).then_some(state.remaining_count);
                view.target2 = state.requested_item;
            }
            _ => {}
        }
    }

    /// Clear nullable state copies of C4Command's object fields as part of
    /// the post-load pointer pass. Required state IDs remain harmless when
    /// absent because command execution resolves them through the same live
    /// object table before use; the creating request above is the canonical
    /// FnGetCommand field view.
    fn denumerate_object_references(&mut self, object_numbers: &HashSet<u64>) {
        match self {
            CommandState::MoveTo(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
            }
            CommandState::Construct(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.construction_id, object_numbers);
            }
            CommandState::Activate(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.container, object_numbers);
            }
            CommandState::PushTo(state) => {
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
                denumerate_object_reference(&mut state.target2, object_numbers);
            }
            CommandState::Acquire(state) => {
                denumerate_object_reference(&mut state.target, object_numbers);
                denumerate_object_reference(&mut state.ignore_container, object_numbers);
                denumerate_object_reference(&mut state.candidate, object_numbers);
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
        let clear = |reference: &mut Option<ObjectId>| {
            clear_matching_object_reference(reference, removed)
        };
        match self {
            CommandState::MoveTo(state) => clear(&mut state.target),
            CommandState::Grab(state) if state.target == removed => {
                let changed = !state.target_cleared;
                state.target_cleared = true;
                changed
            }
            CommandState::Construct(state) => {
                clear(&mut state.target) | clear(&mut state.construction_id)
            }
            CommandState::Activate(state) => {
                clear(&mut state.target) | clear(&mut state.container)
            }
            CommandState::PushTo(state) => clear(&mut state.container),
            CommandState::Put(state) => clear(&mut state.requested_item),
            CommandState::Drop(state) => {
                clear(&mut state.requested_item) | clear(&mut state.delegated_container)
            }
            CommandState::Get(state) => {
                clear(&mut state.target) | clear(&mut state.fallback_container)
            }
            CommandState::Throw(state) => clear(&mut state.target),
            CommandState::Call(state) => clear(&mut state.target2),
            CommandState::Acquire(state) => {
                clear(&mut state.target)
                    | clear(&mut state.ignore_container)
                    | clear(&mut state.candidate)
            }
            CommandState::Sell(state) => {
                clear(&mut state.target) | clear(&mut state.preferred)
            }
            CommandState::Buy(state) => clear(&mut state.target),
            CommandState::Home(state) => clear(&mut state.target),
            CommandState::Energy(state) => {
                clear(&mut state.source) | clear(&mut state.linekit) | clear(&mut state.line)
            }
            _ => false,
        }
    }
}

fn denumerate_object_reference(
    reference: &mut Option<ObjectId>,
    object_numbers: &HashSet<u64>,
) {
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

fn stop_update(ctx: &CommandRuntimeContext<'_>) -> Option<ObjectUpdate> {
    if ctx.object.command_direction != CommandDirection::Stop {
        Some(ObjectUpdate::new().with_command_direction(CommandDirection::Stop))
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ActiveCommand {
    state: CommandState,
    mode: CommandMode,
    retries: i32,
    failures: i32,
    /// The creating request, the FnGetCommand element-view base
    /// (C4Script.cpp:926-945); persisted through CommandSnapshot so
    /// restored stacks keep their elements.
    request: Option<CommandRequest>,
    finished: Option<CommandStatus>,
}

impl ActiveCommand {
    fn from_request(request: CommandRequest) -> Result<Self, CommandError> {
        let state = match request.id {
            CommandId::Follow => CommandState::Follow(FollowState::from_request(&request)?),
            CommandId::MoveTo => CommandState::MoveTo(MoveToState::from_request(&request)),
            CommandId::Enter => CommandState::Enter(EnterState::from_request(&request)?),
            CommandId::Exit => CommandState::Exit(ExitState::from_request(&request)?),
            CommandId::Build => CommandState::Build(BuildState::from_request(&request)?),
            CommandId::Construct => {
                CommandState::Construct(ConstructState::from_request(&request))
            }
            CommandId::Transfer => CommandState::Transfer(TransferState::from_request(&request)?),
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
            CommandId::Activate => CommandState::Activate(ActivateState::from_request(&request)?),
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
            _ => CommandState::Unsupported,
        };

        if matches!(state, CommandState::Unsupported) {
            return Err(CommandError::Unsupported);
        }

        Ok(Self {
            state,
            mode: request.mode,
            retries: request.retries.max(0),
            failures: 0,
            request: Some(request),
            finished: None,
        })
    }

    fn from_snapshot(snapshot: CommandSnapshot) -> Self {
        Self {
            state: snapshot.state,
            mode: snapshot.mode,
            retries: snapshot.retries,
            failures: snapshot.failures,
            request: snapshot.request,
            finished: snapshot.finished,
        }
    }

    fn id(&self) -> Option<CommandId> {
        self.state.id()
    }

    fn step(&mut self, ctx: &CommandRuntimeContext<'_>) -> CommandStepResult {
        if self.failures > 0 {
            if self.retries > 0 {
                self.failures = 0;
                self.retries -= 1;
                let request = CommandRequest::new(CommandId::Retry)
                    .with_update_interval(10)
                    .with_mode(CommandMode::SilentSub);
                let mut result = CommandStepResult::running(stop_update(ctx));
                result.operations.push(CommandOperation::PushFront(request));
                return result;
            }
            let update = stop_update(ctx);
            self.failures = 0;
            return CommandStepResult::failed(update);
        }

        match &mut self.state {
            CommandState::Follow(state) => state.step(ctx),
            CommandState::MoveTo(state) => state.step(ctx),
            CommandState::Enter(state) => state.step(ctx),
            CommandState::Exit(state) => state.step(ctx),
            CommandState::Build(state) => state.step(ctx),
            CommandState::Construct(state) => state.step(ctx),
            CommandState::Transfer(state) => state.step(ctx),
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
            CommandState::Unsupported => {
                let update = ObjectUpdate::new().with_command_direction(CommandDirection::Stop);
                CommandStepResult::failed(Some(update))
            }
        }
    }
}
