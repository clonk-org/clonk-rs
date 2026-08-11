use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use crate::{math::C4Fixed, ObjectId};
use clonk_resources::definition::ACT_HOLD;
use clonk_resources::ActionDefinition as ResourceActionDefinition;
use clonk_script::{ScriptFunctionResolution, Value};

pub const DEFAULT_ACTION_NAME: &str = "Idle";

pub(crate) fn is_builtin_idle_name(action: &str) -> bool {
    matches!(action, DEFAULT_ACTION_NAME | "ActIdle")
}

/// A native callback target selected during `C4DefScriptHost::AfterLink`.
/// Retaining the function body mirrors C++'s cached `C4AulScriptFunc *` and
/// prevents another name lookup before the next relink.
#[derive(Clone, Debug)]
pub(crate) struct ScriptCallbackTarget {
    function_name: String,
    resolution: Option<ScriptFunctionResolution>,
}

impl ScriptCallbackTarget {
    pub(crate) fn unlinked(function_name: impl Into<String>) -> Self {
        Self {
            function_name: function_name.into(),
            resolution: None,
        }
    }

    pub(crate) fn linked(
        function_name: impl Into<String>,
        resolution: ScriptFunctionResolution,
    ) -> Self {
        Self {
            function_name: function_name.into(),
            resolution: Some(resolution),
        }
    }

    pub(crate) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(crate) fn resolution(&self) -> Option<&ScriptFunctionResolution> {
        self.resolution.as_ref()
    }
}

/// Outer state distinguishes an unlinked synthetic fixture from C++'s
/// deliberately cached null pointer for a missing callback. Equality ignores
/// this runtime-only cache so ActionSpec retains metadata/value semantics.
#[derive(Clone, Default)]
pub(crate) enum ScriptCallbackLink {
    #[default]
    Unlinked,
    Linked(Option<ScriptCallbackTarget>),
}

impl std::fmt::Debug for ScriptCallbackLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unlinked => formatter.write_str("Unlinked"),
            Self::Linked(None) => formatter.write_str("Linked(None)"),
            Self::Linked(Some(target)) => formatter
                .debug_tuple("Linked")
                .field(&target.function_name())
                .finish(),
        }
    }
}

impl PartialEq for ScriptCallbackLink {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for ScriptCallbackLink {}

impl ScriptCallbackLink {
    pub(crate) fn target(&self, configured: Option<&str>) -> Option<ScriptCallbackTarget> {
        match self {
            Self::Unlinked => configured
                .filter(|name| !name.is_empty())
                .map(ScriptCallbackTarget::unlinked),
            Self::Linked(target) => target.clone(),
        }
    }

    pub(crate) fn set_linked(&mut self, target: Option<ScriptCallbackTarget>) {
        *self = Self::Linked(target);
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::Unlinked;
    }
}

#[derive(Clone, Debug, Default)]
struct ActionCallbackLinks {
    start: ScriptCallbackLink,
    phase: ScriptCallbackLink,
    end: ScriptCallbackLink,
    abort: ScriptCallbackLink,
}

/// Runtime-only cache. Equality deliberately ignores both link state and
/// retained function bodies so ActionLibrary keeps metadata/value semantics.
#[derive(Clone, Debug, Default)]
struct ActionCallbackCache {
    named: HashMap<String, ActionCallbackLinks>,
    physical: Vec<ActionCallbackLinks>,
}

impl PartialEq for ActionCallbackCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for ActionCallbackCache {}

/// Complete post-load `C4ActionDef::CompileFunc` view for GetActMapVal.
/// Runtime action fields and this reflection payload preserve the exact
/// signed compiler values and typed defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct C4ActionReflection {
    entries: HashMap<&'static str, Vec<Value>>,
}

