use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{math::C4Fixed, ObjectId};

const DEFAULT_ACTION_NAME: &str = "Idle";

/// Configuration for how an action should advance and transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ActionSpec {
    #[serde(default)]
    pub length: Option<u32>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub procedure: Option<String>,
    #[serde(default)]
    pub delay: Option<u32>,
    #[serde(default)]
    pub step: Option<u32>,
    #[serde(default)]
    pub phase_call: Option<String>,
    #[serde(default)]
    pub start_call: Option<String>,
    #[serde(default)]
    pub end_call: Option<String>,
    #[serde(default)]
    pub abort_call: Option<String>,
    #[serde(default)]
    pub no_other_action: bool,
    #[serde(default)]
    pub dig_free: Option<i32>,
    #[serde(default)]
    pub attach: u32,
}

impl ActionSpec {
    pub fn new(length: Option<u32>, next: Option<String>) -> Self {
        Self {
            length,
            next,
            procedure: None,
            delay: None,
            step: None,
            phase_call: None,
            start_call: None,
            end_call: None,
            abort_call: None,
            no_other_action: false,
            dig_free: None,
            attach: 0,
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

    pub fn with_delay(mut self, delay: u32) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn with_step(mut self, step: u32) -> Self {
        self.step = Some(step);
        self
    }

    pub fn with_phase_call(mut self, phase_call: impl Into<String>) -> Self {
        self.phase_call = Some(phase_call.into());
        self
    }

    pub fn with_start_call(mut self, start_call: impl Into<String>) -> Self {
        self.start_call = Some(start_call.into());
        self
    }

    pub fn with_end_call(mut self, end_call: impl Into<String>) -> Self {
        self.end_call = Some(end_call.into());
        self
    }

    pub fn with_abort_call(mut self, abort_call: impl Into<String>) -> Self {
        self.abort_call = Some(abort_call.into());
        self
    }

    pub fn with_no_other_action(mut self, enabled: bool) -> Self {
        self.no_other_action = enabled;
        self
    }

    pub fn with_dig_free(mut self, dig_free: i32) -> Self {
        self.dig_free = Some(dig_free);
        self
    }

    pub fn with_attach(mut self, attach: u32) -> Self {
        self.attach = attach;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionProcedure {
    #[default]
    Undefined,
    Walk,
    Float,
    Flight,
    Hang,
    Swim,
    Kneel,
    Scale,
    Dig,
    Throw,
    Bridge,
    Build,
    Push,
    Chop,
    Lift,
    Attach,
    Fight,
    Connect,
    Pull,
    Other,
}

impl ActionProcedure {
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "walk" => ActionProcedure::Walk,
            "float" => ActionProcedure::Float,
            "flight" => ActionProcedure::Flight,
            "hang" | "hangle" => ActionProcedure::Hang,
            "swim" => ActionProcedure::Swim,
            "kneel" => ActionProcedure::Kneel,
            "scale" => ActionProcedure::Scale,
            "dig" => ActionProcedure::Dig,
            "throw" => ActionProcedure::Throw,
            "bridge" => ActionProcedure::Bridge,
            "build" => ActionProcedure::Build,
            "push" => ActionProcedure::Push,
            "chop" => ActionProcedure::Chop,
            "lift" => ActionProcedure::Lift,
            "attach" => ActionProcedure::Attach,
            "fight" => ActionProcedure::Fight,
            "connect" => ActionProcedure::Connect,
            "pull" => ActionProcedure::Pull,
            "walkto" => ActionProcedure::Walk,
            "dive" => ActionProcedure::Swim,
            "tumble" => ActionProcedure::Flight,
            "dead" | "dead2" => ActionProcedure::Flight,
            _ => ActionProcedure::Other,
        }
    }

    pub fn gravity_component(self, base_gravity: i32) -> i32 {
        match self {
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
            ActionProcedure::Hang | ActionProcedure::Attach | ActionProcedure::Scale => 0,
            ActionProcedure::Dig => 0,
            ActionProcedure::Undefined
            | ActionProcedure::Flight
            | ActionProcedure::Walk
            | ActionProcedure::Kneel
            | ActionProcedure::Throw
            | ActionProcedure::Bridge
            | ActionProcedure::Build
            | ActionProcedure::Push
            | ActionProcedure::Chop
            | ActionProcedure::Lift
            | ActionProcedure::Fight
            | ActionProcedure::Connect
            | ActionProcedure::Pull
            | ActionProcedure::Other => base_gravity,
        }
    }

    pub fn gravity_component_fixed(self, base_gravity: C4Fixed) -> C4Fixed {
        match self {
            ActionProcedure::Float | ActionProcedure::Swim => {
                let raw = i64::from(base_gravity.val());
                let mut magnitude = raw.abs();
                if magnitude > 0 {
                    magnitude = (magnitude + 1) / 2;
                    if magnitude == 0 {
                        magnitude = 1;
                    }
                }
                C4Fixed::from_raw((if raw < 0 { -magnitude } else { magnitude }) as i32)
            }
            ActionProcedure::Hang | ActionProcedure::Attach | ActionProcedure::Scale => {
                C4Fixed::ZERO
            }
            ActionProcedure::Dig => C4Fixed::ZERO,
            ActionProcedure::Undefined
            | ActionProcedure::Flight
            | ActionProcedure::Walk
            | ActionProcedure::Kneel
            | ActionProcedure::Throw
            | ActionProcedure::Bridge
            | ActionProcedure::Build
            | ActionProcedure::Push
            | ActionProcedure::Chop
            | ActionProcedure::Lift
            | ActionProcedure::Fight
            | ActionProcedure::Connect
            | ActionProcedure::Pull
            | ActionProcedure::Other => base_gravity,
        }
    }

    pub fn allows_wind(self) -> bool {
        !matches!(
            self,
            ActionProcedure::Flight
                | ActionProcedure::Hang
                | ActionProcedure::Attach
                | ActionProcedure::Swim
                | ActionProcedure::Dig
                | ActionProcedure::Kneel
                | ActionProcedure::Bridge
                | ActionProcedure::Build
                | ActionProcedure::Throw
                | ActionProcedure::Connect
                | ActionProcedure::Scale
                | ActionProcedure::Push
                | ActionProcedure::Pull
                | ActionProcedure::Chop
                | ActionProcedure::Fight
        )
    }

    pub fn locks_vertical_velocity(self) -> bool {
        matches!(
            self,
            ActionProcedure::Hang
                | ActionProcedure::Attach
                | ActionProcedure::Kneel
                | ActionProcedure::Bridge
                | ActionProcedure::Build
                | ActionProcedure::Throw
                | ActionProcedure::Connect
                | ActionProcedure::Push
                | ActionProcedure::Pull
                | ActionProcedure::Chop
                | ActionProcedure::Fight
        )
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

    pub fn blocks_other_actions(&self, action: &str) -> bool {
        self.specs
            .get(action)
            .map(|spec| spec.no_other_action)
            .unwrap_or(false)
    }

    pub fn start_call_for_action(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.start_call.as_deref())
    }

    pub fn end_call_for_action(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.end_call.as_deref())
    }

    pub fn phase_call_for_action(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.phase_call.as_deref())
    }

