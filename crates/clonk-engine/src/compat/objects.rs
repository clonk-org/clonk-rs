use super::*;

const OWNER_ANY: i32 = -2;
pub(crate) const ANY_CONTAINER_SENTINEL: i32 = 123;
const NO_CONTAINER_SENTINEL: i32 = 124;

/// One queue-time-resolved `C4ObjResort::OrderFunc`. C++ stores the resolved
/// `C4AulFunc *`, rather than looking the function name up again when the
/// deferred resort executes. The immutable resolution pins that body and
/// overload chain; the stable host identity supplies live native/global state
/// without carrying the Rc-based ScriptEngine through Send + Sync errors.
#[derive(Clone)]
#[doc(hidden)]
pub struct ObjectOrderFunction {
    pub(crate) host_identity: clonk_script::ScriptHostIdentity,
    pub(crate) resolution: clonk_script::ScriptFunctionResolution,
    pub(crate) script_name: String,
    pub(crate) definition_context: Option<DefinitionId>,
    pub(crate) function: String,
    /// The resolved SFunc is owned by Game.ScriptEngine. Enable exact
    /// LinkedTo-local lookup while invoking the pinned function body.
    pub(crate) engine_global: bool,
}

impl std::fmt::Debug for ObjectOrderFunction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObjectOrderFunction")
            .field("host_identity", &self.host_identity)
            .field("script_name", &self.script_name)
            .field("definition_context", &self.definition_context)
            .field("function", &self.function)
            .field("engine_global", &self.engine_global)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ObjectOrderFunction {
    fn eq(&self, other: &Self) -> bool {
        self.host_identity == other.host_identity
            && self.resolution == other.resolution
            && self.script_name == other.script_name
            && self.definition_context == other.definition_context
            && self.function == other.function
            && self.engine_global == other.engine_global
    }
}

impl Eq for ObjectOrderFunction {}

/// Deferred object-list ordering work. C++ resolves `C4Object::Resort` flags
/// before executing the newest `C4ObjResort` request (`SetObjectOrder`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub enum ObjectOrderCommand {
    #[doc(hidden)]
    SetRelative {
        relative_to: ObjectId,
        object: ObjectId,
        after: bool,
    },
    #[doc(hidden)]
    ResortObject(ObjectId),
    /// Trigger-only post-CrossCheck sweep of every object still carrying
    /// C4Object::Unsorted. Ordinary `Resort()` calls retain their object id
    /// in `ResortObject`; the engine emits this only for the frame sweep.
    #[doc(hidden)]
    ResortUnsortedSweep,
    #[doc(hidden)]
    SortByCategory,
    #[doc(hidden)]
    OrderFuncAll {
        order: ObjectOrderFunction,
        category: i32,
    },
    #[doc(hidden)]
    OrderFuncObject {
        order: ObjectOrderFunction,
        object: ObjectId,
    },
}

/// FnGetComponent (C4Script.cpp:2685-2709): with `idDef` the def's
/// component list answers; otherwise the object's (scope object when no
/// target). `idComponent` selects the count form, else the indexed form.
/// Object component order is the live C4IDList insertion order, including
/// dynamically added zero-count entries (C4IDList.cpp:38-45,85-103).
pub(crate) fn get_component(args: &[Value]) -> Result<Value, RuntimeError> {
    let component = parse_native_c4id_argument(args.first(), "GetComponent")?;
    let index = parse_optional_i32(args.get(1), "GetComponent", "index")?.unwrap_or(0);
    let target =
        parse_object_reference_argument(args.get(2).unwrap_or(&Value::Nil), "GetComponent", "obj")?;
    let definition = parse_native_c4id_argument(args.get(3), "GetComponent")?;

    let indexed = |components: &[(String, i32)], index: i32| -> Value {
        usize::try_from(index)
            .ok()
            .and_then(|index| components.get(index))
            .map(|(id, _)| Value::C4Id(id.clone()))
            .unwrap_or(Value::Nil)
    };
    if let Some(definition) = definition {
        // C4Def::GetComponentCount/GetIndexedComponent run the definition's
        // GetCustomComponents with cthr->Obj as the builder. Capture that
        // object before the nested callback changes or removes it.
        let builder = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            context.definition_metadata(&definition)?;
            Some(context.script_object_context)
        });
        let Some(builder) = builder else {
            return Ok(Value::Nil);
        };
        let components = resolve_component_list(&definition, None, builder)?;
        if let Some(component) = component {
            let count = components
                .iter()
                .find(|(id, _)| id.eq_ignore_ascii_case(&component))
                .map(|(_, count)| *count)
                .unwrap_or(0);
            return Ok(Value::Int(count));
        }
        return Ok(indexed(&components, index));
    }

    with_host_context(Ok(Value::Nil), |context| {
        let object = match target {
            Some(id) => context.get_world_object(id),
            None => context
                .object_context()
                .map(|object| object.id())
                .and_then(|id| context.get_world_object(id)),
        };
        let Some(object) = object else {
            return Ok(Value::Nil);
        };
        let state_components = context
            .object_scope(object.id)
            .and_then(|scope| scope.pending_update.components.clone())
            .or_else(|| object.full_state().map(|state| state.components.clone()));
        let state_order = context
            .object_scope(object.id)
            .and_then(|scope| scope.pending_update.component_order.clone())
            .or_else(|| {
                object
                    .full_state()
                    .map(|state| state.component_order.clone())
            });
        let def_order = context
            .world
            .definition_metadata(object.definition_id())
            .map(|metadata| metadata.components.clone())
            .unwrap_or_default();
        if let Some(component) = component {
            let count = if let Some(components) = state_components.as_ref() {
                components
                    .iter()
                    .find(|(id, _)| id.as_str().eq_ignore_ascii_case(&component))
                    .map(|(_, count)| count)
                    .unwrap_or(0)
            } else {
                def_order
                    .iter()
                    .find(|(id, _)| id.eq_ignore_ascii_case(&component))
                    .map(|(_, count)| *count)
                    .unwrap_or(0)
            };
            return Ok(Value::Int(count));
        }
        let runtime_order = state_order
            .map(|order| {
                order
                    .into_iter()
                    .map(|id| {
                        let count = state_components
                            .as_ref()
                            .and_then(|components| components.get(&id))
                            .unwrap_or(0);
                        (id.as_str().to_string(), count)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or(def_order);
        Ok(indexed(&runtime_order, index))
    })
}

/// FnGetNeededMatStr (C4Script.cpp:4494-4499;
/// C4Object.cpp:6234-6265): format the target's outstanding definition
/// components against its invested `Component` ledger. `Contents` are not
/// counted. The executing object is both the omitted target and the builder
/// passed to a definition's GetCustomComponents callback; arrow dispatch
/// swaps that context to the receiver before reaching this host.
pub(crate) fn get_needed_mat_str(args: &[Value]) -> Result<Value, RuntimeError> {
    if matches!(args.first(), Some(Value::Proplist(_))) {
        return Err(RuntimeError::new(
            "GetNeededMatStr: expected object, nil, or 0 for target, got proplist",
        ));
    }
    let explicit_target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GetNeededMatStr",
        "target",
    )?;
    // Capture Def before GetCustomComponents runs. The callback may mutate
    // the live object, but C++ has already selected `pObj->Def` for the
    // component query at that point.
    let target_and_recipe = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let target = explicit_target.or(context.script_object_context)?;
        let definition = context.object_effective_definition_id(target)?;
        Some((target, context.script_object_context, definition))
    });
    let Some((target, builder, recipe_definition)) = target_and_recipe else {
        return Ok(Value::Nil);
    };

    let needed_components = resolve_component_list(&recipe_definition, None, builder)?;

    with_host_context(Ok(Value::Nil), |context| {
        // Re-read live state after GetCustomComponents: synchronous callback
        // writes are already visible to the following C++ subtraction/name.
        let current_components = context
            .object_scope(target)
            .and_then(|scope| scope.pending_update.components.clone())
            .or_else(|| {
                context
                    .get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.components.clone()))
            })
            .unwrap_or_default();
        let target_name = context
            .object_custom_name(target)
            .or_else(|| match context.object_scope(target) {
                Some(scope)
                    if !scope.destroy && !matches!(scope.status(), ObjectStatus::Deleted) =>
                {
                    scope.info_core().map(|info| info.name.clone())
                }
                Some(_) => None,
                None => context
                    .world
                    .crew_infos
                    .get(&target)
                    .map(|info| info.name.clone()),
            })
            .or_else(|| {
                context
                    .object_effective_definition_id(target)
                    .and_then(|definition| {
                        context
                            .definition_metadata(&definition)
                            .map(|metadata| metadata.name.clone())
                    })
            })
            .unwrap_or_else(|| recipe_definition.clone());
        let display_name = |id: &str| {
            context
                .definition_metadata(id)
                .map(|metadata| metadata.name.as_str())
                .unwrap_or(id)
                .to_owned()
        };

        let mut missing = String::new();
        for (component, required) in needed_components {
            if required == 0 {
                continue;
            }
            let current = current_components
                .iter()
                .find(|(id, _)| id.as_str().eq_ignore_ascii_case(&component))
                .map(|(_, count)| count)
                .unwrap_or(0);
            let deficit = required.wrapping_sub(current);
            if deficit > 0 {
                missing.push_str(&format!("|{deficit}x {}", display_name(&component)));
            }
        }

        if missing.is_empty() {
            Ok(Value::String(
                context
                    .world
                    .needed_material_strings
                    .format_none(&target_name)
                    .into(),
            ))
        } else {
            Ok(Value::String(
                format!(
                    "{}{missing}",
                    context
                        .world
                        .needed_material_strings
                        .format_need(&target_name)
                )
                .into(),
            ))
        }
    })
}

/// FnComponentAll (C4Script.cpp:1873-1883): the explicit object is required;
/// execute its definition's GetCustomComponents with the queried object as
/// `this` and the caller as its builder argument before falling back to the
/// live instance ledger, then reject every positive-count foreign component.
pub(crate) fn component_all(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "ComponentAll",
        "obj",
    )?;
    let component = parse_native_c4id_argument(args.get(1), "ComponentAll")?;
    let Some(target) = target else {
        return Ok(Value::Nil);
    };

    // Capture Def and cthr->Obj before the callback. The overload may mutate
    // either object, but C++ has already selected the recipe definition and
    // builder passed to C4Def::GetComponents at that point.
    let definition_and_builder = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        Some((
            context.object_effective_definition_id(target)?,
            context.script_object_context,
        ))
    });
    let Some((definition, builder)) = definition_and_builder else {
        return Ok(Value::Nil);
    };

    let components = resolve_component_list(&definition, Some(target), builder)?;
    Ok(Value::Bool(components.iter().all(|(id, count)| {
        *count <= 0
            || component
                .as_deref()
                .is_some_and(|component| id.eq_ignore_ascii_case(component))
    })))
}

/// FnChangeDef (C4Script.cpp) -> C4Object::ChangeDef. The lifecycle is
/// synchronous: a contained object silently exits at (0,0), the old action
/// resets, the new definition becomes callback-visible, and RejectEntrance
/// decides whether a no-calls Enter restores the saved container.
pub(crate) fn change_def(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(new_id) = parse_native_c4id_argument(args.first(), "ChangeDef")? else {
        return Ok(Value::Bool(false));
    };
    let target =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "ChangeDef", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.script_object_context)
    });
    // The optional object argument selects the native receiver; it does not
    // perform a second script lookup on that object's definition. A script
    // function also named ChangeDef therefore cannot shadow FnChangeDef.
    let target = target.or(active);
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(change_def_live(target, &new_id)?))
}

/// Native host-side C4Object::ChangeDef used both by FnChangeDef and
/// BurnTurnTo. It bypasses script function resolution but keeps every
/// callback that the C++ object method itself performs.
pub(crate) fn change_def_live(target: ObjectId, new_id: &str) -> Result<bool, RuntimeError> {
    let known = with_host_context_mut(false, |context| {
        context.definition_metadata(new_id).is_some() && context.ensure_object_scope(target)
    });
    if !known {
        return Ok(false);
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if let Some(scope) = borrow
            .as_mut()
            .and_then(|context| context.object_scope_mut(target))
        {
            // This marker describes the most recent ChangeDef lifecycle,
            // not merely an initial Exit. A later failed re-entry must clear
            // an earlier successful marker in the same staged update.
            scope.pending_update.change_def_reinsert = false;
            scope.pending_update.change_def_contents_sort = None;
            scope.pending_update.change_def_reset_action_time = false;
        }
    });

    let previous_container = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(target))
            .and_then(|object| object.container())
    });
    if let Some(previous) = previous_container {
        let _ = exit_object_at_position_with_calls(target, Vector2::ZERO, false)?;
        with_host_context_mut((), |context| {
            context.relink_content_after_exit(previous, target);
        });
    }

    // SetAction(ActIdle) runs on the OLD definition, including its ordinary
    // Start/Abort callbacks. NoOtherAction may reject it, but ChangeDef then
    // forcibly stores ActIdle before swapping Def.
    let action_was_idle = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(target))
            .is_some_and(|object| {
                object.action_library.is_idle_entry(
                    object.effective_action_name(),
                    object.effective_action_index(),
                )
            })
    });
    let action_applied = native_set_action_by_name(target, "Idle")?;

    let staged = with_host_context_mut(false, |context| {
        let Some(metadata) = context.definition_metadata(new_id).cloned() else {
            return false;
        };
        let world = context.world.clone();
        let current_color = context
            .object_scope(target)
            .and_then(|object| object.pending_update.color)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.color))
            })
            .unwrap_or(0);
        let owner = context.object_scope(target).map(ObjectScopeContext::owner);
        let changed_color = if !context.world.definition_color_by_owner(new_id) {
            Some(0)
        } else if current_color == 0 {
            owner.and_then(|owner| {
                context.world.player(owner).and_then(|player| {
                    player.color.map(|color| {
                        u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                    })
                })
            })
        } else {
            None
        };
        let follows_definition = context
            .object_blit_mode(target)
            .is_none_or(|mode| mode & GFX_BLIT_CUSTOM == 0);
        let had_pending_blit = context
            .object_scope(target)
            .is_some_and(|object| object.pending_update.blit_mode.is_some());
        let pre_swap_contents_sort = context.get_world_object(target).and_then(|object| {
            object.container().map(|container| ChangeDefContentsSort {
                container,
                category: object.category,
                definition_id: object.definition_id().to_string(),
                unsorted: object.unsorted,
            })
        });
        let Some(object) = context.object_scope_mut(target) else {
            return false;
        };

        if let Some(sort) = pre_swap_contents_sort {
            object.pending_update.change_def_reinsert = true;
            object.pending_update.change_def_contents_sort = Some(sort);
        }
        // SetAction resets Time only when Act actually changes. Its boolean
        // return means success, not change; ChangeDef while already Idle
        // preserves a nonzero saved/reentrant Time like C++.
        object.pending_update.change_def_reset_action_time = action_applied && !action_was_idle;

        // C++ writes Action.Act=ActIdle unconditionally after SetAction and
        // all of its callbacks. If an AbortCall selected another action, its
        // Time/Data/Phase/targets remain, but the action slot becomes Idle.
        let action = object
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        action.set_name("Idle".to_string());
        action.set_force(true);
        action.callbacks_dispatched = true;
        object.update_effective_action("Idle");

        // Earlier graphics/mask writes are overwritten at the exact swap
        // point; later statements can stage new overrides normally.
        object.pending_update.solid_mask_override = None;
        object.set_base_graphics(None);
        if let Some(color) = changed_color {
            object.pending_update.color = Some(color);
        }
        if follows_definition && had_pending_blit {
            object.pending_update.blit_mode = Some(metadata.blit_mode);
        }
        object.install_definition_preview(new_id, &metadata);
        object.configure_fair_crew(&world);
        true
    });
    if !staged {
        return Ok(false);
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.preview_live_object_sector(target);
            context.update_live_solid_mask(target, true);
            let _ = refresh_live_object_ocf(context, target);
        }
    });

    if let Some(previous) = previous_container {
        // Return value intentionally ignored: C4Object::ChangeDef succeeds
        // even when new-def RejectEntrance or later Enter checks veto.
        if enter_object_live_with_calls(target, previous, false)? {
            HOST_CONTEXT.with(|cell| {
                let mut borrow = cell.borrow_mut();
                if let Some(scope) = borrow
                    .as_mut()
                    .and_then(|context| context.object_scope_mut(target))
                {
                    scope.pending_update.change_def_reinsert = true;
                    scope.pending_update.change_def_contents_sort = None;
                }
            });
        }
    }
    Ok(true)
}

pub(crate) fn object_is_present(target: ObjectId) -> bool {
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.object_status_present(target))
    })
}

pub(crate) fn first_retained_content(
    context: &EffectHostContext,
    target: ObjectId,
) -> Option<ObjectId> {
    context
        .get_world_object(target)?
        .contents()
        .iter()
        .copied()
        .find(|content| context.object_status_present(*content))
}

fn retained_contents_count(context: &EffectHostContext, target: ObjectId) -> usize {
    context
        .get_world_object(target)
        .map(|object| {
            object
                .contents()
                .iter()
                .filter(|content| context.object_status_present(**content))
                .count()
        })
        .unwrap_or_default()
}

/// C++ truthiness of a raw object Status. Inactive objects still receive
/// callbacks; only Deleted/status-zero objects are suppressed.
pub(crate) fn object_has_status(target: ObjectId) -> bool {
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.object_status_present(target))
    })
}

fn live_collection_eligible(
    context: &EffectHostContext,
    target: ObjectId,
    ignore_no_collect_delay: bool,
) -> bool {
    let Some(scope) = context.object_scope(target) else {
        return false;
    };
    let Some(definition_id) = context.object_effective_definition_id(target) else {
        return false;
    };
    let Some(metadata) = context.definition_metadata(&definition_id) else {
        return false;
    };
    let construction_ready = scope.construction() >= FULL_CON || metadata.fire.incomplete_activity;
    let positive_rect = metadata
        .fire
        .collection_rect
        .is_some_and(|rect| rect.width > 0 && rect.height > 0);
    let below_limit = !crate::collection_limit_reached(
        metadata.collection_limit,
        retained_contents_count(context, target),
    );
    construction_ready
        && positive_rect
        && below_limit
        && !scope.action_library.disables_object_for_entry(
            scope.effective_action_name(),
            scope.effective_action_index(),
        )
        && (ignore_no_collect_delay || scope.no_collect_delay() == 0)
}

/// Synchronous host-side portion of C4Object::SetOCF that depends on live
/// motion/containment/terrain. Definition-static bits remain in the cached
/// seed; every field this helper rebuilds is cleared first like C++.
pub(crate) fn refresh_live_object_ocf(context: &mut EffectHostContext, target: ObjectId) -> bool {
    if !context.ensure_object_scope(target) {
        return false;
    }
    let Some(definition_id) = context.object_effective_definition_id(target) else {
        return false;
    };
    let Some(metadata) = context.definition_metadata(&definition_id).cloned() else {
        return false;
    };
    let Some((
        position,
        container,
        construction,
        rotation,
        energy,
        on_fire,
        alive,
        category,
        action_disabled,
    )) = context.object_scope(target).map(|scope| {
        (
            scope.effective_position(),
            scope.container(),
            scope.construction(),
            scope.rotation(),
            scope.energy(),
            scope.ocf() & ocf::ON_FIRE != 0,
            scope.alive(),
            scope.category(),
            scope.action_library.disables_object_for_entry(
                scope.effective_action_name(),
                scope.effective_action_index(),
            ),
        )
    })
    else {
        return false;
    };
    let collection = live_collection_eligible(context, target, false);
    let prey = metadata
        .fire
        .def_core_values
        .def_core
        .get("Prey")
        .and_then(|values| values.first())
        .is_some_and(|value| matches!(value, DefCorePrimitive::Int(value) if *value != 0));
    let solid_center = context
        .landscape_ref()
        .is_some_and(|landscape| landscape.is_solid_at(position.x, position.y));
    let semi_above = context.landscape_ref().is_some_and(|landscape| {
        landscape.is_semi_solid_at(position.x, position.y.saturating_sub(1))
    });
    let solid_above = context
        .landscape_ref()
        .is_some_and(|landscape| landscape.is_solid_at(position.x, position.y.saturating_sub(1)));
    let semi_high = context.landscape_ref().is_some_and(|landscape| {
        landscape.is_semi_solid_at(position.x, position.y.saturating_sub(8))
    });
    let container_allows_get = container.is_some_and(|container| {
        let grab_get = context
            .object_effective_definition_id(container)
            .and_then(|id| context.definition_metadata(&id))
            .is_some_and(|metadata| metadata.grab_put_get & 2 != 0);
        let entrance = context
            .object_scope(container)
            .map(ObjectScopeContext::ocf)
            .or_else(|| {
                context
                    .get_world_object(container)
                    .map(|object| object.ocf())
            })
            .is_some_and(|mask| mask & ocf::ENTRANCE != 0);
        grab_get || entrance
    });

    let Some(scope) = context.object_scope_mut(target) else {
        return false;
    };
    scope.refresh_cached_ocf();
    let mut mask = scope.cached_ocf.unwrap_or(ocf::NORMAL);
    // Every bit whose SetOCF predicate depends directly or indirectly on
    // Con is rebuilt from the live definition/scope. DoCon calls SetOCF
    // before UpdateFace and before any lifecycle callback.
    mask &= !(ocf::CONSTRUCT
        | ocf::INFLAMMABLE
        | ocf::FULL_CON
        | ocf::ROTATE
        | ocf::ENTRANCE
        | ocf::COLLECTION
        | ocf::LINE_CONSTRUCT
        | ocf::ATTRACT_LIGHTNING
        | ocf::POWER_CONSUMER
        | ocf::POWER_SUPPLY
        | ocf::CONTAINER
        | ocf::FIGHT_READY
        | ocf::PREY);
    if metadata.constructable && construction < FULL_CON && rotation == 0 && !on_fire {
        mask |= ocf::CONSTRUCT;
    }
    // SetOCF, unlike UpdateOCF, rebuilds the definition/alive-dependent
    // inflammability bit. AssignDeath relies on this exact distinction:
    // its raw Alive=false leaves the old bit visible to RemoveDeath, while
    // SetAction("Dead") clears it before Start/Abort/Death callbacks
    // (oracle-src-pinned src/C4Object.cpp:562-566,1164-1205).
    if !on_fire
        && metadata.fire.contact_incinerate > 0
        && (category & crate::CATEGORY_LIVING == 0 || alive)
    {
        mask |= ocf::INFLAMMABLE;
    }
    if construction >= FULL_CON {
        mask |= ocf::FULL_CON;
    }
    if metadata.rotateable != 0 && construction > 100 {
        mask |= ocf::ROTATE;
    }
    if metadata
        .fire
        .entrance_rect
        .is_some_and(|rect| rect.width > 0 && rect.height > 0)
        && mask & ocf::FULL_CON != 0
        && (metadata.fire.rotated_entrance == 1 || rotation <= metadata.fire.rotated_entrance)
    {
        mask |= ocf::ENTRANCE;
    }
    if collection {
        mask |= ocf::COLLECTION;
    }
    if mask & ocf::FULL_CON != 0 && metadata.line_connect & !crate::LINE_CONNECT_ENERGY_HOLDER != 0
    {
        mask |= ocf::LINE_CONSTRUCT;
    }
    if metadata.fire.attract_lightning && mask & ocf::FULL_CON != 0 {
        mask |= ocf::ATTRACT_LIGHTNING;
    }
    if metadata.line_connect & crate::LINE_CONNECT_POWER_CONSUMER != 0 && mask & ocf::FULL_CON != 0
    {
        mask |= ocf::POWER_CONSUMER;
    }
    if (metadata.line_connect & crate::LINE_CONNECT_POWER_GENERATOR != 0
        || (metadata.line_connect & crate::LINE_CONNECT_POWER_OUTPUT != 0 && energy > 0))
        && mask & ocf::FULL_CON != 0
    {
        mask |= ocf::POWER_SUPPLY;
    }
    if metadata.grab_put_get & 3 != 0 || mask & ocf::ENTRANCE != 0 {
        mask |= ocf::CONTAINER;
    }
    if alive
        && category & crate::CATEGORY_LIVING != 0
        && !action_disabled
        && !metadata.fire.no_fight
    {
        mask |= ocf::FIGHT_READY;
    }
    if prey && alive {
        mask |= ocf::PREY;
    }
    mask &= !(ocf::IN_SOLID | ocf::IN_FREE | ocf::AVAILABLE);
    if container.is_none() {
        if solid_center {
            mask |= ocf::IN_SOLID;
        }
        if !semi_above {
            mask |= ocf::IN_FREE;
        }
    }
    if (container.is_none() || container_allows_get)
        && (!semi_above || (!solid_above && !semi_high))
    {
        mask |= ocf::AVAILABLE;
    }
    scope.cached_ocf = Some(mask);
    true
}

fn refresh_container_collection_ocf(context: &mut EffectHostContext, container: ObjectId) {
    let _ = refresh_live_object_ocf(context, container);
}

/// Direct `C4Object::Exit(x, y)` with absolute coordinates. Unlike the
/// script `Exit` wrapper, these receive no caller-relative or Shape.y
/// adjustment.
fn exit_object_at_position(target: ObjectId, position: Vector2) -> Result<bool, RuntimeError> {
    exit_object_at_position_with_calls(target, position, true)
}

/// Exact world-space shape for a callback-live object. This is deliberately
/// separate from `WorldAccessor::object_shape_rect`, whose sector/legacy-At
/// contract includes `C4Object::addtop`.
pub(crate) fn effect_object_live_shape_rect(
    context: &EffectHostContext,
    object: &HostWorldObject,
) -> DefinitionRect {
    live_object_bounds_shape(context, object.id)
        .map(|shape| {
            DefinitionRect::new(
                object.position().x.saturating_add(shape.x),
                object.position().y.saturating_add(shape.y),
                shape.width,
                shape.height,
            )
        })
        .unwrap_or_else(|| DefinitionRect::new(object.position().x, object.position().y, 1, 1))
}

/// The pLayer arm of C4Object::SideBounds/VerticalBounds
/// (C4Movement.cpp:187-201, 209-223), including the DFA_ATTACH skip and the
/// C4D_StaticBack shape-offset branch.
fn live_layer_bounds(
    context: &EffectHostContext,
    target: ObjectId,
    horizontal: bool,
) -> Option<(i32, i32)> {
    let object = context.get_world_object(target)?;
    let definition_id = context.object_effective_definition_id(target)?;
    let metadata = context.definition_metadata(&definition_id)?;
    let (action_name, action_index) = context
        .object_scope(target)
        .map(|scope| {
            (
                scope.effective_action_name(),
                scope.effective_action_index(),
            )
        })
        .unwrap_or((object.action_name.as_str(), object.action_index));
    let procedure = metadata
        .action_library
        .procedure_for_entry(action_name, action_index);
    if !metadata
        .action_library
        .is_idle_entry(action_name, action_index)
        && matches!(procedure, ActionProcedure::Attach)
    {
        return None;
    }

    let layer_id = context.object_layer(target)?;
    let layer = context.get_world_object(layer_id)?;
    let layer_definition_id = context.object_effective_definition_id(layer_id)?;
    let layer_metadata = context.definition_metadata(&layer_definition_id)?;
    if layer_metadata.border_bound & C4D_BORDER_LAYER == 0 {
        return None;
    }
    let object_shape = live_object_bounds_shape(context, target).unwrap_or_default();
    let layer_shape = live_object_bounds_shape(context, layer_id).unwrap_or_default();
    let is_static = object.category & crate::CATEGORY_STATIC_BACK != 0;
    let (layer_origin, layer_size, shape_offset) = if horizontal {
        (
            layer.position.x.saturating_add(layer_shape.x),
            layer_shape.width,
            object_shape.x,
        )
    } else {
        (
            layer.position.y.saturating_add(layer_shape.y),
            layer_shape.height,
            object_shape.y,
        )
    };
    let low = if is_static {
        layer_origin
    } else {
        layer_origin.saturating_sub(shape_offset)
    };
    let high = if is_static {
        layer_origin.saturating_add(layer_size)
    } else {
        layer_origin
            .saturating_add(layer_size)
            .saturating_add(shape_offset)
    };
    Some((low, high))
}

fn apply_live_target_bounds(
    target: ObjectId,
    coordinate: &mut i32,
    low: i32,
    high: i32,
    low_cnat: u32,
    high_cnat: u32,
) {
    // These are deliberately independent, matching C4Object::TargetBounds.
    if *coordinate < low {
        *coordinate = low;
        run_live_bound_contact(target, low_cnat);
    }
    if *coordinate > high {
        *coordinate = high;
        run_live_bound_contact(target, high_cnat);
    }
}

/// C4Object::BoundsCheck (C4Object.h:392-395) for a live script target:
/// SideBounds then VerticalBounds (C4Movement.cpp:185-229). Only pLayer and
/// map-border limits gated on `Def->BorderBound` — landscape solidity never
/// enters it. Runs Contact callbacks, so callers must hold no object-scope
/// borrow across it.
pub(crate) fn bounds_check_live_object(target: ObjectId, position: &mut Vector2) {
    let layer_side = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| live_layer_bounds(context, target, true))
    });
    if let Some((low, high)) = layer_side {
        apply_live_target_bounds(target, &mut position.x, low, high, CNAT_LEFT, CNAT_RIGHT);
    }

    let landscape_side = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let definition_id = context.object_effective_definition_id(target)?;
        let metadata = context.definition_metadata(&definition_id)?;
        if metadata.border_bound & C4D_BORDER_SIDES == 0 {
            return None;
        }
        let shape_x = live_object_bounds_shape(context, target)
            .map(|shape| shape.x)
            .unwrap_or(0);
        let width = i32::try_from(context.landscape_ref()?.width()).ok()?;
        Some((-shape_x, width.saturating_add(shape_x)))
    });
    if let Some((low, high)) = landscape_side {
        apply_live_target_bounds(target, &mut position.x, low, high, CNAT_LEFT, CNAT_RIGHT);
    }

    let layer_vertical = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| live_layer_bounds(context, target, false))
    });
    if let Some((low, high)) = layer_vertical {
        apply_live_target_bounds(target, &mut position.y, low, high, CNAT_TOP, CNAT_BOTTOM);
    }

    let top = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let definition_id = context.object_effective_definition_id(target)?;
        let metadata = context.definition_metadata(&definition_id)?;
        if metadata.border_bound & C4D_BORDER_TOP == 0 {
            return None;
        }
        let shape_y = live_object_bounds_shape(context, target)
            .map(|shape| shape.y)
            .unwrap_or(0);
        Some((-shape_y, 1_000_000))
    });
    if let Some((low, high)) = top {
        apply_live_target_bounds(target, &mut position.y, low, high, CNAT_TOP, CNAT_BOTTOM);
    }

    let bottom = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let definition_id = context.object_effective_definition_id(target)?;
        let metadata = context.definition_metadata(&definition_id)?;
        if metadata.border_bound & C4D_BORDER_BOTTOM == 0 {
            return None;
        }
        let shape_y = live_object_bounds_shape(context, target)
            .map(|shape| shape.y)
            .unwrap_or(0);
        let height = context.landscape_ref()?.estimated_height();
        Some((-1_000_000, height.saturating_add(shape_y)))
    });
    if let Some((low, high)) = bottom {
        apply_live_target_bounds(target, &mut position.y, low, high, CNAT_TOP, CNAT_BOTTOM);
    }
}

pub(crate) fn exit_object_at_position_with_calls(
    target: ObjectId,
    position: Vector2,
    f_calls: bool,
) -> Result<bool, RuntimeError> {
    exit_object_at_position_with_motion_and_calls(
        target,
        position,
        FixedVec2::ZERO,
        C4Fixed::ZERO,
        f_calls,
    )
}

pub(crate) fn exit_object_at_position_with_motion_and_calls(
    target: ObjectId,
    position: Vector2,
    velocity: FixedVec2,
    rotation_velocity: C4Fixed,
    f_calls: bool,
) -> Result<bool, RuntimeError> {
    exit_object_at_position_with_full_motion_and_calls(
        target,
        position,
        0,
        velocity,
        rotation_velocity,
        f_calls,
    )
}

pub(crate) fn exit_object_at_position_with_full_motion_and_calls(
    target: ObjectId,
    mut position: Vector2,
    rotation: i32,
    velocity: FixedVec2,
    rotation_velocity: C4Fixed,
    f_calls: bool,
) -> Result<bool, RuntimeError> {
    let previous = with_host_context_mut(None, |context| {
        if !context.ensure_object_scope(target) {
            return None;
        }
        let previous = context.object_scope(target)?.container()?;
        context.track_contents_link_removal(previous, target);
        let scope = context.object_scope_mut(target)?;
        // Raw unlink first. Contained is null for BoundsCheck, but C++ does
        // not close the menu or refresh this object's OCF until afterward.
        scope.removed_contents_links.insert(previous);
        scope.current_container = None;
        scope.pending_update.container = Some(None);
        scope.reset_contained_compiler_cache();
        scope.pending_update.construction_preserves_fixed_position = false;
        scope.exit_bounds_in_progress = true;
        context.relink_content_after_exit(previous, target);
        refresh_container_collection_ocf(context, previous);
        Some(previous)
    });
    let Some(previous) = previous else {
        return Ok(false);
    };

    bounds_check_live_object(target, &mut position);

    with_host_context_mut((), |context| {
        let definition_metadata = context
            .object_effective_definition_id(target)
            .and_then(|id| context.definition_metadata(&id).cloned())
            .unwrap_or_default();
        let Some(scope) = context.object_scope_mut(target) else {
            return;
        };
        scope.exit_bounds_in_progress = false;
        // Exit assigns x/y even when unchanged and thereby snaps fix_x/y.
        scope.current_position = position;
        scope.current_fixed_position = FixedVec2::from_ints(position.x, position.y);
        scope.pending_update.position = Some(position);
        scope.pending_update.construction_preserves_fixed_position = false;
        scope.current_rotation = rotation;
        scope.current_fixed_rotation = itofix(rotation);
        scope.pending_update.rotation = Some(rotation);
        scope.set_fixed_velocity(velocity);
        scope.set_rotation_velocity(rotation_velocity);
        scope.set_mobile(true);
        scope.current_in_liquid = false;
        // Bounds callbacks may have opened a menu; Exit closes it afterward.
        scope.pending_update.menu = Some(None);
        // UpdateFace(true) rebuilds an ordinary C4Shape from Def after the
        // BoundsCheck callbacks consumed the old live SetShape rectangle.
        // Line shapes keep their independent geometry.
        if definition_metadata.line == 0 {
            scope.pending_update.shape_override = Some(None);
        }
        scope.refresh_shape_preview(&definition_metadata);
        // Pending native creations are already real objects to C++. Keep the
        // deferred SpawnConfig in the same post-Exit state so it can
        // materialize even when its likewise-pending container is removed
        // later in this call. The nested update still carries callback writes
        // that happen after this point.
        if let Some(spawn) = context
            .pending_spawns
            .iter_mut()
            .find(|spawn| spawn.id == Some(target))
        {
            spawn.container = None;
            spawn.position = position;
            spawn.fixed_position = None;
            spawn.rotation = rotation;
            spawn.fixed_rotation = None;
            spawn.velocity = Vector2::new(velocity.int_x(), velocity.int_y());
            spawn.fixed_velocity = Some(velocity);
            spawn.rotation_velocity = Some(rotation_velocity);
            spawn.mobile = Some(true);
            spawn.in_liquid = Some(false);
        }
    });
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            // Exit's UpdateFace(true) updates the shape/solid mask before
            // SetOCF derives the final outside-container flags.
            context.preview_live_object_sector(target);
            context.update_live_solid_mask(target, false);
            let _ = refresh_live_object_ocf(context, target);
        }
    });

    if f_calls && object_has_status(previous) {
        call_object_own_fail_safe(previous, "Ejection", &[object_reference_value(target)]);
    }
    if f_calls && object_has_status(target) {
        call_object_own_fail_safe(target, "Departure", &[object_reference_value(previous)]);
    }
    Ok(HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(target))
            .is_some_and(|object| object.container().is_none())
    }))
}

