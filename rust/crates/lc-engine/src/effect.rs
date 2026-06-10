use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EffectVarValue {
    Int(i32),
    Bool(bool),
    String(String),
    Object(u64),
    Array(Vec<EffectVarValue>),
    Proplist(BTreeMap<String, EffectVarValue>),
    #[default]
    Nil,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectState {
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
}

impl EffectState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            priority: 100,
            interval: 0,
            timer: 0,
            command_target: None,
            command_id: None,
            vars: Vec::new(),
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

impl Default for EffectState {
    fn default() -> Self {
        Self::new("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectCommand {
    Add(EffectState),
    Remove { name: String, no_callbacks: bool },
    Clear,
}

impl EffectCommand {
    pub fn add(effect: EffectState) -> Self {
        Self::Add(effect)
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectStopReason {
    Removed,
    Cleared,
    Destroyed,
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectEventKind {
    Started,
    Timer,
    Stopped(EffectStopReason),
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
