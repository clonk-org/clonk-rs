use super::*;

const MAX_VERTEX_COUNT: i32 = 30;

/// `SetPhysical`/`GetPhysical` modes (C4Script.cpp:552-555).
pub(crate) const PHYS_CURRENT: i32 = 0;
pub(crate) const PHYS_PERMANENT: i32 = 1;
pub(crate) const PHYS_TEMPORARY: i32 = 2;
pub(crate) const PHYS_STACK_TEMPORARY: i32 = 3;

/// FnObjectSetAction (C4Script.cpp:782-789): SetActionByName on ANOTHER
/// object (with start/abort calls). Routed through the reentrancy seam so
/// the target's SetAction host fn runs in the target's scope.
pub(crate) fn object_set_action(args: &[Value]) -> Result<Value, RuntimeError> {
    if std::env::var("LC_DEBUG_CHBM").is_ok() {
        tracing::warn!(?args, "OSA ObjectSetAction");
    }
    let Some(target) = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "ObjectSetAction",
        "obj",
    )?
    else {
        return Ok(Value::Bool(false));
    };
    let Some(action) = parse_optional_string_value(args.get(1), "ObjectSetAction", "action")?
    else {
        return Ok(Value::Bool(false)); // !szAction
    };
    let mut forwarded: Vec<Value> = vec![Value::String(action)];
    forwarded.extend(args.iter().skip(2).take(3).cloned());
    match call_world_object_function(target, "SetAction", &forwarded) {
        Some(result) => result,
        None => Ok(Value::Bool(false)),
    }
}

/// FnKill (C4Script.cpp:335-345): default a null target to `cthr->Obj`,
/// reject missing/dead objects, trace a valid calling controller through
/// UpdatLastEnergyLossCause, and run the complete AssignDeath path.
pub(crate) fn kill(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Kill", "target")?;
    let forced = value_to_bool(args.get(1).unwrap_or(&Value::Nil), "Kill", "forced")?;
    // C4Aul's typed two-parameter dispatch discards surplus values after
    // evaluating them (C4AulExec.cpp:1364-1396).

    let target = with_host_context_mut(Ok(None), |context| {
        // `cthr->Obj`, not the mutable effect carrier: definition-owned
        // callbacks may carry pForObj state while executing with Obj=null.
        let caller = context.script_object_context;
        let Some(target) = target_id.or(caller) else {
            return Ok(None);
        };
        if !context.ensure_object_scope(target)
            || !context
                .object_scope(target)
                .is_some_and(ObjectScopeContext::alive)
        {
            return Ok(None);
        }

        let caller_controller = caller.and_then(|caller| {
            context
                .object_scope(caller)
                .map(ObjectScopeContext::controller)
                .or_else(|| {
                    context
                        .get_world_object(caller)
                        .map(|object| object.controller())
                })
        });
        let valid_controller =
            caller_controller.filter(|controller| context.player_state(*controller).is_some());
        if let Some(controller) = valid_controller {
            stage_energy_loss_cause(context, target, -1, crate::C4FX_CALL_ENG_SCRIPT, controller);
        }
        Ok(Some(target))
    })?;
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    let _ = assign_death_live(target, forced)?;
    Ok(Value::Bool(true))
}

/// Complete synchronous `C4Object::AssignDeath` over the VM's live object
/// scopes. This is shared by script Kill and host DoEnergy so every callback
/// and same-call read observes the native ordering before the invoking script
/// resumes (oracle-src-pinned src/C4Object.cpp:1164-1205).
pub(crate) fn assign_death_live(target: ObjectId, forced: bool) -> Result<bool, RuntimeError> {
    let death_causing_player = with_host_context_mut(None, |context| {
        if !context.ensure_object_scope(target)
            || !context
                .object_scope(target)
                .is_some_and(ObjectScopeContext::alive)
        {
            return None;
        }
        let cause = context
            .object_scope(target)
            .and_then(|scope| scope.pending_update.energy_loss_cause)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.last_energy_loss_cause)
            })
            .unwrap_or(OWNER_NONE);
        if let Some(scope) = context.object_scope_mut(target) {
            // Alive is cleared before RemoveDeath callbacks both to expose
            // the death-in-progress state and to prevent recursive death.
            scope.set_raw_alive(false);
        }
        Some(cause)
    });
    let Some(death_causing_player) = death_causing_player else {
        return Ok(false);
    };

    let _ = clear_effects_for_assign_death(target)?;
    let revived = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(target))
            .is_some_and(ObjectScopeContext::alive)
    });
    if revived && !forced {
        return Ok(true);
    }

    // SetActionByName("Dead") is an ordinary native action transition:
    // NoOtherAction may reject it and Start/Abort callbacks run inline.
    let _ = native_set_action_by_name(target, "Dead")?;
    with_host_context_mut((), |context| {
        if let Some(scope) = context.object_scope_mut(target) {
            scope.set_selected(false);
            // Forced death clears a RemoveDeath callback's revival again.
            scope.set_raw_alive(false);
            scope.clear_command_stack();
        }
        assign_death_host_crew_info(context, target);
    });

    // Re-read the live list head after every callbackful Exit. Ejection or
    // Departure is allowed to mutate the remaining contents.
    loop {
        let content = with_host_context(None, |context| first_retained_content(context, target));
        let Some(content) = content else {
            break;
        };
        let _ = exit_object_at_current_position(content)?;
    }

    // C++ snapshots pPlr from the current Owner before ClearPointers. The
    // callbackful cursor adjustment may subsequently change Owner, Category
    // or FoW membership, but the retention test still uses that same player.
    let owner_player = with_host_context(None, |context| {
        let owner = context.object_scope(target)?.owner();
        context.player_state(owner).map(|_| owner)
    });
    if let Some(owner) = owner_player {
        clear_owner_death_pointers_host(target, owner);
    }
    with_host_context_mut((), |context| {
        let retain_living_owner_view = owner_player.is_some_and(|owner| {
            context
                .object_scope(target)
                .is_some_and(|scope| scope.category() & crate::CATEGORY_LIVING != 0)
                && context.world.player_has_fow_view_object(owner, target)
        });
        let still_in_crew = context.object_in_any_crew(target);
        if let Some(scope) = context.object_scope_mut(target) {
            scope.stage_crew_member_state(still_in_crew);
        }
        if !retain_living_owner_view {
            context.set_object_plr_view_range(target, 0);
        }
        context.record_crew_rosters();
    });

    let call_death = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.object_status_present(target))
    });
    if call_death {
        call_object_own_fail_safe(target, "Death", &[Value::Int(death_causing_player)]);
    }
    with_host_context_mut((), |context| {
        if let Some(scope) = context.object_scope_mut(target) {
            scope.commit_raw_alive();
            scope.persist_final_ocf = true;
        }
        let _ = refresh_live_object_ocf(context, target);
    });
    Ok(true)
}

/// FnPunch (C4Script.cpp:328-332) → ObjectComPunch (C4ObjectCom.cpp:
/// 735-767): a zero punch derives from the Fight physicals
/// (BoundBy(5*attacker/target, 0, 10)); QueryCatchBlow on the target
/// halves punch > 1 and stops the blow; the target loses punch% energy
/// (DoEnergy with the ATTACKER's controller as caused-by, kill-trace
/// marked) and its ComDir stops either way; a stopped blow returns false
/// without a fling; punch >= 10 tries the Tumble action (xdir
/// FIXED100(150)*tdir, ydir -2), the regular path GetPunched (xdir
/// FIXED100(250)*tdir, ydir 0) — each re-writing
/// LastEnergyLossCausePlayer unguarded and firing
/// CatchBlow(punch, attacker) on success.
pub(crate) fn punch(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(target) =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Punch", "target")?
    else {
        return Ok(Value::Bool(false)); // !pTarget (C4ObjectCom.cpp:737)
    };
    let mut punch = parse_optional_i32(args.get(1), "Punch", "punch")?.unwrap_or(0);

    // Resolve only the physicals the native zero-punch branch actually
    // touches, in native order: target guard, attacker numerator, target
    // denominator (C4ObjectCom.cpp:738-740).
    let attacker = with_host_context(None, |context| {
        context.object_context().map(ObjectScopeContext::id)
    });
    let Some(attacker) = attacker else {
        return Ok(Value::Bool(false));
    };
    let target_exists = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        borrow
            .as_mut()
            .is_some_and(|context| context.ensure_object_scope(target))
    });
    if !target_exists {
        return Ok(Value::Bool(false)); // !cthr->Obj / unknown target
    }
    if punch == 0 {
        let Some(target_guard) =
            resolve_object_physical(target, false).map(|physical| physical.fight)
        else {
            return Ok(Value::Bool(false));
        };
        if target_guard != 0 {
            let Some(attacker_fight) =
                resolve_object_physical(attacker, false).map(|physical| physical.fight)
            else {
                return Ok(Value::Bool(false));
            };
            let Some(target_fight) =
                resolve_object_physical(target, false).map(|physical| physical.fight)
            else {
                return Ok(Value::Bool(false));
            };
            if target_fight != 0 {
                punch = (5 * attacker_fight / target_fight).clamp(0, 10);
            }
        }
    }
    if punch == 0 {
        return Ok(Value::Bool(true)); // nothing to do (C4ObjectCom.cpp:741)
    }

    // PSF_QueryCatchBlow (fail-safe; callee errors log and read as false,
    // C4Object::Call fPassErrors=false).
    let blow_stopped = match call_world_object_own_function(
        target,
        "QueryCatchBlow",
        &[object_reference_value(attacker)],
    ) {
        Some(Ok(value)) => value.as_bool(),
        Some(Err(error)) => {
            tracing::error!(
                %error,
                "script error in QueryCatchBlow; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
            false
        }
        None => false,
    };
    if blow_stopped && punch > 1 {
        punch /= 2; // caught blow halves damage (C4ObjectCom.cpp:743)
    }

    // DoEnergy(-punch, false, C4FxCall_EngGetPunched, cObj->Controller)
    // reads the attacker controller after QueryCatchBlow. The target's
    // physical Energy lookup may itself run the first fair-crew fill.
    let attacker_controller = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(attacker))
            .map(ObjectScopeContext::controller)
            .unwrap_or(OWNER_NONE)
    });
    let energy_result = do_energy_with_cause_override(
        &[
            Value::Int(-punch),
            object_reference_value(target),
            Value::Bool(false),
            Value::Int(crate::C4FX_CALL_ENG_GET_PUNCHED),
        ],
        Some(attacker_controller),
    )?;
    if !energy_result.as_bool() {
        return Ok(Value::Bool(false));
    }
    // Native reads the attacker's facing only after DoEnergy returned, then
    // stops the target's command direction (C4ObjectCom.cpp:744-746).
    let tdir_set = with_host_context_mut(None, |context| {
        let tdir = match context.object_scope(attacker)?.direction() {
            Direction::Left => -1,
            _ => 1,
        };
        context
            .object_scope_mut(target)?
            .set_command_direction(CommandDirection::Stop);
        Some(tdir)
    });
    let Some(tdir) = tdir_set else {
        return Ok(Value::Bool(false));
    };
    if blow_stopped {
        return Ok(Value::Bool(false)); // no tumbles for caught blows
    }

    let try_fling = |action: &str, velocity: FixedVec2| -> bool {
        let action_set = matches!(
            call_world_object_function(
                target,
                "SetAction",
                &[Value::String(action.to_string().into())],
            ),
            Some(Ok(value)) if value.as_bool()
        );
        if !action_set {
            return false;
        }
        with_host_context_mut(false, |context| {
            context
                .object_scope_mut(target)
                .map(|scope| scope.set_fixed_velocity(velocity))
                .is_some()
        })
    };
    let flung = (punch >= 10
        && try_fling("Tumble", FixedVec2::new(fixed100(150) * tdir, itofix(-2))))
        || try_fling(
            "GetPunched",
            FixedVec2::new(fixed100(250) * tdir, C4Fixed::ZERO),
        );
    if !flung {
        return Ok(Value::Bool(false));
    }
    // A successful fling writes the kill trace DIRECTLY — no
    // UpdatLastEnergyLossCause guard ("for kill tracing when pushing
    // enemies off a cliff", C4ObjectCom.cpp:755,762).
    let attacker_controller = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(attacker))
            .map(ObjectScopeContext::controller)
            .unwrap_or(OWNER_NONE)
    });
    with_host_context_mut((), |context| {
        if let Some(scope) = context.object_scope_mut(target) {
            scope.pending_update.energy_loss_cause = Some(attacker_controller);
        }
    });
    // PSF_CatchBlow after a successful fling (C4ObjectCom.cpp:754,762).
    // C4Object::Call first checks raw Status, which a synchronous lethal
    // DoEnergy/Death callback may have cleared before Punch reaches here.
    if object_is_present(target) {
        if let Some(Err(error)) = call_world_object_own_function(
            target,
            "CatchBlow",
            &[Value::Int(punch), object_reference_value(attacker)],
        ) {
            tracing::error!(
                %error,
                "script error in CatchBlow; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
        }
    }
    Ok(Value::Bool(true))
}

/// FnSetSolidMask (C4Script.cpp:271-278): sets the object's SolidMask
/// rect (x,y,wdt,hgt,tx,ty); a zero-area rect disables the mask (gates
/// open/close through this from UpdateTransferZone handlers).
pub(crate) fn set_solid_mask(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut values = [0i32; 6];
    for (i, slot) in values.iter_mut().enumerate() {
        *slot = parse_optional_i32(args.get(i), "SetSolidMask", "rect")?.unwrap_or(0);
    }
    let target =
        parse_object_reference_argument(args.get(6).unwrap_or(&Value::Nil), "SetSolidMask", "obj")?;
    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Bool(false));
        };
        let rect = crate::DefinitionTargetRect::new(
            values[0], values[1], values[2], values[3], values[4], values[5],
        );
        if context.object_scope(target).is_none() && !context.ensure_object_scope(target) {
            tracing::debug!(
                target = target.as_u64(),
                "SetSolidMask: unknown target; skipped"
            );
            return Ok(Value::Bool(false));
        }
        let rect = context
            .check_solid_mask_rect_for_object(target, rect)
            .unwrap_or(rect);
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        object.set_solid_mask_rect(rect);
        context.update_live_solid_mask(target, true);
        Ok(Value::Bool(true))
    })
}

/// FnSetVisibility (C4Script.cpp:3860-3869): write the target object's raw
/// VIS_* bitmask; a null target defaults to the calling object.
pub(crate) fn set_visibility(args: &[Value]) -> Result<Value, RuntimeError> {
    let visibility = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetVisibility",
        "visibility",
    )?;
    let target = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetVisibility", "object"))
        .transpose()?
        .flatten();
    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(target) = target.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Bool(false));
        };
        Ok(Value::Bool(
            context.set_object_visibility(target, visibility),
        ))
    })
}

/// FnSetClrModulation (C4Script.cpp:3880-3901): set either the object's
/// `ColorMod` or one existing overlay's modulation. A null object defaults
/// to the calling object; an unknown overlay fails without creating it.
pub(crate) fn set_clr_modulation(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "SetClrModulation expects at most 3 arguments: color, object and overlay id",
        ));
    }
    let color = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetClrModulation",
        "clr",
    )? as u32;
    let target = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetClrModulation", "object"))
        .transpose()?
        .flatten();
    let overlay_id = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "SetClrModulation",
        "overlay id",
    )?;

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(target) = target.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Bool(false));
        };
        let changed = if overlay_id == 0 {
            context.set_object_color_modulation(target, color)
        } else {
            context.set_object_overlay_color_modulation(target, overlay_id, color)
        };
        Ok(Value::Bool(changed))
    })
}

/// FnGetClrModulation (C4Script.cpp:3904-3921): return the raw object or
/// overlay modulation; a missing object/overlay is nil.
pub(crate) fn get_clr_modulation(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetClrModulation expects at most 2 arguments: object and overlay id",
        ));
    }
    let target = args
        .first()
        .map(|value| parse_object_reference_argument(value, "GetClrModulation", "object"))
        .transpose()?
        .flatten();
    let overlay_id = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "GetClrModulation",
        "overlay id",
    )?;

    with_host_context(Ok(Value::Nil), |context| {
        let Some(target) = target.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Nil);
        };
        let color = if overlay_id == 0 {
            context.object_color_modulation(target)
        } else {
            context.object_overlay_color_modulation(target, overlay_id)
        };
        Ok(color
            .map(|color| Value::Int(color as i32))
            .unwrap_or(Value::Nil))
    })
}

/// FnModulateColor (C4Script.cpp:5597-5612): multiply packed RGB channels
/// with the engine's `/ 256` rule and combine inverted alpha upwards.
pub(crate) fn modulate_color(args: &[Value]) -> Result<Value, RuntimeError> {
    let color1 = parse_native_optional_i32(args.first(), "ModulateColor", "color 1")?
        .unwrap_or(0x00ff_ffff) as u32;
    let color2 = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "ModulateColor",
        "color 2",
    )? as u32;
    let channel = |shift: u32| -> u32 {
        ((((color1 >> shift) & 0xff_u32) * ((color2 >> shift) & 0xff_u32)) >> 8) << shift
    };
    let alpha1 = color1 >> 24;
    let alpha2 = color2 >> 24;
    let alpha = (alpha1 + alpha2 - ((alpha1 * alpha2) >> 8)).min(0xff);
    Ok(Value::Int(
        (channel(0) | channel(8) | channel(16) | (alpha << 24)) as i32,
    ))
}

pub(crate) fn live_object_bounds_shape(
    context: &EffectHostContext,
    target: ObjectId,
) -> Option<DefinitionRect> {
    if let Some(shape) = live_object_shape(context, target) {
        return Some(shape);
    }
    let object = context.get_world_object(target)?;
    let vertices = context
        .object_scope(target)
        .map(|scope| scope.vertices().to_vec())
        .unwrap_or_else(|| object.vertices.clone());
    host_vertex_bounds_rect(Vector2::ZERO, &vertices)
}

/// The stop-and-call half of C4Object::TargetBounds (C4Movement.cpp:135-145,
/// 152-162): zero xdir for CNAT_Left/Right or ydir otherwise, then
/// C4Object::Contact — which dispatches only under `Def->ContactFunctionCalls`.
pub(crate) fn run_live_bound_contact(target: ObjectId, cnat: u32) {
    let calls_enabled = with_host_context_mut(false, |context| {
        let definition_id = context.object_effective_definition_id(target);
        let calls_enabled = definition_id
            .as_deref()
            .and_then(|id| context.definition_metadata(id))
            .is_some_and(|metadata| metadata.contact_function_calls);
        if !context.ensure_object_scope(target) {
            return false;
        }
        if let Some(scope) = context.object_scope_mut(target) {
            let component = if cnat == CNAT_LEFT || cnat == CNAT_RIGHT {
                VelocityComponent::X
            } else {
                VelocityComponent::Y
            };
            scope.set_fixed_velocity_component(component, C4Fixed::ZERO);
            return calls_enabled && scope.status() != ObjectStatus::Deleted;
        }
        false
    });
    if calls_enabled {
        if let Some(function) = crate::contact_callback_name(cnat) {
            let _ = call_object_own_fail_safe(target, function, &[]);
        }
    }
}

/// FnGetEnergy: `100 * Energy / C4MaxPhysical` — scripts always read
/// percent of the raw physical scale (C4Script.cpp FnGetEnergy).
fn energy_to_script_value(energy: i32) -> i32 {
    ((energy as i64) * 100 / (LEGACY_MAX_PHYSICAL as i64)) as i32
}

pub(crate) const LEGACY_MAX_PHYSICAL: i32 = 100_000;
const CONTACT_DIRECTION_MASK: u32 = CNAT_LEFT | CNAT_RIGHT | CNAT_TOP | CNAT_BOTTOM | CNAT_CENTER;

pub(crate) fn compute_vertex_contact(
    position: Vector2,
    vertex: &ObjectVertex,
    check_mask: u32,
    contact_density: i32,
    mut density_at: impl FnMut(i32, i32) -> Option<i32>,
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
    let world_x = position.x.saturating_add(vertex.x);
    let world_y = position.y.saturating_add(vertex.y);
    let mut has_contact = |x, y| density_at(x, y).is_some_and(|density| density >= contact_density);
    let mut contact = 0;
    if (mask & CNAT_CENTER) != 0 && has_contact(world_x, world_y) {
        contact |= CNAT_CENTER;
    }
    if (mask & CNAT_LEFT) != 0 && has_contact(world_x - 1, world_y) {
        contact |= CNAT_LEFT;
    }
    if (mask & CNAT_RIGHT) != 0 && has_contact(world_x + 1, world_y) {
        contact |= CNAT_RIGHT;
    }
    if (mask & CNAT_TOP) != 0 && has_contact(world_x, world_y - 1) {
        contact |= CNAT_TOP;
    }
    if (mask & CNAT_BOTTOM) != 0 && has_contact(world_x, world_y + 1) {
        contact |= CNAT_BOTTOM;
    }
    contact
}

fn resolve_contact_density(context: &EffectHostContext, target: Option<ObjectId>) -> i32 {
    target
        .or_else(|| context.object_context().map(|object| object.id()))
        .and_then(|target| {
            context
                .object_scope(target)
                .map(ObjectScopeContext::contact_density)
                .or_else(|| {
                    context
                        .get_world_object(target)
                        .map(|object| object.contact_density())
                })
        })
        .unwrap_or(crate::CONTACT_DENSITY_SOLID)
}

fn resolve_vertices(
    context: &EffectHostContext,
    target: Option<ObjectId>,
) -> Option<(Vector2, Vec<ObjectVertex>)> {
    if let Some(target_id) = target {
        if let Some(object) = context.object_context() {
            if object.id() == target_id {
                return Some((object.effective_position(), object.vertices().to_vec()));
            }
        }
        // Foreign targets read THROUGH the mid-call staging (a scope's
        // SetPosition/SetVertex writes) like the live C4Object the C++
        // engine hands to Shape.CheckContact (FnStuck, C4Script.cpp:1858).
        context
            .get_world_object(target_id)
            .map(|other| (other.position(), other.vertices().to_vec()))
    } else {
        context
            .object_context()
            .map(|object| (object.effective_position(), object.vertices().to_vec()))
    }
}

pub(crate) const DEFAULT_MAX_ENERGY: i32 = 100;
pub(crate) const DEFAULT_VELOCITY_PRECISION: i32 = 10;

/// FnDeathAnnounce (C4Script.cpp:303-318): Film suppresses the announcement,
/// a live crew Info supplies a verbatim custom message, and only the fallback
/// draws one of seven localized object messages with SafeRandom.
pub(crate) fn active_death_message(value: &str) -> Option<String> {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes.truncate(75);
    (!bytes.is_empty()).then(|| clonk_script::c4_string_from_bytes(&bytes))
}

pub(crate) fn death_announce(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new("DeathAnnounce expects no arguments"));
    }

    // `script_object_context` is cthr->Obj. The mutable carrier scope can
    // still exist for a definition-owned callback whose script object is
    // null, and must not make DeathAnnounce succeed.
    let state = with_host_context(None, |context| {
        let target = context.script_object_context?;
        let film = matches!(
            context.world.scenario_value("Film", Some("Head"), 0),
            Some(ScenarioValue::Int(value)) if *value != 0
        );
        if film {
            return Some((target, true, None));
        }
        // An instantiated scope is authoritative even when a same-call
        // GrabObjectInfo/retire operation cleared or replaced its Info.
        let info = match context.object_scope(target) {
            Some(scope) => scope.info_core(),
            None => context.world.crew_infos.get(&target),
        };
        Some((
            target,
            false,
            info.and_then(|info| active_death_message(&info.death_message)),
        ))
    });
    let Some((target, film, custom_message)) = state else {
        return Ok(Value::Bool(false));
    };
    if film {
        return Ok(Value::Bool(true));
    }

    let text = match custom_message {
        Some(message) => message,
        None => {
            let choice = SCRIPT_SAFE_RNG.with(|rng| rng.borrow_mut().random(7)) as usize;
            let name = match get_name(&[Value::Object(target.as_u64())])? {
                Value::String(name) => name,
                _ => return Ok(Value::Bool(false)),
            };
            // planet/System.c4g/LanguageUS.txt IDS_OBJ_DEATH1..7.
            const DEFAULT_MESSAGES: [&str; 7] = [
                "{name} is dead.",
                "{name} has|deceased.",
                "{name}|rests in peace.",
                "{name} is dead.",
                "{name} has|deceased.",
                "{name}|rests in peace.",
                "{name} is dead.",
            ];
            DEFAULT_MESSAGES[choice].replace("{name}", &name)
        }
    };

    try_with_host_context_mut(
        "DeathAnnounce requires an active engine context",
        |context| {
            context.register_message(MessageCommand::Add(
                MessageSpec::target(text, target)
                    .with_color(invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR)),
            ));
            Ok(Value::Bool(true))
        },
    )
}

