use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::{Function, Script as AstScript, VarDecl};
use crate::debugger::DebuggerHooks;
use crate::error::{ParseError, RuntimeError, ScriptError};
use crate::parser::Parser;
use crate::value::Value;
use crate::vm::Vm;

pub type HostFunction = Arc<dyn Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync>;

#[derive(Clone)]
pub struct Script {
    functions: HashMap<String, Function>,
    includes: Vec<String>,
    appendto: Option<crate::ast::AppendTo>,
    strict_level: Option<u8>,
    var_decls: Vec<VarDecl>, // Script-level variable declarations
}

impl Default for Script {
    fn default() -> Self {
        Self {
            functions: HashMap::new(),
            includes: Vec::new(),
            appendto: None,
            strict_level: None,
            var_decls: Vec::new(),
        }
    }
}

impl Script {
    pub fn compile(source: &str) -> Result<Self, ParseError> {
        let mut parser = Parser::new(source);
        let ast = parser.parse_script()?;
        Ok(Self::from_ast(ast))
    }

    fn from_ast(ast: AstScript) -> Self {
        let mut functions = HashMap::new();
        for function in ast.functions {
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
}

impl Engine {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            host_functions: HashMap::new(),
            debugger_hooks: None,
            var_decls: Vec::new(),
        }
    }

    pub fn load_script(&mut self, source: &str) -> Result<(), ScriptError> {
        let script = Script::compile(source)?;
        self.add_script(script);
        Ok(())
    }

    pub fn add_script(&mut self, script: Script) {
        for (name, function) in script.functions.into_iter() {
            self.functions.insert(name, function);
        }
        // Store local variable declarations from the script
        self.var_decls.extend(script.var_decls);
    }

    pub fn merge_from(&mut self, other: &Engine) {
        for (name, function) in other.functions.iter() {
            // Only add if not already defined (child overrides parent)
            self.functions
                .entry(name.clone())
                .or_insert_with(|| function.clone());
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
        );
        vm.call(name, args).map_err(ScriptError::from)
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
        );
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    /// Like [`call_with_locals`], but also provides the `this` object context
    /// returned by `Expr::This`. The value is host-opaque (lc-engine passes an
    /// object reference proplist); pass `Value::Nil` for no context.
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
        .with_this(this);
        vm.call_with_locals(name, args, local_vars)
            .map_err(ScriptError::from)
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
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
