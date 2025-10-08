mod action;
mod compat;
mod effect;
pub mod ffi;
pub mod fixtures;
mod landscape;
mod record;
pub mod scenario;

pub use action::{ActionLibrary, ActionProcedure, ActionSpec, ActionState, ActionUpdate};
pub use effect::EffectState;
pub use landscape::{CollisionResolution, Landscape, LandscapeError};
pub use record::{Playback, PlaybackError, Recorder, Recording};
pub use scenario::{Scenario, ScenarioError};

use compat::{enter_random_context, EffectContextOutcome};
use effect::{EffectCommand, EffectEvent, EffectEventKind, EffectStopReason};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::AddAssign;
use std::path::Path;

use lc_script::{DebuggerHooks, Engine as ScriptEngine, ScriptError, Value};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type DefinitionId = String;

pub const OWNER_NONE: i32 = -1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectId(u64);

impl ObjectId {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectStatus {
    Deleted,
    Normal,
    Inactive,
}

impl ObjectStatus {
    pub const fn is_active(self) -> bool {
        matches!(self, ObjectStatus::Normal)
    }

    pub const fn to_script_value(self) -> i32 {
        match self {
            ObjectStatus::Deleted => 0,
            ObjectStatus::Normal => 1,
            ObjectStatus::Inactive => 2,
        }
    }

    pub fn from_script_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(ObjectStatus::Deleted),
            1 => Some(ObjectStatus::Normal),
            2 => Some(ObjectStatus::Inactive),
            _ => None,
        }
    }
}

