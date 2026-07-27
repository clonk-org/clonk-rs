use super::*;

/// FnReloadParticle (C4Script.cpp:4992-4996). Particle definitions are static
/// in Rust, so retain the native nullable-string conversion and report the
/// unsupported resource reload as C4ValueInt false.
pub(crate) fn reload_particle(args: &[Value]) -> Result<Value, RuntimeError> {
    let _name = parse_native_c4_string_argument(args.first(), "ReloadParticle", "name")?;
    Ok(Value::Int(0))
}

/// `C4Effect::ClearAll(..., C4FxCall_RemoveClear)` for AssignRemoval.
/// Stop callbacks run from the tail of the effect list while the object is
/// still live; effects added by those callbacks are deleted afterwards
/// without receiving a Stop callback (C4Effect.cpp:407-425).
pub(crate) fn clear_effects_for_assign_removal(target: ObjectId) -> Result<bool, RuntimeError> {
    let effects = with_host_context_mut(Vec::new(), |context| {
        if !context.ensure_object_scope(target) {
            return Vec::new();
        }
        context
            .object_scope(target)
            .map(|scope| {
                scope
                    .effects
                    .snapshot()
                    .into_iter()
                    .filter(|effect| effect.priority != 0)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    for effect in effects.into_iter().rev() {
        let marked_dead = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(scope) = borrow
                .as_mut()
                .and_then(|context| context.object_scope_mut(target))
            else {
                return false;
            };
            let Some(live) = scope
                .effects
                .effects
                .iter_mut()
                .find(|live| live.number == effect.number && live.priority != 0)
            else {
                return false;
            };
            live.priority = 0;
            true
        });
        if marked_dead {
            let function = format!("Fx{}Stop", effect.name);
            let stop_result = dispatch_effect_fx_callback_fail_safe(
                &effect,
                &function,
                &[
                    object_reference_value(target),
                    Value::Int(effect.number),
                    Value::Int(3),
                ],
            );
            if !object_is_present(target) {
                return Ok(false);
            }
            if stop_result == -1 {
                HOST_CONTEXT.with(|cell| {
                    let mut borrow = cell.borrow_mut();
                    let Some(scope) = borrow
                        .as_mut()
                        .and_then(|context| context.object_scope_mut(target))
                    else {
                        return;
                    };
                    if let Some(live) = scope
                        .effects
                        .effects
                        .iter_mut()
                        .find(|live| live.number == effect.number && live.priority == 0)
                    {
                        live.priority = effect.priority;
                    }
                });
            }
        }
    }

    loop {
        let removed = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(scope) = borrow
                .as_mut()
                .and_then(|context| context.object_scope_mut(target))
            else {
                return false;
            };
            let Some(number) = scope.effects.effects.first().map(|effect| effect.number) else {
                return false;
            };
            scope.effects.unlink_effect_by_number(number)
        });
        if !removed {
            break;
        }
    }
    Ok(true)
}

/// `C4Effect::ClearAll(..., C4FxCall_RemoveDeath)` for
/// `C4Object::AssignDeath`. The entry list is frozen before the recursive
/// tail-to-head walk, each node is marked dead before its Stop callback, and
/// a `-1` result restores that exact node. Unlike AssignRemoval, dead nodes
/// stay linked and effects added by Stop callbacks are not part of this walk
/// (C4Effect.cpp:407-425; C4Object.cpp:1164-1174).
pub(crate) fn clear_effects_for_assign_death(target: ObjectId) -> Result<bool, RuntimeError> {
    let effects = with_host_context_mut(Vec::new(), |context| {
        if !context.ensure_object_scope(target) {
            return Vec::new();
        }
        context
            .object_scope(target)
            .map(|scope| {
                scope
                    .effects
                    .snapshot()
                    .into_iter()
                    .filter(|effect| effect.priority != 0)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });

    for effect in effects.into_iter().rev() {
        if !object_is_present(target) {
            break;
        }
        let live_callback = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(scope) = borrow
                .as_mut()
                .and_then(|context| context.object_scope_mut(target))
            else {
                return None;
            };
            let live = scope
                .effects
                .effects
                .iter_mut()
                .find(|live| live.number == effect.number && live.priority != 0)?;
            let previous_priority = live.priority;
            live.priority = 0;
            let callback_effect = live.clone();
            scope
                .effects
                .commands
                .push(EffectCommand::remove_number(effect.number, true));
            Some((previous_priority, callback_effect))
        });
        let Some((previous_priority, callback_effect)) = live_callback else {
            continue;
        };

        let function = format!("Fx{}Stop", callback_effect.name);
        let stop_result = dispatch_effect_fx_callback_fail_safe(
            &callback_effect,
            &function,
            &[
                object_reference_value(target),
                Value::Int(callback_effect.number),
                Value::Int(4),
            ],
        );
        if !object_is_present(target) {
            break;
        }
        if stop_result == -1 {
            HOST_CONTEXT.with(|cell| {
                let mut borrow = cell.borrow_mut();
                let Some(scope) = borrow
                    .as_mut()
                    .and_then(|context| context.object_scope_mut(target))
                else {
                    return;
                };
                let Some(live) = scope
                    .effects
                    .effects
                    .iter_mut()
                    .find(|live| live.number == effect.number && live.priority == 0)
                else {
                    return;
                };
                live.priority = previous_priority;
                scope
                    .effects
                    .commands
                    .push(EffectCommand::update(live.clone()));
            });
        }
    }
    Ok(true)
}

/// Effect-name parameter shared by AddEffect/RemoveEffect/GetEffect/
/// GetEffectCount: C++ declares it `C4String *`, and pre-#strict-3 callers
/// legally pass falsy values that CheckConvertFunctionParameters Set0()s to
/// nil before conversion (C4AulExec.cpp:1370-1374); the empty string also
/// means "match all" (C4Script.cpp:5561). Truthy non-strings throw in C++.
fn effect_name_filter<'a>(
    function: &str,
    value: &'a Value,
) -> Result<Option<&'a str>, RuntimeError> {
    match value {
        Value::String(name) if !name.is_empty() => Ok(Some(name.as_ref())),
        Value::String(_) | Value::Nil | Value::Int(0) | Value::Bool(false) => Ok(None),
        other => Err(RuntimeError::new(format!(
            "{function}: expected string or nil for name, got {}",
            other.type_name()
        ))),
    }
}

/// Runs an effect callback with the fail-safe error policy used by C4Effect's
/// temp/add/stop calls (`fPassErrors=false`). Missing callbacks and script
/// errors both fold to integer zero; the latter are logged.
fn dispatch_effect_fx_callback_fail_safe(
    effect: &EffectState,
    function: &str,
    call_args: &[Value],
) -> i32 {
    match dispatch_effect_fx_callback(
        effect.command_target,
        effect.command_id.as_deref(),
        function,
        call_args,
    ) {
        None => 0,
        Some(Ok(value)) => value_as_i32(&value),
        Some(Err(error)) => {
            tracing::warn!(%error, "script error in {function}; continuing like C++ fail-safe effect dispatch");
            0
        }
    }
}

/// Temp-deactivates an acceptor's active `pNext` effects from high to low,
/// exactly like `C4Effect::TempRemoveUpperEffects` (C4Effect.cpp:473-492).
/// The returned low-to-high list is the matching reactivation order.
fn temp_remove_upper_effects(
    scope: EffectScope,
    target: &Value,
    effects: &[EffectState],
    acceptor_number: i32,
) -> Result<Vec<EffectState>, RuntimeError> {
    let Some(acceptor_index) = effects
        .iter()
        .position(|effect| effect.number == acceptor_number)
    else {
        return Ok(Vec::new());
    };
    if effects[acceptor_index].priority == 1 {
        return Ok(Vec::new());
    }
    let uppers: Vec<EffectState> = effects
        .iter()
        .skip(acceptor_index + 1)
        .filter(|effect| effect.priority > 0 && effect.priority != 1)
        .cloned()
        .collect();
    for upper in uppers.iter().rev() {
        // FlipActive happens before Fx*Stop, so nested effect queries see the
        // temporary negative priority just as they do in C++.
        let flipped = with_context_mut(scope, |ctx| {
            let Some(effect) = ctx
                .effects
                .iter_mut()
                .find(|effect| effect.number == upper.number && effect.priority > 0)
            else {
                return false;
            };
            effect.priority = -effect.priority;
            let updated = effect.clone();
            ctx.commands.push(EffectCommand::update(updated));
            true
        })?;
        if flipped {
            let function = format!("Fx{}Stop", upper.name);
            dispatch_effect_fx_callback_fail_safe(
                upper,
                &function,
                &[
                    target.clone(),
                    Value::Int(upper.number),
                    Value::Int(1),
                    Value::Bool(true),
                ],
            );
        }
    }
    Ok(uppers)
}

fn temp_readd_upper_effects(
    scope: EffectScope,
    target: &Value,
    uppers: &[EffectState],
) -> Result<(), RuntimeError> {
    for upper in uppers {
        // C++ flips the effect active before its temp Start callback.
        let flipped = with_context_mut(scope, |ctx| {
            let Some(effect) = ctx
                .effects
                .iter_mut()
                .find(|effect| effect.number == upper.number && effect.priority < 0)
            else {
                return false;
            };
            effect.priority = -effect.priority;
            let updated = effect.clone();
            ctx.commands.push(EffectCommand::update(updated));
            true
        })?;
        if flipped {
            let function = format!("Fx{}Start", upper.name);
            dispatch_effect_fx_callback_fail_safe(
                upper,
                &function,
                &[target.clone(), Value::Int(upper.number), Value::Int(1)],
            );
        }
    }
    Ok(())
}

/// `C4Effect::Kill` for an effect that was resolved from the live host
/// context (C4Effect.cpp:365-405). The node stays linked and dead while its
/// Stop callback runs, so same-call queries and EffectVar writes see the C++
/// state. An accepted removal is materialized later without dispatching a
/// second Stop; a denial restores this exact node and its callback writes.
fn kill_effect_inline(
    scope: EffectScope,
    target: &Value,
    victim: &EffectState,
) -> Result<(), RuntimeError> {
    let uppers = if victim.priority > 0 {
        let effects = snapshot_effects_from_context(scope).unwrap_or_default();
        temp_remove_upper_effects(scope, target, &effects, victim.number)?
    } else {
        // An inactive victim remains negative while its Start callback runs;
        // this is a distinct reactivation-for-removal call, not the ordinary
        // temp readd (C4Effect.cpp:376-387). The callback may mutate the live
        // node, so Stop below resolves it again afterwards.
        let function = format!("Fx{}Start", victim.name);
        if victim.priority != 1
            && effect_fx_callback_exists(
                victim.command_target,
                victim.command_id.as_deref(),
                &function,
            )
        {
            dispatch_effect_fx_callback_fail_safe(
                victim,
                &function,
                &[target.clone(), Value::Int(victim.number), Value::Int(2)],
            );
        }
        Vec::new()
    };

    let marked_dead = with_context_mut(scope, |ctx| {
        let (previous_priority, updated) = {
            let effect = ctx
                .effects
                .iter_mut()
                .find(|effect| effect.number == victim.number)?;
            let previous_priority = effect.priority;
            effect.priority = 0;
            (previous_priority, effect.clone())
        };
        ctx.commands.push(EffectCommand::update(updated.clone()));
        Some((previous_priority, updated))
    })?;

    if let Some((previous_priority, stopped)) = marked_dead {
        let function = format!("Fx{}Stop", stopped.name);
        let stop_result = dispatch_effect_fx_callback_fail_safe(
            &stopped,
            &function,
            &[target.clone(), Value::Int(stopped.number)],
        );
        if stop_result == -1 {
            with_context_mut(scope, |ctx| {
                let updated = {
                    let Some(effect) = ctx
                        .effects
                        .iter_mut()
                        .find(|effect| effect.number == stopped.number && effect.priority == 0)
                    else {
                        return;
                    };
                    effect.priority = previous_priority;
                    effect.clone()
                };
                // This final update also carries EffectVar writes made by
                // Fx*Stop while the node was dead.
                ctx.commands.push(EffectCommand::update(updated));
            })?;
        } else {
            with_context_mut(scope, |ctx| {
                // Fx*Stop already ran above. Re-fold this exact dead node so
                // callback EffectVar writes survive; Execute unlinks it.
                ctx.remove_effect(None, stopped.number.max(0), true);
            })?;
        }
    }

    temp_readd_upper_effects(scope, target, &uppers)
}

/// `FnCheckEffect` / `C4Effect::Check` (C4Script.cpp:5546-5556;
/// C4Effect.cpp:271-317). Unlike AddEffect, this does not create a pending
/// effect: it asks the selected live list synchronously and returns deny,
/// zero, or the accepting effect number.
pub(crate) fn check_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    check_effect_with_policy(args, true, true)
}

