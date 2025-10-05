use crate::{ObjectId, ObjectSnapshot, Playback, Recorder, Recording, SimulationSnapshot, Vector2};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::slice;

#[repr(C)]
pub struct LcEngineObjectSnapshot {
    pub id: u64,
    pub definition_id: *const c_char,
    pub position_x: i32,
    pub position_y: i32,
    pub velocity_x: i32,
    pub velocity_y: i32,
    pub energy: i32,
}

pub struct RecorderHandle {
    recorder: Recorder,
}

pub struct PlaybackHandle {
    playback: Playback,
}

impl RecorderHandle {
    fn new() -> Self {
        Self {
            recorder: Recorder::new(),
        }
    }
}

impl PlaybackHandle {
    fn new(playback: Playback) -> Self {
        Self { playback }
    }
}

unsafe fn make_snapshot(
    frame: u64,
    objects: *const LcEngineObjectSnapshot,
    len: usize,
) -> Option<SimulationSnapshot> {
    if objects.is_null() && len > 0 {
        return None;
    }
    let raw = slice::from_raw_parts(objects, len);
    let mut snapshots = Vec::with_capacity(len);
    for entry in raw.iter() {
        let definition_id = if entry.definition_id.is_null() {
            String::new()
        } else {
            match CStr::from_ptr(entry.definition_id).to_str() {
                Ok(value) => value.to_string(),
                Err(_) => CStr::from_ptr(entry.definition_id)
                    .to_string_lossy()
                    .into_owned(),
            }
        };
        snapshots.push(ObjectSnapshot {
            id: ObjectId::new(entry.id),
            definition_id,
            position: Vector2::new(entry.position_x, entry.position_y),
            velocity: Vector2::new(entry.velocity_x, entry.velocity_y),
            energy: entry.energy,
        });
    }
    snapshots.sort_by_key(|object| object.id);
    Some(SimulationSnapshot {
        frame,
        objects: snapshots,
    })
}

fn set_error(error_out: *mut *mut c_char, message: String) {
    if error_out.is_null() {
        return;
    }
    let c_string = CString::new(message)
        .unwrap_or_else(|_| CString::new("invalid utf-8").expect("static string"));
    unsafe {
        *error_out = c_string.into_raw();
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_recorder_new() -> *mut RecorderHandle {
    Box::into_raw(Box::new(RecorderHandle::new()))
}

#[no_mangle]
pub extern "C" fn lc_engine_recorder_clear(handle: *mut RecorderHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let handle = &mut *handle;
        handle.recorder = Recorder::new();
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_recorder_record(
    handle: *mut RecorderHandle,
    frame: u64,
    objects: *const LcEngineObjectSnapshot,
    len: usize,
) {
    if handle.is_null() {
        return;
    }
    let snapshot = unsafe { make_snapshot(frame, objects, len) };
    if let Some(snapshot) = snapshot {
        unsafe {
            let handle = &mut *handle;
            handle.recorder.record(&snapshot);
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_recorder_export_json(handle: *mut RecorderHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let json = unsafe {
        let handle = &mut *handle;
        let mut recording = Recording::new();
        for snapshot in handle.recorder.frames() {
            recording.push(snapshot.clone());
        }
        match recording.to_string() {
            Ok(value) => value,
            Err(_) => return ptr::null_mut(),
        }
    };
    match CString::new(json) {
        Ok(value) => value.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_recorder_free(handle: *mut RecorderHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_playback_from_json(
    data: *const c_char,
    error_out: *mut *mut c_char,
) -> *mut PlaybackHandle {
    if data.is_null() {
        set_error(error_out, "playback data is null".to_string());
        return ptr::null_mut();
    }
    let input = unsafe { CStr::from_ptr(data) };
    let input_str = match input.to_str() {
        Ok(value) => value,
        Err(_) => {
            set_error(error_out, "playback data is not valid UTF-8".to_string());
            return ptr::null_mut();
        }
    };
    let recording = match Recording::from_str(input_str) {
        Ok(recording) => recording,
        Err(error) => {
            set_error(error_out, error.to_string());
            return ptr::null_mut();
        }
    };
    let playback = Playback::from_recording(recording);
    Box::into_raw(Box::new(PlaybackHandle::new(playback)))
}

#[no_mangle]
pub extern "C" fn lc_engine_playback_compare(
    handle: *mut PlaybackHandle,
    frame: u64,
    objects: *const LcEngineObjectSnapshot,
    len: usize,
    error_out: *mut *mut c_char,
) -> bool {
    if handle.is_null() {
        set_error(error_out, "playback handle is null".to_string());
        return false;
    }
    let snapshot = unsafe { make_snapshot(frame, objects, len) };
    let Some(snapshot) = snapshot else {
        set_error(error_out, "invalid object snapshot data".to_string());
        return false;
    };
    let result = unsafe {
        let handle = &mut *handle;
        handle.playback.validate_snapshot(&snapshot)
    };
    match result {
        Ok(_) => true,
        Err(error) => {
            set_error(error_out, error.to_string());
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_playback_finish(
    handle: *mut PlaybackHandle,
    error_out: *mut *mut c_char,
) -> bool {
    if handle.is_null() {
        set_error(error_out, "playback handle is null".to_string());
        return false;
    }
    let result = unsafe {
        let handle = Box::from_raw(handle);
        handle.playback.finish()
    };
    match result {
        Ok(_) => true,
        Err(error) => {
            set_error(error_out, error.to_string());
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_playback_free(handle: *mut PlaybackHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_string_free(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}