impl Default for ObjectStatus {
    fn default() -> Self {
        ObjectStatus::Normal
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CrewRole(String);

impl CrewRole {
    pub fn new(role: impl Into<String>) -> Self {
        Self(role.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CrewRole {
    fn from(role: &str) -> Self {
        Self::new(role)
    }
}

impl From<String> for CrewRole {
    fn from(role: String) -> Self {
        Self::new(role)
    }
}

impl fmt::Display for CrewRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrewCommandTarget {
    Cursor,
    Selection,
    Role(CrewRole),
}

impl CrewCommandTarget {
    pub const fn cursor() -> Self {
        Self::Cursor
    }

    pub const fn selection() -> Self {
        Self::Selection
    }

    pub fn role(role: impl Into<CrewRole>) -> Self {
        Self::Role(role.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vector2 {
    pub x: i32,
    pub y: i32,
}

impl Vector2 {
    pub const ZERO: Self = Self { x: 0, y: 0 };

    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    fn to_value(self) -> Value {
        Value::Array(vec![Value::Int(self.x), Value::Int(self.y)])
    }
}

impl AddAssign<Vector2> for Vector2 {
    fn add_assign(&mut self, rhs: Vector2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicsSettings {
    pub gravity: i32,
    pub max_fall_speed: i32,
    pub max_rise_speed: i32,
    #[serde(default = "PhysicsSettings::default_max_horizontal_speed")]
    pub max_horizontal_speed: i32,
}

impl PhysicsSettings {
    pub const DEFAULT_MAX_HORIZONTAL_SPEED: i32 = 12;

    pub const fn new(gravity: i32, max_fall_speed: i32, max_rise_speed: i32) -> Self {
        Self {
            gravity,
            max_fall_speed,
            max_rise_speed,
            max_horizontal_speed: Self::DEFAULT_MAX_HORIZONTAL_SPEED,
        }
    }

    pub fn checked(
        gravity: i32,
        max_fall_speed: i32,
        max_rise_speed: i32,
    ) -> Result<Self, &'static str> {
        if max_rise_speed > max_fall_speed {
            return Err("max_rise_speed must be <= max_fall_speed");
        }
        Ok(Self::new(gravity, max_fall_speed, max_rise_speed))
    }

    pub fn with_max_horizontal_speed(
        self,
        max_horizontal_speed: i32,
    ) -> Result<Self, &'static str> {
        if max_horizontal_speed < 0 {
            return Err("max_horizontal_speed must be >= 0");
        }
        Ok(Self {
            max_horizontal_speed,
            ..self
        })
    }

    const fn default_max_horizontal_speed() -> i32 {
        Self::DEFAULT_MAX_HORIZONTAL_SPEED
    }

    fn clamp_velocity(&self, velocity: &mut Vector2) {
        if velocity.y > self.max_fall_speed {
            velocity.y = self.max_fall_speed;
        }
        if velocity.y < self.max_rise_speed {
            velocity.y = self.max_rise_speed;
        }
        let max_horizontal = self.max_horizontal_speed.max(0);
        if velocity.x > max_horizontal {
            velocity.x = max_horizontal;
        }
        if velocity.x < -max_horizontal {
            velocity.x = -max_horizontal;
        }
    }
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            gravity: 1,
            max_fall_speed: 12,
            max_rise_speed: -20,
            max_horizontal_speed: Self::DEFAULT_MAX_HORIZONTAL_SPEED,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSettings {
    pub wind: i32,
    #[serde(default)]
    pub wind_variation: i32,
    #[serde(default)]
    pub wind_period: u32,
    #[serde(default)]
    pub temperature: i32,
    #[serde(default)]
    pub climate: i32,
    #[serde(default)]
    pub temperature_variation: i32,
    #[serde(default)]
    pub temperature_period: u32,
    #[serde(default)]
    pub temperature_phase: u32,
    #[serde(default)]
    pub time_of_day: u16,
    #[serde(default)]
    pub time_speed: i16,
}

impl EnvironmentSettings {
    pub const TIME_CYCLE: u16 = 2400;
    const MAX_TIME_SPEED: i16 = 120;

    pub const fn new(wind: i32) -> Self {
        Self {
            wind,
            wind_variation: 0,
            wind_period: 0,
            temperature: 0,
            climate: 0,
            temperature_variation: 0,
            temperature_period: 0,
            temperature_phase: 0,
            time_of_day: 0,
            time_speed: 0,
        }
    }

    pub fn with_wind_variation(mut self, variation: i32, period: u32) -> Self {
        if variation == 0 {
            self.wind_variation = 0;
            self.wind_period = 0;
            return self;
        }
        self.wind_variation = variation.abs();
        self.wind_period = period.max(2);
        self
    }

    pub fn with_temperature(mut self, temperature: i32) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn with_climate(mut self, climate: i32) -> Self {
        self.climate = climate.clamp(-50, 50);
        self
    }

    pub fn with_temperature_cycle(mut self, variation: i32, period: u32, phase: u32) -> Self {
        if variation == 0 {
            self.temperature_variation = 0;
            self.temperature_period = 0;
            self.temperature_phase = 0;
            return self;
        }

        let amplitude = variation.abs();
        let normalized_period = period.max(2);
        self.temperature_variation = amplitude;
        self.temperature_period = normalized_period;
        self.temperature_phase = if normalized_period == 0 {
            0
        } else {
            phase % normalized_period
        };
        self
    }

    pub fn with_time_of_day(mut self, time_of_day: i32) -> Self {
        self.time_of_day = Self::normalize_time_of_day(time_of_day);
        self
    }

    pub fn with_time_speed(mut self, time_speed: i32) -> Self {
        self.time_speed = Self::clamp_time_speed(time_speed);
        self
    }

    pub fn ambient_temperature(&self, frame: u64) -> i32 {
        let base = self.temperature.saturating_add(self.climate);
        if self.temperature_variation == 0 || self.temperature_period == 0 {
            return base;
        }

        let period = self.temperature_period as f32;
        let frame_offset = if self.temperature_period == 0 {
            0
        } else {
            (frame.wrapping_add(u64::from(self.temperature_phase)))
                % u64::from(self.temperature_period)
        };
        let phase = frame_offset as f32 / period;
        let angle = phase * core::f32::consts::TAU;
        let delta = (self.temperature_variation as f32 * angle.cos()).round() as i32;
        base.saturating_sub(delta)
    }

    pub fn advance_frame(&mut self) {
        if self.time_speed == 0 {
            return;
        }
        let next = (i32::from(self.time_of_day) + i32::from(self.time_speed))
            .rem_euclid(i32::from(Self::TIME_CYCLE));
        self.time_of_day = next as u16;
    }

    pub fn time_of_day(&self) -> u16 {
        self.time_of_day
    }

    pub fn time_speed(&self) -> i16 {
        self.time_speed
    }

    pub fn wind_force(&self, frame: u64) -> i32 {
        if self.wind_variation == 0 || self.wind_period == 0 {
            return self.wind;
        }

        let period = self.wind_period as f32;
        let phase = (frame % self.wind_period as u64) as f32 / period;
        let angle = phase * core::f32::consts::TAU;
        let delta = (self.wind_variation as f32 * angle.sin()).round() as i32;
        self.wind.saturating_add(delta)
    }

    fn apply_to_velocity(&self, velocity: &mut Vector2, frame: u64) {
        let wind_force = self.wind_force(frame);
        if wind_force != 0 {
            velocity.x = velocity.x.saturating_add(wind_force);
        }
    }

    fn normalize_time_of_day(time_of_day: i32) -> u16 {
        let cycle = i32::from(Self::TIME_CYCLE);
        time_of_day.rem_euclid(cycle) as u16
    }

    fn clamp_time_speed(time_speed: i32) -> i16 {
        let max = i32::from(Self::MAX_TIME_SPEED);
        time_speed.clamp(-max, max) as i16
    }
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self::new(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentFrame {
    pub settings: EnvironmentSettings,
    pub wind_force: i32,
    pub ambient_temperature: i32,
}

impl Default for EnvironmentFrame {
    fn default() -> Self {
        Self {
            settings: EnvironmentSettings::default(),
            wind_force: 0,
            ambient_temperature: 0,
        }
    }
}

fn default_owner() -> i32 {
    OWNER_NONE
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectState {
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
    pub action: ActionState,
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    #[serde(default)]
    pub crew_member: bool,
}

impl ObjectState {
    fn apply_delta(
        &mut self,
        delta: &ObjectDelta,
        library: &ActionLibrary,
    ) -> Option<(Option<ObjectId>, Option<ObjectId>)> {
        let previous_container = self.container;
        let mut container_change = None;
        if let Some(position) = delta.position {
            self.position = position;
        }
        if let Some(velocity) = delta.velocity {
            self.velocity = velocity;
        }
        if let Some(energy) = delta.energy {
            self.energy = energy;
        }
        if let Some(action) = &delta.action {
            self.action.apply_update_with_library(action, library);
        } else {
            self.action.reconcile_with_library(library);
        }
        if let Some(owner) = delta.owner {
            self.owner = owner;
        }
        if let Some(crew_member) = delta.crew_member {
            self.crew_member = crew_member;
        }
        if let Some(status) = delta.status {
            self.status = status;
        }
        if let Some(container) = delta.container {
            if self.container != container {
                self.container = container;
                container_change = Some((previous_container, self.container));
            }
        }

        self.action.reconcile_with_library(library);
        container_change
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ObjectDelta {
    position: Option<Vector2>,
    velocity: Option<Vector2>,
    energy: Option<i32>,
    action: Option<ActionUpdate>,
    status: Option<ObjectStatus>,
    owner: Option<i32>,
    crew_member: Option<bool>,
    container: Option<Option<ObjectId>>,
}

impl ObjectDelta {
    fn merge_update(&mut self, update: ObjectUpdate) {
        if let Some(position) = update.position {
            self.position = Some(position);
        }
        if let Some(velocity) = update.velocity {
            self.velocity = Some(velocity);
        }
        if let Some(energy) = update.energy {
            self.energy = Some(energy);
        }
        if let Some(owner) = update.owner {
            self.owner = Some(owner);
        }
        if let Some(crew_member) = update.crew_member {
            self.crew_member = Some(crew_member);
        }
        if let Some(container) = update.container {
            self.container = Some(container);
        }
        if let Some(status) = update.status {
            self.status = Some(status);
        }
        if let Some(action) = update.action {
            match &mut self.action {
                Some(existing) => existing.merge(action),
                None => self.action = Some(action),
            }
        }
    }
}

impl From<ObjectUpdate> for ObjectDelta {
    fn from(update: ObjectUpdate) -> Self {
        Self {
            position: update.position,
            velocity: update.velocity,
            energy: update.energy,
            action: update.action,
            status: update.status,
            owner: update.owner,
            crew_member: update.crew_member,
            container: update.container,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectUpdate {
    pub position: Option<Vector2>,
    pub velocity: Option<Vector2>,
    pub energy: Option<i32>,
    pub action: Option<ActionUpdate>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    #[serde(default)]
    pub owner: Option<i32>,
    #[serde(default)]
    pub crew_member: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Option<ObjectId>>,
}

impl ObjectUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = Some(velocity);
        self
    }

    pub fn with_energy(mut self, energy: i32) -> Self {
        self.energy = Some(energy);
        self
    }

    pub fn with_action(mut self, name: impl Into<String>) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_name(name);
        self.action = Some(update);
        self
    }

    pub fn with_action_phase(mut self, phase: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_phase(phase);
        self.action = Some(update);
        self
    }

    pub fn with_action_ticks(mut self, ticks: u32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_ticks(ticks);
        self.action = Some(update);
        self
    }

    pub fn with_action_update(mut self, update: ActionUpdate) -> Self {
        self.action = Some(update);
        self
    }

    pub fn with_owner(mut self, owner: i32) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn with_status(mut self, status: ObjectStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_container(mut self, container: ObjectId) -> Self {
        self.container = Some(Some(container));
        self
    }

    pub fn clear_container(mut self) -> Self {
        self.container = Some(None);
        self
    }

    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = Some(crew_member);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.position.is_none()
            && self.velocity.is_none()
            && self.energy.is_none()
            && self.action.is_none()
            && self.status.is_none()
            && self.owner.is_none()
            && self.crew_member.is_none()
            && self.container.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub delay: u32,
    pub update: ObjectUpdate,
    pub effects: Vec<EffectCommand>,
    pub destroy: bool,
    pub spawns: Vec<SpawnConfig>,
}

impl QueuedCommand {
    pub fn new(delay: u32, update: ObjectUpdate) -> Self {
        Self {
            delay,
            update,
            effects: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
        }
    }

    pub fn immediate(update: ObjectUpdate) -> Self {
        Self {
            delay: 0,
            update,
            effects: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
        }
    }

    pub fn with_delay(mut self, delay: u32) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_effects(mut self, effects: Vec<EffectCommand>) -> Self {
        self.effects = effects;
        self
    }

    pub fn with_destroy(mut self, destroy: bool) -> Self {
        self.destroy = destroy;
        self
    }

    pub fn with_spawns(mut self, spawns: Vec<SpawnConfig>) -> Self {
        self.spawns = spawns;
        self
    }

    pub fn update(&self) -> &ObjectUpdate {
        &self.update
    }

    pub fn effects(&self) -> &[EffectCommand] {
        &self.effects
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CrewSelection {
    selected: Vec<ObjectId>,
    cursor: Option<ObjectId>,
}

impl CrewSelection {
    fn select(&mut self, id: ObjectId) {
        if !self.selected.contains(&id) {
            self.selected.push(id);
        }
        if self.cursor.is_none() {
            self.cursor = Some(id);
        }
    }

    fn deselect(&mut self, id: ObjectId) {
        let mut removed = false;
        self.selected.retain(|candidate| {
            if *candidate == id {
                removed = true;
                false
            } else {
                true
            }
        });
        if self.cursor == Some(id) {
            if removed {
                self.cursor = self.selected.last().copied();
            } else if !self.selected.contains(&id) {
                self.cursor = self.selected.last().copied();
            }
        }
        if self.selected.is_empty() {
            self.cursor = None;
        }
    }

    fn clear(&mut self) {
        self.selected.clear();
        self.cursor = None;
    }

    fn prune(&mut self, alive: &HashSet<ObjectId>) {
        self.selected.retain(|id| alive.contains(id));
        if let Some(cursor) = self.cursor {
            if !alive.contains(&cursor) {
                self.cursor = self.selected.last().copied();
            }
        }
        if self.selected.is_empty() {
            self.cursor = None;
        }
    }

    fn is_empty(&self) -> bool {
        self.selected.is_empty() && self.cursor.is_none()
    }

    fn cursor(&self) -> Option<ObjectId> {
        self.cursor
    }

    fn selected(&self) -> &[ObjectId] {
        &self.selected
    }

    fn set_cursor(&mut self, cursor: Option<ObjectId>) {
        self.cursor = cursor;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CrewSelectionState {
    #[serde(default)]
    pub selected: Vec<ObjectId>,
    #[serde(default)]
    pub cursor: Option<ObjectId>,
}

impl From<&CrewSelection> for CrewSelectionState {
    fn from(selection: &CrewSelection) -> Self {
        Self {
            selected: selection.selected.clone(),
            cursor: selection.cursor,
        }
    }
}

impl From<CrewSelectionState> for CrewSelection {
    fn from(state: CrewSelectionState) -> Self {
        Self {
            selected: state.selected,
            cursor: state.cursor,
        }
    }
}

#[derive(Debug, Clone)]
struct Object {
    id: ObjectId,
    definition_id: DefinitionId,
    state: ObjectState,
    destroyed: bool,
    command_queue: VecDeque<QueuedCommand>,
}

#[derive(Debug)]
struct ContainerUpdateRecord {
    object_id: ObjectId,
    previous: Option<ObjectId>,
    new: Option<ObjectId>,
}

#[derive(Debug, Default)]
struct CommandQueueOutcome {
    spawns: Vec<SpawnConfig>,
    destroy: bool,
    effect_events: Vec<EffectEvent>,
    container_updates: Vec<ContainerUpdateRecord>,
}

impl Object {
    fn new(id: ObjectId, definition_id: DefinitionId, state: ObjectState) -> Self {
        Self {
            id,
            definition_id,
            destroyed: matches!(state.status, ObjectStatus::Deleted),
            state,
            command_queue: VecDeque::new(),
        }
    }

    fn mark_destroyed(&mut self) -> Vec<EffectEvent> {
        if self.destroyed {
            return Vec::new();
        }
        self.destroyed = true;
        self.state.status = ObjectStatus::Deleted;
        self.drain_effects_with_reason(EffectStopReason::Destroyed)
    }

    fn snapshot(&self, library: Option<&ActionLibrary>) -> ObjectSnapshot {
        let procedure = library
            .and_then(|lib| lib.procedure_name_for_action(&self.state.action.name))
            .map(|name| name.to_string());
        ObjectSnapshot {
            id: self.id,
            definition_id: self.definition_id.clone(),
            position: self.state.position,
            velocity: self.state.velocity,
            energy: self.state.energy,
            action: self.state.action.clone(),
            action_procedure: procedure,
            effects: self.state.effects.clone(),
            container: self.state.container,
            contents: self.state.contents.clone(),
            status: self.state.status,
            owner: self.state.owner,
            crew_member: self.state.crew_member,
        }
    }

    fn apply_effect_commands(&mut self, commands: &[EffectCommand]) -> Vec<EffectEvent> {
        let mut events = Vec::new();
        for command in commands {
            match command {
                EffectCommand::Add(effect) => {
                    let (inserted, replaced) = self.insert_effect(effect.clone());
                    if let Some(replaced) = replaced {
                        events.push(EffectEvent::stopped(replaced, EffectStopReason::Replaced));
                    }
                    events.push(EffectEvent::started(inserted));
                }
                EffectCommand::Remove { name } => {
                    if let Some(removed) = self.remove_effect(name) {
                        events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
                    }
                }
                EffectCommand::Clear => {
                    events.extend(self.drain_effects_with_reason(EffectStopReason::Cleared));
                }
            }
        }
        events
    }

    fn insert_effect(&mut self, mut effect: EffectState) -> (EffectState, Option<EffectState>) {
        if effect.interval <= 0 {
            effect.interval = 1;
        }
        if effect.timer < 0 {
            effect.timer = 0;
        }
        if effect.interval > 0 && effect.timer >= effect.interval {
            effect.timer %= effect.interval;
        }
        let replaced = self
            .state
            .effects
            .iter()
            .position(|existing| existing.name == effect.name)
            .map(|pos| self.state.effects.remove(pos));
        let mut insert_pos = 0;
        while insert_pos < self.state.effects.len()
            && self.state.effects[insert_pos].priority > effect.priority
        {
            insert_pos += 1;
        }
        let inserted = effect.clone();
        self.state.effects.insert(insert_pos, effect);
        (inserted, replaced)
    }

    fn remove_effect(&mut self, name: &str) -> Option<EffectState> {
        self.state
            .effects
            .iter()
            .position(|existing| existing.name == name)
            .map(|index| self.state.effects.remove(index))
    }

    fn drain_effects_with_reason(&mut self, reason: EffectStopReason) -> Vec<EffectEvent> {
        self.state
            .effects
            .drain(..)
            .map(|effect| EffectEvent::stopped(effect, reason))
            .collect()
    }

    fn tick_effects(&mut self) -> Vec<EffectEvent> {
        let mut events = Vec::new();
        for effect in &mut self.state.effects {
            if effect.advance_tick() {
                events.push(EffectEvent::timer(effect.clone()));
            }
        }
        events
    }

    fn enqueue_commands<I>(&mut self, commands: I)
    where
        I: IntoIterator<Item = QueuedCommand>,
    {
        self.command_queue.extend(commands);
    }

    fn apply_status(&mut self, status: ObjectStatus) -> Vec<EffectEvent> {
        if self.state.status == status {
            return Vec::new();
        }

        self.state.status = status;
        match status {
            ObjectStatus::Deleted => self.mark_destroyed(),
            _ => {
                if status.is_active() {
                    self.destroyed = false;
                }
                Vec::new()
            }
        }
    }

    fn execute_command_queue(
        &mut self,
        physics: &PhysicsSettings,
        landscape: Option<&Landscape>,
        action_library: &ActionLibrary,
    ) -> CommandQueueOutcome {
        let mut outcome = CommandQueueOutcome::default();
        loop {
            let execute_now = match self.command_queue.front_mut() {
                Some(command) if command.delay == 0 => true,
                Some(command) => {
                    command.delay -= 1;
                    false
                }
                None => break,
            };

            if !execute_now {
                break;
            }

            let command = self.command_queue.pop_front().expect("front exists");
            let delta: ObjectDelta = command.update.into();
            if let Some((previous, new)) = self.state.apply_delta(&delta, action_library) {
                outcome.container_updates.push(ContainerUpdateRecord {
                    object_id: self.id,
                    previous,
                    new,
                });
            }
            let mut effect_events = self.apply_effect_commands(&command.effects);
            if let Some(status) = delta.status {
                let mut status_events = self.apply_status(status);
                if !status_events.is_empty() {
                    effect_events.append(&mut status_events);
                }
                if matches!(status, ObjectStatus::Deleted) {
                    outcome.destroy = true;
                }
            }
            physics.clamp_velocity(&mut self.state.velocity);
            if command.destroy {
                if !matches!(self.state.status, ObjectStatus::Deleted) {
                    effect_events.extend(self.mark_destroyed());
                }
                outcome.destroy = true;
            }
            if !effect_events.is_empty() {
                outcome.effect_events.extend(effect_events);
            }
            if !command.spawns.is_empty() {
                outcome.spawns.extend(command.spawns);
            }
            if let Some(landscape) = landscape {
                let resolution =
                    landscape.resolve_collision(self.state.position, self.state.velocity);
                if resolution.collided {
                    self.state.position = resolution.position;
                    self.state.velocity = resolution.velocity;
                }
            }

            if outcome.destroy {
                self.command_queue.clear();
                break;
            }
        }
        outcome
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("definition `{0}` is already registered")]
    DefinitionAlreadyExists(String),
    #[error("unknown definition `{0}`")]
    UnknownDefinition(String),
    #[error("unknown object `{0}`")]
    UnknownObject(ObjectId),
    #[error("container error for object {object}: {detail}")]
    Container { object: ObjectId, detail: String },
    #[error("crew selection error for owner {owner}: {detail}")]
    CrewSelection { owner: i32, detail: String },
    #[error("crew role error for owner {owner}: {detail}")]
    CrewRole { owner: i32, detail: String },
    #[error("script error in {function} of `{definition}`")]
    Script {
        definition: String,
        function: &'static str,
        #[source]
        source: ScriptError,
    },
    #[error("invalid script output in {function} of `{definition}`: {detail}")]
    InvalidScriptOutput {
        definition: String,
        function: &'static str,
        detail: String,
    },
}

#[derive(Debug, Error)]
pub enum EngineStateIoError {
    #[error("I/O error while handling engine state")]
    Io(#[from] io::Error),
    #[error("failed to (de)serialize engine state as JSON")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnConfig {
    pub definition_id: DefinitionId,
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
    pub action: Option<ActionState>,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    pub owner: i32,
    #[serde(default)]
    pub crew_member: Option<bool>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    #[serde(default)]
    pub container: Option<ObjectId>,
}

impl SpawnConfig {
    pub fn new(definition_id: impl Into<String>) -> Self {
        Self {
            definition_id: definition_id.into(),
            position: Vector2::ZERO,
            velocity: Vector2::ZERO,
            energy: 0,
            action: None,
            effects: Vec::new(),
            owner: OWNER_NONE,
            crew_member: None,
            status: None,
            container: None,
        }
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = position;
        self
    }

    pub fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = velocity;
        self
    }

    pub fn with_energy(mut self, energy: i32) -> Self {
        self.energy = energy;
        self
    }

    pub fn with_action(mut self, action: ActionState) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_effects(mut self, effects: Vec<EffectState>) -> Self {
        self.effects = effects;
        self
    }

    pub fn add_effect(mut self, effect: EffectState) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn with_owner(mut self, owner: i32) -> Self {
        self.owner = owner;
        self
    }

    pub fn with_status(mut self, status: ObjectStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = Some(crew_member);
        self
    }

    pub fn with_container(mut self, container: ObjectId) -> Self {
        self.container = Some(container);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub id: ObjectId,
    pub definition_id: DefinitionId,
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
    #[serde(default)]
    pub action: ActionState,
    #[serde(default)]
    pub action_procedure: Option<String>,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    #[serde(default)]
    pub crew_member: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationSnapshot {
    pub frame: u64,
    pub objects: Vec<ObjectSnapshot>,
    #[serde(default)]
    pub environment: EnvironmentFrame,
    #[serde(default)]
    pub global_effects: Vec<EffectState>,
    #[serde(default)]
    pub crew_selection: HashMap<i32, CrewSelectionState>,
    #[serde(default)]
    pub crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    #[serde(default)]
    pub known_crew_owners: Vec<i32>,
    #[serde(default)]
    pub eliminated_crew_owners: Vec<i32>,
}

impl SimulationSnapshot {
    pub fn object(&self, id: ObjectId) -> Option<&ObjectSnapshot> {
        self.objects.iter().find(|object| object.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedObject {
    pub snapshot: ObjectSnapshot,
    #[serde(default)]
    pub command_queue: Vec<QueuedCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineState {
    pub frame: u64,
    pub physics: PhysicsSettings,
    pub environment: EnvironmentSettings,
    pub next_object_id: u64,
    #[serde(default)]
    pub landscape: Option<Landscape>,
    #[serde(default)]
    pub objects: Vec<PersistedObject>,
    #[serde(default)]
    pub crew_selection: HashMap<i32, CrewSelectionState>,
    #[serde(default)]
    pub crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    #[serde(default)]
    pub global_effects: Vec<EffectState>,
    #[serde(default)]
    pub known_crew_owners: Vec<i32>,
    #[serde(default)]
    pub eliminated_crew_owners: Vec<i32>,
    pub rng: ChaCha8Rng,
}

impl EngineState {
    /// Serializes the engine state to a writer using pretty-printed JSON.
    pub fn to_writer<W: Write>(&self, mut writer: W) -> Result<(), EngineStateIoError> {
        serde_json::to_writer_pretty(&mut writer, self).map_err(EngineStateIoError::from)?;
        writer.flush().map_err(EngineStateIoError::from)
    }

    /// Deserializes an engine state from any reader containing JSON data.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self, EngineStateIoError> {
        serde_json::from_reader(reader).map_err(EngineStateIoError::from)
    }

    /// Saves the state to a JSON file at the given path.
    pub fn save_to_path<P: AsRef<Path>>(&self, path: P) -> Result<(), EngineStateIoError> {
        let mut file = File::create(path)?;
        self.to_writer(&mut file)
    }

    /// Loads a state from a JSON file at the given path.
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self, EngineStateIoError> {
        let file = File::open(path)?;
        Self::from_reader(file)
    }

    /// Serializes the state into a pretty-printed JSON string.
    pub fn to_json_string(&self) -> Result<String, EngineStateIoError> {
        serde_json::to_string_pretty(self).map_err(EngineStateIoError::from)
    }

    /// Parses an engine state from a JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, EngineStateIoError> {
        serde_json::from_str(json).map_err(EngineStateIoError::from)
    }
}

pub struct Definition {
    id: DefinitionId,
    name: String,
    script: ScriptEngine,
    has_initialize: bool,
    has_step: bool,
    action_library: ActionLibrary,
    crew_member: bool,
}

impl Definition {
    pub fn from_script(
        id: impl Into<String>,
        name: impl Into<String>,
        source: &str,
    ) -> Result<Self, EngineError> {
        let id = id.into();
        let name = name.into();
        let mut script = ScriptEngine::new();
        script
            .load_script(source)
            .map_err(|source| EngineError::Script {
                definition: id.clone(),
                function: "load",
                source,
            })?;
        compat::register_host_functions(&mut script);
        let has_initialize = script.has_function("Initialize");
        let has_step = script.has_function("Step");
        Ok(Self {
            id,
            name,
            script,
            has_initialize,
            has_step,
            action_library: ActionLibrary::default(),
            crew_member: false,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_debugger_hooks(&mut self, hooks: DebuggerHooks) {
        self.script.set_debugger_hooks(hooks);
    }

    pub fn configure_actions(
        &mut self,
        default_action: Option<String>,
        specs: HashMap<String, ActionSpec>,
    ) {
        self.action_library = ActionLibrary::new(default_action, specs);
    }

    pub fn action_library(&self) -> &ActionLibrary {
        &self.action_library
    }

    pub fn default_action_state(&self) -> ActionState {
        ActionState::new(self.action_library.default_action())
    }

    pub fn is_crew(&self) -> bool {
        self.crew_member
    }

    pub fn set_crew_member(&mut self, crew_member: bool) {
        self.crew_member = crew_member;
    }

    fn call_initialize(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        random: i32,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
    ) -> Result<(CommandBatch, ChaCha8Rng), EngineError> {
        if !self.has_initialize {
            return Ok((CommandBatch::default(), rng));
        }
        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::Int(random),
        ];
        let guard = enter_random_context(rng);
        let (result, host_effects) = compat::with_effect_context(
            Some(compat::HostObjectContext::new(
                object_id,
                state.status,
                &state.effects,
            )),
            global_effects,
            || self.script.call("Initialize", &args),
        );
        let rng = guard.finish();
        let result = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "Initialize",
            source,
        })?;
        let mut batch = parse_command(&self.id, "Initialize", result)?;
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            destroy_object,
        } = host_effects;

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        if !object_commands.is_empty() {
            batch.commands.extend(object_commands);
        }
        if destroy_object {
            batch.destroy = true;
        }
        if !host_object_effects.is_empty() {
            batch.effects.extend(host_object_effects);
        }
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        Ok((batch, rng))
    }

    fn call_step(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        frame: u64,
        random: i32,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
    ) -> Result<(CommandBatch, ChaCha8Rng), EngineError> {
        if !self.has_step {
            return Ok((CommandBatch::default(), rng));
        }
        let frame_value = if frame > i32::MAX as u64 {
            i32::MAX
        } else {
            frame as i32
        };
        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::Int(frame_value),
            Value::Int(random),
        ];
        let guard = enter_random_context(rng);
        let (result, host_effects) = compat::with_effect_context(
            Some(compat::HostObjectContext::new(
                object_id,
                state.status,
                &state.effects,
            )),
            global_effects,
            || self.script.call("Step", &args),
        );
        let rng = guard.finish();
        let result = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "Step",
            source,
        })?;
        let mut batch = parse_command(&self.id, "Step", result)?;
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            destroy_object,
        } = host_effects;

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        if !object_commands.is_empty() {
            batch.commands.extend(object_commands);
        }
        if destroy_object {
            batch.destroy = true;
        }
        if !host_object_effects.is_empty() {
            batch.effects.extend(host_object_effects);
        }
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        Ok((batch, rng))
    }

    fn call_effect_start(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
    ) -> Result<(EffectContextOutcome, ChaCha8Rng), EngineError> {
        self.dispatch_effect_callback(
            state,
            object_id,
            effect,
            "Start",
            "FxStart",
            Vec::new(),
            rng,
            global_effects,
        )
    }

    fn call_effect_timer(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
    ) -> Result<(EffectContextOutcome, ChaCha8Rng), EngineError> {
        self.dispatch_effect_callback(
            state,
            object_id,
            effect,
            "Timer",
            "FxTimer",
            vec![Value::Int(effect.timer)],
            rng,
            global_effects,
        )
    }

    fn call_effect_stop(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        reason: EffectStopReason,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
    ) -> Result<(EffectContextOutcome, ChaCha8Rng), EngineError> {
        self.dispatch_effect_callback(
            state,
            object_id,
            effect,
            "Stop",
            "FxStop",
            vec![effect_stop_reason_value(reason)],
            rng,
            global_effects,
        )
    }

    fn dispatch_effect_callback(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        event: &'static str,
        function_label: &'static str,
        mut extras: Vec<Value>,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
    ) -> Result<(EffectContextOutcome, ChaCha8Rng), EngineError> {
        if !self.script.has_effect_callback(&effect.name, event) {
            return Ok((EffectContextOutcome::empty(), rng));
        }

        let mut args = Vec::with_capacity(2 + extras.len());
        args.push(build_state_value(
            &self.id,
            object_id,
            state,
            &self.action_library,
        ));
        args.push(build_effect_value(effect));
        args.append(&mut extras);

        let guard = enter_random_context(rng);
        let (result, commands) = compat::with_effect_context(
            Some(compat::HostObjectContext::new(
                object_id,
                state.status,
                &state.effects,
            )),
            global_effects,
            || {
                self.script
                    .call_effect_callback(&effect.name, event, &args)
                    .map(|_| ())
            },
        );
        let rng = guard.finish();

        result
            .map(|_| (commands, rng))
            .map_err(|source| EngineError::Script {
                definition: format!("{}::{}", self.id, effect.name),
                function: function_label,
                source,
            })
    }
}

#[derive(Debug, Default)]
struct CommandBatch {
    delta: ObjectDelta,
    spawns: Vec<SpawnConfig>,
    destroy: bool,
    commands: Vec<QueuedCommand>,
    effects: Vec<EffectCommand>,
    global_effects: Vec<EffectCommand>,
}

pub struct Engine {
    definitions: HashMap<DefinitionId, Definition>,
    objects: Vec<Object>,
    next_object_id: u64,
    rng: ChaCha8Rng,
    frame: u64,
    landscape: Option<Landscape>,
    physics: PhysicsSettings,
    environment: EnvironmentSettings,
    global_effects: Vec<EffectState>,
    crew_selection: HashMap<i32, CrewSelection>,
    crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    known_crew_owners: HashSet<i32>,
    eliminated_crew_owners: HashSet<i32>,
}

impl Engine {
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            definitions: HashMap::new(),
            objects: Vec::new(),
            next_object_id: 1,
            rng: ChaCha8Rng::seed_from_u64(seed),
            frame: 0,
            landscape: None,
            physics: PhysicsSettings::default(),
            environment: EnvironmentSettings::default(),
            global_effects: Vec::new(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners: HashSet::new(),
            eliminated_crew_owners: HashSet::new(),
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn set_landscape(&mut self, landscape: Landscape) {
        self.landscape = Some(landscape);
    }

    pub fn clear_landscape(&mut self) {
        self.landscape = None;
    }

    pub fn landscape(&self) -> Option<&Landscape> {
        self.landscape.as_ref()
    }

    pub fn physics(&self) -> PhysicsSettings {
        self.physics
    }

    pub fn set_physics(&mut self, physics: PhysicsSettings) {
        self.physics = physics;
        for object in &mut self.objects {
            self.physics.clamp_velocity(&mut object.state.velocity);
        }
    }

    pub fn environment(&self) -> EnvironmentSettings {
        self.environment
    }

    pub fn set_environment(&mut self, environment: EnvironmentSettings) {
        self.environment = environment;
    }

    pub fn crew_members(&self, owner: i32) -> Vec<ObjectId> {
        self.objects
            .iter()
            .filter(|object| {
                object.state.crew_member
                    && object.state.owner == owner
                    && object.state.status.is_active()
            })
            .map(|object| object.id)
            .collect()
    }

    pub fn eliminated_owners(&self) -> Vec<i32> {
        let mut eliminated: Vec<_> = self.eliminated_crew_owners.iter().cloned().collect();
        eliminated.sort_unstable();
        eliminated
    }

    pub fn is_owner_eliminated(&self, owner: i32) -> bool {
        self.eliminated_crew_owners.contains(&owner)
    }

    pub fn selected_crew(&self, owner: i32) -> Vec<ObjectId> {
        self.crew_selection
            .get(&owner)
            .map(|selection| selection.selected().to_vec())
            .unwrap_or_default()
    }

    pub fn crew_cursor(&self, owner: i32) -> Option<ObjectId> {
        self.crew_selection
            .get(&owner)
            .and_then(|selection| selection.cursor())
    }

    pub fn select_crew<I>(&mut self, owner: i32, crew: I) -> Result<(), EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        let mut validated = Vec::new();
        for id in crew {
            let object = self
                .objects
                .iter()
                .find(|object| object.id == id)
                .ok_or(EngineError::UnknownObject(id))?;
            if object.state.owner != owner {
                return Err(EngineError::CrewSelection {
                    owner,
                    detail: format!("object {} is owned by {}", id, object.state.owner),
                });
            }
            if !object.state.crew_member {
                return Err(EngineError::CrewSelection {
                    owner,
                    detail: format!("object {} is not a crew member", id),
                });
            }
            if !object.state.status.is_active() {
                return Err(EngineError::CrewSelection {
                    owner,
                    detail: format!("object {} is not active", id),
                });
            }
            validated.push(id);
        }

        if validated.is_empty() {
            return Ok(());
        }

        let selection = self.crew_selection.entry(owner).or_default();
        for id in validated {
            selection.select(id);
        }
        Ok(())
    }

    pub fn deselect_crew<I>(&mut self, owner: i32, crew: I)
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if let Some(selection) = self.crew_selection.get_mut(&owner) {
            for id in crew {
                selection.deselect(id);
            }
            if selection.is_empty() {
                self.crew_selection.remove(&owner);
            }
        }
    }

    pub fn clear_crew_selection(&mut self, owner: i32) {
        if let Some(selection) = self.crew_selection.get_mut(&owner) {
            selection.clear();
        }
        self.crew_selection.remove(&owner);
    }

    pub fn set_crew_cursor(
        &mut self,
        owner: i32,
        cursor: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        match cursor {
            Some(id) => {
                let object = self
                    .objects
                    .iter()
                    .find(|object| object.id == id)
                    .ok_or(EngineError::UnknownObject(id))?;
                if object.state.owner != owner {
                    return Err(EngineError::CrewSelection {
                        owner,
                        detail: format!("object {} is owned by {}", id, object.state.owner),
                    });
                }
                if !object.state.crew_member {
                    return Err(EngineError::CrewSelection {
                        owner,
                        detail: format!("object {} is not a crew member", id),
                    });
                }
                if !object.state.status.is_active() {
                    return Err(EngineError::CrewSelection {
                        owner,
                        detail: format!("object {} is not active", id),
                    });
                }
                let selection = self.crew_selection.entry(owner).or_default();
                selection.select(id);
                selection.set_cursor(Some(id));
            }
            None => {
                if let Some(selection) = self.crew_selection.get_mut(&owner) {
                    selection.set_cursor(None);
                    if selection.selected().is_empty() {
                        self.crew_selection.remove(&owner);
                    }
                }
            }
        }

        Ok(())
    }

    pub fn set_crew_role(
        &mut self,
        owner: i32,
        object_id: ObjectId,
        role: CrewRole,
    ) -> Result<(), EngineError> {
        if role.as_str().trim().is_empty() {
            return Err(EngineError::CrewRole {
                owner,
                detail: "role name must not be empty".to_string(),
            });
        }

        let object = self
            .objects
            .iter()
            .find(|object| object.id == object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        if object.state.owner != owner {
            return Err(EngineError::CrewRole {
                owner,
                detail: format!("object {} is owned by {}", object_id, object.state.owner),
            });
        }
        if !object.state.crew_member {
            return Err(EngineError::CrewRole {
                owner,
                detail: format!("object {} is not a crew member", object_id),
            });
        }
        if !object.state.status.is_active() {
            return Err(EngineError::CrewRole {
                owner,
                detail: format!("object {} is not active", object_id),
            });
        }

        self.crew_roles
            .entry(owner)
            .or_default()
            .insert(object_id, role);
        Ok(())
    }

    pub fn crew_role(&self, owner: i32, object_id: ObjectId) -> Option<&CrewRole> {
        self.crew_roles
            .get(&owner)
            .and_then(|roles| roles.get(&object_id))
    }

    pub fn crew_role_assignments(&self, owner: i32) -> HashMap<ObjectId, CrewRole> {
        self.crew_roles.get(&owner).cloned().unwrap_or_default()
    }

    pub fn clear_crew_role(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(assignments) = self.crew_roles.get_mut(&owner) {
            assignments.remove(&object_id);
            if assignments.is_empty() {
                self.crew_roles.remove(&owner);
            }
        }
    }

    pub fn clear_roles_for_owner(&mut self, owner: i32) {
        self.crew_roles.remove(&owner);
    }

    pub fn apply_command(
        &mut self,
        owner: i32,
        target: CrewCommandTarget,
        update: ObjectUpdate,
    ) -> Result<(), EngineError> {
        self.prune_roles();
        let mut recipients = self.resolve_command_targets(owner, &target);
        if recipients.is_empty() {
            return Ok(());
        }

        recipients.sort_by_key(|id| id.as_u64());
        recipients.dedup();
        for object_id in recipients {
            self.apply_object_update(object_id, update.clone())?;
        }
        Ok(())
    }

    pub fn global_effects(&self) -> &[EffectState] {
        &self.global_effects
    }

    pub fn register_definition(&mut self, definition: Definition) -> Result<(), EngineError> {
        let id = definition.id().to_string();
        if self.definitions.contains_key(&id) {
            return Err(EngineError::DefinitionAlreadyExists(id));
        }
        self.definitions.insert(id, definition);
        Ok(())
    }

    pub fn spawn_object(&mut self, config: SpawnConfig) -> Result<ObjectId, EngineError> {
        let (id, additional) = self.spawn_single(config)?;
        self.process_spawn_queue(additional)?;
        self.refresh_elimination_state();
        Ok(id)
    }

    pub fn tick(&mut self) -> Result<SimulationSnapshot, EngineError> {
        self.frame += 1;
        let frame = self.frame;
        self.environment.advance_frame();
        let mut spawn_requests = Vec::new();
        self.tick_global_effects();
        let landscape_for_commands = self.landscape.clone();
        for idx in 0..self.objects.len() {
            let definition_id = self.objects[idx].definition_id.clone();
            let action_library = {
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                definition.action_library().clone()
            };
            let (
                queued_spawns,
                queue_destroy,
                queue_events,
                container_updates,
                (object_id, previous_owner, new_owner, new_crew),
            ) = {
                let object = &mut self.objects[idx];
                let previous_owner = object.state.owner;
                let outcome = object.execute_command_queue(
                    &self.physics,
                    landscape_for_commands.as_ref(),
                    &action_library,
                );
                let new_owner = object.state.owner;
                let new_crew = object.state.crew_member;
                (
                    outcome.spawns,
                    outcome.destroy,
                    outcome.effect_events,
                    outcome.container_updates,
                    (object.id, previous_owner, new_owner, new_crew),
                )
            };
            self.update_selection_for_state_change(object_id, previous_owner, new_owner, new_crew);

            for update in container_updates {
                self.apply_container_change(update.object_id, update.previous, update.new)?;
            }

            if !queue_events.is_empty() {
                let object_id = self.objects[idx].id;
                let global_view = self.global_effects.clone();
                let rng_state = self.rng.clone();
                let (global_cmds, new_rng) = {
                    let definition = self
                        .definitions
                        .get(&definition_id)
                        .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                    let object = &mut self.objects[idx];
                    Self::run_effect_events_for_object(
                        definition,
                        rng_state,
                        object_id,
                        object,
                        queue_events,
                        global_view,
                    )?
                };
                self.rng = new_rng;
                if !global_cmds.is_empty() {
                    self.apply_global_effect_commands(&global_cmds);
                }
            }

            if !queued_spawns.is_empty() {
                spawn_requests.extend(queued_spawns);
            }

            if queue_destroy || self.objects[idx].destroyed {
                continue;
            }

            if !self.objects[idx].state.status.is_active() {
                continue;
            }

            let timer_events = {
                let object = &mut self.objects[idx];
                object.tick_effects()
            };

            if !timer_events.is_empty() {
                let object_id = self.objects[idx].id;
                let global_view = self.global_effects.clone();
                let rng_state = self.rng.clone();
                let (global_cmds, new_rng) = {
                    let definition = self
                        .definitions
                        .get(&definition_id)
                        .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                    let object = &mut self.objects[idx];
                    Self::run_effect_events_for_object(
                        definition,
                        rng_state,
                        object_id,
                        object,
                        timer_events,
                        global_view,
                    )?
                };
                self.rng = new_rng;
                if !global_cmds.is_empty() {
                    self.apply_global_effect_commands(&global_cmds);
                }
            }

            {
                let object = &mut self.objects[idx];
                object.state.action.advance_with_library(&action_library);
            }
            self.apply_physics_at_index(idx);
            {
                let object = &mut self.objects[idx];
                object.state.position += object.state.velocity;
            }

            self.apply_landscape_at_index(idx);

            let object_id = self.objects[idx].id;
            let state_snapshot = self.objects[idx].state.clone();
            let random = self.next_random_i32();

            let rng_state = self.rng.clone();
            let (command, new_rng) = {
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                definition.call_step(
                    &state_snapshot,
                    object_id,
                    frame,
                    random,
                    rng_state,
                    &self.global_effects,
                )?
            };
            self.rng = new_rng;

            let CommandBatch {
                delta,
                spawns,
                destroy,
                commands,
                effects,
                global_effects,
            } = command;

            let mut effect_events = Vec::new();
            let (object_id, previous_owner, new_owner, new_crew, container_change) = {
                let object = &mut self.objects[idx];
                let previous_owner = object.state.owner;
                let container_change = object.state.apply_delta(&delta, &action_library);
                let mut applied = object.apply_effect_commands(&effects);
                effect_events.append(&mut applied);
                self.physics.clamp_velocity(&mut object.state.velocity);
                if destroy {
                    effect_events.extend(object.mark_destroyed());
                }
                if !commands.is_empty() {
                    object.enqueue_commands(commands);
                }
                (
                    object.id,
                    previous_owner,
                    object.state.owner,
                    object.state.crew_member,
                    container_change,
                )
            };
            self.update_selection_for_state_change(object_id, previous_owner, new_owner, new_crew);
            if let Some((previous_container, new_container)) = container_change {
                self.apply_container_change(object_id, previous_container, new_container)?;
            }

            if !global_effects.is_empty() {
                self.apply_global_effect_commands(&global_effects);
            }

            if !effect_events.is_empty() {
                let previous_container;
                let new_container;
                let global_cmds = {
                    let definition = self
                        .definitions
                        .get(&definition_id)
                        .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                    let global_view = self.global_effects.clone();
                    let object = &mut self.objects[idx];
                    previous_container = object.state.container;
                    let rng_state = self.rng.clone();
                    let (global_cmds, new_rng) = Self::run_effect_events_for_object(
                        definition,
                        rng_state,
                        object_id,
                        object,
                        effect_events,
                        global_view,
                    )?;
                    self.rng = new_rng;
                    new_container = object.state.container;
                    global_cmds
                };
                if !global_cmds.is_empty() {
                    self.apply_global_effect_commands(&global_cmds);
                }
                if previous_container != new_container {
                    self.apply_container_change(object_id, previous_container, new_container)?;
                }
            }

            if self.objects[idx].destroyed {
                continue;
            }

            self.apply_landscape_at_index(idx);
            spawn_requests.extend(spawns.into_iter());
        }

        self.detach_destroyed_objects()?;
        self.objects.retain(|object| !object.destroyed);
        self.prune_selection();
        self.process_spawn_queue(spawn_requests)?;
        self.refresh_elimination_state();
        Ok(self.snapshot())
    }

    pub fn object_snapshot(&self, id: ObjectId) -> Option<ObjectSnapshot> {
        self.objects
            .iter()
            .find(|object| object.id == id)
            .map(|object| {
                let library = self
                    .definitions
                    .get(&object.definition_id)
                    .map(|definition| definition.action_library());
                object.snapshot(library)
            })
    }

    pub fn apply_object_update(
        &mut self,
        id: ObjectId,
        update: ObjectUpdate,
    ) -> Result<(), EngineError> {
        let landscape = self.landscape.clone();
        let index = self
            .objects
            .iter()
            .position(|object| object.id == id)
            .ok_or(EngineError::UnknownObject(id))?;

        let ObjectUpdate {
            position,
            velocity,
            energy,
            action,
            status,
            owner,
            crew_member,
            container,
        } = update;

        let definition_id = self.objects[index].definition_id.clone();
        let action_library = {
            let definition = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            definition.action_library().clone()
        };

        let (object_id, previous_owner, new_owner, new_crew, container_change) = {
            let object = &mut self.objects[index];
            let previous_owner = object.state.owner;
            let previous_container = object.state.container;
            let mut container_change = None;

            if let Some(position) = position {
                object.state.position = position;
            }
            if let Some(velocity) = velocity {
                object.state.velocity = velocity;
            }
            if let Some(energy) = energy {
                object.state.energy = energy;
            }
            if let Some(action) = action {
                object
                    .state
                    .action
                    .apply_update_with_library(&action, &action_library);
            } else {
                object.state.action.reconcile_with_library(&action_library);
            }
            if let Some(owner) = owner {
                object.state.owner = owner;
            }
            if let Some(crew_member) = crew_member {
                object.state.crew_member = crew_member;
            }
            if let Some(status) = status {
                object.apply_status(status);
            }
            if let Some(container_update) = container {
                if object.state.container != container_update {
                    object.state.container = container_update;
                    container_change = Some((previous_container, object.state.container));
                }
            }

            self.physics.clamp_velocity(&mut object.state.velocity);

            if let Some(landscape) = landscape.as_ref() {
                let resolution =
                    landscape.resolve_collision(object.state.position, object.state.velocity);
                if resolution.collided {
                    object.state.position = resolution.position;
                    object.state.velocity = resolution.velocity;
                }
            }

            (
                object.id,
                previous_owner,
                object.state.owner,
                object.state.crew_member,
                container_change,
            )
        };

        self.update_selection_for_state_change(object_id, previous_owner, new_owner, new_crew);
        if let Some((previous_container, new_container)) = container_change {
            self.apply_container_change(object_id, previous_container, new_container)?;
        }
        if self.objects[index].destroyed
            || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
        {
            self.detach_destroyed_objects()?;
        }
        self.refresh_elimination_state();

        Ok(())
    }

    pub fn queue_object_command(
        &mut self,
        id: ObjectId,
        command: QueuedCommand,
    ) -> Result<(), EngineError> {
        self.queue_object_commands(id, std::iter::once(command))
    }

    pub fn queue_object_commands<I>(&mut self, id: ObjectId, commands: I) -> Result<(), EngineError>
    where
        I: IntoIterator<Item = QueuedCommand>,
    {
        let object = self
            .objects
            .iter_mut()
            .find(|object| object.id == id)
            .ok_or(EngineError::UnknownObject(id))?;
        object.enqueue_commands(commands);
        Ok(())
    }

    pub fn snapshot(&self) -> SimulationSnapshot {
        let mut objects = Vec::with_capacity(self.objects.len());
        for object in &self.objects {
            let library = self
                .definitions
                .get(&object.definition_id)
                .map(|definition| definition.action_library());
            objects.push(object.snapshot(library));
        }
        objects.sort_by_key(|object| object.id);
        let crew_selection = self
            .crew_selection
            .iter()
            .map(|(&owner, selection)| (owner, CrewSelectionState::from(selection)))
            .collect();
        let crew_roles = self.crew_roles.clone();
        let mut known_crew_owners: Vec<_> = self.known_crew_owners.iter().cloned().collect();
        known_crew_owners.sort_unstable();
        let mut eliminated_crew_owners: Vec<_> =
            self.eliminated_crew_owners.iter().cloned().collect();
        eliminated_crew_owners.sort_unstable();
        let environment = EnvironmentFrame {
            settings: self.environment,
            wind_force: self.environment.wind_force(self.frame),
            ambient_temperature: self.environment.ambient_temperature(self.frame),
        };
        SimulationSnapshot {
            frame: self.frame,
            objects,
            environment,
            global_effects: self.global_effects.clone(),
            crew_selection,
            crew_roles,
            known_crew_owners,
            eliminated_crew_owners,
        }
    }

    pub fn capture_state(&self) -> EngineState {
        let objects = self
            .objects
            .iter()
            .map(|object| {
                let library = self
                    .definitions
                    .get(&object.definition_id)
                    .map(|definition| definition.action_library());
                PersistedObject {
                    snapshot: object.snapshot(library),
                    command_queue: object.command_queue.iter().cloned().collect(),
                }
            })
            .collect();

        let crew_selection = self
            .crew_selection
            .iter()
            .map(|(&owner, selection)| (owner, CrewSelectionState::from(selection)))
            .collect();

        let crew_roles = self
            .crew_roles
            .iter()
            .map(|(&owner, roles)| (owner, roles.clone()))
            .collect();

        let mut known_crew_owners: Vec<_> = self.known_crew_owners.iter().cloned().collect();
        known_crew_owners.sort_unstable();
        let mut eliminated_crew_owners: Vec<_> =
            self.eliminated_crew_owners.iter().cloned().collect();
        eliminated_crew_owners.sort_unstable();

        EngineState {
            frame: self.frame,
            physics: self.physics,
            environment: self.environment,
            next_object_id: self.next_object_id,
            landscape: self.landscape.clone(),
            objects,
            crew_selection,
            crew_roles,
            global_effects: self.global_effects.clone(),
            known_crew_owners,
            eliminated_crew_owners,
            rng: self.rng.clone(),
        }
    }

    pub fn restore_state(&mut self, state: &EngineState) -> Result<(), EngineError> {
        for object in &state.objects {
            if !self
                .definitions
                .contains_key(&object.snapshot.definition_id)
            {
                return Err(EngineError::UnknownDefinition(
                    object.snapshot.definition_id.clone(),
                ));
            }
        }

        self.frame = state.frame;
        self.physics = state.physics;
        self.environment = state.environment;
        self.landscape = state.landscape.clone();
        self.rng = state.rng.clone();
        self.objects.clear();
        self.global_effects = state.global_effects.clone();
        self.crew_selection = state
            .crew_selection
            .iter()
            .map(|(&owner, selection)| (owner, CrewSelection::from(selection.clone())))
            .collect();

        let mut container_assignments = Vec::new();
        for persisted in &state.objects {
            let snapshot = &persisted.snapshot;
            let mut object = Object::new(
                snapshot.id,
                snapshot.definition_id.clone(),
                ObjectState {
                    position: snapshot.position,
                    velocity: snapshot.velocity,
                    energy: snapshot.energy,
                    action: snapshot.action.clone(),
                    effects: snapshot.effects.clone(),
                    container: None,
                    contents: Vec::new(),
                    status: snapshot.status,
                    owner: snapshot.owner,
                    crew_member: snapshot.crew_member,
                },
            );
            object.command_queue = VecDeque::from(persisted.command_queue.clone());
            self.objects.push(object);
            if let Some(container) = snapshot.container {
                container_assignments.push((snapshot.id, container));
            }
        }

        for (object_id, container) in container_assignments {
            self.apply_container_change(object_id, None, Some(container))?;
        }

        self.crew_roles = state
            .crew_roles
            .iter()
            .map(|(&owner, roles)| {
                let mut filtered = HashMap::new();
                for (&object_id, role) in roles {
                    if let Some(object) = self.objects.iter().find(|object| object.id == object_id)
                    {
                        if object.state.crew_member && object.state.owner == owner {
                            filtered.insert(object_id, role.clone());
                        }
                    }
                }
                (owner, filtered)
            })
            .filter(|(_, roles)| !roles.is_empty())
            .collect();

        self.known_crew_owners = state.known_crew_owners.iter().cloned().collect();
        self.eliminated_crew_owners = state.eliminated_crew_owners.iter().cloned().collect();

        let highest_id = self
            .objects
            .iter()
            .map(|object| object.id.as_u64())
            .max()
            .unwrap_or(0);
        self.next_object_id = state.next_object_id.max(highest_id + 1);

        self.prune_roles();
        self.prune_selection();
        self.refresh_elimination_state();

        Ok(())
    }

    fn run_effect_events_for_object(
        definition: &Definition,
        mut rng: ChaCha8Rng,
        object_id: ObjectId,
        object: &mut Object,
        events: Vec<EffectEvent>,
        mut global_view: Vec<EffectState>,
    ) -> Result<(Vec<EffectCommand>, ChaCha8Rng), EngineError> {
        if events.is_empty() {
            return Ok((Vec::new(), rng));
        }

        let mut queue: VecDeque<EffectEvent> = VecDeque::from(events);
        let mut state_snapshot = object.state.clone();
        let mut global_commands = Vec::new();

        while let Some(event) = queue.pop_front() {
            let snapshot_for_call = state_snapshot.clone();
            let (outcome, new_rng) = match event.kind {
                EffectEventKind::Started => definition.call_effect_start(
                    &snapshot_for_call,
                    object_id,
                    &event.effect,
                    rng,
                    &global_view,
                )?,
                EffectEventKind::Timer => definition.call_effect_timer(
                    &snapshot_for_call,
                    object_id,
                    &event.effect,
                    rng,
                    &global_view,
                )?,
                EffectEventKind::Stopped(reason) => definition.call_effect_stop(
                    &snapshot_for_call,
                    object_id,
                    &event.effect,
                    reason,
                    rng,
                    &global_view,
                )?,
            };
            rng = new_rng;

            let compat::EffectContextOutcome {
                object: object_effect_commands,
                global: mut global_effect_commands,
                object_update,
                object_commands,
                destroy_object,
            } = outcome;

            if let Some(update) = object_update {
                let mut delta = ObjectDelta::default();
                delta.merge_update(update);
                object
                    .state
                    .apply_delta(&delta, definition.action_library());
                state_snapshot = object.state.clone();
            }

            if !object_commands.is_empty() {
                object.enqueue_commands(object_commands);
            }

            if destroy_object {
                let mut generated = object.mark_destroyed();
                if !generated.is_empty() {
                    queue.extend(generated.drain(..));
                }
            }

            if !object_effect_commands.is_empty() {
                let mut generated = object.apply_effect_commands(&object_effect_commands);
                state_snapshot = object.state.clone();
                if !generated.is_empty() {
                    queue.extend(generated.drain(..));
                }
            }

            if !global_effect_commands.is_empty() {
                apply_effect_commands_to_stack(&mut global_view, &global_effect_commands);
                global_commands.append(&mut global_effect_commands);
            }
        }

        Ok((global_commands, rng))
    }

    fn apply_landscape(&self, state: &mut ObjectState) {
        if let Some(landscape) = &self.landscape {
            let resolution = landscape.resolve_collision(state.position, state.velocity);
            if resolution.collided {
                state.position = resolution.position;
                state.velocity = resolution.velocity;
            }
        }
    }

    fn update_selection_for_state_change(
        &mut self,
        object_id: ObjectId,
        previous_owner: i32,
        new_owner: i32,
        new_crew_member: bool,
    ) {
        if previous_owner != new_owner {
            self.remove_from_selection(previous_owner, object_id);
            self.remove_from_roles(previous_owner, object_id);
        }
        if !new_crew_member {
            self.remove_from_selection(new_owner, object_id);
            self.remove_from_roles(new_owner, object_id);
        }
        if let Some(object) = self.objects.iter().find(|object| object.id == object_id) {
            if !object.state.status.is_active() {
                self.remove_from_selection(new_owner, object_id);
                self.remove_from_roles(new_owner, object_id);
            }
        }
    }

    fn remove_from_selection(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(selection) = self.crew_selection.get_mut(&owner) {
            selection.deselect(object_id);
            if selection.is_empty() {
                self.crew_selection.remove(&owner);
            }
        }
    }

    fn remove_from_roles(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(assignments) = self.crew_roles.get_mut(&owner) {
            assignments.remove(&object_id);
            if assignments.is_empty() {
                self.crew_roles.remove(&owner);
            }
        }
    }

    fn prune_selection(&mut self) {
        self.prune_roles();
        if self.crew_selection.is_empty() {
            return;
        }

        let alive: HashSet<ObjectId> = self
            .objects
            .iter()
            .filter(|object| object.state.crew_member && object.state.status.is_active())
            .map(|object| object.id)
            .collect();
        self.crew_selection.retain(|_, selection| {
            selection.prune(&alive);
            !selection.is_empty()
        });
    }

    fn prune_roles(&mut self) {
        if self.crew_roles.is_empty() {
            return;
        }

        let mut valid = HashMap::new();
        for object in &self.objects {
            if object.state.crew_member && object.state.status.is_active() {
                valid.insert(object.id, object.state.owner);
            }
        }

        self.crew_roles.retain(|owner, assignments| {
            assignments.retain(|object_id, _| match valid.get(object_id) {
                Some(current_owner) if *current_owner == *owner => true,
                _ => false,
            });
            !assignments.is_empty()
        });
    }

    fn resolve_command_targets(&self, owner: i32, target: &CrewCommandTarget) -> Vec<ObjectId> {
        match target {
            CrewCommandTarget::Cursor => self.crew_cursor(owner).into_iter().collect(),
            CrewCommandTarget::Selection => self.selected_crew(owner),
            CrewCommandTarget::Role(role) => self
                .crew_roles
                .get(&owner)
                .map(|assignments| {
                    assignments
                        .iter()
                        .filter_map(|(&object_id, assigned)| {
                            if assigned == role {
                                Some(object_id)
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn refresh_elimination_state(&mut self) {
        if self.objects.is_empty() && self.known_crew_owners.is_empty() {
            return;
        }

        let mut active = HashSet::new();
        for object in &self.objects {
            if !object.state.crew_member {
                continue;
            }
            let owner = object.state.owner;
            if owner == OWNER_NONE {
                continue;
            }
            self.known_crew_owners.insert(owner);
            if !object.state.status.is_active() {
                continue;
            }
            active.insert(owner);
            self.eliminated_crew_owners.remove(&owner);
        }

        let known: Vec<i32> = self.known_crew_owners.iter().cloned().collect();
        for owner in known {
            if !active.contains(&owner) {
                self.eliminated_crew_owners.insert(owner);
            }
        }
    }

    fn apply_physics_at_index(&mut self, idx: usize) {
        if idx >= self.objects.len() {
            return;
        }

        let (procedure, gravity_component) = {
            let object = &self.objects[idx];
            let procedure = self
                .definitions
                .get(&object.definition_id)
                .map(|definition| {
                    definition
                        .action_library()
                        .procedure_for_action(&object.state.action.name)
                })
                .unwrap_or_default();
            let gravity = procedure.gravity_component(self.physics.gravity);
            (procedure, gravity)
        };

        let object = &mut self.objects[idx];
        object.state.velocity.y = object.state.velocity.y.saturating_add(gravity_component);
        if procedure.allows_wind() {
            self.environment
                .apply_to_velocity(&mut object.state.velocity, self.frame);
        }
        if procedure.locks_vertical_velocity() {
            object.state.velocity.y = 0;
        }
        self.physics.clamp_velocity(&mut object.state.velocity);
    }

    fn apply_landscape_at_index(&mut self, idx: usize) {
        let resolution = match self.landscape.as_ref() {
            Some(landscape) => {
                let (position, velocity) = {
                    let object = &self.objects[idx];
                    (object.state.position, object.state.velocity)
                };
                landscape.resolve_collision(position, velocity)
            }
            None => return,
        };
        if !resolution.collided {
            return;
        }
        let object = &mut self.objects[idx];
        object.state.position = resolution.position;
        object.state.velocity = resolution.velocity;
    }

    fn next_random_i32(&mut self) -> i32 {
        self.rng.gen()
    }

    fn next_object_id(&mut self) -> ObjectId {
        let id = self.next_object_id;
        self.next_object_id += 1;
        ObjectId::new(id)
    }

    fn find_object_index(&self, id: ObjectId) -> Option<usize> {
        self.objects.iter().position(|object| object.id == id)
    }

    fn is_container_cycle(&self, object_id: ObjectId, container_id: ObjectId) -> bool {
        let mut current = Some(container_id);
        while let Some(id) = current {
            if id == object_id {
                return true;
            }
            current = self
                .objects
                .iter()
                .find(|object| object.id == id)
                .and_then(|object| object.state.container);
        }
        false
    }

    fn apply_container_change(
        &mut self,
        object_id: ObjectId,
        previous: Option<ObjectId>,
        new: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        if previous == new {
            return Ok(());
        }

        if let Some(prev_id) = previous {
            if let Some(prev_index) = self.find_object_index(prev_id) {
                let contents = &mut self.objects[prev_index].state.contents;
                contents.retain(|&child| child != object_id);
            }
        }

        let object_index = match self.find_object_index(object_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(object_id)),
        };

        match new {
            Some(container_id) => {
                if container_id == object_id {
                    return Err(EngineError::Container {
                        object: object_id,
                        detail: "object cannot contain itself".into(),
                    });
                }
                let container_index = match self.find_object_index(container_id) {
                    Some(index) => index,
                    None => return Err(EngineError::UnknownObject(container_id)),
                };
                let container = &self.objects[container_index];
                if container.destroyed || matches!(container.state.status, ObjectStatus::Deleted) {
                    return Err(EngineError::Container {
                        object: object_id,
                        detail: format!("container {} is destroyed", container_id),
                    });
                }
                if self.is_container_cycle(object_id, container_id) {
                    return Err(EngineError::Container {
                        object: object_id,
                        detail: format!("container {} would create a cycle", container_id),
                    });
                }

                let contents = &mut self.objects[container_index].state.contents;
                if !contents.contains(&object_id) {
                    contents.push(object_id);
                    contents.sort_by_key(|id| id.as_u64());
                }

                self.objects[object_index].state.container = Some(container_id);
            }
            None => {
                self.objects[object_index].state.container = None;
            }
        }

        Ok(())
    }

    fn detach_destroyed_objects(&mut self) -> Result<(), EngineError> {
        let mut updates = Vec::new();
        for object in &self.objects {
            if (object.destroyed || matches!(object.state.status, ObjectStatus::Deleted))
                && object.state.container.is_some()
            {
                updates.push((object.id, object.state.container));
            }

            if object.destroyed || matches!(object.state.status, ObjectStatus::Deleted) {
                for child in &object.state.contents {
                    updates.push((*child, Some(object.id)));
                }
            }
        }

        for (object_id, previous) in updates {
            self.apply_container_change(object_id, previous, None)?;
        }

        for object in &mut self.objects {
            if object.destroyed || matches!(object.state.status, ObjectStatus::Deleted) {
                object.state.contents.clear();
            }
        }

        Ok(())
    }

    fn apply_global_effect_commands(&mut self, commands: &[EffectCommand]) {
        apply_effect_commands_to_stack(&mut self.global_effects, commands);
    }

    fn tick_global_effects(&mut self) {
        for effect in &mut self.global_effects {
            effect.advance_tick();
        }
    }

    fn spawn_single(
        &mut self,
        config: SpawnConfig,
    ) -> Result<(ObjectId, Vec<SpawnConfig>), EngineError> {
        let SpawnConfig {
            definition_id,
            position,
            velocity,
            energy,
            action,
            effects,
            owner,
            crew_member,
            status,
            container,
        } = config;

        let definition_ref = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition_ref.action_library().clone();
        let mut initial_action = match action {
            Some(state) => state,
            None => definition_ref.default_action_state(),
        };
        initial_action.reconcile_with_library(&action_library);
        let initial_crew_member = crew_member.unwrap_or_else(|| definition_ref.is_crew());

        let id = self.next_object_id();
        let mut object = Object::new(
            id,
            definition_id.clone(),
            ObjectState {
                position,
                velocity,
                energy,
                action: initial_action,
                effects: Vec::new(),
                container: None,
                contents: Vec::new(),
                status: status.unwrap_or_default(),
                owner,
                crew_member: initial_crew_member,
            },
        );
        let mut container_changes = Vec::new();
        if let Some(container_id) = container {
            object.state.container = Some(container_id);
            container_changes.push((None, Some(container_id)));
        }

        let mut effect_events = Vec::new();
        if !effects.is_empty() {
            let commands: Vec<_> = effects.into_iter().map(EffectCommand::Add).collect();
            let mut initial_events = object.apply_effect_commands(&commands);
            effect_events.append(&mut initial_events);
        }

        self.physics.clamp_velocity(&mut object.state.velocity);

        let mut additional_spawns = Vec::new();
        if self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.has_initialize)
            .unwrap_or(false)
        {
            let random = self.next_random_i32();
            let rng_state = self.rng.clone();
            let (
                CommandBatch {
                    delta,
                    spawns,
                    destroy,
                    commands,
                    effects,
                    global_effects,
                },
                new_rng,
            ) = {
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .expect("definition must exist");
                definition.call_initialize(
                    &object.state,
                    id,
                    random,
                    rng_state,
                    &self.global_effects,
                )?
            };
            self.rng = new_rng;
            if destroy {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition_id.clone(),
                    function: "Initialize",
                    detail: "Initialize may not destroy the object".into(),
                });
            }
            if let Some(change) = object.state.apply_delta(&delta, &action_library) {
                container_changes.push(change);
            }
            let mut applied = object.apply_effect_commands(&effects);
            effect_events.append(&mut applied);
            if !global_effects.is_empty() {
                self.apply_global_effect_commands(&global_effects);
            }
            self.physics.clamp_velocity(&mut object.state.velocity);
            if !commands.is_empty() {
                object.enqueue_commands(commands);
            }
            additional_spawns = spawns;
        }

        if !effect_events.is_empty() {
            let definition = self
                .definitions
                .get(&definition_id)
                .expect("definition must exist");
            let global_view = self.global_effects.clone();
            let previous_container = object.state.container;
            let rng_state = self.rng.clone();
            let (global_cmds, new_rng) = Self::run_effect_events_for_object(
                definition,
                rng_state,
                id,
                &mut object,
                effect_events,
                global_view,
            )?;
            self.rng = new_rng;
            if !global_cmds.is_empty() {
                self.apply_global_effect_commands(&global_cmds);
            }
            if previous_container != object.state.container {
                container_changes.push((previous_container, object.state.container));
            }
        }

        self.apply_landscape(&mut object.state);
        self.objects.push(object);
        for (previous, new) in container_changes {
            self.apply_container_change(id, previous, new)?;
        }
        Ok((id, additional_spawns))
    }

    fn process_spawn_queue(
        &mut self,
        queue: Vec<SpawnConfig>,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let mut pending: VecDeque<_> = queue.into_iter().collect();
        let mut created = Vec::new();
        while let Some(config) = pending.pop_front() {
            let (id, additional) = self.spawn_single(config)?;
            created.push(id);
            for spawn in additional {
                pending.push_back(spawn);
            }
        }
        Ok(created)
    }
}

fn build_state_value(
    definition_id: &str,
    object_id: ObjectId,
    state: &ObjectState,
    library: &ActionLibrary,
) -> Value {
    let mut map = HashMap::with_capacity(8);
    map.insert(
        "definition".into(),
        Value::String(definition_id.to_string()),
    );
    map.insert("id".into(), Value::Int(truncate_to_i32(object_id.as_u64())));
    map.insert("position".into(), state.position.to_value());
    map.insert("velocity".into(), state.velocity.to_value());
    map.insert("energy".into(), Value::Int(state.energy));
    map.insert("owner".into(), Value::Int(state.owner));
    map.insert("crew_member".into(), Value::Bool(state.crew_member));
    map.insert("status".into(), Value::Int(state.status.to_script_value()));
    match state.container {
        Some(container) => {
            map.insert(
                "container".into(),
                Value::Int(truncate_to_i32(container.as_u64())),
            );
        }
        None => {
            map.insert("container".into(), Value::Nil);
        }
    }
    let contents: Vec<_> = state
        .contents
        .iter()
        .map(|id| Value::Int(truncate_to_i32(id.as_u64())))
        .collect();
    map.insert("contents".into(), Value::Array(contents));
    let mut action = HashMap::with_capacity(4);
    action.insert("name".into(), Value::String(state.action.name.clone()));
    action.insert("phase".into(), Value::Int(state.action.phase));
    let ticks = (state.action.ticks).min(i32::MAX as u32) as i32;
    action.insert("ticks".into(), Value::Int(ticks));
    if let Some(procedure) = library.procedure_name_for_action(&state.action.name) {
        action.insert("procedure".into(), Value::String(procedure.to_string()));
    }
    map.insert("action".into(), Value::Proplist(action));
    let effects: Vec<_> = state
        .effects
        .iter()
        .map(|effect| {
            let mut props = HashMap::with_capacity(6);
            props.insert("name".into(), Value::String(effect.name.clone()));
            props.insert("priority".into(), Value::Int(effect.priority));
            props.insert("interval".into(), Value::Int(effect.interval));
            props.insert("timer".into(), Value::Int(effect.timer));
            if let Some(target) = effect.command_target {
                props.insert("command_target".into(), Value::Int(target));
            }
            if let Some(id) = &effect.command_id {
                props.insert("command_target_id".into(), Value::String(id.clone()));
            }
            Value::Proplist(props)
        })
        .collect();
    map.insert("effects".into(), Value::Array(effects));
    Value::Proplist(map)
}

fn build_effect_value(effect: &EffectState) -> Value {
    let mut props = HashMap::with_capacity(6);
    props.insert("name".into(), Value::String(effect.name.clone()));
    props.insert("priority".into(), Value::Int(effect.priority));
    props.insert("interval".into(), Value::Int(effect.interval));
    props.insert("timer".into(), Value::Int(effect.timer));
    if let Some(target) = effect.command_target {
        props.insert("command_target".into(), Value::Int(target));
    }
    if let Some(id) = &effect.command_id {
        props.insert("command_target_id".into(), Value::String(id.clone()));
    }
    Value::Proplist(props)
}

fn effect_stop_reason_value(reason: EffectStopReason) -> Value {
    let label = match reason {
        EffectStopReason::Removed => "removed",
        EffectStopReason::Cleared => "cleared",
        EffectStopReason::Destroyed => "destroyed",
        EffectStopReason::Replaced => "replaced",
    };
    Value::String(label.to_string())
}

fn apply_effect_commands_to_stack(target: &mut Vec<EffectState>, commands: &[EffectCommand]) {
    for command in commands {
        match command {
            EffectCommand::Add(effect) => insert_effect_into_stack(target, effect.clone()),
            EffectCommand::Remove { name } => {
                if let Some(index) = target.iter().position(|existing| &existing.name == name) {
                    target.remove(index);
                }
            }
            EffectCommand::Clear => target.clear(),
        }
    }
}

fn insert_effect_into_stack(stack: &mut Vec<EffectState>, mut effect: EffectState) {
    if effect.interval <= 0 {
        effect.interval = 1;
    }
    if effect.timer < 0 {
        effect.timer = 0;
    }
    if effect.interval > 0 && effect.timer >= effect.interval {
        effect.timer %= effect.interval;
    }

    if let Some(index) = stack
        .iter()
        .position(|existing| existing.name == effect.name)
    {
        stack.remove(index);
    }

    let mut insert_pos = 0;
    while insert_pos < stack.len() && stack[insert_pos].priority > effect.priority {
        insert_pos += 1;
    }

    stack.insert(insert_pos, effect);
}

fn truncate_to_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn parse_command(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<CommandBatch, EngineError> {
    match value {
        Value::Nil => Ok(CommandBatch::default()),
        Value::Proplist(map) => parse_command_from_proplist(definition, function, map),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected proplist or nil, got {}", other.type_name()),
        }),
    }
}

fn parse_command_from_proplist(
    definition: &str,
    function: &'static str,
    map: HashMap<String, Value>,
) -> Result<CommandBatch, EngineError> {
    let mut batch = CommandBatch::default();
    for (key, value) in map.into_iter() {
        match key.as_str() {
            "position" => {
                batch.delta.position = Some(value_to_vector(definition, function, value)?);
            }
            "velocity" => {
                batch.delta.velocity = Some(value_to_vector(definition, function, value)?);
            }
            "energy" => {
                batch.delta.energy = Some(value_to_int(definition, function, value)?);
            }
            "owner" => {
                batch.delta.owner = Some(value_to_int(definition, function, value)?);
            }
            "action" => {
                let update = value_to_action(definition, function, value)?;
                if let Some(update) = update {
                    ensure_action_delta(&mut batch.delta).merge(update);
                }
            }
            "action_phase" => {
                let phase = value_to_int(definition, function, value)?;
                ensure_action_delta(&mut batch.delta).set_phase(phase);
            }
            "destroy" => {
                batch.destroy = value.as_bool();
            }
            "spawn" => {
                batch
                    .spawns
                    .extend(value_to_spawns(definition, function, value)?);
            }
            "commands" => {
                batch
                    .commands
                    .extend(value_to_commands(definition, function, value)?);
            }
            "effects" => {
                batch
                    .effects
                    .extend(value_to_effect_commands(definition, function, value)?);
            }
            "global_effects" => {
                batch
                    .global_effects
                    .extend(value_to_effect_commands(definition, function, value)?);
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("unexpected key `{other}`"),
                });
            }
        }
    }
    Ok(batch)
}

fn ensure_action_delta(delta: &mut ObjectDelta) -> &mut ActionUpdate {
    delta.action.get_or_insert_with(ActionUpdate::default)
}

fn ensure_action_update(update: &mut ObjectUpdate) -> &mut ActionUpdate {
    update.action.get_or_insert_with(ActionUpdate::default)
}

fn value_to_action(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Option<ActionUpdate>, EngineError> {
    match value {
        Value::Nil => Ok(None),
        Value::String(name) => Ok(Some(ActionUpdate::default().with_name(name))),
        Value::Proplist(map) => parse_action_update(definition, function, map).map(Some),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!(
                "expected string, proplist, or nil for action, got {}",
                other.type_name()
            ),
        }),
    }
}

fn parse_action_update(
    definition: &str,
    function: &'static str,
    map: HashMap<String, Value>,
) -> Result<ActionUpdate, EngineError> {
    let mut update = ActionUpdate::default();
    for (key, value) in map.into_iter() {
        match key.as_str() {
            "name" => match value {
                Value::String(name) => update.set_name(name),
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!(
                            "expected string for action.name, got {}",
                            other.type_name()
                        ),
                    })
                }
            },
            "phase" => {
                let phase = value_to_int(definition, function, value)?;
                update.set_phase(phase);
            }
            "ticks" => {
                let ticks = value_to_int(definition, function, value)?;
                if ticks < 0 {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: "action.ticks must be >= 0".to_string(),
                    });
                }
                update.set_ticks(ticks as u32);
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("unexpected key `{other}` in action proplist"),
                });
            }
        }
    }
    Ok(update)
}

fn value_to_vector(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vector2, EngineError> {
    match value {
        Value::Array(values) if values.len() == 2 => {
            let x = match &values[0] {
                Value::Int(v) => *v,
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("expected int for x component, got {}", other.type_name()),
                    })
                }
            };
            let y = match &values[1] {
                Value::Int(v) => *v,
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("expected int for y component, got {}", other.type_name()),
                    })
                }
            };
            Ok(Vector2::new(x, y))
        }
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected array[2], got {}", other.type_name()),
        }),
    }
}

fn value_to_int(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<i32, EngineError> {
    match value {
        Value::Int(v) => Ok(v),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected int, got {}", other.type_name()),
        }),
    }
}

fn value_to_bool(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<bool, EngineError> {
    match value {
        Value::Bool(v) => Ok(v),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected bool, got {}", other.type_name()),
        }),
    }
}

fn value_to_spawns(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vec<SpawnConfig>, EngineError> {
    let array = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!("expected array for spawn list, got {}", other.type_name()),
            })
        }
    };

    let mut spawns = Vec::with_capacity(array.len());
    for entry in array.into_iter() {
        let map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("spawn entry must be proplist, got {}", other.type_name()),
                })
            }
        };

        let definition_id = match map.get("definition") {
            Some(Value::String(id)) => id.clone(),
            Some(other) => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("spawn definition must be string, got {}", other.type_name()),
                })
            }
            None => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: "spawn entry missing `definition`".into(),
                })
            }
        };

        let position = match map.get("position") {
            Some(value) => value_to_vector(definition, function, value.clone())?,
            None => Vector2::ZERO,
        };
        let velocity = match map.get("velocity") {
            Some(value) => value_to_vector(definition, function, value.clone())?,
            None => Vector2::ZERO,
        };
        let energy = match map.get("energy") {
            Some(value) => value_to_int(definition, function, value.clone())?,
            None => 0,
        };
        let owner = match map.get("owner") {
            Some(value) => value_to_int(definition, function, value.clone())?,
            None => OWNER_NONE,
        };

        let mut action_state = ActionState::default();
        if let Some(value) = map.get("action") {
            if let Some(update) = value_to_action(definition, function, value.clone())? {
                action_state.apply_update(&update);
            }
        }
        if let Some(value) = map.get("action_phase") {
            let phase = value_to_int(definition, function, value.clone())?;
            let mut update = ActionUpdate::default();
            update.set_phase(phase);
            action_state.apply_update(&update);
        }

        let action_override = if action_state == ActionState::default() {
            None
        } else {
            Some(action_state)
        };

        let crew_member = match map.get("crew_member") {
            Some(value) => Some(value_to_bool(definition, function, value.clone())?),
            None => None,
        };

        let mut spawn = SpawnConfig::new(definition_id.clone())
            .with_position(position)
            .with_velocity(velocity)
            .with_energy(energy)
            .with_owner(owner);

        if let Some(action_state) = action_override {
            spawn = spawn.with_action(action_state);
        }

        if let Some(crew_member) = crew_member {
            spawn = spawn.with_crew_member(crew_member);
        }

        spawns.push(spawn);
    }

    Ok(spawns)
}