/// Shared `C4Effect::Check` core. Script CheckEffect/AddEffect calls
/// propagate checker errors; native constructors such as Incinerate use
/// C4Effect's default fail-safe policy and already select the target scope
/// directly instead of resolving a script function named CheckEffect.
fn check_effect_with_policy(
    args: &[Value],
    redirect_foreign: bool,
    pass_errors: bool,
) -> Result<Value, RuntimeError> {
    if args.len() > 8 {
        return Err(RuntimeError::new("CheckEffect expects at most 8 arguments"));
    }
    let name = match effect_name_filter("CheckEffect", args.first().unwrap_or(&Value::Nil))? {
        Some(name) => name.to_owned(),
        None => return Ok(Value::Nil),
    };
    let target_state = args.get(1).unwrap_or(&Value::Nil);
    let target_id = object_id_from_value(target_state);

    // A typed object pointer whose Status is zero is rejected before its
    // effect list is inspected (FnCheckEffect's safety guard).
    if let Some(target_id) = target_id {
        let (active, status) = with_host_context((None, None), |context| {
            let active = context.object_context().map(|object| object.id());
            let status = if active == Some(target_id) {
                context.object_context().map(|object| object.status())
            } else {
                context
                    .get_world_object(target_id)
                    .map(|object| object.status())
            };
            (active, status)
        });
        if status.is_none_or(|status| status == ObjectStatus::Deleted) {
            return Ok(Value::Nil);
        }
        if redirect_foreign && active != Some(target_id) {
            return call_world_object_function(target_id, "CheckEffect", args)
                .unwrap_or(Ok(Value::Nil));
        }
    }

    let scope = determine_scope_from_state(target_state)?;
    if matches!(scope, EffectScope::Object(_))
        && !matches!(target_state, Value::Object(_) | Value::Proplist(_))
    {
        return Err(RuntimeError::new(format!(
            "CheckEffect: expected object or proplist for object state, got {}",
            target_state.type_name()
        )));
    }
    let priority = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "CheckEffect",
        "priority",
    )?;
    let interval = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "CheckEffect",
        "interval",
    )?;
    let values: Vec<Value> = (4..8)
        .map(|index| args.get(index).cloned().unwrap_or(Value::Nil))
        .collect();
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object(_) => extract_effects_from_state(target_state)?,
            EffectScope::Global => Vec::new(),
        },
    };
    // FnCheckEffect returns C4VNull when there is no list head. This is
    // observably distinct from C4Effect::Check's successful integer zero.
    let had_list_head = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.effect_list_had_head(scope))
    });
    if !had_list_head.unwrap_or(!effects.is_empty()) {
        return Ok(Value::Nil);
    }
    if priority == 1 {
        return Ok(Value::Int(0));
    }

    let target = target_id.map(object_reference_value).unwrap_or(Value::Nil);
    let mut acceptor: Option<(EffectState, bool)> = None;
    let checker_numbers: Vec<i32> = effects.iter().map(|effect| effect.number).collect();
    for checker_number in checker_numbers {
        // C++ re-tests IsDead and signed priority as it reaches each linked
        // node. An earlier checker may synchronously remove a later one, so
        // never dispatch from the stale entry snapshot.
        let Some(checker) = snapshot_effects_from_context(scope)
            .unwrap_or_default()
            .into_iter()
            .find(|effect| effect.number == checker_number)
        else {
            continue;
        };
        if checker.priority == 0 || checker.priority < priority {
            continue;
        }
        let function = format!("Fx{}Effect", checker.name);
        let mut call_args = vec![
            Value::String(name.clone().into()),
            target.clone(),
            Value::Int(checker.number),
            Value::Nil,
        ];
        call_args.extend(values.iter().cloned());
        let result = if pass_errors {
            match dispatch_effect_fx_callback(
                checker.command_target,
                checker.command_id.as_deref(),
                &function,
                &call_args,
            ) {
                None => 0,
                Some(result) => value_as_i32(&result?),
            }
        } else {
            dispatch_effect_fx_callback_fail_safe(&checker, &function, &call_args)
        };
        match result {
            -1 => return Ok(Value::Int(-1)),
            -2 => acceptor = Some((checker.clone(), false)),
            -3 => acceptor = Some((checker.clone(), true)),
            _ => {}
        }
    }

    let Some((acceptor, do_temp_calls)) = acceptor else {
        return Ok(Value::Int(0));
    };
    let uppers = if do_temp_calls {
        temp_remove_upper_effects(scope, &target, &effects, acceptor.number)?
    } else {
        Vec::new()
    };
    let function = format!("Fx{}Add", acceptor.name);
    let mut add_args = vec![
        target.clone(),
        Value::Int(acceptor.number),
        Value::String(name.into()),
        Value::Int(interval),
    ];
    add_args.extend(values);
    let add_result = dispatch_effect_fx_callback_fail_safe(&acceptor, &function, &add_args);
    if do_temp_calls {
        temp_readd_upper_effects(scope, &target, &uppers)?;
    }

    if add_result == -1 {
        // Fx*Add returning C4Fx_Start_Deny kills the ACCEPTOR and Check
        // reports Annul. The normal Stop call is fail-safe and may deny that
        // removal, exactly like C4Effect::Kill.
        let current = snapshot_effects_from_context(scope).unwrap_or_default();
        let kill_uppers = temp_remove_upper_effects(scope, &target, &current, acceptor.number)?;
        let previous_priority = with_context_mut(scope, |ctx| {
            let effect = ctx
                .effects
                .iter_mut()
                .find(|effect| effect.number == acceptor.number)?;
            let priority = effect.priority;
            effect.priority = 0;
            Some(priority)
        })?;
        let stop_function = format!("Fx{}Stop", acceptor.name);
        let stop_result = dispatch_effect_fx_callback_fail_safe(
            &acceptor,
            &stop_function,
            &[target.clone(), Value::Int(acceptor.number)],
        );
        if let Some(previous_priority) = previous_priority {
            if stop_result == -1 {
                with_context_mut(scope, |ctx| {
                    if let Some(effect) = ctx
                        .effects
                        .iter_mut()
                        .find(|effect| effect.number == acceptor.number)
                    {
                        effect.priority = previous_priority;
                    }
                })?;
            } else {
                with_context_mut(scope, |ctx| {
                    // The Stop callback already ran synchronously above.
                    // Persist the exact dead acceptor; the next Execute
                    // unlinks it without another Stop dispatch.
                    ctx.remove_effect(None, acceptor.number, true);
                })?;
            }
        }
        temp_readd_upper_effects(scope, &target, &kill_uppers)?;
        return Ok(Value::Int(-2));
    }
    Ok(Value::Int(acceptor.number))
}

enum AddEffectCommandIdSlot {
    Native(Option<String>),
    /// The direct Rust fixture DSL predates the fixed C++ ABI and permits a
    /// final integer in slot 5 as the effect's initial timer. Script callers
    /// never take this branch: their slot 5 is always native `C4ID`.
    DirectFixtureTimer(i32),
}

fn parse_add_effect_command_id_slot(
    args: &[Value],
) -> Result<AddEffectCommandIdSlot, RuntimeError> {
    let Some(value) = args.get(5) else {
        return Ok(AddEffectCommandIdSlot::Native(None));
    };
    if args.len() == 6
        && matches!(
            clonk_script::caller_strictness(),
            clonk_script::HostCallerStrictness::NoCaller
        )
        && matches!(value, Value::Int(raw) if *raw != 0)
    {
        let Value::Int(raw) = value else {
            unreachable!("the direct-fixture timer branch checked its integer value")
        };
        return Ok(AddEffectCommandIdSlot::DirectFixtureTimer(
            parse_timer_from_int(*raw)?,
        ));
    }
    Ok(AddEffectCommandIdSlot::Native(parse_native_c4id_argument(
        Some(value),
        "AddEffect",
    )?))
}

pub(crate) fn add_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    // CheckConvertFunctionParameters converts every slot before FnAddEffect
    // executes, even when an empty name or priority zero makes its body
    // return immediately. Validate the C4ID slot before those early exits.
    let command_id_slot = parse_add_effect_command_id_slot(args)?;
    let name = match effect_name_filter("AddEffect", args.first().unwrap_or(&Value::Nil))? {
        Some(name) => name.to_owned(),
        None => return Ok(Value::Int(0)),
    };

    if let Some(result) = redirect_foreign_effect_target("AddEffect", args) {
        return result;
    }

    add_effect_constructor(args, name, true, command_id_slot)
}

/// C4Effect constructor core after the script ABI's foreign-target routing.
/// Native callers use it directly so a script function named AddEffect
/// cannot replace the C++ constructor call, and choose the constructor's
/// `passErrors` policy for the Fx*Effect check chain.
fn add_effect_constructor(
    args: &[Value],
    name: String,
    check_pass_errors: bool,
    command_id_slot: AddEffectCommandIdSlot,
) -> Result<Value, RuntimeError> {
    // An unfilled pTarget slot is nil — a GLOBAL effect (FnAddEffect's
    // C4Object *pTarget = nullptr).
    let target_state = args.get(1).unwrap_or(&Value::Nil);
    let scope = determine_scope_from_state(target_state)?;
    if matches!(scope, EffectScope::Object(_)) {
        match target_state {
            Value::Object(_) | Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "AddEffect: expected object or proplist for object state, got {}",
                    other.type_name()
                )));
            }
        }
    }

    // `if (... || !iPrio) return 0` (C4Script.cpp:5449) — an unfilled
    // priority nil-fills to 0 like C4AulExec, creating NOTHING. The native
    // receives a C4ValueInt, so a C4V_Bool keeps its tag through the CnvOK
    // conversion but `_getInt()` still extracts its shared Data.Int payload.
    let priority = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "AddEffect", "priority")?;

    if priority == 0 {
        return Ok(Value::Int(0));
    }

    // C++ FnAddEffect: unpassed iTimerIntervall is 0 - no timer callbacks
    // (C4Effect.cpp:342).
    let interval = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "AddEffect", "interval")?;

    let len = args.len();
    let mut idx = 4;
    let mut command_target: Option<i32> = None;
    let mut command_target_id: Option<String> = None;
    let mut timer: Option<i32> = None;
    let mut constructor_values: [Value; 4] = std::array::from_fn(|_| Value::Nil);

    if idx < len {
        match &args[idx] {
            Value::Object(_) | Value::Proplist(_) | Value::Nil => {
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
        match command_id_slot {
            AddEffectCommandIdSlot::Native(id) => command_target_id = id,
            AddEffectCommandIdSlot::DirectFixtureTimer(value) => timer = Some(value),
        }
        idx += 1;
    }

    // Slot six is always rVal1 in the C++ native ABI, including an explicit
    // nil before a later non-nil rVal. The legacy direct-fixture timer is
    // encoded in slot five and only exists when this frame ends there.
    for slot in &mut constructor_values {
        if idx >= len {
            break;
        }
        *slot = args[idx].clone();
        idx += 1;
    }

    // C4Effect::AssignCallbackFunctions immediately resolves an object
    // command target and overwrites idCommandTarget with that object's
    // current definition for save/runtime-join safety (C4Effect.cpp:31-57).
    if let Some(target) = command_target {
        if let Some(definition) = HOST_CONTEXT.with(|cell| {
            cell.borrow().as_ref().and_then(|context| {
                context.object_effective_definition_id(ObjectId::new(target as u64))
            })
        }) {
            command_target_id = Some(definition);
        }
    }

    // Priority-1 effects skip C4Effect::Check entirely (C4Effect.cpp:170).
    // Global and live-object additions negotiate and start synchronously,
    // before AddEffect returns (C4Effect.cpp:97-136). Synthetic proplist
    // fixture targets retain the deferred protocol.
    let global_scope = matches!(scope, EffectScope::Global);
    let live_object_scope = matches!(target_state, Value::Object(_));
    let synchronous_start = global_scope || live_object_scope || priority == 1;
    // The engine-internal fire start (FnFxFireStart, AddFunc
    // C4Script.cpp:6994) runs synchronously inside the C4Effect ctor
    // (C4Effect.cpp:118-133) — unless the selected callback script overloads
    // FxFireStart (the generic synchronous dispatch runs the overload), or
    // same/higher-priority effects exist whose Fx*Effect check chain must
    // rule first (C4Effect.cpp:97-116). Live objects run that check above,
    // then invoke the native start immediately.
    let has_checkers = snapshot_effects_from_context(scope)
        .map(|effects| {
            effects
                .iter()
                .any(|existing| existing.priority != 0 && existing.priority >= priority)
        })
        .unwrap_or(false);
    let engine_fire_start = !global_scope
        && name == crate::C4FX_FIRE
        && !effect_script_fx_callback_exists(
            command_target,
            command_target_id.as_deref(),
            "FxFireStart",
        )
        && (live_object_scope || (!synchronous_start && !has_checkers));
    let effect_name = name.clone();
    let call_vars = constructor_values.to_vec();
    let for_object = effect_callback_target_value(scope, target_state);
    let command_id_for_start = command_target_id.clone();
    let identifier = with_context_mut(scope, move |ctx| {
        let mut effect = EffectState::new(name)
            .with_priority(priority)
            .with_interval(interval);
        if let Some(timer) = timer {
            effect = effect.with_timer(timer);
        }
        effect = effect.with_command_target(command_target);
        effect = effect.with_command_id(command_target_id);
        effect.start_dispatched = synchronous_start || engine_fire_start;
        ctx.reserve_effect(effect, constructor_values)
    })?;

    if (global_scope || live_object_scope) && priority != 1 {
        let mut check_args = vec![
            Value::String(effect_name.clone().into()),
            for_object.clone(),
            Value::Int(priority),
            Value::Int(interval),
        ];
        check_args.extend(call_vars.iter().cloned());
        check_args.resize(8, Value::Nil);
        let check_result = match check_effect_with_policy(&check_args, false, check_pass_errors) {
            Ok(result) => result,
            Err(error) => {
                with_context_mut(scope, |ctx| {
                    ctx.abort_reserved_effect(identifier);
                })?;
                return Err(error);
            }
        };
        let stored_number = match check_result {
            Value::Int(0) | Value::Nil => None,
            // C4Effect's pending node remains dead when Check denies it or
            // merges it into an acceptor. FnAddEffect returns zero for a
            // deny, and the acceptor number (or -2 when Fx*Add killed that
            // acceptor) for an annul (C4Effect.cpp:97-115).
            Value::Int(-1) => Some(0),
            Value::Int(result) => Some(result),
            _ => unreachable!("CheckEffect only returns nil or an integer"),
        };
        if let Some(stored_number) = stored_number {
            with_context_mut(scope, |ctx| {
                ctx.discard_reserved_effect(identifier);
            })?;
            return Ok(Value::Int(stored_number));
        }
    }

    let callback = format!("Fx{effect_name}Start");
    let scripted_start = synchronous_start
        && !engine_fire_start
        && effect_fx_callback_exists(command_target, command_id_for_start.as_deref(), &callback);
    let has_start = engine_fire_start || scripted_start;
    let upper_effects = if synchronous_start && priority != 1 && has_start {
        let effects = snapshot_effects_from_context(scope).unwrap_or_default();
        temp_remove_upper_effects(scope, &for_object, &effects, identifier)?
    } else {
        Vec::new()
    };

    // Temp Stop callbacks may delete the carrier. C++ then leaves the
    // pending node invalid, skips Start/readd, and returns stored number 0
    // (C4Effect.cpp:123-126).
    if object_id_from_value(&for_object).is_some_and(|target| !object_is_present(target)) {
        with_context_mut(scope, |ctx| {
            ctx.discard_reserved_effect(identifier);
        })?;
        return Ok(Value::Int(0));
    }
    with_context_mut(scope, |ctx| {
        ctx.validate_reserved_effect(identifier, priority);
    })?;

    let mut start_denied = false;
    let mut start_error = None;
    if engine_fire_start && identifier > 0 {
        // FnFxFireStart parameter mapping: rVal1 = iCausedBy, rVal2 =
        // fBlasted, rVal3 = pIncineratingObject (C4Effect.cpp:560 +
        // pFnStart->Exec args, :129).
        let target = object_id_from_value(&for_object);
        let caused_by = call_vars.first().and_then(Value::as_c4_int).unwrap_or(0);
        let blasted = call_vars
            .get(1)
            .map(extract_cpp_native_bool)
            .unwrap_or(false);
        let incinerating = call_vars.get(2).and_then(object_id_from_value);
        if let Some(target) = target {
            match fire_effect_start_core(target, identifier, caused_by, blasted, incinerating) {
                Ok(-1) => start_denied = true,
                Ok(_) => {}
                Err(error) => start_error = Some(error),
            }
        }
    } else if scripted_start && identifier > 0 {
        let mut call_args = vec![for_object.clone(), Value::Int(identifier), Value::Int(0)];
        call_args.extend(call_vars);
        call_args.resize(7, Value::Nil);
        // pFnStart->Exec(pCommandTarget, {C4VObj(pForObj), C4VInt(iNumber),
        // C4VInt(0), rVal1..rVal4}, ...) (C4Effect.cpp:128-129); GLOBAL
        // effects resolve like C4Effect::DoCall — command target script,
        // command-id def script, else the engine-global function table
        // (C4Effect.cpp:439-456 via AssignCallbackFunctions :42-57).
        if let Some(result) = dispatch_effect_fx_callback(
            command_target,
            command_id_for_start.as_deref(),
            &callback,
            &call_args,
        ) {
            match result {
                Ok(value) if value_as_i32(&value) == -1 => start_denied = true,
                Ok(_) => {}
                Err(error) => start_error = Some(error),
            }
        }
    }
    if let Some(error) = start_error {
        // Constructor exception unwind unlinks only the new node and does
        // not reactivate the temporarily stopped upper effects.
        with_context_mut(scope, |ctx| {
            ctx.abort_reserved_effect(identifier);
        })?;
        return Err(error);
    }
    if start_denied {
        // C4Fx_Start_Deny marks the new node dead without a Stop callback;
        // FnAddEffect still returns its allocated number after reactivating
        // upper effects (C4Effect.cpp:128-136).
        with_context_mut(scope, |ctx| {
            ctx.discard_reserved_effect(identifier);
        })?;
    }
    temp_readd_upper_effects(scope, &for_object, &upper_effects)?;
    if object_id_from_value(&for_object).is_some_and(|target| !object_is_present(target)) {
        return Ok(Value::Int(0));
    }

    Ok(Value::Int(identifier))
}

pub(crate) fn remove_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(result) = redirect_foreign_effect_target("RemoveEffect", args) {
        return result;
    }
    let name_filter =
        effect_name_filter("RemoveEffect", args.first().unwrap_or(&Value::Nil))?.map(str::to_owned);

    let target_state = args.get(1).unwrap_or(&Value::Nil);
    let scope = determine_scope_from_state(target_state)?;
    if matches!(scope, EffectScope::Object(_)) {
        match target_state {
            Value::Object(_) | Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "RemoveEffect: expected object or proplist for object state, got {}",
                    other.type_name()
                )));
            }
        }
    }

    let index = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "RemoveEffect", "index")?;

    // `bool fDoNoCalls` (FnRemoveEffect, C4Script.cpp:5493): C4Value
    // converts ints freely - CR content passes 1 (the Talker's movie
    // timer).
    let no_callbacks = value_to_bool(
        args.get(3).unwrap_or(&Value::Nil),
        "RemoveEffect",
        "no-call flag",
    )?;

    // Real object and global effect lists execute C4Effect::Kill inside the
    // host call. Synthetic proplist fixtures retain the deferred command
    // protocol because they have no live callback/world context.
    let synchronous_kill =
        matches!(scope, EffectScope::Global) || matches!(target_state, Value::Object(_));
    if !no_callbacks && synchronous_kill {
        let victim = with_context_mut(scope, |ctx| {
            ctx.find_live_effect(name_filter.as_deref(), index)
        })?;
        let Some(victim) = victim else {
            return Ok(Value::Bool(false));
        };
        let target = match scope {
            EffectScope::Object(_) => target_state.clone(),
            EffectScope::Global => Value::Nil,
        };
        kill_effect_inline(scope, &target, &victim)?;
        return Ok(Value::Bool(true));
    }

    let removed = with_context_mut(scope, |ctx| {
        ctx.remove_live_effect(name_filter.as_deref(), index, no_callbacks)
    })?;
    // Synthetic proplist fixtures have no callback script, but preserve the
    // old engine-Fire projection. fDoNoCalls skips the entire Kill bracket.
    if matches!(scope, EffectScope::Object(_))
        && !no_callbacks
        && removed
            .as_ref()
            .is_some_and(|effect| effect.name == crate::C4FX_FIRE)
        && !script_shadows_engine_fx("FxFireStop")
    {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                if let Some(object) = context.object_context_mut() {
                    object.pending_update.stage_fire_flag(false);
                }
            }
        });
    }
    Ok(Value::Bool(removed.is_some()))
}

