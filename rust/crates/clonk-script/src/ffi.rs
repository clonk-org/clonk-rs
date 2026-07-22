use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::engine::Engine;
use crate::value::{c4_string_bytes, c4_string_from_bytes, Value, ValueMap};

pub struct EngineHandle(Engine);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LcScriptValueKind {
    Nil = 0,
    Int = 1,
    Bool = 2,
    String = 3,
    C4Id = 4,
    Array = 5,
    Proplist = 6,
    Object = 7,
}

#[repr(C)]
pub struct LcScriptValue {
    pub kind: LcScriptValueKind,
    pub int_value: c_int,
    pub bool_value: bool,
    pub string_value: *mut c_char,
    pub array_values: *mut LcScriptValue,
    pub array_len: usize,
    pub proplist_entries: *mut LcScriptMapEntry,
    pub proplist_len: usize,
    pub object_id_value: u64,
}

#[repr(C)]
pub struct LcScriptMapEntry {
    pub key: LcScriptValue,
    pub value: LcScriptValue,
}

impl Default for LcScriptValue {
    fn default() -> Self {
        Self {
            kind: LcScriptValueKind::Nil,
            int_value: 0,
            bool_value: false,
            string_value: ptr::null_mut(),
            array_values: ptr::null_mut(),
            array_len: 0,
            proplist_entries: ptr::null_mut(),
            proplist_len: 0,
            object_id_value: 0,
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
    let source = c4_string_from_bytes(c_str.to_bytes());
    engine.0.load_script_c4_string(&source).is_ok()
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
        free_lc_value_fields(value);
    }
}

fn lc_value_to_rust(value: &LcScriptValue) -> Result<Value, ()> {
    match value.kind {
        LcScriptValueKind::Nil => Ok(Value::Nil),
        LcScriptValueKind::Int => Ok(Value::Int(value.int_value)),
        LcScriptValueKind::Bool => Ok(if value.object_id_value != 0 {
            Value::from_c4_bool_data_raw(value.object_id_value as usize)
        } else {
            Value::Bool(value.bool_value)
        }),
        LcScriptValueKind::String => {
            if value.string_value.is_null() {
                return Err(());
            }
            let c_str = unsafe { CStr::from_ptr(value.string_value) };
            Ok(Value::String(c4_string_from_bytes(c_str.to_bytes()).into()))
        }
        LcScriptValueKind::C4Id => {
            if value.object_id_value != 0 {
                return Ok(Value::C4Id(crate::c4_id_from_raw(
                    value.object_id_value as usize,
                )));
            }
            if value.string_value.is_null() {
                return Err(());
            }
            let c_str = unsafe { CStr::from_ptr(value.string_value) };
            let text = c4_string_from_bytes(c_str.to_bytes());
            Ok(Value::C4Id(crate::c4_id_from_raw(crate::c4_id_parse(
                &text,
            ))))
        }
        LcScriptValueKind::Object => Ok(Value::Object(value.object_id_value)),
        LcScriptValueKind::Array => {
            let values = lc_value_slice(value)?
                .iter()
                .map(lc_value_to_rust)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array(values))
        }
        LcScriptValueKind::Proplist => {
            let entries = lc_map_entry_slice(value)?;
            let mut map = ValueMap::with_capacity(entries.len());
            for entry in entries {
                let key = lc_value_to_rust(&entry.key)?;
                map.insert_key(key, lc_value_to_rust(&entry.value)?);
            }
            Ok(Value::Proplist(map))
        }
    }
}