    pub fn abort_call_for_action(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.abort_call.as_deref())
    }

    pub fn advance_state(&self, state: &mut ActionState) -> ActionAdvanceOutcome {
        if let Some(spec) = self.specs.get(&state.name) {
            Self::advance_with_spec(state, spec, self)
        } else {
            state.advance();
            ActionAdvanceOutcome::default()
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

    pub fn dig_free_for_action(&self, action: &str) -> Option<i32> {
        self.specs.get(action).and_then(|spec| spec.dig_free)
    }

    pub fn attach_for_action(&self, action: &str) -> u32 {
        self.specs.get(action).map(|spec| spec.attach).unwrap_or(0)
    }

    fn advance_with_spec(
        state: &mut ActionState,
        spec: &ActionSpec,
        library: &ActionLibrary,
    ) -> ActionAdvanceOutcome {
        let mut outcome = ActionAdvanceOutcome::default();

        if let Some(length) = spec.length {
            if length == 0 {
                Self::transition(state, spec, library);
                return outcome;
            }
        }

        let delay = spec.delay.unwrap_or(1).max(1);
        if delay > 1 {
            state.ticks = state.ticks.saturating_add(1);
            if state.ticks < delay {
                return outcome;
            }
        }
        state.ticks = 0;

        let step = normalize_step(spec.step);
        let current_action = state.name.clone();

        if let Some(length) = spec.length {
            let length = i32::try_from(length).unwrap_or(i32::MAX);
            let next_phase = state.phase.saturating_add(step);
            if spec.phase_call.is_some() {
                outcome.phase_event = Some(ActionPhaseEvent {
                    action: current_action.clone(),
                    phase: next_phase,
                });
            }

            if next_phase >= length {
                state.phase = next_phase;
                Self::transition(state, spec, library);
            } else {
                state.phase = next_phase;
            }
        } else {
            state.phase = state.phase.saturating_add(step);
            if spec.phase_call.is_some() {
                outcome.phase_event = Some(ActionPhaseEvent {
                    action: current_action,
                    phase: state.phase,
                });
            }
        }

        outcome
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
        state.ticks = 0;
    }
}

impl Default for ActionLibrary {
    fn default() -> Self {
        Self::new(None, HashMap::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActionAdvanceOutcome {
    pub phase_event: Option<ActionPhaseEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPhaseEvent {
    pub action: String,
    pub phase: i32,
}

fn normalize_step(step: Option<u32>) -> i32 {
    let value = step.unwrap_or(1).max(1);
    let clamped = value.min(i32::MAX as u32);
    clamped as i32
}

/// Minimal representation of an object's current action state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionState {
    pub name: String,
    pub phase: i32,
    #[serde(default)]
    pub ticks: u32,
    #[serde(default)]
    pub data: i32,
    #[serde(default)]
    pub target: Option<ObjectId>,
    #[serde(default)]
    pub target2: Option<ObjectId>,
}

impl ActionState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: 0,
            ticks: 0,
            data: 0,
            target: None,
            target2: None,
        }
    }

    pub fn advance(&mut self) {
        self.phase = self.phase.saturating_add(1);
        self.ticks = 0;
    }

    pub fn advance_with_library(&mut self, library: &ActionLibrary) -> ActionAdvanceOutcome {
        library.advance_state(self)
    }

    pub fn reset_phase(&mut self) {
        self.phase = 0;
        self.ticks = 0;
    }

    pub fn apply_update(&mut self, update: &ActionUpdate) {
        if let Some(name) = &update.name {
            if *name != self.name {
                self.name = name.clone();
                self.phase = 0;
                self.ticks = 0;
            }
        }
        if let Some(phase) = update.phase {
            self.phase = phase;
            if update.ticks.is_none() {
                self.ticks = 0;
            }
        }
        if let Some(ticks) = update.ticks {
            self.ticks = ticks;
        }
        if let Some(data) = update.data {
            self.data = data;
        }
        if let Some(target) = update.target {
            self.target = target;
        }
        if let Some(target2) = update.target2 {
            self.target2 = target2;
        }
    }

    pub fn apply_update_with_library(
        &mut self,
        update: &ActionUpdate,
        library: &ActionLibrary,
    ) -> ActionUpdateResult {
        let mut resolved = update.clone();
        if let Some(name) = resolved.name.as_ref() {
            if !resolved.force && library.blocks_other_actions(&self.name) && name != &self.name {
                return ActionUpdateResult::Blocked;
            }
        }
        if let Some(name) = resolved.name.as_ref() {
            if !library.contains(name) {
                resolved.name = Some(library.default_action().to_string());
                resolved.phase = Some(0);
                resolved.ticks = Some(0);
            }
        }

        let previous_name = self.name.clone();
        let previous_procedure = library.procedure_for_action(&previous_name);

        self.apply_update(&resolved);

        let next_name = self.name.clone();
        let next_procedure = library.procedure_for_action(&next_name);
        if previous_name != next_name
            && previous_procedure != next_procedure
            && resolved.data.is_none()
        {
            self.data = 0;
        }
        self.reconcile_with_library(library);
        ActionUpdateResult::Applied
    }

    pub fn reconcile_with_library(&mut self, library: &ActionLibrary) {
        if !library.contains(&self.name) {
            self.name = library.default_action().to_string();
            self.phase = 0;
            self.ticks = 0;
        }
    }
}

impl Default for ActionState {
    fn default() -> Self {
        Self::new("Idle")
    }
}

/// Partial update to an object's action state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionUpdate {
    pub name: Option<String>,
    pub phase: Option<i32>,
    #[serde(default)]
    pub ticks: Option<u32>,
    #[serde(default = "ActionUpdate::default_force")]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Option<ObjectId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target2: Option<Option<ObjectId>>,
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

    pub fn with_ticks(mut self, ticks: u32) -> Self {
        self.ticks = Some(ticks);
        self
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = Some(name.into());
    }

    pub fn set_phase(&mut self, phase: i32) {
        self.phase = Some(phase);
    }

    pub fn set_ticks(&mut self, ticks: u32) {
        self.ticks = Some(ticks);
    }

    pub fn with_data(mut self, data: i32) -> Self {
        self.data = Some(data);
        self
    }

    pub fn set_data(&mut self, data: i32) {
        self.data = Some(data);
    }

    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    pub fn set_force(&mut self, force: bool) {
        self.force = force;
    }

    pub fn with_target(mut self, target: Option<ObjectId>) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_target2(mut self, target: Option<ObjectId>) -> Self {
        self.target2 = Some(target);
        self
    }

    pub fn set_target(&mut self, target: Option<ObjectId>) {
        self.target = Some(target);
    }

    pub fn set_target2(&mut self, target: Option<ObjectId>) {
        self.target2 = Some(target);
    }

    pub fn merge(&mut self, other: ActionUpdate) {
        if other.name.is_some() {
            self.name = other.name;
        }
        if other.phase.is_some() {
            self.phase = other.phase;
        }
        if other.ticks.is_some() {
            self.ticks = other.ticks;
        }
        if !other.force {
            self.force = false;
        }
        if other.data.is_some() {
            self.data = other.data;
        }
        if other.target.is_some() {
            self.target = other.target;
        }
        if other.target2.is_some() {
            self.target2 = other.target2;
        }
    }

    fn default_force() -> bool {
        true
    }
}

impl Default for ActionUpdate {
    fn default() -> Self {
        Self {
            name: None,
            phase: None,
            ticks: None,
            force: true,
            data: None,
            target: None,
            target2: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionUpdateResult {
    Applied,
    Blocked,
}
