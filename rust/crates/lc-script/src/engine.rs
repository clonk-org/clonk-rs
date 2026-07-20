use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;

use crate::ast::{Function, Script as AstScript, VarDecl};
use crate::debugger::DebuggerHooks;
use crate::error::{ParseError, RuntimeError, ScriptError};
use crate::parser::Parser;
use crate::value::Value;
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

/// Cross-object `func &` dispatch. Kept separate from [`HostFunction`] so an
/// lvalue call result is never flattened to a copied [`Value`].
pub type MethodReferenceDispatch =
    std::rc::Rc<dyn Fn(&[Value]) -> Result<ValueReference, RuntimeError>>;

/// Enters/leaves the native no-object scope required by strict-3
/// `global->Fn(...)`. The script VM owns unwinding; the embedding engine owns
/// its object/definition context and therefore supplies this small hook.
pub type GlobalCallContextHook = Arc<dyn Fn(bool) + Send + Sync>;

/// The engine-global named-variable table (`static` declarations;
/// C4AulScriptEngine::GlobalNamed): one shared table across every script
/// host. Values live in cells so lvalues (x = .., x++, ...) write through.
pub type GlobalVariables =
    std::rc::Rc<std::cell::RefCell<IndexMap<String, crate::vm::ValueCell>>>;

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

pub fn new_global_variables() -> GlobalVariables {
    std::rc::Rc::new(std::cell::RefCell::new(IndexMap::new()))
}

/// One process-global `C4StringTable` registration ledger.
///
/// Native strings retain their previous `iEnumID` until the next explicit
/// `EnumStrings` call. Scenario-section saves observe that old enumeration
/// before object serialization assigns a new one, so registration order alone
/// is not enough to reproduce their `Strings.txt` payload.
#[derive(Clone, Debug, Default)]
pub struct StringRegistrationLedger {
    entries: Vec<StringRegistration>,
}

#[derive(Clone, Debug)]
struct StringRegistration {
    value: String,
    /// Parser-owned strings survive with zero value references (`Hold`).
    held: bool,
    /// Strings created by `C4StringTable::Load` survive with zero references.
    loaded: bool,
    /// The value last assigned by `C4StringTable::EnumStrings`/`Load`.
    enum_id: Option<i32>,
}

/// Registration/enumeration state shared by every script host in one game.
pub type StringRegistrations =
    std::rc::Rc<std::cell::RefCell<StringRegistrationLedger>>;

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
    let left = crate::value::c4_string_bytes(left);
    let right = crate::value::c4_string_bytes(right);
    c4_string_table_prefix(&left) == c4_string_table_prefix(&right)
}

fn c4_string_table_bytes_equal(left: &[u8], right: &[u8]) -> bool {
    c4_string_table_prefix(left) == c4_string_table_prefix(right)
}

pub fn new_string_registrations() -> StringRegistrations {
    std::rc::Rc::new(std::cell::RefCell::new(StringRegistrationLedger::default()))
}

pub fn register_c4_string(
    registrations: &std::cell::RefCell<StringRegistrationLedger>,
    value: &str,
) {
    let mut registrations = registrations.borrow_mut();
    if registrations
        .entries
        .iter()
        .any(|candidate| crate::value::c4_strings_equal(&candidate.value, value))
    {
        return;
    }
    registrations.entries.push(StringRegistration {
        value: value.to_owned(),
        held: false,
        loaded: false,
        enum_id: None,
    });
}

/// Register a parser-owned string. `C4AulParse` reuses the first equal table
/// entry and sets its `Hold` flag rather than constructing a duplicate.
pub fn register_c4_literal_string(
    registrations: &std::cell::RefCell<StringRegistrationLedger>,
    value: &str,
) {
    let mut registrations = registrations.borrow_mut();
    if let Some(existing) = registrations
        .entries
        .iter_mut()
        .find(|candidate| c4_string_table_values_equal(&candidate.value, value))
    {
        existing.held = true;
        return;
    }
    registrations.entries.push(StringRegistration {
        value: value.to_owned(),
        held: true,
        loaded: false,
        enum_id: None,
    });
}

