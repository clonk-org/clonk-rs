mod action;
mod compat;
mod control;
mod effect;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod fixtures;
mod input;
mod landscape;
mod material;
mod math;
mod message;
pub mod ocf;
mod pathfinder;
mod player;
mod record;
pub mod scenario;
mod sky;
mod transfer;

pub use action::{
    ActionLibrary, ActionProcedure, ActionSpec, ActionState, ActionUpdate, ActionUpdateResult,
};
pub use control::{
    interpret_player_control_command, CommandKind, ControlButton, ControlCommand, ControlEvent,
    ControlPacket, PlayerControlData, COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT, COM_CURSOR_RIGHT,
    COM_CURSOR_TOGGLE, COM_DIG, COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE, COM_MENU_DOWN,
    COM_MENU_ENTER, COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT, COM_MENU_SELECT,
    COM_MENU_SHOW_TEXT, COM_MENU_UP, COM_PLAYER_MENU, COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE,
    COM_SPECIAL, COM_SPECIAL2, COM_THROW, COM_UP,
};
pub use effect::EffectState;
pub use input::PlayerInputState;
pub use landscape::{
    BlastResult, CollisionResolution, Landscape, LandscapeCommand, LandscapeError, LiquidColumn,
    LiquidSegment,
};
pub use material::{Material, MaterialId, MaterialSet};
pub use message::{
    MessageKind, MessageSnapshot, FLAG_BOTTOM, FLAG_HCENTER, FLAG_LEFT, FLAG_RIGHT, FLAG_TOP,
    FLAG_VCENTER, FLAG_X_REL, FLAG_Y_REL,
};
pub use pathfinder::{PathFinder, PathWaypoint};
pub use player::{Player, PlayerConfig, PlayerState, PlayerStatus, PlayerViewport};
pub use record::{Playback, PlaybackError, Recorder, Recording};
pub use scenario::{Scenario, ScenarioError, SkyConfig};
pub use sky::{SkyFrame, SkyParallaxMode, SkySettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommandKind {
    Focus,
    DropAll,
}

impl MenuCommandKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            MenuCommandKind::Focus => "focus",
            MenuCommandKind::DropAll => "drop_all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuCommandSelection {
    pub primary_id: ObjectId,
    pub instances: Vec<ObjectId>,
    pub definition_id: DefinitionId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuEntry {
    pub function: String,
    pub label: String,
    pub description: Option<String>,
}

use compat::{
    enter_audio_context, enter_environment_context, enter_physics_context, enter_random_context,
    AudioRegistry, DefinitionMetadata, EffectContextOutcome, EnvironmentDelta, HostWorldContext,
    HostWorldObject, PhysicsDelta,
};
use effect::{EffectCommand, EffectEvent, EffectEventKind, EffectStopReason};
use material::MaterialReactionKind;
use message::{MessageCommand, MessageManager, MessageSpec, PersistedMessage};
use ocf::NORMAL as OCF_NORMAL;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::AddAssign;
use std::path::Path;
use std::sync::Arc;

use lc_resources::definition::ActionFacet as ResourceActionFacet;
use lc_resources::{
    ActionDefinition as ResourceActionDefinition, PictureRect as ResourcePictureRect,
    ResourceDefinition as ResourceDefinitionData,
};
use lc_script::{DebuggerHooks, Engine as ScriptEngine, ScriptError, Value};
use rand::{seq::SliceRandom, Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sky::SkyState;
use thiserror::Error;
use transfer::{TransferZoneCommand, TransferZoneRect, TransferZoneState, TransferZoneTable};

pub type DefinitionId = String;

pub const OWNER_NONE: i32 = -1;

pub const CNAT_NONE: u32 = 0;
pub const CNAT_LEFT: u32 = 1;
pub const CNAT_RIGHT: u32 = 2;
pub const CNAT_TOP: u32 = 4;
pub const CNAT_BOTTOM: u32 = 8;
pub const CNAT_CENTER: u32 = 16;
pub const CNAT_MULTI_ATTACH: u32 = 32;
pub const CNAT_NO_COLLISION: u32 = 64;

pub const CATEGORY_STATIC_BACK: i32 = 1 << 0;
pub const CATEGORY_STRUCTURE: i32 = 1 << 1;
pub const CATEGORY_VEHICLE: i32 = 1 << 2;
pub const CATEGORY_LIVING: i32 = 1 << 3;
pub const CATEGORY_OBJECT: i32 = 1 << 4;
pub const CATEGORY_SORT_LIMIT: i32 = CATEGORY_STATIC_BACK
    | CATEGORY_STRUCTURE
    | CATEGORY_VEHICLE
    | CATEGORY_LIVING
    | CATEGORY_OBJECT;
pub const DEFAULT_CATEGORY: i32 = CATEGORY_STATIC_BACK;

fn default_rng() -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(0)
}

fn compute_blast_size(radius: i32) -> i64 {
    let r = i64::from(radius.max(0));
    (r * r * 6283) / 2000
}

fn compute_blast_grade(radius: i32) -> i64 {
    let level = radius.max(0);
    let raw = (level / 10) - 1;
    i64::from(raw.clamp(1, 3))
}

pub(crate) fn normalize_category(raw: i32, fallback: i32) -> i32 {
    let sort_bits = raw & CATEGORY_SORT_LIMIT;
    if sort_bits != 0 {
        raw
    } else {
        let fallback_bits = fallback & CATEGORY_SORT_LIMIT;
        let replacement = if fallback_bits != 0 {
            fallback_bits
        } else {
            CATEGORY_STATIC_BACK
        };
        (raw & !CATEGORY_SORT_LIMIT) | replacement
    }
}

pub(crate) fn default_category() -> i32 {
    DEFAULT_CATEGORY
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
}

impl Direction {
    pub const fn to_script_value(self) -> i32 {
        match self {
            Direction::Left => 0,
            Direction::Right => 1,
        }
    }

    pub const fn from_script_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(Direction::Left),
            1 => Some(Direction::Right),
            _ => None,
        }
    }
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Left
    }
}

impl Serialize for Direction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i32(self.to_script_value())
    }
}

impl<'de> Deserialize<'de> for Direction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = i32::deserialize(deserializer)?;
        Ok(Direction::from_script_value(raw).unwrap_or_default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandDirection {
    Stop,
    Up,
    UpRight,
    Right,
    DownRight,
    Down,
    DownLeft,
    Left,
    UpLeft,
}

impl CommandDirection {
    pub const fn to_script_value(self) -> i32 {
        match self {
            CommandDirection::Stop => 0,
            CommandDirection::Up => 1,
            CommandDirection::UpRight => 2,
            CommandDirection::Right => 3,
            CommandDirection::DownRight => 4,
            CommandDirection::Down => 5,
            CommandDirection::DownLeft => 6,
            CommandDirection::Left => 7,
            CommandDirection::UpLeft => 8,
        }
    }

    pub const fn from_script_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(CommandDirection::Stop),
            1 => Some(CommandDirection::Up),
            2 => Some(CommandDirection::UpRight),
            3 => Some(CommandDirection::Right),
            4 => Some(CommandDirection::DownRight),
            5 => Some(CommandDirection::Down),
            6 => Some(CommandDirection::DownLeft),
            7 => Some(CommandDirection::Left),
            8 => Some(CommandDirection::UpLeft),
            _ => None,
        }
    }

    pub const fn axis_components(self) -> (i32, i32) {
        match self {
            CommandDirection::Stop => (0, 0),
            CommandDirection::Up => (0, -1),
            CommandDirection::UpRight => (1, -1),
            CommandDirection::Right => (1, 0),
            CommandDirection::DownRight => (1, 1),
            CommandDirection::Down => (0, 1),
            CommandDirection::DownLeft => (-1, 1),
            CommandDirection::Left => (-1, 0),
            CommandDirection::UpLeft => (-1, -1),
        }
    }
}

impl Default for CommandDirection {
    fn default() -> Self {
        CommandDirection::Stop
    }
}

impl Serialize for CommandDirection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i32(self.to_script_value())
    }
}