/// FnSimFlight (C4Script.cpp:5309-5330) and SimFlight
/// (C4Movement.cpp:623-653). The first four native C4Value* arguments are
/// nullable references; simulation runs on local fixed-point copies so a
/// bounds/iteration failure leaves every caller variable untouched.
pub(crate) fn sim_flight(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    if (0..4).any(|index| !args.get(index).is_some_and(HostCallArg::is_reference)) {
        return Err(RuntimeError::new(
            "SimFlight: first four arguments must be variable references",
        ));
    }

    let values = args
        .iter()
        .map(HostCallArg::read)
        .collect::<Result<Vec<_>, _>>()?;
    // FnSimFlight calls getInt() on the referenced C4Values after native
    // dispatch has validated only that they are references. getInt() returns
    // zero when the referenced value cannot convert to Int.
    let read_int = |index: usize| values.get(index).and_then(Value::as_c4_int).unwrap_or(0);
    let optional_int = |index: usize, parameter: &str| {
        parse_native_optional_i32(values.get(index), "SimFlight", parameter)
    };
    let density_min = optional_int(4, "density_min")?.unwrap_or(50);
    let density_max = optional_int(5, "density_max")?.unwrap_or(100);
    let mut iterations = optional_int(6, "iterations")?.unwrap_or(-1);
    let precision = optional_int(7, "precision")?.unwrap_or(10);
    if precision == 0 {
        return Err(RuntimeError::new("SimFlight: precision must not be zero"));
    }

    let mut x = itofix(read_int(0));
    let mut y = itofix(read_int(1));
    let xdir = itofix_prec(read_int(2), precision);
    let mut ydir = itofix_prec(read_int(3), precision);
    let gravity = PHYSICS_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| fixed100(context.gravity()) / 5)
            .ok_or_else(|| RuntimeError::new("SimFlight requires an active physics context"))
    })?;

    let succeeded = HOST_CONTEXT.with(|cell| -> Result<bool, RuntimeError> {
        let borrow = cell.borrow();
        let context = borrow
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SimFlight requires an active engine context"))?;
        let landscape = context
            .world
            .landscape_ref()
            .ok_or_else(|| RuntimeError::new("SimFlight requires an active landscape"))?;
        let width = landscape.width() as i32;
        let height = landscape.estimated_height();
        let mut cx = fixtoi(x);
        let mut cy = fixtoi(y);

        loop {
            if iterations == 0 {
                return Ok(false);
            }
            iterations = iterations.wrapping_sub(1);
            x += xdir;
            y += ydir;
            let target_x = fixtoi(x);
            let target_y = fixtoi(y);
            // Inside(target_x, 0, GBackWdt) is inclusive in both bounds;
            // C++ only rejects the lower landscape vertically.
            if !(0..=width).contains(&target_x) || target_y >= height {
                return Ok(false);
            }

            let contact = loop {
                cx += (target_x - cx).signum();
                cy += (target_y - cy).signum();
                let density = context.world.movement_density_at(cx, cy).unwrap_or(0);
                if (density_min..=density_max).contains(&density) {
                    break true;
                }
                if cx == target_x && cy == target_y {
                    break false;
                }
            };
            // GravAccel is adjusted for every completed movement frame,
            // including the frame that first contacts the density.
            ydir += gravity;
            if contact {
                x = itofix(cx);
                y = itofix(cy);
                return Ok(true);
            }
        }
    })?;

    if !succeeded {
        return Ok(Value::Bool(false));
    }
    let output = [
        Value::Int(fixtoi(x)),
        Value::Int(fixtoi(y)),
        Value::Int(fixtoi(xdir * precision)),
        Value::Int(fixtoi(ydir * precision)),
    ];
    for (index, value) in output.into_iter().enumerate() {
        let wrote = args[index].write(value)?;
        debug_assert!(wrote, "validated SimFlight reference disappeared");
    }
    Ok(Value::Bool(true))
}

/// FnGetMass (C4Script.cpp:1148-1158): with an id, the DEF mass; else the
/// object's Mass = max(Def->Mass * Con / FullCon, 1)
/// (C4Object.cpp:188; OwnMass/contents mass unmodeled). Nil without both.
pub(crate) fn get_mass(args: &[Value]) -> Result<Value, RuntimeError> {
    if std::env::var("LC_MASSDBG").is_ok() {
        let peek = parse_native_c4id_argument(args.get(1), "GetMass")
            .ok()
            .flatten();
        let mass = peek.as_ref().and_then(|d| {
            with_host_context(None, |c| c.world.definition_metadata(d).map(|m| m.mass))
        });
        eprintln!("MASSDBG GetMass args={args:?} -> {mass:?}");
    }
    let definition = parse_native_c4id_argument(args.get(1), "GetMass")?;
    if let Some(definition) = definition {
        return HOST_CONTEXT.with(|cell| {
            Ok(cell
                .borrow()
                .as_ref()
                .and_then(|context| context.world.definition_metadata(&definition))
                .map(|metadata| Value::Int(metadata.mass))
                .unwrap_or(Value::Nil))
        });
    }
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetMass", "obj")?;
    with_host_context(Ok(Value::Nil), |context| {
        let id = target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(id) = id else {
            return Ok(Value::Nil);
        };
        // UpdateMass: Mass = max((Def->Mass + OwnMass) * Con / FullCon, 1)
        // (C4Object.cpp:497-500); the active scope has the freshest OwnMass.
        if context.get_world_object(id).is_none() {
            return Ok(Value::Nil);
        }
        Ok(Value::Int(reflected_object_mass(
            context,
            id,
            &mut HashSet::new(),
        )))
    })
}

/// FnSetMass (C4Script.cpp:3620-3626): OwnMass = value - Def->Mass, then
/// UpdateMass; foreign targets mutate in their own scope.
pub(crate) fn set_mass(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = parse_optional_i32(args.first(), "SetMass", "mass")?.unwrap_or(0);
    let target =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "SetMass", "obj")?;
    let active = with_host_context(None, |context| {
        context.object_context().map(|object| object.id())
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(target, "SetMass", &[Value::Int(value)]) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(id) = context.object_context().map(|object| object.id()) else {
            return Ok(Value::Bool(false));
        };
        // The scope's own definition id — a synchronous Initialize (the
        // ArrowPack UpdateMass at CreateContents) runs before the object
        // reaches the world/pending views, so get_world_object misses it
        // and OwnMass = iValue - Def->Mass (C4Script.cpp:3627-3631)
        // silently lost the Def->Mass term.
        let def_mass = context
            .object_context()
            .and_then(|object| object.definition_id.clone())
            .and_then(|definition_id| {
                context
                    .world
                    .definition_metadata(&definition_id)
                    .map(|metadata| metadata.mass)
            })
            .or_else(|| {
                context
                    .get_world_object(id)
                    .and_then(|object| context.world.definition_metadata(object.definition_id()))
                    .map(|metadata| metadata.mass)
            })
            .unwrap_or(0);
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        object.set_own_mass(value - def_mass);
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetEnergy expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetEnergy", "target")?;
    }

    with_host_context(Ok(Value::Nil), |context| {
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

/// FnDoBreath (C4Script.cpp:502-506) and C4Object::DoBreath
/// (C4Object.cpp:1406-1413): default the target to the calling object,
/// scale script points by C4MaxPhysical/100, then clamp the live raw value
/// into 0..GetPhysical()->Breath.
pub(crate) fn do_breath(args: &[Value]) -> Result<Value, RuntimeError> {
    let change = value_to_i32(args.first().unwrap_or(&Value::Nil), "DoBreath", "change")?;
    let target_id =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "DoBreath", "target")?;
    let target =
        try_with_host_context_mut("DoBreath requires an active engine context", |context| {
            let Some(target) =
                target_id.or_else(|| context.object_context().map(|object| object.id()))
            else {
                return Ok(None);
            };
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
            Ok(Some(target))
        })?;
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    let Some(capacity) =
        resolve_object_physical(target, false).map(|physical| physical.breath.max(0))
    else {
        return Ok(Value::Bool(false));
    };
    try_with_host_context_mut("DoBreath requires an active engine context", |context| {
        let Some(scope) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        let scaled = change.saturating_mul(LEGACY_MAX_PHYSICAL / 100);
        let breath = scope.breath().saturating_add(scaled).clamp(0, capacity);
        scope.set_breath(breath);
        Ok(Value::Bool(true))
    })
}

/// FnGetBreath (C4Script.cpp:1143-1146): `100 * Breath / C4MaxPhysical`.
/// A scope-local staged write wins over the frame-start world snapshot, as
/// C++ mutates C4Object::Breath synchronously.
pub(crate) fn get_breath(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetBreath", "target")?;
    with_host_context(Ok(Value::Nil), |context| {
        let target = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        let breath = context
            .object_scope(target)
            .map(ObjectScopeContext::breath)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.breath))
            });
        match breath {
            Some(breath) => Ok(Value::Int(
                (100i64 * i64::from(breath) / i64::from(LEGACY_MAX_PHYSICAL)) as i32,
            )),
            None => Ok(Value::Nil),
        }
    })
}

/// FnGetName (C4Script.cpp:992-1005): the definition Name for an id
/// argument; otherwise CustomName, live Info name, then definition name.
pub(crate) fn get_name(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|value| parse_object_reference_argument(value, "GetName", "target"))
        .transpose()?
        .flatten();
    let def_id = parse_native_c4id_argument(args.get(1), "GetName")?.filter(|id| !id.is_empty());
    with_host_context(Ok(Value::Nil), |context| {
        if let Some(definition) = def_id {
            return Ok(context
                .definition_metadata(&definition)
                .map(|metadata| Value::String(metadata.name.clone().into()))
                .unwrap_or(Value::Nil));
        }
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Nil);
        };
        Ok(context
            .object_effective_name(target)
            .map(|name| Value::String(name.into()))
            .unwrap_or(Value::Nil))
    })
}

/// FnGetDesc (C4Script.cpp:1063-1076): an explicit object wins over the
/// definition argument. The caller object is used only when both slots are
/// nil/zero; definition-only script contexts do not supply a fallback.
pub(crate) fn get_desc(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetDesc", "object")?;
    let definition =
        parse_native_c4id_argument(args.get(1), "GetDesc")?.filter(|id| !id.is_empty());
    with_host_context(Ok(Value::Nil), |context| {
        let definition = match (target, definition) {
            (Some(target), _) => context.object_effective_definition_id(target),
            (None, Some(definition)) => Some(definition),
            (None, None) => context
                .script_object_context
                .and_then(|target| context.object_effective_definition_id(target)),
        };
        Ok(definition
            .as_deref()
            .and_then(|id| context.world.definition_description(id))
            .map(|description| Value::String(description.to_owned().into()))
            .unwrap_or(Value::Nil))
    })
}

pub(crate) const C4_MAX_NAME_BYTES: usize = 30;

/// FnSetName (C4Script.cpp:1008-1061): rename a definition, persist a crew
/// info name with owner-list duplicate handling, or set an object's transient
/// CustomName. A missing object defaults to the calling object.
pub(crate) fn set_name(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 5 {
        return Err(RuntimeError::new(
            "SetName expects at most 5 arguments: name, object, definition, set-in-info, make-valid",
        ));
    }
    let requested_name = parse_optional_string(args.first(), "SetName", "name")?;
    let target_id = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetName", "target"))
        .transpose()?
        .flatten();
    let definition =
        parse_native_c4id_argument(args.get(2), "SetName")?.filter(|id| !id.is_empty());
    let set_in_info = args
        .get(3)
        .map(|value| value_to_bool(value, "SetName", "set-in-info"))
        .transpose()?
        .unwrap_or(false);
    let make_valid = args
        .get(4)
        .map(|value| value_to_bool(value, "SetName", "make-valid"))
        .transpose()?
        .unwrap_or(false);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        if set_in_info && definition.is_some() {
            return Ok(Value::Bool(false));
        }
        if let Some(definition) = definition {
            return Ok(Value::Bool(context.set_definition_name(
                definition,
                requested_name.unwrap_or_default(),
            )));
        }
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Bool(false));
        };

        if set_in_info {
            if !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }
            let Some((owner, link, old_name)) = context.object_scope(target).and_then(|scope| {
                Some((
                    scope.owner(),
                    scope.info_link(),
                    scope.info_core()?.name.clone(),
                ))
            }) else {
                return Ok(Value::Bool(false));
            };
            let Some(requested_name) = requested_name else {
                return Ok(Value::Bool(false));
            };
            if requested_name.is_empty()
                || clonk_script::c4_string_byte_len(&requested_name) > C4_MAX_NAME_BYTES
            {
                return Ok(Value::Bool(false));
            }
            if clonk_script::c4_strings_equal(&requested_name, &old_name) {
                return Ok(Value::Bool(true));
            }

            let owner_names = if context.player_state(owner).is_some() {
                context
                    .world
                    .crew_info_state
                    .borrow()
                    .roster_names
                    .get(&owner)
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let duplicate = owner_names.iter().any(|name| {
                c4_bytes_equal_no_case(
                    &clonk_script::c4_string_bytes(name),
                    &clonk_script::c4_string_bytes(&requested_name),
                )
            });
            if duplicate && !make_valid {
                return Ok(Value::Bool(false));
            }
            let final_name = if duplicate {
                make_valid_crew_name(&requested_name, &owner_names)
            } else {
                requested_name
            };

            let Some(scope) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };
            let Some(mut info) = scope.info_core().cloned() else {
                return Ok(Value::Bool(false));
            };
            info.name = final_name.clone();
            scope.set_info_core(Some(info));
            // pObj->SetName() with no argument adopts Info->Name by clearing
            // any transient CustomName override.
            scope.pending_update.custom_name = Some(None);
            if let Some(link) = link {
                let mut state = context.world.crew_info_state.borrow_mut();
                if let Some(entry) = state.entries.get_mut(&link) {
                    entry.name = final_name.clone();
                }
                if let Some(name) = state
                    .roster_names
                    .get_mut(&link.player_id)
                    .and_then(|names| names.get_mut(link.roster_index))
                {
                    *name = final_name.clone();
                }
                for entries in state.idle.values_mut() {
                    for (candidate, entry) in entries {
                        if *candidate == link {
                            entry.name = final_name.clone();
                        }
                    }
                }
            }
            context.record_player_command(PlayerCommand::SetCrewInfoName {
                object_id: target,
                link,
                name: final_name,
            });
            return Ok(Value::Bool(true));
        }

        let custom_name = requested_name.filter(|name| !name.is_empty());
        Ok(Value::Bool(
            context.set_object_custom_name(target, custom_name),
        ))
    })
}

pub(crate) fn get_con(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetCon expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetCon", "target")?;
    }

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_scope(target) {
                return Ok(Value::Int(construction_to_script_value(
                    object.construction(),
                )));
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(construction_to_script_value(
                    other.construction(),
                )));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(construction_to_script_value(
            object.construction(),
        )))
    })
}

/// A physical name argument (`FnStringPar`): None for Nil/absent/empty.
fn physical_name_argument(
    args: &[Value],
    index: usize,
    fn_name: &str,
) -> Result<Option<String>, RuntimeError> {
    match args.get(index) {
        Some(Value::String(name)) if !name.is_empty() => Ok(Some(name.as_ref().to_owned())),
        Some(Value::String(_)) | Some(Value::Nil) | None => Ok(None),
        Some(other) => Err(RuntimeError::new(format!(
            "{fn_name}: expected string for physical name, got {}",
            other.type_name()
        ))),
    }
}

/// Resolve a live object's physicals without retaining a borrow of the host
/// TLS across a possible first-fill definition callback.
pub(crate) fn resolve_object_physical(target: ObjectId, permanent: bool) -> Option<PhysicalInfo> {
    let resolution = with_host_context(None, |context| {
        context.prepare_object_physical(target, permanent)
    });
    resolution.map(PhysicalResolution::resolve)
}

/// `FnGetPhysical` (C4Script.cpp:638-688): `GetPhysical(name, mode, obj,
/// id)`. The def form reads the definition's `[Physical]` section; object
/// reads resolve against the explicitly targeted live object.
pub(crate) fn get_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let name = physical_name_argument(args, 0, "GetPhysical")?;
    let mode = int_argument(args, 1, "GetPhysical")?;
    let target_id = args
        .get(2)
        .map(|arg| parse_object_reference_argument(arg, "GetPhysical", "target"))
        .transpose()?
        .flatten();
    let definition_id =
        parse_native_c4id_argument(args.get(3), "GetPhysical")?.filter(|id| !id.is_empty());
    let Some(name) = name else {
        return Ok(Value::Nil);
    };
    // No object given: a def id reads the definition physicals
    // (C4Script.cpp:644-653). This path cannot enter fair-crew projection.
    if target_id.is_none() {
        if let Some(definition_id) = definition_id.as_deref() {
            return Ok(HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.definition_metadata(definition_id))
                    .and_then(|metadata| metadata.physical.value_by_name(&name))
                    .map(Value::Int)
                    .unwrap_or(Value::Nil)
            }));
        }
    }

    let resolution = with_host_context(None, |context| {
        let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let target = target?;
        if let Some(object) = context.object_scope(target) {
            return object.prepare_get_physical(mode);
        }
        // An explicit pObj is not constrained to cthr->Obj: FnGetPhysical
        // dereferences that live object directly (C4Script.cpp:638-688).
        // Build the same read-only scope used for nested object calls so
        // crew info and temporary physicals retain their normal precedence.
        context
            .get_world_object(target)
            .and_then(|object| context.nested_scope_for(&object))
            .and_then(|(object, _)| object.prepare_get_physical(mode))
    });
    Ok(resolution
        .map(PhysicalResolution::resolve)
        .and_then(|physical| physical.value_by_name(&name))
        .map(Value::Int)
        .unwrap_or(Value::Nil))
}

/// `FnSetPhysical` (C4Script.cpp:557-601): `SetPhysical(name, value, mode,
/// obj)`.
pub(crate) fn set_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(name) = physical_name_argument(args, 0, "SetPhysical")? else {
        return Ok(Value::Bool(false));
    };
    if PhysicalInfo::default().value_mut_by_name(&name).is_none() {
        return Ok(Value::Bool(false));
    }
    let value = int_argument(args, 1, "SetPhysical")?;
    let mode = int_argument(args, 2, "SetPhysical")?;
    let target_id = args
        .get(3)
        .map(|arg| parse_object_reference_argument(arg, "SetPhysical", "target"))
        .transpose()?
        .flatten();

    let prepared =
        try_with_host_context_mut("SetPhysical requires an active engine context", |context| {
            let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
            let Some(target) = target else {
                return Ok(None);
            };
            if context.object_scope(target).is_none() && !context.ensure_object_scope(target) {
                return Ok(None);
            }
            let Some(object) = context.object_scope_mut(target) else {
                return Ok(None);
            };
            let base = matches!(mode, PHYS_TEMPORARY | PHYS_STACK_TEMPORARY)
                .then(|| {
                    object
                        .temporary_physical
                        .is_none()
                        .then(|| object.prepare_resolved_physical(false))
                })
                .flatten();
            Ok(Some((target, base)))
        })?;
    let Some((target, base)) = prepared else {
        return Ok(Value::Bool(false));
    };
    let base = base.map(PhysicalResolution::resolve);
    try_with_host_context_mut("SetPhysical requires an active engine context", |context| {
        Ok(Value::Bool(context.object_scope_mut(target).is_some_and(
            |object| object.set_physical(&name, value, mode, base),
        )))
    })
}

/// `FnTrainPhysical` (C4Script.cpp:603-611): `TrainPhysical(name, by, max,
/// obj)`.
pub(crate) fn train_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(name) = physical_name_argument(args, 0, "TrainPhysical")? else {
        return Ok(Value::Bool(false));
    };
    let train_by = int_argument(args, 1, "TrainPhysical")?;
    let max_train = int_argument(args, 2, "TrainPhysical")?;
    let target_id = args
        .get(3)
        .map(|arg| parse_object_reference_argument(arg, "TrainPhysical", "target"))
        .transpose()?
        .flatten();

    try_with_host_context_mut(
        "TrainPhysical requires an active engine context",
        |context| {
            let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
            let Some(target) = target else {
                return Ok(Value::Bool(false));
            };
            if context.object_scope(target).is_none() && !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }
            let (trained, info_writeback) = {
                let Some(object) = context.object_scope_mut(target) else {
                    return Ok(Value::Bool(false));
                };
                let has_info = object.has_physical_info();
                let trained = object.train_physical(&name, train_by, max_train);
                let info_writeback = (has_info && trained)
                    .then(|| object.info_link().zip(object.info_physical))
                    .flatten();
                (trained, info_writeback)
            };
            if let Some((link, physical)) = info_writeback {
                // The host mirrors the exact roster node so a later Retire,
                // GrabObjectInfo or MakeCrewMember in this same VM call sees the
                // trained values before the copied outcome reaches Engine.
                let mut state = context.world.crew_info_state.borrow_mut();
                if let Some(entry) = state.entries.get_mut(&link) {
                    entry.physical = physical;
                }
                for entries in state.idle.values_mut() {
                    for (candidate, entry) in entries {
                        if *candidate == link {
                            entry.physical = physical;
                        }
                    }
                }
                drop(state);
                context
                    .record_player_command(PlayerCommand::SetCrewInfoPhysical { link, physical });
            }
            Ok(Value::Bool(trained))
        },
    )
}

/// `FnResetPhysical` (C4Script.cpp:613-636): `ResetPhysical(obj, name)` —
/// the object comes FIRST in this one.
pub(crate) fn reset_physical(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "ResetPhysical", "target"))
        .transpose()?
        .flatten();
    let name = physical_name_argument(args, 1, "ResetPhysical")?;

    let prepared = try_with_host_context_mut(
        "ResetPhysical requires an active engine context",
        |context| {
            let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
            let Some(target) = target else {
                return Ok(None);
            };
            if context.object_scope(target).is_none() && !context.ensure_object_scope(target) {
                return Ok(None);
            }
            Ok(context
                .object_scope_mut(target)
                .map(|object| (target, object.begin_reset_physical(name.as_deref()))))
        },
    )?;
    let Some((target, step)) = prepared else {
        return Ok(Value::Bool(false));
    };
    match step {
        ResetPhysicalBegin::Complete(result) => return Ok(Value::Bool(result)),
        ResetPhysicalBegin::ComparePermanent => {}
    }
    let Some(reference) = resolve_object_physical(target, true) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(target))
            .is_some_and(|object| object.finish_reset_physical(reference))
    })))
}

pub(crate) fn do_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    do_energy_with_cause_override(args, None)
}