/// C4Object::ClearContentsAndContained. C4ObjectList::Remove advances every
/// registered iterator that points at the removed link, and the C++ for-loop
/// then advances it once more. Track link generations so callback-driven
/// re-entry cannot make a new link alias the removed iterator position.
fn clear_contents_and_contained_live(target: ObjectId, f_calls: bool) -> Result<(), RuntimeError> {
    let initial_links = with_host_context(Vec::new(), |context| {
        let Some(object) = context.get_world_object(target) else {
            return Vec::new();
        };
        object
            .contents()
            .iter()
            .copied()
            .map(|child| (child, context.contents_link_generation(child)))
            .collect::<Vec<_>>()
    });
    let mut iterator = crate::direct_com::RemovalSafeContentsIterator::new(target, &initial_links);
    loop {
        let links_and_position = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let object = context.get_world_object(target)?;
            let links = object
                .contents()
                .iter()
                .copied()
                .map(|child| (child, context.contents_link_generation(child)))
                .collect::<Vec<_>>();
            Some((links, object.position))
        });
        let Some((links, position)) = links_and_position else {
            break;
        };
        let Some(child) = iterator.next(&links) else {
            break;
        };
        let _ = exit_object_at_position_with_calls(child, position, f_calls)?;
    }

    // C++ re-reads x/y after all content callbacks, so an Ejection or
    // Departure callback that moves the object affects its own later Exit.
    let contained_position = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let object = context.get_world_object(target)?;
        object.container().map(|_| object.position)
    });
    if let Some(position) = contained_position {
        let _ = exit_object_at_position_with_calls(target, position, f_calls)?;
    }
    Ok(())
}

/// Engine-internal exits normally retain the object's current coordinates.
pub(crate) fn exit_object_at_current_position(target: ObjectId) -> Result<bool, RuntimeError> {
    let position = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(target))
            .map(|object| object.position)
    });
    let Some(position) = position else {
        return Ok(false);
    };
    exit_object_at_position(target, position)
}

/// Live `C4Object::Enter(target)` without Collect's RejectCollect,
/// Collection and Hit tail. This is the path used by CreateContents and
/// Split2Components and must finish its callbacks before their next step.
pub(crate) fn enter_object_live(
    target: ObjectId,
    container: ObjectId,
) -> Result<bool, RuntimeError> {
    enter_object_live_with_calls(target, container, true)
}

/// C4Object::Enter's `fCalls` controls only Collection2/Entrance. The
/// RejectEntrance query, a transfer's ordinary callback-enabled Exit,
/// CopyMotion and base auto-sale all run in both modes.
fn enter_object_live_with_calls(
    target: ObjectId,
    container: ObjectId,
    f_calls: bool,
) -> Result<bool, RuntimeError> {
    enter_object_live_internal(target, container, f_calls, false, true)
}

/// ObjectComPut's Enter form supplies a non-null RejectCollect result
/// pointer. It shares the ordinary callback/copy-motion tail but inserts the
/// target query after cycle detection and before Exit (C4Object.cpp:1566-1636).
pub(crate) fn enter_object_live_with_reject_collect(
    target: ObjectId,
    container: ObjectId,
) -> Result<bool, RuntimeError> {
    enter_object_live_internal(target, container, true, true, true)
}

/// C4Object::Collect supplies RejectCollect but defers CopyMotion until after
/// Collection and Hit. All other C4Object::Enter state and callbacks are the
/// ordinary live path.
fn enter_object_live_for_collect(
    target: ObjectId,
    container: ObjectId,
) -> Result<bool, RuntimeError> {
    enter_object_live_internal(target, container, true, true, false)
}

fn enter_object_live_internal(
    target: ObjectId,
    container: ObjectId,
    f_calls: bool,
    reject_collect: bool,
    f_copy_motion: bool,
) -> Result<bool, RuntimeError> {
    // C4Object::Enter rejects null/self up front, but delays raw Status gates
    // until after RejectEntrance/RejectCollect and any old-container Exit.
    if target == container {
        return Ok(false);
    }
    if call_object_own_fail_safe(
        target,
        "RejectEntrance",
        &[object_reference_value(container)],
    )
    .as_bool()
    {
        return Ok(false);
    }

    let would_cycle = with_host_context(true, |context| {
        let mut cursor = Some(container);
        let mut seen = HashSet::new();
        while let Some(current) = cursor {
            if current == target || !seen.insert(current) {
                return true;
            }
            cursor = context
                .get_world_object(current)
                .and_then(|object| object.container());
        }
        false
    });
    if would_cycle {
        return Ok(false);
    }

    if reject_collect {
        let definition_id = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_effective_definition_id(target))
        });
        let Some(definition_id) = definition_id else {
            return Ok(false);
        };
        if call_object_own_fail_safe(
            container,
            "RejectCollect",
            &[
                Value::C4Id(definition_id.to_string()),
                object_reference_value(target),
            ],
        )
        .as_bool()
        {
            return Ok(false);
        }
    }

    let contained = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(target))
            .and_then(|object| object.container())
            .is_some()
    });
    if contained && !exit_object_at_current_position(target)? {
        return Ok(false);
    }

    let entered = with_host_context_mut(false, |context| {
        let definition_metadata = context
            .object_effective_definition_id(target)
            .and_then(|definition_id| context.definition_metadata(&definition_id).cloned())
            .unwrap_or_default();
        let target_ready = context.object_scope(target).is_some_and(|scope| {
            !scope.destroy && scope.status() != ObjectStatus::Deleted && scope.container().is_none()
        }) || context
            .get_world_object(target)
            .is_some_and(|object| object.is_present() && object.container().is_none());
        let container_motion = context
            .object_scope(container)
            .filter(|scope| !scope.destroy && scope.status() != ObjectStatus::Deleted)
            .map(|scope| {
                (
                    scope.effective_position(),
                    scope.fixed_velocity(),
                    scope.controller(),
                )
            })
            .or_else(|| {
                context
                    .get_world_object(container)
                    .filter(|object| object.is_present())
                    .map(|object| (object.position, object.fixed_velocity, object.controller()))
            });
        let Some((position, velocity, controller)) = container_motion else {
            return false;
        };
        if !target_ready || !context.ensure_object_scope(target) {
            return false;
        }
        let Some(scope) = context.object_scope_mut(target) else {
            return false;
        };
        // Enter closes an uncontained object's menu before linking it. A
        // transfer's Exit already did so, but the repeated close is harmless.
        scope.pending_update.menu = Some(None);
        scope.set_container(Some(container));
        if !(scope.alive() && scope.category() & crate::CATEGORY_LIVING != 0) {
            scope.set_controller(controller);
        }
        if f_copy_motion {
            let was_mobile = scope.mobile();
            // CopyMotion writes integer position/fix position and x/y dirs;
            // it intentionally leaves r/rdir untouched.
            scope.current_position = position;
            scope.current_fixed_position = FixedVec2::from_ints(position.x, position.y);
            scope.pending_update.position = Some(position);
            scope.pending_update.construction_preserves_fixed_position = false;
            scope.set_fixed_velocity(velocity);
            // CopyMotion does not mobilize; the generic fixed-dir update does,
            // so carry the pre-Enter native flag explicitly over that fold.
            scope.set_mobile(was_mobile);
        }
        let nonliving = !(scope.alive() && scope.category() & crate::CATEGORY_LIVING != 0);
        if let Some(spawn) = context
            .pending_spawns
            .iter_mut()
            .find(|spawn| spawn.id == Some(target))
        {
            spawn.container = Some(container);
            if f_copy_motion {
                spawn.position = position;
                spawn.fixed_position = None;
                spawn.fixed_velocity = Some(velocity);
            }
            if nonliving {
                spawn.controller = Some(controller);
            }
        }
        context.link_content_after_enter(container, target);
        if f_copy_motion {
            // Native Enter removes the outside solid mask before CopyMotion,
            // so SetOCF at the destination cannot sample the object's own
            // old bake. No callback occurs during CopyMotion; removing here,
            // immediately before SetOCF, has the same observable ordering.
            context.update_live_solid_mask(target, false);
        }
        let _ = refresh_live_object_ocf(context, target);
        if let Some(scope) = context.object_scope_mut(target) {
            if definition_metadata.line == 0 {
                scope.pending_update.shape_override = Some(None);
            }
            scope.refresh_shape_preview(&definition_metadata);
        }
        context.preview_live_object_sector(target);
        context.update_live_solid_mask(target, false);
        refresh_container_collection_ocf(context, container);
        true
    });
    if !entered {
        return Ok(false);
    }

    if f_calls {
        call_object_own_fail_safe(container, "Collection2", &[object_reference_value(target)]);
        let entrance_container = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let current = context.get_world_object(target)?.container()?;
            let current_live = context
                .get_world_object(current)
                .is_some_and(|object| object.is_present());
            let original_live = context
                .get_world_object(container)
                .is_some_and(|object| object.is_present());
            (current_live && original_live).then_some(current)
        });
        if let Some(current) = entrance_container {
            call_object_own_fail_safe(target, "Entrance", &[object_reference_value(current)]);
        }
    }
    auto_sell_after_enter(target, container)?;
    Ok(true)
}

/// `C4Object::AssignRemoval` sets Status=0 before killing contents, but it
/// deliberately keeps Info, object references and the solid mask alive until
/// after contents and containment have been cleaned up (C4Object.cpp:276-313).
fn mark_object_status_deleted(context: &mut EffectHostContext, target: ObjectId) -> bool {
    if !context.ensure_object_scope(target) {
        return false;
    }
    if let Some(scope) = context.object_scope_mut(target) {
        scope.mark_destroy_status();
        true
    } else {
        false
    }
}

fn retire_object_info_and_clear_references(
    context: &mut EffectHostContext,
    target: ObjectId,
    last_position: Option<Vector2>,
) {
    let link = context
        .object_scope(target)
        .and_then(ObjectScopeContext::info_link);
    if let Some(link) = link {
        if retire_host_crew_info(context, link) {
            context.record_player_command(PlayerCommand::RetireCrewInfo {
                object_id: target,
                link,
            });
        }
    }
    if let Some(scope) = context.object_scope_mut(target) {
        scope.clear_info_for_removal();
    }
    context.clear_non_player_script_object_references(target, last_position);
}

pub(crate) fn assign_removal_live(
    target: ObjectId,
    exit_contents: bool,
) -> Result<bool, RuntimeError> {
    // C4Object::AssignRemoval gates on raw Status truthiness. Inactive is a
    // live nonzero status and is activated internally before deletion.
    if !object_has_status(target) {
        return Ok(false);
    }
    let container = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(target))
            .and_then(|object| object.container())
    });
    if let Some(container) = container.filter(|container| object_has_status(*container)) {
        call_object_own_fail_safe(
            container,
            "ContentsDestruction",
            &[object_reference_value(target)],
        );
        if !object_has_status(target) {
            return Ok(true);
        }
    }
    call_object_own_fail_safe(target, "Destruction", &[]);
    if !object_has_status(target) {
        return Ok(true);
    }
    if !clear_effects_for_assign_removal(target)? {
        return Ok(true);
    }

    // Particle lists are cleared before SetAction(ActIdle), while the object
    // is still live (C4Object.cpp:268-274).
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if let Some(context) = borrow.as_mut() {
            context.register_particle(ParticleCommand::Clear {
                definition_id: None,
                scope: ParticleScope::Object(target),
            });
        }
    });
    let _ = native_set_action_by_name(target, "Idle")?;
    if !object_has_status(target) {
        return Ok(true);
    }

    // AssignRemoval(true) passes the removed container's x/y to every
    // direct child's Exit (C4Object.cpp:285-288).
    let exit_position = HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().and_then(|context| {
            context
                .object_scope(target)
                .map(|scope| scope.current_position)
                .or_else(|| {
                    context
                        .get_world_object(target)
                        .map(|object| object.position)
                })
        })
    });

    with_host_context_mut((), |context| {
        mark_object_status_deleted(context, target);
    });

    loop {
        let child = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(target))
                .and_then(|object| object.contents().first().copied())
        });
        let Some(child) = child else { break };
        if exit_contents {
            let Some(position) = exit_position else { break };
            if !exit_object_at_position(child, position)? {
                break;
            }
            continue;
        }
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.unlink_content_for_removal(target, child);
            }
        });
        // AssignRemoval's default `fExitContents=false` unlinks and
        // recursively removes contents without Ejection/Departure calls.
        // The child's Contained pointer remains observable during its own
        // Destruction callback even though the parent is already dead.
        let _ = assign_removal_live(child, false)?;
    }

    with_host_context_mut((), |context| {
        let removed_from = context
            .object_scope(target)
            .and_then(ObjectScopeContext::container);
        context.set_object_container_tracked(target, None);
        if let Some(container) = removed_from {
            // AssignRemoval removes the child's link, then UpdateMass and
            // SetOCF on the surviving parent (C4Object.cpp:297-305).
            refresh_container_collection_ocf(context, container);
        }
        retire_object_info_and_clear_references(context, target, exit_position);
    });
    clear_player_object_pointers_host(target);
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if let Some(context) = borrow.as_mut() {
            // pSolidMaskData->Remove is the last modeled AssignRemoval side
            // arm, after Game.ClearPointers (C4Object.cpp:309-313).
            context.update_live_solid_mask(target, false);
        }
    });
    Ok(true)
}

fn can_sell_object_live(target: ObjectId, player: i32) -> bool {
    with_host_context(false, |context| {
        let player_active = context.player_state(player).is_some_and(|state| {
            !matches!(
                state.status,
                crate::PlayerStatus::Eliminated | crate::PlayerStatus::Surrendered
            ) && !state.surrendered
        });
        player_active
            && context
                .get_world_object(target)
                .is_some_and(|object| object.is_present() && object.ocf() & ocf::CREW_MEMBER == 0)
    })
}

pub(crate) fn sell_object_to_home_live(
    target: ObjectId,
    player: i32,
) -> Result<bool, RuntimeError> {
    if !can_sell_object_live(target, player) {
        return Ok(false);
    }

    loop {
        let child = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| first_retained_content(context, target))
        });
        let Some(child) = child else { break };
        let container = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(target))
                .and_then(|object| object.container())
        });
        let _ = match container {
            Some(container) => enter_object_live(child, container)?,
            None => exit_object_at_current_position(child)?,
        };
        let _ = sell_object_to_home_live(child, player)?;
    }

    // Sell2Home prices the object against its live containing base. Direct
    // Sell calls reach this path before Exit, unlike the base-auto-sell path.
    let base = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(target))
            .and_then(|object| object.container())
    });
    let value = match get_value(&[
        object_reference_value(target),
        Value::Nil,
        base.map(object_reference_value).unwrap_or(Value::Nil),
        Value::Int(player),
    ]) {
        Ok(Value::Int(value)) => value,
        Ok(_) => 0,
        Err(error) => {
            tracing::warn!(%error, "CalcValue failed during home-base sale; using zero");
            0
        }
    };
    // C4Player::DoWealth clamps adjustments to 0..=10000; FnSetWealth's
    // wider 100000 ceiling does not apply to a home-base sale.
    with_host_context_mut((), |context| {
        let updated = {
            let Some(state) = context.player_state_mut(player) else {
                return;
            };
            let updated = (i64::from(state.wealth) + i64::from(value)).clamp(0, 10_000) as i32;
            state.wealth = updated;
            state.view_wealth = 100;
            updated
        };
        context.record_player_command(PlayerCommand::SetWealth {
            player_id: player,
            value: updated,
            show_change: true,
        });
    });

    let original_definition = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_effective_definition_id(target))
    });
    let stock_definition =
        match call_world_object_own_function(target, "SellTo", &[Value::Int(player)]) {
            Some(Ok(Value::C4Id(id))) => {
                definition_id_for_c4id(&id).map(|id| DefinitionId::from(id.as_str()))
            }
            Some(Ok(Value::Int(raw @ 1..=9999))) => {
                Some(DefinitionId::from(format!("{raw:04}").as_str()))
            }
            Some(Ok(_)) => None,
            None => original_definition,
            Some(Err(error)) => {
                tracing::warn!(%error, "SellTo failed during home-base sale; omitting stock");
                None
            }
        };
    if let Some(definition) = stock_definition {
        let (valid_definition, should_stock) = with_host_context((false, false), |context| {
            let valid_definition =
                context.world.definition_known(definition.as_str()) != Some(false);
            let should_stock = valid_definition
                && (context.world.definition_rebuyable(definition.as_str())
                    || context
                        .player_state(player)
                        .is_some_and(|state| state.home_base_material.contains_key(&definition)));
            (valid_definition, should_stock)
        });
        if should_stock {
            let _ = do_homebase_material(&[
                Value::Int(player),
                Value::C4Id(definition.as_str().to_owned()),
                Value::Int(1),
            ])?;
        } else if valid_definition {
            sync_homebase_material_to_team_live(player);
        }
    }

    let contained = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(target))
            .is_some_and(|object| object.container().is_some())
    });
    if contained && object_is_present(target) {
        let _ = exit_object_at_position(target, Vector2::ZERO)?;
    }
    if object_is_present(target) {
        call_object_own_fail_safe(target, "Sale", &[Value::Int(player)]);
    }
    if object_is_present(target) {
        let _ = assign_removal_live(target, true)?;
    }
    Ok(true)
}

fn report_buy_error(
    player: i32,
    message: String,
    target: Option<ObjectId>,
) -> Result<(), RuntimeError> {
    let _ = player_message(&[Value::Int(player), Value::String(message.into())])?;
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            let _ = context.play_sound("Error", target, 100, false, false, None);
        }
    });
    Ok(())
}

/// FnBuy (C4Script.cpp:3732-3751) plus C4Player::Buy
/// (C4Player.cpp:826-864): synchronously consume the paying player's base
/// stock and wealth, create the item for another player, call Purchase, then
/// enter an explicit target or force-position at the calling object.
pub(crate) fn buy(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 5 {
        return Err(RuntimeError::new(
            "Buy expects at most 5 arguments: definition, for_player, pay_player, target, show_errors",
        ));
    }
    let Some(definition) = parse_native_c4id_argument(args.first(), "Buy")? else {
        return Ok(Value::Nil);
    };
    let for_player = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Buy", "for_player")?;
    let pay_player = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "Buy", "pay_player")?;
    let to_base =
        parse_object_reference_argument(args.get(3).unwrap_or(&Value::Nil), "Buy", "target")?;
    let show_errors = value_to_bool(args.get(4).unwrap_or(&Value::Nil), "Buy", "show_errors")?;
    let caller = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.script_object_context)
    });
    let creator = to_base.or(caller);
    let definition_id = DefinitionId::from(definition.as_str());

    let initial = with_host_context(None, |context| {
        // ValidPlr accepts eliminated players; C4Player::Buy reports that
        // state separately below.
        context.player_state(for_player)?;
        let payer = context.player_state(pay_player)?;
        let eliminated = matches!(
            payer.status,
            crate::PlayerStatus::Eliminated | crate::PlayerStatus::Surrendered
        ) || payer.surrendered;
        let available = payer
            .exact_home_base_material_entries()
            .into_iter()
            .find_map(|(id, count)| (id == definition_id).then_some(count))
            .unwrap_or(0);
        let crew_member = context
            .definition_metadata(&definition)
            .map(|metadata| metadata.crew_member);
        Some((eliminated, payer.name.clone(), available, crew_member))
    });
    let Some((eliminated, payer_name, available, crew_member)) = initial else {
        return Ok(Value::Nil);
    };
    if eliminated {
        if show_errors {
            report_buy_error(
                pay_player,
                format!("Player {payer_name}|eliminated."),
                creator,
            )?;
        }
        return Ok(Value::Nil);
    }
    // C4Player::Buy checks stock before resolving the definition. Neither an
    // unavailable item nor an unknown definition produces an error message.
    if available <= 0 {
        return Ok(Value::Nil);
    }
    let Some(crew_member) = crew_member else {
        return Ok(Value::Nil);
    };

    let Some(price) = calculated_definition_value(&definition, creator, pay_player)? else {
        return Ok(Value::Nil);
    };

    // CalcDefValue/CalcBuyValue callbacks above can mutate both wealth and
    // stock. C++ compares/decrements the live values after those callbacks,
    // while retaining only the initial availability decision.
    let charged = with_host_context_mut(false, |context| {
        let Some(current_wealth) = context.player_state(pay_player).map(|payer| payer.wealth)
        else {
            return false;
        };
        if price > current_wealth {
            return false;
        }

        let (team, updated_material, updated_wealth) = {
            let Some(payer) = context.player_state_mut(pay_player) else {
                return false;
            };
            payer.adjust_home_base_material_entry(definition_id.clone(), -1);
            let updated_wealth =
                (i64::from(payer.wealth) - i64::from(price)).clamp(0, 10_000) as i32;
            payer.wealth = updated_wealth;
            payer.view_wealth = 100;
            (
                payer.team,
                payer.exact_home_base_material_entries(),
                updated_wealth,
            )
        };

        if context.team_home_base_rule() {
            if let Some(team) = team {
                let teammates: Vec<i32> = context
                    .player_ids()
                    .iter()
                    .copied()
                    .filter(|other| {
                        *other != pay_player
                            && context.player_state(*other).and_then(|player| player.team)
                                == Some(team)
                    })
                    .collect();
                for teammate in teammates {
                    if let Some(player) = context.player_state_mut(teammate) {
                        player.set_home_base_material_entries(updated_material.clone());
                    }
                }
            }
        }
        context.record_player_command(PlayerCommand::AdjustHomeBaseMaterial {
            player_id: pay_player,
            definition_id: definition_id.clone(),
            delta: -1,
        });
        context.record_player_command(PlayerCommand::SetWealth {
            player_id: pay_player,
            value: updated_wealth,
            show_change: true,
        });
        true
    });
    if !charged {
        if show_errors {
            report_buy_error(pay_player, "Not enough money!".to_string(), creator)?;
        }
        return Ok(Value::Nil);
    }

    let Some(created) = create_native_object(NativeObjectCreation {
        definition: definition.clone(),
        creator,
        owner: for_player,
        controller: for_player,
        construction: FULL_CON,
        position: Vector2::new(50, 50),
        rotation: 0,
        velocity: FixedVec2::ZERO,
        rotation_velocity: C4Fixed::ZERO,
    })?
    else {
        return Ok(Value::Nil);
    };

    if crew_member {
        let _ = make_crew_member_live(created, for_player)?;
    }
    if !object_is_present(created) {
        return Ok(Value::Nil);
    }
    call_object_own_fail_safe(
        created,
        "Purchase",
        &[
            Value::Int(pay_player),
            creator.map(object_reference_value).unwrap_or(Value::Nil),
        ],
    );
    if !object_is_present(created) {
        return Ok(Value::Nil);
    }

    if let Some(to_base) = to_base {
        let _ = enter_object_live(created, to_base)?;
    } else if let Some(caller) = caller {
        let caller_position = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(caller))
                .map(|object| object.position())
        });
        if let Some(position) = caller_position {
            let _ = set_position(&[
                Value::Int(position.x),
                Value::Int(position.y),
                object_reference_value(created),
            ])?;
        }
    }
    Ok(object_reference_value(created))
}

/// FnSell (C4Script.cpp:3753-3760): a nil object means the executing
/// script object; a valid player then runs the complete Sell2Home path.
pub(crate) fn sell(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "Sell expects at most 2 arguments: player, object",
        ));
    }
    let player = value_to_i32(args.first().unwrap_or(&Value::Nil), "Sell", "player")?;
    let explicit =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "Sell", "object")?;
    let target = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| explicit.or(context.script_object_context))
    });
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(sell_object_to_home_live(target, player)?))
}

fn auto_sell_after_enter(
    entering: ObjectId,
    original_target: ObjectId,
) -> Result<(), RuntimeError> {
    let sale = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        if !context.world.base_auto_sell_enabled
            || !context
                .get_world_object(original_target)
                .is_some_and(|object| object.is_present())
        {
            return None;
        }
        let container = context.get_world_object(entering)?.container()?;
        let container_state = context.get_world_object(container)?;
        if !container_state.is_present() {
            return None;
        }
        let base = container_state.full_state()?.base;
        context.player_state(base).map(|_| (container, base))
    });
    let Some((container, player)) = sale else {
        return Ok(());
    };
    let outer = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(container))
            .map(|object| object.contents().to_vec())
            .unwrap_or_default()
    });
    for object in outer {
        let nested = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(object))
                .map(|object| object.contents().to_vec())
                .unwrap_or_default()
        });
        for child in nested {
            let auto_sell = with_host_context(false, |context| {
                context
                    .object_effective_definition_id(child)
                    .is_some_and(|id| context.world.definition_base_auto_sell(id.as_str()))
            });
            if auto_sell && can_sell_object_live(child, player) {
                let _ = exit_object_at_current_position(child)?;
                let _ = sell_object_to_home_live(child, player)?;
            }
        }
        let auto_sell = with_host_context(false, |context| {
            context
                .object_effective_definition_id(object)
                .is_some_and(|id| context.world.definition_base_auto_sell(id.as_str()))
        });
        if auto_sell && can_sell_object_live(object, player) {
            let _ = exit_object_at_current_position(object)?;
            let _ = sell_object_to_home_live(object, player)?;
        }
    }
    Ok(())
}

/// FnEnter (C4Script.cpp:365-370): pObj (or the scope object) enters the
/// container pTarget through the synchronous C4Object::Enter pipeline.
pub(crate) fn enter(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(target) =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Enter", "target")?
    else {
        return Ok(Value::Bool(false)); // C4Object::Enter(nullptr)
    };
    let subject =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "Enter", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    let Some(subject) = subject.or(active) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(enter_object_live(subject, target)?))
}

/// FnCollect (C4Script.cpp:391-415) routes through C4Object::Collect:
/// validate the collector's cached OCF, run Enter's two vetoes and callback
/// tail synchronously, then Collection/Hit before the final CopyMotion
/// (C4Object.cpp:1566-1636,5693-5714). This must stay a live host operation,
/// rather than a deferred container assignment: MFBL collects a same-call
/// freshly-created FRBL and branches on the boolean result.
pub(crate) fn flag_collection_blocked(
    definition_id: &str,
    action_name: &str,
    flag_removeable: bool,
) -> bool {
    !flag_removeable && definition_id == "FLAG" && action_name == "FlyBase"
}

struct CollectDelayRestore {
    collector: ObjectId,
    old_delay: i32,
}

impl Drop for CollectDelayRestore {
    fn drop(&mut self) {
        if self.old_delay == 0 {
            return;
        }
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(scope) = borrow
                .as_mut()
                .and_then(|context| context.object_scope_mut(self.collector))
            else {
                return;
            };
            scope.restore_no_collect_delay(self.old_delay);
        });
    }
}

pub(crate) fn collect(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(item) =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Collect", "item")?
    else {
        return Ok(Value::Bool(false));
    };
    let explicit_collector = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "Collect",
        "collector",
    )?;
    let active = active_object_id();
    let Some(collector) = explicit_collector.or(active) else {
        return Ok(Value::Bool(false));
    };
    if item == collector {
        return Ok(Value::Bool(false));
    }

    let (ready, old_no_collect_delay) = with_host_context_mut((false, 0), |context| {
        let item_ready = context
            .get_world_object(item)
            .is_some_and(|object| object.is_present());
        let collector_state = context.get_world_object(collector);
        let collector_present = collector_state
            .as_ref()
            .is_some_and(HostWorldObject::is_present);
        if !collector_present || !context.ensure_object_scope(collector) {
            return (false, 0);
        }
        let old_no_collect_delay = context
            .object_scope(collector)
            .map(ObjectScopeContext::no_collect_delay)
            .unwrap_or(0);
        // FnCollect trusts the existing cached OCF when the delay is already
        // zero. Only a nonzero delay is cleared and followed by UpdateOCF.
        if old_no_collect_delay != 0 {
            if let Some(scope) = context.object_scope_mut(collector) {
                scope.set_no_collect_delay(0);
            }
            let _ = refresh_live_object_ocf(context, collector);
        }
        let collector_ready = collector_present
            && context
                .object_scope(collector)
                .is_some_and(|scope| scope.ocf() & ocf::COLLECTION != 0);
        (item_ready && collector_ready, old_no_collect_delay)
    });
    // Restore on every success, veto, error and early return after the
    // temporary clear. C++ performs this after Collect returns, even when it
    // failed (C4Script.cpp:410-413).
    let _delay_restore = CollectDelayRestore {
        collector,
        old_delay: old_no_collect_delay,
    };
    if !ready {
        return Ok(Value::Bool(false));
    }

    // C4Object::Collect's first operation: an attached base flag is not
    // collectable without cached C4RULE_FlagRemoveable. FnCollect's
    // temporary NoCollectDelay/OCF work above deliberately precedes this
    // native gate (C4Script.cpp:391-413; C4Object.cpp:5693-5700).
    let blocked_flag = with_host_context(false, |context| {
        context.get_world_object(item).is_some_and(|item| {
            flag_collection_blocked(
                item.definition_id(),
                item.action_name(),
                context.world.flag_removeable(),
            )
        })
    });
    if blocked_flag {
        return Ok(Value::Bool(false));
    }

    let call_fail_safe = |target, function: &str, pars: &[Value]| -> Value {
        match call_world_object_own_function(target, function, pars) {
            Some(Ok(value)) => value,
            Some(Err(error)) => {
                tracing::error!(
                    %error,
                    object = target.as_u64(),
                    callback = function,
                    "script error in Collect callback; continuing like C++ fail-safe Call"
                );
                log_runtime_call_frames("", error.call_frames());
                Value::Nil
            }
            None => Value::Nil,
        }
    };

    // Collect uses the ordinary Enter lifecycle and RejectCollect query, but
    // passes fCopyMotion=false so Collection2/Entrance/Collection/Hit observe
    // the item's post-Exit motion. Its own tail copies collector motion later.
    if !enter_object_live_for_collect(item, collector)? {
        return Ok(Value::Bool(false));
    }

    // C4Object::Collect cancels an ATTACH procedure before Collection. Use
    // ObjectSetAction so Start/Abort calls remain synchronous like C++
    // ObjectComCancelAttach -> SetAction(ActIdle) (C4ObjectCom.cpp:769-774).
    let attached = with_host_context(false, |context| {
        context
            .object_scope(item)
            .map(ObjectScopeContext::effective_action_procedure)
            .or_else(|| {
                context
                    .get_world_object(item)
                    .and_then(|object| object.procedure_name().map(ActionProcedure::from_name))
            })
            == Some(ActionProcedure::Attach)
    });
    if attached {
        let _ = object_set_action(&[object_reference_value(item), Value::String("Idle".into())])?;
    }

    call_fail_safe(collector, "Collection", &[object_reference_value(item)]);
    for (flag, function) in [
        (ocf::HIT_SPEED1, "Hit"),
        (ocf::HIT_SPEED2, "Hit2"),
        (ocf::HIT_SPEED3, "Hit3"),
    ] {
        // Native rereads both Status and OCF before every callback; an
        // earlier Hit may delete the item or synchronously change later
        // hit-speed flags.
        let should_call = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(item))
                .is_some_and(|object| object.is_present() && object.ocf() & flag != 0)
        });
        if should_call {
            call_fail_safe(item, function, &[]);
        }
    }

    // Hit observes the pre-copy item motion. Only afterwards, and only if
    // callbacks left it in this collector, CopyMotion snaps position and
    // fixed dirs to the collector (C4Object.cpp:5711-5713;
    // C4Movement.cpp:518-529).
    with_host_context_mut((), |context| {
        let still_collected = context
            .get_world_object(item)
            .is_some_and(|object| object.is_present() && object.container() == Some(collector));
        if !still_collected {
            return;
        }
        let Some(collector_state) = context.get_world_object(collector) else {
            return;
        };
        let position = context
            .object_scope(collector)
            .map(ObjectScopeContext::effective_position)
            .unwrap_or(collector_state.position);
        let velocity = context
            .object_scope(collector)
            .map(ObjectScopeContext::fixed_velocity)
            .unwrap_or(collector_state.fixed_velocity);
        let moved = {
            let Some(scope) = context.object_scope_mut(item) else {
                return;
            };
            let moved = scope.effective_position() != position;
            let was_mobile = scope.mobile();
            scope.set_position(position);
            scope.set_fixed_velocity(velocity);
            // Native CopyMotion writes x/y and xdir/ydir without changing Mobile.
            scope.set_mobile(was_mobile);
            moved
        };
        if moved {
            context.preview_live_object_sector(item);
        }
    });

    // FnCollect restores the old positive NoCollectDelay but deliberately
    // does not call UpdateOCF again (:412-413). Preserve the cache left by
    // the temporary recompute/Enter callbacks after the deferred outcome
    // fold performs its ordinary container refresh.
    if old_no_collect_delay != 0 {
        with_host_context_mut((), |context| {
            if context.ensure_object_scope(collector) {
                if let Some(scope) = context.object_scope_mut(collector) {
                    let ocf = scope.ocf();
                    scope.pending_update.ocf_override = Some(ocf);
                }
            }
        });
    }

    Ok(Value::Bool(true))
}

/// FnGrabContents (C4Script.cpp:320-327) and C4Object::GrabContents
/// (C4Object.cpp:6162-6171): pTo defaults to the calling object, the source
/// list is copied before any moves, and every still-live entry attempts a
/// regular Enter(pTo). Individual failures do not change the true return.
pub(crate) fn grab_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let from = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GrabContents",
        "from",
    )?;
    let Some(from) = from else {
        return Ok(Value::Bool(false));
    };
    let explicit_to =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "GrabContents", "to")?;
    let active = active_object_id();
    let Some(to) = explicit_to.or(active) else {
        return Ok(Value::Bool(false));
    };
    if to == from {
        return Ok(Value::Bool(false));
    }

    // An explicit foreign destination becomes the call context just like
    // FnGrabContents' pTo default would have been that object. This also
    // lets object-arrow calls operate on freshly created pending objects.
    if Some(to) != active {
        return match call_world_object_function(to, "GrabContents", &[object_reference_value(from)])
        {
            Some(result) => result,
            None => Ok(Value::Bool(false)),
        };
    }

    let contents = with_host_context(None, |context| {
        // A C4Value object reference can only supply live engine objects.
        context.get_world_object(to)?;
        context
            .get_world_object(from)
            .map(|source| source.contents().to_vec())
    });
    let Some(contents) = contents else {
        return Ok(Value::Bool(false));
    };

    // C4Object::GrabContents snapshots the list, then invokes the ordinary
    // C4Object::Enter path for every still-live link. Reuse the live Enter
    // seam so transfers receive their complete Exit/CopyMotion/OCF/face and
    // callback lifecycle instead of maintaining another partial duplicate.
    for child in contents {
        // GrabContents' copied-list loop owns this raw Status check. Enter
        // itself deliberately performs its status gates only after callbacks
        // and an old-container Exit.
        if object_has_status(child) {
            let _ = enter_object_live(child, to)?;
        }
    }

    Ok(Value::Bool(true))
}

