use super::*;
use clonk_core::log_target::SCRIPT_LOG_TARGET;
// `HostObjectContext::new` is the tests' positional constructor; the
// engine builds scopes through `with_category`.
#[cfg(test)]
use crate::ActionLibrary;

/// Parameter-conversion policy for the immediate scripted C4Effect callback.
/// C++ passes `nonStrict3WarnConversionOnly` to Fx callbacks; the marker must
/// never leak into nested or ordinary script invocations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EffectCallbackParameterConversionPolicy {
    Standard,
    WarnForNonStrict3,
}

struct FairCrewHostContextState {
    script_object: Option<ObjectId>,
    script_definition: Option<Option<DefinitionId>>,
    definition: Option<DefinitionId>,
}

struct FairCrewDefinitionContextGuard {
    previous: Option<(DefinitionId, PhysicalInfo)>,
    host: Option<FairCrewHostContextState>,
}

impl Drop for FairCrewDefinitionContextGuard {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            HOST_CONTEXT.with(|cell| {
                let mut borrow = cell.borrow_mut();
                let context = borrow
                    .as_mut()
                    .expect("fair-crew host context must remain installed");
                context.object = context.dormant_scopes.pop().unwrap_or(None);
                context.script_object_context = host.script_object;
                context.script_definition_context = host.script_definition;
                context.definition_context = host.definition;
            });
        }
        FAIR_CREW_DEFINITION_CONTEXT.with(|cell| {
            *cell.borrow_mut() = self.previous.take();
        });
    }
}

pub(crate) fn with_fair_crew_definition_context<T>(
    definition: DefinitionId,
    physical: PhysicalInfo,
    call: impl FnOnce() -> T,
) -> T {
    let previous = FAIR_CREW_DEFINITION_CONTEXT
        .with(|cell| cell.borrow_mut().replace((definition.clone(), physical)));
    // C4PhysicalInfo::PromotionUpdate invokes the definition callback with
    // cthr->Obj=null and cthr->Def set to the physical's definition. Move
    // the suspended object scope, rather than cloning it, so explicit-object
    // natives in the hook still reach and mutate that one live scope.
    let host = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let active = context.object.take();
        context.dormant_scopes.push(active);
        Some(FairCrewHostContextState {
            script_object: context.script_object_context.take(),
            script_definition: context
                .script_definition_context
                .replace(Some(definition.clone())),
            definition: context.definition_context.replace(definition),
        })
    });
    let _guard = FairCrewDefinitionContextGuard { previous, host };
    call()
}

pub(crate) fn fair_crew_definition_context() -> Option<(DefinitionId, PhysicalInfo)> {
    let active = FAIR_CREW_DEFINITION_CONTEXT.with(|cell| cell.borrow().clone())?;
    // The definition callback itself has cthr->Obj=null and cthr->Def set to
    // the fair-crew definition. Nested object, global, or other-definition
    // frames must use their own ordinary GetID/GetPhysical context instead.
    let direct_definition_frame = HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().is_none_or(|context| {
            context.script_object_context.is_none()
                && context
                    .script_definition_context
                    .as_ref()
                    .is_some_and(|definition| definition.as_ref() == Some(&active.0))
        })
    });
    direct_definition_frame.then_some(active)
}

impl WorldAccessor for EffectHostContext {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get_world_object(id)
    }

    fn matches_find_condition_candidate(
        &self,
        id: ObjectId,
        condition: &FindCondition,
    ) -> Option<bool> {
        if self.object_scope(id).is_some() {
            let object = self.get_world_object(id)?;
            return Some(object.status().is_active() && condition.check(self, &object).ok()?);
        }
        if let Some(object) = self.pending_objects.get(&id) {
            return Some(object.status().is_active() && condition.check(self, object).ok()?);
        }
        self.world.matches_find_condition_candidate(id, condition)
    }

    fn matches_find_condition_scalar_prefix(
        &self,
        id: ObjectId,
        condition: &FindCondition,
    ) -> Option<bool> {
        if self.object_scope(id).is_some() {
            let object = self.get_world_object(id)?;
            return if object.status().is_active() {
                condition.matches_host_object(&object)
            } else {
                Some(false)
            };
        }
        if let Some(object) = self.pending_objects.get(&id) {
            return if object.status().is_active() {
                condition.matches_host_object(object)
            } else {
                Some(false)
            };
        }
        self.world
            .matches_find_condition_scalar_prefix(id, condition)
    }

    fn matches_legacy_find_object_candidate(
        &self,
        id: ObjectId,
        params: &FindObjectParams,
    ) -> Option<bool> {
        // Active/dormant/nested scopes carry same-call SetAction, SetOwner,
        // Enter/Exit, ChangeDef, Status and OCF writes. Keep using the full
        // overlay for those few candidates; only untouched engine objects
        // may be tested directly by the lazy provider.
        if self.object_scope(id).is_some() {
            return self
                .get_world_object(id)
                .map(|object| params.matches_object(&object));
        }
        if let Some(object) = self.pending_objects.get(&id) {
            return Some(params.matches_object(object));
        }
        self.world.matches_legacy_find_object_candidate(id, params)
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.world_object_ids()
    }

    fn master_object_ids(&self) -> Vec<ObjectId> {
        EffectHostContext::master_object_ids(self)
    }

    fn script_function_known(&self, name: &str) -> bool {
        self.world.script_function_known(name)
    }

    fn object_live_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        effect_object_live_shape_rect(self, object)
    }

    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        sector_shape_rect(self.object_live_shape_rect(object))
    }

    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata> {
        EffectHostContext::definition_metadata(self, id).cloned()
    }

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        let mut ids = self.world.object_sector_ids_in_rect(rect)?;
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();
        for &id in &self.pending_order {
            let Some(object) = self.get_world_object(id) else {
                continue;
            };
            if rect.contains_point(object.position.x, object.position.y) && seen.insert(id) {
                ids.push(id);
            }
        }
        Some(ids)
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        let mut ids = self.world.shape_sector_ids_in_rect(rect)?;
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();
        for &id in &self.pending_order {
            let Some(object) = self.get_world_object(id) else {
                continue;
            };
            if self.object_shape_rect(&object).overlaps(&rect) && seen.insert(id) {
                ids.push(id);
            }
        }
        Some(ids)
    }

    fn object_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>> {
        let mut lists = self.world.object_sector_id_lists_in_rect(rect)?;
        let mut seen = lists.iter().flatten().copied().collect::<HashSet<_>>();
        let pending: Vec<ObjectId> = self
            .pending_order
            .iter()
            .copied()
            .filter(|&id| {
                self.get_world_object(id)
                    .is_some_and(|object| rect.contains_point(object.position.x, object.position.y))
                    && seen.insert(id)
            })
            .collect();
        if !pending.is_empty() {
            lists.push(pending);
        }
        Some(lists)
    }

    fn shape_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>> {
        let mut lists = self.world.shape_sector_id_lists_in_rect(rect)?;
        let mut seen = lists.iter().flatten().copied().collect::<HashSet<_>>();
        let pending: Vec<ObjectId> = self
            .pending_order
            .iter()
            .copied()
            .filter(|&id| {
                self.get_world_object(id)
                    .is_some_and(|object| self.object_shape_rect(&object).overlaps(&rect))
                    && seen.insert(id)
            })
            .collect();
        if !pending.is_empty() {
            lists.push(pending);
        }
        Some(lists)
    }
}

/// `C4GameScriptHost::GRBroadcast` (C4ScriptHost.cpp:234-248): live
/// goal/rule/environment objects in forward master-list order, followed by
/// the scenario script. Rejection broadcasts stop at the first truthy
/// result. Both hostility and team-switch callbacks use this exact path.
pub(crate) fn broadcast_global_callback(
    function: &str,
    args: &[Value],
    reject_test: bool,
) -> Result<Value, RuntimeError> {
    const BROADCAST_MASK: i32 = (1 << 5) | (1 << 6) | (1 << 19);
    let targets = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(EffectHostContext::master_object_ids)
            .unwrap_or_default()
    });
    for target in targets {
        let eligible = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(target))
                .is_some_and(|object| {
                    object.status().is_active() && object.category() & BROADCAST_MASK != 0
                })
        });
        if !eligible {
            continue;
        }
        if let Some(result) = call_world_object_own_function(target, function, args) {
            let value = result?;
            if reject_test && value_raw_truthy(&value) {
                return Ok(value);
            }
        }
    }

    let script = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.scenario_script().cloned())
    });
    match script {
        Some(script) => {
            call_scoped_scenario_function(script, function, args).unwrap_or(Ok(Value::Nil))
        }
        None => Ok(Value::Nil),
    }
}

/// Run one creatorless engine-side object creation while preserving the
/// calling script object's live scope. `Game.CreateObject(id, nullptr)` does
/// not inherit the script caller's position, owner, layer, or controller.
pub(crate) fn with_creatorless_object_context<T>(
    callback: impl FnOnce() -> T,
) -> Result<T, RuntimeError> {
    let calling_object = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("creatorless object creation requires an active engine context")
        })?;
        Ok(context.object.take())
    })?;

    let result = callback();

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("host context disappeared during creatorless object creation")
        })?;
        debug_assert!(
            context.object.is_none(),
            "creatorless nested creation must restore its empty active scope"
        );
        context.object = calling_object;
        Ok(())
    })?;
    Ok(result)
}

/// The active object of the executing call (`cthr->Obj`).
pub(crate) fn active_object_id() -> Option<ObjectId> {
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    })
}

pub(crate) fn call_object_own_fail_safe(target: ObjectId, function: &str, args: &[Value]) -> Value {
    match call_world_object_own_function(target, function, args) {
        Some(Ok(value)) => value,
        Some(Err(error)) => {
            tracing::error!(
                %error,
                object = target.as_u64(),
                callback = function,
                "script error in object callback; continuing like C++ fail-safe Call"
            );
            log_runtime_call_frames("", error.call_frames());
            Value::Nil
        }
        None => Value::Nil,
    }
}

/// Fail-safe own-script call that also accepts an already-active object
/// scope which has not joined the world list yet. Plain Engine spawns run
/// Construction/Initialize in exactly that pre-insertion state.
pub(crate) fn call_inflight_object_own_fail_safe(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Value {
    match call_world_object_own_function_inflight(target, function, args) {
        Some(Ok(value)) => value,
        Some(Err(error)) => {
            tracing::error!(
                %error,
                object = target.as_u64(),
                callback = function,
                "script error in object callback; continuing like C++ fail-safe Call"
            );
            log_runtime_call_frames("", error.call_frames());
            Value::Nil
        }
        None => Value::Nil,
    }
}

/// FnCall (C4Script.cpp:3424-3432): `Call(name, p0..p8)` runs `name` on the
/// calling object itself — `C4Object::Call` (C4Object.cpp:2197-2201) → the
/// object's own def script, script functions ONLY (owner-scoped GetSFunc,
/// C4Aul.cpp:295-298,562-576; engine functions are never found). Nil name,
/// no object context, or a removed `this` → C4VNull; callee errors
/// propagate (fPassErrors=true); script pars[1..=9] shift to Par(0..=8).
pub(crate) fn call_self(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(name)) = args.first() else {
        return Ok(Value::Nil);
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let target = HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().and_then(|context| {
            context
                .object_context()
                .filter(|scope| !scope.destroy && scope.status() != ObjectStatus::Deleted)
                .map(ObjectScopeContext::id)
        })
    });
    let Some(target) = target else {
        return Ok(Value::Nil);
    };
    let pars: Vec<Value> = args.iter().skip(1).take(9).cloned().collect();
    call_world_object_script_function(target, name, &pars).unwrap_or(Ok(Value::Nil))
}

/// FnObjectCall/FnProtectedCall/FnPrivateCall (C4Script.cpp:3434-3449,
/// 3502-3534): run a function on a target object's def script — failsafe
/// resolution (silent C4VNull on a miss), script functions only, NO Status
/// check on the target (unlike `C4Object::Call`). The three access levels
/// (AA_PUBLIC/AA_PROTECTED/AA_PRIVATE) only LOG on violation and the call
/// still executes (C4Aul.cpp:332-342), so one implementation serves all
/// three names. Script pars[2..=9] shift to callee Par(0..=7).
pub(crate) fn object_call(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(target) = args.first().and_then(object_id_from_value) else {
        return Ok(Value::Nil); // !pObj → C4VNull (C4Script.cpp:3439)
    };
    let Some(Value::String(name)) = args.get(1) else {
        return Ok(Value::Nil); // !szFunction → C4VNull
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let pars: Vec<Value> = args.iter().skip(2).take(8).cloned().collect();
    call_world_object_own_function(target, name, &pars).unwrap_or(Ok(Value::Nil))
}

/// The VM's cross-object LocalN cell supplier (FnLocalN by-reference
/// foreign-local access, C4Script.cpp:4591-4605): hands out live cells
/// managed by the active host context. `None` for non-object targets —
/// the VM then falls back to the executing object like C++'s nullptr
/// conversion.
pub(crate) fn foreign_local_cell_hook(
    target: &Value,
    name: &str,
) -> Option<clonk_script::ValueCell> {
    let target = object_id_from_value(target)?;
    HOST_CONTEXT.with(|cell| {
        let mut context = cell.borrow_mut();
        let context = context.as_mut()?;
        if name
            .strip_prefix("__local_")
            .and_then(|index| index.parse::<i32>().ok())
            .is_some_and(|index| index >= 0)
        {
            return Some(context.foreign_local_cell(target, name));
        }
        let definition = context.object_effective_definition_id(target)?;
        let declared = context
            .world
            .definition_script(&definition)?
            .local_variable_names()
            .any(|local| local == name);
        declared.then(|| context.foreign_local_cell(target, name))
    })
}

/// The VM's Local/LocalN/SetLocal arrow fast paths bypass the ordinary method
/// dispatcher, so they ask the same host-world liveness question explicitly.
pub(crate) fn arrow_object_target_available_by_id(target: u64) -> bool {
    arrow_object_target_available(ObjectId::new(target))
}

/// AB_CALLGLOBAL temporarily runs with `cthr->Obj = cthr->Def = nullptr`
/// while retaining the suspended caller frame. The VM brackets the dynamic
/// extent with this hook after evaluating its arguments.
pub(crate) fn global_call_context_hook(enter: bool) {
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.set_global_call_context(enter);
        }
    });
}

/// FnEval selects its DirectExec receiver from the ACTIVE C4Aul context:
/// object definition, definition, then Game.Script (C4Script.cpp:4501-4513).
/// DirectExec creates a new frame whose Def is the object's current Def or
/// null without an object (C4AulExec.cpp:1658-1706).
pub(crate) fn eval_direct_exec_hook(
    source: &str,
    cells: &clonk_script::LocalCells,
    this: Value,
    strict_level: Option<u8>,
    depth: usize,
) -> Option<Result<Value, RuntimeError>> {
    let (script, direct_object, direct_definition) = HOST_CONTEXT.with(|cell| {
        let context = cell.borrow();
        let context = context.as_ref()?;
        if let Some(target) = object_id_from_value(&this) {
            return context
                .object_effective_definition_id(target)
                .and_then(|definition| {
                    context
                        .world
                        .definition_script(&definition)
                        .cloned()
                        .map(|script| (script, Some(target), Some(DefinitionId::from(definition))))
                });
        }
        match &context.script_definition_context {
            Some(Some(definition)) => context
                .world
                .definition_script(definition)
                .cloned()
                .map(|script| (script, None, None)),
            Some(None) | None => context
                .world
                .scenario_script()
                .cloned()
                .map(|script| (script, None, None)),
        }
    })?;
    let (previous_script_object, previous_script_definition, previous_definition) = HOST_CONTEXT
        .with(|cell| {
            let mut context = cell.borrow_mut();
            let context = context.as_mut()?;
            let previous_script_object = context.script_object_context.take();
            context.script_object_context = direct_object;
            let previous_script_definition = context
                .script_definition_context
                .replace(direct_definition.clone());
            let previous_definition = context.definition_context.take();
            context.definition_context = direct_definition;
            Some((
                previous_script_object,
                previous_script_definition,
                previous_definition,
            ))
        })?;
    let result = script.eval_direct_exec_with_cells_and_this_at_strict(
        source,
        cells,
        this,
        strict_level,
        depth,
    );
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.script_object_context = previous_script_object;
            context.script_definition_context = previous_script_definition;
            context.definition_context = previous_definition;
        }
    });
    Some(result)
}

/// `obj->Method(args)` / `obj->~Method(args)` — the AB_CALL/AB_CALLFS
/// direct object call (C4AulExec.cpp:1216-1305), forwarded by the VM as
/// [target, name, failsafe, args...]. Resolution is FindSameNameFunc on the
/// target (C4Aul.cpp:130-148): its own script functions first, then
/// global/engine functions running with the TARGET's context. A missing
/// function errors unless failsafe (`->~`), which yields nil; falsy targets
/// were already rejected in the VM.
pub(crate) fn arrow_method_dispatch(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_value = args.first().cloned().unwrap_or(Value::Nil);
    let Some(Value::String(name)) = args.get(1) else {
        return Err(RuntimeError::new(
            "Object call: missing function name".to_string(),
        ));
    };
    let failsafe = args.get(2).map(Value::as_bool).unwrap_or(false);
    let pars: Vec<Value> = args
        .iter()
        .skip(3)
        .collect::<Vec<_>>()
        .into_iter()
        .cloned()
        .collect();

    if let Value::C4Id(stored_id) = &target_value {
        // Definition call (C4AulExec.cpp:1235-1245): the definition must be
        // known — that error is NOT covered by the failsafe.
        let def_id = definition_id_for_c4id(stored_id).unwrap_or_default();
        let script = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.world.definition_script(&def_id).cloned())
        });
        let Some(script) = script else {
            return Err(RuntimeError::new(format!(
                "Definition call: Definition for id {} not found!",
                clonk_script::c4_id_text(stored_id)
            )));
        };
        // AB_CALL resolves via FindSameNameFunc: the def's own function
        // first, else a GLOBAL script function running in definition
        // scope (C4AulExec.cpp:1259-1261, C4Aul.cpp:130-148) — unlike
        // FnDefinitionCall's owner-scoped lookup.
        return match call_scoped_script_function_or_global(script, def_id, name, &pars) {
            Some(result) => result,
            None if failsafe => Ok(Value::Nil),
            None => Err(RuntimeError::new(format!(
                "Definition call: No function \"{name}\" in definition \"{}\"!",
                clonk_script::c4_id_text(stored_id)
            ))),
        };
    }

    let Some(target) = object_id_from_value(&target_value) else {
        return Err(RuntimeError::new(format!(
            "Object call: Invalid target type {}, expected object or id!",
            target_value.type_name()
        )));
    };
    if !arrow_object_target_available(target) {
        return Err(RuntimeError::new("Object call: target is zero!"));
    }
    // `obj->ID::Func(...)`: C++ validates ID::Func at parse time, but
    // AB_CALLNS is ignored by the executor (C4AulExec.cpp:1212-1214).
    // Preserve the validation, then let the paired AB_CALL re-resolve Func
    // on the target definition just like a plain arrow call.
    if let Some((namespace, function)) = name.split_once("::") {
        let script = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.world.definition_script(namespace).cloned())
        });
        let Some(script) = script else {
            return Err(RuntimeError::new(format!(
                "direct object call: def not found: {namespace}"
            )));
        };
        if !script.has_function(function) {
            return Err(RuntimeError::new(format!(
                "direct object call: function {namespace}::{function} not found"
            )));
        }
        return match call_world_object_function_from_arrow(target, function, &pars) {
            Some(result) => result,
            None if failsafe => Ok(Value::Nil),
            None => Err(RuntimeError::new(format!(
                "Object call: No function \"{function}\" in object {target}!"
            ))),
        };
    }
    match call_world_object_function_from_arrow(target, name, &pars) {
        Some(result) => result,
        None if failsafe => Ok(Value::Nil),
        None => Err(RuntimeError::new(format!(
            "Object call: No function \"{name}\" in object {target}!"
        ))),
    }
}

/// `anyfunctakesref` for the calling VM (C4AulParse.cpp:2318-2331): the
/// parser consults the engine-wide same-name function chain, which in this
/// port lives in the world's definition/global script tables.
pub(crate) fn arrow_reference_parameter_probe(name: &str, slot: usize) -> bool {
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.world.function_takes_reference_at(name, slot))
    })
}

/// C4AulParse resolves a direct-call name through the whole engine function
/// map before deciding whether to emit AB_CALLFS. The target object is
/// deliberately irrelevant to this lookup (C4AulParse.cpp:3215-3231).
pub(crate) fn arrow_direct_call_function_probe(name: &str) -> bool {
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_none_or(|context| context.world.script_function_known(name))
    })
}

/// AB_CALL twin for an arrow call carrying reference arguments. C4AulParse
/// pushes an lvalue argument as `C4V_pC4Value` whenever any same-named engine
/// function declares `&` at that slot (C4AulParse.cpp:2318-2331), and
/// `CheckConvertFunctionParameters` then lets the callee alias it
/// (C4AulExec.cpp:1381-1397). The `&[Value]` bridge cannot carry a pointer, so
/// this variant reports each parameter slot's final value and the calling VM
/// settles its own cells. Hazard's `this->~WeaponAt(x, y, r)` — which drives
/// both the crosshair vertex and the firing chain — depends on it.
pub(crate) fn arrow_method_ref_args_dispatch(
    args: &[Value],
) -> Result<(Value, Vec<Value>), RuntimeError> {
    let target_value = args.first().cloned().unwrap_or(Value::Nil);
    let Some(Value::String(name)) = args.get(1) else {
        return Err(RuntimeError::new(
            "Object call: missing function name".to_string(),
        ));
    };
    let failsafe = args.get(2).map(Value::as_bool).unwrap_or(false);
    let pars: Vec<Value> = args.iter().skip(3).cloned().collect();
    // A miss leaves every slot exactly as it was passed in.
    let unchanged = || pars.clone();

    if let Value::C4Id(stored_id) = &target_value {
        let def_id = definition_id_for_c4id(stored_id).unwrap_or_default();
        let script = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.world.definition_script(&def_id).cloned())
        });
        let Some(script) = script else {
            return Err(RuntimeError::new(format!(
                "Definition call: Definition for id {} not found!",
                clonk_script::c4_id_text(stored_id)
            )));
        };
        return match call_scoped_script_ref_args_or_global(script, def_id, name, &pars) {
            Some(result) => result,
            None if failsafe => Ok((Value::Nil, unchanged())),
            None => Err(RuntimeError::new(format!(
                "Definition call: No function \"{name}\" in definition \"{}\"!",
                clonk_script::c4_id_text(stored_id)
            ))),
        };
    }

    let Some(target) = object_id_from_value(&target_value) else {
        return Err(RuntimeError::new(format!(
            "Object call: Invalid target type {}, expected object or id!",
            target_value.type_name()
        )));
    };
    if !arrow_object_target_available(target) {
        return Err(RuntimeError::new("Object call: target is zero!"));
    }
    // `obj->ID::Func(...)`: AB_CALLNS only validates, AB_CALL re-resolves.
    let name = match name.split_once("::") {
        Some((namespace, function)) => {
            let script = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.definition_script(namespace).cloned())
            });
            let Some(script) = script else {
                return Err(RuntimeError::new(format!(
                    "direct object call: def not found: {namespace}"
                )));
            };
            if !script.has_function(function) {
                return Err(RuntimeError::new(format!(
                    "direct object call: function {namespace}::{function} not found"
                )));
            }
            function
        }
        None => name.as_ref(),
    };
    match call_world_object_ref_args_from_arrow(target, name, &pars) {
        Some(result) => result,
        None if failsafe => Ok((Value::Nil, unchanged())),
        None => Err(RuntimeError::new(format!(
            "Object call: No function \"{name}\" in object {target}!"
        ))),
    }
}

/// Reference-preserving AB_CALL twin for an arrow call in lvalue position.
/// C++ passes the call-target stack cell as `pReturn`; a `func &` therefore
/// leaves a C4V_pC4Value in the suspended caller instead of a copied value
/// (C4AulExec.cpp:1290-1299, 1054-1067).
pub(crate) fn arrow_method_reference_dispatch(
    args: &[Value],
) -> Result<clonk_script::ValueReference, RuntimeError> {
    let target_value = args.first().cloned().unwrap_or(Value::Nil);
    let Some(Value::String(name)) = args.get(1) else {
        return Err(RuntimeError::new(
            "Object call: missing function name".to_string(),
        ));
    };
    let failsafe = args.get(2).map(Value::as_bool).unwrap_or(false);
    let pars: Vec<Value> = args.iter().skip(3).cloned().collect();

    if let Value::C4Id(stored_id) = &target_value {
        let def_id = definition_id_for_c4id(stored_id).unwrap_or_default();
        let script = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.world.definition_script(&def_id).cloned())
        });
        let Some(script) = script else {
            return Err(RuntimeError::new(format!(
                "Definition call: Definition for id {} not found!",
                clonk_script::c4_id_text(stored_id)
            )));
        };
        return match call_scoped_script_reference(script, Some(def_id), name, &pars) {
            Some(result) => result,
            None if failsafe => Err(RuntimeError::new(format!(
                "function '{name}' does not return a reference"
            ))),
            None => Err(RuntimeError::new(format!(
                "Definition call: No function \"{name}\" in definition \"{}\"!",
                clonk_script::c4_id_text(stored_id)
            ))),
        };
    }

    let Some(target) = object_id_from_value(&target_value) else {
        return Err(RuntimeError::new(format!(
            "Object call: Invalid target type {}, expected object or id!",
            target_value.type_name()
        )));
    };
    if !arrow_object_target_available(target) {
        return Err(RuntimeError::new("Object call: target is zero!"));
    }
    if let Some((namespace, function)) = name.split_once("::") {
        let script = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.world.definition_script(namespace).cloned())
        });
        let Some(script) = script else {
            return Err(RuntimeError::new(format!(
                "direct object call: def not found: {namespace}"
            )));
        };
        if !script.has_function(function) {
            return Err(RuntimeError::new(format!(
                "direct object call: function {namespace}::{function} not found"
            )));
        }
        return match call_world_object_reference_from_arrow(target, function, &pars) {
            Some(result) => result,
            None if failsafe => Err(RuntimeError::new(format!(
                "function '{function}' does not return a reference"
            ))),
            None => Err(RuntimeError::new(format!(
                "Object call: No function \"{function}\" in object {target}!"
            ))),
        };
    }
    match call_world_object_reference_from_arrow(target, name, &pars) {
        Some(result) => result,
        None if failsafe => Err(RuntimeError::new(format!(
            "function '{name}' does not return a reference"
        ))),
        None => Err(RuntimeError::new(format!(
            "Object call: No function \"{name}\" in object {target}!"
        ))),
    }
}

/// A stale Rust object handle must not be translated into FindSameNameFunc's
/// missing-function result. C++ could only reach AB_CALL with either a live
/// object pointer or a freshly minted pointer to the still-active callback
/// scope; every older C4Value was zeroed by AssignRemoval first
/// (C4AulExec.cpp:1216-1279; C4Object.cpp:312).
fn arrow_object_target_available(target: ObjectId) -> bool {
    HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().is_none_or(|context| {
            context.object_status_present(target)
                || context.removed_object_references.contains(&target)
                    && context.object_scope(target).is_some()
        })
    })
}

/// Runs `function` on a script host with NO object context (Obj=nullptr,
/// C4AulExec.cpp:343): the active object scope is parked on the dormant
/// stack while the nested VM runs, so host functions see no `this`. Used by
/// DefinitionCall and GameCall/GameCallEx. Callee locals are per-call empty
/// (C++ throws on object-local access in a definition call,
/// C4AulExec.cpp:418-420; the Rust VM reads them as nil — documented).
pub(crate) fn call_scoped_script_function(
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_scoped_script_function_impl(
        script,
        function,
        args,
        false,
        false,
        false,
        None,
        None,
        false,
        EffectCallbackParameterConversionPolicy::Standard,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

/// Game.Script::Call resolves a named function owned by the scenario host.
/// Pin that exact ordinary function so a same-name engine global cannot be
/// selected during the nested VM call; a global-only declaration is absent.
pub(crate) fn call_scoped_scenario_function(
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    let resolution = script.resolve_function(function, false)?;
    call_scoped_script_function_impl(
        script,
        function,
        args,
        false,
        false,
        false,
        None,
        Some(resolution),
        false,
        EffectCallbackParameterConversionPolicy::Standard,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

pub(crate) fn call_scoped_definition_function(
    script: Arc<ScriptEngine>,
    definition: &str,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    let previous_definition = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut().as_mut().and_then(|context| {
            context
                .definition_context
                .replace(DefinitionId::from(definition))
        })
    });
    let result = call_scoped_script_function(script, function, args);
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.definition_context = previous_definition;
        }
    });
    result
}

/// The AB_CALL definition-call variant: FindSameNameFunc also finds
/// GLOBAL script functions (C4Aul.cpp:130-148) — own functions win.
fn call_scoped_script_function_or_global(
    script: Arc<ScriptEngine>,
    definition: DefinitionId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_scoped_script_function_impl(
        script,
        function,
        args,
        true,
        false,
        true,
        Some(Some(definition)),
        None,
        false,
        EffectCallbackParameterConversionPolicy::Standard,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

/// [`call_scoped_script_function_or_global`] for a definition-scope arrow call
/// whose callee declares `&` parameters.
fn call_scoped_script_ref_args_or_global(
    script: Arc<ScriptEngine>,
    definition: DefinitionId,
    function: &str,
    args: &[Value],
) -> Option<Result<(Value, Vec<Value>), RuntimeError>> {
    call_scoped_script_function_impl(
        script,
        function,
        args,
        true,
        false,
        true,
        Some(Some(definition)),
        None,
        true,
        EffectCallbackParameterConversionPolicy::Standard,
    )
}

/// C4Effect::DoCall's definition/global branch includes engine-native
/// callbacks such as FxFireInfo after script/global lookup.
pub(crate) fn call_scoped_effect_function_or_global(
    script: Arc<ScriptEngine>,
    definition: Option<DefinitionId>,
    function: &str,
    args: &[Value],
    parameter_conversion: EffectCallbackParameterConversionPolicy,
) -> Option<Result<Value, RuntimeError>> {
    call_scoped_script_function_impl(
        script,
        function,
        args,
        true,
        true,
        false,
        Some(definition),
        None,
        false,
        parameter_conversion,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

/// Game.ScriptEngine effect fallback: resolve only the shared engine-global
/// table (then native hosts), never an arbitrary carrier definition's local
/// function. C++ executes that retained engine-owned function with both Obj
/// and Def null (C4Effect.cpp:448-456; C4AulExec.cpp:343-352).
pub(crate) fn call_scoped_global_effect_function(
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
    parameter_conversion: EffectCallbackParameterConversionPolicy,
) -> Option<Result<Value, RuntimeError>> {
    if !script.has_global_function(function) && !script.has_host_function(function) {
        return None;
    }
    let (previous_script_object, previous_script_definition, previous_definition) = HOST_CONTEXT
        .with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                let active = context.object.take();
                context.dormant_scopes.push(active);
                (
                    context.script_object_context.take(),
                    context.script_definition_context.replace(None),
                    context.definition_context.take(),
                )
            } else {
                (None, None, None)
            }
        });
    let call =
        if parameter_conversion == EffectCallbackParameterConversionPolicy::WarnForNonStrict3 {
            script
                .call_global_for_effect_callback(function, args)
                .map(|value| (value, args.to_vec()))
        } else {
            script.call_global_with_ref_args(function, args)
        }
        .map(|(value, _)| value);
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.object = context.dormant_scopes.pop().unwrap_or(None);
            context.script_object_context = previous_script_object;
            context.script_definition_context = previous_script_definition;
            context.definition_context = previous_definition;
        }
    });
    Some(match call {
        Ok(value) => Ok(value),
        Err(clonk_script::ScriptError::Runtime(err)) => Err(err),
        Err(other) => Err(RuntimeError::new(other.to_string())),
    })
}

fn call_scoped_script_reference(
    script: Arc<ScriptEngine>,
    definition_override: Option<DefinitionId>,
    function: &str,
    args: &[Value],
) -> Option<Result<clonk_script::ValueReference, RuntimeError>> {
    let resolution = script.resolve_function(function, true)?;
    let (previous_script_object, previous_script_definition, previous_definition) = HOST_CONTEXT
        .with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                let definition = definition_override.clone().or_else(|| {
                    if resolution.scope == clonk_script::ScriptFunctionScope::Global {
                        None
                    } else {
                        context
                            .world
                            .script_for_host_identity(resolution.host_identity)
                            .and_then(|(_, definition, _)| definition)
                            .or_else(|| context.definition_context.clone())
                    }
                });
                let active = context.object.take();
                context.dormant_scopes.push(active);
                let previous_definition =
                    std::mem::replace(&mut context.definition_context, definition.clone());
                (
                    context.script_object_context.take(),
                    context.script_definition_context.replace(definition),
                    previous_definition,
                )
            } else {
                (None, None, None)
            }
        });
    let cells = clonk_script::LocalCells::from_local_vars(&HashMap::new());
    let call = script.call_reference_with_cells_and_this_preserving_caller(
        function,
        args,
        &cells,
        Value::Nil,
    );
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.object = context.dormant_scopes.pop().unwrap_or(None);
            context.script_object_context = previous_script_object;
            context.script_definition_context = previous_script_definition;
            context.definition_context = previous_definition;
        }
    });
    Some(match call {
        Ok(reference) => Ok(reference),
        Err(clonk_script::ScriptError::Runtime(err)) => Err(err),
        Err(other) => Err(RuntimeError::new(other.to_string())),
    })
}

fn call_scoped_script_function_impl(
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
    include_globals: bool,
    include_host: bool,
    preserve_caller: bool,
    definition_override: Option<Option<DefinitionId>>,
    resolution_override: Option<clonk_script::ScriptFunctionResolution>,
    ref_args: bool,
    parameter_conversion: EffectCallbackParameterConversionPolicy,
) -> Option<Result<(Value, Vec<Value>), RuntimeError>> {
    let pinned_resolution = resolution_override;
    let resolution = pinned_resolution.clone().or_else(|| {
        if include_globals {
            script.resolve_function(function, true)
        } else {
            script.resolve_function(function, false)
        }
    });
    let resolvable = resolution.is_some() || (include_host && script.has_host_function(function));
    if !resolvable {
        return None;
    }
    let (previous_script_object, previous_script_definition, previous_definition) = HOST_CONTEXT
        .with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                let definition = match definition_override {
                    Some(definition) => definition,
                    None => match resolution.as_ref() {
                        Some(resolution)
                            if resolution.scope == clonk_script::ScriptFunctionScope::Global =>
                        {
                            None
                        }
                        Some(resolution) => context
                            .world
                            .script_for_host_identity(resolution.host_identity)
                            .and_then(|(_, definition, _)| definition)
                            .or_else(|| context.definition_context.clone()),
                        None => context.definition_context.clone(),
                    },
                };
                let active = context.object.take();
                context.dormant_scopes.push(active);
                let previous_definition =
                    std::mem::replace(&mut context.definition_context, definition.clone());
                (
                    context.script_object_context.take(),
                    context.script_definition_context.replace(definition),
                    previous_definition,
                )
            } else {
                (None, None, None)
            }
        });
    let locals = HashMap::new();
    let call = if ref_args {
        debug_assert!(preserve_caller && pinned_resolution.is_none());
        debug_assert_eq!(
            parameter_conversion,
            EffectCallbackParameterConversionPolicy::Standard
        );
        let cells = clonk_script::LocalCells::from_local_vars(&locals);
        script.call_ref_args_with_cells_and_this_preserving_caller(
            function,
            args,
            &cells,
            Value::Nil,
        )
    } else if let Some(resolution) = pinned_resolution {
        let cells = clonk_script::LocalCells::from_local_vars(&locals);
        let call = if parameter_conversion
            == EffectCallbackParameterConversionPolicy::WarnForNonStrict3
        {
            script.call_resolved_with_cells_and_this_for_effect_callback(
                &resolution,
                false,
                args,
                &cells,
                Value::Nil,
            )
        } else {
            script.call_resolved_with_cells_and_this(&resolution, false, args, &cells, Value::Nil)
        };
        call.map(|value| (value, args.to_vec()))
    } else if preserve_caller {
        let cells = clonk_script::LocalCells::from_local_vars(&locals);
        script
            .call_with_cells_and_this_preserving_caller(function, args, &cells, Value::Nil)
            .map(|value| (value, args.to_vec()))
    } else {
        let call = if parameter_conversion
            == EffectCallbackParameterConversionPolicy::WarnForNonStrict3
        {
            script.call_effect_callback_with_locals_and_this(function, args, &locals, Value::Nil)
        } else {
            script.call_with_locals_and_this(function, args, &locals, Value::Nil)
        };
        call.map(|(value, _locals)| (value, args.to_vec()))
    };
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.object = context.dormant_scopes.pop().unwrap_or(None);
            context.script_object_context = previous_script_object;
            context.script_definition_context = previous_script_definition;
            context.definition_context = previous_definition;
        }
    });
    Some(match call {
        Ok(outcome) => Ok(outcome),
        Err(clonk_script::ScriptError::Runtime(err)) => Err(err),
        Err(other) => Err(RuntimeError::new(other.to_string())),
    })
}

/// FnDefinitionCall (C4Script.cpp:3451-3468): runs a function on a
/// definition's script with Obj=nullptr — always failsafe ("~" prefix,
/// :3457-3459): unknown id or missing function → silent C4VNull. Script
/// pars[2..=9] shift to callee Par(0..=7).
pub(crate) fn definition_call(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(def_id) = parse_native_c4id_argument(args.first(), "DefinitionCall")? else {
        return Ok(Value::Nil);
    };
    let Some(Value::String(name)) = args.get(1) else {
        return Ok(Value::Nil);
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let script = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.definition_script(&def_id).cloned())
    });
    let Some(script) = script else {
        return Ok(Value::Nil); // C4Id2Def failure → C4VNull (C4Script.cpp:3462)
    };
    let parameter_end = args.len().min(10);
    let pars = args.get(2..parameter_end).unwrap_or(&[]);
    call_scoped_scenario_function(script, name, pars).unwrap_or(Ok(Value::Nil))
}

/// FnGameCall (C4Script.cpp:3470-3484): runs a function on the scenario
/// script host ONLY (owner-scoped lookup — definition globals are not
/// visible), always failsafe, Obj=nullptr. Script pars[1..=9] shift to
/// callee Par(0..=8).
pub(crate) fn game_call(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(name)) = args.first() else {
        return Ok(Value::Nil); // !szFunction → C4VNull (C4Script.cpp:3475)
    };
    let name = strip_failsafe(name);
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let script = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.scenario_script().cloned())
    });
    let Some(script) = script else {
        return Ok(Value::Nil);
    };
    let parameter_end = args.len().min(10);
    let pars = args.get(1..parameter_end).unwrap_or(&[]);
    call_scoped_script_function(script, name, pars).unwrap_or(Ok(Value::Nil))
}

