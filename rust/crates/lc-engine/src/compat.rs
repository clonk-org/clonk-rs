use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::rc::Rc;

use crate::effect::{EffectCommand, EffectState, EffectVarValue};
use crate::{
    ActionLibrary, ActionProcedure, ActionUpdate, CommandDirection, DefinitionId, Direction,
    EnvironmentSettings, FloatVector2, Landscape, LiquidSegment, ObjectId, ObjectStatus,
    ObjectUpdate, ObjectVertex, ParticleCommand, ParticleConfig, ParticleLayer, ParticleScope,
    PhysicsSettings, QueuedCommand, SpawnConfig, Vector2, CNAT_BOTTOM, CNAT_CENTER, CNAT_LEFT,
    CNAT_NO_COLLISION, CNAT_RIGHT, CNAT_TOP, DEFAULT_CATEGORY, OWNER_NONE,
};
use lc_script::{Engine as ScriptEngine, RuntimeError, Value};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

thread_local! {
    static HOST_CONTEXT: RefCell<Option<EffectHostContext>> = const { RefCell::new(None) };
    static RANDOM_CONTEXT: RefCell<Option<Rc<RandomContext>>> = const { RefCell::new(None) };
    static ENVIRONMENT_CONTEXT: RefCell<Option<Rc<EnvironmentContext>>> = const {
        RefCell::new(None)
    };
    static PHYSICS_CONTEXT: RefCell<Option<Rc<PhysicsContext>>> = const {
        RefCell::new(None)
    };
}

const OWNER_ANY: i32 = -2;
const ANY_CONTAINER_SENTINEL: i32 = 123;
const NO_CONTAINER_SENTINEL: i32 = 124;
const MAX_VERTEX_COUNT: i32 = 30;

#[derive(Debug, Clone)]
pub(crate) struct HostWorldObject {
    pub id: ObjectId,
    definition_id: DefinitionId,
    status: ObjectStatus,
    alive: bool,
    pub action_name: String,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub action_procedure: Option<String>,
    pub owner: i32,
    pub category: i32,
    pub energy: i32,
    pub damage: i32,
    pub position: Vector2,
    #[allow(dead_code)]
    pub velocity: Vector2,
    pub vertices: Vec<ObjectVertex>,
    #[allow(dead_code)]
    pub action_data: i32,
    pub action_ticks: u32,
    container: Option<ObjectId>,
}

impl HostWorldObject {
    #[cfg(test)]
    pub(crate) fn new(
        id: ObjectId,
        definition_id: impl Into<String>,
        status: ObjectStatus,
        action_name: impl Into<String>,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        action_procedure: Option<String>,
        owner: i32,
        energy: i32,
        position: Vector2,
        velocity: Vector2,
        vertices: Vec<ObjectVertex>,
        action_data: i32,
        action_ticks: u32,
        container: Option<ObjectId>,
    ) -> Self {
        Self::with_category(
            id,
            definition_id,
            status,
            action_name,
            action_target,
            action_target2,
            action_procedure,
            owner,
            DEFAULT_CATEGORY,
            energy,
            0,
            position,
            velocity,
            vertices,
            action_data,
            action_ticks,
            container,
        )
    }

    pub(crate) fn with_category(
        id: ObjectId,
        definition_id: impl Into<String>,
        status: ObjectStatus,
        action_name: impl Into<String>,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        action_procedure: Option<String>,
        owner: i32,
        category: i32,
        energy: i32,
        damage: i32,
        position: Vector2,
        velocity: Vector2,
        vertices: Vec<ObjectVertex>,
        action_data: i32,
        action_ticks: u32,
        container: Option<ObjectId>,
    ) -> Self {
        Self {
            id,
            definition_id: definition_id.into(),
            status,
            alive: true,
            action_name: action_name.into(),
            action_target,
            action_target2,
            action_procedure,
            owner,
            category,
            energy,
            damage,
            position,
            velocity,
            vertices,
            action_data,
            action_ticks,
            container,
        }
    }

    pub(crate) fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }

    pub fn alive(&self) -> bool {
        self.alive
    }

    pub fn definition_id(&self) -> &str {
        &self.definition_id
    }

    pub fn status(&self) -> ObjectStatus {
        self.status
    }

    pub fn action_target(&self, index: usize) -> Option<ObjectId> {
        match index {
            0 => self.action_target,
            1 => self.action_target2,
            _ => None,
        }
    }

    pub fn procedure_name(&self) -> Option<&str> {
        self.action_procedure.as_deref()
    }

    pub fn owner(&self) -> i32 {
        self.owner
    }

    pub fn category(&self) -> i32 {
        self.category
    }

    pub fn energy(&self) -> i32 {
        self.energy
    }

    pub fn damage(&self) -> i32 {
        self.damage
    }

    pub fn action_name(&self) -> &str {
        &self.action_name
    }

    pub fn container(&self) -> Option<ObjectId> {
        self.container
    }

    pub fn position(&self) -> Vector2 {
        self.position
    }

    pub fn velocity(&self) -> Vector2 {
        self.velocity
    }

    pub fn vertices(&self) -> &[ObjectVertex] {
        &self.vertices
    }

    pub fn action_ticks(&self) -> u32 {
        self.action_ticks
    }

    #[allow(dead_code)]
    pub fn action_data(&self) -> i32 {
        self.action_data
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HostWorldContext {
    objects: Rc<HashMap<ObjectId, HostWorldObject>>,
    order: Rc<Vec<ObjectId>>,
    landscape: Option<Rc<Landscape>>,
    definitions: Rc<HashMap<DefinitionId, i32>>,
    next_object_id: u64,
}

impl Default for HostWorldContext {
    fn default() -> Self {
        Self {
            objects: Rc::new(HashMap::new()),
            order: Rc::new(Vec::new()),
            landscape: None,
            definitions: Rc::new(HashMap::new()),
            next_object_id: 1,
        }
    }
}

impl HostWorldContext {
    #[cfg(test)]
    pub(crate) fn from_objects<I>(objects: I) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        Self::with_landscape(objects, None, HashMap::new(), 1)
    }

    pub(crate) fn with_landscape<I>(
        objects: I,
        landscape: Option<Landscape>,
        definitions: HashMap<DefinitionId, i32>,
        next_object_id: u64,
    ) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        let map = objects.into_iter().collect::<Vec<HostWorldObject>>();
        let mut order = Vec::with_capacity(map.len());
        let mut lookup = HashMap::with_capacity(map.len());
        for object in map {
            let id = object.id;
            order.push(id);
            lookup.insert(id, object);
        }
        Self {
            objects: Rc::new(lookup),
            order: Rc::new(order),
            landscape: landscape.map(Rc::new),
            definitions: Rc::new(definitions),
            next_object_id,
        }
    }

    pub(crate) fn get(&self, id: ObjectId) -> Option<&HostWorldObject> {
        self.objects.get(&id)
    }

    pub(crate) fn object_ids(&self) -> &[ObjectId] {
        self.order.as_ref().as_slice()
    }

    pub(crate) fn landscape_ref(&self) -> Option<&Landscape> {
        self.landscape.as_deref()
    }

    pub(crate) fn next_object_id(&self) -> u64 {
        self.next_object_id
    }

    pub(crate) fn definition_category(&self, id: &str) -> Option<i32> {
        self.definitions.get(id).copied()
    }
}

trait WorldAccessor {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject>;
    fn object_ids(&self) -> Vec<ObjectId>;
}

impl WorldAccessor for HostWorldContext {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get(id).cloned()
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.object_ids().to_vec()
    }
}

impl WorldAccessor for EffectHostContext {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get_world_object(id)
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.world_object_ids()
    }
}

fn truncate_to_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn object_reference_value(id: ObjectId) -> Value {
    let mut map = HashMap::new();
    map.insert("id".into(), Value::Int(truncate_to_i32(id.as_u64())));
    Value::Proplist(map)
}

fn parse_object_reference_argument(
    value: &Value,
    function: &str,
    parameter: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    match value {
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) if *id >= 0 => Ok(Some(ObjectId::new(*id as u64))),
            _ => Ok(None),
        },
        Value::Nil => Ok(None),
        Value::Int(id) if *id == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "{}: expected proplist, nil, or 0 for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn value_to_i32(value: &Value, function: &str, parameter: &str) -> Result<i32, RuntimeError> {
    match value {
        Value::Int(int) => Ok(*int),
        other => Err(RuntimeError::new(format!(
            "{}: expected integer for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn value_to_bool(value: &Value, function: &str, parameter: &str) -> Result<bool, RuntimeError> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::Int(int) => Ok(*int != 0),
        Value::Nil => Ok(false),
        other => Err(RuntimeError::new(format!(
            "{}: expected bool or int for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn parse_optional_i32(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<i32>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        Some(Value::Int(int)) => Ok(Some(*int)),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected integer for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn parse_optional_u32(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<u32>, RuntimeError> {
    Ok(parse_optional_i32(value, function, parameter)?.map(|raw| raw.max(0) as u32))
}

fn parse_optional_string(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<String>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected string for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

fn c4id_to_definition(id: i32) -> Option<String> {
    if id == 0 {
        return None;
    }
    if (0..=9999).contains(&id) {
        return Some(format!("{id:04}"));
    }
    let raw = id as u32;
    let mut bytes = [0u8; 4];
    bytes[0] = (raw & 0x0000_00FF) as u8;
    bytes[1] = ((raw & 0x0000_FF00) >> 8) as u8;
    bytes[2] = ((raw & 0x00FF_0000) >> 16) as u8;
    bytes[3] = ((raw & 0xFF00_0000) >> 24) as u8;
    let end = bytes
        .iter()
        .rposition(|&b| b != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    match String::from_utf8(bytes[..end].to_vec()) {
        Ok(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn parse_definition_argument(
    value: Option<&Value>,
    function: &str,
) -> Result<Option<String>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Int(id)) => Ok(c4id_to_definition(*id)),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected definition identifier, got {}",
            function,
            other.type_name()
        ))),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerFilter {
    Any,
    Exact(ObjectId),
    RequiresContainer,
    RequiresNoContainer,
}

fn parse_container_filter(
    value: Option<&Value>,
    function: &str,
) -> Result<ContainerFilter, RuntimeError> {
    match value {
        None => Ok(ContainerFilter::Any),
        Some(Value::Nil) => Ok(ContainerFilter::Any),
        Some(Value::Int(raw)) if *raw == ANY_CONTAINER_SENTINEL => {
            Ok(ContainerFilter::RequiresContainer)
        }
        Some(Value::Int(raw)) if *raw == NO_CONTAINER_SENTINEL => {
            Ok(ContainerFilter::RequiresNoContainer)
        }
        Some(Value::Int(raw)) if *raw == 0 => Ok(ContainerFilter::Any),
        Some(Value::Proplist(map)) => match map.get("id") {
            Some(Value::Int(id)) if *id > 0 => {
                Ok(ContainerFilter::Exact(ObjectId::new(*id as u64)))
            }
            _ => Err(RuntimeError::new(format!(
                "{}: expected object reference proplist for container",
                function
            ))),
        },
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected object reference or container sentinel, got {}",
            function,
            other.type_name()
        ))),
    }
}

fn squared_distance(position: Vector2, x: i32, y: i32) -> i64 {
    let dx = position.x as i64 - x as i64;
    let dy = position.y as i64 - y as i64;
    dx * dx + dy * dy
}

struct FindObjectParams {
    definition: Option<String>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    _ocf: u32,
    action: Option<String>,
    treat_idle: bool,
    action_target: Option<ObjectId>,
    exclude: Option<ObjectId>,
    container: ContainerFilter,
    owner: i32,
    find_next: Option<ObjectId>,
}

impl FindObjectParams {
    fn parse(args: &[Value]) -> Result<Self, RuntimeError> {
        if args.len() > 12 {
            return Err(RuntimeError::new(
                "FindObject: expected at most 12 arguments",
            ));
        }

        let definition = parse_definition_argument(args.get(0), "FindObject")?;
        let x = parse_optional_i32(args.get(1), "FindObject", "x")?.unwrap_or(0);
        let y = parse_optional_i32(args.get(2), "FindObject", "y")?.unwrap_or(0);
        let width = parse_optional_i32(args.get(3), "FindObject", "width")?.unwrap_or(0);
        let height = parse_optional_i32(args.get(4), "FindObject", "height")?.unwrap_or(0);
        let ocf = parse_optional_u32(args.get(5), "FindObject", "ocf")?.unwrap_or(u32::MAX);
        let action = parse_optional_string(args.get(6), "FindObject", "action")?;
        let treat_idle = matches!(action.as_deref(), Some("Idle") | Some("ActIdle"));
        let action_target = parse_object_reference_argument(
            args.get(7).unwrap_or(&Value::Nil),
            "FindObject",
            "action_target",
        )?;
        let exclude = parse_object_reference_argument(
            args.get(8).unwrap_or(&Value::Nil),
            "FindObject",
            "exclude",
        )?;
        let container = parse_container_filter(args.get(9), "FindObject")?;
        let owner = parse_optional_i32(args.get(10), "FindObject", "owner")?.unwrap_or(OWNER_ANY);
        let find_next = parse_object_reference_argument(
            args.get(11).unwrap_or(&Value::Nil),
            "FindObject",
            "find_next",
        )?;

        Ok(Self {
            definition,
            x,
            y,
            width,
            height,
            _ocf: ocf,
            action,
            treat_idle,
            action_target,
            exclude,
            container,
            owner,
            find_next,
        })
    }

    fn is_full_range(&self) -> bool {
        self.x == 0 && self.y == 0 && self.width == 0 && self.height == 0
    }

    fn is_closest_query(&self) -> bool {
        self.width == -1 && self.height == -1
    }

    fn matches_object(&self, object: &HostWorldObject) -> bool {
        if matches!(object.status(), ObjectStatus::Deleted) {
            return false;
        }

        if let Some(exclude) = self.exclude {
            if object.id == exclude {
                return false;
            }
        }

        if let Some(definition) = &self.definition {
            if object.definition_id() != definition {
                return false;
            }
        }

        match self.container {
            ContainerFilter::Any => {}
            ContainerFilter::Exact(expected) => {
                if object.container() != Some(expected) {
                    return false;
                }
            }
            ContainerFilter::RequiresContainer => {
                if object.container().is_none() {
                    return false;
                }
            }
            ContainerFilter::RequiresNoContainer => {
                if object.container().is_some() {
                    return false;
                }
            }
        }

        if self.owner != OWNER_ANY && object.owner() != self.owner {
            return false;
        }

        if let Some(target) = self.action_target {
            let matches =
                object.action_target(0) == Some(target) || object.action_target(1) == Some(target);
            if !matches {
                return false;
            }
        }

        if let Some(action) = self.action.as_deref() {
            if !action.is_empty() {
                if self.treat_idle {
                    let name = object.action_name();
                    if name != "Idle" && name != "ActIdle" {
                        return false;
                    }
                } else if object.action_name() != action {
                    return false;
                }
            }
        }

        true
    }

    fn matches_area(&self, object: &HostWorldObject) -> bool {
        if self.is_full_range() || self.is_closest_query() {
            return true;
        }

        if self.width == 0 && self.height == 0 {
            let position = object.position();
            return position.x == self.x && position.y == self.y;
        }

        if self.width > 0 && self.height > 0 {
            let position = object.position();
            let dx = position.x - self.x;
            let dy = position.y - self.y;
            return dx >= 0 && dx <= self.width - 1 && dy >= 0 && dy <= self.height - 1;
        }

        false
    }

    fn reference_distance(&self, world: &impl WorldAccessor) -> Option<i64> {
        let id = self.find_next?;
        let object = world.get_object(id)?;
        Some(squared_distance(object.position(), self.x, self.y))
    }
}

fn energy_to_script_value(energy: i32) -> i32 {
    if energy <= DEFAULT_MAX_ENERGY {
        energy
    } else {
        let numerator = (energy as i64) * 100;
        let denominator = LEGACY_MAX_PHYSICAL as i64;
        (numerator / denominator) as i32
    }
}

const LEGACY_MAX_PHYSICAL: i32 = 100_000;
const CONTACT_DIRECTION_MASK: u32 = CNAT_LEFT | CNAT_RIGHT | CNAT_TOP | CNAT_BOTTOM | CNAT_CENTER;

fn compute_vertex_contact(
    landscape: Option<&Landscape>,
    position: Vector2,
    vertex: &ObjectVertex,
    check_mask: u32,
) -> u32 {
    if vertex.cnat & CNAT_NO_COLLISION != 0 {
        return 0;
    }
    let mask = if check_mask == 0 {
        vertex.cnat
    } else {
        check_mask
    };
    let mask = mask & CONTACT_DIRECTION_MASK;
    if mask == 0 {
        return 0;
    }
    let landscape = match landscape {
        Some(value) => value,
        None => return 0,
    };
    let world_x = position.x.saturating_add(vertex.x);
    let world_y = position.y.saturating_add(vertex.y);
    let mut contact = 0;
    if (mask & CNAT_CENTER) != 0 && landscape.is_solid_at(world_x, world_y) {
        contact |= CNAT_CENTER;
    }
    if (mask & CNAT_LEFT) != 0 && landscape.is_solid_at(world_x - 1, world_y) {
        contact |= CNAT_LEFT;
    }
    if (mask & CNAT_RIGHT) != 0 && landscape.is_solid_at(world_x + 1, world_y) {
        contact |= CNAT_RIGHT;
    }
    if (mask & CNAT_TOP) != 0 && landscape.is_solid_at(world_x, world_y - 1) {
        contact |= CNAT_TOP;
    }
    if (mask & CNAT_BOTTOM) != 0 && landscape.is_solid_at(world_x, world_y + 1) {
        contact |= CNAT_BOTTOM;
    }
    contact
}

fn resolve_vertices<'a>(
    context: &'a EffectHostContext,
    target: Option<ObjectId>,
) -> Option<(Vector2, &'a [ObjectVertex])> {
    if let Some(target_id) = target {
        if let Some(object) = context.object_context() {
            if object.id() == target_id {
                return Some((object.effective_position(), object.vertices()));
            }
        }
        context
            .world
            .get(target_id)
            .map(|other| (other.position(), other.vertices()))
    } else {
        context
            .object_context()
            .map(|object| (object.effective_position(), object.vertices()))
    }
}

const DEFAULT_MAX_ENERGY: i32 = 100;
const DEFAULT_VELOCITY_PRECISION: i32 = 10;

fn normalise_precision(value: i32) -> i32 {
    let value = if value == 0 {
        DEFAULT_VELOCITY_PRECISION
    } else {
        value
    };
    let normalised = value.abs();
    if normalised == 0 {
        1
    } else {
        normalised
    }
}

fn scale_velocity_value(value: i32, from_precision: i32, to_precision: i32) -> i32 {
    let from = normalise_precision(from_precision);
    let to = normalise_precision(to_precision);
    let numerator = i64::from(value) * i64::from(to);
    let divisor = i64::from(from);
    if divisor == 0 {
        return 0;
    }
    let adjusted = if numerator >= 0 {
        numerator + divisor / 2
    } else {
        numerator - divisor / 2
    };
    let scaled = adjusted / divisor;
    scaled.max(i64::from(i32::MIN)).min(i64::from(i32::MAX)) as i32
}

pub fn register_host_functions(script: &mut ScriptEngine) {
    script.register_host_function("AddEffect", add_effect);
    script.register_host_function("RemoveEffect", remove_effect);
    script.register_host_function("GetEffect", get_effect);
    script.register_host_function("GetEffectCount", get_effect_count);
    script.register_host_function("EffectVar", effect_var);
    script.register_host_function("SetAction", set_action);
    script.register_host_function("SetActionData", set_action_data);
    script.register_host_function("GetActionData", get_action_data);
    script.register_host_function("GetAction", get_action);
    script.register_host_function("GetActTime", get_act_time);
    script.register_host_function("GetProcedure", get_procedure);
    script.register_host_function("SetActionTargets", set_action_targets);
    script.register_host_function("GetActionTarget", get_action_target);
    script.register_host_function("GetVertexNum", get_vertex_num);
    script.register_host_function("GetVertex", get_vertex);
    script.register_host_function("GetVertexContact", get_vertex_contact);
    script.register_host_function("GetContact", get_contact);
    script.register_host_function("PathFree", path_free);
    script.register_host_function("GBackSolid", g_back_solid);
    script.register_host_function("GBackSemiSolid", g_back_semi_solid);
    script.register_host_function("GBackLiquid", g_back_liquid);
    script.register_host_function("GBackSky", g_back_sky);
    script.register_host_function("SetDir", set_dir);
    script.register_host_function("GetDir", get_dir);
    script.register_host_function("SetComDir", set_com_dir);
    script.register_host_function("GetComDir", get_com_dir);
    script.register_host_function("SetXDir", set_x_dir);
    script.register_host_function("GetXDir", get_x_dir);
    script.register_host_function("SetYDir", set_y_dir);
    script.register_host_function("GetYDir", get_y_dir);
    script.register_host_function("FindObject", find_object);
    script.register_host_function("FindObjects", find_objects);
    script.register_host_function("GetX", get_x);
    script.register_host_function("GetY", get_y);
    script.register_host_function("SetPosition", set_position);
    script.register_host_function("CreateObject", create_object);
    script.register_host_function("CreateParticle", create_particle);
    script.register_host_function("ClearParticles", clear_particles);
    script.register_host_function("Contained", contained);
    script.register_host_function("GetCategory", get_category);
    script.register_host_function("SetCategory", set_category);
    script.register_host_function("SetAlive", set_alive);
    script.register_host_function("GetAlive", get_alive);
    script.register_host_function("SetOwner", set_owner);
    script.register_host_function("GetOwner", get_owner);
    script.register_host_function("SetObjectStatus", set_object_status);
    script.register_host_function("GetObjectStatus", get_object_status);
    script.register_host_function("RemoveObject", remove_object);
    script.register_host_function("GetEnergy", get_energy);
    script.register_host_function("DoEnergy", do_energy);
    script.register_host_function("DoDamage", do_damage);
    script.register_host_function("Random", random);
    script.register_host_function("SetGravity", set_gravity);
    script.register_host_function("GetGravity", get_gravity);
    script.register_host_function("SetWind", set_wind);
    script.register_host_function("GetWind", get_wind);
    script.register_host_function("SetTemperature", set_temperature);
    script.register_host_function("GetTemperature", get_temperature);
    script.register_host_function("SetClimate", set_climate);
    script.register_host_function("GetClimate", get_climate);
}

pub(crate) fn enter_random_context(rng: ChaCha8Rng) -> RandomContextGuard {
    RANDOM_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested random contexts are not supported"
        );
        let context = Rc::new(RandomContext {
            rng: RefCell::new(rng),
        });
        *cell.borrow_mut() = Some(context.clone());
        RandomContextGuard {
            context: Some(context),
        }
    })
}

