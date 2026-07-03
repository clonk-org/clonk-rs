use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{math::C4Fixed, ObjectId};

pub(crate) const DEFAULT_ACTION_NAME: &str = "Idle";

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
    /// `InLiquidAction` (C4ActionDef): the ExecAction head switches to
    /// it while InLiquid with an early return (C4Object.cpp:4749-4753).
    #[serde(default)]
    pub in_liquid_action: Option<String>,
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
            in_liquid_action: None,
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

    pub fn with_in_liquid_action(mut self, action: impl Into<String>) -> Self {
        self.in_liquid_action = Some(action.into());
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

    /// The C4Object::ExecAction gravity map (C4Object.cpp:4690-5437):
    /// only ActIdle-with-Mobile, DFA_FLIGHT (:4885), DFA_LIFT (:5265) and
    /// the no-Attach default case (:5437) run DoGravity — WALK/SCALE/
    /// HANGLE/SWIM/FLOAT/DIG/THROW/BRIDGE/... never add GravAccel (they
    /// steer with their own accelerations and pin ydir themselves).
    pub fn gravity_component_fixed(self, base_gravity: C4Fixed) -> C4Fixed {
        match self {
            ActionProcedure::Undefined
            | ActionProcedure::Flight
            | ActionProcedure::Lift
            | ActionProcedure::Other => base_gravity,
            _ => C4Fixed::ZERO,
        }
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
        // C++ has no default-action concept: objects start ActIdle unless
        // a script or the loader sets one (C4Object::Init leaves Act =
        // ActIdle). Only the synthetic fixture DSL supplies an explicit
        // default; never fabricate one from the map (HashMap order is
        // nondeterministic).
        let default = default_action.unwrap_or_else(|| DEFAULT_ACTION_NAME.to_string());

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
        self.advance_state_by(state, 1)
    }

    /// Phase advance with the C++ `iPhaseAdvance` weight — WALK scales by
    /// fixtoi(|xdir| * 10), SCALE by fixtoi(|ydir| * 14), everything else
    /// is 1 (C4Object.cpp:4696,4787-4789,4830-4832); 0 freezes the
    /// animation (a standing walker).
    pub fn advance_state_by(
        &self,
        state: &mut ActionState,
        phase_advance: i32,
    ) -> ActionAdvanceOutcome {
        if let Some(spec) = self.specs.get(&state.name) {
            Self::advance_with_spec(state, spec, self, phase_advance)
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

    pub fn in_liquid_action_for(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.in_liquid_action.as_deref())
    }

    pub fn dig_free_for_action(&self, action: &str) -> Option<i32> {
        self.specs.get(action).and_then(|spec| spec.dig_free)
    }

    pub fn attach_for_action(&self, action: &str) -> u32 {
        self.specs.get(action).map(|spec| spec.attach).unwrap_or(0)
    }

    /// True for the auto-inserted BARE default "Idle" spec — the C++
    /// `Action.Act <= ActIdle` state (C4Object.cpp:4708). A REAL phased
    /// ActMap action named "Idle" (fixture libraries may define one) is
    /// an active action, not idle.
    pub fn is_idle_action(&self, action: &str) -> bool {
        action == "Idle"
            && self
                .specs
                .get("Idle")
                .map(|spec| *spec == ActionSpec::default())
                .unwrap_or(true)
    }

    fn advance_with_spec(
        state: &mut ActionState,
        spec: &ActionSpec,
        library: &ActionLibrary,
        phase_advance: i32,
    ) -> ActionAdvanceOutcome {
        let mut outcome = ActionAdvanceOutcome::default();

        // Action.Time++ (C4Object.cpp:4745): counts every ExecAction of a
        // real action, independent of the phase machinery below.
        state.time = state.time.saturating_add(1);

        // Phase advance is gated on a nonzero Delay — "zero delay means no
        // phase advance" (C4Object.cpp:5441; the ActMap default is 0,
        // C4Def.h:151).
        let Some(delay) = spec.delay.filter(|delay| *delay > 0) else {
            return outcome;
        };

        // PhaseDelay += iPhaseAdvance; the phase moves when it reaches
        // Delay and the counter restarts (C4Object.cpp:5443-5447) — a zero
        // advance (standing walker) freezes the animation.
        state.ticks = state.ticks.saturating_add(phase_advance.max(0) as u32);
        if state.ticks < delay {
            return outcome;
        }
        state.ticks = 0;

        let step = normalize_step(spec.step);
        let current_action = state.name.clone();
        // Phase += Step, then the PhaseCall, then the length check
        // (C4Object.cpp:5448-5464).
        state.phase = state.phase.saturating_add(step);
        if spec.phase_call.is_some() {
            outcome.phase_event = Some(ActionPhaseEvent {
                action: current_action,
                phase: state.phase,
            });
        }
        // Length defaults to 1 (C4Def.h:150).
        let length = spec
            .length
            .map(|length| i32::try_from(length).unwrap_or(i32::MAX))
            .unwrap_or(1);
        if state.phase >= length {
            // C++ runs the phase-end transition through
            // SetAction(NextAction, SAC_StartCall | SAC_EndCall)
            // (C4Object.cpp:5462) — EndCall + StartCall fire even when
            // NextAction chains the SAME action (the palm's Breeze
            // StartCall re-evaluates the wind every cycle). Hold clamps
            // without a transition.
            outcome.wrapped = Self::transition(state, spec, library, length);
        }

        outcome
    }

    /// Returns true when the NextAction transition ran (false for Hold).
    fn transition(
        state: &mut ActionState,
        spec: &ActionSpec,
        library: &ActionLibrary,
        length: i32,
    ) -> bool {
        // NextAction=Hold clamps at the last phase and keeps the action
        // (ActHold, C4Def.cpp:786-787; C4Object.cpp:5457-5459).
        if spec
            .next
            .as_deref()
            .is_some_and(|next| next.eq_ignore_ascii_case("Hold"))
        {
            state.phase = (length - 1).max(0);
            return false;
        }
        // An absent NextAction is ActIdle (C4Def.h:154), and an unresolved
        // NextActionName stays ActIdle too (the C4Def::Load mapping loop,
        // C4Def.cpp:784-792) — both go to the literal Idle state, NOT the
        // library's default SPAWN action.
        let resolved = spec
            .next
            .as_deref()
            .filter(|next| library.contains(next))
            .unwrap_or(DEFAULT_ACTION_NAME);

        if resolved != state.name {
            state.name = resolved.to_string();
            // Action.Time resets on the action CHANGE only
            // (C4Object.cpp:4106-4108); a self-chain keeps counting.
            state.time = 0;
        }
        state.phase = 0;
        state.ticks = 0;
        true
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
    /// The phase-end NextAction transition ran (C4Object.cpp:5462) —
    /// the caller owes an EndCall+StartCall pair even for a same-name
    /// chain.
    pub wrapped: bool,
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
    /// `Action.PhaseDelay` (C4Object.cpp:5443-5447): the intra-phase
    /// counter, restarting every phase advance.
    #[serde(default)]
    pub ticks: u32,
    /// `Action.Time` (C4Object.cpp:4745): total frames in the current
    /// action, reset only when the action CHANGES (C4Object.cpp:4106-4108).
    /// GetActTime reads this (C4Script.cpp).
    #[serde(default)]
    pub time: u32,
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
            time: 0,
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

    pub fn advance_with_library_by(
        &mut self,
        library: &ActionLibrary,
        phase_advance: i32,
    ) -> ActionAdvanceOutcome {
        library.advance_state_by(self, phase_advance)
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
                // Action.Time resets on the action change
                // (C4Object.cpp:4106-4108).
                self.time = 0;
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
        // ActIdle carries no phase or time: SetActionByName("Idle") clears
        // the action instead of resolving it (C4Object.cpp:4214-4215) —
        // the load-time Phase restore (C4Object.cpp:2840-2849) only
        // applies to real ActMap actions. (Fixture libraries may define a
        // REAL phased "Idle" stays untouched; the auto-inserted BARE default spec marks true idle.)
        if library.is_idle_action(&self.name) {
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
    /// The script SetAction seam already ran AbortCall/StartCall
    /// synchronously (C4Object::SetAction fires them inside the call) —
    /// the fold must not queue duplicate transition callbacks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub callbacks_dispatched: bool,
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
            callbacks_dispatched: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionUpdateResult {
    Applied,
    Blocked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_carries_no_phase_like_cpp() {
        // SetActionByName("Idle"/"ActIdle") clears the action instead of
        // looking it up (C4Object.cpp:4214-4215), and ActIdle execution
        // keeps Phase/Time at zero — a loaded `Action=Idle\nPhase=1`
        // savegame entry reads phase 0 in C++ at the first frame
        // (the load-time restore at C4Object.cpp:2840-2849 only applies
        // to ActMap actions).
        let library = ActionLibrary::default();
        let mut state = ActionState::new("Idle");
        state.phase = 1;
        state.ticks = 3;
        state.reconcile_with_library(&library);
        assert_eq!(state.phase, 0, "idle has no phase");
        assert_eq!(state.ticks, 0, "idle has no action time");
    }

    fn library_with(specs: Vec<(&str, ActionSpec)>) -> ActionLibrary {
        let map: std::collections::HashMap<String, ActionSpec> = specs
            .into_iter()
            .map(|(name, spec)| (name.to_string(), spec))
            .collect();
        ActionLibrary::new(None, map)
    }

    // C4Object::ExecAction scales the WALK phase advance by speed
    // (C4Object.cpp:4787-4789): PhaseDelay += fixtoi(|xdir| * 10) — a
    // walker at xdir 1.5 crosses Delay=10 every frame, a standing walker
    // (xdir 0) never animates.
    #[test]
    fn walk_phase_advance_scales_with_xdir_like_cpp() {
        let library = ActionLibrary::new(
            None,
            HashMap::from([(
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_delay(10)
                    .with_length(8),
            )]),
        );
        let mut state = ActionState::new("Walk");

        state.advance_with_library_by(&library, 15);
        assert_eq!(state.phase, 1, "advance 15 crosses Delay=10 in one frame");

        let phase = state.phase;
        state.advance_with_library_by(&library, 0);
        assert_eq!(state.phase, phase, "standing walker never animates");
    }

    // C4Object::ExecAction phase advance (C4Object.cpp:5441): "zero delay
    // means no phase advance" — the whole block is gated on pAction->Delay,
    // so a Delay=0 action never moves its phase and never chains.
    #[test]
    fn zero_delay_action_never_advances_like_cpp() {
        let library = library_with(vec![(
            "Still",
            ActionSpec::default()
                .with_length(2)
                .with_delay(0)
                .with_next("Gone"),
        )]);
        let mut state = ActionState::new("Still");
        for _ in 0..10 {
            library.advance_state(&mut state);
        }
        assert_eq!(state.name, "Still", "Delay=0 freezes the action");
        assert_eq!(state.phase, 0, "Delay=0 freezes the phase");
    }

    // NextAction=Hold (ActHold, C4Def.cpp:786-787): the phase clamps to
    // Length-1 and the action STAYS (C4Object.cpp:5457-5459).
    #[test]
    fn next_action_hold_clamps_last_phase_like_cpp() {
        let library = library_with(vec![(
            "Open",
            ActionSpec::default()
                .with_length(3)
                .with_delay(1)
                .with_next("Hold"),
        )]);
        let mut state = ActionState::new("Open");
        for _ in 0..10 {
            library.advance_state(&mut state);
        }
        assert_eq!(state.name, "Open", "Hold keeps the action");
        assert_eq!(state.phase, 2, "Hold clamps at Length-1");
    }

    // An empty NextAction maps to ActIdle (C4ActionDef default,
    // C4Def.h:154 / C4Def.cpp:784-792): the action ENDS to Idle when the
    // phase chain elapses — it does not loop.
    #[test]
    fn missing_next_action_ends_to_idle_like_cpp() {
        let library = library_with(vec![(
            "Flash",
            ActionSpec::default().with_length(2).with_delay(1),
        )]);
        let mut state = ActionState::new("Flash");
        library.advance_state(&mut state);
        assert_eq!(state.name, "Flash");
        library.advance_state(&mut state);
        assert_eq!(
            state.name, "Idle",
            "phase end without NextAction goes ActIdle (C4Def.h:154)"
        );
    }

    // Action.Time (C4Object.cpp:4745) counts EVERY ExecAction of a real
    // action — independent of Delay and never reset by phase advances —
    // while PhaseDelay (state.ticks) resets each phase
    // (C4Object.cpp:5443-5447).
    #[test]
    fn action_time_counts_independently_of_the_phase_delay_like_cpp() {
        let library = library_with(vec![(
            "Spin",
            ActionSpec::default()
                .with_length(100)
                .with_delay(3)
                .with_next("Hold"),
        )]);
        let mut state = ActionState::new("Spin");
        for _ in 0..7 {
            library.advance_state(&mut state);
        }
        assert_eq!(state.time, 7, "Action.Time counts every frame");
        assert_eq!(state.phase, 2, "two full 3-frame delays elapsed");
        assert_eq!(state.ticks, 1, "PhaseDelay restarts after each advance");
    }
}
