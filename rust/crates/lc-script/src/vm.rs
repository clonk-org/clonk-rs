use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use crate::ast::{
    AssignmentTarget, BinaryOp, Expr, ForInit, Function, Parameter, Stmt, UnaryOp, VarDecl,
};
use crate::debugger::DebuggerHooks;
use crate::engine::HostFunction;
use crate::error::RuntimeError;
use crate::value::{Literal, Value};

/// Maximum script call-stack depth, matching C++ `MAX_CONTEXT_STACK`
/// (C4AulExec.cpp:62). A script recursing within this bound runs; beyond it the
/// VM returns a clean error (C++ throws "call stack overflow", :143-145).
const MAX_CALL_DEPTH: usize = 512;
/// C4AUL_MAX_Par: every C4Aul call frame carries exactly 10 parameter slots
/// (C4Aul.h); `Par(n)` beyond them reads nil and `F(...)` forwards at most
/// this many.
const MAX_CALL_PARAMETERS: usize = 10;
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

#[derive(Clone)]
enum Binding {
    Direct(ValueCell),
    Reference(LValueRef),
}

impl Binding {
    fn direct(value: Value) -> Self {
        Binding::Direct(value_cell(value))
    }

    fn read(&self) -> Result<Value, RuntimeError> {
        match self {
            Binding::Direct(cell) => Ok(cell.borrow().clone()),
            Binding::Reference(reference) => reference.read(),
        }
    }

    fn write(&self, value: Value) -> Result<(), RuntimeError> {
        match self {
            Binding::Direct(cell) => {
                *cell.borrow_mut() = value;
                Ok(())
            }
            Binding::Reference(reference) => reference.write(value),
        }
    }

    fn lvalue(&self) -> LValueRef {
        match self {
            Binding::Direct(cell) => LValueRef::Cell(cell.clone()),
            Binding::Reference(reference) => reference.clone(),
        }
    }
}

#[derive(Clone)]
pub(crate) enum LValueRef {
    Cell(ValueCell),
    Path {
        root: ValueCell,
        segments: Vec<PathSegment>,
    },
}

impl LValueRef {
    fn read(&self) -> Result<Value, RuntimeError> {
        match self {
            LValueRef::Cell(cell) => Ok(cell.borrow().clone()),
            LValueRef::Path { root, segments } => read_path(&root.borrow(), segments),
        }
    }

    fn write(&self, value: Value) -> Result<(), RuntimeError> {
        match self {
            LValueRef::Cell(cell) => {
                *cell.borrow_mut() = value;
                Ok(())
            }
            LValueRef::Path { root, segments } => {
                write_path(&mut root.borrow_mut(), segments, value)
            }
        }
    }