/// FnExit (C4Script.cpp:372-388): pObj (or the scope object) leaves its
/// container via C4Object::Exit. The optional tx/ty are CALLER-relative
/// (`tx += cthr->Obj->x`, :377-381), tr == -1 draws Random(360) (:382),
/// and Exit writes position/rotation/dirs unconditionally — bare Exit()
/// re-places the object at the caller's position with r = 0 and zeroed
/// dirs (C4Object.cpp:1549-1553), the y target offset by the SUBJECT's
/// Shape.y (:385) and rdir scaled `itofix(trdir) / 10` (:388).
/// ObjectComCancelAttach changes an ATTACH action to Idle (including its
/// AbortCall) before Exit checks containment. Exit unlinks, runs BoundsCheck,
/// installs final state, dispatches Ejection then Departure synchronously and
/// returns the live post-callback `!Contained` state (C4Object.cpp:1532-1563).
pub(crate) fn exit_container(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 7 {
        return Err(RuntimeError::new(
            "Exit expects at most 7 arguments: obj, x, y, r, xdir, ydir, rdir",
        ));
    }
    let subject =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "Exit", "obj")?;
    let tx = parse_optional_i32(args.get(1), "Exit", "x")?.unwrap_or(0);
    let ty = parse_optional_i32(args.get(2), "Exit", "y")?.unwrap_or(0);
    let tr = parse_optional_i32(args.get(3), "Exit", "r")?.unwrap_or(0);
    let txdir = parse_optional_i32(args.get(4), "Exit", "xdir")?.unwrap_or(0);
    let tydir = parse_optional_i32(args.get(5), "Exit", "ydir")?.unwrap_or(0);
    let trdir = parse_optional_i32(args.get(6), "Exit", "rdir")?.unwrap_or(0);
    let prepared = with_host_context_mut(Ok(None), |context| {
        let active = context.object_context().map(|object| object.id());
        let Some(target) = subject.or(active) else {
            return Ok(None); // no pObj and no scope object
        };
        // Caller-relative offset: the CALLING object, also for foreign
        // subjects (C4Script.cpp:377-381).
        let (mut abs_x, mut abs_y) = (tx, ty);
        if let Some(caller) = context.object_context() {
            let position = caller.effective_position();
            abs_x = abs_x.saturating_add(position.x);
            abs_y = abs_y.saturating_add(position.y);
        }
        // The Random(360) draw happens before the contained check — it
        // runs even when Exit then fails (C4Script.cpp:382).
        let rotation = if tr == -1 {
            draw_context_random(360)?
        } else {
            tr
        };
        Ok(Some((target, abs_x, abs_y, rotation)))
    })?;
    let Some((target, abs_x, abs_y, rotation)) = prepared else {
        return Ok(Value::Bool(false));
    };

    // ObjectComCancelAttach runs after the optional rotation draw but before
    // C4Object::Exit checks Contained. SetAction(ActIdle) is a native call:
    // a script function named SetAction may not intercept it, and the old
    // ATTACH action's AbortCall observes the pre-Exit position/container.
    let attached = with_host_context(false, |context| {
        context
            .object_scope(target)
            .map(ObjectScopeContext::effective_action_procedure)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .and_then(|object| object.procedure_name().map(ActionProcedure::from_name))
            })
            == Some(ActionProcedure::Attach)
    });
    if attached {
        let _ = native_set_action_by_name(target, "Idle")?;
    }

    // The SUBJECT's live Shape.y (C4Script.cpp:385): a same-call SetShape
    // override wins over the def shape. Read it only after the attach
    // AbortCall, which may change the shape or definition.
    let shape_y = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        Some(
            live_object_shape(context, target)
                .map(|shape| shape.y)
                .unwrap_or(0),
        )
    });
    let Some(shape_y) = shape_y else {
        return Ok(Value::Bool(false));
    };
    let exited = exit_object_at_position_with_full_motion_and_calls(
        target,
        Vector2::new(abs_x, abs_y.saturating_add(shape_y)),
        rotation,
        FixedVec2::new(itofix(txdir), itofix(tydir)),
        itofix(trdir) / 10,
        true,
    )?;
    Ok(Value::Bool(exited))
}

/// FnSetComponent (C4Script.cpp:2659-2663): sets the component count on
/// pObj or the scope object (C4IDList::SetIDCount with fAddNewID — the
/// entry persists even at zero). Foreign subjects route through the seam.
pub(crate) fn set_component(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(component) = parse_native_c4id_argument(args.first(), "SetComponent")? else {
        return Ok(Value::Bool(false));
    };
    let count = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetComponent", "count")?;
    let target =
        parse_object_reference_argument(args.get(2).unwrap_or(&Value::Nil), "SetComponent", "obj")?;
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "SetComponent",
                &args[..2.min(args.len())],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(self_id) = context.object_context().map(|object| object.id()) else {
            return Ok(Value::Bool(false));
        };
        // Read-modify-write: the update replaces the whole map, so seed
        // from pending writes or the object's current components.
        let current = context
            .object_context()
            .and_then(|object| object.pending_update.components.clone())
            .or_else(|| {
                context
                    .get_world_object(self_id)
                    .and_then(|object| object.full_state().map(|state| state.components.clone()))
            })
            .unwrap_or_default();
        let mut order = context
            .object_context()
            .and_then(|object| object.pending_update.component_order.clone())
            .or_else(|| {
                context.get_world_object(self_id).and_then(|object| {
                    object
                        .full_state()
                        .map(|state| state.component_order.clone())
                })
            })
            .unwrap_or_default();
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        let mut map = current;
        let component = DefinitionId::from(component.as_str());
        if !order.contains(&component) {
            order.push(component.clone());
        }
        map.set(component, count);
        object.pending_update.components = Some(map);
        object.pending_update.component_order = Some(order);
        Ok(Value::Bool(true))
    })
}

/// FnGetDefinition (C4Script.cpp:2668-2677) indexes runtime `Game.Defs` order;
/// C4DefList::GetDef optionally filters by overlapping category bits without
/// reordering the surviving definitions (C4Def.cpp:1141-1158).
pub(crate) fn get_definition(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetDefinition",
        "index",
    )?;
    let category = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "GetDefinition",
        "category",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        Ok(borrow
            .as_ref()
            .and_then(|context| context.world.definition_id_by_index(index, category))
            .map(|id| Value::C4Id(id.as_str().to_string()))
            .unwrap_or(Value::Nil))
    })
}

/// FnValue (C4Script.cpp:1385-1389): the raw DefCore `Value` of a loaded
/// definition. Unlike GetValue this does not run CalcDefValue/CalcBuyValue;
/// an unloaded or zero ID returns null.
pub(crate) fn definition_value(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Value expects at most 1 argument: definition",
        ));
    }
    let Some(definition) = parse_native_c4id_argument(args.first(), "Value")? else {
        return Ok(Value::Nil);
    };
    HOST_CONTEXT.with(|cell| {
        Ok(cell
            .borrow()
            .as_ref()
            .and_then(|context| {
                context
                    .world
                    .definition_metadata(&DefinitionId::from(definition.as_str()))
            })
            .map_or(Value::Nil, |metadata| Value::Int(metadata.value)))
    })
}

/// `C4Def::GetValue` (C4Def.cpp:839-858): run the definition-owned
/// `CalcDefValue(base, player)` override, then the optional base-owned
/// `CalcBuyValue(definition, value)` adjustment. `None` means the definition
/// is not loaded, matching FnGetValue's null return for an unknown id.
pub(crate) fn calculated_definition_value(
    definition: &str,
    base: Option<ObjectId>,
    player: i32,
) -> Result<Option<i32>, RuntimeError> {
    let (metadata, script) = with_host_context((None, None), |context| {
        (
            context.definition_metadata(definition).cloned(),
            context.world.definition_script(definition).cloned(),
        )
    });
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let callback_args = [
        base.map(object_reference_value).unwrap_or(Value::Nil),
        Value::Int(player),
    ];
    let mut value = match script.and_then(|script| {
        call_scoped_definition_function(script, definition, "CalcDefValue", &callback_args)
    }) {
        Some(result) => result?.as_c4_int().unwrap_or(0),
        None => metadata.value,
    };
    if let Some(base) = base {
        if let Some(result) = call_world_object_own_function(
            base,
            "CalcBuyValue",
            &[Value::C4Id(definition.to_string()), Value::Int(value)],
        ) {
            value = result?.as_c4_int().unwrap_or(0);
        }
    }
    Ok(Some(value))
}

/// FnGetValue (C4Script.cpp:1366-1375): with a nonzero definition id, use
/// `C4Def::GetValue`; otherwise use the explicit object or `cthr->Obj` and
/// `C4Object::GetValue` (CalcValue/CalcDefValue, construction percentage,
/// then the containing base's CalcSellValue adjustment).
pub(crate) fn get_value(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetValue", "object")?;
    let definition = parse_native_c4id_argument(args.get(1), "GetValue")?;
    let base =
        parse_object_reference_argument(args.get(2).unwrap_or(&Value::Nil), "GetValue", "base")?;
    let player = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "GetValue", "player")?;

    if let Some(definition) = definition.filter(|id| !id.is_empty()) {
        return Ok(calculated_definition_value(&definition, base, player)?
            .map(Value::Int)
            .unwrap_or(Value::Nil));
    }

    let target = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        target.or_else(|| context.object_context().map(ObjectScopeContext::id))
    });
    let Some(target) = target else {
        return Ok(Value::Nil);
    };
    let Some(definition) = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| effective_definition_id(context, target))
    }) else {
        return Ok(Value::Nil);
    };

    let callback_args = [
        base.map(object_reference_value).unwrap_or(Value::Nil),
        Value::Int(player),
    ];
    let mut value = match call_world_object_own_function(target, "CalcValue", &callback_args) {
        Some(result) => result?.as_c4_int().unwrap_or(0),
        None => calculated_definition_value(&definition, None, player)?.unwrap_or(0),
    };
    let construction = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        context
            .object_scope(target)
            .map(ObjectScopeContext::construction)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.construction())
            })
    });
    let Some(construction) = construction else {
        return Ok(Value::Nil);
    };
    value = value.wrapping_mul(construction) / crate::FULL_CON;

    if let Some(base) = base {
        if let Some(result) = call_world_object_own_function(
            base,
            "CalcSellValue",
            &[object_reference_value(target), Value::Int(value)],
        ) {
            value = result?.as_c4_int().unwrap_or(0);
        }
    }
    Ok(Value::Int(value))
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
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => {
            match object_id_from_value(value) {
                Some(id) => Ok(ContainerFilter::Exact(id)),
                None => Err(RuntimeError::new(format!(
                    "{}: expected nonzero object reference for container",
                    function
                ))),
            }
        }
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected object reference or container sentinel, got {}",
            function,
            other.type_name()
        ))),
    }
}

/// `vContainer` (C4Script.cpp:2122-2127): an object filters by that exact
/// container; the NO_CONTAINER/ANY_CONTAINER int sentinels
/// (C4Object.h:83-84) filter by containment; anything else is
/// `C4Value::getObj()` = nil, i.e. no filter — never an error.
fn container_filter_from_value(value: Option<&Value>) -> ContainerFilter {
    match value {
        Some(Value::Int(raw)) if *raw == ANY_CONTAINER_SENTINEL => {
            ContainerFilter::RequiresContainer
        }
        Some(Value::Int(raw)) if *raw == NO_CONTAINER_SENTINEL => {
            ContainerFilter::RequiresNoContainer
        }
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => object_id_from_value(value)
            .map(ContainerFilter::Exact)
            .unwrap_or(ContainerFilter::Any),
        _ => ContainerFilter::Any,
    }
}

pub(crate) struct FindObjectParams {
    definition: Option<String>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    ocf_mask: u32,
    action: Option<String>,
    treat_idle: bool,
    action_target: Option<ObjectId>,
    exclude: Option<ObjectId>,
    container: ContainerFilter,
    owner: i32,
    find_next: Option<ObjectId>,
}

impl FindObjectParams {
    /// FnFindObject's script layout (C4Script.cpp:2113-2135): (id, x, y,
    /// wdt, hgt, dwOCF, szAction, pActionTarget, vContainer, pFindNext).
    /// Local calls exclude the caller and adjust x/y by its position
    /// (cthr->Obj). The owner filter is not script-settable here
    /// (C++ passes ANY_OWNER); FindObjectOwner injects it after parsing.
    fn parse_cpp_call(
        args: &[Value],
        function: &str,
        caller: Option<(ObjectId, Vector2)>,
    ) -> Result<Self, RuntimeError> {
        let definition = parse_native_c4id_argument(args.first(), function)?;
        let mut x = parse_optional_i32(args.get(1), function, "x")?.unwrap_or(0);
        let mut y = parse_optional_i32(args.get(2), function, "y")?.unwrap_or(0);
        let width = parse_optional_i32(args.get(3), function, "width")?.unwrap_or(0);
        let height = parse_optional_i32(args.get(4), function, "height")?.unwrap_or(0);
        // Adjust default ocf: an explicit 0 means OCF_All (C4Script.cpp:2120).
        let ocf_mask = parse_optional_u32(args.get(5), function, "ocf")?
            .filter(|&mask| mask != 0)
            .unwrap_or(crate::ocf::ALL);
        let action = parse_optional_string(args.get(6), function, "action")?;
        let treat_idle = matches!(action.as_deref(), Some("Idle") | Some("ActIdle"));
        let action_target = parse_object_reference_argument(
            args.get(7).unwrap_or(&Value::Nil),
            function,
            "action_target",
        )?;
        let container = container_filter_from_value(args.get(8));
        let find_next = parse_object_reference_argument(
            args.get(9).unwrap_or(&Value::Nil),
            function,
            "find_next",
        )?;
        // Local call adjust coordinates (C4Script.cpp:2115-2119).
        if let Some((_, position)) = caller {
            if x != 0 || y != 0 || width != 0 || height != 0 {
                x += position.x;
                y += position.y;
            }
        }
        Ok(Self {
            definition,
            x,
            y,
            width,
            height,
            ocf_mask,
            action,
            treat_idle,
            action_target,
            exclude: caller.map(|(id, _)| id),
            container,
            owner: OWNER_ANY,
            find_next,
        })
    }

    fn parse(args: &[Value]) -> Result<Self, RuntimeError> {
        if args.len() > 12 {
            return Err(RuntimeError::new(
                "FindObject: expected at most 12 arguments",
            ));
        }

        let definition = parse_native_c4id_argument(args.first(), "FindObject")?;
        let x = parse_optional_i32(args.get(1), "FindObject", "x")?.unwrap_or(0);
        let y = parse_optional_i32(args.get(2), "FindObject", "y")?.unwrap_or(0);
        let width = parse_optional_i32(args.get(3), "FindObject", "width")?.unwrap_or(0);
        let height = parse_optional_i32(args.get(4), "FindObject", "height")?.unwrap_or(0);
        let ocf_mask = parse_optional_u32(args.get(5), "FindObject", "ocf")?.unwrap_or(u32::MAX);
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
            ocf_mask,
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

    fn is_point_query(&self) -> bool {
        !self.is_full_range() && self.width == 0 && self.height == 0
    }

    fn is_rect_query(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    fn excludes_id(&self, id: ObjectId) -> bool {
        self.exclude == Some(id)
    }

    fn matches_fields(
        &self,
        id: ObjectId,
        definition_id: &str,
        status: ObjectStatus,
        ocf: u32,
        container: Option<ObjectId>,
        owner: i32,
        action_name: &str,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
    ) -> bool {
        if matches!(status, ObjectStatus::Deleted) {
            return false;
        }

        if let Some(exclude) = self.exclude {
            if id == exclude {
                return false;
            }
        }

        if let Some(definition) = &self.definition {
            if definition_id != definition {
                return false;
            }
        }

        if self.ocf_mask != ocf::ALL && ocf & self.ocf_mask == 0 {
            return false;
        }

        match self.container {
            ContainerFilter::Any => {}
            ContainerFilter::Exact(expected) => {
                if container != Some(expected) {
                    return false;
                }
            }
            ContainerFilter::RequiresContainer => {
                if container.is_none() {
                    return false;
                }
            }
            ContainerFilter::RequiresNoContainer => {
                if container.is_some() {
                    return false;
                }
            }
        }

        if self.owner != OWNER_ANY && owner != self.owner {
            return false;
        }

        if let Some(target) = self.action_target {
            let matches = action_target == Some(target) || action_target2 == Some(target);
            if !matches {
                return false;
            }
        }

        if let Some(action) = self.action.as_deref() {
            if !action.is_empty() {
                if self.treat_idle {
                    if action_name != "Idle" && action_name != "ActIdle" {
                        return false;
                    }
                } else if action_name != action {
                    return false;
                }
            }
        }

        true
    }

    pub(crate) fn matches_object(&self, object: &HostWorldObject) -> bool {
        self.matches_fields(
            object.id,
            object.definition_id(),
            object.status(),
            object.ocf(),
            object.container(),
            object.owner(),
            object.action_name(),
            object.action_target(0),
            object.action_target(1),
        )
    }

    pub(crate) fn matches_engine_object(&self, object: &crate::Object) -> bool {
        self.matches_fields(
            object.id,
            object.definition_id.as_str(),
            object.state.status,
            object.state.ocf,
            object.state.container,
            object.state.owner,
            object.state.action.name.as_str(),
            object.state.action.target,
            object.state.action.target2,
        )
    }

    fn matches_area(&self, world: &impl WorldAccessor, object: &HostWorldObject) -> bool {
        if self.is_full_range() || self.is_closest_query() {
            return true;
        }

        if self.is_point_query() {
            return world
                .object_shape_rect(object)
                .contains_point(self.x, self.y);
        }

        if self.is_rect_query() {
            let position = object.position();
            let dx = position.x - self.x;
            let dy = position.y - self.y;
            return dx >= 0 && dx < self.width && dy >= 0 && dy < self.height;
        }

        false
    }

    fn reference_distance(&self, world: &impl WorldAccessor) -> Option<i64> {
        let id = self.find_next?;
        let object = world.get_object(id)?;
        Some(squared_distance(object.position(), self.x, self.y))
    }

    /// Sector-prefiltered candidates for the port-internal fixed-parameter
    /// FindObjects (modelled on C4FindObject::FindMany's bounded arms —
    /// this form has no C++ counterpart) and the order-insensitive
    /// ObjectCount. The legacy single-result FindObject does NOT use this:
    /// C4Game::FindObject scans the MASTER list for every query form
    /// (C4Game.cpp:1367-1424).
    fn candidate_ids(&self, world: &impl WorldAccessor) -> Vec<ObjectId> {
        if self.is_closest_query() || self.is_full_range() {
            return world.master_object_ids();
        }

        if self.is_point_query() {
            let rect = DefinitionRect::new(self.x, self.y, 1, 1);
            return world
                .shape_sector_ids_in_rect(rect)
                .unwrap_or_else(|| world.object_ids());
        }

        if self.is_rect_query() {
            let rect = DefinitionRect::new(self.x, self.y, self.width, self.height);
            return world
                .object_sector_ids_in_rect(rect)
                .unwrap_or_else(|| world.object_ids());
        }

        Vec::new()
    }
}

pub(crate) fn construction_to_script_value(construction: i32) -> i32 {
    ((construction.max(0) as i64) * 100 / (FULL_CON as i64)) as i32
}

pub(crate) fn construction_delta_from_percent(percent: i32) -> i32 {
    ((percent as i64) * (FULL_CON as i64) / 100) as i32
}

/// FnScrollContents (C4Script.cpp:1793-1805): move the raw first contents
/// link to the back and return the new first object. Unlike ShiftContents,
/// this always advances exactly one link, including within a uniform stack.
pub(crate) fn scroll_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_object = parse_native_object_argument(args.first(), "ScrollContents", "target")?;

    with_host_context_mut(Ok(Value::Nil), |context| {
        let Some(target) = target_object.or(context.script_object_context) else {
            return Ok(Value::Nil);
        };
        let Some(container) = context.get_world_object(target) else {
            return Ok(Value::Nil);
        };
        let contents: Vec<ObjectId> = container
            .contents()
            .iter()
            .copied()
            .filter(|child| {
                context
                    .get_world_object(*child)
                    .is_some_and(|object| object.is_present())
            })
            .collect();
        let Some(front) = contents.first().copied() else {
            return Ok(Value::Nil);
        };
        let new_front = contents.get(1).copied().unwrap_or(front);
        let _ = context.move_content_link_to_back(target, front);
        if !context.ensure_object_scope(target) {
            return Ok(Value::Nil);
        }
        let Some(container) = context.object_scope_mut(target) else {
            return Ok(Value::Nil);
        };
        if new_front != front {
            container.shift_contents_front(new_front);
        }
        Ok(object_reference_value(new_front))
    })
}

/// FnShiftContents (C4Script.cpp:1784-1797): rotate the contents list so a
/// different item comes first (C4Object::ShiftContents,
/// C4Object.cpp:5728-5752) or bring the first idTarget content to the
/// front (DirectComContents, :5754-5775). The rotation itself is the C++
/// cyclic relink (C4ObjectList.cpp:815-833), applied via
/// ObjectUpdate.contents_front; with fDoCalls the container's
/// ~ControlContents(id) may veto and the new front gets
/// ~Selection(container) with the Grab sound on a falsy return
/// (C4Object.cpp:5760-5767); the menu Refill (:5769-5772) is
/// presentation-only and unmodeled. Unfiltered shifts reuse the same live
/// CanConcatPictureWith predicate as internal menu grouping, including
/// same-call appearance writes. Contents include same-call CreateContents/
/// Enter scopes, matching the live C4ObjectList.
pub(crate) fn shift_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_object = match args.first() {
        None | Some(Value::Nil | Value::Int(0)) => None,
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => {
            parse_object_reference_argument(value, "ShiftContents", "target")?
        }
        // C++ par conversion nils a non-object slot -> local call.
        Some(_) => None,
    };
    let shift_back = args
        .get(1)
        .map(|value| value_to_bool(value, "ShiftContents", "shift back"))
        .transpose()?
        .unwrap_or(false);
    let id_target = parse_native_c4id_argument(args.get(2), "ShiftContents")?;
    let do_calls = args
        .get(3)
        .map(|value| value_to_bool(value, "ShiftContents", "do calls"))
        .transpose()?
        .unwrap_or(false);

    // FnShiftContents' pObj may name ANOTHER container (C4Script.cpp:1786
    // only defaults nil to cthr->Obj) — re-enter through the nested seam so
    // the target's own scope runs the shift (the ObjectSetAction pattern).
    let active = active_object_id();
    if let Some(target) = target_object {
        if Some(target) != active {
            let forwarded = vec![
                Value::Nil,
                Value::Bool(shift_back),
                id_target.map(Value::C4Id).unwrap_or(Value::Nil),
                Value::Bool(do_calls),
            ];
            return match call_world_object_function(target, "ShiftContents", &forwarded) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    // Phase 1 (borrowed): resolve the frame-start contents view and pick
    // the new front — released before the DirectComContents calls re-enter.
    enum Picked {
        Done(Value),
        Shift {
            container: ObjectId,
            new_front: ObjectId,
            new_front_id: String,
        },
    }
    let picked = with_host_context_mut(Picked::Done(Value::Bool(false)), |context| {
        let Some(self_id) = context.object_context().map(|object| object.id()) else {
            return Picked::Done(Value::Bool(false));
        };
        let contents: Vec<(ObjectId, String)> = match context.get_world_object(self_id) {
            Some(container) => container
                .contents()
                .iter()
                .filter_map(|child_id| {
                    context
                        .get_world_object(*child_id)
                        .filter(|child| child.is_present())
                        .map(|child| (*child_id, child.definition_id().to_string()))
                })
                .collect(),
            None => return Picked::Done(Value::Bool(false)),
        };
        let Some((front_id, _)) = contents.first().cloned() else {
            return Picked::Done(Value::Bool(false));
        };
        let new_front = if let Some(id_target) = id_target {
            // Check if the ID is present within the container
            // (C4Script.cpp:1790-1793).
            let Some((found, _)) = contents
                .iter()
                .find(|(_, definition)| *definition == id_target)
            else {
                return Picked::Done(Value::Bool(false));
            };
            // Desired object already at front? (DirectComContents :5759.)
            if *found == front_id {
                return Picked::Done(Value::Bool(true));
            }
            *found
        } else {
            // Walk next (or prev from the back with fShiftBack) for the
            // first DIFFERENT item (C4Object.cpp:5734-5750).
            let candidate = if shift_back {
                contents
                    .iter()
                    .skip(1)
                    .rev()
                    .find(|(id, _)| !context.object_can_concat_picture_with(front_id, *id))
            } else {
                contents
                    .iter()
                    .skip(1)
                    .find(|(id, _)| !context.object_can_concat_picture_with(front_id, *id))
            };
            match candidate {
                Some((id, _)) => *id,
                None => return Picked::Done(Value::Bool(false)),
            }
        };
        let new_front_id = contents
            .iter()
            .find(|(id, _)| *id == new_front)
            .map(|(_, definition)| definition.clone())
            .unwrap_or_default();
        Picked::Shift {
            container: self_id,
            new_front,
            new_front_id,
        }
    });
    let (container, new_front, new_front_id) = match picked {
        Picked::Done(value) => return Ok(value),
        Picked::Shift {
            container,
            new_front,
            new_front_id,
        } => (container, new_front, new_front_id),
    };
    // DirectComContents (C4Object.cpp:5760-5763): with fDoCalls the
    // container's ~ControlContents(idNewFront) runs first — a truthy
    // return takes over the selection (fail-safe exec, errors read false).
    if do_calls {
        let veto = match call_world_object_own_function(
            container,
            "ControlContents",
            &[Value::C4Id(new_front_id)],
        ) {
            Some(Ok(value)) => value.as_bool(),
            Some(Err(error)) => {
                tracing::error!(
                    %error,
                    "script error in ControlContents; continuing like the C++ fail-safe exec"
                );
                log_runtime_call_frames("", error.call_frames());
                false
            }
            None => false,
        };
        if veto {
            return Ok(Value::Bool(true));
        }
    }
    // The cyclic relink (C4ObjectList::ShiftContents, C4ObjectList.cpp:
    // 815-833) via ObjectUpdate.contents_front.
    let shifted = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let shifted = context.rotate_contents_link_to_front(container, new_front);
        if shifted {
            if let Some(object) = context.object_context_mut() {
                object.shift_contents_front(new_front);
            }
        }
        Some(shifted)
    });
    let Some(shifted) = shifted else {
        return Ok(Value::Bool(false));
    };
    if !shifted {
        // C4Object::ShiftContents already selected a candidate and returns
        // true after DirectComContents even if its callback removed that
        // candidate before the one-shot relink (C4Object.cpp:5790-5828).
        return Ok(Value::Bool(true));
    }
    // ~Selection(container) on the new front; a falsy return plays the
    // Grab sound at the container (C4Object.cpp:5767).
    if do_calls {
        let selected = match call_world_object_own_function(
            new_front,
            "Selection",
            &[object_reference_value(container)],
        ) {
            Some(Ok(value)) => value.as_bool(),
            Some(Err(error)) => {
                tracing::error!(
                    %error,
                    "script error in Selection; continuing like the C++ fail-safe exec"
                );
                log_runtime_call_frames("", error.call_frames());
                false
            }
            None => false,
        };
        if !selected {
            HOST_CONTEXT.with(|cell| {
                if let Some(context) = cell.borrow_mut().as_mut() {
                    let _ = context.play_sound("Grab", Some(container), 100, false, false, None);
                }
            });
        }
    }
    Ok(Value::Bool(true))
}

// ── C4FindObject / C4SortObject condition trees (C4FindObject.{h,cpp}) ──────

/// C4FO_* constants (C4FindObject.h:27-50) as a parsed condition tree.
/// Known divergences: `Layer` is unmodeled on host objects (never
/// matches); shape tests use the vertices bounding box.
#[derive(Debug, Clone)]
pub(crate) enum FindCondition {
    Not(Box<FindCondition>),
    And(Vec<FindCondition>),
    Or(Vec<FindCondition>),
    Exclude(Option<ObjectId>),
    Id(usize),
    InRect(DefinitionRect),
    AtPoint(i32, i32),
    AtRect(DefinitionRect),
    OnLine(i32, i32, i32, i32),
    Distance {
        x: i32,
        y: i32,
        r2: i64,
        /// The enclosing square the C++ constructor precomputes for
        /// GetBounds (C4FindObject.h:253).
        bounds: DefinitionRect,
    },
    Ocf(u32),
    Category(i32),
    Action(String),
    ActionTarget {
        target: Option<ObjectId>,
        index: usize,
    },
    Container(Option<ObjectId>),
    AnyContainer,
    Owner(i32),
    Controller(i32),
    /// C4FindObjectFunc (C4FindObject.cpp:124-136): calls `name` on each
    /// candidate via the nested-call seam.
    Func {
        name: String,
        pars: Vec<Value>,
    },
    /// C4FindObjectLayer (C4FindObject.cpp:671-674): `pObj->pLayer ==
    /// pLayer` — None matches every unlayered object.
    Layer(Option<ObjectId>),
}

/// C4SO_* constants (C4FindObject.h:53-62) as a parsed sort tree.
#[derive(Debug, Clone)]
pub(crate) enum SortCriterion {
    Reverse(Box<SortCriterion>),
    Multiple(Vec<SortCriterion>),
    Distance {
        x: i32,
        y: i32,
    },
    Random,
    Speed,
    Mass,
    Value,
    /// C4SortObjectFunc (C4FindObject.h:521-533): evaluates the named
    /// callback for each candidate and compares its integer result.
    Func {
        name: String,
        pars: Vec<Value>,
    },
}

pub(crate) enum ParsedCriterion {
    Condition(FindCondition),
    Sort(SortCriterion),
    None,
}

/// C4FindObjectFunc/C4SortObjectFunc store their arguments in C4Value cells.
/// AssignRemoval clears registered object references synchronously, so a
/// later candidate must receive nil even though Rust's parsed criterion tree
/// owns an ordinary Value clone.
fn live_find_callback_parameters(pars: &[Value]) -> Vec<Value> {
    let mut live = pars.to_vec();
    with_host_context((), |context| {
        for value in &mut live {
            clear_removed_object_references(value, &context.removed_object_references);
        }
    });
    live
}

impl FindCondition {
    /// `C4FindObject::CreateByValue` (C4FindObject.cpp:37-162): arrays whose
    /// first element is in C4SO_First..=C4SO_Last parse as sort criteria
    /// instead.
    pub(crate) fn parse(value: &Value) -> ParsedCriterion {
        // Must be an array (C4FindObject.cpp:40-41)
        let Value::Array(data) = value else {
            return ParsedCriterion::None;
        };
        let kind = data.first().map(value_as_i32).unwrap_or(0);
        if (100..=200).contains(&kind) {
            return SortCriterion::parse_typed(kind, data)
                .map(ParsedCriterion::Sort)
                .unwrap_or(ParsedCriterion::None);
        }
        let arg_i32 = |index: usize| data.get(index).map(value_as_i32).unwrap_or(0);
        let condition = match kind {
            // C4FO_Not
            1 => match data.get(1).map(Self::parse) {
                Some(ParsedCriterion::Condition(child)) => FindCondition::Not(Box::new(child)),
                _ => return ParsedCriterion::None,
            },
            // C4FO_And / C4FO_Or: trivial single-condition unwrap, dropped
            // null children (C4FindObject.cpp:67-87)
            2 | 3 => {
                let children: Vec<FindCondition> = data[1..]
                    .iter()
                    .filter_map(|entry| match Self::parse(entry) {
                        ParsedCriterion::Condition(child) => Some(child),
                        _ => None,
                    })
                    .collect();
                if data.len() == 2 {
                    match children.into_iter().next() {
                        Some(child) => child,
                        None => return ParsedCriterion::None,
                    }
                } else if kind == 2 {
                    FindCondition::And(children)
                } else {
                    FindCondition::Or(children)
                }
            }
            // C4FO_Exclude
            5 => FindCondition::Exclude(data.get(1).and_then(value_as_object_id)),
            // C4FO_InRect
            10 => FindCondition::InRect(DefinitionRect::new(
                arg_i32(1),
                arg_i32(2),
                arg_i32(3),
                arg_i32(4),
            )),
            // C4FO_AtPoint
            11 => FindCondition::AtPoint(arg_i32(1), arg_i32(2)),
            // C4FO_AtRect
            12 => FindCondition::AtRect(DefinitionRect::new(
                arg_i32(1),
                arg_i32(2),
                arg_i32(3),
                arg_i32(4),
            )),
            // C4FO_OnLine
            13 => FindCondition::OnLine(arg_i32(1), arg_i32(2), arg_i32(3), arg_i32(4)),
            // C4FO_Distance
            14 => {
                let r = arg_i32(3);
                let (x, y) = (arg_i32(1), arg_i32(2));
                FindCondition::Distance {
                    x,
                    y,
                    r2: i64::from(r) * i64::from(r),
                    // (x - r, y - r, 2r + 1, 2r + 1), C4FindObject.h:253 —
                    // wrapping like the C++ int32 arithmetic.
                    bounds: DefinitionRect::new(
                        x.wrapping_sub(r),
                        y.wrapping_sub(r),
                        r.wrapping_mul(2).wrapping_add(1),
                        r.wrapping_mul(2).wrapping_add(1),
                    ),
                }
            }
            // C4FO_ID
            20 => match data.get(1) {
                Some(Value::C4Id(id)) => FindCondition::Id(cast_c4id_payload(id)),
                Some(Value::String(id)) => FindCondition::Id(clonk_script::c4_id_parse(id)),
                _ => return ParsedCriterion::None,
            },
            // C4FO_OCF
            21 => FindCondition::Ocf(arg_i32(1) as u32),
            // C4FO_Category
            22 => FindCondition::Category(arg_i32(1)),
            // C4FO_Action
            30 => match data.get(1) {
                Some(Value::String(name)) => FindCondition::Action(name.as_ref().to_owned()),
                _ => return ParsedCriterion::None,
            },
            // C4FO_ActionTarget (index clamped to 0..=1, C4FindObject.cpp:138-144)
            31 => FindCondition::ActionTarget {
                target: data.get(1).and_then(value_as_object_id),
                index: arg_i32(2).clamp(0, 1) as usize,
            },
            // C4FO_Container
            40 => FindCondition::Container(data.get(1).and_then(value_as_object_id)),
            // C4FO_AnyContainer
            41 => FindCondition::AnyContainer,
            // C4FO_Owner
            50 => FindCondition::Owner(arg_i32(1)),
            // C4FO_Controller
            51 => FindCondition::Controller(arg_i32(1)),
            // C4FO_Func: Data[1] must convert to a string, else the whole
            // criterion is dropped (C4FindObject.cpp:127-128); Data[2] →
            // par 0, capped at 10 pars (SetPar, C4FindObject.cpp:645-651)
            60 => match data.get(1) {
                Some(Value::String(name)) => FindCondition::Func {
                    name: name.as_ref().to_owned(),
                    pars: data.iter().skip(2).take(10).cloned().collect(),
                },
                _ => return ParsedCriterion::None,
            },
            // C4FO_Layer: Data[1].getObj(), nil = the unlayered world
            // (C4FindObject.cpp:157-159).
            70 => FindCondition::Layer(data.get(1).and_then(value_as_object_id)),
            _ => return ParsedCriterion::None,
        };
        ParsedCriterion::Condition(condition)
    }

