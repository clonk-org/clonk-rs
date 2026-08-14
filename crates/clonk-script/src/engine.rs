use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use indexmap::IndexMap;

use crate::ast::{Function, Script as AstScript, VarDecl};
use crate::debugger::DebuggerHooks;
use crate::error::{ParseError, RuntimeError, ScriptError};
use crate::parser::Parser;
use crate::value::{C4StringValue, C4StringValueInner, C4VType, Value};
use crate::vm::{HostCallArg, ValueReference, Vm};

pub type HostFunction = Arc<dyn Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync>;

/// A native callback together with the parameter count declared by its C++
/// registration. Legacy embedding-only callbacks may remain variadic, but
/// engine natives use an exact count so the VM can balance their call frame
/// after evaluating every supplied expression.
#[derive(Clone)]
pub(crate) struct RegisteredHostFunction {
    callback: HostFunction,
    parameter_count: Option<usize>,
}

impl RegisteredHostFunction {
    fn variadic(callback: HostFunction) -> Self {
        Self {
            callback,
            parameter_count: None,
        }
    }

    fn declared(callback: HostFunction, parameter_count: usize) -> Self {
        Self {
            callback,
            parameter_count: Some(parameter_count),
        }
    }

    pub(crate) fn callback(&self) -> &HostFunction {
        &self.callback
    }

    pub(crate) fn parameter_count(&self) -> Option<usize> {
        self.parameter_count
    }
}

type HostReferenceCallback =
    Arc<dyn Fn(&[HostCallArg]) -> Result<Value, RuntimeError> + Send + Sync>;

/// One native function whose selected parameters retain C4Value references.
/// Ordinary [`HostFunction`] registrations remain value-only.
#[derive(Clone)]
pub(crate) struct HostReferenceFunction {
    callback: HostReferenceCallback,
    reference_parameters: Vec<usize>,
    parameter_count: Option<usize>,
}

impl HostReferenceFunction {
    fn new<F, I>(reference_parameters: I, parameter_count: Option<usize>, callback: F) -> Self
    where
        F: Fn(&[HostCallArg]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
        I: IntoIterator<Item = usize>,
    {
        let mut reference_parameters = reference_parameters.into_iter().collect::<Vec<_>>();
        reference_parameters.sort_unstable();
        reference_parameters.dedup();
        Self {
            callback: Arc::new(callback),
            reference_parameters,
            parameter_count,
        }
    }

    pub(crate) fn wants_reference(&self, index: usize) -> bool {
        if self
            .parameter_count
            .is_some_and(|parameter_count| index >= parameter_count)
        {
            return false;
        }
        self.reference_parameters.binary_search(&index).is_ok()
    }

    pub(crate) fn call(&self, args: &[HostCallArg]) -> Result<Value, RuntimeError> {
        (self.callback)(args)
    }

    pub(crate) fn parameter_count(&self) -> Option<usize> {
        self.parameter_count
    }
}

/// An immutable, shareable snapshot of one script host's native callbacks,
/// parameter signatures, and engine constants.
///
/// Engines install snapshots copy-on-write: untouched hosts share the backing
/// maps, while later registrations or overrides remain local to that host.
/// Constants containing C4String values are rejected because their
/// registration order belongs to each host's string-table construction.
#[derive(Clone)]
pub struct HostRegistrationSnapshot {
    host_functions: Arc<FxHashMap<String, RegisteredHostFunction>>,
    host_reference_functions: Arc<FxHashMap<String, HostReferenceFunction>>,
    host_function_parameter_types: Arc<FxHashMap<String, Arc<[C4VType]>>>,
    constants: Arc<FxHashMap<String, Value>>,
}

fn value_contains_c4_string(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Array(values) => values.iter().any(value_contains_c4_string),
        Value::Proplist(values) => {
            values.iter().any(|(key, value)| {
                value_contains_c4_string(key) || value_contains_c4_string(value)
            }) || values.hidden_values().any(value_contains_c4_string)
        }
        Value::Int(_)
        | Value::Bool(_)
        | Value::RawBool(_)
        | Value::C4Id(_)
        | Value::Object(_)
        | Value::Nil => false,
    }
}

fn empty_host_registration_snapshot() -> &'static HostRegistrationSnapshot {
    static EMPTY: std::sync::OnceLock<HostRegistrationSnapshot> = std::sync::OnceLock::new();
    EMPTY.get_or_init(|| HostRegistrationSnapshot {
        host_functions: Arc::new(FxHashMap::default()),
        host_reference_functions: Arc::new(FxHashMap::default()),
        host_function_parameter_types: Arc::new(FxHashMap::default()),
        constants: Arc::new(FxHashMap::default()),
    })
}

/// Cross-object `func &` dispatch. Kept separate from [`HostFunction`] so an
/// lvalue call result is never flattened to a copied [`Value`].
pub type MethodReferenceDispatch =
    std::rc::Rc<dyn Fn(&[Value]) -> Result<ValueReference, RuntimeError>>;

/// Cross-object dispatch for an arrow call whose callee declares `&`
/// parameters. C++ hands the callee `C4V_pC4Value` slots pointing straight at
/// the caller's stack (C4AulExec.cpp:1364-1397), which a `&[Value]` bridge
/// cannot carry; this variant therefore also reports each parameter slot's
/// final value so the caller can settle its reference cells. Slots the callee
/// did not declare `&` keep the value that was passed in, because
/// `CheckConvertFunctionParameters` gives those parameters a dereferenced
/// copy (C4Value.cpp:586-597).
pub type MethodRefArgsDispatch =
    std::rc::Rc<dyn Fn(&[Value]) -> Result<(Value, Vec<Value>), RuntimeError>>;

/// `C4AulParse::Parse_Params`' `anyfunctakesref` test (C4AulParse.cpp:
/// 2318-2331): an argument expression keeps its reference bytecode unless
/// EVERY engine function with the callee's name takes a non-reference at that
/// slot. The parser walks `GetFirstFunc`/`GetNextSNFunc`, which spans all
/// script hosts — knowledge only the embedding engine has, so it supplies this
/// probe: `(function name, zero-based slot) -> any same-name function declares
/// `&` there`.
pub type ReferenceParameterProbe = std::rc::Rc<dyn Fn(&str, usize) -> bool>;

/// `C4AulScriptEngine::GetFirstFunc` lookup used while compiling direct object
/// calls (C4AulParse.cpp:3194-3229). An unresolved `->~Name(...)` emits no
/// AB_CALLFS at all, so the embedding engine supplies its whole linked function
/// namespace rather than a target-specific method lookup.
pub type DirectCallFunctionProbe = std::rc::Rc<dyn Fn(&str) -> bool>;

/// Enters/leaves the native no-object scope required by strict-3
/// `global->Fn(...)`. The script VM owns unwinding; the embedding engine owns
/// its object/definition context and therefore supplies this small hook.
pub type GlobalCallContextHook = Arc<dyn Fn(bool) + Send + Sync>;

/// Embedding-engine implementation of C++ `FnEval` receiver selection and
/// DirectExec. It receives the expression, shared object-local cells, `this`,
/// caller strictness and current VM depth; `None` preserves the standalone
/// VM fallback when no engine host context is active.
pub type EvalDirectExecHook = std::rc::Rc<
    dyn Fn(
        &str,
        &crate::vm::LocalCells,
        Value,
        Option<u8>,
        usize,
    ) -> Option<Result<Value, RuntimeError>>,
>;

/// The engine-global named-variable table (`static` declarations;
/// C4AulScriptEngine::GlobalNamed): one shared table across every script
/// host. Values live in cells so lvalues (x = .., x++, ...) write through.
pub type GlobalVariables = std::rc::Rc<std::cell::RefCell<IndexMap<String, crate::vm::ValueCell>>>;

/// The engine-global numbered-variable table (`C4AulScriptEngine::Global`).
/// It is separate from [`GlobalVariables`] because numeric slots and declared
/// named statics have independent namespaces in C++.
pub type GlobalSlots =
    std::rc::Rc<std::cell::RefCell<std::collections::BTreeMap<i32, crate::vm::ValueCell>>>;

/// Supplies a live cell for a FOREIGN object's named local —
/// FnLocalN returns `pVarN->GetRef()` (C4Script.cpp:4591-4605), a
/// reference into the target's locals, so cross-object reads AND lvalue
/// writes go through it. Registered by the engine like method_dispatch.
pub type LocalCellHook = std::rc::Rc<dyn Fn(&Value, &str) -> Option<crate::vm::ValueCell>>;

/// Reports whether a nonzero object id is still a valid AB_CALL receiver in
/// the embedding world's current synchronous callback.
pub type ObjectTargetAvailabilityProbe = std::rc::Rc<dyn Fn(u64) -> bool>;

pub fn new_global_variables() -> GlobalVariables {
    std::rc::Rc::new(std::cell::RefCell::new(IndexMap::new()))
}

/// One process-global `C4StringTable` registration ledger.
///
/// Native strings retain their previous `iEnumID` until the next explicit
/// `EnumStrings` call. Scenario-section saves observe that old enumeration
/// before object serialization assigns a new one, so registration order alone
/// is not enough to reproduce their `Strings.txt` payload.
#[derive(Debug, Default)]
pub struct StringRegistrationLedger {
    state: Mutex<StringRegistrationLedgerState>,
}

#[cfg(test)]
thread_local! {
    static STRING_REGISTRATION_MUTABLE_BORROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static STRING_REGISTRATION_ENTRY_SWEEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_string_registration_mutable_borrows() {
    STRING_REGISTRATION_MUTABLE_BORROWS.with(|count| count.set(0));
}

#[cfg(test)]
fn string_registration_mutable_borrows() -> usize {
    STRING_REGISTRATION_MUTABLE_BORROWS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_string_registration_entry_sweeps() {
    STRING_REGISTRATION_ENTRY_SWEEPS.with(|count| count.set(0));
}

#[cfg(test)]
fn string_registration_entry_sweeps() -> usize {
    STRING_REGISTRATION_ENTRY_SWEEPS.with(std::cell::Cell::get)
}

#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct StringRegistrationLedgerState {
    entries: Vec<StringRegistration>,
    /// Non-owning O(1) membership for the ordered registration list. Each
    /// indexed pointer still has its Weak in `entries`, keeping the Arc
    /// control-block address reserved until both are pruned together.
    registered_identities: FxHashSet<usize>,
    /// Runtime registrations sweep dead weak entries only after the table has
    /// grown materially beyond its last live size. Keeping the weak control
    /// blocks until then prevents pointer reuse, so identity membership stays
    /// exact while cleanup remains amortized O(1).
    next_entry_prune_len: usize,
    /// Parse-time literal identities by their parser spelling.
    ///
    /// C++ stores the resolved `C4String *` directly in each AB_STRING
    /// instruction (C4AulParse.cpp:2440-2442), so executing a literal never
    /// scans the table again. Keep this index weak: a cache handle must not
    /// increment `C4String::iRefCnt` and make an otherwise unreferenced Hold
    /// string eligible for save enumeration.
    literal_cache: FxHashMap<String, std::sync::Weak<C4StringValueInner>>,
    /// Identities unregistered by C4StringTable::Clear while an external
    /// C4Value still owns them. They remain valid strings, but pTable is null
    /// in C++ and later live-value traversal must not silently re-register
    /// the same pointer.
    detached: Vec<std::sync::Weak<C4StringValueInner>>,
    detached_identities: FxHashSet<usize>,
}

impl StringRegistrationLedgerState {
    fn identity(value: &std::sync::Weak<C4StringValueInner>) -> usize {
        value.as_ptr() as usize
    }

    fn rebuild_registered_identities(&mut self) {
        self.registered_identities.clear();
        self.registered_identities.extend(
            self.entries
                .iter()
                .map(|entry| Self::identity(&entry.value)),
        );
    }

    fn rebuild_detached_identities(&mut self) {
        self.detached_identities.clear();
        self.detached_identities
            .extend(self.detached.iter().map(Self::identity));
    }

    fn retain_live_entries(&mut self) {
        #[cfg(test)]
        STRING_REGISTRATION_ENTRY_SWEEPS.with(|count| count.set(count.get() + 1));
        self.entries
            .retain(|candidate| candidate.value.strong_count() != 0);
        self.rebuild_registered_identities();
        self.next_entry_prune_len = self
            .entries
            .len()
            .saturating_mul(2)
            .max(self.entries.len().saturating_add(64));
    }

    fn retain_live_entries_if_due(&mut self) -> bool {
        let prune_at = self.next_entry_prune_len.max(64);
        if self.entries.len() < prune_at {
            return false;
        }
        self.retain_live_entries();
        true
    }

    fn retain_live_detached(&mut self) {
        self.detached
            .retain(|candidate| candidate.strong_count() != 0);
        self.rebuild_detached_identities();
    }

    fn push_entry(&mut self, entry: StringRegistration) {
        self.registered_identities
            .insert(Self::identity(&entry.value));
        self.entries.push(entry);
    }

    #[cfg(test)]
    fn registered_identity_count(&self) -> usize {
        self.registered_identities.len()
    }

    fn literal_lookup(&self, value: &str) -> Option<C4StringValue> {
        self.literal_cache
            .get(value)
            .and_then(std::sync::Weak::upgrade)
            .map(C4StringValue::from_inner)
    }
}

impl Clone for StringRegistrationLedger {
    fn clone(&self) -> Self {
        Self {
            state: Mutex::new(self.borrow().clone()),
        }
    }
}

impl StringRegistrationLedger {
    /// Lock the shared string-table state for inspection.
    ///
    /// This keeps the former `RefCell::borrow` call surface while allowing
    /// one process-global ledger to cross worker-thread boundaries safely.
    #[doc(hidden)]
    pub fn borrow(&self) -> MutexGuard<'_, StringRegistrationLedgerState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Lock the shared string-table state for mutation.
    ///
    /// A mutex is exclusive for both helpers; the separate name preserves
    /// the old `borrow_mut` call sites and makes mutation intent explicit.
    #[doc(hidden)]
    pub fn borrow_mut(&self) -> MutexGuard<'_, StringRegistrationLedgerState> {
        #[cfg(test)]
        STRING_REGISTRATION_MUTABLE_BORROWS.with(|count| count.set(count.get() + 1));
        self.borrow()
    }
}

#[derive(Clone, Debug)]
struct StringRegistration {
    /// C4StringTable itself owns no ordinary reference to runtime strings.
    /// Their last C4Value release therefore invalidates this weak table link.
    value: std::sync::Weak<C4StringValueInner>,
    /// Parser-owned strings survive with zero C4Value references (`Hold`).
    held: Option<C4StringValue>,
    /// A newly loaded non-Hold C4String starts at refcount zero but survives
    /// until its first IncRef/DecRef cycle. This root is consumed by
    /// `resolve_c4_string`, after which ordinary handle lifetime is decisive.
    untouched_loaded: Option<C4StringValue>,
}

impl StringRegistration {
    fn upgrade(&self) -> Option<C4StringValue> {
        self.value.upgrade().map(C4StringValue::from_inner)
    }

    fn c4_value_ref_count(&self) -> usize {
        // Weak::strong_count observes every live C4StringValue handle. The
        // ledger's Hold and untouched-load roots model native ownership that
        // is deliberately *not* part of C4String::iRefCnt, so subtract those
        // two roots before applying EnumStrings' refcount test.
        self.value
            .strong_count()
            .saturating_sub(usize::from(self.held.is_some()))
            .saturating_sub(usize::from(self.untouched_loaded.is_some()))
    }

    fn enum_eligible(&self) -> bool {
        self.held.is_none() || self.c4_value_ref_count() != 0
    }
}

/// Registration/enumeration state shared by every script host in one game.
pub type StringRegistrations = Arc<StringRegistrationLedger>;

fn c4_string_table_prefix(bytes: &[u8]) -> &[u8] {
    // C4StringTable::{FindString,FindSaveString} use SEqual on `getData()`.
    // This is intentionally narrower than C4Value string equality, which is
    // length-aware through StdStrBuf::operator==.
    &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())]
}

fn c4_string_table_values_equal(left: &str, right: &str) -> bool {
    let left = crate::value::c4_string_bytes_cow(left);
    let right = crate::value::c4_string_bytes_cow(right);
    c4_string_table_prefix(left.as_ref()) == c4_string_table_prefix(right.as_ref())
}

fn c4_string_table_bytes_equal(left: &[u8], right: &[u8]) -> bool {
    c4_string_table_prefix(left) == c4_string_table_prefix(right)
}

pub fn new_string_registrations() -> StringRegistrations {
    Arc::new(StringRegistrationLedger::default())
}

/// `C4StringTable::Clear` at the global script-unlink boundary.
///
/// Every parser-Hold entry is unregistered, not merely un-Held. Dropping the
/// Hold root deletes zero-reference strings immediately. A C4Value-referenced
/// string survives as a detached identity until its final handle is released;
/// the tombstone prevents enumeration's recovery pass from attaching it to
/// the table again.
pub fn clear_c4_string_holds(registrations: &StringRegistrationLedger) {
    let mut registrations = registrations.borrow_mut();
    registrations.literal_cache.clear();
    registrations.retain_live_detached();

    let mut retained = Vec::with_capacity(registrations.entries.len());
    let mut detached = std::mem::take(&mut registrations.detached);
    for mut entry in std::mem::take(&mut registrations.entries) {
        if entry.held.take().is_none() {
            if entry.value.strong_count() != 0 {
                retained.push(entry);
            }
            continue;
        }

        if entry.value.strong_count() != 0
            && !detached
                .iter()
                .any(|candidate| candidate.ptr_eq(&entry.value))
        {
            detached.push(entry.value);
        }
    }
    registrations.entries = retained;
    registrations.detached = detached;
    registrations.rebuild_registered_identities();
    registrations.rebuild_detached_identities();
}

fn register_c4_string_locked(
    registrations: &mut StringRegistrationLedgerState,
    value: &C4StringValue,
    pruned: &mut bool,
) {
    let identity = value.identity();
    if registrations.registered_identities.contains(&identity)
        || registrations.detached_identities.contains(&identity)
    {
        return;
    }
    if !*pruned {
        registrations.retain_live_detached();
        if registrations.detached_identities.contains(&identity) {
            return;
        }
        if registrations.retain_live_entries_if_due() {
            if registrations.registered_identities.contains(&identity) {
                return;
            }
            *pruned = true;
        }
    }
    registrations.push_entry(StringRegistration {
        value: value.downgrade(),
        held: None,
        untouched_loaded: None,
    });
}

pub fn register_c4_string(registrations: &StringRegistrationLedger, value: &C4StringValue) {
    let mut registrations = registrations.borrow_mut();
    register_c4_string_locked(&mut registrations, value, &mut false);
}

