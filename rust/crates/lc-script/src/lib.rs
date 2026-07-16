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
pub use crate::ast::{AppendTo, Function, TypeAnnotation};
pub use crate::ast::{VarDecl, VarDeclKind};
pub use crate::engine::{
    new_global_slots, new_global_variables, register_global_declarations, Engine, GlobalSlots,
    GlobalVariables, MethodReferenceDispatch, Script, ScriptFunctionResolution,
    ScriptFunctionScope,
};
pub use crate::vm::{
    caller_host_identity, caller_is_temporary_script, caller_origin_strictness, caller_strictness,
    caller_uses_engine_scope, caller_var_slots, value_cell, CallerVarSlots, HostCallArg,
    HostCallerStrictness, LocalCells, ScriptHostIdentity, ValueCell,
    ValueReference,
};
pub use crate::error::{ParseError, RuntimeError, ScriptError};
pub use crate::value::{
    c4_hash_combine, c4_id_from_raw, c4_id_parse, c4_id_raw, c4_id_serde, c4_id_text,
    c4_optional_id_serde, c4_optional_string_serde, c4_string_byte, c4_string_byte_len,
    c4_string_bytes, c4_string_from_bytes, c4_string_serde, c4_strings_equal, cnv_fn,
    C4VType, CnvFn, Value, ValueMap,
};

mod value;