/// Internal engine callers already have the decoded player number and must
/// be able to pass explicit NO_OWNER; the public script ABI reserves encoded
/// zero for its caller-controller default.
pub(crate) fn do_energy_with_cause_override(
    args: &[Value],
    caused_by_override: Option<i32>,
) -> Result<Value, RuntimeError> {
    let change = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoEnergy: expected int or nil for change, got {}",
                other.type_name()
            )));
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Object(_) | Value::Proplist(_) => {
                target_id = object_id_from_value(arg);
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
                )));
            }
        }
    }

    let mut eng_type = 0;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                eng_type = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected int or nil for cause, got {}",
                    other.type_name()
                )));
            }
        }
    }
    // C4FxCall_EngScript default (C4Script.cpp:495).
    if eng_type == 0 {
        eng_type = crate::C4FX_CALL_ENG_SCRIPT;
    }

    let mut caused_by_plus_one = 0;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                caused_by_plus_one = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoEnergy: expected int or nil for caused by, got {}",
                    other.type_name()
                )));
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "DoEnergy: additional arguments are not supported",
        ));
    }

    let staged =
        try_with_host_context_mut("DoEnergy requires an active engine context", |context| {
            // iCausedBy = iCausedByPlusOne - 1, else the CALLER's controller
            // (C4Script.cpp:496-497) — resolved in the caller's scope.
            let caused_by = caused_by_override.unwrap_or_else(|| {
                if caused_by_plus_one != 0 {
                    caused_by_plus_one - 1
                } else {
                    context
                        .object_context()
                        .map(|object| object.controller())
                        .unwrap_or(OWNER_NONE)
                }
            });
            // `if (!pObj) pObj = cthr->Obj` is only the local-call default
            // (C4Script.cpp:494) — a named target may be FOREIGN.
            let Some(target) = target_id.or_else(|| context.object_context().map(|o| o.id()))
            else {
                return Ok(None);
            };
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
            // Kill-trace mark before the effects hook (C4Object.cpp:1351-1353).
            stage_energy_loss_cause(context, target, change, eng_type, caused_by);
            let Some(scope) = context.object_scope(target) else {
                return Ok(None);
            };
            Ok(Some((
                target,
                caused_by,
                scope.alive(),
                scope.energy() == 0,
            )))
        })?;
    let Some((target, caused_by, alive, was_zero)) = staged else {
        return Ok(Value::Bool(false));
    };
    // The percent scale precedes the effects hook (C4Object.cpp:1347 vs
    // :1355): living targets' hooks see the SCALED change, and a zero
    // chain outcome returns before the energy write (:1358).
    let scaled = if exact {
        change
    } else {
        change.saturating_mul(LEGACY_MAX_PHYSICAL / 100)
    };
    let scaled = match alive
        .then(|| dispatch_effects_do_damage(target, scaled, eng_type, caused_by))
        .flatten()
    {
        Some(0) => return Ok(Value::Bool(true)),
        Some(modified) => modified,
        None => scaled,
    };
    let Some(max_energy) = resolve_object_physical(target, false).map(|physical| physical.energy)
    else {
        return Ok(Value::Bool(false));
    };
    let should_assign_death = with_host_context_mut(false, |context| {
        if let Some(scope) = context.object_scope_mut(target) {
            let energy = scope.adjust_energy(scaled, true, max_energy);
            // This write has evaluated C++'s death predicate against the
            // same-call state. Copy-out must not derive it again from the
            // frame-entry Alive value.
            scope.pending_update.host_energy_death_checked = true;
            scope.alive() && energy == 0 && !was_zero
        } else {
            false
        }
    });
    if should_assign_death {
        let _ = assign_death_live(target, false)?;
    }
    Ok(Value::Bool(true))
}

/// `MagicPhysicalFactor` (C4Object.h:81): raw MagicEnergy units per
/// script-visible magic point.
const MAGIC_PHYSICAL_FACTOR: i32 = 1000;

/// Reads DoMagicEnergy/GetMagicEnergy's optional object slot: object
/// references, positive ints (object numbers), or nil/0 for the
/// caller-object default.
fn magic_energy_target(
    arg: Option<&Value>,
    function: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    match arg.unwrap_or(&Value::Nil) {
        value @ (Value::Object(_) | Value::Proplist(_)) => Ok(object_id_from_value(value)),
        Value::Nil | Value::Int(0) => Ok(None),
        Value::Int(value) if *value > 0 => Ok(Some(ObjectId::new(*value as u64))),
        other => Err(RuntimeError::new(format!(
            "{function}: expected object or nil for target, got {}",
            other.type_name()
        ))),
    }
}

/// `FnDoMagicEnergy` (C4Script.cpp:517-544): the change scales by
/// MagicPhysicalFactor; an overload (change > 0 past GetPhysical()->Magic)
/// or underload (change < 0 past zero) fails the call unless
/// fAllowPartial clamps it to the remaining headroom — a zero remainder
/// still fails. The result bounds into 0..GetPhysical()->Magic
/// (BoundBy, :541). ViewEnergy = C4ViewDelay (:542) is the magic-bar
/// display flash — presentation-only, not modeled.
pub(crate) fn do_magic_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    let change = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoMagicEnergy: expected int or nil for change, got {}",
                other.type_name()
            )));
        }
    };
    let target_id = magic_energy_target(args.get(1), "DoMagicEnergy")?;
    let allow_partial = match args.get(2).unwrap_or(&Value::Nil) {
        Value::Bool(flag) => *flag,
        Value::Int(value) => *value != 0,
        Value::Nil => false,
        other => {
            return Err(RuntimeError::new(format!(
                "DoMagicEnergy: expected bool or nil for allow-partial flag, got {}",
                other.type_name()
            )));
        }
    };

    let target = try_with_host_context_mut(
        "DoMagicEnergy requires an active engine context",
        |context| {
            // `if (!pObj) pObj = cthr->Obj; if (!pObj) return false` (:519).
            let Some(target) = target_id.or_else(|| context.object_context().map(|o| o.id()))
            else {
                return Ok(None);
            };
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
            Ok(Some(target))
        },
    )?;
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    // C++ arithmetic is plain i32 (wrapping on x86) — keep it exact.
    let mut change = change.wrapping_mul(MAGIC_PHYSICAL_FACTOR);

    // The positive overload expression is the first physical read. A failed
    // negative underload returns before GetPhysical entirely
    // (C4Script.cpp:523-540), which is observable for a lazy fair cache.
    if change > 0 {
        let current = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_scope(target))
                .map(ObjectScopeContext::magic_energy)
        });
        let Some(current) = current else {
            return Ok(Value::Bool(false));
        };
        let Some(cap) = resolve_object_physical(target, false).map(|physical| physical.magic)
        else {
            return Ok(Value::Bool(false));
        };
        if current.wrapping_add(change) > cap {
            if !allow_partial {
                return Ok(Value::Bool(false));
            }
            // The partial branch calls GetPhysical a second time, then reads
            // the live MagicEnergy again (C4Script.cpp:528).
            let Some(cap) = resolve_object_physical(target, false).map(|physical| physical.magic)
            else {
                return Ok(Value::Bool(false));
            };
            let current = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.object_scope(target))
                    .map(ObjectScopeContext::magic_energy)
            });
            let Some(current) = current else {
                return Ok(Value::Bool(false));
            };
            change = cap.wrapping_sub(current);
            if change == 0 {
                return Ok(Value::Bool(false));
            }
        }
    }

    if change < 0 {
        let current = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_scope(target))
                .map(ObjectScopeContext::magic_energy)
        });
        let Some(current) = current else {
            return Ok(Value::Bool(false));
        };
        if current.wrapping_add(change) < 0 {
            if !allow_partial {
                return Ok(Value::Bool(false));
            }
            change = current.wrapping_neg();
            if change == 0 {
                return Ok(Value::Bool(false));
            }
        }
    }

    // BoundBy performs the final GetPhysical even for a zero change. Re-read
    // MagicEnergy afterward so side effects from that first fill are live.
    let Some(cap) = resolve_object_physical(target, false).map(|physical| physical.magic) else {
        return Ok(Value::Bool(false));
    };
    try_with_host_context_mut(
        "DoMagicEnergy requires an active engine context",
        |context| {
            let Some(scope) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };
            let sum = scope.magic_energy().wrapping_add(change);
            scope.set_magic_energy(if sum < 0 {
                0
            } else if sum > cap {
                cap
            } else {
                sum
            });
            Ok(Value::Bool(true))
        },
    )
}

/// `FnGetMagicEnergy` (C4Script.cpp:546-550): MagicEnergy /
/// MagicPhysicalFactor; 0 without an object (`return false`).
pub(crate) fn get_magic_energy(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = magic_energy_target(args.first(), "GetMagicEnergy")?;
    try_with_host_context_mut(
        "GetMagicEnergy requires an active engine context",
        |context| {
            let Some(target) = target_id.or_else(|| context.object_context().map(|o| o.id()))
            else {
                return Ok(Value::Int(0));
            };
            if !context.ensure_object_scope(target) {
                return Ok(Value::Int(0));
            }
            let Some(scope) = context.object_scope(target) else {
                return Ok(Value::Int(0));
            };
            Ok(Value::Int(scope.magic_energy() / MAGIC_PHYSICAL_FACTOR))
        },
    )
}

/// The kill-trace mark of C4Object::DoEnergy (C4Object.cpp:1351-1353):
/// negative changes (and object hits even at zero) record the causing
/// player, with the UpdatLastEnergyLossCause guard (:1369-1378) applied
/// at call time against the freshest tracked value.
pub(crate) fn stage_energy_loss_cause(
    context: &mut EffectHostContext,
    target: ObjectId,
    change: i32,
    eng_type: i32,
    caused_by: i32,
) {
    if change >= 0 && eng_type != crate::C4FX_CALL_ENG_OBJ_HIT {
        return;
    }
    let tracked = context
        .object_scope(target)
        .and_then(|scope| scope.pending_update.energy_loss_cause)
        .or_else(|| {
            context
                .get_world_object(target)
                .map(|object| object.last_energy_loss_cause)
        })
        .unwrap_or(OWNER_NONE);
    let controller = context
        .object_scope(target)
        .map(|scope| scope.controller())
        .unwrap_or(OWNER_NONE);
    if caused_by != controller || tracked < 0 {
        if let Some(scope) = context.object_scope_mut(target) {
            scope.pending_update.energy_loss_cause = Some(caused_by);
        }
    }
}

pub(crate) fn do_con(args: &[Value]) -> Result<Value, RuntimeError> {
    let change_percent = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoCon: expected int or nil for change, got {}",
                other.type_name()
            )));
        }
    };

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(1) {
        target_id = parse_object_reference_argument(arg, "DoCon", "target")?;
    }

    if args.len() > 2 {
        return Err(RuntimeError::new(
            "DoCon: additional arguments are not supported",
        ));
    }

    // FnDoCon invokes the native method on its optional object parameter;
    // it does not dispatch a target-local script function of the same name.
    let target = target_id.or_else(active_object_id);
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    let delta = construction_delta_from_percent(change_percent);
    Ok(Value::Bool(do_con_live(target, delta)?))
}

/// Synchronous host-context `C4Object::DoCon(..., fInitial=false)`. The
/// low-level construction primitive is also used by NewObject's initial
/// pass, so all non-initial side arms live here instead of in that primitive.
pub(crate) fn do_con_live(target: ObjectId, delta: i32) -> Result<bool, RuntimeError> {
    let staged = with_host_context_mut(None, |context| {
        if !context.ensure_object_scope(target) {
            return None;
        }
        let metadata = context
            .object_effective_definition_id(target)
            .and_then(|definition_id| context.definition_metadata(&definition_id).cloned())
            .unwrap_or_default();
        let before = context.object_scope(target)?.construction();
        let entry_position = context.object_scope(target)?.effective_position();
        let entry_shape = live_object_shape(context, target);
        let previous_step = before / (FULL_CON / 100);
        let was_full = before >= FULL_CON;
        let after = context.stage_live_docon_construction(target, delta)?;
        let step_diff = after / (FULL_CON / 100) - previous_step;
        let refresh = crate::docon_refreshes_construction(before, after);
        let _ = refresh_live_object_ocf(context, target);
        if refresh {
            if let Some(scope) = context.object_scope_mut(target) {
                if metadata.line == 0 {
                    scope.pending_update.shape_override = Some(None);
                }
                scope.refresh_shape_preview(&metadata);
            }
            // UpdateFace(true) recreates/puts the mask before contents
            // ejection and SetAction(Idle), so callbacks fired by either
            // path must already see the new landscape (C4Object.cpp:
            // 1450-1472).
            context.preview_live_object_sector(target);
            context.update_live_solid_mask(target, false);
        }
        Some((
            metadata,
            entry_position,
            entry_shape,
            previous_step,
            step_diff,
            was_full,
            after,
            refresh,
        ))
    });
    let Some((
        metadata,
        entry_position,
        entry_shape,
        previous_step,
        step_diff,
        was_full,
        after,
        refresh,
    )) = staged
    else {
        return Ok(false);
    };

    if refresh && after < FULL_CON {
        if !metadata.fire.incomplete_activity {
            loop {
                let next = with_host_context(None, |context| {
                    let object = context.get_world_object(target)?;
                    Some((first_retained_content(context, target)?, object.container()))
                });
                let Some((child, destination)) = next else {
                    break;
                };
                let moved = if let Some(destination) = destination {
                    enter_object_live(child, destination)?
                } else {
                    exit_object_at_current_position(child)?
                };
                if !moved {
                    // A callback can remove/reparent the head while the
                    // requested transfer itself reports false. Re-read like
                    // C4ObjectList::GetObject; stop only if no progress was
                    // made and the exact same child is still first.
                    let same_head = HOST_CONTEXT.with(|cell| {
                        cell.borrow()
                            .as_ref()
                            .and_then(|context| first_retained_content(context, target))
                            == Some(child)
                    });
                    if same_head {
                        break;
                    }
                }
            }
        }
        HOST_CONTEXT.with(|cell| {
            if let Some(scope) = cell
                .borrow_mut()
                .as_mut()
                .and_then(|context| context.object_scope_mut(target))
            {
                scope.set_need_energy(false);
            }
        });
    }

    if refresh && was_full {
        let should_idle = HOST_CONTEXT
            .with(|cell| {
                let borrow = cell.borrow();
                let context = borrow.as_ref()?;
                let scope = context.object_scope(target)?;
                let definition = context.object_effective_definition_id(target)?;
                let incomplete_activity = context
                    .definition_metadata(&definition)
                    .is_some_and(|metadata| metadata.fire.incomplete_activity);
                Some(scope.construction() < FULL_CON && !incomplete_activity)
            })
            .unwrap_or(false);
        if should_idle {
            let _ = native_set_action_by_name(target, "Idle")?;
        }
    }

    if refresh {
        with_host_context_mut((), |context| {
            let Some(definition_id) = context.object_effective_definition_id(target) else {
                return;
            };
            let Some(current_metadata) = context.definition_metadata(&definition_id).cloned()
            else {
                return;
            };
            let current_shape = live_object_shape(context, target);
            let moved = {
                let Some(scope) = context.object_scope_mut(target) else {
                    return;
                };
                let current_position = scope.effective_position();
                let adjusted_y = crate::docon_adjusted_position_y(
                    entry_position.y,
                    entry_shape,
                    current_position.y,
                    current_shape,
                    scope.rotation(),
                    scope.category(),
                    previous_step,
                    step_diff,
                    current_metadata.shape.map_or(0, |shape| shape.height),
                );
                scope.current_position.y = adjusted_y;
                if let Some(position) = scope.pending_update.position.as_mut() {
                    position.y = adjusted_y;
                }
                scope.pending_update.resolved_docon_position = Some(scope.current_position);
                scope.pending_update.resolved_docon_fixed_position =
                    Some(scope.current_fixed_position);
                adjusted_y != current_position.y
            };
            // DoCon's keep-bottom/lift arm calls UpdateSolidMask again at
            // the adjusted position before Completion/Initialize.
            if moved {
                context.preview_live_object_sector(target);
                context.update_live_solid_mask(target, false);
            }
        });
    }

    let crossed_full = !was_full
        && HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_scope(target))
                .is_some_and(|scope| scope.construction() >= FULL_CON)
        });
    if crossed_full {
        call_object_own_fail_safe(target, "Completion", &[]);
        if object_has_status(target) {
            call_object_own_fail_safe(target, "Initialize", &[]);
        }
    }

    let reached_zero = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(target))
            .is_some_and(|scope| scope.construction() <= 0)
    });
    if reached_zero && object_has_status(target) {
        let _ = assign_removal_live(target, false)?;
    }
    Ok(true)
}

/// FnGetDamage (C4Script.cpp:1366-1370): `pObj->Damage`, the optional
/// object parameter defaulting to the caller.
pub(crate) fn get_damage(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetDamage expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetDamage", "target")?;
    }

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(Value::Int(object.damage()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.damage()));
            }
            return Ok(Value::Nil);
        }

        match context.object_context() {
            Some(object) => Ok(Value::Int(object.damage())),
            None => Ok(Value::Nil),
        }
    })
}

pub(crate) fn do_damage(args: &[Value]) -> Result<Value, RuntimeError> {
    do_damage_with_cause_override(args, None)
}

/// Internal counterpart to `do_damage` that bypasses the script ABI's
/// encoded-zero caller-controller default for explicit NO_OWNER.
pub(crate) fn do_damage_with_cause_override(
    args: &[Value],
    caused_by_override: Option<i32>,
) -> Result<Value, RuntimeError> {
    let change = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "DoDamage: expected int or nil for change, got {}",
                other.type_name()
            )));
        }
    };

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        match arg {
            Value::Object(_) | Value::Proplist(_) => {
                target_id = object_id_from_value(arg);
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

    let mut damage_type = crate::C4FX_CALL_DMG_SCRIPT;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                damage_type = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoDamage: expected int or nil for damage type, got {}",
                    other.type_name()
                )));
            }
        }
    }

    let mut caused_by_plus_one = 0;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Int(value) => {
                caused_by_plus_one = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "DoDamage: expected int or nil for caused by, got {}",
                    other.type_name()
                )));
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "DoDamage: additional arguments are not supported",
        ));
    }

    let staged =
        try_with_host_context_mut("DoDamage requires an active engine context", |context| {
            // iCausedBy = iCausedByPlusOne - 1, else the CALLER's controller
            // (C4Script.cpp:511) — resolved in the caller's scope.
            let caused_by = caused_by_override.unwrap_or_else(|| {
                if caused_by_plus_one != 0 {
                    caused_by_plus_one - 1
                } else {
                    context
                        .object_context()
                        .map(|object| object.controller())
                        .unwrap_or(OWNER_NONE)
                }
            });
            // `if (!pObj) pObj = cthr->Obj` is only the local-call default
            // (C4Script.cpp:510) — a named target may be FOREIGN.
            let Some(target) = target_id.or_else(|| context.object_context().map(|o| o.id()))
            else {
                return Ok(None);
            };
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
            let Some(scope) = context.object_scope(target) else {
                return Ok(None);
            };
            Ok(Some((target, caused_by, scope.alive())))
        })?;
    let Some((target, caused_by, alive)) = staged else {
        return Ok(Value::Bool(false));
    };
    // Non-living: ask the effects first (C4Object.cpp:1282-1286); a zero
    // chain outcome returns BEFORE the damage write and the ~Damage call.
    let change = match (!alive)
        .then(|| dispatch_effects_do_damage(target, change, damage_type, caused_by))
        .flatten()
    {
        Some(0) => return Ok(Value::Bool(true)),
        Some(modified) => modified,
        None => change,
    };
    with_host_context_mut((), |context| {
        if let Some(scope) = context.object_scope_mut(target) {
            // Damage = max(Damage + iChange, 0) (C4Object.cpp:1288).
            scope.adjust_damage(change);
        }
    });
    // The Damage engine call after the stat write (PSF_Damage "~Damage",
    // C4Object.cpp:1290 — fail-safe exec, errors log and continue).
    if let Some(Err(error)) = call_world_object_own_function(
        target,
        "Damage",
        &[Value::Int(change), Value::Int(caused_by)],
    ) {
        tracing::error!(
            %error,
            "script error in Damage; continuing like the C++ fail-safe exec"
        );
        log_runtime_call_frames("", error.call_frames());
    }
    Ok(Value::Bool(true))
}

