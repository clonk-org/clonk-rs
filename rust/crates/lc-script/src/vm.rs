use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use crate::ast::{
    AccessLevel, AssignmentTarget, BinaryOp, Expr, ForInit, Function, NavigationOperation,
    Parameter, SafeNavigationStep, Stmt, UnaryOp, VarDecl,
};
use crate::debugger::DebuggerHooks;
use crate::engine::{HostFunction, HostReferenceFunction};
use crate::error::RuntimeError;
use crate::value::{Literal, Value, ValueMap};

/// Maximum script call-stack depth, matching C++ `MAX_CONTEXT_STACK`
/// (C4AulExec.cpp:62). A script recursing within this bound runs; beyond it the
/// VM returns a clean error (C++ throws "call stack overflow", :143-145).
const MAX_CALL_DEPTH: usize = 512;
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

/// Run `f` with native-stack headroom, growing the stack when it runs low. Each
/// script-call level of this tree-walking interpreter uses several KiB of native
/// stack, so deep (but C++-legal, <=512) recursion would otherwise overflow the
/// thread stack. Same thread, so thread-local host context stays visible.
fn maybe_grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(256 * 1024, 2 * 1024 * 1024, f)
}

/// String form of a value for `..` concatenation: the raw text for strings (the
/// `Display` form quotes them), and the `Display` form for everything else.
fn concat_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub type ValueCell = Rc<RefCell<Value>>;
type SlotMap = Rc<RefCell<HashMap<i32, ValueCell>>>;
type NamedLocalMap = Rc<RefCell<HashMap<String, ValueCell>>>;