    /// Per-condition Check (C4FindObject.cpp:390-679). Fallible because a
    /// `Func` callback error passes through (`fPassErrors=true`,
    /// C4FindObject.cpp:661); And/Or evaluate children in array order with
    /// short-circuit, so Func side effects land in C++ order.
    pub(crate) fn check(
        &self,
        world: &impl WorldAccessor,
        object: &HostWorldObject,
    ) -> Result<bool, RuntimeError> {
        // Every C++ node dereferences the same live C4Object pointer. An
        // earlier Func sibling may have changed any field. Scalar nodes
        // cannot mutate it, so retain the driver's fresh clone until a Func
        // actually runs. A vanished pending preview falls back to that
        // pointer-equivalent clone for the remainder of the current Check.
        Ok(match self {
            FindCondition::Not(child) => !child.check(world, object)?,
            FindCondition::And(children) => {
                let mut refreshed = None;
                for (index, child) in children.iter().enumerate() {
                    if !child.check(world, refreshed.as_ref().unwrap_or(object))? {
                        return Ok(false);
                    }
                    if index + 1 < children.len() && child.uses_func() {
                        #[cfg(test)]
                        FIND_CONDITION_OBJECT_REFRESHES.with(|count| count.set(count.get() + 1));
                        refreshed = world.get_object(object.id);
                    }
                }
                true
            }
            FindCondition::Or(children) => {
                let mut refreshed = None;
                for (index, child) in children.iter().enumerate() {
                    if child.check(world, refreshed.as_ref().unwrap_or(object))? {
                        return Ok(true);
                    }
                    if index + 1 < children.len() && child.uses_func() {
                        #[cfg(test)]
                        FIND_CONDITION_OBJECT_REFRESHES.with(|count| count.set(count.get() + 1));
                        refreshed = world.get_object(object.id);
                    }
                }
                false
            }
            FindCondition::Exclude(excluded) => Some(object.id) != *excluded,
            FindCondition::Id(id) => clonk_script::c4_id_parse(object.definition_id()) == *id,
            FindCondition::InRect(rect) => {
                // C4FindObjectInRect::Check is a plain point-in-rect on the
                // object CENTER (C4FindObject.cpp) — the old
                // contains_offset(pos - rect.xy) clause double-subtracted
                // the origin and matched far-away objects whose offset
                // happened to land back inside (the 597 NoDmg class).
                let position = object.position();
                position.x >= rect.x
                    && position.x < rect.x + rect.width
                    && position.y >= rect.y
                    && position.y < rect.y + rect.height
            }
            FindCondition::AtPoint(x, y) => {
                rect_contains_point_cpp(world.object_live_shape_rect(object), *x, *y)
            }
            FindCondition::AtRect(rect) => {
                rects_overlap_cpp(world.object_live_shape_rect(object), *rect)
            }
            FindCondition::OnLine(x1, y1, x2, y2) => {
                rect_intersects_line_cpp(world.object_live_shape_rect(object), *x1, *y1, *x2, *y2)
            }
            FindCondition::Distance { x, y, r2, .. } => {
                let position = object.position();
                let dx = i64::from(position.x - x);
                let dy = i64::from(position.y - y);
                dx * dx + dy * dy <= *r2
            }
            FindCondition::Ocf(mask) => object.ocf() & mask != 0,
            FindCondition::Category(category) => object.category() & category != 0,
            FindCondition::Action(name) => object.action_name() == name,
            FindCondition::ActionTarget { target, index } => {
                object.action_target(*index) == *target
            }
            FindCondition::Container(container) => object.container() == *container,
            FindCondition::AnyContainer => object.container().is_some(),
            FindCondition::Owner(owner) => object.owner() == *owner,
            // C4FindObjectController::Check (C4FindObject.cpp:628-631).
            FindCondition::Controller(controller) => object.controller() == *controller,
            // C4FindObjectFunc::Check (C4FindObject.cpp:653-662): no
            // overload visible to the object's def → silently false; the
            // result converts with raw C4Value truthiness, not getBool.
            FindCondition::Func { name, pars } => {
                let pars = live_find_callback_parameters(pars);
                match call_world_object_function(object.id, name, &pars) {
                    None => false,
                    Some(result) => value_raw_truthy(&result?),
                }
            }
            // C4FindObjectLayer::Check (C4FindObject.cpp:671-674).
            FindCondition::Layer(layer) => {
                object.full_state().and_then(|state| state.layer) == *layer
            }
        })
    }

    /// Evaluate the scalar-only subset directly on the engine's live object.
    /// `None` keeps shape-dependent and callback-bearing conditions on the
    /// complete host-object path.
    pub(crate) fn matches_engine_object(&self, object: &crate::Object) -> Option<bool> {
        match self {
            Self::Not(child) => child.matches_engine_object(object).map(|matches| !matches),
            Self::And(children) => {
                for child in children {
                    if !child.matches_engine_object(object)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Self::Or(children) => {
                for child in children {
                    if child.matches_engine_object(object)? {
                        return Some(true);
                    }
                }
                Some(false)
            }
            Self::Exclude(excluded) => Some(Some(object.id) != *excluded),
            Self::Id(id) => Some(clonk_script::c4_id_parse(&object.definition_id) == *id),
            Self::InRect(rect) => {
                let position = object.state.position;
                Some(
                    position.x >= rect.x
                        && position.x < rect.x + rect.width
                        && position.y >= rect.y
                        && position.y < rect.y + rect.height,
                )
            }
            Self::Distance { x, y, r2, .. } => {
                let dx = i64::from(object.state.position.x - x);
                let dy = i64::from(object.state.position.y - y);
                Some(dx * dx + dy * dy <= *r2)
            }
            Self::Ocf(mask) => Some(object.state.ocf & mask != 0),
            Self::Category(category) => Some(object.state.category & category != 0),
            Self::Action(name) => Some(object.state.action.name == *name),
            Self::ActionTarget { target, index } => Some(
                match index {
                    0 => object.state.action.target,
                    _ => object.state.action.target2,
                } == *target,
            ),
            Self::Container(container) => Some(object.state.container == *container),
            Self::AnyContainer => Some(object.state.container.is_some()),
            Self::Owner(owner) => Some(object.state.owner == *owner),
            Self::Controller(controller) => Some(object.state.controller == *controller),
            Self::Layer(layer) => Some(object.state.layer == *layer),
            Self::AtPoint(..) | Self::AtRect(..) | Self::OnLine(..) | Self::Func { .. } => None,
        }
    }

    /// Evaluate the same non-callback prefix on a callback-local host
    /// projection. `None` means evaluation reached a shape predicate or
    /// `Find_Func`; `Some` is already final under C++ short-circuit order.
    pub(crate) fn matches_host_object(&self, object: &HostWorldObject) -> Option<bool> {
        match self {
            Self::Not(child) => child.matches_host_object(object).map(|matches| !matches),
            Self::And(children) => {
                for child in children {
                    if !child.matches_host_object(object)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Self::Or(children) => {
                for child in children {
                    if child.matches_host_object(object)? {
                        return Some(true);
                    }
                }
                Some(false)
            }
            Self::Exclude(excluded) => Some(Some(object.id) != *excluded),
            Self::Id(id) => Some(clonk_script::c4_id_parse(object.definition_id()) == *id),
            Self::InRect(rect) => {
                let position = object.position();
                Some(
                    position.x >= rect.x
                        && position.x < rect.x + rect.width
                        && position.y >= rect.y
                        && position.y < rect.y + rect.height,
                )
            }
            Self::Distance { x, y, r2, .. } => {
                let position = object.position();
                let dx = i64::from(position.x - x);
                let dy = i64::from(position.y - y);
                Some(dx * dx + dy * dy <= *r2)
            }
            Self::Ocf(mask) => Some(object.ocf() & mask != 0),
            Self::Category(category) => Some(object.category() & category != 0),
            Self::Action(name) => Some(object.action_name() == name),
            Self::ActionTarget { target, index } => Some(object.action_target(*index) == *target),
            Self::Container(container) => Some(object.container() == *container),
            Self::AnyContainer => Some(object.container().is_some()),
            Self::Owner(owner) => Some(object.owner() == *owner),
            Self::Controller(controller) => Some(object.controller() == *controller),
            Self::Layer(layer) => Some(object.full_state().and_then(|state| state.layer) == *layer),
            Self::AtPoint(..) | Self::AtRect(..) | Self::OnLine(..) | Self::Func { .. } => None,
        }
    }

    /// IsImpossible/IsEnsured pruning (C4FindObject.cpp:453-590). `Func` is
    /// impossible only when the name is unknown to every script
    /// (GetFirstFunc miss at construction, C4FindObject.cpp:640-643,
    /// 664-667); Not swaps the two (C4FindObject.h:116-117).
    fn is_impossible(&self, world: &impl WorldAccessor) -> bool {
        match self {
            FindCondition::Not(child) => child.is_ensured(world),
            FindCondition::And(children) => children.iter().any(|child| child.is_impossible(world)),
            FindCondition::Or(children) => {
                !children.iter().any(|child| !child.is_impossible(world))
            }
            FindCondition::InRect(rect) => rect.width == 0 || rect.height == 0,
            FindCondition::Ocf(mask) => *mask == 0,
            FindCondition::Func { name, .. } => !world.script_function_known(name),
            _ => false,
        }
    }

    fn is_ensured(&self, world: &impl WorldAccessor) -> bool {
        match self {
            FindCondition::Not(child) => child.is_impossible(world),
            // C4FindObjectAnd::IsEnsured is `!iCnt` AFTER the constructor
            // filtered ensured children (C4FindObject.h:135) — recursively:
            // ensured iff every child is.
            FindCondition::And(children) => children.iter().all(|child| child.is_ensured(world)),
            // C4FindObjectOr::IsEnsured (C4FindObject.cpp:514-520)
            FindCondition::Or(children) => children.iter().any(|child| child.is_ensured(world)),
            FindCondition::Category(category) => *category == 0,
            _ => false,
        }
    }

    /// The construction-time pruning of C4FindObjectAnd/Or
    /// (C4FindObject.cpp:400-410, 466-476), bottom-up like CreateByValue:
    /// And drops ensured children (whose Check may still be false —
    /// Category(0), C4FindObject.cpp:582-590), Or drops impossible ones
    /// (which would otherwise kill the sibling bounds). The drivers prune
    /// once before Check/bounds/IsImpossible run, matching the C++ tree.
    fn pruned(self, world: &impl WorldAccessor) -> FindCondition {
        match self {
            FindCondition::Not(child) => FindCondition::Not(Box::new(child.pruned(world))),
            FindCondition::And(children) => FindCondition::And(
                children
                    .into_iter()
                    .map(|child| child.pruned(world))
                    .filter(|child| !child.is_ensured(world))
                    .collect(),
            ),
            FindCondition::Or(children) => FindCondition::Or(
                children
                    .into_iter()
                    .map(|child| child.pruned(world))
                    .filter(|child| !child.is_impossible(world))
                    .collect(),
            ),
            other => other,
        }
    }

    /// GetBounds + UseShapes (C4FindObject.h:93-94): `Some((rect, shapes))`
    /// when the criteria bound the search area — the drivers then walk the
    /// sector lists (shape lists when `shapes`) instead of the master list.
    pub(crate) fn bounds(&self) -> Option<(DefinitionRect, bool)> {
        match self {
            // C4FindObjectAnd constructor (C4FindObject.cpp:411-434): a
            // bounded shapes child wins outright ("some objects might be in
            // an rect and at a point not in that rect"); otherwise all
            // bounded children intersect.
            FindCondition::And(children) => {
                let mut bounds: Option<DefinitionRect> = None;
                for child in children {
                    if let Some((child_bounds, child_shapes)) = child.bounds() {
                        if child_shapes {
                            return Some((child_bounds, true));
                        }
                        bounds = Some(match bounds {
                            Some(rect) => rect_intersect_cpp(rect, child_bounds),
                            None => child_bounds,
                        });
                    }
                }
                bounds.map(|rect| (rect, false))
            }
            // C4FindObjectOr constructor (C4FindObject.cpp:477-496): the
            // union of all child bounds; a boundless or shapes child (which
            // could report an object twice) kills the bounds entirely.
            FindCondition::Or(children) => {
                let mut bounds: Option<DefinitionRect> = None;
                for child in children {
                    match child.bounds() {
                        None | Some((_, true)) => return None,
                        Some((child_bounds, false)) => {
                            bounds = Some(match bounds {
                                Some(rect) => rect_add_cpp(rect, child_bounds),
                                None => child_bounds,
                            });
                        }
                    }
                }
                bounds.map(|rect| (rect, false))
            }
            FindCondition::InRect(rect) => Some((*rect, false)),
            FindCondition::AtPoint(x, y) => Some((DefinitionRect::new(*x, *y, 1, 1), true)),
            FindCondition::AtRect(rect) => Some((*rect, true)),
            // bounds(x, y, 1, 1) + Add((x2, y2, 1, 1)), C4FindObject.h:234-237
            FindCondition::OnLine(x1, y1, x2, y2) => Some((
                rect_add_cpp(
                    DefinitionRect::new(*x1, *y1, 1, 1),
                    DefinitionRect::new(*x2, *y2, 1, 1),
                ),
                true,
            )),
            FindCondition::Distance { bounds, .. } => Some((*bounds, false)),
            _ => None,
        }
    }

    /// Whether any node needs the nested-call seam (drives the borrow-free
    /// live-view evaluation path in the drivers).
    fn uses_func(&self) -> bool {
        match self {
            FindCondition::Not(child) => child.uses_func(),
            FindCondition::And(children) | FindCondition::Or(children) => {
                children.iter().any(FindCondition::uses_func)
            }
            FindCondition::Func { .. } => true,
            _ => false,
        }
    }
}

impl SortCriterion {
    /// `C4SortObject::CreateByValue` (C4FindObject.cpp:683-758).
    fn parse_typed(kind: i32, data: &[Value]) -> Option<SortCriterion> {
        let arg_i32 = |index: usize| data.get(index).map(value_as_i32).unwrap_or(0);
        Some(match kind {
            // C4SO_Reverse
            101 => SortCriterion::Reverse(Box::new(data.get(1).and_then(Self::parse)?)),
            // C4SO_Multiple (trivial single unwrap, C4FindObject.cpp:705-726)
            102 => {
                let children: Vec<SortCriterion> =
                    data[1..].iter().filter_map(Self::parse).collect();
                if data.len() == 2 {
                    children.into_iter().next()?
                } else {
                    SortCriterion::Multiple(children)
                }
            }
            // C4SO_Distance
            110 => SortCriterion::Distance {
                x: arg_i32(1),
                y: arg_i32(2),
            },
            // C4SO_Random
            120 => SortCriterion::Random,
            // C4SO_Speed
            130 => SortCriterion::Speed,
            // C4SO_Mass
            140 => SortCriterion::Mass,
            // C4SO_Value
            150 => SortCriterion::Value,
            // C4SO_Func: string name required, else nullptr
            // (C4FindObject.cpp:743-755); pars capped at 10
            160 => match data.get(1) {
                Some(Value::String(name)) => SortCriterion::Func {
                    name: name.as_ref().to_owned(),
                    pars: data.iter().skip(2).take(10).cloned().collect(),
                },
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Whether any node needs the nested-call seam.
    fn uses_func(&self) -> bool {
        match self {
            SortCriterion::Reverse(child) => child.uses_func(),
            SortCriterion::Multiple(children) => children.iter().any(SortCriterion::uses_func),
            // C4SO_Value calls C4Object::GetValue, which may dispatch the
            // definition's CalcValue script just like C4SO_Func.
            SortCriterion::Value | SortCriterion::Func { .. } => true,
            _ => false,
        }
    }

    fn parse(value: &Value) -> Option<SortCriterion> {
        let Value::Array(data) = value else {
            return None;
        };
        Self::parse_typed(data.first().map(value_as_i32).unwrap_or(0), data)
    }

    /// `CompareGetValue` (C4FindObject.cpp:908-956). `Random` draws the
    /// synced `Random(1 << 16)` — exactly once per object, in collection
    /// order, via the cache (C4SortObjectByValue::PrepareCache). `Func`
    /// runs the nested call: no overload → 0 silently, the result converts
    /// with `getInt()` (bools 0/1, pointer types 0), and callback errors
    /// pass through (`fPassErrors=true`, C4FindObject.cpp:947-956).
    fn value_for(
        &self,
        world: &impl WorldAccessor,
        object: &HostWorldObject,
    ) -> Result<i64, RuntimeError> {
        // CompareGetValue dereferences the live pointer for every criterion.
        // In particular, Multiple prepares one complete cache at a time, so
        // a Func cache may mutate fields consumed by the next cache.
        let refreshed = world.get_object(object.id);
        let object = refreshed.as_ref().unwrap_or(object);
        Ok(match self {
            SortCriterion::Distance { x, y } => {
                let position = object.position();
                let dx = position.x.wrapping_sub(*x);
                let dy = position.y.wrapping_sub(*y);
                // int32 wrap like C4SortObjectDistance
                // (C4FindObject.cpp:908-911, CompareGetValue is int32_t).
                i64::from(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)))
            }
            SortCriterion::Random => RANDOM_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|context| i64::from(context.rng.borrow_mut().random(1 << 16)))
                    .unwrap_or(0)
            }),
            SortCriterion::Speed => {
                // C4SortObjectSpeed's C4Fixed sum reaches int32_t through
                // the IMPLICIT `operator bool` (Fixed.h:117, the only
                // conversion C4Fixed offers): the key is 0/1 "moving at
                // all". The fixed squares truncate (val²/65536), so raw
                // |dir| < 256 keys 0 too. Live fixed dirs come from the
                // script-call snapshot; the int-mirror fallback only
                // misses sub-1/256-px movers.
                let (vx, vy) = object
                    .full_state()
                    .and_then(|state| state.script_fixed_velocity)
                    .map(|fixed| (fixed.x.val(), fixed.y.val()))
                    .unwrap_or_else(|| {
                        let velocity = object.velocity();
                        (
                            velocity.x.wrapping_mul(1 << 16),
                            velocity.y.wrapping_mul(1 << 16),
                        )
                    });
                let square = |v: i32| ((i64::from(v) * i64::from(v)) / (1 << 16)) as i32;
                i64::from(square(vx).wrapping_add(square(vy)) != 0)
            }
            SortCriterion::Mass => {
                // pFor->Mass is the LIVE UpdateMass field, including
                // contents unless NoComponentMass (C4Object.cpp:497-505).
                i64::from(sort_object_mass(world, object.id))
            }
            SortCriterion::Value => i64::from(
                get_value(&[
                    object_reference_value(object.id),
                    Value::Nil,
                    Value::Nil,
                    Value::Int(OWNER_NONE),
                ])?
                .as_c4_int()
                .unwrap_or(0),
            ),
            SortCriterion::Func { name, pars } => {
                let pars = live_find_callback_parameters(pars);
                match call_world_object_function(object.id, name, &pars) {
                    None => 0,
                    Some(result) => i64::from(result?.as_c4_int().unwrap_or(0)),
                }
            }
            SortCriterion::Reverse(_) | SortCriterion::Multiple(_) => 0,
        })
    }

    /// `C4SortObject::SortObjects` (C4FindObject.cpp:784-812): per-criterion
    /// value caches computed in collection order, then a stable sort with
    /// `Compare > 0` ⇒ ascending by value (smallest first).
    fn sort(&self, world: &impl WorldAccessor, ids: &mut [ObjectId]) -> Result<(), RuntimeError> {
        let keys = self.cache_keys(world, ids)?;
        let mut order: Vec<usize> = (0..ids.len()).collect();
        order.sort_by(|&a, &b| Self::compare_keys(&keys[a], &keys[b]));
        let sorted: Vec<ObjectId> = order.iter().map(|&index| ids[index]).collect();
        ids.copy_from_slice(&sorted);
        Ok(())
    }

    /// Per-object key vectors: flattened (criterion, direction) values so
    /// Reverse/Multiple compose like the C++ Compare chain.
    fn cache_keys(
        &self,
        world: &impl WorldAccessor,
        ids: &[ObjectId],
    ) -> Result<Vec<Vec<i64>>, RuntimeError> {
        let mut keys = vec![Vec::new(); ids.len()];
        self.fill_keys(world, ids, &mut keys, false)?;
        Ok(keys)
    }

    fn fill_keys(
        &self,
        world: &impl WorldAccessor,
        ids: &[ObjectId],
        keys: &mut [Vec<i64>],
        reverse: bool,
    ) -> Result<(), RuntimeError> {
        match self {
            SortCriterion::Reverse(child) => child.fill_keys(world, ids, keys, !reverse)?,
            SortCriterion::Multiple(children) => {
                for child in children {
                    child.fill_keys(world, ids, keys, reverse)?;
                }
            }
            _ => {
                let sign = if reverse { -1 } else { 1 };
                for (index, id) in ids.iter().enumerate() {
                    let value = match world.get_object(*id) {
                        Some(object) => self.value_for(world, &object)?,
                        None => 0,
                    };
                    keys[index].push(sign * value);
                }
            }
        }
        Ok(())
    }

    fn compare_keys(a: &[i64], b: &[i64]) -> std::cmp::Ordering {
        for (lhs, rhs) in a.iter().zip(b.iter()) {
            match lhs.cmp(rhs) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    }

    /// The UNCACHED `Compare(obj1, obj2)` used by the single-result Find
    /// path (C4FindObject.cpp:834-842): `CompareGetValue` runs for obj1
    /// then obj2 in hardcoded order, returning `value2 - value1` (>0 ⇒
    /// obj1 sorts first). Reverse swaps the arguments
    /// (C4FindObject.cpp:856-859); Multiple returns the first nonzero
    /// child comparison (C4FindObject.cpp:885-895).
    fn compare_uncached(
        &self,
        world: &impl WorldAccessor,
        obj1: &HostWorldObject,
        obj2: &HostWorldObject,
    ) -> Result<i64, RuntimeError> {
        match self {
            SortCriterion::Reverse(child) => child.compare_uncached(world, obj2, obj1),
            SortCriterion::Multiple(children) => {
                for child in children {
                    let result = child.compare_uncached(world, obj1, obj2)?;
                    if result != 0 {
                        return Ok(result);
                    }
                }
                Ok(0)
            }
            _ => {
                let value1 = self.value_for(world, obj1)?;
                let value2 = self.value_for(world, obj2)?;
                Ok(value2 - value1)
            }
        }
    }

    fn compare_uncached_ids(
        &self,
        world: &impl WorldAccessor,
        obj1: ObjectId,
        obj2: ObjectId,
    ) -> Result<i64, RuntimeError> {
        let Some(obj1) = world.get_object(obj1) else {
            return Ok(0);
        };
        let Some(obj2) = world.get_object(obj2) else {
            return Ok(0);
        };
        self.compare_uncached(world, &obj1, &obj2)
    }
}

/// The single-result Find with a sort attached (C4FindObject.cpp:272-308).
/// Bounded criteria walk the per-sector lists: the inner
/// `Find(*pLst)` (C4FindObject.cpp:186-199) keeps a running best PER LIST
/// — replaced when the uncached `Compare(candidate, best)` is positive —
/// and only each list's winner meets the outer running best
/// (C4FindObject.cpp:287-293/299-305). The unbounded walk is the
/// single-list case. No PrepareCache — value functions (and `C4SO_Random`
/// draws) run per comparison, so the pairing is lockstep-relevant. The
/// UseShapes lists carry no Marker: a shape spanning sectors is compared
/// in every list holding it.
fn find_first_with_sort(
    world: &impl WorldAccessor,
    condition: &FindCondition,
    sort: &SortCriterion,
) -> Result<Option<ObjectId>, RuntimeError> {
    if condition.is_impossible(world) {
        return Ok(None);
    }
    let lists = condition
        .bounds()
        .and_then(|(rect, use_shapes)| {
            if use_shapes {
                world.shape_sector_id_lists_in_rect(rect)
            } else {
                world.object_sector_id_lists_in_rect(rect)
            }
        })
        .unwrap_or_else(|| vec![world.object_ids()]);
    let mut best: Option<ObjectId> = None;
    for ids in lists {
        // inner Find(*pLst): the per-list best
        let mut list_best: Option<ObjectId> = None;
        for object_id in ids {
            let Some(object) = world.get_object(object_id) else {
                continue;
            };
            if !object.status().is_active() {
                continue;
            }
            if !condition.check(world, &object)? {
                continue;
            }
            // C4FindObject::Find rechecks Status after Check. Inactive is
            // nonzero in C++ and therefore still eligible here; only a
            // completed AssignRemoval is rejected.
            if !object_present_after_callback(world, object_id) {
                continue;
            }
            list_best = match list_best {
                None => Some(object_id),
                Some(best_id) => {
                    if sort.compare_uncached_ids(world, object_id, best_id)? > 0
                        && object_present_after_callback(world, object_id)
                    {
                        Some(object_id)
                    } else {
                        Some(best_id)
                    }
                }
            };
        }
        // outer walk: the list winner vs the running best
        if let Some(list_id) = list_best {
            best = match best {
                None => object_present_after_callback(world, list_id).then_some(list_id),
                Some(best_id) => {
                    if sort.compare_uncached_ids(world, list_id, best_id)? > 0
                        && object_present_after_callback(world, list_id)
                    {
                        Some(list_id)
                    } else {
                        Some(best_id)
                    }
                }
            };
        }
    }
    Ok(best)
}

/// `CreateCriterionsFromPars` (C4Script.cpp:1985-2034): each argument array
/// parses as a condition or sort; conditions AND together, sorts merge into
/// a Multiple; no conditions at all is a script error.
fn parse_criterions(args: &[Value]) -> Option<(FindCondition, Option<SortCriterion>)> {
    let mut conditions = Vec::new();
    let mut sorts = Vec::new();
    for arg in args.iter().take(10) {
        // The first raw-falsy parameter ends the criteria list
        // (`if (!Data) break;`, C4Script.cpp:1996).
        if !arg.as_bool() {
            break;
        }
        match FindCondition::parse(arg) {
            ParsedCriterion::Condition(condition) => conditions.push(condition),
            ParsedCriterion::Sort(sort) => sorts.push(sort),
            ParsedCriterion::None => {}
        }
    }
    if conditions.is_empty() {
        return None;
    }
    let condition = if conditions.len() == 1 {
        conditions.into_iter().next().expect("one condition")
    } else {
        FindCondition::And(conditions)
    };
    let sort = match sorts.len() {
        0 => None,
        1 => sorts.into_iter().next(),
        _ => Some(SortCriterion::Multiple(sorts)),
    };
    Some((condition, sort))
}

/// FindObject2/ObjectCount2 declare all ten native parameters as C4V_Array.
/// C4Aul validates every slot before CreateCriterionsFromPars scans for its
/// first falsy terminator; pre-strict-3 callers first normalize falsy values
/// to nil, while strict-3 callers retain their concrete types.
fn validate_array_criterion_args(function: &str, args: &[Value]) -> Result<(), RuntimeError> {
    let strict_nil = matches!(
        clonk_script::caller_origin_strictness(),
        clonk_script::HostCallerStrictness::Strict(level) if level >= 3
    );
    for (index, arg) in args.iter().take(10).enumerate() {
        let canonical_nil = matches!(arg, Value::Nil | Value::Object(0))
            || matches!(arg, Value::C4Id(id) if cast_c4id_payload(id) == 0);
        let converted = if canonical_nil || (!strict_nil && !arg.as_bool()) {
            Value::Nil.convert_to(clonk_script::C4VType::Array, true)
        } else {
            arg.convert_to(clonk_script::C4VType::Array, true)
        };
        if !converted {
            return Err(RuntimeError::new(format!(
                "call to \"{function}\" parameter {}: got \"{}\", but expected \"array\"!",
                index + 1,
                arg.type_name()
            )));
        }
    }
    Ok(())
}

/// The C++ bounded-vs-full walk decision (C4FindObject.cpp:315-328):
/// criteria bounds pick the sector walk — the ObjectShapes lists (with
/// first-encounter dedup standing in for the Marker) when the criteria use
/// shapes, the per-sector point lists otherwise. Boundless criteria — or a
/// world without a sector map (legacy fixture contexts) — walk the master
/// list.
fn find_candidate_ids(world: &impl WorldAccessor, condition: &FindCondition) -> Vec<ObjectId> {
    condition
        .bounds()
        .and_then(|(rect, use_shapes)| {
            if use_shapes {
                world.shape_sector_ids_in_rect(rect)
            } else {
                world.object_sector_ids_in_rect(rect)
            }
        })
        // Unbounded criteria walk `Objs.First -> Next`, the forward master
        // list (C4FindObject.cpp:188-216), not the callback's storage order.
        .unwrap_or_else(|| world.master_object_ids())
}

/// Collect matches in C++ walk order (C4FindObject::FindMany,
/// C4FindObject.cpp:203-226 unbounded, :310-355 sector-bounded).
fn find_condition_matches(
    world: &impl WorldAccessor,
    condition: &FindCondition,
) -> Result<Vec<ObjectId>, RuntimeError> {
    if condition.is_impossible(world) {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    if !condition.uses_func() {
        let candidates = find_candidate_ids(world, condition);
        for object_id in candidates {
            if world.matches_find_condition_candidate(object_id, condition) == Some(true) {
                matches.push(object_id);
            }
        }
        return Ok(matches);
    }
    let candidates = find_candidate_ids(world, condition);
    for object_id in candidates {
        match world.matches_find_condition_scalar_prefix(object_id, condition) {
            Some(false) => continue,
            Some(true) => {
                matches.push(object_id);
                continue;
            }
            None => {}
        }
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !object.status().is_active() {
            continue;
        }
        if condition.check(world, &object)? {
            matches.push(object_id);
        }
    }
    Ok(matches)
}

/// Unsorted `C4FindObject::Find` stops immediately after the first matching
/// object whose Status survived its callback. This is observably different
/// from collecting a FindMany result when later predicates have side effects.
fn find_first_condition_match(
    world: &impl WorldAccessor,
    condition: &FindCondition,
) -> Result<Option<ObjectId>, RuntimeError> {
    if condition.is_impossible(world) {
        return Ok(None);
    }
    if !condition.uses_func() {
        let candidates = find_candidate_ids(world, condition);
        let result = candidates.into_iter().find(|object_id| {
            world.matches_find_condition_candidate(*object_id, condition) == Some(true)
        });
        return Ok(result);
    }
    let candidates = find_candidate_ids(world, condition);
    for object_id in candidates {
        match world.matches_find_condition_scalar_prefix(object_id, condition) {
            Some(false) => continue,
            Some(true) => return Ok(Some(object_id)),
            None => {}
        }
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if !object.status().is_active() {
            continue;
        }
        if condition.check(world, &object)? && object_present_after_callback(world, object_id) {
            return Ok(Some(object_id));
        }
    }
    Ok(None)
}

/// Whether the criteria need the reentrant live-view evaluation path.
fn criterions_use_func(condition: &FindCondition, sort: Option<&SortCriterion>) -> bool {
    condition.uses_func() || sort.map(SortCriterion::uses_func).unwrap_or(false)
}

/// FnFindObject2 (C4Script.cpp:2052-2067).
pub(crate) fn find_object2(args: &[Value]) -> Result<Value, RuntimeError> {
    validate_array_criterion_args("FindObject2", args)?;
    let Some((condition, sort)) = parse_criterions(args) else {
        return Err(RuntimeError::new(
            "FindObject: No valid search criterions supplied!",
        ));
    };
    if criterions_use_func(&condition, sort.as_ref()) {
        let Some(view) = LiveFuncFindView::new() else {
            return Ok(Value::Nil);
        };
        let condition = condition.pruned(&view);
        if let Some(sort) = sort {
            return Ok(find_first_with_sort(&view, &condition, &sort)?
                .map(object_reference_value)
                .unwrap_or(Value::Nil));
        }
        return Ok(find_first_condition_match(&view, &condition)?
            .map(object_reference_value)
            .unwrap_or(Value::Nil));
    }
    with_host_context(Ok(Value::Nil), |context| {
        let condition = condition.pruned(context);
        if let Some(sort) = sort {
            return Ok(find_first_with_sort(context, &condition, &sort)?
                .map(object_reference_value)
                .unwrap_or(Value::Nil));
        }
        Ok(find_first_condition_match(context, &condition)?
            .map(object_reference_value)
            .unwrap_or(Value::Nil))
    })
}

/// FnFindObjects array form (C4Script.cpp:2069-2084).
pub(crate) fn find_objects2(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some((condition, sort)) = parse_criterions(args) else {
        return Err(RuntimeError::new(
            "FindObjects: No valid search criterions supplied!",
        ));
    };
    if criterions_use_func(&condition, sort.as_ref()) {
        let Some(view) = LiveFuncFindView::new() else {
            return Ok(Value::Array(Vec::new()));
        };
        let condition = condition.pruned(&view);
        let mut matches = find_condition_matches(&view, &condition)?;
        // Pre-sort: erase objects deleted during Check
        // (C4FindObject.cpp:217-218).
        retain_present_after_callback(&view, &mut matches);
        if let Some(sort) = sort {
            sort.sort(&view, &mut matches)?;
            // Post-sort: objects deleted by sort callbacks keep their slot
            // as nil (CheckObjectStatusAfterSort, C4FindObject.cpp:223,
            // 372-375). Inactive remains a non-null object.
            return Ok(Value::Array(
                matches
                    .into_iter()
                    .map(|id| {
                        if object_present_after_callback(&view, id) {
                            object_reference_value(id)
                        } else {
                            Value::Nil
                        }
                    })
                    .collect(),
            ));
        }
        return Ok(Value::Array(
            matches.into_iter().map(object_reference_value).collect(),
        ));
    }
    with_host_context(Ok(Value::Array(Vec::new())), |context| {
        let condition = condition.pruned(context);
        let mut matches = find_condition_matches(context, &condition)?;
        if let Some(sort) = sort {
            sort.sort(context, &mut matches)?;
        }
        Ok(Value::Array(
            matches.into_iter().map(object_reference_value).collect(),
        ))
    })
}

/// FnObjectCount2 (C4Script.cpp:2036-2050).
pub(crate) fn object_count2(args: &[Value]) -> Result<Value, RuntimeError> {
    validate_array_criterion_args("ObjectCount2", args)?;
    let Some((condition, _)) = parse_criterions(args) else {
        return Err(RuntimeError::new(
            "ObjectCount: No valid search criterions supplied!",
        ));
    };
    if criterions_use_func(&condition, None) {
        let Some(view) = LiveFuncFindView::new() else {
            return Ok(Value::Int(0));
        };
        let condition = condition.pruned(&view);
        if condition.is_ensured(&view) {
            let count = view
                .master_object_ids()
                .into_iter()
                .filter(|id| {
                    view.get_object(*id)
                        .is_some_and(|object| object.status().is_active())
                })
                .count();
            return Ok(Value::Int(truncate_to_i32(count as u64)));
        }
        let matches = find_condition_matches(&view, &condition)?;
        return Ok(Value::Int(truncate_to_i32(matches.len() as u64)));
    }
    with_host_context(Ok(Value::Int(0)), |context| {
        let condition = condition.pruned(context);
        if condition.is_ensured(context) {
            let count = context
                .master_object_ids()
                .into_iter()
                .filter(|id| {
                    context
                        .get_world_object(*id)
                        .is_some_and(|object| object.status().is_active())
                })
                .count();
            return Ok(Value::Int(truncate_to_i32(count as u64)));
        }
        Ok(Value::Int(truncate_to_i32(
            find_condition_matches(context, &condition)?.len() as u64,
        )))
    })
}

pub(crate) fn find_object(args: &[Value]) -> Result<Value, RuntimeError> {
    find_object_cpp(args, "FindObject", None)
}

/// FnFindBase/C4Game::FindBase (C4Script.cpp:1976-1979;
/// C4Game.cpp:3732-3744): validate the player, then walk the active object
/// master list and return the indexed object whose stored Base matches.
pub(crate) fn find_base(args: &[Value]) -> Result<Value, RuntimeError> {
    let player = value_to_i32(args.first().unwrap_or(&Value::Nil), "FindBase", "player")?;
    let mut index = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "FindBase", "index")?;
    with_host_context(Ok(Value::Nil), |context| {
        if context.player_state(player).is_none() || index < 0 {
            return Ok(Value::Nil);
        }
        for object_id in context.master_object_ids() {
            let Some(object) = context.get_world_object(object_id) else {
                continue;
            };
            if !object.status().is_active()
                || object.full_state().map(|state| state.base) != Some(player)
            {
                continue;
            }
            if index == 0 {
                return Ok(object_reference_value(object_id));
            }
            index -= 1;
        }
        Ok(Value::Nil)
    })
}

/// Shared FnFindObject search (C4Script.cpp:2113-2135) with an optional
/// owner filter injected by FindObjectOwner (C4Script.cpp:2137-2161).
fn find_object_cpp(
    args: &[Value],
    function: &str,
    owner_override: Option<i32>,
) -> Result<Value, RuntimeError> {
    with_host_context(Ok(Value::Nil), |context| {
        let mut params = FindObjectParams::parse_cpp_call(args, function, context.caller_scope())?;
        if let Some(owner) = owner_override {
            params.owner = owner;
        }
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

pub(crate) fn find_object_owner(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnFindObjectOwner (C4Script.cpp:2137-2161): FindObject with the
    // owner filter as the SECOND parameter; an owner that is neither a
    // valid player nor NO_OWNER returns nil before any search. The
    // remaining arguments shift by one; exclude/container are not
    // script-settable here (C++ passes caller-exclusion and null).
    if args.len() > 10 {
        return Err(RuntimeError::new(
            "FindObjectOwner: expected at most 10 arguments",
        ));
    }
    let definition = parse_native_c4id_argument(args.first(), "FindObjectOwner")?;
    let owner = parse_optional_i32(args.get(1), "FindObjectOwner", "owner")?.unwrap_or(0);
    let owner_valid = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        borrow
            .as_ref()
            .map(|context| owner == OWNER_NONE || context.player_state(owner).is_some())
            .unwrap_or(false)
    });
    if !owner_valid {
        return Ok(Value::Nil);
    }
    let mut remapped: Vec<Value> = Vec::with_capacity(10);
    remapped.push(definition.map(Value::C4Id).unwrap_or(Value::Nil)); // id
    for slot in 2..=5 {
        remapped.push(args.get(slot).cloned().unwrap_or(Value::Nil)); // x y wdt hgt
    }
    remapped.push(args.get(6).cloned().unwrap_or(Value::Nil)); // ocf
    remapped.push(args.get(7).cloned().unwrap_or(Value::Nil)); // action
    remapped.push(args.get(8).cloned().unwrap_or(Value::Nil)); // action target
    remapped.push(Value::Nil); // container (not script-settable here)
    remapped.push(args.get(9).cloned().unwrap_or(Value::Nil)); // find next
    find_object_cpp(&remapped, "FindObjectOwner", Some(owner))
}

/// C4Game::FindObject (C4Game.cpp:1334-1424): the legacy single-result
/// search scans the MASTER list for every query form — first master-order
/// match wins; sectors are never consulted (unlike the criteria form).
/// The edit cursor's hit test, outside the script host.
///
/// `C4EditCursor::Move` picks its target with
/// `Game.FindObject(0, X, Y, 0, 0, OCF_NotContained, nullptr, nullptr, nullptr,
/// nullptr, ANY_OWNER, Target)` (`C4EditCursor.cpp:150`) — the same
/// `Game.FindObject` script content calls, so it runs through the same query
/// rather than a second hit test that could disagree with it.
///
/// It is a free function over a snapshot rather than an `Engine` method
/// because `find_object_linear` needs a `WorldAccessor`, and a snapshot-backed
/// `HostWorldContext` is one without entering the script host — the console
/// hit-tests between ticks, not during a script call. Callers that walk a
/// stack with `developer_cursor::edit_target` should build the context once
/// and reuse it; a snapshot per click is wasteful, not wrong.
pub(crate) fn edit_cursor_object_at(
    world: &crate::compat::world::HostWorldContext,
    x: i32,
    y: i32,
    after: Option<ObjectId>,
) -> Option<ObjectId> {
    let params = FindObjectParams {
        definition: None,
        x,
        y,
        // A zero-extent query is C++'s point test, not an empty one.
        width: 0,
        height: 0,
        ocf_mask: crate::ocf::NOT_CONTAINED,
        action: None,
        treat_idle: false,
        action_target: None,
        // The console has no caller object to exclude.
        exclude: None,
        container: ContainerFilter::Any,
        owner: OWNER_ANY,
        find_next: after,
    };
    find_object_linear(world, &params)
}

fn find_object_linear(world: &impl WorldAccessor, params: &FindObjectParams) -> Option<ObjectId> {
    let mut skip_until = params.find_next;
    for object_id in world.master_object_ids() {
        if let Some(target) = skip_until {
            if object_id == target {
                skip_until = None;
            }
            continue;
        }
        if params.excludes_id(object_id) {
            continue;
        }
        if !world
            .matches_legacy_find_object_candidate(object_id, params)
            .unwrap_or(false)
        {
            continue;
        }
        if params.is_full_range() {
            return Some(object_id);
        }
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
        if params.matches_area(world, &object) {
            return Some(object_id);
        }
    }
    None
}

fn find_object_closest(world: &impl WorldAccessor, params: &FindObjectParams) -> Option<ObjectId> {
    let reference = params.reference_distance(world);
    let mut best: Option<(ObjectId, i64)> = None;
    for object_id in world.master_object_ids() {
        if params.excludes_id(object_id) {
            continue;
        }
        if !world
            .matches_legacy_find_object_candidate(object_id, params)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
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

/// C++ `FindObjects` is the array-criteria form (C4Script.cpp:7043); the
/// legacy fixed-parameter form predates it in this port and is kept for the
/// existing fixtures. Array first argument → C++ semantics.
pub(crate) fn find_objects_dispatch(args: &[Value]) -> Result<Value, RuntimeError> {
    if matches!(args.first(), Some(Value::Array(_))) {
        find_objects2(args)
    } else {
        find_objects(args)
    }
}

/// `System.c4g/FindObject.c`'s `Find_AtPoint` wrapper: its coordinates are
/// relative to the calling object, or to the world origin in global context.
pub(crate) fn find_at_point(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "Find_AtPoint", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Find_AtPoint", "y")?;
    let origin = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| {
                // System.c4g adds FnGetX/FnGetY's cthr->Obj position. An
                // effect's mutable pForObj carrier is not that receiver.
                let target = context.script_object_context?;
                context
                    .object_scope(target)
                    .map(ObjectScopeContext::effective_position)
                    .or_else(|| {
                        context
                            .get_world_object(target)
                            .map(|object| object.position())
                    })
            })
            .unwrap_or(Vector2::ZERO)
    });
    Ok(Value::Array(vec![
        Value::Int(11),
        Value::Int(origin.x.wrapping_add(x)),
        Value::Int(origin.y.wrapping_add(y)),
    ]))
}

/// `System.c4g/FindObject.c`'s `Find_Category` wrapper: construct the
/// two-cell C4FO_Category criterion consumed by `FindObjects`.
pub(crate) fn find_category(args: &[Value]) -> Result<Value, RuntimeError> {
    let category = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "Find_Category",
        "category",
    )?;
    Ok(Value::Array(vec![Value::Int(22), Value::Int(category)]))
}

/// `System.c4g/FindObject.c`'s `Find_ID` wrapper: preserve the typed C4ID in
/// the two-cell criterion consumed by `FindObject2`/`FindObjects`.
pub(crate) fn find_id(args: &[Value]) -> Result<Value, RuntimeError> {
    let id = parse_native_c4id_argument(args.first(), "Find_ID")?
        .map(Value::C4Id)
        .unwrap_or(Value::Nil);
    Ok(Value::Array(vec![Value::Int(20), id]))
}

pub(crate) fn find_objects(args: &[Value]) -> Result<Value, RuntimeError> {
    let params = FindObjectParams::parse(args)?;
    with_host_context(Ok(Value::Array(Vec::new())), |context| {
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

pub(crate) fn object_count(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnObjectCount (C4Script.cpp:2085-2111): the FindObject layout with
    // iOwner instead of pFindNext as the 10th parameter; an owner of 0
    // becomes ANY_OWNER ("incomplete useless implementation").
    with_host_context(Ok(Value::Int(0)), |context| {
        let mut params = FindObjectParams::parse_cpp_call(
            &args[..args.len().min(9)],
            "ObjectCount",
            context.caller_scope(),
        )?;
        let owner = parse_optional_i32(args.get(9), "ObjectCount", "owner")?.unwrap_or(0);
        params.owner = if owner == 0 { OWNER_ANY } else { owner };
        let matches_len = if params.is_closest_query() {
            collect_closest_matches(context, &params).len()
        } else {
            collect_linear_matches(context, &params).len()
        };
        Ok(Value::Int(truncate_to_i32(matches_len as u64)))
    })
}

/// FnObjectNumber (C4Script.cpp:3321-3325): return C4Object::Number, defaulting
/// a nil/omitted object to the current script object. Number is the immutable
/// C4Game::ObjectEnumerationIndex allocation, not an index into either object
/// list; therefore object status and list membership do not affect this read.
pub(crate) fn object_number(args: &[Value]) -> Result<Value, RuntimeError> {
    let explicit = match args.first() {
        Some(Value::Object(0)) | Some(Value::Nil) | None => None,
        Some(Value::Object(number)) => Some(*number),
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "ObjectNumber: expected object or nil for object, got {}",
                other.type_name()
            )));
        }
    };
    let number = explicit.or_else(|| {
        HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(EffectHostContext::object_context)
                .map(|object| object.id().as_u64())
        })
    });