#[derive(Clone, Debug)]
pub(crate) struct HostObjectContext<'a> {
    pub id: ObjectId,
    pub container: Option<ObjectId>,
    pub status: ObjectStatus,
    pub energy: i32,
    pub damage: i32,
    pub alive: bool,
    pub owner: i32,
    pub category: i32,
    pub position: Vector2,
    pub velocity: Vector2,
    pub effects: &'a [EffectState],
    pub action_name: String,
    pub action_ticks: u32,
    pub action_data: i32,
    pub action_library: ActionLibrary,
    pub direction: Direction,
    pub command_direction: CommandDirection,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub vertices: &'a [ObjectVertex],
}

impl<'a> HostObjectContext<'a> {
    #[cfg(test)]
    pub fn new(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        effects: &'a [EffectState],
        action_name: impl Into<String>,
        action_ticks: u32,
        action_data: i32,
        action_library: ActionLibrary,
        direction: Direction,
        command_direction: CommandDirection,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: &'a [ObjectVertex],
    ) -> Self {
        Self::with_category(
            id,
            container,
            status,
            energy,
            0,
            owner,
            position,
            velocity,
            effects,
            action_name,
            action_ticks,
            action_data,
            action_library,
            direction,
            command_direction,
            action_target,
            action_target2,
            vertices,
            DEFAULT_CATEGORY,
        )
    }

    pub fn with_category(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        damage: i32,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        effects: &'a [EffectState],
        action_name: impl Into<String>,
        action_ticks: u32,
        action_data: i32,
        action_library: ActionLibrary,
        direction: Direction,
        command_direction: CommandDirection,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: &'a [ObjectVertex],
        category: i32,
    ) -> Self {
        Self {
            id,
            container,
            status,
            energy,
            damage,
            alive: true,
            owner,
            category,
            position,
            velocity,
            effects,
            action_name: action_name.into(),
            action_ticks,
            action_data,
            action_library,
            direction,
            command_direction,
            action_target,
            action_target2,
            vertices,
        }
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PhysicsDelta {
    pub gravity: Option<i32>,
}

impl PhysicsDelta {
    pub fn is_empty(&self) -> bool {
        self.gravity.is_none()
    }

    pub fn apply(&self, physics: &mut PhysicsSettings) {
        if let Some(gravity) = self.gravity {
            physics.gravity = gravity.clamp(-300, 300);
        }
    }
}

#[derive(Debug)]
struct PhysicsContext {
    settings: RefCell<PhysicsSettings>,
    pending: RefCell<PhysicsDelta>,
}

impl PhysicsContext {
    fn new(settings: PhysicsSettings) -> Self {
        Self {
            settings: RefCell::new(settings),
            pending: RefCell::new(PhysicsDelta::default()),
        }
    }

    fn set_gravity(&self, gravity: i32) {
        let clamped = gravity.clamp(-300, 300);
        self.settings.borrow_mut().gravity = clamped;
        self.pending.borrow_mut().gravity = Some(clamped);
    }

    fn gravity(&self) -> i32 {
        self.settings.borrow().gravity
    }

    fn into_delta(self) -> PhysicsDelta {
        self.pending.into_inner()
    }
}

pub(crate) struct PhysicsContextGuard {
    context: Option<Rc<PhysicsContext>>,
}

impl PhysicsContextGuard {
    pub fn finish(mut self) -> PhysicsDelta {
        let context = self
            .context
            .take()
            .expect("physics context already consumed");
        PHYSICS_CONTEXT.with(|cell| {
            let stored = cell
                .borrow_mut()
                .take()
                .expect("physics context must be present");
            debug_assert!(Rc::ptr_eq(&stored, &context));
        });
        Rc::try_unwrap(context)
            .expect("physics context still referenced")
            .into_delta()
    }
}

impl Drop for PhysicsContextGuard {
    fn drop(&mut self) {
        if self.context.is_some() {
            PHYSICS_CONTEXT.with(|cell| {
                cell.borrow_mut().take();
            });
        }
    }
}

pub(crate) fn enter_physics_context(settings: PhysicsSettings) -> PhysicsContextGuard {
    PHYSICS_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested physics contexts are not supported",
        );
        let context = Rc::new(PhysicsContext::new(settings));
        *cell.borrow_mut() = Some(context.clone());
        PhysicsContextGuard {
            context: Some(context),
        }
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EnvironmentDelta {
    pub wind: Option<i32>,
    pub temperature: Option<i32>,
    pub climate: Option<i32>,
}

impl EnvironmentDelta {
    pub(crate) fn is_empty(&self) -> bool {
        self.wind.is_none() && self.temperature.is_none() && self.climate.is_none()
    }

    pub fn apply(&self, environment: &mut EnvironmentSettings) {
        if let Some(wind) = self.wind {
            environment.wind = wind.clamp(-100, 100);
        }
        if let Some(temperature) = self.temperature {
            environment.temperature = temperature.clamp(-100, 100);
        }
        if let Some(climate) = self.climate {
            environment.climate = climate.clamp(-50, 50);
        }
    }
}

#[derive(Debug)]
struct EnvironmentContext {
    settings: RefCell<EnvironmentSettings>,
    frame: u64,
    pending: RefCell<EnvironmentDelta>,
}

impl EnvironmentContext {
    fn new(settings: EnvironmentSettings, frame: u64) -> Self {
        Self {
            settings: RefCell::new(settings),
            frame,
            pending: RefCell::new(EnvironmentDelta::default()),
        }
    }

    fn set_wind(&self, wind: i32) {
        let clamped = wind.clamp(-100, 100);
        self.settings.borrow_mut().wind = clamped;
        self.pending.borrow_mut().wind = Some(clamped);
    }

    fn wind_force(&self) -> i32 {
        let settings = self.settings.borrow();
        settings.wind_force(self.frame)
    }

    fn set_temperature(&self, temperature: i32) {
        let clamped = temperature.clamp(-100, 100);
        self.settings.borrow_mut().temperature = clamped;
        self.pending.borrow_mut().temperature = Some(clamped);
    }

    fn ambient_temperature(&self) -> i32 {
        let settings = self.settings.borrow();
        settings.ambient_temperature(self.frame)
    }

    fn set_climate(&self, climate: i32) {
        let clamped = climate.clamp(-50, 50);
        self.settings.borrow_mut().climate = clamped;
        self.pending.borrow_mut().climate = Some(clamped);
    }

    fn climate(&self) -> i32 {
        self.settings.borrow().climate
    }

    fn into_delta(self) -> EnvironmentDelta {
        self.pending.into_inner()
    }
}

pub(crate) struct EnvironmentContextGuard {
    context: Option<Rc<EnvironmentContext>>,
}

impl EnvironmentContextGuard {
    pub fn finish(mut self) -> EnvironmentDelta {
        let context = self
            .context
            .take()
            .expect("environment context already consumed");
        ENVIRONMENT_CONTEXT.with(|cell| {
            let stored = cell
                .borrow_mut()
                .take()
                .expect("environment context must be present");
            debug_assert!(Rc::ptr_eq(&stored, &context));
        });
        Rc::try_unwrap(context)
            .expect("environment context still referenced")
            .into_delta()
    }
}

impl Drop for EnvironmentContextGuard {
    fn drop(&mut self) {
        if self.context.is_some() {
            ENVIRONMENT_CONTEXT.with(|cell| {
                cell.borrow_mut().take();
            });
        }
    }
}

pub(crate) fn enter_environment_context(
    settings: EnvironmentSettings,
    frame: u64,
) -> EnvironmentContextGuard {
    ENVIRONMENT_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested environment contexts are not supported",
        );
        let context = Rc::new(EnvironmentContext::new(settings, frame));
        *cell.borrow_mut() = Some(context.clone());
        EnvironmentContextGuard {
            context: Some(context),
        }
    })
}

pub(crate) fn with_effect_context<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
{
    HOST_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested effect contexts are not supported"
        );
        *cell.borrow_mut() = Some(EffectHostContext::new(
            object,
            global_effects.to_vec(),
            world,
            next_object_id,
        ));
        let result = func();
        let context = cell
            .borrow_mut()
            .take()
            .expect("effect context must be present");
        (result, context.into_commands())
    })
}

#[derive(Debug, Clone)]
pub(crate) struct EffectContextOutcome {
    pub object: Vec<EffectCommand>,
    pub global: Vec<EffectCommand>,
    pub object_update: Option<ObjectUpdate>,
    pub object_commands: Vec<QueuedCommand>,
    pub destroy_object: bool,
    pub environment: Option<EnvironmentDelta>,
    pub physics: Option<PhysicsDelta>,
    pub spawns: Vec<SpawnConfig>,
    pub particles: Vec<ParticleCommand>,
    pub next_object_id: u64,
}

impl EffectContextOutcome {
    fn new(
        object: Vec<EffectCommand>,
        global: Vec<EffectCommand>,
        object_update: Option<ObjectUpdate>,
        object_commands: Vec<QueuedCommand>,
        destroy_object: bool,
        environment: Option<EnvironmentDelta>,
        physics: Option<PhysicsDelta>,
        spawns: Vec<SpawnConfig>,
        next_object_id: u64,
    ) -> Self {
        Self {
            object,
            global,
            object_update,
            object_commands,
            destroy_object,
            environment,
            physics,
            spawns,
            particles: Vec::new(),
            next_object_id,
        }
    }

    pub(crate) fn empty(next_object_id: u64) -> Self {
        Self {
            object: Vec::new(),
            global: Vec::new(),
            object_update: None,
            object_commands: Vec::new(),
            destroy_object: false,
            environment: None,
            physics: None,
            spawns: Vec::new(),
            particles: Vec::new(),
            next_object_id,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum EffectScope {
    Object,
    Global,
}

#[derive(Debug)]
struct RandomContext {
    rng: RefCell<ChaCha8Rng>,
}

impl RandomContext {
    fn into_rng(self) -> ChaCha8Rng {
        self.rng.into_inner()
    }
}

pub(crate) struct RandomContextGuard {
    context: Option<Rc<RandomContext>>,
}

impl RandomContextGuard {
    pub fn finish(mut self) -> ChaCha8Rng {
        let context = self
            .context
            .take()
            .expect("random context already consumed");
        RANDOM_CONTEXT.with(|cell| {
            let stored = cell
                .borrow_mut()
                .take()
                .expect("random context must be present");
            debug_assert!(Rc::ptr_eq(&stored, &context));
        });
        Rc::try_unwrap(context)
            .expect("random context still referenced")
            .into_rng()
    }
}

impl Drop for RandomContextGuard {
    fn drop(&mut self) {
        if self.context.is_some() {
            RANDOM_CONTEXT.with(|cell| {
                cell.borrow_mut().take();
            });
        }
    }
}

fn add_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "AddEffect expects at least 2 arguments: name and state",
        ));
    }

    let name = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Int(0)),
        other => {
            return Err(RuntimeError::new(format!(
                "AddEffect: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;
    if matches!(scope, EffectScope::Object) {
        match &args[1] {
            Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected proplist for object state, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let priority = match args.get(2) {
        Some(Value::Int(value)) => *value,
        Some(Value::Nil) | None => 100,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "AddEffect: expected int for priority, got {}",
                other.type_name()
            )))
        }
    };

    if priority == 0 {
        return Ok(Value::Int(0));
    }

    let interval = match args.get(3) {
        Some(Value::Int(value)) if *value > 0 => *value,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "AddEffect: interval must be > 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 1,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "AddEffect: expected int for interval, got {}",
                other.type_name()
            )))
        }
    };

    let len = args.len();
    let mut idx = 4;
    let mut command_target: Option<i32> = None;
    let mut command_target_id: Option<String> = None;
    let mut timer: Option<i32> = None;
    let mut vars: Vec<EffectVarValue> = Vec::new();

    if idx < len {
        match &args[idx] {
            Value::Proplist(_) | Value::Nil => {
                command_target = parse_command_target(&args[idx])?;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && len > idx + 1 => {
                command_target = None;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && len == idx + 1 => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            Value::Int(value) if len == idx + 1 => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected proplist, nil, or int for command target, got {}",
                    other.type_name()
                )));
            }
        }
    }

    if idx < len {
        match &args[idx] {
            Value::String(_) | Value::Nil => {
                command_target_id = parse_command_target_id(&args[idx])?;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && idx < len - 1 => {
                command_target_id = None;
                idx += 1;
            }
            Value::Int(value) if *value == 0 && timer.is_none() && idx == len - 1 => {
                command_target_id = None;
                idx += 1;
            }
            Value::Int(value) if timer.is_none() && idx == len - 1 => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            Value::Int(_) => {
                return Err(RuntimeError::new(
                    "AddEffect: command target id must be string, nil, or 0",
                ));
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected string or nil for command target id, got {}",
                    other.type_name()
                )));
            }
        }
    }

    if idx < len {
        match &args[idx] {
            Value::Int(value) if timer.is_none() => {
                timer = Some(parse_timer_from_int(*value)?);
                idx += 1;
            }
            Value::Nil => {
                idx += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected int or nil for timer, got {}",
                    other.type_name()
                )));
            }
        }
    }

    while idx < len {
        let value = &args[idx];
        vars.push(value_to_effect_var(value));
        idx += 1;
    }

    let identifier = with_context_mut(scope, move |ctx| {
        let mut effect = EffectState::new(name)
            .with_priority(priority)
            .with_interval(interval);
        if let Some(timer) = timer {
            effect = effect.with_timer(timer);
        }
        effect = effect.with_command_target(command_target);
        effect = effect.with_command_id(command_target_id);
        if !vars.is_empty() {
            effect = effect.with_vars(vars);
        }
        ctx.add_effect(effect)
    })?;

    Ok(Value::Int(identifier))
}