pub(crate) fn change_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(result) = redirect_foreign_effect_target("ChangeEffect", args) {
        return result.map(|value| match value {
            // The shared foreign-effect seam uses integer zero when the
            // target vanished. FnChangeEffect's declared return is bool.
            Value::Int(0) => Value::Bool(false),
            other => other,
        });
    }

    let name_filter =
        effect_name_filter("ChangeEffect", args.first().unwrap_or(&Value::Nil))?.map(str::to_owned);
    let scope = determine_scope_from_state(args.get(1).unwrap_or(&Value::Nil))?;
    if matches!(scope, EffectScope::Object(_)) {
        match args.get(1).unwrap_or(&Value::Nil) {
            Value::Object(_) | Value::Proplist(_) => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "ChangeEffect: expected object or proplist for object state, got {}",
                    other.type_name()
                )));
            }
        }
    }

    let index = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "ChangeEffect", "index")?;
    let Some(new_name) = effect_name_filter("ChangeEffect", args.get(3).unwrap_or(&Value::Nil))?
    else {
        return Ok(Value::Bool(false));
    };
    let new_name = truncate_c4_max_name(new_name);
    let new_timer = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "ChangeEffect",
        "new timer",
    )?;

    let changed = with_context_mut(scope, |ctx| {
        ctx.change_effect(name_filter.as_deref(), index, new_name, new_timer)
    })?;
    Ok(Value::Bool(changed))
}

pub(crate) fn get_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(result) = redirect_foreign_effect_target("GetEffect", args) {
        return result;
    }
    let name_filter = effect_name_filter("GetEffect", args.first().unwrap_or(&Value::Nil))?;

    let scope = determine_scope_from_state(args.get(1).unwrap_or(&Value::Nil))?;
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object(_) => extract_effects_from_state(&args[1])?,
            EffectScope::Global => Vec::new(),
        },
    };

    let desired_index = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "GetEffect", "index")?;

    let query = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "GetEffect", "query")?;

    let max_priority = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "GetEffect",
        "max priority",
    )?;

    let found = match name_filter {
        // Name/wildcard given: find by name and index
        // (C4Script.cpp:5471-5472 -> C4Effect::Get(szName, iIndex,...),
        // wildcard compare at C4Effect.cpp:229).
        Some(filter) => usize::try_from(desired_index).ok().and_then(|index| {
            effects
                .iter()
                .filter(|effect| effect.priority != 0)
                .filter(|effect| s_wildcard_match_ex(&effect.name, filter))
                .filter(|effect| max_priority == 0 || effect.priority <= max_priority)
                .nth(index)
        }),
        // No name: iIndex is the effect NUMBER (C4Script.cpp:5474-5475 ->
        // C4Effect::Get(iNumber, fIncludeDead=true), C4Effect.cpp:240-256).
        None => effects
            .iter()
            .find(|effect| effect.number == desired_index)
            .filter(|effect| max_priority == 0 || effect.priority <= max_priority),
    };

    Ok(found
        .map(|effect| match query {
            // 0: number (C4Script.cpp:5481 `C4VInt(pEffect->iNumber)`)
            0 => Value::Int(effect.number),
            1 => Value::String(effect.name.clone().into()),
            2 => Value::Int(effect.priority.abs()),
            3 => Value::Int(effect.interval),
            4 => effect
                .command_target
                .map(|target| object_reference_value(ObjectId::new(target as u64)))
                .unwrap_or(Value::Nil),
            5 => {
                let live_id = effect.command_target.and_then(|target| {
                    HOST_CONTEXT.with(|cell| {
                        cell.borrow().as_ref().and_then(|context| {
                            context.object_effective_definition_id(ObjectId::new(target as u64))
                        })
                    })
                });
                live_id
                    .or_else(|| effect.command_id.clone())
                    .map(Value::C4Id)
                    .unwrap_or(Value::Nil)
            }
            6 => Value::Int(effect.timer),
            _ => Value::Nil,
        })
        .unwrap_or(Value::Nil))
}

pub(crate) fn get_effect_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(result) = redirect_foreign_effect_target("GetEffectCount", args) {
        return result;
    }
    let name_filter = effect_name_filter("GetEffectCount", args.first().unwrap_or(&Value::Nil))?;

    let scope = determine_scope_from_state(args.get(1).unwrap_or(&Value::Nil))?;
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object(_) => extract_effects_from_state(&args[1])?,
            EffectScope::Global => Vec::new(),
        },
    };

    let max_priority = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "GetEffectCount",
        "max priority",
    )?;

    let count = effects
        .iter()
        .filter(|effect| effect.priority != 0)
        .filter(|effect| {
            if let Some(filter) = name_filter {
                // C4Effect::GetCount wildcard-compares names (C4Effect.cpp:263).
                if !s_wildcard_match_ex(&effect.name, filter) {
                    return false;
                }
            }
            if max_priority != 0 && effect.priority > max_priority {
                return false;
            }
            true
        })
        .count();

    let count = i32::try_from(count).unwrap_or(i32::MAX);
    Ok(Value::Int(count))
}

pub(crate) fn effect_var(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnEffectVar reads/writes the effect list of the GIVEN object directly
    // (C4Script.cpp:5576-5586). A foreign carrier therefore uses its
    // materialized object scope; no script redispatch may intercept this
    // already-resolved native.
    // Unfilled iVarIndex is nil -> 0 (FnEffectVar, C4Script.cpp:5577).
    let var_index = value_to_i32(args.first().unwrap_or(&Value::Nil), "EffectVar", "index")?;
    let Ok(var_index) = usize::try_from(var_index) else {
        return Ok(Value::Nil);
    };

    let scope = determine_scope_from_state(args.get(1).unwrap_or(&Value::Nil))?;

    let effect_number = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "EffectVar", "number")?;
    let Ok(effect_number) = usize::try_from(effect_number) else {
        return Ok(Value::Nil);
    };
    if effect_number == 0 {
        return Ok(Value::Nil);
    }

    let new_value = args.get(3).map(value_to_effect_var);

    let context_value = with_host_context_mut(Ok(None), |context| {
        // The VM's retained-lvalue bridge supplies a private fourth write
        // value after resolving the public three-parameter native. Re-entering
        // a foreign object's ScriptEngine would correctly normalize a public
        // call back to three slots and therefore discard that private value.
        // C++ FnEffectVar writes pObj->pEffects directly, so materialize the
        // foreign scope and perform the same direct write here.
        if let EffectScope::Object(Some(target)) = scope {
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
        }

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

    // FnEffectVar resolves the effect BY NUMBER (C4Effect::Get(iNumber,
    // true), C4Script.cpp:5583); snapshot proplists carry positional
    // stand-in numbers from extract_effects_from_state.
    let effects = extract_effects_from_state(args.get(1).unwrap_or(&Value::Nil))?;
    let number = i32::try_from(effect_number).unwrap_or(i32::MAX);
    Ok(effects
        .iter()
        .find(|effect| effect.number == number)
        .map(|effect| effect_var_to_value(&effect.var(var_index)))
        .unwrap_or(Value::Nil))
}

/// FnEffectCall (C4Script.cpp:5589-5601): `EffectCall(pTarget, iNumber,
/// szCallFn, vVal1..vVal7)` finds the effect BY NUMBER on the target (dead
/// included, `C4Effect::Get(iNumber, true)`, C4Effect.cpp:240-256) and runs
/// `Fx<EffectName><CallFn>` (PSF_FxCustom, C4Script.h:113) through
/// `C4Effect::DoCall` (C4Effect.cpp:439-456): the effect's command target is
/// the call context and its def script the resolution scope (global script
/// functions fall back via GetFuncRecursive); without a live target the
/// command id's def script is used, else Game.ScriptEngine's globals. The
/// callback receives `(pTarget, iNumber, vVal1..vVal7)`; passErrors=true —
/// callback errors abort the calling script like C++.
pub(crate) fn effect_call(args: &[Value]) -> Result<Value, RuntimeError> {
    // Foreign target: run in the target's own scope like the other effect
    // host functions (the C4Effect list lives on the GIVEN object).
    let target = args.first().unwrap_or(&Value::Nil);
    if let Some(foreign) = object_id_from_value(target) {
        let active = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_context().map(|object| object.id()))
        });
        if Some(foreign) != active {
            // A vanished/dead target is the FnEffectCall status guard
            // (C4Script.cpp:5593): silent C4VNull.
            return call_world_object_function(foreign, "EffectCall", args)
                .unwrap_or(Ok(Value::Nil));
        }
    }

    // `if (!szCallFn || !*szCallFn) return C4VNull;` (C4Script.cpp:5594) —
    // the same falsy-name conversion the effect name filters use.
    let call_fn = match effect_name_filter("EffectCall", args.get(2).unwrap_or(&Value::Nil))? {
        Some(name) => name.to_owned(),
        None => return Ok(Value::Nil),
    };

    let number = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "EffectCall",
        "effect number",
    )?;

    let scope = determine_scope_from_state(target)?;
    let effects = match snapshot_effects_from_context(scope) {
        Some(effects) => effects,
        None => match scope {
            EffectScope::Object(_) => extract_effects_from_state(target)?,
            EffectScope::Global => Vec::new(),
        },
    };
    let Some(effect) = effects.iter().find(|effect| effect.number == number) else {
        return Ok(Value::Nil);
    };

    let function = format!("Fx{}{}", effect.name, call_fn);
    // DoCall argument layout (C4Effect.cpp:455): pObj, iNumber, then the
    // seven forwarded values.
    let mut call_args = Vec::with_capacity(9);
    call_args.push(match scope {
        EffectScope::Object(_) => target.clone(),
        EffectScope::Global => Value::Nil,
    });
    call_args.push(Value::Int(number));
    call_args.extend(args.iter().skip(3).take(7).cloned());
    call_args.resize(9, Value::Nil);

    dispatch_effect_fx_callback(
        effect.command_target,
        effect.command_id.as_deref(),
        &function,
        &call_args,
    )
    .unwrap_or(Ok(Value::Nil))
}