    Ok(number.map_or(Value::Nil, |number| Value::Int(truncate_to_i32(number))))
}

/// FnObject (C4Script.cpp:3327-3330): resolve an exact saved object Number.
/// SafeObjectPointer rejects only deleted status; inactive objects remain
/// addressable (C4ObjectList.cpp:544-557, C4GameObjects.cpp:270-276).
pub(crate) fn object_by_number(args: &[Value]) -> Result<Value, RuntimeError> {
    let number = value_to_i32(args.first().unwrap_or(&Value::Nil), "Object", "number")?;
    let Some(id) = (number > 0).then(|| ObjectId::new(number as u64)) else {
        return Ok(Value::Nil);
    };

    with_host_context(Ok(Value::Nil), |context| {
        let status = context
            .get_world_object(id)
            .map(|object| object.status())
            .or_else(|| {
                context.object_scope(id).map(|scope| {
                    if scope.destroy {
                        ObjectStatus::Deleted
                    } else {
                        scope.status()
                    }
                })
            });
        Ok(
            if status.is_some_and(|status| status != ObjectStatus::Deleted) {
                object_reference_value(id)
            } else {
                Value::Nil
            },
        )
    })
}

fn collect_linear_matches(world: &impl WorldAccessor, params: &FindObjectParams) -> Vec<ObjectId> {
    let mut matches = Vec::new();
    let mut skip_until = params.find_next;
    for object_id in params.candidate_ids(world) {
        if let Some(target) = skip_until {
            if object_id == target {
                skip_until = None;
            }
            continue;
        }
        if !world
            .matches_legacy_find_object_candidate(object_id, params)
            .unwrap_or(false)
        {
            continue;
        }
        if params.is_full_range() {
            matches.push(object_id);
            continue;
        }
        if world
            .get_object(object_id)
            .is_some_and(|object| params.matches_area(world, &object))
        {
            matches.push(object_id);
        }
    }
    matches
}

fn collect_closest_matches(world: &impl WorldAccessor, params: &FindObjectParams) -> Vec<ObjectId> {
    let reference = params.reference_distance(world);
    let mut matches = Vec::new();
    for (order_index, object_id) in params.candidate_ids(world).into_iter().enumerate() {
        if !world
            .matches_legacy_find_object_candidate(object_id, params)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(object) = world.get_object(object_id) else {
            continue;
        };
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

pub(crate) fn get_id(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetID expects at most 1 argument: target object",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "GetID", "target")?;
    }
    if target_id.is_none() {
        if let Some((definition, _)) = fair_crew_definition_context() {
            return Ok(Value::C4Id(definition.as_str().to_string()));
        }
    }

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if context.get_world_object(target).is_some() {
                return Ok(context
                    .object_effective_definition_id(target)
                    .map(|id| Value::C4Id(id.as_str().to_string()))
                    .unwrap_or(Value::Nil));
            }
            // If target object not found, return nil
            return Ok(Value::Nil);
        }

        // No argument: cthr->Obj->Def when there is an object, otherwise
        // cthr->Def from the executing function owner.
        if let Some(definition_id) = context.current_definition_id() {
            return Ok(Value::C4Id(definition_id.to_string()));
        }

        Ok(Value::Nil)
    })
}

/// FnGetBase (C4Script.cpp:1406-1410): read C4Object::Base from the
/// explicit object, or from the calling object when the argument is nil;
/// without either object C4ValueInt returns NO_OWNER.
pub(crate) fn get_base(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetBase expects at most 1 argument: target object",
        ));
    }
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "GetBase", "target"))
        .transpose()?
        .flatten();

    with_host_context(Ok(Value::Int(OWNER_NONE)), |context| {
        let target = target.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let base = target
            .and_then(|id| context.get_world_object(id))
            .and_then(|object| object.full_state().map(|state| state.base))
            .unwrap_or(OWNER_NONE);
        Ok(Value::Int(base))
    })
}

pub(crate) fn create_object(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnCreateObject takes a C4ID (C4Script.cpp:1892); our resources address
    // definitions by their id string, so id values and strings coincide.
    let Some(definition) = parse_native_c4id_argument(args.first(), "CreateObject")? else {
        return Ok(Value::Nil);
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

    let requested_owner = if let Some(arg) = args.get(index) {
        let owner = value_to_i32(arg, "CreateObject", "owner")?;
        index += 1;
        owner
    } else {
        0
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "CreateObject: additional arguments are not supported",
        ));
    }

    let registration = try_with_host_context_mut(
        "CreateObject requires an active engine context",
        |context| {
            // C4Id2Def failure: no object, silent nullptr (C4Game.cpp:1146).
            if context.world.definition_known(&definition) == Some(false) {
                return Ok(None);
            }

            let metadata = context
                .definition_metadata(&definition)
                .cloned()
                .unwrap_or_else(|| DefinitionMetadata {
                    category: context
                        .definition_category(&definition)
                        .unwrap_or(DEFAULT_CATEGORY),
                    ocf_base: ocf::NORMAL,
                    ..Default::default()
                });
            let definition_category = metadata.category;
            let creator = context.object_context().map(ObjectScopeContext::id);
            let creator_layer = creator.and_then(|creator| context.object_layer(creator));
            let creator_layer_cache = creator
                .map(|creator| context.object_layer_compiler_cache(creator))
                .unwrap_or(0);

            let base_position = context
                .object_context()
                .map(|object| object.effective_position())
                .unwrap_or(Vector2::ZERO);
            // Typed C4ValueInt conversion makes an omitted/explicit nil owner
            // zero. Local calls replace even an explicit owner when the native
            // has no script caller or its immediate caller is NONSTRICT.
            let substitute_local_owner = matches!(
                clonk_script::caller_strictness(),
                clonk_script::HostCallerStrictness::NoCaller
                    | clonk_script::HostCallerStrictness::NonStrict
            );
            let owner = if substitute_local_owner {
                context
                    .object_context()
                    .map(ObjectScopeContext::owner)
                    .unwrap_or(requested_owner)
            } else {
                requested_owner
            };
            let raw_position = Vector2::new(
                base_position.x.saturating_add(x_offset),
                base_position.y.saturating_add(y_offset),
            );

            let id = context.allocate_object_id();

            let mut spawn = SpawnConfig::new(definition.clone())
                .with_position(raw_position)
                .with_owner(owner)
                .with_category(definition_category)
                // C4Object starts at Con=0. C4Game::NewObject exposes that live
                // state to Construction before applying the initial FullCon.
                .with_construction(0)
                .with_id(id);
            if let Some(layer) = creator_layer {
                spawn = spawn.with_layer(layer);
            }
            spawn.compiler_cache.layer = creator_layer_cache;
            // "Set initial controller to creating controller, so more
            // complicated cause-effect-chains can be traced back to the
            // causing player" (FnCreateObject, C4Script.cpp:1899-1900).
            let creator_controller = context
                .object_context()
                .map(ObjectScopeContext::controller)
                .filter(|value| *value > OWNER_NONE);
            if let Some(controller) = creator_controller {
                spawn = spawn.with_controller(controller);
            }
            // Creation callbacks and initial DoCon run synchronously below;
            // materialization must repeat neither operation.
            spawn.initialized = true;
            spawn.position_adjusted = true;

            let initial_alive = metadata.category & crate::CATEGORY_LIVING != 0;
            let preview_ocf = ocf::compute(
                metadata.ocf_base,
                metadata.crew_member,
                initial_alive,
                ObjectStatus::Normal,
                false,
                0,
                metadata.category,
            );
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
                if initial_alive {
                    metadata.physical.energy
                } else {
                    0
                },
                0,
                0,
                raw_position,
                Vector2::ZERO,
                0,
                metadata.vertices.clone(),
                0,
                0,
                0,
                None,
            )
            .with_compiler_fields(
                0,
                0,
                -1,
                crate::ObjectCompilerCache {
                    layer: creator_layer_cache,
                    ..crate::ObjectCompilerCache::default()
                },
            )
            .with_alive(initial_alive)
            .with_ocf(preview_ocf)
            // A callable scope for nested calls on the fresh object — C++
            // creates objects live mid-call (Game.CreateObject), so scripts
            // arrow-call them immediately (GoldRush: pObj->SetAI right after
            // CreateObject). The spawn stays authoritative; nested outcomes
            // fold only touched fields.
            .with_full_state(Rc::new({
                let mut state = crate::preview_spawn_state_with_components(
                    raw_position,
                    owner,
                    owner,
                    definition_category,
                    0,
                    metadata.contact_density(),
                    metadata.vertices.clone(),
                    metadata.components.as_slice(),
                );
                state.alive = initial_alive;
                state.energy = if initial_alive {
                    metadata.physical.energy
                } else {
                    0
                };
                state.crew_member = metadata.crew_member;
                state.layer = creator_layer;
                state.blit_mode = metadata.blit_mode;
                state
            }));

            context.register_spawn(spawn, preview);
            Ok(Some((
                id,
                context.object_context().map(ObjectScopeContext::id),
                creator_controller,
                metadata.shape,
                metadata.stretch_growth,
                metadata.line,
                metadata.ocf_base,
                metadata.crew_member,
                metadata.category,
                initial_alive,
            )))
        },
    )?;
    let Some((
        target,
        creator,
        creator_controller,
        shape,
        stretch_growth,
        line,
        ocf_base,
        crew_member,
        category,
        alive,
    )) = registration
    else {
        return Ok(Value::Nil);
    };

    // C4Game::NewObject makes the raw Con=0 object script-visible, then
    // invokes Construction with the creator (C4Game.cpp:1102-1121).
    let creator_arg = creator.map(object_reference_value).unwrap_or(Value::Nil);
    if let Some(Err(error)) = call_world_object_own_function(target, "Construction", &[creator_arg])
    {
        tracing::error!(
            id = target.as_u64(),
            callback = "Construction",
            %error,
            "creation callback failed; continuing like C++ fail-safe Call"
        );
        log_runtime_call_frames("", error.call_frames());
    }
    let removed = with_host_context(false, |context| context.nested_object_destroyed(target));
    if removed {
        return Ok(Value::Nil);
    }

    // Construction ran against the raw Con=0 object, before NewObject's
    // initial DoCon. Commit its final action state into the pending spawn at
    // that boundary instead of leaving the ActionUpdate to replay after the
    // spawn has materialized at its post-growth integer position. SetAction
    // synchronizes fix_x/fix_y immediately in C++; replaying it later changed
    // WMPF's raw y=516 fixed coordinate to the intermediate growth y=512.
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.commit_creation_action(target);
        }
    });

    // Initial DoCon(FullCon,true) runs only after Construction. Its straight
    // growth keeps the old bottom fixed in integer coordinates and leaves
    // fix_y at the supplied raw center (C4Object.cpp:1428-1515).
    let crossed_full_con = with_host_context_mut(false, |context| {
        if !context.ensure_object_scope(target) {
            return false;
        }
        let was_full = context
            .object_scope(target)
            .is_some_and(|scope| scope.construction() >= FULL_CON);
        let Some(final_construction) = context.adjust_object_construction(target, FULL_CON) else {
            return false;
        };
        let (pre_growth_position, adjusted_position) = {
            let Some(scope) = context.object_scope_mut(target) else {
                return false;
            };
            let pre_growth_position = scope.effective_position();
            let adjusted_position = Vector2::new(
                pre_growth_position.x,
                crate::docon_initial_center_y(
                    shape,
                    stretch_growth,
                    line,
                    final_construction,
                    pre_growth_position.y,
                ),
            );
            scope.pending_update.construction = None;
            scope.current_position = adjusted_position;
            scope.pending_update.position = None;
            scope.cached_ocf = Some(ocf::compute(
                ocf_base,
                crew_member,
                alive,
                ObjectStatus::Normal,
                false,
                final_construction,
                category,
            ));
            (pre_growth_position, adjusted_position)
        };
        if let Some(spawn) = context
            .pending_spawns
            .iter_mut()
            .find(|spawn| spawn.id == Some(target))
        {
            spawn.position = adjusted_position;
            spawn.construction = FULL_CON;
            spawn.fixed_position = (adjusted_position != pre_growth_position).then_some(
                FixedVec2::from_ints(pre_growth_position.x, pre_growth_position.y),
            );
        }
        context.update_live_solid_mask(target, false);
        !was_full && final_construction >= FULL_CON
    });

    if crossed_full_con {
        for callback in ["Completion", "Initialize"] {
            if let Some(Err(error)) = call_world_object_own_function(target, callback, &[]) {
                tracing::error!(
                    id = target.as_u64(),
                    callback,
                    %error,
                    "creation callback failed; continuing like C++ fail-safe Call"
                );
                log_runtime_call_frames("", error.call_frames());
            }
        }
    }

    let removed = with_host_context_mut(true, |context| {
        if context.nested_object_destroyed(target) {
            return true;
        }
        // FnCreateObject applies the creating controller only after
        // Game.CreateObject (and therefore every lifecycle callback) returns
        // (C4Script.cpp:1886-1902).
        if let Some(controller) = creator_controller {
            if let Some(scope) = context.object_scope_mut(target) {
                scope.set_controller(controller);
            }
            if let Some(preview) = context.pending_objects.get_mut(&target) {
                if let Some(state) = preview.state.as_mut() {
                    Rc::make_mut(state).controller = controller;
                }
            }
        }
        false
    });
    Ok(if removed {
        Value::Nil
    } else {
        object_reference_value(target)
    })
}

pub(crate) fn cast_objects(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnCastObjects -> C4Game::CastObjects (C4Script.cpp:2476-2480,
    // C4Game.cpp:1727-1739): every attempt draws rdir, ydir, xdir and
    // rotation in that order, then creates the object synchronously.
    if args.len() > 5 {
        return Err(RuntimeError::new(
            "CastObjects: additional arguments are not supported",
        ));
    }

    let definition = parse_native_c4id_argument(args.first(), "CastObjects")?;
    let amount = args
        .get(1)
        .map(|value| value_to_i32(value, "CastObjects", "amount"))
        .transpose()?
        .unwrap_or(0);
    let level = args
        .get(2)
        .map(|value| value_to_i32(value, "CastObjects", "level"))
        .transpose()?
        .unwrap_or(0);
    let x_offset = args
        .get(3)
        .map(|value| value_to_i32(value, "CastObjects", "x"))
        .transpose()?
        .unwrap_or(0);
    let y_offset = args
        .get(4)
        .map(|value| value_to_i32(value, "CastObjects", "y"))
        .transpose()?
        .unwrap_or(0);

    let (creator, base_position, owner, controller) =
        try_with_host_context("CastObjects requires an active engine context", |context| {
            let creator = context.object_context().map(ObjectScopeContext::id);
            let base_position = context
                .object_context()
                .map(ObjectScopeContext::effective_position)
                .unwrap_or(Vector2::ZERO);
            let owner = context
                .object_context()
                .map(ObjectScopeContext::owner)
                .unwrap_or(OWNER_NONE);
            let controller = context
                .object_context()
                .map(ObjectScopeContext::controller)
                .unwrap_or(OWNER_NONE);
            Ok((creator, base_position, owner, controller))
        })?;

    let spread = level.wrapping_mul(2).wrapping_add(1);
    for _ in 0..amount {
        // Force the C++ argument-evaluation order. Definition lookup happens
        // only after these draws, so missing ids consume the same ledger.
        let sampled_rdir = itofix(draw_context_random(3)? + 1);
        let ydir = fixed10(draw_context_random(spread)?.wrapping_sub(level));
        let xdir = fixed10(draw_context_random(spread)?.wrapping_sub(level));
        let sampled_rotation = draw_context_random(360)?;

        let Some(definition) = definition.as_ref() else {
            continue;
        };
        let target = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let context = borrow.as_mut().ok_or_else(|| {
                RuntimeError::new("CastObjects requires an active engine context")
            })?;
            if context.world.definition_known(definition) == Some(false) {
                return Ok(None);
            }

            let metadata = context
                .definition_metadata(definition)
                .cloned()
                .unwrap_or_else(|| DefinitionMetadata {
                    category: context
                        .definition_category(definition)
                        .unwrap_or(DEFAULT_CATEGORY),
                    ocf_base: ocf::NORMAL,
                    ..Default::default()
                });
            // C4Object::Init discards sampled rotation/rdir for definitions
            // without Rotateable, after the synced draws already happened.
            let (rotation, rdir) = if metadata.rotateable == 0 {
                (0, C4Fixed::ZERO)
            } else {
                (sampled_rotation, sampled_rdir)
            };
            let fixed_velocity = FixedVec2::new(xdir, ydir);
            let initial_controller = if controller > OWNER_NONE {
                controller
            } else {
                owner
            };
            // Init reads pCreator->pLayer for every object; an earlier
            // synchronous callback may have changed the creator's layer.
            let creator_layer = creator.and_then(|id| context.object_layer(id));
            let creator_layer_cache = creator
                .map(|id| context.object_layer_compiler_cache(id))
                .unwrap_or(0);
            let raw_position = Vector2::new(
                base_position.x.saturating_add(x_offset),
                base_position.y.saturating_add(y_offset),
            );
            let position = Vector2::new(
                raw_position.x,
                crate::docon_initial_center_y(
                    metadata.shape,
                    metadata.stretch_growth,
                    metadata.line,
                    crate::FULL_CON,
                    raw_position.y,
                ),
            );
            let id = context.allocate_object_id();
            let mut spawn = SpawnConfig::new(definition.clone())
                .with_position(position)
                .with_fixed_velocity(fixed_velocity)
                .with_rotation(rotation)
                .with_rotation_velocity(rdir)
                .with_owner(owner)
                .with_controller(controller)
                .with_category(metadata.category)
                .with_id(id);
            if let Some(layer) = creator_layer {
                spawn = spawn.with_layer(layer);
            }
            spawn.compiler_cache.layer = creator_layer_cache;
            // NewObject callbacks run below while this host call is live.
            spawn.initialized = true;
            spawn.position_adjusted = true;
            if position.y != raw_position.y {
                spawn.fixed_position = Some(FixedVec2::from_ints(raw_position.x, raw_position.y));
            }

            let initial_alive = metadata.category & crate::CATEGORY_LIVING != 0;
            let preview_ocf = ocf::compute(
                metadata.ocf_base,
                metadata.crew_member,
                initial_alive,
                ObjectStatus::Normal,
                false,
                0,
                metadata.category,
            );
            let preview_velocity = Vector2::new(fixed_velocity.int_x(), fixed_velocity.int_y());
            let preview = HostWorldObject::with_category(
                id,
                definition.clone(),
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                owner,
                metadata.category,
                if initial_alive {
                    metadata.physical.energy
                } else {
                    0
                },
                0,
                0,
                raw_position,
                preview_velocity,
                rotation,
                metadata.vertices.clone(),
                0,
                0,
                0,
                None,
            )
            .with_compiler_fields(
                0,
                0,
                -1,
                crate::ObjectCompilerCache {
                    layer: creator_layer_cache,
                    ..crate::ObjectCompilerCache::default()
                },
            )
            .with_rotation_velocity(rdir)
            .with_alive(initial_alive)
            .with_ocf(preview_ocf)
            .with_full_state(Rc::new({
                let mut state = crate::preview_spawn_state_with_components(
                    raw_position,
                    owner,
                    initial_controller,
                    metadata.category,
                    0,
                    metadata.contact_density(),
                    metadata.vertices.clone(),
                    metadata.components.as_slice(),
                );
                state.velocity = preview_velocity;
                state.script_fixed_velocity = Some(fixed_velocity);
                state.script_rotation_velocity = Some(rdir);
                state.rotation = rotation;
                state.alive = initial_alive;
                state.energy = if initial_alive {
                    metadata.physical.energy
                } else {
                    0
                };
                state.crew_member = metadata.crew_member;
                state.layer = creator_layer;
                state.blit_mode = metadata.blit_mode;
                state.mobile = metadata.category != crate::CATEGORY_STATIC_BACK
                    && (fixed_velocity != FixedVec2::ZERO || rdir.is_nonzero());
                state
            }));
            context.register_spawn(spawn, preview);
            // Seed exact fixed dirs into the pending object's live scope so
            // its synchronous callbacks observe the Init values.
            if context.ensure_object_scope(id) {
                if let Some(scope) = context.object_scope_mut(id) {
                    scope.current_fixed_velocity = fixed_velocity;
                    scope.pending_update.rotation_velocity = Some(rdir);
                }
            }
            Ok(Some((
                id,
                metadata.shape,
                metadata.stretch_growth,
                metadata.line,
            )))
        })?;
        let Some((target, shape, stretch_growth, line)) = target else {
            continue;
        };

        let creator_arg = creator.map(object_reference_value).unwrap_or(Value::Nil);
        if let Some(Err(error)) =
            call_world_object_own_function(target, "Construction", &[creator_arg])
        {
            tracing::error!(
                id = target.as_u64(),
                callback = "Construction",
                %error,
                "creation callback failed; continuing like C++ fail-safe Call"
            );
            log_runtime_call_frames("", error.call_frames());
        }
        let removed = with_host_context(false, |context| context.nested_object_destroyed(target));
        if removed {
            continue;
        }

        // NewObject calls Construction while Con is still zero, then its
        // initial DoCon grows to FullCon, moves the straight shape's bottom,
        // and only on that transition calls Completion then Initialize
        // (C4Game.cpp:1117-1127; C4Object.cpp:1506-1511).
        let crossed_full_con = with_host_context_mut(false, |context| {
            let was_full = context
                .object_scope(target)
                .is_some_and(|scope| scope.construction() >= FULL_CON);
            let Some(final_construction) = context.adjust_object_construction(target, FULL_CON)
            else {
                return false;
            };
            let (pre_growth_position, adjusted_position) = {
                let Some(scope) = context.object_scope_mut(target) else {
                    return false;
                };
                let pre_growth_position = scope.effective_position();
                let adjusted_position = Vector2::new(
                    pre_growth_position.x,
                    crate::docon_initial_center_y(
                        shape,
                        stretch_growth,
                        line,
                        FULL_CON,
                        pre_growth_position.y,
                    ),
                );
                // Fold the pre-insertion Construction writes into the spawn
                // itself. Completion must see the adjusted integer y, but
                // initial DoCon leaves fix_y at the pre-growth center.
                scope.pending_update.construction = None;
                scope.current_position = adjusted_position;
                scope.pending_update.position = None;
                (pre_growth_position, adjusted_position)
            };
            if let Some(spawn) = context
                .pending_spawns
                .iter_mut()
                .find(|spawn| spawn.id == Some(target))
            {
                spawn.position = adjusted_position;
                spawn.construction = final_construction;
                spawn.fixed_position = (adjusted_position != pre_growth_position).then_some(
                    FixedVec2::from_ints(pre_growth_position.x, pre_growth_position.y),
                );
            }
            context.update_live_solid_mask(target, false);
            !was_full && final_construction >= FULL_CON
        });
        if crossed_full_con {
            if let Some(Err(error)) = call_world_object_own_function(target, "Completion", &[]) {
                tracing::error!(
                    id = target.as_u64(),
                    callback = "Completion",
                    %error,
                    "creation callback failed; continuing like C++ fail-safe Call"
                );
                log_runtime_call_frames("", error.call_frames());
            }
            let removed =
                with_host_context(false, |context| context.nested_object_destroyed(target));
            if !removed {
                if let Some(Err(error)) = call_world_object_own_function(target, "Initialize", &[])
                {
                    tracing::error!(
                        id = target.as_u64(),
                        callback = "Initialize",
                        %error,
                        "creation callback failed; continuing like C++ fail-safe Call"
                    );
                    log_runtime_call_frames("", error.call_frames());
                }
            }
        }
    }

    Ok(Value::Nil)
}

fn placement_find_liquid_height(landscape: &Landscape, x: i32, y: &mut i32, height: i32) -> bool {
    let world_height = landscape.estimated_height();
    let (mut cy1, mut cy2) = (*y, *y);
    let (mut rl1, mut rl2) = (0, 0);
    while cy1 >= 0 || cy2 < world_height {
        if cy1 >= 0 {
            if landscape.is_liquid_at(x, cy1) {
                rl1 += 1;
                if rl1 >= height {
                    *y = cy1 + height / 2;
                    return true;
                }
            } else {
                rl1 = 0;
            }
        }
        if cy2 + 1 < world_height {
            if landscape.is_liquid_at(x, cy2) {
                rl2 += 1;
                if rl2 >= height {
                    *y = cy2 - height / 2;
                    return true;
                }
            } else {
                rl2 = 0;
            }
        }
        cy1 -= 1;
        cy2 += 1;
    }
    false
}

pub(crate) fn placement_find_surface_liquid(
    landscape: &Landscape,
    x: &mut i32,
    y: &mut i32,
    width: i32,
    height: i32,
) -> bool {
    let world_width = landscape.width() as i32;
    let (mut cx1, mut cx2) = (*x, *x);
    let (mut cy1, mut cy2) = (*y, *y);
    let (mut rl1, mut rl2) = (0, 0);
    let mut found = false;
    while cx1 > 0 || cx2 < world_width {
        if cx1 > 0 {
            match landscape.above_semi_solid(cx1, cy1) {
                Some(adjusted) => {
                    cy1 = adjusted;
                    if (0..height).all(|offset| landscape.is_liquid_at(cx1, cy1 + 1 + offset)) {
                        rl1 += 1;
                    } else {
                        rl1 = 0;
                    }
                }
                None => cx1 = -1,
            }
        }
        if cx2 < world_width {
            match landscape.above_semi_solid(cx2, cy2) {
                Some(adjusted) => {
                    cy2 = adjusted;
                    if (0..height).all(|offset| landscape.is_liquid_at(cx2, cy2 + 1 + offset)) {
                        rl2 += 1;
                    } else {
                        rl2 = 0;
                    }
                }
                None => cx2 = world_width,
            }
        }
        if rl1 >= width {
            *x = cx1 + rl1 / 2;
            *y = cy1;
            found = true;
            break;
        }
        if rl2 >= width {
            *x = cx2 - rl2 / 2;
            *y = cy2;
            found = true;
            break;
        }
        cx1 -= 1;
        cx2 += 1;
    }
    if found {
        if let Some(adjusted) = landscape.above_semi_solid(*x, *y) {
            *y = adjusted;
        }
    }
    found
}

pub(crate) fn placement_find_liquid(
    landscape: &Landscape,
    x: &mut i32,
    y: &mut i32,
    width: i32,
    height: i32,
) -> bool {
    let world_width = landscape.width() as i32;
    let (mut cx1, mut cx2) = (*x, *x);
    let (mut cy1, mut cy2) = (*y, *y);
    let (mut rl1, mut rl2) = (0, 0);
    while cx1 > 0 || cx2 < world_width {
        if cx1 > 0 {
            if placement_find_liquid_height(landscape, cx1, &mut cy1, height) {
                rl1 += 1;
            } else {
                rl1 = 0;
            }
        }
        if cx2 < world_width {
            if placement_find_liquid_height(landscape, cx2, &mut cy2, height) {
                rl2 += 1;
            } else {
                rl2 = 0;
            }
        }
        if rl1 >= width {
            *x = cx1 + rl1 / 2;
            *y = cy1;
            return true;
        }
        if rl2 >= width {
            *x = cx2 - rl2 / 2;
            *y = cy2;
            return true;
        }
        cx1 -= 1;
        cx2 += 1;
    }
    false
}