/// Merge one `Strings.txt` line exactly as `C4StringTable::Load`: equal text
/// reuses the existing registration and a repeated line overwrites its old ID.
pub fn register_loaded_c4_string(
    registrations: &std::cell::RefCell<StringRegistrationLedger>,
    enum_id: i32,
    value: &str,
) {
    let mut registrations = registrations.borrow_mut();
    if let Some(existing) = registrations
        .entries
        .iter_mut()
        .find(|candidate| c4_string_table_values_equal(&candidate.value, value))
    {
        existing.enum_id = Some(enum_id);
        return;
    }
    registrations.entries.push(StringRegistration {
        value: value.to_owned(),
        held: false,
        loaded: true,
        enum_id: Some(enum_id),
    });
}

/// Snapshot current table order for diagnostics and compatibility callers.
pub fn c4_string_registration_order(
    registrations: &std::cell::RefCell<StringRegistrationLedger>,
) -> Vec<String> {
    registrations
        .borrow()
        .entries
        .iter()
        .map(|entry| entry.value.clone())
        .collect()
}

fn c4_string_is_referenced(value: &str, referenced: &[Vec<u8>]) -> bool {
    let bytes = crate::value::c4_string_bytes(value);
    referenced.iter().any(|candidate| candidate == &bytes)
}