/// Whether the selected callback script supplies a script implementation
/// before the engine-native Fx* fallback.
fn effect_script_fx_callback_exists(
    command_target: Option<i32>,
    command_id: Option<&str>,
    function: &str,
) -> bool {
    with_host_context(false, |context| {
        if let Some(command_target) = command_target {
            let target = ObjectId::new(command_target as u64);
            if context.has_callable_object(target) {
                return context
                    .object_effective_definition_id(target)
                    .and_then(|id| context.world.definition_script(&id))
                    .is_some_and(|script| script.has_function_or_global(function));
            }
        }
        if let Some(script) = command_id
            .and_then(definition_id_for_c4id)
            .and_then(|id| context.world.definition_script(&id))
        {
            return script.has_function_or_global(function);
        }
        context
            .world
            .resolve_engine_global_script(function)
            .is_some()
    })
}

/// Whether C4Effect::AssignCallbackFunctions resolves this callback through
/// the command object, command definition, or global script engine. Start's
/// upper-effect temp bracket exists only when pFnStart is non-null.
fn effect_fx_callback_exists(
    command_target: Option<i32>,
    command_id: Option<&str>,
    function: &str,
) -> bool {
    with_host_context(false, |context| {
        if let Some(command_target) = command_target {
            let target = ObjectId::new(command_target as u64);
            if context.has_callable_object(target) {
                return context
                    .object_effective_definition_id(target)
                    .and_then(|id| context.world.definition_script(&id))
                    .is_some_and(|script| {
                        script.has_function_or_global(function)
                            || script.has_host_function(function)
                    });
            }
        }
        if let Some(script) = command_id
            .and_then(definition_id_for_c4id)
            .and_then(|id| context.world.definition_script(&id))
        {
            return script.has_function_or_global(function) || script.has_host_function(function);
        }
        context
            .world
            .resolve_engine_global_script(function)
            .is_some()
            || context.world.resolve_engine_host_script(function).is_some()
    })
}

/// Resolves and executes an Fx callback like C4Effect::DoCall
/// (C4Effect.cpp:439-456): the effect's command target's own script, else
/// the command id's def script (Obj=nullptr, GetFuncRecursive reaches
/// globals), else the engine-global function table. `None` when the
/// function exists nowhere (pFn nullptr) — the caller decides whether
/// that means C4Value() or "leave a chained value untouched".
fn dispatch_effect_fx_callback(
    command_target: Option<i32>,
    command_id: Option<&str>,
    function: &str,
    call_args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    if let Some(command_target) = command_target {
        // pFn->Exec(pCommandTarget, ...) — the command target is `this`
        // (C4Effect.cpp:443-445,456). GetFuncRecursive also reaches native
        // engine functions (notably FxFireInfo).
        let command_target = ObjectId::new(command_target as u64);
        let (target_exists, resolution) = with_host_context((false, None), |context| {
            let target_exists = context.has_callable_object(command_target);
            let resolution = target_exists
                .then(|| context.object_effective_definition_id(command_target))
                .flatten()
                .and_then(|id| context.world.definition_script(&id))
                .and_then(|script| script.resolve_function(function, true));
            (target_exists, resolution)
        });
        if target_exists {
            if let Some(resolution) = resolution
                .filter(|resolution| resolution.scope == clonk_script::ScriptFunctionScope::Global)
            {
                let exact_script = HOST_CONTEXT.with(|cell| {
                    cell.borrow().as_ref().and_then(|context| {
                        context
                            .world
                            .script_for_host_identity(resolution.host_identity)
                            .map(|(_, _, script)| script)
                    })
                });
                return exact_script.and_then(|script| {
                    call_world_object_resolved_global_function(
                        command_target,
                        script,
                        resolution,
                        function,
                        call_args,
                    )
                });
            }
            return call_world_object_function_inflight(command_target, function, call_args);
        }
    }
    let definition_script = command_id.and_then(definition_id_for_c4id).and_then(|id| {
        HOST_CONTEXT.with(|cell| {
            cell.borrow().as_ref().and_then(|context| {
                context
                    .world
                    .definition_script(&id)
                    .cloned()
                    .map(|script| (id, script))
            })
        })
    });
    if let Some((definition, script)) = definition_script {
        // idCommandTarget resolves the def script with Obj=nullptr
        // (C4Effect.cpp:446-447); GetFuncRecursive reaches globals.
        let resolution = script.resolve_function(function, true);
        if let Some(resolution) = resolution
            .as_ref()
            .filter(|resolution| resolution.scope == clonk_script::ScriptFunctionScope::Global)
        {
            let exact_script = HOST_CONTEXT.with(|cell| {
                cell.borrow().as_ref().and_then(|context| {
                    context
                        .world
                        .script_for_host_identity(resolution.host_identity)
                        .map(|(_, _, script)| script)
                })
            });
            return exact_script.and_then(|script| {
                call_scoped_global_effect_function(script, function, call_args)
            });
        }
        let local_definition = resolution
            .filter(|resolution| resolution.scope == clonk_script::ScriptFunctionScope::Local)
            .map(|_| definition);
        return call_scoped_effect_function_or_global(
            script,
            local_definition,
            function,
            call_args,
        );
    }
    // No command target at all: Game.ScriptEngine — resolve the retained
    // GLOBAL function's exact LinkedTo host, then native engine functions.
    let global_carrier = HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().and_then(|context| {
            context
                .world
                .resolve_engine_global_script(function)
                .map(|(script, _)| script)
                .or_else(|| context.world.resolve_engine_host_script(function))
        })
    });
    global_carrier
        .and_then(|script| call_scoped_global_effect_function(script, function, call_args))
}

/// `C4Object::GetInfoString` effect suffix: query every attached effect in
/// list order through fail-safe `Fx<Name>Info(target, number)` dispatch.
/// Callback side effects remain in the surrounding host context and are
/// folded by the engine after this returns.
pub(crate) fn object_effect_info_lines(target: ObjectId, effects: &[EffectState]) -> Vec<String> {
    let target_value = object_reference_value(target);
    let mut lines = Vec::new();
    for effect in effects {
        let function = format!("Fx{}Info", effect.name);
        let call_args = [target_value.clone(), Value::Int(effect.number)];
        let Some(result) = dispatch_effect_fx_callback(
            effect.command_target,
            effect.command_id.as_deref(),
            &function,
            &call_args,
        ) else {
            continue;
        };
        match result {
            Ok(Value::String(line)) if !line.is_empty() => lines.push(line.into_string()),
            Ok(value) if !value.as_bool() => {}
            Ok(value) => tracing::warn!(
                effect = effect.name,
                number = effect.number,
                returned = value.type_name(),
                "effect Info callback returned a non-string value"
            ),
            Err(error) => tracing::warn!(
                %error,
                effect = effect.name,
                number = effect.number,
                "script error in effect Info callback; continuing like C++ fail-safe dispatch"
            ),
        }
    }
    lines
}

/// C4Effect::DoDamage on the host seam (C4Effect.cpp:427-437): walks the
/// target's live effects asking each Fx*Damage hook (on the effect's
/// command target) to chain the damage value. The C++ walk is a do-while:
/// the FIRST effect is asked even for a zero change; afterwards the chain
/// continues only while the value stays nonzero, and a target removal
/// mid-chain aborts (`if (pObj && !pObj->Status) return`). A hook the
/// dispatch script does not define leaves the value untouched (the
/// pFnDamage existence gate); an erroring hook folds to 0 like an
/// fPassErrors=false Exec. `None` when the target has no effects at all
/// (pEffects nullptr — the caller must not early-return then).
pub(crate) fn dispatch_effects_do_damage(
    target: ObjectId,
    mut change: i32,
    cause: i32,
    caused_by: i32,
) -> Option<i32> {
    let mut current_number = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(target) {
            return None;
        }
        context
            .object_scope(target)?
            .effects
            .effects
            .first()
            .map(|effect| effect.number)
    });
    current_number?;

    let target_value = object_reference_value(target);
    while let Some(number) = current_number {
        // C4Effect::DoDamage walks live nodes: an earlier callback may mark a
        // later node dead, ChangeEffect may replace its callback name, and
        // AddEffect may insert a new successor before the walk advances.
        let live_effect = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_scope(target))
                .and_then(|scope| {
                    scope
                        .effects
                        .effects
                        .iter()
                        .find(|live| live.number == number)
                        .cloned()
                })
        });
        // IsDead: a zero priority marks a dead effect (C4Effect.h).
        if let Some(effect) = live_effect.filter(|effect| effect.priority != 0) {
            let function = format!("Fx{}Damage", effect.name);
            let call_args = [
                target_value.clone(),
                Value::Int(effect.number),
                Value::Int(change),
                Value::Int(cause),
                Value::Int(caused_by),
            ];
            match dispatch_effect_fx_callback(
                effect.command_target,
                effect.command_id.as_deref(),
                &function,
                &call_args,
            ) {
                // No such hook anywhere: the chained value stays
                // (pFnDamage existence gate, C4Effect.cpp:433).
                None => {}
                // C4Value::getInt() reads non-canonical Bool payloads too.
                Some(Ok(value)) => change = value_as_i32(&value),
                Some(Err(error)) => {
                    tracing::warn!(
                        %error,
                        "script error in {function}; the chained damage folds to 0"
                    );
                    change = 0;
                }
            }
        }
        // `if (pObj && !pObj->Status) return` (C4Effect.cpp:435).
        let target_gone = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .is_none_or(|context| !context.object_status_present(target))
        });
        if target_gone || change == 0 {
            break;
        }

        // This is the linked-list `pEff = pEff->pNext` performed after the
        // callback in C++. Looking up the successor now (rather than freezing
        // the entry list) visits additions after the current node and does not
        // restart at additions inserted before it.
        current_number = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_scope(target))
                .and_then(|scope| {
                    let position = scope
                        .effects
                        .effects
                        .iter()
                        .position(|effect| effect.number == number)?;
                    scope
                        .effects
                        .effects
                        .get(position + 1)
                        .map(|effect| effect.number)
                })
        });
    }
    Some(change)
}

/// FnExplode (C4Script.cpp:238-243) snapshots the immediate container and
/// controller, removes the target, then lets C4Object::Explode read its final
/// position while the removed object is still allocated.
pub(crate) fn explode(args: &[Value]) -> Result<Value, RuntimeError> {
    let level = value_to_i32(args.first().unwrap_or(&Value::Nil), "Explode", "level")?;
    let explicit_target =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "Explode", "object")?;
    let effect_id = parse_native_c4id_argument(args.get(2), "Explode")?;
    // FnStringPar always passes a non-null pointer to Explosion: a null C4String
    // becomes "". Consequently an omitted string does not activate
    // Explosion's `else if (idEffect)` particle-suppression arm.
    let effect_name =
        parse_native_c4_string_argument(args.get(3), "Explode", "effect")?.unwrap_or_default();

    let target = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| explicit_target.or(context.script_object_context))
    });
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };

    let pre_removal = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let object = context.get_world_object(target)?;
        let scope = context.object_scope(target);
        Some((
            scope
                .map(ObjectScopeContext::container)
                .unwrap_or_else(|| object.container()),
            scope
                .map(ObjectScopeContext::controller)
                .unwrap_or_else(|| object.controller()),
        ))
    });
    let Some((in_object, caused_by)) = pre_removal else {
        return Ok(Value::Bool(false));
    };

    let _ = assign_removal_live(target, false)?;
    let position = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        context
            .object_scope(target)
            .map(ObjectScopeContext::effective_position)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.position())
            })
    });
    // A resolved C4Object pointer remains allocated after AssignRemoval. The
    // Rust scope normally remains as well; retain FnExplode's unconditional
    // true result if a synthetic fixture cannot expose that retired scope.
    if let Some(position) = position {
        native_explosion(
            position,
            level,
            in_object,
            caused_by,
            target,
            effect_id,
            &effect_name,
        )?;
    }
    Ok(Value::Bool(true))
}

fn native_explosion(
    position: Vector2,
    level: i32,
    in_object: Option<ObjectId>,
    caused_by: i32,
    by_object: ObjectId,
    effect_id: Option<String>,
    effect_name: &str,
) -> Result<(), RuntimeError> {
    let grade = (level / 10 - 1).clamp(1, 3);
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            // The current C++ oracle's std::format receives the promoted int
            // value of '0' + grade, hence Blast49..Blast51.
            context.audio_mut().events.push(AudioCommand::PlaySoundAt {
                name: format!("Blast{}", i32::from(b'0') + grade),
                position,
            });
        }
    });

    // Resolve containment once, before any visual-effect callbacks, and keep
    // that pointer for the later second BlastObjects pass.
    let contain_blast = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let mut container = in_object;
        while let Some(candidate) = container {
            let definition = effective_definition_id(context, candidate);
            let contains = definition
                .as_deref()
                .and_then(|definition| context.definition_metadata(definition))
                .is_some_and(|metadata| metadata.fire.contain_blast != 0);
            if contains {
                break;
            }
            container = context
                .object_scope(candidate)
                .map(ObjectScopeContext::container)
                .unwrap_or_else(|| {
                    context
                        .get_world_object(candidate)
                        .and_then(|object| object.container())
                });
        }
        container
    });

    if contain_blast.is_none() {
        if !incinerate_landscape_at(position.x, position.y)?.as_bool()
            && !incinerate_landscape_at(position.x, position.y.wrapping_sub(10))?.as_bool()
            && !incinerate_landscape_at(position.x.wrapping_sub(5), position.y.wrapping_sub(5))?
                .as_bool()
        {
            let _ =
                incinerate_landscape_at(position.x.wrapping_add(5), position.y.wrapping_sub(5))?;
        }
        native_explosion_visual(
            position,
            level,
            caused_by,
            by_object,
            effect_id,
            effect_name,
        )?;
    }

    native_blast_objects(
        position.x,
        position.y,
        level,
        in_object,
        caused_by,
        Some(by_object),
    )?;
    if contain_blast != in_object {
        native_blast_objects(
            position.x,
            position.y,
            level,
            contain_blast,
            caused_by,
            Some(by_object),
        )?;
    }
    if contain_blast.is_none() {
        native_blast_free_absolute(position, level, Some(caused_by))?;
    }
    Ok(())
}

