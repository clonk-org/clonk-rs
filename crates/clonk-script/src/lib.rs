mod ast;
mod debugger;
mod engine;
mod error;
mod lexer;
mod parser;
mod token;
mod vm;

pub use crate::ast::{AppendTo, Function, TypeAnnotation};
pub use crate::ast::{VarDecl, VarDeclKind};
pub use crate::debugger::DebuggerHooks;
pub use crate::engine::{
    c4_string_registration_order, clear_c4_string_holds, enumerate_c4_strings, new_global_slots,
    new_global_variables, new_string_registrations, register_c4_literal_string,
    register_c4_referenced_string, register_c4_string, register_c4_value_strings,
    register_global_declarations, register_global_declarations_with_strings,
    register_loaded_c4_string, resolve_c4_string, save_current_c4_string_enumeration,
    DirectCallFunctionProbe, Engine, GlobalSlots, GlobalVariables, HostRegistrationSnapshot,
    MethodRefArgsDispatch, MethodReferenceDispatch, ReferenceParameterProbe, Script,
    ScriptFunctionResolution, ScriptFunctionScope, StaticConstLinkError, StringRegistrationLedger,
    StringRegistrations,
};
pub use crate::error::{ParseError, RuntimeCallFrame, RuntimeError, ScriptError};
pub use crate::value::{
    c4_hash_combine, c4_id_from_raw, c4_id_parse, c4_id_raw, c4_id_serde, c4_id_text,
    c4_optional_id_serde, c4_optional_string_serde, c4_string_byte, c4_string_byte_len,
    c4_string_bytes, c4_string_bytes_cow, c4_string_from_bytes, c4_string_serde, c4_strings_equal,
    cnv_fn, C4StringValue, C4VType, CnvFn, Value, ValueMap,
};
pub use crate::vm::{
    active_direct_exec_diagnostic_frames, caller_host_identity, caller_is_temporary_script,
    caller_origin_strictness, caller_strictness, caller_uses_engine_scope, caller_var_slots,
    start_call_trace, start_script_profiler, stop_script_profiler, value_cell,
    with_diagnostic_object_formatter, CallerVarSlots, HostCallArg, HostCallerStrictness,
    LocalCells, ScriptHostIdentity, ScriptProfileEntry, ValueCell, ValueReference,
};

mod value;