pub(crate) fn set_action(args: &[Value]) -> Result<Value, RuntimeError> {
    if std::env::var("LC_DEBUG_CHBM").is_ok() {
        tracing::warn!(?args, "OSA SetAction");
    }
    let action_name = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) if !name.is_empty() => Some(name.clone()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "SetAction: expected string or nil for action name, got {}",
                other.type_name()
            )));
        }
    };

    // FnSetAction (C4Script.cpp:747-753): (szAction, pTarget, pTarget2,
    // fDirect) — the objects are the ACTION's targets
    // (SetActionByName(..., pTarget, pTarget2)), while fDirect is passed as
    // SetAction's fForce and bypasses NoOtherAction.
    let target1 = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetAction", "target"))
        .transpose()?
        .flatten();
    let target2 = args
        .get(2)
        .map(|arg| parse_object_reference_argument(arg, "SetAction", "target2"))
        .transpose()?
        .flatten();
    if args.len() > 4 {
        return Err(RuntimeError::new(format!(
            "SetAction: expected at most 4 arguments, got {}",
            args.len()
        )));
    }
    let force = args
        .get(3)
        .map(|value| value_to_bool(value, "SetAction", "direct/force"))
        .transpose()?
        .unwrap_or(false);

    let name = match action_name {
        Some(name) => name,
        None => return Ok(Value::Bool(false)),
    };
    let name = clonk_script::c4_string_from_bytes(&clonk_script::c4_string_bytes(&name));
    let builtin_idle = crate::action::is_builtin_idle_name(&name);
    let name = if builtin_idle {
        crate::action::DEFAULT_ACTION_NAME.to_string()
    } else {
        name
    };

    let mut sync_callbacks: Option<(
        ObjectId,
        Option<ScriptCallbackTarget>,
        Option<ScriptCallbackTarget>,
        i32,
        Option<String>,
    )> = None;
    let staged =
        try_with_host_context_mut("SetAction requires an active engine context", |context| {
            // SetActionByName returns false without changing anything when the
            // requested name is absent from the ActMap (C4Object.cpp:4218-4234).
            // "Idle"/"ActIdle" are the one sentinel exception. ChangeDef swaps
            // Def inline in C++, so a later same-call SetAction resolves against
            // the pending NEW definition (the horse Death -> Dead path).
            let action_exists = context.object_context().is_some_and(|object| {
                name == "Idle"
                    || object
                        .pending_update
                        .change_def
                        .as_deref()
                        .and_then(|definition| context.world.definition_metadata(definition))
                        .map(|metadata| metadata.action_library.contains(&name))
                        .unwrap_or_else(|| object.action_library.contains(&name))
            });
            if !action_exists {
                return Ok(Value::Bool(false));
            }
            let incomplete_activity = context
                .object_context()
                .and_then(|object| {
                    object
                        .pending_update
                        .change_def
                        .as_deref()
                        .or(object.definition_id.as_deref())
                })
                .and_then(|definition| context.world.definition_metadata(definition))
                .is_some_and(|metadata| metadata.fire.incomplete_activity);
            let (
                object_id,
                current_action,
                current_index,
                current_phase,
                callback_definition,
                requested_action_changed,
                actual_name,
                actual_index,
                changed_action,
                stop_sound,
                start_sound,
            ) = {
                let object = match context.object_context() {
                    Some(object) => object,
                    None => return Ok(Value::Bool(false)),
                };
                let current_action = object.effective_action_name().to_string();
                let current_index = object.effective_action_index();
                let requested_index = (!builtin_idle)
                    .then(|| object.action_library.named_action_index(&name))
                    .flatten();
                let requested_action_changed =
                    name != current_action || requested_index != current_index;
                if object.effective_blocks_other_actions() && requested_action_changed && !force {
                    return Ok(Value::Bool(false));
                }

                // C4Object::SetAction validates the requested slot and applies
                // the old action's NoOtherAction gate first, then stops the old
                // slot before incomplete construction can coerce the requested
                // slot to ActIdle (C4Object.cpp:4111-4130).
                let actual_name = if object.construction() < FULL_CON && !incomplete_activity {
                    crate::action::DEFAULT_ACTION_NAME.to_string()
                } else {
                    name.clone()
                };
                let actual_index = (actual_name != crate::action::DEFAULT_ACTION_NAME)
                    .then(|| object.action_library.named_action_index(&actual_name))
                    .flatten();
                let changed_action = actual_name != current_action || actual_index != current_index;
                let stop_sound = requested_action_changed
                    .then(|| {
                        object
                            .action_library
                            .spec_for_entry(&current_action, current_index)
                            .and_then(|spec| spec.sound.as_deref())
                            .filter(|sound| !sound.is_empty())
                            .map(str::to_owned)
                    })
                    .flatten();
                let start_sound = changed_action
                    .then(|| {
                        object
                            .action_library
                            .spec_for_entry(&actual_name, actual_index)
                            .and_then(|spec| spec.sound.as_deref())
                            .filter(|sound| !sound.is_empty())
                            .map(str::to_owned)
                    })
                    .flatten();
                (
                    object.id(),
                    current_action,
                    current_index,
                    object.action_phase(),
                    object
                        .pending_update
                        .change_def
                        .clone()
                        .or_else(|| object.definition_id.clone()),
                    requested_action_changed,
                    actual_name,
                    actual_index,
                    changed_action,
                    stop_sound,
                    start_sound,
                )
            };

            // StopSoundEffect is inside SetAction, before the action state is
            // replaced. This must reach the local sound system before a later
            // native in the same script can attempt another NewInstance.
            if let Some(sound) = stop_sound.as_deref() {
                if !context.stop_synchronous_sound(sound, Some(object_id)) {
                    context.audio_mut().stop_sound(sound, Some(object_id));
                }
            }

            let (start_call, abort_call) = {
                let Some(object) = context.object_context_mut() else {
                    return Ok(Value::Bool(false));
                };
                let update = object
                    .pending_update
                    .action
                    .get_or_insert_with(ActionUpdate::default);
                update.set_name(actual_name.clone());
                update.set_force(force);
                update.action_sound_dispatched |= requested_action_changed || changed_action;
                let sound_selection = start_sound
                    .as_ref()
                    .map(|sound| Some(sound.clone()))
                    .or_else(|| stop_sound.is_some().then_some(None));
                if sound_selection.is_some() {
                    update.action_sound_selection = sound_selection;
                }
                // C4Object::SetAction snaps fix_x/fix_y after changing the
                // action (C4Object.cpp:4168-4169). If it follows DoCon in this
                // staged call, that later snap wins over DoCon's stale-fixed
                // UpdatePos behavior.
                object.pending_update.construction_preserves_fixed_position = false;
                let position = object.effective_position();
                object.current_fixed_position = FixedVec2::from_ints(position.x, position.y);

                // SetActionByName carries the action targets, and C4Object::SetAction
                // assigns them ONLY when non-null (C4Object.cpp:4148-4150).
                // Idle/ActIdle discards supplied target args before SetAction
                // (C4Object.cpp:4225-4227).
                if !builtin_idle {
                    if target1.is_some() {
                        object.set_action_target(0, target1);
                    }
                    if target2.is_some() {
                        object.set_action_target(1, target2);
                    }
                }
                if changed_action {
                    object.reset_action_ticks();
                } else {
                    object.reset_action_phase_delay();
                }
                object.set_action_phase(0);

                if object.update_effective_action(&actual_name) {
                    object.reset_action_data();
                }
                // Start/Abort callbacks are also synchronous; the fold must not
                // queue them a second time.
                if let Some(update) = object.pending_update.action.as_mut() {
                    update.callbacks_dispatched = true;
                }
                (
                    (actual_name != crate::action::DEFAULT_ACTION_NAME)
                        .then(|| {
                            object
                                .action_library
                                .start_callback_for_entry(&actual_name, actual_index)
                        })
                        .flatten(),
                    object
                        .action_library
                        .abort_callback_for_entry(&current_action, current_index)
                        .filter(|_| !force),
                )
            };

            // C++ starts the new loop after selecting the action/targets and
            // before SetOCF and every Start/Abort callback
            // (C4Object.cpp:4159-4197). Ignore NewInstance's result here just
            // like C4Object::SetAction does.
            if let Some(sound) = start_sound.as_deref() {
                let _ = context.play_sound(sound, Some(object_id), 100, true, true, None);
            }
            sync_callbacks = Some((
                object_id,
                start_call,
                abort_call,
                current_phase,
                callback_definition,
            ));
            // SetAction calls SetOCF before Start/Abort callbacks. Use the full
            // live refresh so leaving an ObjectDisabled action can re-add
            // OCF_FightReady rather than only clearing stale bits.
            if refresh_live_object_ocf(context, object_id) {
                // SetAction's SetOCF is synchronous and may precede later
                // raw writes in this same callback. Carry that exact cache
                // through the deferred engine fold just like Enter/Exit.
                if let Some(object) = context.object_scope_mut(object_id) {
                    object.persist_final_ocf = true;
                }
            }

            Ok(Value::Bool(true))
        })?;
    // C4Object::SetAction runs the StartCall for the NEW action and then the
    // AbortCall for the OLD one SYNCHRONOUSLY inside the call
    // (SetActionByName defaults SAC_StartCall|SAC_AbortCall) — the
    // coach's Drive0 StartCall reads the PRE-SetDir facing for its seat
    // vertex, so deferral changes the result.
    if let Some((id, start_call, abort_call, previous_phase, callback_definition)) = sync_callbacks
    {
        // C++ has no SetAction-specific recursion guard. Nested callbacks
        // keep dispatching synchronously until they terminate or the shared
        // script VM reports its native-equivalent stack/value limit.
        let callbacks = [
            start_call.map(|callback| (callback, Vec::new())),
            abort_call.map(|callback| (callback, vec![Value::Int(previous_phase)])),
        ];
        for (callback, args) in callbacks.into_iter().flatten() {
            if let Some(Err(error)) = call_world_object_script_callback(id, &callback, &args) {
                tracing::error!(
                    %error,
                    callback = callback.function_name(),
                    "SetAction callback error; continuing like the C++ fail-safe exec"
                );
                log_runtime_call_frames("", error.call_frames());
            }
            let receiver_is_live = HOST_CONTEXT.with(|cell| {
                cell.borrow().as_ref().is_some_and(|context| {
                    context.object_status_present(id)
                        && callback_definition.as_deref().is_none_or(|expected| {
                            context.object_effective_definition_id(id).as_deref() == Some(expected)
                        })
                })
            });
            if !receiver_is_live {
                break;
            }
        }
    }
    Ok(staged)
}

/// C++ always has `Game.Material`; `None` only occurs in legacy Rust host
/// fixtures. Keep their previous byte clamp while making both bridge-data
/// entry points share one conversion.
fn clamp_bridge_material(material: i32, materials: Option<&MaterialSet>) -> i32 {
    match materials {
        Some(materials) if materials.is_empty() => -1,
        Some(materials) => material.min(materials.len().saturating_sub(1) as i32),
        None if material < 0 => -1,
        None => material.min(0xFF),
    }
}

pub(crate) fn set_bridge_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 5 {
        return Err(RuntimeError::new(
            "SetBridgeActionData expects at most 5 arguments: length, move flag, wall flag, material, object",
        ));
    }
    let param_count = args.len().min(4);
    let target_id = args
        .get(4)
        .map(|value| parse_object_reference_argument(value, "SetBridgeActionData", "object"))
        .transpose()?
        .flatten();

    let length = if param_count > 0 {
        value_to_i32(&args[0], "SetBridgeActionData", "length")?
    } else {
        // Unfilled iBridgeLength is nil -> 0 (C4Script.cpp:756).
        0
    };
    let move_clonk = if param_count > 1 {
        value_to_bool(&args[1], "SetBridgeActionData", "move_clonk")?
    } else {
        false
    };
    let wall = if param_count > 2 {
        value_to_bool(&args[2], "SetBridgeActionData", "wall")?
    } else {
        false
    };
    let material = if param_count > 3 {
        match &args[3] {
            Value::Nil => -1,
            other => value_to_i32(other, "SetBridgeActionData", "material")?,
        }
    } else {
        -1
    };

    try_with_host_context_mut(
        "SetBridgeActionData requires an active engine context",
        |context| {
            // C4Action::SetBridgeData clamps to the last loaded material before
            // packing the low byte (C4Object.cpp:54-62). A loaded empty table has
            // Num-1 == -1 and therefore stores the no-material sentinel 0xff.
            let material = clamp_bridge_material(material, context.world.materials());
            let encoded = encode_bridge_action_data(length, move_clonk, wall, material);

            // FnSetBridgeActionData defaults pObj to cthr->Obj, but an explicit
            // object may be foreign (C4Script.cpp:757-765). LOAM::StartBridge
            // runs in the loam's scope after ObjectSetAction staged "Bridge" on
            // the Clonk, so read and write that target's live nested scope.
            let Some(target) = target_id.or(context.script_object_context) else {
                return Ok(Value::Bool(false));
            };
            if !context.object_status_present(target) || !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }
            let Some(object) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };

            if object.effective_action_procedure() != ActionProcedure::Bridge {
                return Ok(Value::Bool(false));
            }

            object.set_action_data(encoded);
            Ok(Value::Bool(true))
        },
    )
}

pub(crate) fn set_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled iData is nil -> 0 (FnSetActionData, C4Script.cpp:767).
    let data = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetActionData", "data")?;
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

    try_with_host_context_mut(
        "SetActionData requires an active engine context",
        |context| {
            let bridge_material = clamp_bridge_material(data, context.world.materials());
            let Some(target) = target_id.or(context.script_object_context) else {
                return Ok(Value::Bool(false));
            };
            if !context.object_status_present(target) || !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }
            let Some(object) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };

            let procedure = object.effective_action_procedure();
            let mut next_data = data;
            match procedure {
                ActionProcedure::Bridge => {
                    next_data = encode_bridge_action_data(0, false, false, bridge_material);
                }
                ActionProcedure::Attach => {
                    let primary_vertex = data & 0xFF;
                    let secondary_vertex = data >> 8;
                    if primary_vertex >= MAX_VERTEX_COUNT || secondary_vertex >= MAX_VERTEX_COUNT {
                        return Ok(Value::Bool(false));
                    }
                }
                _ => {}
            }

            object.set_action_data(next_data);
            Ok(Value::Bool(true))
        },
    )
}

pub(crate) fn set_action_targets(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;

    // FnSetActionTargets assigns BOTH targets unconditionally
    // (C4Script.cpp:1108-1116): unfilled parameter slots are nil, so a
    // bare `SetActionTargets()` clears them (the horse's DisconnectWagon,
    // Horse.c4d Script.c:398).
    let target1 = if let Some(arg) = args.get(index) {
        let target = parse_object_reference_argument(arg, "SetActionTargets", "target1")?;
        index += 1;
        target
    } else {
        None
    };

    let target2 = if let Some(arg) = args.get(index) {
        let target = parse_object_reference_argument(arg, "SetActionTargets", "target2")?;
        index += 1;
        target
    } else {
        None
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

    try_with_host_context_mut(
        "SetActionTargets requires an active engine context",
        |context| {
            let Some(target) = object_id.or(context.script_object_context) else {
                return Ok(Value::Bool(false));
            };
            if !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }
            let Some(object) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };

            object.set_action_target(0, target1);
            object.set_action_target(1, target2);

            Ok(Value::Bool(true))
        },
    )
}

pub(crate) fn get_action(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetAction", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetAction: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    let action_name = object.effective_action_name();
                    let resolved = if action_name.is_empty() {
                        "Idle"
                    } else {
                        action_name
                    };
                    return Ok(Value::String(resolved.to_string().into()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                let resolved = if other.action_name.is_empty() {
                    "Idle"
                } else {
                    other.action_name.as_str()
                };
                return Ok(Value::String(resolved.to_string().into()));
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
        Ok(Value::String(resolved.to_string().into()))
    })
}

pub(crate) fn get_act_time(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetActTime", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActTime: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
        let action_time = |ticks: i32| Value::Int(ticks);

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(action_time(object.effective_action_ticks()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(action_time(other.action_ticks()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(action_time(object.effective_action_ticks()))
    })
}

pub(crate) fn get_phase(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetPhase", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetPhase: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
        let object = if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    object
                } else if let Some(other) = context.get_world_object(target) {
                    return Ok(Value::Int(other.action_phase()));
                } else {
                    return Ok(Value::Nil);
                }
            } else if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.action_phase()));
            } else {
                return Ok(Value::Nil);
            }
        } else {
            match context.object_context() {
                Some(obj) => obj,
                None => return Ok(Value::Nil),
            }
        };

        Ok(Value::Int(object.action_phase()))
    })
}

pub(crate) fn set_phase(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled iVal is nil -> 0 (FnSetPhase, C4Script.cpp:828).
    let phase = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetPhase: expected int or nil for phase, got {}",
                other.type_name()
            )));
        }
    };

    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetPhase", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetPhase: additional arguments are not supported",
        ));
    }

    try_with_host_context_mut("SetPhase requires an active engine context", |context| {
        let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };

        // C4Object::SetPhase (C4Object.cpp:2205-2211): a no-op on idle
        // objects; the phase clamps to [0, Length] (BoundBy is INCLUSIVE
        // of Length — the wrap transition fires at the next ExecAction).
        let action_name = object.effective_action_name().to_string();
        let action_index = object.effective_action_index();
        if object
            .action_library
            .is_idle_entry(&action_name, action_index)
        {
            return Ok(Value::Bool(false));
        }
        let length = object
            .action_library
            .spec_for_entry(&action_name, action_index)
            .and_then(|spec| spec.length)
            .unwrap_or(1);
        // C++ BoundBy evaluates the lower comparison first and does not
        // require ordered bounds. With a negative malformed Length,
        // negative input becomes 0 while nonnegative input becomes Length.
        let phase = if phase < 0 {
            0
        } else if phase > length {
            length
        } else {
            phase
        };
        object.set_action_phase(phase);
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_action_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetActionData", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetActionData: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
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

pub(crate) fn get_procedure(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetProcedure", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetProcedure: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
        let procedure_value = |name: Option<&str>| match name {
            Some(procedure) => Value::String(procedure.to_string().into()),
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

pub(crate) fn get_action_target(args: &[Value]) -> Result<Value, RuntimeError> {
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

    with_host_context(Ok(Value::Nil), |context| {
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

pub(crate) fn get_vertex_num(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetVertexNum", "object"))
        .transpose()?
        .flatten();

    with_host_context(Ok(Value::Nil), |context| {
        match resolve_vertices(context, target_id) {
            Some((_position, vertices)) => Ok(Value::Int(truncate_to_i32(vertices.len() as u64))),
            None => Ok(Value::Nil),
        }
    })
}

pub(crate) fn get_vertex(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "GetVertex: requires at least an index argument",
        ));
    }

    let index_value = value_to_i32(&args[0], "GetVertex", "index")?;
    // FnGetVertex's fixed C4ValueInt slot converts both an omitted value and
    // a legacy zero literal (nil) to VTX_X == 0.
    let attribute = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "GetVertex", "attribute")?;

    let mut target_id: Option<ObjectId> = None;
    let mut arg_index = 2;
    if let Some(arg) = args.get(2) {
        target_id = parse_object_reference_argument(arg, "GetVertex", "object")?;
        arg_index += 1;
    }

    if arg_index < args.len() {
        return Err(RuntimeError::new(
            "GetVertex: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
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

/// FnInside (C4Script.cpp:3350-3353): value within [lo, hi] inclusive.
pub(crate) fn inside(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = value_to_i32(args.first().unwrap_or(&Value::Nil), "Inside", "value")?;
    let lo = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Inside", "lo")?;
    let hi = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "Inside", "hi")?;
    Ok(Value::Bool(value >= lo && value <= hi))
}

/// FnGetVisibility (C4Script.cpp:3871-3877): return the target object's raw
/// visibility mask; a null target defaults to the calling object.
pub(crate) fn get_visibility(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetVisibility", "obj"))
        .transpose()?
        .flatten();
    with_host_context(Ok(Value::Nil), |context| {
        let Some(target) = target.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Nil);
        };
        Ok(context
            .object_visibility(target)
            .map(Value::Int)
            .unwrap_or(Value::Nil))
    })
}

/// FnGetColor (C4Script.cpp:3629-3633): deprecated oldgfx stub.
pub(crate) fn get_color(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(target) = args.first() {
        let _ = parse_object_reference_argument(target, "GetColor", "obj")?;
    }
    Ok(Value::Int(0))
}

/// FnSetCrewEnabled (C4Script.cpp:4814-4836): CrewDisabled = !enabled;
/// disabling also deselects (the cursor re-adjust runs on the engine
/// selection refresh).
/// FnGetColorDw (C4Script.cpp:3652-3656): the object's dword color;
/// nil without an object.
pub(crate) fn get_color_dw(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target = consume_optional_object_reference_argument(args, &mut index, "GetColorDw", "obj")?;
    with_host_context(Ok(Value::Nil), |context| {
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Nil);
        };
        // A same-call SetColorDw staged the value on the scope.
        if Some(target) == active {
            if let Some(color) = context
                .object_context()
                .and_then(|object| object.pending_update.color)
            {
                return Ok(Value::Int(color as i32));
            }
        }
        match context.get_world_object(target) {
            Some(object) => Ok(Value::Int(
                object
                    .full_state()
                    .map(|state| state.color as i32)
                    .unwrap_or(0),
            )),
            None => Ok(Value::Nil),
        }
    })
}

/// FnFling (C4Script.cpp:347-356) -> C4Object::Fling
/// (C4Object.cpp:1612-1624): optional half-speed add, then the Tumble
/// action if the ActMap has one, else the Jump action (after the
/// OnActionJump script hook), else raw velocity; the action-callback
/// bottom attach is cleared either way.
pub(crate) fn fling(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Fling", "obj"))
        .transpose()?
        .flatten();
    let xdir = parse_optional_i32(args.get(1), "Fling", "xdir")?.unwrap_or(0);
    let ydir = parse_optional_i32(args.get(2), "Fling", "ydir")?.unwrap_or(0);
    let prec = parse_optional_i32(args.get(3), "Fling", "precision")?
        .filter(|&prec| prec != 0)
        .unwrap_or(1);
    let add_speed = value_to_bool(args.get(4).unwrap_or(&Value::Nil), "Fling", "add speed")?;
    // FnFling requires an explicit target — `if (!pObj) return false;`
    // (C4Script.cpp:347-349), NO caller fallback (unlike FnJump, :358).
    // The horse/wipf `Fling(GetRider(), ...)` with no rider is a no-op.
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    let caused_by = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context())
            .map(ObjectScopeContext::controller)
            .unwrap_or(OWNER_NONE)
    });
    native_fling(
        target,
        FixedVec2::new(itofix_prec(xdir, prec), itofix_prec(ydir, prec)),
        add_speed,
        caused_by,
    )?;
    HOST_CONTEXT.with(|cell| {
        if let Some(object) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(target))
        {
            // FnFling clears the WHOLE t_attach value after C4Object::Fling
            // (C4Script.cpp:353-355), unlike ShakeObjects' direct call.
            object.set_t_attach(0);
        }
    });
    Ok(Value::Bool(true))
}

/// The `SimFlightHitsLiquid` probe used by `ObjectComJump`
/// (C4Movement.cpp:657-670; C4ObjectCom.cpp:297-305).
fn script_jump_hits_liquid(
    context: &EffectHostContext,
    object: &ObjectScopeContext,
    launch: FixedVec2,
    gravity: C4Fixed,
) -> bool {
    // The scope carries the live C4Shape field, including a preceding
    // SetContactDensity in this same script call (C4Script.cpp:1286-1291).
    if object.contact_density() <= 25 {
        return false;
    }
    let mut position = object.fixed_position();
    if let Some(bottom) = object
        .vertices()
        .iter()
        .filter(|vertex| vertex.cnat & CNAT_BOTTOM != 0)
        .min_by_key(|vertex| vertex.y)
    {
        // C4Shape::GetBottomVertex (C4Shape.cpp:445-455).
        position.x += bottom.x;
        position.y += bottom.y;
    }
    let mut velocity = launch;
    let Some(landscape) = context.world.landscape_ref() else {
        return false;
    };
    let density_at = |x, y| context.world.movement_density_at(x, y).unwrap_or(0);
    let liquid = |density| (25..50).contains(&density);
    if liquid(density_at(fixtoi(position.x), fixtoi(position.y)))
        && !crate::direct_com::sim_flight_to_density(
            &mut position,
            &mut velocity,
            0,
            24,
            10,
            gravity,
            landscape.width() as i32,
            landscape.estimated_height(),
            &density_at,
        )
    {
        return false;
    }
    if !crate::direct_com::sim_flight_to_density(
        &mut position,
        &mut velocity,
        25,
        100,
        -1,
        gravity,
        landscape.width() as i32,
        landscape.estimated_height(),
        &density_at,
    ) {
        return false;
    }
    let x = fixtoi(position.x);
    let y = fixtoi(position.y);
    liquid(density_at(x, y)) && liquid(density_at(x, y + 9))
}

/// FnJump (C4Script.cpp:358-363): synchronous `ObjectComJump`.
pub(crate) fn jump(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Jump", "obj"))
        .transpose()?
        .flatten();
    let active = with_host_context(None, |context| {
        context.object_context().map(|object| object.id())
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(target, "Jump", &[]) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    // FnJump → ObjectComJump runs SYNCHRONOUSLY (C4Script.cpp:358-363,
    // C4ObjectCom.cpp:280-312): the snake's Activity jump takes effect
    // THIS frame, before its movement — a queued command would lag one
    // frame. Gates: only while the WALK procedure runs.
    let jump_target = with_host_context(None, |context| {
        let object = context.object_context()?;
        let action_name = object.effective_action_name().to_string();
        if !matches!(
            object
                .action_library
                .procedure_for_entry(&action_name, object.effective_action_index()),
            crate::action::ActionProcedure::Walk
        ) {
            return None;
        }
        Some(object.id())
    });
    let Some(object_id) = jump_target else {
        return Ok(Value::Bool(false));
    };
    let Some(physical) = resolve_object_physical(object_id, false) else {
        return Ok(Value::Bool(false));
    };
    // SimFlightHitsLiquid reads the current global gravity after
    // GetPhysical; a fair-crew hook may have changed it synchronously.
    let gravity = PHYSICS_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| fixed100(context.gravity()) / 5)
    });
    let launch = with_host_context(None, |context| {
        let object = context.object_scope(object_id)?;
        let walk = physical.walk;
        let jump_physical = physical.jump;
        let con_scale = itofix_prec(object.construction(), crate::FULL_CON);
        let physical_walk = crate::math::val_by_physical(280, walk) * con_scale;
        let physical_jump = crate::math::val_by_physical(1000, jump_physical) * con_scale;
        let txdir = match object.command_direction() {
            CommandDirection::Left | CommandDirection::UpLeft => -physical_walk,
            CommandDirection::Right | CommandDirection::UpRight => physical_walk,
            _ => match object.direction() {
                Direction::Left => -physical_walk,
                Direction::Right => physical_walk,
                _ => C4Fixed::ZERO,
            },
        };
        let launch = FixedVec2::new(txdir, -physical_jump);
        let dive = gravity
            .is_some_and(|gravity| script_jump_hits_liquid(context, object, launch, gravity));
        Some((txdir, -physical_jump, dive))
    });
    let Some((txdir, tydir, dive)) = launch else {
        return Ok(Value::Bool(false));
    };
    if dive {
        let set = set_action(&[Value::String("Dive".into())])?;
        if matches!(set, Value::Bool(true)) {
            HOST_CONTEXT.with(|cell| {
                if let Some(object) = cell
                    .borrow_mut()
                    .as_mut()
                    .and_then(EffectHostContext::object_context_mut)
                {
                    object.set_fixed_velocity(FixedVec2::new(txdir, tydir));
                    object.set_mobile(true);
                    object.set_t_attach(object.t_attach() & !CNAT_BOTTOM);
                }
            });
            return Ok(Value::Bool(true));
        }
    }
    let jump_handled = match call_world_object_own_function(
        object_id,
        "OnActionJump",
        &[
            Value::Int(fixtoi_prec(txdir, 100)),
            Value::Int(fixtoi_prec(tydir, 100)),
            Value::Bool(true),
        ],
    ) {
        Some(Ok(value)) => value_raw_truthy(&value),
        Some(Err(error)) => {
            tracing::error!(
                %error,
                "OnActionJump error; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
            false
        }
        None => false,
    };
    if jump_handled {
        return Ok(Value::Bool(true));
    }
    // ObjectActionJump (C4ObjectCom.cpp:48-61): SetActionByName("Jump")
    // with its Abort/Start calls, then the launch velocities. A failed
    // SetAction (no Jump in the ActMap) returns false without touching
    // the dirs.
    let set = set_action(&[Value::String("Jump".into())])?;
    if !matches!(set, Value::Bool(true)) {
        return Ok(Value::Bool(false));
    }
    with_host_context_mut(Ok(Value::Bool(true)), |context| {
        if let Some(object) = context.object_context_mut() {
            object.set_fixed_velocity(FixedVec2::new(txdir, tydir));
            object.set_mobile(true);
            object.set_t_attach(object.t_attach() & !CNAT_BOTTOM);
        }
        Ok(Value::Bool(true))
    })
}

