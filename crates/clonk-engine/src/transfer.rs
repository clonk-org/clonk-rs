use crate::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferZone {
    pub owner: ObjectId,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub used: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TransferZoneRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferZoneState {
    pub owner: ObjectId,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone)]
pub enum TransferZoneCommand {
    Set {
        owner: ObjectId,
        rect: TransferZoneRect,
    },
    Clear {
        owner: ObjectId,
    },
}

impl TransferZoneCommand {
    pub fn set(owner: ObjectId, rect: TransferZoneRect) -> Self {
        Self::Set { owner, rect }
    }

    pub fn clear(owner: ObjectId) -> Self {
        Self::Clear { owner }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransferZoneTable {
    /// C++ traversal order: `Add` prepends so the newest zone comes first
    /// (C4TransferZone.cpp:96-108) and `Find` returns the first hit in that
    /// order; re-setting an existing owner updates in place, keeping its
    /// position (C4TransferZone.cpp:83-88).
    zones: Vec<TransferZone>,
}

impl TransferZoneTable {
    pub fn get(&self, owner: ObjectId) -> Option<&TransferZone> {
        self.zones.iter().find(|zone| zone.owner == owner)
    }

    pub fn set(&mut self, owner: ObjectId, rect: TransferZoneRect) {
        if rect.width == 0 || rect.height == 0 {
            self.clear(owner);
            return;
        }
        if let Some(zone) = self.zones.iter_mut().find(|zone| zone.owner == owner) {
            zone.x = rect.x;
            zone.y = rect.y;
            zone.width = rect.width;
            zone.height = rect.height;
            return;
        }
        self.zones.insert(
            0,
            TransferZone {
                owner,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                used: false,
            },
        );
    }

    pub fn clear(&mut self, owner: ObjectId) {
        self.zones.retain(|zone| zone.owner != owner);
    }

    pub fn retain_existing(&mut self, alive: &HashSet<ObjectId>) {
        self.zones.retain(|zone| alive.contains(&zone.owner));
    }

    pub fn states(&self) -> Vec<TransferZoneState> {
        self.zones
            .iter()
            .map(|zone| TransferZoneState {
                owner: zone.owner,
                x: zone.x,
                y: zone.y,
                width: zone.width,
                height: zone.height,
            })
            .collect()
    }

    pub fn from_states(states: &[TransferZoneState]) -> Self {
        let mut table = Self::default();
        for state in states {
            table.zones.push(TransferZone {
                owner: state.owner,
                x: state.x,
                y: state.y,
                width: state.width,
                height: state.height,
                used: false,
            });
        }
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> TransferZoneRect {
        TransferZoneRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn zones_keep_cpp_traversal_order() {
        // C4TransferZones::Add prepends — newest zone first
        // (C4TransferZone.cpp:104-105); Set updates an existing zone in
        // place, keeping its list position (:83-88).
        let mut table = TransferZoneTable::default();
        table.set(ObjectId::new(1), rect(0, 0, 10, 10));
        table.set(ObjectId::new(2), rect(5, 0, 10, 10));
        let states = table.states();
        assert_eq!(states[0].owner, ObjectId::new(2), "newest first");
        assert_eq!(states[1].owner, ObjectId::new(1));

        table.set(ObjectId::new(1), rect(0, 0, 20, 20));
        let states = table.states();
        assert_eq!(states[0].owner, ObjectId::new(2), "update keeps position");
        assert_eq!(states[1].owner, ObjectId::new(1));
        assert_eq!(states[1].width, 20);

        let table = TransferZoneTable::from_states(&states);
        assert_eq!(table.states(), states, "round trip preserves order");
    }

    #[test]
    fn negative_transfer_zone_keeps_cpp_traversal_slot() {
        // C4TransferZones::Set clears only exact-zero extents. A negative
        // extent remains as an inert entry and a later positive update
        // mutates that same list node instead of prepending it (:78-91).
        let first = ObjectId::new(1);
        let second = ObjectId::new(2);
        let mut table = TransferZoneTable::default();
        table.set(first, rect(0, 0, 10, 10));
        table.set(second, rect(0, 0, 10, 10));

        table.set(first, rect(0, 0, -1, 10));
        let states = table.states();
        assert_eq!(
            states.iter().map(|zone| zone.owner).collect::<Vec<_>>(),
            [second, first]
        );
        assert_eq!(states[1].width, -1);

        table.set(first, rect(0, 0, 10, 10));
        assert_eq!(
            table
                .states()
                .iter()
                .map(|zone| zone.owner)
                .collect::<Vec<_>>(),
            [second, first],
            "restoring the overlapping zone keeps its original traversal slot"
        );

        table.set(first, rect(0, 0, 0, 10));
        assert_eq!(
            table
                .states()
                .iter()
                .map(|zone| zone.owner)
                .collect::<Vec<_>>(),
            [second]
        );
    }
}
