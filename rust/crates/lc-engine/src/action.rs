use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DEFAULT_ACTION_NAME: &str = "Idle";

/// Configuration for how an action should advance and transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSpec {
    #[serde(default)]
    pub length: Option<u32>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub procedure: Option<String>,
}

impl ActionSpec {
    pub fn new(length: Option<u32>, next: Option<String>) -> Self {
        Self {
            length,
            next,
            procedure: None,
        }
    }

    pub fn with_length(mut self, length: u32) -> Self {
        self.length = Some(length);
        self
    }

    pub fn with_next(mut self, next: impl Into<String>) -> Self {
        self.next = Some(next.into());
        self
    }

    pub fn with_procedure(mut self, procedure: impl Into<String>) -> Self {
        self.procedure = Some(procedure.into());
        self
    }
}

impl Default for ActionSpec {
    fn default() -> Self {
        Self {
            length: None,
            next: None,
            procedure: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionProcedure {
    Undefined,
    Walk,
    Float,
    Flight,
    Hang,
    Swim,
    Other,
}

impl ActionProcedure {
    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("walk") {
            ActionProcedure::Walk
        } else if name.eq_ignore_ascii_case("float") {
            ActionProcedure::Float
        } else if name.eq_ignore_ascii_case("flight") {
            ActionProcedure::Flight
        } else if name.eq_ignore_ascii_case("hang") {
            ActionProcedure::Hang
        } else if name.eq_ignore_ascii_case("swim") {
            ActionProcedure::Swim
        } else {
            ActionProcedure::Other
        }
    }

    pub fn gravity_component(self, base_gravity: i32) -> i32 {
        match self {
            ActionProcedure::Undefined | ActionProcedure::Walk | ActionProcedure::Other => {
                base_gravity
            }
            ActionProcedure::Float | ActionProcedure::Swim => {
                let mut magnitude = base_gravity.abs();
                if magnitude > 0 {
                    magnitude = (magnitude + 1) / 2;
                    if magnitude == 0 {
                        magnitude = 1;
                    }
                }
                if base_gravity < 0 {
                    -magnitude
                } else {
                    magnitude
                }
            }
            ActionProcedure::Flight | ActionProcedure::Hang => 0,
        }
    }

    pub fn allows_wind(self) -> bool {
        match self {
            ActionProcedure::Flight | ActionProcedure::Hang => false,
            _ => true,
        }
    }

    pub fn locks_vertical_velocity(self) -> bool {
        matches!(self, ActionProcedure::Hang)
    }
}

impl Default for ActionProcedure {
    fn default() -> Self {
        ActionProcedure::Undefined
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionLibrary {
    default: String,
    specs: HashMap<String, ActionSpec>,
}

impl ActionLibrary {
    pub fn new(default_action: Option<String>, mut specs: HashMap<String, ActionSpec>) -> Self {
        let default = default_action
            .or_else(|| {
                if specs.contains_key(DEFAULT_ACTION_NAME) {
                    Some(DEFAULT_ACTION_NAME.to_string())
                } else {
                    None
                }
            })
            .or_else(|| specs.keys().next().cloned())
            .unwrap_or_else(|| DEFAULT_ACTION_NAME.to_string());

        if !specs.contains_key(&default) {
            specs.insert(default.clone(), ActionSpec::default());
        }

        Self { default, specs }
    }

    pub fn default_action(&self) -> &str {
        &self.default
    }

    pub fn contains(&self, action: &str) -> bool {
        self.specs.contains_key(action)
    }

    pub fn specs(&self) -> &HashMap<String, ActionSpec> {
        &self.specs
    }

    pub fn advance_state(&self, state: &mut ActionState) {
        if let Some(spec) = self.specs.get(&state.name) {
            Self::advance_with_spec(state, spec, self)
        } else {
            state.advance();
        }
    }

    pub fn procedure_for_action(&self, action: &str) -> ActionProcedure {
        self.specs
            .get(action)
            .and_then(|spec| spec.procedure.as_deref())
            .map(ActionProcedure::from_name)
            .unwrap_or_default()
    }

    pub fn procedure_name_for_action(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.procedure.as_deref())
    }

    fn advance_with_spec(state: &mut ActionState, spec: &ActionSpec, library: &ActionLibrary) {
        if let Some(length) = spec.length {
            if length == 0 {
                Self::transition(state, spec, library);
                return;
            }

            let next_phase = state.phase.saturating_add(1);
            if next_phase >= length as i32 {
                Self::transition(state, spec, library);
                return;
            }

            state.phase = next_phase;
        } else {
            state.advance();
        }
    }

    fn transition(state: &mut ActionState, spec: &ActionSpec, library: &ActionLibrary) {
        let next_name = spec.next.as_deref().unwrap_or(&state.name);
        let resolved = if library.contains(next_name) {
            next_name
        } else {
            library.default_action()
        };

        if resolved != state.name {
            state.name = resolved.to_string();
        }
        state.phase = 0;
    }
}

impl Default for ActionLibrary {
    fn default() -> Self {
        Self::new(None, HashMap::new())
    }
}

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

    pub fn advance_with_library(&mut self, library: &ActionLibrary) {
        library.advance_state(self);
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
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