/// `CheckEnergyNeedChain` (C4Script.cpp:185-207): visit each object once,
/// test power consumers' `NeedEnergy`, then follow active PWRL objects in
/// master-list order from `Action.Target` to `Action.Target2`.
fn energy_chain_needs_power(
    context: &EffectHostContext,
    target: ObjectId,
    checked: &mut HashSet<ObjectId>,
) -> bool {
    if !checked.insert(target) {
        return false;
    }
    let world_object = context.get_world_object(target);
    let scope = context.object_scope(target);
    let definition = scope
        .and_then(|scope| scope.definition_id.as_deref())
        .map(ToOwned::to_owned)
        .or_else(|| {
            world_object
                .as_ref()
                .map(|object| object.definition_id().to_string())
        });
    let need_energy = scope
        .map(ObjectScopeContext::need_energy)
        .or_else(|| world_object.as_ref().map(|object| object.need_energy));
    let (Some(definition), Some(need_energy)) = (definition, need_energy) else {
        return false;
    };
    let is_consumer = context
        .definition_metadata(&definition)
        .is_some_and(|metadata| metadata.line_connect & crate::LINE_CONNECT_POWER_CONSUMER != 0);
    if is_consumer && need_energy {
        return true;
    }
    context.world_object_ids().into_iter().any(|line_id| {
        let Some(line) = context.get_world_object(line_id) else {
            return false;
        };
        line.status().is_active()
            && line.definition_id() == "PWRL"
            && line.action_target(0) == Some(target)
            && line
                .action_target(1)
                .is_some_and(|next| energy_chain_needs_power(context, next, checked))
    })
}

/// FnCheckEnergyNeedChain (C4Script.cpp:1832-1837): nil/default target means
/// the calling object; no object context returns nil.
pub(crate) fn check_energy_need_chain(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| match arg {
            // CheckConvertFunctionParameters Set0s every falsy parameter
            // before converting to C4Object* (C4AulExec.cpp:1370-1396).
            Value::Bool(false) => Ok(None),
            _ => parse_object_reference_argument(arg, "CheckEnergyNeedChain", "object"),
        })
        .transpose()?
        .flatten();
    with_host_context(Ok(Value::Nil), |context| {
        let target = target.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        Ok(Value::Bool(energy_chain_needs_power(
            context,
            target,
            &mut HashSet::new(),
        )))
    })
}

/// FnEnergyCheck (C4Script.cpp:1839-1849): true when the
/// StructuresNeedEnergy rule is off, the object has enough energy, or
/// the def is not a power consumer; update `NeedEnergy` on every branch.
pub(crate) fn energy_check(args: &[Value]) -> Result<Value, RuntimeError> {
    let energy = value_to_i32(args.first().unwrap_or(&Value::Nil), "EnergyCheck", "energy")?;
    let target = args
        .get(1)
        .map(|arg| match arg {
            Value::Bool(false) => Ok(None),
            _ => parse_object_reference_argument(arg, "EnergyCheck", "obj"),
        })
        .transpose()?
        .flatten();
    with_host_context_mut(Ok(Value::Nil), |context| {
        let target = target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        let scope_values = context.object_scope(target).map(|scope| {
            (
                scope.energy(),
                scope.definition_id.as_deref().map(ToOwned::to_owned),
            )
        });
        let world_object = context.get_world_object(target);
        let current_energy = scope_values
            .as_ref()
            .map(|(energy, _)| *energy)
            .or_else(|| world_object.as_ref().map(HostWorldObject::energy));
        let definition = scope_values
            .and_then(|(_, definition)| definition)
            .or_else(|| {
                world_object
                    .as_ref()
                    .map(|object| object.definition_id().to_string())
            });
        let (Some(current_energy), Some(definition)) = (current_energy, definition) else {
            return Ok(Value::Nil);
        };
        let is_consumer = context
            .world
            .definition_metadata(&definition)
            .map(|metadata| metadata.line_connect & crate::LINE_CONNECT_POWER_CONSUMER != 0)
            .unwrap_or(false);
        let need_energy =
            context.world.structures_need_energy() && current_energy < energy && is_consumer;
        if context.object_scope(target).is_none() && !context.ensure_object_scope(target) {
            return Ok(Value::Nil);
        }
        if let Some(object) = context.object_scope_mut(target) {
            object.set_need_energy(need_energy);
        }
        Ok(Value::Bool(!need_energy))
    })
}

/// FnStuck (C4Script.cpp:1858-1862): Shape.CheckContact(x, y) — is any
/// shape vertex inside solid at the current position (C4Shape.cpp
/// CheckContact probes GBackSolid per vertex).
pub(crate) fn stuck(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "Stuck", "obj"))
        .transpose()?
        .flatten();
    with_host_context(Ok(Value::Nil), |context| {
        let Some((position, vertices)) = resolve_vertices(context, target_id) else {
            return Ok(Value::Nil);
        };
        if context.landscape_ref().is_none() {
            return Ok(Value::Bool(false));
        }
        let resolved_target =
            target_id.or_else(|| context.object_context().map(|object| object.id()));
        let contact_density = resolved_target
            .and_then(|target| {
                context
                    .object_scope(target)
                    .map(ObjectScopeContext::contact_density)
                    .or_else(|| {
                        context
                            .get_world_object(target)
                            .map(|object| object.contact_density())
                    })
            })
            .unwrap_or(crate::CONTACT_DENSITY_SOLID);
        let pending_masks = context.pending_solid_masks();
        let stuck = vertices.iter().any(|vertex| {
            if vertex.cnat & CNAT_NO_COLLISION != 0 {
                return false;
            }
            context
                .movement_density_at(&pending_masks, position.x + vertex.x, position.y + vertex.y)
                .is_some_and(|density| density >= contact_density)
        });
        Ok(Value::Bool(stuck))
    })
}

pub(crate) fn get_vertex_contact(args: &[Value]) -> Result<Value, RuntimeError> {
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

    with_host_context(Ok(Value::Nil), |context| {
        let (position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        if vertex_index < 0 || (vertex_index as usize) >= vertices.len() {
            return Ok(Value::Nil);
        }

        let contact_density = resolve_contact_density(context, target_id);
        let pending_masks = context.pending_solid_masks();
        let contact = compute_vertex_contact(
            position,
            &vertices[vertex_index as usize],
            mask,
            contact_density,
            |x, y| context.movement_density_at(&pending_masks, x, y),
        );
        Ok(Value::Int(contact as i32))
    })
}

pub(crate) fn get_contact(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnGetContact(pObj, iVertex, dwCheck) — C4Script.cpp:5611-5626:
    // the OBJECT comes first; iVertex -1 ORs all vertex contacts. A
    // non-object in the pObj slot coerces to nil (self) like C4Value.
    let target_id = match args.first() {
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => {
            parse_object_reference_argument(value, "GetContact", "obj")?
        }
        _ => None,
    };
    let vertex_index = match args.get(1) {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "GetContact", "vertex")?,
    };
    let mask = match args.get(2) {
        None | Some(Value::Nil) => 0u32,
        Some(value) => {
            let raw = value_to_i32(value, "GetContact", "mask")?;
            if raw > 0 {
                raw as u32
            } else {
                0
            }
        }
    };

    with_host_context(Ok(Value::Nil), |context| {
        let (position, vertices) = match resolve_vertices(context, target_id) {
            Some(value) => value,
            None => return Ok(Value::Nil),
        };

        let contact_density = resolve_contact_density(context, target_id);
        let pending_masks = context.pending_solid_masks();

        if vertex_index == -1 {
            if vertices.is_empty() {
                return Ok(Value::Int(0));
            }
            let mut result = 0u32;
            for vertex in &vertices {
                result |=
                    compute_vertex_contact(position, vertex, mask, contact_density, |x, y| {
                        context.movement_density_at(&pending_masks, x, y)
                    });
            }
            return Ok(Value::Int(result as i32));
        }

        if vertex_index < 0 || (vertex_index as usize) >= vertices.len() {
            return Ok(Value::Nil);
        }

        let contact = compute_vertex_contact(
            position,
            &vertices[vertex_index as usize],
            mask,
            contact_density,
            |x, y| context.movement_density_at(&pending_masks, x, y),
        );
        Ok(Value::Int(contact as i32))
    })
}

/// FnFightWith (C4Script.cpp:5117-5132): require both cached
/// OCF_FightReady flags, run target/clonk RejectFight in that order, then
/// start the native Fight action on both objects with mutual targets.
pub(crate) fn fight_with(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "FightWith",
        "target",
    )?;
    let clonk =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "FightWith", "clonk")?;
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    let clonk = clonk.or_else(|| with_host_context(None, |context| context.script_object_context));
    let Some(clonk) = clonk else {
        return Ok(Value::Bool(false));
    };

    let both_ready = with_host_context(false, |context| {
        [target, clonk].into_iter().all(|object| {
            context
                .get_world_object(object)
                .is_some_and(|object| object.ocf() & ocf::FIGHT_READY != 0)
        })
    });
    if !both_ready {
        return Ok(Value::Bool(false));
    }

    // C4Object::Call silently skips an object that was deleted by the
    // preceding callback, but fPassErrors=true propagates callback errors.
    for object in [target, clonk] {
        let present = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .is_some_and(|context| context.object_status_present(object))
        });
        if !present {
            continue;
        }
        let rejected = match call_world_object_own_function(
            object,
            "RejectFight",
            &[object_reference_value(object)],
        ) {
            Some(result) => result?.as_bool(),
            None => false,
        };
        if rejected {
            return Ok(Value::Bool(false));
        }
    }

    // ObjectActionFight calls SetActionByName directly. Its return value is
    // deliberately ignored by FnFightWith, which still returns true.
    let _ = native_set_action_by_name_with_target(clonk, "Fight", Some(target))?;
    let _ = native_set_action_by_name_with_target(target, "Fight", Some(clonk))?;
    Ok(Value::Bool(true))
}

/// Native `C4Object::SetActionByName` staging for engine helpers such as
/// `C4Object::Fling`. This deliberately bypasses script-level function
/// resolution: C++ calls the object method directly.
pub(crate) fn native_set_action_by_name(
    target: ObjectId,
    name: &str,
) -> Result<bool, RuntimeError> {
    native_set_action_by_name_with_target(target, name, None)
}

pub(crate) fn native_set_action_by_name_with_target(
    target: ObjectId,
    name: &str,
    action_target: Option<ObjectId>,
) -> Result<bool, RuntimeError> {
    let builtin_idle = crate::action::is_builtin_idle_name(name);
    let name = if builtin_idle {
        crate::action::DEFAULT_ACTION_NAME
    } else {
        name
    };
    let callbacks = try_with_host_context_mut(
        "native action requires an active engine context",
        |context| {
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
            let incomplete_activity = context
                .object_scope(target)
                .and_then(|object| {
                    object
                        .pending_update
                        .change_def
                        .as_deref()
                        .or(object.definition_id.as_deref())
                })
                .and_then(|definition| context.world.definition_metadata(definition))
                .is_some_and(|metadata| metadata.fire.incomplete_activity);
            let (
                current,
                current_index,
                requested_changed,
                actual_name,
                actual_index,
                changed,
                previous_phase,
                definition,
                stop_sound,
                start_sound,
            ) = {
                let Some(object) = context.object_scope(target) else {
                    return Ok(None);
                };
                // ActIdle is the built-in action slot before ActMap and is
                // always valid even when no action named "Idle" exists.
                if name != "Idle" && !object.action_library.contains(name) {
                    return Ok(None);
                }
                let current = object.effective_action_name().to_string();
                let current_index = object.effective_action_index();
                let requested_index = (!builtin_idle)
                    .then(|| object.action_library.named_action_index(name))
                    .flatten();
                let requested_changed = current != name || current_index != requested_index;
                if object.effective_blocks_other_actions() && requested_changed {
                    return Ok(None);
                }
                let actual_name = if object.construction() < FULL_CON && !incomplete_activity {
                    crate::action::DEFAULT_ACTION_NAME.to_string()
                } else {
                    name.to_string()
                };
                let actual_index = (actual_name != crate::action::DEFAULT_ACTION_NAME)
                    .then(|| object.action_library.named_action_index(&actual_name))
                    .flatten();
                let changed = current != actual_name || current_index != actual_index;
                let stop_sound = requested_changed
                    .then(|| {
                        object
                            .action_library
                            .spec_for_entry(&current, current_index)
                            .and_then(|spec| spec.sound.as_deref())
                            .filter(|sound| !sound.is_empty())
                            .map(str::to_owned)
                    })
                    .flatten();
                let start_sound = changed
                    .then(|| {
                        object
                            .action_library
                            .spec_for_entry(&actual_name, actual_index)
                            .and_then(|spec| spec.sound.as_deref())
                            .filter(|sound| !sound.is_empty())
                            .map(str::to_owned)
                    })
                    .flatten();
                (
                    current,
                    current_index,
                    requested_changed,
                    actual_name,
                    actual_index,
                    changed,
                    object.action_phase(),
                    object
                        .pending_update
                        .change_def
                        .clone()
                        .or_else(|| object.definition_id.clone()),
                    stop_sound,
                    start_sound,
                )
            };

            if let Some(sound) = stop_sound.as_deref() {
                if !context.stop_synchronous_sound(sound, Some(target)) {
                    context.audio_mut().stop_sound(sound, Some(target));
                }
            }

            let callbacks = {
                let Some(object) = context.object_scope_mut(target) else {
                    return Ok(None);
                };
                let update = object
                    .pending_update
                    .action
                    .get_or_insert_with(ActionUpdate::default);
                update.set_name(actual_name.clone());
                update.set_force(false);
                update.callbacks_dispatched = true;
                update.action_sound_dispatched |= requested_changed || changed;
                let sound_selection = start_sound
                    .as_ref()
                    .map(|sound| Some(sound.clone()))
                    .or_else(|| stop_sound.is_some().then_some(None));
                if sound_selection.is_some() {
                    update.action_sound_selection = sound_selection;
                }
                if changed {
                    object.reset_action_ticks();
                } else {
                    object.reset_action_phase_delay();
                }
                object.set_action_phase(0);
                if object.update_effective_action(&actual_name) {
                    object.reset_action_data();
                }
                if !builtin_idle {
                    if let Some(action_target) = action_target {
                        object.set_action_target(0, Some(action_target));
                    }
                }
                let position = object.effective_position();
                object.current_fixed_position = FixedVec2::from_ints(position.x, position.y);

                (
                    (actual_name != crate::action::DEFAULT_ACTION_NAME)
                        .then(|| {
                            object
                                .action_library
                                .start_callback_for_entry(&actual_name, actual_index)
                        })
                        .flatten(),
                    object
                        .action_library
                        .abort_callback_for_entry(&current, current_index),
                    previous_phase,
                    definition,
                )
            };

            if let Some(sound) = start_sound.as_deref() {
                let _ = context.play_sound(sound, Some(target), 100, true, true, None);
            }
            if let Some(object) = context.object_scope_mut(target) {
                object.refresh_cached_ocf();
            }
            let _ = refresh_live_object_ocf(context, target);
            Ok(Some(callbacks))
        },
    )?;
    let Some((start_call, abort_call, previous_phase, definition)) = callbacks else {
        return Ok(false);
    };
    if let Some(callback) = start_call {
        if let Some(Err(error)) = call_world_object_script_callback(target, &callback, &[]) {
            tracing::error!(
                %error,
                callback = callback.function_name(),
                "native SetAction callback error; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
        }
    }
    let callbacks_continue = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(target))
            .is_some_and(|object| {
                !object.destroy
                    && object.status != ObjectStatus::Deleted
                    && object
                        .pending_update
                        .change_def
                        .as_deref()
                        .or(object.definition_id.as_deref())
                        == definition.as_deref()
            })
    });
    if callbacks_continue {
        if let Some(callback) = abort_call {
            if let Some(Err(error)) =
                call_world_object_script_callback(target, &callback, &[Value::Int(previous_phase)])
            {
                tracing::error!(
                    %error,
                    callback = callback.function_name(),
                    "native SetAction callback error; continuing like the C++ fail-safe exec"
                );
                log_runtime_call_frames("", error.call_frames());
            }
        }
    }
    Ok(true)
}

/// Target-aware `C4Object::SetDir` used by native action helpers.
pub(crate) fn native_set_dir(target: ObjectId, direction: Direction) -> Result<(), RuntimeError> {
    let turn_action =
        try_with_host_context_mut("SetDir requires an active engine context", |context| {
            if !context.ensure_object_scope(target) {
                return Ok(None);
            }
            let Some(object) = context.object_scope(target) else {
                return Ok(None);
            };
            let action = object.effective_action_name();
            let action_index = object.effective_action_index();
            let directions = object
                .action_library
                .directions_for_entry(action, action_index);
            let raw_direction = direction.to_script_value();
            if object.action_library.is_idle_entry(action, action_index)
                || raw_direction < 0
                || raw_direction >= directions
            {
                return Ok(None);
            }
            Ok(Some(
                (object.direction() != direction)
                    .then(|| {
                        object
                            .action_library
                            .turn_action_for_entry(action, action_index)
                            .map(str::to_string)
                    })
                    .flatten(),
            ))
        })?;
    let Some(turn_action) = turn_action else {
        return Ok(());
    };
    if let Some(turn_action) = turn_action {
        let _ = native_set_action_by_name(target, &turn_action)?;
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(object) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(target))
        {
            object.set_direction(direction);
        }
    });
    Ok(())
}

/// `C4Object::Fling(..., false, caused_by)` as used by ShakeObjects.
pub(crate) fn native_fling(
    target: ObjectId,
    mut velocity: FixedVec2,
    add_speed: bool,
    caused_by: i32,
) -> Result<(), RuntimeError> {
    let available =
        try_with_host_context_mut("Fling requires an active engine context", |context| {
            let Some(snapshot) = context.get_world_object(target) else {
                return Ok(false);
            };
            if !context.ensure_object_scope(target) {
                return Ok(false);
            }
            let staged_ocf = context
                .object_scope(target)
                .map(|object| object.staged_ocf(snapshot.ocf()))
                .unwrap_or_else(|| snapshot.ocf());
            if staged_ocf & ocf::ALIVE != 0 {
                stage_energy_loss_cause(
                    context,
                    target,
                    -1,
                    crate::C4FX_CALL_ENG_SCRIPT,
                    caused_by,
                );
            } else if context
                .object_scope(target)
                .and_then(ObjectScopeContext::container)
                .is_none()
            {
                if let Some(object) = context.object_scope_mut(target) {
                    object.set_controller(caused_by);
                }
            }
            if add_speed {
                if let Some(object) = context.object_scope(target) {
                    let current = object.fixed_velocity();
                    velocity.x += current.x / 2;
                    velocity.y += current.y / 2;
                }
            }
            Ok(true)
        })?;
    if !available {
        return Ok(());
    }

    if native_set_action_by_name(target, "Tumble")? {
        native_set_dir(
            target,
            if velocity.x < C4Fixed::ZERO {
                Direction::Right
            } else {
                Direction::Left
            },
        )?;
        HOST_CONTEXT.with(|cell| {
            if let Some(object) = cell
                .borrow_mut()
                .as_mut()
                .and_then(|context| context.object_scope_mut(target))
            {
                // ObjectActionTumble assigns xdir/ydir directly and therefore
                // preserves C4Object::Mobile. Counteract ObjectDelta's generic
                // script-velocity mobilization with the live post-callback value.
                let mobile = object.mobile();
                object.set_fixed_velocity(velocity);
                object.set_mobile(mobile);
            }
        });
        return Ok(());
    }

    let jump_handled = match call_world_object_own_function(
        target,
        "OnActionJump",
        &[
            Value::Int(fixtoi_prec(velocity.x, 100)),
            Value::Int(fixtoi_prec(velocity.y, 100)),
            Value::Bool(false),
        ],
    ) {
        Some(Ok(value)) => value.as_bool(),
        Some(Err(error)) => {
            tracing::error!(
                %error,
                "OnActionJump error; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
            false
        }
        None => false,
    };
    if jump_handled {
        return Ok(());
    }

    let _ = native_set_action_by_name(target, "Jump")?;
    HOST_CONTEXT.with(|cell| {
        if let Some(object) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(target))
        {
            object.set_fixed_velocity(velocity);
            object.set_mobile(true);
            object.set_t_attach(object.t_attach() & !CNAT_BOTTOM);
        }
    });
    Ok(())
}

/// FnShakeObjects -> C4Game::ShakeObjects (C4Script.cpp:3104-3106;
/// C4Game.cpp:1300-1314). Selection and random draws follow master-list
/// order; Random(3) precedes both attachment gates, while Rnd3 is consumed
/// only for objects that are actually flung.
pub(crate) fn shake_objects(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "ShakeObjects expects exactly 3 arguments: x, y, radius",
        ));
    }
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "ShakeObjects", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "ShakeObjects", "y")?;
    let radius = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "ShakeObjects", "radius")?;
    let (ids, caused_by) = try_with_host_context(
        "ShakeObjects requires an active engine context",
        |context| {
            Ok::<_, RuntimeError>((
                context.master_object_ids(),
                context
                    .object_context()
                    .map(ObjectScopeContext::controller)
                    .unwrap_or(OWNER_NONE),
            ))
        },
    )?;

    for id in ids {
        let candidate = with_host_context(None, |context| {
            let object = context.get_world_object(id)?;
            let category = context
                .object_scope(id)
                .map(ObjectScopeContext::category)
                .unwrap_or_else(|| object.category());
            Some((object, category))
        });
        let Some((candidate, category)) = candidate else {
            continue;
        };
        let position = candidate.position();
        // C4OS_INACTIVE objects live in InactiveObjects rather than the
        // Game.Objects list traversed by C4Game::ShakeObjects
        // (C4GameObjects.cpp:54-67).
        let inside = candidate.status() == ObjectStatus::Normal
            && candidate.container().is_none()
            && category & crate::CATEGORY_LIVING != 0
            && (i64::from(y) - i64::from(position.y)).abs() <= i64::from(radius)
            && (i64::from(x) - i64::from(position.x)).abs() <= i64::from(radius);
        if !inside || draw_context_random(3)? != 0 {
            continue;
        }
        let attached_to_world = with_host_context(None, |context| {
            let t_attach = context
                .object_scope(id)
                .map(ObjectScopeContext::t_attach)
                .or_else(|| candidate.full_state().map(|state| state.t_attach))?;
            Some(
                t_attach != 0
                    && candidate
                        .full_state()
                        .is_some_and(|state| !state.shape_attach.mat_vehicle),
            )
        });
        if attached_to_world != Some(true) {
            continue;
        }
        native_fling(
            id,
            FixedVec2::new(itofix(draw_context_rnd3()?), C4Fixed::ZERO),
            false,
            caused_by,
        )?;
    }
    Ok(Value::Nil)
}

