use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::rc::Rc;

use crate::effect::{EffectCommand, EffectState, EffectVarValue};
use crate::{
    ActionLibrary, ActionUpdate, CommandDirection, Direction, EnvironmentSettings, Landscape,
    ObjectId, ObjectStatus, ObjectUpdate, ObjectVertex, QueuedCommand, Vector2, CNAT_BOTTOM,
    CNAT_CENTER, CNAT_LEFT, CNAT_NO_COLLISION, CNAT_RIGHT, CNAT_TOP, OWNER_NONE,
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
}

#[derive(Debug, Clone)]
pub(crate) struct HostWorldObject {
    pub id: ObjectId,
    pub action_name: String,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub action_procedure: Option<String>,
    pub owner: i32,
    pub position: Vector2,
    #[allow(dead_code)]
    pub velocity: Vector2,
    pub vertices: Vec<ObjectVertex>,
    pub action_ticks: u32,
}

impl HostWorldObject {
    pub(crate) fn new(
        id: ObjectId,
        action_name: impl Into<String>,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        action_procedure: Option<String>,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        vertices: Vec<ObjectVertex>,
        action_ticks: u32,
    ) -> Self {
        Self {
            id,
            action_name: action_name.into(),
            action_target,
            action_target2,
            action_procedure,
            owner,
            position,
            velocity,
            vertices,
            action_ticks,
        }
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
}

#[derive(Debug, Clone)]
pub(crate) struct HostWorldContext {
    objects: Rc<HashMap<ObjectId, HostWorldObject>>,
    landscape: Option<Rc<Landscape>>,
}

impl Default for HostWorldContext {
    fn default() -> Self {
        Self {
            objects: Rc::new(HashMap::new()),
            landscape: None,
        }
    }
}

impl HostWorldContext {
    #[cfg(test)]
    pub(crate) fn from_objects<I>(objects: I) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        Self::with_landscape(objects, None)
    }

    pub(crate) fn with_landscape<I>(objects: I, landscape: Option<Landscape>) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        let map = objects
            .into_iter()
            .map(|object| (object.id, object))
            .collect();
        Self {
            objects: Rc::new(map),
            landscape: landscape.map(Rc::new),
        }
    }

    pub(crate) fn get(&self, id: ObjectId) -> Option<&HostWorldObject> {
        self.objects.get(&id)
    }

    pub(crate) fn landscape_ref(&self) -> Option<&Landscape> {
        self.landscape.as_deref()
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
    script.register_host_function("GetAction", get_action);
    script.register_host_function("GetActTime", get_act_time);
    script.register_host_function("GetProcedure", get_procedure);
    script.register_host_function("SetActionTargets", set_action_targets);
    script.register_host_function("GetActionTarget", get_action_target);
    script.register_host_function("GetVertexNum", get_vertex_num);
    script.register_host_function("GetVertex", get_vertex);
    script.register_host_function("GetVertexContact", get_vertex_contact);
    script.register_host_function("GetContact", get_contact);
    script.register_host_function("SetDir", set_dir);
    script.register_host_function("GetDir", get_dir);
    script.register_host_function("SetComDir", set_com_dir);
    script.register_host_function("GetComDir", get_com_dir);
    script.register_host_function("SetXDir", set_x_dir);
    script.register_host_function("GetXDir", get_x_dir);
    script.register_host_function("SetYDir", set_y_dir);
    script.register_host_function("GetYDir", get_y_dir);
    script.register_host_function("GetX", get_x);
    script.register_host_function("GetY", get_y);
    script.register_host_function("SetPosition", set_position);
    script.register_host_function("SetOwner", set_owner);
    script.register_host_function("GetOwner", get_owner);
    script.register_host_function("SetObjectStatus", set_object_status);
    script.register_host_function("GetObjectStatus", get_object_status);
    script.register_host_function("DoEnergy", do_energy);
    script.register_host_function("Random", random);
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
    pub status: ObjectStatus,
    pub energy: i32,
    pub owner: i32,
    pub position: Vector2,
    pub velocity: Vector2,
    pub effects: &'a [EffectState],
    pub action_name: String,
    pub action_ticks: u32,
    pub action_library: ActionLibrary,
    pub direction: Direction,
    pub command_direction: CommandDirection,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub vertices: &'a [ObjectVertex],
}