impl<'de> Deserialize<'de> for CommandDirection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = i32::deserialize(deserializer)?;
        Ok(CommandDirection::from_script_value(raw).unwrap_or_default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WeatherEvent {
    Lightning { position: i32 },
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FloatVector2 {
    pub x: f32,
    pub y: f32,
}

impl FloatVector2 {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl PartialEq for FloatVector2 {
    fn eq(&self, other: &Self) -> bool {
        self.x.to_bits() == other.x.to_bits() && self.y.to_bits() == other.y.to_bits()
    }
}

impl Eq for FloatVector2 {}

impl AddAssign<FloatVector2> for FloatVector2 {
    fn add_assign(&mut self, rhs: FloatVector2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", content = "object")]
pub enum ParticleLayer {
    #[serde(rename = "global")]
    Global,
    #[serde(rename = "front")]
    ObjectFront(ObjectId),
    #[serde(rename = "back")]
    ObjectBack(ObjectId),
}

impl ParticleLayer {
    pub fn from_ffi(layer: i32, has_owner: bool, owner_id: u64) -> Option<Self> {
        match layer {
            0 => Some(Self::Global),
            1 => {
                if !has_owner {
                    None
                } else {
                    Some(Self::ObjectFront(ObjectId::new(owner_id)))
                }
            }
            2 => {
                if !has_owner {
                    None
                } else {
                    Some(Self::ObjectBack(ObjectId::new(owner_id)))
                }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleSnapshot {
    pub definition_id: String,
    pub position: FloatVector2,
    pub velocity: FloatVector2,
    pub life: i32,
    #[serde(default)]
    pub parameter_a: f32,
    #[serde(default)]
    pub parameter_b: i32,
    pub layer: ParticleLayer,
}

impl PartialEq for ParticleSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.definition_id == other.definition_id
            && self.position == other.position
            && self.velocity == other.velocity
            && self.life == other.life
            && self.parameter_a.to_bits() == other.parameter_a.to_bits()
            && self.parameter_b == other.parameter_b
            && self.layer == other.layer
    }
}

impl Eq for ParticleSnapshot {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticleScope {
    Global,
    Object(ObjectId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleConfig {
    pub definition_id: String,
    pub position: FloatVector2,
    pub velocity: FloatVector2,
    pub life: i32,
    pub parameter_a: f32,
    pub parameter_b: i32,
    pub layer: ParticleLayer,
}

impl PartialEq for ParticleConfig {
    fn eq(&self, other: &Self) -> bool {
        self.definition_id == other.definition_id
            && self.position == other.position
            && self.velocity == other.velocity
            && self.life == other.life
            && self.parameter_a.to_bits() == other.parameter_a.to_bits()
            && self.parameter_b == other.parameter_b
            && self.layer == other.layer
    }
}

impl Eq for ParticleConfig {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticleCommand {
    Create(ParticleConfig),
    Clear {
        definition_id: Option<String>,
        scope: ParticleScope,
    },
}

#[derive(Debug, Clone)]
struct ActiveParticle {
    snapshot: ParticleSnapshot,
    original_life: i32,
}

impl ActiveParticle {
    fn from_config(config: ParticleConfig) -> Self {
        let ParticleConfig {
            definition_id,
            position,
            velocity,
            life,
            parameter_a,
            parameter_b,
            layer,
        } = config;
        let clamped_life = life.max(0);
        let snapshot = ParticleSnapshot {
            definition_id,
            position,
            velocity,
            life: clamped_life,
            parameter_a,
            parameter_b,
            layer,
        };
        Self {
            snapshot,
            original_life: clamped_life,
        }
    }

    fn from_snapshot(mut snapshot: ParticleSnapshot) -> Self {
        if snapshot.life < 0 {
            snapshot.life = 0;
        }
        let original_life = snapshot.life;
        Self {
            snapshot,
            original_life,
        }
    }

    fn tick(&mut self) {
        self.snapshot.position += self.snapshot.velocity;
        if self.original_life > 0 && self.snapshot.life > 0 {
            self.snapshot.life -= 1;
        }
    }

    fn is_expired(&self) -> bool {
        self.original_life > 0 && self.snapshot.life == 0
    }

    fn snapshot(&self) -> ParticleSnapshot {
        self.snapshot.clone()
    }
}

#[derive(Debug, Clone)]
struct MaterialParticle {
    material: MaterialId,
    position: FloatVector2,
    velocity: FloatVector2,
}

impl MaterialParticle {
    fn new(material: MaterialId, position: FloatVector2, velocity: FloatVector2) -> Self {
        Self {
            material,
            position,
            velocity,
        }
    }

    fn snapshot(&self, materials: &MaterialSet) -> ParticleSnapshot {
        let definition_id = materials
            .get_by_id(self.material)
            .map(|material| format!("material/pxs/{}", material.normalized_name()))
            .unwrap_or_else(|| "material/pxs/unknown".to_string());
        ParticleSnapshot {
            definition_id,
            position: self.position,
            velocity: self.velocity,
            life: 0,
            parameter_a: 0.0,
            parameter_b: self.material.index() as i32,
            layer: ParticleLayer::Global,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HudSnapshot {
    #[serde(default)]
    pub players: Vec<HudPlayerSnapshot>,
    #[serde(default)]
    pub messages: Vec<MessageSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudPlayerSnapshot {
    pub owner: i32,
    #[serde(default)]
    pub crew: Vec<ObjectId>,
    #[serde(default)]
    pub focus: Option<ObjectId>,
    #[serde(default)]
    pub eliminated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SurfaceSnapshot {
    #[serde(default)]
    pub label: String,
    pub width: i32,
    pub height: i32,
    pub hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPacketDirection {
    Inbound,
    Outbound,
}

impl Default for NetworkPacketDirection {
    fn default() -> Self {
        Self::Inbound
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NetworkPacketSnapshot {
    #[serde(default)]
    pub direction: NetworkPacketDirection,
    pub status: u8,
    pub size: u32,
    pub hash: u64,
    pub client_id: i32,
    #[serde(default)]
    pub connection_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObjectVertex {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub cnat: u32,
    #[serde(default)]
    pub friction: i32,
}

impl ObjectVertex {
    pub fn new(x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            cnat: CNAT_NONE,
            friction: 0,
        }
    }

    pub fn with_cnat(mut self, cnat: u32) -> Self {
        self.cnat = cnat;
        self
    }

    pub fn with_friction(mut self, friction: i32) -> Self {
        self.friction = friction;
        self
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
#[serde(default)]
pub struct MovementProfile {
    pub walk_speed: i32,
    pub walk_acceleration: i32,
    pub float_speed: i32,
    pub float_acceleration: i32,
    pub swim_speed: i32,
    pub swim_acceleration: i32,
    pub scale_speed: i32,
    pub scale_acceleration: i32,
    pub hangle_speed: i32,
    pub hangle_acceleration: i32,
    pub dig_speed: i32,
}

impl MovementProfile {
    pub const fn new(float_speed: i32, float_acceleration: i32) -> Self {
        Self {
            walk_speed: 8,
            walk_acceleration: 1,
            float_speed,
            float_acceleration,
            swim_speed: 6,
            swim_acceleration: 1,
            scale_speed: 8,
            scale_acceleration: 1,
            hangle_speed: 8,
            hangle_acceleration: 1,
            dig_speed: 8,
        }
    }

    pub fn with_walk_speed(mut self, walk_speed: i32) -> Self {
        self.walk_speed = walk_speed;
        self
    }

    pub fn with_walk_acceleration(mut self, walk_acceleration: i32) -> Self {
        self.walk_acceleration = walk_acceleration;
        self
    }

    pub fn with_float_speed(mut self, float_speed: i32) -> Self {
        self.float_speed = float_speed;
        self
    }

    pub fn with_float_acceleration(mut self, float_acceleration: i32) -> Self {
        self.float_acceleration = float_acceleration;
        self
    }

    pub fn with_swim_speed(mut self, swim_speed: i32) -> Self {
        self.swim_speed = swim_speed;
        self
    }

    pub fn with_swim_acceleration(mut self, swim_acceleration: i32) -> Self {
        self.swim_acceleration = swim_acceleration;
        self
    }

    pub fn with_scale_speed(mut self, scale_speed: i32) -> Self {
        self.scale_speed = scale_speed;
        self
    }

    pub fn with_scale_acceleration(mut self, scale_acceleration: i32) -> Self {
        self.scale_acceleration = scale_acceleration;
        self
    }

    pub fn with_hangle_speed(mut self, hangle_speed: i32) -> Self {
        self.hangle_speed = hangle_speed;
        self
    }

    pub fn with_hangle_acceleration(mut self, hangle_acceleration: i32) -> Self {
        self.hangle_acceleration = hangle_acceleration;
        self
    }

    pub fn with_dig_speed(mut self, dig_speed: i32) -> Self {
        self.dig_speed = dig_speed;
        self
    }
}

impl Default for MovementProfile {
    fn default() -> Self {
        Self {
            walk_speed: 8,
            walk_acceleration: 1,
            float_speed: 6,
            float_acceleration: 1,
            swim_speed: 6,
            swim_acceleration: 1,
            scale_speed: 8,
            scale_acceleration: 1,
            hangle_speed: 8,
            hangle_acceleration: 1,
            dig_speed: 8,
        }
    }
}

#[derive(Clone, Copy)]
struct BridgeParameters {
    duration: u32,
    move_clonk: bool,
    wall: bool,
    _material: Option<u8>,
}

impl BridgeParameters {
    fn from_action_data(data: i32) -> Self {
        let raw = data as u32;
        let duration_raw = ((raw >> 16) & 0xFFFF) as u32;
        let duration = if duration_raw == 0 { 100 } else { duration_raw };
        let move_clonk = (raw & 0x100) != 0;
        let wall = (raw & 0x200) != 0;
        let material_byte = (raw & 0xFF) as u8;
        let material = if material_byte == 0xFF {
            None
        } else {
            Some(material_byte)
        };
        Self {
            duration,
            move_clonk,
            wall,
            _material: material,
        }
    }

    fn step_interval(&self, direction: CommandDirection) -> Option<u32> {
        use CommandDirection::*;
        if self.wall {
            match direction {
                Left | Right => Some(4),
                UpLeft | UpRight | Up => Some(5),
                _ => None,
            }
        } else {
            match direction {
                Left | Right => Some(5),
                Up => Some(4),
                UpLeft | UpRight => Some(6),
                _ => None,
            }
        }
    }
}

pub(crate) fn encode_bridge_action_data(
    duration: i32,
    move_clonk: bool,
    wall: bool,
    material: i32,
) -> i32 {
    let clamped_duration = duration.clamp(0, 0xFFFF) as u32;
    let mut raw = clamped_duration << 16;
    if move_clonk {
        raw |= 1 << 8;
    }
    if wall {
        raw |= 1 << 9;
    }
    let material_byte = if material < 0 {
        0xFF
    } else {
        (material as u32) & 0xFF
    };
    raw |= material_byte;
    raw as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSettings {
    pub wind: i32,
    #[serde(default)]
    pub base_wind: i32,
    #[serde(default)]
    pub wind_target: i32,
    #[serde(default)]
    pub wind_update_timer: u16,
    #[serde(default)]
    pub wind_update_interval: u16,
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
    #[serde(default)]
    pub precipitation: i32,
    #[serde(default)]
    pub sky_color: Option<RgbColor>,
    #[serde(default)]
    pub season: i32,
    #[serde(default)]
    pub year_speed: i32,
    #[serde(default)]
    pub season_delay: i32,
    #[serde(default = "EnvironmentSettings::default_temperature_range")]
    pub temperature_range: i32,
    #[serde(default)]
    pub lightning: i32,
    #[serde(default)]
    pub meteorite: i32,
    #[serde(default)]
    pub volcano: i32,
    #[serde(default)]
    pub earthquake: i32,
    #[serde(default)]
    pub precipitation_strength: i32,
    #[serde(default = "EnvironmentSettings::default_no_gamma")]
    pub no_gamma: bool,
}

impl EnvironmentSettings {
    pub const TIME_CYCLE: u16 = 2400;
    const MAX_TIME_SPEED: i16 = 120;

    const fn default_temperature_range() -> i32 {
        30
    }

    const fn default_no_gamma() -> bool {
        true
    }

    pub const fn new(wind: i32) -> Self {
        Self {
            wind,
            base_wind: wind,
            wind_target: wind,
            wind_update_timer: 0,
            wind_update_interval: 0,
            wind_variation: 0,
            wind_period: 0,
            temperature: 0,
            climate: 0,
            temperature_variation: 0,
            temperature_period: 0,
            temperature_phase: 0,
            time_of_day: 0,
            time_speed: 0,
            precipitation: 0,
            sky_color: None,
            season: 0,
            year_speed: 0,
            season_delay: 0,
            temperature_range: Self::default_temperature_range(),
            lightning: 0,
            meteorite: 0,
            volcano: 0,
            earthquake: 0,
            precipitation_strength: 0,
            no_gamma: Self::default_no_gamma(),
        }
    }

    pub fn with_wind_variation(mut self, variation: i32, period: u32) -> Self {
        if variation == 0 {
            self.wind_variation = 0;
            self.wind_period = 0;
            self.wind_target = self.base_wind;
            self.wind_update_interval = 0;
            self.wind_update_timer = 0;
            return self;
        }
        self.wind_variation = variation.abs();
        self.wind_period = period.max(2);
        self.wind_update_interval = Self::default_wind_update_interval(self.wind_period);
        self.wind_update_timer = 0;
        self.wind_target = self.wind;
        self.base_wind = self.wind;
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

    pub fn with_precipitation(mut self, precipitation: i32) -> Self {
        let clamped = precipitation.clamp(-100, 100);
        self.precipitation = clamped;
        self
    }

    pub fn with_sky_color(mut self, color: RgbColor) -> Self {
        self.sky_color = Some(color);
        self
    }

    pub fn with_season(mut self, season: i32) -> Self {
        self.season = season.clamp(0, 100);
        self
    }

    pub fn with_year_speed(mut self, year_speed: i32) -> Self {
        self.year_speed = year_speed;
        self
    }

    pub fn with_temperature_range(mut self, range: i32) -> Self {
        self.temperature_range = range.clamp(0, 100);
        self
    }

    pub fn with_lightning(mut self, level: i32) -> Self {
        self.lightning = level.clamp(0, 100);
        self
    }

    pub fn with_meteorite(mut self, level: i32) -> Self {
        self.meteorite = level.clamp(0, 100);
        self
    }

    pub fn with_volcano(mut self, level: i32) -> Self {
        self.volcano = level.clamp(0, 100);
        self
    }

    pub fn with_earthquake(mut self, level: i32) -> Self {
        self.earthquake = level.clamp(0, 100);
        self
    }

    pub fn with_precipitation_strength(mut self, strength: i32) -> Self {
        self.precipitation_strength = strength.clamp(-100, 100);
        self
    }

    pub fn with_gamma_enabled(mut self) -> Self {
        self.no_gamma = false;
        self
    }

    pub fn with_gamma_disabled(mut self) -> Self {
        self.no_gamma = true;
        self
    }

    fn default_wind_update_interval(period: u32) -> u16 {
        if period == 0 {
            return 60;
        }
        let normalized = (period / 2).max(1);
        if normalized >= u32::from(u16::MAX) {
            u16::MAX
        } else {
            normalized as u16
        }
    }

    pub fn without_sky_color(mut self) -> Self {
        self.sky_color = None;
        self
    }

    pub fn sky_color(&self) -> Option<RgbColor> {
        self.sky_color
    }

    pub fn resolved_sky_color(&self, ambient_temperature: i32) -> RgbColor {
        self.sky_color
            .unwrap_or_else(|| Self::dynamic_sky_color(self.time_of_day, ambient_temperature))
    }

    pub fn season_gamma(&self) -> Option<(RgbColor, RgbColor, RgbColor)> {
        if self.no_gamma {
            None
        } else {
            Some(Self::compute_season_gamma(self.season, self.temperature))
        }
    }

    fn dynamic_sky_color(time_of_day: u16, ambient_temperature: i32) -> RgbColor {
        let normalized_time = f32::from(time_of_day) / f32::from(Self::TIME_CYCLE.max(1));
        let daylight = (1.0 - (normalized_time * core::f32::consts::TAU).cos()) * 0.5;
        let daylight = daylight.clamp(0.0, 1.0);

        let temperature_factor = ((ambient_temperature + 50) as f32 / 100.0).clamp(0.0, 1.0);

        let cold_day = [96.0, 140.0, 212.0];
        let warm_day = [148.0, 196.0, 255.0];
        let night = [12.0, 20.0, 48.0];

        let mut day_color = [0.0; 3];
        for (idx, value) in day_color.iter_mut().enumerate() {
            let cold = cold_day[idx];
            let warm = warm_day[idx];
            *value = cold + (warm - cold) * temperature_factor;
        }

        let mut channel = [0u8; 3];
        for idx in 0..3 {
            let value = night[idx] + (day_color[idx] - night[idx]) * daylight;
            channel[idx] = value.round().clamp(0.0, 255.0) as u8;
        }

        RgbColor::new(channel[0], channel[1], channel[2])
    }

    fn compute_season_gamma(season: i32, temperature: i32) -> (RgbColor, RgbColor, RgbColor) {
        const SEASON_COLORS: [[u32; 3]; 4] = [
            [0x000000, 0x7f7f90, 0xefefff],
            [0x070f00, 0x90a07f, 0xffffdf],
            [0x000000, 0x808080, 0xffffff],
            [0x0f0700, 0xa08067, 0xffffdf],
        ];

        let mut season_index = season.rem_euclid(100);
        if season_index < 0 {
            season_index += 100;
        }
        let primary = ((season_index / 25) % 4) as usize;
        let secondary = (primary + 1) % 4;

        let mut offset = season_index % 25;
        offset = offset.clamp(5, 19);
        let offset_primary = offset - 5;
        let offset_secondary = 15 - offset_primary;

        let mut ramp = [0u32; 3];
        for (idx, color) in ramp.iter_mut().enumerate() {
            let mut accumulated = 0u32;
            for channel_shift in [0usize, 8, 16] {
                let c1 = ((SEASON_COLORS[primary][idx] >> channel_shift) & 0xff) as i32;
                let c2 = ((SEASON_COLORS[secondary][idx] >> channel_shift) & 0xff) as i32;
                let mut value = (c1 * offset_secondary + c2 * offset_primary) / 15;
                if temperature < 0 {
                    if channel_shift == 0 {
                        value -= temperature / 2;
                    } else {
                        value += temperature / 2;
                    }
                }
                let value = value.clamp(0, 255) as u32;
                accumulated |= value << channel_shift;
            }
            *color = accumulated;
        }

        (
            Self::color_from_bgr(ramp[0]),
            Self::color_from_bgr(ramp[1]),
            Self::color_from_bgr(ramp[2]),
        )
    }

    fn color_from_bgr(value: u32) -> RgbColor {
        let r = ((value >> 16) & 0xff) as u8;
        let g = ((value >> 8) & 0xff) as u8;
        let b = (value & 0xff) as u8;
        RgbColor::new(r, g, b)
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

    pub fn advance_frame(&mut self, rng: &mut ChaCha8Rng) {
        self.refresh_runtime_fields();
        self.update_season();
        self.update_temperature_from_season();
        self.update_wind(rng);
        self.advance_time_of_day();
        self.update_precipitation_runtime();
    }

    pub fn time_of_day(&self) -> u16 {
        self.time_of_day
    }

    pub fn time_speed(&self) -> i16 {
        self.time_speed
    }

    pub fn precipitation(&self) -> i32 {
        self.precipitation
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

    fn advance_time_of_day(&mut self) {
        if self.time_speed == 0 {
            return;
        }
        let next = (i32::from(self.time_of_day) + i32::from(self.time_speed))
            .rem_euclid(i32::from(Self::TIME_CYCLE));
        self.time_of_day = next as u16;
    }

    fn update_season(&mut self) {
        if self.year_speed == 0 {
            return;
        }
        self.season_delay = self.season_delay.saturating_add(self.year_speed);
        while self.season_delay >= 200 {
            self.season_delay -= 200;
            self.season = (self.season + 1).rem_euclid(100);
        }
        while self.season_delay <= -200 {
            self.season_delay += 200;
            self.season = (self.season - 1).rem_euclid(100);
        }
        if self.season < 0 {
            self.season += 100;
        }
    }

    fn update_temperature_from_season(&mut self) {
        if self.temperature_range <= 0 || self.year_speed == 0 {
            return;
        }
        let season_angle = (self.season.rem_euclid(100) as f32 / 100.0) * core::f32::consts::TAU;
        let delta = (self.temperature_range as f32 * season_angle.cos()).round() as i32;
        let target = self.climate.saturating_sub(delta);
        if self.temperature < target {
            self.temperature = self.temperature.saturating_add(1);
        } else if self.temperature > target {
            self.temperature = self.temperature.saturating_sub(1);
        }
    }

    fn update_precipitation_runtime(&mut self) {
        if self.precipitation_strength != 0 {
            self.precipitation = self.precipitation_strength;
        }
    }

    fn update_wind(&mut self, rng: &mut ChaCha8Rng) {
        if self.wind_update_interval == 0 || self.wind_variation == 0 {
            self.wind_target = self.wind;
            self.wind_update_timer = 0;
        } else {
            if self.wind_update_timer == 0 {
                self.wind_update_timer = self.wind_update_interval;
                let range = self.wind_variation.abs();
                let offset = rng.gen_range(-range..=range);
                let lower = self.base_wind.saturating_sub(range);
                let upper = self.base_wind.saturating_add(range);
                let target = self.base_wind.saturating_add(offset).clamp(lower, upper);
                self.wind_target = target;
            }

            if self.wind_update_timer > 0 {
                self.wind_update_timer -= 1;
            }
        }

        if self.wind < self.wind_target {
            self.wind = self.wind.saturating_add(1);
        } else if self.wind > self.wind_target {
            self.wind = self.wind.saturating_sub(1);
        }
    }

    pub fn refresh_runtime_fields(&mut self) {
        if self.wind_update_interval == 0 && self.wind_variation > 0 {
            self.wind_update_interval = Self::default_wind_update_interval(self.wind_period);
        }

        if self.wind_variation == 0 {
            self.wind_update_interval = 0;
            self.wind_update_timer = 0;
            self.wind_target = self.wind;
        } else {
            if self.wind_update_interval == 0 {
                self.wind_update_interval = 1;
            }
            if self.wind_update_timer >= self.wind_update_interval {
                self.wind_update_timer %= self.wind_update_interval;
            }
            if self.wind_target == 0 && self.wind != 0 {
                self.wind_target = self.wind;
            }
        }

        if self.base_wind == 0 || self.wind_variation == 0 {
            self.base_wind = self.wind;
        }
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
    #[serde(default)]
    pub precipitation: i32,
    #[serde(default)]
    pub sky_color: Option<RgbColor>,
}

impl Default for EnvironmentFrame {
    fn default() -> Self {
        Self {
            settings: EnvironmentSettings::default(),
            wind_force: 0,
            ambient_temperature: 0,
            precipitation: 0,
            sky_color: None,
        }
    }
}

fn default_owner() -> i32 {
    OWNER_NONE
}

fn default_alive() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectState {
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
    #[serde(default)]
    pub damage: i32,
    pub action: ActionState,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    #[serde(default = "default_category")]
    pub category: i32,
    #[serde(default)]
    pub crew_member: bool,
    #[serde(default = "default_alive")]
    pub alive: bool,
}

#[derive(Debug, Clone)]
struct ActionChange {
    previous: ActionState,
    requested_name_change: bool,
}

impl ActionChange {
    fn should_record(&self, current: &ActionState) -> bool {
        self.requested_name_change || self.previous.name != current.name
    }
}

#[derive(Debug, Clone, Default)]
struct ApplyDeltaOutcome {
    container_change: Option<(Option<ObjectId>, Option<ObjectId>)>,
    action_change: Option<ActionChange>,
}

impl ObjectState {
    fn apply_delta(&mut self, delta: &ObjectDelta, library: &ActionLibrary) -> ApplyDeltaOutcome {
        let previous_container = self.container;
        let mut container_change = None;
        let mut action_change = None;
        if let Some(position) = delta.position {
            self.position = position;
        }
        if let Some(velocity) = delta.velocity {
            self.velocity = velocity;
        }
        if let Some(energy) = delta.energy {
            self.energy = energy;
        }
        if let Some(damage) = delta.damage {
            self.damage = damage.max(0);
        }
        if let Some(direction) = delta.direction {
            self.direction = direction;
        }
        if let Some(command_direction) = delta.command_direction {
            self.command_direction = command_direction;
        }
        if let Some(action) = &delta.action {
            let requested_name_change = action.name.is_some();
            let previous_action = self.action.clone();
            let result = self.action.apply_update_with_library(action, library);
            if matches!(result, ActionUpdateResult::Applied) {
                action_change = Some(ActionChange {
                    previous: previous_action,
                    requested_name_change,
                });
            }
        } else {
            self.action.reconcile_with_library(library);
        }
        if let Some(vertices) = &delta.vertices {
            self.vertices = vertices.clone();
        }
        if let Some(owner) = delta.owner {
            self.owner = owner;
        }
        if let Some(category) = delta.category {
            self.category = category;
        }
        if let Some(crew_member) = delta.crew_member {
            self.crew_member = crew_member;
        }
        if let Some(alive) = delta.alive {
            self.alive = alive;
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
        ApplyDeltaOutcome {
            container_change,
            action_change: action_change.and_then(|change| {
                if change.should_record(&self.action) {
                    Some(change)
                } else {
                    None
                }
            }),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ObjectDelta {
    position: Option<Vector2>,
    velocity: Option<Vector2>,
    energy: Option<i32>,
    damage: Option<i32>,
    direction: Option<Direction>,
    command_direction: Option<CommandDirection>,
    action: Option<ActionUpdate>,
    status: Option<ObjectStatus>,
    owner: Option<i32>,
    category: Option<i32>,
    crew_member: Option<bool>,
    alive: Option<bool>,
    container: Option<Option<ObjectId>>,
    vertices: Option<Vec<ObjectVertex>>,
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
        if let Some(damage) = update.damage {
            self.damage = Some(damage);
        }
        if let Some(direction) = update.direction {
            self.direction = Some(direction);
        }
        if let Some(command_direction) = update.command_direction {
            self.command_direction = Some(command_direction);
        }
        if let Some(owner) = update.owner {
            self.owner = Some(owner);
        }
        if let Some(category) = update.category {
            self.category = Some(category);
        }
        if let Some(crew_member) = update.crew_member {
            self.crew_member = Some(crew_member);
        }
        if let Some(alive) = update.alive {
            self.alive = Some(alive);
        }
        if let Some(container) = update.container {
            self.container = Some(container);
        }
        if let Some(status) = update.status {
            self.status = Some(status);
        }
        if let Some(vertices) = update.vertices {
            self.vertices = Some(vertices);
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
            damage: update.damage,
            direction: update.direction,
            command_direction: update.command_direction,
            action: update.action,
            status: update.status,
            owner: update.owner,
            category: update.category,
            crew_member: update.crew_member,
            alive: update.alive,
            container: update.container,
            vertices: update.vertices,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectUpdate {
    pub position: Option<Vector2>,
    pub velocity: Option<Vector2>,
    pub energy: Option<i32>,
    #[serde(default)]
    pub damage: Option<i32>,
    pub action: Option<ActionUpdate>,
    #[serde(default)]
    pub direction: Option<Direction>,
    #[serde(default)]
    pub command_direction: Option<CommandDirection>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    #[serde(default)]
    pub owner: Option<i32>,
    #[serde(default)]
    pub category: Option<i32>,
    #[serde(default)]
    pub crew_member: Option<bool>,
    #[serde(default)]
    pub alive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Option<ObjectId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<ObjectVertex>>,
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

    pub fn with_damage(mut self, damage: i32) -> Self {
        self.damage = Some(damage);
        self
    }

    pub fn set_damage(&mut self, damage: i32) {
        self.damage = Some(damage);
    }

    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    pub fn with_command_direction(mut self, command_direction: CommandDirection) -> Self {
        self.command_direction = Some(command_direction);
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

    pub fn with_action_data(mut self, data: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_data(data);
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

    pub fn with_category(mut self, category: i32) -> Self {
        self.category = Some(category);
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

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = Some(alive);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.position.is_none()
            && self.velocity.is_none()
            && self.energy.is_none()
            && self.damage.is_none()
            && self.direction.is_none()
            && self.command_direction.is_none()
            && self.action.is_none()
            && self.status.is_none()
            && self.owner.is_none()
            && self.category.is_none()
            && self.crew_member.is_none()
            && self.alive.is_none()
            && self.container.is_none()
            && self.vertices.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub delay: u32,
    pub update: ObjectUpdate,
    pub effects: Vec<EffectCommand>,
    pub destroy: bool,
    pub spawns: Vec<SpawnConfig>,
    #[serde(default)]
    pub landscape: Vec<LandscapeCommand>,
    #[serde(default)]
    pub particles: Vec<ParticleCommand>,
}

impl QueuedCommand {
    pub fn new(delay: u32, update: ObjectUpdate) -> Self {
        Self {
            delay,
            update,
            effects: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
            landscape: Vec::new(),
            particles: Vec::new(),
        }
    }

    pub fn immediate(update: ObjectUpdate) -> Self {
        Self {
            delay: 0,
            update,
            effects: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
            landscape: Vec::new(),
            particles: Vec::new(),
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

    pub fn with_landscape(mut self, commands: Vec<LandscapeCommand>) -> Self {
        self.landscape = commands;
        self
    }

    pub fn with_particles(mut self, particles: Vec<ParticleCommand>) -> Self {
        self.particles = particles;
        self
    }

    pub fn update(&self) -> &ObjectUpdate {
        &self.update
    }

    pub fn effects(&self) -> &[EffectCommand] {
        &self.effects
    }

    pub fn landscape(&self) -> &[LandscapeCommand] {
        &self.landscape
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
    pending_action_events: VecDeque<ActionTransitionEvent>,
    material_contents: Vec<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionTransitionKind {
    Natural,
    Forced,
}

#[derive(Debug, Clone)]
struct ActionTransitionEvent {
    previous_action: String,
    kind: ActionTransitionKind,
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
    particles: Vec<ParticleCommand>,
}

impl Object {
    fn new(id: ObjectId, definition_id: DefinitionId, state: ObjectState) -> Self {
        Self {
            id,
            definition_id,
            destroyed: matches!(state.status, ObjectStatus::Deleted),
            state,
            command_queue: VecDeque::new(),
            pending_action_events: VecDeque::new(),
            material_contents: Vec::new(),
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
            damage: self.state.damage,
            action: self.state.action.clone(),
            direction: self.state.direction,
            command_direction: self.state.command_direction,
            action_procedure: procedure,
            effects: self.state.effects.clone(),
            vertices: self.state.vertices.clone(),
            container: self.state.container,
            contents: self.state.contents.clone(),
            status: self.state.status,
            owner: self.state.owner,
            category: self.state.category,
            crew_member: self.state.crew_member,
            alive: self.state.alive,
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
                EffectCommand::Remove { name, no_callbacks } => {
                    if let Some(removed) = self.remove_effect(name) {
                        if !no_callbacks {
                            events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
                        }
                    }
                }
                EffectCommand::Clear => {
                    events.extend(self.drain_effects_with_reason(EffectStopReason::Cleared));
                }
            }
        }
        events
    }

    fn ensure_material_capacity(&mut self, count: usize) {
        if self.material_contents.len() < count {
            self.material_contents.resize(count, 0);
        }
    }

    fn material_content(&self, material: MaterialId) -> i32 {
        let index = material.index();
        self.material_contents.get(index).copied().unwrap_or(0)
    }

    fn set_material_content(&mut self, material: MaterialId, amount: i32) {
        let index = material.index();
        if self.material_contents.len() <= index {
            self.material_contents.resize(index + 1, 0);
        }
        self.material_contents[index] = amount.max(0);
    }

    fn add_material_content(&mut self, material: MaterialId, amount: i32) {
        if amount <= 0 {
            return;
        }
        let index = material.index();
        if self.material_contents.len() <= index {
            self.material_contents.resize(index + 1, 0);
        }
        let slot = &mut self.material_contents[index];
        *slot = slot.saturating_add(amount);
    }

    fn apply_material_interaction(&mut self, material: &Material) {
        let friction = material.friction();
        if friction != 0 {
            self.state.velocity.x = apply_horizontal_friction(self.state.velocity.x, friction);
            for vertex in &mut self.state.vertices {
                vertex.friction = friction;
            }
        }
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

    fn record_action_event(&mut self, previous: ActionState, kind: ActionTransitionKind) {
        self.pending_action_events.push_back(ActionTransitionEvent {
            previous_action: previous.name,
            kind,
        });
    }

    fn execute_command_queue(
        &mut self,
        physics: &PhysicsSettings,
        materials: &MaterialSet,
        mut landscape: Option<&mut Landscape>,
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
            let delta_outcome = self.state.apply_delta(&delta, action_library);
            if let Some(change) = delta_outcome.action_change {
                self.record_action_event(change.previous, ActionTransitionKind::Forced);
            }
            if let Some((previous, new)) = delta_outcome.container_change {
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
            if !command.particles.is_empty() {
                outcome.particles.extend(command.particles);
            }
            if let Some(landscape_ref) = &mut landscape {
                for op in command.landscape.iter() {
                    op.apply(&mut **landscape_ref);
                }
            }
            if let Some(landscape_ref) = &mut landscape {
                let resolution =
                    (**landscape_ref).resolve_collision(self.state.position, self.state.velocity);
                if resolution.collided {
                    self.state.position = resolution.position;
                    self.state.velocity = resolution.velocity;
                    if let Some(material_id) = resolution.material {
                        if let Some(material) = materials.get_by_id(material_id) {
                            self.apply_material_interaction(material);
                        }
                    }
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
    #[error("player {0} already exists")]
    PlayerAlreadyExists(i32),
    #[error("unknown player {0}")]
    UnknownPlayer(i32),
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
    #[error("object id `{0}` is already in use")]
    DuplicateObjectId(ObjectId),
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
    #[serde(default)]
    pub id: Option<ObjectId>,
    pub definition_id: DefinitionId,
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
    pub action: Option<ActionState>,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    pub owner: i32,
    #[serde(default)]
    pub crew_member: Option<bool>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub alive: Option<bool>,
    #[serde(default)]
    pub category: Option<i32>,
}

impl SpawnConfig {
    pub fn new(definition_id: impl Into<String>) -> Self {
        Self {
            id: None,
            definition_id: definition_id.into(),
            position: Vector2::ZERO,
            velocity: Vector2::ZERO,
            energy: 0,
            action: None,
            direction: Direction::default(),
            command_direction: CommandDirection::default(),
            effects: Vec::new(),
            vertices: Vec::new(),
            owner: OWNER_NONE,
            crew_member: None,
            status: None,
            container: None,
            alive: None,
            category: None,
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

    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn with_command_direction(mut self, command_direction: CommandDirection) -> Self {
        self.command_direction = command_direction;
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

    pub fn with_vertices(mut self, vertices: Vec<ObjectVertex>) -> Self {
        self.vertices = vertices;
        self
    }

    pub fn add_vertex(mut self, vertex: ObjectVertex) -> Self {
        self.vertices.push(vertex);
        self
    }

    pub fn with_owner(mut self, owner: i32) -> Self {
        self.owner = owner;
        self
    }

    pub fn with_category(mut self, category: i32) -> Self {
        self.category = Some(category);
        self
    }

    pub fn with_id(mut self, id: ObjectId) -> Self {
        self.id = Some(id);
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

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = Some(alive);
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
    pub damage: i32,
    #[serde(default)]
    pub action: ActionState,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    #[serde(default)]
    pub action_procedure: Option<String>,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    #[serde(default = "default_category")]
    pub category: i32,
    #[serde(default)]
    pub crew_member: bool,
    #[serde(default = "default_alive")]
    pub alive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationSnapshot {
    pub frame: u64,
    #[serde(default)]
    pub physics: Option<PhysicsSettings>,
    pub objects: Vec<ObjectSnapshot>,
    #[serde(default)]
    pub environment: EnvironmentFrame,
    #[serde(default)]
    pub sky: Option<SkyFrame>,
    #[serde(default)]
    pub weather_events: Vec<WeatherEvent>,
    #[serde(default)]
    pub global_effects: Vec<EffectState>,
    #[serde(default)]
    pub particles: Vec<ParticleSnapshot>,
    #[serde(default)]
    pub players: Vec<PlayerState>,
    #[serde(default)]
    pub crew_selection: HashMap<i32, CrewSelectionState>,
    #[serde(default)]
    pub crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    #[serde(default)]
    pub known_crew_owners: Vec<i32>,
    #[serde(default)]
    pub eliminated_crew_owners: Vec<i32>,
    #[serde(default)]
    pub landscape: Option<Landscape>,
    #[serde(default = "default_rng")]
    pub rng: ChaCha8Rng,
    #[serde(default)]
    pub surfaces: Vec<SurfaceSnapshot>,
    #[serde(default)]
    pub hud: HudSnapshot,
    #[serde(default)]
    pub controls: Vec<String>,
    #[serde(default)]
    pub network_packets: Vec<NetworkPacketSnapshot>,
    #[serde(default)]
    pub definition_categories: HashMap<DefinitionId, i32>,
    #[serde(default)]
    pub transfer_zones: Vec<TransferZoneState>,
    #[serde(default)]
    pub audio: Vec<AudioCommand>,
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
    pub particles: Vec<ParticleSnapshot>,
    #[serde(default)]
    pub players: Vec<PlayerState>,
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
    #[serde(default)]
    pub transfer_zones: Vec<TransferZoneState>,
    #[serde(default)]
    pub messages: Vec<PersistedMessage>,
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

    /// Builds an engine state snapshot from a simulation frame.
    pub fn from_snapshot(snapshot: &SimulationSnapshot) -> Self {
        let physics = snapshot.physics.unwrap_or_else(PhysicsSettings::default);

        let mut objects = Vec::with_capacity(snapshot.objects.len());
        for object in &snapshot.objects {
            objects.push(PersistedObject {
                snapshot: object.clone(),
                command_queue: Vec::new(),
            });
        }

        let mut known_crew_owners = snapshot.known_crew_owners.clone();
        known_crew_owners.sort_unstable();
        known_crew_owners.dedup();

        let mut eliminated_crew_owners = snapshot.eliminated_crew_owners.clone();
        eliminated_crew_owners.sort_unstable();
        eliminated_crew_owners.dedup();

        let next_object_id = snapshot
            .objects
            .iter()
            .map(|object| object.id.as_u64())
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        Self {
            frame: snapshot.frame,
            physics,
            environment: snapshot.environment.settings,
            next_object_id,
            landscape: snapshot.landscape.clone(),
            objects,
            particles: snapshot.particles.clone(),
            players: snapshot.players.clone(),
            crew_selection: snapshot.crew_selection.clone(),
            crew_roles: snapshot.crew_roles.clone(),
            global_effects: snapshot.global_effects.clone(),
            known_crew_owners,
            eliminated_crew_owners,
            transfer_zones: snapshot.transfer_zones.clone(),
            messages: Vec::new(),
            rng: snapshot.rng.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionPicture {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl From<ResourcePictureRect> for DefinitionPicture {
    fn from(rect: ResourcePictureRect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

#[derive(Clone)]
pub struct DefinitionPictureImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl DefinitionPictureImage {
    fn from_resource(image: &lc_resources::GraphicsImage) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            pixels: image.clone_pixels(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    pub fn into_pixels(self) -> Arc<[u8]> {
        self.pixels
    }
}

#[derive(Clone)]
pub struct DefinitionSpriteImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl DefinitionSpriteImage {
    fn from_resource(image: &lc_resources::GraphicsImage) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            pixels: image.clone_pixels(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    pub fn into_pixels(self) -> Arc<[u8]> {
        self.pixels
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionActionFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DefinitionActionGraphics {
    pub facet: Option<DefinitionActionFacet>,
    pub directions: u32,
    pub flip_dir: Option<u32>,
    pub reverse: bool,
    pub facet_base: bool,
    pub facet_top_face: bool,
    pub facet_target_stretch: bool,
    pub length: Option<u32>,
}

pub struct Definition {
    id: DefinitionId,
    name: String,
    script: ScriptEngine,
    has_initialize: bool,
    has_step: bool,
    action_library: ActionLibrary,
    action_graphics: HashMap<String, DefinitionActionGraphics>,
    crew_member: bool,
    movement: MovementProfile,
    category: i32,
    ocf_base: u32,
    value: i32,
    mass: i32,
    picture: Option<DefinitionPicture>,
    picture_image: Option<DefinitionPictureImage>,
    sprite_image: Option<DefinitionSpriteImage>,
}

#[derive(Debug, Clone, Copy)]
enum ActionCallbackKind {
    Start,
    End,
    Phase,
    Abort,
}

impl ActionCallbackKind {
    fn context(self) -> &'static str {
        match self {
            ActionCallbackKind::Start => "action start",
            ActionCallbackKind::End => "action end",
            ActionCallbackKind::Phase => "action phase",
            ActionCallbackKind::Abort => "action abort",
        }
    }
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
            action_graphics: HashMap::new(),
            crew_member: false,
            movement: MovementProfile::default(),
            category: DEFAULT_CATEGORY,
            ocf_base: OCF_NORMAL,
            value: 0,
            mass: 0,
            picture: None,
            picture_image: None,
            sprite_image: None,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.script.has_function(name)
    }

    pub fn from_resource(resource: &ResourceDefinitionData) -> Result<Self, EngineError> {
        let name = resource
            .core
            .name
            .clone()
            .unwrap_or_else(|| resource.core.id.clone());
        let mut definition =
            Definition::from_script(resource.core.id.clone(), name, resource.script.combined())?;

        if let Some(action_map) = &resource.action_map {
            let mut specs = HashMap::new();
            let mut visuals = HashMap::new();
            for (action_name, action_def) in &action_map.actions {
                let (spec, graphics) = Self::convert_action_definition(action_def);
                specs.insert(action_name.clone(), spec);
                visuals.insert(action_name.clone(), graphics);
            }
            let default_action = action_map
                .default_action
                .clone()
                .or_else(|| specs.keys().next().cloned());
            definition.configure_actions(default_action.clone(), specs);
            definition.configure_action_graphics(visuals);
        }

        definition.set_crew_member(resource.core.crew_member);
        definition.set_category(resource.core.category);
        definition.set_value(resource.core.value);
        definition.set_mass(resource.core.mass);
        definition.set_picture(resource.core.picture.map(DefinitionPicture::from));
        if let Some(image) = resource.picture_image.as_ref() {
            definition.set_picture_image(Some(DefinitionPictureImage::from_resource(image)));
        }
        if let Some(image) = resource.graphics_image.as_ref() {
            definition.set_sprite_image(Some(DefinitionSpriteImage::from_resource(image)));
        }
        Ok(definition)
    }

    fn convert_action_definition(
        action: &ResourceActionDefinition,
    ) -> (ActionSpec, DefinitionActionGraphics) {
        let mut spec = ActionSpec::default();
        if let Some(procedure) = &action.procedure {
            spec = spec.with_procedure(procedure.clone());
        }
        if let Some(length) = action.length {
            spec = spec.with_length(length);
        }
        if let Some(next) = &action.next_action {
            spec = spec.with_next(next.clone());
        }
        if let Some(delay) = action.delay {
            spec = spec.with_delay(delay);
        }
        if let Some(step) = action.step {
            spec = spec.with_step(step);
        }
        if let Some(phase) = &action.phase_call {
            spec = spec.with_phase_call(phase.clone());
        }
        if let Some(start) = &action.start_call {
            spec = spec.with_start_call(start.clone());
        }
        if let Some(end) = &action.end_call {
            spec = spec.with_end_call(end.clone());
        }
        if let Some(abort) = &action.abort_call {
            spec = spec.with_abort_call(abort.clone());
        }
        if action.no_other_action {
            spec = spec.with_no_other_action(true);
        }
        if let Some(dig_free) = action.dig_free {
            spec = spec.with_dig_free(dig_free);
        }
        let mut graphics = DefinitionActionGraphics::default();
        graphics.length = action.length;
        graphics.directions = action.directions.unwrap_or(1).max(1);
        graphics.flip_dir = action.flip_dir;
        graphics.reverse = action.reverse;
        graphics.facet_base = action.facet_base;
        graphics.facet_top_face = action.facet_top_face;
        graphics.facet_target_stretch = action.facet_target_stretch;
        graphics.facet = action.facet.as_ref().map(Self::convert_action_facet);
        (spec, graphics)
    }

    fn convert_action_facet(facet: &ResourceActionFacet) -> DefinitionActionFacet {
        DefinitionActionFacet {
            x: facet.x,
            y: facet.y,
            width: facet.width,
            height: facet.height,
            target_x: facet.target_x,
            target_y: facet.target_y,
        }
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

    pub fn configure_action_graphics(
        &mut self,
        graphics: HashMap<String, DefinitionActionGraphics>,
    ) {
        self.action_graphics = graphics;
    }

    pub fn action_library(&self) -> &ActionLibrary {
        &self.action_library
    }

    pub fn action_graphics(&self) -> &HashMap<String, DefinitionActionGraphics> {
        &self.action_graphics
    }

    pub fn graphics_for_action(&self, action: &str) -> Option<&DefinitionActionGraphics> {
        self.action_graphics.get(action)
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

    pub fn movement_profile(&self) -> MovementProfile {
        self.movement
    }

    pub fn set_movement_profile(&mut self, movement: MovementProfile) {
        self.movement = movement;
    }

    pub fn category(&self) -> i32 {
        self.category
    }

    pub fn set_category(&mut self, category: i32) {
        self.category = normalize_category(category, DEFAULT_CATEGORY);
    }

    pub fn ocf_base(&self) -> u32 {
        self.ocf_base
    }

    pub fn set_ocf_base(&mut self, ocf: u32) {
        self.ocf_base = ocf | OCF_NORMAL;
    }

    pub fn compute_ocf(&self, state: &ObjectState) -> u32 {
        crate::ocf::compute(
            self.ocf_base,
            self.crew_member,
            state.alive,
            state.status,
            state.container.is_some(),
        )
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn set_value(&mut self, value: i32) {
        self.value = value.max(0);
    }

    pub fn mass(&self) -> i32 {
        self.mass
    }

    pub fn set_mass(&mut self, mass: i32) {
        self.mass = mass.max(0);
    }

    pub fn picture(&self) -> Option<DefinitionPicture> {
        self.picture
    }

    pub fn set_picture(&mut self, picture: Option<DefinitionPicture>) {
        self.picture = picture;
    }

    pub fn picture_image(&self) -> Option<&DefinitionPictureImage> {
        self.picture_image.as_ref()
    }

    pub fn set_picture_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.picture_image = image;
    }

    pub fn sprite_image(&self) -> Option<&DefinitionSpriteImage> {
        self.sprite_image.as_ref()
    }

    pub fn set_sprite_image(&mut self, image: Option<DefinitionSpriteImage>) {
        self.sprite_image = image;
    }

    fn call_initialize(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        random: i32,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(CommandBatch, AudioRegistry, ChaCha8Rng, u64), EngineError> {
        if !self.has_initialize {
            return Ok((CommandBatch::default(), audio, rng, world.next_object_id()));
        }
        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::Int(random),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.owner,
                    state.position,
                    state.velocity,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                )
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            || self.script.call("Initialize", &args),
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
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
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            audio: host_audio,
            next_object_id,
        } = host_effects;
        batch.audio.extend(host_audio.events);

        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if let Some(update) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &update);
        }

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
        if !host_spawns.is_empty() {
            batch.spawns.extend(host_spawns);
        }
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_transfer_zones.is_empty() {
            batch.transfer_zones.extend(host_transfer_zones);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id))
    }

    fn call_step(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        frame: u64,
        random: i32,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(CommandBatch, AudioRegistry, ChaCha8Rng, u64), EngineError> {
        if !self.has_step {
            return Ok((CommandBatch::default(), audio, rng, world.next_object_id()));
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
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.owner,
                    state.position,
                    state.velocity,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                )
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            || self.script.call("Step", &args),
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
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
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            audio: host_audio,
            next_object_id,
        } = host_effects;
        batch.audio.extend(host_audio.events);

        if let Some(delta) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &delta);
        }
        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }

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
        if !host_spawns.is_empty() {
            batch.spawns.extend(host_spawns);
        }
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_transfer_zones.is_empty() {
            batch.transfer_zones.extend(host_transfer_zones);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id))
    }

    fn call_action_callback(
        &self,
        function: &str,
        kind: ActionCallbackKind,
        state: &ObjectState,
        object_id: ObjectId,
        action_name: &str,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(compat::EffectContextOutcome, AudioRegistry, ChaCha8Rng), EngineError> {
        if !self.script.has_function(function) {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: kind.context(),
                detail: format!("callback `{}` is not defined", function),
            });
        }

        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::String(action_name.to_string()),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.owner,
                    state.position,
                    state.velocity,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                )
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            || self.script.call(function, &args),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let value = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: kind.context(),
            source,
        })?;

        if !matches!(value, Value::Nil) {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: kind.context(),
                detail: format!(
                    "callback `{}` must return nil (got {})",
                    function,
                    value.type_name()
                ),
            });
        }

        let mut host_effects = host_effects;
        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((host_effects, audio_state, rng))
    }

    fn call_menu_entries(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(Vec<ContextMenuEntry>, AudioRegistry, ChaCha8Rng), EngineError> {
        if !self.script.has_function("MenuEntries") {
            return Ok((Vec::new(), audio, rng));
        }

        let args = [build_state_value(
            &self.id,
            object_id,
            state,
            &self.action_library,
        )];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.owner,
            state.position,
            state.velocity,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
        )
        .with_alive(state.alive)
        .with_ocf(self.compute_ocf(state));
        let (result, outcome) = compat::with_effect_context(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            || self.script.call("MenuEntries", &args),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let value = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "MenuEntries",
            source,
        })?;
        let entries = self.parse_context_menu_entries(value)?;

        if !outcome.object.is_empty()
            || !outcome.global.is_empty()
            || outcome.object_update.is_some()
            || !outcome.object_commands.is_empty()
            || outcome.destroy_object
            || outcome.environment.is_some()
            || outcome.physics.is_some()
            || !outcome.spawns.is_empty()
            || !outcome.particles.is_empty()
            || !outcome.transfer_zones.is_empty()
            || !outcome.messages.is_empty()
            || !outcome.audio.events.is_empty()
        {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries",
                detail: "callback must not modify game state".to_string(),
            });
        }
        if !environment_delta.is_empty() || !physics_delta.is_empty() {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries",
                detail: "callback must not modify global state".to_string(),
            });
        }

        let audio_state = audio_guard.finish();
        Ok((entries, audio_state, rng))
    }

    fn call_menu_command(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        kind: MenuCommandKind,
        selection: &MenuCommandSelection,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<
        (
            bool,
            compat::EffectContextOutcome,
            AudioRegistry,
            ChaCha8Rng,
        ),
        EngineError,
    > {
        if !self.script.has_function("MenuCommand") {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::String(kind.as_str().to_string()),
            build_menu_selection_value(selection),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.owner,
            state.position,
            state.velocity,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
        )
        .with_alive(state.alive)
        .with_ocf(self.compute_ocf(state));
        let (result, mut host_effects) = compat::with_effect_context(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            || self.script.call("MenuCommand", &args),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let value = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "MenuCommand",
            source,
        })?;
        let handled = match value {
            Value::Nil => false,
            Value::Bool(flag) => flag,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: self.id.clone(),
                    function: "MenuCommand",
                    detail: format!("expected bool or nil (got {})", other.type_name()),
                })
            }
        };

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((handled, host_effects, audio_state, rng))
    }

    fn call_menu_callback(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<
        (
            bool,
            compat::EffectContextOutcome,
            AudioRegistry,
            ChaCha8Rng,
        ),
        EngineError,
    > {
        if !self.script.has_function(function) {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args = [build_state_value(
            &self.id,
            object_id,
            state,
            &self.action_library,
        )];
        let args_call = args.clone();
        let function_name = function.to_string();
        let function_call = function_name.clone();
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.owner,
            state.position,
            state.velocity,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
        )
        .with_alive(state.alive)
        .with_ocf(self.compute_ocf(state));
        let (result, mut host_effects) = compat::with_effect_context(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            move || self.script.call(&function_call, &args_call),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let value = result.map_err(|source| EngineError::Script {
            definition: format!("{}::{}", self.id, function),
            function: "MenuCallback",
            source,
        })?;
        let handled = match value {
            Value::Nil => false,
            Value::Bool(flag) => flag,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: self.id.clone(),
                    function: "MenuCallback",
                    detail: format!(
                        "callback `{}` must return bool or nil (got {})",
                        function_name,
                        other.type_name()
                    ),
                })
            }
        };

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((handled, host_effects, audio_state, rng))
    }

    fn parse_context_menu_entries(
        &self,
        value: Value,
    ) -> Result<Vec<ContextMenuEntry>, EngineError> {
        let Value::Array(entries) = value else {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries",
                detail: format!("expected array (got {})", value.type_name()),
            });
        };

        let mut result = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            let Value::Proplist(props) = entry else {
                return Err(EngineError::InvalidScriptOutput {
                    definition: self.id.clone(),
                    function: "MenuEntries",
                    detail: format!(
                        "entry {index} must be a proplist (got {})",
                        entry.type_name()
                    ),
                });
            };

            let label = match props.get("label") {
                Some(Value::String(text)) if !text.is_empty() => text.clone(),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries",
                        detail: format!(
                            "entry {index} field `label` must be non-empty string (got {})",
                            other.type_name()
                        ),
                    })
                }
                None => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries",
                        detail: format!("entry {index} missing required field `label`"),
                    })
                }
            };

            let function = match props.get("callback") {
                Some(Value::String(name)) if !name.is_empty() => name.clone(),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries",
                        detail: format!(
                            "entry {index} field `callback` must be non-empty string (got {})",
                            other.type_name()
                        ),
                    })
                }
                None => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries",
                        detail: format!("entry {index} missing required field `callback`"),
                    })
                }
            };

            let description = match props.get("description") {
                Some(Value::String(text)) if text.is_empty() => None,
                Some(Value::String(text)) => Some(text.clone()),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries",
                        detail: format!(
                            "entry {index} field `description` must be string (got {})",
                            other.type_name()
                        ),
                    })
                }
                None => None,
            };

            result.push(ContextMenuEntry {
                function,
                label,
                description,
            });
        }

        Ok(result)
    }

    fn call_effect_start(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, ChaCha8Rng), EngineError> {
        self.dispatch_effect_callback(
            state,
            object_id,
            effect,
            "Start",
            "FxStart",
            Vec::new(),
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            audio,
        )
    }

    fn call_effect_timer(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        frame: u64,
        rng: ChaCha8Rng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, ChaCha8Rng), EngineError> {
        self.dispatch_effect_callback(
            state,
            object_id,
            effect,
            "Timer",
            "FxTimer",
            vec![Value::Int(effect.timer)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            audio,
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
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, ChaCha8Rng), EngineError> {
        self.dispatch_effect_callback(
            state,
            object_id,
            effect,
            "Stop",
            "FxStop",
            vec![effect_stop_reason_value(reason)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            audio,
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
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, ChaCha8Rng), EngineError> {
        let next_object_id = world.next_object_id();
        if !self.script.has_effect_callback(&effect.name, event) {
            return Ok((
                EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
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

        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let audio_guard = enter_audio_context(audio);
        let (result, mut commands) = compat::with_effect_context(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.owner,
                    state.position,
                    state.velocity,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                )
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            || {
                self.script
                    .call_effect_callback(&effect.name, event, &args)
                    .map(|_| ())
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let audio_state = audio_guard.finish();

        result
            .map(|_| {
                if !environment_delta.is_empty() {
                    commands.environment = Some(environment_delta);
                }
                if !physics_delta.is_empty() {
                    commands.physics = Some(physics_delta);
                }
                (commands, audio_state, rng)
            })
            .map_err(|source| EngineError::Script {
                definition: format!("{}::{}::{}", self.id, effect.name, function_label),
                function: "EffectCallback",
                source,
            })
    }
}

struct ScenarioScript {
    name: String,
    script: ScriptEngine,
    has_initialize: bool,
    has_step: bool,
}

impl ScenarioScript {
    fn from_source(name: impl Into<String>, source: &str) -> Result<Self, EngineError> {
        let name = name.into();
        let mut script = ScriptEngine::new();
        script
            .load_script(source)
            .map_err(|source| EngineError::Script {
                definition: name.clone(),
                function: "load",
                source,
            })?;
        compat::register_host_functions(&mut script);
        let has_initialize = script.has_function("Initialize");
        let has_step = script.has_function("Step");
        Ok(Self {
            name,
            script,
            has_initialize,
            has_step,
        })
    }

    fn initialize(
        &mut self,
        snapshot: &SimulationSnapshot,
        rng: ChaCha8Rng,
        random: i32,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
    ) -> Result<(ScenarioBatch, AudioRegistry, ChaCha8Rng), EngineError> {
        if !self.has_initialize {
            return Ok((ScenarioBatch::default(), audio, rng));
        }
        self.call(
            "Initialize",
            snapshot,
            rng,
            random,
            None,
            global_effects,
            physics,
            environment,
            audio,
        )
    }

    fn step(
        &mut self,
        snapshot: &SimulationSnapshot,
        rng: ChaCha8Rng,
        random: i32,
        frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
    ) -> Result<(ScenarioBatch, AudioRegistry, ChaCha8Rng), EngineError> {
        if !self.has_step {
            return Ok((ScenarioBatch::default(), audio, rng));
        }
        self.call(
            "Step",
            snapshot,
            rng,
            random,
            Some(frame),
            global_effects,
            physics,
            environment,
            audio,
        )
    }

    fn call(
        &mut self,
        function: &'static str,
        snapshot: &SimulationSnapshot,
        rng: ChaCha8Rng,
        random: i32,
        frame: Option<u64>,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
    ) -> Result<(ScenarioBatch, AudioRegistry, ChaCha8Rng), EngineError> {
        let state_value = build_scenario_state_value(snapshot);
        let env_frame = frame.unwrap_or(snapshot.frame);
        let mut args = Vec::new();
        args.push(state_value);
        if let Some(frame) = frame {
            let truncated = if frame > i32::MAX as u64 {
                i32::MAX
            } else {
                frame as i32
            };
            args.push(Value::Int(truncated));
        }
        args.push(Value::Int(random));

        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, env_frame);
        let guard = enter_random_context(rng);
        let world = host_world_context_from_snapshot(snapshot);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) =
            compat::with_effect_context(None, global_effects, world, next_object_id, || {
                self.script.call(function, &args)
            });
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        let result = result.map_err(|source| EngineError::Script {
            definition: self.name.clone(),
            function,
            source,
        })?;

        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            audio: host_audio,
            next_object_id: _,
        } = host_effects;

        if !host_object_effects.is_empty()
            || object_update.is_some()
            || !object_commands.is_empty()
            || destroy_object
        {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.name.clone(),
                function,
                detail: "scenario scripts may not enqueue object commands".into(),
            });
        }

        let mut batch = parse_scenario_command(&self.name, function, result)?;
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        if let Some(delta) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &delta);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !host_audio.events.is_empty() {
            batch.audio.extend(host_audio.events);
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCommand {
    PlaySound {
        name: String,
        target: Option<ObjectId>,
        volume: u8,
        looped: bool,
        #[serde(default)]
        custom_falloff: Option<i32>,
    },
    StopSound {
        name: String,
        target: Option<ObjectId>,
    },
    SetSoundVolume {
        name: String,
        target: Option<ObjectId>,
        volume: u8,
    },
}

#[derive(Debug, Default)]
struct CommandBatch {
    delta: ObjectDelta,
    spawns: Vec<SpawnConfig>,
    destroy: bool,
    commands: Vec<QueuedCommand>,
    effects: Vec<EffectCommand>,
    global_effects: Vec<EffectCommand>,
    environment: Option<EnvironmentDelta>,
    physics: Option<PhysicsDelta>,
    particles: Vec<ParticleCommand>,
    transfer_zones: Vec<TransferZoneCommand>,
    audio: Vec<AudioCommand>,
    messages: Vec<MessageCommand>,
}

#[derive(Debug, Default)]
struct ScenarioBatch {
    spawns: Vec<SpawnConfig>,
    global_effects: Vec<EffectCommand>,
    environment: Option<EnvironmentDelta>,
    physics: Option<PhysicsDelta>,
    landscape: Vec<LandscapeCommand>,
    particles: Vec<ParticleCommand>,
    transfer_zones: Vec<TransferZoneCommand>,
    audio: Vec<AudioCommand>,
    messages: Vec<MessageCommand>,
}

pub struct Engine {
    definitions: HashMap<DefinitionId, Definition>,
    materials: MaterialSet,
    objects: Vec<Object>,
    next_object_id: u64,
    rng: ChaCha8Rng,
    frame: u64,
    landscape: Option<Landscape>,
    physics: PhysicsSettings,
    environment: EnvironmentSettings,
    sky: Option<SkyState>,
    global_effects: Vec<EffectState>,
    particles: Vec<ActiveParticle>,
    material_particles: Vec<MaterialParticle>,
    weather_events: Vec<WeatherEvent>,
    scenario_script: Option<ScenarioScript>,
    players: HashMap<i32, Player>,
    crew_selection: HashMap<i32, CrewSelection>,
    crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    team_home_base_rule: bool,
    known_crew_owners: HashSet<i32>,
    eliminated_crew_owners: HashSet<i32>,
    transfer_zones: TransferZoneTable,
    audio_registry: AudioRegistry,
    pending_audio: Vec<AudioCommand>,
    messages: MessageManager,
}

fn clamp_to_limit(value: i32, limit: i32) -> i32 {
    if limit <= 0 {
        0
    } else {
        value.clamp(-limit, limit)
    }
}

fn step_toward(current: i32, desired: i32, step: i32) -> i32 {
    if current == desired || step <= 0 {
        return desired;
    }
    let delta = desired - current;
    if delta.abs() <= step {
        desired
    } else if delta > 0 {
        current.saturating_add(step)
    } else {
        current.saturating_sub(step)
    }
}

fn horizontal_span(vertices: &[ObjectVertex]) -> i32 {
    if vertices.is_empty() {
        return 0;
    }
    let mut min_x = vertices[0].x;
    let mut max_x = vertices[0].x;
    for vertex in vertices.iter().skip(1) {
        if vertex.x < min_x {
            min_x = vertex.x;
        }
        if vertex.x > max_x {
            max_x = vertex.x;
        }
    }
    (max_x - min_x).abs()
}

fn fight_distance_threshold(
    fighter_vertices: &[ObjectVertex],
    target_vertices: &[ObjectVertex],
) -> i32 {
    const MIN_THRESHOLD: i32 = 20;
    let fighter_span = horizontal_span(fighter_vertices);
    let target_span = horizontal_span(target_vertices);
    fighter_span.max(target_span).max(MIN_THRESHOLD)
}

fn apply_horizontal_friction(value: i32, friction: i32) -> i32 {
    if value == 0 || friction == 0 {
        return value;
    }
    let friction = friction.max(0).min(100);
    if friction == 0 {
        return value;
    }
    let magnitude = value.abs();
    let mut retained = magnitude.saturating_mul(100 - friction) / 100;
    if retained == magnitude && friction > 0 {
        retained = magnitude.saturating_sub(1);
    }
    if retained == 0 {
        0
    } else if value > 0 {
        retained
    } else {
        -retained
    }
}

fn apply_float_command_movement(
    velocity: &mut Vector2,
    command_direction: CommandDirection,
    profile: MovementProfile,
) {
    let (dx, dy) = command_direction.axis_components();
    let accel = profile.float_acceleration.max(0);

    if dx != 0 && accel > 0 {
        velocity.x = clamp_to_limit(velocity.x.saturating_add(dx * accel), profile.float_speed);
    } else {
        velocity.x = clamp_to_limit(velocity.x, profile.float_speed);
    }

    if dy != 0 && accel > 0 {
        velocity.y = clamp_to_limit(velocity.y.saturating_add(dy * accel), profile.float_speed);
    } else {
        velocity.y = clamp_to_limit(velocity.y, profile.float_speed);
    }
}

fn decelerate_toward_zero(value: i32, accel: i32) -> i32 {
    if accel <= 0 {
        return value;
    }
    if value > 0 {
        (value - accel).max(0)
    } else if value < 0 {
        (value + accel).min(0)
    } else {
        0
    }
}

fn apply_walk_command_movement(
    velocity: &mut Vector2,
    command_direction: CommandDirection,
    profile: MovementProfile,
) {
    let accel = profile.walk_acceleration.max(0);
    let limit = profile.walk_speed;

    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            if accel > 0 {
                velocity.x = velocity.x.saturating_sub(accel);
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            if accel > 0 {
                velocity.x = velocity.x.saturating_add(accel);
            }
        }
        CommandDirection::Stop | CommandDirection::Up | CommandDirection::Down => {
            if accel > 0 {
                velocity.x = decelerate_toward_zero(velocity.x, accel);
            }
        }
    }

    velocity.x = clamp_to_limit(velocity.x, limit);
}

fn apply_swim_command_movement(
    velocity: &mut Vector2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    gravity_component: i32,
) {
    let accel = profile.swim_acceleration.max(0);
    let limit = profile.swim_speed;

    match command_direction {
        CommandDirection::Stop => {
            if accel > 0 {
                velocity.x = decelerate_toward_zero(velocity.x, accel);
                let vertical_without_gravity = velocity.y - gravity_component;
                let decelerated = decelerate_toward_zero(vertical_without_gravity, accel);
                velocity.y = decelerated + gravity_component;
            }
        }
        _ => {
            if accel > 0 {
                let (dx, dy) = command_direction.axis_components();
                if dx != 0 {
                    velocity.x = velocity.x.saturating_add(dx * accel);
                }
                if dy != 0 {
                    velocity.y = velocity.y.saturating_add(dy * accel);
                }
            }
        }
    }

    velocity.x = clamp_to_limit(velocity.x, limit);
    velocity.y = clamp_to_limit(velocity.y, limit);
}

fn apply_scale_command_movement(
    velocity: &mut Vector2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) {
    let accel = profile.scale_acceleration.max(0);
    let limit = profile.scale_speed;
    let effective_direction = match (facing, command_direction) {
        (Direction::Left, CommandDirection::Left) | (Direction::Right, CommandDirection::Right) => {
            CommandDirection::Up
        }
        _ => command_direction,
    };

    match effective_direction {
        CommandDirection::Up | CommandDirection::UpLeft | CommandDirection::UpRight => {
            if accel > 0 {
                velocity.y = velocity.y.saturating_sub(accel);
            }
        }
        CommandDirection::Down | CommandDirection::DownLeft | CommandDirection::DownRight => {
            if accel > 0 {
                velocity.y = velocity.y.saturating_add(accel);
            }
        }
        CommandDirection::Left | CommandDirection::Right | CommandDirection::Stop => {
            if accel > 0 {
                velocity.y = decelerate_toward_zero(velocity.y, accel);
            }
        }
    }

    velocity.y = clamp_to_limit(velocity.y, limit);
    velocity.x = 0;
}

fn apply_hangle_command_movement(
    velocity: &mut Vector2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) -> Option<Direction> {
    let accel = profile.hangle_acceleration.max(0);
    let limit = profile.hangle_speed;

    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            if accel > 0 {
                velocity.x = velocity.x.saturating_sub(accel);
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            if accel > 0 {
                velocity.x = velocity.x.saturating_add(accel);
            }
        }
        CommandDirection::Up => {
            if accel > 0 {
                if matches!(facing, Direction::Left) {
                    velocity.x = velocity.x.saturating_sub(accel);
                } else {
                    velocity.x = velocity.x.saturating_add(accel);
                }
            }
        }
        CommandDirection::Stop | CommandDirection::Down => {
            if accel > 0 {
                velocity.x = decelerate_toward_zero(velocity.x, accel);
            }
        }
    }

    velocity.x = clamp_to_limit(velocity.x, limit);
    velocity.y = 0;

    if velocity.x < 0 {
        Some(Direction::Left)
    } else if velocity.x > 0 {
        Some(Direction::Right)
    } else {
        None
    }
}

fn apply_dig_command_movement(
    velocity: &mut Vector2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) -> Option<Direction> {
    let speed = profile.dig_speed.max(0);
    let half_speed = speed / 2;

    match command_direction {
        CommandDirection::Stop => {
            velocity.x = 0;
            velocity.y = 0;
            return None;
        }
        CommandDirection::Up => {
            velocity.x = if matches!(facing, Direction::Left) {
                -speed
            } else {
                speed
            };
            velocity.y = -half_speed;
        }
        CommandDirection::UpLeft => {
            velocity.x = -speed;
            velocity.y = -half_speed;
        }
        CommandDirection::Left => {
            velocity.x = -speed;
            velocity.y = 0;
        }
        CommandDirection::DownLeft => {
            velocity.x = -speed;
            velocity.y = speed;
        }
        CommandDirection::Down => {
            velocity.x = 0;
            velocity.y = speed;
        }
        CommandDirection::DownRight => {
            velocity.x = speed;
            velocity.y = speed;
        }
        CommandDirection::Right => {
            velocity.x = speed;
            velocity.y = 0;
        }
        CommandDirection::UpRight => {
            velocity.x = speed;
            velocity.y = -half_speed;
        }
    }

    if velocity.x < 0 {
        Some(Direction::Left)
    } else if velocity.x > 0 {
        Some(Direction::Right)
    } else {
        None
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    pub fn with_seed(seed: u64) -> Self {
        let mut engine = Self {
            definitions: HashMap::new(),
            materials: MaterialSet::default(),
            objects: Vec::new(),
            next_object_id: 1,
            rng: ChaCha8Rng::seed_from_u64(seed),
            frame: 0,
            landscape: None,
            physics: PhysicsSettings::default(),
            environment: EnvironmentSettings::default(),
            sky: None,
            global_effects: Vec::new(),
            particles: Vec::new(),
            material_particles: Vec::new(),
            weather_events: Vec::new(),
            scenario_script: None,
            players: HashMap::new(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            team_home_base_rule: false,
            known_crew_owners: HashSet::new(),
            eliminated_crew_owners: HashSet::new(),
            transfer_zones: TransferZoneTable::default(),
            audio_registry: AudioRegistry::new(),
            pending_audio: Vec::new(),
            messages: MessageManager::new(),
        };
        engine.environment.refresh_runtime_fields();
        engine
    }

    pub fn show_scenario_intro(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let cleaned = trimmed.replace('\r', "");
        let normalized = cleaned
            .split('\n')
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("|");
        let spec = MessageSpec {
            kind: message::MessageKind::Global,
            text: normalized,
            target: None,
            player: None,
            offset: Vector2::new(0, 0),
            color: 0xffff_ffff,
            flags: message::FLAG_TOP | message::FLAG_HCENTER | message::FLAG_ALIGN_CENTER,
            width: Some(400),
            decoration: Some("Mission".to_string()),
            portrait: None,
        };
        self.messages.add_message(spec);
    }

    pub fn register_player(&mut self, config: PlayerConfig) -> Result<(), EngineError> {
        let id = config.id();
        if self.players.contains_key(&id) {
            return Err(EngineError::PlayerAlreadyExists(id));
        }
        let player = config.build();
        self.players.insert(id, player);
        self.sync_player_cursor(id);
        self.sync_team_home_base_for(id);
        Ok(())
    }

    pub fn remove_player(&mut self, id: i32) -> Result<Player, EngineError> {
        let player = self
            .players
            .remove(&id)
            .ok_or(EngineError::UnknownPlayer(id))?;
        self.crew_selection.remove(&id);
        self.crew_roles.remove(&id);
        self.eliminated_crew_owners.remove(&id);
        self.known_crew_owners.remove(&id);
        self.refresh_elimination_state();
        if self.team_home_base_rule {
            if let Some(team) = player.team() {
                self.sync_team_home_base_group(team);
            }
        }
        Ok(player)
    }

    pub fn player(&self, id: i32) -> Option<&Player> {
        self.players.get(&id)
    }

    pub fn player_mut(&mut self, id: i32) -> Result<&mut Player, EngineError> {
        self.players
            .get_mut(&id)
            .ok_or(EngineError::UnknownPlayer(id))
    }

    pub fn players(&self) -> impl Iterator<Item = &Player> {
        self.players.values()
    }

    pub fn set_player_status(&mut self, id: i32, status: PlayerStatus) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_status(status);
        Ok(())
    }

    pub fn set_player_team(&mut self, id: i32, team: Option<i32>) -> Result<(), EngineError> {
        {
            let player = self.player_mut(id)?;
            player.set_team(team);
        }
        self.sync_team_home_base_for(id);
        Ok(())
    }

    pub fn set_player_surrendered(
        &mut self,
        id: i32,
        surrendered: bool,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_surrendered(surrendered);
        Ok(())
    }

    pub fn set_player_wealth(&mut self, id: i32, wealth: i32) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_wealth(wealth);
        Ok(())
    }

    pub fn adjust_player_wealth(&mut self, id: i32, delta: i32) -> Result<i32, EngineError> {
        let player = self.player_mut(id)?;
        Ok(player.adjust_wealth(delta))
    }

    pub fn grant_player_knowledge(
        &mut self,
        id: i32,
        definition_id: impl Into<DefinitionId>,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.grant_knowledge(definition_id.into());
        Ok(())
    }

    pub fn revoke_player_knowledge(
        &mut self,
        id: i32,
        definition_id: &DefinitionId,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.revoke_knowledge(definition_id);
        Ok(())
    }

    pub fn player_inventory(&self, id: i32) -> Result<&HashMap<DefinitionId, u32>, EngineError> {
        self.player(id)
            .map(|player| player.inventory())
            .ok_or(EngineError::UnknownPlayer(id))
    }

    pub fn set_player_inventory_item(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        quantity: u32,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_inventory_item(definition_id, quantity);
        Ok(())
    }

    pub fn adjust_player_inventory_item(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        delta: i32,
    ) -> Result<u32, EngineError> {
        let player = self.player_mut(id)?;
        Ok(player.adjust_inventory_item(definition_id, delta))
    }

    pub fn replace_player_viewports(
        &mut self,
        id: i32,
        viewports: Vec<PlayerViewport>,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.replace_viewports(viewports);
        Ok(())
    }

    pub fn set_player_viewport(
        &mut self,
        id: i32,
        index: usize,
        viewport: PlayerViewport,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_viewport(index, viewport);
        Ok(())
    }

    pub fn set_player_home_base_material(
        &mut self,
        id: i32,
        material: HashMap<DefinitionId, u32>,
    ) -> Result<(), EngineError> {
        {
            let player = self.player_mut(id)?;
            player.set_home_base_material(material);
        }
        self.sync_team_home_base_for(id);
        Ok(())
    }

    pub fn set_player_home_base_production(
        &mut self,
        id: i32,
        production: HashMap<DefinitionId, u32>,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_home_base_production(production);
        Ok(())
    }

    pub fn adjust_player_home_base_material(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        delta: i32,
    ) -> Result<u32, EngineError> {
        let player = self.player_mut(id)?;
        Ok(player.adjust_home_base_material(definition_id, delta))
    }

    pub fn set_materials(&mut self, materials: MaterialSet) {
        self.materials = materials;
        let capacity = self.materials.len();
        for object in &mut self.objects {
            object.ensure_material_capacity(capacity);
        }
        if let Some(landscape) = self.landscape.as_mut() {
            let default = self.materials.default_ground_material();
            landscape.set_default_solid_material(default);
        }
    }

    pub fn materials(&self) -> &MaterialSet {
        &self.materials
    }

    pub fn materials_mut(&mut self) -> &mut MaterialSet {
        &mut self.materials
    }

    pub fn configure_materials_from_library(&mut self, library: &lc_resources::MaterialLibrary) {
        self.materials = MaterialSet::from_resource_library(library);
        let capacity = self.materials.len();
        for object in &mut self.objects {
            object.ensure_material_capacity(capacity);
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn set_landscape(&mut self, mut landscape: Landscape) {
        let default = self.materials.default_ground_material();
        if default.is_some() {
            landscape.set_default_solid_material(default);
        }
        self.landscape = Some(landscape);
    }

    pub fn blast_circle(
        &mut self,
        center: Vector2,
        radius: i32,
        controller: Option<i32>,
    ) -> Option<BlastResult> {
        if radius <= 0 {
            return None;
        }
        let result = {
            let landscape = self.landscape.as_mut()?;
            landscape.blast_circle(center, radius, &self.materials)
        };
        if !result.shift_candidates.is_empty() {
            self.apply_blast_shifts(radius, &result);
        }
        if !result.removed_by_material.is_empty() {
            self.process_blast_reactions(center, controller, &result);
        }
        Some(result)
    }

    pub fn clear_landscape(&mut self) {
        self.landscape = None;
        self.material_particles.clear();
    }

    pub fn landscape(&self) -> Option<&Landscape> {
        self.landscape.as_ref()
    }

    pub fn find_path(
        &self,
        from: Vector2,
        to: Vector2,
        level: i32,
        transfer_zones_enabled: bool,
    ) -> Option<pathfinder::Path> {
        let landscape = self.landscape.as_ref()?;
        let zones = self.transfer_zones.states();
        let mut finder = PathFinder::new(landscape, &zones);
        finder.set_level(level);
        finder.enable_transfer_zones(transfer_zones_enabled);
        finder.find(from, to)
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
        let mut environment = environment;
        environment.refresh_runtime_fields();
        self.environment = environment;
    }

    pub fn set_sky(&mut self, settings: SkySettings) {
        self.sky = Some(SkyState::new(settings));
    }

    pub fn clear_sky(&mut self) {
        self.sky = None;
    }

    pub fn sky_settings(&self) -> Option<&SkySettings> {
        self.sky.as_ref().map(SkyState::settings)
    }

    pub fn team_home_base_rule(&self) -> bool {
        self.team_home_base_rule
    }

    pub fn set_team_home_base_rule(&mut self, enabled: bool) {
        if self.team_home_base_rule == enabled {
            return;
        }
        self.team_home_base_rule = enabled;
        if enabled {
            let ids: Vec<_> = self.players.keys().copied().collect();
            for id in ids {
                self.sync_team_home_base_for(id);
            }
        }
    }

    fn host_world_context(&self) -> HostWorldContext {
        let landscape = self.landscape.clone();
        let definition_metadata: HashMap<DefinitionId, DefinitionMetadata> = self
            .definitions
            .iter()
            .map(|(id, definition)| {
                (
                    id.clone(),
                    DefinitionMetadata {
                        category: definition.category(),
                        ocf_base: definition.ocf_base(),
                        crew_member: definition.is_crew(),
                        value: definition.value(),
                        mass: definition.mass(),
                    },
                )
            })
            .collect();
        let transfer_zones = self.transfer_zones.states();
        let players: HashMap<i32, PlayerState> = self
            .players
            .values()
            .map(|player| (player.id(), player.to_state()))
            .collect();
        HostWorldContext::with_landscape(
            self.objects.iter().map(|object| {
                let definition = self.definitions.get(&object.definition_id);
                let procedure = definition
                    .and_then(|definition| {
                        definition
                            .action_library()
                            .procedure_name_for_action(&object.state.action.name)
                    })
                    .map(|name| name.to_string());
                let ocf = definition
                    .map(|definition| definition.compute_ocf(&object.state))
                    .unwrap_or_else(|| {
                        crate::ocf::compute(
                            OCF_NORMAL,
                            false,
                            object.state.alive,
                            object.state.status,
                            object.state.container.is_some(),
                        )
                    });
                HostWorldObject::with_category(
                    object.id,
                    object.definition_id.clone(),
                    object.state.status,
                    object.state.action.name.clone(),
                    object.state.action.target,
                    object.state.action.target2,
                    procedure,
                    object.state.owner,
                    object.state.category,
                    object.state.energy,
                    object.state.damage,
                    object.state.position,
                    object.state.velocity,
                    object.state.vertices.clone(),
                    object.state.action.data,
                    object.state.action.ticks,
                    object.state.container,
                )
                .with_alive(object.state.alive)
                .with_ocf(ocf)
            }),
            landscape,
            definition_metadata,
            transfer_zones,
            players,
            self.next_object_id,
        )
    }

    pub fn clear_scenario_script(&mut self) {
        self.scenario_script = None;
    }

    pub fn install_scenario_script(
        &mut self,
        name: impl Into<String>,
        source: &str,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let name = name.into();
        let mut script = ScenarioScript::from_source(name, source)?;
        let snapshot = self.snapshot();
        let random = self.next_random_i32();
        let rng_state = self.rng.clone();
        let (batch, audio_state, new_rng) = script.initialize(
            &snapshot,
            rng_state,
            random,
            &self.global_effects,
            self.physics,
            self.environment,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        let created = self.apply_scenario_batch(batch)?;
        self.scenario_script = Some(script);
        Ok(created)
    }

    fn apply_scenario_batch(&mut self, batch: ScenarioBatch) -> Result<Vec<ObjectId>, EngineError> {
        let ScenarioBatch {
            spawns,
            global_effects,
            environment,
            physics,
            landscape,
            particles,
            transfer_zones,
            audio,
            messages,
        } = batch;

        if let Some(delta) = environment {
            delta.apply(&mut self.environment);
        }
        if let Some(delta) = physics {
            self.apply_physics_delta(delta);
        }
        if !global_effects.is_empty() {
            self.apply_global_effect_commands(&global_effects);
        }
        if !landscape.is_empty() {
            let mut landscape_slot = self.landscape.take();
            if let Some(landscape_ref) = landscape_slot.as_mut() {
                for command in landscape {
                    command.apply(landscape_ref);
                }
            }
            self.landscape = landscape_slot;
        }
        self.apply_particle_commands(particles);
        if !transfer_zones.is_empty() {
            self.apply_transfer_zone_commands(transfer_zones)?;
        }
        if !audio.is_empty() {
            self.pending_audio.extend(audio);
        }
        if !messages.is_empty() {
            for command in messages {
                self.messages.apply_command(command);
            }
        }
        let mut created = Vec::with_capacity(spawns.len());
        for spawn in spawns {
            let id = self.spawn_object(spawn)?;
            created.push(id);
        }
        Ok(created)
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
        self.sync_player_cursor(owner);
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
        self.sync_player_cursor(owner);
    }

    pub fn clear_crew_selection(&mut self, owner: i32) {
        if let Some(selection) = self.crew_selection.get_mut(&owner) {
            selection.clear();
        }
        self.crew_selection.remove(&owner);
        self.sync_player_cursor(owner);
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

        self.sync_player_cursor(owner);
        Ok(())
    }

    pub fn context_menu_entries(
        &mut self,
        object_id: ObjectId,
    ) -> Result<Vec<ContextMenuEntry>, EngineError> {
        let index = self
            .objects
            .iter()
            .position(|object| object.id == object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        let definition_id = self.objects[index].definition_id.clone();
        let state_snapshot = self.objects[index].state.clone();
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world = self.host_world_context();
        let (entries, audio_state, new_rng) = definition.call_menu_entries(
            &state_snapshot,
            object_id,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        Ok(entries)
    }

    pub fn menu_command(
        &mut self,
        crew_id: ObjectId,
        kind: MenuCommandKind,
        selection: MenuCommandSelection,
    ) -> Result<bool, EngineError> {
        let index = self
            .objects
            .iter()
            .position(|object| object.id == crew_id)
            .ok_or(EngineError::UnknownObject(crew_id))?;
        let definition_id = self.objects[index].definition_id.clone();
        let state_snapshot = self.objects[index].state.clone();
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition.action_library().clone();
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world = self.host_world_context();
        let (handled, outcome, audio_state, new_rng) = definition.call_menu_command(
            &state_snapshot,
            crew_id,
            kind,
            &selection,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            crew_id,
            &definition_id,
        )?;
        Ok(handled)
    }

    pub fn execute_context_menu(
        &mut self,
        object_id: ObjectId,
        function: &str,
    ) -> Result<bool, EngineError> {
        let index = self
            .objects
            .iter()
            .position(|object| object.id == object_id)
            .ok_or(EngineError::UnknownObject(object_id))?;
        let definition_id = self.objects[index].definition_id.clone();
        let state_snapshot = self.objects[index].state.clone();
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition.action_library().clone();
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world = self.host_world_context();
        let (handled, outcome, audio_state, new_rng) = definition.call_menu_callback(
            &state_snapshot,
            object_id,
            function,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            object_id,
            &definition_id,
        )?;
        Ok(handled)
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

        let mut seen = HashSet::new();
        recipients.retain(|id| seen.insert(*id));
        if recipients.len() > 1 {
            let ordering: HashMap<_, _> = self
                .objects
                .iter()
                .enumerate()
                .map(|(index, object)| (object.id, index))
                .collect();
            recipients.sort_by_key(|id| ordering.get(id).copied().unwrap_or(usize::MAX));
        }
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

    pub fn definition_name(&self, definition_id: &str) -> Option<&str> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.name())
    }

    pub fn definition_value(&self, definition_id: &str) -> Option<i32> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.value())
    }

    pub fn definition_mass(&self, definition_id: &str) -> Option<i32> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.mass())
    }

    pub fn definition_picture(&self, definition_id: &str) -> Option<DefinitionPicture> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.picture())
    }

    pub fn definition_picture_image(&self, definition_id: &str) -> Option<DefinitionPictureImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.picture_image().cloned())
    }

    pub fn definition_sprite_image(&self, definition_id: &str) -> Option<DefinitionSpriteImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.sprite_image().cloned())
    }

    pub fn definition_action_graphics(
        &self,
        definition_id: &str,
    ) -> Option<HashMap<String, DefinitionActionGraphics>> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.action_graphics().clone())
    }

    pub fn definition_ids(&self) -> impl Iterator<Item = &str> {
        self.definitions.keys().map(|id| id.as_str())
    }

    pub fn spawn_object(&mut self, config: SpawnConfig) -> Result<ObjectId, EngineError> {
        let (id, additional) = self.spawn_single(config)?;
        self.process_spawn_queue(additional)?;
        self.refresh_elimination_state();
        Ok(id)
    }

    fn tick_player_systems(&mut self) {
        if self.players.is_empty() {
            return;
        }

        let mut player_ids: Vec<_> = self.players.keys().copied().collect();
        player_ids.sort_unstable();

        let mut team_members: HashMap<i32, Vec<i32>> = HashMap::new();
        let mut team_leaders: HashMap<i32, i32> = HashMap::new();

        if self.team_home_base_rule {
            for id in &player_ids {
                if let Some(team) = self.players.get(id).and_then(|player| player.team()) {
                    team_members.entry(team).or_default().push(*id);
                }
            }
            for (&team, members) in team_members.iter_mut() {
                members.sort_unstable();
                let leader = members
                    .iter()
                    .copied()
                    .find(|member_id| {
                        self.players
                            .get(member_id)
                            .map(|player| {
                                matches!(player.status(), PlayerStatus::Active)
                                    && !player.surrendered()
                            })
                            .unwrap_or(false)
                    })
                    .or_else(|| members.first().copied());
                if let Some(leader_id) = leader {
                    team_leaders.insert(team, leader_id);
                }
            }
        }

        let mut team_updates: HashMap<i32, HashMap<DefinitionId, u32>> = HashMap::new();

        for id in player_ids {
            if let Some(player) = self.players.get_mut(&id) {
                let should_produce = match player.team() {
                    Some(team) if self.team_home_base_rule => {
                        team_leaders.get(&team).copied() == Some(id)
                    }
                    _ => true,
                };
                if should_produce && player.advance_home_base_production() {
                    if let Some(team) = player.team() {
                        if self.team_home_base_rule {
                            team_updates.insert(team, player.home_base_material().clone());
                        }
                    }
                }
            }
        }

        if self.team_home_base_rule && !team_updates.is_empty() {
            for (team, material) in team_updates {
                if let Some(members) = team_members.get(&team) {
                    for member_id in members {
                        if let Some(member) = self.players.get_mut(member_id) {
                            member.set_home_base_material(material.clone());
                        }
                    }
                }
            }
        }

        self.update_player_asset_values();
    }

    pub(crate) fn update_player_asset_values(&mut self) {
        if self.players.is_empty() {
            return;
        }

        let mut totals: HashMap<i32, (i64, u32)> = self
            .players
            .iter()
            .map(|(&id, player)| {
                let base = i64::from(player.points()) + i64::from(player.wealth());
                (id, (base, 0))
            })
            .collect();

        for object in &self.objects {
            if !object.state.status.is_active() {
                continue;
            }
            let owner = object.state.owner;
            if owner == OWNER_NONE {
                continue;
            }
            let Some(entry) = totals.get_mut(&owner) else {
                continue;
            };
            let value = self
                .definitions
                .get(&object.definition_id)
                .map(|definition| definition.value())
                .unwrap_or(0);
            entry.0 = (entry.0 + i64::from(value)).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
            entry.1 = entry.1.saturating_add(1);
        }

        for (id, (value, objects)) in totals {
            if let Some(player) = self.players.get_mut(&id) {
                let clamped = value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                player.update_asset_value(clamped, objects);
            }
        }
    }

    fn tick_weather_events(&mut self, frame: u64) {
        if frame % 10 != 0 {
            return;
        }

        if self.environment.lightning > 0
            && self.rng.gen_range(0..35) == 0
            && self.rng.gen_range(0..100) < self.environment.lightning
        {
            if let Some(width) = self
                .landscape
                .as_ref()
                .map(|landscape| landscape.width() as i32)
            {
                if width > 0 {
                    let position = self.rng.gen_range(0..width);
                    self.weather_events
                        .push(WeatherEvent::Lightning { position });
                }
            }
        }
    }

    pub fn tick(&mut self) -> Result<SimulationSnapshot, EngineError> {
        self.frame += 1;
        let frame = self.frame;
        self.tick_material_particles();
        self.tick_particles();
        self.weather_events.clear();
        self.environment.advance_frame(&mut self.rng);
        if let Some(sky) = &mut self.sky {
            sky.advance(&self.environment);
        }
        let ambient_temperature = self.environment.ambient_temperature(self.frame);
        self.apply_landscape_temperature_conversions(ambient_temperature);
        self.tick_player_systems();
        if self.scenario_script.is_some() {
            let snapshot = self.snapshot();
            let random = self.next_random_i32();
            let rng_state = self.rng.clone();
            let environment = self.environment;
            let global_effects = self.global_effects.clone();
            let (batch, audio_state, new_rng) = {
                let script = self
                    .scenario_script
                    .as_mut()
                    .expect("scenario script must be present");
                script.step(
                    &snapshot,
                    rng_state,
                    random,
                    frame,
                    &global_effects,
                    self.physics,
                    environment,
                    self.audio_registry.clone(),
                )?
            };
            self.rng = new_rng;
            self.audio_registry = audio_state;
            self.apply_scenario_batch(batch)?;
        }
        let mut spawn_requests = Vec::new();
        self.tick_global_effects();
        self.tick_weather_events(frame);
        for idx in 0..self.objects.len() {
            let definition_id = self.objects[idx].definition_id.clone();
            let previous_action_state = self.objects[idx].state.action.clone();
            let previous_action_name = previous_action_state.name.clone();
            let action_library = {
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                definition.action_library().clone()
            };
            let mut landscape_slot = self.landscape.take();
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
                    &self.materials,
                    landscape_slot.as_mut(),
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
            self.landscape = landscape_slot;
            self.update_selection_for_state_change(object_id, previous_owner, new_owner, new_crew);

            for update in container_updates {
                self.apply_container_change(update.object_id, update.previous, update.new)?;
            }

            if !queue_events.is_empty() {
                let object_id = self.objects[idx].id;
                let global_view = self.global_effects.clone();
                let rng_state = self.rng.clone();
                let world = self.host_world_context();
                let (
                    global_cmds,
                    emitted_particles,
                    physics_delta,
                    audio_events,
                    event_messages,
                    audio_state,
                    new_rng,
                ) = {
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
                        &mut self.environment,
                        self.physics,
                        self.frame,
                        world.clone(),
                        self.audio_registry.clone(),
                    )?
                };
                self.rng = new_rng;
                self.audio_registry = audio_state;
                if !audio_events.is_empty() {
                    self.pending_audio.extend(audio_events);
                }
                if !event_messages.is_empty() {
                    for command in event_messages {
                        self.messages.apply_command(command);
                    }
                }
                if !physics_delta.is_empty() {
                    self.apply_physics_delta(physics_delta);
                }
                if !global_cmds.is_empty() {
                    self.apply_global_effect_commands(&global_cmds);
                }
                self.apply_particle_commands(emitted_particles);
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
                let world = self.host_world_context();
                let (
                    global_cmds,
                    emitted_particles,
                    physics_delta,
                    audio_events,
                    event_messages,
                    audio_state,
                    new_rng,
                ) = {
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
                        &mut self.environment,
                        self.physics,
                        self.frame,
                        world.clone(),
                        self.audio_registry.clone(),
                    )?
                };
                self.rng = new_rng;
                self.audio_registry = audio_state;
                if !audio_events.is_empty() {
                    self.pending_audio.extend(audio_events);
                }
                if !event_messages.is_empty() {
                    for command in event_messages {
                        self.messages.apply_command(command);
                    }
                }
                if !physics_delta.is_empty() {
                    self.apply_physics_delta(physics_delta);
                }
                if !global_cmds.is_empty() {
                    self.apply_global_effect_commands(&global_cmds);
                }
                self.apply_particle_commands(emitted_particles);
            }

            let mut pre_phase_state = None;
            if action_library
                .phase_call_for_action(&self.objects[idx].state.action.name)
                .is_some()
            {
                pre_phase_state = Some(self.objects[idx].state.clone());
            }

            let advance_outcome = {
                let object = &mut self.objects[idx];
                object.state.action.advance_with_library(&action_library)
            };

            if self.objects[idx].state.action.name != previous_action_state.name {
                self.objects[idx]
                    .record_action_event(previous_action_state, ActionTransitionKind::Natural);
            }

            if let Some(event) = advance_outcome.phase_event {
                if let Some(function_name) = action_library.phase_call_for_action(&event.action) {
                    let state_snapshot = pre_phase_state
                        .take()
                        .unwrap_or_else(|| self.objects[idx].state.clone());
                    self.invoke_action_callback(
                        idx,
                        ActionCallbackKind::Phase,
                        &event.action,
                        Some(function_name),
                        Some(state_snapshot),
                    )?;
                    if self.objects[idx].destroyed
                        || matches!(self.objects[idx].state.status, ObjectStatus::Deleted)
                    {
                        continue;
                    }
                }
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
            let (command, audio_state, new_rng, next_object_id) = {
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
                    self.physics,
                    self.environment,
                    self.host_world_context(),
                    self.audio_registry.clone(),
                )?
            };
            self.rng = new_rng;
            self.next_object_id = next_object_id;
            self.audio_registry = audio_state;

            let CommandBatch {
                delta,
                spawns,
                destroy,
                commands,
                effects,
                global_effects,
                environment,
                physics,
                particles,
                transfer_zones,
                audio,
                messages,
            } = command;

            if let Some(update) = environment {
                update.apply(&mut self.environment);
            }
            if let Some(delta) = physics {
                self.apply_physics_delta(delta);
            }

            let mut effect_events = Vec::new();
            if !messages.is_empty() {
                for command in messages {
                    self.messages.apply_command(command);
                }
            }
            let (object_id, previous_owner, new_owner, new_crew, container_change) = {
                let object = &mut self.objects[idx];
                let previous_owner = object.state.owner;
                let mut container_change = None;
                let delta_outcome = object.state.apply_delta(&delta, &action_library);
                if let Some(change) = delta_outcome.action_change {
                    object.record_action_event(change.previous, ActionTransitionKind::Forced);
                }
                if let Some(change) = delta_outcome.container_change {
                    container_change = Some(change);
                }
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
            if !audio.is_empty() {
                self.pending_audio.extend(audio);
            }
            self.update_selection_for_state_change(object_id, previous_owner, new_owner, new_crew);
            if let Some((previous_container, new_container)) = container_change {
                self.apply_container_change(object_id, previous_container, new_container)?;
            }

            self.apply_particle_commands(particles);
            if !transfer_zones.is_empty() {
                self.apply_transfer_zone_commands(transfer_zones)?;
            }

            if !global_effects.is_empty() {
                self.apply_global_effect_commands(&global_effects);
            }

            if !effect_events.is_empty() {
                let previous_container = self.objects[idx].state.container;
                let world = self.host_world_context();
                let (global_cmds, emitted_particles, physics_delta, audio_events, event_messages) = {
                    let definition = self
                        .definitions
                        .get(&definition_id)
                        .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                    let global_view = self.global_effects.clone();
                    let rng_state = self.rng.clone();
                    let object = &mut self.objects[idx];
                    let (
                        global_cmds,
                        emitted_particles,
                        physics_delta,
                        audio_events,
                        event_messages,
                        audio_state,
                        new_rng,
                    ) = Self::run_effect_events_for_object(
                        definition,
                        rng_state,
                        object_id,
                        object,
                        effect_events,
                        global_view,
                        &mut self.environment,
                        self.physics,
                        self.frame,
                        world.clone(),
                        self.audio_registry.clone(),
                    )?;
                    self.rng = new_rng;
                    self.audio_registry = audio_state;
                    (
                        global_cmds,
                        emitted_particles,
                        physics_delta,
                        audio_events,
                        event_messages,
                    )
                };
                if !audio_events.is_empty() {
                    self.pending_audio.extend(audio_events);
                }
                if !event_messages.is_empty() {
                    for command in event_messages {
                        self.messages.apply_command(command);
                    }
                }
                if !physics_delta.is_empty() {
                    self.apply_physics_delta(physics_delta);
                }
                let new_container = self.objects[idx].state.container;
                if !global_cmds.is_empty() {
                    self.apply_global_effect_commands(&global_cmds);
                }
                self.apply_particle_commands(emitted_particles);
                if previous_container != new_container {
                    self.apply_container_change(object_id, previous_container, new_container)?;
                }
            }

            self.trigger_action_callbacks(idx, Some(previous_action_name))?;

            if self.objects[idx].destroyed {
                continue;
            }

            self.apply_landscape_at_index(idx);
            spawn_requests.extend(spawns.into_iter());
        }

        self.detach_destroyed_objects()?;
        self.objects.retain(|object| !object.destroyed);
        let alive: HashSet<_> = self.objects.iter().map(|object| object.id).collect();
        self.messages.tick(&alive);
        self.transfer_zones.retain_existing(&alive);
        self.prune_selection();
        self.process_spawn_queue(spawn_requests)?;
        self.refresh_elimination_state();
        let mut snapshot = self.snapshot();
        snapshot.audio = self.pending_audio.drain(..).collect();
        Ok(snapshot)
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
            direction,
            command_direction,
            action,
            status,
            owner,
            crew_member,
            alive,
            container,
            vertices,
            ..
        } = update;

        let definition_id = self.objects[index].definition_id.clone();
        let previous_action_name = self.objects[index].state.action.name.clone();
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
            if let Some(direction) = direction {
                object.state.direction = direction;
            }
            if let Some(command_direction) = command_direction {
                object.state.command_direction = command_direction;
            }
            if let Some(action) = action {
                let previous_action = object.state.action.clone();
                let requested_name_change = action.name.is_some();
                let result = object
                    .state
                    .action
                    .apply_update_with_library(&action, &action_library);
                if matches!(result, ActionUpdateResult::Applied)
                    && (requested_name_change || object.state.action.name != previous_action.name)
                {
                    object.record_action_event(previous_action, ActionTransitionKind::Forced);
                }
            } else {
                object.state.action.reconcile_with_library(&action_library);
            }
            if let Some(owner) = owner {
                object.state.owner = owner;
            }
            if let Some(crew_member) = crew_member {
                object.state.crew_member = crew_member;
            }
            if let Some(alive) = alive {
                object.state.alive = alive;
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
            if let Some(vertices) = vertices {
                object.state.vertices = vertices;
            }

            self.physics.clamp_velocity(&mut object.state.velocity);

            if let Some(landscape) = landscape.as_ref() {
                let resolution =
                    landscape.resolve_collision(object.state.position, object.state.velocity);
                if resolution.collided {
                    object.state.position = resolution.position;
                    object.state.velocity = resolution.velocity;
                    if let Some(material_id) = resolution.material {
                        if let Some(material) = self.materials.get_by_id(material_id) {
                            object.apply_material_interaction(material);
                        }
                    }
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
        self.trigger_action_callbacks(index, Some(previous_action_name))?;
        if self.objects[index].destroyed
            || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
        {
            self.detach_destroyed_objects()?;
        }
        self.refresh_elimination_state();

        Ok(())
    }

    fn trigger_action_callbacks(
        &mut self,
        index: usize,
        previous_action: Option<String>,
    ) -> Result<(), EngineError> {
        if self.objects[index].destroyed {
            return Ok(());
        }

        let mut needs_start = previous_action.is_none();

        while let Some(event) = self.objects[index].pending_action_events.pop_front() {
            let callback_kind = match event.kind {
                ActionTransitionKind::Natural => ActionCallbackKind::End,
                ActionTransitionKind::Forced => ActionCallbackKind::Abort,
            };

            self.invoke_action_callback(index, callback_kind, &event.previous_action, None, None)?;
            if self.objects[index].destroyed
                || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
            {
                return Ok(());
            }

            let current_action = self.objects[index].state.action.name.clone();
            self.invoke_action_callback(
                index,
                ActionCallbackKind::Start,
                &current_action,
                None,
                None,
            )?;
            if self.objects[index].destroyed
                || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
            {
                return Ok(());
            }

            needs_start = false;
        }

        if needs_start {
            let current_action = self.objects[index].state.action.name.clone();
            self.invoke_action_callback(
                index,
                ActionCallbackKind::Start,
                &current_action,
                None,
                None,
            )?;
        }

        Ok(())
    }

    fn invoke_action_callback(
        &mut self,
        index: usize,
        kind: ActionCallbackKind,
        action_name: &str,
        function_override: Option<&str>,
        state_override: Option<ObjectState>,
    ) -> Result<(), EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        let action_library = {
            let definition = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            definition.action_library().clone()
        };

        let function = match function_override {
            Some(name) => Some(name),
            None => match kind {
                ActionCallbackKind::Start => action_library.start_call_for_action(action_name),
                ActionCallbackKind::End => action_library.end_call_for_action(action_name),
                ActionCallbackKind::Phase => action_library.phase_call_for_action(action_name),
                ActionCallbackKind::Abort => action_library.abort_call_for_action(action_name),
            },
        };

        let Some(function) = function else {
            return Ok(());
        };

        let object_id = self.objects[index].id;
        let state_snapshot = match state_override {
            Some(state) => state,
            None => self.objects[index].state.clone(),
        };
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world = self.host_world_context();
        let (outcome, audio_state, new_rng) = definition.call_action_callback(
            function,
            kind,
            &state_snapshot,
            object_id,
            action_name,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;

        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            object_id,
            &definition_id,
        )
    }

    fn apply_action_callback_outcome(
        &mut self,
        index: usize,
        outcome: compat::EffectContextOutcome,
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
    ) -> Result<(), EngineError> {
        let compat::EffectContextOutcome {
            object: object_effects,
            global: global_effects,
            object_update,
            object_commands,
            destroy_object,
            environment,
            physics,
            spawns,
            particles,
            transfer_zones,
            messages,
            audio: outcome_audio,
            next_object_id,
        } = outcome;

        if let Some(update) = environment {
            update.apply(&mut self.environment);
        }
        if let Some(delta) = physics {
            self.apply_physics_delta(delta);
        }

        self.next_object_id = next_object_id;
        if !spawns.is_empty() {
            self.process_spawn_queue(spawns)?;
        }
        self.apply_particle_commands(particles);
        if !transfer_zones.is_empty() {
            self.apply_transfer_zone_commands(transfer_zones)?;
        }

        if !outcome_audio.events.is_empty() {
            self.pending_audio.extend(outcome_audio.events);
        }
        if !messages.is_empty() {
            for command in messages {
                self.messages.apply_command(command);
            }
        }

        let mut effect_events = Vec::new();
        let mut container_changes = Vec::new();

        let (previous_owner, previous_crew_member) = {
            let object = &self.objects[index];
            (object.state.owner, object.state.crew_member)
        };

        {
            let object = &mut self.objects[index];

            if let Some(update) = object_update {
                let delta: ObjectDelta = update.into();
                let outcome = object.state.apply_delta(&delta, action_library);
                if let Some(change) = outcome.action_change {
                    object.record_action_event(change.previous, ActionTransitionKind::Forced);
                }
                if let Some(change) = outcome.container_change {
                    container_changes.push(change);
                }
            } else {
                object.state.action.reconcile_with_library(action_library);
            }

            if destroy_object {
                effect_events.extend(object.mark_destroyed());
            }

            if !object_commands.is_empty() {
                object.enqueue_commands(object_commands);
            }

            if !object_effects.is_empty() {
                let mut applied = object.apply_effect_commands(&object_effects);
                effect_events.append(&mut applied);
            }

            self.physics.clamp_velocity(&mut object.state.velocity);
        }

        let (new_owner, new_crew_member) = {
            let object = &self.objects[index];
            (object.state.owner, object.state.crew_member)
        };

        if previous_owner != new_owner || previous_crew_member != new_crew_member {
            self.update_selection_for_state_change(
                object_id,
                previous_owner,
                new_owner,
                new_crew_member,
            );
        }

        if !global_effects.is_empty() {
            self.apply_global_effect_commands(&global_effects);
        }

        if !effect_events.is_empty() {
            let previous_container = self.objects[index].state.container;
            let definition = self
                .definitions
                .get(definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.to_string()))?;
            let global_view = self.global_effects.clone();
            let rng_state = self.rng.clone();
            let world = self.host_world_context();
            let object = &mut self.objects[index];
            let (
                global_cmds,
                emitted_particles,
                physics_delta,
                audio_events,
                event_messages,
                audio_state,
                new_rng,
            ) = Self::run_effect_events_for_object(
                definition,
                rng_state,
                object_id,
                object,
                effect_events,
                global_view,
                &mut self.environment,
                self.physics,
                self.frame,
                world.clone(),
                self.audio_registry.clone(),
            )?;
            self.rng = new_rng;
            self.audio_registry = audio_state;
            if !audio_events.is_empty() {
                self.pending_audio.extend(audio_events);
            }
            if !event_messages.is_empty() {
                for command in event_messages {
                    self.messages.apply_command(command);
                }
            }
            if !physics_delta.is_empty() {
                self.apply_physics_delta(physics_delta);
            }
            if !global_cmds.is_empty() {
                self.apply_global_effect_commands(&global_cmds);
            }
            self.apply_particle_commands(emitted_particles);
            let new_container = self.objects[index].state.container;
            if previous_container != new_container {
                container_changes.push((previous_container, new_container));
            }
        }

        for (previous, new) in container_changes {
            self.apply_container_change(object_id, previous, new)?;
        }

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
        let mut particles: Vec<_> = self
            .particles
            .iter()
            .map(ActiveParticle::snapshot)
            .collect();
        particles.extend(
            self.material_particles
                .iter()
                .map(|particle| particle.snapshot(&self.materials)),
        );
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
        let ambient_temperature = self.environment.ambient_temperature(self.frame);
        let sky_color = self.environment.resolved_sky_color(ambient_temperature);
        let environment = EnvironmentFrame {
            settings: self.environment,
            wind_force: self.environment.wind_force(self.frame),
            ambient_temperature,
            precipitation: self.environment.precipitation(),
            sky_color: Some(sky_color),
        };
        let sky_snapshot = self.sky.as_ref().map(SkyState::snapshot);
        let weather_events = self.weather_events.clone();
        let mut owners: Vec<_> = self
            .players
            .keys()
            .copied()
            .chain(self.known_crew_owners.iter().copied())
            .chain(self.eliminated_crew_owners.iter().copied())
            .collect();
        owners.sort_unstable();
        owners.dedup();
        let mut hud_players = Vec::with_capacity(owners.len());
        for owner in owners {
            let mut crew: Vec<_> = self
                .objects
                .iter()
                .filter(|object| object.state.owner == owner && object.state.crew_member)
                .map(|object| object.id)
                .collect();
            crew.sort_unstable();
            let focus = self
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor());
            let eliminated = self.eliminated_crew_owners.contains(&owner);
            hud_players.push(HudPlayerSnapshot {
                owner,
                crew,
                focus,
                eliminated,
            });
        }
        let mut player_states: Vec<_> = self
            .players
            .values()
            .map(|player| {
                let mut state = player.to_state();
                let owner = player.id();
                let mut crew: Vec<_> = self
                    .objects
                    .iter()
                    .filter(|object| {
                        object.state.owner == owner
                            && object.state.crew_member
                            && object.state.status.is_active()
                    })
                    .map(|object| object.id)
                    .collect();
                crew.sort_unstable();
                state.crew = crew;
                state.cursor = self
                    .crew_selection
                    .get(&owner)
                    .and_then(|selection| selection.cursor())
                    .or(state.cursor);
                if self.eliminated_crew_owners.contains(&owner) {
                    state.status = PlayerStatus::Eliminated;
                } else if state.status == PlayerStatus::Eliminated {
                    state.status = PlayerStatus::Active;
                }
                if state.viewports.is_empty() {
                    let focus_id = state
                        .cursor
                        .or_else(|| state.crew.first().copied())
                        .or_else(|| {
                            self.objects
                                .iter()
                                .find(|object| object.state.owner == owner)
                                .map(|object| object.id)
                        })
                        .or_else(|| self.objects.first().map(|object| object.id));
                    let mut center = Vector2::ZERO;
                    if let Some(focus) =
                        focus_id.and_then(|id| self.objects.iter().find(|object| object.id == id))
                    {
                        center = focus.state.position;
                        state
                            .viewports
                            .push(PlayerViewport::new(center).with_focus(Some(focus.id)));
                    } else {
                        state.viewports.push(PlayerViewport::new(center));
                    }
                }
                state
            })
            .collect();
        player_states.sort_unstable_by_key(|state| state.id);
        let definition_categories = self
            .definitions
            .iter()
            .map(|(id, definition)| (id.clone(), definition.category()))
            .collect();
        let message_snapshots = self.messages.snapshot();
        SimulationSnapshot {
            frame: self.frame,
            physics: Some(self.physics),
            objects,
            environment,
            sky: sky_snapshot,
            weather_events,
            global_effects: self.global_effects.clone(),
            particles,
            players: player_states,
            crew_selection,
            crew_roles,
            known_crew_owners,
            eliminated_crew_owners,
            landscape: self.landscape.clone(),
            rng: self.rng.clone(),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: hud_players,
                messages: message_snapshots,
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories,
            transfer_zones: self.transfer_zones.states(),
            audio: Vec::new(),
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
        let mut particles: Vec<_> = self
            .particles
            .iter()
            .map(ActiveParticle::snapshot)
            .collect();
        particles.extend(
            self.material_particles
                .iter()
                .map(|particle| particle.snapshot(&self.materials)),
        );
        let mut players: Vec<_> = self.players.values().map(Player::to_state).collect();
        players.sort_unstable_by_key(|player| player.id);

        EngineState {
            frame: self.frame,
            physics: self.physics,
            environment: self.environment,
            next_object_id: self.next_object_id,
            landscape: self.landscape.clone(),
            objects,
            particles,
            players,
            crew_selection,
            crew_roles,
            global_effects: self.global_effects.clone(),
            known_crew_owners,
            eliminated_crew_owners,
            transfer_zones: self.transfer_zones.states(),
            messages: self.messages.persisted(),
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
        self.environment.refresh_runtime_fields();
        self.landscape = state.landscape.clone();
        self.rng = state.rng.clone();
        self.objects.clear();
        self.global_effects = state.global_effects.clone();
        self.particles.clear();
        self.material_particles.clear();
        for snapshot in &state.particles {
            if snapshot.definition_id.starts_with("material/pxs/") && snapshot.parameter_b >= 0 {
                if let Some(material) = MaterialId::new(snapshot.parameter_b as usize) {
                    self.material_particles.push(MaterialParticle::new(
                        material,
                        snapshot.position,
                        snapshot.velocity,
                    ));
                }
                continue;
            }
            self.particles
                .push(ActiveParticle::from_snapshot(snapshot.clone()));
        }
        self.transfer_zones = TransferZoneTable::from_states(&state.transfer_zones);
        self.messages.restore(state.messages.clone());
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
                    damage: snapshot.damage,
                    action: snapshot.action.clone(),
                    direction: snapshot.direction,
                    command_direction: snapshot.command_direction,
                    effects: snapshot.effects.clone(),
                    vertices: snapshot.vertices.clone(),
                    container: None,
                    contents: Vec::new(),
                    status: snapshot.status,
                    owner: snapshot.owner,
                    category: snapshot.category,
                    crew_member: snapshot.crew_member,
                    alive: snapshot.alive,
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

        self.players = state
            .players
            .iter()
            .cloned()
            .map(Player::from_state)
            .map(|player| (player.id(), player))
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
        self.sync_all_player_cursors();
        self.refresh_elimination_state();

        if self.team_home_base_rule {
            let mut teams: Vec<i32> = self
                .players
                .values()
                .filter_map(|player| player.team())
                .collect();
            teams.sort_unstable();
            teams.dedup();
            for team in teams {
                self.sync_team_home_base_group(team);
            }
        }

        Ok(())
    }

    pub fn restore_snapshot(&mut self, snapshot: &SimulationSnapshot) -> Result<(), EngineError> {
        let state = EngineState::from_snapshot(snapshot);
        self.restore_state(&state)
    }

    fn run_effect_events_for_object(
        definition: &Definition,
        mut rng: ChaCha8Rng,
        object_id: ObjectId,
        object: &mut Object,
        events: Vec<EffectEvent>,
        mut global_view: Vec<EffectState>,
        environment: &mut EnvironmentSettings,
        physics: PhysicsSettings,
        frame: u64,
        world: HostWorldContext,
        audio: AudioRegistry,
    ) -> Result<
        (
            Vec<EffectCommand>,
            Vec<ParticleCommand>,
            PhysicsDelta,
            Vec<AudioCommand>,
            Vec<MessageCommand>,
            AudioRegistry,
            ChaCha8Rng,
        ),
        EngineError,
    > {
        if events.is_empty() {
            return Ok((
                Vec::new(),
                Vec::new(),
                PhysicsDelta::default(),
                Vec::new(),
                Vec::new(),
                audio,
                rng,
            ));
        }

        let mut queue: VecDeque<EffectEvent> = VecDeque::from(events);
        let mut state_snapshot = object.state.clone();
        let mut global_commands = Vec::new();
        let mut current_environment = *environment;
        let mut current_physics = physics;
        let mut accumulated_physics = PhysicsDelta::default();
        let mut pending_particles = Vec::new();
        let mut pending_audio = Vec::new();
        let mut pending_messages = Vec::new();
        let mut current_audio = audio;

        while let Some(event) = queue.pop_front() {
            let snapshot_for_call = state_snapshot.clone();
            let (outcome, audio_state, new_rng) = match event.kind {
                EffectEventKind::Started => definition.call_effect_start(
                    &snapshot_for_call,
                    object_id,
                    &event.effect,
                    rng,
                    &global_view,
                    current_physics,
                    current_environment,
                    frame,
                    world.clone(),
                    current_audio,
                )?,
                EffectEventKind::Timer => definition.call_effect_timer(
                    &snapshot_for_call,
                    object_id,
                    &event.effect,
                    frame,
                    rng,
                    &global_view,
                    current_physics,
                    current_environment,
                    world.clone(),
                    current_audio,
                )?,
                EffectEventKind::Stopped(reason) => definition.call_effect_stop(
                    &snapshot_for_call,
                    object_id,
                    &event.effect,
                    reason,
                    rng,
                    &global_view,
                    current_physics,
                    current_environment,
                    frame,
                    world.clone(),
                    current_audio,
                )?,
            };
            rng = new_rng;
            current_audio = audio_state;

            let compat::EffectContextOutcome {
                object: object_effect_commands,
                global: mut global_effect_commands,
                object_update,
                object_commands,
                destroy_object,
                environment: environment_update,
                physics: physics_update,
                particles: mut emitted_particles,
                messages: event_messages,
                audio: outcome_audio,
                ..
            } = outcome;

            if let Some(update) = environment_update {
                update.apply(&mut current_environment);
            }
            if let Some(update) = physics_update {
                merge_physics_delta(&mut accumulated_physics, &update);
                update.apply(&mut current_physics);
            }

            if let Some(update) = object_update {
                let mut delta = ObjectDelta::default();
                delta.merge_update(update);
                let outcome = object
                    .state
                    .apply_delta(&delta, definition.action_library());
                if let Some(change) = outcome.action_change {
                    object.record_action_event(change.previous, ActionTransitionKind::Forced);
                }
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

            if !emitted_particles.is_empty() {
                pending_particles.append(&mut emitted_particles);
            }

            if !outcome_audio.events.is_empty() {
                pending_audio.extend(outcome_audio.events);
            }
            if !event_messages.is_empty() {
                pending_messages.extend(event_messages);
            }
        }

        *environment = current_environment;

        Ok((
            global_commands,
            pending_particles,
            accumulated_physics,
            pending_audio,
            pending_messages,
            current_audio,
            rng,
        ))
    }

    fn apply_physics_delta(&mut self, delta: PhysicsDelta) {
        if delta.is_empty() {
            return;
        }
        let mut physics = self.physics;
        delta.apply(&mut physics);
        self.set_physics(physics);
    }

    fn apply_landscape(&self, state: &mut ObjectState) {
        if let Some(landscape) = &self.landscape {
            let resolution = landscape.resolve_collision(state.position, state.velocity);
            if resolution.collided {
                state.position = resolution.position;
                state.velocity = resolution.velocity;
                if let Some(material_id) = resolution.material {
                    if let Some(material) = self.materials.get_by_id(material_id) {
                        let friction = material.friction();
                        if friction != 0 {
                            state.velocity.x =
                                apply_horizontal_friction(state.velocity.x, friction);
                            for vertex in &mut state.vertices {
                                vertex.friction = friction;
                            }
                        }
                    }
                }
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
        self.sync_player_cursor(previous_owner);
        self.sync_player_cursor(new_owner);
    }

    fn remove_from_selection(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(selection) = self.crew_selection.get_mut(&owner) {
            selection.deselect(object_id);
            if selection.is_empty() {
                self.crew_selection.remove(&owner);
            }
        }
        self.sync_player_cursor(owner);
    }

    fn remove_from_roles(&mut self, owner: i32, object_id: ObjectId) {
        if let Some(assignments) = self.crew_roles.get_mut(&owner) {
            assignments.remove(&object_id);
            if assignments.is_empty() {
                self.crew_roles.remove(&owner);
            }
        }
        self.sync_player_cursor(owner);
    }

    fn sync_player_cursor(&mut self, owner: i32) {
        if let Some(player) = self.players.get_mut(&owner) {
            let cursor = self
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor());
            player.set_cursor(cursor);
        }
    }

    fn sync_all_player_cursors(&mut self) {
        let owners: Vec<i32> = self.players.keys().copied().collect();
        for owner in owners {
            self.sync_player_cursor(owner);
        }
    }

    fn sync_team_home_base_for(&mut self, id: i32) {
        if !self.team_home_base_rule {
            return;
        }
        let team = match self.players.get(&id).and_then(|player| player.team()) {
            Some(team) => team,
            None => return,
        };
        self.sync_team_home_base_group(team);
    }

    fn sync_team_home_base_group(&mut self, team: i32) {
        if !self.team_home_base_rule {
            return;
        }
        let mut members: Vec<i32> = self
            .players
            .iter()
            .filter_map(|(&player_id, player)| {
                if player.team() == Some(team) {
                    Some(player_id)
                } else {
                    None
                }
            })
            .collect();
        if members.len() <= 1 {
            return;
        }
        members.sort_unstable();
        let leader_id = members[0];
        let material = match self.players.get(&leader_id) {
            Some(leader) => leader.home_base_material().clone(),
            None => return,
        };
        for member_id in members.into_iter().skip(1) {
            if let Some(member) = self.players.get_mut(&member_id) {
                member.set_home_base_material(material.clone());
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
        self.sync_all_player_cursors();
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
        if self.objects.is_empty() && self.known_crew_owners.is_empty() && self.players.is_empty() {
            return;
        }

        let mut active_alive = HashSet::new();
        let mut crew_map: HashMap<i32, Vec<ObjectId>> = HashMap::new();
        for object in &self.objects {
            if !object.state.crew_member {
                continue;
            }
            let owner = object.state.owner;
            if owner == OWNER_NONE {
                continue;
            }
            self.known_crew_owners.insert(owner);
            if object.state.status.is_active() {
                crew_map.entry(owner).or_default().push(object.id);
            }
            if object.state.status.is_active() && object.state.alive {
                active_alive.insert(owner);
            }
        }

        for crew in crew_map.values_mut() {
            crew.sort_unstable_by_key(|id| id.as_u64());
        }

        if !self.players.is_empty() {
            for (&owner, player) in self.players.iter_mut() {
                let crew = crew_map.get(&owner).cloned().unwrap_or_default();
                player.set_crew(crew);
            }
        }

        let mut known: Vec<i32> = self.known_crew_owners.iter().copied().collect();
        known.sort_unstable();
        known.dedup();

        for owner in known {
            if active_alive.contains(&owner) {
                self.eliminated_crew_owners.remove(&owner);
                if let Some(player) = self.players.get_mut(&owner) {
                    if player.status() == PlayerStatus::Eliminated {
                        player.set_status(PlayerStatus::Active);
                    }
                }
            } else {
                self.eliminated_crew_owners.insert(owner);
                if let Some(player) = self.players.get_mut(&owner) {
                    player.set_status(PlayerStatus::Eliminated);
                }
            }
        }
    }

    fn apply_physics_at_index(&mut self, idx: usize) {
        if idx >= self.objects.len() {
            return;
        }

        let definition_id = self.objects[idx].definition_id.clone();
        let command_direction = self.objects[idx].state.command_direction;
        let action_target = self.objects[idx].state.action.target;

        let (procedure, movement_profile, gravity_component) = {
            if let Some(definition) = self.definitions.get(&definition_id) {
                let object = &self.objects[idx];
                let procedure = definition
                    .action_library()
                    .procedure_for_action(&object.state.action.name);
                let gravity = procedure.gravity_component(self.physics.gravity);
                (procedure, definition.movement_profile(), gravity)
            } else {
                let procedure = ActionProcedure::default();
                let gravity = procedure.gravity_component(self.physics.gravity);
                (procedure, MovementProfile::default(), gravity)
            }
        };

        if matches!(procedure, ActionProcedure::Dig) {
            self.apply_dig_procedure(idx, &definition_id);
        }

        if matches!(procedure, ActionProcedure::Bridge) {
            if !self.apply_bridge_procedure(idx, command_direction, &definition_id) {
                return;
            }
        }

        if matches!(procedure, ActionProcedure::Fight) {
            if !self.apply_fight_procedure(idx, movement_profile, &definition_id) {
                return;
            }
        }

        if matches!(procedure, ActionProcedure::Attach) {
            if !self.apply_attach_procedure(idx, &definition_id) {
                return;
            }
        }

        let mut push_handled = false;
        if matches!(procedure, ActionProcedure::Push) {
            if !self.apply_push_procedure(idx, command_direction, movement_profile, &definition_id)
            {
                return;
            }
            push_handled = true;
        }

        let mut pull_handled = false;
        if matches!(procedure, ActionProcedure::Pull) {
            if !self.apply_pull_procedure(idx, command_direction, movement_profile, &definition_id)
            {
                return;
            }
            pull_handled = true;
        }

        {
            let object = &mut self.objects[idx];
            object.state.velocity.y = object.state.velocity.y.saturating_add(gravity_component);
            if procedure.allows_wind() {
                self.environment
                    .apply_to_velocity(&mut object.state.velocity, self.frame);
            }
            if procedure.locks_vertical_velocity() {
                object.state.velocity.y = 0;
            }
            let mut pending_direction = None;
            match procedure {
                ActionProcedure::Float | ActionProcedure::Flight => {
                    apply_float_command_movement(
                        &mut object.state.velocity,
                        command_direction,
                        movement_profile,
                    );
                }
                ActionProcedure::Swim => {
                    apply_swim_command_movement(
                        &mut object.state.velocity,
                        command_direction,
                        movement_profile,
                        gravity_component,
                    );
                }
                ActionProcedure::Walk => {
                    apply_walk_command_movement(
                        &mut object.state.velocity,
                        command_direction,
                        movement_profile,
                    );
                }
                ActionProcedure::Scale => {
                    apply_scale_command_movement(
                        &mut object.state.velocity,
                        command_direction,
                        movement_profile,
                        object.state.direction,
                    );
                }
                ActionProcedure::Hang => {
                    pending_direction = apply_hangle_command_movement(
                        &mut object.state.velocity,
                        command_direction,
                        movement_profile,
                        object.state.direction,
                    );
                }
                ActionProcedure::Dig => {
                    pending_direction = apply_dig_command_movement(
                        &mut object.state.velocity,
                        command_direction,
                        movement_profile,
                        object.state.direction,
                    );
                }
                ActionProcedure::Push => {
                    if !push_handled {
                        // If push was not handled earlier (shouldn't happen), ensure velocities stay zeroed.
                        object.state.velocity = Vector2::ZERO;
                    }
                }
                ActionProcedure::Pull => {
                    if !pull_handled {
                        object.state.velocity = Vector2::ZERO;
                    }
                }
                _ => {}
            }
            match procedure {
                ActionProcedure::Bridge
                | ActionProcedure::Build
                | ActionProcedure::Attach
                | ActionProcedure::Throw
                | ActionProcedure::Connect
                | ActionProcedure::Chop => {
                    object.state.velocity = Vector2::ZERO;
                }
                _ => {}
            }
            self.physics.clamp_velocity(&mut object.state.velocity);
            match procedure {
                ActionProcedure::Walk => {
                    if object.state.velocity.x < 0 {
                        object.state.direction = Direction::Left;
                    } else if object.state.velocity.x > 0 {
                        object.state.direction = Direction::Right;
                    }
                }
                ActionProcedure::Hang | ActionProcedure::Dig => {
                    if let Some(direction) = pending_direction {
                        object.state.direction = direction;
                    } else if object.state.velocity.x < 0 {
                        object.state.direction = Direction::Left;
                    } else if object.state.velocity.x > 0 {
                        object.state.direction = Direction::Right;
                    }
                }
                ActionProcedure::Push => {
                    if object.state.velocity.x < 0 {
                        object.state.direction = Direction::Left;
                    } else if object.state.velocity.x > 0 {
                        object.state.direction = Direction::Right;
                    }
                }
                ActionProcedure::Pull => {
                    if object.state.velocity.x < 0 {
                        object.state.direction = Direction::Left;
                    } else if object.state.velocity.x > 0 {
                        object.state.direction = Direction::Right;
                    }
                }
                ActionProcedure::Fight => {
                    if object.state.velocity.x < 0 {
                        object.state.direction = Direction::Left;
                    } else if object.state.velocity.x > 0 {
                        object.state.direction = Direction::Right;
                    }
                }
                _ => {}
            }
        }

        if matches!(procedure, ActionProcedure::Lift) {
            if !self.apply_lift_to_target(idx, command_direction, action_target) {
                self.reset_lift_action(idx, &definition_id);
            }
        }
    }

    fn apply_lift_to_target(
        &mut self,
        lifter_idx: usize,
        command_direction: CommandDirection,
        action_target: Option<ObjectId>,
    ) -> bool {
        let target_id = match action_target {
            Some(id) => id,
            None => return false,
        };
        let target_idx = match self.find_object_index(target_id) {
            Some(idx) => idx,
            None => return false,
        };
        if self.objects[target_idx].destroyed
            || !self.objects[target_idx].state.status.is_active()
            || self.objects[target_idx].state.container.is_some()
        {
            return false;
        }

        let base_gravity = self.physics.gravity.abs().max(1);
        let lift_speed = base_gravity.saturating_mul(2).max(2);
        let desired_velocity = match command_direction {
            CommandDirection::Up => -lift_speed,
            CommandDirection::Down => lift_speed,
            CommandDirection::Stop => -self.physics.gravity,
            _ => 0,
        };
        let lift_force = lift_speed.max(1);
        let physics = self.physics;

        let adjust_velocity = |object: &mut Object| {
            let new_velocity = step_toward(object.state.velocity.y, desired_velocity, lift_force);
            object.state.velocity.y = new_velocity;
            physics.clamp_velocity(&mut object.state.velocity);
        };

        if target_idx == lifter_idx {
            let object = &mut self.objects[target_idx];
            adjust_velocity(object);
        } else if target_idx < lifter_idx {
            let (targets, _) = self.objects.split_at_mut(lifter_idx);
            adjust_velocity(&mut targets[target_idx]);
        } else {
            let (_, targets) = self.objects.split_at_mut(target_idx);
            adjust_velocity(&mut targets[0]);
        }

        true
    }

    fn apply_dig_procedure(&mut self, idx: usize, definition_id: &DefinitionId) {
        let materials = &self.materials;
        let landscape = match self.landscape.as_mut() {
            Some(landscape) if landscape.width() > 0 => landscape,
            _ => return,
        };

        let (action_name, requested) = {
            let object = &self.objects[idx];
            (
                object.state.action.name.clone(),
                object.state.action.data != 0,
            )
        };

        let dig_free_value = match self.definitions.get(definition_id).and_then(|definition| {
            definition
                .action_library()
                .dig_free_for_action(&action_name)
        }) {
            Some(value) => value,
            None => return,
        };

        if dig_free_value <= 0 {
            return;
        }

        let mut removal_counts: HashMap<MaterialId, i32> = HashMap::new();

        let (position, (half_width, half_height)) = {
            let object = &self.objects[idx];
            (object.state.position, Self::object_half_extents(object))
        };

        if dig_free_value == 1 {
            let effective_half_width = half_width.max(1);
            let effective_half_height = half_height.max(1);
            let left = position.x - effective_half_width;
            let right = position.x + effective_half_width;
            let bottom = position.y + effective_half_height;
            for column in left..=right {
                if let Some((material_id, removed)) =
                    Self::dig_column(materials, landscape, column, bottom)
                {
                    removal_counts
                        .entry(material_id)
                        .and_modify(|value| *value = value.saturating_add(removed))
                        .or_insert(removed);
                }
            }
        } else {
            let radius = dig_free_value.max(1);
            let center_x = position.x;
            let center_y = position.y.saturating_sub(1);
            let radius_sq = i64::from(radius) * i64::from(radius);
            for offset in -radius..=radius {
                let column = center_x + offset;
                let dx_sq = i64::from(offset) * i64::from(offset);
                if dx_sq > radius_sq {
                    continue;
                }
                let remaining = radius_sq - dx_sq;
                if remaining < 0 {
                    continue;
                }
                let vertical = (remaining as f64).sqrt().floor() as i32;
                let target = center_y.saturating_add(vertical);
                if let Some((material_id, removed)) =
                    Self::dig_column(materials, landscape, column, target)
                {
                    removal_counts
                        .entry(material_id)
                        .and_modify(|value| *value = value.saturating_add(removed))
                        .or_insert(removed);
                }
            }
        }

        if removal_counts.is_empty() {
            return;
        }

        {
            let object = &mut self.objects[idx];
            object.ensure_material_capacity(self.materials.len());
            for (material_id, removed) in &removal_counts {
                object.add_material_content(*material_id, *removed);
            }
        }

        self.process_dig_material_conversions(idx, requested);
    }

    fn dig_column(
        materials: &MaterialSet,
        landscape: &mut Landscape,
        column: i32,
        target_height: i32,
    ) -> Option<(MaterialId, i32)> {
        let width = landscape.width() as i32;
        if column < 0 || width == 0 || column >= width {
            return None;
        }

        if materials.is_empty() {
            landscape.ensure_surface_at_least(column, target_height);
            return None;
        }

        let previous_height = landscape.surface_height(column).unwrap_or(0);
        let Some(material_id) = landscape.solid_material_at(column) else {
            return None;
        };
        let Some(material) = materials.get_by_id(material_id) else {
            return None;
        };
        if !material.dig_free() {
            return None;
        }
        let clamped_target = target_height.max(0);
        if clamped_target <= previous_height {
            return None;
        }

        landscape.ensure_surface_at_least(column, clamped_target);
        let new_height = landscape.surface_height(column).unwrap_or(previous_height);
        let removed = new_height.saturating_sub(previous_height);
        if removed <= 0 {
            return None;
        }

        Some((material_id, removed))
    }

    fn apply_bridge_procedure(
        &mut self,
        idx: usize,
        command_direction: CommandDirection,
        definition_id: &DefinitionId,
    ) -> bool {
        let parameters = BridgeParameters::from_action_data(self.objects[idx].state.action.data);
        let action_time = self.objects[idx].state.action.phase.max(0) as u32;

        if action_time >= parameters.duration {
            self.reset_action_to_default(idx, definition_id, false);
            return false;
        }

        let Some(step_interval) = parameters.step_interval(command_direction) else {
            return true;
        };

        if step_interval == 0 || action_time % step_interval != 0 {
            return true;
        }

        let step_index = (action_time / step_interval) as i32;
        let direction_sign: i32 = match command_direction {
            CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => -1,
            CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => 1,
            _ => 0,
        };

        let base_position = self.objects[idx].state.position;
        let target_column = base_position
            .x
            .saturating_add(direction_sign.saturating_mul(step_index));

        if let Some(landscape) = self.landscape.as_mut() {
            let start = target_column;
            let end = target_column.saturating_add(1);
            landscape.lower_range(start, end, base_position.y);
        }

        if parameters.move_clonk && direction_sign != 0 && action_time > 0 {
            let object = &mut self.objects[idx];
            object.state.position.x = object.state.position.x.saturating_add(direction_sign);
        }

        true
    }

    fn reset_action_to_default(
        &mut self,
        idx: usize,
        definition_id: &DefinitionId,
        clear_targets: bool,
    ) {
        let library = self
            .definitions
            .get(definition_id)
            .map(|definition| definition.action_library().clone())
            .unwrap_or_default();
        let default_action = library.default_action().to_string();
        let previous = self.objects[idx].state.action.clone();

        let update = ActionUpdate {
            name: Some(default_action),
            phase: Some(0),
            ticks: Some(0),
            force: true,
            data: None,
            target: if clear_targets { Some(None) } else { None },
            target2: if clear_targets { Some(None) } else { None },
        };

        let object = &mut self.objects[idx];
        let result = object
            .state
            .action
            .apply_update_with_library(&update, &library);
        if clear_targets {
            object.state.action.target = None;
            object.state.action.target2 = None;
        }
        object.state.command_direction = CommandDirection::Stop;
        object.state.velocity = Vector2::ZERO;
        if matches!(result, ActionUpdateResult::Applied)
            && previous.name != object.state.action.name
        {
            object.record_action_event(previous, ActionTransitionKind::Forced);
        }
    }

    fn reset_lift_action(&mut self, idx: usize, definition_id: &DefinitionId) {
        self.reset_action_to_default(idx, definition_id, true);
    }

    fn apply_attach_procedure(&mut self, idx: usize, definition_id: &DefinitionId) -> bool {
        let Some(target_id) = self.objects[idx].state.action.target else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };

        let Some(target_idx) = self.find_object_index(target_id) else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };

        if target_idx == idx {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let (target_position, target_vertices, target_container, target_destroyed, target_status) = {
            let target = &self.objects[target_idx];
            (
                target.state.position,
                target.state.vertices.clone(),
                target.state.container,
                target.destroyed,
                target.state.status,
            )
        };

        if target_destroyed || !target_status.is_active() {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let (object_id, previous_container, object_vertices, action_data) = {
            let object = &self.objects[idx];
            (
                object.id,
                object.state.container,
                object.state.vertices.clone(),
                object.state.action.data as u32,
            )
        };

        if previous_container != target_container {
            if self
                .apply_container_change(object_id, previous_container, target_container)
                .is_err()
            {
                self.reset_action_to_default(idx, definition_id, true);
                return false;
            }
        }

        let self_vertex_index = ((action_data >> 8) & 0xFF) as usize;
        let target_vertex_index = (action_data & 0xFF) as usize;

        let self_offset = object_vertices
            .get(self_vertex_index)
            .map(|vertex| Vector2::new(vertex.x, vertex.y))
            .unwrap_or(Vector2::ZERO);
        let target_offset = target_vertices
            .get(target_vertex_index)
            .map(|vertex| Vector2::new(vertex.x, vertex.y))
            .unwrap_or(Vector2::ZERO);

        let new_position = Vector2::new(
            target_position
                .x
                .saturating_add(target_offset.x)
                .saturating_sub(self_offset.x),
            target_position
                .y
                .saturating_add(target_offset.y)
                .saturating_sub(self_offset.y),
        );

        let object = &mut self.objects[idx];
        object.state.position = new_position;
        object.state.velocity = Vector2::ZERO;

        true
    }

    fn apply_pull_procedure(
        &mut self,
        idx: usize,
        command_direction: CommandDirection,
        movement_profile: MovementProfile,
        definition_id: &DefinitionId,
    ) -> bool {
        let Some(target_id) = self.objects[idx].state.action.target else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };

        let Some(target_idx) = self.find_object_index(target_id) else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };

        if target_idx == idx {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let puller_container = self.objects[idx].state.container;
        if puller_container == Some(target_id) {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let target_removed = {
            let target = &self.objects[target_idx];
            target.destroyed || matches!(target.state.status, ObjectStatus::Deleted)
        };
        if target_removed {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        if self.objects[target_idx].state.container.is_some() {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let walk_speed = movement_profile.walk_speed.max(0);
        let walk_accel = movement_profile.walk_acceleration.max(0);

        let puller_position = self.objects[idx].state.position;
        let target_position = self.objects[target_idx].state.position;

        let (puller_half_width, _) = Self::object_half_extents(&self.objects[idx]);
        let (target_half_width, target_half_height) =
            Self::object_half_extents(&self.objects[target_idx]);
        let pull_distance = puller_half_width.saturating_add(target_half_width);

        let horizontal_input = Self::horizontal_input_sign(command_direction);
        let base_velocity = horizontal_input.saturating_mul(walk_speed);

        let desired_puller_x = if horizontal_input == 0 {
            puller_position.x
        } else {
            target_position
                .x
                .saturating_add(horizontal_input.saturating_mul(pull_distance))
        };
        let desired_target_x = if horizontal_input == 0 {
            target_position.x
        } else {
            puller_position
                .x
                .saturating_sub(horizontal_input.saturating_mul(pull_distance))
        };

        let desired_target_velocity = Self::desired_pull_velocity(
            target_position.x,
            desired_target_x,
            base_velocity,
            walk_speed,
        );
        let desired_puller_velocity = Self::desired_pull_velocity(
            puller_position.x,
            desired_puller_x,
            base_velocity,
            walk_speed,
        );

        // Mirror C++ pull range tolerance: stay close enough to the target to keep the rope taut.
        let range_extension = puller_half_width.saturating_sub(8).max(0) + 20;
        let horizontal_gap_limit = target_half_width.saturating_add(range_extension);
        let vertical_gap_limit = target_half_height.saturating_add(range_extension);

        let horizontal_gap = (puller_position.x as i64 - target_position.x as i64).abs() as i32;
        let vertical_gap = (puller_position.y as i64 - target_position.y as i64).abs() as i32;
        if horizontal_gap > horizontal_gap_limit || vertical_gap > vertical_gap_limit {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let speed_limit = walk_speed.saturating_mul(2).max(walk_speed);
        let physics = self.physics;

        match idx.cmp(&target_idx) {
            std::cmp::Ordering::Less => {
                let (first, second) = self.objects.split_at_mut(target_idx);
                let puller = &mut first[idx];
                let target = &mut second[0];
                Self::update_pull_pair(
                    puller,
                    target,
                    desired_puller_velocity,
                    desired_target_velocity,
                    speed_limit,
                    walk_accel,
                    physics,
                );
            }
            std::cmp::Ordering::Greater => {
                let (first, second) = self.objects.split_at_mut(idx);
                let target = &mut first[target_idx];
                let puller = &mut second[0];
                Self::update_pull_pair(
                    puller,
                    target,
                    desired_puller_velocity,
                    desired_target_velocity,
                    speed_limit,
                    walk_accel,
                    physics,
                );
            }
            std::cmp::Ordering::Equal => {
                // Should not happen because we guard earlier, but keep the action safe.
                self.reset_action_to_default(idx, definition_id, true);
                return false;
            }
        }

        true
    }

    fn apply_fight_procedure(
        &mut self,
        idx: usize,
        movement_profile: MovementProfile,
        definition_id: &DefinitionId,
    ) -> bool {
        let Some(target_id) = self.objects[idx].state.action.target else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };

        let Some(target_idx) = self.find_object_index(target_id) else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };

        if target_idx == idx {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let target_removed = {
            let target = &self.objects[target_idx];
            target.destroyed || !target.state.status.is_active()
        };
        if target_removed {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let target_definition_id = self.objects[target_idx].definition_id.clone();
        let target_action_name = self.objects[target_idx].state.action.name.clone();
        let target_procedure = self
            .definitions
            .get(&target_definition_id)
            .map(|definition| {
                definition
                    .action_library()
                    .procedure_for_action(&target_action_name)
            })
            .unwrap_or_default();
        if !matches!(target_procedure, ActionProcedure::Fight) {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let fighter_container = self.objects[idx].state.container;
        let target_container = self.objects[target_idx].state.container;
        if fighter_container != target_container {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let fighter_position = self.objects[idx].state.position;
        let target_position = self.objects[target_idx].state.position;
        let fighter_vertices = self.objects[idx].state.vertices.clone();
        let target_vertices = self.objects[target_idx].state.vertices.clone();
        let initial_direction = self.objects[idx].state.direction;

        let threshold = fight_distance_threshold(&fighter_vertices, &target_vertices);
        if (fighter_position.x - target_position.x).abs() > threshold
            || (fighter_position.y - target_position.y).abs() > threshold
        {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let delta_x = target_position.x - fighter_position.x;
        let closeness = (threshold / 2).max(1);
        let walk_speed = movement_profile.walk_speed.max(0);
        let walk_accel = movement_profile.walk_acceleration.max(0);
        let physics = self.physics;

        let desired_direction = if delta_x > 0 {
            Direction::Right
        } else if delta_x < 0 {
            Direction::Left
        } else {
            initial_direction
        };

        let desired_velocity = if walk_speed == 0 || delta_x.abs() <= closeness {
            0
        } else if delta_x > 0 {
            walk_speed
        } else {
            -walk_speed
        };

        let fighter = &mut self.objects[idx];
        fighter.state.direction = desired_direction;
        let new_velocity = step_toward(fighter.state.velocity.x, desired_velocity, walk_accel);
        fighter.state.velocity.x = clamp_to_limit(new_velocity, walk_speed);
        fighter.state.velocity.y = 0;
        physics.clamp_velocity(&mut fighter.state.velocity);

        true
    }

    fn apply_push_procedure(
        &mut self,
        idx: usize,
        command_direction: CommandDirection,
        movement_profile: MovementProfile,
        definition_id: &DefinitionId,
    ) -> bool {
        let Some(target_id) = self.objects[idx].state.action.target else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };
        let Some(target_idx) = self.find_object_index(target_id) else {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        };
        if target_idx == idx {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }
        let target_removed = {
            let target = &self.objects[target_idx];
            target.destroyed || matches!(target.state.status, ObjectStatus::Deleted)
        };
        if target_removed {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }
        if self.objects[target_idx].state.container == Some(self.objects[idx].id) {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let push_speed = movement_profile.walk_speed.max(0);
        let push_accel = movement_profile.walk_acceleration.max(0);
        let straighten = matches!(
            command_direction,
            CommandDirection::Up | CommandDirection::UpLeft | CommandDirection::UpRight
        );
        let desired_target_velocity = match command_direction {
            CommandDirection::Left | CommandDirection::DownLeft | CommandDirection::UpLeft => {
                -push_speed
            }
            CommandDirection::Right | CommandDirection::DownRight | CommandDirection::UpRight => {
                push_speed
            }
            _ => 0,
        };

        let physics = self.physics;

        if idx < target_idx {
            let (first, rest) = self.objects.split_at_mut(target_idx);
            let pusher = &mut first[idx];
            let target = &mut rest[0];
            Self::update_push_pair(
                pusher,
                target,
                desired_target_velocity,
                push_speed,
                push_accel,
                straighten,
                physics,
            );
        } else {
            let (first, rest) = self.objects.split_at_mut(idx);
            let target = &mut first[target_idx];
            let pusher = &mut rest[0];
            Self::update_push_pair(
                pusher,
                target,
                desired_target_velocity,
                push_speed,
                push_accel,
                straighten,
                physics,
            );
        }

        true
    }

    fn update_pull_pair(
        puller: &mut Object,
        target: &mut Object,
        desired_puller_velocity: i32,
        desired_target_velocity: i32,
        speed_limit: i32,
        acceleration: i32,
        physics: PhysicsSettings,
    ) {
        let accel = acceleration.max(0);
        let new_target_velocity =
            step_toward(target.state.velocity.x, desired_target_velocity, accel);
        target.state.velocity.x = clamp_to_limit(new_target_velocity, speed_limit);
        physics.clamp_velocity(&mut target.state.velocity);

        let new_puller_velocity =
            step_toward(puller.state.velocity.x, desired_puller_velocity, accel);
        puller.state.velocity.x = clamp_to_limit(new_puller_velocity, speed_limit);
        puller.state.velocity.y = 0;
        physics.clamp_velocity(&mut puller.state.velocity);
    }

    fn update_push_pair(
        pusher: &mut Object,
        target: &mut Object,
        desired_target_velocity: i32,
        push_speed: i32,
        push_accel: i32,
        straighten: bool,
        physics: PhysicsSettings,
    ) {
        let new_target_velocity =
            step_toward(target.state.velocity.x, desired_target_velocity, push_accel);
        target.state.velocity.x = clamp_to_limit(new_target_velocity, push_speed);
        if straighten && push_accel > 0 {
            target.state.velocity.y = decelerate_toward_zero(target.state.velocity.y, push_accel);
        }
        physics.clamp_velocity(&mut target.state.velocity);

        let mut desired_pusher_velocity = desired_target_velocity;
        if desired_pusher_velocity == 0 {
            let delta = target.state.position.x - pusher.state.position.x;
            let threshold = push_speed.max(1);
            if delta > threshold {
                desired_pusher_velocity = push_speed;
            } else if delta < -threshold {
                desired_pusher_velocity = -push_speed;
            }
        }

        let new_pusher_velocity =
            step_toward(pusher.state.velocity.x, desired_pusher_velocity, push_accel);
        pusher.state.velocity.x = clamp_to_limit(new_pusher_velocity, push_speed);
        pusher.state.velocity.y = 0;
        physics.clamp_velocity(&mut pusher.state.velocity);
    }

    fn desired_pull_velocity(
        current_position: i32,
        desired_position: i32,
        base_velocity: i32,
        walk_speed: i32,
    ) -> i32 {
        let delta = desired_position.saturating_sub(current_position);
        let correction = delta.clamp(-10, 10) / 10;
        base_velocity + walk_speed.saturating_mul(correction)
    }

    fn object_half_extents(object: &Object) -> (i32, i32) {
        if object.state.vertices.is_empty() {
            // Without explicit vertex data fall back to a generous default so pull spacing stays stable.
            return (10, 10);
        }

        let mut min_x = object.state.vertices[0].x;
        let mut max_x = min_x;
        let mut min_y = object.state.vertices[0].y;
        let mut max_y = min_y;
        for vertex in &object.state.vertices {
            if vertex.x < min_x {
                min_x = vertex.x;
            }
            if vertex.x > max_x {
                max_x = vertex.x;
            }
            if vertex.y < min_y {
                min_y = vertex.y;
            }
            if vertex.y > max_y {
                max_y = vertex.y;
            }
        }

        let width = max_x.saturating_sub(min_x);
        let height = max_y.saturating_sub(min_y);
        let half_width = if width <= 0 { 10 } else { (width + 1) / 2 };
        let half_height = if height <= 0 { 10 } else { (height + 1) / 2 };
        (half_width, half_height)
    }

    fn horizontal_input_sign(command_direction: CommandDirection) -> i32 {
        match command_direction {
            CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => -1,
            CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => 1,
            _ => 0,
        }
    }

    fn apply_landscape_at_index(&mut self, idx: usize) {
        if idx >= self.objects.len() {
            return;
        }

        let procedure = {
            let object = &self.objects[idx];
            let definition_id = object.definition_id.clone();
            self.definitions
                .get(&definition_id)
                .map(|definition| {
                    definition
                        .action_library()
                        .procedure_for_action(&object.state.action.name)
                })
                .unwrap_or_default()
        };

        if matches!(procedure, ActionProcedure::Attach) {
            return;
        }

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
        if let Some(material_id) = resolution.material {
            if let Some(material) = self.materials.get_by_id(material_id) {
                object.apply_material_interaction(material);
            }
        }
    }

    fn apply_landscape_temperature_conversions(&mut self, ambient_temperature: i32) {
        if self.materials.is_empty() {
            return;
        }
        if let Some(landscape) = self.landscape.as_mut() {
            landscape.apply_temperature_conversions(&self.materials, ambient_temperature);
        }
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

    fn process_dig_material_conversions(&mut self, idx: usize, requested: bool) {
        if idx >= self.objects.len() || self.materials.is_empty() {
            return;
        }

        let (position, bottom, owner) = {
            let object = &self.objects[idx];
            let (_, half_height) = Self::object_half_extents(object);
            (
                object.state.position,
                object.state.position.y.saturating_add(half_height),
                object.state.owner,
            )
        };

        let mut spawn_requests = Vec::new();

        {
            let object = &mut self.objects[idx];
            object.ensure_material_capacity(self.materials.len());
            for material in self.materials.iter() {
                let Some(definition_id) = material.dig_to_object_name() else {
                    continue;
                };
                let Some(ratio) = material.dig_to_object_ratio() else {
                    continue;
                };
                if ratio <= 0 {
                    continue;
                }
                if material.dig_to_object_on_request_only() && !requested {
                    continue;
                }
                let current = object.material_content(material.id());
                if current < ratio {
                    continue;
                }
                let spawn_count = current / ratio;
                let remainder = current % ratio;
                object.set_material_content(material.id(), remainder);
                if spawn_count <= 0 {
                    continue;
                }
                if !self.definitions.contains_key(definition_id) {
                    continue;
                }
                let spawn_definition = definition_id.to_string();
                let spawn_position = Vector2::new(position.x, bottom);
                for _ in 0..spawn_count {
                    spawn_requests.push(
                        SpawnConfig::new(spawn_definition.clone())
                            .with_position(spawn_position)
                            .with_owner(owner),
                    );
                }
            }
        }

        for config in spawn_requests {
            if let Err(err) = self.spawn_object(config) {
                let _ = err;
            }
        }
    }

    fn process_blast_reactions(
        &mut self,
        center: Vector2,
        controller: Option<i32>,
        result: &BlastResult,
    ) {
        let mut spawn_requests = Vec::new();

        for (material_id, removed) in &result.removed_by_material {
            if *removed <= 0 {
                continue;
            }

            let (
                material_id_value,
                splash_rate,
                blast_to_pxs_ratio,
                blast_to_object_name,
                blast_to_object_ratio,
            ) = match self.materials.get_by_id(*material_id) {
                Some(material) => (
                    material.id(),
                    material.splash_rate(),
                    material.blast_to_pxs_ratio(),
                    material.blast_to_object_name().map(|name| name.to_string()),
                    material.blast_to_object_ratio(),
                ),
                None => continue,
            };

            if let Some(ratio) = blast_to_pxs_ratio {
                if ratio > 0 {
                    let pxs_count = (*removed / ratio).max(0);
                    for _ in 0..pxs_count {
                        let particle = self.build_material_particle(
                            material_id_value,
                            splash_rate,
                            center,
                            &result.affected_columns,
                        );
                        self.material_particles.push(particle);
                    }
                }
            }

            if let (Some(definition_id), Some(ratio)) =
                (blast_to_object_name.as_ref(), blast_to_object_ratio)
            {
                if ratio <= 0 {
                    continue;
                }
                if !self.definitions.contains_key(definition_id) {
                    continue;
                }
                let spawn_count = (*removed / ratio).max(0);
                if spawn_count <= 0 {
                    continue;
                }
                let owner = controller.unwrap_or(OWNER_NONE);
                for _ in 0..spawn_count {
                    let velocity =
                        Vector2::new(self.rng.gen_range(-3..=3), self.rng.gen_range(-7..=-1));
                    spawn_requests.push(
                        SpawnConfig::new(definition_id.clone())
                            .with_position(center)
                            .with_velocity(velocity)
                            .with_owner(owner),
                    );
                }
            }
        }

        if !spawn_requests.is_empty() {
            for config in spawn_requests {
                if let Err(err) = self.spawn_object(config) {
                    let _ = err;
                }
            }
        }
    }

    fn apply_blast_shifts(&mut self, radius: i32, result: &BlastResult) {
        if result.shift_candidates.is_empty() {
            return;
        }
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };

        let blast_size = compute_blast_size(radius);
        if blast_size <= 0 {
            return;
        }
        let grade = compute_blast_grade(radius);
        if grade <= 0 {
            return;
        }
        let threshold = (blast_size * grade) / 6;
        if threshold <= 0 {
            return;
        }
        let limit = threshold as u64;

        for candidate in &result.shift_candidates {
            let total_pixels = match result.pixel_count_by_material.get(&candidate.material) {
                Some(value) => *value,
                None => continue,
            };
            if total_pixels <= 0 {
                continue;
            }
            let pixel_count = candidate.pixel_count.max(0);
            if pixel_count <= 0 {
                continue;
            }
            let total_pixels_u64 = match u64::try_from(total_pixels) {
                Ok(value) if value > 0 => value,
                _ => continue,
            };
            let pixel_count_u64 = match u64::try_from(pixel_count) {
                Ok(value) if value > 0 => value,
                _ => continue,
            };

            let should_shift = if limit >= total_pixels_u64 {
                true
            } else if limit == 0 {
                false
            } else {
                let mut success = false;
                for _ in 0..pixel_count_u64 {
                    if self.rng.gen_range(0..total_pixels_u64) < limit {
                        success = true;
                        break;
                    }
                }
                success
            };

            if !should_shift {
                continue;
            }
            if candidate.column < 0 {
                continue;
            }
            let column = candidate.column as u32;
            landscape.set_solid_material(column, Some(candidate.target));
        }
    }

    fn build_material_particle(
        &mut self,
        material: MaterialId,
        _splash_rate: i32,
        center: Vector2,
        affected_columns: &[(i32, i32)],
    ) -> MaterialParticle {
        const LEVEL: i32 = 60;
        let position = if let Some(&(column, height)) = affected_columns.choose(&mut self.rng) {
            FloatVector2::new(
                column as f32 + self.rng.gen_range(-0.5..=0.5),
                height as f32 + self.rng.gen_range(-0.5..=0.5),
            )
        } else {
            FloatVector2::new(
                center.x as f32 + self.rng.gen_range(-0.5..=0.5),
                center.y as f32 + self.rng.gen_range(-0.5..=0.5),
            )
        };
        let velocity = {
            let x_offset = self.rng.gen_range(0..=LEVEL) as f32 - (LEVEL as f32 / 2.0);
            let y_offset = self.rng.gen_range(0..=LEVEL) as f32 - LEVEL as f32;
            FloatVector2::new(x_offset / 10.0, y_offset / 10.0)
        };
        MaterialParticle::new(material, position, velocity)
    }

    fn apply_particle_commands(&mut self, commands: Vec<ParticleCommand>) {
        if commands.is_empty() {
            return;
        }
        for command in commands {
            match command {
                ParticleCommand::Create(config) => {
                    self.particles.push(ActiveParticle::from_config(config));
                }
                ParticleCommand::Clear {
                    definition_id,
                    scope,
                } => {
                    let definition = definition_id.as_deref();
                    match scope {
                        ParticleScope::Global => {
                            self.particles.retain(|particle| {
                                if !matches!(particle.snapshot.layer, ParticleLayer::Global) {
                                    return true;
                                }
                                match definition {
                                    Some(def) => particle.snapshot.definition_id != def,
                                    None => false,
                                }
                            });
                        }
                        ParticleScope::Object(target) => {
                            self.particles.retain(|particle| {
                                let matches_layer = match particle.snapshot.layer {
                                    ParticleLayer::ObjectFront(id)
                                    | ParticleLayer::ObjectBack(id) => id == target,
                                    ParticleLayer::Global => false,
                                };
                                if !matches_layer {
                                    return true;
                                }
                                match definition {
                                    Some(def) => particle.snapshot.definition_id != def,
                                    None => false,
                                }
                            });
                        }
                    }
                }
            }
        }
    }

    fn tick_material_particles(&mut self) {
        if self.material_particles.is_empty() {
            return;
        }
        let Some(width) = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.width() as i32)
        else {
            self.material_particles.clear();
            return;
        };
        if width <= 0 {
            self.material_particles.clear();
            return;
        }
        let max_height = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.surface().iter().copied().max().unwrap_or(0) + 20)
            .unwrap_or(20);
        let top_limit = -10;
        let wind_force = self.environment.wind_force(self.frame) as f32;
        let gravity = self.physics.gravity as f32;
        let mut survivors = Vec::with_capacity(self.material_particles.len());
        let pending_particles = std::mem::take(&mut self.material_particles);
        for mut particle in pending_particles.into_iter() {
            let Some(material) = self.materials.get_by_id(particle.material) else {
                continue;
            };

            particle.velocity.y += gravity / 10.0;

            let drift = material.wind_drift();
            if drift != 0 {
                let drift_factor = drift as f32 / 100.0;
                particle.velocity.x += (wind_force / 10.0) * drift_factor;
            }

            let jitter_scale = (material.splash_rate().max(1) as f32).sqrt();
            let jitter_x = self.rng.gen_range(-0.5..=0.5) * 0.1 * jitter_scale;
            let jitter_y = self.rng.gen_range(-0.5..=0.5) * 0.05 * jitter_scale;
            particle.velocity.x = (particle.velocity.x + jitter_x).clamp(-8.0, 8.0);
            particle.velocity.y = (particle.velocity.y + jitter_y).clamp(-12.0, 12.0);

            let new_position = FloatVector2::new(
                particle.position.x + particle.velocity.x,
                particle.position.y + particle.velocity.y,
            );

            let new_x = new_position.x.floor() as i32;
            let new_y = new_position.y.floor() as i32;
            if new_x < 0 || new_x >= width || new_y < top_limit || new_y > max_height {
                continue;
            }

            let start = Vector2::new(
                particle.position.x.floor() as i32,
                particle.position.y.floor() as i32,
            );
            let end = Vector2::new(new_x, new_y);

            let collision = self
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.first_collision_on_line(start, end));

            let keep = if let Some(hit) = collision {
                let Some(landscape_mut) = self.landscape.as_mut() else {
                    continue;
                };
                let materials = &self.materials;
                let rng = &mut self.rng;
                Self::resolve_material_particle_collision(
                    materials,
                    rng,
                    &mut particle,
                    hit,
                    new_position,
                    landscape_mut,
                )
            } else {
                particle.position = new_position;
                true
            };

            if keep {
                if self.handle_particle_object_collisions(&particle) {
                    survivors.push(particle);
                }
            }
        }

        self.material_particles = survivors;
    }

    fn resolve_material_particle_collision(
        materials: &MaterialSet,
        rng: &mut ChaCha8Rng,
        particle: &mut MaterialParticle,
        hit: Vector2,
        target: FloatVector2,
        landscape: &mut Landscape,
    ) -> bool {
        let landscape_material = landscape.solid_material_at(hit.x);
        let reaction = materials.reaction(Some(particle.material), landscape_material);
        match reaction {
            MaterialReactionKind::None => {
                particle.position = FloatVector2::new(hit.x as f32, (hit.y - 1) as f32);
                particle.velocity.y = 0.0;
                particle.velocity.x *= 0.5;
                true
            }
            MaterialReactionKind::Convert {
                target: Some(target_material),
                ..
            } => {
                particle.material = target_material;
                particle.position = target;
                particle.velocity = FloatVector2::new(0.0, 0.0);
                true
            }
            MaterialReactionKind::Convert { target: None, .. } => false,
            MaterialReactionKind::Poof => {
                landscape.remove_material_at(hit.x, hit.y);
                false
            }
            MaterialReactionKind::Incinerate => {
                let _ = landscape.incinerate_at(hit.x, hit.y, materials);
                false
            }
            MaterialReactionKind::Corrode {
                corrosive_strength,
                corrode_resistance,
            } => {
                let resistance = corrode_resistance.max(1);
                let success = rng.gen_range(0..=resistance) < corrosive_strength.max(1);
                if success {
                    landscape.remove_material_at(hit.x, hit.y);
                } else {
                    landscape.insert_material_at(hit.x, hit.y, particle.material);
                }
                false
            }
            MaterialReactionKind::Insert => {
                landscape.insert_material_at(hit.x, hit.y, particle.material);
                false
            }
        }
    }

    fn handle_particle_object_collisions(&mut self, particle: &MaterialParticle) -> bool {
        let friction = match self.materials.get_by_id(particle.material) {
            Some(material) => material.friction(),
            None => return true,
        };
        if self.objects.is_empty() {
            return true;
        }
        let px = particle.position.x;
        let py = particle.position.y;
        let mut collided = false;
        for object in &mut self.objects {
            if object.destroyed {
                continue;
            }
            if Self::particle_intersects_object(px, py, object) {
                if friction != 0 {
                    object.state.velocity.x =
                        apply_horizontal_friction(object.state.velocity.x, friction);
                    for vertex in &mut object.state.vertices {
                        vertex.friction = friction;
                    }
                }
                collided = true;
            }
        }
        !collided
    }

    fn particle_intersects_object(px: f32, py: f32, object: &Object) -> bool {
        let ox = object.state.position.x as f32;
        let oy = object.state.position.y as f32;
        let (mut half_width, mut half_height) = (4.0f32, 6.0f32);
        if !object.state.vertices.is_empty() {
            let mut min_x = object.state.vertices[0].x;
            let mut max_x = min_x;
            let mut min_y = object.state.vertices[0].y;
            let mut max_y = min_y;
            for vertex in &object.state.vertices {
                if vertex.x < min_x {
                    min_x = vertex.x;
                }
                if vertex.x > max_x {
                    max_x = vertex.x;
                }
                if vertex.y < min_y {
                    min_y = vertex.y;
                }
                if vertex.y > max_y {
                    max_y = vertex.y;
                }
            }
            half_width = ((max_x - min_x).abs() as f32 / 2.0).max(2.0);
            half_height = ((max_y - min_y).abs() as f32 / 2.0).max(2.0);
        }
        px >= ox - half_width
            && px <= ox + half_width
            && py >= oy - half_height
            && py <= oy + half_height
    }

    fn apply_transfer_zone_commands(
        &mut self,
        commands: Vec<TransferZoneCommand>,
    ) -> Result<(), EngineError> {
        for command in commands {
            match command {
                TransferZoneCommand::Set { owner, rect } => {
                    self.set_transfer_zone(owner, rect)?;
                }
                TransferZoneCommand::Clear { owner } => {
                    self.transfer_zones.clear(owner);
                }
            }
        }
        Ok(())
    }

    fn set_transfer_zone(
        &mut self,
        owner: ObjectId,
        rect: TransferZoneRect,
    ) -> Result<(), EngineError> {
        if !self.objects.iter().any(|object| object.id == owner) {
            return Err(EngineError::UnknownObject(owner));
        }
        self.transfer_zones.set(owner, rect);
        Ok(())
    }

    fn tick_particles(&mut self) {
        if self.particles.is_empty() {
            return;
        }
        for particle in &mut self.particles {
            particle.tick();
        }
        self.particles.retain(|particle| !particle.is_expired());
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
            id: explicit_id,
            definition_id,
            position,
            velocity,
            energy,
            action,
            direction,
            command_direction,
            effects,
            vertices,
            owner,
            crew_member,
            status,
            container,
            alive,
            category,
        } = config;

        let (action_library, definition_category, default_action_state, default_crew_member) = {
            let definition_ref = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            (
                definition_ref.action_library().clone(),
                definition_ref.category(),
                definition_ref.default_action_state(),
                definition_ref.is_crew(),
            )
        };
        let mut initial_action = match action {
            Some(state) => state,
            None => default_action_state,
        };
        initial_action.reconcile_with_library(&action_library);
        let initial_crew_member = crew_member.unwrap_or(default_crew_member);

        let id = match explicit_id {
            Some(explicit) => {
                if self.objects.iter().any(|object| object.id == explicit) {
                    return Err(EngineError::DuplicateObjectId(explicit));
                }
                let raw = explicit.as_u64();
                if raw >= self.next_object_id {
                    self.next_object_id = raw + 1;
                }
                explicit
            }
            None => self.next_object_id(),
        };
        let initial_category = category
            .map(|value| normalize_category(value, definition_category))
            .unwrap_or(definition_category);

        let mut object = Object::new(
            id,
            definition_id.clone(),
            ObjectState {
                position,
                velocity,
                energy,
                damage: 0,
                action: initial_action,
                direction,
                command_direction,
                effects: Vec::new(),
                vertices,
                container: None,
                contents: Vec::new(),
                status: status.unwrap_or_default(),
                owner,
                category: initial_category,
                crew_member: initial_crew_member,
                alive: alive.unwrap_or(true),
            },
        );
        object.ensure_material_capacity(self.materials.len());
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
                    environment,
                    physics,
                    particles,
                    transfer_zones,
                    audio,
                    messages,
                },
                audio_state,
                new_rng,
                next_object_id,
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
                    self.physics,
                    self.environment,
                    self.frame,
                    self.host_world_context(),
                    self.audio_registry.clone(),
                )?
            };
            self.rng = new_rng;
            self.next_object_id = next_object_id;
            self.audio_registry = audio_state;
            if let Some(update) = environment {
                update.apply(&mut self.environment);
            }
            if let Some(delta) = physics {
                self.apply_physics_delta(delta);
            }
            if destroy {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition_id.clone(),
                    function: "Initialize",
                    detail: "Initialize may not destroy the object".into(),
                });
            }
            let outcome = object.state.apply_delta(&delta, &action_library);
            if let Some(change) = outcome.action_change {
                object.record_action_event(change.previous, ActionTransitionKind::Forced);
            }
            if let Some(change) = outcome.container_change {
                container_changes.push(change);
            }
            let mut applied = object.apply_effect_commands(&effects);
            effect_events.append(&mut applied);
            self.apply_particle_commands(particles);
            if !transfer_zones.is_empty() {
                self.apply_transfer_zone_commands(transfer_zones)?;
            }
            if !global_effects.is_empty() {
                self.apply_global_effect_commands(&global_effects);
            }
            self.physics.clamp_velocity(&mut object.state.velocity);
            if !commands.is_empty() {
                object.enqueue_commands(commands);
            }
            additional_spawns = spawns;
            if !audio.is_empty() {
                self.pending_audio.extend(audio);
            }
            if !messages.is_empty() {
                for command in messages {
                    self.messages.apply_command(command);
                }
            }
        }

        if !effect_events.is_empty() {
            let definition = self
                .definitions
                .get(&definition_id)
                .expect("definition must exist");
            let global_view = self.global_effects.clone();
            let previous_container = object.state.container;
            let rng_state = self.rng.clone();
            let world = self.host_world_context();
            let (
                global_cmds,
                emitted_particles,
                physics_delta,
                audio_events,
                event_messages,
                audio_state,
                new_rng,
            ) = Self::run_effect_events_for_object(
                definition,
                rng_state,
                id,
                &mut object,
                effect_events,
                global_view,
                &mut self.environment,
                self.physics,
                self.frame,
                world,
                self.audio_registry.clone(),
            )?;
            self.rng = new_rng;
            self.audio_registry = audio_state;
            if !audio_events.is_empty() {
                self.pending_audio.extend(audio_events);
            }
            if !event_messages.is_empty() {
                for command in event_messages {
                    self.messages.apply_command(command);
                }
            }
            if !physics_delta.is_empty() {
                self.apply_physics_delta(physics_delta);
            }
            if !global_cmds.is_empty() {
                self.apply_global_effect_commands(&global_cmds);
            }
            self.apply_particle_commands(emitted_particles);
            if previous_container != object.state.container {
                container_changes.push((previous_container, object.state.container));
            }
        }

        self.apply_landscape(&mut object.state);
        self.objects.push(object);
        for (previous, new) in container_changes {
            self.apply_container_change(id, previous, new)?;
        }
        let index = self.objects.len() - 1;
        self.trigger_action_callbacks(index, None)?;
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
    map.insert(
        "direction".into(),
        Value::Int(state.direction.to_script_value()),
    );
    map.insert(
        "command_direction".into(),
        Value::Int(state.command_direction.to_script_value()),
    );
    map.insert("owner".into(), Value::Int(state.owner));
    map.insert("category".into(), Value::Int(state.category));
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
    let mut action = HashMap::with_capacity(7);
    action.insert("name".into(), Value::String(state.action.name.clone()));
    action.insert("phase".into(), Value::Int(state.action.phase));
    let ticks = (state.action.ticks).min(i32::MAX as u32) as i32;
    action.insert("ticks".into(), Value::Int(ticks));
    action.insert("data".into(), Value::Int(state.action.data));
    match state.action.target {
        Some(target) => {
            action.insert(
                "target".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target".into(), Value::Nil);
        }
    }
    match state.action.target2 {
        Some(target) => {
            action.insert(
                "target2".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target2".into(), Value::Nil);
        }
    }
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

fn build_menu_selection_value(selection: &MenuCommandSelection) -> Value {
    let mut map = HashMap::with_capacity(4);
    map.insert(
        "primary".into(),
        Value::Int(truncate_to_i32(selection.primary_id.as_u64())),
    );
    let instances: Vec<_> = selection
        .instances
        .iter()
        .map(|id| Value::Int(truncate_to_i32(id.as_u64())))
        .collect();
    map.insert("instances".into(), Value::Array(instances));
    map.insert(
        "definition".into(),
        Value::String(selection.definition_id.clone()),
    );
    map.insert("label".into(), Value::String(selection.label.clone()));
    Value::Proplist(map)
}

fn build_object_snapshot_value(snapshot: &ObjectSnapshot) -> Value {
    let mut map = HashMap::with_capacity(11);
    map.insert(
        "definition".into(),
        Value::String(snapshot.definition_id.clone()),
    );
    map.insert(
        "id".into(),
        Value::Int(truncate_to_i32(snapshot.id.as_u64())),
    );
    map.insert("position".into(), snapshot.position.to_value());
    map.insert("velocity".into(), snapshot.velocity.to_value());
    map.insert("energy".into(), Value::Int(snapshot.energy));
    map.insert("damage".into(), Value::Int(snapshot.damage));
    map.insert(
        "direction".into(),
        Value::Int(snapshot.direction.to_script_value()),
    );
    map.insert(
        "command_direction".into(),
        Value::Int(snapshot.command_direction.to_script_value()),
    );
    map.insert("owner".into(), Value::Int(snapshot.owner));
    map.insert("category".into(), Value::Int(snapshot.category));
    map.insert("crew_member".into(), Value::Bool(snapshot.crew_member));
    map.insert(
        "status".into(),
        Value::Int(snapshot.status.to_script_value()),
    );
    match snapshot.container {
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
    let contents: Vec<_> = snapshot
        .contents
        .iter()
        .map(|id| Value::Int(truncate_to_i32(id.as_u64())))
        .collect();
    map.insert("contents".into(), Value::Array(contents));
    let mut action = HashMap::with_capacity(7);
    action.insert("name".into(), Value::String(snapshot.action.name.clone()));
    action.insert("phase".into(), Value::Int(snapshot.action.phase));
    let ticks = snapshot.action.ticks.min(i32::MAX as u32) as i32;
    action.insert("ticks".into(), Value::Int(ticks));
    action.insert("data".into(), Value::Int(snapshot.action.data));
    match snapshot.action.target {
        Some(target) => {
            action.insert(
                "target".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target".into(), Value::Nil);
        }
    }
    match snapshot.action.target2 {
        Some(target) => {
            action.insert(
                "target2".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target2".into(), Value::Nil);
        }
    }
    if let Some(procedure) = &snapshot.action_procedure {
        action.insert("procedure".into(), Value::String(procedure.clone()));
    }
    map.insert("action".into(), Value::Proplist(action));
    let effects: Vec<_> = snapshot.effects.iter().map(build_effect_value).collect();
    map.insert("effects".into(), Value::Array(effects));
    Value::Proplist(map)
}

fn host_world_context_from_snapshot(snapshot: &SimulationSnapshot) -> HostWorldContext {
    let next_object_id = snapshot
        .objects
        .iter()
        .map(|object| object.id.as_u64())
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let definition_metadata: HashMap<DefinitionId, DefinitionMetadata> = snapshot
        .definition_categories
        .iter()
        .map(|(id, category)| {
            (
                id.clone(),
                DefinitionMetadata {
                    category: *category,
                    ocf_base: OCF_NORMAL,
                    crew_member: false,
                    value: 0,
                    mass: 0,
                },
            )
        })
        .collect();
    let players: HashMap<i32, PlayerState> = snapshot
        .players
        .iter()
        .map(|state| (state.id, state.clone()))
        .collect();
    HostWorldContext::with_landscape(
        snapshot.objects.iter().map(|object| {
            HostWorldObject::with_category(
                object.id,
                object.definition_id.clone(),
                object.status,
                object.action.name.clone(),
                object.action.target,
                object.action.target2,
                object.action_procedure.clone(),
                object.owner,
                object.category,
                object.energy,
                object.damage,
                object.position,
                object.velocity,
                object.vertices.clone(),
                object.action.data,
                object.action.ticks,
                object.container,
            )
        }),
        snapshot.landscape.clone(),
        definition_metadata,
        snapshot.transfer_zones.clone(),
        players,
        next_object_id,
    )
}

fn build_scenario_state_value(snapshot: &SimulationSnapshot) -> Value {
    let mut map = HashMap::with_capacity(5);
    let frame_value = if snapshot.frame > i32::MAX as u64 {
        i32::MAX
    } else {
        snapshot.frame as i32
    };
    map.insert("frame".into(), Value::Int(frame_value));
    match snapshot.physics {
        Some(physics) => {
            map.insert("physics".into(), Value::Proplist(physics_to_map(physics)));
        }
        None => {
            map.insert("physics".into(), Value::Nil);
        }
    }
    map.insert(
        "environment".into(),
        Value::Proplist(environment_frame_to_map(&snapshot.environment)),
    );
    let objects: Vec<_> = snapshot
        .objects
        .iter()
        .map(build_object_snapshot_value)
        .collect();
    map.insert("objects".into(), Value::Array(objects));
    let global_effects: Vec<_> = snapshot
        .global_effects
        .iter()
        .map(build_effect_value)
        .collect();
    map.insert("global_effects".into(), Value::Array(global_effects));
    Value::Proplist(map)
}

fn physics_to_map(settings: PhysicsSettings) -> HashMap<String, Value> {
    let mut map = HashMap::with_capacity(4);
    map.insert("gravity".into(), Value::Int(settings.gravity));
    map.insert("max_fall_speed".into(), Value::Int(settings.max_fall_speed));
    map.insert("max_rise_speed".into(), Value::Int(settings.max_rise_speed));
    map.insert(
        "max_horizontal_speed".into(),
        Value::Int(settings.max_horizontal_speed),
    );
    map
}

fn environment_frame_to_map(frame: &EnvironmentFrame) -> HashMap<String, Value> {
    let mut map = HashMap::with_capacity(12);
    let settings = frame.settings;
    map.insert("wind".into(), Value::Int(settings.wind));
    map.insert("wind_variation".into(), Value::Int(settings.wind_variation));
    let wind_period = settings.wind_period.min(i32::MAX as u32) as i32;
    map.insert("wind_period".into(), Value::Int(wind_period));
    map.insert("temperature".into(), Value::Int(settings.temperature));
    map.insert("climate".into(), Value::Int(settings.climate));
    map.insert(
        "temperature_variation".into(),
        Value::Int(settings.temperature_variation),
    );
    let temperature_period = settings.temperature_period.min(i32::MAX as u32) as i32;
    map.insert("temperature_period".into(), Value::Int(temperature_period));
    let temperature_phase = settings.temperature_phase.min(i32::MAX as u32) as i32;
    map.insert("temperature_phase".into(), Value::Int(temperature_phase));
    map.insert(
        "time_of_day".into(),
        Value::Int(i32::from(settings.time_of_day)),
    );
    map.insert(
        "time_speed".into(),
        Value::Int(i32::from(settings.time_speed)),
    );
    map.insert("precipitation".into(), Value::Int(settings.precipitation));
    map.insert("current_wind".into(), Value::Int(frame.wind_force));
    map.insert(
        "ambient_temperature".into(),
        Value::Int(frame.ambient_temperature),
    );
    map.insert(
        "sky_color".into(),
        frame
            .sky_color
            .map(|color| rgb_to_value(color))
            .unwrap_or(Value::Nil),
    );
    map
}

fn rgb_to_value(color: RgbColor) -> Value {
    Value::Array(vec![
        Value::Int(i32::from(color.r)),
        Value::Int(i32::from(color.g)),
        Value::Int(i32::from(color.b)),
    ])
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

fn merge_environment_delta(target: &mut EnvironmentDelta, source: &EnvironmentDelta) {
    if let Some(wind) = source.wind {
        target.wind = Some(wind);
    }
    if let Some(temperature) = source.temperature {
        target.temperature = Some(temperature);
    }
    if let Some(climate) = source.climate {
        target.climate = Some(climate);
    }
}

fn merge_physics_delta(target: &mut PhysicsDelta, source: &PhysicsDelta) {
    if let Some(gravity) = source.gravity {
        target.gravity = Some(gravity);
    }
}

fn parse_scenario_command(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<ScenarioBatch, EngineError> {
    match value {
        Value::Nil => Ok(ScenarioBatch::default()),
        Value::Proplist(map) => {
            let mut batch = ScenarioBatch::default();
            for (key, value) in map.into_iter() {
                match key.as_str() {
                    "spawn" => {
                        batch
                            .spawns
                            .extend(value_to_spawns(definition, function, value)?);
                    }
                    "global_effects" => {
                        batch
                            .global_effects
                            .extend(value_to_effect_commands(definition, function, value)?);
                    }
                    "physics" => {
                        let delta = value_to_physics_delta(definition, function, value)?;
                        if !delta.is_empty() {
                            if let Some(existing) = &mut batch.physics {
                                merge_physics_delta(existing, &delta);
                            } else {
                                batch.physics = Some(delta);
                            }
                        }
                    }
                    "landscape" => {
                        batch
                            .landscape
                            .extend(value_to_landscape_commands(definition, function, value)?);
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
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected proplist or nil, got {}", other.type_name()),
        }),
    }
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
            EffectCommand::Remove { name, .. } => {
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
            "direction" => {
                batch.delta.direction = Some(value_to_direction(definition, function, value)?);
            }
            "command_direction" => {
                batch.delta.command_direction =
                    Some(value_to_command_direction(definition, function, value)?);
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
            "physics" => {
                let delta = value_to_physics_delta(definition, function, value)?;
                if !delta.is_empty() {
                    if let Some(existing) = &mut batch.physics {
                        merge_physics_delta(existing, &delta);
                    } else {
                        batch.physics = Some(delta);
                    }
                }
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
            "data" => {
                let data = value_to_int(definition, function, value)?;
                update.set_data(data);
            }
            "target" => {
                let target = value_to_object_reference(definition, function, "target", value)?;
                update.set_target(target);
            }
            "target2" => {
                let target = value_to_object_reference(definition, function, "target2", value)?;
                update.set_target2(target);
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
            detail: format!("expected array of two ints, got {}", other.type_name()),
        }),
    }
}

fn value_to_physics_delta(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<PhysicsDelta, EngineError> {
    match value {
        Value::Nil => Ok(PhysicsDelta::default()),
        Value::Proplist(map) => {
            let mut delta = PhysicsDelta::default();
            for (key, entry) in map.into_iter() {
                match key.as_str() {
                    "gravity" => match entry {
                        Value::Int(val) => delta.gravity = Some(val),
                        Value::Nil => delta.gravity = Some(0),
                        other => {
                            return Err(EngineError::InvalidScriptOutput {
                                definition: definition.to_string(),
                                function,
                                detail: format!(
                                    "physics.gravity expects int or nil, got {}",
                                    other.type_name()
                                ),
                            })
                        }
                    },
                    other => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: format!("unexpected physics key `{other}`"),
                        })
                    }
                }
            }
            Ok(delta)
        }
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!(
                "expected proplist or nil for physics, got {}",
                other.type_name()
            ),
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

fn value_to_direction(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Direction, EngineError> {
    let raw = value_to_int(definition, function, value)?;
    Direction::from_script_value(raw).ok_or_else(|| EngineError::InvalidScriptOutput {
        definition: definition.to_string(),
        function,
        detail: format!("unsupported direction value {raw}"),
    })
}

fn value_to_command_direction(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<CommandDirection, EngineError> {
    let raw = value_to_int(definition, function, value)?;
    CommandDirection::from_script_value(raw).ok_or_else(|| EngineError::InvalidScriptOutput {
        definition: definition.to_string(),
        function,
        detail: format!("unsupported command_direction value {raw}"),
    })
}

fn value_to_object_reference(
    definition: &str,
    function: &'static str,
    field: &str,
    value: Value,
) -> Result<Option<ObjectId>, EngineError> {
    match value {
        Value::Nil => Ok(None),
        Value::Int(id) => {
            if id < 0 {
                Ok(None)
            } else {
                Ok(Some(ObjectId::new(id as u64)))
            }
        }
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) if *id >= 0 => Ok(Some(ObjectId::new(*id as u64))),
            Some(other) => Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!(
                    "expected int for action.{} proplist id, got {}",
                    field,
                    other.type_name()
                ),
            }),
            None => Ok(None),
        },
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!(
                "expected int, proplist, or nil for action.{field}, got {}",
                other.type_name()
            ),
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

        let direction = match map.get("direction") {
            Some(value) => value_to_direction(definition, function, value.clone())?,
            None => Direction::default(),
        };

        let command_direction = match map.get("command_direction") {
            Some(value) => value_to_command_direction(definition, function, value.clone())?,
            None => CommandDirection::default(),
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
            .with_direction(direction)
            .with_command_direction(command_direction)
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
        let mut landscape_ops = Vec::new();

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
                "direction" => {
                    update.direction = Some(value_to_direction(definition, function, value)?);
                }
                "command_direction" => {
                    update.command_direction =
                        Some(value_to_command_direction(definition, function, value)?);
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
                "landscape" => {
                    landscape_ops.extend(value_to_landscape_commands(definition, function, value)?);
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
                .with_destroy(destroy)
                .with_landscape(landscape_ops),
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

fn value_to_landscape_commands(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vec<LandscapeCommand>, EngineError> {
    let entries = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!(
                    "expected array for landscape commands, got {}",
                    other.type_name()
                ),
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
                    detail: format!(
                        "landscape entry must be proplist, got {}",
                        other.type_name()
                    ),
                })
            }
        };

        let op = match map.remove("op") {
            Some(Value::String(op)) => op,
            Some(other) => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("landscape.op must be string, got {}", other.type_name()),
                })
            }
            None => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: "landscape entry missing `op`".into(),
                })
            }
        };

        match op.as_str() {
            "lower" => {
                let start = match map.remove("start") {
                    Some(value) => value_to_int(definition, function, value)?,
                    None => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "landscape lower entry missing `start`".into(),
                        })
                    }
                };

                let height = match map.remove("height") {
                    Some(value) => value_to_int(definition, function, value)?,
                    None => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "landscape lower entry missing `height`".into(),
                        })
                    }
                };

                let end = if let Some(value) = map.remove("end") {
                    value_to_int(definition, function, value)?
                } else if let Some(value) = map.remove("width") {
                    let width = value_to_int(definition, function, value)?;
                    if width <= 0 {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "landscape lower width must be > 0".into(),
                        });
                    }
                    start + width
                } else {
                    start + 1
                };

                if end <= start {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: "landscape lower end must be greater than start".into(),
                    });
                }

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("unexpected key `{}` in landscape lower entry", key),
                    });
                }

                commands.push(LandscapeCommand::LowerRange { start, end, height });
            }
            "set_liquid" => {
                let column_value = match map.remove("column").or_else(|| map.remove("x")) {
                    Some(value) => value,
                    None => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "landscape set_liquid entry missing `column`".into(),
                        })
                    }
                };

                let column = value_to_int(definition, function, column_value)?;

                let segments_value = map.remove("segments").unwrap_or(Value::Nil);
                let segments = value_to_liquid_segments(definition, function, segments_value)?;

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("unexpected key `{}` in landscape set_liquid entry", key),
                    });
                }

                commands.push(LandscapeCommand::SetLiquidColumn { column, segments });
            }
            "clear_liquid" => {
                let column_value = match map.remove("column").or_else(|| map.remove("x")) {
                    Some(value) => value,
                    None => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function,
                            detail: "landscape clear_liquid entry missing `column`".into(),
                        })
                    }
                };

                let column = value_to_int(definition, function, column_value)?;

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("unexpected key `{}` in landscape clear_liquid entry", key),
                    });
                }

                commands.push(LandscapeCommand::ClearLiquidColumn { column });
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("unsupported landscape op `{other}`"),
                });
            }
        }
    }

    Ok(commands)
}

