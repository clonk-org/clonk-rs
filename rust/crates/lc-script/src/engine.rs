use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::{Function, Script as AstScript};
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
}

impl Default for Script {
    fn default() -> Self {
        Self {
            functions: HashMap::new(),
            includes: Vec::new(),
            appendto: None,
            strict_level: None,
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

pub struct Engine {
    functions: HashMap<String, Function>,
    host_functions: HashMap<String, HostFunction>,
    debugger_hooks: Option<DebuggerHooks>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            host_functions: HashMap::new(),
            debugger_hooks: None,
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
            self.debugger_hooks.clone(),
        );
        vm.call(name, args).map_err(ScriptError::from)
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
