use std::collections::HashMap;

use crate::ast::{Function, Script as AstScript};
use crate::debugger::DebuggerHooks;
use crate::error::{ParseError, ScriptError};
use crate::parser::Parser;
use crate::value::Value;
use crate::vm::Vm;

#[derive(Clone, Default)]
pub struct Script {
    functions: HashMap<String, Function>,
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
        Self { functions }
    }

    pub fn functions(&self) -> &HashMap<String, Function> {
        &self.functions
    }
}

pub struct Engine {
    functions: HashMap<String, Function>,
    debugger_hooks: Option<DebuggerHooks>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
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

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, ScriptError> {
        let vm = Vm::new(&self.functions, self.debugger_hooks.clone());
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
