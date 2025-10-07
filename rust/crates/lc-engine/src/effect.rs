use serde::{Deserialize, Serialize};

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
}

impl EffectState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            priority: 100,
            interval: 1,
            timer: 0,
            command_target: None,
            command_id: None,
        }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_interval(mut self, interval: i32) -> Self {
        self.interval = interval.max(1);
        if self.timer >= self.interval {
            self.timer %= self.interval;
        }
        self
    }

    pub fn with_timer(mut self, timer: i32) -> Self {
        self.timer = timer.max(0);
        if self.interval > 0 && self.timer >= self.interval {
            self.timer %= self.interval;
        }
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

    pub fn advance_tick(&mut self) -> bool {
        if self.interval <= 0 {
            self.timer = self.timer.saturating_add(1);
            return true;
        }
        self.timer += 1;
        if self.timer >= self.interval {
            self.timer = 0;
            true
        } else {
            false
        }
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
    Remove { name: String },
    Clear,
}

impl EffectCommand {
    pub fn add(effect: EffectState) -> Self {
        Self::Add(effect)
    }

    pub fn remove(name: impl Into<String>) -> Self {
        Self::Remove { name: name.into() }
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