/// Register or reuse a non-Hold string-table entry. The C++ parser uses this
/// `Shift(Ref)` path for string-valued static constants: equal prefix text
/// reuses the first native identity, but the parser does not grant it Hold.
pub fn register_c4_referenced_string(
    registrations: &StringRegistrationLedger,
    value: &str,
) -> C4StringValue {
    let mut registrations = registrations.borrow_mut();
    registrations.retain_live_entries();
    for existing in &mut registrations.entries {
        let Some(existing_value) = existing.upgrade() else {
            continue;
        };
        if c4_string_table_values_equal(&existing_value, value) {
            existing.untouched_loaded = None;
            return existing_value;
        }
    }
    let value = C4StringValue::new(value.to_owned());
    registrations.push_entry(StringRegistration {
        value: value.downgrade(),
        held: None,
        untouched_loaded: None,
    });
    value
}

/// Register a parser-owned string. `C4AulParse` reuses the first equal table
/// entry and sets its `Hold` flag rather than constructing a duplicate.
pub fn register_c4_literal_string(
    registrations: &StringRegistrationLedger,
    value: &str,
) -> C4StringValue {
    let mut registrations = registrations.borrow_mut();
    if let Some(existing) = registrations.literal_lookup(value) {
        return existing;
    }
    registrations.retain_live_entries();
    for existing in &mut registrations.entries {
        let Some(existing_value) = existing.upgrade() else {
            continue;
        };
        if c4_string_table_values_equal(&existing_value, value) {
            existing.held = Some(existing_value.clone());
            existing.untouched_loaded = None;
            registrations
                .literal_cache
                .insert(value.to_owned(), existing_value.downgrade());
            return existing_value;
        }
    }
    let cache_key = value.to_owned();
    let value = C4StringValue::new(cache_key.clone());
    registrations
        .literal_cache
        .insert(cache_key, value.downgrade());
    registrations.push_entry(StringRegistration {
        value: value.downgrade(),
        held: Some(value.clone()),
        untouched_loaded: None,
    });
    value
}

/// Merge one `Strings.txt` line exactly as `C4StringTable::Load`: equal text
/// reuses the existing registration and a repeated line overwrites its old ID.
pub fn register_loaded_c4_string(
    registrations: &StringRegistrationLedger,
    enum_id: i32,
    value: &str,
) {
    let mut registrations = registrations.borrow_mut();
    registrations.retain_live_entries();
    for existing in &mut registrations.entries {
        let Some(existing_value) = existing.upgrade() else {
            continue;
        };
        if c4_string_table_values_equal(&existing_value, value) {
            existing_value.set_enum_id(enum_id);
            return;
        }
    }
    let value = C4StringValue::loaded(value.to_owned(), enum_id);
    registrations.push_entry(StringRegistration {
        value: value.downgrade(),
        held: None,
        untouched_loaded: Some(value),
    });
}

/// Resolve the first live C4String with this current enumeration ID, exactly
/// like `C4StringTable::FindString(int)`. Claiming an untouched loaded string
/// consumes its special refcount-zero root; after the returned handle's final
/// drop the table entry disappears unless the parser also marked it Hold.
pub fn resolve_c4_string(
    registrations: &StringRegistrationLedger,
    enum_id: i32,
) -> Option<C4StringValue> {
    let mut registrations = registrations.borrow_mut();
    registrations.retain_live_entries();
    for entry in &mut registrations.entries {
        let Some(value) = entry.upgrade() else {
            continue;
        };
        if value.enum_id() == enum_id {
            entry.untouched_loaded = None;
            return Some(value);
        }
    }
    None
}

/// Snapshot current table order for diagnostics and compatibility callers.
pub fn c4_string_registration_order(registrations: &StringRegistrationLedger) -> Vec<String> {
    registrations
        .borrow()
        .entries
        .iter()
        .filter_map(StringRegistration::upgrade)
        .map(C4StringValue::into_string)
        .collect()
}

/// Bytes emitted by `C4StringTable::Save` *without* enumerating first.
///
/// This is the native section-switch boundary: stale/non-negative enum IDs
/// decide eligibility, while output still follows linked-list registration
/// order. Dead runtime registrations do not participate in `FindSaveString`.
pub fn save_current_c4_string_enumeration(
    registrations: &StringRegistrationLedger,
    referenced: &[C4StringValue],
) -> Vec<Vec<u8>> {
    for value in referenced {
        register_c4_string(registrations, value);
    }
    let mut registrations = registrations.borrow_mut();
    let mut first_live = Vec::<Vec<u8>>::new();
    let mut saved = Vec::new();
    for entry in &registrations.entries {
        if !entry.enum_eligible() {
            continue;
        }
        let Some(value) = entry.upgrade() else {
            continue;
        };
        let bytes = crate::value::c4_string_bytes(&value);
        if first_live
            .iter()
            .any(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
        {
            continue;
        }
        first_live.push(bytes.clone());
        if value.enum_id() >= 0 {
            saved.push(bytes);
        }
    }
    registrations.retain_live_entries();
    saved
}

/// Execute `C4StringTable::EnumStrings` and return values in their assigned-ID
/// order. Unreferenced non-held runtime registrations are removed, so a later
/// reconstruction of the same text appends after registrations that survived
/// the death boundary.
pub fn enumerate_c4_strings(
    registrations: &StringRegistrationLedger,
    referenced: &[C4StringValue],
) -> Vec<Vec<u8>> {
    // Every live C4Value string necessarily owns a registration. Values may
    // enter Rust engine state without passing through the VM, so recover such
    // registrations at this same enumeration boundary in traversal order.
    for value in referenced {
        register_c4_string(registrations, value);
    }

    let mut registrations = registrations.borrow_mut();
    let mut values = Vec::<Vec<u8>>::new();
    for entry in &registrations.entries {
        if !entry.enum_eligible() {
            if let Some(value) = entry.upgrade() {
                value.set_enum_id(-1);
            }
            continue;
        }
        let Some(value) = entry.upgrade() else {
            continue;
        };
        let bytes = crate::value::c4_string_bytes(&value);
        let enum_id = if let Some(index) = values
            .iter()
            .position(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
        {
            i32::try_from(index).unwrap_or(i32::MAX)
        } else {
            let enum_id = i32::try_from(values.len()).unwrap_or(i32::MAX);
            values.push(bytes);
            enum_id
        };
        value.set_enum_id(enum_id);
    }
    registrations.retain_live_entries();
    values
}

/// Register every string owned by a C4Value in construction/traversal order.
/// The VM invokes this as expressions materialize; embedders also use it for
/// values entering synchronized state without passing through the VM.
pub fn register_c4_value_strings(registrations: &StringRegistrationLedger, value: &Value) {
    if !matches!(
        value,
        Value::String(_) | Value::Array(_) | Value::Proplist(_)
    ) {
        return;
    }

    fn register_value(
        registrations: &mut StringRegistrationLedgerState,
        value: &Value,
        pruned: &mut bool,
    ) {
        match value {
            Value::String(value) => register_c4_string_locked(registrations, value, pruned),
            Value::Array(values) => {
                for value in values {
                    register_value(registrations, value, pruned);
                }
            }
            Value::Proplist(values) => {
                for (key, value) in values {
                    register_value(registrations, key, pruned);
                    register_value(registrations, value, pruned);
                }
                for value in values.hidden_values() {
                    register_value(registrations, value, pruned);
                }
            }
            Value::Int(_)
            | Value::Bool(_)
            | Value::RawBool(_)
            | Value::C4Id(_)
            | Value::Object(_)
            | Value::Nil => {}
        }
    }

    let mut registrations = registrations.borrow_mut();
    let mut pruned = false;
    register_value(&mut registrations, value, &mut pruned);
}

pub fn new_global_slots() -> GlobalSlots {
    std::rc::Rc::new(std::cell::RefCell::new(std::collections::BTreeMap::new()))
}

/// A named `static const` initializer that was not present in the engine's
/// constant table when C4Aul's preparser reached it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "constant value expected for static const `{declaration}`, but found unknown `{initializer}`"
)]
pub struct StaticConstLinkError {
    declaration: String,
    initializer: String,
}

impl StaticConstLinkError {
    pub fn declaration(&self) -> &str {
        &self.declaration
    }

    pub fn initializer(&self) -> &str {
        &self.initializer
    }
}

/// Registers a script's `static` and `static const` declarations in the
/// engine-global tables used by every script host.
pub fn register_global_declarations(
    var_decls: &[VarDecl],
    table: &GlobalVariables,
    globals_consts: Option<&GlobalVariables>,
) -> Result<(), StaticConstLinkError> {
    register_global_declarations_inner(var_decls, table, globals_consts, None, None)
}

/// Register globals while binding string constants into the exact shared
/// C4StringTable identity used by this script engine.
pub fn register_global_declarations_with_strings(
    var_decls: &[VarDecl],
    table: &GlobalVariables,
    globals_consts: Option<&GlobalVariables>,
    strings: &StringRegistrationLedger,
) -> Result<(), StaticConstLinkError> {
    register_global_declarations_inner(var_decls, table, globals_consts, Some(strings), None)
}

fn register_global_declarations_inner(
    var_decls: &[VarDecl],
    table: &GlobalVariables,
    globals_consts: Option<&GlobalVariables>,
    strings: Option<&StringRegistrationLedger>,
    engine_constants: Option<&FxHashMap<String, Value>>,
) -> Result<(), StaticConstLinkError> {
    // Legacy callers may use one fallback table for both mutable statics and
    // constants. Track declarations as this pass encounters them so lookup
    // retains Parse_Const's left-to-right ordering.
    let mut fallback_constants = std::collections::HashSet::new();
    let mut fallback_mutables = std::collections::HashSet::new();
    let mut group_failed = false;
    let mut first_error = None;
    for var_decl in var_decls {
        if var_decl.starts_declaration_group {
            group_failed = false;
        }
        if group_failed {
            continue;
        }
        match var_decl.kind {
            crate::ast::VarDeclKind::Static => {
                let was_present = table.borrow().contains_key(&var_decl.name);
                table
                    .borrow_mut()
                    .entry(var_decl.name.clone())
                    .or_insert_with(|| crate::vm::value_cell(Value::Nil));
                if globals_consts.is_none() && !was_present {
                    fallback_mutables.insert(var_decl.name.clone());
                }
            }
            crate::ast::VarDeclKind::StaticConst => {
                // The dedicated preparser grammar admits only these shapes.
                // Keep this match exhaustive so an AST regression cannot
                // silently turn a malformed initializer into nil.
                let value = match &var_decl.init {
                    Some(crate::ast::Expr::Literal(crate::value::Literal::String(value))) => {
                        Value::String(match strings {
                            Some(strings) => register_c4_referenced_string(strings, value),
                            None => C4StringValue::from(value.clone()),
                        })
                    }
                    Some(crate::ast::Expr::Literal(literal)) => Value::from(literal.clone()),
                    Some(crate::ast::Expr::Unary(crate::ast::UnaryOp::Negate, expression))
                        if matches!(
                            expression.as_ref(),
                            crate::ast::Expr::Literal(crate::value::Literal::Int(_))
                        ) =>
                    {
                        let crate::ast::Expr::Literal(crate::value::Literal::Int(value)) =
                            expression.as_ref()
                        else {
                            unreachable!("guarded static-constant integer shape")
                        };
                        Value::Int(value.wrapping_neg())
                    }
                    Some(crate::ast::Expr::Variable(name)) => {
                        let cell = match globals_consts {
                            Some(constants) => constants.borrow().get(name).cloned(),
                            None if fallback_constants.contains(name) => {
                                table.borrow().get(name).cloned()
                            }
                            None if fallback_mutables.contains(name.as_str()) => None,
                            None => table.borrow().get(name).cloned(),
                        };
                        let value = cell.map(|cell| cell.borrow().clone()).or_else(|| {
                            engine_constants.and_then(|values| values.get(name).cloned())
                        });
                        let Some(value) = value else {
                            // Parse_Const aborts this comma-delimited group.
                            // Parse_Script recovery can then resume at a later
                            // top-level declaration, while constants already
                            // registered before the error stay live.
                            let error = StaticConstLinkError {
                                declaration: var_decl.name.clone(),
                                initializer: name.clone(),
                            };
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                            group_failed = true;
                            continue;
                        };
                        value
                    }
                    Some(initializer) => unreachable!(
                        "static-constant parser produced unsupported initializer: {initializer:?}"
                    ),
                    None => unreachable!("static constant without an initializer"),
                };
                // RegisterGlobalConstant overwrites an existing value
                // (C4Aul.cpp:484-492). Keep the existing shared cell so
                // every already-linked script observes the replacement.
                // Constants live only in GlobalConsts, never GlobalNamed;
                // that keeps them out of the VM's assignable lvalue table.
                let constants = globals_consts.unwrap_or(table);
                let cell = constants
                    .borrow_mut()
                    .entry(var_decl.name.clone())
                    .or_insert_with(|| crate::vm::value_cell(Value::Nil))
                    .clone();
                *cell.borrow_mut() = value;
                if globals_consts.is_none() {
                    fallback_constants.insert(var_decl.name.clone());
                    fallback_mutables.remove(&var_decl.name);
                }
            }
            crate::ast::VarDeclKind::Local => {}
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn latest_function_with_access(function: &Function, global: bool) -> Option<&Function> {
    std::iter::successors(Some(function), |function| function.overloaded.as_deref())
        .find(|function| (function.access == crate::ast::AccessLevel::Global) == global)
}

/// Append every physical function-list node at `FuncL`. Duplicate names stay
/// in the ledger because their position determines which overload is visible
/// when `GetSFunc` later walks backward.
fn append_function_order(destination: &mut Vec<String>, source: &[String]) {
    destination.extend_from_slice(source);
}

/// Include copies are inserted at `Func0` in full physical source order.
fn prepend_function_order(destination: &mut Vec<String>, source: &[String]) {
    let mut imported = source.to_vec();
    imported.append(destination);
    *destination = imported;
}

/// Return the visible node for each exact name in physical `Func0 -> FuncL`
/// order. The backward first-win pass mirrors `!OverloadedBy`; reversing its
/// result restores the order needed while constructing overload tables.
fn visible_function_names_in_physical_order(order: &[String]) -> Vec<&String> {
    let mut seen = HashSet::new();
    let mut visible = order
        .iter()
        .rev()
        .filter(|name| seen.insert(name.as_str()))
        .collect::<Vec<_>>();
    visible.reverse();
    visible
}

#[derive(Clone, Default)]
pub struct Script {
    functions: FxHashMap<String, Function>,
    /// Every named script-function node in C4Aul's physical `Func0 -> FuncL`
    /// order, including same-name overloaded nodes. Global declarations are
    /// omitted because their declaring host keeps only an unnamed `FnLink`,
    /// whose `SFunc()` is null.
    local_function_order: Vec<String>,
    /// `global func` declarations in their physical engine-list insertion
    /// order. The higher-level linker uses this order when it builds the
    /// shared Game.ScriptEngine overload table.
    global_function_order: Vec<String>,
    includes: Vec<String>,
    appends: Vec<crate::ast::AppendTo>,
    strict_level: Option<u8>,
    var_decls: Vec<VarDecl>, // Script-level variable declarations
    string_literals: Vec<String>,
    parse_diagnostics: Vec<ParseError>,
}

impl Script {
    pub fn compile(source: &str) -> Result<Self, ParseError> {
        let mut parser = Parser::new(source);
        let (ast, diagnostics) = parser.parse_script_recovering();
        Ok(Self::from_ast(ast, diagnostics))
    }

    /// Compile a System/global script whose legacy old-style functions have
    /// no definition owner. C++ reports `local` declarations during preparse
    /// without poisoning the retained function's later parser pass.
    pub fn compile_global(source: &str) -> Result<Self, ParseError> {
        let mut parser = Parser::new_global_script(source);
        let (ast, diagnostics) = parser.parse_script_recovering();
        Ok(Self::from_ast(ast, diagnostics))
    }

    #[doc(hidden)]
    pub fn compile_c4_string(source: &str) -> Result<Self, ParseError> {
        let mut parser = Parser::with_strict_level_c4_string(source, None);
        let (ast, diagnostics) = parser.parse_script_recovering();
        Ok(Self::from_ast(ast, diagnostics))
    }

    /// Compile a byte-projected System/global script while retaining the
    /// ownerless parsing rules used by C4Aul's global script table.
    #[doc(hidden)]
    pub fn compile_global_c4_string(source: &str) -> Result<Self, ParseError> {
        let mut parser = Parser::new_global_script_c4_string(source);
        let (ast, diagnostics) = parser.parse_script_recovering();
        Ok(Self::from_ast(ast, diagnostics))
    }

    fn from_ast(ast: AstScript, parse_diagnostics: Vec<ParseError>) -> Self {
        let mut functions: FxHashMap<String, Function> = FxHashMap::default();
        let mut local_function_order = Vec::new();
        let mut global_function_order = Vec::new();
        for mut function in ast.functions {
            // Each function carries its owning script's #strict level so the VM
            // can apply level-correct `==`/`!=` (C++ uses Fn->pOrgScript->Strict).
            function.strict_level = ast.strict_level;
            // A redefinition in the SAME script keeps the earlier definition
            // as its `inherited` target (`Fn->OwnerOverloaded =
            // Fn->Owner->GetOverloadedFunc(Fn)`, C4AulParse.cpp:1404-1406) —
            // the Coach.c4d menu-description wrappers forward through it.
            let order = if function.access == crate::ast::AccessLevel::Global {
                &mut global_function_order
            } else {
                &mut local_function_order
            };
            order.push(function.name.clone());
            if let Some(previous) = functions.remove(&function.name) {
                function.push_overload(previous);
            }
            functions.insert(function.name.clone(), function);
        }
        Self {
            functions,
            local_function_order,
            global_function_order,
            includes: ast.includes,
            appends: ast.appends,
            strict_level: ast.strict_level,
            var_decls: ast.var_decls,
            string_literals: ast.string_literals,
            parse_diagnostics,
        }
    }

    pub fn functions(&self) -> &FxHashMap<String, Function> {
        &self.functions
    }

    pub fn global_access_functions(&self) -> impl Iterator<Item = (&String, &Function)> {
        visible_function_names_in_physical_order(&self.global_function_order)
            .into_iter()
            .filter_map(|name| {
                latest_function_with_access(self.functions.get(name)?, true)
                    .map(|function| (name, function))
            })
    }

    pub fn includes(&self) -> &[String] {
        &self.includes
    }

    pub fn appends(&self) -> &[crate::ast::AppendTo] {
        &self.appends
    }

    pub fn strict_level(&self) -> Option<u8> {
        self.strict_level
    }

    pub fn var_decls(&self) -> &[crate::ast::VarDecl] {
        &self.var_decls
    }

    /// Warnings and errors retained during C4Aul-style parsing. Nonfatal
    /// warnings leave their function executable; recovered errors add an
    /// AB_ERR analogue that raises if execution reaches the bad suffix.
    pub fn parse_diagnostics(&self) -> &[ParseError] {
        &self.parse_diagnostics
    }

    /// Returns the script body used by C4AulScript::AppendTo after global
    /// declarations have already been registered on the script engine.
    /// AppendTo copies LocalNamed only (C4AulLink.cpp:145-157); `static` and
    /// `static const` must never become locals of the target definition.
    pub fn without_static_declarations(mut self) -> Self {
        self.var_decls
            .retain(|var_decl| var_decl.kind == crate::ast::VarDeclKind::Local);
        self
    }
}

#[derive(Clone)]
pub struct Engine {
    functions: FxHashMap<String, Function>,
    /// Every named script-function node in physical `Func0 -> FuncL` order.
    /// `GetSFunc(index)` enumerates this ledger backward and skips nodes with
    /// a same-name successor (`OverloadedBy`).
    local_function_order: Vec<String>,
    /// This host's `global func` declarations in physical insertion order.
    /// They live on Game.ScriptEngine in C++, while this host retains unnamed
    /// links for local lookup provenance.
    global_function_order: Vec<String>,
    /// Stable identity of this C4AulScript destination host. It survives
    /// Rust moves and copy-on-write Engine clones so global Function
    /// `LinkedTo` provenance never depends on a HashMap's address.
    host_identity: crate::vm::ScriptHostIdentity,
    /// Native `C4AulScript::ScriptName`, used by function source diagnostics
    /// and temporary DirectExec contexts (`<context> in <ScriptName>`).
    script_name: Option<String>,
    /// Destination `C4Def::Name` for objectless local-function frames.
    definition_name: Option<String>,
    /// `Game.Script::ScriptName`, which is the receiver selected by C++ when
    /// AB_CALLGLOBAL clears Obj/Def before a native `eval` call.
    game_script_name: Option<String>,
    /// Whether a callerless ordinary frame retains a C4Aul `Def` context.
    /// Definition hosts do; Game.Script and Game.ScriptEngine do not.
    definition_context: bool,
    /// Strictness of this C4AulScript host itself. Linked include/append
    /// function copies keep their source strictness for expression semantics,
    /// but native calls inspect `Func->Owner->Strict` (the destination host).
    /// The outer Option distinguishes an uninitialized bare Engine from a
    /// deliberately NONSTRICT base script.
    owner_strict_level: Option<Option<u8>>,
    host_functions: Arc<FxHashMap<String, RegisteredHostFunction>>,
    host_reference_functions: Arc<FxHashMap<String, HostReferenceFunction>>,
    /// Exact C++ `GetParType()` vectors for native registrations. An absent
    /// entry keeps the public embedding API variadic; game natives always
    /// install a vector, whose length is also their declared arity.
    host_function_parameter_types: Arc<FxHashMap<String, Arc<[C4VType]>>>,
    debugger_hooks: Option<DebuggerHooks>,
    var_decls: Vec<VarDecl>, // Script-level variable declarations (local variables)
    /// Engine script constants (RegisterGlobalConstant, C4Script.cpp:6581),
    /// consulted by the VM when an identifier matches no variable.
    constants: Arc<FxHashMap<String, Value>>,
    /// Engine-global script functions (System.c4g global funcs, owned by
    /// Game.ScriptEngine in C++): shared across every script host, resolved
    /// after the own script and before host functions.
    global_functions: Option<Arc<FxHashMap<String, Function>>>,
    /// `obj->Method(args)` cross-object resolver (AB_CALL,
    /// C4AulExec.cpp:1216-1305): the VM is world-agnostic, so the engine
    /// registers this hook to run the function on the TARGET object's
    /// script. Called with [target, name, failsafe, args...].
    method_dispatch: Option<HostFunction>,
    /// Reference-preserving method resolver for arrow calls in lvalue
    /// position. Arguments use the same [target, name, failsafe, args...]
    /// layout as `method_dispatch`.
    method_reference_dispatch: Option<MethodReferenceDispatch>,
    /// Method resolver for an arrow call whose callee takes `&` parameters.
    /// Same [target, name, failsafe, args...] layout as `method_dispatch`.
    method_ref_args_dispatch: Option<MethodRefArgsDispatch>,
    /// Engine-wide `&`-parameter lookup used when an arrow call's callee is
    /// not resolvable in this host (see [`ReferenceParameterProbe`]).
    reference_parameter_probe: Option<ReferenceParameterProbe>,
    /// Engine-wide name lookup deciding whether a failsafe arrow call emitted
    /// AB_CALLFS at link time (see [`DirectCallFunctionProbe`]).
    direct_call_function_probe: Option<DirectCallFunctionProbe>,
    /// Embedding-engine context switch for AB_CALLGLOBAL's null Obj/Def.
    global_call_context_hook: Option<GlobalCallContextHook>,
    /// Embedding-engine receiver selection and DirectExec for FnEval.
    eval_direct_exec_hook: Option<EvalDirectExecHook>,
    /// The shared `static` table; `None` keeps the legacy per-host
    /// fallback (fixtures without an engine).
    globals_named: Option<GlobalVariables>,
    /// The shared numbered `Global(index)` table.
    globals_numbered: Option<GlobalSlots>,
    /// The shared `static const` registry (C4AulScriptEngine's global
    /// constants, RegisterGlobalConstant C4Aul.cpp:484): script-declared
    /// constants every host sees. Cells are SHARED with `globals_named`
    /// so identifier reads and old-style constant calls agree.
    globals_consts: Option<GlobalVariables>,
    /// Cross-object LocalN cell supplier (see [`LocalCellHook`]).
    local_cell_hook: Option<LocalCellHook>,
    /// World-liveness check shared by ordinary arrow dispatch and the VM's
    /// Local/LocalN/SetLocal fast paths.
    object_target_availability_probe: Option<ObjectTargetAvailabilityProbe>,
    /// Shared engine-global C4StringTable registration ledger.
    string_registrations: Option<StringRegistrations>,
    /// Literals retained by scripts already installed in this host. This lets
    /// a late-attached game ledger recover the native link order.
    string_literals: Vec<String>,
    /// Deferred preparser failures for named static-constant initializers.
    static_const_link_errors: Vec<StaticConstLinkError>,
}

/// A hard `inherited(...)` left without an overload target once linking
/// finished. C4Aul reports the equivalent at load time and leaves the function
/// raising when called (`C4AulParse.cpp:2799`, `:3563-3586`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedInherited {
    /// Name of the function whose body holds the call.
    pub function: String,
    /// Declaring script, when the host knows one.
    pub script_name: Option<String>,
    /// One-based source line of the `inherited` call.
    pub line: usize,
}

impl std::fmt::Display for UnresolvedInherited {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // C4Aul's own wording (C4AulParse.cpp:2799), with the location C4Aul
        // prints through C4AulParseError's script/line context.
        write!(
            formatter,
            "inherited function not found, use _inherited to call failsafe (in {}",
            self.function
        )?;
        if let Some(script_name) = &self.script_name {
            write!(formatter, ", {script_name}")?;
        }
        write!(formatter, ":{})", self.line)
    }
}

/// Ownership scope of a resolved script function. A global function is
/// owned by the script engine even when its local FnLink lives on a
/// definition host. Ownership alone does not determine execution `Obj` or
/// `Def`: native callers may still supply a command object as `this`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptFunctionScope {
    Local,
    Global,
}