fn native_explosion_visual(
    position: Vector2,
    level: i32,
    caused_by: i32,
    by_object: ObjectId,
    effect_id: Option<String>,
    effect_name: &str,
) -> Result<(), RuntimeError> {
    let selected_particle = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let default =
            (context.particle_def_known("Blast") != Some(false)).then(|| "Blast".to_string());
        if !effect_name.is_empty() && context.particle_def_known(effect_name) != Some(false) {
            Some(effect_name.to_string())
        } else {
            default
        }
    });

    if let Some(definition_id) = selected_particle {
        with_host_context_mut((), |context| {
            context.register_particle(ParticleCommand::Create(ParticleConfig {
                definition_id: definition_id.clone(),
                position: FloatVector2::new(position.x as f32, position.y as f32),
                velocity: FloatVector2::new(0.0, 0.0),
                life: 0,
                parameter_a: level as f32,
                parameter_b: 0,
                layer: ParticleLayer::Global,
            }));
            if definition_id.starts_with("Blast")
                && context.particle_def_known("FSpark") != Some(false)
            {
                context.register_particle(ParticleCommand::Cast {
                    definition_id: "FSpark".to_string(),
                    amount: level / 5 + 1,
                    x: position.x as f32,
                    y: position.y as f32,
                    level,
                    a0: (level / 2) as f32 + 1.0,
                    b0: 0x00ef_0000,
                    a1: level as f32 + 1.0,
                    b1: 0xffff_1010,
                    layer: ParticleLayer::Global,
                });
            }
        });
        return Ok(());
    }

    let definition = effect_id.unwrap_or_else(|| "FXB1".to_string());
    let controller = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        context
            .object_scope(by_object)
            .map(ObjectScopeContext::controller)
            .or_else(|| {
                context
                    .get_world_object(by_object)
                    .map(|object| object.controller())
            })
    });
    let Some(controller) = controller else {
        return Ok(());
    };
    if let Some(effect) = create_native_object(NativeObjectCreation {
        definition,
        creator: Some(by_object),
        owner: caused_by,
        controller,
        construction: level.wrapping_mul(FULL_CON) / 20,
        position: Vector2::new(position.x, position.y.wrapping_add(level)),
        rotation: 0,
        velocity: FixedVec2::ZERO,
        rotation_velocity: C4Fixed::ZERO,
    })? {
        call_object_own_fail_safe(effect, "Activate", &[]);
    }
    Ok(())
}

/// FnBlastObjects (C4Script.cpp:2269-2273) -> C4Game::BlastObjects
/// (C4Game.cpp:1248-1296). Coordinates are already global. The calling
/// object's layer restricts both contained children and outside victims;
/// a null calling object selects the null layer.
pub(crate) fn blast_objects(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "BlastObjects", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "BlastObjects", "y")?;
    let level = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "BlastObjects", "level")?;
    let in_object = parse_object_reference_argument(
        args.get(3).unwrap_or(&Value::Nil),
        "BlastObjects",
        "container",
    )?;
    let caused_by_plus_one = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "BlastObjects",
        "caused by",
    )?;

    // Resolve both values before any blast callback. FnBlastObjects passes
    // cthr->Obj as pByObj, and C4Game immediately replaces it with pLayer.
    let (caused_by, by_object) = try_with_host_context(
        "BlastObjects requires an active engine context",
        |context| {
            let caller = context.script_object_context;
            let caused_by = if caused_by_plus_one != 0 {
                caused_by_plus_one.wrapping_sub(1)
            } else {
                caller
                    .and_then(|caller| {
                        context
                            .object_scope(caller)
                            .map(ObjectScopeContext::controller)
                            .or_else(|| {
                                context
                                    .get_world_object(caller)
                                    .map(|object| object.controller())
                            })
                    })
                    .unwrap_or(OWNER_NONE)
            };
            Ok::<_, RuntimeError>((caused_by, caller))
        },
    )?;

    native_blast_objects(x, y, level, in_object, caused_by, by_object)?;
    Ok(Value::Nil)
}

/// Absolute `C4Game::BlastObjects` entry used both by the script wrapper and
/// `C4Object::Explode`. The latter must retain the removed source object's
/// layer instead of inheriting the currently executing caller's layer. The
/// layer is read at each invocation, matching C4Game's live `pByObj->pLayer`.
fn native_blast_objects(
    x: i32,
    y: i32,
    level: i32,
    in_object: Option<ObjectId>,
    caused_by: i32,
    by_object: Option<ObjectId>,
) -> Result<(), RuntimeError> {
    let blast_layer = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        by_object.and_then(|object| context.object_layer(object))
    });

    // Blast the container before obtaining Objects.First: its callbacks can
    // change the children that the subsequent master-list scan observes.
    if let Some(container) = in_object {
        let _ = native_blast_object(container, level, caused_by)?;
        let ids = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(EffectHostContext::master_object_ids)
                .unwrap_or_default()
        });
        for id in ids {
            let eligible = with_host_context(false, |context| {
                let Some(object) = context.get_world_object(id) else {
                    return false;
                };
                let current_container = context
                    .object_scope(id)
                    .map(ObjectScopeContext::container)
                    .unwrap_or_else(|| object.container());
                context.object_status_active(id)
                    && current_container == Some(container)
                    && context.object_layer(id) == blast_layer
            });
            if eligible {
                let _ = native_blast_object(id, level, caused_by)?;
            }
        }
        return Ok(());
    }

    let ids = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(EffectHostContext::master_object_ids)
            .unwrap_or_default()
    });
    for id in ids {
        // Status/containment/layer are the outer C++ if-chain and are tested
        // once. A direct Blast callback may mutate later shockwave inputs.
        let direct_hit = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let object = context.get_world_object(id)?;
            let container = context
                .object_scope(id)
                .map(ObjectScopeContext::container)
                .unwrap_or_else(|| object.container());
            if !context.object_status_active(id)
                || container.is_some()
                || context.object_layer(id) != blast_layer
            {
                return None;
            }
            let position = context
                .object_scope(id)
                .map(ObjectScopeContext::effective_position)
                .unwrap_or_else(|| object.position());
            let shape = live_object_shape(context, id).unwrap_or_default();
            let relative_x = x.wrapping_sub(position.x.wrapping_add(shape.x));
            let relative_y = y.wrapping_sub(position.y.wrapping_add(shape.y));
            Some(
                relative_y >= -5
                    && relative_y <= shape.height.wrapping_sub(1).wrapping_add(10)
                    && relative_x >= -5
                    && relative_x <= shape.width.wrapping_sub(1).wrapping_add(10),
            )
        });
        let Some(direct_hit) = direct_hit else {
            continue;
        };
        if direct_hit {
            let _ = native_blast_object(id, level, caused_by)?;
        }

        // C++ stays inside the already-passed outer block but reads these
        // fields after the direct Blast callback.
        let living_shockwave = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let object = context.get_world_object(id)?;
            let scope = context.object_scope(id);
            let category = scope
                .map(ObjectScopeContext::category)
                .unwrap_or_else(|| object.category());
            if category
                & (crate::CATEGORY_LIVING | crate::CATEGORY_OBJECT | crate::CATEGORY_VEHICLE)
                == 0
            {
                return None;
            }
            let definition = context.object_effective_definition_id(id)?;
            let metadata = context.definition_metadata(&definition)?;
            if metadata.fire.no_horizontal_move != 0 {
                return None;
            }
            let position = scope
                .map(ObjectScopeContext::effective_position)
                .unwrap_or_else(|| object.position());
            if y.wrapping_sub(position.y).wrapping_abs() > level
                || x.wrapping_sub(position.x).wrapping_abs() > level
            {
                return None;
            }
            let floating = scope
                .map(|scope| scope.effective_action_procedure() == ActionProcedure::Float)
                .unwrap_or_else(|| {
                    object
                        .procedure_name()
                        .map(ActionProcedure::from_name)
                        .is_some_and(|procedure| procedure == ActionProcedure::Float)
                });
            if metadata.fire.grab != 1 && (category & crate::CATEGORY_VEHICLE != 0 || floating) {
                return None;
            }
            Some(category & crate::CATEGORY_LIVING != 0)
        });
        let Some(living_shockwave) = living_shockwave else {
            continue;
        };

        if living_shockwave {
            let target = object_reference_value(id);
            let _ = do_energy_with_cause_override(
                &[
                    Value::Int(level.wrapping_neg() / 2),
                    target.clone(),
                    Value::Bool(false),
                    Value::Int(crate::C4FX_CALL_ENG_BLAST),
                ],
                Some(caused_by),
            )?;
            let _ = do_damage_with_cause_override(
                &[
                    Value::Int(level / 2),
                    target,
                    Value::Int(crate::C4FX_CALL_DMG_BLAST),
                ],
                Some(caused_by),
            )?;
        }

        // Living damage callbacks precede both force calculations. p2 is
        // evaluated before p1, and p1 alone consumes one Rnd3 value.
        let force_state = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let object = context.get_world_object(id)?;
            let scope = context.object_scope(id);
            Some((
                scope
                    .map(ObjectScopeContext::effective_position)
                    .unwrap_or_else(|| object.position()),
                scope
                    .map(ObjectScopeContext::category)
                    .unwrap_or_else(|| object.category()),
                reflected_object_mass(context, id, &mut HashSet::new()),
            ))
        });
        let Some((position, category, mass)) = force_state else {
            continue;
        };
        let max_mass_divisor = if category & crate::CATEGORY_LIVING != 0 {
            8
        } else {
            20
        };
        let mass_divisor = (mass / 10).clamp(4, max_mass_divisor);
        let distance_y = y.wrapping_sub(position.y).wrapping_abs();
        let y_force = itofix(distance_y.wrapping_sub(level)) / mass_divisor;
        let rnd3 = draw_context_rnd3()?;
        let distance_x = x.wrapping_sub(position.x).wrapping_abs();
        let direction = position.x.wrapping_sub(x).wrapping_add(rnd3).signum();
        let x_force = itofix(direction.wrapping_mul(level.wrapping_sub(distance_x))) / mass_divisor;
        native_fling(id, FixedVec2::new(x_force, y_force), true, caused_by)?;
    }
    Ok(())
}

/// FnBlastObject (C4Script.cpp:2281-2289) -> C4Object::Blast
/// (C4Object.cpp:1414-1424): DoDamage(level, ..., C4FxCall_DmgBlast) with
/// its synchronous ~Damage callback, then alive targets lose level/3
/// energy percent points (DoEnergy fExact=false, C4FxCall_EngBlast).
pub(crate) fn blast_object(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "BlastObject expects at most 3 arguments: level, target, caused by",
        ));
    }
    let level = parse_optional_i32(args.first(), "BlastObject", "level")?.unwrap_or(0);
    let target_id = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "BlastObject", "target"))
        .transpose()?
        .flatten();
    let caused_by_plus_one =
        parse_optional_i32(args.get(2), "BlastObject", "caused by")?.unwrap_or(0);

    let target_and_cause =
        try_with_host_context("BlastObject requires an active engine context", |context| {
            // iCausedBy = iCausedByPlusOne - 1, else the CALLER's controller
            // (C4Script.cpp:2283) — resolved in the caller's scope.
            let caused_by = if caused_by_plus_one != 0 {
                caused_by_plus_one.wrapping_sub(1)
            } else {
                context
                    .script_object_context
                    .and_then(|caller| {
                        context
                            .object_scope(caller)
                            .map(ObjectScopeContext::controller)
                            .or_else(|| {
                                context
                                    .get_world_object(caller)
                                    .map(|object| object.controller())
                            })
                    })
                    .unwrap_or(OWNER_NONE)
            };
            // `if (!pObj) if (!(pObj = cthr->Obj)) return false` (C4Script.cpp:2284).
            Ok::<_, RuntimeError>(
                target_id
                    .or(context.script_object_context)
                    .map(|target| (target, caused_by)),
            )
        })?;
    let Some((target, caused_by)) = target_and_cause else {
        return Ok(Value::Bool(false));
    };
    // FnBlastObject owns the Status gate. C4Game::BlastObjects deliberately
    // calls its captured `inobj->Blast` without it, even when an earlier
    // callback assigned removal to that container.
    if !object_has_status(target) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(native_blast_object(target, level, caused_by)?))
}

/// Raw exact-cause `C4Object::Blast` entry for engine helpers. The method has
/// no Status gate of its own; public FnBlastObject checks Status before this
/// call, while C4Game::BlastObjects intentionally does not. An already-decoded
/// `OWNER_NONE` must not be reinterpreted as encoded-zero caller fallback.
fn native_blast_object(target: ObjectId, level: i32, caused_by: i32) -> Result<bool, RuntimeError> {
    let alive =
        try_with_host_context_mut("BlastObject requires an active engine context", |context| {
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
            Ok(context.object_scope(target).map(ObjectScopeContext::alive))
        })?;
    let Some(alive) = alive else {
        return Ok(false);
    };
    // DoDamage leg (C4Object.cpp:1416): non-living targets ask their
    // effects first (C4Object.cpp:1282-1286); a zero chain outcome skips
    // the write and the ~Damage call but the Blast proceeds.
    let damage_change = match (!alive)
        .then(|| dispatch_effects_do_damage(target, level, crate::C4FX_CALL_DMG_BLAST, caused_by))
        .flatten()
    {
        Some(0) => None,
        Some(modified) => Some(modified),
        None => Some(level),
    };
    if let Some(damage_change) = damage_change {
        with_host_context_mut((), |context| {
            if let Some(scope) = context.object_scope_mut(target) {
                // Damage = max(Damage + iChange, 0) (C4Object.cpp:1288).
                scope.adjust_damage(damage_change);
            }
        });
        // The ~Damage engine call runs INSIDE DoDamage, before the energy
        // leg (C4Object.cpp:1290 — fail-safe exec, errors log and continue).
        // C4Object::DoDamage always mutates the native field, but its
        // C4Object::Call immediately returns on Status=0. This distinction
        // matters for C4Game::BlastObjects' raw captured-container call.
        if object_has_status(target) {
            if let Some(Err(error)) = call_world_object_own_function(
                target,
                "Damage",
                &[Value::Int(damage_change), Value::Int(caused_by)],
            ) {
                tracing::warn!(
                    %error,
                    "script error in Damage; continuing like the C++ fail-safe exec"
                );
            }
        }
    }
    // Energy leg (C4Object.cpp:1417-1418): only alive targets, reading
    // Alive AFTER the ~Damage callback like the live C4Object would; the
    // living Fx*Damage hook sees the SCALED change (C4Object.cpp:1347,
    // :1355-1359) and a zero chain outcome skips the write.
    let alive_now = with_host_context_mut(false, |context| {
        let alive = context
            .object_scope(target)
            .map(|scope| scope.alive())
            .unwrap_or(false);
        if alive {
            stage_energy_loss_cause(
                context,
                target,
                -level / 3,
                crate::C4FX_CALL_ENG_BLAST,
                caused_by,
            );
        }
        alive
    });
    if alive_now {
        let _ = do_energy_with_cause_override(
            &[
                Value::Int(-level / 3),
                object_reference_value(target),
                Value::Bool(false),
                Value::Int(crate::C4FX_CALL_ENG_BLAST),
            ],
            Some(caused_by),
        )?;
    }
    // Incinerate arm (C4Object.cpp:1420-1423): the LIVE Damage — staged
    // writes plus whatever the ~Damage callback changed — against
    // Def->BlastIncinerate (truthy in C++, so any nonzero value arms it).
    let incinerate = with_host_context(false, |context| {
        let blast_incinerate = effective_definition_id(context, target)
            .and_then(|id| context.world.definition_metadata(&id))
            .map(|metadata| metadata.fire.blast_incinerate)
            .unwrap_or(0);
        blast_incinerate != 0
            && context
                .object_scope(target)
                .map(|scope| scope.damage())
                .unwrap_or(0)
                >= blast_incinerate
    });
    if incinerate {
        incinerate_target_blasted(target, caused_by)?;
    }
    Ok(true)
}