impl C4ActionReflection {
    pub(crate) fn from_resource(name: &str, action: &ResourceActionDefinition) -> Self {
        let raw = |entry: &str, fallback: i32| {
            action
                .reflected_ints
                .get(entry)
                .copied()
                .unwrap_or(fallback)
        };
        let facet = action.facet.as_ref();
        let entries = HashMap::from([
            ("Name", vec![Value::String(name.to_string().into())]),
            (
                "Procedure",
                vec![Value::String(
                    action.procedure.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "Directions",
                vec![Value::Int(raw(
                    "Directions",
                    action.directions.unwrap_or(1),
                ))],
            ),
            (
                "FlipDir",
                vec![Value::Int(raw("FlipDir", action.flip_dir.unwrap_or(0)))],
            ),
            (
                "Length",
                vec![Value::Int(raw("Length", action.length.unwrap_or(1)))],
            ),
            (
                "Attach",
                vec![Value::Int(raw("Attach", action.attach as i32))],
            ),
            (
                "Delay",
                vec![Value::Int(raw("Delay", action.delay.unwrap_or(0)))],
            ),
            (
                "Facet",
                vec![
                    Value::Int(facet.map_or(0, |facet| facet.x)),
                    Value::Int(facet.map_or(0, |facet| facet.y)),
                    Value::Int(facet.map_or(0, |facet| facet.width)),
                    Value::Int(facet.map_or(0, |facet| facet.height)),
                    Value::Int(facet.map_or(0, |facet| facet.target_x)),
                    Value::Int(facet.map_or(0, |facet| facet.target_y)),
                ],
            ),
            (
                "FacetBase",
                vec![Value::Int(raw("FacetBase", i32::from(action.facet_base)))],
            ),
            (
                "FacetTopFace",
                vec![Value::Int(raw(
                    "FacetTopFace",
                    i32::from(action.facet_top_face),
                ))],
            ),
            (
                "FacetTargetStretch",
                vec![Value::Int(raw(
                    "FacetTargetStretch",
                    i32::from(action.facet_target_stretch),
                ))],
            ),
            (
                "NextAction",
                vec![Value::String(
                    action.next_action.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "NoOtherAction",
                vec![Value::Int(raw(
                    "NoOtherAction",
                    i32::from(action.no_other_action),
                ))],
            ),
            (
                "StartCall",
                vec![Value::String(
                    action.start_call.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "EndCall",
                vec![Value::String(
                    action.end_call.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "AbortCall",
                vec![Value::String(
                    action.abort_call.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "PhaseCall",
                vec![Value::String(
                    action.phase_call.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "Sound",
                vec![Value::String(
                    action.sound.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "ObjectDisabled",
                vec![Value::Int(raw(
                    "ObjectDisabled",
                    i32::from(action.disabled),
                ))],
            ),
            (
                "DigFree",
                vec![Value::Int(raw("DigFree", action.dig_free.unwrap_or(0)))],
            ),
            (
                "EnergyUsage",
                vec![Value::Int(raw("EnergyUsage", action.energy_usage))],
            ),
            (
                "InLiquidAction",
                vec![Value::String(
                    action.in_liquid_action.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "TurnAction",
                vec![Value::String(
                    action.turn_action.clone().unwrap_or_default().into(),
                )],
            ),
            (
                "Reverse",
                vec![Value::Int(raw("Reverse", i32::from(action.reverse)))],
            ),
            (
                "Step",
                vec![Value::Int(raw("Step", action.step.unwrap_or(1)))],
            ),
        ]);
        Self { entries }
    }

    pub(crate) fn get(&self, entry: &str, entry_nr: i32) -> Option<Value> {
        let index = usize::try_from(entry_nr).ok()?;
        self.entries.get(entry)?.get(index).cloned()
    }

    #[cfg(test)]
    fn entry_names(&self) -> impl Iterator<Item = &&'static str> {
        self.entries.keys()
    }
}

/// Configuration for how an action should advance and transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ActionSpec {
    #[serde(default)]
    pub length: Option<i32>,
    #[serde(default)]
    pub next: Option<String>,
    /// Resource-backed `C4ActionDef::NextAction`: an already CrossMapActMap-
    /// resolved physical slot, `ActIdle`, or `ActHold`. Synthetic action maps
    /// leave this unset and retain name-based lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_index: Option<i32>,
    #[serde(default)]
    pub procedure: Option<String>,
    #[serde(default)]
    pub delay: Option<i32>,
    #[serde(default)]
    pub step: Option<i32>,
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
    /// `ObjectDisabled=` (C4ActionDef::Disabled, C4Def.cpp:106): the
    /// action suspends the object — vetoes OCF_Collection and
    /// OCF_FightReady (SetOCF, C4Object.cpp:597,608).
    #[serde(default)]
    pub disabled: bool,
    /// `EnergyUsage=` (C4ActionDef::EnergyUsage, C4Def.cpp:108): signed
    /// energy consumed before Action.Time advances while the
    /// StructuresNeedEnergy rule is active (C4Object.cpp:4738-4753).
    #[serde(default)]
    pub energy_usage: i32,
    /// `InLiquidAction` (C4ActionDef): the ExecAction head switches to
    /// it while InLiquid with an early return (C4Object.cpp:4749-4753).
    #[serde(default)]
    pub in_liquid_action: Option<String>,
    /// `Directions` (C4ActionDef, default 1): SetDir rejects out-of-range
    /// directions (C4Object.cpp:4230).
    #[serde(default)]
    pub directions: Option<i32>,
    /// `FlipDir` (C4ActionDef, default 0 = never mirror): directions at or
    /// above it are drawn by mirroring the rows below it, which
    /// C4Object::UpdateFlipDir folds into the object's draw transform
    /// (C4Object.cpp:410-442).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_dir: Option<i32>,
    /// `TurnAction`: fired by SetDir on a direction change
    /// (C4Object.cpp:4233-4237).
    #[serde(default)]
    pub turn_action: Option<String>,
    #[serde(default)]
    pub dig_free: Option<i32>,
    #[serde(default)]
    pub attach: u32,
    /// `Sound=` (C4ActionDef::Sound, C4Def.cpp:104): a looping,
    /// object-attached sample started when the numeric action slot is
    /// entered and stopped when it is left (C4Object.cpp:4149-4152,
    /// 4186-4190). Presentation-only — never synchronized state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
}

impl ActionSpec {
    pub fn new(length: Option<i32>, next: Option<String>) -> Self {
        Self {
            length,
            next,
            next_index: None,
            procedure: None,
            delay: None,
            step: None,
            phase_call: None,
            start_call: None,
            end_call: None,
            abort_call: None,
            in_liquid_action: None,
            directions: None,
            flip_dir: None,
            turn_action: None,
            no_other_action: false,
            disabled: false,
            energy_usage: 0,
            dig_free: None,
            attach: 0,
            sound: None,
        }
    }

    pub fn with_length(mut self, length: i32) -> Self {
        self.length = Some(length);
        self
    }

    pub fn with_next(mut self, next: impl Into<String>) -> Self {
        self.next = Some(next.into());
        self
    }

    pub(crate) fn with_next_index(mut self, next_index: i32) -> Self {
        self.next_index = Some(next_index);
        self
    }

    /// The plain procedure action: defaults with only `procedure` set.
    ///
    /// Overwhelmingly the shape action maps build, and spelling it out let
    /// sites drift into `default()` chains that forgot the procedure.
    pub fn for_procedure(procedure: impl Into<String>) -> Self {
        Self::default().with_procedure(procedure)
    }

    pub fn with_procedure(mut self, procedure: impl Into<String>) -> Self {
        self.procedure = Some(procedure.into());
        self
    }

    pub fn with_sound(mut self, sound: impl Into<String>) -> Self {
        self.sound = Some(sound.into());
        self
    }

    pub fn with_delay(mut self, delay: i32) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn with_step(mut self, step: i32) -> Self {
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

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn with_energy_usage(mut self, energy_usage: i32) -> Self {
        self.energy_usage = energy_usage;
        self
    }

    pub fn with_directions(mut self, directions: i32) -> Self {
        self.directions = Some(directions);
        self
    }

    pub fn with_flip_dir(mut self, flip_dir: i32) -> Self {
        self.flip_dir = Some(flip_dir);
        self
    }

    pub fn with_turn_action(mut self, action: impl Into<String>) -> Self {
        self.turn_action = Some(action.into());
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
    /// Names physically declared in the ActMap/configuration. `specs` also
    /// contains a convenience default action that must not become visible to
    /// GetActMapVal when no matching C4ActionDef exists.
    declared: HashSet<String>,
    /// Resource-backed exact compiler views, omitted for synthetic manifests.
    reflections: HashMap<String, C4ActionReflection>,
    /// The resource ActMap's physical C4ActionDef array. Name-keyed maps
    /// intentionally keep their first match for SetActionByName and script
    /// reflection; this array preserves duplicate slot identity at runtime.
    physical: Vec<(String, ActionSpec)>,
    first_physical: HashMap<String, u32>,
    callback_cache: ActionCallbackCache,
}

/// Cheap, single-threaded sharing for engine-internal script-host scopes.
/// The public [`ActionLibrary`] remains an independently owned value; this
/// handle begins only at the engine's immutable definition-metadata boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedActionLibrary(Rc<ActionLibrary>);

impl From<ActionLibrary> for SharedActionLibrary {
    fn from(library: ActionLibrary) -> Self {
        Self(Rc::new(library))
    }
}

impl Deref for SharedActionLibrary {
    type Target = ActionLibrary;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SharedActionLibrary {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Rc::make_mut(&mut self.0)
    }
}

impl Default for SharedActionLibrary {
    fn default() -> Self {
        ActionLibrary::default().into()
    }
}

impl ActionLibrary {
    pub fn new(default_action: Option<String>, mut specs: HashMap<String, ActionSpec>) -> Self {
        // C++ has no default-action concept: objects start ActIdle unless
        // a script or the loader sets one (C4Object::Init leaves Act =
        // ActIdle). Only the synthetic fixture DSL supplies an explicit
        // default; never fabricate one from the map (HashMap order is
        // nondeterministic).
        let default = default_action.unwrap_or_else(|| DEFAULT_ACTION_NAME.to_string());
        let declared = specs.keys().cloned().collect();

        if !specs.contains_key(&default) {
            specs.insert(default.clone(), ActionSpec::default());
        }

        Self {
            default,
            specs,
            declared,
            reflections: HashMap::new(),
            physical: Vec::new(),
            first_physical: HashMap::new(),
            callback_cache: ActionCallbackCache::default(),
        }
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

    pub(crate) fn set_reflections(&mut self, reflections: HashMap<String, C4ActionReflection>) {
        self.reflections = reflections;
    }

    pub(crate) fn set_physical_actions(&mut self, physical: Vec<(String, ActionSpec)>) {
        let mut first_physical = HashMap::new();
        for (index, (name, _)) in physical.iter().enumerate() {
            first_physical
                .entry(name.clone())
                .or_insert(index.min(u32::MAX as usize) as u32);
        }
        self.physical = physical;
        self.first_physical = first_physical;
        self.callback_cache = ActionCallbackCache::default();
    }

    pub(crate) fn first_physical_index(&self, action: &str) -> Option<u32> {
        self.first_physical.get(action).copied()
    }

    pub(crate) fn named_action_index(&self, action: &str) -> Option<u32> {
        (!is_builtin_idle_name(action))
            .then(|| self.first_physical_index(action))
            .flatten()
    }

    fn physical_entry(&self, index: u32) -> Option<(&str, &ActionSpec)> {
        self.physical
            .get(index as usize)
            .map(|(name, spec)| (name.as_str(), spec))
    }

    pub(crate) fn spec_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<&ActionSpec> {
        let physical_spec = physical_index
            .and_then(|index| self.physical_entry(index))
            .filter(|(name, _)| *name == action)
            .map(|(_, spec)| spec);
        if physical_spec.is_some() {
            return physical_spec;
        }
        if !self.physical.is_empty() {
            if action == DEFAULT_ACTION_NAME {
                return None;
            }
            return self
                .first_physical_index(action)
                .and_then(|index| self.physical_entry(index))
                .map(|(_, spec)| spec);
        }
        self.specs.get(action)
    }

    pub(crate) fn spec_for_state(&self, state: &ActionState) -> Option<&ActionSpec> {
        self.spec_for_entry(&state.name, state.act_map_index)
    }

    pub(crate) fn is_idle_state(&self, state: &ActionState) -> bool {
        self.is_idle_entry(&state.name, state.act_map_index)
    }

    pub(crate) fn is_declared(&self, action: &str) -> bool {
        self.declared.contains(action)
    }

    pub(crate) fn reflection(&self, action: &str) -> Option<&C4ActionReflection> {
        self.reflections.get(action)
    }

    pub fn blocks_other_actions(&self, action: &str) -> bool {
        self.specs
            .get(action)
            .map(|spec| spec.no_other_action)
            .unwrap_or(false)
    }

    pub(crate) fn blocks_other_actions_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> bool {
        self.spec_for_entry(action, physical_index)
            .is_some_and(|spec| spec.no_other_action)
    }

    /// The SetOCF action gate `(Action.Act <= ActIdle) ||
    /// !ActMap[Act].Disabled` (C4Object.cpp:597,608): idle/unknown
    /// actions never disable the object.
    pub fn disables_object(&self, action: &str) -> bool {
        self.specs
            .get(action)
            .map(|spec| spec.disabled)
            .unwrap_or(false)
    }

    pub(crate) fn disables_object_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> bool {
        self.spec_for_entry(action, physical_index)
            .is_some_and(|spec| spec.disabled)
    }

    pub fn energy_usage_for_action(&self, action: &str) -> i32 {
        self.specs
            .get(action)
            .map(|spec| spec.energy_usage)
            .unwrap_or(0)
    }

    pub(crate) fn energy_usage_for_entry(&self, action: &str, physical_index: Option<u32>) -> i32 {
        self.spec_for_entry(action, physical_index)
            .map_or(0, |spec| spec.energy_usage)
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

    pub(crate) fn start_callback_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<ScriptCallbackTarget> {
        let spec = self.spec_for_entry(action, physical_index)?;
        match self.callback_links_for_entry(action, physical_index) {
            Some(links) => links.start.target(spec.start_call.as_deref()),
            None => spec
                .start_call
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(ScriptCallbackTarget::unlinked),
        }
    }

    pub(crate) fn end_callback_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<ScriptCallbackTarget> {
        let spec = self.spec_for_entry(action, physical_index)?;
        match self.callback_links_for_entry(action, physical_index) {
            Some(links) => links.end.target(spec.end_call.as_deref()),
            None => spec
                .end_call
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(ScriptCallbackTarget::unlinked),
        }
    }

    pub(crate) fn phase_callback_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<ScriptCallbackTarget> {
        let spec = self.spec_for_entry(action, physical_index)?;
        match self.callback_links_for_entry(action, physical_index) {
            Some(links) => links.phase.target(spec.phase_call.as_deref()),
            None => spec
                .phase_call
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(ScriptCallbackTarget::unlinked),
        }
    }

    pub(crate) fn abort_callback_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<ScriptCallbackTarget> {
        let spec = self.spec_for_entry(action, physical_index)?;
        match self.callback_links_for_entry(action, physical_index) {
            Some(links) => links.abort.target(spec.abort_call.as_deref()),
            None => spec
                .abort_call
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(ScriptCallbackTarget::unlinked),
        }
    }

    fn callback_links_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<&ActionCallbackLinks> {
        if let Some(index) = physical_index {
            if self
                .physical
                .get(index as usize)
                .is_some_and(|(name, _)| name == action)
            {
                return self.callback_cache.physical.get(index as usize);
            }
        }
        if !self.physical.is_empty() {
            return self
                .first_physical_index(action)
                .and_then(|index| self.callback_cache.physical.get(index as usize));
        }
        self.callback_cache.named.get(action)
    }

    /// Cache callbacks once for the current link. Physical resource slots
    /// retain native order and duplicate identity; synthetic maps use a
    /// deterministic declared-name order.
    pub(crate) fn link_callbacks(
        &mut self,
        mut resolve: impl FnMut(&str, &'static str, &str) -> Option<ScriptCallbackTarget>,
    ) {
        fn link_spec(
            action: &str,
            spec: &ActionSpec,
            resolve: &mut impl FnMut(&str, &'static str, &str) -> Option<ScriptCallbackTarget>,
        ) -> ActionCallbackLinks {
            let mut links = ActionCallbackLinks::default();
            links.start.set_linked(
                spec.start_call
                    .as_deref()
                    .and_then(|name| resolve(action, "StartCall", name)),
            );
            links.phase.set_linked(
                spec.phase_call
                    .as_deref()
                    .and_then(|name| resolve(action, "PhaseCall", name)),
            );
            links.end.set_linked(
                spec.end_call
                    .as_deref()
                    .and_then(|name| resolve(action, "EndCall", name)),
            );
            links.abort.set_linked(
                spec.abort_call
                    .as_deref()
                    .and_then(|name| resolve(action, "AbortCall", name)),
            );
            links
        }

        let mut callback_cache = ActionCallbackCache::default();
        if self.physical.is_empty() {
            let mut actions = self.declared.iter().cloned().collect::<Vec<_>>();
            actions.sort();
            for action in actions {
                if let Some(spec) = self.specs.get(&action) {
                    callback_cache
                        .named
                        .insert(action.clone(), link_spec(&action, spec, &mut resolve));
                }
            }
            self.callback_cache = callback_cache;
            return;
        }

        for (action, spec) in &self.physical {
            callback_cache
                .physical
                .push(link_spec(action, spec, &mut resolve));
        }
        // Name-based helpers retain the first physical entry. Mirror the
        // exact retained function instead of resolving (or warning) twice.
        let first_links = self
            .first_physical
            .iter()
            .filter_map(|(name, index)| {
                callback_cache
                    .physical
                    .get(*index as usize)
                    .map(|links| (name.clone(), links.clone()))
            })
            .collect::<Vec<_>>();
        for (action, linked) in first_links {
            callback_cache.named.insert(action, linked);
        }
        self.callback_cache = callback_cache;
    }

    pub(crate) fn reset_callback_links(&mut self) {
        self.callback_cache = ActionCallbackCache::default();
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
        let source_action = state.name.clone();
        let source_index = state.act_map_index;
        let mut outcome = self.advance_state_from_entry_by(
            state,
            &source_action,
            source_index,
            phase_advance,
            true,
        );
        self.finish_pending_phase_end(state, &mut outcome);
        outcome
    }

    /// Advance the live action state through the ActMap entry captured at the
    /// start of `C4Object::ExecAction`. `SetDir` may switch the live action via
    /// `TurnAction`, but C++ keeps its old `pAction` pointer for phase handling
    /// through the end of that execution (C4Object.cpp:4794, 5440-5465).
    pub fn advance_state_from_action_by(
        &self,
        state: &mut ActionState,
        source_action: &str,
        phase_advance: i32,
        increment_live_time: bool,
    ) -> ActionAdvanceOutcome {
        let mut outcome = self.advance_state_from_entry_by(
            state,
            source_action,
            None,
            phase_advance,
            increment_live_time,
        );
        self.finish_pending_phase_end(state, &mut outcome);
        outcome
    }

    /// Slot-aware form used by ExecAction after it captures the physical
    /// `pAction`. A TurnAction may replace the live action before phase
    /// advance, so both the original display name and array index are kept.
    pub(crate) fn advance_state_from_entry_by(
        &self,
        state: &mut ActionState,
        source_action: &str,
        source_index: Option<u32>,
        phase_advance: i32,
        increment_live_time: bool,
    ) -> ActionAdvanceOutcome {
        if self.is_idle_entry(source_action, source_index) {
            return ActionAdvanceOutcome::default();
        }
        if let Some(spec) = self.spec_for_entry(source_action, source_index) {
            self.advance_with_spec(
                state,
                source_action,
                source_index,
                spec,
                phase_advance,
                increment_live_time,
            )
        } else {
            ActionAdvanceOutcome::default()
        }
    }

    fn finish_pending_phase_end(
        &self,
        state: &mut ActionState,
        outcome: &mut ActionAdvanceOutcome,
    ) {
        if let Some(phase_end) = outcome.phase_end.take() {
            outcome.wrapped = self.finish_phase_end(state, &phase_end);
        }
    }

    pub(crate) fn finish_phase_end(
        &self,
        state: &mut ActionState,
        phase_end: &ActionPhaseEnd,
    ) -> bool {
        self.finish_phase_end_against(state, phase_end, self)
    }

    /// Finish the stale `pAction` phase-end against the object's live
    /// definition. `C4Object::ExecAction` retains its entry-time action
    /// pointer across `PhaseCall`, while the ensuing `SetAction(int)` reads
    /// the current `Def->ActMap` after a callback-side `ChangeDef`
    /// (C4Object.cpp:5448-5464, 4100-4195).
    pub(crate) fn finish_phase_end_against(
        &self,
        state: &mut ActionState,
        phase_end: &ActionPhaseEnd,
        current_library: &ActionLibrary,
    ) -> bool {
        self.finish_phase_end_against_with_activity(state, phase_end, current_library, true)
    }

    /// Activity-aware form of the phase-end `SetAction(int)`. C++ first
    /// validates the stale action's requested `NextAction` against the live
    /// definition, then coerces an accepted action to `ActIdle` when the
    /// object is incomplete and `IncompleteActivity` is disabled
    /// (C4Object.cpp:4111-4130, 5480-5485).
    pub(crate) fn finish_phase_end_against_with_activity(
        &self,
        state: &mut ActionState,
        phase_end: &ActionPhaseEnd,
        current_library: &ActionLibrary,
        active_action_allowed: bool,
    ) -> bool {
        if state.phase < phase_end.length {
            return false;
        }
        let Some(spec) = self.spec_for_entry(&phase_end.action, phase_end.act_map_index) else {
            return false;
        };
        Self::transition(
            state,
            spec,
            current_library,
            phase_end.length,
            active_action_allowed,
        )
    }

    pub fn procedure_for_action(&self, action: &str) -> ActionProcedure {
        self.specs
            .get(action)
            .and_then(|spec| spec.procedure.as_deref())
            .map(ActionProcedure::from_name)
            .unwrap_or_default()
    }

    pub(crate) fn procedure_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> ActionProcedure {
        self.spec_for_entry(action, physical_index)
            .and_then(|spec| spec.procedure.as_deref())
            .map(ActionProcedure::from_name)
            .unwrap_or_default()
    }

    pub fn procedure_name_for_action(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.procedure.as_deref())
    }

    pub(crate) fn procedure_name_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<&str> {
        self.spec_for_entry(action, physical_index)
            .and_then(|spec| spec.procedure.as_deref())
    }

    pub fn directions_for(&self, action: &str) -> i32 {
        self.specs
            .get(action)
            .and_then(|spec| spec.directions)
            .unwrap_or(1)
    }

    pub(crate) fn directions_for_entry(&self, action: &str, physical_index: Option<u32>) -> i32 {
        self.spec_for_entry(action, physical_index)
            .and_then(|spec| spec.directions)
            .unwrap_or(1)
    }

    /// `C4ActionDef::FlipDir` for the entry SetDir acts on, defaulting to the
    /// C++ zero that never mirrors. Idle objects answer zero because
    /// `UpdateFlipDir` only consults the ActMap above `ActIdle`
    /// (C4Object.cpp:412-415).
    pub(crate) fn flip_dir_for_entry(&self, action: &str, physical_index: Option<u32>) -> i32 {
        if self.is_idle_entry(action, physical_index) {
            return 0;
        }
        self.spec_for_entry(action, physical_index)
            .and_then(|spec| spec.flip_dir)
            .unwrap_or(0)
    }

    pub fn turn_action_for(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.turn_action.as_deref())
    }

    pub(crate) fn turn_action_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<&str> {
        self.spec_for_entry(action, physical_index)
            .and_then(|spec| spec.turn_action.as_deref())
    }

    pub fn in_liquid_action_for(&self, action: &str) -> Option<&str> {
        self.specs
            .get(action)
            .and_then(|spec| spec.in_liquid_action.as_deref())
    }

    pub(crate) fn in_liquid_action_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<&str> {
        self.spec_for_entry(action, physical_index)
            .and_then(|spec| spec.in_liquid_action.as_deref())
    }

    pub fn dig_free_for_action(&self, action: &str) -> Option<i32> {
        self.specs.get(action).and_then(|spec| spec.dig_free)
    }

    pub(crate) fn dig_free_for_entry(
        &self,
        action: &str,
        physical_index: Option<u32>,
    ) -> Option<i32> {
        self.spec_for_entry(action, physical_index)
            .and_then(|spec| spec.dig_free)
    }

    pub fn attach_for_action(&self, action: &str) -> u32 {
        self.specs.get(action).map(|spec| spec.attach).unwrap_or(0)
    }

    pub(crate) fn attach_for_entry(&self, action: &str, physical_index: Option<u32>) -> u32 {
        self.spec_for_entry(action, physical_index)
            .map_or(0, |spec| spec.attach)
    }

    pub(crate) fn is_idle_entry(&self, action: &str, physical_index: Option<u32>) -> bool {
        physical_index.is_none()
            && action == DEFAULT_ACTION_NAME
            && (!self.physical.is_empty() || self.is_idle_action(action))
    }

    /// True for the auto-inserted BARE default "Idle" spec — the C++
    /// `Action.Act <= ActIdle` state (C4Object.cpp:4708). A non-default
    /// synthetic action named "Idle" remains distinguishable here.
    pub fn is_idle_action(&self, action: &str) -> bool {
        action == "Idle"
            && self
                .specs
                .get("Idle")
                .map(|spec| *spec == ActionSpec::default())
                .unwrap_or(true)
    }

    fn advance_with_spec(
        &self,
        state: &mut ActionState,
        source_action: &str,
        source_index: Option<u32>,
        spec: &ActionSpec,
        phase_advance: i32,
        increment_live_time: bool,
    ) -> ActionAdvanceOutcome {
        let mut outcome = ActionAdvanceOutcome::default();

        // Action.Time++ (C4Object.cpp:4745): counts every ExecAction of a
        // real action, independent of the phase machinery below. A different
        // TurnAction already reset the NEW action's time after that increment.
        if increment_live_time {
            state.time = state.time.wrapping_add(1);
        }

        // Phase advance is gated on a nonzero signed Delay — "zero delay
        // means no phase advance" (C4Object.cpp:5463-5468). A negative
        // delay is truthy and the `PhaseDelay >= Delay` comparison succeeds
        // immediately, even when iPhaseAdvance is zero.
        let Some(delay) = spec.delay.filter(|delay| *delay != 0) else {
            return outcome;
        };

        // PhaseDelay += iPhaseAdvance; the phase moves when it reaches
        // Delay and the counter restarts (C4Object.cpp:5443-5447) — a zero
        // advance (standing walker) freezes the animation.
        state.ticks = state.ticks.wrapping_add(phase_advance);
        if state.ticks < delay {
            return outcome;
        }
        state.ticks = 0;

        // C4ActionDef::Step is a signed int32. C++ adds it verbatim: zero
        // keeps the phase fixed and negative values run it backwards.
        let step = spec.step.unwrap_or(1);
        let live_action = state.name.clone();
        let live_act_map_index = state.act_map_index;
        // Phase += Step, then the PhaseCall, then the length check
        // (C4Object.cpp:5448-5464).
        state.phase = state.phase.wrapping_add(step);
        if self
            .phase_callback_for_entry(source_action, source_index)
            .is_some()
        {
            outcome.phase_event = Some(ActionPhaseEvent {
                action: source_action.to_string(),
                act_map_index: source_index,
                live_action,
                live_act_map_index,
                phase: state.phase,
            });
        }
        // Length defaults to 1 (C4Def.h:150).
        let length = spec.length.unwrap_or(1);
        // The caller must run PhaseCall before this length check. Keeping a
        // pending check also lets callback-side SetPhase/SetAction affect the
        // same live Action that C++ examines afterward (C4Object.cpp:5471-5485).
        outcome.phase_end = Some(ActionPhaseEnd {
            action: source_action.to_string(),
            act_map_index: source_index,
            length,
        });

        outcome
    }

    /// Returns true when the NextAction transition ran (false for Hold).
    fn transition(
        state: &mut ActionState,
        spec: &ActionSpec,
        library: &ActionLibrary,
        length: i32,
        active_action_allowed: bool,
    ) -> bool {
        // NextAction=Hold clamps at the last phase and keeps the action
        // (ActHold, C4Def.cpp:786-787; C4Object.cpp:5457-5459).
        if spec.next_index == Some(ACT_HOLD)
            || (spec.next_index.is_none()
                && spec
                    .next
                    .as_deref()
                    .is_some_and(|next| next.eq_ignore_ascii_case("Hold")))
        {
            state.phase = length.wrapping_sub(1);
            return false;
        }
        // An absent NextAction is ActIdle (C4Def.h:154), and an unresolved
        // NextActionName stays ActIdle too (the C4Def::Load mapping loop,
        // C4Def.cpp:784-792) — both go to the literal Idle state, NOT the
        // library's default SPAWN action.
        let (requested, requested_index) = match spec.next_index {
            // SetAction(int) validates the numeric target against the LIVE
            // definition. This matters when a stale PhaseCall pAction came
            // from another definition: an index valid there may be outside
            // the new ActMap and must fail without falling back to ActIdle.
            Some(index) if index >= 0 => {
                let Some((name, _)) = library.physical_entry(index as u32) else {
                    return false;
                };
                (name, Some(index as u32))
            }
            Some(_) => (DEFAULT_ACTION_NAME, None),
            None => spec
                .next
                .as_deref()
                .filter(|next| library.contains(next))
                .map(|next| (next, library.named_action_index(next)))
                .unwrap_or((DEFAULT_ACTION_NAME, None)),
        };
        let current_index = state
            .act_map_index
            .or_else(|| library.named_action_index(&state.name));
        let requested_action_changed = requested != state.name || requested_index != current_index;

        // Phase end calls ordinary non-forced SetAction. The LIVE old slot
        // may differ from the stale pAction after TurnAction/PhaseCall, and
        // its NoOtherAction gate rejects a different numeric target.
        if requested_action_changed
            && library
                .spec_for_entry(&state.name, current_index)
                .is_some_and(|current| current.no_other_action)
        {
            return false;
        }

        // SetAction validates and applies NoOtherAction to the requested
        // numeric slot before this construction gate. The accepted call
        // still returns true and resets Phase/PhaseDelay when coercion leaves
        // an already-idle object in ActIdle.
        let (resolved, resolved_index) = if active_action_allowed {
            (requested, requested_index)
        } else {
            (DEFAULT_ACTION_NAME, None)
        };
        let action_changed = resolved != state.name || resolved_index != current_index;

        if action_changed {
            let source_procedure = library
                .spec_for_entry(&state.name, current_index)
                .and_then(|current| current.procedure.as_deref())
                .map(ActionProcedure::from_name)
                .unwrap_or_default();
            let target_procedure = library
                .spec_for_entry(resolved, resolved_index)
                .and_then(|target| target.procedure.as_deref())
                .map(ActionProcedure::from_name)
                .unwrap_or_default();
            if source_procedure != target_procedure {
                state.data = 0;
            }
            state.name = resolved.to_string();
            state.act_map_index = resolved_index;
            state.raw_name = None;
            // Action.Time resets on the action CHANGE only
            // (C4Object.cpp:4106-4108); a self-chain keeps counting.
            state.time = 0;
        } else {
            state.act_map_index = resolved_index;
            state.raw_name = None;
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
    pub(crate) phase_end: Option<ActionPhaseEnd>,
    /// The phase-end NextAction transition ran (C4Object.cpp:5480-5485) —
    /// the caller owes a StartCall+EndCall pair even for a same-name chain.
    pub wrapped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionPhaseEnd {
    /// Stale physical pAction captured at ExecAction entry.
    pub action: String,
    pub act_map_index: Option<u32>,
    pub length: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPhaseEvent {
    /// ActMap entry whose PhaseCall C++ invokes through the stale `pAction`.
    pub action: String,
    /// Physical slot of the stale `pAction` captured by ExecAction.
    pub act_map_index: Option<u32>,
    /// Live action name visible to that callback after an earlier TurnAction.
    pub live_action: String,
    pub live_act_map_index: Option<u32>,
    pub phase: i32,
}

/// Minimal representation of an object's current action state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionState {
    pub name: String,
    /// Raw `C4Action::Name` when it differs from the numeric action. This is
    /// observable through `GetObjectVal("Action")` and survives a re-save:
    /// a failed loaded `SetActionByName` leaves `ActIdle` selected while the
    /// compiled, unresolved name remains in this buffer (C4Object.cpp:
    /// 2867-2877, C4Action.cpp:45-54).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_name: Option<String>,
    /// Physical C4ActionDef array slot for resource-backed ActMaps. Script
    /// names are not unique, so this identity must survive a NextAction
    /// transition to a later duplicate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act_map_index: Option<u32>,
    pub phase: i32,
    /// `Action.PhaseDelay` (C4Object.cpp:5443-5447): the intra-phase
    /// counter, restarting every phase advance.
    #[serde(default)]
    pub ticks: i32,
    /// `Action.Time` (C4Object.cpp:4745): total frames in the current
    /// action, reset only when the action CHANGES (C4Object.cpp:4106-4108).
    /// GetActTime reads this (C4Script.cpp).
    #[serde(default)]
    pub time: i32,
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
            raw_name: None,
            act_map_index: None,
            phase: 0,
            ticks: 0,
            time: 0,
            data: 0,
            target: None,
            target2: None,
        }
    }

    /// The fixed-size `C4Action::Name` buffer, distinct from script
    /// `GetAction()`. Numeric ActIdle stores an empty buffer; a physical slot
    /// named Idle/ActIdle stores its real name, and a failed loaded lookup
    /// retains the unresolved compiler text.
    pub(crate) fn compiled_name(&self) -> &str {
        self.raw_name.as_deref().unwrap_or_else(|| {
            if self.act_map_index.is_none() && is_builtin_idle_name(&self.name) {
                ""
            } else {
                &self.name
            }
        })
    }

    pub fn advance(&mut self) {
        self.phase = self.phase.wrapping_add(1);
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
            self.raw_name = None;
            // Name-based updates mirror SetActionByName. Without an
            // ActionLibrary this can only discard a previously retained
            // duplicate-slot identity; the library-aware path restores the
            // first matching slot below.
            self.act_map_index = None;
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
        if resolved.name.as_deref().is_some_and(is_builtin_idle_name) {
            resolved.name = Some(DEFAULT_ACTION_NAME.to_string());
        }
        if let Some(name) = resolved.name.as_ref() {
            let current_index = self
                .act_map_index
                .or_else(|| library.named_action_index(&self.name));
            let requested_index = library.named_action_index(name);
            if !resolved.force
                && library
                    .spec_for_state(self)
                    .is_some_and(|spec| spec.no_other_action)
                && (name != &self.name || requested_index != current_index)
            {
                return ActionUpdateResult::Blocked;
            }
        }
        if let Some(name) = resolved.name.as_ref() {
            // ActIdle is a built-in action slot before ActMap, so it remains
            // a valid target when a fixture library has a different default
            // and no explicit "Idle" entry.
            if name != DEFAULT_ACTION_NAME && !library.contains(name) {
                resolved.name = Some(library.default_action().to_string());
                resolved.phase = Some(0);
                resolved.ticks = Some(0);
            }
        }

        let previous_name = self.name.clone();
        let previous_index = self.act_map_index;
        let previous_procedure = library
            .spec_for_state(self)
            .and_then(|spec| spec.procedure.as_deref())
            .map(ActionProcedure::from_name)
            .unwrap_or_default();

        let requested_index = resolved
            .name
            .as_deref()
            .and_then(|name| library.named_action_index(name));
        let previous_resolved_index =
            previous_index.or_else(|| library.named_action_index(previous_name.as_str()));
        let entry_changed = resolved.name.as_ref().is_some_and(|name| {
            name != &previous_name || requested_index != previous_resolved_index
        });
        if entry_changed {
            self.phase = 0;
            self.ticks = 0;
            self.time = 0;
        }
        self.apply_update(&resolved);
        if resolved.name.is_some() {
            self.act_map_index = requested_index;
            self.raw_name = None;
        }

        let next_name = self.name.clone();
        let next_procedure = library
            .spec_for_state(self)
            .and_then(|spec| spec.procedure.as_deref())
            .map(ActionProcedure::from_name)
            .unwrap_or_default();
        if entry_changed && previous_procedure != next_procedure && resolved.data.is_none() {
            self.data = 0;
        }
        self.reconcile_with_library(library);
        ActionUpdateResult::Applied
    }

    /// Apply the post-compile action restoration from `C4Object::CompileFunc`.
    /// The object starts at numeric `ActIdle`; a successful name lookup runs
    /// `SetAction`, then the saved Time/Phase/PhaseDelay values are restored.
    /// `Action.Data` is cleared only when that transition enters a non-NONE
    /// procedure. A failed lookup keeps the raw compiled name beside numeric
    /// `ActIdle` (C4Object.cpp:2867-2877,4102-4146).
    pub fn restore_loaded_with_library(
        &mut self,
        library: &ActionLibrary,
        active_action_allowed: bool,
    ) {
        let compiled_name = self.name.clone();
        self.name = DEFAULT_ACTION_NAME.to_string();
        self.act_map_index = None;

        if is_builtin_idle_name(&compiled_name) {
            // SetAction(ActIdle) clears the fixed-size Name buffer.
            self.raw_name = None;
            return;
        }

        let resolved_index = library.named_action_index(&compiled_name);
        if resolved_index.is_some()
            || (library.physical.is_empty() && library.contains(&compiled_name))
        {
            // A resolved name still enters SetAction, which coerces partial
            // objects back to ActIdle unless IncompleteActivity is enabled.
            // Because old and new numeric procedures are both DFA_NONE,
            // Action.Data survives; CompileFunc restores the three saved
            // counters after SetAction clears the Name buffer.
            if !active_action_allowed {
                self.raw_name = None;
                return;
            }
            self.name = compiled_name;
            self.act_map_index = resolved_index;
            self.raw_name = None;
            if library.procedure_for_entry(&self.name, self.act_map_index)
                != ActionProcedure::Undefined
            {
                self.data = 0;
            }
        } else {
            self.raw_name = Some(compiled_name);
        }
    }

    pub fn reconcile_with_library(&mut self, library: &ActionLibrary) {
        // Saved Action= is restored through SetActionByName. Both spellings
        // select the built-in ActIdle sentinel before the physical ActMap is
        // scanned (C4Object.cpp:4211-4216). A numerically reached physical
        // slot named ActIdle retains its explicit index and remains real.
        if self.act_map_index.is_none() && is_builtin_idle_name(&self.name) {
            self.name = DEFAULT_ACTION_NAME.to_string();
        }
        if self.name != DEFAULT_ACTION_NAME && !library.contains(&self.name) {
            self.name = library.default_action().to_string();
            // The synthetic SpawnConfig/action-update seam resolves an
            // unknown requested action to its configured default as an
            // action change. C4Object::SetAction clears Phase and
            // PhaseDelay on every successful selection (C4Object.cpp:
            // 4131-4132); preserve that established fixture contract here.
            self.phase = 0;
            self.ticks = 0;
        }
        self.act_map_index = self
            .act_map_index
            .filter(|index| {
                library
                    .physical_entry(*index)
                    .is_some_and(|(name, _)| name == self.name)
            })
            .or_else(|| library.named_action_index(&self.name));
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
    pub ticks: Option<i32>,
    #[serde(default = "ActionUpdate::default_force")]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Option<ObjectId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target2: Option<Option<ObjectId>>,
    /// The script SetAction seam already ran StartCall/AbortCall
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

    pub fn with_ticks(mut self, ticks: i32) -> Self {
        self.ticks = Some(ticks);
        self
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = Some(name.into());
    }

    pub fn set_phase(&mut self, phase: i32) {
        self.phase = Some(phase);
    }

    pub fn set_ticks(&mut self, ticks: i32) {
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
            self.callbacks_dispatched = other.callbacks_dispatched;
            self.name = other.name;
        } else {
            // Phase/data/target writes belong to the current action change and
            // must not erase its already-dispatched callback marker.
            self.callbacks_dispatched |= other.callbacks_dispatched;
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
    fn action_library_remains_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ActionLibrary>();
    }

    #[test]
    fn c4_action_reflection_covers_the_complete_compiler_table() {
        let reflection =
            C4ActionReflection::from_resource("Idle", &ResourceActionDefinition::default());
        let actual = reflection
            .entry_names()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let expected = [
            "Name",
            "Procedure",
            "Directions",
            "FlipDir",
            "Length",
            "Attach",
            "Delay",
            "Facet",
            "FacetBase",
            "FacetTopFace",
            "FacetTargetStretch",
            "NextAction",
            "NoOtherAction",
            "StartCall",
            "EndCall",
            "AbortCall",
            "PhaseCall",
            "Sound",
            "ObjectDisabled",
            "DigFree",
            "EnergyUsage",
            "InLiquidAction",
            "TurnAction",
            "Reverse",
            "Step",
        ]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
        assert_eq!(actual, expected);
        assert_eq!(
            reflection.get("Name", 0),
            Some(Value::String("Idle".into()))
        );
        assert_eq!(
            reflection.get("Sound", 0),
            Some(Value::String(String::new().into()))
        );
        assert_eq!(reflection.get("Directions", 0), Some(Value::Int(1)));
        assert_eq!(reflection.get("Step", 0), Some(Value::Int(1)));
        for index in 0..=5 {
            assert_eq!(reflection.get("Facet", index), Some(Value::Int(0)));
        }
        assert_eq!(reflection.get("Facet", 6), None);
        assert_eq!(reflection.get("Length", 1), None);
        assert_eq!(reflection.get("Length", -1), None);
        assert_eq!(reflection.get("Unknown", 0), None);
    }

    #[test]
    fn loaded_idle_preserves_saved_counters_like_cpp() {
        // Load first enters ActIdle through SetActionByName, then restores
        // saved Time/Phase/PhaseDelay without checking the selected action
        // (C4Object.cpp:2862-2876).
        let library = ActionLibrary::default();
        let mut state = ActionState::new("Idle");
        state.phase = 1;
        state.ticks = 3;
        state.time = 4;
        state.reconcile_with_library(&library);
        assert_eq!((state.phase, state.ticks, state.time), (1, 3, 4));
    }

    fn library_with(specs: Vec<(&str, ActionSpec)>) -> ActionLibrary {
        let map: std::collections::HashMap<String, ActionSpec> = specs
            .into_iter()
            .map(|(name, spec)| (name.to_string(), spec))
            .collect();
        ActionLibrary::new(None, map)
    }

    #[test]
    fn next_action_keeps_the_last_duplicate_slot_while_name_lookup_uses_first() {
        let source = ActionSpec::default()
            .with_length(1)
            .with_delay(1)
            .with_next("Dup")
            .with_next_index(2);
        let first_dup = ActionSpec::default()
            .with_length(2)
            .with_delay(1)
            .with_next("Hold")
            .with_next_index(ACT_HOLD);
        let last_dup = ActionSpec::default()
            .with_length(5)
            .with_delay(1)
            .with_next("Hold")
            .with_next_index(ACT_HOLD);
        let mut library = ActionLibrary::new(
            None,
            HashMap::from([
                ("Source".to_string(), source.clone()),
                ("Dup".to_string(), first_dup.clone()),
            ]),
        );
        library.set_physical_actions(vec![
            ("Source".to_string(), source),
            ("Dup".to_string(), first_dup),
            ("Dup".to_string(), last_dup),
        ]);

        let mut state = ActionState::new("Source");
        state.reconcile_with_library(&library);
        library.advance_state(&mut state);
        assert_eq!(state.name, "Dup");
        assert_eq!(
            state.act_map_index,
            Some(2),
            "CrossMap target is physical slot 2"
        );

        for _ in 0..3 {
            library.advance_state(&mut state);
        }
        assert_eq!(
            state.phase, 3,
            "later frames keep using the last duplicate's Length=5"
        );

        state.apply_update_with_library(&ActionUpdate::default().with_name("Dup"), &library);
        assert_eq!(
            state.act_map_index,
            Some(1),
            "SetActionByName scans from slot zero"
        );
        assert_eq!(
            state.phase, 0,
            "switching duplicate slots is a real action change"
        );
    }

    #[test]
    fn builtin_idle_stays_distinct_from_a_physical_action_named_idle() {
        let source = ActionSpec::default()
            .with_length(1)
            .with_delay(1)
            .with_next_index(1);
        let physical_idle = ActionSpec::default()
            .with_procedure("ATTACH")
            .with_length(3)
            .with_delay(1)
            .with_next_index(ACT_HOLD);
        let mut library = ActionLibrary::new(
            None,
            HashMap::from([
                ("Source".to_string(), source.clone()),
                ("Idle".to_string(), physical_idle.clone()),
            ]),
        );
        library.set_physical_actions(vec![
            ("Source".to_string(), source),
            ("Idle".to_string(), physical_idle),
        ]);

        let mut builtin = ActionState::new("Idle");
        builtin.reconcile_with_library(&library);
        assert_eq!(builtin.act_map_index, None);
        assert!(library.is_idle_state(&builtin));

        let mut physical = ActionState {
            act_map_index: Some(0),
            ..ActionState::new("Source")
        };
        library.advance_state(&mut physical);
        assert_eq!(physical.name, "Idle");
        assert_eq!(physical.act_map_index, Some(1));
        assert!(!library.is_idle_state(&physical));
        assert_eq!(
            library.procedure_for_entry(&physical.name, physical.act_map_index),
            ActionProcedure::Attach
        );
    }

    #[test]
    fn builtin_idle_does_not_advance_time_or_phase_like_cpp() {
        let library = ActionLibrary::new(None, HashMap::new());
        let mut state = ActionState::new("ActIdle");
        state.phase = 4;
        state.ticks = 7;
        state.time = 9;
        state.reconcile_with_library(&library);

        let outcome = library.advance_state(&mut state);

        assert_eq!(state.name, DEFAULT_ACTION_NAME);
        assert_eq!((state.phase, state.ticks, state.time), (4, 7, 9));
        assert_eq!(outcome, ActionAdvanceOutcome::default());
    }

    #[test]
    fn actidle_name_selects_builtin_but_numeric_slot_stays_physical() {
        let source = ActionSpec::default()
            .with_length(1)
            .with_delay(1)
            .with_next_index(1);
        let physical_act_idle = ActionSpec::default()
            .with_procedure("ATTACH")
            .with_length(3)
            .with_delay(1)
            .with_next_index(ACT_HOLD);
        let mut library = ActionLibrary::new(
            None,
            HashMap::from([
                ("Source".to_string(), source.clone()),
                ("ActIdle".to_string(), physical_act_idle.clone()),
            ]),
        );
        library.set_physical_actions(vec![
            ("Source".to_string(), source),
            ("ActIdle".to_string(), physical_act_idle),
        ]);

        let mut named = ActionState::new("ActIdle");
        named.phase = 2;
        named.ticks = 3;
        named.reconcile_with_library(&library);
        assert_eq!(named.name, DEFAULT_ACTION_NAME);
        assert_eq!(named.act_map_index, None);
        assert_eq!((named.phase, named.ticks), (2, 3));

        let mut numeric = ActionState {
            act_map_index: Some(0),
            ..ActionState::new("Source")
        };
        library.advance_state(&mut numeric);
        assert_eq!(numeric.name, "ActIdle");
        assert_eq!(numeric.act_map_index, Some(1));
        assert_eq!(
            library.procedure_for_entry(&numeric.name, numeric.act_map_index),
            ActionProcedure::Attach
        );
        assert!(!library.is_idle_state(&numeric));
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

    // C++ tests signed Delay for nonzero, then compares the signed
    // PhaseDelay against it (C4Object.cpp:5464-5468). A negative Delay
    // therefore advances immediately, even when a standing WALK supplies
    // iPhaseAdvance=0.
    #[test]
    fn negative_delay_advances_even_with_zero_phase_weight_like_cpp() {
        let library = library_with(vec![(
            "Odd",
            ActionSpec::default()
                .with_length(3)
                .with_delay(-7)
                .with_next("Hold"),
        )]);
        let mut state = ActionState::new("Odd");

        state.advance_with_library_by(&library, 0);

        assert_eq!(state.phase, 1);
        assert_eq!(
            state.ticks, 0,
            "the successful comparison resets PhaseDelay"
        );
    }

    #[test]
    fn zero_and_negative_steps_are_added_verbatim_like_cpp() {
        let frozen = library_with(vec![(
            "Frozen",
            ActionSpec::default()
                .with_length(2)
                .with_delay(1)
                .with_step(0)
                .with_next("Hold"),
        )]);
        let mut frozen_state = ActionState::new("Frozen");
        for _ in 0..4 {
            frozen.advance_state(&mut frozen_state);
        }
        assert_eq!(
            frozen_state.phase, 0,
            "Step=0 must not be normalized to one"
        );

        let backwards = library_with(vec![(
            "Backwards",
            ActionSpec::default()
                .with_length(2)
                .with_delay(1)
                .with_step(-2)
                .with_next("Hold"),
        )]);
        let mut backwards_state = ActionState::new("Backwards");
        for _ in 0..3 {
            backwards.advance_state(&mut backwards_state);
        }
        assert_eq!(
            backwards_state.phase, -6,
            "negative Step runs the phase backwards"
        );
    }

    #[test]
    fn nonpositive_length_uses_signed_end_check_and_hold_clamp_like_cpp() {
        for (length, expected_phase) in [(0, -1), (-3, -4)] {
            let library = library_with(vec![(
                "Odd",
                ActionSpec::default()
                    .with_length(length)
                    .with_delay(1)
                    .with_next("Hold"),
            )]);
            let mut state = ActionState::new("Odd");

            library.advance_state(&mut state);

            assert_eq!(state.name, "Odd");
            assert_eq!(state.phase, expected_phase, "Length={length}");
        }
    }

    #[test]
    fn signed_directions_are_retained_for_the_set_dir_gate() {
        let library = library_with(vec![("Odd", ActionSpec::default().with_directions(-2))]);
        assert_eq!(library.directions_for("Odd"), -2);
    }

    #[test]
    fn phase_call_precedes_and_can_suppress_the_length_transition() {
        let library = library_with(vec![
            (
                "Loop",
                ActionSpec::default()
                    .with_length(1)
                    .with_delay(1)
                    .with_phase_call("OnPhase")
                    .with_next("Done"),
            ),
            ("Done", ActionSpec::default()),
        ]);
        let mut state = ActionState::new("Loop");
        let mut outcome = library.advance_state_from_entry_by(&mut state, "Loop", None, 1, true);

        assert_eq!(state.name, "Loop", "NextAction waits for PhaseCall");
        assert_eq!(state.phase, 1);
        assert!(outcome.phase_event.is_some());
        state.phase = 0; // what a synchronous PhaseCall SetPhase(0) does
        let phase_end = outcome.phase_end.take().expect("length check is pending");
        assert!(!library.finish_phase_end(&mut state, &phase_end));
        assert_eq!(state.name, "Loop");
    }

    #[test]
    fn natural_next_action_honors_live_no_other_action() {
        let library = library_with(vec![
            (
                "Locked",
                ActionSpec::default()
                    .with_length(1)
                    .with_delay(1)
                    .with_next("Other")
                    .with_no_other_action(true),
            ),
            ("Other", ActionSpec::default()),
        ]);
        let mut state = ActionState::new("Locked");

        let outcome = library.advance_state(&mut state);

        assert!(!outcome.wrapped);
        assert_eq!(state.name, "Locked");
        assert_eq!(state.phase, 1, "rejected SetAction leaves the old phase");
    }

    #[test]
    fn stale_phase_end_resolves_and_validates_numeric_target_in_live_library() {
        let stale_source = ActionSpec::default()
            .with_length(1)
            .with_delay(1)
            .with_next_index(1);
        let mut stale_library = ActionLibrary::new(
            None,
            HashMap::from([("Source".to_string(), stale_source.clone())]),
        );
        stale_library.set_physical_actions(vec![("Source".to_string(), stale_source)]);
        let phase_end = ActionPhaseEnd {
            action: "Source".to_string(),
            act_map_index: Some(0),
            length: 1,
        };

        let first = ActionSpec::default().with_procedure("WALK");
        let second = ActionSpec::default().with_procedure("FLIGHT");
        let mut live_library = ActionLibrary::new(
            None,
            HashMap::from([
                ("NewZero".to_string(), first.clone()),
                ("NewOne".to_string(), second.clone()),
            ]),
        );
        live_library.set_physical_actions(vec![
            ("NewZero".to_string(), first),
            ("NewOne".to_string(), second),
        ]);

        let mut state = ActionState::new("Idle");
        state.phase = 1;
        state.data = 77;
        assert!(stale_library.finish_phase_end_against(&mut state, &phase_end, &live_library,));
        assert_eq!(
            (state.name.as_str(), state.act_map_index),
            ("NewOne", Some(1))
        );
        assert_eq!(state.data, 0, "the live target procedure owns Data reset");

        let only = ActionSpec::default();
        let mut short_library =
            ActionLibrary::new(None, HashMap::from([("Only".to_string(), only.clone())]));
        short_library.set_physical_actions(vec![("Only".to_string(), only)]);
        let mut invalid = ActionState::new("Idle");
        invalid.phase = 1;
        assert!(!stale_library.finish_phase_end_against(&mut invalid, &phase_end, &short_library,));
        assert_eq!(invalid.name, "Idle", "invalid SetAction(int) is a no-op");
        assert_eq!(invalid.phase, 1);

        let locked = ActionSpec::default().with_no_other_action(true);
        let other = ActionSpec::default();
        let mut locked_library = ActionLibrary::new(
            None,
            HashMap::from([
                ("Locked".to_string(), locked.clone()),
                ("Other".to_string(), other.clone()),
            ]),
        );
        locked_library.set_physical_actions(vec![
            ("Locked".to_string(), locked),
            ("Other".to_string(), other),
        ]);
        let mut blocked = ActionState {
            act_map_index: Some(0),
            phase: 1,
            ..ActionState::new("Locked")
        };
        assert!(!stale_library.finish_phase_end_against(&mut blocked, &phase_end, &locked_library,));
        assert_eq!(blocked.name, "Locked", "live NoOtherAction owns the gate");
        assert_eq!(blocked.phase, 1);
    }

    #[test]
    fn phase_end_rechecks_live_activity_after_the_phase_callback() {
        let source = ActionSpec::default()
            .with_length(1)
            .with_delay(1)
            .with_phase_call("OnPhase")
            .with_next_index(1);
        let target = ActionSpec::default().with_procedure("WALK");
        let mut library = ActionLibrary::new(
            None,
            HashMap::from([
                ("Source".to_string(), source.clone()),
                ("Target".to_string(), target.clone()),
            ]),
        );
        library.set_physical_actions(vec![
            ("Source".to_string(), source),
            ("Target".to_string(), target),
        ]);
        let mut state = ActionState::new("Source");
        state.act_map_index = Some(0);
        let mut outcome =
            library.advance_state_from_entry_by(&mut state, "Source", Some(0), 1, true);

        assert!(outcome.phase_event.is_some(), "PhaseCall is now due");
        let phase_end = outcome.phase_end.take().expect("phase end is pending");

        // This boolean is computed from live Con + IncompleteActivity only
        // after the synchronous PhaseCall returns. Simulate that callback
        // making active actions illegal without invoking DoCon (which has
        // its own immediate SetAction(ActIdle) side effect).
        assert!(library
            .finish_phase_end_against_with_activity(&mut state, &phase_end, &library, false,));
        assert_eq!((state.name.as_str(), state.act_map_index), ("Idle", None));
        assert_eq!(state.phase, 0);
        assert_eq!(state.ticks, 0);
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

    #[test]
    fn merge_preserves_synchronously_dispatched_set_action_callbacks() {
        let mut accumulated = ActionUpdate::default().with_phase(7);
        accumulated.merge(ActionUpdate {
            name: Some("Sit".to_string()),
            callbacks_dispatched: true,
            ..ActionUpdate::default()
        });

        assert!(
            accumulated.callbacks_dispatched,
            "the outcome fold must not replay SetAction's synchronous callbacks"
        );
    }

    #[test]
    fn merge_carries_the_dispatch_state_of_a_replacing_action_name() {
        let mut accumulated = ActionUpdate {
            name: Some("Sit".to_string()),
            callbacks_dispatched: true,
            ..ActionUpdate::default()
        };
        accumulated.merge(ActionUpdate {
            name: Some("Walk".to_string()),
            callbacks_dispatched: false,
            ..ActionUpdate::default()
        });

        assert!(!accumulated.callbacks_dispatched);
    }
}