fn value_to_commands(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vec<QueuedCommand>, EngineError> {
    let array = match value {
        Value::Array(values) => values,
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!("expected array for commands, got {}", other.type_name()),
            })
        }
    };

    let mut commands = Vec::with_capacity(array.len());
    for value in array {
        let map = match value {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!(
                        "expected proplist for command entry, got {}",
                        other.type_name()
                    ),
                })
            }
        };

        let mut delay: Option<u32> = None;
        let mut update = ObjectUpdate::default();
        let mut effects = Vec::new();
        let mut destroy = false;
        let mut spawns = Vec::new();

        for (key, value) in map.into_iter() {
            match key.as_str() {
                "delay" => {
                    let raw_delay = value_to_int(definition, function, value)?;
                    if raw_delay < 0 {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "delay must be >= 0".into(),
                        });
                    }
                    delay = Some(raw_delay as u32);
                }
                "position" => {
                    update.position = Some(value_to_vector(definition, function, value)?);
                }
                "velocity" => {
                    update.velocity = Some(value_to_vector(definition, function, value)?);
                }
                "energy" => {
                    update.energy = Some(value_to_int(definition, function, value)?);
                }
                "action" => {
                    if let Some(action) = value_to_action(definition, function, value)? {
                        ensure_action_update(&mut update).merge(action);
                    }
                }
                "action_phase" => {
                    let phase = value_to_int(definition, function, value)?;
                    ensure_action_update(&mut update).set_phase(phase);
                }
                "owner" => {
                    update.owner = Some(value_to_int(definition, function, value)?);
                }
                "effects" => {
                    effects.extend(value_to_effect_commands(definition, function, value)?);
                }
                "destroy" => {
                    destroy = value_to_bool(definition, function, value)?;
                }
                "spawn" => {
                    spawns.extend(value_to_spawns(definition, function, value)?);
                }
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("unexpected key `{other}` in command entry"),
                    });
                }
            }
        }

        commands.push(
            QueuedCommand::new(delay.unwrap_or(0), update)
                .with_effects(effects)
                .with_spawns(spawns)
                .with_destroy(destroy),
        );
    }

    Ok(commands)
}