/// FnGameCallEx (C4Script.cpp:3486-3500) → `C4GameScriptHost::GRBroadcast`
/// (C4ScriptHost.cpp:234-248): calls the function on every LIVE object whose
/// Category has a C4D_Goal|C4D_Rule|C4D_Environment bit, in list order, with
/// results DISCARDED (fRejectTest=false) — "call objects first - scenario
/// script might overwrite hostility, etc." — then on the scenario script,
/// whose result is the sole return value. Always failsafe ("~" prefix);
/// callee errors still pass through (fPassErrors=true).
pub(crate) fn game_call_ex(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(name)) = args.first() else {
        return Ok(Value::Nil); // !szFunction → C4VNull (C4Script.cpp:3491)
    };
    let name = strip_failsafe(name).to_string();
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    let parameter_end = args.len().min(10);
    let pars = args.get(1..parameter_end).unwrap_or(&[]);

    // C4D_Goal | C4D_Environment | C4D_Rule (definition.rs:1608-1622)
    const BROADCAST_MASK: i32 = (1 << 5) | (1 << 6) | (1 << 19);
    let targets: Vec<ObjectId> = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(EffectHostContext::master_object_ids)
            .unwrap_or_default()
    });
    for target in targets {
        // C++ evaluates Category and Status at each live-list node's turn;
        // an earlier callback may delete, deactivate, or recategorize a later
        // object into or out of the set.
        let eligible = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(target))
                .map(|object| {
                    object.status().is_active() && object.category() & BROADCAST_MASK != 0
                })
                .unwrap_or(false)
        });
        if !eligible {
            continue;
        }
        if let Some(result) = call_world_object_own_function(target, &name, pars) {
            result?;
        }
    }

    let script = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.scenario_script().cloned())
    });
    match script {
        Some(script) => {
            call_scoped_scenario_function(script, &name, pars).unwrap_or(Ok(Value::Nil))
        }
        None => Ok(Value::Nil),
    }
}

/// One synced draw through the active random context (host-side engine
/// draws that C++ makes inside script-called engine functions).
pub(crate) fn draw_context_random(range: i32) -> Result<i32, RuntimeError> {
    RANDOM_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("random context unavailable"))?
            .clone();
        let mut rng = context.rng.borrow_mut();
        Ok(rng.random(range))
    })
}

/// One `Rnd3()` ring read through the active synced random context.
pub(crate) fn draw_context_rnd3() -> Result<i32, RuntimeError> {
    RANDOM_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("random context unavailable"))?
            .clone();
        let value = context.rng.borrow_mut().rnd3();
        Ok(value)
    })
}

pub(crate) fn enter_random_context(rng: LcgRng) -> RandomContextGuard {
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

#[derive(Clone, Copy)]
enum LocateFuncContextError {
    InvalidDefinition,
    NoValidContext,
}

fn parse_locate_func_object_argument(value: &Value) -> Result<Option<ObjectId>, RuntimeError> {
    let eager_falsy_conversion = !matches!(
        clonk_script::caller_origin_strictness(),
        clonk_script::HostCallerStrictness::Strict(level) if level >= 3
    );
    if eager_falsy_conversion && !value.as_bool() {
        return Ok(None);
    }
    match value {
        Value::Nil => Ok(None),
        Value::Object(_) | Value::Proplist(_) => Ok(object_id_from_value(value)),
        other => Err(RuntimeError::new(format!(
            "LocateFunc: expected object for object, got {}",
            other.type_name()
        ))),
    }
}

/// FnLocateFunc (C4Script.cpp:4515-4575): select an object/definition/caller
/// script, then print the active function and its `OwnerOverloaded` chain.
/// This is intentionally diagnostic-only; a valid context returns true even
/// when the requested function does not exist.
pub(crate) fn locate_func(args: &[Value]) -> Result<Value, RuntimeError> {
    // C4Aul applies all three native parameter conversions before entering
    // FnLocateFunc, so validate the typed slots before the missing-name guard.
    let function = parse_native_c4_string_argument(args.first(), "LocateFunc", "function")?;
    let object = parse_locate_func_object_argument(args.get(1).unwrap_or(&Value::Nil))?;
    let definition = parse_native_c4id_argument(args.get(2), "LocateFunc")?;

    let Some(function) = function else {
        error!(target: SCRIPT_LOG_TARGET, "No func name");
        return Ok(Value::Bool(false));
    };

    let caller_host = clonk_script::caller_host_identity();
    let caller_uses_engine_scope = clonk_script::caller_uses_engine_scope().unwrap_or(false);
    let lookup = HOST_CONTEXT.with(|cell| {
        let context = cell.borrow();
        let Some(context) = context.as_ref() else {
            return Err(if definition.is_some() {
                LocateFuncContextError::InvalidDefinition
            } else {
                LocateFuncContextError::NoValidContext
            });
        };

        // Explicit object wins over even an invalid explicit definition.
        // Otherwise a non-empty ID wins over caller fallback.
        let (script, engine_scope) = if let Some(object) = object {
            let Some(definition) = context.object_effective_definition_id(object) else {
                return Err(LocateFuncContextError::NoValidContext);
            };
            let Some(script) = context.world.definition_script(&definition).cloned() else {
                return Err(LocateFuncContextError::NoValidContext);
            };
            (script, false)
        } else if let Some(definition) = definition.as_deref() {
            let Some(script) = context.world.definition_script(definition).cloned() else {
                return Err(LocateFuncContextError::InvalidDefinition);
            };
            (script, false)
        } else {
            let Some(caller_host) = caller_host else {
                return Err(LocateFuncContextError::NoValidContext);
            };
            let Some((_, _, script)) = context.world.script_for_host_identity(caller_host) else {
                return Err(LocateFuncContextError::NoValidContext);
            };
            (script, caller_uses_engine_scope)
        };

        let resolution = if engine_scope {
            script.resolve_global_function(&function)
        } else {
            script.resolve_function(&function, true)
        };
        let root_scope = resolution.as_ref().map(|resolution| resolution.scope);
        let mut messages = Vec::new();
        let mut seen = Vec::new();

        let mut append_chain = |root: &clonk_script::Function, engine_globals_only: bool| {
            let mut current = Some(root);
            while let Some(candidate) = current {
                if (!engine_globals_only || candidate.is_global())
                    && !seen.iter().any(|emitted| emitted == candidate)
                {
                    seen.push(candidate.clone());
                    let suffix = candidate
                        .source_host_identity()
                        .and_then(|identity| {
                            context
                                .world
                                .script_for_host_identity(identity)
                                .map(|(name, _, _)| format!("{name}:{}", candidate.source_line()))
                        })
                        .unwrap_or_else(|| "no owner".to_string());
                    messages.push(format!("{} ({suffix})", candidate.name));
                }
                current = candidate.overloaded.as_deref();
            }
        };

        if let Some(resolution) = resolution.as_ref() {
            append_chain(
                resolution.function.as_ref(),
                resolution.scope == clonk_script::ScriptFunctionScope::Global,
            );
        }
        // A definition-local chain falls through to Game.ScriptEngine. The
        // VM keeps that global table separately, so reconstruct the tail for
        // this diagnostic and suppress any node already linked explicitly.
        if root_scope == Some(clonk_script::ScriptFunctionScope::Local) {
            if let Some(global) = script.resolve_global_function(&function) {
                append_chain(global.function.as_ref(), true);
            }
        }
        if script.has_host_function(&function) {
            messages.push(format!("{function} (engine)"));
        }

        Ok((
            messages,
            resolution.is_some() || script.has_host_function(&function),
        ))
    });

    let (messages, found) = match lookup {
        Ok(result) => result,
        Err(LocateFuncContextError::InvalidDefinition) => {
            error!(target: SCRIPT_LOG_TARGET, "Invalid or unloaded def");
            return Ok(Value::Bool(false));
        }
        Err(LocateFuncContextError::NoValidContext) => {
            error!(target: SCRIPT_LOG_TARGET, "No valid script context");
            return Ok(Value::Bool(false));
        }
    };

    if !found {
        error!(target: SCRIPT_LOG_TARGET, "Func {} not found", function);
        return Ok(Value::Bool(true));
    }
    for (index, message) in messages.into_iter().enumerate() {
        if index == 0 {
            info!(target: SCRIPT_LOG_TARGET, "{}", message);
        } else {
            info!(target: SCRIPT_LOG_TARGET, "overloads {}", message);
        }
    }
    Ok(Value::Bool(true))
}

/// The FnAdjustWalkRotation seam (C4Script.cpp:5439-5448 +
/// C4Object::AdjustWalkRotation, C4Object.cpp:6019-6086): Def->Rotateable,
/// the frame's Action.t_attach, the shape attach record, and
/// Def->Shape.VtxX[iAttachVtx] (the DEF vertex for the middle-bottom
/// check; the LIVE vertex comes from the scope's vertices).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WalkRotationSeed {
    pub rotateable: i32,
    pub t_attach: u32,
    pub attach: ShapeAttachRecord,
    pub def_attach_vtx_x: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct HostObjectContext<'a> {
    pub id: ObjectId,
    /// The object's definition — GetID's source when the world snapshot
    /// predates the object (its own materialize-time Initialize).
    pub definition_id: Option<String>,
    pub container: Option<ObjectId>,
    pub status: ObjectStatus,
    pub energy: i32,
    /// C4Object::Breath on the raw physical scale.
    pub breath: i32,
    pub need_energy: bool,
    /// C4Object::MagicEnergy (C4Object.h:139), on the
    /// MagicPhysicalFactor-scaled raw scale.
    pub magic_energy: i32,
    pub damage: i32,
    pub alive: bool,
    /// C4Object::InLiquid (the cached flag FnInLiquid reads).
    pub in_liquid: bool,
    /// C4Object::OwnMass (SetMass leftovers).
    pub own_mass: i32,
    pub owner: i32,
    /// C4Object::Controller (C4Object.h:127) — cause tracing.
    pub controller: i32,
    pub category: i32,
    pub ocf: u32,
    pub ocf_base: u32,
    pub crew_member: bool,
    pub plr_view_range: i32,
    pub position: Vector2,
    pub velocity: Vector2,
    pub rotation: i32,
    pub effects: &'a [EffectState],
    pub action_name: String,
    pub action_index: Option<u32>,
    pub action_ticks: i32,
    pub action_data: i32,
    pub action_phase: i32,
    pub action_library: SharedActionLibrary,
    pub direction: Direction,
    pub command_direction: CommandDirection,
    pub command_count: usize,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub vertices: &'a [ObjectVertex],
    /// `pub(in crate::compat)` so the sibling `compat::tests` module can
    /// build a scope through a record update; the field stays invisible
    /// outside this subtree.
    pub(in crate::compat) shape_vertices: ShapeVertexBuffer,
    pub construction: i32,
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    pub draw_transform: Option<DrawTransform>,
    pub base_graphics: Option<ObjectBaseGraphics>,
    pub info_physical: Option<PhysicalInfo>,
    pub temporary_physical: Option<PhysicalInfo>,
    pub physical_changes: Vec<(String, i32)>,
    pub definition_physical: PhysicalInfo,
    pub walk_rotation: WalkRotationSeed,
    /// The TRUE fix_x/fix_y at call time; None reconstructs whole pixels.
    pub script_fixed_position: Option<FixedVec2>,
    /// The TRUE sub-pixel dirs at call time (C++ Fn(Get|Set)XDir read the
    /// live C4Fixed xdir/ydir, C4Script.cpp:697-732/:1160-1180); None
    /// falls back to the int-velocity reconstruction.
    pub script_fixed_velocity: Option<FixedVec2>,
    /// The TRUE angular velocity (`rdir`) at call time. None falls back to
    /// the world snapshot or zero for legacy fixtures.
    pub script_rotation_velocity: Option<C4Fixed>,
    /// The TRUE raw rotation accumulator (`fix_r`) at call time. None falls
    /// back to the whole-degree rotation for legacy fixtures.
    pub script_fixed_rotation: Option<C4Fixed>,
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
        action_ticks: i32,
        action_data: i32,
        action_library: ActionLibrary,
        direction: Direction,
        command_direction: CommandDirection,
        command_count: usize,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: &'a [ObjectVertex],
        construction: i32,
    ) -> Self {
        Self::with_category(
            id,
            container,
            status,
            energy,
            0,
            construction,
            owner,
            position,
            velocity,
            0,
            effects,
            action_name,
            action_ticks,
            action_data,
            0,
            action_library,
            direction,
            command_direction,
            command_count,
            action_target,
            action_target2,
            vertices,
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
    }

    pub fn with_category(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        damage: i32,
        construction: i32,
        owner: i32,
        position: Vector2,
        velocity: Vector2,
        rotation: i32,
        effects: &'a [EffectState],
        action_name: impl Into<String>,
        action_ticks: i32,
        action_data: i32,
        action_phase: i32,
        action_library: impl Into<SharedActionLibrary>,
        direction: Direction,
        command_direction: CommandDirection,
        command_count: usize,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        vertices: &'a [ObjectVertex],
        category: i32,
        ocf_base: u32,
        crew_member: bool,
        draw_transform: Option<DrawTransform>,
        base_graphics: Option<ObjectBaseGraphics>,
    ) -> Self {
        Self {
            id,
            definition_id: None,
            container,
            status,
            energy,
            breath: 0,
            need_energy: false,
            magic_energy: 0,
            damage,
            construction: construction.max(0),
            alive: true,
            in_liquid: false,
            own_mass: 0,
            owner,
            // The Init default (C4Object.cpp:162); real controllers ride
            // in via with_controller.
            controller: owner,
            category,
            ocf: ocf::NORMAL,
            ocf_base,
            crew_member,
            plr_view_range: 0,
            position,
            velocity,
            rotation,
            effects,
            action_name: action_name.into(),
            action_index: None,
            action_ticks,
            action_data,
            action_phase,
            action_library: action_library.into(),
            direction,
            command_direction,
            command_count,
            action_target,
            action_target2,
            vertices,
            shape_vertices: ShapeVertexBuffer::from_active(vertices),
            graphics_overlays: Vec::new(),
            draw_transform,
            base_graphics,
            info_physical: None,
            temporary_physical: None,
            physical_changes: Vec::new(),
            definition_physical: PhysicalInfo::default(),
            walk_rotation: WalkRotationSeed::default(),
            script_fixed_position: None,
            script_fixed_velocity: None,
            script_rotation_velocity: None,
            script_fixed_rotation: None,
        }
    }

    pub fn with_walk_rotation(mut self, walk_rotation: WalkRotationSeed) -> Self {
        self.walk_rotation = walk_rotation;
        self
    }

    pub(crate) fn with_action_index(mut self, action_index: Option<u32>) -> Self {
        self.action_index = action_index;
        self
    }

    pub(crate) fn with_shape_vertices(mut self, vertices: &ShapeVertexBuffer) -> Self {
        self.shape_vertices = vertices.clone();
        self
    }

    pub fn with_script_fixed_position(mut self, position: Option<FixedVec2>) -> Self {
        self.script_fixed_position = position;
        self
    }

    pub fn with_script_fixed_velocity(mut self, velocity: Option<FixedVec2>) -> Self {
        self.script_fixed_velocity = velocity;
        self
    }

    pub fn with_script_rotation_velocity(mut self, velocity: Option<C4Fixed>) -> Self {
        self.script_rotation_velocity = velocity;
        self
    }

    pub fn with_script_fixed_rotation(mut self, rotation: Option<C4Fixed>) -> Self {
        self.script_fixed_rotation = rotation;
        self
    }

    pub fn with_magic_energy(mut self, magic_energy: i32) -> Self {
        self.magic_energy = magic_energy;
        self
    }

    pub fn with_breath(mut self, breath: i32) -> Self {
        self.breath = breath;
        self
    }

    pub fn with_need_energy(mut self, need_energy: bool) -> Self {
        self.need_energy = need_energy;
        self
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }

    pub fn with_controller(mut self, controller: i32) -> Self {
        self.controller = controller;
        self
    }

    pub fn with_in_liquid(mut self, in_liquid: bool) -> Self {
        self.in_liquid = in_liquid;
        self
    }

    pub fn with_own_mass(mut self, own_mass: i32) -> Self {
        self.own_mass = own_mass;
        self
    }

    pub fn with_definition_id(mut self, definition_id: impl Into<String>) -> Self {
        self.definition_id = Some(definition_id.into());
        self
    }

    #[cfg(test)]
    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = crew_member;
        self
    }

    pub fn with_plr_view_range(mut self, plr_view_range: i32) -> Self {
        self.plr_view_range = plr_view_range;
        self
    }

    pub fn with_physicals(
        mut self,
        info: Option<PhysicalInfo>,
        temporary: Option<PhysicalInfo>,
        changes: Vec<(String, i32)>,
        definition: PhysicalInfo,
    ) -> Self {
        self.info_physical = info;
        self.temporary_physical = temporary;
        self.physical_changes = changes;
        self.definition_physical = definition;
        self
    }

    pub fn with_graphics_overlays(mut self, overlays: Vec<ObjectGraphicsOverlay>) -> Self {
        self.graphics_overlays = overlays;
        self
    }

    #[allow(dead_code)]
    pub fn with_draw_transform(mut self, transform: Option<DrawTransform>) -> Self {
        self.draw_transform = transform;
        self
    }

    pub fn with_base_graphics(mut self, base: Option<ObjectBaseGraphics>) -> Self {
        self.base_graphics = base;
        self
    }

    pub fn with_ocf(mut self, ocf: u32) -> Self {
        self.ocf = ocf;
        self
    }

    #[allow(dead_code)]
    pub fn ocf(&self) -> u32 {
        self.ocf
    }

    #[allow(dead_code)]
    pub fn ocf_base(&self) -> u32 {
        self.ocf_base
    }

    #[allow(dead_code)]
    pub fn is_crew_member(&self) -> bool {
        self.crew_member
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct PhysicsDelta {
    pub gravity: Option<i32>,
}

impl PhysicsDelta {
    pub fn is_empty(&self) -> bool {
        self.gravity.is_none()
    }

    pub fn apply(&self, physics: &mut PhysicsSettings) {
        if let Some(gravity) = self.gravity {
            physics.set_script_gravity(gravity);
        }
    }
}

#[derive(Debug)]
pub(crate) struct PhysicsContext {
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

    pub(crate) fn set_gravity(&self, gravity: i32) {
        let clamped = gravity.clamp(-300, 300);
        self.settings.borrow_mut().set_script_gravity(clamped);
        self.pending.borrow_mut().gravity = Some(clamped);
    }

    pub(crate) fn gravity(&self) -> i32 {
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
#[doc(hidden)]
pub struct EnvironmentDelta {
    pub wind: Option<i32>,
    pub temperature: Option<i32>,
    pub climate: Option<i32>,
    pub season: Option<i32>,
    pub(crate) season_gamma_handled: bool,
}

impl EnvironmentDelta {
    pub(crate) fn is_empty(&self) -> bool {
        self.wind.is_none()
            && self.temperature.is_none()
            && self.climate.is_none()
            && self.season.is_none()
    }

    pub(crate) fn requests_season_gamma_refresh(&self) -> bool {
        !self.season_gamma_handled
            && (self.temperature.is_some() || self.climate.is_some() || self.season.is_some())
    }

    pub fn apply(&self, environment: &mut EnvironmentSettings) {
        if let Some(wind) = self.wind {
            let clamped = wind.clamp(-100, 100);
            environment.wind = clamped;
            environment.wind_target = clamped;
        }
        if let Some(temperature) = self.temperature {
            environment.temperature = temperature.clamp(-100, 100);
        }
        if let Some(climate) = self.climate {
            environment.climate = climate.clamp(-50, 50);
        }
        if let Some(season) = self.season {
            // C4Weather::SetSeason (C4Weather.cpp:229-233): BoundBy 0..100.
            environment.season = season.clamp(0, 100);
        }
    }
}

#[derive(Debug)]
pub(crate) struct EnvironmentContext {
    pub(crate) settings: RefCell<EnvironmentSettings>,
    pub(crate) frame: u64,
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

    pub(crate) fn set_wind(&self, wind: i32) {
        let clamped = wind.clamp(-100, 100);
        let mut settings = self.settings.borrow_mut();
        settings.wind = clamped;
        settings.wind_target = clamped;
        self.pending.borrow_mut().wind = Some(clamped);
    }

    pub(crate) fn wind_force(&self) -> i32 {
        let settings = self.settings.borrow();
        settings.wind_force(self.frame)
    }

    pub(crate) fn set_temperature(&self, temperature: i32) {
        let clamped = temperature.clamp(-100, 100);
        self.settings.borrow_mut().temperature = clamped;
        self.pending.borrow_mut().temperature = Some(clamped);
        self.queue_season_gamma_control();
    }

    /// C4Weather::GetTemperature returns the mutable Weather.Temperature
    /// field verbatim (C4Weather.cpp:173-176). Climate only influences that
    /// field during Init/Execute; it is not added again at script read time.
    pub(crate) fn temperature(&self) -> i32 {
        self.settings.borrow().temperature
    }

    pub(crate) fn set_climate(&self, climate: i32) {
        let clamped = climate.clamp(-50, 50);
        self.settings.borrow_mut().climate = clamped;
        self.pending.borrow_mut().climate = Some(clamped);
        self.queue_season_gamma_control();
    }

    pub(crate) fn climate(&self) -> i32 {
        self.settings.borrow().climate
    }

    /// C4Weather::SetSeason (C4Weather.cpp:229-233): BoundBy(iSeason, 0,
    /// 100), then SetSeasonGamma immediately. Queueing the gamma operation
    /// here preserves its order relative to an explicit SetGamma call in
    /// the same callback.
    pub(crate) fn set_season(&self, season: i32) {
        let clamped = season.clamp(0, 100);
        self.settings.borrow_mut().season = clamped;
        self.pending.borrow_mut().season = Some(clamped);
        self.queue_season_gamma_control();
    }

    fn queue_season_gamma_control(&self) {
        let points = self.settings.borrow().season_gamma_control_points();
        let handled = points.is_none_or(|points| {
            HOST_CONTEXT.with(|cell| {
                cell.borrow_mut().as_mut().is_some_and(|context| {
                    context.register_landscape_operation(LandscapeOperation::GammaRamp {
                        index: 1,
                        points,
                    });
                    true
                })
            })
        });
        self.pending.borrow_mut().season_gamma_handled = handled;
    }

    pub(crate) fn season(&self) -> i32 {
        self.settings.borrow().season
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

#[allow(dead_code)]
pub(crate) fn with_effect_context<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
    E: From<RuntimeError>,
{
    with_effect_context_with_state(object, global_effects, world, next_object_id, false, func)
}

pub(crate) fn with_effect_context_with_state<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    game_over_triggered: bool,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
    E: From<RuntimeError>,
{
    let script_object_context = object.as_ref().map(|object| object.id);
    with_effect_context_with_definition_state(
        object,
        None,
        script_object_context,
        global_effects,
        world,
        next_object_id,
        game_over_triggered,
        func,
    )
}

pub(crate) fn with_effect_context_with_state_and_definition<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    definition_context: Option<DefinitionId>,
    script_object_context: Option<ObjectId>,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    game_over_triggered: bool,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
    E: From<RuntimeError>,
{
    with_effect_context_with_definition_state(
        object,
        definition_context,
        script_object_context,
        global_effects,
        world,
        next_object_id,
        game_over_triggered,
        func,
    )
}

pub(crate) fn with_definition_effect_context_with_state<F, T, E>(
    definition_context: DefinitionId,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    game_over_triggered: bool,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
    E: From<RuntimeError>,
{
    with_effect_context_with_definition_state(
        None,
        Some(definition_context),
        None,
        global_effects,
        world,
        next_object_id,
        game_over_triggered,
        func,
    )
}

struct EffectHostContextTlsGuard<'a> {
    cell: &'a RefCell<Option<EffectHostContext>>,
    active: bool,
}

impl EffectHostContextTlsGuard<'_> {
    fn finish(mut self) -> EffectHostContext {
        let context = self
            .cell
            .borrow_mut()
            .take()
            .expect("effect context must be present");
        self.active = false;
        context
    }
}

impl Drop for EffectHostContextTlsGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.cell.borrow_mut().take();
        }
    }
}

fn with_effect_context_with_definition_state<F, T, E>(
    object: Option<HostObjectContext<'_>>,
    definition_context: Option<DefinitionId>,
    script_object_context: Option<ObjectId>,
    global_effects: &[EffectState],
    world: HostWorldContext,
    next_object_id: u64,
    game_over_triggered: bool,
    func: F,
) -> (Result<T, E>, EffectContextOutcome)
where
    F: FnOnce() -> Result<T, E>,
    E: From<RuntimeError>,
{
    let audio_state = AUDIO_CONTEXT
        .with(|cell| cell.borrow_mut().take())
        .unwrap_or_default();
    HOST_CONTEXT.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "nested effect contexts are not supported"
        );
        *cell.borrow_mut() = Some(EffectHostContext::new(
            object,
            definition_context,
            script_object_context,
            global_effects.to_vec(),
            world,
            next_object_id,
            audio_state,
            game_over_triggered,
        ));
        let guard = EffectHostContextTlsGuard { cell, active: true };
        let result =
            clonk_script::with_diagnostic_object_formatter(diagnostic_object_data_string, func);
        let context = guard.finish();
        let outcome = context.into_commands();
        AUDIO_CONTEXT.with(|cell| {
            *cell.borrow_mut() = Some(outcome.audio.state.clone());
        });
        (result, outcome)
    })
}

#[derive(Debug, Clone)]
pub struct NestedObjectOutcome {
    pub object_id: ObjectId,
    pub effects: Vec<EffectCommand>,
    pub update: Option<ObjectUpdate>,
    pub commands: Vec<QueuedCommand>,
    pub command_operations: Vec<CommandOperation>,
    pub destroy: bool,
    /// A staged C4Object::AssignDeath request from script Kill. `Some(false)`
    /// is distinct from no request; `true` bypasses effect revival.
    pub assign_death: Option<bool>,
    /// Callback-final raw contents lists for containers whose links changed
    /// during this VM invocation. The host preview applies the chronological
    /// Remove/Insert/MoveToBack/RotateToFront stream immediately, like C++;
    /// the authoritative copy-out installs these orders only after nested
    /// child container updates have materialized.
    #[doc(hidden)]
    pub contents_orders: Vec<HostContentsOrder>,
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct HostContentsOrder {
    pub(crate) container: ObjectId,
    pub(crate) contents: Vec<ObjectId>,
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct EffectContextOutcome {
    pub object: Vec<EffectCommand>,
    pub global: Vec<EffectCommand>,
    pub object_update: Option<ObjectUpdate>,
    pub object_commands: Vec<QueuedCommand>,
    pub command_operations: Vec<CommandOperation>,
    /// Events produced by a synchronous FnExecuteCommand step. C++ applies
    /// these before FnExecuteCommand returns (C4Script.cpp:922-929), so the
    /// engine folds them with this callback rather than the next object tick.
    pub command_events: Vec<CommandEvent>,
    pub destroy_object: bool,
    /// Mutations nested script calls made to other objects, in first-call
    /// order. C++ mutates live state during the call; the copy-in/copy-out
    /// architecture applies them when the outer call commits.
    pub other_objects: Vec<NestedObjectOutcome>,
    pub environment: Option<EnvironmentDelta>,
    pub physics: Option<PhysicsDelta>,
    pub spawns: Vec<SpawnConfig>,
    pub landscape: Vec<LandscapeOperation>,
    /// Synchronous C4Object::UpdateSolidMask calls in their original host
    /// call order. Object/foreign-object outcome channels lose that order,
    /// so the engine replays this dedicated stream after state copy-out.
    pub(crate) solid_mask_operations: Vec<crate::HostSolidMaskOperation>,
    /// Callback-final COW raster used only as the next synchronous phase's
    /// read view; authoritative state comes from ordered operation replay.
    pub(crate) host_raster_preview: Option<HostRasterPreview>,
    pub particles: Vec<ParticleCommand>,
    pub transfer_zones: Vec<TransferZoneCommand>,
    pub messages: Vec<MessageCommand>,
    pub player_commands: Vec<PlayerCommand>,
    pub object_order_commands: Vec<ObjectOrderCommand>,
    pub next_mission_commands: Vec<NextMissionCommand>,
    pub menu_requests: Vec<crate::MenuRequest>,
    pub audio: AudioOutcome,
    pub trigger_game_over: bool,
    pub script_go: Option<bool>,
    /// Last synchronous write to `Game.Script.Counter` made during this VM
    /// call. The engine folds it even when the call subsequently errors.
    pub script_counter: Option<i32>,
    pub next_object_id: u64,
    /// VM-final locals of an effect callback that ran in its command
    /// target's own context (pFn->Exec(pCommandTarget, ...),
    /// C4Effect.cpp:129): the dispatch layer records them, the effect
    /// event loop persists them onto the object.
    pub context_locals: Option<HashMap<String, Value>>,
}

impl EffectContextOutcome {
    fn new(
        object: Vec<EffectCommand>,
        global: Vec<EffectCommand>,
        object_update: Option<ObjectUpdate>,
        object_commands: Vec<QueuedCommand>,
        command_operations: Vec<CommandOperation>,
        destroy_object: bool,
        environment: Option<EnvironmentDelta>,
        physics: Option<PhysicsDelta>,
        spawns: Vec<SpawnConfig>,
        landscape: Vec<LandscapeOperation>,
        transfer_zones: Vec<TransferZoneCommand>,
        messages: Vec<MessageCommand>,
        player_commands: Vec<PlayerCommand>,
        object_order_commands: Vec<ObjectOrderCommand>,
        audio: AudioOutcome,
        trigger_game_over: bool,
        script_go: Option<bool>,
        script_counter: Option<i32>,
        next_object_id: u64,
    ) -> Self {
        Self {
            object,
            global,
            object_update,
            object_commands,
            command_operations,
            command_events: Vec::new(),
            destroy_object,
            other_objects: Vec::new(),
            environment,
            physics,
            spawns,
            landscape,
            solid_mask_operations: Vec::new(),
            host_raster_preview: None,
            particles: Vec::new(),
            transfer_zones,
            messages,
            player_commands,
            object_order_commands,
            next_mission_commands: Vec::new(),
            menu_requests: Vec::new(),
            audio,
            trigger_game_over,
            script_go,
            script_counter,
            next_object_id,
            context_locals: None,
        }
    }

    pub(crate) fn empty(next_object_id: u64, audio: AudioRegistry) -> Self {
        Self {
            object: Vec::new(),
            global: Vec::new(),
            object_update: None,
            object_commands: Vec::new(),
            command_operations: Vec::new(),
            command_events: Vec::new(),
            destroy_object: false,
            other_objects: Vec::new(),
            environment: None,
            physics: None,
            spawns: Vec::new(),
            landscape: Vec::new(),
            solid_mask_operations: Vec::new(),
            host_raster_preview: None,
            particles: Vec::new(),
            transfer_zones: Vec::new(),
            messages: Vec::new(),
            player_commands: Vec::new(),
            object_order_commands: Vec::new(),
            next_mission_commands: Vec::new(),
            menu_requests: Vec::new(),
            audio: AudioOutcome {
                state: audio,
                events: Vec::new(),
            },
            trigger_game_over: false,
            script_go: None,
            script_counter: None,
            next_object_id,
            context_locals: None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum EffectScope {
    Object(Option<ObjectId>),
    Global,
}

#[derive(Debug)]
pub(crate) struct RandomContext {
    pub(crate) rng: RefCell<LcgRng>,
}

impl RandomContext {
    fn into_rng(self) -> LcgRng {
        self.rng.into_inner()
    }
}

pub(crate) struct RandomContextGuard {
    context: Option<Rc<RandomContext>>,
}

impl RandomContextGuard {
    pub fn finish(mut self) -> LcgRng {
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

/// Effect host functions accept ANY object as the state target (the
/// C4Effect operations attach to the GIVEN object, C4Effect.cpp): a
/// foreign target re-dispatches through the reentrancy seam so the
/// effect operation runs in the target's own scope (and folds with its
/// nested outcome). Returns None when the state is not a foreign object
/// — the caller proceeds locally.
pub(crate) fn redirect_foreign_effect_target(
    function: &'static str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    let target = match args.get(1) {
        Some(value @ (Value::Object(_) | Value::Proplist(_))) => object_id_from_value(value)?,
        _ => return None,
    };
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if Some(target) == active {
        return None;
    }
    Some(match call_world_object_function(target, function, args) {
        Some(result) => result,
        None => Ok(Value::Int(0)),
    })
}

pub(crate) fn with_context_mut<R>(
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

pub(crate) fn snapshot_effects_from_context(scope: EffectScope) -> Option<Vec<EffectState>> {
    HOST_CONTEXT.with(|cell| cell.borrow().as_ref().and_then(|ctx| ctx.snapshot(scope)))
}

pub(crate) fn with_effects_from_context<R>(
    scope: EffectScope,
    func: impl FnOnce(&[EffectState]) -> R,
) -> Option<R> {
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        context
            .scope(scope)
            .map(|scope| func(scope.effects.as_slice()))
    })
}

#[cfg(test)]
thread_local! {
    static EFFECT_SNAPSHOT_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn record_effect_snapshot() {
    EFFECT_SNAPSHOT_COUNT.set(EFFECT_SNAPSHOT_COUNT.get() + 1);
}

#[cfg(test)]
pub(crate) fn reset_effect_snapshot_count() {
    EFFECT_SNAPSHOT_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn effect_snapshot_count() -> usize {
    EFFECT_SNAPSHOT_COUNT.get()
}

pub(crate) fn determine_scope_from_state(value: &Value) -> Result<EffectScope, RuntimeError> {
    match value {
        // A zero C4Object payload is a null pointer. Every C++ effect native
        // selects Game.pGlobalEffects for nullptr; Object(None) is reserved for
        // the synthetic proplist fixture that means the active object's list.
        Value::Object(0) => Ok(EffectScope::Global),
        Value::Object(_) => Ok(EffectScope::Object(object_id_from_value(value))),
        Value::Proplist(_) => Ok(EffectScope::Object(None)),
        Value::Nil => Ok(EffectScope::Global),
        Value::Int(id) if *id == 0 => Ok(EffectScope::Global),
        other => Err(RuntimeError::new(format!(
            "effect host functions expected object, proplist, nil, or 0 for state, got {}",
            other.type_name()
        ))),
    }
}

pub(crate) fn extract_effects_from_state(state: &Value) -> Result<Vec<EffectState>, RuntimeError> {
    let map = match state {
        Value::Proplist(map) => map,
        Value::Object(_) => return Ok(Vec::new()),
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected object, proplist, or nil for state, got {}",
                other.type_name()
            )));
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
                    Some(Value::Int(value)) => *value,
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
                    Some(Value::C4Id(value)) if cast_c4id_payload(value) != 0 => {
                        Some(clonk_script::c4_id_from_raw(cast_c4id_payload(value)))
                    }
                    Some(Value::String(value)) if !value.is_empty() => {
                        let raw = clonk_script::c4_id_parse(value);
                        (raw != 0).then(|| clonk_script::c4_id_from_raw(raw))
                    }
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
                // Fixture proplists usually carry no allocated iNumber; the
                // list position + 1 stands in so by-number lookups
                // (GetEffect without name, EffectVar, EffectCall) stay
                // usable on snapshot state.
                effect.number = match props.get("number") {
                    Some(Value::Int(value)) if *value > 0 => *value,
                    _ => i32::try_from(effects.len() + 1).unwrap_or(i32::MAX),
                };
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

/// A completed nested call's scope plus its VM-final local variables, kept so
/// a later nested call on the same object resumes from the accumulated state
/// (C++ mutates live state, so repeat calls see earlier changes).
pub(crate) struct NestedScopeState {
    pub(crate) scope: ObjectScopeContext,
    pub(crate) local_vars: HashMap<String, Value>,
}

/// Where a nested call's scope came from (and must return to).
enum NestedScopeOrigin {
    /// `dormant_scopes[index]` — the target is an in-flight outer call.
    Dormant(usize),
    /// The completed-call map (or a fresh snapshot scope).
    Completed,
}

/// Phase-1 result of [`EffectHostContext::prepare_nested_call`]: everything
/// the caller needs to run the nested VM after releasing the borrow.
/// `origin: None` means the target was already the active scope.
struct NestedCallPrep {
    script: Arc<ScriptEngine>,
    local_vars: HashMap<String, Value>,
    origin: Option<NestedScopeOrigin>,
}

/// Runs `function` on `target`'s definition script from inside a running VM
/// call — the host→VM reentrancy seam (C4FindObjectFunc::Check,
/// C4FindObject.cpp:653-662: `pCallFunc->Exec(pObj, Pars, true)`): the
/// target object is the call context (`this`), never a parameter. Returns
/// `None` when the function is not visible to the target (C++ fails the
/// check silently) and `Some(Err(_))` for runtime errors (`fPassErrors=true`
/// — the caller rethrows, aborting the calling script).
/// Registers the ACTIVE outer call's live local cells so nested calls and
/// cross-object LocalN/Local references onto the same object mutate the
/// running session's storage (C++ mutates the one live C4Object). The
/// per-callback host context owns the entry's lifetime.
pub(crate) fn register_session_local_cells(target: ObjectId, cells: clonk_script::LocalCells) {
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.session_local_cells.insert(target, cells);
        }
    });
}

pub(crate) fn call_world_object_function(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, true, true, None, false)
}

fn call_world_object_function_from_arrow(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, true, true, None, true)
}

fn call_world_object_reference_from_arrow(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<clonk_script::ValueReference, RuntimeError>> {
    call_world_object_reference_with(target, function, args, true, None, true)
}

/// Runs a function from an already-resolved script owner with `target` as
/// its object context. Native paths such as GetCustomComponents use this
/// exact-owner dispatch; arrow calls must instead re-resolve on the target.
pub(crate) fn call_world_object_function_in_scope(
    target: ObjectId,
    script: Arc<ScriptEngine>,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, false, false, Some(script), false)
}

fn call_world_object_reference_with(
    target: ObjectId,
    function: &str,
    args: &[Value],
    include_globals: bool,
    script_override: Option<Arc<ScriptEngine>>,
    preserve_caller: bool,
) -> Option<Result<clonk_script::ValueReference, RuntimeError>> {
    let prep = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut().as_mut().and_then(|context| {
            context.prepare_nested_call(
                target,
                function,
                false,
                include_globals,
                script_override,
                false,
                false,
            )
        })
    })?;
    let NestedCallPrep {
        script,
        local_vars,
        origin,
    } = prep;
    let entry_locals = local_vars.clone();
    let (cells, created_session) = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        match borrow.as_mut() {
            Some(context) => match context.session_local_cells.get(&target) {
                Some(cells) => (cells.clone(), false),
                None => {
                    let cells = clonk_script::LocalCells::from_local_vars(&local_vars);
                    context.session_local_cells.insert(target, cells.clone());
                    (cells, true)
                }
            },
            None => (
                clonk_script::LocalCells::from_local_vars(&local_vars),
                false,
            ),
        }
    });
    let (previous_script_object, previous_script_definition) =
        with_host_context_mut((None, None), |context| {
            let definition = context.object_effective_definition_id(target);
            (
                context.script_object_context.replace(target),
                context.script_definition_context.replace(definition),
            )
        });
    let this = object_reference_value(target);
    let call = if preserve_caller {
        script.call_reference_with_cells_and_this_preserving_caller(function, args, &cells, this)
    } else {
        script.call_reference_with_cells_and_this(function, args, &cells, this)
    };
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.script_object_context = previous_script_object;
            context.script_definition_context = previous_script_definition;
        }
    });
    let succeeded = call.is_ok();
    if created_session && !succeeded {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.session_local_cells.remove(&target);
            }
        });
    }
    let result = match call {
        Ok(reference) => Ok(reference),
        Err(clonk_script::ScriptError::Runtime(err)) => Err(err),
        Err(other) => Err(RuntimeError::new(other.to_string())),
    };
    let stored_locals = cells.snapshot();
    if let Some(origin) = origin {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                let mut stored_locals = stored_locals;
                for ((object, name), slot) in &context.foreign_local_cells {
                    if *object != target {
                        continue;
                    }
                    let outer_unchanged = entry_locals.get(name).unwrap_or(&Value::Nil)
                        == stored_locals.get(name).unwrap_or(&Value::Nil);
                    if outer_unchanged {
                        stored_locals.insert(name.clone(), slot.borrow().clone());
                    }
                }
                context.finish_nested_call(target, origin, stored_locals);
            }
        });
    }
    Some(result)
}