pub(crate) fn set_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    if std::env::var("LC_DIRDBG").is_ok() && !matches!(args.first(), Some(Value::Int(_))) {
        eprintln!("DIRDBG SetDir args={args:?}");
    }
    // Unfilled ndir is nil -> 0 = DIR_Left; native int parameters also
    // accept C4Value bools through CheckConvertFunctionParameters
    // (C4AulExec.cpp:1364-1396; C4Value.cpp:514-518).
    let raw_direction = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetDir", "direction")?;

    let direction = Direction::from_raw(raw_direction);

    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetDir: additional arguments are not supported",
        ));
    }

    // Kept on the direct borrow rather than `try_with_host_context_mut`: this
    // wrapper releases the borrow part-way through (`drop(borrow)`) so the
    // TurnAction it schedules can re-enter host state, and a helper holds the
    // borrow for its whole closure. The borrow duration is the behaviour here,
    // not scaffolding.
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("SetDir requires an active engine context"))?;
        // FnSetDir works on ANY object (`if (!pObj) pObj = cthr->Obj`) —
        // GoldRush's init calls SetDir(1, pHorse) from the scenario
        // scope. Foreign writes stage into the target's nested scope.
        let active = context.object_context().map(|object| object.id());
        if let Some(target) = target_id {
            if Some(target) != active {
                if !context.nested_objects.contains_key(&target) {
                    let Some(world_object) = context.get_world_object(target) else {
                        return Ok(Value::Bool(false));
                    };
                    let Some((scope, local_vars)) = context.nested_scope_for(&world_object) else {
                        return Ok(Value::Bool(false));
                    };
                    context
                        .nested_objects
                        .insert(target, NestedScopeState { scope, local_vars });
                }
                if !context.nested_order.contains(&target) {
                    context.nested_order.push(target);
                }
                let state = context
                    .nested_objects
                    .get_mut(&target)
                    .expect("scope just ensured");
                let scope = &mut state.scope;
                // The same C4Object::SetDir gates as the self path; the
                // TurnAction fires through the staged action update.
                let action_name = scope.effective_action_name().to_string();
                let action_index = scope.effective_action_index();
                if scope
                    .action_library
                    .is_idle_entry(&action_name, action_index)
                {
                    return Ok(Value::Bool(true));
                }
                let directions = scope
                    .action_library
                    .directions_for_entry(&action_name, action_index);
                if direction.to_script_value() < 0 || direction.to_script_value() >= directions {
                    return Ok(Value::Bool(true));
                }
                let turn_action = (scope.direction() != direction)
                    .then(|| {
                        scope
                            .action_library
                            .turn_action_for_entry(&action_name, action_index)
                            .map(str::to_string)
                    })
                    .flatten();
                drop(borrow);
                if let Some(turn_action) = turn_action {
                    let _ = native_set_action_by_name(target, &turn_action)?;
                }
                return try_with_host_context_mut(
                    "SetDir requires an active engine context",
                    |context| {
                        if let Some(scope) = context.object_scope_mut(target) {
                            scope.set_direction(direction);
                        }
                        Ok(Value::Bool(true))
                    },
                );
            }
        }
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };

        // C4Object::SetDir (C4Object.cpp:4225-4245): a no-op on idle
        // objects and out-of-range directions (Directions defaults 1 —
        // single-direction actions reject SetDir(1)); a CHANGE fires the
        // action's TurnAction through SetActionByName first.
        let action_name = object.effective_action_name().to_string();
        let action_index = object.effective_action_index();
        if object
            .action_library
            .is_idle_entry(&action_name, action_index)
        {
            return Ok(Value::Bool(true));
        }
        let directions = object
            .action_library
            .directions_for_entry(&action_name, action_index);
        if direction.to_script_value() < 0 || direction.to_script_value() >= directions {
            return Ok(Value::Bool(true));
        }
        let turn_action = (object.direction() != direction)
            .then(|| {
                object
                    .action_library
                    .turn_action_for_entry(&action_name, action_index)
                    .map(str::to_string)
            })
            .flatten();
        drop(borrow);
        if let Some(turn_action) = turn_action {
            let _ = set_action(&[Value::String(turn_action.into())])?;
        }
        try_with_host_context_mut("SetDir requires an active engine context", |context| {
            if let Some(object) = context.object_context_mut() {
                object.set_direction(direction);
            }
            Ok(Value::Bool(true))
        })
    })
}

pub(crate) fn get_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetDir: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(object.direction().to_script_value()));
                }
            }
            // FnGetDir reads ANY pObj's Action.Dir (C4Script.cpp:1118-1122;
            // `if (!pObj) pObj = cthr->Obj` is only the local-call default —
            // an explicit target needs NO object context: GoldRush's
            // WINC->ActualizePhase(pClonk) reads GetDir(pClonk) from a
            // DEFINITION call, Winchester.c4d/Script.c:118-121). The CLNK
            // Riding() PhaseCall does SetDir(GetDir(GetActionTarget()))
            // on the ridden vehicle — a Nil here flipped riders Left.
            return Ok(context
                .get_world_object(target)
                .map(|other| Value::Int(other.direction))
                .unwrap_or(Value::Nil));
        }

        match context.object_context() {
            Some(object) => Ok(Value::Int(object.direction().to_script_value())),
            None => Ok(Value::Nil),
        }
    })
}

pub(crate) fn set_r(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled nr is nil -> 0 (FnSetR, C4Script.cpp:737).
    let rotation = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetR", "rotation")?;

    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetR", "object")?;
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetR: additional arguments are not supported",
        ));
    }

    try_with_host_context_mut("SetR requires an active engine context", |context| {
        let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let metadata = context
            .object_effective_definition_id(target)
            .and_then(|definition_id| context.definition_metadata(&definition_id).cloned())
            .unwrap_or_default();
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };

        object.set_rotation(rotation, &metadata);
        context.preview_live_object_sector(target);
        context.update_live_solid_mask(target, false);
        Ok(Value::Bool(true))
    })
}

pub(crate) fn script_rotation(rotation: i32) -> i32 {
    let rotation = rotation % 360;
    if rotation > 180 {
        rotation - 360
    } else if rotation < -180 {
        rotation + 360
    } else {
        rotation
    }
}

pub(crate) fn get_r(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new("GetR expects at most 1 argument: target"));
    }

    let target_id =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetR", "target")?;

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_scope(target) {
                return Ok(Value::Int(script_rotation(object.rotation())));
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(script_rotation(other.rotation)));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(Value::Int(script_rotation(object.rotation())))
    })
}

pub(crate) fn set_com_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled ncomdir is nil -> 0 = COMD_Stop (FnSetComDir,
    // C4Script.cpp:791, C4Object.h:56-57).
    let raw_direction = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetComDir: expected int or nil for command direction, got {}",
                other.type_name()
            )));
        }
    };

    let command_direction = CommandDirection::from_raw(raw_direction);

    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetComDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetComDir: additional arguments are not supported",
        ));
    }

    try_with_host_context_mut("SetComDir requires an active engine context", |context| {
        let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };

        object.set_command_direction(command_direction);
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_com_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "GetComDir", "target")?;

    if index < args.len() {
        return Err(RuntimeError::new(
            "GetComDir: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_scope(target) {
                return Ok(Value::Int(object.command_direction().to_script_value()));
            }
            return Ok(context
                .get_world_object(target)
                .map(|object| {
                    Value::Int(
                        object
                            .full_state()
                            .map(|state| state.command_direction)
                            .unwrap_or_default()
                            .to_script_value(),
                    )
                })
                .unwrap_or(Value::Nil));
        }

        match context.object_context() {
            Some(object) => Ok(Value::Int(object.command_direction().to_script_value())),
            None => Ok(Value::Nil),
        }
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
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, component.function_name(), "target")?;
    }

    with_host_context(Ok(Value::Nil), |context| {
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

        // FnGetX/FnGetY default to cthr->Obj, not the affected object that
        // C4Effect carries as pForObj (C4Script.cpp:1198-1202,1293-1297).
        // Definition-commanded effects have a mutable carrier scope but a
        // null script receiver, so their implicit coordinate origin is nil.
        let Some(target) = context.script_object_context else {
            return Ok(Value::Nil);
        };
        let position = context
            .object_scope(target)
            .map(ObjectScopeContext::effective_position)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.position())
            });
        Ok(position
            .map(|position| Value::Int(component.extract(position)))
            .unwrap_or(Value::Nil))
    })
}

pub(crate) fn object_distance(args: &[Value]) -> Result<Value, RuntimeError> {
    let other_id = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "ObjectDistance",
        "other",
    )?;
    let reference_id = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "ObjectDistance",
        "object",
    )?;
    let other_id = match other_id {
        Some(id) => id,
        None => return Ok(Value::Nil),
    };

    with_host_context(Ok(Value::Nil), |context| {
        let locate_position = |id: ObjectId| -> Option<Vector2> {
            if let Some(object) = context.object_context() {
                if object.id() == id {
                    return Some(object.effective_position());
                }
            }
            context.get_world_object(id).map(|object| object.position())
        };

        let anchor_position = if let Some(id) = reference_id {
            locate_position(id)
        } else {
            context
                .object_context()
                .map(|object| object.effective_position())
        };

        let anchor_position = match anchor_position {
            Some(position) => position,
            None => return Ok(Value::Nil),
        };

        let other_position = match locate_position(other_id) {
            Some(position) => position,
            None => return Ok(Value::Nil),
        };

        let distance = integer_distance(
            anchor_position.x,
            anchor_position.y,
            other_position.x,
            other_position.y,
        );
        Ok(Value::Int(distance))
    })
}

pub(crate) enum VelocityComponent {
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

    fn extract_fixed(&self, velocity: FixedVec2) -> C4Fixed {
        match self {
            VelocityComponent::X => velocity.x,
            VelocityComponent::Y => velocity.y,
        }
    }

    pub(crate) fn assign_fixed(&self, velocity: &mut FixedVec2, value: C4Fixed) {
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
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
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

    if has_remaining_native_argument(args, index) {
        return Err(RuntimeError::new(format!(
            "{}: additional arguments are not supported",
            component.get_function_name()
        )));
    }

    with_host_context(Ok(Value::Nil), |context| {
        let effective_precision = normalise_precision(precision);
        let fetch_velocity = |fixed_velocity: FixedVec2| {
            // C++ GetXDir/GetYDir return fixtoi(xdir/ydir, prec). `C4Script.cpp:1167`.
            let component_value = component.extract_fixed(fixed_velocity);
            Value::Int(fixtoi_prec(component_value, effective_precision))
        };

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(fetch_velocity(object.fixed_velocity()));
                }
            }

            if let Some(other) = context.get_world_object(target) {
                return Ok(fetch_velocity(other.fixed_velocity()));
            }

            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };
        Ok(fetch_velocity(object.fixed_velocity()))
    })
}

fn set_velocity_component(
    args: &[Value],
    component: VelocityComponent,
) -> Result<Value, RuntimeError> {
    // Unfilled parameter slots are nil -> 0 (C4AulExec parameter filling):
    // a bare FnSetXDir()/FnSetYDir() zeroes the dir (C4Script.cpp:697-708).
    let value = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        component.set_function_name(),
        "value",
    )?;
    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;

    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
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

    if has_remaining_native_argument(args, index) {
        return Err(RuntimeError::new(format!(
            "{}: additional arguments are not supported",
            component.set_function_name()
        )));
    }

    try_with_host_context_mut(
        &format!(
            "{} requires an active engine context",
            component.set_function_name()
        ),
        |context| {
            let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
            let Some(target) = target else {
                return Ok(Value::Bool(false));
            };
            if !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }
            let Some(object) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };

            // C++ SetXDir/SetYDir set ONLY xdir/ydir = itofix(value, prec)
            // (default precision 10, C4Script.cpp:697-732) — the other
            // component keeps its full sub-pixel value. A whole-vector write
            // here would quantize it through the scope's int-seeded mirror
            // (the GoldRush snake's SetXDir(0) turned ydir 2.6 into 3.0).
            object.set_fixed_velocity_component(
                component,
                itofix_prec(value, normalise_precision(precision)),
            );
            // FnSetXDir/FnSetYDir assign Mobile=1 synchronously after the dir
            // write (C4Script.cpp:697-732). Stage it here so later native
            // Tumble preserves the live value and call order remains observable.
            object.set_mobile(true);
            Ok(Value::Bool(true))
        },
    )
}

pub(crate) fn get_x_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    get_velocity_component(args, VelocityComponent::X)
}

pub(crate) fn get_y_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    get_velocity_component(args, VelocityComponent::Y)
}

pub(crate) fn set_x_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    set_velocity_component(args, VelocityComponent::X)
}

pub(crate) fn set_y_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    set_velocity_component(args, VelocityComponent::Y)
}

/// FnAdjustWalkRotation (C4Script.cpp:5439-5448): bails unless the def is
/// Rotateable, this frame's Action.t_attach carries CNAT_Bottom and the
/// last shape attach hit a material; then C4Object::AdjustWalkRotation
/// (C4Object.cpp:6019-6086) probes the floor around the attach position
/// and steers rdir toward the slope.
pub(crate) fn adjust_walk_rotation(args: &[Value]) -> Result<Value, RuntimeError> {
    let range_x = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "AdjustWalkRotation",
        "range_x",
    )?;
    let range_y = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "AdjustWalkRotation",
        "range_y",
    )?;
    let speed = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "AdjustWalkRotation",
        "speed",
    )?;
    let target_id = args
        .get(3)
        .map(|arg| parse_object_reference_argument(arg, "AdjustWalkRotation", "target"))
        .transpose()?
        .flatten();

    try_with_host_context_mut(
        "AdjustWalkRotation requires an active engine context",
        |context| {
            let target = target_id.or(context.script_object_context);
            let Some(target) = target else {
                return Ok(Value::Bool(false));
            };
            if !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }

            let (seed, rotation, live_vtx_x) = {
                let Some(object) = context.object_scope(target) else {
                    return Ok(Value::Bool(false));
                };
                let seed = object.walk_rotation;
                let rotation = object.rotation();
                // The LIVE Shape.VtxX for the else-branch (C4Object.cpp:6072).
                let live_vtx_x = usize::try_from(seed.attach.vtx)
                    .ok()
                    .and_then(|vtx| object.vertices().get(vtx))
                    .map(|vertex| vertex.x)
                    .unwrap_or(0);
                (seed, rotation, live_vtx_x)
            };

            // Guard: Rotateable + bottom attach + attached material
            // (C4Script.cpp:5443-5446).
            if seed.rotateable == 0 || seed.t_attach & CNAT_BOTTOM == 0 || !seed.attach.mat_valid {
                return Ok(Value::Bool(false));
            }

            let rotation_velocity = {
                let landscape = context.landscape_ref();
                crate::calculate_walk_rotation_velocity(
                    rotation,
                    seed.attach,
                    seed.def_attach_vtx_x,
                    live_vtx_x,
                    range_x,
                    range_y,
                    speed,
                    |x, y| evaluate_landscape_query(landscape, LandscapeQuery::Solid, x, y),
                )
            };

            let Some(object) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };
            // Move to destination angle (C4Object.cpp:6085-6088). C++ writes
            // rdir directly and never touches Mobile, so this uses the raw
            // staging path. The mobilising one outlived its procedure: a
            // walking object that died in the same frame still carried the
            // staged field into ChangeDef's fold, which re-mobilised the corpse
            // after ExecMovement had demobilised it (clonk-org/clonk-rs#1157).
            object.set_rotation_velocity_raw(rotation_velocity);
            Ok(Value::Bool(true))
        },
    )
}

pub(crate) fn set_r_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnSetRDir(value, [target], [precision = 10]) sets rdir = itofix(value,
    // precision), a fractional `C4Fixed` angular velocity. `C4Script.cpp:710`.
    // Unfilled parameter slots are nil -> 0 (C4AulExec parameter filling): a
    // bare FnSetRDir() zeroes the spin (the dragon's Jumping/StopFlying).
    let value = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetRDir", "value")?;
    let mut index = 1;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id = parse_object_reference_argument(arg, "SetRDir", "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, "SetRDir", "precision")?;
        index += 1;
    }

    if has_remaining_native_argument(args, index) {
        return Err(RuntimeError::new(
            "SetRDir: additional arguments are not supported",
        ));
    }

    try_with_host_context_mut("SetRDir requires an active engine context", |context| {
        let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };

        object.set_rotation_velocity(itofix_prec(value, normalise_precision(precision)));
        // FnSetRDir sets Mobile=1 just like the linear dir setters
        // (C4Script.cpp:704-715).
        object.set_mobile(true);
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_r_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnGetRDir([target], [precision = 10]) returns fixtoi(rdir, precision).
    // `C4Script.cpp` GetRDir.
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetRDir expects at most 2 arguments: target, precision",
        ));
    }

    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id = parse_object_reference_argument(arg, "GetRDir", "target")?;
            index += 1;
        }
    }

    let mut precision = DEFAULT_VELOCITY_PRECISION;
    if let Some(arg) = args.get(index) {
        precision = value_to_i32(arg, "GetRDir", "precision")?;
        index += 1;
    }

    if has_remaining_native_argument(args, index) {
        return Err(RuntimeError::new(
            "GetRDir: additional arguments are not supported",
        ));
    }

    with_host_context(Ok(Value::Nil), |context| {
        let effective_precision = normalise_precision(precision);
        if let Some(target) = target_id {
            if let Some(object) = context.object_scope(target) {
                return Ok(Value::Int(fixtoi_prec(
                    object.rotation_velocity(),
                    effective_precision,
                )));
            }
            return Ok(context
                .get_world_object(target)
                .map(|object| {
                    Value::Int(fixtoi_prec(object.rotation_velocity, effective_precision))
                })
                .unwrap_or(Value::Nil));
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };
        Ok(Value::Int(fixtoi_prec(
            object.rotation_velocity(),
            effective_precision,
        )))
    })
}

pub(crate) fn get_x(args: &[Value]) -> Result<Value, RuntimeError> {
    get_position_component(args, PositionComponent::X)
}

pub(crate) fn get_y(args: &[Value]) -> Result<Value, RuntimeError> {
    get_position_component(args, PositionComponent::Y)
}

/// FnGetDefBottom (C4Script.cpp:4445-4449): the object's integer Y plus
/// the untransformed definition-shape bottom. Live Shape changes, Con and
/// rotation do not participate.
pub(crate) fn get_def_bottom(args: &[Value]) -> Result<Value, RuntimeError> {
    let explicit_target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetDefBottom", "object"))
        .transpose()?
        .flatten();

    with_host_context(Ok(Value::Nil), |context| {
        let target = explicit_target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        let position = context
            .object_scope(target)
            .map(ObjectScopeContext::effective_position)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.position())
            });
        let Some(position) = position else {
            return Ok(Value::Nil);
        };
        let definition = context
            .object_scope(target)
            .and_then(|object| {
                object
                    .pending_update
                    .change_def
                    .clone()
                    .or_else(|| object.definition_id.clone())
            })
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.definition_id().to_string())
            });
        let Some(shape) = definition
            .as_deref()
            .and_then(|definition| context.definition_metadata(definition))
            .map(|metadata| metadata.shape.unwrap_or(DefinitionRect::new(0, 0, 0, 0)))
        else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(
            position.y.wrapping_add(shape.y).wrapping_add(shape.height),
        ))
    })
}

pub(crate) fn set_position(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled iX/iY are nil -> 0 (FnSetPosition, C4Script.cpp:465).
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetPosition", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetPosition", "y")?;

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
                )));
            }
        };
        index += 1;
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetPosition: additional arguments are not supported",
        ));
    }

    // FnSetPosition (C4Script.cpp:465-481): no pObj means the caller,
    // and ANY object force-positions live (`pObj->ForcePosition`) — the
    // BAS7 MoveOutClonk loop repositions a FOREIGN stuck object and
    // re-reads it within the same call.
    let target =
        try_with_host_context_mut("SetPosition requires an active engine context", |context| {
            let active = context.object_context().map(|object| object.id());
            Ok(target_id
                .or(active)
                .filter(|target| context.ensure_object_scope(*target)))
        })?;
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };

    let mut position = Vector2::new(x, y);
    if check_bounds {
        // fCheckBounds is C4Object::BoundsCheck (C4Script.cpp:470-476) —
        // pLayer and map-border limits under Def->BorderBound, never
        // landscape solidity. It can run Contact callbacks, so it must not
        // hold an object scope.
        bounds_check_live_object(target, &mut position);
    }

    try_with_host_context_mut("SetPosition requires an active engine context", |context| {
        let changed = {
            let Some(scope) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };

            let changed = scope.effective_position() != position;
            scope.set_position(position);
            changed
        };
        if changed {
            context.preview_live_object_sector(target);
            // C4Object::ForcePosition removes and re-puts the live mask only
            // after the integer X/Y early-return gate (C4Movement.cpp:
            // 552-561). The C4SolidMask instance itself survives.
            context.update_live_solid_mask(target, false);
        }
        update_in_liquid(context, target)?;
        Ok(Value::Bool(true))
    })
}

impl crate::engine_splash::SplashHost for EffectHostContext {
    type Error = RuntimeError;

    fn splash_is_semi_solid(&self, x: i32, y: i32) -> bool {
        self.landscape_ref()
            .is_some_and(|landscape| landscape.is_semi_solid_at(x, y))
    }

    fn splash_material_is_liquid(&self, x: i32, y: i32) -> bool {
        self.landscape_ref()
            .and_then(|landscape| landscape.material_at(x, y))
            .and_then(|material| {
                self.world
                    .materials()
                    .and_then(|set| set.get_by_id(material))
            })
            .is_some_and(|material| (25..50).contains(&material.density()) && material.instable())
    }

    fn splash_is_liquid(&self, x: i32, y: i32) -> bool {
        self.landscape_ref()
            .is_some_and(|landscape| landscape.is_liquid_at(x, y))
    }

    fn splash_random(&mut self, upper_bound: i32) -> Result<i32, Self::Error> {
        draw_context_random(upper_bound)
    }

    fn splash_bubble_out(&mut self, x: i32, y: i32) -> Result<(), Self::Error> {
        crate::compat::register_bubble(self, x, y)
    }

    fn splash_extract_and_cast(
        &mut self,
        source: Vector2,
        destination: Vector2,
        velocity: FixedVec2,
    ) -> Result<(), Self::Error> {
        if let Some(material) = self.preview_extract_liquid(source) {
            self.register_landscape_operation(LandscapeOperation::CastPxs {
                material,
                position: destination,
                velocities: vec![velocity],
            });
        }
        Ok(())
    }
}

/// C4Object::UpdateInLiquid (C4Object.cpp:6093-6110), called by
/// FnSetPosition after ForcePosition (C4Script.cpp:479). The cached flag is
/// deliberately updated synchronously so a following InLiquid() in the same
/// callback sees the native result.
fn update_in_liquid(context: &mut EffectHostContext, target: ObjectId) -> Result<(), RuntimeError> {
    let (position, construction, was_in_liquid, ocf, scope_definition) = {
        let Some(scope) = context.object_scope(target) else {
            return Ok(());
        };
        (
            scope.effective_position(),
            scope.construction(),
            scope.in_liquid(),
            scope.ocf(),
            scope
                .pending_update
                .change_def
                .clone()
                .or_else(|| scope.definition_id.clone()),
        )
    };
    let definition = scope_definition.or_else(|| {
        context
            .get_world_object(target)
            .map(|object| object.definition_id().to_string())
    });
    let float_line = definition
        .as_deref()
        .and_then(|id| context.definition_metadata(id))
        .map(|metadata| metadata.float_line)
        .unwrap_or(0);
    let probe_y = crate::engine_splash::liquid_probe_y(position.y, float_line, construction);
    let in_liquid = context
        .landscape_ref()
        .is_some_and(|landscape| landscape.is_liquid_at(position.x, probe_y));
    if crate::engine_splash::entered_liquid(in_liquid, was_in_liquid) {
        let mass = current_object_mass(context, target);
        if crate::engine_splash::should_splash(in_liquid, was_in_liquid, ocf, mass) {
            let amount = live_object_shape(context, target)
                .map(|shape| crate::engine_splash::splash_amount(shape.width, shape.height))
                .unwrap_or(0);
            crate::engine_splash::run_splash(
                context,
                position.x,
                position.y.saturating_add(1),
                amount,
            )?;
        }
    }
    if in_liquid != was_in_liquid {
        if let Some(scope) = context.object_scope_mut(target) {
            scope.set_in_liquid(in_liquid);
        }
    }
    Ok(())
}

