use crate::{
    ActionState, CommandDirection, CrewRole, CrewSelectionState, Direction, EffectState, Engine,
    EngineState, EnvironmentFrame, FloatVector2, HudPlayerSnapshot, HudSnapshot, ObjectId,
    ObjectSnapshot, ObjectStatus, ObjectVertex, ParticleLayer, ParticleSnapshot, Playback,
    Recorder, Recording, Scenario, SimulationSnapshot, SurfaceSnapshot, Vector2,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
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
pub struct LcEngineParticleSnapshot {
    pub definition_id: *const c_char,
    pub x: f32,
    pub y: f32,
    pub xdir: f32,
    pub ydir: f32,
    pub life: i32,
    pub parameter_a: f32,
    pub parameter_b: i32,
    pub layer: i32,
    pub has_owner: bool,
    pub owner_id: u64,
}

#[repr(C)]
pub struct LcEngineObjectVertexSnapshot {
    pub x: i32,
    pub y: i32,
    pub cnat: u32,
    pub friction: i32,
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
    pub direction: i32,
    pub command_direction: i32,
    pub effects: *const LcEngineEffectSnapshot,
    pub effect_count: usize,
    pub vertices: *const LcEngineObjectVertexSnapshot,
    pub vertex_count: usize,
    pub has_container: bool,
    pub container_id: u64,
    pub contents: *const u64,
    pub contents_len: usize,
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

#[repr(C)]
pub struct LcEngineHudPlayerSnapshot {
    pub owner: i32,
    pub crew: *const u64,
    pub crew_count: usize,
    pub has_focus: bool,
    pub focus_object: u64,
    pub eliminated: bool,
}

#[repr(C)]
pub struct LcEngineSurfaceSnapshot {
    pub width: i32,
    pub height: i32,
    pub hash: u64,
}

pub struct RecorderHandle {
    recorder: Recorder,
}

pub struct PlaybackHandle {
    playback: Playback,
}

pub struct RuntimeHandle {
    engine: Engine,
    scenario_path: Option<PathBuf>,
    seed: u64,
    last_frame: u64,
    control_log: BTreeMap<u64, Vec<String>>,
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

impl RuntimeHandle {
    fn new() -> Self {
        Self {
            engine: Engine::with_seed(0),
            scenario_path: None,
            seed: 0,
            last_frame: 0,
            control_log: BTreeMap::new(),
        }
    }
}

unsafe fn make_snapshot(
    frame: u64,
    objects: *const LcEngineObjectSnapshot,
    object_len: usize,
    global_effects: *const LcEngineEffectSnapshot,
    global_effect_len: usize,
    particles: *const LcEngineParticleSnapshot,
    particle_len: usize,
    crew_selection: *const LcEngineCrewSelectionSnapshot,
    crew_selection_len: usize,
    crew_roles: *const LcEngineCrewRoleSnapshot,
    crew_roles_len: usize,
    known_crew_owners: *const i32,
    known_crew_owner_len: usize,
    eliminated_crew_owners: *const i32,
    eliminated_crew_owner_len: usize,
    hud_players: *const LcEngineHudPlayerSnapshot,
    hud_player_len: usize,
    surfaces: *const LcEngineSurfaceSnapshot,
    surface_len: usize,
    controls: *const *const c_char,
    control_len: usize,
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

        let direction = Direction::from_script_value(entry.direction).unwrap_or_default();
        let command_direction =
            CommandDirection::from_script_value(entry.command_direction).unwrap_or_default();

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

        let vertices_slice: &[LcEngineObjectVertexSnapshot] = if entry.vertex_count == 0 {
            &[]
        } else if entry.vertices.is_null() {
            return None;
        } else {
            slice::from_raw_parts(entry.vertices, entry.vertex_count)
        };
        let mut vertices = Vec::with_capacity(vertices_slice.len());
        for vertex_entry in vertices_slice {
            vertices.push(
                ObjectVertex::new(vertex_entry.x, vertex_entry.y)
                    .with_cnat(vertex_entry.cnat)
                    .with_friction(vertex_entry.friction),
            );
        }

        let container = if entry.has_container {
            Some(ObjectId::new(entry.container_id))
        } else {
            None
        };

        let contents_slice: &[u64] = if entry.contents_len == 0 {
            &[]
        } else if entry.contents.is_null() {
            return None;
        } else {
            slice::from_raw_parts(entry.contents, entry.contents_len)
        };
        let contents = contents_slice.iter().copied().map(ObjectId::new).collect();

        snapshots.push(ObjectSnapshot {
            id: ObjectId::new(entry.id),
            definition_id,
            position: Vector2::new(entry.position_x, entry.position_y),
            velocity: Vector2::new(entry.velocity_x, entry.velocity_y),
            energy: entry.energy,
            action,
            direction,
            command_direction,
            action_procedure: None,
            effects,
            vertices,
            container,
            contents,
            status: ObjectStatus::Normal,
            owner: entry.owner,
            crew_member: entry.crew_member,
        });
    }
    snapshots.sort_by_key(|object| object.id);

    let particle_slice: &[LcEngineParticleSnapshot] = if particle_len == 0 {
        &[]
    } else if particles.is_null() {
        return None;
    } else {
        slice::from_raw_parts(particles, particle_len)
    };
    let mut particle_snapshots = Vec::with_capacity(particle_slice.len());
    for entry in particle_slice {
        if entry.definition_id.is_null() {
            return None;
        }
        let definition_id = match CStr::from_ptr(entry.definition_id).to_str() {
            Ok(value) => value.to_string(),
            Err(_) => CStr::from_ptr(entry.definition_id)
                .to_string_lossy()
                .into_owned(),
        };
        let layer = match ParticleLayer::from_ffi(entry.layer, entry.has_owner, entry.owner_id) {
            Some(layer) => layer,
            None => return None,
        };
        particle_snapshots.push(ParticleSnapshot {
            definition_id,
            position: FloatVector2::new(entry.x, entry.y),
            velocity: FloatVector2::new(entry.xdir, entry.ydir),
            life: entry.life,
            parameter_a: entry.parameter_a,
            parameter_b: entry.parameter_b,
            layer,
        });
    }

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

    let hud_slice: &[LcEngineHudPlayerSnapshot] = if hud_player_len == 0 {
        &[]
    } else if hud_players.is_null() {
        return None;
    } else {
        slice::from_raw_parts(hud_players, hud_player_len)
    };
    let mut hud_players_vec = Vec::with_capacity(hud_slice.len());
    for entry in hud_slice {
        let crew_slice: &[u64] = if entry.crew_count == 0 {
            &[]
        } else if entry.crew.is_null() {
            return None;
        } else {
            slice::from_raw_parts(entry.crew, entry.crew_count)
        };
        let mut crew = Vec::with_capacity(crew_slice.len());
        crew.extend(crew_slice.iter().copied().map(ObjectId::new));
        let focus = entry.has_focus.then(|| ObjectId::new(entry.focus_object));
        hud_players_vec.push(HudPlayerSnapshot {
            owner: entry.owner,
            crew,
            focus,
            eliminated: entry.eliminated,
        });
    }

    let surface_slice: &[LcEngineSurfaceSnapshot] = if surface_len == 0 {
        &[]
    } else if surfaces.is_null() {
        return None;
    } else {
        slice::from_raw_parts(surfaces, surface_len)
    };
    let mut surface_snapshots = Vec::with_capacity(surface_slice.len());
    for entry in surface_slice {
        surface_snapshots.push(SurfaceSnapshot {
            width: entry.width,
            height: entry.height,
            hash: entry.hash,
        });
    }

    let control_slice: &[*const c_char] = if control_len == 0 {
        &[]
    } else if controls.is_null() {
        return None;
    } else {
        slice::from_raw_parts(controls, control_len)
    };
    let mut control_entries = Vec::with_capacity(control_slice.len());
    for &entry in control_slice {
        if entry.is_null() {
            return None;
        }
        let ini = match CStr::from_ptr(entry).to_str() {
            Ok(value) => value.to_string(),
            Err(_) => CStr::from_ptr(entry).to_string_lossy().into_owned(),
        };
        control_entries.push(ini);
    }

    Some(SimulationSnapshot {
        frame,
        physics: None,
        objects: snapshots,
        environment: EnvironmentFrame::default(),
        global_effects: global_effects_vec,
        particles: particle_snapshots,
        crew_selection: crew_selection_map,
        crew_roles: crew_role_map,
        known_crew_owners,
        eliminated_crew_owners,
        landscape: None,
        rng: ChaCha8Rng::seed_from_u64(frame),
        hud: HudSnapshot {
            players: hud_players_vec,
        },
        surfaces: surface_snapshots,
        controls: control_entries,
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

fn runtime_snapshot_mismatch(
    expected: &SimulationSnapshot,
    actual: &SimulationSnapshot,
) -> Option<String> {
    if expected.frame != actual.frame {
        return Some(format!(
            "expected frame {}, got {}",
            expected.frame, actual.frame
        ));
    }

    if expected.objects.len() != actual.objects.len() {
        return Some(format!(
            "object count mismatch (expected {}, got {})",
            expected.objects.len(),
            actual.objects.len()
        ));
    }

    let expected_objects: HashMap<_, _> = expected
        .objects
        .iter()
        .map(|object| (object.id, object))
        .collect();
    let actual_objects: HashMap<_, _> = actual
        .objects
        .iter()
        .map(|object| (object.id, object))
        .collect();

    let mut problems = Vec::new();

    for (&id, expected_object) in &expected_objects {
        match actual_objects.get(&id) {
            Some(actual_object) => {
                if expected_object.position != actual_object.position {
                    problems.push(format!(
                        "object {} position expected {:?}, got {:?}",
                        id, expected_object.position, actual_object.position
                    ));
                }
                if expected_object.velocity != actual_object.velocity {
                    problems.push(format!(
                        "object {} velocity expected {:?}, got {:?}",
                        id, expected_object.velocity, actual_object.velocity
                    ));
                }
                if expected_object.energy != actual_object.energy {
                    problems.push(format!(
                        "object {} energy expected {}, got {}",
                        id, expected_object.energy, actual_object.energy
                    ));
                }
                if expected_object.owner != actual_object.owner {
                    problems.push(format!(
                        "object {} owner expected {}, got {}",
                        id, expected_object.owner, actual_object.owner
                    ));
                }
                if expected_object.crew_member != actual_object.crew_member {
                    problems.push(format!(
                        "object {} crew member expected {}, got {}",
                        id, expected_object.crew_member, actual_object.crew_member
                    ));
                }
                if expected_object.action.name != actual_object.action.name {
                    problems.push(format!(
                        "object {} action expected {}, got {}",
                        id, expected_object.action.name, actual_object.action.name
                    ));
                }
                if expected_object.action.phase != actual_object.action.phase {
                    problems.push(format!(
                        "object {} action phase expected {}, got {}",
                        id, expected_object.action.phase, actual_object.action.phase
                    ));
                }
                if expected_object.command_direction != actual_object.command_direction {
                    problems.push(format!(
                        "object {} command direction expected {:?}, got {:?}",
                        id, expected_object.command_direction, actual_object.command_direction
                    ));
                }
                if expected_object.effects != actual_object.effects {
                    problems.push(format!("object {} effects differed", id));
                }
                if expected_object.vertices != actual_object.vertices {
                    problems.push(format!(
                        "object {} vertices mismatch (expected {:?}, got {:?})",
                        id, expected_object.vertices, actual_object.vertices
                    ));
                }
            }
            None => problems.push(format!("missing object {}", id)),
        }
    }

    for id in actual_objects.keys() {
        if !expected_objects.contains_key(id) {
            problems.push(format!("unexpected object {}", id));
        }
    }

    if expected.global_effects != actual.global_effects {
        problems.push("global effects mismatch".into());
    }

    if expected.particles != actual.particles {
        problems.push(format!(
            "particle state mismatch (expected {} entries, got {})",
            expected.particles.len(),
            actual.particles.len()
        ));
    }

    if expected.crew_selection != actual.crew_selection {
        problems.push(format!(
            "crew selection mismatch (expected {:?}, got {:?})",
            expected.crew_selection, actual.crew_selection
        ));
    }

    if expected.crew_roles != actual.crew_roles {
        problems.push(format!(
            "crew roles mismatch (expected {:?}, got {:?})",
            expected.crew_roles, actual.crew_roles
        ));
    }

    if expected.known_crew_owners != actual.known_crew_owners {
        problems.push(format!(
            "known crew owners mismatch (expected {:?}, got {:?})",
            expected.known_crew_owners, actual.known_crew_owners
        ));
    }

    if expected.eliminated_crew_owners != actual.eliminated_crew_owners {
        problems.push(format!(
            "eliminated crew owners mismatch (expected {:?}, got {:?})",
            expected.eliminated_crew_owners, actual.eliminated_crew_owners
        ));
    }

    if expected.controls != actual.controls {
        problems.push(format!(
            "controls mismatch (expected {:?}, got {:?})",
            expected.controls, actual.controls
        ));
    }

    if expected.hud != actual.hud {
        problems.push(format!(
            "hud mismatch (expected {:?}, got {:?})",
            expected.hud, actual.hud
        ));
    }

    if expected.surfaces != actual.surfaces {
        problems.push(format!(
            "surface hash mismatch (expected {:?}, got {:?})",
            expected.surfaces, actual.surfaces
        ));
    }

    if problems.is_empty() {
        None
    } else {
        Some(problems.join(", "))
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_new() -> *mut RuntimeHandle {
    Box::into_raw(Box::new(RuntimeHandle::new()))
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_free(handle: *mut RuntimeHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

fn load_scenario_into_runtime(
    runtime: &mut RuntimeHandle,
    path: &PathBuf,
    seed: u64,
) -> Result<(), String> {
    let scenario = Scenario::load_from_path(path)
        .map_err(|error| format!("failed to load scenario: {error}"))?;
    runtime.engine = Engine::with_seed(seed);
    scenario
        .apply(&mut runtime.engine)
        .map_err(|error| format!("failed to apply scenario: {error}"))?;
    runtime.seed = seed;
    runtime.last_frame = runtime.engine.frame();
    runtime.scenario_path = Some(path.clone());
    runtime.control_log.clear();
    Ok(())
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_load_scenario(
    handle: *mut RuntimeHandle,
    path: *const c_char,
    seed: u64,
    error_out: *mut *mut c_char,
) -> bool {
    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return false;
    };

    if path.is_null() {
        set_error(error_out, "scenario path is null".into());
        return false;
    }

    let path_cstr = unsafe { CStr::from_ptr(path) };
    let path_str = match path_cstr.to_str() {
        Ok(value) => value.to_string(),
        Err(_) => path_cstr.to_string_lossy().into_owned(),
    };
    let scenario_path = PathBuf::from(path_str);

    match load_scenario_into_runtime(runtime, &scenario_path, seed) {
        Ok(()) => true,
        Err(message) => {
            set_error(error_out, message);
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_record_control_ini(
    handle: *mut RuntimeHandle,
    frame: u64,
    ini: *const c_char,
    error_out: *mut *mut c_char,
) -> bool {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return false;
    };

    if ini.is_null() {
        set_error(error_out, "control data is null".into());
        return false;
    }

    let ini_cstr = unsafe { CStr::from_ptr(ini) };
    let ini_string = match ini_cstr.to_str() {
        Ok(value) => value.to_owned(),
        Err(_) => ini_cstr.to_string_lossy().into_owned(),
    };

    if let Some((&last_frame, _)) = runtime.control_log.iter().next_back() {
        if frame < last_frame {
            set_error(
                error_out,
                format!(
                    "control frame {} out of order (last recorded frame {})",
                    frame, last_frame
                ),
            );
            return false;
        }
    }

    runtime
        .control_log
        .entry(frame)
        .or_insert_with(Vec::new)
        .push(ini_string);

    true
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_reset(
    handle: *mut RuntimeHandle,
    error_out: *mut *mut c_char,
) -> bool {
    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return false;
    };

    let Some(path) = runtime.scenario_path.clone() else {
        runtime.engine = Engine::with_seed(runtime.seed);
        runtime.last_frame = runtime.engine.frame();
        runtime.control_log.clear();
        return true;
    };

    match load_scenario_into_runtime(runtime, &path, runtime.seed) {
        Ok(()) => true,
        Err(message) => {
            set_error(error_out, message);
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_compare_snapshot(
    handle: *mut RuntimeHandle,
    frame: u64,
    objects: *const LcEngineObjectSnapshot,
    object_count: usize,
    global_effects: *const LcEngineEffectSnapshot,
    global_effect_count: usize,
    particles: *const LcEngineParticleSnapshot,
    particle_count: usize,
    crew_selection: *const LcEngineCrewSelectionSnapshot,
    crew_selection_count: usize,
    crew_roles: *const LcEngineCrewRoleSnapshot,
    crew_role_count: usize,
    known_crew_owners: *const i32,
    known_crew_owner_count: usize,
    eliminated_crew_owners: *const i32,
    eliminated_crew_owner_count: usize,
    hud_players: *const LcEngineHudPlayerSnapshot,
    hud_player_count: usize,
    surfaces: *const LcEngineSurfaceSnapshot,
    surface_len: usize,
    controls: *const *const c_char,
    control_count: usize,
    error_out: *mut *mut c_char,
) -> bool {
    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return false;
    };

    let snapshot = match unsafe {
        make_snapshot(
            frame,
            objects,
            object_count,
            global_effects,
            global_effect_count,
            particles,
            particle_count,
            crew_selection,
            crew_selection_count,
            crew_roles,
            crew_role_count,
            known_crew_owners,
            known_crew_owner_count,
            eliminated_crew_owners,
            eliminated_crew_owner_count,
            hud_players,
            hud_player_count,
            surfaces,
            surface_len,
            controls,
            control_count,
        )
    } {
        Some(snapshot) => snapshot,
        None => {
            set_error(error_out, "invalid snapshot input".into());
            return false;
        }
    };

    if frame == 0 && runtime.engine.frame() == 0 {
        let mut expected = runtime.engine.snapshot();
        if let Some(entries) = runtime.control_log.get(&frame) {
            expected.controls = entries.clone();
        } else {
            expected.controls.clear();
        }
        if let Some(detail) = runtime_snapshot_mismatch(&expected, &snapshot) {
            set_error(error_out, detail);
            return false;
        }
        runtime.last_frame = 0;
        return true;
    }

    if frame <= runtime.last_frame {
        set_error(
            error_out,
            format!(
                "frame {} out of order (last validated frame {})",
                frame, runtime.last_frame
            ),
        );
        return false;
    }

    while runtime.engine.frame() < frame {
        if let Err(error) = runtime.engine.tick() {
            set_error(error_out, format!("engine tick failed: {error}"));
            return false;
        }
    }

    if runtime.engine.frame() != frame {
        set_error(
            error_out,
            format!(
                "engine advanced to frame {} while validating frame {}",
                runtime.engine.frame(),
                frame
            ),
        );
        return false;
    }

    let mut expected = runtime.engine.snapshot();
    if let Some(entries) = runtime.control_log.get(&frame) {
        expected.controls = entries.clone();
    } else {
        expected.controls.clear();
    }

    let stale_frames: Vec<u64> = runtime
        .control_log
        .range(..frame)
        .map(|(&key, _)| key)
        .collect();
    for stale in stale_frames {
        runtime.control_log.remove(&stale);
    }

    if let Some(detail) = runtime_snapshot_mismatch(&expected, &snapshot) {
        set_error(error_out, detail);
        return false;
    }

    runtime.last_frame = frame;
    true
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_export_snapshot_json(
    handle: *mut RuntimeHandle,
    error_out: *mut *mut c_char,
) -> *mut c_char {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return ptr::null_mut();
    };

    let mut snapshot = runtime.engine.snapshot();
    let frame = snapshot.frame;
    let control = runtime.control_log.remove(&frame);
    match &control {
        Some(entries) => {
            snapshot.controls = entries.clone();
        }
        None => {
            snapshot.controls.clear();
        }
    }

    #[derive(Serialize)]
    struct RuntimeSnapshotExport {
        snapshot: SimulationSnapshot,
        #[serde(skip_serializing_if = "Option::is_none")]
        control: Option<Vec<String>>,
    }

    let export = RuntimeSnapshotExport { snapshot, control };

    let json = match serde_json::to_string(&export) {
        Ok(json) => json,
        Err(error) => {
            set_error(
                error_out,
                format!("failed to serialize runtime snapshot: {error}"),
            );
            return ptr::null_mut();
        }
    };

    match CString::new(json) {
        Ok(string) => string.into_raw(),
        Err(_) => {
            set_error(
                error_out,
                "runtime snapshot JSON contained interior null byte".into(),
            );
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_export_state_json(
    handle: *mut RuntimeHandle,
    error_out: *mut *mut c_char,
) -> *mut c_char {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return ptr::null_mut();
    };

    let state = runtime.engine.capture_state();
    let json = match serde_json::to_string_pretty(&state) {
        Ok(json) => json,
        Err(error) => {
            set_error(
                error_out,
                format!("failed to serialize runtime state: {error}"),
            );
            return ptr::null_mut();
        }
    };

    match CString::new(json) {
        Ok(string) => string.into_raw(),
        Err(_) => {
            set_error(
                error_out,
                "runtime state JSON contained interior null byte".into(),
            );
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_import_state_json(
    handle: *mut RuntimeHandle,
    json: *const c_char,
    error_out: *mut *mut c_char,
) -> bool {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return false;
    };

    if json.is_null() {
        set_error(error_out, "runtime state JSON is null".into());
        return false;
    }

    let json_cstr = unsafe { CStr::from_ptr(json) };
    let json_string = match json_cstr.to_str() {
        Ok(value) => value.to_owned(),
        Err(_) => json_cstr.to_string_lossy().into_owned(),
    };

    let state: EngineState = match serde_json::from_str(&json_string) {
        Ok(state) => state,
        Err(error) => {
            set_error(error_out, format!("failed to parse runtime state: {error}"));
            return false;
        }
    };

    if let Err(error) = runtime.engine.restore_state(&state) {
        set_error(
            error_out,
            format!("failed to restore runtime state: {error}"),
        );
        return false;
    }

    runtime.last_frame = runtime.engine.frame();
    runtime.control_log.clear();

    true
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
    particles: *const LcEngineParticleSnapshot,
    particle_len: usize,
    crew_selection: *const LcEngineCrewSelectionSnapshot,
    crew_selection_len: usize,
    crew_roles: *const LcEngineCrewRoleSnapshot,
    crew_roles_len: usize,
    known_crew_owners: *const i32,
    known_crew_owner_len: usize,
    eliminated_crew_owners: *const i32,
    eliminated_crew_owner_len: usize,
    hud_players: *const LcEngineHudPlayerSnapshot,
    hud_player_len: usize,
    surfaces: *const LcEngineSurfaceSnapshot,
    surface_len: usize,
    controls: *const *const c_char,
    control_len: usize,
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
            particles,
            particle_len,
            crew_selection,
            crew_selection_len,
            crew_roles,
            crew_roles_len,
            known_crew_owners,
            known_crew_owner_len,
            eliminated_crew_owners,
            eliminated_crew_owner_len,
            hud_players,
            hud_player_len,
            surfaces,
            surface_len,
            controls,
            control_len,
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
    particles: *const LcEngineParticleSnapshot,
    particle_len: usize,
    crew_selection: *const LcEngineCrewSelectionSnapshot,
    crew_selection_len: usize,
    crew_roles: *const LcEngineCrewRoleSnapshot,
    crew_roles_len: usize,
    known_crew_owners: *const i32,
    known_crew_owner_len: usize,
    eliminated_crew_owners: *const i32,
    eliminated_crew_owner_len: usize,
    hud_players: *const LcEngineHudPlayerSnapshot,
    hud_player_len: usize,
    surfaces: *const LcEngineSurfaceSnapshot,
    surface_len: usize,
    controls: *const *const c_char,
    control_len: usize,
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
            particles,
            particle_len,
            crew_selection,
            crew_selection_len,
            crew_roles,
            crew_roles_len,
            known_crew_owners,
            known_crew_owner_len,
            eliminated_crew_owners,
            eliminated_crew_owner_len,
            hud_players,
            hud_player_len,
            surfaces,
            surface_len,
            controls,
            control_len,
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
    use crate::{Definition, SpawnConfig, Vector2};
    use serde_json::Value;
    use std::{ffi::CString, ptr};

    unsafe fn call_make_snapshot(
        frame: u64,
        objects: *const LcEngineObjectSnapshot,
        object_len: usize,
        global_effects: *const LcEngineEffectSnapshot,
        global_effect_len: usize,
        particles: *const LcEngineParticleSnapshot,
        particle_len: usize,
        crew_selection: *const LcEngineCrewSelectionSnapshot,
        crew_selection_len: usize,
        crew_roles: *const LcEngineCrewRoleSnapshot,
        crew_roles_len: usize,
        known_crew_owners: *const i32,
        known_crew_owner_len: usize,
        eliminated_crew_owners: *const i32,
        eliminated_crew_owner_len: usize,
        hud_players: *const LcEngineHudPlayerSnapshot,
        hud_player_len: usize,
        controls: *const *const c_char,
        control_len: usize,
    ) -> SimulationSnapshot {
        make_snapshot(
            frame,
            objects,
            object_len,
            global_effects,
            global_effect_len,
            particles,
            particle_len,
            crew_selection,
            crew_selection_len,
            crew_roles,
            crew_roles_len,
            known_crew_owners,
            known_crew_owner_len,
            eliminated_crew_owners,
            eliminated_crew_owner_len,
            hud_players,
            hud_player_len,
            ptr::null(),
            0,
            controls,
            control_len,
        )
        .expect("snapshot should deserialize")
    }

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
            direction: 1,
            command_direction: 3,
            effects: &effect_snapshot,
            effect_count: 1,
            vertices: ptr::null(),
            vertex_count: 0,
            has_container: false,
            container_id: 0,
            contents: ptr::null(),
            contents_len: 0,
        };

        let snapshot = unsafe {
            call_make_snapshot(
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
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };

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
    fn make_snapshot_collects_vertices() {
        let definition = CString::new("Clonk").unwrap();
        let action = CString::new("Walk").unwrap();
        let vertices = [
            LcEngineObjectVertexSnapshot {
                x: 1,
                y: 2,
                cnat: 3,
                friction: 4,
            },
            LcEngineObjectVertexSnapshot {
                x: -5,
                y: 6,
                cnat: 7,
                friction: -2,
            },
        ];

        let object = LcEngineObjectSnapshot {
            id: 99,
            definition_id: definition.as_ptr(),
            position_x: 0,
            position_y: 0,
            velocity_x: 0,
            velocity_y: 0,
            energy: 0,
            owner: -1,
            crew_member: false,
            action_name: action.as_ptr(),
            action_phase: 0,
            action_ticks: 0,
            direction: 0,
            command_direction: 0,
            effects: ptr::null(),
            effect_count: 0,
            vertices: vertices.as_ptr(),
            vertex_count: vertices.len(),
            has_container: false,
            container_id: 0,
            contents: ptr::null(),
            contents_len: 0,
        };

        let snapshot = unsafe {
            call_make_snapshot(
                1,
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
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };

        assert_eq!(snapshot.objects.len(), 1);
        let recorded = &snapshot.objects[0];
        assert_eq!(recorded.vertices.len(), 2);
        assert_eq!(recorded.vertices[0].x, 1);
        assert_eq!(recorded.vertices[0].y, 2);
        assert_eq!(recorded.vertices[0].cnat, 3);
        assert_eq!(recorded.vertices[0].friction, 4);
        assert_eq!(recorded.vertices[1].x, -5);
        assert_eq!(recorded.vertices[1].y, 6);
        assert_eq!(recorded.vertices[1].cnat, 7);
        assert_eq!(recorded.vertices[1].friction, -2);
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
            call_make_snapshot(
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
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };

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
            call_make_snapshot(
                1,
                ptr::null(),
                0,
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
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };

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

    #[test]
    fn make_snapshot_collects_container_relationships() {
        let container_contents = [2u64];
        let container_snapshot = LcEngineObjectSnapshot {
            id: 1,
            definition_id: ptr::null(),
            position_x: 0,
            position_y: 0,
            velocity_x: 0,
            velocity_y: 0,
            energy: 0,
            owner: -1,
            crew_member: false,
            action_name: ptr::null(),
            action_phase: 0,
            action_ticks: 0,
            direction: 0,
            command_direction: 0,
            effects: ptr::null(),
            effect_count: 0,
            vertices: ptr::null(),
            vertex_count: 0,
            has_container: false,
            container_id: 0,
            contents: container_contents.as_ptr(),
            contents_len: container_contents.len(),
        };

        let child_snapshot = LcEngineObjectSnapshot {
            id: 2,
            definition_id: ptr::null(),
            position_x: 0,
            position_y: 0,
            velocity_x: 0,
            velocity_y: 0,
            energy: 0,
            owner: -1,
            crew_member: false,
            action_name: ptr::null(),
            action_phase: 0,
            action_ticks: 0,
            direction: 0,
            command_direction: 0,
            effects: ptr::null(),
            effect_count: 0,
            vertices: ptr::null(),
            vertex_count: 0,
            has_container: true,
            container_id: 1,
            contents: ptr::null(),
            contents_len: 0,
        };

        let objects = [container_snapshot, child_snapshot];

        let snapshot = unsafe {
            call_make_snapshot(
                1,
                objects.as_ptr(),
                objects.len(),
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
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };

        let container = snapshot
            .objects
            .iter()
            .find(|object| object.id.as_u64() == 1)
            .expect("container present");
        let mut contents: Vec<_> = container.contents.iter().map(|id| id.as_u64()).collect();
        contents.sort_unstable();
        assert_eq!(contents, vec![2]);
        assert!(container.container.is_none());

        let child = snapshot
            .objects
            .iter()
            .find(|object| object.id.as_u64() == 2)
            .expect("child present");
        assert_eq!(child.container.map(|id| id.as_u64()), Some(1));
        assert!(child.contents.is_empty());
    }

    #[test]
    fn make_snapshot_collects_controls() {
        let control_ini = CString::new("[Control]\nType=Player\n").unwrap();
        let controls = [control_ini.as_ptr()];

        let snapshot = unsafe {
            call_make_snapshot(
                3,
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
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                controls.as_ptr(),
                controls.len(),
            )
        };

        assert_eq!(
            snapshot.controls,
            vec!["[Control]\nType=Player\n".to_string()]
        );
    }

    #[test]
    fn runtime_mismatch_reports_controls_difference() {
        let snapshot = unsafe {
            call_make_snapshot(
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
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };

        let mut expected = snapshot.clone();
        expected.controls = vec!["[Control]\nPlayer=1\n".to_string()];

        let detail =
            runtime_snapshot_mismatch(&expected, &snapshot).expect("should report mismatch");
        assert!(
            detail.contains("controls mismatch"),
            "detail did not mention controls: {detail}"
        );
    }

    #[test]
    fn runtime_state_export_and_import_roundtrip() {
        const TEST_SCRIPT: &str = r#"
global func Initialize(state, random)
{
    return {};
}

global func Step(state, frame, random)
{
    return {
        velocity = [state.velocity[0] + 1, state.velocity[1] + 2],
        energy = state.energy - 1,
    };
}
"#;

        let mut runtime = RuntimeHandle::new();
        let definition =
            Definition::from_script("Mover", "Mover", TEST_SCRIPT).expect("definition compiles");
        runtime
            .engine
            .register_definition(definition)
            .expect("register definition");
        runtime
            .engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(5, 10))
                    .with_velocity(Vector2::new(3, -4))
                    .with_energy(80),
            )
            .expect("spawn succeeds");

        runtime.engine.tick().expect("tick succeeds");
        let expected_snapshot = runtime.engine.snapshot();

        let mut export_error: *mut c_char = ptr::null_mut();
        let json_ptr = lc_engine_runtime_export_state_json(&mut runtime, &mut export_error);
        assert!(export_error.is_null());
        assert!(!json_ptr.is_null());

        let original_json = unsafe {
            let json = CStr::from_ptr(json_ptr)
                .to_str()
                .expect("exported JSON is UTF-8")
                .to_owned();
            lc_engine_string_free(json_ptr);
            json
        };

        let mut imported = RuntimeHandle::new();
        let import_definition = Definition::from_script("Mover", "Mover", TEST_SCRIPT)
            .expect("definition compiles for import");
        imported
            .engine
            .register_definition(import_definition)
            .expect("register definition for import");

        let json_cstring = CString::new(original_json.clone()).expect("json CString");
        let mut import_error: *mut c_char = ptr::null_mut();
        let import_ok = lc_engine_runtime_import_state_json(
            &mut imported,
            json_cstring.as_ptr(),
            &mut import_error,
        );
        if !import_error.is_null() {
            lc_engine_string_free(import_error);
        }
        assert!(import_ok, "import should succeed");
        let restored_snapshot = imported.engine.snapshot();
        assert_eq!(restored_snapshot, expected_snapshot);

        let mut roundtrip_error: *mut c_char = ptr::null_mut();
        let roundtrip_ptr =
            lc_engine_runtime_export_state_json(&mut imported, &mut roundtrip_error);
        assert!(roundtrip_error.is_null());
        assert!(!roundtrip_ptr.is_null());

        let roundtrip_json = unsafe {
            let json = CStr::from_ptr(roundtrip_ptr)
                .to_str()
                .expect("roundtrip JSON is UTF-8")
                .to_owned();
            lc_engine_string_free(roundtrip_ptr);
            json
        };

        let original_value: Value =
            serde_json::from_str(&original_json).expect("original JSON parses");
        let roundtrip_value: Value =
            serde_json::from_str(&roundtrip_json).expect("roundtrip JSON parses");
        assert_eq!(roundtrip_value, original_value);
    }
}