/// Like [`call_world_object_function`], but resolves SCRIPT functions only —
/// the owner-scoped `GetSFunc` lookup the Call family uses (C4Aul.cpp:
/// 295-298, 562-576): engine (host) functions are never found, unlike
/// Find_Func's `FindSameNameFunc` engine fallback.
pub(crate) fn call_world_object_script_function(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, false, true, None, false)
}

/// Object-call resolution (`C4Object::Call` -> GetSFunc): the target's OWN
/// script chain only — engine-global script functions do NOT resolve
/// (unlike GetFuncRecursive). Creation callbacks (PSF_Construction /
/// PSF_Initialize) use this; resolving a same-name scenario global here
/// would recurse (a def without Initialize must be a silent miss).
pub(crate) fn call_world_object_own_function(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with(target, function, args, false, false, None, false)
}

/// ActMap callbacks carry the exact function retained during definition
/// linking. Unlinked synthetic fixtures preserve their name-based fallback.
pub(crate) fn call_world_object_script_callback(
    target: ObjectId,
    callback: &ScriptCallbackTarget,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    match callback.resolution() {
        Some(resolution) => call_world_object_function_with_options(
            target,
            callback.function_name(),
            args,
            false,
            false,
            None,
            false,
            false,
            Some(resolution.clone()),
            false,
            EffectCallbackParameterConversionPolicy::Standard,
        )
        .map(|outcome| outcome.map(|(value, _)| value)),
        None => call_world_object_own_function(target, callback.function_name(), args),
    }
}

/// C4Object::Call for a scope that may still be in pre-insertion
/// Construction/Initialize. Kept private to callbacks that C++ must run
/// synchronously from that state; ordinary nested calls still require a
/// world object.
fn call_world_object_own_function_inflight(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with_options(
        target,
        function,
        args,
        false,
        false,
        None,
        false,
        true,
        None,
        false,
        EffectCallbackParameterConversionPolicy::Standard,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

/// C4Effect callbacks may target the object whose Construction callback is
/// currently running, before Rust inserts it into HostWorldContext. C++
/// already has that C4Object pointer and executes recursive/global/native
/// callback lookup with it as `this` (C4Effect.cpp:42-56,439-456).
pub(crate) fn call_world_object_function_inflight(
    target: ObjectId,
    function: &str,
    args: &[Value],
    parameter_conversion: EffectCallbackParameterConversionPolicy,
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with_options(
        target,
        function,
        args,
        true,
        true,
        None,
        false,
        true,
        None,
        false,
        parameter_conversion,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

/// Execute an engine-global function already selected through a command
/// object's definition while retaining that object's live `this` and locals.
/// The pinned body and exact `LinkedTo` host prevent the command definition's
/// ordinary helpers from shadowing helpers owned by the global declaration.
pub(crate) fn call_world_object_resolved_global_function(
    target: ObjectId,
    script: Arc<ScriptEngine>,
    resolution: clonk_script::ScriptFunctionResolution,
    function: &str,
    args: &[Value],
    parameter_conversion: EffectCallbackParameterConversionPolicy,
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with_options(
        target,
        function,
        args,
        false,
        true,
        Some(script),
        false,
        true,
        Some(resolution),
        false,
        parameter_conversion,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

fn call_world_object_function_with(
    target: ObjectId,
    function: &str,
    args: &[Value],
    host_fallback: bool,
    include_globals: bool,
    script_override: Option<Arc<ScriptEngine>>,
    preserve_caller: bool,
) -> Option<Result<Value, RuntimeError>> {
    call_world_object_function_with_options(
        target,
        function,
        args,
        host_fallback,
        include_globals,
        script_override,
        preserve_caller,
        false,
        None,
        false,
        EffectCallbackParameterConversionPolicy::Standard,
    )
    .map(|outcome| outcome.map(|(value, _)| value))
}

/// [`call_world_object_function_from_arrow`] for a callee that declares `&`
/// parameters: also returns each parameter slot's final value so the calling
/// VM can settle its reference cells.
fn call_world_object_ref_args_from_arrow(
    target: ObjectId,
    function: &str,
    args: &[Value],
) -> Option<Result<(Value, Vec<Value>), RuntimeError>> {
    call_world_object_function_with_options(
        target,
        function,
        args,
        true,
        true,
        None,
        true,
        false,
        None,
        true,
        EffectCallbackParameterConversionPolicy::Standard,
    )
}

fn call_world_object_function_with_options(
    target: ObjectId,
    function: &str,
    args: &[Value],
    host_fallback: bool,
    include_globals: bool,
    script_override: Option<Arc<ScriptEngine>>,
    preserve_caller: bool,
    allow_scope_without_world_object: bool,
    pinned_resolution: Option<clonk_script::ScriptFunctionResolution>,
    ref_args: bool,
    parameter_conversion: EffectCallbackParameterConversionPolicy,
) -> Option<Result<(Value, Vec<Value>), RuntimeError>> {
    let prep = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut().as_mut().and_then(|context| {
            context.prepare_nested_call(
                target,
                function,
                host_fallback,
                include_globals,
                script_override,
                allow_scope_without_world_object,
                pinned_resolution.is_some(),
            )
        })
    })?;
    let NestedCallPrep {
        script,
        local_vars,
        origin,
    } = prep;
    let entry_locals = local_vars.clone();
    // LIVE local cells shared across every session on this object within
    // the outer call (C++ mutates the one live C4Object): the first
    // session seeds them from the snapshot, deeper sessions reuse them —
    // and their writes are visible mid-call. The creating session owns
    // cleanup.
    let (cells, created_session) = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        match borrow.as_mut() {
            Some(context) => match context.session_local_cells.get(&target) {
                Some(cells) => (cells.clone(), false),
                None => {
                    let cells = clonk_script::LocalCells::from_local_vars(&local_vars);
                    context.session_local_cells.insert(target, cells.clone());
                    (cells, true)
                }
            },
            None => (
                clonk_script::LocalCells::from_local_vars(&local_vars),
                false,
            ),
        }
    });
    // The HOST_CONTEXT borrow is released here: the nested VM's host
    // functions re-borrow it against the swapped-in scope.
    let (previous_script_object, previous_script_definition) =
        with_host_context_mut((None, None), |context| {
            let definition = context.object_effective_definition_id(target);
            (
                context.script_object_context.replace(target),
                context.script_definition_context.replace(definition),
            )
        });
    let this = object_reference_value(target);
    let unchanged_finals = || args.to_vec();
    let call = if ref_args {
        debug_assert!(preserve_caller && pinned_resolution.is_none());
        debug_assert_eq!(
            parameter_conversion,
            EffectCallbackParameterConversionPolicy::Standard
        );
        script.call_ref_args_with_cells_and_this_preserving_caller(function, args, &cells, this)
    } else if let Some(resolution) = pinned_resolution {
        debug_assert!(!preserve_caller);
        let call =
            if parameter_conversion == EffectCallbackParameterConversionPolicy::WarnForNonStrict3 {
                script.call_resolved_with_cells_and_this_for_effect_callback(
                    &resolution,
                    resolution.scope == clonk_script::ScriptFunctionScope::Global,
                    args,
                    &cells,
                    this,
                )
            } else {
                script.call_resolved_with_cells_and_this(
                    &resolution,
                    resolution.scope == clonk_script::ScriptFunctionScope::Global,
                    args,
                    &cells,
                    this,
                )
            };
        call.map(|value| (value, unchanged_finals()))
    } else if preserve_caller {
        debug_assert_eq!(
            parameter_conversion,
            EffectCallbackParameterConversionPolicy::Standard
        );
        script
            .call_with_cells_and_this_preserving_caller(function, args, &cells, this)
            .map(|value| (value, unchanged_finals()))
    } else {
        let call =
            if parameter_conversion == EffectCallbackParameterConversionPolicy::WarnForNonStrict3 {
                script.call_effect_callback_with_cells_and_this(function, args, &cells, this)
            } else {
                script.call_with_cells_and_this(function, args, &cells, this)
            };
        call.map(|value| (value, unchanged_finals()))
    };
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.script_object_context = previous_script_object;
            context.script_definition_context = previous_script_definition;
        }
    });
    if created_session {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.session_local_cells.remove(&target);
            }
        });
    }
    let (result, stored_locals) = match call {
        Ok(value) => (Ok(value), cells.snapshot()),
        // Partial side effects before the error still fold (C++ mutates
        // live state) — the shared cells carry every write made before
        // the unwind.
        Err(clonk_script::ScriptError::Runtime(err)) => (Err(err), cells.snapshot()),
        Err(other) => (Err(RuntimeError::new(other.to_string())), cells.snapshot()),
    };
    if let Some(origin) = origin {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                // Writes made by DEEPER same-scope calls (e.g. a
                // synchronous Fx*Start fired from inside this call) sit in
                // the foreign cells; they win over entries THIS call left
                // untouched — C++ mutates the one live object, so the
                // deepest write is simply the latest.
                let mut stored_locals = stored_locals;
                for ((object, name), slot) in &context.foreign_local_cells {
                    if *object != target {
                        continue;
                    }
                    // Unset locals read as nil (C4Value default) — a local
                    // the outer call never touched may be absent from its
                    // entry snapshot but nil-present in the VM result.
                    let outer_unchanged = entry_locals.get(name).unwrap_or(&Value::Nil)
                        == stored_locals.get(name).unwrap_or(&Value::Nil);
                    if outer_unchanged {
                        stored_locals.insert(name.clone(), slot.borrow().clone());
                    }
                }
                context.finish_nested_call(target, origin, stored_locals);
            }
        });
    } else {
        // Same-scope call (the target IS the in-flight active scope): the
        // outer VM owns the live locals, so fold this call's writes
        // through the foreign-cell channel — later nested calls overlay
        // them and the outcome fold persists them (C++ mutates the live
        // object directly).
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                for (name, value) in &stored_locals {
                    let slot = context.foreign_local_cell(target, name);
                    *slot.borrow_mut() = value.clone();
                }
            }
        });
    }
    Some(result)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContentsLinkOperation {
    Remove {
        container: ObjectId,
        child: ObjectId,
    },
    Insert {
        container: ObjectId,
        child: ObjectId,
        position: usize,
    },
    MoveToBack {
        container: ObjectId,
        child: ObjectId,
    },
    RotateToFront {
        container: ObjectId,
        child: ObjectId,
    },
}

impl ContentsLinkOperation {
    fn container(self) -> ObjectId {
        match self {
            Self::Remove { container, .. }
            | Self::Insert { container, .. }
            | Self::MoveToBack { container, .. }
            | Self::RotateToFront { container, .. } => container,
        }
    }

    fn mutates_link(self, container: ObjectId, child: ObjectId) -> bool {
        match self {
            Self::Remove {
                container: operation_container,
                child: operation_child,
            }
            | Self::Insert {
                container: operation_container,
                child: operation_child,
                ..
            }
            | Self::MoveToBack {
                container: operation_container,
                child: operation_child,
            } => operation_container == container && operation_child == child,
            Self::RotateToFront { .. } => false,
        }
    }

    fn reorders(self, container: ObjectId) -> bool {
        matches!(
            self,
            Self::MoveToBack {
                container: operation_container,
                ..
            } | Self::RotateToFront {
                container: operation_container,
                ..
            } if operation_container == container
        )
    }
}

pub(crate) struct EffectHostContext {
    object: Option<ObjectScopeContext>,
    /// Definition context for no-object script execution (`cthr->Def`).
    /// InitializeDef and similar definition-owned callbacks retain this
    /// even though `cthr->Obj` is null (C4AulExec.cpp:343-352).
    definition_context: Option<DefinitionId>,
    /// `cthr->Obj`, independently of the affected object whose mutable
    /// state this host context carries. Definition-commanded effects keep
    /// their carrier in `object` but execute with a null script object.
    pub(crate) script_object_context: Option<ObjectId>,
    /// `cthr->Def` captured when the active script frame was entered.
    /// ChangeDef mutates `cthr->Obj->Def`, but the suspended frame keeps its
    /// original definition until a nested object/definition call replaces
    /// the whole script context (C4AulExec.cpp:343-352, 1417-1456).
    script_definition_context: Option<Option<DefinitionId>>,
    global: Option<EffectScopeContext>,
    /// LIVE local cells per object with an in-flight VM session: deeper
    /// nested calls onto the same object share them, so mid-call local
    /// writes are visible immediately (C++ mutates the live C4Object —
    /// the Talker's DoStartMovie sets sMovName and the synchronous
    /// FxMovieStart reads it within the same outer call).
    pub(crate) session_local_cells: HashMap<ObjectId, clonk_script::LocalCells>,
    /// Objects removed during this synchronous script call. C++
    /// C4Object::AssignRemoval clears every registered C4Value reference
    /// before returning (C4Object.cpp:312); later nested calls must not
    /// reload those references from the frame-start world snapshot.
    pub(crate) removed_object_references: HashSet<ObjectId>,
    /// Contents links removed by `AssignRemoval` while the child's
    /// `Contained` pointer deliberately still names its dead parent.
    /// C++ removes the list link before recursing into the child and only
    /// clears `Contained` at the end of the child's own cleanup
    /// (C4Object.cpp:287-306).
    unlinked_content_links: HashSet<(ObjectId, ObjectId)>,
    /// Contents links that were removed and may be re-added during this
    /// call. Unlike `unlinked_content_links`, these are omitted from the
    /// snapshot base but remain eligible for the live Enter-growth pass.
    relinked_content_links: HashSet<(ObjectId, ObjectId)>,
    /// Exact chronological contents-link mutations made during this VM
    /// invocation. C4ObjectList::Add chooses its stContents position at the
    /// instant Enter runs (C4ObjectList.cpp:147-175); rebuilding from final
    /// scopes would reorder already-scoped equal-key re-entries.
    contents_link_operations: Vec<ContentsLinkOperation>,
    /// Callback-private copy of every active grid solid-mask bake. This is
    /// updated together with `world.landscape` so nested callbacks observe
    /// C++'s immediate C4SolidMask Remove/Put lifecycle.
    solid_mask_bakes: Rc<Vec<(ObjectId, crate::SolidMaskBake)>>,
    /// Live instance ages also cover eligible masks clipped fully outside
    /// the raster and therefore absent from `solid_mask_bakes`.
    solid_mask_instance_sequences: Rc<RefCell<HashMap<ObjectId, u64>>>,
    next_solid_mask_instance_sequence: Rc<Cell<u64>>,
    /// C4Object::UpdateSolidMask calls made by this VM invocation, retained
    /// independently of the outer/foreign object outcome split.
    solid_mask_operations: Vec<crate::HostSolidMaskOperation>,
    pub(crate) world: HostWorldContext,
    /// Mutable C4Def metadata preview for this synchronous VM session.
    /// Definition writes are folded into Engine after the callback returns,
    /// but later host calls must already observe them.
    definition_metadata_overrides: HashMap<DefinitionId, DefinitionMetadata>,
    player_overrides: HashMap<i32, PlayerState>,
    /// Live C4TeamList projection for this synchronous VM session. Runtime
    /// TEAMID_New generation must be visible to GetTeam* immediately and to
    /// callbacks nested later in the same outer call.
    teams: Vec<TeamInfo>,
    player_commands: Vec<PlayerCommand>,
    object_order_commands: Vec<ObjectOrderCommand>,
    /// Same-VM-call logical Game.Objects view after global Resort(). The
    /// authoritative engine applies the sort when the host batch returns;
    /// this preview exposes its synchronous C++ visibility meanwhile.
    master_order_preview: Option<Vec<ObjectId>>,
    pub(crate) next_mission_commands: Vec<NextMissionCommand>,
    team_home_base_rule: bool,
    pub(crate) pending_spawns: Vec<SpawnConfig>,
    pub(crate) pending_objects: HashMap<ObjectId, HostWorldObject>,
    pending_order: Vec<ObjectId>,
    pending_particles: Vec<ParticleCommand>,
    transfer_zone_commands: Vec<TransferZoneCommand>,
    pending_messages: Vec<MessageCommand>,
    pub(crate) pending_menu_requests: Vec<crate::MenuRequest>,
    /// Non-menu events emitted by FnExecuteCommand, kept in call order for
    /// the synchronous callback-outcome fold.
    pub(crate) pending_command_events: Vec<CommandEvent>,
    pending_landscape_ops: Vec<LandscapeOperation>,
    /// Live C4Object::MaterialContents overlays for DigFree/DigFreeRect.
    /// Entries are seeded lazily from HostWorldObject and copied into each
    /// affected object's final ObjectUpdate.
    dig_material_contents: HashMap<ObjectId, Vec<i32>>,
    /// Live C4TextureMap preview for synchronous GetIndexMatTex return
    /// values. DrawMaterialQuad/DrawMap operations still fold into the real
    /// engine, but later calls in this same VM session must see slots
    /// allocated by earlier calls (C4Texture.cpp:319-369); the companion COW
    /// Landscape supplies their same-callback pixel visibility.
    runtime_texmap: OnceCell<Option<crate::landscape::RuntimeTexMapState>>,
    /// Live script-visible sky values. Host writes update this before their
    /// deferred landscape operation is folded into the engine.
    pub(crate) sky_adjustment: SkyAdjustment,
    audio: AudioRegistry,
    next_object_id: u64,
    trigger_game_over: bool,
    pub(crate) script_go_request: Option<bool>,
    pub(crate) scenario_script_counter: i32,
    pub(crate) script_counter_request: Option<i32>,
    game_over_triggered: bool,
    /// Saved `object` scopes of in-flight nested calls, one per nesting
    /// level (`None` = the level had no object scope). The active scope is
    /// always `object`; scopes move between locations by identity, so one
    /// object never has two scopes (no double-apply on fold).
    dormant_scopes: Vec<Option<ObjectScopeContext>>,
    /// Script/definition contexts paired with each AB_CALLGLOBAL suspension.
    /// Object scopes use `dormant_scopes` so explicit-object natives and
    /// nested arrow calls can still reach the suspended caller object.
    global_call_contexts: Vec<(
        Option<ObjectId>,
        Option<DefinitionId>,
        Option<Option<DefinitionId>>,
    )>,
    /// Completed nested-call scopes by target, resumed on repeat calls and
    /// folded into `EffectContextOutcome::other_objects` in first-call order.
    pub(crate) nested_objects: HashMap<ObjectId, NestedScopeState>,
    pub(crate) nested_order: Vec<ObjectId>,
    /// Live cells handed to the VM for cross-object LocalN references
    /// (FnLocalN by-reference access, C4Script.cpp:4591-4605): seeded from
    /// the target's current locals, overlaid into nested calls, synced
    /// back after them, and folded into the outcomes. Targets whose scope
    /// is the suspended OUTER call see the pre-call snapshot (the same
    /// divergence prepare_nested_call documents).
    foreign_local_cells: HashMap<(ObjectId, String), clonk_script::ValueCell>,
}

impl EffectHostContext {
    fn new(
        object: Option<HostObjectContext<'_>>,
        definition_context: Option<DefinitionId>,
        script_object_context: Option<ObjectId>,
        global_effects: Vec<EffectState>,
        world: HostWorldContext,
        next_object_id: u64,
        audio: AudioRegistry,
        game_over_triggered: bool,
    ) -> Self {
        let team_home_base_rule = world.team_home_base_rule();
        let scenario_script_counter = world.scenario_script_counter();
        let sky_adjustment = world.sky_adjustment();
        let teams = world.teams().to_vec();
        let solid_mask_bakes = Rc::clone(&world.solid_mask_bakes);
        let solid_mask_instance_sequences = Rc::clone(&world.solid_mask_instance_sequences);
        let next_solid_mask_instance_sequence = Rc::clone(&world.next_solid_mask_instance_sequence);
        let resolved_script_definition = definition_context.clone().or_else(|| {
            script_object_context.and_then(|script_object| {
                object
                    .as_ref()
                    .filter(|object| object.id == script_object)
                    .and_then(|object| object.definition_id.as_deref())
                    .map(DefinitionId::from)
                    .or_else(|| {
                        world
                            .get_shared(script_object)
                            .map(|object| DefinitionId::from(object.definition_id()))
                    })
            })
        });
        // A null-object frame is authoritatively `Def=null`. Object-only
        // unit fixtures that supply neither a definition id nor a world
        // object have no modeled VM frame, so preserve their synthetic
        // ActionLibrary fallback instead of treating the missing metadata as
        // an authoritative null definition.
        let script_definition_context =
            if resolved_script_definition.is_some() || script_object_context.is_none() {
                Some(resolved_script_definition)
            } else {
                None
            };
        let mut object = object.map(|ctx| {
            let HostObjectContext {
                id,
                definition_id,
                container,
                status,
                energy,
                breath,
                need_energy,
                magic_energy,
                damage,
                construction,
                alive,
                in_liquid,
                own_mass,
                owner,
                controller,
                position,
                velocity,
                rotation,
                effects,
                action_name,
                action_index,
                action_ticks,
                action_data,
                action_phase,
                action_library,
                direction,
                command_direction,
                command_count,
                action_target,
                action_target2,
                vertices,
                shape_vertices,
                graphics_overlays,
                base_graphics,
                category,
                ocf,
                ocf_base,
                crew_member,
                plr_view_range,
                draw_transform,
                info_physical,
                temporary_physical,
                physical_changes,
                definition_physical,
                walk_rotation,
                script_fixed_position,
                script_fixed_velocity,
                script_rotation_velocity,
                script_fixed_rotation,
            } = ctx;
            {
                let mut scope = ObjectScopeContext::new(
                    id,
                    container,
                    status,
                    energy,
                    damage,
                    construction,
                    alive,
                    in_liquid,
                    own_mass,
                    owner,
                    controller,
                    category,
                    position,
                    velocity,
                    rotation,
                    effects.to_vec(),
                    action_library,
                    action_name,
                    action_index,
                    action_ticks,
                    action_data,
                    action_phase,
                    direction,
                    command_direction,
                    command_count,
                    action_target,
                    action_target2,
                    shape_vertices,
                    ocf_base,
                    crew_member,
                    plr_view_range,
                    graphics_overlays,
                    base_graphics,
                    draw_transform,
                    info_physical,
                    temporary_physical,
                    physical_changes,
                    definition_physical,
                );
                scope.current_info_rank = world
                    .crew_rank(scope.id().as_u64())
                    .or_else(|| scope.info_physical.map(|_| 0));
                scope.current_info_link = world.crew_info_link(scope.id());
                scope.current_info_core = world.crew_infos.get(&scope.id()).cloned();
                scope.definition_id = definition_id;
                scope.configure_fair_crew(&world);
                // FnGetOCF reads the cached obj->OCF (C4Script.cpp:1354-1358).
                scope.cached_ocf = Some(ocf);
                scope.walk_rotation = walk_rotation;
                scope.current_t_attach = walk_rotation.t_attach;
                scope.current_magic_energy = magic_energy;
                scope.current_breath = breath;
                scope.current_need_energy = need_energy;
                scope.current_selected = world
                    .get_shared(scope.id())
                    .is_some_and(|object| object.selected);
                scope.current_no_collect_delay = world
                    .get_shared(scope.id())
                    .map(|object| object.no_collect_delay)
                    .unwrap_or(0);
                if let Some(position) = script_fixed_position {
                    scope.current_fixed_position = position;
                }
                // Seed the TRUE fixed dirs when the caller provided them —
                // GetXDir must see a 0.4 px/f drift as 4 at precision 10
                // like C++ reading pObj->xdir (C4Script.cpp:1167).
                if let Some(velocity) = script_fixed_velocity {
                    scope.current_fixed_velocity = velocity;
                }
                if let Some(rotation_velocity) = script_rotation_velocity {
                    scope.current_rotation_velocity = rotation_velocity;
                }
                if let Some(fixed_rotation) = script_fixed_rotation {
                    scope.current_fixed_rotation = fixed_rotation;
                }
                scope
            }
        });
        if let Some(scope) = object.as_mut() {
            if let Some(world_object) = world.get_shared(scope.id()) {
                scope.current_compiler_cache = world_object.compiler_cache.clone();
                scope.unsorted = world_object.unsorted;
                scope.staged_own_vertices = world_object.own_vertices;
                scope
                    .live_commands
                    .restore_from_snapshot(&world_object.command_stack);
                scope.command_count = scope.live_commands.len();
                if let Some(state) = world_object.full_state() {
                    scope.current_mobile = state.mobile;
                    scope.current_t_attach = state.t_attach;
                    scope.current_contact_density = state.contact_density;
                    scope.current_contents_link_generation = state.contents_link_generation;
                    scope.current_plr_view_range = state.plr_view_range;
                    scope.walk_rotation.t_attach = state.t_attach;
                }
                scope.current_rotation_velocity = world_object.rotation_velocity;
                scope.current_fixed_rotation = world_object.fixed_rotation;
            }
        }
        let global = Some(EffectScopeContext::new(global_effects));
        Self {
            object,
            definition_context,
            script_object_context,
            script_definition_context,
            global,
            solid_mask_bakes,
            solid_mask_instance_sequences,
            next_solid_mask_instance_sequence,
            solid_mask_operations: Vec::new(),
            world,
            definition_metadata_overrides: HashMap::new(),
            player_overrides: HashMap::new(),
            teams,
            player_commands: Vec::new(),
            object_order_commands: Vec::new(),
            master_order_preview: None,
            next_mission_commands: Vec::new(),
            team_home_base_rule,
            pending_spawns: Vec::new(),
            pending_objects: HashMap::new(),
            pending_order: Vec::new(),
            pending_particles: Vec::new(),
            transfer_zone_commands: Vec::new(),
            pending_messages: Vec::new(),
            pending_menu_requests: Vec::new(),
            pending_command_events: Vec::new(),
            pending_landscape_ops: Vec::new(),
            dig_material_contents: HashMap::new(),
            runtime_texmap: OnceCell::new(),
            sky_adjustment,
            audio,
            next_object_id,
            trigger_game_over: false,
            script_go_request: None,
            scenario_script_counter,
            script_counter_request: None,
            game_over_triggered,
            dormant_scopes: Vec::new(),
            global_call_contexts: Vec::new(),
            nested_objects: HashMap::new(),
            session_local_cells: HashMap::new(),
            removed_object_references: HashSet::new(),
            unlinked_content_links: HashSet::new(),
            relinked_content_links: HashSet::new(),
            contents_link_operations: Vec::new(),
            nested_order: Vec::new(),
            foreign_local_cells: HashMap::new(),
        }
    }

    pub(crate) fn scope_mut(
        &mut self,
        scope: EffectScope,
    ) -> Result<&mut EffectScopeContext, RuntimeError> {
        match scope {
            EffectScope::Object(Some(target)) => self
                .object_scope_mut(target)
                .map(|ctx| &mut ctx.effects)
                .ok_or_else(|| {
                    RuntimeError::new("object effect operations require a live target context")
                }),
            EffectScope::Object(None) => self
                .object
                .as_mut()
                .map(|ctx| &mut ctx.effects)
                .ok_or_else(|| {
                    RuntimeError::new("object effect operations require an active engine context")
                }),
            EffectScope::Global => self.global.as_mut().ok_or_else(|| {
                RuntimeError::new("global effect operations require an active engine context")
            }),
        }
    }

    fn scope(&self, scope: EffectScope) -> Option<&EffectScopeContext> {
        match scope {
            EffectScope::Object(Some(target)) => self.object_scope(target).map(|ctx| &ctx.effects),
            EffectScope::Object(None) => self.object.as_ref().map(|ctx| &ctx.effects),
            EffectScope::Global => self.global.as_ref(),
        }
    }

    pub(crate) fn allocate_object_id(&mut self) -> ObjectId {
        let id = ObjectId::new(self.next_object_id);
        self.next_object_id += 1;
        id
    }

    pub(crate) fn register_spawn(&mut self, spawn: SpawnConfig, mut preview: HostWorldObject) {
        let id = preview.id;
        // C4Object::Init copies Def->SolidMask and checks it against the
        // already-selected base bitmap before Construction/Initialize can
        // observe the object (C4Object.cpp:172-174,206-211).
        if !spawn.loaded {
            if let Some(metadata) = self.world.solid_mask_metadata.get(&preview.definition_id) {
                if let Some(raw) = spawn.solid_mask.or(metadata.default_mask) {
                    if let Some(checked) = metadata.check_mask_rect(raw, None) {
                        if spawn.solid_mask.is_some() || checked != raw {
                            if let Some(state) = preview.state.as_mut() {
                                Rc::make_mut(state).solid_mask_override = Some(checked);
                            }
                        }
                    }
                }
            }
        }
        if !self.pending_objects.contains_key(&id) {
            self.pending_order.push(id);
        }
        self.pending_objects.insert(id, preview);
        self.pending_spawns.push(spawn);
        if self.master_order_preview.is_some() {
            self.preview_sort_master_by_category();
        }
    }

    fn live_solid_mask_rect(&self, id: ObjectId) -> Option<crate::DefinitionTargetRect> {
        let scope = self.object_scope(id)?;
        let definition_id = self.object_effective_definition_id(id)?;
        let definition = self.world.solid_mask_metadata.get(&definition_id)?;
        // ChangeDef clears the old object's SolidMask override at the swap
        // point; otherwise the frame-start override remains effective until
        // a same-call SetSolidMask replaces it.
        let persisted_override = scope
            .pending_update
            .change_def
            .is_none()
            .then(|| {
                self.get_world_object(id).and_then(|object| {
                    object
                        .full_state()
                        .and_then(|state| state.solid_mask_override)
                })
            })
            .flatten();
        scope
            .pending_update
            .solid_mask_override
            .or(persisted_override)
            .or(definition.default_mask)
    }

    pub(crate) fn check_solid_mask_rect_for_object(
        &self,
        id: ObjectId,
        mask: crate::DefinitionTargetRect,
    ) -> Option<crate::DefinitionTargetRect> {
        let scope = self.object_scope(id)?;
        let definition_id = self.object_effective_definition_id(id)?;
        let (graphics_definition, graphics_name) = scope
            .base_graphics
            .as_ref()
            .map(|graphics| {
                (
                    graphics.definition.as_str(),
                    graphics.graphics_name.as_deref(),
                )
            })
            .unwrap_or((definition_id.as_str(), None));
        self.world
            .solid_mask_metadata
            .get(graphics_definition)?
            .check_mask_rect(mask, graphics_name)
    }

    /// The effective parameters C4Object::UpdateSolidMask would use for a
    /// live object at this exact point in the synchronous callback.
    pub(crate) fn live_solid_mask_spec(&self, id: ObjectId) -> Option<crate::SolidMaskSpec> {
        let scope = self.object_scope(id)?;
        if scope.destroy
            || matches!(scope.status(), ObjectStatus::Deleted)
            || scope.container().is_some()
            || scope.construction() < FULL_CON
        {
            return None;
        }
        let definition_id = self.object_effective_definition_id(id)?;
        let definition = self.world.solid_mask_metadata.get(&definition_id)?;
        let rotation = scope.rotation();
        if rotation != 0 && !definition.rotated_solid_masks {
            return None;
        }
        let mask = self.live_solid_mask_rect(id)?;
        if !mask.is_positive() {
            return None;
        }
        let (graphics_definition, graphics_name) = scope
            .base_graphics
            .as_ref()
            .map(|graphics| {
                (
                    graphics.definition.as_str(),
                    graphics.graphics_name.as_deref(),
                )
            })
            .unwrap_or((definition_id.as_str(), None));
        let pixels = self
            .world
            .solid_mask_metadata
            .get(graphics_definition)?
            .pixels_for_checked_mask(mask, graphics_name)?;
        let shape = definition.shape.unwrap_or_default();
        Some(crate::SolidMaskSpec {
            mask,
            pixels,
            shape_x: shape.x,
            shape_y: shape.y,
            rotation,
        })
    }

    /// Raster-only C4SolidMask::Remove for the callback-private landscape.
    /// It restores saved bytes and re-puts every overlapping bake, including
    /// refreshing those masks' buffers exactly like C4SolidMask.cpp:233-283.
    fn remove_live_solid_mask(&mut self, id: ObjectId) -> Option<(usize, u64)> {
        if !self
            .solid_mask_bakes
            .iter()
            .any(|(object_id, _)| *object_id == id)
        {
            return None;
        }
        let landscape = self.world.landscape_mut()?;
        remove_host_solid_mask_raster(landscape, Rc::make_mut(&mut self.solid_mask_bakes), id)
    }

    fn allocate_solid_mask_instance_sequence(&mut self) -> u64 {
        let sequence = self.next_solid_mask_instance_sequence.get();
        self.next_solid_mask_instance_sequence.set(
            sequence
                .checked_add(1)
                .expect("C4SolidMask instance sequence overflow"),
        );
        sequence
    }

    /// Persist a host-time construction token across the callback copy-out
    /// boundary. C++ has already linked this instance even when Rust will
    /// materialize its spawn or object update later.
    fn record_solid_mask_instance_sequence(&mut self, id: ObjectId, sequence: u64) {
        if let Some(scope) = self.object_scope_mut(id) {
            scope.pending_update.solid_mask_instance_sequence = Some(sequence);
        }
        if let Some(spawn) = self
            .pending_spawns
            .iter_mut()
            .rev()
            .find(|spawn| spawn.id == Some(id))
        {
            spawn.solid_mask_instance_sequence = Some(sequence);
        }
    }

    /// Callback-time C4Object::UpdateSolidMask. The authoritative engine
    /// performs the real update when this outcome folds; this copy exists
    /// so nested callbacks observe both the immediate raster and exact live
    /// C4SolidMask instance age. `recreate` mirrors callers that delete the
    /// instance first (SetSolidMask, ChangeDef, real SetGraphics changes).
    pub(crate) fn update_live_solid_mask(&mut self, id: ObjectId, recreate: bool) {
        let previous = self.remove_live_solid_mask(id);
        let Some(spec) = self.live_solid_mask_spec(id) else {
            self.solid_mask_instance_sequences.borrow_mut().remove(&id);
            self.solid_mask_operations
                .push(crate::HostSolidMaskOperation::Remove { object_id: id });
            return;
        };
        let previous_sequence = self
            .solid_mask_instance_sequences
            .borrow()
            .get(&id)
            .copied()
            .or_else(|| previous.map(|(_, sequence)| sequence));
        let (instance_sequence, constructed) = if recreate {
            (self.allocate_solid_mask_instance_sequence(), true)
        } else if let Some(sequence) = previous_sequence {
            (sequence, false)
        } else {
            (self.allocate_solid_mask_instance_sequence(), true)
        };
        self.solid_mask_instance_sequences
            .borrow_mut()
            .insert(id, instance_sequence);
        if constructed {
            self.record_solid_mask_instance_sequence(id, instance_sequence);
        }
        let Some(position) = self
            .object_scope(id)
            .map(ObjectScopeContext::effective_position)
        else {
            self.solid_mask_operations
                .push(crate::HostSolidMaskOperation::Remove { object_id: id });
            return;
        };
        self.solid_mask_operations
            .push(crate::HostSolidMaskOperation::Put {
                object_id: id,
                spec: spec.clone(),
                position,
                instance_sequence,
            });
        let world_order = self.world.object_ids();
        let Some(landscape) = self.world.landscape_mut() else {
            return;
        };
        let Some(bake) = crate::put_solid_mask_raster(landscape, spec, position, instance_sequence)
        else {
            return;
        };
        let insert_at = previous.map(|(index, _)| index).unwrap_or_else(|| {
            let rank = world_order
                .iter()
                .position(|object_id| *object_id == id)
                .unwrap_or(usize::MAX);
            self.solid_mask_bakes
                .iter()
                .position(|(other_id, _)| {
                    world_order
                        .iter()
                        .position(|object_id| object_id == other_id)
                        .unwrap_or(usize::MAX)
                        > rank
                })
                .unwrap_or(self.solid_mask_bakes.len())
        });
        let solid_mask_bakes = Rc::make_mut(&mut self.solid_mask_bakes);
        solid_mask_bakes.insert(insert_at.min(solid_mask_bakes.len()), (id, bake));
    }