struct PlacementObjectRegistration {
    target: ObjectId,
    growth: i32,
    shape: Option<DefinitionRect>,
    stretch_growth: bool,
    line: i32,
}

/// Register the zero-construction object that C4Game::NewObject exposes
/// before PSF_Construction. Runtime placement hosts share this exact preview
/// and pending-spawn setup; the callback/DoCon half lives in
/// [`finish_placement_object_creation`].
fn register_placement_object(
    context: &mut EffectHostContext,
    definition: String,
    metadata: DefinitionMetadata,
    position: Vector2,
    growth: i32,
) -> PlacementObjectRegistration {
    let id = context.allocate_object_id();
    let mut spawn = SpawnConfig::new(definition.clone())
        .with_position(position)
        .with_owner(OWNER_NONE)
        .with_controller(OWNER_NONE)
        .with_category(metadata.category)
        .with_construction(0)
        .with_id(id);
    // NewObject's callbacks and initial DoCon run below while this host
    // call is live. Materialization must not repeat either operation.
    spawn.initialized = true;
    spawn.position_adjusted = true;
    let initial_alive = metadata.category & crate::CATEGORY_LIVING != 0;
    let preview_ocf = ocf::compute(
        metadata.ocf_base,
        metadata.crew_member,
        initial_alive,
        ObjectStatus::Normal,
        false,
        0,
        metadata.category,
    );
    let preview = HostWorldObject::with_category(
        id,
        definition,
        ObjectStatus::Normal,
        "Idle",
        None,
        None,
        None,
        OWNER_NONE,
        metadata.category,
        if initial_alive {
            metadata.physical.energy
        } else {
            0
        },
        0,
        0,
        position,
        Vector2::ZERO,
        0,
        metadata.vertices.clone(),
        0,
        0,
        0,
        None,
    )
    .with_alive(initial_alive)
    .with_ocf(preview_ocf)
    .with_full_state(Rc::new({
        let mut state = crate::preview_spawn_state_with_components(
            position,
            OWNER_NONE,
            OWNER_NONE,
            metadata.category,
            0,
            metadata.contact_density(),
            metadata.vertices.clone(),
            metadata.components.as_slice(),
        );
        state.alive = initial_alive;
        state.energy = if initial_alive {
            metadata.physical.energy
        } else {
            0
        };
        state.crew_member = metadata.crew_member;
        state.blit_mode = metadata.blit_mode;
        state
    }));
    context.register_spawn(spawn, preview);
    context.ensure_object_scope(id);
    PlacementObjectRegistration {
        target: id,
        growth: growth.clamp(0, FULL_CON),
        shape: metadata.shape,
        stretch_growth: metadata.stretch_growth,
        line: metadata.line,
    }
}

/// Complete C4Game::NewObject after the object has become script-visible:
/// Construction(nil), initial DoCon, Completion/Initialize on the full-con
/// transition, and nullptr if any callback assigned removal.
fn finish_placement_object_creation(
    registration: PlacementObjectRegistration,
) -> Result<Value, RuntimeError> {
    let PlacementObjectRegistration {
        target,
        growth,
        shape,
        stretch_growth,
        line,
    } = registration;

    if let Some(Err(error)) = call_world_object_own_function(target, "Construction", &[Value::Nil])
    {
        tracing::error!(
            id = target.as_u64(),
            callback = "Construction",
            %error,
            "creation callback failed; continuing like C++ fail-safe Call"
        );
        log_runtime_call_frames("", error.call_frames());
    }
    let removed = with_host_context(false, |context| context.nested_object_destroyed(target));
    if removed {
        return Ok(Value::Nil);
    }

    let crossed_full_con = with_host_context_mut(false, |context| {
        let was_full = context
            .object_scope(target)
            .is_some_and(|scope| scope.construction() >= FULL_CON);
        let Some(final_construction) = context.adjust_object_construction(target, growth) else {
            return false;
        };
        let (pre_growth_position, adjusted_position) = {
            let Some(scope) = context.object_scope_mut(target) else {
                return false;
            };
            let pre_growth_position = scope.effective_position();
            let adjusted_position = Vector2::new(
                pre_growth_position.x,
                crate::docon_initial_center_y(
                    shape,
                    stretch_growth,
                    line,
                    final_construction,
                    pre_growth_position.y,
                ),
            );
            scope.pending_update.construction = None;
            scope.current_position = adjusted_position;
            scope.pending_update.position = None;
            (pre_growth_position, adjusted_position)
        };
        if let Some(spawn) = context
            .pending_spawns
            .iter_mut()
            .find(|spawn| spawn.id == Some(target))
        {
            spawn.position = adjusted_position;
            spawn.construction = final_construction;
            spawn.fixed_position = (adjusted_position != pre_growth_position).then_some(
                FixedVec2::from_ints(pre_growth_position.x, pre_growth_position.y),
            );
        }
        context.update_live_solid_mask(target, false);
        !was_full && final_construction >= FULL_CON
    });
    if crossed_full_con {
        if let Some(Err(error)) = call_world_object_own_function(target, "Completion", &[]) {
            tracing::error!(
                id = target.as_u64(),
                callback = "Completion",
                %error,
                "creation callback failed; continuing like C++ fail-safe Call"
            );
            log_runtime_call_frames("", error.call_frames());
        }
        let removed = with_host_context(false, |context| context.nested_object_destroyed(target));
        if !removed {
            if let Some(Err(error)) = call_world_object_own_function(target, "Initialize", &[]) {
                tracing::error!(
                    id = target.as_u64(),
                    callback = "Initialize",
                    %error,
                    "creation callback failed; continuing like C++ fail-safe Call"
                );
                log_runtime_call_frames("", error.call_frames());
            }
        }
    }

    let removed = with_host_context(false, |context| context.nested_object_destroyed(target));
    Ok(if removed {
        Value::Nil
    } else {
        object_reference_value(target)
    })
}

/// FnPlaceAnimal -> C4Game::PlaceAnimal (C4Script.cpp:2495-2499;
/// C4Game.cpp:3028-3061): placement is GLOBAL and creatorless. Definition
/// validation and the Placement switch happen before any synced draw; the
/// three supported arms then use the exact C++ search order.
pub(crate) fn place_animal(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(definition) = parse_native_c4id_argument(args.first(), "PlaceAnimal")? else {
        return Ok(Value::Nil);
    };

    let registration =
        try_with_host_context_mut("PlaceAnimal requires an active engine context", |context| {
            let Some(metadata) = context.definition_metadata(&definition).cloned() else {
                // C4Id2Def failure precedes the Placement switch and Random
                // (C4Game.cpp:3028-3035).
                return Ok(None);
            };
            if !matches!(metadata.placement, 0..=2) {
                return Ok(None);
            }
            let (shape_width, shape_height) = metadata
                .shape
                .map(|shape| (shape.width, shape.height))
                .unwrap_or((0, 0));
            let landscape = context.landscape_ref();
            let world_width = landscape
                .map(|landscape| landscape.width() as i32)
                .unwrap_or(0);
            let world_height = landscape.map(Landscape::estimated_height).unwrap_or(0);
            let position = match metadata.placement {
                // Running free: exactly two draws, even when the ground search
                // later fails (C4Game.cpp:3037-3041).
                0 => {
                    let x = draw_context_random(world_width)?;
                    let y = draw_context_random(world_height)?;
                    let Some((x, y)) = landscape
                        .and_then(|landscape| landscape.find_solid_ground(x, y, shape_width))
                    else {
                        return Ok(None);
                    };
                    Vector2::new(x, y)
                }
                // In liquid: surface search first, then the deep fallback. Both
                // consume only the initial x/y draws (C4Game.cpp:3043-3051).
                1 => {
                    let mut x = draw_context_random(world_width)?;
                    let mut y = draw_context_random(world_height)?;
                    let Some(landscape) = landscape else {
                        return Ok(None);
                    };
                    if !placement_find_surface_liquid(
                        landscape,
                        &mut x,
                        &mut y,
                        shape_width,
                        shape_height,
                    ) && !placement_find_liquid(
                        landscape,
                        &mut x,
                        &mut y,
                        shape_width,
                        shape_height,
                    ) {
                        return Ok(None);
                    }
                    Vector2::new(x, y.wrapping_add(shape_height / 2))
                }
                // Air: x draw, top-down first-semisolid scan, then y draw only
                // when the scan result is positive (C4Game.cpp:3053-3060).
                2 => {
                    let x = draw_context_random(world_width)?;
                    let mut y = 0;
                    while y < world_height
                        && landscape.is_some_and(|landscape| !landscape.is_semi_solid_at(x, y))
                    {
                        y += 1;
                    }
                    if y <= 0 {
                        return Ok(None);
                    }
                    Vector2::new(x, draw_context_random(y)?)
                }
                _ => unreachable!("placement validated above"),
            };
            Ok(Some(register_placement_object(
                context, definition, metadata, position, FULL_CON,
            )))
        })?;
    let Some(registration) = registration else {
        return Ok(Value::Nil);
    };
    finish_placement_object_creation(registration)
}

pub(crate) fn place_vegetation(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(definition) = parse_native_c4id_argument(args.first(), "PlaceVegetation")? else {
        return Ok(Value::Nil);
    };
    let x = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "PlaceVegetation", "x")?;
    let y = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "PlaceVegetation", "y")?;
    let width = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "PlaceVegetation",
        "width",
    )?;
    let height = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "PlaceVegetation",
        "height",
    )?;
    let requested_growth = value_to_i32(
        args.get(5).unwrap_or(&Value::Nil),
        "PlaceVegetation",
        "growth",
    )?;

    let registration = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("PlaceVegetation requires an active engine context")
        })?;
        let Some(metadata) = context.definition_metadata(&definition).cloned() else {
            // C4Id2Def failure happens before the first Random draw
            // (C4Game.cpp:2985-2986).
            return Ok(None);
        };
        let base = context
            .object_context()
            .map(ObjectScopeContext::effective_position)
            .unwrap_or(Vector2::ZERO);
        let area_x = base.x.wrapping_add(x);
        let area_y = base.y.wrapping_add(y);

        let mut growth = requested_growth;
        if growth <= 0 {
            growth = FULL_CON;
            if metadata.growth != 0 && draw_context_random(3)? == 0 {
                growth = draw_context_random(FULL_CON)?.wrapping_add(1);
            }
        }

        let (shape_width, shape_height) = metadata
            .shape
            .map(|shape| (shape.width, shape.height))
            .unwrap_or((0, 0));
        let Some(landscape) = context.landscape_ref() else {
            return Ok(None);
        };
        let bottom = match metadata.placement {
            // C4D_Place_Surface (C4Game.cpp:2998-3024).
            0 => {
                let mut found = None;
                for _ in 0..20 {
                    let tx = area_x.wrapping_add(draw_context_random(width)?);
                    let mut ty = area_y.wrapping_add(draw_context_random(height)?);
                    while ty > 0 && landscape.is_ift_at(tx, ty) {
                        ty -= 1;
                    }
                    let Some(ty) = landscape.above_semi_solid(tx, ty) else {
                        continue;
                    };
                    if !(50..=landscape.estimated_height() - 50).contains(&ty) {
                        continue;
                    }
                    if landscape.is_semi_solid_at(tx, ty - shape_height)
                        || landscape.is_semi_solid_at(tx, ty - shape_height / 2)
                        || landscape
                            .is_semi_solid_at(tx - shape_width / 2, ty - shape_height * 2 / 3)
                        || landscape
                            .is_semi_solid_at(tx + shape_width / 2, ty - shape_height * 2 / 3)
                    {
                        continue;
                    }
                    let soil_y = ty.wrapping_add(3);
                    let is_soil = landscape
                        .border_material_at(tx, soil_y)
                        .and_then(|material| {
                            context
                                .world
                                .materials()
                                .and_then(|materials| materials.get_by_id(material))
                        })
                        .and_then(|material| material.definition().int("Soil"))
                        .is_some_and(|soil| soil != 0);
                    if !is_soil {
                        continue;
                    }
                    if metadata.growth == 0 {
                        growth = FULL_CON;
                    }
                    found = Some(Vector2::new(tx, soil_y.wrapping_add(5)));
                    break;
                }
                let Some(found) = found else {
                    return Ok(None);
                };
                found
            }
            // C4D_Place_Liquid (C4Game.cpp:3027-3039).
            1 => {
                let mut tx = area_x.wrapping_add(draw_context_random(width)?);
                let mut ty = area_y.wrapping_add(draw_context_random(height)?);
                if !placement_find_surface_liquid(
                    landscape,
                    &mut tx,
                    &mut ty,
                    shape_width,
                    shape_height,
                ) && !placement_find_liquid(
                    landscape,
                    &mut tx,
                    &mut ty,
                    shape_width,
                    shape_height,
                ) {
                    return Ok(None);
                }
                let Some(ty) = landscape.semi_above_solid(tx, ty) else {
                    return Ok(None);
                };
                Vector2::new(tx, ty.wrapping_add(3))
            }
            _ => return Ok(None),
        };

        Ok(Some(register_placement_object(
            context, definition, metadata, bottom, growth,
        )))
    })?;
    let Some(registration) = registration else {
        return Ok(Value::Nil);
    };
    finish_placement_object_creation(registration)
}

pub(crate) fn create_construction(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnCreateConstruction takes a C4ID (C4Script.cpp:1911-1912); our
    // resources address definitions by their id string, so id values and
    // strings coincide.
    let Some(definition) = parse_native_c4id_argument(args.first(), "CreateConstruction")? else {
        return Ok(Value::Nil);
    };

    let mut index = 1;

    let x_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateConstruction", "x")?;
        index += 1;
        value
    } else {
        0
    };

    let y_offset = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateConstruction", "y")?;
        index += 1;
        value
    } else {
        0
    };

    let requested_owner = if let Some(arg) = args.get(index) {
        let owner = value_to_i32(arg, "CreateConstruction", "owner")?;
        index += 1;
        owner
    } else {
        0
    };

    let completion_percent = if let Some(arg) = args.get(index) {
        let value = value_to_i32(arg, "CreateConstruction", "completion")?;
        index += 1;
        value
    } else {
        0
    };

    let terrain_flag = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "CreateConstruction", "terrain")?;
        index += 1;
        flag
    } else {
        false
    };

    let check_site = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "CreateConstruction", "check_site")?;
        index += 1;
        flag
    } else {
        false
    };

    if index < args.len() {
        return Err(RuntimeError::new(
            "CreateConstruction: additional arguments are not supported",
        ));
    }

    let registration = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("CreateConstruction requires an active engine context")
        })?;

        // ConstructionCheck resolves the definition first and reports
        // IDS_OBJ_UNDEF through the calling object when a site check was
        // requested (C4Landscape.cpp:2131-2138); the later
        // CreateObjectConstruction C4Id2Def failure stays a silent nullptr
        // (C4Game.cpp:1183).
        if context.world.definition_known(&definition) == Some(false) {
            if check_site {
                let text = context
                    .world
                    .construction_check_strings
                    .format_undefined(&definition);
                register_construction_check_feedback(context, text);
            }
            return Ok(None);
        }

        let metadata = context
            .definition_metadata(&definition)
            .cloned()
            .unwrap_or_else(|| DefinitionMetadata {
                category: context
                    .definition_category(&definition)
                    .unwrap_or(DEFAULT_CATEGORY),
                ocf_base: ocf::NORMAL,
                constructable: true,
                ..Default::default()
            });
        let definition_category = metadata.category;
        let creator = context.object_context().map(ObjectScopeContext::id);
        let creator_layer = creator.and_then(|creator| context.object_layer(creator));
        let creator_layer_cache = creator
            .map(|creator| context.object_layer_compiler_cache(creator))
            .unwrap_or(0);

        let base_position = context
            .object_context()
            .map(|object| object.effective_position())
            .unwrap_or(Vector2::ZERO);
        let substitute_local_owner = matches!(
            clonk_script::caller_strictness(),
            clonk_script::HostCallerStrictness::NoCaller
                | clonk_script::HostCallerStrictness::NonStrict
        );
        let owner = if substitute_local_owner {
            context
                .object_context()
                .map(ObjectScopeContext::owner)
                .unwrap_or(requested_owner)
        } else {
            requested_owner
        };
        let position = Vector2::new(
            base_position.x.saturating_add(x_offset),
            base_position.y.saturating_add(y_offset),
        );

        // FnCreateConstruction forwards `iCompletion * FullCon / 100`
        // without clamping the percentage (C4Script.cpp:1911-1933). Normal
        // definitions clamp in DoCon; Oversize definitions retain values
        // above FullCon, including EMDropDef's unusual FullCon argument.
        let construction_value =
            ((i64::from(completion_percent) * i64::from(FULL_CON)) / 100) as i32;

        if check_site {
            if let Some(failure) = construction_check(context, &definition, &metadata, position)? {
                // GameMsgObject(..., cthr->Obj, FRed) feedback per failed
                // branch (C4Landscape.cpp:2139-2163).
                let text = match failure {
                    crate::command::ConstructionCheckFailure::NotConstructable => {
                        let name = if metadata.name.is_empty() {
                            definition.clone()
                        } else {
                            metadata.name.clone()
                        };
                        context
                            .world
                            .construction_check_strings
                            .format_not_constructable(&name)
                    }
                    crate::command::ConstructionCheckFailure::NoRoom => {
                        context.world.construction_check_strings.no_room.clone()
                    }
                    crate::command::ConstructionCheckFailure::NoLevel => {
                        context.world.construction_check_strings.no_level.clone()
                    }
                    crate::command::ConstructionCheckFailure::Blocked(blocker) => {
                        let name = context
                            .object_effective_name(blocker)
                            .filter(|name| !name.is_empty())
                            .unwrap_or_default();
                        context
                            .world
                            .construction_check_strings
                            .format_blocked(&name)
                    }
                };
                register_construction_check_feedback(context, text);
                return Ok(None);
            }
        }

        if terrain_flag {
            let (width, height) = metadata
                .shape
                .map(|shape| (shape.width, shape.height))
                .unwrap_or_default();
            context.prepare_construction_terrain(
                position.x,
                position.y,
                width,
                height,
                metadata.basement,
            );
        }

        let id = context.allocate_object_id();

        let mut spawn = SpawnConfig::new(definition.clone())
            .with_position(position)
            .with_owner(owner)
            .with_category(definition_category)
            // C4Game::NewObject inserts the object at Con=0, calls
            // Construction, and only then applies the requested iCon.
            .with_construction(0)
            .with_id(id);
        if let Some(layer) = creator_layer {
            spawn = spawn.with_layer(layer);
        }
        spawn.compiler_cache.layer = creator_layer_cache;
        // The creating controller rides onto the site (FnCreateConstruction,
        // C4Script.cpp:1932-1933).
        let creator_controller = context
            .object_context()
            .map(ObjectScopeContext::controller)
            .filter(|value| *value > OWNER_NONE);
        if let Some(controller) = creator_controller {
            spawn = spawn.with_controller(controller);
        }
        // The lifecycle below runs while this host call is live; the later
        // copy-out spawn must not repeat callbacks or the initial DoCon.
        spawn.initialized = true;
        spawn.position_adjusted = true;

        let initial_alive = metadata.category & crate::CATEGORY_LIVING != 0;
        let preview_ocf = ocf::compute(
            metadata.ocf_base,
            metadata.crew_member,
            initial_alive,
            ObjectStatus::Normal,
            false,
            0,
            metadata.category,
        );
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
            if initial_alive {
                metadata.physical.energy
            } else {
                0
            },
            0,
            0,
            position,
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0,
            None,
        )
        .with_compiler_fields(
            0,
            0,
            -1,
            crate::ObjectCompilerCache {
                layer: creator_layer_cache,
                ..crate::ObjectCompilerCache::default()
            },
        )
        .with_alive(initial_alive)
        .with_ocf(preview_ocf)
        .with_full_state(Rc::new({
            let mut state = crate::preview_spawn_state_with_components(
                position,
                owner,
                creator_controller.unwrap_or(owner),
                definition_category,
                0,
                metadata.contact_density(),
                metadata.vertices.clone(),
                metadata.components.as_slice(),
            );
            state.alive = initial_alive;
            state.energy = if initial_alive {
                metadata.physical.energy
            } else {
                0
            };
            state.crew_member = metadata.crew_member;
            state.layer = creator_layer;
            state.blit_mode = metadata.blit_mode;
            state
        }));

        context.register_spawn(spawn, preview);
        Ok(Some((
            id,
            creator,
            construction_value,
            metadata.shape,
            metadata.stretch_growth,
            metadata.line,
            metadata.ocf_base,
            metadata.crew_member,
            metadata.category,
            initial_alive,
        )))
    })?;
    let Some((
        target,
        creator,
        construction_value,
        shape,
        stretch_growth,
        line,
        ocf_base,
        crew_member,
        category,
        alive,
    )) = registration
    else {
        return Ok(Value::Nil);
    };

    // C4Game::NewObject exposes the inserted object to scripts before
    // running PSF_Construction, passing the creator as its sole argument
    // (C4Game.cpp:1110-1121).
    let creator_arg = creator.map(object_reference_value).unwrap_or(Value::Nil);
    if let Some(Err(error)) = call_world_object_own_function(target, "Construction", &[creator_arg])
    {
        tracing::error!(
            id = target.as_u64(),
            callback = "Construction",
            %error,
            "creation callback failed; continuing like C++ fail-safe Call"
        );
        log_runtime_call_frames("", error.call_frames());
    }
    let removed = with_host_context(false, |context| context.nested_object_destroyed(target));
    if removed {
        return Ok(Value::Nil);
    }

    // Initial DoCon adds iCon to whatever Construction left behind, keeps
    // the old shape bottom fixed in integer coordinates, and leaves fix_y
    // at the pre-growth position (C4Object.cpp:1432-1500).
    let (crossed_full_con, final_construction) = with_host_context_mut((false, 0), |context| {
        if !context.ensure_object_scope(target) {
            return (false, 0);
        }
        let was_full = context
            .object_scope(target)
            .is_some_and(|scope| scope.construction() >= FULL_CON);
        let Some(final_construction) =
            context.adjust_object_construction(target, construction_value)
        else {
            return (false, 0);
        };
        let (pre_growth_position, adjusted_position) = {
            let Some(scope) = context.object_scope_mut(target) else {
                return (false, 0);
            };
            let pre_growth_position = scope.effective_position();
            let adjusted_position = Vector2::new(
                pre_growth_position.x,
                crate::docon_initial_center_y(
                    shape,
                    stretch_growth,
                    line,
                    final_construction,
                    pre_growth_position.y,
                ),
            );
            scope.pending_update.construction = None;
            scope.current_position = adjusted_position;
            scope.pending_update.position = None;
            scope.cached_ocf = Some(ocf::compute(
                ocf_base,
                crew_member,
                alive,
                ObjectStatus::Normal,
                false,
                final_construction,
                category,
            ));
            (pre_growth_position, adjusted_position)
        };
        if let Some(spawn) = context
            .pending_spawns
            .iter_mut()
            .find(|spawn| spawn.id == Some(target))
        {
            spawn.position = adjusted_position;
            spawn.construction = final_construction;
            spawn.fixed_position = (adjusted_position != pre_growth_position).then_some(
                FixedVec2::from_ints(pre_growth_position.x, pre_growth_position.y),
            );
        }
        context.update_live_solid_mask(target, false);
        (
            !was_full && final_construction >= FULL_CON,
            final_construction,
        )
    });

    // DoCon(0) removes a zero-construction object, and NewObject returns
    // nullptr after its status re-check (C4Object.cpp:1513-1517;
    // C4Game.cpp:1122-1128).
    if final_construction <= 0 {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.cancel_pending_spawn(target);
                context.nested_objects.remove(&target);
                context.nested_order.retain(|id| *id != target);
            }
        });
        return Ok(Value::Nil);
    }

    if crossed_full_con {
        for callback in ["Completion", "Initialize"] {
            if let Some(Err(error)) = call_world_object_own_function(target, callback, &[]) {
                tracing::error!(
                    id = target.as_u64(),
                    callback,
                    %error,
                    "creation callback failed; continuing like C++ fail-safe Call"
                );
                log_runtime_call_frames("", error.call_frames());
            }
        }
    }

    let removed = with_host_context(false, |context| context.nested_object_destroyed(target));
    Ok(if removed {
        Value::Nil
    } else {
        object_reference_value(target)
    })
}

/// `GameMsgObject(szMsg, pByObj, FRed)` for a failed ConstructionCheck: the
/// red feedback lands on the calling object and only when one exists
/// (C4Landscape.cpp:2131-2163). Global calls stay silent like C++'s null
/// `pByObj`.
fn register_construction_check_feedback(context: &mut EffectHostContext, text: String) {
    let Some(target) = context.object_context().map(ObjectScopeContext::id) else {
        return;
    };
    context.register_message(MessageCommand::Add(
        MessageSpec::target(text, target).with_color(crate::CONSTRUCTION_CHECK_MESSAGE_COLOR),
    ));
}

pub(crate) fn construction_check(
    context: &EffectHostContext,
    _definition_id: &str,
    metadata: &DefinitionMetadata,
    position: Vector2,
) -> Result<Option<crate::command::ConstructionCheckFailure>, RuntimeError> {
    use crate::command::ConstructionCheckFailure;

    if !metadata.constructable {
        return Ok(Some(ConstructionCheckFailure::NotConstructable));
    }

    let (raw_width, raw_height) = metadata
        .shape
        .map(|rect| (rect.width, rect.height))
        .unwrap_or((20, 40));
    let width = raw_width.max(1);
    let height = raw_height.max(1);
    let effective_height = height.saturating_sub(metadata.construction_offset).max(1);

    let rect_left = position.x - width / 2;
    let rect_right = rect_left + width;
    let rect_top = position.y - effective_height;
    let rect_bottom = position.y;

    let Some(landscape) = context.landscape_ref() else {
        return Ok(None);
    };

    // ConstructionCheck uses AreaSolidCount over the actual pixel plane;
    // column surface heights are not equivalent in caves or below a closed
    // roof (C4Landscape.cpp:1090-1098,2125-2158).
    let solid_count = (rect_top..rect_bottom)
        .flat_map(|y| (rect_left..rect_right).map(move |x| (x, y)))
        .filter(|&(x, y)| landscape.is_solid_at(x, y))
        .count()
        .min(i32::MAX as usize) as i32;
    let support_count = (rect_bottom..rect_bottom.saturating_add(5))
        .flat_map(|y| (rect_left..rect_right).map(move |x| (x, y)))
        .filter(|&(x, y)| landscape.is_solid_at(x, y))
        .count()
        .min(i32::MAX as usize) as i32;

    let area_threshold = ((i64::from(width) * i64::from(effective_height)) / 20)
        .clamp(0, i64::from(i32::MAX)) as i32;
    if solid_count > area_threshold {
        return Ok(Some(ConstructionCheckFailure::NoRoom));
    }

    if support_count < width.saturating_mul(2) {
        return Ok(Some(ConstructionCheckFailure::NoLevel));
    }

    let overlap_mask = metadata.category & CATEGORY_SORT_LIMIT;
    if overlap_mask == 0 {
        return Ok(None);
    }

    let current_object_id = context.object_context().map(|object| object.id());
    for object_id in context.world_object_ids() {
        let Some(other) = context.get_world_object(object_id) else {
            continue;
        };
        if Some(other.id) == current_object_id {
            continue;
        }
        if !other.is_present() || !other.status().is_active() {
            continue;
        }
        if other.container().is_some() {
            continue;
        }
        if other.category() & overlap_mask & CATEGORY_SORT_LIMIT == 0 {
            continue;
        }
        let candidate = effect_object_live_shape_rect(context, &other);
        let requested = DefinitionRect::new(
            rect_left,
            rect_top,
            rect_right.wrapping_sub(rect_left),
            rect_bottom.wrapping_sub(rect_top),
        );
        if rects_overlap_cpp(requested, candidate) {
            return Ok(Some(ConstructionCheckFailure::Blocked(other.id)));
        }
    }

    Ok(None)
}

/// `FnFindConstructionSite` (C4Script.cpp:1958-1981): stages coordinates
/// through the CALLER's Var slots — reads the start position from
/// `Caller->NumVars[iVarX/iVarY]`, accepts it when ConstructionCheck
/// passes, else runs the FindConSiteSpot landscape probe
/// (C4Landscape.cpp:1987-2043, hrange 20) with the Game.OverlapObject
/// veto and writes the coordinates back into the caller's slots. The
/// planet System.c4g FindConstructionSiteX wrapper (Commits.c:384-390)
/// drives it — 10x DLAR Initialize in SkiesOfFire.
pub(crate) fn find_construction_site(args: &[Value]) -> Result<Value, RuntimeError> {
    // C4Id2Def failure yields the empty optional (:1962).
    let Some(definition) = parse_native_c4id_argument(args.first(), "FindConstructionSite")? else {
        return Ok(Value::Nil);
    };
    let var_x = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "FindConstructionSite",
        "iVarX",
    )?;
    let var_y = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "FindConstructionSite",
        "iVarY",
    )?;
    // Var indices out of range (:1964).
    if !(0..AUL_MAX_PAR).contains(&var_x) || !(0..AUL_MAX_PAR).contains(&var_y) {
        return Ok(Value::Nil);
    }
    // `if (!cthr->Caller) return {}` (:1966).
    let Some(slots) = clonk_script::caller_var_slots() else {
        return Ok(Value::Nil);
    };
    let v1 = value_as_int(&slots.get(var_x));
    let v2 = value_as_int(&slots.get(var_y));

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref().ok_or_else(|| {
            RuntimeError::new("FindConstructionSite requires an active engine context")
        })?;
        let Some(metadata) = context.definition_metadata(&definition) else {
            return Ok(Value::Nil);
        };
        // Construction check at the starting position (:1970-1971): the
        // caller's vars stay untouched on an immediate hit. C++ passes no
        // pByObj here, so a failure produces no feedback (:1964).
        if construction_check(context, &definition, metadata, Vector2::new(v1, v2))?.is_none() {
            return Ok(Value::Bool(true));
        }
        // Search for real (:1973-1977) with pDef->Shape.Wdt/Hgt and
        // Category.
        let (wdt, hgt) = metadata
            .shape
            .map(|rect| (rect.width, rect.height))
            .unwrap_or((0, 0));
        let category = metadata.category;
        let found = context.landscape_ref().and_then(|landscape| {
            landscape.find_con_site_spot(v1, v2, wdt, hgt, 20, |x, y, w, h| {
                host_overlap_object(context, x, y, w, h, category)
            })
        });
        // V1 = C4VInt(v1); V2 = C4VInt(v2) — written back even when the
        // probe found nothing (:1978).
        let (out_x, out_y) = found.unwrap_or((v1, v2));
        slots.set(var_x, Value::Int(out_x));
        slots.set(var_y, Value::Int(out_y));
        Ok(Value::Bool(found.is_some()))
    })
}

pub(crate) fn contained(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Contained expects at most 1 argument: target",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "Contained", "target")?;
    }

    with_host_context(Ok(Value::Nil), |context| {
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

pub(crate) fn contents(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "Contents expects at most 3 arguments: index, object, include_attached",
        ));
    }

    let index = match args.first() {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "Contents", "index")?,
    };
    if index < 0 {
        return Ok(Value::Nil);
    }

    let target_id =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "Contents", "object")?;
    let include_attached = if let Some(value) = args.get(2) {
        value_to_bool(value, "Contents", "include_attached")?
    } else {
        false
    };

    with_host_context(Ok(Value::Nil), |context| {
        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Nil),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) if !context.removed_object_references.contains(&container_id) => object,
            _ => return Ok(Value::Nil),
        };

        // C4ObjectList::GetObject applies the Status filter before indexing.
        // FnContents then advances that raw index only while the selected
        // entry uses DFA_ATTACH. Filtering attached entries up front would
        // shift every later index and lose C++'s duplicate-return quirk.
        let mut entries = Vec::new();
        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                entries.push(child);
            }
        }

        let mut raw_index = index as usize;
        while let Some(selected) = entries.get(raw_index) {
            let attached = selected
                .procedure_name()
                .is_some_and(|procedure| procedure.eq_ignore_ascii_case("attach"));
            if include_attached || !attached {
                return Ok(object_reference_value(selected.id));
            }
            raw_index += 1;
        }
        Ok(Value::Nil)
    })
}

pub(crate) fn contents_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "ContentsCount expects at most 2 arguments: definition, object",
        ));
    }

    let definition = parse_native_c4id_argument(args.first(), "ContentsCount")?;
    let target_id = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "ContentsCount",
        "object",
    )?;

    with_host_context(Ok(Value::Int(0)), |context| {
        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Int(0)),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) if !context.removed_object_references.contains(&container_id) => object,
            _ => return Ok(Value::Int(0)),
        };

        let mut count = 0;
        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                if let Some(definition_id) = definition.as_ref() {
                    if context
                        .object_effective_definition_id(*child_id)
                        .is_none_or(|id| id.as_str() != definition_id)
                    {
                        continue;
                    }
                }
                count += 1;
            }
        }

        Ok(Value::Int(count))
    })
}

pub(crate) fn find_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition =
        parse_native_c4id_argument(Some(args.first().unwrap_or(&Value::Nil)), "FindContents")?;
    let Some(definition) = definition else {
        return Ok(Value::Nil);
    };

    let target_id = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "FindContents",
        "object",
    )?;

    with_host_context(Ok(Value::Nil), |context| {
        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Nil),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) if !context.removed_object_references.contains(&container_id) => object,
            _ => return Ok(Value::Nil),
        };

        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                if context
                    .object_effective_definition_id(*child_id)
                    .is_some_and(|id| id.as_str() == definition)
                {
                    return Ok(object_reference_value(child.id));
                }
            }
        }

        Ok(Value::Nil)
    })
}

pub(crate) fn find_other_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition = parse_native_c4id_argument(
        Some(args.first().unwrap_or(&Value::Nil)),
        "FindOtherContents",
    )?;
    let target_id = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "FindOtherContents",
        "object",
    )?;

    with_host_context(Ok(Value::Nil), |context| {
        let container_id = if let Some(id) = target_id {
            id
        } else {
            match context.object_context() {
                Some(object) => object.id(),
                None => return Ok(Value::Nil),
            }
        };

        let container = match context.get_world_object(container_id) {
            Some(object) => object,
            _ => return Ok(Value::Nil),
        };

        for child_id in container.contents() {
            if let Some(child) = context.get_world_object(*child_id) {
                if !child.is_present() {
                    continue;
                }
                let matches = match definition.as_ref() {
                    Some(definition_id) => context
                        .object_effective_definition_id(*child_id)
                        .is_none_or(|id| id.as_str() != definition_id),
                    None => true,
                };
                if matches {
                    return Ok(object_reference_value(child.id));
                }
            }
        }

        Ok(Value::Nil)
    })
}

pub(crate) fn get_ocf(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetOCF expects at most 1 argument: target",
        ));
    }

    let target_value = args.first().unwrap_or(&Value::Nil);
    let target_id = parse_object_reference_argument(target_value, "GetOCF", "target")?;

    with_host_context(Ok(Value::Nil), |context| {
        let ocf_value = |mask: u32| Value::Int(mask as i32);

        if let Some(target) = target_id {
            if let Some(object) = context.object_context() {
                if object.id() == target {
                    return Ok(ocf_value(object.ocf()));
                }
            }
            if let Some(other) = context.get_world_object(target) {
                return Ok(ocf_value(other.ocf()));
            }
            return Ok(Value::Nil);
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };

        Ok(ocf_value(object.ocf()))
    })
}

