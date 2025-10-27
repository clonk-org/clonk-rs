use crate::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
    zones: HashMap<ObjectId, TransferZone>,
}

impl TransferZoneTable {
    pub fn get(&self, owner: ObjectId) -> Option<&TransferZone> {
        self.zones.get(&owner)
    }

    pub fn set(&mut self, owner: ObjectId, rect: TransferZoneRect) {
        if rect.width <= 0 || rect.height <= 0 {
            self.zones.remove(&owner);
            return;
        }
        let zone = TransferZone {
            owner,
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            used: false,
        };
        self.zones.insert(owner, zone);
    }

    pub fn clear(&mut self, owner: ObjectId) {
        self.zones.remove(&owner);
    }

    pub fn retain_existing(&mut self, alive: &HashSet<ObjectId>) {
        self.zones.retain(|owner, _| alive.contains(owner));
    }

    pub fn states(&self) -> Vec<TransferZoneState> {
        let mut states: Vec<_> = self
            .zones
            .values()
            .map(|zone| TransferZoneState {
                owner: zone.owner,
                x: zone.x,
                y: zone.y,
                width: zone.width,
                height: zone.height,
            })
            .collect();
        states.sort_by_key(|state| state.owner);
        states
    }

    pub fn from_states(states: &[TransferZoneState]) -> Self {
        let mut table = Self::default();
        for state in states {
            table.zones.insert(
                state.owner,
                TransferZone {
                    owner: state.owner,
                    x: state.x,
                    y: state.y,
                    width: state.width,
                    height: state.height,
                    used: false,
                },
            );
        }
        table
    }
}