fn remove_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "RemoveEffect expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.clone()),
        Value::String(_) | Value::Nil => None,
        Value::Int(value) if *value == 0 => None,
        other => {
            return Err(RuntimeError::new(format!(
                "RemoveEffect: expected string, nil, or 0 for name, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;
    if matches!(scope, EffectScope::Object) {
        match &args[1] {
            Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "RemoveEffect: expected proplist for object state, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let index = match args.get(2) {
        Some(Value::Int(value)) if *value >= 0 => *value as usize,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "RemoveEffect: index must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "RemoveEffect: expected int for index, got {}",
                other.type_name()
            )))
        }
    };

    let mut no_callbacks = false;
    if let Some(flag) = args.get(3) {
        match flag {
            Value::Bool(value) => no_callbacks = *value,
            Value::Nil => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "RemoveEffect: expected bool or nil for no-call flag, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let removed = with_context_mut(scope, |ctx| {
        ctx.remove_effect(name_filter.as_deref(), index, no_callbacks)
    })?;
    Ok(Value::Bool(removed))
}

fn get_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetEffect expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.as_str()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object => extract_effects_from_state(&args[1])?,
            EffectScope::Global => Vec::new(),
        },
    };

    let desired_index = match args.get(2) {
        Some(Value::Int(value)) if *value >= 0 => *value as usize,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffect: index argument must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for index, got {}",
                other.type_name()
            )))
        }
    };

    let query = match args.get(3) {
        Some(Value::Int(value)) => *value,
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for query, got {}",
                other.type_name()
            )))
        }
    };

    let max_priority = match args.get(4) {
        Some(Value::Int(value)) if *value >= 0 => Some(*value),
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffect: max priority must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => None,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for max priority, got {}",
                other.type_name()
            )))
        }
    };

    let mut match_index = 0;
    for effect in &effects {
        if let Some(filter) = name_filter {
            if effect.name != filter {
                continue;
            }
        }

        if let Some(limit) = max_priority {
            if effect.priority.abs() > limit {
                continue;
            }
        }

        if match_index == desired_index {
            return Ok(match query {
                0 => {
                    let identifier = match_index.saturating_add(1);
                    let id = i32::try_from(identifier).unwrap_or(i32::MAX);
                    Value::Int(id)
                }
                1 => Value::String(effect.name.clone()),
                2 => Value::Int(effect.priority),
                3 => Value::Int(effect.interval),
                4 => effect.command_target.map(Value::Int).unwrap_or(Value::Nil),
                5 => effect
                    .command_id
                    .as_ref()
                    .map(|id| Value::String(id.clone()))
                    .unwrap_or(Value::Nil),
                6 => Value::Int(effect.timer),
                _ => build_effect_value(effect),
            });
        }

        match_index += 1;
    }

    Ok(Value::Nil)
}

fn get_effect_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetEffectCount expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.as_str()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffectCount: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object => extract_effects_from_state(&args[1])?,
            EffectScope::Global => Vec::new(),
        },
    };

    let max_priority = match args.get(2) {
        Some(Value::Int(value)) if *value >= 0 => Some(*value),
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffectCount: max priority must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => None,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffectCount: expected int for max priority, got {}",
                other.type_name()
            )))
        }
    };

    let count = effects
        .iter()
        .filter(|effect| {
            if let Some(filter) = name_filter {
                if effect.name != filter {
                    return false;
                }
            }
            if let Some(limit) = max_priority {
                if effect.priority.abs() > limit {
                    return false;
                }
            }
            true
        })
        .count();

    let count = i32::try_from(count).unwrap_or(i32::MAX);
    Ok(Value::Int(count))
}

fn effect_var(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 3 {
        return Err(RuntimeError::new(
            "EffectVar expects at least 3 arguments: index, state, and number",
        ));
    }

    let var_index = match &args[0] {
        Value::Int(value) if *value >= 0 => *value as usize,
        Value::Int(_) | Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "EffectVar: expected int for index, got {}",
                other.type_name()
            )))
        }
    };

    let scope = determine_scope_from_state(&args[1])?;

    let effect_number = match &args[2] {
        Value::Int(value) if *value > 0 => *value as usize,
        Value::Int(_) | Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "EffectVar: expected positive int for number, got {}",
                other.type_name()
            )))
        }
    };

    let new_value = args.get(3).map(value_to_effect_var);

    let context_value = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(None);
        };

        match context.scope_mut(scope) {
            Ok(stack) => Ok(stack.effect_var(effect_number, var_index, new_value.clone())),
            Err(_) => Ok(None),
        }
    })?;

    if let Some(value) = context_value {
        return Ok(effect_var_to_value(&value));
    }

    if new_value.is_some() {
        return Err(RuntimeError::new(
            "EffectVar: setting variables requires an active engine context",
        ));
    }

    let effects = extract_effects_from_state(&args[1])?;
    if effect_number == 0 || effect_number > effects.len() {
        return Ok(Value::Nil);
    }
    let effect = &effects[effect_number - 1];
    let value = effect.var(var_index);
    Ok(effect_var_to_value(&value))
}

fn random(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "Random expects exactly 1 argument: upper exclusive bound",
        ));
    }

    let range = match &args[0] {
        Value::Int(value) => *value,
        other => {
            return Err(RuntimeError::new(format!(
                "Random: expected int for range, got {}",
                other.type_name()
            )))
        }
    };

    if range <= 0 {
        return Ok(Value::Int(0));
    }

    RANDOM_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("Random: host context unavailable"))?
            .clone();
        let mut rng = context.rng.borrow_mut();
        let upper = range as u32;
        let value = rng.gen_range(0..upper) as i32;
        Ok(Value::Int(value))
    })
}

fn set_gravity(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetGravity expects 1 argument: gravity"));
    }

    let gravity = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetGravity: expected int or nil for gravity, got {}",
                other.type_name()
            )))
        }
    };

    PHYSICS_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetGravity requires an active engine context"))?
            .clone();
        context.set_gravity(gravity);
        Ok(Value::Nil)
    })
}

fn get_gravity(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetGravity does not accept any arguments",
        ));
    }

    PHYSICS_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetGravity requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.gravity()))
    })
}

fn set_wind(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetWind expects 1 argument: wind"));
    }

    let wind = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetWind: expected int or nil for wind, got {}",
                other.type_name()
            )))
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetWind requires an active engine context"))?
            .clone();
        context.set_wind(wind);
        Ok(Value::Nil)
    })
}

fn get_wind(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "GetWind expects at most 3 arguments: x, y, global",
        ));
    }

    for (index, arg) in args.iter().enumerate() {
        match arg {
            Value::Int(_) | Value::Nil => {}
            Value::Bool(_) if index == 2 => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "GetWind: unexpected argument type {} at position {}",
                    other.type_name(),
                    index + 1
                )))
            }
        }
    }

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetWind requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.wind_force()))
    })
}

fn set_temperature(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetTemperature expects 1 argument: temperature",
        ));
    }

    let temperature = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetTemperature: expected int or nil for temperature, got {}",
                other.type_name()
            )))
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetTemperature requires an active engine context"))?
            .clone();
        context.set_temperature(temperature);
        Ok(Value::Nil)
    })
}

fn get_temperature(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetTemperature does not accept any arguments",
        ));
    }

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetTemperature requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.ambient_temperature()))
    })
}

fn set_climate(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetClimate expects 1 argument: climate"));
    }

    let climate = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetClimate: expected int or nil for climate, got {}",
                other.type_name()
            )))
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetClimate requires an active engine context"))?
            .clone();
        context.set_climate(climate);
        Ok(Value::Nil)
    })
}

fn get_climate(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetClimate does not accept any arguments",
        ));
    }

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetClimate requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.climate()))
    })
}

fn get_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetEnergy expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        target_id = parse_object_reference_argument(arg, "GetEnergy", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(Value::Int(energy_to_script_value(object.energy())));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(energy_to_script_value(other.energy())));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(energy_to_script_value(object.energy())))
    })
}

fn do_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "DoEnergy expects at least 1 argument: change",
        ));
    }

    let change = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoEnergy: expected int or nil for change, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            Value::Int(value) if *value == 0 => {
                index += 1;
            }
            Value::Int(value) if *value > 0 => {
                target_id = Some(ObjectId::new(*value as u64));
                index += 1;
            }
            _ => {}
        }
    }

    let mut exact = false;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Bool(flag) => {
                exact = *flag;
                index += 1;
            }
            Value::Int(value) => {
                exact = *value != 0;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected bool, int, or nil for exact flag, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected int or nil for cause, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected int or nil for caused by, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "DoEnergy: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DoEnergy requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.adjust_energy(change, exact);
        Ok(Value::Bool(true))
    })
}

fn do_damage(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "DoDamage expects at least 1 argument: change",
        ));
    }

    let change = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoDamage: expected int or nil for change, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            Value::Int(value) if *value == 0 => {
                index += 1;
            }
            Value::Int(value) if *value > 0 => {
                target_id = Some(ObjectId::new(*value as u64));
                index += 1;
            }
            _ => {}
        }
    }

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoDamage: expected int or nil for damage type, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(_) | Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoDamage: expected int or nil for caused by, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "DoDamage: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("DoDamage requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.adjust_damage(change);
        Ok(Value::Bool(true))
    })
}

fn set_action(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetAction expects at least 1 argument: action name",
        ));
    }

    let action_name = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.clone()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "SetAction: expected string or nil for action name, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    let mut phase: Option<i32> = None;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                phase = Some(*value);
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {
                return Err(RuntimeError::new(format!(
                    "SetAction: expected int or nil for phase, got {}",
                    arg.type_name()
                )))
            }
        }
    }

    let mut ticks: Option<u32> = None;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) if *value >= 0 => {
                ticks = Some(*value as u32);
                index += 1;
            }
            Value::Int(_) => {
                return Err(RuntimeError::new(
                    "SetAction: ticks must be >= 0 when provided",
                ));
            }
            Value::Nil => {
                index += 1;
            }
            _ => {
                return Err(RuntimeError::new(format!(
                    "SetAction: expected int or nil for ticks, got {}",
                    arg.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetAction: additional arguments are not supported",
        ));
    }

    let name = match action_name {
        Some(name) => name,
        None => return Ok(Value::Bool(false)),
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetAction requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        let current_action = object.effective_action_name().to_string();
        let blocks_other_actions = object.effective_blocks_other_actions();
        let changed_action = name != current_action;
        if blocks_other_actions && name != current_action {
            return Ok(Value::Bool(false));
        }

        let update = object
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_name(name.clone());
        update.set_force(false);
        if let Some(phase) = phase {
            update.set_phase(phase);
        }

        if let Some(ticks) = ticks {
            object.set_action_ticks(ticks);
        } else if changed_action {
            object.reset_action_ticks();
        }

        let procedure_changed = object.update_effective_action(&name);
        if procedure_changed {
            object.reset_action_data();
        }

        Ok(Value::Bool(true))
    })
}

fn set_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetActionData expects at least 1 argument: data",
        ));
    }

    let data = value_to_i32(&args[0], "SetActionData", "data")?;
    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetActionData", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetActionData: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetActionData requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        if !object.status().is_active() {
            return Ok(Value::Bool(false));
        }

        let procedure = object.effective_action_procedure();
        let mut next_data = data;
        match procedure {
            ActionProcedure::Bridge => {
                next_data = if data < 0 { 0xFF } else { data.min(0xFF) };
            }
            ActionProcedure::Attach => {
                let primary_vertex = (data & 0xFF) as i32;
                let secondary_vertex = data >> 8;
                if primary_vertex >= MAX_VERTEX_COUNT || secondary_vertex >= MAX_VERTEX_COUNT {
                    return Ok(Value::Bool(false));
                }
            }
            _ => {}
        }

        object.set_action_data(next_data);
        Ok(Value::Bool(true))
    })
}

fn set_action_targets(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;

    let (target1, update_target1) = if let Some(arg) = args.get(index) {
        let target = parse_object_reference_argument(arg, "SetActionTargets", "target1")?;
        index += 1;
        (target, true)
    } else {
        (None, false)
    };

    let (target2, update_target2) = if let Some(arg) = args.get(index) {
        let target = parse_object_reference_argument(arg, "SetActionTargets", "target2")?;
        index += 1;
        (target, true)
    } else {
        (None, false)
    };

    let mut object_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        object_id = parse_object_reference_argument(arg, "SetActionTargets", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetActionTargets: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetActionTargets requires an active engine context")
        })?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = object_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        if update_target1 {
            object.set_action_target(0, target1);
        }
        if update_target2 {
            object.set_action_target(1, target2);
        }

        Ok(Value::Bool(true))
    })
}

fn get_action(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetAction: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let action_name = object.effective_action_name();
                    let resolved = if action_name.is_empty() {
                        "Idle"
                    } else {
                        action_name
                    };
                    return Ok(Value::String(resolved.to_string()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                let resolved = if other.action_name.is_empty() {
                    "Idle"
                } else {
                    other.action_name.as_str()
                };
                return Ok(Value::String(resolved.to_string()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let action_name = object.effective_action_name();
        let resolved = if action_name.is_empty() {
            "Idle"
        } else {
            action_name
        };
        Ok(Value::String(resolved.to_string()))
    })
}

fn get_act_time(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActTime: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let clamp_ticks = |ticks: u32| -> Value {
            let clamped = ticks.min(i32::MAX as u32) as i32;
            Value::Int(clamped)
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(clamp_ticks(object.effective_action_ticks()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(clamp_ticks(other.action_ticks()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(clamp_ticks(object.effective_action_ticks()))
    })
}

fn get_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActionData: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(object.effective_action_data()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.action_data()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(object.effective_action_data()))
    })
}

fn get_procedure(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetProcedure: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let procedure_value = |name: Option<&str>| match name {
            Some(procedure) => Value::String(procedure.to_string()),
            None => Value::Nil,
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let procedure = object.effective_procedure_name();
                    return Ok(procedure_value(procedure));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(procedure_value(other.procedure_name()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let procedure = object.effective_procedure_name();
        Ok(procedure_value(procedure))
    })
}

fn get_action_target(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_index = 0;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                target_index = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    let mut object_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        object_id = parse_object_reference_argument(arg, "GetActionTarget", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActionTarget: additional arguments are not supported",
        ));
    }

    if target_index < 0 {
        return Ok(Value::Nil);
    }

    let slot = target_index as usize;
    if slot > 1 {
        return Ok(Value::Nil);
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = object_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let target_value = object.effective_action_target(slot);
                    return Ok(target_value.map_or(Value::Nil, object_reference_value));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                let target_value = other.action_target(slot);
                return Ok(target_value.map_or(Value::Nil, object_reference_value));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let target_value = object.effective_action_target(slot);
        Ok(target_value.map_or(Value::Nil, object_reference_value))
    })
}

fn get_vertex_num(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(_) | Value::Nil => {
                target_id = parse_object_reference_argument(arg, "GetVertexNum", "object")?;
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetVertexNum: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        match resolve_vertices(context, target_id) {
            Some((_position, vertices)) => Ok(Value::Int(truncate_to_i32(vertices.len() as u64))),
            None => Ok(Value::Nil),
        }
    })
}

fn get_vertex(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetVertex: requires at least an index argument",
        ));
    }

    let index_value = value_to_i32(&args[0], "GetVertex", "index")?;
    let mut arg_index = 1;
    let mut attribute = 1;

    if let Some(arg) = args.get(arg_index) {
        match arg {
            Value::Int(value) => {
                attribute = *value;
                arg_index += 1;
            }
            Value::Nil => {
                arg_index += 1;
            }
            _ => {}
        }
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(arg_index) {
        target_id = parse_object_reference_argument(arg, "GetVertex", "object")?;
        arg_index += 1;
    }

    if arg_index < args.len() {
        return Err(RuntimeError::new(
            "GetVertex: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let (_position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        if vertices.is_empty() {
            return Ok(Value::Nil);
        }

        let limit = vertices.len() as i32 - 1;
        let mut clamped = index_value;
        if clamped < 0 {
            clamped = 0;
        } else if clamped > limit {
            clamped = limit;
        }
        let vertex = &vertices[clamped as usize];
        let result = match attribute {
            0 => vertex.x,
            1 => vertex.y,
            2 => truncate_to_i32(vertex.cnat as u64),
            3 => vertex.friction,
            _ => vertex.y,
        };
        Ok(Value::Int(result))
    })
}

fn get_vertex_contact(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetVertexContact: requires a vertex index argument",
        ));
    }

    let vertex_index = value_to_i32(&args[0], "GetVertexContact", "index")?;
    let mut arg_index = 1;
    let mut mask: u32 = 0;

    if let Some(arg) = args.get(arg_index) {
        match arg {
            Value::Int(value) => {
                if *value > 0 {
                    mask = *value as u32;
                }
                arg_index += 1;
            }
            Value::Nil => {
                arg_index += 1;
            }
            _ => {}
        }
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(arg_index) {
        target_id = parse_object_reference_argument(arg, "GetVertexContact", "object")?;
        arg_index += 1;
    }

    if arg_index < args.len() {
        return Err(RuntimeError::new(
            "GetVertexContact: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let (position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        if vertex_index < 0 || (vertex_index as usize) >= vertices.len() {
            return Ok(Value::Nil);
        }

        let landscape = context.landscape_ref();
        let contact =
            compute_vertex_contact(landscape, position, &vertices[vertex_index as usize], mask);
        Ok(Value::Int(contact as i32))
    })
}

fn get_contact(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetContact: requires a vertex index argument",
        ));
    }

    let vertex_index = value_to_i32(&args[0], "GetContact", "index")?;
    let mut arg_index = 1;
    let mut mask: u32 = 0;

    if let Some(arg) = args.get(arg_index) {
        match arg {
            Value::Int(value) => {
                if *value > 0 {
                    mask = *value as u32;
                }
                arg_index += 1;
            }
            Value::Nil => {
                arg_index += 1;
            }
            _ => {}
        }
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(arg_index) {
        target_id = parse_object_reference_argument(arg, "GetContact", "object")?;
        arg_index += 1;
    }

    if arg_index < args.len() {
        return Err(RuntimeError::new(
            "GetContact: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let (position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        let landscape = context.landscape_ref();

        if vertex_index == -1 {
            if vertices.is_empty() {
                return Ok(Value::Int(0));
            }
            let mut result = 0u32;
            for vertex in vertices {
                result |= compute_vertex_contact(landscape, position, vertex, mask);
            }
            return Ok(Value::Int(result as i32));
        }

        if vertex_index < 0 || (vertex_index as usize) >= vertices.len() {
            return Ok(Value::Nil);
        }

        let contact =
            compute_vertex_contact(landscape, position, &vertices[vertex_index as usize], mask);
        Ok(Value::Int(contact as i32))
    })
}

fn path_free(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 4 {
        return Err(RuntimeError::new(
            "PathFree expects 4 arguments: x1, y1, x2, y2",
        ));
    }

    let x1 = value_to_i32(&args[0], "PathFree", "x1")?;
    let y1 = value_to_i32(&args[1], "PathFree", "y1")?;
    let x2 = value_to_i32(&args[2], "PathFree", "x2")?;
    let y2 = value_to_i32(&args[3], "PathFree", "y2")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Bool(true)),
        };

        let Some(landscape) = context.landscape_ref() else {
            return Ok(Value::Bool(true));
        };

        let clear = landscape.path_is_clear(Vector2::new(x1, y1), Vector2::new(x2, y2));
        Ok(Value::Bool(clear))
    })
}

