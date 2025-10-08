use crate::{
    ActionState, CrewRole, CrewSelectionState, EffectState, ObjectId, ObjectSnapshot, Playback,
    Recorder, Recording, SimulationSnapshot, Vector2,
};
use std::collections::HashMap;
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
    pub action_ticks: i32,
    pub effects: *const LcEngineEffectSnapshot,
    pub effect_count: usize,
}

#[repr(C)]
pub struct LcEngineCrewSelectionSnapshot {
    pub owner: i32,
    pub selected: *const u64,
    pub selected_count: usize,
    pub has_cursor: bool,
    pub cursor: u64,
}

#[repr(C)]
pub struct LcEngineCrewRoleAssignment {
    pub object_id: u64,
    pub role: *const c_char,
}

#[repr(C)]
pub struct LcEngineCrewRoleSnapshot {
    pub owner: i32,
    pub assignments: *const LcEngineCrewRoleAssignment,
    pub assignment_count: usize,
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
    object_len: usize,
    global_effects: *const LcEngineEffectSnapshot,
    global_effect_len: usize,
    crew_selection: *const LcEngineCrewSelectionSnapshot,
    crew_selection_len: usize,
    crew_roles: *const LcEngineCrewRoleSnapshot,
    crew_roles_len: usize,
    known_crew_owners: *const i32,
    known_crew_owner_len: usize,
    eliminated_crew_owners: *const i32,
    eliminated_crew_owner_len: usize,
) -> Option<SimulationSnapshot> {
    let objects_slice: &[LcEngineObjectSnapshot] = if object_len == 0 {
        &[]
    } else if objects.is_null() {
        return None;
    } else {
        slice::from_raw_parts(objects, object_len)
    };
    let mut snapshots = Vec::with_capacity(objects_slice.len());
    for entry in objects_slice.iter() {
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
        if entry.action_ticks >= 0 {
            action.ticks = entry.action_ticks as u32;
        } else {
            action.ticks = 0;
        }

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

    let global_effects_slice: &[LcEngineEffectSnapshot] = if global_effect_len == 0 {
        &[]
    } else if global_effects.is_null() {
        return None;
    } else {
        slice::from_raw_parts(global_effects, global_effect_len)
    };
    let mut global_effects_vec = Vec::with_capacity(global_effects_slice.len());
    for effect_entry in global_effects_slice {
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
        global_effects_vec.push(effect);
    }

    let crew_selection_slice: &[LcEngineCrewSelectionSnapshot] = if crew_selection_len == 0 {
        &[]
    } else if crew_selection.is_null() {
        return None;
    } else {
        slice::from_raw_parts(crew_selection, crew_selection_len)
    };
    let mut crew_selection_map = HashMap::with_capacity(crew_selection_slice.len());
    for entry in crew_selection_slice {
        let selected_slice: &[u64] = if entry.selected_count == 0 {
            &[]
        } else if entry.selected.is_null() {
            return None;
        } else {
            slice::from_raw_parts(entry.selected, entry.selected_count)
        };
        let selected = selected_slice.iter().copied().map(ObjectId::new).collect();
        let cursor = if entry.has_cursor {
            Some(ObjectId::new(entry.cursor))
        } else {
            None
        };
        crew_selection_map.insert(entry.owner, CrewSelectionState { selected, cursor });
    }

    let crew_roles_slice: &[LcEngineCrewRoleSnapshot] = if crew_roles_len == 0 {
        &[]
    } else if crew_roles.is_null() {
        return None;
    } else {
        slice::from_raw_parts(crew_roles, crew_roles_len)
    };
    let mut crew_role_map = HashMap::with_capacity(crew_roles_slice.len());
    for entry in crew_roles_slice {
        let assignments_slice: &[LcEngineCrewRoleAssignment] = if entry.assignment_count == 0 {
            &[]
        } else if entry.assignments.is_null() {
            return None;
        } else {
            slice::from_raw_parts(entry.assignments, entry.assignment_count)
        };
        let mut assignments = HashMap::with_capacity(assignments_slice.len());
        for assignment in assignments_slice {
            if assignment.role.is_null() {
                return None;
            }
            let role = match CStr::from_ptr(assignment.role).to_str() {
                Ok(value) => value.to_string(),
                Err(_) => CStr::from_ptr(assignment.role)
                    .to_string_lossy()
                    .into_owned(),
            };
            assignments.insert(ObjectId::new(assignment.object_id), CrewRole::from(role));
        }
        crew_role_map.insert(entry.owner, assignments);
    }

    let known_crew_slice: &[i32] = if known_crew_owner_len == 0 {
        &[]
    } else if known_crew_owners.is_null() {
        return None;
    } else {
        slice::from_raw_parts(known_crew_owners, known_crew_owner_len)
    };
    let known_crew_owners = known_crew_slice.to_vec();

    let eliminated_crew_slice: &[i32] = if eliminated_crew_owner_len == 0 {
        &[]
    } else if eliminated_crew_owners.is_null() {
        return None;
    } else {
        slice::from_raw_parts(eliminated_crew_owners, eliminated_crew_owner_len)
    };
    let eliminated_crew_owners = eliminated_crew_slice.to_vec();

    Some(SimulationSnapshot {
        frame,
        objects: snapshots,
        global_effects: global_effects_vec,
        crew_selection: crew_selection_map,
        crew_roles: crew_role_map,
        known_crew_owners,
        eliminated_crew_owners,
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
    global_effects: *const LcEngineEffectSnapshot,
    global_effect_len: usize,
    crew_selection: *const LcEngineCrewSelectionSnapshot,
    crew_selection_len: usize,
    crew_roles: *const LcEngineCrewRoleSnapshot,
    crew_roles_len: usize,
    known_crew_owners: *const i32,
    known_crew_owner_len: usize,
    eliminated_crew_owners: *const i32,
    eliminated_crew_owner_len: usize,
) {
    if handle.is_null() {
        return;
    }
    let snapshot = unsafe {
        make_snapshot(
            frame,
            objects,
            len,
            global_effects,
            global_effect_len,
            crew_selection,
            crew_selection_len,
            crew_roles,
            crew_roles_len,
            known_crew_owners,
            known_crew_owner_len,
            eliminated_crew_owners,
            eliminated_crew_owner_len,
        )
    };
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
    global_effects: *const LcEngineEffectSnapshot,
    global_effect_len: usize,
    crew_selection: *const LcEngineCrewSelectionSnapshot,
    crew_selection_len: usize,
    crew_roles: *const LcEngineCrewRoleSnapshot,
    crew_roles_len: usize,
    known_crew_owners: *const i32,
    known_crew_owner_len: usize,
    eliminated_crew_owners: *const i32,
    eliminated_crew_owner_len: usize,
    error_out: *mut *mut c_char,
) -> bool {
    if handle.is_null() {
        set_error(error_out, "playback handle is null".to_string());
        return false;
    }
    let snapshot = unsafe {
        make_snapshot(
            frame,
            objects,
            len,
            global_effects,
            global_effect_len,
            crew_selection,
            crew_selection_len,
            crew_roles,
            crew_roles_len,
            known_crew_owners,
            known_crew_owner_len,
            eliminated_crew_owners,
            eliminated_crew_owner_len,
        )
    };
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
            action_ticks: 2,
            effects: &effect_snapshot,
            effect_count: 1,
        };

        let snapshot = unsafe {
            make_snapshot(
                5,
                &object,
                1,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        }
        .expect("snapshot should deserialize");

        assert_eq!(snapshot.objects.len(), 1);
        let recorded = &snapshot.objects[0];
        assert_eq!(recorded.effects.len(), 1);
        assert_eq!(recorded.owner, -1);
        assert!(recorded.crew_member);
        assert_eq!(recorded.action.ticks, 2);

        let effect = &recorded.effects[0];
        assert_eq!(effect.name, "FxFire");
        assert_eq!(effect.priority, 100);
        assert_eq!(effect.interval, 2);
        assert_eq!(effect.timer, 1);
    }

    #[test]
    fn make_snapshot_collects_global_effects() {
        let effect_name = CString::new("FxGlobal").unwrap();

        let effect_snapshot = LcEngineEffectSnapshot {
            name: effect_name.as_ptr(),
            priority: 42,
            interval: 10,
            timer: 3,
        };

        let snapshot = unsafe {
            make_snapshot(
                1,
                ptr::null(),
                0,
                &effect_snapshot,
                1,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        }
        .expect("snapshot should deserialize");

        assert!(snapshot.objects.is_empty());
        assert_eq!(snapshot.global_effects.len(), 1);
        let effect = &snapshot.global_effects[0];
        assert_eq!(effect.name, "FxGlobal");
        assert_eq!(effect.priority, 42);
        assert_eq!(effect.interval, 10);
        assert_eq!(effect.timer, 3);
    }

    #[test]
    fn make_snapshot_collects_crew_state() {
        let selected = [1u64, 2u64];
        let crew_selection = LcEngineCrewSelectionSnapshot {
            owner: 1,
            selected: selected.as_ptr(),
            selected_count: selected.len(),
            has_cursor: true,
            cursor: 2,
        };

        let role_name = CString::new("builder").unwrap();
        let role_assignment = LcEngineCrewRoleAssignment {
            object_id: 1,
            role: role_name.as_ptr(),
        };
        let crew_role = LcEngineCrewRoleSnapshot {
            owner: 1,
            assignments: &role_assignment,
            assignment_count: 1,
        };

        let known = [1i32];
        let eliminated = [2i32];

        let snapshot = unsafe {
            make_snapshot(
                1,
                ptr::null(),
                0,
                ptr::null(),
                0,
                &crew_selection,
                1,
                &crew_role,
                1,
                known.as_ptr(),
                known.len(),
                eliminated.as_ptr(),
                eliminated.len(),
            )
        }
        .expect("snapshot should include crew data");

        assert_eq!(snapshot.crew_selection.len(), 1);
        let selection = snapshot.crew_selection.get(&1).expect("owner present");
        let mut selected_ids: Vec<_> = selection.selected.iter().map(|id| id.as_u64()).collect();
        selected_ids.sort_unstable();
        assert_eq!(selected_ids, vec![1, 2]);
        assert_eq!(selection.cursor.map(|id| id.as_u64()), Some(2));

        let roles = snapshot.crew_roles.get(&1).expect("roles present");
        let role = roles
            .get(&ObjectId::new(1))
            .expect("assignment present")
            .as_str();
        assert_eq!(role, "builder");

        assert_eq!(snapshot.known_crew_owners, vec![1]);
        assert_eq!(snapshot.eliminated_crew_owners, vec![2]);
    }
}
