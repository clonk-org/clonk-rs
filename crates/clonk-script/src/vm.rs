use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use smallvec::SmallVec;

use crate::ast::{
    AccessLevel, AssignmentTarget, BinaryOp, Expr, ForInit, Function, IndexOperand,
    NavigationOperation, Parameter, SafeNavigationStep, Stmt, TypeAnnotation, UnaryOp, VarDecl,
};
use crate::debugger::DebuggerHooks;
use crate::engine::{
    EvalDirectExecHook, GlobalCallContextHook, HostFunction, HostReferenceFunction,
    RegisteredHostFunction,
};
use crate::error::{RuntimeCallFrame, RuntimeError};
use crate::value::{
    c4_id_text, c4_string_bytes, c4_string_from_bytes, c4_strings_equal, C4StringValue, C4VType,
    Literal, Value, ValueMap,
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

#[derive(Clone, Copy)]
enum CompiledCallTarget<'a> {
    Host(ResolvedHostFunction<'a>),
    Script(ScriptFunctionTarget<'a>),
    LegacyConstant,
}

/// Whether a C4Aul entry point treats a failed script-parameter conversion as
/// an error. Scripted C4Effect callbacks request C++'s
/// `nonStrict3WarnConversionOnly` behavior for pre-`#strict 3` functions.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ParameterConversionFailurePolicy {
    Error,
    WarnForNonStrict3EffectCallback,
}

struct AssignmentOperator<'a> {
    operation: Option<&'a BinaryOp>,
    spelling: &'a str,
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
    /// Live C4Value cells in every suspended C4Aul frame. AssignRemoval
    /// synchronously clears the equivalent intrusive C++ reference list
    /// (C4Object.cpp:312).
    static ACTIVE_OBJECT_REFERENCE_CELLS: RefCell<Vec<Vec<Weak<RefCell<Value>>>>> = const {
        RefCell::new(Vec::new())
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
fn reset_compiled_function_execution_count() {
    COMPILED_FUNCTION_EXECUTIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn compiled_function_execution_count() -> usize {
    COMPILED_FUNCTION_EXECUTIONS.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_compiled_source_validations() {
    COMPILED_SOURCE_VALIDATIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn compiled_source_validations() -> usize {
    COMPILED_SOURCE_VALIDATIONS.with(Cell::get)
}

#[cfg(test)]
fn reset_compiled_binding_heap_spills() {
    COMPILED_BINDING_HEAP_SPILLS.with(|count| count.set(0));
}

#[cfg(test)]
fn compiled_binding_heap_spills() -> usize {
    COMPILED_BINDING_HEAP_SPILLS.with(Cell::get)
}

#[cfg(test)]
fn reset_compiled_executor_heap_spills() {
    COMPILED_STACK_HEAP_SPILLS.with(|count| count.set(0));
    COMPILED_REGISTERED_SLOT_HEAP_SPILLS.with(|count| count.set(0));
    COMPILED_CALL_ARGUMENT_TEMPORARIES.with(|count| count.set(0));
}

#[cfg(test)]
fn reset_diagnostic_object_formatter_calls() {
    DIAGNOSTIC_OBJECT_FORMATTER_CALLS.with(|count| count.set(0));
}

#[cfg(test)]
fn diagnostic_object_formatter_calls() -> usize {
    DIAGNOSTIC_OBJECT_FORMATTER_CALLS.with(Cell::get)
}

#[cfg(test)]
fn reset_diagnostic_frame_string_allocations() {
    DIAGNOSTIC_FRAME_STRING_ALLOCATIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn diagnostic_frame_string_allocations() -> usize {
    DIAGNOSTIC_FRAME_STRING_ALLOCATIONS.with(Cell::get)
}

#[cfg(test)]
fn reset_runtime_container_registration_traversals() {
    RUNTIME_CONTAINER_REGISTRATION_TRAVERSALS.with(|count| count.set(0));
}

#[cfg(test)]
fn runtime_container_registration_traversals() -> usize {
    RUNTIME_CONTAINER_REGISTRATION_TRAVERSALS.with(Cell::get)
}

#[cfg(test)]
fn reset_generic_host_resolutions() {
    GENERIC_HOST_RESOLUTIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn generic_host_resolutions() -> usize {
    GENERIC_HOST_RESOLUTIONS.with(Cell::get)
}

#[cfg(test)]
fn reset_direct_binding_allocations() {
    DIRECT_BINDING_ALLOCATIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn direct_binding_allocations() -> usize {
    DIRECT_BINDING_ALLOCATIONS.with(Cell::get)
}

#[cfg(test)]
fn reset_nested_generic_script_resolutions() {
    NESTED_GENERIC_SCRIPT_RESOLUTIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn nested_generic_script_resolutions() -> usize {
    NESTED_GENERIC_SCRIPT_RESOLUTIONS.with(Cell::get)
}

struct ValueStackReservation {
    count: usize,
}

impl ValueStackReservation {
    fn empty() -> Self {
        Self { count: 0 }
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
        Self::check(count)?;
        VALUE_STACK_SIZE.with(|size| size.set(size.get() + count));
        self.count += count;
        Ok(())
    }

    fn shrink(&mut self, count: usize) {
        debug_assert!(self.count >= count);
        VALUE_STACK_SIZE.with(|size| {
            debug_assert!(size.get() >= count);
            size.set(size.get().saturating_sub(count));
        });
        self.count = self.count.saturating_sub(count);
    }
}

impl Drop for ValueStackReservation {
    fn drop(&mut self) {
        VALUE_STACK_SIZE.with(|size| {
            debug_assert!(size.get() >= self.count);
            size.set(size.get().saturating_sub(self.count));
        });
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
/// script-call level of this tree-walking interpreter uses several KiB of native
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

pub fn value_cell(value: Value) -> ValueCell {
    let cell = Rc::new(RefCell::new(value));
    ACTIVE_OBJECT_REFERENCE_CELLS.with(|frames| {
        // A cell created by a nested call can escape into a persistent global
        // table. Register it with the outermost frame, which spans the whole
        // re-entrant execution and is the last frame to leave.
        if let Some(frame) = frames.borrow_mut().first_mut() {
            frame.push(Rc::downgrade(&cell));
        }
    });
    cell
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
        ACTIVE_OBJECT_REFERENCE_CELLS.with(|frames| {
            let frames = frames.borrow();
            let mut seen = std::collections::HashSet::new();
            for weak in frames.iter().flat_map(|frame| frame.iter()) {
                let Some(cell) = weak.upgrade() else {
                    continue;
                };
                if seen.insert(Rc::as_ptr(&cell)) {
                    sweep.clear_value(&mut cell.borrow_mut());
                }
            }
        });
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
    if c4_set_copy_is_zero_id(&value) {
        map.assign_key_zero_c4id(key);
    } else {
        map.assign_key(key, value);
    }
}

fn c4_map_assign_property_set(map: &mut ValueMap, key: String, value: Value) {
    if c4_set_copy_is_zero_id(&value) {
        map.assign_zero_c4id(key);
    } else {
        map.assign(key, value);
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

    fn collect_object_reference_cells(&self, cells: &mut Vec<ValueCell>) {
        match self {
            Self::Direct { value, .. } => cells.push(value.clone()),
            Self::Inline(inline) => {
                if let Some((value, _)) = inline.promoted.get() {
                    cells.push(value.clone());
                } else if inline.initial.value.contains_any_object_reference() {
                    cells.push(inline.cells().0.clone());
                }
            }
            Self::Reference(reference) => reference.collect_object_reference_cells(cells),
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

    fn read(&self) -> Result<Value, RuntimeError> {
        self.read_tracked().map(|tracked| tracked.value)
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
        self.0
    }
}

impl LValueRef {
    fn collect_object_reference_cells(&self, cells: &mut Vec<ValueCell>) {
        match self {
            Self::Cell { value, .. } => cells.push(value.clone()),
            Self::Path { root, .. } => cells.push(root.clone()),
            Self::HostPath { .. } => {}
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
                write_path(&mut root.borrow_mut(), segments, value)?;
                if let Some(identity) = root_identity {
                    let next_identity = {
                        let current = identity.borrow().clone();
                        RawIdentity::after_path_write(
                            current.as_ref(),
                            &root.borrow(),
                            segments,
                            replacement_identity,
                        )
                    };
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
    fn enter(cells: Vec<ValueCell>) -> Self {
        let outermost = ACTIVE_OBJECT_REFERENCE_CELLS.with(|frames| {
            let mut frames = frames.borrow_mut();
            let outermost = frames.is_empty();
            frames.push(cells.iter().map(Rc::downgrade).collect());
            outermost
        });
        if outermost {
            ACTIVE_OBJECT_REFERENCE_SWEEPS.with(|sweeps| sweeps.borrow_mut().clear());
        }
        Self { outermost }
    }
}

impl Drop for ActiveObjectReferenceCellsGuard {
    fn drop(&mut self) {
        ACTIVE_OBJECT_REFERENCE_CELLS.with(|frames| {
            frames.borrow_mut().pop();
        });
        if self.outermost {
            ACTIVE_OBJECT_REFERENCE_SWEEPS.with(|sweeps| sweeps.borrow_mut().clear());
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
    let mut current = value.clone();
    for segment in segments {
        current = match (segment, current) {
            (PathSegment::Property(property), Value::Proplist(entries)) => {
                entries.get(property).cloned().unwrap_or(Value::Nil)
            }
            (PathSegment::Property(property), other) => {
                return Err(RuntimeError::new(format!(
                    "cannot access property '{property}' on value of type {}",
                    other.type_name()
                )))
            }
            (PathSegment::Index(index), Value::Array(elements)) => elements
                .get(array_index(index)?)
                .cloned()
                .unwrap_or(Value::Nil),
            (PathSegment::Index(index), Value::String(text)) => string_index(&text, index)?,
            (PathSegment::Index(key), Value::Proplist(entries)) => {
                entries.get_key(key).cloned().unwrap_or(Value::Nil)
            }
            (PathSegment::Index(_), other) => {
                return Err(RuntimeError::new(format!(
                    "cannot index into value of type {}",
                    other.type_name()
                )))
            }
        };
    }
    Ok(current)
}

fn write_path(
    value: &mut Value,
    segments: &[PathSegment],
    new_value: Value,
) -> Result<(), RuntimeError> {
    let Some((segment, rest)) = segments.split_first() else {
        let same_destination = c4_set_copy_is_zero_id(&new_value) && c4_set_copy_is_zero_id(value);
        *value = c4_set_copy_value_into(new_value, same_destination);
        return Ok(());
    };

    match (value, segment) {
        (Value::Proplist(entries), PathSegment::Property(property)) => {
            if rest.is_empty() {
                c4_map_assign_property_set(entries, property.clone(), new_value);
                Ok(())
            } else {
                let Some(next) = entries.get_mut(property) else {
                    return Err(RuntimeError::new(format!(
                        "cannot access property '{property}' on nil"
                    )));
                };
                write_path(next, rest, new_value)
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
                elements[index] = c4_set_copy_value_into(new_value, same_destination);
                Ok(())
            } else {
                write_path(&mut elements[index], rest, new_value)
            }
        }
        (Value::Proplist(entries), PathSegment::Index(key)) => {
            if rest.is_empty() {
                c4_map_assign_set(entries, key.clone(), new_value);
                Ok(())
            } else {
                let Some(next) = entries.get_key_mut(key) else {
                    return Err(RuntimeError::new(format!(
                        "cannot access map key {key} on nil"
                    )));
                };
                write_path(next, rest, new_value)
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

    fn clear_object_reference_sweeps(&mut self, cursor: usize) {
        if let Self::Value(tracked) = self {
            clear_value_for_object_reference_sweeps(&mut tracked.value, cursor);
        }
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

fn c4_effective_nil(value: &Value) -> bool {
    matches!(value, Value::Nil | Value::Object(0))
}

fn c4_operator_equal(left: &Value, right: &Value) -> bool {
    if c4_effective_nil(left) {
        // Null object constructors collapse to C4V_Any in C++. A C4ID zero
        // that reaches this comparator retained its C4V_C4ID tag and must use
        // the asymmetric type table below (notably, neither operand order
        // compares equal to C4V_Bool(false)).
        return c4_scalar_payload(right) == Some(0);
    }
    if c4_effective_nil(right) {
        return c4_scalar_payload(left) == Some(0);
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
                .iter()
                .find(|(right_key, _)| c4_typed_equal(left_key, right_key))
                // C4ValueHash::operator== spells this `other[key] != value`,
                // so the other map's value is the asymmetric operator lhs.
                .is_some_and(|(_, right_value)| c4_operator_equal(right_value, left_value))
        })
}

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

    fn into_tracked_on_stack(self) -> Result<TrackedValue, RuntimeError> {
        let _result_slot = ValueStackReservation::reserve(1)?;
        self.into_tracked()
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
    /// References returned from a global callee may outlive its temporary
    /// null Obj/Def context. Lazy host-backed references must recreate it.
    retain_global_call_context_for_host_paths: bool,
    /// The engine-global `static` table (GlobalNamed); resolved after
    /// locals, before global constants (C4AulParse.cpp:2836-2839).
    globals_named: Option<&'a std::cell::RefCell<IndexMap<String, ValueCell>>>,
    /// The engine-global numbered-variable table (`C4AulScriptEngine::Global`).
    globals_numbered: Option<&'a std::cell::RefCell<BTreeMap<i32, ValueCell>>>,
    /// The engine-global `static const` registry (GetGlobalConstant,
    /// C4Aul.cpp:494): script-declared constants shared across hosts,
    /// resolvable via the pre-#strict-2 `NAME()` call idiom.
    globals_consts: Option<&'a std::cell::RefCell<IndexMap<String, ValueCell>>>,
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

    pub fn with_global_variables(
        mut self,
        table: Option<&'a std::cell::RefCell<IndexMap<String, ValueCell>>>,
    ) -> Self {
        self.globals_named = table;
        self
    }

    pub fn with_global_slots(
        mut self,
        table: Option<&'a std::cell::RefCell<BTreeMap<i32, ValueCell>>>,
    ) -> Self {
        self.globals_numbered = table;
        self
    }

    /// Attach the engine-global `static const` registry (GetGlobalConstant,
    /// C4Aul.cpp:494) consulted by the old-style constant-call idiom.
    pub fn with_global_constants(
        mut self,
        table: Option<&'a std::cell::RefCell<IndexMap<String, ValueCell>>>,
    ) -> Self {
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
        self.constants
            .and_then(|constants| constants.get(name).cloned())
            .map(|value| {
                Self::fold_legacy_zero_tracked(self.tracked_constant(name, value), env.strict_level)
            })
            .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'")))
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

    fn evaluate_global_slot(
        &self,
        args: &[Expr],
        env: &mut Environment,
        depth: usize,
    ) -> Result<ValueCell, RuntimeError> {
        // All supplied arguments evaluate before an engine call, even though
        // FnGlobal consumes only the first C4Aul parameter slot.
        let values = self.build_call_args(None, None, args, env, depth)?;
        let _parameter_slot = ValueStackReservation::reserve(1)?;
        let index = match values
            .first()
            .map(CallArg::read)
            .transpose()?
            .unwrap_or(Value::Nil)
        {
            Value::Int(index) => index,
            Value::Bool(flag) => i32::from(flag),
            Value::RawBool(raw) => raw as u32 as i32,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "call to \"Global\" parameter 1: got \"{}\", but expected \"int\"!",
                    other.type_name()
                )))
            }
        };
        self.numbered_global_cell(index)
    }

    /// FnGlobalN's GlobalNamed lookup (C4Script.cpp:4607-4617). The name
    /// must already have been registered by a `static` declaration; a miss
    /// returns nil rather than creating a new global.
    fn evaluate_named_global(
        &self,
        args: &[Expr],
        env: &mut Environment,
        depth: usize,
    ) -> Result<Option<ValueCell>, RuntimeError> {
        let values = self.build_call_args(None, None, args, env, depth)?;
        let _parameter_slot = ValueStackReservation::reserve(1)?;
        let value = values
            .first()
            .map(CallArg::read)
            .transpose()?
            .unwrap_or(Value::Nil);
        let name = match value {
            Value::String(name) => name.into_string(),
            Value::Nil => String::new(),
            Value::Int(0) | Value::Bool(false) | Value::RawBool(0)
                if env.strict_level.unwrap_or(0) < 3 =>
            {
                String::new()
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "call to \"GlobalN\" parameter 1: got \"{}\", but expected \"string\"!",
                    other.type_name()
                )))
            }
        };
        Ok(self.global_variable_cell(&name))
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::external).collect();
        self.invoke_value(name, args, 0, ObjectState::default(), None)
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
        let mut env = Environment::new_with_params(&[], &[], strict_level, object_state.clone())?;
        env.temporary_script = true;
        env.definition_context = matches!(&self.this_value, Value::Object(id) if *id != 0);
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }
        let _object_reference_cells =
            ActiveObjectReferenceCellsGuard::enter(env.object_reference_cells(self));
        let value = self.evaluate(&expr, &mut env, 0)?;
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
        let mut env = Environment::new_with_params(&[], &[], strict_level, cells.state.clone())?;
        env.temporary_script = true;
        env.definition_context = matches!(&self.this_value, Value::Object(id) if *id != 0);
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }
        let _object_reference_cells =
            ActiveObjectReferenceCellsGuard::enter(env.object_reference_cells(self));
        let value = self.evaluate(&expr, &mut env, 0)?;
        if let Some(diagnostic) = &mut diagnostic {
            diagnostic.returned(&value);
        }
        Ok(value)
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
        let _object_reference_cells =
            ActiveObjectReferenceCellsGuard::enter(env.object_reference_cells(self));
        let value = self.evaluate(&expr, &mut env, depth)?;
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

    fn invoke_tracked_value(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<TrackedValue, RuntimeError> {
        self.invoke_raw(name, args, depth, object_state, caller)?
            .into_tracked_on_stack()
    }

    fn invoke_engine_value(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_engine_tracked_value(name, args, depth, object_state, caller)
            .map(|tracked| tracked.value)
    }

    fn invoke_engine_tracked_value(
        &self,
        name: &str,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<TrackedValue, RuntimeError> {
        self.invoke_engine_raw(name, args, depth, object_state, caller)?
            .into_tracked_on_stack()
    }

    fn invoke_engine_raw(
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
            if let Some(function) = self.engine_script_function(name) {
                #[cfg(test)]
                if caller.is_some() {
                    NESTED_GENERIC_SCRIPT_RESOLUTIONS.with(|count| count.set(count.get() + 1));
                }
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
                    object_state,
                    caller.clone(),
                );
            }

            if name == "VarN" && !self.has_host_function(name) {
                let parameter_slots = take_call_parameter_slots(1);
                let _value_stack = ValueStackReservation::reserve(parameter_slots)?;
                return self.invoke_varn_raw(&args, caller.as_ref());
            }

            if let Some(function) = self.host_functions.get(name) {
                #[cfg(test)]
                GENERIC_HOST_RESOLUTIONS.with(|count| count.set(count.get() + 1));
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
        self.global_functions
            .map_or_else(|| self.functions.get(name), |functions| functions.get(name))
    }

    /// Named functions visible in the destination script's own scope.
    /// A C4Aul `global func` leaves only an unnamed FnLink in its declaring
    /// host, so every named lookup skips global nodes and falls through to
    /// the engine table. Ordinary same-name local functions still win. A
    /// bare/partial fixture VM with no table entry retains its only global.
    fn own_script_function(&self, name: &str) -> Option<&Function> {
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

    fn invoke_resolved_script_tracked_value(
        &self,
        name: &str,
        target: ScriptFunctionTarget<'_>,
        args: CallArgs,
        depth: usize,
        object_state: ObjectState,
        caller: Option<ScriptCallerContext>,
    ) -> Result<TrackedValue, RuntimeError> {
        self.invoke_resolved_script_raw(name, target, args, depth, object_state, caller)?
            .into_tracked_on_stack()
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
        let compiled = function
            .compiled
            .get_or_init(|| CompiledFunctionCache::new(function))
            .validated(function, target.validate_compiled_source);
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
        if let Some(compiled) = compiled {
            for name in &compiled.function_vars {
                env.declare_hoisted(name);
            }
        } else {
            hoist_function_vars(&function.body, &mut env);
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
        let cached_diagnostic_strings = compiled.filter(|compiled| {
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
        let _object_reference_cells =
            ActiveObjectReferenceCellsGuard::enter(env.object_reference_cells(self));

        let result = if let Some(compiled) = compiled {
            match compiled.execute(self, &env, depth)? {
                Some(result) => {
                    #[cfg(test)]
                    COMPILED_FUNCTION_EXECUTIONS.with(|count| count.set(count.get() + 1));
                    result
                }
                None => self.execute_statements(
                    &function.body,
                    &mut env,
                    depth,
                    function.returns_reference,
                )?,
            }
        } else {
            self.execute_statements(&function.body, &mut env, depth, function.returns_reference)?
        };
        let value = match result {
            ControlFlow::Return(v) => v,
            ControlFlow::Normal => ReturnValue::Value(TrackedValue::runtime(Value::Nil)),
            ControlFlow::Break | ControlFlow::LoopContinue => {
                return Err(RuntimeError::new(format!(
                    "{} statement outside of loop",
                    if matches!(result, ControlFlow::Break) {
                        "break"
                    } else {
                        "continue"
                    }
                )));
            }
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
        self.host_reference_functions
            .and_then(|functions| functions.get(name))
    }

    fn has_host_function(&self, name: &str) -> bool {
        self.host_functions.contains_key(name) || self.host_reference_function(name).is_some()
    }

    fn resolved_host_function(&self, name: &str) -> Option<ResolvedHostFunction<'_>> {
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

    fn invoke_resolved_host_tracked_value(
        &self,
        name: &str,
        function: ResolvedHostFunction<'_>,
        args: CallArgs,
        depth: usize,
        caller: Option<ScriptCallerContext>,
    ) -> Result<TrackedValue, RuntimeError> {
        self.invoke_resolved_host_raw(name, function, args, depth, caller)?
            .into_tracked_on_stack()
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
        let result = self.invoke_host_function_raw(name, function, &values)?;
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

        let result = function.call(&args)?;

        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &result);
            }
        }
        Ok(materialize_internal_native_call_result(result, &call_args))
    }

    fn execute_statements(
        &self,
        statements: &[Stmt],
        env: &mut Environment,
        depth: usize,
        returns_reference: bool,
    ) -> Result<ControlFlow, RuntimeError> {
        for statement in statements {
            match self.execute_statement(statement, env, depth, returns_reference)? {
                ControlFlow::Normal => continue,
                other => return Ok(other),
            }
        }
        Ok(ControlFlow::Normal)
    }

    fn execute_statement(
        &self,
        statement: &Stmt,
        env: &mut Environment,
        depth: usize,
        returns_reference: bool,
    ) -> Result<ControlFlow, RuntimeError> {
        match statement {
            Stmt::ParseError {
                message,
                line,
                column,
            } => Err(RuntimeError::new(format!(
                "parse error at {line}:{column}: {message}"
            ))),
            Stmt::VarDecl { name, init } => {
                let tracked = match init {
                    Some(expr) => self.evaluate_tracked(expr, env, depth)?,
                    None => TrackedValue::runtime(Value::Nil),
                };
                // Vars are FUNCTION-scoped in C4Aul: the hoisted slot
                // (declared at function entry) receives the value — a
                // `var` inside a block must not shadow it. Address the
                // function-var table directly because a same-name parameter
                // remains the bare-name binding but has a distinct slot.
                env.assign_function_var_tracked(name, tracked)?;
                Ok(ControlFlow::Normal)
            }
            Stmt::Assignment { target, value } => {
                self.evaluate_assignment(target, value, env, depth)?;
                Ok(ControlFlow::Normal)
            }
            Stmt::LegacyGoto { call, expression } => {
                // C4Aul checks parameters, function/object locals, statics and
                // constants before entering its direct-function/goto branch.
                let goto_is_bound = env.lvalue("goto").is_some()
                    || self.global_variable_cell("goto").is_some()
                    || self.global_constant_cell("goto").is_some();
                if env.strict_level.is_none() && !goto_is_bound {
                    return Ok(ControlFlow::Return(self.evaluate_return_value(
                        Some(call),
                        env,
                        depth,
                        returns_reference,
                    )?));
                }
                self.evaluate(expression, env, depth)?;
                Ok(ControlFlow::Normal)
            }
            Stmt::Return(expr) => Ok(ControlFlow::Return(self.evaluate_return_value(
                expr.as_ref(),
                env,
                depth,
                returns_reference,
            )?)),
            Stmt::Break => Ok(ControlFlow::Break),
            Stmt::Continue => Ok(ControlFlow::LoopContinue),
            Stmt::Expr(expr) => {
                self.evaluate(expr, env, depth)?;
                Ok(ControlFlow::Normal)
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.evaluate(condition, env, depth)?.as_bool() {
                    return self.execute_block(then_branch, env, depth, returns_reference);
                } else if let Some(branch) = else_branch {
                    return self.execute_block(branch, env, depth, returns_reference);
                }
                Ok(ControlFlow::Normal)
            }
            Stmt::While { condition, body } => {
                while self.evaluate(condition, env, depth)?.as_bool() {
                    match self.execute_block(body, env, depth, returns_reference)? {
                        ControlFlow::Normal => {}
                        ControlFlow::LoopContinue => continue,
                        ControlFlow::Break => break,
                        ControlFlow::Return(value) => return Ok(ControlFlow::Return(value)),
                    }
                }
                Ok(ControlFlow::Normal)
            }
            Stmt::For {
                init,
                condition,
                increment,
                body,
            } => {
                // Execute init clause (variables are function-scoped, so no new scope)
                if let Some(init_clause) = init {
                    match init_clause {
                        ForInit::VarDecls(decls) => {
                            for (name, init_expr) in decls {
                                let tracked = match init_expr {
                                    Some(expr) => self.evaluate_tracked(expr, env, depth)?,
                                    None => TrackedValue::runtime(Value::Nil),
                                };
                                env.assign_function_var_tracked(name, tracked)?;
                            }
                        }
                        ForInit::Expr(expr) => {
                            self.evaluate(expr, env, depth)?;
                        }
                    }
                }

                // Loop while condition is true (or forever if no condition)
                loop {
                    // Check condition (defaults to true if not specified)
                    if let Some(cond) = condition {
                        if !self.evaluate(cond, env, depth)?.as_bool() {
                            break;
                        }
                    }

                    // Execute body
                    match self.execute_block(body, env, depth, returns_reference)? {
                        ControlFlow::Normal => {}
                        ControlFlow::LoopContinue => {
                            // Execute increment before continuing
                            if let Some(incr) = increment {
                                self.evaluate(incr, env, depth)?;
                            }
                            continue;
                        }
                        ControlFlow::Break => break,
                        ControlFlow::Return(value) => return Ok(ControlFlow::Return(value)),
                    }

                    // Execute increment
                    if let Some(incr) = increment {
                        self.evaluate(incr, env, depth)?;
                    }
                }
                Ok(ControlFlow::Normal)
            }
            Stmt::ForIn {
                variable,
                value_variable,
                iterable,
                body,
                ..
            } => {
                // C4Aul evaluates the container once and keeps its key order
                // stable for the duration of the loop.
                let iterable_value = self.evaluate(iterable, env, depth)?;
                // AB_FOREACH reserves its cursor metadata before checking the
                // container type. Arrays retain iterable+cursor; maps retain
                // iterable+key+value throughout the body.
                let _foreach_slots =
                    ValueStackReservation::reserve(if value_variable.is_some() { 3 } else { 2 })?;

                let items: Vec<(Value, Option<Value>)> = if value_variable.is_some() {
                    match &iterable_value {
                        Value::Proplist(entries) => entries
                            .iter()
                            .map(|(key, value)| (key.clone(), Some(value.clone())))
                            .collect(),
                        other => {
                            return Err(RuntimeError::new(format!(
                                "for: map expected, but got {}!",
                                other.type_name()
                            )))
                        }
                    }
                } else {
                    match &iterable_value {
                        Value::Array(values) => {
                            values.iter().cloned().map(|value| (value, None)).collect()
                        }
                        other => {
                            return Err(RuntimeError::new(format!(
                                "for: array expected, but got {}!",
                                other.type_name()
                            )))
                        }
                    }
                };
                for (key_or_item, map_value) in items {
                    // Both header spellings use the function-scoped named-var
                    // slots populated by the pre-parser/hoisting pass.
                    env.assign_function_var_tracked(variable, TrackedValue::runtime(key_or_item))?;
                    if let (Some(value_variable), Some(map_value)) = (value_variable, map_value) {
                        env.assign_function_var_tracked(
                            value_variable,
                            TrackedValue::runtime(map_value),
                        )?;
                    }

                    match self.execute_block(body, env, depth, returns_reference)? {
                        ControlFlow::Normal => {}
                        ControlFlow::LoopContinue => continue,
                        ControlFlow::Break => break,
                        ControlFlow::Return(value) => return Ok(ControlFlow::Return(value)),
                    }
                }

                Ok(ControlFlow::Normal)
            }
            Stmt::Block(statements) => {
                self.execute_block(statements, env, depth, returns_reference)
            }
            Stmt::Sequence(statements) => {
                // Execute statements sequentially WITHOUT creating a new scope
                // Used for multi-variable declarations
                self.execute_statements(statements, env, depth, returns_reference)
            }
        }
    }

    fn evaluate_return_value(
        &self,
        expression: Option<&Expr>,
        env: &mut Environment,
        depth: usize,
        returns_reference: bool,
    ) -> Result<ReturnValue, RuntimeError> {
        if returns_reference {
            let expression = expression.ok_or_else(|| {
                RuntimeError::new("reference-returning function must return an lvalue")
            })?;
            let _pin_creation = LegacyPathPinCreationGuard::enter();
            self.evaluate_reference_or_value(expression, env, depth)
        } else {
            Ok(ReturnValue::Value(match expression {
                Some(expression) => self.evaluate_tracked(expression, env, depth)?,
                None => TrackedValue::runtime(Value::Nil),
            }))
        }
    }

    fn global_variable(&self, name: &str) -> Option<Value> {
        self.globals_named
            .and_then(|table| table.borrow().get(name).map(|cell| cell.borrow().clone()))
    }

    fn global_variable_cell(&self, name: &str) -> Option<ValueCell> {
        self.globals_named
            .and_then(|table| table.borrow().get(name).cloned())
    }

    fn has_bound_this(&self, env: &Environment) -> bool {
        env.lvalue("this").is_some() || self.global_variable_cell("this").is_some()
    }

    fn global_constant(&self, name: &str) -> Option<Value> {
        self.global_constant_cell(name)
            .map(|cell| cell.borrow().clone())
    }

    fn global_constant_cell(&self, name: &str) -> Option<ValueCell> {
        self.globals_consts
            .and_then(|table| table.borrow().get(name).cloned())
    }

    fn execute_block(
        &self,
        statements: &[Stmt],
        env: &mut Environment,
        depth: usize,
        returns_reference: bool,
    ) -> Result<ControlFlow, RuntimeError> {
        env.push_scope();
        let result = self.execute_statements(statements, env, depth, returns_reference);
        env.pop_scope();
        result
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

    /// AB_FUNC writes an ordinary direct-call result into its first parameter
    /// slot (or a fresh slot for zero arguments). `Call` has therefore already
    /// applied C4Value::Set, including its identical-value early return. Other
    /// expression forms still need their ordinary SetNoRef/value-stack copy.
    fn direct_value_call_has_materialized_result(&self, expr: &Expr, env: &Environment) -> bool {
        let Expr::Call { callee, .. } = expr else {
            return false;
        };
        let Expr::Variable(name) = callee.as_ref() else {
            return false;
        };
        if matches!(name.as_str(), "inherited" | "_inherited") {
            // Same precedence as the dispatch arm, so the C4Value::Set
            // decision cannot disagree with the function actually called.
            if let Some(function) = self
                .inherited_engine_hop(env)
                .or(env.inherited_target.as_deref())
            {
                return !function.returns_reference;
            }
            return self.has_host_function(&env.function_name);
        }
        if self.call_expression_returns_reference(expr, env) {
            return false;
        }
        let function = if env.engine_scope {
            self.engine_script_function(name)
        } else {
            self.own_or_global_script_function(name)
        };
        function.is_some() || self.has_host_function(name)
    }

    /// `??` and strict-2+ `&&`/`||` are jump regions rather than ordinary
    /// result-producing opcodes. The selected operand remains in its existing
    /// stack slot, so there is no additional C4Value::Set at the outer binary
    /// expression boundary.
    fn is_transparent_short_circuit(&self, expr: &Expr, env: &Environment) -> bool {
        matches!(expr, Expr::Binary(_, BinaryOp::NilCoalescing, _))
            || env.strict_level.unwrap_or(0) >= 2
                && matches!(expr, Expr::Binary(_, BinaryOp::And | BinaryOp::Or, _))
    }

    fn expression_result_skips_set_copy(&self, expr: &Expr, env: &Environment) -> bool {
        self.direct_value_call_has_materialized_result(expr, env)
            || self.is_transparent_short_circuit(expr, env)
    }

    /// SetNoRef rewrites ordinary lvalues to value reads, but it cannot rewrite
    /// the result opcode of a reference-returning call, AB_ARRAY_APPEND, or an
    /// assignment opcode. AB_MAP keys then use GetRefVal plus C4Value's copy
    /// constructor, retaining an exceptional zero-ID tag.
    fn set_no_ref_keeps_reference(&self, expr: &Expr, env: &Environment) -> bool {
        self.is_transparent_short_circuit(expr, env)
            || self.call_expression_returns_reference(expr, env)
            || matches!(expr, Expr::GlobalCall { name, .. } if self.global_call_may_return_reference(name))
            || matches!(
                expr,
                Expr::ArrayAppend(_)
                    | Expr::PreIncrement(_)
                    | Expr::PreDecrement(_)
                    | Expr::Assignment(_, _)
                    | Expr::ArrayAppendAssignment { .. }
                    | Expr::CompoundAssignment { .. }
            )
    }

    fn evaluate_set_no_ref_result(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        if let Expr::Binary(left, operation, right) = expr {
            if self.is_transparent_short_circuit(expr, env) {
                return self.evaluate_short_circuit_raw(left, operation, right, env, depth, false);
            }
        }
        if self.set_no_ref_keeps_reference(expr, env) {
            self.evaluate_reference_or_value(expr, env, depth)
        } else {
            self.evaluate_tracked(expr, env, depth)
                .map(ReturnValue::Value)
        }
    }

    fn evaluate_short_circuit_raw(
        &self,
        left: &Expr,
        operation: &BinaryOp,
        right: &Expr,
        env: &mut Environment,
        depth: usize,
        preserve_rhs_reference: bool,
    ) -> Result<ReturnValue, RuntimeError> {
        let left = self.evaluate_set_no_ref_result(left, env, depth)?;
        let left_value = left.as_value()?;
        let keep_left = match operation {
            BinaryOp::NilCoalescing => !matches!(left_value, Value::Nil),
            BinaryOp::And => !left_value.as_bool(),
            BinaryOp::Or => left_value.as_bool(),
            _ => unreachable!("only transparent short-circuit operators reach this helper"),
        };
        if keep_left {
            Ok(left)
        } else if preserve_rhs_reference {
            self.evaluate_reference_or_value(right, env, depth)
        } else {
            self.evaluate_set_no_ref_result(right, env, depth)
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

    fn evaluate(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        let _pin_creation = LegacyPathPinCreationGuard::suspend();
        let materialized_call = self.expression_result_skips_set_copy(expr, env);
        let value = self.evaluate_inner(expr, env, depth)?;
        let value = if materialized_call {
            value
        } else {
            c4_set_copy_value(value)
        };
        // Every expression leaves one C4Value (or C4Value reference) for its
        // parent opcode. Parents explicitly retain that slot while evaluating
        // later operands; the root statement drops it on return.
        ValueStackReservation::check(1)?;
        self.register_runtime_value(&value);
        Ok(value)
    }

    fn evaluate_legacy_parameter_list(
        &self,
        args: &[Expr],
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        let Some((first, discarded)) = args.split_first() else {
            if !forward_rest {
                return Ok(Value::Nil);
            }
            return env
                .call_args
                .get(env.named_param_count)
                .map(Binding::read)
                .transpose()
                .map(|value| value.unwrap_or(Value::Nil));
        };

        // Even an exact-one legacy condition can contain internal `_R`
        // operations (assignment targets and reference parameters). Its final
        // SetNoRef only converts the expression result, not those lifetimes.
        let _pin_registry = LegacyPathPinRegistryGuard::enter();

        if discarded.is_empty() {
            // With exactly one parameter, Parse_If/Parse_While's SetNoRef
            // rewrites the expression to a value before execution.
            return self.evaluate(first, env, depth);
        }

        // Parse_Params leaves references intact, evaluates every surplus
        // expression, then AB_STACK drops the surplus. Because that stack
        // opcode blocks the later SetNoRef rewrite, a first lvalue must stay
        // live until all later side effects finish.
        let first = {
            let _pin_creation = LegacyPathPinCreationGuard::enter();
            self.evaluate_reference_or_value(first, env, depth)?
        };
        let mut value_stack = ValueStackReservation::reserve(1)?;
        let mut discarded_values = Vec::with_capacity(discarded.len());
        for expression in discarded {
            let value = {
                let _pin_creation = LegacyPathPinCreationGuard::enter();
                self.evaluate_reference_or_value(expression, env, depth)?
            };
            discarded_values.push(value);
            value_stack.grow(1)?;
        }
        // AB_STACK pops surplus results in reverse order before AB_CONDN
        // dereferences the retained first result. Keeping every temporary
        // alive until this point also preserves C++ element-reference COW.
        while let Some(value) = discarded_values.pop() {
            drop(value);
        }
        value_stack.shrink(discarded.len());
        first.into_value()
    }

    fn evaluate_inner(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Literal(literal) => Ok(self.literal_value(literal, env.strict_level)),
            Expr::LegacyParameterList { args, forward_rest } => {
                self.evaluate_legacy_parameter_list(args, *forward_rest, env, depth)
            }
            // `this` yields the object context the call runs on (host-provided),
            // mirroring C4Script's `this` (C4V_C4Object); Nil for global calls.
            Expr::This => Ok(self.this_value.clone()),
            Expr::Variable(name) => match env.get(name)? {
                Some(value) => Ok(value),
                // Engine-global statics (GlobalNamed) resolve next; script
                // constants last ("global constants have lowest priority",
                // C4AulParse.cpp:2836-2839). `this` is the context-function
                // fallback between mutable variables and constants.
                None => {
                    if let Some(value) = self.global_variable(name) {
                        return Ok(value);
                    }
                    if name == "this" {
                        return Ok(self.this_value.clone());
                    }
                    if let Some(value) = self.global_constant(name) {
                        return Ok(Self::fold_legacy_zero(value, env.strict_level));
                    }
                    self.constants
                        .and_then(|constants| constants.get(name).cloned())
                        .map(|value| Self::fold_legacy_zero(value, env.strict_level))
                        .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'")))
                }
            },
            Expr::Unary(op, expr) => {
                let value = self.evaluate(expr, env, depth)?;
                self.eval_unary(op, value)
            }
            Expr::Binary(lhs, op, rhs) => {
                let transparent = matches!(op, BinaryOp::NilCoalescing)
                    || env.strict_level.unwrap_or(0) >= 2
                        && matches!(op, BinaryOp::And | BinaryOp::Or);
                if transparent {
                    return Self::materialize_set_no_ref_result(
                        self.evaluate_short_circuit_raw(lhs, op, rhs, env, depth, false)?,
                    )
                    .map(|tracked| tracked.value);
                }
                if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
                    let left = self.evaluate_tracked(lhs, env, depth)?;
                    let _left_slot = ValueStackReservation::reserve(1)?;
                    let right = self.evaluate_tracked(rhs, env, depth)?;
                    let equal = self.values_equal(
                        &left.value,
                        &right.value,
                        env.strict_level,
                        left.identity.as_ref(),
                        right.identity.as_ref(),
                    );
                    return Ok(Value::Bool(if matches!(op, BinaryOp::Equal) {
                        equal
                    } else {
                        !equal
                    }));
                }
                let left = self.evaluate(lhs, env, depth)?;
                // && and || are Lua-style: they return the surviving operand
                // value unchanged, not a coerced bool (C4AulExec.cpp:999-1021,
                // AB_JUMPAND/AB_JUMPOR leave the operand on the stack).
                // Short-circuit && / || exist only at #strict 2
                // (C4AulParse.cpp:3003 gates AB_JUMPAND/AB_JUMPOR on
                // STRICT2). NONSTRICT and #strict scripts run the EAGER
                // AB_And/AB_Or opcodes: both sides always evaluate (their
                // Random draws land on the synced ledger!) and the result
                // coerces to bool (C4AulExec.cpp:733-748).
                if matches!(op, BinaryOp::And) {
                    if env.strict_level.unwrap_or(0) >= 2 {
                        if !left.as_bool() {
                            return Ok(left);
                        }
                        return self.evaluate(rhs, env, depth);
                    }
                    let _left_slot = ValueStackReservation::reserve(1)?;
                    let right = self.evaluate(rhs, env, depth)?;
                    return Ok(Value::Bool(left.as_bool() && right.as_bool()));
                }
                if matches!(op, BinaryOp::Or) {
                    if env.strict_level.unwrap_or(0) >= 2 {
                        if left.as_bool() {
                            return Ok(left);
                        }
                        return self.evaluate(rhs, env, depth);
                    }
                    let _left_slot = ValueStackReservation::reserve(1)?;
                    let right = self.evaluate(rhs, env, depth)?;
                    return Ok(Value::Bool(left.as_bool() || right.as_bool()));
                }
                let _left_slot = ValueStackReservation::reserve(1)?;
                let right = self.evaluate(rhs, env, depth)?;
                self.eval_binary(left, op, right, env.strict_level, None)
            }
            Expr::GlobalCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => self.invoke_global_call(name, args, *failsafe, *forward_rest, env, depth),
            Expr::Call {
                callee,
                args,
                is_optional,
                forward_rest,
            } => {
                // For optional calls (->~Method()), return nil if method doesn't exist
                // instead of throwing an error
                if *is_optional {
                    match callee.as_ref() {
                        Expr::Property(base, name) => self.invoke_property_call(
                            base,
                            name,
                            args,
                            true,
                            *forward_rest,
                            env,
                            depth,
                        ),
                        _ => {
                            // Optional calls only make sense for property access
                            Err(RuntimeError::new(
                                "optional call (~) can only be used with property access (->~Method())".to_string(),
                            ))
                        }
                    }
                } else {
                    // `Var(n)` / `Local(n)` are engine builtins that read numeric
                    // scratch slots (C++ NumVars / object Local), not user
                    // functions — route reads to the same slot accessor as the
                    // lvalue path (clonk-engine registers neither as a host function).
                    if let Expr::Variable(name) = callee.as_ref() {
                        if name == "this" {
                            let function_target =
                                self.resolved_script_function(name, env.engine_scope);
                            let function = function_target.map(|target| target.function);
                            // C4Aul resolves variables before the builtin
                            // context function. A bound `this()` therefore
                            // cannot escape to that function. Without a
                            // binding, every explicit argument still runs
                            // before the zero-arity builtin discards it. This
                            // lookup also precedes old-style constants.
                            if self.has_bound_this(env) {
                                return Err(RuntimeError::new("cannot call bound variable 'this'"));
                            }
                            if function.is_none() && !self.has_host_function(name) {
                                let _ =
                                    self.build_call_args(Some(name), None, args, env, depth + 1)?;
                                return Ok(self.this_value.clone());
                            }
                        }
                        if (name == "Var" || name == "Local")
                            && (args.is_empty() || args.len() == 1)
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            let evaluated_args =
                                self.build_call_args(None, None, args, env, depth + 1)?;
                            let _parameter_slots =
                                ValueStackReservation::reserve(if name == "Var" { 1 } else { 2 })?;
                            let index = Self::slot_index_from_value(
                                if name == "Var" { "Var()" } else { "Local()" },
                                evaluated_args
                                    .first()
                                    .map(CallArg::read)
                                    .transpose()?
                                    .unwrap_or(Value::Nil),
                            )?;
                            if name == "Local" && self.retain_global_call_context_for_host_paths {
                                return Ok(Value::Nil);
                            }
                            let cell = if name == "Var" {
                                frame_slot_cell(&env.frame_locals, index)
                            } else {
                                env.object_state.local_slot_cell(index)
                            };
                            return Ok(cell.borrow().clone());
                        }
                        // `Local(n, pObj)` reads ANOTHER object's numbered
                        // slot through the returned reference (FnLocal,
                        // C4Script.cpp:3423-3433); a negative index is nil.
                        if name == "Local"
                            && args.len() == 2
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            let evaluated_args =
                                self.build_call_args(None, None, args, env, depth + 1)?;
                            let _parameter_slots = ValueStackReservation::reserve(2)?;
                            let index =
                                Self::slot_index_from_value("Local()", evaluated_args[0].read()?)?;
                            if index < 0 {
                                return Ok(Value::Nil);
                            }
                            let target = evaluated_args[1].read()?;
                            let cell = self.numbered_local_cell(env, index, Some(target));
                            let value = cell.borrow().clone();
                            return Ok(value);
                        }
                        // FnSetLocal (C4Script.cpp:3408-3414): writes the
                        // numbered Local slot, returns the value; a nil or
                        // absent object defaults to the executing object.
                        if name == "SetLocal"
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            return self
                                .set_local_tracked(args, None, env, depth + 1, 3)
                                .map(|tracked| tracked.value);
                        }
                        // FnSetGlobal writes the same engine-global numbered
                        // cell returned by Global(index) and returns the value
                        // after native parameter conversion (C4Script.cpp:
                        // 3398-3402).
                        if name == "SetGlobal"
                            && !self.functions.contains_key(name)
                            && !self
                                .global_functions
                                .is_some_and(|functions| functions.contains_key(name))
                            && !self.has_host_function(name)
                        {
                            return self
                                .set_global_tracked(args, *forward_rest, env, depth + 1)
                                .map(|tracked| tracked.value);
                        }
                        // `LocalN("name")` is a reference to the executing
                        // object's named local (FnLocalN, C4Script.cpp:4591-4605,
                        // pObj defaulting to cthr->Obj). The two-argument
                        // cross-object form goes to the host.
                        if name == "LocalN"
                            && (1..=2).contains(&args.len())
                            && !self.functions.contains_key(name)
                        {
                            let evaluated_args =
                                self.build_call_args(None, None, args, env, depth + 1)?;
                            let _parameter_slots = ValueStackReservation::reserve(2)?;
                            let local_name = match evaluated_args[0].read()? {
                                Value::String(local_name) => local_name,
                                other => {
                                    return Err(RuntimeError::new(format!(
                                        "LocalN: expected string for name, got {}",
                                        other.type_name()
                                    )))
                                }
                            };
                            let target = evaluated_args.get(1).map(CallArg::read).transpose()?;
                            if self.retain_global_call_context_for_host_paths
                                && target.as_ref().is_none_or(|value| {
                                    matches!(
                                        value,
                                        Value::Nil
                                            | Value::Int(0)
                                            | Value::Bool(false)
                                            | Value::RawBool(0)
                                            | Value::Object(0)
                                    )
                                })
                            {
                                return Ok(Value::Nil);
                            }
                            let cell = self.localn_cell(env, &local_name, target);
                            let value = cell.borrow().clone();
                            return Ok(value);
                        }
                        // FnGlobal returns a live reference into the one
                        // engine-global numbered table (C4Script.cpp:
                        // 3404-3407). Ordinary calls read that cell; the
                        // assignment/ref-return paths below keep the cell.
                        if name == "Global"
                            && !self.functions.contains_key(name)
                            && !self
                                .global_functions
                                .is_some_and(|functions| functions.contains_key(name))
                            && !self.has_host_function(name)
                        {
                            let cell = self.evaluate_global_slot(args, env, depth + 1)?;
                            let value = cell.borrow().clone();
                            return Ok(value);
                        }
                        // FnGlobalN returns a reference only when the static
                        // name is already registered (C4Script.cpp:4607-4617).
                        if name == "GlobalN"
                            && !self.functions.contains_key(name)
                            && !self
                                .global_functions
                                .is_some_and(|functions| functions.contains_key(name))
                            && !self.has_host_function(name)
                        {
                            let value = self
                                .evaluate_named_global(args, env, depth + 1)?
                                .map(|cell| cell.borrow().clone())
                                .unwrap_or(Value::Nil);
                            return Ok(value);
                        }
                        // FnEval (C4Script.cpp:4507-4520) ->
                        // C4AulScript::DirectExec (C4AulExec.cpp:1658-1707):
                        // parse the string as ONE expression (ParseFn
                        // fExprOnly ignores trailing text) and run it in
                        // the calling object's context with a fresh var
                        // space. This is the ENGINE's script-language eval:
                        // it executes sandboxed C4Script in the same VM as
                        // every other script (no host-language execution) —
                        // the C++ oracle exposes it to content (the planet
                        // Schedule() helper runs on it).
                        if name == "eval"
                            && args.len() <= 1
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            let evaluated_args =
                                self.build_call_args(None, None, args, env, depth + 1)?;
                            let _eval_parameter_slot = ValueStackReservation::reserve(1)?;
                            let code =
                                match evaluated_args.first().map(CallArg::read).transpose()? {
                                    Some(Value::String(code)) => code,
                                    // A null string cannot parse; DirectExec's
                                    // catch yields C4VNull (C4AulExec.cpp:
                                    // 1693-1699).
                                    _ => return Ok(Value::Nil),
                                };
                            let cells = LocalCells {
                                state: env.object_state.clone(),
                            };
                            if let Some(result) = self.eval_direct_exec_hook.and_then(|hook| {
                                hook(
                                    &code,
                                    &cells,
                                    self.this_value.clone(),
                                    env.strict_level,
                                    depth + 1,
                                )
                            }) {
                                return result;
                            }
                            start_direct_exec_profile();
                            let Ok(expr) = crate::parser::Parser::with_strict_level_c4_string(
                                &code,
                                env.strict_level,
                            )
                            .parse_direct_exec_expression() else {
                                // Parse errors log and yield C4VNull
                                // (DirectExec's catch, C4AulExec.cpp:1693).
                                return Ok(Value::Nil);
                            };
                            let mut diagnostic = ScriptDiagnosticGuard::enter_direct(
                                self.eval_direct_exec_diagnostic_frame(env.definition_context),
                                false,
                            );
                            let mut exec_env = Environment::new_with_params(
                                &[],
                                &[],
                                env.strict_level,
                                env.object_state.clone(),
                            )?;
                            exec_env.temporary_script = true;
                            exec_env.definition_context =
                                matches!(&self.this_value, Value::Object(id) if *id != 0);
                            for var_decl in self.var_decls {
                                let cell = exec_env.object_state.named_local_cell(&var_decl.name);
                                exec_env.define_object_local(
                                    &var_decl.name,
                                    self.identity_for_cell(&cell),
                                );
                            }
                            // Runtime errors propagate (fPassErrors=true,
                            // C4Script.cpp:4514).
                            let value = self.evaluate(&expr, &mut exec_env, depth + 1)?;
                            diagnostic.returned(&value);
                            return Ok(value);
                        }
                        // `Par(n)` reads the executing call's parameter slot n;
                        // outside 0..ParCnt it is nil (C4AulExec.cpp:1127-1140).
                        if name == "Par"
                            && args.len() <= 1
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            let evaluated_args =
                                self.build_call_args(None, None, args, env, depth + 1)?;
                            let _parameter_slot = ValueStackReservation::reserve(1)?;
                            let index = evaluated_args
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
                            return usize::try_from(index)
                                .ok()
                                .filter(|index| *index < MAX_CALL_PARAMETERS)
                                .and_then(|index| env.call_args.get(index))
                                .map(Binding::read)
                                .transpose()
                                .map(|value| value.unwrap_or(Value::Nil));
                        }
                    }
                    // Extract function name from callee expression
                    match callee.as_ref() {
                        Expr::Variable(name) if name == "inherited" || name == "_inherited" => {
                            // `inherited` calls the overloaded function; the
                            // `_inherited` spelling yields nil when there is
                            // none (C4AulParse.cpp:2775-2798). The own-owner
                            // list wins; C4Aul's owner hop into the live engine
                            // table supersedes the chain when that list held
                            // nothing.
                            let hop = self
                                .inherited_engine_hop(env)
                                .map(|found| std::sync::Arc::new(found.clone()));
                            let Some(target) = hop.or_else(|| env.inherited_target.clone()) else {
                                let inherited_name = env.function_name.clone();
                                // Script functions overload same-name ENGINE
                                // functions: inherited() chains to the host
                                // fn (C4Aul OwnerOverloaded includes engine
                                // funcs — GoldRush AI.c4d's global
                                // GetOwner/Hostile overrides rely on it).
                                if let Some(host) = self.host_functions.get(&inherited_name) {
                                    let mut evaluated_args = self.build_call_args(
                                        Some(&inherited_name),
                                        None,
                                        args,
                                        env,
                                        depth + 1,
                                    )?;
                                    if *forward_rest {
                                        Self::append_forwarded_args(
                                            &mut evaluated_args,
                                            env,
                                            host.parameter_count().unwrap_or(MAX_CALL_PARAMETERS),
                                        )?;
                                    }
                                    // The overriding script function is the
                                    // host fn's cthr->Caller.
                                    let _guard =
                                        CallerContextGuard::enter(Some(env.caller_context()));
                                    return self.invoke_host_function_call_args(
                                        &env.function_name.clone(),
                                        host,
                                        evaluated_args,
                                    );
                                }
                                if let Some(host) = self.host_reference_function(&inherited_name) {
                                    let mut evaluated_args = self.build_call_args(
                                        Some(&inherited_name),
                                        None,
                                        args,
                                        env,
                                        depth + 1,
                                    )?;
                                    if *forward_rest {
                                        Self::append_forwarded_args(
                                            &mut evaluated_args,
                                            env,
                                            host.parameter_count().unwrap_or(MAX_CALL_PARAMETERS),
                                        )?;
                                    }
                                    let _guard =
                                        CallerContextGuard::enter(Some(env.caller_context()));
                                    return self.invoke_host_reference_function(
                                        &inherited_name,
                                        host,
                                        evaluated_args,
                                    );
                                }
                                return if name == "_inherited" {
                                    // Even the failsafe no-parent path parses
                                    // and evaluates every explicit argument
                                    // before discarding it and pushing nil
                                    // (C4AulParse.cpp:2793-2797).
                                    let _ =
                                        self.build_call_args(None, None, args, env, depth + 1)?;
                                    Ok(Value::Nil)
                                } else {
                                    Err(RuntimeError::new(format!(
                                        "inherited: no overloaded function (in {})",
                                        env.function_name
                                    )))
                                };
                            };
                            let mut evaluated_args = self.build_call_args(
                                Some(&target.name),
                                Some(&target),
                                args,
                                env,
                                depth + 1,
                            )?;
                            if *forward_rest {
                                Self::append_forwarded_args(
                                    &mut evaluated_args,
                                    env,
                                    MAX_CALL_PARAMETERS,
                                )?;
                            }
                            self.invoke_script_function(
                                &target.name.clone(),
                                ScriptFunctionTarget::validated(&target),
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.caller_context()),
                            )?
                            .as_value()
                        }
                        Expr::Variable(name) => {
                            // Old-style constant calls: below #strict 2, a
                            // global constant used as `OCF_Chop()` yields the
                            // constant with the call parens ignored
                            // (C4AulParse.cpp:2838-2860, "old-style usage").
                            // Script `static const`s resolve here too via the
                            // shared registry (GetGlobalConstant) — MagiClonk's
                            // `MCLK_ComboExtraDataName()`.
                            if env.strict_level.unwrap_or(0) < 2
                                && !self.functions.contains_key(name)
                                && !self
                                    .global_functions
                                    .map(|functions| functions.contains_key(name))
                                    .unwrap_or(false)
                                && !self.has_host_function(name)
                            {
                                if let Some(value) = self.global_constant(name).or_else(|| {
                                    self.constants
                                        .and_then(|constants| constants.get(name).cloned())
                                }) {
                                    // C++ requires an immediate ')' after
                                    // the '(' (Match(ATT_BCLOSE),
                                    // C4AulParse.cpp:2860).
                                    if !args.is_empty() {
                                        return Err(RuntimeError::new(
                                            "parameters not allowed in functional usage of constants",
                                        ));
                                    }
                                    return Ok(Self::fold_legacy_zero(value, env.strict_level));
                                }
                            }
                            let function_target =
                                self.resolved_script_function(name, env.engine_scope);
                            let function = function_target.map(|target| target.function);
                            let host_function = function
                                .is_none()
                                .then(|| self.resolved_host_function(name))
                                .flatten();
                            let mut evaluated_args =
                                self.build_call_args(Some(name), function, args, env, depth + 1)?;
                            if *forward_rest {
                                Self::append_forwarded_args(
                                    &mut evaluated_args,
                                    env,
                                    self.direct_call_parameter_limit(name, function),
                                )?;
                            }
                            if let Some(function_target) = function_target {
                                return self.invoke_resolved_script_value(
                                    name,
                                    function_target,
                                    evaluated_args,
                                    depth + 1,
                                    env.object_state.clone(),
                                    Some(env.caller_context()),
                                );
                            }
                            if let Some(host_function) = host_function {
                                return self.invoke_resolved_host_value(
                                    name,
                                    host_function,
                                    evaluated_args,
                                    depth + 1,
                                    Some(env.caller_context()),
                                );
                            }
                            if env.engine_scope {
                                self.invoke_engine_value(
                                    name,
                                    evaluated_args,
                                    depth + 1,
                                    env.object_state.clone(),
                                    Some(env.caller_context()),
                                )
                            } else {
                                self.invoke_value(
                                    name,
                                    evaluated_args,
                                    depth + 1,
                                    env.object_state.clone(),
                                    Some(env.caller_context()),
                                )
                            }
                        }
                        Expr::Property(base, name) => self.invoke_property_call(
                            base,
                            name,
                            args,
                            false,
                            *forward_rest,
                            env,
                            depth,
                        ),
                        _ => Err(RuntimeError::new(format!(
                            "cannot call non-function expression: {:?}",
                            callee
                        ))),
                    }
                }
            }
            Expr::Array(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                let mut value_stack = ValueStackReservation::empty();
                for element in elements {
                    let sweep_cursor = object_reference_sweep_cursor();
                    let value = c4_set_copy_value(self.evaluate(element, env, depth)?);
                    for retained in &mut values {
                        clear_value_for_object_reference_sweeps(retained, sweep_cursor);
                    }
                    values.push(value);
                    value_stack.grow(1)?;
                }
                Ok(Value::Array(values))
            }
            Expr::Proplist(entries) => {
                let mut map = ValueMap::with_capacity(entries.len());
                let mut value_stack = ValueStackReservation::empty();
                for (key_expr, value_expr) in entries {
                    let key = self.evaluate_set_no_ref_result(key_expr, env, depth)?;
                    value_stack.grow(1)?;
                    let key = key.into_value()?;
                    let value = self.evaluate_set_no_ref_result(value_expr, env, depth)?;
                    value_stack.grow(1)?;
                    let value = value.into_value()?;
                    c4_map_assign_set(&mut map, key, value);
                }
                Ok(Value::Proplist(map))
            }
            Expr::Index(_, _) | Expr::ArrayAppend(_) => self
                .evaluate_reference_or_value(expr, env, depth)?
                .into_value_on_stack(),
            Expr::ArrayAppendAssignment {
                target,
                operation,
                operator,
                value,
            } => self
                .evaluate_reference_assignment_tracked(
                    target,
                    operation.as_ref(),
                    operator,
                    value,
                    env,
                    depth,
                )
                .map(|tracked| tracked.value),
            Expr::CompoundAssignment {
                target,
                operation,
                operator,
                value,
            } => self
                .evaluate_reference_assignment_tracked(
                    target,
                    Some(operation),
                    operator,
                    value,
                    env,
                    depth,
                )
                .map(|tracked| tracked.value),
            Expr::Property(target, _) if Self::expression_contains_array_append(target) => self
                .evaluate_reference_or_value(expr, env, depth)?
                .into_value_on_stack(),
            Expr::Property(target, name) => {
                let proplist = self.evaluate(target, env, depth)?;
                let _target_slot = ValueStackReservation::reserve(1)?;
                self.eval_property(proplist, name, env)
            }
            Expr::SafeNavigation { receiver, steps } => self
                .evaluate_safe_navigation_tracked(receiver, steps, env, depth)
                .map(|tracked| tracked.value),
            Expr::Assignment(target, value_expr) => {
                self.evaluate_assignment(target, value_expr, env, depth)
            }
            Expr::PreIncrement(expr) => self.update_counter(expr, env, 1, false, "increment"),
            Expr::PreDecrement(expr) => self.update_counter(expr, env, -1, false, "decrement"),
            Expr::PostIncrement(expr) => self.update_counter(expr, env, 1, true, "increment"),
            Expr::PostDecrement(expr) => self.update_counter(expr, env, -1, true, "decrement"),
        }
    }

    fn evaluate_tracked(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        let _pin_creation = LegacyPathPinCreationGuard::suspend();
        let materialized_call = self.expression_result_skips_set_copy(expr, env);
        let tracked = self.evaluate_tracked_inner(expr, env, depth)?;
        let tracked = if materialized_call {
            tracked
        } else {
            tracked.set_copy()
        };
        ValueStackReservation::check(1)?;
        self.register_runtime_value(&tracked.value);
        Ok(tracked)
    }

    fn evaluate_tracked_inner(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        match expr {
            Expr::Literal(literal) => Ok(TrackedValue::literal(
                self.literal_value(literal, env.strict_level),
                literal,
            )),
            Expr::LegacyParameterList { args, forward_rest } => self
                .evaluate_legacy_parameter_list(args, *forward_rest, env, depth)
                .map(TrackedValue::runtime),
            Expr::Variable(name) => match env.get_tracked(name)? {
                Some(tracked) => Ok(tracked),
                None => {
                    if let Some(cell) = self.global_variable_cell(name) {
                        Ok(self.read_tracked_named_cell(name, &cell))
                    } else if name == "this" {
                        Ok(TrackedValue::runtime(self.this_value.clone()))
                    } else if let Some(cell) = self.global_constant_cell(name) {
                        Ok(Self::fold_legacy_zero_tracked(
                            self.read_tracked_named_cell(name, &cell),
                            env.strict_level,
                        ))
                    } else if let Some(value) = self
                        .constants
                        .and_then(|constants| constants.get(name).cloned())
                    {
                        Ok(Self::fold_legacy_zero_tracked(
                            self.tracked_constant(name, value),
                            env.strict_level,
                        ))
                    } else {
                        self.evaluate(expr, env, depth).map(TrackedValue::runtime)
                    }
                }
            },
            Expr::Array(elements) => {
                let mut tracked: Vec<TrackedValue> = Vec::with_capacity(elements.len());
                let mut value_stack = ValueStackReservation::empty();
                for element in elements {
                    let sweep_cursor = object_reference_sweep_cursor();
                    let value = self.evaluate_tracked(element, env, depth)?.set_copy();
                    for retained in &mut tracked {
                        retained.clear_object_reference_sweeps(sweep_cursor);
                    }
                    tracked.push(value);
                    value_stack.grow(1)?;
                }
                Ok(TrackedValue::array(tracked))
            }
            Expr::Proplist(entries) => {
                let mut tracked = Vec::with_capacity(entries.len());
                let mut value_stack = ValueStackReservation::empty();
                for (key_expr, value_expr) in entries {
                    let key = self.evaluate_set_no_ref_result(key_expr, env, depth)?;
                    value_stack.grow(1)?;
                    let key = key.into_value()?;
                    let value = self.evaluate_set_no_ref_result(value_expr, env, depth)?;
                    value_stack.grow(1)?;
                    let value = value.into_tracked()?;
                    tracked.push((key, value));
                }
                Ok(TrackedValue::proplist(tracked))
            }
            Expr::Index(_, _) | Expr::ArrayAppend(_) => self
                .evaluate_reference_or_value(expr, env, depth)?
                .into_tracked_on_stack(),
            Expr::ArrayAppendAssignment {
                target,
                operation,
                operator,
                value,
            } => self.evaluate_reference_assignment_tracked(
                target,
                operation.as_ref(),
                operator,
                value,
                env,
                depth,
            ),
            Expr::CompoundAssignment {
                target,
                operation,
                operator,
                value,
            } => self.evaluate_reference_assignment_tracked(
                target,
                Some(operation),
                operator,
                value,
                env,
                depth,
            ),
            Expr::Property(target, _) if Self::expression_contains_array_append(target) => self
                .evaluate_reference_or_value(expr, env, depth)?
                .into_tracked_on_stack(),
            Expr::Property(target, name) => {
                let collection = self.evaluate_tracked(target, env, depth)?;
                let _target_slot = ValueStackReservation::reserve(1)?;
                self.eval_property_tracked(collection, name, env)
            }
            Expr::SafeNavigation { receiver, steps } => {
                self.evaluate_safe_navigation_tracked(receiver, steps, env, depth)
            }
            Expr::Binary(left, BinaryOp::Concat, right) => {
                let left = self.evaluate_tracked(left, env, depth)?;
                let _left_slot = ValueStackReservation::reserve(1)?;
                let right = self.evaluate_tracked(right, env, depth)?;
                self.eval_concat_tracked(left, right, env.strict_level, "..")
            }
            Expr::Binary(left, BinaryOp::NilCoalescing, right) => {
                Self::materialize_set_no_ref_result(self.evaluate_short_circuit_raw(
                    left,
                    &BinaryOp::NilCoalescing,
                    right,
                    env,
                    depth,
                    false,
                )?)
            }
            Expr::Binary(left, BinaryOp::And, right) if env.strict_level.unwrap_or(0) >= 2 => {
                Self::materialize_set_no_ref_result(self.evaluate_short_circuit_raw(
                    left,
                    &BinaryOp::And,
                    right,
                    env,
                    depth,
                    false,
                )?)
            }
            Expr::Binary(left, BinaryOp::Or, right) if env.strict_level.unwrap_or(0) >= 2 => {
                Self::materialize_set_no_ref_result(self.evaluate_short_circuit_raw(
                    left,
                    &BinaryOp::Or,
                    right,
                    env,
                    depth,
                    false,
                )?)
            }
            Expr::GlobalCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => self
                .invoke_global_call_raw(name, args, *failsafe, *forward_rest, env, depth)?
                .into_tracked_on_stack(),
            Expr::Call {
                callee,
                args,
                is_optional,
                forward_rest,
            } if !*is_optional => {
                if let Expr::Property(base, name) = callee.as_ref() {
                    let target = self.evaluate(base, env, depth + 1)?;
                    let _target_slot = ValueStackReservation::reserve(1)?;
                    return self
                        .invoke_property_call_with_target(
                            target,
                            name,
                            args,
                            false,
                            *forward_rest,
                            env,
                            depth,
                        )
                        .map(TrackedValue::runtime);
                }
                if let Expr::Variable(name) = callee.as_ref() {
                    let function_target = self.resolved_script_function(name, env.engine_scope);
                    let function = function_target.map(|target| target.function);
                    let host_function = function
                        .is_none()
                        .then(|| self.resolved_host_function(name))
                        .flatten();
                    let bound_context_name = name == "this" && self.has_bound_this(env);
                    if name == "this"
                        && (bound_context_name || function.is_none() && host_function.is_none())
                    {
                        return self.evaluate(expr, env, depth).map(TrackedValue::runtime);
                    }
                    if name == "SetLocal" && function.is_none() && host_function.is_none() {
                        return self.set_local_tracked(args, None, env, depth + 1, 3);
                    }
                    if name == "SetGlobal" && function.is_none() && host_function.is_none() {
                        return self.set_global_tracked(args, *forward_rest, env, depth + 1);
                    }
                    if env.strict_level.unwrap_or(0) < 2
                        && function.is_none()
                        && host_function.is_none()
                        && args.is_empty()
                    {
                        if let Some(cell) = self.global_constant_cell(name) {
                            return Ok(Self::fold_legacy_zero_tracked(
                                self.read_tracked_named_cell(name, &cell),
                                env.strict_level,
                            ));
                        }
                        if let Some(value) = self
                            .constants
                            .and_then(|constants| constants.get(name).cloned())
                        {
                            return Ok(Self::fold_legacy_zero_tracked(
                                self.tracked_constant(name, value),
                                env.strict_level,
                            ));
                        }
                    }
                    let builtin_reference = matches!(name.as_str(), "Var" | "Local")
                        && args.len() <= 1
                        || name == "LocalN" && (1..=2).contains(&args.len())
                        || name == "Global";
                    let null_implicit_local = self.retain_global_call_context_for_host_paths
                        && (name == "Local" && args.len() <= 1
                            || name == "LocalN" && args.len() == 1);
                    if builtin_reference
                        && !null_implicit_local
                        && function.is_none()
                        && host_function.is_none()
                    {
                        return self.expr_to_lvalue(expr, env, depth)?.read_tracked();
                    }
                    if !matches!(name.as_str(), "inherited" | "_inherited")
                        && (function.is_some() || host_function.is_some())
                    {
                        let mut evaluated_args =
                            self.build_call_args(Some(name), function, args, env, depth + 1)?;
                        if *forward_rest {
                            Self::append_forwarded_args(
                                &mut evaluated_args,
                                env,
                                self.direct_call_parameter_limit(name, function),
                            )?;
                        }
                        if let Some(function_target) = function_target {
                            return self.invoke_resolved_script_tracked_value(
                                name,
                                function_target,
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.caller_context()),
                            );
                        }
                        if let Some(host_function) = host_function {
                            return self.invoke_resolved_host_tracked_value(
                                name,
                                host_function,
                                evaluated_args,
                                depth + 1,
                                Some(env.caller_context()),
                            );
                        }
                        return if env.engine_scope {
                            self.invoke_engine_tracked_value(
                                name,
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.caller_context()),
                            )
                        } else {
                            self.invoke_tracked_value(
                                name,
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.caller_context()),
                            )
                        };
                    }
                }
                self.evaluate(expr, env, depth).map(TrackedValue::runtime)
            }
            Expr::Assignment(target, value_expr) => {
                if matches!(target, AssignmentTarget::InvalidValue { .. }) {
                    return self
                        .evaluate_assignment(target, value_expr, env, depth)
                        .map(TrackedValue::runtime);
                }
                self.evaluate_plain_assignment_tracked(target, value_expr, env, depth)
            }
            _ => self.evaluate(expr, env, depth).map(TrackedValue::runtime),
        }
    }

    /// Evaluate strict-3 `?` navigation. A guard applies only at its own
    /// question-mark boundary: once a non-nil value crosses that boundary,
    /// the remaining contiguous `->`/`[]`/`.` suffix executes normally until
    /// another guarded boundary is reached. The returned value deliberately
    /// carries no assignable path, mirroring the final C++ `AB_DEREF`.
    fn evaluate_safe_navigation_tracked(
        &self,
        receiver: &Expr,
        steps: &[SafeNavigationStep],
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        let current = self.evaluate_set_no_ref_result(receiver, env, depth)?;
        // Every navigation opcode rewrites one receiver/result slot in place.
        // Index and method arguments execute while that slot remains live.
        let _current_slot = ValueStackReservation::reserve(1)?;
        let mut current = current.into_tracked()?;

        for step in steps {
            if step.nil_guard && matches!(current.value, Value::Nil) {
                return Ok(TrackedValue::runtime(Value::Nil));
            }

            current = match &step.operation {
                NavigationOperation::Index(index_operand) => {
                    let (index, _index_slot) =
                        self.evaluate_index_operand(index_operand, env, depth)?;
                    self.eval_index_tracked(current, index, env)?.set_copy()
                }
                // SetNoRef before AB_JUMPNIL makes the guarded base a value.
                // AB_ARRAY_APPEND therefore operates on that detached value:
                // it yields nil but does not grow the original array.
                NavigationOperation::ArrayAppend => match current.value {
                    Value::Array(elements) if elements.len() < ARRAY_MAX_SIZE => {
                        TrackedValue::runtime(Value::Nil)
                    }
                    Value::Array(_) => return Err(RuntimeError::new("out of memory")),
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
                },
                NavigationOperation::Property(name) => {
                    self.eval_property_tracked(current, name, env)?.set_copy()
                }
                NavigationOperation::MethodCall {
                    name,
                    args,
                    is_optional,
                    forward_rest,
                } => TrackedValue::runtime(self.invoke_property_call_with_target(
                    current.value,
                    name,
                    args,
                    *is_optional,
                    *forward_rest,
                    env,
                    depth,
                )?),
            };
        }

        Ok(current)
    }

    fn evaluate_reference_assignment_tracked(
        &self,
        target: &AssignmentTarget,
        operation: Option<&BinaryOp>,
        operator: &str,
        value: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        self.evaluate_reference_assignment_raw(
            target,
            AssignmentOperator {
                operation,
                spelling: operator,
            },
            value,
            env,
            depth,
            false,
        )?
        .into_tracked_on_stack()
    }

    fn evaluate_reference_assignment_raw(
        &self,
        target: &AssignmentTarget,
        assignment_operator: AssignmentOperator<'_>,
        value: &Expr,
        env: &mut Environment,
        depth: usize,
        preserve_reference: bool,
    ) -> Result<ReturnValue, RuntimeError> {
        let AssignmentOperator {
            operation,
            spelling: operator,
        } = assignment_operator;
        // C++ evaluates an assignment target into one reference before its
        // RHS. Compound bytecodes read and mutate that retained reference;
        // re-evaluating the target would repeat address-side effects.
        let target = {
            // Even inside a value-producing expression, the assignment's
            // left operand is compiled as `_R` and must survive its RHS.
            let _pin_creation = LegacyPathPinCreationGuard::enter();
            self.assignment_target_to_reference_or_value(env, target, depth)?
        };
        let _target_slot = ValueStackReservation::reserve(1)?;
        let reference = match target {
            ReturnValue::Reference(reference) => reference,
            ReturnValue::Value(left) => {
                if matches!(operation, Some(BinaryOp::NilCoalescing))
                    && !matches!(left.value, Value::Nil)
                {
                    return Ok(ReturnValue::Value(left));
                }
                self.evaluate_tracked(value, env, depth)?;
                let expected = if operation.is_some()
                    && !matches!(operation, Some(BinaryOp::Concat | BinaryOp::NilCoalescing))
                {
                    "int&"
                } else {
                    "&"
                };
                return Err(RuntimeError::new(format!(
                    "operator \"{operator}\" left side: got \"{}\", but expected \"{expected}\"!",
                    Self::c4v_type_name(left.value.c4v_type())
                )));
            }
        };
        let invalidated_error = |left: TrackedValue| {
            let expected = if operation.is_some()
                && !matches!(operation, Some(BinaryOp::Concat | BinaryOp::NilCoalescing))
            {
                "int&"
            } else {
                "&"
            };
            RuntimeError::new(format!(
                "operator \"{operator}\" left side: got \"{}\", but expected \"{expected}\"!",
                Self::c4v_type_name(left.value.c4v_type())
            ))
        };
        let mut right_slot = ValueStackReservation::empty();
        let result = if matches!(operation, Some(BinaryOp::NilCoalescing)) {
            let left = reference.read_tracked()?;
            if !matches!(left.value, Value::Nil) {
                // AB_NilCoalescingIt jumps over both the RHS and AB_Set.
                return Ok(if preserve_reference {
                    ReturnValue::Reference(reference)
                } else {
                    ReturnValue::Value(left)
                });
            }
            let right = self.evaluate_tracked(value, env, depth)?;
            right_slot.grow(1)?;
            if let Some(left) = reference.resolved_legacy_value() {
                return Err(invalidated_error(left));
            }
            right
        } else if let Some(operation) = operation {
            // The RHS runs while the reference is live. Read only afterward:
            // it may have changed the referenced slot before AB_*It executes.
            let right = self.evaluate_tracked(value, env, depth)?;
            right_slot.grow(1)?;
            if let Some(left) = reference.resolved_legacy_value() {
                return Err(invalidated_error(left));
            }
            let left = reference.read_tracked()?;
            if matches!(operation, BinaryOp::Concat) {
                self.eval_concat_tracked(left, right, env.strict_level, operator)?
            } else {
                TrackedValue::runtime(self.eval_binary(
                    left.value,
                    operation,
                    right.value,
                    env.strict_level,
                    Some(operator),
                )?)
            }
        } else {
            // Path references validate lazily in Rust. Force the completed
            // append target now so a nested nil access errors before the RHS,
            // as AB_ARRAYA_R does while evaluating the target.
            reference.read_tracked()?;
            let right = self.evaluate_tracked(value, env, depth)?;
            right_slot.grow(1)?;
            if let Some(left) = reference.resolved_legacy_value() {
                return Err(invalidated_error(left));
            }
            right
        };
        reference.write_tracked(result.clone())?;
        Ok(if preserve_reference {
            ReturnValue::Reference(reference)
        } else {
            ReturnValue::Value(result)
        })
    }

    /// Resolve an increment/decrement operand to its C4Value reference once,
    /// then read and mutate that reference. C++'s AB_Inc1/AB_Dec1 bytecodes
    /// receive one already-evaluated reference on the value stack
    /// (C4AulExec.cpp:450-487); evaluating the lvalue again for the write
    /// repeats side effects in expressions such as `++Var(i++)`.
    fn update_counter(
        &self,
        expr: &Expr,
        env: &mut Environment,
        delta: i32,
        return_old: bool,
        operation: &str,
    ) -> Result<Value, RuntimeError> {
        self.update_counter_raw(expr, env, delta, return_old, operation)?
            .into_value_on_stack()
    }

    fn update_counter_raw(
        &self,
        expr: &Expr,
        env: &mut Environment,
        delta: i32,
        return_old: bool,
        operation: &str,
    ) -> Result<ReturnValue, RuntimeError> {
        let reference = if Self::expression_contains_array_append(expr) {
            match self.evaluate_reference_or_value(expr, env, 0)? {
                ReturnValue::Reference(reference) => reference,
                ReturnValue::Value(value) => {
                    let operator = if delta > 0 { "++" } else { "--" };
                    return Err(RuntimeError::new(format!(
                        "operator \"{operator}\": got \"{}\", but expected \"int&\"!",
                        Self::c4v_type_name(value.value.c4v_type())
                    )));
                }
            }
        } else {
            let target = Self::expr_to_assignment_target(expr)?;
            let _pin_creation = LegacyPathPinCreationGuard::enter();
            match self.assignment_target_to_reference_or_value(env, &target, 0)? {
                ReturnValue::Reference(reference) => reference,
                ReturnValue::Value(value) => {
                    let operator = if delta > 0 { "++" } else { "--" };
                    return Err(RuntimeError::new(format!(
                        "operator \"{operator}\": got \"{}\", but expected \"int&\"!",
                        Self::c4v_type_name(value.value.c4v_type())
                    )));
                }
            }
        };
        let _operand_slot = ValueStackReservation::reserve(1)?;
        let old_value = Self::counter_operand(reference.read()?, operation)?;
        let new_value = old_value.wrapping_add(delta);
        reference.write(Value::Int(new_value))?;
        Ok(if return_old {
            ReturnValue::Value(TrackedValue::runtime(Value::Int(old_value)))
        } else {
            // Prefix AB_Inc1/AB_Dec1 mutates through the stack reference and
            // leaves that same reference in place. Postfix explicitly turns
            // it into the old integer value instead.
            ReturnValue::Reference(reference)
        })
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

    fn global_call_may_return_reference(&self, name: &str) -> bool {
        self.engine_global_script_function(name)
            .map(|function| function.returns_reference)
            .unwrap_or_else(|| {
                name == "EffectVar"
                    || !self.has_host_function(name)
                        && matches!(
                            name,
                            "Var" | "VarN" | "Local" | "LocalN" | "Global" | "GlobalN"
                        )
            })
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
                let cells = LocalCells::default();
                if let Some(result) = self
                    .eval_direct_exec_hook
                    .and_then(|hook| hook(&code, &cells, Value::Nil, env.strict_level, depth + 1))
                {
                    return result.map(|value| ReturnValue::Value(TrackedValue::runtime(value)));
                }
                start_direct_exec_profile();
                let Ok(expr) =
                    crate::parser::Parser::with_strict_level_c4_string(&code, env.strict_level)
                        .parse_direct_exec_expression()
                else {
                    return Ok(value(Value::Nil));
                };
                let mut diagnostic = ScriptDiagnosticGuard::enter_direct(
                    DirectExecDiagnosticFrame::new(
                        format!(
                            "eval in {}",
                            self.game_script_name.unwrap_or(self.script_name)
                        ),
                        None,
                    ),
                    false,
                );
                let mut exec_env = Environment::new_with_params(
                    &[],
                    &[],
                    env.strict_level,
                    ObjectState::default(),
                )?;
                exec_env.engine_scope = true;
                exec_env.temporary_script = true;
                let tracked = self.evaluate_tracked(&expr, &mut exec_env, depth + 1)?;
                diagnostic.returned(&tracked.value);
                Ok(ReturnValue::Value(tracked))
            }
            _ => Err(RuntimeError::new(format!("unknown function '{name}'"))),
        }
    }

    /// Strict-3 `global->Fn(args)`: arguments belong to the suspended caller
    /// and therefore evaluate before AB_CALLGLOBAL clears Obj/Def. The raw
    /// return preserves `func &` and native reference results.
    #[allow(clippy::too_many_arguments)]
    fn invoke_global_call_raw(
        &self,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        let function = self.engine_global_script_function(name);
        let vm_builtin =
            function.is_none() && !self.has_host_function(name) && Self::is_global_vm_builtin(name);
        let known = function.is_some() || self.has_host_function(name) || vm_builtin;
        if !known {
            if !failsafe {
                return Err(RuntimeError::new(format!("unknown function '{name}'")));
            }
            // Parse_Params(0, nullptr) still evaluates every explicit
            // argument, but a missing failsafe call forwards no `...` slots.
            let _target_slot = ValueStackReservation::reserve(1)?;
            let _ = self.build_call_args(Some(name), None, args, env, depth + 1)?;
            return Ok(ReturnValue::Value(TrackedValue::runtime(Value::Nil)));
        }

        // AB_CALLGLOBAL pushes a nil target/return slot before parsing its
        // arguments. It stays live through the selected global function.
        let _target_slot = ValueStackReservation::reserve(1)?;
        let mut evaluated_args =
            self.build_call_args(Some(name), function, args, env, depth + 1)?;
        // Parse_Params evaluates every explicit expression, then balances the
        // stack down to C4AUL_MAX_Par before AB_CALLGLOBAL dispatches.
        evaluated_args.truncate(MAX_CALL_PARAMETERS);
        if forward_rest {
            Self::append_forwarded_args(&mut evaluated_args, env, MAX_CALL_PARAMETERS)?;
        }
        // Parse_Params balances every global call to ten slots. Reserve them
        // here and tell the eventual script/native boundary that these are
        // already the callee's parameter frame.
        let _parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;

        let _context = GlobalCallContextGuard::enter(self.global_call_context_hook);
        let global_vm = self.engine_global_vm();
        if vm_builtin {
            return global_vm
                .invoke_global_builtin_raw(name, &evaluated_args, env, depth + 1)
                .map(materialize_target_call_result);
        }
        if function.is_none() && name == "EffectVar" {
            if let Some(host) = global_vm.host_functions.get(name) {
                let caller = env.caller_context();
                let _guard = CallerContextGuard::enter(Some(caller.clone()));
                let args =
                    global_vm.prepare_registered_host_call_args(name, host, evaluated_args)?;
                let args = global_vm.call_args_to_values(&args)?.into_vec();
                return Ok(ReturnValue::Reference(LValueRef::HostPath {
                    function: host.callback().clone(),
                    args,
                    caller,
                    global_call_context_hook: global_vm.global_call_context_hook.cloned(),
                    segments: Vec::new(),
                    legacy_pin: None,
                }));
            }
        }
        // Install the one-shot only after hooks and VM builtins have run: an
        // eval builtin or context hook may itself call script and must not
        // consume the parameter ownership intended for this dispatch.
        let _parameter_override = CallParameterOverrideGuard::enter(0);
        global_vm
            .invoke_engine_global_raw(name, evaluated_args, depth + 1, Some(env.caller_context()))
            .map(materialize_target_call_result)
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_global_call(
        &self,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        self.invoke_global_call_raw(name, args, failsafe, forward_rest, env, depth)?
            .into_value_on_stack()
    }

    /// `base->name(args)` / `base->~name(args)`: the direct object call
    /// (AB_CALL/AB_CALLFS, C4AulExec.cpp:1216-1305). The target evaluates
    /// first; a FALSY target throws even for the failsafe form (:1224-1226);
    /// object and id targets resolve on the TARGET's live context through the
    /// engine-registered method dispatch, including `this`: ChangeDef may
    /// have replaced its definition while the current callback remains on
    /// the stack. The `~` only forgives a missing FUNCTION.
    #[allow(clippy::too_many_arguments)]
    fn invoke_property_call(
        &self,
        base: &Expr,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        let target = self.evaluate(base, env, depth + 1)?;
        let _target_slot = ValueStackReservation::reserve(1)?;
        self.invoke_property_call_with_target(
            target,
            name,
            args,
            failsafe,
            forward_rest,
            env,
            depth,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_property_call_with_target(
        &self,
        target: Value,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        self.invoke_property_call_with_target_raw(
            target,
            name,
            args,
            failsafe,
            forward_rest,
            env,
            depth,
        )
        .map(c4_set_copy_value)
    }

    /// AB_CALL stores a value result in the former target slot through
    /// C4Value::Set. Keep the raw implementation separate so reference-call
    /// routing can continue to use its dedicated lvalue path.
    #[allow(clippy::too_many_arguments)]
    fn invoke_property_call_with_target_raw(
        &self,
        mut target: Value,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        let target_sweep_cursor = object_reference_sweep_cursor();
        if failsafe && !self.direct_call_function_known(name) {
            // GetFirstFunc failed during C++ parsing, so no AB_CALLFS exists:
            // Parse_Params(0) still evaluates every explicit argument after
            // the already-evaluated target, then the target slot becomes nil
            // (C4AulParse.cpp:3215-3231). A forwarded `...` supplies no slots
            // to this zero-parameter pseudo-call.
            self.evaluate_discarded_call_args(args, env, depth + 1)?;
            return Ok(Value::Nil);
        }

        // Effect-callback state maps carry the object id ("id" key): an
        // arrow call on one targets THAT object, matching the host-fn
        // object-reference convention (C++ pTarget is C4VObj —
        // FxLifeStop's `pTarget->RemWarning(...)`).
        if let Value::Proplist(map) = &target {
            if let Some(Value::Int(id)) = map.get("id") {
                if *id > 0 {
                    target = Value::Object(*id as u64);
                }
            }
        }
        // `pObj->LocalN("name")`: the `->` operator supplies Obj=pObj, so the
        // global engine function FnLocalN reads pObj's named local
        // (C4Script.cpp:4598-4611, pObj defaulting to cthr->Obj). It is not an
        // object script method, so it resolves through the cross-object cell
        // hook here — never through world method dispatch, which would raise
        // "No function LocalN in object N" (Goal.c4d's
        // `curr_goal->LocalN("missionPassword")`, of which content has 14
        // call sites). Matches the two-argument `LocalN("name", pObj)` form.
        // A zero target still falls through to the "target is zero" guard, as
        // the C++ arrow-call check fires before FnLocalN runs.
        if matches!(&target, Value::Object(id) if *id != 0)
            && name == "LocalN"
            && args.len() == 1
            && !self.functions.contains_key(name)
        {
            let evaluated_args = self.build_call_args(None, None, args, env, depth + 1)?;
            let _parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;
            clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
            if matches!(target, Value::Nil | Value::Object(0))
                || !self.object_target_available(&target)
            {
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
            let cell = self.localn_cell(env, &local_name, Some(target));
            let value = cell.borrow().clone();
            return Ok(value);
        }
        // `pObj->Local(n)`: the numbered-slot analogue (FnLocal by-reference,
        // C4Script.cpp:3423-3433). Same routing as LocalN — resolve the
        // TARGET's `__local_{n}` slot through the cross-object cell hook, not
        // world dispatch. A negative index reads nil like FnLocal. Hazard's
        // Ammo.c `return(ammo->Local(0))` depends on it.
        if matches!(&target, Value::Object(id) if *id != 0)
            && name == "Local"
            && args.len() == 1
            && !self.functions.contains_key(name)
            && !self.has_host_function(name)
        {
            let evaluated_args = self.build_call_args(None, None, args, env, depth + 1)?;
            let _parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;
            clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
            if matches!(target, Value::Nil | Value::Object(0))
                || !self.object_target_available(&target)
            {
                return Err(RuntimeError::new("Object call: target is zero!"));
            }
            let index = Self::slot_index_from_value("Local()", evaluated_args[0].read()?)?;
            if index < 0 {
                return Ok(Value::Nil);
            }
            let cell = self.numbered_local_cell(env, index, Some(target));
            let value = cell.borrow().clone();
            return Ok(value);
        }
        // `pObj->SetLocal(index, value[, target])`: arrow dispatch makes
        // pObj the executing object, so an omitted/falsy explicit target
        // defaults to the receiver. Route directly through the same numbered
        // local cell hook as Local(index, pObj), never world method dispatch.
        if matches!(&target, Value::Object(id) if *id != 0)
            && name == "SetLocal"
            && !self.functions.contains_key(name)
            && !self.has_host_function(name)
        {
            let evaluated_args = self.build_call_args(None, None, args, env, depth + 1)?;
            let _parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;
            clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
            if matches!(target, Value::Nil | Value::Object(0))
                || !self.object_target_available(&target)
            {
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
            return Ok(value.value);
        }
        if matches!(
            &target,
            Value::Nil | Value::Int(0) | Value::Bool(false) | Value::RawBool(0) | Value::Object(0)
        ) || matches!(&target, Value::C4Id(id) if crate::value::c4_id_raw(id) == 0)
        {
            // Parse_Params emits every argument expression before AB_CALL or
            // AB_CALLFS checks the target (C4AulParse.cpp:3240;
            // C4AulExec.cpp:1216-1226). Preserve those side effects and let
            // an argument error win before reporting the zero target.
            let function = self.functions.get(name);
            let _ = self.build_call_args(Some(name), function, args, env, depth + 1)?;
            let _parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;
            return Err(RuntimeError::new("Object call: target is zero!"));
        }
        match &target {
            Value::Object(_) | Value::C4Id(_) if self.method_dispatch.is_some() => {
                let function = self.functions.get(name);
                let mut evaluated_args =
                    self.build_call_args(Some(name), function, args, env, depth + 1)?;
                if forward_rest {
                    Self::append_forwarded_args(&mut evaluated_args, env, MAX_CALL_PARAMETERS)?;
                }
                let _parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;
                clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
                if matches!(target, Value::Nil | Value::Object(0))
                    || !self.object_target_available(&target)
                {
                    return Err(RuntimeError::new("Object call: target is zero!"));
                }
                let mut dispatch_args = Vec::with_capacity(evaluated_args.len() + 3);
                dispatch_args.push(target.clone());
                dispatch_args.push(Value::String(name.to_string().into()));
                dispatch_args.push(Value::Bool(failsafe));
                for arg in &evaluated_args {
                    dispatch_args.push(arg.read()?);
                }
                // C++ pushes lvalue arguments as `C4V_pC4Value` and the callee
                // writes straight through them (C4AulParse.cpp:2318-2331,
                // C4AulExec.cpp:1381-1397). A `&[Value]` bridge flattens that,
                // so route reference arguments through the dispatch twin that
                // reports the callee's final parameter slots and settle the
                // caller's cells from them. Hazard's
                // `this->~WeaponAt(x, y, r)` needs exactly this
                // (Hazard.c4d/Libraries.c4d/Functionalities.c4d/CanAim.c4d/
                // Script.c:220-226, HazardClonk.c4d/Script.c:930).
                let references_out = evaluated_args
                    .iter()
                    .any(|arg| matches!(arg, CallArg::Reference(_)))
                    .then_some(self.method_ref_args_dispatch)
                    .flatten();
                let dispatch = self
                    .method_dispatch
                    .ok_or_else(|| RuntimeError::new("method dispatch vanished".to_string()))?;
                // The Rust world bridge may need to re-enter another VM and
                // resolve this arrow call directly to a native function. Keep
                // the suspended script frame visible while the bridge runs so
                // its dedicated preserving entry can reproduce C++ AB_CALL's
                // `CallCtx.Caller = pCurCtx`.
                let _guard = CallerContextGuard::enter(Some(env.caller_context()));
                let _parameter_override = CallParameterOverrideGuard::enter(0);
                let Some(references_out) = references_out else {
                    return dispatch(&dispatch_args);
                };
                let (result, finals) = references_out(&dispatch_args)?;
                for (arg, settled) in evaluated_args.iter().zip(finals) {
                    // A plain parameter received a dereferenced copy, so its
                    // slot still holds what was passed in and this is a no-op.
                    if let CallArg::Reference(reference) = arg {
                        if reference.read()? != settled {
                            reference.write(settled)?;
                        }
                    }
                }
                Ok(result)
            }
            Value::Object(_) | Value::C4Id(_) => {
                // Self-target (or a bare engine without a world): resolve in
                // the executing context — FindSameNameFunc with
                // pDestDef == own def is the plain own->global->host chain.
                self.invoke_property_call_local(
                    name,
                    args,
                    failsafe,
                    forward_rest,
                    env,
                    depth,
                    Some((&mut target, target_sweep_cursor)),
                )
            }
            other => {
                if self.method_dispatch.is_some() {
                    let function = self.functions.get(name);
                    let _ = self.build_call_args(Some(name), function, args, env, depth + 1)?;
                    let _parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;
                    Err(RuntimeError::new(format!(
                        "Object call: Invalid target type {}, expected object or id!",
                        other.type_name()
                    )))
                } else {
                    // Bare scripting engines have no object world: keep the
                    // legacy resolve-by-name behavior for their tests.
                    self.invoke_property_call_local(
                        name,
                        args,
                        failsafe,
                        forward_rest,
                        env,
                        depth,
                        None,
                    )
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_property_call_local(
        &self,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
        retained_target: Option<(&mut Value, usize)>,
    ) -> Result<Value, RuntimeError> {
        let function = self.own_or_global_script_function(name);
        if failsafe
            && function.is_none()
            && !self.has_host_function(name)
            && !self
                .global_functions
                .map(|functions| functions.contains_key(name))
                .unwrap_or(false)
        {
            // ->~ on a missing function: the parameters still evaluate (they
            // are on the stack before AB_CALLFS pops them, C4AulExec.cpp:
            // 1262-1267), the result is nil.
            let _ = self.build_call_args(Some(name), function, args, env, depth + 1)?;
            if let Some((target, cursor)) = retained_target {
                clear_value_for_object_reference_sweeps(target, cursor);
                if matches!(target, Value::Nil | Value::Object(0))
                    || !self.object_target_available(target)
                {
                    return Err(RuntimeError::new("Object call: target is zero!"));
                }
            }
            return Ok(Value::Nil);
        }
        let mut evaluated_args =
            self.build_call_args(Some(name), function, args, env, depth + 1)?;
        if forward_rest {
            Self::append_forwarded_args(&mut evaluated_args, env, MAX_CALL_PARAMETERS)?;
        }
        if let Some((target, cursor)) = retained_target {
            clear_value_for_object_reference_sweeps(target, cursor);
            if matches!(target, Value::Nil | Value::Object(0))
                || !self.object_target_available(target)
            {
                return Err(RuntimeError::new("Object call: target is zero!"));
            }
        }
        let _parameter_override = CallParameterOverrideGuard::enter(MAX_CALL_PARAMETERS);
        self.invoke_value_with_reserved_result(
            name,
            evaluated_args,
            depth + 1,
            env.object_state.clone(),
            Some(env.caller_context()),
        )
    }

    fn build_call_args(
        &self,
        name: Option<&str>,
        function: Option<&Function>,
        args: &[Expr],
        env: &mut Environment,
        depth: usize,
    ) -> Result<CallArgs, RuntimeError> {
        let mut evaluated_args = CallArgs::with_capacity(args.len());
        let mut value_stack = ValueStackReservation::empty();
        for (index, arg) in args.iter().enumerate() {
            let sweep_cursor = object_reference_sweep_cursor();
            // `anyfunctakesref` (C4AulParse.cpp:2318-2331) unions the resolved
            // callee with every other engine function of that name, so a slot
            // stays a reference even when THIS host's same-named function
            // takes a value — or has no such function at all. That is what
            // lets Hazard's weapon reach the Clonk's `WeaponAt(&x, &y, &r)`
            // across definitions (Items.c4d/Weapons.c4d/Weapon.c4d/
            // Script.c:810).
            let script_wants_reference = function
                .and_then(|function| function.params.get(index))
                .is_some_and(|param| param.is_reference)
                || name
                    .zip(self.reference_parameter_probe)
                    .is_some_and(|(name, probe)| probe(name, index));
            let host_wants_reference = function.is_none()
                && name
                    .and_then(|name| self.host_reference_function(name))
                    .is_some_and(|function| function.wants_reference(index));
            // An unresolved `this` is the context-function result, an rvalue;
            // a parameter/function-var/object-local named `this` remains the
            // ordinary live reference found by the same syntax.
            let unbound_context_this =
                matches!(arg, Expr::Variable(name) if name == "this") && !self.has_bound_this(env);
            let can_be_reference = if unbound_context_this {
                false
            } else if host_wants_reference {
                self.expr_can_be_host_reference(arg)
            } else {
                Self::expr_can_be_lvalue(arg)
            };
            if (script_wants_reference || host_wants_reference)
                && (Self::expression_contains_array_append(arg)
                    || self.set_no_ref_keeps_reference(arg, env)
                    || matches!(arg, Expr::Call { callee, .. } if matches!(callee.as_ref(), Expr::Variable(_)))
                    || matches!(arg, Expr::GlobalCall { .. })
                    || matches!(arg, Expr::PreIncrement(_) | Expr::PreDecrement(_))
                    || matches!(
                        arg,
                        Expr::Assignment(target, _)
                            if !matches!(target, AssignmentTarget::InvalidValue { .. })
                    )
                    || matches!(
                        arg,
                        Expr::CompoundAssignment { .. } | Expr::ArrayAppendAssignment { .. }
                    ))
            {
                let argument = {
                    let _pin_creation = LegacyPathPinCreationGuard::enter();
                    self.evaluate_reference_or_value(arg, env, depth)?
                };
                for retained in &mut evaluated_args {
                    retained.clear_object_reference_sweeps(sweep_cursor);
                }
                evaluated_args.push(match argument {
                    ReturnValue::Reference(reference) => CallArg::Reference(reference),
                    ReturnValue::Value(value) => CallArg::Value(value),
                });
                value_stack.grow(1)?;
                continue;
            }
            if (script_wants_reference || host_wants_reference) && can_be_reference {
                let argument = {
                    let _pin_creation = LegacyPathPinCreationGuard::enter();
                    self.evaluate_reference_or_value(arg, env, depth)?
                };
                for retained in &mut evaluated_args {
                    retained.clear_object_reference_sweeps(sweep_cursor);
                }
                evaluated_args.push(match argument {
                    ReturnValue::Reference(reference) => CallArg::Reference(reference),
                    ReturnValue::Value(value) => CallArg::Value(value),
                });
                value_stack.grow(1)?;
            } else {
                let argument = self.evaluate_tracked(arg, env, depth)?;
                for retained in &mut evaluated_args {
                    retained.clear_object_reference_sweeps(sweep_cursor);
                }
                evaluated_args.push(CallArg::Value(argument));
                value_stack.grow(1)?;
            }
        }
        #[cfg(test)]
        record_call_arg_heap_spill(evaluated_args.spilled());
        Ok(evaluated_args)
    }

    /// `Parse_Params(0, nullptr)` for a globally unresolved fail-safe call.
    /// With no candidate function, C++ deliberately leaves each operand's
    /// reference bytecode intact, holds every slot until all expressions have
    /// run, and then drops the complete zero-parameter frame
    /// (C4AulParse.cpp:2311-2344).
    fn evaluate_discarded_call_args(
        &self,
        args: &[Expr],
        env: &mut Environment,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        let mut evaluated = Vec::with_capacity(args.len());
        let mut value_stack = ValueStackReservation::empty();
        for arg in args {
            let value = {
                let _pin_creation = LegacyPathPinCreationGuard::enter();
                self.evaluate_reference_or_value(arg, env, depth)?
            };
            evaluated.push(value);
            value_stack.grow(1)?;
        }
        Ok(())
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

    fn direct_call_parameter_limit(&self, name: &str, script_function: Option<&Function>) -> usize {
        if script_function.is_some() {
            return MAX_CALL_PARAMETERS;
        }
        self.host_functions
            .get(name)
            .and_then(RegisteredHostFunction::parameter_count)
            .or_else(|| {
                self.host_reference_function(name)
                    .and_then(HostReferenceFunction::parameter_count)
            })
            .unwrap_or(MAX_CALL_PARAMETERS)
    }

    fn expr_can_be_lvalue(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Variable(_)
                | Expr::Property(_, _)
                | Expr::Index(_, _)
                | Expr::ArrayAppend(_)
                | Expr::Call { .. }
                | Expr::GlobalCall { .. }
        )
    }

    fn expression_contains_array_append(expr: &Expr) -> bool {
        match expr {
            Expr::ArrayAppend(_) => true,
            Expr::Property(base, _) | Expr::Index(base, _) => {
                Self::expression_contains_array_append(base)
            }
            _ => false,
        }
    }

    /// Native reference parameters receive a null pointer for rvalue call
    /// results. Unlike script `&` parameters, we can decide that before
    /// evaluation for ordinary named functions and avoid executing a
    /// value-returning call once through `invoke_reference` and then again as
    /// a value (FnSimFlight's C4Value* parameters, C4Script.cpp:5309-5312).
    fn expr_can_be_host_reference(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Variable(_) | Expr::Property(_, _) | Expr::Index(_, _) | Expr::ArrayAppend(_) => {
                true
            }
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Variable(name) => {
                    matches!(
                        name.as_str(),
                        "Local" | "LocalN" | "Var" | "VarN" | "EffectVar" | "Global" | "GlobalN"
                    ) || self
                        .functions
                        .get(name)
                        .or_else(|| {
                            self.global_functions
                                .and_then(|functions| functions.get(name))
                        })
                        .is_some_and(|function| function.returns_reference)
                }
                // Cross-object reference-return metadata is resolved by the
                // method-reference dispatcher at runtime.
                Expr::Property(_, _) => true,
                _ => false,
            },
            Expr::GlobalCall { name, .. } => self.global_call_may_return_reference(name),
            _ => false,
        }
    }

    fn evaluate_assignment(
        &self,
        target: &AssignmentTarget,
        value_expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        if let AssignmentTarget::InvalidValue {
            expression,
            operator,
        } = target
        {
            let left = self.evaluate(expression, env, depth)?;
            if *operator == "??=" && !matches!(left, Value::Nil) {
                return Ok(left);
            }
            let _left_slot = ValueStackReservation::reserve(1)?;
            self.evaluate(value_expr, env, depth)?;
            return Err(RuntimeError::new(format!(
                "operator \"{operator}\" left side: got \"{}\", but expected \"&\"!",
                left.type_name()
            )));
        }

        self.evaluate_plain_assignment_tracked(target, value_expr, env, depth)
            .map(|tracked| tracked.value)
    }

    fn evaluate_plain_assignment_tracked(
        &self,
        target: &AssignmentTarget,
        value_expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        self.evaluate_plain_assignment_raw(target, value_expr, env, depth)?
            .into_tracked_on_stack()
    }

    fn evaluate_plain_assignment_raw(
        &self,
        target: &AssignmentTarget,
        value_expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        // AB_Set receives one already-evaluated reference, followed by the
        // RHS. Retain that reference across RHS evaluation without reading it:
        // value-style host references need only their address arguments here.
        let target = {
            // AB_Set always receives a reference, even when a surrounding
            // operator later turns the assignment result into a value.
            let _pin_creation = LegacyPathPinCreationGuard::enter();
            self.assignment_target_to_reference_or_value(env, target, depth)?
        };
        let _target_slot = ValueStackReservation::reserve(1)?;
        let reference = match target {
            ReturnValue::Reference(reference) => reference,
            ReturnValue::Value(left) => {
                self.evaluate_tracked(value_expr, env, depth)?;
                return Err(RuntimeError::new(format!(
                    "operator \"=\" left side: got \"{}\", but expected \"&\"!",
                    Self::c4v_type_name(left.value.c4v_type())
                )));
            }
        };
        let value = self.evaluate_set_no_ref_result(value_expr, env, depth)?;
        let _right_slot = ValueStackReservation::reserve(1)?;
        let tracked = match value {
            // SetNoRef cannot rewrite a reference-returning call opcode, so
            // AB_Set's CheckOpPar<Any> reaches FnCnvDeref. Deref copies the
            // referent through Set before the destination assignment.
            ReturnValue::Reference(reference) => reference.read_tracked()?.set_copy(),
            ReturnValue::Value(tracked) => tracked,
        };
        if let Some(left) = reference.resolved_legacy_value() {
            return Err(RuntimeError::new(format!(
                "operator \"=\" left side: got \"{}\", but expected \"&\"!",
                Self::c4v_type_name(left.value.c4v_type())
            )));
        }
        reference.write_tracked(tracked.clone())?;
        Ok(ReturnValue::Reference(reference))
    }

    fn assignment_target_to_reference_or_value(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        match target {
            AssignmentTarget::FunctionCall { name, args } => {
                let expression = Expr::Call {
                    callee: Box::new(Expr::Variable(name.clone())),
                    args: args.clone(),
                    is_optional: false,
                    forward_rest: false,
                };
                return self.evaluate_reference_or_value(&expression, env, depth);
            }
            AssignmentTarget::GlobalFunctionCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => {
                return self.invoke_global_call_raw(
                    name,
                    args,
                    *failsafe,
                    *forward_rest,
                    env,
                    depth,
                );
            }
            _ => {}
        }

        match target {
            AssignmentTarget::ArrayAppend(base) => self.evaluate_array_append(base, env, depth),
            AssignmentTarget::Property(base, property) => {
                let base = self.assignment_target_to_reference_or_value(env, base, depth)?;
                let _base_slot = ValueStackReservation::reserve(1)?;
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
                self.property_reference_or_value(base, property, env)
            }
            AssignmentTarget::Index(base, index) => {
                let base = self.assignment_target_to_reference_or_value(env, base, depth)?;
                let _base_slot = ValueStackReservation::reserve(1)?;
                self.index_reference_or_value(base, index, env, depth)
            }
            _ => self
                .assignment_target_to_lvalue(env, target, depth)
                .map(ReturnValue::Reference),
        }
    }

    fn assignment_target_to_lvalue(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
        depth: usize,
    ) -> Result<LValueRef, RuntimeError> {
        match target {
            AssignmentTarget::InvalidValue { .. } => Err(RuntimeError::new(
                "this assignment target is a value, not a reference",
            )),
            AssignmentTarget::Variable(name) => env
                .lvalue(name)
                .or_else(|| {
                    self.global_variable_cell(name)
                        .map(|cell| self.tracked_cell(cell))
                })
                .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'"))),
            AssignmentTarget::Property(base, property) => {
                let reference = self.assignment_target_to_lvalue(env, base, depth)?;
                let _base_slot = ValueStackReservation::reserve(1)?;
                if !matches!(&reference, LValueRef::HostPath { .. }) {
                    let collection = reference.read()?;
                    match &collection {
                        Value::Object(0) => {
                            return Err(RuntimeError::new(
                                "map access with .: map expected, but got nil!",
                            ));
                        }
                        target @ Value::Object(_) => {
                            return self
                                .object_local_cell(env, target, property)
                                .map(|cell| self.tracked_cell(cell))
                                .ok_or_else(|| {
                                    RuntimeError::new(
                                        "this assignment target is a value, not a reference",
                                    )
                                });
                        }
                        _ => {}
                    }
                }
                reference.detach_container_identity_if_shared();
                reference.append(PathSegment::Property(property.clone()))
            }
            AssignmentTarget::Index(base, index_operand) => {
                let reference = self.assignment_target_to_lvalue(env, base, depth)?;
                let _base_slot = ValueStackReservation::reserve(1)?;
                if !matches!(&reference, LValueRef::HostPath { .. }) {
                    let (index, _index_slot) =
                        self.evaluate_index_operand(index_operand, env, depth)?;
                    let collection = reference.read()?;
                    match (&collection, &index) {
                        (Value::Object(0), _) => {
                            return Err(RuntimeError::new(
                                "indexed access [index]: array, map or string expected, but got nil",
                            ));
                        }
                        (target @ Value::Object(_), Value::String(name)) => {
                            return self
                                .object_local_cell(env, target, name)
                                .map(|cell| self.tracked_cell(cell))
                                .ok_or_else(|| {
                                    RuntimeError::new(
                                        "this assignment target is a value, not a reference",
                                    )
                                });
                        }
                        (Value::Object(_), _) => {
                            return Err(RuntimeError::new(
                                "indexed access on object: only string keys are allowed",
                            ));
                        }
                        _ => {
                            reference.detach_container_identity_if_shared();
                            return reference.append(PathSegment::Index(index));
                        }
                    }
                }
                let (index, _index_slot) =
                    self.evaluate_index_operand(index_operand, env, depth)?;
                reference.detach_container_identity_if_shared();
                reference.append(PathSegment::Index(index))
            }
            AssignmentTarget::ArrayAppend(base) => {
                match self.evaluate_array_append(base, env, depth)? {
                    ReturnValue::Reference(reference) => Ok(reference),
                    ReturnValue::Value(_) => Err(RuntimeError::new(
                        "this assignment target is a value, not a reference",
                    )),
                }
            }
            AssignmentTarget::LocalSlot(index_expr) => {
                let index = self.evaluate_slot_index("Local()", index_expr, env, depth)?;
                let _parameter_slots = ValueStackReservation::reserve(2)?;
                if self.retain_global_call_context_for_host_paths {
                    return Err(RuntimeError::new(
                        "function 'Local' does not return a reference",
                    ));
                }
                Ok(self.tracked_cell(env.object_state.local_slot_cell(index)))
            }
            AssignmentTarget::VarSlot(index_expr) => {
                let index = self.evaluate_slot_index("Var()", index_expr, env, depth)?;
                let _parameter_slot = ValueStackReservation::reserve(1)?;
                Ok(self.tracked_cell(frame_slot_cell(&env.frame_locals, index)))
            }
            AssignmentTarget::EffectSlot(args) => {
                let evaluated_args = self.build_call_args(None, None, args, env, depth + 1)?;
                if let Some(function) = self.host_functions.get("EffectVar") {
                    let _parameter_slots =
                        ValueStackReservation::reserve(function.parameter_count().unwrap_or(3))?;
                    let caller = env.caller_context();
                    let _guard = CallerContextGuard::enter(Some(caller.clone()));
                    let prepared_args = self.prepare_registered_host_call_args(
                        "EffectVar",
                        function,
                        evaluated_args,
                    )?;
                    let arg_values = self.call_args_to_values(&prepared_args)?.into_vec();
                    return Ok(LValueRef::HostPath {
                        function: function.callback().clone(),
                        args: arg_values,
                        caller,
                        global_call_context_hook: self
                            .retain_global_call_context_for_host_paths
                            .then(|| self.global_call_context_hook.cloned())
                            .flatten(),
                        segments: Vec::new(),
                        legacy_pin: None,
                    });
                }
                let _parameter_slots = ValueStackReservation::reserve(3)?;
                let raw_arg_values = evaluated_args.iter().map(CallArg::read).collect::<Result<
                    CallValues,
                    _,
                >>(
                )?;

                // Host-less fixture VMs retain EffectVar slots in ordinary
                // environment cells; exposing that cell keeps the same
                // reference/path behavior as the engine-backed variant.
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
                env.lvalue(&slot_name)
                    .ok_or_else(|| RuntimeError::new("EffectVar slot disappeared"))
            }
            AssignmentTarget::GlobalFunctionCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => match self.invoke_global_call_raw(
                name,
                args,
                *failsafe,
                *forward_rest,
                env,
                depth,
            )? {
                ReturnValue::Reference(reference) => Ok(reference),
                ReturnValue::Value(_) => Err(RuntimeError::new(format!(
                    "function '{name}' does not return a reference"
                ))),
            },
            AssignmentTarget::FunctionCall { name, args }
                if name == "Global"
                    && !self.functions.contains_key(name)
                    && !self
                        .global_functions
                        .is_some_and(|functions| functions.contains_key(name))
                    && !self.has_host_function(name) =>
            {
                Ok(self.tracked_cell(self.evaluate_global_slot(args, env, depth + 1)?))
            }
            AssignmentTarget::FunctionCall { name, args }
                if name == "GlobalN"
                    && !self.functions.contains_key(name)
                    && !self
                        .global_functions
                        .is_some_and(|functions| functions.contains_key(name))
                    && !self.has_host_function(name) =>
            {
                self.evaluate_named_global(args, env, depth + 1)?
                    .map(|cell| self.tracked_cell(cell))
                    .ok_or_else(|| {
                        RuntimeError::new("function 'GlobalN' does not return a reference")
                    })
            }
            AssignmentTarget::FunctionCall { name, args }
                if name == "LocalN"
                    && (1..=2).contains(&args.len())
                    && !self.functions.contains_key(name) =>
            {
                // FnLocalN returns pVarN->GetRef() (C4Script.cpp:4604):
                // `LocalN("x") = v` writes the named object local through;
                // the two-argument form targets ANOTHER object's local via
                // the host cell hook.
                let evaluated_args = self.build_call_args(None, None, args, env, depth + 1)?;
                let _parameter_slots = ValueStackReservation::reserve(2)?;
                let local_name = match evaluated_args[0].read()? {
                    Value::String(local_name) => local_name,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "LocalN: expected string for name, got {}",
                            other.type_name()
                        )))
                    }
                };
                let target = evaluated_args.get(1).map(CallArg::read).transpose()?;
                if self.retain_global_call_context_for_host_paths
                    && target.as_ref().is_none_or(|value| {
                        matches!(
                            value,
                            Value::Nil
                                | Value::Int(0)
                                | Value::Bool(false)
                                | Value::RawBool(0)
                                | Value::Object(0)
                        )
                    })
                {
                    return Err(RuntimeError::new(
                        "function 'LocalN' does not return a reference",
                    ));
                }
                Ok(self.tracked_cell(self.localn_cell(env, &local_name, target)))
            }
            AssignmentTarget::FunctionCall { name, args }
                if name == "Par"
                    && args.len() <= 1
                    && !self.functions.contains_key(name)
                    && !self.has_host_function(name) =>
            {
                let evaluated_args = self.build_call_args(None, None, args, env, depth + 1)?;
                let _parameter_slot = ValueStackReservation::reserve(1)?;
                let index = evaluated_args
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
                Ok(usize::try_from(index)
                    .ok()
                    .filter(|index| *index < MAX_CALL_PARAMETERS)
                    .and_then(|index| env.call_args.get(index))
                    .map(Binding::lvalue)
                    .unwrap_or_else(|| Binding::direct(Value::Nil).lvalue()))
            }
            AssignmentTarget::FunctionCall { name, args } => {
                if name == "this" && self.has_bound_this(env) {
                    return Err(RuntimeError::new("cannot call bound variable 'this'"));
                }
                let function = self.own_or_global_script_function(name);
                let args = self.build_call_args(Some(name), function, args, env, depth + 1)?;
                self.invoke_reference(
                    name,
                    args,
                    depth + 1,
                    env.object_state.clone(),
                    Some(env.caller_context()),
                )
            }
            // `LocalN("name", obj) += v` and friends: the foreign-local
            // cell IS the reference (FnLocalN, C4Script.cpp:4591-4605).
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
                is_arrow,
            } if method == "LocalN" && args.len() == 1 => {
                let (object_value, evaluated_args, _target_slot, _parameter_slots) = self
                    .evaluate_method_slot_operands(
                        object, args, *is_arrow, None, None, 2, env, depth,
                    )?;
                if *is_arrow
                    && matches!(
                        &object_value,
                        Value::Nil
                            | Value::Int(0)
                            | Value::Bool(false)
                            | Value::RawBool(0)
                            | Value::Object(0)
                    )
                {
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
                Ok(self.tracked_cell(self.localn_cell(env, &local_name, Some(object_value))))
            }
            // `Local(n, obj)` by reference: FnLocal returns
            // `pObj->Local[iIndex].GetRef()` (C4Script.cpp:3423-3433).
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
                is_arrow,
            } if method == "Local" && args.len() == 1 => {
                let (object_value, evaluated_args, _target_slot, _parameter_slots) = self
                    .evaluate_method_slot_operands(
                        object, args, *is_arrow, None, None, 2, env, depth,
                    )?;
                if *is_arrow
                    && matches!(
                        &object_value,
                        Value::Nil
                            | Value::Int(0)
                            | Value::Bool(false)
                            | Value::RawBool(0)
                            | Value::Object(0)
                    )
                {
                    return Err(RuntimeError::new("Object call: target is zero!"));
                }
                let index = Self::slot_index_from_value("Local()", evaluated_args[0].read()?)?;
                Ok(self.tracked_cell(self.numbered_local_cell(env, index, Some(object_value))))
            }
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
                is_arrow,
            } if matches!(method.as_str(), "Var" | "EffectVar") => {
                // Preserve the legacy method-slot shim as an actual retained
                // cell so plain assignment can resolve it before the RHS.
                let native_slots = if method == "Var" { 1 } else { 3 };
                let (object_value, evaluated_args, _target_slot, _parameter_slots) = self
                    .evaluate_method_slot_operands(
                        object,
                        args,
                        *is_arrow,
                        None,
                        None,
                        native_slots,
                        env,
                        depth,
                    )?;
                let object_id = match object_value {
                    Value::Int(value) => value.to_string(),
                    Value::String(value) => value.into_string(),
                    other => format!("{other:?}"),
                };
                let arg_values = evaluated_args
                    .iter()
                    .map(CallArg::read)
                    .collect::<Result<Vec<_>, _>>()?;
                let key = arg_values
                    .iter()
                    .map(|value| match value {
                        Value::Int(value) => value.to_string(),
                        Value::String(value) => value.to_string(),
                        other => format!("{other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join("_");
                let slot_name = format!("__method_{object_id}_{method}_{key}");
                if env.get(&slot_name)?.is_none() {
                    env.define(&slot_name, Value::Nil);
                }
                env.lvalue(&slot_name)
                    .ok_or_else(|| RuntimeError::new("method slot disappeared"))
            }
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
                is_arrow,
            } if !matches!(method.as_str(), "Var" | "EffectVar") => {
                let function = self.functions.get(method);
                let (mut target, evaluated_args, _target_slot, _parameter_slots) = self
                    .evaluate_method_slot_operands(
                        object,
                        args,
                        *is_arrow,
                        Some(method),
                        function,
                        MAX_CALL_PARAMETERS,
                        env,
                        depth,
                    )?;
                if let Value::Proplist(map) = &target {
                    if let Some(Value::Int(id)) = map.get("id") {
                        if *id > 0 {
                            target = Value::Object(*id as u64);
                        }
                    }
                }
                if matches!(
                    target,
                    Value::Nil
                        | Value::Int(0)
                        | Value::Bool(false)
                        | Value::RawBool(0)
                        | Value::Object(0)
                ) || matches!(&target, Value::C4Id(id) if crate::value::c4_id_raw(id) == 0)
                {
                    return Err(RuntimeError::new("Object call: target is zero!"));
                }

                match &target {
                    Value::Object(_) | Value::C4Id(_)
                        if self.method_reference_dispatch.is_some() =>
                    {
                        let mut dispatch_args = Vec::with_capacity(evaluated_args.len() + 3);
                        dispatch_args.push(target);
                        dispatch_args.push(Value::String(method.clone().into()));
                        dispatch_args.push(Value::Bool(false));
                        for arg in &evaluated_args {
                            dispatch_args.push(arg.read()?);
                        }
                        let dispatch = self.method_reference_dispatch.ok_or_else(|| {
                            RuntimeError::new("method reference dispatch vanished")
                        })?;
                        let _guard = CallerContextGuard::enter(Some(env.caller_context()));
                        let _parameter_override =
                            (*is_arrow).then(|| CallParameterOverrideGuard::enter(0));
                        dispatch(&dispatch_args).map(ValueReference::into_lvalue)
                    }
                    Value::Object(_) | Value::C4Id(_) => {
                        let _parameter_override =
                            (*is_arrow).then(|| CallParameterOverrideGuard::enter(0));
                        self.invoke_reference(
                            method,
                            evaluated_args,
                            depth + 1,
                            env.object_state.clone(),
                            Some(env.caller_context()),
                        )
                    }
                    other if self.method_reference_dispatch.is_some() => {
                        Err(RuntimeError::new(format!(
                            "Object call: Invalid target type {}, expected object or id!",
                            other.type_name()
                        )))
                    }
                    _ => {
                        let _parameter_override =
                            (*is_arrow).then(|| CallParameterOverrideGuard::enter(0));
                        self.invoke_reference(
                            method,
                            evaluated_args,
                            depth + 1,
                            env.object_state.clone(),
                            Some(env.caller_context()),
                        )
                    }
                }
            }
            AssignmentTarget::MethodSlot { .. } => Err(RuntimeError::new(
                "this assignment target cannot be passed by reference",
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_method_slot_operands(
        &self,
        object: &Expr,
        args: &[Expr],
        is_arrow: bool,
        name: Option<&str>,
        function: Option<&Function>,
        native_parameter_slots: usize,
        env: &mut Environment,
        depth: usize,
    ) -> Result<
        (
            Value,
            CallArgs,
            ValueStackReservation,
            ValueStackReservation,
        ),
        RuntimeError,
    > {
        if is_arrow {
            let mut target = self.evaluate(object, env, depth + 1)?;
            let target_sweep_cursor = object_reference_sweep_cursor();
            let target_slot = ValueStackReservation::reserve(1)?;
            let evaluated_args = self.build_call_args(name, function, args, env, depth + 1)?;
            let parameter_slots = ValueStackReservation::reserve(MAX_CALL_PARAMETERS)?;
            clear_value_for_object_reference_sweeps(&mut target, target_sweep_cursor);
            if matches!(target, Value::Nil | Value::Object(0))
                || !self.object_target_available(&target)
            {
                return Err(RuntimeError::new("Object call: target is zero!"));
            }
            return Ok((target, evaluated_args, target_slot, parameter_slots));
        }

        // Direct Local/LocalN syntax evaluates its ordinary parameter first
        // and object parameter second. Preserve that source order even though
        // AssignmentTarget stores the object separately for cell lookup.
        let mut direct_args = args.to_vec();
        direct_args.push(object.clone());
        let mut evaluated_args =
            self.build_call_args(name, function, &direct_args, env, depth + 1)?;
        let parameter_slots = ValueStackReservation::reserve(native_parameter_slots)?;
        let object = evaluated_args
            .pop()
            .map(|value| value.read())
            .transpose()?
            .unwrap_or(Value::Nil);
        Ok((
            object,
            evaluated_args,
            ValueStackReservation::empty(),
            parameter_slots,
        ))
    }

    fn expr_to_lvalue(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<LValueRef, RuntimeError> {
        let target = Self::expr_to_assignment_target(expr)?;
        // A direct call is already the terminal lvalue-producing operation.
        // Sending it through `assignment_target_to_reference_or_value` would
        // reconstruct the same call and re-enter `expr_to_lvalue` forever for
        // built-ins such as Global() and LocalN(). Nested property/index
        // targets still need the reference-or-value path so legacy element
        // pins can resolve while their trailing operations are evaluated.
        if matches!(&target, AssignmentTarget::FunctionCall { .. }) {
            return self.assignment_target_to_lvalue(env, &target, depth);
        }
        match self.assignment_target_to_reference_or_value(env, &target, depth)? {
            ReturnValue::Reference(reference) => Ok(reference),
            ReturnValue::Value(_) => Err(RuntimeError::new(
                "this assignment target is a value, not a reference",
            )),
        }
    }

    fn evaluate_reference_function_call(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Option<ReturnValue>, RuntimeError> {
        let Expr::Call {
            callee,
            args,
            is_optional,
            forward_rest,
        } = expr
        else {
            return Ok(None);
        };
        let Expr::Variable(name) = callee.as_ref() else {
            return Ok(None);
        };
        // Reference-retaining contexts take this fast path before ordinary
        // call evaluation. Preserve the same identifier precedence here: a
        // parameter/variable named `this` must not escape to a `func &this`.
        if name == "this" && self.has_bound_this(env) {
            return Err(RuntimeError::new("cannot call bound variable 'this'"));
        }
        if *is_optional {
            return Ok(None);
        }
        let function = if env.engine_scope {
            self.engine_script_function(name)
        } else {
            self.own_or_global_script_function(name)
        };
        let Some(function) = function.filter(|function| function.returns_reference) else {
            return Ok(None);
        };

        let mut evaluated_args =
            self.build_call_args(Some(name), Some(function), args, env, depth + 1)?;
        if *forward_rest {
            Self::append_forwarded_args(&mut evaluated_args, env, MAX_CALL_PARAMETERS)?;
        }
        let value = if env.engine_scope {
            self.invoke_engine_raw(
                name,
                evaluated_args,
                depth + 1,
                env.object_state.clone(),
                Some(env.caller_context()),
            )?
        } else {
            self.invoke_raw(
                name,
                evaluated_args,
                depth + 1,
                env.object_state.clone(),
                Some(env.caller_context()),
            )?
        };
        Ok(Some(value))
    }

    fn evaluate_conditional_reference_builtin(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Option<ReturnValue>, RuntimeError> {
        let Expr::Call {
            callee,
            args,
            is_optional: false,
            ..
        } = expr
        else {
            return Ok(None);
        };
        let Expr::Variable(name) = callee.as_ref() else {
            return Ok(None);
        };
        let function = if env.engine_scope {
            self.engine_script_function(name)
        } else {
            self.own_or_global_script_function(name)
        };
        if function.is_some() || self.has_host_function(name) {
            return Ok(None);
        }

        if name == "GlobalN" && args.len() == 1 {
            return Ok(Some(
                match self.evaluate_named_global(args, env, depth + 1)? {
                    Some(cell) => ReturnValue::Reference(self.tracked_cell(cell)),
                    None => ReturnValue::Value(TrackedValue::runtime(Value::Nil)),
                },
            ));
        }
        if name == "VarN" {
            let values = self.build_call_args(None, None, args, env, depth + 1)?;
            let _parameter_slot = ValueStackReservation::reserve(1)?;
            return self
                .invoke_varn_raw(&values, Some(&env.caller_context()))
                .map(Some);
        }
        if matches!(name.as_str(), "Local" | "Var") && args.len() <= 1 {
            if name == "Local" && self.retain_global_call_context_for_host_paths {
                return Ok(Some(ReturnValue::Value(TrackedValue::runtime(Value::Nil))));
            }
            let index = self.evaluate_slot_index(
                if name == "Local" { "Local()" } else { "Var()" },
                args.first().unwrap_or(&Expr::Literal(Literal::Int(0))),
                env,
                depth + 1,
            )?;
            let _parameter_slots =
                ValueStackReservation::reserve(if name == "Local" { 2 } else { 1 })?;
            if name == "Local" && index < 0 {
                return Ok(Some(ReturnValue::Value(TrackedValue::runtime(Value::Nil))));
            }
            let cell = if name == "Local" {
                env.object_state.local_slot_cell(index)
            } else {
                frame_slot_cell(&env.frame_locals, index)
            };
            return Ok(Some(ReturnValue::Reference(self.tracked_cell(cell))));
        }
        if name == "Par" && args.len() <= 1 {
            let index = self.evaluate_slot_index(
                "Par",
                args.first().unwrap_or(&Expr::Literal(Literal::Int(0))),
                env,
                depth + 1,
            )?;
            let _parameter_slot = ValueStackReservation::reserve(1)?;
            let value = usize::try_from(index)
                .ok()
                .filter(|index| *index < MAX_CALL_PARAMETERS)
                .and_then(|index| env.call_args.get(index));
            return Ok(Some(match value {
                Some(value) => ReturnValue::Reference(value.lvalue()),
                None => ReturnValue::Value(TrackedValue::runtime(Value::Nil)),
            }));
        }
        Ok(None)
    }

    fn call_expression_returns_reference(&self, expr: &Expr, env: &Environment) -> bool {
        let Expr::Call { callee, args, .. } = expr else {
            return false;
        };
        match callee.as_ref() {
            Expr::Variable(name) => {
                let function = if env.engine_scope {
                    self.engine_script_function(name)
                } else {
                    self.own_or_global_script_function(name)
                };
                if function.is_some_and(|function| function.returns_reference) {
                    return true;
                }
                if function.is_some() {
                    return false;
                }

                let null_implicit_local = self.retain_global_call_context_for_host_paths
                    && (name == "Local" && args.len() <= 1 || name == "LocalN" && args.len() == 1);
                if null_implicit_local {
                    return false;
                }

                name == "EffectVar"
                    || !self.has_host_function(name)
                        && (matches!(name.as_str(), "Var" | "Local") && args.len() <= 2
                            || name == "Par" && args.len() <= 1
                            || name == "VarN"
                            || name == "LocalN" && (1..=2).contains(&args.len())
                            || name == "Global"
                            || name == "GlobalN" && args.len() == 1)
            }
            Expr::Property(_, method) => {
                matches!(method.as_str(), "Local" | "LocalN" | "Var" | "EffectVar")
                    || self
                        .own_or_global_script_function(method)
                        .is_some_and(|function| function.returns_reference)
            }
            _ => false,
        }
    }

    /// Evaluate an expression exactly once while retaining the distinction
    /// between a C4Value reference and an ordinary value. AB_ARRAY_APPEND
    /// needs that distinction: replacing a self-owned temporary array with a
    /// reference to its new element destroys the container and leaves the
    /// stack value as ordinary nil (C4Value.cpp:217-227).
    fn evaluate_reference_or_value(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        let value = self.evaluate_reference_or_value_inner(expr, env, depth)?;
        ValueStackReservation::check(1)?;
        Ok(value)
    }

    fn evaluate_reference_or_value_inner(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        match expr {
            Expr::Binary(left, operation, right)
                if matches!(operation, BinaryOp::NilCoalescing)
                    || env.strict_level.unwrap_or(0) >= 2
                        && matches!(operation, BinaryOp::And | BinaryOp::Or) =>
            {
                self.evaluate_short_circuit_raw(left, operation, right, env, depth, true)
            }
            Expr::Variable(name) => {
                if let Some(reference) = env.lvalue(name).or_else(|| {
                    self.global_variable_cell(name)
                        .map(|cell| self.tracked_cell(cell))
                }) {
                    Ok(ReturnValue::Reference(reference))
                } else {
                    self.evaluate_tracked(expr, env, depth)
                        .map(ReturnValue::Value)
                }
            }
            Expr::Property(base, property) => {
                let base = self.evaluate_reference_or_value(base, env, depth)?;
                let _base_slot = ValueStackReservation::reserve(1)?;
                self.property_reference_or_value(base, property, env)
            }
            Expr::Index(base, index_operand) => {
                let base = self.evaluate_reference_or_value(base, env, depth)?;
                let _base_slot = ValueStackReservation::reserve(1)?;
                self.index_reference_or_value(base, index_operand, env, depth)
            }
            Expr::ArrayAppend(base) => self.evaluate_array_append(base, env, depth),
            Expr::GlobalCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => self.invoke_global_call_raw(name, args, *failsafe, *forward_rest, env, depth),
            Expr::Call { .. } => {
                if let Some(value) = self.evaluate_reference_function_call(expr, env, depth)? {
                    return Ok(value);
                }
                if let Some(value) =
                    self.evaluate_conditional_reference_builtin(expr, env, depth)?
                {
                    return Ok(value);
                }
                if self.call_expression_returns_reference(expr, env) {
                    let normalized;
                    let reference_expr = if let Expr::Call {
                        callee,
                        args,
                        is_optional: true,
                        forward_rest,
                    } = expr
                    {
                        normalized = Expr::Call {
                            callee: callee.clone(),
                            args: args.clone(),
                            is_optional: false,
                            forward_rest: *forward_rest,
                        };
                        &normalized
                    } else {
                        expr
                    };
                    self.expr_to_lvalue(reference_expr, env, depth)
                        .map(ReturnValue::Reference)
                } else {
                    self.evaluate_tracked(expr, env, depth)
                        .map(ReturnValue::Value)
                }
            }
            Expr::PreIncrement(expr) => self.update_counter_raw(expr, env, 1, false, "increment"),
            Expr::PreDecrement(expr) => self.update_counter_raw(expr, env, -1, false, "decrement"),
            Expr::Assignment(target, value)
                if !matches!(target, AssignmentTarget::InvalidValue { .. }) =>
            {
                self.evaluate_plain_assignment_raw(target, value, env, depth)
            }
            Expr::ArrayAppendAssignment {
                target,
                operation,
                operator,
                value,
            } => self.evaluate_reference_assignment_raw(
                target,
                AssignmentOperator {
                    operation: operation.as_ref(),
                    spelling: operator,
                },
                value,
                env,
                depth,
                true,
            ),
            Expr::CompoundAssignment {
                target,
                operation,
                operator,
                value,
            } => self.evaluate_reference_assignment_raw(
                target,
                AssignmentOperator {
                    operation: Some(operation),
                    spelling: operator,
                },
                value,
                env,
                depth,
                true,
            ),
            _ => self
                .evaluate_tracked(expr, env, depth)
                .map(ReturnValue::Value),
        }
    }

    fn property_reference_or_value(
        &self,
        base: ReturnValue,
        property: &str,
        env: &Environment,
    ) -> Result<ReturnValue, RuntimeError> {
        self.property_reference_or_value_with_hook_stack(base, property, env, None)
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

    fn index_reference_or_value(
        &self,
        base: ReturnValue,
        index_operand: &IndexOperand,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        let (index, _index_slot) = self.evaluate_index_operand(index_operand, env, depth)?;
        self.index_value_reference_or_value(base, index, env)
    }

    fn index_value_reference_or_value(
        &self,
        base: ReturnValue,
        index: Value,
        env: &Environment,
    ) -> Result<ReturnValue, RuntimeError> {
        self.index_value_reference_or_value_with_hook_stack(base, index, env, None)
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

    /// Constant string keys are embedded directly in C++ AB_MAPA_R/V and do
    /// not consume a second value-stack slot. Every dynamic index does.
    fn evaluate_index_operand(
        &self,
        index_operand: &IndexOperand,
        env: &mut Environment,
        depth: usize,
    ) -> Result<(Value, ValueStackReservation), RuntimeError> {
        match index_operand {
            IndexOperand::EmbeddedString(value) => Ok((
                Value::String(self.literal_string(value)),
                ValueStackReservation::empty(),
            )),
            IndexOperand::Dynamic(index_expr) => {
                let index = self.evaluate(index_expr, env, depth)?;
                let index_slot = ValueStackReservation::reserve(1)?;
                Ok((index, index_slot))
            }
        }
    }

    fn evaluate_array_append(
        &self,
        base: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ReturnValue, RuntimeError> {
        let base = self.evaluate_reference_or_value(base, env, depth)?;
        let _base_slot = ValueStackReservation::reserve(1)?;
        match base {
            ReturnValue::Reference(reference) => self
                .append_array_slot(reference)
                .map(ReturnValue::Reference),
            ReturnValue::Value(value) => {
                match &value.value {
                    Value::Array(elements) if elements.len() < ARRAY_MAX_SIZE => {}
                    Value::Array(_) => return Err(RuntimeError::new("out of memory")),
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
                }
                Ok(ReturnValue::Value(TrackedValue::runtime(Value::Nil)))
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

    fn evaluate_slot_index(
        &self,
        name: &str,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<i32, RuntimeError> {
        Self::slot_index_from_value(name, self.evaluate(expr, env, depth)?)
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

    fn set_local_tracked(
        &self,
        args: &[Expr],
        default_target: Option<Value>,
        env: &mut Environment,
        depth: usize,
        parameter_slots: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        // Parse_Params retains all explicit operands while evaluating them,
        // then pads/truncates to the selected call layout before FnSetLocal
        // can mutate the destination.
        let evaluated_args = self.build_call_args(None, None, args, env, depth)?;
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

    fn set_global_tracked(
        &self,
        args: &[Expr],
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        // Parse_Params evaluates every explicit argument before balancing the
        // native two-parameter frame, so even ignored surplus arguments run
        // before FnSetGlobal performs its write (C4AulParse.cpp:2311-2344).
        let mut evaluated_args = self.build_call_args(Some("SetGlobal"), None, args, env, depth)?;
        if forward_rest {
            Self::append_forwarded_args(&mut evaluated_args, env, 2)?;
        }
        let _parameter_slots = ValueStackReservation::reserve(2)?;
        self.invoke_global_builtin_raw("SetGlobal", &evaluated_args, env, depth)?
            .into_tracked()
    }

    fn expr_to_assignment_target(expr: &Expr) -> Result<AssignmentTarget, RuntimeError> {
        match expr {
            Expr::Variable(name) => Ok(AssignmentTarget::Variable(name.clone())),
            Expr::Property(base, name) => {
                let base_target = Self::expr_to_assignment_target(base)?;
                Ok(AssignmentTarget::Property(
                    Box::new(base_target),
                    name.clone(),
                ))
            }
            Expr::Index(base, index) => {
                let base_target = Self::expr_to_assignment_target(base)?;
                Ok(AssignmentTarget::Index(
                    Box::new(base_target),
                    index.clone(),
                ))
            }
            Expr::ArrayAppend(base) => Ok(AssignmentTarget::ArrayAppend(base.clone())),
            // Special case: Local(expr), Var(expr), and EffectVar(args...) are valid for increment/decrement
            Expr::Call {
                callee,
                args,
                is_optional,
                ..
            } => {
                if let Expr::Variable(ref name) = **callee {
                    if !is_optional {
                        if name == "Local" && (args.is_empty() || args.len() == 1) {
                            return Ok(AssignmentTarget::LocalSlot(Box::new(
                                args.first()
                                    .cloned()
                                    .unwrap_or(Expr::Literal(Literal::Int(0))),
                            )));
                        } else if name == "Var" && (args.is_empty() || args.len() == 1) {
                            return Ok(AssignmentTarget::VarSlot(Box::new(
                                args.first()
                                    .cloned()
                                    .unwrap_or(Expr::Literal(Literal::Int(0))),
                            )));
                        } else if name == "EffectVar" {
                            return Ok(AssignmentTarget::EffectSlot(args.clone()));
                        } else if matches!(name.as_str(), "Local" | "LocalN" | "Var")
                            && args.len() == 2
                        {
                            return Ok(AssignmentTarget::MethodSlot {
                                object: Box::new(args[1].clone()),
                                method: name.clone(),
                                args: vec![args[0].clone()],
                                is_arrow: false,
                            });
                        }
                        // NEW: Allow any function call to be used with increment/decrement
                        // This supports reference-returning functions (func &)
                        return Ok(AssignmentTarget::FunctionCall {
                            name: name.clone(),
                            args: args.clone(),
                        });
                    }
                }
                // Direct calls may return a C4Value reference. The operator
                // validates that fact at runtime, after dynamic dispatch.
                else if let Expr::Property(ref object, ref method) = **callee {
                    if !is_optional {
                        return Ok(AssignmentTarget::MethodSlot {
                            object: object.clone(),
                            method: method.clone(),
                            args: args.clone(),
                            is_arrow: true,
                        });
                    }
                }
                Err(RuntimeError::new(format!(
                    "invalid increment/decrement target: {:?}",
                    expr
                )))
            }
            Expr::GlobalCall {
                name,
                args,
                failsafe,
                forward_rest,
            } => Ok(AssignmentTarget::GlobalFunctionCall {
                name: name.clone(),
                args: args.clone(),
                failsafe: *failsafe,
                forward_rest: *forward_rest,
            }),
            _ => Err(RuntimeError::new(format!(
                "invalid increment/decrement target: {:?}",
                expr
            ))),
        }
    }
}

enum ControlFlow {
    Normal,
    Break,
    LoopContinue,
    Return(ReturnValue),
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
    Literal(Literal),
    Load(usize),
    LoadName(String),
    LoadPath {
        slot: usize,
        segments: Vec<CompiledPathSegment>,
    },
    Store(usize),
    Unary(UnaryOp),
    Binary(BinaryOp),
    CompoundStore {
        slot: usize,
        operation: BinaryOp,
        operator: &'static str,
    },
    IncrementSlot {
        slot: usize,
        delta: i32,
    },
    IncrementEffectSlot {
        argument_count: usize,
        delta: i32,
        return_old: bool,
    },
    Call {
        site: usize,
    },
    MakeArray(usize),
    MakeProplist(usize),
    Pop,
    JumpAnd(usize),
    JumpOr(usize),
    JumpIfFalse(usize),
    Jump(usize),
    Return,
    Finish,
}

/// A conservative, slot-resolved instruction stream for local scalar script
/// code. Any dynamic/reference-bearing construct keeps using the full AST VM.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompiledFunction {
    slots: Vec<CompiledSlot>,
    function_vars: Vec<String>,
    instructions: Vec<CompiledInstruction>,
    call_sites: Vec<CompiledCallSite>,
    max_stack: usize,
    uses_effect_slots: bool,
    diagnostic_name: Arc<str>,
    diagnostic_source_name: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq)]
struct CompiledCallSite {
    name: String,
    argument_count: usize,
}

pub(crate) struct CompiledFunctionCache {
    params: Vec<Parameter>,
    body: Vec<Stmt>,
    strict_level: Option<u8>,
    returns_reference: bool,
    compiled: Option<Arc<CompiledFunction>>,
}

impl CompiledFunctionCache {
    fn new(function: &Function) -> Self {
        Self {
            params: function.params.clone(),
            body: function.body.clone(),
            strict_level: function.strict_level,
            returns_reference: function.returns_reference,
            compiled: CompiledFunction::compile(function).map(Arc::new),
        }
    }

    fn validated(&self, function: &Function, validate_source: bool) -> Option<&CompiledFunction> {
        #[cfg(test)]
        if validate_source {
            COMPILED_SOURCE_VALIDATIONS.with(|count| count.set(count.get() + 1));
        }
        (!validate_source
            || self.params == function.params
                && self.body == function.body
                && self.strict_level == function.strict_level
                && self.returns_reference == function.returns_reference)
            .then_some(self.compiled.as_deref())
            .flatten()
    }
}

struct CompiledFunctionBuilder {
    slots: Vec<CompiledSlot>,
    bare_slots: FxHashMap<String, usize>,
    function_var_slots: FxHashMap<String, usize>,
    function_vars: Vec<String>,
    instructions: Vec<CompiledInstruction>,
    call_sites: Vec<CompiledCallSite>,
    stack_depth: usize,
    max_stack: usize,
    uses_effect_slots: bool,
    strict_level: Option<u8>,
}

impl CompiledFunctionBuilder {
    fn new(function: &Function) -> Option<Self> {
        if function.returns_reference || function.params.iter().any(|param| param.is_reference) {
            return None;
        }

        let mut builder = Self {
            slots: Vec::new(),
            bare_slots: FxHashMap::default(),
            function_var_slots: FxHashMap::default(),
            function_vars: Vec::new(),
            instructions: Vec::new(),
            call_sites: Vec::new(),
            stack_depth: 0,
            max_stack: 0,
            uses_effect_slots: false,
            strict_level: function.strict_level,
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
            Expr::Literal(literal) => {
                self.push_instruction(CompiledInstruction::Literal(literal.clone()));
            }
            Expr::Variable(name) => match self.bare_slots.get(name).copied() {
                Some(slot) => self.push_instruction(CompiledInstruction::Load(slot)),
                None => self.push_instruction(CompiledInstruction::LoadName(name.clone())),
            },
            Expr::LegacyParameterList {
                args,
                forward_rest: false,
            } if args.len() == 1 => {
                self.compile_expression(&args[0])?;
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
                let Expr::Call {
                    callee,
                    args,
                    is_optional: false,
                    forward_rest: false,
                } = value.as_ref()
                else {
                    return None;
                };
                if !matches!(callee.as_ref(), Expr::Variable(name) if name == "EffectVar") {
                    return None;
                }
                for argument in args {
                    self.compile_expression(argument)?;
                }
                self.uses_effect_slots = true;
                let delta = if matches!(expression, Expr::PreIncrement(_) | Expr::PostIncrement(_))
                {
                    1
                } else {
                    -1
                };
                self.collection_instruction(
                    args.len(),
                    CompiledInstruction::IncrementEffectSlot {
                        argument_count: args.len(),
                        delta,
                        return_old: matches!(
                            expression,
                            Expr::PostIncrement(_) | Expr::PostDecrement(_)
                        ),
                    },
                )?;
            }
            Expr::Binary(left, operation @ (BinaryOp::And | BinaryOp::Or), right)
                if self.strict_level.unwrap_or(0) >= 2 =>
            {
                self.compile_expression(left)?;
                let short_circuit = self.instructions.len();
                self.instructions.push(match operation {
                    BinaryOp::And => CompiledInstruction::JumpAnd(usize::MAX),
                    BinaryOp::Or => CompiledInstruction::JumpOr(usize::MAX),
                    _ => unreachable!(),
                });
                self.stack_depth = self.stack_depth.checked_sub(1)?;
                self.compile_expression(right)?;
                let end = self.instructions.len();
                self.instructions[short_circuit] = match operation {
                    BinaryOp::And => CompiledInstruction::JumpAnd(end),
                    BinaryOp::Or => CompiledInstruction::JumpOr(end),
                    _ => unreachable!(),
                };
            }
            Expr::Binary(left, operation, right)
                if !matches!(operation, BinaryOp::Concat | BinaryOp::NilCoalescing) =>
            {
                self.compile_expression(left)?;
                self.compile_expression(right)?;
                self.binary_instruction(operation.clone())?;
            }
            Expr::Call {
                callee,
                args,
                is_optional: false,
                forward_rest: false,
            } => {
                let Expr::Variable(name) = callee.as_ref() else {
                    return None;
                };
                if matches!(
                    name.as_str(),
                    "inherited" | "_inherited" | "this" | "Par" | "SetLocal" | "SetGlobal"
                ) {
                    return None;
                }
                for argument in args {
                    self.compile_expression(argument)?;
                }
                let site = self.call_sites.len();
                self.call_sites.push(CompiledCallSite {
                    name: name.clone(),
                    argument_count: args.len(),
                });
                self.collection_instruction(args.len(), CompiledInstruction::Call { site })?;
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
                    self.compile_expression(key)?;
                    self.compile_expression(value)?;
                }
                self.collection_instruction(
                    entries.len().checked_mul(2)?,
                    CompiledInstruction::MakeProplist(entries.len()),
                )?;
            }
            _ => return None,
        }
        Some(())
    }

    fn compile_statements(&mut self, statements: &[Stmt]) -> Option<()> {
        for statement in statements {
            match statement {
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
                    self.compile_expression(value)?;
                    let slot = self.bare_slot(name);
                    self.pop_instruction(CompiledInstruction::Store(slot))?;
                }
                Stmt::Return(expression) => {
                    match expression {
                        Some(expression) => self.compile_expression(expression)?,
                        None => self.push_instruction(CompiledInstruction::Literal(Literal::Nil)),
                    }
                    self.pop_instruction(CompiledInstruction::Return)?;
                }
                Stmt::Expr(expression) => match expression {
                    Expr::CompoundAssignment {
                        target: AssignmentTarget::Variable(name),
                        operation,
                        operator,
                        value,
                    } if !matches!(operation, BinaryOp::Concat | BinaryOp::NilCoalescing) => {
                        self.compile_expression(value)?;
                        let slot = self.bare_slot(name);
                        self.pop_instruction(CompiledInstruction::CompoundStore {
                            slot,
                            operation: operation.clone(),
                            operator,
                        })?;
                    }
                    Expr::PreIncrement(value)
                    | Expr::PostIncrement(value)
                    | Expr::PreDecrement(value)
                    | Expr::PostDecrement(value) => {
                        let Expr::Variable(name) = value.as_ref() else {
                            return None;
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
                    self.compile_statements(body)?;
                    self.instructions.push(CompiledInstruction::Jump(start));
                    let end = self.instructions.len();
                    self.instructions[end_jump] = CompiledInstruction::JumpIfFalse(end);
                }
                Stmt::Block(statements) | Stmt::Sequence(statements) => {
                    self.compile_statements(statements)?;
                }
                _ => return None,
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
        Some(CompiledFunction {
            slots: self.slots,
            function_vars: self.function_vars,
            instructions: self.instructions,
            call_sites: self.call_sites,
            max_stack: self.max_stack,
            uses_effect_slots: self.uses_effect_slots,
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
    retained_operands: usize,
) -> Result<TrackedValue, RuntimeError> {
    match binding {
        Binding::Direct { value, identity } => {
            let identity = legacy_identity_for_value_copy(value, &[], identity.borrow().clone());
            let value = value.borrow();
            if register_root {
                vm.register_runtime_value(&value);
            }
            read_compiled_path_value(
                vm,
                env,
                &value,
                identity,
                segments,
                env.strict_level,
                retained_operands,
            )
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
                retained_operands,
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
                retained_operands,
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
    retained_operands: usize,
) -> Result<TrackedValue, RuntimeError> {
    let _pin_creation = LegacyPathPinCreationGuard::suspend();
    let mut current = ReturnValue::Reference(binding.lvalue());
    for segment in segments {
        current = match segment {
            CompiledPathSegment::Property(property) => vm
                .property_reference_or_value_with_hook_stack(
                    current,
                    property,
                    env,
                    Some(retained_operands + 1),
                )?,
            CompiledPathSegment::EmbeddedIndex(value) => vm
                .index_value_reference_or_value_with_hook_stack(
                    current,
                    Value::String(vm.literal_string(value)),
                    env,
                    Some(retained_operands + 1),
                )?,
            CompiledPathSegment::LiteralIndex(literal) => {
                let index = TrackedValue::literal(vm.literal_value(literal, strict_level), literal)
                    .set_copy();
                vm.register_runtime_value(&index.value);
                vm.index_value_reference_or_value_with_hook_stack(
                    current,
                    index.value,
                    env,
                    Some(retained_operands + 2),
                )?
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
    retained_operands: usize,
) -> Result<TrackedValue, RuntimeError> {
    if c4_set_copy_is_zero_id(current) {
        return read_compiled_path_value(
            vm,
            env,
            &Value::Nil,
            None,
            segments,
            strict_level,
            retained_operands,
        );
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
                        retained_operands,
                    )
                }
                Value::Object(0) => Err(RuntimeError::new(
                    "map access with .: map expected, but got nil!",
                )),
                target @ Value::Object(_) => {
                    let _hook_stack =
                        compiled_object_hook_stack(target, Some(retained_operands + 1))?;
                    let child = vm.object_local_tracked(env, target, property).set_copy();
                    vm.register_runtime_value(&child.value);
                    read_compiled_path_value(
                        vm,
                        env,
                        &child.value,
                        child.identity,
                        remaining,
                        strict_level,
                        retained_operands,
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
            let index = Value::String(vm.literal_string(value));
            if matches!(segment, CompiledPathSegment::LiteralIndex(_)) {
                vm.register_runtime_value(&index);
            }
            read_compiled_index(
                vm,
                env,
                current,
                identity,
                index,
                remaining,
                strict_level,
                retained_operands,
                matches!(segment, CompiledPathSegment::LiteralIndex(_)),
            )
        }
        CompiledPathSegment::LiteralIndex(literal) => {
            let index = c4_set_copy_value(vm.literal_value(literal, strict_level));
            vm.register_runtime_value(&index);
            read_compiled_index(
                vm,
                env,
                current,
                identity,
                index,
                remaining,
                strict_level,
                retained_operands,
                true,
            )
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
    retained_operands: usize,
    dynamic_index_slot: bool,
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
            read_compiled_path_value(
                vm,
                env,
                child,
                child_identity,
                remaining,
                strict_level,
                retained_operands,
            )
        }
        Value::Proplist(entries) => {
            let nil = Value::Nil;
            let child = entries.get_key(&index).unwrap_or(&nil);
            read_compiled_path_value(
                vm,
                env,
                child,
                child_identity,
                remaining,
                strict_level,
                retained_operands,
            )
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
                retained_operands,
            )
        }
        target @ Value::Object(_) => {
            let _hook_stack = compiled_object_hook_stack(
                target,
                Some(retained_operands + 1 + usize::from(dynamic_index_slot)),
            )?;
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
                retained_operands,
            )
        }
        other => Err(RuntimeError::new(format!(
            "cannot index value of type {}",
            other.type_name()
        ))),
    }
}

impl CompiledFunction {
    fn compile(function: &Function) -> Option<Self> {
        CompiledFunctionBuilder::new(function)?.finish(function)
    }

    fn bindings(&self, env: &Environment) -> Option<SmallVec<[Binding; 16]>> {
        let bindings = self
            .slots
            .iter()
            .map(|slot| match slot.kind {
                CompiledSlotKind::Bare => env.binding(&slot.name),
                CompiledSlotKind::FunctionVar => env.function_var_binding(&slot.name),
            })
            .collect::<Option<SmallVec<_>>>()?;
        #[cfg(test)]
        if bindings.spilled() {
            COMPILED_BINDING_HEAP_SPILLS.with(|count| count.set(count.get() + 1));
        }
        Some(bindings)
    }

    fn execute(
        &self,
        vm: &Vm<'_>,
        env: &Environment,
        depth: usize,
    ) -> Result<Option<ControlFlow>, RuntimeError> {
        let Some(bindings) = self.bindings(env) else {
            return Ok(None);
        };
        if ValueStackReservation::check(self.max_stack).is_err() {
            return Ok(None);
        }
        if self.uses_effect_slots && !vm.host_functions.contains_key("EffectVar") {
            return Ok(None);
        }
        let mut call_targets = SmallVec::<[CompiledCallTarget<'_>; 32]>::new();
        for site in &self.call_sites {
            let name = &site.name;
            let argument_count = site.argument_count;
            if (0..argument_count).any(|index| {
                vm.reference_parameter_probe
                    .is_some_and(|probe| probe(name, index))
            }) {
                return Ok(None);
            }
            if let Some(target) = vm.resolved_script_function(name, env.engine_scope) {
                if target.function.returns_reference
                    || target
                        .function
                        .params
                        .iter()
                        .any(|param| param.is_reference)
                {
                    return Ok(None);
                }
                call_targets.push(CompiledCallTarget::Script(target));
                continue;
            }
            if vm.host_reference_function(name).is_some()
                || vm
                    .host_function_parameter_types
                    .and_then(|types| types.get(name))
                    .is_some_and(|types| types.contains(&C4VType::Ref))
            {
                return Ok(None);
            }
            if let Some(target @ ResolvedHostFunction::Value(_)) = vm.resolved_host_function(name) {
                call_targets.push(CompiledCallTarget::Host(target));
                continue;
            }
            let legacy_constant = env.strict_level.unwrap_or(0) < 2
                && argument_count == 0
                && (vm.global_constant_cell(name).is_some()
                    || vm
                        .constants
                        .is_some_and(|constants| constants.contains_key(name)));
            if legacy_constant {
                call_targets.push(CompiledCallTarget::LegacyConstant);
                continue;
            }
            return Ok(None);
        }
        let mut stack = SmallVec::<[TrackedValue; 16]>::with_capacity(self.max_stack);
        let mut registered_slots = SmallVec::<[bool; 16]>::from_elem(false, bindings.len());
        #[cfg(test)]
        if stack.spilled() {
            COMPILED_STACK_HEAP_SPILLS.with(|count| count.set(count.get() + 1));
        }
        #[cfg(test)]
        if registered_slots.spilled() {
            COMPILED_REGISTERED_SLOT_HEAP_SPILLS.with(|count| count.set(count.get() + 1));
        }
        let mut instruction = 0;
        loop {
            match &self.instructions[instruction] {
                CompiledInstruction::Literal(literal) => {
                    let value =
                        TrackedValue::literal(vm.literal_value(literal, env.strict_level), literal)
                            .set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(value);
                }
                CompiledInstruction::Load(slot) => {
                    let value = bindings[*slot].read_tracked()?.set_copy();
                    if !registered_slots[*slot] {
                        vm.register_runtime_value(&value.value);
                        registered_slots[*slot] = true;
                    }
                    stack.push(value);
                }
                CompiledInstruction::LoadName(name) => {
                    let value = vm.compiled_named_value(name, env)?;
                    vm.register_runtime_value(&value.value);
                    stack.push(value);
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
                            &bindings[*slot],
                            segments,
                            env.strict_level,
                            stack.len(),
                        )?
                    } else {
                        let value = read_compiled_path(
                            vm,
                            env,
                            &bindings[*slot],
                            segments,
                            !has_index && !registered_slots[*slot],
                            stack.len(),
                        )?;
                        if has_index {
                            vm.register_runtime_value(&value.value);
                        } else {
                            registered_slots[*slot] = true;
                        }
                        value
                    };
                    stack.push(value);
                }
                CompiledInstruction::Store(slot) => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    bindings[*slot].write_tracked(value)?;
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::Unary(operation) => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let value =
                        TrackedValue::runtime(vm.eval_unary(operation, value.value)?).set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(value);
                }
                CompiledInstruction::Binary(operation) => {
                    let right = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let left = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let value = match operation {
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
                    stack.push(value);
                }
                CompiledInstruction::CompoundStore {
                    slot,
                    operation,
                    operator,
                } => {
                    let right = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let reference = bindings[*slot].lvalue();
                    let left = reference.read_tracked()?;
                    let value = TrackedValue::runtime(vm.eval_binary(
                        left.value,
                        operation,
                        right.value,
                        env.strict_level,
                        Some(operator),
                    )?);
                    reference.write_tracked(value)?;
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::IncrementSlot { slot, delta } => {
                    let reference = bindings[*slot].lvalue();
                    let operation = if *delta > 0 { "increment" } else { "decrement" };
                    let old_value = Vm::counter_operand(reference.read()?, operation)?;
                    reference.write(Value::Int(old_value.wrapping_add(*delta)))?;
                    registered_slots[*slot] = true;
                }
                CompiledInstruction::IncrementEffectSlot {
                    argument_count,
                    delta,
                    return_old,
                } => {
                    let argument_start = stack
                        .len()
                        .checked_sub(*argument_count)
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let _retained_stack = ValueStackReservation::reserve(argument_start)?;
                    let arguments = stack
                        .drain(argument_start..)
                        .map(CallArg::Value)
                        .collect::<CallArgs>();
                    #[cfg(test)]
                    record_call_arg_heap_spill(arguments.spilled());
                    let function = vm
                        .host_functions
                        .get("EffectVar")
                        .expect("compiled effect-slot host prevalidated");
                    let caller = env.caller_context();
                    let arg_values = {
                        let _parameter_slots = ValueStackReservation::reserve(
                            function.parameter_count().unwrap_or(3),
                        )?;
                        let _guard = CallerContextGuard::enter(Some(caller.clone()));
                        let prepared_args =
                            vm.prepare_registered_host_call_args("EffectVar", function, arguments)?;
                        vm.call_args_to_values(&prepared_args)?.into_vec()
                    };
                    let reference = LValueRef::HostPath {
                        function: function.callback().clone(),
                        args: arg_values,
                        caller,
                        global_call_context_hook: vm
                            .retain_global_call_context_for_host_paths
                            .then(|| vm.global_call_context_hook.cloned())
                            .flatten(),
                        segments: Vec::new(),
                        legacy_pin: None,
                    };
                    let _operand_slot = ValueStackReservation::reserve(1)?;
                    let operation = if *delta > 0 { "increment" } else { "decrement" };
                    let old_value = Vm::counter_operand(reference.read()?, operation)?;
                    let new_value = old_value.wrapping_add(*delta);
                    reference.write(Value::Int(new_value))?;
                    let value = if *return_old {
                        TrackedValue::runtime(Value::Int(old_value))
                    } else {
                        // Prefix AB_Inc1/AB_Dec1 leaves the reference on the
                        // stack. Materialize it after the host write so an
                        // invalid EffectVar slot retains the host's nil result.
                        reference.read_tracked()?
                    }
                    .set_copy();
                    vm.register_runtime_value(&value.value);
                    stack.push(value);
                }
                CompiledInstruction::Call { site } => {
                    let call_site = &self.call_sites[*site];
                    let name = &call_site.name;
                    let argument_count = call_site.argument_count;
                    let argument_start = stack
                        .len()
                        .checked_sub(argument_count)
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let _retained_stack = ValueStackReservation::reserve(argument_start)?;
                    let arguments = stack
                        .drain(argument_start..)
                        .map(CallArg::Value)
                        .collect::<CallArgs>();
                    #[cfg(test)]
                    record_call_arg_heap_spill(arguments.spilled());
                    let target = call_targets[*site];
                    let sweep_cursor = object_reference_sweep_cursor();
                    let value = match target {
                        CompiledCallTarget::Host(target) => {
                            TrackedValue::runtime(vm.invoke_resolved_host_value(
                                name,
                                target,
                                arguments,
                                depth + 1,
                                Some(env.caller_context()),
                            )?)
                        }
                        CompiledCallTarget::Script(target) => vm
                            .invoke_resolved_script_tracked_value(
                                name,
                                target,
                                arguments,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.caller_context()),
                            )?,
                        CompiledCallTarget::LegacyConstant => {
                            debug_assert!(arguments.is_empty());
                            vm.compiled_named_value(name, env)?
                        }
                    };
                    for retained in &mut stack {
                        retained.clear_object_reference_sweeps(sweep_cursor);
                    }
                    vm.register_runtime_value(&value.value);
                    stack.push(value);
                }
                CompiledInstruction::MakeArray(element_count) => {
                    let start = stack
                        .len()
                        .checked_sub(*element_count)
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    let value = TrackedValue::array(stack.drain(start..).collect()).set_copy();
                    stack.push(value);
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
                            entries.push((key.value, value));
                        }
                        entries
                    };
                    let value = TrackedValue::proplist(entries).set_copy();
                    stack.push(value);
                }
                CompiledInstruction::Pop => {
                    stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                }
                CompiledInstruction::JumpAnd(target) => {
                    let condition = stack
                        .last()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    if !condition.value.as_bool() {
                        instruction = *target;
                        continue;
                    }
                    stack.pop();
                }
                CompiledInstruction::JumpOr(target) => {
                    let condition = stack
                        .last()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    if condition.value.as_bool() {
                        instruction = *target;
                        continue;
                    }
                    stack.pop();
                }
                CompiledInstruction::JumpIfFalse(target) => {
                    let condition = stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("internal compiled stack underflow"))?;
                    if !condition.value.as_bool() {
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
                    return Ok(Some(ControlFlow::Return(ReturnValue::Value(value))));
                }
                CompiledInstruction::Finish => return Ok(Some(ControlFlow::Normal)),
            }
            instruction += 1;
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

/// Collect every `var` name in a function body (all nesting levels) and
/// pre-declare it nil: C4Aul vars are FUNCTION-scoped — the parser fills
/// Fn->VarNamed before execution, so a read before the `var` statement is
/// nil, never an "undefined variable" error.
fn hoist_function_vars(body: &[Stmt], env: &mut Environment) {
    for statement in body {
        match statement {
            Stmt::VarDecl { name, .. } => env.declare_hoisted(name),
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                hoist_function_vars(then_branch, env);
                if let Some(else_branch) = else_branch {
                    hoist_function_vars(else_branch, env);
                }
            }
            Stmt::While { body, .. } => hoist_function_vars(body, env),
            Stmt::For { init, body, .. } => {
                if let Some(ForInit::VarDecls(declarations)) = init {
                    for (name, _) in declarations {
                        env.declare_hoisted(name);
                    }
                }
                hoist_function_vars(body, env);
            }
            Stmt::ForIn {
                variable,
                value_variable,
                body,
                ..
            } => {
                // C4Aul's pre-parser adds foreach binders to the function's
                // named-var table even when the optional `var` is omitted.
                env.declare_hoisted(variable);
                if let Some(value_variable) = value_variable {
                    env.declare_hoisted(value_variable);
                }
                hoist_function_vars(body, env);
            }
            Stmt::Block(inner) | Stmt::Sequence(inner) => hoist_function_vars(inner, env),
            _ => {}
        }
    }
}

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
    /// Dynamic `cthr->Def` presence. Unlike the VM's owning-script identity,
    /// this is cleared by a nil-object DirectExec and `global->`.
    definition_context: bool,
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
            definition_context: false,
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

    fn object_reference_cells(&self, vm: &Vm<'_>) -> Vec<ValueCell> {
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
        cells.extend(self.object_state.named_locals.borrow().values().cloned());
        cells.extend(self.object_state.local_slots.borrow().values().cloned());
        if let Some(globals) = vm.globals_named {
            cells.extend(globals.borrow().values().cloned());
        }
        if let Some(globals) = vm.globals_numbered {
            cells.extend(globals.borrow().values().cloned());
        }
        if let Some(globals) = vm.globals_consts {
            cells.extend(globals.borrow().values().cloned());
        }
        cells
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

    fn push_scope(&mut self) {
        self.scopes.push(FxHashMap::default());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
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

    fn assign_function_var_tracked(
        &self,
        name: &str,
        tracked: TrackedValue,
    ) -> Result<(), RuntimeError> {
        self.frame_locals
            .function_vars
            .borrow()
            .get(name)
            .ok_or_else(|| RuntimeError::new(format!("undefined function variable '{name}'")))?
            .write_tracked(tracked)
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

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn wide_raw_bool_keeps_native_union_equality_and_low_word_bool_semantics() {
        let raw = 1_usize << 32;
        let wide_bool = Value::from_c4_bool_data_raw(raw);
        let source_id = Value::C4Id(crate::value::c4_id_from_raw(raw));

        assert!(c4_values_equal(&wide_bool, &source_id, Some(0), None, None));
        assert!(!c4_values_equal(
            &wide_bool,
            &source_id,
            Some(2),
            None,
            None
        ));
        assert!(c4_values_equal(
            &wide_bool,
            &Value::Bool(false),
            Some(3),
            None,
            None
        ));
    }
    use crate::parser::Parser;

    fn execute_script(
        source: &str,
        entry_point: &str,
        args: &[Value],
    ) -> Result<Value, RuntimeError> {
        let script = Parser::new(source)
            .parse_script_strict()
            .expect("parse should succeed");
        let functions: FxHashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|f| (f.name.clone(), f))
            .collect();
        let host_functions = FxHashMap::default();
        let var_decls: Vec<VarDecl> = Vec::new();
        let vm = Vm::new(&functions, &host_functions, &var_decls, None);
        vm.call(entry_point, args)
    }

    #[test]
    fn local_scalar_control_flow_uses_compiled_executor() {
        reset_compiled_function_execution_count();

        let result = execute_script(
            r#"
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
            &[Value::Int(128)],
        )
        .expect("slot-resolved scalar loop runs");

        assert_eq!(result, Value::Int(379));
        assert_eq!(compiled_function_execution_count(), 1);
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

        assert_eq!(
            engine.call("Probe", &[]).expect("native call succeeds"),
            Value::Int(41)
        );
        assert_eq!(generic_host_resolutions(), 0);
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

        assert_eq!(
            engine
                .call("Probe", &[Value::Int(20)])
                .expect("native call succeeds"),
            Value::Int(41)
        );
        assert_eq!(compiled_function_execution_count(), 1);
    }

    #[test]
    fn typical_compiled_function_bindings_stay_inline() {
        // C++ addresses parameters and function vars as offsets in the active
        // C4AulExec value stack (C4AulExec.cpp:62-63,330-347), without a
        // per-call heap table for a small ordinary frame.
        reset_compiled_binding_heap_spills();
        let result = execute_script(
            "#strict 2\nfunc Probe(value) { var a = value + 1; var b = a + 1; return b; }",
            "Probe",
            &[Value::Int(39)],
        )
        .expect("compiled frame executes");

        assert_eq!(result, Value::Int(41));
        assert_eq!(compiled_binding_heap_spills(), 0);
    }

    #[test]
    fn repeated_compiled_scalar_calls_keep_executor_buffers_inline() {
        // C++ evaluates AB_CALL arguments in its fixed C4AulExec::Values stack
        // and keeps the frame's local slots there as well (C4AulExec.cpp:
        // 62-63,330-347,1217-1223), without per-call buffer allocations.
        reset_compiled_executor_heap_spills();
        let result = execute_script(
            r#"#strict 2
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
            &[Value::Int(64)],
        )
        .expect("repeated compiled calls succeed");

        assert_eq!(result, Value::Int(64));
        assert_eq!(
            (
                COMPILED_STACK_HEAP_SPILLS.with(Cell::get),
                COMPILED_REGISTERED_SLOT_HEAP_SPILLS.with(Cell::get),
                COMPILED_CALL_ARGUMENT_TEMPORARIES.with(Cell::get),
            ),
            (0, 0, 0),
        );
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

        assert_eq!(
            engine
                .call("Probe", &[Value::Int(3)])
                .expect("effect slot loop succeeds"),
            Value::Int(247)
        );
        assert_eq!(
            *writes.lock().expect("effect write log lock"),
            vec![9, 8, 7]
        );
        assert_eq!(compiled_function_execution_count(), 1);
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

        assert_eq!(
            engine.call("Probe", &[]).expect("probe succeeds"),
            Value::Nil
        );
        assert_eq!(compiled_function_execution_count(), 1);
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
        assert_eq!(
            engine.call("Probe", &[]).expect("probe succeeds"),
            Value::Int(2)
        );
        assert_eq!(compiled_function_execution_count(), 1);
        assert_eq!(
            *observed_stack_sizes.lock().expect("stack-size log lock"),
            vec![12, 12, 12],
            "the external ten-slot frame, lower operand, and counter reference stay live",
        );
        *slot.lock().expect("effect slot lock") = 2;
        observed_stack_sizes
            .lock()
            .expect("stack-size log lock")
            .clear();
        assert_eq!(
            engine.call("Interpreted", &[]).expect("probe succeeds"),
            Value::Int(2)
        );
        assert_eq!(compiled_function_execution_count(), 1);
        assert_eq!(
            *observed_stack_sizes.lock().expect("stack-size log lock"),
            vec![12, 12, 12],
            "the compiled instruction must retain exactly the AST path's C++ stack shape",
        );
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

        assert_eq!(result, Value::Nil);
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

        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn successful_object_calls_defer_diagnostic_object_formatting() {
        // C++ keeps the live C4Object pointer in the executor frame and only
        // asks GetDataString while dumping an error stack (C4AulExec.cpp:
        // 1328-1342), not on every successful function entry.
        fn format_object(id: u64) -> Option<(String, Option<String>)> {
            Some((format!("Object #{id}"), Some("CALL".to_owned())))
        }

        let script =
            Parser::new("func Helper() { return 41; } func Probe() { return Helper() + 1; }")
                .parse_script_strict()
                .expect("script parses");
        let functions = script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect::<FxHashMap<_, _>>();
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        reset_diagnostic_object_formatter_calls();
        let result = with_diagnostic_object_formatter(format_object, || {
            Vm::new(&functions, &host_functions, &var_decls, None)
                .with_this(Value::Object(7))
                .call("Probe", &[])
        })
        .expect("nested object call succeeds");

        assert_eq!(result, Value::Int(42));
        assert_eq!(diagnostic_object_formatter_calls(), 0);
    }

    #[test]
    fn compiled_diagnostic_frames_share_stable_function_strings() {
        // C++ frames retain pointers to their C4AulFunc/C4AulScript metadata;
        // stable function and source names are not copied per call
        // (C4AulExec.cpp:62-63,1328-1342).
        let script =
            Parser::new("func Helper() { return 41; } func Probe() { return Helper() + 1; }")
                .parse_script_strict()
                .expect("script parses");
        let functions = script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect::<FxHashMap<_, _>>();
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        reset_diagnostic_frame_string_allocations();
        let result = Vm::new(&functions, &host_functions, &var_decls, None)
            .call("Probe", &[])
            .expect("nested compiled call succeeds");

        assert_eq!(result, Value::Int(42));
        assert_eq!(diagnostic_frame_string_allocations(), 0);
    }

    #[test]
    fn unnamed_nil_parameter_slots_do_not_allocate_bindings() {
        // C++ keeps the ten AB_CALL parameter slots in C4AulExec::Values;
        // unused trailing nils are stack values, not heap cells
        // (C4AulExec.cpp:62-63, 1217-1223).
        reset_direct_binding_allocations();
        let result = execute_script(
            "#strict 2\nfunc Leaf() { return 41; }\nfunc Probe() { return Leaf() + 1; }",
            "Probe",
            &[],
        )
        .expect("zero-argument calls succeed");

        assert_eq!(result, Value::Int(42));
        assert_eq!(direct_binding_allocations(), 0);
    }

    #[test]
    fn nested_script_calls_keep_their_resolved_target() {
        // C++ saves the resolved function pointer back into AB_CALL before
        // invoking it (C4AulExec.cpp:1250-1297).
        reset_nested_generic_script_resolutions();
        let result = execute_script(
            "#strict 2\nfunc Leaf() { return 41; }\nfunc Probe() { return Leaf() + 1; }",
            "Probe",
            &[],
        )
        .expect("nested call succeeds");

        assert_eq!(result, Value::Int(42));
        assert_eq!(nested_generic_script_resolutions(), 0);
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

        assert_eq!(result, Value::Int(17));
        assert_eq!(locals.get("counter"), Some(&Value::Int(1)));
        assert_eq!(compiled_function_execution_count(), 2);
    }

    #[test]
    fn cloned_function_does_not_reuse_a_plan_for_mutated_source() {
        let script = Parser::new("func Probe() { return 1; }")
            .parse_script_strict()
            .expect("first source parses");
        let functions: FxHashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect();
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        Vm::new(&functions, &host_functions, &var_decls, None)
            .call_pinned_args(&functions["Probe"], Vec::new())
            .expect("original function warms its plan");

        let replacement = Parser::new("func Probe() { return 2; }")
            .parse_script_strict()
            .expect("replacement source parses")
            .functions
            .into_iter()
            .next()
            .expect("replacement function exists");
        let mut cloned = functions["Probe"].clone();
        cloned.body = replacement.body;
        let cloned_functions = FxHashMap::from_iter([(cloned.name.clone(), cloned)]);

        let value = Vm::new(&cloned_functions, &host_functions, &var_decls, None)
            .call("Probe", &[])
            .expect("mutated clone executes");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn warmed_function_does_not_reuse_a_plan_after_in_place_mutation() {
        let script = Parser::new("func Probe() { return 1; }")
            .parse_script_strict()
            .expect("first source parses");
        let mut functions: FxHashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect();
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        Vm::new(&functions, &host_functions, &var_decls, None)
            .call("Probe", &[])
            .expect("original function warms its plan");

        let replacement = Parser::new("func Probe() { return 2; }")
            .parse_script_strict()
            .expect("replacement source parses")
            .functions
            .into_iter()
            .next()
            .expect("replacement function exists");
        functions
            .get_mut("Probe")
            .expect("original function remains owned")
            .body = replacement.body;

        reset_compiled_source_validations();
        let value = Vm::new(&functions, &host_functions, &var_decls, None)
            .call_pinned_args(&functions["Probe"], Vec::new())
            .expect("mutated function executes");
        assert_eq!(value, Value::Int(2));
        assert_eq!(compiled_source_validations(), 1);
    }

    #[test]
    fn inherited_function_does_not_reuse_a_plan_after_in_place_mutation() {
        let inherited = Parser::new("func Probe() { return 1; }")
            .parse_script_strict()
            .expect("inherited source parses")
            .functions
            .into_iter()
            .next()
            .expect("inherited function exists");
        let mut function = Parser::new("#strict 2\nfunc Probe() { return inherited(); }")
            .parse_script_strict()
            .expect("overriding source parses")
            .functions
            .into_iter()
            .next()
            .expect("overriding function exists");
        function.overloaded = Some(std::sync::Arc::new(inherited));
        let mut functions = FxHashMap::from_iter([(function.name.clone(), function)]);
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        Vm::new(&functions, &host_functions, &var_decls, None)
            .call("Probe", &[])
            .expect("inherited function warms its plan");

        let replacement = Parser::new("func Probe() { return 2; }")
            .parse_script_strict()
            .expect("replacement source parses")
            .functions
            .into_iter()
            .next()
            .expect("replacement function exists");
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

        let value = Vm::new(&functions, &host_functions, &var_decls, None)
            .call_pinned_args(&functions["Probe"], Vec::new())
            .expect("mutated inherited function executes");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn global_function_does_not_reuse_a_plan_after_in_place_mutation() {
        let global = Parser::new("global func Probe() { return 1; }")
            .parse_script_strict()
            .expect("global source parses")
            .functions
            .into_iter()
            .next()
            .expect("global function exists");
        let functions = FxHashMap::default();
        let mut global_functions = FxHashMap::from_iter([(global.name.clone(), global)]);
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        Vm::new(&functions, &host_functions, &var_decls, None)
            .with_optional_globals(Some(&global_functions))
            .call("Probe", &[])
            .expect("global function warms its plan");

        let replacement = Parser::new("global func Probe() { return 2; }")
            .parse_script_strict()
            .expect("replacement source parses")
            .functions
            .into_iter()
            .next()
            .expect("replacement function exists");
        global_functions
            .get_mut("Probe")
            .expect("global function remains owned")
            .body = replacement.body;

        let value = Vm::new(&functions, &host_functions, &var_decls, None)
            .with_optional_globals(Some(&global_functions))
            .call("Probe", &[])
            .expect("mutated global function executes");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn linked_function_does_not_reuse_a_caller_warmed_plan() {
        let mut function = Parser::new("global func Probe() { return 1; }")
            .parse_script_strict()
            .expect("linked source parses")
            .functions
            .into_iter()
            .next()
            .expect("linked function exists");
        let functions = FxHashMap::from_iter([(function.name.clone(), function.clone())]);
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        Vm::new(&functions, &host_functions, &var_decls, None)
            .call_pinned_args(&function, Vec::new())
            .expect("caller-owned function warms its plan");

        let replacement = Parser::new("global func Probe() { return 2; }")
            .parse_script_strict()
            .expect("replacement source parses")
            .functions
            .into_iter()
            .next()
            .expect("replacement function exists");
        function.body = replacement.body;

        let mut engine = crate::engine::Engine::new();
        engine
            .load_script("global func Probe() { return 0; }")
            .expect("destination link parses");
        assert!(engine.link_global_access_function("Probe", function));
        assert_eq!(
            engine.call("Probe", &[]).expect("linked function executes"),
            Value::Int(2)
        );
    }

    #[test]
    fn function_debug_omits_the_derived_compilation_cache() {
        let function = Parser::new("func Probe() { return 1; }")
            .parse_script_strict()
            .expect("source parses")
            .functions
            .into_iter()
            .next()
            .expect("function exists");

        assert!(!format!("{function:?}").contains("compiled"));
    }

    #[test]
    fn compiled_repeated_path_reads_register_composite_parameter_once() {
        reset_runtime_container_registration_traversals();
        let state = Value::Proplist(ValueMap::from([
            ("a".to_string(), Value::Int(1)),
            ("b".to_string(), Value::Int(2)),
            ("c".to_string(), Value::Int(3)),
        ]));

        let result = execute_script(
            "#strict 3\nfunc Probe(state) { return state.a + state.b + state.c; }",
            "Probe",
            &[state],
        )
        .expect("compiled property reads run");

        assert_eq!(result, Value::Int(6));
        assert_eq!(runtime_container_registration_traversals(), 1);
    }

    #[test]
    fn compiled_negative_index_grows_a_referenced_empty_array() {
        reset_compiled_function_execution_count();
        let result = execute_script(
            "#strict 3\nfunc Probe() { var state = []; var ignored = state[0xffffffff]; return state; }",
            "Probe",
            &[],
        )
        .expect("negative index follows native array growth");

        assert_eq!(result, Value::Array(vec![Value::Nil]));
        assert_eq!(compiled_function_execution_count(), 1);
    }

    #[test]
    fn compiled_indexed_path_preserves_ast_string_registration_order() {
        fn run(source: &str) -> (Value, Vec<Vec<u8>>, usize) {
            let script = Parser::new(source)
                .parse_script_strict()
                .expect("source parses");
            let functions: FxHashMap<String, Function> = script
                .functions
                .into_iter()
                .map(|function| (function.name.clone(), function))
                .collect();
            let host_functions = FxHashMap::default();
            let var_decls = Vec::new();
            let registrations = crate::engine::new_string_registrations();
            let state = Value::Proplist(ValueMap::from([
                ("text".to_string(), Value::from("Zulu")),
                ("other".to_string(), Value::from("Other")),
            ]));
            reset_compiled_function_execution_count();
            let result = Vm::new(&functions, &host_functions, &var_decls, None)
                .with_string_registrations(Some(&registrations))
                .call("Probe", &[state])
                .expect("probe executes");
            let order = crate::engine::enumerate_c4_strings(&registrations, &[]);
            (result, order, compiled_function_execution_count())
        }

        let compiled = run("#strict 3\nfunc Probe(state) { return [state.text[0], state.other]; }");
        let ast =
            run("#strict 3\nfunc Probe(state) { return [state.text[0], state.other]; Unknown(); }");

        assert_eq!(compiled.0, ast.0);
        assert_eq!(compiled.1, ast.1);
        assert_eq!(compiled.2, 1);
        assert_eq!(ast.2, 0);
    }

    #[test]
    fn compiled_stack_overflow_falls_back_before_observable_execution() {
        fn run(source: &str) -> (String, Vec<Vec<u8>>) {
            let script = Parser::new(source)
                .parse_script_strict()
                .expect("source parses");
            let functions: FxHashMap<String, Function> = script
                .functions
                .into_iter()
                .map(|function| (function.name.clone(), function))
                .collect();
            let host_functions = FxHashMap::default();
            let var_decls = Vec::new();
            let registrations = crate::engine::new_string_registrations();
            let state = Value::Proplist(ValueMap::from([(
                "other".to_string(),
                Value::from("Observed"),
            )]));
            let live_state = state.clone();
            let error = Vm::new(&functions, &host_functions, &var_decls, None)
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

        assert_eq!(compiled.0, ast.0);
        assert_eq!(compiled.1, ast.1);
        assert!(!ast.1.is_empty());
    }

    #[test]
    fn compiled_object_path_hook_observes_the_ast_value_stack_depth() {
        fn run(source: &str) -> (Value, usize, usize) {
            let script = Parser::new(source)
                .parse_script_strict()
                .expect("source parses");
            let functions: FxHashMap<String, Function> = script
                .functions
                .into_iter()
                .map(|function| (function.name.clone(), function))
                .collect();
            let host_functions = FxHashMap::default();
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
            let value = Vm::new(&functions, &host_functions, &var_decls, None)
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

        assert_eq!(compiled.0, ast.0);
        assert_eq!(compiled.1, ast.1);
        assert_eq!(compiled.2, 1);
        assert_eq!(ast.2, 0);
    }

    #[test]
    fn compiled_aggregate_construction_does_not_reregister_children() {
        reset_runtime_container_registration_traversals();
        let state = Value::Proplist(ValueMap::from([("value".to_string(), Value::Int(7))]));

        let result = execute_script(
            "#strict 3\nfunc Wrap(state) { return { copy = state }; }",
            "Wrap",
            std::slice::from_ref(&state),
        )
        .expect("compiled aggregate construction runs");

        assert_eq!(
            result,
            Value::Proplist(ValueMap::from([("copy".to_string(), state)]))
        );
        assert_eq!(runtime_container_registration_traversals(), 1);
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

        let result = execute_script(
            r#"
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
            &[state, Value::Int(0), Value::Int(0)],
        )
        .expect("slot-resolved container computation runs");

        assert_eq!(
            result,
            Value::Proplist(ValueMap::from([
                (
                    "position".to_string(),
                    Value::Array(vec![Value::Int(42), Value::Int(21)])
                ),
                (
                    "velocity".to_string(),
                    Value::Array(vec![Value::Int(2), Value::Int(1)])
                ),
                ("energy".to_string(), Value::Int(99)),
            ]))
        );
        assert_eq!(compiled_function_execution_count(), 1);
    }

    #[test]
    fn ordinary_ten_slot_call_arguments_stay_inline() {
        // C++ evaluates directly into C4AulExec::Values[1024] and balances
        // script calls to C4AUL_MAX_Par == 10 without allocating a parameter
        // vector (C4AulExec.cpp:62-63, 1112-1130; C4Aul.h).
        CALL_ARG_HEAP_SPILLS.with(|count| count.set(0));
        let result = execute_script(
            r#"
                func Callee(a, b, c, d, e, f, g, h, i, j) {
                    return a + b + c + d + e + f + g + h + i + j;
                }
                func Test() { return Callee(1, 2, 3, 4, 5, 6, 7, 8, 9, 10); }
            "#,
            "Test",
            &[],
        )
        .expect("ten-argument script call runs");

        assert_eq!(result, Value::Int(55));
        assert_eq!(
            CALL_ARG_HEAP_SPILLS.with(Cell::get),
            0,
            "C4Aul's fixed-size call frame must not spill ordinary arguments to the heap"
        );
    }

    #[test]
    fn calls_inside_global_functions_stay_in_engine_scope() {
        // A global function is owned by Game.ScriptEngine, so its unqualified
        // calls resolve through that engine rather than the current object's
        // definition (C4AulParse.cpp:2808-2813). Hazard's AddLightCone must
        // therefore call the global CreateLight, not FLHH::CreateLight.
        let object_script = Parser::new(
            r#"
                func CreateLight() { return AddLightCone(); }
                func Test() { return AddLightCone(); }
            "#,
        )
        .parse_script_strict()
        .expect("object script parses");
        let object_functions: FxHashMap<String, Function> = object_script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect();
        let global_script = Parser::new(
            r#"
                global func CreateLight() { return 42; }
                global func AddLightCone() { return CreateLight(); }
            "#,
        )
        .parse_script_strict()
        .expect("global script parses");
        let global_functions: FxHashMap<String, Function> = global_script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect();
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        let vm = Vm::new(&object_functions, &host_functions, &var_decls, None)
            .with_optional_globals(Some(&global_functions));

        assert_eq!(
            vm.call("Test", &[]).expect("global call resolves"),
            Value::Int(42)
        );
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
        let script = Parser::new(source)
            .parse_script_strict()
            .expect("parse should succeed");
        let functions: FxHashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|f| (f.name.clone(), f))
            .collect();
        let host_functions = FxHashMap::default();
        let var_decls: Vec<VarDecl> = Vec::new();
        let cell = value_cell(Value::Nil);
        let hook_cell = cell.clone();
        let hook: crate::engine::LocalCellHook = std::rc::Rc::new(move |target, name| {
            (matches!(target, Value::Int(42)) && name == "__local_2").then(|| hook_cell.clone())
        });
        let vm = Vm::new(&functions, &host_functions, &var_decls, None)
            .with_local_cell_hook(Some(&hook));
        let result = vm.call("Test", &[Value::Int(42)]).expect("script runs");
        assert_eq!(result, Value::Int(85), "the read sees the earlier write");
        assert_eq!(
            *cell.borrow(),
            Value::Int(84),
            "the write landed in the foreign cell"
        );
    }

    #[test]
    fn vm_executes_basic_arithmetic() {
        let source = "func Test() { return 5 + 3; }";
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(8));
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
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(30));
    }

    #[test]
    fn function_var_shadows_same_named_object_local_with_shared_cells() {
        // C4Aul's function VarNamed table precedes the object's LocalNamed
        // table. MART relies on this: FxIntDoMagicTimer declares a temporary
        // `var pClonk` without overwriting MART's persistent `local pClonk`.
        let script = Parser::new(
            r#"
                local pClonk;
                func Timer() {
                    var pClonk;
                    pClonk = 99;
                    return pClonk;
                }
            "#,
        )
        .parse_script_strict()
        .expect("script parses");
        let var_decls = script.var_decls.clone();
        let functions: FxHashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect();
        let host_functions = FxHashMap::default();
        let vm = Vm::new(&functions, &host_functions, &var_decls, None);
        let cells = LocalCells::from_local_vars(&HashMap::from([(
            "pClonk".to_string(),
            Value::Object(574),
        )]));

        assert_eq!(
            vm.call_with_cells("Timer", &[], &cells)
                .expect("function-local assignment runs"),
            Value::Int(99)
        );
        assert_eq!(
            cells.snapshot().get("pClonk"),
            Some(&Value::Object(574)),
            "the call-scoped var must not alias the persistent object local"
        );
    }

    #[test]
    fn varn_reads_and_writes_only_named_function_vars() {
        let script = Parser::new(
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
        )
        .parse_script_strict()
        .expect("script parses");
        let var_decls = script.var_decls.clone();
        let functions: FxHashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect();
        let host_functions = FxHashMap::default();
        let vm = Vm::new(&functions, &host_functions, &var_decls, None);

        assert_eq!(
            vm.call("Probe", &[Value::Int(42), Value::Int(84)])
                .expect("VarN reads and writes the live function-var cell"),
            Value::Array(vec![
                Value::Int(5),
                Value::Int(42),
                Value::Int(7),
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn varn_without_a_script_caller_returns_nil() {
        let engine = crate::engine::Engine::new();

        assert_eq!(
            engine
                .call("VarN", &[Value::String("x".to_string().into())])
                .expect("a direct VarN dispatch is not an unknown-function error"),
            Value::Nil
        );
    }

    #[test]
    fn vm_handles_function_parameters() {
        let source = "func Add(a, b) { return a + b; }";
        let result = execute_script(source, "Add", &[Value::Int(7), Value::Int(3)]).unwrap();
        assert_eq!(result, Value::Int(10));
    }

    #[test]
    fn vm_binds_duplicate_parameter_names_like_c4value_map_names() {
        // The duplicate fourth name reuses slot zero in C4Aul; timer and
        // change consequently read call arguments 4 and 5, not 5 and 6.
        let source =
            "#strict\nfunc Merge(target, number, name, target, timer, change) { return [target, timer, change]; }";
        let result = execute_script(
            source,
            "Merge",
            &[
                Value::Int(10),
                Value::Int(20),
                Value::Int(30),
                Value::Int(40),
                Value::Int(50),
                Value::Int(60),
            ],
        )
        .expect("duplicate-name function runs");
        assert_eq!(
            result,
            Value::Array(vec![Value::Int(10), Value::Int(40), Value::Int(50)])
        );
    }

    #[test]
    fn vm_reports_undefined_variable() {
        let source = "func Test() { return undefined_var; }";
        let error = execute_script(source, "Test", &[]).unwrap_err();
        assert!(error.message().contains("undefined variable"));
    }

    #[test]
    fn vm_reports_unknown_function() {
        let source = "func Test() { return 1; }";
        let error = execute_script(source, "Missing", &[]).unwrap_err();
        assert!(error.message().contains("unknown function"));
    }

    #[test]
    fn vm_handles_nested_function_calls() {
        let source = r#"
            func Inner() { return 42; }
            func Outer() { return Inner(); }
        "#;
        let result = execute_script(source, "Outer", &[]).unwrap();
        assert_eq!(result, Value::Int(42));
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
        assert_eq!(error.message(), "internal error: value stack overflow!");
    }

    #[test]
    fn vm_handles_array_creation() {
        let source = "#strict\nfunc Test() { var arr = [1, 2, 3]; return arr[1]; }";
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(2));
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
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(42));
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
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(99));
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

        assert_eq!(
            execute_script(source, "Test", &[]).expect("array accesses succeed"),
            Value::Array(vec![
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
            ])
        );
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

        assert_eq!(
            execute_script(source, "Test", &[]).expect("negative array indices clamp"),
            Value::Array(vec![Value::Int(7), Value::Int(1), Value::Int(2)])
        );
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

        assert_eq!(
            engine
                .call("GrowThroughReference", &[])
                .expect("reference read succeeds"),
            Value::Array(vec![Value::Nil])
        );
        assert!(engine.call("GrowBeforeNestedFailure", &[]).is_err());
        assert_eq!(
            *captured.lock().unwrap(),
            Some(Value::Array(vec![Value::Nil]))
        );
    }

    #[test]
    fn vm_array_growth_stops_at_cpp_value_list_max_size() {
        let source = "#strict\nfunc Grow(index) { var a = []; a[index] = 1; return a; }";
        let grown = execute_script(source, "Grow", &[Value::Int(999_999)])
            .expect("last valid array index grows");
        let Value::Array(elements) = grown else {
            panic!("array expected");
        };
        assert_eq!(elements.len(), ARRAY_MAX_SIZE);
        assert_eq!(elements.last(), Some(&Value::Int(1)));
        drop(elements);

        match execute_script(source, "Grow", &[Value::Int(1_000_000)]) {
            Ok(_) => panic!("index at array cap unexpectedly succeeded"),
            Err(error) => assert_eq!(error.message(), "out of memory"),
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

        assert_eq!(
            execute_script(source, "Test", &[]).expect("string accesses succeed"),
            Value::Array(vec![
                Value::String("a".into()),
                Value::String("c".into()),
                Value::Nil,
                Value::Nil,
                Value::String("a".into()),
                Value::String("a".into()),
                Value::String("b".into()),
                Value::String("b".into()),
            ])
        );
    }

    #[test]
    fn dynamic_eval_reads_internal_byte_projection_as_source_bytes() {
        let source = "func Probe(string code) { return eval(code); }";
        let code = c4_string_from_bytes(&[b'\"', 0xff, b'\"']);
        assert_eq!(
            execute_script(source, "Probe", &[Value::String(code.into())])
                .expect("projected source evaluates"),
            Value::String(c4_string_from_bytes(&[0xff]).into())
        );

        assert_eq!(
            execute_script(
                source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"1\0+1").into())],
            )
            .expect("NUL-terminated source evaluates its prefix"),
            Value::Int(1)
        );
        assert_eq!(
            execute_script(
                source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"\"open\0\"").into())],
            )
            .expect("a literal truncated by NUL is a DirectExec parse failure"),
            Value::Nil
        );

        assert_eq!(
            execute_script(
                source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"1\x1f+1").into())],
            )
            .expect("all C++ control-byte whitespace is skipped"),
            Value::Int(2)
        );
        assert_eq!(
            execute_script(
                source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"1\xc2\xa0+1").into())],
            )
            .expect("non-ASCII whitespace is a DirectExec parse failure"),
            Value::Nil
        );
        assert_eq!(
            execute_script(
                source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"\"a\nb\"").into())],
            )
            .expect("a raw newline in a string is a DirectExec parse failure"),
            Value::Nil
        );
        assert_eq!(
            execute_script(
                source,
                "Probe",
                &[Value::String(c4_string_from_bytes(b"true\xc3\xbf").into())],
            )
            .expect("the non-ASCII source byte causes a DirectExec parse failure"),
            Value::Nil
        );
        assert_eq!(
            execute_script(
                source,
                "Probe",
                &[Value::String(
                    c4_string_from_bytes(b"1//comment\r+1").into()
                )],
            )
            .expect("a carriage return ends a C++ line comment"),
            Value::Int(2)
        );
    }

    #[test]
    fn host_direct_exec_reads_internal_byte_projection_as_source_bytes() {
        let functions = FxHashMap::default();
        let host_functions = FxHashMap::default();
        let var_decls = Vec::new();
        let vm = Vm::new(&functions, &host_functions, &var_decls, None);
        let source = c4_string_from_bytes(&[b'\"', 0xff, b'\"']);
        let expected = Value::String(c4_string_from_bytes(&[0xff]).into());

        let (value, _) = vm
            .direct_exec_with_locals(&source, &HashMap::new(), None)
            .expect("projected source executes with copied locals");
        assert_eq!(value, expected);

        let cells = LocalCells::default();
        assert_eq!(
            vm.direct_exec_with_cells(&source, &cells, None)
                .expect("projected source executes with live cells"),
            expected
        );

        let nul_terminated = c4_string_from_bytes(b"1\0+1");
        let (value, _) = vm
            .direct_exec_with_locals(&nul_terminated, &HashMap::new(), None)
            .expect("NUL-terminated host source evaluates its prefix");
        assert_eq!(value, Value::Int(1));

        let truncated_literal = c4_string_from_bytes(b"\"open\0\"");
        assert_eq!(
            vm.direct_exec_with_cells(&truncated_literal, &cells, None)
                .expect("a host literal truncated by NUL is a parse failure"),
            Value::Nil
        );
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

        assert_eq!(
            error.message(),
            "indexed string access: index of type string, int expected!"
        );
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

        assert_eq!(
            execute_script(source, "Test", &[]).expect("string identity checks succeed"),
            Value::Array(vec![Value::Bool(true), Value::Bool(false)])
        );
    }

    #[test]
    fn vm_handles_proplist_creation() {
        let source = "#strict 3\nfunc Test() { var obj = { x = 10 }; return obj.x; }";
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(10));
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
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(42));
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

        assert_eq!(
            execute_script(source, "Test", &[]).expect("declared map foreach runs"),
            Value::Proplist(ValueMap::from([
                ("alpha".to_string(), Value::Int(11)),
                ("beta".to_string(), Value::Int(22)),
            ]))
        );
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

        assert_eq!(
            execute_script(source, "Test", &[]).expect("predeclared map foreach runs"),
            Value::Array(vec![Value::Int(3), Value::Int(4), Value::Int(1)])
        );
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

        assert_eq!(
            execute_script(source, "Test", &[]).expect("ordered map foreach runs"),
            Value::Array(vec![
                Value::String("secondfirstthird".to_string().into()),
                Value::Int(36),
                Value::String("third".to_string().into()),
                Value::Int(3),
            ])
        );
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
            assert_eq!(
                execute_script(source, "Test", &[]).expect("map remove/reinsert foreach runs"),
                expected
            );
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
        assert!(
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

        assert_eq!(
            execute_script(
                source,
                "Test",
                &[Value::Proplist(ValueMap::from([(
                    "entry".to_string(),
                    Value::String("same".into()),
                )]))],
            )
            .expect("map entry removal runs"),
            Value::Array(vec![Value::Bool(false), Value::Bool(true)])
        );
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
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(15));
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
        let result1 = execute_script(source, "Test", &[Value::Int(15)]).unwrap();
        assert_eq!(result1, Value::Int(1));
        let result2 = execute_script(source, "Test", &[Value::Int(5)]).unwrap();
        assert_eq!(result2, Value::Nil);
    }
}
