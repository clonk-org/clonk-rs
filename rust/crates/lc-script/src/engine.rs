use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::{Function, Script as AstScript, VarDecl};
use crate::debugger::DebuggerHooks;
use crate::error::{ParseError, RuntimeError, ScriptError};
use crate::parser::Parser;
use crate::value::Value;
use crate::vm::Vm;

pub type HostFunction = Arc<dyn Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync>;

/// The engine-global named-variable table (`static` declarations;
/// C4AulScriptEngine::GlobalNamed): one shared table across every script
/// host. Values live in cells so lvalues (x = .., x++, ...) write through.
pub type GlobalVariables =
    std::rc::Rc<std::cell::RefCell<HashMap<String, crate::vm::ValueCell>>>;

/// Supplies a live cell for a FOREIGN object's named local —
/// FnLocalN returns `pVarN->GetRef()` (C4Script.cpp:4591-4605), a
/// reference into the target's locals, so cross-object reads AND lvalue
/// writes go through it. Registered by the engine like method_dispatch.
pub type LocalCellHook = std::rc::Rc<dyn Fn(&Value, &str) -> Option<crate::vm::ValueCell>>;

pub fn new_global_variables() -> GlobalVariables {
    std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()))
}

#[derive(Clone, Default)]
pub struct Script {
    functions: HashMap<String, Function>,
    includes: Vec<String>,
    appends: Vec<crate::ast::AppendTo>,
    strict_level: Option<u8>,
    var_decls: Vec<VarDecl>, // Script-level variable declarations
}

impl Script {
    pub fn compile(source: &str) -> Result<Self, ParseError> {
        let mut parser = Parser::new(source);
        let ast = parser.parse_script()?;
        Ok(Self::from_ast(ast))
    }

    fn from_ast(ast: AstScript) -> Self {
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
        }
    }

    pub fn functions(&self) -> &HashMap<String, Function> {
        &self.functions
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
}

#[derive(Clone)]
pub struct Engine {
    functions: HashMap<String, Function>,
    host_functions: HashMap<String, HostFunction>,
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
    /// The shared `static` table; `None` keeps the legacy per-host
    /// fallback (fixtures without an engine).
    globals_named: Option<GlobalVariables>,
    /// The shared `static const` registry (C4AulScriptEngine's global
    /// constants, RegisterGlobalConstant C4Aul.cpp:484): script-declared
    /// constants every host sees. Cells are SHARED with `globals_named`
    /// so identifier reads and old-style constant calls agree.
    globals_consts: Option<GlobalVariables>,
    /// Cross-object LocalN cell supplier (see [`LocalCellHook`]).
    local_cell_hook: Option<LocalCellHook>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            host_functions: HashMap::new(),
            debugger_hooks: None,
            var_decls: Vec::new(),
            constants: HashMap::new(),
            global_functions: None,
            method_dispatch: None,
            globals_named: None,
            globals_consts: None,
            local_cell_hook: None,
        }
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

    pub fn add_script(&mut self, script: Script) {
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
                Some(own) => own.push_overload(function.clone()),
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

    pub fn function_count(&self) -> usize {
        self.functions.len()
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
        self.host_functions.insert(name.into(), Arc::new(func));
    }

    pub fn host_function_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.host_functions.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn clear_host_functions(&mut self) {
        self.host_functions.clear();
    }

    pub fn remove_host_function(&mut self, name: &str) -> Option<HostFunction> {
        self.host_functions.remove(name)
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

    /// Attaches the engine-global `static const` registry (the C4Aul
    /// global-constant table, C4Aul.cpp:484). Scripts adopted afterwards
    /// register their constants here so every host resolves them — both
    /// as identifiers and via the pre-#strict-2 `NAME()` call idiom
    /// (C4AulParse.cpp:2834-2864).
    pub fn set_global_constants(&mut self, table: GlobalVariables) {
        self.globals_consts = Some(table);
    }

    /// Moves `static` declarations that were compiled BEFORE the table was
    /// attached out of the per-object locals and into the shared table
    /// (existing values persist).
    pub fn adopt_statics_into_globals(&mut self) {
        let Some(table) = self.globals_named.clone() else {
            return;
        };
        let globals_consts = self.globals_consts.clone();
        self.var_decls.retain(|var_decl| {
            match var_decl.kind {
                crate::ast::VarDeclKind::Static => {
                    table
                        .borrow_mut()
                        .entry(var_decl.name.clone())
                        .or_insert_with(|| crate::vm::value_cell(Value::Nil));
                    false
                }
                // `static const` names are engine-global constants in C4Aul
                // (every script sees them — Talker.c4d's _TLK_TimerInterval
                // is read from GLOBAL funcs executing in other hosts).
                // Initializers are constant expressions; literals and
                // references to already-registered constants resolve here.
                crate::ast::VarDeclKind::StaticConst => {
                    let value = match &var_decl.init {
                        Some(crate::ast::Expr::Literal(literal)) => {
                            Value::from(literal.clone())
                        }
                        Some(crate::ast::Expr::Variable(name)) => table
                            .borrow()
                            .get(name)
                            .map(|cell| cell.borrow().clone())
                            .unwrap_or(Value::Nil),
                        _ => Value::Nil,
                    };
                    let cell = table
                        .borrow_mut()
                        .entry(var_decl.name.clone())
                        .or_insert_with(|| crate::vm::value_cell(value))
                        .clone();
                    // Also register the SAME cell as a global constant so
                    // the old-style `NAME()` call idiom resolves it
                    // (GetGlobalConstant, C4AulParse.cpp:2834-2864) —
                    // plain `static` variables stay uncallable.
                    if let Some(consts) = &globals_consts {
                        consts
                            .borrow_mut()
                            .entry(var_decl.name.clone())
                            .or_insert(cell);
                    }
                    false
                }
                _ => true,
            }
        });
    }

    /// Registers the cross-object LocalN cell supplier (FnLocalN's
    /// by-reference foreign-local access, C4Script.cpp:4591-4605).
    pub fn register_local_cell_hook(&mut self, hook: LocalCellHook) {
        self.local_cell_hook = Some(hook);
    }

    pub fn register_method_dispatch(&mut self, dispatch: HostFunction) {
        self.method_dispatch = Some(dispatch);
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, ScriptError> {
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref());
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
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref());
        let cells: Vec<crate::vm::ValueCell> =
            args.iter().cloned().map(crate::vm::value_cell).collect();
        let call_args = cells
            .iter()
            .map(|cell| crate::vm::CallArg::Reference(crate::vm::LValueRef::Cell(cell.clone())))
            .collect();
        let result = vm.call_args(name, call_args).map_err(ScriptError::from)?;
        let finals = cells.iter().map(|cell| cell.borrow().clone()).collect();
        Ok((result, finals))
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
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref());
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
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_this(this);
        vm.call_with_cells(name, args, cells).map_err(ScriptError::from)
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
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref())
        .with_method_dispatch(self.method_dispatch.as_ref())
        .with_global_variables(self.globals_named.as_deref())
        .with_global_constants(self.globals_consts.as_deref())
        .with_local_cell_hook(self.local_cell_hook.as_ref())
        .with_this(this);
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
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

    pub fn has_host_function(&self, name: &str) -> bool {
        self.host_functions.contains_key(name)
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