/// The function selected by C4Aul's caller-local lookup, including the
/// destination host that owns its named link. Engine-global functions live
/// in one shared table, but retain their declaring `LinkedTo` host so native
/// code can pin the exact script used by a deferred callback.
#[derive(Clone, Debug)]
pub struct ScriptFunctionResolution {
    pub scope: ScriptFunctionScope,
    pub host_identity: crate::vm::ScriptHostIdentity,
    /// Immutable queue-time function body and overload provenance.
    pub function: Arc<Function>,
    /// Proves `function` is still the immutable snapshot minted by this
    /// engine. `Arc::make_mut` dissociates this weak pointer, so a caller that
    /// mutates the public snapshot automatically falls back to validation.
    trusted_snapshot: Weak<Function>,
}

impl ScriptFunctionResolution {
    fn new(
        scope: ScriptFunctionScope,
        host_identity: crate::vm::ScriptHostIdentity,
        function: &Function,
    ) -> Self {
        let function = function.resolved_snapshot();
        let trusted_snapshot = Arc::downgrade(&function);
        Self {
            scope,
            host_identity,
            function,
            trusted_snapshot,
        }
    }

    pub(crate) fn has_trusted_snapshot(&self) -> bool {
        self.trusted_snapshot
            .upgrade()
            .is_some_and(|trusted| Arc::ptr_eq(&trusted, &self.function))
    }
}

impl PartialEq for ScriptFunctionResolution {
    fn eq(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.host_identity == other.host_identity
            && self.function == other.function
    }
}

impl Engine {
    pub fn new() -> Self {
        let empty_registrations = empty_host_registration_snapshot();
        Self {
            functions: FxHashMap::default(),
            local_function_order: Vec::new(),
            global_function_order: Vec::new(),
            host_identity: crate::vm::ScriptHostIdentity::fresh(),
            script_name: None,
            definition_name: None,
            game_script_name: None,
            definition_context: false,
            owner_strict_level: None,
            host_functions: Arc::clone(&empty_registrations.host_functions),
            host_reference_functions: Arc::clone(&empty_registrations.host_reference_functions),
            host_function_parameter_types: Arc::clone(
                &empty_registrations.host_function_parameter_types,
            ),
            debugger_hooks: None,
            var_decls: Vec::new(),
            constants: Arc::clone(&empty_registrations.constants),
            global_functions: None,
            method_dispatch: None,
            method_reference_dispatch: None,
            method_ref_args_dispatch: None,
            reference_parameter_probe: None,
            direct_call_function_probe: None,
            global_call_context_hook: None,
            eval_direct_exec_hook: None,
            globals_named: None,
            globals_numbered: Some(new_global_slots()),
            globals_consts: None,
            local_cell_hook: None,
            object_target_availability_probe: None,
            // Even standalone clonk-script engines own one native string table.
            // Embedders may replace it with their game-global shared ledger.
            string_registrations: Some(new_string_registrations()),
            string_literals: Vec::new(),
            static_const_link_errors: Vec::new(),
        }
    }

    /// Named static-constant references that failed during declaration
    /// registration. C++ reports these from its preparser and continues
    /// loading the remaining script hosts.
    pub fn static_const_link_errors(&self) -> &[StaticConstLinkError] {
        &self.static_const_link_errors
    }

    /// Process-local identity of this destination script host. Native
    /// compatibility code uses it to match a caller's local-lookup host
    /// (`Func->Owner` for local functions, `Func->LinkedTo` for globals)
    /// back to the exact retained engine without consulting the object def.
    pub fn host_identity(&self) -> crate::vm::ScriptHostIdentity {
        self.host_identity
    }