/// The definition the world currently sees for `target`, honoring a
/// same-call staged ChangeDef (C++ reads the live C4Object::Def).
pub(crate) fn effective_definition_id(
    context: &EffectHostContext,
    target: ObjectId,
) -> Option<String> {
    context
        .object_scope(target)
        .and_then(|scope| scope.pending_update.change_def.clone())
        .or_else(|| {
            context
                .get_world_object(target)
                .map(|object| object.definition_id().to_string())
        })
}

/// `C4Object::Incinerate(iCausedBy, fBlasted=true)` on the host seam for
/// BlastObject's ignition arm. The shared constructor path below performs
/// the refusal checks and complete Fire effect negotiation/start sequence.
fn incinerate_target_blasted(target: ObjectId, caused_by: i32) -> Result<(), RuntimeError> {
    incinerate_target(target, caused_by, true, None).map(|_| ())
}

/// `C4Object::Incinerate(iCausedBy, fBlasted)` on the host seam
/// (C4Object.cpp:1257-1266): the refusal checks, the "Fire" C4Effect
/// constructor (:1263-1265), including its Fx*Effect check chain and the
/// script-overridable synchronous FxFireStart.
pub(crate) fn incinerate_target(
    target: ObjectId,
    caused_by: i32,
    blasted: bool,
    incinerating: Option<ObjectId>,
) -> Result<bool, RuntimeError> {
    let eligible = with_host_context_mut(false, |context| {
        if !context.ensure_object_scope(target) {
            return false;
        }
        // Already on fire (C4Object.cpp:1259) — a same-call incinerate
        // shows through the staged fire channel.
        let already_burning = context
            .object_scope(target)
            .and_then(|scope| scope.pending_update.staged_on_fire())
            .unwrap_or_else(|| {
                context
                    .get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.on_fire))
                    .unwrap_or(false)
            });
        if already_burning {
            return false;
        }
        // Dead living don't burn (C4Object.cpp:1261).
        let (category, alive) = context
            .object_scope(target)
            .map(|scope| (scope.current_category, scope.alive()))
            .unwrap_or((0, false));
        if category & crate::CATEGORY_LIVING != 0 && !alive {
            return false;
        }
        true
    });
    if !eligible {
        return Ok(false);
    }

    // Reuse the live-object C4Effect constructor path. Besides keeping the
    // pending node callback-visible, this runs all Fx*Effect checks, honors
    // annul/merge, brackets Start with upper temp calls, and resolves a
    // global script FxFireStart before the native engine fallback.
    let result = add_effect_constructor(
        &[
            Value::String(crate::C4FX_FIRE.into()),
            object_reference_value(target),
            Value::Int(crate::C4FX_FIRE_PRIORITY),
            Value::Int(crate::C4FX_FIRE_TIMER_INTERVAL),
            Value::Nil,
            Value::Nil,
            Value::Int(caused_by),
            Value::Bool(blasted),
            incinerating
                .map(object_reference_value)
                .unwrap_or(Value::Nil),
            Value::Nil,
        ],
        crate::C4FX_FIRE.to_string(),
        false,
        AddEffectCommandIdSlot::Native(None),
    )?;
    Ok(matches!(result, Value::Int(number) if number != 0))
}

/// The engine FnFxFireStart body (C4Effect.cpp:560-641; AddFunc
/// C4Script.cpp:6994) against an EXISTING fire effect entry: the
/// extinguisher gate (checked BEFORE the FirePhase draw), BurnTurnTo
/// changedef, burning contents ejection, the ~FireMode determination,
/// the effect-var writes [Mode, CausedBy, Blasted, IncineratingObj]
/// (:628-631), ONE FirePhase = Random(15) draw on the shared ledger
/// (:634) and the Incineration/IncinerationEx callback — in C++ ledger
/// order. Returns C4Fx_OK (0) or C4Fx_Start_Deny (-1).
fn fire_effect_start_core(
    target: ObjectId,
    fire_number: i32,
    caused_by: i32,
    blasted: bool,
    incinerating: Option<ObjectId>,
) -> Result<i32, RuntimeError> {
    enum FireStage {
        Deny,
        NoFire { blasted: bool },
        Ignite,
    }
    let (stage, burn_turn_to) = HOST_CONTEXT.with(
        |cell| -> Result<(FireStage, Option<String>), RuntimeError> {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return Ok((FireStage::Deny, None));
            };
            if !context.ensure_object_scope(target) {
                return Ok((FireStage::Deny, None));
            }
            // fail if already on fire (C4Effect.cpp:567)
            let already_burning = context
                .object_scope(target)
                .and_then(|scope| scope.pending_update.staged_on_fire())
                .unwrap_or_else(|| {
                    context
                        .get_world_object(target)
                        .and_then(|object| object.full_state().map(|state| state.on_fire))
                        .unwrap_or(false)
                });
            if already_burning {
                return Ok((FireStage::Deny, None));
            }
            // get associated effect (C4Effect.cpp:569-571)
            let entry_exists = context.object_scope(target).is_some_and(|scope| {
                scope
                    .effects
                    .snapshot()
                    .iter()
                    .any(|effect| effect.number == fire_number)
            });
            if !entry_exists {
                return Ok((FireStage::Deny, None));
            }
            // In extinguishing material: no fire caused, checked BEFORE the
            // FirePhase draw (C4Effect.cpp:574-583).
            let position = context
                .object_scope(target)
                .map(|scope| scope.effective_position())
                .unwrap_or(Vector2::ZERO);
            let in_extinguisher = context
                .landscape_ref()
                .and_then(|landscape| landscape.material_at(position.x, position.y))
                .zip(context.world.materials())
                .and_then(|(material_id, materials)| materials.get_by_id(material_id))
                .map(|material| material.extinguisher() != 0)
                .unwrap_or(false);
            let fire_caused = !in_extinguisher;
            let fire_meta = effective_definition_id(context, target)
                .and_then(|id| context.world.definition_metadata(&id))
                .map(|metadata| metadata.fire.clone())
                .unwrap_or_default();
            // BurnTurnTo: blasts changedef in water too (C4Effect.cpp:579-585).
            let turn_to = (fire_caused || blasted)
                .then_some(fire_meta.burn_turn_to)
                .flatten()
                .filter(|turn_to| context.world.definition_metadata(turn_to).is_some());
            if !fire_caused {
                return Ok((FireStage::NoFire { blasted }, turn_to));
            }
            Ok((FireStage::Ignite, turn_to))
        },
    )?;

    if matches!(&stage, FireStage::Deny) {
        return Ok(-1);
    }

    if let Some(turn_to) = burn_turn_to {
        let _ = change_def_live(target, &turn_to)?;
    }

    // ChangeDef above is immediately live in C++. Re-read the effective
    // definition before each guarded block so BurnTurnTo and callbacks can
    // change whether contents are ejected and attached objects detached.
    let eject_contents = with_host_context(false, |context| {
        effective_definition_id(context, target)
            .and_then(|id| context.world.definition_metadata(&id))
            .map(|metadata| !metadata.fire.incomplete_activity && !metadata.fire.no_burn_decay)
            .unwrap_or(true)
    });
    if eject_contents {
        // Snapshot the current contents order, then re-check every link after
        // the preceding callbacks. Controller is assigned before the real
        // Enter/Exit, so RejectEntrance/Ejection/Departure see iCausedBy.
        let contents = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(target))
                .map(|object| object.contents().to_vec())
                .unwrap_or_default()
        });
        for content in contents {
            let parent = HOST_CONTEXT.with(|cell| {
                let mut borrow = cell.borrow_mut();
                let context = borrow.as_mut()?;
                let target_object = context.get_world_object(target)?;
                let content_object = context.get_world_object(content)?;
                if !target_object.is_present()
                    || !content_object.is_present()
                    || content_object.container() != Some(target)
                    || !context.ensure_object_scope(content)
                {
                    return None;
                }
                context.object_scope_mut(content)?.set_controller(caused_by);
                Some(target_object.container())
            });
            let Some(parent) = parent else {
                continue;
            };
            match parent {
                Some(container) => {
                    let _ = enter_object_live(content, container)?;
                }
                None => {
                    let _ = exit_object_at_current_position(content)?;
                }
            }
        }
    }

    let detach_attached = with_host_context(false, |context| {
        effective_definition_id(context, target)
            .and_then(|id| context.world.definition_metadata(&id))
            .map(|metadata| !metadata.fire.incomplete_activity && !metadata.fire.no_burn_decay)
            .unwrap_or(true)
    });
    if detach_attached {
        // C++ performs a fresh FindObject(..., pFindNext) against the live
        // forward master list after every AbortCall. In particular, removing
        // pFindNext from Game.Objects makes the next search stop.
        let mut previous = None;
        loop {
            let candidate = HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let context = borrow.as_ref()?;
                let master_ids = context
                    .master_object_ids()
                    .into_iter()
                    .filter(|candidate| {
                        if let Some(scope) = context.object_scope(*candidate) {
                            return scope.status() != ObjectStatus::Inactive;
                        }
                        context
                            .get_world_object(*candidate)
                            .is_some_and(|object| object.status() != ObjectStatus::Inactive)
                    })
                    .collect::<Vec<_>>();
                let start = match previous {
                    Some(previous) => {
                        master_ids
                            .iter()
                            .position(|candidate| *candidate == previous)?
                            + 1
                    }
                    None => 0,
                };
                master_ids.into_iter().skip(start).find(|candidate| {
                    if let Some(scope) = context.object_scope(*candidate) {
                        return !scope.destroy
                            && scope.status().is_active()
                            && !scope.action_library.is_idle_entry(
                                scope.effective_action_name(),
                                scope.effective_action_index(),
                            )
                            && (scope.effective_action_target(0) == Some(target)
                                || scope.effective_action_target(1) == Some(target));
                    }
                    context.get_world_object(*candidate).is_some_and(|object| {
                        object.status().is_active()
                            && context
                                .world
                                .definition_metadata(object.definition_id())
                                .is_some_and(|metadata| {
                                    !metadata
                                        .action_library
                                        .is_idle_entry(object.action_name(), object.action_index)
                                })
                            && (object.action_target(0) == Some(target)
                                || object.action_target(1) == Some(target))
                    })
                })
            });
            let Some(candidate) = candidate else {
                break;
            };
            previous = Some(candidate);
            // Earlier SetAction callbacks may remove or retarget later
            // candidates; test the live state immediately before detaching.
            let attached = with_host_context(false, |context| {
                if let Some(scope) = context.object_scope(candidate) {
                    return !scope.destroy
                        && scope.status().is_active()
                        && scope.effective_action_procedure() == ActionProcedure::Attach
                        && (scope.effective_action_target(0) == Some(target)
                            || scope.effective_action_target(1) == Some(target));
                }
                context.get_world_object(candidate).is_some_and(|object| {
                    object.is_present()
                        && object
                            .procedure_name()
                            .map(ActionProcedure::from_name)
                            .unwrap_or_default()
                            == ActionProcedure::Attach
                        && (object.action_target(0) == Some(target)
                            || object.action_target(1) == Some(target))
                })
            });
            if attached {
                // C++ calls C4Object::SetAction(ActIdle) directly. A script
                // function named SetAction must not intercept this detach.
                let _ = native_set_action_by_name(candidate, "Idle")?;
            }
        }
    }

    match stage {
        FireStage::Deny => unreachable!("denied fire start returned above"),
        FireStage::NoFire { blasted } => {
            // Blasted but not incinerated: IncinerationEx
            // (C4Effect.cpp:602-607) — fail-safe exec.
            if blasted {
                if let Some(Err(error)) = call_world_object_own_function(
                    target,
                    "IncinerationEx",
                    &[Value::Int(caused_by)],
                ) {
                    tracing::warn!(
                        %error,
                        "script error in IncinerationEx; continuing like the C++ fail-safe exec"
                    );
                }
            }
            return Ok(-1);
        }
        FireStage::Ignite => {}
    }
    let category = with_host_context(0, |context| {
        let Some(scope) = context.object_scope(target) else {
            return 0;
        };
        scope.pending_update.category.unwrap_or_else(|| {
            scope
                .pending_update
                .change_def
                .as_deref()
                .and_then(|id| context.world.definition_metadata(id))
                .map(|metadata| metadata.category)
                .unwrap_or(scope.current_category)
        })
    });
    // determine fire appearance (C4Effect.cpp:609-626): the ~FireMode
    // script answer wins; zero falls back to the category default; an
    // out-of-range answer degrades to Object mode.
    let mode_answer = match call_world_object_own_function(target, "FireMode", &[]) {
        Some(Ok(value)) => value_as_i32(&value),
        Some(Err(error)) => {
            tracing::warn!(
                %error,
                "script error in FireMode; continuing like the C++ fail-safe exec"
            );
            0
        }
        _ => 0,
    };
    let fire_mode = if mode_answer == 0 {
        if category & (crate::CATEGORY_LIVING | crate::CATEGORY_STATIC_BACK) != 0 {
            crate::C4FX_FIRE_MODE_LIVING_VEG
        } else if category & (crate::CATEGORY_STRUCTURE | crate::CATEGORY_VEHICLE) != 0 {
            crate::C4FX_FIRE_MODE_STRUCT_VEH
        } else {
            crate::C4FX_FIRE_MODE_OBJECT
        }
    } else if !(1..=crate::C4FX_FIRE_MODE_OBJECT).contains(&mode_answer) {
        tracing::warn!(
            mode = mode_answer,
            object = target.as_u64(),
            "FireMode is invalid; using Object mode like C++"
        );
        crate::C4FX_FIRE_MODE_OBJECT
    } else {
        mode_answer
    };
    HOST_CONTEXT.with(|cell| -> Result<(), RuntimeError> {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(());
        };
        // store causes in effect vars (C4Effect.cpp:628-631)
        if let Some(scope) = context.object_scope_mut(target) {
            let number = fire_number.max(0) as usize;
            scope
                .effects
                .effect_var(number, 0, Some(EffectVarValue::Int(fire_mode)));
            scope
                .effects
                .effect_var(number, 1, Some(EffectVarValue::Int(caused_by)));
            scope
                .effects
                .effect_var(number, 2, Some(EffectVarValue::Bool(blasted)));
            scope.effects.effect_var(
                number,
                3,
                Some(
                    incinerating
                        .map(|id| EffectVarValue::Object(id.as_u64()))
                        .unwrap_or(EffectVarValue::Nil),
                ),
            );
        }
        // Set values + FirePhase = Random(15), one synced draw
        // (C4Effect.cpp:632-634).
        let phase = draw_context_random(15)?;
        if let Some(scope) = context.object_scope_mut(target) {
            scope.pending_update.stage_ignite(caused_by, phase);
        }
        Ok(())
    })?;
    // Engine script call (C4Effect.cpp:638) — fail-safe exec.
    if let Some(Err(error)) =
        call_world_object_own_function(target, "Incineration", &[Value::Int(caused_by)])
    {
        tracing::warn!(
            %error,
            "script error in Incineration; continuing like the C++ fail-safe exec"
        );
    }
    Ok(0)
}

