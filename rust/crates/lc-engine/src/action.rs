use serde::{Deserialize, Serialize};

/// Minimal representation of an object's current action state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionState {
    pub name: String,
    pub phase: i32,
}

impl ActionState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: 0,
        }
    }

    pub fn advance(&mut self) {
        if self.phase < i32::MAX {
            self.phase += 1;
        }
    }

    pub fn reset_phase(&mut self) {
        self.phase = 0;
    }

    pub fn apply_update(&mut self, update: &ActionUpdate) {
        if let Some(name) = &update.name {
            if *name != self.name {
                self.name = name.clone();
                self.phase = 0;
            }
        }
        if let Some(phase) = update.phase {
            self.phase = phase;
        }
    }
}

impl Default for ActionState {
    fn default() -> Self {
        Self::new("Idle")
    }
}

/// Partial update to an object's action state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActionUpdate {
    pub name: Option<String>,
    pub phase: Option<i32>,
}

impl ActionUpdate {
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_phase(mut self, phase: i32) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = Some(name.into());
    }

    pub fn set_phase(&mut self, phase: i32) {
        self.phase = Some(phase);
    }

    pub fn merge(&mut self, other: ActionUpdate) {
        if other.name.is_some() {
            self.name = other.name;
        }
        if other.phase.is_some() {
            self.phase = other.phase;
        }
    }
}
