use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::rc::Rc;

use crate::effect::{EffectCommand, EffectState};
use crate::{ActionUpdate, ObjectId, ObjectUpdate, QueuedCommand};
use lc_script::{Engine as ScriptEngine, RuntimeError, Value};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

thread_local! {
    static HOST_CONTEXT: RefCell<Option<EffectHostContext>> = const { RefCell::new(None) };
    static RANDOM_CONTEXT: RefCell<Option<Rc<RandomContext>>> = const { RefCell::new(None) };
}

pub fn register_host_functions(script: &mut ScriptEngine) {
    script.register_host_function("AddEffect", add_effect);
    script.register_host_function("RemoveEffect", remove_effect);
    script.register_host_function("GetEffect", get_effect);
    script.register_host_function("GetEffectCount", get_effect_count);
    script.register_host_function("SetAction", set_action);
    script.register_host_function("Random", random);
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct HostObjectContext<'a> {
    pub id: ObjectId,
    pub effects: &'a [EffectState],
}

impl<'a> HostObjectContext<'a> {
    pub fn new(id: ObjectId, effects: &'a [EffectState]) -> Self {
        Self { id, effects }
    }
}

pub(crate) fn with_effect_context<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    global_effects: &[EffectState],
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
        *cell.borrow_mut() = Some(EffectHostContext::new(object, global_effects.to_vec()));
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
}

impl EffectContextOutcome {
    fn new(
        object: Vec<EffectCommand>,
        global: Vec<EffectCommand>,
        object_update: Option<ObjectUpdate>,
        object_commands: Vec<QueuedCommand>,
        destroy_object: bool,
    ) -> Self {
        Self {
            object,
            global,
            object_update,
            object_commands,
            destroy_object,
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            object: Vec::new(),
            global: Vec::new(),
            object_update: None,
            object_commands: Vec::new(),
            destroy_object: false,
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

    if idx < len {
        return Err(RuntimeError::new(
            "AddEffect: additional arguments are not supported",
        ));
    }

    let identifier = with_context_mut(scope, |ctx| {
        let mut effect = EffectState::new(name)
            .with_priority(priority)
            .with_interval(interval);
        if let Some(timer) = timer {
            effect = effect.with_timer(timer);
        }
        effect = effect.with_command_target(command_target);
        effect = effect.with_command_id(command_target_id);
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

    if let Some(flag) = args.get(3) {
        match flag {
            Value::Bool(_) | Value::Nil => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "RemoveEffect: expected bool or nil for no-call flag, got {}",
                    other.type_name()
                )))
            }
        }
    }

    let removed = with_context_mut(scope, |ctx| {
        ctx.remove_effect(name_filter.as_deref(), index)
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

        let update = object
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_name(name);
        if let Some(phase) = phase {
            update.set_phase(phase);
        }
        if let Some(ticks) = ticks {
            update.set_ticks(ticks);
        }

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

                let effect = EffectState::new(name)
                    .with_priority(priority)
                    .with_interval(interval)
                    .with_timer(timer)
                    .with_command_target(command_target)
                    .with_command_id(command_id);
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
    let mut map = HashMap::with_capacity(4);
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
    Value::Proplist(map)
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
}

impl EffectHostContext {
    fn new(object: Option<HostObjectContext<'_>>, global_effects: Vec<EffectState>) -> Self {
        let object = object.map(|ctx| ObjectScopeContext::new(ctx.id, ctx.effects.to_vec()));
        let global = Some(EffectScopeContext::new(global_effects));
        Self { object, global }
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

    fn remove_effect(&mut self, name_filter: Option<&str>, index: usize) -> bool {
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
        self.commands.push(EffectCommand::remove(effect.name));
        true
    }

    fn into_commands(self) -> Vec<EffectCommand> {
        self.commands
    }
}

struct ObjectScopeContext {
    id: ObjectId,
    effects: EffectScopeContext,
    pending_update: ObjectUpdate,
    queued_commands: Vec<QueuedCommand>,
    destroy: bool,
}

impl ObjectScopeContext {
    fn new(id: ObjectId, effects: Vec<EffectState>) -> Self {
        Self {
            id,
            effects: EffectScopeContext::new(effects),
            pending_update: ObjectUpdate::default(),
            queued_commands: Vec::new(),
            destroy: false,
        }
    }

    fn id(&self) -> ObjectId {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_state() -> Value {
        let mut map = HashMap::new();
        map.insert("effects".into(), Value::Array(Vec::new()));
        Value::Proplist(map)
    }

    fn with_object_host_context<F, T>(func: F) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_effect_context(
            Some(HostObjectContext::new(ObjectId::new(1), &[])),
            &[],
            func,
        )
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
        assert!(matches!(outcome.object[1], EffectCommand::Remove { .. }));
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
    fn add_global_effect_records_global_command() {
        let (result, outcome) = with_effect_context(None, &[], || {
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
        let (result, _) = with_effect_context(None, &[], || -> Result<Value, RuntimeError> {
            add_effect(&[Value::String("Glow".into()), Value::Nil, Value::Int(90)])?;
            get_effect(&[
                Value::String("Glow".into()),
                Value::Nil,
                Value::Int(0),
                Value::Int(1),
            ])
        });

        let value = result.expect("GetEffect succeeds");
        assert_eq!(value, Value::String("Glow".into()));
    }

    #[test]
    fn remove_global_effect_handles_missing() {
        let (result, _) = with_effect_context(None, &[], || {
            remove_effect(&[Value::Nil, Value::Nil, Value::Int(0)])
        });

        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }
}