fn value_to_liquid_segments(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vec<LiquidSegment>, EngineError> {
    let entries = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!(
                    "landscape segments must be array or nil, got {}",
                    other.type_name()
                ),
            })
        }
    };

    let mut segments = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut segment_map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!(
                        "landscape segment must be proplist, got {}",
                        other.type_name()
                    ),
                })
            }
        };

        let top_value =
            segment_map
                .remove("top")
                .ok_or_else(|| EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: "landscape segment missing `top`".into(),
                })?;

        let bottom_value =
            segment_map
                .remove("bottom")
                .ok_or_else(|| EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: "landscape segment missing `bottom`".into(),
                })?;

        let top = value_to_int(definition, function, top_value)?;
        let bottom = value_to_int(definition, function, bottom_value)?;

        if let Some((key, _)) = segment_map.into_iter().next() {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!("unexpected key `{}` in landscape segment entry", key),
            });
        }

        segments.push(LiquidSegment::new(top, bottom));
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_resources::MaterialLibrary;
    use lc_script::Value;
    use rand::Rng;
    use rand_chacha::ChaCha8Rng;
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

    #[test]
    fn blast_circle_emits_particles_for_blastable_materials() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1
            Blast2PXSRatio=2
            SplashRate=15
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(earth)));

        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, None)
            .expect("blast applies");
        let removed = result
            .removed_by_material
            .get(&earth)
            .copied()
            .unwrap_or_default();
        assert!(removed > 0, "expected blast to remove material");

        let snapshot = engine.snapshot();
        assert!(
            !snapshot.particles.is_empty(),
            "expected blast to emit particles"
        );
        assert_eq!(snapshot.particles[0].definition_id, "material/pxs/earth");
        assert_eq!(snapshot.particles[0].parameter_b, earth.index() as i32);
        Ok(())
    }

    #[test]
    fn blast_circle_spawns_objects_for_material_reactions() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Rock]
            Name=Rock
            Density=110
            Friction=35
            BlastFree=1
            Blast2Object=GEM0
            Blast2ObjectRatio=2
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let rock = materials.id_of("Rock").expect("rock exists");
        let mut engine = Engine::with_seed(11);
        engine.set_materials(materials);
        engine
            .register_definition(simple_definition("GEM0"))
            .expect("gem definition registers");
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(rock)));

        let before = engine.snapshot().objects.len();
        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, Some(1))
            .expect("blast applies");
        let removed = result
            .removed_by_material
            .get(&rock)
            .copied()
            .unwrap_or_default();
        assert!(removed > 0, "expected blast to remove rock material");

        let after = engine.snapshot().objects.len();
        assert!(
            after > before,
            "expected blast to spawn objects for blast reaction"
        );
        Ok(())
    }

    #[test]
    fn blast_circle_shifts_materials_with_blast_shift_to() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Granite]
            Name=Granite
            Density=110
            Friction=35
            BlastShiftTo=Earth

            [Material Earth]
            Name=Earth
            Density=90
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let granite = materials.id_of("Granite").expect("granite exists");
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(29);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(25, 40, Some(granite)));

        engine
            .blast_circle(Vector2::new(12, 40), 10, None)
            .expect("blast applies");

        let landscape = engine.landscape().expect("landscape present");
        let mut shifted_columns = 0;
        for x in 0..landscape.width() as i32 {
            if landscape.solid_material_at(x) == Some(earth) {
                shifted_columns += 1;
            }
        }
        assert!(
            shifted_columns > 0,
            "expected blast to shift some columns to target material"
        );
        Ok(())
    }

    #[test]
    fn material_particles_settle_into_landscape() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1
            Blast2PXSRatio=1
            SplashRate=15
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(19);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(12, 30, Some(earth)));

        engine
            .blast_circle(Vector2::new(6, 30), 3, None)
            .expect("blast applies");

        let post_blast_surface = {
            let snapshot = engine.snapshot();
            let landscape = snapshot.landscape.as_ref().expect("landscape present");
            landscape.surface()[6]
        };
        assert!(
            post_blast_surface > 30,
            "blast should lower the surface before particles settle"
        );

        for _ in 0..24 {
            engine.tick().expect("tick succeeds");
        }

        let snapshot = engine.snapshot();
        let landscape = snapshot.landscape.expect("landscape present");
        assert!(
            landscape.surface()[6] <= 30,
            "expected particles to backfill the landscape surface"
        );
        assert!(
            landscape.surface()[6] < post_blast_surface,
            "expected surface height to recover after particle settling"
        );
        Ok(())
    }

    #[test]
    fn material_particles_apply_friction_to_objects() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=45
            BlastFree=1
            Blast2PXSRatio=1
            SplashRate=10
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 24, Some(earth)));
        engine
            .register_definition(simple_definition("Crate"))
            .expect("definition registers");

        let crate_id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_position(Vector2::new(8, 24))
                    .with_velocity(Vector2::new(8, 0)),
            )
            .expect("spawn succeeds");

        engine
            .blast_circle(Vector2::new(8, 24), 2, None)
            .expect("blast applies");

        for _ in 0..20 {
            engine.tick().expect("tick succeeds");
        }

        let object = engine
            .object_snapshot(crate_id)
            .expect("crate still present");
        assert!(
            object.velocity.x.abs() < 8,
            "expected particle collision to reduce horizontal velocity"
        );

        let snapshot = engine.snapshot();
        assert!(snapshot.particles.is_empty(), "particles dissipated");
        Ok(())
    }

    const PASSIVE_PLAYER_SCRIPT: &str = r#"