#[derive(Clone, Copy)]
enum LandscapeQuery {
    Solid,
    SemiSolid,
    Liquid,
    Sky,
}

fn g_back_solid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSolid", LandscapeQuery::Solid)
}

fn g_back_semi_solid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSemiSolid", LandscapeQuery::SemiSolid)
}

fn g_back_liquid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackLiquid", LandscapeQuery::Liquid)
}

fn g_back_sky(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSky", LandscapeQuery::Sky)
}

fn g_back_common(
    args: &[Value],
    function: &str,
    query: LandscapeQuery,
) -> Result<Value, RuntimeError> {
    if args.len() != 2 {
        return Err(RuntimeError::new(format!(
            "{function} expects 2 arguments: x, y"
        )));
    }

    let local_x = value_to_i32(&args[0], function, "x")?;
    let local_y = value_to_i32(&args[1], function, "y")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Bool(fallback_without_context(query))),
        };

        let mut global_x = local_x;
        let mut global_y = local_y;
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            global_x = global_x.saturating_add(position.x);
            global_y = global_y.saturating_add(position.y);
        }

        let landscape = context.landscape_ref();
        let result = evaluate_landscape_query(landscape, query, global_x, global_y);
        Ok(Value::Bool(result))
    })
}

fn evaluate_landscape_query(
    landscape: Option<&Landscape>,
    query: LandscapeQuery,
    x: i32,
    y: i32,
) -> bool {
    match landscape {
        Some(landscape) => match query {
            LandscapeQuery::Solid => landscape.is_solid_at(x, y),
            LandscapeQuery::SemiSolid => landscape.is_solid_at(x, y),
            LandscapeQuery::Liquid => landscape.is_liquid_at(x, y),
            LandscapeQuery::Sky => !landscape.is_solid_at(x, y),
        },
        None => fallback_without_context(query),
    }
}

fn fallback_without_context(query: LandscapeQuery) -> bool {
    match query {
        LandscapeQuery::Sky => true,
        LandscapeQuery::Solid | LandscapeQuery::SemiSolid | LandscapeQuery::Liquid => false,
    }
}

fn find_object(args: &[Value]) -> Result<Value, RuntimeError> {
    let params = FindObjectParams::parse(args)?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let result = if params.is_closest_query() {
            find_object_closest(context, &params)
        } else {
            find_object_linear(context, &params)
        };
        Ok(match result {
            Some(id) => object_reference_value(id),
            None => Value::Nil,
        })
    })
}

fn find_object_linear(world: &impl WorldAccessor, params: &FindObjectParams) -> Option<ObjectId> {
    let mut skip_until = params.find_next;
    for object_id in world.object_ids() {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if let Some(target) = skip_until {
            if object_id == target {
                skip_until = None;
            }
            continue;
        }
        if !params.matches_object(&object) {
            continue;
        }
        if params.matches_area(&object) {
            return Some(object_id);
        }
    }
    None
}

fn find_object_closest(world: &impl WorldAccessor, params: &FindObjectParams) -> Option<ObjectId> {
    let reference = params.reference_distance(world);
    let mut best: Option<(ObjectId, i64)> = None;
    for object_id in world.object_ids() {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !params.matches_object(&object) {
            continue;
        }
        let distance = squared_distance(object.position(), params.x, params.y);
        if let Some(reference) = reference {
            if distance <= reference {
                continue;
            }
        }
        match best {
            None => best = Some((object_id, distance)),
            Some((_, best_distance)) if distance < best_distance => {
                best = Some((object_id, distance));
            }
            _ => {}
        }
    }
    best.map(|(id, _)| id)
}

fn find_objects(args: &[Value]) -> Result<Value, RuntimeError> {
    let params = FindObjectParams::parse(args)?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Array(Vec::new())),
        };
        let ids = if params.is_closest_query() {
            collect_closest_matches(context, &params)
        } else {
            collect_linear_matches(context, &params)
        };
        let values = ids
            .into_iter()
            .map(object_reference_value)
            .collect::<Vec<_>>();
        Ok(Value::Array(values))
    })
}

fn collect_linear_matches(world: &impl WorldAccessor, params: &FindObjectParams) -> Vec<ObjectId> {
    let mut matches = Vec::new();
    let mut skip_until = params.find_next;
    for object_id in world.object_ids() {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if let Some(target) = skip_until {
            if object_id == target {
                skip_until = None;
            }
            continue;
        }
        if params.matches_object(&object) && params.matches_area(&object) {
            matches.push(object_id);
        }
    }
    matches
}

fn collect_closest_matches(world: &impl WorldAccessor, params: &FindObjectParams) -> Vec<ObjectId> {
    let reference = params.reference_distance(world);
    let mut matches = Vec::new();
    for (order_index, object_id) in world.object_ids().into_iter().enumerate() {
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !params.matches_object(&object) {
            continue;
        }
        let distance = squared_distance(object.position(), params.x, params.y);
        if let Some(reference) = reference {
            if distance <= reference {
                continue;
            }
        }
        matches.push((distance, order_index, object_id));
    }
    matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    matches.into_iter().map(|(_, _, id)| id).collect()
}

fn set_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetDir expects at least 1 argument: direction",
        ));
    }

    let raw_direction = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetDir: expected int or nil for direction, got {}",
                other.type_name()
            )))
        }
    };

    let direction = match Direction::from_script_value(raw_direction) {
        Some(direction) => direction,
        None => return Ok(Value::Bool(false)),
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetDir requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_direction(direction);
        Ok(Value::Bool(true))
    })
}

fn get_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Nil);
            }
        }

        Ok(Value::Int(object.direction().to_script_value()))
    })
}

fn set_com_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetComDir expects at least 1 argument: command direction",
        ));
    }

    let raw_direction = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetComDir: expected int or nil for command direction, got {}",
                other.type_name()
            )))
        }
    };

    let command_direction = match CommandDirection::from_script_value(raw_direction) {
        Some(direction) => direction,
        None => return Ok(Value::Bool(false)),
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetComDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetComDir requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_command_direction(command_direction);
        Ok(Value::Bool(true))
    })
}

fn get_com_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetComDir: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };
        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Nil);
            }
        }

        Ok(Value::Int(object.command_direction().to_script_value()))
    })
}

enum PositionComponent {
    X,
    Y,
}

impl PositionComponent {
    fn function_name(&self) -> &'static str {
        match self {
            PositionComponent::X => "GetX",
            PositionComponent::Y => "GetY",
        }
    }

    fn extract(&self, position: Vector2) -> i32 {
        match self {
            PositionComponent::X => position.x,
            PositionComponent::Y => position.y,
        }
    }
}

fn get_position_component(
    args: &[Value],
    component: PositionComponent,
) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(format!(
            "{} expects at most 1 argument: target",
            component.function_name()
        )));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        target_id = parse_object_reference_argument(arg, component.function_name(), "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let position = object.effective_position();
                    return Ok(Value::Int(component.extract(position)));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                let position = other.position();
                return Ok(Value::Int(component.extract(position)));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        let position = object.effective_position();
        Ok(Value::Int(component.extract(position)))
    })
}

enum VelocityComponent {
    X,
    Y,
}

impl VelocityComponent {
    fn get_function_name(&self) -> &'static str {
        match self {
            VelocityComponent::X => "GetXDir",
            VelocityComponent::Y => "GetYDir",
        }
    }

    fn set_function_name(&self) -> &'static str {
        match self {
            VelocityComponent::X => "SetXDir",
            VelocityComponent::Y => "SetYDir",
        }
    }

    fn extract(&self, velocity: Vector2) -> i32 {
        match self {
            VelocityComponent::X => velocity.x,
            VelocityComponent::Y => velocity.y,
        }
    }

    fn assign(&self, velocity: &mut Vector2, value: i32) {
        match self {
            VelocityComponent::X => velocity.x = value,
            VelocityComponent::Y => velocity.y = value,
        }
    }
}

fn get_velocity_component(
    args: &[Value],
    component: VelocityComponent,
) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(format!(
            "{} expects at most 2 arguments: target, precision",
            component.get_function_name()
        )));
    }

    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(arg, Value::Proplist(_) | Value::Nil | Value::Int(0)) {
            target_id =
                parse_object_reference_argument(arg, component.get_function_name(), "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, component.get_function_name(), "precision")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(format!(
            "{}: additional arguments are not supported",
            component.get_function_name()
        )));
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let fetch_velocity = |object_velocity: Vector2| {
            let component_value = component.extract(object_velocity);
            let scaled =
                scale_velocity_value(component_value, DEFAULT_VELOCITY_PRECISION, precision);
            Value::Int(scaled)
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(fetch_velocity(object.velocity()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(fetch_velocity(other.velocity()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };
        Ok(fetch_velocity(object.velocity()))
    })
}

fn set_velocity_component(
    args: &[Value],
    component: VelocityComponent,
) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(format!(
            "{} expects at least 1 argument: value",
            component.set_function_name()
        )));
    }

    let value = value_to_i32(&args[0], component.set_function_name(), "value")?;
    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        if matches!(arg, Value::Proplist(_) | Value::Nil | Value::Int(0)) {
            target_id =
                parse_object_reference_argument(arg, component.set_function_name(), "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, component.set_function_name(), "precision")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(format!(
            "{}: additional arguments are not supported",
            component.set_function_name()
        )));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new(format!(
                "{} requires an active engine context",
                component.set_function_name()
            ))
        })?;

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        let mut updated_velocity = object.velocity();
        let scaled = scale_velocity_value(value, precision, DEFAULT_VELOCITY_PRECISION);
        component.assign(&mut updated_velocity, scaled);
        object.set_velocity(updated_velocity);
        Ok(Value::Bool(true))
    })
}

fn get_x_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    get_velocity_component(args, VelocityComponent::X)
}

fn get_y_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    get_velocity_component(args, VelocityComponent::Y)
}

fn set_x_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    set_velocity_component(args, VelocityComponent::X)
}

fn set_y_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    set_velocity_component(args, VelocityComponent::Y)
}

fn get_x(args: &[Value]) -> Result<Value, RuntimeError> {
    get_position_component(args, PositionComponent::X)
}

fn get_y(args: &[Value]) -> Result<Value, RuntimeError> {
    get_position_component(args, PositionComponent::Y)
}

fn apply_position_bounds(
    desired: Vector2,
    vertices: &[ObjectVertex],
    landscape: Option<&Landscape>,
) -> Vector2 {
    let mut bounded = desired;
    let Some(landscape) = landscape else {
        return bounded;
    };

    let width = landscape.width() as i32;
    if width > 0 {
        let (mut min_allowed, mut max_allowed) = if vertices.is_empty() {
            (0, width.saturating_sub(1))
        } else {
            vertices
                .iter()
                .fold((i32::MIN, i32::MAX), |(min_acc, max_acc), vertex| {
                    (
                        min_acc.max(-vertex.x),
                        max_acc.min(width.saturating_sub(1).saturating_sub(vertex.x)),
                    )
                })
        };

        if min_allowed == i32::MIN {
            min_allowed = 0;
        }
        if max_allowed == i32::MAX {
            max_allowed = width.saturating_sub(1);
        }

        if min_allowed <= max_allowed {
            bounded.x = bounded.x.clamp(min_allowed, max_allowed);
        } else {
            bounded.x = bounded.x.clamp(0, width.saturating_sub(1));
        }
    }

    let min_y_allowed = if vertices.is_empty() {
        0
    } else {
        vertices.iter().map(|vertex| -vertex.y).max().unwrap_or(0)
    };

    let mut max_y_allowed = i32::MAX;
    if vertices.is_empty() {
        if let Some(surface_y) = landscape.surface_height(bounded.x) {
            max_y_allowed = surface_y;
        }
    } else {
        for vertex in vertices {
            let world_x = bounded.x.saturating_add(vertex.x);
            if let Some(surface_y) = landscape.surface_height(world_x) {
                max_y_allowed = max_y_allowed.min(surface_y - vertex.y);
            }
        }
    }

    if max_y_allowed < min_y_allowed {
        max_y_allowed = min_y_allowed;
    }

    bounded.y = bounded.y.clamp(min_y_allowed, max_y_allowed);
    bounded
}

fn set_position(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "SetPosition expects at least 2 arguments: x, y",
        ));
    }

    let x = value_to_i32(&args[0], "SetPosition", "x")?;
    let y = value_to_i32(&args[1], "SetPosition", "y")?;

    let mut index = 2;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetPosition", "target")?;
        index += 1;
    }

    let mut check_bounds = false;
    if let Some(arg) = args.get(index) {
        check_bounds = match arg {
            Value::Bool(value) => *value,
            Value::Int(value) => *value != 0,
            Value::Nil => false,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetPosition: expected bool for check_bounds, got {}",
                    other.type_name()
                )))
            }
        };
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetPosition: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetPosition requires an active engine context"))?;

        let landscape_snapshot = if check_bounds {
            context.landscape_ref().cloned()
        } else {
            None
        };

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        let mut position = Vector2::new(x, y);
        if check_bounds {
            let vertices: Vec<ObjectVertex> = object.vertices().to_vec();
            position = apply_position_bounds(position, &vertices, landscape_snapshot.as_ref());
        }

        object.set_position(position);
        Ok(Value::Bool(true))
    })
}

fn create_object(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "CreateObject expects at least 1 argument: definition",
        ));
    }

    let definition = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateObject: expected string for definition, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;

    let x_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateObject", "x")?;
        index += 1;
        value
    } else {
        0
    };

    let y_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateObject", "y")?;
        index += 1;
        value
    } else {
        0
    };

    let mut owner_override: Option<i32> = None;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                owner_override = Some(*value);
                index += 1;
            }
            Value::Nil => {
                owner_override = Some(OWNER_NONE);
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "CreateObject: expected int or nil for owner, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "CreateObject: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("CreateObject requires an active engine context"))?;

        let definition_category = context
            .definition_category(&definition)
            .unwrap_or(DEFAULT_CATEGORY);

        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);
        let base_owner = context
            .object_context()
            .map(|object| object.owner())
            .unwrap_or(OWNER_NONE);

        let owner = owner_override.unwrap_or(base_owner);
        let position = Vector2::new(
            base_position.x.saturating_add(x_offset),
            base_position.y.saturating_add(y_offset),
        );

        let id = context.allocate_object_id();

        let spawn = SpawnConfig::new(definition.clone())
            .with_position(position)
            .with_owner(owner)
            .with_category(definition_category)
            .with_id(id);

        let preview = HostWorldObject::with_category(
            id,
            definition,
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            owner,
            definition_category,
            0,
            0,
            position,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );

        context.register_spawn(spawn, preview);
        Ok(object_reference_value(id))
    })
}

fn create_particle(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "CreateParticle expects at least 1 argument: name",
        ));
    }

    let definition = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateParticle: expected string for name, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;

    let x = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "x")?;
        index += 1;
        value
    } else {
        0
    };

    let y = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "y")?;
        index += 1;
        value
    } else {
        0
    };

    let x_dir = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "xdir")?;
        index += 1;
        value
    } else {
        0
    };

    let y_dir = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "ydir")?;
        index += 1;
        value
    } else {
        0
    };

    let parameter_a = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "a")?;
        index += 1;
        value
    } else {
        0
    };

    let life_raw = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateParticle", "b")?;
        index += 1;
        value
    } else {
        0
    };

    let mut target_object: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_object = parse_object_reference_argument(arg, "CreateParticle", "object")?;
        index += 1;
    }

    let mut back = false;
    if let Some(arg) = args.get(index) {
        back = value_to_bool(arg, "CreateParticle", "back")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "CreateParticle: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("CreateParticle requires an active engine context"))?;

        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);

        let world_x = base_position.x.saturating_add(x);
        let world_y = base_position.y.saturating_add(y);

        let layer = if let Some(target) = target_object {
            let world_object = match context.get_world_object(target) {
                Some(object) => object,
                None => return Ok(Value::Bool(false)),
            };
            if !world_object.status().is_active() {
                return Ok(Value::Bool(false));
            }
            if back {
                ParticleLayer::ObjectBack(target)
            } else {
                ParticleLayer::ObjectFront(target)
            }
        } else {
            ParticleLayer::Global
        };

        let config = ParticleConfig {
            definition_id: definition,
            position: FloatVector2::new(world_x as f32, world_y as f32),
            velocity: FloatVector2::new(x_dir as f32 / 10.0, y_dir as f32 / 10.0),
            life: life_raw.max(0),
            parameter_a: parameter_a as f32 / 10.0,
            parameter_b: life_raw,
            layer,
        };

        context.register_particle(ParticleCommand::Create(config));
        Ok(Value::Bool(true))
    })
}

fn clear_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut definition: Option<String> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::String(name) if !name.is_empty() => definition = Some(name.clone()),
            Value::String(_) | Value::Nil => definition = None,
            other => {
                return Err(RuntimeError::new(format!(
                    "ClearParticles: expected string or nil for name, got {}",
                    other.type_name()
                )))
            }
        }
        index += 1;
    }

    let mut target_object: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_object = parse_object_reference_argument(arg, "ClearParticles", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "ClearParticles: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(false)),
        };

        let scope = if let Some(target) = target_object {
            if context.get_world_object(target).is_none() {
                return Ok(Value::Bool(false));
            }
            ParticleScope::Object(target)
        } else {
            ParticleScope::Global
        };

        context.register_particle(ParticleCommand::Clear {
            definition_id: definition.clone(),
            scope,
        });
        Ok(Value::Bool(true))
    })
}