    /// The virtual C4SolidMask for one object created inside this still-
    /// running script call. C++ has already inserted and put such an object
    /// before Initialize; Rust materializes it only after the call folds, so
    /// collision host functions need this pending-only view in the interim.
    fn pending_solid_mask(&self, id: ObjectId) -> Option<crate::SolidMaskRect> {
        if !self.pending_objects.contains_key(&id) || !self.pending_order.contains(&id) {
            return None;
        }
        let scope = self.object_scope(id)?;
        if scope.destroy
            || matches!(scope.status(), ObjectStatus::Deleted)
            || scope.container().is_some()
            || scope.construction() < FULL_CON
            // The existing non-grid movement overlay likewise suppresses
            // rotation: a rectangle cannot faithfully represent C++'s
            // RotatedSolidmasks inverse-mapped square.
            || scope.rotation() != 0
        {
            return None;
        }
        let definition_id = self.object_effective_definition_id(id)?;
        let definition = self.world.solid_mask_metadata.get(&definition_id)?;
        let mask = self.live_solid_mask_rect(id)?;
        if !mask.is_positive() {
            return None;
        }
        let (graphics_definition, graphics_name) = scope
            .base_graphics
            .as_ref()
            .map(|graphics| {
                (
                    graphics.definition.as_str(),
                    graphics.graphics_name.as_deref(),
                )
            })
            .unwrap_or((definition_id.as_str(), None));
        let pixels = self
            .world
            .solid_mask_metadata
            .get(graphics_definition)?
            .pixels_for_checked_mask(mask, graphics_name)?;
        let shape = definition.shape.unwrap_or_default();
        let position = scope.effective_position();
        Some(crate::SolidMaskRect {
            object_id: id,
            x: position.x + shape.x + mask.target_x,
            y: position.y + shape.y + mask.target_y,
            width: mask.width,
            height: mask.height,
            pixels,
        })
    }

    pub(crate) fn pending_solid_masks(&self) -> Vec<crate::SolidMaskRect> {
        self.pending_order
            .iter()
            .filter_map(|id| self.pending_solid_mask(*id))
            .collect()
    }

    pub(crate) fn movement_density_at(
        &self,
        pending_masks: &[crate::SolidMaskRect],
        x: i32,
        y: i32,
    ) -> Option<i32> {
        let landscape = self.world.landscape_ref()?;
        let (width, height) = landscape
            .grid_dimensions()
            .unwrap_or_else(|| (landscape.width() as i32, landscape.estimated_height()));
        if x >= 0
            && y >= 0
            && x < width
            && y < height
            && (pending_masks.iter().any(|mask| mask.contains(x, y))
                || self.world.movement_solid_masks.iter().any(|mask| {
                    let remains_put = self.object_scope(mask.object_id).is_none_or(|scope| {
                        !scope.destroy
                            && scope.status() != ObjectStatus::Deleted
                            && scope.container().is_none()
                            && scope.construction() >= FULL_CON
                    });
                    remains_put && mask.contains(x, y)
                }))
        {
            return Some(crate::C4M_VEHICLE);
        }
        self.world.movement_density_at(x, y).or_else(|| {
            Some(if landscape.is_solid_at(x, y) {
                crate::C4M_SOLID
            } else if landscape.is_semi_solid_at(x, y) {
                crate::C4M_SEMI_SOLID
            } else {
                0
            })
        })
    }

    /// Fold the action mutations produced by Construction into a same-call
    /// pending spawn before NewObject's initial DoCon. Other pending fields
    /// remain live in the nested scope for subsequent arrow calls (for
    /// example WMPF::Place's SetActionTargets and SetCon).
    pub(crate) fn commit_creation_action(&mut self, target: ObjectId) {
        let Some(spawn_index) = self
            .pending_spawns
            .iter()
            .position(|spawn| spawn.id == Some(target))
        else {
            return;
        };
        let definition_id = self
            .object_effective_definition_id(target)
            .unwrap_or_else(|| {
                DefinitionId::from(self.pending_spawns[spawn_index].definition_id.as_str())
            });
        let Some(library) = self
            .definition_metadata(&definition_id)
            .map(|metadata| metadata.action_library.clone())
        else {
            return;
        };
        let Some(update) = self
            .object_scope_mut(target)
            .and_then(|scope| scope.pending_update.action.take())
        else {
            return;
        };
        let mut action = self.pending_spawns[spawn_index]
            .action
            .take()
            .unwrap_or_else(|| ActionState::new(library.default_action()));
        action.apply_update_with_library(&update, &library);
        self.pending_spawns[spawn_index].action = Some(action);
    }

    pub(crate) fn register_particle(&mut self, command: ParticleCommand) {
        self.pending_particles.push(command);
    }

    /// `Some(known?)` when the engine attached its particle def registry,
    /// `None` for legacy fixture contexts. See `HostWorldContext`.
    pub(crate) fn particle_def_known(&self, name: &str) -> Option<bool> {
        self.world.particle_def_known(name)
    }

    /// `C4ParticleSystem::IsFireParticleLoaded` (C4Particles.h:214).
    pub(crate) fn fire_particles_loaded(&self) -> bool {
        self.world.fire_particles_loaded()
    }

    pub(crate) fn register_transfer_zone_command(&mut self, command: TransferZoneCommand) {
        self.world.preview_transfer_zone_command(&command);
        self.transfer_zone_commands.push(command);
    }

    pub(crate) fn register_message(&mut self, command: MessageCommand) {
        self.pending_messages.push(command);
    }

    pub(crate) fn register_landscape_operation(&mut self, operation: LandscapeOperation) {
        self.solid_mask_operations
            .push(crate::HostSolidMaskOperation::Landscape {
                operation: operation.clone(),
            });
        self.pending_landscape_ops.push(operation);
    }

    /// DrawMatChunks must use this callback's LIVE mask vector: an earlier
    /// SetPosition/DoCon may already have removed or re-put a mask without
    /// changing the entry snapshot stored on HostWorldContext.
    pub(crate) fn preview_draw_mat_chunks(&mut self, operation: &LandscapeOperation) {
        let LandscapeOperation::DrawMatChunks {
            origin,
            width,
            height,
            count_x,
            count_y,
            material,
            byte,
            map_seed,
            random_offsets,
            texmap,
        } = operation
        else {
            return;
        };
        let Some(landscape) = self.world.landscape_mut() else {
            return;
        };
        let _ = landscape.preview_draw_material_chunks_with_masks(
            Rc::make_mut(&mut self.solid_mask_bakes).as_mut_slice(),
            *origin,
            *width,
            *height,
            *count_x,
            *count_y,
            material,
            *byte,
            *map_seed,
            random_offsets,
            texmap.clone(),
        );
        self.world.solid_mask_bakes = Rc::clone(&self.solid_mask_bakes);
    }

    pub(crate) fn preview_draw_material_quad(&mut self, operation: &LandscapeOperation) {
        let LandscapeOperation::DrawMaterialQuad {
            material_texture,
            vertices,
            ift,
        } = operation
        else {
            return;
        };
        let Some(landscape) = self.world.landscape_mut() else {
            return;
        };
        let _ = landscape.preview_draw_material_quad_with_masks(
            Rc::make_mut(&mut self.solid_mask_bakes).as_mut_slice(),
            material_texture,
            *vertices,
            *ift,
        );
    }

    pub(crate) fn preview_draw_indexed_map(&mut self, operation: &LandscapeOperation) {
        let (origin, bitmap, map_width, map_height, texmap, map_creator) = match operation {
            LandscapeOperation::DrawMap {
                origin,
                bitmap,
                map_width,
                map_height,
                texmap,
                map_creator,
            } => (
                *origin,
                bitmap,
                *map_width,
                *map_height,
                texmap,
                map_creator.as_ref(),
            ),
            LandscapeOperation::DrawDefMap {
                origin,
                bitmap,
                map_width,
                map_height,
                texmap,
                map_creator,
            } => (
                *origin,
                bitmap,
                *map_width,
                *map_height,
                texmap,
                Some(map_creator),
            ),
            _ => return,
        };
        let Some(landscape) = self.world.landscape_mut() else {
            return;
        };
        let _ = landscape.preview_draw_indexed_map_with_masks(
            Rc::make_mut(&mut self.solid_mask_bakes).as_mut_slice(),
            origin,
            bitmap,
            map_width,
            map_height,
            texmap.clone(),
        );
        if let Some(map_creator) = map_creator {
            let _ = landscape.replace_runtime_map_creator_state(map_creator.0.clone());
        }
        self.world.solid_mask_bakes = Rc::clone(&self.solid_mask_bakes);
    }

    pub(crate) fn preview_dig_circle(
        &mut self,
        center: Vector2,
        radius: i32,
    ) -> HashMap<crate::MaterialId, i32> {
        let materials = self.world.materials.clone().unwrap_or_default();
        self.world
            .landscape_mut()
            .map(|landscape| {
                preview_dig_circle_pixels(landscape, materials.as_ref(), center, radius)
            })
            .unwrap_or_default()
    }

    pub(crate) fn preview_dig_rect(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
    ) -> HashMap<crate::MaterialId, i32> {
        let materials = self.world.materials.clone().unwrap_or_default();
        self.world
            .landscape_mut()
            .map(|landscape| {
                preview_dig_rect_pixels(landscape, materials.as_ref(), origin, width, height)
            })
            .unwrap_or_default()
    }

    fn ensure_dig_material_contents(&mut self, target: ObjectId) -> bool {
        if self.dig_material_contents.contains_key(&target) {
            return true;
        }
        let Some(object) = self.get_world_object(target) else {
            return false;
        };
        self.dig_material_contents
            .insert(target, object.material_contents);
        true
    }

    fn stage_dig_material_contents(&mut self, target: ObjectId) {
        let Some(contents) = self.dig_material_contents.get(&target).cloned() else {
            return;
        };
        if self.ensure_object_scope(target) {
            if let Some(scope) = self.object_scope_mut(target) {
                scope.pending_update.material_contents = Some(contents.clone());
            }
        }
        if let Some(object) = self.pending_objects.get_mut(&target) {
            object.material_contents = contents;
        }
    }

    pub(crate) fn add_dig_material_counts(
        &mut self,
        target: ObjectId,
        counts: &HashMap<crate::MaterialId, i32>,
    ) -> bool {
        if !self.ensure_dig_material_contents(target) {
            return false;
        }
        let contents = self
            .dig_material_contents
            .get_mut(&target)
            .expect("dig contents seeded above");
        for (material, amount) in counts {
            if *amount <= 0 {
                continue;
            }
            if contents.len() <= material.index() {
                contents.resize(material.index() + 1, 0);
            }
            let slot = &mut contents[material.index()];
            *slot = slot.saturating_add(*amount);
        }
        self.stage_dig_material_contents(target);
        true
    }

    pub(crate) fn dig_material_content(
        &mut self,
        target: ObjectId,
        material: crate::MaterialId,
    ) -> i32 {
        if !self.ensure_dig_material_contents(target) {
            return 0;
        }
        self.dig_material_contents
            .get(&target)
            .and_then(|contents| contents.get(material.index()))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn reset_dig_material_content(
        &mut self,
        target: ObjectId,
        material: crate::MaterialId,
    ) {
        if !self.ensure_dig_material_contents(target) {
            return;
        }
        let contents = self
            .dig_material_contents
            .get_mut(&target)
            .expect("dig contents seeded above");
        if contents.len() <= material.index() {
            contents.resize(material.index() + 1, 0);
        }
        contents[material.index()] = 0;
        self.stage_dig_material_contents(target);
    }

    pub(crate) fn preview_blast_circle(
        &mut self,
        center: Vector2,
        radius: i32,
    ) -> Option<(BlastReplay, HashMap<crate::MaterialId, i32>)> {
        if radius < 0 || self.world.landscape_ref().is_none() {
            return None;
        }
        let random = RANDOM_CONTEXT.with(|cell| cell.borrow().as_ref().cloned())?;
        let materials = self.world.materials.clone().unwrap_or_default();
        let landscape = self.world.landscape_mut()?;
        let mut rng = random.rng.borrow_mut();
        let (pixels, counts) = if landscape.pixel_grid().is_some() {
            preview_raster_blast(landscape, materials.as_ref(), center, radius, &mut rng)
        } else {
            preview_column_blast(landscape, materials.as_ref(), center, radius, &mut rng)
        };
        Some((BlastReplay { pixels }, counts))
    }

    /// FnDrawVolcanoBranch mutates Surface8 before returning to script, so
    /// later GBack*/GetTexture calls in the same callback must see it. The
    /// helper intentionally leaves solid-mask bakes alone because C++ uses
    /// raw SetPix rather than PrepareChange/FinishChange here.
    pub(crate) fn preview_draw_volcano_branch(&mut self, operation: &LandscapeOperation) {
        let LandscapeOperation::DrawVolcanoBranch {
            from,
            to,
            size,
            material_byte,
        } = operation
        else {
            return;
        };
        let Some(landscape) = self.world.landscape_mut() else {
            return;
        };
        let _ = landscape.draw_volcano_branch(*from, *to, *size, *material_byte);
    }

    /// Run FnFreeRect against this callback's private COW landscape before
    /// returning to script. C++ mutates Surface8 synchronously, so later
    /// GBack*/GetMaterial calls in the same callback must see the clear. The
    /// queued operation folds the same mutation into the authoritative engine
    /// after the VM returns; its Rnd3 reads have already happened here.
    pub(crate) fn preview_clear_rect(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        density: Option<i32>,
    ) -> Result<(), RuntimeError> {
        let materials = self.world.materials.clone().unwrap_or_default();
        let landscape_height = self
            .world
            .landscape_ref()
            .map(Landscape::estimated_height)
            .unwrap_or(0);
        let bracket_masks = density.is_none()
            && self
                .world
                .landscape_ref()
                .is_some_and(|landscape| landscape.pixel_grid().is_some());
        if bracket_masks {
            let bounds = crate::landscape::RasterChangeRect::new(origin.x, origin.y, width, height);
            let landscape = self
                .world
                .landscape_mut()
                .expect("mask-bracketed FreeRect has a landscape");
            landscape.preview_raster_transaction_with_masks(
                Rc::make_mut(&mut self.solid_mask_bakes).as_mut_slice(),
                bounds,
                |landscape| -> Result<(), RuntimeError> {
                    for row in origin.y..origin.y.saturating_add(height) {
                        crate::Engine::mutate_clear_rect_landscape_row(
                            landscape,
                            materials.as_ref(),
                            origin.x,
                            row,
                            width,
                            None,
                            landscape_height,
                        );
                        if draw_context_rnd3()? != 0 {
                            draw_context_rnd3()?;
                        }
                    }
                    landscape.finish_clear_rect_change(bounds);
                    Ok(())
                },
            )?;
            self.world.solid_mask_bakes = Rc::clone(&self.solid_mask_bakes);
        } else {
            for row in origin.y..origin.y.saturating_add(height) {
                if let Some(landscape) = self.world.landscape_mut() {
                    crate::Engine::mutate_clear_rect_landscape_row(
                        landscape,
                        materials.as_ref(),
                        origin.x,
                        row,
                        width,
                        density,
                        landscape_height,
                    );
                }
                if draw_context_rnd3()? != 0 {
                    draw_context_rnd3()?;
                }
            }
        }
        self.register_landscape_operation(match density {
            Some(density) => LandscapeOperation::ClearRectDensity {
                origin,
                width,
                height,
                density,
            },
            None => LandscapeOperation::ClearRect {
                origin,
                width,
                height,
            },
        });
        Ok(())
    }

    /// Apply FnExtractLiquid's landscape half to this callback's private
    /// COW view. The matching operation is still folded into `Engine` after
    /// the VM returns, where CheckInstabilityRange/PXS side effects happen
    /// exactly once; this preview exists only so later host calls observe
    /// C++'s already-cleared Surface8 pixel.
    pub(crate) fn preview_extract_liquid(
        &mut self,
        position: Vector2,
    ) -> Option<crate::material::MaterialId> {
        if !self
            .world
            .landscape_ref()
            .is_some_and(|landscape| landscape.is_liquid_at(position.x, position.y))
        {
            return None;
        }
        let materials = self.world.materials.clone()?;
        let material = {
            let landscape = self.world.landscape_mut()?;
            landscape
                .extract_material_probe(position.x, position.y, materials.as_ref())
                .map(|(material, _, _)| material)?
        };
        self.register_landscape_operation(LandscapeOperation::ExtractLiquid { position });
        Some(material)
    }

    pub(crate) fn prepare_construction_terrain(
        &mut self,
        center_x: i32,
        bottom_y: i32,
        width: i32,
        height: i32,
        basement: i32,
    ) {
        if let (Some(materials), Some(landscape)) =
            (self.world.materials.clone(), self.world.landscape_mut())
        {
            preview_construction_terrain(
                landscape,
                materials.as_ref(),
                center_x,
                bottom_y,
                width,
                height,
                basement,
            );
        }
        self.register_landscape_operation(LandscapeOperation::PrepareConstructionTerrain {
            center_x,
            bottom_y,
            width,
            height,
            basement,
        });
    }

    pub(crate) fn resolve_runtime_material_texture(&mut self, material_texture: &str) -> bool {
        self.runtime_texmap_mut()
            .is_some_and(|texmap| texmap.get_index_mat_tex(material_texture, None) != 0)
    }

    pub(crate) fn preview_runtime_map_creator(
        &mut self,
        creator: crate::map_creator_s2::MapCreatorS2State,
    ) {
        let Some(landscape) = self.world.landscape_mut() else {
            return;
        };
        let Some(raster) = landscape.raster_state_mut() else {
            return;
        };
        raster.set_map_creator(Some(creator));
    }

    pub(crate) fn get_world_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get_world_object_preserving_contents_link(id, None)
    }

    /// Overlay one live object while optionally retaining one raw contents
    /// link whose child has already reached Status=0. AssignRemoval marks the
    /// child dead before `pCont->Contents.Remove(this)`, but registered C++
    /// iterators still observe that link at the later Remove call.
    fn get_world_object_preserving_contents_link(
        &self,
        id: ObjectId,
        preserved_child: Option<ObjectId>,
    ) -> Option<HostWorldObject> {
        let mut object = if let Some(object) = self.pending_objects.get(&id) {
            object.clone()
        } else {
            self.world.get(id)?
        };
        // C++ mutates live state mid-call: an object with a scope in THIS
        // call (active, dormant outer, or finished nested) reads through
        // that scope, so later host queries in the same call see earlier
        // writes exactly like the live C4Object. Removals surface as
        // deleted status (C4Object::AssignRemoval sets Status=0
        // immediately, C4Object.cpp:282) which FindObject & friends skip
        // (C4Game.cpp:1360-1365); containment reflects Enter/Exit
        // (FnFindObject vContainer, C4Script.cpp:2122-2127); the freshest
        // ACTION overlays too — GoldRush's WINC::CheckAmmo gates on
        // GetAction(pClonk) right after a nested SetAction("AimRifle") on
        // the suspended caller (Winchester.c4d/Script.c:292,
        // Cowboy.c4d/Script.c:442-443).
        if let Some(scope) = self.object_scope(id) {
            if let Some(definition_id) = scope.pending_update.change_def.as_ref() {
                object.definition_id = definition_id.clone();
            }
            object.unsorted = scope.unsorted;
            object.status = if scope.destroy {
                ObjectStatus::Deleted
            } else {
                scope.status
            };
            object.container = scope.current_container;
            object.position = scope.effective_position();
            object.fixed_position = scope.fixed_position();
            object.fixed_velocity = scope.fixed_velocity();
            object.fixed_rotation = scope.fixed_rotation();
            object.vertices = scope.vertices().to_vec();
            object.action_name = scope.current_action_name.clone();
            object.action_index = scope.current_action_index;
            object.action_procedure = scope.effective_procedure_name().map(str::to_string);
            object.action_phase = scope.current_action_phase;
            object.action_ticks = scope.current_action_ticks;
            object.action_target = scope.current_action_target;
            object.action_target2 = scope.current_action_target2;
            object.action_data = scope.current_action_data;
            object.damage = scope.current_damage;
            object.need_energy = scope.need_energy();
            let construction = scope.construction();
            let own_mass = scope.own_mass();
            object.construction = construction;
            if let Some(state) = object.state.as_mut() {
                let state = Rc::make_mut(state);
                if state.construction != construction || state.own_mass != own_mass {
                    state.construction = construction;
                    state.own_mass = own_mass;
                }
            }
            object.category = scope.category();
            if let Some(layer) = scope.pending_update.layer {
                if let Some(state) = object.state.as_mut() {
                    Rc::make_mut(state).layer = layer;
                }
            }
            object.selected = scope.selected();
            object.crew_disabled = scope
                .pending_update
                .crew_disabled
                .unwrap_or(object.crew_disabled);
            object.direction = scope.current_direction.to_script_value();
            object.owner = scope.owner();
            object.controller = Some(scope.controller());
            if let Some(base) = scope.pending_update.base {
                if let Some(state) = object.state.as_mut() {
                    Rc::make_mut(state).base = base;
                }
            }
            // Keep the whole-pixel mirror coherent for integer-velocity
            // consumers; `fixed_velocity` above retains exact sub-pixel dirs.
            if scope.pending_update.fixed_velocity.is_some()
                || scope.pending_update.fixed_velocity_x.is_some()
                || scope.pending_update.fixed_velocity_y.is_some()
                || scope.pending_update.velocity.is_some()
            {
                let fixed = scope.fixed_velocity();
                object.velocity = Vector2::new(fixed.int_x(), fixed.int_y());
            }
            // Energy stays the snapshot value on purpose: the paths that
            // depend on mid-call energy (DoEnergy, the death checks, the
            // active-scope GetEnergy) read the scope state directly, so
            // overlaying it here would change nothing they observe. What
            // it WOULD change is a foreign GetEnergy, whose stale read is
            // pinned by existing tests — and energy feeds AssignDeath, so
            // moving when a new value becomes visible is a sync-visible
            // change. Do not overlay it without a C++ differential first.
            object.ocf = scope.staged_ocf(scope.ocf());
        }
        // The snapshot contents list re-checks each child's live state:
        // C4Object::Exit removes the child from its container's Contents
        // IMMEDIATELY (C4Object.cpp:1529-1533), as does AssignRemoval for
        // removed children (C4Object.cpp:297-305) — a same-call eject loop
        // (`while(Contents()) Exit(Contents())`, the TotemHunt _PLO
        // DoPlrLaunch) must see the list shrink.
        if !object.contents.is_empty() {
            object.contents.retain(|child_id| {
                let touched = self
                    .contents_link_operations
                    .iter()
                    .any(|operation| operation.mutates_link(id, *child_id));
                preserved_child == Some(*child_id)
                    || touched
                    || (!self.unlinked_content_links.contains(&(id, *child_id))
                        && !self.relinked_content_links.contains(&(id, *child_id))
                        && self
                            .object_scope(*child_id)
                            // Contents is a raw C4ObjectList. AssignRemoval sets
                            // Status=0 before recursively removing contents, but
                            // does not unlink this object from its own container
                            // until that recursion returns. GetObject/Find/count
                            // callers apply their separate Status filters.
                            .map(|scope| scope.current_container == Some(id))
                            .unwrap_or(true))
            });
        }
        for operation in self
            .contents_link_operations
            .iter()
            .copied()
            .filter(|operation| operation.container() == id)
        {
            match operation {
                ContentsLinkOperation::Remove { child, .. } => {
                    object.contents.retain(|candidate| *candidate != child);
                }
                ContentsLinkOperation::Insert {
                    child, position, ..
                } => {
                    if !object.contents.contains(&child) {
                        object
                            .contents
                            .insert(position.min(object.contents.len()), child);
                    }
                }
                ContentsLinkOperation::MoveToBack { child, .. } => {
                    if let Some(position) = object
                        .contents
                        .iter()
                        .position(|candidate| *candidate == child)
                    {
                        let child = object.contents.remove(position);
                        object.contents.push(child);
                    }
                }
                ContentsLinkOperation::RotateToFront { child, .. } => {
                    if let Some(position) = object
                        .contents
                        .iter()
                        .position(|candidate| *candidate == child)
                    {
                        object.contents.rotate_left(position);
                    }
                }
            }
        }
        // ...and it GROWS for same-call Enters: C4Object::Enter adds to the
        // container's Contents immediately (`Contents.Add(this,
        // C4ObjectList::stContents)`, C4Object.cpp:1601-1605), sorting into
        // the matching category/id cluster (C4ObjectList::Add).
        let entered: Vec<ObjectId> = if self.contents_link_operations.is_empty() {
            Vec::new()
        } else {
            self.scopes_in_call_order()
                .filter(|scope| {
                    #[cfg(test)]
                    CONTENTS_SCOPE_GROWTH_VISITS.with(|count| count.set(count.get() + 1));
                    scope.current_container == Some(id)
                        && !self.unlinked_content_links.contains(&(id, scope.id))
                        && !object.contents.contains(&scope.id)
                        && !self
                            .contents_link_operations
                            .iter()
                            .any(|operation| operation.mutates_link(id, scope.id))
                })
                .map(|scope| scope.id)
                .collect()
        };
        for child in entered {
            let position = self.contents_insert_position(&object.contents, child, preserved_child);
            object.contents.insert(position, child);
        }
        let recorded_reorder = self
            .contents_link_operations
            .iter()
            .any(|operation| operation.reorders(id));
        if let Some(new_front) = (!recorded_reorder)
            .then(|| {
                self.object_scope(id)
                    .and_then(|scope| scope.pending_update.contents_front)
            })
            .flatten()
        {
            if let Some(index) = object.contents.iter().position(|child| *child == new_front) {
                object.contents.rotate_left(index);
            }
        }
        Some(object)
    }

    pub(crate) fn command_runtime_data(
        &self,
        physicals: &HashMap<ObjectId, PhysicalInfo>,
        deferred_physical_actor: Option<ObjectId>,
    ) -> (
        CommandObjectSnapshots,
        HashMap<i32, CommandPlayerSnapshot>,
        HashMap<DefinitionId, CommandDefinitionSnapshot>,
        TransferZoneTable,
    ) {
        // Command FindObject-style scans use forward `Game.Objects` order,
        // not the host snapshot's storage order. Keep every storage object
        // in the map for direct-target resolution, including inactive ones
        // omitted from the master list; only the scan tie rank comes from
        // the retained master list.
        let master_ids = self.master_object_ids();
        let master_list_indices = master_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect::<HashMap<_, _>>();
        let fallback_master_list_order = master_ids.len();
        let pending_masks = self.pending_solid_masks();
        let ids = self.all_world_object_ids();
        let objects = ids
            .into_iter()
            .enumerate()
            .filter_map(|(storage_order, id)| {
                let object = self.get_world_object(id)?;
                let master_list_order = master_list_indices
                    .get(&id)
                    .copied()
                    .unwrap_or_else(|| fallback_master_list_order.saturating_add(storage_order));
                let scope = self.object_scope(id);
                let metadata = self.world.definition_metadata(object.definition_id());
                let position = scope
                    .map(|scope| scope.current_position)
                    .unwrap_or(object.position);
                let construction = scope
                    .map(ObjectScopeContext::construction)
                    .unwrap_or(object.construction);
                let vertices = scope
                    .map(ObjectScopeContext::vertices)
                    .unwrap_or(object.vertices.as_slice());
                let local_shape = live_object_bounds_shape(self, id)
                    .unwrap_or_else(|| DefinitionRect::new(-1, -1, 2, 2));
                let shape_height = live_object_shape(self, id)
                    .map(|shape| shape.height)
                    .unwrap_or(local_shape.height);
                let contact_density = scope
                    .map(ObjectScopeContext::contact_density)
                    .unwrap_or_else(|| object.contact_density());
                let add_top = (18 - local_shape.height).max(0);
                let shape = DefinitionRect::new(
                    position.x.saturating_add(local_shape.x),
                    position
                        .y
                        .saturating_add(local_shape.y)
                        .saturating_sub(add_top),
                    local_shape.width,
                    local_shape.height.saturating_add(add_top),
                );
                let contact = vertices.iter().fold(0, |bits, vertex| {
                    bits | compute_vertex_contact(position, vertex, 0, contact_density, |x, y| {
                        self.movement_density_at(&pending_masks, x, y)
                    })
                });
                let owner = scope.map(ObjectScopeContext::owner).unwrap_or(object.owner);
                let selected = object.selected;
                let command_ocf = scope
                    .map(|scope| scope.staged_ocf(object.ocf))
                    .unwrap_or(object.ocf);
                let entrance = (command_ocf & ocf::ENTRANCE != 0)
                    .then(|| {
                        metadata
                            .and_then(|metadata| metadata.fire.entrance_rect)
                            .map(|rect| {
                                DefinitionRect::new(
                                    position.x.saturating_add(rect.x),
                                    position.y.saturating_add(rect.y),
                                    rect.width,
                                    rect.height,
                                )
                            })
                    })
                    .flatten();
                let action_name = scope
                    .map(|scope| scope.effective_action_name().to_string())
                    .unwrap_or_else(|| object.action_name.clone());
                let action_index = scope
                    .map(ObjectScopeContext::effective_action_index)
                    .unwrap_or(object.action_index);
                let action_idle = scope
                    .map(|scope| {
                        scope
                            .action_library
                            .is_idle_entry(&action_name, action_index)
                    })
                    .or_else(|| {
                        metadata.map(|metadata| {
                            metadata
                                .action_library
                                .is_idle_entry(&action_name, action_index)
                        })
                    })
                    .unwrap_or(true);
                let action_disabled = scope
                    .map(|scope| {
                        scope
                            .action_library
                            .disables_object_for_entry(&action_name, action_index)
                    })
                    .or_else(|| {
                        metadata.map(|metadata| {
                            metadata
                                .action_library
                                .disables_object_for_entry(&action_name, action_index)
                        })
                    })
                    .unwrap_or(false);
                let snapshot = CommandObjectSnapshot {
                    id,
                    master_list_order,
                    definition_id: object.definition_id.clone(),
                    position,
                    fixed_position: scope
                        .map(ObjectScopeContext::fixed_position)
                        .unwrap_or(object.fixed_position),
                    fixed_velocity: scope
                        .map(ObjectScopeContext::fixed_velocity)
                        .unwrap_or(object.fixed_velocity),
                    move_to_range: object.move_to_range,
                    pathfinder: object.pathfinder,
                    no_transfer_zones: object.no_transfer_zones,
                    no_push_enter: metadata
                        .map(|metadata| metadata.fire.no_push_enter)
                        .unwrap_or(object.no_push_enter),
                    status: scope
                        .map(ObjectScopeContext::status)
                        .unwrap_or(object.status),
                    destroyed: scope.is_some_and(|scope| scope.destroy),
                    category: scope
                        .map(ObjectScopeContext::category)
                        .unwrap_or(object.category),
                    container: scope
                        .map(ObjectScopeContext::container)
                        .unwrap_or(object.container),
                    action_name,
                    action_idle,
                    action_disabled,
                    action_target: scope
                        .map(|scope| scope.effective_action_target(0))
                        .unwrap_or(object.action_target),
                    action_target2: scope
                        .map(|scope| scope.effective_action_target(1))
                        .unwrap_or(object.action_target2),
                    action_procedure: scope
                        .map(ObjectScopeContext::effective_action_procedure)
                        .unwrap_or_else(|| {
                            object
                                .procedure_name()
                                .map(ActionProcedure::from_name)
                                .unwrap_or_default()
                        }),
                    command_direction: scope
                        .map(ObjectScopeContext::command_direction)
                        .or_else(|| object.full_state().map(|state| state.command_direction))
                        .unwrap_or_default(),
                    construction,
                    direction: scope
                        .map(|scope| scope.current_direction)
                        .unwrap_or_else(|| Direction::from_script_value(object.direction)),
                    physical: physicals
                        .get(&id)
                        .copied()
                        .or_else(|| {
                            object
                                .full_state()
                                .and_then(|state| state.temporary_physical)
                        })
                        .or_else(|| object.full_state().and_then(|state| state.info_physical))
                        .or_else(|| metadata.map(|metadata| metadata.physical))
                        .unwrap_or_default(),
                    physical_deferred: deferred_physical_actor == Some(id),
                    owner,
                    controller: scope
                        .map(ObjectScopeContext::controller)
                        .unwrap_or_else(|| object.controller()),
                    base: scope
                        .and_then(|scope| scope.pending_update.base)
                        .or_else(|| object.full_state().map(|state| state.base))
                        .unwrap_or(OWNER_NONE),
                    crew_member: scope
                        .map(|scope| scope.crew_member)
                        .or_else(|| object.full_state().map(|state| state.crew_member))
                        .unwrap_or(false),
                    selected,
                    alive: scope.map(ObjectScopeContext::alive).unwrap_or(object.alive),
                    need_energy: scope
                        .map(ObjectScopeContext::need_energy)
                        .unwrap_or(object.need_energy),
                    on_fire: scope
                        .and_then(|scope| scope.pending_update.staged_on_fire())
                        .or_else(|| object.full_state().map(|state| state.on_fire))
                        .unwrap_or(false),
                    contents: object.contents.clone(),
                    commands: scope
                        .map(|scope| scope.live_commands.command_views())
                        .unwrap_or_else(|| object.commands.clone()),
                    line_connect: metadata.map(|metadata| metadata.line_connect).unwrap_or(0),
                    ocf: command_ocf,
                    entrance_status: scope
                        .and_then(|scope| scope.pending_update.entrance_status)
                        .or_else(|| object.full_state().map(|state| state.entrance_status))
                        .unwrap_or(false),
                    collectible: object.collectible,
                    contact,
                    action_time: scope
                        .map(ObjectScopeContext::effective_action_ticks)
                        .unwrap_or(object.action_ticks),
                    shape_top: local_shape.y,
                    shape_height,
                    shape,
                    entrance,
                };
                Some((id, snapshot))
            })
            .collect();

        let players = self
            .world
            .player_states()
            .map(|(id, state)| {
                let state = self.player_overrides.get(&id).unwrap_or(state);
                (
                    id,
                    CommandPlayerSnapshot {
                        status: state.status,
                        surrendered: state.surrendered,
                        wealth: state.wealth,
                        home_base_material: state.home_base_material.clone(),
                        home_base_material_entries: state.exact_home_base_material_entries(),
                        knowledge: state
                            .exact_knowledge_entries()
                            .into_iter()
                            .map(|(id, _)| id)
                            .collect(),
                        hostile_to: state
                            .exact_hostility_entries()
                            .into_iter()
                            .filter_map(|(opponent, hostile)| {
                                (hostile != 0).then_some(opponent.wrapping_sub(1))
                            })
                            .collect(),
                    },
                )
            })
            .collect();
        let definitions = self
            .world
            .definitions
            .iter()
            .map(|(id, metadata)| {
                (
                    id.clone(),
                    CommandDefinitionSnapshot {
                        value: metadata.value,
                        shape: metadata.shape,
                        category: metadata.category,
                        construction_offset: metadata.construction_offset,
                        collection_limit: metadata.collection_limit,
                        collection_rect: metadata.fire.collection_rect,
                        fragile: metadata.fire.fragile,
                        projectile: metadata.fire.projectile,
                        can_chop: metadata.action_library.specs().iter().any(|(_, spec)| {
                            spec.procedure.as_deref().is_some_and(|name| {
                                ActionProcedure::from_name(name) == ActionProcedure::Chop
                            })
                        }),
                        chop_action: metadata.action_library.specs().iter().find_map(
                            |(name, spec)| {
                                spec.procedure
                                    .as_deref()
                                    .filter(|procedure| {
                                        ActionProcedure::from_name(procedure)
                                            == ActionProcedure::Chop
                                    })
                                    .map(|_| name.clone())
                            },
                        ),
                        constructable: metadata.constructable,
                        grab: metadata.fire.grab,
                        grab_put_get: metadata.grab_put_get,
                        no_get: metadata.fire.no_get,
                    },
                )
            })
            .collect();
        let transfers = TransferZoneTable::from_states(&self.world.transfer_zones);
        (objects, players, definitions, transfers)
    }