global func Initialize(state, random)
{
    return nil;
}

global func Step(state, frame, random)
{
    return nil;
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

    const MENU_COMMAND_SCRIPT: &str = r#"
global func Initialize(state, random)
{
    return nil;
}

global func MenuCommand(state, kind, selection)
{
    if (kind == "focus")
    {
        SetOwner(42);
        return true;
    }
    return false;
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

    #[test]
    fn wind_variation_adjusts_over_time() {
        let mut settings = EnvironmentSettings::new(5).with_wind_variation(4, 40);
        let base = settings.base_wind;
        let mut rng = ChaCha8Rng::seed_from_u64(1234);
        let mut probe = rng.clone();
        let range = settings.wind_variation.abs();
        let lower = base.saturating_sub(range);
        let upper = base.saturating_add(range);
        let offset = probe.gen_range(-range..=range);
        let expected_target = (base.saturating_add(offset)).clamp(lower, upper);

        settings.advance_frame(&mut rng);

        assert_eq!(
            settings.wind_target, expected_target,
            "wind target should move within configured variation"
        );
        assert_eq!(
            settings.wind_update_timer,
            settings.wind_update_interval.saturating_sub(1),
            "timer should be primed for next update cycle"
        );

        if expected_target > base {
            assert_eq!(
                settings.wind,
                base.saturating_add(1),
                "wind should move toward higher target by one unit"
            );
        } else if expected_target < base {
            assert_eq!(
                settings.wind,
                base.saturating_sub(1),
                "wind should move toward lower target by one unit"
            );
        } else {
            assert_eq!(
                settings.wind, base,
                "wind should remain unchanged when target equals base"
            );
        }
    }

    const SET_BRIDGE_ACTION_DATA_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        if (SetBridgeActionData(200, true, false, 7))
        {
            return { energy = 1 };
        }
        return { energy = 0 };
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    const SET_BRIDGE_ACTION_DATA_FAILURE_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        if (SetBridgeActionData(120, false, false, -1))
        {
            return { energy = 1 };
        }
        return { energy = 0 };
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

    const PATHFINDING_HELPER_SCRIPT: &str = r#"
    global func Initialize(state, random)
    {
        var success = PathFree(0, 0, 10, 0);
        var failure = PathFree(0, 0, 10, 12);
        var value = 0;
        if (success)
        {
            value = value + 1;
        }
        if (failure)
        {
            value = value + 2;
        }
        return { energy = value };
    }

    global func Step(state, frame, random)
    {
        return nil;
    }
    "#;

    fn build_lift_definition(id: &str) -> Definition {
        let mut definition =
            Definition::from_script(id, id, PROCEDURE_MOVEMENT_SCRIPT).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Lift".to_string(),
            ActionSpec::default().with_procedure("lift"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition
    }

    fn build_idle_definition(id: &str) -> Definition {
        Definition::from_script(id, id, PROCEDURE_MOVEMENT_SCRIPT).expect("script compiles")
    }

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
    fn home_base_production_shared_across_team_when_rule_enabled() {
        let mut engine = Engine::new();
        engine.set_team_home_base_rule(true);

        let mut production = HashMap::new();
        production.insert("Brick".to_string(), 10);

        let leader = PlayerConfig::new(1, "Leader")
            .with_team(Some(1))
            .with_home_base_production(production.clone());
        let follower = PlayerConfig::new(2, "Follower")
            .with_team(Some(1))
            .with_home_base_production(production.clone());

        engine.register_player(leader).expect("leader registered");
        engine
            .register_player(follower)
            .expect("follower registered");

        for _ in 0..60 {
            engine.tick_player_systems();
        }

        let leader = engine.player(1).expect("leader present");
        let follower = engine.player(2).expect("follower present");
        assert_eq!(leader.home_base_material().get("Brick"), Some(&1));
        assert_eq!(follower.home_base_material().get("Brick"), Some(&1));
    }

    #[test]
    fn home_base_production_respects_rule_toggle() {
        let mut engine = Engine::new();
        engine.set_team_home_base_rule(false);

        let mut production = HashMap::new();
        production.insert("Brick".to_string(), 10);

        let leader = PlayerConfig::new(1, "Leader")
            .with_team(Some(2))
            .with_home_base_production(production.clone());
        let follower = PlayerConfig::new(2, "Follower").with_team(Some(2));

        engine.register_player(leader).expect("leader registered");
        engine
            .register_player(follower)
            .expect("follower registered");

        for _ in 0..60 {
            engine.tick_player_systems();
        }

        {
            let leader = engine.player(1).expect("leader present");
            let follower = engine.player(2).expect("follower present");
            assert_eq!(leader.home_base_material().get("Brick"), Some(&1));
            assert!(
                follower.home_base_material().get("Brick").is_none(),
                "follower should not receive materials when rule disabled"
            );
        }

        engine.set_team_home_base_rule(true);
        let leader_material = engine
            .player(1)
            .expect("leader present")
            .home_base_material()
            .clone();
        engine
            .set_player_home_base_material(1, leader_material)
            .expect("update succeeds");

        let follower_after = engine.player(2).expect("follower present");
        assert_eq!(follower_after.home_base_material().get("Brick"), Some(&1));
    }

    #[test]
    fn enabling_team_rule_synchronizes_existing_members() {
        let mut engine = Engine::new();

        let mut material = HashMap::new();
        material.insert("Brick".to_string(), 5);

        let leader = PlayerConfig::new(1, "Leader")
            .with_team(Some(3))
            .with_home_base_material(material.clone());
        let follower = PlayerConfig::new(2, "Follower").with_team(Some(3));

        engine.register_player(leader).expect("leader registered");
        engine
            .register_player(follower)
            .expect("follower registered");

        let follower_before = engine.player(2).expect("follower present");
        assert!(
            follower_before.home_base_material().is_empty(),
            "rule disabled keeps member inventory separate"
        );

        engine.set_team_home_base_rule(true);

        let follower_after = engine.player(2).expect("follower present");
        assert_eq!(follower_after.home_base_material().get("Brick"), Some(&5));
    }

    #[test]
    fn path_free_host_function_queries_landscape() {
        let mut definition =
            Definition::from_script("PathTester", "PathTester", PATHFINDING_HELPER_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(32, 8));

        let id = engine
            .spawn_object(SpawnConfig::new("PathTester"))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.energy, 1);
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
    fn menu_command_invokes_definition_script() {
        let mut definition =
            Definition::from_script("Crew", "Crew", MENU_COMMAND_SCRIPT).expect("script compiles");
        definition.set_crew_member(true);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Crew").with_owner(1))
            .expect("spawn succeeds");

        let selection = MenuCommandSelection {
            primary_id: id,
            instances: vec![id],
            definition_id: "Crew".to_string(),
            label: "Crew".to_string(),
        };

        let handled = engine
            .menu_command(id, MenuCommandKind::Focus, selection)
            .expect("menu command succeeds");
        assert!(handled, "script should report handled command");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(
            snapshot.owner, 42,
            "script should update object owner via SetOwner"
        );
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
    fn action_start_and_end_callbacks_fire() {
        use std::sync::{Arc, Mutex};

        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        global func OnIdleStart(state, action) { return nil; }
        global func OnIdleEnd(state, action) { return nil; }
        global func OnWalkStart(state, action) { return nil; }
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

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default()
                .with_length(1)
                .with_next("Walk")
                .with_start_call("OnIdleStart")
                .with_end_call("OnIdleEnd"),
        );
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_start_call("OnWalkStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_end, 0);
            assert_eq!(walk_start, 0);
        }

        engine.tick().expect("first tick succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_end, 1);
            assert_eq!(walk_start, 1);
        }

        engine.tick().expect("second tick succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_end, 1);
            assert_eq!(walk_start, 1);
        }
    }

    #[test]
    fn forced_action_change_triggers_abort_callbacks() {
        use std::sync::{Arc, Mutex};

        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        global func OnIdleStart(state, action) { return nil; }
        global func OnIdleEnd(state, action) { return nil; }
        global func OnIdleAbort(state, action) { return nil; }
        global func OnRunStart(state, action) { return nil; }
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

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default()
                .with_length(20)
                .with_start_call("OnIdleStart")
                .with_end_call("OnIdleEnd")
                .with_abort_call("OnIdleAbort"),
        );
        actions.insert(
            "Run".to_string(),
            ActionSpec::default().with_start_call("OnRunStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_abort = calls.iter().filter(|name| *name == "OnIdleAbort").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let run_start = calls.iter().filter(|name| *name == "OnRunStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_abort, 0);
            assert_eq!(idle_end, 0);
            assert_eq!(run_start, 0);
        }

        engine
            .apply_object_update(id, ObjectUpdate::new().with_action("Run"))
            .expect("update succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_start = calls.iter().filter(|name| *name == "OnIdleStart").count();
            let idle_abort = calls.iter().filter(|name| *name == "OnIdleAbort").count();
            let idle_end = calls.iter().filter(|name| *name == "OnIdleEnd").count();
            let run_start = calls.iter().filter(|name| *name == "OnRunStart").count();
            assert_eq!(idle_start, 1);
            assert_eq!(idle_abort, 1);
            assert_eq!(idle_end, 0);
            assert_eq!(run_start, 1);
        }
    }

    #[test]
    fn non_forced_action_update_respects_no_other_action() {
        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        "#;

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_no_other_action(true),
        );
        actions.insert("Run".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_action_update(ActionUpdate::default().with_name("Run").with_force(false)),
            )
            .expect("update succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.action.name, "Idle");
    }

    #[test]
    fn forced_action_update_overrides_no_other_action() {
        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        "#;

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_no_other_action(true),
        );
        actions.insert("Run".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine
            .apply_object_update(id, ObjectUpdate::new().with_action("Run"))
            .expect("update succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.action.name, "Run");
    }

    #[test]
    fn action_phase_callbacks_fire() {
        use std::sync::{Arc, Mutex};

        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        global func OnIdlePhase(state, action) { return nil; }
        global func OnWalkStart(state, action) { return nil; }
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

        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default()
                .with_length(3)
                .with_next("Walk")
                .with_phase_call("OnIdlePhase"),
        );
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_start_call("OnWalkStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(2);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            assert_eq!(idle_phase, 0);
        }

        engine.tick().expect("first tick succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            assert_eq!(idle_phase, 1);
        }

        engine.tick().expect("second tick succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            assert_eq!(idle_phase, 2);
        }

        engine.tick().expect("third tick succeeds");

        {
            let calls = call_log.lock().unwrap().clone();
            let idle_phase = calls.iter().filter(|name| *name == "OnIdlePhase").count();
            let walk_start = calls.iter().filter(|name| *name == "OnWalkStart").count();
            assert_eq!(idle_phase, 3);
            assert_eq!(walk_start, 1);
        }

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);
    }

    #[test]
    fn action_step_advances_multiple_phases() {
        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        "#;

        let mut definition =
            Definition::from_script("Stepper", "Stepper", script).expect("script compiles");

        let mut actions = HashMap::new();
        actions.insert(
            "Pulse".to_string(),
            ActionSpec::default()
                .with_length(5)
                .with_step(2)
                .with_next("Pulse"),
        );
        definition.configure_actions(Some("Pulse".to_string()), actions);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Stepper"))
            .expect("spawn succeeds");

        let after_first = engine.tick().expect("first tick succeeds");
        let object = after_first.object(id).expect("object present");
        assert_eq!(object.action.phase, 2);

        let after_second = engine.tick().expect("second tick succeeds");
        let object = after_second.object(id).expect("object present");
        assert_eq!(object.action.phase, 4);

        let after_third = engine.tick().expect("third tick succeeds");
        let object = after_third.object(id).expect("object present");
        assert_eq!(object.action.phase, 0);
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
    fn flight_command_direction_updates_velocity() {
        let mut definition = Definition::from_script("Glider", "Glider", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Fly".to_string(),
            ActionSpec::default().with_procedure("flight"),
        );
        definition.configure_actions(Some("Fly".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_float_speed(6)
                .with_float_acceleration(3),
        );

        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Glider").with_command_direction(CommandDirection::DownRight),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(3, 3));
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
    fn float_command_direction_updates_velocity() {
        let mut definition =
            Definition::from_script("Balloon", "Balloon", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Float".to_string(),
            ActionSpec::default().with_procedure("float"),
        );
        definition.configure_actions(Some("Float".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_float_speed(6)
                .with_float_acceleration(2),
        );

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Balloon").with_command_direction(CommandDirection::UpRight),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(2, -1));
    }

    #[test]
    fn swim_procedure_reduces_gravity_and_blocks_wind() {
        let mut definition =
            Definition::from_script("Swimmer", "Swimmer", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Swim".to_string(),
            ActionSpec::default().with_procedure("swim"),
        );
        definition.configure_actions(Some("Swim".to_string()), actions);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(6, 20, -30)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(5));

        let id = engine
            .spawn_object(SpawnConfig::new("Swimmer"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 3);
        assert_eq!(object.velocity.x, 0);
    }

    #[test]
    fn swim_command_direction_updates_velocity_and_stop_decelerates() {
        let mut definition =
            Definition::from_script("Swimmer", "Swimmer", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Swim".to_string(),
            ActionSpec::default().with_procedure("swim"),
        );
        definition.configure_actions(Some("Swim".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_swim_speed(10)
                .with_swim_acceleration(2),
        );

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(0, 20, -20)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Swimmer").with_command_direction(CommandDirection::DownRight),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(2, 2));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(4, 4));

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new().with_command_direction(CommandDirection::Stop),
            )
            .expect("update succeeds");

        let snapshot = engine.tick().expect("third tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(2, 2));

        let snapshot = engine.tick().expect("fourth tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(0, 0));
    }

    #[test]
    fn lift_procedure_adjusts_target_velocity() {
        let lifter_definition = build_lift_definition("Lifter");
        let crate_definition = build_idle_definition("Crate");

        let mut engine = Engine::with_seed(31);
        engine
            .register_definition(lifter_definition)
            .expect("lifter registers");
        engine
            .register_definition(crate_definition)
            .expect("crate registers");

        let target_id = engine
            .spawn_object(SpawnConfig::new("Crate"))
            .expect("target spawns");

        let mut lift_action = ActionState::new("Lift");
        lift_action.target = Some(target_id);

        let lifter_id = engine
            .spawn_object(
                SpawnConfig::new("Lifter")
                    .with_action(lift_action)
                    .with_command_direction(CommandDirection::Up),
            )
            .expect("lifter spawns");

        let snapshot = engine.tick().expect("tick succeeds");
        let target = snapshot.object(target_id).expect("target present");
        assert!(target.velocity.y < 0, "lift should pull target upward");
        let lifter = snapshot.object(lifter_id).expect("lifter present");
        assert_eq!(lifter.action.name, "Lift");
    }

    #[test]
    fn lift_procedure_resets_without_target() {
        let lifter_definition = build_lift_definition("Lifter");

        let mut engine = Engine::with_seed(37);
        engine
            .register_definition(lifter_definition)
            .expect("definition registers");

        let lifter_id = engine
            .spawn_object(
                SpawnConfig::new("Lifter")
                    .with_action(ActionState::new("Lift"))
                    .with_command_direction(CommandDirection::Up),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let lifter = snapshot.object(lifter_id).expect("lifter present");
        assert_eq!(lifter.action.name, "Idle");
        assert!(lifter.action.target.is_none());
    }

    #[test]
    fn hang_procedure_locks_vertical_velocity() {
        let mut definition =
            Definition::from_script("Clinger", "Clinger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Hang".to_string(),
            ActionSpec::default().with_procedure("hang"),
        );
        definition.configure_actions(Some("Hang".to_string()), actions);

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(6, 20, -30)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(4));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Clinger")
                    .with_velocity(Vector2::new(1, 5))
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 0);
    }

    #[test]
    fn set_bridge_action_data_updates_action_data() {
        let mut definition =
            Definition::from_script("Bridger", "Bridger", SET_BRIDGE_ACTION_DATA_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Bridge".to_string(),
            ActionSpec::default().with_procedure("bridge"),
        );
        definition.configure_actions(Some("Bridge".to_string()), actions);

        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Bridger").with_action(ActionState::new("Bridge")))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.energy, 1);
        let expected = encode_bridge_action_data(200, true, false, 7);
        assert_eq!(snapshot.action.data, expected);
    }

    #[test]
    fn set_bridge_action_data_returns_false_when_not_in_bridge_procedure() {
        let mut definition = Definition::from_script(
            "IdleActor",
            "IdleActor",
            SET_BRIDGE_ACTION_DATA_FAILURE_SCRIPT,
        )
        .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(41);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("IdleActor"))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.energy, 0);
        assert_eq!(snapshot.action.data, 0);
    }

    #[test]
    fn bridge_procedure_freezes_velocity_and_ignores_wind() {
        let mut definition =
            Definition::from_script("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Bridge".to_string(),
            ActionSpec::default().with_procedure("bridge"),
        );
        definition.configure_actions(Some("Bridge".to_string()), actions);

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(6));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Bridger")
                    .with_velocity(Vector2::new(8, -3))
                    .with_action(ActionState::new("Bridge")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
    }

    #[test]
    fn bridge_procedure_updates_landscape_height() {
        let mut definition =
            Definition::from_script("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Bridge".to_string(),
            ActionSpec::default().with_procedure("bridge"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let mut heights = vec![5; 16];
        for column in 6..16 {
            heights[column] = 0;
        }
        engine.set_landscape(Landscape::new(16, heights).expect("landscape constructs"));

        let mut action = ActionState::new("Bridge");
        action.data = encode_bridge_action_data(10, false, false, -1);

        let id = engine
            .spawn_object(
                SpawnConfig::new("Bridger")
                    .with_position(Vector2::new(5, 5))
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::Right)
                    .with_action(action),
            )
            .expect("spawn succeeds");

        let mut snapshot = engine.tick().expect("tick succeeds");
        for _ in 1..10 {
            snapshot = engine.tick().expect("tick succeeds");
        }

        let landscape = snapshot.landscape.as_ref().expect("landscape present");
        assert_eq!(landscape.surface()[5], 5);
        assert_eq!(landscape.surface()[6], 5);

        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Idle");
    }

    #[test]
    fn connect_procedure_freezes_velocity_and_ignores_wind() {
        let mut definition =
            Definition::from_script("Connector", "Connector", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Connect".to_string(),
            ActionSpec::default().with_procedure("connect"),
        );
        definition.configure_actions(Some("Connect".to_string()), actions);

        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(10));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Connector")
                    .with_velocity(Vector2::new(-7, 4))
                    .with_action(ActionState::new("Connect")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
    }

    #[test]
    fn kneel_procedure_locks_vertical_velocity_and_blocks_wind() {
        let mut definition =
            Definition::from_script("Kneeler", "Kneeler", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Kneel".to_string(),
            ActionSpec::default().with_procedure("kneel"),
        );
        definition.configure_actions(Some("Kneel".to_string()), actions);

        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(8));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Kneeler")
                    .with_velocity(Vector2::new(5, -4))
                    .with_action(ActionState::new("Kneel")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 5);
    }

    #[test]
    fn dig_procedure_zeroes_velocity_when_stopped() {
        let mut definition = Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig"),
        );
        definition.configure_actions(Some("Dig".to_string()), actions);

        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_physics(PhysicsSettings::default());
        engine.set_environment(EnvironmentSettings::new(7));

        let initial_velocity = Vector2::new(4, -3);

        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_velocity(initial_velocity)
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
    }

    #[test]
    fn dig_procedure_carves_diggable_material() {
        let mut definition = Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig").with_dig_free(6),
        );
        definition.configure_actions(Some("Dig".to_string()), actions);

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
        "#;
        let library =
            lc_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let mut materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_position(Vector2::new(12, 8))
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let mut snapshot = engine.tick().expect("tick succeeds");
        for _ in 0..5 {
            snapshot = engine.tick().expect("tick succeeds");
        }

        let landscape = snapshot.landscape.as_ref().expect("landscape present");
        let center_height = landscape.surface()[12];
        let edge_height = landscape.surface()[2];
        assert!(center_height > 6);
        assert_eq!(edge_height, 6);

        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Dig");
    }

    #[test]
    fn dig_procedure_spawns_dig2object_when_ratio_reached() {
        let mut digger = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig").with_dig_free(6),
        );
        digger.configure_actions(Some("Dig".to_string()), actions);

        let gem = Definition::from_script("GEM_", "Gem", "func Initialize() { }\n")
            .expect("script compiles");

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=3
        "#;
        let library =
            lc_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let mut materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(digger)
            .expect("digger registers");
        engine.register_definition(gem).expect("gem registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

        engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(Vector2::new(12, 8))
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let mut spawned = false;
        for _ in 0..20 {
            let snapshot = engine.tick().expect("tick succeeds");
            if snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "GEM_")
            {
                spawned = true;
                break;
            }
        }

        assert!(
            spawned,
            "expected Dig2Object conversion to spawn target definition"
        );
    }

    #[test]
    fn scale_procedure_zeroes_horizontal_velocity() {
        let mut definition = Definition::from_script("Scaler", "Scaler", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Scale".to_string(),
            ActionSpec::default().with_procedure("scale"),
        );
        definition.configure_actions(Some("Scale".to_string()), actions);

        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(3));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Scaler")
                    .with_velocity(Vector2::new(-7, 2))
                    .with_action(ActionState::new("Scale")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.x, 0);
        assert_eq!(object.velocity.y, 1);
    }

    #[test]
    fn scale_command_direction_moves_up_when_pressing_wall_direction() {
        let mut definition = Definition::from_script("Scaler", "Scaler", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Scale".to_string(),
            ActionSpec::default().with_procedure("scale"),
        );
        definition.configure_actions(Some("Scale".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_scale_speed(6)
                .with_scale_acceleration(3),
        );

        let mut engine = Engine::with_seed(41);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Scaler")
                    .with_direction(Direction::Left)
                    .with_command_direction(CommandDirection::Left)
                    .with_action(ActionState::new("Scale")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(0, -3));
        assert_eq!(object.direction, Direction::Left);
    }

    #[test]
    fn hangle_command_direction_updates_velocity_and_direction() {
        let mut definition =
            Definition::from_script("Hangler", "Hangler", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Hangle".to_string(),
            ActionSpec::default().with_procedure("hang"),
        );
        definition.configure_actions(Some("Hangle".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_hangle_speed(5)
                .with_hangle_acceleration(2),
        );

        let mut engine = Engine::with_seed(43);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Hangler")
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::Left)
                    .with_action(ActionState::new("Hangle")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(-2, 0));
        assert_eq!(object.direction, Direction::Left);
    }

    #[test]
    fn dig_command_direction_sets_directional_velocity() {
        let mut definition = Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig"),
        );
        definition.configure_actions(Some("Dig".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default().with_dig_speed(6));

        let mut engine = Engine::with_seed(47);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::DownLeft)
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(-6, 6));
        assert_eq!(object.direction, Direction::Left);

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new().with_command_direction(CommandDirection::Up),
            )
            .expect("update succeeds");

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(-6, -3));
        assert_eq!(object.direction, Direction::Left);
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
    fn push_procedure_without_target_resets_to_default() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Pusher", "Pusher", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        actions.insert(
            "Push".to_string(),
            ActionSpec::default().with_procedure("push"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(12);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let push_state = ActionState::new("Push");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_action(push_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Idle");
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.command_direction, CommandDirection::Stop);
    }

    #[test]
    fn push_procedure_moves_target_and_pusher() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut pusher_definition = Definition::from_script("Pusher", "Pusher", script).unwrap();
        let mut pusher_actions = HashMap::new();
        pusher_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        pusher_actions.insert(
            "Push".to_string(),
            ActionSpec::default().with_procedure("push"),
        );
        pusher_definition.configure_actions(Some("Idle".to_string()), pusher_actions);
        pusher_definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut target_definition = Definition::from_script("Crate", "Crate", script).unwrap();
        let mut target_actions = HashMap::new();
        target_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        target_definition.configure_actions(Some("Idle".to_string()), target_actions);

        let mut engine = Engine::with_seed(18);
        engine
            .register_definition(pusher_definition)
            .expect("pusher registers");
        engine
            .register_definition(target_definition)
            .expect("target registers");

        let target_id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(10, 0)))
            .expect("target spawns");
        let target_initial_position = engine
            .object_snapshot(target_id)
            .expect("snapshot available")
            .position;

        let mut push_state = ActionState::new("Push");
        push_state.target = Some(target_id);

        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_position(Vector2::new(0, 0))
                    .with_action(push_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("pusher spawns");

        let snapshot = engine.tick().expect("tick succeeds");
        let pusher = snapshot
            .object(pusher_id)
            .expect("pusher present after tick");
        assert_eq!(pusher.action.name, "Push");
        assert!(pusher.velocity.x > 0, "pusher should move forward");
        assert_eq!(pusher.direction, Direction::Right);

        let target = snapshot
            .object(target_id)
            .expect("target present after tick");
        assert!(target.velocity.x >= 0);

        let snapshot = engine.tick().expect("second tick succeeds");
        let target_after = snapshot
            .object(target_id)
            .expect("target present after second tick");
        assert!(
            target_after.position.x > target_initial_position.x,
            "target should advance horizontally"
        );
    }

    #[test]
    fn pull_procedure_without_target_resets_to_default() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Puller", "Puller", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        actions.insert(
            "Pull".to_string(),
            ActionSpec::default().with_procedure("pull"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let pull_state = ActionState::new("Pull");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Puller")
                    .with_action(pull_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Idle");
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.command_direction, CommandDirection::Stop);
    }

    #[test]
    fn pull_procedure_moves_target_and_puller() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut puller_definition = Definition::from_script("Puller", "Puller", script).unwrap();
        let mut puller_actions = HashMap::new();
        puller_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        puller_actions.insert(
            "Pull".to_string(),
            ActionSpec::default().with_procedure("pull"),
        );
        puller_definition.configure_actions(Some("Idle".to_string()), puller_actions);
        puller_definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut target_definition = Definition::from_script("Crate", "Crate", script).unwrap();
        let mut target_actions = HashMap::new();
        target_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        target_definition.configure_actions(Some("Idle".to_string()), target_actions);

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(puller_definition)
            .expect("puller registers");
        engine
            .register_definition(target_definition)
            .expect("target registers");

        let vertices = vec![
            ObjectVertex::new(-10, -10),
            ObjectVertex::new(10, -10),
            ObjectVertex::new(10, 10),
            ObjectVertex::new(-10, 10),
        ];

        let target_id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_position(Vector2::new(0, 0))
                    .with_vertices(vertices.clone()),
            )
            .expect("target spawns");
        let target_initial_position = engine
            .object_snapshot(target_id)
            .expect("target snapshot available")
            .position;

        let mut pull_state = ActionState::new("Pull");
        pull_state.target = Some(target_id);

        let puller_id = engine
            .spawn_object(
                SpawnConfig::new("Puller")
                    .with_position(Vector2::new(20, 0))
                    .with_vertices(vertices)
                    .with_action(pull_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("puller spawns");

        let snapshot = engine.tick().expect("tick succeeds");
        let puller = snapshot
            .object(puller_id)
            .expect("puller present after tick");
        assert_eq!(puller.action.name, "Pull");
        assert!(puller.velocity.x > 0, "puller should move forward");
        assert_eq!(puller.direction, Direction::Right);

        let target = snapshot
            .object(target_id)
            .expect("target present after tick");
        assert!(target.velocity.x >= 0);

        let snapshot = engine.tick().expect("second tick succeeds");
        let target_after = snapshot
            .object(target_id)
            .expect("target present after second tick");
        assert!(
            target_after.position.x > target_initial_position.x,
            "target should advance horizontally",
        );
    }

    #[test]
    fn fight_procedure_without_target_resets_to_default() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Fighter", "Fighter", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        actions.insert(
            "Fight".to_string(),
            ActionSpec::default().with_procedure("fight"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut engine = Engine::with_seed(27);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Fighter")
                    .with_position(Vector2::new(0, 0))
                    .with_action(ActionState::new("Fight")),
            )
            .expect("fighter spawns");

        let snapshot = engine.tick().expect("tick succeeds");
        let fighter = snapshot.object(id).expect("fighter present");
        assert_eq!(fighter.action.name, "Idle");
        assert_eq!(fighter.velocity, Vector2::ZERO);
    }

    #[test]
    fn fight_procedure_moves_toward_target() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut fighter_definition = Definition::from_script("Fighter", "Fighter", script).unwrap();
        let mut fighter_actions = HashMap::new();
        fighter_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        fighter_actions.insert(
            "Fight".to_string(),
            ActionSpec::default().with_procedure("fight"),
        );
        fighter_definition.configure_actions(Some("Idle".to_string()), fighter_actions);
        fighter_definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut opponent_definition =
            Definition::from_script("Opponent", "Opponent", script).unwrap();
        let mut opponent_actions = HashMap::new();
        opponent_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        opponent_actions.insert(
            "Fight".to_string(),
            ActionSpec::default().with_procedure("fight"),
        );
        opponent_definition.configure_actions(Some("Idle".to_string()), opponent_actions);

        let mut engine = Engine::with_seed(33);
        engine
            .register_definition(fighter_definition)
            .expect("fighter definition registers");
        engine
            .register_definition(opponent_definition)
            .expect("opponent definition registers");

        let vertices = vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ];

        let opponent_id = engine
            .spawn_object(
                SpawnConfig::new("Opponent")
                    .with_position(Vector2::new(12, 0))
                    .with_vertices(vertices.clone())
                    .with_action(ActionState::new("Fight")),
            )
            .expect("opponent spawns");

        let mut fight_state = ActionState::new("Fight");
        fight_state.target = Some(opponent_id);
        let fighter_id = engine
            .spawn_object(
                SpawnConfig::new("Fighter")
                    .with_position(Vector2::new(0, 0))
                    .with_vertices(vertices.clone())
                    .with_action(fight_state),
            )
            .expect("fighter spawns");

        engine
            .apply_object_update(
                opponent_id,
                ObjectUpdate::new()
                    .with_action_update(ActionUpdate::default().with_target(Some(fighter_id))),
            )
            .expect("opponent target update succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let fighter = snapshot
            .object(fighter_id)
            .expect("fighter present after tick");
        assert_eq!(fighter.action.name, "Fight");
        assert!(
            fighter.velocity.x > 0,
            "fighter should advance towards the opponent"
        );
        assert_eq!(fighter.direction, Direction::Right);
        assert_eq!(fighter.velocity.y, 0);
        assert!(
            fighter.position.x > 0,
            "fighter should have moved horizontally"
        );
    }

    #[test]
    fn fight_procedure_resets_when_target_not_fighting() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut fighter_definition = Definition::from_script("Fighter", "Fighter", script).unwrap();
        let mut fighter_actions = HashMap::new();
        fighter_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        fighter_actions.insert(
            "Fight".to_string(),
            ActionSpec::default().with_procedure("fight"),
        );
        fighter_definition.configure_actions(Some("Idle".to_string()), fighter_actions);
        fighter_definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut passive_definition = Definition::from_script("Passive", "Passive", script).unwrap();
        let mut passive_actions = HashMap::new();
        passive_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        passive_definition.configure_actions(Some("Idle".to_string()), passive_actions);

        let mut engine = Engine::with_seed(41);
        engine
            .register_definition(fighter_definition)
            .expect("fighter definition registers");
        engine
            .register_definition(passive_definition)
            .expect("passive definition registers");

        let vertices = vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ];

        let passive_id = engine
            .spawn_object(
                SpawnConfig::new("Passive")
                    .with_position(Vector2::new(10, 0))
                    .with_vertices(vertices.clone())
                    .with_action(ActionState::new("Idle")),
            )
            .expect("passive target spawns");

        let mut fight_state = ActionState::new("Fight");
        fight_state.target = Some(passive_id);
        let fighter_id = engine
            .spawn_object(
                SpawnConfig::new("Fighter")
                    .with_position(Vector2::new(0, 0))
                    .with_vertices(vertices)
                    .with_action(fight_state),
            )
            .expect("fighter spawns");

        let snapshot = engine.tick().expect("tick succeeds");
        let fighter = snapshot
            .object(fighter_id)
            .expect("fighter present after tick");
        assert_eq!(fighter.action.name, "Idle");
        assert_eq!(fighter.velocity, Vector2::ZERO);
    }

    #[test]
    fn chop_procedure_zeroes_existing_velocity() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Chopper", "Chopper", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Chop".to_string(),
            ActionSpec::default().with_procedure("chop"),
        );
        definition.configure_actions(Some("Chop".to_string()), actions);

        let mut engine = Engine::with_seed(20);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(3, 40, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Chopper")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(5, -3)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Chop");
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.position, Vector2::new(0, 0));
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
            .with_time_speed(30)
            .with_precipitation(-45)
            .with_sky_color(RgbColor::new(24, 48, 192));
        engine.set_environment(environment);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.environment.settings, environment);
        assert_eq!(snapshot.environment.wind_force, environment.wind_force(0));
        assert_eq!(
            snapshot.environment.ambient_temperature,
            environment.ambient_temperature(0)
        );
        assert_eq!(
            snapshot.environment.precipitation,
            environment.precipitation()
        );
        assert_eq!(snapshot.environment.sky_color, environment.sky_color());
    }

    #[test]
    fn environment_sky_color_can_be_configured_and_cleared() {
        let configured = EnvironmentSettings::new(0).with_sky_color(RgbColor::new(5, 10, 15));
        assert_eq!(configured.sky_color(), Some(RgbColor::new(5, 10, 15)));

        let cleared = configured.without_sky_color();
        assert!(cleared.sky_color().is_none());
    }

    #[test]
    fn resolved_sky_color_reflects_time_and_temperature() {
        let midnight = EnvironmentSettings::new(0).with_time_of_day(0);
        let midnight_color = midnight.resolved_sky_color(midnight.ambient_temperature(0));

        let midday = midnight.with_time_of_day(1200);
        let midday_color = midday.resolved_sky_color(midday.ambient_temperature(0));

        assert!(
            midday_color.r > midnight_color.r
                && midday_color.g > midnight_color.g
                && midday_color.b > midnight_color.b,
            "daylight should brighten sky color"
        );

        let cold = midday.with_temperature(-40);
        let warm = midday.with_temperature(40);
        let cold_color = cold.resolved_sky_color(cold.ambient_temperature(0));
        let warm_color = warm.resolved_sky_color(warm.ambient_temperature(0));

        assert!(
            warm_color.r >= cold_color.r,
            "warmer temperatures should not reduce red channel"
        );
        assert!(
            warm_color.b >= cold_color.b,
            "warmer temperatures should not reduce blue channel"
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
    fn precipitation_clamps_to_range() {
        let wet = EnvironmentSettings::new(0).with_precipitation(140);
        assert_eq!(wet.precipitation(), 100);

        let balanced = EnvironmentSettings::new(0).with_precipitation(42);
        assert_eq!(balanced.precipitation(), 42);

        let dry = EnvironmentSettings::new(0).with_precipitation(-180);
        assert_eq!(dry.precipitation(), -100);
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
    fn register_player_populates_snapshot_state() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(1, "Alice").with_wealth(75))?;
        let mut definition = Definition::from_script("Walker", "Walker", PASSIVE_PLAYER_SCRIPT)?;
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("Walker")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        assert_eq!(engine.player(1).unwrap().crew(), &[crew]);
        let snapshot = engine.snapshot();
        let player_state = snapshot
            .players
            .iter()
            .find(|state| state.id == 1)
            .expect("player state present");
        assert_eq!(player_state.name, "Alice");
        assert_eq!(player_state.wealth, 75);
        assert_eq!(player_state.status, PlayerStatus::Active);
        assert_eq!(player_state.crew, vec![crew]);
        Ok(())
    }

    #[test]
    fn player_asset_value_accounts_for_owned_objects() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(
            PlayerConfig::new(1, "Miner")
                .with_wealth(25)
                .with_points(10),
        )?;

        let mut definition = Definition::from_script("Ore", "Ore", PASSIVE_PLAYER_SCRIPT)?;
        definition.set_value(60);
        engine.register_definition(definition)?;

        engine.spawn_object(SpawnConfig::new("Ore").with_owner(1))?;

        engine.update_player_asset_values();

        let player = engine.player(1).expect("player present");
        assert_eq!(player.value(), 95);
        assert_eq!(player.value_gain(), 0);
        assert_eq!(player.objects_owned(), 1);
        Ok(())
    }

    #[test]
    fn player_cursor_tracks_selection_changes() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(1, "Cursor"))?;
        let mut definition =
            Definition::from_script("CursorCrew", "CursorCrew", PASSIVE_PLAYER_SCRIPT)?;
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("CursorCrew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        engine.select_crew(1, [crew])?;
        assert_eq!(engine.player(1).unwrap().cursor(), Some(crew));
        let snapshot = engine.snapshot();
        let cursor = snapshot
            .players
            .iter()
            .find(|state| state.id == 1)
            .and_then(|state| state.cursor);
        assert_eq!(cursor, Some(crew));
        Ok(())
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
    fn apply_command_uses_engine_order_for_selection() {
        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        global func OnIdleAbort(state, action) { return nil; }
        global func OnWalkStart(state, action) { return nil; }
        "#;

        let call_log: Arc<Mutex<Vec<(String, i32)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                if name == "OnIdleAbort" || name == "OnWalkStart" {
                    if let Some(Value::Proplist(state)) = args.get(0) {
                        if let Some(Value::Int(id)) = state.get("id") {
                            call_log.lock().unwrap().push((name.to_string(), *id));
                        }
                    }
                }
            });
        }

        let mut definition =
            Definition::from_script("Crew", "Crew", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);
        definition.set_crew_member(true);
        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_abort_call("OnIdleAbort"),
        );
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_start_call("OnWalkStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("Crew")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_id(ObjectId::new(200)),
            )
            .expect("first spawn succeeds");
        let second = engine
            .spawn_object(
                SpawnConfig::new("Crew")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_id(ObjectId::new(100)),
            )
            .expect("second spawn succeeds");

        engine
            .select_crew(1, vec![first, second])
            .expect("selection succeeds");

        engine
            .apply_command(
                1,
                CrewCommandTarget::selection(),
                ObjectUpdate::new().with_action("Walk"),
            )
            .expect("command applies");

        let log = call_log.lock().unwrap().clone();
        let expected = vec![
            ("OnIdleAbort".to_string(), 200),
            ("OnWalkStart".to_string(), 200),
            ("OnIdleAbort".to_string(), 100),
            ("OnWalkStart".to_string(), 100),
        ];
        assert_eq!(log, expected);
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
    fn engine_state_from_snapshot_allows_resuming_simulation() {
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(5, -3))
                    .with_velocity(Vector2::new(2, -1))
                    .with_energy(75),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let expected_next = engine.tick().expect("second tick succeeds");

        let state = EngineState::from_snapshot(&snapshot);

        let mut restored = Engine::with_seed(1234);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored.restore_state(&state).expect("state restores");

        let resumed = restored.tick().expect("tick after restore succeeds");
        assert_eq!(resumed, expected_next);
    }

    #[test]
    fn restore_snapshot_wrapper_matches_state_restore() {
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        engine
            .spawn_object(SpawnConfig::new("Test").with_velocity(Vector2::new(1, 0)))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let expected_next = engine.tick().expect("second tick succeeds");

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored
            .restore_snapshot(&snapshot)
            .expect("snapshot restores");

        let resumed = restored.tick().expect("tick after restore succeeds");
        assert_eq!(resumed, expected_next);
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
    fn capture_state_preserves_transfer_zones() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let object_id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        engine
            .set_transfer_zone(
                object_id,
                TransferZoneRect {
                    x: 12,
                    y: -3,
                    width: 8,
                    height: 10,
                },
            )
            .expect("set transfer zone succeeds");

        let state = engine.capture_state();
        assert_eq!(state.transfer_zones.len(), 1);
        let zone = &state.transfer_zones[0];
        assert_eq!(zone.owner, object_id);
        assert_eq!(zone.x, 12);
        assert_eq!(zone.y, -3);
        assert_eq!(zone.width, 8);
        assert_eq!(zone.height, 10);

        let mut restored = Engine::with_seed(3);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored.restore_state(&state).expect("restore succeeds");
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.transfer_zones.len(), 1);
        let restored_zone = &snapshot.transfer_zones[0];
        assert_eq!(restored_zone.owner, object_id);
        assert_eq!(restored_zone.x, 12);
        assert_eq!(restored_zone.y, -3);
        assert_eq!(restored_zone.width, 8);
        assert_eq!(restored_zone.height, 10);
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
    fn remove_effect_no_calls_skips_stop_callback() {
        let script = r#"
        global func Initialize(state, random)
        {
            AddEffect("Pulse", state);
            return nil;
        }

        global func FxPulseStart(state, effect)
        {
            return nil;
        }

        global func FxPulseTimer(state, effect, timer)
        {
            if (GetEffect("Pulse", state))
            {
                RemoveEffect("Pulse", state, 0, true);
            }
            return nil;
        }

        global func FxPulseStop(state, effect, reason)
        {
            return nil;
        }

        global func Step(state, frame, random)
        {
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

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert!(object.effects.is_empty());

        let calls = call_log.lock().unwrap().clone();
        let timer_calls = calls.iter().filter(|name| *name == "FxPulseTimer").count();
        let stop_calls = calls.iter().filter(|name| *name == "FxPulseStop").count();

        assert!(timer_calls >= 1);
        assert_eq!(stop_calls, 0);
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

    #[test]
    fn queued_commands_lower_landscape_columns() {
        let script = r#"
        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    commands = [
                        {
                            landscape = [
                                { op = "lower", start = 4, width = 3, height = 18 }
                            ]
                        }
                    ]
                };
            }
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Miner", "Miner", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(16, 10));

        engine
            .spawn_object(SpawnConfig::new("Miner"))
            .expect("spawn succeeds");

        let _ = engine.tick().expect("first tick succeeds");
        let surface = engine
            .landscape()
            .expect("landscape present")
            .surface()
            .to_vec();
        assert_eq!(surface[4], 10);
        assert_eq!(surface[6], 10);

        let _ = engine.tick().expect("second tick succeeds");
        let surface = engine
            .landscape()
            .expect("landscape present")
            .surface()
            .to_vec();
        assert_eq!(&surface[4..7], &[18, 18, 18]);
        assert_eq!(surface[7], 10);
    }

    #[test]
    fn queued_commands_set_and_clear_liquid_columns() {
        let script = r#"
        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    commands = [
                        {
                            landscape = [
                                { op = "set_liquid", column = 3, segments = [ { top = 5, bottom = 8 } ] }
                            ]
                        }
                    ]
                };
            }
            if (frame == 2) {
                return {
                    commands = [
                        {
                            landscape = [
                                { op = "clear_liquid", column = 3 }
                            ]
                        }
                    ]
                };
            }
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Diver", "Diver", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(16, 12));

        let diver_id = engine
            .spawn_object(SpawnConfig::new("Diver"))
            .expect("spawn succeeds");

        assert_eq!(engine.frame(), 0);

        let _ = engine.tick().expect("first tick succeeds");
        assert!(engine.landscape().expect("landscape present").liquids()[3]
            .segments()
            .is_empty());

        let _ = engine.tick().expect("second tick succeeds");
        assert_eq!(
            engine.landscape().expect("landscape present").liquids()[3].segments(),
            &[LiquidSegment::new(5, 8)]
        );

        let _ = engine.tick().expect("third tick succeeds");
        assert!(engine.landscape().expect("landscape present").liquids()[3]
            .segments()
            .is_empty());

        // Ensure object persistence unaffected by landscape edits
        assert!(engine.object_snapshot(diver_id).is_some());
    }

    #[test]
    fn scenario_script_applies_landscape_commands() -> Result<(), EngineError> {
        const SCRIPT: &str = r#"
        global func Initialize(state, random)
        {
            return {
                landscape = [
                    { op = "lower", start = 2, width = 2, height = 12 }
                ]
            };
        }

        global func Step(state, frame, random)
        {
            if (frame == 1)
            {
                return {
                    landscape = [
                        { op = "lower", start = 5, width = 2, height = 16 }
                    ]
                };
            }
            return nil;
        }
        "#;

        let mut engine = Engine::with_seed(11);
        engine.set_landscape(Landscape::flat(12, 8));

        engine
            .install_scenario_script("Scenario", SCRIPT)
            .expect("scenario script installs");

        let surface = engine
            .landscape()
            .expect("landscape present after install")
            .surface()
            .to_vec();
        assert_eq!(&surface[0..2], &[8, 8]);
        assert_eq!(&surface[2..4], &[12, 12]);

        let _snapshot = engine.tick()?;
        let surface = engine
            .landscape()
            .expect("landscape present after tick")
            .surface()
            .to_vec();
        assert_eq!(&surface[5..7], &[16, 16]);

        Ok(())
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

    #[test]
    fn tick_applies_temperature_conversions_to_landscape() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(0xC0);
        let library = MaterialLibrary::parse(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            Friction=15
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Water
            TempConvStrength=4

            [Material Water]
            Name=Water
            Density=60
            Friction=0
        "#,
        )
        .expect("material library parses");
        engine.configure_materials_from_library(&library);
        let ice = engine
            .materials()
            .id_of("Ice")
            .expect("ice material id available");
        engine.set_landscape(Landscape::flat_with_material(4, 10, Some(ice)));
        let environment = EnvironmentSettings::new(0).with_temperature(10);
        engine.set_environment(environment);

        let _ = engine.tick()?;

        let water = engine
            .materials()
            .id_of("Water")
            .expect("water material id available");
        let landscape = engine.landscape().expect("landscape present after tick");
        assert_eq!(landscape.solid_material_at(0), Some(water));
        assert_eq!(landscape.default_solid_material(), Some(water));

        Ok(())
    }
}
