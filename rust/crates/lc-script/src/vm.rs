use std::collections::HashMap;

use crate::ast::{AssignmentTarget, BinaryOp, Expr, Function, Stmt, UnaryOp};
use crate::debugger::DebuggerHooks;
use crate::engine::HostFunction;
use crate::error::RuntimeError;
use crate::value::{Literal, Value};

const MAX_CALL_DEPTH: usize = 64;

pub struct Vm<'a> {
    functions: &'a HashMap<String, Function>,
    host_functions: &'a HashMap<String, HostFunction>,
    debugger: Option<DebuggerHooks>,
}

impl<'a> Vm<'a> {
    pub fn new(
        functions: &'a HashMap<String, Function>,
        host_functions: &'a HashMap<String, HostFunction>,
        debugger: Option<DebuggerHooks>,
    ) -> Self {
        Self {
            functions,
            host_functions,
            debugger,
        }
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
        self.invoke(name, args, 0)
    }

    fn invoke(&self, name: &str, args: &[Value], depth: usize) -> Result<Value, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        if let Some(function) = self.functions.get(name) {
            return self.invoke_script_function(name, function, args, depth);
        }

        if let Some(function) = self.host_functions.get(name) {
            return self.invoke_host_function(name, function, args);
        }

        Err(RuntimeError::new(format!("unknown function '{name}'")))
    }

    fn invoke_script_function(
        &self,
        name: &str,
        function: &Function,
        args: &[Value],
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        if args.len() != function.params.len() {
            return Err(RuntimeError::new(format!(
                "function '{name}' expects {} arguments but received {}",
                function.params.len(),
                args.len()
            )));
        }

        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_call() {
                callback(name, args);
            }
        }

        let mut env = Environment::new_with_params(&function.params, args);
        let result = self.execute_statements(&function.body, &mut env, depth)?;
        let value = result.unwrap_or(Value::Nil);

        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &value);
            }
        }

        Ok(value)
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
    ) -> Result<Option<Value>, RuntimeError> {
        for statement in statements {
            match self.execute_statement(statement, env, depth)? {
                ControlFlow::Continue => continue,
                ControlFlow::Return(value) => return Ok(Some(value)),
            }
        }
        Ok(None)
    }

    fn execute_statement(
        &self,
        statement: &Stmt,
        env: &mut Environment,
        depth: usize,
    ) -> Result<ControlFlow, RuntimeError> {
        match statement {
            Stmt::VarDecl { name, init } => {
                let value = match init {
                    Some(expr) => self.evaluate(expr, env, depth)?,
                    None => Value::Nil,
                };
                env.define(name, value);
                Ok(ControlFlow::Continue)
            }
            Stmt::Assignment { target, value } => {
                let evaluated = self.evaluate(value, env, depth)?;
                self.assign_target(env, target, evaluated)?;
                Ok(ControlFlow::Continue)
            }
            Stmt::Return(expr) => {
                let value = match expr {
                    Some(expr) => self.evaluate(expr, env, depth)?,
                    None => Value::Nil,
                };
                Ok(ControlFlow::Return(value))
            }
            Stmt::Expr(expr) => {
                self.evaluate(expr, env, depth)?;
                Ok(ControlFlow::Continue)
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.evaluate(condition, env, depth)?.as_bool() {
                    if let Some(value) = self
                        .execute_block(then_branch, env, depth)?
                        .map(ControlFlow::Return)
                    {
                        return Ok(value);
                    }
                } else if let Some(branch) = else_branch {
                    if let Some(value) = self
                        .execute_block(branch, env, depth)?
                        .map(ControlFlow::Return)
                    {
                        return Ok(value);
                    }
                }
                Ok(ControlFlow::Continue)
            }
            Stmt::While { condition, body } => {
                while self.evaluate(condition, env, depth)?.as_bool() {
                    if let Some(value) = self
                        .execute_block(body, env, depth)?
                        .map(ControlFlow::Return)
                    {
                        return Ok(value);
                    }
                }
                Ok(ControlFlow::Continue)
            }
            Stmt::Block(statements) => self.execute_block(statements, env, depth).map(|opt| {
                opt.map(ControlFlow::Return)
                    .unwrap_or(ControlFlow::Continue)
            }),
        }
    }

    fn execute_block(
        &self,
        statements: &[Stmt],
        env: &mut Environment,
        depth: usize,
    ) -> Result<Option<Value>, RuntimeError> {
        env.push_scope();
        let result = self.execute_statements(statements, env, depth);
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
            Expr::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'"))),
            Expr::Unary(op, expr) => {
                let value = self.evaluate(expr, env, depth)?;
                self.eval_unary(op, value)
            }
            Expr::Binary(lhs, op, rhs) => {
                let left = self.evaluate(lhs, env, depth)?;
                if matches!(op, BinaryOp::And) {
                    if !left.as_bool() {
                        return Ok(Value::Bool(false));
                    }
                    let right = self.evaluate(rhs, env, depth)?;
                    return Ok(Value::Bool(right.as_bool()));
                }
                if matches!(op, BinaryOp::Or) {
                    if left.as_bool() {
                        return Ok(Value::Bool(true));
                    }
                    let right = self.evaluate(rhs, env, depth)?;
                    return Ok(Value::Bool(right.as_bool()));
                }
                let right = self.evaluate(rhs, env, depth)?;
                self.eval_binary(left, op, right)
            }
            Expr::Call { callee, args } => {
                let mut evaluated_args = Vec::with_capacity(args.len());
                for arg in args {
                    evaluated_args.push(self.evaluate(arg, env, depth + 1)?);
                }
                // Extract function name from callee expression
                // For now, we support Variable and Property expressions
                match callee.as_ref() {
                    Expr::Variable(name) => {
                        self.invoke(name, &evaluated_args, depth + 1)
                    }
                    Expr::Property(_base, name) => {
                        // For now, just call the method name directly
                        // TODO: Implement proper object method dispatch when we have object support
                        self.invoke(name, &evaluated_args, depth + 1)
                    }
                    _ => Err(RuntimeError::new(format!(
                        "cannot call non-function expression: {:?}",
                        callee
                    ))),
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
        }
    }

    fn literal_value(&self, literal: &Literal) -> Value {
        match literal {
            Literal::Int(i) => Value::Int(*i),
            Literal::Bool(b) => Value::Bool(*b),
            Literal::String(s) => Value::String(s.clone()),
            Literal::Nil => Value::Nil,
        }
    }

    fn eval_unary(&self, op: &UnaryOp, value: Value) -> Result<Value, RuntimeError> {
        match op {
            UnaryOp::Negate => match value {
                Value::Int(i) => Ok(Value::Int(-i)),
                other => Err(RuntimeError::new(format!(
                    "cannot apply unary '-' to {}",
                    other.type_name()
                ))),
            },
            UnaryOp::Not => Ok(Value::Bool(!value.as_bool())),
        }
    }

    fn eval_binary(&self, left: Value, op: &BinaryOp, right: Value) -> Result<Value, RuntimeError> {
        use BinaryOp::*;
        match op {
            Add => self.eval_add(left, right),
            Sub => self.eval_int_op(left, right, |a, b| a - b, "-"),
            Mul => self.eval_int_op(left, right, |a, b| a * b, "*"),
            Div => {
                let rhs = match right {
                    Value::Int(i) => i,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot apply '/' to operands of type int and {}",
                            other.type_name()
                        )))
                    }
                };
                if rhs == 0 {
                    return Err(RuntimeError::new("division by zero"));
                }
                match left {
                    Value::Int(lhs) => Ok(Value::Int(lhs / rhs)),
                    other => Err(RuntimeError::new(format!(
                        "cannot apply '/' to operands of type {} and int",
                        other.type_name()
                    ))),
                }
            }
            Mod => {
                let rhs = match right {
                    Value::Int(i) => i,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot apply '%' to operands of type int and {}",
                            other.type_name()
                        )))
                    }
                };
                if rhs == 0 {
                    return Err(RuntimeError::new("modulo by zero"));
                }
                match left {
                    Value::Int(lhs) => Ok(Value::Int(lhs % rhs)),
                    other => Err(RuntimeError::new(format!(
                        "cannot apply '%' to operands of type {} and int",
                        other.type_name()
                    ))),
                }
            }
            Equal => Ok(Value::Bool(self.values_equal(&left, &right))),
            NotEqual => Ok(Value::Bool(!self.values_equal(&left, &right))),
            Less => self.eval_int_cmp(left, right, |a, b| a < b, "<"),
            LessEqual => self.eval_int_cmp(left, right, |a, b| a <= b, "<="),
            Greater => self.eval_int_cmp(left, right, |a, b| a > b, ">"),
            GreaterEqual => self.eval_int_cmp(left, right, |a, b| a >= b, ">="),
            And | Or => unreachable!(),
        }
    }

    fn eval_add(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
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
            (a, b) => Err(RuntimeError::new(format!(
                "cannot apply '+' to operands of type {} and {}",
                a.type_name(),
                b.type_name()
            ))),
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
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(op(a, b))),
            (a, b) => Err(RuntimeError::new(format!(
                "cannot apply '{symbol}' to operands of type {} and {}",
                a.type_name(),
                b.type_name()
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
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(cmp(a, b))),
            (a, b) => Err(RuntimeError::new(format!(
                "cannot apply '{symbol}' to operands of type {} and {}",
                a.type_name(),
                b.type_name()
            ))),
        }
    }

    fn values_equal(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
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

    fn assign_target(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match target {
            AssignmentTarget::Variable(name) => env.assign(name, value),
            AssignmentTarget::Property(base, property) => {
                let base_value = self.assignment_target_value(env, base)?;
                let mut entries = match base_value {
                    Value::Proplist(entries) => entries,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot assign property '{property}' on value of type {}",
                            other.type_name()
                        )))
                    }
                };
                entries.insert(property.clone(), value);
                self.assign_target(env, base, Value::Proplist(entries))
            }
        }
    }

    fn assignment_target_value(
        &self,
        env: &mut Environment,
        target: &AssignmentTarget,
    ) -> Result<Value, RuntimeError> {
        match target {
            AssignmentTarget::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| RuntimeError::new(format!("undefined variable '{name}'"))),
            AssignmentTarget::Property(base, property) => {
                let container = self.assignment_target_value(env, base)?;
                match container {
                    Value::Proplist(entries) => {
                        Ok(entries.get(property).cloned().unwrap_or(Value::Nil))
                    }
                    other => Err(RuntimeError::new(format!(
                        "cannot access property '{property}' on value of type {}",
                        other.type_name()
                    ))),
                }
            }
        }
    }
}

enum ControlFlow {
    Continue,
    Return(Value),
}

struct Environment {
    scopes: Vec<HashMap<String, Value>>,
}

impl Environment {
    fn new_with_params(params: &[String], args: &[Value]) -> Self {
        let mut scopes = vec![HashMap::new()];
        let base = scopes.last_mut().unwrap();
        for (param, value) in params.iter().zip(args.iter()) {
            base.insert(param.clone(), value.clone());
        }
        Self { scopes }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: &str, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    fn assign(&mut self, name: &str, value: Value) -> Result<(), RuntimeError> {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return Ok(());
            }
        }
        Err(RuntimeError::new(format!("undefined variable '{name}'")))
    }

    fn get(&self, name: &str) -> Option<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value);
            }
        }
        None
    }
}