pub(crate) fn get_category(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetCategory expects at most 2 arguments: target, definition",
        ));
    }

    let target_value = args.first().unwrap_or(&Value::Nil);
    let target_id = parse_object_reference_argument(target_value, "GetCategory", "target")?;
    let definition = parse_native_c4id_argument(args.get(1), "GetCategory")?;

    with_host_context(Ok(Value::Nil), |context| {
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
pub(crate) struct NativeObjectCreation {
    pub(crate) definition: String,
    pub(crate) creator: Option<ObjectId>,
    pub(crate) owner: i32,
    pub(crate) controller: i32,
    pub(crate) construction: i32,
    pub(crate) position: Vector2,
    pub(crate) rotation: i32,
    pub(crate) velocity: FixedVec2,
    pub(crate) rotation_velocity: C4Fixed,
}

/// Synchronous `C4Game::CreateObject` for native engine operations whose
/// exact initial position and fixed motion are not expressible through the
/// script `CreateObject` wrapper. The pending spawn is only storage: every
/// NewObject lifecycle callback runs here before this function returns.
pub(crate) fn create_native_object(
    request: NativeObjectCreation,
) -> Result<Option<ObjectId>, RuntimeError> {
    let registration = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("native object creation requires an active engine context")
        })?;
        if context.world.definition_known(&request.definition) == Some(false) {
            return Ok(None);
        }
        let metadata = context
            .definition_metadata(&request.definition)
            .cloned()
            .unwrap_or_else(|| DefinitionMetadata {
                category: context
                    .definition_category(&request.definition)
                    .unwrap_or(DEFAULT_CATEGORY),
                ..DefinitionMetadata::default()
            });
        // C4Object::Init discards initial r/rdir for non-rotateable defs,
        // after their callers have already consumed every random draw.
        let (rotation, rotation_velocity) = if metadata.rotateable == 0 {
            (0, C4Fixed::ZERO)
        } else {
            (request.rotation, request.rotation_velocity)
        };
        let controller = if request.controller > OWNER_NONE {
            request.controller
        } else {
            request.owner
        };
        let creator_layer = request
            .creator
            .and_then(|creator| context.object_layer(creator));
        let creator_layer_cache = request
            .creator
            .map(|creator| context.object_layer_compiler_cache(creator))
            .unwrap_or(0);
        let id = context.allocate_object_id();
        let mut spawn = SpawnConfig::new(request.definition.clone())
            .with_position(request.position)
            .with_fixed_velocity(request.velocity)
            .with_rotation(rotation)
            .with_rotation_velocity(rotation_velocity)
            .with_owner(request.owner)
            .with_controller(controller)
            .with_category(metadata.category)
            .with_construction(0)
            .with_id(id);
        if let Some(layer) = creator_layer {
            spawn = spawn.with_layer(layer);
        }
        spawn.compiler_cache.layer = creator_layer_cache;
        spawn.initialized = true;
        spawn.position_adjusted = true;

        let alive = metadata.category & crate::CATEGORY_LIVING != 0;
        let preview_ocf = ocf::compute(
            metadata.ocf_base,
            metadata.crew_member,
            alive,
            ObjectStatus::Normal,
            false,
            0,
            metadata.category,
        );
        let preview_velocity = Vector2::new(request.velocity.int_x(), request.velocity.int_y());
        let preview = HostWorldObject::with_category(
            id,
            request.definition.clone(),
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            request.owner,
            metadata.category,
            if alive { metadata.physical.energy } else { 0 },
            0,
            0,
            request.position,
            preview_velocity,
            rotation,
            metadata.vertices.clone(),
            0,
            0,
            0,
            None,
        )
        .with_compiler_fields(
            0,
            0,
            -1,
            crate::ObjectCompilerCache {
                layer: creator_layer_cache,
                ..crate::ObjectCompilerCache::default()
            },
        )
        .with_fixed_motion(
            FixedVec2::from_ints(request.position.x, request.position.y),
            request.velocity,
        )
        .with_rotation_velocity(rotation_velocity)
        .with_alive(alive)
        .with_ocf(preview_ocf)
        .with_full_state(Rc::new({
            let mut state = crate::preview_spawn_state_with_components(
                request.position,
                request.owner,
                controller,
                metadata.category,
                0,
                metadata.contact_density(),
                metadata.vertices.clone(),
                metadata.components.as_slice(),
            );
            state.velocity = preview_velocity;
            state.script_fixed_velocity = Some(request.velocity);
            state.script_rotation_velocity = Some(rotation_velocity);
            state.rotation = rotation;
            state.alive = alive;
            state.energy = if alive { metadata.physical.energy } else { 0 };
            state.breath = metadata.physical.breath;
            state.crew_member = metadata.crew_member;
            state.layer = creator_layer;
            state.blit_mode = metadata.blit_mode;
            if context.world.definition_color_by_owner(&request.definition) {
                state.color = context
                    .player_state(request.owner)
                    .and_then(|player| player.color)
                    .map(|color| {
                        u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                    })
                    .unwrap_or(0);
            }
            state.mobile = metadata.category != crate::CATEGORY_STATIC_BACK
                && (request.velocity != FixedVec2::ZERO || rotation_velocity.is_nonzero());
            state
        }));
        context.register_spawn(spawn, preview);
        if context.ensure_object_scope(id) {
            if let Some(scope) = context.object_scope_mut(id) {
                scope.current_fixed_velocity = request.velocity;
                scope.current_rotation_velocity = rotation_velocity;
            }
        }
        Ok(Some((id, metadata, rotation)))
    })?;
    let Some((target, metadata, initial_rotation)) = registration else {
        return Ok(None);
    };

    let creator_arg = request
        .creator
        .map(object_reference_value)
        .unwrap_or(Value::Nil);
    call_object_own_fail_safe(target, "Construction", &[creator_arg]);
    if !object_has_status(target) {
        return Ok(None);
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.commit_creation_action(target);
        }
    });

    // Construction may ChangeDef synchronously. Initial DoCon and the
    // Completion/Initialize lookup use the object's live definition in C++.
    let metadata = HOST_CONTEXT
        .with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let definition = context.object_effective_definition_id(target)?;
            context.definition_metadata(definition.as_str()).cloned()
        })
        .unwrap_or(metadata);

    let staged = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let before = context.object_scope(target)?.construction();
        let entry_position = context.object_scope(target)?.effective_position();
        let entry_shape = live_object_shape(context, target);
        let was_full = before >= FULL_CON;
        let Some(final_construction) =
            context.stage_live_docon_construction(target, request.construction)
        else {
            return None;
        };
        let refresh = crate::docon_refreshes_construction(before, final_construction);
        let _ = refresh_live_object_ocf(context, target);
        if refresh {
            let Some(scope) = context.object_scope_mut(target) else {
                return None;
            };
            if metadata.line == 0 {
                scope.pending_update.shape_override = Some(None);
            }
            scope.refresh_shape_preview(&metadata);
            context.update_live_solid_mask(target, false);
        }
        if let Some(scope) = context.object_scope_mut(target) {
            scope.pending_update.construction = None;
        }
        if let Some(spawn) = context
            .pending_spawns
            .iter_mut()
            .find(|spawn| spawn.id == Some(target))
        {
            spawn.construction = final_construction;
            // Construction may have changed r. The nested update remains
            // authoritative, but seed the spawn for the no-write case.
            spawn.rotation = initial_rotation;
        }
        Some((
            was_full,
            final_construction,
            entry_position,
            entry_shape,
            refresh,
        ))
    });
    let Some((was_full, staged_construction, entry_position, entry_shape, refresh)) = staged else {
        return Ok(None);
    };

    // Initial DoCon performs the same incomplete-construction side arms as
    // an ordinary DoCon before its keep-bottom position update. Construction
    // may have added contents or armed NeedEnergy, and Exit/Enter callbacks
    // here are synchronously visible before NewObject returns.
    if refresh && staged_construction < FULL_CON {
        if !metadata.fire.incomplete_activity {
            loop {
                let next = HOST_CONTEXT.with(|cell| {
                    let borrow = cell.borrow();
                    let context = borrow.as_ref()?;
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

    // C4Object::DoCon idles an object that decays from full construction
    // before applying its keep-bottom position adjustment. Construction and
    // the incomplete-object side arms above can both change the live
    // definition or construction, so re-read them at this exact point.
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

    let final_construction = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let current_shape = live_object_shape(context, target);
        let scope = context.object_scope_mut(target)?;
        let current_position = scope.effective_position();
        let preserved_fixed_position = scope.fixed_position();
        let adjusted_y = match (entry_shape, current_shape) {
            (Some(entry), Some(current))
                if entry.height != current.height || entry.y != current.y =>
            {
                entry_position
                    .y
                    .saturating_add(entry.y)
                    .saturating_add(entry.height)
                    .saturating_sub(current.height)
                    .saturating_sub(current.y)
            }
            _ => current_position.y,
        };
        let adjusted_position = Vector2::new(current_position.x, adjusted_y);
        let final_construction = scope.construction();
        scope.current_position = adjusted_position;
        scope.pending_update.position = None;
        if let Some(spawn) = context
            .pending_spawns
            .iter_mut()
            .find(|spawn| spawn.id == Some(target))
        {
            spawn.position = adjusted_position;
            spawn.construction = final_construction;
            let adjusted_fixed = FixedVec2::from_ints(adjusted_position.x, adjusted_position.y);
            spawn.fixed_position =
                (preserved_fixed_position != adjusted_fixed).then_some(preserved_fixed_position);
        }
        if adjusted_position != current_position {
            context.update_live_solid_mask(target, false);
        }
        Some(final_construction)
    });
    let Some(final_construction) = final_construction else {
        return Ok(None);
    };
    if final_construction <= 0 {
        // C4Object::DoCon reaches AssignRemoval at Con=0 before NewObject's
        // status re-check returns nullptr. Run the complete synchronous
        // Destruction/effect/contents lifecycle. Keep the destroyed pending
        // scope until the generic outcome fold filters its spawn; this is the
        // same path used when Construction removes a newly created object.
        let _ = assign_removal_live(target, false)?;
        return Ok(None);
    }
    let crossed_full_con = !was_full && final_construction >= FULL_CON;
    if crossed_full_con && object_has_status(target) {
        call_object_own_fail_safe(target, "Completion", &[]);
        if object_has_status(target) {
            call_object_own_fail_safe(target, "Initialize", &[]);
        }
    }
    Ok(object_has_status(target).then_some(target))
}

/// FnCreateContents (C4Script.cpp:1938-1951): create `count` (default 1)
/// objects of `c_id` inside the container, returning the last one. C++
/// routes through pObj->CreateContents -> CreateObject + Enter, with the
/// container's owner.
pub(crate) fn create_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(definition) = parse_native_c4id_argument(args.first(), "CreateContents")? else {
        return Ok(Value::Nil);
    };

    let mut index = 1;
    let target_id = consume_optional_object_reference_argument(
        args,
        &mut index,
        "CreateContents",
        "container",
    )?;
    let count = match args.get(index) {
        // C++: `if (!iCount) ++iCount;`
        Some(arg) => match value_to_i32(arg, "CreateContents", "count")? {
            0 => 1,
            value => value,
        },
        None => 1,
    };

    let container = try_with_host_context(
        "CreateContents requires an active engine context",
        |context| Ok::<_, RuntimeError>(target_id.or(context.script_object_context)),
    )?;
    let Some(container) = container else {
        return Ok(Value::Nil);
    };
    if !object_is_present(container) {
        return Ok(Value::Nil);
    }

    let mut last = Value::Nil;
    for _ in 0..count {
        let owner = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            context
                .object_scope(container)
                .map(ObjectScopeContext::owner)
                .or_else(|| {
                    context
                        .get_world_object(container)
                        .map(|object| object.owner)
                })
        });
        let Some(owner) = owner else {
            last = Value::Nil;
            continue;
        };
        // C4Game::CreateObject's omitted coordinates are (50,50). Only the
        // subsequent Enter copies the container's current motion.
        let Some(created) = create_native_object(NativeObjectCreation {
            definition: definition.clone(),
            creator: Some(container),
            owner,
            controller: owner,
            construction: FULL_CON,
            position: Vector2::new(50, 50),
            rotation: 0,
            velocity: FixedVec2::ZERO,
            rotation_velocity: C4Fixed::ZERO,
        })?
        else {
            last = Value::Nil;
            continue;
        };
        if enter_object_live(created, container)? {
            last = object_reference_value(created);
        } else {
            let _ = assign_removal_live(created, false)?;
            last = Value::Nil;
        }
    }
    Ok(last)
}

fn resolve_component_list(
    definition: &str,
    instance: Option<ObjectId>,
    builder: Option<ObjectId>,
) -> Result<Vec<(String, i32)>, RuntimeError> {
    let (script, static_components) = with_host_context((None, Vec::new()), |context| {
        (
            context.world.definition_script(definition).cloned(),
            context
                .definition_metadata(definition)
                .map(|metadata| metadata.components.clone())
                .unwrap_or_default(),
        )
    });
    let builder = builder.map(object_reference_value).unwrap_or(Value::Nil);
    let custom = script
        .filter(|script| script.has_local_function("GetCustomComponents"))
        .and_then(|script| {
            match instance {
                Some(instance) => call_world_object_function_in_scope(
                    instance,
                    script,
                    "GetCustomComponents",
                    std::slice::from_ref(&builder),
                ),
                None => call_scoped_definition_function(
                    script,
                    definition,
                    "GetCustomComponents",
                    std::slice::from_ref(&builder),
                ),
            }
            .and_then(|result| match result {
                Ok(value) => Some(value),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        definition,
                        "GetCustomComponents failed; using stored components like C++"
                    );
                    None
                }
            })
        });
    if let Some(Value::Array(values)) = custom {
        return Ok(component_list_from_custom_array(&values));
    }
    let Some(instance) = instance else {
        return Ok(static_components);
    };

    Ok(with_host_context(Vec::new(), |context| {
        let object = context.get_world_object(instance);
        let components = context
            .object_scope(instance)
            .and_then(|scope| scope.pending_update.components.clone())
            .or_else(|| {
                object
                    .as_ref()
                    .and_then(|object| object.full_state().map(|state| state.components.clone()))
            })
            .unwrap_or_else(|| {
                static_components
                    .iter()
                    .map(|(id, count)| (DefinitionId::from(id.as_str()), *count))
                    .collect()
            });
        let order = context
            .object_scope(instance)
            .and_then(|scope| scope.pending_update.component_order.clone())
            .or_else(|| {
                object.as_ref().and_then(|object| {
                    object
                        .full_state()
                        .map(|state| state.component_order.clone())
                })
            })
            .unwrap_or_else(|| {
                static_components
                    .iter()
                    .map(|(id, _)| DefinitionId::from(id.as_str()))
                    .collect()
            });
        order
            .into_iter()
            .map(|id| {
                let count = components.get(&id).unwrap_or(0);
                (id.as_str().to_owned(), count)
            })
            .collect()
    }))
}

fn live_contents_matching(container: ObjectId, definition: &str) -> Vec<ObjectId> {
    with_host_context(Vec::new(), |context| {
        context
            .get_world_object(container)
            .map(|container| {
                container
                    .contents()
                    .iter()
                    .copied()
                    .filter(|child| {
                        context.get_world_object(*child).is_some_and(|object| {
                            object.is_present()
                                && context
                                    .object_effective_definition_id(*child)
                                    .is_some_and(|id| id.as_str() == definition)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// FnComposeContents -> C4Object::ComposeContents
/// (C4Script.cpp:1946-1950; C4Object.cpp:3764-3806).
pub(crate) fn compose_contents(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "ComposeContents expects at most 2 arguments: definition and container",
        ));
    }
    let Some(definition) = parse_native_c4id_argument(args.first(), "ComposeContents")? else {
        return Ok(Value::Nil);
    };
    let explicit = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "ComposeContents",
        "container",
    )?;
    let container = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| explicit.or(context.script_object_context))
    });
    let Some(container) = container.filter(|container| object_is_present(*container)) else {
        return Ok(Value::Nil);
    };
    let definition_known = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.definition_known(&definition))
    });
    if definition_known == Some(false) {
        return Ok(Value::Nil);
    }

    let components = resolve_component_list(&definition, None, Some(container))?;
    let mut missing = Vec::<(String, i32)>::new();
    for (component, needed) in &components {
        let available =
            i32::try_from(live_contents_matching(container, component).len()).unwrap_or(i32::MAX);
        if *needed > available {
            missing.push((component.clone(), needed - available));
        }
    }
    if let Some((first_id, first_count)) = missing.first() {
        let handled = call_object_own_fail_safe(
            container,
            "BuildNeedsMaterial",
            &[Value::C4Id(first_id.clone()), Value::Int(*first_count)],
        )
        .as_bool();
        if !handled {
            let text = HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let context = borrow.as_ref();
                let display_name = |id: &str| {
                    context
                        .and_then(|context| context.definition_metadata(id))
                        .map(|metadata| metadata.name.as_str())
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| clonk_script::c4_id_text(id))
                };
                let mut text = format!("{}|needs", display_name(&definition));
                for (component, count) in &missing {
                    text.push_str(&format!("|{count}x {}", display_name(component)));
                }
                text
            });
            HOST_CONTEXT.with(|cell| {
                if let Some(context) = cell.borrow_mut().as_mut() {
                    context.register_message(MessageCommand::Add(
                        MessageSpec::target(text, container)
                            .with_color(invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR)),
                    ));
                }
            });
        }
        return Ok(Value::Nil);
    }

    for (component, count) in components {
        for _ in 0..count {
            let Some(item) = live_contents_matching(container, &component)
                .into_iter()
                .next()
            else {
                return Ok(Value::Nil);
            };
            let _ = assign_removal_live(item, false)?;
        }
    }
    create_contents(&[
        Value::C4Id(definition),
        object_reference_value(container),
        Value::Int(1),
    ])
}

/// FnSplit2Components (C4Script.cpp:415-454): transfer contents, resolve
/// the source's live/custom recipe, create every piece with exact random
/// draw order, then remove the source.
pub(crate) fn split_to_components(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Split2Components expects at most 1 argument: object",
        ));
    }
    let explicit = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "Split2Components",
        "object",
    )?;
    let (source, builder) = with_host_context((None, None), |context| {
        (
            explicit.or(context.script_object_context),
            context.script_object_context,
        )
    });
    let Some(source) = source.filter(|source| object_is_present(*source)) else {
        return Ok(Value::Bool(false));
    };
    let original_container = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(source))
            .and_then(|object| object.container())
    });

    loop {
        let child = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| first_retained_content(context, source))
        });
        let Some(child) = child else { break };
        let moved = match original_container {
            Some(container) => enter_object_live(child, container)?,
            None => exit_object_at_current_position(child)?,
        };
        if !moved {
            break;
        }
    }

    let definition = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_effective_definition_id(source))
    });
    let Some(definition) = definition else {
        return Ok(Value::Bool(false));
    };
    let components = resolve_component_list(&definition, Some(source), builder)?;
    let still_contained = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.get_world_object(source))
            .is_some_and(|object| object.container().is_some())
    });
    if still_contained {
        let _ = exit_object_at_current_position(source)?;
    }

    for (component, count) in components {
        for _ in 0..count {
            let rdir = draw_context_rnd3()?;
            let ydir = draw_context_rnd3()?;
            let xdir = draw_context_rnd3()?;
            let rotation = draw_context_random(360)?;
            let creation = HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let context = borrow.as_ref()?;
                let source_state = context.get_world_object(source)?;
                let owner = context
                    .object_scope(source)
                    .map(ObjectScopeContext::owner)
                    .unwrap_or(source_state.owner);
                let position = context
                    .object_scope(source)
                    .map(ObjectScopeContext::effective_position)
                    .unwrap_or(source_state.position);
                Some((owner, position))
            });
            let Some((owner, position)) = creation else {
                continue;
            };
            let Some(created) = create_native_object(NativeObjectCreation {
                definition: component.clone(),
                creator: Some(source),
                owner,
                controller: owner,
                construction: FULL_CON,
                position,
                rotation,
                velocity: FixedVec2::new(itofix(xdir), itofix(ydir)),
                rotation_velocity: itofix(rdir),
            })?
            else {
                continue;
            };
            let (burning, fire_owner) = with_host_context((false, OWNER_NONE), |context| {
                let Some(source_state) = context.get_world_object(source) else {
                    return (false, OWNER_NONE);
                };
                let owner = context
                    .object_scope(source)
                    .map(ObjectScopeContext::owner)
                    .unwrap_or(source_state.owner);
                let burning = context
                    .object_scope(source)
                    .and_then(|scope| scope.pending_update.staged_on_fire())
                    .or_else(|| source_state.full_state().map(|state| state.on_fire))
                    .unwrap_or(false);
                (burning, owner)
            });
            if burning {
                let _ = incinerate_target(created, fire_owner, false, None)?;
            }
            if let Some(container) = original_container {
                let _ = enter_object_live(created, container)?;
            }
        }
    }
    let _ = assign_removal_live(source, false)?;
    Ok(Value::Bool(true))
}