    /// Assign the native script-host label used for runtime diagnostics.
    /// Existing functions are updated as well so embedders may name a host
    /// before or after installing its first parsed script.
    pub fn set_script_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        for function in self.functions.values_mut() {
            function.rebind_source_name_for_host(self.host_identity, &name);
        }
        self.script_name = Some(name);
    }

    pub fn script_name(&self) -> &str {
        self.script_name.as_deref().unwrap_or("")
    }

    /// Assign the destination definition label used by objectless local
    /// function frames. Scenario and engine-global hosts leave this unset.
    pub fn set_definition_name(&mut self, name: impl Into<String>) {
        self.definition_name = Some(name.into());
    }

    pub fn set_game_script_name(&mut self, script_name: impl Into<String>) {
        self.game_script_name = Some(script_name.into());
    }

    pub fn set_definition_context(&mut self, definition_context: bool) {
        self.definition_context = definition_context;
    }

    /// Installs the engine-global script function table (System.c4g
    /// global funcs). Shared by Arc so every definition script host sees
    /// the same copy.
    pub fn set_global_functions(&mut self, functions: Option<Arc<FxHashMap<String, Function>>>) {
        self.global_functions = functions;
    }

    /// Whether the global table knows `name`.
    pub fn has_global_function(&self, name: &str) -> bool {
        self.global_functions
            .as_ref()
            .map(|functions| functions.contains_key(name))
            .unwrap_or(false)
    }

    /// Captures this host's native registration surface for cheap reuse by
    /// other script hosts. Script functions, globals, hooks, and host identity
    /// are deliberately not part of the snapshot.
    pub fn host_registration_snapshot(&self) -> HostRegistrationSnapshot {
        assert!(
            !self.constants.values().any(value_contains_c4_string),
            "host registration snapshots cannot share C4String constants"
        );
        HostRegistrationSnapshot {
            host_functions: Arc::clone(&self.host_functions),
            host_reference_functions: Arc::clone(&self.host_reference_functions),
            host_function_parameter_types: Arc::clone(&self.host_function_parameter_types),
            constants: Arc::clone(&self.constants),
        }
    }

    /// Installs a captured native surface with the same overwrite semantics as
    /// replaying its registrations. Empty hosts take the O(1) shared-map path;
    /// nonempty hosts retain unrelated embedding callbacks and constants.
    pub fn apply_host_registration_snapshot(&mut self, snapshot: &HostRegistrationSnapshot) {
        let registrations_already_shared =
            Arc::ptr_eq(&self.host_functions, &snapshot.host_functions)
                && Arc::ptr_eq(
                    &self.host_reference_functions,
                    &snapshot.host_reference_functions,
                )
                && Arc::ptr_eq(
                    &self.host_function_parameter_types,
                    &snapshot.host_function_parameter_types,
                );
        if !registrations_already_shared {
            if self.host_functions.is_empty()
                && self.host_reference_functions.is_empty()
                && self.host_function_parameter_types.is_empty()
            {
                self.host_functions = Arc::clone(&snapshot.host_functions);
                self.host_reference_functions = Arc::clone(&snapshot.host_reference_functions);
                self.host_function_parameter_types =
                    Arc::clone(&snapshot.host_function_parameter_types);
            } else {
                let host_functions = Arc::make_mut(&mut self.host_functions);
                let host_reference_functions = Arc::make_mut(&mut self.host_reference_functions);
                let host_function_parameter_types =
                    Arc::make_mut(&mut self.host_function_parameter_types);

                for name in snapshot.host_functions.keys() {
                    host_reference_functions.remove(name);
                    host_function_parameter_types.remove(name);
                }
                for name in snapshot.host_reference_functions.keys() {
                    host_functions.remove(name);
                    host_function_parameter_types.remove(name);
                }
                host_functions.extend(
                    snapshot
                        .host_functions
                        .iter()
                        .map(|(name, function)| (name.clone(), function.clone())),
                );
                host_reference_functions.extend(
                    snapshot
                        .host_reference_functions
                        .iter()
                        .map(|(name, function)| (name.clone(), function.clone())),
                );
                host_function_parameter_types.extend(
                    snapshot
                        .host_function_parameter_types
                        .iter()
                        .map(|(name, parameter_types)| (name.clone(), Arc::clone(parameter_types))),
                );
            }
        }

        if !Arc::ptr_eq(&self.constants, &snapshot.constants) {
            if self.constants.is_empty() {
                self.constants = Arc::clone(&snapshot.constants);
            } else {
                Arc::make_mut(&mut self.constants).extend(
                    snapshot
                        .constants
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone())),
                );
            }
        }
    }

    /// Registers an engine script constant (RegisterGlobalConstant,
    /// C4Script.cpp:6581): identifiers resolve to it when no variable
    /// matches; variables shadow constants.
    pub fn register_constant(&mut self, name: impl Into<String>, value: Value) {
        if let Some(strings) = self.string_registrations.as_deref() {
            register_c4_value_strings(strings, &value);
        }
        Arc::make_mut(&mut self.constants).insert(name.into(), value);
    }

    pub fn load_script(&mut self, source: &str) -> Result<(), ScriptError> {
        let script = Script::compile(source)?;
        self.add_script(script);
        Ok(())
    }

    pub fn add_script(&mut self, mut script: Script) {
        // C4Aul's preparse pass registers every static declaration before the
        // later global Parse pass marks function-body strings Hold. Preserve
        // that construction order even for this standalone immediate-link
        // path; it determines C4StringTable enumeration IDs.
        let registration = self.globals_named.as_ref().map(|table| {
            register_global_declarations_inner(
                &script.var_decls,
                table,
                self.globals_consts.as_ref(),
                self.string_registrations.as_deref(),
                Some(&self.constants),
            )
        });
        if let Some(Err(error)) = registration {
            self.static_const_link_errors.push(error);
        }
        if let Some(registrations) = &self.string_registrations {
            for literal in &script.string_literals {
                register_c4_literal_string(registrations, literal);
            }
        }
        self.string_literals
            .extend(script.string_literals.iter().cloned());
        for function in script.functions.values_mut() {
            function.bind_source_host(self.host_identity);
            if let Some(script_name) = &self.script_name {
                function.bind_source_name(script_name);
            }
            function.bind_global_link_host(self.host_identity);
        }
        if self.owner_strict_level.is_none() {
            self.owner_strict_level = Some(script.strict_level);
        }
        append_function_order(&mut self.local_function_order, &script.local_function_order);
        append_function_order(
            &mut self.global_function_order,
            &script.global_function_order,
        );
        for (name, mut function) in script.functions.into_iter() {
            // A redefinition overloads the earlier function: `inherited`
            // reaches it (C++ Fn->OwnerOverloaded).
            if let Some(previous) = self.functions.remove(&name) {
                function.push_overload(previous);
            }
            self.functions.insert(name, function);
        }
        // Store variable declarations from the script. `static` names are
        // ENGINE-GLOBAL (GlobalNamed) when the shared table is attached:
        // they register there (keeping any existing value — statics
        // persist across script loads) and never become per-object locals.
        for var_decl in script.var_decls {
            if self.globals_named.is_some() && var_decl.kind != crate::ast::VarDeclKind::Local {
                continue;
            }
            self.var_decls.push(var_decl);
        }
    }

    /// Replaces this host's parsed script while retaining all host-side
    /// configuration (native functions, debugger hooks, dispatch hooks, and
    /// shared global tables). This is the unlink/reparse primitive used by a
    /// higher-level relink pass: linked include/append overloads disappear
    /// because only the pristine replacement script is installed.
    ///
    /// When `register_declarations` is true, `static` and `static const`
    /// declarations are registered once in the attached engine-global
    /// tables. With a global table attached they never become object-local
    /// declarations, even when registration is skipped because a relink is
    /// rebuilding an otherwise unchanged host.
    pub fn replace_script(&mut self, script: Script, register_declarations: bool) {
        self.replace_script_inner(script, register_declarations, true);
    }

    /// Restore a preparsed host without acquiring its function-body string
    /// Holds yet. `C4AulScriptEngine::ReLink` resets every host first, resolves
    /// appends/includes, and only then runs the global Parse pass.
    pub fn replace_script_deferred(&mut self, script: Script, register_declarations: bool) {
        self.replace_script_inner(script, register_declarations, false);
    }

    fn replace_script_inner(
        &mut self,
        mut script: Script,
        register_declarations: bool,
        acquire_string_holds: bool,
    ) {
        self.string_literals.clone_from(&script.string_literals);
        for function in script.functions.values_mut() {
            function.bind_source_host(self.host_identity);
            if let Some(script_name) = &self.script_name {
                function.bind_source_name(script_name);
            }
            function.bind_global_link_host(self.host_identity);
        }
        self.owner_strict_level = Some(script.strict_level);
        self.functions.clear();
        self.local_function_order
            .clone_from(&script.local_function_order);
        self.global_function_order
            .clone_from(&script.global_function_order);
        self.var_decls.clear();
        self.static_const_link_errors.clear();

        if register_declarations {
            let registration = self.globals_named.as_ref().map(|table| {
                register_global_declarations_inner(
                    &script.var_decls,
                    table,
                    self.globals_consts.as_ref(),
                    self.string_registrations.as_deref(),
                    Some(&self.constants),
                )
            });
            if let Some(Err(error)) = registration {
                self.static_const_link_errors.push(error);
            }
        }

        if acquire_string_holds {
            if let Some(registrations) = &self.string_registrations {
                for literal in &script.string_literals {
                    register_c4_literal_string(registrations, literal);
                }
            }
        }

        self.functions.extend(script.functions);
        if self.globals_named.is_some() {
            self.var_decls.extend(
                script
                    .var_decls
                    .into_iter()
                    .filter(|declaration| declaration.kind == crate::ast::VarDeclKind::Local),
            );
        } else {
            // Standalone ScriptEngine fixtures retain the historical
            // per-host fallback when no GlobalNamed table is attached.
            self.var_decls.extend(script.var_decls);
        }
    }

    /// `C4AulScript::AppendTo` with bHighPrio=true (C4AulLink.cpp:114-141,
    /// driven by ResolveAppends :29-64): COPIES `other`'s functions here so
    /// they OVERRIDE same-name functions — the appended function wins and
    /// the original stays reachable as its `inherited` target. Global
    /// functions are skipped (":127 no need to append global funcs").
    /// Script-level variable declarations join too: appended code reads
    /// object locals by name, which must resolve on the target.
    pub fn append_overrides_from(&mut self, other: &Engine) {
        for name in visible_function_names_in_physical_order(&other.local_function_order) {
            let Some(function) = other
                .functions
                .get(name)
                .and_then(|function| latest_function_with_access(function, false))
            else {
                continue;
            };
            let mut function = function.clone();
            if let Some(previous) = self.functions.remove(name) {
                function.push_overload(previous);
            }
            self.functions.insert(name.clone(), function);
        }
        append_function_order(&mut self.local_function_order, &other.local_function_order);
        for var_decl in other.var_decls.iter() {
            if !self.var_decls.iter().any(|v| v.name == var_decl.name) {
                self.var_decls.push(var_decl.clone());
            }
        }
    }

    pub fn merge_from(&mut self, other: &Engine) {
        for name in visible_function_names_in_physical_order(&other.local_function_order) {
            // Includes are AppendTo with bHighPrio=false in C++ — global
            // funcs are never copied (C4AulLink.cpp:127); they stay
            // reachable through the engine table.
            let Some(function) = other
                .functions
                .get(name)
                .and_then(|function| latest_function_with_access(function, false))
            else {
                continue;
            };
            match self.functions.get_mut(name) {
                // Child overrides parent, but the parent's function stays
                // reachable as the child's `inherited` target (C++ include
                // linking sets OwnerOverloaded).
                Some(own) => own.append_include_overload(function.clone()),
                None => {
                    self.functions.insert(name.clone(), function.clone());
                }
            }
        }
        prepend_function_order(&mut self.local_function_order, &other.local_function_order);

        // Merge local variable declarations from parent
        // Child definitions inherit parent's local variables
        for var_decl in other.var_decls.iter() {
            // Only add if not already declared (child overrides parent)
            if !self.var_decls.iter().any(|v| v.name == var_decl.name) {
                self.var_decls.push(var_decl.clone());
            }
        }
    }

    /// The script's `global func` declarations (AA_GLOBAL): C4Aul
    /// registers these at the script ENGINE, not the local host. Enumeration
    /// follows physical declaration order because callers build the engine's
    /// overload chain from oldest to newest.
    pub fn global_access_functions(&self) -> impl Iterator<Item = (&String, &Function)> {
        visible_function_names_in_physical_order(&self.global_function_order)
            .into_iter()
            .filter_map(|name| {
                latest_function_with_access(self.functions.get(name)?, true)
                    .map(|function| (name, function))
            })
    }

    /// Every physical `global func` node declared by this host, from
    /// `Func0` to `FuncL`, including same-name nodes hidden by a later
    /// overload. Embedders concatenate these ledgers in script-link order,
    /// then apply the same backward exact-name dedupe as [`Self::global_functions_in_get_sfunc_order`].
    pub fn global_function_names_in_link_order(&self) -> impl Iterator<Item = &str> {
        self.global_function_order.iter().map(String::as_str)
    }

    /// This host's active global declarations in
    /// `C4AulScript::GetSFunc(index)` order (`FuncL -> Func0`). This is the
    /// per-host contribution to Game.ScriptEngine's context-function rows.
    pub fn global_functions_in_get_sfunc_order(
        &self,
    ) -> impl Iterator<Item = (&String, &Function)> {
        let mut seen = HashSet::new();
        self.global_function_order
            .iter()
            .rev()
            .filter(move |name| seen.insert(name.as_str()))
            .filter_map(|name| {
                latest_function_with_access(self.functions.get(name)?, true)
                    .map(|function| (name, function))
            })
    }

    /// Repoints a declaring script's local global-function link at the
    /// engine-owned function. C4Aul creates both objects for `global func`:
    /// the function lives on the script engine while a `FnLink` remains in
    /// the original script (C4AulParse.cpp:1603-1610). The linked function
    /// carries the engine overload chain used by `inherited()`.
    pub fn link_global_access_function(&mut self, name: &str, mut function: Function) -> bool {
        let Some(local) = self.functions.get(name) else {
            return false;
        };
        if local.access != crate::ast::AccessLevel::Global {
            return false;
        }
        function.reset_compiled_cache();
        self.functions.insert(name.to_string(), function);
        true
    }

    /// Functions whose hard `inherited(...)` has no overload target now that
    /// the func tables are built.
    ///
    /// C4Aul binds `inherited` while parsing bodies, which happens only after
    /// every table exists (`C4AulParse.cpp:1406`, "all func tables are built
    /// now"), and throws `"inherited function not found, use _inherited to
    /// call failsafe"` when `Fn->OwnerOverloaded` is null
    /// (`C4AulParse.cpp:2799`). `C4AulScript::Parse` catches that, reports it
    /// and counts it into `errCnt` (`C4AulParse.cpp:3563-3586`), so an author
    /// learns at load time rather than when the call first executes. The port
    /// parses before linking, so the equivalent check runs here.
    ///
    /// Resolution deliberately mirrors the VM exactly — own-owner list, then
    /// C4Aul's owner hop into the engine table, then the same-name native.
    /// A narrower oracle would report functions that resolve perfectly well.
    pub fn unresolved_inherited_diagnostics(&self) -> Vec<UnresolvedInherited> {
        let mut unresolved = self
            .functions
            .values()
            .filter_map(|function| {
                let line = function.hard_inherited_line?;
                let resolved = function.owner_overloaded().is_some()
                    || self.inherited_engine_hop_exists(function)
                    || self.host_functions.contains_key(&function.name)
                    || self.host_reference_functions.contains_key(&function.name);
                (!resolved).then(|| UnresolvedInherited {
                    function: function.name.clone(),
                    script_name: function
                        .source_name
                        .clone()
                        .or_else(|| self.script_name.clone()),
                    line,
                })
            })
            .collect::<Vec<_>>();
        // `functions` is an FxHashMap, whose iteration order is unspecified.
        // Reporting has to be deterministic: replay and record comparisons in
        // this port have been fed by log text before.
        unresolved
            .sort_by(|left, right| (&left.function, left.line).cmp(&(&right.function, right.line)));
        unresolved
    }

    /// Whether `GetOverloadedFunc`'s owner hop (`C4Aul.cpp:281-288`) finds an
    /// engine-owned target for this function. Engine-owned functions never
    /// hop: the engine has no owner above it.
    fn inherited_engine_hop_exists(&self, function: &Function) -> bool {
        function.access != crate::ast::AccessLevel::Global
            && self
                .global_functions
                .as_deref()
                .and_then(|table| table.get(&function.name))
                .is_some_and(|found| found.access == crate::ast::AccessLevel::Global)
    }

    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    /// Number of installed function nodes, including every recursive
    /// `inherited()` overload behind each root function.
    pub fn linked_function_count(&self) -> usize {
        self.functions
            .values()
            .map(|function| {
                let mut count = 1usize;
                let mut overloaded = function.overloaded.as_deref();
                while let Some(function) = overloaded {
                    count += 1;
                    overloaded = function.overloaded.as_deref();
                }
                count
            })
            .sum()
    }

    /// Final per-object local declarations after includes/appends have been
    /// linked and engine-global statics have been adopted.
    pub fn local_variable_names(&self) -> impl Iterator<Item = &str> {
        self.var_decls
            .iter()
            .filter(|declaration| declaration.kind == crate::ast::VarDeclKind::Local)
            .map(|declaration| declaration.name.as_str())
    }

    pub fn includes(&self) -> Vec<String> {
        // Extract includes from the loaded script
        // Note: This is a simplified version that returns empty since we don't
        // store the original Script object. The actual includes are tracked
        // at a higher level.
        Vec::new()
    }

    pub fn register_host_function<F>(&mut self, name: impl Into<String>, func: F)
    where
        F: Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
    {
        self.register_host_function_erased(name.into(), Arc::new(func), None);
    }

    /// Register a C++-style native host function with its exact declared
    /// parameter count. Script calls evaluate every supplied expression, then
    /// the VM trims surplus values or appends `nil` values to this count.
    pub fn register_host_function_with_arity<F>(
        &mut self,
        name: impl Into<String>,
        parameter_count: usize,
        func: F,
    ) where
        F: Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
    {
        assert!(
            parameter_count <= 10,
            "C4Aul native functions cannot declare more than 10 parameters"
        );
        self.register_host_function_erased(name.into(), Arc::new(func), Some(parameter_count));
    }

    #[inline(never)]
    fn register_host_function_erased(
        &mut self,
        name: String,
        func: HostFunction,
        parameter_count: Option<usize>,
    ) {
        if self.host_reference_functions.contains_key(&name) {
            Arc::make_mut(&mut self.host_reference_functions).remove(&name);
        }
        if self.host_function_parameter_types.contains_key(&name) {
            Arc::make_mut(&mut self.host_function_parameter_types).remove(&name);
        }
        let function = match parameter_count {
            Some(parameter_count) => RegisteredHostFunction::declared(func, parameter_count),
            None => RegisteredHostFunction::variadic(func),
        };
        Arc::make_mut(&mut self.host_functions).insert(name, function);
    }

    /// Register a host function whose listed zero-based parameters receive
    /// live script lvalues when the call expression supplies one. Untyped
    /// embedding callbacks may still inspect a non-lvalue through
    /// [`HostCallArg::read`]; once a matching [`C4VType::Ref`] signature is
    /// attached, native conversion rejects that non-reference before entry.
    pub fn register_host_reference_function<F, I>(
        &mut self,
        name: impl Into<String>,
        reference_parameters: I,
        func: F,
    ) where
        F: Fn(&[HostCallArg]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
        I: IntoIterator<Item = usize>,
    {
        let name = name.into();
        if self.host_functions.contains_key(&name) {
            Arc::make_mut(&mut self.host_functions).remove(&name);
        }
        if self.host_function_parameter_types.contains_key(&name) {
            Arc::make_mut(&mut self.host_function_parameter_types).remove(&name);
        }
        Arc::make_mut(&mut self.host_reference_functions).insert(
            name,
            HostReferenceFunction::new(reference_parameters, None, func),
        );
    }

    /// Reference-aware counterpart to
    /// [`Engine::register_host_function_with_arity`].
    pub fn register_host_reference_function_with_arity<F, I>(
        &mut self,
        name: impl Into<String>,
        parameter_count: usize,
        reference_parameters: I,
        func: F,
    ) where
        F: Fn(&[HostCallArg]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
        I: IntoIterator<Item = usize>,
    {
        assert!(
            parameter_count <= 10,
            "C4Aul native functions cannot declare more than 10 parameters"
        );
        let reference_parameters = reference_parameters.into_iter().collect::<Vec<_>>();
        assert!(
            reference_parameters
                .iter()
                .all(|index| *index < parameter_count),
            "native reference parameters must be inside the declared parameter list"
        );
        let name = name.into();
        if self.host_functions.contains_key(&name) {
            Arc::make_mut(&mut self.host_functions).remove(&name);
        }
        if self.host_function_parameter_types.contains_key(&name) {
            Arc::make_mut(&mut self.host_function_parameter_types).remove(&name);
        }
        Arc::make_mut(&mut self.host_reference_functions).insert(
            name,
            HostReferenceFunction::new(reference_parameters, Some(parameter_count), func),
        );
    }

    /// The declared native parameter count, or `None` for an embedding-only
    /// variadic callback or an unknown name.
    pub fn host_function_parameter_count(&self, name: &str) -> Option<usize> {
        self.host_functions
            .get(name)
            .and_then(RegisteredHostFunction::parameter_count)
            .or_else(|| {
                self.host_reference_functions
                    .get(name)
                    .and_then(HostReferenceFunction::parameter_count)
            })
    }

    /// Attach the complete C4V parameter signature to an already-registered
    /// native function. The signature length is the native arity; each slot
    /// is converted before the debugger or callback can observe the call.
    ///
    /// Returns `false` when `name` has no host registration.
    pub fn set_host_function_parameter_types<I>(&mut self, name: &str, parameter_types: I) -> bool
    where
        I: IntoIterator<Item = C4VType>,
    {
        if !self.host_functions.contains_key(name)
            && !self.host_reference_functions.contains_key(name)
        {
            return false;
        }
        let parameter_types = parameter_types.into_iter().collect::<Vec<_>>();
        assert!(
            parameter_types.len() <= 10,
            "C4Aul native functions cannot declare more than 10 parameters"
        );
        if let Some(parameter_count) = self.host_function_parameter_count(name) {
            assert_eq!(
                parameter_types.len(),
                parameter_count,
                "native C4V signature length must match its declared parameter count"
            );
        }
        let declared_references = parameter_types
            .iter()
            .enumerate()
            .filter_map(|(index, value_type)| (*value_type == C4VType::Ref).then_some(index))
            .collect::<Vec<_>>();
        if self.host_functions.contains_key(name) {
            assert!(
                declared_references.is_empty(),
                "native Ref slots require a reference-aware host callback"
            );
        } else if let Some(function) = self.host_reference_functions.get(name) {
            assert_eq!(
                declared_references, function.reference_parameters,
                "native C4V Ref slots must match the reference-aware registration"
            );
        }
        Arc::make_mut(&mut self.host_function_parameter_types)
            .insert(name.to_string(), Arc::from(parameter_types));
        true
    }

    pub fn host_function_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .host_functions
            .keys()
            .chain(self.host_reference_functions.keys())
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn clear_host_functions(&mut self) {
        let empty_registrations = empty_host_registration_snapshot();
        self.host_functions = Arc::clone(&empty_registrations.host_functions);
        self.host_reference_functions = Arc::clone(&empty_registrations.host_reference_functions);
        self.host_function_parameter_types =
            Arc::clone(&empty_registrations.host_function_parameter_types);
    }

    /// Remove either native-host registration kind under `name`. The return
    /// value remains the ordinary value-host callback for API compatibility;
    /// removing a reference-aware registration succeeds with `None`.
    pub fn remove_host_function(&mut self, name: &str) -> Option<HostFunction> {
        if self.host_reference_functions.contains_key(name) {
            Arc::make_mut(&mut self.host_reference_functions).remove(name);
        }
        if self.host_function_parameter_types.contains_key(name) {
            Arc::make_mut(&mut self.host_function_parameter_types).remove(name);
        }
        if self.host_functions.contains_key(name) {
            Arc::make_mut(&mut self.host_functions)
                .remove(name)
                .map(|function| function.callback)
        } else {
            None
        }
    }

    /// Registers the cross-object method resolver for `obj->Method(args)`
    /// (AB_CALL, C4AulExec.cpp:1216-1305). Arguments: [target, name,
    /// failsafe, args...].
    /// Attaches the engine-global `static` table
    /// (C4AulScriptEngine::GlobalNamed). Scripts added afterwards register
    /// their `static` declarations here instead of the per-object locals.
    pub fn set_global_variables(&mut self, table: GlobalVariables) {
        self.globals_named = Some(table);
    }

    /// Attaches the engine-global numbered-variable table
    /// (`C4AulScriptEngine::Global`).
    pub fn set_global_slots(&mut self, table: GlobalSlots) {
        self.globals_numbered = Some(table);
    }

    /// Attaches the engine-global `static const` registry (the C4Aul
    /// global-constant table, C4Aul.cpp:484). Scripts adopted afterwards
    /// register their constants here so every host resolves them — both
    /// as identifiers and via the pre-#strict-2 `NAME()` call idiom
    /// (C4AulParse.cpp:2834-2864).
    pub fn set_global_constants(&mut self, table: GlobalVariables) {
        self.globals_consts = Some(table);
    }

    /// Attach the game-global C4StringTable registration ledger. Existing
    /// parsed literals are registered immediately; later scripts register as
    /// they are installed and runtime values register through the VM.
    pub fn set_string_registrations(&mut self, registrations: StringRegistrations) {
        for literal in &self.string_literals {
            register_c4_literal_string(&registrations, literal);
        }
        self.string_registrations = Some(registrations);
    }

    /// Attach the game-global string table during C4Aul's preparse phase.
    /// Existing function-body operands are deliberately not marked Hold until
    /// the later engine-global Parse pass, after every host's constants exist.
    pub fn set_string_registrations_deferred(&mut self, registrations: StringRegistrations) {
        self.string_registrations = Some(registrations);
    }

    /// Acquire every function-body string operand in this host's parser order.
    /// The embedding script engine calls this once for each child host at the
    /// global Link/Parse boundary (and again after ReLink's Clear/reset pass).
    pub fn acquire_string_literal_holds(&mut self) {
        let Some(registrations) = &self.string_registrations else {
            return;
        };
        for literal in &self.string_literals {
            register_c4_literal_string(registrations, literal);
        }
    }

    /// Moves `static` declarations that were compiled BEFORE the table was
    /// attached out of the per-object locals and into the shared table
    /// (existing values persist).
    pub fn adopt_statics_into_globals(&mut self) {
        let Some(table) = self.globals_named.clone() else {
            return;
        };
        let globals_consts = self.globals_consts.clone();
        let registration = register_global_declarations_inner(
            &self.var_decls,
            &table,
            globals_consts.as_ref(),
            self.string_registrations.as_deref(),
            Some(&self.constants),
        );
        if let Err(error) = registration {
            self.static_const_link_errors.push(error);
        }
        self.var_decls
            .retain(|var_decl| var_decl.kind == crate::ast::VarDeclKind::Local);
    }

    /// Registers the cross-object LocalN cell supplier (FnLocalN's
    /// by-reference foreign-local access, C4Script.cpp:4591-4605).
    pub fn register_local_cell_hook(&mut self, hook: LocalCellHook) {
        self.local_cell_hook = Some(hook);
    }

    pub fn register_object_target_availability_probe(
        &mut self,
        probe: ObjectTargetAvailabilityProbe,
    ) {
        self.object_target_availability_probe = Some(probe);
    }

    pub fn register_method_dispatch(&mut self, dispatch: HostFunction) {
        self.method_dispatch = Some(dispatch);
    }

    pub fn register_method_reference_dispatch(&mut self, dispatch: MethodReferenceDispatch) {
        self.method_reference_dispatch = Some(dispatch);
    }

    pub fn register_method_ref_args_dispatch(&mut self, dispatch: MethodRefArgsDispatch) {
        self.method_ref_args_dispatch = Some(dispatch);
    }

    pub fn register_reference_parameter_probe(&mut self, probe: ReferenceParameterProbe) {
        self.reference_parameter_probe = Some(probe);
    }

    pub fn register_direct_call_function_probe(&mut self, probe: DirectCallFunctionProbe) {
        self.direct_call_function_probe = Some(probe);
    }

    pub fn register_global_call_context_hook(&mut self, hook: GlobalCallContextHook) {
        self.global_call_context_hook = Some(hook);
    }

    pub fn register_eval_direct_exec_hook(&mut self, hook: EvalDirectExecHook) {
        self.eval_direct_exec_hook = Some(hook);
    }

    /// The VM every call path on this host runs. Each entry point below
    /// attaches the same function tables, host seams and global tables, so
    /// assembling it once keeps a newly registered channel from reaching
    /// some call paths and silently missing others.
    fn vm(&self) -> Vm<'_> {
        Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_owner_definition_name(self.definition_name.as_deref())
        .with_script_name(self.script_name.as_deref().unwrap_or(""))
        .with_game_script_name(self.game_script_name.as_deref())
        .with_definition_context(self.definition_context)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_host_function_parameter_types(&self.host_function_parameter_types)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_method_ref_args_dispatch(self.method_ref_args_dispatch.as_ref())
        .with_reference_parameter_probe(self.reference_parameter_probe.as_ref())
        .with_direct_call_function_probe(self.direct_call_function_probe.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_eval_direct_exec_hook(self.eval_direct_exec_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_object_target_availability_probe(self.object_target_availability_probe.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, ScriptError> {
        let vm = self.vm();
        vm.call(name, args).map_err(ScriptError::from)
    }

    /// Calls a function passing every argument as a REFERENCE cell — the
    /// host-side C4AulParSet-of-refs pattern (C4Material.cpp:814-815):
    /// callee `&` parameters alias the cells so their writes are visible in
    /// the returned final argument values; plain parameters receive
    /// dereferenced copies (C4Value.cpp:586-597). Returns the call result
    /// plus the final value of every argument cell.
    pub fn call_with_ref_args(
        &self,
        name: &str,
        args: &[Value],
    ) -> Result<(Value, Vec<Value>), ScriptError> {
        let vm = self.vm();
        let cells: Vec<crate::vm::ValueCell> =
            args.iter().cloned().map(crate::vm::value_cell).collect();
        let call_args = cells
            .iter()
            .map(|cell| crate::vm::CallArg::Reference(crate::vm::LValueRef::cell(cell.clone())))
            .collect();
        let result = vm.call_args(name, call_args).map_err(ScriptError::from)?;
        let finals = cells.iter().map(|cell| cell.borrow().clone()).collect();
        Ok((result, finals))
    }

    /// Execute the named function from the currently attached engine-global
    /// table, bypassing this host's own function map. Deferred C4Aul callers
    /// use this after retaining an engine-owned SFunc pointer at queue time.
    #[doc(hidden)]
    pub fn call_global_with_ref_args(
        &self,
        name: &str,
        args: &[Value],
    ) -> Result<(Value, Vec<Value>), ScriptError> {
        let vm = self.vm().with_exact_global_link_lookup();
        let cells: Vec<crate::vm::ValueCell> =
            args.iter().cloned().map(crate::vm::value_cell).collect();
        let call_args = cells
            .iter()
            .map(|cell| crate::vm::CallArg::Reference(crate::vm::LValueRef::cell(cell.clone())))
            .collect();
        let result = vm
            .call_engine_global_args(name, call_args)
            .map_err(ScriptError::from)?;
        let finals = cells.iter().map(|cell| cell.borrow().clone()).collect();
        Ok((result, finals))
    }

    /// Exact engine-global entry for a scripted C4Effect callback. C++ builds
    /// its callback argument set from owned C4Values, so `&` parameters do not
    /// receive aliases unless the native caller explicitly supplied one.
    #[doc(hidden)]
    pub fn call_global_for_effect_callback(
        &self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, ScriptError> {
        let vm = self
            .vm()
            .with_exact_global_link_lookup()
            .with_effect_callback_parameter_conversion();
        vm.call_engine_global(name, args).map_err(ScriptError::from)
    }

    /// Execute an immutable function captured by a deferred native callback.
    /// The destination Engine still contributes its live host functions,
    /// globals and local-helper scope; the entry body is never re-resolved by
    /// name. `engine_global` enables exact LinkedTo lookup inside the body.
    #[doc(hidden)]
    pub fn call_pinned_with_ref_args(
        &self,
        function: &Function,
        engine_global: bool,
        args: &[Value],
    ) -> Result<(Value, Vec<Value>), ScriptError> {
        let vm = self.vm();
        let vm = if engine_global {
            vm.with_exact_global_link_lookup()
        } else {
            vm
        };
        let cells: Vec<crate::vm::ValueCell> =
            args.iter().cloned().map(crate::vm::value_cell).collect();
        let call_args = cells
            .iter()
            .map(|cell| crate::vm::CallArg::Reference(crate::vm::LValueRef::cell(cell.clone())))
            .collect();
        let result = vm
            .call_pinned_args(function, call_args)
            .map_err(ScriptError::from)?;
        let finals = cells.iter().map(|cell| cell.borrow().clone()).collect();
        Ok((result, finals))
    }

    /// Execute a function snapshot returned by [`Engine::resolve_function`]
    /// or [`Engine::resolve_global_function`].
    #[doc(hidden)]
    pub fn call_resolved_with_ref_args(
        &self,
        resolution: &ScriptFunctionResolution,
        engine_global: bool,
        args: &[Value],
    ) -> Result<(Value, Vec<Value>), ScriptError> {
        let vm = self.vm();
        let vm = if engine_global {
            vm.with_exact_global_link_lookup()
        } else {
            vm
        };
        let cells: Vec<crate::vm::ValueCell> =
            args.iter().cloned().map(crate::vm::value_cell).collect();
        let call_args = cells
            .iter()
            .map(|cell| crate::vm::CallArg::Reference(crate::vm::LValueRef::cell(cell.clone())))
            .collect();
        let result = vm
            .call_resolved_args(resolution, call_args)
            .map_err(ScriptError::from)?;
        let finals = cells.iter().map(|cell| cell.borrow().clone()).collect();
        Ok((result, finals))
    }

    /// Execute an immutable function captured by a native callback against
    /// shared object-local cells and an explicit `this`. The entry body is
    /// never re-resolved by name; `engine_global` enables exact lookup through
    /// a global function's retained `LinkedTo` host for calls inside its body.
    /// This is a fresh engine callback and does not inherit an ambient caller.
    #[doc(hidden)]
    pub fn call_pinned_with_cells_and_this(
        &self,
        function: &Function,
        engine_global: bool,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        let vm = self.vm().with_this(this);
        let vm = if engine_global {
            vm.with_exact_global_link_lookup()
        } else {
            vm
        };
        vm.call_pinned_with_cells(function, args, cells)
            .map_err(ScriptError::from)
    }

    /// Execute a resolved immutable snapshot against shared object-local
    /// cells and an explicit `this` value.
    #[doc(hidden)]
    pub fn call_resolved_with_cells_and_this(
        &self,
        resolution: &ScriptFunctionResolution,
        engine_global: bool,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        let vm = self.vm().with_this(this);
        let vm = if engine_global {
            vm.with_exact_global_link_lookup()
        } else {
            vm
        };
        vm.call_resolved_with_cells(resolution, args, cells)
            .map_err(ScriptError::from)
    }

    /// Scripted-C4Effect counterpart to
    /// [`Engine::call_pinned_with_cells_and_this`]. The selected callback
    /// alone receives C++'s pre-STRICT3 conversion-warning compatibility.
    #[doc(hidden)]
    pub fn call_pinned_with_cells_and_this_for_effect_callback(
        &self,
        function: &Function,
        engine_global: bool,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        let vm = self
            .vm()
            .with_this(this)
            .with_effect_callback_parameter_conversion();
        let vm = if engine_global {
            vm.with_exact_global_link_lookup()
        } else {
            vm
        };
        vm.call_pinned_with_cells(function, args, cells)
            .map_err(ScriptError::from)
    }

    /// Resolved-snapshot counterpart to
    /// [`Engine::call_pinned_with_cells_and_this_for_effect_callback`].
    #[doc(hidden)]
    pub fn call_resolved_with_cells_and_this_for_effect_callback(
        &self,
        resolution: &ScriptFunctionResolution,
        engine_global: bool,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        let vm = self
            .vm()
            .with_this(this)
            .with_effect_callback_parameter_conversion();
        let vm = if engine_global {
            vm.with_exact_global_link_lookup()
        } else {
            vm
        };
        vm.call_resolved_with_cells(resolution, args, cells)
            .map_err(ScriptError::from)
    }

    /// Call a function with per-object local variable context
    /// Returns (result, updated_local_vars)
    pub fn call_with_locals(
        &self,
        name: &str,
        args: &[Value],
        local_vars: &std::collections::HashMap<String, Value>,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        let vm = self.vm();
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    /// Like [`call_with_locals`], but also provides the object context returned
    /// by an unbound script `this`. Pass `Value::Object(id)` for an object
    /// context or `Value::Nil` for no context.
    /// Like [`call_with_locals_and_this`], against SHARED live cells: the
    /// session mutates them in place (C++ object locals), so callers fold
    /// via [`crate::vm::LocalCells::snapshot`] instead of a return map.
    pub fn call_with_cells_and_this(
        &self,
        name: &str,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        let vm = self.vm().with_this(this);
        vm.call_with_cells(name, args, cells)
            .map_err(ScriptError::from)
    }

    /// Calls the selected scripted C4Effect callback with C++'s
    /// `nonStrict3WarnConversionOnly` parameter behavior. Do not use for
    /// ordinary script calls.
    #[doc(hidden)]
    pub fn call_effect_callback_with_cells_and_this(
        &self,
        name: &str,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        let vm = self
            .vm()
            .with_this(this)
            .with_effect_callback_parameter_conversion();
        vm.call_with_cells(name, args, cells)
            .map_err(ScriptError::from)
    }

    /// Cross-object AB_CALL bridge entry. This preserves the script caller
    /// installed by the VM around method dispatch, so a target that resolves
    /// directly to a native host function still observes the suspended
    /// caller's Var slots and strict level. Ordinary engine-driven callbacks
    /// must use [`Engine::call_with_cells_and_this`] and remain callerless.
    #[doc(hidden)]
    pub fn call_with_cells_and_this_preserving_caller(
        &self,
        name: &str,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        let vm = self.vm().with_this(this);
        vm.call_with_cells_preserving_caller(name, args, cells)
            .map_err(ScriptError::from)
    }

    /// `call_with_cells_and_this_preserving_caller` for a callee that declares
    /// `&` parameters: the arguments enter as reference cells so those
    /// parameters alias them, and each slot's final value comes back with the
    /// result. Plain parameters receive a dereferenced copy
    /// (C4Value.cpp:586-597), so their slots read back unchanged.
    pub fn call_ref_args_with_cells_and_this_preserving_caller(
        &self,
        name: &str,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<(Value, Vec<Value>), ScriptError> {
        let vm = self.vm().with_this(this);
        let arg_cells: Vec<crate::vm::ValueCell> =
            args.iter().cloned().map(crate::vm::value_cell).collect();
        let call_args = arg_cells
            .iter()
            .map(|cell| crate::vm::CallArg::Reference(crate::vm::LValueRef::cell(cell.clone())))
            .collect();
        let result = vm
            .call_args_with_cells_preserving_caller(name, call_args, cells)
            .map_err(ScriptError::from)?;
        let finals = arg_cells.iter().map(|cell| cell.borrow().clone()).collect();
        Ok((result, finals))
    }

    /// Calls a `func &` against shared object-local cells without
    /// dereferencing its result. Engine method dispatch uses this to carry an
    /// arrow-call lvalue back to the suspended caller.
    pub fn call_reference_with_cells_and_this(
        &self,
        name: &str,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<ValueReference, ScriptError> {
        let vm = self.vm().with_this(this);
        vm.call_reference_with_cells(name, args, cells)
            .map_err(ScriptError::from)
    }

    /// Reference-returning counterpart to
    /// [`Engine::call_with_cells_and_this_preserving_caller`].
    #[doc(hidden)]
    pub fn call_reference_with_cells_and_this_preserving_caller(
        &self,
        name: &str,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<ValueReference, ScriptError> {
        let vm = self.vm().with_this(this);
        vm.call_reference_with_cells_preserving_caller(name, args, cells)
            .map_err(ScriptError::from)
    }

    pub fn call_with_locals_and_this(
        &self,
        name: &str,
        args: &[Value],
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        let vm = self.vm().with_this(this);
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    /// Scripted-C4Effect counterpart to [`Engine::call_with_locals_and_this`]
    /// for a definition-scope callback with no object receiver.
    #[doc(hidden)]
    pub fn call_effect_callback_with_locals_and_this(
        &self,
        name: &str,
        args: &[Value],
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        let vm = self
            .vm()
            .with_this(this)
            .with_effect_callback_parameter_conversion();
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    /// C4AulScript::DirectExec in an object context (the C4Object::
    /// MenuCommand seam, C4Object.cpp:3756-3760): runs `source` as ONE
    /// expression with the object's locals and `this`, at the script's own
    /// strict level (DirectExec's Strict parameter). Parse errors yield
    /// nil; runtime errors surface for the caller's fPassErrors handling.
    pub fn direct_exec_with_locals_and_this(
        &self,
        source: &str,
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        self.direct_exec_with_locals_and_this_in_context(source, local_vars, this, "DirectExec")
    }

    pub fn direct_exec_with_locals_and_this_in_context(
        &self,
        source: &str,
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
        context: &str,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        self.direct_exec_with_locals_and_this_at_strict_in_context(
            source,
            local_vars,
            this,
            self.script_strict_level(),
            context,
        )
    }

    /// Explicit-strictness counterpart used by synchronized DirectExec
    /// controls. The control packet's strictness belongs to the temporary
    /// expression script and must not be inferred from the destination host.
    pub fn direct_exec_with_locals_and_this_at_strict(
        &self,
        source: &str,
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
        strict_level: Option<u8>,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        self.direct_exec_with_locals_and_this_at_strict_in_context(
            source,
            local_vars,
            this,
            strict_level,
            "DirectExec",
        )
    }

    pub fn direct_exec_with_locals_and_this_at_strict_in_context(
        &self,
        source: &str,
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
        strict_level: Option<u8>,
        context: &str,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        self.direct_exec_with_locals_and_this_at_strict_in_context_diagnostics(
            source,
            local_vars,
            this,
            strict_level,
            context,
            true,
        )
    }

    #[doc(hidden)]
    pub fn direct_exec_with_locals_and_this_at_strict_in_context_diagnostics(
        &self,
        source: &str,
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
        strict_level: Option<u8>,
        context: &str,
        diagnostics: bool,
    ) -> Result<(Value, std::collections::HashMap<String, Value>), ScriptError> {
        let vm = self.vm().with_this(this);
        vm.direct_exec_with_locals_in_context(
            source,
            local_vars,
            strict_level,
            context,
            diagnostics,
        )
        .map_err(ScriptError::from)
    }

    /// Like [`direct_exec_with_locals_and_this`], against SHARED live
    /// cells: the session mutates them in place (C++ object locals), so
    /// nested calls the host routes back onto the same object see the
    /// mid-exec writes. Callers fold via [`crate::vm::LocalCells::snapshot`].
    ///
    /// [`direct_exec_with_locals_and_this`]: Engine::direct_exec_with_locals_and_this
    pub fn direct_exec_with_cells_and_this(
        &self,
        source: &str,
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Value, ScriptError> {
        self.direct_exec_with_cells_and_this_in_context(source, cells, this, "DirectExec")
    }

    pub fn direct_exec_with_cells_and_this_in_context(
        &self,
        source: &str,
        cells: &crate::vm::LocalCells,
        this: Value,
        context: &str,
    ) -> Result<Value, ScriptError> {
        self.direct_exec_with_cells_and_this_at_strict_in_context(
            source,
            cells,
            this,
            self.script_strict_level(),
            context,
        )
    }

    #[doc(hidden)]
    pub fn direct_exec_with_cells_and_this_in_context_diagnostics(
        &self,
        source: &str,
        cells: &crate::vm::LocalCells,
        this: Value,
        context: &str,
        diagnostics: bool,
    ) -> Result<Value, ScriptError> {
        self.direct_exec_with_cells_and_this_at_strict_in_context_diagnostics(
            source,
            cells,
            this,
            self.script_strict_level(),
            context,
            diagnostics,
        )
    }

    /// Shared-cell DirectExec with packet-supplied strictness. This is kept
    /// separate from the object-menu API above so existing callers retain the
    /// destination script's strictness.
    pub fn direct_exec_with_cells_and_this_at_strict(
        &self,
        source: &str,
        cells: &crate::vm::LocalCells,
        this: Value,
        strict_level: Option<u8>,
    ) -> Result<Value, ScriptError> {
        self.direct_exec_with_cells_and_this_at_strict_in_context(
            source,
            cells,
            this,
            strict_level,
            "DirectExec",
        )
    }

    pub fn direct_exec_with_cells_and_this_at_strict_in_context(
        &self,
        source: &str,
        cells: &crate::vm::LocalCells,
        this: Value,
        strict_level: Option<u8>,
        context: &str,
    ) -> Result<Value, ScriptError> {
        self.direct_exec_with_cells_and_this_at_strict_in_context_diagnostics(
            source,
            cells,
            this,
            strict_level,
            context,
            true,
        )
    }

    #[doc(hidden)]
    pub fn direct_exec_with_cells_and_this_at_strict_in_context_diagnostics(
        &self,
        source: &str,
        cells: &crate::vm::LocalCells,
        this: Value,
        strict_level: Option<u8>,
        context: &str,
        diagnostics: bool,
    ) -> Result<Value, ScriptError> {
        let vm = self.vm().with_this(this);
        vm.direct_exec_with_cells_in_context(source, cells, strict_level, context, diagnostics)
            .map_err(ScriptError::from)
    }

    /// Exact receiver-side entry for FnEval. The embedding engine first
    /// selects the active object's definition script, active definition, or
    /// Game.Script, then this method creates DirectExec's temporary child
    /// against the caller's shared object-local cells.
    #[doc(hidden)]
    pub fn eval_direct_exec_with_cells_and_this_at_strict(
        &self,
        source: &str,
        cells: &crate::vm::LocalCells,
        this: Value,
        strict_level: Option<u8>,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        let vm = self.vm().with_this(this);
        vm.eval_direct_exec_with_cells(source, cells, strict_level, depth)
    }

    /// The destination host script's own parsed strict level. Included and
    /// appended function copies retain their origin strictness, but never
    /// change `C4AulScript::Strict` on the host receiving them.
    fn script_strict_level(&self) -> Option<u8> {
        self.owner_strict_level.unwrap_or(None)
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    /// Whether this host has a named, non-global script function. A C4Aul
    /// `global func` is stored on the script engine and leaves only an
    /// unnamed FnLink in its declaring script, so it cannot arm an object's
    /// own lifecycle callback (Construction/Initialize/Step).
    pub fn has_local_function(&self, name: &str) -> bool {
        self.functions
            .get(name)
            .and_then(crate::ast::Function::first_non_global)
            .is_some()
    }

    /// Own linked script functions, including inherited definition functions.
    /// Consumers such as C4MN_Context need the retained C4Aul description
    /// metadata, not merely name-based execution.
    pub fn functions(&self) -> &FxHashMap<String, Function> {
        &self.functions
    }

    /// Active named script functions in C4Aul's indexed enumeration order.
    /// `FuncL` is visited first, so appended functions precede declarations
    /// in this host, which in turn precede low-priority include copies.
    /// Engine-global declarations are absent because their local `FnLink`s
    /// are unnamed and therefore have no `SFunc()` result.
    pub fn local_functions_in_get_sfunc_order(&self) -> impl Iterator<Item = (&String, &Function)> {
        let mut seen = HashSet::new();
        self.local_function_order
            .iter()
            .rev()
            .filter(move |name| seen.insert(name.as_str()))
            .filter_map(|name| {
                latest_function_with_access(self.functions.get(name)?, false)
                    .map(|function| (name, function))
            })
    }

    /// Own functions OR the engine-global table. Object callbacks
    /// (Initialize/TimerCall/…) resolve own-script only, but EFFECT
    /// callbacks recurse up the C4Aul tree to the script engine
    /// (FxIntScheduleCallTimer lives in the planet Helpers.c) —
    /// C4Effect resolves Fx* against the command target's Def script
    /// with engine-level fallback.
    pub fn has_function_or_global(&self, name: &str) -> bool {
        self.functions.contains_key(name)
            || self
                .global_functions
                .as_deref()
                .is_some_and(|table| table.contains_key(name))
    }

    /// Resolve C4AulFunc::GetLocalSFunc and report the selected function's
    /// ownership scope. A script-local global declaration is represented in
    /// C++ by an unnamed FnLink, so it is never a named local candidate:
    /// global callers search ordinary functions in LinkedTo's host first and
    /// then the current engine-global table; local callers search only the
    /// ordinary functions in their owner host.
    pub fn resolve_function(
        &self,
        name: &str,
        include_engine_globals: bool,
    ) -> Option<ScriptFunctionResolution> {
        let local = self
            .functions
            .get(name)
            .and_then(crate::ast::Function::first_non_global);
        let function = local.or_else(|| {
            include_engine_globals
                .then(|| self.global_functions.as_deref()?.get(name))
                .flatten()
        })?;
        let scope = if function.access == crate::ast::AccessLevel::Global {
            ScriptFunctionScope::Global
        } else {
            ScriptFunctionScope::Local
        };
        let host_identity = if scope == ScriptFunctionScope::Global {
            function.global_link_host.unwrap_or(self.host_identity)
        } else {
            self.host_identity
        };
        Some(ScriptFunctionResolution::new(
            scope,
            host_identity,
            function,
        ))
    }

    /// Resolve only the engine-owned global script table. This is the
    /// `GetFuncRecursive` starting point when the calling function's owner
    /// is `Game.ScriptEngine`, so declaring-host locals must not shadow it.
    pub fn resolve_global_function(&self, name: &str) -> Option<ScriptFunctionResolution> {
        let function = self.global_functions.as_deref()?.get(name)?;
        Some(ScriptFunctionResolution::new(
            ScriptFunctionScope::Global,
            function.global_link_host.unwrap_or(self.host_identity),
            function,
        ))
    }

    /// Backward-compatible ownership-only view of [`Self::resolve_function`].
    pub fn resolve_function_scope(
        &self,
        name: &str,
        include_engine_globals: bool,
    ) -> Option<ScriptFunctionScope> {
        self.resolve_function(name, include_engine_globals)
            .map(|resolution| resolution.scope)
    }

    pub fn has_host_function(&self, name: &str) -> bool {
        self.host_functions.contains_key(name) || self.host_reference_functions.contains_key(name)
    }

    pub fn call_effect_callback(
        &self,
        effect_name: &str,
        event: &str,
        args: &[Value],
    ) -> Result<Option<Value>, ScriptError> {
        let mut function_name = String::with_capacity(effect_name.len() + event.len() + 2);
        function_name.push_str("Fx");
        function_name.push_str(effect_name);
        function_name.push_str(event);
        if !self.has_function(&function_name) {
            return Ok(None);
        }
        self.call(&function_name, args).map(Some)
    }

    /// Like [`call_effect_callback`], but with the C++ execution context:
    /// effect callbacks run on the effect's command target
    /// (`pFn->Exec(pCommandTarget, ...)`, C4Effect.cpp:129,345,392,456),
    /// so `this` and the target's object locals are live. Returns the
    /// result and the final local values.
    #[allow(clippy::type_complexity)]
    pub fn call_effect_callback_in_context(
        &self,
        effect_name: &str,
        event: &str,
        args: &[Value],
        local_vars: &std::collections::HashMap<String, Value>,
        this: Value,
    ) -> Result<Option<(Value, std::collections::HashMap<String, Value>)>, ScriptError> {
        let mut function_name = String::with_capacity(effect_name.len() + event.len() + 2);
        function_name.push_str("Fx");
        function_name.push_str(effect_name);
        function_name.push_str(event);
        if !self.has_function_or_global(&function_name) {
            return Ok(None);
        }
        self.call_with_locals_and_this(&function_name, args, local_vars, this)
            .map(Some)
    }

    /// Like [`call_effect_callback_in_context`], but against SHARED live
    /// cells: nested calls that the host routes back onto the same object
    /// mutate the identical storage mid-call (C++ mutates the one live
    /// C4Object). Returns the result and the final cell snapshot.
    #[allow(clippy::type_complexity)]
    pub fn call_effect_callback_in_context_with_cells(
        &self,
        effect_name: &str,
        event: &str,
        args: &[Value],
        cells: &crate::vm::LocalCells,
        this: Value,
    ) -> Result<Option<(Value, std::collections::HashMap<String, Value>)>, ScriptError> {
        let mut function_name = String::with_capacity(effect_name.len() + event.len() + 2);
        function_name.push_str("Fx");
        function_name.push_str(effect_name);
        function_name.push_str(event);
        if !self.has_function_or_global(&function_name) {
            return Ok(None);
        }
        let value = self.call_with_cells_and_this(&function_name, args, cells, this)?;
        Ok(Some((value, cells.snapshot())))
    }

    pub fn has_effect_callback(&self, effect_name: &str, event: &str) -> bool {
        let mut function_name = String::with_capacity(effect_name.len() + event.len() + 2);
        function_name.push_str("Fx");
        function_name.push_str(effect_name);
        function_name.push_str(event);
        self.has_function_or_global(&function_name)
    }

    pub fn set_debugger_hooks(&mut self, hooks: DebuggerHooks) {
        self.debugger_hooks = Some(hooks);
    }

    pub fn clear_debugger_hooks(&mut self) {
        self.debugger_hooks = None;
    }

    pub fn debugger_hooks(&self) -> Option<&DebuggerHooks> {
        self.debugger_hooks.as_ref()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn string_registration_membership_uses_fixed_seed_hashing() {
        fn assert_fx_set<T>(_: &rustc_hash::FxHashSet<T>) {}
        fn assert_fx_map<K, V>(_: &rustc_hash::FxHashMap<K, V>) {}

        let registrations = StringRegistrationLedger::default();
        let state = registrations.borrow();
        assert_fx_set(&state.registered_identities);
        assert_fx_set(&state.detached_identities);
        assert_fx_map(&state.literal_cache);
    }

    #[test]
    fn composite_string_registration_holds_the_ledger_once() {
        let registrations = StringRegistrationLedger::default();
        let value = Value::Array(vec![
            Value::from("Alpha"),
            Value::Array(vec![Value::from("Beta"), Value::from("Gamma")]),
        ]);

        reset_string_registration_mutable_borrows();
        register_c4_value_strings(&registrations, &value);

        assert_eq!(string_registration_mutable_borrows(), 1);
        assert_eq!(
            enumerate_c4_strings(&registrations, &[]),
            [b"Alpha".to_vec(), b"Beta".to_vec(), b"Gamma".to_vec()]
        );
    }

    #[test]
    fn scalar_registration_does_not_borrow_the_string_ledger() {
        let registrations = StringRegistrationLedger::default();

        reset_string_registration_mutable_borrows();
        register_c4_value_strings(&registrations, &Value::Int(7));

        assert_eq!(string_registration_mutable_borrows(), 0);
    }

    #[test]
    fn runtime_string_registration_amortizes_dead_entry_sweeps() {
        let registrations = StringRegistrationLedger::default();

        reset_string_registration_entry_sweeps();
        for index in 0..32 {
            let value = C4StringValue::new(format!("temporary-{index}"));
            register_c4_string(&registrations, &value);
        }

        assert_eq!(string_registration_entry_sweeps(), 0);
    }

    fn compile(source: &str) -> Script {
        Script::compile(source).expect("test script compiles")
    }

    fn local_get_sfunc_names(engine: &Engine) -> Vec<&str> {
        engine
            .local_functions_in_get_sfunc_order()
            .map(|(name, _)| name.as_str())
            .collect()
    }

    fn global_declaration_names(engine: &Engine) -> Vec<&str> {
        engine
            .global_access_functions()
            .map(|(name, _)| name.as_str())
            .collect()
    }

    #[test]
    fn linked_function_order_tracks_append_local_and_include_layers() {
        let mut included = Engine::new();
        included
            .load_script(
                "func IncludedEarly() { return 1; }\n\
                 func IncludedLate() { return 2; }",
            )
            .expect("included script compiles");

        let mut destination = Engine::new();
        destination
            .load_script(
                "func LocalEarly() { return 3; }\n\
                 func LocalLate() { return 4; }",
            )
            .expect("destination script compiles");

        let mut appended = Engine::new();
        appended
            .load_script(
                "func AppendedEarly() { return 5; }\n\
                 func AppendedLate() { return 6; }",
            )
            .expect("append script compiles");

        // C4Aul resolves high-priority appends before low-priority includes.
        destination.append_overrides_from(&appended);
        destination.merge_from(&included);

        assert_eq!(
            local_get_sfunc_names(&destination),
            [
                "AppendedLate",
                "AppendedEarly",
                "LocalLate",
                "LocalEarly",
                "IncludedLate",
                "IncludedEarly",
            ]
        );
    }

    #[test]
    fn local_and_global_orders_survive_same_name_links_independently() {
        let mut host = Engine::new();
        host.load_script(
            "func Shared() { return 1; }\n\
             global func GlobalEarly() { return 2; }\n\
             func Shared() { return 20; }\n\
             global func Shared() { return 3; }\n\
             global func GlobalEarly() { return 30; }\n\
             func LocalLate() { return 4; }\n\
             global func GlobalLate() { return 5; }",
        )
        .expect("mixed script compiles");

        assert_eq!(local_get_sfunc_names(&host), ["LocalLate", "Shared"]);
        assert_eq!(
            global_declaration_names(&host),
            ["Shared", "GlobalEarly", "GlobalLate"]
        );
        assert_eq!(
            host.global_functions_in_get_sfunc_order()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["GlobalLate", "GlobalEarly", "Shared"]
        );
    }

    #[test]
    fn global_fallback_keeps_the_declaring_host_while_local_calls_use_destination() {
        let mut declaring = Engine::new();
        declaring
            .load_script("global func Queue() { return Capture(); }")
            .expect("declaring global compiles");
        let declaring_identity = declaring.host_identity();
        let moved_declaring = declaring;
        assert_eq!(
            moved_declaring.host_identity(),
            declaring_identity,
            "host identity survives Engine moves"
        );
        assert_eq!(
            moved_declaring.clone().host_identity(),
            declaring_identity,
            "copy-on-write clones retain the logical host identity"
        );
        let globals = Arc::new(
            moved_declaring
                .global_access_functions()
                .map(|(name, function)| (name.clone(), function.clone()))
                .collect::<FxHashMap<_, _>>(),
        );

        let observed = Arc::new(Mutex::new(Vec::new()));
        let mut destination = Engine::new();
        let destination_identity = destination.host_identity();
        assert_ne!(declaring_identity, destination_identity);
        let observed_from_host = Arc::clone(&observed);
        destination.register_host_function("Capture", move |_| {
            observed_from_host.lock().expect("capture log locks").push((
                crate::vm::caller_host_identity(),
                crate::vm::caller_uses_engine_scope(),
            ));
            Ok(Value::Bool(true))
        });
        destination
            .load_script("func LocalQueue() { return Capture(); }")
            .expect("destination local compiles");
        destination.set_global_functions(Some(globals));

        assert_eq!(
            destination
                .call("LocalQueue", &[])
                .expect("local call runs"),
            Value::Bool(true)
        );
        assert_eq!(
            destination
                .call("Queue", &[])
                .expect("global fallback runs"),
            Value::Bool(true)
        );
        assert_eq!(
            observed.lock().expect("capture log locks").as_slice(),
            [
                (Some(destination_identity), Some(false)),
                (Some(declaring_identity), Some(true)),
            ]
        );
    }

    #[test]
    fn host_registration_snapshots_share_storage_and_detach_on_mutation() {
        let mut template = Engine::new();
        template.register_host_function_with_arity("Native", 1, |_| Ok(Value::Int(41)));
        assert!(template.set_host_function_parameter_types("Native", [C4VType::Int]));
        template.register_host_reference_function_with_arity("Reference", 1, [0], |_| {
            Ok(Value::Int(7))
        });
        assert!(template.set_host_function_parameter_types("Reference", [C4VType::Ref]));
        template.register_constant("BUILTIN", Value::Int(3));
        let snapshot = template.host_registration_snapshot();

        let mut first = Engine::new();
        first.apply_host_registration_snapshot(&snapshot);
        let mut second = Engine::new();
        second.apply_host_registration_snapshot(&snapshot);

        assert!(Arc::ptr_eq(&first.host_functions, &second.host_functions));
        assert!(Arc::ptr_eq(
            &first.host_reference_functions,
            &second.host_reference_functions
        ));
        assert!(Arc::ptr_eq(
            &first.host_function_parameter_types,
            &second.host_function_parameter_types
        ));
        assert!(Arc::ptr_eq(&first.constants, &second.constants));

        first.register_host_function_with_arity("Native", 1, |_| Ok(Value::Int(99)));
        assert!(first.set_host_function_parameter_types("Native", [C4VType::Int]));
        first.remove_host_function("Reference");
        first.register_constant("BUILTIN", Value::Int(8));

        assert_eq!(
            first.call("Native", &[]).expect("override runs"),
            Value::Int(99)
        );
        assert_eq!(
            second.call("Native", &[]).expect("cached native runs"),
            Value::Int(41)
        );
        assert!(!first.has_host_function("Reference"));
        assert!(second.has_host_function("Reference"));
        assert_eq!(first.constants.get("BUILTIN"), Some(&Value::Int(8)));
        assert_eq!(second.constants.get("BUILTIN"), Some(&Value::Int(3)));
    }

    #[test]
    fn applying_host_registration_snapshot_preserves_unrelated_entries() {
        let mut template = Engine::new();
        template.register_host_function_with_arity("Ordinary", 1, |_| Ok(Value::Int(1)));
        assert!(template.set_host_function_parameter_types("Ordinary", [C4VType::Int]));
        template.register_host_reference_function_with_arity("Reference", 1, [0], |_| {
            Ok(Value::Int(2))
        });
        assert!(template.set_host_function_parameter_types("Reference", [C4VType::Ref]));
        template.register_constant("BUILTIN", Value::Int(4));
        let snapshot = template.host_registration_snapshot();

        let mut destination = Engine::new();
        destination.register_host_reference_function_with_arity("Ordinary", 1, [0], |_| {
            Ok(Value::Int(10))
        });
        assert!(destination.set_host_function_parameter_types("Ordinary", [C4VType::Ref]));
        destination.register_host_function_with_arity("Reference", 1, |_| Ok(Value::Int(20)));
        assert!(destination.set_host_function_parameter_types("Reference", [C4VType::Int]));
        destination.register_host_function("Custom", |_| Ok(Value::Int(30)));
        destination.register_constant("BUILTIN", Value::Int(40));
        destination.register_constant("CUSTOM", Value::Int(50));

        destination.apply_host_registration_snapshot(&snapshot);

        assert!(destination.host_functions.contains_key("Ordinary"));
        assert!(!destination
            .host_reference_functions
            .contains_key("Ordinary"));
        assert!(!destination.host_functions.contains_key("Reference"));
        assert!(destination
            .host_reference_functions
            .contains_key("Reference"));
        assert!(destination.has_host_function("Custom"));
        assert_eq!(
            destination.host_function_parameter_types["Ordinary"].as_ref(),
            &[C4VType::Int]
        );
        assert_eq!(
            destination.host_function_parameter_types["Reference"].as_ref(),
            &[C4VType::Ref]
        );
        assert_eq!(destination.constants.get("BUILTIN"), Some(&Value::Int(4)));
        assert_eq!(destination.constants.get("CUSTOM"), Some(&Value::Int(50)));
    }

    #[test]
    fn host_registration_snapshot_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<HostRegistrationSnapshot>();
    }

    #[test]
    #[should_panic(expected = "cannot share C4String constants")]
    fn host_registration_snapshots_reject_string_constants() {
        let mut engine = Engine::new();
        engine.register_constant("TEXT", Value::String("cached".into()));
        let _ = engine.host_registration_snapshot();
    }

    #[test]
    fn renaming_a_script_host_updates_only_its_function_source_names() {
        let mut foreign = Engine::new();
        foreign.set_script_name("Foreign.c4d/Script.c");
        foreign
            .load_script("func Boom() { return 1; }")
            .expect("foreign script compiles");
        let foreign_boom = foreign
            .functions()
            .get("Boom")
            .expect("foreign function exists")
            .clone();

        let mut host = Engine::new();
        host.set_script_name("OLD/Script.c");
        host.load_script("func Boom() { return Missing(); }")
            .expect("host script compiles");
        host.functions
            .get_mut("Boom")
            .expect("host function exists")
            .append_include_overload(foreign_boom);

        host.set_script_name("Scenario.c4s/Objects.c4d/Script.c");
        let boom = host.functions().get("Boom").expect("host function remains");
        assert_eq!(
            boom.source_name(),
            Some("Scenario.c4s/Objects.c4d/Script.c")
        );
        assert_eq!(
            boom.overloaded.as_deref().and_then(Function::source_name),
            Some("Foreign.c4d/Script.c")
        );

        let error = host
            .call("Boom", &[])
            .expect_err("runtime error is captured");
        assert_eq!(
            error.call_frames()[0].source_name(),
            Some("Scenario.c4s/Objects.c4d/Script.c")
        );
    }

    #[test]
    fn runtime_call_frames_preserve_references_and_trim_nil_arguments() {
        let mut engine = Engine::new();
        engine
            .load_script(
                "#strict 3\n\
                 func Fail(first, &second, third, &fourth) { return Missing(); }\n\
                 func Run() { var value = 42; var trailing = nil; return Fail(nil, value, nil, trailing); }",
            )
            .expect("diagnostic script compiles");

        let error = engine.call("Run", &[]).expect_err("Missing fails");
        let fail = error
            .call_frames()
            .iter()
            .find(|frame| frame.function() == "Fail")
            .unwrap_or_else(|| panic!("Fail remains in the captured call stack: {error:?}"));
        assert_eq!(fail.arguments(), "nil,42*,nil,nil*");
    }

    #[test]
    fn ordinary_entries_skip_unnamed_global_links_and_use_engine_scope() {
        let mut declaring = Engine::new();
        declaring
            .load_script(
                "global func Pick() { return 1; }\n\
                 global func Queue() { return Pick(); }\n\
                 func LocalQueue() { return Pick(); }",
            )
            .expect("declaring functions compile");
        let mut functions = declaring
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<FxHashMap<_, _>>();

        let mut later = Engine::new();
        later
            .load_script("global func Pick() { return 2; }")
            .expect("later overload compiles");
        let mut latest = later.functions().get("Pick").expect("Pick exists").clone();
        latest.push_overload(functions.remove("Pick").expect("old Pick exists"));
        functions.insert("Pick".to_string(), latest);
        declaring.set_global_functions(Some(Arc::new(functions)));

        assert_eq!(
            declaring.call("Pick", &[]).expect("global entry resolves"),
            Value::Int(2)
        );
        assert_eq!(
            declaring
                .call("Queue", &[])
                .expect("global helper resolves"),
            Value::Int(2)
        );
        assert_eq!(
            declaring
                .call("LocalQueue", &[])
                .expect("local global fallback resolves"),
            Value::Int(2)
        );
    }

    #[test]
    fn definition_scope_inherited_hops_to_the_live_engine_global_table() {
        // `GetOverloadedFunc`'s owner hop is a LIVE table lookup, not a walk of
        // the caller's own overload chain. A definition script's Owner IS the
        // script engine (C4Def.cpp:649 `Script.Reg2List(&Game.ScriptEngine,
        // &Game.ScriptEngine)`), so `if (!f && Owner) { f =
        // Owner->GetFuncRecursive(ByFunc->Name); }` (C4Aul.cpp:281-288) reads
        // the engine's function map (C4Aul.cpp:293-301). Same-name entries are
        // head-inserted there — the `C4AulFunc` constructor passes its
        // `bAtEnd` default of true into `FuncLookUp.Add(this, bAtEnd)`
        // (C4Aul.cpp:76-79), which `C4AulFuncMap::Add` receives as `bAtStart`
        // (C4Aul.cpp:586-628) — so `GetFirstFunc` (C4Aul.cpp:553-560) yields
        // the NEWEST global from ANY host. A definition that declares no global
        // of its own therefore still reaches one.
        let mut provider = Engine::new();
        provider
            .load_script("global func Pick() { return 1; }")
            .expect("first global compiles");
        let mut functions = provider
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<FxHashMap<_, _>>();

        let mut later = Engine::new();
        later
            .load_script("global func Pick() { return 2; }")
            .expect("newer global compiles");
        let mut latest = later.functions().get("Pick").expect("Pick exists").clone();
        latest.push_overload(functions.remove("Pick").expect("older Pick exists"));
        functions.insert("Pick".to_string(), latest);

        // The definition declares no global at all, so nothing in its own
        // overload chain can supply the target.
        let mut definition = Engine::new();
        definition
            .load_script("#strict\nfunc Pick() { return inherited() + 10; }")
            .expect("definition-scope declaration compiles");
        definition.set_global_functions(Some(Arc::new(functions)));

        assert_eq!(
            definition.call("Pick", &[]).expect("call succeeds"),
            Value::Int(12),
            "the owner hop reaches the newest engine global, not the older one"
        );
    }

    #[test]
    fn engine_hop_inherited_result_copies_like_a_chain_target_call() {
        // The hop dispatches a script function, so its result must carry the
        // same C4Value::Set semantics as the equivalent chain-target call — an
        // array result is copied, not aliased (C4AulExec.cpp:330-337). This is
        // an equivalence guard, not a discriminator: the two paths agree today
        // whichever target `direct_value_call_has_materialized_result` picks,
        // and it exists so they keep agreeing once one of them changes.
        fn tail_mutates_source(engine: &Engine) -> Value {
            engine.call("Probe", &[]).expect("call succeeds")
        }

        let mut provider = Engine::new();
        provider
            .load_script("#strict\nglobal func Make() { return [1, 2]; }")
            .expect("global provider compiles");
        let globals = provider
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<FxHashMap<_, _>>();

        // Chain target: the same-host earlier declaration supplies `inherited`.
        let mut chained = Engine::new();
        chained
            .load_script(
                "#strict\n\
                 func Make() { return [1, 2]; }\n\
                 func Make() { var a = inherited(); a[0] = 9; return [a, inherited()]; }\n\
                 func Probe() { return Make(); }",
            )
            .expect("chained declaration compiles");

        // Engine hop: the definition declares no `Make` of its own to inherit.
        let mut hopped = Engine::new();
        hopped
            .load_script(
                "#strict\n\
                 func Make() { var a = inherited(); a[0] = 9; return [a, inherited()]; }\n\
                 func Probe() { return Make(); }",
            )
            .expect("hopping declaration compiles");
        hopped.set_global_functions(Some(Arc::new(globals)));

        let expected = Value::Array(vec![
            Value::Array(vec![Value::Int(9), Value::Int(2)]),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
        ]);
        assert_eq!(tail_mutates_source(&chained), expected);
        assert_eq!(
            tail_mutates_source(&hopped),
            expected,
            "the hopped target's result copies exactly like the chain target's"
        );
    }

    #[test]
    fn link_reports_a_hard_inherited_with_no_overload_target() {
        // C4Aul binds `inherited` after every func table is built
        // (C4AulParse.cpp:1406) and throws "inherited function not found, use
        // _inherited to call failsafe" when there is no OwnerOverloaded
        // (C4AulParse.cpp:2799); C4AulScript::Parse reports and counts it
        // (C4AulParse.cpp:3563-3586) rather than deferring to the first call.
        let mut orphan = Engine::new();
        orphan
            .load_script("#strict\nfunc Orphan() { return inherited(); }")
            .expect("script compiles");
        let diagnostics = orphan.unresolved_inherited_diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].function, "Orphan");
        assert_eq!(diagnostics[0].line, 2);

        // The failsafe spelling is silent — C4Aul only raises for the hard
        // one, and discards the parameters instead (C4AulParse.cpp:2801-2806).
        let mut failsafe = Engine::new();
        failsafe
            .load_script("#strict\nfunc Safe() { return _inherited(); }")
            .expect("script compiles");
        assert!(failsafe.unresolved_inherited_diagnostics().is_empty());

        // So is a resolvable one.
        let mut chained = Engine::new();
        chained
            .load_script("func Base() { return 1; }")
            .expect("base compiles");
        chained
            .load_script("#strict\nfunc Base() { return inherited() + 1; }")
            .expect("overload compiles");
        assert!(chained.unresolved_inherited_diagnostics().is_empty());
    }

    #[test]
    fn link_does_not_report_an_inherited_that_resolves_off_the_chain() {
        // The oracle has to be the VM's, not a chain walk. Both of these
        // resolve at run time — one through C4Aul's owner hop into the engine
        // table (C4Aul.cpp:281-288), one through the same-name native that
        // overloading a host function reaches — so neither may be reported.
        // A chain-only oracle would report both, and shipped content carries
        // ~99 hard `inherited()` sites that lean on exactly these two routes.
        let mut provider = Engine::new();
        provider
            .load_script("global func Hop() { return 1; }")
            .expect("global compiles");
        let globals = provider
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<FxHashMap<_, _>>();

        let mut hopping = Engine::new();
        hopping
            .load_script("#strict\nfunc Hop() { return inherited() + 10; }")
            .expect("definition compiles");
        hopping.set_global_functions(Some(Arc::new(globals)));
        assert!(
            hopping.unresolved_inherited_diagnostics().is_empty(),
            "the engine hop resolves it: {:?}",
            hopping.unresolved_inherited_diagnostics()
        );
        assert_eq!(
            hopping.call("Hop", &[]).expect("call succeeds"),
            Value::Int(11)
        );

        let mut native = Engine::new();
        native.register_host_function("Native", |_| Ok(Value::Int(5)));
        native
            .load_script("#strict\nfunc Native() { return inherited() + 1; }")
            .expect("override compiles");
        assert!(
            native.unresolved_inherited_diagnostics().is_empty(),
            "the same-name native resolves it"
        );
        assert_eq!(
            native.call("Native", &[]).expect("call succeeds"),
            Value::Int(6)
        );
    }

    #[test]
    fn named_local_survives_a_newer_own_global_in_the_overload_chain() {
        let mut host = Engine::new();
        host.load_script("func Pick() { return 1; }")
            .expect("local declaration compiles");
        host.load_script("global func Pick() { return 2; }")
            .expect("same-name global declaration compiles");
        host.load_script("global func Queue() { return Pick(); }")
            .expect("global caller compiles");
        let globals = host
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<FxHashMap<_, _>>();
        host.set_global_functions(Some(Arc::new(globals)));

        assert!(host.has_local_function("Pick"));
        let resolution = host
            .resolve_function("Pick", false)
            .expect("native local lookup resolves");
        assert_eq!(resolution.scope, ScriptFunctionScope::Local);
        assert_eq!(resolution.host_identity, host.host_identity());
        assert_eq!(resolution.function.access, crate::ast::AccessLevel::Public);
        assert_eq!(
            host.call("Pick", &[]).expect("ordinary own root resolves"),
            Value::Int(1)
        );
        // The global caller's body is a different question from the named
        // local lookup above. An engine-owned function switches its WHOLE
        // identifier lookup to the engine — `if (Fn->Owner ==
        // &Game.ScriptEngine) FoundFn = a->Owner->GetFuncRecursive(Idtf);`
        // (C4AulParse.cpp:2216-2219 on the statement path, :2818-2823 on the
        // expression path), where `a->Owner` is the script engine
        // (C4Def.cpp:649) — so `Queue` reads the engine table's `Pick`, not
        // the definition-scope one declared beside it. `GetLocalSFunc`'s
        // "search linked scope first" (C4Aul.cpp:118-127) does not apply: its
        // only callers are FnResortObjects/FnResortObject (C4Script.cpp:4491,
        // :4512), a by-name runtime lookup rather than body resolution.
        assert_eq!(
            host.call_global_with_ref_args("Queue", &[])
                .expect("exact global callback resolves")
                .0,
            Value::Int(2),
            "the global body resolves against the engine, not its LinkedTo host"
        );
    }

    #[test]
    fn repeated_function_resolution_retains_the_same_function_snapshot() {
        // C4Aul lookup returns the installed C4AulFunc pointer, and deferred
        // effect dispatch retains that pointer rather than copying its body
        // on every lookup (C4Aul.cpp:118-127; C4Effect.cpp:42-56).
        let mut host = Engine::new();
        host.load_script("func Pulse() { return 1; }")
            .expect("function compiles");

        let first = host
            .resolve_function("Pulse", false)
            .expect("first lookup resolves");
        let second = host
            .resolve_function("Pulse", false)
            .expect("second lookup resolves");

        assert!(Arc::ptr_eq(&first.function, &second.function));
    }

    #[test]
    fn resolved_function_call_trusts_its_installed_compiled_plan() {
        // C4AulScriptFunc::Exec executes the already-resolved function's Code
        // pointer directly (C4AulExec.cpp:330-363,1629-1635); it does not
        // compare that code back to the parsed body on every callback.
        let mut host = Engine::new();
        host.load_script("func Pulse() { return 1; }")
            .expect("function compiles");
        let resolution = host
            .resolve_function("Pulse", false)
            .expect("function resolves");

        crate::vm::reset_compiled_source_validations();
        let value = host
            .call_resolved_with_ref_args(&resolution, false, &[])
            .expect("resolved callback executes")
            .0;

        assert_eq!(value, Value::Int(1));
        assert_eq!(crate::vm::compiled_source_validations(), 0);
    }

    #[test]
    fn mutated_resolved_function_snapshot_falls_back_to_source_validation() {
        let mut host = Engine::new();
        host.load_script("func Pulse() { return 1; }")
            .expect("function compiles");
        let mut resolution = host
            .resolve_function("Pulse", false)
            .expect("function resolves");
        host.call_resolved_with_ref_args(&resolution, false, &[])
            .expect("original snapshot warms its plan");

        let replacement = Parser::new("func Pulse() { return 2; }")
            .parse_script_strict()
            .expect("replacement parses")
            .functions
            .into_iter()
            .next()
            .expect("replacement function exists");
        Arc::make_mut(&mut resolution.function).body = replacement.body;

        crate::vm::reset_compiled_source_validations();
        let value = host
            .call_resolved_with_ref_args(&resolution, false, &[])
            .expect("mutated snapshot executes")
            .0;

        assert_eq!(value, Value::Int(2));
        assert_eq!(crate::vm::compiled_source_validations(), 1);
    }

    #[test]
    fn include_relink_invalidates_the_resolved_function_snapshot() {
        // Include linking appends the parent to OwnerOverloaded. A function
        // pointer retained before that link stays immutable, while later
        // lookups see the newly linked node (C4AulLink.cpp:113-141;
        // C4AulParse.cpp:1404-1408).
        let mut child = Engine::new();
        child
            .load_script("#strict 3\nfunc Pulse() { return inherited() + 1; }")
            .expect("child function compiles");
        let before = child
            .resolve_function("Pulse", false)
            .expect("pre-link lookup resolves");

        let mut parent = Engine::new();
        parent
            .load_script("#strict 3\nfunc Pulse() { return 41; }")
            .expect("parent function compiles");
        child.merge_from(&parent);

        let after = child
            .resolve_function("Pulse", false)
            .expect("post-link lookup resolves");
        assert!(!Arc::ptr_eq(&before.function, &after.function));
        crate::vm::reset_compiled_source_validations();
        assert_eq!(
            child
                .call_resolved_with_ref_args(&after, false, &[])
                .expect("new snapshot carries the inherited target")
                .0,
            Value::Int(42)
        );
        // The newly resolved entry itself is trusted. Its inherited target is
        // still an ordinary Function edge and therefore performs the one
        // validation counted here.
        assert_eq!(crate::vm::compiled_source_validations(), 1);
    }

    #[test]
    fn exact_global_ref_entry_skips_a_same_name_local_without_a_shared_table() {
        let mut host = Engine::new();
        host.load_script("global func Pick(&slot) { slot = 7; return 70; }")
            .expect("global declaration compiles");
        host.load_script("func Pick(&slot) { slot = 1; return 10; }")
            .expect("same-name local declaration compiles");
        host.register_host_function("NativeOnly", |args| {
            Ok(Value::Int(
                args.first().and_then(Value::as_c4_int).unwrap_or(0) + 1,
            ))
        });

        let (ordinary, ordinary_args) = host
            .call_with_ref_args("Pick", &[Value::Int(0)])
            .expect("ordinary local entry resolves");
        assert_eq!(ordinary, Value::Int(10));
        assert_eq!(ordinary_args, vec![Value::Int(1)]);

        let (global, global_args) = host
            .call_global_with_ref_args("Pick", &[Value::Int(0)])
            .expect("exact global entry resolves");
        assert_eq!(global, Value::Int(70));
        assert_eq!(global_args, vec![Value::Int(7)]);

        let (native, native_args) = host
            .call_global_with_ref_args("NativeOnly", &[Value::Int(40)])
            .expect("host fallback resolves");
        assert_eq!(native, Value::Int(41));
        assert_eq!(native_args, vec![Value::Int(40)]);
    }

    #[test]
    fn pinned_global_callback_keeps_this_and_resolves_its_helper_in_the_engine() {
        // A pinned engine-owned callback keeps the supplied `this`, but its
        // body resolves identifiers in the ENGINE table, not in the host it is
        // linked to: `if (Fn->Owner == &Game.ScriptEngine) FoundFn =
        // Fn->Owner->GetFuncRecursive(Idtf);` (C4AulParse.cpp:2818-2823). The
        // declaring host's definition-scope `Helper` is therefore invisible
        // here, and the engine table's `global func Helper` wins.
        let mut linked_host = Engine::new();
        linked_host
            .load_script(
                "#strict\n\
                 func Helper() { return 4; }\n\
                 global func Deferred() { return [this, Helper()]; }",
            )
            .expect("linked-host script compiles");

        let mut destination = Engine::new();
        destination
            .load_script("global func Helper() { return 999; }")
            .expect("engine-table helper compiles");

        let mut globals = linked_host
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<FxHashMap<_, _>>();
        globals.extend(
            destination
                .global_access_functions()
                .map(|(name, function)| (name.clone(), function.clone())),
        );
        linked_host.set_global_functions(Some(Arc::new(globals)));
        let pinned = linked_host
            .resolve_global_function("Deferred")
            .expect("global callback resolves");
        let cells = crate::vm::LocalCells::default();

        let value = linked_host
            .call_resolved_with_cells_and_this(&pinned, true, &[], &cells, Value::Object(42))
            .expect("pinned callback runs");

        assert_eq!(
            value,
            Value::Array(vec![Value::Object(42), Value::Int(999)]),
            "`this` survives the pin; the helper comes from the engine table"
        );
    }

    #[test]
    fn pinned_definition_callback_round_trips_its_local_cells() {
        // The other half of the split: object `local`s are legal only in a
        // definition-scope function — C4Aul rejects a `local` read or write
        // inside an engine-owned body outright ("using local variable in
        // global function!", C4AulParse.cpp:2000-2004 for the lvalue path and
        // :2731-2737 for the rvalue path). This pins the supplied cells
        // round-tripping through a pinned callback that may legally use them.
        let mut host = Engine::new();
        host.load_script(
            "#strict\n\
             local Shared;\n\
             func Deferred() { Shared = Shared + 2; return [this, Shared]; }",
        )
        .expect("definition-scope script compiles");

        let pinned = host
            .resolve_function("Deferred", false)
            .expect("definition callback resolves");
        let cells = crate::vm::LocalCells::from_local_vars(&HashMap::from([(
            "Shared".to_string(),
            Value::Int(5),
        )]));

        let value = host
            .call_resolved_with_cells_and_this(&pinned, false, &[], &cells, Value::Object(42))
            .expect("pinned callback runs");

        assert_eq!(value, Value::Array(vec![Value::Object(42), Value::Int(7)]));
        assert_eq!(cells.snapshot().get("Shared"), Some(&Value::Int(7)));
    }

    #[test]
    fn replace_script_removes_linked_overloads_and_preserves_host_functions() {
        let mut engine = Engine::new();
        engine
            .load_script("func Probe() { return 1; }")
            .expect("base script loads");
        engine
            .load_script("#strict\nfunc Probe() { return inherited() + 1; }")
            .expect("overload script loads");
        engine.register_host_function("Native", |_| Ok(Value::Int(41)));

        assert_eq!(engine.function_count(), 1);
        assert_eq!(engine.linked_function_count(), 2);

        engine.replace_script(
            compile(
                "func Probe() { return Native() + 1; }\n\
                 func Other() { return 7; }",
            ),
            false,
        );

        assert_eq!(engine.function_count(), 2);
        assert_eq!(engine.linked_function_count(), 2);
        assert!(engine
            .functions()
            .get("Probe")
            .is_some_and(|function| function.overloaded.is_none()));
        assert!(engine.has_host_function("Native"));
        assert_eq!(
            engine.call("Probe", &[]).expect("replacement runs"),
            Value::Int(42)
        );
    }

    #[test]
    fn replace_script_preserves_static_cells_and_controls_registration() {
        let globals = new_global_variables();
        let constants = new_global_variables();
        let mut engine = Engine::new();
        engine.set_global_variables(globals.clone());
        engine.set_global_constants(constants.clone());

        engine.replace_script(
            compile(
                "static counter;\n\
                 static const LIMIT = 4;\n\
                 local old_local;\n\
                 func Probe() { return LIMIT; }",
            ),
            true,
        );
        let counter = globals
            .borrow()
            .get("counter")
            .cloned()
            .expect("static registered");
        let limit = constants
            .borrow()
            .get("LIMIT")
            .cloned()
            .expect("constant registered");
        *counter.borrow_mut() = Value::Int(23);
        assert_eq!(*limit.borrow(), Value::Int(4));
        assert_eq!(
            engine.local_variable_names().collect::<Vec<_>>(),
            ["old_local"]
        );

        engine.replace_script(
            compile(
                "static counter, added;\n\
                 static const LIMIT = 8;\n\
                 local fresh_local;\n\
                 func Probe() { return LIMIT; }",
            ),
            true,
        );
        let replacement_counter = globals
            .borrow()
            .get("counter")
            .cloned()
            .expect("static remains registered");
        let replacement_limit = constants
            .borrow()
            .get("LIMIT")
            .cloned()
            .expect("constant remains registered");
        assert!(Rc::ptr_eq(&counter, &replacement_counter));
        assert!(Rc::ptr_eq(&limit, &replacement_limit));
        assert_eq!(*replacement_counter.borrow(), Value::Int(23));
        assert_eq!(*replacement_limit.borrow(), Value::Int(8));
        assert_eq!(
            engine.local_variable_names().collect::<Vec<_>>(),
            ["fresh_local"]
        );

        engine.replace_script(
            compile(
                "static counter, skipped;\n\
                 static const LIMIT = 99;\n\
                 local final_local;\n\
                 func Probe() { return LIMIT; }",
            ),
            false,
        );
        assert!(!globals.borrow().contains_key("skipped"));
        assert_eq!(*replacement_counter.borrow(), Value::Int(23));
        assert_eq!(*replacement_limit.borrow(), Value::Int(8));
        assert_eq!(
            engine.local_variable_names().collect::<Vec<_>>(),
            ["final_local"]
        );
    }

    #[test]
    fn global_names_and_strings_keep_native_link_and_construction_order() {
        let globals = new_global_variables();
        let strings = new_string_registrations();
        let mut engine = Engine::new();
        engine.set_global_variables(globals.clone());
        engine.set_string_registrations(strings.clone());
        engine
            .load_script(
                "static Zed, Alpha;\n\
                 func Build() { return \"zeta\" .. \"alpha\"; }",
            )
            .expect("script loads");

        assert_eq!(
            globals
                .borrow()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["Zed", "Alpha"]
        );
        assert_eq!(
            c4_string_registration_order(&strings),
            ["zeta".to_string(), "alpha".to_string()]
        );
        let built = engine.call("Build", &[]).expect("string concatenates");
        assert_eq!(built, Value::from("zetaalpha"));
        assert_eq!(
            c4_string_registration_order(&strings),
            [
                "zeta".to_string(),
                "alpha".to_string(),
                "zetaalpha".to_string(),
            ]
        );
        drop(built);
    }

    #[test]
    fn immediate_script_install_registers_string_constants_before_body_holds() {
        let source = "static const LABEL = \"constant\";\n\
                      func Read() { return \"body\"; }";

        for replace in [false, true] {
            let globals = new_global_variables();
            let constants = new_global_variables();
            let strings = new_string_registrations();
            let mut engine = Engine::new();
            engine.set_global_variables(globals);
            engine.set_global_constants(constants.clone());
            engine.set_string_registrations(strings.clone());
            if replace {
                engine.replace_script(compile(source), true);
            } else {
                engine.add_script(compile(source));
            }

            assert_eq!(
                c4_string_registration_order(&strings),
                ["constant".to_owned(), "body".to_owned()],
                "preparse constant Refs precede the later function Parse Holds"
            );
            let constant = constants
                .borrow()
                .get("LABEL")
                .cloned()
                .expect("constant registered")
                .borrow()
                .clone();
            let body = engine.call("Read", &[]).expect("body literal evaluates");
            let (Value::String(constant), Value::String(body)) = (constant, body) else {
                panic!("both values remain strings");
            };
            assert_eq!(
                enumerate_c4_strings(&strings, &[constant.clone(), body.clone()]),
                [b"constant".to_vec(), b"body".to_vec()]
            );
            assert_eq!((constant.enum_id(), body.enum_id()), (0, 1));
        }
    }

    #[test]
    fn string_registrations_are_thread_safe_and_deduplicate_atomically() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<StringRegistrations>();

        let strings = new_string_registrations();
        let workers = (0..8)
            .map(|_| {
                let strings = Arc::clone(&strings);
                std::thread::spawn(move || register_c4_literal_string(strings.as_ref(), "shared"))
            })
            .collect::<Vec<_>>();
        let values = workers
            .into_iter()
            .map(|worker| worker.join().expect("string worker completes"))
            .collect::<Vec<_>>();

        assert!(values
            .iter()
            .all(|value| value.ptr_eq(values.first().expect("one string value"))));
        assert_eq!(c4_string_registration_order(&strings), ["shared"]);
    }

    #[test]
    fn parser_literal_lookup_cache_is_weak_and_clear_scoped() {
        // C4AulParse stores the C4String pointer in the emitted AB_STRING
        // operand (C4AulParse.cpp:773-788), so execution must not repeat
        // C4StringTable::FindString's linked-list scan.
        let strings = new_string_registrations();
        let literal = register_c4_literal_string(&strings, "shared");
        let cached = strings
            .borrow()
            .literal_lookup("shared")
            .expect("linked parser literal is cached");
        assert!(literal.ptr_eq(&cached));
        let nul_alias = register_c4_literal_string(&strings, "shared\0suffix");
        assert!(
            literal.ptr_eq(&nul_alias),
            "distinct parser spellings retain C4StringTable's first-NUL identity"
        );
        assert!(strings.borrow().literal_lookup("shared\0suffix").is_some());

        clear_c4_string_holds(&strings);
        assert!(
            strings.borrow().literal_lookup("shared").is_none(),
            "C4StringTable::Clear invalidates parser operand identities"
        );
    }

    #[test]
    fn runtime_string_membership_is_identity_indexed_without_text_interning() {
        // C4StringTable::Reg appends a newly constructed C4String in O(1);
        // repeated C4Value sightings retain that pointer, while equal-text
        // RegString calls remain distinct (C4StringTable.cpp:67-82,159-162).
        let strings = new_string_registrations();
        let first = C4StringValue::from("same");
        let second = C4StringValue::from("same");

        register_c4_string(&strings, &first);
        register_c4_string(&strings, &first);
        register_c4_string(&strings, &second);

        let registrations = strings.borrow();
        assert_eq!(registrations.entries.len(), 2);
        assert_eq!(registrations.registered_identity_count(), 2);
        assert!(!first.ptr_eq(&second));
    }

    #[test]
    fn string_table_preserves_stale_section_enumeration_until_objects_save() {
        let strings = new_string_registrations();
        register_loaded_c4_string(&strings, 0, "loaded");
        let runtime = C4StringValue::from("runtime");
        register_c4_string(&strings, &runtime);

        assert_eq!(
            save_current_c4_string_enumeration(&strings, std::slice::from_ref(&runtime)),
            [b"loaded".to_vec()]
        );
        assert_eq!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&runtime)),
            [b"loaded".to_vec(), b"runtime".to_vec()]
        );
        assert_eq!(
            save_current_c4_string_enumeration(&strings, std::slice::from_ref(&runtime)),
            [b"loaded".to_vec(), b"runtime".to_vec()]
        );
    }

    #[test]
    fn enumeration_uses_live_handle_refcounts_and_save_does_not_reassign_ids() {
        let strings = new_string_registrations();
        let runtime = C4StringValue::from("runtime");
        register_c4_string(&strings, &runtime);
        assert_eq!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&runtime)),
            [b"runtime".to_vec()]
        );
        assert_eq!(runtime.enum_id(), 0);

        assert_eq!(
            save_current_c4_string_enumeration(&strings, &[]),
            [b"runtime".to_vec()],
            "native Save observes C4String::iRefCnt even outside a serializer traversal"
        );
        assert_eq!(runtime.enum_id(), 0, "Save never performs EnumStrings");
    }

    #[test]
    fn dead_runtime_string_rebirth_appends_after_survivors() {
        let strings = new_string_registrations();
        let first = C4StringValue::from("first");
        let survivor = C4StringValue::from("survivor");
        register_c4_string(&strings, &first);
        register_c4_string(&strings, &survivor);
        assert_eq!(
            enumerate_c4_strings(&strings, &[first.clone(), survivor.clone()]),
            [b"first".to_vec(), b"survivor".to_vec()]
        );

        drop(first);
        assert_eq!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&survivor)),
            [b"survivor".to_vec()]
        );
        let reborn = C4StringValue::from("first");
        register_c4_string(&strings, &reborn);
        assert_eq!(
            enumerate_c4_strings(&strings, &[survivor, reborn]),
            [b"survivor".to_vec(), b"first".to_vec()]
        );
    }

    #[test]
    fn duplicate_runtime_identity_controls_surviving_enumeration_order() {
        let strings = new_string_registrations();
        let first_x = C4StringValue::from("x");
        let y = C4StringValue::from("y");
        let second_x = C4StringValue::from("x");
        register_c4_string(&strings, &first_x);
        register_c4_string(&strings, &y);
        register_c4_string(&strings, &second_x);

        assert_eq!(first_x.enum_id(), -1);
        assert_eq!(second_x.enum_id(), -1);
        assert!(!first_x.ptr_eq(&second_x));
        assert_eq!(first_x, second_x, "strict string equality is textual");
        assert_eq!(
            c4_string_registration_order(&strings),
            ["x".to_owned(), "y".to_owned(), "x".to_owned()]
        );
        assert_eq!(
            enumerate_c4_strings(&strings, &[first_x.clone(), y.clone(), second_x.clone()],),
            [b"x".to_vec(), b"y".to_vec()]
        );
        assert_eq!(first_x.enum_id(), 0);
        assert_eq!(second_x.enum_id(), 0);

        drop(first_x);
        assert_eq!(
            enumerate_c4_strings(&strings, &[y.clone(), second_x.clone()]),
            [b"y".to_vec(), b"x".to_vec()]
        );
        assert_eq!(y.enum_id(), 0);
        assert_eq!(second_x.enum_id(), 1);
    }

    #[test]
    fn untouched_loaded_string_dies_after_its_first_resolved_reference() {
        let strings = new_string_registrations();
        register_loaded_c4_string(&strings, 7, "loaded");

        assert_eq!(
            enumerate_c4_strings(&strings, &[]),
            [b"loaded".to_vec()],
            "a newly loaded non-Hold C4String survives at refcount zero"
        );
        let loaded = resolve_c4_string(&strings, 0).expect("S0 resolves the loaded string");
        assert_eq!(loaded.as_ref(), "loaded");
        drop(loaded);

        assert!(
            resolve_c4_string(&strings, 0).is_none(),
            "the final C4Value release deletes a non-Hold loaded string"
        );
        assert!(enumerate_c4_strings(&strings, &[]).is_empty());
        assert!(c4_string_registration_order(&strings).is_empty());
    }

    #[test]
    fn duplicate_loaded_lines_reuse_identity_and_overwrite_the_old_id() {
        let strings = new_string_registrations();
        register_loaded_c4_string(&strings, 0, "same");
        register_loaded_c4_string(&strings, 1, "same");

        assert_eq!(c4_string_registration_order(&strings), ["same".to_owned()]);
        assert!(resolve_c4_string(&strings, 0).is_none());
        let loaded = resolve_c4_string(&strings, 1).expect("the last line ID resolves");
        assert_eq!(loaded.enum_id(), 1);
    }

    #[test]
    fn id_resolution_uses_the_first_registration_when_loaded_ids_collide() {
        let strings = new_string_registrations();
        let earlier = C4StringValue::from("earlier");
        register_c4_string(&strings, &earlier);
        assert_eq!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&earlier)),
            [b"earlier".to_vec()]
        );
        register_loaded_c4_string(&strings, 0, "later");

        let resolved = resolve_c4_string(&strings, 0).expect("one colliding ID resolves");
        assert!(resolved.ptr_eq(&earlier));
    }

    #[test]
    fn literal_reuses_loaded_identity_and_hold_excludes_zero_ref_string() {
        let strings = new_string_registrations();
        register_loaded_c4_string(&strings, 3, "shared");
        let literal = register_c4_literal_string(&strings, "shared");
        let deserialized = resolve_c4_string(&strings, 3).expect("stale S3 still resolves");

        assert!(literal.ptr_eq(&deserialized));
        drop(literal);
        drop(deserialized);
        assert_eq!(
            save_current_c4_string_enumeration(&strings, &[]),
            Vec::<Vec<u8>>::new(),
            "Load does not clear Hold, and held refcount-zero strings are ineligible"
        );
        assert!(enumerate_c4_strings(&strings, &[]).is_empty());
        assert_eq!(
            c4_string_registration_order(&strings),
            ["shared".to_owned()],
            "Hold keeps the parser registration alive despite enum exclusion"
        );
        let referenced = register_c4_literal_string(&strings, "shared");
        assert_eq!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&referenced)),
            [b"shared".to_vec()]
        );
        assert_eq!(referenced.enum_id(), 0);
    }

    #[test]
    fn map_and_property_link_operands_are_held_in_registration_order() {
        let strings = new_string_registrations();
        let mut engine = Engine::new();
        engine.set_string_registrations(strings.clone());
        engine
            .load_script(
                "#strict 3\n\
                 func Probe(target) {\n\
                     var map = { bare = 1 };\n\
                     return [target.dot, target->arrow];\n\
                 }",
            )
            .expect("map and property operands link");

        assert_eq!(
            c4_string_registration_order(&strings),
            ["bare".to_owned(), "dot".to_owned(), "arrow".to_owned()]
        );
        assert!(
            enumerate_c4_strings(&strings, &[]).is_empty(),
            "link operands have Hold but no C4Value reference"
        );
    }

    #[test]
    fn clearing_holds_deletes_unreferenced_and_detaches_referenced_strings() {
        let strings = new_string_registrations();
        let obsolete = register_c4_literal_string(&strings, "obsolete");
        let referenced = register_c4_literal_string(&strings, "shared");
        drop(obsolete);
        assert_eq!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&referenced)),
            [b"shared".to_vec()]
        );

        clear_c4_string_holds(&strings);
        assert!(c4_string_registration_order(&strings).is_empty());
        assert!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&referenced)).is_empty(),
            "a surviving C4Value keeps the C4String alive but not table-registered"
        );
        register_c4_string(&strings, &referenced);
        assert!(
            c4_string_registration_order(&strings).is_empty(),
            "live-value recovery must not reattach an UnReg'd pointer"
        );

        let replacement = register_c4_literal_string(&strings, "shared");
        assert!(!replacement.ptr_eq(&referenced));
        assert_eq!(
            c4_string_registration_order(&strings),
            ["shared".to_owned()]
        );
        assert_eq!(
            enumerate_c4_strings(&strings, std::slice::from_ref(&replacement)),
            [b"shared".to_vec()]
        );
    }

    #[test]
    fn string_table_enumeration_uses_the_first_nul_as_its_identity_boundary() {
        let strings = new_string_registrations();
        let first = C4StringValue::from("shared\0first suffix");
        let second = C4StringValue::from("shared\0second suffix");
        register_c4_string(&strings, &first);
        register_c4_string(&strings, &second);

        assert_eq!(
            enumerate_c4_strings(&strings, &[first.clone(), second.clone()]),
            [b"shared\0first suffix".to_vec()],
            "FindSaveString gives both registrations the first live prefix-equivalent ID"
        );
        assert!(!first.ptr_eq(&second));
        assert_ne!(first, second, "strict equality remains full-length");
        assert_eq!(first.enum_id(), second.enum_id());
        assert_eq!(
            save_current_c4_string_enumeration(&strings, &[first, second]),
            [b"shared\0first suffix".to_vec()]
        );
    }
}
