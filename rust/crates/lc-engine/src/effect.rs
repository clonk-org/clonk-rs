use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectState {
    pub name: String,
    pub priority: i32,
    pub interval: i32,
    pub timer: i32,
}

impl EffectState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            priority: 100,
            interval: 1,
            timer: 0,
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

    pub fn advance_tick(&mut self) {
        if self.interval <= 0 {
            self.timer = self.timer.saturating_add(1);
            return;
        }
        self.timer += 1;
        if self.timer >= self.interval {
            self.timer = 0;
        }
    }
}

impl Default for EffectState {
    fn default() -> Self {
        Self::new("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
