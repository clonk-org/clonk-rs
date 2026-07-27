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

mod geometry;
mod machine;
mod model;
mod snapshot;

pub(crate) use geometry::*;
pub use machine::*;
pub use model::*;
pub use snapshot::*;

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