fn native_c_string_prefix(mut bytes: Vec<u8>) -> Vec<u8> {
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes
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
        Value::RawBool(raw) => LcScriptValue {
            kind: LcScriptValueKind::Bool,
            int_value: *raw as u32 as i32,
            bool_value: *raw != 0,
            object_id_value: *raw as u64,
            ..LcScriptValue::default()
        },
        Value::String(s) => match CString::new(native_c_string_prefix(c4_string_bytes(s))) {
            Ok(c_string) => LcScriptValue {
                kind: LcScriptValueKind::String,
                string_value: c_string.into_raw(),
                ..LcScriptValue::default()
            },
            Err(_) => LcScriptValue::default(),
        },
        Value::C4Id(id) => match CString::new(c4_string_bytes(&crate::c4_id_text(id))) {
            Ok(c_string) => LcScriptValue {
                kind: LcScriptValueKind::C4Id,
                string_value: c_string.into_raw(),
                object_id_value: crate::c4_id_raw(id) as u64,
                ..LcScriptValue::default()
            },
            Err(_) => LcScriptValue::default(),
        },
        Value::Object(id) => LcScriptValue {
            kind: LcScriptValueKind::Object,
            object_id_value: *id,
            ..LcScriptValue::default()
        },
        Value::Array(values) => {
            let (array_values, array_len) =
                boxed_slice_into_raw_parts(values.iter().map(rust_value_to_lc).collect());
            LcScriptValue {
                kind: LcScriptValueKind::Array,
                array_values,
                array_len,
                ..LcScriptValue::default()
            }
        }
        Value::Proplist(entries) => {
            let mut ffi_entries = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                ffi_entries.push(LcScriptMapEntry {
                    key: rust_value_to_lc(key),
                    value: rust_value_to_lc(value),
                });
            }

            let (proplist_entries, proplist_len) = boxed_slice_into_raw_parts(ffi_entries);
            LcScriptValue {
                kind: LcScriptValueKind::Proplist,
                proplist_entries,
                proplist_len,
                ..LcScriptValue::default()
            }
        }
    }
}

fn boxed_slice_into_raw_parts<T>(values: Vec<T>) -> (*mut T, usize) {
    let len = values.len();
    if len == 0 {
        return (ptr::null_mut(), 0);
    }

    let mut values = values.into_boxed_slice();
    let ptr = values.as_mut_ptr();
    std::mem::forget(values);
    (ptr, len)
}

fn lc_value_slice(value: &LcScriptValue) -> Result<&[LcScriptValue], ()> {
    let ptr = value.array_values;
    let len = value.array_len;
    if len == 0 {
        Ok(&[])
    } else if ptr.is_null() {
        Err(())
    } else {
        Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
    }
}

fn lc_map_entry_slice(value: &LcScriptValue) -> Result<&[LcScriptMapEntry], ()> {
    let ptr = value.proplist_entries;
    let len = value.proplist_len;
    if len == 0 {
        Ok(&[])
    } else if ptr.is_null() {
        Err(())
    } else {
        Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
    }
}

unsafe fn free_lc_value_fields(value: &mut LcScriptValue) {
    if !value.string_value.is_null() {
        drop(CString::from_raw(value.string_value));
    }

    if !value.array_values.is_null() {
        let values = std::slice::from_raw_parts_mut(value.array_values, value.array_len);
        for nested in values.iter_mut() {
            free_lc_value_fields(nested);
        }
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            value.array_values,
            value.array_len,
        )));
    }

    if !value.proplist_entries.is_null() {
        let entries = std::slice::from_raw_parts_mut(value.proplist_entries, value.proplist_len);
        for entry in entries.iter_mut() {
            free_lc_map_entry(entry);
        }
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            value.proplist_entries,
            value.proplist_len,
        )));
    }

    *value = LcScriptValue::default();
}

