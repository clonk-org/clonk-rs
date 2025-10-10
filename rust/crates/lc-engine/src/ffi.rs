use crate::{
    control::{
        interpret_player_control_command, parse_control_ini, ControlButton, ControlEvent,
        ControlPacket, PlayerControlData,
    },
    ActionState, CommandDirection, CrewCommandTarget, CrewRole, CrewSelectionState, Direction,
    EffectState, Engine, EngineError, EngineState, EnvironmentFrame, FloatVector2,
    HudPlayerSnapshot, HudSnapshot, Landscape, NetworkPacketDirection, NetworkPacketSnapshot,
    ObjectId, ObjectSnapshot, ObjectStatus, ObjectUpdate, ObjectVertex, ParticleLayer,
    ParticleSnapshot, Playback, Recorder, Recording, Scenario, SimulationSnapshot, SurfaceSnapshot,
    Vector2,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
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
    pub category: i32,
    pub crew_member: bool,
    pub alive: bool,
    pub action_name: *const c_char,
    pub action_phase: i32,
    pub action_ticks: i32,
    pub action_data: i32,
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
    pub label: *const c_char,
    pub width: i32,
    pub height: i32,
    pub hash: u64,
}

#[repr(C)]
pub struct LcEngineNetworkPacketSnapshot {
    pub direction: u8,
    pub status: u8,
    pub reserved: u16,
    pub size: u32,
    pub hash: u64,
    pub client_id: i32,
    pub connection_id: u32,
}

