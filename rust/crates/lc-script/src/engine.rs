use std::collections::HashMap;

use crate::ast::{Function, Script as AstScript};
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
}

impl Engine {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
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
        let vm = Vm::new(&self.functions);
        vm.call(name, args).map_err(ScriptError::from)
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
