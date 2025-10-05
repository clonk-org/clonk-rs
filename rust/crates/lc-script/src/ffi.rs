use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::engine::Engine;
use crate::value::Value;

pub struct EngineHandle(Engine);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LcScriptValueKind {
    Nil = 0,
    Int = 1,
    Bool = 2,
    String = 3,
}

#[repr(C)]
pub struct LcScriptValue {
    pub kind: LcScriptValueKind,
    pub int_value: c_int,
    pub bool_value: bool,
    pub string_value: *mut c_char,
}

impl Default for LcScriptValue {
    fn default() -> Self {
        Self {
            kind: LcScriptValueKind::Nil,
            int_value: 0,
            bool_value: false,
            string_value: ptr::null_mut(),
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_script_engine_new() -> *mut EngineHandle {
    Box::into_raw(Box::new(EngineHandle(Engine::new())))
}

#[no_mangle]
pub extern "C" fn lc_script_engine_free(handle: *mut EngineHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_script_engine_load(handle: *mut EngineHandle, source: *const c_char) -> bool {
    if handle.is_null() || source.is_null() {
        return false;
    }
    let engine = unsafe { &mut *handle };
    let c_str = unsafe { CStr::from_ptr(source) };
    let source = c_str.to_string_lossy();
    engine.0.load_script(source.as_ref()).is_ok()
}

#[no_mangle]
pub extern "C" fn lc_script_engine_call(
    handle: *mut EngineHandle,
    name: *const c_char,
    args: *const LcScriptValue,
    arg_len: usize,
    out_value: *mut LcScriptValue,
) -> bool {
    if handle.is_null() || name.is_null() || out_value.is_null() {
        return false;
    }
    let engine = unsafe { &mut *handle };
    let name = unsafe { CStr::from_ptr(name) };
    let name = match name.to_str() {
        Ok(value) => value,
        Err(_) => return false,
    };
    let arg_slice = if arg_len == 0 {
        &[]
    } else if args.is_null() {
        return false;
    } else {
        unsafe { std::slice::from_raw_parts(args, arg_len) }
    };
    let mut rust_args = Vec::with_capacity(arg_slice.len());
    for arg in arg_slice {
        match lc_value_to_rust(arg) {
            Ok(value) => rust_args.push(value),
            Err(_) => return false,
        }
    }

    match engine.0.call(name, &rust_args) {
        Ok(value) => {
            unsafe {
                *out_value = rust_value_to_lc(&value);
            }
            true
        }
        Err(_) => false,
    }
}

#[no_mangle]
pub extern "C" fn lc_script_value_free(value: *mut LcScriptValue) {
    if value.is_null() {
        return;
    }
    unsafe {
        let value = &mut *value;
        if !value.string_value.is_null() {
            drop(CString::from_raw(value.string_value));
            value.string_value = ptr::null_mut();
        }
    }
}

fn lc_value_to_rust(value: &LcScriptValue) -> Result<Value, ()> {
    match value.kind {
        LcScriptValueKind::Nil => Ok(Value::Nil),
        LcScriptValueKind::Int => Ok(Value::Int(value.int_value)),
        LcScriptValueKind::Bool => Ok(Value::Bool(value.bool_value)),
        LcScriptValueKind::String => {
            if value.string_value.is_null() {
                return Err(());
            }
            let c_str = unsafe { CStr::from_ptr(value.string_value) };
            Ok(Value::String(c_str.to_string_lossy().into_owned()))
        }
    }
}

fn rust_value_to_lc(value: &Value) -> LcScriptValue {
    match value {
        Value::Nil => LcScriptValue::default(),
        Value::Int(i) => LcScriptValue {
            kind: LcScriptValueKind::Int,
            int_value: *i,
            ..LcScriptValue::default()
        },
        Value::Bool(b) => LcScriptValue {
            kind: LcScriptValueKind::Bool,
            bool_value: *b,
            ..LcScriptValue::default()
        },
        Value::String(s) => match CString::new(s.as_str()) {
            Ok(c_string) => LcScriptValue {
                kind: LcScriptValueKind::String,
                string_value: c_string.into_raw(),
                ..LcScriptValue::default()
            },
            Err(_) => LcScriptValue::default(),
        },
    }
}