#[repr(C)]
pub struct LcEngineRuntimeObjectState {
    pub id: u64,
    pub definition_id: *const c_char,
    pub position_x: i32,
    pub position_y: i32,
    pub velocity_x: i32,
    pub velocity_y: i32,
    pub energy: i32,
    pub owner: i32,
    pub category: i32,
    pub crew_member: bool,
    pub alive: bool,
    pub status: i32,
    pub action_name: *const c_char,
    pub action_phase: i32,
    pub action_ticks: i32,
    pub action_data: i32,
    pub direction: i32,
    pub command_direction: i32,
    pub has_container: bool,
    pub container_id: u64,
    pub contents: *const u64,
    pub contents_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LcEngineRuntimeObjectStateSlice {
    pub frame: u64,
    pub objects: *const LcEngineRuntimeObjectState,
    pub object_count: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LcEngineRuntimeEnvironmentState {
    pub wind: i32,
    pub wind_variation: i32,
    pub wind_period: u32,
    pub temperature: i32,
    pub climate: i32,
    pub temperature_variation: i32,
    pub temperature_period: u32,
    pub temperature_phase: u32,
    pub time_of_day: u16,
    pub time_speed: i16,
    pub precipitation: i32,
    pub has_sky_color: bool,
    pub sky_color_r: u8,
    pub sky_color_g: u8,
    pub sky_color_b: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LcEngineRuntimeLandscapeSlice {
    pub width: u32,
    pub heights: *const i32,
}

impl Default for LcEngineRuntimeLandscapeSlice {
    fn default() -> Self {
        Self {
            width: 0,
            heights: ptr::null(),
        }
    }
}

pub struct LcEngineRuntimeLandscapeArray {
    slice: LcEngineRuntimeLandscapeSlice,
    heights: Vec<i32>,
}

impl LcEngineRuntimeLandscapeArray {
    fn from_landscape(landscape: Option<&Landscape>) -> Self {
        let mut buffer = Self {
            slice: LcEngineRuntimeLandscapeSlice {
                width: 0,
                heights: ptr::null(),
            },
            heights: Vec::new(),
        };

        if let Some(landscape) = landscape {
            buffer.slice.width = landscape.width();
            buffer.heights.extend_from_slice(landscape.surface());
            if !buffer.heights.is_empty() {
                buffer.slice.heights = buffer.heights.as_ptr();
            }
        }

        buffer
    }
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
    control_log_strings: BTreeMap<u64, Vec<String>>,
    control_packets: BTreeMap<u64, Vec<ControlPacket>>,
    player_controls: HashMap<i32, PlayerControlState>,
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

#[derive(Debug, Clone)]
struct PlayerControlState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    last_direction: CommandDirection,
}

impl Default for PlayerControlState {
    fn default() -> Self {
        Self {
            left: false,
            right: false,
            up: false,
            down: false,
            last_direction: CommandDirection::Stop,
        }
    }
}

impl PlayerControlState {
    fn press(&mut self, button: ControlButton) -> Option<CommandDirection> {
        self.set_button(button, true)
    }

    fn release(&mut self, button: ControlButton) -> Option<CommandDirection> {
        self.set_button(button, false)
    }

    fn clear(&mut self) -> Option<CommandDirection> {
        self.left = false;
        self.right = false;
        self.up = false;
        self.down = false;
        self.update_direction(CommandDirection::Stop)
    }

    fn set_button(&mut self, button: ControlButton, state: bool) -> Option<CommandDirection> {
        match button {
            ControlButton::Left => self.left = state,
            ControlButton::Right => self.right = state,
            ControlButton::Up => self.up = state,
            ControlButton::Down => self.down = state,
        }
        let direction = self.compute_direction();
        self.update_direction(direction)
    }

    fn compute_direction(&self) -> CommandDirection {
        let horizontal = match (self.left, self.right) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        let vertical = match (self.up, self.down) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        match (horizontal, vertical) {
            (-1, -1) => CommandDirection::UpLeft,
            (-1, 0) => CommandDirection::Left,
            (-1, 1) => CommandDirection::DownLeft,
            (0, -1) => CommandDirection::Up,
            (0, 0) => CommandDirection::Stop,
            (0, 1) => CommandDirection::Down,
            (1, -1) => CommandDirection::UpRight,
            (1, 0) => CommandDirection::Right,
            (1, 1) => CommandDirection::DownRight,
            _ => CommandDirection::Stop,
        }
    }

    fn update_direction(&mut self, direction: CommandDirection) -> Option<CommandDirection> {
        if direction != self.last_direction {
            self.last_direction = direction;
            Some(direction)
        } else {
            None
        }
    }
}

impl RuntimeHandle {
    fn new() -> Self {
        Self {
            engine: Engine::with_seed(0),
            scenario_path: None,
            seed: 0,
            last_frame: 0,
            control_log_strings: BTreeMap::new(),
            control_packets: BTreeMap::new(),
            player_controls: HashMap::new(),
        }
    }

    fn apply_control_packets_for_frame(&mut self, frame: u64) -> Result<(), String> {
        let packets = match self.control_packets.remove(&frame) {
            Some(packets) => packets,
            None => return Ok(()),
        };
        for packet in packets {
            if let ControlPacket::PlayerControl(data) = packet {
                self.apply_player_control(&data)
                    .map_err(|error| format!("{error} (player {})", data.player))?;
            }
        }
        Ok(())
    }

    fn apply_player_control(&mut self, data: &PlayerControlData) -> Result<(), String> {
        let player = data.player;
        if player < 0 {
            // Ignore observers or invalid players.
            return Ok(());
        }
        let event = match interpret_player_control_command(data.command) {
            Some(event) => event,
            None => return Ok(()),
        };
        let state = self.player_controls.entry(player).or_default();
        let maybe_direction = match event {
            ControlEvent::Press(button) => state.press(button),
            ControlEvent::Release(button) => state.release(button),
            ControlEvent::ClearPressed => state.clear(),
        };
        if let Some(direction) = maybe_direction {
            self.set_player_command_direction(player, direction)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn set_player_command_direction(
        &mut self,
        owner: i32,
        direction: CommandDirection,
    ) -> Result<(), EngineError> {
        self.ensure_cursor(owner)?;
        let update = ObjectUpdate::new().with_command_direction(direction);
        match self
            .engine
            .apply_command(owner, CrewCommandTarget::cursor(), update.clone())
        {
            Ok(()) => Ok(()),
            Err(EngineError::CrewSelection { .. }) => {
                self.engine
                    .apply_command(owner, CrewCommandTarget::selection(), update)
            }
            Err(error) => Err(error),
        }
    }

    fn ensure_cursor(&mut self, owner: i32) -> Result<(), EngineError> {
        if self.engine.crew_cursor(owner).is_some() {
            return Ok(());
        }
        let mut crew = self.engine.crew_members(owner);
        crew.sort_by_key(|id| id.as_u64());
        if let Some(first) = crew.first().copied() {
            self.engine.set_crew_cursor(owner, Some(first))?;
        }
        Ok(())
    }

    fn advance_to_frame(&mut self, frame: u64) -> Result<(), String> {
        let current = self.engine.frame();
        if frame < current {
            return Err(format!(
                "target frame {} precedes current engine frame {}",
                frame, current
            ));
        }

        if frame == 0 && current == 0 {
            self.apply_control_packets_for_frame(0)?;
            return Ok(());
        }

        while self.engine.frame() < frame {
            let next_frame = self.engine.frame().saturating_add(1);
            self.apply_control_packets_for_frame(next_frame)?;
            self.engine
                .tick()
                .map_err(|error| format!("engine tick failed: {error}"))?;
        }

        if self.engine.frame() != frame {
            return Err(format!(
                "engine advanced to frame {} while targeting frame {}",
                self.engine.frame(),
                frame
            ));
        }

        let stale_frames: Vec<u64> = self
            .control_log_strings
            .range(..frame)
            .map(|(&key, _)| key)
            .collect();
        for stale in stale_frames {
            self.control_log_strings.remove(&stale);
            self.control_packets.remove(&stale);
        }

        Ok(())
    }
}

pub struct LcEngineRuntimeObjectStateArray {
    slice: LcEngineRuntimeObjectStateSlice,
    objects: Vec<LcEngineRuntimeObjectState>,
    definition_ids: Vec<CString>,
    action_names: Vec<CString>,
    contents: Vec<Box<[u64]>>,
}

impl LcEngineRuntimeObjectStateArray {
    fn from_snapshot(snapshot: &SimulationSnapshot) -> Result<Self, String> {
        let mut buffer = Self {
            slice: LcEngineRuntimeObjectStateSlice {
                frame: snapshot.frame,
                objects: ptr::null(),
                object_count: 0,
            },
            objects: Vec::with_capacity(snapshot.objects.len()),
            definition_ids: Vec::with_capacity(snapshot.objects.len()),
            action_names: Vec::with_capacity(snapshot.objects.len()),
            contents: Vec::with_capacity(snapshot.objects.len()),
        };

        for object in &snapshot.objects {
            let definition_id = CString::new(object.definition_id.clone()).map_err(|_| {
                format!("definition id for object {} contains null byte", object.id)
            })?;
            let action_name = CString::new(object.action.name.clone())
                .map_err(|_| format!("action name for object {} contains null byte", object.id))?;

            let mut contents_ptr = ptr::null();
            let mut contents_len = 0;
            if !object.contents.is_empty() {
                let boxed = object
                    .contents
                    .iter()
                    .map(|id| id.as_u64())
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                contents_ptr = boxed.as_ptr();
                contents_len = boxed.len();
                buffer.contents.push(boxed);
            }

            buffer.definition_ids.push(definition_id);
            buffer.action_names.push(action_name);

            let action_ticks = i32::try_from(object.action.ticks).unwrap_or_else(|_| i32::MAX);

            let has_container = object.container.is_some();
            let container_id = object.container.map(|id| id.as_u64()).unwrap_or_default();

            buffer.objects.push(LcEngineRuntimeObjectState {
                id: object.id.as_u64(),
                definition_id: buffer.definition_ids.last().unwrap().as_ptr(),
                position_x: object.position.x,
                position_y: object.position.y,
                velocity_x: object.velocity.x,
                velocity_y: object.velocity.y,
                energy: object.energy,
                owner: object.owner,
                category: object.category,
                crew_member: object.crew_member,
                alive: object.alive,
                status: object.status.to_script_value(),
                action_name: buffer.action_names.last().unwrap().as_ptr(),
                action_phase: object.action.phase,
                action_ticks,
                action_data: object.action.data,
                direction: object.direction.to_script_value(),
                command_direction: object.command_direction.to_script_value(),
                has_container,
                container_id,
                contents: contents_ptr,
                contents_len,
            });
        }

        buffer.slice.object_count = buffer.objects.len();
        buffer.slice.objects = if buffer.objects.is_empty() {
            ptr::null()
        } else {
            buffer.objects.as_ptr()
        };

        Ok(buffer)
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
    network_packets: *const LcEngineNetworkPacketSnapshot,
    network_packet_len: usize,
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
        action.data = entry.action_data;

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
            category: entry.category,
            crew_member: entry.crew_member,
            alive: entry.alive,
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
        let label = if entry.label.is_null() {
            String::new()
        } else {
            let cstr = unsafe { CStr::from_ptr(entry.label) };
            match cstr.to_str() {
                Ok(value) => value.to_owned(),
                Err(_) => cstr.to_string_lossy().into_owned(),
            }
        };
        surface_snapshots.push(SurfaceSnapshot {
            label,
            width: entry.width,
            height: entry.height,
            hash: entry.hash,
        });
    }

    let network_slice: &[LcEngineNetworkPacketSnapshot] = if network_packet_len == 0 {
        &[]
    } else if network_packets.is_null() {
        return None;
    } else {
        slice::from_raw_parts(network_packets, network_packet_len)
    };
    let mut network_snapshots = Vec::with_capacity(network_slice.len());
    for entry in network_slice {
        let direction = match entry.direction {
            0 => NetworkPacketDirection::Inbound,
            1 => NetworkPacketDirection::Outbound,
            _ => return None,
        };
        network_snapshots.push(NetworkPacketSnapshot {
            direction,
            status: entry.status,
            size: entry.size,
            hash: entry.hash,
            client_id: entry.client_id,
            connection_id: entry.connection_id,
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
        network_packets: network_snapshots,
        definition_categories: HashMap::new(),
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
                if expected_object.alive != actual_object.alive {
                    problems.push(format!(
                        "object {} alive expected {}, got {}",
                        id, expected_object.alive, actual_object.alive
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

    if expected.network_packets != actual.network_packets {
        problems.push(format!(
            "network packets mismatch (expected {:?}, got {:?})",
            expected.network_packets, actual.network_packets
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
    runtime.control_log_strings.clear();
    runtime.control_packets.clear();
    runtime.player_controls.clear();
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

    if let Some((&last_frame, _)) = runtime.control_log_strings.iter().next_back() {
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

    match parse_control_ini(&ini_string) {
        Ok(packets) => {
            runtime
                .control_packets
                .entry(frame)
                .or_insert_with(Vec::new)
                .extend(packets);
        }
        Err(error) => {
            set_error(
                error_out,
                format!("failed to parse control payload for frame {frame}: {error}"),
            );
            return false;
        }
    }

    runtime
        .control_log_strings
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
        runtime.control_log_strings.clear();
        runtime.control_packets.clear();
        runtime.player_controls.clear();
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
pub extern "C" fn lc_engine_runtime_advance_to_frame(
    handle: *mut RuntimeHandle,
    frame: u64,
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

    if let Err(message) = runtime.advance_to_frame(frame) {
        set_error(
            error_out,
            format!("failed to advance runtime to frame {frame}: {message}"),
        );
        return false;
    }

    runtime.last_frame = frame;
    true
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_step(
    handle: *mut RuntimeHandle,
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

    let current = runtime.engine.frame();
    if current == u64::MAX {
        set_error(
            error_out,
            "engine frame counter reached maximum value".into(),
        );
        return false;
    }
    let target = current + 1;

    if let Err(message) = runtime.advance_to_frame(target) {
        set_error(
            error_out,
            format!("failed to advance runtime to frame {target}: {message}"),
        );
        return false;
    }

    runtime.last_frame = runtime.engine.frame();
    true
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_current_frame(handle: *const RuntimeHandle) -> u64 {
    let Some(runtime) = (unsafe { handle.as_ref() }) else {
        return 0;
    };
    runtime.engine.frame()
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_export_object_states(
    handle: *mut RuntimeHandle,
    error_out: *mut *mut c_char,
) -> *mut LcEngineRuntimeObjectStateArray {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_mut() }) else {
        set_error(error_out, "runtime handle is null".into());
        return ptr::null_mut();
    };

    let snapshot = runtime.engine.snapshot();
    match LcEngineRuntimeObjectStateArray::from_snapshot(&snapshot) {
        Ok(buffer) => Box::into_raw(Box::new(buffer)),
        Err(message) => {
            set_error(error_out, message);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_object_states_slice(
    buffer: *const LcEngineRuntimeObjectStateArray,
) -> LcEngineRuntimeObjectStateSlice {
    if buffer.is_null() {
        return LcEngineRuntimeObjectStateSlice {
            frame: 0,
            objects: ptr::null(),
            object_count: 0,
        };
    }

    unsafe { (*buffer).slice }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_object_states_free(
    buffer: *mut LcEngineRuntimeObjectStateArray,
) {
    if buffer.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(buffer));
    }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_export_environment(
    handle: *const RuntimeHandle,
    out: *mut LcEngineRuntimeEnvironmentState,
    error_out: *mut *mut c_char,
) -> bool {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_ref() }) else {
        set_error(error_out, "runtime handle is null".into());
        return false;
    };

    let Some(out_ref) = (unsafe { out.as_mut() }) else {
        set_error(error_out, "environment output is null".into());
        return false;
    };

    let environment = runtime.engine.environment();
    let (has_sky_color, sky_color_r, sky_color_g, sky_color_b) = environment
        .sky_color
        .map(|color| (true, color.r, color.g, color.b))
        .unwrap_or((false, 0, 0, 0));

    *out_ref = LcEngineRuntimeEnvironmentState {
        wind: environment.wind,
        wind_variation: environment.wind_variation,
        wind_period: environment.wind_period,
        temperature: environment.temperature,
        climate: environment.climate,
        temperature_variation: environment.temperature_variation,
        temperature_period: environment.temperature_period,
        temperature_phase: environment.temperature_phase,
        time_of_day: environment.time_of_day,
        time_speed: environment.time_speed,
        precipitation: environment.precipitation,
        has_sky_color,
        sky_color_r,
        sky_color_g,
        sky_color_b,
    };
    true
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_export_landscape(
    handle: *const RuntimeHandle,
    error_out: *mut *mut c_char,
) -> *mut LcEngineRuntimeLandscapeArray {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_ref() }) else {
        set_error(error_out, "runtime handle is null".into());
        return ptr::null_mut();
    };

    let buffer = LcEngineRuntimeLandscapeArray::from_landscape(runtime.engine.landscape());
    Box::into_raw(Box::new(buffer))
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_landscape_slice(
    buffer: *const LcEngineRuntimeLandscapeArray,
) -> LcEngineRuntimeLandscapeSlice {
    if buffer.is_null() {
        return LcEngineRuntimeLandscapeSlice::default();
    }

    unsafe { (*buffer).slice }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_landscape_free(buffer: *mut LcEngineRuntimeLandscapeArray) {
    if buffer.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(buffer));
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
    network_packets: *const LcEngineNetworkPacketSnapshot,
    network_packet_count: usize,
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
            network_packets,
            network_packet_count,
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
        if let Err(message) = runtime.advance_to_frame(0) {
            set_error(
                error_out,
                format!("failed to prepare runtime for frame 0: {message}"),
            );
            return false;
        }
        let mut expected = runtime.engine.snapshot();
        if let Some(entries) = runtime.control_log_strings.get(&frame) {
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

    if let Err(message) = runtime.advance_to_frame(frame) {
        set_error(
            error_out,
            format!("failed to advance runtime to frame {frame}: {message}"),
        );
        return false;
    }

    let mut expected = runtime.engine.snapshot();
    if let Some(entries) = runtime.control_log_strings.get(&frame) {
        expected.controls = entries.clone();
    } else {
        expected.controls.clear();
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
    let control = runtime.control_log_strings.remove(&frame);
    match &control {
        Some(entries) => {
            snapshot.controls = entries.clone();
        }
        None => {
            snapshot.controls.clear();
        }
    }
    runtime.control_packets.remove(&frame);

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
    runtime.control_log_strings.clear();
    runtime.control_packets.clear();
    runtime.player_controls.clear();

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
    network_packets: *const LcEngineNetworkPacketSnapshot,
    network_packet_len: usize,
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
            network_packets,
            network_packet_len,
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
    network_packets: *const LcEngineNetworkPacketSnapshot,
    network_packet_len: usize,
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
            network_packets,
            network_packet_len,
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
    use crate::{
        control::{COM_RELEASE_OFFSET, COM_RIGHT},
        Definition, EnvironmentSettings, RgbColor, SpawnConfig, Vector2,
    };
    use serde_json::Value;
    use std::{ffi::CString, ptr};

    unsafe fn call_make_snapshot_with_io(
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
        network_packets: *const LcEngineNetworkPacketSnapshot,
        network_packet_len: usize,
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
            surfaces,
            surface_len,
            network_packets,
            network_packet_len,
            controls,
            control_len,
        )
        .expect("snapshot should deserialize")
    }

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
        call_make_snapshot_with_io(
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
            ptr::null(),
            0,
            controls,
            control_len,
        )
    }

    fn runtime_with_simple_object() -> RuntimeHandle {
        const STEP_SCRIPT: &str = r#"
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
            Definition::from_script("Mover", "Mover", STEP_SCRIPT).expect("definition compiles");
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
        runtime
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
            category: crate::DEFAULT_CATEGORY,
            crew_member: true,
            alive: true,
            action_name: action.as_ptr(),
            action_phase: 3,
            action_ticks: 2,
            action_data: 0,
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
            category: crate::DEFAULT_CATEGORY,
            crew_member: false,
            alive: true,
            action_name: action.as_ptr(),
            action_phase: 0,
            action_ticks: 0,
            action_data: 0,
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
            category: crate::DEFAULT_CATEGORY,
            crew_member: false,
            alive: true,
            action_name: ptr::null(),
            action_phase: 0,
            action_ticks: 0,
            action_data: 0,
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
            category: crate::DEFAULT_CATEGORY,
            crew_member: false,
            alive: true,
            action_name: ptr::null(),
            action_phase: 0,
            action_ticks: 0,
            action_data: 0,
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
    fn make_snapshot_collects_hud_players() {
        let crew_members = [5u64, 3u64];
        let hud_entries = [LcEngineHudPlayerSnapshot {
            owner: 2,
            crew: crew_members.as_ptr(),
            crew_count: crew_members.len(),
            has_focus: true,
            focus_object: 3,
            eliminated: true,
        }];

        let snapshot = unsafe {
            call_make_snapshot(
                4,
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
                hud_entries.as_ptr(),
                hud_entries.len(),
                ptr::null(),
                0,
            )
        };

        assert_eq!(snapshot.hud.players.len(), 1);
        let player = &snapshot.hud.players[0];
        assert_eq!(player.owner, 2);
        assert!(player.eliminated);
        let crew: Vec<_> = player.crew.iter().map(|id| id.as_u64()).collect();
        assert_eq!(crew, vec![5, 3]);
        assert_eq!(player.focus.map(|id| id.as_u64()), Some(3));
    }

    #[test]
    fn make_snapshot_collects_network_packets() {
        let packets = [
            LcEngineNetworkPacketSnapshot {
                direction: 0,
                status: 7,
                reserved: 0,
                size: 32,
                hash: 0xDEADBEEFu64,
                client_id: 4,
                connection_id: 17,
            },
            LcEngineNetworkPacketSnapshot {
                direction: 1,
                status: 3,
                reserved: 0,
                size: 12,
                hash: 0xFEEDFACEu64,
                client_id: 2,
                connection_id: 9,
            },
        ];

        let snapshot = unsafe {
            call_make_snapshot_with_io(
                7,
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
                packets.as_ptr(),
                packets.len(),
                ptr::null(),
                0,
            )
        };

        assert_eq!(snapshot.network_packets.len(), 2);
        let inbound = &snapshot.network_packets[0];
        assert_eq!(inbound.direction, NetworkPacketDirection::Inbound);
        assert_eq!(inbound.status, 7);
        assert_eq!(inbound.size, 32);
        assert_eq!(inbound.hash, 0xDEADBEEFu64);
        assert_eq!(inbound.client_id, 4);
        assert_eq!(inbound.connection_id, 17);

        let outbound = &snapshot.network_packets[1];
        assert_eq!(outbound.direction, NetworkPacketDirection::Outbound);
        assert_eq!(outbound.status, 3);
        assert_eq!(outbound.size, 12);
        assert_eq!(outbound.hash, 0xFEEDFACEu64);
        assert_eq!(outbound.client_id, 2);
        assert_eq!(outbound.connection_id, 9);
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
    fn runtime_mismatch_reports_hud_difference() {
        let crew_members = [11u64];
        let hud_entries = [LcEngineHudPlayerSnapshot {
            owner: 5,
            crew: crew_members.as_ptr(),
            crew_count: crew_members.len(),
            has_focus: false,
            focus_object: 0,
            eliminated: false,
        }];

        let snapshot = unsafe {
            call_make_snapshot(
                2,
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
                hud_entries.as_ptr(),
                hud_entries.len(),
                ptr::null(),
                0,
            )
        };

        let mut expected = snapshot.clone();
        assert_eq!(expected.hud.players.len(), 1, "baseline missing hud player");
        expected.hud.players[0].eliminated = true;

        let detail =
            runtime_snapshot_mismatch(&expected, &snapshot).expect("should report mismatch");
        assert!(
            detail.contains("hud mismatch"),
            "detail did not mention hud: {detail}"
        );
    }

    #[test]
    fn runtime_mismatch_reports_network_difference() {
        let packets = [LcEngineNetworkPacketSnapshot {
            direction: 0,
            status: 4,
            reserved: 0,
            size: 48,
            hash: 0xABCD1234u64,
            client_id: 6,
            connection_id: 21,
        }];

        let snapshot = unsafe {
            call_make_snapshot_with_io(
                6,
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
                packets.as_ptr(),
                packets.len(),
                ptr::null(),
                0,
            )
        };

        let mut expected = snapshot.clone();
        assert_eq!(
            expected.network_packets.len(),
            1,
            "baseline missing network packets"
        );
        expected.network_packets[0].hash ^= 1;

        let detail =
            runtime_snapshot_mismatch(&expected, &snapshot).expect("should report mismatch");
        assert!(
            detail.contains("network packets mismatch"),
            "detail did not mention network packets: {detail}"
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

    #[test]
    fn runtime_step_advances_engine_and_exports_snapshot() {
        let mut runtime = runtime_with_simple_object();
        let handle: *mut RuntimeHandle = &mut runtime;

        assert_eq!(runtime.engine.frame(), 0, "engine starts at frame 0");
        assert!(
            lc_engine_runtime_step(handle, ptr::null_mut()),
            "step call should succeed"
        );
        assert_eq!(
            runtime.engine.frame(),
            1,
            "engine should advance to next frame"
        );
        assert_eq!(
            lc_engine_runtime_current_frame(&runtime),
            1,
            "current frame query matches engine"
        );

        let mut export_error: *mut c_char = ptr::null_mut();
        let json_ptr = lc_engine_runtime_export_snapshot_json(handle, &mut export_error);
        assert!(export_error.is_null(), "snapshot export should succeed");
        assert!(!json_ptr.is_null(), "snapshot JSON pointer expected");

        let json_string = unsafe {
            let json = CStr::from_ptr(json_ptr)
                .to_str()
                .expect("snapshot JSON is UTF-8")
                .to_owned();
            lc_engine_string_free(json_ptr);
            json
        };
        let value: Value = serde_json::from_str(&json_string).expect("snapshot JSON parses");
        assert_eq!(
            value["snapshot"]["frame"].as_u64(),
            Some(1),
            "snapshot reports advanced frame"
        );
    }

    #[test]
    fn runtime_export_environment_populates_state() {
        let mut runtime = RuntimeHandle::new();
        let mut environment = EnvironmentSettings::new(35);
        environment.wind_variation = 12;
        environment.wind_period = 180;
        environment.temperature = -15;
        environment.climate = 25;
        environment.temperature_variation = 8;
        environment.temperature_period = 720;
        environment.temperature_phase = 3;
        environment.time_of_day = 1200;
        environment.time_speed = 15;
        environment.precipitation = 40;
        environment.sky_color = Some(RgbColor::new(10, 20, 30));
        runtime.engine.set_environment(environment);

        let runtime_ptr: *const RuntimeHandle = &runtime;
        let mut state = LcEngineRuntimeEnvironmentState::default();
        assert!(
            lc_engine_runtime_export_environment(runtime_ptr, &mut state, ptr::null_mut()),
            "environment export should succeed"
        );
        assert_eq!(state.wind, 35);
        assert_eq!(state.wind_variation, 12);
        assert_eq!(state.wind_period, 180);
        assert_eq!(state.temperature, -15);
        assert_eq!(state.climate, 25);
        assert_eq!(state.temperature_variation, 8);
        assert_eq!(state.temperature_period, 720);
        assert_eq!(state.temperature_phase, 3);
        assert_eq!(state.time_of_day, 1200);
        assert_eq!(state.time_speed, 15);
        assert_eq!(state.precipitation, 40);
        assert!(state.has_sky_color, "sky color should be flagged");
        assert_eq!(state.sky_color_r, 10);
        assert_eq!(state.sky_color_g, 20);
        assert_eq!(state.sky_color_b, 30);
    }

    #[test]
    fn runtime_advance_rejects_rewind() {
        let mut runtime = runtime_with_simple_object();
        let handle: *mut RuntimeHandle = &mut runtime;
        assert!(
            lc_engine_runtime_step(handle, ptr::null_mut()),
            "initial step succeeds"
        );
        assert_eq!(runtime.engine.frame(), 1, "engine advanced to frame 1");

        let mut error_ptr: *mut c_char = ptr::null_mut();
        let ok = lc_engine_runtime_advance_to_frame(handle, 0, &mut error_ptr);
        assert!(!ok, "advancing backwards should fail");
        assert!(
            !error_ptr.is_null(),
            "error message should be populated when rewind rejected"
        );
        let message = unsafe {
            let text = CStr::from_ptr(error_ptr)
                .to_str()
                .expect("error string valid UTF-8")
                .to_owned();
            lc_engine_string_free(error_ptr);
            text
        };
        assert!(
            message.contains("precedes current engine frame"),
            "unexpected error wording: {message}"
        );
    }

    #[test]
    fn runtime_control_application_updates_direction() {
        let mut runtime = RuntimeHandle::new();
        let mut definition = Definition::from_script("Test", "Test", "").expect("script compiles");
        definition.set_crew_member(true);
        runtime
            .engine
            .register_definition(definition)
            .expect("definition registers");

        let crew = runtime
            .engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        runtime.control_packets.insert(
            1,
            vec![ControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                command: i32::from(COM_RIGHT),
                data: 0,
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(1)
            .expect("controls apply");
        let snapshot = runtime
            .engine
            .object_snapshot(crew)
            .expect("object snapshot");
        assert_eq!(snapshot.command_direction, CommandDirection::Right);

        runtime.control_packets.insert(
            2,
            vec![ControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                command: i32::from(COM_RIGHT) + i32::from(COM_RELEASE_OFFSET),
                data: 0,
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(2)
            .expect("release apply");
        let snapshot = runtime
            .engine
            .object_snapshot(crew)
            .expect("object snapshot");
        assert_eq!(snapshot.command_direction, CommandDirection::Stop);
    }
}