fn current_object_mass(context: &EffectHostContext, target: ObjectId) -> i32 {
    // C4Object::Mass is a live compiled cache (C4Object.cpp:497-505), not
    // merely DefCore mass plus contents. SetMass/DoCon/ChangeDef invalidate
    // it before their deferred update is folded, so use the cached value only
    // while those same-call invalidations are absent.
    let compiled_mass = context.object_scope(target).and_then(|scope| {
        let pending_invalidates_cache = scope.pending_update.own_mass.is_some()
            || scope.pending_update.construction.is_some()
            || scope.pending_update.change_def.is_some()
            || scope.pending_update.contents_front.is_some();
        (!pending_invalidates_cache)
            .then(|| context.get_world_object(target))
            .flatten()
            .and_then(|object| object.compiled_mass)
    });
    compiled_mass.unwrap_or_else(|| reflected_object_mass(context, target, &mut HashSet::new()))
}

/// FnIsNewgfx (C4Script.cpp:4947): the compatibility probe is always true.
pub(crate) fn is_newgfx(_args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(true))
}

/// FnGetUnusedOverlayID (C4Script.cpp:5942-5951): search away from a
/// nonzero base index until the target object has no overlay in that slot.
pub(crate) fn get_unused_overlay_id(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut overlay_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetUnusedOverlayID",
        "base index",
    )?;
    let target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "GetUnusedOverlayID",
        "object",
    )?;
    if overlay_id == 0 {
        return Ok(Value::Nil);
    }

    with_host_context(Ok(Value::Nil), |context| {
        let target = target.or(context.script_object_context);
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        if context.object_scope(target).is_none() && context.get_world_object(target).is_none() {
            return Ok(Value::Nil);
        }

        let search_step = if overlay_id < 0 { -1 } else { 1 };
        while context.object_has_graphics_overlay(target, overlay_id) {
            overlay_id = overlay_id.wrapping_add(search_step);
        }
        Ok(Value::Int(overlay_id))
    })
}

pub(crate) fn set_graphics(args: &[Value]) -> Result<Value, RuntimeError> {
    // A null pGfxName restores the DEFAULT graphics (FnSetGraphics,
    // C4Script.cpp:4378).
    let graphics_name = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) if !name.is_empty() => Some(name.as_ref().to_owned()),
        Value::String(_) | Value::Nil => None,
        // Falsy parameters reset to nil before the type check
        // (C4AulExec.cpp:1372): SetGraphics(0) selects the default graphics.
        Value::Int(0) | Value::Bool(false) => None,
        other => {
            return Err(RuntimeError::new(format!(
                "SetGraphics: expected string or nil for graphics name, got {}",
                other.type_name()
            )));
        }
    };

    let mut index = 1;

    let target_id = if let Some(arg) = args.get(index) {
        index += 1;
        parse_object_reference_argument(arg, "SetGraphics", "object")?
    } else {
        None
    };

    let definition = if let Some(arg) = args.get(index) {
        index += 1;
        parse_native_c4id_argument(Some(arg), "SetGraphics")?
    } else {
        None
    };

    let overlay_id = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => *value,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected int or nil for overlay id, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        0
    };

    let mode_value = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => *value,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected int or nil for overlay mode, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        0
    };

    let action_name = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::String(name) if !name.is_empty() => Some(name.as_ref().to_owned()),
            Value::String(_) | Value::Nil | Value::Int(0) | Value::Bool(false) => None,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected string or nil for action, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        None
    };

    let blit_mode = if let Some(arg) = args.get(index) {
        index += 1;
        match arg {
            Value::Int(value) => (*value).max(0) as u32,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetGraphics: expected int or nil for blit mode, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        0
    };

    let overlay_object = if let Some(arg) = args.get(index) {
        index += 1;
        parse_object_reference_argument(arg, "SetGraphics", "overlay_object")?
    } else {
        None
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetGraphics: additional arguments are not supported",
        ));
    }

    try_with_host_context_mut("SetGraphics requires an active engine context", |context| {
        let object_id = if let Some(target) = target_id {
            target
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Bool(false)),
            }
        };
        if !context.ensure_object_scope(object_id)
            || context
                .object_scope(object_id)
                .is_none_or(|object| object.status() == ObjectStatus::Deleted)
        {
            return Ok(Value::Bool(false));
        }

        let mut resolved_definition = definition.clone();
        if overlay_id <= 0 && resolved_definition.is_none() {
            resolved_definition = context.object_effective_definition_id(object_id);
            if resolved_definition.is_none() {
                return Ok(Value::Bool(false));
            }
        }

        if overlay_id <= 0 {
            let definition_id = resolved_definition.expect("resolved definition present");

            if context.definition_metadata(&definition_id).is_none() {
                return Ok(Value::Bool(false));
            }
            let color_by_owner = context.world.definition_color_by_owner(&definition_id);
            let target_definition = context.object_effective_definition_id(object_id);
            // UpdateGraphics(true) only re-checks the object rectangle when
            // a live C4SolidMask instance existed before the graphics swap.
            let active_solid_mask = context
                .live_solid_mask_spec(object_id)
                .map(|spec| spec.mask);

            let base_graphics = if graphics_name.is_none()
                && target_definition.as_deref() == Some(definition_id.as_str())
            {
                None
            } else {
                Some(ObjectBaseGraphics {
                    definition: definition_id,
                    graphics_name: graphics_name.clone(),
                    // FnSetGraphics forwards blit mode only to overlays;
                    // C4Object::SetGraphics receives just name/source def.
                    blit_mode: 0,
                })
            };
            let changed = {
                let Some(object) = context.object_scope_mut(object_id) else {
                    return Ok(Value::Bool(false));
                };
                let own_definition = target_definition.as_deref().unwrap_or_default();
                let same_name = |left: Option<&str>, right: Option<&str>| match (
                    left.filter(|name| !name.is_empty()),
                    right.filter(|name| !name.is_empty()),
                ) {
                    (None, None) => true,
                    (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
                    _ => false,
                };
                let same_graphics = match (object.base_graphics.as_ref(), base_graphics.as_ref()) {
                    (None, None) => true,
                    (Some(left), Some(right)) => {
                        left.definition.eq_ignore_ascii_case(&right.definition)
                            && same_name(
                                left.graphics_name.as_deref(),
                                right.graphics_name.as_deref(),
                            )
                    }
                    (None, Some(right)) => {
                        right.definition.eq_ignore_ascii_case(own_definition)
                            && same_name(None, right.graphics_name.as_deref())
                    }
                    (Some(left), None) => {
                        left.definition.eq_ignore_ascii_case(own_definition)
                            && same_name(left.graphics_name.as_deref(), None)
                    }
                };
                let changed = !same_graphics && object.set_base_graphics(base_graphics);
                if changed && !color_by_owner {
                    object.pending_update.color = Some(0);
                }
                changed
            };
            if changed {
                if let Some(mask) = active_solid_mask {
                    if let Some(checked) = context.check_solid_mask_rect_for_object(object_id, mask)
                    {
                        if checked != mask {
                            if let Some(object) = context.object_scope_mut(object_id) {
                                object.set_solid_mask_rect(checked);
                            }
                        }
                    }
                }
                context.update_live_solid_mask(object_id, true);
            }
            return Ok(Value::Bool(true));
        }

        let Some(object) = context.object_scope_mut(object_id) else {
            return Ok(Value::Bool(false));
        };

        if overlay_id < 0 {
            return Ok(Value::Bool(false));
        }

        let mode = if mode_value == 0 {
            GraphicsOverlayMode::Base
        } else {
            match GraphicsOverlayMode::from_script_value(mode_value) {
                Some(mode) => mode,
                None => return Ok(Value::Bool(false)),
            }
        };

        if mode == GraphicsOverlayMode::Object && overlay_object.is_none() {
            let removed = object.remove_graphics_overlay(overlay_id);
            return Ok(Value::Bool(removed));
        }

        if mode != GraphicsOverlayMode::Object && definition.is_none() {
            let removed = object.remove_graphics_overlay(overlay_id);
            return Ok(Value::Bool(removed));
        }

        let overlay = ObjectGraphicsOverlay::new(overlay_id, mode)
            .with_definition(if mode == GraphicsOverlayMode::Object {
                None
            } else {
                definition.clone()
            })
            .with_graphics_name(graphics_name.clone())
            .with_action(action_name)
            .with_blit_mode(blit_mode)
            .with_overlay_object(overlay_object);

        // FnSetGraphics returns true for every valid overlay it sets --
        // "// Okay, valid overlay set!" -- and false only when IsValid rejects
        // the result (src/C4Script.cpp:4596-4603). The bool below reports
        // whether the stored state moved, which drives the pending update; it
        // is not the script-visible result.
        object.set_graphics_overlay(overlay);
        Ok(Value::Bool(true))
    })
}

fn parse_draw_transform_components(
    args: &[Value],
    function: &str,
) -> Result<[i32; 6], RuntimeError> {
    Ok([
        value_to_i32(args.first().unwrap_or(&Value::Nil), function, "a")?,
        value_to_i32(args.get(1).unwrap_or(&Value::Nil), function, "b")?,
        value_to_i32(args.get(2).unwrap_or(&Value::Nil), function, "c")?,
        value_to_i32(args.get(3).unwrap_or(&Value::Nil), function, "d")?,
        value_to_i32(args.get(4).unwrap_or(&Value::Nil), function, "e")?,
        value_to_i32(args.get(5).unwrap_or(&Value::Nil), function, "f")?,
    ])
}

fn parse_draw_transform_matrix(args: &[Value], function: &str) -> Result<[i32; 9], RuntimeError> {
    Ok([
        value_to_i32(args.first().unwrap_or(&Value::Nil), function, "a")?,
        value_to_i32(args.get(1).unwrap_or(&Value::Nil), function, "b")?,
        value_to_i32(args.get(2).unwrap_or(&Value::Nil), function, "c")?,
        value_to_i32(args.get(3).unwrap_or(&Value::Nil), function, "d")?,
        value_to_i32(args.get(4).unwrap_or(&Value::Nil), function, "e")?,
        value_to_i32(args.get(5).unwrap_or(&Value::Nil), function, "f")?,
        value_to_i32(args.get(6).unwrap_or(&Value::Nil), function, "g")?,
        value_to_i32(args.get(7).unwrap_or(&Value::Nil), function, "h")?,
        value_to_i32(args.get(8).unwrap_or(&Value::Nil), function, "i")?,
    ])
}

pub(crate) fn set_obj_draw_transform(args: &[Value]) -> Result<Value, RuntimeError> {
    let components = parse_draw_transform_components(args, "SetObjDrawTransform")?;
    let mut index = 6;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        target_id = parse_object_reference_argument(arg, "SetObjDrawTransform", "object")?;
        index += 1;
    }
    let overlay_id = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "SetObjDrawTransform", "overlay")?;
        index += 1;
        value
    } else {
        0
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetObjDrawTransform: additional arguments are not supported",
        ));
    }

    let transform = DrawTransform::from_matrix([
        components[0] as f32 / 1000.0,
        components[1] as f32 / 1000.0,
        components[2] as f32 / 1000.0,
        components[3] as f32 / 1000.0,
        components[4] as f32 / 1000.0,
        components[5] as f32 / 1000.0,
        0.0,
        0.0,
        1.0,
    ]);
    let resets_base = components[1] == 0
        && components[2] == 0
        && components[3] == 0
        && components[5] == 0
        && components[0] == components[4]
        && matches!(components[0], 0 | 1000);

    try_with_host_context_mut(
        "SetObjDrawTransform requires an active engine context",
        |context| {
            let object_id =
                match target_id.or_else(|| context.object_context().map(|object| object.id())) {
                    Some(object) => object,
                    None => return Ok(Value::Bool(false)),
                };
            if !context.ensure_object_scope(object_id) {
                return Ok(Value::Bool(false));
            }
            let Some(object) = context.object_scope_mut(object_id) else {
                return Ok(Value::Bool(false));
            };

            if overlay_id == 0 {
                let current = object.draw_transform();
                let flip_dir = current.map_or(1, |transform| transform.flip_dir());
                if resets_base && flip_dir == 1 {
                    object.set_draw_transform(None);
                } else {
                    object.set_draw_transform(Some(transform.with_flip_dir(flip_dir)));
                }
                Ok(Value::Bool(true))
            } else {
                let flip_dir = object
                    .overlay_transform(overlay_id)
                    .flatten()
                    .map_or(1, |transform| transform.flip_dir());
                let changed = object
                    .set_overlay_transform(overlay_id, Some(transform.with_flip_dir(flip_dir)));
                Ok(Value::Bool(changed))
            }
        },
    )
}

/// FnSetObjDrawTransform2 (C4Script.cpp:5276-5305): nine matrix integers
/// followed by iOverlayID, always applied to `cthr->Obj`.
pub(crate) fn set_obj_draw_transform2(args: &[Value]) -> Result<Value, RuntimeError> {
    let matrix = parse_draw_transform_matrix(args, "SetObjDrawTransform2")?;
    let overlay_id = args
        .get(9)
        .map(|arg| value_to_i32(arg, "SetObjDrawTransform2", "overlay"))
        .transpose()?
        .unwrap_or(0);

    if args.len() > 10 {
        return Err(RuntimeError::new(
            "SetObjDrawTransform2: additional arguments are not supported",
        ));
    }

    let delta = DrawTransform::from_matrix([
        matrix[0] as f32 / 1000.0,
        matrix[1] as f32 / 1000.0,
        matrix[2] as f32 / 1000.0,
        matrix[3] as f32 / 1000.0,
        matrix[4] as f32 / 1000.0,
        matrix[5] as f32 / 1000.0,
        matrix[6] as f32 / 1000.0,
        matrix[7] as f32 / 1000.0,
        matrix[8] as f32 / 1000.0,
    ]);

    try_with_host_context_mut(
        "SetObjDrawTransform2 requires an active engine context",
        |context| {
            let Some(object_id) = context.script_object_context else {
                return Ok(Value::Bool(false));
            };
            if !context.ensure_object_scope(object_id) {
                return Ok(Value::Bool(false));
            }
            let Some(object) = context.object_scope_mut(object_id) else {
                return Ok(Value::Bool(false));
            };

            if overlay_id == 0 {
                let current = object.draw_transform().unwrap_or(DrawTransform::identity());
                let combined = current.combined(delta);
                object.set_draw_transform(Some(combined));
                Ok(Value::Bool(true))
            } else {
                let existing = match object.overlay_transform(overlay_id) {
                    Some(transform) => transform.unwrap_or(DrawTransform::identity()),
                    None => return Ok(Value::Bool(false)),
                };
                let combined = existing.combined(delta);
                object.set_overlay_transform(overlay_id, Some(combined));
                Ok(Value::Bool(true))
            }
        },
    )
}

/// FnGetActMapVal (C4Script.cpp:4216-4241): one entry of one action in a
/// definition's ActMap, addressed by its serialization name
/// (C4ActionDef::CompileFunc, C4Def.cpp). Unknown definition, action or
/// entry -> nil. C4ActionDef compile defaults: Length 1, Delay 0, strings "".
pub(crate) fn get_act_map_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let entry = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) => Some(name.clone()),
        Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "GetActMapVal: expected string for entry, got {}",
                other.type_name()
            )));
        }
    };
    let action = match args.get(1).unwrap_or(&Value::Nil) {
        Value::String(name) => Some(name.clone()),
        Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "GetActMapVal: expected string for action, got {}",
                other.type_name()
            )));
        }
    };
    let definition = parse_native_c4id_argument(args.get(2), "GetActMapVal")?;
    let entry_index = parse_optional_i32(args.get(3), "GetActMapVal", "entry_nr")?.unwrap_or(0);
    let Some(entry) = entry else {
        return Ok(Value::Nil);
    };
    let action = action.unwrap_or_default();

    with_host_context(Ok(Value::Nil), |context| {
        // `idDef` defaults to the executing definition (cthr->Def).
        let (library, graphics) = match definition {
            Some(id) => match context.definition_metadata(&id) {
                Some(metadata) => (&metadata.action_library, Some(&metadata.action_graphics)),
                None => return Ok(Value::Nil),
            },
            None => match context.executing_definition_id() {
                Some(Some(definition)) => {
                    let Some(metadata) = context.definition_metadata(definition.as_str()) else {
                        return Ok(Value::Nil);
                    };
                    (&metadata.action_library, Some(&metadata.action_graphics))
                }
                // An active C++ frame with `cthr->Def == nullptr` has no
                // implicit definition, even if the copied host context still
                // carries an affected object for deferred state folding.
                Some(None) => return Ok(Value::Nil),
                // Direct unit host contexts have no active VM frame. Retain
                // their historical synthetic ActionLibrary fallback.
                None => match context.object_context() {
                    Some(object) => (&object.action_library, None),
                    None => return Ok(Value::Nil),
                },
            },
        };
        if !library.is_declared(action.as_ref()) {
            return Ok(Value::Nil);
        }
        if let Some(reflection) = library.reflection(action.as_ref()) {
            return Ok(reflection
                .get(entry.as_ref(), entry_index)
                .unwrap_or(Value::Nil));
        }
        let Some(spec) = library.specs().get(action.as_ref()) else {
            return Ok(Value::Nil);
        };

        // Synthetic fixtures have no resource-backed exact reflection. Keep
        // their historical modeled values and derive the newly covered table
        // entries from ActionSpec/graphics with C++ defaults.
        let graphics = graphics.and_then(|graphics| graphics.get(action.as_ref()));
        if entry.as_ref() == "Facet" {
            let facet = graphics.and_then(|graphics| graphics.facet.as_ref());
            return Ok(match entry_index {
                0 => Value::Int(facet.map_or(0, |facet| facet.x)),
                1 => Value::Int(facet.map_or(0, |facet| facet.y)),
                2 => Value::Int(facet.map_or(0, |facet| facet.width)),
                3 => Value::Int(facet.map_or(0, |facet| facet.height)),
                4 => Value::Int(facet.map_or(0, |facet| facet.target_x)),
                5 => Value::Int(facet.map_or(0, |facet| facet.target_y)),
                _ => Value::Nil,
            });
        }
        if entry_index != 0 {
            return Ok(Value::Nil);
        }

        Ok(match entry.as_ref() {
            "Name" => Value::String(action.clone()),
            "Procedure" => Value::String(spec.procedure.clone().unwrap_or_default().into()),
            "Directions" => Value::Int(spec.directions.unwrap_or(1)),
            "FlipDir" => Value::Int(graphics.and_then(|graphics| graphics.flip_dir).unwrap_or(0)),
            "Length" => Value::Int(spec.length.unwrap_or(1)),
            "Delay" => Value::Int(spec.delay.unwrap_or(0)),
            "Attach" => Value::Int(spec.attach as i32),
            "FacetBase" => Value::Int(i32::from(
                graphics.is_some_and(|graphics| graphics.facet_base),
            )),
            "FacetTopFace" => Value::Int(i32::from(
                graphics.is_some_and(|graphics| graphics.facet_top_face),
            )),
            "FacetTargetStretch" => Value::Int(i32::from(
                graphics.is_some_and(|graphics| graphics.facet_target_stretch),
            )),
            "NextAction" => Value::String(spec.next.clone().unwrap_or_default().into()),
            "StartCall" => Value::String(spec.start_call.clone().unwrap_or_default().into()),
            "EndCall" => Value::String(spec.end_call.clone().unwrap_or_default().into()),
            "AbortCall" => Value::String(spec.abort_call.clone().unwrap_or_default().into()),
            "PhaseCall" => Value::String(spec.phase_call.clone().unwrap_or_default().into()),
            "Sound" => Value::String(spec.sound.clone().unwrap_or_default().into()),
            "NoOtherAction" => Value::Int(i32::from(spec.no_other_action)),
            "ObjectDisabled" => Value::Int(i32::from(spec.disabled)),
            "DigFree" => Value::Int(spec.dig_free.unwrap_or(0)),
            "EnergyUsage" => Value::Int(spec.energy_usage),
            "InLiquidAction" => {
                Value::String(spec.in_liquid_action.clone().unwrap_or_default().into())
            }
            "TurnAction" => Value::String(spec.turn_action.clone().unwrap_or_default().into()),
            "Reverse" => Value::Int(i32::from(graphics.is_some_and(|graphics| graphics.reverse))),
            "Step" => Value::Int(spec.step.unwrap_or(1)),
            _ => Value::Nil,
        })
    })
}

/// The live `C4Object::Shape` reflected by GetObjectVal. Same-call SetShape
/// and persisted shape overrides win over the definition-derived shape.
pub(crate) fn live_object_shape(
    context: &EffectHostContext,
    target: ObjectId,
) -> Option<DefinitionRect> {
    let pending_override = context
        .object_scope(target)
        .and_then(|scope| scope.pending_update.shape_override);
    match pending_override {
        Some(Some(shape)) => return Some(shape),
        Some(None) => {}
        None => {
            if let Some(shape) = context
                .get_world_object(target)
                .and_then(|object| object.full_state().and_then(|state| state.shape_override))
            {
                return Some(shape);
            }
        }
    }

    let scope = context.object_scope(target);
    let world_object = context.get_world_object(target);
    let definition = scope
        .and_then(|scope| {
            scope
                .pending_update
                .change_def
                .clone()
                .or_else(|| scope.definition_id.clone())
        })
        .or_else(|| {
            world_object
                .as_ref()
                .map(|object| object.definition_id().to_string())
        })?;
    let metadata = context.definition_metadata(&definition)?;
    if metadata.line != 0 {
        return metadata.shape;
    }
    let construction = scope
        .map(ObjectScopeContext::construction)
        .or_else(|| world_object.as_ref().map(HostWorldObject::construction))
        .unwrap_or(FULL_CON);
    let rotation = scope
        .map(ObjectScopeContext::rotation)
        .or_else(|| world_object.as_ref().map(|object| object.rotation))
        .unwrap_or(0);
    crate::transformed_shape_rect(
        metadata.shape,
        construction,
        metadata.stretch_growth,
        metadata.rotateable,
        rotation,
    )
}

/// Engine System.c4g wrappers (GetXVal.c:78-79). Nil forwards into
/// FnGetObjectVal and therefore defaults to the calling object.
pub(crate) fn get_obj_width(args: &[Value]) -> Result<Value, RuntimeError> {
    get_obj_dimension(args.first(), "Width")
}

pub(crate) fn get_obj_height(args: &[Value]) -> Result<Value, RuntimeError> {
    get_obj_dimension(args.first(), "Height")
}

fn get_obj_dimension(target: Option<&Value>, entry: &str) -> Result<Value, RuntimeError> {
    get_object_val(&[
        Value::String(entry.to_string().into()),
        Value::Nil,
        target.cloned().unwrap_or(Value::Nil),
    ])
}

/// `FColors[FPlayer..FPlayer + C4MaxColor]` (StdColors.h:32,
/// C4Surface.cpp:1304, C4Constants.h:38). The referenced entries in C4.PAL
/// have no alpha, so FnSetColor packs only their expanded 24-bit RGB value.
const LEGACY_PLAYER_COLOR_INDICES: [usize; 12] = [39, 47, 55, 63, 71, 79, 87, 95, 23, 30, 99, 103];

fn legacy_player_color(index: i32) -> Option<u32> {
    let palette_index = usize::try_from(index)
        .ok()
        .and_then(|index| LEGACY_PLAYER_COLOR_INDICES.get(index))?;
    let offset = palette_index * 3;
    let red = u32::from(LEGACY_GAME_PALETTE[offset]) << 2;
    let green = u32::from(LEGACY_GAME_PALETTE[offset + 1]) << 2;
    let blue = u32::from(LEGACY_GAME_PALETTE[offset + 2]) << 2;
    Some((red << 16) | (green << 8) | blue)
}

