use indexmap::IndexSet;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smallvec::SmallVec;

use crate::ast::{
    AccessLevel, AssignmentTarget, BinaryOp, Expr, ForInit, Function, IndexOperand,
    NavigationOperation, Parameter, Stmt, TypeAnnotation, UnaryOp, VarDecl,
};
use crate::debugger::DebuggerHooks;
use crate::engine::{
    EvalDirectExecContinuationHook, EvalDirectExecHook, GlobalCallContextHook, GlobalSlots,
    GlobalVariables, HostFunction, HostReferenceFunction, RegisteredHostFunction,
};
use crate::error::{RuntimeCallFrame, RuntimeControl, RuntimeError};
use crate::lookup_profile;
use crate::value::{
    c4_id_text, c4_string_bytes, c4_string_from_bytes, c4_strings_equal, C4StringValue, C4VType,
    Literal, Value, ValueMap, ValueMapReferenceChange,
};

/// Maximum script call-stack depth, matching C++ `MAX_CONTEXT_STACK`
/// (C4AulExec.cpp:62). A script recursing within this bound runs; beyond it the
/// VM returns a clean error (C++ throws "call stack overflow", :143-145).
const MAX_CALL_DEPTH: usize = 512;
/// Fixed `C4AulExec::Values` capacity (C4AulExec.cpp:62-63). This is one
/// execution-wide stack: suspended callers, nested script hosts and DirectExec
/// all share the same 1,024 C4Value slots.
const MAX_VALUE_STACK: usize = 1024;
/// C4AUL_MAX_Par: every C4Aul call frame carries exactly 10 parameter slots
/// (C4Aul.h); `Par(n)` beyond them reads nil and `F(...)` forwards at most
/// this many.
const MAX_CALL_PARAMETERS: usize = 10;
/// `C4ValueList::MaxSize` (C4ValueList.h:30): array reference access may grow
/// through index 999,999, but the next slot throws "out of memory".
const ARRAY_MAX_SIZE: usize = 1_000_000;
/// `C4ValueList::MaxSize` (C4ValueList.h:32): `Global(index)` may grow up to,
/// but not including, this index.
const GLOBAL_SLOT_MAX_SIZE: i32 = 1_000_000;
/// C++ `DebugLog` diagnostics use a presentation route separate from script
/// `Log()`. The Mars integration regression compares this value with
/// `clonk-core`'s canonical routing constant without coupling the standalone
/// VM crate to the rest of the engine.
const SCRIPT_DEBUG_LOG_TARGET: &str = "clonk-script-debug";

type CallArgs = SmallVec<[CallArg; MAX_CALL_PARAMETERS]>;
type CallValues = SmallVec<[Value; MAX_CALL_PARAMETERS]>;
type HostCallArgs = SmallVec<[HostCallArg; MAX_CALL_PARAMETERS]>;
type CallBindings = SmallVec<[Binding; MAX_CALL_PARAMETERS]>;
type DiagnosticObjectFormatter = fn(u64) -> Option<(String, Option<String>)>;

#[derive(Clone, Copy)]
enum ResolvedHostFunction<'a> {
    Value(&'a RegisteredHostFunction),
    Reference(&'a HostReferenceFunction),
}

#[derive(Clone)]
enum CompiledCallTarget {
    /// A compiled frame owns the selected native callback.  C++ bytecode
    /// stores the `C4AulFunc *` in `AB_FUNC`; resolving the name again after a
    /// synchronous section switch can select a different overload or no
    /// function at all.
    Host(CompiledHostTarget),
    /// Keep the selected script body alive for the same reason.  The Arc is a
    /// queue-time snapshot, so unlink/relink of the destination host cannot
    /// invalidate a suspended child call.
    Script(CompiledScriptTarget),
    Method {
        failsafe: bool,
        reference: bool,
    },
    LegacyConstant,
    Builtin,
    Missing {
        error: Option<String>,
    },
    Global {
        target: RetainedCallTarget,
        failsafe: bool,
    },
}

#[derive(Clone)]
struct CompiledCallBinding {
    target: CompiledCallTarget,
    reference_parameters: u32,
}

#[derive(Clone)]
enum CompiledHostTarget {
    Value(RegisteredHostFunction),
    Reference(HostReferenceFunction),
}

#[derive(Clone)]
struct CompiledScriptTarget {
    function: Arc<Function>,
    validate_compiled_source: bool,
}

/// Whether a C4Aul entry point treats a failed script-parameter conversion as
/// an error. Scripted C4Effect callbacks request C++'s
/// `nonStrict3WarnConversionOnly` behavior for pre-`#strict 3` functions.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ParameterConversionFailurePolicy {
    Error,
    WarnForNonStrict3EffectCallback,
}

thread_local! {
    /// C++ owns one process-global executor. Rust tests execute VMs in
    /// parallel, so thread-local state preserves that synchronous singleton
    /// behavior without coupling unrelated test threads.
    static VALUE_STACK_SIZE: Cell<usize> = const { Cell::new(0) };
    /// AB_CALL/AB_CALLGLOBAL always supply ten parameter slots even when the
    /// selected native declares fewer. A cross-host dispatch consumes this
    /// one-shot override at the actual callee boundary.
    static CALL_PARAMETER_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
    /// Number of suspended C4Aul frames sharing the current removal index.
    static ACTIVE_OBJECT_REFERENCE_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Weak reverse links from each referenced object to the active C4Value
    /// cells that may contain it. This mirrors C++'s intrusive FirstRef lists
    /// without making the execution registry an owner of script values.
    static ACTIVE_OBJECT_REFERENCE_INDEX: RefCell<Option<ActiveObjectReferenceIndex>> = const {
        RefCell::new(None)
    };
    /// Shared object/global tables already discovered by an active frame.
    /// Entries retain their `Rc` owners so allocator address reuse cannot make
    /// a later, distinct table look registered.
    static ACTIVE_OBJECT_REFERENCE_TABLES: RefCell<Option<ActiveObjectReferenceTables>> = const {
        RefCell::new(None)
    };
    /// Ordered AssignRemoval events observed during the current re-entrant VM
    /// execution. Plain Rust temporaries that cannot be registered as cells
    /// replay only events occurring after they were evaluated.
    static ACTIVE_OBJECT_REFERENCE_SWEEPS: RefCell<Vec<u64>> = const {
        RefCell::new(Vec::new())
    };
    #[cfg(test)]
    static CALL_ARG_HEAP_SPILLS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static COMPILED_FUNCTION_EXECUTIONS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static COMPILED_BINDING_HEAP_SPILLS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static COMPILED_STACK_HEAP_SPILLS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static COMPILED_REGISTERED_SLOT_HEAP_SPILLS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static COMPILED_CALL_ARGUMENT_TEMPORARIES: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static DIAGNOSTIC_OBJECT_FORMATTER_CALLS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static DIAGNOSTIC_FRAME_STRING_ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static RUNTIME_CONTAINER_REGISTRATION_TRAVERSALS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static OBJECT_REFERENCE_TABLE_TRAVERSALS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static ACTIVE_OBJECT_REFERENCE_SWEEP_VISITS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static OBJECT_REFERENCE_INDEX_VALUE_VISITS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static OBJECT_REFERENCE_DISCOVERY_BORROWS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static OBJECT_REFERENCE_PENDING_PRUNE_VISITS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static GENERIC_HOST_RESOLUTIONS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static DIRECT_BINDING_ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static NESTED_GENERIC_SCRIPT_RESOLUTIONS: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static COMPILED_SOURCE_VALIDATIONS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn record_call_arg_heap_spill(spilled: bool) {
    if spilled {
        CALL_ARG_HEAP_SPILLS.with(|count| count.set(count.get() + 1));
    }
}

#[cfg(test)]
macro_rules! test_counter_accessors {
    ($(fn $reset:ident, $get:ident => $counter:ident;)+) => {
        $(
            fn $reset() {
                $counter.with(|count| count.set(0));
            }

            fn $get() -> usize {
                $counter.with(Cell::get)
            }
        )+
    };
    ($(pub(crate) fn $reset:ident, $get:ident => $counter:ident;)+) => {
        $(
            pub(crate) fn $reset() {
                $counter.with(|count| count.set(0));
            }

            pub(crate) fn $get() -> usize {
                $counter.with(Cell::get)
            }
        )+
    };
}

#[cfg(test)]
test_counter_accessors! {
    fn reset_compiled_function_execution_count, compiled_function_execution_count => COMPILED_FUNCTION_EXECUTIONS;
    fn reset_compiled_binding_heap_spills, compiled_binding_heap_spills => COMPILED_BINDING_HEAP_SPILLS;
    fn reset_diagnostic_object_formatter_calls, diagnostic_object_formatter_calls => DIAGNOSTIC_OBJECT_FORMATTER_CALLS;
    fn reset_diagnostic_frame_string_allocations, diagnostic_frame_string_allocations => DIAGNOSTIC_FRAME_STRING_ALLOCATIONS;
    fn reset_runtime_container_registration_traversals, runtime_container_registration_traversals => RUNTIME_CONTAINER_REGISTRATION_TRAVERSALS;
    fn reset_object_reference_table_traversals, object_reference_table_traversals => OBJECT_REFERENCE_TABLE_TRAVERSALS;
    fn reset_active_object_reference_sweep_visits, active_object_reference_sweep_visits => ACTIVE_OBJECT_REFERENCE_SWEEP_VISITS;
    fn reset_object_reference_index_value_visits, object_reference_index_value_visits => OBJECT_REFERENCE_INDEX_VALUE_VISITS;
    fn reset_object_reference_discovery_borrows, object_reference_discovery_borrows => OBJECT_REFERENCE_DISCOVERY_BORROWS;
    fn reset_object_reference_pending_prune_visits, object_reference_pending_prune_visits => OBJECT_REFERENCE_PENDING_PRUNE_VISITS;
    fn reset_generic_host_resolutions, generic_host_resolutions => GENERIC_HOST_RESOLUTIONS;
    fn reset_direct_binding_allocations, direct_binding_allocations => DIRECT_BINDING_ALLOCATIONS;
    fn reset_nested_generic_script_resolutions, nested_generic_script_resolutions => NESTED_GENERIC_SCRIPT_RESOLUTIONS;
}

#[cfg(test)]
test_counter_accessors! {
    pub(crate) fn reset_compiled_source_validations, compiled_source_validations => COMPILED_SOURCE_VALIDATIONS;
}

#[cfg(test)]
fn reset_compiled_executor_heap_spills() {
    COMPILED_STACK_HEAP_SPILLS.with(|count| count.set(0));
    COMPILED_REGISTERED_SLOT_HEAP_SPILLS.with(|count| count.set(0));
    COMPILED_CALL_ARGUMENT_TEMPORARIES.with(|count| count.set(0));
}

struct ValueStackReservation {
    count: usize,
    attached: bool,
}

impl ValueStackReservation {
    fn empty() -> Self {
        Self {
            count: 0,
            attached: true,
        }
    }

    fn reserve(count: usize) -> Result<Self, RuntimeError> {
        let mut reservation = Self::empty();
        reservation.grow(count)?;
        Ok(reservation)
    }

    fn check(count: usize) -> Result<(), RuntimeError> {
        VALUE_STACK_SIZE.with(|size| {
            let fits = size
                .get()
                .checked_add(count)
                .is_some_and(|next| next <= MAX_VALUE_STACK);
            if fits {
                Ok(())
            } else {
                Err(RuntimeError::new("internal error: value stack overflow!"))
            }
        })
    }

    fn grow(&mut self, count: usize) -> Result<(), RuntimeError> {
        if count == 0 {
            return Ok(());
        }
        if self.attached {
            Self::check(count)?;
            VALUE_STACK_SIZE.with(|size| size.set(size.get() + count));
        }
        self.count += count;
        Ok(())
    }

    fn shrink(&mut self, count: usize) {
        debug_assert!(self.count >= count);
        if self.attached {
            VALUE_STACK_SIZE.with(|size| {
                debug_assert!(size.get() >= count);
                size.set(size.get().saturating_sub(count));
            });
        }
        self.count = self.count.saturating_sub(count);
    }

    fn resize_to(&mut self, count: usize) -> Result<(), RuntimeError> {
        if count > self.count {
            self.grow(count - self.count)
        } else {
            self.shrink(self.count - count);
            Ok(())
        }
    }

    fn detach(&mut self) {
        if !self.attached {
            return;
        }
        VALUE_STACK_SIZE.with(|size| {
            debug_assert!(size.get() >= self.count);
            size.set(size.get().saturating_sub(self.count));
        });
        self.attached = false;
    }

    fn attach_unchecked(&mut self) {
        if self.attached || self.count == 0 {
            self.attached = true;
            return;
        }
        VALUE_STACK_SIZE.with(|size| size.set(size.get() + self.count));
        self.attached = true;
    }

    fn attach(&mut self) -> Result<(), RuntimeError> {
        if !self.attached {
            Self::check(self.count)?;
            self.attach_unchecked();
        }
        Ok(())
    }

    fn is_attached(&self) -> bool {
        self.attached
    }

    fn count(&self) -> usize {
        self.count
    }
}

impl Drop for ValueStackReservation {
    fn drop(&mut self) {
        if self.attached {
            VALUE_STACK_SIZE.with(|size| {
                debug_assert!(size.get() >= self.count);
                size.set(size.get().saturating_sub(self.count));
            });
        }
    }
}

struct CallParameterOverrideGuard {
    previous: Option<usize>,
    restore_previous: bool,
}

impl CallParameterOverrideGuard {
    fn enter(parameter_slots: usize) -> Self {
        let previous = CALL_PARAMETER_OVERRIDE.with(|slot| slot.replace(Some(parameter_slots)));
        Self {
            previous,
            restore_previous: true,
        }
    }

    /// Method-dispatch bridges establish their own ten-slot frame only when
    /// the caller has not already reserved it and installed a zero-slot
    /// ownership handoff.
    fn enter_if_absent(parameter_slots: usize) -> Self {
        CALL_PARAMETER_OVERRIDE.with(|slot| {
            if slot.get().is_some() {
                Self {
                    previous: None,
                    restore_previous: false,
                }
            } else {
                slot.set(Some(parameter_slots));
                Self {
                    previous: None,
                    restore_previous: true,
                }
            }
        })
    }
}

impl Drop for CallParameterOverrideGuard {
    fn drop(&mut self) {
        if self.restore_previous {
            CALL_PARAMETER_OVERRIDE.with(|slot| slot.set(self.previous));
        }
    }
}

fn take_call_parameter_slots(default: usize) -> usize {
    CALL_PARAMETER_OVERRIDE.with(|slot| slot.take().unwrap_or(default))
}

fn ensure_array_concat_size(left: usize, right: usize) -> Result<(), RuntimeError> {
    match left.checked_add(right) {
        Some(size) if size <= ARRAY_MAX_SIZE => Ok(()),
        _ => Err(RuntimeError::new("out of memory")),
    }
}

/// Run `f` with native-stack headroom, growing the stack when it runs low. Each
/// script-call level of this VM uses several KiB of native
/// stack, so deep (but C++-legal, <=512) recursion would otherwise overflow the
/// thread stack. Same thread, so thread-local host context stays visible.
fn maybe_grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(256 * 1024, 2 * 1024 * 1024, f)
}

/// C4Value::toString for `..`/`..=` (C4Value.cpp:47-65). Only strings,
/// integers, booleans and C4IDs have a string representation here.
fn concat_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.to_string()),
        Value::Int(value) => Some(value.to_string()),
        Value::Bool(value) => Some(i32::from(*value).to_string()),
        Value::RawBool(value) => Some((*value as u32 as i32).to_string()),
        Value::C4Id(value) => Some(c4_id_text(value)),
        _ => None,
    }
}

fn concat_type_name(value: &Value) -> &'static str {
    // C4Value's zero-data nil slot has C4V_Any type at this conversion site.
    if matches!(value, Value::Nil) {
        "any"
    } else {
        value.type_name()
    }
}

pub type ValueCell = Rc<RefCell<Value>>;
/// The per-call variable tables. Every read is a probe by name or slot and
/// every fold over them writes into another name-keyed map, so the fixed-seed
/// hasher changes nothing but the cost of a lookup.
type SlotMap = Rc<RefCell<FxHashMap<i32, ValueCell>>>;
type NamedLocalMap = Rc<RefCell<FxHashMap<String, ValueCell>>>;

#[derive(Default)]
struct FrameLocals {
    var_slots: RefCell<FxHashMap<i32, ValueCell>>,
    function_vars: RefCell<FxHashMap<String, Binding>>,
}

type FrameLocalMap = Rc<FrameLocals>;
type FxIndexSet<T> = IndexSet<T, FxBuildHasher>;

pub fn value_cell(value: Value) -> ValueCell {
    let cell = Rc::new(RefCell::new(value));
    register_active_object_reference_cell(&cell);
    cell
}

/// Replace a shared C4Value cell while keeping the active AssignRemoval index
/// aware of object references introduced by an embedding host.
#[doc(hidden)]
pub fn set_value_cell(cell: &ValueCell, value: Value) {
    *cell.borrow_mut() = value;
    register_active_object_reference_cell(cell);
}

fn register_shared_object_reference_cells<C: std::borrow::Borrow<ValueCell>>(
    cells: impl IntoIterator<Item = C>,
) {
    ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
        #[cfg(test)]
        OBJECT_REFERENCE_DISCOVERY_BORROWS.with(|count| count.set(count.get() + 1));
        if let Some(index) = index.borrow_mut().as_mut() {
            for cell in cells {
                index.ensure_registered(std::borrow::Borrow::borrow(&cell));
            }
        }
    });
}

/// Clear one object's references from every active C4Aul value cell, like
/// AssignRemoval's `while (FirstRef) FirstRef->Set0()` (C4Object.cpp:312).
#[doc(hidden)]
pub fn clear_active_object_references(object_id: u64) {
    let _sweep = ObjectReferenceSweep::active(object_id);
}

/// One instantaneous AssignRemoval reference sweep. The engine extends the
/// same sweep to persistent object locals and EffectVars before returning to
/// script.
#[doc(hidden)]
pub struct ObjectReferenceSweep {
    object_id: u64,
}

impl ObjectReferenceSweep {
    #[doc(hidden)]
    pub fn active(object_id: u64) -> Self {
        let mut sweep = Self { object_id };
        let cells = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            index
                .borrow_mut()
                .as_mut()
                .and_then(|index| {
                    index.prune_pending();
                    index.take_cells_for_object(object_id)
                })
                .unwrap_or_default()
        });
        for weak in cells.into_values() {
            let Some(cell) = weak.upgrade() else {
                continue;
            };
            #[cfg(test)]
            ACTIVE_OBJECT_REFERENCE_SWEEP_VISITS.with(|count| count.set(count.get() + 1));
            {
                let mut value = cell.borrow_mut();
                sweep.clear_value(&mut value);
            }
            refresh_active_object_reference_cell_after_sweep(&cell, object_id);
        }
        ACTIVE_OBJECT_REFERENCE_SWEEPS.with(|sweeps| sweeps.borrow_mut().push(object_id));
        sweep
    }

    #[doc(hidden)]
    pub fn clear_value(&mut self, value: &mut Value) -> bool {
        value.clear_object_reference(self.object_id)
    }

    #[doc(hidden)]
    pub fn clear_map(&mut self, map: &mut ValueMap) -> bool {
        let mut value = Value::Proplist(std::mem::take(map));
        let changed = self.clear_value(&mut value);
        let Value::Proplist(cleared) = value else {
            unreachable!("a reference-swept map remains a map");
        };
        *map = cleared;
        changed
    }
}

fn object_reference_sweep_cursor() -> usize {
    ACTIVE_OBJECT_REFERENCE_SWEEPS.with(|sweeps| sweeps.borrow().len())
}

fn clear_value_for_object_reference_sweeps(value: &mut Value, cursor: usize) {
    ACTIVE_OBJECT_REFERENCE_SWEEPS.with(|sweeps| {
        for object_id in sweeps.borrow().iter().skip(cursor).copied() {
            value.clear_object_reference(object_id);
        }
    });
}

fn object_target_id(value: &Value) -> Option<u64> {
    match value {
        Value::Object(id) if *id != 0 => Some(*id),
        _ => None,
    }
}

#[derive(Clone, Default)]
struct ObjectState {
    named_locals: NamedLocalMap,
    local_slots: SlotMap,
}

#[derive(Default)]
struct ActiveObjectReferenceTables {
    object_states: SmallVec<[(ObjectState, usize); 4]>,
}

#[derive(Default)]
struct ActiveObjectReferenceIndex {
    cells_by_object: FxHashMap<u64, FxHashMap<usize, Weak<RefCell<Value>>>>,
    memberships_by_cell: FxHashMap<usize, ActiveObjectReferenceMembership>,
    frame_addresses: Vec<FxHashSet<usize>>,
    pending_prune: FxIndexSet<usize>,
    deferred_prune: FxIndexSet<usize>,
}

struct ActiveObjectReferenceMembership {
    cell: Weak<RefCell<Value>>,
    object_counts: FxHashMap<u64, usize>,
}

impl ActiveObjectReferenceIndex {
    fn enter_frame(&mut self) {
        self.prune_pending();
        self.frame_addresses.push(FxHashSet::default());
    }

    fn leave_frame(&mut self) {
        let addresses = self
            .frame_addresses
            .pop()
            .expect("the active reference frame was entered");
        for address in addresses {
            let dead = self
                .memberships_by_cell
                .get(&address)
                .is_none_or(|membership| membership.cell.upgrade().is_none());
            if dead {
                self.remove_cell(address);
            } else {
                // The environment owning this cell drops immediately after
                // its guard. Recheck at the next nested entry or sweep; cells
                // that escaped into globals remain live and indexed.
                self.enqueue_pending_prune(address);
            }
        }
    }

    fn register(&mut self, cell: &ValueCell) {
        let value = cell.borrow();
        if !matches!(
            &*value,
            Value::Object(1..) | Value::Array(_) | Value::Proplist(_)
        ) {
            // Most shared globals and fixed call slots are scalars. They
            // cannot introduce a FirstRef link; only an overwrite of an
            // existing reference needs index or frame-lifetime maintenance.
            let address = Rc::as_ptr(cell) as usize;
            if self.memberships_by_cell.contains_key(&address) {
                self.remove_cell(address);
            }
            return;
        }
        let mut object_counts = FxHashMap::default();
        collect_object_reference_counts(&value, &mut object_counts);
        self.register_counts(cell, object_counts);
    }

    fn ensure_registered(&mut self, cell: &ValueCell) {
        // Discovery may avoid a recursive walk only while the exact Rc is
        // still indexed. Whole-cell writes use `register` (embedding hosts
        // use `set_value_cell`), and path writes apply their recorded delta.
        // Upgrading the Weak before ptr_eq keeps allocator address reuse from
        // making a later cell look like the departed owner of this slot.
        let address = Rc::as_ptr(cell) as usize;
        let already_tracked = self
            .memberships_by_cell
            .get(&address)
            .is_some_and(|membership| {
                membership
                    .cell
                    .upgrade()
                    .is_some_and(|registered| Rc::ptr_eq(&registered, cell))
            });
        if already_tracked {
            self.touch(address, true);
        } else {
            self.register(cell);
        }
    }

    fn register_counts(&mut self, cell: &ValueCell, object_counts: FxHashMap<u64, usize>) {
        let address = Rc::as_ptr(cell) as usize;
        let already_tracked = self
            .memberships_by_cell
            .get(&address)
            .is_some_and(|membership| {
                membership
                    .cell
                    .upgrade()
                    .is_some_and(|registered| Rc::ptr_eq(&registered, cell))
            });
        if let Some(membership) = self.memberships_by_cell.remove(&address) {
            for object_id in membership.object_counts.into_keys() {
                self.remove_link(object_id, address);
            }
        }
        if object_counts.is_empty() {
            self.clear_pending_prune(address);
            self.forget_address(address);
            return;
        }
        self.touch(address, already_tracked);
        let weak = Rc::downgrade(cell);
        for object_id in object_counts.keys() {
            self.cells_by_object
                .entry(*object_id)
                .or_default()
                .insert(address, weak.clone());
        }
        self.memberships_by_cell.insert(
            address,
            ActiveObjectReferenceMembership {
                cell: weak,
                object_counts,
            },
        );
    }

    fn apply_delta(&mut self, cell: &ValueCell, delta: ObjectReferenceDelta) {
        let address = Rc::as_ptr(cell) as usize;
        let mut membership = self
            .memberships_by_cell
            .remove(&address)
            .unwrap_or_else(|| ActiveObjectReferenceMembership {
                cell: Rc::downgrade(cell),
                object_counts: FxHashMap::default(),
            });
        let already_tracked = membership
            .cell
            .upgrade()
            .is_some_and(|registered| Rc::ptr_eq(&registered, cell));
        membership.cell = Rc::downgrade(cell);
        self.touch(address, already_tracked);

        for (object_id, removed) in delta.removed {
            let current = membership
                .object_counts
                .get(&object_id)
                .copied()
                .unwrap_or(0);
            debug_assert!(
                current >= removed,
                "path-write delta removed {removed} references to object {object_id}, but the root index held {current}"
            );
            if current <= removed {
                membership.object_counts.remove(&object_id);
                self.remove_link(object_id, address);
            } else {
                membership
                    .object_counts
                    .insert(object_id, current - removed);
            }
        }
        for (object_id, added) in delta.added {
            if added == 0 {
                continue;
            }
            let count = membership.object_counts.entry(object_id).or_default();
            if *count == 0 {
                self.cells_by_object
                    .entry(object_id)
                    .or_default()
                    .insert(address, membership.cell.clone());
            }
            *count = count.saturating_add(added);
        }

        if membership.object_counts.is_empty() {
            self.forget_address(address);
        } else {
            self.memberships_by_cell.insert(address, membership);
        }
    }

    fn touch(&mut self, address: usize, already_tracked: bool) {
        self.clear_pending_prune(address);
        if already_tracked
            && self
                .frame_addresses
                .iter()
                .any(|frame| frame.contains(&address))
        {
            return;
        }
        if let Some(frame) = self.frame_addresses.last_mut() {
            frame.insert(address);
        }
    }

    fn prune_pending(&mut self) {
        if self.pending_prune.is_empty() {
            std::mem::swap(&mut self.pending_prune, &mut self.deferred_prune);
        }
        let Some(address) = self.pending_prune.pop() else {
            return;
        };
        #[cfg(test)]
        OBJECT_REFERENCE_PENDING_PRUNE_VISITS.with(|count| count.set(count.get() + 1));
        let dead = self
            .memberships_by_cell
            .get(&address)
            .is_none_or(|membership| membership.cell.upgrade().is_none());
        if dead {
            self.remove_cell(address);
        } else {
            // Inspect one escaped owner per lifecycle event. Moving a live
            // cell into the next round keeps AssignRemoval independent of
            // the number of pending weak entries while still revisiting it
            // after its host owner releases it.
            self.deferred_prune.insert(address);
        }
    }

    fn take_cells_for_object(
        &mut self,
        object_id: u64,
    ) -> Option<FxHashMap<usize, Weak<RefCell<Value>>>> {
        let cells = self.cells_by_object.remove(&object_id)?;
        let mut emptied = Vec::new();
        for address in cells.keys().copied() {
            if let Some(membership) = self.memberships_by_cell.get_mut(&address) {
                membership.object_counts.remove(&object_id);
                if membership.object_counts.is_empty() {
                    emptied.push(address);
                }
            }
        }
        for address in emptied {
            self.remove_cell(address);
        }
        Some(cells)
    }

    fn remove_cell(&mut self, address: usize) {
        let Some(membership) = self.memberships_by_cell.remove(&address) else {
            self.clear_pending_prune(address);
            self.forget_address(address);
            return;
        };
        for object_id in membership.object_counts.into_keys() {
            self.remove_link(object_id, address);
        }
        self.clear_pending_prune(address);
        self.forget_address(address);
    }

    fn remove_link(&mut self, object_id: u64, address: usize) {
        let empty = self
            .cells_by_object
            .get_mut(&object_id)
            .is_some_and(|cells| {
                cells.remove(&address);
                cells.is_empty()
            });
        if empty {
            self.cells_by_object.remove(&object_id);
        }
    }

    fn forget_address(&mut self, address: usize) {
        for frame in &mut self.frame_addresses {
            frame.remove(&address);
        }
    }

    fn enqueue_pending_prune(&mut self, address: usize) {
        // New arrivals wait for the next round. Otherwise a steady stream of
        // nested escapes can keep the current set nonempty forever and starve
        // older cells that were deferred after a live check.
        self.pending_prune.swap_remove(&address);
        self.deferred_prune.insert(address);
    }

    fn clear_pending_prune(&mut self, address: usize) {
        self.pending_prune.swap_remove(&address);
        self.deferred_prune.swap_remove(&address);
    }

    #[cfg(test)]
    fn pending_prune_count(&self) -> usize {
        self.pending_prune.len() + self.deferred_prune.len()
    }
}

type ObjectReferenceCounts = FxHashMap<u64, usize>;

#[derive(Default)]
struct ObjectReferenceDelta {
    removed: ObjectReferenceCounts,
    added: ObjectReferenceCounts,
}

impl ObjectReferenceDelta {
    fn remove_value(&mut self, value: &Value) {
        collect_object_reference_counts(value, &mut self.removed);
    }

    fn add_value(&mut self, value: &Value) {
        collect_object_reference_counts(value, &mut self.added);
    }
}

fn collect_object_reference_counts(value: &Value, object_counts: &mut ObjectReferenceCounts) {
    #[cfg(test)]
    OBJECT_REFERENCE_INDEX_VALUE_VISITS.with(|count| count.set(count.get() + 1));
    match value {
        Value::Object(object_id) if *object_id != 0 => {
            *object_counts.entry(*object_id).or_default() += 1;
        }
        Value::Array(values) => {
            for value in values {
                collect_object_reference_counts(value, object_counts);
            }
        }
        Value::Proplist(entries) => {
            for (key, value) in entries {
                collect_object_reference_counts(key, object_counts);
                collect_object_reference_counts(value, object_counts);
            }
            for value in entries.hidden_values() {
                collect_object_reference_counts(value, object_counts);
            }
        }
        Value::Object(_) => {}
        Value::Int(_)
        | Value::Bool(_)
        | Value::RawBool(_)
        | Value::String(_)
        | Value::C4Id(_)
        | Value::Nil => {}
    }
}

fn register_active_object_reference_cell(cell: &ValueCell) {
    ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
        if let Some(index) = index.borrow_mut().as_mut() {
            index.register(cell);
        }
    });
}

fn refresh_active_object_reference_cell_after_sweep(cell: &ValueCell, swept_object_id: u64) {
    // Clearing one referenced object can destroy a containing map node and
    // therefore remove other object-valued C4Values from that same cell. Scan
    // only the cell selected by FirstRef, after releasing its mutable borrow,
    // and rebuild its reverse links from the resulting value.
    let mut object_counts = FxHashMap::default();
    collect_object_reference_counts(&cell.borrow(), &mut object_counts);
    object_counts.remove(&swept_object_id);
    ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
        if let Some(index) = index.borrow_mut().as_mut() {
            index.register_counts(cell, object_counts);
        }
    });
}

fn ensure_active_object_reference_cell_registered(cell: &ValueCell) {
    ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
        #[cfg(test)]
        OBJECT_REFERENCE_DISCOVERY_BORROWS.with(|count| count.set(count.get() + 1));
        if let Some(index) = index.borrow_mut().as_mut() {
            index.ensure_registered(cell);
        }
    });
}

fn apply_active_object_reference_delta(cell: &ValueCell, delta: ObjectReferenceDelta) {
    ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
        if let Some(index) = index.borrow_mut().as_mut() {
            index.apply_delta(cell, delta);
        }
    });
}

impl ActiveObjectReferenceTables {
    fn register_object_state(&mut self, state: &ObjectState, depth: usize) -> bool {
        if self.object_states.iter().any(|(registered, _)| {
            Rc::ptr_eq(&registered.named_locals, &state.named_locals)
                && Rc::ptr_eq(&registered.local_slots, &state.local_slots)
        }) {
            return false;
        }
        self.object_states.push((state.clone(), depth));
        true
    }

    fn leave_depth(&mut self, depth: usize) {
        self.object_states
            .retain(|(_, registered_depth)| *registered_depth != depth);
    }
}

impl ObjectState {
    fn from_local_vars(local_vars: &HashMap<String, Value>) -> Self {
        let state = Self::default();
        for (key, value) in local_vars {
            if let Some(idx) = key
                .strip_prefix("__local_")
                .and_then(|s| s.parse::<i32>().ok())
            {
                state
                    .local_slots
                    .borrow_mut()
                    .insert(idx.max(0), value_cell(value.clone()));
            } else {
                state
                    .named_locals
                    .borrow_mut()
                    .insert(key.clone(), value_cell(value.clone()));
            }
        }
        state
    }

    fn named_local_cell(&self, name: &str) -> ValueCell {
        self.named_locals
            .borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| value_cell(Value::Nil))
            .clone()
    }

    fn local_slot_cell(&self, index: i32) -> ValueCell {
        slot_cell(&self.local_slots, index)
    }

    fn clear_object_reference(&self, object_id: u64) {
        for cell in self.named_locals.borrow().values() {
            cell.borrow_mut().clear_object_reference(object_id);
        }
        for cell in self.local_slots.borrow().values() {
            cell.borrow_mut().clear_object_reference(object_id);
        }
    }

    fn to_local_vars(&self, var_decls: &[VarDecl]) -> HashMap<String, Value> {
        let mut updated_locals = HashMap::new();
        let named_locals = self.named_locals.borrow();
        for var_decl in var_decls {
            if let Some(cell) = named_locals.get(&var_decl.name) {
                updated_locals.insert(var_decl.name.clone(), cell.borrow().clone());
            }
        }
        drop(named_locals);

        for (idx, slot_value) in self.local_slots.borrow().iter() {
            updated_locals.insert(format!("__local_{idx}"), slot_value.borrow().clone());
        }
        updated_locals
    }
}

/// A shareable handle to an object's live local-variable cells: every VM
/// session created from the same handle reads and writes the SAME cells —
/// C++ semantics, where nested calls onto an in-flight object see its
/// mid-call local writes immediately (C4Aul mutates the live C4Object).
#[derive(Clone, Default)]
pub struct LocalCells {
    state: ObjectState,
}

impl LocalCells {
    pub fn from_local_vars(local_vars: &HashMap<String, Value>) -> Self {
        Self {
            state: ObjectState::from_local_vars(local_vars),
        }
    }

    /// The LIVE cell for one local by its persistence name — the engine's
    /// `__local_{i}` keys map to numbered slots, everything else to named
    /// locals. Cross-object references (LocalN/Local hooks) hand this out
    /// so foreign writes mutate the in-flight session directly (C++
    /// mutates the one live C4Object).
    pub fn cell(&self, name: &str) -> ValueCell {
        name.strip_prefix("__local_")
            .and_then(|index| index.parse::<i32>().ok())
            .map(|index| self.state.local_slot_cell(index))
            .unwrap_or_else(|| self.state.named_local_cell(name))
    }

    /// Every named local and indexed slot as a plain map (the fold shape
    /// call_with_locals returns).
    pub fn snapshot(&self) -> HashMap<String, Value> {
        let mut out = HashMap::new();
        for (name, cell) in self.state.named_locals.borrow().iter() {
            out.insert(name.clone(), cell.borrow().clone());
        }
        for (idx, cell) in self.state.local_slots.borrow().iter() {
            out.insert(format!("__local_{idx}"), cell.borrow().clone());
        }
        out
    }

    /// Clear object references from the live cells shared with an embedding
    /// engine. Suspended callbacks keep this handle outside the VM frame, so
    /// AssignRemoval must visit it before the host tears down the old object
    /// list (C4Object.cpp:312).
    pub fn clear_object_references(&self, object_id: u64) {
        self.state.clear_object_reference(object_id);
    }
}

fn slot_cell(slots: &SlotMap, index: i32) -> ValueCell {
    slots
        .borrow_mut()
        .entry(index.max(0))
        .or_insert_with(|| value_cell(Value::Nil))
        .clone()
}

fn frame_slot_cell(frame: &FrameLocals, index: i32) -> ValueCell {
    frame
        .var_slots
        .borrow_mut()
        .entry(index.max(0))
        .or_insert_with(|| value_cell(Value::Nil))
        .clone()
}

impl FrameLocals {
    fn clear_object_reference(&self, object_id: u64) {
        for cell in self.var_slots.borrow().values() {
            cell.borrow_mut().clear_object_reference(object_id);
        }
        for binding in self.function_vars.borrow_mut().values_mut() {
            binding.clear_object_reference(object_id);
        }
    }
}

/// The script frame immediately calling a native host function. C++ exposes
/// all pieces through `cthr->Caller`: `NumVars` backs `Var(n)`, while native
/// compatibility functions select either `Func->Owner->Strict` or
/// `Func->pOrgScript->Strict` depending on their C++ implementation.
#[derive(Clone)]
pub(crate) struct ScriptCallerContext {
    /// `cthr->Caller->NumVars` and `cthr->Caller->Vars`. Parameters and
    /// object locals deliberately do not appear in the named table.
    frame_locals: FrameLocalMap,
    /// Caller-local lookup host: `Func->Owner` for an ordinary function and
    /// the declaring `Func->LinkedTo` host for an engine-global function.
    /// This is intentionally independent from `this`, whose definition may
    /// change during the call.
    owner_host: ScriptHostIdentity,
    /// Whether the caller function resolves unqualified names through the
    /// engine/global scope. `GetLocalSFunc` keeps the linked destination
    /// host first, then permits the engine table only for this case.
    engine_scope: bool,
    /// Whether the current C4Aul context carries a non-null `Def`. DirectExec
    /// without an object clears it even when its receiver is a definition.
    definition_context: bool,
    /// `cthr->Caller->Func->Owner->Strict`, used by native compatibility
    /// functions. Includes/appends therefore use their destination owner.
    owner_strict_level: Option<u8>,
    /// `cthr->Caller->Func->pOrgScript->Strict` / `HasStrictNil()`, used by
    /// source-sensitive native conversions and script-function parameter
    /// conversion. Includes/appends retain source strictness here.
    origin_strict_level: Option<u8>,
    /// `C4AulScript::TemporaryScript` on the immediate caller frame.
    /// DirectExec/eval expressions set this; ordinary function calls do not.
    temporary_script: bool,
}

impl ScriptCallerContext {
    fn clear_object_reference(&self, object_id: u64) {
        self.frame_locals.clear_object_reference(object_id);
    }
}

/// Process-local identity of one compiled script host. It is meaningful only
/// while the host is alive and is used to match a native call's suspended
/// caller frame back to the exact `Engine` retained by clonk-engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScriptHostIdentity(usize);

impl ScriptHostIdentity {
    pub(crate) fn fresh() -> Self {
        static NEXT_IDENTITY: AtomicUsize = AtomicUsize::new(1);
        let identity = NEXT_IDENTITY
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("script host identity space exhausted");
        Self(identity)
    }
}

thread_local! {
    /// None while a native host function has no script caller (an
    /// engine-driven direct call). `owner_strict_level == None` inside a
    /// PRESENT frame instead means a NONSTRICT script caller.
    static HOST_CALLER_CONTEXT: RefCell<Option<ScriptCallerContext>> = const {
        RefCell::new(None)
    };
}

type ScriptTraceSink = Arc<dyn Fn(&str) + Send + Sync>;

struct ActiveDiagnosticFrame {
    kind: DiagnosticFrameKind,
    profile_started_at: Option<Instant>,
}

// Function frames deliberately embed C4AUL_MAX_Par inline. Boxing that variant
// would restore one heap allocation on every ordinary script call.
#[allow(clippy::large_enum_variant)]
enum DiagnosticFrameKind {
    Function {
        host_identity: Option<ScriptHostIdentity>,
        function: Arc<str>,
        arguments: CallValues,
        argument_reference_mask: u16,
        object_id: Option<u64>,
        definition_context: Option<Arc<str>>,
        source_host_identity: Option<ScriptHostIdentity>,
        source_name: Option<Arc<str>>,
        source_line: usize,
    },
    DirectExec(DirectExecDiagnosticFrame),
}

#[derive(Clone)]
struct DirectExecDiagnosticFrame {
    script_display: String,
    object_id: Option<u64>,
    object_fallback: Option<String>,
}

impl DirectExecDiagnosticFrame {
    fn new(script_display: String, object_id: Option<u64>) -> Self {
        let object_fallback = object_id.map(|id| {
            diagnostic_object_display(id)
                .map(|(display, _)| display)
                .unwrap_or_else(|| id.to_string())
        });
        Self {
            script_display,
            object_id,
            object_fallback,
        }
    }

    fn display(&self) -> String {
        let Some(id) = self.object_id else {
            return self.script_display.clone();
        };
        let object = diagnostic_object_display(id)
            .map(|(display, _)| display)
            .or_else(|| self.object_fallback.clone())
            .unwrap_or_else(|| id.to_string());
        format!("{} (obj {object})", self.script_display)
    }
}

#[derive(Clone)]
struct DirectExecContinuationContext {
    frame: DirectExecDiagnosticFrame,
    profile_on_error: bool,
}

impl DirectExecContinuationContext {
    fn new(frame: DirectExecDiagnosticFrame, profile_on_error: bool) -> Self {
        Self {
            frame,
            profile_on_error,
        }
    }
}

impl DiagnosticFrameKind {
    fn matches_profiler_target(&self, target: Option<ScriptHostIdentity>) -> bool {
        match self {
            Self::DirectExec(_) => true,
            Self::Function { host_identity, .. } => match target {
                None => true,
                Some(target) => *host_identity == Some(target),
            },
        }
    }

    fn trace_return_name(&self) -> &str {
        match self {
            Self::Function { function, .. } => function,
            Self::DirectExec(_) => "",
        }
    }
}

struct ScriptTraceRun {
    start_depth: usize,
    sink: ScriptTraceSink,
}

struct ScriptProfilerRun {
    target: Option<ScriptHostIdentity>,
    elapsed: HashMap<(Option<ScriptHostIdentity>, String), Duration>,
    direct_exec_started_at: Option<Instant>,
    direct_exec_elapsed: Duration,
}

#[derive(Default)]
struct ExecutionDiagnostics {
    frames: Vec<ActiveDiagnosticFrame>,
    trace: Option<ScriptTraceRun>,
    profiler: Option<ScriptProfilerRun>,
}

thread_local! {
    // C4AulExec owns one trace/profiler controller for the active execution
    // thread. Keeping this outside an individual Vm lets diagnostics survive
    // top-level calls and follow synchronous calls into another script host.
    static EXECUTION_DIAGNOSTICS: RefCell<ExecutionDiagnostics> =
        RefCell::new(ExecutionDiagnostics::default());
    /// Optional engine-side C4Object::GetDataString bridge. clonk-script knows
    /// object numbers, while the embedding engine owns live names/status.
    static DIAGNOSTIC_OBJECT_FORMATTER: Cell<Option<DiagnosticObjectFormatter>> =
        const { Cell::new(None) };
}

struct DiagnosticObjectFormatterGuard(Option<DiagnosticObjectFormatter>);

impl Drop for DiagnosticObjectFormatterGuard {
    fn drop(&mut self) {
        DIAGNOSTIC_OBJECT_FORMATTER.with(|cell| cell.set(self.0));
    }
}

/// Run one script entry with an embedding-provided C4Object::GetDataString
/// formatter. The bridge is thread-local and nesting-safe, matching the
/// thread-local C4Aul execution/host context.
#[doc(hidden)]
pub fn with_diagnostic_object_formatter<R>(
    formatter: fn(u64) -> Option<(String, Option<String>)>,
    action: impl FnOnce() -> R,
) -> R {
    let previous = DIAGNOSTIC_OBJECT_FORMATTER.with(|cell| cell.replace(Some(formatter)));
    let _guard = DiagnosticObjectFormatterGuard(previous);
    action()
}

fn diagnostic_object_display(id: u64) -> Option<(String, Option<String>)> {
    #[cfg(test)]
    DIAGNOSTIC_OBJECT_FORMATTER_CALLS.with(|count| count.set(count.get() + 1));
    DIAGNOSTIC_OBJECT_FORMATTER.with(|cell| cell.get().and_then(|formatter| formatter(id)))
}

/// `C4Value::GetDataString` (`C4Value.cpp`), the format the console's property
/// panel and every runtime diagnostic print values in.
///
/// Object values resolve through the embedding engine's formatter — see
/// [`with_diagnostic_object_formatter`] — so `Name #N` (or `{Name #N}` for a
/// non-normal status) needs that bridge installed; without it an object prints
/// as its bare number, exactly as C++ does for an object it cannot find.
///
/// C++'s `C4V_pC4Value` arm appends `*` to a reference's target. The port has
/// no reference *value* — references are cells at the VM level — so that arm is
/// unreachable here.
pub fn data_string(value: &Value) -> String {
    diagnostic_value_display(value)
}

fn diagnostic_value_display(value: &Value) -> String {
    match value {
        Value::Object(id) => diagnostic_object_display(*id)
            .map(|(display, _)| display)
            .unwrap_or_else(|| id.to_string()),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(diagnostic_value_display)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Proplist(entries) if entries.is_empty() => "{}".to_string(),
        Value::Proplist(entries) => format!(
            "{{ {} }}",
            entries
                .iter()
                .map(|(key, value)| format!(
                    "{} = {}",
                    diagnostic_value_display(key),
                    diagnostic_value_display(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => value.to_string(),
    }
}

/// Snapshot the singleton executor stack without changing its lifetime.
/// `RuntimeError::new` calls this before propagation can drop a diagnostic
/// guard, matching C++'s dump-before-unwind ordering.
pub(crate) fn snapshot_active_runtime_frames() -> Vec<RuntimeCallFrame> {
    EXECUTION_DIAGNOSTICS.with(|cell| {
        cell.borrow()
            .frames
            .iter()
            .rev()
            .map(|frame| match &frame.kind {
                DiagnosticFrameKind::DirectExec(frame) => {
                    RuntimeCallFrame::direct_exec(frame.display())
                }
                DiagnosticFrameKind::Function {
                    function,
                    arguments,
                    argument_reference_mask,
                    object_id,
                    definition_context,
                    source_host_identity,
                    source_name,
                    source_line,
                    ..
                } => {
                    let mut argument_count = arguments.len();
                    while argument_count != 0
                        && argument_reference_mask & (1_u16 << (argument_count - 1)) == 0
                        && matches!(arguments.get(argument_count - 1), Some(Value::Nil))
                    {
                        argument_count -= 1;
                    }
                    RuntimeCallFrame::new(
                        function.to_string(),
                        arguments[..argument_count]
                            .iter()
                            .enumerate()
                            .map(|(index, value)| {
                                let mut value = diagnostic_value_display(value);
                                if argument_reference_mask & (1_u16 << index) != 0 {
                                    value.push('*');
                                }
                                value
                            })
                            .collect::<Vec<_>>()
                            .join(","),
                        object_id.map(|id| {
                            diagnostic_object_display(id)
                                .map(|(display, _)| display)
                                .unwrap_or_else(|| id.to_string())
                        }),
                        definition_context.as_deref().map(str::to_owned),
                        *source_host_identity,
                        source_name.as_deref().map(str::to_owned),
                        *source_line,
                    )
                }
            })
            .collect()
    })
}

/// One completed script function in a [`stop_script_profiler`] report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptProfileEntry {
    /// `None` identifies a function owned by Game.ScriptEngine. DirectExec
    /// also uses `None`, but is distinguished by [`Self::direct_exec`].
    pub host_identity: Option<ScriptHostIdentity>,
    pub function: String,
    pub elapsed: Duration,
    /// The C++ profiler's one host-independent `Direct exec` aggregate.
    pub direct_exec: bool,
}

/// Arm C4Aul-style call tracing at the currently active script-stack depth.
/// Repeated starts while a trace is active are ignored like C++.
pub fn start_call_trace<F>(sink: F)
where
    F: Fn(&str) + Send + Sync + 'static,
{
    EXECUTION_DIAGNOSTICS.with(|cell| {
        let mut diagnostics = cell.borrow_mut();
        // A native entry has no script frame whose unwind could clear the
        // trace. DirectExec does own a temporary diagnostic frame, like C++.
        if !diagnostics.frames.is_empty() && diagnostics.trace.is_none() {
            diagnostics.trace = Some(ScriptTraceRun {
                start_depth: diagnostics.frames.len(),
                sink: Arc::new(sink),
            });
        }
    });
}

/// Reset and arm the singleton script profiler. `None` profiles the complete
/// engine script tree; a host identity restricts collection to that script.
pub fn start_script_profiler(target: Option<ScriptHostIdentity>) {
    EXECUTION_DIAGNOSTICS.with(|cell| {
        let mut diagnostics = cell.borrow_mut();
        let now = Instant::now();
        for frame in &mut diagnostics.frames {
            frame.profile_started_at = match frame.kind {
                DiagnosticFrameKind::Function { .. } => {
                    frame.kind.matches_profiler_target(target).then_some(now)
                }
                DiagnosticFrameKind::DirectExec(_) => None,
            };
        }
        diagnostics.profiler = Some(ScriptProfilerRun {
            target,
            elapsed: HashMap::new(),
            // C++ initializes its singleton timestamp in case profiling was
            // armed from inside an already-active DirectExec frame.
            direct_exec_started_at: Some(now),
            direct_exec_elapsed: Duration::ZERO,
        });
    });
}

/// Stop profiling and return the completed nonzero-millisecond entries in
/// descending elapsed-time order. Active frames are deliberately excluded:
/// C++ disables profiling before the caller of StopScriptProfiler unwinds.
pub fn stop_script_profiler() -> Option<Vec<ScriptProfileEntry>> {
    EXECUTION_DIAGNOSTICS.with(|cell| {
        let run = cell.borrow_mut().profiler.take()?;
        let direct_exec_elapsed = run.direct_exec_elapsed;
        let mut entries = run
            .elapsed
            .into_iter()
            .filter(|(_, elapsed)| elapsed.as_millis() != 0)
            .map(|((host_identity, function), elapsed)| ScriptProfileEntry {
                host_identity,
                function,
                elapsed,
                direct_exec: false,
            })
            .collect::<Vec<_>>();
        if direct_exec_elapsed.as_millis() != 0 {
            entries.push(ScriptProfileEntry {
                host_identity: None,
                function: "Direct exec".to_string(),
                elapsed: direct_exec_elapsed,
                direct_exec: true,
            });
        }
        entries.sort_by(|left, right| {
            right
                .elapsed
                .cmp(&left.elapsed)
                .then_with(|| left.function.cmp(&right.function))
                .then_with(|| left.host_identity.cmp(&right.host_identity))
        });
        Some(entries)
    })
}

/// Bottom-to-top display strings for active C4Aul DirectExec contexts.
///
/// This is intentionally limited to temporary frames: retaining formatted
/// argument strings on every ordinary call would tax gameplay while tracing
/// is inactive.
#[doc(hidden)]
pub fn active_direct_exec_diagnostic_frames() -> Vec<String> {
    EXECUTION_DIAGNOSTICS.with(|cell| {
        cell.borrow()
            .frames
            .iter()
            .filter_map(|frame| match &frame.kind {
                DiagnosticFrameKind::DirectExec(frame) => Some(frame.display()),
                DiagnosticFrameKind::Function { .. } => None,
            })
            .collect()
    })
}

fn start_direct_exec_profile() {
    EXECUTION_DIAGNOSTICS.with(|cell| {
        if let Some(run) = cell.borrow_mut().profiler.as_mut() {
            // Native C4AulExec owns one timestamp, not a nested timer stack.
            // A nested DirectExec deliberately overwrites the outer start.
            run.direct_exec_started_at = Some(Instant::now());
        }
    });
}

struct ScriptDiagnosticGuard {
    active: bool,
    profile_on_error: bool,
}

impl ScriptDiagnosticGuard {
    #[allow(clippy::too_many_arguments)]
    fn enter(
        name: Arc<str>,
        profile_host_identity: Option<ScriptHostIdentity>,
        args: CallValues,
        argument_reference_mask: u16,
        this_value: &Value,
        definition_context: Option<Arc<str>>,
        source_name: Option<Arc<str>>,
        function: &Function,
    ) -> Self {
        let emission = EXECUTION_DIAGNOSTICS.with(|cell| {
            let mut diagnostics = cell.borrow_mut();
            let depth = diagnostics.frames.len() + 1;
            let emission = diagnostics.trace.as_ref().map(|trace| {
                let indent = ">".repeat(depth.saturating_sub(trace.start_depth));
                let args = args
                    .iter()
                    .map(diagnostic_value_display)
                    .collect::<Vec<_>>()
                    .join(", ");
                (Arc::clone(&trace.sink), format!("T{indent}{name}({args})"))
            });
            let profile_started_at = diagnostics
                .profiler
                .as_ref()
                .filter(|run| match run.target {
                    None => true,
                    Some(target) => profile_host_identity == Some(target),
                })
                .map(|_| Instant::now());
            let object_id = match this_value {
                Value::Object(0) | Value::Nil => None,
                Value::Object(id) => Some(*id),
                _ => None,
            };
            diagnostics.frames.push(ActiveDiagnosticFrame {
                kind: DiagnosticFrameKind::Function {
                    host_identity: profile_host_identity,
                    function: name,
                    arguments: args,
                    argument_reference_mask,
                    object_id,
                    definition_context,
                    source_host_identity: function.source_host_identity(),
                    source_name,
                    source_line: function.source_line(),
                },
                profile_started_at,
            });
            emission
        });

        let guard = Self {
            active: true,
            profile_on_error: true,
        };
        if let Some((sink, message)) = emission {
            sink(&message);
        }
        guard
    }

    fn enter_direct(frame: DirectExecDiagnosticFrame, profile_on_error: bool) -> Self {
        let emission = EXECUTION_DIAGNOSTICS.with(|cell| {
            let mut diagnostics = cell.borrow_mut();
            let depth = diagnostics.frames.len() + 1;
            let stack_display = frame.display();
            let emission = diagnostics.trace.as_ref().map(|trace| {
                let indent = ">".repeat(depth.saturating_sub(trace.start_depth));
                (Arc::clone(&trace.sink), format!("T{indent}{stack_display}"))
            });
            diagnostics.frames.push(ActiveDiagnosticFrame {
                kind: DiagnosticFrameKind::DirectExec(frame),
                profile_started_at: None,
            });
            emission
        });

        let guard = Self {
            active: true,
            profile_on_error,
        };
        if let Some((sink, message)) = emission {
            sink(&message);
        }
        guard
    }

    fn returned(&mut self, value: &Value) {
        if self.active {
            self.active = false;
            exit_diagnostic_frame(Some(value), true);
        }
    }
}

impl Drop for ScriptDiagnosticGuard {
    fn drop(&mut self) {
        if self.active {
            self.active = false;
            exit_diagnostic_frame(None, self.profile_on_error);
        }
    }
}

fn exit_diagnostic_frame(returned: Option<&Value>, record_profile: bool) {
    let emission = EXECUTION_DIAGNOSTICS.with(|cell| {
        let mut diagnostics = cell.borrow_mut();
        let depth = diagnostics.frames.len();
        let frame = diagnostics.frames.pop()?;

        if record_profile {
            if let Some(run) = diagnostics.profiler.as_mut() {
                match &frame.kind {
                    DiagnosticFrameKind::Function {
                        host_identity,
                        function,
                        ..
                    } => {
                        if let Some(started_at) = frame.profile_started_at {
                            if frame.kind.matches_profiler_target(run.target) {
                                *run.elapsed
                                    .entry((*host_identity, function.to_string()))
                                    .or_default() += started_at.elapsed();
                            }
                        }
                    }
                    DiagnosticFrameKind::DirectExec(_) => {
                        if let Some(started_at) = run.direct_exec_started_at {
                            run.direct_exec_elapsed += started_at.elapsed();
                        }
                    }
                }
            }
        }

        let emission = diagnostics.trace.as_ref().and_then(|trace| {
            returned.map(|value| {
                let indent = ">".repeat(depth.saturating_sub(trace.start_depth));
                let value = diagnostic_value_display(value);
                (
                    Arc::clone(&trace.sink),
                    format!(
                        "T{indent}{} returned {value}",
                        frame.kind.trace_return_name()
                    ),
                )
            })
        });
        let trace_finished = diagnostics
            .trace
            .as_ref()
            .is_some_and(|trace| depth <= trace.start_depth);
        if trace_finished {
            diagnostics.trace = None;
        }
        emission
    });

    if let Some((sink, message)) = emission {
        sink(&message);
    }
}

/// Strictness of the script frame immediately calling the currently-running
/// native host function. `NoCaller` and `NonStrict` are deliberately distinct:
/// C++ native functions can branch on `!cthr->Caller` separately from the
/// caller script's `NONSTRICT` level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCallerStrictness {
    NoCaller,
    NonStrict,
    Strict(u8),
}

pub fn caller_strictness() -> HostCallerStrictness {
    HOST_CALLER_CONTEXT.with(|cell| match cell.borrow().as_ref() {
        None => HostCallerStrictness::NoCaller,
        Some(context) => match context.owner_strict_level {
            None | Some(0) => HostCallerStrictness::NonStrict,
            Some(level) => HostCallerStrictness::Strict(level),
        },
    })
}

/// Strictness of the script that originally defined the immediately calling
/// function (`cthr->Caller->Func->pOrgScript->Strict`). Included/appended
/// functions retain this level even when their destination owner has a
/// different strictness.
pub fn caller_origin_strictness() -> HostCallerStrictness {
    HOST_CALLER_CONTEXT.with(|cell| match cell.borrow().as_ref() {
        None => HostCallerStrictness::NoCaller,
        Some(context) => match context.origin_strict_level {
            None | Some(0) => HostCallerStrictness::NonStrict,
            Some(level) => HostCallerStrictness::Strict(level),
        },
    })
}

/// Exact local-lookup host of the function immediately calling the current
/// native host function: its destination owner for a local function or its
/// declaring `LinkedTo` host for a global. `None` means direct native entry.
pub fn caller_host_identity() -> Option<ScriptHostIdentity> {
    HOST_CALLER_CONTEXT.with(|cell| cell.borrow().as_ref().map(|context| context.owner_host))
}

/// Whether the script frame immediately calling the native host function is
/// an engine/global-scope function. `None` distinguishes a direct native
/// invocation with no suspended script caller.
pub fn caller_uses_engine_scope() -> Option<bool> {
    HOST_CALLER_CONTEXT.with(|cell| cell.borrow().as_ref().map(|context| context.engine_scope))
}

/// Whether the script frame immediately calling the current native host
/// function is a C4Aul DirectExec/eval temporary script. `None` means the
/// native was entered without a suspended script caller.
pub fn caller_is_temporary_script() -> Option<bool> {
    HOST_CALLER_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| context.temporary_script)
    })
}

/// The calling script function's numbered `Var(n)` slots, exposed to host
/// functions — the `cthr->Caller->NumVars` seam (FnFindConstructionSite
/// reads and writes them, C4Script.cpp:1958-1981). None when the
/// executing host function has no script caller
/// (`if (!cthr->Caller) return {}`, :1966).
pub fn caller_var_slots() -> Option<CallerVarSlots> {
    HOST_CALLER_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| CallerVarSlots(context.frame_locals.clone()))
    })
}

/// A live handle onto the caller's numbered var slots; writes go straight
/// into the suspended call's storage like C++ reference assignment.
pub struct CallerVarSlots(FrameLocalMap);

impl CallerVarSlots {
    /// C4ValueList::GetItem semantics: unset slots read nil.
    pub fn get(&self, index: i32) -> Value {
        frame_slot_cell(&self.0, index).borrow().clone()
    }

    pub fn set(&self, index: i32, value: Value) {
        let cell = frame_slot_cell(&self.0, index);
        notify_legacy_path_pins_before_cell_write(&cell, None, false);
        *cell.borrow_mut() = value;
        register_active_object_reference_cell(&cell);
    }
}

/// Scopes HOST_CALLER_CONTEXT to one host-function invocation, restoring the
/// previous frame on drop. Nested host calls through re-entrant VMs therefore
/// see the inner caller while executing and resume the outer attribution on
/// return, including during error unwinding.
struct CallerContextGuard(Option<ScriptCallerContext>);

impl CallerContextGuard {
    fn enter(context: Option<ScriptCallerContext>) -> Self {
        Self(HOST_CALLER_CONTEXT.with(|cell| cell.replace(context)))
    }
}

impl Drop for CallerContextGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        HOST_CALLER_CONTEXT.with(|cell| cell.replace(previous));
    }
}

fn current_caller_context() -> Option<ScriptCallerContext> {
    HOST_CALLER_CONTEXT.with(|cell| cell.borrow().clone())
}

#[derive(Clone, Debug)]
pub(crate) enum RawIdentity {
    /// String values carry their native shared C4String pointer directly.
    String(C4StringValue),
    /// Runtime strings and newly evaluated containers own distinct pointers.
    Heap(Rc<HeapIdentity>),
}

/// Identity metadata mirrors a C4ValueHash keyed by script values. The mutable
/// state inside string values is enumeration metadata and does not participate
/// in their equality or hash implementations.
#[derive(Clone, Debug)]
pub(crate) struct ProplistIdentities(HashMap<Value, Option<RawIdentity>>);

impl ProplistIdentities {
    fn with_capacity(capacity: usize) -> Self {
        Self(HashMap::with_capacity(capacity))
    }

    fn iter(&self) -> impl Iterator<Item = (&Value, &Option<RawIdentity>)> {
        self.0.iter()
    }

    fn get(&self, key: &Value) -> Option<&Option<RawIdentity>> {
        self.0.get(key)
    }

    fn remove(&mut self, key: &Value) -> Option<Option<RawIdentity>> {
        self.0.remove(key)
    }

    fn insert(&mut self, key: Value, identity: Option<RawIdentity>) -> Option<Option<RawIdentity>> {
        self.0.insert(key, identity)
    }

    fn retain(&mut self, predicate: impl FnMut(&Value, &mut Option<RawIdentity>) -> bool) {
        self.0.retain(predicate);
    }
}

impl FromIterator<(Value, Option<RawIdentity>)> for ProplistIdentities {
    fn from_iter<T: IntoIterator<Item = (Value, Option<RawIdentity>)>>(iter: T) -> Self {
        Self(HashMap::from_iter(iter))
    }
}

#[derive(Clone, Debug)]
pub(crate) enum HeapIdentity {
    Opaque,
    Array(Vec<Option<RawIdentity>>),
    Proplist(ProplistIdentities),
}

impl HeapIdentity {
    fn opaque_for(value: &Value) -> Self {
        match value {
            Value::Array(elements) => {
                Self::Array(elements.iter().map(RawIdentity::runtime).collect())
            }
            Value::Proplist(entries) => Self::Proplist(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), RawIdentity::runtime(value)))
                    .collect(),
            ),
            _ => Self::Opaque,
        }
    }

    fn identity_at(&self, segment: &PathSegment) -> Option<RawIdentity> {
        self.identity_ref_at(segment).cloned()
    }

    fn identity_ref_at(&self, segment: &PathSegment) -> Option<&RawIdentity> {
        match (self, segment) {
            (Self::Array(identities), PathSegment::Index(index)) => identities
                .get(array_index(index).ok()?)
                .and_then(Option::as_ref),
            (Self::Proplist(identities), PathSegment::Property(key)) => identities
                .get(&Value::String(key.clone().into()))
                .and_then(Option::as_ref),
            (Self::Proplist(identities), PathSegment::Index(key)) => {
                identities.get(key).and_then(Option::as_ref)
            }
            _ => None,
        }
    }

    fn after_path_write(
        current: Option<&Self>,
        value: &Value,
        segments: &[PathSegment],
        replacement: Option<RawIdentity>,
    ) -> Self {
        let Some((segment, rest)) = segments.split_first() else {
            return current.cloned().unwrap_or_else(|| Self::opaque_for(value));
        };

        match (value, segment) {
            (Value::Array(elements), PathSegment::Index(index)) => {
                let mut identities = match current {
                    Some(Self::Array(identities)) => identities.clone(),
                    _ => match Self::opaque_for(value) {
                        Self::Array(identities) => identities,
                        _ => unreachable!(),
                    },
                };
                identities.resize(elements.len(), None);
                let Some(index) = array_index(index)
                    .ok()
                    .filter(|index| *index < elements.len())
                else {
                    return Self::Array(identities);
                };
                identities[index] = if rest.is_empty() {
                    replacement
                } else {
                    RawIdentity::after_path_write(
                        identities[index].as_ref(),
                        &elements[index],
                        rest,
                        replacement,
                    )
                };
                Self::Array(identities)
            }
            (
                Value::Proplist(entries),
                segment @ (PathSegment::Property(_) | PathSegment::Index(_)),
            ) => {
                let mut identities = match current {
                    Some(Self::Proplist(identities)) => identities.clone(),
                    _ => match Self::opaque_for(value) {
                        Self::Proplist(identities) => identities,
                        _ => unreachable!(),
                    },
                };
                let key = match segment {
                    PathSegment::Property(key) => Value::String(key.clone().into()),
                    PathSegment::Index(key) => key.clone(),
                };
                let Some(child) = entries.get_key(&key) else {
                    identities.remove(&key);
                    return Self::Proplist(identities);
                };
                let current_child = identities.get(&key).and_then(Option::as_ref);
                let identity = if rest.is_empty() {
                    replacement
                } else {
                    RawIdentity::after_path_write(current_child, child, rest, replacement)
                };
                identities.insert(key, identity);
                Self::Proplist(identities)
            }
            _ => current.cloned().unwrap_or_else(|| Self::opaque_for(value)),
        }
    }
}

impl RawIdentity {
    fn runtime(value: &Value) -> Option<Self> {
        match value {
            Value::String(value) => Some(Self::String(value.clone())),
            Value::Array(_) | Value::Proplist(_) => {
                Some(Self::Heap(Rc::new(HeapIdentity::opaque_for(value))))
            }
            Value::Nil
            | Value::Int(_)
            | Value::Bool(_)
            | Value::RawBool(_)
            | Value::C4Id(_)
            | Value::Object(_) => None,
        }
    }

    fn identity_at(&self, segment: &PathSegment) -> Option<Self> {
        match self {
            Self::Heap(identity) => identity.identity_at(segment),
            Self::String(_) => None,
        }
    }

    fn identity_at_path(&self, segments: &[PathSegment]) -> Option<Self> {
        self.identity_ref_at_path(segments).cloned()
    }

    fn identity_ref_at_path(&self, segments: &[PathSegment]) -> Option<&Self> {
        let mut current = self;
        for segment in segments {
            current = match current {
                Self::Heap(identity) => identity.identity_ref_at(segment)?,
                Self::String(_) => return None,
            };
        }
        Some(current)
    }

    fn after_path_write(
        current: Option<&Self>,
        value: &Value,
        segments: &[PathSegment],
        replacement: Option<Self>,
    ) -> Option<Self> {
        if !matches!(value, Value::Array(_) | Value::Proplist(_)) {
            return Self::runtime(value);
        }
        let current = match current {
            Some(Self::Heap(identity)) => Some(identity.as_ref()),
            _ => None,
        };
        Some(Self::Heap(Rc::new(HeapIdentity::after_path_write(
            current,
            value,
            segments,
            replacement,
        ))))
    }
}

impl PartialEq for RawIdentity {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (RawIdentity::String(left), RawIdentity::String(right)) => left.ptr_eq(right),
            (RawIdentity::Heap(left), RawIdentity::Heap(right)) => Rc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl Eq for RawIdentity {}

#[derive(Clone)]
pub(crate) struct TrackedValue {
    value: Value,
    identity: Option<RawIdentity>,
}

impl TrackedValue {
    fn runtime(value: Value) -> Self {
        let identity = Self::runtime_identity(&value);
        Self { value, identity }
    }

    fn runtime_identity(value: &Value) -> Option<RawIdentity> {
        RawIdentity::runtime(value)
    }

    fn literal(value: Value, literal: &Literal) -> Self {
        let _ = literal;
        let identity = Self::runtime_identity(&value);
        Self { value, identity }
    }

    fn array(elements: Vec<Self>) -> Self {
        let identities = elements
            .iter()
            .map(|element| element.identity.clone())
            .collect();
        let value = Value::Array(elements.into_iter().map(|element| element.value).collect());
        Self {
            value,
            identity: Some(RawIdentity::Heap(Rc::new(HeapIdentity::Array(identities)))),
        }
    }

    fn proplist(entries: Vec<(Value, Self)>) -> Self {
        let mut values = ValueMap::with_capacity(entries.len());
        let mut identities = ProplistIdentities::with_capacity(entries.len());
        for (key, entry) in entries {
            let TrackedValue { value, identity } = entry;
            c4_map_assign_set(&mut values, key.clone(), value);
            if values.contains_value_key(&key) {
                identities.insert(key, identity);
            } else {
                identities.remove(&key);
            }
        }
        Self {
            value: Value::Proplist(values),
            identity: Some(RawIdentity::Heap(Rc::new(HeapIdentity::Proplist(
                identities,
            )))),
        }
    }

    fn identity_at(&self, segment: &PathSegment) -> Option<RawIdentity> {
        self.identity
            .as_ref()
            .and_then(|identity| identity.identity_at(segment))
    }

    /// Apply the zero-C4ID part of C++ `C4Value::Set`
    /// (C4Value.cpp:121-140). A retained zero-payload ID tag can exist in a
    /// parameter or container slot, but an ordinary value-stack copy
    /// canonicalizes it to `C4V_Any`.
    fn set_copy(self) -> Self {
        if c4_set_copy_is_zero_id(&self.value) {
            Self::runtime(Value::Nil)
        } else {
            self
        }
    }

    /// Assign through `C4Value::Set`, including its same-data/type early
    /// return. That early return is observable for the exceptional retained
    /// `C4V_C4ID(0)` value: writing it over itself keeps the tag.
    fn set_copy_into(self, destination_is_same_zero_id: bool) -> Self {
        if destination_is_same_zero_id {
            self
        } else {
            self.set_copy()
        }
    }

    fn clear_object_reference_sweeps(&mut self, cursor: usize) {
        clear_value_for_object_reference_sweeps(&mut self.value, cursor);
    }

    fn clear_object_reference(&mut self, object_id: u64) {
        self.value.clear_object_reference(object_id);
    }
}

fn c4_set_copy_is_zero_id(value: &Value) -> bool {
    matches!(value, Value::C4Id(id) if crate::value::c4_id_raw(id) == 0)
}

fn c4_set_copy_value(value: Value) -> Value {
    if c4_set_copy_is_zero_id(&value) {
        Value::Nil
    } else {
        value
    }
}

fn c4_set_copy_value_into(value: Value, destination_is_same_zero_id: bool) -> Value {
    if destination_is_same_zero_id {
        value
    } else {
        c4_set_copy_value(value)
    }
}

fn c4_map_assign_set(map: &mut ValueMap, key: Value, value: Value) {
    c4_map_assign_set_recording(map, key, value, None);
}

fn c4_map_assign_set_recording(
    map: &mut ValueMap,
    key: Value,
    value: Value,
    mut reference_delta: Option<&mut ObjectReferenceDelta>,
) {
    let mut record = |change, value: &Value| {
        let Some(delta) = reference_delta.as_deref_mut() else {
            return;
        };
        match change {
            ValueMapReferenceChange::Removed => delta.remove_value(value),
            ValueMapReferenceChange::Added => delta.add_value(value),
        }
    };
    if c4_set_copy_is_zero_id(&value) {
        map.assign_key_zero_c4id_recording(key, &mut record);
    } else {
        map.assign_key_recording(key, value, &mut record);
    }
}

fn c4_map_assign_property_set_recording(
    map: &mut ValueMap,
    key: String,
    value: Value,
    mut reference_delta: Option<&mut ObjectReferenceDelta>,
) {
    let mut record = |change, value: &Value| {
        let Some(delta) = reference_delta.as_deref_mut() else {
            return;
        };
        match change {
            ValueMapReferenceChange::Removed => delta.remove_value(value),
            ValueMapReferenceChange::Added => delta.add_value(value),
        }
    };
    if c4_set_copy_is_zero_id(&value) {
        map.assign_zero_c4id_recording(key, &mut record);
    } else {
        map.assign_recording(key, value, &mut record);
    }
}

type RawIdentityCell = Rc<RefCell<Option<RawIdentity>>>;

struct InlineBinding {
    initial: TrackedValue,
    promoted: std::cell::OnceCell<(ValueCell, RawIdentityCell)>,
}

impl InlineBinding {
    fn new(initial: TrackedValue) -> Self {
        Self {
            initial,
            promoted: std::cell::OnceCell::new(),
        }
    }

    fn cells(&self) -> &(ValueCell, RawIdentityCell) {
        self.promoted.get_or_init(|| {
            (
                value_cell(self.initial.value.clone()),
                Rc::new(RefCell::new(self.initial.identity.clone())),
            )
        })
    }

    fn read_tracked(&self) -> TrackedValue {
        let Some((value, identity)) = self.promoted.get() else {
            return self.initial.clone();
        };
        let identity = legacy_identity_for_value_copy(value, &[], identity.borrow().clone());
        TrackedValue {
            value: value.borrow().clone(),
            identity,
        }
    }

    fn lvalue(&self) -> LValueRef {
        let (value, identity) = self.cells();
        LValueRef::tracked_cell(value.clone(), identity.clone())
    }
}

impl Clone for InlineBinding {
    fn clone(&self) -> Self {
        let cloned = Self::new(self.read_tracked());
        if let Some((value, identity)) = self.promoted.get() {
            cloned
                .promoted
                .set((value.clone(), identity.clone()))
                .expect("fresh inline binding accepts promoted cells");
        }
        cloned
    }
}

enum Binding {
    Direct {
        value: ValueCell,
        identity: RawIdentityCell,
    },
    /// An unnamed C4Aul parameter slot. `Par()` and forwarded `...` can read
    /// it, but no source-level name can take its address or assign it.
    Inline(InlineBinding),
    Reference(LValueRef),
}

impl Clone for Binding {
    fn clone(&self) -> Self {
        match self {
            Self::Direct { value, identity } => Self::Direct {
                value: value.clone(),
                identity: identity.clone(),
            },
            Self::Inline(inline) => Self::Inline(inline.clone()),
            Self::Reference(reference) => Self::Reference(reference.clone()),
        }
    }
}

impl Binding {
    fn direct(value: Value) -> Self {
        Self::tracked(TrackedValue::runtime(value))
    }

    fn collect_object_reference_cells(&self, cells: &mut Vec<Weak<RefCell<Value>>>) {
        match self {
            Self::Direct { value, .. } => cells.push(Rc::downgrade(value)),
            Self::Inline(inline) => {
                if let Some((value, _)) = inline.promoted.get() {
                    cells.push(Rc::downgrade(value));
                } else if inline.initial.value.contains_any_object_reference() {
                    cells.push(Rc::downgrade(&inline.cells().0));
                }
            }
            Self::Reference(reference) => reference.collect_object_reference_cells(cells),
        }
    }

    fn clear_object_reference(&mut self, object_id: u64) {
        match self {
            Self::Direct { value, .. } => {
                value.borrow_mut().clear_object_reference(object_id);
            }
            Self::Inline(inline) => {
                if let Some((value, _)) = inline.promoted.get() {
                    value.borrow_mut().clear_object_reference(object_id);
                } else {
                    inline.initial.clear_object_reference(object_id);
                }
            }
            Self::Reference(reference) => reference.clear_object_reference(object_id),
        }
    }

    fn tracked(tracked: TrackedValue) -> Self {
        #[cfg(test)]
        DIRECT_BINDING_ALLOCATIONS.with(|count| count.set(count.get() + 1));
        Binding::Direct {
            value: value_cell(tracked.value),
            identity: Rc::new(RefCell::new(tracked.identity)),
        }
    }

    fn read_tracked(&self) -> Result<TrackedValue, RuntimeError> {
        match self {
            Binding::Direct { value, identity } => {
                let identity =
                    legacy_identity_for_value_copy(value, &[], identity.borrow().clone());
                Ok(TrackedValue {
                    value: value.borrow().clone(),
                    identity,
                })
            }
            Binding::Inline(inline) => Ok(inline.read_tracked()),
            Binding::Reference(reference) => reference.read_tracked(),
        }
    }

    fn write_tracked(&self, tracked: TrackedValue) -> Result<(), RuntimeError> {
        match self {
            Binding::Direct { value, identity } => {
                if c4_set_copy_is_zero_id(&tracked.value) && c4_set_copy_is_zero_id(&value.borrow())
                {
                    return Ok(());
                }
                let tracked = tracked.set_copy();
                let preserves_container = identity
                    .borrow()
                    .as_ref()
                    .zip(tracked.identity.as_ref())
                    .is_some_and(|(current, replacement)| current == replacement);
                notify_legacy_path_pins_before_cell_write(
                    value,
                    Some(identity),
                    preserves_container,
                );
                *value.borrow_mut() = tracked.value;
                register_active_object_reference_cell(value);
                *identity.borrow_mut() = tracked.identity;
                Ok(())
            }
            Binding::Inline(inline) => inline.lvalue().write_tracked(tracked),
            Binding::Reference(reference) => reference.write_tracked(tracked),
        }
    }

    fn lvalue(&self) -> LValueRef {
        match self {
            Binding::Direct { value, identity } => {
                LValueRef::tracked_cell(value.clone(), identity.clone())
            }
            Binding::Inline(inline) => inline.lvalue(),
            Binding::Reference(reference) => reference.clone(),
        }
    }

    fn value_slot_is_same_zero_id(&self, value: &Value) -> bool {
        c4_set_copy_is_zero_id(value)
            && match self {
                Binding::Direct { value, .. } => c4_set_copy_is_zero_id(&value.borrow()),
                Binding::Inline(inline) => c4_set_copy_is_zero_id(&inline.read_tracked().value),
                Binding::Reference(_) => false,
            }
    }
}

#[derive(Clone)]
pub(crate) enum LValueRef {
    Cell {
        value: ValueCell,
        identity: Option<RawIdentityCell>,
    },
    Path {
        root: ValueCell,
        root_identity: Option<RawIdentityCell>,
        segments: Vec<PathSegment>,
        legacy_pin: Option<Rc<RefCell<LegacyPathPin>>>,
    },
    /// A reference returned by a value-style host getter/setter. The engine's
    /// `EffectVar` host uses three addressing arguments for reads and accepts
    /// the replacement value as a fourth argument for writes. Retaining the
    /// call and an optional container path models C++'s `C4V_pC4Value` through
    /// `AB_ARRAYA_R` without flattening it to a copied array.
    HostPath {
        function: HostFunction,
        args: Vec<Value>,
        caller: ScriptCallerContext,
        global_call_context_hook: Option<GlobalCallContextHook>,
        segments: Vec<PathSegment>,
        legacy_pin: Option<Rc<RefCell<LegacyHostPathPin>>>,
    },
}

/// An opaque live C4Value reference returned across the engine's method
/// dispatch boundary. The VM alone interprets the underlying lvalue; hosts
/// may retain and route it without flattening it to a value.
#[derive(Clone)]
pub struct ValueReference(LValueRef);

impl ValueReference {
    pub fn from_cell(cell: ValueCell) -> Self {
        Self(LValueRef::cell(cell))
    }

    fn into_lvalue(self) -> LValueRef {
        self.0.ensure_active_object_reference_cell_registered();
        self.0
    }
}

impl LValueRef {
    fn ensure_active_object_reference_cell_registered(&self) {
        match self {
            Self::Cell { value, .. } => ensure_active_object_reference_cell_registered(value),
            Self::Path { root, .. } => ensure_active_object_reference_cell_registered(root),
            Self::HostPath { .. } => {}
        }
    }

    fn collect_object_reference_cells(&self, cells: &mut Vec<Weak<RefCell<Value>>>) {
        match self {
            Self::Cell { value, .. } => cells.push(Rc::downgrade(value)),
            Self::Path { root, .. } => cells.push(Rc::downgrade(root)),
            Self::HostPath { .. } => {}
        }
    }

    fn clear_object_reference(&mut self, object_id: u64) {
        match self {
            Self::Cell { value, .. } => {
                value.borrow_mut().clear_object_reference(object_id);
            }
            Self::Path {
                root, legacy_pin, ..
            } => {
                root.borrow_mut().clear_object_reference(object_id);
                if let Some(legacy_pin) = legacy_pin {
                    if let Some(resolved) = legacy_pin.borrow_mut().resolved.as_mut() {
                        resolved.clear_object_reference(object_id);
                    }
                }
            }
            Self::HostPath {
                args,
                caller,
                legacy_pin,
                ..
            } => {
                caller.clear_object_reference(object_id);
                for arg in args {
                    arg.clear_object_reference(object_id);
                }
                if let Some(legacy_pin) = legacy_pin {
                    let mut legacy_pin = legacy_pin.borrow_mut();
                    legacy_pin.root.clear_object_reference(object_id);
                    for arg in &mut legacy_pin.args {
                        arg.clear_object_reference(object_id);
                    }
                    if let Some(resolved) = legacy_pin.resolved.as_mut() {
                        resolved.clear_object_reference(object_id);
                    }
                }
            }
        }
    }

    pub(crate) fn cell(value: ValueCell) -> Self {
        let identity = TrackedValue::runtime_identity(&value.borrow());
        Self::Cell {
            value,
            identity: Some(Rc::new(RefCell::new(identity))),
        }
    }

    fn tracked_cell(value: ValueCell, identity: RawIdentityCell) -> Self {
        Self::Cell {
            value,
            identity: Some(identity),
        }
    }

    fn detach_container_identity_if_shared(&self) {
        let (identity, root, segments) = match self {
            Self::Cell {
                value,
                identity: Some(identity),
            } => (identity.clone(), value.clone(), &[][..]),
            Self::Path {
                root,
                root_identity,
                segments,
                legacy_pin,
            } => {
                if resolved_legacy_path_value(legacy_pin).is_some() {
                    return;
                }
                let Some(identity) = root_identity.clone() else {
                    return;
                };
                (identity, root.clone(), segments.as_slice())
            }
            _ => return,
        };
        detach_container_identity_at_path(&root, &identity, segments);
    }

    fn prepare_legacy_path_step(&self) -> Result<(), RuntimeError> {
        let Self::Path {
            root,
            root_identity,
            segments,
            legacy_pin: Some(legacy_pin),
        } = self
        else {
            return Ok(());
        };
        let Some((last, parent_segments)) = segments.split_last() else {
            return Ok(());
        };
        if legacy_pin.borrow().resolved.is_some() {
            return Ok(());
        }
        if let Some(identity) = root_identity {
            detach_container_identity_at_path(root, identity, parent_segments);
        }

        let parent = read_path(&root.borrow(), parent_segments)?;
        let needs_slot = match (&parent, last) {
            (Value::Array(elements), PathSegment::Index(index)) => {
                let index = array_index(index)?;
                if index >= ARRAY_MAX_SIZE {
                    return Err(RuntimeError::new("out of memory"));
                }
                index >= elements.len()
            }
            (Value::Proplist(entries), PathSegment::Property(property)) => {
                entries.get(property).is_none()
            }
            (Value::Proplist(entries), PathSegment::Index(key)) => entries.get_key(key).is_none(),
            (other, PathSegment::Property(_)) => {
                return Err(RuntimeError::new(format!(
                    "map access with .: map expected, but got \"{}\"!",
                    other.type_name()
                )))
            }
            (other, PathSegment::Index(_)) => {
                return Err(RuntimeError::new(format!(
                    "indexed access: can't access {} by index!",
                    other.type_name()
                )))
            }
        };
        if needs_slot {
            self.write(Value::Nil)?;
        }
        Ok(())
    }

    fn prepare_legacy_host_path_step(&self) -> Result<(), RuntimeError> {
        let Self::HostPath {
            segments,
            legacy_pin: Some(legacy_pin),
            ..
        } = self
        else {
            return Ok(());
        };
        let Some((last, parent_segments)) = segments.split_last() else {
            return Ok(());
        };
        if legacy_pin.borrow().resolved.is_some() {
            return Ok(());
        }

        let parent = {
            let legacy_pin = legacy_pin.borrow();
            read_path(&legacy_pin.root.value, parent_segments)?
        };
        let needs_slot = match (&parent, last) {
            (Value::Array(elements), PathSegment::Index(index)) => {
                let index = array_index(index)?;
                if index >= ARRAY_MAX_SIZE {
                    return Err(RuntimeError::new("out of memory"));
                }
                index >= elements.len()
            }
            (Value::Proplist(entries), PathSegment::Property(property)) => {
                entries.get(property).is_none()
            }
            (Value::Proplist(entries), PathSegment::Index(key)) => entries.get_key(key).is_none(),
            (other, PathSegment::Property(_)) => {
                return Err(RuntimeError::new(format!(
                    "map access with .: map expected, but got \"{}\"!",
                    other.type_name()
                )))
            }
            (other, PathSegment::Index(_)) => {
                return Err(RuntimeError::new(format!(
                    "indexed access: can't access {} by index!",
                    other.type_name()
                )))
            }
        };
        if needs_slot {
            self.write(Value::Nil)?;
        }
        Ok(())
    }

    fn read(&self) -> Result<Value, RuntimeError> {
        self.read_tracked().map(|tracked| tracked.value)
    }

    fn resolved_legacy_value(&self) -> Option<TrackedValue> {
        match self {
            Self::Path { legacy_pin, .. } => resolved_legacy_path_value(legacy_pin),
            Self::HostPath { legacy_pin, .. } => resolved_legacy_host_path_value(legacy_pin),
            _ => None,
        }
    }

    fn read_tracked(&self) -> Result<TrackedValue, RuntimeError> {
        match self {
            LValueRef::Cell { value, identity } => {
                let identity = legacy_identity_for_value_copy(
                    value,
                    &[],
                    identity
                        .as_ref()
                        .and_then(|identity| identity.borrow().clone()),
                );
                Ok(TrackedValue {
                    value: value.borrow().clone(),
                    identity,
                })
            }
            LValueRef::Path {
                root,
                root_identity,
                segments,
                legacy_pin,
            } => {
                if let Some(resolved) = resolved_legacy_path_value(legacy_pin) {
                    return Ok(resolved);
                }
                let value = read_path(&root.borrow(), segments)?;
                let identity = root_identity.as_ref().and_then(|identity| {
                    identity
                        .borrow()
                        .as_ref()
                        .and_then(|identity| identity.identity_at_path(segments))
                });
                let identity = legacy_identity_for_value_copy(root, segments, identity);
                Ok(TrackedValue { value, identity })
            }
            LValueRef::HostPath {
                function,
                args,
                caller,
                global_call_context_hook,
                segments,
                legacy_pin,
            } => {
                if let Some(resolved) = resolved_legacy_host_path_value(legacy_pin) {
                    return Ok(resolved);
                }
                if let Some(legacy_pin) = legacy_pin {
                    let legacy_pin = legacy_pin.borrow();
                    return tracked_value_at_path(&legacy_pin.root, &legacy_pin.segments);
                }
                let _context = GlobalCallContextGuard::enter(global_call_context_hook.as_ref());
                let _guard = CallerContextGuard::enter(Some(caller.clone()));
                read_path(&function(args)?, segments).map(TrackedValue::runtime)
            }
        }
    }

    fn write(&self, value: Value) -> Result<(), RuntimeError> {
        self.write_tracked(TrackedValue::runtime(value))
    }

    fn write_tracked(&self, tracked: TrackedValue) -> Result<(), RuntimeError> {
        match self {
            LValueRef::Cell { value, identity } => {
                if c4_set_copy_is_zero_id(&tracked.value) && c4_set_copy_is_zero_id(&value.borrow())
                {
                    return Ok(());
                }
                let tracked = tracked.set_copy();
                let preserves_container = identity
                    .as_ref()
                    .and_then(|identity| {
                        identity
                            .borrow()
                            .as_ref()
                            .zip(tracked.identity.as_ref())
                            .map(|(current, replacement)| current == replacement)
                    })
                    .unwrap_or(false);
                notify_legacy_path_pins_before_cell_write(
                    value,
                    identity.as_ref(),
                    preserves_container,
                );
                *value.borrow_mut() = tracked.value;
                register_active_object_reference_cell(value);
                if let Some(identity) = identity {
                    *identity.borrow_mut() = tracked.identity;
                }
                Ok(())
            }
            LValueRef::Path {
                root,
                root_identity,
                segments,
                legacy_pin,
            } => {
                if let Some(resolved) = resolved_legacy_path_value(legacy_pin) {
                    return Err(RuntimeError::new(format!(
                        "resolved container reference is a {}, not an lvalue",
                        resolved.value.type_name()
                    )));
                }
                if c4_set_copy_is_zero_id(&tracked.value) && c4_set_copy_is_zero_id(&self.read()?) {
                    return Ok(());
                }
                let TrackedValue {
                    value,
                    identity: replacement_identity,
                } = tracked;
                let preserves_container = root_identity
                    .as_ref()
                    .and_then(|identity| {
                        identity
                            .borrow()
                            .as_ref()
                            .and_then(|identity| identity.identity_at_path(segments))
                    })
                    .as_ref()
                    .zip(replacement_identity.as_ref())
                    .is_some_and(|(current, replacement)| current == replacement);
                notify_legacy_path_pins_before_path_write(root, segments, preserves_container);
                let mut reference_delta = ACTIVE_OBJECT_REFERENCE_INDEX
                    .with(|index| index.borrow().is_some().then(ObjectReferenceDelta::default));
                write_path_recording(
                    &mut root.borrow_mut(),
                    segments,
                    value,
                    reference_delta.as_mut(),
                )?;
                if let Some(reference_delta) = reference_delta {
                    apply_active_object_reference_delta(root, reference_delta);
                }
                if let Some(identity) = root_identity {
                    // Move the old identity out rather than cloning it:
                    // `RawIdentity` is a recursive tree, so a clone here costs
                    // the size of the whole structure on *every* element write,
                    // which is what made building an n x n array cubic
                    // (clonk-org/clonk-rs#759). `after_path_write` only reads
                    // it, and the cell is written back immediately.
                    let current = identity.borrow_mut().take();
                    let next_identity = RawIdentity::after_path_write(
                        current.as_ref(),
                        &root.borrow(),
                        segments,
                        replacement_identity,
                    );
                    *identity.borrow_mut() = next_identity;
                }
                Ok(())
            }
            LValueRef::HostPath {
                function,
                args,
                caller,
                global_call_context_hook,
                segments,
                legacy_pin,
            } => {
                if let Some(resolved) = resolved_legacy_host_path_value(legacy_pin) {
                    return Err(RuntimeError::new(format!(
                        "resolved container reference is a {}, not an lvalue",
                        resolved.value.type_name()
                    )));
                }
                if c4_set_copy_is_zero_id(&tracked.value) && c4_set_copy_is_zero_id(&self.read()?) {
                    return Ok(());
                }
                let _context = GlobalCallContextGuard::enter(global_call_context_hook.as_ref());
                let _guard = CallerContextGuard::enter(Some(caller.clone()));
                let replacement = if segments.is_empty() {
                    tracked.set_copy().value
                } else {
                    let mut root = if let Some(legacy_pin) = legacy_pin {
                        legacy_pin.borrow().root.value.clone()
                    } else {
                        function(args)?
                    };
                    write_path(&mut root, segments, tracked.value)?;
                    root
                };
                notify_legacy_host_path_pins_before_write(args, segments);
                let mut write_args = args.clone();
                write_args.truncate(3);
                write_args.resize(3, Value::Nil);
                write_args.push(replacement.clone());
                function(&write_args)?;
                update_legacy_host_path_pins_after_write(args, replacement);
                Ok(())
            }
        }
    }

    fn append(&self, segment: PathSegment) -> Result<Self, RuntimeError> {
        let appended = match self {
            LValueRef::Cell { value, identity } => {
                let segments = vec![segment];
                LValueRef::Path {
                    root: value.clone(),
                    root_identity: identity.clone(),
                    legacy_pin: legacy_path_pin_for_append(value, identity, &segments),
                    segments,
                }
            }
            LValueRef::Path {
                root,
                root_identity,
                segments,
                legacy_pin,
            } => {
                if let Some(resolved) = resolved_legacy_path_value(legacy_pin) {
                    // Once container destruction resolves a C4Value ref, a
                    // subsequent `_R` traversal operates on that stack value
                    // itself. Replacing it with a child ref releases its sole
                    // container owner, so the child immediately resolves to a
                    // value as well (C4Value.cpp:217-227).
                    let resolved = resolved_legacy_path_step(&resolved, &segment)?;
                    let root = value_cell(resolved.value.clone());
                    let root_identity = Some(Rc::new(RefCell::new(resolved.identity.clone())));
                    let legacy_pin = Some(Rc::new(RefCell::new(LegacyPathPin {
                        root: root.clone(),
                        root_identity: root_identity.clone(),
                        segments: Vec::new(),
                        resolved: Some(resolved),
                    })));
                    LValueRef::Path {
                        root,
                        root_identity,
                        segments: Vec::new(),
                        legacy_pin,
                    }
                } else {
                    let mut segments = segments.clone();
                    segments.push(segment);
                    LValueRef::Path {
                        root: root.clone(),
                        root_identity: root_identity.clone(),
                        legacy_pin: legacy_path_pin_for_append(root, root_identity, &segments),
                        segments,
                    }
                }
            }
            LValueRef::HostPath {
                function,
                args,
                caller,
                global_call_context_hook,
                segments,
                legacy_pin,
            } => {
                if let Some(resolved) = resolved_legacy_host_path_value(legacy_pin) {
                    let resolved = resolved_legacy_path_step(&resolved, &segment)?;
                    let root = value_cell(resolved.value.clone());
                    let root_identity = Some(Rc::new(RefCell::new(resolved.identity.clone())));
                    let legacy_pin = Some(Rc::new(RefCell::new(LegacyPathPin {
                        root: root.clone(),
                        root_identity: root_identity.clone(),
                        segments: Vec::new(),
                        resolved: Some(resolved),
                    })));
                    return Ok(LValueRef::Path {
                        root,
                        root_identity,
                        segments: Vec::new(),
                        legacy_pin,
                    });
                }
                let mut segments = segments.clone();
                segments.push(segment);
                let legacy_pin = legacy_host_path_pin_for_append(
                    function,
                    args,
                    caller,
                    global_call_context_hook,
                    legacy_pin,
                    &segments,
                )?;
                LValueRef::HostPath {
                    function: function.clone(),
                    args: args.clone(),
                    caller: caller.clone(),
                    global_call_context_hook: global_call_context_hook.clone(),
                    segments,
                    legacy_pin,
                }
            }
        };
        appended.prepare_legacy_path_step()?;
        appended.prepare_legacy_host_path_step()?;
        Ok(appended)
    }
}

struct ActiveObjectReferenceCellsGuard {
    outermost: bool,
}

impl ActiveObjectReferenceCellsGuard {
    fn enter_frame() -> Self {
        let outermost = ACTIVE_OBJECT_REFERENCE_DEPTH.with(|depth| {
            let outermost = depth.get() == 0;
            depth.set(depth.get() + 1);
            outermost
        });
        if outermost {
            ACTIVE_OBJECT_REFERENCE_SWEEPS.with(|sweeps| sweeps.borrow_mut().clear());
            ACTIVE_OBJECT_REFERENCE_INDEX
                .with(|index| *index.borrow_mut() = Some(ActiveObjectReferenceIndex::default()));
            ACTIVE_OBJECT_REFERENCE_TABLES
                .with(|tables| *tables.borrow_mut() = Some(ActiveObjectReferenceTables::default()));
        }
        ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            index
                .borrow_mut()
                .as_mut()
                .expect("the reference index was installed")
                .enter_frame();
        });
        Self { outermost }
    }

    #[cfg(test)]
    fn enter(env: &Environment, vm: &Vm<'_>) -> Self {
        let guard = Self::enter_frame();
        guard.register_environment(env, vm);
        guard
    }

    fn register_environment(&self, env: &Environment, vm: &Vm<'_>) {
        let cells = env.object_reference_cells(vm);
        register_shared_object_reference_cells(cells.into_iter().filter_map(|cell| cell.upgrade()));
    }
}

impl Drop for ActiveObjectReferenceCellsGuard {
    fn drop(&mut self) {
        let depth = ACTIVE_OBJECT_REFERENCE_DEPTH.with(Cell::get);
        ACTIVE_OBJECT_REFERENCE_TABLES.with(|tables| {
            if let Some(tables) = tables.borrow_mut().as_mut() {
                tables.leave_depth(depth);
            }
        });
        ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            index
                .borrow_mut()
                .as_mut()
                .expect("the reference index remains installed through frame exit")
                .leave_frame();
        });
        ACTIVE_OBJECT_REFERENCE_DEPTH.with(|active_depth| {
            debug_assert_eq!(active_depth.get(), depth);
            active_depth.set(depth.saturating_sub(1));
        });
        if self.outermost {
            ACTIVE_OBJECT_REFERENCE_SWEEPS.with(|sweeps| sweeps.borrow_mut().clear());
            ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| *index.borrow_mut() = None);
            ACTIVE_OBJECT_REFERENCE_TABLES.with(|tables| *tables.borrow_mut() = None);
        }
    }
}

#[derive(Clone)]
pub(crate) enum PathSegment {
    Property(String),
    Index(Value),
}

/// C++ container references retain the concrete element reached by
/// `AB_ARRAYA_R`. Rust paths normally re-resolve from their root cell, so a
/// short-lived pin records when replacement destroys that element. C++ then
/// resolves the stack reference to an ordinary value; `resolved` mirrors that
/// transition instead of retargeting the path into the replacement container.
pub(crate) struct LegacyPathPin {
    root: ValueCell,
    root_identity: Option<RawIdentityCell>,
    segments: Vec<PathSegment>,
    resolved: Option<TrackedValue>,
}

pub(crate) struct LegacyHostPathPin {
    args: Vec<Value>,
    root: TrackedValue,
    segments: Vec<PathSegment>,
    resolved: Option<TrackedValue>,
}

thread_local! {
    static LEGACY_PATH_PIN_SCOPE_DEPTH: Cell<usize> = const { Cell::new(0) };
    static LEGACY_PATH_PIN_CREATION_DEPTH: Cell<usize> = const { Cell::new(0) };
    static LEGACY_PATH_PINS: RefCell<Vec<Weak<RefCell<LegacyPathPin>>>> =
        const { RefCell::new(Vec::new()) };
    static LEGACY_HOST_PATH_PINS: RefCell<Vec<Weak<RefCell<LegacyHostPathPin>>>> =
        const { RefCell::new(Vec::new()) };
}

struct LegacyPathPinCreationGuard {
    previous: usize,
}

struct LegacyPathPinRegistryGuard {
    previous: usize,
}

impl LegacyPathPinCreationGuard {
    fn enter() -> Self {
        LEGACY_PATH_PIN_CREATION_DEPTH.with(|depth| {
            let previous = depth.get();
            depth.set(previous + 1);
            Self { previous }
        })
    }

    fn suspend() -> Self {
        LEGACY_PATH_PIN_CREATION_DEPTH.with(|depth| {
            let previous = depth.replace(0);
            Self { previous }
        })
    }
}

impl LegacyPathPinRegistryGuard {
    fn enter() -> Self {
        LEGACY_PATH_PIN_SCOPE_DEPTH.with(|depth| {
            let previous = depth.get();
            depth.set(previous + 1);
            Self { previous }
        })
    }
}

impl Drop for LegacyPathPinCreationGuard {
    fn drop(&mut self) {
        LEGACY_PATH_PIN_CREATION_DEPTH.with(|depth| depth.set(self.previous));
    }
}

impl Drop for LegacyPathPinRegistryGuard {
    fn drop(&mut self) {
        LEGACY_PATH_PIN_SCOPE_DEPTH.with(|depth| depth.set(self.previous));
        LEGACY_PATH_PINS.with(|pins| {
            pins.borrow_mut().retain(|pin| pin.strong_count() > 0);
        });
        LEGACY_HOST_PATH_PINS.with(|pins| {
            pins.borrow_mut().retain(|pin| pin.strong_count() > 0);
        });
    }
}

fn legacy_path_pin_scope_active() -> bool {
    LEGACY_PATH_PIN_SCOPE_DEPTH.with(|depth| depth.get() > 0)
}

fn legacy_path_pin_creation_active() -> bool {
    LEGACY_PATH_PIN_CREATION_DEPTH.with(|depth| depth.get() > 0)
}

fn live_legacy_path_pins() -> Vec<Rc<RefCell<LegacyPathPin>>> {
    LEGACY_PATH_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        let live = pins.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        pins.retain(|pin| pin.strong_count() > 0);
        live
    })
}

fn live_legacy_host_path_pins() -> Vec<Rc<RefCell<LegacyHostPathPin>>> {
    LEGACY_HOST_PATH_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        let live = pins.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        pins.retain(|pin| pin.strong_count() > 0);
        live
    })
}

fn legacy_path_pin_for_append(
    root: &ValueCell,
    root_identity: &Option<RawIdentityCell>,
    segments: &[PathSegment],
) -> Option<Rc<RefCell<LegacyPathPin>>> {
    if !legacy_path_pin_creation_active() {
        return None;
    }
    let pin = Rc::new(RefCell::new(LegacyPathPin {
        root: root.clone(),
        root_identity: root_identity.clone(),
        segments: segments.to_vec(),
        resolved: None,
    }));
    LEGACY_PATH_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        pins.retain(|pin| pin.strong_count() > 0);
        pins.push(Rc::downgrade(&pin));
    });
    Some(pin)
}

fn resolved_legacy_path_value(pin: &Option<Rc<RefCell<LegacyPathPin>>>) -> Option<TrackedValue> {
    pin.as_ref().and_then(|pin| pin.borrow().resolved.clone())
}

fn legacy_host_path_pin_for_append(
    function: &HostFunction,
    args: &[Value],
    caller: &ScriptCallerContext,
    global_call_context_hook: &Option<GlobalCallContextHook>,
    previous: &Option<Rc<RefCell<LegacyHostPathPin>>>,
    segments: &[PathSegment],
) -> Result<Option<Rc<RefCell<LegacyHostPathPin>>>, RuntimeError> {
    if !legacy_path_pin_creation_active() {
        return Ok(None);
    }
    let root = if let Some(previous) = previous {
        previous.borrow().root.clone()
    } else {
        let _context = GlobalCallContextGuard::enter(global_call_context_hook.as_ref());
        let _guard = CallerContextGuard::enter(Some(caller.clone()));
        TrackedValue::runtime(function(args)?)
    };
    let pin = Rc::new(RefCell::new(LegacyHostPathPin {
        args: args.to_vec(),
        root,
        segments: segments.to_vec(),
        resolved: None,
    }));
    LEGACY_HOST_PATH_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        pins.retain(|pin| pin.strong_count() > 0);
        pins.push(Rc::downgrade(&pin));
    });
    Ok(Some(pin))
}

fn resolved_legacy_host_path_value(
    pin: &Option<Rc<RefCell<LegacyHostPathPin>>>,
) -> Option<TrackedValue> {
    pin.as_ref().and_then(|pin| pin.borrow().resolved.clone())
}

fn tracked_value_at_path(
    root: &TrackedValue,
    segments: &[PathSegment],
) -> Result<TrackedValue, RuntimeError> {
    let value = read_path(&root.value, segments)?;
    let identity = root
        .identity
        .as_ref()
        .and_then(|identity| identity.identity_at_path(segments));
    Ok(TrackedValue { value, identity })
}

fn host_path_address_args(args: &[Value]) -> [Value; 3] {
    let integer = |index| Value::Int(args.get(index).and_then(Value::as_c4_int).unwrap_or(0));
    let target = match args.get(1) {
        Some(Value::Object(id)) => Value::Object(*id),
        None | Some(Value::Nil | Value::Int(0) | Value::Bool(false) | Value::RawBool(0)) => {
            Value::Object(0)
        }
        Some(value) => value.clone(),
    };
    [integer(0), target, integer(2)]
}

fn legacy_host_path_roots_match(pin: &LegacyHostPathPin, args: &[Value]) -> bool {
    host_path_address_args(&pin.args) == host_path_address_args(args)
}

fn notify_legacy_host_path_pins_before_write(args: &[Value], segments: &[PathSegment]) {
    if !legacy_path_pin_scope_active() {
        return;
    }
    let pins = live_legacy_host_path_pins()
        .into_iter()
        .filter(|pin| {
            let pin = pin.borrow();
            pin.resolved.is_none()
                && legacy_host_path_roots_match(&pin, args)
                && path_is_strict_prefix(&pin.root.value, segments, &pin.segments)
        })
        .collect::<Vec<_>>();
    for pin in pins {
        let resolved = {
            let pin = pin.borrow();
            tracked_value_at_path(&pin.root, &pin.segments)
        };
        if let Ok(resolved) = resolved {
            pin.borrow_mut().resolved = Some(resolved);
        }
    }
}

fn update_legacy_host_path_pins_after_write(args: &[Value], replacement: Value) {
    let replacement = TrackedValue::runtime(replacement);
    for pin in live_legacy_host_path_pins() {
        let mut pin = pin.borrow_mut();
        if pin.resolved.is_none() && legacy_host_path_roots_match(&pin, args) {
            pin.root = replacement.clone();
        }
    }
}

fn resolved_legacy_path_step(
    resolved: &TrackedValue,
    segment: &PathSegment,
) -> Result<TrackedValue, RuntimeError> {
    if let (Value::Array(_), PathSegment::Index(index)) = (&resolved.value, segment) {
        if array_index(index)? >= ARRAY_MAX_SIZE {
            return Err(RuntimeError::new("out of memory"));
        }
    }
    let value = read_path(&resolved.value, std::slice::from_ref(segment))?;
    Ok(TrackedValue {
        value,
        identity: resolved.identity_at(segment),
    })
}

fn resolve_legacy_path_pin(pin: &Rc<RefCell<LegacyPathPin>>) {
    let (root, root_identity, segments) = {
        let pin = pin.borrow();
        if pin.resolved.is_some() {
            return;
        }
        (
            pin.root.clone(),
            pin.root_identity.clone(),
            pin.segments.clone(),
        )
    };
    let Ok(value) = read_path(&root.borrow(), &segments) else {
        return;
    };
    let identity = root_identity.as_ref().and_then(|identity| {
        identity
            .borrow()
            .as_ref()
            .and_then(|identity| identity.identity_at_path(&segments))
    });
    let identity = legacy_identity_for_value_copy(&root, &segments, identity);
    pin.borrow_mut().resolved = Some(TrackedValue { value, identity });
}

fn detach_container_identity_at_path(
    root: &ValueCell,
    identity: &RawIdentityCell,
    segments: &[PathSegment],
) {
    let mut identity = identity.borrow_mut();
    let Some(RawIdentity::Heap(heap)) = identity
        .as_ref()
        .and_then(|identity| identity.identity_ref_at_path(segments))
    else {
        return;
    };
    if Rc::strong_count(heap) <= 1 {
        return;
    }
    let detached = RawIdentity::Heap(Rc::new(clone_heap_identity_for_container_copy(
        root, segments, heap,
    )));
    *identity = if segments.is_empty() {
        Some(detached)
    } else {
        RawIdentity::after_path_write(identity.as_ref(), &root.borrow(), segments, Some(detached))
    };
}

fn path_segments_target_same(container: &Value, left: &PathSegment, right: &PathSegment) -> bool {
    match container {
        Value::Array(_) => match (left, right) {
            (PathSegment::Index(left), PathSegment::Index(right)) => {
                matches!(
                    (array_index(left), array_index(right)),
                    (Ok(left), Ok(right)) if left == right
                )
            }
            _ => false,
        },
        Value::Proplist(_) => match (left, right) {
            (PathSegment::Property(left), PathSegment::Property(right)) => {
                c4_strings_equal(left, right)
            }
            (PathSegment::Property(left), PathSegment::Index(Value::String(right)))
            | (PathSegment::Index(Value::String(right)), PathSegment::Property(left)) => {
                c4_strings_equal(left, right)
            }
            (PathSegment::Index(left), PathSegment::Index(right)) => left == right,
            _ => false,
        },
        _ => false,
    }
}

fn path_child<'a>(container: &'a Value, segment: &PathSegment) -> Option<&'a Value> {
    match (container, segment) {
        (Value::Array(elements), PathSegment::Index(index)) => {
            elements.get(array_index(index).ok()?)
        }
        (Value::Proplist(entries), PathSegment::Property(property)) => entries.get(property),
        (Value::Proplist(entries), PathSegment::Index(key)) => entries.get_key(key),
        _ => None,
    }
}

fn path_is_strict_prefix(root: &Value, prefix: &[PathSegment], path: &[PathSegment]) -> bool {
    if prefix.len() >= path.len() {
        return false;
    }
    let mut container = root;
    for (prefix_segment, path_segment) in prefix.iter().zip(path) {
        if !path_segments_target_same(container, prefix_segment, path_segment) {
            return false;
        }
        let Some(child) = path_child(container, path_segment) else {
            return false;
        };
        container = child;
    }
    true
}

fn legacy_container_has_element_reference(
    root: &ValueCell,
    root_value: &Value,
    segments: &[PathSegment],
) -> bool {
    live_legacy_path_pins().into_iter().any(|pin| {
        let pin = pin.borrow();
        pin.resolved.is_none()
            && Rc::ptr_eq(&pin.root, root)
            && pin.segments.len() == segments.len() + 1
            && path_is_strict_prefix(root_value, segments, &pin.segments)
    })
}

fn clone_c4value_identity_for_container_copy(
    root: &ValueCell,
    root_value: &Value,
    segments: &[PathSegment],
    identity: Option<RawIdentity>,
) -> Option<RawIdentity> {
    let heap = match identity {
        Some(RawIdentity::Heap(heap)) => heap,
        identity => return identity,
    };
    if !legacy_container_has_element_reference(root, root_value, segments) {
        return Some(RawIdentity::Heap(heap));
    }
    Some(RawIdentity::Heap(Rc::new(
        clone_heap_identity_for_container_copy(root, segments, &heap),
    )))
}

fn clone_heap_identity_for_container_copy(
    root: &ValueCell,
    segments: &[PathSegment],
    heap: &HeapIdentity,
) -> HeapIdentity {
    let root_value = root.borrow();
    match heap {
        HeapIdentity::Opaque => HeapIdentity::Opaque,
        HeapIdentity::Array(identities) => HeapIdentity::Array(
            identities
                .iter()
                .enumerate()
                .map(|(index, identity)| {
                    let mut child_segments = segments.to_vec();
                    child_segments.push(PathSegment::Index(Value::Int(
                        i32::try_from(index).expect("array identity index fits C4 int"),
                    )));
                    clone_c4value_identity_for_container_copy(
                        root,
                        &root_value,
                        &child_segments,
                        identity.clone(),
                    )
                })
                .collect(),
        ),
        HeapIdentity::Proplist(identities) => HeapIdentity::Proplist(
            identities
                .iter()
                .map(|(key, identity)| {
                    let mut child_segments = segments.to_vec();
                    child_segments.push(PathSegment::Index(key.clone()));
                    (
                        key.clone(),
                        clone_c4value_identity_for_container_copy(
                            root,
                            &root_value,
                            &child_segments,
                            identity.clone(),
                        ),
                    )
                })
                .collect(),
        ),
    }
}

/// Copying a C++ array/map while one of its elements is referenced does not
/// share that container: `IncRef` clones whenever `elementReferenceCount` is
/// nonzero. Raw identities model that copy-on-write distinction for equality
/// and later mutation. A live pin targets an element of `segments` exactly
/// when its path is one segment longer and has the same prefix.
fn legacy_identity_for_value_copy(
    root: &ValueCell,
    segments: &[PathSegment],
    identity: Option<RawIdentity>,
) -> Option<RawIdentity> {
    if !legacy_path_pin_scope_active() {
        return identity;
    }
    let heap = match identity {
        Some(RawIdentity::Heap(heap)) => heap,
        identity => return identity,
    };
    let has_element_reference =
        legacy_container_has_element_reference(root, &root.borrow(), segments);
    if has_element_reference {
        Some(RawIdentity::Heap(Rc::new(
            clone_heap_identity_for_container_copy(root, segments, &heap),
        )))
    } else {
        Some(RawIdentity::Heap(heap))
    }
}

fn notify_legacy_path_pins_before_cell_write(
    root: &ValueCell,
    _root_identity: Option<&RawIdentityCell>,
    preserves_container: bool,
) {
    if preserves_container || !legacy_path_pin_scope_active() {
        return;
    }
    let pins = live_legacy_path_pins()
        .into_iter()
        .filter(|pin| {
            let pin = pin.borrow();
            pin.resolved.is_none() && Rc::ptr_eq(&pin.root, root)
        })
        .collect::<Vec<_>>();
    for pin in pins {
        resolve_legacy_path_pin(&pin);
    }
}

fn notify_legacy_path_pins_before_path_write(
    root: &ValueCell,
    segments: &[PathSegment],
    preserves_container: bool,
) {
    if preserves_container || !legacy_path_pin_scope_active() {
        return;
    }
    let pins = {
        let root_value = root.borrow();
        live_legacy_path_pins()
            .into_iter()
            .filter(|pin| {
                let pin = pin.borrow();
                pin.resolved.is_none()
                    && Rc::ptr_eq(&pin.root, root)
                    && path_is_strict_prefix(&root_value, segments, &pin.segments)
            })
            .collect::<Vec<_>>()
    };
    for pin in pins {
        resolve_legacy_path_pin(&pin);
    }
}

fn array_index(index: &Value) -> Result<usize, RuntimeError> {
    index
        .as_c4_int()
        .map(|index| index.max(0) as usize)
        .ok_or_else(|| {
            RuntimeError::new(format!(
                "array access: can not convert \"{}\" to int",
                index.type_name()
            ))
        })
}

/// AB_ARRAYA_R/V's string branch (C4AulExec.cpp:923-947). Classic strings
/// are byte buffers. High native bytes use the reversible private-use
/// representation defined in `value` so indexing remains exact.
fn string_index(text: &str, index: &Value) -> Result<Value, RuntimeError> {
    let index = index.as_c4_int().ok_or_else(|| {
        RuntimeError::new(format!(
            "indexed string access: index of type {}, int expected!",
            index.type_name()
        ))
    })?;
    let bytes = c4_string_bytes(text);
    let len = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    let mut index = i64::from(index);
    if index < 0 {
        index += len;
    }
    let Some(byte) = usize::try_from(index)
        .ok()
        .and_then(|index| bytes.get(index))
    else {
        return Ok(Value::Nil);
    };
    Ok(Value::String(c4_string_from_bytes(&[*byte]).into()))
}

fn read_path(value: &Value, segments: &[PathSegment]) -> Result<Value, RuntimeError> {
    // Walk by reference and clone only what is actually returned. Cloning the
    // root up front cost the size of the whole container on every read, and a
    // read sits on the element-assignment path, so building an array was
    // quadratic in its length (clonk-org/clonk-rs#759).
    //
    // Two steps cannot be followed by reference -- indexing a string
    // manufactures a value, and a missing element reads as nil -- so each
    // continues the walk over the produced value instead.
    let mut current: &Value = value;
    for (position, segment) in segments.iter().enumerate() {
        let rest = &segments[position + 1..];
        let child: Option<&Value> = match (segment, current) {
            (PathSegment::Property(property), Value::Proplist(entries)) => entries.get(property),
            (PathSegment::Property(property), other) => {
                return Err(RuntimeError::new(format!(
                    "cannot access property '{property}' on value of type {}",
                    other.type_name()
                )))
            }
            (PathSegment::Index(index), Value::Array(elements)) => {
                elements.get(array_index(index)?)
            }
            (PathSegment::Index(index), Value::String(text)) => {
                return read_path(&string_index(text, index)?, rest)
            }
            (PathSegment::Index(key), Value::Proplist(entries)) => entries.get_key(key),
            (PathSegment::Index(_), other) => {
                return Err(RuntimeError::new(format!(
                    "cannot index into value of type {}",
                    other.type_name()
                )))
            }
        };
        match child {
            Some(child) => current = child,
            None => return read_path(&Value::Nil, rest),
        }
    }
    Ok(current.clone())
}

fn write_path(
    value: &mut Value,
    segments: &[PathSegment],
    new_value: Value,
) -> Result<(), RuntimeError> {
    write_path_recording(value, segments, new_value, None)
}

fn write_path_recording(
    value: &mut Value,
    segments: &[PathSegment],
    new_value: Value,
    reference_delta: Option<&mut ObjectReferenceDelta>,
) -> Result<(), RuntimeError> {
    let Some((segment, rest)) = segments.split_first() else {
        let same_destination = c4_set_copy_is_zero_id(&new_value) && c4_set_copy_is_zero_id(value);
        let replacement = c4_set_copy_value_into(new_value, same_destination);
        if let Some(delta) = reference_delta {
            delta.remove_value(value);
            delta.add_value(&replacement);
        }
        *value = replacement;
        return Ok(());
    };

    match (value, segment) {
        (Value::Proplist(entries), PathSegment::Property(property)) => {
            if rest.is_empty() {
                c4_map_assign_property_set_recording(
                    entries,
                    property.clone(),
                    new_value,
                    reference_delta,
                );
                Ok(())
            } else {
                let Some(next) = entries.get_mut(property) else {
                    return Err(RuntimeError::new(format!(
                        "cannot access property '{property}' on nil"
                    )));
                };
                write_path_recording(next, rest, new_value, reference_delta)
            }
        }
        (other, PathSegment::Property(property)) => Err(RuntimeError::new(format!(
            "cannot assign property '{property}' on value of type {}",
            other.type_name()
        ))),
        (Value::Array(elements), PathSegment::Index(index)) => {
            let index = array_index(index)?;
            if index >= ARRAY_MAX_SIZE {
                return Err(RuntimeError::new("out of memory"));
            }
            if index >= elements.len() {
                elements.resize(index + 1, Value::Nil);
            }
            if rest.is_empty() {
                let same_destination =
                    c4_set_copy_is_zero_id(&new_value) && c4_set_copy_is_zero_id(&elements[index]);
                let replacement = c4_set_copy_value_into(new_value, same_destination);
                if let Some(delta) = reference_delta {
                    delta.remove_value(&elements[index]);
                    delta.add_value(&replacement);
                }
                elements[index] = replacement;
                Ok(())
            } else {
                write_path_recording(&mut elements[index], rest, new_value, reference_delta)
            }
        }
        (Value::Proplist(entries), PathSegment::Index(key)) => {
            if rest.is_empty() {
                c4_map_assign_set_recording(entries, key.clone(), new_value, reference_delta);
                Ok(())
            } else {
                let Some(next) = entries.get_key_mut(key) else {
                    return Err(RuntimeError::new(format!(
                        "cannot access map key {key} on nil"
                    )));
                };
                write_path_recording(next, rest, new_value, reference_delta)
            }
        }
        (other, PathSegment::Index(_)) => Err(RuntimeError::new(format!(
            "cannot index into value of type {}",
            other.type_name()
        ))),
    }
}

#[derive(Clone)]
pub(crate) enum CallArg {
    Value(TrackedValue),
    Reference(LValueRef),
}

impl CallArg {
    fn runtime(value: Value) -> Self {
        CallArg::Value(TrackedValue::runtime(value))
    }

    fn external(value: Value) -> Self {
        // `C4AulParSet(par0, ...)` initializes each fresh slot with
        // `C4Value::Set` before C4AulFunc::Exec performs type conversion.
        CallArg::runtime(c4_set_copy_value(value))
    }

    fn read_tracked(&self) -> Result<TrackedValue, RuntimeError> {
        match self {
            CallArg::Value(tracked) => Ok(tracked.clone()),
            CallArg::Reference(reference) => reference.read_tracked(),
        }
    }

    fn read(&self) -> Result<Value, RuntimeError> {
        self.read_tracked().map(|tracked| tracked.value)
    }

    fn into_value(self) -> Result<Value, RuntimeError> {
        match self {
            CallArg::Value(tracked) => Ok(tracked.value),
            CallArg::Reference(reference) => reference.read(),
        }
    }

    fn value_slot_is_zero_id(&self) -> bool {
        matches!(self, CallArg::Value(tracked) if c4_set_copy_is_zero_id(&tracked.value))
    }

    fn value_slot_is_same_zero_id(&self, value: &Value) -> bool {
        c4_set_copy_is_zero_id(value) && self.value_slot_is_zero_id()
    }
}

fn materialize_internal_native_call_result(result: Value, args: &[CallArg]) -> Value {
    if matches!(caller_origin_strictness(), HostCallerStrictness::NoCaller) {
        return result;
    }
    let same_destination = args
        .first()
        .is_some_and(|destination| destination.value_slot_is_same_zero_id(&result));
    c4_set_copy_value_into(result, same_destination)
}

fn materialize_target_call_result(result: ReturnValue) -> ReturnValue {
    match result {
        ReturnValue::Value(tracked) => ReturnValue::Value(tracked.set_copy()),
        ReturnValue::Reference(reference) => ReturnValue::Reference(reference),
    }
}

/// Opaque argument supplied to a reference-aware native host function.
///
/// For an untyped embedding callback, a reference-aware parameter still
/// arrives here when the script expression is not an lvalue; it remains
/// readable but [`HostCallArg::is_reference`] is false and
/// [`HostCallArg::write`] returns `Ok(false)`. A typed native `C4V_pC4Value`
/// slot rejects that value before constructing `HostCallArg`.
#[derive(Clone)]
pub struct HostCallArg(CallArg);

impl HostCallArg {
    pub fn read(&self) -> Result<Value, RuntimeError> {
        self.0.read()
    }

    pub fn is_reference(&self) -> bool {
        matches!(self.0, CallArg::Reference(_))
    }

    pub fn write(&self, value: Value) -> Result<bool, RuntimeError> {
        match &self.0 {
            CallArg::Value(_) => Ok(false),
            CallArg::Reference(reference) => reference.write(value).map(|()| true),
        }
    }

    /// Read an array argument as tracked child arguments. This preserves the
    /// C4Value backing identity of strings/arrays/maps stored in the array,
    /// which native functions need for NONSTRICT/STRICT1 raw comparisons.
    pub fn array_items(&self) -> Result<Option<Vec<Self>>, RuntimeError> {
        let tracked = self.0.read_tracked()?;
        let Value::Array(values) = tracked.value else {
            return Ok(None);
        };
        let identities = match tracked.identity {
            Some(RawIdentity::Heap(identity)) => match identity.as_ref() {
                HeapIdentity::Array(identities) => Some(identities.clone()),
                _ => None,
            },
            _ => None,
        };
        Ok(Some(
            values
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    let identity = identities
                        .as_ref()
                        .and_then(|identities| identities.get(index))
                        .cloned()
                        .flatten()
                        .or_else(|| TrackedValue::runtime_identity(&value));
                    Self(CallArg::Value(TrackedValue { value, identity }))
                })
                .collect(),
        ))
    }

    /// `C4Value::Equals` for native host functions, retaining raw backing
    /// identity below STRICT2 and the asymmetric C4Value operator semantics at
    /// STRICT2 and above. `strict_level` is the numeric C4Aul strict level.
    pub fn c4_equals(&self, other: &Self, strict_level: u8) -> Result<bool, RuntimeError> {
        let left = self.0.read_tracked()?;
        let right = other.0.read_tracked()?;
        Ok(c4_values_equal(
            &left.value,
            &right.value,
            Some(strict_level),
            left.identity.as_ref(),
            right.identity.as_ref(),
        ))
    }
}

/// `C4Value::Equals` plus the backing-pointer provenance needed by its raw
/// NONSTRICT/STRICT1 branch. STRICT2 deliberately keeps the left-tag
/// asymmetry of `C4Value::operator==` (notably Bool versus C4ID), while
/// STRICT3 checks only the outer type before container content recurses
/// through that same operator.
impl Value {
    /// `C4Value::operator==` (C4Value.cpp:862-919) applied directly to two
    /// values.
    ///
    /// This is the STRICT2 arm of [`c4_values_equal`] without the backing
    /// identity the NONSTRICT/STRICT1 arm needs, so it is the operator itself
    /// rather than the script `==`. Use `HostCallArg::c4_equals` when the
    /// comparison has to honour a lower strict level's raw provenance.
    pub fn c4_operator_equals(&self, other: &Self) -> bool {
        c4_operator_equal(self, other)
    }

    /// `C4Value::Equals(MAXSTRICT)` (C4Value.cpp:823-858), used by
    /// `C4ValueHash::KeyEqual` for map lookup (C4ValueHash.h:39-44).
    pub(crate) fn c4_maxstrict_equals(&self, other: &Self) -> bool {
        c4_typed_equal(self, other)
    }
}

fn c4_values_equal(
    left: &Value,
    right: &Value,
    strict: Option<u8>,
    left_identity: Option<&RawIdentity>,
    right_identity: Option<&RawIdentity>,
) -> bool {
    match strict.unwrap_or(0) {
        0 | 1 => c4_raw_equal(left, right, left_identity, right_identity),
        2 => c4_operator_equal(left, right),
        _ => c4_typed_equal(left, right),
    }
}

fn c4_raw_scalar(value: &Value) -> Option<u64> {
    match value {
        Value::Nil => Some(0),
        // C++ zeroes the full Data union and then writes its 32-bit Int/ID
        // member, so negative integers retain a zero upper half on 64-bit.
        Value::Int(value) => Some(u64::from(*value as u32)),
        Value::Bool(value) => Some(u64::from(*value as u8)),
        Value::RawBool(value) => Some(*value as u64),
        Value::C4Id(value) => Some(crate::value::c4_id_raw(value) as u64),
        Value::Object(0) => Some(0),
        Value::Object(_) | Value::String(_) | Value::Array(_) | Value::Proplist(_) => None,
    }
}

fn c4_raw_equal(
    left: &Value,
    right: &Value,
    left_identity: Option<&RawIdentity>,
    right_identity: Option<&RawIdentity>,
) -> bool {
    let left_pointer = matches!(
        left,
        Value::String(_) | Value::Array(_) | Value::Proplist(_)
    );
    let right_pointer = matches!(
        right,
        Value::String(_) | Value::Array(_) | Value::Proplist(_)
    );
    if left_pointer || right_pointer {
        return left_identity
            .zip(right_identity)
            .is_some_and(|(left, right)| left == right);
    }
    if let (Some(left), Some(right)) = (c4_raw_scalar(left), c4_raw_scalar(right)) {
        return left == right;
    }
    // Rust object handles are stable numeric IDs rather than process pointer
    // addresses. Their observable raw identity is therefore equality of the
    // handle; unlike a C++ address, it must not be compared with script ints.
    matches!((left, right), (Value::Object(left), Value::Object(right)) if left == right)
}

fn c4_scalar_payload(value: &Value) -> Option<u64> {
    match value {
        Value::Nil => Some(0),
        Value::Int(value) => Some(u64::from(*value as u32)),
        Value::Bool(value) => Some(u64::from(*value as u8)),
        Value::RawBool(value) => Some(*value as u64),
        Value::C4Id(value) => Some(crate::value::c4_id_raw(value) as u64),
        Value::Object(0) => Some(0),
        _ => None,
    }
}

fn c4_operator_equal(left: &Value, right: &Value) -> bool {
    // Only a genuine C4V_Any short-circuits here. Native object constructors
    // and `SetObject` both canonicalize a null pointer to C4V_Any through
    // `Set` (C4Value.h:119,195; C4Value.cpp:121-143). Rust still retains
    // `Object(0)` as a low-level compatibility representation for an already
    // tagged payload, so if one is supplied directly it must take the object
    // arm below rather than being folded into Nil.
    //
    // A C4ID zero likewise retains its C4V_C4ID tag and must use the
    // asymmetric type table below (notably, neither operand order compares
    // equal to C4V_Bool(false)).
    // A right-hand C4V_Any gets no short-circuit of its own: C++ handles it
    // inside each left arm, and the object, string, array and map arms have no
    // such case, so they compare tags and report false. The scalar arms below
    // already list `Value::Nil` among their accepted right operands.
    if matches!(left, Value::Nil) {
        return c4_scalar_payload(right) == Some(0);
    }
    match left {
        // C4V_Any has Data == 0 and compares that union payload without a
        // right-tag check.
        Value::Nil => c4_scalar_payload(right) == Some(0),
        Value::Int(left) => {
            matches!(
                right,
                Value::Nil | Value::Int(_) | Value::Bool(_) | Value::RawBool(_) | Value::C4Id(_)
            ) && c4_scalar_payload(right) == Some(u64::from(*left as u32))
        }
        Value::Bool(left) => {
            matches!(
                right,
                Value::Nil | Value::Int(_) | Value::Bool(_) | Value::RawBool(_)
            ) && c4_scalar_payload(right) == Some(u64::from(*left as u8))
        }
        Value::RawBool(left) => {
            matches!(
                right,
                Value::Nil | Value::Int(_) | Value::Bool(_) | Value::RawBool(_)
            ) && c4_scalar_payload(right) == Some(*left as u64)
        }
        Value::C4Id(left) => {
            matches!(right, Value::Nil | Value::Int(_) | Value::C4Id(_))
                && c4_scalar_payload(right) == Some(crate::value::c4_id_raw(left) as u64)
        }
        Value::Object(left) => matches!(right, Value::Object(right) if left == right),
        Value::String(left) => {
            matches!(right, Value::String(right) if c4_strings_equal(left, right))
        }
        Value::Array(left) => {
            matches!(right, Value::Array(right) if c4_array_operator_equal(left, right))
        }
        Value::Proplist(left) => {
            matches!(right, Value::Proplist(right) if c4_map_operator_equal(left, right))
        }
    }
}

fn c4_typed_equal(left: &Value, right: &Value) -> bool {
    // Ordinary zero C4ID constructors/literals are canonicalized to Nil
    // before reaching this comparator. A zero C4Id variant that survives is
    // the retained C4V_C4ID tag produced by FnCnvInt2Id (which writes Type
    // directly even for zero), so STRICT3 must not fold it back to Any here.
    // Null object constructors still canonicalize to C4V_Any in C++ and keep
    // using Rust's Object(0) compatibility representation.
    let left_nil = matches!(left, Value::Nil | Value::Object(0));
    let right_nil = matches!(right, Value::Nil | Value::Object(0));
    if left_nil || right_nil {
        return left_nil && right_nil;
    }
    match (left, right) {
        (Value::Nil, Value::Nil) => true,
        (Value::Int(left), Value::Int(right)) => left == right,
        (
            left @ (Value::Bool(_) | Value::RawBool(_)),
            right @ (Value::Bool(_) | Value::RawBool(_)),
        ) => left.c4_bool_raw().map(|raw| raw != 0) == right.c4_bool_raw().map(|raw| raw != 0),
        (Value::C4Id(left), Value::C4Id(right)) => {
            crate::value::c4_id_raw(left) == crate::value::c4_id_raw(right)
        }
        (Value::Object(left), Value::Object(right)) => left == right,
        (Value::String(left), Value::String(right)) => c4_strings_equal(left, right),
        (Value::Array(left), Value::Array(right)) => c4_array_operator_equal(left, right),
        (Value::Proplist(left), Value::Proplist(right)) => c4_map_operator_equal(left, right),
        _ => false,
    }
}

fn c4_array_operator_equal(left: &[Value], right: &[Value]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| c4_operator_equal(left, right))
}

fn c4_map_operator_equal(left: &ValueMap, right: &ValueMap) -> bool {
    left.len() == right.len()
        && left.iter().all(|(left_key, left_value)| {
            right
                .get_key(left_key)
                // C4ValueHash::operator== spells this `other[key] != value`,
                // so the other map's value is the asymmetric operator lhs.
                .is_some_and(|right_value| c4_operator_equal(right_value, left_value))
        })
}

#[derive(Clone)]
enum ReturnValue {
    Value(TrackedValue),
    Reference(LValueRef),
}

impl ReturnValue {
    fn into_value(self) -> Result<Value, RuntimeError> {
        self.into_tracked().map(|tracked| tracked.value)
    }

    fn into_tracked(self) -> Result<TrackedValue, RuntimeError> {
        match self {
            ReturnValue::Value(value) => Ok(value),
            ReturnValue::Reference(reference) => reference.read_tracked(),
        }
    }

    fn as_value(&self) -> Result<Value, RuntimeError> {
        match self {
            ReturnValue::Value(tracked) => Ok(tracked.value.clone()),
            ReturnValue::Reference(reference) => reference.read(),
        }
    }

    fn into_value_on_stack(self) -> Result<Value, RuntimeError> {
        let _result_slot = ValueStackReservation::reserve(1)?;
        self.into_value()
    }

    fn into_set_tracked_on_stack(self) -> Result<TrackedValue, RuntimeError> {
        let _result_slot = ValueStackReservation::reserve(1)?;
        match self {
            Self::Value(value) => Ok(value),
            Self::Reference(reference) => reference.read_tracked().map(TrackedValue::set_copy),
        }
    }

    fn clear_object_reference_sweeps(&mut self, cursor: usize) {
        if let Self::Value(tracked) = self {
            tracked.clear_object_reference_sweeps(cursor);
        }
    }

    fn clear_object_reference(&mut self, object_id: u64) {
        match self {
            Self::Value(tracked) => tracked.clear_object_reference(object_id),
            Self::Reference(reference) => reference.clear_object_reference(object_id),
        }
    }
}

struct GlobalCallContextGuard<'a> {
    hook: Option<&'a GlobalCallContextHook>,
}

impl<'a> GlobalCallContextGuard<'a> {
    fn enter(hook: Option<&'a GlobalCallContextHook>) -> Self {
        if let Some(hook) = hook {
            hook(true);
        }
        Self { hook }
    }
}

impl Drop for GlobalCallContextGuard<'_> {
    fn drop(&mut self) {
        if let Some(hook) = self.hook {
            hook(false);
        }
    }
}

#[derive(Clone)]
pub struct Vm<'a> {
    functions: &'a FxHashMap<String, Function>,
    host_identity: ScriptHostIdentity,
    /// Destination definition name for local-function diagnostics.
    owner_definition_name: Option<Arc<str>>,
    /// `C4AulScript::ScriptName` of the DirectExec receiver. Temporary
    /// expression contexts derive their visible name from this host, never
    /// from an enclosing temporary script.
    script_name: &'a str,
    /// Receiver name for `global->eval`, whose native call context has no
    /// object or definition and therefore selects Game.Script in C++.
    game_script_name: Option<&'a str>,
    /// Native `cthr->Def` availability for callerless ordinary frames.
    definition_context: bool,
    /// Destination script strictness for `Func->Owner->Strict`. None means
    /// this bare VM has no configured base script; Some(None) is an
    /// explicitly NONSTRICT destination.
    owner_strict_level: Option<Option<u8>>,
    host_functions: &'a FxHashMap<String, RegisteredHostFunction>,
    host_reference_functions: Option<&'a FxHashMap<String, HostReferenceFunction>>,
    host_function_parameter_types: Option<&'a FxHashMap<String, Arc<[C4VType]>>>,
    var_decls: &'a [VarDecl], // Script-level variable declarations
    debugger: Option<DebuggerHooks>,
    /// Engine-registered script constants (`RegisterGlobalConstant`,
    /// C4Script.cpp:6581): consulted when an identifier matches no variable.
    constants: Option<&'a FxHashMap<String, Value>>,
    /// Engine-global script functions (System.c4g `global func`s): the
    /// resolution fallback between the own script and host functions.
    global_functions: Option<&'a FxHashMap<String, Function>>,
    /// Exact retained engine-global callback mode. Ordinary Engine::call
    /// keeps the historical own-root dispatch used by synthetic callbacks;
    /// a captured C4AulFunc pointer skips unnamed own global links.
    exact_global_link_lookup: bool,
    /// One-shot parameter conversion policy for a host-selected script entry.
    /// A scripted C4Effect callback consumes the warning-only exception at
    /// its immediate function; all nested calls return to ordinary strict
    /// conversion.
    entry_parameter_conversion: Cell<ParameterConversionFailurePolicy>,
    /// The object context the call runs on, returned by an unbound script
    /// `this` (`Value::Object` in clonk-engine). Nil when the call has no object
    /// context (e.g. global functions).
    this_value: Value,
    /// The cross-object resolver for `obj->Method(args)` (AB_CALL,
    /// C4AulExec.cpp:1216-1305), registered by the engine. Called with
    /// [target, name, failsafe, args...].
    method_dispatch: Option<&'a HostFunction>,
    /// Reference-preserving twin of `method_dispatch`, used when an arrow
    /// call occupies an lvalue position.
    method_reference_dispatch: Option<&'a crate::engine::MethodReferenceDispatch>,
    /// Twin of `method_dispatch` for an arrow call carrying `&` arguments: it
    /// also reports the callee's final parameter slots so the caller can
    /// settle the reference cells the `&[Value]` bridge cannot carry.
    method_ref_args_dispatch: Option<&'a crate::engine::MethodRefArgsDispatch>,
    /// Engine-wide `&`-parameter lookup for callees this host cannot resolve
    /// (crate::engine::ReferenceParameterProbe).
    reference_parameter_probe: Option<&'a crate::engine::ReferenceParameterProbe>,
    /// Whole-engine name lookup used by C4AulParse before emitting a direct
    /// AB_CALL/AB_CALLFS (crate::engine::DirectCallFunctionProbe).
    direct_call_function_probe: Option<&'a crate::engine::DirectCallFunctionProbe>,
    /// Embedding-engine context switch for AB_CALLGLOBAL's null Obj/Def.
    global_call_context_hook: Option<&'a GlobalCallContextHook>,
    /// Embedding-engine receiver selection and DirectExec for FnEval.
    eval_direct_exec_hook: Option<&'a EvalDirectExecHook>,
    /// Continuation-capable twin used by the bytecode executor's nested `eval`.
    eval_direct_exec_continuation_hook: Option<&'a EvalDirectExecContinuationHook>,
    /// References returned from a global callee may outlive its temporary
    /// null Obj/Def context. Lazy host-backed references must recreate it.
    retain_global_call_context_for_host_paths: bool,
    /// The engine-global `static` table (GlobalNamed); resolved after
    /// locals, before global constants (C4AulParse.cpp:2836-2839).
    globals_named: Option<&'a GlobalVariables>,
    /// The engine-global numbered-variable table (`C4AulScriptEngine::Global`).
    globals_numbered: Option<&'a GlobalSlots>,
    /// The engine-global `static const` registry (GetGlobalConstant,
    /// C4Aul.cpp:494): script-declared constants shared across hosts,
    /// resolvable via the pre-#strict-2 `NAME()` call idiom.
    globals_consts: Option<&'a GlobalVariables>,
    /// Cross-object LocalN cell supplier (crate::engine::LocalCellHook).
    local_cell_hook: Option<&'a crate::engine::LocalCellHook>,
    /// Embedding-world receiver check used before every nonzero Object
    /// reaches AB_CALL or one of its Local* fast paths.
    object_target_availability_probe: Option<&'a crate::engine::ObjectTargetAvailabilityProbe>,
    string_registrations: Option<&'a crate::engine::StringRegistrationLedger>,
    /// Fallback literal interning for direct VM fixtures without a Script
    /// engine's shared C4StringTable.
    literal_strings: Rc<RefCell<HashMap<Vec<u8>, C4StringValue>>>,
    /// Per-call provenance for persistent/global cells that store only the
    /// public value representation. Nested script calls share this VM/cache.
    cell_identities: RefCell<HashMap<usize, RawIdentityCell>>,
    constant_identities: RefCell<HashMap<String, RawIdentityCell>>,
}

#[derive(Clone, Copy)]
struct ScriptFunctionTarget<'a> {
    function: &'a Function,
    validate_compiled_source: bool,
}

impl<'a> ScriptFunctionTarget<'a> {
    fn installed(function: &'a Function) -> Self {
        Self {
            function,
            validate_compiled_source: false,
        }
    }

    fn validated(function: &'a Function) -> Self {
        Self {
            function,
            validate_compiled_source: true,
        }
    }

    fn resolved(resolution: &'a crate::engine::ScriptFunctionResolution) -> Self {
        if resolution.has_trusted_snapshot() {
            Self::installed(&resolution.function)
        } else {
            Self::validated(&resolution.function)
        }
    }
}

impl<'a> Vm<'a> {
    fn object_target_available(&self, target: &Value) -> bool {
        object_target_id(target)
            .zip(self.object_target_availability_probe)
            .is_none_or(|(target, probe)| probe(target))
    }

    pub(crate) fn new(
        functions: &'a FxHashMap<String, Function>,
        host_functions: &'a FxHashMap<String, RegisteredHostFunction>,
        var_decls: &'a [VarDecl],
        debugger: Option<DebuggerHooks>,
    ) -> Self {
        Self {
            functions,
            host_identity: ScriptHostIdentity::fresh(),
            owner_definition_name: None,
            script_name: "",
            game_script_name: None,
            definition_context: false,
            owner_strict_level: None,
            host_functions,
            host_reference_functions: None,
            host_function_parameter_types: None,
            var_decls,
            debugger,
            constants: None,
            global_functions: None,
            exact_global_link_lookup: false,
            entry_parameter_conversion: Cell::new(ParameterConversionFailurePolicy::Error),
            this_value: Value::Nil,
            method_dispatch: None,
            method_reference_dispatch: None,
            method_ref_args_dispatch: None,
            reference_parameter_probe: None,
            direct_call_function_probe: None,
            global_call_context_hook: None,
            eval_direct_exec_hook: None,
            eval_direct_exec_continuation_hook: None,
            retain_global_call_context_for_host_paths: false,
            globals_named: None,
            globals_numbered: None,
            globals_consts: None,
            local_cell_hook: None,
            object_target_availability_probe: None,
            string_registrations: None,
            literal_strings: Rc::new(RefCell::new(HashMap::new())),
            cell_identities: RefCell::new(HashMap::new()),
            constant_identities: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn with_host_reference_functions(
        mut self,
        functions: &'a FxHashMap<String, HostReferenceFunction>,
    ) -> Self {
        self.host_reference_functions = Some(functions);
        self
    }

    pub(crate) fn with_host_function_parameter_types(
        mut self,
        parameter_types: &'a FxHashMap<String, Arc<[C4VType]>>,
    ) -> Self {
        self.host_function_parameter_types = Some(parameter_types);
        self
    }

    pub(crate) fn with_host_identity(mut self, identity: ScriptHostIdentity) -> Self {
        self.host_identity = identity;
        self
    }

    pub(crate) fn with_owner_definition_name(mut self, name: Option<&'a str>) -> Self {
        self.owner_definition_name = name.map(Arc::from);
        self
    }

    pub(crate) fn with_script_name(mut self, script_name: &'a str) -> Self {
        self.script_name = script_name;
        self
    }

    pub(crate) fn with_game_script_name(mut self, script_name: Option<&'a str>) -> Self {
        self.game_script_name = script_name;
        self
    }

    pub(crate) fn with_definition_context(mut self, definition_context: bool) -> Self {
        self.definition_context = definition_context;
        self
    }

    pub(crate) fn with_owner_strict_level(mut self, strict_level: Option<u8>) -> Self {
        self.owner_strict_level = Some(strict_level);
        self
    }

    /// Set the `this` object context for this call session. Nested plain calls
    /// share it (they run on the same object).
    pub fn with_this(mut self, this: Value) -> Self {
        self.this_value = this;
        self
    }

    /// Attach the engine constants table consulted on variable-lookup misses.
    pub fn with_constants(mut self, constants: &'a FxHashMap<String, Value>) -> Self {
        self.constants = Some(constants);
        self
    }

    /// Attach the engine-global script functions (System.c4g global funcs);
    /// `None` = no globals installed.
    pub fn with_optional_globals(
        mut self,
        functions: Option<&'a FxHashMap<String, Function>>,
    ) -> Self {
        self.global_functions = functions;
        self
    }

    pub(crate) fn with_exact_global_link_lookup(mut self) -> Self {
        self.exact_global_link_lookup = true;
        self
    }

    /// Marks the next selected script function as a C4Effect callback. The
    /// marker is consumed before its parameter frame is built, so calls
    /// originating inside that callback retain ordinary conversion behavior.
    pub(crate) fn with_effect_callback_parameter_conversion(self) -> Self {
        self.entry_parameter_conversion
            .set(ParameterConversionFailurePolicy::WarnForNonStrict3EffectCallback);
        self
    }

    /// Attach the cross-object method resolver for `obj->Method(args)`
    /// (AB_CALL, C4AulExec.cpp:1216-1305).
    pub fn with_method_dispatch(mut self, dispatch: Option<&'a HostFunction>) -> Self {
        self.method_dispatch = dispatch;
        self
    }

    pub fn with_method_reference_dispatch(
        mut self,
        dispatch: Option<&'a crate::engine::MethodReferenceDispatch>,
    ) -> Self {
        self.method_reference_dispatch = dispatch;
        self
    }

    pub fn with_method_ref_args_dispatch(
        mut self,
        dispatch: Option<&'a crate::engine::MethodRefArgsDispatch>,
    ) -> Self {
        self.method_ref_args_dispatch = dispatch;
        self
    }

    pub fn with_reference_parameter_probe(
        mut self,
        probe: Option<&'a crate::engine::ReferenceParameterProbe>,
    ) -> Self {
        self.reference_parameter_probe = probe;
        self
    }

    pub fn with_direct_call_function_probe(
        mut self,
        probe: Option<&'a crate::engine::DirectCallFunctionProbe>,
    ) -> Self {
        self.direct_call_function_probe = probe;
        self
    }

    pub fn with_global_call_context_hook(
        mut self,
        hook: Option<&'a GlobalCallContextHook>,
    ) -> Self {
        self.global_call_context_hook = hook;
        self
    }

    pub fn with_eval_direct_exec_hook(mut self, hook: Option<&'a EvalDirectExecHook>) -> Self {
        self.eval_direct_exec_hook = hook;
        self
    }

    pub fn with_eval_direct_exec_continuation_hook(
        mut self,
        hook: Option<&'a crate::engine::EvalDirectExecContinuationHook>,
    ) -> Self {
        self.eval_direct_exec_continuation_hook = hook;
        self
    }

    pub fn with_global_variables(mut self, table: Option<&'a GlobalVariables>) -> Self {
        self.globals_named = table;
        self
    }

    pub fn with_global_slots(mut self, table: Option<&'a GlobalSlots>) -> Self {
        self.globals_numbered = table;
        self
    }

    /// Attach the engine-global `static const` registry (GetGlobalConstant,
    /// C4Aul.cpp:494) consulted by the old-style constant-call idiom.
    pub fn with_global_constants(mut self, table: Option<&'a GlobalVariables>) -> Self {
        self.globals_consts = table;
        self
    }

    pub fn with_local_cell_hook(mut self, hook: Option<&'a crate::engine::LocalCellHook>) -> Self {
        self.local_cell_hook = hook;
        self
    }

    pub fn with_object_target_availability_probe(
        mut self,
        probe: Option<&'a crate::engine::ObjectTargetAvailabilityProbe>,
    ) -> Self {
        self.object_target_availability_probe = probe;
        self
    }

    pub fn with_string_registrations(
        mut self,
        registrations: Option<&'a crate::engine::StringRegistrationLedger>,
    ) -> Self {
        self.string_registrations = registrations;
        self
    }

    fn identity_for_cell(&self, cell: &ValueCell) -> RawIdentityCell {
        let key = Rc::as_ptr(cell) as usize;
        let existing = self.cell_identities.borrow().get(&key).cloned();
        if let Some(identity) = existing {
            return identity;
        }
        let identity = Rc::new(RefCell::new(TrackedValue::runtime_identity(&cell.borrow())));
        self.cell_identities
            .borrow_mut()
            .insert(key, identity.clone());
        identity
    }

    fn tracked_cell(&self, cell: ValueCell) -> LValueRef {
        ensure_active_object_reference_cell_registered(&cell);
        let identity = self.identity_for_cell(&cell);
        LValueRef::tracked_cell(cell, identity)
    }

    fn read_tracked_cell(&self, cell: &ValueCell) -> TrackedValue {
        let identity = legacy_identity_for_value_copy(
            cell,
            &[],
            self.identity_for_cell(cell).borrow().clone(),
        );
        TrackedValue {
            value: cell.borrow().clone(),
            identity,
        }
    }

    fn read_tracked_named_cell(&self, name: &str, cell: &ValueCell) -> TrackedValue {
        let value = cell.borrow().clone();
        let identity = self.identity_for_cell(cell);
        let _ = name;
        let tracked_identity = identity.borrow().clone();
        TrackedValue {
            value,
            identity: tracked_identity,
        }
    }

    fn tracked_constant(&self, name: &str, value: Value) -> TrackedValue {
        let existing = self.constant_identities.borrow().get(name).cloned();
        let identity = if let Some(identity) = existing {
            identity
        } else {
            let identity = Rc::new(RefCell::new(TrackedValue::runtime_identity(&value)));
            self.constant_identities
                .borrow_mut()
                .insert(name.to_string(), identity.clone());
            identity
        };
        let tracked_identity = identity.borrow().clone();
        TrackedValue {
            value,
            identity: tracked_identity,
        }
    }

    fn compiled_named_value(
        &self,
        name: &str,
        env: &Environment,
    ) -> Result<TrackedValue, RuntimeError> {
        if let Some(value) = env.get_tracked(name)? {
            return Ok(value);
        }
        if let Some(cell) = self.global_variable_cell(name) {
            return Ok(self.read_tracked_named_cell(name, &cell));
        }
        if name == "this" {
            return Ok(TrackedValue::runtime(self.this_value.clone()));
        }
        if let Some(cell) = self.global_constant_cell(name) {
            return Ok(Self::fold_legacy_zero_tracked(
                self.read_tracked_named_cell(name, &cell),
                env.strict_level,
            ));
        }
        lookup_profile::record(lookup_profile::LookupFamily::Constant, name);
        self.constants
            .and_then(|constants| constants.get(name).cloned())
            .map(|value| {
                Self::fold_legacy_zero_tracked(self.tracked_constant(name, value), env.strict_level)
            })
            .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'")))
    }

    /// The object an explicit `LocalN`/`Local` object argument selects on an
    /// arrow call. FnLocalN and FnLocal read the given `pObj` and fall back to
    /// `cthr->Obj`, the arrow target, only when it is null
    /// (C4Script.cpp:3417-3433, 4592-4605); a dead object converts to null
    /// the same way (clonk-org/clonk-rs#1531).
    fn explicit_local_owner(&self, explicit: Option<Value>) -> Option<Value> {
        explicit
            .filter(|value| matches!(value, Value::Object(id) if *id != 0))
            .filter(|value| self.object_target_available(value))
    }

    /// Resolves a LocalN target cell: falsy targets and the executing
    /// object use the VM's own object locals (FnLocalN's
    /// `if (!pObj) pObj = cthr->Obj`, C4Script.cpp:4593-4596); anything
    /// else asks the host hook for the foreign object's live cell. A
    /// hook miss falls back to self like C++'s nullptr conversion of
    /// dead objects.
    fn localn_cell(
        &self,
        env: &mut Environment,
        local_name: &str,
        target: Option<Value>,
    ) -> ValueCell {
        let foreign = target.filter(|value| {
            !matches!(
                value,
                Value::Nil | Value::Int(0) | Value::Bool(false) | Value::RawBool(0)
            ) && *value != self.this_value
        });
        if let Some(target) = foreign {
            return self
                .local_cell_hook
                .and_then(|hook| hook(&target, local_name))
                .unwrap_or_else(|| value_cell(Value::Nil));
        }
        env.object_state.named_local_cell(local_name)
    }

    /// C4Value::GetContainerElement's object branch: object `[]` and `.`
    /// reads resolve the named local on that object. The executing object
    /// owns its cells in this VM; foreign objects are supplied by the host.
    fn object_local_cell(
        &self,
        env: &Environment,
        target: &Value,
        name: &str,
    ) -> Option<ValueCell> {
        if matches!(target, Value::Object(0)) {
            return None;
        }
        if target == &self.this_value {
            self.var_decls
                .iter()
                .any(|declaration| {
                    declaration.kind == crate::ast::VarDeclKind::Local && declaration.name == name
                })
                .then(|| env.object_state.named_local_cell(name))
        } else {
            self.local_cell_hook.and_then(|hook| hook(target, name))
        }
    }

    fn object_local_tracked(&self, env: &Environment, target: &Value, name: &str) -> TrackedValue {
        self.object_local_cell(env, target, name)
            .map(|cell| self.read_tracked_cell(&cell))
            .unwrap_or_else(|| TrackedValue::runtime(Value::Nil))
    }

    fn object_local_value(&self, env: &Environment, target: &Value, name: &str) -> Value {
        self.object_local_tracked(env, target, name).value
    }

    /// Numbered Local slot cell (FnLocal by-reference, C4Script.cpp:
    /// 3423-3433: `pObj->Local[iIndex].GetRef()`): a FOREIGN target
    /// resolves through the cross-object cell hook under the engine's
    /// `__local_{index}` persistence key (ObjectState round-trips
    /// numbered slots as those local_vars entries); otherwise the
    /// executing object's own slot.
    fn numbered_local_cell(
        &self,
        env: &mut Environment,
        index: i32,
        target: Option<Value>,
    ) -> ValueCell {
        let foreign = target.filter(|value| {
            !matches!(
                value,
                Value::Nil | Value::Int(0) | Value::Bool(false) | Value::RawBool(0)
            ) && *value != self.this_value
        });
        if let Some(target) = foreign {
            if let Some(cell) = self
                .local_cell_hook
                .and_then(|hook| hook(&target, &format!("__local_{}", index.max(0))))
            {
                return cell;
            }
        }
        env.object_state.local_slot_cell(index)
    }

    /// FnGlobal's mutable `C4ValueList::operator[]` target
    /// (C4Script.cpp:3404-3407; C4ValueList.cpp:50-64).
    fn numbered_global_cell(&self, index: i32) -> Result<ValueCell, RuntimeError> {
        if index >= GLOBAL_SLOT_MAX_SIZE {
            return Err(RuntimeError::new("out of memory"));
        }
        let table = self
            .globals_numbered
            .ok_or_else(|| RuntimeError::new("unknown function 'Global'"))?;
        Ok(table
            .borrow_mut()
            .entry(index.max(0))
            .or_insert_with(|| value_cell(Value::Nil))
            .clone())
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        self.invoke_value(name, args, 0, ObjectState::default(), None)
    }

    pub(crate) fn call_with_continuation(
        &self,
        name: &str,
        args: &[Value],
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        self.call_args_with_continuation(name, args)
    }

    pub(crate) fn call_args_with_continuation(
        &self,
        name: &str,
        args: CallArgs,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        match self.invoke_value(name, args, 0, ObjectState::default(), None) {
            Ok(value) => Ok(ScriptCallOutcome::Complete(value)),
            Err(error) => self.script_call_outcome_from_error(error),
        }
    }

    fn script_call_outcome_from_error(
        &self,
        mut error: RuntimeError,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        let Some(control) = error.take_control() else {
            return Err(error);
        };
        let (request, resume_value, continuation) = match control {
            RuntimeControl::HostContinuation {
                request,
                resume_value,
                continuation,
            } => (request, resume_value, continuation),
        };
        let Some(continuation) = continuation else {
            return Err(error.with_control(RuntimeControl::HostContinuation {
                request,
                resume_value,
                continuation: None,
            }));
        };
        let mut continuation = match continuation.downcast::<ScriptContinuation>() {
            Ok(continuation) => continuation,
            Err(continuation) => {
                return Err(error.with_control(RuntimeControl::HostContinuation {
                    request,
                    resume_value,
                    continuation: Some(continuation),
                }));
            }
        };
        // A continuation owns its logical operand state, but an ordinary
        // standalone host call must not leave that state charged in the
        // process-local executor after the call has unwound. Inline engine
        // work can explicitly reattach it through the public context guard.
        continuation.detach_value_stack();
        Ok(ScriptCallOutcome::Suspended(ScriptSuspension {
            request,
            resume_value,
            continuation,
            this_value: self.this_value.clone(),
        }))
    }

    pub(crate) fn call_with_cells_with_continuation(
        &self,
        name: &str,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        self.call_args_with_cells_with_continuation(name, args, cells)
    }

    pub(crate) fn call_args_with_cells_with_continuation(
        &self,
        name: &str,
        args: CallArgs,
        cells: &LocalCells,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        match self.invoke_value(name, args, 0, cells.state.clone(), None) {
            Ok(value) => Ok(ScriptCallOutcome::Complete(value)),
            Err(error) => self.script_call_outcome_from_error(error),
        }
    }

    pub(crate) fn call_resolved_with_cells_with_continuation(
        &self,
        resolution: &crate::engine::ScriptFunctionResolution,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        match self.invoke_resolved_script_value(
            &resolution.function.name,
            ScriptFunctionTarget::resolved(resolution),
            args,
            0,
            cells.state.clone(),
            None,
        ) {
            Ok(value) => Ok(ScriptCallOutcome::Complete(value)),
            Err(error) => self.script_call_outcome_from_error(error),
        }
    }

    /// Call with caller-prepared arguments (reference cells included) — the
    /// host-side C4AulParSet pattern where pars carry `GetRef()` values.
    pub(crate) fn call_args(&self, name: &str, args: Vec<CallArg>) -> Result<Value, RuntimeError> {
        self.invoke_value(
            name,
            args.into_iter().collect(),
            0,
            ObjectState::default(),
            None,
        )
    }

    /// Exact engine-global entry with caller-prepared arguments. Unlike
    /// ordinary engine-scope invocation, a standalone VM without an attached
    /// shared table skips same-name local overloads and selects the `global
    /// func` node retained in the host's overload chain.
    pub(crate) fn call_engine_global_args(
        &self,
        name: &str,
        args: Vec<CallArg>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_engine_global_raw(name, args.into_iter().collect(), 0, None)?
            .into_value_on_stack()
    }

    /// Exact engine-global entry whose native caller supplied ordinary
    /// C4Values rather than explicit `GetRef()` cells.
    pub(crate) fn call_engine_global(
        &self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        self.call_engine_global_args(name, args)
    }

    /// Invoke an already-resolved immutable script function without another
    /// name lookup. Deferred native callbacks use this to mirror a retained
    /// C4AulFunc pointer while the VM still supplies the live host surface.
    pub(crate) fn call_pinned_args(
        &self,
        function: &Function,
        args: Vec<CallArg>,
    ) -> Result<Value, RuntimeError> {
        self.call_script_target_args(ScriptFunctionTarget::validated(function), args)
    }

    pub(crate) fn call_resolved_args(
        &self,
        resolution: &crate::engine::ScriptFunctionResolution,
        args: Vec<CallArg>,
    ) -> Result<Value, RuntimeError> {
        self.call_script_target_args(ScriptFunctionTarget::resolved(resolution), args)
    }

    fn call_script_target_args(
        &self,
        target: ScriptFunctionTarget<'_>,
        args: Vec<CallArg>,
    ) -> Result<Value, RuntimeError> {
        let depth = 0usize;
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }
        maybe_grow(|| {
            self.invoke_script_function(
                &target.function.name,
                target,
                args.into_iter().collect(),
                depth,
                ObjectState::default(),
                None,
            )?
            .into_value_on_stack()
        })
    }

    /// Invoke an already-resolved function against shared object-local cells.
    /// This is a fresh native/engine callback entry, so it deliberately does
    /// not inherit any ambient script caller retained by a dispatch bridge.
    pub(crate) fn call_pinned_with_cells(
        &self,
        function: &Function,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<Value, RuntimeError> {
        self.call_script_target_with_cells(ScriptFunctionTarget::validated(function), args, cells)
    }

    pub(crate) fn call_resolved_with_cells(
        &self,
        resolution: &crate::engine::ScriptFunctionResolution,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<Value, RuntimeError> {
        self.call_script_target_with_cells(ScriptFunctionTarget::resolved(resolution), args, cells)
    }

    fn call_script_target_with_cells(
        &self,
        target: ScriptFunctionTarget<'_>,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<Value, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        let depth = 0usize;
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }
        maybe_grow(|| {
            self.invoke_script_function(
                &target.function.name,
                target,
                args,
                depth,
                cells.state.clone(),
                None,
            )?
            .into_value_on_stack()
        })
    }

    /// Call against SHARED local cells (see [`LocalCells`]): writes land
    /// live — deeper sessions on the same object observe them mid-call.
    pub(crate) fn call_with_cells(
        &self,
        name: &str,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<Value, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        self.invoke_value(name, args, 0, cells.state.clone(), None)
    }

    /// Arrow-dispatch bridge entry. Unlike an ordinary engine-driven call,
    /// AB_CALL already has a suspended script frame; when the target resolves
    /// directly to a native function, that frame remains `cthr->Caller`.
    /// The method-dispatch guard makes it available here. Other host-to-VM
    /// callbacks must keep using [`Vm::call_with_cells`] so they start with no
    /// caller like C4AulFunc::Exec.
    pub(crate) fn call_with_cells_preserving_caller(
        &self,
        name: &str,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<Value, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::runtime).collect();
        let mut caller = current_caller_context();
        if let Some(caller) = &mut caller {
            // This entry is used only after AB_CALL has resolved an explicit
            // object/definition target. C4Id dispatch represents that target
            // with a nil `this`, so the destination host must restore Def.
            caller.definition_context |= self.definition_context;
        }
        let _parameter_override = CallParameterOverrideGuard::enter_if_absent(MAX_CALL_PARAMETERS);
        self.invoke_value_with_reserved_result(name, args, 0, cells.state.clone(), caller)
    }

    /// [`Vm::call_with_cells_preserving_caller`] with caller-prepared
    /// arguments, so a callee's `&` parameters alias the supplied cells.
    pub(crate) fn call_args_with_cells_preserving_caller(
        &self,
        name: &str,
        args: Vec<CallArg>,
        cells: &LocalCells,
    ) -> Result<Value, RuntimeError> {
        let mut caller = current_caller_context();
        if let Some(caller) = &mut caller {
            caller.definition_context |= self.definition_context;
        }
        let _parameter_override = CallParameterOverrideGuard::enter_if_absent(MAX_CALL_PARAMETERS);
        self.invoke_value_with_reserved_result(
            name,
            args.into_iter().collect(),
            0,
            cells.state.clone(),
            caller,
        )
    }

    /// Reference-returning counterpart to [`Vm::call_with_cells`].
    pub(crate) fn call_reference_with_cells(
        &self,
        name: &str,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<ValueReference, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        self.invoke_reference(name, args, 0, cells.state.clone(), None)
            .map(ValueReference)
    }

    /// Reference-returning counterpart to
    /// [`Vm::call_with_cells_preserving_caller`].
    pub(crate) fn call_reference_with_cells_preserving_caller(
        &self,
        name: &str,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<ValueReference, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::runtime).collect();
        let mut caller = current_caller_context();
        if let Some(caller) = &mut caller {
            caller.definition_context |= self.definition_context;
        }
        let _parameter_override = CallParameterOverrideGuard::enter_if_absent(MAX_CALL_PARAMETERS);
        self.invoke_reference(name, args, 0, cells.state.clone(), caller)
            .map(ValueReference)
    }

    /// Call a function with per-object local variable context
    /// Returns (result, updated_local_vars)
    pub fn call_with_locals(
        &self,
        name: &str,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
    ) -> Result<(Value, HashMap<String, Value>), RuntimeError> {
        let object_state = ObjectState::from_local_vars(local_vars);
        let args = args.iter().cloned().map(CallArg::external).collect();
        let value = self.invoke_value(name, args, 0, object_state.clone(), None)?;
        Ok((value, object_state.to_local_vars(self.var_decls)))
    }

    /// C4AulScript::DirectExec (C4AulExec.cpp:1658-1707): parse `source`
    /// as ONE expression (ParseFn fExprOnly — trailing text is ignored)
    /// and evaluate it in the object context — the host-side twin of the
    /// script-language `eval` special form. Parse errors yield C4VNull
    /// (DirectExec's catch, :1693-1699); runtime errors propagate for the
    /// caller's fPassErrors handling. Returns (result, updated_local_vars).
    #[cfg(test)]
    fn direct_exec_with_locals(
        &self,
        source: &str,
        local_vars: &HashMap<String, Value>,
        strict_level: Option<u8>,
    ) -> Result<(Value, HashMap<String, Value>), RuntimeError> {
        self.direct_exec_with_locals_in_context(
            source,
            local_vars,
            strict_level,
            "DirectExec",
            true,
        )
    }

    pub(crate) fn direct_exec_with_locals_in_context(
        &self,
        source: &str,
        local_vars: &HashMap<String, Value>,
        strict_level: Option<u8>,
        context: &str,
        diagnostics: bool,
    ) -> Result<(Value, HashMap<String, Value>), RuntimeError> {
        if diagnostics {
            start_direct_exec_profile();
        }
        let object_state = ObjectState::from_local_vars(local_vars);
        let Ok(expr) = crate::parser::Parser::with_strict_level_c4_string(source, strict_level)
            .parse_direct_exec_expression()
        else {
            return Ok((Value::Nil, object_state.to_local_vars(self.var_decls)));
        };
        let mut diagnostic = diagnostics.then(|| {
            ScriptDiagnosticGuard::enter_direct(self.direct_exec_diagnostic_frame(context), true)
        });
        let _object_reference_cells = ActiveObjectReferenceCellsGuard::enter_frame();
        let mut env = Environment::new_with_params(&[], &[], strict_level, object_state.clone())?;
        env.temporary_script = true;
        env.definition_context = matches!(&self.this_value, Value::Object(id) if *id != 0);
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }
        _object_reference_cells.register_environment(&env, self);
        let value = self.execute_direct_expression(expr, &mut env, 0)?;
        if let Some(diagnostic) = &mut diagnostic {
            diagnostic.returned(&value);
        }
        Ok((value, object_state.to_local_vars(self.var_decls)))
    }

    /// DirectExec against SHARED live cells (see [`LocalCells`]): writes land
    /// live, so deeper sessions on the same object observe them mid-call.
    #[cfg(test)]
    fn direct_exec_with_cells(
        &self,
        source: &str,
        cells: &LocalCells,
        strict_level: Option<u8>,
    ) -> Result<Value, RuntimeError> {
        self.direct_exec_with_cells_in_context(source, cells, strict_level, "DirectExec", true)
    }

    pub(crate) fn direct_exec_with_cells_in_context(
        &self,
        source: &str,
        cells: &LocalCells,
        strict_level: Option<u8>,
        context: &str,
        diagnostics: bool,
    ) -> Result<Value, RuntimeError> {
        if diagnostics {
            start_direct_exec_profile();
        }
        let Ok(expr) = crate::parser::Parser::with_strict_level_c4_string(source, strict_level)
            .parse_direct_exec_expression()
        else {
            return Ok(Value::Nil);
        };
        let mut diagnostic = diagnostics.then(|| {
            ScriptDiagnosticGuard::enter_direct(self.direct_exec_diagnostic_frame(context), true)
        });
        let _object_reference_cells = ActiveObjectReferenceCellsGuard::enter_frame();
        let mut env = Environment::new_with_params(&[], &[], strict_level, cells.state.clone())?;
        env.temporary_script = true;
        env.definition_context = matches!(&self.this_value, Value::Object(id) if *id != 0);
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }
        _object_reference_cells.register_environment(&env, self);
        let value = self.execute_direct_expression(expr, &mut env, 0)?;
        if let Some(diagnostic) = &mut diagnostic {
            diagnostic.returned(&value);
        }
        Ok(value)
    }

    /// Continuation-capable C4Aul DirectExec. The expression is wrapped in a
    /// temporary one-statement function so bytecode execution retains every
    /// operand and instruction position after a host callback yields. The temporary function
    /// is borrowed for the initial run and is copied only by `suspend` when a
    /// continuation must outlive this call.
    pub(crate) fn direct_exec_with_cells_in_context_with_continuation(
        &self,
        source: &str,
        cells: &LocalCells,
        strict_level: Option<u8>,
        context: &str,
        diagnostics: bool,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        if diagnostics {
            start_direct_exec_profile();
        }
        let Ok(expr) = crate::parser::Parser::with_strict_level_c4_string(source, strict_level)
            .parse_direct_exec_expression()
        else {
            return Ok(ScriptCallOutcome::Complete(Value::Nil));
        };

        let direct_exec_context = diagnostics.then(|| {
            DirectExecContinuationContext::new(self.direct_exec_diagnostic_frame(context), true)
        });
        self.run_direct_exec_with_continuation(
            expr,
            cells,
            strict_level,
            0,
            true,
            direct_exec_context,
        )
    }

    /// FnEval's continuation-capable DirectExec. It keeps the same diagnostic
    /// identity and depth as the synchronous eval path; in particular,
    /// `profile_on_error` stays false because the enclosing native frame owns
    /// the runtime-error profile interval.
    pub(crate) fn eval_direct_exec_with_cells_with_continuation(
        &self,
        source: &str,
        cells: &LocalCells,
        strict_level: Option<u8>,
        depth: usize,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        start_direct_exec_profile();
        let Ok(expr) = crate::parser::Parser::with_strict_level_c4_string(source, strict_level)
            .parse_direct_exec_expression()
        else {
            return Ok(ScriptCallOutcome::Complete(Value::Nil));
        };
        let has_object = matches!(&self.this_value, Value::Object(id) if *id != 0);
        let direct_exec_context = Some(DirectExecContinuationContext::new(
            self.eval_direct_exec_diagnostic_frame(self.definition_context),
            false,
        ));
        self.run_direct_exec_with_continuation(
            expr,
            cells,
            strict_level,
            depth,
            has_object,
            direct_exec_context,
        )
    }

    fn run_direct_exec_with_continuation(
        &self,
        expr: Expr,
        cells: &LocalCells,
        strict_level: Option<u8>,
        depth: usize,
        define_var_decls: bool,
        direct_exec_context: Option<DirectExecContinuationContext>,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        let _object_reference_cells = ActiveObjectReferenceCellsGuard::enter_frame();
        let mut env = Environment::new_with_params(&[], &[], strict_level, cells.state.clone())?;
        env.temporary_script = true;
        env.definition_context = matches!(&self.this_value, Value::Object(id) if *id != 0);
        env.engine_scope = self.retain_global_call_context_for_host_paths;
        env.global_call_context = self.retain_global_call_context_for_host_paths;
        if define_var_decls {
            for var_decl in self.var_decls {
                let cell = env.object_state.named_local_cell(&var_decl.name);
                env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
            }
        }
        _object_reference_cells.register_environment(&env, self);

        env.direct_exec_context = direct_exec_context;
        match self.execute_direct_expression(expr, &mut env, depth) {
            Ok(value) => Ok(ScriptCallOutcome::Complete(value)),
            Err(error) => self.script_call_outcome_from_error(error),
        }
    }

    fn execute_direct_expression(
        &self,
        expr: Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        let function = Self::direct_exec_function(expr, env.strict_level);
        let compiled = Arc::new(CompiledFunction::compile(&function).ok_or_else(|| {
            RuntimeError::new("internal error: DirectExec expression did not compile")
        })?);
        let result =
            compiled.execute(self, env, depth, &function, None, Arc::clone(&compiled), 0)?;
        crate::execution_profile::record_compiled();
        #[cfg(test)]
        COMPILED_FUNCTION_EXECUTIONS.with(|count| count.set(count.get() + 1));
        match result {
            ControlFlow::Return(value) => value.into_value_on_stack(),
            ControlFlow::Normal => Ok(Value::Nil),
        }
    }

    fn direct_exec_function(expr: Expr, strict_level: Option<u8>) -> Function {
        Function {
            name: String::new(),
            params: Vec::new(),
            body: vec![Stmt::Return(Some(expr))],
            access: AccessLevel::Public,
            returns_reference: false,
            description: None,
            strict_level,
            source_host: None,
            source_name: None,
            source_line: 0,
            global_link_host: None,
            overloaded: None,
            hard_inherited_line: None,
            hard_inherited_column: None,
            hard_inherited_stmt_index: None,
            global_local_candidates: Vec::new(),
            global_local_reference: None,
            compiled: std::sync::OnceLock::new(),
            resolved_snapshot: std::sync::OnceLock::new(),
        }
    }

    /// FnEval's DirectExec entry. Unlike host-initiated DirectExec, an eval
    /// runtime error is profiled when its enclosing native frame unwinds, so
    /// this temporary frame must not record the same interval a second time.
    pub(crate) fn eval_direct_exec_with_cells(
        &self,
        source: &str,
        cells: &LocalCells,
        strict_level: Option<u8>,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        start_direct_exec_profile();
        let Ok(expr) = crate::parser::Parser::with_strict_level_c4_string(source, strict_level)
            .parse_direct_exec_expression()
        else {
            return Ok(Value::Nil);
        };
        let mut diagnostic = ScriptDiagnosticGuard::enter_direct(
            self.eval_direct_exec_diagnostic_frame(self.definition_context),
            false,
        );
        let _object_reference_cells = ActiveObjectReferenceCellsGuard::enter_frame();
        let mut env = Environment::new_with_params(&[], &[], strict_level, cells.state.clone())?;
        env.temporary_script = true;
        let has_object = matches!(&self.this_value, Value::Object(id) if *id != 0);
        env.definition_context = has_object;
        if has_object {
            for var_decl in self.var_decls {
                let cell = env.object_state.named_local_cell(&var_decl.name);
                env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
            }
        }
        _object_reference_cells.register_environment(&env, self);
        let value = self.execute_direct_expression(expr, &mut env, depth)?;
        diagnostic.returned(&value);
        Ok(value)
    }

    fn direct_exec_diagnostic_frame(&self, context: &str) -> DirectExecDiagnosticFrame {
        DirectExecDiagnosticFrame::new(
            format!("{context} in {}", self.script_name),
            match &self.this_value {
                Value::Object(id) if *id != 0 => Some(*id),
                _ => None,
            },
        )
    }

    fn eval_direct_exec_diagnostic_frame(
        &self,
        definition_context: bool,
    ) -> DirectExecDiagnosticFrame {
        if let Value::Object(id) = &self.this_value {
            if *id != 0 {
                let dynamic_script_name =
                    diagnostic_object_display(*id).and_then(|(_, script_name)| script_name);
                let receiver = dynamic_script_name.as_deref().unwrap_or(self.script_name);
                return DirectExecDiagnosticFrame::new(format!("eval in {receiver}"), Some(*id));
            }
        }
        if !definition_context {
            DirectExecDiagnosticFrame::new(
                format!(
                    "eval in {}",
                    self.game_script_name.unwrap_or(self.script_name)
                ),
                None,
            )
        } else {
            self.direct_exec_diagnostic_frame("eval")
        }
    }

    fn invoke_value(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_raw(name, args, depth, object_state, caller)?
            .into_value_on_stack()
    }

    fn invoke_value_with_reserved_result(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_raw(name, args, depth, object_state, caller)?
            .into_value()
    }

    /// Exact Game.ScriptEngine lookup used by strict-3 `global->Fn()`.
    /// Unlike ordinary engine-scope calls, a bare VM may only fall back to
    /// an OWN declaration when that declaration is itself `global func`.
    fn engine_global_script_function(&self, name: &str) -> Option<&Function> {
        match self.global_functions {
            Some(functions) => functions.get(name),
            None => self.functions.get(name).and_then(|function| {
                std::iter::successors(Some(function), |function| function.overloaded.as_deref())
                    .find(|function| function.access == AccessLevel::Global)
            }),
        }
    }

    /// `GetOverloadedFunc`'s owner hop: `if (!f && Owner) { f =
    /// Owner->GetFuncRecursive(ByFunc->Name); }` (C4Aul.cpp:281-288). A
    /// definition script's Owner IS the script engine (C4Def.cpp:649
    /// `Script.Reg2List(&Game.ScriptEngine, &Game.ScriptEngine)`, and every
    /// other script kind registers the same way), so the hop resolves against
    /// the LIVE engine function table (C4Aul.cpp:293-301). Its same-name
    /// entries are head-inserted — the `C4AulFunc` constructor's `bAtEnd`
    /// default of true reaches `C4AulFuncMap::Add` as `bAtStart`
    /// (C4Aul.cpp:76-79, :613-629) — so the hop yields the NEWEST global from
    /// ANY host, with the engine-init natives left at the bucket tail.
    ///
    /// The stored overload chain can only approximate that, so it is consulted
    /// first for the own-host result and superseded here whenever C4Aul would
    /// have taken the hop. An engine-owned function never hops: the engine has
    /// no owner above it.
    fn inherited_engine_hop(&self, env: &Environment) -> Option<&Function> {
        let own_list_found_none = env
            .inherited_target
            .as_ref()
            .is_none_or(|target| target.access == AccessLevel::Global);
        (!env.engine_scope && own_list_found_none)
            .then(|| self.engine_global_script_function(&env.function_name))
            .flatten()
            .filter(|found| found.access == AccessLevel::Global)
    }

    fn inherited_target(&self, env: &Environment) -> Option<Arc<Function>> {
        self.inherited_engine_hop(env)
            .map(|function| Arc::new(function.clone()))
            .or_else(|| env.inherited_target.clone())
    }

    fn invoke_engine_global_raw(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        caller: Option<ScriptCallerContext>,
    ) -> Result<ReturnValue, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        maybe_grow(|| {
            if let Some(function) = self.engine_global_script_function(name) {
                let target = if self.global_functions.is_some() {
                    ScriptFunctionTarget::validated(function)
                } else {
                    ScriptFunctionTarget::installed(function)
                };
                return self.invoke_script_function(
                    name,
                    target,
                    args,
                    depth,
                    ObjectState::default(),
                    caller.clone(),
                );
            }

            if name == "VarN" && !self.has_host_function(name) {
                return self.invoke_varn_raw(&args, caller.as_ref());
            }

            if let Some(function) = self.host_functions.get(name) {
                let _guard = CallerContextGuard::enter(caller);
                return self
                    .invoke_host_function_call_args(name, function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value);
            }

            if let Some(function) = self.host_reference_function(name) {
                let _guard = CallerContextGuard::enter(caller);
                return self
                    .invoke_host_reference_function(name, function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value);
            }

            Err(RuntimeError::new(format!("unknown function '{name}'")))
        })
    }

    /// A global call is a fresh null-`this` VM frame but shares every engine
    /// table and host bridge with the suspended caller.
    fn engine_global_vm(&self) -> Vm<'a> {
        Vm {
            functions: self.functions,
            host_identity: self.host_identity,
            owner_definition_name: None,
            script_name: self.script_name,
            game_script_name: self.game_script_name,
            definition_context: false,
            owner_strict_level: self.owner_strict_level,
            host_functions: self.host_functions,
            host_reference_functions: self.host_reference_functions,
            host_function_parameter_types: self.host_function_parameter_types,
            var_decls: self.var_decls,
            debugger: self.debugger.clone(),
            constants: self.constants,
            global_functions: self.global_functions,
            exact_global_link_lookup: true,
            entry_parameter_conversion: Cell::new(ParameterConversionFailurePolicy::Error),
            this_value: Value::Nil,
            method_dispatch: self.method_dispatch,
            method_reference_dispatch: self.method_reference_dispatch,
            method_ref_args_dispatch: self.method_ref_args_dispatch,
            reference_parameter_probe: self.reference_parameter_probe,
            direct_call_function_probe: self.direct_call_function_probe,
            global_call_context_hook: self.global_call_context_hook,
            eval_direct_exec_hook: self.eval_direct_exec_hook,
            eval_direct_exec_continuation_hook: self.eval_direct_exec_continuation_hook,
            retain_global_call_context_for_host_paths: true,
            globals_named: self.globals_named,
            globals_numbered: self.globals_numbered,
            globals_consts: self.globals_consts,
            string_registrations: self.string_registrations,
            literal_strings: self.literal_strings.clone(),
            local_cell_hook: self.local_cell_hook,
            object_target_availability_probe: self.object_target_availability_probe,
            cell_identities: RefCell::new(HashMap::new()),
            constant_identities: RefCell::new(HashMap::new()),
        }
    }

    fn engine_script_function(&self, name: &str) -> Option<&Function> {
        lookup_profile::record(lookup_profile::LookupFamily::ScriptFunction, name);
        self.global_functions
            .map_or_else(|| self.functions.get(name), |functions| functions.get(name))
    }

    /// Named functions visible in the destination script's own scope.
    /// A C4Aul `global func` leaves only an unnamed FnLink in its declaring
    /// host, so every named lookup skips global nodes and falls through to
    /// the engine table. Ordinary same-name local functions still win. A
    /// bare/partial fixture VM with no table entry retains its only global.
    fn own_script_function(&self, name: &str) -> Option<&Function> {
        lookup_profile::record(lookup_profile::LookupFamily::ScriptFunction, name);
        let function = self.functions.get(name)?;
        if self.exact_global_link_lookup
            || self
                .global_functions
                .is_some_and(|functions| functions.contains_key(name))
        {
            function.first_non_global()
        } else {
            Some(function)
        }
    }

    fn own_or_global_script_function(&self, name: &str) -> Option<&Function> {
        self.own_script_function(name)
            .or_else(|| self.engine_global_script_function(name))
    }

    fn resolved_script_function(
        &self,
        name: &str,
        engine_scope: bool,
    ) -> Option<ScriptFunctionTarget<'_>> {
        if engine_scope {
            return self.engine_script_function(name).map(|function| {
                if self.global_functions.is_some() {
                    ScriptFunctionTarget::validated(function)
                } else {
                    ScriptFunctionTarget::installed(function)
                }
            });
        }
        self.own_script_function(name)
            .map(ScriptFunctionTarget::installed)
            .or_else(|| {
                self.global_functions
                    .and_then(|functions| functions.get(name))
                    .map(ScriptFunctionTarget::validated)
            })
    }

    fn invoke_resolved_script_raw(
        &self,
        name: &str,
        target: ScriptFunctionTarget<'_>,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<ReturnValue, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }
        maybe_grow(|| self.invoke_script_function(name, target, args, depth, object_state, caller))
    }

    fn invoke_resolved_script_value(
        &self,
        name: &str,
        target: ScriptFunctionTarget<'_>,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_resolved_script_raw(name, target, args, depth, object_state, caller)?
            .into_value_on_stack()
    }

    fn invoke_reference(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<LValueRef, RuntimeError> {
        match self.invoke_raw(name, args, depth, object_state, caller)? {
            ReturnValue::Reference(reference) => Ok(reference),
            ReturnValue::Value(_) => Err(RuntimeError::new(format!(
                "function '{name}' does not return a reference"
            ))),
        }
    }

    fn invoke_raw(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<ReturnValue, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        maybe_grow(|| {
            let _profiled_dispatch =
                lookup_profile::enter_site(lookup_profile::LookupSite::GenericDispatch);
            if let Some(function) = self.own_script_function(name) {
                #[cfg(test)]
                if caller.is_some() {
                    NESTED_GENERIC_SCRIPT_RESOLUTIONS.with(|count| count.set(count.get() + 1));
                }
                return self.invoke_script_function(
                    name,
                    ScriptFunctionTarget::installed(function),
                    args,
                    depth,
                    object_state,
                    caller.clone(),
                );
            }

            // Engine-global script functions (System.c4g `global func`s,
            // owned by Game.ScriptEngine in C++): the fallback after the
            // own script, before C++ engine functions — the
            // FindSameNameFunc own-def-then-engine order (C4Aul.cpp:130-148).
            if let Some(function) = self
                .global_functions
                .and_then(|functions| functions.get(name))
            {
                #[cfg(test)]
                if caller.is_some() {
                    NESTED_GENERIC_SCRIPT_RESOLUTIONS.with(|count| count.set(count.get() + 1));
                }
                return self.invoke_script_function(
                    name,
                    ScriptFunctionTarget::validated(function),
                    args,
                    depth,
                    object_state,
                    caller.clone(),
                );
            }

            if name == "VarN" && !self.has_host_function(name) {
                return self.invoke_varn_raw(&args, caller.as_ref());
            }

            if let Some(function) = self.host_functions.get(name) {
                #[cfg(test)]
                GENERIC_HOST_RESOLUTIONS.with(|count| count.set(count.get() + 1));
                // Host functions run under the CALLER's var-slot table
                // (cthr->Caller->NumVars) for the FindConstructionSite
                // write-back seam (C4Script.cpp:1966-1978).
                let _guard = CallerContextGuard::enter(caller);
                return self
                    .invoke_host_function_call_args(name, function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value);
            }

            if let Some(function) = self.host_reference_function(name) {
                #[cfg(test)]
                GENERIC_HOST_RESOLUTIONS.with(|count| count.set(count.get() + 1));
                let _guard = CallerContextGuard::enter(caller);
                return self
                    .invoke_host_reference_function(name, function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value);
            }

            Err(RuntimeError::new(format!("unknown function '{name}'")))
        })
    }

    fn invoke_script_function(
        &self,
        name: &str,
        target: ScriptFunctionTarget<'_>,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<ReturnValue, RuntimeError> {
        let function = target.function;
        // A `global func` that named one of its declaring script's `local`s
        // never linked: C4Aul threw while parsing the body, and
        // `C4AulScript::Parse` caught it, counted it and left the function's
        // code an `AB_ERR` chunk (`C4AulParse.cpp:2000-2004,3563-3586`). So the
        // call raises rather than running. The statements C4Aul had already
        // emitted before the offending token still run there and do not here;
        // truncating a body at a token needs expression spans the parser does
        // not carry, and is tracked as clonk-org/clonk-rs#344.
        if let Some((local, line)) = function.global_local_reference.as_ref() {
            return Err(RuntimeError::new(format!(
                "using local variable in global function! ({} names local `{local}` at line {line})",
                function.name
            )));
        }
        // C4AulScriptFunc inherits GetParCount()==10. These are the caller's
        // balanced argument slots and become the callee's parameter frame;
        // cross-host AB_CALL may provide the same count through the one-shot
        // override, so consume it exactly once at the true call boundary.
        let policy = self
            .entry_parameter_conversion
            .replace(ParameterConversionFailurePolicy::Error);
        let parameter_slots = take_call_parameter_slots(MAX_CALL_PARAMETERS);
        let mut value_stack = ValueStackReservation::reserve(parameter_slots)?;
        // Every script call carries the full ten-slot C4AulParSet. Parameter
        // conversion also visits the unnamed tail (whose declared type is
        // C4V_Any), so Par(n) observes the same eager-zero normalization as a
        // named parameter.
        let mut args = args;
        let debug_arg_count = args.len().min(MAX_CALL_PARAMETERS);
        args.truncate(MAX_CALL_PARAMETERS);
        // `resize_with` reserves the complete ten-slot C4AulParSet once.
        // Repeated `push` growth otherwise reallocates and moves the common
        // zero-to-three-argument call vector several times on every script
        // invocation.
        args.resize_with(MAX_CALL_PARAMETERS, || CallArg::runtime(Value::Nil));
        #[cfg(test)]
        record_call_arg_heap_spill(args.spilled());
        Self::check_convert_function_parameters(
            name,
            function,
            &mut args,
            caller.as_ref(),
            policy,
            match &self.this_value {
                Value::Object(object) => Some(*object),
                _ => None,
            },
        )?;

        // The external C4AulScriptFunc::Exec overload converts its temporary
        // C4AulParSet first, then C4AulExec::Exec pushes every slot with
        // C4Value::Set (C4AulExec.cpp:1638-1649,330-337). Script-to-script
        // Call uses the already-resident stack slots directly and therefore
        // deliberately skips this second copy.
        if caller.is_none() {
            for arg in &mut args {
                if let CallArg::Value(tracked) = arg {
                    let owned = std::mem::replace(tracked, TrackedValue::runtime(Value::Nil));
                    *tracked = owned.set_copy();
                }
            }
        }

        let debug_arg_reference_mask = args[..debug_arg_count]
            .iter()
            .enumerate()
            .fold(0_u16, |mask, (index, arg)| {
                mask | (u16::from(matches!(arg, CallArg::Reference(_))) << index)
            });
        let compiled_cache = function
            .compiled
            .get_or_init(|| CompiledFunctionCache::new(function));
        let rebuilt_cache;
        let validated_cache =
            match compiled_cache.validated(function, target.validate_compiled_source) {
                Some(cache) => Some(cache),
                None => {
                    rebuilt_cache = CompiledFunctionCache::new(function);
                    Some(&rebuilt_cache)
                }
            };
        let compiled = validated_cache
            .and_then(|cache| cache.compiled.as_ref())
            .ok_or_else(|| RuntimeError::new("internal error: script function did not compile"))?;
        // The callee's parameter bindings allocate C4Value cells while the
        // caller remains active. Enter its frame before constructing that
        // environment so their cleanup is charged to the callee, not the
        // long-lived caller.
        let _object_reference_cells = ActiveObjectReferenceCellsGuard::enter_frame();
        let mut env = Environment::new_with_params(
            &function.params,
            &args,
            function.strict_level,
            object_state,
        )?;
        // `Fn->OwnerOverloaded` is resolved in the function's OWN owner list,
        // which for a `global func` is the engine's (C4AulParse.cpp:1406-1408).
        env.inherited_target = function.owner_overloaded().cloned();
        env.function_name = function.name.clone();
        env.engine_scope = function.access == AccessLevel::Global;
        env.global_call_context = self.retain_global_call_context_for_host_paths;
        let explicit_definition_context = match &self.this_value {
            Value::Object(id) => *id != 0,
            Value::C4Id(id) => crate::value::c4_id_raw(id) != 0,
            _ => false,
        };
        let inherited_definition_context = caller
            .as_ref()
            .map_or(function.access != AccessLevel::Global, |caller| {
                caller.definition_context
            });
        env.definition_context = explicit_definition_context
            || (self.definition_context && inherited_definition_context);
        env.caller_host_identity = if env.engine_scope {
            function.global_link_host.unwrap_or(self.host_identity)
        } else {
            self.host_identity
        };
        env.caller_owner_strict_level = if env.engine_scope {
            Some(3)
        } else {
            self.owner_strict_level.unwrap_or(function.strict_level)
        };

        // C4Aul `var` declarations are FUNCTION-scoped and hoisted: the
        // parser builds the whole Fn->VarNamed table up front, so a var
        // read BEFORE its `var` statement is nil, never an error
        // (Dynamite.c4d reads iX three lines above `var iX`). Function vars,
        // like parameters, precede object locals in C4Aul's named-variable
        // table (C4AulParse.cpp:2709-2729). Hoist them first so an effect
        // callback's `var pClonk` cannot alias MART's persistent `pClonk`.
        for name in &compiled.function_vars {
            env.declare_hoisted(name);
        }
        let function_var_count = env.frame_locals.function_vars.borrow().len();
        value_stack.grow(function_var_count)?;

        let debug_args = self.call_args_to_values(&args[..debug_arg_count])?;
        let debugger_callback = self
            .debugger
            .as_ref()
            .and_then(|debugger| debugger.on_call());
        let debugger_args = debugger_callback.map(|_| debug_args.clone());
        let profile_host_identity =
            (function.access != AccessLevel::Global).then_some(self.host_identity);
        let cached_diagnostic_strings = Some(compiled).filter(|compiled| {
            compiled.diagnostic_name.as_ref() == name
                && compiled.diagnostic_source_name.as_deref() == function.source_name()
        });
        let (diagnostic_name, diagnostic_source_name, _diagnostic_string_allocations) =
            match cached_diagnostic_strings {
                Some(compiled) => (
                    Arc::clone(&compiled.diagnostic_name),
                    compiled.diagnostic_source_name.clone(),
                    0,
                ),
                None => (
                    Arc::from(name),
                    function.source_name().map(Arc::from),
                    1 + usize::from(function.source_name().is_some()),
                ),
            };
        let diagnostic_definition_name = (function.access != AccessLevel::Global)
            .then(|| self.owner_definition_name.clone())
            .flatten();
        let mut diagnostic = ScriptDiagnosticGuard::enter(
            diagnostic_name,
            profile_host_identity,
            debug_args,
            debug_arg_reference_mask,
            &self.this_value,
            diagnostic_definition_name,
            diagnostic_source_name,
            function,
        );
        #[cfg(test)]
        DIAGNOSTIC_FRAME_STRING_ALLOCATIONS
            .with(|count| count.set(count.get() + _diagnostic_string_allocations));
        if let Some(callback) = debugger_callback {
            callback(
                name,
                debugger_args
                    .as_deref()
                    .expect("debugger arguments are captured with its callback"),
            );
        }

        // `define_object_local` preserves parameter/function-var bindings so
        // MART::Mode0(pObj, ...) receives its argument and a same-name local
        // declaration remains call-scoped instead of mutating the object.
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }
        _object_reference_cells.register_environment(&env, self);

        let result = compiled.execute(
            self,
            &mut env,
            depth,
            function,
            caller.clone(),
            Arc::clone(compiled),
            value_stack.count,
        )?;
        crate::execution_profile::record_compiled();
        #[cfg(test)]
        COMPILED_FUNCTION_EXECUTIONS.with(|count| count.set(count.get() + 1));
        let value = match result {
            ControlFlow::Return(v) => v,
            ControlFlow::Normal => ReturnValue::Value(TrackedValue::runtime(Value::Nil)),
        };

        let _return_slot = ValueStackReservation::reserve(1)?;
        let debug_return = value.as_value()?;
        diagnostic.returned(&debug_return);
        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &debug_return);
            }
        }

        if caller.is_some() {
            Ok(match value {
                ReturnValue::Value(tracked) => {
                    let same_destination = env.call_args.first().is_some_and(|destination| {
                        destination.value_slot_is_same_zero_id(&tracked.value)
                    });
                    ReturnValue::Value(tracked.set_copy_into(same_destination))
                }
                ReturnValue::Reference(reference) => ReturnValue::Reference(reference),
            })
        } else {
            Ok(value)
        }
    }

    /// C++ `CheckConvertFunctionParameters` (C4AulExec.cpp:1364-1397).
    /// This is deliberately a call-boundary operation: it runs before the
    /// callee frame exists and mutates the copied parameter slots, not caller
    /// lvalues (except that `&` parameters retain their references).
    fn check_convert_function_parameters(
        name: &str,
        function: &Function,
        args: &mut [CallArg],
        caller: Option<&ScriptCallerContext>,
        policy: ParameterConversionFailurePolicy,
        context_object: Option<u64>,
    ) -> Result<(), RuntimeError> {
        let callee_has_strict_nil = function.strict_level.unwrap_or(0) >= 3;
        let (convert_to_any_eagerly, convert_nil_to_int_bool) = match caller {
            Some(caller) => {
                let caller_has_strict_nil = caller.origin_strict_level.unwrap_or(0) >= 3;
                (
                    !caller_has_strict_nil,
                    !caller_has_strict_nil && callee_has_strict_nil,
                )
            }
            // Engine entry points have no script caller. C4AulScriptFunc::Exec
            // uses the callee strictness and defaults convertNilToIntBool on.
            None => (!callee_has_strict_nil, callee_has_strict_nil),
        };

        for (index, arg) in args.iter_mut().enumerate().take(MAX_CALL_PARAMETERS) {
            let expected = Self::function_parameter_type(function.params.get(index));

            if expected == C4VType::Ref {
                if matches!(arg, CallArg::Reference(_)) {
                    continue;
                }
                let got = Self::c4v_type_name(arg.read()?.c4v_type());
                let message = format!(
                    "call to \"{name}\" parameter {}: got \"{got}\", but expected \"&\"!",
                    index + 1
                );
                if policy == ParameterConversionFailurePolicy::WarnForNonStrict3EffectCallback
                    && function.strict_level.unwrap_or(0) < 3
                {
                    // C4Value::ConvertTo(C4V_pC4Value) fails for a value
                    // slot, but a C4Effect callback still executes a
                    // pre-STRICT3 function after its warning. Leave the value
                    // slot in place: its parameter is readable but not an
                    // alias of the caller (C4AulExec.cpp:1364-1397;
                    // C4Value.cpp:488-620).
                    Self::warn_parameter_conversion_failure(&message, context_object);
                    continue;
                }
                return Err(RuntimeError::new(message));
            }

            // Non-reference parameters receive a dereferenced copy even when
            // an engine caller supplied C4Value refs.
            let mut tracked = arg.read_tracked()?;
            if matches!(arg, CallArg::Reference(_)) {
                // FnCnvDeref calls C4Value::Deref, which copies the referent
                // through Set before retrying the requested conversion.
                tracked = tracked.set_copy();
            }
            if convert_to_any_eagerly && !tracked.value.as_bool() {
                tracked = TrackedValue::runtime(Value::Nil);
            }

            if !tracked.value.convert_to_in_place(expected, true) {
                let message = format!(
                    "call to \"{name}\" parameter {}: got \"{}\", but expected \"{}\"!",
                    index + 1,
                    Self::c4v_type_name(tracked.value.c4v_type()),
                    Self::c4v_type_name(expected)
                );
                if policy == ParameterConversionFailurePolicy::WarnForNonStrict3EffectCallback
                    && function.strict_level.unwrap_or(0) < 3
                {
                    // C4AulScriptFunc::Exec keeps the original C4Value in
                    // this one mode: it emits a warning, then executes the
                    // pre-STRICT3 function (C4AulExec.cpp:1621-1648).
                    Self::warn_parameter_conversion_failure(&message, context_object);
                    *arg = CallArg::Value(tracked);
                    continue;
                }
                return Err(RuntimeError::new(message));
            }

            if convert_nil_to_int_bool && matches!(tracked.value, Value::Nil) {
                tracked = match expected {
                    C4VType::Int => TrackedValue::runtime(Value::Int(0)),
                    C4VType::Bool => TrackedValue::runtime(Value::Bool(false)),
                    _ => tracked,
                };
            }
            *arg = CallArg::Value(tracked);
        }
        Ok(())
    }

    fn warn_parameter_conversion_failure(message: &str, context_object: Option<u64>) {
        // C++'s ErrorOrWarning sends this same message to DebugLog and adds
        // the command-target object when one exists (C4AulExec.cpp:1345-1362).
        // Keep the object structured so presentation can choose its label;
        // tracing never enters simulation state or the lockstep hash.
        if let Some(object) = context_object {
            tracing::warn!(target: SCRIPT_DEBUG_LOG_TARGET, object, "{message}");
        } else {
            tracing::warn!(target: SCRIPT_DEBUG_LOG_TARGET, "{message}");
        }
    }

    fn function_parameter_type(parameter: Option<&Parameter>) -> C4VType {
        let Some(parameter) = parameter else {
            return C4VType::Any;
        };
        if parameter.is_reference {
            return C4VType::Ref;
        }
        match parameter.type_annotation.as_ref() {
            None | Some(TypeAnnotation::Any) => C4VType::Any,
            Some(TypeAnnotation::Int) => C4VType::Int,
            Some(TypeAnnotation::Bool) => C4VType::Bool,
            Some(TypeAnnotation::String) => C4VType::String,
            Some(TypeAnnotation::Object) => C4VType::C4Object,
            Some(TypeAnnotation::Id) => C4VType::C4Id,
            Some(TypeAnnotation::Array) => C4VType::Array,
            Some(TypeAnnotation::Map) => C4VType::Map,
        }
    }

    fn c4v_type_name(value_type: C4VType) -> &'static str {
        match value_type {
            C4VType::Any => "any",
            C4VType::Int => "int",
            C4VType::Bool => "bool",
            C4VType::C4Id => "id",
            C4VType::C4Object => "object",
            C4VType::String => "string",
            C4VType::Array => "array",
            C4VType::Map => "map",
            C4VType::Ref => "&",
        }
    }

    fn call_args_to_values(&self, args: &[CallArg]) -> Result<CallValues, RuntimeError> {
        let values: CallValues = args.iter().map(CallArg::read).collect::<Result<_, _>>()?;
        #[cfg(test)]
        record_call_arg_heap_spill(values.spilled());
        Ok(values)
    }

    fn call_args_into_values(&self, args: CallArgs) -> Result<CallValues, RuntimeError> {
        let values: CallValues = args
            .into_iter()
            .map(CallArg::into_value)
            .collect::<Result<_, _>>()?;
        #[cfg(test)]
        record_call_arg_heap_spill(values.spilled());
        Ok(values)
    }

    fn host_reference_function(&self, name: &str) -> Option<&HostReferenceFunction> {
        lookup_profile::record(lookup_profile::LookupFamily::HostFunction, name);
        self.host_reference_functions
            .and_then(|functions| functions.get(name))
    }

    fn has_host_function(&self, name: &str) -> bool {
        // Counted here rather than left to `host_reference_function`: `||`
        // short-circuits past that call whenever the value table hits, so a
        // successful probe would otherwise never be recorded and the host
        // family would under-report exactly its cheapest case.
        lookup_profile::record(lookup_profile::LookupFamily::HostFunction, name);
        self.host_functions.contains_key(name) || self.host_reference_function(name).is_some()
    }

    fn resolved_host_function(&self, name: &str) -> Option<ResolvedHostFunction<'_>> {
        lookup_profile::record(lookup_profile::LookupFamily::HostFunction, name);
        self.host_functions
            .get(name)
            .map(ResolvedHostFunction::Value)
            .or_else(|| {
                self.host_reference_function(name)
                    .map(ResolvedHostFunction::Reference)
            })
    }

    fn invoke_resolved_host_raw(
        &self,
        name: &str,
        function: ResolvedHostFunction<'_>,
        args: CallArgs,
        depth: usize,
        caller: Option<ScriptCallerContext>,
    ) -> Result<ReturnValue, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        maybe_grow(|| {
            let _guard = CallerContextGuard::enter(caller);
            match function {
                ResolvedHostFunction::Value(function) => self
                    .invoke_host_function_call_args(name, function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value),
                ResolvedHostFunction::Reference(function) => self
                    .invoke_host_reference_function(name, function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value),
            }
        })
    }

    fn invoke_resolved_host_value(
        &self,
        name: &str,
        function: ResolvedHostFunction<'_>,
        args: CallArgs,
        depth: usize,
        caller: Option<ScriptCallerContext>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_resolved_host_raw(name, function, args, depth, caller)?
            .into_value_on_stack()
    }

    /// C++ `CheckConvertFunctionParameters` for engine/native functions.
    /// Native callees never enable the script-only nil-to-int/bool bridge:
    /// legacy callers only collapse falsy non-reference values to `Any` nil,
    /// and every subsequent table conversion remains strict.
    fn prepare_native_host_call_args(
        &self,
        name: &str,
        args: CallArgs,
        declared_parameter_count: Option<usize>,
    ) -> Result<CallArgs, RuntimeError> {
        let parameter_types = self
            .host_function_parameter_types
            .and_then(|functions| functions.get(name));

        // Argument expressions have already run left-to-right. Only now does
        // C4Aul balance the native frame to the declared signature. Build the
        // final frame once: the registered arity limits which supplied slots
        // survive, while a conversion table (when present) owns the final
        // frame size just like the old two-stage normalization.
        let source_limit = declared_parameter_count
            .unwrap_or(args.len())
            .min(args.len());
        let parameter_count = parameter_types.map_or_else(
            || declared_parameter_count.unwrap_or(args.len()),
            |parameter_types| parameter_types.len(),
        );
        let mut prepared = CallArgs::with_capacity(parameter_count);
        prepared.extend(args.into_iter().take(source_limit).take(parameter_count));
        prepared.resize_with(parameter_count, || CallArg::runtime(Value::Nil));
        #[cfg(test)]
        record_call_arg_heap_spill(prepared.spilled());

        let Some(parameter_types) = parameter_types else {
            return Ok(prepared);
        };

        let convert_to_any_eagerly = !matches!(
            caller_origin_strictness(),
            HostCallerStrictness::Strict(level) if level >= 3
        );

        for (index, (arg, expected)) in prepared
            .iter_mut()
            .zip(parameter_types.iter().copied())
            .enumerate()
        {
            if expected == C4VType::Ref {
                if matches!(arg, CallArg::Reference(_)) {
                    continue;
                }
                let got = Self::c4v_type_name(arg.read()?.c4v_type());
                return Err(RuntimeError::new(format!(
                    "call to \"{name}\" parameter {}: got \"{got}\", but expected \"&\"!",
                    index + 1
                )));
            }

            // A native's non-reference C++ parameter receives a dereferenced
            // copy. Conversions therefore never mutate the caller's lvalue.
            let mut tracked = arg.read_tracked()?;
            if matches!(arg, CallArg::Reference(_)) {
                tracked = tracked.set_copy();
            }
            if convert_to_any_eagerly && !tracked.value.as_bool() {
                tracked = TrackedValue::runtime(Value::Nil);
            }
            if !tracked.value.convert_to_in_place(expected, true) {
                return Err(RuntimeError::new(format!(
                    "call to \"{name}\" parameter {}: got \"{}\", but expected \"{}\"!",
                    index + 1,
                    Self::c4v_type_name(tracked.value.c4v_type()),
                    Self::c4v_type_name(expected)
                )));
            }
            *arg = CallArg::Value(tracked);
        }
        Ok(prepared)
    }

    fn prepare_registered_host_call_args(
        &self,
        name: &str,
        function: &RegisteredHostFunction,
        args: CallArgs,
    ) -> Result<CallArgs, RuntimeError> {
        self.prepare_native_host_call_args(name, args, function.parameter_count())
    }

    fn invoke_host_function_call_args(
        &self,
        name: &str,
        function: &RegisteredHostFunction,
        args: CallArgs,
    ) -> Result<Value, RuntimeError> {
        // Reserve before dereferencing/converting CallArgs: a lazy HostPath
        // may invoke engine code, but C++ has already balanced the callee's
        // native frame at that point.
        let parameter_slots =
            take_call_parameter_slots(function.parameter_count().unwrap_or(MAX_CALL_PARAMETERS));
        let _value_stack = ValueStackReservation::reserve(parameter_slots)?;
        let args = self.prepare_registered_host_call_args(name, function, args)?;
        let destination_is_zero_id = args.first().is_some_and(CallArg::value_slot_is_zero_id);
        let values = self.call_args_into_values(args)?;
        let result = match self.invoke_host_function_raw(name, function, &values) {
            Ok(result) => result,
            Err(error) => return Err(error.with_host_parameter_slots(parameter_slots)),
        };
        if matches!(caller_origin_strictness(), HostCallerStrictness::NoCaller) {
            return Ok(result);
        }
        Ok(c4_set_copy_value_into(result, destination_is_zero_id))
    }

    /// Invoke the callback without applying its public script signature.
    /// EffectVar's retained-lvalue bridge uses a private fourth write value
    /// even though the script-visible native declares three parameters.
    fn invoke_host_function_raw(
        &self,
        name: &str,
        function: &RegisteredHostFunction,
        args: &[Value],
    ) -> Result<Value, RuntimeError> {
        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_call() {
                callback(name, args);
            }
        }

        let outcome = (function.callback())(args);
        let result = outcome?;

        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &result);
            }
        }

        Ok(result)
    }

    fn invoke_host_reference_function(
        &self,
        name: &str,
        function: &HostReferenceFunction,
        args: CallArgs,
    ) -> Result<Value, RuntimeError> {
        let parameter_slots =
            take_call_parameter_slots(function.parameter_count().unwrap_or(MAX_CALL_PARAMETERS));
        let _value_stack = ValueStackReservation::reserve(parameter_slots)?;
        let call_args =
            self.prepare_native_host_call_args(name, args, function.parameter_count())?;
        let args = call_args
            .iter()
            .cloned()
            .map(HostCallArg)
            .collect::<HostCallArgs>();
        #[cfg(test)]
        record_call_arg_heap_spill(args.spilled());
        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_call() {
                let debug_args = args
                    .iter()
                    .map(HostCallArg::read)
                    .collect::<Result<CallValues, _>>()?;
                callback(name, &debug_args);
            }
        }

        let result = match function.call(&args) {
            Ok(result) => result,
            Err(error) => return Err(error.with_host_parameter_slots(parameter_slots)),
        };

        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &result);
            }
        }
        Ok(materialize_internal_native_call_result(result, &call_args))
    }

    fn global_call_target(&self, name: &str) -> RetainedCallTarget {
        if let Some(function) = self.engine_global_script_function(name) {
            return RetainedCallTarget::Script(CompiledScriptTarget {
                function: function.resolved_snapshot(),
                validate_compiled_source: self.global_functions.is_some(),
            });
        }
        if let Some(target) = self.resolved_host_function(name) {
            return match target {
                ResolvedHostFunction::Value(function) => RetainedCallTarget::Host(function.clone()),
                ResolvedHostFunction::Reference(function) => {
                    RetainedCallTarget::HostReference(function.clone())
                }
            };
        }
        if Self::is_global_vm_builtin(name) {
            RetainedCallTarget::Builtin
        } else {
            RetainedCallTarget::Dynamic
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_retained_direct_target(
        &self,
        target: RetainedCallTarget,
        name: &str,
        args: CallArgs,
        depth: usize,
        env: &mut Environment,
        caller: Option<ScriptCallerContext>,
        return_reference: bool,
    ) -> Result<ReturnValue, RuntimeError> {
        match target {
            RetainedCallTarget::Script(target) => self.invoke_resolved_script_raw(
                name,
                ScriptFunctionTarget {
                    function: &target.function,
                    validate_compiled_source: target.validate_compiled_source,
                },
                args,
                depth + 1,
                env.object_state.clone(),
                caller,
            ),
            RetainedCallTarget::Host(function) => {
                if return_reference && name == "EffectVar" {
                    // The call target was captured before the argument list
                    // ran. Keep that exact native callback in the lvalue so a
                    // host yield cannot make the resumed path re-resolve a
                    // different EffectVar overload.
                    return self.effect_slot_from_registered_host_call_args(&function, args, env);
                }
                let _guard = CallerContextGuard::enter(caller);
                self.invoke_host_function_call_args(name, &function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value)
            }
            RetainedCallTarget::HostReference(function) => {
                let _guard = CallerContextGuard::enter(caller);
                self.invoke_host_reference_function(name, &function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value)
            }
            RetainedCallTarget::Builtin => {
                if name == "this" && self.has_bound_this(env) {
                    return Err(RuntimeError::new("cannot call bound variable 'this'"));
                }
                if name == "this" {
                    return Ok(ReturnValue::Value(TrackedValue::runtime(
                        self.this_value.clone(),
                    )));
                }
                if let Some(value) =
                    self.invoke_direct_vm_builtin_call_args(name, &args, env, return_reference)?
                {
                    return Ok(value);
                }
                self.invoke_global_builtin_raw(name, &args, env, depth + 1)
            }
            RetainedCallTarget::Dynamic => Err(RuntimeError::new(
                "internal error: dynamic call target was dispatched as retained",
            )),
        }
    }

    fn invoke_retained_global_target(
        &self,
        target: RetainedCallTarget,
        name: &str,
        args: CallArgs,
        depth: usize,
        env: &mut Environment,
        caller: Option<ScriptCallerContext>,
    ) -> Result<ReturnValue, RuntimeError> {
        match target {
            RetainedCallTarget::Script(target) => self.invoke_resolved_script_raw(
                name,
                ScriptFunctionTarget {
                    function: &target.function,
                    validate_compiled_source: target.validate_compiled_source,
                },
                args,
                depth + 1,
                ObjectState::default(),
                caller,
            ),
            RetainedCallTarget::Host(function) => {
                let _guard = CallerContextGuard::enter(caller);
                self.invoke_host_function_call_args(name, &function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value)
            }
            RetainedCallTarget::HostReference(function) => {
                let _guard = CallerContextGuard::enter(caller);
                self.invoke_host_reference_function(name, &function, args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value)
            }
            RetainedCallTarget::Builtin => {
                self.invoke_global_builtin_raw(name, &args, env, depth + 1)
            }
            RetainedCallTarget::Dynamic => Err(RuntimeError::new(
                "internal error: non-global call target was dispatched as retained",
            )),
        }
    }

    fn global_variable_cell(&self, name: &str) -> Option<ValueCell> {
        lookup_profile::record(lookup_profile::LookupFamily::Global, name);
        self.globals_named
            .and_then(|table| table.borrow().get(name).cloned())
    }

    fn has_bound_this(&self, env: &Environment) -> bool {
        env.lvalue("this").is_some() || self.global_variable_cell("this").is_some()
    }

    fn global_constant_cell(&self, name: &str) -> Option<ValueCell> {
        lookup_profile::record(lookup_profile::LookupFamily::Constant, name);
        self.globals_consts
            .and_then(|table| table.borrow().get(name).cloned())
    }

    fn register_runtime_value(&self, value: &Value) {
        #[cfg(test)]
        if matches!(value, Value::Array(_) | Value::Proplist(_)) {
            RUNTIME_CONTAINER_REGISTRATION_TRAVERSALS.with(|count| count.set(count.get() + 1));
        }
        if let Some(registrations) = self.string_registrations {
            crate::engine::register_c4_value_strings(registrations, value);
        }
    }

    fn materialize_set_no_ref_result(result: ReturnValue) -> Result<TrackedValue, RuntimeError> {
        match result {
            ReturnValue::Value(value) => Ok(value),
            ReturnValue::Reference(reference) => {
                reference.read_tracked().map(TrackedValue::set_copy)
            }
        }
    }

    /// `++`/`--` operand conversion: CheckOpPar<C4V_Int> converts nil to 0 and
    /// bool to int before the operation (C4AulExec.cpp:450-458,
    /// C4Value.cpp:453-466 FnCnvGuess); other types stay errors.
    fn counter_operand(value: Value, operation: &str) -> Result<i32, RuntimeError> {
        match value {
            Value::Int(value) => Ok(value),
            Value::Nil => Ok(0),
            Value::Bool(flag) => Ok(i32::from(flag)),
            Value::RawBool(raw) => Ok(raw as u32 as i32),
            other => Err(RuntimeError::new(format!(
                "cannot {operation} non-integer value: {other:?}"
            ))),
        }
    }

    fn fold_legacy_zero(value: Value, strict_level: Option<u8>) -> Value {
        match value {
            Value::Int(0) | Value::Bool(false) | Value::RawBool(0)
                if strict_level.unwrap_or(0) < 3 =>
            {
                Value::Nil
            }
            value => value,
        }
    }

    fn fold_legacy_zero_tracked(mut value: TrackedValue, strict_level: Option<u8>) -> TrackedValue {
        value.value = Self::fold_legacy_zero(value.value, strict_level);
        value
    }

    fn literal_string(&self, value: &str) -> C4StringValue {
        if let Some(registrations) = self.string_registrations {
            return crate::engine::register_c4_literal_string(registrations, value);
        }
        let mut key = c4_string_bytes(value);
        if let Some(nul) = key.iter().position(|byte| *byte == 0) {
            key.truncate(nul);
        }
        if let Some(existing) = self.literal_strings.borrow().get(&key) {
            return existing.clone();
        }
        let value = C4StringValue::new(value.to_owned());
        self.literal_strings.borrow_mut().insert(key, value.clone());
        value
    }

    fn literal_value(&self, literal: &Literal, strict_level: Option<u8>) -> Value {
        let value = match literal {
            Literal::Int(i) => Value::Int(*i),
            Literal::Bool(b) => Value::Bool(*b),
            Literal::String(s) => Value::String(self.literal_string(s)),
            Literal::C4Id(id) if crate::value::c4_id_raw(id) == 0 => Value::Nil,
            Literal::C4Id(id) => Value::C4Id(id.clone()),
            Literal::Nil => Value::Nil,
        };
        // AddBCC rewrites emitted zero-valued AB_INT/AB_BOOL operands to a
        // default (nil) stack slot below STRICT3. Only literals and expanded
        // constants pass through this path; computed zero values remain typed.
        Self::fold_legacy_zero(value, strict_level)
    }

    fn eval_unary(&self, op: &UnaryOp, value: Value) -> Result<Value, RuntimeError> {
        match op {
            // C4AulExec.cpp:468-470 AB_Neg: SetInt(-_getInt()) — coerce nil->0,
            // bool->0/1; wrapping_neg matches C++ on i32::MIN instead of panicking.
            UnaryOp::Negate => value
                .as_c4_int()
                .map(|i| Value::Int(i.wrapping_neg()))
                .ok_or_else(|| {
                    RuntimeError::new(format!("cannot apply unary '-' to {}", value.type_name()))
                }),
            UnaryOp::Not => Ok(Value::Bool(!value.as_bool())),
            // C4AulExec.cpp:460-462 AB_BitNot: SetInt(~_getInt()).
            UnaryOp::BitwiseNot => value.as_c4_int().map(|i| Value::Int(!i)).ok_or_else(|| {
                RuntimeError::new(format!("cannot apply unary '~' to {}", value.type_name()))
            }),
        }
    }

    fn eval_binary(
        &self,
        left: Value,
        op: &BinaryOp,
        right: Value,
        strict: Option<u8>,
        display_symbol: Option<&str>,
    ) -> Result<Value, RuntimeError> {
        use BinaryOp::*;
        // Binary integer operators instantiate CheckOpPars with both
        // allowAny flags false (C4AulExec.cpp:490-593, 710-730). At strict 3
        // that rejects nil before the opcode reads its integer payload. Keep
        // the coercive Value::as_c4_int behavior for older strict levels and
        // for unrelated call sites such as array indices.
        if let Some(symbol) = match op {
            Add => Some("+"),
            Sub => Some("-"),
            Mul => Some("*"),
            Div => Some("/"),
            Mod => Some("%"),
            Pow => Some("**"),
            Less => Some("<"),
            LessEqual => Some("<="),
            Greater => Some(">"),
            GreaterEqual => Some(">="),
            BitAnd => Some("&"),
            BitOr => Some("|"),
            BitXor => Some("^"),
            LeftShift => Some("<<"),
            RightShift => Some(">>"),
            _ => None,
        } {
            let symbol = display_symbol.unwrap_or(symbol);
            Self::reject_strict3_nil_operand(&left, strict, symbol, " left side")?;
            Self::reject_strict3_nil_operand(&right, strict, symbol, " right side")?;
        }

        match op {
            Add => self.eval_add(left, right),
            Concat => self.eval_concat(left, right, strict, display_symbol.unwrap_or("..")),
            // Reached only via non-short-circuit paths (the Binary arm in
            // `evaluate` handles `??` before both sides run); keep the same
            // nil-only semantics.
            NilCoalescing => Ok(if matches!(left, Value::Nil) {
                right
            } else {
                left
            }),
            // C++ AB_Sub stores the native C4ValueInt difference directly in
            // C4Value (`SetInt(lhs - rhs)`, C4AulExec.cpp:546-553), so it wraps
            // on 32-bit two's-complement overflow rather than trapping. Match
            // AB_Sum/AB_Mul below instead of panicking in a checked build.
            Sub => self.eval_int_op(left, right, i32::wrapping_sub, "-"),
            // C++ AB_Mul stores the native C4ValueInt product directly in
            // C4Value (`SetInt(lhs * rhs)`, C4AulExec.cpp:511-518). Preserve
            // that 32-bit two's-complement result instead of panicking in a
            // checked Rust build; Helpers.c::DrawParticleLine deliberately
            // multiplies packed RGB channels by interpolation weights large
            // enough to cross i32::MAX.
            Mul => self.eval_int_op(left, right, i32::wrapping_mul, "*"),
            Div => match (left.as_c4_int(), right.as_c4_int()) {
                // C4AulExec.cpp:504-507: divisor 0 yields 0, not an error.
                (Some(_), Some(0)) => Ok(Value::Int(0)),
                // wrapping_div avoids a debug panic on i32::MIN / -1 (C++ wraps).
                (Some(lhs), Some(rhs)) => Ok(Value::Int(lhs.wrapping_div(rhs))),
                _ => Err(RuntimeError::new(format!(
                    "cannot apply '/' to operands of type {} and {}",
                    left.type_name(),
                    right.type_name()
                ))),
            },
            Mod => match (left.as_c4_int(), right.as_c4_int()) {
                // C4AulExec.cpp:523-526: modulo by 0 yields 0, not an error.
                (Some(_), Some(0)) => Ok(Value::Int(0)),
                (Some(lhs), Some(rhs)) => Ok(Value::Int(lhs.wrapping_rem(rhs))),
                _ => Err(RuntimeError::new(format!(
                    "cannot apply '%' to operands of type {} and {}",
                    left.type_name(),
                    right.type_name()
                ))),
            },
            Pow => match (left.as_c4_int(), right.as_c4_int()) {
                // C4Math.cpp:48-65 returns 0 for negative exponents. For
                // non-negative exponents, preserve C4ValueInt's 32-bit
                // two's-complement overflow instead of panicking in debug
                // builds. `as_c4_int` also mirrors `_getInt()` for nil/bool.
                (Some(_), Some(rhs)) if rhs < 0 => Ok(Value::Int(0)),
                (Some(lhs), Some(rhs)) => Ok(Value::Int(lhs.wrapping_pow(rhs as u32))),
                _ => Err(RuntimeError::new(format!(
                    "cannot apply '**' to operands of type {} and {}",
                    left.type_name(),
                    right.type_name()
                ))),
            },
            Equal => Ok(Value::Bool(
                self.values_equal(&left, &right, strict, None, None),
            )),
            NotEqual => Ok(Value::Bool(
                !self.values_equal(&left, &right, strict, None, None),
            )),
            Less => self.eval_int_cmp(left, right, |a, b| a < b, "<"),
            LessEqual => self.eval_int_cmp(left, right, |a, b| a <= b, "<="),
            Greater => self.eval_int_cmp(left, right, |a, b| a > b, ">"),
            GreaterEqual => self.eval_int_cmp(left, right, |a, b| a >= b, ">="),
            And | Or => unreachable!(),
            BitAnd => self.eval_int_op(left, right, |a, b| a & b, "&"),
            BitOr => self.eval_int_op(left, right, |a, b| a | b, "|"),
            BitXor => self.eval_int_op(left, right, |a, b| a ^ b, "^"),
            // C++ executes these with native 32-bit integer shifts. Its x86
            // runtime masks the count to five bits, including negative counts
            // read through `_getInt()`. Use wrapping shifts to make that
            // behavior deterministic and avoid Rust debug-build panics.
            LeftShift => {
                self.eval_int_op(left, right, |a, b| a.wrapping_shl((b as u32) & 31), "<<")
            }
            RightShift => {
                self.eval_int_op(left, right, |a, b| a.wrapping_shr((b as u32) & 31), ">>")
            }
            // String comparison operators
            StringEqual => self.eval_string_cmp(left, right, strict, |a, b| a == b, "S="),
            KeywordStringEqual => self.eval_string_cmp(left, right, strict, |a, b| a == b, "eq"),
            KeywordStringNotEqual => self.eval_string_cmp(left, right, strict, |a, b| a != b, "ne"),
        }
    }

    fn reject_strict3_nil_operand(
        value: &Value,
        strict: Option<u8>,
        symbol: &str,
        side: &str,
    ) -> Result<(), RuntimeError> {
        if strict.unwrap_or(0) >= 3 && matches!(value, Value::Nil) {
            return Err(RuntimeError::new(format!(
                "operator \"{symbol}\"{side}: got nil, but expected \"int\"!"
            )));
        }
        Ok(())
    }

    /// `..` concatenation (C4Script AB_Concat, C4AulExec.cpp:594-657): array .. array
    /// appends, map .. map merges (right wins on key collision), otherwise both
    /// operands are converted to strings and joined. Unlike `+`, `..` never does
    /// integer arithmetic — `5 .. 3` is the string "53".
    fn eval_concat_tracked(
        &self,
        left: TrackedValue,
        right: TrackedValue,
        strict: Option<u8>,
        operator: &str,
    ) -> Result<TrackedValue, RuntimeError> {
        match (&left.value, &right.value) {
            (Value::Array(left_values), Value::Array(right_values)) => {
                // Reject before cloning/extending the parallel identity list.
                // The value-level check below remains authoritative for
                // untracked concat callers as well.
                ensure_array_concat_size(left_values.len(), right_values.len())?;
                let mut identities = match left.identity.as_ref() {
                    Some(RawIdentity::Heap(identity)) => match identity.as_ref() {
                        HeapIdentity::Array(identities) => identities.clone(),
                        _ => unreachable!(),
                    },
                    _ => match HeapIdentity::opaque_for(&left.value) {
                        HeapIdentity::Array(identities) => identities,
                        _ => unreachable!(),
                    },
                };
                let right_identities = match right.identity.as_ref() {
                    Some(RawIdentity::Heap(identity)) => match identity.as_ref() {
                        HeapIdentity::Array(identities) => identities.clone(),
                        _ => unreachable!(),
                    },
                    _ => match HeapIdentity::opaque_for(&right.value) {
                        HeapIdentity::Array(identities) => identities,
                        _ => unreachable!(),
                    },
                };
                identities.extend(right_identities);
                let value = self.eval_concat(left.value, right.value, strict, operator)?;
                Ok(TrackedValue {
                    value,
                    identity: Some(RawIdentity::Heap(Rc::new(HeapIdentity::Array(identities)))),
                })
            }
            (Value::Proplist(left_entries), Value::Proplist(right_entries)) => {
                let mut identities = match left.identity.as_ref() {
                    Some(RawIdentity::Heap(identity)) => match identity.as_ref() {
                        HeapIdentity::Proplist(identities) => identities.clone(),
                        _ => unreachable!(),
                    },
                    _ => match HeapIdentity::opaque_for(&left.value) {
                        HeapIdentity::Proplist(identities) => identities,
                        _ => unreachable!(),
                    },
                };
                let right_identities = match right.identity.as_ref() {
                    Some(RawIdentity::Heap(identity)) => match identity.as_ref() {
                        HeapIdentity::Proplist(identities) => identities.clone(),
                        _ => unreachable!(),
                    },
                    _ => match HeapIdentity::opaque_for(&right.value) {
                        HeapIdentity::Proplist(identities) => identities,
                        _ => unreachable!(),
                    },
                };
                let right_identity_updates = right_entries
                    .iter()
                    .map(|(key, value)| {
                        let preserve_left = operator == "..="
                            && c4_set_copy_is_zero_id(value)
                            && left_entries
                                .get_key(key)
                                .is_some_and(c4_set_copy_is_zero_id);
                        (
                            key.clone(),
                            preserve_left,
                            right_identities.get(key).cloned().unwrap_or(None),
                        )
                    })
                    .collect::<Vec<_>>();
                let value = self.eval_concat(left.value, right.value, strict, operator)?;
                let Value::Proplist(result_entries) = &value else {
                    unreachable!();
                };
                identities.retain(|key, _| result_entries.contains_value_key(key));
                for (key, preserve_left, right_identity) in right_identity_updates {
                    if result_entries.contains_value_key(&key) {
                        if !preserve_left {
                            identities.insert(key, right_identity);
                        }
                    } else {
                        identities.remove(&key);
                    }
                }
                Ok(TrackedValue {
                    value,
                    identity: Some(RawIdentity::Heap(Rc::new(HeapIdentity::Proplist(
                        identities,
                    )))),
                })
            }
            _ => self
                .eval_concat(left.value, right.value, strict, operator)
                .map(TrackedValue::runtime),
        }
    }

    fn eval_concat(
        &self,
        mut left: Value,
        mut right: Value,
        strict: Option<u8>,
        operator: &str,
    ) -> Result<Value, RuntimeError> {
        // Below STRICT3, CheckOpPar rewrites falsey value operands to the
        // zero-data C4V_Any slot. `..=` keeps its left reference intact, but
        // its ordinary RHS still receives this normalization.
        if strict.unwrap_or(0) < 3 {
            if operator != "..=" && !left.as_bool() {
                left = Value::Nil;
            }
            if !right.as_bool() {
                right = Value::Nil;
            }
        }

        // CheckOpPars<Any, Any, false, false> rejects nil before AB_Concat at
        // STRICT3. For `..=`, GetType() dereferences the left stack reference,
        // but its expected operator-map type remains C4V_pC4Value (`"&"`).
        if strict.unwrap_or(0) >= 3 {
            if matches!(left, Value::Nil) {
                let expected = if operator == "..=" { "&" } else { "any" };
                return Err(RuntimeError::new(format!(
                    "operator \"{operator}\" left side: got nil, but expected \"{expected}\"!"
                )));
            }
            if matches!(right, Value::Nil) {
                return Err(RuntimeError::new(format!(
                    "operator \"{operator}\" right side: got nil, but expected \"any\"!"
                )));
            }
        }

        match left {
            Value::Array(mut a) => match right {
                Value::Array(b) => {
                    ensure_array_concat_size(a.len(), b.len())?;
                    // AB_Concat/AB_ConcatIt assign every appended element with
                    // C4Value::operator=, which routes through Set.
                    a.extend(b.into_iter().map(c4_set_copy_value));
                    Ok(Value::Array(a))
                }
                other => Err(RuntimeError::new(format!(
                    "operator \"{operator}\" right side: got \"{}\", but expected \"array\"!",
                    concat_type_name(&other)
                ))),
            },
            Value::Proplist(a) => match right {
                Value::Proplist(b) => {
                    let mut result = if operator == "..=" {
                        a
                    } else {
                        // AB_Concat forces a C4ValueHash copy before applying
                        // the RHS. Mapped values enter fresh Any slots through
                        // Set, unlike AB_ConcatIt's in-place destinations.
                        let mut copy = ValueMap::with_capacity(a.len());
                        for (key, value) in a {
                            c4_map_assign_set(&mut copy, key, value);
                        }
                        copy
                    };
                    for (key, value) in b {
                        c4_map_assign_set(&mut result, key, value);
                    }
                    Ok(Value::Proplist(result))
                }
                other => Err(RuntimeError::new(format!(
                    "operator \"{operator}\" right side: got \"{}\", but expected \"map\"!",
                    concat_type_name(&other)
                ))),
            },
            left => {
                let left = concat_string(&left).ok_or_else(|| {
                    RuntimeError::new(format!(
                        "operator \"{operator}\" left side: can not convert \"{}\" to \"string\", \"array\" or \"map\"!",
                        concat_type_name(&left)
                    ))
                })?;
                let right = concat_string(&right).ok_or_else(|| {
                    RuntimeError::new(format!(
                        "operator \"{operator}\" right side: can not convert \"{}\" to \"string\"!",
                        concat_type_name(&right)
                    ))
                })?;
                let mut bytes = c4_string_bytes(&left);
                bytes.extend(c4_string_bytes(&right));
                Ok(Value::String(c4_string_from_bytes(&bytes).into()))
            }
        }
    }

    fn eval_add(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        // C++ AB_Sum (C4AulExec.cpp:538-545): integer add with `_getInt()`
        // coercion (nil->0, bool->0/1). wrapping_add matches C++ 2's-complement
        // overflow instead of panicking in debug builds. String concatenation
        // belongs exclusively to AB_Concat (`..`).
        match (left.as_c4_int(), right.as_c4_int()) {
            (Some(x), Some(y)) => Ok(Value::Int(x.wrapping_add(y))),
            _ => Err(RuntimeError::new(format!(
                "cannot apply '+' to operands of type {} and {}",
                left.type_name(),
                right.type_name()
            ))),
        }
    }

    fn eval_int_op<F>(
        &self,
        left: Value,
        right: Value,
        op: F,
        symbol: &str,
    ) -> Result<Value, RuntimeError>
    where
        F: Fn(i32, i32) -> i32,
    {
        // Coerce operands like C++ `_getInt()` (nil->0, bool->0/1) for every
        // integer operator (C4AulExec.cpp `CheckOpPars<C4V_Any, ...>`).
        match (left.as_c4_int(), right.as_c4_int()) {
            (Some(a), Some(b)) => Ok(Value::Int(op(a, b))),
            _ => Err(RuntimeError::new(format!(
                "cannot apply '{symbol}' to operands of type {} and {}",
                left.type_name(),
                right.type_name()
            ))),
        }
    }

    fn eval_int_cmp<F>(
        &self,
        left: Value,
        right: Value,
        cmp: F,
        symbol: &str,
    ) -> Result<Value, RuntimeError>
    where
        F: Fn(i32, i32) -> bool,
    {
        // C++ comparisons (<, <=, >, >=) coerce both sides via `_getInt()` and
        // return a bool (C4AulExec.cpp:562-592).
        match (left.as_c4_int(), right.as_c4_int()) {
            (Some(a), Some(b)) => Ok(Value::Bool(cmp(a, b))),
            _ => Err(RuntimeError::new(format!(
                "cannot apply '{symbol}' to operands of type {} and {}",
                left.type_name(),
                right.type_name()
            ))),
        }
    }

    fn eval_string_cmp<F>(
        &self,
        left: Value,
        right: Value,
        strict: Option<u8>,
        cmp: F,
        symbol: &str,
    ) -> Result<Value, RuntimeError>
    where
        F: Fn(&[u8], &[u8]) -> bool,
    {
        // CheckOpPars for S=/eq/ne converts the left operand first, then the
        // right (C4AulExec.cpp:289-299,691-707). At the supported NONSTRICT
        // and STRICT1 levels, raw-falsy concrete values are Set0() before
        // conversion and therefore compare as the empty string.
        let convert = |value: Value, side: &str| {
            let canonical_nil = match &value {
                Value::Nil | Value::Object(0) => true,
                Value::C4Id(id) => crate::value::c4_id_raw(id) == 0,
                _ => false,
            };
            let typed_falsy = matches!(
                &value,
                Value::Int(0) | Value::Bool(false) | Value::RawBool(0)
            );
            if canonical_nil || (strict.unwrap_or(0) < 3 && typed_falsy) {
                return Ok(String::new());
            }
            match value {
                Value::String(text) => Ok(text.into_string()),
                Value::Nil => Ok(String::new()),
                other => Err(RuntimeError::new(format!(
                    "operator \"{symbol}\" {side} side: got \"{}\", but expected \"string\"!",
                    other.type_name()
                ))),
            }
        };
        let left_str = convert(left, "left")?;
        let right_str = convert(right, "right")?;
        let mut left_bytes = c4_string_bytes(&left_str);
        let mut right_bytes = c4_string_bytes(&right_str);
        if let Some(nul) = left_bytes.iter().position(|byte| *byte == 0) {
            left_bytes.truncate(nul);
        }
        if let Some(nul) = right_bytes.iter().position(|byte| *byte == 0) {
            right_bytes.truncate(nul);
        }
        Ok(Value::Bool(cmp(&left_bytes, &right_bytes)))
    }

    /// `==` per `C4Value::Equals` (C4Value.cpp:823-919). NONSTRICT/STRICT1
    /// compare the raw Data union, so pointer-backed values need their VM-side
    /// provenance; STRICT2 compares their content and keeps numeric leniency;
    /// STRICT3 requires matching outer types.
    fn values_equal(
        &self,
        left: &Value,
        right: &Value,
        strict: Option<u8>,
        left_identity: Option<&RawIdentity>,
        right_identity: Option<&RawIdentity>,
    ) -> bool {
        c4_values_equal(left, right, strict, left_identity, right_identity)
    }

    fn eval_index(
        &self,
        collection: Value,
        index: Value,
        env: &Environment,
    ) -> Result<Value, RuntimeError> {
        match (&collection, index) {
            (Value::Object(0), _) => Err(RuntimeError::new(
                "indexed access [index]: array, map or string expected, but got nil",
            )),
            (Value::Array(elements), index) => Ok(elements
                .get(array_index(&index)?)
                .cloned()
                .unwrap_or(Value::Nil)),
            (Value::String(text), index) => string_index(text, &index),
            (Value::Proplist(entries), key) => {
                Ok(entries.get_key(&key).cloned().unwrap_or(Value::Nil))
            }
            (target @ Value::Object(_), Value::String(name)) => {
                Ok(self.object_local_value(env, target, &name))
            }
            (Value::Object(_), _) => Err(RuntimeError::new(
                "indexed access on object: only string keys are allowed",
            )),
            (other, _) => Err(RuntimeError::new(format!(
                "cannot index value of type {}",
                other.type_name()
            ))),
        }
    }

    fn eval_index_tracked(
        &self,
        collection: TrackedValue,
        index: Value,
        env: &Environment,
    ) -> Result<TrackedValue, RuntimeError> {
        match (&collection.value, &index) {
            (Value::Object(0), _) => {
                return self
                    .eval_index(collection.value, index, env)
                    .map(TrackedValue::runtime);
            }
            (target @ Value::Object(_), Value::String(name)) => {
                return Ok(self.object_local_tracked(env, target, name));
            }
            (Value::Object(_), _) => {
                return self
                    .eval_index(collection.value, index, env)
                    .map(TrackedValue::runtime);
            }
            _ => {}
        }

        let segment = PathSegment::Index(index.clone());
        let string_result = matches!(&collection.value, Value::String(_));
        let inherited_identity = collection.identity_at(&segment);
        let value = self.eval_index(collection.value, index, env)?;
        let identity = if string_result {
            RawIdentity::runtime(&value)
        } else {
            inherited_identity
        };
        Ok(TrackedValue { value, identity })
    }

    fn eval_property(
        &self,
        value: Value,
        name: &str,
        env: &Environment,
    ) -> Result<Value, RuntimeError> {
        match &value {
            Value::Object(0) => Err(RuntimeError::new(
                "map access with .: map expected, but got nil!",
            )),
            Value::Proplist(entries) => Ok(entries.get(name).cloned().unwrap_or(Value::Nil)),
            target @ Value::Object(_) => Ok(self.object_local_value(env, target, name)),
            other => Err(RuntimeError::new(format!(
                "cannot access property '{name}' on value of type {}",
                other.type_name()
            ))),
        }
    }

    fn eval_property_tracked(
        &self,
        collection: TrackedValue,
        name: &str,
        env: &Environment,
    ) -> Result<TrackedValue, RuntimeError> {
        match &collection.value {
            Value::Object(0) => self
                .eval_property(collection.value, name, env)
                .map(TrackedValue::runtime),
            target @ Value::Object(_) => Ok(self.object_local_tracked(env, target, name)),
            _ => {
                let identity = collection.identity_at(&PathSegment::Property(name.to_string()));
                let value = self.eval_property(collection.value, name, env)?;
                Ok(TrackedValue { value, identity })
            }
        }
    }

    fn is_global_vm_builtin(name: &str) -> bool {
        matches!(
            name,
            "this"
                | "Var"
                | "VarN"
                | "Local"
                | "SetLocal"
                | "LocalN"
                | "Global"
                | "SetGlobal"
                | "GlobalN"
                | "eval"
        )
    }

    fn direct_call_function_known(&self, name: &str) -> bool {
        self.functions.contains_key(name)
            || self
                .global_functions
                .is_some_and(|functions| functions.contains_key(name))
            || self.has_host_function(name)
            || Self::is_global_vm_builtin(name)
            || self
                .direct_call_function_probe
                .map_or_else(|| self.method_dispatch.is_some(), |probe| probe(name))
    }

    fn global_builtin_int_arg(
        &self,
        name: &str,
        args: &[CallArg],
        index: usize,
    ) -> Result<i32, RuntimeError> {
        match args
            .get(index)
            .map(CallArg::read)
            .transpose()?
            .unwrap_or(Value::Nil)
        {
            Value::Int(value) => Ok(value),
            Value::Bool(value) => Ok(i32::from(value)),
            Value::RawBool(value) => Ok(value as u32 as i32),
            Value::Nil => Ok(0),
            other => Err(RuntimeError::new(format!(
                "call to \"{name}\" parameter {}: got \"{}\", but expected \"int\"!",
                index + 1,
                other.type_name()
            ))),
        }
    }

    fn global_builtin_object_arg(
        &self,
        name: &str,
        args: &[CallArg],
        index: usize,
    ) -> Result<Option<Value>, RuntimeError> {
        match args
            .get(index)
            .map(CallArg::read)
            .transpose()?
            .unwrap_or(Value::Nil)
        {
            Value::Nil
            | Value::Int(0)
            | Value::Bool(false)
            | Value::RawBool(0)
            | Value::Object(0) => Ok(None),
            value @ Value::Object(_) => Ok(Some(value)),
            other => Err(RuntimeError::new(format!(
                "call to \"{name}\" parameter {}: got \"{}\", but expected \"object\"!",
                index + 1,
                other.type_name()
            ))),
        }
    }

    fn global_builtin_string_arg(
        &self,
        name: &str,
        args: &[CallArg],
        index: usize,
        strict_level: Option<u8>,
    ) -> Result<String, RuntimeError> {
        match args
            .get(index)
            .map(CallArg::read)
            .transpose()?
            .unwrap_or(Value::Nil)
        {
            Value::String(value) => Ok(value.into_string()),
            Value::Nil => Ok(String::new()),
            Value::Int(0) | Value::Bool(false) | Value::RawBool(0)
                if strict_level.unwrap_or(0) < 3 =>
            {
                Ok(String::new())
            }
            other => Err(RuntimeError::new(format!(
                "call to \"{name}\" parameter {}: got \"{}\", but expected \"string\"!",
                index + 1,
                other.type_name()
            ))),
        }
    }

    /// FnVarN resolves only the immediate script caller's `Func->VarNamed`
    /// storage and preserves the cell as a live reference. A direct host
    /// dispatch has no suspended script caller and therefore yields nil.
    fn invoke_varn_raw(
        &self,
        args: &[CallArg],
        caller: Option<&ScriptCallerContext>,
    ) -> Result<ReturnValue, RuntimeError> {
        let strict_level = caller.and_then(|caller| caller.origin_strict_level);
        let name = self.global_builtin_string_arg("VarN", args, 0, strict_level)?;
        Ok(
            match caller.and_then(|caller| {
                caller
                    .frame_locals
                    .function_vars
                    .borrow()
                    .get(&name)
                    .map(Binding::lvalue)
            }) {
                Some(reference) => ReturnValue::Reference(reference),
                None => ReturnValue::Value(TrackedValue::runtime(Value::Nil)),
            },
        )
    }

    /// Dispatch a VM builtin from an unqualified call after its arguments have
    /// already been evaluated by the continuation machine. The global raw
    /// entry point deliberately has a nil object target for `global->...`; an
    /// ordinary direct call instead uses the executing frame/object, just as
    /// the recursive evaluator does above (C4Script.cpp:3390-3433,
    /// 4591-4617).
    fn invoke_direct_vm_builtin_call_args(
        &self,
        name: &str,
        args: &CallArgs,
        env: &mut Environment,
        return_reference: bool,
    ) -> Result<Option<ReturnValue>, RuntimeError> {
        let value = |value| ReturnValue::Value(TrackedValue::runtime(value));
        let result = match name {
            "this" => None,
            "Par" if args.len() <= 1 => {
                let index = args
                    .first()
                    .map(CallArg::read)
                    .transpose()?
                    .map(|value| match value {
                        Value::Int(index) => Ok(index),
                        Value::Nil => Ok(0),
                        Value::Bool(flag) => Ok(i32::from(flag)),
                        Value::RawBool(raw) => Ok(raw as u32 as i32),
                        other => Err(RuntimeError::new(format!(
                            "Par: index of type {}, int expected",
                            other.type_name()
                        ))),
                    })
                    .transpose()?
                    .unwrap_or(0);
                let reference = usize::try_from(index)
                    .ok()
                    .filter(|index| *index < MAX_CALL_PARAMETERS)
                    .and_then(|index| env.call_args.get(index))
                    .map(Binding::lvalue)
                    .unwrap_or_else(|| Binding::direct(Value::Nil).lvalue());
                Some(if return_reference {
                    ReturnValue::Reference(reference)
                } else {
                    ReturnValue::Value(reference.read_tracked()?)
                })
            }
            "Var" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                let reference = self.tracked_cell(frame_slot_cell(&env.frame_locals, index));
                Some(if return_reference {
                    ReturnValue::Reference(reference)
                } else {
                    ReturnValue::Value(reference.read_tracked()?)
                })
            }
            "VarN" => {
                let local_name = self.global_builtin_string_arg(name, args, 0, env.strict_level)?;
                Some(match env.function_var_lvalue(&local_name) {
                    Some(reference) if return_reference => ReturnValue::Reference(reference),
                    Some(reference) => ReturnValue::Value(reference.read_tracked()?),
                    None => value(Value::Nil),
                })
            }
            "Global" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                let reference = self.tracked_cell(self.numbered_global_cell(index)?);
                Some(if return_reference {
                    ReturnValue::Reference(reference)
                } else {
                    ReturnValue::Value(reference.read_tracked()?)
                })
            }
            "GlobalN" => {
                let local_name = self.global_builtin_string_arg(name, args, 0, env.strict_level)?;
                Some(match self.global_variable_cell(&local_name) {
                    Some(cell) if return_reference => {
                        ReturnValue::Reference(self.tracked_cell(cell))
                    }
                    Some(cell) => ReturnValue::Value(self.read_tracked_cell(&cell)),
                    None => value(Value::Nil),
                })
            }
            "Local" if args.len() <= 1 => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                if self.retain_global_call_context_for_host_paths || index < 0 {
                    Some(value(Value::Nil))
                } else {
                    let reference = self.tracked_cell(env.object_state.local_slot_cell(index));
                    Some(if return_reference {
                        ReturnValue::Reference(reference)
                    } else {
                        ReturnValue::Value(reference.read_tracked()?)
                    })
                }
            }
            "Local" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                if index < 0 {
                    Some(value(Value::Nil))
                } else {
                    let target = args.get(1).map(CallArg::read).transpose()?;
                    let reference = self.tracked_cell(self.numbered_local_cell(env, index, target));
                    Some(if return_reference {
                        ReturnValue::Reference(reference)
                    } else {
                        ReturnValue::Value(reference.read_tracked()?)
                    })
                }
            }
            "LocalN" if (1..=2).contains(&args.len()) => {
                let local_name = self.global_builtin_string_arg(name, args, 0, env.strict_level)?;
                let target = args.get(1).map(CallArg::read).transpose()?;
                let target_is_falsy = target.as_ref().is_none_or(|target| {
                    matches!(
                        target,
                        Value::Nil
                            | Value::Int(0)
                            | Value::Bool(false)
                            | Value::RawBool(0)
                            | Value::Object(0)
                    )
                });
                if self.retain_global_call_context_for_host_paths && target_is_falsy {
                    Some(value(Value::Nil))
                } else {
                    let reference = self.tracked_cell(self.localn_cell(env, &local_name, target));
                    Some(if return_reference {
                        ReturnValue::Reference(reference)
                    } else {
                        ReturnValue::Value(reference.read_tracked()?)
                    })
                }
            }
            "SetLocal" => Some(
                self.set_local_evaluated_tracked(args, None, env, 3)
                    .map(ReturnValue::Value)?,
            ),
            "SetGlobal" => Some(self.invoke_global_builtin_raw(name, args, env, 0)?),
            _ => None,
        };
        Ok(result)
    }

    fn invoke_global_builtin_raw(
        &self,
        name: &str,
        args: &[CallArg],
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        let value = |value| ReturnValue::Value(TrackedValue::runtime(value));
        match name {
            "this" => Ok(value(Value::Nil)),
            "Var" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                Ok(ReturnValue::Reference(
                    self.tracked_cell(frame_slot_cell(&env.frame_locals, index)),
                ))
            }
            "VarN" => {
                let name = self.global_builtin_string_arg(name, args, 0, env.strict_level)?;
                match env.function_var_lvalue(&name) {
                    Some(reference) => Ok(ReturnValue::Reference(reference)),
                    None => Ok(value(Value::Nil)),
                }
            }
            "Global" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                Ok(ReturnValue::Reference(
                    self.tracked_cell(self.numbered_global_cell(index)?),
                ))
            }
            "SetGlobal" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                let tracked = args
                    .get(1)
                    .map(CallArg::read_tracked)
                    .transpose()?
                    .unwrap_or_else(|| TrackedValue::runtime(Value::Nil));
                // Native C4V_Any parameter conversion canonicalizes every
                // falsy value to nil for callers below strict 3
                // (C4AulExec.cpp:1435-1439).
                let tracked = if env.strict_level.unwrap_or(0) < 3 && !tracked.value.as_bool() {
                    TrackedValue::runtime(Value::Nil)
                } else {
                    tracked
                };
                self.tracked_cell(self.numbered_global_cell(index)?)
                    .write_tracked(tracked.clone())?;
                Ok(ReturnValue::Value(tracked))
            }
            "GlobalN" => {
                let name = self.global_builtin_string_arg(name, args, 0, env.strict_level)?;
                match self.global_variable_cell(&name) {
                    Some(cell) => Ok(ReturnValue::Reference(self.tracked_cell(cell))),
                    None => Ok(value(Value::Nil)),
                }
            }
            "Local" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                let Some(target) = self.global_builtin_object_arg(name, args, 1)? else {
                    return Ok(value(Value::Nil));
                };
                if index < 0 {
                    return Ok(value(Value::Nil));
                }
                Ok(ReturnValue::Reference(self.tracked_cell(
                    self.numbered_local_cell(env, index, Some(target)),
                )))
            }
            "LocalN" => {
                let local_name = self.global_builtin_string_arg(name, args, 0, env.strict_level)?;
                let Some(target) = self.global_builtin_object_arg(name, args, 1)? else {
                    return Ok(value(Value::Nil));
                };
                Ok(ReturnValue::Reference(self.tracked_cell(self.localn_cell(
                    env,
                    &local_name,
                    Some(target),
                ))))
            }
            "SetLocal" => {
                let index = self.global_builtin_int_arg(name, args, 0)?;
                let tracked = args
                    .get(1)
                    .map(CallArg::read_tracked)
                    .transpose()?
                    .unwrap_or_else(|| TrackedValue::runtime(Value::Nil));
                let Some(target) = self.global_builtin_object_arg(name, args, 2)? else {
                    return Ok(value(Value::Bool(false)));
                };
                self.tracked_cell(self.numbered_local_cell(env, index, Some(target)))
                    .write_tracked(tracked.clone())?;
                Ok(ReturnValue::Value(tracked))
            }
            "eval" => {
                let code = match args.first().map(CallArg::read).transpose()? {
                    Some(Value::String(code)) => code,
                    _ => return Ok(value(Value::Nil)),
                };
                let cells = LocalCells {
                    state: env.object_state.clone(),
                };
                if let Some(result) = self.eval_direct_exec_continuation_hook.and_then(|hook| {
                    hook(
                        &code,
                        &cells,
                        self.this_value.clone(),
                        env.strict_level,
                        depth + 1,
                    )
                }) {
                    return match result {
                        Ok(ScriptCallOutcome::Complete(value)) => {
                            Ok(ReturnValue::Value(TrackedValue::runtime(value)))
                        }
                        Ok(ScriptCallOutcome::Suspended(suspension)) => {
                            let (request, resume_value, continuation) = suspension.into_parts();
                            Err(RuntimeError::new(
                                "script execution suspended by nested host callback",
                            )
                            .with_control(
                                RuntimeControl::HostContinuation {
                                    request,
                                    resume_value,
                                    continuation: Some(continuation),
                                },
                            ))
                        }
                        Err(error) => Err(error),
                    };
                }
                if let Some(result) = self.eval_direct_exec_hook.and_then(|hook| {
                    hook(
                        &code,
                        &cells,
                        self.this_value.clone(),
                        env.strict_level,
                        depth + 1,
                    )
                }) {
                    return result.map(|value| ReturnValue::Value(TrackedValue::runtime(value)));
                }
                let (eval_cells, definition_context) =
                    if self.retain_global_call_context_for_host_paths {
                        (LocalCells::default(), false)
                    } else {
                        (cells, env.definition_context)
                    };
                let direct_vm = self.clone().with_definition_context(definition_context);
                match direct_vm.eval_direct_exec_with_cells_with_continuation(
                    &code,
                    &eval_cells,
                    env.strict_level,
                    depth + 1,
                )? {
                    ScriptCallOutcome::Complete(value) => {
                        Ok(ReturnValue::Value(TrackedValue::runtime(value)))
                    }
                    ScriptCallOutcome::Suspended(suspension) => {
                        let (request, resume_value, continuation) = suspension.into_parts();
                        Err(
                            RuntimeError::new("script execution suspended by nested host callback")
                                .with_control(RuntimeControl::HostContinuation {
                                    request,
                                    resume_value,
                                    continuation: Some(continuation),
                                }),
                        )
                    }
                }
            }
            _ => Err(RuntimeError::new(format!("unknown function '{name}'"))),
        }
    }

    fn effect_slot_from_registered_host_call_args(
        &self,
        function: &RegisteredHostFunction,
        evaluated_args: CallArgs,
        env: &mut Environment,
    ) -> Result<ReturnValue, RuntimeError> {
        let _parameter_slots =
            ValueStackReservation::reserve(function.parameter_count().unwrap_or(3))?;
        let caller = env.caller_context();
        let _guard = CallerContextGuard::enter(Some(caller.clone()));
        let prepared_args =
            self.prepare_registered_host_call_args("EffectVar", function, evaluated_args)?;
        let args = self.call_args_to_values(&prepared_args)?.into_vec();
        Ok(ReturnValue::Reference(LValueRef::HostPath {
            function: function.callback().clone(),
            args,
            caller,
            global_call_context_hook: self
                .retain_global_call_context_for_host_paths
                .then(|| self.global_call_context_hook.cloned())
                .flatten(),
            segments: Vec::new(),
            legacy_pin: None,
        }))
    }

    fn effect_slot_from_call_args(
        &self,
        evaluated_args: CallArgs,
        env: &mut Environment,
    ) -> Result<ReturnValue, RuntimeError> {
        if let Some(function) = self.host_functions.get("EffectVar") {
            return self.effect_slot_from_registered_host_call_args(function, evaluated_args, env);
        }

        let _parameter_slots = ValueStackReservation::reserve(3)?;
        let raw_arg_values = evaluated_args
            .iter()
            .map(CallArg::read)
            .collect::<Result<CallValues, _>>()?;
        let slot_name = format!(
            "__effect_{}",
            raw_arg_values
                .iter()
                .map(|value| match value {
                    Value::Int(value) => value.to_string(),
                    Value::String(value) => value.to_string(),
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>()
                .join("_")
        );
        if env.get(&slot_name)?.is_none() {
            env.define(&slot_name, Value::Nil);
        }
        let reference = env
            .lvalue(&slot_name)
            .ok_or_else(|| RuntimeError::new("EffectVar slot disappeared"))?;
        Ok(ReturnValue::Reference(reference))
    }

    /// Arrow method lvalue entry for the continuation executor. Its argument
    /// task has already evaluated every operand exactly once, so this path
    /// must preserve the method's reference result without reconstructing an
    /// expression after a host boundary.
    #[allow(clippy::too_many_arguments)]
    fn invoke_method_reference_call_args_raw(
        &self,
        mut target: Value,
        name: &str,
        evaluated_args: CallArgs,
        target_sweep_cursor: usize,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        if let Value::Proplist(map) = &target {
            if let Some(Value::Int(id)) = map.get("id") {
                if *id > 0 {
                    target = Value::Object(*id as u64);
                }
            }
        }
        clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
        if matches!(
            &target,
            Value::Nil | Value::Int(0) | Value::Bool(false) | Value::RawBool(0) | Value::Object(0)
        ) || matches!(&target, Value::C4Id(id) if crate::value::c4_id_raw(id) == 0)
        {
            return Err(RuntimeError::new("Object call: target is zero!"));
        }

        if matches!(&target, Value::Object(_)) && !self.object_target_available(&target) {
            return Err(RuntimeError::new("Object call: target is zero!"));
        }

        // FnLocal/LocalN return the selected object's live C4Value cell
        // (C4Script.cpp:3423-3433,4591-4605), including through an arrow. An
        // explicit object argument selects that object over the arrow
        // target (clonk-org/clonk-rs#1531).
        if (1..=2).contains(&evaluated_args.len()) && name == "LocalN" {
            let local_name = match evaluated_args[0].read()? {
                Value::String(name) => name,
                other => {
                    return Err(RuntimeError::new(format!(
                        "LocalN: expected string for name, got {}",
                        other.type_name()
                    )))
                }
            };
            let explicit = evaluated_args
                .get(1)
                .map(|argument| argument.read())
                .transpose()?;
            let owner = self.explicit_local_owner(explicit).unwrap_or(target);
            return Ok(ReturnValue::Reference(self.tracked_cell(self.localn_cell(
                env,
                &local_name,
                Some(owner),
            ))));
        }
        if (1..=2).contains(&evaluated_args.len()) && name == "Local" {
            let index = Self::slot_index_from_value("Local()", evaluated_args[0].read()?)?;
            let explicit = evaluated_args
                .get(1)
                .map(|argument| argument.read())
                .transpose()?;
            let owner = self.explicit_local_owner(explicit).unwrap_or(target);
            return Ok(ReturnValue::Reference(
                self.tracked_cell(self.numbered_local_cell(env, index, Some(owner))),
            ));
        }

        if let Some(dispatch) = self.method_reference_dispatch {
            let mut dispatch_args = Vec::with_capacity(evaluated_args.len() + 3);
            dispatch_args.push(target);
            dispatch_args.push(Value::String(name.to_owned().into()));
            dispatch_args.push(Value::Bool(false));
            for arg in &evaluated_args {
                dispatch_args.push(arg.read()?);
            }
            let _guard = CallerContextGuard::enter(Some(env.caller_context()));
            let _parameter_override = CallParameterOverrideGuard::enter(0);
            return dispatch(&dispatch_args)
                .map(ValueReference::into_lvalue)
                .map(ReturnValue::Reference);
        }

        // Without a host method bridge, an arrow call can still select a
        // script `func &` from the executing object context. The continuation
        // call-result task owns the target and ten parameter slots already.
        let _parameter_override = CallParameterOverrideGuard::enter(0);
        self.invoke_reference(
            name,
            evaluated_args,
            depth + 1,
            env.object_state.clone(),
            Some(env.caller_context()),
        )
        .map(ReturnValue::Reference)
    }

    /// Object-call entry for the continuation executor. Its argument task has
    /// already evaluated every operand exactly once, so this path must never
    /// call `build_call_args` (doing so would replay side effects after a
    /// resumed nested host call). The ordinary recursive evaluator above uses
    /// `invoke_property_call_with_target_raw` and remains responsible for
    /// constructing the call arguments itself.
    #[allow(clippy::too_many_arguments)]
    fn invoke_property_call_with_target_call_args_raw(
        &self,
        mut target: Value,
        name: &str,
        evaluated_args: CallArgs,
        failsafe: bool,
        return_reference: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        let target_sweep_cursor = object_reference_sweep_cursor();
        if let Value::Proplist(map) = &target {
            if let Some(Value::Int(id)) = map.get("id") {
                if *id > 0 {
                    target = Value::Object(*id as u64);
                }
            }
        }
        if failsafe && !self.direct_call_function_known(name) {
            // An unresolved `->~name` was compiled without AB_CALLFS. Its
            // explicit operands have already run in the continuation path;
            // the zero target is therefore discarded with a nil result
            // (C4AulParse.cpp:3215-3231).
            return Ok(ReturnValue::Value(TrackedValue::runtime(Value::Nil)));
        }

        if matches!(&target, Value::Object(id) if *id != 0)
            && name == "LocalN"
            && (1..=2).contains(&evaluated_args.len())
            && !self.functions.contains_key(name)
        {
            clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
            if !self.object_target_available(&target) {
                return Err(RuntimeError::new("Object call: target is zero!"));
            }
            let local_name = match evaluated_args[0].read()? {
                Value::String(local_name) => local_name,
                other => {
                    return Err(RuntimeError::new(format!(
                        "LocalN: expected string for name, got {}",
                        other.type_name()
                    )))
                }
            };
            let explicit = evaluated_args
                .get(1)
                .map(|argument| argument.read())
                .transpose()?;
            let owner = self.explicit_local_owner(explicit).unwrap_or(target);
            return Ok(ReturnValue::Value(TrackedValue::runtime(
                self.localn_cell(env, &local_name, Some(owner))
                    .borrow()
                    .clone(),
            )));
        }

        if matches!(&target, Value::Object(id) if *id != 0)
            && name == "Local"
            && (1..=2).contains(&evaluated_args.len())
            && !self.functions.contains_key(name)
            && !self.has_host_function(name)
        {
            clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
            if !self.object_target_available(&target) {
                return Err(RuntimeError::new("Object call: target is zero!"));
            }
            let index = Self::slot_index_from_value("Local()", evaluated_args[0].read()?)?;
            if index < 0 {
                return Ok(ReturnValue::Value(TrackedValue::runtime(Value::Nil)));
            }
            let explicit = evaluated_args
                .get(1)
                .map(|argument| argument.read())
                .transpose()?;
            let owner = self.explicit_local_owner(explicit).unwrap_or(target);
            return Ok(ReturnValue::Value(TrackedValue::runtime(
                self.numbered_local_cell(env, index, Some(owner))
                    .borrow()
                    .clone(),
            )));
        }

        if matches!(&target, Value::Object(id) if *id != 0)
            && name == "SetLocal"
            && !self.functions.contains_key(name)
            && !self.has_host_function(name)
        {
            clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
            if !self.object_target_available(&target) {
                return Err(RuntimeError::new("Object call: target is zero!"));
            }
            let index = Self::slot_index_from_value(
                "SetLocal()",
                evaluated_args
                    .first()
                    .map(CallArg::read)
                    .transpose()?
                    .unwrap_or(Value::Nil),
            )?;
            let value = evaluated_args
                .get(1)
                .map(CallArg::read_tracked)
                .transpose()?
                .unwrap_or_else(|| TrackedValue::runtime(Value::Nil));
            let explicit_target = evaluated_args.get(2).map(CallArg::read).transpose()?;
            let target = explicit_target
                .filter(|value| {
                    !matches!(
                        value,
                        Value::Nil
                            | Value::Int(0)
                            | Value::Bool(false)
                            | Value::RawBool(0)
                            | Value::Object(0)
                    )
                })
                .unwrap_or(target);
            self.tracked_cell(self.numbered_local_cell(env, index, Some(target)))
                .write_tracked(value.clone())?;
            return Ok(ReturnValue::Value(value));
        }

        if matches!(
            &target,
            Value::Nil | Value::Int(0) | Value::Bool(false) | Value::RawBool(0) | Value::Object(0)
        ) || matches!(&target, Value::C4Id(id) if crate::value::c4_id_raw(id) == 0)
        {
            return Err(RuntimeError::new("Object call: target is zero!"));
        }

        match &target {
            Value::Object(_) | Value::C4Id(_) if self.method_dispatch.is_some() => {
                clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
                if !self.object_target_available(&target) {
                    return Err(RuntimeError::new("Object call: target is zero!"));
                }
                let mut dispatch_args = Vec::with_capacity(evaluated_args.len() + 3);
                dispatch_args.push(target.clone());
                dispatch_args.push(Value::String(name.to_owned().into()));
                dispatch_args.push(Value::Bool(failsafe));
                for arg in &evaluated_args {
                    dispatch_args.push(arg.read()?);
                }
                let references_out = evaluated_args
                    .iter()
                    .any(|arg| matches!(arg, CallArg::Reference(_)))
                    .then_some(self.method_ref_args_dispatch)
                    .flatten();
                let dispatch = self
                    .method_dispatch
                    .ok_or_else(|| RuntimeError::new("method dispatch vanished"))?;
                let _guard = CallerContextGuard::enter(Some(env.caller_context()));
                let _parameter_override = CallParameterOverrideGuard::enter(0);
                let Some(references_out) = references_out else {
                    return dispatch(&dispatch_args)
                        .map(|value| ReturnValue::Value(TrackedValue::runtime(value)));
                };
                let (result, finals) = references_out(&dispatch_args)?;
                for (arg, settled) in evaluated_args.iter().zip(finals) {
                    if let CallArg::Reference(reference) = arg {
                        if reference.read()? != settled {
                            reference.write(settled)?;
                        }
                    }
                }
                Ok(ReturnValue::Value(TrackedValue::runtime(result)))
            }
            Value::Object(_) | Value::C4Id(_) => {
                let _parameter_override = CallParameterOverrideGuard::enter(0);
                let result = self.invoke_raw(
                    name,
                    evaluated_args,
                    depth + 1,
                    env.object_state.clone(),
                    Some(env.caller_context()),
                )?;
                Ok(if return_reference {
                    result
                } else {
                    materialize_target_call_result(result)
                })
            }
            other if self.method_dispatch.is_some() => Err(RuntimeError::new(format!(
                "Object call: Invalid target type {}, expected object or id!",
                other.type_name()
            ))),
            _ => {
                let _parameter_override = CallParameterOverrideGuard::enter(0);
                let result = self.invoke_raw(
                    name,
                    evaluated_args,
                    depth + 1,
                    env.object_state.clone(),
                    Some(env.caller_context()),
                )?;
                Ok(if return_reference {
                    result
                } else {
                    materialize_target_call_result(result)
                })
            }
        }
    }

    /// `Callee(args, ...)`: after the explicit arguments, forward every
    /// parameter slot of the executing function past its named parameters,
    /// stopping at the resolved callee's declared frame size
    /// (C4AulParse.cpp:2293-2306). Direct native calls use their exact arity;
    /// script, object and global calls retain the 10-slot frame.
    fn append_forwarded_args(
        evaluated_args: &mut CallArgs,
        env: &Environment,
        parameter_limit: usize,
    ) -> Result<(), RuntimeError> {
        let mut index = env.named_param_count;
        while evaluated_args.len() < parameter_limit && index < MAX_CALL_PARAMETERS {
            let forwarded = env
                .call_args
                .get(index)
                // Parse_Params emits AB_PARN_R for `...`. A reference-typed
                // destination keeps the alias; non-reference conversion later
                // dereferences through C4Value::Set/FnCnvDeref.
                .map(|binding| CallArg::Reference(binding.lvalue()))
                .unwrap_or_else(|| CallArg::runtime(Value::Nil));
            evaluated_args.push(forwarded);
            index += 1;
        }
        // Fresh value-nil tails are indistinguishable from missing C++ slots
        // and may be dropped for host arity. A forwarded reference whose
        // current value is nil must remain: a `&` callee can write through it.
        while matches!(
            evaluated_args.last(),
            Some(CallArg::Value(TrackedValue {
                value: Value::Nil,
                ..
            }))
        ) {
            evaluated_args.pop();
        }
        Ok(())
    }

    fn property_reference_or_value_with_hook_stack(
        &self,
        base: ReturnValue,
        property: &str,
        env: &Environment,
        hook_stack_slots: Option<usize>,
    ) -> Result<ReturnValue, RuntimeError> {
        match base {
            ReturnValue::Value(value) if matches!(value.value, Value::Nil | Value::Object(0)) => {
                Err(RuntimeError::new(
                    "map access with .: map expected, but got nil!",
                ))
            }
            ReturnValue::Value(value) => {
                let _hook_stack = compiled_object_hook_stack(&value.value, hook_stack_slots)?;
                self.eval_property_tracked(value, property, env)
                    .map(ReturnValue::Value)
            }
            ReturnValue::Reference(reference) => {
                if let Some(resolved) = reference.resolved_legacy_value() {
                    let _hook_stack =
                        compiled_object_hook_stack(&resolved.value, hook_stack_slots)?;
                    return self
                        .eval_property_tracked(resolved, property, env)
                        .map(ReturnValue::Value);
                }
                if !legacy_path_pin_creation_active() {
                    let value = reference.read_tracked()?;
                    let _hook_stack = compiled_object_hook_stack(&value.value, hook_stack_slots)?;
                    return self
                        .eval_property_tracked(value, property, env)
                        .map(ReturnValue::Value);
                }
                if matches!(&reference, LValueRef::HostPath { .. }) {
                    return Ok(ReturnValue::Reference(
                        reference.append(PathSegment::Property(property.to_string()))?,
                    ));
                }
                let collection = reference.read()?;
                if matches!(collection, Value::Nil | Value::Object(0)) {
                    return Err(RuntimeError::new(
                        "map access with .: map expected, but got nil!",
                    ));
                }
                if matches!(collection, Value::Object(_)) {
                    let _hook_stack = compiled_object_hook_stack(&collection, hook_stack_slots)?;
                    let cell = self
                        .object_local_cell(env, &collection, property)
                        .unwrap_or_else(|| value_cell(Value::Nil));
                    return Ok(ReturnValue::Reference(self.tracked_cell(cell)));
                }
                if legacy_path_pin_creation_active() {
                    reference.detach_container_identity_if_shared();
                }
                Ok(ReturnValue::Reference(
                    reference.append(PathSegment::Property(property.to_string()))?,
                ))
            }
        }
    }

    fn index_value_reference_or_value_with_hook_stack(
        &self,
        base: ReturnValue,
        index: Value,
        env: &Environment,
        hook_stack_slots: Option<usize>,
    ) -> Result<ReturnValue, RuntimeError> {
        if !legacy_path_pin_creation_active() {
            let base = match base {
                ReturnValue::Value(value) => value,
                ReturnValue::Reference(reference) => {
                    if let Some(resolved) = reference.resolved_legacy_value() {
                        resolved
                    } else {
                        let collection = reference.read()?;
                        Self::grow_empty_negative_array(Some(&reference), &collection, &index)?;
                        reference.read_tracked()?
                    }
                }
            };
            if matches!(&base.value, Value::Nil | Value::Object(0)) {
                return Err(RuntimeError::new(
                    "indexed access [index]: array, map or string expected, but got nil",
                ));
            }
            let _hook_stack = compiled_object_hook_stack(&base.value, hook_stack_slots)?;
            return self
                .eval_index_tracked(base, index, env)
                .map(ReturnValue::Value);
        }
        match base {
            ReturnValue::Value(value) if matches!(value.value, Value::Nil | Value::Object(0)) => {
                Err(RuntimeError::new(
                    "indexed access [index]: array, map or string expected, but got nil",
                ))
            }
            ReturnValue::Value(value) => {
                let _hook_stack = compiled_object_hook_stack(&value.value, hook_stack_slots)?;
                self.eval_index_tracked(value, index, env)
                    .map(ReturnValue::Value)
            }
            ReturnValue::Reference(reference) => {
                if let Some(resolved) = reference.resolved_legacy_value() {
                    let _hook_stack =
                        compiled_object_hook_stack(&resolved.value, hook_stack_slots)?;
                    return self
                        .eval_index_tracked(resolved, index, env)
                        .map(ReturnValue::Value);
                }
                let collection = reference.read()?;
                if matches!(collection, Value::Nil | Value::Object(0)) {
                    return Err(RuntimeError::new(
                        "indexed access [index]: array, map or string expected, but got nil",
                    ));
                }
                if matches!(collection, Value::Object(_)) {
                    let Value::String(name) = &index else {
                        return Err(RuntimeError::new(
                            "indexed access on object: only string keys are allowed",
                        ));
                    };
                    let _hook_stack = compiled_object_hook_stack(&collection, hook_stack_slots)?;
                    let cell = self
                        .object_local_cell(env, &collection, name)
                        .unwrap_or_else(|| value_cell(Value::Nil));
                    return Ok(ReturnValue::Reference(self.tracked_cell(cell)));
                }
                if matches!(collection, Value::String(_)) {
                    let value = reference.read_tracked()?;
                    let _hook_stack = compiled_object_hook_stack(&value.value, hook_stack_slots)?;
                    return self
                        .eval_index_tracked(value, index, env)
                        .map(ReturnValue::Value);
                }
                if legacy_path_pin_creation_active() {
                    Self::grow_empty_negative_array(Some(&reference), &collection, &index)?;
                    reference.detach_container_identity_if_shared();
                }
                Ok(ReturnValue::Reference(
                    reference.append(PathSegment::Index(index))?,
                ))
            }
        }
    }

    /// AB_ARRAY_APPEND grows the referenced array immediately and leaves a
    /// live reference to its new nil slot (C4AulExec.cpp:971-981). Creating
    /// the slot here, rather than waiting for a later write, preserves the
    /// side effect of a plain `array[]` read and of an operator that errors.
    fn append_array_slot(&self, reference: LValueRef) -> Result<LValueRef, RuntimeError> {
        let array = reference.read()?;
        let length = match array {
            Value::Array(elements) => elements.len(),
            Value::Nil => {
                return Err(RuntimeError::new(
                    "array append accesss: can't access nil as an array!",
                ))
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "array append accesss: can't access {} as an array!",
                    other.type_name()
                )))
            }
        };
        reference.detach_container_identity_if_shared();
        if length >= ARRAY_MAX_SIZE {
            return Err(RuntimeError::new("out of memory"));
        }
        let index = i32::try_from(length).map_err(|_| RuntimeError::new("out of memory"))?;
        let appended = reference.append(PathSegment::Index(Value::Int(index)))?;
        appended.write(Value::Nil)?;
        Ok(appended)
    }

    fn grow_empty_negative_array(
        reference: Option<&LValueRef>,
        collection: &Value,
        index: &Value,
    ) -> Result<(), RuntimeError> {
        let grows = matches!(
            (collection, index.as_c4_int()),
            (Value::Array(elements), Some(raw_index))
                if elements.is_empty() && raw_index < 0
        );
        let Some(reference) = reference.filter(|_| grows) else {
            return Ok(());
        };

        // Avoid clobbering a nonempty or non-array value if evaluating the
        // index reassigned this Rust owner after the collection was read.
        if matches!(
            reference.read(),
            Ok(Value::Array(elements)) if elements.is_empty()
        ) {
            reference.write(Value::Array(vec![Value::Nil]))?;
        }
        Ok(())
    }

    fn slot_index_from_value(name: &str, value: Value) -> Result<i32, RuntimeError> {
        match value {
            Value::Int(index) => Ok(index),
            // Var/Local/SetLocal are typed C4ValueInt engine functions in
            // C++; C4Value::getInt converts nil to zero and bool directly
            // before FnVar/FnLocal sees the index (C4Value.h:159,317-321;
            // C4Value.cpp:453-466,499-522).
            Value::Nil => Ok(0),
            Value::Bool(flag) => Ok(i32::from(flag)),
            Value::RawBool(raw) => Ok(raw as u32 as i32),
            other => Err(RuntimeError::new(format!(
                "{name} index must be an integer, got {}",
                other.type_name()
            ))),
        }
    }

    fn set_local_evaluated_tracked(
        &self,
        evaluated_args: &CallArgs,
        default_target: Option<Value>,
        env: &mut Environment,
        parameter_slots: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        // The argument expressions have already run. This helper is shared by
        // the continuation dispatcher so an explicit target expression is not
        // evaluated a second time after the call frame is assembled.
        let _parameter_slots = ValueStackReservation::reserve(parameter_slots)?;
        let index = match evaluated_args
            .first()
            .map(CallArg::read)
            .transpose()?
            .unwrap_or(Value::Nil)
        {
            Value::Int(index) => index,
            Value::Nil => 0,
            Value::Bool(flag) => i32::from(flag),
            Value::RawBool(raw) => raw as u32 as i32,
            other => {
                return Err(RuntimeError::new(format!(
                    "SetLocal() index must be an integer, got {}",
                    other.type_name()
                )))
            }
        };
        let tracked = evaluated_args
            .get(1)
            .map(CallArg::read_tracked)
            .transpose()?
            .unwrap_or_else(|| TrackedValue::runtime(Value::Nil));
        let explicit_target = evaluated_args.get(2).map(CallArg::read).transpose()?;
        let target = explicit_target
            .filter(|value| {
                !matches!(
                    value,
                    Value::Nil
                        | Value::Int(0)
                        | Value::Bool(false)
                        | Value::RawBool(0)
                        | Value::Object(0)
                )
            })
            .or(default_target);
        if target.is_none() && self.retain_global_call_context_for_host_paths {
            return Ok(TrackedValue::runtime(Value::Bool(false)));
        }
        let cell = self.numbered_local_cell(env, index, target);
        self.tracked_cell(cell).write_tracked(tracked.clone())?;
        Ok(tracked)
    }
}

enum ControlFlow {
    Normal,
    Return(ReturnValue),
}

/// Result of a script call that may cross an embedding-owned synchronous
/// boundary. A suspension owns the complete compiled frame state; the VM and
/// its host tables are borrowed only while one run step is active.
pub enum ScriptCallOutcome {
    Complete(Value),
    Suspended(ScriptSuspension),
}

/// Result of a native callback that resumed a nested script call.  The
/// suspended child stays attached to the native phase machine until the host
/// completes that child's request; this prevents the VM from treating the
/// child as the native callback's final return value and replaying the native
/// prefix on the next call.
pub enum NativeCallOutcome {
    Complete(Value),
    Suspended {
        child: ScriptSuspension,
        continuation: Box<dyn NativeContinuation>,
    },
}

/// Owned suffix of a native callback that started a nested script call.
///
/// Implementations must retain only owned state.  In particular, an engine
/// adapter must reacquire its host context around every `resume_child` call;
/// retaining `&mut Engine`, a host-context borrow, or a TLS guard here would
/// let a section switch resume through stale state.  `resume` consumes the
/// adapter so a completed suffix cannot be invoked a second time.
pub trait NativeContinuation: 'static {
    /// Consume the native phase machine after the child completed (or failed)
    /// and either finish the native call or park its next child suspension.
    fn resume(
        self: Box<Self>,
        child_result: Result<Value, RuntimeError>,
    ) -> Result<NativeCallOutcome, RuntimeError>;

    /// Resume one retained child frame using the embedding engine that owns
    /// that child.  The implementation must enter and leave a fresh host
    /// context around this one slice, including when the child yields again.
    fn resume_child(
        &mut self,
        child: ScriptSuspension,
        value: Value,
    ) -> Result<ScriptCallOutcome, RuntimeError>;

    /// Sweep object references held by the native suffix at the real removal
    /// boundary.  The default is correct for adapters with no object-bearing
    /// state; engine adapters override it for child cells and retained IDs.
    fn clear_object_references(&mut self, _object_id: u64) {}

    /// Additional C4Value slots owned by the native suffix while it is parked.
    /// The child frame's own slots are counted by the VM recursively.
    fn value_stack_slots(&self) -> usize {
        0
    }
}

/// Lift a nested script call into the surrounding native callback's
/// `Result<Value, RuntimeError>` ABI.
///
/// A completed child immediately enters the native suffix.  A suspended
/// child becomes a host continuation containing both the child frame and the
/// owned suffix, so the outer script resumes the suffix exactly once after
/// the embedding commits the child's request.  A suffix may park another
/// child, allowing section switches (or other host boundaries) to repeat.
pub fn lift_native_continuation(
    child: ScriptCallOutcome,
    continuation: Box<dyn NativeContinuation>,
) -> Result<Value, RuntimeError> {
    match child {
        ScriptCallOutcome::Complete(value) => match continuation.resume(Ok(value))? {
            NativeCallOutcome::Complete(value) => Ok(value),
            NativeCallOutcome::Suspended {
                child,
                continuation,
            } => Err(native_continuation_error(child, continuation)),
        },
        ScriptCallOutcome::Suspended(child) => Err(native_continuation_error(child, continuation)),
    }
}

fn native_continuation_error(
    child: ScriptSuspension,
    continuation: Box<dyn NativeContinuation>,
) -> RuntimeError {
    RuntimeError::new("script execution suspended by nested native callback").with_control(
        RuntimeControl::HostContinuation {
            request: Rc::clone(&child.request),
            resume_value: child.resume_value.clone(),
            continuation: Some(Box::new(NativeContinuationState {
                child,
                continuation,
            })),
        },
    )
}

/// Internal result used while one suspended script frame resumes another.
/// A public call materializes a reference return into [`Value`], but a nested
/// caller must receive the original `ReturnValue` so an assignment can still
/// write through the same lvalue after the child crosses a host boundary.
enum ContinuationResult {
    Complete(ReturnValue),
    Suspended(ScriptSuspension),
}

pub struct ScriptSuspension {
    request: Rc<dyn Any>,
    resume_value: Value,
    continuation: Box<ScriptContinuation>,
    this_value: Value,
}

impl ScriptSuspension {
    pub fn request<T: Any>(&self) -> Option<&T> {
        self.request.as_ref().downcast_ref()
    }

    /// Clear a removed object's references from every owned frame, operand,
    /// argument and lvalue before the host performs its synchronous removal.
    /// Suspensions deliberately outlive the active-reference TLS registry, so
    /// this direct walk is the ownership boundary used by embedding engines.
    pub fn clear_object_references(&mut self, object_id: u64) {
        self.resume_value.clear_object_reference(object_id);
        self.this_value.clear_object_reference(object_id);
        self.continuation.clear_object_reference(object_id);
    }

    /// Reserve the complete captured C4Aul value-stack context while an
    /// embedding engine performs work synchronous with the yielding native
    /// call. The returned guard owns the TLS charge and borrows no part of
    /// this suspension, so the engine may release its suspension registry
    /// borrow, run nested callbacks, and clear object references before the
    /// guard is dropped. Stored continuation reservations remain detached.
    pub fn attach_value_stack_context(&self) -> Result<ScriptValueStackContext, RuntimeError> {
        self.continuation.attach_value_stack_context()
    }

    fn into_parts(self) -> (Rc<dyn Any>, Value, Box<ScriptContinuation>) {
        (self.request, self.resume_value, self.continuation)
    }

    pub(crate) fn resume(self, vm: &Vm<'_>) -> Result<ScriptCallOutcome, RuntimeError> {
        let vm = vm.clone().with_this(self.this_value);
        match self
            .continuation
            .resume_with_value(&vm, self.resume_value)?
        {
            ContinuationResult::Complete(value) => {
                value.into_value_on_stack().map(ScriptCallOutcome::Complete)
            }
            ContinuationResult::Suspended(suspension) => {
                Ok(ScriptCallOutcome::Suspended(suspension))
            }
        }
    }

    pub(crate) fn resume_with_value(
        self,
        vm: &Vm<'_>,
        value: Value,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        let vm = vm.clone().with_this(self.this_value);
        match self.continuation.resume_with_value(&vm, value)? {
            ContinuationResult::Complete(value) => {
                value.into_value_on_stack().map(ScriptCallOutcome::Complete)
            }
            ContinuationResult::Suspended(suspension) => {
                Ok(ScriptCallOutcome::Suspended(suspension))
            }
        }
    }

    /// Resume a frame with an embedding-selected receiver. The original
    /// receiver is retained for standalone suspension users, but an engine
    /// section switch can remove that object before the suffix runs. In that
    /// case the C++ callback resumes without the stale `this` object.
    pub(crate) fn resume_with_value_and_this(
        self,
        vm: &Vm<'_>,
        value: Value,
        this_value: Value,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        let vm = vm.clone().with_this(this_value);
        let this_value = vm.this_value.clone();
        match self
            .continuation
            .resume_with_value_and_this(&vm, value, this_value)?
        {
            ContinuationResult::Complete(value) => {
                value.into_value_on_stack().map(ScriptCallOutcome::Complete)
            }
            ContinuationResult::Suspended(suspension) => {
                Ok(ScriptCallOutcome::Suspended(suspension))
            }
        }
    }
}

/// Owned value-stack charge for synchronous work performed while a
/// [`ScriptSuspension`] remains detached. It intentionally carries no borrow
/// of the suspension so an embedding engine can reacquire or remove that
/// suspension during the guarded operation.
#[must_use = "drop the context after synchronous host work completes"]
pub struct ScriptValueStackContext {
    _reservation: ValueStackReservation,
}

struct ScriptContinuation {
    frame: ContinuationFrame,
    /// Receiver selected by the frame that yielded. A nested object call can
    /// be resumed through an outer VM after that outer host scope has been
    /// restored, so the child must carry its own `this` value.
    this_value: Value,
}

#[allow(clippy::large_enum_variant)]
enum ContinuationFrame {
    Compiled(CompiledContinuationFrame),
}

/// A suspended child and the native suffix that owns its next phase.  The
/// complete [`ScriptSuspension`] is retained so its request, receiver, cells,
/// and object-bearing operands remain sweepable while the parent is parked.
struct NativeContinuationState {
    child: ScriptSuspension,
    continuation: Box<dyn NativeContinuation>,
}

enum NativeResumeOutcome {
    Complete(Value),
    Suspended {
        request: Rc<dyn Any>,
        resume_value: Value,
        pending: PendingContinuation,
    },
}

fn resume_native_continuation(
    state: NativeContinuationState,
    resume_value: Value,
    parameter_slots: usize,
) -> Result<NativeResumeOutcome, RuntimeError> {
    let NativeContinuationState {
        child,
        mut continuation,
    } = state;
    // The callback's native frame was detached when the parent suspension was
    // handed to the host. Reacquire both that frame and the adapter-owned
    // slots for this one child-resume slice; both guards drop before a new
    // suspension is returned to the host.
    let _native_value_stack = ValueStackReservation::reserve(
        parameter_slots.saturating_add(continuation.value_stack_slots()),
    )?;
    let child_result = match continuation.resume_child(child, resume_value) {
        Ok(ScriptCallOutcome::Complete(value)) => Ok(value),
        Ok(ScriptCallOutcome::Suspended(child)) => {
            return Ok(native_resume_outcome(
                NativeCallOutcome::Suspended {
                    child,
                    continuation,
                },
                parameter_slots,
            ));
        }
        Err(error) => Err(error),
    };
    Ok(native_resume_outcome(
        continuation.resume(child_result)?,
        parameter_slots,
    ))
}

fn native_resume_outcome(
    outcome: NativeCallOutcome,
    parameter_slots: usize,
) -> NativeResumeOutcome {
    match outcome {
        NativeCallOutcome::Complete(value) => NativeResumeOutcome::Complete(value),
        NativeCallOutcome::Suspended {
            child,
            continuation,
        } => {
            let request = Rc::clone(&child.request);
            let resume_value = child.resume_value.clone();
            NativeResumeOutcome::Suspended {
                request,
                resume_value,
                pending: PendingContinuation::Native {
                    state: NativeContinuationState {
                        child,
                        continuation,
                    },
                    parameter_slots,
                },
            }
        }
    }
}

enum PendingContinuation {
    Host {
        value: Value,
        /// Native callback parameter slots remain live while the embedding
        /// engine completes the requested operation synchronously. They are
        /// counted by the inline context guard and are not reattached for
        /// ordinary script resumption.
        parameter_slots: usize,
    },
    Child(Box<ScriptContinuation>),
    Native {
        state: NativeContinuationState,
        /// The native callback's own parameter slots remain live while its
        /// suffix resumes the child.  The callback's reservation unwinds when
        /// the host takes ownership of the continuation, so this count must
        /// travel with the pending native state and be reacquired per slice.
        parameter_slots: usize,
    },
}

struct CompiledContinuationFrame {
    function: Arc<Function>,
    compiled: Arc<CompiledFunction>,
    call_targets: SmallVec<[CompiledCallBinding; 32]>,
    env: Environment,
    depth: usize,
    caller: Option<ScriptCallerContext>,
    returns_reference: bool,
    instruction: usize,
    stack: SmallVec<[ReturnValue; 16]>,
    registered_slots: SmallVec<[bool; 16]>,
    assignment_targets: SmallVec<[(usize, LValueRef); 4]>,
    iterators: SmallVec<[CompiledIterator; 2]>,
    /// The callee's ten parameter slots plus hoisted function vars remain
    /// live across a C++ host boundary. Re-acquire them only while this
    /// continuation is executing; dropping a standalone suspension must not
    /// charge an unrelated later call.
    frame_value_stack: usize,
    /// Operand prefix retained after the yielding call's arguments were
    /// removed. This reservation is detached while the host owns the
    /// continuation and reattached only for resume or inline work.
    stack_value_stack: ValueStackReservation,
    pending: PendingContinuation,
}

/// Queue-time resolution retained by an interpreted call. A C4Aul `AB_CALL`
/// stores its selected function pointer in the bytecode frame; resolving the
/// same name after an argument callback yields could otherwise observe a
/// later host registration or overload.
#[derive(Clone)]
enum RetainedCallTarget {
    Script(CompiledScriptTarget),
    Host(RegisteredHostFunction),
    HostReference(HostReferenceFunction),
    Builtin,
    Dynamic,
}

impl PendingContinuation {
    fn total_value_stack_count(&self) -> usize {
        match self {
            Self::Host {
                parameter_slots, ..
            } => *parameter_slots,
            Self::Child(child) => child.total_value_stack_count(),
            Self::Native {
                state,
                parameter_slots,
            } => parameter_slots
                .saturating_add(state.child.continuation.total_value_stack_count())
                .saturating_add(state.continuation.value_stack_slots()),
        }
    }

    fn detach_value_stack(&mut self) {
        match self {
            Self::Child(child) => child.detach_value_stack(),
            Self::Native { state, .. } => state.child.continuation.detach_value_stack(),
            Self::Host { .. } => {}
        }
    }

    fn clear_object_reference(&mut self, object_id: u64) {
        match self {
            Self::Host { value, .. } => {
                value.clear_object_reference(object_id);
            }
            Self::Child(child) => {
                child.clear_object_reference(object_id);
            }
            Self::Native { state, .. } => {
                state.child.clear_object_references(object_id);
                state.continuation.clear_object_references(object_id);
            }
        }
    }
}

impl ScriptContinuation {
    fn clear_object_reference(&mut self, object_id: u64) {
        self.this_value.clear_object_reference(object_id);
        self.frame.clear_object_reference(object_id);
    }
}

impl ContinuationFrame {
    fn total_value_stack_count(&self) -> usize {
        match self {
            Self::Compiled(frame) => {
                let stack = if frame.stack_value_stack.is_attached() {
                    0
                } else {
                    frame.stack_value_stack.count()
                };
                frame.frame_value_stack
                    + stack
                    + frame
                        .iterators
                        .iter()
                        .map(CompiledIterator::detached_value_stack_count)
                        .sum::<usize>()
                    + frame.pending.total_value_stack_count()
            }
        }
    }

    fn detach_value_stack(&mut self) {
        match self {
            Self::Compiled(frame) => frame.detach_value_stack(),
        }
    }

    fn clear_object_reference(&mut self, object_id: u64) {
        match self {
            Self::Compiled(frame) => frame.clear_object_reference(object_id),
        }
    }
}

impl CompiledContinuationFrame {
    fn detach_value_stack(&mut self) {
        self.stack_value_stack.detach();
        for iterator in &mut self.iterators {
            iterator.value_stack.detach();
        }
        self.pending.detach_value_stack();
    }

    fn clear_object_reference(&mut self, object_id: u64) {
        self.env.clear_object_reference(object_id);
        if let Some(caller) = &self.caller {
            caller.clear_object_reference(object_id);
        }
        for value in &mut self.stack {
            value.clear_object_reference(object_id);
        }
        for (_, reference) in &mut self.assignment_targets {
            reference.clear_object_reference(object_id);
        }
        for iterator in &mut self.iterators {
            iterator.clear_object_reference(object_id);
        }
        self.pending.clear_object_reference(object_id);
    }
}

impl ScriptContinuation {
    fn total_value_stack_count(&self) -> usize {
        self.frame.total_value_stack_count()
    }

    fn detach_value_stack(&mut self) {
        self.frame.detach_value_stack();
    }

    fn attach_value_stack_context(&self) -> Result<ScriptValueStackContext, RuntimeError> {
        let total = self.total_value_stack_count();
        ValueStackReservation::check(total)?;
        Ok(ScriptValueStackContext {
            _reservation: ValueStackReservation::reserve(total)?,
        })
    }

    // A suspended child can yield again before its parent reaches the instruction
    // loop. Normalize that owned control transfer at every frame boundary so
    // the caller retains its own suffix instead of unwinding past it.
    fn normalize_resume_result(
        vm: &Vm<'_>,
        result: Result<ContinuationResult, RuntimeError>,
    ) -> Result<ContinuationResult, RuntimeError> {
        result.or_else(|error| match vm.script_call_outcome_from_error(error)? {
            ScriptCallOutcome::Suspended(suspension) => {
                Ok(ContinuationResult::Suspended(suspension))
            }
            ScriptCallOutcome::Complete(value) => Ok(ContinuationResult::Complete(
                ReturnValue::Value(TrackedValue::runtime(value)),
            )),
        })
    }

    fn context_vm<'a>(&self, vm: &Vm<'a>) -> Vm<'a> {
        let env = match &self.frame {
            ContinuationFrame::Compiled(frame) => &frame.env,
        };
        let vm = if env.global_call_context {
            vm.engine_global_vm()
        } else {
            vm.clone()
        };
        vm.with_definition_context(env.definition_context)
    }

    fn resume(self, vm: &Vm<'_>) -> Result<ContinuationResult, RuntimeError> {
        let vm = self.context_vm(vm).with_this(self.this_value.clone());
        let _context = GlobalCallContextGuard::enter(
            vm.retain_global_call_context_for_host_paths
                .then_some(vm.global_call_context_hook)
                .flatten(),
        );
        Self::normalize_resume_result(
            &vm,
            match self.frame {
                ContinuationFrame::Compiled(frame) => frame.resume(&vm),
            },
        )
    }

    fn resume_with_value(
        self,
        vm: &Vm<'_>,
        value: Value,
    ) -> Result<ContinuationResult, RuntimeError> {
        let vm = self.context_vm(vm).with_this(self.this_value.clone());
        let _context = GlobalCallContextGuard::enter(
            vm.retain_global_call_context_for_host_paths
                .then_some(vm.global_call_context_hook)
                .flatten(),
        );
        Self::normalize_resume_result(
            &vm,
            match self.frame {
                ContinuationFrame::Compiled(frame) => frame.resume_with_value(&vm, Some(value)),
            },
        )
    }

    fn resume_with_value_and_this(
        self,
        vm: &Vm<'_>,
        value: Value,
        this_value: Value,
    ) -> Result<ContinuationResult, RuntimeError> {
        let vm = self.context_vm(vm).with_this(this_value);
        let _context = GlobalCallContextGuard::enter(
            vm.retain_global_call_context_for_host_paths
                .then_some(vm.global_call_context_hook)
                .flatten(),
        );
        Self::normalize_resume_result(
            &vm,
            match self.frame {
                ContinuationFrame::Compiled(frame) => frame.resume_with_value(&vm, Some(value)),
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CompiledSlotKind {
    Bare,
    FunctionVar,
}

#[derive(Debug, Clone, PartialEq)]
struct CompiledSlot {
    name: String,
    kind: CompiledSlotKind,
}

#[derive(Debug, Clone, PartialEq)]
enum CompiledPathSegment {
    Property(String),
    EmbeddedIndex(String),
    LiteralIndex(Literal),
}

#[derive(Debug, Clone, PartialEq)]
enum CompiledInstruction {
    Error(String),
    This,
    Literal(Literal),
    Load(usize),
    LoadReference(usize),
    LoadNamedReference(String),
    SlotReference {
        local: bool,
    },
    IndexReference {
        embedded: Option<String>,
        create: bool,
    },
    AppendReference,
    Materialize,
    Dereference,
    LegacyParameters {
        count: usize,
        forward_rest: bool,
    },
    PropertyReference {
        property: String,
        assignment: bool,
        create: bool,
    },
    StoreReference {
        copy_result: bool,
    },
    InvalidAssignment {
        operator: &'static str,
    },
    LoadArgument {
        slot: usize,
        site: usize,
        index: usize,
    },
    JumpIfValueArgument {
        site: usize,
        index: usize,
        target: usize,
    },
    MaterializeArgument {
        site: usize,
        index: usize,
    },
    LoadName(String),
    LoadPath {
        slot: usize,
        segments: Vec<CompiledPathSegment>,
    },
    BeginAssignment(usize),
    Store(usize),
    StoreAssignment(usize),
    StoreKeep {
        slot: usize,
        copy_result: bool,
    },
    Unary(UnaryOp),
    Binary(BinaryOp),
    CompoundStore {
        slot: usize,
        operation: BinaryOp,
        operator: &'static str,
    },
    CompoundReference {
        operation: BinaryOp,
        operator: &'static str,
        copy_result: bool,
    },
    IncrementReference {
        delta: i32,
        return_old: bool,
        copy_result: bool,
    },
    IncrementSlot {
        slot: usize,
        delta: i32,
    },
    Call {
        site: usize,
    },
    MakeArray(usize),
    MakeProplist(usize),
    Pop,
    JumpAnd(usize),
    JumpNotNil(usize),
    JumpIfNotNil {
        target: usize,
        materialize: bool,
    },
    JumpIfNil(usize),
    JumpIfGotoBound(usize),
    JumpOr(usize),
    JumpIfFalse(usize),
    Jump(usize),
    Return,
    IteratorInit {
        map: bool,
    },
    IteratorNext {
        slot: usize,
        value_slot: Option<usize>,
        end: usize,
    },
    IteratorEnd,
    Finish,
}

/// The slot-resolved bytecode for a complete C4Script function. Syntax trees
/// are retained for parsing and cache validation, never for execution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompiledFunction {
    slots: Vec<CompiledSlot>,
    function_vars: Vec<String>,
    instructions: Vec<CompiledInstruction>,
    call_sites: Vec<CompiledCallSite>,
    legacy_pin_instructions: Vec<bool>,
    max_stack: usize,
    diagnostic_name: Arc<str>,
    diagnostic_source_name: Option<Arc<str>>,
}

struct CompiledIterator {
    iterable: Value,
    items: Vec<(Value, Option<Value>)>,
    index: usize,
    sweep_cursor: usize,
    value_stack: ValueStackReservation,
}

impl CompiledIterator {
    fn clear_object_reference(&mut self, object_id: u64) {
        self.iterable.clear_object_reference(object_id);
        for (item, value) in &mut self.items {
            item.clear_object_reference(object_id);
            if let Some(value) = value {
                value.clear_object_reference(object_id);
            }
        }
    }

    fn apply_removals(&mut self) {
        let current = object_reference_sweep_cursor();
        if self.sweep_cursor == current {
            return;
        }
        clear_value_for_object_reference_sweeps(&mut self.iterable, self.sweep_cursor);
        for (item, value) in &mut self.items {
            clear_value_for_object_reference_sweeps(item, self.sweep_cursor);
            if let Some(value) = value {
                clear_value_for_object_reference_sweeps(value, self.sweep_cursor);
            }
        }
        self.sweep_cursor = current;
    }

    fn detached_value_stack_count(&self) -> usize {
        if self.value_stack.is_attached() {
            0
        } else {
            self.value_stack.count()
        }
    }
}

struct CompiledExecutionState {
    instruction: usize,
    stack: SmallVec<[ReturnValue; 16]>,
    registered_slots: SmallVec<[bool; 16]>,
    assignment_targets: SmallVec<[(usize, LValueRef); 4]>,
    iterators: SmallVec<[CompiledIterator; 2]>,
    stack_value_stack: ValueStackReservation,
    pending: Option<PendingContinuation>,
    /// The embedding host may replace the value suggested by the native
    /// callback when it commits a suspended request.  Keep that replacement
    /// separate from `pending`: a nested child has to receive the same value
    /// before its caller resumes.
    resume_value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
struct CompiledCallSite {
    name: String,
    argument_count: usize,
    forward_rest: bool,
    return_reference: bool,
    kind: CompiledCallKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CompiledCallKind {
    Direct,
    Method { failsafe: bool, reference: bool },
    Global { failsafe: bool },
}

pub(crate) struct CompiledFunctionCache {
    params: Vec<Parameter>,
    body: Arc<Vec<Stmt>>,
    strict_level: Option<u8>,
    returns_reference: bool,
    compiled: Option<Arc<CompiledFunction>>,
}

impl CompiledFunctionCache {
    fn new(function: &Function) -> Self {
        let compiled = CompiledFunction::compile(function).map(Arc::new);
        Self {
            params: function.params.clone(),
            body: Arc::new(function.body.clone()),
            strict_level: function.strict_level,
            returns_reference: function.returns_reference,
            compiled,
        }
    }

    fn validated(&self, function: &Function, validate_source: bool) -> Option<&Self> {
        #[cfg(test)]
        if validate_source {
            COMPILED_SOURCE_VALIDATIONS.with(|count| count.set(count.get() + 1));
        }
        (!validate_source
            || self.params == function.params
                && *self.body == function.body
                && self.strict_level == function.strict_level
                && self.returns_reference == function.returns_reference)
            .then_some(self)
    }
}

struct CompiledLoopContext {
    /// Value-stack depth C4Aul records in `Loop::StackSize` when the loop is
    /// pushed. A control statement at a different depth would need C4Aul's
    /// AB_STACK unwind, which this builder does not emit.
    stack_depth: usize,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

struct CompiledFunctionBuilder {
    slots: Vec<CompiledSlot>,
    bare_slots: FxHashMap<String, usize>,
    function_var_slots: FxHashMap<String, usize>,
    function_vars: Vec<String>,
    instructions: Vec<CompiledInstruction>,
    call_sites: Vec<CompiledCallSite>,
    loops: Vec<CompiledLoopContext>,
    legacy_pin_ranges: Vec<std::ops::Range<usize>>,
    stack_depth: usize,
    max_stack: usize,
    strict_level: Option<u8>,
    returns_reference: bool,
}

impl CompiledFunctionBuilder {
    fn new(function: &Function) -> Option<Self> {
        let mut builder = Self {
            slots: Vec::new(),
            bare_slots: FxHashMap::default(),
            function_var_slots: FxHashMap::default(),
            function_vars: Vec::new(),
            instructions: Vec::new(),
            call_sites: Vec::new(),
            loops: Vec::new(),
            legacy_pin_ranges: Vec::new(),
            stack_depth: 0,
            max_stack: 0,
            strict_level: function.strict_level,
            returns_reference: function.returns_reference,
        };

        for parameter in &function.params {
            let slot = builder.slots.len();
            builder.slots.push(CompiledSlot {
                name: parameter.name.clone(),
                kind: CompiledSlotKind::Bare,
            });
            // C4Aul's named parameter table keeps the last duplicate.
            builder.bare_slots.insert(parameter.name.clone(), slot);
        }

        let mut function_vars = Vec::new();
        collect_function_var_names(&function.body, &mut function_vars);
        for name in function_vars {
            if builder.function_var_slots.contains_key(&name) {
                continue;
            }
            let slot = builder.slots.len();
            builder.slots.push(CompiledSlot {
                name: name.clone(),
                kind: CompiledSlotKind::FunctionVar,
            });
            builder.function_var_slots.insert(name.clone(), slot);
            builder.bare_slots.entry(name.clone()).or_insert(slot);
            builder.function_vars.push(name);
        }

        Some(builder)
    }

    fn bare_slot(&mut self, name: &str) -> usize {
        if let Some(slot) = self.bare_slots.get(name) {
            return *slot;
        }
        let slot = self.slots.len();
        self.slots.push(CompiledSlot {
            name: name.to_string(),
            kind: CompiledSlotKind::Bare,
        });
        self.bare_slots.insert(name.to_string(), slot);
        slot
    }

    fn push_instruction(&mut self, instruction: CompiledInstruction) {
        self.instructions.push(instruction);
        self.stack_depth += 1;
        self.max_stack = self.max_stack.max(self.stack_depth);
    }

    /// C4Aul pushes the loop only once its condition has been consumed, so the
    /// recorded stack size is the depth every control statement must unwind to
    /// (C4AulParse.cpp:2492-2496,2593-2594).
    fn push_loop(&mut self) {
        self.loops.push(CompiledLoopContext {
            stack_depth: self.stack_depth,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
    }

    /// Emits the `break`/`continue` jump against the innermost loop, leaving
    /// its target for the enclosing loop form to patch.
    fn compile_loop_control(&mut self, is_break: bool) -> Option<()> {
        let context = self.loops.last()?;
        // C4Aul precedes the jump with an AB_STACK unwind when the control
        // statement sits deeper than the loop entry. Statement boundaries here
        // are always balanced, so a mismatch means an unmodelled construct.
        if self.stack_depth != context.stack_depth {
            return None;
        }
        let jump = self.instructions.len();
        self.instructions
            .push(CompiledInstruction::Jump(usize::MAX));
        let context = self.loops.last_mut()?;
        if is_break {
            context.breaks.push(jump);
        } else {
            context.continues.push(jump);
        }
        Some(())
    }

    /// Patches the innermost loop's controls, mirroring the fixup C4Aul runs
    /// before `PopLoop` (C4AulParse.cpp:2502-2508,2613-2619).
    fn pop_loop(&mut self, break_target: usize, continue_target: usize) -> Option<()> {
        let context = self.loops.pop()?;
        for jump in context.breaks {
            self.instructions[jump] = CompiledInstruction::Jump(break_target);
        }
        for jump in context.continues {
            self.instructions[jump] = CompiledInstruction::Jump(continue_target);
        }
        Some(())
    }

    fn pop_instruction(&mut self, instruction: CompiledInstruction) -> Option<()> {
        self.stack_depth = self.stack_depth.checked_sub(1)?;
        self.instructions.push(instruction);
        Some(())
    }

    fn binary_instruction(&mut self, operation: BinaryOp) -> Option<()> {
        self.stack_depth = self.stack_depth.checked_sub(1)?;
        self.instructions
            .push(CompiledInstruction::Binary(operation));
        Some(())
    }

    fn collection_instruction(
        &mut self,
        operand_count: usize,
        instruction: CompiledInstruction,
    ) -> Option<()> {
        self.stack_depth = self
            .stack_depth
            .checked_sub(operand_count)?
            .checked_add(1)?;
        self.instructions.push(instruction);
        self.max_stack = self.max_stack.max(self.stack_depth);
        Some(())
    }

    fn local_path(&self, expression: &Expr) -> Option<(usize, Vec<CompiledPathSegment>)> {
        fn collect(
            builder: &CompiledFunctionBuilder,
            expression: &Expr,
            segments: &mut Vec<CompiledPathSegment>,
        ) -> Option<usize> {
            match expression {
                Expr::Variable(name) => builder.bare_slots.get(name).copied(),
                Expr::Property(base, property) => {
                    let slot = collect(builder, base, segments)?;
                    segments.push(CompiledPathSegment::Property(property.clone()));
                    Some(slot)
                }
                Expr::Index(base, index) => {
                    let slot = collect(builder, base, segments)?;
                    segments.push(match index {
                        IndexOperand::EmbeddedString(value) => {
                            CompiledPathSegment::EmbeddedIndex(value.clone())
                        }
                        IndexOperand::Dynamic(index) => match index.as_ref() {
                            Expr::Literal(literal) => {
                                CompiledPathSegment::LiteralIndex(literal.clone())
                            }
                            _ => return None,
                        },
                    });
                    Some(slot)
                }
                _ => None,
            }
        }

        let mut segments = Vec::new();
        let slot = collect(self, expression, &mut segments)?;
        let mut saw_index = false;
        for segment in &segments {
            match segment {
                CompiledPathSegment::Property(_) if saw_index => return None,
                CompiledPathSegment::Property(_) => {}
                CompiledPathSegment::EmbeddedIndex(_) | CompiledPathSegment::LiteralIndex(_) => {
                    saw_index = true
                }
            }
        }
        (!segments.is_empty()).then_some((slot, segments))
    }

    fn compile_expression(&mut self, expression: &Expr) -> Option<()> {
        if let Some((slot, segments)) = self.local_path(expression) {
            let dynamic_index_slot = usize::from(
                segments
                    .iter()
                    .any(|segment| matches!(segment, CompiledPathSegment::LiteralIndex(_))),
            );
            self.max_stack = self
                .max_stack
                .max(self.stack_depth + 1 + dynamic_index_slot);
            self.push_instruction(CompiledInstruction::LoadPath { slot, segments });
            return Some(());
        }

        match expression {
            Expr::This => self.push_instruction(CompiledInstruction::This),
            Expr::SafeNavigation { receiver, steps } => {
                self.compile_set_no_ref_expression(receiver)?;
                self.instructions.push(CompiledInstruction::Dereference);
                let mut nil_jumps = Vec::new();
                for step in steps {
                    if step.nil_guard {
                        nil_jumps.push(self.instructions.len());
                        self.instructions
                            .push(CompiledInstruction::JumpIfNil(usize::MAX));
                    }
                    match &step.operation {
                        NavigationOperation::Index(index) => {
                            let (embedded, operands) = match index {
                                IndexOperand::EmbeddedString(key) => (Some(key.clone()), 1),
                                IndexOperand::Dynamic(index) => {
                                    self.compile_expression(index)?;
                                    (None, 2)
                                }
                            };
                            self.collection_instruction(
                                operands,
                                CompiledInstruction::IndexReference {
                                    embedded,
                                    create: false,
                                },
                            )?;
                            self.instructions.push(CompiledInstruction::Materialize);
                        }
                        NavigationOperation::Property(property) => {
                            self.instructions
                                .push(CompiledInstruction::PropertyReference {
                                    property: property.clone(),
                                    assignment: false,
                                    create: false,
                                });
                            self.instructions.push(CompiledInstruction::Materialize);
                        }
                        NavigationOperation::ArrayAppend => {
                            self.instructions.push(CompiledInstruction::AppendReference)
                        }
                        NavigationOperation::MethodCall {
                            name,
                            args,
                            is_optional,
                            forward_rest,
                        } => {
                            self.compile_call(
                                name,
                                CompiledCallKind::Method {
                                    failsafe: *is_optional,
                                    reference: false,
                                },
                                args,
                                *forward_rest,
                                1,
                            )?;
                        }
                    }
                }
                let end = self.instructions.len();
                for jump in nil_jumps {
                    self.instructions[jump] = CompiledInstruction::JumpIfNil(end);
                }
            }
            Expr::Literal(literal) => {
                self.push_instruction(CompiledInstruction::Literal(literal.clone()));
            }
            Expr::Variable(name) => match self.bare_slots.get(name).copied() {
                Some(slot) => self.push_instruction(CompiledInstruction::Load(slot)),
                None => self.push_instruction(CompiledInstruction::LoadName(name.clone())),
            },
            Expr::LegacyParameterList { args, forward_rest } => {
                let start = self.instructions.len();
                if args.len() == 1 {
                    self.compile_expression(&args[0])?;
                } else {
                    for argument in args {
                        self.compile_reference_expression(argument)?;
                    }
                    self.collection_instruction(
                        args.len(),
                        CompiledInstruction::LegacyParameters {
                            count: args.len(),
                            forward_rest: *forward_rest,
                        },
                    )?;
                }
                self.legacy_pin_ranges.push(start..self.instructions.len());
            }
            Expr::Unary(operation, value) => {
                self.compile_expression(value)?;
                self.instructions
                    .push(CompiledInstruction::Unary(operation.clone()));
            }
            Expr::PreIncrement(value)
            | Expr::PostIncrement(value)
            | Expr::PreDecrement(value)
            | Expr::PostDecrement(value) => {
                let delta = if matches!(expression, Expr::PreIncrement(_) | Expr::PostIncrement(_))
                {
                    1
                } else {
                    -1
                };
                self.compile_increment_expression(
                    value,
                    delta,
                    matches!(expression, Expr::PostIncrement(_) | Expr::PostDecrement(_)),
                    true,
                )?;
            }
            Expr::Binary(left, operation, right)
                if matches!(operation, BinaryOp::NilCoalescing)
                    || self.strict_level.unwrap_or(0) >= 2
                        && matches!(operation, BinaryOp::And | BinaryOp::Or) =>
            {
                self.compile_short_circuit(left, operation, right, false)?;
                self.instructions.push(CompiledInstruction::Dereference);
            }
            Expr::Binary(left, operation, right) => {
                self.compile_expression(left)?;
                self.compile_expression(right)?;
                self.binary_instruction(operation.clone())?;
            }
            Expr::Call {
                callee,
                args,
                is_optional,
                forward_rest,
            } => {
                let (name, kind, receiver_count) = match callee.as_ref() {
                    Expr::Variable(name) if !is_optional => (name, CompiledCallKind::Direct, 0),
                    Expr::Property(receiver, name) => {
                        self.compile_expression(receiver)?;
                        (
                            name,
                            CompiledCallKind::Method {
                                failsafe: *is_optional,
                                reference: false,
                            },
                            1,
                        )
                    }
                    _ => return None,
                };
                self.compile_call(name, kind, args, *forward_rest, receiver_count)?;
            }
            Expr::GlobalCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => {
                self.compile_expression(&Expr::Call {
                    callee: Box::new(Expr::Property(
                        Box::new(Expr::Literal(Literal::Nil)),
                        name.clone(),
                    )),
                    args: args.clone(),
                    is_optional: *failsafe,
                    forward_rest: *forward_rest,
                })?;
                let CompiledInstruction::Call { site } = self.instructions.last()? else {
                    return None;
                };
                self.call_sites[*site].kind = CompiledCallKind::Global {
                    failsafe: *failsafe,
                };
            }
            Expr::Array(elements) => {
                for element in elements {
                    self.compile_expression(element)?;
                }
                self.collection_instruction(
                    elements.len(),
                    CompiledInstruction::MakeArray(elements.len()),
                )?;
            }
            Expr::Proplist(entries) => {
                for (key, value) in entries {
                    self.compile_set_no_ref_expression(key)?;
                    self.compile_set_no_ref_expression(value)?;
                }
                self.collection_instruction(
                    entries.len().checked_mul(2)?,
                    CompiledInstruction::MakeProplist(entries.len()),
                )?;
            }
            Expr::ArrayAppend(_) => {
                self.compile_reference_expression(expression)?;
                self.instructions.push(CompiledInstruction::Materialize);
            }
            Expr::ArrayAppendAssignment {
                target,
                operation,
                operator,
                value,
            } => match operation {
                Some(operation) => {
                    self.compile_compound_assignment(target, operation, operator, value, false)?
                }
                None => self.compile_reference_assignment(target, value, false)?,
            },
            Expr::Index(..) | Expr::Property(..) => {
                self.compile_reference_expression_mode(expression, false)?;
                self.instructions.push(CompiledInstruction::Materialize);
            }
            Expr::Assignment(AssignmentTarget::Variable(name), value) => {
                self.compile_assignment_expression(name, value, false)?;
            }
            Expr::CompoundAssignment {
                target,
                operation,
                operator,
                value,
            } => {
                self.compile_compound_assignment(target, operation, operator, value, false)?;
            }
            Expr::Assignment(target, value) => {
                self.compile_reference_assignment(target, value, false)?;
            }
        }
        Some(())
    }

    fn compile_call(
        &mut self,
        name: &str,
        kind: CompiledCallKind,
        args: &[Expr],
        forward_rest: bool,
        receiver_count: usize,
    ) -> Option<()> {
        let site = self.call_sites.len();
        self.call_sites.push(CompiledCallSite {
            name: name.to_owned(),
            argument_count: args.len(),
            forward_rest,
            return_reference: false,
            kind,
        });
        for (index, argument) in args.iter().enumerate() {
            if let Expr::Variable(name) = argument {
                if let Some(slot) = self.bare_slots.get(name).copied() {
                    self.push_instruction(CompiledInstruction::LoadArgument { slot, site, index });
                } else {
                    self.compile_reference_expression(argument)?;
                    self.instructions
                        .push(CompiledInstruction::MaterializeArgument { site, index });
                }
            } else if matches!(argument, Expr::Call { .. } | Expr::GlobalCall { .. }) {
                self.compile_reference_expression(argument)?;
                self.instructions
                    .push(CompiledInstruction::MaterializeArgument { site, index });
            } else if self.reference_argument_differs(argument) {
                let start_depth = self.stack_depth;
                let value_jump = self.instructions.len();
                self.instructions
                    .push(CompiledInstruction::JumpIfValueArgument {
                        site,
                        index,
                        target: usize::MAX,
                    });
                self.compile_reference_expression(argument)?;
                let end_jump = self.instructions.len();
                self.instructions
                    .push(CompiledInstruction::Jump(usize::MAX));
                self.instructions[value_jump] = CompiledInstruction::JumpIfValueArgument {
                    site,
                    index,
                    target: self.instructions.len(),
                };
                self.stack_depth = start_depth;
                self.compile_expression(argument)?;
                self.instructions[end_jump] = CompiledInstruction::Jump(self.instructions.len());
            } else {
                self.compile_expression(argument)?;
            }
        }
        if receiver_count != 0 {
            self.max_stack = self
                .max_stack
                .max(self.stack_depth - args.len() + MAX_CALL_PARAMETERS);
        }
        self.collection_instruction(
            args.len() + receiver_count,
            CompiledInstruction::Call { site },
        )?;
        Some(())
    }

    /// Emit a runtime reference/value branch only when the lowerings differ.
    /// Duplicating arrays or arithmetic here would duplicate every nested call
    /// site, growing the plan exponentially with nested value arguments.
    fn reference_argument_differs(&self, expression: &Expr) -> bool {
        match expression {
            Expr::Variable(_)
            | Expr::Call { .. }
            | Expr::GlobalCall { .. }
            | Expr::Property(..)
            | Expr::Index(..)
            | Expr::Assignment(..)
            | Expr::CompoundAssignment { .. }
            | Expr::PreIncrement(_)
            | Expr::PreDecrement(_)
            | Expr::ArrayAppend(_)
            | Expr::ArrayAppendAssignment { .. } => true,
            Expr::Binary(_, operation, _) => {
                matches!(operation, BinaryOp::NilCoalescing)
                    || self.strict_level.unwrap_or(0) >= 2
                        && matches!(operation, BinaryOp::And | BinaryOp::Or)
            }
            Expr::This
            | Expr::Literal(_)
            | Expr::Array(_)
            | Expr::Proplist(_)
            | Expr::Unary(..)
            | Expr::PostIncrement(_)
            | Expr::PostDecrement(_)
            | Expr::SafeNavigation { .. }
            | Expr::LegacyParameterList { .. } => false,
        }
    }

    fn compile_reference_expression(&mut self, expression: &Expr) -> Option<()> {
        self.compile_reference_expression_mode(expression, true)
    }

    fn compile_reference_expression_mode(&mut self, expression: &Expr, create: bool) -> Option<()> {
        match expression {
            Expr::GlobalCall { .. } => {
                self.compile_expression(expression)?;
                let CompiledInstruction::Call { site } = self.instructions.last()? else {
                    return None;
                };
                self.call_sites[*site].return_reference = true;
            }
            Expr::Binary(left, operation, right)
                if matches!(operation, BinaryOp::NilCoalescing)
                    || self.strict_level.unwrap_or(0) >= 2
                        && matches!(operation, BinaryOp::And | BinaryOp::Or) =>
            {
                self.compile_short_circuit(left, operation, right, true)?;
            }
            Expr::Variable(name) => {
                if let Some(slot) = self.bare_slots.get(name).copied() {
                    self.push_instruction(CompiledInstruction::LoadReference(slot));
                } else {
                    self.push_instruction(CompiledInstruction::LoadNamedReference(name.clone()));
                }
            }
            Expr::Property(base, property) => {
                self.compile_reference_expression_mode(base, create)?;
                self.instructions
                    .push(CompiledInstruction::PropertyReference {
                        property: property.clone(),
                        assignment: false,
                        create,
                    });
            }
            Expr::Index(base, index) => {
                self.compile_reference_expression_mode(base, create)?;
                let (embedded, operands) = match index {
                    IndexOperand::EmbeddedString(key) => (Some(key.clone()), 1),
                    IndexOperand::Dynamic(index) => {
                        self.compile_expression(index)?;
                        (None, 2)
                    }
                };
                self.collection_instruction(
                    operands,
                    CompiledInstruction::IndexReference { embedded, create },
                )?;
            }
            Expr::Call { .. } => {
                self.compile_expression(expression)?;
                let CompiledInstruction::Call { site } = self.instructions.last()? else {
                    return None;
                };
                self.call_sites[*site].return_reference = true;
            }
            Expr::ArrayAppend(base) => {
                self.compile_reference_expression(base)?;
                self.instructions.push(CompiledInstruction::AppendReference);
            }
            Expr::ArrayAppendAssignment {
                target,
                operation,
                operator,
                value,
            } => match operation {
                Some(operation) => {
                    self.compile_compound_assignment(target, operation, operator, value, true)?
                }
                None => self.compile_reference_assignment(target, value, true)?,
            },
            Expr::PreIncrement(value) | Expr::PreDecrement(value) => {
                let delta = if matches!(expression, Expr::PreIncrement(_)) {
                    1
                } else {
                    -1
                };
                self.compile_increment_expression(value, delta, false, false)?;
            }
            Expr::PostIncrement(_) | Expr::PostDecrement(_) => {
                self.compile_expression(expression)?
            }
            Expr::CompoundAssignment {
                target,
                operation,
                operator,
                value,
            } => {
                self.compile_compound_assignment(target, operation, operator, value, true)?;
            }
            Expr::Assignment(target, value) => {
                self.compile_reference_assignment(target, value, true)?;
            }
            Expr::SafeNavigation { .. }
            | Expr::LegacyParameterList { .. }
            | Expr::This
            | Expr::Literal(_)
            | Expr::Array(_)
            | Expr::Proplist(_)
            | Expr::Unary(..)
            | Expr::Binary(..) => self.compile_expression(expression)?,
        }
        Some(())
    }

    fn compile_set_no_ref_expression(&mut self, expression: &Expr) -> Option<()> {
        match expression {
            Expr::ArrayAppend(_)
            | Expr::ArrayAppendAssignment { .. }
            | Expr::PreIncrement(_)
            | Expr::PreDecrement(_) => self.compile_reference_expression(expression),
            Expr::Call { .. } => self.compile_reference_expression(expression),
            Expr::Assignment(AssignmentTarget::Variable(name), value) => {
                self.compile_assignment_expression(name, value, true)
            }
            Expr::CompoundAssignment {
                target,
                operation,
                operator,
                value,
            } => self.compile_compound_assignment(target, operation, operator, value, true),
            Expr::Assignment(target, value) => {
                self.compile_reference_assignment(target, value, true)
            }
            Expr::Binary(left, operation, right)
                if matches!(operation, BinaryOp::NilCoalescing)
                    || self.strict_level.unwrap_or(0) >= 2
                        && matches!(operation, BinaryOp::And | BinaryOp::Or) =>
            {
                self.compile_short_circuit(left, operation, right, false)
            }
            _ => self.compile_expression(expression),
        }
    }

    fn compile_short_circuit(
        &mut self,
        left: &Expr,
        operation: &BinaryOp,
        right: &Expr,
        preserve_rhs_reference: bool,
    ) -> Option<()> {
        self.compile_set_no_ref_expression(left)?;
        let jump = self.instructions.len();
        let instruction = |target| match operation {
            BinaryOp::And => CompiledInstruction::JumpAnd(target),
            BinaryOp::Or => CompiledInstruction::JumpOr(target),
            BinaryOp::NilCoalescing => CompiledInstruction::JumpNotNil(target),
            _ => unreachable!("only short circuit operators have conditional operands"),
        };
        self.instructions.push(instruction(usize::MAX));
        self.stack_depth = self.stack_depth.checked_sub(1)?;
        if preserve_rhs_reference {
            self.compile_reference_expression(right)?;
        } else {
            self.compile_set_no_ref_expression(right)?;
        }
        self.instructions[jump] = instruction(self.instructions.len());
        Some(())
    }

    fn compile_assignment_expression(
        &mut self,
        name: &str,
        value: &Expr,
        preserve_result_reference: bool,
    ) -> Option<()> {
        let slot = self.bare_slot(name);
        self.instructions
            .push(CompiledInstruction::BeginAssignment(slot));
        self.stack_depth += 1;
        self.max_stack = self.max_stack.max(self.stack_depth);
        self.compile_set_no_ref_expression(value)?;
        self.stack_depth = self.stack_depth.checked_sub(1)?;
        self.instructions.push(CompiledInstruction::StoreKeep {
            slot,
            copy_result: !preserve_result_reference,
        });
        Some(())
    }

    fn compile_compound_assignment(
        &mut self,
        target: &AssignmentTarget,
        operation: &BinaryOp,
        operator: &'static str,
        value: &Expr,
        preserve_reference: bool,
    ) -> Option<()> {
        self.compile_assignment_target(target)?;
        let nil_jump = if matches!(operation, BinaryOp::NilCoalescing) {
            let jump = self.instructions.len();
            self.instructions.push(CompiledInstruction::JumpIfNotNil {
                target: usize::MAX,
                materialize: !preserve_reference,
            });
            Some(jump)
        } else {
            None
        };
        self.compile_expression(value)?;
        self.collection_instruction(
            2,
            CompiledInstruction::CompoundReference {
                operation: operation.clone(),
                operator,
                copy_result: !preserve_reference,
            },
        )?;
        if let Some(jump) = nil_jump {
            self.instructions[jump] = CompiledInstruction::JumpIfNotNil {
                target: self.instructions.len(),
                materialize: !preserve_reference,
            };
        }
        Some(())
    }

    fn compile_increment_expression(
        &mut self,
        value: &Expr,
        delta: i32,
        return_old: bool,
        copy_result: bool,
    ) -> Option<()> {
        self.compile_reference_expression(value)?;
        self.instructions
            .push(CompiledInstruction::IncrementReference {
                delta,
                return_old,
                copy_result,
            });
        Some(())
    }

    fn compile_assignment_target(&mut self, target: &AssignmentTarget) -> Option<()> {
        match target {
            AssignmentTarget::InvalidValue { expression, .. } => {
                self.compile_expression(expression)?
            }
            AssignmentTarget::PrefixChange { target, delta } => {
                self.compile_assignment_target(target)?;
                self.instructions
                    .push(CompiledInstruction::IncrementReference {
                        delta: *delta,
                        return_old: false,
                        copy_result: false,
                    });
            }
            AssignmentTarget::EffectSlot(args) => {
                self.compile_call("EffectVar", CompiledCallKind::Direct, args, false, 0)?;
                let CompiledInstruction::Call { site } = self.instructions.last()? else {
                    return None;
                };
                self.call_sites[*site].return_reference = true;
            }
            AssignmentTarget::GlobalFunctionCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => {
                self.compile_reference_expression(&Expr::GlobalCall {
                    name: name.clone(),
                    args: args.clone(),
                    failsafe: *failsafe,
                    forward_rest: *forward_rest,
                })?;
            }
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
                is_arrow,
            } => {
                let mut args = args.clone();
                let callee = if *is_arrow {
                    Expr::Property(object.clone(), method.clone())
                } else {
                    args.push(*object.clone());
                    Expr::Variable(method.clone())
                };
                self.compile_expression(&Expr::Call {
                    callee: Box::new(callee),
                    args,
                    is_optional: false,
                    forward_rest: false,
                })?;
                let CompiledInstruction::Call { site } = self.instructions.last()? else {
                    return None;
                };
                self.call_sites[*site].return_reference = true;
                if *is_arrow {
                    self.call_sites[*site].kind = CompiledCallKind::Method {
                        failsafe: false,
                        reference: true,
                    };
                }
            }
            AssignmentTarget::LocalSlot(index) | AssignmentTarget::VarSlot(index) => {
                self.compile_expression(index)?;
                self.instructions.push(CompiledInstruction::SlotReference {
                    local: matches!(target, AssignmentTarget::LocalSlot(_)),
                });
            }
            AssignmentTarget::Variable(name) => {
                let slot = self.bare_slot(name);
                self.push_instruction(CompiledInstruction::LoadReference(slot));
            }
            AssignmentTarget::Property(base, property) => {
                self.compile_assignment_target(base)?;
                self.instructions
                    .push(CompiledInstruction::PropertyReference {
                        property: property.clone(),
                        assignment: true,
                        create: true,
                    });
            }
            AssignmentTarget::ArrayAppend(base) => {
                self.compile_reference_expression(base)?;
                self.instructions.push(CompiledInstruction::AppendReference);
            }
            AssignmentTarget::Index(base, index) => {
                self.compile_assignment_target(base)?;
                let (embedded, operands) = match index {
                    IndexOperand::EmbeddedString(key) => (Some(key.clone()), 1),
                    IndexOperand::Dynamic(index) => {
                        self.compile_expression(index)?;
                        (None, 2)
                    }
                };
                self.collection_instruction(
                    operands,
                    CompiledInstruction::IndexReference {
                        embedded,
                        create: true,
                    },
                )?;
            }
            AssignmentTarget::FunctionCall { name, args } => {
                self.compile_reference_expression(&Expr::Call {
                    callee: Box::new(Expr::Variable(name.clone())),
                    args: args.clone(),
                    is_optional: false,
                    forward_rest: false,
                })?;
            }
        }
        Some(())
    }

    fn compile_reference_assignment(
        &mut self,
        target: &AssignmentTarget,
        value: &Expr,
        preserve_reference: bool,
    ) -> Option<()> {
        if let AssignmentTarget::InvalidValue {
            expression,
            operator,
        } = target
        {
            self.compile_expression(expression)?;
            let jump = if *operator == "??=" {
                let jump = self.instructions.len();
                self.instructions.push(CompiledInstruction::JumpIfNotNil {
                    target: usize::MAX,
                    materialize: false,
                });
                Some(jump)
            } else {
                None
            };
            self.compile_expression(value)?;
            self.collection_instruction(2, CompiledInstruction::InvalidAssignment { operator })?;
            if let Some(jump) = jump {
                self.instructions[jump] = CompiledInstruction::JumpIfNotNil {
                    target: self.instructions.len(),
                    materialize: false,
                };
            }
            return Some(());
        }
        self.compile_assignment_target(target)?;
        self.compile_set_no_ref_expression(value)?;
        self.collection_instruction(
            2,
            CompiledInstruction::StoreReference {
                copy_result: !preserve_reference,
            },
        )
    }

    fn compile_discarded_expression(&mut self, expression: &Expr) -> Option<()> {
        match expression {
            Expr::Assignment(AssignmentTarget::Variable(name), value) => {
                let slot = self.bare_slot(name);
                self.instructions
                    .push(CompiledInstruction::BeginAssignment(slot));
                self.stack_depth += 1;
                self.max_stack = self.max_stack.max(self.stack_depth);
                self.compile_set_no_ref_expression(value)?;
                self.stack_depth = self.stack_depth.checked_sub(2)?;
                self.instructions
                    .push(CompiledInstruction::StoreAssignment(slot));
            }
            Expr::CompoundAssignment {
                target: AssignmentTarget::Variable(name),
                operation,
                operator,
                value,
            } if !matches!(operation, BinaryOp::Concat | BinaryOp::NilCoalescing) => {
                let slot = self.bare_slot(name);
                self.instructions
                    .push(CompiledInstruction::BeginAssignment(slot));
                self.stack_depth += 1;
                self.max_stack = self.max_stack.max(self.stack_depth);
                self.compile_expression(value)?;
                self.stack_depth = self.stack_depth.checked_sub(2)?;
                self.instructions.push(CompiledInstruction::CompoundStore {
                    slot,
                    operation: operation.clone(),
                    operator,
                });
            }
            Expr::PreIncrement(value)
            | Expr::PostIncrement(value)
            | Expr::PreDecrement(value)
            | Expr::PostDecrement(value) => {
                let Expr::Variable(name) = value.as_ref() else {
                    self.compile_expression(expression)?;
                    self.pop_instruction(CompiledInstruction::Pop)?;
                    return Some(());
                };
                let delta = if matches!(expression, Expr::PreIncrement(_) | Expr::PostIncrement(_))
                {
                    1
                } else {
                    -1
                };
                let slot = self.bare_slot(name);
                self.instructions
                    .push(CompiledInstruction::IncrementSlot { slot, delta });
            }
            _ => {
                self.compile_expression(expression)?;
                self.pop_instruction(CompiledInstruction::Pop)?;
            }
        }
        Some(())
    }

    fn compile_statements(&mut self, statements: &[Stmt]) -> Option<()> {
        for statement in statements {
            match statement {
                Stmt::LegacyGoto { call, expression } => {
                    if self.strict_level.is_none() {
                        let bound_jump = self.instructions.len();
                        self.instructions
                            .push(CompiledInstruction::JumpIfGotoBound(usize::MAX));
                        if self.returns_reference {
                            self.compile_reference_expression(call)?;
                        } else {
                            self.compile_expression(call)?;
                        }
                        self.pop_instruction(CompiledInstruction::Return)?;
                        self.instructions[bound_jump] =
                            CompiledInstruction::JumpIfGotoBound(self.instructions.len());
                    }
                    self.compile_discarded_expression(expression)?;
                }
                Stmt::ParseError {
                    message,
                    line,
                    column,
                } => {
                    self.instructions.push(CompiledInstruction::Error(format!(
                        "parse error at {line}:{column}: {message}"
                    )));
                }
                Stmt::VarDecl { name, init } => {
                    let Some(initializer) = init else {
                        continue;
                    };
                    self.compile_expression(initializer)?;
                    let slot = *self.function_var_slots.get(name)?;
                    self.pop_instruction(CompiledInstruction::Store(slot))?;
                }
                Stmt::Assignment {
                    target: AssignmentTarget::Variable(name),
                    value,
                } => {
                    let slot = self.bare_slot(name);
                    self.instructions
                        .push(CompiledInstruction::BeginAssignment(slot));
                    self.stack_depth += 1;
                    self.max_stack = self.max_stack.max(self.stack_depth);
                    self.compile_set_no_ref_expression(value)?;
                    self.stack_depth = self.stack_depth.checked_sub(2)?;
                    self.instructions
                        .push(CompiledInstruction::StoreAssignment(slot));
                }
                Stmt::Return(expression) => {
                    match expression {
                        Some(expression) if self.returns_reference => {
                            self.compile_reference_expression(expression)?
                        }
                        Some(expression) => self.compile_expression(expression)?,
                        None => self.push_instruction(CompiledInstruction::Literal(Literal::Nil)),
                    }
                    self.pop_instruction(CompiledInstruction::Return)?;
                }
                Stmt::Assignment { target, value } => {
                    self.compile_reference_assignment(target, value, true)?;
                    self.pop_instruction(CompiledInstruction::Pop)?;
                }
                Stmt::Expr(expression) => match expression {
                    Expr::CompoundAssignment {
                        target: AssignmentTarget::Variable(name),
                        operation,
                        operator,
                        value,
                    } if !matches!(operation, BinaryOp::Concat | BinaryOp::NilCoalescing) => {
                        let slot = self.bare_slot(name);
                        self.instructions
                            .push(CompiledInstruction::BeginAssignment(slot));
                        self.stack_depth += 1;
                        self.max_stack = self.max_stack.max(self.stack_depth);
                        self.compile_expression(value)?;
                        self.stack_depth = self.stack_depth.checked_sub(2)?;
                        self.instructions.push(CompiledInstruction::CompoundStore {
                            slot,
                            operation: operation.clone(),
                            operator,
                        });
                    }
                    Expr::PreIncrement(value)
                    | Expr::PostIncrement(value)
                    | Expr::PreDecrement(value)
                    | Expr::PostDecrement(value) => {
                        let Expr::Variable(name) = value.as_ref() else {
                            self.compile_expression(expression)?;
                            self.pop_instruction(CompiledInstruction::Pop)?;
                            continue;
                        };
                        let delta =
                            if matches!(expression, Expr::PreIncrement(_) | Expr::PostIncrement(_))
                            {
                                1
                            } else {
                                -1
                            };
                        let slot = self.bare_slot(name);
                        self.instructions
                            .push(CompiledInstruction::IncrementSlot { slot, delta });
                    }
                    _ => {
                        self.compile_expression(expression)?;
                        self.pop_instruction(CompiledInstruction::Pop)?;
                    }
                },
                Stmt::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    self.compile_expression(condition)?;
                    let false_jump = self.instructions.len();
                    self.pop_instruction(CompiledInstruction::JumpIfFalse(usize::MAX))?;
                    self.compile_statements(then_branch)?;
                    if let Some(else_branch) = else_branch {
                        let end_jump = self.instructions.len();
                        self.instructions
                            .push(CompiledInstruction::Jump(usize::MAX));
                        let else_start = self.instructions.len();
                        self.instructions[false_jump] =
                            CompiledInstruction::JumpIfFalse(else_start);
                        self.compile_statements(else_branch)?;
                        let end = self.instructions.len();
                        self.instructions[end_jump] = CompiledInstruction::Jump(end);
                    } else {
                        let end = self.instructions.len();
                        self.instructions[false_jump] = CompiledInstruction::JumpIfFalse(end);
                    }
                }
                Stmt::While { condition, body } => {
                    let start = self.instructions.len();
                    self.compile_expression(condition)?;
                    let end_jump = self.instructions.len();
                    self.pop_instruction(CompiledInstruction::JumpIfFalse(usize::MAX))?;
                    self.push_loop();
                    self.compile_statements(body)?;
                    self.instructions.push(CompiledInstruction::Jump(start));
                    let end = self.instructions.len();
                    self.instructions[end_jump] = CompiledInstruction::JumpIfFalse(end);
                    // `continue` re-tests the condition; `break` leaves the loop.
                    self.pop_loop(end, start)?;
                }
                Stmt::For {
                    init,
                    condition,
                    increment,
                    body,
                } => {
                    if let Some(init) = init {
                        match init {
                            ForInit::VarDecls(declarations) => {
                                for (name, value) in declarations {
                                    let Some(value) = value else {
                                        continue;
                                    };
                                    self.compile_expression(value)?;
                                    let slot = *self.function_var_slots.get(name)?;
                                    self.pop_instruction(CompiledInstruction::Store(slot))?;
                                }
                            }
                            ForInit::Expr(expression) => {
                                self.compile_discarded_expression(expression)?;
                            }
                        }
                    }

                    let condition_start = self.instructions.len();
                    let end_jump = if let Some(condition) = condition {
                        self.compile_expression(condition)?;
                        let jump = self.instructions.len();
                        self.pop_instruction(CompiledInstruction::JumpIfFalse(usize::MAX))?;
                        Some(jump)
                    } else {
                        None
                    };
                    self.push_loop();
                    self.compile_statements(body)?;
                    let increment_start = self.instructions.len();
                    if let Some(increment) = increment {
                        self.compile_discarded_expression(increment)?;
                    }
                    self.instructions
                        .push(CompiledInstruction::Jump(condition_start));
                    let end = self.instructions.len();
                    if let Some(end_jump) = end_jump {
                        self.instructions[end_jump] = CompiledInstruction::JumpIfFalse(end);
                    }
                    // C4Aul's back edge is the incrementor when there is one,
                    // otherwise the condition, otherwise the body; `continue`
                    // shares it (C4AulParse.cpp:2604-2619).
                    let back_edge = if increment.is_some() {
                        increment_start
                    } else {
                        condition_start
                    };
                    self.pop_loop(end, back_edge)?;
                }
                Stmt::ForIn {
                    variable,
                    value_variable,
                    iterable,
                    body,
                    ..
                } => {
                    self.compile_expression(iterable)?;
                    self.pop_instruction(CompiledInstruction::IteratorInit {
                        map: value_variable.is_some(),
                    })?;
                    let control_slots = if value_variable.is_some() { 3 } else { 2 };
                    self.stack_depth += control_slots;
                    self.max_stack = self.max_stack.max(self.stack_depth);
                    let next = self.instructions.len();
                    let slot = *self.function_var_slots.get(variable)?;
                    let value_slot = match value_variable {
                        Some(name) => Some(*self.function_var_slots.get(name)?),
                        None => None,
                    };
                    self.instructions.push(CompiledInstruction::IteratorNext {
                        slot,
                        value_slot,
                        end: usize::MAX,
                    });
                    self.push_loop();
                    self.compile_statements(body)?;
                    self.instructions.push(CompiledInstruction::Jump(next));
                    let end = self.instructions.len();
                    self.instructions[next] = CompiledInstruction::IteratorNext {
                        slot,
                        value_slot,
                        end,
                    };
                    self.pop_loop(end, next)?;
                    self.instructions.push(CompiledInstruction::IteratorEnd);
                    self.stack_depth = self.stack_depth.checked_sub(control_slots)?;
                }
                Stmt::Break => self.compile_loop_control(true)?,
                Stmt::Continue => self.compile_loop_control(false)?,
                Stmt::Block(statements) | Stmt::Sequence(statements) => {
                    self.compile_statements(statements)?;
                }
            }
        }
        Some(())
    }

    fn finish(mut self, function: &Function) -> Option<CompiledFunction> {
        self.compile_statements(&function.body)?;
        if self.stack_depth != 0 {
            return None;
        }
        self.instructions.push(CompiledInstruction::Finish);
        let mut legacy_pin_instructions = vec![false; self.instructions.len()];
        for range in self.legacy_pin_ranges {
            legacy_pin_instructions[range].fill(true);
        }
        Some(CompiledFunction {
            legacy_pin_instructions,
            slots: self.slots,
            function_vars: self.function_vars,
            instructions: self.instructions,
            call_sites: self.call_sites,
            max_stack: self.max_stack,
            diagnostic_name: Arc::from(function.name.as_str()),
            diagnostic_source_name: function.source_name().map(Arc::from),
        })
    }
}

fn compiled_object_hook_stack(
    value: &Value,
    hook_stack_slots: Option<usize>,
) -> Result<ValueStackReservation, RuntimeError> {
    match (value, hook_stack_slots) {
        (Value::Object(object), Some(slots)) if *object != 0 => {
            ValueStackReservation::reserve(slots)
        }
        _ => Ok(ValueStackReservation::empty()),
    }
}

fn read_compiled_path(
    vm: &Vm<'_>,
    env: &Environment,
    binding: &Binding,
    segments: &[CompiledPathSegment],
    register_root: bool,
) -> Result<TrackedValue, RuntimeError> {
    match binding {
        Binding::Direct { value, identity } => {
            let identity = legacy_identity_for_value_copy(value, &[], identity.borrow().clone());
            let value = value.borrow();
            if register_root {
                vm.register_runtime_value(&value);
            }
            read_compiled_path_value(vm, env, &value, identity, segments, env.strict_level)
        }
        Binding::Inline(inline) => {
            let tracked = inline.read_tracked();
            read_compiled_path_value(
                vm,
                env,
                &tracked.value,
                tracked.identity.clone(),
                segments,
                env.strict_level,
            )
        }
        Binding::Reference(reference) => {
            let root = reference.read_tracked()?.set_copy();
            if register_root {
                vm.register_runtime_value(&root.value);
            }
            read_compiled_path_value(
                vm,
                env,
                &root.value,
                root.identity,
                segments,
                env.strict_level,
            )
        }
    }
}

fn read_compiled_indexed_path(
    vm: &Vm<'_>,
    env: &Environment,
    binding: &Binding,
    segments: &[CompiledPathSegment],
    strict_level: Option<u8>,
) -> Result<TrackedValue, RuntimeError> {
    let _pin_creation = LegacyPathPinCreationGuard::suspend();
    let mut current = ReturnValue::Reference(binding.lvalue());
    for segment in segments {
        current = match segment {
            CompiledPathSegment::Property(property) => {
                vm.property_reference_or_value_with_hook_stack(current, property, env, None)?
            }
            CompiledPathSegment::EmbeddedIndex(value) => vm
                .index_value_reference_or_value_with_hook_stack(
                    current,
                    Value::String(vm.literal_string(value)),
                    env,
                    None,
                )?,
            CompiledPathSegment::LiteralIndex(literal) => {
                let _index_slot = ValueStackReservation::reserve(1)?;
                let index = TrackedValue::literal(vm.literal_value(literal, strict_level), literal)
                    .set_copy();
                vm.register_runtime_value(&index.value);
                vm.index_value_reference_or_value_with_hook_stack(current, index.value, env, None)?
            }
        };
    }
    let value = current.into_tracked()?.set_copy();
    vm.register_runtime_value(&value.value);
    Ok(value)
}

fn read_compiled_path_value(
    vm: &Vm<'_>,
    env: &Environment,
    current: &Value,
    identity: Option<RawIdentity>,
    segments: &[CompiledPathSegment],
    strict_level: Option<u8>,
) -> Result<TrackedValue, RuntimeError> {
    if c4_set_copy_is_zero_id(current) {
        return read_compiled_path_value(vm, env, &Value::Nil, None, segments, strict_level);
    }
    let Some((segment, remaining)) = segments.split_first() else {
        return Ok(TrackedValue {
            value: current.clone(),
            identity,
        });
    };

    match segment {
        CompiledPathSegment::Property(property) => {
            let path_segment = PathSegment::Property(property.clone());
            let child_identity = identity
                .as_ref()
                .and_then(|identity| identity.identity_at(&path_segment));
            match current {
                Value::Proplist(entries) => {
                    let nil = Value::Nil;
                    let child = entries.get(property).unwrap_or(&nil);
                    read_compiled_path_value(
                        vm,
                        env,
                        child,
                        child_identity,
                        remaining,
                        strict_level,
                    )
                }
                Value::Object(0) => Err(RuntimeError::new(
                    "map access with .: map expected, but got nil!",
                )),
                target @ Value::Object(_) => {
                    let _hook_stack = compiled_object_hook_stack(target, None)?;
                    let child = vm.object_local_tracked(env, target, property).set_copy();
                    vm.register_runtime_value(&child.value);
                    read_compiled_path_value(
                        vm,
                        env,
                        &child.value,
                        child.identity,
                        remaining,
                        strict_level,
                    )
                }
                other => Err(RuntimeError::new(format!(
                    "cannot access property '{property}' on value of type {}",
                    other.type_name()
                ))),
            }
        }
        CompiledPathSegment::EmbeddedIndex(value)
        | CompiledPathSegment::LiteralIndex(Literal::String(value)) => {
            let _index_slot = ValueStackReservation::reserve(usize::from(matches!(
                segment,
                CompiledPathSegment::LiteralIndex(_)
            )))?;
            let index = Value::String(vm.literal_string(value));
            if matches!(segment, CompiledPathSegment::LiteralIndex(_)) {
                vm.register_runtime_value(&index);
            }
            read_compiled_index(vm, env, current, identity, index, remaining, strict_level)
        }
        CompiledPathSegment::LiteralIndex(literal) => {
            let _index_slot = ValueStackReservation::reserve(1)?;
            let index = c4_set_copy_value(vm.literal_value(literal, strict_level));
            vm.register_runtime_value(&index);
            read_compiled_index(vm, env, current, identity, index, remaining, strict_level)
        }
    }
}

// Keep path-recursion state explicit: grouping these borrowed values only to
// satisfy the generic argument-count threshold obscures which fields change
// at each segment and adds no domain abstraction.
#[allow(clippy::too_many_arguments)]
fn read_compiled_index(
    vm: &Vm<'_>,
    env: &Environment,
    current: &Value,
    identity: Option<RawIdentity>,
    index: Value,
    remaining: &[CompiledPathSegment],
    strict_level: Option<u8>,
) -> Result<TrackedValue, RuntimeError> {
    let path_segment = PathSegment::Index(index.clone());
    let child_identity = identity
        .as_ref()
        .and_then(|identity| identity.identity_at(&path_segment));
    match current {
        Value::Nil | Value::Object(0) => Err(RuntimeError::new(
            "indexed access [index]: array, map or string expected, but got nil",
        )),
        Value::Array(elements) => {
            let index = array_index(&index)?;
            let nil = Value::Nil;
            let child = elements.get(index).unwrap_or(&nil);
            read_compiled_path_value(vm, env, child, child_identity, remaining, strict_level)
        }
        Value::Proplist(entries) => {
            let nil = Value::Nil;
            let child = entries.get_key(&index).unwrap_or(&nil);
            read_compiled_path_value(vm, env, child, child_identity, remaining, strict_level)
        }
        Value::String(text) => {
            let child = TrackedValue::runtime(string_index(text, &index)?).set_copy();
            vm.register_runtime_value(&child.value);
            read_compiled_path_value(
                vm,
                env,
                &child.value,
                child.identity,
                remaining,
                strict_level,
            )
        }
        target @ Value::Object(_) => {
            let _hook_stack = compiled_object_hook_stack(target, None)?;
            let child = vm.eval_index_tracked(
                TrackedValue {
                    value: target.clone(),
                    identity,
                },
                index,
                env,
            )?;
            vm.register_runtime_value(&child.value);
            read_compiled_path_value(
                vm,
                env,
                &child.value,
                child.identity,
                remaining,
                strict_level,
            )
        }
        other => Err(RuntimeError::new(format!(
            "cannot index value of type {}",
            other.type_name()
        ))),
    }
}

impl CompiledFunction {
    fn call_result(
        &self,
        instruction: usize,
        value: ReturnValue,
    ) -> Result<ReturnValue, RuntimeError> {
        let CompiledInstruction::Call { site } = self.instructions[instruction] else {
            return Err(RuntimeError::new("internal compiled result without a call"));
        };
        let value = if matches!(self.call_sites[site].kind, CompiledCallKind::Global { .. }) {
            materialize_target_call_result(value)
        } else {
            value
        };
        if self.call_sites[site].return_reference {
            Ok(value)
        } else {
            value.into_set_tracked_on_stack().map(ReturnValue::Value)
        }
    }

    fn compile(function: &Function) -> Option<Self> {
        CompiledFunctionBuilder::new(function)?.finish(function)
    }

    fn bindings(&self, vm: &Vm<'_>, env: &Environment) -> SmallVec<[Option<Binding>; 16]> {
        let bindings = self
            .slots
            .iter()
            .map(|slot| match slot.kind {
                CompiledSlotKind::Bare => env.binding(&slot.name).or_else(|| {
                    vm.global_variable_cell(&slot.name)
                        .map(|cell| Binding::Reference(vm.tracked_cell(cell)))
                }),
                CompiledSlotKind::FunctionVar => env.function_var_binding(&slot.name),
            })
            .collect::<SmallVec<_>>();
        #[cfg(test)]
        if bindings.spilled() {
            COMPILED_BINDING_HEAP_SPILLS.with(|count| count.set(count.get() + 1));
        }
        bindings
    }

    fn binding<'a>(
        &self,
        slot: usize,
        bindings: &'a mut [Option<Binding>],
        vm: &Vm<'_>,
        env: &Environment,
    ) -> Result<&'a Binding, RuntimeError> {
        let binding = &mut bindings[slot];
        if binding.is_none() {
            let name = &self.slots[slot].name;
            *binding = env.binding(name).or_else(|| {
                vm.global_variable_cell(name)
                    .map(|cell| Binding::Reference(vm.tracked_cell(cell)))
            });
        }
        binding.as_ref().ok_or_else(|| {
            RuntimeError::new(format!("undefined variable '{}'", self.slots[slot].name))
        })
    }

    fn resolve_call_targets(
        &self,
        vm: &Vm<'_>,
        env: &Environment,
    ) -> SmallVec<[CompiledCallBinding; 32]> {
        let mut call_targets = SmallVec::<[CompiledCallTarget; 32]>::new();
        // Scoped to the resolution prelude alone: the executed body below
        // attributes its own lookups to whichever path it reaches.
        let profiled_prelude =
            lookup_profile::enter_site(lookup_profile::LookupSite::CompiledPrelude);
        for site in &self.call_sites {
            let name = &site.name;
            let argument_count = site.argument_count;
            if let CompiledCallKind::Global { failsafe } = site.kind {
                call_targets.push(CompiledCallTarget::Global {
                    target: vm.global_call_target(name),
                    failsafe,
                });
                continue;
            }
            if let CompiledCallKind::Method {
                failsafe,
                reference,
            } = site.kind
            {
                let reference = reference
                    || site.return_reference
                        && (matches!(name.as_str(), "Local" | "LocalN" | "Var" | "EffectVar")
                            || vm
                                .own_or_global_script_function(name)
                                .is_some_and(|function| function.returns_reference));
                call_targets.push(CompiledCallTarget::Method {
                    failsafe,
                    reference,
                });
                continue;
            }
            if name == "this" && vm.has_bound_this(env) {
                call_targets.push(CompiledCallTarget::Builtin);
                continue;
            }
            if let Some(target) = vm.resolved_script_function(name, env.engine_scope) {
                call_targets.push(CompiledCallTarget::Script(CompiledScriptTarget {
                    function: target.function.resolved_snapshot(),
                    validate_compiled_source: target.validate_compiled_source,
                }));
                continue;
            }
            // One walk of the host tables serves both the reference guard and
            // the value target. `register_host_function` and
            // `register_host_reference_function` each remove a same-named
            // entry from the other table, so a name is in at most one of them
            // and asking for the reference first could never have changed
            // which target is selected — it only probed twice.
            let host = vm.resolved_host_function(name);
            if let Some(ResolvedHostFunction::Reference(function)) = host {
                call_targets.push(CompiledCallTarget::Host(CompiledHostTarget::Reference(
                    function.clone(),
                )));
                continue;
            }
            if let Some(ResolvedHostFunction::Value(function)) = host {
                call_targets.push(CompiledCallTarget::Host(CompiledHostTarget::Value(
                    function.clone(),
                )));
                continue;
            }
            if matches!(name.as_str(), "inherited" | "_inherited") {
                let target = if let Some(function) = vm.inherited_target(env) {
                    CompiledCallTarget::Script(CompiledScriptTarget {
                        function,
                        validate_compiled_source: true,
                    })
                } else if let Some(host) = vm.resolved_host_function(&env.function_name) {
                    CompiledCallTarget::Host(match host {
                        ResolvedHostFunction::Value(function) => {
                            CompiledHostTarget::Value(function.clone())
                        }
                        ResolvedHostFunction::Reference(function) => {
                            CompiledHostTarget::Reference(function.clone())
                        }
                    })
                } else {
                    CompiledCallTarget::Missing {
                        error: (name == "inherited").then(|| {
                            format!(
                                "inherited: no overloaded function (in {})",
                                env.function_name
                            )
                        }),
                    }
                };
                call_targets.push(target);
                continue;
            }
            let legacy_constant = env.strict_level.unwrap_or(0) < 2
                && (vm.global_constant_cell(name).is_some()
                    || vm
                        .constants
                        .is_some_and(|constants| constants.contains_key(name)));
            if legacy_constant && argument_count == 0 {
                call_targets.push(CompiledCallTarget::LegacyConstant);
                continue;
            }
            if Vm::is_global_vm_builtin(name)
                || name == "Par" && argument_count <= 1
                || name == "EffectVar" && site.return_reference
            {
                call_targets.push(CompiledCallTarget::Builtin);
                continue;
            }
            call_targets.push(CompiledCallTarget::Missing {
                error: Some(if legacy_constant {
                    "parameters not allowed in functional usage of constants".to_owned()
                } else {
                    format!("unknown function '{name}'")
                }),
            });
        }
        let bindings = call_targets
            .into_iter()
            .zip(&self.call_sites)
            .map(|(target, site)| {
                let mut reference_parameters = 0u32;
                for index in 0..site.argument_count {
                    let selected = match &target {
                        CompiledCallTarget::Global {
                            target: RetainedCallTarget::Script(target),
                            ..
                        } => target
                            .function
                            .params
                            .get(index)
                            .is_some_and(|param| param.is_reference),
                        CompiledCallTarget::Global {
                            target: RetainedCallTarget::HostReference(function),
                            ..
                        } => function.wants_reference(index),
                        CompiledCallTarget::Script(target) => target
                            .function
                            .params
                            .get(index)
                            .is_some_and(|param| param.is_reference),
                        CompiledCallTarget::Host(CompiledHostTarget::Reference(function)) => {
                            function.wants_reference(index)
                        }
                        CompiledCallTarget::Method { .. } => vm
                            .own_or_global_script_function(&site.name)
                            .and_then(|function| function.params.get(index))
                            .is_some_and(|param| param.is_reference),
                        _ => false,
                    };
                    let engine = vm
                        .reference_parameter_probe
                        .is_some_and(|probe| probe(&site.name, index));
                    if (selected || engine) && index < u32::BITS as usize {
                        reference_parameters |= 1 << index;
                    }
                }
                CompiledCallBinding {
                    target,
                    reference_parameters,
                }
            })
            .collect();
        drop(profiled_prelude);
        bindings
    }

    #[allow(clippy::too_many_arguments)]
    fn suspend(
        &self,
        error: RuntimeError,
        this_value: Value,
        request: Rc<dyn Any>,
        resume_value: Value,
        function: &Function,
        compiled: &Arc<CompiledFunction>,
        call_targets: &[CompiledCallBinding],
        env: &Environment,
        depth: usize,
        caller: &Option<ScriptCallerContext>,
        instruction: usize,
        stack: SmallVec<[ReturnValue; 16]>,
        registered_slots: SmallVec<[bool; 16]>,
        assignment_targets: SmallVec<[(usize, LValueRef); 4]>,
        iterators: SmallVec<[CompiledIterator; 2]>,
        stack_value_stack: ValueStackReservation,
        frame_value_stack: usize,
        pending: PendingContinuation,
    ) -> RuntimeError {
        let frame = CompiledContinuationFrame {
            // A compiled call normally borrows its installed function.  Only
            // a real host boundary needs an owned C4Aul-style target for the
            // continuation that outlives this invocation.
            function: Arc::new(function.clone()),
            compiled: Arc::clone(compiled),
            call_targets: call_targets.iter().cloned().collect(),
            env: env.clone(),
            depth,
            caller: caller.clone(),
            returns_reference: function.returns_reference,
            instruction,
            stack,
            registered_slots,
            assignment_targets,
            iterators,
            stack_value_stack,
            frame_value_stack,
            pending,
        };
        error.with_control(RuntimeControl::HostContinuation {
            request,
            resume_value,
            continuation: Some(Box::new(ScriptContinuation {
                frame: ContinuationFrame::Compiled(frame),
                this_value,
            })),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn execute(
        &self,
        vm: &Vm<'_>,
        env: &mut Environment,
        depth: usize,
        function: &Function,
        caller: Option<ScriptCallerContext>,
        compiled: Arc<CompiledFunction>,
        frame_value_stack: usize,
    ) -> Result<ControlFlow, RuntimeError> {
        let _execution_timer = crate::execution_profile::ExecutionTimer::enter();
        let bindings = self.bindings(vm, env);
        let call_targets = self.resolve_call_targets(vm, env);
        let state = CompiledExecutionState {
            instruction: 0,
            stack: SmallVec::<[ReturnValue; 16]>::with_capacity(self.max_stack),
            registered_slots: SmallVec::<[bool; 16]>::from_elem(false, bindings.len()),
            assignment_targets: SmallVec::<[(usize, LValueRef); 4]>::new(),
            iterators: SmallVec::new(),
            stack_value_stack: ValueStackReservation::empty(),
            pending: None,
            resume_value: None,
        };
        #[cfg(test)]
        if state.stack.spilled() {
            COMPILED_STACK_HEAP_SPILLS.with(|count| count.set(count.get() + 1));
        }
        #[cfg(test)]
        if state.registered_slots.spilled() {
            COMPILED_REGISTERED_SLOT_HEAP_SPILLS.with(|count| count.set(count.get() + 1));
        }
        self.execute_state(
            vm,
            env,
            depth,
            function,
            caller,
            compiled,
            bindings,
            call_targets,
            state,
            frame_value_stack,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_state(
        &self,
        vm: &Vm<'_>,
        env: &mut Environment,
        depth: usize,
        function: &Function,
        caller: Option<ScriptCallerContext>,
        compiled: Arc<CompiledFunction>,
        mut bindings: SmallVec<[Option<Binding>; 16]>,
        call_targets: SmallVec<[CompiledCallBinding; 32]>,
        state: CompiledExecutionState,
        frame_value_stack: usize,
    ) -> Result<ControlFlow, RuntimeError> {
        let _execution_timer = crate::execution_profile::ExecutionTimer::enter();
        let mut direct_diagnostic = env.direct_exec_context.as_ref().map(|context| {
            ScriptDiagnosticGuard::enter_direct(context.frame.clone(), context.profile_on_error)
        });
        let CompiledExecutionState {
            mut stack,
            mut registered_slots,
            mut assignment_targets,
            mut iterators,
            mut stack_value_stack,
            mut instruction,
            pending,
            resume_value,
        } = state;
        let resumed_method = pending.is_some()
            && matches!(&self.instructions[instruction], CompiledInstruction::Call { site }
                if matches!(self.call_sites[*site].kind, CompiledCallKind::Method { .. } | CompiledCallKind::Global { .. }));
        if let Some(pending) = pending {
            let _pin_registry =
                self.legacy_pin_instructions[instruction].then(LegacyPathPinRegistryGuard::enter);
            match pending {
                PendingContinuation::Host { value, .. } => {
                    if resumed_method {
                        stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    }
                    let value = TrackedValue::runtime(resume_value.unwrap_or(value)).set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                    instruction += 1;
                }
                PendingContinuation::Child(child) => match match resume_value {
                    Some(value) => child.resume_with_value(vm, value),
                    None => child.resume(vm),
                }? {
                    ContinuationResult::Complete(value) => {
                        if resumed_method {
                            stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                        }
                        let value = self.call_result(instruction, value)?;
                        if let ReturnValue::Value(value) = &value {
                            vm.register_runtime_value(&value.value);
                        }
                        stack.push(value);
                        instruction += 1;
                    }
                    ContinuationResult::Suspended(suspension) => {
                        let (request, resume_value, child) = suspension.into_parts();
                        return Err(self.suspend(
                            RuntimeError::new("script execution suspended by nested host callback"),
                            vm.this_value.clone(),
                            request,
                            resume_value,
                            function,
                            &compiled,
                            &call_targets,
                            env,
                            depth,
                            &caller,
                            instruction,
                            stack,
                            registered_slots,
                            assignment_targets,
                            iterators,
                            stack_value_stack,
                            frame_value_stack,
                            PendingContinuation::Child(child),
                        ));
                    }
                },
                PendingContinuation::Native {
                    state,
                    parameter_slots,
                } => match resume_native_continuation(
                    state,
                    resume_value.unwrap_or(Value::Nil),
                    parameter_slots,
                )? {
                    NativeResumeOutcome::Complete(value) => {
                        if resumed_method {
                            stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                        }
                        let value = TrackedValue::runtime(value).set_copy();
                        vm.register_runtime_value(&value.value);
                        stack.push(ReturnValue::Value(value));
                        instruction += 1;
                    }
                    NativeResumeOutcome::Suspended {
                        request,
                        resume_value,
                        pending,
                    } => {
                        return Err(self.suspend(
                            RuntimeError::new(
                                "script execution suspended by nested native callback",
                            ),
                            vm.this_value.clone(),
                            request,
                            resume_value,
                            function,
                            &compiled,
                            &call_targets,
                            env,
                            depth,
                            &caller,
                            instruction,
                            stack,
                            registered_slots,
                            assignment_targets,
                            iterators,
                            stack_value_stack,
                            frame_value_stack,
                            pending,
                        ));
                    }
                },
            }
        }
        loop {
            let _pin_registry =
                self.legacy_pin_instructions[instruction].then(LegacyPathPinRegistryGuard::enter);
            let opcode = &self.instructions[instruction];
            let pushes_slot = matches!(
                opcode,
                CompiledInstruction::This
                    | CompiledInstruction::Literal(_)
                    | CompiledInstruction::Load(_)
                    | CompiledInstruction::LoadReference(_)
                    | CompiledInstruction::LoadNamedReference(_)
                    | CompiledInstruction::LoadName(_)
                    | CompiledInstruction::LoadArgument { .. }
                    | CompiledInstruction::LoadPath { .. }
                    | CompiledInstruction::BeginAssignment(_)
                    | CompiledInstruction::IncrementSlot { .. }
                    | CompiledInstruction::MakeArray(0)
                    | CompiledInstruction::MakeProplist(0)
                    | CompiledInstruction::LegacyParameters { count: 0, .. }
            );
            stack_value_stack
                .resize_to(stack.len() + assignment_targets.len() + usize::from(pushes_slot))?;
            match opcode {
                CompiledInstruction::Error(message) => {
                    return Err(RuntimeError::new(message.clone()))
                }
                CompiledInstruction::This => {
                    let value = TrackedValue::runtime(vm.this_value.clone());
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::Literal(literal) => {
                    let value =
                        TrackedValue::literal(vm.literal_value(literal, env.strict_level), literal)
                            .set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::Load(slot) => {
                    let value = match &bindings[*slot] {
                        Some(binding) => binding.read_tracked()?.set_copy(),
                        None => vm.compiled_named_value(&self.slots[*slot].name, env)?,
                    };
                    if !registered_slots[*slot] {
                        vm.register_runtime_value(&value.value);
                        registered_slots[*slot] = true;
                    }
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::LoadReference(slot) => {
                    stack.push(ReturnValue::Reference(
                        self.binding(*slot, &mut bindings, vm, env)?.lvalue(),
                    ));
                }
                CompiledInstruction::LoadNamedReference(name) => {
                    let reference = env.lvalue(name).or_else(|| {
                        vm.global_variable_cell(name)
                            .map(|cell| vm.tracked_cell(cell))
                    });
                    stack.push(match reference {
                        Some(reference) => ReturnValue::Reference(reference),
                        None => ReturnValue::Value(vm.compiled_named_value(name, env)?),
                    });
                }
                CompiledInstruction::SlotReference { local } => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled slot index missing"))?
                        .into_value()?;
                    let index =
                        Vm::slot_index_from_value(if *local { "Local()" } else { "Var()" }, value)?;
                    let value = if *local && vm.retain_global_call_context_for_host_paths {
                        ReturnValue::Value(TrackedValue::runtime(Value::Nil))
                    } else if *local && index < 0 {
                        ReturnValue::Reference(vm.tracked_cell(value_cell(Value::Nil)))
                    } else if *local {
                        ReturnValue::Reference(
                            vm.tracked_cell(env.object_state.local_slot_cell(index)),
                        )
                    } else {
                        ReturnValue::Reference(
                            vm.tracked_cell(frame_slot_cell(&env.frame_locals, index)),
                        )
                    };
                    stack.push(value);
                }
                CompiledInstruction::IndexReference { embedded, create } => {
                    let index = match embedded {
                        Some(key) => Value::String(vm.literal_string(key)),
                        None => stack
                            .pop()
                            .ok_or_else(|| RuntimeError::new("internal compiled index missing"))?
                            .into_value()?,
                    };
                    let base = stack.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled indexed base missing")
                    })?;
                    let _registry = LegacyPathPinRegistryGuard::enter();
                    let _creation = if *create {
                        LegacyPathPinCreationGuard::enter()
                    } else {
                        LegacyPathPinCreationGuard::suspend()
                    };
                    let value =
                        vm.index_value_reference_or_value_with_hook_stack(base, index, env, None)?;
                    stack.push(value);
                }
                CompiledInstruction::PropertyReference {
                    property,
                    assignment,
                    create,
                } => {
                    let base = stack.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled property base missing")
                    })?;
                    if *assignment {
                        if let ReturnValue::Reference(reference) = &base {
                            if reference.resolved_legacy_value().is_none()
                                && !matches!(reference, LValueRef::HostPath { .. })
                            {
                                let collection = reference.read()?;
                                if !matches!(
                                    collection,
                                    Value::Nil | Value::Object(_) | Value::Proplist(_)
                                ) {
                                    return Err(RuntimeError::new(format!(
                                        "cannot assign property '{property}' on value of type {}",
                                        collection.type_name()
                                    )));
                                }
                            }
                        }
                    }
                    let _registry = LegacyPathPinRegistryGuard::enter();
                    let _creation = if *create {
                        LegacyPathPinCreationGuard::enter()
                    } else {
                        LegacyPathPinCreationGuard::suspend()
                    };
                    let value =
                        vm.property_reference_or_value_with_hook_stack(base, property, env, None)?;
                    stack.push(value);
                }
                CompiledInstruction::JumpIfValueArgument {
                    site,
                    index,
                    target,
                } => {
                    let wants_reference = *index < u32::BITS as usize
                        && call_targets[*site].reference_parameters & (1 << index) != 0;
                    if !wants_reference {
                        instruction = *target;
                        continue;
                    }
                }
                CompiledInstruction::Dereference => {
                    let operand = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled result missing"))?;
                    let value = Vm::materialize_set_no_ref_result(operand)?;
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::LegacyParameters {
                    count,
                    forward_rest,
                } => {
                    let start = stack.len().checked_sub(*count).ok_or_else(|| {
                        RuntimeError::new("internal compiled legacy parameters missing")
                    })?;
                    let value = if *count == 0 {
                        if *forward_rest {
                            env.call_args
                                .get(env.named_param_count)
                                .map(Binding::read_tracked)
                                .transpose()?
                                .unwrap_or_else(|| TrackedValue::runtime(Value::Nil))
                        } else {
                            TrackedValue::runtime(Value::Nil)
                        }
                    } else {
                        stack.truncate(start + 1);
                        stack
                            .pop()
                            .ok_or_else(|| {
                                RuntimeError::new("internal compiled legacy result missing")
                            })?
                            .into_tracked()?
                    };
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::AppendReference => {
                    stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    let base = stack.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled append base missing")
                    })?;
                    let _creation = LegacyPathPinCreationGuard::enter();
                    stack.push(match base {
                        ReturnValue::Reference(reference) => {
                            ReturnValue::Reference(vm.append_array_slot(reference)?)
                        }
                        ReturnValue::Value(value) => match value.value {
                            Value::Array(elements) if elements.len() < ARRAY_MAX_SIZE => {
                                ReturnValue::Value(TrackedValue::runtime(Value::Nil))
                            }
                            Value::Array(_) => return Err(RuntimeError::new("out of memory")),
                            other => {
                                return Err(RuntimeError::new(format!(
                                    "array append accesss: can't access {} as an array!",
                                    other.type_name()
                                )))
                            }
                        },
                    });
                }
                CompiledInstruction::Materialize => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled value missing"))?
                        .into_tracked()?
                        .set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::MaterializeArgument { site, index } => {
                    let wants_reference = *index < u32::BITS as usize
                        && call_targets[*site].reference_parameters & (1 << index) != 0;
                    if !wants_reference {
                        let value = stack.pop().ok_or_else(|| {
                            RuntimeError::new("internal compiled argument missing")
                        })?;
                        let value = match value {
                            ReturnValue::Value(value) => value,
                            ReturnValue::Reference(reference) => {
                                reference.read_tracked()?.set_copy()
                            }
                        };
                        stack.push(ReturnValue::Value(value));
                    }
                }
                CompiledInstruction::LoadArgument { slot, site, index } => {
                    let wants_reference = *index < u32::BITS as usize
                        && call_targets[*site].reference_parameters & (1 << index) != 0;
                    let operand = if wants_reference {
                        ReturnValue::Reference(
                            self.binding(*slot, &mut bindings, vm, env)?.lvalue(),
                        )
                    } else {
                        let value = match &bindings[*slot] {
                            Some(binding) => binding.read_tracked()?.set_copy(),
                            None => vm.compiled_named_value(&self.slots[*slot].name, env)?,
                        };
                        vm.register_runtime_value(&value.value);
                        ReturnValue::Value(value)
                    };
                    stack.push(operand);
                }
                CompiledInstruction::LoadName(name) => {
                    let value = vm.compiled_named_value(name, env)?;
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::LoadPath { slot, segments } => {
                    let has_index = segments.iter().any(|segment| {
                        matches!(
                            segment,
                            CompiledPathSegment::EmbeddedIndex(_)
                                | CompiledPathSegment::LiteralIndex(_)
                        )
                    });
                    let needs_reference_path = segments.iter().any(|segment| {
                        matches!(
                            segment,
                            CompiledPathSegment::LiteralIndex(Literal::Int(index)) if *index < 0
                        ) || matches!(segment, CompiledPathSegment::LiteralIndex(Literal::C4Id(_)))
                    });
                    let value = if needs_reference_path {
                        read_compiled_indexed_path(
                            vm,
                            env,
                            self.binding(*slot, &mut bindings, vm, env)?,
                            segments,
                            env.strict_level,
                        )?
                    } else {
                        let value = read_compiled_path(
                            vm,
                            env,
                            self.binding(*slot, &mut bindings, vm, env)?,
                            segments,
                            !has_index && !registered_slots[*slot],
                        )?;
                        if has_index {
                            vm.register_runtime_value(&value.value);
                        } else {
                            registered_slots[*slot] = true;
                        }
                        value
                    };
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::BeginAssignment(slot) => {
                    let reference = self.binding(*slot, &mut bindings, vm, env)?.lvalue();
                    assignment_targets.push((*slot, reference));
                }
                CompiledInstruction::Store(slot) => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    self.binding(*slot, &mut bindings, vm, env)?
                        .write_tracked(value.into_tracked()?)?;
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::StoreReference { copy_result } => {
                    let value = stack.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled assignment value missing")
                    })?;
                    let target = stack.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled assignment target missing")
                    })?;
                    let reference = match target {
                        ReturnValue::Reference(reference) => reference,
                        ReturnValue::Value(left) => {
                            return Err(RuntimeError::new(format!(
                                "operator \"=\" left side: got \"{}\", but expected \"&\"!",
                                Vm::c4v_type_name(left.value.c4v_type())
                            )))
                        }
                    };
                    if let Some(left) = reference.resolved_legacy_value() {
                        return Err(RuntimeError::new(format!(
                            "operator \"=\" left side: got \"{}\", but expected \"&\"!",
                            Vm::c4v_type_name(left.value.c4v_type())
                        )));
                    }
                    let value = match value {
                        ReturnValue::Reference(reference) => reference.read_tracked()?.set_copy(),
                        ReturnValue::Value(value) => value,
                    };
                    reference.write_tracked(value)?;
                    stack.push(if *copy_result {
                        ReturnValue::Value(reference.read_tracked()?.set_copy())
                    } else {
                        ReturnValue::Reference(reference)
                    });
                }
                CompiledInstruction::InvalidAssignment { operator } => {
                    stack.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled assignment rhs missing")
                    })?;
                    let left = stack
                        .pop()
                        .ok_or_else(|| {
                            RuntimeError::new("internal compiled assignment target missing")
                        })?
                        .into_value()?;
                    return Err(RuntimeError::new(format!(
                        "operator \"{operator}\" left side: got \"{}\", but expected \"&\"!",
                        left.type_name()
                    )));
                }
                CompiledInstruction::StoreAssignment(slot) => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let (target_slot, reference) = assignment_targets.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled assignment target underflow")
                    })?;
                    debug_assert_eq!(target_slot, *slot);
                    reference.write_tracked(value.into_tracked()?)?;
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::StoreKeep { slot, copy_result } => {
                    let (target_slot, reference) = assignment_targets.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled assignment target underflow")
                    })?;
                    debug_assert_eq!(target_slot, *slot);
                    let value = stack
                        .last()
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    reference.write_tracked(value.into_tracked()?)?;
                    let value = reference.read_tracked()?;
                    *stack
                        .last_mut()
                        .expect("the assigned value was just read from this slot") = if *copy_result
                    {
                        ReturnValue::Value(value.set_copy())
                    } else {
                        ReturnValue::Value(value)
                    };
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::Unary(operation) => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let value =
                        TrackedValue::runtime(vm.eval_unary(operation, value.into_value()?)?)
                            .set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::Binary(operation) => {
                    let right = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let left = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let left = left.into_tracked()?;
                    let right = right.into_tracked()?;
                    let value = match operation {
                        BinaryOp::Concat => {
                            vm.eval_concat_tracked(left, right, env.strict_level, "..")?
                        }
                        BinaryOp::Equal | BinaryOp::NotEqual => {
                            let equal = vm.values_equal(
                                &left.value,
                                &right.value,
                                env.strict_level,
                                left.identity.as_ref(),
                                right.identity.as_ref(),
                            );
                            TrackedValue::runtime(Value::Bool(
                                if matches!(operation, BinaryOp::Equal) {
                                    equal
                                } else {
                                    !equal
                                },
                            ))
                        }
                        BinaryOp::And => TrackedValue::runtime(Value::Bool(
                            left.value.as_bool() && right.value.as_bool(),
                        )),
                        BinaryOp::Or => TrackedValue::runtime(Value::Bool(
                            left.value.as_bool() || right.value.as_bool(),
                        )),
                        _ => TrackedValue::runtime(vm.eval_binary(
                            left.value,
                            operation,
                            right.value,
                            env.strict_level,
                            None,
                        )?),
                    }
                    .set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::CompoundStore {
                    slot,
                    operation,
                    operator,
                } => {
                    let right = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let (target_slot, reference) = assignment_targets.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled assignment target underflow")
                    })?;
                    debug_assert_eq!(target_slot, *slot);
                    let left = reference.read_tracked()?;
                    let value = TrackedValue::runtime(vm.eval_binary(
                        left.value,
                        operation,
                        right.into_value()?,
                        env.strict_level,
                        Some(operator),
                    )?);
                    reference.write_tracked(value)?;
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::CompoundReference {
                    operation,
                    operator,
                    copy_result,
                } => {
                    stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    let right = stack
                        .pop()
                        .ok_or_else(|| {
                            RuntimeError::new("internal compiled compound value missing")
                        })?
                        .into_tracked()?;
                    let target = stack.pop().ok_or_else(|| {
                        RuntimeError::new("internal compiled compound target missing")
                    })?;
                    let expected =
                        if matches!(operation, BinaryOp::Concat | BinaryOp::NilCoalescing) {
                            "&"
                        } else {
                            "int&"
                        };
                    let invalid = |left: TrackedValue| {
                        RuntimeError::new(format!(
                        "operator \"{operator}\" left side: got \"{}\", but expected \"{expected}\"!",
                        Vm::c4v_type_name(left.value.c4v_type())
                    ))
                    };
                    let reference = match target {
                        ReturnValue::Reference(reference) => reference,
                        ReturnValue::Value(left) => return Err(invalid(left)),
                    };
                    if let Some(left) = reference.resolved_legacy_value() {
                        return Err(invalid(left));
                    }
                    let result = match operation {
                        BinaryOp::NilCoalescing => right,
                        BinaryOp::Concat => vm.eval_concat_tracked(
                            reference.read_tracked()?,
                            right,
                            env.strict_level,
                            operator,
                        )?,
                        _ => TrackedValue::runtime(vm.eval_binary(
                            reference.read()?,
                            operation,
                            right.value,
                            env.strict_level,
                            Some(operator),
                        )?),
                    };
                    reference.write_tracked(result.clone())?;
                    stack.push(if *copy_result {
                        ReturnValue::Value(result.set_copy())
                    } else {
                        ReturnValue::Reference(reference)
                    });
                }
                CompiledInstruction::IncrementReference {
                    delta,
                    return_old,
                    copy_result,
                } => {
                    stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    let operand = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled counter missing"))?;
                    let reference = match operand {
                        ReturnValue::Reference(reference) => reference,
                        ReturnValue::Value(value) => {
                            let operator = if *delta > 0 { "++" } else { "--" };
                            return Err(RuntimeError::new(format!(
                                "operator \"{operator}\": got \"{}\", but expected \"int&\"!",
                                Vm::c4v_type_name(value.value.c4v_type())
                            )));
                        }
                    };
                    let operation = if *delta > 0 { "increment" } else { "decrement" };
                    let old = Vm::counter_operand(reference.read()?, operation)?;
                    reference.write(Value::Int(old.wrapping_add(*delta)))?;
                    stack.push(if *return_old {
                        ReturnValue::Value(TrackedValue::runtime(Value::Int(old)))
                    } else if *copy_result {
                        ReturnValue::Value(reference.read_tracked()?.set_copy())
                    } else {
                        ReturnValue::Reference(reference)
                    });
                }
                CompiledInstruction::IncrementSlot { slot, delta } => {
                    let reference = self.binding(*slot, &mut bindings, vm, env)?.lvalue();
                    let operation = if *delta > 0 { "increment" } else { "decrement" };
                    let old_value = Vm::counter_operand(reference.read()?, operation)?;
                    reference.write(Value::Int(old_value.wrapping_add(*delta)))?;
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::Call { site } => {
                    let call_site = &self.call_sites[*site];
                    let name = if matches!(call_site.name.as_str(), "inherited" | "_inherited") {
                        &env.function_name.clone()
                    } else {
                        &call_site.name
                    };
                    let argument_count = call_site.argument_count;
                    let argument_start = stack
                        .len()
                        .checked_sub(argument_count)
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    let mut arguments = stack
                        .drain(argument_start..)
                        .map(|value| match value {
                            ReturnValue::Value(value) => CallArg::Value(value),
                            ReturnValue::Reference(reference) => CallArg::Reference(reference),
                        })
                        .collect::<CallArgs>();
                    stack_value_stack.shrink(argument_count);
                    #[cfg(test)]
                    record_call_arg_heap_spill(arguments.spilled());
                    let target = &call_targets[*site].target;
                    let parameter_limit = match target {
                        CompiledCallTarget::Host(CompiledHostTarget::Value(function)) => {
                            function.parameter_count().unwrap_or(MAX_CALL_PARAMETERS)
                        }
                        CompiledCallTarget::Host(CompiledHostTarget::Reference(function)) => {
                            function.parameter_count().unwrap_or(MAX_CALL_PARAMETERS)
                        }
                        _ => MAX_CALL_PARAMETERS,
                    };
                    arguments.truncate(parameter_limit);
                    if call_site.forward_rest {
                        Vm::append_forwarded_args(&mut arguments, env, parameter_limit)?;
                    }
                    let sweep_cursor = object_reference_sweep_cursor();
                    let result = match target {
                        CompiledCallTarget::Builtin
                            if name == "EffectVar" && call_site.return_reference =>
                        {
                            vm.effect_slot_from_call_args(arguments, env)
                        }
                        CompiledCallTarget::Global { target, failsafe } => {
                            stack.pop().ok_or_else(|| {
                                RuntimeError::new("internal compiled global target missing")
                            })?;
                            let known = !matches!(target, RetainedCallTarget::Dynamic);
                            stack_value_stack.resize_to(
                                stack.len()
                                    + assignment_targets.len()
                                    + 1
                                    + if known { MAX_CALL_PARAMETERS } else { 0 },
                            )?;
                            if !known {
                                if *failsafe {
                                    Ok(ReturnValue::Value(TrackedValue::runtime(Value::Nil)))
                                } else {
                                    Err(RuntimeError::new(format!("unknown function '{name}'")))
                                }
                            } else {
                                let _context =
                                    GlobalCallContextGuard::enter(vm.global_call_context_hook);
                                let global_vm = vm.engine_global_vm();
                                let _parameters = (!matches!(target, RetainedCallTarget::Builtin))
                                    .then(|| CallParameterOverrideGuard::enter(0));
                                let caller = Some(env.caller_context());
                                global_vm.invoke_retained_global_target(
                                    target.clone(),
                                    name,
                                    arguments,
                                    depth,
                                    env,
                                    caller,
                                )
                            }
                        }
                        CompiledCallTarget::Missing { error } => match error {
                            Some(message) => Err(RuntimeError::new(message.clone())),
                            None => Ok(ReturnValue::Value(TrackedValue::runtime(Value::Nil))),
                        },
                        CompiledCallTarget::Builtin => {
                            let _parameters =
                                ValueStackReservation::reserve(if name == "SetLocal" {
                                    0
                                } else {
                                    arguments.len()
                                })?;
                            let caller = Some(env.caller_context());
                            vm.invoke_retained_direct_target(
                                RetainedCallTarget::Builtin,
                                name,
                                arguments,
                                depth,
                                env,
                                caller,
                                call_site.return_reference,
                            )
                        }
                        CompiledCallTarget::Method {
                            failsafe,
                            reference,
                        } => {
                            let receiver = stack.pop().ok_or_else(|| {
                                RuntimeError::new("internal compiled receiver missing")
                            })?;
                            arguments.truncate(MAX_CALL_PARAMETERS);
                            // AB_CALL retains the receiver and ten parameter
                            // slots, including across a nested suspension.
                            stack_value_stack.resize_to(
                                stack.len() + assignment_targets.len() + 1 + MAX_CALL_PARAMETERS,
                            )?;
                            if *reference {
                                vm.invoke_method_reference_call_args_raw(
                                    receiver.into_value()?,
                                    name,
                                    arguments,
                                    sweep_cursor,
                                    env,
                                    depth,
                                )
                            } else {
                                vm.invoke_property_call_with_target_call_args_raw(
                                    receiver.into_value()?,
                                    name,
                                    arguments,
                                    *failsafe,
                                    call_site.return_reference,
                                    env,
                                    depth,
                                )
                            }
                        }
                        CompiledCallTarget::Host(CompiledHostTarget::Value(target))
                            if call_site.return_reference && name == "EffectVar" =>
                        {
                            vm.effect_slot_from_registered_host_call_args(target, arguments, env)
                        }
                        CompiledCallTarget::Host(CompiledHostTarget::Value(target)) => vm
                            .invoke_resolved_host_value(
                                name,
                                ResolvedHostFunction::Value(target),
                                arguments,
                                depth + 1,
                                Some(env.caller_context()),
                            )
                            .map(TrackedValue::runtime)
                            .map(ReturnValue::Value),
                        CompiledCallTarget::Host(CompiledHostTarget::Reference(target)) => vm
                            .invoke_resolved_host_value(
                                name,
                                ResolvedHostFunction::Reference(target),
                                arguments,
                                depth + 1,
                                Some(env.caller_context()),
                            )
                            .map(TrackedValue::runtime)
                            .map(ReturnValue::Value),
                        CompiledCallTarget::Script(target) => {
                            let target = ScriptFunctionTarget {
                                function: target.function.as_ref(),
                                validate_compiled_source: target.validate_compiled_source,
                            };
                            vm.invoke_resolved_script_raw(
                                name,
                                target,
                                arguments,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.caller_context()),
                            )
                        }
                        CompiledCallTarget::LegacyConstant => {
                            debug_assert!(arguments.is_empty());
                            vm.compiled_named_value(name, env).map(ReturnValue::Value)
                        }
                    };
                    let value = match result {
                        Ok(value) => value,
                        Err(mut error) => {
                            let host_parameter_slots = error.take_host_parameter_slots();
                            let Some(control) = error.take_control() else {
                                return Err(error);
                            };
                            let RuntimeControl::HostContinuation {
                                request,
                                resume_value,
                                continuation,
                            } = control;
                            let pending = match continuation {
                                None => PendingContinuation::Host {
                                    value: resume_value.clone(),
                                    parameter_slots: host_parameter_slots.unwrap_or(0),
                                },
                                Some(continuation) => match continuation
                                    .downcast::<ScriptContinuation>()
                                {
                                    Ok(continuation) => PendingContinuation::Child(continuation),
                                    Err(continuation) => {
                                        match continuation.downcast::<NativeContinuationState>() {
                                            Ok(state) => PendingContinuation::Native {
                                                state: *state,
                                                parameter_slots: host_parameter_slots.unwrap_or(0),
                                            },
                                            Err(continuation) => {
                                                return Err(error.with_control(
                                                    RuntimeControl::HostContinuation {
                                                        request,
                                                        resume_value,
                                                        continuation: Some(continuation),
                                                    },
                                                ));
                                            }
                                        }
                                    }
                                },
                            };
                            return Err(self.suspend(
                                error,
                                vm.this_value.clone(),
                                request,
                                resume_value,
                                function,
                                &compiled,
                                &call_targets,
                                env,
                                depth,
                                &caller,
                                instruction,
                                stack,
                                registered_slots,
                                assignment_targets,
                                iterators,
                                stack_value_stack,
                                frame_value_stack,
                                pending,
                            ));
                        }
                    };
                    if matches!(
                        target,
                        CompiledCallTarget::Method { .. } | CompiledCallTarget::Global { .. }
                    ) {
                        stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    }
                    for retained in &mut stack {
                        retained.clear_object_reference_sweeps(sweep_cursor);
                    }
                    let value = self.call_result(instruction, value)?;
                    if let ReturnValue::Value(value) = &value {
                        vm.register_runtime_value(&value.value);
                    }
                    stack.push(value);
                }
                CompiledInstruction::MakeArray(element_count) => {
                    let start = stack
                        .len()
                        .checked_sub(*element_count)
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let value = TrackedValue::array(
                        stack
                            .drain(start..)
                            .map(ReturnValue::into_tracked)
                            .collect::<Result<_, _>>()?,
                    )
                    .set_copy();
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::MakeProplist(entry_count) => {
                    let value_count = entry_count.checked_mul(2).ok_or_else(|| {
                        RuntimeError::new("internal compiled proplist size overflow")
                    })?;
                    let start = stack
                        .len()
                        .checked_sub(value_count)
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let entries = {
                        let mut values = stack.drain(start..);
                        let mut entries = Vec::with_capacity(*entry_count);
                        while let Some(key) = values.next() {
                            let value = values.next().ok_or_else(|| {
                                RuntimeError::new("internal compiled proplist value missing")
                            })?;
                            entries.push((key.into_value()?, value.into_tracked()?));
                        }
                        entries
                    };
                    let value = TrackedValue::proplist(entries).set_copy();
                    stack.push(ReturnValue::Value(value));
                }
                CompiledInstruction::Pop => {
                    stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                }
                CompiledInstruction::JumpIfNotNil {
                    target,
                    materialize,
                } => {
                    stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    let condition = stack
                        .last_mut()
                        .ok_or_else(|| RuntimeError::new("internal compiled condition missing"))?;
                    let value = match condition {
                        ReturnValue::Reference(reference) => reference.read_tracked()?,
                        ReturnValue::Value(value) => value.clone(),
                    };
                    if !matches!(value.value, Value::Nil) {
                        if *materialize {
                            *condition = ReturnValue::Value(value);
                        }
                        instruction = *target;
                        continue;
                    }
                }
                CompiledInstruction::JumpIfNil(target) => {
                    if matches!(
                        stack
                            .last()
                            .ok_or_else(|| RuntimeError::new(
                                "internal compiled nil guard missing"
                            ))?
                            .as_value()?,
                        Value::Nil
                    ) {
                        instruction = *target;
                        continue;
                    }
                }
                CompiledInstruction::JumpIfGotoBound(target) => {
                    if env.lvalue("goto").is_some()
                        || vm.global_variable_cell("goto").is_some()
                        || vm.global_constant_cell("goto").is_some()
                    {
                        instruction = *target;
                        continue;
                    }
                }
                CompiledInstruction::JumpNotNil(target) => {
                    let condition = stack
                        .last()
                        .ok_or_else(|| RuntimeError::new("internal compiled condition missing"))?;
                    if !matches!(condition.as_value()?, Value::Nil) {
                        instruction = *target;
                        continue;
                    }
                    stack.pop();
                }
                CompiledInstruction::JumpAnd(target) => {
                    let condition = stack
                        .last()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    if !condition.as_value()?.as_bool() {
                        instruction = *target;
                        continue;
                    }
                    stack.pop();
                }
                CompiledInstruction::JumpOr(target) => {
                    let condition = stack
                        .last()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    if condition.as_value()?.as_bool() {
                        instruction = *target;
                        continue;
                    }
                    stack.pop();
                }
                CompiledInstruction::JumpIfFalse(target) => {
                    let condition = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    if !condition.as_value()?.as_bool() {
                        instruction = *target;
                        continue;
                    }
                }
                CompiledInstruction::Jump(target) => {
                    instruction = *target;
                    continue;
                }
                CompiledInstruction::Return => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    if let Some(diagnostic) = &mut direct_diagnostic {
                        diagnostic.returned(&value.as_value()?);
                    }
                    return Ok(ControlFlow::Return(value));
                }
                CompiledInstruction::IteratorInit { map } => {
                    let iterable = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled iterable missing"))?
                        .into_value()?;
                    stack_value_stack.resize_to(stack.len() + assignment_targets.len())?;
                    // AB_FOREACH retains a fixed control prefix, not one C4 slot per item.
                    let value_stack = ValueStackReservation::reserve(if *map { 3 } else { 2 })?;
                    let items = match (&iterable, map) {
                        (Value::Array(values), false) => {
                            values.iter().cloned().map(|value| (value, None)).collect()
                        }
                        (Value::Proplist(entries), true) => entries
                            .iter()
                            .map(|(key, value)| (key.clone(), Some(value.clone())))
                            .collect(),
                        (other, _) => {
                            return Err(RuntimeError::new(format!(
                                "for: {} expected, but got {}!",
                                if *map { "map" } else { "array" },
                                other.type_name()
                            )))
                        }
                    };
                    iterators.push(CompiledIterator {
                        iterable,
                        items,
                        index: 0,
                        sweep_cursor: object_reference_sweep_cursor(),
                        value_stack,
                    });
                }
                CompiledInstruction::IteratorNext {
                    slot,
                    value_slot,
                    end,
                } => {
                    let iterator = iterators
                        .last_mut()
                        .ok_or_else(|| RuntimeError::new("internal compiled iterator missing"))?;
                    iterator.apply_removals();
                    let Some((item, value)) = iterator.items.get(iterator.index) else {
                        instruction = *end;
                        continue;
                    };
                    self.binding(*slot, &mut bindings, vm, env)?
                        .write_tracked(TrackedValue::runtime(item.clone()))?;
                    if let (Some(slot), Some(value)) = (value_slot, value) {
                        self.binding(*slot, &mut bindings, vm, env)?
                            .write_tracked(TrackedValue::runtime(value.clone()))?;
                    }
                    iterator.index += 1;
                }
                CompiledInstruction::IteratorEnd => {
                    iterators
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled iterator missing"))?;
                }
                CompiledInstruction::Finish => {
                    if let Some(diagnostic) = &mut direct_diagnostic {
                        diagnostic.returned(&Value::Nil);
                    }
                    return Ok(ControlFlow::Normal);
                }
            }
            instruction += 1;
        }
    }
}

impl CompiledContinuationFrame {
    fn resume(self, vm: &Vm<'_>) -> Result<ContinuationResult, RuntimeError> {
        self.resume_with_value(vm, None)
    }

    fn resume_with_value(
        self,
        vm: &Vm<'_>,
        resume_value: Option<Value>,
    ) -> Result<ContinuationResult, RuntimeError> {
        let CompiledContinuationFrame {
            function,
            compiled,
            call_targets,
            mut env,
            depth,
            caller,
            returns_reference,
            instruction,
            stack,
            registered_slots,
            assignment_targets,
            mut iterators,
            mut stack_value_stack,
            frame_value_stack,
            pending,
        } = self;
        debug_assert_eq!(returns_reference, function.returns_reference);
        stack_value_stack.attach()?;
        for iterator in &mut iterators {
            iterator.value_stack.attach()?;
        }
        // The original invocation's parameter/function-var reservation is
        // dropped while the continuation is owned by the host. Reacquire the
        // exact frame prefix for this execution slice so nested calls observe
        // the same stack budget they saw before the boundary.
        let _frame_value_stack = ValueStackReservation::reserve(frame_value_stack)?;
        let _object_reference_cells = ActiveObjectReferenceCellsGuard::enter_frame();
        _object_reference_cells.register_environment(&env, vm);
        let bindings = compiled.bindings(vm, &env);
        let state = CompiledExecutionState {
            instruction,
            stack,
            registered_slots,
            assignment_targets,
            iterators,
            stack_value_stack,
            pending: Some(pending),
            resume_value,
        };
        match compiled.execute_state(
            vm,
            &mut env,
            depth,
            function.as_ref(),
            caller,
            Arc::clone(&compiled),
            bindings,
            call_targets,
            state,
            frame_value_stack,
        ) {
            Ok(ControlFlow::Return(value)) => Ok(ContinuationResult::Complete(value)),
            Ok(ControlFlow::Normal) => Ok(ContinuationResult::Complete(ReturnValue::Value(
                TrackedValue::runtime(Value::Nil),
            ))),
            Err(error) => match Vm::script_call_outcome_from_error(vm, error)? {
                ScriptCallOutcome::Complete(value) => Ok(ContinuationResult::Complete(
                    ReturnValue::Value(TrackedValue::runtime(value)),
                )),
                ScriptCallOutcome::Suspended(suspension) => {
                    Ok(ContinuationResult::Suspended(suspension))
                }
            },
        }
    }
}

fn collect_function_var_names(body: &[Stmt], names: &mut Vec<String>) {
    for statement in body {
        match statement {
            Stmt::VarDecl { name, .. } => names.push(name.clone()),
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_function_var_names(then_branch, names);
                if let Some(else_branch) = else_branch {
                    collect_function_var_names(else_branch, names);
                }
            }
            Stmt::While { body, .. } => collect_function_var_names(body, names),
            Stmt::For { init, body, .. } => {
                if let Some(ForInit::VarDecls(declarations)) = init {
                    names.extend(declarations.iter().map(|(name, _)| name.clone()));
                }
                collect_function_var_names(body, names);
            }
            Stmt::ForIn {
                variable,
                value_variable,
                body,
                ..
            } => {
                names.push(variable.clone());
                names.extend(value_variable.iter().cloned());
                collect_function_var_names(body, names);
            }
            Stmt::Block(inner) | Stmt::Sequence(inner) => {
                collect_function_var_names(inner, names);
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
struct Environment {
    scopes: SmallVec<[FxHashMap<String, Binding>; 2]>,
    named_parameters: SmallVec<[(String, Binding); 4]>,
    /// Per-invocation storage for `Func->VarNamed`/`cthr->Vars`. A separate
    /// table is required because parameters win bare-name lookup while VarN
    /// can still address a same-name function variable.
    frame_locals: FrameLocalMap,
    /// `#strict` level of the executing function, for level-correct `==`/`!=`.
    strict_level: Option<u8>,
    /// `cthr->Caller->Func->Owner->Strict` for native calls. Linked function
    /// bodies keep source strictness above but are owned by the destination
    /// script, whose strictness can differ.
    caller_owner_strict_level: Option<u8>,
    caller_host_identity: ScriptHostIdentity,
    /// C4Script numeric scratch slots, addressed by `Var(n)` / `Local(n)`. These
    /// are SEPARATE from named variables (C++ `NumVars` and the object `Local`
    /// array, not `Vars`/`LocalNamed`) and are function-scoped, not block-scoped,
    /// so a `Local(0) = x` inside a block stays visible after it. Unset reads as
    /// nil and the index is clamped to >= 0 (C4ValueList::GetItem). `var_slots`
    /// are per-call; `local_slots` round-trip through the object's `local_vars`.
    object_state: ObjectState,
    /// The full argument slots of the executing call: `Par(i)` reads them
    /// (C4AulExec.cpp:1127-1140) and `Callee(...)` forwards the slots past
    /// `named_param_count` (C4AulParse.cpp:2293-2306, ParNamed.iSize).
    call_args: CallBindings,
    named_param_count: usize,
    /// The function the executing one overloaded — the `inherited(...)` /
    /// `_inherited(...)` target (C++ Fn->OwnerOverloaded,
    /// C4AulParse.cpp:2775-2798).
    inherited_target: Option<std::sync::Arc<Function>>,
    /// The executing function's name — the `inherited` fallback to the
    /// same-name ENGINE function when no script overload exists
    /// (C4Aul: script functions overload engine functions; OwnerOverloaded
    /// chains to the C4AulFunc base).
    function_name: String,
    /// C4Aul global functions are owned by Game.ScriptEngine; unqualified
    /// calls inside them resolve in engine scope, not against `this`'s def.
    engine_scope: bool,
    /// DirectExec/eval expression frames are backed by temporary scripts;
    /// ordinary function invocation leaves this false.
    temporary_script: bool,
    direct_exec_context: Option<DirectExecContinuationContext>,
    /// Dynamic `cthr->Def` presence. Unlike the VM's owning-script identity,
    /// this is cleared by a nil-object DirectExec and `global->`.
    definition_context: bool,
    global_call_context: bool,
}

impl Environment {
    fn new_with_params(
        params: &[Parameter],
        args: &[CallArg],
        strict_level: Option<u8>,
        object_state: ObjectState,
    ) -> Result<Self, RuntimeError> {
        let mut call_args = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                if index >= params.len() {
                    return arg
                        .read_tracked()
                        .map(InlineBinding::new)
                        .map(Binding::Inline);
                }
                match arg {
                    CallArg::Reference(reference) if params[index].is_reference => {
                        Ok(Binding::Reference(reference.clone()))
                    }
                    _ => Ok(Binding::tracked(arg.read_tracked()?)),
                }
            })
            .collect::<Result<CallBindings, RuntimeError>>()?;
        while call_args.len() < MAX_CALL_PARAMETERS {
            call_args.push(Binding::direct(Value::Nil));
        }
        #[cfg(test)]
        record_call_arg_heap_spill(call_args.spilled());
        let mut scopes: SmallVec<[FxHashMap<String, Binding>; 2]> = SmallVec::new();
        scopes.push(FxHashMap::default());
        let mut named_parameters = SmallVec::<[(String, Binding); 4]>::new();
        for (param, binding) in params.iter().zip(call_args.iter()) {
            if let Some((_, current)) = named_parameters
                .iter_mut()
                .find(|(name, _)| name == &param.name)
            {
                *current = binding.clone();
            } else {
                named_parameters.push((param.name.clone(), binding.clone()));
            }
        }
        Ok(Self {
            scopes,
            named_parameters,
            frame_locals: Rc::new(FrameLocals::default()),
            strict_level,
            caller_owner_strict_level: strict_level,
            // invoke_script_function stamps the owning VM before executing the
            // body; zero is only the construction sentinel.
            caller_host_identity: ScriptHostIdentity(0),
            object_state,
            call_args,
            named_param_count: params.len(),
            inherited_target: None,
            function_name: String::new(),
            engine_scope: false,
            temporary_script: false,
            direct_exec_context: None,
            definition_context: false,
            global_call_context: false,
        })
    }

    fn caller_context(&self) -> ScriptCallerContext {
        ScriptCallerContext {
            frame_locals: self.frame_locals.clone(),
            owner_host: self.caller_host_identity,
            engine_scope: self.engine_scope,
            definition_context: self.definition_context,
            owner_strict_level: self.caller_owner_strict_level,
            origin_strict_level: self.strict_level,
            temporary_script: self.temporary_script,
        }
    }

    fn object_reference_cells(&self, vm: &Vm<'_>) -> Vec<Weak<RefCell<Value>>> {
        let mut cells = Vec::new();
        for binding in &self.call_args {
            binding.collect_object_reference_cells(&mut cells);
        }
        for binding in self.frame_locals.function_vars.borrow().values() {
            binding.collect_object_reference_cells(&mut cells);
        }
        for scope in &self.scopes {
            for binding in scope.values() {
                binding.collect_object_reference_cells(&mut cells);
            }
        }
        for (_, binding) in &self.named_parameters {
            binding.collect_object_reference_cells(&mut cells);
        }
        let depth = ACTIVE_OBJECT_REFERENCE_DEPTH.with(Cell::get);
        let scan_object_state = ACTIVE_OBJECT_REFERENCE_TABLES.with(|tables| {
            tables
                .borrow_mut()
                .as_mut()
                .expect("the reference guard installs its table registry first")
                .register_object_state(&self.object_state, depth)
        });
        if scan_object_state {
            #[cfg(test)]
            OBJECT_REFERENCE_TABLE_TRAVERSALS.with(|count| count.set(count.get() + 2));
            let named_locals = self.object_state.named_locals.borrow();
            register_shared_object_reference_cells(named_locals.values());
            let local_slots = self.object_state.local_slots.borrow();
            register_shared_object_reference_cells(local_slots.values());
        }
        if let Some(globals) = vm.globals_named {
            #[cfg(test)]
            OBJECT_REFERENCE_TABLE_TRAVERSALS.with(|count| count.set(count.get() + 1));
            let globals = globals.borrow();
            register_shared_object_reference_cells(globals.values());
        }
        if let Some(globals) = vm.globals_numbered {
            #[cfg(test)]
            OBJECT_REFERENCE_TABLE_TRAVERSALS.with(|count| count.set(count.get() + 1));
            let globals = globals.borrow();
            register_shared_object_reference_cells(globals.values());
        }
        if let Some(globals) = vm.globals_consts {
            #[cfg(test)]
            OBJECT_REFERENCE_TABLE_TRAVERSALS.with(|count| count.set(count.get() + 1));
            let globals = globals.borrow();
            register_shared_object_reference_cells(globals.values());
        }
        cells
    }

    /// Clear references held by this suspended frame before the embedding
    /// engine removes an object. The active-reference registry is scoped to
    /// one running call and is intentionally empty while a continuation is
    /// owned by the host; walking the owned cells here keeps AssignRemoval
    /// synchronous without storing an aliased VM pointer or TLS guard.
    fn clear_object_reference(&mut self, object_id: u64) {
        for binding in &mut self.call_args {
            binding.clear_object_reference(object_id);
        }
        for binding in self.frame_locals.function_vars.borrow_mut().values_mut() {
            binding.clear_object_reference(object_id);
        }
        for scope in &mut self.scopes {
            for binding in scope.values_mut() {
                binding.clear_object_reference(object_id);
            }
        }
        for (_, binding) in &mut self.named_parameters {
            binding.clear_object_reference(object_id);
        }
        self.object_state.clear_object_reference(object_id);
    }

    fn define_object_local(&mut self, name: &str, identity: RawIdentityCell) {
        let cell = self.object_state.named_local_cell(name);
        if self.scopes.iter().any(|scope| scope.contains_key(name))
            || self
                .named_parameters
                .iter()
                .any(|(parameter, _)| parameter == name)
        {
            return;
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name.to_string(),
                Binding::Direct {
                    value: cell,
                    identity,
                },
            );
        }
    }

    fn define(&mut self, name: &str, value: Value) {
        self.define_tracked(name, TrackedValue::runtime(value));
    }

    fn define_tracked(&mut self, name: &str, tracked: TrackedValue) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), Binding::tracked(tracked));
        }
    }

    /// Pre-declare a hoisted `Func->VarNamed` slot. Bare-name lookup reuses
    /// that binding unless a parameter already owns the name; VarN still sees
    /// the distinct function-var slot in the collision case.
    fn declare_hoisted(&mut self, name: &str) {
        let binding = self
            .frame_locals
            .function_vars
            .borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| Binding::direct(Value::Nil))
            .clone();
        if !self.scopes.iter().any(|scope| scope.contains_key(name))
            && !self
                .named_parameters
                .iter()
                .any(|(parameter, _)| parameter == name)
        {
            self.scopes
                .first_mut()
                .expect("environment has a base scope")
                .insert(name.to_string(), binding);
        }
    }

    fn function_var_lvalue(&self, name: &str) -> Option<LValueRef> {
        self.frame_locals
            .function_vars
            .borrow()
            .get(name)
            .map(Binding::lvalue)
    }

    fn get(&self, name: &str) -> Result<Option<Value>, RuntimeError> {
        self.get_tracked(name)
            .map(|tracked| tracked.map(|tracked| tracked.value))
    }

    fn get_tracked(&self, name: &str) -> Result<Option<TrackedValue>, RuntimeError> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return value.read_tracked().map(Some);
            }
        }
        self.named_parameters
            .iter()
            .rev()
            .find(|(parameter, _)| parameter == name)
            .map(|(_, value)| value.read_tracked())
            .transpose()
    }

    fn binding(&self, name: &str) -> Option<Binding> {
        lookup_profile::record(lookup_profile::LookupFamily::Local, name);
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
            .or_else(|| {
                self.named_parameters
                    .iter()
                    .rev()
                    .find(|(parameter, _)| parameter == name)
                    .map(|(_, binding)| binding.clone())
            })
    }

    fn function_var_binding(&self, name: &str) -> Option<Binding> {
        lookup_profile::record(lookup_profile::LookupFamily::Local, name);
        self.frame_locals.function_vars.borrow().get(name).cloned()
    }

    fn lvalue(&self, name: &str) -> Option<LValueRef> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value.lvalue());
            }
        }
        self.named_parameters
            .iter()
            .rev()
            .find(|(parameter, _)| parameter == name)
            .map(|(_, binding)| binding.lvalue())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! check_eq {
        ($left:expr => $right:expr) => {
            assert_eq!($left, $right)
        };
        ($left:expr => $right:expr, $($message:tt)+) => {
            assert_eq!($left, $right, $($message)+)
        };
    }

    macro_rules! check {
        ($condition:expr) => {
            assert!($condition)
        };
        ($condition:expr, $($message:tt)+) => {
            assert!($condition, $($message)+)
        };
    }

    /// Opaque host-continuation request for the suspension probes below.
    #[derive(Debug)]
    struct PauseProbeRequest;

    macro_rules! check_script {
        ($source:expr, $entry:expr, $args:expr; unwrap => $expected:expr) => {
            check_eq!(execute_script($source, $entry, $args).unwrap() => $expected);
        };
        ($source:expr, $entry:expr, $args:expr; expect $message:expr => $expected:expr) => {
            check_eq!(execute_script($source, $entry, $args).expect($message) => $expected);
        };
    }

    #[test]
    fn active_object_reference_collection_keeps_only_weak_owners() {
        let binding = Binding::tracked(TrackedValue::runtime(Value::Object(7)));
        let cell = match &binding {
            Binding::Direct { value, .. } => Rc::clone(value),
            Binding::Inline(_) | Binding::Reference(_) => {
                unreachable!("tracked bindings own a direct value cell")
            }
        };
        let strong_owners = Rc::strong_count(&cell);
        let mut cells = Vec::new();

        binding.collect_object_reference_cells(&mut cells);

        assert_eq!(cells.len(), 1);
        assert_eq!(
            Rc::strong_count(&cell),
            strong_owners,
            "the removal guard must not clone strong owners before downgrading them"
        );
    }

    #[test]
    fn active_object_reference_sweep_visits_only_cells_for_removed_object() {
        // C++ clears only the removed object's intrusive FirstRef list
        // (C4Object.cpp:312), independent of unrelated live C4Values.
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let target = value_cell(Value::Nil);
        LValueRef::cell(Rc::clone(&target))
            .write(Value::Object(7))
            .expect("target assignment succeeds");
        let _unrelated = (0..128)
            .map(|value| value_cell(Value::Int(value)))
            .collect::<Vec<_>>();
        reset_active_object_reference_sweep_visits();

        clear_active_object_references(7);

        check_eq!(*target.borrow() => Value::Nil);
        check_eq!(active_object_reference_sweep_visits() => 1);
    }

    #[test]
    fn active_object_reference_index_replaces_and_prunes_memberships() {
        // C++ moves a C4Value between intrusive FirstRef lists on assignment
        // and unlinks it on destruction (C4Value.cpp:104-140). Overwrites and
        // completed nested frames therefore cannot leave old object IDs or
        // one dead link per call in the active index.
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let survivor = value_cell(Value::Object(7));
        LValueRef::cell(Rc::clone(&survivor))
            .write(Value::Object(9))
            .expect("overwrite succeeds");
        reset_active_object_reference_sweep_visits();

        clear_active_object_references(7);

        check_eq!(active_object_reference_sweep_visits() => 0);
        check_eq!(*survivor.borrow() => Value::Object(9));

        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
            let finished_frame_cells = (0..64)
                .map(|_| value_cell(Value::Object(11)))
                .collect::<Vec<_>>();
            check_eq!(finished_frame_cells.len() => 64);
        }
        let _live = value_cell(Value::Object(12));
        let stale_links = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            index
                .borrow()
                .as_ref()
                .and_then(|index| index.cells_by_object.get(&11))
                .map_or(0, FxHashMap::len)
        });

        check_eq!(stale_links => 0);
    }

    #[test]
    fn nested_call_temporaries_do_not_accumulate_in_outer_frame_bookkeeping() {
        // A script call constructs its parameter cells before it begins
        // executing its body. Those short-lived cells belong to the callee's
        // frame, even while a long-running caller remains active. C++ unlinks
        // the corresponding C4Value from its intrusive object list when that
        // temporary is destroyed (C4Value.cpp:104-140).
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("ActiveFrameAddressCount", |_| {
            let count = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
                index
                    .borrow()
                    .as_ref()
                    .and_then(|index| index.frame_addresses.last())
                    .map_or(0, FxHashSet::len)
            });
            Ok(Value::Int(count as i32))
        });
        engine.register_host_function("ObjectValue", |_| Ok(Value::Object(7)));
        engine
            .load_script(
                "
                    func Leaf(value) { return value; }
                    func Outer() {
                        var index = 0;
                        var before = ActiveFrameAddressCount();
                        while (index < 512) {
                            Leaf(ObjectValue());
                            ++index;
                        }
                        return ActiveFrameAddressCount() - before;
                    }
                ",
            )
            .expect("script loads");

        check_eq!(engine.call("Outer", &[]).expect("outer call succeeds") => Value::Int(0));
    }

    #[test]
    fn repeated_eval_temporaries_do_not_accumulate_in_outer_frame_bookkeeping() {
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("ActiveFrameAddressCount", |_| {
            let count = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
                index
                    .borrow()
                    .as_ref()
                    .and_then(|index| index.frame_addresses.last())
                    .map_or(0, FxHashSet::len)
            });
            Ok(Value::Int(count as i32))
        });
        engine.register_host_function("ObjectValue", |_| Ok(Value::Object(7)));
        engine
            .load_script(
                r#"
                    func Outer() {
                        var index = 0;
                        var before = ActiveFrameAddressCount();
                        while (index < 512) {
                            eval("Var(0) = ObjectValue()");
                            ++index;
                        }
                        return ActiveFrameAddressCount() - before;
                    }
                "#,
            )
            .expect("script loads");

        check_eq!(engine.call("Outer", &[]).expect("outer call succeeds") => Value::Int(0));
    }

    #[test]
    fn shared_cell_tracked_by_an_ancestor_is_not_queued_for_nested_pruning() {
        let functions = FxHashMap::default();
        let globals = crate::engine::new_global_variables();
        globals
            .borrow_mut()
            .insert("shared".to_owned(), value_cell(Value::Object(7)));
        let vm = test_vm(&functions, &[]).with_global_variables(Some(&globals));
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);

        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }
        let (frame_addresses, pending) = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            let index = index.borrow();
            let index = index.as_ref().expect("outer guard keeps the index active");
            (
                index
                    .frame_addresses
                    .iter()
                    .map(FxHashSet::len)
                    .sum::<usize>(),
                index.pending_prune_count(),
            )
        });

        check_eq!(frame_addresses => 1);
        check_eq!(pending => 0);
    }

    #[test]
    fn path_write_updates_one_subtree_without_rescanning_its_root() {
        // AB_SET writes through the addressed C4Value, not every sibling in
        // its owning array (C4AulExec.cpp:858-865). Reverse-index maintenance
        // must keep that property; a whole-root scan restores the quadratic
        // element-assignment cost fixed in clonk-org/clonk-rs#759.
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let root = value_cell(Value::Array(
            std::iter::once(Value::Object(7))
                .chain((1..128).map(Value::Int))
                .collect(),
        ));
        let element = LValueRef::cell(Rc::clone(&root))
            .append(PathSegment::Index(Value::Int(0)))
            .expect("array element is an lvalue");
        reset_object_reference_index_value_visits();

        element
            .write(Value::Nil)
            .expect("element overwrite succeeds");

        check!(
            object_reference_index_value_visits() <= 2,
            "only the replaced and replacement values should be indexed"
        );
        reset_active_object_reference_sweep_visits();
        clear_active_object_references(7);
        check_eq!(active_object_reference_sweep_visits() => 0);
    }

    #[test]
    fn path_write_counts_duplicate_object_references_in_sibling_values() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let root = value_cell(Value::Array(vec![Value::Object(7), Value::Object(7)]));
        let first = LValueRef::cell(Rc::clone(&root))
            .append(PathSegment::Index(Value::Int(0)))
            .expect("first array element is an lvalue");

        first.write(Value::Nil).expect("element overwrite succeeds");
        reset_active_object_reference_sweep_visits();
        clear_active_object_references(7);

        check_eq!(active_object_reference_sweep_visits() => 1);
        check_eq!(*root.borrow() => Value::Array(vec![Value::Nil, Value::Nil]));
    }

    #[test]
    fn map_path_writes_update_key_and_recycled_slot_counts() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);

        let mut keyed = ValueMap::new();
        keyed.insert_key(Value::Object(7), Value::Int(1));
        let keyed = value_cell(Value::Proplist(keyed));
        LValueRef::cell(Rc::clone(&keyed))
            .append(PathSegment::Index(Value::Object(7)))
            .expect("object-keyed entry is an lvalue")
            .write(Value::Nil)
            .expect("nil removes the keyed entry");

        let mut recycled = ValueMap::new();
        recycled.recycle_value_slot(Value::Object(9));
        let recycled = value_cell(Value::Proplist(recycled));
        LValueRef::cell(Rc::clone(&recycled))
            .append(PathSegment::Property("fresh".to_owned()))
            .expect("map property is an lvalue")
            .write(Value::Int(1))
            .expect("new property reuses the retained slot");

        reset_active_object_reference_sweep_visits();
        clear_active_object_references(7);
        clear_active_object_references(9);

        check_eq!(active_object_reference_sweep_visits() => 0);
    }

    #[test]
    fn map_path_removal_does_not_reindex_an_equal_successor() {
        // AssignRemoval can make two keys in one native hash bucket compare
        // equal. Removing the first node must not count the already-indexed
        // successor again (C4ValueHash.cpp:49-75,117-136).
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);

        let mut collapsing = ValueMap::new();
        collapsing.insert_key(Value::Array(vec![Value::Bool(true)]), Value::Object(7));
        collapsing.insert_key(Value::Array(vec![Value::RawBool(2)]), Value::Object(7));
        let empty = ValueMap::new();
        let mut outer = ValueMap::new();
        outer.insert_key(Value::Proplist(collapsing), Value::Object(9));
        outer.insert_key(Value::Proplist(empty.clone()), Value::Object(9));
        let root = value_cell(Value::Proplist(outer));

        clear_active_object_references(7);
        for _ in 0..2 {
            LValueRef::cell(Rc::clone(&root))
                .append(PathSegment::Index(Value::Proplist(empty.clone())))
                .expect("equal map key remains addressable")
                .write(Value::Nil)
                .expect("nil removes one equal entry");
        }

        reset_active_object_reference_sweep_visits();
        clear_active_object_references(9);

        check_eq!(active_object_reference_sweep_visits() => 0);
    }

    #[test]
    fn map_node_removal_unlinks_other_object_references_from_the_swept_cell() {
        // Removing a directly object-valued map entry destroys that node's
        // key C4Value too, which unlinks it from its own FirstRef list
        // (C4Value.cpp:78-99; C4Object.cpp:312).
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let mut map = ValueMap::new();
        map.insert_key(Value::Object(9), Value::Object(7));
        let root = value_cell(Value::Proplist(map));
        let address = Rc::as_ptr(&root) as usize;
        reset_active_object_reference_sweep_visits();

        clear_active_object_references(7);

        check_eq!(active_object_reference_sweep_visits() => 1);
        let (seven_bucket, nine_bucket, has_membership) =
            ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
                let index = index.borrow();
                let index = index.as_ref().expect("the guard installs the index");
                (
                    index.cells_by_object.contains_key(&7),
                    index.cells_by_object.contains_key(&9),
                    index.memberships_by_cell.contains_key(&address),
                )
            });
        check!(!seven_bucket);
        check!(!nine_bucket);
        check!(!has_membership);

        reset_active_object_reference_sweep_visits();
        clear_active_object_references(9);

        check_eq!(active_object_reference_sweep_visits() => 0);
        let root = root.borrow();
        let Value::Proplist(map) = &*root else {
            panic!("the swept cell remains a map");
        };
        check_eq!(map.len() => 0);
        check_eq!(map.hidden_values().cloned().collect::<Vec<_>>() => vec![Value::Nil]);
    }

    #[test]
    fn zero_object_does_not_create_a_reverse_index_bucket() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);

        let _zero = value_cell(Value::Object(0));
        let has_zero_bucket = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            index
                .borrow()
                .as_ref()
                .is_some_and(|index| index.cells_by_object.contains_key(&0))
        });

        check!(!has_zero_bucket);
    }

    #[test]
    fn escaped_cell_remains_pending_while_its_owner_is_alive() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let escaped;
        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
            escaped = value_cell(Value::Object(7));
        }
        reset_object_reference_pending_prune_visits();

        {
            let _next = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }
        let pending = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            index
                .borrow()
                .as_ref()
                .map_or(0, ActiveObjectReferenceIndex::pending_prune_count)
        });

        check_eq!(object_reference_pending_prune_visits() => 1);
        check_eq!(pending => 1);
        check_eq!(*escaped.borrow() => Value::Object(7));
    }

    #[test]
    fn released_escaped_cell_is_removed_from_reverse_buckets_and_frame_sets() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let retained;
        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
            retained = value_cell(Value::Object(7));
        }
        {
            let _recheck = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }
        drop(retained);
        {
            let _cleanup = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }
        let (bucket_cells, memberships, frame_addresses, pending) = ACTIVE_OBJECT_REFERENCE_INDEX
            .with(|index| {
                let index = index.borrow();
                let index = index.as_ref().expect("outer guard keeps the index active");
                (
                    index.cells_by_object.get(&7).map_or(0, FxHashMap::len),
                    index.memberships_by_cell.len(),
                    index
                        .frame_addresses
                        .iter()
                        .map(FxHashSet::len)
                        .sum::<usize>(),
                    index.pending_prune_count(),
                )
            });

        check_eq!(bucket_cells => 0);
        check_eq!(memberships => 0);
        check_eq!(frame_addresses => 0);
        check_eq!(pending => 0);
    }

    #[test]
    fn pending_pruning_checks_one_escaped_cell_per_removal_and_drains_after_release() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let mut retained = Vec::new();
        for object_id in 1..=64 {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
            retained.push(value_cell(Value::Object(object_id)));
        }
        reset_object_reference_pending_prune_visits();

        clear_active_object_references(999);

        check!(
            object_reference_pending_prune_visits() <= 1,
            "one removal must not scan every live escaped reference"
        );
        drop(retained);
        for _ in 0..128 {
            let _cleanup = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }
        let (buckets, memberships, frame_addresses, pending) =
            ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
                let index = index.borrow();
                let index = index.as_ref().expect("outer guard keeps the index active");
                (
                    index.cells_by_object.len(),
                    index.memberships_by_cell.len(),
                    index
                        .frame_addresses
                        .iter()
                        .map(FxHashSet::len)
                        .sum::<usize>(),
                    index.pending_prune_count(),
                )
            });

        check_eq!(buckets => 0);
        check_eq!(memberships => 0);
        check_eq!(frame_addresses => 0);
        check_eq!(pending => 0);
    }

    #[test]
    fn sustained_new_escapes_do_not_starve_an_older_released_cell() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let released;
        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
            released = value_cell(Value::Object(1));
        }
        let mut retained_arrivals = Vec::new();
        {
            // Recheck the older cell while it is live, then add the first of
            // a sustained stream before releasing the older owner.
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
            retained_arrivals.push(value_cell(Value::Object(2)));
        }
        drop(released);
        for object_id in 3..=66 {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
            retained_arrivals.push(value_cell(Value::Object(object_id)));
        }
        let older_bucket = ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            index
                .borrow()
                .as_ref()
                .and_then(|index| index.cells_by_object.get(&1))
                .map_or(0, FxHashMap::len)
        });

        check_eq!(older_bucket => 0);
        check_eq!(retained_arrivals.len() => 65);
    }

    #[test]
    fn nested_calls_scan_shared_object_reference_state_once() {
        // C++ attaches every live C4Value to one process-global intrusive
        // FirstRef list, so AB_CALL only adds its frame values
        // (C4AulExec.cpp:62-63,1217-1223; C4Object.cpp:312). Rust can retain
        // the private object-state scan, while public mutable global tables
        // must still be checked at each nested frame.
        let script = parse_script(
            "static persisted; local target; func Leaf() { return target; } func Probe() { return Leaf(); }",
            "script parses",
        );
        let functions = function_map(script.clone());
        let globals = crate::engine::new_global_variables();
        crate::engine::register_global_declarations(&script.var_decls, &globals, None)
            .expect("static declaration registers");
        reset_object_reference_table_traversals();

        let result = test_vm(&functions, &script.var_decls)
            .with_global_variables(Some(&globals))
            .call_with_locals(
                "Probe",
                &[],
                &HashMap::from([("target".to_owned(), Value::Object(7))]),
            )
            .expect("nested call succeeds")
            .0;

        check_eq!(result => Value::Object(7));
        check_eq!(object_reference_table_traversals() => 4);
    }

    #[test]
    fn shared_reference_discovery_borrows_the_index_once_per_batch() {
        // AB_CALL must preserve every existing object's FirstRef link
        // (C4AulExec.cpp:1217-1223; C4Object.cpp:312). Discovering a shared
        // table should acquire its thread-local index once, independently
        // of the number of cells that table contains.
        let cells = (0..128)
            .map(|_| value_cell(Value::Object(7)))
            .collect::<Vec<_>>();
        let _guard = ActiveObjectReferenceCellsGuard::enter_frame();
        reset_object_reference_discovery_borrows();

        register_shared_object_reference_cells(cells.iter());

        check_eq!(object_reference_discovery_borrows() => 1);
        clear_active_object_references(7);
        check!(cells.iter().all(|cell| *cell.borrow() == Value::Nil));
    }

    #[test]
    fn scalar_globals_do_not_walk_reference_values_on_nested_calls() {
        // C++ AB_CALL adds the callee's frame; integer globals have no
        // intrusive FirstRef membership to rebuild (C4AulExec.cpp:1217-1223;
        // C4Value.cpp:104-140). Their later object assignments must still
        // participate in synchronous AssignRemoval (C4Object.cpp:312).
        let functions = FxHashMap::default();
        let globals = crate::engine::new_global_variables();
        for index in 0..128 {
            globals
                .borrow_mut()
                .insert(format!("scalar{index}"), value_cell(Value::Int(index)));
        }
        let target = value_cell(Value::Nil);
        globals
            .borrow_mut()
            .insert("target".to_owned(), Rc::clone(&target));
        let vm = test_vm(&functions, &[]).with_global_variables(Some(&globals));
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        reset_object_reference_index_value_visits();

        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }

        check_eq!(object_reference_index_value_visits() => 0);
        set_value_cell(&target, Value::Object(7));
        clear_active_object_references(7);
        check_eq!(*target.borrow() => Value::Nil);
    }

    #[test]
    fn nested_frame_does_not_reindex_an_unchanged_shared_global_cell() {
        // C++ links an existing C4Value into FirstRef once; entering AB_CALL
        // does not recursively revisit the value (C4Value.cpp:104-140;
        // C4AulExec.cpp:1217-1223). Keep counting the recursive value walk,
        // rather than table enumeration, so a relink hidden behind the same
        // global-table traversal remains visible.
        let functions = FxHashMap::default();
        let globals = crate::engine::new_global_variables();
        let shared = value_cell(Value::Array(
            std::iter::once(Value::Object(7))
                .chain((0..128).map(Value::Int))
                .collect(),
        ));
        globals
            .borrow_mut()
            .insert("shared".to_owned(), Rc::clone(&shared));
        let vm = test_vm(&functions, &[]).with_global_variables(Some(&globals));
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        reset_object_reference_index_value_visits();

        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }

        check!(
            object_reference_index_value_visits() <= MAX_CALL_PARAMETERS,
            "only the object-free fixed call slots may need rediscovery; the unchanged 129-value global must not be recursively reindexed"
        );
        check_eq!(*shared.borrow() => Value::Array(
            std::iter::once(Value::Object(7))
                .chain((0..128).map(Value::Int))
                .collect()
        ));

        set_value_cell(
            &shared,
            Value::Array(
                std::iter::once(Value::Object(9))
                    .chain((0..128).map(Value::Int))
                    .collect(),
            ),
        );
        reset_object_reference_index_value_visits();
        {
            let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        }
        check!(
            object_reference_index_value_visits() <= MAX_CALL_PARAMETERS,
            "set_value_cell must refresh once, then leave the unchanged global eligible for O(1) discovery"
        );
        clear_active_object_references(7);
        check!(
            matches!(&*shared.borrow(), Value::Array(values) if matches!(values.first(), Some(Value::Object(9))))
        );
        clear_active_object_references(9);
        check!(
            matches!(&*shared.borrow(), Value::Array(values) if matches!(values.first(), Some(Value::Nil)))
        );
    }

    #[test]
    fn cell_discovery_rebuilds_a_stale_weak_registration_at_the_same_address_key() {
        let functions = FxHashMap::default();
        let vm = test_vm(&functions, &[]);
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _guard = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        let departed = Rc::new(RefCell::new(Value::Object(7)));
        let stale = Rc::downgrade(&departed);
        drop(departed);
        let current = Rc::new(RefCell::new(Value::Object(9)));
        let address = Rc::as_ptr(&current) as usize;
        ACTIVE_OBJECT_REFERENCE_INDEX.with(|index| {
            let mut index = index.borrow_mut();
            let index = index.as_mut().expect("the guard installs the index");
            index
                .cells_by_object
                .entry(7)
                .or_default()
                .insert(address, stale.clone());
            index.memberships_by_cell.insert(
                address,
                ActiveObjectReferenceMembership {
                    cell: stale,
                    object_counts: FxHashMap::from_iter([(7, 1)]),
                },
            );
        });
        reset_object_reference_index_value_visits();

        ensure_active_object_reference_cell_registered(&current);

        check_eq!(object_reference_index_value_visits() => 1);
        clear_active_object_references(7);
        check_eq!(*current.borrow() => Value::Object(9));
        clear_active_object_references(9);
        check_eq!(*current.borrow() => Value::Nil);
    }

    #[test]
    fn nested_call_tracks_preexisting_cell_replacing_shared_global_entry() {
        // Every live C4Value remains on C++'s process-global FirstRef list even
        // when a table starts owning it between calls (C4AulExec.cpp:62-63,
        // 1217-1223; C4Object.cpp:312).
        let functions = FxHashMap::default();
        let globals = crate::engine::new_global_variables();
        globals
            .borrow_mut()
            .insert("late".to_owned(), value_cell(Value::Nil));
        let replacement = value_cell(Value::Object(7));
        let vm = test_vm(&functions, &[]).with_global_variables(Some(&globals));
        let env = Environment::new_with_params(&[], &[], None, ObjectState::default())
            .expect("empty environment builds");
        let _outer = ActiveObjectReferenceCellsGuard::enter(&env, &vm);
        globals
            .borrow_mut()
            .insert("late".to_owned(), Rc::clone(&replacement));
        let _nested = ActiveObjectReferenceCellsGuard::enter(&env, &vm);

        clear_active_object_references(7);

        check_eq!(*replacement.borrow() => Value::Nil);
    }

    #[test]
    fn execution_profile_counts_scalar_and_foreach_calls() {
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "func Compiled() { return 42; }\n\
                 func Ast(values) { var total = 0; for (var value in values) total += value; return total; }",
            )
            .expect("profile script loads");
        crate::execution_profile::reset();

        check_eq!(engine.call("Compiled", &[]).expect("compiled call succeeds") => Value::Int(42));
        check_eq!(engine.call("Ast", &[Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])]).expect("AST call succeeds") => Value::Int(6));
        let profile = crate::execution_profile::snapshot();

        check_eq!(profile.compiled => 2);
    }

    #[cfg(feature = "execution-profile")]
    #[test]
    fn execution_timing_records_foreach_calls() {
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "func Ast(values) { var total = 0; for (var value in values) total += value; return total; }",
            )
            .expect("profile script loads");
        crate::execution_profile::reset();
        crate::execution_profile::set_timing_enabled(true);

        check_eq!(engine.call("Ast", &[Value::Array(vec![Value::Int(1), Value::Int(2)])]).expect("AST call succeeds") => Value::Int(3));
        let timing = crate::execution_profile::timing_snapshot();
        crate::execution_profile::set_timing_enabled(false);

        assert!(timing.compiled_ns > 0, "{timing:?}");
    }

    #[test]
    fn execution_profile_counts_loop_control_calls() {
        // Loop control lowers into while and classic-for, so the surviving
        // blocker in a foreach body is the foreach itself.
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "func Compiled() {\n\
                     var total = 0;\n\
                     for (var i = 0; i < 3; i++) {\n\
                         if (i == 1) continue;\n\
                         total += i;\n\
                     }\n\
                     return total;\n\
                 }\n\
                 func Ast(values) {\n\
                     var total = 0;\n\
                     for (var value in values) {\n\
                         if (value == 1) continue;\n\
                         total += value;\n\
                     }\n\
                     return total;\n\
                 }",
            )
            .expect("profile script loads");
        crate::execution_profile::reset();

        check_eq!(engine.call("Compiled", &[]).expect("compiled call succeeds") => Value::Int(2));
        check_eq!(engine.call("Ast", &[Value::Array(vec![Value::Int(0), Value::Int(1), Value::Int(2)])]).expect("AST call succeeds") => Value::Int(2));
        let profile = crate::execution_profile::snapshot();

        check_eq!(profile.compiled => 2);
    }

    #[test]
    fn classic_for_with_local_counter_uses_compiled_executor() {
        // C++ lowers the initializer, condition, body, increment and back edge
        // into the ordinary C4Aul bytecode stream (C4AulParse.cpp:2789-3088;
        // C4AulExec.cpp:330-1297).
        reset_compiled_function_execution_count();
        check_script!(
            "func Sum(count) { var total = 0; for (var i = 0; i < count; i++) total += i; return total; }",
            "Sum",
            &[Value::Int(5)];
            expect "classic for loop succeeds" => Value::Int(10)
        );
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_classic_for_preserves_clause_order_and_short_circuiting() {
        // C4Aul emits init once, then condition/body/increment in that order;
        // AB_JUMPAND skips the right condition operand once the left is false
        // (C4AulParse.cpp:2789-3088; C4AulExec.cpp:730-737).
        reset_compiled_function_execution_count();
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\n\
                 local trace;\n\
                 func Mark(value) { trace = trace * 10 + value; return value; }\n\
                 func Probe() {\n\
                     trace = 0; var i;\n\
                     for (i = Mark(1); i < 3 && Mark(2); i = Mark(i + 1)) Mark(3);\n\
                     return trace;\n\
                 }",
            )
            .expect("loop script loads");
        let result = engine
            .call_with_locals("Probe", &[], &HashMap::new())
            .expect("compiled loop preserves clause order")
            .0;

        check_eq!(result => Value::Int(1_232_233));
        check_eq!(compiled_function_execution_count() => 8);
    }

    #[test]
    fn compiled_classic_for_assignment_keeps_target_live_during_clause_rhs() {
        // The for initializer is emitted before its condition, and AB_Set keeps
        // its target reference live across the RHS AB_CALL
        // (C4AulParse.cpp:2789-3088; C4AulExec.cpp:404-415,1216-1297).
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let host_observed = std::sync::Arc::clone(&observed);
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("ObserveStack", move |_| {
            host_observed
                .lock()
                .expect("stack observation lock")
                .push(VALUE_STACK_SIZE.with(Cell::get));
            Ok(Value::Int(7))
        });
        engine
            .load_script(
                "func Compiled() {\n\
                     var value = 0;\n\
                     for (value = ObserveStack(); false;) {}\n\
                     return value;\n\
                 }\n\
                 func Interpreted() {\n\
                     var value = 0;\n\
                     if (false) return nil ?? 1;\n\
                     for (value = ObserveStack(); false;) {}\n\
                     return value;\n\
                 }",
            )
            .expect("classic-for assignment script loads");

        reset_compiled_function_execution_count();
        check_eq!(engine.call("Compiled", &[]).expect("compiled loop succeeds") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
        check_eq!(engine.call("Interpreted", &[]).expect("AST loop succeeds") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 2);
        let observed = observed.lock().expect("stack observation lock");

        check_eq!(observed.len() => 2);
        check_eq!(observed[0] => observed[1]);
    }

    #[test]
    fn local_assignment_expression_uses_compiled_executor() {
        // AB_Set leaves the assigned reference on C4AulExec::Values; the
        // surrounding arithmetic then dereferences its just-written value
        // (C4AulExec.cpp:404-415,490-593).
        reset_compiled_function_execution_count();
        check_script!(
            "func Probe() { var value = 0; return (value = 2) + (value = 3) * 10; }",
            "Probe",
            &[];
            expect "assignment expressions yield their stored values" => Value::Int(32)
        );
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_assignment_keeps_target_reference_live_during_rhs_call() {
        // AB_Set's target reference is already on C4AulExec::Values while the
        // RHS AB_CALL runs (C4AulExec.cpp:404-415,1216-1297).
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let host_observed = std::sync::Arc::clone(&observed);
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("ObserveStack", move |_| {
            host_observed
                .lock()
                .expect("stack observation lock")
                .push(VALUE_STACK_SIZE.with(Cell::get));
            Ok(Value::Int(7))
        });
        engine
            .load_script(
                "func Compiled() { var value = 0; return value = ObserveStack(); }\n\
                 func Interpreted() {\n\
                     var value = 0;\n\
                     if (false) return nil ?? 1;\n\
                     return value = ObserveStack();\n\
                 }",
            )
            .expect("assignment script loads");

        reset_compiled_function_execution_count();
        check_eq!(engine.call("Compiled", &[]).expect("compiled assignment succeeds") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
        check_eq!(engine.call("Interpreted", &[]).expect("AST assignment succeeds") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 2);
        let observed = observed.lock().expect("stack observation lock");

        check_eq!(observed.len() => 2);
        check_eq!(observed[0] => observed[1]);
    }

    #[test]
    fn compiled_map_key_keeps_same_zero_id_assignment_reference() {
        // AB_Set leaves its destination reference on the stack, and AB_MAP
        // copies GetRefVal without an intervening SetNoRef conversion
        // (C4AulExec.cpp:404-415; C4Value.cpp:121-140).
        let zero_id = Value::C4Id(crate::value::c4_id_from_raw(0));
        let expected = Value::Proplist(ValueMap::from([(zero_id.clone(), Value::Int(1))]));
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function_with_arity("ToId", 1, |args| Ok(args[0].clone()));
        check!(engine.set_host_function_parameter_types("ToId", [crate::value::C4VType::C4Id]));
        engine
            .load_script(
                "#strict 3\n\
                 local slot;\n\
                 func Probe() { return { [(slot = ToId(0))] = 1 }; }\n\
                 func Interpreted() {\n\
                     if (false) return nil ?? 1;\n\
                     return { [(slot = ToId(0))] = 1 };\n\
                 }",
            )
            .expect("assignment map script loads");
        let locals = HashMap::from([("slot".to_owned(), zero_id)]);

        reset_compiled_function_execution_count();
        let (ast_result, ast_locals) = engine
            .call_with_locals("Interpreted", &[], &locals)
            .expect("AST assignment map succeeds");
        check_eq!(ast_locals.get("slot") => locals.get("slot"));
        check_eq!(ast_result => expected.clone());
        check_eq!(compiled_function_execution_count() => 1);
        check_eq!(engine.call_with_locals("Probe", &[], &locals).expect("compiled assignment map succeeds").0 => expected);
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn compiled_while_break_leaves_the_loop() {
        // `break` unwinds to the loop's stack size and emits AB_JUMP
        // (C4AulParse.cpp:2109-2127); Parse_While patches it to the loop exit
        // (C4AulParse.cpp:2502-2508).
        reset_compiled_function_execution_count();
        check_script!(
            "func Sum(limit) {\n\
                 var total = 0;\n\
                 var i = 0;\n\
                 while (i < 10) {\n\
                     if (i == limit) break;\n\
                     total += i;\n\
                     i++;\n\
                 }\n\
                 return total;\n\
             }",
            "Sum",
            &[Value::Int(4)];
            expect "break leaves the while loop" => Value::Int(6)
        );
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_loop_control_binds_to_the_innermost_loop() {
        // C4Aul patches only `pLoopStack`'s own control list before PopLoop,
        // so an inner loop consumes its own break/continue and leaves the
        // outer loop's edges alone (C4AulParse.cpp:2502-2508,2613-2619).
        reset_compiled_function_execution_count();
        check_script!(
            "func Probe() {\n\
                 var trace = 0;\n\
                 for (var outer = 0; outer < 3; outer++) {\n\
                     if (outer == 1) continue;\n\
                     var inner = 0;\n\
                     while (inner < 3) {\n\
                         inner++;\n\
                         if (inner == 2) break;\n\
                         trace = trace * 10 + inner;\n\
                     }\n\
                     trace = trace * 10 + 9;\n\
                 }\n\
                 return trace;\n\
             }",
            "Probe",
            &[];
            expect "inner break leaves only the inner loop" => Value::Int(1919)
        );
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_loop_control_matches_the_ast_side_effect_order() {
        // `continue` re-enters through the incrementor and `break` skips both
        // it and the condition, so every clause callback keeps the AST order
        // (C4AulParse.cpp:2604-2619).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\n\
                 local trace;\n\
                 func Mark(value) { trace = trace * 10 + value; return value; }\n\
                 func Body() {\n\
                     trace = 0;\n\
                     for (var i = Mark(1); Mark(i) < 4; i = Mark(i + 1)) {\n\
                         if (i == 2) { Mark(7); continue; }\n\
                         if (i == 3) { Mark(8); break; }\n\
                         Mark(9);\n\
                     }\n\
                     return trace;\n\
                 }\n\
                 func Compiled() { return Body(); }\n\
                 func Interpreted() {\n\
                     if (false) return nil ?? 1;\n\
                     trace = 0;\n\
                     for (var i = Mark(1); Mark(i) < 4; i = Mark(i + 1)) {\n\
                         if (i == 2) { Mark(7); continue; }\n\
                         if (i == 3) { Mark(8); break; }\n\
                         Mark(9);\n\
                     }\n\
                     return trace;\n\
                 }",
            )
            .expect("loop control script loads");

        reset_compiled_function_execution_count();
        let compiled = engine
            .call_with_locals("Compiled", &[], &HashMap::new())
            .expect("compiled loop control succeeds")
            .0;
        // Compiled, Body, and its nine Mark callbacks all lower.
        check_eq!(compiled_function_execution_count() => 11);
        let interpreted = engine
            .call_with_locals("Interpreted", &[], &HashMap::new())
            .expect("AST loop control succeeds")
            .0;

        check_eq!(compiled => Value::Int(119_227_338));
        check_eq!(interpreted => compiled);
        // The second driver and its nine Mark callbacks also lower.
        check_eq!(compiled_function_execution_count() => 21);
    }

    #[test]
    fn compiled_loop_resumes_a_host_suspension_and_still_takes_its_break_edge() {
        // A suspension stores the instruction pointer, so a resumed body has
        // to land on the same patched loop edges the first pass would have
        // taken (C4AulParse.cpp:2109-2127,2604-2619).
        let marks = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mark_sink = std::sync::Arc::clone(&marks);
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("Mark", move |args| {
            mark_sink
                .lock()
                .expect("mark lock")
                .push(args.first().cloned());
            Ok(Value::Nil)
        });
        engine.register_host_function("Pause", |_| {
            Err(RuntimeError::host_continuation(
                PauseProbeRequest,
                Value::Nil,
            ))
        });
        const SOURCE: &str = "#strict 3\n\
             func Probe() {\n\
                 var total = 0;\n\
                 for (var i = 0; i < 4; i++) {\n\
                     if (i == 1) { Mark(i); continue; }\n\
                     if (i == 3) break;\n\
                     total = total + Pause();\n\
                     Mark(total);\n\
                 }\n\
                 return total;\n\
             }";
        engine
            .load_script(SOURCE)
            .expect("suspending loop script loads");
        // The invocation counter only sees completed direct executions, so a
        // suspending call is pinned through its plan instead.
        let functions = parse_functions(SOURCE, "suspending loop script parses");
        check!(CompiledFunction::compile(&functions["Probe"]).is_some());

        let mut outcome = engine
            .call_with_continuation("Probe", &[])
            .expect("Probe suspends inside the loop");
        let mut suspensions = 0;
        let result = loop {
            match outcome {
                ScriptCallOutcome::Suspended(suspension) => {
                    suspensions += 1;
                    check!(suspension.request::<PauseProbeRequest>().is_some());
                    outcome = engine
                        .resume_script_continuation_with_value(suspension, Value::Int(5))
                        .expect("Probe resumes inside the loop");
                }
                ScriptCallOutcome::Complete(value) => break value,
            }
        };

        // `i == 1` continues past Pause and `i == 3` breaks out before it.
        check_eq!(suspensions => 2);
        check_eq!(result => Value::Int(10));
        check_eq!(
            *marks.lock().expect("mark lock")
                => vec![Some(Value::Int(5)), Some(Value::Int(1)), Some(Value::Int(10))]
        );
    }

    #[test]
    fn compiled_loop_control_leaves_no_value_stack_residue() {
        // C4Aul precedes every loop control with an AB_STACK unwind back to
        // `Loop::StackSize`, so a break or continue cannot leak operands past
        // the loop (C4AulParse.cpp:2109-2149).
        const BODY: &str = "ObserveStack();\n\
             var i = 0;\n\
             while (i < 3) { i++; if (i == 2) break; }\n\
             ObserveStack();\n\
             for (var j = 0; j < 4; j++) {\n\
                 if (j == 1) continue;\n\
                 if (j == 2) break;\n\
             }\n\
             ObserveStack();\n\
             return 7;";
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let host_observed = std::sync::Arc::clone(&observed);
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("ObserveStack", move |_| {
            host_observed
                .lock()
                .expect("stack observation lock")
                .push(VALUE_STACK_SIZE.with(Cell::get));
            Ok(Value::Nil)
        });
        engine
            .load_script(&format!(
                "func Compiled() {{ {BODY} }}\n\
                 func Interpreted() {{ if (false) return nil ?? 1; {BODY} }}"
            ))
            .expect("loop residue script loads");

        reset_compiled_function_execution_count();
        check_eq!(engine.call("Compiled", &[]).expect("compiled loop succeeds") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
        check_eq!(engine.call("Interpreted", &[]).expect("AST loop succeeds") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 2);
        let observed = observed.lock().expect("stack observation lock");

        check_eq!(observed.len() => 6);
        check_eq!(observed[1] => observed[0]);
        check_eq!(observed[2] => observed[0]);
        check_eq!(observed[4] => observed[3]);
        check_eq!(observed[5] => observed[3]);
    }

    #[test]
    fn compiled_clauseless_for_continues_to_its_body() {
        // With neither incrementor nor condition C4Aul's back edge is the body
        // itself, and `continue` shares it (C4AulParse.cpp:2604-2619).
        reset_compiled_function_execution_count();
        check_script!(
            "func Probe() {\n\
                 var i = 0;\n\
                 var trace = 0;\n\
                 for (;;) {\n\
                     i++;\n\
                     if (i == 2) continue;\n\
                     trace = trace * 10 + i;\n\
                     if (i >= 4) break;\n\
                 }\n\
                 return trace;\n\
             }",
            "Probe",
            &[];
            expect "a clauseless for re-enters at its body" => Value::Int(134)
        );
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_conditionless_for_continues_to_its_incrementor() {
        // An incrementor without a condition still owns the back edge, and
        // C4Aul emits no jump from it to a condition that does not exist
        // (C4AulParse.cpp:2586-2619).
        reset_compiled_function_execution_count();
        check_script!(
            "func Probe() {\n\
                 var trace = 0;\n\
                 for (var i = 0;; i++) {\n\
                     if (i == 1) continue;\n\
                     trace = trace * 10 + i;\n\
                     if (i == 3) break;\n\
                 }\n\
                 return trace;\n\
             }",
            "Probe",
            &[];
            expect "continue reaches the incrementor without a condition" => Value::Int(23)
        );
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_loop_control_keeps_unreachable_trailing_statements_lowerable() {
        // C4Aul emits the statements after an unconditional break as ordinary
        // dead code rather than rejecting the function, so the plan keeps the
        // same shape (C4AulParse.cpp:2109-2127).
        reset_compiled_function_execution_count();
        check_script!(
            "func Probe() {\n\
                 var total = 1;\n\
                 while (total < 100) {\n\
                     break;\n\
                     total = 50;\n\
                 }\n\
                 return total;\n\
             }",
            "Probe",
            &[];
            expect "the statement after break never runs" => Value::Int(1)
        );
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn wide_raw_bool_keeps_native_union_equality_and_low_word_bool_semantics() {
        let raw = 1_usize << 32;
        let wide_bool = Value::from_c4_bool_data_raw(raw);
        let source_id = Value::C4Id(crate::value::c4_id_from_raw(raw));

        check!(c4_values_equal(&wide_bool, &source_id, Some(0), None, None));
        check!(!c4_values_equal(
            &wide_bool,
            &source_id,
            Some(2),
            None,
            None
        ));
        check!(c4_values_equal(
            &wide_bool,
            &Value::Bool(false),
            Some(3),
            None,
            None
        ));
    }
    use crate::parser::Parser;

    fn parse_script(source: &str, message: &str) -> crate::ast::Script {
        Parser::new(source).parse_script_strict().expect(message)
    }

    fn function_map(script: crate::ast::Script) -> FxHashMap<String, Function> {
        script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect()
    }

    fn parse_functions(source: &str, message: &str) -> FxHashMap<String, Function> {
        function_map(parse_script(source, message))
    }

    fn parse_function(source: &str, parse_message: &str, function_message: &str) -> Function {
        parse_script(source, parse_message)
            .functions
            .into_iter()
            .next()
            .expect(function_message)
    }

    fn test_vm<'a>(functions: &'a FxHashMap<String, Function>, var_decls: &'a [VarDecl]) -> Vm<'a> {
        static HOST_FUNCTIONS: std::sync::OnceLock<FxHashMap<String, RegisteredHostFunction>> =
            std::sync::OnceLock::new();
        Vm::new(
            functions,
            HOST_FUNCTIONS.get_or_init(FxHashMap::default),
            var_decls,
            None,
        )
    }

    fn execute_script(
        source: &str,
        entry_point: &str,
        args: &[Value],
    ) -> Result<Value, RuntimeError> {
        let functions = parse_functions(source, "parse should succeed");
        test_vm(&functions, &[]).call(entry_point, args)
    }

    #[test]
    fn local_scalar_control_flow_uses_compiled_executor() {
        reset_compiled_function_execution_count();

        check_script!(r#"
                func SumLoop(iterations) {
                    var acc = 0;
                    var index = 0;
                    while (index < iterations) {
                        acc = acc + (index % 7);
                        index = index + 1;
                    }
                    return acc;
                }
            "#,
            "SumLoop",
            &[Value::Int(128)]; expect "slot-resolved scalar loop runs" => Value::Int(379));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn value_method_calls_use_compiled_execution() {
        // AB_CALL resolves the receiver and then dispatches the named method
        // with ten parameter slots (C4AulExec.cpp:1216-1297).
        let functions = parse_functions(
            "func Target(value) { return value + 1; } func Probe(target) { return target->Target(41); }",
            "method script parses",
        );
        reset_compiled_function_execution_count();
        check_eq!(test_vm(&functions, &[]).call("Probe", &[Value::Object(7)]).expect("method call") => Value::Int(42));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn compiled_method_suspension_matches_interpreted_stack_and_result() {
        // AB_CALL keeps its target and ten arguments until return
        // (C4AulExec.cpp:1216-1297), even across a host suspension.
        let run = |interpreted: bool| {
            let observations = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let mut engine = crate::engine::Engine::new();
            let method_observations = observations.clone();
            engine.register_method_dispatch(std::sync::Arc::new(move |_| {
                method_observations
                    .lock()
                    .unwrap()
                    .push(VALUE_STACK_SIZE.with(Cell::get));
                Err(RuntimeError::host_continuation(
                    PauseProbeRequest,
                    Value::Nil,
                ))
            }));
            let after_observations = observations.clone();
            engine.register_host_function("Observe", move |_| {
                after_observations
                    .lock()
                    .unwrap()
                    .push(VALUE_STACK_SIZE.with(Cell::get));
                Ok(Value::Int(0))
            });
            // Unreachable short-circuit syntax must also compile without adding
            // a local slot or executing any additional side effects.
            let fallback = if interpreted {
                "if (false) { 0 ?? 0; }"
            } else {
                ""
            };
            let source = format!("#strict 3\nfunc Probe(target) {{ {fallback} return 1 + target->Pause(2) + Observe(); }}");
            let functions = parse_functions(&source, "suspending method parses");
            check!(CompiledFunction::compile(&functions["Probe"]).is_some());
            engine.load_script(&source).expect("method script loads");
            let ScriptCallOutcome::Suspended(suspension) = engine
                .call_with_continuation("Probe", &[Value::Object(7)])
                .expect("method suspends")
            else {
                panic!("method completed before host continuation")
            };
            check_eq!(VALUE_STACK_SIZE.with(Cell::get) => 0);
            let ScriptCallOutcome::Complete(result) = engine
                .resume_script_continuation_with_value(suspension, Value::Int(2))
                .expect("method resumes")
            else {
                panic!("method suspended twice")
            };
            check_eq!(VALUE_STACK_SIZE.with(Cell::get) => 0);
            let observations = observations.lock().unwrap().clone();
            (result, observations)
        };
        let interpreted = run(true);
        check_eq!(interpreted.0 => Value::Int(3));
        check_eq!(run(false) => interpreted);
    }

    #[test]
    fn direct_native_calls_keep_their_compiled_target() {
        // C++ stores the resolved C4AulFunc pointer in AB_CALL and passes it
        // straight to Call (C4AulExec.cpp:1250-1297).
        reset_generic_host_resolutions();
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("Native", |args| {
            Ok(args.first().cloned().unwrap_or(Value::Nil))
        });
        engine
            .load_script("#strict 2\nfunc Probe() { return Native(41); }")
            .expect("script loads");

        check_eq!(engine.call("Probe", &[]).expect("native call succeeds") => Value::Int(41));
        check_eq!(generic_host_resolutions() => 0);
    }

    #[test]
    fn direct_native_calls_stay_in_the_compiled_executor() {
        // C++ emits a resolved AB_CALL inside the surrounding bytecode rather
        // than returning to an AST evaluator (C4AulExec.cpp:1217-1297).
        reset_compiled_function_execution_count();
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("Double", |args| {
            Ok(Value::Int(
                args.first().and_then(Value::as_c4_int).unwrap_or(0) * 2,
            ))
        });
        engine
            .load_script(
                "#strict 2\nfunc Probe(value) { var doubled = Double(value); return doubled + 1; }",
            )
            .expect("script loads");

        check_eq!(engine
                .call("Probe", &[Value::Int(20)])
                .expect("native call succeeds") => Value::Int(41));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn typical_compiled_function_bindings_stay_inline() {
        // C++ addresses parameters and function vars as offsets in the active
        // C4AulExec value stack (C4AulExec.cpp:62-63,330-347), without a
        // per-call heap table for a small ordinary frame.
        reset_compiled_binding_heap_spills();
        check_script!("#strict 2\nfunc Probe(value) { var a = value + 1; var b = a + 1; return b; }",
            "Probe",
            &[Value::Int(39)]; expect "compiled frame executes" => Value::Int(41));
        check_eq!(compiled_binding_heap_spills() => 0);
    }

    #[test]
    fn repeated_compiled_scalar_calls_keep_executor_buffers_inline() {
        // C++ evaluates AB_CALL arguments in its fixed C4AulExec::Values stack
        // and keeps the frame's local slots there as well (C4AulExec.cpp:
        // 62-63,330-347,1217-1223), without per-call buffer allocations.
        reset_compiled_executor_heap_spills();
        check_script!(r#"#strict 2
                func AddOne(value) { return value + 1; }
                func Probe(iterations) {
                    var value = 0;
                    var index = 0;
                    while (index < iterations) {
                        value = AddOne(value);
                        index++;
                    }
                    return value;
                }
            "#,
            "Probe",
            &[Value::Int(64)]; expect "repeated compiled calls succeed" => Value::Int(64));
        check_eq!((
                COMPILED_STACK_HEAP_SPILLS.with(Cell::get),
                COMPILED_REGISTERED_SLOT_HEAP_SPILLS.with(Cell::get),
                COMPILED_CALL_ARGUMENT_TEMPORARIES.with(Cell::get),
            ) => (0, 0, 0));
    }

    #[test]
    fn effect_slot_decrements_stay_in_the_compiled_executor() {
        // EffectVar(...) is parsed as one retained C4Value reference and
        // AB_Dec1 reads and writes that reference once (C4AulParse.cpp:
        // 2311-2344; C4AulExec.cpp:450-487).
        reset_compiled_function_execution_count();
        let slot = std::sync::Arc::new(std::sync::Mutex::new(10_i32));
        let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let host_slot = std::sync::Arc::clone(&slot);
        let host_writes = std::sync::Arc::clone(&writes);
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("EffectVar", move |args| {
            if let Some(value) = args.get(3).and_then(Value::as_c4_int) {
                *host_slot.lock().expect("effect slot lock") = value;
                host_writes
                    .lock()
                    .expect("effect write log lock")
                    .push(value);
            }
            Ok(Value::Int(*host_slot.lock().expect("effect slot lock")))
        });
        engine
            .load_script(
                r#"#strict 2
                    func Probe(iterations) {
                        var total = 0;
                        var index = 0;
                        while (index < iterations) {
                            total += --EffectVar(0, 0, 1);
                            index++;
                        }
                        return total * 10 + EffectVar(0, 0, 1);
                    }
                "#,
            )
            .expect("script loads");

        check_eq!(engine
                .call("Probe", &[Value::Int(3)])
                .expect("effect slot loop succeeds") => Value::Int(247));
        check_eq!(*writes.lock().expect("effect write log lock") => vec![9, 8, 7]);
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_prefix_effect_slot_decrement_materializes_the_written_reference() {
        // Prefix AB_Dec1 leaves its C4Value reference on the stack, so the
        // result is materialized through FnEffectVar after the write. An
        // invalid effect number therefore remains nil rather than exposing
        // the arithmetic temporary (C4AulExec.cpp:450-487;
        // C4Script.cpp:5576-5586).
        reset_compiled_function_execution_count();
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("EffectVar", |_| Ok(Value::Nil));
        engine
            .load_script(
                r#"#strict 2
                    func Probe() {
                        return --EffectVar(0, 0, 0);
                    }
                "#,
            )
            .expect("script loads");

        check_eq!(engine.call("Probe", &[]).expect("probe succeeds") => Value::Nil);
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_effect_slot_update_retains_lower_expression_operands_through_host_access() {
        // AB_CALL pops EffectVar's three native parameters after retaining its
        // returned reference. AB_Add's left operand then remains below that
        // reference while AB_Dec1 reads, writes, and materializes it
        // (C4AulExec.cpp:450-487,682-702,1216-1297).
        let observed_stack_sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let host_observed_stack_sizes = std::sync::Arc::clone(&observed_stack_sizes);
        let slot = std::sync::Arc::new(std::sync::Mutex::new(2_i32));
        let host_slot = std::sync::Arc::clone(&slot);
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("EffectVar", move |args| {
            host_observed_stack_sizes
                .lock()
                .expect("stack-size log lock")
                .push(VALUE_STACK_SIZE.with(Cell::get));
            if let Some(value) = args.get(3).and_then(Value::as_c4_int) {
                *host_slot.lock().expect("effect slot lock") = value;
            }
            Ok(Value::Int(*host_slot.lock().expect("effect slot lock")))
        });
        engine
            .load_script(
                "#strict 2\n\
                 func Probe() { return 1 + --EffectVar(0, 0, 1); }\n\
                 func Interpreted() { if (false) return [1][0]; return 1 + --EffectVar(0, 0, 1); }",
            )
            .expect("script loads");

        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("probe succeeds") => Value::Int(2));
        check_eq!(compiled_function_execution_count() => 1);
        check_eq!(*observed_stack_sizes.lock().expect("stack-size log lock") => vec![12, 12, 12], "the external ten-slot frame, lower operand, and counter reference stay live");
        *slot.lock().expect("effect slot lock") = 2;
        observed_stack_sizes
            .lock()
            .expect("stack-size log lock")
            .clear();
        check_eq!(engine.call("Interpreted", &[]).expect("probe succeeds") => Value::Int(2));
        check_eq!(compiled_function_execution_count() => 2);
        check_eq!(*observed_stack_sizes.lock().expect("stack-size log lock") => vec![12, 12, 12], "the compiled instruction must retain exactly the AST path's C++ stack shape");
    }

    #[test]
    fn bytecode_reference_result_survives_the_callee_frame() {
        // AB_RETURN preserves a reference from a reference-returning function
        // (C4AulExec.cpp:1053-1090; C4Value.cpp:67-102).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nlocal data;\nfunc &GetData() { return data; }\n\
                 func Probe() { GetData() = 9; return data; }",
            )
            .expect("reference return script loads");
        reset_compiled_function_execution_count();
        let (result, locals) = engine
            .call_with_locals(
                "Probe",
                &[],
                &HashMap::from([("data".into(), Value::Int(4))]),
            )
            .expect("reference return remains writable");
        check_eq!(result => Value::Int(9));
        check_eq!(locals.get("data") => Some(&Value::Int(9)));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_reference_return_is_forwarded_through_nested_calls() {
        // AB_CALL can leave a C4V_pC4Value for the enclosing AB_RETURN
        // (C4AulExec.cpp:1053-1090; C4Value.cpp:67-102).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc &Identity(&slot) { return slot; }\n\
             func &Forward(&slot) { return Identity(slot); }\n\
             func Probe() { var slot = 4; Forward(slot) = 9; return slot; }",
            )
            .expect("reference forwarding script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("forwarded reference remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 3);
    }

    #[test]
    fn bytecode_effect_increment_uses_the_selected_script_overload() {
        // A selected script overload supplies the reference consumed by
        // AB_Inc1 (C4AulExec.cpp:450-454,1095-1140).
        let mut engine = crate::engine::Engine::new();
        engine.load_script("#strict 3\nlocal value; func &EffectVar(a, b, c) { return value; } func Probe() { value = 4; return ++EffectVar(0, 0, 1); }").expect("effect overload script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("selected effect overload increments") => Value::Int(5));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_unknown_call_is_checked_only_when_reached() {
        // An unexecuted branch must not change the executor for its enclosing
        // function (C4AulExec.cpp:995-999,1044-1050).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script("#strict 3\nfunc Probe(fail) { if (fail) Missing(); return 7; }")
            .expect("unreached call script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[Value::Bool(false)]).expect("unreached call is skipped") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
        check!(engine
            .call("Probe", &[Value::Bool(true)])
            .expect_err("reached missing call fails")
            .to_string()
            .contains("unknown function 'Missing'"));
    }

    #[test]
    fn bytecode_missing_binding_is_checked_only_when_reached() {
        // C4Aul branches jump over unexecuted instructions; lookup failures
        // must not force a different executor for the whole function
        // (C4AulExec.cpp:995-999,1044-1050).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script("#strict 3\nfunc Probe() { if (false) missing = 9; return 7; }")
            .expect("unreached binding script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("unreached binding is skipped") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_constant_argument_does_not_require_a_local_binding() {
        // Constants are pushed as values before Parse_Params invokes the
        // destination function (C4AulParse.cpp:2311-2344).
        let mut engine = crate::engine::Engine::new();
        engine.register_constant("ANSWER", Value::Int(7));
        engine.load_script("#strict 3\nfunc Identity(value) { return value; } func Probe() { return Identity(ANSWER); }").expect("constant argument script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("constant argument resolves") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_method_result_can_be_passed_as_a_reference_argument() {
        // AB_CALL can return a reference retained by Parse_Params for the
        // next call (C4AulExec.cpp:1216-1305; C4AulParse.cpp:2311-2325).
        let mut engine = crate::engine::Engine::new();
        engine.load_script("#strict 3\nlocal value; func &Slot() { return value; } func Set(&slot) { slot = 9; } func Probe(target) { Set(target->Slot()); return value; }").expect("method reference argument script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[Value::Object(42)]).expect("method reference reaches callee") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 3);
    }

    #[test]
    fn bytecode_nonstrict_goto_returns_before_its_expression_suffix() {
        // The NONSTRICT goto hack emits AB_RETURN immediately after the
        // call, before its expression suffix (C4AulParse.cpp:2193-2246).
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("goto", |args| {
            Ok(args.first().cloned().unwrap_or(Value::Nil))
        });
        engine
            .load_script("func Probe() { goto(41) + 1; return 99; }")
            .expect("legacy goto script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("goto returns its call result") => Value::Int(41));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_context_expression_returns_the_active_object() {
        // The public AST's explicit context expression has the same result
        // as C4Aul's this context function (C4Script.cpp:211-223).
        let mut function = parse_function(
            "func Probe() { return 0; }",
            "context fixture parses",
            "probe exists",
        );
        function.body = vec![Stmt::Return(Some(Expr::This))];
        let functions = FxHashMap::from_iter([(function.name.clone(), function)]);
        let vm = test_vm(&functions, &[]).with_this(Value::Object(42));
        reset_compiled_function_execution_count();
        check_eq!(vm.call("Probe", &[]).expect("context expression succeeds") => Value::Object(42));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_reference_argument_preserves_a_prefix_increment_result() {
        // AB_Inc1 leaves its operand reference; Parse_Params keeps it for
        // a C4V_pC4Value parameter (C4AulExec.cpp:450-454; C4AulParse.cpp:2311-2325).
        let mut engine = crate::engine::Engine::new();
        engine.load_script("#strict 3\nfunc Set(&slot) { slot = 9; } func Probe() { var value = 1; Set(++value); return value; }").expect("reference expression script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("increment argument remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_parse_error_only_fails_when_its_instruction_is_reached() {
        // AB_ERR is executable code, not a reason to reject the complete
        // function before its branches run (C4AulExec.cpp:401-402).
        let mut function = parse_function(
            "#strict 3\nfunc Probe(fail) { if (fail) return 1; return 7; }",
            "error fixture parses",
            "probe exists",
        );
        let Stmt::If { then_branch, .. } = &mut function.body[0] else {
            panic!("conditional fixture expected");
        };
        *then_branch = vec![Stmt::ParseError {
            message: "broken suffix".into(),
            line: 2,
            column: 3,
        }];
        let functions = FxHashMap::from_iter([(function.name.clone(), function)]);
        let vm = test_vm(&functions, &[]);
        reset_compiled_function_execution_count();
        check_eq!(vm.call("Probe", &[Value::Bool(false)]).expect("unreached error is skipped") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
        check!(vm
            .call("Probe", &[Value::Bool(true)])
            .expect_err("reached error fails")
            .to_string()
            .contains("parse error at 2:3: broken suffix"));
    }

    #[test]
    fn bytecode_non_nil_value_coalescing_assignment_skips_reference_validation() {
        // AB_NilCoalescingIt skips AB_Set for a non-nil value, even when it
        // is not a reference (C4AulExec.cpp:849-865).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script("#strict 3\nfunc Probe() { return !0 ??= 7; }")
            .expect("value coalescing script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("non-nil skips reference validation") => Value::Bool(true));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_prefix_change_remains_an_assignment_target() {
        // AB_Inc1 leaves the changed reference for the following changer
        // opcode (C4AulExec.cpp:450-460).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script("#strict 3\nfunc Probe() { var value = 1; ++value += 5; return value; }")
            .expect("prefix target script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("prefix result remains writable") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_effect_assignment_writes_the_native_slot() {
        // AB_Set retains the EffectVar reference while evaluating its RHS
        // (C4Script.cpp:5571-5578; C4AulExec.cpp:858-879).
        let slot = std::sync::Arc::new(std::sync::Mutex::new(4));
        let native_slot = slot.clone();
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("EffectVar", move |args| {
            let mut value = native_slot.lock().expect("effect fixture lock");
            if let Some(Value::Int(replacement)) = args.get(3) {
                *value = *replacement;
            }
            Ok(Value::Int(*value))
        });
        engine
            .load_script(
                "#strict 3\nfunc Probe() { EffectVar(0, 0, 1) += 3; return EffectVar(0, 0, 1); }",
            )
            .expect("effect assignment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("effect slot is writable") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_legacy_condition_keeps_the_first_reference_through_surplus_arguments() {
        // Parse_Params(1) retains the first reference until AB_STACK drops
        // the surplus arguments (C4AulParse.cpp:2311-2344,2492-2496).
        let mut engine = crate::engine::Engine::new();
        engine.load_script("#strict\nfunc Probe() { var value = 0; if (value, value = 1) return 7; return 0; }").expect("legacy condition script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("condition reads the retained reference") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_property_read_accepts_a_computed_receiver() {
        // AB_MAPA_V reads the evaluated receiver rather than requiring a
        // named local (C4AulExec.cpp:952-969).
        let mut engine = crate::engine::Engine::new();
        engine.load_script("#strict 3\nfunc Make() { return { value = 7 }; } func Probe() { return Make().value; }").expect("computed property script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("computed property reads") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_safe_navigation_skips_the_entire_remaining_suffix() {
        // AB_JUMPNIL skips every suffix operand until the final AB_DEREF
        // (C4AulParse.cpp:3105-3129).
        let mut engine = crate::engine::Engine::new();
        engine.load_script("#strict 3\nfunc Probe(target) { var calls = 0; var result = target?[++calls].key; return [result, calls]; }").expect("safe navigation script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[Value::Nil]).expect("nil skips the suffix") => Value::Array(vec![Value::Nil, Value::Int(0)]));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_optional_missing_call_evaluates_explicit_arguments() {
        // An unresolved failsafe name emits its explicit operands followed
        // by nil, without calling a target (C4AulParse.cpp:3215-3231).
        let mut engine = crate::engine::Engine::new();
        engine.load_script("#strict 2\nfunc Probe() { var calls = 0; var result = 0->~Missing(++calls); return [result, calls]; }").expect("optional script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("optional missing call succeeds") => Value::Array(vec![Value::Nil, Value::Int(1)]));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_global_call_uses_the_engine_owner_without_object_context() {
        // AB_CALLGLOBAL selects the engine owner and a nil object target
        // (C4AulExec.cpp:1216-1305).
        let object_functions = parse_functions(
            "#strict 3\nfunc Pick() { return 99; } func Probe() { return global->Pick(); }",
            "object script parses",
        );
        let global_functions = parse_functions(
            "#strict 3\nglobal func Pick() { return [7, this()]; }",
            "global script parses",
        );
        let declarations = Vec::new();
        let vm = test_vm(&object_functions, &declarations)
            .with_optional_globals(Some(&global_functions))
            .with_this(Value::Object(42));
        reset_compiled_function_execution_count();
        check_eq!(vm.call("Probe", &[]).expect("global call selects the engine") => Value::Array(vec![Value::Int(7), Value::Nil]));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_inherited_calls_follow_the_retained_owner_chain() {
        // inherited selects Fn->OwnerOverloaded rather than resolving the
        // current overload again (C4AulParse.cpp:2775-2798).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script("#strict 2\nfunc Probe(value) { return value + 1; }")
            .expect("base loads");
        engine
            .load_script("#strict 2\nfunc Probe(value) { return inherited(value) + 10; }")
            .expect("override loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[Value::Int(4)]).expect("inherited selects the base") => Value::Int(15));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_forwarded_arguments_keep_the_unnamed_parameter_reference() {
        // Parse_Params emits AB_PARN_R for the unnamed tail, so a reference
        // parameter can write through a forwarded argument (C4AulParse.cpp:2293-2306).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nfunc Set(&slot) { slot = 9; }\nfunc Forward(named) { Set(...); return Par(1); }\nfunc Probe() { return Forward(4, 7); }",
        ).expect("forwarding script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("forwarded parameter remains live") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 3);
    }

    #[test]
    fn bytecode_special_builtins_access_the_active_call_frame() {
        // FnPar reads the ten-slot C4AulParSet; FnSetLocal updates the
        // active object's local list (C4AulExec.cpp:404-409; C4Script.cpp:3409-3425).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nfunc Probe() { SetLocal(1, Par(1)); SetGlobal(2, Local(1)); return [Global(2), this()]; }",
        ).expect("context builtin script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[Value::Int(4), Value::Int(7)]).expect("builtins preserve the frame") => Value::Array(vec![Value::Int(7), Value::Nil]));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_method_slot_assignment_updates_the_selected_object() {
        // AB_CALL preserves the selected function's reference result
        // (C4AulExec.cpp:1216-1305; C4AulParse.cpp:2293-2344).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nlocal value; func &Slot() { return value; }\nfunc Probe(target) { target->Slot() = 7; return value; }",
        ).expect("method slot script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[Value::Object(42)]).expect("method reference remains writable") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_slot_assignments_preserve_frame_and_object_storage() {
        // FnVar addresses the call frame; FnLocal addresses the active object
        // (C4Script.cpp:3391-3396,3417-3425).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nfunc Probe() { Var(0) = 4; Local(2) = 7; Var(0) += Local(2); return [Var(0), Local(2)]; }",
        ).expect("slot assignment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("slot assignments remain live") => Value::Array(vec![Value::Int(11), Value::Int(7)]));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_effect_slot_return_keeps_the_native_reference() {
        // FnEffectVar exposes the effect's live C4Value slot, so a func &
        // result remains writable after AB_RETURN (C4Script.cpp:5571-5578;
        // C4AulParse.cpp:2293-2344).
        let slot = std::sync::Arc::new(std::sync::Mutex::new(4));
        let native_slot = slot.clone();
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("EffectVar", move |args| {
            let mut value = native_slot.lock().expect("effect fixture lock");
            if let Some(Value::Int(replacement)) = args.get(3) {
                *value = *replacement;
            }
            Ok(Value::Int(*value))
        });
        engine
            .load_script(
                "#strict 2\nfunc &Slot() { return EffectVar(0, 0, 1); }\n\
             func Probe() { Slot() = 9; return Slot(); }",
            )
            .expect("native reference script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("native reference remains writable") => Value::Int(9));
        check_eq!(*slot.lock().expect("effect fixture lock") => 9);
        check_eq!(compiled_function_execution_count() => 3);
    }

    #[test]
    fn bytecode_foreach_keeps_iteration_order_through_continue_and_break() {
        // AB_FOREACH_NEXT advances the retained collection cursor, while
        // loop controls jump to the next item or cleanup (C4AulExec.cpp:1135-1210).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nfunc Probe() { var result = 0; for (var item in [1, 2, 3, 4]) { if (item == 2) continue; result = result * 10 + item; if (item == 3) break; } return result; }",
        ).expect("foreach script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("foreach controls preserve order") => Value::Int(13));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_array_append_grows_before_the_rhs_runs() {
        // AB_ARRAY_APPEND inserts the nil slot before the assignment RHS
        // (C4AulExec.cpp:971-981).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 3\nfunc Probe() { var items = []; items[] = items == []; return items; }",
            )
            .expect("array append script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("array append succeeds") => Value::Array(vec![Value::Bool(false)]));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_nil_assignment_skips_a_non_nil_zero() {
        // AB_NilCoalescingIt tests the type, not truthiness, before AB_Set
        // (C4AulExec.cpp:849-856).
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let mut engine = crate::engine::Engine::new();
        engine.register_host_function("Mark", move |_| {
            observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Value::Int(9))
        });
        engine.load_script(
            "#strict 3\nfunc Probe() { var slot = 0; var empty; slot ??= Mark(); empty ??= 8; return [slot, empty]; }",
        ).expect("nil assignment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("nil assignment succeeds") => Value::Array(vec![Value::Int(0), Value::Int(8)]));
        check_eq!(calls.load(std::sync::atomic::Ordering::Relaxed) => 0);
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_nil_coalescing_returns_the_selected_reference() {
        // AB_NilCoalescing skips its RHS only for a non-nil left operand
        // (C4AulExec.cpp:1309-1320); the selected RHS can remain a reference.
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 3\nfunc &Pick(&left, &right) { return left ?? right; }\n\
             func Probe() { var left = nil; var right = 4; Pick(left, right) = 9; return right; }",
            )
            .expect("nil coalescing script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("selected reference remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_concat_assignment_preserves_nested_array_identity() {
        // AB_ConcatIt copies the appended entries through C4Value::Set
        // (C4AulExec.cpp:594-657).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nfunc Probe() { var nested = [1]; var joined = [nested]; joined ..= [nested]; return joined[0] == joined[1]; }",
        ).expect("concatenation assignment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("concatenation assignment preserves identity") => Value::Bool(true));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_concatenation_preserves_nested_array_identity() {
        // AB_Concat copies array entries through C4Value::Set
        // (C4AulExec.cpp:594-657).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nfunc Probe() { var nested = [1]; var left = [nested]; var joined = left .. [nested]; return joined[0] == joined[1]; }",
        ).expect("concatenation script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("concatenation preserves identity") => Value::Bool(true));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_compound_assignment_returns_the_indexed_reference() {
        // AB_Inc evaluates the RHS with the destination reference retained
        // and leaves that reference as the result (C4AulExec.cpp:786-803).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc &Add(&items, index) { return items[index] += 2; }\n\
             func Probe() { var items = [4]; Add(items, 0) = 9; return items[0]; }",
            )
            .expect("indexed compound assignment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("compound assignment result remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_prefix_increment_returns_the_indexed_reference() {
        // AB_Inc1 modifies and retains its reference operand; AB_RETURN does
        // not dereference a func & result (C4AulExec.cpp:450-458).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc &Increment(&items, index) { return ++items[index]; }\n\
             func Probe() { var items = [4]; Increment(items, 0) = 9; return items[0]; }",
            )
            .expect("indexed increment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("increment result remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_named_variable_builtin_retains_the_function_local() {
        // FnVarN resolves the immediate caller's VarNamed cell
        // (C4Script.cpp:4577-4588).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc Probe() { var count = 4; VarN(\"count\") = 9; return count; }",
            )
            .expect("named variable script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("named variable remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_dynamic_index_read_does_not_grow_the_array() {
        // AB_ARRAYA_V reads an absent positive index without inserting a slot
        // (C4AulExec.cpp:916-950).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 3\nfunc Probe(index) { var items = []; var value = items[index]; return [value, items]; }",
        ).expect("dynamic index script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[Value::Int(3)]).expect("dynamic index read succeeds") => Value::Array(vec![Value::Nil, Value::Array(vec![])]));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_index_argument_updates_the_callers_array() {
        // Parse_Params preserves array element references for an & parameter
        // (C4AulParse.cpp:2311-2344).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc Set(&slot) { slot = 9; }\n\
             func Probe() { var items = [4]; Set(items[0]); return items[0]; }",
            )
            .expect("indexed argument script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("indexed argument remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_property_assignment_writes_the_selected_entry() {
        // AB_MAPA_R selects the entry before AB_Set consumes the RHS
        // (C4AulExec.cpp:858-865,952-969; C4Value.cpp:67-102).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 3\nfunc Probe() { var items = {value = 4}; items.value = 9; return items.value; }",
        ).expect("property assignment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("property assignment succeeds") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_index_assignment_writes_the_selected_slot() {
        // AB_Set writes through its already evaluated left operand
        // (C4AulExec.cpp:858-865,952-969; C4Value.cpp:67-102).
        let mut engine = crate::engine::Engine::new();
        engine.load_script(
            "#strict 2\nfunc Probe() { var items = [4]; var index = 0; items[index] = 9; return items[0]; }",
        ).expect("indexed assignment script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("indexed assignment succeeds") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_property_return_retains_the_map_entry() {
        // AB_MAPA_R retains the selected entry through AB_RETURN
        // (C4Value.cpp:185-227; C4AulExec.cpp:1053-1090).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 3\nfunc &Entry(&items) { return items.value; }\n\
             func Probe() { var items = {value = 4}; Entry(items) = 9; return items.value; }",
            )
            .expect("property reference script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("property reference remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_dynamic_index_return_retains_the_array_slot() {
        // AB_ARRAYA_R keeps the selected array element as a reference across
        // AB_RETURN (C4Value.cpp:185-227; C4AulExec.cpp:1053-1090).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc &At(&items, index) { return items[index]; }\n\
             func Probe() { var items = [4]; At(items, 0) = 9; return items[0]; }",
            )
            .expect("indexed reference script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("indexed reference remains writable") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn bytecode_native_reference_argument_updates_the_script_local() {
        // Native C4V_pC4Value parameters retain the caller's slot through
        // conversion (C4Value.cpp:488-620).
        let mut engine = crate::engine::Engine::new();
        engine.register_host_reference_function("Set", [0], |args| {
            assert!(args[0].write(Value::Int(9))?);
            Ok(Value::Nil)
        });
        engine
            .load_script("func Probe() { var value = 1; Set(value); return value; }")
            .expect("native reference script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("native call succeeds") => Value::Int(9));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn bytecode_call_retains_a_reference_while_later_arguments_mutate_it() {
        // Parse_Params evaluates left-to-right and retains reference slots
        // until AB_CALL (C4AulParse.cpp:2311-2344).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc Add(&slot, amount) { slot += amount; return slot; }\n\
             func Mutate(&slot) { slot = 5; return 2; }\n\
             func Probe() { var slot = 1; Add(slot, Mutate(slot)); return slot; }",
            )
            .expect("nested reference script loads");
        reset_compiled_function_execution_count();
        check_eq!(engine.call("Probe", &[]).expect("nested call succeeds") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 3);
    }

    #[test]
    fn bytecode_reference_parameter_writes_the_callers_cell() {
        // A C4V_pC4Value parameter retains its caller's reference through
        // parameter conversion (C4AulExec.cpp:1364-1397; C4Value.cpp:488-620).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                "#strict 2\nfunc Add(&value) { value += 3; return value; }\n\
                 func Probe() { var value = 4; Add(value); return value; }",
            )
            .expect("reference parameter script loads");
        reset_compiled_function_execution_count();
        let (result, cells) = engine
            .call_with_ref_args("Add", &[Value::Int(4)])
            .expect("call succeeds");
        check_eq!(result => Value::Int(7));
        check_eq!(cells[0] => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_call_materializes_a_reference_return_before_returning_it_as_a_value() {
        // A value-context AB_CALL of a `func &` result is followed by
        // SetNoRef/C4Value::Set, which canonicalizes a retained C4ID(0) to
        // nil (C4AulParse.cpp:2293-2344; C4Value.cpp:121-140).
        let mut engine = crate::engine::Engine::new();
        engine
            .load_script(
                r#"#strict 2
                    local data;
                    func &GetData() { return data; }
                    func Probe() { return GetData(); }
                "#,
            )
            .expect("script loads");
        let locals = HashMap::from([(
            "data".to_owned(),
            Value::C4Id(crate::value::c4_id_from_raw(0)),
        )]);

        let (result, _) = engine
            .call_with_locals("Probe", &[], &locals)
            .expect("reference-returning call succeeds");

        check_eq!(result => Value::Nil);
    }

    #[test]
    fn compiled_call_honors_engine_wide_reference_parameter_candidates() {
        // Parse_Params' `anyfunctakesref` keeps the first argument as a live
        // reference when ANY same-name engine function declares `&` there.
        // The selected value-parameter callee dereferences only after every
        // argument has run (C4AulParse.cpp:2318-2331; C4AulExec.cpp:1364-1397).
        let mut engine = crate::engine::Engine::new();
        engine.register_reference_parameter_probe(std::rc::Rc::new(|name, slot| {
            name == "ReadBeforeMutation" && slot == 0
        }));
        engine
            .load_script(
                r#"#strict 2
                    local data;
                    func Mutate() { data = 2; }
                    func ReadBeforeMutation(value, ignored) { return value; }
                    func Probe() {
                        data = 1;
                        return ReadBeforeMutation(data, Mutate());
                    }
                "#,
            )
            .expect("script loads");

        let (result, _) = engine
            .call_with_locals("Probe", &[], &HashMap::new())
            .expect("same-name reference-aware call succeeds");

        check_eq!(result => Value::Int(2));
    }

    #[test]
    fn successful_object_calls_defer_diagnostic_object_formatting() {
        // C++ keeps the live C4Object pointer in the executor frame and only
        // asks GetDataString while dumping an error stack (C4AulExec.cpp:
        // 1328-1342), not on every successful function entry.
        fn format_object(id: u64) -> Option<(String, Option<String>)> {
            Some((format!("Object #{id}"), Some("CALL".to_owned())))
        }

        let functions = parse_functions(
            "func Helper() { return 41; } func Probe() { return Helper() + 1; }",
            "script parses",
        );
        let var_decls = Vec::new();
        reset_diagnostic_object_formatter_calls();
        let result = with_diagnostic_object_formatter(format_object, || {
            test_vm(&functions, &var_decls)
                .with_this(Value::Object(7))
                .call("Probe", &[])
        })
        .expect("nested object call succeeds");

        check_eq!(result => Value::Int(42));
        check_eq!(diagnostic_object_formatter_calls() => 0);
    }

    #[test]
    fn compiled_diagnostic_frames_share_stable_function_strings() {
        // C++ frames retain pointers to their C4AulFunc/C4AulScript metadata;
        // stable function and source names are not copied per call
        // (C4AulExec.cpp:62-63,1328-1342).
        let functions = parse_functions(
            "func Helper() { return 41; } func Probe() { return Helper() + 1; }",
            "script parses",
        );
        let var_decls = Vec::new();
        reset_diagnostic_frame_string_allocations();
        let result = test_vm(&functions, &var_decls)
            .call("Probe", &[])
            .expect("nested compiled call succeeds");

        check_eq!(result => Value::Int(42));
        check_eq!(diagnostic_frame_string_allocations() => 0);
    }

    #[test]
    fn unnamed_nil_parameter_slots_do_not_allocate_bindings() {
        // C++ keeps the ten AB_CALL parameter slots in C4AulExec::Values;
        // unused trailing nils are stack values, not heap cells
        // (C4AulExec.cpp:62-63, 1217-1223).
        reset_direct_binding_allocations();
        check_script!("#strict 2\nfunc Leaf() { return 41; }\nfunc Probe() { return Leaf() + 1; }",
            "Probe",
            &[]; expect "zero-argument calls succeed" => Value::Int(42));
        check_eq!(direct_binding_allocations() => 0);
    }

    #[test]
    fn nested_script_calls_keep_their_resolved_target() {
        // C++ saves the resolved function pointer back into AB_CALL before
        // invoking it (C4AulExec.cpp:1250-1297).
        reset_nested_generic_script_resolutions();
        check_script!("#strict 2\nfunc Leaf() { return 41; }\nfunc Probe() { return Leaf() + 1; }",
            "Probe",
            &[]; expect "nested call succeeds" => Value::Int(42));
        check_eq!(nested_generic_script_resolutions() => 0);
    }

    #[test]
    fn stippel_scalar_call_chain_uses_compiled_executor() {
        // C++ lowers ordinary locals, calls and branches into one bytecode
        // stream (C4AulParse.cpp:2789-3088; C4AulExec.cpp:330-1297).
        reset_compiled_function_execution_count();
        let mut engine = crate::engine::Engine::new();
        engine.register_constant("DIR_Left", Value::Int(0));
        engine
            .load_script(
                r#"#strict
                    local counter;
                    func Action() { return "Walk"; }
                    func Probe() {
                        counter++;
                        var speed = 10;
                        speed += 5;
                        if ((Action() eq "Walk") && (DIR_Left() == 0)) speed = speed + 1;
                        return speed + counter;
                    }
                "#,
            )
            .expect("Stippel-shaped scalar script loads");

        let (result, locals) = engine
            .call_with_locals("Probe", &[], &HashMap::new())
            .expect("Stippel-shaped scalar chain runs");

        check_eq!(result => Value::Int(17));
        check_eq!(locals.get("counter") => Some(&Value::Int(1)));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn cloned_function_does_not_reuse_a_plan_for_mutated_source() {
        let functions = parse_functions("func Probe() { return 1; }", "first source parses");
        let var_decls = Vec::new();
        test_vm(&functions, &var_decls)
            .call_pinned_args(&functions["Probe"], Vec::new())
            .expect("original function warms its plan");

        let replacement = parse_function(
            "func Probe() { return 2; }",
            "replacement source parses",
            "replacement function exists",
        );
        let mut cloned = functions["Probe"].clone();
        cloned.body = replacement.body;
        let cloned_functions = FxHashMap::from_iter([(cloned.name.clone(), cloned)]);

        let value = test_vm(&cloned_functions, &var_decls)
            .call("Probe", &[])
            .expect("mutated clone executes");
        check_eq!(value => Value::Int(2));
    }

    #[test]
    fn warmed_function_does_not_reuse_a_plan_after_in_place_mutation() {
        let mut functions = parse_functions("func Probe() { return 1; }", "first source parses");
        let var_decls = Vec::new();
        test_vm(&functions, &var_decls)
            .call("Probe", &[])
            .expect("original function warms its plan");

        let replacement = parse_function(
            "func Probe() { return 2; }",
            "replacement source parses",
            "replacement function exists",
        );
        functions
            .get_mut("Probe")
            .expect("original function remains owned")
            .body = replacement.body;

        reset_compiled_source_validations();
        reset_compiled_function_execution_count();
        let value = test_vm(&functions, &var_decls)
            .call_pinned_args(&functions["Probe"], Vec::new())
            .expect("mutated function executes");
        check_eq!(value => Value::Int(2));
        check_eq!(compiled_function_execution_count() => 1);
        check_eq!(compiled_source_validations() => 1);
    }

    #[test]
    fn inherited_function_does_not_reuse_a_plan_after_in_place_mutation() {
        let inherited = parse_function(
            "func Probe() { return 1; }",
            "inherited source parses",
            "inherited function exists",
        );
        let mut function = parse_function(
            "#strict 2\nfunc Probe() { return inherited(); }",
            "overriding source parses",
            "overriding function exists",
        );
        function.overloaded = Some(std::sync::Arc::new(inherited));
        let mut functions = FxHashMap::from_iter([(function.name.clone(), function)]);
        let var_decls = Vec::new();
        test_vm(&functions, &var_decls)
            .call("Probe", &[])
            .expect("inherited function warms its plan");

        let replacement = parse_function(
            "func Probe() { return 2; }",
            "replacement source parses",
            "replacement function exists",
        );
        std::sync::Arc::get_mut(
            functions
                .get_mut("Probe")
                .expect("overriding function remains owned")
                .overloaded
                .as_mut()
                .expect("inherited function remains installed"),
        )
        .expect("inherited function remains uniquely owned")
        .body = replacement.body;

        let value = test_vm(&functions, &var_decls)
            .call_pinned_args(&functions["Probe"], Vec::new())
            .expect("mutated inherited function executes");
        check_eq!(value => Value::Int(2));
    }

    #[test]
    fn global_function_does_not_reuse_a_plan_after_in_place_mutation() {
        let global = parse_function(
            "global func Probe() { return 1; }",
            "global source parses",
            "global function exists",
        );
        let functions = FxHashMap::default();
        let mut global_functions = FxHashMap::from_iter([(global.name.clone(), global)]);
        let var_decls = Vec::new();
        test_vm(&functions, &var_decls)
            .with_optional_globals(Some(&global_functions))
            .call("Probe", &[])
            .expect("global function warms its plan");

        let replacement = parse_function(
            "global func Probe() { return 2; }",
            "replacement source parses",
            "replacement function exists",
        );
        global_functions
            .get_mut("Probe")
            .expect("global function remains owned")
            .body = replacement.body;

        let value = test_vm(&functions, &var_decls)
            .with_optional_globals(Some(&global_functions))
            .call("Probe", &[])
            .expect("mutated global function executes");
        check_eq!(value => Value::Int(2));
    }

    #[test]
    fn linked_function_does_not_reuse_a_caller_warmed_plan() {
        let mut function = parse_function(
            "global func Probe() { return 1; }",
            "linked source parses",
            "linked function exists",
        );
        let functions = FxHashMap::from_iter([(function.name.clone(), function.clone())]);
        let var_decls = Vec::new();
        test_vm(&functions, &var_decls)
            .call_pinned_args(&function, Vec::new())
            .expect("caller-owned function warms its plan");

        let replacement = parse_function(
            "global func Probe() { return 2; }",
            "replacement source parses",
            "replacement function exists",
        );
        function.body = replacement.body;

        let mut engine = crate::engine::Engine::new();
        engine
            .load_script("global func Probe() { return 0; }")
            .expect("destination link parses");
        check!(engine.link_global_access_function("Probe", function));
        check_eq!(engine.call("Probe", &[]).expect("linked function executes") => Value::Int(2));
    }

    #[test]
    fn function_debug_omits_the_derived_compilation_cache() {
        let function = parse_function(
            "func Probe() { return 1; }",
            "source parses",
            "function exists",
        );

        check!(!format!("{function:?}").contains("compiled"));
    }

    #[test]
    fn compiled_repeated_path_reads_register_composite_parameter_once() {
        reset_runtime_container_registration_traversals();
        let state = Value::Proplist(ValueMap::from([
            ("a".to_string(), Value::Int(1)),
            ("b".to_string(), Value::Int(2)),
            ("c".to_string(), Value::Int(3)),
        ]));

        check_script!("#strict 3\nfunc Probe(state) { return state.a + state.b + state.c; }",
            "Probe",
            &[state]; expect "compiled property reads run" => Value::Int(6));
        check_eq!(runtime_container_registration_traversals() => 1);
    }

    #[test]
    fn compiled_negative_index_grows_a_referenced_empty_array() {
        reset_compiled_function_execution_count();
        check_script!("#strict 3\nfunc Probe() { var state = []; var ignored = state[0xffffffff]; return state; }",
            "Probe",
            &[]; expect "negative index follows native array growth" => Value::Array(vec![Value::Nil]));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_indexed_path_preserves_ast_string_registration_order() {
        fn run(source: &str) -> (Value, Vec<Vec<u8>>, usize) {
            let functions = parse_functions(source, "source parses");
            let var_decls = Vec::new();
            let registrations = crate::engine::new_string_registrations();
            let state = Value::Proplist(ValueMap::from([
                ("text".to_string(), Value::from("Zulu")),
                ("other".to_string(), Value::from("Other")),
            ]));
            reset_compiled_function_execution_count();
            let result = test_vm(&functions, &var_decls)
                .with_string_registrations(Some(&registrations))
                .call("Probe", &[state])
                .expect("probe executes");
            let order = crate::engine::enumerate_c4_strings(&registrations, &[]);
            (result, order, compiled_function_execution_count())
        }

        let compiled = run("#strict 3\nfunc Probe(state) { return [state.text[0], state.other]; }");
        let ast =
            run("#strict 3\nfunc Probe(state) { return [state.text[0], state.other]; Unknown(); }");

        check_eq!(compiled.0 => ast.0);
        check_eq!(compiled.1 => ast.1);
        check_eq!(compiled.2 => 1);
        check_eq!(ast.2 => 1);
    }

    #[test]
    fn bytecode_unreached_array_does_not_consume_the_value_stack() {
        // PushValue checks the running stack, not the largest static branch
        // (C4AulExec.cpp:179-212).
        let elements = std::iter::repeat_n("0", MAX_VALUE_STACK + 1)
            .collect::<Vec<_>>()
            .join(",");
        let source =
            format!("#strict 3\nfunc Probe() {{ if (false) return [{elements}]; return 7; }}");
        reset_compiled_function_execution_count();
        check_eq!(execute_script(&source, "Probe", &[]).expect("unreached branch is free") => Value::Int(7));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn compiled_stack_overflow_preserves_prior_string_registration() {
        fn run(source: &str) -> (String, Vec<Vec<u8>>) {
            let functions = parse_functions(source, "source parses");
            let var_decls = Vec::new();
            let registrations = crate::engine::new_string_registrations();
            let state = Value::Proplist(ValueMap::from([(
                "other".to_string(),
                Value::from("Observed"),
            )]));
            let live_state = state.clone();
            let error = test_vm(&functions, &var_decls)
                .with_string_registrations(Some(&registrations))
                .call("Probe", &[state])
                .expect_err("oversized value stack errors")
                .to_string();
            let order = crate::engine::enumerate_c4_strings(&registrations, &[]);
            drop(live_state);
            (error, order)
        }

        let elements = std::iter::repeat_n("0", MAX_VALUE_STACK + 1)
            .collect::<Vec<_>>()
            .join(",");
        let compiled = run(&format!(
            "#strict 3\nfunc Probe(state) {{ var earlier = state.other; return [{elements}]; }}"
        ));
        let ast = run(&format!(
            "#strict 3\nfunc Probe(state) {{ var earlier = state.other; return [{elements}]; Unknown(); }}"
        ));

        check_eq!(compiled.0 => ast.0);
        check_eq!(compiled.1 => ast.1);
        check!(!ast.1.is_empty());
    }

    #[test]
    fn compiled_object_path_hook_observes_the_ast_value_stack_depth() {
        fn run(source: &str) -> (Value, usize, usize) {
            let functions = parse_functions(source, "source parses");
            let var_decls = Vec::new();
            let observed_depth = Rc::new(Cell::new(0));
            let hook_depth = Rc::clone(&observed_depth);
            let hook: crate::engine::LocalCellHook = Rc::new(move |target, name| {
                if target == &Value::Object(7) && name == "value" {
                    hook_depth.set(VALUE_STACK_SIZE.with(Cell::get));
                    Some(value_cell(Value::Int(42)))
                } else {
                    None
                }
            });
            reset_compiled_function_execution_count();
            let value = test_vm(&functions, &var_decls)
                .with_local_cell_hook(Some(&hook))
                .call("Probe", &[Value::Object(7)])
                .expect("object property probe executes");
            (
                value,
                observed_depth.get(),
                compiled_function_execution_count(),
            )
        }

        let compiled = run("#strict 3\nfunc Probe(target) { return target.value; }");
        let ast =
            run("#strict 3\nfunc Probe(target) { return target.value; UnknownAfterReturn(); }");

        check_eq!(compiled.0 => ast.0);
        check_eq!(compiled.1 => ast.1);
        check_eq!(compiled.2 => 1);
        check_eq!(ast.2 => 1);
    }

    #[test]
    fn compiled_aggregate_construction_does_not_reregister_children() {
        reset_runtime_container_registration_traversals();
        let state = Value::Proplist(ValueMap::from([("value".to_string(), Value::Int(7))]));

        check_script!("#strict 3\nfunc Wrap(state) { return { copy = state }; }",
            "Wrap",
            std::slice::from_ref(&state); expect "compiled aggregate construction runs" => Value::Proplist(ValueMap::from([("copy".to_string(), state)])));
        check_eq!(runtime_container_registration_traversals() => 1);
    }

    #[test]
    fn local_container_reads_and_result_building_use_compiled_executor() {
        reset_compiled_function_execution_count();
        let state = Value::Proplist(ValueMap::from([
            (
                "position".to_string(),
                Value::Array(vec![Value::Int(40), Value::Int(20)]),
            ),
            (
                "velocity".to_string(),
                Value::Array(vec![Value::Int(2), Value::Int(0)]),
            ),
            ("energy".to_string(), Value::Int(100)),
        ]));

        check_script!(r#"
                #strict 3
                func Step(state, frame, random) {
                    var vx = state.velocity[0];
                    var vy = state.velocity[1] + 1;
                    var x = state.position[0] + vx;
                    var y = state.position[1] + vy;

                    if (y > 96) {
                        y = 96;
                        vy = -vy / 2;
                    }
                    if (x > 480) {
                        x = 480;
                        vx = -vx;
                    }
                    if (x < 0) {
                        x = 0;
                        vx = -vx;
                    }

                    return {
                        position = [x, y],
                        velocity = [vx, vy],
                        energy = state.energy - 1,
                    };
                }
            "#,
            "Step",
            &[state, Value::Int(0), Value::Int(0)]; expect "slot-resolved container computation runs" => Value::Proplist(ValueMap::from([
            (
                "position".to_string(),
                Value::Array(vec![Value::Int(42), Value::Int(21)])
            ),
            (
                "velocity".to_string(),
                Value::Array(vec![Value::Int(2), Value::Int(1)])
            ),
            ("energy".to_string(), Value::Int(99)),
        ])));
        check_eq!(compiled_function_execution_count() => 1);
    }

    #[test]
    fn ordinary_ten_slot_call_arguments_stay_inline() {
        // C++ evaluates directly into C4AulExec::Values[1024] and balances
        // script calls to C4AUL_MAX_Par == 10 without allocating a parameter
        // vector (C4AulExec.cpp:62-63, 1112-1130; C4Aul.h).
        CALL_ARG_HEAP_SPILLS.with(|count| count.set(0));
        check_script!(r#"
                func Callee(a, b, c, d, e, f, g, h, i, j) {
                    return a + b + c + d + e + f + g + h + i + j;
                }
                func Test() { return Callee(1, 2, 3, 4, 5, 6, 7, 8, 9, 10); }
            "#,
            "Test",
            &[]; expect "ten-argument script call runs" => Value::Int(55));
        check_eq!(CALL_ARG_HEAP_SPILLS.with(Cell::get) => 0, "C4Aul's fixed-size call frame must not spill ordinary arguments to the heap");
    }

    #[test]
    fn calls_inside_global_functions_stay_in_engine_scope() {
        // A global function is owned by Game.ScriptEngine, so its unqualified
        // calls resolve through that engine rather than the current object's
        // definition (C4AulParse.cpp:2808-2813). Hazard's AddLightCone must
        // therefore call the global CreateLight, not FLHH::CreateLight.
        let object_functions = parse_functions(
            r#"
                func CreateLight() { return AddLightCone(); }
                func Test() { return AddLightCone(); }
            "#,
            "object script parses",
        );
        let global_functions = parse_functions(
            r#"
                global func CreateLight() { return 42; }
                global func AddLightCone() { return CreateLight(); }
            "#,
            "global script parses",
        );
        let var_decls = Vec::new();
        let vm =
            test_vm(&object_functions, &var_decls).with_optional_globals(Some(&global_functions));

        check_eq!(vm.call("Test", &[]).expect("global call resolves") => Value::Int(42));
    }

    #[test]
    fn foreign_numbered_local_resolves_through_the_cell_hook() {
        // FnLocal (C4Script.cpp:3423-3433): `Local(i, pObj)` returns
        // `pObj->Local[iIndex].GetRef()` — reads AND writes reach the
        // FOREIGN object's numbered slot. The cross-object cell hook
        // carries it under the engine's `__local_{i}` persistence key.
        let source = r#"
            func Test(target) {
                Local(2, target) = 84;
                return Local(2, target) + 1;
            }
        "#;
        let functions = parse_functions(source, "parse should succeed");
        let var_decls: Vec<VarDecl> = Vec::new();
        let cell = value_cell(Value::Nil);
        let hook_cell = cell.clone();
        let hook: crate::engine::LocalCellHook = std::rc::Rc::new(move |target, name| {
            (matches!(target, Value::Int(42)) && name == "__local_2").then(|| hook_cell.clone())
        });
        let vm = test_vm(&functions, &var_decls).with_local_cell_hook(Some(&hook));
        let result = vm.call("Test", &[Value::Int(42)]).expect("script runs");
        check_eq!(result => Value::Int(85), "the read sees the earlier write");
        check_eq!(*cell.borrow() => Value::Int(84), "the write landed in the foreign cell");
    }

    #[test]
    fn vm_executes_basic_arithmetic() {
        let source = "func Test() { return 5 + 3; }";
        check_script!(source, "Test", &[]; unwrap => Value::Int(8));
    }

    #[test]
    fn vm_handles_local_variables() {
        let source = r#"
            func Test() {
                var x = 10;
                var y = 20;
                return x + y;
            }
        "#;
        check_script!(source, "Test", &[]; unwrap => Value::Int(30));
    }

    #[test]
    fn function_var_shadows_same_named_object_local_with_shared_cells() {
        // C4Aul's function VarNamed table precedes the object's LocalNamed
        // table. MART relies on this: FxIntDoMagicTimer declares a temporary
        // `var pClonk` without overwriting MART's persistent `local pClonk`.
        let script = parse_script(
            r#"
                local pClonk;
                func Timer() {
                    var pClonk;
                    pClonk = 99;
                    return pClonk;
                }
            "#,
            "script parses",
        );
        let var_decls = script.var_decls.clone();
        let functions = function_map(script);
        let vm = test_vm(&functions, &var_decls);
        let cells = LocalCells::from_local_vars(&HashMap::from([(
            "pClonk".to_string(),
            Value::Object(574),
        )]));

        check_eq!(vm.call_with_cells("Timer", &[], &cells)
                .expect("function-local assignment runs") => Value::Int(99));
        check_eq!(cells.snapshot().get("pClonk") => Some(&Value::Object(574)), "the call-scoped var must not alias the persistent object local");
    }

    #[test]
    fn varn_reads_and_writes_only_named_function_vars() {
        let script = parse_script(
            r#"
                #strict
                local persisted;
                func Probe(x, only_param) {
                    persisted = 9;
                    var x = 5;
                    var dynamic_name = "x";
                    var before = VarN(dynamic_name);
                    VarN(dynamic_name) = 7;
                    return [before, x, VarN("x"), VarN("only_param"), VarN("persisted"), VarN("missing")];
                }
            "#,
            "script parses",
        );
        let var_decls = script.var_decls.clone();
        let functions = function_map(script);
        let vm = test_vm(&functions, &var_decls);

        check_eq!(vm.call("Probe", &[Value::Int(42), Value::Int(84)])
            .expect("VarN reads and writes the live function-var cell") => Value::Array(vec![
            Value::Int(5),
            Value::Int(42),
            Value::Int(7),
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ]));
    }

    #[test]
    fn varn_without_a_script_caller_returns_nil() {
        let engine = crate::engine::Engine::new();

        check_eq!(engine
                .call("VarN", &[Value::String("x".to_string().into())])
                .expect("a direct VarN dispatch is not an unknown-function error") => Value::Nil);
    }

    #[test]
    fn vm_handles_function_parameters() {
        let source = "func Add(a, b) { return a + b; }";
        check_script!(source, "Add", &[Value::Int(7), Value::Int(3)]; unwrap => Value::Int(10));
    }

    #[test]
    fn vm_binds_duplicate_parameter_names_like_c4value_map_names() {
        // The duplicate fourth name reuses slot zero in C4Aul; timer and
        // change consequently read call arguments 4 and 5, not 5 and 6.
        let source =
            "#strict\nfunc Merge(target, number, name, target, timer, change) { return [target, timer, change]; }";
        check_script!(source,
            "Merge",
            &[
                Value::Int(10),
                Value::Int(20),
                Value::Int(30),
                Value::Int(40),
                Value::Int(50),
                Value::Int(60),
            ]; expect "duplicate-name function runs" => Value::Array(vec![Value::Int(10), Value::Int(40), Value::Int(50)]));
    }

    #[test]
    fn vm_reports_undefined_variable() {
        let source = "func Test() { return undefined_var; }";
        let error = execute_script(source, "Test", &[]).unwrap_err();
        check!(error.message().contains("undefined variable"));
    }

    #[test]
    fn vm_reports_unknown_function() {
        let source = "func Test() { return 1; }";
        let error = execute_script(source, "Missing", &[]).unwrap_err();
        check!(error.message().contains("unknown function"));
    }

    #[test]
    fn vm_handles_nested_function_calls() {
        let source = r#"
            func Inner() { return 42; }
            func Outer() { return Inner(); }
        "#;
        check_script!(source, "Outer", &[]; unwrap => Value::Int(42));
    }

    #[test]
    fn vm_enforces_value_stack_before_context_limit() {
        let source = r#"
            func Recursive(n) {
                if (n <= 0) return 0;
                return Recursive(n - 1);
            }
        "#;
        // Ten parameter slots per call reach C++'s value-stack ceiling before
        // its independent 512-context ceiling.
        let error = execute_script(source, "Recursive", &[Value::Int(102)]).unwrap_err();
        check_eq!(error.message() => "internal error: value stack overflow!");
    }

    #[test]
    fn vm_handles_array_creation() {
        let source = "#strict\nfunc Test() { var arr = [1, 2, 3]; return arr[1]; }";
        check_script!(source, "Test", &[]; unwrap => Value::Int(2));
    }

    #[test]
    fn vm_handles_array_index_assignment() {
        let source = r#"
            #strict
            func Test() {
                var arr = [0, 0, 0];
                arr[1] = 42;
                return arr[1];
            }
        "#;
        check_script!(source, "Test", &[]; unwrap => Value::Int(42));
    }

    #[test]
    fn vm_auto_resizes_array_on_assignment() {
        let source = r#"
            #strict
            func Test() {
                var arr = [1];
                arr[5] = 99;
                return arr[5];
            }
        "#;
        check_script!(source, "Test", &[]; unwrap => Value::Int(99));
    }

    #[test]
    fn vm_array_indices_coerce_clamp_and_grow_like_cpp() {
        let source = r#"
            #strict
            func Test() {
                var nil_index;
                var a = [7, 8];
                var reads = [a[-1], a[nil_index], a[true], a[2]];
                a[-1] = 5;
                var written = a[0];
                var e = [];
                var empty = e[-1];
                var old = a[-1]++;
                var coerced = [0, 0];
                coerced[nil_index] = 3;
                coerced[true] = 4;
                return [reads, written, empty, e, old, a[0], coerced];
            }
        "#;

        check_script!(source, "Test", &[]; expect "array accesses succeed" => Value::Array(vec![
            Value::Array(vec![
                Value::Int(7),
                Value::Int(7),
                Value::Int(8),
                Value::Nil,
            ]),
            Value::Int(5),
            Value::Nil,
            Value::Array(vec![Value::Nil]),
            Value::Int(5),
            Value::Int(6),
            Value::Array(vec![Value::Int(3), Value::Int(4)]),
        ]));
    }

    #[test]
    fn vm_negative_array_indices_clamp_reads_writes_and_compound_ops_to_zero() {
        let source = r#"#strict 2
            func Test() {
                var a = [7, 8];
                var read = a[-1];
                a[-2] = 1;
                var written = a[0];
                a[-1] += 1;
                return [read, written, a[0]];
            }
        "#;

        check_script!(source, "Test", &[]; expect "negative array indices clamp" => Value::Array(vec![Value::Int(7), Value::Int(1), Value::Int(2)]));
    }

    #[test]
    fn vm_empty_negative_read_grows_nested_and_reference_return_paths() {
        let mut engine = crate::engine::Engine::new();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured_by_host = std::sync::Arc::clone(&captured);
        engine.register_host_function("Capture", move |args| {
            *captured_by_host.lock().unwrap() = args.first().cloned();
            Ok(Value::Int(0))
        });
        engine
            .load_script(
                r#"
                    #strict
                    local Data;

                    func & GetData() { return Data; }
                    func GrowThroughReference() {
                        Data = [];
                        var ignored = GetData()[-1];
                        return Data;
                    }
                    func GrowBeforeNestedFailure() {
                        Data = [];
                        return Data[-1][Capture(Data)];
                    }
                "#,
            )
            .expect("script loads");

        check_eq!(engine
                .call("GrowThroughReference", &[])
                .expect("reference read succeeds") => Value::Array(vec![Value::Nil]));
        check!(engine.call("GrowBeforeNestedFailure", &[]).is_err());
        check_eq!(*captured.lock().unwrap() => Some(Value::Array(vec![Value::Nil])));
    }

    #[test]
    fn vm_array_growth_stops_at_cpp_value_list_max_size() {
        let source = "#strict\nfunc Grow(index) { var a = []; a[index] = 1; return a; }";
        let grown = execute_script(source, "Grow", &[Value::Int(999_999)])
            .expect("last valid array index grows");
        let Value::Array(elements) = grown else {
            panic!("array expected");
        };
        check_eq!(elements.len() => ARRAY_MAX_SIZE);
        check_eq!(elements.last() => Some(&Value::Int(1)));
        drop(elements);

        match execute_script(source, "Grow", &[Value::Int(1_000_000)]) {
            Ok(_) => panic!("index at array cap unexpectedly succeeded"),
            Err(error) => check_eq!(error.message() => "out of memory"),
        }
    }

    #[test]
    fn vm_string_indices_follow_cpp_offsets_bounds_and_coercion() {
        let source = r#"#strict 2
            func Test() {
                var nil_index;
                var nested = [["abc"]];
                return [
                    "abc"[0],
                    "abc"[-1],
                    "abc"[5],
                    "abc"[-5],
                    "abc"[nil_index],
                    "abc"[false],
                    "abc"[true],
                    nested[0][0][1][0]
                ];
            }
        "#;

        check_script!(source, "Test", &[]; expect "string accesses succeed" => Value::Array(vec![
            Value::String("a".into()),
            Value::String("c".into()),
            Value::Nil,
            Value::Nil,
            Value::String("a".into()),
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("b".into()),
        ]));
    }

    #[test]
    fn dynamic_eval_reads_internal_byte_projection_as_source_bytes() {
        let source = "func Probe(string code) { return eval(code); }";
        let code = c4_string_from_bytes(&[b'\"', 0xff, b'\"']);
        check_script!(source, "Probe", &[Value::String(code.into())]; expect "projected source evaluates" => Value::String(c4_string_from_bytes(&[0xff]).into()));

        check_script!(source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"1\0+1").into())]; expect "NUL-terminated source evaluates its prefix" => Value::Int(1));
        check_script!(source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"\"open\0\"").into())]; expect "a literal truncated by NUL is a DirectExec parse failure" => Value::Nil);

        check_script!(source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"1\x1f+1").into())]; expect "all C++ control-byte whitespace is skipped" => Value::Int(2));
        check_script!(source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"1\xc2\xa0+1").into())]; expect "non-ASCII whitespace is a DirectExec parse failure" => Value::Nil);
        check_script!(source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"\"a\nb\"").into())]; expect "a raw newline in a string is a DirectExec parse failure" => Value::Nil);
        check_script!(source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"true\xc3\xbf").into())]; expect "the non-ASCII source byte causes a DirectExec parse failure" => Value::Nil);
        check_script!(source,
                "Probe",
                &[Value::String(
                    c4_string_from_bytes(b"1//comment\r+1").into()
                )]; expect "a carriage return ends a C++ line comment" => Value::Int(2));
    }

    #[test]
    fn bytecode_direct_exec_updates_live_local_cells() {
        // DirectExec parses a temporary function and executes its bytecode
        // in the supplied object context (C4AulExec.cpp:1657-1699).
        let functions = FxHashMap::default();
        let declarations = Vec::new();
        let cells = LocalCells::from_local_vars(&HashMap::new());
        let vm = test_vm(&functions, &declarations);
        reset_compiled_function_execution_count();
        check_eq!(vm.direct_exec_with_cells("Local(0) = 7", &cells, Some(3)).expect("expression runs") => Value::Int(7));
        check_eq!(vm.direct_exec_with_cells("++Local(0)", &cells, Some(3)).expect("live cell increments") => Value::Int(8));
        check_eq!(compiled_function_execution_count() => 2);
    }

    #[test]
    fn host_direct_exec_reads_internal_byte_projection_as_source_bytes() {
        let functions = FxHashMap::default();
        let var_decls = Vec::new();
        let vm = test_vm(&functions, &var_decls);
        let source = c4_string_from_bytes(&[b'\"', 0xff, b'\"']);
        let expected = Value::String(c4_string_from_bytes(&[0xff]).into());

        let (value, _) = vm
            .direct_exec_with_locals(&source, &HashMap::new(), None)
            .expect("projected source executes with copied locals");
        check_eq!(value => expected);

        let cells = LocalCells::default();
        check_eq!(vm.direct_exec_with_cells(&source, &cells, None)
                .expect("projected source executes with live cells") => expected);

        let nul_terminated = c4_string_from_bytes(b"1\0+1");
        let (value, _) = vm
            .direct_exec_with_locals(&nul_terminated, &HashMap::new(), None)
            .expect("NUL-terminated host source evaluates its prefix");
        check_eq!(value => Value::Int(1));

        let truncated_literal = c4_string_from_bytes(b"\"open\0\"");
        check_eq!(vm.direct_exec_with_cells(&truncated_literal, &cells, None)
                .expect("a host literal truncated by NUL is a parse failure") => Value::Nil);
    }

    #[test]
    fn vm_string_index_reports_cpp_type_error() {
        let source = r#"#strict 2
            func Test() {
                var index = "x";
                return "abc"[index];
            }
        "#;
        let error = execute_script(source, "Test", &[]).expect_err("string index must fail");

        check_eq!(error.message() => "indexed string access: index of type string, int expected!");
    }

    #[test]
    fn vm_string_index_result_has_fresh_cpp_string_identity() {
        let source = r#"#strict
            func Test() {
                var source = "abc";
                var indexed = source[0];
                return [indexed == indexed, source[0] == source[0]];
            }
        "#;

        check_script!(source, "Test", &[]; expect "string identity checks succeed" => Value::Array(vec![Value::Bool(true), Value::Bool(false)]));
    }

    #[test]
    fn vm_handles_proplist_creation() {
        let source = "#strict 3\nfunc Test() { var obj = { x = 10 }; return obj.x; }";
        check_script!(source, "Test", &[]; unwrap => Value::Int(10));
    }

    #[test]
    fn vm_handles_proplist_property_assignment() {
        let source = r#"
            #strict 3
            func Test() {
                var obj = { x = 1 };
                obj.x = 42;
                return obj.x;
            }
        "#;
        check_script!(source, "Test", &[]; unwrap => Value::Int(42));
    }

    #[test]
    fn vm_map_for_in_declares_and_binds_key_value_pairs() {
        let source = r#"#strict 3
            func Test() {
                var seen = {};
                for (var key, value in { alpha = 11, beta = 22 }) {
                    seen[key] = value;
                }
                return seen;
            }
        "#;

        check_script!(source, "Test", &[]; expect "declared map foreach runs" => Value::Proplist(ValueMap::from([
            ("alpha".to_string(), Value::Int(11)),
            ("beta".to_string(), Value::Int(22)),
        ])));
    }

    #[test]
    fn vm_map_for_in_predeclared_variables_honor_continue_and_break() {
        let source = r#"#strict 3
            func Test() {
                var key, value;
                var visited = 0, sum = 0;
                var entries = { one = 1, two = 2, three = 3 };

                for (key, value in entries) {
                    visited += 1;
                    if (value == 2) continue;
                    sum += value;
                }

                var break_visits = 0;
                for (key, value in entries) {
                    break_visits += 1;
                    break;
                }

                return [visited, sum, break_visits];
            }
        "#;

        check_script!(source, "Test", &[]; expect "predeclared map foreach runs" => Value::Array(vec![Value::Int(3), Value::Int(4), Value::Int(1)]));
    }

    #[test]
    fn vm_map_for_in_uses_insertion_order_and_hoists_implicit_binders() {
        let source = r#"#strict 3
            func Test() {
                var order = "";
                var total = 0;
                var entries = { second = 2, first = 1, second = 22 };
                entries ..= { first = 11, third = 3 };
                for (key, value in entries) {
                    order = order .. key;
                    total += value;
                }
                return [order, total, key, value];
            }
        "#;

        check_script!(source, "Test", &[]; expect "ordered map foreach runs" => Value::Array(vec![
            Value::String("secondfirstthird".to_string().into()),
            Value::Int(36),
            Value::String("third".to_string().into()),
            Value::Int(3),
        ]));
    }

    #[test]
    fn vm_map_for_in_reinsert_moves_key_to_end_deterministically() {
        let source = r#"#strict 3
            func Test() {
                var entries = {};
                entries["a"] = 1;
                entries["b"] = 2;
                entries["a"] = 3;
                entries["a"] = nil;
                entries["a"] = 4;

                var flattened = [];
                var index = 0;
                for (var key, value in entries) {
                    flattened[index++] = key;
                    flattened[index++] = value;
                }
                return flattened;
            }
        "#;
        let expected = Value::Array(vec![
            Value::String("b".to_string().into()),
            Value::Int(2),
            Value::String("a".to_string().into()),
            Value::Int(4),
        ]);

        for _ in 0..2 {
            check_script!(source, "Test", &[]; expect "map remove/reinsert foreach runs" => expected);
        }
    }

    #[test]
    fn vm_map_for_in_rejects_non_map_iterable() {
        let source = r#"#strict 3
            func Test() {
                for (var key, value in 5) {}
            }
        "#;

        let error = execute_script(source, "Test", &[]).expect_err("map foreach rejects int");
        check!(
            error.message().contains("for: map expected, but got int!"),
            "unexpected error: {}",
            error.message()
        );
    }

    #[test]
    fn vm_map_entry_removal_clears_the_removed_value_identity() {
        let source = r#"#strict
            func Test(entries) {
                var old_value = entries["entry"];
                entries["entry"] = 0;
                return [entries["entry"] == old_value, entries["entry"] == 0];
            }
        "#;

        check_script!(source,
                "Test",
                &[Value::Proplist(ValueMap::from([(
                    "entry".to_string(),
                    Value::String("same".into()),
                )]))]; expect "map entry removal runs" => Value::Array(vec![Value::Bool(false), Value::Bool(true)]));
    }

    #[test]
    fn vm_handles_while_loop() {
        let source = r#"
            func Test() {
                var sum = 0;
                var i = 1;
                while (i <= 5) {
                    sum = sum + i;
                    i = i + 1;
                }
                return sum;
            }
        "#;
        check_script!(source, "Test", &[]; unwrap => Value::Int(15));
    }

    #[test]
    fn vm_handles_if_statement() {
        let source = r#"
            func Test(x) {
                if (x > 10) {
                    return 1;
                }
                return 0;
            }
        "#;
        check_script!(source, "Test", &[Value::Int(15)]; unwrap => Value::Int(1));
        check_script!(source, "Test", &[Value::Int(5)]; unwrap => Value::Nil);
    }
}