fn contained(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Contained expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        target_id = parse_object_reference_argument(arg, "Contained", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let to_value = |container: Option<ObjectId>| {
            container.map(object_reference_value).unwrap_or(Value::Nil)
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(to_value(object.container()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(to_value(other.container()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(to_value(object.container()))
    })
}

fn get_category(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetCategory expects at most 2 arguments: target, definition",
        ));
    }

    let target_value = args.get(0).unwrap_or(&Value::Nil);
    let target_id = parse_object_reference_argument(target_value, "GetCategory", "target")?;
    let definition = parse_definition_argument(args.get(1), "GetCategory")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(definition_id) = definition {
            if let Some(category) = context.definition_category(&definition_id) {
                return Ok(Value::Int(category));
            }
            return Ok(Value::Nil);
        }

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(Value::Int(object.category()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.category()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(object.category()))
    })
}

fn set_category(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetCategory expects at least 1 argument: category",
        ));
    }

    let category = value_to_i32(&args[0], "SetCategory", "category")?;

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetCategory", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetCategory: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetCategory requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_category(category);
        Ok(Value::Bool(true))
    })
}

fn set_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetOwner expects at least 1 argument: owner",
        ));
    }

    let owner = match &args[0] {
        Value::Int(value) => *value,
        other => {
            return Err(RuntimeError::new(format!(
                "SetOwner: expected int for owner, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetOwner", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetOwner: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetOwner requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_owner(owner);
        Ok(Value::Bool(true))
    })
}

fn set_alive(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetAlive expects at least 1 argument: alive",
        ));
    }

    let alive = match &args[0] {
        Value::Bool(flag) => *flag,
        Value::Int(value) => *value != 0,
        Value::Nil => false,
        other => {
            return Err(RuntimeError::new(format!(
                "SetAlive: expected bool, int, or nil for alive, got {}",
                other.type_name()
            )))
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetAlive", "target")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetAlive: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetAlive requires an active engine context"))?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_alive(alive);
        Ok(Value::Bool(true))
    })
}

fn get_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetOwner expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        target_id = parse_object_reference_argument(arg, "GetOwner", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Int(OWNER_NONE)),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(object.owner()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.owner()));
            }
            return Ok(Value::Int(OWNER_NONE));
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Int(OWNER_NONE)),
        };

        Ok(Value::Int(object.owner()))
    })
}

fn get_alive(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetAlive expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        target_id = parse_object_reference_argument(arg, "GetAlive", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Bool(object.alive()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Bool(other.alive()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Bool(object.alive()))
    })
}

fn remove_object(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "RemoveObject expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        target_id = parse_object_reference_argument(arg, "RemoveObject", "target")?;
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = match borrow.as_mut() {
            Some(context) => context,
            None => return Ok(Value::Bool(false)),
        };

        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.mark_destroy();
        Ok(Value::Bool(true))
    })
}

fn with_context_mut<R>(
    scope: EffectScope,
    func: impl FnOnce(&mut EffectScopeContext) -> R,
) -> Result<R, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("effect host functions require an active engine context")
        })?;
        let stack = context.scope_mut(scope)?;
        Ok(func(stack))
    })
}

fn set_object_status(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetObjectStatus expects at least 1 argument: status",
        ));
    }

    let status_value = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetObjectStatus: expected int or nil for status, got {}",
                other.type_name()
            )))
        }
    };

    let status = match ObjectStatus::from_script_value(status_value) {
        Some(status) => status,
        None => return Ok(Value::Bool(false)),
    };

    if matches!(status, ObjectStatus::Deleted) {
        return Ok(Value::Bool(false));
    }

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            _ => {}
        }
    }

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Bool(_) | Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "SetObjectStatus: expected bool or nil for clear pointers, got {}",
                    other.type_name()
                )))
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetObjectStatus: additional arguments are not supported",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetObjectStatus requires an active engine context")
        })?;
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Bool(false));
            }
        }

        object.set_status(status);
        Ok(Value::Bool(true))
    })
}

fn get_object_status(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetObjectStatus expects at most 1 argument",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(0) {
        match arg {
            Value::Proplist(map) => {
                if let Some(Value::Int(id)) = map.get("id") {
                    if *id >= 0 {
                        target_id = Some(ObjectId::new(*id as u64));
                    }
                }
            }
            Value::Nil => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "GetObjectStatus: expected proplist or nil for target, got {}",
                    other.type_name()
                )))
            }
        }
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = match borrow.as_ref() {
            Some(context) => context,
            None => return Ok(Value::Nil),
        };

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        if let Some(target) = target_id {
            if target != object.id() {
                return Ok(Value::Nil);
            }
        }

        Ok(Value::Int(object.status().to_script_value()))
    })
}

fn snapshot_effects_from_context(scope: EffectScope) -> Option<Vec<EffectState>> {
    HOST_CONTEXT.with(|cell| cell.borrow().as_ref().and_then(|ctx| ctx.snapshot(scope)))
}

fn determine_scope_from_state(value: &Value) -> Result<EffectScope, RuntimeError> {
    match value {
        Value::Proplist(_) => Ok(EffectScope::Object),
        Value::Nil => Ok(EffectScope::Global),
        Value::Int(id) if *id == 0 => Ok(EffectScope::Global),
        other => Err(RuntimeError::new(format!(
            "effect host functions expected proplist, nil, or 0 for state, got {}",
            other.type_name()
        ))),
    }
}

fn extract_effects_from_state(state: &Value) -> Result<Vec<EffectState>, RuntimeError> {
    let map = match state {
        Value::Proplist(map) => map,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected proplist or nil for state, got {}",
                other.type_name()
            )))
        }
    };

    let effects_value = map.get("effects").unwrap_or(&Value::Nil);
    match effects_value {
        Value::Nil => Ok(Vec::new()),
        Value::Array(entries) => {
            let mut effects = Vec::new();
            for entry in entries {
                let props = match entry {
                    Value::Proplist(props) => props,
                    _ => continue,
                };

                let name = match props.get("name") {
                    Some(Value::String(name)) if !name.is_empty() => name.clone(),
                    _ => continue,
                };

                let priority = match props.get("priority") {
                    Some(Value::Int(value)) => *value,
                    _ => 0,
                };

                let interval = match props.get("interval") {
                    Some(Value::Int(value)) if *value > 0 => *value,
                    _ => 1,
                };

                let timer = match props.get("timer") {
                    Some(Value::Int(value)) if *value >= 0 => *value,
                    _ => 0,
                };

                let command_target = match props.get("command_target") {
                    Some(Value::Int(value)) => Some(*value),
                    _ => None,
                };
                let command_id = match props.get("command_target_id") {
                    Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
                    _ => None,
                };

                let vars = match props.get("vars") {
                    Some(Value::Array(entries)) => {
                        entries.iter().map(value_to_effect_var).collect()
                    }
                    _ => Vec::new(),
                };

                let mut effect = EffectState::new(name)
                    .with_priority(priority)
                    .with_interval(interval)
                    .with_timer(timer)
                    .with_command_target(command_target)
                    .with_command_id(command_id);
                if !vars.is_empty() {
                    effect = effect.with_vars(vars);
                }
                effects.push(effect);
            }
            Ok(effects)
        }
        other => Err(RuntimeError::new(format!(
            "GetEffect: state.effects must be an array, got {}",
            other.type_name()
        ))),
    }
}

fn build_effect_value(effect: &EffectState) -> Value {
    let mut map = HashMap::with_capacity(5);
    map.insert("name".into(), Value::String(effect.name.clone()));
    map.insert("priority".into(), Value::Int(effect.priority));
    map.insert("interval".into(), Value::Int(effect.interval));
    map.insert("timer".into(), Value::Int(effect.timer));
    if let Some(target) = effect.command_target {
        map.insert("command_target".into(), Value::Int(target));
    }
    if let Some(id) = &effect.command_id {
        map.insert("command_target_id".into(), Value::String(id.clone()));
    }
    if !effect.vars().is_empty() {
        let vars = effect
            .vars()
            .iter()
            .map(|value| effect_var_to_value(value))
            .collect();
        map.insert("vars".into(), Value::Array(vars));
    }
    Value::Proplist(map)
}

fn value_to_effect_var(value: &Value) -> EffectVarValue {
    match value {
        Value::Int(value) => EffectVarValue::Int(*value),
        Value::Bool(value) => EffectVarValue::Bool(*value),
        Value::String(value) => EffectVarValue::String(value.clone()),
        Value::Array(entries) => {
            let vars = entries
                .iter()
                .map(|entry| value_to_effect_var(entry))
                .collect();
            EffectVarValue::Array(vars)
        }
        Value::Proplist(map) => {
            let mut entries = BTreeMap::new();
            for (key, value) in map {
                entries.insert(key.clone(), value_to_effect_var(value));
            }
            EffectVarValue::Proplist(entries)
        }
        Value::Nil => EffectVarValue::Nil,
    }
}

fn effect_var_to_value(value: &EffectVarValue) -> Value {
    match value {
        EffectVarValue::Int(value) => Value::Int(*value),
        EffectVarValue::Bool(value) => Value::Bool(*value),
        EffectVarValue::String(value) => Value::String(value.clone()),
        EffectVarValue::Array(entries) => {
            let vars = entries
                .iter()
                .map(|entry| effect_var_to_value(entry))
                .collect();
            Value::Array(vars)
        }
        EffectVarValue::Proplist(map) => {
            let mut entries = HashMap::with_capacity(map.len());
            for (key, value) in map {
                entries.insert(key.clone(), effect_var_to_value(value));
            }
            Value::Proplist(entries)
        }
        EffectVarValue::Nil => Value::Nil,
    }
}

fn parse_command_target(value: &Value) -> Result<Option<i32>, RuntimeError> {
    match value {
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) => Ok(Some(*id)),
            _ => Err(RuntimeError::new(
                "AddEffect: command target proplist must contain int `id`",
            )),
        },
        Value::Nil => Ok(None),
        Value::Int(value) if *value == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "AddEffect: expected proplist, nil, or 0 for command target, got {}",
            other.type_name()
        ))),
    }
}

fn parse_command_target_id(value: &Value) -> Result<Option<String>, RuntimeError> {
    match value {
        Value::String(id) if !id.is_empty() => Ok(Some(id.clone())),
        Value::String(_) | Value::Nil => Ok(None),
        Value::Int(value) if *value == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "AddEffect: expected string or nil for command target id, got {}",
            other.type_name()
        ))),
    }
}

fn parse_timer_from_int(value: i32) -> Result<i32, RuntimeError> {
    if value < 0 {
        Err(RuntimeError::new(
            "AddEffect: timer must be >= 0 when provided",
        ))
    } else {
        Ok(value)
    }
}

struct EffectHostContext {
    object: Option<ObjectScopeContext>,
    global: Option<EffectScopeContext>,
    world: HostWorldContext,
    pending_spawns: Vec<SpawnConfig>,
    pending_objects: HashMap<ObjectId, HostWorldObject>,
    pending_order: Vec<ObjectId>,
    pending_particles: Vec<ParticleCommand>,
    next_object_id: u64,
}

impl EffectHostContext {
    fn new(
        object: Option<HostObjectContext<'_>>,
        global_effects: Vec<EffectState>,
        world: HostWorldContext,
        next_object_id: u64,
    ) -> Self {
        let object = object.map(|ctx| {
            let HostObjectContext {
                id,
                container,
                status,
                energy,
                damage,
                alive,
                owner,
                position,
                velocity,
                effects,
                action_name,
                action_ticks,
                action_data,
                action_library,
                direction,
                command_direction,
                action_target,
                action_target2,
                vertices,
                category,
            } = ctx;
            ObjectScopeContext::new(
                id,
                container,
                status,
                energy,
                damage,
                alive,
                owner,
                category,
                position,
                velocity,
                effects.to_vec(),
                action_library,
                action_name,
                action_ticks,
                action_data,
                direction,
                command_direction,
                action_target,
                action_target2,
                vertices.to_vec(),
            )
        });
        let global = Some(EffectScopeContext::new(global_effects));
        Self {
            object,
            global,
            world,
            pending_spawns: Vec::new(),
            pending_objects: HashMap::new(),
            pending_order: Vec::new(),
            pending_particles: Vec::new(),
            next_object_id,
        }
    }

    fn scope_mut(&mut self, scope: EffectScope) -> Result<&mut EffectScopeContext, RuntimeError> {
        match scope {
            EffectScope::Object => {
                self.object
                    .as_mut()
                    .map(|ctx| &mut ctx.effects)
                    .ok_or_else(|| {
                        RuntimeError::new(
                            "object effect operations require an active engine context",
                        )
                    })
            }
            EffectScope::Global => self.global.as_mut().ok_or_else(|| {
                RuntimeError::new("global effect operations require an active engine context")
            }),
        }
    }

    fn allocate_object_id(&mut self) -> ObjectId {
        let id = ObjectId::new(self.next_object_id);
        self.next_object_id += 1;
        id
    }

    fn register_spawn(&mut self, spawn: SpawnConfig, preview: HostWorldObject) {
        let id = preview.id;
        if !self.pending_objects.contains_key(&id) {
            self.pending_order.push(id);
        }
        self.pending_objects.insert(id, preview);
        self.pending_spawns.push(spawn);
    }

    fn register_particle(&mut self, command: ParticleCommand) {
        self.pending_particles.push(command);
    }

    fn get_world_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        if let Some(object) = self.pending_objects.get(&id) {
            Some(object.clone())
        } else {
            self.world.get(id).cloned()
        }
    }

    fn world_object_ids(&self) -> Vec<ObjectId> {
        let mut ids = self.world.object_ids().to_vec();
        ids.extend(self.pending_order.iter().copied());
        ids
    }

    fn definition_category(&self, id: &str) -> Option<i32> {
        self.world.definition_category(id)
    }

    fn landscape_ref(&self) -> Option<&Landscape> {
        self.world.landscape_ref()
    }

    fn snapshot(&self, scope: EffectScope) -> Option<Vec<EffectState>> {
        match scope {
            EffectScope::Object => self.object.as_ref().map(|ctx| ctx.effects.snapshot()),
            EffectScope::Global => self.global.as_ref().map(EffectScopeContext::snapshot),
        }
    }

    fn object_context_mut(&mut self) -> Option<&mut ObjectScopeContext> {
        self.object.as_mut()
    }

    fn object_context(&self) -> Option<&ObjectScopeContext> {
        self.object.as_ref()
    }

    fn into_commands(self) -> EffectContextOutcome {
        let (object_effects, object_update, object_commands, destroy) = match self.object {
            Some(object) => {
                let update = if object.pending_update.is_empty() {
                    None
                } else {
                    Some(object.pending_update)
                };
                (
                    object.effects.into_commands(),
                    update,
                    object.queued_commands,
                    object.destroy,
                )
            }
            None => (Vec::new(), None, Vec::new(), false),
        };

        let global = self
            .global
            .map(EffectScopeContext::into_commands)
            .unwrap_or_default();

        let mut outcome = EffectContextOutcome::new(
            object_effects,
            global,
            object_update,
            object_commands,
            destroy,
            None,
            None,
            self.pending_spawns,
            self.next_object_id,
        );
        outcome.particles = self.pending_particles;
        outcome
    }
}

struct EffectScopeContext {
    effects: Vec<EffectState>,
    commands: Vec<EffectCommand>,
}

impl EffectScopeContext {
    fn new(effects: Vec<EffectState>) -> Self {
        Self {
            effects,
            commands: Vec::new(),
        }
    }

    fn snapshot(&self) -> Vec<EffectState> {
        self.effects.clone()
    }

    fn add_effect(&mut self, mut effect: EffectState) -> i32 {
        if effect.interval <= 0 {
            effect.interval = 1;
        }
        if effect.timer < 0 {
            effect.timer = 0;
        }
        if effect.interval > 0 && effect.timer >= effect.interval {
            effect.timer %= effect.interval;
        }

        if let Some(index) = self
            .effects
            .iter()
            .position(|existing| existing.name == effect.name)
        {
            self.effects.remove(index);
        }

        let mut insert_pos = 0;
        while insert_pos < self.effects.len() && self.effects[insert_pos].priority > effect.priority
        {
            insert_pos += 1;
        }

        self.effects.insert(insert_pos, effect.clone());
        self.commands.push(EffectCommand::add(effect));
        (insert_pos + 1) as i32
    }

    fn effect_var(
        &mut self,
        effect_number: usize,
        var_index: usize,
        new_value: Option<EffectVarValue>,
    ) -> Option<EffectVarValue> {
        if effect_number == 0 {
            return None;
        }
        let index = effect_number - 1;
        if index >= self.effects.len() {
            return None;
        }
        let effect = &mut self.effects[index];
        if let Some(value) = new_value {
            effect.set_var(var_index, value);
            let updated = effect.clone();
            self.commands.push(EffectCommand::add(updated));
        }
        Some(effect.var(var_index))
    }

    fn remove_effect(
        &mut self,
        name_filter: Option<&str>,
        index: usize,
        no_callbacks: bool,
    ) -> bool {
        let position = if let Some(name) = name_filter {
            let mut remaining = index;
            self.effects.iter().position(|effect| {
                if effect.name == name {
                    if remaining == 0 {
                        true
                    } else {
                        remaining -= 1;
                        false
                    }
                } else {
                    false
                }
            })
        } else {
            if self.effects.is_empty() {
                None
            } else if index == 0 {
                Some(0)
            } else {
                let effect_number = index.saturating_sub(1);
                if effect_number < self.effects.len() {
                    Some(effect_number)
                } else {
                    None
                }
            }
        };

        let position = match position {
            Some(pos) => pos,
            None => return false,
        };

        let effect = self.effects.remove(position);
        let command = if no_callbacks {
            EffectCommand::remove_without_callbacks(effect.name)
        } else {
            EffectCommand::remove(effect.name)
        };
        self.commands.push(command);
        true
    }

    fn into_commands(self) -> Vec<EffectCommand> {
        self.commands
    }
}

struct ObjectScopeContext {
    id: ObjectId,
    current_container: Option<ObjectId>,
    status: ObjectStatus,
    effects: EffectScopeContext,
    pending_update: ObjectUpdate,
    queued_commands: Vec<QueuedCommand>,
    destroy: bool,
    action_library: ActionLibrary,
    current_action_name: String,
    current_action_blocks_other_actions: bool,
    current_action_target: Option<ObjectId>,
    current_action_target2: Option<ObjectId>,
    current_action_data: i32,
    current_action_ticks: u32,
    current_energy: i32,
    current_damage: i32,
    current_alive: bool,
    max_energy: i32,
    current_owner: i32,
    current_category: i32,
    current_direction: Direction,
    current_command_direction: CommandDirection,
    current_position: Vector2,
    current_velocity: Vector2,
    vertices: Vec<ObjectVertex>,
}