impl<'a> HostObjectContext<'a> {
    pub fn new(
        id: ObjectId,
        status: ObjectStatus,
        energy: i32,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        effects: &'a [EffectState],
        action_name: impl Into<String>,
        action_ticks: u32,
        action_library: ActionLibrary,
        direction: Direction,
        command_direction: CommandDirection,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: &'a [ObjectVertex],
    ) -> Self {
        Self {
            id,
            status,
            energy,
            owner,
            position,
            velocity,
            effects,
            action_name: action_name.into(),
            action_ticks,
            action_library,
            direction,
            command_direction,
            action_target,
            action_target2,
            vertices,
        }
    }
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
}

impl EffectContextOutcome {
    fn new(
        object: Vec<EffectCommand>,
        global: Vec<EffectCommand>,
        object_update: Option<ObjectUpdate>,
        object_commands: Vec<QueuedCommand>,
        destroy_object: bool,
        environment: Option<EnvironmentDelta>,
    ) -> Self {
        Self {
            object,
            global,
            object_update,
            object_commands,
            destroy_object,
            environment,
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            object: Vec::new(),
            global: Vec::new(),
            object_update: None,
            object_commands: Vec::new(),
            destroy_object: false,
            environment: None,
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

        object.update_effective_action(&name);

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

            if let Some(other) = context.world.get(target) {
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

            if let Some(other) = context.world.get(target) {
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

            if let Some(other) = context.world.get(target) {
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

            if let Some(other) = context.world.get(target) {
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

        let landscape = context.world.landscape_ref();
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

        let landscape = context.world.landscape_ref();

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

            if let Some(other) = context.world.get(target) {
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

            if let Some(other) = context.world.get(target) {
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
            context.world.landscape_ref().cloned()
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
            if let Some(other) = context.world.get(target) {
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
}

impl EffectHostContext {
    fn new(
        object: Option<HostObjectContext<'_>>,
        global_effects: Vec<EffectState>,
        world: HostWorldContext,
    ) -> Self {
        let object = object.map(|ctx| {
            let HostObjectContext {
                id,
                status,
                energy,
                owner,
                position,
                velocity,
                effects,
                action_name,
                action_ticks,
                action_library,
                direction,
                command_direction,
                action_target,
                action_target2,
                vertices,
            } = ctx;
            ObjectScopeContext::new(
                id,
                status,
                energy,
                owner,
                position,
                velocity,
                effects.to_vec(),
                action_library,
                action_name,
                action_ticks,
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

        EffectContextOutcome::new(
            object_effects,
            global,
            object_update,
            object_commands,
            destroy,
            None,
        )
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
    current_action_ticks: u32,
    current_energy: i32,
    max_energy: i32,
    current_owner: i32,
    current_direction: Direction,
    current_command_direction: CommandDirection,
    current_position: Vector2,
    current_velocity: Vector2,
    vertices: Vec<ObjectVertex>,
}

impl ObjectScopeContext {
    fn new(
        id: ObjectId,
        status: ObjectStatus,
        energy: i32,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        effects: Vec<EffectState>,
        action_library: ActionLibrary,
        action_name: String,
        action_ticks: u32,
        direction: Direction,
        command_direction: CommandDirection,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: Vec<ObjectVertex>,
    ) -> Self {
        let blocks_other_actions = action_library.blocks_other_actions(&action_name);
        let max_energy = energy.max(DEFAULT_MAX_ENERGY);
        Self {
            id,
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
            current_action_ticks: action_ticks,
            current_energy: energy,
            max_energy,
            current_owner: owner,
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

    fn update_effective_action(&mut self, action: &str) {
        self.current_action_name = action.to_string();
        self.current_action_blocks_other_actions = self.action_library.blocks_other_actions(action);
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            || set_action(&[Value::String("Walk".into())]),
        );

        let value = result.expect("SetAction returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());

        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(2),
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            "Swim",
            None,
            None,
            Some("swim".to_string()),
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
        )]);
        let (result, _) = with_effect_context(None, &[], world, || {
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
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
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
            "Dig",
            None,
            None,
            None,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
        )]);
        let (result, _) = with_effect_context(None, &[], world, || {
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                5,
                library,
                Direction::Left,
                CommandDirection::Stop,
                None,
                None,
                &[],
            )),
            &[],
            HostWorldContext::default(),
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
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            12,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let (result, _) = with_effect_context(None, &[], world, || {
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            || get_vertex(&[Value::Int(0), Value::Int(0)]),
        );
        assert_eq!(x.expect("x succeeds"), Value::Int(2));
        let (y, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            || get_vertex(&[Value::Int(0), Value::Int(1)]),
        );
        assert_eq!(y.expect("y succeeds"), Value::Int(-3));
        let (cnat, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            || get_vertex(&[Value::Int(0), Value::Int(2)]),
        );
        assert_eq!(
            cnat.expect("cnat succeeds"),
            Value::Int((CNAT_CENTER | CNAT_BOTTOM) as i32)
        );
        let (friction, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            || get_vertex(&[Value::Int(0), Value::Int(3)]),
        );
        assert_eq!(friction.expect("friction succeeds"), Value::Int(7));
    }

    #[test]
    fn get_vertex_contact_uses_landscape_sampling() {
        let vertices = [ObjectVertex::new(0, 0).with_cnat(CNAT_CENTER | CNAT_BOTTOM)];
        let landscape = Landscape::flat(8, 0);
        let world = HostWorldContext::with_landscape(Vec::new(), Some(landscape));
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
        let world = HostWorldContext::with_landscape(Vec::new(), Some(landscape));
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            "Walk",
            Some(ObjectId::new(77)),
            None,
            None,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(42, -7),
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::new(-5, 63),
                Vector2::ZERO,
                &[],
                "Idle",
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
            || get_y(&[]),
        );

        let value = result.expect("GetY succeeds");
        assert_eq!(value, Value::Int(63));
    }

    #[test]
    fn get_x_reads_world_when_target_provided() {
        let other = HostWorldObject::new(
            ObjectId::new(99),
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            Vector2::new(-12, 34),
            Vector2::ZERO,
            Vec::new(),
            0,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(99))];

        let (result, _) = with_effect_context(None, &[], world, || get_x(&args));
        let value = result.expect("GetX target succeeds");
        assert_eq!(value, Value::Int(-12));
    }

    #[test]
    fn get_y_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(1234))];
        let (result, _) =
            with_effect_context(None, &[], HostWorldContext::default(), || get_y(&args));
        let value = result.expect("GetY handles missing target");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_x_dir_returns_object_velocity() {
        let context = HostObjectContext::new(
            ObjectId::new(7),
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::new(12, -3),
            &[],
            "Idle",
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            None,
            None,
            &[],
        );
        let (result, _) =
            with_effect_context(Some(context), &[], HostWorldContext::default(), || {
                get_x_dir(&[])
            });
        let value = result.expect("GetXDir succeeds");
        assert_eq!(value, Value::Int(12));
    }

    #[test]
    fn get_y_dir_applies_precision_scaling() {
        let context = HostObjectContext::new(
            ObjectId::new(8),
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::new(0, 25),
            &[],
            "Idle",
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
            with_effect_context(Some(context), &[], HostWorldContext::default(), || {
                get_y_dir(&args)
            });
        let value = result.expect("GetYDir succeeds");
        assert_eq!(value, Value::Int(13));
    }

    #[test]
    fn get_x_dir_reads_world_velocity_when_target_provided() {
        let other = HostWorldObject::new(
            ObjectId::new(42),
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::new(-8, 3),
            Vec::new(),
            0,
        );
        let world = HostWorldContext::from_objects(vec![other]);
        let args = [object_reference_value(ObjectId::new(42))];
        let (result, _) = with_effect_context(None, &[], world, || get_x_dir(&args));
        let value = result.expect("GetXDir target succeeds");
        assert_eq!(value, Value::Int(-8));
    }

    #[test]
    fn get_x_dir_returns_nil_for_missing_target() {
        let args = [object_reference_value(ObjectId::new(77))];
        let (result, _) =
            with_effect_context(None, &[], HostWorldContext::default(), || get_x_dir(&args));
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
        let world = HostWorldContext::with_landscape(Vec::new(), Some(landscape));
        let args = [
            Value::Int(10),
            Value::Int(20),
            Value::Nil,
            Value::Bool(true),
        ];
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                5,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            || get_owner(&[]),
        );

        let value = result.expect("GetOwner succeeds");
        assert_eq!(value, Value::Int(5));
    }

    #[test]
    fn get_owner_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(7),
            "Idle",
            None,
            None,
            None,
            42,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
        )]);
        let args = [object_reference_value(ObjectId::new(7))];
        let (result, _) = with_effect_context(None, &[], world, || get_owner(&args));

        let value = result.expect("GetOwner for target succeeds");
        assert_eq!(value, Value::Int(42));
    }

    #[test]
    fn set_owner_records_owner_update() {
        let (result, outcome) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                ObjectStatus::Normal,
                100,
                1,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
                ObjectStatus::Normal,
                100,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
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
            || set_owner(&args),
        );

        let value = result.expect("SetOwner returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
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
        let (result, outcome) = with_effect_context(None, &[], HostWorldContext::default(), || {
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
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), || {
            remove_effect(&[Value::Nil, Value::Nil, Value::Int(0)])
        });

        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }
}
