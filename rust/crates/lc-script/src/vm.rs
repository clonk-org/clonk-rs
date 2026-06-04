use std::collections::HashMap;

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

pub struct Vm<'a> {
    functions: &'a HashMap<String, Function>,
    host_functions: &'a HashMap<String, HostFunction>,
    var_decls: &'a [VarDecl], // Script-level variable declarations
    debugger: Option<DebuggerHooks>,
    /// The object context the call runs on, returned by `Expr::This`. Host-opaque
    /// (in lc-engine an object reference is `Proplist {"id": <number>}`). Nil when
    /// the call has no object context (e.g. global functions).
    this_value: Value,
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
            this_value: Value::Nil,
        }
    }

    /// Set the `this` object context for this call session. Nested plain calls
    /// share it (they run on the same object).
    pub fn with_this(mut self, this: Value) -> Self {
        self.this_value = this;
        self
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
        self.invoke(name, args, 0)
    }

    /// Call a function with per-object local variable context
    /// Returns (result, updated_local_vars)
    pub fn call_with_locals(
        &self,
        name: &str,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
    ) -> Result<(Value, HashMap<String, Value>), RuntimeError> {
        self.invoke_with_locals(name, args, local_vars, 0)
    }

    fn invoke_with_locals(
        &self,
        name: &str,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
        depth: usize,
    ) -> Result<(Value, HashMap<String, Value>), RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        // Grow the native stack on demand so deep (but C++-legal) recursion in
        // this tree-walking interpreter doesn't overflow the thread stack.
        maybe_grow(|| {
            if let Some(function) = self.functions.get(name) {
                return self
                    .invoke_script_function_with_locals(name, function, args, local_vars, depth);
            }

            if let Some(function) = self.host_functions.get(name) {
                // Host functions don't modify local variables
                let result = self.invoke_host_function(name, function, args)?;
                return Ok((result, local_vars.clone()));
            }

            Err(RuntimeError::new(format!("unknown function '{name}'")))
        })
    }

    fn invoke(&self, name: &str, args: &[Value], depth: usize) -> Result<Value, RuntimeError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new("maximum call depth exceeded"));
        }

        maybe_grow(|| {
            if let Some(function) = self.functions.get(name) {
                return self.invoke_script_function(name, function, args, depth);
            }

            if let Some(function) = self.host_functions.get(name) {
                return self.invoke_host_function(name, function, args);
            }

            Err(RuntimeError::new(format!("unknown function '{name}'")))
        })
    }

    fn invoke_script_function_with_locals(
        &self,
        name: &str,
        function: &Function,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
        depth: usize,
    ) -> Result<(Value, HashMap<String, Value>), RuntimeError> {
        // Allow calling with MORE arguments than declared (extras ignored)
        // This matches C++ OpenClonk behavior for action callbacks
        if args.len() < function.params.len() {
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

        let mut env = Environment::new_with_params(&function.params, args, function.strict_level);

        // Initialize script-level local variables from the per-object storage
        // Use passed-in values instead of always initializing to nil
        for var_decl in self.var_decls {
            let value = local_vars
                .get(&var_decl.name)
                .cloned()
                .unwrap_or(Value::Nil);
            env.define(&var_decl.name, value);
        }

        // Restore the object's numeric Local(n) slots, which persist across calls
        // like named locals (C++ pObj->Local). They round-trip through local_vars
        // under "__local_{n}" keys.
        for (key, value) in local_vars {
            if let Some(idx) = key
                .strip_prefix("__local_")
                .and_then(|s| s.parse::<i32>().ok())
            {
                env.local_slots.insert(idx, value.clone());
            }
        }

        let result = self.execute_statements(&function.body, &mut env, depth)?;
        let value = match result {
            ControlFlow::Return(v) => v,
            ControlFlow::Normal => Value::Nil,
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

        if let Some(debugger) = &self.debugger {
            if let Some(callback) = debugger.on_return() {
                callback(name, &value);
            }
        }

        // Extract updated local variable values from the environment
        let mut updated_locals = HashMap::new();
        for var_decl in self.var_decls {
            if let Some(val) = env.get(&var_decl.name) {
                updated_locals.insert(var_decl.name.clone(), val.clone());
            }
        }
        // Persist the object's numeric Local(n) slots back to local_vars.
        for (idx, slot_value) in &env.local_slots {
            updated_locals.insert(format!("__local_{idx}"), slot_value.clone());
        }

        Ok((value, updated_locals))
    }

    fn invoke_script_function(
        &self,
        name: &str,
        function: &Function,
        args: &[Value],
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        // Allow calling with MORE arguments than declared (extras ignored)
        // This matches C++ OpenClonk behavior for action callbacks
        if args.len() < function.params.len() {
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

        let mut env = Environment::new_with_params(&function.params, args, function.strict_level);

        // Initialize script-level local variables to nil
        // This matches C++ engine behavior where local variables default to nil
        for var_decl in self.var_decls {
            env.define(&var_decl.name, Value::Nil);
        }

        let result = self.execute_statements(&function.body, &mut env, depth)?;
        let value = match result {
            ControlFlow::Return(v) => v,
            ControlFlow::Normal => Value::Nil,
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
    ) -> Result<ControlFlow, RuntimeError> {
        for statement in statements {
            match self.execute_statement(statement, env, depth)? {
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
    ) -> Result<ControlFlow, RuntimeError> {
        match statement {
            Stmt::VarDecl { name, init } => {
                let value = match init {
                    Some(expr) => self.evaluate(expr, env, depth)?,
                    None => Value::Nil,
                };
                env.define(name, value);
                Ok(ControlFlow::Normal)
            }
            Stmt::Assignment { target, value } => {
                let evaluated = self.evaluate(value, env, depth)?;
                self.assign_target(env, target, evaluated)?;
                Ok(ControlFlow::Normal)
            }
            Stmt::Return(expr) => {
                let value = match expr {
                    Some(expr) => self.evaluate(expr, env, depth)?,
                    None => Value::Nil,
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
                    return self.execute_block(then_branch, env, depth);
                } else if let Some(branch) = else_branch {
                    return self.execute_block(branch, env, depth);
                }
                Ok(ControlFlow::Normal)
            }
            Stmt::While { condition, body } => {
                while self.evaluate(condition, env, depth)?.as_bool() {
                    match self.execute_block(body, env, depth)? {
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
                    match self.execute_block(body, env, depth)? {
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
                    match self.execute_block(body, env, depth)? {
                        ControlFlow::Normal => {}
                        ControlFlow::LoopContinue => continue,
                        ControlFlow::Break => break,
                        ControlFlow::Return(value) => return Ok(ControlFlow::Return(value)),
                    }
                }

                Ok(ControlFlow::Normal)
            }
            Stmt::Block(statements) => self.execute_block(statements, env, depth),
            Stmt::Sequence(statements) => {
                // Execute statements sequentially WITHOUT creating a new scope
                // Used for multi-variable declarations
                self.execute_statements(statements, env, depth)
            }
        }
    }

    fn execute_block(
        &self,
        statements: &[Stmt],
        env: &mut Environment,
        depth: usize,
    ) -> Result<ControlFlow, RuntimeError> {
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
            // `this` yields the object context the call runs on (host-provided),
            // mirroring C4Script's `this` (C4V_C4Object); Nil for global calls.
            Expr::This => Ok(self.this_value.clone()),
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
                // && and || are Lua-style: they return the surviving operand
                // value unchanged, not a coerced bool (C4AulExec.cpp:999-1021,
                // AB_JUMPAND/AB_JUMPOR leave the operand on the stack).
                if matches!(op, BinaryOp::And) {
                    if !left.as_bool() {
                        return Ok(left);
                    }
                    return self.evaluate(rhs, env, depth);
                }
                if matches!(op, BinaryOp::Or) {
                    if left.as_bool() {
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
                        Expr::Property(_base, name) => {
                            // Try to find the function
                            if !self.functions.contains_key(name)
                                && !self.host_functions.contains_key(name)
                            {
                                // Method doesn't exist - return nil without evaluating args
                                return Ok(Value::Nil);
                            }
                            // Method exists, evaluate args and call it
                            let mut evaluated_args = Vec::with_capacity(args.len());
                            for arg in args {
                                evaluated_args.push(self.evaluate(arg, env, depth + 1)?);
                            }
                            // TODO: Handle forward_rest - append remaining args from current function
                            if *forward_rest {
                                // For now, just ignore - proper implementation needs access to current call args
                            }
                            // If the call fails for other reasons (arity, runtime error), propagate
                            self.invoke(name, &evaluated_args, depth + 1)
                        }
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
                            && args.len() == 1
                            && !self.functions.contains_key(name)
                            && !self.host_functions.contains_key(name)
                        {
                            let index = Box::new(args[0].clone());
                            let target = if name == "Var" {
                                AssignmentTarget::VarSlot(index)
                            } else {
                                AssignmentTarget::LocalSlot(index)
                            };
                            return self.get_target_value(env, &target);
                        }
                    }
                    // Normal call - evaluate args first, then invoke
                    let mut evaluated_args = Vec::with_capacity(args.len());
                    for arg in args {
                        evaluated_args.push(self.evaluate(arg, env, depth + 1)?);
                    }
                    // TODO: Handle forward_rest - append remaining args from current function
                    if *forward_rest {
                        // For now, just ignore - proper implementation needs access to current call args
                    }
                    // Extract function name from callee expression
                    match callee.as_ref() {
                        Expr::Variable(name) => self.invoke(name, &evaluated_args, depth + 1),
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
                let new_value = match old_value {
                    Value::Int(i) => Value::Int(i + 1),
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot increment non-integer value: {:?}",
                            other
                        )))
                    }
                };
                self.assign_target(env, &target, new_value.clone())?;
                Ok(new_value)
            }
            Expr::PreDecrement(expr) => {
                let target = Self::expr_to_assignment_target(expr)?;
                let old_value = self.get_target_value(env, &target)?;
                let new_value = match old_value {
                    Value::Int(i) => Value::Int(i - 1),
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot decrement non-integer value: {:?}",
                            other
                        )))
                    }
                };
                self.assign_target(env, &target, new_value.clone())?;
                Ok(new_value)
            }
            Expr::PostIncrement(expr) => {
                let target = Self::expr_to_assignment_target(expr)?;
                let old_value = self.get_target_value(env, &target)?;
                let new_value = match &old_value {
                    Value::Int(i) => Value::Int(i + 1),
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot increment non-integer value: {:?}",
                            other
                        )))
                    }
                };
                self.assign_target(env, &target, new_value)?;
                Ok(old_value)
            }
            Expr::PostDecrement(expr) => {
                let target = Self::expr_to_assignment_target(expr)?;
                let old_value = self.get_target_value(env, &target)?;
                let new_value = match &old_value {
                    Value::Int(i) => Value::Int(i - 1),
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot decrement non-integer value: {:?}",
                            other
                        )))
                    }
                };
                self.assign_target(env, &target, new_value)?;
                Ok(old_value)
            }
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
            AssignmentTarget::Index(base, index_expr) => {
                let base_value = self.assignment_target_value(env, base)?;
                let index_value = self.evaluate(index_expr, env, 0)?;
                let index_int = match index_value {
                    Value::Int(n) => n,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "array index must be an integer, got {}",
                            other.type_name()
                        )))
                    }
                };
                let mut elements = match base_value {
                    Value::Array(elements) => elements,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot index into value of type {}",
                            other.type_name()
                        )))
                    }
                };
                // Grow array if necessary
                let index = index_int as usize;
                if index >= elements.len() {
                    elements.resize(index + 1, Value::Nil);
                }
                elements[index] = value;
                self.assign_target(env, base, Value::Array(elements))
            }
            AssignmentTarget::LocalSlot(index_expr) => {
                // Evaluate the index expression
                let index_value = self.evaluate(index_expr, env, 0)?;
                let index = match index_value {
                    Value::Int(n) => n,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "Local() index must be an integer, got {}",
                            other.type_name()
                        )))
                    }
                };
                // Object numeric local slot (C++ pObj->Local[n]), persisted via
                // local_vars; function-scoped, negative index clamped to 0.
                env.set_local_slot(index, value);
                Ok(())
            }
            AssignmentTarget::VarSlot(index_expr) => {
                // Evaluate the index expression
                let index_value = self.evaluate(index_expr, env, 0)?;
                let index = match index_value {
                    Value::Int(n) => n,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "Var() index must be an integer, got {}",
                            other.type_name()
                        )))
                    }
                };
                // Per-call numeric scratch slot (C++ NumVars[n]); function-scoped,
                // negative index clamped to 0.
                env.set_var_slot(index, value);
                Ok(())
            }
            AssignmentTarget::EffectSlot(args) => {
                // Evaluate all arguments to create the slot identifier
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                // Store in environment with special naming scheme
                // Format: __effect_{index}_{target_id}_{effect_num}
                // For simplicity, join all args with underscores
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
            AssignmentTarget::FunctionCall { name, args } => {
                // Call the reference-returning function to get the lvalue reference
                // For now, implement a simplified version using slot naming
                // TODO: Properly implement reference semantics
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                // Store using a naming scheme based on the function name and arguments
                let arg_str = arg_values
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => n.to_string(),
                        Value::String(s) => s.clone(),
                        _ => format!("{:?}", v),
                    })
                    .collect::<Vec<_>>()
                    .join("_");
                let slot_name = format!("__funcref_{}_{}", name, arg_str);
                env.define(&slot_name, value);
                Ok(())
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
            AssignmentTarget::Index(base, index_expr) => {
                let container = self.assignment_target_value(env, base)?;
                let index_value = self.evaluate(index_expr, env, 0)?;
                let index_int = match index_value {
                    Value::Int(n) => n,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "array index must be an integer, got {}",
                            other.type_name()
                        )))
                    }
                };
                match container {
                    Value::Array(elements) => {
                        let index = index_int as usize;
                        Ok(elements.get(index).cloned().unwrap_or(Value::Nil))
                    }
                    other => Err(RuntimeError::new(format!(
                        "cannot index into value of type {}",
                        other.type_name()
                    ))),
                }
            }
            AssignmentTarget::LocalSlot(index_expr) => {
                // Evaluate the index expression
                let index_value = self.evaluate(index_expr, env, 0)?;
                let index = match index_value {
                    Value::Int(n) => n,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "Local() index must be an integer, got {}",
                            other.type_name()
                        )))
                    }
                };
                Ok(env.get_local_slot(index))
            }
            AssignmentTarget::VarSlot(index_expr) => {
                // Evaluate the index expression
                let index_value = self.evaluate(index_expr, env, 0)?;
                let index = match index_value {
                    Value::Int(n) => n,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "Var() index must be an integer, got {}",
                            other.type_name()
                        )))
                    }
                };
                Ok(env.get_var_slot(index))
            }
            AssignmentTarget::EffectSlot(args) => {
                // Evaluate all arguments to create the slot identifier
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                // Retrieve from environment with special naming scheme
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
                Ok(env.get(&slot_name).cloned().unwrap_or(Value::Nil))
            }
            AssignmentTarget::MethodSlot {
                object,
                method,
                args,
            } => {
                // Evaluate the object to get its identity
                let object_value = self.evaluate(object, env, 0)?;
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
                Ok(env.get(&slot_name).cloned().unwrap_or(Value::Nil))
            }
            AssignmentTarget::FunctionCall { name, args } => {
                // Retrieve the value stored for this reference-returning function call
                let mut arg_values = Vec::new();
                for arg in args {
                    arg_values.push(self.evaluate(arg, env, 0)?);
                }
                let arg_str = arg_values
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => n.to_string(),
                        Value::String(s) => s.clone(),
                        _ => format!("{:?}", v),
                    })
                    .collect::<Vec<_>>()
                    .join("_");
                let slot_name = format!("__funcref_{}_{}", name, arg_str);
                Ok(env.get(&slot_name).cloned().unwrap_or(Value::Nil))
            }
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
            Expr::Index(_, _) => {
                // Index expressions are not yet supported as assignment targets
                // This would require extending AssignmentTarget to include Index variant
                Err(RuntimeError::new(
                    "index expressions as increment/decrement targets not yet supported"
                        .to_string(),
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
                        if name == "Local" && args.len() == 1 {
                            return Ok(AssignmentTarget::LocalSlot(Box::new(args[0].clone())));
                        } else if name == "Var" && args.len() == 1 {
                            return Ok(AssignmentTarget::VarSlot(Box::new(args[0].clone())));
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
    Return(Value),
}

struct Environment {
    scopes: Vec<HashMap<String, Value>>,
    /// `#strict` level of the executing function, for level-correct `==`/`!=`.
    strict_level: Option<u8>,
    /// C4Script numeric scratch slots, addressed by `Var(n)` / `Local(n)`. These
    /// are SEPARATE from named variables (C++ `NumVars` and the object `Local`
    /// array, not `Vars`/`LocalNamed`) and are function-scoped, not block-scoped,
    /// so a `Local(0) = x` inside a block stays visible after it. Unset reads as
    /// nil and the index is clamped to >= 0 (C4ValueList::GetItem). `var_slots`
    /// are per-call; `local_slots` round-trip through the object's `local_vars`.
    var_slots: HashMap<i32, Value>,
    local_slots: HashMap<i32, Value>,
}

impl Environment {
    fn new_with_params(params: &[Parameter], args: &[Value], strict_level: Option<u8>) -> Self {
        let mut scopes = vec![HashMap::new()];
        let base = scopes.last_mut().unwrap();
        for (param, value) in params.iter().zip(args.iter()) {
            base.insert(param.name.clone(), value.clone());
        }
        Self {
            scopes,
            strict_level,
            var_slots: HashMap::new(),
            local_slots: HashMap::new(),
        }
    }

    fn set_var_slot(&mut self, index: i32, value: Value) {
        self.var_slots.insert(index.max(0), value);
    }

    fn get_var_slot(&self, index: i32) -> Value {
        self.var_slots
            .get(&index.max(0))
            .cloned()
            .unwrap_or(Value::Nil)
    }

    fn set_local_slot(&mut self, index: i32, value: Value) {
        self.local_slots.insert(index.max(0), value);
    }

    fn get_local_slot(&self, index: i32) -> Value {
        self.local_slots
            .get(&index.max(0))
            .cloned()
            .unwrap_or(Value::Nil)
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