/// FnFxFireStart (C4Effect.cpp:560-641; AddFunc C4Script.cpp:6994) — the
/// engine-internal fire start, reachable from a script FxFireStart
/// overload's inherited(...) chain. A temp readd only re-arms the flag
/// (:563-565).
pub(crate) fn fx_fire_start(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "FxFireStart",
        "object",
    )?;
    // safety (C4Effect.cpp:563)
    let Some(target) = target else {
        return Ok(Value::Int(-1));
    };
    let fire_number = parse_optional_i32(args.get(1), "FxFireStart", "number")?.unwrap_or(0);
    let temp = args.get(2).is_some_and(|value| value_as_i32(value) != 0);
    if temp {
        // temp readd: SetOnFire(true), return 1 (C4Effect.cpp:565)
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                if context.ensure_object_scope(target) {
                    if let Some(scope) = context.object_scope_mut(target) {
                        scope.pending_update.stage_fire_flag(true);
                    }
                }
            }
        });
        return Ok(Value::Int(1));
    }
    let caused_by = parse_optional_i32(args.get(3), "FxFireStart", "caused by")?.unwrap_or(0);
    let blasted = args.get(4).map(extract_cpp_native_bool).unwrap_or(false);
    let incinerating = args
        .get(5)
        .map(|arg| parse_object_reference_argument(arg, "FxFireStart", "incinerating object"))
        .transpose()?
        .flatten();
    fire_effect_start_core(target, fire_number, caused_by, blasted, incinerating).map(Value::Int)
}

/// FnFxFireTimer (C4Effect.cpp:643-666; AddFunc C4Script.cpp:6995) →
/// C4Object::ExecFire (C4Object.cpp:766-810) through the staged seam —
/// a script FxFireTimer overload's inherited(...) chain lands here. The
/// deterministic arms (phase, decay, Tick10 damage, Tick5 energy,
/// extinguisher, the Random(3) inflame draw) run in C++ ledger order;
/// fire particles, SmokeRate smoke, and sounds are presentation-only.
pub(crate) fn fx_fire_timer(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "FxFireTimer",
        "object",
    )?;
    // safety: no object → C4Fx_Execute_Kill (C4Effect.cpp:646)
    let Some(target) = target else {
        return Ok(Value::Int(-1));
    };
    let fire_number = parse_optional_i32(args.get(1), "FxFireTimer", "number")?.unwrap_or(0);
    let frame = ENVIRONMENT_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| context.frame)
            .unwrap_or(0)
    });
    struct FireExecState {
        caused_by: i32,
        attribution_caused_by: i32,
        phase: i32,
        no_burn_decay: bool,
        no_burn_damage: bool,
        position: Vector2,
    }
    let state = HOST_CONTEXT.with(|cell| -> Option<FireExecState> {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(target) {
            return None;
        }
        let scope = context.object_scope(target)?;
        let burning = scope.pending_update.staged_on_fire().unwrap_or_else(|| {
            context
                .get_world_object(target)
                .and_then(|object| object.full_state().map(|state| state.on_fire))
                .unwrap_or(false)
        });
        if !burning {
            return None;
        }
        let world_fire = context.get_world_object(target).and_then(|object| {
            object
                .full_state()
                .map(|state| (state.fire_caused_by, state.fire_phase))
        });
        // staged ignite wins over the world snapshot
        let (caused_by, phase) = scope
            .pending_update
            .fire
            .or(world_fire)
            .unwrap_or((OWNER_NONE, 0));
        // FnFxFireTimer reads the cause from the effect variable every
        // time, then validates that local value without rewriting Var(1).
        let effect_caused_by = scope
            .effects
            .snapshot()
            .iter()
            .find(|effect| effect.number == fire_number)
            .and_then(|effect| effect.vars.get(1))
            .map(|value| match value {
                EffectVarValue::Int(value) => *value,
                EffectVarValue::Bool(value) => i32::from(*value),
                EffectVarValue::RawBool(value) => *value as u32 as i32,
                _ => 0,
            })
            .unwrap_or(caused_by);
        let attribution_caused_by = context
            .player_state(effect_caused_by)
            .map(|_| effect_caused_by)
            .unwrap_or(OWNER_NONE);
        let fire_meta = effective_definition_id(context, target)
            .and_then(|id| context.world.definition_metadata(&id))
            .map(|metadata| metadata.fire.clone())
            .unwrap_or_default();
        let scope = context.object_scope(target)?;
        Some(FireExecState {
            caused_by,
            attribution_caused_by,
            phase,
            no_burn_decay: fire_meta.no_burn_decay,
            no_burn_damage: fire_meta.no_burn_damage,
            position: scope.effective_position(),
        })
    });
    // fire already out: kill the effect (C4Effect.cpp:663-666)
    let Some(state) = state else {
        return Ok(Value::Int(-1));
    };
    // Fire Phase (C4Object.cpp:770)
    let next_phase = (state.phase + 1) % crate::MAX_FIRE_PHASE;
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            if let Some(scope) = context.object_scope_mut(target) {
                scope
                    .pending_update
                    .stage_ignite(state.caused_by, next_phase);
            }
        }
    });
    // C4Object::ExecFire's Tick5 base arm runs immediately after the phase
    // advance and before decay/damage/energy. ValidPlr is only membership in
    // the live player list; the container need not belong to the victim.
    if frame.is_multiple_of(5) {
        let extinguish_in_base = with_host_context(false, |context| {
            if !context.world.base_extinguish_enabled {
                return false;
            }
            let Some(scope) = context.object_scope(target) else {
                return false;
            };
            if scope.category() & crate::CATEGORY_LIVING == 0 {
                return false;
            }
            let Some(container) = scope.container() else {
                return false;
            };
            let base = context
                .object_scope(container)
                .and_then(|scope| scope.pending_update.base)
                .or_else(|| {
                    context
                        .get_world_object(container)
                        .and_then(|object| object.full_state().map(|state| state.base))
                });
            base.is_some_and(|base| context.player_state(base).is_some())
        });
        if extinguish_in_base {
            extinguish_effect_target(target, fire_number)?;
        }
    }
    // Decay: DoCon(-100) every frame; burned away at zero construction
    // (C4Object.cpp:779-781 + the engine-side burn loop).
    if !state.no_burn_decay {
        let _ = do_con_live(target, -100)?;
    }
    let target_value = object_reference_value(target);
    // Damage: Tick10 DoDamage(+2) by fire (C4Object.cpp:783)
    if frame.is_multiple_of(10) && !state.no_burn_damage {
        do_damage_with_cause_override(
            &[
                Value::Int(2),
                target_value.clone(),
                Value::Int(crate::C4FX_CALL_DMG_FIRE),
                Value::Int(state.attribution_caused_by + 1),
            ],
            Some(state.attribution_caused_by),
        )?;
    }
    // Energy: Tick5 DoEnergy(-1) (C4Object.cpp:785)
    if frame.is_multiple_of(5) {
        do_energy_with_cause_override(
            &[
                Value::Int(-1),
                target_value,
                Value::Bool(false),
                Value::Int(crate::C4FX_CALL_ENG_FIRE),
                Value::Int(state.attribution_caused_by + 1),
            ],
            Some(state.attribution_caused_by),
        )?;
    }
    // Background effects: Tick5 over valid landscape material — extinguish
    // in extinguisher material, then the unconditional Random(3) inflame
    // draw (C4Object.cpp:794-809).
    if frame.is_multiple_of(5) {
        let material_extinguisher = HOST_CONTEXT.with(|cell| {
            cell.borrow().as_ref().and_then(|context| {
                context
                    .landscape_ref()
                    .and_then(|landscape| landscape.material_at(state.position.x, state.position.y))
                    .zip(context.world.materials())
                    .and_then(|(material_id, materials)| materials.get_by_id(material_id))
                    .map(|material| material.extinguisher() != 0)
            })
        });
        if let Some(extinguisher) = material_extinguisher {
            if extinguisher {
                // Extinguish(iFireNumber) — C4Object.cpp:801; the number
                // form kills exactly this effect.
                extinguish_effect_target(target, fire_number)?;
            }
            // Inflame (C4Object.cpp:803-804)
            if draw_context_random(3)? == 0 {
                incinerate_landscape_at(state.position.x, state.position.y)?;
            }
        }
    }
    // FnFxFireTimer returns C4Fx_Execute_Kill once the flag is gone
    // (C4Effect.cpp:663-666).
    let still_burning = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(target))
            .and_then(|scope| scope.pending_update.staged_on_fire())
            .unwrap_or(true)
    });
    Ok(Value::Int(if still_burning { 0 } else { -1 }))
}

/// FnFxFireStop (C4Effect.cpp:775-792; AddFunc C4Script.cpp:6996): clear
/// the OnFire flag — real and temp removals alike; the Fire sound stop
/// is presentation-only.
pub(crate) fn fx_fire_stop(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "FxFireStop",
        "object",
    )?;
    // safety (C4Effect.cpp:778)
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            if context.ensure_object_scope(target) {
                if let Some(scope) = context.object_scope_mut(target) {
                    scope.pending_update.stage_fire_flag(false);
                }
            }
        }
    });
    Ok(Value::Bool(true))
}

/// FnFxFireInfo (C4Effect.cpp:794-797; AddFunc C4Script.cpp:6997): the
/// burning status line (IDS_OBJ_BURNS).
pub(crate) fn fx_fire_info(_args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::String("{{FLAM}} The object burns.".into()))
}

/// FnIncinerate (C4Script.cpp:245-252): the target defaults to the
/// caller; iCausedBy is the CALLING object's controller (NO_OWNER
/// without one).
pub(crate) fn incinerate(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Incinerate", "target"))
        .transpose()?
        .flatten();
    let (active, caused_by) = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context())
            .map(|object| (Some(object.id()), object.controller()))
            .unwrap_or((None, OWNER_NONE))
    });
    let Some(target) = target_id.or(active) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(incinerate_target(
        target, caused_by, false, None,
    )?))
}

/// FnExtinguish (C4Script.cpp:264-270) → C4Object::Extinguish(0)
/// (C4Object.cpp:1269-1301): kill every "*Fire*" effect, skipping the
/// engine-internal "Int*" names (C4Fx_AnyFire/C4Fx_Internal,
/// C4Effects.h:154-155).
pub(crate) fn extinguish(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Extinguish", "target"))
        .transpose()?
        .flatten();
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    let Some(target) = target_id.or(active) else {
        return Ok(Value::Bool(false));
    };
    extinguish_target(target).map(Value::Bool)
}

/// The staged half of C4Object::Extinguish(0): the kill loop over
/// "*Fire*"-matching, non-"Int*" effects (C4Object.cpp:1281-1298). The
/// engine-internal FnFxFireStop clears the OnFire flag synchronously
/// (C4Effect.cpp:787) unless a script global shadows it; script Fx*Stop
/// callbacks ride the deferred Stopped dispatch of the staged removals.
fn extinguish_target(target: ObjectId) -> Result<bool, RuntimeError> {
    extinguish_effect_target(target, 0)
}

/// The numbered `C4Object::Extinguish(iFireNumber)` form used by ExecFire:
/// zero selects every public `*Fire*` effect, while a nonzero number kills
/// exactly that effect (C4Object.cpp:1276-1299).
fn extinguish_effect_target(target: ObjectId, fire_number: i32) -> Result<bool, RuntimeError> {
    let engine_fire_stop = !script_shadows_engine_fx("FxFireStop");
    with_host_context_mut(Ok(false), |context| {
        if !context.ensure_object_scope(target) {
            return Ok(false);
        }
        let Some(scope) = context.object_scope_mut(target) else {
            return Ok(false);
        };
        let mut killed = 0usize;
        loop {
            let number = scope.effects.effects.iter().find_map(|effect| {
                (effect.priority != 0
                    && if fire_number != 0 {
                        effect.number == fire_number
                    } else {
                        effect.name.contains("Fire") && !effect.name.starts_with("Int")
                    })
                .then_some(effect.number)
            });
            let Some(number) = number else { break };
            let Some(removed) = scope.effects.remove_live_effect(None, number.max(0), false) else {
                break;
            };
            if removed.name == crate::C4FX_FIRE && engine_fire_stop {
                scope.pending_update.stage_fire_flag(false);
            }
            killed += 1;
            if fire_number != 0 {
                break;
            }
        }
        Ok(killed > 0)
    })
}