    fn append(&self, segment: PathSegment) -> Self {
        match self {
            LValueRef::Cell(root) => LValueRef::Path {
                root: root.clone(),
                segments: vec![segment],
            },
            LValueRef::Path { root, segments } => {
                let mut segments = segments.clone();
                segments.push(segment);
                LValueRef::Path {
                    root: root.clone(),
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
            (PathSegment::Index(Value::Int(raw_index)), Value::Array(elements)) => {
                if *raw_index < 0 {
                    return Err(RuntimeError::new("array index cannot be negative"));
                }
                elements
                    .get(*raw_index as usize)
                    .cloned()
                    .unwrap_or(Value::Nil)
            }
            (PathSegment::Index(Value::String(key)), Value::Proplist(entries)) => {
                entries.get(key).cloned().unwrap_or(Value::Nil)
            }
            (PathSegment::Index(index), Value::Proplist(_)) => {
                return Err(RuntimeError::new(format!(
                    "proplist keys must be strings, got {}",
                    index.type_name()
                )))
            }
            (PathSegment::Index(index), Value::Array(_)) => {
                return Err(RuntimeError::new(format!(
                    "array index must be an integer, got {}",
                    index.type_name()
                )))
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
        *value = new_value;
        return Ok(());
    };

    match segment {
        PathSegment::Property(property) => {
            let Value::Proplist(entries) = value else {
                return Err(RuntimeError::new(format!(
                    "cannot assign property '{property}' on value of type {}",
                    value.type_name()
                )));
            };
            if rest.is_empty() {
                entries.insert(property.clone(), new_value);
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
        PathSegment::Index(Value::Int(raw_index)) => {
            if *raw_index < 0 {
                return Err(RuntimeError::new("array index cannot be negative"));
            }
            let Value::Array(elements) = value else {
                return Err(RuntimeError::new(format!(
                    "cannot index into value of type {}",
                    value.type_name()
                )));
            };
            let index = *raw_index as usize;
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
        PathSegment::Index(Value::String(key)) => {
            let Value::Proplist(entries) = value else {
                return Err(RuntimeError::new(format!(
                    "cannot index into value of type {}",
                    value.type_name()
                )));
            };
            if rest.is_empty() {
                entries.insert(key.clone(), new_value);
                Ok(())
            } else {
                let Some(next) = entries.get_mut(key) else {
                    return Err(RuntimeError::new(format!(
                        "cannot access property '{key}' on nil"
                    )));
                };
                write_path(next, rest, new_value)
            }
        }
        PathSegment::Index(index) => Err(RuntimeError::new(format!(
            "array index must be an integer, got {}",
            index.type_name()
        ))),
    }
}

pub(crate) enum CallArg {
    Value(Value),
    Reference(LValueRef),
}

impl CallArg {
    fn read(&self) -> Result<Value, RuntimeError> {
        match self {
            CallArg::Value(value) => Ok(value.clone()),
            CallArg::Reference(reference) => reference.read(),
        }
    }
}

enum ReturnValue {
    Value(Value),
    Reference(LValueRef),
}

impl ReturnValue {
    fn into_value(self) -> Result<Value, RuntimeError> {
        match self {
            ReturnValue::Value(value) => Ok(value),
            ReturnValue::Reference(reference) => reference.read(),
        }
    }

    fn as_value(&self) -> Result<Value, RuntimeError> {
        match self {
            ReturnValue::Value(value) => Ok(value.clone()),
            ReturnValue::Reference(reference) => reference.read(),
        }
    }
}

pub struct Vm<'a> {
    functions: &'a HashMap<String, Function>,
    host_functions: &'a HashMap<String, HostFunction>,
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
            var_decls,
            debugger,
            constants: None,
            global_functions: None,
            this_value: Value::Nil,
            method_dispatch: None,
            globals_named: None,
            globals_numbered: None,
            globals_consts: None,
            local_cell_hook: None,
        }
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
        let args = args.iter().cloned().map(CallArg::Value).collect();
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
        let args = args.iter().cloned().map(CallArg::Value).collect();
        self.invoke_value(name, args, 0, cells.state.clone(), None)
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
        let args = args.iter().cloned().map(CallArg::Value).collect();
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
        let Ok(expr) = crate::parser::Parser::new(source).parse_direct_exec_expression() else {
            return Ok((Value::Nil, object_state.to_local_vars(self.var_decls)));
        };
        let mut env = Environment::new_with_params(&[], &[], strict_level, object_state.clone())?;
        for var_decl in self.var_decls {
            env.define_object_local(&var_decl.name);
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
        let Ok(expr) = crate::parser::Parser::new(source).parse_direct_exec_expression() else {
            return Ok(Value::Nil);
        };
        let mut env =
            Environment::new_with_params(&[], &[], strict_level, cells.state.clone())?;
        for var_decl in self.var_decls {
            env.define_object_local(&var_decl.name);
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
            args.push(CallArg::Value(Value::Nil));
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

        // Script-level `local` declarations are object-local storage. Nested
        // calls share the same object state, matching C++ pObj->Local/LocalNamed.
        for var_decl in self.var_decls {
            env.define_object_local(&var_decl.name);
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
            ControlFlow::Normal => ReturnValue::Value(Value::Nil),
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
            Stmt::VarDecl { name, init } => {
                let value = match init {
                    Some(expr) => self.evaluate(expr, env, depth)?,
                    None => Value::Nil,
                };
                // Vars are FUNCTION-scoped in C4Aul: the hoisted slot
                // (declared at function entry) receives the value — a
                // `var` inside a block must not shadow it.
                if env.assign(name, value.clone()).is_err() {
                    env.define(name, value);
                }
                Ok(ControlFlow::Normal)
            }
            Stmt::Assignment { target, value } => {
                let evaluated = self.evaluate(value, env, depth)?;
                self.assign_target(env, target, evaluated)?;
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
                        Some(expr) => self.evaluate(expr, env, depth)?,
                        None => Value::Nil,
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
                                let value = match init_expr {
                                    Some(expr) => self.evaluate(expr, env, depth)?,
                                    None => Value::Nil,
                                };
                                env.define(name, value);
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
                declare_var,
                iterable,
                body,
            } => {
                // Evaluate the iterable expression
                let iterable_value = self.evaluate(iterable, env, depth)?;

                // Extract the collection to iterate over
                let items = match &iterable_value {
                    Value::Array(arr) => arr.clone(),
                    // For non-arrays, treat as empty iteration (matches C++ behavior)
                    _ => Vec::new(),
                };

                // Iterate over each item
                for item in items {
                    // Assign the item to the iteration variable
                    if *declare_var {
                        // Define new variable (or redefine if in same scope)
                        env.define(variable, item);
                    } else {
                        // Assign to existing variable
                        env.assign(variable, item)?;
                    }

                    // Execute body
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
        self.globals_consts.and_then(|table| {
            let cell = table.borrow().get(name).cloned();
            cell.map(|cell| cell.borrow().clone())
        })
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
                self.eval_binary(left, op, right, env.strict_level)
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
                            && !self.host_functions.contains_key(name)
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
                            && !self.host_functions.contains_key(name)
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
                        // numbered Local slot, returns the value; the object
                        // defaults to the caller. An explicit FOREIGN target
                        // still writes the caller's slot (numbered locals are
                        // not on the cross-object cell hook yet — PORT_STATUS).
                        if name == "SetLocal"
                            && (1..=3).contains(&args.len())
                            && !self.functions.contains_key(name)
                            && !self.host_functions.contains_key(name)
                        {
                            let index_expr = Box::new(
                                args.first()
                                    .cloned()
                                    .unwrap_or(Expr::Literal(Literal::Int(0))),
                            );
                            let value = args
                                .get(1)
                                .map(|arg| self.evaluate(arg, env, depth + 1))
                                .transpose()?
                                .unwrap_or(Value::Nil);
                            let index =
                                self.evaluate_slot_index("SetLocal()", &index_expr, env, depth)?;
                            if let LValueRef::Cell(cell) = env.local_slot_lvalue(index) {
                                *cell.borrow_mut() = value.clone();
                            }
                            return Ok(value);
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
                            && !self.host_functions.contains_key(name)
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
                            && !self.host_functions.contains_key(name)
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
                            && !self.host_functions.contains_key(name)
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
                            let Ok(expr) = crate::parser::Parser::new(&code)
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
                                exec_env.define_object_local(&var_decl.name);
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
                            && !self.host_functions.contains_key(name)
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
                            return Ok(usize::try_from(index)
                                .ok()
                                .filter(|index| *index < MAX_CALL_PARAMETERS)
                                .and_then(|index| env.call_args.get(index).cloned())
                                .unwrap_or(Value::Nil));
                        }
                    }
                    // Extract function name from callee expression
                    match callee.as_ref() {
                        Expr::Variable(name) if name == "inherited" || name == "_inherited" => {
                            // `inherited` calls the overloaded function; the
                            // `_inherited` spelling yields nil when there is
                            // none (C4AulParse.cpp:2775-2798).
                            let Some(target) = env.inherited_target.clone() else {
                                // Script functions overload same-name ENGINE
                                // functions: inherited() chains to the host
                                // fn (C4Aul OwnerOverloaded includes engine
                                // funcs — GoldRush AI.c4d's global
                                // GetOwner/Hostile overrides rely on it).
                                if let Some(host) =
                                    self.host_functions.get(&env.function_name)
                                {
                                    let mut evaluated_args =
                                        self.build_call_args(None, args, env, depth + 1)?;
                                    if *forward_rest {
                                        Self::append_forwarded_args(&mut evaluated_args, env);
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
                                self.build_call_args(Some(&target), args, env, depth + 1)?;
                            if *forward_rest {
                                Self::append_forwarded_args(&mut evaluated_args, env);
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
                                && !self.host_functions.contains_key(name)
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
                            let function = self.functions.get(name);
                            let mut evaluated_args =
                                self.build_call_args(function, args, env, depth + 1)?;
                            if *forward_rest {
                                Self::append_forwarded_args(&mut evaluated_args, env);
                            }
                            self.invoke_value(
                                name,
                                evaluated_args,
                                depth + 1,
                                env.object_state.clone(),
                                Some(env.var_slots.clone()),
                            )
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
                let mut map = HashMap::with_capacity(entries.len());
                for (key, expr) in entries {
                    let value = self.evaluate(expr, env, depth)?;
                    map.insert(key.clone(), value);
                }
                Ok(Value::Proplist(map))
            }
            Expr::Index(target, index) => {
                let collection = self.evaluate(target, env, depth)?;
                let idx = self.evaluate(index, env, depth)?;
                self.eval_index(collection, idx)
            }
            Expr::Property(target, name) => {
                let proplist = self.evaluate(target, env, depth)?;
                self.eval_property(proplist, name)
            }
            Expr::Assignment(target, value_expr) => {
                // Evaluate the value first
                let value = self.evaluate(value_expr, env, depth)?;
                // Assign to target
                self.assign_target(env, target, value.clone())?;
                // Return the assigned value (assignment is an expression)
                Ok(value)
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
                let target = Self::expr_to_assignment_target(expr)?;
                let old_value = self.get_target_value(env, &target)?;
                let new_value = Value::Int(Self::counter_operand(old_value, "increment")? + 1);
                self.assign_target(env, &target, new_value.clone())?;
                Ok(new_value)
            }
            Expr::PreDecrement(expr) => {
                let target = Self::expr_to_assignment_target(expr)?;
                let old_value = self.get_target_value(env, &target)?;
                let new_value = Value::Int(Self::counter_operand(old_value, "decrement")? - 1);
                self.assign_target(env, &target, new_value.clone())?;
                Ok(new_value)
            }
            Expr::PostIncrement(expr) => {
                let target = Self::expr_to_assignment_target(expr)?;
                let old_value =
                    Self::counter_operand(self.get_target_value(env, &target)?, "increment")?;
                self.assign_target(env, &target, Value::Int(old_value + 1))?;
                Ok(Value::Int(old_value))
            }
            Expr::PostDecrement(expr) => {
                let target = Self::expr_to_assignment_target(expr)?;
                let old_value =
                    Self::counter_operand(self.get_target_value(env, &target)?, "decrement")?;
                self.assign_target(env, &target, Value::Int(old_value - 1))?;
                Ok(Value::Int(old_value))
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
    ) -> Result<Value, RuntimeError> {
        use BinaryOp::*;
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
            Mul => self.eval_int_op(left, right, |a, b| a * b, "*"),
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
            Equal => Ok(Value::Bool(self.values_equal(&left, &right, strict))),
            NotEqual => Ok(Value::Bool(!self.values_equal(&left, &right, strict))),
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
            StringEqual => self.eval_string_cmp(left, right, |a, b| a == b, "S="),
            StringNotEqual => self.eval_string_cmp(left, right, |a, b| a != b, "S!="),
            StringLess => self.eval_string_cmp(left, right, |a, b| a < b, "S<"),
            StringLessEqual => self.eval_string_cmp(left, right, |a, b| a <= b, "S<="),
            StringGreater => self.eval_string_cmp(left, right, |a, b| a > b, "S>"),
            StringGreaterEqual => self.eval_string_cmp(left, right, |a, b| a >= b, "S>="),
        }
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
        cmp: F,
        _symbol: &str,
    ) -> Result<Value, RuntimeError>
    where
        F: Fn(&str, &str) -> bool,
    {
        // Convert both operands to strings for comparison
        let left_str = left.to_string();
        let right_str = right.to_string();
        Ok(Value::Bool(cmp(&left_str, &right_str)))
    }

    /// `==` per the script's #strict level (C4Value::Equals, C4Value.cpp:823).
    /// `#strict 3` is type-checked (Int and Bool are distinct types); lower
    /// levels (NONSTRICT/STRICT1 raw-bits, STRICT2 cross-type numeric) collapse,
    /// for a value-typed Value, to: compare Int/Bool/nil by integer value
    /// (so 0==nil, 1==true, 0==false), everything else by type+content.
    fn values_equal(&self, left: &Value, right: &Value, strict: Option<u8>) -> bool {
        if strict < Some(3) {
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
            (Value::Array(elements), Value::Int(raw_index)) => {
                if raw_index < 0 {
                    return Err(RuntimeError::new("array index cannot be negative"));
                }
                let index = raw_index as usize;
                elements
                    .get(index)
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("array index out of bounds"))
            }
            (Value::Proplist(entries), Value::String(key)) => {
                Ok(entries.get(&key).cloned().unwrap_or(Value::Nil))
            }
            (Value::Proplist(_), other) => Err(RuntimeError::new(format!(
                "proplist keys must be strings, got {}",
                other.type_name()
            ))),
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
    /// object and id targets resolve on the TARGET's context through the
    /// engine-registered method dispatch; self-targets stay in-VM (same
    /// resolution, live locals). The `~` only forgives a missing FUNCTION.
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
        let mut target = self.evaluate(base, env, depth + 1)?;
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
            && !self.host_functions.contains_key(name)
        {
            let index = self.evaluate_slot_index("Local()", &args[0], env, depth)?;
            if index < 0 {
                return Ok(Value::Nil);
            }
            let cell = self.numbered_local_cell(env, index, Some(target));
            let value = cell.borrow().clone();
            return Ok(value);
        }
        match &target {
            Value::Nil | Value::Int(0) | Value::Bool(false) => Err(RuntimeError::new(
                "Object call: target is zero!".to_string(),
            )),
            Value::Object(_) | Value::C4Id(_)
                if self.method_dispatch.is_some() && target != self.this_value =>
            {
                let function = self.functions.get(name);
                let mut evaluated_args = self.build_call_args(function, args, env, depth + 1)?;
                if forward_rest {
                    Self::append_forwarded_args(&mut evaluated_args, env);
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
            && !self.host_functions.contains_key(name)
            && !self
                .global_functions
                .map(|functions| functions.contains_key(name))
                .unwrap_or(false)
        {
            // ->~ on a missing function: the parameters still evaluate (they
            // are on the stack before AB_CALLFS pops them, C4AulExec.cpp:
            // 1262-1267), the result is nil.
            let _ = self.build_call_args(function, args, env, depth + 1)?;
            return Ok(Value::Nil);
        }
        let mut evaluated_args = self.build_call_args(function, args, env, depth + 1)?;
        if forward_rest {
            Self::append_forwarded_args(&mut evaluated_args, env);
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
        function: Option<&Function>,
        args: &[Expr],
        env: &mut Environment,
        depth: usize,
    ) -> Result<Vec<CallArg>, RuntimeError> {
        let mut evaluated_args = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let wants_reference = function
                .and_then(|function| function.params.get(index))
                .is_some_and(|param| param.is_reference);
            if wants_reference && Self::expr_can_be_lvalue(arg) {
                evaluated_args.push(CallArg::Reference(self.expr_to_lvalue(arg, env, depth)?));
            } else {
                evaluated_args.push(CallArg::Value(self.evaluate(arg, env, depth)?));
            }
        }
        Ok(evaluated_args)
    }

    /// `Callee(args, ...)`: after the explicit arguments, forward every
    /// parameter slot of the executing function past its named parameters,
    /// stopping at the 10-slot frame limit (C4AulParse.cpp:2293-2306).
    fn append_forwarded_args(evaluated_args: &mut Vec<CallArg>, env: &Environment) {
        let mut index = env.named_param_count;
        while evaluated_args.len() < MAX_CALL_PARAMETERS && index < MAX_CALL_PARAMETERS {
            let value = env.call_args.get(index).cloned().unwrap_or(Value::Nil);
            evaluated_args.push(CallArg::Value(value));
            index += 1;
        }
        // C++ callees always see 10 slots and cannot tell a missing argument
        // from an explicit nil, so dropping the nil tail is observationally
        // identical — and keeps host functions that count arguments honest.
        while matches!(evaluated_args.last(), Some(CallArg::Value(Value::Nil))) {
            evaluated_args.pop();
        }
    }

    fn expr_can_be_lvalue(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Variable(_) | Expr::Property(_, _) | Expr::Index(_, _) | Expr::Call { .. }
        )
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
            AssignmentTarget::Variable(name) => env
                .lvalue(name)
                .or_else(|| self.global_variable_cell(name).map(LValueRef::Cell))
                .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'"))),
            AssignmentTarget::Property(base, property) => Ok(self
                .assignment_target_to_lvalue(env, base, depth)?
                .append(PathSegment::Property(property.clone()))),
            AssignmentTarget::Index(base, index_expr) => {
                let index = self.evaluate(index_expr, env, depth)?;
                Ok(self
                    .assignment_target_to_lvalue(env, base, depth)?
                    .append(PathSegment::Index(index)))
            }
            AssignmentTarget::LocalSlot(index_expr) => {
                let index = self.evaluate_slot_index("Local()", index_expr, env, depth)?;
                Ok(env.local_slot_lvalue(index))
            }
            AssignmentTarget::VarSlot(index_expr) => {
                let index = self.evaluate_slot_index("Var()", index_expr, env, depth)?;
                Ok(env.var_slot_lvalue(index))
            }
            AssignmentTarget::FunctionCall { name, args }
                if name == "Global"
                    && !self.functions.contains_key(name)
                    && !self
                        .global_functions
                        .is_some_and(|functions| functions.contains_key(name))
                    && !self.host_functions.contains_key(name) =>
            {
                Ok(LValueRef::Cell(self.evaluate_global_slot(
                    args,
                    env,
                    depth + 1,
                )?))
            }
            AssignmentTarget::FunctionCall { name, args }
                if name == "GlobalN"
                    && !self.functions.contains_key(name)
                    && !self
                        .global_functions
                        .is_some_and(|functions| functions.contains_key(name))
                    && !self.host_functions.contains_key(name) =>
            {
                self.evaluate_named_global(args, env, depth + 1)?
                    .map(LValueRef::Cell)
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
                Ok(LValueRef::Cell(self.localn_cell(env, &local_name, target)))
            }
            AssignmentTarget::FunctionCall { name, args } => {
                let function = self.functions.get(name);
                let args = self.build_call_args(function, args, env, depth + 1)?;
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
                Ok(LValueRef::Cell(self.localn_cell(
                    env,
                    &local_name,
                    Some(object_value),
                )))
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
                Ok(LValueRef::Cell(self.numbered_local_cell(
                    env,
                    index,
                    Some(object_value),
                )))
            }
            AssignmentTarget::EffectSlot(_) | AssignmentTarget::MethodSlot { .. } => Err(
                RuntimeError::new("this assignment target cannot be passed by reference"),
            ),
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

    fn evaluate_slot_index(
        &self,
        name: &str,
        expr: &Expr,
        env: &mut Environment,
        depth: usize,
    ) -> Result<i32, RuntimeError> {
        match self.evaluate(expr, env, depth)? {
            Value::Int(index) => Ok(index),
            other => Err(RuntimeError::new(format!(
                "{name} index must be an integer, got {}",
                other.type_name()
            ))),
        }
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
                // Handle obj->LocalN("key"), obj->Local(index), etc.
                else if let Expr::Property(ref object, ref method) = **callee {
                    if !is_optional
                        && matches!(method.as_str(), "LocalN" | "Local" | "Var" | "EffectVar")
                    {
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
                declare_var,
                body,
                ..
            } => {
                if *declare_var {
                    env.declare_hoisted(variable);
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
    call_args: Rc<Vec<Value>>,
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
}

impl Environment {
    fn new_with_params(
        params: &[Parameter],
        args: &[CallArg],
        strict_level: Option<u8>,
        object_state: ObjectState,
    ) -> Result<Self, RuntimeError> {
        let mut scopes = vec![HashMap::new()];
        let base = scopes.last_mut().unwrap();
        for (param, arg) in params.iter().zip(args.iter()) {
            let binding = if param.is_reference {
                match arg {
                    CallArg::Reference(reference) => Binding::Reference(reference.clone()),
                    CallArg::Value(value) => Binding::direct(value.clone()),
                }
            } else {
                Binding::direct(arg.read()?)
            };
            base.insert(param.name.clone(), binding);
        }
        let call_args = args
            .iter()
            .map(CallArg::read)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            scopes,
            strict_level,
            var_slots: Rc::new(RefCell::new(HashMap::new())),
            object_state,
            call_args: Rc::new(call_args),
            named_param_count: params.len(),
            inherited_target: None,
            function_name: String::new(),
        })
    }

    fn var_slot_lvalue(&mut self, index: i32) -> LValueRef {
        LValueRef::Cell(slot_cell(&self.var_slots, index))
    }

    fn local_slot_lvalue(&mut self, index: i32) -> LValueRef {
        LValueRef::Cell(self.object_state.local_slot_cell(index))
    }

    fn define_object_local(&mut self, name: &str) {
        let cell = self.object_state.named_local_cell(name);
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), Binding::Direct(cell));
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: &str, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), Binding::direct(value));
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
        for scope in self.scopes.iter_mut().rev() {
            if let Some(binding) = scope.get(name) {
                return binding.write(value);
            }
        }
        Err(RuntimeError::new(format!("undefined variable '{name}'")))
    }

    fn get(&self, name: &str) -> Result<Option<Value>, RuntimeError> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return value.read().map(Some);
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
