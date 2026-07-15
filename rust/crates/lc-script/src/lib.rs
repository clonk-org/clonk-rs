mod ast;
mod debugger;
mod engine;
mod error;
#[cfg(feature = "ffi")]
mod ffi;
mod lexer;
mod parser;
mod token;
mod vm;

pub use crate::debugger::DebuggerHooks;
pub use crate::ast::{AppendTo, Function};
pub use crate::ast::{VarDecl, VarDeclKind};
pub use crate::engine::{
    new_global_slots, new_global_variables, register_global_declarations, Engine, GlobalSlots,
    GlobalVariables, MethodReferenceDispatch, Script, ScriptFunctionResolution,
    ScriptFunctionScope,
};
pub use crate::vm::{
    caller_host_identity, caller_origin_strictness, caller_strictness, caller_uses_engine_scope,
    caller_var_slots, value_cell, CallerVarSlots, HostCallArg, HostCallerStrictness, LocalCells,
    ScriptHostIdentity, ValueCell,
    ValueReference,
};
pub use crate::error::{ParseError, RuntimeError, ScriptError};
pub use crate::value::{c4_hash_combine, cnv_fn, C4VType, CnvFn, Value, ValueMap};

mod value;