fn stage_object_color(target_id: Option<ObjectId>, value: u32) -> bool {
    with_host_context_mut(false, |context| {
        let target = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return false;
        };
        if !context.ensure_object_scope(target) {
            return false;
        }
        let staged = context
            .object_scope_mut(target)
            .map(|object| object.pending_update.color = Some(value))
            .is_some();
        if staged {
            // SetColor/SetColorDw unconditionally run UpdateFace(false),
            // whose UpdateSolidMask keeps the instance but re-puts it.
            context.update_live_solid_mask(target, false);
        }
        staged
    })
}

/// FnSetColor (C4Script.cpp:3635-3645): map an old-gfx player-color index
/// through the game palette and refresh the same object color as SetColorDw.
pub(crate) fn set_color(args: &[Value]) -> Result<Value, RuntimeError> {
    let color_index = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetColor", "value")?;
    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetColor", "target")?;
    let Some(value) = legacy_player_color(color_index) else {
        return Ok(Value::Int(0));
    };
    Ok(Value::Int(i32::from(stage_object_color(target_id, value))))
}

/// FnSetColorDw (C4Script.cpp:3661-3668): set the object's 32-bit color.
pub(crate) fn set_color_dw(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetColorDw", "value")?;
    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetColorDw", "target")?;
    Ok(Value::Bool(stage_object_color(target_id, value as u32)))
}

/// FnSetPicture (C4Script.cpp:3708-3715): write the object's raw picture
/// rectangle. A null explicit target falls back to the calling object; like
/// SetShape, C++ accepts any live object pointer supplied by script.
pub(crate) fn set_picture(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetPicture", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetPicture", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "SetPicture", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "SetPicture", "hgt")?;
    let mut index = 4;
    let explicit_target =
        consume_optional_object_reference_argument(args, &mut index, "SetPicture", "target")?;

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let target = explicit_target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let changed = context
            .object_scope_mut(target)
            .map(|object| {
                object.set_picture_rect(DefinitionRect::new(x, y, width, height));
            })
            .is_some();
        Ok(Value::Bool(changed))
    })
}

/// FnSetShape (C4Script.cpp:5182-5196): overwrite the object's shape rect.
pub(crate) fn set_shape(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetShape", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetShape", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "SetShape", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "SetShape", "hgt")?;
    let mut index = 4;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetShape", "target")?;

    try_with_host_context_mut("SetShape requires an active engine context", |context| {
        let target = target_id.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let changed = context
            .object_scope_mut(target)
            .map(|object| {
                object.pending_update.shape_override =
                    Some(Some(DefinitionRect::new(x, y, width, height)));
            })
            .is_some();
        if changed {
            context.preview_live_object_sector(target);
        }
        Ok(Value::Bool(changed))
    })
}

/// FnSetContactDensity (C4Script.cpp:1286-1291): overwrite the live
/// C4Shape field on the explicit object, or on the calling object when the
/// optional pointer is nil. This is per object rather than a definition
/// mutation; later host calls in this same VM call observe the write.
pub(crate) fn set_contact_density(args: &[Value]) -> Result<Value, RuntimeError> {
    let density = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetContactDensity",
        "density",
    )?;
    let mut index = 1;
    let explicit_target = consume_optional_object_reference_argument(
        args,
        &mut index,
        "SetContactDensity",
        "target",
    )?;

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let target = explicit_target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        object.set_contact_density(density);
        Ok(Value::Bool(true))
    })
}

/// FnAddVertex (C4Script.cpp:1274-1278): append one raw X/Y pair to the
/// current live shape. C4Shape::AddVertex rejects the 31st vertex without
/// enabling C4Object::fOwnVertices (C4Shape.cpp:26-32).
pub(crate) fn add_vertex(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "AddVertex", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "AddVertex", "y")?;
    let explicit_target = args
        .get(2)
        .map(|arg| parse_object_reference_argument(arg, "AddVertex", "object"))
        .transpose()?
        .flatten();

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let target = explicit_target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        let mut vertices = object.shape_vertex_buffer();
        if !vertices.add(x, y) {
            return Ok(Value::Bool(false));
        }
        object.set_shape_vertex_buffer(vertices);
        Ok(Value::Bool(true))
    })
}

/// FnRemoveVertex (C4Script.cpp:1280-1284): remove one active vertex by
/// shifting only its X/Y slots. CNAT/friction and the former trailing slot
/// remain untouched (C4Shape.cpp:346-354), which lets Warp remove all
/// vertices and AddVertex later restore their original slot metadata.
pub(crate) fn remove_vertex(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(args.first().unwrap_or(&Value::Nil), "RemoveVertex", "index")?;
    let explicit_target = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "RemoveVertex", "object"))
        .transpose()?
        .flatten();

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let target = explicit_target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        let mut vertices = object.shape_vertex_buffer();
        if !vertices.remove(index) {
            return Ok(Value::Bool(false));
        }
        object.set_shape_vertex_buffer(vertices);
        Ok(Value::Bool(true))
    })
}

/// FnSetVertex (C4Script.cpp:1292-1326): set one vertex attribute (VTX_X=0,
/// VTX_Y=1, VTX_CNAT=2, VTX_Friction=3); unknown attributes fall back to
/// VtxY like the old-style C++ behaviour.
///
/// Own-vertex mode enters `fOwnVertices` — seeding the shape's backup half
/// from the *definition* shape (C4Shape::CreateOwnOriginalCopy,
/// C4Shape.cpp:484-494) — and writes slot `iIndex + C4D_VertexCpyPos`
/// (index + 15, C4Shape.h:27). `VTX_SetPermanentUpd` (2) then runs
/// `UpdateShape(true)`, which restores the live vertices from that backup
/// half and re-applies Con/rotation (C4Object.cpp:322-350). Plain `VTX_Set`
/// (1) leaves the live shape alone until some later UpdateShape.
pub(crate) fn set_vertex(args: &[Value]) -> Result<Value, RuntimeError> {
    const MAX_VERTEX: usize = 30;
    const VERTEX_CPY_POS: usize = MAX_VERTEX / 2;
    const VTX_SET_PERMANENT_UPD: i32 = 2;
    let index_arg = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetVertex", "index")?;
    let kind = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetVertex", "attribute")?;
    let value = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "SetVertex", "value")?;
    let mut arg_index = 3;
    let target_id =
        consume_optional_object_reference_argument(args, &mut arg_index, "SetVertex", "target")?;
    let own_vertex_mode = match args.get(arg_index) {
        Some(arg) => value_to_i32(arg, "SetVertex", "own vertex mode")?,
        None => 0,
    };

    let Ok(mut slot) = usize::try_from(index_arg) else {
        return Ok(Value::Bool(false));
    };
    if own_vertex_mode != 0 {
        slot += VERTEX_CPY_POS;
    }
    if slot >= MAX_VERTEX {
        return Ok(Value::Bool(false));
    }

    try_with_host_context_mut("SetVertex requires an active engine context", |context| {
        let active = context.object_context().map(|object| object.id());
        // FnSetVertex works on ANY object (`if (!pObj) pObj = cthr->Obj`,
        // C4Script.cpp) — the Gatling aims its crosshair with
        // SetVertex(0, .., pCrosshair). Foreign writes stage into the
        // target's nested scope like every other cross-object fold.
        let foreign = match target_id {
            Some(target) if Some(target) != active => {
                if !context.nested_objects.contains_key(&target) {
                    let Some(world_object) = context.get_world_object(target) else {
                        return Ok(Value::Bool(false));
                    };
                    let Some((scope, local_vars)) = context.nested_scope_for(&world_object) else {
                        return Ok(Value::Bool(false));
                    };
                    context
                        .nested_objects
                        .insert(target, NestedScopeState { scope, local_vars });
                }
                if !context.nested_order.contains(&target) {
                    context.nested_order.push(target);
                }
                Some(target)
            }
            _ => None,
        };
        // UpdateShape reads the live definition shape, so resolve it before
        // borrowing the scope.
        let (definition_vertices, line, stretch_growth, rotateable) = foreign
            .or(active)
            .and_then(|id| context.object_effective_definition_id(id))
            .and_then(|definition_id| context.definition_metadata(&definition_id))
            .map(|metadata| {
                (
                    metadata.vertices.clone(),
                    metadata.line,
                    metadata.stretch_growth,
                    metadata.rotateable,
                )
            })
            .unwrap_or_default();
        let preview_target = foreign.or(active);
        let scope = match foreign {
            Some(target) => {
                &mut context
                    .nested_objects
                    .get_mut(&target)
                    .expect("scope just ensured")
                    .scope
            }
            None => match context.object_context_mut() {
                Some(object) => object,
                None => return Ok(Value::Bool(false)),
            },
        };

        let mut buffer = scope.shape_vertex_buffer();
        if own_vertex_mode != 0 && !scope.staged_own_vertices {
            buffer.create_own_original_copy(&definition_vertices);
            scope.staged_own_vertices = true;
        }
        let Some(vertex) = buffer.slot_mut(slot) else {
            return Ok(Value::Bool(false));
        };
        match kind {
            0 => vertex.x = value,
            2 => vertex.cnat = value as u32,
            3 => vertex.friction = value,
            // VTX_Y and the old-style fallback for any other attribute.
            _ => vertex.y = value,
        }
        let own_base = buffer.own_original_vertices();
        scope.set_shape_vertex_buffer(buffer);
        if own_vertex_mode != 0 {
            // fOwnVertices makes the backup half the object's permanent shape
            // base, so every later UpdateShape restores from it.
            scope.pending_update.vertices = Some(own_base);
            scope.pending_update.vertices_defer_shape_update =
                own_vertex_mode != VTX_SET_PERMANENT_UPD;
        }
        if own_vertex_mode == VTX_SET_PERMANENT_UPD {
            if line == 0 {
                scope.pending_update.shape_override = Some(None);
            }
            scope.refresh_shape_preview_from_parts(
                &definition_vertices,
                line,
                stretch_growth,
                rotateable,
            );
            if let Some(target) = preview_target {
                context.preview_live_object_sector(target);
            }
        }
        Ok(Value::Bool(true))
    })
}

/// FnActIdle (C4Script.cpp:1831-1836): true only for the built-in ActIdle
/// slot. A physical ActMap entry named "Idle" remains an active action.
pub(crate) fn act_idle(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "ActIdle", "target")?;

    with_host_context(Ok(Value::Nil), |context| {
        let idle = if let Some(target) = target_id {
            match context.object_context() {
                Some(object) if target == object.id() => {
                    let name = object.effective_action_name();
                    Some(
                        name.is_empty()
                            || object
                                .action_library
                                .is_idle_entry(name, object.effective_action_index()),
                    )
                }
                _ => context.get_world_object(target).map(|other| {
                    other.action_name.is_empty()
                        || context
                            .definition_metadata(other.definition_id())
                            .map(|metadata| {
                                metadata
                                    .action_library
                                    .is_idle_entry(&other.action_name, other.action_index)
                            })
                            .unwrap_or(other.action_index.is_none() && other.action_name == "Idle")
                }),
            }
        } else {
            context.object_context().map(|object| {
                let name = object.effective_action_name();
                name.is_empty()
                    || object
                        .action_library
                        .is_idle_entry(name, object.effective_action_index())
            })
        };

        Ok(idle.map(Value::Bool).unwrap_or(Value::Nil))
    })
}

/// Live host-side `C4Object::SetOwner`. The callback must run before the
/// outer VM resumes, so this cannot be deferred to ObjectUpdate folding.
fn set_owner_live(target: ObjectId, new_owner: i32) -> bool {
    let staged: Option<Option<i32>> = with_host_context_mut(None, |context| {
        if new_owner != OWNER_NONE && context.player_state(new_owner).is_none() {
            return None;
        }
        if !context.ensure_object_scope(target) {
            return None;
        }

        // Bare host fixtures and a handful of legacy definition-less scopes
        // still carry a valid C4Object owner slot.
        let definition_id = context
            .object_effective_definition_id(target)
            .unwrap_or_default();
        let graphics_definition_id = context
            .object_scope(target)
            .and_then(|scope| {
                scope
                    .base_graphics
                    .as_ref()
                    .map(|graphics| graphics.definition.as_str().to_string())
            })
            .unwrap_or_else(|| definition_id.clone());
        let color_by_owner = context
            .world
            .definition_color_by_owner(&graphics_definition_id);
        let owner_color = (new_owner != OWNER_NONE && color_by_owner).then(|| {
            context
                .player_state(new_owner)
                .and_then(|player| player.color)
                .map(|color| {
                    u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                })
                .unwrap_or(0)
        });

        let (old_owner, flag_base_target, owner_changed, plr_view_range) = {
            let object = context.object_scope_mut(target)?;
            // C++ refreshes the currently selected ColorByOwner graphics
            // before its same-owner early return.
            if let Some(color) = owner_color {
                object.pending_update.color = Some(color);
            }
            let old_owner = object.owner();
            let flag_base_target = (definition_id == "FLAG"
                && object.effective_action_name() == "FlyBase")
                .then(|| object.effective_action_target(0))
                .flatten();
            let owner_changed = old_owner != new_owner;
            if owner_changed {
                object.set_owner(new_owner);
            }
            (
                old_owner,
                flag_base_target,
                owner_changed,
                object.plr_view_range(),
            )
        };

        if owner_changed {
            context.world.change_player_fow_view_object_owner(
                target,
                old_owner,
                new_owner,
                plr_view_range,
            );
        }

        // C4Object::SetOwner refreshes selected ColorByOwner graphics before
        // its same-owner early return. UpdateFace(false) performs an ordinary
        // sequence-preserving solid-mask re-put.
        if new_owner != OWNER_NONE && color_by_owner {
            context.update_live_solid_mask(target, false);
        }
        if !owner_changed {
            return Some(None);
        }

        // A flying flag transfers only a still-present base that belongs to
        // the old owner. Inactive targets have nonzero C++ Status and count.
        if let Some(base_target) = flag_base_target {
            let base = context
                .object_scope(base_target)
                .and_then(|scope| scope.pending_update.base)
                .or_else(|| {
                    context
                        .get_world_object(base_target)
                        .and_then(|object| object.full_state().map(|state| state.base))
                });
            if context.object_status_present(base_target)
                && base == Some(old_owner)
                && context.ensure_object_scope(base_target)
            {
                if let Some(base) = context.object_scope_mut(base_target) {
                    base.pending_update.base = Some(new_owner);
                }
            }
        }
        Some(Some(old_owner))
    });

    let Some(old_owner) = staged else {
        return false;
    };
    if let Some(old_owner) = old_owner {
        let present = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .is_some_and(|context| context.object_status_present(target))
        });
        if present {
            let _ = call_inflight_object_own_fail_safe(
                target,
                "OnOwnerChanged",
                &[Value::Int(new_owner), Value::Int(old_owner)],
            );
        }
    }
    true
}

pub(crate) fn set_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled iOwner is nil -> 0 (FnSetOwner, C4Script.cpp:820).
    let owner = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetOwner: expected int for owner, got {}",
                other.type_name()
            )));
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

    let target = with_host_context(None, |context| target_id.or(context.script_object_context));
    Ok(Value::Bool(
        target.is_some_and(|target| set_owner_live(target, owner)),
    ))
}

pub(crate) fn set_alive(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled nalv is nil -> false (FnSetAlive, C4Script.cpp:813).
    let alive = match args.first().unwrap_or(&Value::Nil) {
        Value::Bool(flag) => *flag,
        Value::Int(value) => *value != 0,
        Value::Nil => false,
        other => {
            return Err(RuntimeError::new(format!(
                "SetAlive: expected bool, int, or nil for alive, got {}",
                other.type_name()
            )));
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

    let updated =
        try_with_host_context_mut("SetAlive requires an active engine context", |context| {
            let Some(target) =
                target_id.or_else(|| context.object_context().map(|object| object.id()))
            else {
                return Ok(Value::Bool(false));
            };
            if !context.ensure_object_scope(target) {
                return Ok(Value::Bool(false));
            }
            let Some(object) = context.object_scope_mut(target) else {
                return Ok(Value::Bool(false));
            };

            object.set_alive(alive);
            Ok(Value::Bool(true))
        })?;
    if updated == Value::Bool(true) {
        let target = with_host_context(None, |context| {
            target_id.or_else(|| context.object_context().map(ObjectScopeContext::id))
        });
        if let Some(target) = target {
            with_host_context_mut((), |context| {
                let _ = refresh_live_object_ocf(context, target);
            });
        }
    }
    Ok(updated)
}

pub(crate) fn get_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetOwner expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetOwner", "target")?;
    }

    with_host_context(Ok(Value::Int(OWNER_NONE)), |context| {
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

/// FnGetController (C4Script.cpp:1316-1320): the object's Controller,
/// NO_OWNER without an object.
pub(crate) fn get_controller(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetController expects at most 1 argument: target",
        ));
    }

    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetController", "target"))
        .transpose()?
        .flatten();

    with_host_context(Ok(Value::Int(OWNER_NONE)), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if target == object.id() {
                    return Ok(Value::Int(object.controller()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(Value::Int(other.controller()));
            }
            return Ok(Value::Int(OWNER_NONE));
        }

        let controller = context
            .object_context()
            .map(|object| object.controller())
            .unwrap_or(OWNER_NONE);
        Ok(Value::Int(controller))
    })
}

/// FnGetKiller (C4Script.cpp:1333-1337): read the object's
/// LastEnergyLossCausePlayer, defaulting a nil target to the calling object
/// and returning NO_OWNER without an object.
pub(crate) fn get_killer(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetKiller expects at most 1 argument: target",
        ));
    }

    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetKiller", "target"))
        .transpose()?
        .flatten();

    with_host_context(Ok(Value::Int(OWNER_NONE)), |context| {
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Int(OWNER_NONE));
        };
        // C++ writes the live field directly, so a SetKiller earlier in the
        // same call must win over the frame-start world snapshot.
        let killer = context
            .object_scope(target)
            .and_then(|scope| scope.pending_update.energy_loss_cause)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.last_energy_loss_cause)
            })
            .unwrap_or(OWNER_NONE);
        Ok(Value::Int(killer))
    })
}

/// FnSetKiller (C4Script.cpp:1339-1347): accept NO_OWNER or a valid player,
/// default a nil target to the calling object, and directly replace
/// LastEnergyLossCausePlayer (without the DoEnergy self-damage guard).
pub(crate) fn set_killer(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "SetKiller expects at most 2 arguments: killer and optional target",
        ));
    }

    let new_killer = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetKiller", "killer")?;
    let target_id = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetKiller", "target"))
        .transpose()?
        .flatten();

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        if new_killer != OWNER_NONE && context.player_state(new_killer).is_none() {
            return Ok(Value::Bool(false));
        }
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(scope) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        scope.pending_update.energy_loss_cause = Some(new_killer);
        Ok(Value::Bool(true))
    })
}

/// FnGetObjectLayer (C4Script.cpp:5160-5166): the object's effective pLayer.
pub(crate) fn get_object_layer(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetObjectLayer expects at most 1 argument: target",
        ));
    }

    let target_id = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetObjectLayer", "target"))
        .transpose()?
        .flatten();

    with_host_context(Ok(Value::Nil), |context| {
        let target = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let layer = target.and_then(|target| context.object_layer(target));
        Ok(layer.map(object_reference_value).unwrap_or(Value::Nil))
    })
}

/// FnSetObjectLayer (C4Script.cpp:5168-5180): set or clear pLayer on the
/// explicit target, defaulting a nil target to the calling object.
pub(crate) fn set_object_layer(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "SetObjectLayer expects at most 2 arguments: layer and target",
        ));
    }
    let layer = args
        .first()
        .map(|value| parse_object_reference_argument(value, "SetObjectLayer", "layer"))
        .transpose()?
        .flatten();
    let target_id = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetObjectLayer", "target"))
        .transpose()?
        .flatten();

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Bool(false));
        };
        let contents = context
            .get_world_object(target)
            .map(|object| object.contents().to_vec())
            .unwrap_or_default();
        if !context.set_object_layer(target, layer) {
            return Ok(Value::Bool(false));
        }
        for content in contents {
            if context
                .get_world_object(content)
                .map(|object| object.is_present())
                .unwrap_or(false)
            {
                context.set_object_layer(content, layer);
            }
        }
        Ok(Value::Bool(true))
    })
}

pub(crate) const GFX_BLIT_CUSTOM: u32 = 128;

/// FnGetObjectBlitMode (C4Script.cpp:5663-5679): read the raw base-object
/// mode or one existing graphics overlay's literal mode.
pub(crate) fn get_object_blit_mode(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetObjectBlitMode expects at most 2 arguments: object and overlay id",
        ));
    }
    let target_id = args
        .first()
        .map(|value| parse_object_reference_argument(value, "GetObjectBlitMode", "object"))
        .transpose()?
        .flatten();
    let overlay_id = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "GetObjectBlitMode",
        "overlay id",
    )?;

    with_host_context(Ok(Value::Nil), |context| {
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Nil);
        };
        let mode = if overlay_id == 0 {
            context.object_blit_mode(target)
        } else {
            context.object_overlay_blit_mode(target, overlay_id)
        };
        Ok(mode
            .map(|mode| Value::Int(mode as i32))
            .unwrap_or(Value::Nil))
    })
}

/// FnSetObjectBlitMode (C4Script.cpp:5634-5661): base-object nonzero modes
/// gain CUSTOM, zero resets to the effective definition default; overlays
/// store the literal mode and return true rather than their previous value.
pub(crate) fn set_object_blit_mode(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "SetObjectBlitMode expects at most 3 arguments: mode, object and overlay id",
        ));
    }
    let requested = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetObjectBlitMode",
        "mode",
    )? as u32;
    let target_id = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetObjectBlitMode", "object"))
        .transpose()?
        .flatten();
    let overlay_id = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "SetObjectBlitMode",
        "overlay id",
    )?;

    with_host_context_mut(Ok(Value::Nil), |context| {
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Nil);
        };

        if overlay_id != 0 {
            return Ok(
                if context.set_object_overlay_blit_mode(target, overlay_id, requested) {
                    Value::Int(1)
                } else {
                    Value::Nil
                },
            );
        }

        let Some(previous) = context.object_blit_mode(target) else {
            return Ok(Value::Nil);
        };
        let mode = if requested == 0 {
            context.object_definition_blit_mode(target).unwrap_or(0)
        } else {
            requested | GFX_BLIT_CUSTOM
        };
        if !context.set_object_blit_mode(target, mode) {
            return Ok(Value::Nil);
        }
        Ok(Value::Int(previous as i32))
    })
}

/// FnSetController (C4Script.cpp:1322-1331): NO_OWNER always passes, any
/// other value must be a valid player; foreign targets are written via the
/// nested-call seam like RemoveObject.
pub(crate) fn set_controller(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 2 {
        return Err(RuntimeError::new(
            "SetController expects 1 or 2 arguments: controller and optional target",
        ));
    }

    let new_controller = value_to_i32(&args[0], "SetController", "controller")?;
    let target_id = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetController", "target"))
        .transpose()?
        .flatten();

    let (valid_player, active) = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref();
        (
            context
                .map(|context| context.player_state(new_controller).is_some())
                .unwrap_or(false),
            context.and_then(|context| context.object_context().map(|object| object.id())),
        )
    });
    // validate player (C4Script.cpp:1325)
    if new_controller != OWNER_NONE && !valid_player {
        return Ok(Value::Bool(false));
    }

    if let Some(target) = target_id {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "SetController",
                &[Value::Int(new_controller)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };
        object.set_controller(new_controller);
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_alive(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetAlive expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetAlive", "target")?;
    }

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            // C++ reads the live object. A foreign target may already have
            // a nested scope because Kill/SetAlive touched it earlier in
            // this same VM call; that scope must beat the frame snapshot.
            if let Some(object) = context.object_scope(target) {
                return Ok(Value::Bool(object.alive()));
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
