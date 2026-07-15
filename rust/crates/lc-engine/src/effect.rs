use lc_script::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EffectVarValue {
    Int(i32),
    Bool(bool),
    String(String),
    /// A definition identifier is a distinct C4Value type from String.
    /// EffectVar must preserve that distinction because scripts may use a
    /// stored id as an object-call target (`idMagic->~Callback()`).
    C4Id(String),
    Object(u64),
    Array(Vec<EffectVarValue>),
    Proplist(
        #[serde(with = "effect_var_map_serde")]
        Vec<(Value, EffectVarValue)>,
    ),
    #[default]
    Nil,
}

/// Keep the established JSON object representation for string-only effect
/// maps, while retaining arbitrary C4Value keys in the general case. The
/// sequence representation is unambiguous and preserves C4ValueHash key order.
mod effect_var_map_serde {
    use indexmap::IndexMap;
    use lc_script::Value;
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::EffectVarValue;

    pub fn serialize<S>(
        entries: &[(Value, EffectVarValue)],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if entries
            .iter()
            .all(|(key, _)| matches!(key, Value::String(_)))
        {
            let mut map = serializer.serialize_map(Some(entries.len()))?;
            for (key, value) in entries {
                let Value::String(key) = key else {
                    unreachable!("all keys were checked as strings");
                };
                map.serialize_entry(key, value)?;
            }
            map.end()
        } else {
            entries.serialize(serializer)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<(Value, EffectVarValue)>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Legacy(IndexMap<String, EffectVarValue>),
            Entries(Vec<(Value, EffectVarValue)>),
        }

        Ok(match Repr::deserialize(deserializer)? {
            Repr::Legacy(entries) => entries
                .into_iter()
                .map(|(key, value)| (Value::String(key), value))
                .collect(),
            Repr::Entries(entries) => entries,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectState {
    /// C4Effect::iNumber (C4Effect.cpp:76-78): per-object monotonic
    /// (max existing + 1). Zero = not yet allocated.
    #[serde(default)]
    pub number: i32,
    pub name: String,
    pub priority: i32,
    pub interval: i32,
    pub timer: i32,
    #[serde(default)]
    pub command_target: Option<i32>,
    #[serde(default)]
    pub command_id: Option<String>,
    #[serde(default)]
    pub vars: Vec<EffectVarValue>,
    /// True when Fx*Start already ran synchronously inside FnAddEffect
    /// (C4Effect ctor semantics, C4Effect.cpp:96-152) — the deferred
    /// Started event must not dispatch it again.
    #[serde(default)]
    pub start_dispatched: bool,
}

impl EffectState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            number: 0,
            name: name.into(),
            priority: 100,
            interval: 0,
            timer: 0,
            command_target: None,
            command_id: None,
            vars: Vec::new(),
            start_dispatched: false,
        }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// `iIntervall` is stored verbatim (C4Effect.cpp:67) — zero means the
    /// timer never fires.
    pub fn with_interval(mut self, interval: i32) -> Self {
        self.interval = interval.max(0);
        self
    }

    /// `iTime` is stored verbatim — C++ persists it monotonically
    /// (C4Effect.cpp:523).
    pub fn with_timer(mut self, timer: i32) -> Self {
        self.timer = timer.max(0);
        self
    }

    pub fn with_command_target(mut self, target: Option<i32>) -> Self {
        self.command_target = target;
        self
    }

    pub fn with_command_id<I>(mut self, id: Option<I>) -> Self
    where
        I: Into<String>,
    {
        self.command_id = id.map(|value| value.into());
        self
    }

    pub fn with_vars(mut self, vars: Vec<EffectVarValue>) -> Self {
        self.vars = vars;
        self
    }

    /// One frame of effect time (C4Effect.cpp:339-342): `iTime` increments
    /// every frame and is never reset (script-visible via the Fx*Timer
    /// `iTime` argument); the timer fires when `iIntervall != 0` and
    /// `iTime % iIntervall == 0` — a zero interval never fires.
    pub fn advance_tick(&mut self) -> bool {
        self.timer = self.timer.saturating_add(1);
        self.interval > 0 && self.timer % self.interval == 0
    }

    pub fn set_var(&mut self, index: usize, value: EffectVarValue) {
        if self.vars.len() <= index {
            self.vars.resize(index + 1, EffectVarValue::default());
        }
        self.vars[index] = value;
    }

    pub fn var(&self, index: usize) -> EffectVarValue {
        self.vars
            .get(index)
            .cloned()
            .unwrap_or_else(EffectVarValue::default)
    }

    pub fn vars(&self) -> &[EffectVarValue] {
        &self.vars
    }
}

#[cfg(test)]
mod map_serde_tests {
    use super::*;

    #[test]
    fn effect_var_string_map_keeps_legacy_json_shape() {
        let value = EffectVarValue::Proplist(vec![
            (Value::String("beta".into()), EffectVarValue::Int(2)),
            (Value::String("alpha".into()), EffectVarValue::Int(1)),
        ]);

        let encoded = serde_json::to_string(&value).expect("effect map serializes");
        assert_eq!(
            encoded,
            r#"{"Proplist":{"beta":{"Int":2},"alpha":{"Int":1}}}"#
        );
        let decoded: EffectVarValue =
            serde_json::from_str(&encoded).expect("legacy effect map deserializes");
        assert_eq!(decoded, value);
    }

    #[test]
    fn effect_var_map_round_trips_arbitrary_keys_in_order() {
        let value = EffectVarValue::Proplist(vec![
            (Value::Int(7), EffectVarValue::String("seven".into())),
            (Value::Bool(true), EffectVarValue::Object(42)),
        ]);

        let encoded = serde_json::to_value(&value).expect("effect map serializes");
        assert!(encoded["Proplist"].is_array());
        let decoded: EffectVarValue =
            serde_json::from_value(encoded).expect("effect map deserializes");
        assert_eq!(decoded, value);
    }
}

impl Default for EffectState {
    fn default() -> Self {
        Self::new("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectCommand {
    Add(EffectState),
    /// Number-keyed in-place update (EffectVar writes). Folding an
    /// Update whose number no longer exists is a NO-OP — a var write
    /// must never resurrect an effect the timer killed in the same
    /// frame (C4Effect vars live inside the effect; death is final).
    Update(EffectState),
    /// Marks the selected live effect dead. C4Effect::Kill/SetDead leaves
    /// the node linked at priority zero until the next Execute walk.
    Remove { name: String, no_callbacks: bool },
    /// Identity-keyed death for callers that address an effect by its C++
    /// `iNumber`. Names are not unique, so folding this as `Remove { name }`
    /// can target the wrong same-name peer.
    RemoveNumber { number: i32, no_callbacks: bool },
    /// Immediate structural unlink for constructor exception unwind and
    /// final object-list destruction. Ordinary effect removal must use one
    /// of the dead-marking variants above.
    UnlinkNumber { number: i32 },
    Clear,
}

impl EffectCommand {
    pub fn add(effect: EffectState) -> Self {
        Self::Add(effect)
    }

    pub fn update(effect: EffectState) -> Self {
        Self::Update(effect)
    }

    pub fn remove(name: impl Into<String>) -> Self {
        Self::Remove {
            name: name.into(),
            no_callbacks: false,
        }
    }

    pub fn remove_without_callbacks(name: impl Into<String>) -> Self {
        Self::Remove {
            name: name.into(),
            no_callbacks: true,
        }
    }

    pub fn remove_number(number: i32, no_callbacks: bool) -> Self {
        Self::RemoveNumber {
            number,
            no_callbacks,
        }
    }

    pub fn unlink_number(number: i32) -> Self {
        Self::UnlinkNumber { number }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectStopReason {
    Removed,
    Cleared,
    /// C4FxCall_RemoveDeath (C4Effects.h:50): AssignDeath clears the
    /// pre-existing effect chain with reason 4, and Stop may return -1 to
    /// survive that clear (the Reincarnation spell relies on both).
    Death,
    Destroyed,
    Replaced,
    /// Temporary deactivation (C4FxCall_Temp, C4Effects.h:47): the effect
    /// is NOT removed — Fx*Stop runs with fTemp = true
    /// (TempRemoveUpperEffects, C4Effect.cpp:489).
    Temp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectEventKind {
    Started,
    Timer,
    Stopped(EffectStopReason),
    /// `C4Effect::Check` (C4Effect.cpp:271-317): the carried effect is the
    /// CHECKER asked about a pending new effect (whose full state rides
    /// along so the add-to-other-effect merge can hand its parameters to
    /// `Fx*Add`, C4Effect.cpp:300-301).
    Check {
        pending: EffectState,
    },
    /// Add-to-other-effect merge (C4Effect.cpp:295-313): the carried
    /// effect is the ACCEPTOR whose `Fx<Name>Add` receives the annulled
    /// pending effect's parameters.
    AddTo {
        pending: EffectState,
    },
    /// Temporary deactivation of an upper effect (TempRemoveUpperEffects,
    /// C4Effect.cpp:473-492): Fx*Stop(C4FxCall_Temp, fTemp = true); the
    /// effect stays in the list.
    TempRemoved,
    /// Reactivation of a temp-removed upper effect
    /// (TempReaddUpperEffects, C4Effect.cpp:494-510):
    /// Fx*Start(C4FxCall_Temp).
    TempReadded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEvent {
    pub effect: EffectState,
    pub kind: EffectEventKind,
}

impl EffectEvent {
    pub fn started(effect: EffectState) -> Self {
        Self {
            effect,
            kind: EffectEventKind::Started,
        }
    }

    pub fn timer(effect: EffectState) -> Self {
        Self {
            effect,
            kind: EffectEventKind::Timer,
        }
    }

    pub fn check(checker: EffectState, pending: EffectState) -> Self {
        Self {
            effect: checker,
            kind: EffectEventKind::Check { pending },
        }
    }

    pub fn add_to(acceptor: EffectState, pending: EffectState) -> Self {
        Self {
            effect: acceptor,
            kind: EffectEventKind::AddTo { pending },
        }
    }

    pub fn temp_removed(effect: EffectState) -> Self {
        Self {
            effect,
            kind: EffectEventKind::TempRemoved,
        }
    }

    pub fn temp_readded(effect: EffectState) -> Self {
        Self {
            effect,
            kind: EffectEventKind::TempReadded,
        }
    }

    pub fn stopped(effect: EffectState, reason: EffectStopReason) -> Self {
        Self {
            effect,
            kind: EffectEventKind::Stopped(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_timer_follows_cpp_modulo_semantics() {
        // C4Effect::Execute (C4Effect.cpp:339-357): iTime increments every
        // frame and is never reset (script-visible via the Fx*Timer iTime
        // argument); the timer fires when iIntervall != 0 and
        // iTime % iIntervall == 0; iIntervall == 0 NEVER fires.
        let mut effect = EffectState::new("Glow").with_interval(3);
        let fired: Vec<bool> = (0..7).map(|_| effect.advance_tick()).collect();
        assert_eq!(
            fired,
            vec![false, false, true, false, false, true, false],
            "fires on iTime 3 and 6"
        );
        assert_eq!(effect.timer, 7, "iTime is monotonic");

        let mut inert = EffectState::new("Inert").with_interval(0);
        let fired: Vec<bool> = (0..5).map(|_| inert.advance_tick()).collect();
        assert_eq!(fired, vec![false; 5], "iIntervall == 0 never fires");
        assert_eq!(inert.timer, 5, "time still elapses");
    }
}