fn value_to_effect_commands(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vec<EffectCommand>, EngineError> {
    let entries = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!("expected array for effects, got {}", other.type_name()),
            })
        }
    };

    let mut commands = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("effect entry must be proplist, got {}", other.type_name()),
                })
            }
        };

        let op = match map.remove("op") {
            Some(Value::String(op)) => op,
            Some(other) => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("effects.op must be string, got {}", other.type_name()),
                })
            }
            None => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: "effect entry missing `op`".into(),
                })
            }
        };

        match op.as_str() {
            "add" => {
                let name_value =
                    map.remove("name")
                        .ok_or_else(|| EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "effect add entry missing `name`".into(),
                        })?;
                let name = match name_value {
                    Value::String(name) => name,
                    other => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: format!(
                                "effect name must be string, got {}",
                                other.type_name()
                            ),
                        })
                    }
                };

                let priority = match map.remove("priority") {
                    Some(value) => value_to_int(definition, function, value)?,
                    None => 100,
                };

                let interval = match map.remove("interval") {
                    Some(value) => {
                        let interval = value_to_int(definition, function, value)?;
                        if interval <= 0 {
                            return Err(EngineError::InvalidScriptOutput {
                                definition: definition.to_string(),
                                function,
                                detail: "effect interval must be > 0".into(),
                            });
                        }
                        interval
                    }
                    None => 1,
                };

                let timer = match map.remove("timer") {
                    Some(value) => {
                        let timer = value_to_int(definition, function, value)?;
                        if timer < 0 {
                            return Err(EngineError::InvalidScriptOutput {
                                definition: definition.to_string(),
                                function,
                                detail: "effect timer must be >= 0".into(),
                            });
                        }
                        timer
                    }
                    None => 0,
                };

                let command_target = match map.remove("command_target") {
                    Some(Value::Int(value)) => Some(value),
                    Some(Value::Nil) | None => None,
                    Some(other) => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: format!(
                                "effect command_target must be int or nil, got {}",
                                other.type_name()
                            ),
                        })
                    }
                };

                let command_target_id = match map.remove("command_target_id") {
                    Some(Value::String(value)) if !value.is_empty() => Some(value),
                    Some(Value::String(_)) | Some(Value::Nil) | None => None,
                    Some(other) => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: format!(
                                "effect command_target_id must be string or nil, got {}",
                                other.type_name()
                            ),
                        })
                    }
                };

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("unexpected key `{}` in effect add entry", key),
                    });
                }

                let effect = EffectState::new(name)
                    .with_priority(priority)
                    .with_interval(interval)
                    .with_timer(timer)
                    .with_command_target(command_target)
                    .with_command_id(command_target_id);
                commands.push(EffectCommand::add(effect));
            }
            "remove" => {
                let name_value =
                    map.remove("name")
                        .ok_or_else(|| EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "effect remove entry missing `name`".into(),
                        })?;
                let name = match name_value {
                    Value::String(name) => name,
                    other => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: format!(
                                "effect name must be string, got {}",
                                other.type_name()
                            ),
                        })
                    }
                };
                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("unexpected key `{}` in effect remove entry", key),
                    });
                }
                commands.push(EffectCommand::remove(name));
            }
            "clear" => {
                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("unexpected key `{}` in effect clear entry", key),
                    });
                }
                commands.push(EffectCommand::Clear);
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("unsupported effect op `{}`", other),
                });
            }
        }
    }

    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::NamedTempFile;

    const STATEFUL_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        var vx = state.velocity[0] + (random % 5);
        var phase = random % 3;
        return {
            velocity = [vx, state.velocity[1]],
            energy = state.energy + (random % 7),
            action = { name = "Active", phase = phase }
        };
    }

    global func Step(state, frame, random)
    {
        var vx = state.velocity[0] + (random % 3) - 1;
        var energy = state.energy + (random % 5) - 2;
        if (energy < 0)
        {
            energy = 0;
        }
        return {
            velocity = [vx, state.velocity[1]],
            energy = energy
        };
    }
    "#;

    const EFFECT_HOST_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        if (!GetEffect("Glow", state))
        {
            return { effects = [ { op = "add", name = "Glow", priority = 150, interval = 4 } ] };
        }
        return nil;
    }

    global func Step(state, frame, random)
    {
        if (frame == 1)
        {
            return { effects = [ { op = "add", name = "Spark", priority = 60 } ] };
        }
        if (frame == 2)
        {
            var glow_number = GetEffect("Glow", state);
            var glow_priority = GetEffect("Glow", state, 0, 2);
            var spark_priority = GetEffect("Spark", state, 0, 2);
            var interval = GetEffect("Glow", state, 0, 3);
            var filtered = GetEffect("Glow", state, 0, 2, 100);
            var allowed = GetEffect("Glow", state, 0, 2, 200);
            if (filtered)
            {
                return { energy = -1 };
            }
            return { energy = glow_number + glow_priority + spark_priority + interval + allowed };
        }
        return nil;
    }
    "#;

    const EFFECT_HOST_ADD_REMOVE_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        AddEffect("Glow", state, 120, 3);
        AddEffect("Spark", state);
        return nil;
    }

    global func Step(state, frame, random)
    {
        if (frame == 1)
        {
            RemoveEffect("Glow", state);
        }
        if (frame == 2)
        {
            var spark_id = GetEffect("Spark", state);
            if (spark_id)
            {
                RemoveEffect(nil, state, spark_id);
            }
        }
        return nil;
    }
    "#;

    const GLOBAL_EFFECT_HELPER_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        AddEffect("WorldPulse", nil, 80, 3);
        return nil;
    }

    global func Step(state, frame, random)
    {
        if (frame == 1)
        {
            RemoveEffect("WorldPulse", nil);
        }
        return nil;
    }
    "#;

    const PROCEDURE_STATE_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        if (state.action && state.action.procedure == "flight")
        {
            return { energy = 7 };
        }
        return { energy = -1 };
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    const RANDOM_HELPER_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        return nil;
    }

    global func Step(state, frame, random)
    {
        return { energy = Random(10) };
    }
    "#;

    const PROCEDURE_MOVEMENT_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        return nil;
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    fn build_definition() -> Definition {
        let source = r#"
        global func Initialize(state, random) {
            return { energy = 100 };
        }

        global func Step(state, frame, random) {
            var vx = state.velocity[0] + 1;
            return { velocity = [vx, state.velocity[1]] };
        }
        "#;
        Definition::from_script("Test", "Test", source).expect("script compiles")
    }

    #[test]
    fn advances_actions_using_definition_map() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_length(2).with_next("Idle"),
        );
        actions.insert("Idle".to_string(), ActionSpec::default().with_length(1));
        definition.configure_actions(Some("Walk".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Walk");
        assert_eq!(object.action.phase, 1);
        assert_eq!(object.action.ticks, 0);

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Idle");
        assert_eq!(object.action.phase, 0);
        assert_eq!(object.action.ticks, 0);
    }

    #[test]
    fn action_delay_requires_multiple_ticks() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert(
            "Loop".to_string(),
            ActionSpec::default().with_length(3).with_delay(2),
        );
        definition.configure_actions(Some("Loop".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        let initial = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(initial.action.phase, 0);
        assert_eq!(initial.action.ticks, 0);

        let after_first = engine.tick().expect("first tick succeeds");
        let object = after_first.object(id).expect("object present");
        assert_eq!(object.action.phase, 0);
        assert_eq!(object.action.ticks, 1);

        let after_second = engine.tick().expect("second tick succeeds");
        let object = after_second.object(id).expect("object present");
        assert_eq!(object.action.phase, 1);
        assert_eq!(object.action.ticks, 0);

        let after_third = engine.tick().expect("third tick succeeds");
        let object = after_third.object(id).expect("object present");
        assert_eq!(object.action.phase, 1);
        assert_eq!(object.action.ticks, 1);

        let after_fourth = engine.tick().expect("fourth tick succeeds");
        let object = after_fourth.object(id).expect("object present");
        assert_eq!(object.action.phase, 2);
        assert_eq!(object.action.ticks, 0);
    }

    #[test]
    fn host_get_effect_queries_effect_stack() {
        let definition = Definition::from_script("EffectUser", "Effect User", EFFECT_HOST_SCRIPT)
            .expect("script compiles");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("EffectUser"))
            .expect("spawn succeeds");

        engine.tick().expect("first tick runs");
        engine.tick().expect("second tick runs");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.effects.len(), 2);
        assert_eq!(snapshot.effects[0].name, "Glow");
        assert_eq!(snapshot.effects[0].priority, 150);
        assert_eq!(snapshot.effects[1].name, "Spark");
        assert_eq!(snapshot.effects[1].priority, 60);
        assert_eq!(snapshot.energy, 365);
    }

    #[test]
    fn host_add_effect_and_remove_effect_via_helpers() {
        let definition = Definition::from_script(
            "EffectBridge",
            "Effect Bridge",
            EFFECT_HOST_ADD_REMOVE_SCRIPT,
        )
        .expect("script compiles");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("EffectBridge"))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.effects.len(), 2);
        assert_eq!(snapshot.effects[0].name, "Glow");
        assert_eq!(snapshot.effects[1].name, "Spark");

        let first_tick = engine.tick().expect("first tick succeeds");
        let object = first_tick.object(id).expect("object present");
        assert_eq!(object.effects.len(), 1);
        assert_eq!(object.effects[0].name, "Spark");

        let second_tick = engine.tick().expect("second tick succeeds");
        let object = second_tick.object(id).expect("object present");
        assert!(object.effects.is_empty());
    }

    #[test]
    fn host_helpers_modify_global_effects() {
        let definition =
            Definition::from_script("GlobalEffect", "Global Effect", GLOBAL_EFFECT_HELPER_SCRIPT)
                .expect("script compiles");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine
            .spawn_object(SpawnConfig::new("GlobalEffect"))
            .expect("spawn succeeds");

        assert_eq!(engine.global_effects().len(), 1);
        assert_eq!(engine.global_effects()[0].name, "WorldPulse");

        engine.tick().expect("first tick succeeds");

        assert!(engine.global_effects().is_empty());
    }

    #[test]
    fn inactive_objects_skip_physics_and_step() {
        let mut definition = build_definition();
        definition.configure_actions(Some("Idle".to_string()), HashMap::new());

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_velocity(Vector2::new(3, 0))
                    .with_energy(50),
            )
            .expect("spawn succeeds");

        engine.tick().expect("initial tick runs");

        engine
            .apply_object_update(id, ObjectUpdate::new().with_status(ObjectStatus::Inactive))
            .expect("status update applies");

        let before = engine
            .object_snapshot(id)
            .expect("snapshot available before tick");

        engine.tick().expect("tick with inactive object runs");

        let after = engine
            .object_snapshot(id)
            .expect("snapshot available after tick");

        assert_eq!(after.velocity, before.velocity);
        assert_eq!(after.position, before.position);
        assert_eq!(after.energy, before.energy);
        assert_eq!(after.status, ObjectStatus::Inactive);
    }

    #[test]
    fn engine_state_persists_object_status() {
        let mut definition = build_definition();
        definition.configure_actions(Some("Idle".to_string()), HashMap::new());

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_status(ObjectStatus::Inactive)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .apply_object_update(id, ObjectUpdate::new().with_status(ObjectStatus::Inactive))
            .expect("status update applies");

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored.restore_state(&state).expect("state restores");

        let snapshot = restored
            .object_snapshot(id)
            .expect("restored object available");
        assert_eq!(snapshot.status, ObjectStatus::Inactive);
        assert!(restored.crew_members(1).is_empty());
        assert!(restored.is_owner_eliminated(1));
    }

    #[test]
    fn host_random_consumes_engine_rng() {
        let definition = Definition::from_script("RandomUser", "Random User", RANDOM_HELPER_SCRIPT)
            .expect("script compiles");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("RandomUser"))
            .expect("spawn succeeds");

        let mut expected_rng = ChaCha8Rng::seed_from_u64(0);
        let _ = expected_rng.gen::<i32>(); // Initialize random draw
        let _ = expected_rng.gen::<i32>(); // First tick random parameter
        let first_expected = expected_rng.gen_range(0..10);

        let first_tick = engine.tick().expect("first tick succeeds");
        let object = first_tick.object(id).expect("object present");
        assert_eq!(object.energy, first_expected);

        let _ = expected_rng.gen::<i32>(); // Second tick random parameter
        let second_expected = expected_rng.gen_range(0..10);

        let second_tick = engine.tick().expect("second tick succeeds");
        let object = second_tick.object(id).expect("object present");
        assert_eq!(object.energy, second_expected);
    }

    #[test]
    fn action_procedure_surfaces_in_state_value() {
        let mut definition =
            Definition::from_script("Airborne", "Airborne", PROCEDURE_STATE_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Fly".to_string(),
            ActionSpec::default().with_procedure("flight"),
        );
        definition.configure_actions(Some("Fly".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Airborne"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot available");
        assert_eq!(snapshot.energy, 7);
    }

    #[test]
    fn snapshot_includes_action_procedure() {
        let mut definition =
            Definition::from_script("Airborne", "Airborne", PROCEDURE_STATE_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Fly".to_string(),
            ActionSpec::default().with_procedure("flight"),
        );
        definition.configure_actions(Some("Fly".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Airborne"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot available");
        assert_eq!(snapshot.action_procedure.as_deref(), Some("flight"));
    }

    #[test]
    fn flight_procedure_suppresses_gravity_and_wind() {
        let mut definition = Definition::from_script("Glider", "Glider", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Fly".to_string(),
            ActionSpec::default().with_procedure("flight"),
        );
        definition.configure_actions(Some("Fly".to_string()), actions);

        let mut engine = Engine::with_seed(1);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(4, 12, -20)
            .expect("physics settings valid")
            .with_max_horizontal_speed(24)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(5));

        let id = engine
            .spawn_object(SpawnConfig::new("Glider"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 0);
    }

    #[test]
    fn float_procedure_reduces_gravity_pull() {
        let mut definition =
            Definition::from_script("Balloon", "Balloon", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Float".to_string(),
            ActionSpec::default().with_procedure("float"),
        );
        definition.configure_actions(Some("Float".to_string()), actions);

        let mut engine = Engine::with_seed(2);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(6, 20, -30)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(SpawnConfig::new("Balloon"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 3);
    }

    #[test]
    fn applies_velocity_changes_from_step_callback() {
        let mut engine = Engine::with_seed(123);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0))
                    .with_energy(50),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(1, 1));
        assert_eq!(object.velocity, Vector2::new(2, 1));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(3, 3));
        assert_eq!(object.velocity, Vector2::new(3, 2));
    }

    #[test]
    fn applies_environment_wind_to_velocity() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(Definition::from_script("Drift", "Drift", script).unwrap())
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Drift")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        engine.set_environment(EnvironmentSettings::new(2));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(2, 1));
        assert_eq!(object.position, Vector2::new(2, 1));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(4, 2));
        assert_eq!(object.position, Vector2::new(6, 3));
    }

    #[test]
    fn wind_force_respects_variation_and_period() {
        let settings = EnvironmentSettings::new(2).with_wind_variation(4, 4);
        assert_eq!(settings.wind_force(0), 2);
        assert_eq!(settings.wind_force(1), 6);
        assert_eq!(settings.wind_force(2), 2);
        assert_eq!(settings.wind_force(3), -2);
        assert_eq!(settings.wind_force(4), 2);

        let default_period = EnvironmentSettings::new(1).with_wind_variation(3, 0);
        assert_eq!(default_period.wind_variation, 3);
        assert_eq!(default_period.wind_period, 2);
    }

    #[test]
    fn ambient_temperature_cycles_with_climate() {
        let settings = EnvironmentSettings::new(0)
            .with_temperature(10)
            .with_climate(5)
            .with_temperature_cycle(8, 12, 3);

        assert_eq!(settings.ambient_temperature(0), 15);
        assert_eq!(settings.ambient_temperature(3), 23);
        assert_eq!(settings.ambient_temperature(9), 7);
    }

    #[test]
    fn ambient_temperature_resets_when_cycle_disabled() {
        let mut settings = EnvironmentSettings::new(0)
            .with_temperature(5)
            .with_climate(-2)
            .with_temperature_cycle(10, 6, 0);
        assert_ne!(settings.ambient_temperature(1), 3);

        settings = settings.with_temperature_cycle(0, 6, 0);
        assert_eq!(settings.temperature_variation, 0);
        assert_eq!(settings.temperature_period, 0);
        assert_eq!(settings.temperature_phase, 0);
        assert_eq!(settings.ambient_temperature(1), 3);
    }

    #[test]
    fn snapshot_reports_environment_metrics() {
        let mut engine = Engine::with_seed(15);
        let environment = EnvironmentSettings::new(4)
            .with_wind_variation(6, 8)
            .with_temperature(12)
            .with_climate(-4)
            .with_temperature_cycle(6, 16, 5)
            .with_time_of_day(900)
            .with_time_speed(30);
        engine.set_environment(environment);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.environment.settings, environment);
        assert_eq!(snapshot.environment.wind_force, environment.wind_force(0));
        assert_eq!(
            snapshot.environment.ambient_temperature,
            environment.ambient_temperature(0)
        );
    }

    #[test]
    fn environment_time_advances_each_tick() {
        let mut engine = Engine::with_seed(7);
        engine.set_environment(
            EnvironmentSettings::new(0)
                .with_time_of_day(2300)
                .with_time_speed(75),
        );

        assert_eq!(engine.environment().time_of_day, 2300);

        engine.tick().expect("first tick succeeds");
        assert_eq!(engine.environment().time_of_day, 2375);

        engine.tick().expect("second tick succeeds");
        assert_eq!(engine.environment().time_of_day, 50);
    }

    #[test]
    fn spawn_object_tracks_owner() {
        let mut engine = Engine::with_seed(99);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_owner(2),
            )
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot available");
        assert_eq!(snapshot.owner, 2);
    }

    #[test]
    fn crew_members_enumerates_owned_crew() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew_owner_one = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let crew_owner_two = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(2))
            .expect("spawn succeeds");
        engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_owner(1)
                    .with_crew_member(false),
            )
            .expect("spawn succeeds");

        let mut owner_one_members = engine.crew_members(1);
        owner_one_members.sort_by_key(|id| id.as_u64());
        assert_eq!(owner_one_members, vec![crew_owner_one]);

        assert_eq!(engine.crew_members(2), vec![crew_owner_two]);
        assert!(engine.crew_members(3).is_empty());
    }

    #[test]
    fn select_crew_tracks_selection_and_cursor() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first])
            .expect("selection succeeds");

        assert_eq!(engine.selected_crew(1), vec![first]);
        assert_eq!(engine.crew_cursor(1), Some(first));

        engine
            .select_crew(1, vec![second])
            .expect("second selection succeeds");

        let mut selected = engine.selected_crew(1);
        selected.sort_by_key(|id| id.as_u64());
        assert_eq!(selected, vec![first, second]);
        assert_eq!(engine.crew_cursor(1), Some(first));
    }

    #[test]
    fn deselect_crew_updates_cursor() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first, second])
            .expect("selection succeeds");

        engine.deselect_crew(1, vec![first]);
        assert_eq!(engine.selected_crew(1), vec![second]);
        assert_eq!(engine.crew_cursor(1), Some(second));

        engine.deselect_crew(1, vec![second]);
        assert!(engine.selected_crew(1).is_empty());
        assert_eq!(engine.crew_cursor(1), None);
    }

    #[test]
    fn set_cursor_promotes_selection() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first])
            .expect("selection succeeds");

        engine
            .set_crew_cursor(1, Some(second))
            .expect("cursor assignment succeeds");

        let mut selected = engine.selected_crew(1);
        selected.sort_by_key(|id| id.as_u64());
        assert_eq!(selected, vec![first, second]);
        assert_eq!(engine.crew_cursor(1), Some(second));
    }

    #[test]
    fn select_crew_rejects_wrong_owner() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owned = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let other_owner = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(2))
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![owned])
            .expect("selection succeeds");

        let error = engine
            .select_crew(1, vec![other_owner])
            .expect_err("selection should fail");
        match error {
            EngineError::CrewSelection { owner, detail } => {
                assert_eq!(owner, 1);
                assert!(detail.contains("owned by"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn selection_pruned_after_object_destroyed() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![crew])
            .expect("selection succeeds");

        engine
            .queue_object_command(
                crew,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");

        engine.tick().expect("tick succeeds");

        assert!(engine.selected_crew(1).is_empty());
        assert_eq!(engine.crew_cursor(1), None);
    }

    #[test]
    fn crew_role_assignment_requires_valid_owner() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owned = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let other_owner = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(2))
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, owned, CrewRole::from("builder"))
            .expect("role assignment succeeds");

        let error = engine
            .set_crew_role(1, other_owner, CrewRole::from("builder"))
            .expect_err("assignment should fail");
        match error {
            EngineError::CrewRole { owner, detail } => {
                assert_eq!(owner, 1);
                assert!(detail.contains("owned"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn crew_roles_removed_when_object_destroyed() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, crew, CrewRole::from("scout"))
            .expect("role assignment succeeds");

        engine
            .queue_object_command(
                crew,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");

        engine.tick().expect("tick succeeds");

        assert!(engine.crew_role_assignments(1).is_empty());
    }

    #[test]
    fn apply_command_targets_role_assignments() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, first, CrewRole::from("builder"))
            .expect("role assignment succeeds");
        engine
            .set_crew_role(1, second, CrewRole::from("builder"))
            .expect("role assignment succeeds");

        engine
            .apply_command(
                1,
                CrewCommandTarget::role("builder"),
                ObjectUpdate::new().with_energy(42),
            )
            .expect("command routes");

        assert_eq!(engine.object_snapshot(first).unwrap().energy, 42);
        assert_eq!(engine.object_snapshot(second).unwrap().energy, 42);
    }

    #[test]
    fn capture_state_preserves_crew_roles() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, crew, CrewRole::from("pilot"))
            .expect("role assignment succeeds");

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(0);
        let mut restored_definition = build_definition();
        restored_definition.set_crew_member(true);
        restored
            .register_definition(restored_definition)
            .expect("definition registers");
        restored.restore_state(&state).expect("state restores");

        let assignments = restored.crew_role_assignments(1);
        assert_eq!(
            assignments.get(&crew).map(|role| role.as_str()),
            Some("pilot")
        );
    }

    #[test]
    fn crew_elimination_marks_owner_after_last_crew_destroyed() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owner_one = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        engine
            .spawn_object(SpawnConfig::new("Test").with_owner(2))
            .expect("spawn succeeds");

        assert!(engine.eliminated_owners().is_empty());

        engine
            .queue_object_command(
                owner_one,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");

        engine.tick().expect("tick succeeds");

        assert!(engine.is_owner_eliminated(1));
        assert_eq!(engine.eliminated_owners(), vec![1]);
        assert!(!engine.is_owner_eliminated(2));
    }

    #[test]
    fn crew_elimination_clears_when_new_crew_spawned() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owner = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .queue_object_command(
                owner,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");
        engine.tick().expect("tick succeeds");

        assert!(engine.is_owner_eliminated(1));

        engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        assert!(!engine.is_owner_eliminated(1));
        assert!(engine.eliminated_owners().is_empty());
    }

    #[test]
    fn capture_state_preserves_crew_selection() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first, second])
            .expect("selection succeeds");
        engine
            .set_crew_cursor(1, Some(second))
            .expect("cursor assignment succeeds");

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(5);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        restored
            .register_definition(definition)
            .expect("definition registers");
        restored.restore_state(&state).expect("restore succeeds");

        let mut restored_selected = restored.selected_crew(1);
        restored_selected.sort_by_key(|id| id.as_u64());
        assert_eq!(restored_selected, vec![first, second]);
        assert_eq!(restored.crew_cursor(1), Some(second));
    }

    #[test]
    fn capture_state_preserves_elimination_status() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let eliminated = engine
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");
        engine
            .spawn_object(SpawnConfig::new("Test").with_owner(2))
            .expect("spawn succeeds");

        engine
            .queue_object_command(
                eliminated,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");
        engine.tick().expect("tick succeeds");

        assert!(engine.is_owner_eliminated(1));
        assert!(!engine.is_owner_eliminated(2));

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(5);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        restored
            .register_definition(definition)
            .expect("definition registers");
        restored.restore_state(&state).expect("restore succeeds");

        assert!(restored.is_owner_eliminated(1));
        assert!(!restored.is_owner_eliminated(2));

        restored
            .spawn_object(SpawnConfig::new("Test").with_owner(1))
            .expect("spawn succeeds");

        assert!(!restored.is_owner_eliminated(1));
        assert!(restored.eliminated_owners().is_empty());
    }

    #[test]
    fn tracks_action_state_changes() {
        let source = r#"
        global func Initialize(state, random) {
            return { action = "Walk" };
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                return { action = { name = "Jump", phase = 3 } };
            }
            return nil;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        let mut definition =
            Definition::from_script("Actor", "Actor", source).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::default());
        actions.insert("Jump".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Walk".to_string()), actions);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.action.phase, 3);

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.action.phase, 4);
    }

    #[test]
    fn spawns_additional_objects_from_step() {
        let source = r#"
        global func Initialize(state, random) {
            return { energy = 42 };
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    spawn = [
                        { definition = "Test", position = [state.position[0] + 5, state.position[1]], velocity = [0, 0], energy = 10 }
                    ]
                };
            }
            return nil;
        }
        "#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(Definition::from_script("Test", "Test", source).unwrap())
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(0, 1));
        assert_eq!(object.energy, 42);
        assert_eq!(snapshot.objects.len(), 2, "spawned child should exist");

        let spawned = snapshot
            .objects
            .iter()
            .find(|obj| obj.id != id)
            .expect("child object present");
        assert_eq!(spawned.position, Vector2::new(5, 1));
        assert_eq!(spawned.energy, 42);
    }

    #[test]
    fn produces_deterministic_snapshots() {
        let source = r#"
        global func Step(state, frame, random) {
            var new_y = state.position[1] + (random % 3) - 1;
            return { velocity = [state.velocity[0], new_y - state.position[1]] };
        }
        "#;
        let definition = Definition::from_script("Mover", "Mover", source).unwrap();

        let mut engine_a = Engine::with_seed(7);
        engine_a
            .register_definition(definition)
            .expect("definition registers");
        let mut engine_b = Engine::with_seed(7);
        engine_b
            .register_definition(Definition::from_script("Mover", "Mover", source).unwrap())
            .expect("definition registers");

        let id_a = engine_a
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0)),
            )
            .unwrap();
        let id_b = engine_b
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0)),
            )
            .unwrap();

        for _ in 0..5 {
            let snap_a = engine_a.tick().unwrap();
            let snap_b = engine_b.tick().unwrap();
            let obj_a = snap_a.object(id_a).unwrap();
            let obj_b = snap_b.object(id_b).unwrap();
            assert_eq!(obj_a.position, obj_b.position);
            assert_eq!(obj_a.velocity, obj_b.velocity);
        }
    }

    #[test]
    fn clamps_objects_to_landscape_surface() {
        let script = r#"
        global func Step(state, frame, random) {
            return nil;
        }
        "#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(Definition::from_script("Static", "Static", script).unwrap())
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(16, 5));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Static")
                    .with_position(Vector2::new(4, 12))
                    .with_velocity(Vector2::new(0, 3)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 5));
        assert_eq!(object.velocity, Vector2::new(0, 0));
    }

    #[test]
    fn applies_effect_stack_operations() {
        let source = r#"
        global func Initialize(state, random) {
            return {
                effects = [
                    { op = "add", name = "Heal", priority = 150, interval = 2 }
                ]
            };
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    effects = [
                        { op = "add", name = "Boost", priority = 50, interval = 3, timer = 1 }
                    ]
                };
            }
            if (frame == 2) {
                return { effects = [ { op = "remove", name = "Heal" } ] };
            }
            return nil;
        }
        "#;

        let mut engine = Engine::with_seed(0);
        let definition =
            Definition::from_script("Actor", "Actor", source).expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot available");
        assert_eq!(snapshot.effects.len(), 1);
        assert_eq!(snapshot.effects[0].name, "Heal");
        assert_eq!(snapshot.effects[0].priority, 150);
        assert_eq!(snapshot.effects[0].interval, 2);
        assert_eq!(snapshot.effects[0].timer, 0);

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.effects.len(), 2);
        assert_eq!(object.effects[0].name, "Heal");
        assert_eq!(object.effects[0].timer, 1);
        assert_eq!(object.effects[1].name, "Boost");
        assert_eq!(object.effects[1].timer, 1);

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.effects.len(), 1);
        assert_eq!(object.effects[0].name, "Boost");
        assert_eq!(object.effects[0].priority, 50);
        assert_eq!(object.effects[0].timer, 2);
    }

    #[test]
    fn queued_commands_apply_effect_changes() {
        let mut engine = Engine::with_seed(1);
        let definition = Definition::from_script(
            "Dummy",
            "Dummy",
            "global func Step(state, frame, random) { return nil; }",
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Dummy"))
            .expect("spawn succeeds");

        let command = QueuedCommand::immediate(ObjectUpdate::default())
            .with_delay(1)
            .with_effects(vec![EffectCommand::add(EffectState::new("Queued"))]);
        engine
            .queue_object_command(id, command)
            .expect("queue succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert!(object.effects.is_empty());

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.effects.len(), 1);
        assert_eq!(object.effects[0].name, "Queued");
    }

    #[test]
    fn effect_callbacks_fire_across_lifecycle_events() {
        let script = r#"
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Pulse", interval = 2 } ] };
        }

        global func FxPulseStart(state, effect) {
            return nil;
        }

        global func FxPulseTimer(state, effect, timer) {
            return nil;
        }

        global func FxPulseStop(state, effect, reason) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 3) {
                return { effects = [ { op = "remove", name = "Pulse" } ] };
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let first = engine.tick().expect("first tick succeeds");
        let object = first.object(id).expect("object present");
        assert!(object.effects.iter().any(|effect| effect.name == "Pulse"));

        let second = engine.tick().expect("second tick succeeds");
        let object = second.object(id).expect("object present");
        assert!(object.effects.iter().any(|effect| effect.name == "Pulse"));

        let third = engine.tick().expect("third tick succeeds");
        let object = third.object(id).expect("object present");
        assert!(object.effects.is_empty());

        let calls = call_log.lock().unwrap().clone();
        let start_calls = calls.iter().filter(|name| *name == "FxPulseStart").count();
        let timer_calls = calls.iter().filter(|name| *name == "FxPulseTimer").count();
        let stop_calls = calls.iter().filter(|name| *name == "FxPulseStop").count();

        assert_eq!(start_calls, 1);
        assert!(timer_calls >= 1);
        assert_eq!(stop_calls, 1);
    }

    #[test]
    fn queued_commands_can_spawn_and_destroy() {
        let mut engine = Engine::with_seed(42);
        let definition = Definition::from_script(
            "Dummy",
            "Dummy",
            "global func Step(state, frame, random) { return nil; }",
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Dummy"))
            .expect("spawn succeeds");

        let command = QueuedCommand::immediate(ObjectUpdate::default())
            .with_delay(1)
            .with_destroy(true)
            .with_spawns(vec![
                SpawnConfig::new("Dummy").with_position(Vector2::new(5, 5))
            ]);
        engine
            .queue_object_command(id, command)
            .expect("queue succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(snapshot.objects.len(), 1);
        assert_eq!(object.id, id);

        let snapshot = engine.tick().expect("second tick succeeds");
        assert!(snapshot.object(id).is_none());
        assert_eq!(snapshot.objects.len(), 1);
        let new_object = &snapshot.objects[0];
        assert_ne!(new_object.id, id);
        assert_eq!(new_object.definition_id, "Dummy");
        assert_eq!(new_object.position, Vector2::new(5, 5));
    }

    #[test]
    fn recorder_playback_roundtrip_matches_engine() {
        let mut engine_a = Engine::with_seed(99);
        engine_a
            .register_definition(build_definition())
            .expect("definition registers");
        engine_a.set_landscape(Landscape::flat(32, 0));

        let spawn = SpawnConfig::new("Test")
            .with_position(Vector2::new(0, 0))
            .with_velocity(Vector2::new(1, 0));
        engine_a
            .spawn_object(spawn.clone())
            .expect("spawn succeeds");

        let mut recorder = Recorder::new();
        for _ in 0..5 {
            let snapshot = engine_a.tick().expect("tick succeeds");
            recorder.record(&snapshot);
        }
        let recording = recorder.into_recording();
        let serialized = recording.to_string().expect("serializes");

        let mut playback = Playback::from_str(&serialized).expect("playback loads");

        let mut engine_b = Engine::with_seed(99);
        engine_b
            .register_definition(build_definition())
            .expect("definition registers");
        engine_b.set_landscape(Landscape::flat(32, 0));
        engine_b.spawn_object(spawn).expect("spawn succeeds");

        for _ in 0..5 {
            let snapshot = engine_b.tick().expect("tick succeeds");
            playback
                .validate_snapshot(&snapshot)
                .expect("snapshots match");
        }
        playback.finish().expect("playback completed");
    }

    #[test]
    fn apply_object_update_overrides_velocity() {
        let mut engine = Engine::with_seed(1);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_velocity(Vector2::new(5, -3))
                    .with_owner(7),
            )
            .expect("update applies");

        let snapshot = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(snapshot.velocity, Vector2::new(5, -3));
        assert_eq!(snapshot.owner, 7);
    }

    #[test]
    fn apply_object_update_unknown_action_falls_back_to_default() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Run".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_action("Run")
                    .with_action_phase(2)
                    .with_action_ticks(5),
            )
            .expect("valid update applies");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Run");
        assert_eq!(snapshot.action.phase, 2);
        assert_eq!(snapshot.action.ticks, 5);

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_action("Ghost")
                    .with_action_phase(1)
                    .with_action_ticks(3),
            )
            .expect("invalid action handled");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Idle");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);
    }

    #[test]
    fn spawn_config_unknown_action_falls_back_to_default() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(4);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let mut requested = ActionState::new("Ghost");
        requested.phase = 3;
        requested.ticks = 7;

        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_action(requested))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Idle");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);
    }

    #[test]
    fn initialize_with_unknown_action_falls_back_to_default() {
        let source = r#"
        global func Initialize(state, random) {
            return { action = "Ghost" };
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition =
            Definition::from_script("Actor", "Actor", source).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::default());
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Walk".to_string()), actions);

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);
    }

    #[test]
    fn apply_object_update_unknown_object_errors() {
        let mut engine = Engine::with_seed(1);
        let error = engine
            .apply_object_update(ObjectId::new(999), ObjectUpdate::default())
            .expect_err("update fails");
        match error {
            EngineError::UnknownObject(id) => assert_eq!(id.as_u64(), 999),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn custom_physics_settings_affect_integration() {
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(2, 6, -8));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 2);
        assert_eq!(object.position.y, 2);
    }

    #[test]
    fn physics_clamps_horizontal_velocity() {
        let mut engine = Engine::with_seed(7);
        let definition = Definition::from_script(
            "Actor",
            "Actor",
            r#"
            global func Initialize(state, random) { return nil; }
            global func Step(state, frame, random) { return nil; }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let physics = PhysicsSettings::checked(1, 12, -20)
            .expect("physics valid")
            .with_max_horizontal_speed(4)
            .expect("horizontal speed valid");
        engine.set_physics(physics);

        let id = engine
            .spawn_object(
                SpawnConfig::new("Actor")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(20, 0)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(snapshot.velocity.x, 4);

        engine
            .apply_object_update(id, ObjectUpdate::new().with_velocity(Vector2::new(-9, 0)))
            .expect("update applies");

        let snapshot = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(snapshot.velocity.x, -4);

        let tick_snapshot = engine.tick().expect("tick succeeds");
        let object = tick_snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.x, -4);
    }

    #[test]
    fn queued_commands_apply_on_next_tick() {
        let script = r#"
        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Actor", "Actor", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Jump".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Actor")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        engine
            .queue_object_command(
                id,
                QueuedCommand::immediate(
                    ObjectUpdate::new()
                        .with_velocity(Vector2::new(3, -5))
                        .with_action("Jump"),
                ),
            )
            .expect("command enqueues");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.velocity, Vector2::new(3, -4));
        assert_eq!(object.position, Vector2::new(3, -4));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(6, -7));
    }

    fn simple_definition(id: &str) -> Definition {
        Definition::from_script(
            id,
            id,
            r#"
            global func Initialize(state, random) { return nil; }
            global func Step(state, frame, random) { return nil; }
            "#,
        )
        .expect("script compiles")
    }

    #[test]
    fn spawn_assigns_container_relationships() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Crate"))
            .expect("crate registers");
        engine
            .register_definition(simple_definition("Gem"))
            .expect("gem registers");

        let crate_id = engine
            .spawn_object(SpawnConfig::new("Crate"))
            .expect("crate spawns");
        let gem_id = engine
            .spawn_object(SpawnConfig::new("Gem").with_container(crate_id))
            .expect("gem spawns");

        let crate_snapshot = engine.object_snapshot(crate_id).expect("crate snapshot");
        assert_eq!(crate_snapshot.contents, vec![gem_id]);

        let gem_snapshot = engine.object_snapshot(gem_id).expect("gem snapshot");
        assert_eq!(gem_snapshot.container, Some(crate_id));
    }

    #[test]
    fn object_update_moves_between_containers() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Crate"))
            .expect("crate registers");
        engine
            .register_definition(simple_definition("Chest"))
            .expect("chest registers");
        engine
            .register_definition(simple_definition("Gem"))
            .expect("gem registers");

        let crate_id = engine
            .spawn_object(SpawnConfig::new("Crate"))
            .expect("crate spawns");
        let chest_id = engine
            .spawn_object(SpawnConfig::new("Chest"))
            .expect("chest spawns");
        let gem_id = engine
            .spawn_object(SpawnConfig::new("Gem").with_container(crate_id))
            .expect("gem spawns");

        engine
            .apply_object_update(gem_id, ObjectUpdate::new().with_container(chest_id))
            .expect("update succeeds");

        let crate_snapshot = engine.object_snapshot(crate_id).expect("crate snapshot");
        assert!(crate_snapshot.contents.is_empty());

        let chest_snapshot = engine.object_snapshot(chest_id).expect("chest snapshot");
        assert_eq!(chest_snapshot.contents, vec![gem_id]);

        let gem_snapshot = engine.object_snapshot(gem_id).expect("gem snapshot");
        assert_eq!(gem_snapshot.container, Some(chest_id));
    }

    #[test]
    fn destroying_container_detaches_contents() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Crate"))
            .expect("crate registers");
        engine
            .register_definition(simple_definition("Gem"))
            .expect("gem registers");

        let crate_id = engine
            .spawn_object(SpawnConfig::new("Crate"))
            .expect("crate spawns");
        let gem_id = engine
            .spawn_object(SpawnConfig::new("Gem").with_container(crate_id))
            .expect("gem spawns");

        engine
            .apply_object_update(
                crate_id,
                ObjectUpdate::new().with_status(ObjectStatus::Deleted),
            )
            .expect("delete succeeds");

        let gem_snapshot = engine.object_snapshot(gem_id).expect("gem snapshot");
        assert_eq!(gem_snapshot.container, None);

        let crate_snapshot = engine.object_snapshot(crate_id).expect("crate snapshot");
        assert!(crate_snapshot.contents.is_empty());
        assert_eq!(crate_snapshot.status, ObjectStatus::Deleted);
    }

    #[test]
    fn capture_state_restores_container_relationships() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Crate"))
            .expect("crate registers");
        engine
            .register_definition(simple_definition("Gem"))
            .expect("gem registers");

        let crate_id = engine
            .spawn_object(SpawnConfig::new("Crate"))
            .expect("crate spawns");
        let gem_id = engine
            .spawn_object(SpawnConfig::new("Gem").with_container(crate_id))
            .expect("gem spawns");

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(1);
        restored
            .register_definition(simple_definition("Crate"))
            .expect("crate registers");
        restored
            .register_definition(simple_definition("Gem"))
            .expect("gem registers");
        restored.restore_state(&state).expect("restore succeeds");

        let crate_snapshot = restored.object_snapshot(crate_id).expect("crate snapshot");
        assert_eq!(crate_snapshot.contents, vec![gem_id]);

        let gem_snapshot = restored.object_snapshot(gem_id).expect("gem snapshot");
        assert_eq!(gem_snapshot.container, Some(crate_id));
    }

    #[test]
    fn step_script_can_enqueue_commands() {
        let script = r#"
        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    commands = [
                        { velocity = [5, 0] },
                        { delay = 1, action = { name = "Slide", phase = 0 } }
                    ]
                };
            }
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Actor", "Actor", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Slide".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Actor")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        let first = engine.tick().expect("first tick succeeds");
        let object = first.object(id).expect("object present");
        assert_eq!(object.velocity.x, 0);

        let second = engine.tick().expect("second tick succeeds");
        let object = second.object(id).expect("object present");
        assert_eq!(object.velocity.x, 5);

        let third = engine.tick().expect("third tick succeeds");
        let object = third.object(id).expect("object present");
        assert_eq!(object.action.name, "Slide");
    }

    #[test]
    fn saves_and_loads_engine_state_via_files() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(0xC0FFEE);
        engine.set_physics(PhysicsSettings::new(3, 8, -5));
        engine.set_environment(EnvironmentSettings::new(5));
        engine.set_landscape(Landscape::flat(48, 12));

        let definition = Definition::from_script("Stateful", "Stateful", STATEFUL_SCRIPT)?;
        engine.register_definition(definition)?;

        let object_id = engine.spawn_object(
            SpawnConfig::new("Stateful")
                .with_position(Vector2::new(4, 2))
                .with_velocity(Vector2::new(1, -1))
                .with_energy(7),
        )?;

        engine.queue_object_command(
            object_id,
            QueuedCommand::new(1, ObjectUpdate::new().with_velocity(Vector2::new(2, -3))),
        )?;
        let _ = engine.tick()?;

        let state = engine.capture_state();
        let temp_file = NamedTempFile::new().expect("create temp state file");
        state
            .save_to_path(temp_file.path())
            .expect("write state to disk");

        let loaded = EngineState::load_from_path(temp_file.path()).expect("load state from disk");
        assert_eq!(loaded.frame, state.frame);
        assert_eq!(loaded.physics, state.physics);
        assert_eq!(loaded.environment, state.environment);
        assert_eq!(loaded.objects.len(), state.objects.len());
        assert_eq!(loaded.global_effects, state.global_effects);
        assert_eq!(loaded.crew_selection, state.crew_selection);
        assert_eq!(loaded.crew_roles, state.crew_roles);
        assert_eq!(loaded.known_crew_owners, state.known_crew_owners);
        assert_eq!(loaded.eliminated_crew_owners, state.eliminated_crew_owners);

        let mut restored = Engine::with_seed(77);
        let definition = Definition::from_script("Stateful", "Stateful", STATEFUL_SCRIPT)?;
        restored.register_definition(definition)?;
        restored.restore_state(&loaded)?;

        assert_eq!(engine.tick()?, restored.tick()?);

        Ok(())
    }

    #[test]
    fn captures_and_restores_engine_state() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(0xBAD_F00D);
        engine.set_physics(PhysicsSettings::new(2, 9, -6));
        engine.set_landscape(Landscape::flat(128, 15));
        engine.set_environment(EnvironmentSettings::new(-4));

        let definition = Definition::from_script("Stateful", "Stateful", STATEFUL_SCRIPT)?;
        engine.register_definition(definition)?;

        let object_id = engine.spawn_object(
            SpawnConfig::new("Stateful")
                .with_position(Vector2::new(10, 5))
                .with_velocity(Vector2::new(1, -2))
                .with_energy(12),
        )?;

        engine.queue_object_command(
            object_id,
            QueuedCommand::new(
                2,
                ObjectUpdate::new()
                    .with_action_update(ActionUpdate::default().with_name("Rest").with_phase(4)),
            )
            .with_effects(vec![EffectCommand::add(
                EffectState::new("Glow")
                    .with_priority(90)
                    .with_interval(3)
                    .with_timer(1),
            )])
            .with_spawns(vec![SpawnConfig::new("Stateful")
                .with_position(Vector2::new(3, 0))
                .with_velocity(Vector2::new(0, 0))
                .with_energy(5)
                .with_action(ActionState::new("Helper"))]),
        )?;

        let _ = engine.tick()?;

        let state = engine.capture_state();
        assert_eq!(state.environment, EnvironmentSettings::new(-4));
        let serialized = state
            .to_json_string()
            .expect("state serializes through helper");
        let decoded =
            EngineState::from_json_str(&serialized).expect("state round-trips via helper");

        let mut restored = Engine::with_seed(123);
        restored.set_physics(PhysicsSettings::new(5, 11, -8));
        restored.set_landscape(Landscape::flat(64, 9));
        restored.set_environment(EnvironmentSettings::new(9));
        let definition = Definition::from_script("Stateful", "Stateful", STATEFUL_SCRIPT)?;
        restored.register_definition(definition)?;
        restored.restore_state(&decoded)?;

        assert_eq!(restored.physics(), state.physics);
        assert_eq!(restored.environment(), state.environment);
        assert_eq!(restored.landscape(), state.landscape.as_ref());
        assert_eq!(engine.snapshot(), restored.snapshot());

        let next_original = engine.tick()?;
        let next_restored = restored.tick()?;
        assert_eq!(next_original, next_restored);

        let spawn_original = engine.tick()?;
        let spawn_restored = restored.tick()?;
        assert_eq!(spawn_original, spawn_restored);

        Ok(())
    }
}
