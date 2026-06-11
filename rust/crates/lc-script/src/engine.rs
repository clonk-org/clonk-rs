use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::{Function, Script as AstScript, VarDecl};
use crate::debugger::DebuggerHooks;
use crate::error::{ParseError, RuntimeError, ScriptError};
use crate::parser::Parser;
use crate::value::Value;
use crate::vm::Vm;

pub type HostFunction = Arc<dyn Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync>;

#[derive(Clone, Default)]
pub struct Script {
    functions: HashMap<String, Function>,
    includes: Vec<String>,
    appendto: Option<crate::ast::AppendTo>,
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
        let mut functions = HashMap::new();
        for mut function in ast.functions {
            // Each function carries its owning script's #strict level so the VM
            // can apply level-correct `==`/`!=` (C++ uses Fn->pOrgScript->Strict).
            function.strict_level = ast.strict_level;
            functions.insert(function.name.clone(), function);
        }
        Self {
            functions,
            includes: ast.includes,
            appendto: ast.appendto,
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

    pub fn appendto(&self) -> Option<&crate::ast::AppendTo> {
        self.appendto.as_ref()
    }

    pub fn strict_level(&self) -> Option<u8> {
        self.strict_level
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
                Self::append_overload(&mut function, previous);
            }
            self.functions.insert(name, function);
        }
        // Store local variable declarations from the script
        self.var_decls.extend(script.var_decls);
    }

    /// Hang `parent` at the tail of `function`'s overload chain. Idempotent:
    /// include resolution re-merges to a fixpoint, so a parent already on the
    /// chain is replaced (it may have gained its own chain since) rather than
    /// appended twice.
    fn append_overload(function: &mut Function, parent: Function) {
        fn same_definition(a: &Function, b: &Function) -> bool {
            a.name == b.name
                && a.params == b.params
                && a.body == b.body
                && a.access == b.access
                && a.returns_reference == b.returns_reference
                && a.strict_level == b.strict_level
        }
        let mut tail = &mut function.overloaded;
        loop {
            let found = tail
                .as_deref()
                .is_some_and(|next| same_definition(next, &parent));
            if found {
                if parent.overloaded.is_some() {
                    *tail = Some(std::sync::Arc::new(parent));
                }
                return;
            }
            match tail {
                Some(next) => tail = &mut std::sync::Arc::make_mut(next).overloaded,
                None => {
                    *tail = Some(std::sync::Arc::new(parent));
                    return;
                }
            }
        }
    }

    pub fn merge_from(&mut self, other: &Engine) {
        for (name, function) in other.functions.iter() {
            match self.functions.get_mut(name) {
                // Child overrides parent, but the parent's function stays
                // reachable as the child's `inherited` target (C++ include
                // linking sets OwnerOverloaded).
                Some(own) => Self::append_overload(own, function.clone()),
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

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, ScriptError> {
        let vm = Vm::new(
            &self.functions,
            &self.host_functions,
            &self.var_decls,
            self.debugger_hooks.clone(),
        )
        .with_constants(&self.constants)
        .with_optional_globals(self.global_functions.as_deref());
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
        .with_optional_globals(self.global_functions.as_deref());
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
        .with_optional_globals(self.global_functions.as_deref());
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    /// Like [`call_with_locals`], but also provides the `this` object context
    /// returned by `Expr::This`. Pass `Value::Object(id)` for an object context
    /// or `Value::Nil` for no context.
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
        .with_this(this);
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
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

    pub fn has_effect_callback(&self, effect_name: &str, event: &str) -> bool {
        let mut function_name = String::with_capacity(effect_name.len() + event.len() + 2);
        function_name.push_str("Fx");
        function_name.push_str(effect_name);
        function_name.push_str(event);
        self.has_function(&function_name)
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