pub fn value_cell(value: Value) -> ValueCell {
    Rc::new(RefCell::new(value))
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

thread_local! {
    /// The `Var(n)` slot table of the script function that invoked the
    /// currently-running HOST function — `cthr->Caller->NumVars`. None
    /// while no host function with a script caller is executing.
    static HOST_CALLER_VAR_SLOTS: RefCell<Option<SlotMap>> = const { RefCell::new(None) };
}

/// The calling script function's numbered `Var(n)` slots, exposed to host
/// functions — the `cthr->Caller->NumVars` seam (FnFindConstructionSite
/// reads and writes them, C4Script.cpp:1958-1981). None when the
/// executing host function has no script caller
/// (`if (!cthr->Caller) return {}`, :1966).
pub fn caller_var_slots() -> Option<CallerVarSlots> {
    HOST_CALLER_VAR_SLOTS
        .with(|cell| cell.borrow().clone())
        .map(CallerVarSlots)
}

/// A live handle onto the caller's numbered var slots; writes go straight
/// into the suspended call's storage like C++ reference assignment.
pub struct CallerVarSlots(SlotMap);

impl CallerVarSlots {
    /// C4ValueList::GetItem semantics: unset slots read nil.
    pub fn get(&self, index: i32) -> Value {
        slot_cell(&self.0, index).borrow().clone()
    }

    pub fn set(&self, index: i32, value: Value) {
        *slot_cell(&self.0, index).borrow_mut() = value;
    }
}

/// Scopes HOST_CALLER_VAR_SLOTS to one host-function invocation,
/// restoring the previous value on drop (nested host calls through
/// re-entrant VMs keep correct caller attribution).
struct CallerSlotsGuard(Option<SlotMap>);

impl CallerSlotsGuard {
    fn enter(slots: Option<SlotMap>) -> Self {
        Self(HOST_CALLER_VAR_SLOTS.with(|cell| cell.replace(slots)))
    }
}

impl Drop for CallerSlotsGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        HOST_CALLER_VAR_SLOTS.with(|cell| cell.replace(previous));
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RawIdentity {
    /// Parser literals share the engine string table entry for equal text.
    InternedString(String),
    /// Runtime strings and newly evaluated containers own distinct pointers.
    Heap(Rc<HeapIdentity>),
}

#[derive(Clone, Debug)]
pub(crate) enum HeapIdentity {
    Opaque,
    Array(Vec<Option<RawIdentity>>),
    Proplist(HashMap<Value, Option<RawIdentity>>),
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
                .get(&Value::String(key.clone()))
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
            (Value::Proplist(entries), segment @ (PathSegment::Property(_) | PathSegment::Index(_))) => {
                let mut identities = match current {
                    Some(Self::Proplist(identities)) => identities.clone(),
                    _ => match Self::opaque_for(value) {
                        Self::Proplist(identities) => identities,
                        _ => unreachable!(),
                    },
                };
                let key = match segment {
                    PathSegment::Property(key) => Value::String(key.clone()),
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
        matches!(
            value,
            Value::String(_) | Value::Array(_) | Value::Proplist(_)
        )
        .then(|| Self::Heap(Rc::new(HeapIdentity::opaque_for(value))))
    }

    fn identity_at(&self, segment: &PathSegment) -> Option<Self> {
        match self {
            Self::Heap(identity) => identity.identity_at(segment),
            Self::InternedString(_) => None,
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
                Self::InternedString(_) => return None,
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
            (RawIdentity::InternedString(left), RawIdentity::InternedString(right)) => {
                left == right
            }
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
        let identity = match literal {
            Literal::String(text) => Some(RawIdentity::InternedString(text.clone())),
            _ => None,
        };
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
        let mut identities = HashMap::with_capacity(entries.len());
        for (key, entry) in entries {
            if matches!(&entry.value, Value::Nil)
                && values
                    .get_key(&key)
                    .is_some_and(|value| !matches!(value, Value::Nil))
            {
                values.shift_remove_key(&key);
                identities.remove(&key);
                continue;
            }
            identities.insert(key.clone(), entry.identity);
            values.insert_key(key, entry.value);
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
}

type RawIdentityCell = Rc<RefCell<Option<RawIdentity>>>;

#[derive(Clone)]
enum Binding {
    Direct {
        value: ValueCell,
        identity: RawIdentityCell,
    },
    Reference(LValueRef),
}

impl Binding {
    fn direct(value: Value) -> Self {
        Self::tracked(TrackedValue::runtime(value))
    }

    fn tracked(tracked: TrackedValue) -> Self {
        Binding::Direct {
            value: value_cell(tracked.value),
            identity: Rc::new(RefCell::new(tracked.identity)),
        }
    }

    fn read_tracked(&self) -> Result<TrackedValue, RuntimeError> {
        match self {
            Binding::Direct { value, identity } => Ok(TrackedValue {
                value: value.borrow().clone(),
                identity: identity.borrow().clone(),
            }),
            Binding::Reference(reference) => reference.read_tracked(),
        }
    }

    fn read(&self) -> Result<Value, RuntimeError> {
        self.read_tracked().map(|tracked| tracked.value)
    }

    fn write_tracked(&self, tracked: TrackedValue) -> Result<(), RuntimeError> {
        match self {
            Binding::Direct { value, identity } => {
                *value.borrow_mut() = tracked.value;
                *identity.borrow_mut() = tracked.identity;
                Ok(())
            }
            Binding::Reference(reference) => reference.write_tracked(tracked),
        }
    }

    fn lvalue(&self) -> LValueRef {
        match self {
            Binding::Direct { value, identity } => {
                LValueRef::tracked_cell(value.clone(), identity.clone())
            }
            Binding::Reference(reference) => reference.clone(),
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
    },
    /// A reference returned by a value-style host getter/setter. The engine's
    /// `EffectVar` host uses three addressing arguments for reads and accepts
    /// the replacement value as a fourth argument for writes. Retaining the
    /// call and an optional container path models C++'s `C4V_pC4Value` through
    /// `AB_ARRAYA_R` without flattening it to a copied array.
    HostPath {
        function: HostFunction,
        args: Vec<Value>,
        caller_slots: SlotMap,
        segments: Vec<PathSegment>,
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
            } => (identity, value, &[][..]),
            Self::Path {
                root,
                root_identity: Some(identity),
                segments,
            } => (identity, root, segments.as_slice()),
            _ => return,
        };
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
        let detached = RawIdentity::Heap(Rc::new(heap.as_ref().clone()));
        *identity = if segments.is_empty() {
            Some(detached)
        } else {
            RawIdentity::after_path_write(
                identity.as_ref(),
                &root.borrow(),
                segments,
                Some(detached),
            )
        };
    }

    fn read(&self) -> Result<Value, RuntimeError> {
        self.read_tracked().map(|tracked| tracked.value)
    }

    fn read_tracked(&self) -> Result<TrackedValue, RuntimeError> {
        match self {
            LValueRef::Cell { value, identity } => Ok(TrackedValue {
                value: value.borrow().clone(),
                identity: identity
                    .as_ref()
                    .and_then(|identity| identity.borrow().clone()),
            }),
            LValueRef::Path {
                root,
                root_identity,
                segments,
            } => {
                let value = read_path(&root.borrow(), segments)?;
                let identity = root_identity.as_ref().and_then(|identity| {
                    identity
                        .borrow()
                        .as_ref()
                        .and_then(|identity| identity.identity_at_path(segments))
                });
                Ok(TrackedValue { value, identity })
            }
            LValueRef::HostPath {
                function,
                args,
                caller_slots,
                segments,
            } => {
                let _guard = CallerSlotsGuard::enter(Some(caller_slots.clone()));
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
            } => {
                let TrackedValue {
                    value,
                    identity: replacement_identity,
                } = tracked;
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
                caller_slots,
                segments,
            } => {
                let _guard = CallerSlotsGuard::enter(Some(caller_slots.clone()));
                let replacement = if segments.is_empty() {
                    tracked.value
                } else {
                    let mut root = function(args)?;
                    write_path(&mut root, segments, tracked.value)?;
                    root
                };
                let mut write_args = args.clone();
                write_args.truncate(3);
                write_args.resize(3, Value::Nil);
                write_args.push(replacement);
                function(&write_args).map(|_| ())
            }
        }
    }

    fn append(&self, segment: PathSegment) -> Self {
        match self {
            LValueRef::Cell { value, identity } => LValueRef::Path {
                root: value.clone(),
                root_identity: identity.clone(),
                segments: vec![segment],
            },
            LValueRef::Path {
                root,
                root_identity,
                segments,
            } => {
                let mut segments = segments.clone();
                segments.push(segment);
                LValueRef::Path {
                    root: root.clone(),
                    root_identity: root_identity.clone(),
                    segments,
                }
            }
            LValueRef::HostPath {
                function,
                args,
                caller_slots,
                segments,
            } => {
                let mut segments = segments.clone();
                segments.push(segment);
                LValueRef::HostPath {
                    function: function.clone(),
                    args: args.clone(),
                    caller_slots: caller_slots.clone(),
                    segments,
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) enum PathSegment {
    Property(String),
    Index(Value),
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
/// are byte buffers. Rust's public string value is UTF-8, so project the
/// selected byte onto the same-valued Latin-1 scalar to keep the one-character
/// result representable without switching the entire value model to bytes.
fn string_index(text: &str, index: &Value) -> Result<Value, RuntimeError> {
    let index = index.as_c4_int().ok_or_else(|| {
        RuntimeError::new(format!(
            "indexed string access: index of type {}, int expected!",
            index.type_name()
        ))
    })?;
    let len = i64::try_from(text.len()).unwrap_or(i64::MAX);
    let mut index = i64::from(index);
    if index < 0 {
        index += len;
    }
    let Some(byte) = usize::try_from(index)
        .ok()
        .and_then(|index| text.as_bytes().get(index))
    else {
        return Ok(Value::Nil);
    };
    Ok(Value::String(char::from(*byte).to_string()))
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
            (PathSegment::Index(key), Value::Proplist(entries)) => entries
                .get_key(key)
                .cloned()
                .unwrap_or(Value::Nil),
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
        *value = new_value;
        return Ok(());
    };

    match (value, segment) {
        (Value::Proplist(entries), PathSegment::Property(property)) => {
            if rest.is_empty() {
                if matches!(new_value, Value::Nil) {
                    entries.shift_remove(property);
                } else {
                    entries.insert(property.clone(), new_value);
                }
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
                elements[index] = new_value;
                Ok(())
            } else {
                write_path(&mut elements[index], rest, new_value)
            }
        }
        (Value::Proplist(entries), PathSegment::Index(key)) => {
            if rest.is_empty() {
                if matches!(new_value, Value::Nil) {
                    entries.shift_remove_key(key);
                } else {
                    entries.insert_key(key.clone(), new_value);
                }
                Ok(())
            } else {
                let Some(next) = entries.get_key_mut(key) else {
                    return Err(RuntimeError::new(format!("cannot access map key {key} on nil")));
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

    fn read_tracked(&self) -> Result<TrackedValue, RuntimeError> {
        match self {
            CallArg::Value(tracked) => Ok(tracked.clone()),
            CallArg::Reference(reference) => reference.read_tracked(),
        }
    }

    fn read(&self) -> Result<Value, RuntimeError> {
        self.read_tracked().map(|tracked| tracked.value)
    }
}

/// Opaque argument supplied to a reference-aware native host function.
///
/// A parameter declared reference-aware still arrives here when the script
/// expression is not an lvalue; in that case it remains readable but
/// [`HostCallArg::is_reference`] is false and [`HostCallArg::write`] returns
/// `Ok(false)`. This models nullable native `C4Value *` parameters without
/// exposing the VM's lvalue representation.
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
}

pub struct Vm<'a> {
    functions: &'a HashMap<String, Function>,
    host_functions: &'a HashMap<String, HostFunction>,
    host_reference_functions: Option<&'a HashMap<String, HostReferenceFunction>>,
    var_decls: &'a [VarDecl], // Script-level variable declarations
    debugger: Option<DebuggerHooks>,
    /// Engine-registered script constants (`RegisterGlobalConstant`,
    /// C4Script.cpp:6581): consulted when an identifier matches no variable.
    constants: Option<&'a HashMap<String, Value>>,
    /// Engine-global script functions (System.c4g `global func`s): the
    /// resolution fallback between the own script and host functions.
    global_functions: Option<&'a HashMap<String, Function>>,
    /// The object context the call runs on, returned by `Expr::This`
    /// (`Value::Object` in lc-engine). Nil when the call has no object
    /// context (e.g. global functions).
    this_value: Value,
    /// The cross-object resolver for `obj->Method(args)` (AB_CALL,
    /// C4AulExec.cpp:1216-1305), registered by the engine. Called with
    /// [target, name, failsafe, args...].
    method_dispatch: Option<&'a HostFunction>,
    /// Reference-preserving twin of `method_dispatch`, used when an arrow
    /// call occupies an lvalue position.
    method_reference_dispatch: Option<&'a crate::engine::MethodReferenceDispatch>,
    /// The engine-global `static` table (GlobalNamed); resolved after
    /// locals, before global constants (C4AulParse.cpp:2836-2839).
    globals_named: Option<&'a std::cell::RefCell<HashMap<String, ValueCell>>>,
    /// The engine-global numbered-variable table (`C4AulScriptEngine::Global`).
    globals_numbered: Option<&'a std::cell::RefCell<BTreeMap<i32, ValueCell>>>,
    /// The engine-global `static const` registry (GetGlobalConstant,
    /// C4Aul.cpp:494): script-declared constants shared across hosts,
    /// resolvable via the pre-#strict-2 `NAME()` call idiom.
    globals_consts: Option<&'a std::cell::RefCell<HashMap<String, ValueCell>>>,
    /// Cross-object LocalN cell supplier (crate::engine::LocalCellHook).
    local_cell_hook: Option<&'a crate::engine::LocalCellHook>,
    /// Per-call provenance for persistent/global cells that store only the
    /// public value representation. Nested script calls share this VM/cache.
    cell_identities: RefCell<HashMap<usize, RawIdentityCell>>,
    constant_identities: RefCell<HashMap<String, RawIdentityCell>>,
}

impl<'a> Vm<'a> {
    pub fn new(
        functions: &'a HashMap<String, Function>,
        host_functions: &'a HashMap<String, HostFunction>,
        var_decls: &'a [VarDecl],
        debugger: Option<DebuggerHooks>,
    ) -> Self {
        Self {
            functions,
            host_functions,
            host_reference_functions: None,
            var_decls,
            debugger,
            constants: None,
            global_functions: None,
            this_value: Value::Nil,
            method_dispatch: None,
            method_reference_dispatch: None,
            globals_named: None,
            globals_numbered: None,
            globals_consts: None,
            local_cell_hook: None,
            cell_identities: RefCell::new(HashMap::new()),
            constant_identities: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn with_host_reference_functions(
        mut self,
        functions: &'a HashMap<String, HostReferenceFunction>,
    ) -> Self {
        self.host_reference_functions = Some(functions);
        self
    }

    /// Set the `this` object context for this call session. Nested plain calls
    /// share it (they run on the same object).
    pub fn with_this(mut self, this: Value) -> Self {
        self.this_value = this;
        self
    }

    /// Attach the engine constants table consulted on variable-lookup misses.
    pub fn with_constants(mut self, constants: &'a HashMap<String, Value>) -> Self {
        self.constants = Some(constants);
        self
    }

    /// Attach the engine-global script functions (System.c4g global funcs);
    /// `None` = no globals installed.
    pub fn with_optional_globals(mut self, functions: Option<&'a HashMap<String, Function>>) -> Self {
        self.global_functions = functions;
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

    pub fn with_global_variables(
        mut self,
        table: Option<&'a std::cell::RefCell<HashMap<String, ValueCell>>>,
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
        table: Option<&'a std::cell::RefCell<HashMap<String, ValueCell>>>,
    ) -> Self {
        self.globals_consts = table;
        self
    }

    pub fn with_local_cell_hook(
        mut self,
        hook: Option<&'a crate::engine::LocalCellHook>,
    ) -> Self {
        self.local_cell_hook = hook;
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
        TrackedValue {
            value: cell.borrow().clone(),
            identity: self.identity_for_cell(cell).borrow().clone(),
        }
    }

    fn read_tracked_named_cell(&self, name: &str, cell: &ValueCell) -> TrackedValue {
        let value = cell.borrow().clone();
        let identity = self.identity_for_cell(cell);
        let is_script_constant = self
            .global_constant_cell(name)
            .is_some_and(|constant| Rc::ptr_eq(&constant, cell));
        if !is_script_constant {
            return self.read_tracked_cell(cell);
        }
        if let Value::String(text) = &value {
            *identity.borrow_mut() = Some(RawIdentity::InternedString(text.clone()));
        }
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
            !matches!(value, Value::Nil | Value::Int(0) | Value::Bool(false))
                && *value != self.this_value
        });
        if let Some(target) = foreign {
            if let Some(cell) = self
                .local_cell_hook
                .and_then(|hook| hook(&target, local_name))
            {
                return cell;
            }
        }
        env.object_state.named_local_cell(local_name)
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
            !matches!(value, Value::Nil | Value::Int(0) | Value::Bool(false))
                && *value != self.this_value
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
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(self.evaluate(arg, env, depth)?);
        }
        let index = match values.first().cloned().unwrap_or(Value::Nil) {
            Value::Int(index) => index,
            Value::Bool(flag) => i32::from(flag),
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
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(self.evaluate(arg, env, depth)?);
        }
        let value = values.first().cloned().unwrap_or(Value::Nil);
        let name = match value {
            Value::String(name) => name,
            Value::Nil => String::new(),
            Value::Int(0) | Value::Bool(false) if env.strict_level.unwrap_or(0) < 3 => {
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
        let args = args.iter().cloned().map(CallArg::runtime).collect();
        self.invoke_value(name, args, 0, ObjectState::default(), None)
    }

    /// Call with caller-prepared arguments (reference cells included) — the
    /// host-side C4AulParSet pattern where pars carry `GetRef()` values.
    pub(crate) fn call_args(&self, name: &str, args: Vec<CallArg>) -> Result<Value, RuntimeError> {
        self.invoke_value(name, args, 0, ObjectState::default(), None)
    }

    /// Call against SHARED local cells (see [`LocalCells`]): writes land
    /// live — deeper sessions on the same object observe them mid-call.
    pub(crate) fn call_with_cells(
        &self,
        name: &str,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<Value, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::runtime).collect();
        self.invoke_value(name, args, 0, cells.state.clone(), None)
    }

    /// Reference-returning counterpart to [`Vm::call_with_cells`].
    pub(crate) fn call_reference_with_cells(
        &self,
        name: &str,
        args: &[Value],
        cells: &LocalCells,
    ) -> Result<ValueReference, RuntimeError> {
        let args = args.iter().cloned().map(CallArg::runtime).collect();
        self.invoke_reference(name, args, 0, cells.state.clone(), None)
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
        let args = args.iter().cloned().map(CallArg::runtime).collect();
        let value = self.invoke_value(name, args, 0, object_state.clone(), None)?;
        Ok((value, object_state.to_local_vars(self.var_decls)))
    }

    /// C4AulScript::DirectExec (C4AulExec.cpp:1658-1707): parse `source`
    /// as ONE expression (ParseFn fExprOnly — trailing text is ignored)
    /// and evaluate it in the object context — the host-side twin of the
    /// script-language `eval` special form. Parse errors yield C4VNull
    /// (DirectExec's catch, :1693-1699); runtime errors propagate for the
    /// caller's fPassErrors handling. Returns (result, updated_local_vars).
    pub fn direct_exec_with_locals(
        &self,
        source: &str,
        local_vars: &HashMap<String, Value>,
        strict_level: Option<u8>,
    ) -> Result<(Value, HashMap<String, Value>), RuntimeError> {
        let object_state = ObjectState::from_local_vars(local_vars);
        let Ok(expr) = crate::parser::Parser::with_strict_level(source, strict_level)
            .parse_direct_exec_expression()
        else {
            return Ok((Value::Nil, object_state.to_local_vars(self.var_decls)));
        };
        let mut env = Environment::new_with_params(&[], &[], strict_level, object_state.clone())?;
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }
        let value = self.evaluate(&expr, &mut env, 0)?;
        Ok((value, object_state.to_local_vars(self.var_decls)))
    }

    /// [`Vm::direct_exec_with_locals`] against SHARED live cells (see
    /// [`LocalCells`]): writes land live — deeper sessions on the same
    /// object observe them mid-call.
    pub(crate) fn direct_exec_with_cells(
        &self,
        source: &str,
        cells: &LocalCells,
        strict_level: Option<u8>,
    ) -> Result<Value, RuntimeError> {
        let Ok(expr) = crate::parser::Parser::with_strict_level(source, strict_level)
            .parse_direct_exec_expression()
        else {
            return Ok(Value::Nil);
        };
        let mut env =
            Environment::new_with_params(&[], &[], strict_level, cells.state.clone())?;
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }
        self.evaluate(&expr, &mut env, 0)
    }

    fn invoke_value(
        &self,
        name: &str,
        args: Vec<CallArg>,
        depth: usize,
        object_state: ObjectState,
        caller_slots: Option<SlotMap>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_raw(name, args, depth, object_state, caller_slots)?
            .into_value()
    }

    fn invoke_tracked_value(
        &self,
        name: &str,
        args: Vec<CallArg>,
        depth: usize,
        object_state: ObjectState,
        caller_slots: Option<SlotMap>,
    ) -> Result<TrackedValue, RuntimeError> {
        self.invoke_raw(name, args, depth, object_state, caller_slots)?
            .into_tracked()
    }

    fn invoke_engine_value(
        &self,
        name: &str,
        args: Vec<CallArg>,
        depth: usize,
        object_state: ObjectState,
        caller_slots: Option<SlotMap>,
    ) -> Result<Value, RuntimeError> {
        self.invoke_engine_tracked_value(name, args, depth, object_state, caller_slots)
            .map(|tracked| tracked.value)
    }

    fn invoke_engine_tracked_value(
        &self,
        name: &str,
        args: Vec<CallArg>,
        depth: usize,
        object_state: ObjectState,
        caller_slots: Option<SlotMap>,
    ) -> Result<TrackedValue, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        maybe_grow(|| {
            if let Some(function) = self.engine_script_function(name) {
                return self
                    .invoke_script_function(name, function, args, depth, object_state)?
                    .into_tracked();
            }

            if let Some(function) = self.host_functions.get(name) {
                let values = self.call_args_to_values(&args)?;
                let _guard = CallerSlotsGuard::enter(caller_slots);
                return self
                    .invoke_host_function(name, function, &values)
                    .map(TrackedValue::runtime);
            }

            if let Some(function) = self.host_reference_function(name) {
                let _guard = CallerSlotsGuard::enter(caller_slots);
                return self
                    .invoke_host_reference_function(name, function, &args)
                    .map(TrackedValue::runtime);
            }

            Err(RuntimeError::new(format!("unknown function '{name}'")))
        })
    }

    fn engine_script_function(&self, name: &str) -> Option<&Function> {
        self.global_functions
            .map_or_else(|| self.functions.get(name), |functions| functions.get(name))
    }

    fn invoke_reference(
        &self,
        name: &str,
        args: Vec<CallArg>,
        depth: usize,
        object_state: ObjectState,
        caller_slots: Option<SlotMap>,
    ) -> Result<LValueRef, RuntimeError> {
        match self.invoke_raw(name, args, depth, object_state, caller_slots)? {
            ReturnValue::Reference(reference) => Ok(reference),
            ReturnValue::Value(_) => Err(RuntimeError::new(format!(
                "function '{name}' does not return a reference"
            ))),
        }
    }

    fn invoke_raw(
        &self,
        name: &str,
        args: Vec<CallArg>,
        depth: usize,
        object_state: ObjectState,
        caller_slots: Option<SlotMap>,
    ) -> Result<ReturnValue, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        maybe_grow(|| {
            if let Some(function) = self.functions.get(name) {
                return self.invoke_script_function(name, function, args, depth, object_state);
            }

            // Engine-global script functions (System.c4g `global func`s,
            // owned by Game.ScriptEngine in C++): the fallback after the
            // own script, before C++ engine functions — the
            // FindSameNameFunc own-def-then-engine order (C4Aul.cpp:130-148).
            if let Some(function) = self
                .global_functions
                .and_then(|functions| functions.get(name))
            {
                return self.invoke_script_function(name, function, args, depth, object_state);
            }

            if let Some(function) = self.host_functions.get(name) {
                let values = self.call_args_to_values(&args)?;
                // Host functions run under the CALLER's var-slot table
                // (cthr->Caller->NumVars) for the FindConstructionSite
                // write-back seam (C4Script.cpp:1966-1978).
                let _guard = CallerSlotsGuard::enter(caller_slots);
                return self
                    .invoke_host_function(name, function, &values)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value);
            }

            if let Some(function) = self.host_reference_function(name) {
                let _guard = CallerSlotsGuard::enter(caller_slots);
                return self
                    .invoke_host_reference_function(name, function, &args)
                    .map(TrackedValue::runtime)
                    .map(ReturnValue::Value);
            }

            Err(RuntimeError::new(format!("unknown function '{name}'")))
        })
    }

    fn invoke_script_function(
        &self,
        name: &str,
        function: &Function,
        args: Vec<CallArg>,
        depth: usize,
        object_state: ObjectState,
    ) -> Result<ReturnValue, RuntimeError> {
        // Allow calling with MORE arguments than declared (extras ignored)
        // This matches C++ OpenClonk behavior for action callbacks
        // C++ pads missing arguments with nil: every call carries a full
        // C4AulParSet (10 slots, unfilled = nil — C4Aul.h:104-121,
        // C4AulExec.cpp:333-336), so callees legally declare more params
        // than the caller passes.
        let mut args = args;
        while args.len() < function.params.len() {
            args.push(CallArg::runtime(Value::Nil));
        }

        let debug_args = self.call_args_to_values(&args)?;
        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_call() {
                callback(name, &debug_args);
            }
        }

        let mut env = Environment::new_with_params(
            &function.params,
            &args,
            function.strict_level,
            object_state,
        )?;
        env.inherited_target = function.overloaded.clone();
        env.function_name = function.name.clone();
        env.engine_scope = function.access == AccessLevel::Global;

        // Parameters resolve before object locals in C++
        // (C4AulParse.cpp:2709-2729). `define_object_local` preserves an
        // existing binding so MART::Mode0(pObj, ...) receives its argument,
        // rather than the definition's same-name object-local `pObj`.
        for var_decl in self.var_decls {
            let cell = env.object_state.named_local_cell(&var_decl.name);
            env.define_object_local(&var_decl.name, self.identity_for_cell(&cell));
        }

        // C4Aul `var` declarations are FUNCTION-scoped and hoisted: the
        // parser builds the whole Fn->VarNamed table up front, so a var
        // read BEFORE its `var` statement is nil, never an error
        // (Dynamite.c4d reads iX three lines above `var iX`).
        hoist_function_vars(&function.body, &mut env);

        let result =
            self.execute_statements(&function.body, &mut env, depth, function.returns_reference)?;
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

        let debug_return = value.as_value()?;
        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &debug_return);
            }
        }

        Ok(value)
    }

    fn call_args_to_values(&self, args: &[CallArg]) -> Result<Vec<Value>, RuntimeError> {
        args.iter().map(CallArg::read).collect()
    }

    fn host_reference_function(&self, name: &str) -> Option<&HostReferenceFunction> {
        self.host_reference_functions
            .and_then(|functions| functions.get(name))
    }

    fn has_host_function(&self, name: &str) -> bool {
        self.host_functions.contains_key(name) || self.host_reference_function(name).is_some()
    }

    fn invoke_host_function(
        &self,
        name: &str,
        function: &HostFunction,
        args: &[Value],
    ) -> Result<Value, RuntimeError> {
        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_call() {
                callback(name, args);
            }
        }

        let outcome = function(args);
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
        args: &[CallArg],
    ) -> Result<Value, RuntimeError> {
        let args = args
            .iter()
            .cloned()
            .map(HostCallArg)
            .collect::<Vec<_>>();
        let debug_args = args
            .iter()
            .map(HostCallArg::read)
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_call() {
                callback(name, &debug_args);
            }
        }

        let result = function.call(&args)?;

        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &result);
            }
        }
        Ok(result)
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
                // `var` inside a block must not shadow it.
                if env.assign_tracked(name, tracked.clone()).is_err() {
                    env.define_tracked(name, tracked);
                }
                Ok(ControlFlow::Normal)
            }
            Stmt::Assignment { target, value } => {
                self.evaluate_assignment(target, value, env, depth)?;
                Ok(ControlFlow::Normal)
            }
            Stmt::Return(expr) => {
                let value = if returns_reference {
                    match expr {
                        Some(expr) => {
                            ReturnValue::Reference(self.expr_to_lvalue(expr, env, depth)?)
                        }
                        None => {
                            return Err(RuntimeError::new(
                                "reference-returning function must return an lvalue",
                            ))
                        }
                    }
                } else {
                    ReturnValue::Value(match expr {
                        Some(expr) => self.evaluate_tracked(expr, env, depth)?,
                        None => TrackedValue::runtime(Value::Nil),
                    })
                };
                Ok(ControlFlow::Return(value))
            }
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
                                env.define_tracked(name, tracked);
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
                        Value::Array(values) => values
                            .iter()
                            .cloned()
                            .map(|value| (value, None))
                            .collect(),
                        // Preserve the existing array-foreach behavior here;
                        // its non-array diagnostics are tracked separately.
                        _ => Vec::new(),
                    }
                };

                for (key_or_item, map_value) in items {
                    // Both header spellings use the function-scoped named-var
                    // slots populated by the pre-parser/hoisting pass.
                    env.assign(variable, key_or_item)?;
                    if let (Some(value_variable), Some(map_value)) =
                        (value_variable, map_value)
                    {
                        env.assign(value_variable, map_value)?;
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

    fn global_variable(&self, name: &str) -> Option<Value> {
        self.globals_named
            .and_then(|table| table.borrow().get(name).map(|cell| cell.borrow().clone()))
    }

    fn global_variable_cell(&self, name: &str) -> Option<ValueCell> {
        self.globals_named
            .and_then(|table| table.borrow().get(name).cloned())
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

    fn evaluate(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Literal(literal) => Ok(self.literal_value(literal)),
            // `this` yields the object context the call runs on (host-provided),
            // mirroring C4Script's `this` (C4V_C4Object); Nil for global calls.
            Expr::This => Ok(self.this_value.clone()),
            Expr::Variable(name) => match env.get(name)? {
                Some(value) => Ok(value),
                // Engine-global statics (GlobalNamed) resolve next; script
                // constants last ("global constants have lowest priority",
                // C4AulParse.cpp:2836-2839).
                None => self
                    .global_variable(name)
                    .or_else(|| self.constants.and_then(|constants| constants.get(name).cloned()))
                    .or_else(|| self.global_constant(name))
                    .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'"))),
            },
            Expr::Unary(op, expr) => {
                let value = self.evaluate(expr, env, depth)?;
                self.eval_unary(op, value)
            }
            Expr::Binary(lhs, op, rhs) => {
                if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
                    let left = self.evaluate_tracked(lhs, env, depth)?;
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
                    let right = self.evaluate(rhs, env, depth)?;
                    return Ok(Value::Bool(left.as_bool() || right.as_bool()));
                }
                // `??` coalesces on NIL only (0 and false are kept), with
                // the right side skipped for non-nil left operands
                // (AB_JUMPNOTNIL, C4AulParse.cpp:1050-1056).
                if matches!(op, BinaryOp::NilCoalescing) {
                    if !matches!(left, Value::Nil) {
                        return Ok(left);
                    }
                    return self.evaluate(rhs, env, depth);
                }
                let right = self.evaluate(rhs, env, depth)?;
                self.eval_binary(left, op, right, env.strict_level, None)
            }
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
                    // lvalue path (lc-engine registers neither as a host function).
                    if let Expr::Variable(name) = callee.as_ref() {
                        if (name == "Var" || name == "Local")
                            && (args.is_empty() || args.len() == 1)
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            let index = Box::new(
                                args.first()
                                    .cloned()
                                    .unwrap_or(Expr::Literal(Literal::Int(0))),
                            );
                            let target = if name == "Var" {
                                AssignmentTarget::VarSlot(index)
                            } else {
                                AssignmentTarget::LocalSlot(index)
                            };
                            return self.get_target_value(env, &target);
                        }
                        // `Local(n, pObj)` reads ANOTHER object's numbered
                        // slot through the returned reference (FnLocal,
                        // C4Script.cpp:3423-3433); a negative index is nil.
                        if name == "Local"
                            && args.len() == 2
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            let index =
                                self.evaluate_slot_index("Local()", &args[0], env, depth)?;
                            if index < 0 {
                                return Ok(Value::Nil);
                            }
                            let target = self.evaluate(&args[1], env, depth + 1)?;
                            let cell = self.numbered_local_cell(env, index, Some(target));
                            let value = cell.borrow().clone();
                            return Ok(value);
                        }
                        // FnSetLocal (C4Script.cpp:3408-3414): writes the
                        // numbered Local slot, returns the value; a nil or
                        // absent object defaults to the executing object.
                        if name == "SetLocal"
                            && (1..=3).contains(&args.len())
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            return self
                                .set_local_tracked(args, None, env, depth + 1)
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
                            let local_name = match self.evaluate(&args[0], env, depth + 1)? {
                                Value::String(local_name) => local_name,
                                other => {
                                    return Err(RuntimeError::new(format!(
                                        "LocalN: expected string for name, got {}",
                                        other.type_name()
                                    )))
                                }
                            };
                            let target = args
                                .get(1)
                                .map(|arg| self.evaluate(arg, env, depth + 1))
                                .transpose()?;
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
                            let code = match args
                                .first()
                                .map(|arg| self.evaluate(arg, env, depth + 1))
                                .transpose()?
                            {
                                Some(Value::String(code)) => code,
                                // A null string cannot parse; DirectExec's
                                // catch yields C4VNull (C4AulExec.cpp:
                                // 1693-1699).
                                _ => return Ok(Value::Nil),
                            };
                            let Ok(expr) =
                                crate::parser::Parser::with_strict_level(&code, env.strict_level)
                                    .parse_direct_exec_expression()
                            else {
                                // Parse errors log and yield C4VNull
                                // (DirectExec's catch, C4AulExec.cpp:1693).
                                return Ok(Value::Nil);
                            };
                            let mut exec_env = Environment::new_with_params(
                                &[],
                                &[],
                                env.strict_level,
                                env.object_state.clone(),
                            )?;
                            for var_decl in self.var_decls {
                                let cell = exec_env.object_state.named_local_cell(&var_decl.name);
                                exec_env.define_object_local(
                                    &var_decl.name,
                                    self.identity_for_cell(&cell),
                                );
                            }
                            // Runtime errors propagate (fPassErrors=true,
                            // C4Script.cpp:4514).
                            return self.evaluate(&expr, &mut exec_env, depth + 1);
                        }
                        // `Par(n)` reads the executing call's parameter slot n;
                        // outside 0..ParCnt it is nil (C4AulExec.cpp:1127-1140).
                        if name == "Par"
                            && args.len() <= 1
                            && !self.functions.contains_key(name)
                            && !self.has_host_function(name)
                        {
                            let index = args
                                .first()
                                .map(|arg| self.evaluate(arg, env, depth + 1))
                                .transpose()?
                                .map(|value| match value {
                                    Value::Int(index) => Ok(index),
                                    Value::Nil => Ok(0),
                                    Value::Bool(flag) => Ok(i32::from(flag)),
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
                            // none (C4AulParse.cpp:2775-2798).
                            let Some(target) = env.inherited_target.clone() else {
                                let inherited_name = env.function_name.clone();
                                // Script functions overload same-name ENGINE
                                // functions: inherited() chains to the host
                                // fn (C4Aul OwnerOverloaded includes engine
                                // funcs — GoldRush AI.c4d's global
                                // GetOwner/Hostile overrides rely on it).
                                if let Some(host) =
                                    self.host_functions.get(&inherited_name)
                                {
                                    let mut evaluated_args =
                                        self.build_call_args(
                                            Some(&inherited_name),
                                            None,
                                            args,
                                            env,
                                            depth + 1,
                                        )?;
                                    if *forward_rest {
                                        Self::append_forwarded_args(&mut evaluated_args, env)?;
                                    }
                                    let values = self.call_args_to_values(&evaluated_args)?;
                                    // The overriding script function is the
                                    // host fn's cthr->Caller.
                                    let _guard =
                                        CallerSlotsGuard::enter(Some(env.var_slots.clone()));
                                    return self
                                        .invoke_host_function(
                                            &env.function_name.clone(),
                                            host,
                                            &values,
                                        );
                                }
                                if let Some(host) =
                                    self.host_reference_function(&inherited_name)
                                {
                                    let mut evaluated_args = self.build_call_args(
                                        Some(&inherited_name),
                                        None,
                                        args,
                                        env,
                                        depth + 1,
                                    )?;
                                    if *forward_rest {
                                        Self::append_forwarded_args(&mut evaluated_args, env)?;
                                    }
                                    let _guard =
                                        CallerSlotsGuard::enter(Some(env.var_slots.clone()));
                                    return self.invoke_host_reference_function(
                                        &inherited_name,
                                        host,
                                        &evaluated_args,
                                    );
                                }
                                return if name == "_inherited" {
                                    Ok(Value::Nil)
                                } else {
                                    Err(RuntimeError::new(format!(
                                        "inherited: no overloaded function (in {})",
                                        env.function_name
                                    )))
                                };
                            };
                            let mut evaluated_args =
                                self.build_call_args(
                                    Some(&target.name),
                                    Some(&target),
                                    args,
                                    env,
                                    depth + 1,
                                )?;
                            if *forward_rest {
                                Self::append_forwarded_args(&mut evaluated_args, env)?;
                            }
                            self.invoke_script_function(
                                &target.name.clone(),
                                &target,
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
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
                                if let Some(value) = self
                                    .constants
                                    .and_then(|constants| constants.get(name).cloned())
                                    .or_else(|| {
                                        self.globals_consts.and_then(|table| {
                                            table
                                                .borrow()
                                                .get(name)
                                                .map(|cell| cell.borrow().clone())
                                        })
                                    })
                                {
                                    // C++ requires an immediate ')' after
                                    // the '(' (Match(ATT_BCLOSE),
                                    // C4AulParse.cpp:2860).
                                    if !args.is_empty() {
                                        return Err(RuntimeError::new(
                                            "parameters not allowed in functional usage of constants",
                                        ));
                                    }
                                    return Ok(value);
                                }
                            }
                            let function = if env.engine_scope {
                                self.engine_script_function(name)
                            } else {
                                self.functions.get(name).or_else(|| {
                                    self.global_functions
                                        .and_then(|functions| functions.get(name))
                                })
                            };
                            let mut evaluated_args =
                                self.build_call_args(Some(name), function, args, env, depth + 1)?;
                            if *forward_rest {
                                Self::append_forwarded_args(&mut evaluated_args, env)?;
                            }
                            if env.engine_scope {
                                self.invoke_engine_value(
                                    name,
                                    evaluated_args,
                                    depth + 1,
                                    env.object_state.clone(),
                                    Some(env.var_slots.clone()),
                                )
                            } else {
                                self.invoke_value(
                                    name,
                                    evaluated_args,
                                    depth + 1,
                                    env.object_state.clone(),
                                    Some(env.var_slots.clone()),
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
                for element in elements {
                    values.push(self.evaluate(element, env, depth)?);
                }
                Ok(Value::Array(values))
            }
            Expr::Proplist(entries) => {
                let mut map = ValueMap::with_capacity(entries.len());
                for (key_expr, value_expr) in entries {
                    let key = self.evaluate(key_expr, env, depth)?;
                    let value = self.evaluate(value_expr, env, depth)?;
                    if matches!(&value, Value::Nil)
                        && map
                            .get_key(&key)
                            .is_some_and(|value| !matches!(value, Value::Nil))
                    {
                        map.shift_remove_key(&key);
                    } else {
                        map.insert_key(key, value);
                    }
                }
                Ok(Value::Proplist(map))
            }
            Expr::Index(target, index) => {
                // Array values are reference-counted containers in C++. Keep
                // an addressable Rust path live so the otherwise surprising
                // empty `array[-1]` growth remains visible to the caller.
                let collection_reference = self.existing_path_lvalue(target, env, depth)?;
                let collection = if let Some(reference) = &collection_reference {
                    reference.read()?
                } else {
                    self.evaluate(target, env, depth)?
                };
                let idx = self.evaluate(index, env, depth)?;
                Self::grow_empty_negative_array(
                    collection_reference.as_ref(),
                    &collection,
                    &idx,
                )?;
                self.eval_index(collection, idx)
            }
            Expr::ArrayAppend(_) => self.expr_to_lvalue(expr, env, depth)?.read(),
            Expr::ArrayAppendAssignment {
                target,
                operation,
                operator,
                value,
            } => self
                .evaluate_array_append_assignment_tracked(
                    target, operation, operator, value, env, depth,
                )
                .map(|tracked| tracked.value),
            Expr::Property(target, name) => {
                let proplist = self.evaluate(target, env, depth)?;
                self.eval_property(proplist, name)
            }
            Expr::SafeNavigation { receiver, steps } => self
                .evaluate_safe_navigation_tracked(receiver, steps, env, depth)
                .map(|tracked| tracked.value),
            Expr::Assignment(target, value_expr) => {
                self.evaluate_assignment(target, value_expr, env, depth)
            }
            Expr::Comma(exprs) => {
                // Comma operator: evaluate all expressions left-to-right, return the last value
                let mut result = Value::Nil;
                for expr in exprs {
                    result = self.evaluate(expr, env, depth)?;
                }
                Ok(result)
            }
            Expr::PreIncrement(expr) => {
                self.update_counter(expr, env, 1, false, "increment")
            }
            Expr::PreDecrement(expr) => {
                self.update_counter(expr, env, -1, false, "decrement")
            }
            Expr::PostIncrement(expr) => {
                self.update_counter(expr, env, 1, true, "increment")
            }
            Expr::PostDecrement(expr) => {
                self.update_counter(expr, env, -1, true, "decrement")
            }
        }
    }

    fn evaluate_tracked(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        match expr {
            Expr::Literal(literal) => {
                Ok(TrackedValue::literal(self.literal_value(literal), literal))
            }
            Expr::Variable(name) => match env.get_tracked(name)? {
                Some(tracked) => Ok(tracked),
                None => {
                    if let Some(cell) = self
                        .global_variable_cell(name)
                        .or_else(|| self.global_constant_cell(name))
                    {
                        Ok(self.read_tracked_named_cell(name, &cell))
                    } else if let Some(value) = self
                        .constants
                        .and_then(|constants| constants.get(name).cloned())
                    {
                        Ok(self.tracked_constant(name, value))
                    } else {
                        self.evaluate(expr, env, depth).map(TrackedValue::runtime)
                    }
                }
            },
            Expr::Array(elements) => {
                let mut tracked = Vec::with_capacity(elements.len());
                for element in elements {
                    tracked.push(self.evaluate_tracked(element, env, depth)?);
                }
                Ok(TrackedValue::array(tracked))
            }
            Expr::Proplist(entries) => {
                let mut tracked = Vec::with_capacity(entries.len());
                for (key_expr, value_expr) in entries {
                    let key = self.evaluate(key_expr, env, depth)?;
                    let value = self.evaluate_tracked(value_expr, env, depth)?;
                    tracked.push((key, value));
                }
                Ok(TrackedValue::proplist(tracked))
            }
            Expr::Index(target, index_expr) => {
                let collection_reference = self.existing_path_lvalue(target, env, depth)?;
                let mut collection = match &collection_reference {
                    Some(reference) => reference.read_tracked()?,
                    None => self.evaluate_tracked(target, env, depth)?,
                };
                let index = self.evaluate(index_expr, env, depth)?;
                Self::grow_empty_negative_array(
                    collection_reference.as_ref(),
                    &collection.value,
                    &index,
                )?;
                if let Some(reference) = &collection_reference {
                    collection = reference.read_tracked()?;
                }
                let segment = PathSegment::Index(index.clone());
                let string_result = matches!(&collection.value, Value::String(_));
                let inherited_identity = collection.identity_at(&segment);
                let value = self.eval_index(collection.value, index)?;
                let identity = if string_result {
                    RawIdentity::runtime(&value)
                } else {
                    inherited_identity
                };
                Ok(TrackedValue { value, identity })
            }
            Expr::ArrayAppend(_) => self.expr_to_lvalue(expr, env, depth)?.read_tracked(),
            Expr::ArrayAppendAssignment {
                target,
                operation,
                operator,
                value,
            } => self.evaluate_array_append_assignment_tracked(
                target, operation, operator, value, env, depth,
            ),
            Expr::Property(target, name) => {
                let collection = self.evaluate_tracked(target, env, depth)?;
                let identity = collection.identity_at(&PathSegment::Property(name.clone()));
                let value = self.eval_property(collection.value, name)?;
                Ok(TrackedValue { value, identity })
            }
            Expr::SafeNavigation { receiver, steps } => {
                self.evaluate_safe_navigation_tracked(receiver, steps, env, depth)
            }
            Expr::Binary(left, BinaryOp::Concat, right) => {
                let left = self.evaluate_tracked(left, env, depth)?;
                let right = self.evaluate_tracked(right, env, depth)?;
                match (&left.value, &right.value) {
                    (Value::Array(_), Value::Array(_)) => {
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
                        let value = self.eval_concat(left.value, right.value)?;
                        Ok(TrackedValue {
                            value,
                            identity: Some(RawIdentity::Heap(Rc::new(HeapIdentity::Array(
                                identities,
                            )))),
                        })
                    }
                    (Value::Proplist(_), Value::Proplist(_)) => {
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
                        identities.extend(right_identities);
                        let value = self.eval_concat(left.value, right.value)?;
                        Ok(TrackedValue {
                            value,
                            identity: Some(RawIdentity::Heap(Rc::new(HeapIdentity::Proplist(
                                identities,
                            )))),
                        })
                    }
                    _ => self
                        .eval_concat(left.value, right.value)
                        .map(TrackedValue::runtime),
                }
            }
            Expr::Binary(left, BinaryOp::NilCoalescing, right) => {
                let left = self.evaluate_tracked(left, env, depth)?;
                if !matches!(left.value, Value::Nil) {
                    Ok(left)
                } else {
                    self.evaluate_tracked(right, env, depth)
                }
            }
            Expr::Binary(left, BinaryOp::And, right) if env.strict_level.unwrap_or(0) >= 2 => {
                let left = self.evaluate_tracked(left, env, depth)?;
                if left.value.as_bool() {
                    self.evaluate_tracked(right, env, depth)
                } else {
                    Ok(left)
                }
            }
            Expr::Binary(left, BinaryOp::Or, right) if env.strict_level.unwrap_or(0) >= 2 => {
                let left = self.evaluate_tracked(left, env, depth)?;
                if left.value.as_bool() {
                    Ok(left)
                } else {
                    self.evaluate_tracked(right, env, depth)
                }
            }
            Expr::Call {
                callee,
                args,
                is_optional,
                forward_rest,
            } if !*is_optional => {
                if let Expr::Property(base, name) = callee.as_ref() {
                    let target = self.evaluate(base, env, depth + 1)?;
                    if matches!(target, Value::Object(_))
                        && name == "SetLocal"
                        && (1..=3).contains(&args.len())
                        && !self.functions.contains_key(name)
                        && !self.has_host_function(name)
                    {
                        return self.set_local_tracked(args, Some(target), env, depth + 1);
                    }
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
                    let function = if env.engine_scope {
                        self.engine_script_function(name)
                    } else {
                        self.functions.get(name).or_else(|| {
                            self.global_functions
                                .and_then(|functions| functions.get(name))
                        })
                    };
                    if name == "SetLocal"
                        && (1..=3).contains(&args.len())
                        && function.is_none()
                        && !self.has_host_function(name)
                    {
                        return self.set_local_tracked(args, None, env, depth + 1);
                    }
                    if env.strict_level.unwrap_or(0) < 2
                        && function.is_none()
                        && !self.has_host_function(name)
                        && args.is_empty()
                    {
                        if let Some(cell) = self.global_constant_cell(name) {
                            return Ok(self.read_tracked_named_cell(name, &cell));
                        }
                        if let Some(value) = self
                            .constants
                            .and_then(|constants| constants.get(name).cloned())
                        {
                            return Ok(self.tracked_constant(name, value));
                        }
                    }
                    let builtin_reference = matches!(name.as_str(), "Var" | "Local")
                        && args.len() <= 1
                        || name == "LocalN" && (1..=2).contains(&args.len())
                        || name == "Global";
                    if builtin_reference && function.is_none() && !self.has_host_function(name) {
                        return self.expr_to_lvalue(expr, env, depth)?.read_tracked();
                    }
                    if !matches!(name.as_str(), "inherited" | "_inherited")
                        && (function.is_some() || self.has_host_function(name))
                    {
                        let mut evaluated_args =
                            self.build_call_args(Some(name), function, args, env, depth + 1)?;
                        if *forward_rest {
                            Self::append_forwarded_args(&mut evaluated_args, env)?;
                        }
                        return if env.engine_scope {
                            self.invoke_engine_tracked_value(
                                name,
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.var_slots.clone()),
                            )
                        } else {
                            self.invoke_tracked_value(
                                name,
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.var_slots.clone()),
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
                let tracked = self.evaluate_tracked(value_expr, env, depth)?;
                self.assign_target_tracked(env, target, tracked.clone())?;
                Ok(tracked)
            }
            Expr::Comma(exprs) => {
                let mut result = TrackedValue::runtime(Value::Nil);
                for expr in exprs {
                    result = self.evaluate_tracked(expr, env, depth)?;
                }
                Ok(result)
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
        let mut current = self.evaluate_tracked(receiver, env, depth)?;

        for step in steps {
            if step.nil_guard && matches!(current.value, Value::Nil) {
                return Ok(TrackedValue::runtime(Value::Nil));
            }

            current = match &step.operation {
                NavigationOperation::Index(index_expr) => {
                    let index = self.evaluate(index_expr, env, depth)?;
                    let segment = PathSegment::Index(index.clone());
                    let string_result = matches!(&current.value, Value::String(_));
                    let inherited_identity = current.identity_at(&segment);
                    let value = self.eval_index(current.value, index)?;
                    let identity = if string_result {
                        RawIdentity::runtime(&value)
                    } else {
                        inherited_identity
                    };
                    TrackedValue { value, identity }
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
                    let identity = current.identity_at(&PathSegment::Property(name.clone()));
                    let value = self.eval_property(current.value, name)?;
                    TrackedValue { value, identity }
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

    fn evaluate_array_append_assignment_tracked(
        &self,
        target: &AssignmentTarget,
        operation: &Option<BinaryOp>,
        operator: &str,
        value: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<TrackedValue, RuntimeError> {
        // AB_ARRAY_APPEND creates and retains one reference before the RHS
        // runs. Generic compound desugaring cannot model that side effect: it
        // would evaluate `array[]` once for the old value and again for the
        // write, appending two slots.
        let reference = self.assignment_target_to_lvalue(env, target, depth)?;
        let left = reference.read_tracked()?;
        let result = if operation.is_none() {
            self.evaluate_tracked(value, env, depth)?
        } else if matches!(operation, Some(BinaryOp::NilCoalescing))
            && !matches!(left.value, Value::Nil)
        {
            left
        } else {
            let right = self.evaluate_tracked(value, env, depth)?;
            if matches!(operation, Some(BinaryOp::NilCoalescing)) {
                right
            } else {
                TrackedValue::runtime(self.eval_binary(
                    left.value,
                    operation.as_ref().expect("compound operation exists"),
                    right.value,
                    env.strict_level,
                    Some(operator),
                )?)
            }
        };
        reference.write_tracked(result.clone())?;
        Ok(result)
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
        let target = Self::expr_to_assignment_target(expr)?;

        // EffectVar is exposed by the engine as a value-style read/write host
        // pair rather than a retained ValueReference. Evaluate its addressing
        // arguments once and reuse those values for both halves.
        if let AssignmentTarget::EffectSlot(args) = &target {
            let arg_values = args
                .iter()
                .map(|arg| self.evaluate(arg, env, 0))
                .collect::<Result<Vec<_>, _>>()?;
            let old_value = if let Some(host) = self.host_functions.get("EffectVar") {
                let _guard = CallerSlotsGuard::enter(Some(env.var_slots.clone()));
                self.invoke_host_function("EffectVar", host, &arg_values)?
            } else {
                env.get(&format!(
                    "__effect_{}",
                    arg_values
                        .iter()
                        .map(|value| match value {
                            Value::Int(value) => value.to_string(),
                            Value::String(value) => value.clone(),
                            other => format!("{other:?}"),
                        })
                        .collect::<Vec<_>>()
                        .join("_")
                ))?
                .unwrap_or(Value::Nil)
            };
            let old_value = Self::counter_operand(old_value, operation)?;
            let new_value = old_value.wrapping_add(delta);
            if let Some(host) = self.host_functions.get("EffectVar") {
                let mut write_args = arg_values;
                write_args.push(Value::Int(new_value));
                let _guard = CallerSlotsGuard::enter(Some(env.var_slots.clone()));
                self.invoke_host_function("EffectVar", host, &write_args)?;
            } else {
                let slot_name = format!(
                    "__effect_{}",
                    arg_values
                        .iter()
                        .map(|value| match value {
                            Value::Int(value) => value.to_string(),
                            Value::String(value) => value.clone(),
                            other => format!("{other:?}"),
                        })
                        .collect::<Vec<_>>()
                        .join("_")
                );
                env.define(&slot_name, Value::Int(new_value));
            }
            return Ok(Value::Int(if return_old {
                old_value
            } else {
                new_value
            }));
        }

        let reference = self.assignment_target_to_lvalue(env, &target, 0)?;
        let old_value = Self::counter_operand(reference.read()?, operation)?;
        let new_value = old_value.wrapping_add(delta);
        reference.write(Value::Int(new_value))?;
        Ok(Value::Int(if return_old {
            old_value
        } else {
            new_value
        }))
    }

    /// `++`/`--` operand conversion: CheckOpPar<C4V_Int> converts nil to 0 and
    /// bool to int before the operation (C4AulExec.cpp:450-458,
    /// C4Value.cpp:453-466 FnCnvGuess); other types stay errors.
    fn counter_operand(value: Value, operation: &str) -> Result<i32, RuntimeError> {
        match value {
            Value::Int(value) => Ok(value),
            Value::Nil => Ok(0),
            Value::Bool(flag) => Ok(i32::from(flag)),
            other => Err(RuntimeError::new(format!(
                "cannot {operation} non-integer value: {other:?}"
            ))),
        }
    }

    fn literal_value(&self, literal: &Literal) -> Value {
        match literal {
            Literal::Int(i) => Value::Int(*i),
            Literal::Bool(b) => Value::Bool(*b),
            Literal::String(s) => Value::String(s.clone()),
            Literal::C4Id(id) => Value::C4Id(id.clone()),
            Literal::Nil => Value::Nil,
        }
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
            Concat => self.eval_concat(left, right),
            // Reached only via non-short-circuit paths (the Binary arm in
            // `evaluate` handles `??` before both sides run); keep the same
            // nil-only semantics.
            NilCoalescing => Ok(if matches!(left, Value::Nil) {
                right
            } else {
                left
            }),
            Sub => self.eval_int_op(left, right, |a, b| a - b, "-"),
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
            Pow => {
                let rhs = match right {
                    Value::Int(i) => i,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot apply '**' to operands of type int and {}",
                            other.type_name()
                        )))
                    }
                };
                if rhs < 0 {
                    return Err(RuntimeError::new("negative exponent not supported"));
                }
                match left {
                    Value::Int(lhs) => Ok(Value::Int(lhs.pow(rhs as u32))),
                    other => Err(RuntimeError::new(format!(
                        "cannot apply '**' to operands of type {} and int",
                        other.type_name()
                    ))),
                }
            }
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
            LeftShift => self.eval_int_op(left, right, |a, b| a << b, "<<"),
            RightShift => self.eval_int_op(left, right, |a, b| a >> b, ">>"),
            // String comparison operators
            StringEqual => self.eval_string_cmp(left, right, strict, |a, b| a == b, "S="),
            StringNotEqual => self.eval_display_string_cmp(left, right, |a, b| a != b),
            KeywordStringEqual => {
                self.eval_string_cmp(left, right, strict, |a, b| a == b, "eq")
            }
            KeywordStringNotEqual => {
                self.eval_string_cmp(left, right, strict, |a, b| a != b, "ne")
            }
            // Rust currently accepts these non-C++ operators. Preserve their
            // existing behavior until the grammar-removal parity issue lands.
            StringLess => self.eval_display_string_cmp(left, right, |a, b| a < b),
            StringLessEqual => self.eval_display_string_cmp(left, right, |a, b| a <= b),
            StringGreater => self.eval_display_string_cmp(left, right, |a, b| a > b),
            StringGreaterEqual => self.eval_display_string_cmp(left, right, |a, b| a >= b),
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
    fn eval_concat(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match left {
            Value::Array(mut a) => match right {
                Value::Array(b) => {
                    a.extend(b);
                    Ok(Value::Array(a))
                }
                other => Err(RuntimeError::new(format!(
                    "operator '..' right side: cannot concatenate array with {}",
                    other.type_name()
                ))),
            },
            Value::Proplist(mut a) => match right {
                Value::Proplist(b) => {
                    a.extend(b);
                    Ok(Value::Proplist(a))
                }
                other => Err(RuntimeError::new(format!(
                    "operator '..' right side: cannot concatenate proplist with {}",
                    other.type_name()
                ))),
            },
            left => {
                let mut s = concat_string(&left);
                s.push_str(&concat_string(&right));
                Ok(Value::String(s))
            }
        }
    }

    fn eval_add(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (left, right) {
            // String concatenation stands in for C4Script's `..` operator (the
            // lexer does not yet tokenize `..`); keep it when either side is a
            // string. C++'s own `+` (AB_Sum) is integer-only. String+String uses
            // the raw inner text (to_string() would quote it).
            (Value::String(mut a), Value::String(b)) => {
                a.push_str(&b);
                Ok(Value::String(a))
            }
            (Value::String(mut a), other) => {
                a.push_str(&other.to_string());
                Ok(Value::String(a))
            }
            (other, Value::String(b)) => {
                let mut result = other.to_string();
                result.push_str(&b);
                Ok(Value::String(result))
            }
            // C++ AB_Sum (C4AulExec.cpp:538-545): integer add with `_getInt()`
            // coercion (nil->0, bool->0/1). wrapping_add matches C++ 2's-complement
            // overflow instead of panicking in debug builds.
            (a, b) => match (a.as_c4_int(), b.as_c4_int()) {
                (Some(x), Some(y)) => Ok(Value::Int(x.wrapping_add(y))),
                _ => Err(RuntimeError::new(format!(
                    "cannot apply '+' to operands of type {} and {}",
                    a.type_name(),
                    b.type_name()
                ))),
            },
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
        F: Fn(&str, &str) -> bool,
    {
        // CheckOpPars for S=/eq/ne converts the left operand first, then the
        // right (C4AulExec.cpp:289-299,691-707). Below #strict 3, raw-falsy
        // concrete values are Set0() before conversion and therefore compare
        // as the empty string. Nil itself converts to String without changing
        // its Any type, so `_getStr()==nullptr` also reads as "" at strict 3.
        let convert = |value: Value, side: &str| {
            let canonical_nil = match &value {
                Value::Nil | Value::Object(0) => true,
                Value::C4Id(id) => {
                    id.len() < 4 || id == "NONE" || id.bytes().all(|byte| byte == b'0')
                }
                _ => false,
            };
            let typed_falsy = matches!(&value, Value::Int(0) | Value::Bool(false));
            if canonical_nil || (strict.unwrap_or(0) < 3 && typed_falsy) {
                return Ok(String::new());
            }
            match value {
                Value::String(text) => Ok(text),
                Value::Nil => Ok(String::new()),
                other => Err(RuntimeError::new(format!(
                    "operator \"{symbol}\" {side} side: got \"{}\", but expected \"string\"!",
                    other.type_name()
                ))),
            }
        };
        let left_str = convert(left, "left")?;
        let right_str = convert(right, "right")?;
        Ok(Value::Bool(cmp(&left_str, &right_str)))
    }

    fn eval_display_string_cmp<F>(
        &self,
        left: Value,
        right: Value,
        cmp: F,
    ) -> Result<Value, RuntimeError>
    where
        F: Fn(&str, &str) -> bool,
    {
        Ok(Value::Bool(cmp(&left.to_string(), &right.to_string())))
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
        let level = strict.unwrap_or(0);
        if level < 2 {
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
        }
        if level < 3 {
            if let (Some(a), Some(b)) = (left.as_c4_int(), right.as_c4_int()) {
                return a == b;
            }
        }
        self.values_equal_typed(left, right)
    }

    /// Type-checked equality (`#strict 3`, C4Value.cpp:835-849): different types
    /// are never equal, same type compares by content.
    fn values_equal_typed(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::C4Id(a), Value::C4Id(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Proplist(a), Value::Proplist(b)) => a == b,
            (Value::Nil, Value::Nil) => true,
            _ => false,
        }
    }

    fn eval_index(&self, collection: Value, index: Value) -> Result<Value, RuntimeError> {
        match (&collection, index) {
            (Value::Array(elements), index) => Ok(elements
                .get(array_index(&index)?)
                .cloned()
                .unwrap_or(Value::Nil)),
            (Value::String(text), index) => string_index(text, &index),
            (Value::Proplist(entries), key) => {
                Ok(entries.get_key(&key).cloned().unwrap_or(Value::Nil))
            }
            (other, _) => Err(RuntimeError::new(format!(
                "cannot index value of type {}",
                other.type_name()
            ))),
        }
    }

    fn eval_property(&self, value: Value, name: &str) -> Result<Value, RuntimeError> {
        match &value {
            Value::Proplist(entries) => Ok(entries.get(name).cloned().unwrap_or(Value::Nil)),
            other => Err(RuntimeError::new(format!(
                "cannot access property '{name}' on value of type {}",
                other.type_name()
            ))),
        }
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
        mut target: Value,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
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
        if matches!(target, Value::Object(_))
            && name == "LocalN"
            && args.len() == 1
            && !self.functions.contains_key(name)
        {
            let local_name = match self.evaluate(&args[0], env, depth + 1)? {
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
        if matches!(target, Value::Object(_))
            && name == "Local"
            && args.len() == 1
            && !self.functions.contains_key(name)
            && !self.has_host_function(name)
        {
            let index = self.evaluate_slot_index("Local()", &args[0], env, depth)?;
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
        if matches!(target, Value::Object(_))
            && name == "SetLocal"
            && (1..=3).contains(&args.len())
            && !self.functions.contains_key(name)
            && !self.has_host_function(name)
        {
            return self
                .set_local_tracked(args, Some(target), env, depth + 1)
                .map(|tracked| tracked.value);
        }
        match &target {
            Value::Nil | Value::Int(0) | Value::Bool(false) => Err(RuntimeError::new(
                "Object call: target is zero!".to_string(),
            )),
            Value::Object(_) | Value::C4Id(_) if self.method_dispatch.is_some() =>
            {
                let function = self.functions.get(name);
                let mut evaluated_args =
                    self.build_call_args(Some(name), function, args, env, depth + 1)?;
                if forward_rest {
                    Self::append_forwarded_args(&mut evaluated_args, env)?;
                }
                let mut dispatch_args = Vec::with_capacity(evaluated_args.len() + 3);
                dispatch_args.push(target.clone());
                dispatch_args.push(Value::String(name.to_string()));
                dispatch_args.push(Value::Bool(failsafe));
                for arg in &evaluated_args {
                    dispatch_args.push(arg.read()?);
                }
                let dispatch = self
                    .method_dispatch
                    .ok_or_else(|| RuntimeError::new("method dispatch vanished".to_string()))?;
                dispatch(&dispatch_args)
            }
            Value::Object(_) | Value::C4Id(_) => {
                // Self-target (or a bare engine without a world): resolve in
                // the executing context — FindSameNameFunc with
                // pDestDef == own def is the plain own->global->host chain.
                self.invoke_property_call_local(name, args, failsafe, forward_rest, env, depth)
            }
            other => {
                if self.method_dispatch.is_some() {
                    Err(RuntimeError::new(format!(
                        "Object call: Invalid target type {}, expected object or id!",
                        other.type_name()
                    )))
                } else {
                    // Bare scripting engines have no object world: keep the
                    // legacy resolve-by-name behavior for their tests.
                    self.invoke_property_call_local(name, args, failsafe, forward_rest, env, depth)
                }
            }
        }
    }

    fn invoke_property_call_local(
        &self,
        name: &str,
        args: &[Expr],
        failsafe: bool,
        forward_rest: bool,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        let function = self.functions.get(name);
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
            return Ok(Value::Nil);
        }
        let mut evaluated_args =
            self.build_call_args(Some(name), function, args, env, depth + 1)?;
        if forward_rest {
            Self::append_forwarded_args(&mut evaluated_args, env)?;
        }
        self.invoke_value(
            name,
            evaluated_args,
            depth + 1,
            env.object_state.clone(),
            Some(env.var_slots.clone()),
        )
    }

    fn build_call_args(
        &self,
        name: Option<&str>,
        function: Option<&Function>,
        args: &[Expr],
        env: &mut Environment,
        depth: usize,
    ) -> Result<Vec<CallArg>, RuntimeError> {
        let mut evaluated_args = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let script_wants_reference = function
                .and_then(|function| function.params.get(index))
                .is_some_and(|param| param.is_reference);
            let host_wants_reference = function.is_none()
                && name
                    .and_then(|name| self.host_reference_function(name))
                    .is_some_and(|function| function.wants_reference(index));
            let can_be_reference = if host_wants_reference {
                self.expr_can_be_host_reference(arg)
            } else {
                Self::expr_can_be_lvalue(arg)
            };
            if (script_wants_reference || host_wants_reference) && can_be_reference {
                evaluated_args.push(CallArg::Reference(self.expr_to_lvalue(arg, env, depth)?));
            } else {
                evaluated_args.push(CallArg::Value(self.evaluate_tracked(arg, env, depth)?));
            }
        }
        Ok(evaluated_args)
    }

    /// `Callee(args, ...)`: after the explicit arguments, forward every
    /// parameter slot of the executing function past its named parameters,
    /// stopping at the 10-slot frame limit (C4AulParse.cpp:2293-2306).
    fn append_forwarded_args(
        evaluated_args: &mut Vec<CallArg>,
        env: &Environment,
    ) -> Result<(), RuntimeError> {
        let mut index = env.named_param_count;
        while evaluated_args.len() < MAX_CALL_PARAMETERS && index < MAX_CALL_PARAMETERS {
            let tracked = env
                .call_args
                .get(index)
                .map(Binding::read_tracked)
                .transpose()?
                .unwrap_or_else(|| TrackedValue::runtime(Value::Nil));
            evaluated_args.push(CallArg::Value(tracked));
            index += 1;
        }
        // C++ callees always see 10 slots and cannot tell a missing argument
        // from an explicit nil, so dropping the nil tail is observationally
        // identical — and keeps host functions that count arguments honest.
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

    fn expr_can_be_lvalue(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Variable(_)
                | Expr::Property(_, _)
                | Expr::Index(_, _)
                | Expr::ArrayAppend(_)
                | Expr::Call { .. }
        )
    }

    /// Native reference parameters receive a null pointer for rvalue call
    /// results. Unlike script `&` parameters, we can decide that before
    /// evaluation for ordinary named functions and avoid executing a
    /// value-returning call once through `invoke_reference` and then again as
    /// a value (FnSimFlight's C4Value* parameters, C4Script.cpp:5309-5312).
    fn expr_can_be_host_reference(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Variable(_)
            | Expr::Property(_, _)
            | Expr::Index(_, _)
            | Expr::ArrayAppend(_) => true,
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Variable(name) => {
                    matches!(
                        name.as_str(),
                        "Local" | "LocalN" | "Var" | "EffectVar" | "Global" | "GlobalN"
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
            _ => false,
        }
    }

    fn assign_target(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match target {
            AssignmentTarget::EffectSlot(args) => {
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                // `EffectVar(i, obj, num) = v`: FnEffectVar returns a
                // REFERENCE into the effect's variables (C4Script.cpp) —
                // write through the host's set path (4th argument).
                if let Some(host) = self.host_functions.get("EffectVar") {
                    arg_values.push(value);
                    let _guard = CallerSlotsGuard::enter(Some(env.var_slots.clone()));
                    return self
                        .invoke_host_function("EffectVar", host, &arg_values)
                        .map(|_| ());
                }
                // Host-less fixture VMs keep the legacy env-slot shim.
                let slot_name = format!(
                    "__effect_{}",
                    arg_values
                        .iter()
                        .map(|v| match v {
                            Value::Int(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => format!("{:?}", v),
                        })
                        .collect::<Vec<_>>()
                        .join("_")
                );
                env.define(&slot_name, value);
                Ok(())
            }
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
            } => {
                if !matches!(method.as_str(), "LocalN" | "Local" | "Var" | "EffectVar") {
                    return self
                        .assignment_target_to_lvalue(env, target, 0)?
                        .write(value);
                }
                // Evaluate the object to get its identity
                let object_value = self.evaluate(object, env, 0)?;
                // `LocalN("name", obj) = v` / `obj->LocalN("name") = v`:
                // FnLocalN returns a reference into the TARGET's named
                // locals (C4Script.cpp:4591-4605) — write through the
                // resolved cell (self or host-supplied foreign cell).
                if method == "LocalN" && args.len() == 1 {
                    let local_name = match self.evaluate(&args[0], env, 0)? {
                        Value::String(local_name) => local_name,
                        other => {
                            return Err(RuntimeError::new(format!(
                                "LocalN: expected string for name, got {}",
                                other.type_name()
                            )))
                        }
                    };
                    let cell = self.localn_cell(env, &local_name, Some(object_value));
                    *cell.borrow_mut() = value;
                    return Ok(());
                }
                // `Local(n, obj) = v` / `obj->Local(n) = v`: FnLocal's
                // returned reference targets the FOREIGN numbered slot
                // (C4Script.cpp:3423-3433).
                if method == "Local" && args.len() == 1 {
                    let index = self.evaluate_slot_index("Local()", &args[0], env, 0)?;
                    let cell = self.numbered_local_cell(env, index, Some(object_value));
                    *cell.borrow_mut() = value;
                    return Ok(());
                }
                let object_id = match object_value {
                    Value::Int(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => format!("{:?}", object_value),
                };

                // Evaluate arguments to create the key
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                let key = arg_values
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => n.to_string(),
                        Value::String(s) => s.clone(),
                        _ => format!("{:?}", v),
                    })
                    .collect::<Vec<_>>()
                    .join("_");

                // Store in environment with naming scheme: __method_{object_id}_{method}_{key}
                let slot_name = format!("__method_{}_{}_{}", object_id, method, key);
                env.define(&slot_name, value);
                Ok(())
            }
            _ => self
                .assignment_target_to_lvalue(env, target, 0)?
                .write(value),
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
            self.evaluate(value_expr, env, depth)?;
            return Err(RuntimeError::new(format!(
                "operator \"{operator}\" left side: got \"{}\", but expected \"&\"!",
                left.type_name()
            )));
        }

        // Preserve the existing RHS-first behavior for valid targets; this
        // issue only restores AB_Set's invalid-value path.
        let tracked = self.evaluate_tracked(value_expr, env, depth)?;
        let value = tracked.value.clone();
        self.assign_target_tracked(env, target, tracked)?;
        Ok(value)
    }

    fn assign_target_tracked(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
        tracked: TrackedValue,
    ) -> Result<(), RuntimeError> {
        if let AssignmentTarget::Variable(name) = target {
            if env.assign_tracked(name, tracked.clone()).is_ok() {
                return Ok(());
            }
        }
        match target {
            AssignmentTarget::MethodSlot { method, .. }
                if matches!(method.as_str(), "LocalN" | "Local") =>
            {
                self.assignment_target_to_lvalue(env, target, 0)?
                    .write_tracked(tracked)
            }
            AssignmentTarget::EffectSlot(_) | AssignmentTarget::MethodSlot { .. } => {
                self.assign_target(env, target, tracked.value)
            }
            _ => self
                .assignment_target_to_lvalue(env, target, 0)?
                .write_tracked(tracked),
        }
    }

    fn assignment_target_value(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
    ) -> Result<Value, RuntimeError> {
        match target {
            AssignmentTarget::EffectSlot(args) => {
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                // `EffectVar(...)` reads through the host's real effect
                // variables (FnEffectVar by-reference, C4Script.cpp) —
                // compound assignments (--EffectVar) must see live values.
                if let Some(host) = self.host_functions.get("EffectVar") {
                    let _guard = CallerSlotsGuard::enter(Some(env.var_slots.clone()));
                    return self.invoke_host_function("EffectVar", host, &arg_values);
                }
                // Host-less fixture VMs keep the legacy env-slot shim.
                let slot_name = format!(
                    "__effect_{}",
                    arg_values
                        .iter()
                        .map(|v| match v {
                            Value::Int(n) => n.to_string(),
                            Value::String(s) => s.clone(),
                            _ => format!("{:?}", v),
                        })
                        .collect::<Vec<_>>()
                        .join("_")
                );
                Ok(env.get(&slot_name)?.unwrap_or(Value::Nil))
            }
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
            } => {
                if !matches!(method.as_str(), "LocalN" | "Local" | "Var" | "EffectVar") {
                    return self.assignment_target_to_lvalue(env, target, 0)?.read();
                }
                // Evaluate the object to get its identity
                let object_value = self.evaluate(object, env, 0)?;
                // `LocalN("name", obj)` / `obj->LocalN("name")` reads the
                // TARGET's named local through the resolved cell (FnLocalN
                // by-reference access, C4Script.cpp:4591-4605).
                if method == "LocalN" && args.len() == 1 {
                    let local_name = match self.evaluate(&args[0], env, 0)? {
                        Value::String(local_name) => local_name,
                        other => {
                            return Err(RuntimeError::new(format!(
                                "LocalN: expected string for name, got {}",
                                other.type_name()
                            )))
                        }
                    };
                    let cell = self.localn_cell(env, &local_name, Some(object_value));
                    let value = cell.borrow().clone();
                    return Ok(value);
                }
                // `Local(n, obj)` compound reads (FnLocal by-reference,
                // C4Script.cpp:3423-3433).
                if method == "Local" && args.len() == 1 {
                    let index = self.evaluate_slot_index("Local()", &args[0], env, 0)?;
                    if index < 0 {
                        return Ok(Value::Nil);
                    }
                    let cell = self.numbered_local_cell(env, index, Some(object_value));
                    let value = cell.borrow().clone();
                    return Ok(value);
                }
                let object_id = match object_value {
                    Value::Int(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => format!("{:?}", object_value),
                };

                // Evaluate arguments to create the key
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                let key = arg_values
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => n.to_string(),
                        Value::String(s) => s.clone(),
                        _ => format!("{:?}", v),
                    })
                    .collect::<Vec<_>>()
                    .join("_");

                // Retrieve from environment with naming scheme: __method_{object_id}_{method}_{key}
                let slot_name = format!("__method_{}_{}_{}", object_id, method, key);
                Ok(env.get(&slot_name)?.unwrap_or(Value::Nil))
            }
            _ => self.assignment_target_to_lvalue(env, target, 0)?.read(),
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
                reference.detach_container_identity_if_shared();
                Ok(reference.append(PathSegment::Property(property.clone())))
            }
            AssignmentTarget::Index(base, index_expr) => {
                let index = self.evaluate(index_expr, env, depth)?;
                let reference = self.assignment_target_to_lvalue(env, base, depth)?;
                reference.detach_container_identity_if_shared();
                Ok(reference.append(PathSegment::Index(index)))
            }
            AssignmentTarget::ArrayAppend(base) => {
                let reference = self.assignment_target_to_lvalue(env, base, depth)?;
                self.append_array_slot(reference)
            }
            AssignmentTarget::LocalSlot(index_expr) => {
                let index = self.evaluate_slot_index("Local()", index_expr, env, depth)?;
                Ok(self.tracked_cell(env.object_state.local_slot_cell(index)))
            }
            AssignmentTarget::VarSlot(index_expr) => {
                let index = self.evaluate_slot_index("Var()", index_expr, env, depth)?;
                Ok(self.tracked_cell(slot_cell(&env.var_slots, index)))
            }
            AssignmentTarget::EffectSlot(args) => {
                let arg_values = args
                    .iter()
                    .map(|arg| self.evaluate(arg, env, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(function) = self.host_functions.get("EffectVar") {
                    return Ok(LValueRef::HostPath {
                        function: function.clone(),
                        args: arg_values,
                        caller_slots: env.var_slots.clone(),
                        segments: Vec::new(),
                    });
                }

                // Host-less fixture VMs retain EffectVar slots in ordinary
                // environment cells; exposing that cell keeps the same
                // reference/path behavior as the engine-backed variant.
                let slot_name = format!(
                    "__effect_{}",
                    arg_values
                        .iter()
                        .map(|value| match value {
                            Value::Int(value) => value.to_string(),
                            Value::String(value) => value.clone(),
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
                let local_name = match self.evaluate(&args[0], env, depth + 1)? {
                    Value::String(local_name) => local_name,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "LocalN: expected string for name, got {}",
                            other.type_name()
                        )))
                    }
                };
                let target = args
                    .get(1)
                    .map(|arg| self.evaluate(arg, env, depth + 1))
                    .transpose()?;
                Ok(self.tracked_cell(self.localn_cell(env, &local_name, target)))
            }
            AssignmentTarget::FunctionCall { name, args }
                if name == "Par"
                    && args.len() <= 1
                    && !self.functions.contains_key(name)
                    && !self.has_host_function(name) =>
            {
                let index = args
                    .first()
                    .map(|arg| self.evaluate(arg, env, depth + 1))
                    .transpose()?
                    .map(|value| match value {
                        Value::Int(index) => Ok(index),
                        Value::Nil => Ok(0),
                        Value::Bool(flag) => Ok(i32::from(flag)),
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
                let function = self.functions.get(name);
                let args =
                    self.build_call_args(Some(name), function, args, env, depth + 1)?;
                self.invoke_reference(
                    name,
                    args,
                    depth + 1,
                    env.object_state.clone(),
                    Some(env.var_slots.clone()),
                )
            }
            // `LocalN("name", obj) += v` and friends: the foreign-local
            // cell IS the reference (FnLocalN, C4Script.cpp:4591-4605).
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
            } if method == "LocalN" && args.len() == 1 => {
                let object_value = self.evaluate(object, env, depth + 1)?;
                let local_name = match self.evaluate(&args[0], env, depth + 1)? {
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
            } if method == "Local" && args.len() == 1 => {
                let object_value = self.evaluate(object, env, depth + 1)?;
                let index = self.evaluate_slot_index("Local()", &args[0], env, depth)?;
                Ok(self.tracked_cell(self.numbered_local_cell(env, index, Some(object_value))))
            }
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
            } if !matches!(method.as_str(), "Var" | "EffectVar") => {
                let mut target = self.evaluate(object, env, depth + 1)?;
                if let Value::Proplist(map) = &target {
                    if let Some(Value::Int(id)) = map.get("id") {
                        if *id > 0 {
                            target = Value::Object(*id as u64);
                        }
                    }
                }
                if matches!(target, Value::Nil | Value::Int(0) | Value::Bool(false)) {
                    return Err(RuntimeError::new("Object call: target is zero!"));
                }

                let function = self.functions.get(method);
                let evaluated_args =
                    self.build_call_args(Some(method), function, args, env, depth + 1)?;
                match &target {
                    Value::Object(_) | Value::C4Id(_)
                        if self.method_reference_dispatch.is_some() =>
                    {
                        let mut dispatch_args = Vec::with_capacity(evaluated_args.len() + 3);
                        dispatch_args.push(target);
                        dispatch_args.push(Value::String(method.clone()));
                        dispatch_args.push(Value::Bool(false));
                        for arg in &evaluated_args {
                            dispatch_args.push(arg.read()?);
                        }
                        self.method_reference_dispatch
                            .ok_or_else(|| {
                                RuntimeError::new("method reference dispatch vanished")
                            })?(&dispatch_args)
                        .map(ValueReference::into_lvalue)
                    }
                    Value::Object(_) | Value::C4Id(_) => self.invoke_reference(
                        method,
                        evaluated_args,
                        depth + 1,
                        env.object_state.clone(),
                        Some(env.var_slots.clone()),
                    ),
                    other if self.method_reference_dispatch.is_some() => {
                        Err(RuntimeError::new(format!(
                            "Object call: Invalid target type {}, expected object or id!",
                            other.type_name()
                        )))
                    }
                    _ => self.invoke_reference(
                        method,
                        evaluated_args,
                        depth + 1,
                        env.object_state.clone(),
                        Some(env.var_slots.clone()),
                    ),
                }
            }
            AssignmentTarget::MethodSlot { .. } => Err(RuntimeError::new(
                "this assignment target cannot be passed by reference",
            )),
        }
    }

    fn expr_to_lvalue(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<LValueRef, RuntimeError> {
        let target = Self::expr_to_assignment_target(expr)?;
        self.assignment_target_to_lvalue(env, &target, depth)
    }

    /// Resolve variable-rooted container paths without evaluating them to a
    /// detached `Value` clone. Definite `func &` calls retain their returned
    /// reference too; value-call results remain ordinary rvalues.
    fn existing_path_lvalue(
        &self,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<Option<LValueRef>, RuntimeError> {
        match expr {
            Expr::Variable(name) => Ok(env.lvalue(name).or_else(|| {
                self.global_variable_cell(name)
                    .map(|cell| self.tracked_cell(cell))
            })),
            Expr::Property(base, property) => Ok(self
                .existing_path_lvalue(base, env, depth)?
                .map(|reference| reference.append(PathSegment::Property(property.clone())))),
            Expr::Index(base, index_expr) => {
                let Some(reference) = self.existing_path_lvalue(base, env, depth)? else {
                    return Ok(None);
                };
                let collection = reference.read()?;
                let index = self.evaluate(index_expr, env, depth)?;
                Self::grow_empty_negative_array(Some(&reference), &collection, &index)?;
                Ok(Some(reference.append(PathSegment::Index(index))))
            }
            Expr::ArrayAppend(_) => self.expr_to_lvalue(expr, env, depth).map(Some),
            Expr::Call {
                callee,
                is_optional,
                ..
            } if !is_optional
                && matches!(callee.as_ref(), Expr::Variable(name) if self
                    .functions
                    .get(name)
                    .or_else(|| self.global_functions.and_then(|functions| functions.get(name)))
                    .is_some_and(|function| function.returns_reference)) =>
            {
                self.expr_to_lvalue(expr, env, depth).map(Some)
            }
            _ => Ok(None),
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
        let appended = reference.append(PathSegment::Index(Value::Int(index)));
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
        match self.evaluate(expr, env, depth)? {
            Value::Int(index) => Ok(index),
            // Var/Local/SetLocal are typed C4ValueInt engine functions in
            // C++; C4Value::getInt converts nil to zero and bool directly
            // before FnVar/FnLocal sees the index (C4Value.h:159,317-321;
            // C4Value.cpp:453-466,499-522).
            Value::Nil => Ok(0),
            Value::Bool(flag) => Ok(i32::from(flag)),
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
    ) -> Result<TrackedValue, RuntimeError> {
        let index = self.evaluate_slot_index("SetLocal()", &args[0], env, depth)?;
        let tracked = args
            .get(1)
            .map(|arg| self.evaluate_tracked(arg, env, depth))
            .transpose()?
            .unwrap_or_else(|| TrackedValue::runtime(Value::Nil));
        let explicit_target = args
            .get(2)
            .map(|arg| self.evaluate(arg, env, depth))
            .transpose()?;
        let target = explicit_target
            .filter(|value| !matches!(value, Value::Nil | Value::Int(0) | Value::Bool(false)))
            .or(default_target);
        let cell = self.numbered_local_cell(env, index, target);
        self.tracked_cell(cell).write_tracked(tracked.clone())?;
        Ok(tracked)
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
                    Box::new((**index).clone()),
                ))
            }
            Expr::ArrayAppend(base) => {
                let base_target = Self::expr_to_assignment_target(base)?;
                Ok(AssignmentTarget::ArrayAppend(Box::new(base_target)))
            }
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
                        });
                    }
                }
                Err(RuntimeError::new(format!(
                    "invalid increment/decrement target: {:?}",
                    expr
                )))
            }
            _ => Err(RuntimeError::new(format!(
                "invalid increment/decrement target: {:?}",
                expr
            ))),
        }
    }

    fn get_target_value(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
    ) -> Result<Value, RuntimeError> {
        self.assignment_target_value(env, target)
    }
}

enum ControlFlow {
    Normal,
    Break,
    LoopContinue,
    Return(ReturnValue),
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
    scopes: Vec<HashMap<String, Binding>>,
    /// `#strict` level of the executing function, for level-correct `==`/`!=`.
    strict_level: Option<u8>,
    /// C4Script numeric scratch slots, addressed by `Var(n)` / `Local(n)`. These
    /// are SEPARATE from named variables (C++ `NumVars` and the object `Local`
    /// array, not `Vars`/`LocalNamed`) and are function-scoped, not block-scoped,
    /// so a `Local(0) = x` inside a block stays visible after it. Unset reads as
    /// nil and the index is clamped to >= 0 (C4ValueList::GetItem). `var_slots`
    /// are per-call; `local_slots` round-trip through the object's `local_vars`.
    var_slots: SlotMap,
    object_state: ObjectState,
    /// The full argument slots of the executing call: `Par(i)` reads them
    /// (C4AulExec.cpp:1127-1140) and `Callee(...)` forwards the slots past
    /// `named_param_count` (C4AulParse.cpp:2293-2306, ParNamed.iSize).
    call_args: Rc<Vec<Binding>>,
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
            .map(|(index, arg)| match arg {
                CallArg::Reference(reference)
                    if params
                        .get(index)
                        .is_some_and(|parameter| parameter.is_reference) =>
                {
                    Ok(Binding::Reference(reference.clone()))
                }
                _ => Ok(Binding::tracked(arg.read_tracked()?)),
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        while call_args.len() < MAX_CALL_PARAMETERS {
            call_args.push(Binding::direct(Value::Nil));
        }
        let mut scopes = vec![HashMap::new()];
        let base = scopes.last_mut().unwrap();
        for (param, binding) in params.iter().zip(call_args.iter()) {
            base.insert(param.name.clone(), binding.clone());
        }
        Ok(Self {
            scopes,
            strict_level,
            var_slots: Rc::new(RefCell::new(HashMap::new())),
            object_state,
            call_args: Rc::new(call_args),
            named_param_count: params.len(),
            inherited_target: None,
            function_name: String::new(),
            engine_scope: false,
        })
    }

    fn define_object_local(&mut self, name: &str, identity: RawIdentityCell) {
        let cell = self.object_state.named_local_cell(name);
        if self.scopes.iter().any(|scope| scope.contains_key(name)) {
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
        self.scopes.push(HashMap::new());
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

    /// Pre-declare a hoisted `var` slot (nil) unless the name already
    /// exists — parameters share the C4Aul var table and keep their value.
    fn declare_hoisted(&mut self, name: &str) {
        let exists = self.scopes.iter().any(|scope| scope.contains_key(name));
        if !exists {
            self.define(name, Value::Nil);
        }
    }

    fn assign(&mut self, name: &str, value: Value) -> Result<(), RuntimeError> {
        self.assign_tracked(name, TrackedValue::runtime(value))
    }

    fn assign_tracked(&mut self, name: &str, tracked: TrackedValue) -> Result<(), RuntimeError> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(binding) = scope.get(name) {
                return binding.write_tracked(tracked);
            }
        }
        Err(RuntimeError::new(format!("undefined variable '{name}'")))
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
        Ok(None)
    }

    fn lvalue(&self, name: &str) -> Option<LValueRef> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value.lvalue());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn execute_script(
        source: &str,
        entry_point: &str,
        args: &[Value],
    ) -> Result<Value, RuntimeError> {
        let script = Parser::new(source)
            .parse_script()
            .expect("parse should succeed");
        let functions: HashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|f| (f.name.clone(), f))
            .collect();
        let host_functions = HashMap::new();
        let var_decls: Vec<VarDecl> = Vec::new();
        let vm = Vm::new(&functions, &host_functions, &var_decls, None);
        vm.call(entry_point, args)
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
        .parse_script()
        .expect("object script parses");
        let object_functions: HashMap<String, Function> = object_script
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
        .parse_script()
        .expect("global script parses");
        let global_functions: HashMap<String, Function> = global_script
            .functions
            .into_iter()
            .map(|function| (function.name.clone(), function))
            .collect();
        let host_functions = HashMap::new();
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
            .parse_script()
            .expect("parse should succeed");
        let functions: HashMap<String, Function> = script
            .functions
            .into_iter()
            .map(|f| (f.name.clone(), f))
            .collect();
        let host_functions = HashMap::new();
        let var_decls: Vec<VarDecl> = Vec::new();
        let cell = value_cell(Value::Nil);
        let hook_cell = cell.clone();
        let hook: crate::engine::LocalCellHook = std::rc::Rc::new(move |target, name| {
            (matches!(target, Value::Int(42)) && name == "__local_2")
                .then(|| hook_cell.clone())
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
            "func Merge(target, number, name, target, timer, change) { return [target, timer, change]; }";
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
    fn vm_enforces_call_depth_limit() {
        let source = r#"
            func Recursive(n) {
                if (n <= 0) return 0;
                return Recursive(n - 1);
            }
        "#;
        // Should fail past MAX_CALL_DEPTH (512, matching C++ MAX_CONTEXT_STACK).
        let error = execute_script(source, "Recursive", &[Value::Int(1000)]).unwrap_err();
        assert!(error.message().contains("maximum call depth exceeded"));
    }

    #[test]
    fn vm_handles_array_creation() {
        let source = "func Test() { var arr = [1, 2, 3]; return arr[1]; }";
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn vm_handles_array_index_assignment() {
        let source = r#"
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
            func Test() {
                var a = [7, 8];
                var reads = [a[-1], a[nil], a[true], a[2]];
                a[-1] = 5;
                var written = a[0];
                var e = [];
                var empty = e[-1];
                var old = a[-1]++;
                var coerced = [0, 0];
                coerced[nil] = 3;
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
        let source = "func Grow(index) { var a = []; a[index] = 1; return a; }";
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
                var nested = [["abc"]];
                return [
                    "abc"[0],
                    "abc"[-1],
                    "abc"[5],
                    "abc"[-5],
                    "abc"[nil],
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
        let source = r#"#strict 1
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
        let source = "func Test() { var obj = { x = 10 }; return obj.x; }";
        let result = execute_script(source, "Test", &[]).unwrap();
        assert_eq!(result, Value::Int(10));
    }

    #[test]
    fn vm_handles_proplist_property_assignment() {
        let source = r#"
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
                Value::String("secondfirstthird".to_string()),
                Value::Int(36),
                Value::String("third".to_string()),
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
            Value::String("b".to_string()),
            Value::Int(2),
            Value::String("a".to_string()),
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
        let source = r#"#strict 1
            func Test() {
                var entries = { entry = "same" };
                var old_value = entries.entry;
                entries.entry = nil;
                return [entries.entry == old_value, entries.entry == nil];
            }
        "#;

        assert_eq!(
            execute_script(source, "Test", &[]).expect("map property removal runs"),
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
        assert_eq!(result2, Value::Int(0));
    }
}