/// Bytes emitted by `C4StringTable::Save` *without* enumerating first.
///
/// This is the native section-switch boundary: stale/non-negative enum IDs
/// decide eligibility, while output still follows linked-list registration
/// order. Dead runtime registrations do not participate in `FindSaveString`.
pub fn save_current_c4_string_enumeration(
    registrations: &std::cell::RefCell<StringRegistrationLedger>,
    referenced: &[Vec<u8>],
) -> Vec<Vec<u8>> {
    let registrations = registrations.borrow();
    let mut first_live = Vec::<Vec<u8>>::new();
    let mut saved = Vec::new();
    for entry in &registrations.entries {
        let live = entry.loaded || c4_string_is_referenced(&entry.value, referenced);
        if !live {
            continue;
        }
        let bytes = crate::value::c4_string_bytes(&entry.value);
        if first_live
            .iter()
            .any(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
        {
            continue;
        }
        first_live.push(bytes.clone());
        if entry.enum_id.is_some() {
            saved.push(bytes);
        }
    }
    saved
}

/// Execute `C4StringTable::EnumStrings` and return values in their assigned-ID
/// order. Unreferenced non-held runtime registrations are removed, so a later
/// reconstruction of the same text appends after registrations that survived
/// the death boundary.
pub fn enumerate_c4_strings(
    registrations: &std::cell::RefCell<StringRegistrationLedger>,
    referenced: &[Vec<u8>],
) -> Vec<Vec<u8>> {
    let mut registrations = registrations.borrow_mut();

    // Every live C4Value string necessarily owns a registration. Values may
    // enter Rust engine state without passing through the VM, so recover such
    // registrations at this same enumeration boundary in traversal order.
    for bytes in referenced {
        if registrations.entries.iter().any(|entry| {
            crate::value::c4_string_bytes(&entry.value) == *bytes
        }) {
            continue;
        }
        registrations.entries.push(StringRegistration {
            value: crate::value::c4_string_from_bytes(bytes),
            held: false,
            loaded: false,
            enum_id: None,
        });
    }

    let mut values = Vec::<Vec<u8>>::new();
    let mut next_id = 0_i32;
    for entry in &mut registrations.entries {
        let live = entry.loaded || c4_string_is_referenced(&entry.value, referenced);
        if !live {
            entry.enum_id = None;
            continue;
        }
        let bytes = crate::value::c4_string_bytes(&entry.value);
        let enum_id = if let Some(index) = values
            .iter()
            .position(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
        {
            i32::try_from(index).unwrap_or(i32::MAX)
        } else {
            let enum_id = next_id;
            next_id = next_id.saturating_add(1);
            values.push(bytes);
            enum_id
        };
        entry.enum_id = Some(enum_id);
    }
    registrations.entries.retain(|entry| {
        entry.enum_id.is_some() || entry.held || entry.loaded
    });
    values
}

/// Register every string owned by a C4Value in construction/traversal order.
/// The VM invokes this as expressions materialize; embedders also use it for
/// values entering synchronized state without passing through the VM.
pub fn register_c4_value_strings(
    registrations: &std::cell::RefCell<StringRegistrationLedger>,
    value: &Value,
) {
    match value {
        Value::String(value) => register_c4_string(registrations, value),
        Value::Array(values) => {
            for value in values {
                register_c4_value_strings(registrations, value);
            }
        }
        Value::Proplist(values) => {
            for (key, value) in values {
                register_c4_value_strings(registrations, key);
                register_c4_value_strings(registrations, value);
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

pub fn new_global_slots() -> GlobalSlots {
    std::rc::Rc::new(std::cell::RefCell::new(std::collections::BTreeMap::new()))
}

/// Registers a script's `static` and `static const` declarations in the
/// engine-global tables used by every script host.
pub fn register_global_declarations(
    var_decls: &[VarDecl],
    table: &GlobalVariables,
    globals_consts: Option<&GlobalVariables>,
) {
    for var_decl in var_decls {
        match var_decl.kind {
            crate::ast::VarDeclKind::Static => {
                table
                    .borrow_mut()
                    .entry(var_decl.name.clone())
                    .or_insert_with(|| crate::vm::value_cell(Value::Nil));
            }
            crate::ast::VarDeclKind::StaticConst => {
                // C4Aul accepts direct constants only. Its tokenizer folds a
                // leading sign into ATT_INT when parsing a constant value;
                // our parser represents a negative integer as Unary(Negate).
                let value = match &var_decl.init {
                    Some(crate::ast::Expr::Literal(literal)) => Value::from(literal.clone()),
                    Some(crate::ast::Expr::Unary(
                        crate::ast::UnaryOp::Negate,
                        expression,
                    )) => match expression.as_ref() {
                        crate::ast::Expr::Literal(crate::value::Literal::Int(value)) => {
                            Value::Int(value.wrapping_neg())
                        }
                        _ => Value::Nil,
                    },
                    Some(crate::ast::Expr::Variable(name)) => {
                        let constants = globals_consts.unwrap_or(table);
                        let cell = constants.borrow().get(name).cloned();
                        cell.map(|cell| cell.borrow().clone())
                            .unwrap_or(Value::Nil)
                    }
                    _ => Value::Nil,
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
            }
            crate::ast::VarDeclKind::Local => {}
        }
    }
}

#[derive(Clone, Default)]
pub struct Script {
    functions: HashMap<String, Function>,
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
        let mut functions: HashMap<String, Function> = HashMap::new();
        for mut function in ast.functions {
            // Each function carries its owning script's #strict level so the VM
            // can apply level-correct `==`/`!=` (C++ uses Fn->pOrgScript->Strict).
            function.strict_level = ast.strict_level;
            // A redefinition in the SAME script keeps the earlier definition
            // as its `inherited` target (`Fn->OwnerOverloaded =
            // Fn->Owner->GetOverloadedFunc(Fn)`, C4AulParse.cpp:1404-1406) —
            // the Coach.c4d menu-description wrappers forward through it.
            if let Some(previous) = functions.remove(&function.name) {
                function.push_overload(previous);
            }
            functions.insert(function.name.clone(), function);
        }
        Self {
            functions,
            includes: ast.includes,
            appends: ast.appends,
            strict_level: ast.strict_level,
            var_decls: ast.var_decls,
            string_literals: ast.string_literals,
            parse_diagnostics,
        }
    }

    pub fn functions(&self) -> &HashMap<String, Function> {
        &self.functions
    }

    pub fn global_access_functions(&self) -> impl Iterator<Item = (&String, &Function)> {
        self.functions
            .iter()
            .filter(|(_, function)| function.access == crate::ast::AccessLevel::Global)
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
    functions: HashMap<String, Function>,
    /// Stable identity of this C4AulScript destination host. It survives
    /// Rust moves and copy-on-write Engine clones so global Function
    /// `LinkedTo` provenance never depends on a HashMap's address.
    host_identity: crate::vm::ScriptHostIdentity,
    /// Strictness of this C4AulScript host itself. Linked include/append
    /// function copies keep their source strictness for expression semantics,
    /// but native calls inspect `Func->Owner->Strict` (the destination host).
    /// The outer Option distinguishes an uninitialized bare Engine from a
    /// deliberately NONSTRICT base script.
    owner_strict_level: Option<Option<u8>>,
    host_functions: HashMap<String, RegisteredHostFunction>,
    host_reference_functions: HashMap<String, HostReferenceFunction>,
    debugger_hooks: Option<DebuggerHooks>,
    var_decls: Vec<VarDecl>, // Script-level variable declarations (local variables)
    /// Engine script constants (RegisterGlobalConstant, C4Script.cpp:6581),
    /// consulted by the VM when an identifier matches no variable.
    constants: HashMap<String, Value>,
    /// Engine-global script functions (System.c4g global funcs, owned by
    /// Game.ScriptEngine in C++): shared across every script host, resolved
    /// after the own script and before host functions.
    global_functions: Option<Arc<HashMap<String, Function>>>,
    /// `obj->Method(args)` cross-object resolver (AB_CALL,
    /// C4AulExec.cpp:1216-1305): the VM is world-agnostic, so the engine
    /// registers this hook to run the function on the TARGET object's
    /// script. Called with [target, name, failsafe, args...].
    method_dispatch: Option<HostFunction>,
    /// Reference-preserving method resolver for arrow calls in lvalue
    /// position. Arguments use the same [target, name, failsafe, args...]
    /// layout as `method_dispatch`.
    method_reference_dispatch: Option<MethodReferenceDispatch>,
    /// Embedding-engine context switch for AB_CALLGLOBAL's null Obj/Def.
    global_call_context_hook: Option<GlobalCallContextHook>,
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
    /// Shared engine-global C4StringTable registration ledger.
    string_registrations: Option<StringRegistrations>,
    /// Literals retained by scripts already installed in this host. This lets
    /// a late-attached game ledger recover the native link order.
    string_literals: Vec<String>,
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
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptFunctionResolution {
    pub scope: ScriptFunctionScope,
    pub host_identity: crate::vm::ScriptHostIdentity,
    /// Immutable queue-time function body and overload provenance.
    pub function: Arc<Function>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            host_identity: crate::vm::ScriptHostIdentity::fresh(),
            owner_strict_level: None,
            host_functions: HashMap::new(),
            host_reference_functions: HashMap::new(),
            debugger_hooks: None,
            var_decls: Vec::new(),
            constants: HashMap::new(),
            global_functions: None,
            method_dispatch: None,
            method_reference_dispatch: None,
            global_call_context_hook: None,
            globals_named: None,
            globals_numbered: Some(new_global_slots()),
            globals_consts: None,
            local_cell_hook: None,
            string_registrations: None,
            string_literals: Vec::new(),
        }
    }

    /// Process-local identity of this destination script host. Native
    /// compatibility code uses it to match a caller's local-lookup host
    /// (`Func->Owner` for local functions, `Func->LinkedTo` for globals)
    /// back to the exact retained engine without consulting the object def.
    pub fn host_identity(&self) -> crate::vm::ScriptHostIdentity {
        self.host_identity
    }

    /// Installs the engine-global script function table (System.c4g
    /// global funcs). Shared by Arc so every definition script host sees
    /// the same copy.
    pub fn set_global_functions(&mut self, functions: Option<Arc<HashMap<String, Function>>>) {
        self.global_functions = functions;
    }

    /// Whether the global table knows `name`.
    pub fn has_global_function(&self, name: &str) -> bool {
        self.global_functions
            .as_ref()
            .map(|functions| functions.contains_key(name))
            .unwrap_or(false)
    }

    /// Registers an engine script constant (RegisterGlobalConstant,
    /// C4Script.cpp:6581): identifiers resolve to it when no variable
    /// matches; variables shadow constants.
    pub fn register_constant(&mut self, name: impl Into<String>, value: Value) {
        self.constants.insert(name.into(), value);
    }

    pub fn load_script(&mut self, source: &str) -> Result<(), ScriptError> {
        let script = Script::compile(source)?;
        self.add_script(script);
        Ok(())
    }

    #[cfg(feature = "ffi")]
    pub(crate) fn load_script_c4_string(&mut self, source: &str) -> Result<(), ScriptError> {
        let script = Script::compile_c4_string(source)?;
        self.add_script(script);
        Ok(())
    }

    pub fn add_script(&mut self, mut script: Script) {
        if let Some(registrations) = &self.string_registrations {
            for literal in &script.string_literals {
                register_c4_literal_string(registrations, literal);
            }
        }
        self.string_literals.extend(script.string_literals.iter().cloned());
        for function in script.functions.values_mut() {
            function.bind_source_host(self.host_identity);
            function.bind_global_link_host(self.host_identity);
        }
        if self.owner_strict_level.is_none() {
            self.owner_strict_level = Some(script.strict_level);
        }
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
            if var_decl.kind == crate::ast::VarDeclKind::Static {
                if let Some(table) = &self.globals_named {
                    table
                        .borrow_mut()
                        .entry(var_decl.name.clone())
                        .or_insert_with(|| crate::vm::value_cell(Value::Nil));
                    continue;
                }
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
    pub fn replace_script(&mut self, mut script: Script, register_declarations: bool) {
        if let Some(registrations) = &self.string_registrations {
            for literal in &script.string_literals {
                register_c4_literal_string(registrations, literal);
            }
        }
        self.string_literals.clone_from(&script.string_literals);
        for function in script.functions.values_mut() {
            function.bind_source_host(self.host_identity);
            function.bind_global_link_host(self.host_identity);
        }
        self.owner_strict_level = Some(script.strict_level);
        self.functions.clear();
        self.var_decls.clear();

        if register_declarations {
            if let Some(table) = &self.globals_named {
                register_global_declarations(
                    &script.var_decls,
                    table,
                    self.globals_consts.as_ref(),
                );
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
        for (name, function) in other.functions.iter() {
            if function.access == crate::ast::AccessLevel::Global {
                continue;
            }
            let mut function = function.clone();
            if let Some(previous) = self.functions.remove(name) {
                function.push_overload(previous);
            }
            self.functions.insert(name.clone(), function);
        }
        for var_decl in other.var_decls.iter() {
            if !self.var_decls.iter().any(|v| v.name == var_decl.name) {
                self.var_decls.push(var_decl.clone());
            }
        }
    }

    pub fn merge_from(&mut self, other: &Engine) {
        for (name, function) in other.functions.iter() {
            // Includes are AppendTo with bHighPrio=false in C++ — global
            // funcs are never copied (C4AulLink.cpp:127); they stay
            // reachable through the engine table.
            if function.access == crate::ast::AccessLevel::Global {
                continue;
            }
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
    /// registers these at the script ENGINE, not the local host.
    pub fn global_access_functions(&self) -> impl Iterator<Item = (&String, &Function)> {
        self.functions
            .iter()
            .filter(|(_, function)| function.access == crate::ast::AccessLevel::Global)
    }

    /// Repoints a declaring script's local global-function link at the
    /// engine-owned function. C4Aul creates both objects for `global func`:
    /// the function lives on the script engine while a `FnLink` remains in
    /// the original script (C4AulParse.cpp:1603-1610). The linked function
    /// carries the engine overload chain used by `inherited()`.
    pub fn link_global_access_function(&mut self, name: &str, function: Function) -> bool {
        let Some(local) = self.functions.get(name) else {
            return false;
        };
        if local.access != crate::ast::AccessLevel::Global {
            return false;
        }
        self.functions.insert(name.to_string(), function);
        true
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
        self.host_reference_functions.remove(&name);
        let function = match parameter_count {
            Some(parameter_count) => RegisteredHostFunction::declared(func, parameter_count),
            None => RegisteredHostFunction::variadic(func),
        };
        self.host_functions.insert(name, function);
    }

    /// Register a native function whose listed zero-based parameters receive
    /// live script lvalues when the call expression supplies one. A declared
    /// reference parameter passed a non-lvalue remains a readable value
    /// argument with [`HostCallArg::is_reference`] false, matching C4Aul's
    /// nullable `C4Value *` native parameters.
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
        self.host_functions.remove(&name);
        self.host_reference_functions.insert(
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
        self.host_functions.remove(&name);
        self.host_reference_functions.insert(
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
        self.host_functions.clear();
        self.host_reference_functions.clear();
    }

    /// Remove either native-host registration kind under `name`. The return
    /// value remains the ordinary value-host callback for API compatibility;
    /// removing a reference-aware registration succeeds with `None`.
    pub fn remove_host_function(&mut self, name: &str) -> Option<HostFunction> {
        self.host_reference_functions.remove(name);
        self.host_functions
            .remove(name)
            .map(|function| function.callback)
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

    /// Moves `static` declarations that were compiled BEFORE the table was
    /// attached out of the per-object locals and into the shared table
    /// (existing values persist).
    pub fn adopt_statics_into_globals(&mut self) {
        let Some(table) = self.globals_named.clone() else {
            return;
        };
        let globals_consts = self.globals_consts.clone();
        register_global_declarations(&self.var_decls, &table, globals_consts.as_ref());
        self.var_decls
            .retain(|var_decl| var_decl.kind == crate::ast::VarDeclKind::Local);
    }

    /// Registers the cross-object LocalN cell supplier (FnLocalN's
    /// by-reference foreign-local access, C4Script.cpp:4591-4605).
    pub fn register_local_cell_hook(&mut self, hook: LocalCellHook) {
        self.local_cell_hook = Some(hook);
    }

    pub fn register_method_dispatch(&mut self, dispatch: HostFunction) {
        self.method_dispatch = Some(dispatch);
    }

    pub fn register_method_reference_dispatch(&mut self, dispatch: MethodReferenceDispatch) {
        self.method_reference_dispatch = Some(dispatch);
    }

    pub fn register_global_call_context_hook(&mut self, hook: GlobalCallContextHook) {
        self.global_call_context_hook = Some(hook);
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, ScriptError> {
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref());
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref());
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_exact_global_link_lookup()
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref());
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref());
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
        let vm = if engine_global {
            vm.with_exact_global_link_lookup()
        } else {
            vm
        };
        vm.call_pinned_with_cells(function, args, cells)
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref());
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    /// Like [`call_with_locals`], but also provides the `this` object context
    /// returned by `Expr::This`. Pass `Value::Object(id)` for an object context
    /// or `Value::Nil` for no context.
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
        vm.call_with_cells(name, args, cells).map_err(ScriptError::from)
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
        vm.call_with_cells_preserving_caller(name, args, cells)
            .map_err(ScriptError::from)
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
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
        self.direct_exec_with_locals_and_this_at_strict(
            source,
            local_vars,
            this,
            self.script_strict_level(),
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
        vm.direct_exec_with_locals(source, local_vars, strict_level)
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
        self.direct_exec_with_cells_and_this_at_strict(
            source,
            cells,
            this,
            self.script_strict_level(),
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
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_host_identity(self.host_identity)
        .with_host_reference_functions(&self.host_reference_functions)
        .with_owner_strict_level(self.owner_strict_level.unwrap_or(None))
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_method_reference_dispatch(self.method_reference_dispatch.as_ref())
        .with_global_call_context_hook(self.global_call_context_hook.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_slots(self.globals_numbered.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_string_registrations(self.string_registrations.as_deref())
        .with_this(this);
        vm.direct_exec_with_cells(source, cells, strict_level)
            .map_err(ScriptError::from)
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
    pub fn functions(&self) -> &HashMap<String, Function> {
        &self.functions
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
        Some(ScriptFunctionResolution {
            scope,
            host_identity,
            function: Arc::new(function.clone()),
        })
    }

    /// Resolve only the engine-owned global script table. This is the
    /// `GetFuncRecursive` starting point when the calling function's owner
    /// is `Game.ScriptEngine`, so declaring-host locals must not shadow it.
    pub fn resolve_global_function(&self, name: &str) -> Option<ScriptFunctionResolution> {
        let function = self.global_functions.as_deref()?.get(name)?;
        Some(ScriptFunctionResolution {
            scope: ScriptFunctionScope::Global,
            host_identity: function.global_link_host.unwrap_or(self.host_identity),
            function: Arc::new(function.clone()),
        })
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
    use std::rc::Rc;
    use std::sync::Mutex;

    use super::*;

    fn compile(source: &str) -> Script {
        Script::compile(source).expect("test script compiles")
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
                .collect::<HashMap<_, _>>(),
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
            destination.call("LocalQueue", &[]).expect("local call runs"),
            Value::Bool(true)
        );
        assert_eq!(
            destination.call("Queue", &[]).expect("global fallback runs"),
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
            .collect::<HashMap<_, _>>();

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
            declaring.call("Queue", &[]).expect("global helper resolves"),
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
            .collect::<HashMap<_, _>>();
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
        assert_eq!(
            host.call_global_with_ref_args("Queue", &[])
                .expect("exact global callback resolves")
                .0,
            Value::Int(1)
        );
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
    fn pinned_global_shared_context_uses_its_linked_host_helper() {
        let mut linked_host = Engine::new();
        linked_host
            .load_script(
                "local Shared;\n\
                 func Helper() { Shared = Shared + 4; return Shared; }\n\
                 global func Deferred() {\n\
                     Shared = Shared + 2;\n\
                     return [this, Helper(), Shared];\n\
                 }",
            )
            .expect("linked-host script compiles");

        let mut destination = Engine::new();
        destination
            .load_script("global func Helper() { return 999; }")
            .expect("conflicting destination helper compiles");

        let mut globals = linked_host
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<HashMap<_, _>>();
        globals.extend(
            destination
                .global_access_functions()
                .map(|(name, function)| (name.clone(), function.clone())),
        );
        linked_host.set_global_functions(Some(Arc::new(globals)));
        let pinned = linked_host
            .resolve_global_function("Deferred")
            .expect("global callback resolves")
            .function;
        let cells = crate::vm::LocalCells::from_local_vars(&HashMap::from([(
            "Shared".to_string(),
            Value::Int(5),
        )]));

        let value = linked_host
            .call_pinned_with_cells_and_this(
                &pinned,
                true,
                &[],
                &cells,
                Value::Object(42),
            )
            .expect("pinned callback runs on its LinkedTo host");

        assert_eq!(
            value,
            Value::Array(vec![Value::Object(42), Value::Int(11), Value::Int(11)])
        );
        assert_eq!(cells.snapshot().get("Shared"), Some(&Value::Int(11)));
    }

    #[test]
    fn replace_script_removes_linked_overloads_and_preserves_host_functions() {
        let mut engine = Engine::new();
        engine
            .load_script("func Probe() { return 1; }")
            .expect("base script loads");
        engine
            .load_script("func Probe() { return inherited() + 1; }")
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
            globals.borrow().keys().map(String::as_str).collect::<Vec<_>>(),
            ["Zed", "Alpha"]
        );
        assert_eq!(
            c4_string_registration_order(&strings),
            ["zeta".to_string(), "alpha".to_string()]
        );
        assert_eq!(
            engine.call("Build", &[]).expect("string concatenates"),
            Value::String("zetaalpha".to_string())
        );
        assert_eq!(
            c4_string_registration_order(&strings),
            [
                "zeta".to_string(),
                "alpha".to_string(),
                "zetaalpha".to_string(),
            ]
        );
    }

    #[test]
    fn string_table_preserves_stale_section_enumeration_until_objects_save() {
        let strings = new_string_registrations();
        register_loaded_c4_string(&strings, 0, "loaded");
        register_c4_string(&strings, "runtime");
        let referenced = [b"loaded".to_vec(), b"runtime".to_vec()];

        assert_eq!(
            save_current_c4_string_enumeration(&strings, &referenced),
            [b"loaded".to_vec()]
        );
        assert_eq!(
            enumerate_c4_strings(&strings, &referenced),
            [b"loaded".to_vec(), b"runtime".to_vec()]
        );
        assert_eq!(
            save_current_c4_string_enumeration(&strings, &referenced),
            [b"loaded".to_vec(), b"runtime".to_vec()]
        );
    }

    #[test]
    fn dead_runtime_string_rebirth_appends_after_survivors() {
        let strings = new_string_registrations();
        register_c4_string(&strings, "first");
        register_c4_string(&strings, "survivor");
        assert_eq!(
            enumerate_c4_strings(
                &strings,
                &[b"first".to_vec(), b"survivor".to_vec()],
            ),
            [b"first".to_vec(), b"survivor".to_vec()]
        );

        assert_eq!(
            enumerate_c4_strings(&strings, &[b"survivor".to_vec()]),
            [b"survivor".to_vec()]
        );
        register_c4_string(&strings, "first");
        assert_eq!(
            enumerate_c4_strings(
                &strings,
                &[b"first".to_vec(), b"survivor".to_vec()],
            ),
            [b"survivor".to_vec(), b"first".to_vec()]
        );
    }

    #[test]
    fn held_and_loaded_strings_survive_zero_reference_enumeration() {
        let strings = new_string_registrations();
        register_c4_literal_string(&strings, "literal");
        register_loaded_c4_string(&strings, 0, "loaded");

        assert_eq!(
            enumerate_c4_strings(&strings, &[]),
            [b"loaded".to_vec()]
        );
        assert_eq!(
            c4_string_registration_order(&strings),
            ["literal".to_string(), "loaded".to_string()]
        );
        assert_eq!(
            enumerate_c4_strings(&strings, &[b"literal".to_vec()]),
            [b"literal".to_vec(), b"loaded".to_vec()]
        );
    }

    #[test]
    fn string_table_enumeration_uses_the_first_nul_as_its_identity_boundary() {
        let strings = new_string_registrations();
        register_c4_string(&strings, "shared\0first suffix");
        register_c4_string(&strings, "shared\0second suffix");
        let referenced = [
            b"shared\0first suffix".to_vec(),
            b"shared\0second suffix".to_vec(),
        ];

        assert_eq!(
            enumerate_c4_strings(&strings, &referenced),
            [b"shared\0first suffix".to_vec()],
            "FindSaveString gives both registrations the first live prefix-equivalent ID"
        );
        assert_eq!(
            save_current_c4_string_enumeration(&strings, &referenced),
            [b"shared\0first suffix".to_vec()]
        );
    }
}