impl ObjectScopeContext {
    fn new(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        damage: i32,
        alive: bool,
        owner: i32,
        category: i32,
        position: Vector2,
        velocity: Vector2,
        effects: Vec<EffectState>,
        action_library: ActionLibrary,
        action_name: String,
        action_ticks: u32,
        action_data: i32,
        direction: Direction,
        command_direction: CommandDirection,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: Vec<ObjectVertex>,
    ) -> Self {
        let blocks_other_actions = action_library.blocks_other_actions(&action_name);
        let max_energy = energy.max(DEFAULT_MAX_ENERGY);
        let clamped_damage = damage.max(0);
        Self {
            id,
            current_container: container,
            status,
            effects: EffectScopeContext::new(effects),
            pending_update: ObjectUpdate::default(),
            queued_commands: Vec::new(),
            destroy: false,
            action_library,
            current_action_name: action_name,
            current_action_blocks_other_actions: blocks_other_actions,
            current_action_target: action_target,
            current_action_target2: action_target2,
            current_action_data: action_data,
            current_action_ticks: action_ticks,
            current_energy: energy,
            current_damage: clamped_damage,
            current_alive: alive,
            max_energy,
            current_owner: owner,
            current_category: category,
            current_direction: direction,
            current_command_direction: command_direction,
            current_position: position,
            current_velocity: velocity,
            vertices,
        }
    }

    fn id(&self) -> ObjectId {
        self.id
    }

    fn status(&self) -> ObjectStatus {
        self.pending_update.status.unwrap_or(self.status)
    }

    fn set_status(&mut self, status: ObjectStatus) {
        self.status = status;
        self.pending_update.status = Some(status);
    }

    fn owner(&self) -> i32 {
        self.pending_update.owner.unwrap_or(self.current_owner)
    }

    fn set_owner(&mut self, owner: i32) {
        self.current_owner = owner;
        self.pending_update.owner = Some(owner);
    }

    fn alive(&self) -> bool {
        self.pending_update.alive.unwrap_or(self.current_alive)
    }

    fn set_alive(&mut self, alive: bool) {
        self.current_alive = alive;
        self.pending_update.alive = Some(alive);
    }

    fn category(&self) -> i32 {
        self.pending_update
            .category
            .unwrap_or(self.current_category)
    }

    fn set_category(&mut self, category: i32) {
        let normalized = crate::normalize_category(category, self.current_category);
        self.current_category = normalized;
        self.pending_update.category = Some(normalized);
    }

    fn container(&self) -> Option<ObjectId> {
        match self.pending_update.container {
            Some(container) => container,
            None => self.current_container,
        }
    }

    #[allow(dead_code)]
    fn set_container(&mut self, container: Option<ObjectId>) {
        if self.container() == container {
            return;
        }
        self.current_container = container;
        self.pending_update.container = Some(container);
    }

    fn mark_destroy(&mut self) {
        self.destroy = true;
    }

    fn update_effective_action(&mut self, action: &str) -> bool {
        let previous_name = self.current_action_name.clone();
        let previous_procedure = self.action_library.procedure_for_action(&previous_name);
        self.current_action_name = action.to_string();
        self.current_action_blocks_other_actions = self.action_library.blocks_other_actions(action);
        let next_procedure = self.action_library.procedure_for_action(action);
        previous_name != action && previous_procedure != next_procedure
    }