    pub(crate) fn execute_command_preview(
        &mut self,
        target: ObjectId,
        rng: Option<&RefCell<LcgRng>>,
        command_data: &PreparedCommandRuntimeData,
    ) -> Option<CommandPreviewOutcome> {
        let (objects, players, definitions, transfers) = command_data;
        let object_snapshot = objects.get(&target)?;
        let landscape = self.world.landscape_shared();
        let context = CommandRuntimeContext {
            rng,
            frame: self.world.frame,
            position: object_snapshot.position,
            landscape: landscape.as_deref(),
            object: object_snapshot,
            objects,
            players,
            definitions,
            structures_need_energy: self.world.structures_need_energy,
            base_buy_enabled: self.world.base_buy_enabled,
            base_sell_enabled: self.world.base_sell_enabled,
            transfer_zones: transfers,
        };
        let gravity = PHYSICS_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|context| fixed100(context.gravity()) / 5)
                .unwrap_or_else(|| PhysicsSettings::default().gravity_as_c4fixed())
        });

        let (events, finished, update) = {
            let scope = self.object_scope_mut(target)?;
            let result = scope
                .live_commands
                .execute_front_with_gravity(&context, gravity);
            if result.is_some() {
                scope.command_stack_replaced = true;
            }
            let mut events = Vec::new();
            let mut update = None;
            if let Some(mut result) = result {
                update = result.update.take();
                events = result.events;
            }
            scope.command_count = scope.live_commands.len();
            (events, scope.live_commands.finished_front_view(), update)
        };
        if let Some(update) = update {
            self.stage_object_command_update(target, update);
        }

        self.collect_command_preview_events(target, finished, events)
    }

    /// Resume a command which suspended exactly at GetPhysical. The caller
    /// resolves the callbackful reads outside HOST_CONTEXT and supplies the
    /// final captured value plus a freshly rebuilt snapshot table.
    pub(crate) fn execute_pending_command_physical_preview(
        &mut self,
        target: ObjectId,
        command_instance_id: u64,
        physical: PhysicalInfo,
        rng: Option<&RefCell<LcgRng>>,
        command_data: &PreparedCommandRuntimeData,
    ) -> Option<(Vec<CommandEvent>, Option<CommandView>)> {
        let (objects, players, definitions, transfers) = command_data;
        let object_snapshot = objects.get(&target)?;
        let landscape = self.world.landscape_shared();
        let context = CommandRuntimeContext {
            rng,
            frame: self.world.frame,
            position: object_snapshot.position,
            landscape: landscape.as_deref(),
            object: object_snapshot,
            objects,
            players,
            definitions,
            structures_need_energy: self.world.structures_need_energy,
            base_buy_enabled: self.world.base_buy_enabled,
            base_sell_enabled: self.world.base_sell_enabled,
            transfer_zones: transfers,
        };
        let gravity = PHYSICS_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|context| fixed100(context.gravity()) / 5)
                .unwrap_or_else(|| PhysicsSettings::default().gravity_as_c4fixed())
        });
        let (events, finished, update) = {
            let scope = self.object_scope_mut(target)?;
            let result = scope.live_commands.execute_pending_physical(
                &context,
                gravity,
                command_instance_id,
                physical,
            );
            if result.is_some() {
                scope.command_stack_replaced = true;
            }
            let mut events = Vec::new();
            let mut update = None;
            if let Some(mut result) = result {
                update = result.update.take();
                events = result.events;
            }
            scope.command_count = scope.live_commands.len();
            (events, scope.live_commands.finished_front_view(), update)
        };
        if let Some(update) = update {
            self.stage_object_command_update(target, update);
        }
        Some((events, finished))
    }

    pub(crate) fn collect_command_preview_events(
        &mut self,
        target: ObjectId,
        finished: Option<CommandView>,
        events: Vec<CommandEvent>,
    ) -> Option<CommandPreviewOutcome> {
        let mut outcome = CommandPreviewOutcome {
            finished,
            ..CommandPreviewOutcome::default()
        };
        let mut deferred_events = Vec::new();
        for event in events {
            match event {
                CommandEvent::EvaluateBuy {
                    actor_id,
                    base_id,
                    definition_id,
                    buyer,
                    payer,
                    count,
                } => outcome.buy_attempts.push((
                    actor_id,
                    base_id,
                    definition_id,
                    buyer,
                    payer,
                    count,
                )),
                CommandEvent::EvaluateSell {
                    actor_id,
                    base_id,
                    definition_id,
                    preferred,
                    count,
                } => {
                    outcome
                        .sell_attempts
                        .push((actor_id, base_id, definition_id, preferred, count))
                }
                CommandEvent::SetPathFinderSettings {
                    level,
                    transfer_zones_enabled,
                } => {
                    // FnExecuteCommand is synchronous. A later GetPath in
                    // this same VM call must see the just-written global
                    // settings before the copied context folds back into
                    // Engine; retain the event for that eventual fold too.
                    self.world
                        .set_pathfinder_settings(level, transfer_zones_enabled);
                    deferred_events.push(CommandEvent::SetPathFinderSettings {
                        level,
                        transfer_zones_enabled,
                    });
                }
                CommandEvent::SetPathFinderDebug { snapshot } => {
                    // FnExecuteCommand mutates the global pathfinder before a
                    // later GetPath in the same script call. The shared sink
                    // is already Engine's process-presentation state, so do
                    // not defer and later overwrite the newer GetPath graph.
                    *self.world.pathfinder_debug.borrow_mut() = snapshot;
                }
                CommandEvent::AttemptGrab {
                    actor_id,
                    target_id,
                } => outcome.grab_attempts.push((actor_id, target_id)),
                CommandEvent::ObjectComPut {
                    actor_id,
                    target_id,
                    object_id,
                    ungrab_on_success,
                    command_instance_id,
                } => outcome.put_attempts.push((
                    actor_id,
                    target_id,
                    object_id,
                    ungrab_on_success,
                    command_instance_id,
                )),
                CommandEvent::ObjectComDrop {
                    actor_id,
                    object_id,
                    command_instance_id,
                } => outcome
                    .drop_attempts
                    .push((actor_id, object_id, command_instance_id)),
                CommandEvent::ObjectComUnGrabCommand {
                    actor_id,
                    command_instance_id,
                } => outcome
                    .ungrab_attempts
                    .push((actor_id, command_instance_id)),
                CommandEvent::ObjectComPutTake {
                    actor_id,
                    target_id,
                    requested_item,
                    command,
                    command_instance_id,
                } => outcome.put_take_attempts.push((
                    actor_id,
                    target_id,
                    requested_item,
                    command,
                    command_instance_id,
                )),
                CommandEvent::ThrowObject {
                    actor_id,
                    object_id,
                    complete_command_on_success,
                    command_instance_id,
                } => outcome.throw_attempts.push((
                    actor_id,
                    object_id,
                    complete_command_on_success,
                    command_instance_id,
                )),
                event @ (CommandEvent::ObjectComStopThrow { .. }
                | CommandEvent::ObjectComSetDirThrow { .. }
                | CommandEvent::ObjectComStopDrop { .. }
                | CommandEvent::MoveToFlightControlTakeoff { .. }
                | CommandEvent::ObjectComStopPut { .. }
                | CommandEvent::ObjectComStopChop { .. }
                | CommandEvent::ObjectComStopConstruct { .. }
                | CommandEvent::ControlCommandConstruction { .. }
                | CommandEvent::SpawnConstruction { .. }
                | CommandEvent::ObjectComStopExit { .. }) => {
                    outcome.throw_preludes.push(event);
                }
                CommandEvent::ActivateEntrance {
                    object_id,
                    caller,
                    on_result,
                    command_instance_id,
                } => outcome.entrance_attempts.push((
                    object_id,
                    caller,
                    on_result,
                    command_instance_id,
                )),
                CommandEvent::ControlTransfer {
                    object_id,
                    caller,
                    tx_value,
                    ty,
                    command_instance_id,
                } => outcome.control_transfers.push((
                    object_id,
                    caller,
                    tx_value,
                    ty,
                    command_instance_id,
                )),
                event @ CommandEvent::CallObjectFunction { .. } => {
                    outcome.call_attempts.push(event)
                }
                event @ (CommandEvent::CommandExitObject { .. }
                | CommandEvent::CommandExitIntoParent { .. }) => outcome.exit_attempts.push(event),
                CommandEvent::NativeCommandSuccess { object_id, command } => {
                    apply_preview_native_command_success(self, object_id, command);
                }
                CommandEvent::FailureFeedback { actor_id, feedback } => {
                    outcome.failure_feedback.push((actor_id, feedback));
                }
                CommandEvent::ObjectComStopMoveTo { object_id } => {
                    outcome.move_to_stops.push(object_id);
                }
                CommandEvent::ObjectComStopBuild {
                    object_id,
                    command_instance_id,
                } => outcome.build_stops.push((object_id, command_instance_id)),
                CommandEvent::ObjectComBuild {
                    object_id,
                    target_id,
                    stop_first,
                } => outcome
                    .build_actions
                    .push((object_id, target_id, stop_first)),
                CommandEvent::ObjectComDig {
                    actor_id,
                    dig_out_material,
                    direction,
                    command_instance_id,
                } => outcome.dig_attempts.push((
                    actor_id,
                    dig_out_material,
                    direction,
                    command_instance_id,
                )),
                CommandEvent::ResolveCommandPhysical {
                    object_id,
                    reads,
                    command_instance_id,
                } => outcome
                    .physical_reads
                    .push((object_id, reads, command_instance_id)),
                CommandEvent::OpenMenu(request) => self.pending_menu_requests.push(request),
                other => deferred_events.push(other),
            }
        }
        if !deferred_events.is_empty() {
            self.object_scope_mut(target)?.queued_commands.push(
                QueuedCommand::immediate(ObjectUpdate::default())
                    .with_events(deferred_events.clone()),
            );
            self.pending_command_events.extend(deferred_events);
        }
        Some(outcome)
    }

    pub(crate) fn clear_finished_command_fronts(&mut self, target: ObjectId) {
        if let Some(scope) = self.object_scope_mut(target) {
            scope.live_commands.clear_finished_fronts();
            scope.command_count = scope.live_commands.len();
            scope.command_stack_replaced = true;
        }
    }

    /// Every scope this call holds pending writes for, in a deterministic
    /// order: the active scope, suspended outer calls, completed nested
    /// calls (first-call order).
    fn scopes_in_call_order(&self) -> impl Iterator<Item = &ObjectScopeContext> {
        self.object
            .iter()
            .chain(self.dormant_scopes.iter().flatten())
            .chain(
                self.nested_order
                    .iter()
                    .filter_map(|id| self.nested_objects.get(id).map(|state| &state.scope)),
            )
    }

    /// A contents entry's sort inputs — the live scope first, then the
    /// same-call previews and the world snapshot.
    fn contents_sort_key(&self, id: ObjectId) -> Option<(i32, String)> {
        let snapshot = self
            .pending_objects
            .get(&id)
            .map(|object| (object.category, object.definition_id().to_string()))
            .or_else(|| {
                self.world
                    .get(id)
                    .map(|object| (object.category, object.definition_id().to_string()))
            });
        match self.object_scope(id) {
            Some(scope) => {
                let definition_id = scope
                    .pending_update
                    .change_def
                    .clone()
                    .or_else(|| scope.definition_id.clone())
                    .or_else(|| snapshot.as_ref().map(|(_, def)| def.clone()))?;
                Some((scope.current_category, definition_id))
            }
            None => snapshot,
        }
    }

    fn contents_object_unsorted(&self, id: ObjectId) -> bool {
        self.object_scope(id)
            .map(|scope| scope.unsorted)
            .or_else(|| self.pending_objects.get(&id).map(|object| object.unsorted))
            .or_else(|| self.world.get(id).map(|object| object.unsorted))
            .unwrap_or(false)
    }

    fn contents_object_is_present(&self, id: ObjectId) -> bool {
        self.object_scope(id)
            .map(|scope| !scope.destroy && scope.status != ObjectStatus::Deleted)
            .or_else(|| {
                self.pending_objects
                    .get(&id)
                    .map(HostWorldObject::is_present)
            })
            .or_else(|| self.world.get(id).map(|object| object.is_present()))
            .unwrap_or(false)
    }

    pub(crate) fn contents_link_generation(&self, id: ObjectId) -> u64 {
        self.object_scope(id)
            .map(|scope| scope.current_contents_link_generation)
            .or_else(|| {
                self.get_world_object(id).and_then(|object| {
                    object
                        .full_state()
                        .map(|state| state.contents_link_generation)
                })
            })
            .unwrap_or(0)
    }

    pub(crate) fn track_contents_link_removal(&self, container: ObjectId, child: ObjectId) {
        let Some(contents) = self
            .get_world_object_preserving_contents_link(container, Some(child))
            .map(|object| object.contents().to_vec())
        else {
            return;
        };
        let Some(position) = contents.iter().position(|candidate| *candidate == child) else {
            return;
        };
        let successor = contents
            .get(position + 1)
            .copied()
            .map(|successor| (successor, self.contents_link_generation(successor)));
        crate::direct_com::track_internal_object_menu_link_removal(
            container,
            child,
            self.contents_link_generation(child),
            successor,
        );
    }

    pub(crate) fn set_object_container_tracked(
        &mut self,
        child: ObjectId,
        container: Option<ObjectId>,
    ) -> bool {
        let previous = match self.object_scope(child) {
            Some(scope) => scope.container(),
            None => self
                .get_world_object(child)
                .and_then(|object| object.container()),
        };
        if previous != container {
            if let Some(previous) = previous {
                self.track_contents_link_removal(previous, child);
                self.record_contents_link_removal(previous, child);
            }
        }
        let changed = previous != container;
        let Some(scope) = self.object_scope_mut(child) else {
            return false;
        };
        scope.set_container(container);
        if changed {
            if let Some(container) = container {
                self.link_content_after_enter(container, child);
            }
        }
        true
    }

    pub(crate) fn stage_object_command_update(&mut self, object: ObjectId, update: ObjectUpdate) {
        let container_change = update.container;
        let mut container_changed = false;
        if let Some(container) = container_change {
            let previous = match self.object_scope(object) {
                Some(scope) => scope.container(),
                None => self
                    .get_world_object(object)
                    .and_then(|state| state.container()),
            };
            if previous != container {
                container_changed = true;
                if let Some(previous) = previous {
                    self.track_contents_link_removal(previous, object);
                    self.record_contents_link_removal(previous, object);
                }
            }
        }
        if let Some(scope) = self.object_scope_mut(object) {
            scope.stage_command_update(update);
        }
        if let Some(container) = container_change {
            if container_changed {
                if let Some(container) = container {
                    self.link_content_after_enter(container, object);
                }
            }
        }
    }

    /// C4ObjectList::Add stContents sort-in (C4ObjectList.cpp:104-152):
    /// cluster with the first same-(SortLimit-category, id) entry, else
    /// before the first entry of lower-or-equal sort category; lines (and
    /// StaticBack children, which skip the cluster pass) fall through to
    /// the relative-category walk, lines append at the end. The
    /// engine-side twin is `Engine::contents_insert_position`.
    fn contents_insert_position(
        &self,
        contents: &[ObjectId],
        child: ObjectId,
        preserved_child: Option<ObjectId>,
    ) -> usize {
        let Some((category, definition_id)) = self.contents_sort_key(child) else {
            return contents.len();
        };
        let is_line = self
            .world
            .definition_metadata(&definition_id)
            .map(|metadata| metadata.line != 0)
            .unwrap_or(false);
        if is_line || self.contents_object_unsorted(child) {
            return contents.len();
        }
        let sort_category = category & crate::CATEGORY_SORT_LIMIT;
        let mut predecessor = None;
        let mut found_cluster = false;
        if category & crate::CATEGORY_STATIC_BACK == 0 {
            for (position, &other) in contents.iter().enumerate() {
                if (preserved_child != Some(other) && !self.contents_object_is_present(other))
                    || self.contents_object_unsorted(other)
                {
                    continue;
                }
                let Some((other_category, other_definition)) = self.contents_sort_key(other) else {
                    continue;
                };
                if other_category & crate::CATEGORY_SORT_LIMIT == sort_category
                    && other_definition == definition_id
                {
                    found_cluster = true;
                    break;
                }
                predecessor = Some(position);
            }
        }
        if !found_cluster {
            predecessor = None;
            for (position, &other) in contents.iter().enumerate() {
                if (preserved_child != Some(other) && !self.contents_object_is_present(other))
                    || self.contents_object_unsorted(other)
                {
                    continue;
                }
                let Some((other_category, _)) = self.contents_sort_key(other) else {
                    continue;
                };
                if other_category & crate::CATEGORY_SORT_LIMIT <= sort_category {
                    break;
                }
                predecessor = Some(position);
            }
        }
        predecessor.map_or(0, |position| position + 1)
    }

    /// Drops a spawn queued in THIS call before it materializes. The id
    /// stays allocated (C++ objects removed in the creating call still
    /// consumed their Number).
    pub(crate) fn cancel_pending_spawn(&mut self, target: ObjectId) -> bool {
        let before = self.pending_spawns.len();
        self.pending_spawns.retain(|spawn| spawn.id != Some(target));
        let removed = self.pending_spawns.len() != before;
        if removed {
            self.pending_order.retain(|id| *id != target);
            self.pending_objects.remove(&target);
        }
        removed
    }

    /// C4Game::ClearObjectPtrs -> C4Object::ClearPointers for the pointer
    /// kinds modeled by ObjectScopeContext. Unlike object removal, status
    /// deactivation must leave ordinary script values that reference the
    /// inactive object intact. pLayer is an engine pointer, so it is cleared
    /// here even though ordinary script values are handled separately.
    pub(crate) fn clear_object_action_and_command_pointers(&mut self, target: ObjectId) {
        let target_number = i32::try_from(target.as_u64()).ok();
        let mut object_ids = self.world.object_pointer_referrer_ids(target);
        for id in self.pending_order.iter().copied() {
            if !object_ids.contains(&id) {
                object_ids.push(id);
            }
        }
        for id in object_ids {
            let clears_layer = self.object_layer(id) == Some(target);
            let references_target = self
                .object_scope(id)
                .is_some_and(|scope| scope.references_object_pointer(target))
                || self.get_world_object(id).is_some_and(|object| {
                    object.action_target(0) == Some(target)
                        || object.action_target(1) == Some(target)
                        || object.commands.iter().any(|command| {
                            command.target == Some(target) || command.target2 == Some(target)
                        })
                        || target_number.is_some_and(|target| {
                            object.full_state().is_some_and(|state| {
                                state
                                    .effects
                                    .iter()
                                    .any(|effect| effect.command_target == Some(target))
                            })
                        })
                });
            if (references_target || clears_layer) && self.ensure_object_scope(id) {
                if let Some(scope) = self.object_scope_mut(id) {
                    if references_target {
                        scope.clear_object_pointer(target);
                    }
                    if clears_layer {
                        scope.pending_update.layer = Some(None);
                        scope.reset_layer_compiler_cache();
                    }
                }
            }
        }
        // Same-call creations may not have a materializable ObjectScope yet.
        // Keep both their eventual SpawnConfig and callback-visible preview
        // in sync with the pointer sweep.
        for spawn in &mut self.pending_spawns {
            if spawn.layer == Some(target) {
                spawn.layer = None;
                spawn.compiler_cache.layer = 0;
            }
        }
        for object in self.pending_objects.values_mut() {
            if let Some(state) = object.state.as_mut() {
                let state = Rc::make_mut(state);
                if state.layer == Some(target) {
                    state.layer = None;
                    object.compiler_cache.layer = 0;
                }
            }
        }
        if let (Some(global), Some(target)) = (self.global.as_mut(), target_number) {
            global.clear_command_target(target);
        }
    }

    pub(crate) fn clear_non_player_script_object_references(
        &mut self,
        target: ObjectId,
        last_position: Option<Vector2>,
    ) {
        if let Some(position) = last_position
            .or_else(|| {
                self.object_scope(target)
                    .map(|scope| scope.current_position)
            })
            .or_else(|| self.get_world_object(target).map(|object| object.position))
        {
            self.audio.detach_object_sounds(target, position);
        }
        self.removed_object_references.insert(target);
        let mut reference_sweep = clonk_script::ObjectReferenceSweep::active(target.as_u64());
        // Bring untouched persistent holders into this callback's ordinary
        // nested-outcome pipeline before clearing their locals/effect vars.
        self.clear_object_action_and_command_pointers(target);
        for id in self.world.object_script_value_referrer_ids(target) {
            self.ensure_object_scope(id);
        }
        if let Some(scope) = self.object.as_mut() {
            scope.effects.clear_object_references(&mut reference_sweep);
        }
        if let Some(scope) = self.global.as_mut() {
            scope.clear_object_references(&mut reference_sweep);
        }
        for scope in self.dormant_scopes.iter_mut().flatten() {
            scope.effects.clear_object_references(&mut reference_sweep);
        }
        for state in self.nested_objects.values_mut() {
            state
                .scope
                .effects
                .clear_object_references(&mut reference_sweep);
        }
        for state in self.nested_objects.values_mut() {
            for value in state.local_vars.values_mut() {
                reference_sweep.clear_value(value);
            }
        }
        // Game.ClearPointers removes every transfer zone owned by the
        // object before AssignRemoval returns. Mutate this callback's world
        // immediately for later same-VM-call GetPath/ExecuteCommand reads,
        // and retain the command so the live Engine observes the same clear
        // when the copied host outcome folds back (C4Game.cpp:1020-1031;
        // C4TransferZone.cpp:68-76).
        let command = TransferZoneCommand::clear(target);
        self.register_transfer_zone_command(command);
    }

    pub(crate) fn unlink_content_for_removal(&mut self, parent: ObjectId, child: ObjectId) {
        self.track_contents_link_removal(parent, child);
        self.record_contents_link_removal(parent, child);
        self.unlinked_content_links.insert((parent, child));
    }

    pub(crate) fn relink_content_after_exit(&mut self, parent: ObjectId, child: ObjectId) {
        self.record_contents_link_removal(parent, child);
        self.relinked_content_links.insert((parent, child));
    }

    fn record_contents_link_removal(&mut self, container: ObjectId, child: ObjectId) {
        let linked = self
            .get_world_object_preserving_contents_link(container, Some(child))
            .is_some_and(|object| object.contents().contains(&child));
        if linked {
            self.contents_link_operations
                .push(ContentsLinkOperation::Remove { container, child });
        }
    }

    pub(crate) fn link_content_after_enter(&mut self, container: ObjectId, child: ObjectId) {
        let Some(mut contents) = self
            .get_world_object(container)
            .map(|object| object.contents().to_vec())
        else {
            return;
        };
        // `scope.set_container` has already exposed the new Contained word,
        // so the legacy scope-growth fallback may have projected `child`
        // before this explicit C4ObjectList::Add event is recorded. Native
        // Enter computes the insertion against the list before that add.
        contents.retain(|candidate| *candidate != child);
        let position = self.contents_insert_position(&contents, child, None);
        self.contents_link_operations
            .push(ContentsLinkOperation::Insert {
                container,
                child,
                position,
            });
    }

    pub(crate) fn move_content_link_to_back(
        &mut self,
        container: ObjectId,
        child: ObjectId,
    ) -> bool {
        let linked = self
            .get_world_object(container)
            .is_some_and(|object| object.contents().contains(&child));
        if linked {
            // FnScrollContents removes the raw first live link and appends
            // that same link with stNone (C4Script.cpp:1879-1891).
            self.contents_link_operations
                .push(ContentsLinkOperation::MoveToBack { container, child });
        }
        linked
    }

    pub(crate) fn rotate_contents_link_to_front(
        &mut self,
        container: ObjectId,
        child: ObjectId,
    ) -> bool {
        let linked = self
            .get_world_object(container)
            .is_some_and(|object| object.contents().contains(&child));
        if linked {
            // C4ObjectList::ShiftContents is one imperative cyclic relink,
            // not a persistent front preference (C4ObjectList.cpp:815-833).
            self.contents_link_operations
                .push(ContentsLinkOperation::RotateToFront { container, child });
        }
        linked
    }

    /// The live cell for a FOREIGN object's named local (cross-object
    /// LocalN). Seeded from the freshest known value: an accumulated
    /// nested-call state first, the world snapshot otherwise.
    fn foreign_local_cell(&mut self, target: ObjectId, name: &str) -> clonk_script::ValueCell {
        // An object with an in-flight VM session shares its LIVE cells:
        // the foreign write mutates the running call's storage directly
        // and its fold carries it (C++ mutates the one live C4Object).
        if let Some(cells) = self.session_local_cells.get(&target) {
            return cells.cell(name);
        }
        if let Some(cell) = self.foreign_local_cells.get(&(target, name.to_string())) {
            return cell.clone();
        }
        let seed = self
            .nested_objects
            .get(&target)
            .and_then(|state| state.local_vars.get(name).cloned())
            .or_else(|| {
                self.get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.local_vars.clone()))
                    .and_then(|locals| locals.get(name).cloned())
            })
            .unwrap_or(Value::Nil);
        let cell = clonk_script::value_cell(seed);
        self.foreign_local_cells
            .insert((target, name.to_string()), cell.clone());
        cell
    }

    /// Cross-object LocalN writes must be visible to a later nested call
    /// on the same target (C++ mutates live state mid-call).
    pub(crate) fn overlay_foreign_cells(
        &self,
        target: ObjectId,
        locals: &mut HashMap<String, Value>,
    ) {
        for ((object, name), cell) in &self.foreign_local_cells {
            if *object == target {
                locals.insert(name.clone(), cell.borrow().clone());
            }
        }
    }

    /// ...and a nested call's writes must be visible to later LocalN reads.
    fn sync_foreign_cells(&mut self, target: ObjectId, locals: &HashMap<String, Value>) {
        for ((object, name), cell) in &self.foreign_local_cells {
            if *object == target {
                if let Some(value) = locals.get(name) {
                    *cell.borrow_mut() = value.clone();
                }
            }
        }
    }

    /// Phase 1 of a nested call (borrow held): resolve the target's script
    /// and move its scope to active. Function resolution follows
    /// `FindSameNameFunc` (C4Aul.cpp:130-148): the target def's own script
    /// function wins, engine (host) functions are the fallback, anything
    /// else is a silent miss (`None`).
    fn prepare_nested_call(
        &mut self,
        target: ObjectId,
        function: &str,
        host_fallback: bool,
        include_globals: bool,
        script_override: Option<Arc<ScriptEngine>>,
        allow_scope_without_world_object: bool,
        function_is_pinned: bool,
    ) -> Option<NestedCallPrep> {
        let world_object = self.get_world_object(target);
        if world_object.is_none()
            && !(allow_scope_without_world_object && self.object_scope(target).is_some())
        {
            return None;
        }
        // Namespaced calls (`obj->ID::Func`) run the NAMED def's script in
        // the target's scope (AB_CALLNS); plain calls resolve on the
        // target's own def.
        let script = match script_override {
            Some(script) => script,
            None => {
                let definition_id = self.object_effective_definition_id(target)?;
                self.world.definition_script(&definition_id)?.clone()
            }
        };
        // Recursive/effect calls may fall through to Game.ScriptEngine;
        // C4Object::Call/GetSFunc requires an ordinary function owned by
        // the exact object script. A `global func` leaves only an unnamed
        // link in that host and is not an owner-local candidate.
        let resolvable = function_is_pinned
            || (if include_globals {
                script.has_function_or_global(function)
            } else {
                script.has_local_function(function)
            })
            || (host_fallback && script.has_host_function(function));
        if !resolvable {
            return None;
        }
        // A VM session owns its `local` cells for the length of the call,
        // so a call onto an object whose own script is already in flight
        // cannot see that session's uncommitted writes: it starts from the
        // pre-call snapshot. C++ keeps named locals on the C4Object itself,
        // where the nested call would read them live — a known divergence,
        // narrowed by the `overlay_foreign_cells` pass below, which does
        // replay earlier cross-object LocalN writes onto the snapshot.
        let mut snapshot_locals = world_object
            .as_ref()
            .and_then(|object| object.full_state())
            .map(|state| state.local_vars.snapshot())
            .or_else(|| {
                self.session_local_cells
                    .get(&target)
                    .map(clonk_script::LocalCells::snapshot)
            })
            .unwrap_or_default();
        // Earlier cross-object LocalN writes are part of the target's
        // current state.
        self.overlay_foreign_cells(target, &mut snapshot_locals);
        if self.object.as_ref().map(ObjectScopeContext::id) == Some(target) {
            return Some(NestedCallPrep {
                script,
                local_vars: snapshot_locals,
                origin: None,
            });
        }
        if let Some(index) = self
            .dormant_scopes
            .iter()
            .position(|slot| slot.as_ref().map(ObjectScopeContext::id) == Some(target))
        {
            let scope = self.dormant_scopes[index].take();
            self.dormant_scopes.push(self.object.take());
            self.object = scope;
            return Some(NestedCallPrep {
                script,
                local_vars: snapshot_locals,
                origin: Some(NestedScopeOrigin::Dormant(index)),
            });
        }
        let (scope, mut local_vars) = match self.nested_objects.remove(&target) {
            Some(state) => (state.scope, state.local_vars),
            None => self.nested_scope_for(world_object.as_ref()?)?,
        };
        self.overlay_foreign_cells(target, &mut local_vars);
        self.dormant_scopes.push(self.object.take());
        self.object = Some(scope);
        Some(NestedCallPrep {
            script,
            local_vars,
            origin: Some(NestedScopeOrigin::Completed),
        })
    }

    /// A fresh nested scope from the world snapshot. `None` for objects
    /// without a full-state snapshot (pending spawns of the same call).
    pub(crate) fn nested_scope_for(
        &self,
        object: &HostWorldObject,
    ) -> Option<(ObjectScopeContext, HashMap<String, Value>)> {
        // Legacy host-only fixtures may expose a full object snapshot
        // without a definition table. A pending native-created preview is
        // still a real C4Object in that context; field defaults supply its
        // inert scope so Enter/Exit and removal remain observable.
        let metadata = self.world.definition_metadata(object.definition_id());
        let action_library = metadata
            .map(|metadata| metadata.action_library.clone())
            .unwrap_or_default();
        let ocf_base = metadata.map_or(0, |metadata| metadata.ocf_base);
        let definition_physical =
            metadata.map_or_else(PhysicalInfo::default, |metadata| metadata.physical);
        let state = object.full_state()?;
        // C4Object::SetOCF derives OCF_CrewMember from Def->CrewMember,
        // independently of the player's live Crew roster. Engine-backed
        // nested scopes therefore use definition metadata; old host-only
        // fixtures without a definition table retain their snapshot field.
        let crew_member = metadata.map_or(state.crew_member, |metadata| metadata.crew_member);
        let mut scope = ObjectScopeContext::new(
            object.id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.construction,
            state.alive,
            state.in_liquid,
            state.own_mass,
            state.owner,
            state.controller,
            state.category,
            state.position,
            state.velocity,
            state.rotation,
            state.effects.clone(),
            action_library,
            state.action.name.clone(),
            state.action.act_map_index,
            state.action.time,
            state.action.data,
            state.action.phase,
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            state.shape_vertices.clone(),
            ocf_base,
            crew_member,
            state.plr_view_range,
            state.graphics_overlays.clone(),
            state.base_graphics.clone(),
            state.draw_transform,
            state.info_physical,
            state.temporary_physical,
            state.physical_changes.clone(),
            definition_physical,
        );
        // Engine projections may borrow the authoritative ObjectState rather
        // than an owned script snapshot. Preserve the callback-entry active
        // shape that `script_state_snapshot` installs for owned projections.
        scope.shape_vertices.replace_active(object.vertices());
        scope.current_info_rank = self
            .world
            .crew_rank(object.id.as_u64())
            .or_else(|| scope.info_physical.map(|_| 0));
        scope.current_info_link = self.world.crew_info_link(object.id);
        scope.current_info_core = self.world.crew_infos.get(&object.id).cloned();
        scope.definition_id = Some(object.definition_id().to_string());
        scope.configure_fair_crew(&self.world);
        scope.current_fixed_position = object.fixed_position;
        scope.current_fixed_velocity = object.fixed_velocity;
        scope.current_fixed_rotation = object.fixed_rotation;
        scope.current_rotation_velocity = object.rotation_velocity;
        scope.current_compiler_cache = object.compiler_cache.clone();
        scope.current_mobile = state.mobile;
        scope.current_t_attach = state.t_attach;
        scope.current_contact_density = state.contact_density;
        scope.current_contents_link_generation = state.contents_link_generation;
        scope.unsorted = object.unsorted;
        scope.staged_own_vertices = object.own_vertices;
        scope.walk_rotation = WalkRotationSeed {
            rotateable: metadata.map_or(0, |metadata| metadata.rotateable),
            t_attach: state.t_attach,
            attach: state.shape_attach,
            def_attach_vtx_x: usize::try_from(state.shape_attach.vtx)
                .ok()
                .and_then(|vtx| metadata.and_then(|metadata| metadata.vertices.get(vtx)))
                .map(|vertex| vertex.x)
                .unwrap_or(0),
        };
        scope
            .live_commands
            .restore_from_snapshot(&object.command_stack);
        scope.current_magic_energy = state.magic_energy;
        scope.current_breath = state.breath;
        scope.current_need_energy = state.need_energy;
        scope.current_selected = state.selected;
        scope.current_no_collect_delay = state.no_collect_delay;
        // FnGetOCF reads the cached obj->OCF (C4Script.cpp:1354-1358) —
        // nested scopes carry the snapshot mask like outer scopes do, not
        // the preview-grade recompute.
        scope.cached_ocf = Some(state.ocf);
        let local_vars = state.local_vars.snapshot();
        Some((scope, local_vars))
    }

    /// Phase 3 of a nested call (borrow re-taken): move the finished scope
    /// back to where it came from. Completed scopes keep `local_vars` for
    /// resumption and the outcome fold.
    fn finish_nested_call(
        &mut self,
        target: ObjectId,
        origin: NestedScopeOrigin,
        local_vars: HashMap<String, Value>,
    ) {
        // The call's writes become visible to later cross-object LocalN
        // reads on the same target.
        self.sync_foreign_cells(target, &local_vars);
        let finished = self.object.take();
        self.object = self.dormant_scopes.pop().unwrap_or(None);
        match origin {
            NestedScopeOrigin::Dormant(index) => {
                if let Some(slot) = self.dormant_scopes.get_mut(index) {
                    *slot = finished;
                }
            }
            NestedScopeOrigin::Completed => {
                if let Some(scope) = finished {
                    if !self.nested_order.contains(&target) {
                        self.nested_order.push(target);
                    }
                    self.nested_objects
                        .insert(target, NestedScopeState { scope, local_vars });
                }
            }
        }
    }

    /// The scope currently holding `target`'s pending writes: the active
    /// scope, a dormant (in-flight outer) scope, or a completed nested one.
    pub(crate) fn object_scope(&self, target: ObjectId) -> Option<&ObjectScopeContext> {
        self.object
            .as_ref()
            .filter(|scope| scope.id == target)
            .or_else(|| {
                self.dormant_scopes
                    .iter()
                    .flatten()
                    .find(|scope| scope.id == target)
            })
            .or_else(|| self.nested_objects.get(&target).map(|state| &state.scope))
    }

    pub(crate) fn object_scope_mut(&mut self, target: ObjectId) -> Option<&mut ObjectScopeContext> {
        if self.object.as_ref().map(ObjectScopeContext::id) == Some(target) {
            return self.object.as_mut();
        }
        if self
            .dormant_scopes
            .iter()
            .flatten()
            .any(|scope| scope.id == target)
        {
            return self
                .dormant_scopes
                .iter_mut()
                .flatten()
                .find(|scope| scope.id == target);
        }
        self.nested_objects
            .get_mut(&target)
            .map(|state| &mut state.scope)
    }

    /// Clone an owned physical-resolution plan while the host context is
    /// borrowed. The returned plan deliberately performs no script work and
    /// may therefore be resolved after releasing the TLS `Ref`/`RefMut`.
    pub(crate) fn prepare_object_physical(
        &self,
        target: ObjectId,
        permanent: bool,
    ) -> Option<PhysicalResolution> {
        if let Some(scope) = self.object_scope(target) {
            return Some(scope.prepare_resolved_physical(permanent));
        }
        let object = self.get_world_object(target)?;
        self.nested_scope_for(&object)
            .map(|(scope, _)| scope.prepare_resolved_physical(permanent))
    }

    pub(crate) fn refresh_scope_fair_crew(&mut self, target: ObjectId) {
        let world = self.world.clone();
        if let Some(scope) = self.object_scope_mut(target) {
            scope.configure_fair_crew(&world);
        }
    }

    /// A C4Object pointer is callable while its scope is in-flight even when
    /// the object has not yet entered the copied world/master list.
    pub(crate) fn has_callable_object(&self, target: ObjectId) -> bool {
        self.object_scope(target).is_some() || self.get_world_object(target).is_some()
    }

    /// Stage one C4Object::DoCon call, including its percent-step component
    /// cutoff/gain. Keeping this at call time preserves multiple DoCon and
    /// SetComponent ordering inside one script callback.
    pub(crate) fn adjust_object_construction(
        &mut self,
        target: ObjectId,
        delta: i32,
    ) -> Option<i32> {
        self.adjust_object_construction_mode(target, delta, true)
    }

    /// The same construction/component fold for a live non-initial DoCon.
    /// Its caller performs AssignRemoval synchronously after the callback and
    /// position side arms, so zero construction must remain callable here.
    pub(crate) fn stage_live_docon_construction(
        &mut self,
        target: ObjectId,
        delta: i32,
    ) -> Option<i32> {
        self.adjust_object_construction_mode(target, delta, false)
    }

    fn adjust_object_construction_mode(
        &mut self,
        target: ObjectId,
        delta: i32,
        destroy_at_zero: bool,
    ) -> Option<i32> {
        let scope = self.object_scope(target)?;
        let before = scope.construction();
        let definition_id = scope
            .pending_update
            .change_def
            .clone()
            .or_else(|| scope.definition_id.clone())
            .or_else(|| {
                self.get_world_object(target)
                    .map(|object| object.definition_id().to_string())
            });
        let (definition_components, oversize) = definition_id
            .as_deref()
            .and_then(|id| self.definition_metadata(id))
            .map(|metadata| (metadata.components.clone(), metadata.fire.oversize))
            .unwrap_or_default();
        let pending_components = scope.pending_update.components.clone();
        let pending_component_order = scope.pending_update.component_order.clone();
        let pending_spawn_components = self
            .pending_spawns
            .iter()
            .find(|spawn| spawn.id == Some(target))
            .map(|spawn| {
                spawn.components.clone().unwrap_or_else(|| {
                    let initial_components = self
                        .definition_metadata(&spawn.definition_id)
                        .map(|metadata| metadata.components.as_slice())
                        .unwrap_or_default();
                    crate::definition_component_counts(initial_components, before)
                })
            });
        let current_components = pending_components
            .or(pending_spawn_components)
            .or_else(|| {
                self.get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.components.clone()))
            })
            .unwrap_or_default();
        let current_component_order = pending_component_order
            .or_else(|| {
                self.pending_spawns
                    .iter()
                    .find(|spawn| spawn.id == Some(target))
                    .and_then(|spawn| spawn.component_order.clone())
            })
            .or_else(|| {
                self.get_world_object(target).and_then(|object| {
                    object
                        .full_state()
                        .map(|state| state.component_order.clone())
                })
            })
            .unwrap_or_else(|| {
                definition_components
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect()
            });

        let after = self
            .object_scope_mut(target)?
            .adjust_construction(delta, oversize);
        if crate::docon_refreshes_construction(before, after) {
            let components = crate::docon_component_counts(
                &current_components,
                &current_component_order,
                &definition_components,
                after,
                delta,
            );
            if let Some(scope) = self.object_scope_mut(target) {
                scope.pending_update.components = Some(components);
                scope.pending_update.component_order = Some(current_component_order);
            }
        }
        if destroy_at_zero && after == 0 {
            if let Some(scope) = self.object_scope_mut(target) {
                scope.destroy = true;
            }
        }
        Some(after)
    }

    pub(crate) fn object_crew_disabled(&self, target: ObjectId) -> Option<bool> {
        if let Some(disabled) = self
            .object_scope(target)
            .and_then(|scope| scope.pending_update.crew_disabled)
        {
            return Some(disabled);
        }
        self.get_world_object(target)
            .map(|object| object.crew_disabled)
    }

    pub(crate) fn set_object_selected(&mut self, target: ObjectId, selected: bool) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.set_selected(selected))
            .is_some()
    }

    /// Live `C4Object::SetPlrViewRange`: update the object word and
    /// synchronously resort it in the current owner's FoWViewObjs list.
    pub(crate) fn set_object_plr_view_range(&mut self, target: ObjectId, range: i32) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        let Some(owner) = self.object_scope_mut(target).map(|scope| {
            scope.set_plr_view_range(range);
            scope.owner()
        }) else {
            return false;
        };
        self.world
            .actualize_player_fow_view_object(target, owner, range);
        true
    }

    /// Live `C4Object::PlrFoWActualize` without changing PlrViewRange. The
    /// same-value update token replays the list remove/add ordering when the
    /// copied host outcome is folded back into the authoritative Engine.
    pub(crate) fn actualize_object_plr_view_range(&mut self, target: ObjectId) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        let Some((owner, range)) = self.object_scope_mut(target).map(|scope| {
            let range = scope.plr_view_range();
            scope.pending_update.plr_view_range = Some(range);
            (scope.owner(), range)
        }) else {
            return false;
        };
        self.world
            .actualize_player_fow_view_object(target, owner, range);
        true
    }

    /// Materializes a nested scope for `target` so per-object writes (menus)
    /// can fold through the standard nested-outcome pipeline even when no
    /// script call ever ran on the target this session. False for unknown
    /// objects and same-call pending spawns (no full-state snapshot yet).
    pub(crate) fn ensure_object_scope(&mut self, target: ObjectId) -> bool {
        if self.object_scope(target).is_some() {
            return true;
        }
        let Some(world_object) = self.get_world_object(target) else {
            return false;
        };
        let Some((scope, local_vars)) = self.nested_scope_for(&world_object) else {
            return false;
        };
        if !self.nested_order.contains(&target) {
            self.nested_order.push(target);
        }
        self.nested_objects
            .insert(target, NestedScopeState { scope, local_vars });
        true
    }

    /// The freshest menu state known for `target` mid-call: a pending write
    /// in any scope wins over the world snapshot (C++ mutates the live
    /// C4Object::Menu).
    pub(crate) fn object_menu(&self, target: ObjectId) -> Option<crate::ObjectMenuState> {
        if let Some(menu) = self
            .object_scope(target)
            .and_then(|scope| scope.pending_update.menu.as_ref())
        {
            return menu.clone();
        }
        self.get_world_object(target)
            .and_then(|object| object.full_state().and_then(|state| state.menu.clone()))
    }

    /// Records a menu write for `target` (Some = open/replace, None =
    /// closed). False when no scope can be materialized for the target.
    pub(crate) fn set_object_menu(
        &mut self,
        target: ObjectId,
        menu: Option<crate::ObjectMenuState>,
    ) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.pending_update.menu = Some(menu))
            .is_some()
    }

    /// The effective C4Object::CustomName mid-call. A pending clear must
    /// shadow the frame-start snapshot just as a pending set does.
    pub(crate) fn object_custom_name(&self, target: ObjectId) -> Option<String> {
        if let Some(custom_name) = self
            .object_scope(target)
            .and_then(|scope| scope.pending_update.custom_name.as_ref())
        {
            return custom_name.clone().filter(|name| !name.is_empty());
        }
        self.world
            .get_shared(target)
            .and_then(|object| object.full_state().map(|state| state.custom_name.clone()))
            .flatten()
            .filter(|name| !name.is_empty())
    }

    /// C4Object::GetName: pending/live CustomName, crew Info name, then the
    /// effective definition name. GetDataString uses this same chain.
    pub(crate) fn object_effective_name(&self, target: ObjectId) -> Option<String> {
        if let Some(custom_name) = self.object_custom_name(target) {
            return Some(custom_name);
        }
        let info_name = match self.object_scope(target) {
            Some(scope) => scope.info_core().map(|info| info.name.clone()),
            None => self
                .world
                .crew_infos
                .get(&target)
                .map(|info| info.name.clone()),
        };
        if info_name.is_some() {
            return info_name;
        }
        self.object_effective_definition_id(target)
            .and_then(|id| self.definition_metadata(&id))
            .map(|metadata| metadata.name.clone())
    }

    /// Stage an ordinary-object SetName write through the normal scope fold.
    pub(crate) fn set_object_custom_name(
        &mut self,
        target: ObjectId,
        custom_name: Option<String>,
    ) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.pending_update.custom_name = Some(custom_name))
            .is_some()
    }

    /// The effective C4Object::pLayer mid-call. A pending clear shadows the
    /// frame-start state just like a pending layer assignment.
    pub(crate) fn object_layer(&self, target: ObjectId) -> Option<ObjectId> {
        if let Some(layer) = self
            .object_scope(target)
            .and_then(|scope| scope.pending_update.layer.as_ref())
        {
            return *layer;
        }
        self.get_world_object(target)
            .and_then(|object| object.full_state().and_then(|state| state.layer))
    }

    /// The raw `C4EnumeratedObjectPtr::number` paired with pLayer. Fresh
    /// children copy this word from their creator independently of whether
    /// that creator's live pointer resolved (C4Object::Init copy assignment).
    pub(crate) fn object_layer_compiler_cache(&self, target: ObjectId) -> i32 {
        self.object_scope(target)
            .map(|scope| scope.current_compiler_cache.layer)
            .or_else(|| {
                self.get_world_object(target)
                    .map(|object| object.compiler_cache.layer)
            })
            .unwrap_or(0)
    }

    /// Stage one object's pLayer through the normal nested-scope fold.
    pub(crate) fn set_object_layer(&mut self, target: ObjectId, layer: Option<ObjectId>) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.pending_update.layer = Some(layer))
            .is_some()
    }

    /// Effective C4Object::Visibility, including pending same-call writes.
    pub(crate) fn object_visibility(&self, target: ObjectId) -> Option<i32> {
        self.object_scope(target)
            .and_then(|scope| scope.pending_update.visibility)
            .or_else(|| {
                self.get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.visibility))
            })
    }

    pub(crate) fn set_object_visibility(&mut self, target: ObjectId, visibility: i32) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.pending_update.visibility = Some(visibility))
            .is_some()
    }

    /// Effective C4Object::BlitMode, including pending same-call writes.
    pub(crate) fn object_blit_mode(&self, target: ObjectId) -> Option<u32> {
        let scope = self.object_scope(target);
        if let Some(blit_mode) = scope.and_then(|scope| scope.pending_update.blit_mode) {
            return Some(blit_mode);
        }
        let current = self
            .get_world_object(target)
            .and_then(|object| object.full_state().map(|state| state.blit_mode))?;
        if current & GFX_BLIT_CUSTOM == 0 {
            if let Some(blit_mode) = scope
                .and_then(|scope| scope.pending_update.change_def.as_deref())
                .and_then(|definition| self.definition_metadata(definition))
                .map(|metadata| metadata.blit_mode)
            {
                return Some(blit_mode);
            }
        }
        Some(current)
    }

    pub(crate) fn set_object_blit_mode(&mut self, target: ObjectId, blit_mode: u32) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.pending_update.blit_mode = Some(blit_mode))
            .is_some()
    }

    pub(crate) fn object_color_modulation(&self, target: ObjectId) -> Option<u32> {
        self.object_scope(target)
            .and_then(|scope| scope.pending_update.color_modulation)
            .or_else(|| {
                self.get_world_object(target)
                    .and_then(|object| object.full_state().map(|state| state.color_modulation))
            })
    }

    pub(crate) fn set_object_color_modulation(&mut self, target: ObjectId, color: u32) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        self.object_scope_mut(target)
            .map(|scope| scope.pending_update.color_modulation = Some(color))
            .is_some()
    }

    pub(crate) fn object_overlay_color_modulation(
        &self,
        target: ObjectId,
        overlay_id: i32,
    ) -> Option<u32> {
        if let Some(scope) = self.object_scope(target) {
            return scope
                .graphics_overlays
                .iter()
                .find(|overlay| overlay.id == overlay_id)
                .map(|overlay| overlay.color_modulation);
        }
        self.get_world_object(target).and_then(|object| {
            object.full_state().and_then(|state| {
                state
                    .graphics_overlays
                    .iter()
                    .find(|overlay| overlay.id == overlay_id)
                    .map(|overlay| overlay.color_modulation)
            })
        })
    }

    pub(crate) fn object_has_graphics_overlay(&self, target: ObjectId, overlay_id: i32) -> bool {
        if let Some(scope) = self.object_scope(target) {
            return scope
                .graphics_overlays
                .iter()
                .any(|overlay| overlay.id == overlay_id);
        }
        self.get_world_object(target).is_some_and(|object| {
            object.full_state().is_some_and(|state| {
                state
                    .graphics_overlays
                    .iter()
                    .any(|overlay| overlay.id == overlay_id)
            })
        })
    }

    pub(crate) fn set_object_overlay_color_modulation(
        &mut self,
        target: ObjectId,
        overlay_id: i32,
        color: u32,
    ) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        let Some(scope) = self.object_scope_mut(target) else {
            return false;
        };
        let Some(overlay) = scope
            .graphics_overlays
            .iter_mut()
            .find(|overlay| overlay.id == overlay_id)
        else {
            return false;
        };
        overlay.color_modulation = color;
        scope.pending_update.graphics_overlays = Some(scope.graphics_overlays.clone());
        true
    }

    /// The target's effective definition follows a same-call ChangeDef.
    pub(crate) fn object_effective_definition_id(&self, target: ObjectId) -> Option<String> {
        self.object_scope(target)
            .and_then(|scope| {
                scope
                    .pending_update
                    .change_def
                    .clone()
                    .or_else(|| scope.definition_id.clone())
            })
            .or_else(|| {
                self.get_world_object(target)
                    .map(|object| object.definition_id().to_string())
            })
    }

    pub(crate) fn object_definition_blit_mode(&self, target: ObjectId) -> Option<u32> {
        let definition_id = self.object_effective_definition_id(target)?;
        self.definition_metadata(&definition_id)
            .map(|metadata| metadata.blit_mode)
    }

    pub(crate) fn object_overlay_blit_mode(
        &self,
        target: ObjectId,
        overlay_id: i32,
    ) -> Option<u32> {
        if let Some(scope) = self.object_scope(target) {
            return scope
                .graphics_overlays
                .iter()
                .find(|overlay| overlay.id == overlay_id)
                .map(|overlay| overlay.blit_mode);
        }
        self.get_world_object(target).and_then(|object| {
            object.full_state().and_then(|state| {
                state
                    .graphics_overlays
                    .iter()
                    .find(|overlay| overlay.id == overlay_id)
                    .map(|overlay| overlay.blit_mode)
            })
        })
    }

    pub(crate) fn set_object_overlay_blit_mode(
        &mut self,
        target: ObjectId,
        overlay_id: i32,
        blit_mode: u32,
    ) -> bool {
        if !self.ensure_object_scope(target) {
            return false;
        }
        let Some(scope) = self.object_scope_mut(target) else {
            return false;
        };
        let Some(overlay) = scope
            .graphics_overlays
            .iter_mut()
            .find(|overlay| overlay.id == overlay_id)
        else {
            return false;
        };
        overlay.blit_mode = blit_mode;
        scope.pending_update.graphics_overlays = Some(scope.graphics_overlays.clone());
        true
    }

    /// C4Object::Status of `target` as the current call sees it.
    pub(crate) fn object_status_active(&self, target: ObjectId) -> bool {
        self.object_scope(target)
            .map(|scope| scope.status().is_active())
            .unwrap_or_else(|| {
                self.get_world_object(target)
                    .map(|object| object.status().is_active())
                    .unwrap_or(false)
            })
    }

    /// C++ truthiness of C4Object::Status: Normal=1 and Inactive=2 are both
    /// valid for APIs that test only `!pObj->Status`.
    pub(crate) fn object_status_present(&self, target: ObjectId) -> bool {
        self.object_scope(target)
            .map(|scope| !scope.destroy && scope.status() != ObjectStatus::Deleted)
            .unwrap_or_else(|| {
                self.get_world_object(target)
                    .map(|object| object.status() != ObjectStatus::Deleted)
                    .unwrap_or(false)
            })
    }

    /// Capture the effective Picture2Facet inputs at the instant a script
    /// adds an Object/ObjectRank menu image. Pending same-call writes must
    /// win over the frame-start object snapshot just as they do for C++'s
    /// live object (`C4Script.cpp:1617-1678`).
    pub(crate) fn object_menu_picture_snapshot(
        &self,
        target: ObjectId,
        include_rank: bool,
        symbol_size: i32,
    ) -> Option<crate::ObjectMenuPictureSnapshot> {
        let object = self.get_world_object(target)?;
        if include_rank && object.status() == ObjectStatus::Deleted {
            return None;
        }
        let state = object.full_state()?;
        let scope = self.object_scope(target);
        let definition_id = self.object_effective_definition_id(target)?;
        Some(crate::ObjectMenuPictureSnapshot {
            definition_id,
            symbol_size,
            base_graphics: scope
                .map(|scope| scope.base_graphics.clone())
                .unwrap_or_else(|| state.base_graphics.clone()),
            graphics_overlays: scope
                .map(|scope| scope.graphics_overlays.clone())
                .unwrap_or_else(|| state.graphics_overlays.clone()),
            blit_mode: self.object_blit_mode(target).unwrap_or(state.blit_mode),
            color: scope
                .and_then(|scope| scope.pending_update.color)
                .unwrap_or(state.color),
            color_modulation: self
                .object_color_modulation(target)
                .unwrap_or(state.color_modulation),
            picture_rect: scope
                .and_then(|scope| scope.pending_update.picture_rect)
                .unwrap_or(state.picture_rect),
            rank: include_rank
                .then(|| {
                    scope
                        .and_then(ObjectScopeContext::info_rank)
                        .or_else(|| self.world.crew_rank(target.as_u64()))
                })
                .flatten(),
        })
    }

    /// Live C4Object::CanConcatPictureWith shared by script ShiftContents and
    /// internal menu refill rows. Pending scope overlays make nested calls see
    /// the same picture grouping as the ordinary Engine builder.
    pub(crate) fn object_can_concat_picture_with(&self, object: ObjectId, other: ObjectId) -> bool {
        let Some(object_picture) = self.object_menu_picture_snapshot(object, false, 0) else {
            return false;
        };
        let Some(other_picture) = self.object_menu_picture_snapshot(other, false, 0) else {
            return false;
        };
        if object_picture.definition_id != other_picture.definition_id {
            return false;
        }
        let definition_id = object_picture.definition_id.as_str();
        let Some(definition) = self.definition_metadata(definition_id) else {
            return false;
        };
        let allowed = definition.allow_picture_stack;
        if allowed & crate::APS_COLOR == 0 {
            if self.world.definition_color_by_owner(definition_id)
                && object_picture.color != other_picture.color
            {
                return false;
            }
            if object_picture.color_modulation != other_picture.color_modulation
                || object_picture.blit_mode != other_picture.blit_mode
            {
                return false;
            }
        }
        if allowed & crate::APS_GRAPHICS == 0 {
            fn graphics_key(picture: &crate::ObjectMenuPictureSnapshot) -> (&str, Option<&str>) {
                picture
                    .base_graphics
                    .as_ref()
                    .map(|graphics| {
                        (
                            graphics.definition.as_str(),
                            graphics.graphics_name.as_deref(),
                        )
                    })
                    .unwrap_or((picture.definition_id.as_str(), None))
            }
            let (object_definition, object_name) = graphics_key(&object_picture);
            let (other_definition, other_name) = graphics_key(&other_picture);
            if !crate::resolved_graphics_equal(
                Some(object_definition),
                object_name,
                Some(other_definition),
                other_name,
            ) || object_picture.picture_rect != other_picture.picture_rect
            {
                return false;
            }
        }
        if allowed & crate::APS_NAME == 0 {
            let object_name = self
                .object_custom_name(object)
                .or_else(|| {
                    self.object_scope(object)
                        .and_then(ObjectScopeContext::info_core)
                        .map(|info| info.name.clone())
                })
                .or_else(|| {
                    self.world
                        .crew_infos
                        .get(&object)
                        .map(|info| info.name.clone())
                })
                .unwrap_or_else(|| definition.name.clone());
            let other_name = self
                .object_custom_name(other)
                .or_else(|| {
                    self.object_scope(other)
                        .and_then(ObjectScopeContext::info_core)
                        .map(|info| info.name.clone())
                })
                .or_else(|| {
                    self.world
                        .crew_infos
                        .get(&other)
                        .map(|info| info.name.clone())
                })
                .unwrap_or_else(|| definition.name.clone());
            if object_name != other_name {
                return false;
            }
        }
        if allowed & crate::APS_OVERLAY == 0 {
            for overlay in object_picture
                .graphics_overlays
                .iter()
                .filter(|overlay| overlay.mode == GraphicsOverlayMode::Picture)
            {
                let Some(other_overlay) = other_picture
                    .graphics_overlays
                    .iter()
                    .find(|candidate| candidate.id == overlay.id)
                else {
                    return false;
                };
                if !crate::picture_overlays_equal(other_overlay, overlay) {
                    return false;
                }
            }
            for overlay in other_picture
                .graphics_overlays
                .iter()
                .filter(|overlay| overlay.mode == GraphicsOverlayMode::Picture)
            {
                if !object_picture
                    .graphics_overlays
                    .iter()
                    .any(|candidate| candidate.id == overlay.id)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Whether a nested call removed the object — the C++ Status re-check
    /// after `Check` (C4FindObject.cpp:186-199) against the deferred-destroy
    /// model.
    pub(crate) fn nested_object_destroyed(&self, id: ObjectId) -> bool {
        self.nested_objects
            .get(&id)
            .map(|state| state.scope.destroy || state.scope.status() == ObjectStatus::Deleted)
            .unwrap_or(false)
    }

    pub(crate) fn world_object_ids(&self) -> Vec<ObjectId> {
        if let Some(order) = &self.master_order_preview {
            return order.clone();
        }
        self.all_world_object_ids()
    }

    /// Storage-order objects, including inactive entries omitted from the
    /// active master-list preview and same-call pending creations.
    pub(crate) fn all_world_object_ids(&self) -> Vec<ObjectId> {
        let mut ids = self.world.object_ids();
        ids.extend(self.pending_order.iter().copied());
        ids
    }

    pub(crate) fn master_object_ids(&self) -> Vec<ObjectId> {
        if let Some(order) = &self.master_order_preview {
            return order.clone();
        }
        let mut ids = self.world.master_object_ids().to_vec();
        ids.extend(self.pending_order.iter().copied());
        ids
    }

    /// Preview `C4Object::UpdatePos` after a same-call shape/position write.
    /// `SetShape` invokes this synchronously in C++, so later bounded Find
    /// calls must already enumerate the object's new ObjectShapes sectors.
    pub(crate) fn preview_live_object_sector(&self, target: ObjectId) {
        let Some(object) = self.get_world_object(target) else {
            return;
        };
        if !object.status().is_active() {
            return;
        }
        let record = SectorObject {
            id: target,
            position: object.position(),
            shape_rect: sector_shape_rect(effect_object_live_shape_rect(self, &object)),
        };
        self.world
            .preview_object_sector_update(record, &self.master_object_ids());
    }

    fn commit_object_status_preview(&mut self, target: ObjectId, ids: Vec<ObjectId>) {
        if let Some(object) = self.get_world_object(target) {
            self.world.preview_object_status_sector(&object, &ids);
        }
        self.master_order_preview = Some(ids);
    }

    /// Preview C4Object::StatusDeactivate/StatusActivate's synchronous list
    /// transition for callbacks nested before the host outcome folds into
    /// Engine. Activation uses the same stMain insertion rules as the
    /// authoritative exec-list fold: same category/definition cluster first,
    /// then the category bracket; lines and Unsorted objects append.
    pub(crate) fn preview_object_status_change(&mut self, target: ObjectId, status: ObjectStatus) {
        let mut ids = self
            .master_order_preview
            .clone()
            .unwrap_or_else(|| self.world.master_object_ids().to_vec());
        ids.retain(|id| *id != target);
        if status != ObjectStatus::Normal {
            self.commit_object_status_preview(target, ids);
            return;
        }

        let Some((category, definition_id)) = self.contents_sort_key(target) else {
            ids.push(target);
            self.commit_object_status_preview(target, ids);
            return;
        };
        let is_line = self
            .definition_metadata(&definition_id)
            .is_some_and(|metadata| metadata.line != 0);
        if is_line || self.contents_object_unsorted(target) {
            ids.push(target);
            self.commit_object_status_preview(target, ids);
            return;
        }

        let sort_category = category & CATEGORY_SORT_LIMIT;
        let mut predecessor = None;
        let mut found_cluster = false;
        if category & crate::CATEGORY_STATIC_BACK == 0 {
            for (position, other) in ids.iter().copied().enumerate() {
                let live_sorted = self
                    .get_world_object(other)
                    .is_some_and(|object| object.status().is_active())
                    && !self.contents_object_unsorted(other);
                if !live_sorted {
                    continue;
                }
                let Some((other_category, other_definition)) = self.contents_sort_key(other) else {
                    continue;
                };
                if other_category & CATEGORY_SORT_LIMIT == sort_category
                    && other_definition == definition_id
                {
                    found_cluster = true;
                    break;
                }
                predecessor = Some(position);
            }
        }
        if !found_cluster {
            predecessor = None;
            for (position, other) in ids.iter().copied().enumerate() {
                let live_sorted = self
                    .get_world_object(other)
                    .is_some_and(|object| object.status().is_active())
                    && !self.contents_object_unsorted(other);
                if !live_sorted {
                    continue;
                }
                let Some((other_category, _)) = self.contents_sort_key(other) else {
                    continue;
                };
                if other_category & CATEGORY_SORT_LIMIT <= sort_category {
                    break;
                }
                predecessor = Some(position);
            }
        }
        ids.insert(predecessor.map_or(0, |position| position + 1), target);
        self.commit_object_status_preview(target, ids);
    }

    pub(crate) fn preview_sort_master_by_category(&mut self) {
        let mut ids = self
            .master_order_preview
            .clone()
            .unwrap_or_else(|| self.world.master_object_ids().to_vec());
        let mut seen = ids.iter().copied().collect::<HashSet<_>>();
        ids.extend(
            self.world
                .object_ids()
                .iter()
                .chain(&self.pending_order)
                .copied()
                .filter(|id| seen.insert(*id)),
        );
        ids.retain(|id| {
            self.get_world_object(*id)
                .is_some_and(|object| object.status() != ObjectStatus::Inactive)
        });
        ids.sort_by(|left, right| {
            let left_category = self
                .get_world_object(*left)
                .map(|object| object.category() & CATEGORY_SORT_LIMIT)
                .unwrap_or(0);
            let right_category = self
                .get_world_object(*right)
                .map(|object| object.category() & CATEGORY_SORT_LIMIT)
                .unwrap_or(0);
            right_category.cmp(&left_category)
        });
        self.master_order_preview = Some(ids);
    }

    /// `cthr->Obj` for the executing host call: the FindObject family
    /// excludes the caller and searches caller-relative coordinates on
    /// local calls (C4Script.cpp:2115-2131).
    pub(crate) fn caller_scope(&self) -> Option<(ObjectId, Vector2)> {
        let caller = self.script_object_context?;
        self.object_scope(caller)
            .map(|scope| (caller, scope.effective_position()))
            .or_else(|| {
                self.get_world_object(caller)
                    .map(|object| (caller, object.position()))
            })
    }

    pub(crate) fn definition_category(&self, id: &str) -> Option<i32> {
        self.world.definition_category(id)
    }

    pub(crate) fn definition_metadata(&self, id: &str) -> Option<&DefinitionMetadata> {
        self.definition_metadata_overrides
            .get(id)
            .or_else(|| self.world.definition_metadata(id))
    }

    /// Stage one mutable `C4Def::Name` write and expose it to the remainder
    /// of this VM session before Engine folds the ordered command.
    pub(crate) fn set_definition_name(&mut self, id: DefinitionId, name: String) -> bool {
        let Some(mut metadata) = self.definition_metadata(id.as_str()).cloned() else {
            return false;
        };
        metadata.name = name.clone();
        if let Some(values) = Rc::make_mut(&mut metadata.fire.def_core_values)
            .def_core
            .get_mut("Name")
        {
            *values = vec![DefCorePrimitive::String(name.clone())];
        }
        self.definition_metadata_overrides
            .insert(id.clone(), metadata);
        self.record_player_command(PlayerCommand::SetDefinitionName {
            definition_id: id,
            name,
        });
        true
    }

    pub(crate) fn landscape_ref(&self) -> Option<&Landscape> {
        self.world.landscape_ref()
    }

    pub(crate) fn runtime_texmap(&self) -> Option<&crate::landscape::RuntimeTexMapState> {
        self.runtime_texmap
            .get_or_init(|| {
                self.world
                    .landscape_ref()
                    .and_then(Landscape::raster_state)
                    .map(|state| state.texmap().clone())
            })
            .as_ref()
    }

    pub(crate) fn runtime_texmap_mut(
        &mut self,
    ) -> Option<&mut crate::landscape::RuntimeTexMapState> {
        if self.runtime_texmap.get().is_none() {
            let initial = self
                .world
                .landscape_ref()
                .and_then(Landscape::raster_state)
                .map(|state| state.texmap().clone());
            let _ = self.runtime_texmap.set(initial);
        }
        self.runtime_texmap
            .get_mut()
            .expect("runtime texture map slot initialized above")
            .as_mut()
    }

    fn take_runtime_texmap(&mut self) -> Option<crate::landscape::RuntimeTexMapState> {
        let _ = self.runtime_texmap_mut();
        self.runtime_texmap.get_mut()?.take()
    }

    pub(crate) fn set_runtime_texmap(&mut self, texmap: crate::landscape::RuntimeTexMapState) {
        if let Some(slot) = self.runtime_texmap.get_mut() {
            *slot = Some(texmap);
        } else {
            let _ = self.runtime_texmap.set(Some(texmap));
        }
    }

    fn snapshot(&self, scope: EffectScope) -> Option<Vec<EffectState>> {
        #[cfg(test)]
        record_effect_snapshot();
        match scope {
            EffectScope::Object(Some(target)) => {
                self.object_scope(target).map(|ctx| ctx.effects.snapshot())
            }
            EffectScope::Object(None) => self.object.as_ref().map(|ctx| ctx.effects.snapshot()),
            EffectScope::Global => self.global.as_ref().map(EffectScopeContext::snapshot),
        }
    }

    pub(crate) fn effect_list_had_head(&self, scope: EffectScope) -> Option<bool> {
        match scope {
            EffectScope::Object(Some(target)) => self
                .object_scope(target)
                .map(|ctx| ctx.effects.had_list_head),
            EffectScope::Object(None) => self.object.as_ref().map(|ctx| ctx.effects.had_list_head),
            EffectScope::Global => self.global.as_ref().map(|ctx| ctx.had_list_head),
        }
    }

    pub(crate) fn player_ids(&self) -> &[i32] {
        self.world.player_ids()
    }

    pub(crate) fn teams(&self) -> &[TeamInfo] {
        &self.teams
    }

    pub(crate) fn player_state(&self, id: i32) -> Option<&PlayerState> {
        self.player_overrides
            .get(&id)
            .or_else(|| self.world.player(id))
    }

    pub(crate) fn player_state_mut(&mut self, id: i32) -> Option<&mut PlayerState> {
        if !self.player_overrides.contains_key(&id) {
            let state = self.world.player(id)?.clone();
            self.player_overrides.insert(id, state);
        }
        self.player_overrides.get_mut(&id)
    }

    pub(crate) fn team_is_full(&self, team_id: i32) -> bool {
        let Some(team) = self.teams.iter().find(|team| team.id == team_id) else {
            return true;
        };
        team.max_players != 0
            && self
                .player_ids()
                .iter()
                .filter(|player| {
                    self.player_state(**player).and_then(|state| state.team) == Some(team_id)
                })
                .count()
                >= team.max_players.max(0) as usize
    }

    /// C4TeamList::GetGenerateTeamByID(TEAMID_New), with the deterministic
    /// prefix of C4Team::RecheckColor shared with engine-side lobby joins.
    /// The returned color remains None after the fixed palette is exhausted:
    /// C++ uses presentation SafeRandom there, which must not consume the
    /// lockstep RNG in this copied host context.
    pub(crate) fn generate_runtime_team(&mut self) -> (Option<TeamInfo>, Option<u32>) {
        if !self.world.auto_generate_teams() {
            return (None, None);
        }
        let Some(id) = self
            .teams
            .iter()
            .map(|team| team.id)
            .fold(0, i32::max)
            .checked_add(1)
        else {
            return (None, None);
        };
        let color = crate::default_generated_team_color(id);
        let team = TeamInfo::new(id, format!("Team {id}"), color.unwrap_or(0));
        self.teams.push(team.clone());
        (Some(team), color)
    }

    /// C4Player::SetPlayerColor's immediate player and owned-object view.
    /// Object writes use the ordinary nested-outcome path so GetColorDw and
    /// callbacks later in this same VM session observe the replacement.
    pub(crate) fn set_player_color_preview(&mut self, player_id: i32, color: u32) {
        let old_color = self
            .player_state(player_id)
            .map(PlayerState::exact_color_dw)
            .unwrap_or(0);
        if old_color == color {
            return;
        }
        if let Some(player) = self.player_state_mut(player_id) {
            player.set_color_dw(color);
        }

        let recolor = self
            .master_object_ids()
            .into_iter()
            .filter_map(|id| {
                if !self.object_status_active(id) {
                    return None;
                }
                let object = self.get_world_object(id)?;
                let scope = self.object_scope(id);
                let owner = scope
                    .map(ObjectScopeContext::owner)
                    .unwrap_or(object.owner());
                if owner != player_id {
                    return None;
                }
                let object_color = scope
                    .and_then(|scope| scope.pending_update.color)
                    .or_else(|| object.full_state().map(|state| state.color))
                    .unwrap_or(0);
                ((object_color & 0x00ff_ffff) == (old_color & 0x00ff_ffff))
                    .then_some((id, object_color))
            })
            .collect::<Vec<_>>();
        for (id, object_color) in recolor {
            if self.ensure_object_scope(id) {
                if let Some(scope) = self.object_scope_mut(id) {
                    scope.pending_update.color =
                        Some((object_color & 0xff00_0000) | (color & 0x00ff_ffff));
                }
            }
        }
    }

    pub(crate) fn synchronize_team_hostility(&mut self, player_id: i32, team: i32) {
        let others = self
            .player_ids()
            .iter()
            .copied()
            .filter(|other| *other != player_id)
            .map(|other| {
                let hostile = self.player_state(other).and_then(|state| state.team) != Some(team);
                (other, hostile)
            })
            .collect::<Vec<_>>();
        for (other, hostile) in others {
            if let Some(player) = self.player_state_mut(player_id) {
                set_player_hostility_declaration(player, other, hostile);
            }
            if let Some(opponent) = self.player_state_mut(other) {
                set_player_hostility_declaration(opponent, player_id, hostile);
            }
        }
    }

    /// C4ObjectList::Add(stMain) insertion into one C4Player::Crew list
    /// (C4ObjectList.cpp:110-195). This list is independent of Owner.
    fn crew_insert_position(&self, roster: &[ObjectId], target: ObjectId) -> usize {
        let Some((category, definition_id)) = self.contents_sort_key(target) else {
            return roster.len();
        };
        if self
            .definition_metadata(&definition_id)
            .is_some_and(|metadata| metadata.line != 0)
        {
            return roster.len();
        }
        let sort_category = category & CATEGORY_SORT_LIMIT;
        if category & crate::CATEGORY_STATIC_BACK == 0 {
            if let Some(position) = roster.iter().position(|other| {
                self.contents_sort_key(*other)
                    .is_some_and(|(other_category, other_definition)| {
                        other_category & CATEGORY_SORT_LIMIT == sort_category
                            && other_definition == definition_id
                    })
            }) {
                return position;
            }
        }
        roster
            .iter()
            .position(|other| {
                self.contents_sort_key(*other)
                    .is_some_and(|(other_category, _)| {
                        other_category & CATEGORY_SORT_LIMIT <= sort_category
                    })
            })
            .unwrap_or(roster.len())
    }

    pub(crate) fn insert_player_crew(&mut self, player_id: i32, target: ObjectId) -> bool {
        let Some(roster) = self
            .player_state(player_id)
            .map(|player| player.crew.clone())
        else {
            return false;
        };
        if roster.contains(&target) {
            return true;
        }
        let position = self.crew_insert_position(&roster, target);
        self.player_state_mut(player_id)
            .map(|player| player.crew.insert(position.min(player.crew.len()), target))
            .is_some()
    }

    pub(crate) fn object_in_any_crew(&self, target: ObjectId) -> bool {
        self.world.player_ids().iter().any(|player| {
            self.player_state(*player)
                .is_some_and(|state| state.crew.contains(&target))
        })
    }

    pub(crate) fn record_crew_rosters(&mut self) {
        let rosters = self
            .world
            .player_ids()
            .iter()
            .filter_map(|player| {
                self.player_state(*player)
                    .map(|state| (*player, state.crew.clone()))
            })
            .collect();
        self.record_player_command(PlayerCommand::SetCrewRosters { rosters });
    }

    fn clear_player_object_pointers(&mut self, target: ObjectId) {
        let players = self.player_ids().to_vec();
        for player in players {
            if let Some(state) = self.player_state_mut(player) {
                state.clear_object_pointers(target);
            }
        }
        self.record_player_command(PlayerCommand::ClearObjectPointers { object: target });
    }

    pub(crate) fn record_player_command(&mut self, command: PlayerCommand) {
        self.player_commands.push(command);
    }

    pub(crate) fn record_object_order_command(&mut self, command: ObjectOrderCommand) {
        self.object_order_commands.push(command);
    }

    pub(crate) fn team_home_base_rule(&self) -> bool {
        self.team_home_base_rule
    }

    pub(crate) fn object_context_mut(&mut self) -> Option<&mut ObjectScopeContext> {
        self.object.as_mut()
    }

    pub(crate) fn object_context(&self) -> Option<&ObjectScopeContext> {
        self.object.as_ref()
    }

    fn set_global_call_context(&mut self, enter: bool) {
        if enter {
            self.dormant_scopes.push(self.object.take());
            let script_definition = self.script_definition_context.replace(None);
            self.global_call_contexts.push((
                self.script_object_context.take(),
                self.definition_context.take(),
                script_definition,
            ));
            return;
        }

        let Some((script_object, definition, script_definition)) = self.global_call_contexts.pop()
        else {
            debug_assert!(false, "unbalanced global-call context exit");
            return;
        };
        self.object = self.dormant_scopes.pop().unwrap_or(None);
        self.script_object_context = script_object;
        self.definition_context = definition;
        self.script_definition_context = script_definition;
    }

    pub(crate) fn current_definition_id(&self) -> Option<DefinitionId> {
        // FnGetID prefers cthr->Obj->Def. Definition-commanded effects may
        // still carry a mutation object in `object` while their actual
        // script object is null, so only that case falls through to cthr->Def.
        self.script_object_context
            .and_then(|object| self.object_effective_definition_id(object))
            .or_else(|| {
                // Entering an ordinary helper from a no-object global call
                // gives the nested C4Aul frame its destination definition as
                // cthr->Def. HOST_CONTEXT itself remains no-object, so derive
                // that definition from the active local frame's exact host.
                (self.global_call_contexts.is_empty()
                    && clonk_script::caller_uses_engine_scope() == Some(false))
                .then(clonk_script::caller_host_identity)
                .flatten()
                .and_then(|identity| {
                    self.world
                        .script_for_host_identity(identity)
                        .and_then(|(_, definition, _)| definition)
                })
            })
            .or_else(|| self.definition_context.clone())
            .or_else(|| {
                self.object.as_ref().and_then(|object| {
                    object.definition_id.clone().or_else(|| {
                        self.get_world_object(object.id())
                            .map(|world_object| DefinitionId::from(world_object.definition_id()))
                    })
                })
            })
    }

    /// Exact `cthr->Def` for natives whose fallback is the executing script
    /// host rather than the object's potentially changed live definition.
    pub(crate) fn executing_definition_id(&self) -> Option<Option<DefinitionId>> {
        match &self.script_definition_context {
            Some(definition) => Some(definition.clone()),
            None => {
                if let Some((_, definition, _)) = clonk_script::caller_host_identity()
                    .and_then(|identity| self.world.script_for_host_identity(identity))
                {
                    return Some(definition);
                }
                self.definition_context.clone().map(Some)
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn audio_mut(&mut self) -> &mut AudioRegistry {
        &mut self.audio
    }

    #[allow(dead_code)]
    fn audio(&self) -> &AudioRegistry {
        &self.audio
    }

    pub(crate) fn request_game_over(&mut self) -> bool {
        if self.game_over_triggered {
            return false;
        }
        self.game_over_triggered = true;
        self.trigger_game_over = true;
        true
    }

    fn into_commands(mut self) -> EffectContextOutcome {
        debug_assert!(
            self.dormant_scopes.is_empty(),
            "all nested calls must have finished before the context closes"
        );
        debug_assert!(
            self.global_call_contexts.is_empty(),
            "all global calls must have finished before the context closes"
        );
        // C++ mutates Contents links synchronously. The Rust host preview
        // already has the exact callback-final order because
        // get_world_object replays contents_link_operations in call order.
        // Retain one final list per touched container so copy-out can restore
        // that order after outer/spawn/nested object channels materialize.
        let mut touched_containers = Vec::new();
        for operation in &self.contents_link_operations {
            let container = operation.container();
            if !touched_containers.contains(&container) {
                touched_containers.push(container);
            }
        }
        let contents_orders = touched_containers
            .into_iter()
            .filter_map(|container| {
                self.get_world_object(container)
                    .map(|object| HostContentsOrder {
                        container,
                        contents: object.contents().to_vec(),
                    })
            })
            .collect::<Vec<_>>();
        // Cross-object LocalN cells fold like any other foreign mutation:
        // merged into the target's outcome locals (cells hold the LATEST
        // value, after any nested calls), with cell-only targets getting a
        // locals-only outcome seeded from their current state.
        let mut cell_locals: HashMap<ObjectId, HashMap<String, Value>> = HashMap::new();
        for ((object, name), cell) in &self.foreign_local_cells {
            cell_locals
                .entry(*object)
                .or_default()
                .insert(name.clone(), cell.borrow().clone());
        }
        let mut other_objects = Vec::new();
        for id in mem::take(&mut self.nested_order) {
            let Some(NestedScopeState {
                mut scope,
                mut local_vars,
            }) = self.nested_objects.remove(&id)
            else {
                continue;
            };
            // A successful `func &` call keeps its VM session alive until
            // the suspended caller consumes the returned reference. Fold the
            // final cells here, after that write, rather than the pre-AB_Set
            // snapshot stored when the nested call returned.
            if let Some(cells) = self.session_local_cells.get(&id) {
                local_vars.extend(cells.snapshot());
            }
            if let Some(cells) = cell_locals.remove(&id) {
                local_vars.extend(cells);
            }
            scope.finalize_persisted_ocf();
            let command_operations = scope.final_command_operations();
            let mut update = scope.pending_update;
            // Mirror the outer call's unconditional local-vars store
            // (Definition::call_object_function).
            update.local_vars = Some(local_vars);
            other_objects.push(NestedObjectOutcome {
                object_id: id,
                assign_death: scope.assign_death,
                effects: scope.effects.into_commands(),
                update: Some(update),
                commands: scope.queued_commands,
                command_operations,
                destroy: scope.destroy,
                contents_orders: Vec::new(),
            });
        }
        // Cell-only targets (LocalN writes without any nested call): a
        // locals-only outcome, full map seeded from the current state so
        // the unconditional store does not drop untouched locals. Sorted
        // for a deterministic fold order.
        let mut cell_only: Vec<ObjectId> = cell_locals.keys().copied().collect();
        cell_only.sort_unstable();
        for id in cell_only {
            let Some(cells) = cell_locals.remove(&id) else {
                continue;
            };
            let mut local_vars = self
                .get_world_object(id)
                .and_then(|object| object.full_state().map(|state| state.local_vars.snapshot()))
                .unwrap_or_default();
            local_vars.extend(cells);
            let update = ObjectUpdate {
                local_vars: Some(local_vars),
                ..ObjectUpdate::default()
            };
            other_objects.push(NestedObjectOutcome {
                object_id: id,
                assign_death: None,
                effects: Vec::new(),
                update: Some(update),
                commands: Vec::new(),
                command_operations: Vec::new(),
                destroy: false,
                contents_orders: Vec::new(),
            });
        }
        let (
            object_effects,
            object_update,
            object_commands,
            command_operations,
            destroy,
            active_assign_death,
        ) = match self.object {
            Some(mut object) => {
                let active_assign_death = object.assign_death.map(|forced| (object.id(), forced));
                object.finalize_persisted_ocf();
                let command_operations = object.final_command_operations();
                let update = if object.pending_update.is_empty() {
                    None
                } else {
                    Some(object.pending_update)
                };
                (
                    object.effects.into_commands(),
                    update,
                    object.queued_commands,
                    command_operations,
                    object.destroy,
                    active_assign_death,
                )
            }
            None => (Vec::new(), None, Vec::new(), Vec::new(), false, None),
        };

        // The outer object update has its dedicated channel. Raw list order
        // must fold after every child container pointer: e.g. Eke's retained
        // pistol performs Enter(this(), pistol) and then ShiftContents to
        // select that just-entered C4D_StaticBack object. Applying the outer
        // contents_front first made the rotation miss and left PT5B at the
        // inventory tail.
        if let Some(anchor) = contents_orders.first().map(|order| order.container) {
            other_objects.push(NestedObjectOutcome {
                object_id: anchor,
                assign_death: None,
                effects: Vec::new(),
                update: None,
                commands: Vec::new(),
                command_operations: Vec::new(),
                destroy: false,
                contents_orders,
            });
        }

        // Carry Kill as a same-id nested operation so every existing
        // scenario, definition, effect and object-callback batch applies it
        // only after the object's accumulated writes have folded.
        if let Some((object_id, forced)) = active_assign_death {
            other_objects.push(NestedObjectOutcome {
                object_id,
                assign_death: Some(forced),
                effects: Vec::new(),
                update: None,
                commands: Vec::new(),
                command_operations: Vec::new(),
                destroy: false,
                contents_orders: Vec::new(),
            });
        }

        let global = self
            .global
            .map(EffectScopeContext::into_commands)
            .unwrap_or_default();

        // Spawns of this call whose nested scope was destroyed before
        // materializing (create -> RemoveObject within one call, the
        // GoldRush TRPR temp) never reach the world; their ids stay
        // consumed like C++.
        let destroyed: std::collections::HashSet<ObjectId> = other_objects
            .iter()
            .filter(|outcome| outcome.destroy)
            .map(|outcome| outcome.object_id)
            .collect();
        if !destroyed.is_empty() {
            self.pending_spawns
                .retain(|spawn| spawn.id.is_none_or(|id| !destroyed.contains(&id)));
        }

        let host_raster_preview = (!self.solid_mask_operations.is_empty()).then(|| {
            let (inherit_landscape, landscape) = self.world.host_raster_landscape_preview();
            HostRasterPreview {
                inherit_landscape,
                landscape,
                solid_mask_bakes: self.solid_mask_bakes.as_ref().clone(),
                solid_mask_instance_sequences: self.solid_mask_instance_sequences.borrow().clone(),
                next_solid_mask_instance_sequence: self.next_solid_mask_instance_sequence.get(),
            }
        });
        let audio_events = self.audio.take_events();
        let mut outcome = EffectContextOutcome::new(
            object_effects,
            global,
            object_update,
            object_commands,
            command_operations,
            destroy,
            None,
            None,
            self.pending_spawns,
            self.pending_landscape_ops,
            self.transfer_zone_commands,
            self.pending_messages,
            self.player_commands,
            self.object_order_commands,
            AudioOutcome {
                state: self.audio,
                events: audio_events,
            },
            self.trigger_game_over,
            self.script_go_request,
            self.script_counter_request,
            self.next_object_id,
        );
        outcome.menu_requests = self.pending_menu_requests;
        outcome.command_events = self.pending_command_events;
        outcome.particles = self.pending_particles;
        outcome.next_mission_commands = self.next_mission_commands;
        outcome.other_objects = other_objects;
        outcome.solid_mask_operations = self.solid_mask_operations;
        outcome.host_raster_preview = host_raster_preview;
        outcome
    }
}

pub(crate) struct EffectScopeContext {
    pub(crate) effects: Vec<EffectState>,
    pub(crate) commands: Vec<EffectCommand>,
    /// C++ keeps dead effect nodes linked until the next Execute cleanup.
    /// Once a list head existed in this VM call, a later CheckEffect reaches
    /// C4Effect::Check and returns integer zero even if removals left no live
    /// entries; a truly null head returns nil in FnCheckEffect.
    had_list_head: bool,
}

impl EffectScopeContext {
    fn new(effects: Vec<EffectState>) -> Self {
        let had_list_head = !effects.is_empty();
        Self {
            effects,
            commands: Vec::new(),
            had_list_head,
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<EffectState> {
        self.effects.clone()
    }

    fn clear_object_references(&mut self, sweep: &mut clonk_script::ObjectReferenceSweep) {
        let mut updates = Vec::new();
        for effect in &mut self.effects {
            if effect.clear_object_reference(sweep) {
                updates.push(EffectCommand::update(effect.clone()));
            }
        }
        self.commands.extend(updates);
    }

    // iIntervall/iTime stored verbatim (C4Effect.cpp:66-67).
    fn add_effect(&mut self, mut effect: EffectState) -> i32 {
        if effect.timer < 0 {
            effect.timer = 0;
        }

        // C4Effect::New: same-name effects coexist; the number is the
        // per-object max + 1 (C4Effect.cpp:76-78) and is the script-side
        // handle AddEffect returns.
        effect.number = self
            .effects
            .iter()
            .map(|existing| existing.number)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);

        let mut insert_pos = 0;
        while insert_pos < self.effects.len()
            && self.effects[insert_pos].priority.abs() < effect.priority.abs()
        {
            insert_pos += 1;
        }

        self.effects.insert(insert_pos, effect.clone());
        self.had_list_head = true;
        self.commands.push(EffectCommand::add(effect.clone()));
        effect.number
    }

    /// Links the constructor's not-yet-valid node before C4Effect::Check.
    /// The host copy sees priority zero while callbacks run, but the queued
    /// Add carries the requested priority so the final fold preserves the
    /// C++ insertion position. A later Update validates or removes it.
    pub(crate) fn reserve_effect(
        &mut self,
        mut effect: EffectState,
        constructor_values: [Value; 4],
    ) -> i32 {
        if effect.timer < 0 {
            effect.timer = 0;
        }
        effect.number = self
            .effects
            .iter()
            .map(|existing| existing.number)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);

        let requested_priority = effect.priority;
        let mut insert_pos = 0;
        while insert_pos < self.effects.len()
            && self.effects[insert_pos].priority.abs() < requested_priority.abs()
        {
            insert_pos += 1;
        }

        let mut pending = effect.clone();
        pending.priority = 0;
        self.effects.insert(insert_pos, pending);
        self.had_list_head = true;
        self.commands
            .push(EffectCommand::add_with_constructor_values(
                effect.clone(),
                constructor_values,
            ));
        effect.number
    }

    pub(crate) fn validate_reserved_effect(&mut self, number: i32, priority: i32) -> bool {
        let Some(effect) = self
            .effects
            .iter_mut()
            .find(|effect| effect.number == number)
        else {
            return false;
        };
        effect.priority = priority;
        true
    }

    /// Keeps the dead node linked so later AddEffect calls cannot reuse its
    /// number before the list's next Execute cleanup. C++ leaves a
    /// start-denied constructor node at priority zero too.
    pub(crate) fn discard_reserved_effect(&mut self, number: i32) -> bool {
        let Some(effect) = self
            .effects
            .iter_mut()
            .find(|effect| effect.number == number)
        else {
            return false;
        };
        effect.priority = 0;
        self.commands.push(EffectCommand::update(effect.clone()));
        true
    }

    /// Constructor exception unwind unlinks the pending node immediately.
    pub(crate) fn abort_reserved_effect(&mut self, number: i32) -> bool {
        let Some(position) = self
            .effects
            .iter()
            .position(|effect| effect.number == number)
        else {
            return false;
        };
        self.effects.remove(position);
        self.commands.push(EffectCommand::unlink_number(number));
        true
    }

    pub(crate) fn effect_var(
        &mut self,
        effect_number: usize,
        var_index: usize,
        new_value: Option<EffectVarValue>,
    ) -> Option<EffectVarValue> {
        if effect_number == 0 {
            return None;
        }
        let index = self
            .effects
            .iter()
            .position(|effect| effect.number == effect_number as i32)?;
        let effect = &mut self.effects[index];
        if let Some(value) = new_value {
            effect.set_var(var_index, value);
            let updated = effect.clone();
            self.commands.push(EffectCommand::update(updated));
        }
        Some(effect.var(var_index))
    }

    pub(crate) fn change_effect(
        &mut self,
        name_filter: Option<&str>,
        index: i32,
        new_name: String,
        new_timer: i32,
    ) -> bool {
        let position = if let Some(name) = name_filter {
            if index < 0 {
                None
            } else {
                let mut remaining = index;
                self.effects.iter().position(|effect| {
                    if effect.priority != 0 && s_wildcard_match_ex(&effect.name, name) {
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
            }
        } else {
            self.effects
                .iter()
                .position(|effect| effect.number == index && effect.priority != 0)
        };
        let Some(position) = position else {
            return false;
        };

        let effect = &mut self.effects[position];
        effect.name = new_name;
        if new_timer >= 0 {
            effect.interval = new_timer;
            effect.timer = 0;
        }
        let updated = effect.clone();
        self.commands.push(EffectCommand::update(updated));
        true
    }

    pub(crate) fn remove_effect(
        &mut self,
        name_filter: Option<&str>,
        index: i32,
        no_callbacks: bool,
    ) -> Option<EffectState> {
        self.remove_effect_with_dead(name_filter, index, no_callbacks, true)
    }

    pub(crate) fn remove_live_effect(
        &mut self,
        name_filter: Option<&str>,
        index: i32,
        no_callbacks: bool,
    ) -> Option<EffectState> {
        self.remove_effect_with_dead(name_filter, index, no_callbacks, false)
    }

    pub(crate) fn find_live_effect(
        &self,
        name_filter: Option<&str>,
        index: i32,
    ) -> Option<EffectState> {
        self.effect_position(name_filter, index, false)
            .map(|position| self.effects[position].clone())
    }

    fn effect_position(
        &self,
        name_filter: Option<&str>,
        index: i32,
        include_dead: bool,
    ) -> Option<usize> {
        if let Some(name) = name_filter {
            if index < 0 {
                return None;
            }
            let mut remaining = index;
            self.effects.iter().position(|effect| {
                // FnRemoveEffect resolves named removals through the
                // wildcard-aware C4Effect::Get (C4Script.cpp:5494).
                if (include_dead || effect.priority != 0) && s_wildcard_match_ex(&effect.name, name)
                {
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
            // No name: iIndex is the effect NUMBER (FnRemoveEffect,
            // C4Script.cpp:5502-5507 -> C4Effect::Get(iNumber, false),
            // C4Effect.cpp:240-256). Numbers start at 1, so 0 matches
            // nothing.
            (index > 0)
                .then(|| {
                    self.effects.iter().position(|effect| {
                        effect.number == index && (include_dead || effect.priority != 0)
                    })
                })
                .flatten()
        }
    }

    fn remove_effect_with_dead(
        &mut self,
        name_filter: Option<&str>,
        index: i32,
        no_callbacks: bool,
        include_dead: bool,
    ) -> Option<EffectState> {
        let position = self.effect_position(name_filter, index, include_dead)?;

        // SetDead only clears iPriority. The node, its number, and its vars
        // remain addressable through include-dead lookups until the next
        // C4Effect::Execute walk unlinks it (C4Effect.cpp:326-336,389).
        let effect = &mut self.effects[position];
        effect.priority = 0;
        let effect = effect.clone();
        // The name/wildcard and index have already resolved one concrete
        // C4Effect node. Preserve that identity through the deferred fold:
        // same-name peers are legal and a name-keyed command could mark the
        // wrong peer dead or dispatch Stop with its number.
        self.commands
            .push(EffectCommand::remove_number(effect.number, no_callbacks));
        Some(effect)
    }

    pub(crate) fn unlink_effect_by_number(&mut self, number: i32) -> bool {
        let Some(position) = self
            .effects
            .iter()
            .position(|effect| effect.number == number)
        else {
            return false;
        };
        self.effects.remove(position);
        self.commands.push(EffectCommand::unlink_number(number));
        true
    }

    /// `C4Effect::ClearPointers` (C4Effect.cpp): losing the live object
    /// command target silently marks the effect dead and clears only that
    /// pointer. The node (and its number/id command target) stays linked
    /// until the list's next Execute pass.
    fn clear_command_target(&mut self, target: i32) {
        let mut updates = Vec::new();
        for effect in &mut self.effects {
            if effect.command_target == Some(target) {
                effect.priority = 0;
                effect.command_target = None;
                updates.push(EffectCommand::update(effect.clone()));
            }
        }
        self.commands.extend(updates);
    }

    fn into_commands(self) -> Vec<EffectCommand> {
        self.commands
    }
}

#[derive(Clone)]
pub(crate) enum PhysicalResolution {
    Ready(PhysicalInfo),
    FairCrew {
        definition: PhysicalInfo,
        strength: i32,
        rank_base: i32,
        definition_id: DefinitionId,
        script: Option<Arc<ScriptEngine>>,
        cache: crate::FairCrewPhysicalCache,
    },
}

impl PhysicalResolution {
    /// Resolve only after releasing the `HOST_CONTEXT` RefCell borrow. A
    /// first fair-crew miss synchronously enters the definition script and
    /// its host natives must be able to borrow that same context again.
    pub(crate) fn resolve(self) -> PhysicalInfo {
        match self {
            Self::Ready(physical) => physical,
            Self::FairCrew {
                definition,
                strength,
                rank_base,
                definition_id,
                script,
                cache,
            } => match script {
                Some(script) => crate::fair_crew_physical_with_script(
                    definition,
                    strength,
                    rank_base,
                    &definition_id,
                    script.as_ref(),
                    &cache,
                ),
                None => crate::fair_crew_physical_cached(
                    definition,
                    strength,
                    rank_base,
                    &definition_id,
                    &cache,
                ),
            },
        }
    }

    pub(crate) fn needs_fair_crew_fill(&self) -> bool {
        match self {
            Self::Ready(_) => false,
            Self::FairCrew {
                definition_id,
                cache,
                ..
            } => !cache.borrow().contains_key(definition_id),
        }
    }
}

pub(crate) enum ResetPhysicalBegin {
    Complete(bool),
    ComparePermanent,
}

pub(crate) struct ObjectScopeContext {
    id: ObjectId,
    pub(crate) definition_id: Option<String>,
    /// Live C4Object::Unsorted flag. ChangeDef sets it immediately and it
    /// changes every subsequent C4ObjectList::Add position in this call.
    pub(crate) unsorted: bool,
    pub(crate) current_container: Option<ObjectId>,
    /// The independently serialized nInfo/C4EnumeratedObjectPtr cache words.
    /// These cannot be reconstructed from the live pointer fields.
    pub(crate) current_compiler_cache: crate::ObjectCompilerCache,
    /// Parents whose concrete contents link was removed in this VM call.
    /// Re-entering one of them must allocate a fresh link even if the final
    /// `Contained` value equals the frame-start value.
    pub(crate) removed_contents_links: HashSet<ObjectId>,
    pub(crate) current_contents_link_generation: u64,
    /// Between Exit's raw contents unlink and its post-BoundsCheck tail,
    /// `Contained` is already null but the object's cached OCF and menu are
    /// deliberately still stale (C4Object.cpp:1549-1557).
    pub(crate) exit_bounds_in_progress: bool,
    pub(crate) status: ObjectStatus,
    pub(crate) effects: EffectScopeContext,
    pub(crate) pending_update: ObjectUpdate,
    pub(crate) queued_commands: Vec<QueuedCommand>,
    pub(crate) command_count: usize,
    pub(crate) command_operations: Vec<CommandOperation>,
    pub(crate) live_commands: CommandStack,
    pub(crate) command_stack_replaced: bool,
    pub(crate) destroy: bool,
    /// Script Kill request carried separately from Alive: AssignDeath owns
    /// effect revival, action, inventory and Death-callback semantics.
    assign_death: Option<bool>,
    pub(crate) action_library: SharedActionLibrary,
    current_action_name: String,
    current_action_index: Option<u32>,
    current_action_blocks_other_actions: bool,
    current_action_target: Option<ObjectId>,
    current_action_target2: Option<ObjectId>,
    current_action_data: i32,
    /// Legacy-named storage for C4Action::Time, distinct from PhaseDelay
    /// (`ActionUpdate::ticks`).
    pub(crate) current_action_ticks: i32,
    current_action_phase: i32,
    current_energy: i32,
    /// C4Object::Breath on the raw physical scale.
    current_breath: i32,
    current_need_energy: bool,
    /// C4Object::MagicEnergy (C4Object.h:139), MagicPhysicalFactor scale.
    current_magic_energy: i32,
    current_damage: i32,
    current_construction: i32,
    /// Live C4Shape::ContactDensity, independently mutable from the def.
    current_contact_density: i32,
    current_alive: bool,
    /// Raw `C4Object::Alive` assignment whose matching `SetOCF` has not run
    /// yet. AssignDeath writes Alive directly before RemoveDeath and again
    /// after SetAction; unlike script SetAlive those writes deliberately
    /// leave the cached OCF stale until AssignDeath's final SetOCF.
    raw_alive_override: Option<bool>,
    /// C4Object::Mobile, including explicit native helper overrides.
    current_mobile: bool,
    /// This frame's live C4Action::t_attach bitset.
    current_t_attach: u32,
    pub(crate) current_in_liquid: bool,
    /// Live C4Object::NoCollectDelay for same-call ordered reads/writes.
    current_no_collect_delay: i32,
    /// Same-call transition into C4Object::fOwnVertices via SetVertex's
    /// nonzero own-vertex mode.
    pub(crate) staged_own_vertices: bool,
    current_own_mass: i32,
    current_owner: i32,
    /// C4Object::Select, staged synchronously so nested calls and later host
    /// reads in the same script call observe the write.
    current_selected: bool,
    /// C4Object::Controller (C4Object.h:127) — cause tracing.
    current_controller: i32,
    pub(crate) current_category: i32,
    ocf_base: u32,
    /// The object's CACHED OCF at call entry — FnGetOCF returns pObj->OCF
    /// verbatim (C4Script.cpp:1354-1358). None for bare fixture scopes,
    /// which fall back to the preview-grade recompute.
    pub(crate) cached_ocf: Option<u32>,
    /// ObjectComDrop needs the final live cache after ExecuteCommand's
    /// ControlCommandFinished callback and all later same-call statements.
    pub(crate) persist_final_ocf: bool,
    crew_member: bool,
    current_plr_view_range: i32,
    pub(crate) current_direction: Direction,
    current_command_direction: CommandDirection,
    pub(crate) current_position: Vector2,
    pub(crate) current_fixed_position: FixedVec2,
    /// Sub-pixel velocity in 16.16 fixed-point. Precision-aware velocity
    /// surfaces (`SetXDir`/`GetXDir`) read and write this directly so that
    /// fractional `C4Fixed` velocity survives the script boundary. Seeded from
    /// the whole-pixel velocity at scope entry (full sub-pixel fidelity on
    /// entry awaits the snapshot work, task B).
    pub(crate) current_fixed_velocity: FixedVec2,
    pub(crate) current_rotation: i32,
    /// Live raw `C4Object::fix_r`, independently reflected from Rotation.
    pub(crate) current_fixed_rotation: C4Fixed,
    /// Live `C4Object::rdir`, including writes earlier in this VM call.
    pub(crate) current_rotation_velocity: C4Fixed,
    shape_vertices: ShapeVertexBuffer,
    graphics_overlays: Vec<ObjectGraphicsOverlay>,
    pub(crate) base_graphics: Option<ObjectBaseGraphics>,
    current_draw_transform: Option<DrawTransform>,
    pub(crate) info_physical: Option<PhysicalInfo>,
    /// Fair-crew selection is live round state; the source definition is
    /// `Info->pDef`, which deliberately survives ChangeDef.
    use_fair_crew: bool,
    fair_crew_strength: i32,
    fair_crew_physical_cache: crate::FairCrewPhysicalCache,
    pub(crate) info_definition_physical: Option<PhysicalInfo>,
    info_definition_id: Option<DefinitionId>,
    info_definition_rank_base: i32,
    info_definition_script: Option<Arc<ScriptEngine>>,
    /// Live pObj->Info->Rank. Unlike `crew_member`, this distinguishes an
    /// object registered as crew from one that actually owns C4ObjectInfo.
    current_info_rank: Option<i32>,
    /// C4Player::CrewInfoList that owns the live Info pointer.
    current_info_link: Option<CrewInfoLink>,
    /// Full live C4ObjectInfoCore payload. Kept in the scope so a nested
    /// GrabObjectInfo can move an info created earlier in the same callback.
    current_info_core: Option<CrewObjectInfo>,
    pub(crate) temporary_physical: Option<PhysicalInfo>,
    physical_changes: Vec<(String, i32)>,
    pub(crate) definition_physical: PhysicalInfo,
    /// FnAdjustWalkRotation seam — see [`WalkRotationSeed`].
    pub(crate) walk_rotation: WalkRotationSeed,
}

impl ObjectScopeContext {
    fn new(
        id: ObjectId,
        container: Option<ObjectId>,
        status: ObjectStatus,
        energy: i32,
        damage: i32,
        construction: i32,
        alive: bool,
        in_liquid: bool,
        own_mass: i32,
        owner: i32,
        controller: i32,
        category: i32,
        position: Vector2,
        velocity: Vector2,
        rotation: i32,
        effects: Vec<EffectState>,
        action_library: SharedActionLibrary,
        action_name: String,
        action_index: Option<u32>,
        action_ticks: i32,
        action_data: i32,
        action_phase: i32,
        direction: Direction,
        command_direction: CommandDirection,
        command_count: usize,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        shape_vertices: ShapeVertexBuffer,
        ocf_base: u32,
        crew_member: bool,
        plr_view_range: i32,
        graphics_overlays: Vec<ObjectGraphicsOverlay>,
        base_graphics: Option<ObjectBaseGraphics>,
        draw_transform: Option<DrawTransform>,
        info_physical: Option<PhysicalInfo>,
        temporary_physical: Option<PhysicalInfo>,
        physical_changes: Vec<(String, i32)>,
        definition_physical: PhysicalInfo,
    ) -> Self {
        let blocks_other_actions =
            action_library.blocks_other_actions_for_entry(&action_name, action_index);
        let clamped_damage = damage.max(0);
        let clamped_construction = construction.max(0);
        Self {
            definition_id: None,
            id,
            unsorted: false,
            current_container: container,
            current_compiler_cache: crate::ObjectCompilerCache::default(),
            removed_contents_links: HashSet::new(),
            current_contents_link_generation: 0,
            exit_bounds_in_progress: false,
            status,
            effects: EffectScopeContext::new(effects),
            pending_update: ObjectUpdate::default(),
            queued_commands: Vec::new(),
            command_count,
            command_operations: Vec::new(),
            live_commands: CommandStack::new(),
            command_stack_replaced: false,
            destroy: false,
            assign_death: None,
            action_library,
            current_action_name: action_name,
            current_action_index: action_index,
            current_action_blocks_other_actions: blocks_other_actions,
            current_action_target: action_target,
            current_action_target2: action_target2,
            current_action_data: action_data,
            current_action_ticks: action_ticks,
            current_action_phase: action_phase,
            current_energy: energy,
            current_breath: 0,
            current_need_energy: false,
            current_magic_energy: 0,
            current_damage: clamped_damage,
            current_construction: clamped_construction,
            current_contact_density: crate::CONTACT_DENSITY_SOLID,
            current_alive: alive,
            raw_alive_override: None,
            current_mobile: false,
            current_t_attach: 0,
            current_in_liquid: in_liquid,
            current_no_collect_delay: 0,
            staged_own_vertices: false,
            current_own_mass: own_mass,
            current_owner: owner,
            current_selected: false,
            current_controller: controller,
            current_category: category,
            ocf_base,
            cached_ocf: None,
            persist_final_ocf: false,
            crew_member,
            current_plr_view_range: plr_view_range,
            current_direction: direction,
            current_command_direction: command_direction,
            current_position: position,
            current_fixed_position: FixedVec2::from_ints(position.x, position.y),
            current_fixed_velocity: FixedVec2::from_ints(velocity.x, velocity.y),
            // Seed the RAW engine r: the movement circle bounds keep it
            // within (-180, 180] and FnGetR reads it unnormalized; only
            // SetR normalizes (C4Object::SetRotation, C4Object.cpp:5632).
            current_rotation: rotation,
            current_fixed_rotation: itofix(rotation),
            current_rotation_velocity: C4Fixed::ZERO,
            shape_vertices,
            graphics_overlays,
            base_graphics,
            current_draw_transform: draw_transform,
            info_physical,
            use_fair_crew: false,
            fair_crew_strength: 1_000,
            fair_crew_physical_cache: Rc::new(RefCell::new(HashMap::new())),
            info_definition_physical: None,
            info_definition_id: None,
            info_definition_rank_base: 1_000,
            info_definition_script: None,
            current_info_rank: None,
            current_info_link: None,
            current_info_core: None,
            temporary_physical,
            physical_changes,
            definition_physical,
            walk_rotation: WalkRotationSeed::default(),
        }
    }

    /// Install the callback-visible half of C4Object::ChangeDef after the
    /// old-definition SetAction(ActIdle) phase. Runtime Category and the
    /// live ContactDensity are object fields and intentionally survive.
    pub(crate) fn install_definition_preview(
        &mut self,
        definition_id: &str,
        metadata: &DefinitionMetadata,
    ) {
        self.pending_update.change_def = Some(definition_id.to_string());
        self.definition_id = Some(definition_id.to_string());
        self.unsorted = true;
        self.action_library = metadata.action_library.clone();
        self.current_action_blocks_other_actions = self
            .action_library
            .blocks_other_actions_for_entry(&self.current_action_name, self.current_action_index);
        self.definition_physical = metadata.physical;
        self.ocf_base = metadata.ocf_base;
        self.crew_member = metadata.crew_member;
        self.walk_rotation.rotateable = metadata.rotateable;

        if metadata.rotateable == 0 {
            self.current_rotation = 0;
            self.current_fixed_rotation = C4Fixed::ZERO;
            self.current_rotation_velocity = C4Fixed::ZERO;
            self.pending_update.rotation = Some(0);
            self.pending_update.rotation_velocity = Some(C4Fixed::ZERO);
        }

        if metadata.line == 0 {
            self.pending_update.shape_override = Some(None);
        }
        self.refresh_shape_preview(metadata);

        // SetOCF runs after the definition swap. Drop the old cached mask so
        // same-call GetOCF/RejectEntrance code derives from the new Def.
        self.cached_ocf = None;
        let mask = self.staged_ocf(self.ocf());
        self.cached_ocf = Some(mask);
    }

    pub(crate) fn refresh_shape_preview(&mut self, metadata: &DefinitionMetadata) {
        self.refresh_shape_preview_from_parts(
            &metadata.vertices,
            metadata.line,
            metadata.stretch_growth,
            metadata.rotateable,
        );
    }

    pub(crate) fn refresh_shape_preview_from_parts(
        &mut self,
        definition_vertices: &[ObjectVertex],
        line: i32,
        stretch_growth: bool,
        rotateable: i32,
    ) {
        // C4Object::UpdateShape returns immediately for line defs. Ordinary
        // definitions copy the current definition shape while fOwnVertices
        // restores the object's private backup.
        if line == 0 {
            let replaces_staged_vertex_edit = self.pending_update.live_vertices.is_some()
                || self.pending_update.shape_vertices.is_some();
            // A same-call SetVertex has already staged the backup half, so
            // restore from the effective buffer rather than the committed one.
            let mut buffer = self.shape_vertex_buffer();
            let base = if self.staged_own_vertices {
                buffer.own_original_vertices()
            } else {
                definition_vertices.to_vec()
            };
            let vertices = crate::transformed_shape_vertices(
                &base,
                self.construction(),
                stretch_growth,
                rotateable,
                self.rotation(),
            );
            buffer.replace_active(&vertices);
            self.shape_vertices = buffer;
            if replaces_staged_vertex_edit {
                let vertices = self.shape_vertices.clone();
                self.pending_update.live_vertices = Some(vertices.active_vec());
                self.pending_update.shape_vertices = Some(vertices);
            }
        }
    }

    pub(crate) fn id(&self) -> ObjectId {
        self.id
    }

    /// Record the full physical state into the pending update (applied
    /// wholesale by the engine — a cleared temp mode must overwrite).
    pub(crate) fn record_physicals(&mut self) {
        self.pending_update.physicals = Some(PhysicalsUpdate {
            info: self.info_physical,
            temporary: self.temporary_physical,
            changes: self.physical_changes.clone(),
        });
    }

    pub(crate) fn info_rank(&self) -> Option<i32> {
        self.current_info_rank
    }

    pub(crate) fn set_info_rank(&mut self, rank: Option<i32>) {
        self.current_info_rank = rank;
        match (self.current_info_core.as_mut(), rank) {
            (Some(info), Some(rank)) => info.rank = rank,
            (_, None) => self.current_info_core = None,
            (None, Some(_)) => {}
        }
        self.pending_update.info_rank = Some(rank);
    }

    pub(crate) fn info_link(&self) -> Option<CrewInfoLink> {
        self.current_info_link
    }

    pub(crate) fn set_info_link(&mut self, link: Option<CrewInfoLink>) {
        self.current_info_link = link;
        self.pending_update.info_link = Some(link);
    }

    pub(crate) fn info_core(&self) -> Option<&CrewObjectInfo> {
        self.current_info_core.as_ref()
    }

    pub(crate) fn set_info_core(&mut self, info: Option<CrewObjectInfo>) {
        self.current_info_core = info;
    }

    pub(crate) fn configure_fair_crew(&mut self, world: &HostWorldContext) {
        self.use_fair_crew = world.use_fair_crew;
        self.fair_crew_strength = world.fair_crew_strength;
        self.fair_crew_physical_cache = Rc::clone(&world.fair_crew_physical_cache);
        let retained_definition_id = self
            .current_info_core
            .as_ref()
            .map(|info| info.definition_id.clone());
        self.info_definition_id = retained_definition_id
            .filter(|id| world.definition_metadata(id.as_str()).is_some())
            .or_else(|| self.definition_id.as_deref().map(DefinitionId::from));
        self.info_definition_physical = self
            .info_definition_id
            .as_ref()
            .and_then(|id| world.definition_metadata(id.as_str()))
            .map(|metadata| metadata.physical)
            .or(Some(self.definition_physical));
        self.info_definition_rank_base = self
            .info_definition_id
            .as_ref()
            .and_then(|id| world.definition_rank_base(id.as_str()))
            .unwrap_or(1_000);
        self.info_definition_script = self
            .info_definition_id
            .as_ref()
            .and_then(|id| world.definition_script(id.as_str()).cloned());
    }

    /// Real engine contexts carry the full info core. Legacy host fixtures
    /// predate it and use an info-physical/rank projection while fair crew is
    /// disabled; keep that compatibility without treating it as Info in the
    /// live fair-crew branch.
    pub(crate) fn has_physical_info(&self) -> bool {
        self.current_info_core.is_some()
            || (!self.use_fair_crew && self.current_info_rank.is_some())
    }

    /// `C4Object::GetPhysical` (C4Object.cpp:2118-2134): temporary set when
    /// active (unless `permanent`), then the actual Info branch using live
    /// fair-crew parameters, else the current definition.
    pub(crate) fn prepare_resolved_physical(&self, permanent: bool) -> PhysicalResolution {
        let temporary = (!permanent).then_some(self.temporary_physical).flatten();
        if let Some(temporary) = temporary {
            return PhysicalResolution::Ready(temporary);
        }
        if self.current_info_core.is_some() {
            let info_definition = self
                .info_definition_physical
                .unwrap_or(self.definition_physical);
            if self.use_fair_crew {
                if let Some(id) = self.info_definition_id.as_ref() {
                    return PhysicalResolution::FairCrew {
                        definition: info_definition,
                        strength: self.fair_crew_strength,
                        rank_base: self.info_definition_rank_base,
                        definition_id: id.clone(),
                        script: self.info_definition_script.clone(),
                        cache: Rc::clone(&self.fair_crew_physical_cache),
                    };
                }
                return PhysicalResolution::Ready(crate::fair_crew_physical(
                    info_definition,
                    self.fair_crew_strength,
                    self.info_definition_rank_base,
                ));
            }
            return PhysicalResolution::Ready(self.info_physical.unwrap_or(info_definition));
        }
        if self.has_physical_info() {
            return PhysicalResolution::Ready(
                self.info_physical.unwrap_or(self.definition_physical),
            );
        }
        PhysicalResolution::Ready(self.definition_physical)
    }

    /// `FnGetPhysical` mode dispatch (C4Script.cpp:638-688).
    pub(crate) fn prepare_get_physical(&self, mode: i32) -> Option<PhysicalResolution> {
        match mode {
            PHYS_CURRENT => Some(self.prepare_resolved_physical(false)),
            PHYS_PERMANENT => {
                // Info objects only (C4Script.cpp:668).
                if !self.has_physical_info() {
                    return None;
                }
                Some(self.prepare_resolved_physical(true))
            }
            PHYS_TEMPORARY => {
                // Info objects only, and only in temporary mode
                // (C4Script.cpp:680-682).
                if !self.has_physical_info() {
                    return None;
                }
                self.temporary_physical.map(PhysicalResolution::Ready)
            }
            _ => None,
        }
    }

    /// `FnSetPhysical` mode dispatch (C4Script.cpp:557-601).
    pub(crate) fn set_physical(
        &mut self,
        name: &str,
        value: i32,
        mode: i32,
        resolved_base: Option<PhysicalInfo>,
    ) -> bool {
        // Unknown names fail (C4Script.cpp:562).
        if PhysicalInfo::default().value_mut_by_name(name).is_none() {
            return false;
        }
        match mode {
            PHYS_CURRENT => {
                // Temporary mode or info objects only (C4Script.cpp:569).
                if let Some(temporary) = self.temporary_physical.as_mut() {
                    temporary.set_by_name(name, value);
                } else if self.has_physical_info() && !self.use_fair_crew {
                    let definition_physical = self.definition_physical;
                    self.info_physical
                        .get_or_insert(definition_physical)
                        .set_by_name(name, value);
                } else {
                    return false;
                }
                self.record_physicals();
                true
            }
            PHYS_PERMANENT => {
                // Info objects only (C4Script.cpp:576).
                if !self.has_physical_info() || self.use_fair_crew {
                    return false;
                }
                let definition_physical = self.definition_physical;
                self.info_physical
                    .get_or_insert(definition_physical)
                    .set_by_name(name, value);
                self.record_physicals();
                true
            }
            PHYS_TEMPORARY | PHYS_STACK_TEMPORARY => {
                // Auto-switch to temporary mode (C4Script.cpp:587-591).
                // `resolved_base` also records that the outer call observed
                // PhysicalTemporary=false. C++ unconditionally copies that
                // captured `GetPhysical()` result afterward, even when a
                // nested fair-crew hook enabled temporary mode meanwhile.
                if let Some(base) = resolved_base {
                    self.temporary_physical = Some(base);
                } else if self.temporary_physical.is_none() {
                    return false;
                }
                let temporary = self
                    .temporary_physical
                    .as_mut()
                    .expect("temporary physical was initialized above");
                // PHYS_StackTemporary remembers the old value
                // (C4Script.cpp:593-594; C4InfoCore.cpp:333-337).
                if mode == PHYS_STACK_TEMPORARY {
                    if let Some(previous) = temporary.value_by_name(name) {
                        self.physical_changes.push((name.to_string(), previous));
                    }
                }
                self.temporary_physical
                    .as_mut()
                    .map(|physical| physical.set_by_name(name, value));
                self.record_physicals();
                true
            }
            _ => false,
        }
    }

    /// `C4Object::TrainPhysical` (C4Object.cpp:2136-2146) over the scope
    /// copies; trains stacked previous values too (C4InfoCore.cpp:309-317).
    pub(crate) fn train_physical(&mut self, name: &str, train_by: i32, max_train: i32) -> bool {
        if PhysicalInfo::default().value_mut_by_name(name).is_none() {
            return false;
        }
        let mut trained = false;
        if let Some(temporary) = self.temporary_physical.as_mut() {
            if let Some(value) = temporary.value_mut_by_name(name) {
                PhysicalInfo::train_value(value, train_by, max_train);
            }
            for (_, previous) in self
                .physical_changes
                .iter_mut()
                .filter(|(changed, _)| changed.eq_ignore_ascii_case(name))
            {
                PhysicalInfo::train_value(previous, train_by, max_train);
            }
            trained = true;
        }
        if self.has_physical_info() {
            let definition_physical = self
                .info_definition_physical
                .unwrap_or(self.definition_physical);
            let info = self.info_physical.get_or_insert(definition_physical);
            if let Some(value) = info.value_mut_by_name(name) {
                PhysicalInfo::train_value(value, train_by, max_train);
            }
            trained = true;
        }
        if trained {
            self.record_physicals();
        }
        trained
    }

    /// `FnResetPhysical` (C4Script.cpp:613-636).
    pub(crate) fn begin_reset_physical(&mut self, name: Option<&str>) -> ResetPhysicalBegin {
        // Only in temporary mode (C4Script.cpp:619).
        if self.temporary_physical.is_none() {
            return ResetPhysicalBegin::Complete(false);
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            if PhysicalInfo::default().value_mut_by_name(name).is_none() {
                return ResetPhysicalBegin::Complete(false);
            }
            // Undo the last registered change for this physical
            // (C4InfoCore.cpp:339-351).
            let Some(position) = self
                .physical_changes
                .iter()
                .rposition(|(changed, _)| changed.eq_ignore_ascii_case(name))
            else {
                return ResetPhysicalBegin::Complete(false);
            };
            let (_, previous) = self.physical_changes.remove(position);
            self.temporary_physical
                .as_mut()
                .map(|physical| physical.set_by_name(name, previous));
            return ResetPhysicalBegin::ComparePermanent;
        }
        // Full reset (C4Script.cpp:631-635).
        self.temporary_physical = None;
        self.physical_changes.clear();
        self.record_physicals();
        ResetPhysicalBegin::Complete(true)
    }

    pub(crate) fn finish_reset_physical(&mut self, reference: PhysicalInfo) -> bool {
        // Keep temporary mode while other changes remain or the set still
        // deviates from the reference (C4Script.cpp:628;
        // C4InfoCore.cpp:319-331). The callback may itself have changed the
        // target while its permanent physical was being resolved, so inspect
        // the live scope again here.
        let deviates = self
            .temporary_physical
            .map(|physical| physical != reference)
            .unwrap_or(false);
        if !self.physical_changes.is_empty() || deviates {
            self.record_physicals();
            return true;
        }
        self.temporary_physical = None;
        self.physical_changes.clear();
        self.record_physicals();
        true
    }

    pub(crate) fn status(&self) -> ObjectStatus {
        if self.destroy {
            ObjectStatus::Deleted
        } else {
            self.pending_update.status.unwrap_or(self.status)
        }
    }

    pub(crate) fn set_status(&mut self, status: ObjectStatus) {
        self.status = status;
        self.pending_update.status = Some(status);
    }

    pub(crate) fn owner(&self) -> i32 {
        self.pending_update.owner.unwrap_or(self.current_owner)
    }

    pub(crate) fn selected(&self) -> bool {
        self.pending_update
            .selected
            .unwrap_or(self.current_selected)
    }

    pub(crate) fn set_selected(&mut self, selected: bool) {
        self.current_selected = selected;
        self.pending_update.selected = Some(selected);
    }

    pub(crate) fn set_owner(&mut self, owner: i32) {
        self.current_owner = owner;
        self.pending_update.owner = Some(owner);
        // C4Object::SetOwner "automatically updates controller"
        // (C4Object.cpp:5499-5500).
        self.set_controller(owner);
    }

    /// C4Object::Controller (FnGetController, C4Script.cpp:1316-1320).
    pub(crate) fn controller(&self) -> i32 {
        self.pending_update
            .controller
            .unwrap_or(self.current_controller)
    }

    /// FnSetController (C4Script.cpp:1322-1331).
    pub(crate) fn set_controller(&mut self, controller: i32) {
        self.current_controller = controller;
        self.pending_update.controller = Some(controller);
    }

    /// C4Player::MakeCrewMember adds the object to the crew
    /// (C4Player.cpp:1195-1196); the port keys crew off Owner +
    /// crew_member.
    pub(crate) fn set_crew_member(&mut self, crew_member: bool) {
        self.crew_member = crew_member;
        self.pending_update.crew_member = Some(crew_member);
    }

    /// AssignDeath changes the player's runtime roster projection, not the
    /// definition's CrewMember capability used by SetOCF. Keep that live
    /// capability intact while transporting the final roster bit.
    pub(crate) fn stage_crew_member_state(&mut self, crew_member: bool) {
        self.pending_update.crew_member = Some(crew_member);
    }

    pub(crate) fn set_crew_status_member(&mut self, crew_member: bool) {
        self.set_crew_member(crew_member);
        self.pending_update.crew_status_change = true;
    }

    pub(crate) fn plr_view_range(&self) -> i32 {
        self.pending_update
            .plr_view_range
            .unwrap_or(self.current_plr_view_range)
    }

    pub(crate) fn set_plr_view_range(&mut self, range: i32) {
        self.current_plr_view_range = range;
        self.pending_update.plr_view_range = Some(range);
    }

    /// FnSetSolidMask bookkeeping (C4Script.cpp:271-278).
    pub(crate) fn set_solid_mask_rect(&mut self, rect: crate::DefinitionTargetRect) {
        self.pending_update.solid_mask_override = Some(rect);
    }

    /// FnSetPicture bookkeeping (C4Script.cpp:3708-3715).
    pub(crate) fn set_picture_rect(&mut self, rect: DefinitionRect) {
        self.pending_update.picture_rect = Some(rect);
    }

    pub(crate) fn alive(&self) -> bool {
        self.raw_alive_override
            .unwrap_or_else(|| self.pending_update.alive.unwrap_or(self.current_alive))
    }

    pub(crate) fn mobile(&self) -> bool {
        self.pending_update.mobile.unwrap_or(self.current_mobile)
    }

    pub(crate) fn set_mobile(&mut self, mobile: bool) {
        self.current_mobile = mobile;
        self.pending_update.mobile = Some(mobile);
    }

    pub(crate) fn t_attach(&self) -> u32 {
        self.pending_update
            .t_attach
            .unwrap_or(self.current_t_attach)
    }

    pub(crate) fn set_t_attach(&mut self, t_attach: u32) {
        self.current_t_attach = t_attach;
        self.walk_rotation.t_attach = t_attach;
        self.pending_update.t_attach = Some(t_attach);
    }

    /// The cached InLiquid flag (scripts cannot set it; only
    /// FnSetPosition re-derives it, C4Script.cpp:475).
    pub(crate) fn in_liquid(&self) -> bool {
        self.current_in_liquid
    }

    pub(crate) fn set_in_liquid(&mut self, in_liquid: bool) {
        self.current_in_liquid = in_liquid;
        self.pending_update.in_liquid = Some(in_liquid);
    }

    pub(crate) fn no_collect_delay(&self) -> i32 {
        self.current_no_collect_delay
    }

    pub(crate) fn set_no_collect_delay(&mut self, delay: i32) {
        self.current_no_collect_delay = delay;
    }

    pub(crate) fn restore_no_collect_delay(&mut self, old_delay: i32) {
        if old_delay <= self.current_no_collect_delay {
            return;
        }
        self.current_no_collect_delay = old_delay;
        // FnCollect's restoration mutates the live field but deliberately
        // does not call UpdateOCF. Persist the field together with the cache
        // left by the temporary recompute/callback chain.
        self.command_operations
            .push(CommandOperation::SetNoCollectDelay {
                value: old_delay,
                ocf: self.ocf(),
            });
    }

    /// Record ObjectComDrop's adjacent NoCollectDelay assignment and
    /// SetOCF result after the live cache has been refreshed.
    pub(crate) fn record_no_collect_delay_assignment(&mut self) {
        let ocf = self.ocf();
        self.command_operations
            .push(CommandOperation::SetNoCollectDelay {
                value: self.current_no_collect_delay,
                ocf,
            });
    }

    /// Copy-out applies command operations before the final OCF override.
    /// Keep the ordered delay assignment's cache payload synchronized with
    /// any later UnGrab/Grab SetOCF calls as well.
    fn update_recorded_no_collect_delay_ocf(&mut self, final_ocf: u32) {
        if let Some(CommandOperation::SetNoCollectDelay { ocf, .. }) = self
            .command_operations
            .iter_mut()
            .rev()
            .find(|operation| matches!(operation, CommandOperation::SetNoCollectDelay { .. }))
        {
            *ocf = final_ocf;
        }
    }

    fn finalize_persisted_ocf(&mut self) {
        if self.pending_update.resolved_docon_position.is_some() {
            self.pending_update.resolved_docon_position = Some(self.effective_position());
            self.pending_update.resolved_docon_fixed_position = Some(self.current_fixed_position);
        }
        if !self.persist_final_ocf {
            return;
        }
        let final_ocf = self.ocf();
        self.update_recorded_no_collect_delay_ocf(final_ocf);
        self.pending_update.ocf_override = Some(final_ocf);
    }

    pub(crate) fn own_mass(&self) -> i32 {
        self.current_own_mass
    }

    /// SetMass (C4Script.cpp:3620-3626): OwnMass = value - Def->Mass.
    pub(crate) fn set_own_mass(&mut self, own_mass: i32) {
        self.current_own_mass = own_mass;
        self.pending_update.own_mass = Some(own_mass);
    }

    pub(crate) fn set_alive(&mut self, alive: bool) {
        self.raw_alive_override = None;
        self.current_alive = alive;
        self.pending_update.alive = Some(alive);
    }

    /// AssignDeath's direct `Alive = 0` assignment. This changes raw
    /// same-call reads without acknowledging the change in cached OCF.
    pub(crate) fn set_raw_alive(&mut self, alive: bool) {
        self.raw_alive_override = Some(alive);
    }

    /// Commit AssignDeath's last raw Alive word immediately before its
    /// explicit final SetOCF. A Death callback that called SetAlive already
    /// cleared the override and therefore keeps its revived value.
    pub(crate) fn commit_raw_alive(&mut self) {
        if let Some(alive) = self.raw_alive_override.take() {
            self.current_alive = alive;
            self.pending_update.alive = Some(alive);
        }
    }

    pub(crate) fn category(&self) -> i32 {
        self.pending_update
            .category
            .unwrap_or(self.current_category)
    }

    pub(crate) fn set_category(&mut self, category: i32) {
        // FnSetCategory preserves the object's current sorting bits only
        // when the requested value has none. Unlike DefCore/load repair it
        // does not invent StaticBack when both masks are zero.
        let category = if category & CATEGORY_SORT_LIMIT == 0 {
            category | (self.category() & CATEGORY_SORT_LIMIT)
        } else {
            category
        };
        self.current_category = category;
        self.pending_update.category = Some(category);
    }

    pub(crate) fn clear_command_stack(&mut self) {
        let preserve_grab_pointer_order = self.live_commands.has_pending_grab_attempt();
        self.live_commands.clear();
        // Callback continuations queued before ClearCommands belong to the
        // cleared C4Command stack. Later same-call command events may append
        // again and therefore survive, preserving the native call order.
        self.queued_commands.clear();
        self.command_operations.push(CommandOperation::Clear);
        self.command_count = 0;
        if preserve_grab_pointer_order {
            self.command_stack_replaced = true;
        }
    }

    /// C4Object::SetCommand's NoCollectDelay entry decrement
    /// (C4Object.cpp:3941-3942), staged in command-op order.
    pub(crate) fn decrement_no_collect_delay(&mut self) {
        if self.current_no_collect_delay > 0 {
            self.current_no_collect_delay -= 1;
        }
        self.command_operations
            .push(CommandOperation::DecrementNoCollectDelay);
    }

    pub(crate) fn push_command_front(&mut self, request: CommandRequest) -> bool {
        if self.command_count >= MAX_COMMAND_STACK {
            return false;
        }
        if self.live_commands.push_front(request.clone()).is_err() {
            return false;
        }
        self.command_operations
            .push(CommandOperation::PushFront(request));
        self.command_count += 1;
        true
    }

    pub(crate) fn push_command_back(&mut self, request: CommandRequest) -> bool {
        if self.command_count >= MAX_COMMAND_STACK {
            return false;
        }
        if self.live_commands.push_back(request.clone()).is_err() {
            return false;
        }
        self.command_operations
            .push(CommandOperation::PushBack(request));
        self.command_count += 1;
        true
    }

    fn final_command_operations(&mut self) -> Vec<CommandOperation> {
        if !self.command_stack_replaced {
            return mem::take(&mut self.command_operations);
        }
        let mut operations = mem::take(&mut self.command_operations)
            .into_iter()
            .filter(|operation| {
                matches!(
                    operation,
                    CommandOperation::DecrementNoCollectDelay
                        | CommandOperation::SetNoCollectDelay { .. }
                )
            })
            .collect::<Vec<_>>();
        operations.push(CommandOperation::Restore(self.live_commands.snapshot()));
        operations
    }

    /// Apply the current-object delta emitted by one C4Command execution
    /// to the same live script scope. Command states only emit this core
    /// field set; cross-object writes travel as CommandEvents.
    pub(crate) fn stage_command_update(&mut self, mut update: ObjectUpdate) {
        if let Some(compiler_cache) = update.compiler_cache.take() {
            self.current_compiler_cache = compiler_cache.clone();
            self.pending_update.compiler_cache = Some(compiler_cache);
        }
        if let Some(position) = update.position.take() {
            self.set_position(position);
        }
        let fixed_velocity = update.fixed_velocity.take();
        if let Some(velocity) = update.velocity.take() {
            self.current_fixed_velocity = FixedVec2::from_ints(velocity.x, velocity.y);
            self.pending_update.velocity = Some(velocity);
        }
        if let Some(velocity) = fixed_velocity {
            self.set_fixed_velocity(velocity);
        }
        if let Some(value) = update.fixed_velocity_x.take() {
            self.set_fixed_velocity_component(VelocityComponent::X, value);
        }
        if let Some(value) = update.fixed_velocity_y.take() {
            self.set_fixed_velocity_component(VelocityComponent::Y, value);
        }
        if let Some(direction) = update.direction.take() {
            self.set_direction(direction);
        }
        if let Some(direction) = update.command_direction.take() {
            self.set_command_direction(direction);
        }
        if let Some(container) = update.container.take() {
            self.set_container(container);
        }
        if let Some(owner) = update.owner.take() {
            self.set_owner(owner);
        }
        if let Some(status) = update.status.take() {
            self.set_status(status);
        }
        if let Some(alive) = update.alive.take() {
            self.set_alive(alive);
        }
        if let Some(action) = update.action.take() {
            if let Some(name) = action.name.as_deref() {
                self.update_effective_action(name);
            }
            if let Some(target) = action.target {
                self.current_action_target = target;
            }
            if let Some(target) = action.target2 {
                self.current_action_target2 = target;
            }
            if let Some(data) = action.data {
                self.current_action_data = data;
            }
            if let Some(ticks) = action.ticks {
                self.current_action_ticks = ticks;
            }
            if let Some(phase) = action.phase {
                self.current_action_phase = phase;
            }
            match self.pending_update.action.as_mut() {
                Some(existing) => existing.merge(action),
                None => self.pending_update.action = Some(action),
            }
        }
        if !update.is_empty() {
            self.queued_commands.push(QueuedCommand::immediate(update));
        }
    }

    pub(crate) fn ocf(&self) -> u32 {
        // FnGetOCF returns pObj->OCF verbatim (C4Script.cpp:1354-1358):
        // the engine seeds the cached mask at call entry. Bare fixture
        // scopes without a seed keep the preview-grade recompute.
        let mut mask = self.cached_ocf.unwrap_or_else(|| {
            let alive = self.alive();
            let status = self.status();
            let is_contained = self.container().is_some();
            ocf::compute(
                self.ocf_base,
                self.crew_member,
                alive,
                status,
                is_contained,
                self.construction(),
                self.current_category,
            )
        });
        // SetAction calls SetOCF before Start/Abort callbacks
        // (C4Object.cpp:4165-4169). A disabled action immediately removes
        // the two action-gated bits from same-call GetOCF/world reads.
        if self
            .action_library
            .disables_object_for_entry(self.effective_action_name(), self.effective_action_index())
        {
            mask &= !(ocf::COLLECTION | ocf::FIGHT_READY);
        }
        mask
    }

    /// The OCF mask mid-call world reads see: `base` (the snapshot mask)
    /// with the bits re-derived whose driving state THIS call staged.
    /// C++ SetOCF runs synchronously on Enter/Exit (C4Object.cpp:
    /// 1531,1570), DoCon and the alive transitions (AssignDeath/
    /// AssignAlive -> SetOCF), so the live mask never lags those changes;
    /// bits driven by unstaged state keep their cached value (the NoFight
    /// and landscape gates stay unevaluated here).
    pub(crate) fn staged_ocf(&self, base: u32) -> u32 {
        let mut mask = base;
        if self.pending_update.container.is_some() && !self.exit_bounds_in_progress {
            // OCF_NotContained / OCF_Available (SetOCF, C4Object.cpp:
            // 611-618; Available's open-entrance arm is unmodeled).
            if self.container().is_some() {
                mask &= !(ocf::NOT_CONTAINED | ocf::AVAILABLE);
            } else {
                mask |= ocf::NOT_CONTAINED | ocf::AVAILABLE;
            }
        }
        if self.pending_update.construction.is_some() {
            if self.construction() >= FULL_CON {
                mask |= ocf::FULL_CON;
            } else {
                mask &= !ocf::FULL_CON;
            }
        }
        if self.pending_update.alive.is_some() || self.pending_update.category.is_some() {
            // OCF_Living/OCF_Alive gate on C4D_Living (C4Object.cpp:
            // 600-605), OCF_CrewMember on Def->CrewMember && Alive
            // (:619-622), OCF_FightReady on the Alive BIT (:606-610).
            mask &= !(ocf::LIVING | ocf::ALIVE | ocf::CREW_MEMBER | ocf::FIGHT_READY);
            let alive = self.alive();
            if self.category() & crate::CATEGORY_LIVING != 0 {
                mask |= ocf::LIVING;
                if alive {
                    mask |= ocf::ALIVE | ocf::FIGHT_READY;
                }
            }
            if self.crew_member && alive {
                mask |= ocf::CREW_MEMBER;
            }
        }
        if self
            .action_library
            .disables_object_for_entry(self.effective_action_name(), self.effective_action_index())
        {
            mask &= !(ocf::COLLECTION | ocf::FIGHT_READY);
        }
        mask
    }

    pub(crate) fn container(&self) -> Option<ObjectId> {
        match self.pending_update.container {
            Some(container) => container,
            None => self.current_container,
        }
    }

    pub(crate) fn set_container(&mut self, container: Option<ObjectId>) {
        let previous = self.container();
        if previous == container {
            return;
        }
        if let Some(previous) = previous {
            self.removed_contents_links.insert(previous);
        }
        if let Some(container) = container {
            self.current_contents_link_generation = self
                .current_contents_link_generation
                .checked_add(1)
                .unwrap_or(1);
            if self.removed_contents_links.contains(&container) {
                // Reuse the established engine fold for a remove/re-add
                // whose final Contained value otherwise collapses to a no-op.
                self.pending_update.change_def_reinsert = true;
            }
        }
        self.exit_bounds_in_progress = false;
        self.current_container = container;
        self.pending_update.container = Some(container);
        if container.is_none() {
            self.reset_contained_compiler_cache();
        }
        // Enter/Exit copy or explicitly assign position and therefore
        // resynchronize fix_x/fix_y after any earlier DoCon in this call.
        self.pending_update.construction_preserves_fixed_position = false;
        // C4Object::Enter/Exit force-close the moving object's menu
        // synchronously (CloseMenu(true), C4Object.cpp:1555 and :1594) —
        // staged here so a later same-call CreateMenu can still reopen one.
        self.pending_update.menu = Some(None);
    }

    pub(crate) fn refresh_cached_ocf(&mut self) {
        let base = self.ocf();
        let mut mask = self.staged_ocf(base);
        // AssignDeath's direct Alive writes deliberately do not invalidate
        // cached OCF. This helper represents an explicit SetOCF call
        // (including SetActionByName("Dead")), so acknowledge the raw word
        // here without making ordinary world reads during RemoveDeath do so.
        if self.raw_alive_override.is_some() {
            mask &= !(ocf::LIVING | ocf::ALIVE | ocf::CREW_MEMBER | ocf::FIGHT_READY);
            let alive = self.alive();
            if self.category() & crate::CATEGORY_LIVING != 0 {
                mask |= ocf::LIVING;
                if alive {
                    mask |= ocf::ALIVE | ocf::FIGHT_READY;
                }
            }
            if self.crew_member && alive {
                mask |= ocf::CREW_MEMBER;
            }
        }
        // SetOCF recomputes these object-state bits from scratch. Ordinary
        // SetXDir/SetYDir do not call this helper, while Exit/Enter/SetAction
        // do, preserving the C++ timing of stale versus refreshed masks.
        mask &= !(ocf::HIT_SPEED1
            | ocf::HIT_SPEED2
            | ocf::HIT_SPEED3
            | ocf::HIT_SPEED4
            | ocf::IN_LIQUID);
        mask |= crate::movement_hit_speed_flags(self.fixed_velocity());
        if self.in_liquid() && self.container().is_none() {
            mask |= ocf::IN_LIQUID;
        }
        self.cached_ocf = Some(mask);
    }

    pub(crate) fn mark_destroy_status(&mut self) {
        self.destroy = true;
    }

    pub(crate) fn clear_info_for_removal(&mut self) {
        self.set_info_rank(None);
        self.set_info_link(None);
        self.set_info_core(None);
    }

    pub(crate) fn update_effective_action(&mut self, action: &str) -> bool {
        let previous_name = self.current_action_name.clone();
        let previous_index = self.current_action_index;
        let previous_procedure = self
            .action_library
            .procedure_for_entry(&previous_name, previous_index);
        self.current_action_name = action.to_string();
        self.current_action_index = self.action_library.named_action_index(action);
        self.current_action_blocks_other_actions = self
            .action_library
            .blocks_other_actions_for_entry(action, self.current_action_index);
        let next_procedure = self
            .action_library
            .procedure_for_entry(action, self.current_action_index);
        (previous_name != action || previous_index != self.current_action_index)
            && previous_procedure != next_procedure
    }

    pub(crate) fn effective_action_name(&self) -> &str {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(name) = update.name.as_ref() {
                return name;
            }
        }
        &self.current_action_name
    }

    pub(crate) fn effective_action_index(&self) -> Option<u32> {
        if let Some(name) = self
            .pending_update
            .action
            .as_ref()
            .and_then(|update| update.name.as_deref())
        {
            return self.action_library.named_action_index(name);
        }
        self.current_action_index
    }

    pub(crate) fn effective_procedure_name(&self) -> Option<&str> {
        let action = self.effective_action_name();
        self.action_library
            .procedure_name_for_entry(action, self.effective_action_index())
    }

    pub(crate) fn effective_blocks_other_actions(&self) -> bool {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(name) = update.name.as_ref() {
                return self.action_library.blocks_other_actions_for_entry(
                    name,
                    self.action_library.named_action_index(name),
                );
            }
        }
        self.current_action_blocks_other_actions
    }

    pub(crate) fn effective_action_target(&self, index: usize) -> Option<ObjectId> {
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

    pub(crate) fn effective_action_ticks(&self) -> i32 {
        // Legacy name: this is C4Action::Time. ActionUpdate::ticks is the
        // distinct C4Action::PhaseDelay and must not shadow GetActTime.
        self.current_action_ticks
    }

    pub(crate) fn effective_action_procedure(&self) -> ActionProcedure {
        let action = self.effective_action_name();
        self.action_library
            .procedure_for_entry(action, self.effective_action_index())
    }

    #[allow(dead_code)]
    pub(crate) fn effective_action_data(&self) -> i32 {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(data) = update.data {
                return data;
            }
        }
        self.current_action_data
    }

    pub(crate) fn set_action_data(&mut self, data: i32) {
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

    pub(crate) fn reset_action_data(&mut self) {
        self.set_action_data(0);
    }

    pub(crate) fn action_phase(&self) -> i32 {
        if let Some(update) = self.pending_update.action.as_ref() {
            if let Some(phase) = update.phase {
                return phase;
            }
        }
        self.current_action_phase
    }

    pub(crate) fn set_action_phase(&mut self, phase: i32) {
        if self.current_action_phase == phase {
            if let Some(existing) = self
                .pending_update
                .action
                .as_ref()
                .and_then(|update| update.phase)
            {
                if existing == phase {
                    return;
                }
            } else {
                return;
            }
        }
        self.current_action_phase = phase;
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_phase(phase);
    }

    pub(crate) fn reset_action_ticks(&mut self) {
        self.reset_action_phase_delay();
        self.current_action_ticks = 0;
    }

    pub(crate) fn reset_action_phase_delay(&mut self) {
        let update = self
            .pending_update
            .action
            .get_or_insert_with(ActionUpdate::default);
        update.set_ticks(0);
    }

    pub(crate) fn energy(&self) -> i32 {
        self.pending_update.energy.unwrap_or(self.current_energy)
    }

    fn set_energy(&mut self, energy: i32) {
        self.current_energy = energy;
        self.pending_update.energy = Some(energy);
    }

    pub(crate) fn breath(&self) -> i32 {
        self.pending_update.breath.unwrap_or(self.current_breath)
    }

    pub(crate) fn set_breath(&mut self, breath: i32) {
        self.current_breath = breath;
        self.pending_update.breath = Some(breath);
    }

    pub(crate) fn need_energy(&self) -> bool {
        self.pending_update
            .need_energy
            .unwrap_or(self.current_need_energy)
    }

    pub(crate) fn set_need_energy(&mut self, need_energy: bool) {
        self.current_need_energy = need_energy;
        self.pending_update.need_energy = Some(need_energy);
    }

    /// C4Object::MagicEnergy (C4Object.h:139) through the pending overlay.
    pub(crate) fn magic_energy(&self) -> i32 {
        self.pending_update
            .magic_energy
            .unwrap_or(self.current_magic_energy)
    }

    pub(crate) fn set_magic_energy(&mut self, magic_energy: i32) {
        self.current_magic_energy = magic_energy;
        self.pending_update.magic_energy = Some(magic_energy);
    }

    pub(crate) fn damage(&self) -> i32 {
        self.pending_update.damage.unwrap_or(self.current_damage)
    }

    fn set_damage(&mut self, damage: i32) {
        let clamped = damage.max(0);
        self.current_damage = clamped;
        self.pending_update.damage = Some(clamped);
    }

    pub(crate) fn adjust_damage(&mut self, delta: i32) -> i32 {
        let current = self.damage();
        let mut next = current.saturating_add(delta);
        if next < 0 {
            next = 0;
        }
        self.set_damage(next);
        next
    }

    pub(crate) fn construction(&self) -> i32 {
        self.pending_update
            .construction
            .unwrap_or(self.current_construction)
    }

    fn set_construction(&mut self, construction: i32) {
        let clamped = construction.clamp(0, FULL_CON);
        self.current_construction = clamped;
        self.pending_update.construction = Some(clamped);
    }

    pub(crate) fn contact_density(&self) -> i32 {
        self.pending_update
            .contact_density
            .unwrap_or(self.current_contact_density)
    }

    pub(crate) fn set_contact_density(&mut self, contact_density: i32) {
        self.current_contact_density = contact_density;
        self.pending_update.contact_density = Some(contact_density);
    }

    fn adjust_construction(&mut self, delta: i32, oversize: bool) -> i32 {
        let current = self.construction();
        let mut next = current.saturating_add(delta);
        if next < 0 {
            next = 0;
        } else if !oversize && next > FULL_CON {
            next = FULL_CON;
        }
        self.current_construction = next;
        self.pending_update.construction = Some(next);
        self.pending_update.construction_via_docon = true;
        self.pending_update.construction_preserves_fixed_position = true;
        next
    }

    pub(crate) fn direction(&self) -> Direction {
        self.pending_update
            .direction
            .unwrap_or(self.current_direction)
    }

    pub(crate) fn set_direction(&mut self, direction: Direction) {
        if self.direction() == direction {
            return;
        }
        self.current_direction = direction;
        self.pending_update.direction = Some(direction);
        // C4Object::SetDir only refreshes the mirror for actions that declare
        // a FlipDir; the plain `Action.DrawDir = iDir` branch keeps whatever
        // transform the object already carries (C4Object.cpp:4276-4279).
        if self.action_flip_dir() != 0 {
            self.update_flip_dir();
        }
    }

    /// `C4ActionDef::FlipDir` of the action the object is currently in.
    pub(crate) fn action_flip_dir(&self) -> i32 {
        let action_name = self.effective_action_name();
        let action_index = self.effective_action_index();
        self.action_library
            .flip_dir_for_entry(action_name, action_index)
    }

    /// `C4Object::UpdateFlipDir` (C4Object.cpp:410-442). The mirror lives in
    /// the draw transform itself, so the renderer never re-derives it: a
    /// mirrored direction folds the sign into mat[0], and leaving the
    /// mirrored range unfolds it and drops a transform that became identity.
    pub(crate) fn update_flip_dir(&mut self) {
        let updated = DrawTransform::updated_flip_dir(
            self.draw_transform(),
            self.direction().to_script_value(),
            self.action_flip_dir(),
        );
        self.set_draw_transform(updated);
    }

    pub(crate) fn rotation(&self) -> i32 {
        self.pending_update
            .rotation
            .unwrap_or(self.current_rotation)
    }

    pub(crate) fn set_rotation(&mut self, rotation: i32, metadata: &DefinitionMetadata) {
        let normalized = rotation.rem_euclid(360);
        // C4Object::SetRotation always re-seeds fix_r and refreshes the
        // solid mask/face, even when the integer angle is unchanged.
        self.current_rotation = normalized;
        self.current_fixed_rotation = itofix(normalized);
        self.pending_update.rotation = Some(normalized);
        if metadata.line == 0 {
            self.pending_update.shape_override = Some(None);
        }
        self.refresh_shape_preview(metadata);
    }

    pub(crate) fn fixed_rotation(&self) -> C4Fixed {
        self.current_fixed_rotation
    }

    pub(crate) fn command_direction(&self) -> CommandDirection {
        self.pending_update
            .command_direction
            .unwrap_or(self.current_command_direction)
    }

    pub(crate) fn set_command_direction(&mut self, command_direction: CommandDirection) {
        if self.command_direction() == command_direction {
            return;
        }
        self.current_command_direction = command_direction;
        self.pending_update.command_direction = Some(command_direction);
    }

    pub(crate) fn fixed_velocity(&self) -> FixedVec2 {
        self.pending_update
            .fixed_velocity
            .unwrap_or(self.current_fixed_velocity)
    }

    /// Set the sub-pixel velocity and keep the whole-pixel mirror derived from
    /// it (`fixtoi`), so both `GetXDir`-style reads and the integer snapshot
    /// stay consistent with the `C4Fixed` source of truth.
    pub(crate) fn set_fixed_velocity(&mut self, velocity: FixedVec2) {
        self.current_fixed_velocity = velocity;
        self.pending_update.fixed_velocity = Some(velocity);
        // A later whole-vector native write supersedes component setters
        // staged earlier in the same script call. Component writes after
        // this remain separate and win during ObjectDelta application.
        self.pending_update.fixed_velocity_x = None;
        self.pending_update.fixed_velocity_y = None;
        // Keep the whole-pixel mirror consistent (fixtoi of the fixed value).
        self.pending_update.velocity = Some(Vector2::new(velocity.int_x(), velocity.int_y()));
    }

    /// Component-only dir write (FnSetXDir/FnSetYDir): stages just the
    /// touched component — the fold lands it on the object's TRUE fixed
    /// velocity without disturbing the other component.
    pub(crate) fn set_fixed_velocity_component(
        &mut self,
        component: VelocityComponent,
        value: C4Fixed,
    ) {
        let mut current = self.fixed_velocity();
        component.assign_fixed(&mut current, value);
        self.current_fixed_velocity = current;
        match component {
            VelocityComponent::X => self.pending_update.fixed_velocity_x = Some(value),
            VelocityComponent::Y => self.pending_update.fixed_velocity_y = Some(value),
        }
    }

    /// Angular velocity (`rdir`) as seen by `GetRDir`.
    pub(crate) fn rotation_velocity(&self) -> C4Fixed {
        self.pending_update
            .rotation_velocity
            .unwrap_or(self.current_rotation_velocity)
    }

    pub(crate) fn set_rotation_velocity(&mut self, rotation_velocity: C4Fixed) {
        self.current_rotation_velocity = rotation_velocity;
        self.pending_update.rotation_velocity = Some(rotation_velocity);
    }

    /// Queue a cyclic contents rotation so `new_front` becomes the first
    /// content (C4ObjectList::ShiftContents, C4ObjectList.cpp:815-833).
    pub(crate) fn shift_contents_front(&mut self, new_front: ObjectId) {
        self.pending_update.contents_front = Some(new_front);
    }

    pub(crate) fn effective_position(&self) -> Vector2 {
        self.pending_update
            .position
            .unwrap_or(self.current_position)
    }

    pub(crate) fn fixed_position(&self) -> FixedVec2 {
        self.current_fixed_position
    }

    pub(crate) fn set_position(&mut self, position: Vector2) {
        // ForcePosition always resets fix_x/fix_y, including its
        // same-integer-position fast path. Stage the write so that fixed
        // resynchronization survives the deferred host fold.
        self.current_position = position;
        self.current_fixed_position = FixedVec2::from_ints(position.x, position.y);
        self.pending_update.position = Some(position);
        self.pending_update.construction_preserves_fixed_position = false;
    }

    pub(crate) fn vertices(&self) -> &[ObjectVertex] {
        if let Some(vertices) = self.pending_update.shape_vertices.as_ref() {
            vertices.active()
        } else if let Some(vertices) = self.pending_update.live_vertices.as_ref() {
            vertices
        } else if let Some(vertices) = self.pending_update.vertices.as_ref() {
            vertices
        } else {
            self.shape_vertices.active()
        }
    }

    pub(crate) fn shape_vertex_buffer(&self) -> ShapeVertexBuffer {
        if let Some(vertices) = self.pending_update.shape_vertices.as_ref() {
            return vertices.clone();
        }
        let mut vertices = self.shape_vertices.clone();
        if let Some(active) = self.pending_update.live_vertices.as_ref() {
            vertices.replace_active(active);
        } else if let Some(active) = self.pending_update.vertices.as_ref() {
            vertices.replace_active(active);
        }
        vertices
    }

    pub(crate) fn set_shape_vertex_buffer(&mut self, vertices: ShapeVertexBuffer) {
        self.pending_update.live_vertices = Some(vertices.active_vec());
        self.pending_update.shape_vertices = Some(vertices);
    }

    pub(crate) fn set_graphics_overlay(&mut self, mut overlay: ObjectGraphicsOverlay) -> bool {
        let mut change = false;
        if let Some(existing) = self
            .graphics_overlays
            .iter_mut()
            .find(|existing| existing.id == overlay.id)
        {
            // C4GraphicsOverlay::Set reassigns mode, graphics, action, blit mode
            // and overlay object, resets iPhase, and deliberately keeps the
            // transform ("// (keep transform)") and dwClrModulation
            // (src/C4DefGraphics.cpp:682-693). Content sets a graphics overlay
            // and its SetObjDrawTransform independently, so rebuilding from
            // scratch would drop the transform on every graphics refresh.
            overlay.transform = existing.transform;
            overlay.color_modulation = existing.color_modulation;
            if *existing != overlay {
                *existing = overlay;
                change = true;
            }
        } else {
            self.graphics_overlays.push(overlay);
            self.graphics_overlays.sort_by_key(|overlay| overlay.id);
            change = true;
        }

        if change {
            self.pending_update.graphics_overlays = Some(self.graphics_overlays.clone());
        }
        change
    }

    pub(crate) fn remove_graphics_overlay(&mut self, id: i32) -> bool {
        let original_len = self.graphics_overlays.len();
        self.graphics_overlays.retain(|overlay| overlay.id != id);
        if self.graphics_overlays.len() != original_len {
            self.pending_update.graphics_overlays = Some(self.graphics_overlays.clone());
            true
        } else {
            false
        }
    }

    pub(crate) fn set_base_graphics(&mut self, base: Option<ObjectBaseGraphics>) -> bool {
        if self.base_graphics == base {
            return false;
        }
        self.base_graphics = base.clone();
        self.pending_update.base_graphics = Some(base);
        true
    }

    pub(crate) fn draw_transform(&self) -> Option<DrawTransform> {
        self.pending_update
            .draw_transform
            .unwrap_or(self.current_draw_transform)
    }

    pub(crate) fn set_draw_transform(&mut self, transform: Option<DrawTransform>) {
        if self.draw_transform() == transform {
            return;
        }
        self.current_draw_transform = transform;
        self.pending_update.draw_transform = Some(transform);
    }

    pub(crate) fn set_overlay_transform(
        &mut self,
        id: i32,
        transform: Option<DrawTransform>,
    ) -> bool {
        let mut changed = false;
        if let Some(existing) = self
            .graphics_overlays
            .iter_mut()
            .find(|overlay| overlay.id == id)
        {
            if existing.transform != transform {
                existing.transform = transform;
                changed = true;
            }
        } else {
            return false;
        }

        if changed {
            self.pending_update.graphics_overlays = Some(self.graphics_overlays.clone());
        }
        true
    }

    pub(crate) fn overlay_transform(&self, id: i32) -> Option<Option<DrawTransform>> {
        self.graphics_overlays
            .iter()
            .find(|overlay| overlay.id == id)
            .map(|overlay| overlay.transform)
    }

    pub(crate) fn set_action_target(&mut self, index: usize, target: Option<ObjectId>) {
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

    fn stage_compiler_cache(&mut self) {
        self.pending_update.compiler_cache = Some(self.current_compiler_cache.clone());
    }

    pub(crate) fn reset_contained_compiler_cache(&mut self) {
        self.current_compiler_cache.contained = 0;
        self.stage_compiler_cache();
    }

    fn reset_action_target_compiler_cache(&mut self, index: usize) {
        match index {
            0 => self.current_compiler_cache.action_target1 = 0,
            1 => self.current_compiler_cache.action_target2 = 0,
            _ => return,
        }
        self.stage_compiler_cache();
    }

    fn reset_layer_compiler_cache(&mut self) {
        self.current_compiler_cache.layer = 0;
        self.stage_compiler_cache();
    }

    fn references_object_pointer(&self, target: ObjectId) -> bool {
        self.current_action_target == Some(target)
            || self.current_action_target2 == Some(target)
            || self
                .live_commands
                .command_views()
                .iter()
                .any(|command| command.target == Some(target) || command.target2 == Some(target))
            || i32::try_from(target.as_u64()).is_ok_and(|target| {
                self.effects
                    .effects
                    .iter()
                    .any(|effect| effect.command_target == Some(target))
            })
    }

    fn clear_object_pointer(&mut self, target: ObjectId) {
        if self.current_action_target == Some(target) {
            self.set_action_target(0, None);
            self.reset_action_target_compiler_cache(0);
        }
        if self.current_action_target2 == Some(target) {
            self.set_action_target(1, None);
            self.reset_action_target_compiler_cache(1);
        }
        if self.live_commands.clear_object_reference(target) {
            self.command_count = self.live_commands.len();
            self.command_stack_replaced = true;
        }
        if let Ok(target) = i32::try_from(target.as_u64()) {
            self.effects.clear_command_target(target);
        }
    }

    /// `C4Object::DoEnergy` (C4Object.cpp:1345-1364): percent scale unless
    /// fExact (`iChange *= C4MaxPhysical/100`), clamped to
    /// 0..GetPhysical()->Energy, including a zero ceiling when the
    /// definition has no Physical Energy.
    pub(crate) fn adjust_energy(&mut self, delta: i32, exact: bool, max_energy: i32) -> i32 {
        let delta = if exact {
            delta
        } else {
            delta.saturating_mul(LEGACY_MAX_PHYSICAL / 100)
        };
        let next = crate::bound_energy(self.energy().saturating_add(delta), max_energy);
        self.set_energy(next);
        next
    }
}