#[derive(Default)]
pub(crate) struct ObjectValueReflection {
    primitives: Vec<(Vec<&'static str>, Value)>,
}

impl ObjectValueReflection {
    fn push(&mut self, path: &[&'static str], value: Value) {
        self.primitives.push((path.to_vec(), value));
    }

    fn push_ints(&mut self, path: &[&'static str], values: impl IntoIterator<Item = i32>) {
        for value in values {
            self.push(path, Value::Int(value));
        }
    }

    /// StdArrayDefaultAdapt removes only the trailing default-valued slots.
    fn push_trimmed_ints(&mut self, path: &[&'static str], mut values: Vec<i32>) {
        while values.last() == Some(&0) {
            values.pop();
        }
        self.push_ints(path, values);
    }

    fn push_fixed(&mut self, path: &[&'static str], value: C4Fixed) {
        // C4Fixed::CompileFunc emits its format character before the raw
        // signed 16.16 payload (Fixed.h:248-265).
        self.push(path, Value::String("F".into()));
        self.push(path, Value::Int(value.val()));
    }

    pub(crate) fn get(&self, entry: &str, section: Option<&str>, entry_nr: i32) -> Option<Value> {
        let mut remaining = usize::try_from(entry_nr).ok()?;
        for (path, value) in &self.primitives {
            let matches = match section {
                Some(section) => {
                    path.len() >= 2
                        && path[path.len() - 2] == section
                        && path[path.len() - 1] == entry
                }
                None => path.last().is_some_and(|name| *name == entry),
            };
            if !matches {
                continue;
            }
            if remaining == 0 {
                return Some(value.clone());
            }
            remaining -= 1;
        }
        None
    }
}

pub(crate) fn push_reflected_c4value(
    reflection: &mut ObjectValueReflection,
    path: &[&'static str],
    value: &Value,
) {
    let (kind, payload) = match value {
        Value::Nil => ("A", 0),
        Value::Int(value) => ("i", *value),
        Value::Bool(value) => ("b", i32::from(*value)),
        Value::RawBool(value) => ("b", *value as u32 as i32),
        Value::C4Id(value) if cast_c4id_payload(value) == 0 => ("A", 0),
        Value::C4Id(value) => ("I", cast_c4id_payload(value) as i32),
        Value::Object(value) if *value == 0 => ("A", 0),
        Value::Object(value) => ("O", *value as i32),
        Value::String(value) => ("S", value.enum_id()),
        Value::Array(values) => {
            reflection.push(path, Value::String("a".into()));
            reflection.push(path, Value::Int(values.len() as i32));
            for value in values {
                push_reflected_c4value(reflection, path, value);
            }
            return;
        }
        Value::Proplist(values) => {
            reflection.push(path, Value::String("m".into()));
            reflection.push(path, Value::Int(values.len() as i32));
            for (key, value) in values {
                push_reflected_c4value(reflection, path, key);
                push_reflected_c4value(reflection, path, value);
            }
            return;
        }
    };
    reflection.push(path, Value::String(kind.to_string().into()));
    reflection.push(path, Value::Int(payload));
}

pub(crate) fn reflected_object_mass(
    context: &EffectHostContext,
    target: ObjectId,
    visited: &mut HashSet<ObjectId>,
) -> i32 {
    if !visited.insert(target) {
        return 0;
    }
    let scope = context.object_scope(target);
    let Some(object) = context.get_world_object(target) else {
        visited.remove(&target);
        return 1;
    };
    let state = object.full_state();
    let definition = scope
        .and_then(|scope| {
            scope
                .pending_update
                .change_def
                .as_deref()
                .or(scope.definition_id.as_deref())
        })
        .unwrap_or_else(|| object.definition_id());
    let Some(metadata) = context.definition_metadata(definition) else {
        visited.remove(&target);
        return 1;
    };
    let own_mass = scope
        .map(ObjectScopeContext::own_mass)
        .or_else(|| state.map(|state| state.own_mass))
        .unwrap_or(0);
    let construction = scope
        .map(ObjectScopeContext::construction)
        .or_else(|| state.map(|state| state.construction))
        .unwrap_or(FULL_CON);
    let mut mass = metadata
        .mass
        .saturating_add(own_mass)
        .saturating_mul(construction)
        / FULL_CON;
    mass = mass.max(1);
    if !metadata.no_component_mass {
        for content in object.contents() {
            if context.object_status_present(*content) {
                mass = mass.saturating_add(reflected_object_mass(context, *content, visited));
            }
        }
    }
    visited.remove(&target);
    mass
}

fn reflected_object_locals(
    context: &EffectHostContext,
    target: ObjectId,
    state: Option<&ObjectState>,
    scope: Option<&ObjectScopeContext>,
) -> HashMap<String, Value> {
    let mut locals = state
        .map(|state| state.local_vars.snapshot())
        .unwrap_or_default();
    if let Some(nested) = context.nested_objects.get(&target) {
        locals.extend(nested.local_vars.clone());
    }
    if let Some(update) = scope.and_then(|scope| scope.pending_update.local_vars.as_ref()) {
        locals.extend(update.clone());
    }
    if let Some(cells) = context.session_local_cells.get(&target) {
        locals.extend(cells.snapshot());
    }
    context.overlay_foreign_cells(target, &mut locals);
    locals
}

fn reflect_object_values(
    context: &EffectHostContext,
    target: ObjectId,
) -> Option<ObjectValueReflection> {
    let scope = context.object_scope(target);
    let world_object = context.get_world_object(target);
    if scope.is_none() && world_object.is_none() {
        return None;
    }
    let state = world_object.as_ref().and_then(|object| object.full_state());
    let definition_id = scope
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
        });
    let metadata = definition_id
        .as_deref()
        .and_then(|definition| context.definition_metadata(definition));
    let position = scope
        .map(ObjectScopeContext::effective_position)
        .or_else(|| world_object.as_ref().map(HostWorldObject::position))
        .unwrap_or_default();
    let fixed_position = scope
        .map(ObjectScopeContext::fixed_position)
        .or_else(|| world_object.as_ref().map(|object| object.fixed_position))
        .unwrap_or_else(|| FixedVec2::from_ints(position.x, position.y));
    let fixed_velocity = scope
        .map(ObjectScopeContext::fixed_velocity)
        .or_else(|| world_object.as_ref().map(|object| object.fixed_velocity))
        .unwrap_or_default();
    let rotation = scope
        .map(ObjectScopeContext::rotation)
        .or_else(|| world_object.as_ref().map(|object| object.rotation))
        .unwrap_or(0);
    let fixed_rotation = scope
        .map(ObjectScopeContext::fixed_rotation)
        .or_else(|| world_object.as_ref().map(|object| object.fixed_rotation))
        .unwrap_or_else(|| itofix(rotation));
    let rotation_velocity = scope
        .map(ObjectScopeContext::rotation_velocity)
        .or_else(|| world_object.as_ref().map(|object| object.rotation_velocity))
        .unwrap_or_default();
    let shape = live_object_shape(context, target).unwrap_or_default();
    let shape_vertices = scope
        .map(ObjectScopeContext::shape_vertex_buffer)
        .or_else(|| state.map(|state| state.shape_vertices.clone()))
        .unwrap_or_default();
    let local_vars = reflected_object_locals(context, target, state.map(Rc::as_ref), scope);
    let compiler_cache = scope
        .map(|scope| &scope.current_compiler_cache)
        .or_else(|| world_object.as_ref().map(|object| &object.compiler_cache))
        .cloned()
        .unwrap_or_default();

    let mut reflection = ObjectValueReflection::default();
    let object_path = |name| ["Object", name];
    reflection.push(
        &object_path("id"),
        definition_id
            .clone()
            .filter(|id| !id.is_empty())
            .map(Value::C4Id)
            .unwrap_or(Value::Nil),
    );
    if let Some(name) = context.object_custom_name(target) {
        reflection.push(&object_path("Name"), Value::String(name.into()));
    }
    reflection.push(&object_path("Number"), Value::Int(target.as_u64() as i32));
    reflection.push(
        &object_path("Status"),
        Value::Int(
            scope
                .map(ObjectScopeContext::status)
                .or_else(|| world_object.as_ref().map(HostWorldObject::status))
                .unwrap_or(ObjectStatus::Deleted)
                .to_script_value(),
        ),
    );
    // C4Object::nInfo is a stale compiler cache. Pointer enumeration refreshes
    // it, but ordinary Info changes do not.
    reflection.push(
        &object_path("Info"),
        Value::String(compiler_cache.info.clone().into()),
    );
    reflection.push(
        &object_path("Owner"),
        Value::Int(
            scope
                .map(ObjectScopeContext::owner)
                .or_else(|| state.map(|state| state.owner))
                .or_else(|| world_object.as_ref().map(HostWorldObject::owner))
                .unwrap_or(OWNER_NONE),
        ),
    );
    reflection.push(
        &object_path("Timer"),
        Value::Int(state.map(|state| state.timer).unwrap_or(0)),
    );
    reflection.push(
        &object_path("Controller"),
        Value::Int(
            scope
                .map(ObjectScopeContext::controller)
                .or_else(|| state.map(|state| state.controller))
                .or_else(|| world_object.as_ref().map(HostWorldObject::controller))
                .unwrap_or(OWNER_NONE),
        ),
    );
    reflection.push(
        &object_path("LastEngLossPlr"),
        Value::Int(
            scope
                .and_then(|scope| scope.pending_update.energy_loss_cause)
                .or_else(|| {
                    world_object
                        .as_ref()
                        .map(|object| object.last_energy_loss_cause)
                })
                .unwrap_or(OWNER_NONE),
        ),
    );
    reflection.push(
        &object_path("Category"),
        Value::Int(
            scope
                .map(ObjectScopeContext::category)
                .or_else(|| state.map(|state| state.category))
                .or_else(|| world_object.as_ref().map(HostWorldObject::category))
                .unwrap_or(0),
        ),
    );
    reflection.push(&object_path("X"), Value::Int(position.x));
    reflection.push(&object_path("Y"), Value::Int(position.y));
    reflection.push(&object_path("Rotation"), Value::Int(rotation));
    reflection.push(
        &object_path("MotionX"),
        Value::Int(world_object.as_ref().map_or(0, |object| object.motion_x)),
    );
    reflection.push(
        &object_path("MotionY"),
        Value::Int(world_object.as_ref().map_or(0, |object| object.motion_y)),
    );
    reflection.push(
        &object_path("LastSolidAtchFrame"),
        Value::Int(
            world_object
                .as_ref()
                .map_or(-1, |object| object.last_attach_movement_frame),
        ),
    );
    reflection.push(
        &object_path("NoCollectDelay"),
        Value::Int(
            scope
                .map(ObjectScopeContext::no_collect_delay)
                .or_else(|| state.map(|state| state.no_collect_delay))
                .or_else(|| world_object.as_ref().map(|object| object.no_collect_delay))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("Base"),
        Value::Int(state.map(|state| state.base).unwrap_or(OWNER_NONE)),
    );
    let construction = scope
        .map(ObjectScopeContext::construction)
        .or_else(|| state.map(|state| state.construction))
        .or_else(|| world_object.as_ref().map(HostWorldObject::construction))
        .unwrap_or(0);
    reflection.push(&object_path("Size"), Value::Int(construction));
    reflection.push(
        &object_path("OwnMass"),
        Value::Int(
            scope
                .map(ObjectScopeContext::own_mass)
                .or_else(|| state.map(|state| state.own_mass))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("Mass"),
        Value::Int(reflected_object_mass(context, target, &mut HashSet::new())),
    );
    reflection.push(
        &object_path("Damage"),
        Value::Int(
            scope
                .map(ObjectScopeContext::damage)
                .or_else(|| state.map(|state| state.damage))
                .or_else(|| world_object.as_ref().map(HostWorldObject::damage))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("Energy"),
        Value::Int(
            scope
                .map(ObjectScopeContext::energy)
                .or_else(|| state.map(|state| state.energy))
                .or_else(|| world_object.as_ref().map(HostWorldObject::energy))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("MagicEnergy"),
        Value::Int(
            scope
                .map(ObjectScopeContext::magic_energy)
                .or_else(|| state.map(|state| state.magic_energy))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("Alive"),
        Value::Bool(
            scope
                .map(ObjectScopeContext::alive)
                .or_else(|| state.map(|state| state.alive))
                .or_else(|| world_object.as_ref().map(HostWorldObject::alive))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("Breath"),
        Value::Int(
            scope
                .map(ObjectScopeContext::breath)
                .or_else(|| state.map(|state| state.breath))
                .unwrap_or(0),
        ),
    );
    let fire_phase = scope
        .and_then(|scope| scope.pending_update.fire.map(|(_, phase)| phase))
        .or_else(|| state.map(|state| state.fire_phase))
        .unwrap_or(0);
    reflection.push(&object_path("FirePhase"), Value::Int(fire_phase));
    let color = scope
        .and_then(|scope| scope.pending_update.color)
        .or_else(|| state.map(|state| state.color))
        .unwrap_or(0);
    reflection.push(&object_path("Color"), Value::Int(color as i32));
    reflection.push(&object_path("ColorDw"), Value::Int(color as i32));

    let numbered_size = local_vars
        .keys()
        .filter_map(|name| name.strip_prefix("__local_")?.parse::<usize>().ok())
        .max()
        .map_or(0, |index| index.saturating_add(1));
    reflection.push(&object_path("Locals"), Value::Int(numbered_size as i32));
    for index in 0..numbered_size {
        let value = local_vars
            .get(&format!("__local_{index}"))
            .unwrap_or(&Value::Nil);
        push_reflected_c4value(&mut reflection, &object_path("Locals"), value);
    }

    reflection.push_fixed(&object_path("FixX"), fixed_position.x);
    reflection.push_fixed(&object_path("FixY"), fixed_position.y);
    reflection.push_fixed(&object_path("FixR"), fixed_rotation);
    reflection.push_fixed(&object_path("XDir"), fixed_velocity.x);
    reflection.push_fixed(&object_path("YDir"), fixed_velocity.y);
    reflection.push_fixed(&object_path("RDir"), rotation_velocity);

    reflection.push(&object_path("Width"), Value::Int(shape.width));
    reflection.push(&object_path("Height"), Value::Int(shape.height));
    reflection.push_trimmed_ints(&object_path("Offset"), vec![shape.x, shape.y]);
    reflection.push(
        &object_path("Vertices"),
        Value::Int(shape_vertices.active_count() as i32),
    );
    reflection.push_trimmed_ints(
        &object_path("VertexX"),
        shape_vertices.slots.iter().map(|vertex| vertex.x).collect(),
    );
    reflection.push_trimmed_ints(
        &object_path("VertexY"),
        shape_vertices.slots.iter().map(|vertex| vertex.y).collect(),
    );
    reflection.push_trimmed_ints(
        &object_path("VertexCNAT"),
        shape_vertices
            .slots
            .iter()
            .map(|vertex| vertex.cnat as i32)
            .collect(),
    );
    reflection.push_trimmed_ints(
        &object_path("VertexFriction"),
        shape_vertices
            .slots
            .iter()
            .map(|vertex| vertex.friction)
            .collect(),
    );
    reflection.push(
        &object_path("ContactDensity"),
        Value::Int(
            scope
                .map(ObjectScopeContext::contact_density)
                .or_else(|| state.map(|state| state.contact_density))
                .or_else(|| world_object.as_ref().map(HostWorldObject::contact_density))
                .unwrap_or(crate::CONTACT_DENSITY_SOLID),
        ),
    );
    let fire_top = metadata.map_or(0, |metadata| {
        if metadata.line != 0 || construction == FULL_CON {
            metadata.fire.fire_top
        } else {
            let percent = construction.saturating_mul(100) / FULL_CON;
            metadata.fire.fire_top.saturating_mul(percent) / 100
        }
    });
    reflection.push(&object_path("FireTop"), Value::Int(fire_top));
    let attach = scope
        .map(|scope| scope.walk_rotation.attach)
        .or_else(|| state.map(|state| state.shape_attach))
        .unwrap_or_default();
    reflection.push(&object_path("AttachX"), Value::Int(attach.x));
    reflection.push(&object_path("AttachY"), Value::Int(attach.y));
    reflection.push(&object_path("AttachVtx"), Value::Int(attach.vtx));

    reflection.push(
        &object_path("OwnVertices"),
        Value::Bool(
            scope.is_some_and(|scope| scope.staged_own_vertices)
                || world_object
                    .as_ref()
                    .is_some_and(|object| object.own_vertices),
        ),
    );
    let solid_mask = scope
        .and_then(|scope| scope.pending_update.solid_mask_override)
        .or_else(|| state.and_then(|state| state.solid_mask_override))
        .or_else(|| {
            definition_id.as_ref().and_then(|definition| {
                context
                    .world
                    .solid_mask_metadata
                    .get(definition)
                    .and_then(|metadata| metadata.default_mask)
            })
        })
        .unwrap_or_else(|| crate::DefinitionTargetRect::new(0, 0, 0, 0, 0, 0));
    reflection.push_ints(
        &object_path("SolidMask"),
        [
            solid_mask.x,
            solid_mask.y,
            solid_mask.width,
            solid_mask.height,
            solid_mask.target_x,
            solid_mask.target_y,
        ],
    );
    let picture = scope
        .and_then(|scope| scope.pending_update.picture_rect)
        .or_else(|| state.map(|state| state.picture_rect))
        .unwrap_or_default();
    reflection.push_ints(
        &object_path("Picture"),
        [picture.x, picture.y, picture.width, picture.height],
    );
    reflection.push(
        &object_path("Mobile"),
        Value::Bool(
            scope
                .map(ObjectScopeContext::mobile)
                .or_else(|| state.map(|state| state.mobile))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("Selected"),
        Value::Bool(
            scope
                .map(ObjectScopeContext::selected)
                .or_else(|| state.map(|state| state.selected))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("OnFire"),
        Value::Bool(
            scope
                .and_then(|scope| scope.pending_update.staged_on_fire())
                .or_else(|| state.map(|state| state.on_fire))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("InLiquid"),
        Value::Bool(
            scope
                .map(ObjectScopeContext::in_liquid)
                .or_else(|| state.map(|state| state.in_liquid))
                .or_else(|| world_object.as_ref().map(HostWorldObject::in_liquid))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("EntranceStatus"),
        Value::Bool(
            scope
                .and_then(|scope| scope.pending_update.entrance_status)
                .or_else(|| state.map(|state| state.entrance_status))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("PhysicalTemporary"),
        Value::Bool(
            scope
                .map(|scope| scope.temporary_physical.is_some())
                .or_else(|| state.map(|state| state.temporary_physical.is_some()))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("NeedEnergy"),
        Value::Bool(
            scope
                .map(ObjectScopeContext::need_energy)
                .or_else(|| state.map(|state| state.need_energy))
                .or_else(|| world_object.as_ref().map(|object| object.need_energy))
                .unwrap_or(false),
        ),
    );
    reflection.push(
        &object_path("OCF"),
        Value::Int(
            scope
                .map(|scope| scope.staged_ocf(scope.ocf()))
                .or_else(|| world_object.as_ref().map(HostWorldObject::ocf))
                .unwrap_or(0) as i32,
        ),
    );

    let action_name = scope
        .map(|scope| {
            if scope
                .pending_update
                .action
                .as_ref()
                .and_then(|action| action.name.as_ref())
                .is_some()
            {
                if scope.effective_action_index().is_none()
                    && crate::action::is_builtin_idle_name(scope.effective_action_name())
                {
                    String::new()
                } else {
                    scope.effective_action_name().to_string()
                }
            } else {
                state
                    .map(|state| state.action.compiled_name().to_string())
                    .unwrap_or_else(|| {
                        if scope.effective_action_index().is_none()
                            && crate::action::is_builtin_idle_name(scope.effective_action_name())
                        {
                            String::new()
                        } else {
                            scope.effective_action_name().to_string()
                        }
                    })
            }
        })
        .or_else(|| state.map(|state| state.action.compiled_name().to_string()))
        .or_else(|| {
            world_object
                .as_ref()
                .map(|object| object.action_name.clone())
        })
        .unwrap_or_default();
    reflection.push(&object_path("Action"), Value::String(action_name.into()));
    reflection.push(
        &object_path("Dir"),
        Value::Int(
            scope
                .map(ObjectScopeContext::direction)
                .or_else(|| state.map(|state| state.direction))
                .unwrap_or_default()
                .to_script_value(),
        ),
    );
    reflection.push(
        &object_path("ComDir"),
        Value::Int(
            scope
                .map(ObjectScopeContext::command_direction)
                .or_else(|| state.map(|state| state.command_direction))
                .unwrap_or_default()
                .to_script_value(),
        ),
    );
    reflection.push(
        &object_path("ActionTime"),
        Value::Int(
            scope
                .map(|scope| scope.current_action_ticks)
                .or_else(|| state.map(|state| state.action.time))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("ActionData"),
        Value::Int(
            scope
                .map(ObjectScopeContext::effective_action_data)
                .or_else(|| state.map(|state| state.action.data))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("Phase"),
        Value::Int(
            scope
                .map(ObjectScopeContext::action_phase)
                .or_else(|| state.map(|state| state.action.phase))
                .unwrap_or(0),
        ),
    );
    let phase_delay = scope
        .and_then(|scope| {
            scope
                .pending_update
                .action
                .as_ref()
                .and_then(|action| action.ticks.or_else(|| action.name.as_ref().map(|_| 0)))
        })
        .or_else(|| state.map(|state| state.action.ticks))
        .unwrap_or(0);
    reflection.push(&object_path("PhaseDelay"), Value::Int(phase_delay));

    // These are C4EnumeratedObjectPtr::number, deliberately independent of
    // the resolved live pointers above. Denumeration preserves zero,
    // unresolved, negative, and legacy-offset words verbatim.
    reflection.push(
        &object_path("Contained"),
        Value::Int(compiler_cache.contained),
    );
    reflection.push(
        &object_path("ActionTarget1"),
        Value::Int(compiler_cache.action_target1),
    );
    reflection.push(
        &object_path("ActionTarget2"),
        Value::Int(compiler_cache.action_target2),
    );

    let components = scope
        .and_then(|scope| scope.pending_update.components.as_ref())
        .or_else(|| state.map(|state| &state.components));
    let component_order = scope
        .and_then(|scope| scope.pending_update.component_order.as_ref())
        .or_else(|| state.map(|state| &state.component_order));
    if let (Some(components), Some(order)) = (components, component_order) {
        for id in order {
            reflection.push(
                &object_path("Component"),
                if id.as_str().is_empty() {
                    Value::Nil
                } else {
                    Value::C4Id(id.as_str().to_string())
                },
            );
            reflection.push(
                &object_path("Component"),
                Value::Int(components.get(id).unwrap_or(0)),
            );
        }
    }
    if let Some(object) = world_object.as_ref() {
        for content in object.contents() {
            reflection.push(
                &object_path("Contents"),
                Value::Int(content.as_u64() as i32),
            );
        }
    }
    reflection.push(
        &object_path("PlrViewRange"),
        Value::Int(
            scope
                .map(ObjectScopeContext::plr_view_range)
                .or_else(|| state.map(|state| state.plr_view_range))
                .unwrap_or(0),
        ),
    );
    reflection.push(
        &object_path("Visibility"),
        Value::Int(
            scope
                .and_then(|scope| scope.pending_update.visibility)
                .or_else(|| state.map(|state| state.visibility))
                .unwrap_or(0),
        ),
    );
    let local_names = definition_id
        .as_deref()
        .and_then(|definition| context.world.definition_script(definition))
        .map(|script| {
            script
                .local_variable_names()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            let mut names = local_vars
                .keys()
                .filter(|name| !name.starts_with("__local_"))
                .cloned()
                .collect::<Vec<_>>();
            names.sort();
            names
        });
    reflection.push(
        &object_path("LocalNamed"),
        Value::Int(local_names.len() as i32),
    );
    for name in &local_names {
        reflection.push(
            &object_path("LocalNamed"),
            Value::String(name.clone().into()),
        );
        push_reflected_c4value(
            &mut reflection,
            &object_path("LocalNamed"),
            local_vars.get(name).unwrap_or(&Value::Nil),
        );
    }
    reflection.push(
        &object_path("ColorMod"),
        Value::Int(
            scope
                .and_then(|scope| scope.pending_update.color_modulation)
                .or_else(|| state.map(|state| state.color_modulation))
                .unwrap_or(0) as i32,
        ),
    );
    reflection.push(
        &object_path("BlitMode"),
        Value::Int(
            context
                .object_blit_mode(target)
                .or_else(|| state.map(|state| state.blit_mode))
                .unwrap_or(0) as i32,
        ),
    );
    reflection.push(
        &object_path("CrewDisabled"),
        Value::Bool(
            context
                .object_crew_disabled(target)
                .or_else(|| state.map(|state| state.crew_disabled))
                .unwrap_or(false),
        ),
    );
    reflection.push(&object_path("Layer"), Value::Int(compiler_cache.layer));
    let base_graphics = match scope {
        Some(scope) => scope.base_graphics.as_ref(),
        None => state.and_then(|state| state.base_graphics.as_ref()),
    };
    let graphics_definition = base_graphics
        .map(|graphics| graphics.definition.as_str())
        .or(definition_id.as_deref())
        .unwrap_or_default();
    let graphics_name = base_graphics
        .and_then(|graphics| graphics.graphics_name.as_deref())
        .unwrap_or_default();
    reflection.push(
        &object_path("Graphics"),
        Value::C4Id(graphics_definition.to_string()),
    );
    reflection.push(
        &object_path("Graphics"),
        Value::String(graphics_name.to_string().into()),
    );

    // PhysicalTemporary follows the Object root as a sibling section.
    let physical = match scope {
        Some(scope) => scope.temporary_physical,
        None => state.and_then(|state| state.temporary_physical),
    };
    if let Some(physical) = physical {
        for (name, value) in [
            ("Energy", physical.energy),
            ("Breath", physical.breath),
            ("Walk", physical.walk),
            ("Jump", physical.jump),
            ("Scale", physical.scale),
            ("Hangle", physical.hangle),
            ("Dig", physical.dig),
            ("Swim", physical.swim),
            ("Throw", physical.throw),
            ("Push", physical.push),
            ("Fight", physical.fight),
            ("Magic", physical.magic),
            ("Float", physical.float),
            ("CanScale", physical.can_scale),
            ("CanHangle", physical.can_hangle),
            ("CanDig", physical.can_dig),
            ("CanConstruct", physical.can_construct),
            ("CanChop", physical.can_chop),
            ("CanFly", physical.can_fly),
            ("CorrosionResist", physical.corrosion_resist),
            ("BreatheWater", physical.breathe_water),
        ] {
            reflection.push(&["Physical", name], Value::Int(value));
        }
    }

    Some(reflection)
}

/// FnGetObjectVal (C4Script.cpp:4184-4195): reflect the modeled live portion
/// of C4Object::CompileFunc's named primitive stream. Shape, Action and
/// Graphics fields are inline under Object; Physical is a sibling section.
pub(crate) fn get_object_val(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 4 {
        return Err(RuntimeError::new(
            "GetObjectVal expects at most 4 arguments",
        ));
    }
    let Some(entry) = parse_optional_string(args.first(), "GetObjectVal", "entry")? else {
        return Ok(Value::Nil);
    };
    let section = parse_optional_string(args.get(1), "GetObjectVal", "section")?
        .filter(|section| !section.is_empty());
    let target_arg = args.get(2).unwrap_or(&Value::Nil);
    let target = if matches!(target_arg, Value::Bool(false)) {
        None
    } else {
        parse_object_reference_argument(target_arg, "GetObjectVal", "target")?
    };
    let entry_nr = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "GetObjectVal",
        "entry_nr",
    )?;

    with_host_context(Ok(Value::Nil), |context| {
        let target = target.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        // These shipped hot paths run from movement callbacks every frame.
        // Their compiler paths are unique and inline directly under Object,
        // so avoid materializing the complete reflection stream (including
        // locals and recursive contents mass) just to read one shape scalar.
        if section.as_deref().is_none_or(|section| section == "Object") {
            if let Some(shape) = live_object_shape(context, target) {
                let value = match entry.as_str() {
                    "Width" if entry_nr == 0 => Some(Value::Int(shape.width)),
                    "Height" if entry_nr == 0 => Some(Value::Int(shape.height)),
                    "Offset" => {
                        let mut values = vec![shape.x, shape.y];
                        while values.last() == Some(&0) {
                            values.pop();
                        }
                        usize::try_from(entry_nr)
                            .ok()
                            .and_then(|index| values.get(index).copied())
                            .map(Value::Int)
                    }
                    _ => None,
                };
                if matches!(entry.as_str(), "Width" | "Height" | "Offset") {
                    return Ok(value.unwrap_or(Value::Nil));
                }
            }
        }
        Ok(reflect_object_values(context, target)
            .and_then(|reflection| reflection.get(&entry, section.as_deref(), entry_nr))
            .unwrap_or(Value::Nil))
    })
}

/// FnGetObjectInfoCoreVal (C4Script.cpp:4197-4214): reflect the linked
/// C4ObjectInfoCore rather than the object's current definition. In
/// particular, `id` remains the recruited crew's source definition across a
/// ChangeDef; System.c4g/Magic.c uses that value for physical-training caps.
pub(crate) fn get_object_info_core_val(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 4 {
        return Err(RuntimeError::new(
            "GetObjectInfoCoreVal expects at most 4 arguments",
        ));
    }
    let Some(entry) = parse_optional_string(args.first(), "GetObjectInfoCoreVal", "entry")? else {
        return Ok(Value::Nil);
    };
    let section = parse_optional_string(args.get(1), "GetObjectInfoCoreVal", "section")?
        .filter(|section| !section.is_empty());
    let target = parse_object_reference_argument(
        args.get(2).unwrap_or(&Value::Nil),
        "GetObjectInfoCoreVal",
        "target",
    )?;
    let entry_number = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "GetObjectInfoCoreVal",
        "entry number",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let target = target.or_else(|| context.object_context().map(|object| object.id()));
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        let physical = context
            .object_scope(target)
            .and_then(|scope| scope.info_physical)
            .or_else(|| {
                context
                    .get_world_object(target)
                    .and_then(|object| object.full_state().and_then(|state| state.info_physical))
            })
            .or_else(|| {
                let link = context.world.crew_info_link(target)?;
                context
                    .world
                    .crew_info_state
                    .borrow()
                    .entries
                    .get(&link)
                    .map(|entry| entry.physical)
            });
        let info = match context.object_scope(target) {
            Some(scope) => scope.info_core(),
            None => context.world.crew_infos.get(&target),
        };
        let Some(info) = info else {
            return Ok(Value::Nil);
        };
        if section
            .as_deref()
            .is_none_or(|section| section == "Physical")
        {
            if entry_number == 0 {
                if let Some(value) = physical.and_then(|physical| match entry.as_str() {
                    "Energy" => Some(physical.energy),
                    "Breath" => Some(physical.breath),
                    "Walk" => Some(physical.walk),
                    "Jump" => Some(physical.jump),
                    "Scale" => Some(physical.scale),
                    "Hangle" => Some(physical.hangle),
                    "Dig" => Some(physical.dig),
                    "Swim" => Some(physical.swim),
                    "Throw" => Some(physical.throw),
                    "Push" => Some(physical.push),
                    "Fight" => Some(physical.fight),
                    "Magic" => Some(physical.magic),
                    "Float" => Some(physical.float),
                    "CanScale" => Some(physical.can_scale),
                    "CanHangle" => Some(physical.can_hangle),
                    "CanDig" => Some(physical.can_dig),
                    "CanConstruct" => Some(physical.can_construct),
                    "CanChop" => Some(physical.can_chop),
                    "CanFly" => Some(physical.can_fly),
                    "CorrosionResist" => Some(physical.corrosion_resist),
                    "BreatheWater" => Some(physical.breathe_water),
                    _ => None,
                }) {
                    return Ok(Value::Int(value));
                }
            }
            if section.as_deref() == Some("Physical") {
                return Ok(Value::Nil);
            }
        }
        if section
            .as_deref()
            .is_some_and(|section| section != "ObjectInfo")
        {
            tracing::debug!(?section, %entry, "GetObjectInfoCoreVal section not modeled; nil");
            return Ok(Value::Nil);
        }
        if entry == "ExtraData" {
            let path = ["ObjectInfo", "ExtraData"];
            let mut reflection = ObjectValueReflection::default();
            reflection.push(
                &path,
                Value::Int(i32::try_from(info.extra_data.len()).unwrap_or(i32::MAX)),
            );
            for (name, value) in &info.extra_data {
                reflection.push(&path, Value::String(name.clone().into()));
                push_reflected_c4value(&mut reflection, &path, value);
            }
            return Ok(reflection
                .get(&entry, section.as_deref(), entry_number)
                .unwrap_or(Value::Nil));
        }
        if entry_number != 0 {
            return Ok(Value::Nil);
        }
        Ok(match entry.as_str() {
            "id" if info.definition_id.is_empty() => Value::Nil,
            "id" => Value::C4Id(info.definition_id.as_str().to_string()),
            "Name" => Value::String(info.name.clone().into()),
            "DeathMessage" => Value::String(
                active_death_message(&info.death_message)
                    .unwrap_or_default()
                    .into(),
            ),
            "PortraitFile" => Value::String(info.core.portrait_file.clone().into()),
            "Rank" => Value::Int(info.rank),
            "RankName" => Value::String(info.rank_name.clone().into()),
            "NextRankName" => Value::String(info.core.next_rank_name.clone().into()),
            "TypeName" => Value::String(info.core.type_name.clone().into()),
            "Participation" => Value::Int(info.participation),
            "Experience" => Value::Int(info.experience),
            "NextRankExp" => Value::Int(info.core.next_rank_exp),
            "Rounds" => Value::Int(info.rounds),
            "DeathCount" => Value::Int(info.death_count),
            "Birthday" => Value::Int(info.birthday),
            "TotalPlayingTime" => Value::Int(info.total_playing_time),
            "Age" => Value::Int(info.age),
            other => {
                tracing::debug!(entry = other, "GetObjectInfoCoreVal entry not modeled; nil");
                Value::Nil
            }
        })
    })
}

/// FnGetEntrance (C4Script.cpp:1125-1129): read the object's live
/// EntranceStatus as an integer, defaulting a nil target to cthr->Obj.
pub(crate) fn get_entrance(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetEntrance expects at most 1 argument: target",
        ));
    }

    let target_id = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GetEntrance",
        "target",
    )?;

    with_host_context(Ok(Value::Nil), |context| {
        let Some(target) = target_id.or(context.script_object_context) else {
            return Ok(Value::Nil);
        };

        let scope = context.object_scope(target);
        let world_object = context.get_world_object(target);
        if scope.is_none() && world_object.is_none() {
            return Ok(Value::Nil);
        }

        let enabled = scope
            .and_then(|scope| scope.pending_update.entrance_status)
            .or_else(|| {
                world_object
                    .as_ref()
                    .and_then(|object| object.full_state())
                    .map(|state| state.entrance_status)
            })
            .unwrap_or(false);
        Ok(Value::Int(i32::from(enabled)))
    })
}

/// FnSetEntrance (C4Script.cpp:690-695): toggle the object's EntranceStatus.
pub(crate) fn set_entrance(args: &[Value]) -> Result<Value, RuntimeError> {
    let enabled = args.first().unwrap_or(&Value::Nil).as_bool();
    let mut index = 1;
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetEntrance", "target")?;

    try_with_host_context_mut("SetEntrance requires an active engine context", |context| {
        let target = target_id.or(context.script_object_context);
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(
            context
                .object_scope_mut(target)
                .map(|object| object.pending_update.entrance_status = Some(enabled))
                .is_some(),
        ))
    })
}

pub(crate) fn no_container(_args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::Int(124))
}

pub(crate) fn any_container(_args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::Int(123))
}

pub(crate) fn set_category(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled iCategory is nil -> 0 (FnSetCategory, C4Script.cpp:805).
    let category = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetCategory",
        "category",
    )?;

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

    try_with_host_context_mut("SetCategory requires an active engine context", |context| {
        let target = target_id.or(context.script_object_context);
        let Some(target) = target else {
            return Ok(Value::Bool(false));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };

        object.set_category(category);
        object.unsorted = true;
        // C4Object::SetCategory immediately calls Resort(), which leaves the
        // link in place but arms the post-CrossCheck global unsorted sweep.
        context.record_object_order_command(ObjectOrderCommand::ResortObject(target));
        // SetCategory's trailing SetOCF is synchronous. In particular, a
        // category change after AssignDeath must replace its earlier final
        // cache before both the next statement and copy-out
        // (oracle-src-pinned src/C4Object.h:311).
        let _ = refresh_live_object_ocf(context, target);
        Ok(Value::Bool(true))
    })
}

/// FnSetObjectOrder (C4Script.cpp:5090-5111): queue a deferred main-list
/// resort. A nil sort object means the caller; invalid/self pairs fail.
pub(crate) fn set_object_order(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "SetObjectOrder expects at most 3 arguments: relative object, sort object and after flag",
        ));
    }
    let relative_to = args
        .first()
        .map(|value| parse_object_reference_argument(value, "SetObjectOrder", "relative object"))
        .transpose()?
        .flatten();
    let explicit_object = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetObjectOrder", "sort object"))
        .transpose()?
        .flatten();
    let after = args.get(2).map(Value::as_bool).unwrap_or(false);

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let object = explicit_object.or_else(|| context.object_context().map(|scope| scope.id()));
        let Some((relative_to, object)) = relative_to.zip(object) else {
            return Ok(Value::Bool(false));
        };
        if relative_to == object {
            return Ok(Value::Bool(false));
        }
        let resolves =
            |id| context.object_scope(id).is_some() || context.get_world_object(id).is_some();
        if !resolves(relative_to) || !resolves(object) {
            return Ok(Value::Bool(false));
        }
        context.record_object_order_command(ObjectOrderCommand::SetRelative {
            relative_to,
            object,
            after,
        });
        Ok(Value::Bool(true))
    })
}

/// Resolve the caller-local order function at queue time. C++ retains the
/// resulting `C4AulFunc *`; cloning the selected Function prevents a later
/// script-table replacement from changing which body the resort calls.
fn capture_object_order_function(
    context: &EffectHostContext,
    function: String,
) -> Result<Option<ObjectOrderFunction>, RuntimeError> {
    let Some(caller_host) = clonk_script::caller_host_identity() else {
        return Ok(None);
    };
    let caller_uses_engine_scope = clonk_script::caller_uses_engine_scope().unwrap_or(false);
    let Some((mut script_name, mut definition_context, script)) =
        context.world.script_for_host_identity(caller_host)
    else {
        return Ok(None);
    };
    let Some(resolution) = script.resolve_function(&function, caller_uses_engine_scope) else {
        return Err(RuntimeError::new(format!(
            "ResortObjects: Resort function {function} not found"
        )));
    };
    let engine_global = resolution.scope == clonk_script::ScriptFunctionScope::Global;
    let mut host_identity = script.host_identity();
    if engine_global {
        definition_context = None;
        // A global caller's LinkedTo host and the currently selected global
        // comparator need not match. C++ queues the selected C4AulFunc*, so
        // pin the comparator's own declaring host (and its local helpers),
        // not the caller's host. Detached clonk-script fixtures may provide a
        // global table without retaining its source Engine; their attached
        // caller VM remains the only executable host and is a safe fallback.
        if let Some((resolved_name, _, _)) = context
            .world
            .script_for_host_identity(resolution.host_identity)
        {
            script_name = resolved_name;
            host_identity = resolution.host_identity;
        }
    }
    Ok(Some(ObjectOrderFunction {
        host_identity,
        resolution,
        script_name,
        definition_context,
        function,
        engine_global,
    }))
}

/// FnResortObjects (C4Script.cpp:4318-4338): resolve a caller-local function
/// immediately and prepend a category-mask resort for post-CrossCheck work.
pub(crate) fn resort_objects(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "ResortObjects expects at most 2 arguments: function and category",
        ));
    }
    let Some(function) = parse_optional_string(args.first(), "ResortObjects", "function")? else {
        return Ok(Value::Bool(false));
    };
    let mut category = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "ResortObjects",
        "category",
    )?;
    if category == 0 {
        category = CATEGORY_SORT_LIMIT;
    }

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(order) = capture_object_order_function(context, function)? else {
            return Ok(Value::Bool(false));
        };
        context.record_object_order_command(ObjectOrderCommand::OrderFuncAll { order, category });
        Ok(Value::Bool(true))
    })
}

/// FnResortObject (C4Script.cpp:4340-4359): nil pObj defaults to cthr->Obj;
/// the same queue-time local-function resolution is retained for one object.
pub(crate) fn resort_object(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "ResortObject expects at most 2 arguments: function and object",
        ));
    }
    let Some(function) = parse_optional_string(args.first(), "ResortObject", "function")? else {
        return Ok(Value::Bool(false));
    };
    let explicit = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "ResortObject", "object"))
        .transpose()?
        .flatten();

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(object) = explicit.or(context.script_object_context) else {
            return Ok(Value::Bool(false));
        };
        let resolves =
            context.object_scope(object).is_some() || context.get_world_object(object).is_some();
        if !resolves {
            return Ok(Value::Bool(false));
        }
        let Some(order) = capture_object_order_function(context, function)? else {
            return Ok(Value::Bool(false));
        };
        context.record_object_order_command(ObjectOrderCommand::OrderFuncObject { order, object });
        Ok(Value::Bool(true))
    })
}

/// FnResort (C4Script.cpp:3543-3552): an explicit object wins, otherwise
/// `cthr->Obj` is used. Object resorts are deferred to the post-CrossCheck
/// phase; a call without either object performs the stable category sort.
pub(crate) fn resort(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Resort expects at most 1 argument: object",
        ));
    }
    let explicit = args
        .first()
        .map(|value| parse_object_reference_argument(value, "Resort", "object"))
        .transpose()?
        .flatten();

    with_host_context_mut(Ok(Value::Nil), |context| {
        let target = explicit.or(context.script_object_context);
        if let Some(target) = target {
            let resolves = context.object_scope(target).is_some()
                || context.get_world_object(target).is_some();
            if resolves {
                if context.ensure_object_scope(target) {
                    if let Some(scope) = context.object_scope_mut(target) {
                        scope.unsorted = true;
                    }
                }
                context.record_object_order_command(ObjectOrderCommand::ResortObject(target));
            }
        } else {
            context.preview_sort_master_by_category();
            context.record_object_order_command(ObjectOrderCommand::SortByCategory);
        }
        Ok(Value::Nil)
    })
}

pub(crate) fn remove_object(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "RemoveObject expects at most 2 arguments: target, eject contents",
        ));
    }

    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.first() {
        target_id = parse_object_reference_argument(arg, "RemoveObject", "target")?;
    }
    let exit_contents = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "RemoveObject",
        "eject contents",
    )?;

    // FnRemoveObject (C4Script.cpp:455-460): a nil target means the calling
    // object, and ANY object may be removed. AssignRemoval runs the complete
    // callback/effect/contents lifecycle synchronously.
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target_id {
        if Some(target) != active {
            if let Some(result) = call_world_object_function(
                target,
                "RemoveObject",
                &[Value::Nil, Value::Bool(exit_contents)],
            ) {
                return result;
            }
            // The nested seam cannot reach spawns of the SAME call (no
            // full state yet) — cancel the pending spawn directly. Its
            // number stays consumed, exactly like C++ where the object
            // existed and died (the GoldRush TRPR Recruitment temp,
            // Trapper.c4d/Script.c:19-25).
            let removed = with_host_context_mut(false, |context| {
                let last_position = context
                    .object_scope(target)
                    .map(|scope| scope.current_position)
                    .or_else(|| {
                        context
                            .get_world_object(target)
                            .map(|object| object.position)
                    });
                mark_object_status_deleted(context, target);
                let removed = context.cancel_pending_spawn(target);
                if removed {
                    retire_object_info_and_clear_references(context, target, last_position);
                }
                removed
            });
            if removed {
                clear_player_object_pointers_host(target);
                HOST_CONTEXT.with(|cell| {
                    if let Some(context) = cell.borrow_mut().as_mut() {
                        context.update_live_solid_mask(target, false);
                    }
                });
            }
            return Ok(Value::Bool(removed));
        }
    }

    let Some(target) = target_id.or(active) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(assign_removal_live(target, exit_contents)?))
}

pub(crate) fn set_object_status(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled iNewStatus is nil -> 0 = STATUS_Deleted rejection path
    // (FnSetObjectStatus, C4Script.cpp:5416-5428).
    let status_value = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetObjectStatus: expected int or nil for status, got {}",
                other.type_name()
            )));
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
    let target_id =
        consume_optional_object_reference_argument(args, &mut index, "SetObjectStatus", "target")?;

    let mut clear_pointers = false;
    if let Some(arg) = args.get(index) {
        match arg {
            Value::Bool(value) => {
                clear_pointers = *value;
                index += 1;
            }
            Value::Nil => {
                index += 1;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "SetObjectStatus: expected bool or nil for clear pointers, got {}",
                    other.type_name()
                )));
            }
        }
    }

    if index < args.len() {
        return Err(RuntimeError::new(
            "SetObjectStatus: additional arguments are not supported",
        ));
    }

    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target_id {
        if Some(target) != active {
            let target_status = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.get_world_object(target))
                    .map(|object| object.status)
            });
            match target_status {
                None | Some(ObjectStatus::Deleted) => return Ok(Value::Bool(false)),
                Some(current) if current == status => return Ok(Value::Bool(true)),
                Some(_) => {}
            }
            return match call_world_object_function(
                target,
                "SetObjectStatus",
                &[
                    Value::Int(status.to_script_value()),
                    Value::Nil,
                    Value::Bool(clear_pointers),
                ],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }

    let (success, clear_target, activate_target) = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetObjectStatus requires an active engine context")
        })?;
        let object_id = match context.object_context() {
            Some(object) => object.id(),
            None => return Ok((false, None, None)),
        };

        if let Some(target) = target_id {
            if target != object_id {
                return Ok((false, None, None));
            }
        }
        let current = context
            .object_scope(object_id)
            .map(ObjectScopeContext::status)
            .unwrap_or(ObjectStatus::Deleted);
        if current == ObjectStatus::Deleted {
            return Ok((false, None, None));
        }
        if current == status {
            return Ok((true, None, None));
        }
        if status == ObjectStatus::Inactive {
            // StatusDeactivate clears both front and back particle lists
            // before it leaves the active object list.
            context.register_particle(ParticleCommand::Clear {
                definition_id: None,
                scope: ParticleScope::Object(object_id),
            });
        }
        if let Some(object) = context.object_scope_mut(object_id) {
            object.set_status(status);
        }
        context.preview_object_status_change(object_id, status);
        if status == ObjectStatus::Normal {
            let metadata = context
                .object_effective_definition_id(object_id)
                .and_then(|definition_id| context.definition_metadata(&definition_id).cloned())
                .unwrap_or_default();
            if let Some(object) = context.object_scope_mut(object_id) {
                if metadata.line == 0 {
                    object.pending_update.shape_override = Some(None);
                }
                object.refresh_shape_preview(&metadata);
            }
            // StatusActivate::UpdateFace(true) still performs the ordinary
            // UpdateSolidMask remove/re-put before UpdateTransferZone.
            context.preview_live_object_sector(object_id);
            context.update_live_solid_mask(object_id, false);
        }
        if status == ObjectStatus::Inactive && !clear_pointers {
            // The no-clear branch only clears transfer zones. Clear-mode
            // does this later as part of Game.ClearPointers, after every
            // Ejection and Departure callback has completed.
            context.register_transfer_zone_command(TransferZoneCommand::clear(object_id));
        }
        Ok((
            true,
            (status == ObjectStatus::Inactive && clear_pointers).then_some(object_id),
            (status == ObjectStatus::Normal).then_some(object_id),
        ))
    })?;
    if !success {
        return Ok(Value::Bool(false));
    }
    if let Some(object_id) = clear_target {
        // ClearContentsAndContained precedes Game.ClearPointers. In
        // particular, callbacks must still observe action/command pointers
        // and may create a transfer zone that the later sweep removes.
        clear_contents_and_contained_live(object_id, true)?;
        with_host_context_mut((), |context| {
            context.clear_object_action_and_command_pointers(object_id);
        });
        clear_player_object_pointers_host(object_id);
        with_host_context_mut((), |context| {
            let still_member = context.object_in_any_crew(object_id);
            if let Some(object) = context.object_scope_mut(object_id) {
                object.set_crew_status_member(still_member);
            }
            context.record_crew_rosters();
            context.register_transfer_zone_command(TransferZoneCommand::clear(object_id));
        });
    }
    if let Some(object_id) = activate_target {
        // StatusActivate's final operation is the fail-safe own-script
        // UpdateTransferZone callback, after re-listing and UpdatePos.
        let _ = call_inflight_object_own_fail_safe(object_id, "UpdateTransferZone", &[]);
    }
    Ok(Value::Bool(true))
}

pub(crate) fn get_object_status(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetObjectStatus expects at most 1 argument",
        ));
    }

    let target_id = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GetObjectStatus",
        "target",
    )?;

    with_host_context(Ok(Value::Nil), |context| {
        if let Some(target) = target_id {
            if let Some(object) = context.object_scope(target) {
                return Ok(Value::Int(object.status().to_script_value()));
            }
            return Ok(context
                .get_world_object(target)
                .map(|object| Value::Int(object.status().to_script_value()))
                .unwrap_or(Value::Nil));
        }

        let object = match context.object_context() {
            Some(object) => object,
            None => return Ok(Value::Nil),
        };
        Ok(Value::Int(object.status().to_script_value()))
    })
}

pub(crate) fn parse_timer_from_int(value: i32) -> Result<i32, RuntimeError> {
    if value < 0 {
        Err(RuntimeError::new(
            "AddEffect: timer must be >= 0 when provided",
        ))
    } else {
        Ok(value)
    }
}

pub(crate) fn clear_removed_object_references(value: &mut Value, removed: &HashSet<ObjectId>) {
    match value {
        Value::Object(id) if removed.contains(&ObjectId::new(*id)) => *value = Value::Nil,
        Value::Array(values) => {
            for value in values {
                clear_removed_object_references(value, removed);
            }
        }
        Value::Proplist(entries) => {
            let previous = std::mem::take(entries);
            let mut rebuilt = ValueMap::with_capacity(previous.len());
            for (mut key, mut value) in previous {
                if is_removed_object_value(&key, removed)
                    || is_removed_object_value(&value, removed)
                {
                    continue;
                }
                clear_removed_object_references(&mut key, removed);
                clear_removed_object_references(&mut value, removed);
                rebuilt.insert_key(key, value);
            }
            *entries = rebuilt;
        }
        _ => {}
    }
}