    fn effective_action_name(&self) -> &str {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(name) = update.name.as_ref() {
                return name;
            }
        }
        &self.current_action_name
    }

    fn effective_procedure_name(&self) -> Option<&str> {
        let action = self.effective_action_name();
        self.action_library.procedure_name_for_action(action)
    }

    fn effective_blocks_other_actions(&self) -> bool {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(name) = update.name.as_ref() {
                return self.action_library.blocks_other_actions(name);
            }
        }
        self.current_action_blocks_other_actions
    }

    fn effective_action_target(&self, index: usize) -> Option<ObjectId> {
        if let Some(update) = self.pending_update.action.as_ref() {
            match index {
                0 => {
                    if let Some(target) = update.target {
                        return target;
                    }
                }
                1 => {
                    if let Some(target) = update.target2 {
                        return target;
                    }
                }
                _ => return None,
            }
        }

        match index {
            0 => self.current_action_target,
            1 => self.current_action_target2,
            _ => None,
        }
    }

    fn effective_action_ticks(&self) -> u32 {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(ticks) = update.ticks {
                return ticks;
            }
        }
        self.current_action_ticks
    }

    fn effective_action_procedure(&self) -> ActionProcedure {
        let action = self.effective_action_name();
        self.action_library.procedure_for_action(action)
    }

    #[allow(dead_code)]
    fn effective_action_data(&self) -> i32 {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(data) = update.data {
                return data;
            }
        }
        self.current_action_data
    }

    fn set_action_data(&mut self, data: i32) {
        if self.current_action_data == data {
            if let Some(existing) = self
                .pending_update
                .action
                .as_ref()
                .and_then(|update| update.data)
            {
                if existing == data {
                    return;
                }
            } else {
                return;
            }
        }
        self.current_action_data = data;
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_data(data);
    }

    fn reset_action_data(&mut self) {
        self.set_action_data(0);
    }

    fn set_action_ticks(&mut self, ticks: u32) {
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_ticks(ticks);
        self.current_action_ticks = ticks;
    }

    fn reset_action_ticks(&mut self) {
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_ticks(0);
        self.current_action_ticks = 0;
    }

    fn energy(&self) -> i32 {
        self.pending_update.energy.unwrap_or(self.current_energy)
    }

    fn set_energy(&mut self, energy: i32) {
        self.current_energy = energy;
        if energy > self.max_energy {
            self.max_energy = energy;
        }
        self.pending_update.energy = Some(energy);
    }

    fn damage(&self) -> i32 {
        self.pending_update.damage.unwrap_or(self.current_damage)
    }

    fn set_damage(&mut self, damage: i32) {
        let clamped = damage.max(0);
        self.current_damage = clamped;
        self.pending_update.damage = Some(clamped);
    }

    fn adjust_damage(&mut self, delta: i32) -> i32 {
        let current = self.damage();
        let mut next = current.saturating_add(delta);
        if next < 0 {
            next = 0;
        }
        self.set_damage(next);
        next
    }

    fn direction(&self) -> Direction {
        self.pending_update
            .direction
            .unwrap_or(self.current_direction)
    }

    fn set_direction(&mut self, direction: Direction) {
        if self.direction() == direction {
            return;
        }
        self.current_direction = direction;
        self.pending_update.direction = Some(direction);
    }

    fn command_direction(&self) -> CommandDirection {
        self.pending_update
            .command_direction
            .unwrap_or(self.current_command_direction)
    }

    fn set_command_direction(&mut self, command_direction: CommandDirection) {
        if self.command_direction() == command_direction {
            return;
        }
        self.current_command_direction = command_direction;
        self.pending_update.command_direction = Some(command_direction);
    }

    fn velocity(&self) -> Vector2 {
        self.pending_update
            .velocity
            .unwrap_or(self.current_velocity)
    }

    fn set_velocity(&mut self, velocity: Vector2) {
        if self.velocity() == velocity && self.pending_update.velocity.is_none() {
            return;
        }
        self.current_velocity = velocity;
        self.pending_update.velocity = Some(velocity);
    }

    fn effective_position(&self) -> Vector2 {
        self.pending_update
            .position
            .unwrap_or(self.current_position)
    }

    fn set_position(&mut self, position: Vector2) {
        if self.effective_position() == position && self.pending_update.position.is_none() {
            return;
        }
        self.current_position = position;
        self.pending_update.position = Some(position);
    }

    fn vertices(&self) -> &[ObjectVertex] {
        if let Some(vertices) = self.pending_update.vertices.as_ref() {
            vertices
        } else {
            &self.vertices
        }
    }

    fn set_action_target(&mut self, index: usize, target: Option<ObjectId>) {
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        match index {
            0 => {
                update.set_target(target);
                self.current_action_target = target;
            }
            1 => {
                update.set_target2(target);
                self.current_action_target2 = target;
            }
            _ => {}
        }
    }

    fn adjust_energy(&mut self, delta: i32, _exact: bool) -> i32 {
        let mut next = self.energy().saturating_add(delta);
        if next < 0 {
            next = 0;
        }
        if next > self.max_energy {
            next = self.max_energy;
        }
        self.set_energy(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActionSpec;
    use proptest::prelude::*;
    use rand::{Rng, SeedableRng};
    use std::collections::HashMap;

    fn empty_state() -> Value {
        let mut map = HashMap::new();
        map.insert("effects".into(), Value::Array(Vec::new()));
        Value::Proplist(map)
    }

    fn with_object_host_context<F, T>(func: F) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_object_host_context_with_world(HostWorldContext::default(), func)
    }

    fn with_object_host_context_with_world<F, T>(
        world: HostWorldContext,
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            world,
            1,
            func,
        )
    }

    fn with_environment_context<F, T>(
        settings: EnvironmentSettings,
        frame: u64,
        func: F,
    ) -> (Result<T, RuntimeError>, EnvironmentDelta)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        let guard = enter_environment_context(settings, frame);
        let result = func();
        let delta = guard.finish();
        (result, delta)
    }

    fn with_physics_context<F, T>(
        settings: PhysicsSettings,
        func: F,
    ) -> (Result<T, RuntimeError>, PhysicsDelta)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        let guard = enter_physics_context(settings);
        let result = func();
        let delta = guard.finish();
        (result, delta)
    }

    #[test]
    fn g_back_solid_returns_false_without_landscape() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            g_back_solid(&[Value::Int(0), Value::Int(0)])
        });
        let value = result.expect("GBackSolid without landscape succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_solid_detects_surface_in_landscape() {
        let landscape = Landscape::flat(32, 10);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            1,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_solid(&[Value::Int(5), Value::Int(12)])
        });
        let value = result.expect("GBackSolid with landscape succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn g_back_solid_respects_surface_height() {
        let landscape = Landscape::flat(16, 20);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            1,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_solid(&[Value::Int(3), Value::Int(15)])
        });
        let value = result.expect("GBackSolid above surface succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_solid_applies_object_relative_coordinates() {
        let object_id = ObjectId::new(7);
        let landscape = Landscape::flat(32, 12);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            8,
        );
        let object_context = HostObjectContext::new(
            object_id,
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::new(4, 6),
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            None,
            None,
            &[],
        );
        let (result, _) = with_effect_context(Some(object_context), &[], world, 9, || {
            g_back_solid(&[Value::Int(0), Value::Int(7)])
        });
        let value = result.expect("GBackSolid with object context succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn g_back_sky_reports_inverse_of_solid() {
        let landscape = Landscape::flat(20, 5);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            1,
        );
        let (solid, _) = with_effect_context(None, &[], world.clone(), 1, || {
            g_back_solid(&[Value::Int(2), Value::Int(2)])
        });
        let (sky, _) = with_effect_context(None, &[], world, 1, || {
            g_back_sky(&[Value::Int(2), Value::Int(2)])
        });
        let solid_value = solid.expect("GBackSolid succeeds");
        let sky_value = sky.expect("GBackSky succeeds");
        match (solid_value, sky_value) {
            (Value::Bool(solid), Value::Bool(sky)) => assert_eq!(sky, !solid),
            other => panic!("expected bool results, got {:?}", other),
        }
    }

    #[test]
    fn g_back_liquid_returns_false_in_height_landscape() {
        let landscape = Landscape::flat(8, 4);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            1,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_liquid(&[Value::Int(1), Value::Int(6)])
        });
        let value = result.expect("GBackLiquid succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_liquid_detects_liquid_column() {
        let mut landscape = Landscape::flat(8, 4);
        landscape.set_liquid_column(1, vec![LiquidSegment::new(5, 9)]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            1,
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_liquid(&[Value::Int(1), Value::Int(6)])
        });
        let value = result.expect("GBackLiquid succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn add_effect_registers_command_and_updates_view() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(150),
                Value::Int(3),
            ])
        });
        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.name, "Glow");
                assert_eq!(effect.priority, 150);
                assert_eq!(effect.interval, 3);
                assert_eq!(effect.command_target, None);
                assert!(effect.command_id.is_none());
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn add_effect_records_command_target_metadata() {
        let state = empty_state();
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(42));
        let target = Value::Proplist(target_map);

        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(120),
                Value::Int(2),
                target.clone(),
                Value::String("FOOB".into()),
            ])
        });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.command_target, Some(42));
                assert_eq!(effect.command_id.as_deref(), Some("FOOB"));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn set_gravity_records_physics_update() {
        let (result, delta) =
            with_physics_context(PhysicsSettings::default(), || set_gravity(&[Value::Int(5)]));
        let value = result.expect("SetGravity succeeds");
        assert_eq!(value, Value::Nil);
        assert_eq!(delta.gravity, Some(5));
    }

    #[test]
    fn set_gravity_clamps_bounds() {
        let (_, delta) = with_physics_context(PhysicsSettings::default(), || {
            set_gravity(&[Value::Int(400)])
        });
        assert_eq!(delta.gravity, Some(300));
        let (_, delta) = with_physics_context(PhysicsSettings::default(), || {
            set_gravity(&[Value::Int(-500)])
        });
        assert_eq!(delta.gravity, Some(-300));
    }

    #[test]
    fn get_gravity_returns_current_value() {
        let settings = PhysicsSettings::new(6, 20, -30);
        let (result, _) = with_physics_context(settings, || get_gravity(&[]));
        let value = result.expect("GetGravity succeeds");
        assert_eq!(value, Value::Int(6));
    }

    #[test]
    fn set_wind_records_environment_update() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_wind(&[Value::Int(75)])?;
            get_wind(&[])
        });

        let value = result.expect("SetWind/GetWind succeeds");
        assert_eq!(value, Value::Int(75));
        assert_eq!(delta.wind, Some(75));
    }

    #[test]
    fn set_wind_clamps_to_bounds() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_wind(&[Value::Int(150)])?;
            get_wind(&[])
        });

        let value = result.expect("SetWind/GetWind succeeds");
        assert_eq!(value, Value::Int(100));
        assert_eq!(delta.wind, Some(100));
    }

    #[test]
    fn set_temperature_updates_context() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 42, || {
            set_temperature(&[Value::Int(-30)])?;
            get_temperature(&[])
        });

        let value = result.expect("SetTemperature/GetTemperature succeeds");
        assert_eq!(value, Value::Int(-30));
        assert_eq!(delta.temperature, Some(-30));
    }

    #[test]
    fn set_climate_clamps_and_updates() {
        let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
            set_climate(&[Value::Int(-80)])?;
            get_climate(&[])
        });

        let value = result.expect("SetClimate/GetClimate succeeds");
        assert_eq!(value, Value::Int(-50));
        assert_eq!(delta.climate, Some(-50));
    }

    #[test]
    fn random_requires_context_for_positive_ranges() {
        let error = random(&[Value::Int(5)]).expect_err("Random without context fails");
        assert_eq!(error.message(), "Random: host context unavailable");
    }

    #[test]
    fn random_zero_or_negative_range_short_circuits() {
        let zero = random(&[Value::Int(0)]).expect("zero range succeeds");
        let negative = random(&[Value::Int(-3)]).expect("negative range succeeds");
        assert_eq!(zero, Value::Int(0));
        assert_eq!(negative, Value::Int(0));
    }

    proptest! {
        #[test]
        fn set_wind_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_wind(&[Value::Int(raw)])?;
                get_wind(&[])
            });

            let expected = raw.clamp(-100, 100);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.wind, Some(expected));
            prop_assert!(delta.temperature.is_none());
            prop_assert!(delta.climate.is_none());
        }

        #[test]
        fn set_temperature_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_temperature(&[Value::Int(raw)])?;
                get_temperature(&[])
            });

            let expected = raw.clamp(-100, 100);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.temperature, Some(expected));
            prop_assert!(delta.wind.is_none());
            prop_assert!(delta.climate.is_none());
        }

        #[test]
        fn set_climate_clamps_across_range(raw in any::<i32>()) {
            let (result, delta) = with_environment_context(EnvironmentSettings::new(0), 0, || {
                set_climate(&[Value::Int(raw)])?;
                get_climate(&[])
            });

            let expected = raw.clamp(-50, 50);
            prop_assert!(matches!(result, Ok(Value::Int(value)) if value == expected));
            prop_assert_eq!(delta.climate, Some(expected));
            prop_assert!(delta.wind.is_none());
            prop_assert!(delta.temperature.is_none());
        }

        #[test]
        fn random_matches_chacha_stream(seed in any::<u64>(), range in 1i32..=1024) {
            let mut expected_rng = ChaCha8Rng::seed_from_u64(seed);
            let expected = expected_rng.gen_range(0..(range as u32)) as i32;

            let guard = enter_random_context(ChaCha8Rng::seed_from_u64(seed));
            let value = random(&[Value::Int(range)]).expect("Random with context succeeds");
            let _ = guard.finish();

            prop_assert_eq!(value, Value::Int(expected));
            prop_assert!(expected >= 0 && expected < range);
        }

        #[test]
        fn random_sequence_remains_deterministic(seed in any::<u64>()) {
            let mut expected_rng = ChaCha8Rng::seed_from_u64(seed);
            let expected = [
                expected_rng.gen_range(0..100) as i32,
                expected_rng.gen_range(0..100) as i32,
                expected_rng.gen_range(0..100) as i32,
            ];

            let guard = enter_random_context(ChaCha8Rng::seed_from_u64(seed));
            let first = random(&[Value::Int(100)]).expect("first draw succeeds");
            let second = random(&[Value::Int(100)]).expect("second draw succeeds");
            let third = random(&[Value::Int(100)]).expect("third draw succeeds");
            let _ = guard.finish();

            prop_assert_eq!(first, Value::Int(expected[0]));
            prop_assert_eq!(second, Value::Int(expected[1]));
            prop_assert_eq!(third, Value::Int(expected[2]));
        }
    }

    #[test]
    fn add_effect_captures_initial_vars() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(120),
                Value::Int(2),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(7),
                Value::Bool(true),
            ])
        });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.vars().len(), 2);
                assert_eq!(effect.vars()[0], EffectVarValue::Int(7));
                assert_eq!(effect.vars()[1], EffectVarValue::Bool(true));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn remove_effect_rejects_when_missing() {
        let state = empty_state();
        let (result, _) =
            with_object_host_context(|| remove_effect(&[Value::Nil, state.clone(), Value::Int(0)]));
        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn add_and_remove_effect_flow() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone()])?;
            remove_effect(&[Value::String("Glow".into()), state.clone()])
        });

        let value = result.expect("calls succeed");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.object.len(), 2);
        assert!(matches!(outcome.object[0], EffectCommand::Add(_)));
        assert!(matches!(
            outcome.object[1],
            EffectCommand::Remove {
                no_callbacks: false,
                ..
            }
        ));
    }

    #[test]
    fn remove_effect_can_skip_callbacks() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone()])?;
            remove_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Bool(true),
            ])
        });

        let value = result.expect("calls succeed");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.object.len(), 2);
        assert!(matches!(outcome.object[0], EffectCommand::Add(_)));
        assert!(matches!(
            outcome.object[1],
            EffectCommand::Remove {
                no_callbacks: true,
                ..
            }
        ));
    }

    #[test]
    fn set_action_respects_no_other_action() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_no_other_action(true),
        );
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library.clone(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action(&[Value::String("Walk".into())]),
        );

        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(2),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action(&[Value::String("Idle".into())]),
        );

        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("pending update exists");
        let action = update.action.expect("action update recorded");
        assert_eq!(action.name.as_deref(), Some("Idle"));
        assert!(!action.force);
    }

    #[test]
    fn set_action_data_records_object_update() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("bridge"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action_data(&[Value::Int(512)]),
        );

        let value = result.expect("SetActionData returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update present");
        assert_eq!(action.data, Some(255));
    }

    #[test]
    fn set_action_data_rejects_invalid_attach_vertices() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("attach"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action_data(&[Value::Int(31 << 8)]),
        );

        let value = result.expect("SetActionData returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_action_data_requires_active_object() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Inactive,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_action_data(&[Value::Int(5)]),
        );

        let value = result.expect("SetActionData returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_data_returns_zero_by_default() {
        let (result, outcome) = with_object_host_context(|| get_action_data(&[]));
        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(0));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_data_reflects_pending_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action_data(&[Value::Int(42)])?;
                get_action_data(&[])
            },
        );

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(42));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.data, Some(42));
    }

    #[test]
    fn get_action_data_reads_world_context() {
        let other = HostWorldObject::new(
            ObjectId::new(23),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            77,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(23));
            get_action_data(&[Value::Proplist(target)])
        });

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Int(77));
    }

    #[test]
    fn get_action_data_respects_target_filter() {
        let (result, _) = with_object_host_context(|| {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action_data(&[Value::Proplist(target)])
        });

        let value = result.expect("GetActionData succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_data_returns_nil_without_context() {
        let value = get_action_data(&[]).expect("GetActionData succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_returns_idle_by_default() {
        let (result, outcome) = with_object_host_context(|| get_action(&[]));
        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::String("Idle".into()));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_action_reflects_pending_update() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Walk".into())])?;
                get_action(&[])
            },
        );

        let value = result.expect("SetAction/GetAction succeed");
        assert_eq!(value, Value::String("Walk".into()));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.name.as_deref(), Some("Walk"));
    }

    #[test]
    fn get_procedure_returns_nil_when_unspecified() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_procedure(&[]),
        );

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_procedure_returns_configured_value() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_procedure(&[]),
        );

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::String("walk".into()));
    }

    #[test]
    fn get_procedure_reflects_pending_action_change() {
        let mut specs = HashMap::new();
        specs.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        specs.insert(
            "Float".to_string(),
            ActionSpec::default().with_procedure("float"),
        );
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Float".into())])?;
                get_procedure(&[])
            },
        );

        let value = result.expect("SetAction/GetProcedure succeed");
        assert_eq!(value, Value::String("float".into()));
    }

    #[test]
    fn get_procedure_reads_world_context() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(42),
            "Dummy",
            ObjectStatus::Normal,
            "Swim",
            None,
            None,
            Some("swim".to_string()),
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(42));
            get_procedure(&[Value::Proplist(target)])
        });

        let value = result.expect("GetProcedure succeeds");
        assert_eq!(value, Value::String("swim".into()));
    }

    #[test]
    fn get_action_respects_target_filter() {
        let (result, _) = with_object_host_context(|| {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            let target = Value::Proplist(target);
            get_action(&[target])
        });

        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_action_reads_other_object_from_world() {
        let other = HostWorldObject::new(
            ObjectId::new(99),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_object_host_context_with_world(world, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action(&[Value::Proplist(target)])
        });

        let value = result.expect("GetAction succeeds");
        assert_eq!(value, Value::String("Walk".into()));
    }

    #[test]
    fn get_action_uses_world_without_context() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Dummy",
            ObjectStatus::Normal,
            "Dig",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(7));
            get_action(&[Value::Proplist(target)])
        });

        let value = result.expect("GetAction resolves world lookup");
        assert_eq!(value, Value::String("Dig".into()));
    }

    #[test]
    fn get_action_returns_nil_without_context() {
        let value = get_action(&[]).expect("GetAction succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_act_time_returns_zero_by_default() {
        let (result, outcome) = with_object_host_context(|| get_act_time(&[]));
        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(0));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_act_time_reflects_pending_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[
                    Value::String("Idle".into()),
                    Value::Nil,
                    Value::Nil,
                    Value::Int(7),
                ])?;
                get_act_time(&[])
            },
        );

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(7));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.ticks, Some(7));
    }

    #[test]
    fn get_act_time_resets_on_action_change() {
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Walk".to_string(), ActionSpec::default());
        let library = ActionLibrary::new(Some("Idle".to_string()), specs);

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                5,
                0,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || {
                set_action(&[Value::String("Walk".into())])?;
                get_act_time(&[])
            },
        );

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(0));
        let update = outcome.object_update.expect("action update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.ticks, Some(0));
    }

    #[test]
    fn get_act_time_reads_world_context() {
        let other = HostWorldObject::new(
            ObjectId::new(23),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            12,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(23));
            get_act_time(&[Value::Proplist(target)])
        });

        let value = result.expect("GetActTime succeeds");
        assert_eq!(value, Value::Int(12));
    }

    #[test]
    fn get_act_time_returns_nil_without_context() {
        let value = get_act_time(&[]).expect("GetActTime succeeds without context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_vertex_num_counts_vertices() {
        let vertices = [ObjectVertex::new(0, 0), ObjectVertex::new(1, -1)];
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &vertices,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex_num(&[]),
        );

        let value = result.expect("GetVertexNum succeeds");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn get_vertex_returns_requested_attributes() {
        let vertex = ObjectVertex::new(2, -3)
            .with_cnat(CNAT_CENTER | CNAT_BOTTOM)
            .with_friction(7);
        let vertices = [vertex];
        let (x, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &vertices,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(0)]),
        );
        assert_eq!(x.expect("x succeeds"), Value::Int(2));
        let (y, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &vertices,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(1)]),
        );
        assert_eq!(y.expect("y succeeds"), Value::Int(-3));
        let (cnat, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &vertices,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(2)]),
        );
        assert_eq!(
            cnat.expect("cnat succeeds"),
            Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32)
        );
        let (friction, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &vertices,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_vertex(&[Value::Int(0), Value::Int(3)]),
        );
        assert_eq!(friction.expect("friction succeeds"), Value::Int(7));
    }

    #[test]
    fn get_vertex_contact_uses_landscape_sampling() {
        let vertices = [ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER | CNAT_BOTTOM)];
        let landscape = Landscape::flat(8, 0);
        let world =
            HostWorldContext::with_landscape(Vec::new(), Some(landscape), HashMap::new(), 1);
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &vertices,
            )),
            &[],
            world,
            1,
            || get_vertex_contact(&[Value::Int(0)]),
        );

        let value = result.expect("GetVertexContact succeeds");
        assert_eq!(value, Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32));
    }

    #[test]
    fn get_contact_aggregates_vertices() {
        let vertices = [
            ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER | CNAT_BOTTOM),
            ObjectVertex::new(0, -5).with_cnat(CNAT_TOP),
        ];
        let landscape = Landscape::flat(4, 0);
        let world =
            HostWorldContext::with_landscape(Vec::new(), Some(landscape), HashMap::new(), 1);
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &vertices,
            )),
            &[],
            world,
            1,
            || get_contact(&[Value::Int(-1)]),
        );

        let value = result.expect("GetContact succeeds");
        assert_eq!(value, Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32));
    }

    #[test]
    fn set_action_targets_records_target_updates() {
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(42));

        let (result, outcome) =
            with_object_host_context(|| set_action_targets(&[Value::Proplist(target_map.clone())]));

        let value = result.expect("SetActionTargets succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(
            action.target,
            Some(Some(ObjectId::new(42))),
            "target update recorded",
        );
        assert!(action.target2.is_none(), "second target untouched");
    }

    #[test]
    fn set_action_targets_updates_second_slot_when_provided() {
        let mut first = HashMap::new();
        first.insert("id".into(), Value::Int(5));
        let mut second = HashMap::new();
        second.insert("id".into(), Value::Int(6));

        let (result, outcome) = with_object_host_context(|| {
            set_action_targets(&[
                Value::Proplist(first.clone()),
                Value::Proplist(second.clone()),
            ])
        });

        let value = result.expect("SetActionTargets succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, Some(Some(ObjectId::new(5))));
        assert_eq!(action.target2, Some(Some(ObjectId::new(6))));
    }

    #[test]
    fn get_action_target_reflects_pending_update() {
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(12));

        let (result, outcome) = with_object_host_context(|| {
            set_action_targets(&[Value::Proplist(target_map.clone())])?;
            get_action_target(&[Value::Int(0)])
        });

        let value = result.expect("GetActionTarget succeeds");
        match value {
            Value::Proplist(map) => {
                assert_eq!(map.get("id"), Some(&Value::Int(12)));
            }
            other => panic!("expected proplist, got {:?}", other),
        }

        let update = outcome.object_update.expect("object update recorded");
        let action = update.action.expect("action update exists");
        assert_eq!(action.target, Some(Some(ObjectId::new(12))));
    }

    #[test]
    fn get_action_target_reads_world_context() {
        let other = HostWorldObject::new(
            ObjectId::new(99),
            "Dummy",
            ObjectStatus::Normal,
            "Walk",
            Some(ObjectId::new(77)),
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_object_host_context_with_world(world, || {
            let mut target = HashMap::new();
            target.insert("id".into(), Value::Int(99));
            get_action_target(&[Value::Int(0), Value::Proplist(target)])
        });

        let value = result.expect("GetActionTarget succeeds");
        match value {
            Value::Proplist(map) => {
                assert_eq!(map.get("id"), Some(&Value::Int(77)));
            }
            other => panic!("expected proplist, got {:?}", other),
        }
    }

    #[test]
    fn get_action_target_returns_nil_for_out_of_range_index() {
        let value = with_object_host_context(|| get_action_target(&[Value::Int(2)]))
            .0
            .expect("GetActionTarget succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn set_dir_records_direction_update() {
        let (result, outcome) = with_object_host_context(|| set_dir(&[Value::Int(1)]));
        let value = result.expect("SetDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn get_dir_observes_effective_direction() {
        let (result, outcome) = with_object_host_context(|| {
            set_dir(&[Value::Int(1)])?;
            get_dir(&[])
        });
        let value = result.expect("GetDir succeeds");
        assert_eq!(value, Value::Int(Direction::Right.to_script_value()));
        let update = outcome.object_update.expect("direction update recorded");
        assert_eq!(update.direction, Some(Direction::Right));
    }

    #[test]
    fn set_com_dir_records_command_direction_update() {
        let (result, outcome) = with_object_host_context(|| set_com_dir(&[Value::Int(3)]));
        let value = result.expect("SetComDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome
            .object_update
            .expect("command direction update recorded");
        assert_eq!(update.command_direction, Some(CommandDirection::Right));
    }

    #[test]
    fn get_com_dir_observes_effective_command_direction() {
        let (result, outcome) = with_object_host_context(|| {
            set_com_dir(&[Value::Int(4)])?;
            get_com_dir(&[])
        });
        let value = result.expect("GetComDir succeeds");
        assert_eq!(
            value,
            Value::Int(CommandDirection::DownRight.to_script_value())
        );
        let update = outcome
            .object_update
            .expect("command direction update recorded");
        assert_eq!(update.command_direction, Some(CommandDirection::DownRight));
    }

    #[test]
    fn get_x_returns_current_position() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(42, -7),
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_x(&[]),
        );

        let value = result.expect("GetX succeeds");
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn get_y_returns_current_position() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(2),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(-5, 63),
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_y(&[]),
        );

        let value = result.expect("GetY succeeds");
        assert_eq!(value, Value::Int(63));
    }

    #[test]
    fn get_x_reads_world_when_target_provided() {
        let other = HostWorldObject::new(
            ObjectId::new(99),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::new(-12, 34),
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(99))];

        let (result, _) = with_effect_context(None, &[], world, 1, || get_x(&args));
        let value = result.expect("GetX target succeeds");
        assert_eq!(value, Value::Int(-12));
    }

    #[test]
    fn get_y_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(1234))];
        let (result, _) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || get_y(&args));
        let value = result.expect("GetY handles missing target");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_x_dir_returns_object_velocity() {
        let context = HostObjectContext::new(
            ObjectId::new(7),
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::new(12, -3),
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            None,
            None,
            &[],
        );
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), 1, || {
                get_x_dir(&[])
            });
        let value = result.expect("GetXDir succeeds");
        assert_eq!(value, Value::Int(12));
    }

    #[test]
    fn get_y_dir_applies_precision_scaling() {
        let context = HostObjectContext::new(
            ObjectId::new(8),
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::new(0, 25),
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            None,
            None,
            &[],
        );
        let args = [Value::Nil, Value::Int(5)];
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), 1, || {
                get_y_dir(&args)
            });
        let value = result.expect("GetYDir succeeds");
        assert_eq!(value, Value::Int(13));
    }

    #[test]
    fn get_x_dir_reads_world_velocity_when_target_provided() {
        let other = HostWorldObject::new(
            ObjectId::new(42),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::new(-8, 3),
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(42))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_x_dir(&args));
        let value = result.expect("GetXDir target succeeds");
        assert_eq!(value, Value::Int(-8));
    }

    #[test]
    fn get_x_dir_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(77))];
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            get_x_dir(&args)
        });
        let value = result.expect("GetXDir handles missing target");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn set_x_dir_records_object_update() {
        let args = [Value::Int(15)];
        let (result, outcome) = with_object_host_context(|| set_x_dir(&args));
        let value = result.expect("SetXDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("velocity update recorded");
        assert_eq!(update.velocity, Some(Vector2::new(15, 0)));
    }

    #[test]
    fn set_y_dir_applies_precision_when_recording_update() {
        let args = [Value::Int(5), Value::Nil, Value::Int(5)];
        let (result, outcome) = with_object_host_context(|| set_y_dir(&args));
        let value = result.expect("SetYDir succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("velocity update recorded");
        assert_eq!(update.velocity, Some(Vector2::new(0, 10)));
    }

    #[test]
    fn set_x_dir_respects_target_filter() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(4), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_x_dir(&args));
        let value = result.expect("SetXDir returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_position_records_object_update() {
        let args = [Value::Int(15), Value::Int(27)];
        let (result, outcome) = with_object_host_context(|| set_position(&args));

        let value = result.expect("SetPosition succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("position update recorded");
        assert_eq!(update.position, Some(Vector2::new(15, 27)));
    }

    #[test]
    fn set_position_respects_target_filter() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(42));
        let args = [Value::Int(5), Value::Int(6), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| set_position(&args));

        let value = result.expect("SetPosition returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_position_clamps_coordinates_when_requested() {
        let landscape = Landscape::flat(4, 6);
        let world =
            HostWorldContext::with_landscape(Vec::new(), Some(landscape), HashMap::new(), 1);
        let args = [
            Value::Int(10),
            Value::Int(20),
            Value::Nil,
            Value::Bool(true),
        ];
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[ObjectVertex::new(0, 0)],
            )),
            &[],
            world,
            1,
            || set_position(&args),
        );

        let value = result.expect("SetPosition returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("position update recorded");
        assert_eq!(update.position, Some(Vector2::new(3, 6)));
    }

    #[test]
    fn get_x_rejects_additional_arguments() {
        let (result, _) = with_object_host_context(|| get_x(&[Value::Nil, Value::Nil]));
        let error = result.expect_err("GetX rejects extra arguments");
        assert_eq!(error.to_string(), "GetX expects at most 1 argument: target");
    }

    #[test]
    fn get_effect_uses_context_view() {
        let state = empty_state();
        let (result, _) = with_object_host_context(|| {
            add_effect(&[Value::String("Glow".into()), state.clone()])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(1),
            ])
        });

        let value = result.expect("GetEffect succeeds");
        assert_eq!(value, Value::String("Glow".into()));
    }

    #[test]
    fn get_effect_returns_command_target_metadata() {
        let state = empty_state();
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(7));
        let target = Value::Proplist(target_map);

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                target.clone(),
                Value::String("BARL".into()),
            ])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(4),
            ])
        });
        let value = result.expect("GetEffect command target succeeds");
        assert_eq!(value, Value::Int(7));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                target.clone(),
                Value::String("BARL".into()),
            ])?;
            get_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(0),
                Value::Int(5),
            ])
        });
        let value = result.expect("GetEffect command id succeeds");
        assert_eq!(value, Value::String("BARL".into()));
    }

    #[test]
    fn get_effect_count_filters_by_name_and_priority() {
        let state = empty_state();
        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::Nil, state.clone()])
        });
        let value = result.expect("GetEffectCount succeeds");
        assert_eq!(value, Value::Int(3));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::String("Glow".into()), state.clone()])
        });
        let value = result.expect("GetEffectCount with name succeeds");
        assert_eq!(value, Value::Int(1));

        let (result, _) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), state.clone(), Value::Int(120)])?;
            add_effect(&[Value::String("Spark".into()), state.clone(), Value::Int(80)])?;
            add_effect(&[Value::String("Flame".into()), state.clone(), Value::Int(50)])?;
            get_effect_count(&[Value::Nil, state.clone(), Value::Int(90)])
        });
        let value = result.expect("GetEffectCount with priority succeeds");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn get_effect_count_reads_state_snapshot_when_no_context() {
        let mut glow = HashMap::new();
        glow.insert("name".into(), Value::String("Glow".into()));
        glow.insert("priority".into(), Value::Int(100));
        glow.insert("interval".into(), Value::Int(1));
        glow.insert("timer".into(), Value::Int(0));

        let mut spark = HashMap::new();
        spark.insert("name".into(), Value::String("Spark".into()));
        spark.insert("priority".into(), Value::Int(60));
        spark.insert("interval".into(), Value::Int(1));
        spark.insert("timer".into(), Value::Int(0));

        let state = {
            let mut map = HashMap::new();
            map.insert(
                "effects".into(),
                Value::Array(vec![Value::Proplist(glow), Value::Proplist(spark)]),
            );
            Value::Proplist(map)
        };

        let value = get_effect_count(&[Value::Nil, state.clone(), Value::Nil])
            .expect("GetEffectCount without context succeeds");
        assert_eq!(value, Value::Int(2));

        let value = get_effect_count(&[Value::String("Spark".into()), state, Value::Int(50)])
            .expect("GetEffectCount with state filter succeeds");
        assert_eq!(value, Value::Int(0));
    }

    #[test]
    fn effect_var_reads_and_writes_values() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Spark".into()),
                state.clone(),
                Value::Int(100),
                Value::Int(1),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(3),
            ])?;

            let initial = effect_var(&[Value::Int(0), state.clone(), Value::Int(1)])?;
            assert_eq!(initial, Value::Int(3));

            let unset = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])?;
            assert_eq!(unset, Value::Nil);

            let updated = effect_var(&[
                Value::Int(1),
                state.clone(),
                Value::Int(1),
                Value::String("beam".into()),
            ])?;
            assert_eq!(updated, Value::String("beam".into()));

            let reread = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])?;
            assert_eq!(reread, Value::String("beam".into()));

            Ok(Value::Nil)
        });

        result.expect("EffectVar interactions succeed");
        assert_eq!(outcome.object.len(), 2);
        match &outcome.object[1] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.vars().len(), 2);
                assert_eq!(effect.vars()[0], EffectVarValue::Int(3));
                assert_eq!(effect.vars()[1], EffectVarValue::String("beam".into()));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn effect_var_reads_from_state_without_context() {
        let mut effect_map = HashMap::new();
        effect_map.insert("name".into(), Value::String("Glow".into()));
        effect_map.insert("priority".into(), Value::Int(80));
        effect_map.insert("interval".into(), Value::Int(1));
        effect_map.insert("timer".into(), Value::Int(0));
        effect_map.insert(
            "vars".into(),
            Value::Array(vec![Value::Int(9), Value::String("pulse".into())]),
        );

        let mut state_map = HashMap::new();
        state_map.insert(
            "effects".into(),
            Value::Array(vec![Value::Proplist(effect_map.clone())]),
        );
        let state = Value::Proplist(state_map);

        let read_value = effect_var(&[Value::Int(0), state.clone(), Value::Int(1)])
            .expect("EffectVar read succeeds");
        assert_eq!(read_value, Value::Int(9));

        let read_string = effect_var(&[Value::Int(1), state.clone(), Value::Int(1)])
            .expect("EffectVar string read succeeds");
        assert_eq!(read_string, Value::String("pulse".into()));

        let set_result = effect_var(&[Value::Int(0), state, Value::Int(1), Value::Int(5)]);
        assert!(set_result.is_err());
    }

    #[test]
    fn set_action_records_object_update() {
        let args = vec![Value::String("Walk".into())];
        let (result, outcome) = with_object_host_context(|| set_action(&args));
        let value = result.expect("SetAction should succeed");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("action update present");
        let action = update.action.expect("action delta present");
        assert_eq!(action.name.as_deref(), Some("Walk"));
        assert!(outcome.object_commands.is_empty());
        assert!(!outcome.destroy_object);
    }

    #[test]
    fn set_action_respects_target_id() {
        let mut target_map = HashMap::new();
        target_map.insert("id".into(), Value::Int(2));
        let args = vec![Value::String("Jump".into()), Value::Proplist(target_map)];
        let (result, outcome) = with_object_host_context(|| set_action(&args));
        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_object_status_records_update() {
        let args = vec![Value::Int(ObjectStatus::Inactive.to_script_value())];
        let (result, outcome) = with_object_host_context(|| set_object_status(&args));
        let value = result.expect("SetObjectStatus succeeds");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("status update present");
        assert_eq!(update.status, Some(ObjectStatus::Inactive));
    }

    #[test]
    fn set_object_status_rejects_deleted() {
        let args = vec![Value::Int(ObjectStatus::Deleted.to_script_value())];
        let (result, outcome) = with_object_host_context(|| set_object_status(&args));
        let value = result.expect("SetObjectStatus returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_object_status_reflects_pending_update() {
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let set_value =
                set_object_status(&[Value::Int(ObjectStatus::Inactive.to_script_value())])?;
            assert_eq!(set_value, Value::Bool(true));
            get_object_status(&[])
        });

        let value = result.expect("GetObjectStatus succeeds");
        assert_eq!(value, Value::Int(ObjectStatus::Inactive.to_script_value()));
        let update = outcome.object_update.expect("status update present");
        assert_eq!(update.status, Some(ObjectStatus::Inactive));
    }

    #[test]
    fn get_owner_returns_current_owner() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                5,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_owner(&[]),
        );

        let value = result.expect("GetOwner succeeds");
        assert_eq!(value, Value::Int(5));
    }

    #[test]
    fn get_owner_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            42,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [object_reference_value(ObjectId::new(7))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_owner(&args));

        let value = result.expect("GetOwner for target succeeds");
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn set_owner_records_owner_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                1,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_owner(&[Value::Int(3)]),
        );

        let value = result.expect("SetOwner returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("owner update recorded");
        assert_eq!(update.owner, Some(3));
    }

    #[test]
    fn set_owner_respects_target_filter() {
        let world = HostWorldContext::default();
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(2), Value::Proplist(target)];

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            world,
            1,
            || set_owner(&args),
        );

        let value = result.expect("SetOwner returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn set_alive_records_alive_update() {
        let (result, outcome) = with_effect_context(
            Some(
                HostObjectContext::new(
                    ObjectId::new(1),
                    None,
                    ObjectStatus::Normal,
                    100,
                    OWNER_NONE,
                    Vector2::ZERO,
                    Vector2::ZERO,
                    &[],
                    "Idle",
                    0,
                    0,
                    ActionLibrary::default(),
                    Direction::Left,
                    CommandDirection::Stop,
                    None,
                    None,
                    &[],
                )
                .with_alive(true),
            ),
            &[],
            HostWorldContext::default(),
            1,
            || set_alive(&[Value::Bool(false)]),
        );

        let value = result.expect("SetAlive returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("alive update recorded");
        assert_eq!(update.alive, Some(false));
    }

    #[test]
    fn set_alive_respects_target_filter() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(42));
        let args = [Value::Bool(true), Value::Proplist(target)];

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || set_alive(&args),
        );

        let value = result.expect("SetAlive returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn get_alive_returns_current_state() {
        let (result, _) = with_effect_context(
            Some(
                HostObjectContext::new(
                    ObjectId::new(1),
                    None,
                    ObjectStatus::Normal,
                    100,
                    OWNER_NONE,
                    Vector2::ZERO,
                    Vector2::ZERO,
                    &[],
                    "Idle",
                    0,
                    0,
                    ActionLibrary::default(),
                    Direction::Left,
                    CommandDirection::Stop,
                    None,
                    None,
                    &[],
                )
                .with_alive(false),
            ),
            &[],
            HostWorldContext::default(),
            1,
            || get_alive(&[]),
        );

        let value = result.expect("GetAlive returns bool");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn get_alive_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_alive(false)]);
        let args = [object_reference_value(ObjectId::new(7))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_alive(&args));

        let value = result.expect("GetAlive for target succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn do_energy_applies_delta_and_clamps() {
        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(-25)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(75));

        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(50)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(100));
    }

    #[test]
    fn do_energy_respects_target_argument() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(-10), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| do_energy(&args));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn do_damage_applies_delta_and_clamps() {
        let (result, outcome) = with_object_host_context(|| do_damage(&[Value::Int(15)]));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("damage update recorded");
        assert_eq!(update.damage, Some(15));

        let (result, outcome) = with_object_host_context(|| do_damage(&[Value::Int(-20)]));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("damage update recorded");
        assert_eq!(update.damage, Some(0));
    }

    #[test]
    fn do_damage_respects_target_argument() {
        let mut target = HashMap::new();
        target.insert("id".into(), Value::Int(77));
        let args = [Value::Int(5), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| do_damage(&args));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn do_energy_accepts_exact_flag() {
        let args = [Value::Int(0), Value::Nil, Value::Bool(true)];
        let (result, outcome) = with_object_host_context(|| do_energy(&args));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        assert!(outcome
            .object_update
            .as_ref()
            .and_then(|update| update.energy)
            .is_some());
    }

    #[test]
    fn get_energy_returns_current_energy() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                75,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_energy(&[]),
        );

        let value = result.expect("GetEnergy succeeds");
        assert_eq!(value, Value::Int(75));
    }

    #[test]
    fn get_energy_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(55),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            33,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [object_reference_value(ObjectId::new(55))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_energy(&args));

        let value = result.expect("GetEnergy target succeeds");
        assert_eq!(value, Value::Int(33));
    }

    #[test]
    fn get_energy_returns_nil_without_context() {
        let (result, _) =
            with_effect_context(
                None,
                &[],
                HostWorldContext::default(),
                1,
                || get_energy(&[]),
            );
        let value = result.expect("GetEnergy handles missing context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_energy_converts_raw_units_to_percent() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(3),
                None,
                ObjectStatus::Normal,
                LEGACY_MAX_PHYSICAL / 2,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_energy(&[]),
        );

        let value = result.expect("GetEnergy converts raw energy");
        assert_eq!(value, Value::Int(50));
    }

    #[test]
    fn create_object_registers_spawn_and_returns_reference() {
        let args = [Value::String("Clonk".into())];
        let (result, outcome) = with_object_host_context(|| create_object(&args));
        let value = result.expect("CreateObject succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
        assert_eq!(outcome.spawns.len(), 1);
        let spawn = &outcome.spawns[0];
        assert_eq!(spawn.definition_id, "Clonk");
        assert_eq!(spawn.position, Vector2::ZERO);
        assert_eq!(spawn.owner, OWNER_NONE);
        assert_eq!(spawn.id, Some(ObjectId::new(1)));
        assert_eq!(outcome.next_object_id, 2);
    }

    #[test]
    fn create_particle_registers_command() {
        let args = [
            Value::String("Smoke".into()),
            Value::Int(8),
            Value::Int(-4),
            Value::Int(20),
            Value::Int(-10),
            Value::Int(15),
            Value::Int(60),
        ];
        let (result, outcome) = with_object_host_context(|| create_particle(&args));
        let value = result.expect("CreateParticle succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Create(config) => {
                assert_eq!(config.definition_id, "Smoke");
                assert_eq!(config.position, FloatVector2::new(8.0, -4.0));
                assert_eq!(config.velocity, FloatVector2::new(2.0, -1.0));
                assert_eq!(config.parameter_a, 1.5);
                assert_eq!(config.parameter_b, 60);
                assert_eq!(config.life, 60);
                assert!(matches!(config.layer, ParticleLayer::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn create_particle_with_object_sets_layer() {
        let target_id = ObjectId::new(5);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Torch",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Spark".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(30),
            object_reference_value(target_id),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || create_particle(&args));
        let value = result.expect("CreateParticle succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Create(config) => {
                assert!(matches!(
                    config.layer,
                    ParticleLayer::ObjectBack(id) if id == target_id
                ));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn create_particle_rejects_unknown_object() {
        let args = [
            Value::String("Spark".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(10),
            object_reference_value(ObjectId::new(99)),
        ];
        let (result, outcome) = with_object_host_context(|| create_particle(&args));
        let value = result.expect("CreateParticle handles missing object");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.particles.is_empty());
    }

    #[test]
    fn clear_particles_registers_command() {
        let (result, outcome) = with_object_host_context(|| clear_particles(&[]));
        let value = result.expect("ClearParticles succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Clear {
                definition_id,
                scope,
            } => {
                assert!(definition_id.is_none());
                assert!(matches!(scope, ParticleScope::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn clear_particles_with_object_sets_scope() {
        let target_id = ObjectId::new(12);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Emitter",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Smoke".into()),
            object_reference_value(target_id),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || clear_particles(&args));
        let value = result.expect("ClearParticles succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Clear {
                definition_id,
                scope,
            } => {
                assert_eq!(definition_id.as_deref(), Some("Smoke"));
                assert!(matches!(scope, ParticleScope::Object(id) if *id == target_id));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn contained_returns_nil_when_object_has_no_container() {
        let (result, _) = with_object_host_context(|| contained(&[]));
        let value = result.expect("Contained without container succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn contained_returns_container_reference() {
        let container_id = ObjectId::new(42);
        let object_id = ObjectId::new(7);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                container_id,
                "Chest",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                0,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                object_id,
                "Gem",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                0,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container_id),
            ),
        ]);
        let context = HostObjectContext::new(
            object_id,
            Some(container_id),
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            None,
            None,
            &[],
        );
        let (result, _) = with_effect_context(Some(context), &[], world, 100, || contained(&[]));
        let value = result.expect("Contained with container succeeds");
        match value {
            Value::Proplist(map) => {
                assert_eq!(
                    map.get("id"),
                    Some(&Value::Int(container_id.as_u64() as i32))
                );
            }
            other => panic!("expected proplist for container reference, got {other:?}"),
        }
    }

    #[test]
    fn remove_object_marks_destroy_flag() {
        let (result, outcome) = with_object_host_context(|| remove_object(&[]));
        assert_eq!(result.expect("RemoveObject succeeds"), Value::Bool(true));
        assert!(outcome.destroy_object);
    }

    #[test]
    fn find_object_returns_first_matching_definition() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(1),
                "Flag",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                Vector2::new(10, 5),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(2),
                "Rock",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                Vector2::new(50, 5),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);

        let args = [Value::String("Flag".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
    }

    #[test]
    fn find_object_respects_owner_filter() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(10),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                1,
                100,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(11),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                2,
                100,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::String("Dummy".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(2),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject owner succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(11)));
    }

    #[test]
    fn find_object_closest_mode_orders_by_distance() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(20),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                Vector2::new(2, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(21),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                Vector2::new(6, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::String("Dummy".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(-1),
        ];
        let (first_result, _) =
            with_effect_context(None, &[], world.clone(), 1, || find_object(&args));
        let first_value = first_result.expect("FindObject closest succeeds");
        assert_eq!(first_value, object_reference_value(ObjectId::new(20)));

        let mut find_next = HashMap::new();
        find_next.insert("id".into(), Value::Int(20));
        let args_with_next = [
            Value::String("Dummy".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(-1),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Proplist(find_next),
        ];
        let (second_result, _) =
            with_effect_context(None, &[], world, 1, || find_object(&args_with_next));
        let second_value = second_result.expect("FindObject closest with next succeeds");
        assert_eq!(second_value, object_reference_value(ObjectId::new(21)));
    }

    #[test]
    fn find_objects_returns_all_matches_in_order() {
        let container = ObjectId::new(40);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                container,
                "Container",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(41),
                "Item",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                Vector2::new(3, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container),
            ),
            HostWorldObject::new(
                ObjectId::new(42),
                "Item",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container),
            ),
        ]);
        let args = [
            Value::String("Item".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(ANY_CONTAINER_SENTINEL),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_objects(&args));
        let value = result.expect("FindObjects succeeds");
        match value {
            Value::Array(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0], object_reference_value(ObjectId::new(41)));
                assert_eq!(entries[1], object_reference_value(ObjectId::new(42)));
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    proptest! {
        #[test]
        fn do_energy_sequence_clamps_within_bounds(deltas in proptest::collection::vec(-200..=200i32, 0..16)) {
            let start_energy = DEFAULT_MAX_ENERGY;
            let expected = expected_energy_after_sequence(start_energy, &deltas);

            let sequence = deltas.clone();
            let (result, outcome) = with_object_host_context(move || {
                for delta in sequence.iter().copied() {
                    let value = do_energy(&[Value::Int(delta)])?;
                    match value {
                        Value::Bool(true) => {}
                        Value::Bool(false) => {
                            return Err(RuntimeError::new("DoEnergy rejected update"));
                        }
                        other => {
                            return Err(RuntimeError::new(format!(
                                "DoEnergy returned unexpected value: {}",
                                other.type_name()
                            )));
                        }
                    }
                }
                Ok(Value::Nil)
            });

            prop_assert!(result.is_ok());

            let final_energy = outcome
                .object_update
                .and_then(|update| update.energy)
                .unwrap_or(start_energy);

            prop_assert_eq!(final_energy, expected);
        }
    }

    fn expected_energy_after_sequence(start: i32, deltas: &[i32]) -> i32 {
        let mut energy = start;
        for &delta in deltas {
            energy = energy.saturating_add(delta);
            if energy < 0 {
                energy = 0;
            } else if energy > DEFAULT_MAX_ENERGY {
                energy = DEFAULT_MAX_ENERGY;
            }
        }
        energy
    }

    #[test]
    fn add_global_effect_records_global_command() {
        let (result, outcome) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || {
                add_effect(&[Value::String("Glow".into()), Value::Nil, Value::Int(120)])
            });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert!(outcome.object.is_empty());
        assert_eq!(outcome.global.len(), 1);
        match &outcome.global[0] {
            EffectCommand::Add(effect) => {
                assert_eq!(effect.name, "Glow");
                assert_eq!(effect.priority, 120);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn global_effect_queries_use_context_view() {
        let (result, _) = with_effect_context(
            None,
            &[],
            HostWorldContext::default(),
            1,
            || -> Result<Value, RuntimeError> {
                add_effect(&[Value::String("Glow".into()), Value::Nil, Value::Int(90)])?;
                get_effect(&[
                    Value::String("Glow".into()),
                    Value::Nil,
                    Value::Int(0),
                    Value::Int(1),
                ])
            },
        );

        let value = result.expect("GetEffect succeeds");
        assert_eq!(value, Value::String("Glow".into()));
    }

    #[test]
    fn remove_global_effect_handles_missing() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            remove_effect(&[Value::Nil, Value::Nil, Value::Int(0)])
        });

        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }
}