/// Whether a script global shadows an engine-registered Fx* function —
/// C4Effect callback resolution finds script functions before the
/// engine's own (GetFuncRecursive over AddFunc'd C++ functions,
/// C4Effect.cpp:30-56 + C4Script.cpp:6994-6997).
fn script_shadows_engine_fx(function: &str) -> bool {
    HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().is_some_and(|context| {
            context
                .world
                .scenario_script()
                .is_some_and(|script| script.has_global_function(function))
                || context
                    .world
                    .definition_scripts()
                    .any(|script| script.has_global_function(function))
        })
    })
}

pub(crate) fn create_particle(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) if !name.is_empty() => name.as_ref().to_owned(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "CreateParticle: expected string for name, got {}",
                other.type_name()
            )));
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

    try_with_host_context_mut(
        "CreateParticle requires an active engine context",
        |context| {
            let base_position = context
                .object_context()
                .map(|object| object.effective_position())
                .unwrap_or(Vector2::ZERO);

            let world_x = base_position.x.saturating_add(x);
            let world_y = base_position.y.saturating_add(y);

            let layer = if let Some(target) = target_object {
                if !context.object_status_present(target) {
                    return Ok(Value::Bool(false));
                }
                if context.get_world_object(target).is_none() {
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

            // GetDef failure → false (C4Script.cpp:4874)
            if context.particle_def_known(&definition) == Some(false) {
                return Ok(Value::Bool(false));
            }

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
        },
    )
}

/// FnCastAParticles (C4Script.cpp:4881-4898), shared by CastParticles and
/// CastBackParticles. Args: name, amount, level, x, y, a0, a1, b0, b1, obj.
fn cast_a_particles(args: &[Value], back: bool, fn_name: &str) -> Result<Value, RuntimeError> {
    let definition = match args.first() {
        Some(Value::String(name)) if !name.is_empty() => name.as_ref().to_owned(),
        Some(Value::String(_)) | Some(Value::Nil) | None => return Ok(Value::Bool(false)),
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "{fn_name}: expected string for name, got {}",
                other.type_name()
            )));
        }
    };

    let int_arg = |index: usize, label: &str| -> Result<i32, RuntimeError> {
        args.get(index)
            .map(|arg| value_to_i32(arg, fn_name, label))
            .transpose()
            .map(|value| value.unwrap_or(0))
    };
    let amount = int_arg(1, "amount")?;
    let level = int_arg(2, "level")?;
    let x = int_arg(3, "x")?;
    let y = int_arg(4, "y")?;
    let a0 = int_arg(5, "a0")?;
    let a1 = int_arg(6, "a1")?;
    let b0 = int_arg(7, "b0")? as u32;
    let b1 = int_arg(8, "b1")? as u32;

    let target_object = args
        .get(9)
        .map(|arg| parse_object_reference_argument(arg, fn_name, "object"))
        .transpose()?
        .flatten();

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new(format!("{fn_name} requires an active engine context"))
        })?;

        // safety: pObj && !pObj->Status → false (C4Script.cpp:4884)
        let layer = if let Some(target) = target_object {
            if !context.object_status_present(target) {
                return Ok(Value::Bool(false));
            }
            if context.get_world_object(target).is_none() {
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

        // GetDef failure → false (C4Script.cpp:4893)
        if context.particle_def_known(&definition) == Some(false) {
            return Ok(Value::Bool(false));
        }

        // local offset (C4Script.cpp:4886-4890)
        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);

        context.register_particle(ParticleCommand::Cast {
            definition_id: definition,
            amount,
            x: base_position.x.saturating_add(x) as f32,
            y: base_position.y.saturating_add(y) as f32,
            level,
            a0: a0 as f32 / 10.0,
            b0,
            a1: a1 as f32 / 10.0,
            b1,
            layer,
        });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn cast_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    cast_a_particles(args, false, "CastParticles")
}

pub(crate) fn cast_back_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    cast_a_particles(args, true, "CastBackParticles")
}

/// FnPushParticles (C4Script.cpp:4910-4923): name nil → push all particles;
/// a named def that is not loaded → false.
pub(crate) fn push_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition = match args.first() {
        Some(Value::String(name)) if !name.is_empty() => Some(name.as_ref().to_owned()),
        Some(Value::String(_)) | Some(Value::Nil) | None => None,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "PushParticles: expected string or nil for name, got {}",
                other.type_name()
            )));
        }
    };
    let ax = args
        .get(1)
        .map(|arg| value_to_i32(arg, "PushParticles", "xdir"))
        .transpose()?
        .unwrap_or(0);
    let ay = args
        .get(2)
        .map(|arg| value_to_i32(arg, "PushParticles", "ydir"))
        .transpose()?
        .unwrap_or(0);

    try_with_host_context_mut(
        "PushParticles requires an active engine context",
        |context| {
            if let Some(name) = &definition {
                if context.particle_def_known(name) == Some(false) {
                    return Ok(Value::Bool(false));
                }
            }
            context.register_particle(ParticleCommand::Push {
                definition_id: definition,
                dxdir: ax as f32 / 10.0,
                dydir: ay as f32 / 10.0,
            });
            Ok(Value::Bool(true))
        },
    )
}

pub(crate) fn clear_particles(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut definition: Option<String> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::String(name) if !name.is_empty() => definition = Some(name.as_ref().to_owned()),
            Value::String(_) | Value::Nil => definition = None,
            other => {
                return Err(RuntimeError::new(format!(
                    "ClearParticles: expected string or nil for name, got {}",
                    other.type_name()
                )));
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

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        // a named def that is not loaded → false (C4Script.cpp:4932)
        if let Some(name) = &definition {
            if context.particle_def_known(name) == Some(false) {
                return Ok(Value::Bool(false));
            }
        }

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

/// FnFlameConsumeMaterial (C4Script.cpp:2172-2180): consume one
/// caller-relative landscape pixel only when its material is inflammable.
/// The synchronous return value is computed from the current landscape and
/// the matching real ExtractMaterial mutation is folded after the callback.
pub(crate) fn flame_consume_material(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "FlameConsumeMaterial",
        "x",
    )?;
    let y = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "FlameConsumeMaterial",
        "y",
    )?;
    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let mut position = Vector2::new(x, y);
        if let Some(object) = context.object_context() {
            let base = object.effective_position();
            position = Vector2::new(base.x.saturating_add(x), base.y.saturating_add(y));
        }

        let extracted = match (context.landscape_ref(), context.world.materials()) {
            (Some(landscape), Some(materials)) => landscape
                .material_at(position.x, position.y)
                .filter(|&material| {
                    materials
                        .get_by_id(material)
                        .is_some_and(|entry| entry.inflammable() != 0)
                })
                .is_some_and(|material| {
                    landscape.simulate_extract_material_amount(
                        materials, position.x, position.y, material, 1,
                    ) == 1
                }),
            _ => false,
        };
        if extracted {
            let material = context
                .landscape_ref()
                .and_then(|landscape| landscape.material_at(position.x, position.y))
                .expect("successful material extraction keeps its source material");
            context.register_landscape_operation(LandscapeOperation::ExtractMaterialAmount {
                material: material.index() as i32,
                position,
                amount: 1,
            });
        }
        Ok(Value::Bool(extracted))
    })
}

/// FnOnFire (C4Script.cpp:1866-1877): burning when the OnFire flag is set
/// or any *Fire* effect (C4Fx_AnyFire) sits on the object; nil without one.
/// Staged same-call writes (Incinerate/Extinguish/RemoveEffect) win over
/// the world snapshot like C++'s live flag.
pub(crate) fn on_fire(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "OnFire", "target")?;
    with_host_context(Ok(Value::Nil), |context| {
        let target = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        let world_flag = || {
            context
                .get_world_object(target)
                .and_then(|object| object.full_state().map(|state| state.on_fire))
                .unwrap_or(false)
        };
        if let Some(scope) = context.object_scope(target) {
            let flag = scope
                .pending_update
                .staged_on_fire()
                .unwrap_or_else(world_flag);
            let burning = flag
                || scope
                    .effects
                    .snapshot()
                    .iter()
                    .any(|effect| effect.name.contains("Fire"));
            return Ok(Value::Bool(burning));
        }
        match context.get_world_object(target) {
            Some(other) => Ok(Value::Bool(other.full_state().is_some_and(|state| {
                state.on_fire
                    || state
                        .effects
                        .iter()
                        .any(|effect| effect.name.contains("Fire"))
            }))),
            None => Ok(Value::Nil),
        }
    })
}

/// FnIncinerateLandscape (C4Script.cpp:253-261) -> C4Landscape::Incinerate
/// (C4Landscape.cpp:1430-1441): caller-relative point; inflammable
/// material lights one FLAM unless another already burns in the
/// (x-4, y-1, 8, 20) rect (C4Game::FindObject center-in-rect range check,
/// C4Game.cpp: `Inside(cObj->x - iX, 0, iWdt-1)`). C++ creates the FLAM
/// mid-call (Game.CreateObject), including its synchronous lifecycle.
pub(crate) fn incinerate_landscape(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut x = parse_optional_i32(args.first(), "IncinerateLandscape", "x")?.unwrap_or(0);
    let mut y = parse_optional_i32(args.get(1), "IncinerateLandscape", "y")?.unwrap_or(0);

    // Local calls offset by the object position (C4Script.cpp:255-259).
    let offset = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context())
            .map(|object| object.effective_position())
    });
    if let Some(position) = offset {
        x = x.saturating_add(position.x);
        y = y.saturating_add(position.y);
    }
    incinerate_landscape_at(x, y)
}

/// `C4Landscape::Incinerate` (C4Landscape.cpp:1430-1441) at ABSOLUTE
/// coordinates — shared by FnIncinerateLandscape and the fire timer's
/// inflame arm (C4Object.cpp:803-804).
fn incinerate_landscape_at(x: i32, y: i32) -> Result<Value, RuntimeError> {
    let can_create = HOST_CONTEXT.with(|cell| -> Result<bool, RuntimeError> {
        let borrow = cell.borrow();
        let context = borrow.as_ref().ok_or_else(|| {
            RuntimeError::new("IncinerateLandscape requires an active engine context")
        })?;

        let inflammable = match (context.landscape_ref(), context.world.materials()) {
            (Some(landscape), Some(materials)) => landscape.can_incinerate(x, y, materials),
            _ => false,
        };
        if !inflammable {
            return Ok(false);
        }

        if context
            .definition_metadata(crate::FIRE_DEFINITION_ID)
            .is_none()
        {
            // Unknown FLAM def: Game.CreateObject returns nullptr.
            return Ok(false);
        }

        // "Not too much FLAMs" (C4Landscape.cpp:1436-1437) — pending
        // same-call spawns count like the live objects C++ would find.
        let left = x.saturating_sub(4);
        let right = left.saturating_add(8);
        let top = y.saturating_sub(1);
        let bottom = top.saturating_add(20);
        let burning = context.world_object_ids().into_iter().any(|id| {
            context
                .get_world_object(id)
                .filter(|object| object.definition_id() == crate::FIRE_DEFINITION_ID)
                .filter(|object| object.status().is_active())
                .map(|object| {
                    let pos = object.position;
                    pos.x >= left && pos.x < right && pos.y >= top && pos.y < bottom
                })
                .unwrap_or(false)
        });
        if burning {
            return Ok(false);
        }
        Ok(true)
    })?;
    if !can_create {
        return Ok(Value::Bool(false));
    }

    // Game.CreateObject completes Construction, initial DoCon and
    // Completion/Initialize before Incinerate reports success. In particular,
    // a FLAM that removes itself during Construction makes this probe fail so
    // Explosion continues with the next ignition point.
    Ok(Value::Bool(
        create_native_object(NativeObjectCreation {
            definition: crate::FIRE_DEFINITION_ID.to_string(),
            creator: None,
            owner: OWNER_NONE,
            controller: OWNER_NONE,
            construction: FULL_CON,
            position: Vector2::new(x, y),
            rotation: 0,
            velocity: FixedVec2::ZERO,
            rotation_velocity: C4Fixed::ZERO,
        })?
        .is_some(),
    ))
}

/// The `C4VObj(pForObj)` first argument sent to Fx callbacks. A typed null
/// object extracts to `pForObj == nullptr`, and constructing the callback value
/// canonicalizes that pointer to C4V_Any nil rather than preserving its source
/// C4V_C4Object tag.
pub(crate) fn effect_callback_target_value(scope: EffectScope, target: &Value) -> Value {
    match (scope, target) {
        (EffectScope::Object(_), value @ Value::Object(_)) => value.clone(),
        _ => Value::Nil,
    }
}

pub(crate) fn value_to_effect_var(value: &Value) -> EffectVarValue {
    match value {
        Value::Int(value) => EffectVarValue::Int(*value),
        Value::Bool(value) => EffectVarValue::Bool(*value),
        Value::RawBool(value) => EffectVarValue::RawBool(*value),
        Value::String(value) => EffectVarValue::String(value.clone()),
        Value::C4Id(id) => EffectVarValue::C4Id(id.clone()),
        Value::Object(id) => EffectVarValue::Object(*id),
        Value::Array(entries) => {
            let vars = entries.iter().map(value_to_effect_var).collect();
            EffectVarValue::Array(vars)
        }
        Value::Proplist(map) => EffectVarValue::Proplist(map.clone()),
        Value::Nil => EffectVarValue::Nil,
    }
}

pub(crate) fn effect_var_to_value(value: &EffectVarValue) -> Value {
    match value {
        EffectVarValue::Int(value) => Value::Int(*value),
        EffectVarValue::Bool(value) => Value::Bool(*value),
        EffectVarValue::RawBool(value) => Value::from_c4_bool_data_raw(*value),
        EffectVarValue::String(value) => Value::String(value.clone()),
        EffectVarValue::C4Id(id) => Value::C4Id(id.clone()),
        EffectVarValue::Object(id) => Value::Object(*id),
        EffectVarValue::Array(entries) => {
            let vars = entries.iter().map(effect_var_to_value).collect();
            Value::Array(vars)
        }
        EffectVarValue::Proplist(map) => Value::Proplist(map.clone()),
        EffectVarValue::Nil => Value::Nil,
    }
}
