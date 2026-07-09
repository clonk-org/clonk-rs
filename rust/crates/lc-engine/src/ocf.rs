//! Object Characteristic Flags (OCF) compatibility helpers.
//!
//! The values mirror the legacy C++ constants from `C4Constants.h`.  They
//! intentionally remain `u32` so we can faithfully represent bit masks that
//! rely on the signed interpretation (e.g. `OCF_All == !0u32` in C++).

use crate::{ObjectStatus, FULL_CON as OBJECT_FULL_CON};

pub const NONE: u32 = 0;
pub const ALL: u32 = u32::MAX;
pub const NORMAL: u32 = 1 << 0;
pub const CONSTRUCT: u32 = 1 << 1;
pub const GRAB: u32 = 1 << 2;
pub const CARRYABLE: u32 = 1 << 3;
pub const ON_FIRE: u32 = 1 << 4;
pub const HIT_SPEED1: u32 = 1 << 5;
pub const FULL_CON: u32 = 1 << 6;
pub const INFLAMMABLE: u32 = 1 << 7;
pub const CHOP: u32 = 1 << 8;
pub const ROTATE: u32 = 1 << 9;
pub const EXCLUSIVE: u32 = 1 << 10;
pub const ENTRANCE: u32 = 1 << 11;
pub const HIT_SPEED2: u32 = 1 << 12;
pub const HIT_SPEED3: u32 = 1 << 13;
pub const COLLECTION: u32 = 1 << 14;
pub const LIVING: u32 = 1 << 15;
pub const HIT_SPEED4: u32 = 1 << 16;
pub const FIGHT_READY: u32 = 1 << 17;
pub const LINE_CONSTRUCT: u32 = 1 << 18;
pub const PREY: u32 = 1 << 19;
pub const ATTRACT_LIGHTNING: u32 = 1 << 20;
pub const NOT_CONTAINED: u32 = 1 << 21;
pub const CREW_MEMBER: u32 = 1 << 22;
pub const EDIBLE: u32 = 1 << 23;
pub const IN_LIQUID: u32 = 1 << 24;
pub const IN_SOLID: u32 = 1 << 25;
pub const IN_FREE: u32 = 1 << 26;
pub const AVAILABLE: u32 = 1 << 27;
pub const POWER_CONSUMER: u32 = 1 << 28;
pub const POWER_SUPPLY: u32 = 1 << 29;
pub const CONTAINER: u32 = 1 << 30;
pub const ALIVE: u32 = 1 << 31;

/// Compute a PREVIEW-grade OCF mask from the fixture baseline and the
/// def-independent object state — used where the engine's cached mask is
/// not available yet (creation previews, bare fixture scopes). The full
/// C4Object::SetOCF port lives in `Definition::compute_ocf` +
/// `Engine::compute_object_ocf`.
pub fn compute(
    base: u32,
    crew_member: bool,
    alive: bool,
    _status: ObjectStatus,
    is_contained: bool,
    construction: i32,
    category: i32,
) -> u32 {
    let mut ocf = base | NORMAL;

    if !is_contained {
        ocf |= NOT_CONTAINED | AVAILABLE;
    }

    if construction >= OBJECT_FULL_CON {
        ocf |= FULL_CON;
    }

    // OCF_Living/OCF_Alive gate on C4D_Living (SetOCF, C4Object.cpp:600-605)
    if category & crate::CATEGORY_LIVING != 0 {
        ocf |= LIVING;
        if alive {
            ocf |= ALIVE;
        }
    }

    // OCF_CrewMember: Def->CrewMember && the RAW Alive flag (SetOCF,
    // C4Object.cpp:619-622)
    if crew_member && alive {
        ocf |= CREW_MEMBER;
    }

    // OCF_FightReady from the OCF_Alive BIT (SetOCF, C4Object.cpp:606-610);
    // the NoFight/ActMap gates need the def and stay preview-approximated.
    if ocf & ALIVE != 0 {
        ocf |= FIGHT_READY;
    }

    ocf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectStatus;

    #[test]
    fn compute_adds_dynamic_bits() {
        let ocf = compute(
            NORMAL | CONTAINER,
            true,
            true,
            ObjectStatus::Normal,
            false,
            OBJECT_FULL_CON,
            crate::CATEGORY_LIVING,
        );
        assert!(ocf & NOT_CONTAINED != 0);
        assert!(ocf & AVAILABLE != 0);
        assert!(ocf & FULL_CON != 0);
        assert!(ocf & ALIVE != 0);
        assert!(ocf & CREW_MEMBER != 0);
        assert!(ocf & FIGHT_READY != 0);
        assert!(ocf & CONTAINER != 0);
    }

    #[test]
    fn compute_gates_living_and_alive_on_category() {
        // SetOCF C4Object.cpp:600-605: OCF_Living/OCF_Alive need
        // C4D_Living; a crew def flag alone grants neither.
        let ocf = compute(
            NORMAL,
            true,
            true,
            ObjectStatus::Normal,
            false,
            OBJECT_FULL_CON,
            crate::CATEGORY_OBJECT,
        );
        assert_eq!(ocf & LIVING, 0);
        assert_eq!(ocf & ALIVE, 0);
        assert_eq!(ocf & FIGHT_READY, 0);
        // OCF_CrewMember only needs Def->CrewMember && Alive
        // (C4Object.cpp:619-622).
        assert_ne!(ocf & CREW_MEMBER, 0);
    }

    #[test]
    fn compute_handles_contained_objects() {
        let ocf = compute(
            NORMAL,
            false,
            false,
            ObjectStatus::Inactive,
            true,
            0,
            crate::CATEGORY_STATIC_BACK,
        );
        assert_eq!(ocf & NOT_CONTAINED, 0);
        assert_eq!(ocf & AVAILABLE, 0);
        assert_eq!(ocf & FULL_CON, 0);
        assert_eq!(ocf & ALIVE, 0);
    }
}