unsafe fn free_lc_map_entry(entry: &mut LcScriptMapEntry) {
    free_lc_value_fields(&mut entry.key);
    free_lc_value_fields(&mut entry.value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_load_preserves_raw_c_source_bytes() {
        let handle = lc_script_engine_new();
        let source = CString::new(b"func Probe() { return \"\xff\"; }".as_slice())
            .expect("source has no interior NUL");
        assert!(lc_script_engine_load(handle, source.as_ptr()));

        let name = CString::new("Probe").expect("name has no NUL");
        let mut out = LcScriptValue::default();
        assert!(lc_script_engine_call(
            handle,
            name.as_ptr(),
            std::ptr::null(),
            0,
            &mut out,
        ));
        assert_eq!(
            lc_value_to_rust(&out),
            Ok(Value::String(c4_string_from_bytes(&[0xff]).into()))
        );

        lc_script_value_free(&mut out);
        lc_script_engine_free(handle);
    }

    #[test]
    fn string_ffi_round_trip_uses_the_native_c_string_prefix() {
        let value = Value::String(c4_string_from_bytes(&[0xff, 0, b'X']).into());
        let mut ffi_value = rust_value_to_lc(&value);

        assert_eq!(ffi_value.kind, LcScriptValueKind::String);
        assert_eq!(
            unsafe { CStr::from_ptr(ffi_value.string_value) }.to_bytes(),
            [0xff]
        );
        assert_eq!(
            lc_value_to_rust(&ffi_value),
            Ok(Value::String(c4_string_from_bytes(&[0xff]).into()))
        );

        lc_script_value_free(&mut ffi_value);
    }

    #[test]
    fn rust_to_lc_preserves_arrays() {
        let value = Value::Array(vec![
            Value::Int(7),
            Value::Bool(true),
            Value::C4Id("ROCK".to_string()),
            Value::C4Id(crate::c4_id_from_raw(12345)),
            Value::Object(42),
            Value::Array(vec![Value::String("nested".to_string().into())]),
            Value::Array(Vec::new()),
        ]);

        let mut ffi_value = rust_value_to_lc(&value);
        assert_ne!(ffi_value.kind, LcScriptValueKind::Nil);
        assert_eq!(lc_value_to_rust(&ffi_value), Ok(value));

        lc_script_value_free(&mut ffi_value);
    }

    #[test]
    fn rust_to_lc_preserves_proplists() {
        let value = Value::Proplist(ValueMap::from([
            ("flag".to_string(), Value::Bool(true)),
            (
                "items".to_string(),
                Value::Array(vec![Value::Int(1), Value::String("two".to_string().into())]),
            ),
            ("id".to_string(), Value::C4Id("ROCK".to_string())),
            ("object".to_string(), Value::Object(42)),
            (
                "nested".to_string(),
                Value::Proplist(ValueMap::from([("answer".to_string(), Value::Int(42))])),
            ),
            ("empty".to_string(), Value::Proplist(ValueMap::new())),
        ]));

        let mut ffi_value = rust_value_to_lc(&value);
        assert_ne!(ffi_value.kind, LcScriptValueKind::Nil);
        let ffi_keys: Vec<_> = lc_map_entry_slice(&ffi_value)
            .expect("FFI map entries")
            .iter()
            .map(|entry| lc_value_to_rust(&entry.key).expect("valid FFI key"))
            .collect();
        assert_eq!(
            ffi_keys,
            ["flag", "items", "id", "object", "nested", "empty"]
                .map(|key| Value::String(key.to_string().into()))
        );
        assert_eq!(lc_value_to_rust(&ffi_value), Ok(value));

        lc_script_value_free(&mut ffi_value);
    }

    #[test]
    fn rust_to_lc_preserves_arbitrary_map_keys() {
        let mut entries = ValueMap::new();
        entries.insert_key(Value::Int(7), Value::String("int".into()));
        entries.insert_key(Value::Bool(true), Value::String("bool".into()));
        entries.insert_key(Value::Object(42), Value::String("object".into()));
        entries.insert_key(
            Value::Array(vec![Value::Int(1), Value::Bool(false)]),
            Value::String("array".into()),
        );
        entries.insert_key(
            Value::Proplist(ValueMap::from([("nested", Value::Int(9))])),
            Value::String("map".into()),
        );
        let value = Value::Proplist(entries);

        let mut ffi_value = rust_value_to_lc(&value);
        assert_eq!(lc_value_to_rust(&ffi_value), Ok(value));
        lc_script_value_free(&mut ffi_value);
    }
}
