use crate::{
    ActionState, EffectState, ObjectId, ObjectSnapshot, Playback, Recorder, Recording,
    SimulationSnapshot, Vector2,
};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::slice;

#[repr(C)]
pub struct LcEngineEffectSnapshot {
    pub name: *const c_char,
    pub priority: i32,
    pub interval: i32,
    pub timer: i32,
}

#[repr(C)]
pub struct LcEngineObjectSnapshot {
    pub id: u64,
    pub definition_id: *const c_char,
    pub position_x: i32,
    pub position_y: i32,
    pub velocity_x: i32,
    pub velocity_y: i32,
    pub energy: i32,
    pub owner: i32,
    pub crew_member: bool,
    pub action_name: *const c_char,
    pub action_phase: i32,
    pub effects: *const LcEngineEffectSnapshot,
    pub effect_count: usize,
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
        let action_name = if entry.action_name.is_null() {
            String::from("Idle")
        } else {
            match CStr::from_ptr(entry.action_name).to_str() {
                Ok(value) => value.to_string(),
                Err(_) => CStr::from_ptr(entry.action_name)
                    .to_string_lossy()
                    .into_owned(),
            }
        };
        let mut action = ActionState::new(action_name);
        action.phase = entry.action_phase;

        let effects_slice: &[LcEngineEffectSnapshot] = if entry.effect_count == 0 {
            &[]
        } else if entry.effects.is_null() {
            return None;
        } else {
            slice::from_raw_parts(entry.effects, entry.effect_count)
        };
        let mut effects = Vec::with_capacity(effects_slice.len());
        for effect_entry in effects_slice {
            let effect_name = if effect_entry.name.is_null() {
                String::new()
            } else {
                match CStr::from_ptr(effect_entry.name).to_str() {
                    Ok(value) => value.to_string(),
                    Err(_) => CStr::from_ptr(effect_entry.name)
                        .to_string_lossy()
                        .into_owned(),
                }
            };
            let effect = EffectState::new(effect_name)
                .with_priority(effect_entry.priority)
                .with_interval(effect_entry.interval)
                .with_timer(effect_entry.timer);
            effects.push(effect);
        }

        snapshots.push(ObjectSnapshot {
            id: ObjectId::new(entry.id),
            definition_id,
            position: Vector2::new(entry.position_x, entry.position_y),
            velocity: Vector2::new(entry.velocity_x, entry.velocity_y),
            energy: entry.energy,
            owner: entry.owner,
            crew_member: entry.crew_member,
            action,
            effects,
        });
    }
    snapshots.sort_by_key(|object| object.id);
    Some(SimulationSnapshot {
        frame,
        objects: snapshots,
        global_effects: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn make_snapshot_collects_effects() {
        let effect_name = CString::new("FxFire").unwrap();
        let definition = CString::new("Clonk").unwrap();
        let action = CString::new("Walk").unwrap();

        let effect_snapshot = LcEngineEffectSnapshot {
            name: effect_name.as_ptr(),
            priority: 100,
            interval: 2,
            timer: 1,
        };

        let object = LcEngineObjectSnapshot {
            id: 42,
            definition_id: definition.as_ptr(),
            position_x: 10,
            position_y: 20,
            velocity_x: -1,
            velocity_y: 2,
            energy: 95,
            owner: -1,
            crew_member: true,
            action_name: action.as_ptr(),
            action_phase: 3,
            effects: &effect_snapshot,
            effect_count: 1,
        };

        let snapshot =
            unsafe { make_snapshot(5, &object, 1) }.expect("snapshot should deserialize");

        assert_eq!(snapshot.objects.len(), 1);
        let recorded = &snapshot.objects[0];
        assert_eq!(recorded.effects.len(), 1);
        assert_eq!(recorded.owner, -1);
        assert!(recorded.crew_member);

        let effect = &recorded.effects[0];
        assert_eq!(effect.name, "FxFire");
        assert_eq!(effect.priority, 100);
        assert_eq!(effect.interval, 2);
        assert_eq!(effect.timer, 1);
    }
}
