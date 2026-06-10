#![allow(dead_code, unreachable_patterns, unused_variables)]
#![allow(
    clippy::doc_lazy_continuation,
    clippy::field_reassign_with_default,
    clippy::if_same_then_else,
    clippy::large_enum_variant,
    clippy::manual_clamp,
    clippy::match_like_matches_macro,
    clippy::needless_range_loop,
    clippy::question_mark,
    clippy::should_implement_trait,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::vec_init_then_push
)]

mod action;
mod command;
mod compat;
mod control;
mod effect;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod fixtures;
mod input;
mod landscape;
mod mass_mover;
mod material;
mod math;
mod message;
pub mod ocf;
#[cfg(test)]
mod parity_differential;
pub mod particles;
mod pathfinder;
pub mod pxs;
mod player;
mod record;
mod rng;
pub mod scenario;
mod sector;
mod sky;
#[cfg(test)]
mod test_game_call_ex;
mod transfer;

pub use action::{
    ActionLibrary, ActionProcedure, ActionSpec, ActionState, ActionUpdate, ActionUpdateResult,
};
pub use command::{CommandStackSnapshot, MenuRequest, MenuRequestKind};
pub use control::{
    interpret_player_control_command, CommandKind, ControlButton, ControlCommand, ControlEvent,
    ControlPacket, PlayerControlData, SyncCheckPacket, COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT,
    COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG, COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE,
    COM_MENU_DOWN, COM_MENU_ENTER, COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT,
    COM_MENU_SELECT, COM_MENU_SHOW_TEXT, COM_MENU_UP, COM_PLAYER_MENU, COM_RELEASE_OFFSET,
    COM_RIGHT, COM_SINGLE, COM_SPECIAL, COM_SPECIAL2, COM_THROW, COM_UP,
};
pub use effect::EffectState;
pub use input::PlayerInputState;
pub use landscape::{
    BlastResult, CollisionResolution, Landscape, LandscapeCommand, LandscapeError, LiquidColumn,
    LiquidSegment,
};
pub use material::{Material, MaterialId, MaterialSet};
pub use message::{
    MessageKind, MessageSnapshot, FLAG_ALIGN_CENTER, FLAG_ALIGN_LEFT, FLAG_ALIGN_RIGHT,
    FLAG_BOTTOM, FLAG_HCENTER, FLAG_LEFT, FLAG_NO_BREAK, FLAG_RIGHT, FLAG_TOP, FLAG_VCENTER,
    FLAG_WIDTH_REL, FLAG_X_REL, FLAG_Y_REL,
};
pub use pathfinder::{PathFinder, PathWaypoint};
pub use player::{Player, PlayerConfig, PlayerState, PlayerStatus, PlayerViewport};
pub use record::{Playback, PlaybackError, Recorder, Recording};
pub use scenario::{Scenario, ScenarioError, ScenarioObjectives, SkyConfig};
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

use command::{
    definition_id_to_c4id, AcquireScriptResult, CallResultAction, CommandDefinitionSnapshot,
    CommandEvent, CommandId, CommandObjectSnapshot, CommandOperation, CommandPlayerSnapshot,
    CommandRuntimeContext, CommandStack, CommandStepResult,
};
use compat::{
    enter_audio_context, enter_environment_context, enter_physics_context, enter_random_context,
    object_reference_value, AudioRegistry, DefinitionMetadata, EffectContextOutcome,
    EnvironmentDelta, HostWorldContext, HostWorldObject, LandscapeOperation, PhysicsDelta,
    PlayerCommand,
};
use effect::{EffectCommand, EffectEvent, EffectEventKind, EffectStopReason};
use material::{evaluate_corrosion, MaterialInteractionEvent, MaterialReactionKind};
use message::{MessageCommand, MessageManager, MessageSpec, PersistedMessage};
use ocf::NORMAL as OCF_NORMAL;
use sector::{SectorMap, SectorObject};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::AddAssign;
use std::path::Path;
use std::sync::Arc;

use crate::math::{
    fixed10, fixed100, fixed256, fixtoi, fixtoi_prec, itofix, itofix_prec, C4Fixed, FixedVec2,
};
pub use crate::rng::LcgRng;
use lc_resources::definition::{
    ActionFacet as ResourceActionFacet, TargetRect as ResourceTargetRect,
};
use lc_resources::{
    ActionDefinition as ResourceActionDefinition, PhysicalInfo, PictureRect as ResourcePictureRect,
    ResourceDefinition as ResourceDefinitionData, C4_MAX_PHYSICAL,
};
use lc_script::{DebuggerHooks, Engine as ScriptEngine, ScriptError, Value};
use mass_mover::MassMoverSet;
use rand::Rng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sky::SkyState;
use thiserror::Error;
use transfer::{TransferZoneCommand, TransferZoneRect, TransferZoneState, TransferZoneTable};

pub type DefinitionId = String;

pub const OWNER_NONE: i32 = -1;
pub const FULL_CON: i32 = 100_000;
const GAME_OVER_CHECK_INTERVAL: u8 = 35;
const FIRE_DEFINITION_ID: &str = "FLAM";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphicsOverlayMode {
    None = 0,
    Base = 1,
    Action = 2,
    Picture = 3,
    IngamePicture = 4,
    Object = 5,
    ExtraGraphics = 6,
}

impl GraphicsOverlayMode {
    pub fn from_script_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(GraphicsOverlayMode::None),
            1 => Some(GraphicsOverlayMode::Base),
            2 => Some(GraphicsOverlayMode::Action),
            3 => Some(GraphicsOverlayMode::Picture),
            4 => Some(GraphicsOverlayMode::IngamePicture),
            5 => Some(GraphicsOverlayMode::Object),
            6 => Some(GraphicsOverlayMode::ExtraGraphics),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DrawTransform {
    pub scale_x: f32,
    pub scale_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl DrawTransform {
    pub fn identity() -> Self {
        Self {
            scale_x: 1.0,
            scale_y: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    pub fn is_identity(&self) -> bool {
        (self.scale_x - 1.0).abs() <= f32::EPSILON
            && (self.scale_y - 1.0).abs() <= f32::EPSILON
            && self.offset_x.abs() <= f32::EPSILON
            && self.offset_y.abs() <= f32::EPSILON
    }

    pub fn from_components(scale_x: f32, scale_y: f32, offset_x: f32, offset_y: f32) -> Self {
        Self {
            scale_x,
            scale_y,
            offset_x,
            offset_y,
        }
    }

    pub fn combined(self, other: Self) -> Self {
        let delta_scale_x = other.scale_x;
        let delta_scale_y = other.scale_y;
        let delta_offset_x = other.offset_x;
        let delta_offset_y = other.offset_y;

        let mut combined = Self {
            scale_x: self.scale_x * delta_scale_x,
            scale_y: self.scale_y * delta_scale_y,
            offset_x: self.offset_x + self.scale_x * delta_offset_x,
            offset_y: self.offset_y + self.scale_y * delta_offset_y,
        };

        if combined.scale_x.abs() <= f32::EPSILON {
            combined.scale_x = 0.0;
        }
        if combined.scale_y.abs() <= f32::EPSILON {
            combined.scale_y = 0.0;
        }

        combined
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectGraphicsOverlay {
    pub id: i32,
    pub mode: GraphicsOverlayMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<DefinitionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default)]
    pub phase: i32,
    #[serde(default)]
    pub blit_mode: u32,
    #[serde(default)]
    pub color_modulation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_object: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<DrawTransform>,
}

impl ObjectGraphicsOverlay {
    pub fn new(id: i32, mode: GraphicsOverlayMode) -> Self {
        Self {
            id,
            mode,
            definition: None,
            graphics_name: None,
            action: None,
            phase: 0,
            blit_mode: 0,
            color_modulation: 0x00ff_ffff,
            overlay_object: None,
            transform: None,
        }
    }

    pub fn with_definition(mut self, definition: Option<DefinitionId>) -> Self {
        self.definition = definition;
        self
    }

    pub fn with_graphics_name(mut self, name: Option<String>) -> Self {
        self.graphics_name = name;
        self
    }

    pub fn with_action(mut self, action: Option<String>) -> Self {
        self.action = action;
        self
    }

    pub fn with_blit_mode(mut self, blit_mode: u32) -> Self {
        self.blit_mode = blit_mode;
        self
    }

    pub fn with_overlay_object(mut self, overlay_object: Option<ObjectId>) -> Self {
        self.overlay_object = overlay_object;
        self
    }

    pub fn with_transform(mut self, transform: Option<DrawTransform>) -> Self {
        self.transform = transform;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectBaseGraphics {
    pub definition: DefinitionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_name: Option<String>,
    #[serde(default)]
    pub blit_mode: u32,
}

pub const CNAT_NONE: u32 = 0;
pub const CNAT_LEFT: u32 = 1;
pub const CNAT_RIGHT: u32 = 2;
pub const CNAT_TOP: u32 = 4;
pub const CNAT_BOTTOM: u32 = 8;
pub const CNAT_CENTER: u32 = 16;
pub const CNAT_MULTI_ATTACH: u32 = 32;
pub const CNAT_NO_COLLISION: u32 = 64;
const CNAT_FLAGS: u32 = CNAT_MULTI_ATTACH | CNAT_NO_COLLISION;
const C4D_BORDER_SIDES: i32 = 1;
const C4D_BORDER_TOP: i32 = 2;
const C4D_BORDER_BOTTOM: i32 = 4;
const C4D_BORDER_LAYER: i32 = 8;
const CONTACT_DENSITY_SOLID: i32 = 50;
const C4M_VEHICLE: i32 = 100;
const ATTACH_RANGE: i32 = 5;
const FIX_FULL_CIRCLE: i32 = 360;
const FIX_HALF_CIRCLE: i32 = 180;

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

pub const LINE_CONNECT_POWER_INPUT: u32 = 1;
pub const LINE_CONNECT_POWER_OUTPUT: u32 = 1 << 1;
pub const LINE_CONNECT_LIQUID_INPUT: u32 = 1 << 2;
pub const LINE_CONNECT_LIQUID_OUTPUT: u32 = 1 << 3;
pub const LINE_CONNECT_POWER_GENERATOR: u32 = 1 << 4;
pub const LINE_CONNECT_POWER_CONSUMER: u32 = 1 << 5;
pub const LINE_CONNECT_LIQUID_PUMP: u32 = 1 << 6;
pub const LINE_CONNECT_CONNECT_ROPE: u32 = 1 << 7;
pub const LINE_CONNECT_ENERGY_HOLDER: u32 = 1 << 8;

fn default_rng() -> LcgRng {
    LcgRng::default()
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
#[derive(Default)]
pub enum ObjectStatus {
    Deleted,
    #[default]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandDirection {
    #[default]
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
    Meteorite { x: i32 },
    Earthquake { x: i32, y: i32 },
    Volcano { x: i32, y: i32, size: i32 },
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
    /// Raw `C4Fixed` `[x, y, xdir, ydir]` for C4PXS pixel sprites — the
    /// sync-relevant state; the float fields above are lossy projections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pxs_fixed: Option<[i32; 4]>,
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
            && self.pxs_fixed == other.pxs_fixed
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParticleCommand {
    Create(ParticleConfig),
    Clear {
        definition_id: Option<String>,
        scope: ParticleScope,
    },
    /// `C4ParticleSystem::Cast` via FnCastParticles/FnCastBackParticles
    /// (C4Script.cpp:4881-4908). Coordinates are world coordinates (the
    /// caller's local offset is already applied); `a0`/`a1` carry the script
    /// ints divided by 10, `b0`/`b1` the raw color bounds.
    Cast {
        definition_id: String,
        amount: i32,
        x: f32,
        y: f32,
        level: i32,
        a0: f32,
        b0: u32,
        a1: f32,
        b1: u32,
        layer: ParticleLayer,
    },
    /// `C4ParticleSystem::Push` via FnPushParticles (C4Script.cpp:4910-4923):
    /// deltas are script ints divided by 10; no def = push every particle.
    Push {
        definition_id: Option<String>,
        dxdir: f32,
        dydir: f32,
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
            pxs_fixed: None,
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

/// Snapshot form of a `C4ParticleSystem` particle (save/load + FFI surface).
fn system_particle_snapshot(particle: &particles::Particle) -> ParticleSnapshot {
    ParticleSnapshot {
        definition_id: particle.def_name.clone(),
        position: FloatVector2::new(particle.x, particle.y),
        velocity: FloatVector2::new(particle.xdir, particle.ydir),
        life: particle.life,
        parameter_a: particle.a,
        parameter_b: particle.b,
        layer: particle.layer.clone(),
        pxs_fixed: None,
    }
}

/// Snapshot form of a C4PXS pixel sprite. The float position/velocity are
/// `fixtof` projections for display; `pxs_fixed` carries the raw sync-relevant
/// `C4Fixed` state for lossless save/load.
fn pxs_snapshot(pxs: &pxs::Pxs, materials: &MaterialSet) -> ParticleSnapshot {
    let definition_id = materials
        .get_by_id(pxs.mat)
        .map(|material| format!("material/pxs/{}", material.normalized_name()))
        .unwrap_or_else(|| "material/pxs/unknown".to_string());
    ParticleSnapshot {
        definition_id,
        position: FloatVector2::new(math::fixtof(pxs.x), math::fixtof(pxs.y)),
        velocity: FloatVector2::new(math::fixtof(pxs.xdir), math::fixtof(pxs.ydir)),
        life: 0,
        parameter_a: 0.0,
        parameter_b: pxs.mat.index() as i32,
        layer: ParticleLayer::Global,
        pxs_fixed: Some([pxs.x.val(), pxs.y.val(), pxs.xdir.val(), pxs.ydir.val()]),
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
    #[serde(default)]
    pub wealth: i32,
    #[serde(default)]
    pub score: i32,
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

    pub fn gravity_as_c4fixed(&self) -> C4Fixed {
        fixed100(self.gravity) / 5
    }

    fn clamp_fixed_velocity(&self, velocity: &mut FixedVec2) {
        let min_vertical = self.max_rise_speed.min(self.max_fall_speed);
        let max_vertical = self.max_rise_speed.max(self.max_fall_speed);
        velocity.y =
            clamp_fixed_to_limit_pair(velocity.y, itofix(min_vertical), itofix(max_vertical));
        let max_horizontal = self.max_horizontal_speed.max(0);
        velocity.x = clamp_fixed_to_limit(velocity.x, max_horizontal);
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
        let duration_raw = (raw >> 16) & 0xFFFF;
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
    /// Scenario wind bounds (C4SWeather::Default: Wind.Set(0, 70, -100, 100),
    /// C4Scenario.cpp:377).
    #[serde(default = "default_wind_min")]
    pub wind_min: i32,
    #[serde(default = "default_wind_max")]
    pub wind_max: i32,
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
            wind_min: -100,
            wind_max: 100,
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

    pub fn temperature_at_height(&self, frame: u64, y: i32, world_height: i32) -> i32 {
        let ambient = self.ambient_temperature(frame);
        if world_height <= 0 {
            return ambient.clamp(-100, 100);
        }
        let clamped_height = y.clamp(0, world_height);
        let fraction = clamped_height as f32 / world_height as f32;
        let gradient = (fraction * 2.0) - 1.0;
        let max_offset = (self.temperature_range / 2).clamp(0, 50);
        let offset = (gradient * max_offset as f32).round() as i32;
        ambient.saturating_add(offset).clamp(-100, 100)
    }

    /// `C4Weather::Execute` (C4Weather.cpp:72-101): season and temperature
    /// step on Tick35 frames; `TargetWind = C4S.Weather.Wind.Evaluate()` on
    /// Tick1000 frames — ONE synced draw,
    /// `BoundBy(Std + Random(2*Rnd + 1) - Rnd, Min, Max)`
    /// (C4SVal::Evaluate, C4Scenario.cpp:43-46); the wind itself steps ±1
    /// toward the target on Tick10 frames.
    pub fn advance_frame(&mut self, rng: &mut LcgRng, frame: u64) {
        self.refresh_runtime_fields();
        if frame % 35 == 0 {
            self.update_season();
            self.update_temperature_from_season();
        }
        if frame % 1000 == 0 {
            let rnd = self.wind_variation.max(0);
            self.wind_target = (self.base_wind + rng.random(2 * rnd + 1) - rnd)
                .clamp(self.wind_min, self.wind_max);
        }
        if frame % 10 == 0 {
            self.wind = (self.wind + (self.wind_target - self.wind).signum())
                .clamp(self.wind_min, self.wind_max);
        }
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

    /// The current wind (C4Weather::Wind). State advances in `advance_frame`
    /// with the C++ tick gates; the frame parameter is kept for caller
    /// compatibility but the value is the mutable wind state, matching
    /// `C4Weather::GetWind` minus the position-dependent tunnel check.
    pub fn wind_force(&self, _frame: u64) -> i32 {
        self.wind
    }

    fn apply_to_velocity(&self, velocity: &mut FixedVec2, frame: u64) {
        let wind_force = self.wind_force(frame);
        if wind_force != 0 {
            velocity.x += fixed100(wind_force);
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
        if self.temperature_range <= 0 {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EnvironmentFrame {
    pub settings: EnvironmentSettings,
    pub wind_force: i32,
    pub ambient_temperature: i32,
    #[serde(default)]
    pub precipitation: i32,
    #[serde(default)]
    pub sky_color: Option<RgbColor>,
}

fn default_wind_min() -> i32 {
    -100
}

fn default_wind_max() -> i32 {
    100
}

fn default_owner() -> i32 {
    OWNER_NONE
}

fn default_alive() -> bool {
    true
}

fn default_construction() -> i32 {
    FULL_CON
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectState {
    pub position: Vector2,
    pub velocity: Vector2,
    #[serde(default)]
    pub rotation: i32,
    pub energy: i32,
    #[serde(default)]
    pub damage: i32,
    #[serde(default)]
    pub magic_energy: i32,
    #[serde(default)]
    pub magic_capacity: i32,
    #[serde(default = "default_construction")]
    pub construction: i32,
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
    pub layer: Option<ObjectId>,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub components: HashMap<DefinitionId, u32>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default)]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    /// Per-object storage for script-level local variables
    /// These are initialized to nil in Construction() and persist across all function calls
    #[serde(default)]
    pub local_vars: HashMap<String, Value>,
    /// Burning state (C4Object::OnFire, C4Object.h:205). Set by Incinerate
    /// via the fire effect start (C4Effect.cpp:633); drives OCF_OnFire and
    /// the per-frame ExecFire burning.
    #[serde(default)]
    pub on_fire: bool,
    /// Fire animation phase 0..MaxFirePhase (C4Object::FirePhase; initialized
    /// to Random(MaxFirePhase) at fire start, C4Effect.cpp:634 — a synced
    /// draw).
    #[serde(default)]
    pub fire_phase: i32,
    /// Player that caused the fire (the fire effect's CausedBy var, read by
    /// C4Object::GetFireCausePlr for contact-incineration attribution).
    #[serde(default = "default_owner")]
    pub fire_caused_by: i32,
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
        if let Some(rotation) = delta.rotation {
            self.rotation = rotation.rem_euclid(360);
        }
        if let Some(energy) = delta.energy {
            self.energy = energy;
        }
        if let Some(damage) = delta.damage {
            self.damage = damage.max(0);
        }
        if let Some(magic_energy) = delta.magic_energy {
            self.magic_energy = magic_energy.max(0);
        }
        if let Some(magic_capacity) = delta.magic_capacity {
            self.magic_capacity = magic_capacity.max(0);
        }
        if let Some(construction) = delta.construction {
            self.construction = construction.clamp(0, FULL_CON);
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
        if let Some(overlays) = &delta.graphics_overlays {
            self.graphics_overlays = overlays.clone();
        }
        if let Some(transform) = &delta.draw_transform {
            self.draw_transform = *transform;
        }
        if let Some(base_graphics) = &delta.base_graphics {
            self.base_graphics = base_graphics.clone();
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
            if let Some(components) = &delta.components {
                self.components = components.clone();
            }
        }
        if let Some(local_vars) = &delta.local_vars {
            self.local_vars = local_vars.clone();
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

#[derive(Debug, Default, Clone, PartialEq)]
struct ObjectDelta {
    position: Option<Vector2>,
    velocity: Option<Vector2>,
    /// Sub-pixel velocity in 16.16 fixed-point. When present, this takes
    /// precedence over the whole-pixel `velocity` mirror so that script
    /// surfaces (e.g. `SetXDir`) can express fractional `C4Fixed` velocity
    /// exactly, matching C++ `pObj->xdir = itofix(n, prec)` (`C4Script.cpp:697`).
    fixed_velocity: Option<FixedVec2>,
    rotation: Option<i32>,
    /// Sub-pixel angular velocity (16.16 fixed-point degrees/frame) set by
    /// `SetRDir`. Mirrors C++ `pObj->rdir = itofix(n, prec)` (`C4Script.cpp:710`).
    rotation_velocity: Option<C4Fixed>,
    energy: Option<i32>,
    damage: Option<i32>,
    magic_energy: Option<i32>,
    magic_capacity: Option<i32>,
    construction: Option<i32>,
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
    graphics_overlays: Option<Vec<ObjectGraphicsOverlay>>,
    draw_transform: Option<Option<DrawTransform>>,
    base_graphics: Option<Option<ObjectBaseGraphics>>,
    components: Option<HashMap<DefinitionId, u32>>,
    local_vars: Option<HashMap<String, Value>>,
}

impl ObjectDelta {
    fn merge_update(&mut self, update: ObjectUpdate) {
        if let Some(position) = update.position {
            self.position = Some(position);
        }
        if let Some(velocity) = update.velocity {
            self.velocity = Some(velocity);
        }
        if let Some(fixed_velocity) = update.fixed_velocity {
            self.fixed_velocity = Some(fixed_velocity);
        }
        if let Some(rotation) = update.rotation {
            self.rotation = Some(rotation);
        }
        if let Some(rotation_velocity) = update.rotation_velocity {
            self.rotation_velocity = Some(rotation_velocity);
        }
        if let Some(energy) = update.energy {
            self.energy = Some(energy);
        }
        if let Some(construction) = update.construction {
            self.construction = Some(construction);
        }
        if let Some(damage) = update.damage {
            self.damage = Some(damage);
        }
        if let Some(magic_energy) = update.magic_energy {
            self.magic_energy = Some(magic_energy);
        }
        if let Some(magic_capacity) = update.magic_capacity {
            self.magic_capacity = Some(magic_capacity);
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
        if let Some(overlays) = update.graphics_overlays {
            self.graphics_overlays = Some(overlays);
        }
        if let Some(transform) = update.draw_transform {
            self.draw_transform = Some(transform);
        }
        if let Some(base_graphics) = update.base_graphics {
            self.base_graphics = Some(base_graphics);
        }
        if let Some(components) = update.components {
            self.components = Some(components);
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
            fixed_velocity: update.fixed_velocity,
            rotation: update.rotation,
            rotation_velocity: update.rotation_velocity,
            energy: update.energy,
            construction: update.construction,
            damage: update.damage,
            magic_energy: update.magic_energy,
            magic_capacity: update.magic_capacity,
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
            graphics_overlays: update.graphics_overlays,
            draw_transform: update.draw_transform,
            base_graphics: update.base_graphics,
            components: update.components,
            local_vars: update.local_vars,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjectUpdate {
    pub position: Option<Vector2>,
    pub velocity: Option<Vector2>,
    /// Sub-pixel velocity in 16.16 fixed-point, set by precision-aware script
    /// surfaces (`SetXDir`/`SetYDir`). Takes precedence over `velocity` when
    /// applied. Mirrors C++ storing velocity as `C4Fixed` (`C4Object.h` xdir/ydir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    /// Sub-pixel angular velocity (16.16 fixed degrees/frame) from `SetRDir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    #[serde(default)]
    pub rotation: Option<i32>,
    pub energy: Option<i32>,
    #[serde(default)]
    pub damage: Option<i32>,
    #[serde(default)]
    pub magic_energy: Option<i32>,
    #[serde(default)]
    pub magic_capacity: Option<i32>,
    #[serde(default)]
    pub construction: Option<i32>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_overlays: Option<Vec<ObjectGraphicsOverlay>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<Option<DrawTransform>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<Option<ObjectBaseGraphics>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<HashMap<DefinitionId, u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_vars: Option<HashMap<String, Value>>,
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

    pub fn with_rotation(mut self, rotation: i32) -> Self {
        self.rotation = Some(rotation);
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

    pub fn with_construction(mut self, construction: i32) -> Self {
        self.construction = Some(construction.clamp(0, FULL_CON));
        self
    }

    pub fn with_magic_energy(mut self, magic_energy: i32) -> Self {
        self.magic_energy = Some(magic_energy);
        self
    }

    pub fn with_magic_capacity(mut self, magic_capacity: i32) -> Self {
        self.magic_capacity = Some(magic_capacity);
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
            && self.fixed_velocity.is_none()
            && self.rotation.is_none()
            && self.rotation_velocity.is_none()
            && self.energy.is_none()
            && self.construction.is_none()
            && self.damage.is_none()
            && self.magic_energy.is_none()
            && self.magic_capacity.is_none()
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
            && self.graphics_overlays.is_none()
            && self.draw_transform.is_none()
            && self.base_graphics.is_none()
            && self.components.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub delay: u32,
    pub update: ObjectUpdate,
    pub effects: Vec<EffectCommand>,
    #[serde(default)]
    pub events: Vec<CommandEvent>,
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
            events: Vec::new(),
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
            events: Vec::new(),
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

    pub fn with_events(mut self, events: Vec<CommandEvent>) -> Self {
        self.events = events;
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
    fixed_position: FixedVec2,
    fixed_velocity: FixedVec2,
    /// 16.16 fixed-point rotation accumulator (C++ `fix_r`, `C4Object.h:149`).
    /// `state.rotation` (whole degrees) is its `fixtoi` projection.
    fixed_rotation: C4Fixed,
    /// 16.16 fixed-point angular velocity in degrees/frame (C++ `rdir`,
    /// `C4Object.h:150`). Set by `SetRDir`; applied as `fix_r += rdir * 5` each
    /// frame (`C4Movement.cpp:376`).
    rotation_velocity: C4Fixed,
    destroyed: bool,
    /// Last energy-loss causing player (C4Object::LastEnergyLossCausePlayer,
    /// read by AssignDeath for kill attribution). Not yet snapshot-persisted.
    last_energy_loss_cause: i32,
    /// Trained per-object physicals (the C4ObjectInfo::Physical analog —
    /// the info model is absent, so training clones the def physicals onto
    /// the object). None = use the definition physicals (GetPhysical
    /// fallback, C4Object.cpp:2118-2134). Not yet snapshot-persisted.
    physical_override: Option<PhysicalInfo>,
    command_queue: VecDeque<QueuedCommand>,
    commands: CommandStack,
    pending_action_events: VecDeque<ActionTransitionEvent>,
    material_contents: Vec<i32>,
    shape_template: ObjectShapeTemplate,
    own_shape_vertices: Option<Vec<ObjectVertex>>,
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

#[derive(Debug, Clone)]
struct ObjectShapeTemplate {
    vertices: Vec<ObjectVertex>,
    rect: Option<DefinitionRect>,
    stretch_growth: bool,
    rotateable: i32,
}

impl ObjectShapeTemplate {
    fn new(
        vertices: Vec<ObjectVertex>,
        rect: Option<DefinitionRect>,
        stretch_growth: bool,
        rotateable: i32,
    ) -> Self {
        Self {
            vertices,
            rect,
            stretch_growth,
            rotateable,
        }
    }
}

#[derive(Debug, Default)]
struct CommandQueueOutcome {
    spawns: Vec<SpawnConfig>,
    destroy: bool,
    effect_events: Vec<EffectEvent>,
    container_updates: Vec<ContainerUpdateRecord>,
    command_events: Vec<CommandEvent>,
    particles: Vec<ParticleCommand>,
}

/// Record a fixed-point vector in a snapshot only when it carries sub-pixel
/// detail beyond its whole-pixel projection — i.e. when `fixtoi(fixed)` would
/// not round-trip back to `fixed`. Returns `None` for whole-pixel values so the
/// snapshot stays minimal and reconstructs losslessly via `itofix(pixels)`.
fn subpixel_or_none(fixed: FixedVec2, pixels: Vector2) -> Option<FixedVec2> {
    if fixed == FixedVec2::from_ints(pixels.x, pixels.y) {
        None
    } else {
        Some(fixed)
    }
}

impl Object {
    fn new(
        id: ObjectId,
        definition_id: DefinitionId,
        state: ObjectState,
        shape_template: ObjectShapeTemplate,
        own_shape_vertices: Option<Vec<ObjectVertex>>,
    ) -> Self {
        let fixed_position = FixedVec2::from_ints(state.position.x, state.position.y);
        let fixed_velocity = FixedVec2::from_ints(state.velocity.x, state.velocity.y);
        let fixed_rotation = itofix(state.rotation);
        Self {
            id,
            definition_id,
            fixed_position,
            fixed_velocity,
            fixed_rotation,
            rotation_velocity: C4Fixed::ZERO,
            destroyed: matches!(state.status, ObjectStatus::Deleted),
            state,
            last_energy_loss_cause: OWNER_NONE,
            physical_override: None,
            command_queue: VecDeque::new(),
            commands: CommandStack::new(),
            pending_action_events: VecDeque::new(),
            material_contents: Vec::new(),
            shape_template,
            own_shape_vertices,
        }
    }

    #[allow(dead_code)]
    fn command_count(&self) -> usize {
        self.commands.len()
    }

    fn fixed_vec_to_pixels(value: FixedVec2) -> Vector2 {
        Vector2::new(value.int_x(), value.int_y())
    }

    fn position_pixels(&self) -> Vector2 {
        Self::fixed_vec_to_pixels(self.fixed_position)
    }

    fn velocity_pixels(&self) -> Vector2 {
        Self::fixed_vec_to_pixels(self.fixed_velocity)
    }

    fn set_position(&mut self, position: Vector2) {
        self.state.position = position;
        self.fixed_position = FixedVec2::from_ints(position.x, position.y);
    }

    fn set_velocity(&mut self, velocity: Vector2) {
        self.state.velocity = velocity;
        self.fixed_velocity = FixedVec2::from_ints(velocity.x, velocity.y);
    }

    fn set_velocity_preserving_subpixel(&mut self, velocity: Vector2) {
        fn preserve_axis(fixed: &mut C4Fixed, previous: i32, resolved: i32) {
            if resolved == previous {
                return;
            }
            if resolved == 0 {
                *fixed = C4Fixed::ZERO;
                return;
            }
            if previous != 0 && previous.signum() != resolved.signum() {
                *fixed = -*fixed;
                return;
            }
            *fixed = itofix(resolved);
        }

        let previous = self.state.velocity;
        preserve_axis(&mut self.fixed_velocity.x, previous.x, velocity.x);
        preserve_axis(&mut self.fixed_velocity.y, previous.y, velocity.y);
        self.refresh_velocity_from_fixed();
    }

    fn refresh_velocity_from_fixed(&mut self) {
        self.state.velocity = self.velocity_pixels();
    }

    fn shape_base_vertices(&self) -> &[ObjectVertex] {
        self.own_shape_vertices
            .as_deref()
            .unwrap_or(&self.shape_template.vertices)
    }

    fn unrotated_shape_vertices(&self) -> Vec<ObjectVertex> {
        transformed_shape_vertices(
            self.shape_base_vertices(),
            self.state.construction,
            self.shape_template.stretch_growth,
            0,
            0,
        )
    }

    fn current_shape_rect(&self) -> Option<DefinitionRect> {
        transformed_shape_rect(
            self.shape_template.rect,
            self.state.construction,
            self.shape_template.stretch_growth,
            self.shape_template.rotateable,
            self.state.rotation,
        )
    }

    fn refresh_shape_after_state_change(
        &mut self,
        previous_construction: i32,
        previous_rect: Option<DefinitionRect>,
        preserve_bottom: bool,
    ) {
        self.state.vertices = transformed_shape_vertices(
            self.shape_base_vertices(),
            self.state.construction,
            self.shape_template.stretch_growth,
            self.shape_template.rotateable,
            self.state.rotation,
        );

        let new_rect = self.current_shape_rect();
        if preserve_bottom && self.state.rotation.rem_euclid(360) == 0 {
            if let (Some(previous), Some(current)) = (previous_rect, new_rect) {
                if previous.height != current.height || previous.y != current.y {
                    let bottom = self
                        .state
                        .position
                        .y
                        .saturating_add(previous.y)
                        .saturating_add(previous.height);
                    self.set_position(Vector2::new(
                        self.state.position.x,
                        bottom
                            .saturating_sub(current.height)
                            .saturating_sub(current.y),
                    ));
                }
            }
        } else if self.state.category & CATEGORY_STRUCTURE != 0 {
            let step_size = FULL_CON / 100;
            let previous_step = previous_construction / step_size;
            let current_step = self.state.construction / step_size;
            let step_diff = current_step - previous_step;
            if step_diff > 0 {
                if let Some(rect) = self.shape_template.rect {
                    let previous_lift = previous_step * rect.height / 100;
                    let current_lift = current_step * rect.height / 100;
                    let lift = current_lift - previous_lift;
                    if lift != 0 {
                        self.set_position(Vector2::new(
                            self.state.position.x,
                            self.state.position.y.saturating_sub(lift),
                        ));
                    }
                }
            }
        }
    }

    fn set_owned_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.own_shape_vertices = Some(vertices);
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.refresh_shape_after_state_change(previous_construction, previous_rect, false);
    }

    fn set_construction(&mut self, construction: i32) {
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.state.construction = construction.clamp(0, FULL_CON);
        self.refresh_shape_after_state_change(previous_construction, previous_rect, true);
    }

    #[cfg(test)]
    fn set_fixed_velocity(&mut self, velocity: FixedVec2) {
        self.fixed_velocity = velocity;
        self.state.velocity = self.velocity_pixels();
    }

    fn clamp_velocity(&mut self, physics: &PhysicsSettings) {
        physics.clamp_fixed_velocity(&mut self.fixed_velocity);
        self.refresh_velocity_from_fixed();
    }

    fn apply_delta(
        &mut self,
        delta: &ObjectDelta,
        action_library: &ActionLibrary,
    ) -> ApplyDeltaOutcome {
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        let shape_changed =
            delta.construction.is_some() || delta.rotation.is_some() || delta.vertices.is_some();
        let outcome = self.state.apply_delta(delta, action_library);
        if let Some(position) = delta.position {
            self.fixed_position = FixedVec2::from_ints(position.x, position.y);
        } else {
            self.state.position = self.position_pixels();
        }
        if let Some(fixed_velocity) = delta.fixed_velocity {
            // Sub-pixel velocity is authoritative; derive the whole-pixel mirror
            // from it (matches C++ where xdir/ydir as C4Fixed are the source of
            // truth and the integer view is `fixtoi`).
            self.fixed_velocity = fixed_velocity;
            self.state.velocity = self.velocity_pixels();
        } else if let Some(velocity) = delta.velocity {
            self.fixed_velocity = FixedVec2::from_ints(velocity.x, velocity.y);
        } else {
            self.state.velocity = self.velocity_pixels();
        }
        if delta.rotation.is_some() {
            // An explicit rotation re-seeds the fixed accumulator, mirroring C++
            // forcing `fix_r = itofix(r)` (`C4Movement.cpp:418`).
            self.fixed_rotation = itofix(self.state.rotation);
        }
        if let Some(rotation_velocity) = delta.rotation_velocity {
            self.rotation_velocity = rotation_velocity;
        }
        if let Some(vertices) = &delta.vertices {
            self.own_shape_vertices = Some(vertices.clone());
        }
        if shape_changed {
            self.refresh_shape_after_state_change(
                previous_construction,
                previous_rect,
                delta.construction.is_some(),
            );
        }
        outcome
    }

    fn advance_fixed_position(&mut self) {
        self.fixed_position += self.fixed_velocity;
        self.state.position = self.position_pixels();
        self.state.velocity = self.velocity_pixels();
    }

    fn advance_fixed_position_per_pixel(
        &mut self,
        landscape: Option<&Landscape>,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        mut on_contact: impl FnMut(&mut Object, u32) -> Result<(), EngineError>,
    ) -> Result<MovementStepOutcome, EngineError> {
        let Some(landscape) = landscape else {
            let previous_position = self.state.position;
            self.advance_fixed_position();
            return Ok(MovementStepOutcome {
                no_attach: false,
                any_contact: false,
                solid_mask_removed: self.state.position != previous_position,
            });
        };

        if self.state.vertices.is_empty() {
            self.advance_fixed_position_heightmap(landscape, materials);
            return Ok(MovementStepOutcome::default());
        }

        if movement.attach != 0 {
            return self.advance_attached_shape_position(
                landscape,
                materials,
                movement,
                &mut on_contact,
            );
        }

        let mut outcome = MovementStepOutcome::default();
        let mut solid_mask_removed = false;
        self.fixed_position.x += self.fixed_velocity.x;
        let mut target_x = fixtoi(self.fixed_position.x);
        self.apply_side_bounds(&mut target_x, landscape, movement, &mut on_contact)?;
        while self.state.position.x != target_x {
            let next_x = self.state.position.x + sign_i32(target_x - self.state.position.x);
            let candidate = Vector2::new(next_x, self.state.position.y);
            let excluded_solid_mask = solid_mask_removed.then_some(movement.object_id);
            let contact = shape_contact_check(
                &self.state.vertices,
                candidate,
                landscape,
                materials,
                movement.solid_masks,
                excluded_solid_mask,
                movement.contact_density,
            );
            if contact.is_contact() {
                outcome.any_contact = true;
                on_contact(self, contact.contact_cnat)?;
                self.fixed_position.x = itofix(self.state.position.x);
                redirect_force(&mut self.fixed_velocity.x, &mut self.fixed_velocity.y, -1);
                apply_contact_friction(&mut self.fixed_velocity.y, contact.first_friction());
                break;
            }
            self.state.position.x = next_x;
            solid_mask_removed = true;
        }

        self.fixed_position.y += self.fixed_velocity.y;
        let mut target_y = fixtoi(self.fixed_position.y);
        self.apply_vertical_bounds(&mut target_y, landscape, movement, &mut on_contact)?;
        while self.state.position.y != target_y {
            let next_y = self.state.position.y + sign_i32(target_y - self.state.position.y);
            let candidate = Vector2::new(self.state.position.x, next_y);
            let excluded_solid_mask = solid_mask_removed.then_some(movement.object_id);
            let contact = shape_contact_check(
                &self.state.vertices,
                candidate,
                landscape,
                materials,
                movement.solid_masks,
                excluded_solid_mask,
                movement.contact_density,
            );
            if contact.is_contact() {
                outcome.any_contact = true;
                on_contact(self, contact.contact_cnat)?;
                self.fixed_position.y = itofix(self.state.position.y);
                apply_contact_friction(&mut self.fixed_velocity.x, contact.first_friction());
                if !contact.has_vertex_cnat(CNAT_LEFT) {
                    redirect_force(&mut self.fixed_velocity.y, &mut self.fixed_velocity.x, -1);
                } else if !contact.has_vertex_cnat(CNAT_RIGHT) {
                    redirect_force(&mut self.fixed_velocity.y, &mut self.fixed_velocity.x, 1);
                } else {
                    if movement.rotateable > 0 && contact.count() == 1 && !self.state.alive {
                        redirect_force(
                            &mut self.fixed_velocity.y,
                            &mut self.rotation_velocity,
                            -contact.first_weight(),
                        );
                    }
                    self.fixed_velocity.y = C4Fixed::ZERO;
                }
                break;
            }
            self.state.position.y = next_y;
            solid_mask_removed = true;
        }

        self.state.velocity = self.velocity_pixels();
        outcome.solid_mask_removed = solid_mask_removed;
        Ok(outcome)
    }

    fn advance_fixed_position_heightmap(&mut self, landscape: &Landscape, materials: &MaterialSet) {
        self.fixed_position.x += self.fixed_velocity.x;
        let target_x = fixtoi(self.fixed_position.x);
        while self.state.position.x != target_x {
            let next_x = self.state.position.x + sign_i32(target_x - self.state.position.x);
            let candidate = Vector2::new(next_x, self.state.position.y);
            if landscape
                .resolve_collision(candidate, self.state.velocity)
                .collided
            {
                self.fixed_position.x = itofix(self.state.position.x);
                self.fixed_velocity.x = C4Fixed::ZERO;
                self.apply_landscape_contact_material(landscape, materials, candidate.x);
                break;
            }
            self.state.position.x = next_x;
        }

        self.fixed_position.y += self.fixed_velocity.y;
        let target_y = fixtoi(self.fixed_position.y);
        while self.state.position.y != target_y {
            let next_y = self.state.position.y + sign_i32(target_y - self.state.position.y);
            let candidate = Vector2::new(self.state.position.x, next_y);
            if landscape
                .resolve_collision(candidate, self.state.velocity)
                .collided
            {
                self.fixed_position.y = itofix(self.state.position.y);
                self.fixed_velocity.y = C4Fixed::ZERO;
                self.apply_landscape_contact_material(landscape, materials, candidate.x);
                break;
            }
            self.state.position.y = next_y;
        }

        self.state.velocity = self.velocity_pixels();
    }

    fn advance_attached_shape_position(
        &mut self,
        landscape: &Landscape,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        on_contact: &mut impl FnMut(&mut Object, u32) -> Result<(), EngineError>,
    ) -> Result<MovementStepOutcome, EngineError> {
        self.fixed_position += self.fixed_velocity;
        let mut target_x = fixtoi(self.fixed_position.x);
        let mut target_y = fixtoi(self.fixed_position.y);
        self.apply_side_bounds(&mut target_x, landscape, movement, on_contact)?;
        self.apply_vertical_bounds(&mut target_y, landscape, movement, on_contact)?;

        let mut no_attach = false;
        let mut any_contact = false;
        let mut solid_mask_removed = false;
        let mut first_step = true;
        while first_step || self.state.position.x != target_x || self.state.position.y != target_y {
            first_step = false;
            let original = Vector2::new(
                self.state.position.x + sign_i32(target_x - self.state.position.x),
                self.state.position.y + sign_i32(target_y - self.state.position.y),
            );
            let mut candidate = original;
            if !shape_attach(
                &self.state.vertices,
                &mut candidate,
                movement.attach,
                landscape,
                materials,
                movement.solid_masks,
                solid_mask_removed.then_some(movement.object_id),
                movement.contact_density,
            ) {
                no_attach = true;
            }

            let contact = shape_contact_check(
                &self.state.vertices,
                candidate,
                landscape,
                materials,
                movement.solid_masks,
                solid_mask_removed.then_some(movement.object_id),
                movement.contact_density,
            );
            if contact.is_contact() {
                any_contact = true;
                on_contact(self, contact.contact_cnat)?;
                self.fixed_position =
                    FixedVec2::from_ints(self.state.position.x, self.state.position.y);
                break;
            }

            if candidate == self.state.position {
                break;
            }

            let override_x = candidate.x != original.x;
            let override_y = candidate.y != original.y;
            self.state.position = candidate;

            if override_x {
                target_x = self.state.position.x;
                self.fixed_velocity.x = C4Fixed::ZERO;
                self.fixed_position.x = itofix(self.state.position.x);
            }
            if override_y {
                target_y = self.state.position.y;
                self.fixed_velocity.y = C4Fixed::ZERO;
                self.fixed_position.y = itofix(self.state.position.y);
            }
            solid_mask_removed = true;
        }

        self.state.velocity = self.velocity_pixels();
        Ok(MovementStepOutcome {
            no_attach,
            any_contact,
            solid_mask_removed,
        })
    }

    fn apply_side_bounds(
        &mut self,
        target_x: &mut i32,
        landscape: &Landscape,
        movement: MovementContactConfig<'_>,
        on_contact: &mut impl FnMut(&mut Object, u32) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        if let Some(layer) = movement.layer_bounds {
            if layer.border_bound & C4D_BORDER_LAYER != 0
                && !matches!(movement.action_procedure, ActionProcedure::Attach)
            {
                let shape_x = movement.shape_rect.map(|shape| shape.x).unwrap_or(0);
                let (low, high) = if self.state.category & CATEGORY_STATIC_BACK != 0 {
                    (
                        layer.position.x + layer.shape_rect.x,
                        layer.position.x + layer.shape_rect.x + layer.shape_rect.width,
                    )
                } else {
                    (
                        layer.position.x + layer.shape_rect.x - shape_x,
                        layer.position.x + layer.shape_rect.x + layer.shape_rect.width + shape_x,
                    )
                };
                if let Some(bound) = target_bounds(&mut self.fixed_position.x, low, high) {
                    *target_x = fixtoi(self.fixed_position.x);
                    self.fixed_velocity.x = C4Fixed::ZERO;
                    self.state.velocity = self.velocity_pixels();
                    let cnat = if bound < 0 { CNAT_LEFT } else { CNAT_RIGHT };
                    on_contact(self, cnat)?;
                }
            }
        }

        if movement.border_bound & C4D_BORDER_SIDES == 0 {
            return Ok(());
        }
        let shape_x = movement.shape_rect.map(|shape| shape.x).unwrap_or(0);
        if let Some(bound) = target_bounds(
            &mut self.fixed_position.x,
            -shape_x,
            landscape.width() as i32 + shape_x,
        ) {
            *target_x = fixtoi(self.fixed_position.x);
            self.fixed_velocity.x = C4Fixed::ZERO;
            self.state.velocity = self.velocity_pixels();
            let cnat = if bound < 0 { CNAT_LEFT } else { CNAT_RIGHT };
            on_contact(self, cnat)?;
        }
        Ok(())
    }

    fn apply_vertical_bounds(
        &mut self,
        target_y: &mut i32,
        landscape: &Landscape,
        movement: MovementContactConfig<'_>,
        on_contact: &mut impl FnMut(&mut Object, u32) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let shape_y = movement.shape_rect.map(|shape| shape.y).unwrap_or(0);
        if let Some(layer) = movement.layer_bounds {
            if layer.border_bound & C4D_BORDER_LAYER != 0
                && !matches!(movement.action_procedure, ActionProcedure::Attach)
            {
                let (low, high) = if self.state.category & CATEGORY_STATIC_BACK != 0 {
                    (
                        layer.position.y + layer.shape_rect.y,
                        layer.position.y + layer.shape_rect.y + layer.shape_rect.height,
                    )
                } else {
                    (
                        layer.position.y + layer.shape_rect.y - shape_y,
                        layer.position.y + layer.shape_rect.y + layer.shape_rect.height + shape_y,
                    )
                };
                if let Some(bound) = target_bounds(&mut self.fixed_position.y, low, high) {
                    *target_y = fixtoi(self.fixed_position.y);
                    self.fixed_velocity.y = C4Fixed::ZERO;
                    self.state.velocity = self.velocity_pixels();
                    let cnat = if bound < 0 { CNAT_TOP } else { CNAT_BOTTOM };
                    on_contact(self, cnat)?;
                }
            }
        }

        if movement.border_bound & C4D_BORDER_TOP != 0 && self.fixed_position.y < itofix(-shape_y) {
            self.fixed_position.y = itofix(-shape_y);
            *target_y = fixtoi(self.fixed_position.y);
            self.fixed_velocity.y = C4Fixed::ZERO;
            self.state.velocity = self.velocity_pixels();
            on_contact(self, CNAT_TOP)?;
        }
        if movement.border_bound & C4D_BORDER_BOTTOM != 0 {
            let bottom = landscape.estimated_height() + shape_y;
            if self.fixed_position.y > itofix(bottom) {
                self.fixed_position.y = itofix(bottom);
                *target_y = fixtoi(self.fixed_position.y);
                self.fixed_velocity.y = C4Fixed::ZERO;
                self.state.velocity = self.velocity_pixels();
                on_contact(self, CNAT_BOTTOM)?;
            }
        }
        self.state.velocity = self.velocity_pixels();
        Ok(())
    }

    fn apply_landscape_contact_material(
        &mut self,
        landscape: &Landscape,
        materials: &MaterialSet,
        x: i32,
    ) {
        if let Some(material_id) = landscape.solid_material_at(x) {
            if let Some(material) = materials.get_by_id(material_id) {
                self.apply_material_interaction(material);
            }
        }
    }

    /// Accumulate angular velocity into the fixed rotation, mirroring the fixed
    /// state pieces of C++ `C4Movement.cpp:373-436`: rotation only advances for
    /// rotateable definitions, `fix_r += rdir * 5`, finite `Def->Rotateable`
    /// ranges clamp `fix_r`/zero `rdir`, then contact-aware rotation walks one
    /// degree at a time before the half-circle wrap projects the integer degree.
    fn advance_fixed_rotation(
        &mut self,
        landscape: Option<&Landscape>,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        no_attach: bool,
        solid_mask_removed: bool,
        mut on_contact: impl FnMut(&mut Object, u32) -> Result<(), EngineError>,
    ) -> Result<bool, EngineError> {
        if movement.rotateable <= 0 {
            self.fixed_rotation = C4Fixed::ZERO;
            self.rotation_velocity = C4Fixed::ZERO;
            self.state.rotation = 0;
            return Ok(false);
        }
        if !self.rotation_velocity.is_nonzero() {
            return Ok(false);
        }
        self.fixed_rotation += self.rotation_velocity * 5;
        if movement.rotateable > 1 {
            let limit = itofix(movement.rotateable);
            if self.fixed_rotation > limit {
                self.fixed_rotation = limit;
                self.rotation_velocity = C4Fixed::ZERO;
            }
            if self.fixed_rotation < -limit {
                self.fixed_rotation = -limit;
                self.rotation_velocity = C4Fixed::ZERO;
            }
        }

        let target_rotation = fixtoi(self.fixed_rotation);
        let mut any_contact = false;
        if let Some(landscape) = landscape {
            if !self.state.vertices.is_empty() {
                any_contact = self.advance_fixed_rotation_with_contact(
                    target_rotation,
                    landscape,
                    materials,
                    movement,
                    no_attach,
                    solid_mask_removed,
                    &mut on_contact,
                )?;
            } else {
                self.state.rotation = target_rotation;
            }
        } else {
            self.state.rotation = target_rotation;
        }

        // Circle bounds: keep fix_r within (-180°, 180°]. C4Movement.cpp:434-435.
        let half_circle = itofix(FIX_HALF_CIRCLE);
        let full_circle = itofix(FIX_FULL_CIRCLE);
        if self.fixed_rotation < -half_circle {
            self.fixed_rotation += full_circle;
            self.state.rotation = fixtoi(self.fixed_rotation);
        }
        if self.fixed_rotation > half_circle {
            self.fixed_rotation -= full_circle;
            self.state.rotation = fixtoi(self.fixed_rotation);
        }
        Ok(any_contact)
    }

    fn advance_fixed_rotation_with_contact(
        &mut self,
        target_rotation: i32,
        landscape: &Landscape,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        no_attach: bool,
        solid_mask_removed: bool,
        on_contact: &mut impl FnMut(&mut Object, u32) -> Result<(), EngineError>,
    ) -> Result<bool, EngineError> {
        let fallback_base;
        let base_vertices = if movement.definition_vertices.is_empty() {
            fallback_base = self.state.vertices.clone();
            fallback_base.as_slice()
        } else {
            movement.definition_vertices
        };

        let mut any_contact = false;
        while self.state.rotation != target_rotation {
            let previous_rotation = self.state.rotation;
            let previous_vertices = self.state.vertices.clone();
            let previous_position = self.state.position;

            self.state.rotation += sign_i32(target_rotation - self.state.rotation);
            self.state.vertices = rotated_vertices(base_vertices, self.state.rotation);

            let mut candidate_position = self.state.position;
            if movement.attach != 0 && !no_attach {
                shape_attach(
                    &self.state.vertices,
                    &mut candidate_position,
                    movement.attach,
                    landscape,
                    materials,
                    movement.solid_masks,
                    solid_mask_removed.then_some(movement.object_id),
                    movement.contact_density,
                );
            }

            let contact = shape_contact_check(
                &self.state.vertices,
                candidate_position,
                landscape,
                materials,
                movement.solid_masks,
                solid_mask_removed.then_some(movement.object_id),
                movement.contact_density,
            );
            if contact.is_contact() {
                any_contact = true;
                on_contact(self, contact.contact_cnat)?;
                self.state.rotation = previous_rotation;
                self.state.vertices = previous_vertices;
                self.state.position = previous_position;
                self.fixed_position =
                    FixedVec2::from_ints(previous_position.x, previous_position.y);
                self.fixed_rotation = itofix(previous_rotation);
                if contact.count() == 1 {
                    redirect_force(&mut self.rotation_velocity, &mut self.fixed_velocity.y, -1);
                }
                self.rotation_velocity = C4Fixed::ZERO;
                self.refresh_velocity_from_fixed();
                break;
            }

            self.state.position = candidate_position;
            self.fixed_position = FixedVec2::from_ints(candidate_position.x, candidate_position.y);
        }
        Ok(any_contact)
    }

    fn apply_command_operations<I>(&mut self, operations: I)
    where
        I: IntoIterator<Item = CommandOperation>,
    {
        for operation in operations {
            match operation {
                CommandOperation::Clear => self.commands.clear(),
                CommandOperation::PushFront(request) => {
                    let _ = self.commands.push_front(request);
                }
                CommandOperation::PushBack(request) => {
                    let _ = self.commands.push_back(request);
                }
            }
        }
    }

    fn step_command_stack(&mut self, ctx: CommandRuntimeContext<'_>) -> Option<CommandStepResult> {
        self.commands.step(&ctx)
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
        let position = self.position_pixels();
        let velocity = self.velocity_pixels();
        // Persist the exact `rdir`/`fix_r` only while actively rotating; a static
        // object's orientation is fully captured by the whole-degree `rotation`.
        let rotation_state = if self.rotation_velocity.is_nonzero() {
            (Some(self.rotation_velocity), Some(self.fixed_rotation))
        } else {
            (None, None)
        };
        ObjectSnapshot {
            id: self.id,
            definition_id: self.definition_id.clone(),
            position,
            velocity,
            rotation: self.state.rotation.rem_euclid(360),
            energy: self.state.energy,
            damage: self.state.damage,
            magic_energy: self.state.magic_energy,
            magic_capacity: self.state.magic_capacity,
            construction: self.state.construction,
            action: self.state.action.clone(),
            direction: self.state.direction,
            command_direction: self.state.command_direction,
            action_procedure: procedure,
            effects: self.state.effects.clone(),
            vertices: self.state.vertices.clone(),
            own_vertices: self.own_shape_vertices.clone(),
            container: self.state.container,
            contents: self.state.contents.clone(),
            components: self.state.components.clone(),
            status: self.state.status,
            owner: self.state.owner,
            category: self.state.category,
            crew_member: self.state.crew_member,
            alive: self.state.alive,
            base_graphics: self.state.base_graphics.clone(),
            graphics_overlays: self.state.graphics_overlays.clone(),
            draw_transform: self.state.draw_transform,
            command_queue: self.command_queue.iter().cloned().collect(),
            command_stack: self.commands.snapshot(),
            local_vars: self.state.local_vars.clone(),
            on_fire: self.state.on_fire,
            fire_phase: self.state.fire_phase,
            fire_caused_by: self.state.fire_caused_by,
            fixed_position: subpixel_or_none(self.fixed_position, position),
            fixed_velocity: subpixel_or_none(self.fixed_velocity, velocity),
            rotation_velocity: rotation_state.0,
            fixed_rotation: rotation_state.1,
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
            self.fixed_velocity.x =
                apply_horizontal_friction_fixed(self.fixed_velocity.x, friction);
            self.refresh_velocity_from_fixed();
            for vertex in &mut self.state.vertices {
                vertex.friction = friction;
            }
        }
    }

    fn apply_collision_resolution(&mut self, resolution: &CollisionResolution) {
        self.set_position(resolution.position);
        self.set_velocity_preserving_subpixel(resolution.velocity);
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
            let delta_outcome = self.apply_delta(&delta, action_library);
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
            self.clamp_velocity(physics);
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
            if !command.events.is_empty() {
                outcome.command_events.extend(command.events);
            }
            if !command.particles.is_empty() {
                outcome.particles.extend(command.particles);
            }
            if let Some(landscape_ref) = &mut landscape {
                for op in command.landscape.iter() {
                    op.apply(landscape_ref);
                }
            }
            if let Some(landscape_ref) = &mut landscape {
                let resolution =
                    (**landscape_ref).resolve_collision(self.state.position, self.state.velocity);
                if resolution.collided {
                    self.apply_collision_resolution(&resolution);
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
    #[serde(default)]
    pub rotation: i32,
    pub energy: i32,
    #[serde(default = "default_construction")]
    pub construction: i32,
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
    pub layer: Option<ObjectId>,
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
            rotation: 0,
            energy: 0,
            construction: FULL_CON,
            action: None,
            direction: Direction::default(),
            command_direction: CommandDirection::default(),
            effects: Vec::new(),
            vertices: Vec::new(),
            owner: OWNER_NONE,
            crew_member: None,
            status: None,
            container: None,
            layer: None,
            alive: None,
            category: None,
        }
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = position;
        self
    }

    pub fn with_rotation(mut self, rotation: i32) -> Self {
        self.rotation = rotation;
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

    pub fn with_construction(mut self, construction: i32) -> Self {
        self.construction = construction.clamp(0, FULL_CON);
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

    pub fn with_layer(mut self, layer: ObjectId) -> Self {
        self.layer = Some(layer);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub id: ObjectId,
    pub definition_id: DefinitionId,
    pub position: Vector2,
    pub velocity: Vector2,
    #[serde(default)]
    pub rotation: i32,
    pub energy: i32,
    #[serde(default)]
    pub damage: i32,
    #[serde(default)]
    pub magic_energy: i32,
    #[serde(default)]
    pub magic_capacity: i32,
    #[serde(default = "default_construction")]
    pub construction: i32,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_vertices: Option<Vec<ObjectVertex>>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub components: HashMap<DefinitionId, u32>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default)]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    #[serde(default)]
    pub command_queue: Vec<QueuedCommand>,
    #[serde(default)]
    pub command_stack: CommandStackSnapshot,
    #[serde(default)]
    pub local_vars: HashMap<String, Value>,
    /// Burning state (C4Object::OnFire) with its animation phase and the
    /// causing player (the fire effect's CausedBy var).
    #[serde(default)]
    pub on_fire: bool,
    #[serde(default)]
    pub fire_phase: i32,
    #[serde(default = "default_owner")]
    pub fire_caused_by: i32,
    /// Raw 16.16 fixed-point position, recorded only when it carries sub-pixel
    /// detail beyond the whole-pixel `position` (i.e. `position != fixtoi(fix)`).
    /// `None` ⇒ reconstruct losslessly via `itofix(position)`. Mirrors C++
    /// persisting both `x` and `fix_x` (`C4Object.cpp:2742`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_position: Option<FixedVec2>,
    /// Raw 16.16 fixed-point velocity, recorded only when it carries sub-pixel
    /// detail beyond the whole-pixel `velocity`. `None` ⇒ `itofix(velocity)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    /// Raw 16.16 fixed-point angular velocity (`rdir`) and rotation accumulator
    /// (`fix_r`), recorded together only while the object is actively rotating
    /// (`rdir != 0`). `None` ⇒ a static object whose orientation is fully
    /// captured by `rotation`. Preserves rotation across save/restore so a
    /// reloaded spinning object stays in lockstep (C++ persists `fix_r`/`rdir`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_rotation: Option<C4Fixed>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationSnapshot {
    pub frame: u64,
    #[serde(default)]
    pub game_over: bool,
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
    pub rng: LcgRng,
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
    pub menu_requests: Vec<MenuRequest>,
    #[serde(default)]
    pub audio: Vec<AudioCommand>,
}

impl SimulationSnapshot {
    pub fn object(&self, id: ObjectId) -> Option<&ObjectSnapshot> {
        self.objects.iter().find(|object| object.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedObject {
    pub snapshot: ObjectSnapshot,
    #[serde(default)]
    pub command_queue: Vec<QueuedCommand>,
    #[serde(default)]
    pub command_stack: CommandStackSnapshot,
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
    #[serde(default)]
    pub pending_menu_requests: Vec<MenuRequest>,
    #[serde(default)]
    pub game_over: bool,
    #[serde(default)]
    pub landscape_insert_thrust: bool,
    pub rng: LcgRng,
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
        let physics = snapshot.physics.unwrap_or_default();

        let mut objects = Vec::with_capacity(snapshot.objects.len());
        for object in &snapshot.objects {
            objects.push(PersistedObject {
                snapshot: object.clone(),
                command_queue: object.command_queue.clone(),
                command_stack: object.command_stack.clone(),
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
            pending_menu_requests: snapshot.menu_requests.clone(),
            messages: Vec::new(),
            game_over: snapshot.game_over,
            landscape_insert_thrust: false,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DefinitionRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_positive(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    pub fn contains_offset(&self, dx: i32, dy: i32) -> bool {
        let local_x = dx - self.x;
        let local_y = dy - self.y;
        if local_x < 0 || local_y < 0 {
            return false;
        }
        if local_x >= self.width || local_y >= self.height {
            return false;
        }
        true
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        if !self.is_positive() {
            return false;
        }
        let local_x = i64::from(x) - i64::from(self.x);
        let local_y = i64::from(y) - i64::from(self.y);
        local_x >= 0
            && local_y >= 0
            && local_x < i64::from(self.width)
            && local_y < i64::from(self.height)
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        if !self.is_positive() || !other.is_positive() {
            return false;
        }
        let self_right = i64::from(self.x) + i64::from(self.width);
        let self_bottom = i64::from(self.y) + i64::from(self.height);
        let other_right = i64::from(other.x) + i64::from(other.width);
        let other_bottom = i64::from(other.y) + i64::from(other.height);
        i64::from(self.x) < other_right
            && i64::from(other.x) < self_right
            && i64::from(self.y) < other_bottom
            && i64::from(other.y) < self_bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionTargetRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

impl DefinitionTargetRect {
    pub const fn new(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        target_x: i32,
        target_y: i32,
    ) -> Self {
        Self {
            x,
            y,
            width,
            height,
            target_x,
            target_y,
        }
    }

    pub fn is_positive(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

impl From<ResourceTargetRect> for DefinitionTargetRect {
    fn from(rect: ResourceTargetRect) -> Self {
        Self::new(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.target_x,
            rect.target_y,
        )
    }
}

impl From<ResourcePictureRect> for DefinitionRect {
    fn from(rect: ResourcePictureRect) -> Self {
        Self::new(rect.x, rect.y, rect.width, rect.height)
    }
}

fn vertex_bounds_rect(position: Vector2, vertices: &[ObjectVertex]) -> Option<DefinitionRect> {
    let first = vertices.first()?;
    let mut min_x = first.x;
    let mut max_x = first.x;
    let mut min_y = first.y;
    let mut max_y = first.y;
    for vertex in &vertices[1..] {
        min_x = min_x.min(vertex.x);
        max_x = max_x.max(vertex.x);
        min_y = min_y.min(vertex.y);
        max_y = max_y.max(vertex.y);
    }
    Some(DefinitionRect::new(
        position.x.saturating_add(min_x),
        position.y.saturating_add(min_y),
        max_x.saturating_sub(min_x).saturating_add(1),
        max_y.saturating_sub(min_y).saturating_add(1),
    ))
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
    color_mask: Option<Arc<[u8]>>,
}

impl DefinitionSpriteImage {
    fn from_resource(
        image: &lc_resources::GraphicsImage,
        mask: Option<&lc_resources::ColorByOwnerMask>,
    ) -> Self {
        let color_mask = mask.and_then(|mask| {
            if mask.width != image.width() || mask.height != image.height() {
                return None;
            }
            if mask.pixels.iter().all(|value| *value == 0) {
                return None;
            }
            Some(Arc::from(mask.pixels.clone().into_boxed_slice()))
        });
        Self {
            width: image.width(),
            height: image.height(),
            pixels: image.clone_pixels(),
            color_mask,
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

    pub fn color_mask(&self) -> Option<Arc<[u8]>> {
        self.color_mask.as_ref().map(Arc::clone)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionComponent {
    pub id: DefinitionId,
    pub count: u32,
}

#[derive(Clone)]
pub struct Definition {
    id: DefinitionId,
    name: String,
    script: ScriptEngine,
    includes: Vec<String>,
    has_construction: bool,
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
    sprite_variants: HashMap<String, DefinitionSpriteImage>,
    shape: Option<DefinitionRect>,
    solid_mask: Option<DefinitionTargetRect>,
    shape_vertices: Vec<ObjectVertex>,
    contact_density: i32,
    contact_function_calls: bool,
    collection_rect: Option<DefinitionRect>,
    collection_limit: Option<u32>,
    collectible: bool,
    constructable: bool,
    construction_offset: i32,
    stretch_growth: bool,
    basement: i32,
    rotateable: i32,
    border_bound: i32,
    upright_attach: u32,
    components: Vec<DefinitionComponent>,
    line_connect: u32,
    /// ContactIncinerate=N: 1-in-N contact-fire chance (0 = not inflammable).
    contact_incinerate: i32,
    no_burn_decay: bool,
    no_burn_damage: bool,
    burn_turn_to: Option<String>,
    incomplete_activity: bool,
    /// The [Physical] DefCore section (C4Def::Physical).
    physical: PhysicalInfo,
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

        // Compile the script to extract includes before adding to engine
        let compiled_script =
            lc_script::Script::compile(source).map_err(|parse_error| EngineError::Script {
                definition: id.clone(),
                function: "load",
                source: parse_error.into(),
            })?;

        let includes = compiled_script.includes().to_vec();

        let mut script = ScriptEngine::new();
        script.add_script(compiled_script);
        compat::register_host_functions(&mut script);
        let has_construction = script.has_function("Construction");
        let has_initialize = script.has_function("Initialize");
        let has_step = script.has_function("Step");
        Ok(Self {
            id,
            name,
            script,
            includes,
            has_construction,
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
            sprite_variants: HashMap::new(),
            shape: None,
            solid_mask: None,
            shape_vertices: Vec::new(),
            contact_density: CONTACT_DENSITY_SOLID,
            contact_function_calls: false,
            collection_rect: None,
            collection_limit: None,
            collectible: false,
            constructable: false,
            construction_offset: 0,
            stretch_growth: false,
            basement: 0,
            rotateable: 0,
            border_bound: 0,
            upright_attach: 0,
            components: Vec::new(),
            line_connect: 0,
            contact_incinerate: 0,
            no_burn_decay: false,
            no_burn_damage: false,
            burn_turn_to: None,
            incomplete_activity: false,
            physical: PhysicalInfo::default(),
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

    pub fn includes(&self) -> &[String] {
        &self.includes
    }

    pub fn merge_from(&mut self, parent: &Definition) {
        self.script.merge_from(&parent.script);
        // Re-check function existence flags after merging parent functions
        if !self.has_construction {
            self.has_construction = self.script.has_function("Construction");
        }
        if !self.has_initialize {
            self.has_initialize = self.script.has_function("Initialize");
        }
    }

    pub fn function_count(&self) -> usize {
        self.script.function_count()
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
        definition.set_solid_mask(resource.core.solid_mask.map(DefinitionTargetRect::from));
        if let Some(image) = resource.picture_image.as_ref() {
            definition.set_picture_image(Some(DefinitionPictureImage::from_resource(image)));
        }
        if let Some(image) = resource.graphics_image.as_ref() {
            let mask = resource.color_by_owner_mask.as_ref();
            definition.set_sprite_image(Some(DefinitionSpriteImage::from_resource(image, mask)));
        }
        if !resource.additional_graphics.is_empty() {
            let mut variants = HashMap::with_capacity(resource.additional_graphics.len());
            for (key, variant) in &resource.additional_graphics {
                let mask = variant.color_by_owner_mask.as_ref();
                variants.insert(
                    key.clone(),
                    DefinitionSpriteImage::from_resource(&variant.image, mask),
                );
            }
            definition.set_sprite_variants(variants);
        }
        definition.set_shape_rect(resource.core.shape.map(DefinitionRect::from));
        definition.set_shape_vertices(
            resource
                .core
                .vertices
                .iter()
                .map(|vertex| {
                    ObjectVertex::new(vertex.x, vertex.y)
                        .with_cnat(vertex.cnat)
                        .with_friction(vertex.friction)
                })
                .collect(),
        );
        definition.set_contact_density(resource.core.contact_density);
        definition.set_contact_function_calls(resource.core.contact_function_calls);
        definition.set_collection_rect(resource.core.collection.map(DefinitionRect::from));
        definition.set_collection_limit(resource.core.collection_limit);
        definition.set_fire_properties(
            resource.core.contact_incinerate,
            resource.core.no_burn_decay,
            resource.core.no_burn_damage,
        );
        definition.set_burn_turn_to(resource.core.burn_turn_to.clone());
        definition.set_incomplete_activity(resource.core.incomplete_activity);
        definition.set_physical(resource.core.physical);
        definition.set_collectible(resource.core.collectible);
        definition.set_constructable(resource.core.constructable);
        definition.set_construction_offset(resource.core.con_size_off);
        definition.set_stretch_growth(resource.core.stretch_growth);
        definition.set_basement(resource.core.basement);
        definition.set_rotateable(resource.core.rotateable);
        definition.set_border_bound(resource.core.border_bound);
        definition.set_upright_attach(resource.core.upright_attach);
        if !resource.core.components.is_empty() {
            let components = resource
                .core
                .components
                .iter()
                .map(|component| DefinitionComponent {
                    id: component.id.clone(),
                    count: component.count,
                })
                .collect();
            definition.set_components(components);
        }
        definition.set_line_connect(resource.core.line_connect);
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
        if action.attach != 0 {
            spec = spec.with_attach(action.attach);
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
        if self.rotateable > 0 {
            self.ocf_base | crate::ocf::ROTATE
        } else {
            self.ocf_base
        }
    }

    pub fn set_ocf_base(&mut self, ocf: u32) {
        self.ocf_base = ocf | OCF_NORMAL;
    }

    pub fn rotateable(&self) -> i32 {
        self.rotateable
    }

    pub fn set_rotateable(&mut self, rotateable: i32) {
        self.rotateable = rotateable.max(0);
    }

    pub fn compute_ocf(&self, state: &ObjectState) -> u32 {
        let mut ocf = crate::ocf::compute(
            self.ocf_base(),
            self.crew_member,
            state.alive,
            state.status,
            state.container.is_some(),
            state.construction,
        );
        if self.collectible {
            ocf |= crate::ocf::CARRYABLE;
        }
        // OCF_OnFire (SetOCF, C4Object.cpp:559-561)
        if state.on_fire {
            ocf |= crate::ocf::ON_FIRE;
        }
        // OCF_Inflammable: not burning, ContactIncinerate set, not a dead
        // living (SetOCF, C4Object.cpp:562-566)
        if !state.on_fire
            && self.contact_incinerate > 0
            && (state.category & CATEGORY_LIVING == 0 || state.alive)
        {
            ocf |= crate::ocf::INFLAMMABLE;
        }
        if let Some(rect) = self.collection_rect {
            if rect.is_positive() {
                let below_limit = self
                    .collection_limit
                    .map(|limit| state.contents.len() < limit as usize)
                    .unwrap_or(true);
                if below_limit {
                    ocf |= crate::ocf::COLLECTION;
                }
            }
        }
        ocf
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

    pub fn sprite_image_variant(
        &self,
        graphics_name: Option<&str>,
    ) -> Option<&DefinitionSpriteImage> {
        match graphics_name {
            None | Some("") => self.sprite_image.as_ref(),
            Some(name) => {
                let key = name.to_ascii_lowercase();
                self.sprite_variants.get(&key)
            }
        }
    }

    pub fn set_sprite_variants(&mut self, variants: HashMap<String, DefinitionSpriteImage>) {
        self.sprite_variants = variants;
    }

    pub fn sprite_variant_keys(&self) -> Vec<String> {
        self.sprite_variants.keys().cloned().collect()
    }

    pub fn shape_rect(&self) -> Option<DefinitionRect> {
        self.shape
    }

    pub fn set_shape_rect(&mut self, rect: Option<DefinitionRect>) {
        self.shape = rect;
    }

    pub fn solid_mask(&self) -> Option<DefinitionTargetRect> {
        self.solid_mask
    }

    pub fn set_solid_mask(&mut self, rect: Option<DefinitionTargetRect>) {
        self.solid_mask = rect.filter(DefinitionTargetRect::is_positive);
    }

    pub fn shape_vertices(&self) -> &[ObjectVertex] {
        &self.shape_vertices
    }

    pub fn set_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.shape_vertices = vertices;
    }

    pub fn contact_density(&self) -> i32 {
        self.contact_density
    }

    pub fn set_contact_density(&mut self, contact_density: i32) {
        self.contact_density = contact_density;
    }

    pub fn contact_function_calls(&self) -> bool {
        self.contact_function_calls
    }

    pub fn set_contact_function_calls(&mut self, contact_function_calls: bool) {
        self.contact_function_calls = contact_function_calls;
    }

    pub fn border_bound(&self) -> i32 {
        self.border_bound
    }

    pub fn set_border_bound(&mut self, border_bound: i32) {
        self.border_bound = border_bound.max(0);
    }

    pub fn upright_attach(&self) -> u32 {
        self.upright_attach
    }

    pub fn set_upright_attach(&mut self, upright_attach: u32) {
        self.upright_attach = upright_attach;
    }

    pub fn collection_rect(&self) -> Option<DefinitionRect> {
        self.collection_rect
    }

    pub fn set_collection_rect(&mut self, rect: Option<DefinitionRect>) {
        self.collection_rect = rect.and_then(|r| if r.is_positive() { Some(r) } else { None });
    }

    pub fn collection_limit(&self) -> Option<u32> {
        self.collection_limit
    }

    pub fn contact_incinerate(&self) -> i32 {
        self.contact_incinerate
    }

    pub fn no_burn_decay(&self) -> bool {
        self.no_burn_decay
    }

    pub fn no_burn_damage(&self) -> bool {
        self.no_burn_damage
    }

    pub fn set_fire_properties(
        &mut self,
        contact_incinerate: i32,
        no_burn_decay: bool,
        no_burn_damage: bool,
    ) {
        self.contact_incinerate = contact_incinerate.max(0);
        self.no_burn_decay = no_burn_decay;
        self.no_burn_damage = no_burn_damage;
    }

    pub fn burn_turn_to(&self) -> Option<&str> {
        self.burn_turn_to.as_deref()
    }

    pub fn incomplete_activity(&self) -> bool {
        self.incomplete_activity
    }

    pub fn set_burn_turn_to(&mut self, target: Option<String>) {
        self.burn_turn_to = target;
    }

    pub fn set_incomplete_activity(&mut self, enabled: bool) {
        self.incomplete_activity = enabled;
    }

    pub fn physical(&self) -> &PhysicalInfo {
        &self.physical
    }

    pub fn set_physical(&mut self, physical: PhysicalInfo) {
        self.physical = physical;
    }

    pub fn set_collection_limit(&mut self, limit: Option<u32>) {
        self.collection_limit = limit.and_then(|value| if value > 0 { Some(value) } else { None });
    }

    pub fn is_collectible(&self) -> bool {
        self.collectible
    }

    pub fn set_collectible(&mut self, collectible: bool) {
        self.collectible = collectible;
    }

    pub fn is_constructable(&self) -> bool {
        self.constructable
    }

    pub fn set_constructable(&mut self, constructable: bool) {
        self.constructable = constructable;
    }

    pub fn construction_offset(&self) -> i32 {
        self.construction_offset
    }

    pub fn set_construction_offset(&mut self, offset: i32) {
        self.construction_offset = offset.max(0);
    }

    pub fn stretch_growth(&self) -> bool {
        self.stretch_growth
    }

    pub fn set_stretch_growth(&mut self, stretch_growth: bool) {
        self.stretch_growth = stretch_growth;
    }

    pub fn basement(&self) -> i32 {
        self.basement
    }

    pub fn set_basement(&mut self, basement: i32) {
        self.basement = basement.max(0);
    }

    pub fn components(&self) -> &[DefinitionComponent] {
        &self.components
    }

    pub fn set_components(&mut self, components: Vec<DefinitionComponent>) {
        self.components = components;
    }

    pub fn line_connect(&self) -> u32 {
        self.line_connect
    }

    pub fn set_line_connect(&mut self, line_connect: u32) {
        self.line_connect = line_connect;
    }

    fn call_initialize(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        random: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(CommandBatch, AudioRegistry, LcgRng, u64), EngineError> {
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
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    state.action.phase,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    "Initialize",
                    &args,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        let (result, updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "Initialize",
            source,
        })?;
        let mut batch = parse_command(&self.id, "Initialize", result)?;
        // Store updated local variables in the delta so they persist
        batch.delta.local_vars = Some(updated_local_vars);
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            next_object_id,
        } = host_effects;
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }

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
        if !command_operations.is_empty() {
            batch.command_ops.extend(command_operations);
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
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
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
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id))
    }

    fn call_construction(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(CommandBatch, AudioRegistry, LcgRng, u64), EngineError> {
        if !self.has_construction {
            return Ok((CommandBatch::default(), audio, rng, world.next_object_id()));
        }
        // Construction() takes no arguments
        let args: [Value; 0] = [];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    state.action.phase,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    "Construction",
                    &args,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        let (_result, updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "Construction",
            source,
        })?;
        // Construction() return value is not used (it just returns 0 or nil)
        // We only care about side effects (initializing local variables, etc.)
        // But we DO need to capture updated local variable values
        let mut batch = CommandBatch::default();
        // Store updated local variables in the delta so they persist
        batch.delta.local_vars = Some(updated_local_vars);
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            next_object_id,
        } = host_effects;
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }

        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if let Some(update) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &update);
        }

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        batch.spawns.extend(host_spawns);
        batch.landscape_ops.extend(host_landscape_ops);
        batch.particles.extend(host_particles);
        batch.transfer_zones.extend(host_transfer_zones);
        batch.messages.extend(host_messages);
        batch.commands.extend(object_commands);
        batch.command_ops.extend(command_operations);
        batch.effects.extend(host_object_effects);
        batch.global_effects.extend(host_global_effects);
        batch.destroy = destroy_object;
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
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
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(CommandBatch, AudioRegistry, LcgRng, u64), EngineError> {
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
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    state.action.phase,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    "Step",
                    &args,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        let (result, updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "Step",
            source,
        })?;
        let mut batch = parse_command(&self.id, "Step", result)?;
        batch.delta.local_vars = Some(updated_local_vars);
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            next_object_id,
        } = host_effects;
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }

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
        if !command_operations.is_empty() {
            batch.command_ops.extend(command_operations);
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
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
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
        if host_trigger_game_over {
            batch.trigger_game_over = true;
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
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        // Note: We don't validate function existence here because has_function() only
        // checks the current script's functions and doesn't traverse #include inheritance.
        // Scripts can inherit callbacks from parent scripts (e.g., TRPR inherits Throwing
        // from COWB). The VM will naturally handle truly missing functions when called.

        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::String(action_name.to_string()),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    state.action.phase,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    function,
                    &args,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: kind.context(),
            source,
        })?;

        // Action callbacks can return any value in C4Script.
        // The return value is typically used to indicate success/failure (e.g., return 1).
        // Unlike some other callback types, we don't validate or use the return value here.
        // This matches the C++ engine behavior where callbacks like Scaling() return int.
        drop(value);

        let mut host_effects = host_effects;
        // Store updated local variables so they persist
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
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
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Vec<ContextMenuEntry>, AudioRegistry, LcgRng), EngineError> {
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
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            state.action.phase,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_alive(state.alive)
        .with_base_graphics(state.base_graphics.clone())
        .with_ocf(self.compute_ocf(state));
        let (result, outcome) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    "MenuEntries",
                    &args,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, _updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "MenuEntries",
            source,
        })?;
        // MenuEntries shouldn't modify local vars, so we discard them
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
            || outcome.trigger_game_over
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
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
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
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            state.action.phase,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_alive(state.alive)
        .with_base_graphics(state.base_graphics.clone())
        .with_ocf(self.compute_ocf(state));
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    "MenuCommand",
                    &args,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "MenuCommand",
            source,
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
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

    fn call_control(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function(function) {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args: [Value; 0] = [];
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
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            state.action.phase,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_graphics_overlays(state.graphics_overlays.clone())
        .with_base_graphics(state.base_graphics.clone())
        .with_alive(state.alive)
        .with_ocf(self.compute_ocf(state));
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    function,
                    &args,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: self.id.clone(),
            function: "Control",
            source,
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
        let handled = match value {
            Value::Nil => false,
            Value::Bool(flag) => flag,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: self.id.clone(),
                    function: "Control",
                    detail: format!(
                        "control function `{function}` must return bool or nil (got {})",
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

    fn call_object_function(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        args: &[Value],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function(function) {
            let next_object_id = world.next_object_id();
            return Ok((
                Value::Nil,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let arg_values: Vec<Value> = args.to_vec();
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
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            state.action.phase,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_graphics_overlays(state.graphics_overlays.clone())
        .with_base_graphics(state.base_graphics.clone())
        .with_alive(state.alive)
        .with_ocf(self.compute_ocf(state));
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                self.script.call_with_locals_and_this(
                    function,
                    &arg_values,
                    &state.local_vars,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| {
            let function_label: &'static str = Box::leak(function.to_string().into_boxed_str());
            EngineError::Script {
                definition: self.id.clone(),
                function: function_label,
                source,
            }
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((value, host_effects, audio_state, rng))
    }

    fn call_menu_callback(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
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
        let local_vars_call = state.local_vars.clone();
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
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.ticks,
            state.action.data,
            state.action.phase,
            self.action_library.clone(),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_alive(state.alive)
        .with_base_graphics(state.base_graphics.clone())
        .with_ocf(self.compute_ocf(state));
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            move || {
                self.script.call_with_locals_and_this(
                    &function_call,
                    &args_call,
                    &local_vars_call,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| EngineError::Script {
            definition: format!("{}::{}", self.id, function),
            function: "MenuCallback",
            source,
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
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
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
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
            game_over_triggered,
            audio,
        )
    }

    fn call_effect_timer(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        frame: u64,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
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
            game_over_triggered,
            audio,
        )
    }

    fn call_effect_stop(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        effect: &EffectState,
        reason: EffectStopReason,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
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
            game_over_triggered,
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
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
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
        let (result, mut commands) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.ticks,
                    state.action.data,
                    state.action.phase,
                    self.action_library.clone(),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_alive(state.alive)
                .with_base_graphics(state.base_graphics.clone())
                .with_ocf(self.compute_ocf(state)),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
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

    #[allow(clippy::too_many_arguments)]
    fn initialize(
        &mut self,
        snapshot: &SimulationSnapshot,
        rng: LcgRng,
        random: i32,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        particle_defs: HashSet<String>,
    ) -> Result<(ScenarioBatch, AudioRegistry, LcgRng), EngineError> {
        if !self.has_initialize {
            return Ok((ScenarioBatch::default(), audio, rng));
        }
        let mut args = Vec::with_capacity(2);
        args.push(build_scenario_state_value(snapshot));
        args.push(Value::Int(random));
        self.call_raw(
            "Initialize",
            args,
            snapshot,
            rng,
            snapshot.frame,
            global_effects,
            physics,
            environment,
            audio,
            particle_defs,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        snapshot: &SimulationSnapshot,
        rng: LcgRng,
        random: i32,
        frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        particle_defs: HashSet<String>,
    ) -> Result<(ScenarioBatch, AudioRegistry, LcgRng), EngineError> {
        if !self.has_step {
            return Ok((ScenarioBatch::default(), audio, rng));
        }
        let mut args = Vec::with_capacity(3);
        args.push(build_scenario_state_value(snapshot));
        let truncated = if frame > i32::MAX as u64 {
            i32::MAX
        } else {
            frame as i32
        };
        args.push(Value::Int(truncated));
        args.push(Value::Int(random));
        self.call_raw(
            "Step",
            args,
            snapshot,
            rng,
            frame,
            global_effects,
            physics,
            environment,
            audio,
            particle_defs,
        )
    }

    fn has_function(&self, function: &str) -> bool {
        self.script.has_function(function)
    }

    #[allow(clippy::too_many_arguments)]
    fn call_raw(
        &mut self,
        function: &'static str,
        args: Vec<Value>,
        snapshot: &SimulationSnapshot,
        rng: LcgRng,
        env_frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        particle_defs: HashSet<String>,
    ) -> Result<(ScenarioBatch, AudioRegistry, LcgRng), EngineError> {
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, env_frame);
        let guard = enter_random_context(rng);
        let world =
            host_world_context_from_snapshot(snapshot).with_particle_defs(particle_defs);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            None,
            global_effects,
            world,
            next_object_id,
            snapshot.game_over,
            || self.script.call(function, &args),
        );
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
            command_operations,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            next_object_id: _,
        } = host_effects;

        if !host_object_effects.is_empty()
            || object_update.is_some()
            || !object_commands.is_empty()
            || !command_operations.is_empty()
            || destroy_object
        {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.name.clone(),
                function,
                detail: "scenario scripts may not enqueue object commands".into(),
            });
        }

        let mut batch = parse_scenario_command(&self.name, function, result)?;
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
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
        if !host_audio.events.is_empty() {
            batch.audio.extend(host_audio.events);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
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
    command_ops: Vec<CommandOperation>,
    effects: Vec<EffectCommand>,
    global_effects: Vec<EffectCommand>,
    environment: Option<EnvironmentDelta>,
    physics: Option<PhysicsDelta>,
    landscape_ops: Vec<LandscapeOperation>,
    particles: Vec<ParticleCommand>,
    transfer_zones: Vec<TransferZoneCommand>,
    audio: Vec<AudioCommand>,
    messages: Vec<MessageCommand>,
    player_commands: Vec<PlayerCommand>,
    trigger_game_over: bool,
}

#[derive(Debug, Default)]
struct ScenarioBatch {
    spawns: Vec<SpawnConfig>,
    global_effects: Vec<EffectCommand>,
    environment: Option<EnvironmentDelta>,
    physics: Option<PhysicsDelta>,
    landscape_ops: Vec<LandscapeOperation>,
    landscape: Vec<LandscapeCommand>,
    particles: Vec<ParticleCommand>,
    transfer_zones: Vec<TransferZoneCommand>,
    audio: Vec<AudioCommand>,
    messages: Vec<MessageCommand>,
    player_commands: Vec<PlayerCommand>,
    trigger_game_over: bool,
}

pub struct Engine {
    definitions: HashMap<DefinitionId, Definition>,
    materials: MaterialSet,
    objects: Vec<Object>,
    next_object_id: u64,
    rng: LcgRng,
    frame: u64,
    landscape: Option<Landscape>,
    sectors: Option<SectorMap>,
    physics: PhysicsSettings,
    environment: EnvironmentSettings,
    sky: Option<SkyState>,
    global_effects: Vec<EffectState>,
    particles: Vec<ActiveParticle>,
    /// C4ParticleSystem port (def-based particles, src/C4Particles.cpp). The
    /// `particles` Vec above only serves def-less legacy fixture particles.
    particle_system: particles::ParticleSystem,
    /// C4PXSSystem port (sync-relevant pixel sprites, src/C4PXS.cpp).
    pxs_system: pxs::PxsSystem,
    /// Control/sync-check state machine (C4GameControl): ControlTick advances
    /// every ControlRate frames; a sync check is digested every SyncRate
    /// frames (C4SyncCheckRate = 100) and kept for 50 frames.
    control_rate: i32,
    control_tick: i32,
    sync_rate: i32,
    do_sync: bool,
    sync_checks: Vec<SyncCheckPacket>,
    mass_movers: MassMoverSet,
    weather_events: Vec<WeatherEvent>,
    scenario_script: Option<ScenarioScript>,
    game_over_triggered: bool,
    objectives: ScenarioObjectives,
    objective_check_counter: u8,
    players_registered: bool,
    players: HashMap<i32, Player>,
    crew_selection: HashMap<i32, CrewSelection>,
    crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    team_home_base_rule: bool,
    construction_needs_material: bool,
    structures_need_energy: bool,
    base_buy_enabled: bool,
    base_sell_enabled: bool,
    landscape_insert_thrust: bool,
    known_crew_owners: HashSet<i32>,
    eliminated_crew_owners: HashSet<i32>,
    transfer_zones: TransferZoneTable,
    audio_registry: AudioRegistry,
    pending_audio: Vec<AudioCommand>,
    pending_menu_requests: Vec<MenuRequest>,
    messages: MessageManager,
}

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn clamp_fixed_to_limit(value: C4Fixed, limit: i32) -> C4Fixed {
    if limit <= 0 {
        C4Fixed::ZERO
    } else {
        value.clamp(itofix(-limit), itofix(limit))
    }
}

fn clamp_fixed_to_limit_pair(value: C4Fixed, min: C4Fixed, max: C4Fixed) -> C4Fixed {
    value.clamp(min, max)
}

fn saturating_i64_to_i32(value: i64) -> i32 {
    if value > i64::from(i32::MAX) {
        i32::MAX
    } else if value < i64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}

fn saturating_u64_to_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn step_fixed_toward(current: C4Fixed, desired: C4Fixed, step: C4Fixed) -> C4Fixed {
    if current == desired || step <= C4Fixed::ZERO {
        return desired;
    }
    let delta = i64::from(desired.val()) - i64::from(current.val());
    let step = i64::from(step.val());
    if delta.abs() <= step {
        desired
    } else {
        let next = if delta > 0 {
            i64::from(current.val()).saturating_add(step)
        } else {
            i64::from(current.val()).saturating_sub(step)
        };
        C4Fixed::from_raw(saturating_i64_to_i32(next))
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

fn control_function_name(command: ControlCommand, kind: CommandKind) -> Option<String> {
    let base = match command {
        ControlCommand::Throw => "Throw",
        ControlCommand::Dig => "Dig",
        ControlCommand::Special => "Special",
        ControlCommand::Special2 => "Special2",
        _ => return None,
    };

    let suffix = match kind {
        CommandKind::Press => "",
        CommandKind::Single => "Single",
        CommandKind::Double => "Double",
        CommandKind::Release => "Released",
    };

    let mut name = String::from("Control");
    name.push_str(base);
    name.push_str(suffix);
    Some(name)
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

fn apply_horizontal_friction_fixed(value: C4Fixed, friction: i32) -> C4Fixed {
    let raw = value.val();
    if raw == 0 || friction == 0 {
        return value;
    }
    let friction = friction.clamp(0, 100);
    if friction == 0 {
        return value;
    }
    let magnitude = i64::from(raw).abs();
    let mut retained = magnitude.saturating_mul(i64::from(100 - friction)) / 100;
    if retained == magnitude && friction > 0 {
        retained = magnitude.saturating_sub(1);
    }
    let signed = if retained == 0 {
        0
    } else if raw > 0 {
        retained
    } else {
        -retained
    };
    C4Fixed::from_raw(saturating_i64_to_i32(signed))
}

#[derive(Debug, Clone, Copy)]
struct LayerMovementBounds {
    position: Vector2,
    shape_rect: DefinitionRect,
    border_bound: i32,
}

#[derive(Debug, Clone)]
struct SolidMaskRect {
    object_id: ObjectId,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    pixels: Option<Vec<u8>>,
}

impl SolidMaskRect {
    fn contains(&self, x: i32, y: i32) -> bool {
        if self.width <= 0 || self.height <= 0 {
            return false;
        }
        let local_x = i64::from(x) - i64::from(self.x);
        let local_y = i64::from(y) - i64::from(self.y);
        local_x >= 0
            && local_y >= 0
            && local_x < i64::from(self.width)
            && local_y < i64::from(self.height)
            && self
                .pixels
                .as_ref()
                .map(|pixels| {
                    let index = local_y as usize * self.width as usize + local_x as usize;
                    pixels.get(index).copied().unwrap_or(0) != 0
                })
                .unwrap_or(true)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct MovementStepOutcome {
    no_attach: bool,
    any_contact: bool,
    solid_mask_removed: bool,
}

#[derive(Debug, Clone, Copy)]
struct MovementContactConfig<'a> {
    definition_vertices: &'a [ObjectVertex],
    contact_density: i32,
    contact_function_calls: bool,
    border_bound: i32,
    shape_rect: Option<DefinitionRect>,
    attach: u32,
    rotateable: i32,
    action_procedure: ActionProcedure,
    layer_bounds: Option<LayerMovementBounds>,
    solid_masks: &'a [SolidMaskRect],
    object_id: ObjectId,
}

#[derive(Debug, Clone, Copy)]
struct ContactVertexInfo {
    x: i32,
    contact_cnat: u32,
    friction: i32,
}

#[derive(Debug, Clone, Default)]
struct ShapeContact {
    contact_cnat: u32,
    vertices: Vec<ContactVertexInfo>,
}

impl ShapeContact {
    fn count(&self) -> i32 {
        self.vertices.len() as i32
    }

    fn is_contact(&self) -> bool {
        !self.vertices.is_empty()
    }

    fn has_vertex_cnat(&self, cnat: u32) -> bool {
        self.vertices
            .iter()
            .any(|vertex| vertex.contact_cnat & cnat != 0)
    }

    fn first_friction(&self) -> i32 {
        self.vertices
            .first()
            .map(|vertex| vertex.friction)
            .unwrap_or(0)
    }

    fn first_weight(&self) -> i32 {
        match self.vertices.first().map(|vertex| vertex.x).unwrap_or(0) {
            x if x < 0 => -1,
            x if x > 0 => 1,
            _ => 0,
        }
    }
}

fn sign_i32(value: i32) -> i32 {
    value.signum()
}

fn redirect_force(from: &mut C4Fixed, to: &mut C4Fixed, direction: i32) {
    let redirect = fixed100(50);
    let magnitude =
        C4Fixed::from_raw(saturating_i64_to_i32(i64::from(from.val()).abs())).min(redirect);
    if magnitude == C4Fixed::ZERO {
        return;
    }
    *from -= magnitude * from.val().signum();
    *to += magnitude * direction;
}

fn apply_contact_friction(value: &mut C4Fixed, percent: i32) {
    let friction = fixed100(30) * percent / 100;
    if *value > friction {
        *value -= friction;
    } else if *value < -friction {
        *value += friction;
    } else {
        *value = C4Fixed::ZERO;
    }
}

fn contact_callback_name(cnat: u32) -> Option<&'static str> {
    match cnat {
        CNAT_LEFT => Some("ContactLeft"),
        CNAT_RIGHT => Some("ContactRight"),
        CNAT_TOP => Some("ContactTop"),
        CNAT_BOTTOM => Some("ContactBottom"),
        _ => None,
    }
}

fn movement_hit_speed_flags(velocity: FixedVec2) -> u32 {
    let speed = i64::from(velocity.x.val()).abs() + i64::from(velocity.y.val()).abs();
    let mut flags = 0;
    if speed >= i64::from(fixed100(150).val()) {
        flags |= crate::ocf::HIT_SPEED1;
    }
    if speed >= i64::from(itofix(2).val()) {
        flags |= crate::ocf::HIT_SPEED2;
    }
    if speed >= i64::from(itofix(6).val()) {
        flags |= crate::ocf::HIT_SPEED3;
    }
    if speed >= i64::from(itofix(8).val()) {
        flags |= crate::ocf::HIT_SPEED4;
    }
    flags
}

fn construction_percent(construction: i32) -> i32 {
    construction.clamp(0, FULL_CON) * 100 / FULL_CON
}

fn construction_scaled_vertices(
    vertices: &[ObjectVertex],
    construction: i32,
    stretch_growth: bool,
) -> Vec<ObjectVertex> {
    let percent = construction_percent(construction);
    vertices
        .iter()
        .map(|vertex| {
            let mut scaled = *vertex;
            if stretch_growth {
                scaled.x = scaled.x * percent / 100;
            }
            scaled.y = scaled.y * percent / 100;
            scaled
        })
        .collect()
}

fn transformed_shape_vertices(
    vertices: &[ObjectVertex],
    construction: i32,
    stretch_growth: bool,
    rotateable: i32,
    rotation: i32,
) -> Vec<ObjectVertex> {
    let scaled = if construction.clamp(0, FULL_CON) == FULL_CON {
        vertices.to_vec()
    } else {
        construction_scaled_vertices(vertices, construction, stretch_growth)
    };
    if rotateable > 0 && rotation.rem_euclid(360) != 0 {
        rotated_vertices(&scaled, rotation)
    } else {
        scaled
    }
}

fn transformed_shape_rect(
    rect: Option<DefinitionRect>,
    construction: i32,
    stretch_growth: bool,
    rotateable: i32,
    rotation: i32,
) -> Option<DefinitionRect> {
    let mut rect = rect?;
    if construction.clamp(0, FULL_CON) != FULL_CON {
        let percent = construction_percent(construction);
        if stretch_growth {
            rect.x = rect.x * percent / 100;
            rect.width = rect.width * percent / 100;
        }
        rect.y = rect.y * percent / 100;
        rect.height = rect.height * percent / 100;
    }
    if rotateable > 0 && rotation.rem_euclid(360) != 0 {
        let radius = ((i64::from(rect.x) * i64::from(rect.x)
            + i64::from(rect.y) * i64::from(rect.y)) as f64)
            .sqrt() as i32
            + 2;
        rect.x = -radius;
        rect.y = -radius;
        rect.width = 2 * radius;
        rect.height = 2 * radius;
    }
    Some(rect)
}

fn rotated_vertices(vertices: &[ObjectVertex], rotation: i32) -> Vec<ObjectVertex> {
    if rotation.rem_euclid(360) == 0 {
        return vertices.to_vec();
    }
    let angle = itofix(rotation.rem_euclid(360));
    let cos = angle.cos_deg();
    let sin = angle.sin_deg();
    vertices
        .iter()
        .map(|vertex| {
            let x = fixtoi(cos * vertex.x - sin * vertex.y);
            let y = fixtoi(sin * vertex.x + cos * vertex.y);
            ObjectVertex {
                x,
                y,
                cnat: vertex.cnat,
                friction: vertex.friction,
            }
        })
        .collect()
}

fn movement_density_at(
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    excluded_solid_mask: Option<ObjectId>,
    x: i32,
    y: i32,
) -> i32 {
    if solid_masks
        .iter()
        .any(|mask| Some(mask.object_id) != excluded_solid_mask && mask.contains(x, y))
    {
        return C4M_VEHICLE;
    }
    landscape.density_at(x, y, materials)
}

fn shape_contact_check(
    vertices: &[ObjectVertex],
    position: Vector2,
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    excluded_solid_mask: Option<ObjectId>,
    contact_density: i32,
) -> ShapeContact {
    let mut contact = ShapeContact::default();
    for vertex in vertices {
        if vertex.cnat & CNAT_NO_COLLISION != 0 {
            continue;
        }
        let x = position.x + vertex.x;
        let y = position.y + vertex.y;
        if movement_density_at(landscape, materials, solid_masks, excluded_solid_mask, x, y)
            < contact_density
        {
            continue;
        }

        contact.contact_cnat |= vertex.cnat;
        let mut vertex_contact = CNAT_CENTER;
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x,
            y - 1,
        ) >= contact_density
        {
            vertex_contact |= CNAT_TOP;
        }
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x,
            y + 1,
        ) >= contact_density
        {
            vertex_contact |= CNAT_BOTTOM;
        }
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x - 1,
            y,
        ) >= contact_density
        {
            vertex_contact |= CNAT_LEFT;
        }
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x + 1,
            y,
        ) >= contact_density
        {
            vertex_contact |= CNAT_RIGHT;
        }
        contact.vertices.push(ContactVertexInfo {
            x: vertex.x,
            contact_cnat: vertex_contact,
            friction: vertex.friction,
        });
    }
    contact
}

fn attach_direction(attach: u32) -> (i32, i32) {
    match attach & !CNAT_FLAGS {
        CNAT_TOP => (0, -1),
        CNAT_BOTTOM => (0, 1),
        CNAT_LEFT => (-1, 0),
        CNAT_RIGHT => (1, 0),
        _ => (0, 0),
    }
}

fn shape_attach(
    vertices: &[ObjectVertex],
    position: &mut Vector2,
    attach: u32,
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    excluded_solid_mask: Option<ObjectId>,
    contact_density: i32,
) -> bool {
    let (xcd, ycd) = attach_direction(attach);
    if xcd == 0 && ycd == 0 {
        return false;
    }
    let xcrng = -(ATTACH_RANGE * xcd);
    let ycrng = -(ATTACH_RANGE * ycd);
    let mut attached = false;

    if attach & CNAT_MULTI_ATTACH == 0 {
        for vertex in vertices {
            if vertex.cnat & attach == 0 {
                continue;
            }
            let mut xcnt = xcrng;
            let mut ycnt = ycrng;
            while xcnt != -xcrng || ycnt != -ycrng {
                let ax = position.x + vertex.x + xcnt + xcd;
                let ay = position.y + vertex.y + ycnt + ycd;
                if ax >= 0
                    && ax < landscape.width() as i32
                    && movement_density_at(
                        landscape,
                        materials,
                        solid_masks,
                        excluded_solid_mask,
                        ax,
                        ay,
                    ) >= contact_density
                {
                    position.x += xcnt;
                    position.y += ycnt;
                    attached = true;
                    break;
                }
                xcnt += xcd;
                ycnt += ycd;
            }
        }
    } else {
        let mut xcnt = xcrng;
        let mut ycnt = ycrng;
        'search: while xcnt != -xcrng || ycnt != -ycrng {
            for vertex in vertices {
                if vertex.cnat & attach == 0 {
                    continue;
                }
                let ax = position.x + vertex.x + xcnt + xcd;
                let ay = position.y + vertex.y + ycnt + ycd;
                if ax >= 0
                    && ax < landscape.width() as i32
                    && landscape.density_at(ax, ay, materials) >= contact_density
                {
                    position.x += xcnt;
                    position.y += ycnt;
                    attached = true;
                    break 'search;
                }
            }
            xcnt += xcd;
            ycnt += ycd;
        }
    }

    attached
}

fn target_bounds(target: &mut C4Fixed, low: i32, high: i32) -> Option<i32> {
    let low = itofix(low);
    let high = itofix(high);
    if *target < low {
        *target = low;
        Some(-1)
    } else if *target > high {
        *target = high;
        Some(1)
    } else {
        None
    }
}

fn apply_float_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
) {
    let (dx, dy) = command_direction.axis_components();
    let accel = itofix(profile.float_acceleration.max(0));

    if dx != 0 && accel > C4Fixed::ZERO {
        velocity.x = clamp_fixed_to_limit(velocity.x + accel * dx, profile.float_speed);
    } else {
        velocity.x = clamp_fixed_to_limit(velocity.x, profile.float_speed);
    }

    if dy != 0 && accel > C4Fixed::ZERO {
        velocity.y = clamp_fixed_to_limit(velocity.y + accel * dy, profile.float_speed);
    } else {
        velocity.y = clamp_fixed_to_limit(velocity.y, profile.float_speed);
    }
}

fn decelerate_fixed_toward_zero(value: C4Fixed, accel: C4Fixed) -> C4Fixed {
    if accel <= C4Fixed::ZERO {
        return value;
    }
    if value > C4Fixed::ZERO {
        (value - accel).max(C4Fixed::ZERO)
    } else if value < C4Fixed::ZERO {
        (value + accel).min(C4Fixed::ZERO)
    } else {
        C4Fixed::ZERO
    }
}

fn apply_walk_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
) {
    let accel = itofix(profile.walk_acceleration.max(0));
    let limit = profile.walk_speed;

    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            if accel > C4Fixed::ZERO {
                velocity.x -= accel;
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            if accel > C4Fixed::ZERO {
                velocity.x += accel;
            }
        }
        CommandDirection::Stop | CommandDirection::Up | CommandDirection::Down => {
            if accel > C4Fixed::ZERO {
                velocity.x = decelerate_fixed_toward_zero(velocity.x, accel);
            }
        }
    }

    velocity.x = clamp_fixed_to_limit(velocity.x, limit);
}

fn apply_swim_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    gravity_component: C4Fixed,
) {
    let accel = itofix(profile.swim_acceleration.max(0));
    let limit = profile.swim_speed;

    match command_direction {
        CommandDirection::Stop => {
            if accel > C4Fixed::ZERO {
                velocity.x = decelerate_fixed_toward_zero(velocity.x, accel);
                let vertical_without_gravity = velocity.y - gravity_component;
                let decelerated = decelerate_fixed_toward_zero(vertical_without_gravity, accel);
                velocity.y = decelerated + gravity_component;
            }
        }
        _ => {
            if accel > C4Fixed::ZERO {
                let (dx, dy) = command_direction.axis_components();
                if dx != 0 {
                    velocity.x += accel * dx;
                }
                if dy != 0 {
                    velocity.y += accel * dy;
                }
            }
        }
    }

    velocity.x = clamp_fixed_to_limit(velocity.x, limit);
    velocity.y = clamp_fixed_to_limit(velocity.y, limit);
}

fn apply_scale_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) {
    let accel = itofix(profile.scale_acceleration.max(0));
    let limit = profile.scale_speed;
    let effective_direction = match (facing, command_direction) {
        (Direction::Left, CommandDirection::Left) | (Direction::Right, CommandDirection::Right) => {
            CommandDirection::Up
        }
        _ => command_direction,
    };

    match effective_direction {
        CommandDirection::Up | CommandDirection::UpLeft | CommandDirection::UpRight => {
            if accel > C4Fixed::ZERO {
                velocity.y -= accel;
            }
        }
        CommandDirection::Down | CommandDirection::DownLeft | CommandDirection::DownRight => {
            if accel > C4Fixed::ZERO {
                velocity.y += accel;
            }
        }
        CommandDirection::Left | CommandDirection::Right | CommandDirection::Stop => {
            if accel > C4Fixed::ZERO {
                velocity.y = decelerate_fixed_toward_zero(velocity.y, accel);
            }
        }
    }

    velocity.y = clamp_fixed_to_limit(velocity.y, limit);
    velocity.x = C4Fixed::ZERO;
}

fn apply_hangle_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) -> Option<Direction> {
    let accel = itofix(profile.hangle_acceleration.max(0));
    let limit = profile.hangle_speed;

    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            if accel > C4Fixed::ZERO {
                velocity.x -= accel;
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            if accel > C4Fixed::ZERO {
                velocity.x += accel;
            }
        }
        CommandDirection::Up => {
            if accel > C4Fixed::ZERO {
                if matches!(facing, Direction::Left) {
                    velocity.x -= accel;
                } else {
                    velocity.x += accel;
                }
            }
        }
        CommandDirection::Stop | CommandDirection::Down => {
            if accel > C4Fixed::ZERO {
                velocity.x = decelerate_fixed_toward_zero(velocity.x, accel);
            }
        }
    }

    velocity.x = clamp_fixed_to_limit(velocity.x, limit);
    velocity.y = C4Fixed::ZERO;

    if velocity.x < C4Fixed::ZERO {
        Some(Direction::Left)
    } else if velocity.x > C4Fixed::ZERO {
        Some(Direction::Right)
    } else {
        None
    }
}

fn apply_dig_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) -> Option<Direction> {
    let speed = profile.dig_speed.max(0);
    let half_speed = speed / 2;
    let speed = itofix(speed);
    let half_speed = itofix(half_speed);

    match command_direction {
        CommandDirection::Stop => {
            velocity.x = C4Fixed::ZERO;
            velocity.y = C4Fixed::ZERO;
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
            velocity.y = C4Fixed::ZERO;
        }
        CommandDirection::DownLeft => {
            velocity.x = -speed;
            velocity.y = speed;
        }
        CommandDirection::Down => {
            velocity.x = C4Fixed::ZERO;
            velocity.y = speed;
        }
        CommandDirection::DownRight => {
            velocity.x = speed;
            velocity.y = speed;
        }
        CommandDirection::Right => {
            velocity.x = speed;
            velocity.y = C4Fixed::ZERO;
        }
        CommandDirection::UpRight => {
            velocity.x = speed;
            velocity.y = -half_speed;
        }
    }

    if velocity.x < C4Fixed::ZERO {
        Some(Direction::Left)
    } else if velocity.x > C4Fixed::ZERO {
        Some(Direction::Right)
    } else {
        None
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
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
            rng: LcgRng::seed_from_u64(seed),
            frame: 0,
            landscape: None,
            sectors: None,
            physics: PhysicsSettings::default(),
            environment: EnvironmentSettings::default(),
            sky: None,
            global_effects: Vec::new(),
            particles: Vec::new(),
            particle_system: particles::ParticleSystem::default(),
            pxs_system: pxs::PxsSystem::default(),
            control_rate: 1,
            control_tick: 0,
            sync_rate: 100,
            do_sync: false,
            sync_checks: Vec::new(),
            mass_movers: MassMoverSet::new(),
            weather_events: Vec::new(),
            scenario_script: None,
            game_over_triggered: false,
            objectives: ScenarioObjectives::default(),
            objective_check_counter: 0,
            players_registered: false,
            players: HashMap::new(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            team_home_base_rule: false,
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            landscape_insert_thrust: false,
            known_crew_owners: HashSet::new(),
            eliminated_crew_owners: HashSet::new(),
            transfer_zones: TransferZoneTable::default(),
            audio_registry: AudioRegistry::new(),
            pending_audio: Vec::new(),
            pending_menu_requests: Vec::new(),
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

    pub fn set_construction_needs_material(&mut self, enabled: bool) {
        self.construction_needs_material = enabled;
    }

    pub fn structures_need_energy(&self) -> bool {
        self.structures_need_energy
    }

    pub fn set_structures_need_energy(&mut self, enabled: bool) {
        self.structures_need_energy = enabled;
    }

    pub fn set_base_buy_enabled(&mut self, enabled: bool) {
        self.base_buy_enabled = enabled;
    }

    pub fn set_base_sell_enabled(&mut self, enabled: bool) {
        self.base_sell_enabled = enabled;
    }

    pub fn set_landscape_insert_thrust(&mut self, enabled: bool) {
        self.landscape_insert_thrust = enabled;
        self.mass_movers.set_landscape_insert_thrust(enabled);
    }

    pub fn register_player(&mut self, config: PlayerConfig) -> Result<(), EngineError> {
        let id = config.id();
        if self.players.contains_key(&id) {
            return Err(EngineError::PlayerAlreadyExists(id));
        }
        let player = config.build();
        self.players.insert(id, player);
        self.players_registered = true;
        self.sync_player_cursor(id);
        self.sync_team_home_base_for(id);

        if let Err(error) =
            self.broadcast_scenario_function("PreInitializePlayer", vec![Value::Int(id)])
        {
            self.players.remove(&id);
            return Err(error);
        }

        let position = self
            .objects
            .iter()
            .filter(|object| object.state.owner == id && object.state.crew_member)
            .min_by_key(|object| object.id.as_u64())
            .map(|object| object.state.position);
        let (x_value, y_value) = match position {
            Some(pos) => (Value::Int(pos.x), Value::Int(pos.y)),
            None => (Value::Nil, Value::Nil),
        };
        let base_value = self
            .objects
            .iter()
            .filter(|object| {
                object.state.owner == id
                    && (object.state.category & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK)) != 0
            })
            .min_by_key(|object| object.id.as_u64())
            .map(|object| object_reference_value(object.id))
            .unwrap_or(Value::Nil);
        let team_value = self
            .players
            .get(&id)
            .and_then(|player| player.team())
            .map(Value::Int)
            .unwrap_or(Value::Nil);
        let mut init_args = Vec::with_capacity(6);
        init_args.push(Value::Int(id));
        init_args.push(x_value);
        init_args.push(y_value);
        init_args.push(base_value);
        init_args.push(team_value);
        init_args.push(Value::Nil);

        if let Err(error) = self.broadcast_scenario_function("InitializePlayer", init_args) {
            self.players.remove(&id);
            return Err(error);
        }

        self.refresh_elimination_state();
        self.check_game_over()?;
        Ok(())
    }

    /// Declare or revoke hostility between two players
    /// (C4Player::Hostility; queried by `C4PlayerList::Hostile`).
    pub fn set_hostility(
        &mut self,
        player: i32,
        opponent: i32,
        hostile: bool,
    ) -> Result<(), EngineError> {
        let plr = self
            .players
            .get_mut(&player)
            .ok_or(EngineError::UnknownPlayer(player))?;
        plr.set_hostile_towards(opponent, hostile);
        Ok(())
    }

    pub fn remove_player(&mut self, id: i32) -> Result<Player, EngineError> {
        let team = match self.players.get(&id) {
            Some(player) => player.team(),
            None => return Err(EngineError::UnknownPlayer(id)),
        };
        let mut args = Vec::with_capacity(2);
        args.push(Value::Int(id));
        args.push(team.map(Value::Int).unwrap_or(Value::Nil));
        self.broadcast_scenario_function("RemovePlayer", args)?;

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
        self.check_game_over()?;
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
        let count = {
            let player = self.player_mut(id)?;
            player.adjust_home_base_material(definition_id, delta)
        };
        self.sync_team_home_base_for(id);
        Ok(count)
    }

    pub fn adjust_player_home_base_production(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        delta: i32,
    ) -> Result<u32, EngineError> {
        let player = self.player_mut(id)?;
        Ok(player.adjust_home_base_production(definition_id, delta))
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

    pub fn configure_objectives(&mut self, objectives: ScenarioObjectives) {
        self.objectives = objectives;
        self.objective_check_counter = GAME_OVER_CHECK_INTERVAL.saturating_sub(1);
    }

    pub fn set_landscape(&mut self, mut landscape: Landscape) {
        let default = self.materials.default_ground_material();
        if default.is_some() {
            landscape.set_default_solid_material(default);
        }
        self.landscape = Some(landscape);
        self.reset_sectors_from_landscape();
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
        self.sectors = None;
        self.pxs_system.clear();
    }

    pub fn landscape(&self) -> Option<&Landscape> {
        self.landscape.as_ref()
    }

    fn reset_sectors_from_landscape(&mut self) {
        let Some(landscape) = self.landscape.as_ref() else {
            self.sectors = None;
            return;
        };
        let width = saturating_u64_to_i32(u64::from(landscape.width()));
        let height = landscape.estimated_height();
        self.sectors = Some(SectorMap::new(width, height));
        self.rebuild_sectors();
    }

    fn rebuild_sectors(&mut self) {
        let records = self
            .objects
            .iter()
            .filter_map(|object| self.sector_record_for_object(object))
            .collect::<Vec<_>>();
        if let Some(sectors) = self.sectors.as_mut() {
            sectors.rebuild(records);
        }
    }

    fn update_sector_for_index(&mut self, index: usize) {
        let Some(object) = self.objects.get(index) else {
            return;
        };
        let object_id = object.id;
        let record = self.sector_record_for_object(object);
        let Some(sectors) = self.sectors.as_mut() else {
            return;
        };
        match record {
            Some(record) => sectors.update(record),
            None => sectors.remove(object_id),
        }
    }

    fn sector_record_for_object(&self, object: &Object) -> Option<SectorObject> {
        if object.destroyed || matches!(object.state.status, ObjectStatus::Deleted) {
            return None;
        }
        let position = object.state.position;
        let shape_rect = self.object_shape_rect(object);
        Some(SectorObject {
            id: object.id,
            position,
            shape_rect,
        })
    }

    fn object_shape_rect(&self, object: &Object) -> DefinitionRect {
        let position = object.state.position;
        object
            .current_shape_rect()
            .map(|rect| {
                DefinitionRect::new(
                    position.x.saturating_add(rect.x),
                    position.y.saturating_add(rect.y),
                    rect.width,
                    rect.height,
                )
            })
            .or_else(|| vertex_bounds_rect(position, &object.state.vertices))
            .unwrap_or_else(|| DefinitionRect::new(position.x, position.y, 1, 1))
    }

    #[allow(dead_code)]
    fn at_object(
        &self,
        point: Vector2,
        mask: u32,
        exclude: Option<ObjectId>,
    ) -> Option<(usize, ObjectId, u32)> {
        let candidate_ids = self
            .sectors
            .as_ref()
            .map(|sectors| sectors.object_ids_at(point.x, point.y).to_vec())
            .unwrap_or_else(|| self.objects.iter().map(|object| object.id).collect());
        for candidate_id in candidate_ids {
            if exclude == Some(candidate_id) {
                continue;
            }
            let Some(candidate_idx) = self.find_object_index(candidate_id) else {
                continue;
            };
            let candidate = &self.objects[candidate_idx];
            if candidate.destroyed
                || !candidate.state.status.is_active()
                || candidate.state.container.is_some()
            {
                continue;
            }
            let candidate_ocf = self.object_ocf_at_index(candidate_idx);
            if candidate_ocf & (mask | crate::ocf::EXCLUSIVE) == 0 {
                continue;
            }
            if !self
                .object_shape_rect(candidate)
                .contains_point(point.x, point.y)
            {
                continue;
            }
            if candidate_ocf & mask != 0 {
                return Some((candidate_idx, candidate_id, candidate_ocf));
            }
            return None;
        }
        None
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

    /// Register a particle definition, mirroring `C4ParticleDef::Load`
    /// (C4Particles.cpp:118-192). `gfx_length` is the number of animation
    /// phases in the graphics, `aspect` the phase height:width ratio — both
    /// derived from Graphics.png at load time in C++.
    pub fn register_particle_definition(
        &mut self,
        core: particles::ParticleDefCore,
        gfx_length: i32,
        aspect: f32,
    ) -> Result<(), particles::ParticleDefError> {
        self.particle_system.register_def(core, gfx_length, aspect)
    }

    pub fn particle_system(&self) -> &particles::ParticleSystem {
        &self.particle_system
    }

    pub fn set_physics(&mut self, physics: PhysicsSettings) {
        self.physics = physics;
        for object in &mut self.objects {
            object.clamp_velocity(&self.physics);
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
                        constructable: definition.is_constructable(),
                        shape: definition.shape_rect(),
                        construction_offset: definition.construction_offset(),
                        basement: definition.basement(),
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
        let crew_selection: HashMap<i32, CrewSelectionState> = self
            .crew_selection
            .iter()
            .map(|(&owner, selection)| (owner, CrewSelectionState::from(selection)))
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
                            object.state.construction,
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
                    object.state.construction,
                    object.state.damage,
                    object.state.position,
                    object.state.velocity,
                    object.state.rotation,
                    object.state.vertices.clone(),
                    object.state.action.data,
                    object.state.action.ticks,
                    object.state.action.phase,
                    object.state.container,
                    object.state.draw_transform,
                )
                .with_contents(object.state.contents.clone())
                .with_alive(object.state.alive)
                .with_ocf(ocf)
            }),
            landscape,
            definition_metadata,
            transfer_zones,
            players,
            crew_selection,
            self.next_object_id,
            self.team_home_base_rule,
        )
        .with_particle_defs(self.particle_system.def_names())
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
            self.particle_system.def_names(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        let created = self.apply_scenario_batch(batch)?;
        self.game_over_triggered = false;
        self.scenario_script = Some(script);
        Ok(created)
    }

    fn broadcast_scenario_function(
        &mut self,
        function: &'static str,
        mut extra_args: Vec<Value>,
    ) -> Result<(), EngineError> {
        if self.scenario_script.is_none() {
            return Ok(());
        }
        let snapshot = self.snapshot();
        let mut args = Vec::with_capacity(extra_args.len() + 1);
        args.push(build_scenario_state_value(&snapshot));
        args.append(&mut extra_args);
        let rng_state = self.rng.clone();
        let env_frame = self.frame;
        let global_effects = self.global_effects.clone();
        let physics = self.physics;
        let environment = self.environment;
        let audio_state = self.audio_registry.clone();
        let particle_defs = self.particle_system.def_names();
        let script = match self.scenario_script.as_mut() {
            Some(script) if script.has_function(function) => script,
            Some(_) => return Ok(()),
            None => unreachable!("scenario script must be present"),
        };
        let (batch, audio_state, new_rng) = script.call_raw(
            function,
            args,
            &snapshot,
            rng_state,
            env_frame,
            &global_effects,
            physics,
            environment,
            audio_state,
            particle_defs,
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        let _ = self.apply_scenario_batch(batch)?;
        Ok(())
    }

    fn check_game_over(&mut self) -> Result<(), EngineError> {
        if self.game_over_triggered || !self.players_registered {
            return Ok(());
        }

        let mut should_trigger = !self.has_active_players();

        if !should_trigger && self.should_evaluate_objectives() && self.objectives_met() {
            should_trigger = true;
        }

        if should_trigger {
            self.request_game_over()?;
        }

        Ok(())
    }

    fn has_active_players(&self) -> bool {
        self.players
            .values()
            .any(|player| matches!(player.status(), PlayerStatus::Active) && !player.surrendered())
    }

    fn should_evaluate_objectives(&self) -> bool {
        !self.objectives.is_empty() && self.objective_check_counter == 0
    }

    fn objectives_met(&self) -> bool {
        if self.objectives.is_empty() {
            return false;
        }

        let mut game_over_valid = false;
        let mut game_over = true;

        if !self.objectives.create_objects.is_empty() {
            let mut condition_valid = false;
            let mut condition_true = true;
            for objective in &self.objectives.create_objects {
                if objective.count <= 0 {
                    continue;
                }
                condition_valid = true;
                let target_id = objective.definition.as_str();
                let current = self
                    .objects
                    .iter()
                    .filter(|object| object.definition_id.as_str() == target_id)
                    .filter(|object| object.state.status.is_active())
                    .filter(|object| object.state.construction >= FULL_CON)
                    .count() as i32;
                if current < objective.count {
                    condition_true = false;
                }
            }
            if condition_valid {
                game_over_valid = true;
                if !condition_true {
                    game_over = false;
                }
            }
        }

        if !self.objectives.clear_objects.is_empty() {
            let mut condition_valid = false;
            let mut condition_true = true;
            for objective in &self.objectives.clear_objects {
                condition_valid = true;
                let limit = objective.count.max(0);
                let target_id = objective.definition.as_str();
                let alive_only = self
                    .definitions
                    .get(target_id)
                    .map(|definition| definition.category() & CATEGORY_LIVING != 0)
                    .unwrap_or(false);
                let count = self
                    .objects
                    .iter()
                    .filter(|object| object.definition_id.as_str() == target_id)
                    .filter(|object| object.state.status.is_active())
                    .filter(|object| !alive_only || object.state.alive)
                    .count() as i32;
                if count > limit {
                    condition_true = false;
                }
            }
            if condition_valid {
                game_over_valid = true;
                if !condition_true {
                    game_over = false;
                }
            }
        }

        if !self.objectives.clear_materials.is_empty() {
            let mut condition_valid = false;
            let mut condition_true = true;
            if let Some(landscape) = self.landscape.as_ref() {
                for objective in &self.objectives.clear_materials {
                    if let Some(material_id) = self.materials.id_of(&objective.material) {
                        condition_valid = true;
                        let limit = i64::from(objective.count.max(0));
                        let total = self.count_material_pixels(landscape, material_id);
                        if total > limit {
                            condition_true = false;
                        }
                    }
                }
            }
            if condition_valid {
                game_over_valid = true;
                if !condition_true {
                    game_over = false;
                }
            }
        }

        game_over_valid && game_over
    }

    fn count_material_pixels(&self, landscape: &Landscape, material_id: MaterialId) -> i64 {
        let mut total: i64 = 0;

        for x in 0..landscape.width() {
            if landscape.solid_material_at(x as i32) == Some(material_id) {
                let height = landscape
                    .surface()
                    .get(x as usize)
                    .copied()
                    .unwrap_or_default()
                    .max(0);
                total += i64::from(height);
            }
        }

        for column in landscape.liquids() {
            for segment in column.segments() {
                if segment.material == Some(material_id) {
                    let span = i64::from(segment.bottom) - i64::from(segment.top);
                    if span > 0 {
                        total += span;
                    }
                }
            }
        }

        total
    }

    fn request_game_over(&mut self) -> Result<bool, EngineError> {
        if self.game_over_triggered {
            return Ok(false);
        }
        self.game_over_triggered = true;
        self.broadcast_scenario_function("OnGameOver", Vec::new())?;
        Ok(true)
    }

    fn apply_scenario_batch(&mut self, batch: ScenarioBatch) -> Result<Vec<ObjectId>, EngineError> {
        let ScenarioBatch {
            spawns,
            global_effects,
            environment,
            physics,
            landscape_ops,
            landscape,
            particles,
            transfer_zones,
            audio,
            messages,
            player_commands,
            trigger_game_over,
        } = batch;

        if !player_commands.is_empty() {
            self.apply_player_commands(player_commands)?;
        }

        if !landscape_ops.is_empty() {
            self.apply_landscape_operations(landscape_ops);
        }

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
            if let Some(landscape_ref) = self.landscape.as_ref() {
                self.mass_movers
                    .seed_from_landscape(landscape_ref, &self.materials);
            }
            if let Some(landscape_mut) = self.landscape.as_mut() {
                landscape_mut.take_mass_mover_dirty();
            }
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

        // Pre-scan spawns to find maximum explicit ID and reserve ID space
        // This prevents conflicts between auto-assigned IDs (from earlier objects like crew)
        // and explicit IDs (from scenario Objects.txt)
        let max_explicit_id = spawns
            .iter()
            .filter_map(|spawn| spawn.id)
            .map(|id| id.as_u64())
            .max();

        if let Some(max_id) = max_explicit_id {
            // Reserve ID space: ensure next_object_id is beyond all explicit IDs
            if max_id >= self.next_object_id {
                self.next_object_id = max_id + 1;
            }
        }

        let mut created = Vec::with_capacity(spawns.len());
        for spawn in spawns {
            let id = self.spawn_object(spawn)?;
            created.push(id);
        }
        if trigger_game_over {
            self.request_game_over()?;
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

    pub fn ensure_cursor(&mut self, owner: i32) -> Result<(), EngineError> {
        if self.crew_cursor(owner).is_some() {
            return Ok(());
        }
        let mut crew = self.crew_members(owner);
        if crew.is_empty() {
            return Ok(());
        }
        crew.sort_by_key(|id| id.as_u64());
        let first = crew[0];
        self.set_crew_cursor(owner, Some(first))
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
            self.game_over_triggered,
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
            self.game_over_triggered,
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
            self.game_over_triggered,
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

    fn call_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: Vec<Value>,
    ) -> Result<Value, EngineError> {
        let (object_id, definition_id, state_snapshot) = {
            let object = self
                .objects
                .get(index)
                .ok_or_else(|| EngineError::UnknownObject(ObjectId::new(u64::MAX)))?;
            (
                object.id,
                object.definition_id.clone(),
                object.state.clone(),
            )
        };
        let definition = self
            .definitions
            .get(&definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
        let action_library = definition.action_library().clone();
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world = self.host_world_context();
        let (value, outcome, audio_state, new_rng) = definition.call_object_function(
            &state_snapshot,
            object_id,
            function,
            &args,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
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
        Ok(value)
    }

    fn call_movement_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: &[Value],
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
    ) -> Result<Value, EngineError> {
        let state_snapshot = self
            .objects
            .get(index)
            .ok_or_else(|| EngineError::UnknownObject(ObjectId::new(u64::MAX)))?
            .state
            .clone();
        let definition = self
            .definitions
            .get(definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.to_string()))?;
        let rng_state = self.rng.clone();
        let global_view = self.global_effects.clone();
        let world = self.host_world_context();
        let (value, outcome, audio_state, new_rng) = definition.call_object_function(
            &state_snapshot,
            object_id,
            function,
            args,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_callback_outcome(
            index,
            outcome,
            action_library,
            object_id,
            definition_id,
            false,
        )?;
        Ok(value)
    }

    fn invoke_movement_hit_callbacks(
        &mut self,
        index: usize,
        old_velocity: FixedVec2,
        hit_speed_flags: u32,
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
    ) -> Result<(), EngineError> {
        let args = [
            Value::Int(fixtoi_prec(old_velocity.x, 100)),
            Value::Int(fixtoi_prec(old_velocity.y, 100)),
        ];
        for (flag, function) in [
            (crate::ocf::HIT_SPEED1, "Hit"),
            (crate::ocf::HIT_SPEED2, "Hit2"),
            (crate::ocf::HIT_SPEED3, "Hit3"),
        ] {
            if hit_speed_flags & flag == 0 {
                continue;
            }
            self.call_movement_object_function(
                index,
                function,
                &args,
                action_library,
                object_id,
                definition_id,
            )?;
        }
        Ok(())
    }

    pub fn handle_control_command(
        &mut self,
        owner: i32,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Result<bool, EngineError> {
        let Some(function_name) = control_function_name(command, kind) else {
            return Ok(false);
        };

        self.ensure_cursor(owner)?;
        let Some(cursor) = self.crew_cursor(owner) else {
            return Ok(false);
        };

        let index = self
            .objects
            .iter()
            .position(|object| object.id == cursor)
            .ok_or(EngineError::UnknownObject(cursor))?;
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
        let (handled, outcome, audio_state, new_rng) = definition.call_control(
            &state_snapshot,
            cursor,
            &function_name,
            rng_state,
            &global_view,
            self.physics,
            self.environment,
            self.frame,
            world,
            self.game_over_triggered,
            self.audio_registry.clone(),
        )?;
        self.rng = new_rng;
        self.audio_registry = audio_state;
        self.apply_action_callback_outcome(
            index,
            outcome,
            &action_library,
            cursor,
            &definition_id,
        )?;
        Ok(handled)
    }

    pub fn try_grab_nearby(&mut self, owner: i32) -> Result<bool, EngineError> {
        self.ensure_cursor(owner)?;
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(false);
        };
        let crew_index = match self.find_object_index(crew_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(crew_id)),
        };
        if !self.objects[crew_index].state.status.is_active() {
            return Ok(false);
        }
        let crew_position = self.objects[crew_index].state.position;
        let Some((_, target_id)) =
            self.find_nearby_object_with_mask(crew_id, crew_position, ocf::GRAB, 22, |object| {
                object.state.container.is_none() && object.state.status.is_active()
            })
        else {
            return Ok(false);
        };
        let update = ObjectUpdate::new()
            .with_container(crew_id)
            .with_position(crew_position)
            .with_velocity(Vector2::ZERO);
        self.apply_object_update(target_id, update)?;
        Ok(true)
    }

    pub fn try_drop_held_object(&mut self, owner: i32) -> Result<bool, EngineError> {
        self.ensure_cursor(owner)?;
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(false);
        };
        let crew_index = match self.find_object_index(crew_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(crew_id)),
        };
        let crew_state = &self.objects[crew_index].state;
        if !crew_state.status.is_active() {
            return Ok(false);
        }
        let Some(item_id) = crew_state.contents.first().copied() else {
            return Ok(false);
        };
        let offset = match crew_state.direction {
            Direction::Left => -8,
            Direction::Right => 8,
        };
        let drop_position = Vector2::new(crew_state.position.x + offset, crew_state.position.y);
        let update = ObjectUpdate::new()
            .clear_container()
            .with_position(drop_position)
            .with_velocity(Vector2::ZERO);
        self.apply_object_update(item_id, update)?;
        Ok(true)
    }

    pub fn try_enter_nearby(&mut self, owner: i32) -> Result<bool, EngineError> {
        self.ensure_cursor(owner)?;
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(false);
        };
        let crew_index = match self.find_object_index(crew_id) {
            Some(index) => index,
            None => return Err(EngineError::UnknownObject(crew_id)),
        };
        let crew_state = &self.objects[crew_index].state;
        if crew_state.container.is_some() || !crew_state.status.is_active() {
            return Ok(false);
        }
        let crew_position = crew_state.position;
        let Some((target_index, target_id)) = self.find_nearby_object_with_mask(
            crew_id,
            crew_position,
            ocf::ENTRANCE,
            24,
            |object| object.state.status.is_active(),
        ) else {
            return Ok(false);
        };
        let target_position = self.objects[target_index].state.position;
        let update = ObjectUpdate::new()
            .with_container(target_id)
            .with_position(target_position)
            .with_velocity(Vector2::ZERO);
        self.apply_object_update(crew_id, update)?;
        Ok(true)
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

    pub fn resolve_includes(&mut self) -> Result<(), EngineError> {
        // Iteratively merge includes until no more changes occur
        // This ensures transitive dependencies are fully resolved
        // (e.g., TRE2 -> TRE1 -> TREE means TRE2 gets functions from TREE)
        let mut changed = true;

        while changed {
            changed = false;

            // Collect all definitions that have includes
            let ids_with_includes: Vec<String> = self
                .definitions
                .iter()
                .filter(|(_, def)| !def.includes().is_empty())
                .map(|(id, _)| id.clone())
                .collect();

            // For each definition with includes, merge parent functions
            for child_id in ids_with_includes {
                let includes = self
                    .definitions
                    .get(&child_id)
                    .map(|def| def.includes().to_vec())
                    .unwrap_or_default();

                for parent_id in &includes {
                    // Check if parent exists
                    if !self.definitions.contains_key(parent_id) {
                        return Err(EngineError::UnknownDefinition(parent_id.clone()));
                    }

                    // Clone the parent to avoid borrow checker issues
                    let parent = self.definitions.get(parent_id).unwrap().clone();

                    // Count functions before merge to detect changes
                    let before_count = self
                        .definitions
                        .get(&child_id)
                        .map(|def| def.function_count())
                        .unwrap_or(0);

                    // Merge parent into child
                    if let Some(child) = self.definitions.get_mut(&child_id) {
                        child.merge_from(&parent);

                        // Check if we added any functions
                        let after_count = child.function_count();
                        if after_count > before_count {
                            changed = true;
                        }
                    }
                }
            }
        }

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

    pub fn definition_sprite_image(
        &self,
        definition_id: &str,
        graphics_name: Option<&str>,
    ) -> Option<DefinitionSpriteImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.sprite_image_variant(graphics_name).cloned())
    }

    pub fn definition_sprite_variant_names(&self, definition_id: &str) -> Vec<String> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.sprite_variant_keys())
            .unwrap_or_default()
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
        self.check_game_over()?;
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

    fn trigger_lightning(&mut self, position: i32) -> Result<bool, EngineError> {
        const LIGHTNING_DEFINITION: &str = "FXL1";
        if !self.definitions.contains_key(LIGHTNING_DEFINITION) {
            return Ok(false);
        }
        let position = position.max(0);
        let config =
            SpawnConfig::new(LIGHTNING_DEFINITION).with_position(Vector2::new(position, 0));
        let lightning_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(false),
            Err(err) => return Err(err),
        };
        let Some(index) = self.find_object_index(lightning_id) else {
            return Ok(false);
        };
        let args = vec![
            Value::Int(position),
            Value::Int(0),
            Value::Int(-20),
            Value::Int(41),
            Value::Int(5),
            Value::Int(15),
            Value::Bool(true),
        ];
        let _ = self.call_object_function(index, "Activate", args)?;
        Ok(true)
    }

    /// Disaster launch from `C4Weather::Execute` (C4Weather.cpp:104-148),
    /// run on Tick10 frames. The gate Random draws are unconditional — the
    /// configured levels only decide whether a launch follows — so the synced
    /// RNG stream advances identically whether or not disasters are enabled.
    fn tick_weather_events(&mut self, frame: u64) -> Result<(), EngineError> {
        if frame % 10 != 0 {
            return Ok(());
        }
        let width = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.width() as i32)
            .unwrap_or(0);
        let height = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.estimated_height())
            .unwrap_or(0);

        // Meteorite (C4Weather.cpp:106-120)
        if self.rng.random(60) == 0 && self.rng.random(100) < self.environment.meteorite {
            // force argument evaluation order (C4Weather.cpp:113-115)
            let r2 = self.rng.random(100 + 1);
            let r1 = self.rng.random(width);
            if self.trigger_meteorite(r1, r2)? {
                self.weather_events.push(WeatherEvent::Meteorite { x: r1 });
            }
        }
        // Lightning (C4Weather.cpp:122-127)
        if self.rng.random(35) == 0 && self.rng.random(100) < self.environment.lightning {
            let position = self.rng.random(width);
            if self.trigger_lightning(position)? {
                self.weather_events
                    .push(WeatherEvent::Lightning { position });
            }
        }
        // Earthquake (C4Weather.cpp:129-136)
        if self.rng.random(50) == 0 && self.rng.random(100) < self.environment.earthquake {
            // force argument evaluation order (C4Weather.cpp:132-134)
            let r2 = self.rng.random(height);
            let r1 = self.rng.random(width);
            if self.trigger_earthquake(r1, r2)? {
                self.weather_events
                    .push(WeatherEvent::Earthquake { x: r1, y: r2 });
            }
        }
        // Volcano (C4Weather.cpp:138-147)
        if self.rng.random(60) == 0 && self.rng.random(100) < self.environment.volcano {
            // force argument evaluation order (C4Weather.cpp:141-143)
            let r2 = self.rng.random(10);
            let r1 = self.rng.random(width);
            let size = (15 * height / 500 + r2).clamp(10, 60);
            if self.trigger_volcano(r1, height - 1, size)? {
                self.weather_events.push(WeatherEvent::Volcano {
                    x: r1,
                    y: height - 1,
                    size,
                });
            }
        }
        Ok(())
    }

    /// Meteor creation (C4Weather.cpp:110-119): "METO" at y = -20 (TopOpen;
    /// the cave-landscape y = 5 variant needs the scenario TopOpen flag,
    /// which the engine does not carry yet) with xdir = itofix(r2-50)/10,
    /// ydir = 0, rdir = itofix(1)/5.
    fn trigger_meteorite(&mut self, x: i32, r2: i32) -> Result<bool, EngineError> {
        const METEOR_DEFINITION: &str = "METO";
        if !self.definitions.contains_key(METEOR_DEFINITION) {
            return Ok(false);
        }
        let config =
            SpawnConfig::new(METEOR_DEFINITION).with_position(Vector2::new(x.max(0), -20));
        let meteor_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(false),
            Err(err) => return Err(err),
        };
        let Some(index) = self.find_object_index(meteor_id) else {
            return Ok(false);
        };
        let object = &mut self.objects[index];
        object.fixed_velocity = FixedVec2::new(
            C4Fixed::from_raw(itofix(r2 - 50).val() / 10),
            C4Fixed::ZERO,
        );
        object.rotation_velocity = C4Fixed::from_raw(itofix(1).val() / 5);
        Ok(true)
    }

    /// `LaunchEarthquake` (C4Weather.cpp:196-203): FXQ1 + Activate().
    fn trigger_earthquake(&mut self, x: i32, y: i32) -> Result<bool, EngineError> {
        const EARTHQUAKE_DEFINITION: &str = "FXQ1";
        if !self.definitions.contains_key(EARTHQUAKE_DEFINITION) {
            return Ok(false);
        }
        let config = SpawnConfig::new(EARTHQUAKE_DEFINITION)
            .with_position(Vector2::new(x.max(0), y.max(0)));
        let quake_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(false),
            Err(err) => return Err(err),
        };
        let Some(index) = self.find_object_index(quake_id) else {
            return Ok(false);
        };
        let _ = self.call_object_function(index, "Activate", Vec::new())?;
        Ok(true)
    }

    /// `LaunchVolcano` (C4Weather.cpp:178-184): FXV1 + Activate(x, y, size,
    /// mat) with mat = Material "Lava" (C4Weather.cpp:144).
    fn trigger_volcano(&mut self, x: i32, y: i32, size: i32) -> Result<bool, EngineError> {
        const VOLCANO_DEFINITION: &str = "FXV1";
        if !self.definitions.contains_key(VOLCANO_DEFINITION) {
            return Ok(false);
        }
        let config = SpawnConfig::new(VOLCANO_DEFINITION)
            .with_position(Vector2::new(x.max(0), y.max(0)));
        let volcano_id = match self.spawn_object(config) {
            Ok(id) => id,
            Err(EngineError::UnknownDefinition(_)) => return Ok(false),
            Err(err) => return Err(err),
        };
        let Some(index) = self.find_object_index(volcano_id) else {
            return Ok(false);
        };
        let lava = self
            .materials
            .id_of("Lava")
            .map(|id| id.index() as i32)
            .unwrap_or(-1);
        let args = vec![
            Value::Int(x),
            Value::Int(y),
            Value::Int(size),
            Value::Int(lava),
        ];
        let _ = self.call_object_function(index, "Activate", args)?;
        Ok(true)
    }

    pub fn tick(&mut self) -> Result<SimulationSnapshot, EngineError> {
        self.frame += 1;
        self.objective_check_counter =
            (self.objective_check_counter + 1) % GAME_OVER_CHECK_INTERVAL;
        let frame = self.frame;
        // C4GameControl::Ticks runs with the frame advance (C4Game.cpp:801)
        self.control_ticks();
        self.tick_pxs();
        self.tick_particles();
        let mut rescan_mass_movers = false;
        if let Some(landscape) = self.landscape.as_mut() {
            self.mass_movers
                .execute(landscape, &self.materials, &mut self.rng);
            if landscape.take_mass_mover_dirty() {
                rescan_mass_movers = true;
            }
        }
        if rescan_mass_movers {
            if let Some(landscape) = self.landscape.as_ref() {
                self.mass_movers
                    .seed_from_landscape(landscape, &self.materials);
            }
        }
        self.weather_events.clear();
        self.environment.advance_frame(&mut self.rng, frame);
        self.tick_weather_events(frame)?;
        if let Some(sky) = &mut self.sky {
            sky.advance(&self.environment);
        }
        self.apply_landscape_temperature_conversions();
        self.tick_player_systems();
        if self.scenario_script.is_some() {
            let snapshot = self.snapshot();
            let random = self.next_random_i32();
            let rng_state = self.rng.clone();
            let environment = self.environment;
            let global_effects = self.global_effects.clone();
            let particle_defs = self.particle_system.def_names();
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
                    particle_defs,
                )?
            };
            self.rng = new_rng;
            self.audio_registry = audio_state;
            self.apply_scenario_batch(batch)?;
        }
        let mut spawn_requests = Vec::new();
        self.tick_global_effects();
        let mut selected_objects = HashSet::new();
        for selection in self.crew_selection.values() {
            for id in selection.selected() {
                selected_objects.insert(*id);
            }
            if let Some(cursor) = selection.cursor() {
                selected_objects.insert(cursor);
            }
        }

        let mut command_snapshots: HashMap<ObjectId, CommandObjectSnapshot> =
            HashMap::with_capacity(self.objects.len());
        for object in &self.objects {
            let (procedure, line_connect, ocf_base, collectible) = self
                .definitions
                .get(&object.definition_id)
                .map(|definition| {
                    let procedure = definition
                        .action_library()
                        .procedure_for_action(&object.state.action.name);
                    (
                        procedure,
                        definition.line_connect(),
                        definition.ocf_base(),
                        definition.is_collectible(),
                    )
                })
                .unwrap_or((ActionProcedure::default(), OCF_NORMAL, OCF_NORMAL, false));
            let ocf = ocf::compute(
                ocf_base,
                object.state.crew_member,
                object.state.alive,
                object.state.status,
                object.state.container.is_some(),
                object.state.construction,
            );
            command_snapshots.insert(
                object.id,
                CommandObjectSnapshot {
                    id: object.id,
                    definition_id: object.definition_id.clone(),
                    position: object.state.position,
                    status: object.state.status,
                    destroyed: object.destroyed,
                    category: object.state.category,
                    container: object.state.container,
                    action_target: object.state.action.target,
                    action_procedure: procedure,
                    command_direction: object.state.command_direction,
                    construction: object.state.construction,
                    owner: object.state.owner,
                    crew_member: object.state.crew_member,
                    selected: selected_objects.contains(&object.id),
                    alive: object.state.alive,
                    contents: object.state.contents.clone(),
                    line_connect,
                    ocf,
                    collectible,
                },
            );
        }

        let player_snapshots: HashMap<i32, CommandPlayerSnapshot> = self
            .players
            .iter()
            .map(|(&id, player)| {
                (
                    id,
                    CommandPlayerSnapshot {
                        status: player.status(),
                        surrendered: player.surrendered(),
                        wealth: player.wealth(),
                        home_base_material: player.home_base_material().clone(),
                        knowledge: player.knowledge().cloned().collect(),
                    },
                )
            })
            .collect();

        let definition_snapshots: HashMap<DefinitionId, CommandDefinitionSnapshot> = self
            .definitions
            .iter()
            .map(|(id, definition)| {
                let mut chop_action = None;
                let mut can_chop = false;
                for (action_name, spec) in definition.action_library().specs() {
                    if let Some(procedure_name) = spec.procedure.as_deref() {
                        if ActionProcedure::from_name(procedure_name) == ActionProcedure::Chop {
                            can_chop = true;
                            if chop_action.is_none() {
                                chop_action = Some(action_name.clone());
                            }
                        }
                    }
                }
                (
                    id.clone(),
                    CommandDefinitionSnapshot {
                        value: definition.value(),
                        can_chop,
                        chop_action,
                        constructable: definition.is_constructable(),
                    },
                )
            })
            .collect();

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
                command_events,
                (object_id, previous_owner, new_owner, new_crew),
            ) = {
                let object = &mut self.objects[idx];
                let object_id = object.id;
                let current_position = object.state.position;
                let builder_snapshot = command_snapshots
                    .get(&object_id)
                    .expect("command snapshot exists");
                let command_context = CommandRuntimeContext {
                    frame: self.frame,
                    position: current_position,
                    object: builder_snapshot,
                    objects: &command_snapshots,
                    players: &player_snapshots,
                    definitions: &definition_snapshots,
                    structures_need_energy: self.structures_need_energy,
                    base_buy_enabled: self.base_buy_enabled,
                    base_sell_enabled: self.base_sell_enabled,
                    transfer_zones: &self.transfer_zones,
                };
                if let Some(result) = object.step_command_stack(command_context) {
                    if result.update.is_some() || !result.events.is_empty() {
                        let update = result.update.unwrap_or_default();
                        let mut queued = QueuedCommand::immediate(update);
                        if !result.events.is_empty() {
                            queued = queued.with_events(result.events.clone());
                        }
                        object.command_queue.push_front(queued);
                    }
                }
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
                    outcome.command_events,
                    (object.id, previous_owner, new_owner, new_crew),
                )
            };
            self.landscape = landscape_slot;
            self.update_selection_for_state_change(object_id, previous_owner, new_owner, new_crew);

            for update in container_updates {
                self.apply_container_change(update.object_id, update.previous, update.new)?;
            }

            for event in command_events {
                self.apply_command_event(event)?;
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
                    player_commands,
                    landscape_ops,
                    triggered_game_over,
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
                        self.game_over_triggered,
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
                if !landscape_ops.is_empty() {
                    self.apply_landscape_operations(landscape_ops);
                }
                if !player_commands.is_empty() {
                    self.apply_player_commands(player_commands)?;
                }
                if !audio_events.is_empty() {
                    self.pending_audio.extend(audio_events);
                }
                if !event_messages.is_empty() {
                    for command in event_messages {
                        self.messages.apply_command(command);
                    }
                }
                if triggered_game_over {
                    self.request_game_over()?;
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
                    player_commands,
                    landscape_ops,
                    triggered_game_over,
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
                        self.game_over_triggered,
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
                if !landscape_ops.is_empty() {
                    self.apply_landscape_operations(landscape_ops);
                }
                if !player_commands.is_empty() {
                    self.apply_player_commands(player_commands)?;
                }
                if !audio_events.is_empty() {
                    self.pending_audio.extend(audio_events);
                }
                if !event_messages.is_empty() {
                    for command in event_messages {
                        self.messages.apply_command(command);
                    }
                }
                if triggered_game_over {
                    self.request_game_over()?;
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
            let old_movement_velocity = self.objects[idx].fixed_velocity;
            let old_movement_hit_flags = movement_hit_speed_flags(old_movement_velocity);
            let action_name = self.objects[idx].state.action.name.clone();
            let (
                contact_density,
                contact_function_calls,
                border_bound,
                rotateable,
                attach,
                action_procedure,
            ) = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .map(|definition| {
                    (
                        definition.contact_density(),
                        definition.contact_function_calls(),
                        definition.border_bound(),
                        definition.rotateable(),
                        action_library.attach_for_action(&action_name),
                        definition
                            .action_library()
                            .procedure_for_action(&action_name),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        CONTACT_DENSITY_SOLID,
                        false,
                        0,
                        0,
                        action_library.attach_for_action(&action_name),
                        action_library.procedure_for_action(&action_name),
                    )
                });
            let rotation_base_vertices = self.objects[idx].unrotated_shape_vertices();
            let shape_rect = self.objects[idx].current_shape_rect();
            let layer_bounds = self.layer_movement_bounds_for(idx);
            let solid_masks = self.solid_masks_for_movement();
            let object_id = self.objects[idx].id;
            let movement = MovementContactConfig {
                definition_vertices: &rotation_base_vertices,
                contact_density,
                contact_function_calls,
                border_bound,
                shape_rect,
                attach,
                rotateable,
                action_procedure,
                layer_bounds,
                solid_masks: &solid_masks,
                object_id,
            };
            let definition_for_contact = self.definitions.get(&definition_id).cloned();
            let mut contact_rng = self.rng.clone();
            let mut contact_audio = self.audio_registry.clone();
            let mut contact_next_object_id = self.next_object_id;
            let contact_global_effects = self.global_effects.clone();
            let contact_world = self.host_world_context();
            let contact_physics = self.physics;
            let contact_environment = self.environment;
            let contact_frame = self.frame;
            let contact_game_over_triggered = self.game_over_triggered;
            let mut contact_outcomes = Vec::new();
            let mut contact_container_changes = Vec::new();
            let mut contact_selection_changes = Vec::new();
            let contact_function_calls_enabled = movement.contact_function_calls;
            let movement_outcome = {
                let definition_for_contact = definition_for_contact.as_ref();
                let mut run_contact_callback =
                    |object: &mut Object, contact_cnat: u32| -> Result<(), EngineError> {
                        if !contact_function_calls_enabled {
                            return Ok(());
                        }
                        let Some(definition) = definition_for_contact else {
                            return Ok(());
                        };
                        for cnat in [CNAT_LEFT, CNAT_RIGHT, CNAT_TOP, CNAT_BOTTOM] {
                            if contact_cnat & cnat == 0 {
                                continue;
                            }
                            let Some(function_name) = contact_callback_name(cnat) else {
                                continue;
                            };
                            let state_snapshot = object.state.clone();
                            let world = contact_world
                                .clone()
                                .with_next_object_id(contact_next_object_id);
                            let (value, mut outcome, audio_state, new_rng) = definition
                                .call_object_function(
                                    &state_snapshot,
                                    object.id,
                                    function_name,
                                    &[],
                                    contact_rng.clone(),
                                    &contact_global_effects,
                                    contact_physics,
                                    contact_environment,
                                    contact_frame,
                                    world,
                                    contact_game_over_triggered,
                                    contact_audio.clone(),
                                )?;
                            contact_rng = new_rng;
                            contact_audio = audio_state;
                            contact_next_object_id = outcome.next_object_id;

                            if let Some(update) = outcome.object_update.take() {
                                let previous_owner = object.state.owner;
                                let previous_crew_member = object.state.crew_member;
                                let previous_position = object.state.position;
                                let preserves_position = update.position.is_none();
                                let delta: ObjectDelta = update.into();
                                let apply_outcome = object.apply_delta(&delta, &action_library);
                                if preserves_position {
                                    object.state.position = previous_position;
                                }
                                if let Some(change) = apply_outcome.action_change {
                                    object.record_action_event(
                                        change.previous,
                                        ActionTransitionKind::Forced,
                                    );
                                }
                                if let Some((previous, new)) = apply_outcome.container_change {
                                    contact_container_changes.push((object.id, previous, new));
                                }
                                let new_owner = object.state.owner;
                                let new_crew_member = object.state.crew_member;
                                if previous_owner != new_owner
                                    || previous_crew_member != new_crew_member
                                {
                                    contact_selection_changes.push((
                                        object.id,
                                        previous_owner,
                                        new_owner,
                                        new_crew_member,
                                    ));
                                }
                            } else {
                                object.state.action.reconcile_with_library(&action_library);
                            }

                            contact_outcomes.push(outcome);
                            if value.as_bool() {
                                break;
                            }
                        }
                        Ok(())
                    };
                let landscape = self.landscape.as_ref();
                let materials = &self.materials;
                let object = &mut self.objects[idx];
                let mut outcome = object.advance_fixed_position_per_pixel(
                    landscape,
                    materials,
                    movement,
                    &mut run_contact_callback,
                )?;
                outcome.any_contact |= object.advance_fixed_rotation(
                    landscape,
                    materials,
                    movement,
                    outcome.no_attach,
                    outcome.solid_mask_removed,
                    &mut run_contact_callback,
                )?;
                outcome
            };
            self.rng = contact_rng;
            self.audio_registry = contact_audio;
            self.next_object_id = contact_next_object_id;
            for (changed_object_id, previous_owner, new_owner, new_crew_member) in
                contact_selection_changes
            {
                self.update_selection_for_state_change(
                    changed_object_id,
                    previous_owner,
                    new_owner,
                    new_crew_member,
                );
            }
            for (changed_object_id, previous, new) in contact_container_changes {
                self.apply_container_change(changed_object_id, previous, new)?;
            }
            for outcome in contact_outcomes {
                self.apply_callback_outcome(
                    idx,
                    outcome,
                    &action_library,
                    object_id,
                    &definition_id,
                    false,
                )?;
            }
            if self.objects[idx].destroyed
                || matches!(self.objects[idx].state.status, ObjectStatus::Deleted)
            {
                continue;
            }
            self.update_sector_for_index(idx);
            if movement_outcome.no_attach {
                self.apply_no_attach_action(idx, &action_library);
            }
            if movement_outcome.any_contact {
                self.invoke_movement_hit_callbacks(
                    idx,
                    old_movement_velocity,
                    old_movement_hit_flags,
                    &action_library,
                    object_id,
                    &definition_id,
                )?;
            }
            if self.objects[idx].destroyed
                || matches!(self.objects[idx].state.status, ObjectStatus::Deleted)
            {
                continue;
            }

            self.apply_landscape_at_index(idx);
            self.update_sector_for_index(idx);
            // effects (fire) run after movement (C4Object.cpp:1073-1077)
            self.exec_object_fire(idx, frame);

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
                    self.game_over_triggered,
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
                command_ops,
                effects,
                global_effects,
                environment,
                physics,
                landscape_ops,
                particles,
                transfer_zones,
                audio,
                messages,
                player_commands,
                trigger_game_over,
            } = command;

            if trigger_game_over {
                self.request_game_over()?;
            }

            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }

            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }

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
                let delta_outcome = object.apply_delta(&delta, &action_library);
                if let Some(change) = delta_outcome.action_change {
                    object.record_action_event(change.previous, ActionTransitionKind::Forced);
                }
                if let Some(change) = delta_outcome.container_change {
                    container_change = Some(change);
                }
                let mut applied = object.apply_effect_commands(&effects);
                effect_events.append(&mut applied);
                object.clamp_velocity(&self.physics);
                if destroy {
                    effect_events.extend(object.mark_destroyed());
                }
                if !command_ops.is_empty() {
                    object.apply_command_operations(command_ops);
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
            self.update_sector_for_index(idx);
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
                let (
                    global_cmds,
                    emitted_particles,
                    physics_delta,
                    audio_events,
                    event_messages,
                    player_commands,
                    landscape_ops,
                    triggered_game_over,
                    audio_state,
                    new_rng,
                ) = {
                    let definition = self
                        .definitions
                        .get(&definition_id)
                        .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                    let global_view = self.global_effects.clone();
                    let rng_state = self.rng.clone();
                    let object = &mut self.objects[idx];
                    Self::run_effect_events_for_object(
                        definition,
                        self.game_over_triggered,
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
                    )?
                };
                self.rng = new_rng;
                self.audio_registry = audio_state;
                if !player_commands.is_empty() {
                    self.apply_player_commands(player_commands)?;
                }
                if !landscape_ops.is_empty() {
                    self.apply_landscape_operations(landscape_ops);
                }
                if !audio_events.is_empty() {
                    self.pending_audio.extend(audio_events);
                }
                if !event_messages.is_empty() {
                    for command in event_messages {
                        self.messages.apply_command(command);
                    }
                }
                if triggered_game_over {
                    self.request_game_over()?;
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
            self.update_sector_for_index(idx);

            self.trigger_action_callbacks(idx, Some(previous_action_name))?;
            self.update_sector_for_index(idx);

            if self.objects[idx].destroyed {
                continue;
            }

            self.apply_landscape_at_index(idx);
            self.update_sector_for_index(idx);
            // effects (fire) run after movement (C4Object.cpp:1073-1077)
            self.exec_object_fire(idx, frame);
            let (procedure, line_connect, ocf_base, collectible) = self
                .definitions
                .get(&self.objects[idx].definition_id)
                .map(|definition| {
                    (
                        definition
                            .action_library()
                            .procedure_for_action(&self.objects[idx].state.action.name),
                        definition.line_connect(),
                        definition.ocf_base(),
                        definition.is_collectible(),
                    )
                })
                .unwrap_or((
                    action_library.procedure_for_action(&self.objects[idx].state.action.name),
                    OCF_NORMAL,
                    OCF_NORMAL,
                    false,
                ));
            let ocf = ocf::compute(
                ocf_base,
                self.objects[idx].state.crew_member,
                self.objects[idx].state.alive,
                self.objects[idx].state.status,
                self.objects[idx].state.container.is_some(),
                self.objects[idx].state.construction,
            );
            command_snapshots.insert(
                object_id,
                CommandObjectSnapshot {
                    id: object_id,
                    definition_id: self.objects[idx].definition_id.clone(),
                    position: self.objects[idx].state.position,
                    status: self.objects[idx].state.status,
                    destroyed: self.objects[idx].destroyed,
                    category: self.objects[idx].state.category,
                    container: self.objects[idx].state.container,
                    action_target: self.objects[idx].state.action.target,
                    action_procedure: procedure,
                    command_direction: self.objects[idx].state.command_direction,
                    construction: self.objects[idx].state.construction,
                    owner: self.objects[idx].state.owner,
                    crew_member: self.objects[idx].state.crew_member,
                    selected: selected_objects.contains(&object_id),
                    alive: self.objects[idx].state.alive,
                    contents: self.objects[idx].state.contents.clone(),
                    line_connect,
                    ocf,
                    collectible,
                },
            );
            spawn_requests.extend(spawns.into_iter());
        }

        // C4GameObjects::CrossCheck runs once per frame after object
        // execution (C4Game.cpp ExecObjects → Objects.CrossCheck()).
        self.cross_check(frame)?;

        self.detach_destroyed_objects()?;
        self.objects.retain(|object| !object.destroyed);
        self.rebuild_sectors();
        let alive: HashSet<_> = self.objects.iter().map(|object| object.id).collect();
        self.messages.tick(&alive);
        self.transfer_zones.retain_existing(&alive);
        self.prune_selection();
        self.process_spawn_queue(spawn_requests)?;
        self.refresh_elimination_state();
        self.check_game_over()?;
        // Control.DoSyncCheck() closes the frame (C4Game.cpp:829)
        self.do_sync_check();
        let mut snapshot = self.snapshot();
        snapshot.menu_requests = self.pending_menu_requests.drain(..).collect();
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
            fixed_velocity,
            rotation,
            rotation_velocity,
            energy,
            construction,
            damage,
            magic_energy,
            magic_capacity,
            direction,
            command_direction,
            action,
            status,
            owner,
            crew_member,
            alive,
            container,
            vertices,
            graphics_overlays,
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
                object.set_position(position);
            }
            if let Some(velocity) = velocity {
                object.set_velocity(velocity);
            }
            if let Some(fixed_velocity) = fixed_velocity {
                object.fixed_velocity = fixed_velocity;
                object.state.velocity = object.velocity_pixels();
            }
            if let Some(rotation) = rotation {
                let previous_rect = object.current_shape_rect();
                let previous_construction = object.state.construction;
                object.state.rotation = rotation.rem_euclid(360);
                object.fixed_rotation = itofix(object.state.rotation);
                object.refresh_shape_after_state_change(
                    previous_construction,
                    previous_rect,
                    false,
                );
            }
            if let Some(rotation_velocity) = rotation_velocity {
                object.rotation_velocity = rotation_velocity;
            }
            if let Some(energy) = energy {
                object.state.energy = energy;
            }
            if let Some(damage) = damage {
                object.state.damage = damage.max(0);
            }
            if let Some(magic_energy) = magic_energy {
                object.state.magic_energy = magic_energy.max(0);
            }
            if let Some(magic_capacity) = magic_capacity {
                object.state.magic_capacity = magic_capacity.max(0);
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
                object.set_owned_shape_vertices(vertices);
            }
            if let Some(construction) = construction {
                object.set_construction(construction);
            }
            if let Some(overlays) = graphics_overlays {
                object.state.graphics_overlays = overlays;
            }

            object.clamp_velocity(&self.physics);

            if let Some(landscape) = landscape.as_ref() {
                let resolution =
                    landscape.resolve_collision(object.state.position, object.state.velocity);
                if resolution.collided {
                    object.apply_collision_resolution(&resolution);
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

        self.update_sector_for_index(index);
        self.update_selection_for_state_change(object_id, previous_owner, new_owner, new_crew);
        if let Some((previous_container, new_container)) = container_change {
            self.apply_container_change(object_id, previous_container, new_container)?;
        }
        self.trigger_action_callbacks(index, Some(previous_action_name))?;
        self.update_sector_for_index(index);
        if self.objects[index].destroyed
            || matches!(self.objects[index].state.status, ObjectStatus::Deleted)
        {
            self.detach_destroyed_objects()?;
            self.update_sector_for_index(index);
        }
        self.refresh_elimination_state();
        self.check_game_over()?;

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
            self.game_over_triggered,
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
        self.apply_callback_outcome(
            index,
            outcome,
            action_library,
            object_id,
            definition_id,
            true,
        )
    }

    fn apply_callback_outcome(
        &mut self,
        index: usize,
        outcome: compat::EffectContextOutcome,
        action_library: &ActionLibrary,
        object_id: ObjectId,
        definition_id: &str,
        clamp_velocity: bool,
    ) -> Result<(), EngineError> {
        let compat::EffectContextOutcome {
            object: object_effects,
            global: global_effects,
            object_update,
            object_commands,
            command_operations,
            destroy_object,
            environment,
            physics,
            spawns,
            landscape: host_landscape_ops,
            particles,
            transfer_zones,
            messages,
            player_commands,
            audio: outcome_audio,
            trigger_game_over,
            next_object_id,
        } = outcome;

        if !host_landscape_ops.is_empty() {
            self.apply_landscape_operations(host_landscape_ops);
        }

        if !player_commands.is_empty() {
            self.apply_player_commands(player_commands)?;
        }

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

        if trigger_game_over {
            self.request_game_over()?;
        }

        let mut effect_events = Vec::new();
        let mut container_changes = Vec::new();

        let mut command_operations = command_operations;

        let (previous_owner, previous_crew_member) = {
            let object = &self.objects[index];
            (object.state.owner, object.state.crew_member)
        };

        {
            let object = &mut self.objects[index];

            if let Some(update) = object_update {
                let delta: ObjectDelta = update.into();
                let outcome = object.apply_delta(&delta, action_library);
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

            if !command_operations.is_empty() {
                let operations: Vec<_> = std::mem::take(&mut command_operations);
                object.apply_command_operations(operations);
            }

            if !object_commands.is_empty() {
                object.enqueue_commands(object_commands);
            }

            if !object_effects.is_empty() {
                let mut applied = object.apply_effect_commands(&object_effects);
                effect_events.append(&mut applied);
            }

            if clamp_velocity {
                object.clamp_velocity(&self.physics);
            }
        }
        self.update_sector_for_index(index);

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
                player_commands,
                landscape_ops,
                triggered_game_over,
                audio_state,
                new_rng,
            ) = Self::run_effect_events_for_object(
                definition,
                self.game_over_triggered,
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
            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }
            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }
            if !audio_events.is_empty() {
                self.pending_audio.extend(audio_events);
            }
            if !event_messages.is_empty() {
                for command in event_messages {
                    self.messages.apply_command(command);
                }
            }
            if triggered_game_over {
                self.request_game_over()?;
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
        self.update_sector_for_index(index);

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
            self.pxs_system
                .iter()
                .map(|pixel| pxs_snapshot(pixel, &self.materials)),
        );
        particles.extend(
            self.particle_system
                .particles()
                .iter()
                .map(system_particle_snapshot),
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
            let (wealth, score) = self
                .players
                .get(&owner)
                .map(|player| (player.wealth(), player.points()))
                .unwrap_or((0, 0));
            hud_players.push(HudPlayerSnapshot {
                owner,
                crew,
                focus,
                eliminated,
                wealth,
                score,
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
            game_over: self.game_over_triggered,
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
            menu_requests: Vec::new(),
            audio: Vec::new(),
        }
    }

    /// `C4ControlSyncCheck::Set` (C4Control.cpp:445-458): the per-frame
    /// determinism digest. `Random3` is the Rnd3 ring pointer (`FRndPtr3`),
    /// `AllCrewPosX` sums `fixtoi(fix_x, 100)` (centipixels) over the
    /// players' crew lists (C4Control.cpp:460-467), `SectShapeSum` counts the
    /// sector shape lists (C4Sector.cpp:197-203). `MassMoverIndex` remains a
    /// signature hash until the mass-mover gets C++ `CreatePtr` slots.
    pub fn sync_check(&self, by_client: i32) -> SyncCheckPacket {
        let frame = saturating_u64_to_i32(self.frame);
        let crew_positions_sum: i64 = {
            let mut owners: Vec<&Player> = self.players.values().collect();
            owners.sort_unstable_by_key(|player| player.id());
            owners
                .iter()
                .flat_map(|player| player.crew().iter())
                .filter_map(|id| self.find_object_index(*id))
                .map(|index| i64::from(fixtoi_prec(self.objects[index].fixed_position.x, 100)))
                .sum()
        };
        let pxs_count = i32::try_from(self.pxs_system.count()).unwrap_or(i32::MAX);
        // MassMover.CreatePtr (C4Control.cpp:454)
        let mass_mover_index = self.mass_movers.create_ptr();
        let object_count = i32::try_from(self.objects.len()).unwrap_or(i32::MAX);
        let object_enumeration_index = saturating_u64_to_i32(self.next_object_id);
        let sector_shape_sum = self
            .sectors
            .as_ref()
            .map(|sectors| i32::try_from(sectors.shape_sum()).unwrap_or(i32::MAX))
            .unwrap_or(0);

        SyncCheckPacket {
            frame,
            control_tick: self.control_tick,
            random3: self.rng.rnd3_ptr(),
            random_count: self.rng.count,
            crew_positions_sum: saturating_i64_to_i32(crew_positions_sum),
            pxs_count,
            mass_mover_index,
            object_count,
            object_enumeration_index,
            sector_shape_sum,
            by_client,
        }
    }

    /// `C4GameControl::Ticks` (C4GameControl.cpp:326-332): advance
    /// ControlTick every ControlRate frames and request a sync check every
    /// SyncRate frames (C4SyncCheckRate = 100, C4GameControl.h:38).
    fn control_ticks(&mut self) {
        if self.frame % self.control_rate.max(1) as u64 == 0 {
            self.control_tick += 1;
        }
        if self.frame % self.sync_rate.max(1) as u64 == 0 {
            self.do_sync = true;
        }
    }

    /// `C4GameControl::DoSyncCheck` (C4GameControl.cpp:441-468), run at the
    /// end of the frame (C4Game.cpp:829): build the digest once per DoSync,
    /// keep it in the local queue (the network layer exchanges packets and
    /// feeds foreign ones to `register_remote_sync_check`), drop old entries.
    fn do_sync_check(&mut self) {
        if !self.do_sync {
            return;
        }
        self.do_sync = false;
        let packet = self.sync_check(0);
        if self.get_sync_check(packet.frame).is_none() {
            self.sync_checks.push(packet);
        }
        self.remove_old_sync_checks();
    }

    /// `C4GameControl::GetSyncCheck` (C4GameControl.cpp:493-506).
    pub fn get_sync_check(&self, frame: i32) -> Option<&SyncCheckPacket> {
        self.sync_checks.iter().find(|check| check.frame == frame)
    }

    /// `C4GameControl::RemoveOldSyncChecks` (C4GameControl.cpp:508-522):
    /// drop checks older than `FrameCounter - C4SyncCheckMaxKeep` (50).
    fn remove_old_sync_checks(&mut self) {
        let cutoff = saturating_u64_to_i32(self.frame) - 50;
        self.sync_checks.retain(|check| check.frame >= cutoff);
    }

    /// `C4ControlSyncCheck::Execute` (C4Control.cpp:469-525) for a sync check
    /// received from another client: compare against the local digest for the
    /// same frame, or queue it until that frame's local check exists. Returns
    /// false on synchronization loss.
    pub fn register_remote_sync_check(&mut self, packet: SyncCheckPacket) -> bool {
        let Some(local) = self.get_sync_check(packet.frame) else {
            self.sync_checks.push(packet);
            return true;
        };
        local.matches(&packet)
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
                    command_stack: object.commands.snapshot(),
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
            self.pxs_system
                .iter()
                .map(|pixel| pxs_snapshot(pixel, &self.materials)),
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
            pending_menu_requests: self.pending_menu_requests.clone(),
            game_over: self.game_over_triggered,
            landscape_insert_thrust: self.landscape_insert_thrust,
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
        self.landscape_insert_thrust = state.landscape_insert_thrust;
        self.mass_movers
            .set_landscape_insert_thrust(self.landscape_insert_thrust);
        self.landscape = state.landscape.clone();
        if let Some(landscape) = self.landscape.as_ref() {
            self.mass_movers
                .seed_from_landscape(landscape, &self.materials);
        }
        if let Some(landscape) = self.landscape.as_mut() {
            landscape.take_mass_mover_dirty();
        } else {
            self.mass_movers.clear();
        }
        self.rng = state.rng.clone();
        self.objects.clear();
        self.global_effects = state.global_effects.clone();
        self.particles.clear();
        self.pxs_system.clear();
        self.particle_system.clear_particles();
        for snapshot in &state.particles {
            if snapshot.definition_id.starts_with("material/pxs/") && snapshot.parameter_b >= 0 {
                if let Some(material) = MaterialId::new(snapshot.parameter_b as usize) {
                    // raw C4Fixed state when present (lossless save/load);
                    // float projections only for legacy snapshots
                    let [x, y, xdir, ydir] = snapshot.pxs_fixed.unwrap_or([
                        math::ftofix(snapshot.position.x).val(),
                        math::ftofix(snapshot.position.y).val(),
                        math::ftofix(snapshot.velocity.x).val(),
                        math::ftofix(snapshot.velocity.y).val(),
                    ]);
                    self.pxs_system.create(
                        material,
                        C4Fixed::from_raw(x),
                        C4Fixed::from_raw(y),
                        C4Fixed::from_raw(xdir),
                        C4Fixed::from_raw(ydir),
                    );
                }
                continue;
            }
            if self
                .particle_system
                .get_def(&snapshot.definition_id)
                .is_some()
            {
                self.particle_system.restore_particle(particles::Particle {
                    def_name: snapshot.definition_id.clone(),
                    x: snapshot.position.x,
                    y: snapshot.position.y,
                    xdir: snapshot.velocity.x,
                    ydir: snapshot.velocity.y,
                    life: snapshot.life,
                    a: snapshot.parameter_a,
                    b: snapshot.parameter_b,
                    layer: snapshot.layer.clone(),
                });
                continue;
            }
            self.particles
                .push(ActiveParticle::from_snapshot(snapshot.clone()));
        }
        self.transfer_zones = TransferZoneTable::from_states(&state.transfer_zones);
        self.messages.restore(state.messages.clone());
        self.pending_menu_requests = state.pending_menu_requests.clone();
        self.crew_selection = state
            .crew_selection
            .iter()
            .map(|(&owner, selection)| (owner, CrewSelection::from(selection.clone())))
            .collect();

        let mut container_assignments = Vec::new();
        for persisted in &state.objects {
            let snapshot = &persisted.snapshot;
            let shape_template = {
                let definition =
                    self.definitions
                        .get(&snapshot.definition_id)
                        .ok_or_else(|| {
                            EngineError::UnknownDefinition(snapshot.definition_id.clone())
                        })?;
                ObjectShapeTemplate::new(
                    definition.shape_vertices().to_vec(),
                    definition.shape_rect(),
                    definition.stretch_growth(),
                    definition.rotateable(),
                )
            };
            let mut object = Object::new(
                snapshot.id,
                snapshot.definition_id.clone(),
                ObjectState {
                    position: snapshot.position,
                    velocity: snapshot.velocity,
                    rotation: snapshot.rotation.rem_euclid(360),
                    energy: snapshot.energy,
                    construction: snapshot.construction,
                    damage: snapshot.damage,
                    magic_energy: snapshot.magic_energy,
                    magic_capacity: snapshot.magic_capacity,
                    action: snapshot.action.clone(),
                    direction: snapshot.direction,
                    command_direction: snapshot.command_direction,
                    effects: snapshot.effects.clone(),
                    vertices: snapshot.vertices.clone(),
                    container: None,
                    layer: None,
                    contents: Vec::new(),
                    components: snapshot.components.clone(),
                    status: snapshot.status,
                    owner: snapshot.owner,
                    category: snapshot.category,
                    crew_member: snapshot.crew_member,
                    alive: snapshot.alive,
                    base_graphics: snapshot.base_graphics.clone(),
                    graphics_overlays: snapshot.graphics_overlays.clone(),
                    draw_transform: snapshot.draw_transform,
                    local_vars: snapshot.local_vars.clone(),
                    on_fire: snapshot.on_fire,
                    fire_phase: snapshot.fire_phase,
                    fire_caused_by: snapshot.fire_caused_by,
                },
                shape_template,
                snapshot.own_vertices.clone(),
            );
            // Restore authoritative sub-pixel state when the snapshot carried it
            // (whole-pixel objects fall back to the `itofix` set by `Object::new`).
            if let Some(fixed_position) = snapshot.fixed_position {
                object.fixed_position = fixed_position;
                object.state.position = object.position_pixels();
            }
            if let Some(fixed_velocity) = snapshot.fixed_velocity {
                object.fixed_velocity = fixed_velocity;
                object.state.velocity = object.velocity_pixels();
            }
            if let Some(rotation_velocity) = snapshot.rotation_velocity {
                object.rotation_velocity = rotation_velocity;
            }
            if let Some(fixed_rotation) = snapshot.fixed_rotation {
                object.fixed_rotation = fixed_rotation;
                object.state.rotation = fixtoi(fixed_rotation);
            }
            object.command_queue = VecDeque::from(persisted.command_queue.clone());
            object
                .commands
                .restore_from_snapshot(&persisted.command_stack);
            self.objects.push(object);
            if let Some(container) = snapshot.container {
                container_assignments.push((snapshot.id, container));
            }
        }
        self.reset_sectors_from_landscape();

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
        self.players_registered = !self.players.is_empty();
        self.game_over_triggered = state.game_over;

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
        self.check_game_over()?;

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
        game_over_triggered: bool,
        mut rng: LcgRng,
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
            Vec<PlayerCommand>,
            Vec<LandscapeOperation>,
            bool,
            AudioRegistry,
            LcgRng,
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
                Vec::new(),
                Vec::new(),
                false,
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
        let mut pending_player_commands = Vec::new();
        let mut pending_landscape_ops = Vec::new();
        let mut game_over_requested = false;

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
                    game_over_triggered,
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
                    game_over_triggered,
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
                    game_over_triggered,
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
                command_operations,
                destroy_object,
                environment: environment_update,
                physics: physics_update,
                landscape: host_landscape_ops,
                particles: mut emitted_particles,
                messages: event_messages,
                player_commands: effect_player_commands,
                audio: outcome_audio,
                trigger_game_over,
                ..
            } = outcome;

            if !host_landscape_ops.is_empty() {
                pending_landscape_ops.extend(host_landscape_ops);
            }

            if !effect_player_commands.is_empty() {
                pending_player_commands.extend(effect_player_commands);
            }

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
                let outcome = object.apply_delta(&delta, definition.action_library());
                if let Some(change) = outcome.action_change {
                    object.record_action_event(change.previous, ActionTransitionKind::Forced);
                }
                state_snapshot = object.state.clone();
            }

            if !command_operations.is_empty() {
                object.apply_command_operations(command_operations);
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

            if trigger_game_over {
                game_over_requested = true;
            }
        }

        *environment = current_environment;

        Ok((
            global_commands,
            pending_particles,
            accumulated_physics,
            pending_audio,
            pending_messages,
            pending_player_commands,
            pending_landscape_ops,
            game_over_requested,
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

    fn apply_landscape(&self, object: &mut Object) {
        if let Some(landscape) = &self.landscape {
            let resolution =
                landscape.resolve_collision(object.state.position, object.state.velocity);
            if resolution.collided {
                object.apply_collision_resolution(&resolution);
                if let Some(material_id) = resolution.material {
                    if let Some(material) = self.materials.get_by_id(material_id) {
                        object.apply_material_interaction(material);
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

    fn apply_player_commands(&mut self, commands: Vec<PlayerCommand>) -> Result<(), EngineError> {
        for command in commands {
            match command {
                PlayerCommand::AdjustHomeBaseMaterial {
                    player_id,
                    definition_id,
                    delta,
                } => {
                    self.adjust_player_home_base_material(player_id, definition_id, delta)?;
                }
                PlayerCommand::AdjustHomeBaseProduction {
                    player_id,
                    definition_id,
                    delta,
                } => {
                    self.adjust_player_home_base_production(player_id, definition_id, delta)?;
                }
                PlayerCommand::GrantKnowledge {
                    player_id,
                    definition_id,
                } => {
                    self.grant_player_knowledge(player_id, definition_id)?;
                }
                PlayerCommand::RevokeKnowledge {
                    player_id,
                    definition_id,
                } => {
                    self.revoke_player_knowledge(player_id, &definition_id)?;
                }
            }
        }
        Ok(())
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
            let gravity = self.physics.gravity_as_c4fixed();
            if let Some(definition) = self.definitions.get(&definition_id) {
                let object = &self.objects[idx];
                let procedure = definition
                    .action_library()
                    .procedure_for_action(&object.state.action.name);
                let gravity = procedure.gravity_component_fixed(gravity);
                (procedure, definition.movement_profile(), gravity)
            } else {
                let procedure = ActionProcedure::default();
                let gravity = procedure.gravity_component_fixed(gravity);
                (procedure, MovementProfile::default(), gravity)
            }
        };

        if matches!(procedure, ActionProcedure::Dig) {
            self.apply_dig_procedure(idx, &definition_id);
        }

        if matches!(procedure, ActionProcedure::Bridge)
            && !self.apply_bridge_procedure(idx, command_direction, &definition_id)
        {
            return;
        }

        if matches!(procedure, ActionProcedure::Build)
            && !self.apply_build_procedure(idx, &definition_id)
        {
            return;
        }

        if matches!(procedure, ActionProcedure::Fight)
            && !self.apply_fight_procedure(idx, &definition_id)
        {
            return;
        }

        if matches!(procedure, ActionProcedure::Attach)
            && !self.apply_attach_procedure(idx, &definition_id)
        {
            return;
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
            object.fixed_velocity.y += gravity_component;
            if procedure.allows_wind() {
                self.environment
                    .apply_to_velocity(&mut object.fixed_velocity, self.frame);
            }
            if procedure.locks_vertical_velocity() {
                object.fixed_velocity.y = C4Fixed::ZERO;
            }
            let mut pending_direction = None;
            match procedure {
                ActionProcedure::Float | ActionProcedure::Flight => {
                    apply_float_command_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        movement_profile,
                    );
                }
                ActionProcedure::Swim => {
                    apply_swim_command_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        movement_profile,
                        gravity_component,
                    );
                }
                ActionProcedure::Walk => {
                    apply_walk_command_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        movement_profile,
                    );
                }
                ActionProcedure::Scale => {
                    apply_scale_command_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        movement_profile,
                        object.state.direction,
                    );
                }
                ActionProcedure::Hang => {
                    pending_direction = apply_hangle_command_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        movement_profile,
                        object.state.direction,
                    );
                }
                ActionProcedure::Dig => {
                    pending_direction = apply_dig_command_movement(
                        &mut object.fixed_velocity,
                        command_direction,
                        movement_profile,
                        object.state.direction,
                    );
                }
                ActionProcedure::Push => {
                    if !push_handled {
                        // If push was not handled earlier (shouldn't happen), ensure velocities stay zeroed.
                        object.fixed_velocity = FixedVec2::ZERO;
                    }
                }
                ActionProcedure::Pull => {
                    if !pull_handled {
                        object.fixed_velocity = FixedVec2::ZERO;
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
                    object.fixed_velocity = FixedVec2::ZERO;
                }
                _ => {}
            }
            self.physics
                .clamp_fixed_velocity(&mut object.fixed_velocity);
            object.refresh_velocity_from_fixed();
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

        if matches!(procedure, ActionProcedure::Lift)
            && !self.apply_lift_to_target(idx, command_direction, action_target)
        {
            self.reset_lift_action(idx, &definition_id);
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
        let desired_velocity = itofix(desired_velocity);
        let lift_force = itofix(lift_force);

        let adjust_velocity = |object: &mut Object| {
            object.fixed_velocity.y =
                step_fixed_toward(object.fixed_velocity.y, desired_velocity, lift_force);
            physics.clamp_fixed_velocity(&mut object.fixed_velocity);
            object.refresh_velocity_from_fixed();
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
        let desired_target = if clamped_target <= previous_height {
            let one_beyond = clamped_target.saturating_add(1);
            if one_beyond <= previous_height {
                return None;
            }
            previous_height.saturating_add(1)
        } else {
            clamped_target
        };

        landscape.ensure_surface_at_least(column, desired_target);
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

    fn apply_build_procedure(&mut self, idx: usize, definition_id: &DefinitionId) -> bool {
        let category = self.objects[idx].state.category;
        let is_structure = (category & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK)) != 0;

        let target_id = match self.objects[idx].state.action.target {
            Some(id) => id,
            None => {
                if is_structure {
                    return true;
                }
                self.reset_action_to_default(idx, definition_id, true);
                return false;
            }
        };

        let target_idx = match self.find_object_index(target_id) {
            Some(index) if index != idx => index,
            _ => {
                self.reset_action_to_default(idx, definition_id, true);
                return false;
            }
        };

        if self.objects[target_idx].destroyed || !self.objects[target_idx].state.status.is_active()
        {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        if self.objects[target_idx].state.construction >= FULL_CON {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        let target_definition_id = self.objects[target_idx].definition_id.clone();
        let need_material = self.construction_needs_material
            || (self.objects[target_idx].state.category
                & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK))
                == 0;
        let required_components = self
            .definitions
            .get(&target_definition_id)
            .map(|definition| definition.components().to_vec())
            .unwrap_or_default();
        let level = if self.objects[target_idx].state.container.is_some() {
            1
        } else {
            10
        };
        let target_mass = self
            .definitions
            .get(&target_definition_id)
            .map(|definition| definition.mass().max(1))
            .unwrap_or(100)
            .max(1);

        let build_speed = 100; // Legacy default; physical training not yet modeled.
        let mut delta = (i64::from(level) * i64::from(build_speed) * 150) / i64::from(target_mass);
        if delta <= 0 {
            delta = 1;
        }
        let current_construction = self.objects[target_idx].state.construction;
        let desired_construction =
            (i64::from(current_construction) + delta).clamp(0, i64::from(FULL_CON)) as i32;

        if need_material
            && !required_components.is_empty()
            && !self.ensure_build_components(
                idx,
                target_idx,
                desired_construction,
                &required_components,
            )
        {
            return false;
        }

        {
            let target = &mut self.objects[target_idx];
            target.set_construction(desired_construction);
        }

        if self.objects[target_idx].state.construction >= FULL_CON {
            self.reset_action_to_default(idx, definition_id, true);
        }

        true
    }

    fn ensure_build_components(
        &mut self,
        builder_idx: usize,
        target_idx: usize,
        desired_construction: i32,
        required: &[DefinitionComponent],
    ) -> bool {
        if required.is_empty() {
            return true;
        }

        for component in required {
            if component.count == 0 {
                continue;
            }

            let mut inserted = self.objects[target_idx]
                .state
                .components
                .get(&component.id)
                .copied()
                .unwrap_or(0);

            if inserted > component.count {
                inserted = component.count;
                self.objects[target_idx]
                    .state
                    .components
                    .insert(component.id.clone(), inserted);
            }

            let required_for_progress = ((i64::from(component.count)
                * i64::from(desired_construction))
                + (i64::from(FULL_CON) - 1))
                / i64::from(FULL_CON);

            while i64::from(inserted) < required_for_progress {
                if inserted >= component.count {
                    return false;
                }
                let consumed = self.consume_component_from_contents(builder_idx, &component.id)
                    || self.consume_component_from_container_of(builder_idx, &component.id)
                    || self.consume_component_from_container_of(target_idx, &component.id);
                if !consumed {
                    return false;
                }
                inserted += 1;
                self.objects[target_idx]
                    .state
                    .components
                    .insert(component.id.clone(), inserted);
            }
        }

        true
    }

    fn consume_component_from_contents(
        &mut self,
        container_index: usize,
        component_id: &DefinitionId,
    ) -> bool {
        if container_index >= self.objects.len() {
            return false;
        }
        let contents = self.objects[container_index].state.contents.clone();
        for object_id in contents {
            let Some(child_index) = self.find_object_index(object_id) else {
                continue;
            };
            let child = &self.objects[child_index];
            if child.definition_id != *component_id
                || child.destroyed
                || matches!(child.state.status, ObjectStatus::Deleted)
                || child.state.construction < FULL_CON
            {
                continue;
            }
            self.objects[container_index]
                .state
                .contents
                .retain(|&id| id != object_id);
            self.objects[child_index].state.container = None;
            self.objects[child_index].mark_destroyed();
            return true;
        }
        false
    }

    fn consume_component_from_container_of(
        &mut self,
        object_index: usize,
        component_id: &DefinitionId,
    ) -> bool {
        if object_index >= self.objects.len() {
            return false;
        }
        let container_id = match self.objects[object_index].state.container {
            Some(id) => id,
            None => return false,
        };
        let Some(container_index) = self.find_object_index(container_id) else {
            return false;
        };
        self.consume_component_from_contents(container_index, component_id)
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
        object.set_velocity(Vector2::ZERO);
        if matches!(result, ActionUpdateResult::Applied)
            && previous.name != object.state.action.name
        {
            object.record_action_event(previous, ActionTransitionKind::Forced);
        }
    }

    fn reset_lift_action(&mut self, idx: usize, definition_id: &DefinitionId) {
        self.reset_action_to_default(idx, definition_id, true);
    }

    fn apply_no_attach_action(&mut self, idx: usize, library: &ActionLibrary) {
        if idx >= self.objects.len() {
            return;
        }

        let previous = self.objects[idx].state.action.clone();
        if previous.name == library.default_action() {
            return;
        }

        let next_action = if library.contains("Jump") {
            "Jump"
        } else {
            library.default_action()
        };

        let update = ActionUpdate {
            name: Some(next_action.to_string()),
            phase: Some(0),
            ticks: Some(0),
            force: true,
            data: None,
            target: Some(None),
            target2: Some(None),
        };

        let object = &mut self.objects[idx];
        let result = object
            .state
            .action
            .apply_update_with_library(&update, library);
        object.state.action.target = None;
        object.state.action.target2 = None;
        object.state.command_direction = CommandDirection::Stop;
        if matches!(result, ActionUpdateResult::Applied)
            && previous.name != object.state.action.name
        {
            object.record_action_event(previous, ActionTransitionKind::Forced);
        }
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

        if previous_container != target_container
            && self
                .apply_container_change(object_id, previous_container, target_container)
                .is_err()
        {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
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
        object.set_position(new_position);
        object.set_velocity(Vector2::ZERO);

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

    fn apply_fight_procedure(&mut self, idx: usize, definition_id: &DefinitionId) -> bool {
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

        // Physical training (C4Object.cpp:5214-5216): Tick5 trains Fight.
        if self.frame % 5 == 0 {
            self.train_physical(idx, |physical| &mut physical.fight, 1, C4_MAX_PHYSICAL);
        }

        // Direction (C4Object.cpp:5218-5220): face the target; equal x keeps
        // the previous facing.
        let direction = if target_position.x > fighter_position.x {
            Direction::Right
        } else if target_position.x < fighter_position.x {
            Direction::Left
        } else {
            initial_direction
        };

        // Position (C4Object.cpp:5221-5228): stand beside the target at half
        // its shape width + 2, approach with the Walk physical:
        // lLimit = ValByPhysical(95, Walk), Towards(xdir, ±lLimit, lLimit).
        let target_half_width = fight_distance_threshold(&target_vertices, &target_vertices) / 2;
        let mut approach_x = fighter_position.x;
        if direction == Direction::Left {
            approach_x = target_position.x + target_half_width + 2;
        }
        if direction == Direction::Right {
            approach_x = target_position.x - target_half_width - 2;
        }
        let limit = math::val_by_physical(95, self.object_physical(idx).walk);
        let physics = self.physics;
        let fighter = &mut self.objects[idx];
        fighter.state.direction = direction;
        let mut xdir = fighter.fixed_velocity.x;
        match fighter_position.x.cmp(&approach_x) {
            std::cmp::Ordering::Equal => math::towards(&mut xdir, C4Fixed::ZERO, limit),
            std::cmp::Ordering::Less => math::towards(&mut xdir, limit, limit),
            std::cmp::Ordering::Greater => math::towards(&mut xdir, -limit, limit),
        }
        fighter.fixed_velocity.x = xdir;

        // Distance check (C4Object.cpp:5229-5234): own shape width bounds.
        let threshold = fight_distance_threshold(&fighter_vertices, &fighter_vertices);
        if (fighter_position.x - target_position.x).abs() > threshold
            || (fighter_position.y - target_position.y).abs() > threshold
        {
            self.reset_action_to_default(idx, definition_id, true);
            return false;
        }

        // Other (C4Object.cpp:5235-5238): grounded fighting. The Tick35
        // DoExperience(+2) needs the C4ObjectInfo model.
        let fighter = &mut self.objects[idx];
        fighter.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut fighter.fixed_velocity);
        fighter.refresh_velocity_from_fixed();

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
        let accel = itofix(acceleration.max(0));
        let new_target_velocity = step_fixed_toward(
            target.fixed_velocity.x,
            itofix(desired_target_velocity),
            accel,
        );
        target.fixed_velocity.x = clamp_fixed_to_limit(new_target_velocity, speed_limit);
        physics.clamp_fixed_velocity(&mut target.fixed_velocity);
        target.refresh_velocity_from_fixed();

        let new_puller_velocity = step_fixed_toward(
            puller.fixed_velocity.x,
            itofix(desired_puller_velocity),
            accel,
        );
        puller.fixed_velocity.x = clamp_fixed_to_limit(new_puller_velocity, speed_limit);
        puller.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut puller.fixed_velocity);
        puller.refresh_velocity_from_fixed();
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
        let push_accel = push_accel.max(0);
        let push_accel_fixed = itofix(push_accel);
        let new_target_velocity = step_fixed_toward(
            target.fixed_velocity.x,
            itofix(desired_target_velocity),
            push_accel_fixed,
        );
        target.fixed_velocity.x = clamp_fixed_to_limit(new_target_velocity, push_speed);
        if straighten && push_accel > 0 {
            target.fixed_velocity.y =
                decelerate_fixed_toward_zero(target.fixed_velocity.y, push_accel_fixed);
        }
        physics.clamp_fixed_velocity(&mut target.fixed_velocity);
        target.refresh_velocity_from_fixed();

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

        let new_pusher_velocity = step_fixed_toward(
            pusher.fixed_velocity.x,
            itofix(desired_pusher_velocity),
            push_accel_fixed,
        );
        pusher.fixed_velocity.x = clamp_fixed_to_limit(new_pusher_velocity, push_speed);
        pusher.fixed_velocity.y = C4Fixed::ZERO;
        physics.clamp_fixed_velocity(&mut pusher.fixed_velocity);
        pusher.refresh_velocity_from_fixed();
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
        object.apply_collision_resolution(&resolution);
        if let Some(material_id) = resolution.material {
            if let Some(material) = self.materials.get_by_id(material_id) {
                object.apply_material_interaction(material);
            }
        }
    }

    /// `C4GameObjects::CrossCheck` reverse area check
    /// (C4GameObjects.cpp:140-197), run once per frame after object
    /// execution: every frame an OCF_Alive victim takes OCF_HitSpeed2 hits
    /// from C4D_Object projectiles inside its shape; on Tick3 frames
    /// collection runs (OCF_Collection vs OCF_Carryable, Collection rect).
    /// Candidates are deduplicated per victim like the C++ Marker. Pass 1
    /// (Tick5 fight / Tick35 contact incineration) and pass 3 (contained
    /// fight) still need the hostility and fire models.
    fn cross_check(&mut self, frame: u64) -> Result<(), EngineError> {
        self.cross_check_at_object_pass(frame)?;
        self.cross_check_reverse_area_pass(frame)?;
        self.cross_check_contained_pass(frame)
    }

    /// CrossCheck pass 3: Contained check (C4GameObjects.cpp:199-230). On
    /// Tick10 frames, contained FightReady objects fight hostile FightReady
    /// company sharing their container — directly, with no RejectFight veto.
    fn cross_check_contained_pass(&mut self, frame: u64) -> Result<(), EngineError> {
        if frame % 10 != 0 {
            return Ok(());
        }
        let focf = crate::ocf::FIGHT_READY;
        let tocf = crate::ocf::FIGHT_READY;
        let object_ids: Vec<ObjectId> = self.objects.iter().map(|object| object.id).collect();
        'outer: for obj1_id in object_ids {
            let Some(idx) = self.find_object_index(obj1_id) else {
                continue;
            };
            let container = {
                let obj1 = &self.objects[idx];
                if obj1.destroyed || !obj1.state.status.is_active() {
                    continue;
                }
                match obj1.state.container {
                    Some(container) => container,
                    None => continue,
                }
            };
            if self.object_ocf_at_index(idx) & focf == 0 {
                continue;
            }
            let obj1_layer = self.objects[idx].state.layer;
            let contents = match self
                .find_object_index(container)
                .map(|container_idx| self.objects[container_idx].state.contents.clone())
            {
                Some(contents) => contents,
                None => continue,
            };
            for obj2_id in contents {
                if obj2_id == obj1_id {
                    continue;
                }
                let Some(idx) = self.find_object_index(obj1_id) else {
                    continue 'outer;
                };
                let Some(obj2_idx) = self.find_object_index(obj2_id) else {
                    continue;
                };
                {
                    let obj2 = &self.objects[obj2_idx];
                    if obj2.destroyed
                        || !obj2.state.status.is_active()
                        || obj2.state.container.is_none()
                        || obj2.state.layer != obj1_layer
                    {
                        continue;
                    }
                }
                if self.object_ocf_at_index(obj2_idx) & tocf == 0 {
                    continue;
                }
                let ocf1 = self.object_ocf_at_index(idx);
                // Fight (C4GameObjects.cpp:218-227)
                if ocf1 & crate::ocf::FIGHT_READY != 0 {
                    let owner1 = self.objects[idx].state.owner;
                    let owner2 = self.objects[obj2_idx].state.owner;
                    if self.players_hostile(owner1, owner2) {
                        self.object_action_fight(obj1_id, obj2_id);
                        self.object_action_fight(obj2_id, obj1_id);
                        // obj1 might have been tampered with
                        let Some(idx) = self.find_object_index(obj1_id) else {
                            continue 'outer;
                        };
                        let obj1 = &self.objects[idx];
                        if obj1.destroyed
                            || !obj1.state.status.is_active()
                            || obj1.state.container.is_none()
                            || self.object_ocf_at_index(idx) & focf == 0
                        {
                            continue 'outer;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// CrossCheck pass 1: AtObject check (C4GameObjects.cpp:97-138). On Tick5
    /// frames FightReady objects standing at a hostile FightReady object
    /// start fighting both ways after the RejectFight callbacks. The Tick35
    /// contact-incineration arm (OCF_OnFire vs OCF_Inflammable with the
    /// `!Random(ContactIncinerate)` draw) still needs the fire model — no
    /// Rust object ever carries OCF_OnFire yet, so the C++ stream consumes no
    /// draws for it either.
    fn cross_check_at_object_pass(&mut self, frame: u64) -> Result<(), EngineError> {
        let tick5 = frame % 5 == 0;
        let tick35 = frame % 35 == 0;
        let mut focf = crate::ocf::NONE;
        let mut tocf = crate::ocf::NONE;
        if tick5 {
            focf |= crate::ocf::FIGHT_READY;
            tocf |= crate::ocf::FIGHT_READY;
        }
        // Very low level: Incineration (C4GameObjects.cpp:106-110)
        if tick35 {
            focf |= crate::ocf::ON_FIRE;
            tocf |= crate::ocf::INFLAMMABLE;
        }
        if focf == 0 || tocf == 0 {
            return Ok(());
        }
        let object_ids: Vec<ObjectId> = self.objects.iter().map(|object| object.id).collect();
        for obj1_id in object_ids {
            let Some(idx) = self.find_object_index(obj1_id) else {
                continue;
            };
            {
                let obj1 = &self.objects[idx];
                if obj1.destroyed
                    || !obj1.state.status.is_active()
                    || obj1.state.container.is_some()
                {
                    continue;
                }
            }
            let ocf1 = self.object_ocf_at_index(idx);
            if ocf1 & focf == 0 {
                continue;
            }
            let position = self.objects[idx].state.position;
            let Some((obj2_idx, obj2_id, ocf2)) = self.at_object(position, tocf, Some(obj1_id))
            else {
                continue;
            };
            // Incineration (C4GameObjects.cpp:120-125): the Random draw runs
            // whenever the OCF pair matches, regardless of its outcome.
            if ocf1 & crate::ocf::ON_FIRE != 0 && ocf2 & crate::ocf::INFLAMMABLE != 0 {
                let contact_incinerate = self
                    .definitions
                    .get(&self.objects[obj2_idx].definition_id)
                    .map(|definition| definition.contact_incinerate())
                    .unwrap_or(0);
                if self.rng.random(contact_incinerate) == 0 {
                    // GetFireCausePlr: the fire effect's CausedBy, NO_OWNER
                    // unless it is a valid player.
                    let cause = self.objects[idx].state.fire_caused_by;
                    let cause = if self.players.contains_key(&cause) {
                        cause
                    } else {
                        OWNER_NONE
                    };
                    let _ = self.incinerate_object(obj2_idx, cause, false, Some(obj1_id))?;
                    continue;
                }
            }
            // Fight (C4GameObjects.cpp:126-136)
            if ocf1 & crate::ocf::FIGHT_READY != 0 && ocf2 & crate::ocf::FIGHT_READY != 0 {
                let owner1 = self.objects[idx].state.owner;
                let owner2 = self.objects[obj2_idx].state.owner;
                if !self.players_hostile(owner1, owner2) {
                    continue;
                }
                // RejectFight callbacks (C4GameObjects.cpp:131-132)
                let reject1 = self.call_object_function(
                    idx,
                    "RejectFight",
                    vec![object_reference_value(obj2_id)],
                )?;
                if reject1.as_bool() {
                    continue;
                }
                let Some(obj2_idx) = self.find_object_index(obj2_id) else {
                    continue;
                };
                let reject2 = self.call_object_function(
                    obj2_idx,
                    "RejectFight",
                    vec![object_reference_value(obj1_id)],
                )?;
                if reject2.as_bool() {
                    continue;
                }
                self.object_action_fight(obj1_id, obj2_id);
                self.object_action_fight(obj2_id, obj1_id);
            }
        }
        Ok(())
    }

    /// `ObjectActionFight` (C4ObjectCom.cpp:157-160):
    /// `SetActionByName("Fight", target)`.
    fn object_action_fight(&mut self, object_id: ObjectId, target_id: ObjectId) {
        let Some(idx) = self.find_object_index(object_id) else {
            return;
        };
        let definition_id = self.objects[idx].definition_id.clone();
        let Some(library) = self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.action_library().clone())
        else {
            return;
        };
        if !library.contains("Fight") {
            return;
        }
        let previous = self.objects[idx].state.action.clone();
        let update = ActionUpdate {
            name: Some("Fight".to_string()),
            phase: Some(0),
            ticks: Some(0),
            force: true,
            data: None,
            target: Some(Some(target_id)),
            target2: Some(None),
        };
        let object = &mut self.objects[idx];
        let result = object
            .state
            .action
            .apply_update_with_library(&update, &library);
        if matches!(result, ActionUpdateResult::Applied)
            && previous.name != object.state.action.name
        {
            object.record_action_event(previous, ActionTransitionKind::Forced);
        }
    }

    /// `C4PlayerList::Hostile` (C4PlayerList.cpp:82-92): false for missing or
    /// identical players; one-way declarations count both ways.
    fn players_hostile(&self, player1: i32, player2: i32) -> bool {
        let (Some(plr1), Some(plr2)) = (self.players.get(&player1), self.players.get(&player2))
        else {
            return false;
        };
        if plr1.id() == plr2.id() {
            return false;
        }
        plr1.is_hostile_towards(plr2.id()) || plr2.is_hostile_towards(plr1.id())
    }

    /// CrossCheck pass 2: reverse area check (C4GameObjects.cpp:140-197).
    fn cross_check_reverse_area_pass(&mut self, frame: u64) -> Result<(), EngineError> {
        let tick3 = frame % 3 == 0;
        let mut focf = crate::ocf::ALIVE;
        let mut tocf = crate::ocf::HIT_SPEED2;
        if tick3 {
            focf |= crate::ocf::COLLECTION;
            tocf |= crate::ocf::CARRYABLE;
        }
        let object_ids: Vec<ObjectId> = self.objects.iter().map(|object| object.id).collect();
        'outer: for obj1_id in object_ids {
            let Some(idx) = self.find_object_index(obj1_id) else {
                continue;
            };
            {
                let obj1 = &self.objects[idx];
                if obj1.destroyed
                    || !obj1.state.status.is_active()
                    || obj1.state.container.is_some()
                {
                    continue;
                }
            }
            if self.object_ocf_at_index(idx) & focf == 0 {
                continue;
            }
            let (definition_id, obj1_layer) = {
                let obj1 = &self.objects[idx];
                (obj1.definition_id.clone(), obj1.state.layer)
            };
            let (shape_rect, collection_rect, obj1_mass) = self
                .definitions
                .get(&definition_id)
                .map(|definition| {
                    (
                        definition.shape_rect(),
                        definition.collection_rect(),
                        definition.mass(),
                    )
                })
                .unwrap_or((None, None, 0));
            let fallback_half_extents = if shape_rect.is_none() {
                Some(Self::object_half_extents(&self.objects[idx]))
            } else {
                None
            };
            // obj1->Area: candidates from the sector lists under the shape
            let collector_shape_rect = self.object_shape_rect(&self.objects[idx]);
            let candidate_ids = self
                .sectors
                .as_ref()
                .map(|sectors| {
                    let area = sectors.area(collector_shape_rect);
                    sectors.object_ids_in_area(&area)
                })
                .unwrap_or_else(|| self.objects.iter().map(|object| object.id).collect());
            // handle collision only once (Marker, C4GameObjects.cpp:163-165)
            let mut marker: HashSet<ObjectId> = HashSet::new();
            for candidate_id in candidate_ids {
                let Some(idx) = self.find_object_index(obj1_id) else {
                    continue 'outer;
                };
                let Some(candidate_idx) = self.find_object_index(candidate_id) else {
                    continue;
                };
                if candidate_idx == idx {
                    continue;
                }
                {
                    let candidate = &self.objects[candidate_idx];
                    if candidate.destroyed
                        || !candidate.state.status.is_active()
                        || candidate.state.container.is_some()
                        || candidate.state.layer != obj1_layer
                    {
                        continue;
                    }
                }
                let ocf2 = self.object_ocf_at_index(candidate_idx);
                if ocf2 & tocf == 0 {
                    continue;
                }
                let obj1_position = self.objects[idx].state.position;
                let candidate_position = self.objects[candidate_idx].state.position;
                let dx = candidate_position.x - obj1_position.x;
                let dy = candidate_position.y - obj1_position.y;
                // Inside(obj2->x - (obj1->x + Shape.x), 0, Shape.Wdt - 1)
                if let Some(shape) = shape_rect {
                    if !shape.contains_offset(dx, dy) {
                        continue;
                    }
                } else if let Some((half_w, half_h)) = fallback_half_extents {
                    if dx < -half_w || dx >= half_w || dy < -half_h || dy >= half_h {
                        continue;
                    }
                }
                if !marker.insert(candidate_id) {
                    continue;
                }
                let ocf1 = self.object_ocf_at_index(idx);
                // Hit (C4GameObjects.cpp:167-184)
                if ocf2 & crate::ocf::HIT_SPEED2 != 0
                    && ocf1 & crate::ocf::ALIVE != 0
                    && self.objects[candidate_idx].state.category & CATEGORY_OBJECT != 0
                {
                    let by_value = object_reference_value(candidate_id);
                    let query = self.call_object_function(
                        idx,
                        "QueryCatchBlow",
                        vec![by_value.clone()],
                    )?;
                    if !query.as_bool() {
                        // "realistic" hit energy (C4GameObjects.cpp:171-173)
                        let v1 = self.objects[idx].fixed_velocity;
                        let v2 = self.objects[candidate_idx].fixed_velocity;
                        let dx_dir = v2.x - v1.x;
                        let dy_dir = v2.y - v1.y;
                        let candidate_mass = self
                            .definitions
                            .get(&self.objects[candidate_idx].definition_id)
                            .map(|definition| definition.mass())
                            .unwrap_or(0);
                        let hit_energy =
                            fixtoi((dx_dir * dx_dir + dy_dir * dy_dir) * candidate_mass / 5);
                        // reduced to 1/3rd, but never dropped to zero by it
                        let hit_energy = (hit_energy / 3).max(i32::from(hit_energy != 0));
                        self.change_object_energy(idx, -(hit_energy / 5), self.objects[candidate_idx].state.owner);
                        let tmass = obj1_mass.max(50);
                        let candidate_velocity = self.objects[candidate_idx].fixed_velocity;
                        // fling unless airborne off-Tick3 (C4GameObjects.cpp:176)
                        let procedure = self
                            .definitions
                            .get(&definition_id)
                            .map(|definition| {
                                definition
                                    .action_library()
                                    .procedure_for_action(&self.objects[idx].state.action.name)
                            })
                            .unwrap_or_default();
                        let has_action = !self.objects[idx].state.action.name.is_empty();
                        if tick3 || (has_action && procedure != ActionProcedure::Flight) {
                            let txdir = C4Fixed::from_raw(
                                candidate_velocity.x.val().wrapping_mul(50) / tmass,
                            );
                            let tydir = C4Fixed::from_raw(
                                -(candidate_velocity.y.val() / 2).abs().wrapping_mul(50) / tmass,
                            );
                            self.fling_object(idx, txdir, tydir);
                        }
                        let _ = self.call_object_function(
                            idx,
                            "CatchBlow",
                            vec![Value::Int(-(hit_energy / 5)), by_value],
                        )?;
                        // obj1 might have been tampered with
                        let Some(idx) = self.find_object_index(obj1_id) else {
                            continue 'outer;
                        };
                        let obj1 = &self.objects[idx];
                        if obj1.destroyed
                            || !obj1.state.status.is_active()
                            || obj1.state.container.is_some()
                            || self.object_ocf_at_index(idx) & focf == 0
                        {
                            continue 'outer;
                        }
                        continue;
                    }
                }
                // Collection (C4GameObjects.cpp:185-194)
                if ocf1 & crate::ocf::COLLECTION != 0 && ocf2 & crate::ocf::CARRYABLE != 0 {
                    let Some(collection_rect) =
                        collection_rect.filter(|rect| rect.is_positive())
                    else {
                        continue;
                    };
                    if !collection_rect.contains_offset(dx, dy) {
                        continue;
                    }
                    let update = ObjectUpdate::new()
                        .with_container(obj1_id)
                        .with_position(obj1_position)
                        .with_velocity(Vector2::ZERO);
                    match self.apply_object_update(candidate_id, update) {
                        Ok(_) => {}
                        Err(EngineError::UnknownObject(_)) => continue,
                        Err(err) => return Err(err),
                    }
                    // obj1 might have been tampered with
                    let Some(idx) = self.find_object_index(obj1_id) else {
                        continue 'outer;
                    };
                    let obj1 = &self.objects[idx];
                    if obj1.destroyed
                        || !obj1.state.status.is_active()
                        || obj1.state.container.is_some()
                        || self.object_ocf_at_index(idx) & focf == 0
                    {
                        continue 'outer;
                    }
                }
            }
        }
        Ok(())
    }

    /// `C4Object::Incinerate` (C4Object.cpp:1230-1241) + the deterministic
    /// core of fxFireStart (C4Effect.cpp:560-641): refuse when already
    /// burning or a dead living; no fire in extinguishing material (checked
    /// BEFORE the FirePhase draw); otherwise set OnFire, draw
    /// `FirePhase = Random(MaxFirePhase)` (one synced draw), store the cause,
    /// and run the Incineration script callback. Still open: BurnTurnTo
    /// ChangeDef, contents ejection, attached-object detach, fire modes,
    /// sounds, and the IncinerationEx blasted-in-water callback.
    fn incinerate_object(
        &mut self,
        idx: usize,
        caused_by: i32,
        blasted: bool,
        _incinerating: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        {
            let state = &self.objects[idx].state;
            // Already on fire (C4Object.cpp:1233)
            if state.on_fire {
                return Ok(false);
            }
            // Dead living don't burn (C4Object.cpp:1235)
            if state.category & CATEGORY_LIVING != 0 && !state.alive {
                return Ok(false);
            }
        }
        // In extinguishing material: no fire caused (C4Effect.cpp:574-583)
        let position = self.objects[idx].state.position;
        let in_extinguisher = self
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.material_at(position.x, position.y))
            .and_then(|material_id| self.materials.get_by_id(material_id))
            .map(|material| material.extinguisher() > 0)
            .unwrap_or(false);
        let fire_caused = !in_extinguisher;
        let (burn_turn_to, incomplete_activity, no_burn_decay) = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .map(|definition| {
                (
                    definition.burn_turn_to().map(str::to_string),
                    definition.incomplete_activity(),
                    definition.no_burn_decay(),
                )
            })
            .unwrap_or((None, false, false));
        // BurnTurnTo: blasts changedef in water too (C4Effect.cpp:579-585)
        if let Some(target) = burn_turn_to.filter(|_| fire_caused || blasted) {
            self.change_object_def(idx, &target);
        }
        // eject contents (C4Effect.cpp:586-594): into the burning object's
        // container when contained, else exit at the object's position
        if !incomplete_activity && !no_burn_decay {
            let container = self.objects[idx].state.container;
            let contents = std::mem::take(&mut self.objects[idx].state.contents);
            for content_id in contents {
                let update = match container {
                    Some(parent) => ObjectUpdate::new().with_container(parent),
                    None => ObjectUpdate::new()
                        .clear_container()
                        .with_position(position),
                };
                let _ = self.apply_object_update(content_id, update);
            }
        }
        // (attached-object detach, C4Effect.cpp:595-600: needs the
        // DFA_ATTACH action scan — open)
        if !fire_caused {
            // blasted but not incinerated: IncinerationEx (C4Effect.cpp:602-607)
            if blasted {
                let _ = self.call_object_function(
                    idx,
                    "IncinerationEx",
                    vec![Value::Int(caused_by)],
                )?;
            }
            return Ok(false);
        }
        // Set values (C4Effect.cpp:632-634)
        {
            let object = &mut self.objects[idx];
            object.state.on_fire = true;
            object.state.fire_caused_by = caused_by;
        }
        self.objects[idx].state.fire_phase = self.rng.random(15); // Random(MaxFirePhase)
        // Engine script call (C4Effect.cpp:638)
        let _ = self.call_object_function(idx, "Incineration", vec![Value::Int(caused_by)])?;
        Ok(true)
    }

    /// `C4Object::ExecFire` (C4Object.cpp:766-810), run for burning objects
    /// after movement like the C++ fire effect timer. Still open: the Tick5
    /// base extinguish (needs the base model), SmokeRate smoke (visual), and
    /// death/removal callbacks from the energy and damage changes.
    fn exec_object_fire(&mut self, idx: usize, frame: u64) {
        if !self.objects[idx].state.on_fire {
            return;
        }
        // Fire Phase (C4Object.cpp:769)
        {
            let object = &mut self.objects[idx];
            object.state.fire_phase = (object.state.fire_phase + 1) % 15;
        }
        let (no_burn_decay, no_burn_damage) = self
            .definitions
            .get(&self.objects[idx].definition_id)
            .map(|definition| (definition.no_burn_decay(), definition.no_burn_damage()))
            .unwrap_or((false, false));
        // Decay: DoCon(-100) every frame (C4Object.cpp:776-778); burned away
        // at zero construction (C4Object::DoCon removal)
        if !no_burn_decay {
            let object = &mut self.objects[idx];
            object.state.construction = (object.state.construction - 100).clamp(0, FULL_CON);
            if object.state.construction == 0 {
                let _ = object.mark_destroyed();
                return;
            }
        }
        // Damage: Tick10 DoDamage(+2) (C4Object.cpp:780)
        if frame % 10 == 0 && !no_burn_damage {
            let object = &mut self.objects[idx];
            object.state.damage = object.state.damage.saturating_add(2);
        }
        // Energy: Tick5 DoEnergy(-1) (C4Object.cpp:782)
        if frame % 5 == 0 {
            self.change_object_energy(idx, -1, self.objects[idx].state.fire_caused_by);
        }
        // Background effects: Tick5 over valid landscape material
        // (C4Object.cpp:791-806) — extinguish in extinguisher material, then
        // the unconditional Random(3) landscape-inflame draw.
        if frame % 5 == 0 {
            let position = self.objects[idx].state.position;
            let material = self
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(position.x, position.y));
            if let Some(material_id) = material {
                let extinguisher = self
                    .materials
                    .get_by_id(material_id)
                    .map(|material| material.extinguisher() > 0)
                    .unwrap_or(false);
                if extinguisher {
                    // Extinguish (C4Object.cpp:799-801); the Pshshsh sound is
                    // presentation-only.
                    self.objects[idx].state.on_fire = false;
                }
                // Inflame (C4Object.cpp:803-804)
                if self.rng.random(3) == 0 {
                    let _ = self.spawn_fire_at(position.x, position.y);
                }
            }
        }
    }

    /// `C4Object::GetPhysical` (C4Object.cpp:2118-2134): the trained
    /// per-object physicals when present, else the definition's `[Physical]`
    /// section. (Temporary physicals and the fair-crew info path need the
    /// C4ObjectInfo model.)
    fn object_physical(&self, idx: usize) -> PhysicalInfo {
        self.objects[idx].physical_override.unwrap_or_else(|| {
            self.definitions
                .get(&self.objects[idx].definition_id)
                .map(|definition| *definition.physical())
                .unwrap_or_default()
        })
    }

    /// `C4Object::TrainPhysical` (C4Object.cpp:2136-2146) against the
    /// per-object physicals, cloned from the definition on first training
    /// (the permanent info physicals need the info model).
    fn train_physical(
        &mut self,
        idx: usize,
        field: fn(&mut PhysicalInfo) -> &mut i32,
        train_by: i32,
        max_train: i32,
    ) {
        let base = self.object_physical(idx);
        let object = &mut self.objects[idx];
        let physical = object.physical_override.get_or_insert(base);
        PhysicalInfo::train_value(field(physical), train_by, max_train);
    }

    /// Engine-side `C4Object::DoEnergy` slice (C4Object.cpp:1345-1365) in the
    /// engine's percent-point energy units: clamp between zero and the
    /// physical Energy ceiling (scaled from the 0..C4MaxPhysical range to
    /// percent points), track the last energy-loss cause (C4Object.cpp:1353),
    /// and assign death when an alive object's energy first reaches zero
    /// (C4Object.cpp:1363). Zero-physical definitions keep the unclamped
    /// legacy ceiling so physical-less fixtures behave as before.
    fn change_object_energy(&mut self, idx: usize, change: i32, caused_by: i32) {
        let max_energy = self.object_physical(idx).energy / (C4_MAX_PHYSICAL / 100);
        let was_zero = {
            let object = &mut self.objects[idx];
            if change < 0 {
                object.last_energy_loss_cause = caused_by;
            }
            let was_zero = object.state.energy == 0;
            let mut energy = object.state.energy.saturating_add(change).max(0);
            if max_energy > 0 {
                energy = energy.min(max_energy);
            }
            object.state.energy = energy;
            was_zero
        };
        if self.objects[idx].state.alive && self.objects[idx].state.energy == 0 && !was_zero {
            let _ = self.assign_death(idx, false);
        }
    }

    /// `C4Object::AssignDeath` core (C4Object.cpp:1137-1177): alive objects
    /// only; set the "Dead" action, clear commands, eject contents at the
    /// object's position, run the Death script callback with the death
    /// causing player. Still open: effect ClearAll with the revival abort,
    /// player crew/cursor/view cleanup, Info death counters.
    fn assign_death(&mut self, idx: usize, _forced: bool) -> Result<(), EngineError> {
        if !self.objects[idx].state.alive {
            return Ok(());
        }
        let death_causing_player = self.objects[idx].last_energy_loss_cause;
        self.objects[idx].state.alive = false;
        // SetActionByName("Dead") (C4Object.cpp:1153)
        let object_id = self.objects[idx].id;
        self.force_action_by_name(idx, "Dead");
        // ClearCommands (C4Object.cpp:1157)
        self.objects[idx].command_queue.clear();
        // Lose contents (C4Object.cpp:1165)
        let contents = std::mem::take(&mut self.objects[idx].state.contents);
        let position = self.objects[idx].state.position;
        for content_id in contents {
            let update = ObjectUpdate::new()
                .clear_container()
                .with_position(position);
            let _ = self.apply_object_update(content_id, update);
        }
        // Engine script call (C4Object.cpp:1173)
        if let Some(idx) = self.find_object_index(object_id) {
            let _ = self.call_object_function(
                idx,
                "Death",
                vec![Value::Int(death_causing_player)],
            )?;
        }
        Ok(())
    }

    /// Minimal `C4Object::ChangeDef` (C4Object.cpp:1180-1228): swap the
    /// definition, reset the action to the new library's default, refresh
    /// the shape template and vertices from the new definition. Graphics,
    /// mass, color, and effect-pointer updates are still open.
    fn change_object_def(&mut self, idx: usize, new_def: &str) {
        let Some(definition) = self.definitions.get(new_def) else {
            return;
        };
        let action_state = definition.default_action_state();
        let vertices = definition.shape_vertices().to_vec();
        let template = ObjectShapeTemplate::new(
            vertices.clone(),
            definition.shape_rect(),
            definition.stretch_growth(),
            definition.rotateable(),
        );
        let category = definition.category();
        let rotateable = definition.rotateable();
        let material_capacity = self.materials.len();
        let object = &mut self.objects[idx];
        object.definition_id = new_def.to_string();
        object.state.action = action_state;
        object.state.category = category;
        object.state.vertices = vertices;
        object.shape_template = template;
        object.own_shape_vertices = None;
        // Non-rotateable defs reset rotation (C4Object.cpp:1211)
        if rotateable == 0 {
            object.state.rotation = 0;
            object.fixed_rotation = C4Fixed::ZERO;
            object.rotation_velocity = C4Fixed::ZERO;
        }
        object.ensure_material_capacity(material_capacity);
    }

    /// `SetActionByName` for engine-forced transitions (Dead, etc.).
    fn force_action_by_name(&mut self, idx: usize, action: &str) {
        let definition_id = self.objects[idx].definition_id.clone();
        let Some(library) = self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.action_library().clone())
        else {
            return;
        };
        if !library.contains(action) {
            return;
        }
        let previous = self.objects[idx].state.action.clone();
        let update = ActionUpdate {
            name: Some(action.to_string()),
            phase: Some(0),
            ticks: Some(0),
            force: true,
            data: None,
            target: Some(None),
            target2: Some(None),
        };
        let object = &mut self.objects[idx];
        let result = object
            .state
            .action
            .apply_update_with_library(&update, &library);
        if matches!(result, ActionUpdateResult::Applied)
            && previous.name != object.state.action.name
        {
            object.record_action_event(previous, ActionTransitionKind::Forced);
        }
    }

    /// `C4Object::Fling` (C4Object.cpp:1612-1625) without fAddSpeed: try the
    /// Tumble action, then Jump (ObjectActionTumble/Jump,
    /// C4ObjectCom.cpp:48-80), else set the velocity directly.
    fn fling_object(&mut self, idx: usize, txdir: C4Fixed, tydir: C4Fixed) {
        let definition_id = self.objects[idx].definition_id.clone();
        let library = self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.action_library().clone());
        if let Some(library) = library {
            for action in ["Tumble", "Jump"] {
                if !library.contains(action) {
                    continue;
                }
                let previous = self.objects[idx].state.action.clone();
                let update = ActionUpdate {
                    name: Some(action.to_string()),
                    phase: Some(0),
                    ticks: Some(0),
                    force: true,
                    data: None,
                    target: Some(None),
                    target2: Some(None),
                };
                let object = &mut self.objects[idx];
                let result = object
                    .state
                    .action
                    .apply_update_with_library(&update, &library);
                if matches!(result, ActionUpdateResult::Applied) {
                    if previous.name != object.state.action.name {
                        object.record_action_event(previous, ActionTransitionKind::Forced);
                    }
                    // Tumble also turns the object (SetDir, C4ObjectCom.cpp:77)
                    if action == "Tumble" {
                        object.state.direction = if txdir < C4Fixed::ZERO {
                            Direction::Left
                        } else {
                            Direction::Right
                        };
                    }
                    object.fixed_velocity = FixedVec2::new(txdir, tydir);
                    object.refresh_velocity_from_fixed();
                    return;
                }
            }
        }
        let object = &mut self.objects[idx];
        object.fixed_velocity = FixedVec2::new(txdir, tydir);
        object.refresh_velocity_from_fixed();
    }

    fn apply_landscape_temperature_conversions(&mut self) {
        if self.materials.is_empty() {
            return;
        }
        if let Some(landscape) = self.landscape.as_mut() {
            let environment = self.environment;
            let frame = self.frame;
            landscape.apply_temperature_conversions(&self.materials, &environment, frame);
        }
    }

    fn next_random_i32(&mut self) -> i32 {
        self.rng.random(i32::MAX)
    }

    fn next_object_id(&mut self) -> ObjectId {
        let id = self.next_object_id;
        self.next_object_id += 1;
        ObjectId::new(id)
    }

    fn find_object_index(&self, id: ObjectId) -> Option<usize> {
        self.objects.iter().position(|object| object.id == id)
    }

    fn layer_movement_bounds_for(&self, index: usize) -> Option<LayerMovementBounds> {
        let layer_id = self.objects.get(index)?.state.layer?;
        let layer = self.objects.iter().find(|object| object.id == layer_id)?;
        let definition = self.definitions.get(&layer.definition_id)?;
        Some(LayerMovementBounds {
            position: layer.position_pixels(),
            shape_rect: definition.shape_rect()?,
            border_bound: definition.border_bound(),
        })
    }

    fn solid_masks_for_movement(&self) -> Vec<SolidMaskRect> {
        let mut masks = Vec::new();
        for object in &self.objects {
            if object.destroyed
                || matches!(object.state.status, ObjectStatus::Deleted)
                || object.state.container.is_some()
                || object.state.construction < FULL_CON
                || object.state.rotation != 0
            {
                continue;
            }
            let Some(definition) = self.definitions.get(&object.definition_id) else {
                continue;
            };
            let Some(mask) = definition.solid_mask() else {
                continue;
            };
            let mask_pixels = if let Some(image) = definition.sprite_image.as_ref() {
                let image_width = image.width as i32;
                let image_height = image.height as i32;
                if mask.x < 0
                    || mask.y < 0
                    || mask.x.saturating_add(mask.width) > image_width
                    || mask.y.saturating_add(mask.height) > image_height
                {
                    continue;
                }
                let source = image.pixels.as_ref();
                let stride = image.width as usize * 4;
                let mut pixels = Vec::with_capacity((mask.width * mask.height) as usize);
                for y in 0..mask.height {
                    let source_y = (mask.y + y) as usize;
                    for x in 0..mask.width {
                        let source_x = (mask.x + x) as usize;
                        let alpha_index = source_y * stride + source_x * 4 + 3;
                        pixels.push(u8::from(source.get(alpha_index).copied().unwrap_or(0) != 0));
                    }
                }
                Some(pixels)
            } else {
                None
            };
            let shape_offset = definition
                .shape_rect()
                .map(|shape| Vector2::new(shape.x, shape.y))
                .unwrap_or(Vector2::ZERO);
            let position = object.position_pixels();
            masks.push(SolidMaskRect {
                object_id: object.id,
                x: position.x + shape_offset.x + mask.target_x,
                y: position.y + shape_offset.y + mask.target_y,
                width: mask.width,
                height: mask.height,
                pixels: mask_pixels,
            });
        }
        masks
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

    fn object_ocf_at_index(&self, index: usize) -> u32 {
        let object = &self.objects[index];
        let ocf = self
            .definitions
            .get(&object.definition_id)
            .map(|definition| definition.compute_ocf(&object.state))
            .unwrap_or_else(|| {
                crate::ocf::compute(
                    OCF_NORMAL,
                    object.state.crew_member,
                    object.state.alive,
                    object.state.status,
                    object.state.container.is_some(),
                    object.state.construction,
                )
            });
        // HitSpeeds from the fixed speed |xdir| + |ydir| (SetOCF,
        // C4Object.cpp:588-592)
        ocf | movement_hit_speed_flags(object.fixed_velocity)
    }

    fn object_has_ocf(&self, index: usize, mask: u32) -> bool {
        self.object_ocf_at_index(index) & mask != 0
    }

    fn find_nearby_object_with_mask<F>(
        &self,
        origin_id: ObjectId,
        origin_pos: Vector2,
        mask: u32,
        radius: i32,
        mut filter: F,
    ) -> Option<(usize, ObjectId)>
    where
        F: FnMut(&Object) -> bool,
    {
        if radius <= 0 {
            return None;
        }
        let radius_sq = i64::from(radius) * i64::from(radius);
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.id != origin_id)
            .filter_map(|(index, object)| {
                if !self.object_has_ocf(index, mask) || !filter(object) {
                    return None;
                }
                let dx = i64::from(object.state.position.x - origin_pos.x);
                let dy = i64::from(object.state.position.y - origin_pos.y);
                let distance_sq = dx * dx + dy * dy;
                if distance_sq <= radius_sq {
                    Some((index, object.id, distance_sq))
                } else {
                    None
                }
            })
            .min_by_key(|(_, _, distance_sq)| *distance_sq)
            .map(|(index, id, _)| (index, id))
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
                }

                self.objects[object_index].state.container = Some(container_id);
            }
            None => {
                self.objects[object_index].state.container = None;
            }
        }

        Ok(())
    }

    fn apply_command_event(&mut self, event: CommandEvent) -> Result<(), EngineError> {
        match event {
            CommandEvent::ApplyObjectUpdate { object_id, update } => {
                self.apply_object_update(object_id, update)?;
            }
            CommandEvent::ControlCommandAcquire {
                caller,
                target,
                range_x,
                range_y,
                ignore_container,
                definition_id,
            } => {
                let result = self.call_control_command_acquire(
                    caller,
                    target,
                    range_x,
                    range_y,
                    ignore_container,
                    &definition_id,
                )?;
                self.set_acquire_script_result(caller, result)?;
            }
            CommandEvent::SpawnObject {
                definition_id,
                owner,
                position,
                container,
                construction,
            } => {
                if !self.definitions.contains_key(definition_id.as_str()) {
                    return Err(EngineError::UnknownDefinition(definition_id));
                }
                let mut config = SpawnConfig::new(definition_id)
                    .with_position(position)
                    .with_owner(owner);
                if let Some(value) = construction {
                    config = config.with_construction(value);
                }
                if let Some(container_id) = container {
                    config = config.with_container(container_id);
                }
                let _ = self.spawn_object(config)?;
            }
            CommandEvent::CallObjectFunction {
                object_id,
                function,
                caller,
                tx,
                ty,
                target2,
                on_result,
            } => {
                let Some(index) = self.find_object_index(object_id) else {
                    return Err(EngineError::UnknownObject(object_id));
                };
                let mut args = Vec::new();
                args.push(object_reference_value(caller));
                args.push(tx.map(Value::Int).unwrap_or(Value::Nil));
                let ty_value = Value::Int(ty.unwrap_or(0));
                args.push(ty_value);
                let target2_value = target2.map(object_reference_value).unwrap_or(Value::Nil);
                args.push(target2_value);
                let value = self.call_object_function(index, &function, args)?;
                if let Some(action) = on_result {
                    self.apply_call_result(action, caller, value.as_bool())?;
                }
            }
            CommandEvent::OpenMenu(request) => {
                if self.find_object_index(request.crew_id).is_some() {
                    self.pending_menu_requests.push(request);
                }
            }
            CommandEvent::AdjustPlayerHomeBaseMaterial {
                player_id,
                definition_id,
                delta,
            } => {
                let _ = self.adjust_player_home_base_material(player_id, definition_id, delta)?;
            }
            CommandEvent::AdjustPlayerWealth { player_id, delta } => {
                let _ = self.adjust_player_wealth(player_id, delta)?;
            }
        }
        Ok(())
    }

    fn apply_call_result(
        &mut self,
        action: CallResultAction,
        caller: ObjectId,
        result: bool,
    ) -> Result<(), EngineError> {
        match action {
            CallResultAction::CompleteCommandOnFalse { command } => {
                if !result {
                    self.complete_command(caller, command)?;
                }
            }
            CallResultAction::CompleteCommandOnTrue { command } => {
                if result {
                    self.complete_command(caller, command)?;
                }
            }
        }
        Ok(())
    }

    fn complete_command(
        &mut self,
        object_id: ObjectId,
        command: CommandId,
    ) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Err(EngineError::UnknownObject(object_id));
        };
        let object = self
            .objects
            .get_mut(index)
            .ok_or(EngineError::UnknownObject(object_id))?;
        object.commands.complete_front_if(command);
        Ok(())
    }

    fn set_acquire_script_result(
        &mut self,
        object_id: ObjectId,
        result: AcquireScriptResult,
    ) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        if let Some(object) = self.objects.get_mut(index) {
            let _ = object.commands.set_acquire_script_result(result);
        }
        Ok(())
    }

    fn call_control_command_acquire(
        &mut self,
        caller: ObjectId,
        target: Option<ObjectId>,
        range_x: i32,
        range_y: i32,
        ignore_container: Option<ObjectId>,
        definition_id: &str,
    ) -> Result<AcquireScriptResult, EngineError> {
        let Some(index) = self.find_object_index(caller) else {
            return Ok(AcquireScriptResult::Continue);
        };
        let mut args = Vec::new();
        args.push(target.map(object_reference_value).unwrap_or(Value::Nil));
        args.push(Value::Int(range_x));
        args.push(Value::Int(range_y));
        args.push(
            ignore_container
                .map(object_reference_value)
                .unwrap_or(Value::Nil),
        );
        let definition_value = definition_id_to_c4id(definition_id)
            .map(Value::Int)
            .unwrap_or_else(|| Value::String(definition_id.to_string()));
        args.push(definition_value);

        let value = self.call_object_function(index, "~ControlCommandAcquire", args)?;
        let code = match value {
            Value::Int(code) => Some(code),
            Value::Bool(flag) => Some(if flag { 1 } else { 0 }),
            _ => None,
        };

        Ok(match code.and_then(AcquireScriptResult::from_code) {
            Some(result) => result,
            None => AcquireScriptResult::Continue,
        })
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

        let mut spawn_definitions: Vec<DefinitionId> = Vec::new();

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
                object.set_material_content(material.id(), 0);
                spawn_definitions.push(definition_id.to_string());
            }
        }

        let spawn_position = Vector2::new(position.x, bottom);
        for definition_id in spawn_definitions {
            if !self.definitions.contains_key(&definition_id) {
                continue;
            }
            let rotation = self.rng.gen_range(0..360);
            let config = SpawnConfig::new(definition_id)
                .with_position(spawn_position)
                .with_owner(owner)
                .with_rotation(rotation);
            let _ = self.spawn_object(config);
        }
    }

    fn apply_landscape_operations(&mut self, operations: Vec<LandscapeOperation>) {
        if operations.is_empty() {
            return;
        }
        for operation in operations {
            match operation {
                LandscapeOperation::DigCircle {
                    center,
                    radius,
                    requested,
                    by_object,
                } => self.execute_dig_circle_operation(center, radius, requested, by_object),
                LandscapeOperation::DigRect {
                    origin,
                    width,
                    height,
                    requested,
                    by_object,
                } => self.execute_dig_rect_operation(origin, width, height, requested, by_object),
                LandscapeOperation::BlastCircle {
                    center,
                    radius,
                    controller,
                } => self.execute_blast_circle_operation(center, radius, controller),
                LandscapeOperation::ShakeCircle { center, radius } => {
                    self.execute_shake_circle_operation(center, radius)
                }
            }
        }
    }

    fn execute_dig_circle_operation(
        &mut self,
        center: Vector2,
        radius: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    ) {
        if radius <= 0 {
            return;
        }
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };
        let mut removal_counts: HashMap<MaterialId, i32> = HashMap::new();
        let width = landscape.width() as i32;
        let radius_sq = i64::from(radius) * i64::from(radius);
        for dx in -radius..=radius {
            let column = center.x.saturating_add(dx);
            if column < 0 || column >= width {
                continue;
            }
            let dx_sq = i64::from(dx) * i64::from(dx);
            if dx_sq > radius_sq {
                continue;
            }
            let remaining = radius_sq - dx_sq;
            if remaining < 0 {
                continue;
            }
            let vertical = (remaining as f64).sqrt().floor() as i32;
            let target = center.y.saturating_add(vertical);
            if let Some((material_id, removed)) =
                Self::dig_column(&self.materials, landscape, column, target)
            {
                removal_counts
                    .entry(material_id)
                    .and_modify(|value| *value = value.saturating_add(removed))
                    .or_insert(removed);
            }
        }
        self.apply_dig_removal_counts(removal_counts, requested, by_object);
    }

    fn execute_dig_rect_operation(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    ) {
        if width <= 0 || height <= 0 {
            return;
        }
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };
        let mut removal_counts: HashMap<MaterialId, i32> = HashMap::new();
        let landscape_width = landscape.width() as i32;
        let bottom = origin.y.saturating_add(height);
        for offset in 0..width {
            let column = origin.x.saturating_add(offset);
            if column < 0 || column >= landscape_width {
                continue;
            }
            if let Some((material_id, removed)) =
                Self::dig_column(&self.materials, landscape, column, bottom)
            {
                removal_counts
                    .entry(material_id)
                    .and_modify(|value| *value = value.saturating_add(removed))
                    .or_insert(removed);
            }
        }
        self.apply_dig_removal_counts(removal_counts, requested, by_object);
    }

    fn apply_dig_removal_counts(
        &mut self,
        removal_counts: HashMap<MaterialId, i32>,
        requested: bool,
        by_object: Option<ObjectId>,
    ) {
        if removal_counts.is_empty() {
            return;
        }
        let Some(object_id) = by_object else {
            return;
        };
        let Some(object_index) = self.find_object_index(object_id) else {
            return;
        };
        if self.materials.is_empty() {
            return;
        }
        {
            let object = &mut self.objects[object_index];
            object.ensure_material_capacity(self.materials.len());
            for (material_id, removed) in &removal_counts {
                object.add_material_content(*material_id, *removed);
            }
        }
        self.process_dig_material_conversions(object_index, requested);
    }

    fn execute_blast_circle_operation(
        &mut self,
        center: Vector2,
        radius: i32,
        controller: Option<i32>,
    ) {
        if radius <= 0 {
            return;
        }
        let _ = self.blast_circle(center, radius, controller);
    }

    fn execute_shake_circle_operation(&mut self, center: Vector2, radius: i32) {
        if radius <= 0 {
            return;
        }
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };
        if self.materials.is_empty() {
            return;
        }
        let width = landscape.width() as i32;
        if width <= 0 {
            return;
        }
        let radius_sq = i64::from(radius) * i64::from(radius);
        for dx in -radius..=radius {
            let column = center.x.saturating_add(dx);
            if column < 0 || column >= width {
                continue;
            }
            let dx_sq = i64::from(dx) * i64::from(dx);
            if dx_sq > radius_sq {
                continue;
            }
            let remaining = radius_sq - dx_sq;
            if remaining < 0 {
                continue;
            }
            let vertical = (remaining as f64).sqrt().floor() as i32;
            let mut target_height = center.y.saturating_add(vertical);
            let previous_height = match landscape.surface_height(column) {
                Some(height) => height,
                None => continue,
            };
            if target_height <= previous_height {
                let distance = previous_height.saturating_sub(target_height);
                if distance > radius {
                    continue;
                }
                target_height = previous_height.saturating_add(1);
            }
            let Some((material_id, removed)) =
                Self::dig_column(&self.materials, landscape, column, target_height)
            else {
                continue;
            };
            if removed <= 0 {
                continue;
            }
            let count = match usize::try_from(removed) {
                Ok(count) if count > 0 => count,
                _ => continue,
            };
            // Freed pixels become zero-velocity PXS at their integer
            // positions, like DigFreePix → PXS.Create (C4Landscape.cpp:947-954).
            for offset in 0..count {
                self.pxs_system.create(
                    material_id,
                    itofix(column),
                    itofix(previous_height.saturating_add(offset as i32)),
                    C4Fixed::ZERO,
                    C4Fixed::ZERO,
                );
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
                    // BlastFree → PXS.Cast(mat, count, tx, ty, 60)
                    // (C4Landscape.cpp:1075-1078)
                    let pxs_count = (*removed / ratio).max(0);
                    self.pxs_system.cast(
                        &mut self.rng,
                        material_id_value,
                        pxs_count,
                        center.x,
                        center.y,
                        60,
                    );
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
                    let rotation = self.rng.gen_range(0..360);
                    let velocity =
                        Vector2::new(self.rng.gen_range(-30..=30), self.rng.gen_range(-40..=20));
                    spawn_requests.push(
                        SpawnConfig::new(definition_id.clone())
                            .with_position(center)
                            .with_velocity(velocity)
                            .with_rotation(rotation)
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

    /// Object origin for the particle Attach offset (C4Particles.cpp:404-408
    /// subtracts the target object's position when the def has Attach set).
    fn particle_attach_origin(&self, layer: &ParticleLayer) -> Option<(i32, i32)> {
        match layer {
            ParticleLayer::Global => None,
            ParticleLayer::ObjectFront(id) | ParticleLayer::ObjectBack(id) => {
                self.find_object_index(*id).map(|index| {
                    let object = &self.objects[index];
                    (
                        object.fixed_position.int_x(),
                        object.fixed_position.int_y(),
                    )
                })
            }
        }
    }

    fn apply_particle_commands(&mut self, commands: Vec<ParticleCommand>) {
        if commands.is_empty() {
            return;
        }
        for command in commands {
            match command {
                ParticleCommand::Create(config) => {
                    // Def-based path: full C4ParticleSystem::Create semantics.
                    // Def-less names keep the legacy fixture particle.
                    if self.particle_system.get_def(&config.definition_id).is_some() {
                        let attach_origin = self.particle_attach_origin(&config.layer);
                        self.particle_system.create(
                            &config.definition_id.clone(),
                            config.position.x,
                            config.position.y,
                            config.velocity.x,
                            config.velocity.y,
                            config.parameter_a,
                            config.parameter_b,
                            config.layer,
                            attach_origin,
                        );
                    } else {
                        self.particles.push(ActiveParticle::from_config(config));
                    }
                }
                ParticleCommand::Cast {
                    definition_id,
                    amount,
                    x,
                    y,
                    level,
                    a0,
                    b0,
                    a1,
                    b1,
                    layer,
                } => {
                    let attach_origin = self.particle_attach_origin(&layer);
                    self.particle_system.cast(
                        &definition_id, amount, x, y, level, a0, b0, a1, b1, layer,
                        attach_origin,
                    );
                }
                ParticleCommand::Push {
                    definition_id,
                    dxdir,
                    dydir,
                } => {
                    self.particle_system
                        .push(definition_id.as_deref(), dxdir, dydir);
                }
                ParticleCommand::Clear {
                    definition_id,
                    scope,
                } => {
                    self.particle_system
                        .remove(definition_id.as_deref(), &scope);
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

    /// `C4PXSSystem::Execute` (C4PXS.cpp:212-234): free empty chunks, then
    /// run every live PXS in chunk-major slot order.
    fn tick_pxs(&mut self) {
        self.pxs_system.free_empty_chunks();
        for chunk in 0..pxs::PXS_MAX_CHUNK {
            if !self.pxs_system.chunk_allocated(chunk) {
                continue;
            }
            for slot in 0..pxs::PXS_CHUNK_SIZE {
                let Some(pixel) = self.pxs_system.take_slot(chunk, slot) else {
                    continue;
                };
                match self.execute_pxs(pixel) {
                    Some(updated) => self.pxs_system.put_slot(chunk, slot, updated),
                    None => self.pxs_system.release_slot(chunk),
                }
            }
        }
    }

    fn landscape_material(&self, x: i32, y: i32) -> Option<MaterialId> {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.material_at(x, y))
    }

    /// `C4PXS::Execute` (C4PXS.cpp:28-127). Returns the surviving PXS, or
    /// `None` when it deactivates.
    fn execute_pxs(&mut self, mut pixel: pxs::Pxs) -> Option<pxs::Pxs> {
        // Safety (C4PXS.cpp:40-43)
        let Some(material) = self.materials.get_by_id(pixel.mat) else {
            return None;
        };
        let density = material.density();
        let wind_drift_param = material.wind_drift();
        // Out of bounds (C4PXS.cpp:45-49)
        let (back_wdt, back_hgt) = self
            .landscape
            .as_ref()
            .map(|landscape| (landscape.width() as i32, landscape.estimated_height()))
            .unwrap_or((0, 0));
        if pixel.x < C4Fixed::ZERO
            || pixel.x >= itofix(back_wdt)
            || pixel.y < itofix(-10)
            || pixel.y >= itofix(back_hgt)
        {
            return None;
        }
        // Material conversion: meePXSPos check before movement (C4PXS.cpp:51-57)
        let mut ix = fixtoi(pixel.x);
        let mut iy = fixtoi(pixel.y);
        let inmat = self.landscape_material(ix, iy);
        let reaction =
            self.materials
                .reaction_for_event(Some(pixel.mat), inmat, MaterialInteractionEvent::PxsPos);
        if !matches!(reaction, MaterialReactionKind::None) {
            // C++ passes nullptr for pfPosChanged at the PXSPos event; the
            // landscape position equals the PXS position here (C4PXS.cpp:55).
            let (ls_x, ls_y) = (ix, iy);
            let mut pos_changed = false;
            if self.execute_pxs_reaction(
                reaction,
                &mut ix,
                &mut iy,
                ls_x,
                ls_y,
                &mut pixel,
                inmat,
                MaterialInteractionEvent::PxsPos,
                &mut pos_changed,
            ) {
                return None;
            }
        }
        // Gravity (C4PXS.cpp:60)
        pixel.ydir += self.physics.gravity_as_c4fixed();
        // Free fall: wind drift with synced jitter (C4PXS.cpp:62-74). The
        // Random(1200) draws are unconditional in free fall; WindDrift only
        // scales the result. GBackWind(x, y) is approximated by the global
        // wind force (the Rust environment model is not position-dependent).
        let below_density = self
            .landscape
            .as_ref()
            .map(|landscape| landscape.density_at(ix, iy + 1, &self.materials))
            .unwrap_or(0);
        if below_density < density {
            let wind = self.environment.wind_force(self.frame);
            let txdir = itofix_prec(wind, 15) + fixed256(self.rng.random(1200) - 600);
            let tydir = fixed256(self.rng.random(1200) - 600);
            let wind_drift = (wind_drift_param - 20).max(0);
            // WindDrift_Factor = itofix(1, 800) (C4PXS.cpp:26)
            let factor = itofix_prec(1, 800);
            pixel.xdir += (txdir - pixel.xdir) * wind_drift * factor;
            pixel.ydir += (tydir - pixel.ydir) * wind_drift * factor;
        }
        // Target position (C4PXS.cpp:76-81)
        let ctcox = pixel.x + pixel.xdir;
        let ctcoy = pixel.y + pixel.ydir;
        let ito_x = fixtoi(ctcox);
        let ito_y = fixtoi(ctcoy);
        // In bounds + free path → move (C4PXS.cpp:83-89)
        // Inside<int32_t>(iToX, 0, GBackWdt - 1) / (iToY, 0, GBackHgt - 1)
        if ito_x >= 0
            && ito_x < back_wdt
            && ito_y >= 0
            && ito_y < back_hgt
            && self
                .landscape
                .as_ref()
                .map(|landscape| landscape.path_free(ix, iy, ito_x, ito_y, &self.materials))
                .unwrap_or(false)
        {
            pixel.x = ctcox;
            pixel.y = ctcoy;
            return Some(pixel);
        }
        // Step toward the target (C4PXS.cpp:91-117), do-while
        loop {
            let in_x = ix + (ito_x - ix).signum();
            let in_y = iy + (ito_y - iy).signum();
            let inmat = self.landscape_material(in_x, in_y);
            let reaction = self.materials.reaction_for_event(
                Some(pixel.mat),
                inmat,
                MaterialInteractionEvent::PxsMove,
            );
            if !matches!(reaction, MaterialReactionKind::None) {
                let mut pos_changed = false;
                if self.execute_pxs_reaction(
                    reaction,
                    &mut ix,
                    &mut iy,
                    in_x,
                    in_y,
                    &mut pixel,
                    inmat,
                    MaterialInteractionEvent::PxsMove,
                    &mut pos_changed,
                ) {
                    // destructive contact
                    return None;
                }
                if pos_changed {
                    // speed or position changed: stop moving for now
                    pixel.x = itofix(ix);
                    pixel.y = itofix(iy);
                    return Some(pixel);
                }
                // reaction did nothing — continue movement
            }
            ix = in_x;
            iy = in_y;
            if ix == ito_x && iy == ito_y {
                break;
            }
        }
        // No contact: free movement (C4PXS.cpp:119-120)
        pixel.x = ctcox;
        pixel.y = ctcoy;
        Some(pixel)
    }

    /// Reaction proc dispatch for the PXS events, mirroring the mrf*
    /// functions (C4Material.cpp:626-798). Returns true when the PXS dies
    /// (the C++ procs' return value).
    #[allow(clippy::too_many_arguments)]
    fn execute_pxs_reaction(
        &mut self,
        reaction: MaterialReactionKind,
        x: &mut i32,
        y: &mut i32,
        ls_x: i32,
        ls_y: i32,
        pixel: &mut pxs::Pxs,
        ls_mat: Option<MaterialId>,
        event: MaterialInteractionEvent,
        pos_changed: &mut bool,
    ) -> bool {
        match reaction {
            MaterialReactionKind::None => false,
            // mrfConvert (C4Material.cpp:626-661)
            MaterialReactionKind::Convert { target, depth } => {
                if event != MaterialInteractionEvent::PxsPos {
                    // hardcoded InMatConvert has no collision proc
                    // (C4Material.cpp:631-633)
                    return false;
                }
                // Check depth (C4Material.cpp:638-650)
                let depth = depth.unwrap_or(0);
                if depth != 0 && self.landscape_material(*x, *y - depth) != ls_mat {
                    return false;
                }
                match target.filter(|id| self.materials.get_by_id(*id).is_some()) {
                    Some(target) => {
                        pixel.mat = target;
                        pixel.xdir = C4Fixed::ZERO;
                        pixel.ydir = C4Fixed::ZERO;
                        *pos_changed = true;
                        false
                    }
                    // Convert failure (target not loaded or sky): kill pix
                    None => true,
                }
            }
            // mrfPoof (C4Material.cpp:663-689)
            MaterialReactionKind::Poof => {
                if event == MaterialInteractionEvent::PxsMove
                    && !self.mrf_insert_check(
                        x,
                        y,
                        &mut pixel.xdir,
                        &mut pixel.ydir,
                        pixel.mat,
                        ls_mat,
                        pos_changed,
                    )
                {
                    // either splash or slide prevented interaction
                    return false;
                }
                // Always kill both landscape and PXS mat
                if let Some(landscape) = self.landscape.as_mut() {
                    let _ = landscape.extract_material_at(ls_x, ls_y);
                }
                if self.rng.rnd3() == 0 {
                    self.spawn_smoke(*x, *y, 3);
                }
                // !Rnd3() → "Pshshsh" sound; the draw is sync-relevant.
                let _ = self.rng.rnd3();
                true
            }
            // mrfCorrode (C4Material.cpp:691-745)
            MaterialReactionKind::Corrode {
                corrosive_strength,
                corrode_resistance,
                corrosion_probability,
            } => {
                if event != MaterialInteractionEvent::PxsMove {
                    // No corrosion before movement (C4Material.cpp:696-698)
                    return false;
                }
                if !self.mrf_insert_check(
                    x,
                    y,
                    &mut pixel.xdir,
                    &mut pixel.ydir,
                    pixel.mat,
                    ls_mat,
                    pos_changed,
                ) {
                    return false;
                }
                let corroded = evaluate_corrosion(
                    corrosive_strength,
                    corrode_resistance,
                    corrosion_probability,
                    &mut self.rng,
                );
                if corroded {
                    if let Some(landscape) = self.landscape.as_mut() {
                        let _ = landscape.extract_material_at(ls_x, ls_y);
                    }
                    // effect draws (C4Material.cpp:734-735): 1/5 smoke with a
                    // Random(3) size component, then the 1/20 sound draw
                    if self.rng.random(5) == 0 {
                        let level = 3 + self.rng.random(3);
                        self.spawn_smoke(*x, *y, level);
                    }
                    let _ = self.rng.random(20);
                } else if let Some(landscape) = self.landscape.as_mut() {
                    // Else: dead. Insert material here (C4Material.cpp:739)
                    landscape.insert_material_at(*x, *y, pixel.mat);
                }
                true
            }
            // mrfIncinerate (C4Material.cpp:747-771)
            MaterialReactionKind::Incinerate => {
                if event == MaterialInteractionEvent::PxsMove
                    && !self.mrf_insert_check(
                        x,
                        y,
                        &mut pixel.xdir,
                        &mut pixel.ydir,
                        pixel.mat,
                        ls_mat,
                        pos_changed,
                    )
                {
                    return false;
                }
                let can_incinerate = self
                    .landscape
                    .as_ref()
                    .map(|landscape| landscape.can_incinerate(*x, *y, &self.materials))
                    .unwrap_or(false);
                if can_incinerate && self.spawn_fire_at(*x, *y) {
                    return true;
                }
                if event == MaterialInteractionEvent::PxsMove {
                    // Else: dead. Insert material here (C4Material.cpp:765-767)
                    if let Some(landscape) = self.landscape.as_mut() {
                        landscape.insert_material_at(*x, *y, pixel.mat);
                    }
                    return true;
                }
                false
            }
            // mrfInsert (C4Material.cpp:773-798)
            MaterialReactionKind::Insert => {
                if event != MaterialInteractionEvent::PxsMove {
                    return false;
                }
                if !self.mrf_insert_check(
                    x,
                    y,
                    &mut pixel.xdir,
                    &mut pixel.ydir,
                    pixel.mat,
                    ls_mat,
                    pos_changed,
                ) {
                    // continue existing
                    return false;
                }
                // Else: dead. Insert material here (C4Material.cpp:789)
                if let Some(landscape) = self.landscape.as_mut() {
                    landscape.insert_material_at(*x, *y, pixel.mat);
                }
                true
            }
        }
    }

    /// `Smoke()` (C4Effect.cpp:859-865): create a "Smoke" particle if the def
    /// is loaded. (The FXS1 object fallback for missing particle defs is not
    /// ported.) `level/2` is integer division like the C++ call.
    fn spawn_smoke(&mut self, x: i32, y: i32, level: i32) {
        self.particle_system.create(
            "Smoke",
            x as f32,
            y as f32 - (level / 2) as f32,
            0.0,
            0.0,
            level as f32,
            0,
            ParticleLayer::Global,
            None,
        );
    }

    /// `mrfInsertCheck` (C4Material.cpp:567-610): splash/slide preamble run by
    /// the default Poof/Corrode/Incinerate/Insert reactions on the PXS-move
    /// event. Returns true when insertion may proceed; false keeps the PXS
    /// alive (splashed or sliding). Mutates pos/speed like the C++ by-ref
    /// parameters.
    #[allow(clippy::too_many_arguments)]
    fn mrf_insert_check(
        &mut self,
        x: &mut i32,
        y: &mut i32,
        xdir: &mut C4Fixed,
        ydir: &mut C4Fixed,
        pxs_mat: MaterialId,
        ls_mat: Option<MaterialId>,
        pos_changed: &mut bool,
    ) -> bool {
        // always manipulating pos/speed here (C4Material.cpp:570)
        *pos_changed = true;
        let Some(material) = self.materials.get_by_id(pxs_mat) else {
            return true;
        };
        let splash_rate = material.splash_rate();
        let incindiary = material.incindiary();
        let density = material.density();
        let max_slide = material.max_slide();

        // Rough contact? May splash (C4Material.cpp:572-579)
        if *ydir > itofix(1)
            && splash_rate != 0
            && self.rng.random(splash_rate) == 0
        {
            *ydir = -*ydir / 8;
            *xdir = *xdir / 8 + fixed100(self.rng.random(200) - 100);
            if ydir.is_nonzero() {
                return false;
            }
        }

        // Contact: Stop (C4Material.cpp:581-582)
        *ydir = C4Fixed::ZERO;

        // Incindiary mats smoke on contact even before doing their slide
        // (C4Material.cpp:584-586). Rnd3 is consumed as the call argument.
        if incindiary != 0 && self.rng.random(25) == 0 {
            let level = 4 + self.rng.rnd3();
            self.spawn_smoke(*x, *y, level);
        }

        // Move by mat path/slide (C4Material.cpp:588-607)
        let gravity_sign = self.physics.gravity_as_c4fixed().val().signum();
        let (mut slide_x, mut slide_y) = (*x, *y);
        let found_slide = self
            .landscape
            .as_ref()
            .map(|landscape| {
                landscape.find_mat_slide(
                    &mut slide_x,
                    &mut slide_y,
                    gravity_sign,
                    density,
                    max_slide,
                    &self.materials,
                )
            })
            .unwrap_or(false);
        if found_slide {
            if Some(pxs_mat) == ls_mat {
                *x = slide_x;
                *y = slide_y;
                *xdir = C4Fixed::ZERO;
                return false;
            }
            // Accelerate into the direction (C4Material.cpp:597)
            *xdir = C4Fixed::from_raw(
                (xdir.val().wrapping_mul(10) + itofix((slide_x - *x).signum()).val()) / 11,
            ) + fixed10(self.rng.random(5) - 2);
            // Slide target in range? Move there directly. (C4Material.cpp:599-604)
            if (*x - slide_x).abs() <= fixtoi(*xdir).abs() {
                *x = slide_x;
                *y = slide_y;
                if *ydir <= C4Fixed::ZERO {
                    *xdir = C4Fixed::ZERO;
                }
            }
            // Continue existance
            return false;
        }
        // insertion OK
        true
    }

    fn spawn_fire_at(&mut self, x: i32, y: i32) -> bool {
        if !self
            .landscape
            .as_ref()
            .map(|landscape| landscape.can_incinerate(x, y, &self.materials))
            .unwrap_or(false)
        {
            return false;
        }

        if !self.definitions.contains_key(FIRE_DEFINITION_ID) {
            return false;
        }

        let left = x.saturating_sub(4);
        let right = left.saturating_add(8);
        let top = y.saturating_sub(1);
        let bottom = top.saturating_add(20);

        let has_existing = self.objects.iter().any(|object| {
            if object.destroyed {
                return false;
            }
            if object.definition_id != FIRE_DEFINITION_ID {
                return false;
            }
            if !object.state.status.is_active() {
                return false;
            }
            let pos = object.state.position;
            pos.x >= left && pos.x < right && pos.y >= top && pos.y < bottom
        });

        if has_existing {
            return false;
        }

        let result = self
            .spawn_object(SpawnConfig::new(FIRE_DEFINITION_ID).with_position(Vector2::new(x, y)));
        result.is_ok()
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
        // Legacy def-less fixture particles (additive command-DSL path).
        if !self.particles.is_empty() {
            for particle in &mut self.particles {
                particle.tick();
            }
            self.particles.retain(|particle| !particle.is_expired());
        }
        // C4ParticleSystem exec: each object's Back then Front list
        // (C4Object.cpp:1071-1072), then GlobalParticles (C4Game.cpp:814).
        if self.particle_system.particles().is_empty() {
            return;
        }
        let gravity = self.physics.gravity_as_c4fixed();
        let frame_counter = self.frame as i32;
        let wind_force = self.environment.wind_force(self.frame);
        // GBackWdt/GBackHgt; the Rust landscape is a height-map model, so the
        // world height is the estimated extent rather than a pixel-map height.
        let (back_wdt, back_hgt) = self
            .landscape
            .as_ref()
            .map(|landscape| (landscape.width() as i32, landscape.estimated_height()))
            .unwrap_or((0, 0));
        let attached_ids: HashSet<ObjectId> = self
            .particle_system
            .particles()
            .iter()
            .filter_map(|particle| match particle.layer {
                ParticleLayer::ObjectFront(id) | ParticleLayer::ObjectBack(id) => Some(id),
                ParticleLayer::Global => None,
            })
            .collect();
        let targets: Vec<(ObjectId, particles::ParticleTarget)> = self
            .objects
            .iter()
            .filter(|object| attached_ids.contains(&object.id))
            .map(|object| {
                (
                    object.id,
                    particles::ParticleTarget {
                        x: object.fixed_position.int_x(),
                        y: object.fixed_position.int_y(),
                        xdir: object.fixed_velocity.x,
                        ydir: object.fixed_velocity.y,
                    },
                )
            })
            .collect();
        let landscape = self.landscape.as_ref();
        let solid = move |x: i32, y: i32| {
            landscape
                .map(|landscape| landscape.is_solid_at(x, y))
                .unwrap_or(false)
        };
        // GBackWind(x, y) is position-dependent in C++; the Rust environment
        // model only carries a global wind force (visual-only divergence).
        let wind = move |_x: i32, _y: i32| wind_force;
        let env = particles::ParticleEnv {
            gravity,
            frame_counter,
            back_wdt,
            back_hgt,
            solid: &solid,
            wind: &wind,
        };
        let mut system = std::mem::take(&mut self.particle_system);
        for (id, target) in targets {
            system.exec_layer(&ParticleLayer::ObjectBack(id), Some(target), &env);
            system.exec_layer(&ParticleLayer::ObjectFront(id), Some(target), &env);
        }
        system.exec_layer(&ParticleLayer::Global, None, &env);
        self.particle_system = system;
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
            rotation,
            energy,
            construction,
            action,
            direction,
            command_direction,
            effects,
            vertices,
            owner,
            crew_member,
            status,
            container,
            layer,
            alive,
            category,
        } = config;

        let (
            action_library,
            definition_category,
            default_action_state,
            default_crew_member,
            definition_vertices,
            definition_shape_rect,
            definition_stretch_growth,
            definition_rotateable,
        ) = {
            let definition_ref = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            (
                definition_ref.action_library().clone(),
                definition_ref.category(),
                definition_ref.default_action_state(),
                definition_ref.is_crew(),
                definition_ref.shape_vertices().to_vec(),
                definition_ref.shape_rect(),
                definition_ref.stretch_growth(),
                definition_ref.rotateable(),
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

        let owns_vertices = !vertices.is_empty();
        let shape_template = ObjectShapeTemplate::new(
            definition_vertices.clone(),
            definition_shape_rect,
            definition_stretch_growth,
            definition_rotateable,
        );
        let shape_base_vertices = if owns_vertices {
            vertices.clone()
        } else {
            definition_vertices
        };
        let initial_vertices = transformed_shape_vertices(
            &shape_base_vertices,
            construction,
            shape_template.stretch_growth,
            shape_template.rotateable,
            rotation,
        );
        let own_shape_vertices = owns_vertices.then_some(shape_base_vertices);

        let mut object = Object::new(
            id,
            definition_id.clone(),
            ObjectState {
                position,
                velocity,
                rotation: rotation.rem_euclid(360),
                energy,
                damage: 0,
                magic_energy: 0,
                magic_capacity: 0,
                construction: construction.clamp(0, FULL_CON),
                action: initial_action,
                direction,
                command_direction,
                effects: Vec::new(),
                vertices: initial_vertices,
                container: None,
                layer,
                contents: Vec::new(),
                components: HashMap::new(),
                status: status.unwrap_or_default(),
                owner,
                category: initial_category,
                crew_member: initial_crew_member,
                alive: alive.unwrap_or(true),
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                local_vars: HashMap::new(),
                on_fire: false,
                fire_phase: 0,
                fire_caused_by: OWNER_NONE,
            },
            shape_template,
            own_shape_vertices,
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

        object.clamp_velocity(&self.physics);

        let mut additional_spawns = Vec::new();

        // Call Construction() before Initialize()
        // Construction() initializes local variables that may be used in Initialize() or action callbacks
        if self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.has_construction)
            .unwrap_or(false)
        {
            let rng_state = self.rng.clone();
            let (
                CommandBatch {
                    delta,
                    spawns,
                    destroy,
                    commands,
                    command_ops,
                    effects,
                    global_effects,
                    environment,
                    physics,
                    landscape_ops,
                    particles,
                    transfer_zones,
                    audio,
                    messages,
                    player_commands,
                    trigger_game_over,
                },
                audio_state,
                new_rng,
                next_object_id,
            ) = {
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .expect("definition must exist");
                definition.call_construction(
                    &object.state,
                    id,
                    rng_state,
                    &self.global_effects,
                    self.physics,
                    self.environment,
                    self.frame,
                    self.host_world_context(),
                    self.game_over_triggered,
                    self.audio_registry.clone(),
                )?
            };
            self.rng = new_rng;
            self.next_object_id = next_object_id;
            self.audio_registry = audio_state;
            if trigger_game_over {
                self.request_game_over()?;
            }
            if let Some(update) = environment {
                update.apply(&mut self.environment);
            }
            if let Some(delta) = physics {
                self.apply_physics_delta(delta);
            }
            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }
            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }
            if destroy {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition_id.clone(),
                    function: "Construction",
                    detail: "Construction may not destroy the object".into(),
                });
            }
            let outcome = object.apply_delta(&delta, &action_library);
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
            object.clamp_velocity(&self.physics);
            if !command_ops.is_empty() {
                object.apply_command_operations(command_ops);
            }
            if !commands.is_empty() {
                object.enqueue_commands(commands);
            }
            additional_spawns.extend(spawns);
            if !audio.is_empty() {
                self.pending_audio.extend(audio);
            }
            if !messages.is_empty() {
                for command in messages {
                    self.messages.apply_command(command);
                }
            }
        }

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
                    command_ops,
                    effects,
                    global_effects,
                    environment,
                    physics,
                    landscape_ops,
                    particles,
                    transfer_zones,
                    audio,
                    messages,
                    player_commands,
                    trigger_game_over,
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
                    self.game_over_triggered,
                    self.audio_registry.clone(),
                )?
            };
            self.rng = new_rng;
            self.next_object_id = next_object_id;
            self.audio_registry = audio_state;
            if trigger_game_over {
                self.request_game_over()?;
            }
            if let Some(update) = environment {
                update.apply(&mut self.environment);
            }
            if let Some(delta) = physics {
                self.apply_physics_delta(delta);
            }
            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }
            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }
            if destroy {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition_id.clone(),
                    function: "Initialize",
                    detail: "Initialize may not destroy the object".into(),
                });
            }
            let outcome = object.apply_delta(&delta, &action_library);
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
            object.clamp_velocity(&self.physics);
            if !command_ops.is_empty() {
                object.apply_command_operations(command_ops);
            }
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
                player_commands,
                landscape_ops,
                triggered_game_over,
                audio_state,
                new_rng,
            ) = Self::run_effect_events_for_object(
                definition,
                self.game_over_triggered,
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
            if !player_commands.is_empty() {
                self.apply_player_commands(player_commands)?;
            }
            if !landscape_ops.is_empty() {
                self.apply_landscape_operations(landscape_ops);
            }
            if !audio_events.is_empty() {
                self.pending_audio.extend(audio_events);
            }
            if !event_messages.is_empty() {
                for command in event_messages {
                    self.messages.apply_command(command);
                }
            }
            if triggered_game_over {
                self.request_game_over()?;
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

        self.apply_landscape(&mut object);
        self.objects.push(object);
        let index = self.objects.len() - 1;
        self.update_sector_for_index(index);
        for (previous, new) in container_changes {
            self.apply_container_change(id, previous, new)?;
        }
        self.trigger_action_callbacks(index, None)?;
        self.update_sector_for_index(index);
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
    map.insert("construction".into(), Value::Int(state.construction));
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
    map.insert(
        "rotation".into(),
        Value::Int(snapshot.rotation.rem_euclid(360)),
    );
    map.insert("energy".into(), Value::Int(snapshot.energy));
    map.insert("construction".into(), Value::Int(snapshot.construction));
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
                    constructable: false,
                    shape: None,
                    construction_offset: 0,
                    basement: 0,
                },
            )
        })
        .collect();
    let players: HashMap<i32, PlayerState> = snapshot
        .players
        .iter()
        .map(|state| (state.id, state.clone()))
        .collect();
    let crew_selection = snapshot.crew_selection.clone();
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
                object.construction,
                object.damage,
                object.position,
                object.velocity,
                object.rotation,
                object.vertices.clone(),
                object.action.data,
                object.action.ticks,
                object.action.phase,
                object.container,
                object.draw_transform,
            )
        }),
        snapshot.landscape.clone(),
        definition_metadata,
        snapshot.transfer_zones.clone(),
        players,
        crew_selection,
        next_object_id,
        false,
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
    map.insert("game_over".into(), Value::Bool(snapshot.game_over));
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
        frame.sky_color.map(rgb_to_value).unwrap_or(Value::Nil),
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
        // C++ parity: lifecycle callbacks (Initialize/Step) have their return
        // value DISCARDED by the engine (e.g. C4Object.cpp:1483 calls
        // `Call(PSF_Initialize)` as a bare statement). Real definitions routinely
        // return an int from Initialize. The command-delta proplist is an additive
        // Rust convenience; any other return type is simply ignored, never an error.
        _ => Ok(CommandBatch::default()),
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
        Value::Object(id) => {
            if id == 0 {
                Ok(None)
            } else {
                Ok(Some(ObjectId::new(id)))
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
                "expected object, int, proplist, or nil for action.{field}, got {}",
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
    use crate::math::C4Fixed;
    use crate::rng::LcgRng;
    use crate::scenario::{ClearObjectObjective, CreateObjectObjective, ScenarioObjectives};
    use lc_resources::MaterialLibrary;
    use lc_script::Value;
    use std::collections::{HashMap, HashSet};
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

    const BASIC_OBJECT_SCRIPT: &str = r#"
    global func Initialize(state, random) { return nil; }
    global func Step(state, frame, random) { return nil; }
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

        let controller = 1;
        let before_snapshot = engine.snapshot();
        let existing_ids: HashSet<_> = before_snapshot
            .objects
            .iter()
            .map(|object| object.id)
            .collect();
        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, Some(controller))
            .expect("blast applies");
        let removed = result
            .removed_by_material
            .get(&rock)
            .copied()
            .unwrap_or_default();
        assert!(removed > 0, "expected blast to remove rock material");

        let ratio = 2;
        let expected_spawns = (removed / ratio).max(0);
        assert!(
            expected_spawns > 0,
            "expected blast to spawn objects when material is removed"
        );
        let after_snapshot = engine.snapshot();
        let new_objects: Vec<_> = after_snapshot
            .objects
            .iter()
            .filter(|object| !existing_ids.contains(&object.id))
            .collect();
        assert_eq!(
            new_objects.len() as i32,
            expected_spawns,
            "blast should spawn one object per {:?} removed pixels",
            ratio
        );

        for object in new_objects {
            assert_eq!(
                object.definition_id, "GEM0",
                "blast should spawn configured definition"
            );
            assert!(
                (-30..=30).contains(&object.velocity.x),
                "expected horizontal velocity to follow legacy FIXED10 distribution"
            );
            assert!(
                (-40..=20).contains(&object.velocity.y),
                "expected vertical velocity to follow legacy FIXED10 distribution"
            );
            assert!(
                (0..360).contains(&object.rotation),
                "expected rotation to be normalised"
            );
            assert_eq!(
                object.owner, controller,
                "expected spawned object owner to match controller"
            );
        }
        Ok(())
    }

    #[test]
    fn apply_landscape_operations_executes_blast_circle() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1
            Blast2PXSRatio=2
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(5);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(earth)));

        engine.apply_landscape_operations(vec![LandscapeOperation::BlastCircle {
            center: Vector2::new(8, 40),
            radius: 4,
            controller: Some(1),
        }]);

        let snapshot = engine.snapshot();
        assert!(
            snapshot
                .particles
                .iter()
                .any(|particle| particle.definition_id == "material/pxs/earth"),
            "blast operation should emit earth particles"
        );
        Ok(())
    }

    #[test]
    fn cross_check_hit_damages_and_flings_like_cpp() -> Result<(), EngineError> {
        // CrossCheck reverse area check, Hit branch (C4GameObjects.cpp:148,
        // 167-184): an OCF_HitSpeed2 object of category C4D_Object overlapping
        // an alive object deals "realistic" hit energy
        // fixtoi((dX²+dY²)*Mass/5), reduced to 1/3 (min 1); the victim takes
        // DoEnergy(-e/5) and is flung by (xdir*50/tmass, -|ydir/2|*50/tmass)
        // with tmass = max(victim mass, 50).
        let mut engine = Engine::with_seed(40);
        let mut victim_def = simple_definition("Clonk");
        victim_def.set_mass(100);
        engine.register_definition(victim_def)?;
        let mut rock_def = simple_definition("Rock");
        rock_def.set_category(CATEGORY_OBJECT);
        rock_def.set_mass(50);
        engine.register_definition(rock_def)?;

        let victim = engine.spawn_object(
            SpawnConfig::new("Clonk")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(100),
        )?;
        let _rock = engine.spawn_object(
            SpawnConfig::new("Rock")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(5, 0)),
        )?;

        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        let energy_before = engine.objects[victim_idx].state.energy;
        engine.cross_check(1)?;

        // dX = itofix(5): hit energy = fixtoi(itofix(25)*50/5) = 250,
        // reduced: max(250/3, 1) = 83, energy change = -(83/5) = -16.
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.energy,
            energy_before - 16,
            "hit energy applied"
        );
        // fling: xdir = itofix(5)*50/100 = itofix(2.5), ydir = 0; no
        // Tumble/Jump actions on the def → raw velocity (C4Object.cpp:1612-1625)
        assert_eq!(
            engine.objects[victim_idx].fixed_velocity.x,
            math::C4Fixed::from_raw(math::itofix(5).val() * 50 / 100),
            "flung horizontally"
        );
        assert_eq!(
            engine.objects[victim_idx].fixed_velocity.y,
            math::C4Fixed::ZERO
        );
        Ok(())
    }

    #[test]
    fn energy_loss_to_zero_assigns_death_like_cpp() -> Result<(), EngineError> {
        // C4Object::DoEnergy (C4Object.cpp:1361-1363): an alive object whose
        // energy first reaches zero dies. AssignDeath (C4Object.cpp:1137-1177)
        // sets the "Dead" action, clears commands, ejects contents at the
        // object's position, and runs the Death callback with the death
        // causing player (the last energy-loss cause).
        let mut engine = Engine::with_seed(90);
        let mut clonk_def = Definition::from_script(
            "Clonk",
            "Clonk",
            r#"
            func Death(by) { return 1; }
            "#,
        )?;
        clonk_def.set_crew_member(true);
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Dead".to_string(), ActionSpec::default());
        clonk_def.configure_actions(Some("Idle".to_string()), specs);
        engine.register_definition(clonk_def)?;
        engine.register_definition(simple_definition("Gem"))?;

        let clonk = engine.spawn_object(
            SpawnConfig::new("Clonk")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(5),
        )?;
        let gem = engine.spawn_object(
            SpawnConfig::new("Gem")
                .with_position(Vector2::new(50, 50))
                .with_container(clonk),
        )?;

        let idx = engine.find_object_index(clonk).expect("clonk exists");
        engine.change_object_energy(idx, -3, 7);
        assert!(engine.objects[idx].state.alive, "energy 2 left");
        engine.change_object_energy(idx, -2, 7);
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert!(!engine.objects[idx].state.alive, "dead at zero energy");
        assert_eq!(engine.objects[idx].state.action.name, "Dead");
        assert!(
            engine.objects[idx].state.contents.is_empty(),
            "contents lost"
        );
        let gem_idx = engine.find_object_index(gem).expect("gem exists");
        assert_eq!(engine.objects[gem_idx].state.container, None, "gem ejected");
        assert_eq!(
            engine.objects[gem_idx].state.position,
            Vector2::new(50, 50),
            "ejected at the dying object's position"
        );

        // Death is not re-assigned (already dead, C4Object.cpp:1141)
        engine.change_object_energy(idx, -1, 9);
        assert_eq!(engine.objects[idx].last_energy_loss_cause, 9);
        Ok(())
    }

    #[test]
    fn sync_check_digest_and_state_machine_match_cpp() -> Result<(), EngineError> {
        // C4ControlSyncCheck::Set (C4Control.cpp:445-468): Random3 is the
        // Rnd3 ring pointer, RandomCount the synced draw count, AllCrewPosX
        // sums fixtoi(fix_x, 100) (centipixels) over the players' crew
        // lists. C4GameControl::Ticks (C4GameControl.cpp:326-332) advances
        // ControlTick every ControlRate frames and requests a sync check
        // every SyncRate frames; old checks drop after 50 frames
        // (C4GameControl.cpp:508-522, C4SyncCheckMaxKeep).
        let mut engine = Engine::with_seed(80);
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        let mut crew_def = simple_definition("Clonk");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("Clonk")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        // give the crew sub-pixel x so the centipixel precision is visible
        let idx = engine.find_object_index(crew).expect("crew exists");
        engine.objects[idx].fixed_position.x =
            math::itofix(10) + math::C4Fixed::from_raw(math::itofix(1).val() / 4); // 10.25

        engine.tick()?; // builds crew lists
        let packet = engine.sync_check(0);
        assert_eq!(packet.random3, engine.rng.rnd3_ptr());
        assert_eq!(packet.random_count, engine.rng.count);
        assert_eq!(
            packet.crew_positions_sum,
            math::fixtoi_prec(engine.objects[idx].fixed_position.x, 100),
            "centipixel crew sum over the player's crew list"
        );
        assert_eq!(packet.object_count, 1);

        // ControlRate gating: with rate 2, ControlTick advances on even frames.
        let mut gated = Engine::with_seed(81);
        gated.control_rate = 2;
        for _ in 0..4 {
            gated.tick()?;
        }
        assert_eq!(gated.control_tick, 2, "frames 2 and 4 advance the tick");

        // SyncRate: the digest is queued on frame % 100 == 0 and pruned
        // after 50 frames.
        let mut machine = Engine::with_seed(82);
        machine.sync_rate = 10;
        for _ in 0..10 {
            machine.tick()?;
        }
        assert!(machine.get_sync_check(10).is_some(), "queued on frame 10");
        // strict cutoff (C4GameControl.cpp:519: frame < FrameCounter - 50):
        // check 10 survives the frame-60 prune and drops at the frame-70 one.
        for _ in 0..50 {
            machine.tick()?;
        }
        assert!(
            machine.get_sync_check(10).is_some(),
            "10 >= 60 - 50 keeps it at frame 60"
        );
        for _ in 0..10 {
            machine.tick()?;
        }
        assert!(
            machine.get_sync_check(10).is_none(),
            "pruned once frame - 50 exceeds it"
        );
        assert!(machine.get_sync_check(60).is_some());

        // Remote comparison (C4ControlSyncCheck::Execute, C4Control.cpp:469+):
        // matching digest → ok; tampered digest → synchronization loss.
        let local = machine.get_sync_check(60).cloned().expect("local check");
        assert!(machine.register_remote_sync_check(local.clone()));
        let mut tampered = local;
        tampered.random_count += 1;
        assert!(!machine.register_remote_sync_check(tampered));
        Ok(())
    }

    #[test]
    fn incinerate_burn_turn_to_and_contents_ejection_match_cpp() -> Result<(), EngineError> {
        // fxFireStart (C4Effect.cpp:579-594): BurnTurnTo changes the
        // definition when fire is caused; contents are ejected at the
        // object's position unless IncompleteActivity or NoBurnDecay.
        let mut engine = Engine::with_seed(95);
        let mut hut_def = simple_definition("Hut");
        hut_def.set_burn_turn_to(Some("Ruin".to_string()));
        engine.register_definition(hut_def)?;
        engine.register_definition(simple_definition("Ruin"))?;
        engine.register_definition(simple_definition("Gem"))?;

        let hut = engine.spawn_object(
            SpawnConfig::new("Hut").with_position(Vector2::new(30, 30)),
        )?;
        let gem = engine.spawn_object(
            SpawnConfig::new("Gem")
                .with_position(Vector2::new(30, 30))
                .with_container(hut),
        )?;

        let idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        let idx = engine.find_object_index(hut).expect("hut exists");
        assert_eq!(
            engine.objects[idx].definition_id, "Ruin",
            "BurnTurnTo changed the definition"
        );
        assert!(engine.objects[idx].state.on_fire);
        assert!(engine.objects[idx].state.contents.is_empty());
        let gem_idx = engine.find_object_index(gem).expect("gem exists");
        assert_eq!(engine.objects[gem_idx].state.container, None, "ejected");
        assert_eq!(engine.objects[gem_idx].state.position, Vector2::new(30, 30));

        // NoBurnDecay keeps the contents (C4Effect.cpp:588).
        let mut keeper_def = simple_definition("Chest");
        keeper_def.set_fire_properties(0, true, false);
        engine.register_definition(keeper_def)?;
        let chest = engine.spawn_object(
            SpawnConfig::new("Chest").with_position(Vector2::new(50, 30)),
        )?;
        let coin = engine.spawn_object(
            SpawnConfig::new("Gem")
                .with_position(Vector2::new(50, 30))
                .with_container(chest),
        )?;
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert!(engine.incinerate_object(chest_idx, 1, false, None)?);
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(engine.objects[chest_idx].state.contents, vec![coin]);
        Ok(())
    }

    #[test]
    fn incinerate_object_matches_cpp_start_semantics() -> Result<(), EngineError> {
        // C4Object::Incinerate (C4Object.cpp:1230-1241) + fxFireStart core
        // (C4Effect.cpp:560-641): already burning → false; dead livings don't
        // burn; in extinguishing material → no fire and NO FirePhase draw
        // (the extinguisher check precedes it); otherwise OnFire is set and
        // FirePhase = Random(MaxFirePhase) consumes one synced draw
        // (C4Effect.cpp:633-634, MaxFirePhase = 15).
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
            Extinguisher=1

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(70);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(40, 30, Some(earth)));
        engine.register_definition(simple_definition("Tree"))?;
        let tree = engine.spawn_object(
            SpawnConfig::new("Tree").with_position(Vector2::new(10, 10)),
        )?;
        let idx = engine.find_object_index(tree).expect("tree exists");

        let mut mirror = engine.rng.clone();
        let expected_phase = mirror.random(15);
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        assert!(engine.objects[idx].state.on_fire);
        assert_eq!(engine.objects[idx].state.fire_phase, expected_phase);
        assert_eq!(engine.objects[idx].state.fire_caused_by, 1);
        assert_eq!(engine.rng, mirror, "one FirePhase draw");

        // already burning → false, no draw (C4Object.cpp:1233)
        assert!(!engine.incinerate_object(idx, 2, false, None)?);
        assert_eq!(engine.rng, mirror);
        assert_eq!(engine.objects[idx].state.fire_caused_by, 1);

        // dead living → false (C4Object.cpp:1235)
        let mut dead_def = simple_definition("Corpse");
        dead_def.set_crew_member(true);
        dead_def.set_category(CATEGORY_LIVING);
        engine.register_definition(dead_def)?;
        let corpse = engine.spawn_object(
            SpawnConfig::new("Corpse")
                .with_position(Vector2::new(20, 10))
                .with_alive(false),
        )?;
        let corpse_idx = engine.find_object_index(corpse).expect("corpse exists");
        assert!(!engine.incinerate_object(corpse_idx, 1, false, None)?);
        assert!(!engine.objects[corpse_idx].state.on_fire);

        // submerged in extinguisher material → no fire, no draw
        // (C4Effect.cpp:574-583)
        if let Some(landscape) = engine.landscape.as_mut() {
            landscape.set_liquid_column(30, vec![LiquidSegment::with_material(5, 12, Some(water))]);
        }
        let soaked = engine.spawn_object(
            SpawnConfig::new("Tree").with_position(Vector2::new(30, 8)),
        )?;
        let soaked_idx = engine.find_object_index(soaked).expect("soaked exists");
        let mirror = engine.rng.clone();
        assert!(!engine.incinerate_object(soaked_idx, 1, false, None)?);
        assert!(!engine.objects[soaked_idx].state.on_fire);
        assert_eq!(engine.rng, mirror, "no draw when extinguished at start");
        Ok(())
    }

    #[test]
    fn exec_fire_burns_objects_like_cpp() -> Result<(), EngineError> {
        // C4Object::ExecFire (C4Object.cpp:766-810): every frame FirePhase
        // cycles mod 15 and Con decays by 100 raw units (unless NoBurnDecay);
        // Tick10 deals +2 damage (unless NoBurnDamage); Tick5 drains 1
        // energy; Tick5 over valid landscape material extinguishes in
        // extinguisher material and otherwise draws Random(3) for landscape
        // inflammation.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(71);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(40, 30, Some(earth)));
        engine.register_definition(simple_definition("Hut"))?;
        // in open air: the Tick5 background-material block never fires
        let hut = engine.spawn_object(
            SpawnConfig::new("Hut")
                .with_position(Vector2::new(10, 10))
                .with_energy(50),
        )?;
        let idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        let phase_after_start = engine.objects[idx].state.fire_phase;
        let con_before = engine.objects[idx].state.construction;
        let mirror = engine.rng.clone();

        // frame 1: neither Tick5 nor Tick10 — only phase + decay
        engine.exec_object_fire(idx, 1);
        assert_eq!(
            engine.objects[idx].state.fire_phase,
            (phase_after_start + 1) % 15
        );
        assert_eq!(engine.objects[idx].state.construction, con_before - 100);
        assert_eq!(engine.objects[idx].state.energy, 50);
        assert_eq!(engine.objects[idx].state.damage, 0);
        assert_eq!(engine.rng, mirror, "no draws in open air off-tick");

        // frame 5: Tick5 → energy -1 (air: no background draw)
        engine.exec_object_fire(idx, 5);
        assert_eq!(engine.objects[idx].state.energy, 49);
        // frame 10: Tick10 + Tick5 → damage +2, energy -1
        engine.exec_object_fire(idx, 10);
        assert_eq!(engine.objects[idx].state.damage, 2);
        assert_eq!(engine.objects[idx].state.energy, 48);
        assert_eq!(engine.rng, mirror, "still no draws in open air");

        // Buried in earth (below the flat surface at y = 30): Tick5 draws
        // Random(3) for landscape inflammation (C4Object.cpp:797-805).
        let buried = engine.spawn_object(
            SpawnConfig::new("Hut")
                .with_position(Vector2::new(20, 35))
                .with_energy(50),
        )?;
        let buried_idx = engine.find_object_index(buried).expect("buried exists");
        assert!(engine.incinerate_object(buried_idx, 1, false, None)?);
        let mut mirror = engine.rng.clone();
        engine.exec_object_fire(buried_idx, 15);
        mirror.random(3);
        assert_eq!(engine.rng, mirror, "Tick5 inflame draw over material");
        assert!(engine.objects[buried_idx].state.on_fire, "earth does not extinguish");
        Ok(())
    }

    #[test]
    fn cross_check_contact_incineration_on_tick35() -> Result<(), EngineError> {
        // CrossCheck pass 1, incineration arm (C4GameObjects.cpp:106-125):
        // on Tick35 frames an OCF_OnFire object standing at an
        // OCF_Inflammable object's shape incinerates it when
        // !Random(ContactIncinerate), attributing the original fire cause.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(72);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(80, 60, Some(earth)));
        // GetFireCausePlr only forwards VALID players (C4Object.cpp:6193-6203)
        engine.register_player(PlayerConfig::new(7, "P7"))?;
        let mut torch_def = simple_definition("Torch");
        torch_def.set_fire_properties(1, false, false);
        engine.register_definition(torch_def)?;
        let mut tree_def = simple_definition("Tree");
        tree_def.set_fire_properties(1, false, false); // Random(1) == 0 always
        tree_def.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 16)));
        engine.register_definition(tree_def)?;

        let torch = engine.spawn_object(
            SpawnConfig::new("Torch").with_position(Vector2::new(40, 20)),
        )?;
        let tree = engine.spawn_object(
            SpawnConfig::new("Tree").with_position(Vector2::new(41, 20)),
        )?;
        let torch_idx = engine.find_object_index(torch).expect("torch exists");
        assert!(engine.incinerate_object(torch_idx, 7, false, None)?);

        // Not a Tick35 frame: nothing happens, no draws.
        let mirror = engine.rng.clone();
        engine.cross_check(34)?;
        let tree_idx = engine.find_object_index(tree).expect("tree exists");
        assert!(!engine.objects[tree_idx].state.on_fire);
        assert_eq!(engine.rng, mirror);

        // Tick35: Random(ContactIncinerate=1) == 0 → incinerate, which draws
        // the new fire's FirePhase. The fire cause carries over (GetFireCausePlr).
        let mut mirror = engine.rng.clone();
        mirror.random(1);
        mirror.random(15);
        engine.cross_check(35)?;
        let tree_idx = engine.find_object_index(tree).expect("tree exists");
        assert!(engine.objects[tree_idx].state.on_fire, "tree caught fire");
        assert_eq!(engine.objects[tree_idx].state.fire_caused_by, 7);
        assert_eq!(engine.rng, mirror, "contact draw then FirePhase draw");
        Ok(())
    }

    #[test]
    fn cross_check_fight_pass_engages_hostile_fight_ready_objects() -> Result<(), EngineError> {
        // CrossCheck pass 1 (C4GameObjects.cpp:97-138): on Tick5 frames,
        // FightReady objects standing at a hostile FightReady object's shape
        // start fighting both ways (ObjectActionFight = SetActionByName
        // "Fight" with target, C4ObjectCom.cpp:157-160), unless a RejectFight
        // callback vetoes (C4GameObjects.cpp:131-132).
        fn fighter_def(id: &str, script: &str) -> Result<Definition, EngineError> {
            let mut definition = Definition::from_script(id, id, script)?;
            definition.set_crew_member(true);
            definition.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 16)));
            let mut specs = HashMap::new();
            specs.insert("Idle".to_string(), ActionSpec::default());
            specs.insert("Fight".to_string(), ActionSpec::default());
            definition.configure_actions(Some("Idle".to_string()), specs);
            Ok(definition)
        }
        const PLAIN: &str = r#"
        global func Initialize(state, random) { return nil; }
        "#;

        let mut engine = Engine::with_seed(50);
        engine.register_definition(fighter_def("KnightA", PLAIN)?)?;
        engine.register_definition(fighter_def("KnightB", PLAIN)?)?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        engine.set_hostility(1, 2, true)?;

        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50)),
        )?;
        let knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(52, 50)),
        )?;

        // Frame 4 is not a Tick5 frame: nothing happens.
        engine.cross_check(4)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");

        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        let idx_b = engine.find_object_index(knight_b).expect("knight B");
        assert_eq!(engine.objects[idx_a].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_b].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_a].state.action.target, Some(knight_b));
        assert_eq!(engine.objects[idx_b].state.action.target, Some(knight_a));

        // Friendly players never fight (C4PlayerList::Hostile,
        // C4PlayerList.cpp:82-92).
        let mut engine = Engine::with_seed(51);
        engine.register_definition(fighter_def("KnightA", PLAIN)?)?;
        engine.register_definition(fighter_def("KnightB", PLAIN)?)?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50)),
        )?;
        let _knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(52, 50)),
        )?;
        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");

        // A truthy RejectFight callback on either side vetoes the fight.
        let mut engine = Engine::with_seed(52);
        engine.register_definition(fighter_def(
            "KnightA",
            r#"
            func RejectFight(enemy) { return 1; }
            "#,
        )?)?;
        engine.register_definition(fighter_def("KnightB", PLAIN)?)?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        engine.set_hostility(1, 2, true)?;
        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50)),
        )?;
        let _knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(52, 50)),
        )?;
        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");
        Ok(())
    }

    #[test]
    fn cross_check_contained_fight_runs_on_tick10() -> Result<(), EngineError> {
        // CrossCheck pass 3 (C4GameObjects.cpp:199-230): contained FightReady
        // objects in the same container fight hostile company on Tick10
        // frames — with no RejectFight veto. Pass 1 explicitly skips
        // contained objects (C4GameObjects.cpp:114), so frame 5 does nothing.
        fn fighter_def(id: &str) -> Result<Definition, EngineError> {
            let mut definition = Definition::from_script(
                id,
                id,
                r#"
                global func Initialize(state, random) { return nil; }
                "#,
            )?;
            definition.set_crew_member(true);
            definition.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 16)));
            let mut specs = HashMap::new();
            specs.insert("Idle".to_string(), ActionSpec::default());
            specs.insert("Fight".to_string(), ActionSpec::default());
            definition.configure_actions(Some("Idle".to_string()), specs);
            Ok(definition)
        }

        let mut engine = Engine::with_seed(60);
        engine.register_definition(fighter_def("KnightA")?)?;
        engine.register_definition(fighter_def("KnightB")?)?;
        engine.register_definition(simple_definition("Hut"))?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        engine.set_hostility(1, 2, true)?;

        let hut = engine
            .spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(50, 50)))?;
        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50))
                .with_container(hut),
        )?;
        let knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50))
                .with_container(hut),
        )?;

        // Tick5 frame: pass 1 skips contained objects.
        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");

        // Tick10 frame: contained fight engages both ways.
        engine.cross_check(10)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        let idx_b = engine.find_object_index(knight_b).expect("knight B");
        assert_eq!(engine.objects[idx_a].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_b].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_a].state.action.target, Some(knight_b));
        assert_eq!(engine.objects[idx_b].state.action.target, Some(knight_a));
        Ok(())
    }

    #[test]
    fn cross_check_hit_respects_query_catch_blow() -> Result<(), EngineError> {
        // C4GameObjects.cpp:168: a truthy QueryCatchBlow callback on the
        // victim suppresses the hit entirely.
        let mut engine = Engine::with_seed(41);
        let mut victim_def = Definition::from_script(
            "Guard",
            "Guard",
            r#"
            func QueryCatchBlow(by) { return 1; }
            "#,
        )
        .expect("script compiles");
        victim_def.set_mass(100);
        engine.register_definition(victim_def)?;
        let mut rock_def = simple_definition("Rock");
        rock_def.set_category(CATEGORY_OBJECT);
        rock_def.set_mass(50);
        engine.register_definition(rock_def)?;

        let victim = engine.spawn_object(
            SpawnConfig::new("Guard")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(100),
        )?;
        let _rock = engine.spawn_object(
            SpawnConfig::new("Rock")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(5, 0)),
        )?;

        engine.cross_check(1)?;
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.energy,
            100,
            "QueryCatchBlow rejected the blow"
        );
        assert_eq!(
            engine.objects[victim_idx].fixed_velocity.x,
            math::C4Fixed::ZERO,
            "no fling on rejected blow"
        );
        Ok(())
    }

    #[test]
    fn weather_disaster_rng_draw_order_matches_cpp() {
        // C4Weather::Execute disaster block (C4Weather.cpp:104-148): on every
        // Tick10 frame the gates Random(60) [meteorite], Random(35)
        // [lightning], Random(50) [earthquake], Random(60) [volcano] are
        // drawn UNCONDITIONALLY — the configured levels only gate the
        // follow-up Random(100) comparison, never the gate draw itself.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(123);
        engine.set_materials(materials.clone());
        engine.set_landscape(Landscape::flat_with_material(64, 40, Some(earth)));
        let mut mirror = engine.rng.clone();

        engine
            .tick_weather_events(10)
            .expect("weather tick succeeds");
        // levels all default to 0: each zero gate still draws Random(100)
        if mirror.random(60) == 0 {
            mirror.random(100);
        }
        if mirror.random(35) == 0 {
            mirror.random(100);
        }
        if mirror.random(50) == 0 {
            mirror.random(100);
        }
        if mirror.random(60) == 0 {
            mirror.random(100);
        }
        assert_eq!(engine.rng, mirror, "gate draws are level-independent");

        // Non-Tick10 frame: no draws at all (C4Weather.cpp:104).
        let before = engine.rng.clone();
        engine
            .tick_weather_events(11)
            .expect("weather tick succeeds");
        assert_eq!(engine.rng, before);

        // With a level at 100, a zero gate launches: lightning consumes
        // Random(GBackWdt) for its position (C4Weather.cpp:125); earthquake
        // consumes Random(GBackHgt) then Random(GBackWdt) (:133-134);
        // volcano consumes Random(10) then Random(GBackWdt) (:142-143);
        // meteorite consumes Random(101) then Random(GBackWdt) (:114-115).
        // No FX definitions are registered, so object creation is skipped,
        // but the synced draws must still happen.
        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(64, 40, Some(earth)));
        let mut environment = engine.environment();
        environment.meteorite = 100;
        environment.lightning = 100;
        environment.earthquake = 100;
        environment.volcano = 100;
        engine.set_environment(environment);
        let height = engine
            .landscape
            .as_ref()
            .map(|landscape| landscape.estimated_height())
            .unwrap_or(0);

        for frame in 1..=400u64 {
            let mut mirror = engine.rng.clone();
            engine
                .tick_weather_events(frame)
                .expect("weather tick succeeds");
            if frame % 10 != 0 {
                assert_eq!(engine.rng, mirror);
                continue;
            }
            if mirror.random(60) == 0 && mirror.random(100) < 100 {
                mirror.random(100 + 1);
                mirror.random(64);
            }
            if mirror.random(35) == 0 && mirror.random(100) < 100 {
                mirror.random(64);
            }
            if mirror.random(50) == 0 && mirror.random(100) < 100 {
                mirror.random(height);
                mirror.random(64);
            }
            if mirror.random(60) == 0 && mirror.random(100) < 100 {
                mirror.random(10);
                mirror.random(64);
            }
            assert_eq!(engine.rng, mirror, "frame {frame}");
        }
    }

    #[test]
    fn mrf_insert_check_splash_matches_cpp() {
        // mrfInsertCheck splash (C4Material.cpp:572-579): with fYDir >
        // itofix(1) and SplashRate set, !Random(SplashRate) bounces the PXS:
        // fYDir = -fYDir/8 (raw int division), fXDir = fXDir/8 +
        // FIXED100(Random(200)-100), and the nonzero fYDir keeps it alive.
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
            SplashRate=1

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(9, 10, Some(earth)));

        let mut mirror = engine.rng.clone();
        assert_eq!(mirror.random(1), 0, "SplashRate=1 always splashes");
        let expected_xdir =
            math::itofix(8) / 8 + math::fixed100(mirror.random(200) - 100);

        let (mut x, mut y) = (4, 9);
        let mut xdir = math::itofix(8);
        let mut ydir = math::itofix(16);
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            water,
            Some(earth),
            &mut pos_changed,
        );
        assert!(!insert_ok, "splash keeps the PXS alive");
        assert!(pos_changed);
        assert_eq!(ydir, -math::itofix(16) / 8);
        assert_eq!(xdir, expected_xdir);
        assert_eq!((x, y), (4, 9), "splash does not move the pixel");
        assert_eq!(engine.rng, mirror, "exactly two synced draws");
    }

    #[test]
    fn mrf_insert_check_incendiary_smokes_and_allows_insert_like_cpp() {
        // mrfInsertCheck (C4Material.cpp:584-586): incendiary materials
        // consume Random(25) and, on zero, Rnd3() for the smoke level
        // (Smoke(x, y, 4+Rnd3()), C4Effect.cpp:859-863); with no slide
        // available the check returns true (insertion OK,
        // C4Material.cpp:608-609).
        let library = MaterialLibrary::parse(
            r#"
            [Material Lava]
            Name=Lava
            Density=30
            Friction=20
            Incindiary=1

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let lava = materials.id_of("Lava").expect("lava exists");
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(2);
        engine.set_materials(materials);
        // Deep inside flat earth: no slide target anywhere.
        engine.set_landscape(Landscape::flat_with_material(9, 5, Some(earth)));
        // Force the Random(25) == 0 branch deterministically.
        let smoke_seed = (0u32..)
            .find(|&seed| LcgRng::new(seed).random(25) == 0)
            .expect("seed exists");
        engine.rng = LcgRng::new(smoke_seed);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Smoke".into(),
                    init_fn: "SmokeInit".into(),
                    exec_fn: "SmokeExec".into(),
                    draw_fn: "Smoke".into(),
                    min_lifetime: 10,
                    max_lifetime: 10,
                    ..Default::default()
                },
                4,
                1.0,
            )
            .expect("smoke def registers");

        let mut mirror = engine.rng.clone();
        assert_eq!(mirror.random(25), 0);
        let expected_level = 4 + mirror.rnd3();

        let (mut x, mut y) = (4, 20);
        let mut xdir = math::C4Fixed::ZERO;
        let mut ydir = math::C4Fixed::ZERO;
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            lava,
            Some(earth),
            &mut pos_changed,
        );
        assert!(insert_ok, "no slide target → insertion OK");
        assert_eq!(ydir, math::C4Fixed::ZERO);
        assert_eq!(engine.rng, mirror, "Random(25) then Rnd3 consumed");
        let smoke: Vec<_> = engine
            .particle_system()
            .particles()
            .iter()
            .filter(|particle| particle.def_name == "Smoke")
            .collect();
        assert_eq!(smoke.len(), 1, "smoke particle spawned");
        assert_eq!(smoke[0].x.to_bits(), 4.0f32.to_bits());
        assert_eq!(
            smoke[0].y.to_bits(),
            (20.0f32 - (expected_level / 2) as f32).to_bits()
        );
        assert_eq!(smoke[0].a.to_bits(), (expected_level as f32).to_bits());
    }

    #[test]
    fn mrf_insert_check_slide_accelerates_or_absorbs_like_cpp() {
        // mrfInsertCheck slide (C4Material.cpp:588-607): FindMatSlide with
        // Sign(GravAccel), the PXS material's density and MaxSlide. Same
        // material → absorb (move there, fXDir = 0). Different material →
        // fXDir = (fXDir*10 + Sign(slide_x - x))/11 + FIXED10(Random(5)-2),
        // with the direct jump only when the target is within |fixtoi(fXDir)|.
        let library = MaterialLibrary::parse(
            r#"
            [Material Sand]
            Name=Sand
            Density=25
            Friction=10
            MaxSlide=2

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let sand = materials.id_of("Sand").expect("sand exists");
        let earth = materials.id_of("Earth").expect("earth exists");

        // Slide target two columns left: |x - slide_x| = 2 > |fixtoi(xdir')|
        // → no direct jump; the acceleration stays observable.
        let mut engine = Engine::with_seed(13);
        engine.set_materials(materials.clone());
        engine.set_landscape(
            Landscape::with_default_material(5, vec![11, 10, 10, 10, 11], Some(earth))
                .expect("landscape builds"),
        );
        let mut mirror = engine.rng.clone();
        let expected_xdir = math::C4Fixed::from_raw(
            (math::itofix(1).val() * 10 + math::itofix(-1).val()) / 11,
        ) + math::fixed10(mirror.random(5) - 2);

        let (mut x, mut y) = (2, 9);
        let mut xdir = math::itofix(1);
        let mut ydir = math::C4Fixed::ZERO;
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            sand,
            Some(earth),
            &mut pos_changed,
        );
        assert!(!insert_ok, "slide keeps the PXS alive");
        assert_eq!((x, y), (2, 9), "target out of reach → no jump");
        assert_eq!(xdir, expected_xdir);
        assert_eq!(engine.rng, mirror, "exactly one Random(5) draw");

        // Same material at the slide target → absorb without any draw.
        let mut engine = Engine::with_seed(13);
        engine.set_materials(materials);
        engine.set_landscape(
            Landscape::with_default_material(3, vec![11, 10, 11], Some(earth))
                .expect("landscape builds"),
        );
        let mirror = engine.rng.clone();
        let (mut x, mut y) = (1, 9);
        let mut xdir = math::itofix(1);
        let mut ydir = math::C4Fixed::ZERO;
        let mut pos_changed = false;
        let insert_ok = engine.mrf_insert_check(
            &mut x,
            &mut y,
            &mut xdir,
            &mut ydir,
            sand,
            Some(sand),
            &mut pos_changed,
        );
        assert!(!insert_ok);
        assert_eq!((x, y), (0, 10), "absorbed at the slide target");
        assert_eq!(xdir, math::C4Fixed::ZERO);
        assert_eq!(engine.rng, mirror, "no synced draws on same-mat slide");
    }

    #[test]
    fn pxs_insert_reaction_runs_insert_check_like_cpp() {
        // mrfInsert on meePXSMove runs mrfInsertCheck before inserting
        // (C4Material.cpp:781-790): a PXS with a slide path keeps existing
        // (snapped to its int position, fStopMovement C4PXS.cpp:106-112);
        // an enclosed PXS inserts and dies.
        let library = MaterialLibrary::parse(
            r#"
            [Material Sand]
            Name=Sand
            Density=25
            Friction=10
            MaxSlide=2
            SplashRate=0

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let sand = materials.id_of("Sand").expect("sand exists");
        let earth = materials.id_of("Earth").expect("earth exists");

        // Slide available: the step loop hits earth below, mrfInsertCheck
        // finds the two-column slide, the PXS survives snapped to its int
        // position with the accelerated xdir (fStopMovement, C4PXS.cpp:106-112).
        // Default gravity stays on: Sign(GravAccel) feeds the slide direction
        // (C4Material.cpp:590) and the added ydir is small enough not to
        // shift fixtoi. SplashRate=0 keeps the splash branch draw-free.
        let mut engine = Engine::with_seed(21);
        engine.set_materials(materials.clone());
        engine.set_landscape(
            Landscape::with_default_material(5, vec![11, 10, 10, 10, 11], Some(earth))
                .expect("landscape builds"),
        );
        let mut mirror = engine.rng.clone();
        // fXDir = (0*10 + Sign(0-2))/11 + FIXED10(Random(5)-2) (C4Material.cpp:597)
        let expected_xdir = math::C4Fixed::from_raw(math::itofix(-1).val() / 11)
            + math::fixed10(mirror.random(5) - 2);

        assert!(engine.pxs_system.create(
            sand,
            math::itofix(2),
            math::itofix(9),
            math::C4Fixed::ZERO,
            math::itofix(1),
        ));
        engine.tick_pxs();
        let survivors: Vec<pxs::Pxs> = engine.pxs_system.iter().copied().collect();
        assert_eq!(survivors.len(), 1, "slide path keeps the PXS alive");
        assert_eq!(survivors[0].x, math::itofix(2), "snapped to int position");
        assert_eq!(survivors[0].y, math::itofix(9));
        assert_eq!(survivors[0].xdir, expected_xdir);
        assert_eq!(survivors[0].ydir, math::C4Fixed::ZERO, "contact stops ydir");
        assert_eq!(engine.rng, mirror, "exactly one Random(5) draw");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.surface_height(2)),
            Some(10),
            "nothing inserted while sliding"
        );

        // Enclosed: no slide anywhere → insertion proceeds and the PXS dies
        // (C4Material.cpp:788-790). The liquid column only extends the world
        // height so the buried PXS stays in bounds (C4PXS.cpp:45-49).
        let mut engine = Engine::with_seed(21);
        engine.set_materials(materials);
        let mut landscape = Landscape::flat_with_material(5, 10, Some(earth));
        landscape.set_liquid_column(0, vec![LiquidSegment::new(25, 28)]);
        engine.set_landscape(landscape);
        let mirror = engine.rng.clone();
        assert!(engine.pxs_system.create(
            sand,
            math::itofix(2),
            math::itofix(20),
            math::C4Fixed::ZERO,
            math::itofix(1),
        ));
        engine.tick_pxs();
        assert_eq!(engine.pxs_system.count(), 0, "enclosed PXS inserts and dies");
        assert_eq!(engine.rng, mirror, "no synced draws while enclosed");
    }

    #[test]
    fn apply_landscape_operations_executes_shake_circle() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            DigFree=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(9);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(earth)));

        engine.apply_landscape_operations(vec![LandscapeOperation::ShakeCircle {
            center: Vector2::new(8, 35),
            radius: 3,
        }]);

        let snapshot = engine.snapshot();
        assert!(
            snapshot
                .particles
                .iter()
                .any(|particle| particle.definition_id == "material/pxs/earth"),
            "shake operation should release earth particles"
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
    fn incendiary_particles_spawn_fire_without_eroding_surface() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Flame]
            Name=Flame
            Density=60
            Friction=10
            SplashRate=0
            Incindiary=100

            [Material Wood]
            Name=Wood
            Density=90
            Friction=25
            Inflammable=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let flame = materials.id_of("Flame").expect("flame exists");
        let wood = materials.id_of("Wood").expect("wood exists");

        // Ignition happens via the meePXSPos check (C4PXS.cpp:51-57): when a
        // flame PXS's rounded position lies inside inflammable material,
        // mrfIncinerate calls Landscape.Incinerate(x, y), which reads the
        // material AT the position (C4Landscape.cpp:1430-1440). A contact
        // from above inserts at the air cell instead, in C++ too. The liquid
        // column only extends the estimated world height so the embedded PXS
        // stays in bounds (the column model has no separate map height).
        let mut engine = Engine::with_seed(31);
        engine.set_materials(materials);
        engine
            .register_definition(simple_definition(FIRE_DEFINITION_ID))
            .expect("fire definition registers");
        let mut landscape = Landscape::flat_with_material(17, 80, Some(wood));
        landscape.set_liquid_column(0, vec![LiquidSegment::new(150, 160)]);
        engine.set_landscape(landscape);

        let column_x = 8;
        let before_height = engine
            .landscape()
            .expect("landscape present")
            .surface_height(column_x)
            .expect("surface height available");

        engine.pxs_system.create(
            flame,
            math::itofix(column_x),
            math::ftofix(before_height as f32 + 0.25),
            math::C4Fixed::ZERO,
            math::C4Fixed::ZERO,
        );

        engine.tick_pxs();
        let flame_spawned = engine.objects.iter().any(|object| {
            !object.destroyed
                && object.state.status.is_active()
                && object.definition_id == FIRE_DEFINITION_ID
        });
        assert!(
            flame_spawned,
            "expected a flame to spawn from the embedded PXS"
        );
        assert_eq!(engine.pxs_system.count(), 0, "ignited PXS deactivates");

        let after_height = engine
            .landscape()
            .expect("landscape present")
            .surface_height(column_x)
            .expect("surface height available");
        assert_eq!(
            after_height, before_height,
            "incineration should not erode the landscape surface"
        );

        engine.pxs_system.create(
            flame,
            math::itofix(column_x),
            math::ftofix(before_height as f32 + 0.25),
            math::C4Fixed::ZERO,
            math::C4Fixed::ZERO,
        );
        for _ in 0..3 {
            engine.tick_pxs();
        }

        let capped_flame_count = engine
            .objects
            .iter()
            .filter(|object| {
                !object.destroyed
                    && object.state.status.is_active()
                    && object.definition_id == FIRE_DEFINITION_ID
            })
            .count();
        assert_eq!(
            capped_flame_count, 1,
            "incineration should respect the fire density cap"
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
        let final_surface = landscape.surface()[6];
        assert!(
            final_surface <= post_blast_surface + 1,
            "expected particles to prevent the crater from deepening"
        );
        assert!(
            final_surface >= 30,
            "expected final surface to remain at or above the original baseline"
        );
        Ok(())
    }

    // NOTE: the former `material_particles_apply_friction_to_objects` test
    // was removed with the C4PXS port: C++ PXS never interact with objects
    // (C4PXS.cpp has no object coupling), so the friction behavior it pinned
    // was an invention of the placeholder particle loop.

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
        // C4Weather::Execute (C4Weather.cpp:94-100): TargetWind re-evaluates
        // only on Tick1000 frames with ONE synced draw
        // (BoundBy(Std + Random(2*Rnd+1) - Rnd, Min, Max), C4SVal::Evaluate,
        // C4Scenario.cpp:43-46); the wind steps ±1 toward the target on
        // Tick10 frames.
        let mut settings = EnvironmentSettings::new(5).with_wind_variation(4, 40);
        let mut rng = LcgRng::seed_from_u64(1234);
        let mut probe = rng.clone();
        let rnd = settings.wind_variation.max(0);
        let expected_target = (settings.base_wind + probe.random(2 * rnd + 1) - rnd)
            .clamp(settings.wind_min, settings.wind_max);

        // Off-gate frames consume no draws and leave the wind unchanged.
        let before = settings;
        settings.advance_frame(&mut rng, 7);
        assert_eq!(settings.wind, before.wind);
        assert_eq!(settings.wind_target, before.wind_target);

        settings.advance_frame(&mut rng, 1000);
        assert_eq!(
            settings.wind_target, expected_target,
            "Tick1000 target evaluation"
        );
        assert_eq!(rng, probe, "exactly one synced draw");
        let stepped = (before.wind + (expected_target - before.wind).signum())
            .clamp(settings.wind_min, settings.wind_max);
        assert_eq!(settings.wind, stepped, "Tick10 step toward the target");
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
    fn initialize_returning_non_proplist_is_ignored_like_cpp() {
        // C++ parity: C4Object.cpp:1483 invokes `Call(PSF_Initialize)` as a bare
        // statement and DISCARDS the return value. Real Clonk definitions return
        // an int (or anything) from Initialize; the engine must not reject such a
        // return. The command-delta proplist convention is an additive Rust
        // convenience for synthetic fixtures, not a requirement on real content.
        let source = r#"
        global func Initialize(state, random) { return 1; }
        "#;
        let definition = Definition::from_script("CLNK", "Clonk", source).expect("script compiles");
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("register definition");
        let spawned = engine.spawn_object(SpawnConfig::new("CLNK").with_energy(50));
        assert!(
            spawned.is_ok(),
            "Initialize returning an int must not error (C++ discards the return): {spawned:?}"
        );
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
    fn apply_player_commands_updates_home_base_material() {
        let mut engine = Engine::new();
        engine
            .register_player(PlayerConfig::new(1, "Leader"))
            .expect("player registered");

        engine
            .apply_player_commands(vec![PlayerCommand::AdjustHomeBaseMaterial {
                player_id: 1,
                definition_id: "Brick".to_string(),
                delta: 3,
            }])
            .expect("commands applied");

        let player = engine.player(1).expect("player present");
        assert_eq!(player.home_base_material().get("Brick"), Some(&3));
    }

    #[test]
    fn apply_player_commands_synchronizes_team_materials_when_rule_enabled() {
        let mut engine = Engine::new();
        engine.set_team_home_base_rule(true);

        engine
            .register_player(PlayerConfig::new(1, "Leader").with_team(Some(1)))
            .expect("leader registered");
        engine
            .register_player(PlayerConfig::new(2, "Follower").with_team(Some(1)))
            .expect("follower registered");

        engine
            .apply_player_commands(vec![PlayerCommand::AdjustHomeBaseMaterial {
                player_id: 1,
                definition_id: "Brick".to_string(),
                delta: 2,
            }])
            .expect("commands applied");

        let leader = engine.player(1).expect("leader present");
        let follower = engine.player(2).expect("follower present");
        assert_eq!(leader.home_base_material().get("Brick"), Some(&2));
        assert_eq!(follower.home_base_material().get("Brick"), Some(&2));
    }

    #[test]
    fn apply_player_commands_grants_player_knowledge() {
        let mut engine = Engine::new();
        engine
            .register_player(PlayerConfig::new(1, "Scholar"))
            .expect("player registered");

        engine
            .apply_player_commands(vec![PlayerCommand::GrantKnowledge {
                player_id: 1,
                definition_id: "BRIK".to_string(),
            }])
            .expect("commands applied");

        let player = engine.player(1).expect("player present");
        assert!(
            player.knowledge().any(|id| id == "BRIK"),
            "player gains requested knowledge"
        );
    }

    #[test]
    fn apply_player_commands_revokes_player_knowledge() {
        let mut engine = Engine::new();
        engine
            .register_player(
                PlayerConfig::new(1, "Scholar").with_knowledge(vec!["BRIK".to_string()]),
            )
            .expect("player registered");

        engine
            .apply_player_commands(vec![PlayerCommand::RevokeKnowledge {
                player_id: 1,
                definition_id: "BRIK".to_string(),
            }])
            .expect("commands applied");

        let player = engine.player(1).expect("player present");
        assert!(
            player.knowledge().all(|id| id != "BRIK"),
            "player no longer knows revoked definition"
        );
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

        let mut expected_rng = LcgRng::seed_from_u64(0);
        let _ = expected_rng.random(i32::MAX); // Initialize random argument
        let _ = expected_rng.random(i32::MAX); // First tick random argument
        let first_expected = expected_rng.random(10);

        let first_tick = engine.tick().expect("first tick succeeds");
        let object = first_tick.object(id).expect("object present");
        assert_eq!(object.energy, first_expected);

        let _ = expected_rng.random(i32::MAX); // Second tick random argument
        let second_expected = expected_rng.random(10);

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
        assert_eq!(
            object
                .fixed_velocity
                .expect("gravity should remain sub-pixel")
                .y
                .val(),
            524
        );
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
        assert_eq!(
            object
                .fixed_velocity
                .expect("gravity should remain sub-pixel")
                .y
                .val(),
            196739
        );
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
        assert_eq!(object.velocity.y, 0);
        assert_eq!(
            object
                .fixed_velocity
                .expect("float gravity should remain sub-pixel")
                .y
                .val(),
            393
        );
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
        assert_eq!(object.velocity, Vector2::new(2, -2));
        assert_eq!(
            object
                .fixed_velocity
                .expect("float gravity should remain sub-pixel")
                .y
                .val(),
            -131006
        );
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
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 0);
        assert_eq!(
            object
                .fixed_velocity
                .expect("swim gravity should remain sub-pixel")
                .y
                .val(),
            393
        );
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
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let target_id = engine
            .spawn_object(SpawnConfig::new("Crate"))
            .expect("target spawns");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        engine.objects[target_idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, C4Fixed::from_raw(300)));

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
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        assert_eq!(engine.objects[target_idx].fixed_velocity.y.val(), -130772);
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
    fn control_command_invokes_object_script() -> Result<(), EngineError> {
        let script = r#"
global func Initialize(state, random) { return nil; }
func ControlDig() { SetAction("Dig"); return true; }
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("control script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default());

        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let object_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle")),
            )
            .expect("spawn succeeds");

        engine.set_crew_cursor(1, Some(object_id))?;
        let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
        assert!(handled, "control command should report handled");

        let snapshot = engine.snapshot();
        let object = snapshot.object(object_id).expect("object present");
        assert_eq!(object.action.name, "Dig");
        Ok(())
    }

    #[test]
    fn object_function_this_is_the_current_object_not_nil() -> Result<(), EngineError> {
        // `this` used to evaluate to nil (vm.rs hardcoded Expr::This => Nil), so a
        // script that branches on `this` took the wrong path. Here SetAction is
        // gated on `this` being truthy: before the fix `this` was nil (falsy) and
        // the action stayed "Idle"; now `this` is the object reference so the
        // action becomes "Dig".
        let script = r#"
global func Initialize(state, random) { return nil; }
func ControlDig() { if (this) { SetAction("Dig"); } return true; }
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("control script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default());

        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let object_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle")),
            )
            .expect("spawn succeeds");

        engine.set_crew_cursor(1, Some(object_id))?;
        let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
        assert!(handled, "control command should report handled");

        let snapshot = engine.snapshot();
        let object = snapshot.object(object_id).expect("object present");
        assert_eq!(
            object.action.name, "Dig",
            "`this` should be truthy (the current object), so the gated SetAction runs"
        );
        Ok(())
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
        let materials = MaterialSet::from_resource_library(&library);
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
    fn dig_procedure_removes_surface_pixel_when_circle_touches_ground() -> Result<(), EngineError> {
        let mut definition = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
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
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 20, Some(earth)));

        let dig_radius = 6;
        let position_y = 21 - dig_radius;
        let column_x = 12;

        engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(Vector2::new(column_x, position_y))
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        for _ in 0..12 {
            engine.tick().expect("tick succeeds");
        }

        let snapshot = engine.snapshot();
        let landscape = snapshot.landscape.as_ref().expect("landscape present");
        let height = landscape
            .surface()
            .get(column_x as usize)
            .copied()
            .expect("column present");
        assert!(
            height > 20,
            "expected dig to raise surface beyond 20, got {height}"
        );
        Ok(())
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

        let gem = Definition::from_script(
            "GEM_",
            "Gem",
            "global func Initialize(state, random) { return nil; }\n",
        )
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
        let materials = MaterialSet::from_resource_library(&library);
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
    fn dig_procedure_spawns_at_most_one_dig2object_per_tick() {
        let mut digger = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig").with_dig_free(6),
        );
        digger.configure_actions(Some("Dig".to_string()), actions);

        let gem = Definition::from_script(
            "GEM_",
            "Gem",
            "global func Initialize(state, random) { return nil; }\n",
        )
        .expect("script compiles");

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
        "#;
        let library =
            lc_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(13);
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

        let mut previous_count = 0;
        let mut observed_spawn = false;
        for _ in 0..20 {
            let snapshot = engine.tick().expect("tick succeeds");
            let current_count = snapshot
                .objects
                .iter()
                .filter(|object| object.definition_id == "GEM_")
                .count();
            if current_count > previous_count {
                assert_eq!(
                    current_count - previous_count,
                    1,
                    "expected at most one Dig2Object spawn per tick"
                );
                observed_spawn = true;
                break;
            }
            previous_count = current_count;
        }

        assert!(
            observed_spawn,
            "expected Dig2Object conversion to occur within 20 ticks"
        );
    }

    #[test]
    fn dig2object_request_only_requires_explicit_request() {
        fn build_digger_definition() -> Definition {
            let mut digger = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
            let mut actions = HashMap::new();
            actions.insert(
                "Dig".to_string(),
                ActionSpec::default().with_procedure("dig").with_dig_free(6),
            );
            digger.configure_actions(Some("Dig".to_string()), actions);
            digger
        }

        fn build_gem_definition() -> Definition {
            Definition::from_script(
                "GEM_",
                "Gem",
                "global func Initialize(state, random) { return nil; }\n",
            )
            .expect("script compiles")
        }

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
            Dig2ObjectRequest=1
        "#;
        let library =
            lc_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        // Without request flag set on the action we should not spawn anything.
        {
            let mut engine = Engine::with_seed(19);
            engine
                .register_definition(build_digger_definition())
                .expect("digger registers");
            engine
                .register_definition(build_gem_definition())
                .expect("gem registers");
            engine.set_materials(materials.clone());
            engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

            engine
                .spawn_object(
                    SpawnConfig::new("DGRR")
                        .with_position(Vector2::new(12, 8))
                        .with_action(ActionState::new("Dig")),
                )
                .expect("spawn succeeds");

            for _ in 0..20 {
                let snapshot = engine.tick().expect("tick succeeds");
                assert!(
                    !snapshot
                        .objects
                        .iter()
                        .any(|object| object.definition_id == "GEM_"),
                    "expected no Dig2Object spawn without request"
                );
            }
        }

        // With request flag set, the conversion should occur.
        {
            let mut engine = Engine::with_seed(19);
            engine
                .register_definition(build_digger_definition())
                .expect("digger registers");
            engine
                .register_definition(build_gem_definition())
                .expect("gem registers");
            engine.set_materials(materials);
            engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

            let mut requested_action = ActionState::new("Dig");
            requested_action.data = 1;
            engine
                .spawn_object(
                    SpawnConfig::new("DGRR")
                        .with_position(Vector2::new(12, 8))
                        .with_action(requested_action),
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
                "expected Dig2Object conversion to respect request flag when set"
            );
        }
    }

    #[test]
    fn throw_procedure_zeroes_velocity() {
        let mut definition =
            Definition::from_script("Thrower", "Thrower", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Throw".to_string(),
            ActionSpec::default().with_procedure("throw"),
        );
        definition.configure_actions(Some("Throw".to_string()), actions);

        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Thrower")
                    .with_velocity(Vector2::new(6, -3))
                    .with_action(ActionState::new("Throw")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.action.name, "Throw");
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
    fn build_procedure_requires_components_before_progress() -> Result<(), EngineError> {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut builder_definition = Definition::from_script("Builder", "Builder", script)?;
        let mut builder_actions = HashMap::new();
        builder_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        builder_actions.insert(
            "Build".to_string(),
            ActionSpec::default().with_procedure("build"),
        );
        builder_definition.configure_actions(Some("Idle".to_string()), builder_actions);
        builder_definition.set_category(DEFAULT_CATEGORY);
        builder_definition.set_mass(50);

        let mut structure_definition = Definition::from_script("Structure", "Structure", script)?;
        structure_definition.set_constructable(true);
        structure_definition.set_category(CATEGORY_STRUCTURE);
        structure_definition.set_mass(100);
        structure_definition.set_components(vec![DefinitionComponent {
            id: "Wood".to_string(),
            count: 1,
        }]);

        let mut material_definition = Definition::from_script("Wood", "Wood", script)?;
        material_definition.set_mass(20);

        let mut engine = Engine::with_seed(7);
        engine.register_definition(builder_definition)?;
        engine.register_definition(structure_definition)?;
        engine.register_definition(material_definition)?;
        engine.set_construction_needs_material(true);

        let structure_id = engine
            .spawn_object(SpawnConfig::new("Structure").with_construction(0))
            .expect("structure spawns");

        let mut build_state = ActionState::new("Build");
        build_state.target = Some(structure_id);
        engine
            .spawn_object(
                SpawnConfig::new("Builder")
                    .with_action(build_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("builder spawns");

        let before = engine
            .object_snapshot(structure_id)
            .expect("structure present")
            .construction;
        let snapshot = engine.tick()?;
        let after = snapshot
            .object(structure_id)
            .expect("structure present")
            .construction;
        assert_eq!(before, 0);
        assert_eq!(
            after, 0,
            "construction should not progress without components"
        );
        Ok(())
    }

    #[test]
    fn build_procedure_consumes_components_from_builder() -> Result<(), EngineError> {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut builder_definition = Definition::from_script("Builder", "Builder", script)?;
        let mut builder_actions = HashMap::new();
        builder_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        builder_actions.insert(
            "Build".to_string(),
            ActionSpec::default().with_procedure("build"),
        );
        builder_definition.configure_actions(Some("Idle".to_string()), builder_actions);
        builder_definition.set_category(DEFAULT_CATEGORY);
        builder_definition.set_mass(50);

        let mut structure_definition = Definition::from_script("Structure", "Structure", script)?;
        structure_definition.set_constructable(true);
        structure_definition.set_category(CATEGORY_STRUCTURE);
        structure_definition.set_mass(100);
        structure_definition.set_components(vec![DefinitionComponent {
            id: "Wood".to_string(),
            count: 1,
        }]);

        let mut material_definition = Definition::from_script("Wood", "Wood", script)?;
        material_definition.set_mass(20);

        let mut engine = Engine::with_seed(11);
        engine.register_definition(builder_definition)?;
        engine.register_definition(structure_definition)?;
        engine.register_definition(material_definition)?;
        engine.set_construction_needs_material(true);

        let structure_id = engine
            .spawn_object(SpawnConfig::new("Structure").with_construction(0))
            .expect("structure spawns");

        let mut build_state = ActionState::new("Build");
        build_state.target = Some(structure_id);
        let builder_id = engine
            .spawn_object(
                SpawnConfig::new("Builder")
                    .with_action(build_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("builder spawns");

        let wood_id = engine
            .spawn_object(SpawnConfig::new("Wood").with_construction(FULL_CON))
            .expect("wood spawns");
        engine
            .apply_object_update(wood_id, ObjectUpdate::new().with_container(builder_id))
            .expect("assign container succeeds");

        let snapshot = engine.tick()?;
        let structure = snapshot
            .object(structure_id)
            .expect("structure present after tick");
        assert!(
            structure.construction > 0,
            "construction should advance when components are available"
        );
        let components = structure.components.get("Wood");
        assert_eq!(components, Some(&1));
        assert!(
            snapshot.object(wood_id).is_none(),
            "component should be consumed during build"
        );
        Ok(())
    }

    #[test]
    fn applies_velocity_changes_from_step_callback() {
        let mut engine = Engine::with_seed(123);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

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
        assert_eq!(object.position, Vector2::new(1, 0));
        assert_eq!(object.velocity, Vector2::new(2, 0));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(3, 0));
        assert_eq!(object.velocity, Vector2::new(3, 0));
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
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

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
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.position, Vector2::ZERO);
        assert_eq!(
            object
                .fixed_velocity
                .expect("wind should remain sub-pixel")
                .x
                .val(),
            1310
        );
        assert_eq!(
            object
                .fixed_position
                .expect("wind movement should remain sub-pixel")
                .x
                .val(),
            1310
        );

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.position, Vector2::ZERO);
        assert_eq!(
            object
                .fixed_velocity
                .expect("wind should accumulate in fixed velocity")
                .x
                .val(),
            2620
        );
        assert_eq!(
            object
                .fixed_position
                .expect("wind movement should accumulate in fixed position")
                .x
                .val(),
            3930
        );
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
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );

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
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        engine.objects[pusher_idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));

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
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        assert_eq!(engine.objects[pusher_idx].fixed_velocity.x.val(), 294912);
        assert_eq!(engine.objects[target_idx].fixed_velocity.x.val(), 196608);

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
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );

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
        let puller_idx = engine.find_object_index(puller_id).expect("puller exists");
        engine.objects[puller_idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));

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
        let puller_idx = engine.find_object_index(puller_id).expect("puller exists");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        assert_eq!(engine.objects[puller_idx].fixed_velocity.x.val(), 294912);
        assert_eq!(engine.objects[target_idx].fixed_velocity.x.val(), 196608);

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
        // DFA_FIGHT approaches with the Walk physical (C4Object.cpp:5225-5228),
        // not the movement profile. 35000 is the stock Clonk DefCore value.
        fighter_definition.set_physical(PhysicalInfo {
            walk: 35_000,
            ..PhysicalInfo::default()
        });

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
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );

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
        let fighter_idx = engine
            .find_object_index(fighter_id)
            .expect("fighter exists");
        engine.objects[fighter_idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));

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
        let fighter_idx = engine
            .find_object_index(fighter_id)
            .expect("fighter exists");
        // C4Object.cpp:5221-5228: facing Right, stand-beside target_x at
        // 12 - 16/2 - 2 = 2; lLimit = ValByPhysical(95, 35000)
        // = itofix(35000*19, 2000000) = raw 21790; Towards steps the initial
        // raw 98304 down by one lLimit: 98304 - 21790 = 76514.
        assert_eq!(engine.objects[fighter_idx].fixed_velocity.x.val(), 76514);
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
    fn fight_procedure_trains_fight_physical_on_tick5() {
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
            "Fight".to_string(),
            ActionSpec::default().with_procedure("fight"),
        );
        fighter_definition.configure_actions(Some("Fight".to_string()), fighter_actions);
        fighter_definition.set_physical(PhysicalInfo {
            walk: 35_000,
            fight: 20_000,
            ..PhysicalInfo::default()
        });

        let mut opponent_definition =
            Definition::from_script("Opponent", "Opponent", script).unwrap();
        let mut opponent_actions = HashMap::new();
        opponent_actions.insert(
            "Fight".to_string(),
            ActionSpec::default().with_procedure("fight"),
        );
        opponent_definition.configure_actions(Some("Fight".to_string()), opponent_actions);

        let mut engine = Engine::with_seed(33);
        engine
            .register_definition(fighter_definition)
            .expect("fighter definition registers");
        engine
            .register_definition(opponent_definition)
            .expect("opponent definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

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
                    .with_vertices(vertices)
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

        // C4Object.cpp:5214-5216: `if (!Tick5) TrainPhysical(Fight, 1,
        // C4MaxPhysical)` — the gate fires on frames divisible by 5 only.
        for _ in 0..4 {
            engine.tick().expect("tick succeeds");
        }
        let fighter_idx = engine
            .find_object_index(fighter_id)
            .expect("fighter exists");
        assert_eq!(
            engine.objects[fighter_idx].physical_override, None,
            "no training before the first Tick5 frame"
        );

        engine.tick().expect("tick succeeds");
        let trained = engine.objects[fighter_idx]
            .physical_override
            .expect("Tick5 training clones the definition physicals");
        assert_eq!(trained.fight, 20_001);
        assert_eq!(trained.walk, 35_000, "other physicals copied untouched");
    }

    #[test]
    fn do_energy_clamps_to_physical_energy_ceiling() {
        let script = r#"
        global func Initialize(state, random) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Clonk", "Clonk", script).unwrap();
        // DoEnergy bounds energy by GetPhysical()->Energy (C4Object.cpp:1361);
        // 50000 on the 0..C4MaxPhysical scale is 50 percent points.
        definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        let legacy_definition = Definition::from_script("Crate", "Crate", script).unwrap();

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .register_definition(legacy_definition)
            .expect("legacy definition registers");

        let clonk_id = engine
            .spawn_object(SpawnConfig::new("Clonk").with_energy(40))
            .expect("clonk spawns");
        let clonk_idx = engine.find_object_index(clonk_id).expect("clonk exists");
        engine.change_object_energy(clonk_idx, 30, -1);
        assert_eq!(
            engine.objects[clonk_idx].state.energy, 50,
            "gain clamps to the physical Energy ceiling"
        );

        // Documented deviation: zero-physical fixture definitions keep the
        // legacy unclamped ceiling instead of C++'s clamp-to-zero.
        let crate_id = engine
            .spawn_object(SpawnConfig::new("Crate").with_energy(40))
            .expect("crate spawns");
        let crate_idx = engine.find_object_index(crate_id).expect("crate exists");
        engine.change_object_energy(crate_idx, 30, -1);
        assert_eq!(engine.objects[crate_idx].state.energy, 70);
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
        // The old sinusoidal per-frame wind model was an invention; the C++
        // wind is mutable state (C4Weather::Wind) advanced by the tick gates
        // in advance_frame - wind_force reports it regardless of the frame.
        let settings = EnvironmentSettings::new(2).with_wind_variation(4, 4);
        for frame in 0..5 {
            assert_eq!(settings.wind_force(frame), 2);
        }

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
    fn temperature_at_height_respects_gradient() {
        let settings = EnvironmentSettings::new(0)
            .with_temperature(0)
            .with_climate(0)
            .with_temperature_range(40);
        let world_height = 200;
        let top = settings.temperature_at_height(0, 0, world_height);
        let middle = settings.temperature_at_height(0, world_height / 2, world_height);
        let bottom = settings.temperature_at_height(0, world_height, world_height);

        assert_eq!(middle, settings.ambient_temperature(0));
        assert!(
            top < middle,
            "expected top of map to be colder than mid level"
        );
        assert!(
            bottom > middle,
            "expected bottom of map to be warmer than mid level"
        );
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
    fn temperature_rises_towards_target_without_year_speed() {
        let mut settings = EnvironmentSettings::new(0)
            .with_climate(20)
            .with_temperature_range(30)
            .with_season(0)
            .with_temperature(-40);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.temperature, -39);
    }

    #[test]
    fn temperature_falls_towards_target_without_year_speed() {
        let mut settings = EnvironmentSettings::new(0)
            .with_climate(-10)
            .with_temperature_range(20)
            .with_season(50)
            .with_temperature(40);
        let mut rng = LcgRng::seed_from_u64(1);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.temperature, 39);
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
    fn lightning_event_spawns_effect_and_calls_activate() {
        let script = r#"
        func Initialize(state, random) { return nil; }
        func Step(state, frame, random) { return nil; }
        func Activate(x, y, xdir, xrange, ydir, yrange, gamma) { return true; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FXL1", "Lightning", script).expect("definition builds"),
            )
            .expect("definition registers");

        assert!(
            engine
                .trigger_lightning(120)
                .expect("lightning trigger succeeds"),
            "lightning definition should spawn effect"
        );

        let index = engine
            .objects
            .iter()
            .position(|object| object.definition_id == "FXL1")
            .expect("lightning effect spawned");
        assert_eq!(
            engine.objects[index].state.position,
            Vector2::new(120, 0),
            "lightning effect should spawn at requested x position"
        );
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
                    if let Some(Value::Proplist(state)) = args.first() {
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
    fn snapshot_round_trip_preserves_sub_pixel_velocity() {
        // Sub-pixel velocity (raw 16.16 fractions below one whole pixel) must
        // survive a snapshot save/restore. C++ persists both the integer mirror
        // and the fixed value (`C4Object.cpp:2742`); the integer-only path would
        // round the velocity to whole pixels (fixtoi) and lose the fraction.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(5, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        // x: pure sub-pixel (rounds to 0 px); y: 1 px + sub-pixel fraction.
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(300),
            C4Fixed::from_raw(70000),
        ));

        let snapshot = engine.snapshot();

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored
            .restore_snapshot(&snapshot)
            .expect("snapshot restores");

        let ridx = restored
            .find_object_index(id)
            .expect("restored object exists");
        assert_eq!(restored.objects[ridx].fixed_velocity.x.val(), 300);
        assert_eq!(restored.objects[ridx].fixed_velocity.y.val(), 70000);
    }

    #[test]
    fn json_save_load_preserves_sub_pixel_velocity() {
        // The save-game path serializes through JSON; sub-pixel velocity must
        // survive serialize -> deserialize -> restore so a reloaded game stays
        // in lockstep with one that ran continuously.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(5, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(300),
            C4Fixed::from_raw(70000),
        ));

        let json = engine
            .capture_state()
            .to_json_string()
            .expect("state serializes");

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        let state = EngineState::from_json_str(&json).expect("state deserializes");
        restored.restore_state(&state).expect("state restores");

        let ridx = restored
            .find_object_index(id)
            .expect("restored object exists");
        assert_eq!(restored.objects[ridx].fixed_velocity.x.val(), 300);
        assert_eq!(restored.objects[ridx].fixed_velocity.y.val(), 70000);
    }

    #[test]
    fn snapshot_round_trip_preserves_rotation_velocity() {
        // A spinning object's angular velocity (rdir) and rotation accumulator
        // (fix_r) must survive save/restore so a reloaded game keeps turning in
        // lockstep — mirroring C++ persisting rdir/fix_r.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(5, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        // 1.0 deg/frame angular velocity, mid-rotation with a sub-degree fix_r.
        engine.objects[idx].rotation_velocity = itofix(1);
        engine.objects[idx].fixed_rotation = C4Fixed::from_raw(327680 + 300);

        let json = engine
            .capture_state()
            .to_json_string()
            .expect("state serializes");

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        let state = EngineState::from_json_str(&json).expect("state deserializes");
        restored.restore_state(&state).expect("state restores");

        let ridx = restored
            .find_object_index(id)
            .expect("restored object exists");
        assert_eq!(
            restored.objects[ridx].rotation_velocity.val(),
            itofix(1).val()
        );
        assert_eq!(restored.objects[ridx].fixed_rotation.val(), 327680 + 300);
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
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(0, 0));
        assert_eq!(object.energy, 42);
        assert_eq!(snapshot.objects.len(), 2, "spawned child should exist");

        let spawned = snapshot
            .objects
            .iter()
            .find(|obj| obj.id != id)
            .expect("child object present");
        assert_eq!(spawned.position, Vector2::new(5, 0));
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
            .register_definition(simple_definition("Test"))
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
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.position.y, 0);
        assert_eq!(
            object
                .fixed_velocity
                .expect("custom gravity should remain sub-pixel")
                .y
                .val(),
            262
        );
        assert_eq!(
            object
                .fixed_position
                .expect("custom gravity movement should remain sub-pixel")
                .y
                .val(),
            262
        );
    }

    #[test]
    fn fixed_point_velocity_accumulates_sub_pixel_motion() {
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(simple_definition("Test"))
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(300), C4Fixed::ZERO));

        for _ in 0..109 {
            let snapshot = engine.tick().expect("tick succeeds");
            let object = snapshot.object(id).expect("object present");
            assert_eq!(object.position.x, 0);
        }

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        // `fixtoi` rounds to nearest: 110 * 300 = 33000, just over 0.5px.
        assert_eq!(object.position.x, 1);
        assert_eq!(object.velocity.x, 0);
    }

    #[test]
    fn gravity_accumulates_as_c4fixed_matching_cpp_golden() {
        // Mirrors parity/golden/parity_golden.json movement[0]: C4Movement.cpp:643
        // applies ydir += GravAccel with raw GravAccel 13107 each frame.
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(simple_definition("Test"))
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(100, 200, -200));
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let expected_ydir = [13107, 26214, 39321, 52428, 65535];

        for raw_ydir in expected_ydir {
            let _ = engine.tick().expect("tick succeeds");
            let idx = engine.find_object_index(id).expect("object exists");
            assert_eq!(engine.objects[idx].fixed_velocity.y.val(), raw_ydir);
        }
    }

    #[test]
    fn spawn_landscape_friction_applies_to_fixed_velocity() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(17);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 10, Some(earth)));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        let definition = Definition::from_script(
            "Slider",
            "Slider",
            r#"
            global func Initialize(state, random) { SetXDir(15); return nil; }
            global func Step(state, frame, random) { return nil; }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id =
            engine.spawn_object(SpawnConfig::new("Slider").with_position(Vector2::new(5, 12)))?;
        let idx = engine.find_object_index(id).expect("object exists");

        assert_eq!(engine.objects[idx].state.position.y, 10);
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 49152);
        assert_eq!(engine.objects[idx].state.velocity.x, 1);
        Ok(())
    }

    #[test]
    fn landscape_collision_preserves_fixed_x_and_zeroes_contact_y() {
        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(simple_definition("Crate"))
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(20, 10));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(5, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_position(Vector2::new(5, 12));
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(300),
            C4Fixed::from_raw(70000),
        ));

        engine.apply_landscape_at_index(idx);

        assert_eq!(engine.objects[idx].state.position.y, 10);
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 300);
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].state.velocity, Vector2::ZERO);
    }

    #[test]
    fn per_pixel_horizontal_movement_stops_at_first_solid_column() {
        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(simple_definition("Crate"))
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let mut surface = vec![20; 12];
        surface[6] = 0;
        engine.set_landscape(Landscape::new(12, surface).expect("landscape constructs"));

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(4, 10)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(4), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");

        assert_eq!(object.position, Vector2::new(5, 10));
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(5));
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
    }

    #[test]
    fn spawn_initializes_vertices_from_definition_shape() {
        let mut definition = simple_definition("Rock");
        definition.set_shape_vertices(vec![
            ObjectVertex::new(-1, 1)
                .with_cnat(CNAT_BOTTOM)
                .with_friction(80),
            ObjectVertex::new(1, 1)
                .with_cnat(CNAT_BOTTOM)
                .with_friction(80),
        ]);

        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Rock"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot exists");
        assert_eq!(snapshot.vertices.len(), 2);
        assert_eq!(snapshot.vertices[0].x, -1);
        assert_eq!(snapshot.vertices[0].cnat, CNAT_BOTTOM);
        assert_eq!(snapshot.vertices[0].friction, 80);
    }

    #[test]
    fn sectors_index_spawned_objects_by_point_and_shape_area() {
        let mut definition = simple_definition("Crate");
        definition.set_shape_rect(Some(DefinitionRect::new(-10, -5, 70, 10)));

        let mut engine = Engine::with_seed(31);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(20, 20)))
            .expect("spawn succeeds");

        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert_eq!(
            sectors.object_ids(sector::SectorKey::Inside { x: 0, y: 0 }),
            &[id]
        );
        assert_eq!(
            sectors.shape_ids(sector::SectorKey::Inside { x: 0, y: 0 }),
            &[id]
        );
        assert_eq!(
            sectors.shape_ids(sector::SectorKey::Inside { x: 1, y: 0 }),
            &[id]
        );
        assert!(sectors
            .shape_ids(sector::SectorKey::Inside { x: 2, y: 0 })
            .is_empty());
        let area = sectors.area(DefinitionRect::new(0, 0, 100, 50));
        assert_eq!(sectors.shape_ids_in_area(&area), vec![id]);
        assert_eq!(sectors.shape_sum(), 2);
    }

    #[test]
    fn sectors_track_object_position_updates_across_sector_boundaries() {
        let mut engine = Engine::with_seed(32);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(simple_definition("Mover"))
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Mover").with_position(Vector2::new(10, 10)))
            .expect("spawn succeeds");
        engine
            .apply_object_update(id, ObjectUpdate::new().with_position(Vector2::new(70, 10)))
            .expect("update succeeds");

        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert!(sectors
            .object_ids(sector::SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
        assert_eq!(
            sectors.object_ids(sector::SectorKey::Inside { x: 1, y: 0 }),
            &[id]
        );
    }

    #[test]
    fn sectors_remove_deleted_objects_from_membership_lists() {
        let mut engine = Engine::with_seed(33);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(simple_definition("Rock"))
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Rock").with_position(Vector2::new(10, 10)))
            .expect("spawn succeeds");
        engine
            .apply_object_update(id, ObjectUpdate::new().with_status(ObjectStatus::Deleted))
            .expect("delete succeeds");

        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert!(sectors
            .object_ids(sector::SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
        assert!(sectors
            .shape_ids(sector::SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
    }

    #[test]
    fn at_object_uses_point_sector_and_shape_test() {
        let mut definition = simple_definition("Target");
        definition.set_shape_rect(Some(DefinitionRect::new(-10, -5, 20, 10)));
        definition.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(34);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Target").with_position(Vector2::new(40, 10)))
            .expect("spawn succeeds");

        let hit = engine
            .at_object(Vector2::new(31, 10), ocf::GRAB, None)
            .expect("object found at point");
        assert_eq!(hit.1, id);
        assert_ne!(hit.2 & ocf::GRAB, 0);
        assert!(engine
            .at_object(Vector2::new(29, 10), ocf::GRAB, None)
            .is_none());
    }

    #[test]
    fn at_object_exclusive_candidate_blocks_later_matches() {
        let mut blocker = simple_definition("Blocker");
        blocker.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        blocker.set_ocf_base(ocf::EXCLUSIVE);
        let mut target = simple_definition("Target");
        target.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        target.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(35);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(blocker)
            .expect("blocker definition registers");
        engine
            .register_definition(target)
            .expect("target definition registers");

        engine
            .spawn_object(SpawnConfig::new("Blocker").with_position(Vector2::new(20, 20)))
            .expect("blocker spawns");
        engine
            .spawn_object(SpawnConfig::new("Target").with_position(Vector2::new(20, 20)))
            .expect("target spawns");

        assert!(engine
            .at_object(Vector2::new(20, 20), ocf::GRAB, None)
            .is_none());
    }

    #[test]
    fn cross_check_collection_uses_sector_area_candidates() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(36);
        engine.set_landscape(Landscape::flat(160, 160));
        let mut crew_definition = Definition::from_script("Crew", "Crew", BASIC_OBJECT_SCRIPT)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_shape_rect(Some(DefinitionRect::new(-30, -10, 80, 20)));
        crew_definition.set_collection_rect(Some(DefinitionRect::new(-30, -10, 80, 20)));
        engine.register_definition(crew_definition)?;

        let mut item_definition = Definition::from_script("Gem", "Gem", BASIC_OBJECT_SCRIPT)?;
        item_definition.set_collectible(true);
        engine.register_definition(item_definition)?;

        let crew = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(70, 20)),
        )?;
        let item =
            engine.spawn_object(SpawnConfig::new("Gem").with_position(Vector2::new(115, 20)))?;

        // Collection runs on Tick3 frames only (C4GameObjects.cpp:144-148).
        for _ in 0..3 {
            let _ = engine.tick()?;
        }

        let item_snapshot = engine.object_snapshot(item).expect("item snapshot");
        assert_eq!(item_snapshot.container, Some(crew));
        Ok(())
    }

    #[test]
    fn shape_bottom_vertex_contact_stops_before_solid_surface() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Crate");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -2, 2, 4)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 2)
            .with_cnat(CNAT_BOTTOM)
            .with_friction(100)]);
        definition.set_contact_density(50);

        let mut engine = Engine::with_seed(31);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 12, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(5, 8)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(4)));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(5, 9));
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.y, itofix(9));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn shape_horizontal_contact_redirects_force_like_cpp() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Crate");
        definition.set_shape_vertices(vec![ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT)]);
        definition.set_contact_density(50);

        let mut engine = Engine::with_seed(37);
        engine.set_materials(materials);
        let mut surface = vec![20; 12];
        surface[6] = 0;
        let mut landscape =
            Landscape::new_with_material(12, surface, Some(earth)).expect("landscape constructs");
        landscape.fill_solid_material(Some(earth));
        engine.set_landscape(landscape);
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(4, 10)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(4), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 10));
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(4));
        assert_eq!(
            engine.objects[idx].fixed_velocity.x,
            itofix(4) - fixed100(50)
        );
        assert_eq!(engine.objects[idx].fixed_velocity.y, -fixed100(50));
    }

    #[test]
    fn vehicle_density_boundary_below_contact_density_allows_motion_like_cpp() {
        // Mirrors src/C4Movement.cpp:260-281 horizontal per-pixel loop:
        // `ContactCheck(ctx, y)` gates `DoMotion(ctx - x, 0)`. Contact is
        // `GBackDensity >= ContactDensity` through src/C4Movement.cpp:166-182
        // and src/C4Shape.cpp:389.
        //
        // Hand-derived golden: src/C4Landscape.h:144-150 returns MCVehic for a
        // closed left border, and src/C4Material.h:200 defines C4M_Vehicle = 100.
        // With ContactDensity = 101, 100 >= 101 is false, so C++ takes DoMotion
        // at src/C4Movement.cpp:281 and moves x from 0 to -1 without redirecting.
        let mut definition = simple_definition("Probe");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_LEFT)]);
        definition.set_contact_density(101);

        let mut engine = Engine::with_seed(53);
        engine.set_landscape(Landscape::flat(8, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Probe").with_position(Vector2::new(0, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(-itofix(1), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(-1, 5));

        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, -itofix(1));
        assert_eq!(engine.objects[idx].fixed_velocity.x, -itofix(1));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn border_bound_sides_clamps_fixed_target_and_velocity() {
        let mut definition = simple_definition("Bounded");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition.set_border_bound(C4D_BORDER_SIDES);

        let mut engine = Engine::with_seed(41);
        engine.set_landscape(Landscape::flat(10, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Bounded").with_position(Vector2::new(8, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(5), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position.x, 9);
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(9));
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
    }

    #[test]
    fn layer_border_bound_clamps_horizontal_target_like_cpp() {
        // Mirrors src/C4Movement.cpp:185-196. For a non-static object, C++ applies
        // layer-side TargetBounds when `pLayer->Def->BorderBound & C4D_Border_Layer`:
        // low  = layer.x + layer.Shape.x - object.Shape.x
        // high = layer.x + layer.Shape.x + layer.Shape.Wdt + object.Shape.x
        //
        // Hand-derived golden for this setup: layer.x=20, layer.Shape.x=-1,
        // layer.Shape.Wdt=10, object.Shape.x=0, so high=29. `fix_x += xdir`
        // targets x=33, SideBounds clamps ctcox to 29 and zeroes xdir via
        // TargetBounds at src/C4Movement.cpp:147-155, then the per-pixel loop
        // moves from x=28 to x=29.
        let mut layer_definition = simple_definition("Layer");
        layer_definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 10, 10)));
        layer_definition.set_border_bound(C4D_BORDER_LAYER);

        let mut mover_definition = simple_definition("Mover");
        mover_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);

        let mut engine = Engine::with_seed(57);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(layer_definition)
            .expect("layer definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let layer_id = engine
            .spawn_object(SpawnConfig::new("Layer").with_position(Vector2::new(20, 10)))
            .expect("layer spawns");
        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(28, 10))
                    .with_layer(layer_id),
            )
            .expect("mover spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(5), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position.x, 29);

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(29));
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
    }

    #[test]
    fn solid_mask_vehicle_density_blocks_per_pixel_contact_like_cpp() {
        // Mirrors src/C4Movement.cpp:260-282: the horizontal per-pixel loop
        // aborts before `DoMotion` when `ContactCheck(ctx, y)` reports contact.
        // `C4SolidMask::Put` writes solid-mask pixels as MCVehic at
        // src/C4SolidMask.cpp:66-104, and C4Material.h:200 defines vehicle
        // density as 100.
        //
        // Hand-derived golden: blocker.x=5, blocker.Shape.x=0, SolidMask.tx=0,
        // so its one-pixel mask is put at world (5,5). The mover tests candidate
        // (5,5), and 100 >= ContactDensity 50 is contact, so C++ keeps x=4,
        // rewinds fix_x to itofix(4), and RedirectForce moves FIXED100(50) from
        // xdir to ydir at C4Movement.cpp:277.
        let mut blocker_definition =
            Definition::from_script("Blocker", "Blocker", "").expect("script compiles");
        blocker_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker_definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut mover_definition = simple_definition("Mover");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);

        let mut engine = Engine::with_seed(59);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(blocker_definition)
            .expect("blocker definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let mover_id = engine
            .spawn_object(SpawnConfig::new("Mover").with_position(Vector2::new(4, 5)))
            .expect("mover spawns");
        engine
            .spawn_object(SpawnConfig::new("Blocker").with_position(Vector2::new(5, 5)))
            .expect("blocker spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 5));

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(4));
        assert_eq!(engine.objects[idx].fixed_velocity.x, fixed100(50));
        assert_eq!(engine.objects[idx].fixed_velocity.y, -fixed100(50));
    }

    #[test]
    fn solid_mask_transparent_bitmap_pixel_allows_motion_like_cpp() -> Result<(), EngineError> {
        // Mirrors src/C4SolidMask.cpp:401-411: the object solid-mask bitmap is
        // copied from definition graphics transparency, and src/C4SolidMask.cpp:
        // 80-104 only writes MCVehic for non-transparent mask pixels. The
        // movement loop at src/C4Movement.cpp:260-282 therefore takes `DoMotion`
        // when `ContactCheck(ctx, y)` probes a transparent source pixel.
        //
        // Hand-derived golden: Blocker's SolidMask=0,0,2,1,0,0 at object (5,5)
        // covers world x=5..6, but graphics pixel 0 is transparent and pixel 1
        // is opaque. The mover's one-step candidate vertex probes (5,5), so
        // C++ sees background density 0 < ContactDensity 50 and moves to x=5
        // without redirecting xdir into ydir.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Blocker.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=BLCK\nName=Blocker\nCategory=C4D_Object\nShape=0,0,2,1\nSolidMask=0,0,2,1,0,0\n",
        )
        .expect("write defcore");
        let mut image = image::RgbaImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        image
            .save(def_dir.join("Graphics.png"))
            .expect("write graphics");

        let group = lc_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let blocker_definition = Definition::from_resource(&resource)?;

        let mut mover_definition = simple_definition("Mover");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);

        let mut engine = Engine::with_seed(69);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine.register_definition(blocker_definition)?;
        engine.register_definition(mover_definition)?;

        let mover_id =
            engine.spawn_object(SpawnConfig::new("Mover").with_position(Vector2::new(4, 5)))?;
        engine.spawn_object(SpawnConfig::new("BLCK").with_position(Vector2::new(5, 5)))?;
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));

        let snapshot = engine.tick()?;
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(5, 5));

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(5));
        assert_eq!(engine.objects[idx].fixed_velocity.x, itofix(1));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        Ok(())
    }

    #[test]
    fn contact_right_callback_runs_before_redirect_and_next_rng_consumer_like_cpp() {
        // Mirrors src/C4Movement.cpp:271-278: horizontal contact calls
        // ContactCheck before RedirectForce. ContactCheck runs shape contact and
        // then contact callbacks in src/C4Movement.cpp:166-182 via
        // C4Object::Contact at src/C4Movement.cpp:112-119.
        //
        // Hand-derived golden for seed 61: Engine startup does Randomize3(), i.e.
        // 500 calls to Random(3). ContactRight then consumes Random(100) = 13.
        // The following Step random argument is therefore the next
        // Random(i32::MAX) = 30827. ContactRight's SetXDir(40) runs before
        // RedirectForce, so xdir is itofix(4) - FIXED100(50), not the old xdir
        // redirect result.
        let script = r#"
            global func ContactRight()
            {
                SetXDir(40);
                return Random(100);
            }

            global func Step(state, frame, random)
            {
                return { energy = random };
            }
        "#;

        let mut blocker_definition =
            Definition::from_script("Blocker", "Blocker", "").expect("script compiles");
        blocker_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker_definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut mover_definition =
            Definition::from_script("Mover", "Mover", script).expect("script compiles");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);
        mover_definition.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(61);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(blocker_definition)
            .expect("blocker definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let mover_id = engine
            .spawn_object(SpawnConfig::new("Mover").with_position(Vector2::new(4, 5)))
            .expect("mover spawns");
        engine
            .spawn_object(SpawnConfig::new("Blocker").with_position(Vector2::new(5, 5)))
            .expect("blocker spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 5));
        assert_eq!(object.energy, 30827);

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(4));
        assert_eq!(
            engine.objects[idx].fixed_velocity.x,
            itofix(4) - fixed100(50)
        );
        assert_eq!(engine.objects[idx].fixed_velocity.y, -fixed100(50));
    }

    #[test]
    fn hit_callbacks_run_after_contact_with_old_velocity_args_like_cpp() {
        // Mirrors src/C4Movement.cpp:247-252,468-478: movement stores oldxdir,
        // oldydir, and old_ocf before stepping; after contact and NoAttachAction,
        // it calls Hit/Hit2/Hit3 in that order based on the old OCF hit-speed
        // bits, passing fixtoi(oldxdir, 100), fixtoi(oldydir, 100). The hit-speed
        // thresholds are src/C4Movement.cpp:35-38; the flags are set from
        // C4Object::GetSpeed() = abs(xdir)+abs(ydir) at src/C4Object.cpp:588-592.
        //
        // Hand-derived golden for seed 63: Engine startup does Randomize3(), i.e.
        // 500 calls to Random(3). No contact callback consumes RNG here, so the
        // following Step random argument is Random(i32::MAX) = 36328. With
        // oldxdir = itofix(2), oldydir = 0, C++ sets HitSpeed1 and HitSpeed2 but
        // not HitSpeed3. The callback arguments are (200, 0), so Hit subtracts
        // 210 energy and Hit2 subtracts 220; Step encodes the total callback
        // delta plus RNG as 430 + 36328 = 36758.
        let script = r#"
            global func Hit(x, y)
            {
                DoEnergy(0 - (10 + x + y), nil, true);
                return nil;
            }

            global func Hit2(x, y)
            {
                DoEnergy(0 - (20 + x + y), nil, true);
                return nil;
            }

            global func Hit3(x, y)
            {
                DoEnergy(0 - (40 + x + y), nil, true);
                return nil;
            }

            global func Step(state, frame, random)
            {
                return { energy = 1000000 - state.energy + random };
            }
        "#;

        let mut blocker_definition =
            Definition::from_script("Blocker", "Blocker", "").expect("script compiles");
        blocker_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker_definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut mover_definition =
            Definition::from_script("Mover", "Mover", script).expect("script compiles");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);

        let mut engine = Engine::with_seed(63);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(blocker_definition)
            .expect("blocker definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(4, 5))
                    .with_energy(1000000),
            )
            .expect("mover spawns");
        engine
            .spawn_object(SpawnConfig::new("Blocker").with_position(Vector2::new(5, 5)))
            .expect("blocker spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(2), C4Fixed::ZERO));

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 5));
        assert_eq!(object.energy, 36758);

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(4));
        assert_eq!(
            engine.objects[idx].fixed_velocity.x,
            itofix(2) - fixed100(50)
        );
        assert_eq!(engine.objects[idx].fixed_velocity.y, -fixed100(50));
    }

    #[test]
    fn construction_jolt_updates_vertices_and_preserves_bottom_like_cpp() -> Result<(), EngineError>
    {
        // Mirrors src/C4Object.cpp:1401-1428: DoCon stores the old shape bottom,
        // changes Con, then calls UpdateFace(true) -> UpdateShape(true).
        // UpdateShape copies definition vertices at src/C4Object.cpp:320-333 and
        // non-stretch construction growth calls C4Shape::Jolt, whose vertex path
        // scales only VtxY at src/C4Shape.cpp:121-127. Finally DoCon preserves
        // the old bottom edge for straight objects at src/C4Object.cpp:1462-1468.
        //
        // Hand-derived golden: full shape y=0,h=4 and object y=8 gives old bottom
        // 12. Changing Con from FullCon to FullCon/2 jolts Hgt 4->2 and VtxY 4->2,
        // then bottom preservation moves y to 12 - 2 - 0 = 10.
        let mut definition = simple_definition("Structure");
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 4)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 4).with_cnat(CNAT_BOTTOM)]);

        let mut engine = Engine::with_seed(65);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("Structure")
                .with_position(Vector2::new(3, 8))
                .with_construction(FULL_CON),
        )?;

        engine.apply_object_update(id, ObjectUpdate::new().with_construction(FULL_CON / 2))?;

        let object = engine.object_snapshot(id).expect("object present");
        assert_eq!(object.construction, FULL_CON / 2);
        assert_eq!(object.position, Vector2::new(3, 10));
        assert_eq!(object.vertices[0].y, 2);
        assert_eq!(object.vertices[0].cnat, CNAT_BOTTOM);
        Ok(())
    }

    #[test]
    fn construction_owned_vertices_survive_restore_like_cpp() -> Result<(), EngineError> {
        // Mirrors src/C4Object.cpp:2769 and src/C4Shape.cpp:486-494: saved
        // objects persist the `OwnVertices` flag, and own original vertices are
        // stored separately from the active shape. UpdateShape then copies from
        // that own base at src/C4Object.cpp:326 before non-stretch construction
        // calls C4Shape::Jolt at src/C4Shape.cpp:121-127.
        //
        // Hand-derived golden: the definition base vertex is y=4, but the owned
        // base vertex is y=8. After restore, changing Con from FullCon to
        // FullCon/2 must jolt the owned base to y=4, not the definition base to
        // y=2. The full shape y=8,h=4 has old bottom 12, so the straight-object
        // bottom preserve also moves y to 12 - 2 - 0 = 10.
        let mut definition = simple_definition("OwnedShape");
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 4)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 4).with_cnat(CNAT_BOTTOM)]);

        let mut engine = Engine::with_seed(66);
        engine.register_definition(definition.clone())?;
        let id = engine.spawn_object(
            SpawnConfig::new("OwnedShape")
                .with_position(Vector2::new(3, 8))
                .with_construction(FULL_CON)
                .with_vertices(vec![ObjectVertex::new(0, 8).with_cnat(CNAT_BOTTOM)]),
        )?;

        let state = engine.capture_state();
        let mut restored = Engine::with_seed(67);
        restored.register_definition(definition)?;
        restored.restore_state(&state)?;

        restored.apply_object_update(id, ObjectUpdate::new().with_construction(FULL_CON / 2))?;

        let object = restored.object_snapshot(id).expect("object present");
        assert_eq!(object.construction, FULL_CON / 2);
        assert_eq!(object.position, Vector2::new(3, 10));
        assert_eq!(object.vertices[0].y, 4);
        assert_eq!(object.vertices[0].cnat, CNAT_BOTTOM);
        Ok(())
    }

    #[test]
    fn construction_stretch_growth_scales_x_axis_like_cpp() -> Result<(), EngineError> {
        // Mirrors src/C4Def.cpp:387 and src/C4Object.cpp:329-333: DefCore
        // `StretchGrowth` sets `Def->GrowthType`, so UpdateShape calls
        // C4Shape::Stretch instead of Jolt. Stretch scales x/y/w/h and VtxX/VtxY
        // at src/C4Shape.cpp:105-116, then DoCon preserves the straight-object
        // bottom at src/C4Object.cpp:1462-1468.
        //
        // Hand-derived golden: shape x=2,w=6,h=4 and vertex (8,4) at 50%
        // construction stretch to shape x=1,w=3,h=2 and vertex (4,2). The old
        // bottom is y 8 + shape.y 0 + h 4 = 12, so bottom preservation moves y
        // to 12 - 2 - 0 = 10.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Stretch.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=STRG\nName=Stretch\nCategory=C4D_Object\nShape=2,0,6,4\nVertices=1\nVertexX=8\nVertexY=4\nVertexCNAT=8\nStretchGrowth=1\n",
        )
        .expect("write defcore");

        let group = lc_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;

        let mut engine = Engine::with_seed(68);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("STRG")
                .with_position(Vector2::new(3, 8))
                .with_construction(FULL_CON),
        )?;

        engine.apply_object_update(id, ObjectUpdate::new().with_construction(FULL_CON / 2))?;

        let object = engine.object_snapshot(id).expect("object present");
        assert_eq!(object.construction, FULL_CON / 2);
        assert_eq!(object.position, Vector2::new(3, 10));
        assert_eq!(object.vertices[0].x, 4);
        assert_eq!(object.vertices[0].y, 2);
        assert_eq!(object.vertices[0].cnat, CNAT_BOTTOM);
        Ok(())
    }

    #[test]
    fn border_bound_vertical_clamps_fixed_target_and_velocity() {
        let mut definition = simple_definition("Bounded");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition.set_border_bound(C4D_BORDER_TOP | C4D_BORDER_BOTTOM);

        let mut engine = Engine::with_seed(43);
        engine.set_landscape(Landscape::flat(10, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let top_id = engine
            .spawn_object(SpawnConfig::new("Bounded").with_position(Vector2::new(5, 2)))
            .expect("spawn succeeds");
        let top_idx = engine.find_object_index(top_id).expect("object exists");
        engine.objects[top_idx].set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, -itofix(5)));

        let bottom_id = engine
            .spawn_object(SpawnConfig::new("Bounded").with_position(Vector2::new(6, 18)))
            .expect("spawn succeeds");
        let bottom_idx = engine.find_object_index(bottom_id).expect("object exists");
        engine.objects[bottom_idx].set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(5)));

        let snapshot = engine.tick().expect("tick succeeds");
        let top = snapshot.object(top_id).expect("top object present");
        let bottom = snapshot.object(bottom_id).expect("bottom object present");
        assert_eq!(top.position.y, 1);
        assert_eq!(bottom.position.y, 19);

        let top_idx = engine.find_object_index(top_id).expect("object exists");
        assert_eq!(engine.objects[top_idx].fixed_position.y, itofix(1));
        assert_eq!(engine.objects[top_idx].fixed_velocity.y, C4Fixed::ZERO);
        let bottom_idx = engine.find_object_index(bottom_id).expect("object exists");
        assert_eq!(engine.objects[bottom_idx].fixed_position.y, itofix(19));
        assert_eq!(engine.objects[bottom_idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn attached_shape_checks_attachment_without_momentum_and_forces_jump_on_loss() {
        use std::sync::{Arc, Mutex};

        let script = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        global func OnSlideAbort(state, action) { return nil; }
        global func OnJumpStart(state, action) { return nil; }
        "#;
        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                if name == "OnSlideAbort" || name == "OnJumpStart" {
                    call_log.lock().unwrap().push(name.to_string());
                }
            });
        }

        let mut definition =
            Definition::from_script("Climber", "Climber", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Slide".to_string(),
            ActionSpec::default()
                .with_attach(CNAT_BOTTOM)
                .with_abort_call("OnSlideAbort"),
        );
        actions.insert(
            "Jump".to_string(),
            ActionSpec::default().with_start_call("OnJumpStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(47);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_position(Vector2::new(5, 5))
                    .with_action(ActionState::new("Slide")),
            )
            .expect("spawn succeeds");
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.velocity, Vector2::ZERO);

        let calls = call_log.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec!["OnSlideAbort".to_string(), "OnJumpStart".to_string()]
        );
    }

    #[test]
    fn attached_shape_keeps_action_when_attachment_is_still_present_without_momentum() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Climber");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Slide".to_string(),
            ActionSpec::default().with_attach(CNAT_BOTTOM),
        );
        actions.insert("Jump".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(49);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 7, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_position(Vector2::new(5, 5))
                    .with_action(ActionState::new("Slide")),
            )
            .expect("spawn succeeds");
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Slide");
        assert_eq!(object.position, Vector2::new(5, 5));
    }

    #[test]
    fn rotation_steps_rollback_on_shape_contact() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Wheel");
        definition.set_rotateable(360);
        definition.set_shape_vertices(vec![ObjectVertex::new(2, 0).with_cnat(CNAT_RIGHT)]);
        definition.set_contact_density(50);

        let mut engine = Engine::with_seed(43);
        engine.set_materials(materials);
        let mut surface = vec![20; 12];
        surface[6] = 0;
        let mut landscape =
            Landscape::new_with_material(12, surface, Some(earth)).expect("landscape constructs");
        landscape.fill_solid_material(Some(earth));
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Wheel").with_position(Vector2::new(4, 10)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].rotation_velocity = itofix(1);

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.rotation, 0);
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_rotation, itofix(0));
        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].state.vertices[0].x, 2);
    }

    #[test]
    fn set_x_dir_script_applies_subpixel_velocity_end_to_end() {
        // A script calling SetXDir(15) with the default precision (10) must set
        // xdir = itofix(15, 10) = 1.5 px/frame (raw 16.16 value 98304), matching
        // C++ FnSetXDir (`C4Script.cpp:697`) — NOT the pre-fix integer-mirror
        // behaviour that treated 15 as 15 whole px/frame (a 10x desync).
        let mut engine = Engine::with_seed(1);
        let definition = Definition::from_script(
            "Mover",
            "Mover",
            r#"
            global func Initialize(state, random) { SetXDir(15); return nil; }
            global func Step(state, frame, random) { return nil; }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(SpawnConfig::new("Mover").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");

        // Initialize ran at spawn: the live object holds true sub-pixel velocity.
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 98304);

        // One frame advances 1.5 px; fixtoi(1.5) = 2 (the old bug produced 15).
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position.x, 2);
        assert_eq!(object.velocity.x, 2);

        // A second frame accumulates to 3.0 px; fixtoi(3.0) = 3.
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position.x, 3);
    }

    #[test]
    fn set_r_dir_script_rotates_object_like_cpp() {
        // SetRDir(10) with the default precision (10) sets rdir = itofix(10, 10)
        // = 1.0 deg/frame (`C4Script.cpp:710`). C++ applies fix_r += rdir * 5
        // each frame (`C4Movement.cpp:376`), so the object turns 5°/frame.
        let mut engine = Engine::with_seed(1);
        let mut definition = Definition::from_script(
            "Spinner",
            "Spinner",
            r#"
            global func Initialize(state, random) { SetRDir(10); return nil; }
            global func Step(state, frame, random) { return nil; }
            "#,
        )
        .expect("script compiles");
        definition.set_rotateable(1);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(SpawnConfig::new("Spinner").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");

        // Initialize ran at spawn: rdir = itofix(10, 10) = 1.0 deg/frame (raw 65536).
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].rotation_velocity.val(), 65536);

        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 5);
        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 10);
    }

    #[test]
    fn set_r_dir_is_gated_by_rotateable_definition() {
        let mut engine = Engine::with_seed(2);
        let definition = Definition::from_script(
            "Fixed",
            "Fixed",
            r#"
            global func Initialize(state, random) { SetRDir(10); return nil; }
            global func Step(state, frame, random) { return nil; }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(SpawnConfig::new("Fixed").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].rotation_velocity.val(), 65536);

        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 0);
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].fixed_rotation, C4Fixed::ZERO);
    }

    #[test]
    fn finite_rotateable_range_clamps_fixed_rotation_and_stops_rdir() {
        let mut engine = Engine::with_seed(3);
        let mut definition = Definition::from_script(
            "Limited",
            "Limited",
            r#"
            global func Initialize(state, random) { SetRDir(10); return nil; }
            global func Step(state, frame, random) { return nil; }
            "#,
        )
        .expect("script compiles");
        definition.set_rotateable(4);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(SpawnConfig::new("Limited").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 4);
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_rotation, itofix(4));
        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);
    }

    #[test]
    fn rotateable_definition_reports_ocf_rotate() {
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Wheel");
        definition.set_rotateable(1);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Wheel").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");

        assert_ne!(engine.object_ocf_at_index(idx) & ocf::ROTATE, 0);
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
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

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
        assert_eq!(object.velocity, Vector2::new(3, -5));
        assert_eq!(object.position, Vector2::new(3, -5));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(6, -10));
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

    #[test]
    fn register_player_invokes_scenario_callbacks() -> Result<(), EngineError> {
        const SCRIPT: &str = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }

        global func PreInitializePlayer(state, player)
        {
            return { physics = { gravity = 100 } };
        }

        global func InitializePlayer(state, player, x, y, base, team, extra)
        {
            return {
                spawn = [
                    { definition = "Flag", owner = player, position = [x, y] }
                ]
            };
        }

        global func RemovePlayer(state, player, team) { return nil; }
        global func OnGameOver(state) { return nil; }
        "#;

        let mut engine = Engine::with_seed(5);

        let mut crew_def = simple_definition("Crew");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;

        let mut base_def = simple_definition("Base");
        base_def.set_category(CATEGORY_STRUCTURE);
        engine.register_definition(base_def)?;

        let mut flag_def = simple_definition("Flag");
        flag_def.set_category(CATEGORY_STRUCTURE);
        engine.register_definition(flag_def)?;

        let _crew_id = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(100, 200)),
        )?;
        let _base_id = engine.spawn_object(
            SpawnConfig::new("Base")
                .with_owner(1)
                .with_position(Vector2::new(150, 220)),
        )?;

        engine.install_scenario_script("Scenario", SCRIPT)?;

        engine.register_player(PlayerConfig::new(1, "Player"))?;

        assert_eq!(engine.physics().gravity, 100);

        let snapshot = engine.snapshot();
        let flag_snapshot = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "Flag")
            .expect("flag spawned by InitializePlayer");
        assert_eq!(flag_snapshot.owner, 1);
        assert_eq!(flag_snapshot.position, Vector2::new(100, 200));

        Ok(())
    }

    #[test]
    fn scenario_cast_particles_creates_and_executes_system_particles(
    ) -> Result<(), EngineError> {
        // End-to-end FnCastParticles → C4ParticleSystem::Cast → fxStdExec:
        // (C4Script.cpp:4881-4903, C4Particles.cpp:421-443,614-697).
        // level = 0 makes the cast velocity spread deterministic (zero), so
        // PushParticles and gravity fully determine the motion.
        const SCRIPT: &str = r#"
        global func Initialize(state, random) {
            CastParticles("Flame", 5, 0, 60, 40, 10, 20, 100, 200);
            PushParticles("Flame", 10, 0);
            return nil;
        }
        global func Step(state, frame, random) { return nil; }
        "#;
        let mut engine = Engine::with_seed(3);
        let core = particles::ParticleDefCore {
            name: "Flame".into(),
            init_fn: "StdInit".into(),
            exec_fn: "StdExec".into(),
            draw_fn: "Std".into(),
            gravity_acc: 100,
            delay: 1,
            repeats: 1000,
            ..Default::default()
        };
        engine
            .register_particle_definition(core, 8, 1.0)
            .expect("def registers");
        engine.install_scenario_script("Scenario", SCRIPT)?;

        let system = engine.particle_system();
        assert_eq!(system.particles().len(), 5, "cast created 5 particles");
        assert_eq!(system.get_def("Flame").unwrap().count, 5);
        for particle in system.particles() {
            assert_eq!(particle.x.to_bits(), 60.0f32.to_bits());
            assert_eq!(particle.y.to_bits(), 40.0f32.to_bits());
            assert_eq!(particle.xdir.to_bits(), 1.0f32.to_bits(), "pushed");
            assert_eq!(particle.ydir.to_bits(), 0.0f32.to_bits());
        }

        engine.tick()?;
        let gravity = engine.physics().gravity_as_c4fixed();
        let expected_ydir =
            math::fixtof(math::C4Fixed::from_raw(gravity.val().wrapping_mul(100))) / 100.0;
        for particle in engine.particle_system().particles() {
            assert_eq!(particle.x.to_bits(), 61.0f32.to_bits(), "moved by xdir");
            assert_eq!(particle.ydir.to_bits(), expected_ydir.to_bits(), "gravity");
            assert_eq!(particle.life, 1, "delay lifetime advanced");
        }

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot
                .particles
                .iter()
                .filter(|particle| particle.definition_id == "Flame")
                .count(),
            5,
            "system particles appear in the snapshot"
        );
        Ok(())
    }

    #[test]
    fn remove_player_triggers_on_game_over() -> Result<(), EngineError> {
        const SCRIPT: &str = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        global func PreInitializePlayer(state, player) { return nil; }
        global func InitializePlayer(state, player, x, y, base, team, extra) { return nil; }
        global func RemovePlayer(state, player, team)
        {
            return { physics = { gravity = 50 } };
        }
        global func OnGameOver(state)
        {
            return { physics = { gravity = 77 } };
        }
        "#;

        let mut engine = Engine::with_seed(11);
        engine.register_definition(simple_definition("Crew"))?;

        engine.install_scenario_script("Scenario", SCRIPT)?;

        engine.register_player(PlayerConfig::new(1, "Player"))?;

        let _ = engine.remove_player(1)?;

        assert_eq!(engine.physics().gravity, 77);

        Ok(())
    }

    #[test]
    fn script_game_over_triggers_on_game_over() -> Result<(), EngineError> {
        const SCRIPT: &str = r#"
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random)
        {
            if (frame == 1)
            {
                GameOver();
            }
            return nil;
        }
        global func OnGameOver(state)
        {
            return { physics = { gravity = 42 } };
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine.register_definition(simple_definition("Crew"))?;
        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(0)
                .with_crew_member(true)
                .with_position(Vector2::new(50, 50)),
        )?;

        engine.install_scenario_script("Scenario", SCRIPT)?;
        engine.register_player(PlayerConfig::new(0, "Player"))?;

        let snapshot = engine.tick()?;
        assert!(snapshot.game_over);
        assert_eq!(engine.physics().gravity, 42);

        Ok(())
    }

    #[test]
    fn legacy_create_object_objective_triggers_game_over() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(0);

        let mut crew_def = simple_definition("Crew");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;
        engine.register_definition(simple_definition("FLAG"))?;

        let objectives = ScenarioObjectives {
            create_objects: vec![CreateObjectObjective {
                definition: "FLAG".into(),
                count: 1,
            }],
            ..ScenarioObjectives::default()
        };
        engine.configure_objectives(objectives);

        engine.register_player(PlayerConfig::new(0, "Player"))?;

        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(0)
                .with_crew_member(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        engine.spawn_object(
            SpawnConfig::new("FLAG")
                .with_owner(0)
                .with_construction(FULL_CON),
        )?;

        let mut triggered = false;
        for _ in 0..40 {
            let snapshot = engine.tick()?;
            if snapshot.game_over {
                triggered = true;
                break;
            }
        }

        assert!(triggered, "expected game over once required object exists");
        Ok(())
    }

    #[test]
    fn legacy_clear_object_objective_triggers_after_removal() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(0);

        let mut crew_def = simple_definition("Crew");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;
        engine.register_definition(simple_definition("ROCK"))?;

        let objectives = ScenarioObjectives {
            clear_objects: vec![ClearObjectObjective {
                definition: "ROCK".into(),
                count: 0,
            }],
            ..ScenarioObjectives::default()
        };
        engine.configure_objectives(objectives);

        engine.register_player(PlayerConfig::new(0, "Player"))?;

        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(0)
                .with_crew_member(true)
                .with_position(Vector2::new(15, 20)),
        )?;
        let rock_id = engine.spawn_object(SpawnConfig::new("ROCK").with_owner(0))?;

        for _ in 0..5 {
            let snapshot = engine.tick()?;
            assert!(
                !snapshot.game_over,
                "game over should not trigger before removal"
            );
        }

        engine.apply_object_update(
            rock_id,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )?;

        // Process removal and allow periodic polling to run.
        let _ = engine.tick()?;

        let mut triggered = false;
        for _ in 0..40 {
            let snapshot = engine.tick()?;
            if snapshot.game_over {
                triggered = true;
                break;
            }
        }

        assert!(
            triggered,
            "expected game over once disallowed objects are cleared"
        );
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
        assert_eq!(state.environment, engine.environment());
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

    #[test]
    fn try_grab_nearby_moves_object_into_inventory() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        let mut crew_definition = Definition::from_script("Crew", "Crew", BASIC_OBJECT_SCRIPT)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_movement_profile(MovementProfile::default());
        engine.register_definition(crew_definition)?;

        let mut item_definition = Definition::from_script("Gem", "Gem", BASIC_OBJECT_SCRIPT)?;
        item_definition.set_ocf_base(ocf::GRAB | ocf::CARRYABLE);
        engine.register_definition(item_definition)?;

        let crew = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        let item =
            engine.spawn_object(SpawnConfig::new("Gem").with_position(Vector2::new(8, 0)))?;

        assert!(engine.try_grab_nearby(1)?);
        let snapshot = engine.object_snapshot(item).expect("item snapshot");
        assert_eq!(snapshot.container, Some(crew));
        Ok(())
    }

    #[test]
    fn try_drop_held_object_places_item_next_to_crew() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        let mut crew_definition = Definition::from_script("Crew", "Crew", BASIC_OBJECT_SCRIPT)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_movement_profile(MovementProfile::default());
        engine.register_definition(crew_definition)?;

        let mut item_definition = Definition::from_script("Gem", "Gem", BASIC_OBJECT_SCRIPT)?;
        item_definition.set_ocf_base(ocf::GRAB | ocf::CARRYABLE);
        engine.register_definition(item_definition)?;

        let crew = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        let item =
            engine.spawn_object(SpawnConfig::new("Gem").with_position(Vector2::new(6, 0)))?;

        assert!(engine.try_grab_nearby(1)?);
        let crew_before_drop = engine.object_snapshot(crew).expect("crew snapshot");
        assert!(engine.try_drop_held_object(1)?);
        let item_snapshot = engine.object_snapshot(item).expect("item snapshot");
        assert!(
            item_snapshot.container.is_none(),
            "item should be released from inventory"
        );
        assert_ne!(
            item_snapshot.position, crew_before_drop.position,
            "item should be positioned away from crew after drop"
        );
        Ok(())
    }

    #[test]
    fn auto_collect_moves_carryable_into_inventory() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        let mut crew_definition = Definition::from_script("Crew", "Crew", BASIC_OBJECT_SCRIPT)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_shape_rect(Some(DefinitionRect::new(-8, -16, 16, 32)));
        crew_definition.set_collection_rect(Some(DefinitionRect::new(-6, -12, 12, 24)));
        engine.register_definition(crew_definition)?;

        let mut item_definition = Definition::from_script("Gem", "Gem", BASIC_OBJECT_SCRIPT)?;
        item_definition.set_collectible(true);
        engine.register_definition(item_definition)?;

        let crew = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        let item =
            engine.spawn_object(SpawnConfig::new("Gem").with_position(Vector2::new(2, 0)))?;

        // Collection runs on Tick3 frames only (C4GameObjects.cpp:144-148).
        for _ in 0..3 {
            let _ = engine.tick()?;
        }

        let item_snapshot = engine.object_snapshot(item).expect("item snapshot");
        assert_eq!(item_snapshot.container, Some(crew));
        Ok(())
    }

    #[test]
    fn auto_collect_respects_collection_limit() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        let mut crew_definition = Definition::from_script("Crew", "Crew", BASIC_OBJECT_SCRIPT)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_shape_rect(Some(DefinitionRect::new(-8, -16, 16, 32)));
        crew_definition.set_collection_rect(Some(DefinitionRect::new(-6, -12, 12, 24)));
        crew_definition.set_collection_limit(Some(1));
        engine.register_definition(crew_definition)?;

        let mut item_definition = Definition::from_script("Gem", "Gem", BASIC_OBJECT_SCRIPT)?;
        item_definition.set_collectible(true);
        engine.register_definition(item_definition)?;

        let crew = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        let first =
            engine.spawn_object(SpawnConfig::new("Gem").with_position(Vector2::new(3, 0)))?;
        let second =
            engine.spawn_object(SpawnConfig::new("Gem").with_position(Vector2::new(-3, 0)))?;

        // Collection runs on Tick3 frames only (C4GameObjects.cpp:144-148).
        for _ in 0..3 {
            let _ = engine.tick()?;
        }

        let first_snapshot = engine.object_snapshot(first).expect("first item snapshot");
        let second_snapshot = engine
            .object_snapshot(second)
            .expect("second item snapshot");
        let collected = [first_snapshot.container, second_snapshot.container];
        assert_eq!(
            collected.iter().filter(|entry| entry.is_some()).count(),
            1,
            "exactly one item should be collected due to the limit"
        );
        assert_eq!(first_snapshot.container, Some(crew));
        assert!(second_snapshot.container.is_none());
        Ok(())
    }

    #[test]
    fn try_enter_nearby_moves_crew_into_structure() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        let mut crew_definition = Definition::from_script("Crew", "Crew", BASIC_OBJECT_SCRIPT)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_movement_profile(MovementProfile::default());
        engine.register_definition(crew_definition)?;

        let mut structure_definition = Definition::from_script("Hut", "Hut", BASIC_OBJECT_SCRIPT)?;
        structure_definition.set_ocf_base(ocf::ENTRANCE | ocf::CONTAINER);
        engine.register_definition(structure_definition)?;

        let crew = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        engine.select_crew(1, vec![crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        let hut = engine.spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(0, 0)))?;

        assert!(engine.try_enter_nearby(1)?);
        let crew_snapshot = engine.object_snapshot(crew).expect("crew snapshot");
        assert_eq!(crew_snapshot.container, Some(hut));
        Ok(())
    }
}
