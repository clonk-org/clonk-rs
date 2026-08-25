//! The C ABI the pinned oracle's `USE_RUST_ENGINE_VALIDATION` bridge links
//! against (clonk-org/clonk-rs#585).
//!
//! Ported forward from the Rust tree bundled at the oracle pin
//! (`7d43b47b7d789b533f32d005e64596e0a07019cd:rust/crates/clonk-engine/src/ffi.rs`)
//! so the symbol set and the comparison semantics stay the ones
//! `src/rust/RustEngineBridge.cpp` was written against. `rust/include/lc_engine_ffi.h`
//! at that commit is the authoritative header.
//!
//! Every entry point is an `extern "C"` boundary: pointer validity, lifetime and
//! aliasing are the C++ caller's contract, which is what
//! `clippy::not_unsafe_ptr_arg_deref` is asking about. Marking the 77 exports
//! `unsafe fn` would not change the ABI or the symbols, but it would diverge
//! from the pinned reference for no safety gained on the only caller there is,
//! so the lint is silenced module-wide and the contract documented here instead.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::pathfinder::Path;
use crate::rng::LcgRng;
use crate::{
    control::{
        parse_control_ini, ControlPacket, ControlPlayerInfoEntry, JoinPlayerControlData,
        JoinPlayerSource, LegacyCString, PlayerCommandControlData, PlayerControlData,
    },
    ActionState, CommandDirection, CommandStackSnapshot, ControlClientRegistry, CrewRole,
    CrewSelectionState, Direction, DrawTransform, EffectState, Engine, EngineState,
    EnvironmentFrame, FloatVector2, HudPlayerSnapshot, HudSnapshot, Landscape,
    NetworkPacketDirection, NetworkPacketSnapshot, ObjectBaseGraphics, ObjectId, ObjectSnapshot,
    ObjectStatus, ObjectVertex, ParticleLayer, ParticleSnapshot, Playback, Recorder, Recording,
    Scenario, ScriptControlPolicy, SimulationSnapshot, SurfaceSnapshot, Vector2,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::ptr;
use std::slice;

#[cfg(unix)]
fn legacy_filename_path(filename: &LegacyCString) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(filename.as_bytes().to_vec()))
}

#[cfg(not(unix))]
fn legacy_filename_path(filename: &LegacyCString) -> PathBuf {
    PathBuf::from(filename.to_string_lossy().as_ref())
}

use crate::math::{itofix, C4Fixed, FixedVec2};

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
    pub rotation: i32,
    pub fixed_position_x: i32,
    pub fixed_position_y: i32,
    pub fixed_velocity_x: i32,
    pub fixed_velocity_y: i32,
    pub fixed_rotation: i32,
    pub mobile: bool,
    pub in_liquid: bool,
    pub object_timer: i32,
    pub rotation_velocity: i32,
    pub energy: i32,
    pub construction: i32,
    pub damage: i32,
    pub magic_energy: i32,
    pub magic_capacity: i32,
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
    pub has_base_graphics: bool,
    pub base_definition_id: *const c_char,
    pub base_graphics_name: *const c_char,
    pub base_blit_mode: u32,
    pub has_draw_transform: bool,
    pub draw_scale_x: f32,
    pub draw_scale_y: f32,
    pub draw_offset_x: f32,
    pub draw_offset_y: f32,
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
    pub wealth: i32,
    pub score: i32,
}

#[repr(C)]
pub struct LcEngineSurfaceSnapshot {
    pub label: *const c_char,
    pub width: i32,
    pub height: i32,
    pub hash: u64,
}

#[repr(C)]
pub struct LcEnginePathWaypoint {
    pub x: i32,
    pub y: i32,
    pub has_transfer_target: bool,
    pub transfer_target: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LcEnginePathSlice {
    pub found: bool,
    pub length: i32,
    pub waypoints: *const LcEnginePathWaypoint,
    pub waypoint_count: usize,
}

impl Default for LcEnginePathSlice {
    fn default() -> Self {
        Self {
            found: false,
            length: 0,
            waypoints: ptr::null(),
            waypoint_count: 0,
        }
    }
}

pub struct LcEngineRuntimePathResult {
    slice: LcEnginePathSlice,
    waypoints: Vec<LcEnginePathWaypoint>,
}

impl LcEngineRuntimePathResult {
    fn from_path(path: Option<Path>) -> Self {
        match path {
            Some(path) => {
                let mut waypoints = Vec::with_capacity(path.waypoints.len());
                for waypoint in path.waypoints {
                    let (has_transfer_target, transfer_target) = match waypoint.transfer_target {
                        Some(id) => (true, id.as_u64()),
                        None => (false, 0),
                    };
                    waypoints.push(LcEnginePathWaypoint {
                        x: waypoint.x,
                        y: waypoint.y,
                        has_transfer_target,
                        transfer_target,
                    });
                }
                let slice = LcEnginePathSlice {
                    found: true,
                    length: path.length,
                    waypoints: if waypoints.is_empty() {
                        ptr::null()
                    } else {
                        waypoints.as_ptr()
                    },
                    waypoint_count: waypoints.len(),
                };
                Self { slice, waypoints }
            }
            None => Self {
                slice: LcEnginePathSlice {
                    found: false,
                    length: 0,
                    waypoints: ptr::null(),
                    waypoint_count: 0,
                },
                waypoints: Vec::new(),
            },
        }
    }

    fn slice(&self) -> LcEnginePathSlice {
        self.slice
    }
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
    pub rotation: i32,
    pub fixed_position_x: i32,
    pub fixed_position_y: i32,
    pub fixed_velocity_x: i32,
    pub fixed_velocity_y: i32,
    pub fixed_rotation: i32,
    pub mobile: bool,
    pub in_liquid: bool,
    pub object_timer: i32,
    pub rotation_velocity: i32,
    pub energy: i32,
    pub construction: i32,
    pub damage: i32,
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
    pub has_base_graphics: bool,
    pub base_definition_id: *const c_char,
    pub base_graphics_name: *const c_char,
    pub base_blit_mode: u32,
    pub has_draw_transform: bool,
    pub draw_scale_x: f32,
    pub draw_scale_y: f32,
    pub draw_offset_x: f32,
    pub draw_offset_y: f32,
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
    /// Whether the loaded group is a native replay. A live comparison starts
    /// from the original scenario and must reconstruct the already-installed
    /// local startup info that the C++ bridge cannot pass through this ABI.
    scenario_is_replay: bool,
    /// Native `Game.Clients.getLocalID()`, or `None` for replay lists whose
    /// compiled clients are all remote (`C4Client::CompileFunc`).
    local_client_id: Option<i32>,
    seed: u64,
    last_frame: u64,
    control_log_strings: BTreeMap<u64, Vec<String>>,
    control_packets: BTreeMap<u64, Vec<ControlPacket>>,
    /// C4PlayerInfo registry (Game.PlayerInfos): CID_PlrInfo fills it,
    /// CID_JoinPlr resolves `InfoID` against it (C4Control.cpp:716-722).
    player_infos: HashMap<i32, ControlPlayerInfoEntry>,
    /// Player-info IDs in each C4ClientPlayerInfos list, used to distinguish
    /// AddPlayers append packets from replacement packets.
    player_info_clients: HashMap<i32, Vec<i32>>,
    /// Replay-side projection of `Game.Clients`. Recorded client controls
    /// rebuild this list in packet order even though playback has no live
    /// network transport (`C4Control.cpp:552-687`).
    control_clients: ControlClientRegistry,
    /// One-shot latch for the RNG-ledger divergence report.
    rng_mismatch_reported: bool,
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
        let mut engine = Engine::with_seed(0);
        // Playback consumes the recorded control stream; it is never the
        // control host that synthesizes new Game.Input packets.
        engine.set_control_host(false);
        engine.set_replay_control(true);
        Self {
            engine,
            scenario_path: None,
            scenario_is_replay: true,
            local_client_id: None,
            seed: 0,
            last_frame: 0,
            control_log_strings: BTreeMap::new(),
            control_packets: BTreeMap::new(),
            player_infos: HashMap::new(),
            player_info_clients: HashMap::new(),
            control_clients: ControlClientRegistry::default(),
            rng_mismatch_reported: false,
        }
    }

    fn apply_set_control(&mut self, control: crate::SetControlData) {
        // Every mutating C4ControlSet subtype is host-only except the
        // deliberate DisableDebug escape hatch (C4Control.cpp:128-247).
        if control.by_client != 0 && control.value_type != crate::SET_VALUE_DISABLE_DEBUG {
            return;
        }
        match control.value_type {
            crate::SET_VALUE_CONTROL_RATE => {
                self.engine
                    .set_control_rate(self.engine.control_rate().saturating_add(control.data));
            }
            crate::SET_VALUE_DISABLE_DEBUG => self.engine.disable_debug(),
            crate::SET_VALUE_MAX_PLAYER => {
                if !self.engine.league_game {
                    self.engine.set_max_players(control.data);
                }
            }
            crate::SET_VALUE_TEAM_DISTRIBUTION => {
                self.engine.set_team_distribution(control.data);
            }
            crate::SET_VALUE_TEAM_COLORS => self.engine.set_team_colors(control.data != 0),
            crate::SET_VALUE_FAIR_CREW => {
                if self.engine.fair_crew_forced() {
                    return;
                }
                if control.data < 0 {
                    self.engine.set_use_fair_crew(false);
                    self.engine.set_fair_crew_strength(0);
                } else {
                    self.engine.set_use_fair_crew(true);
                    self.engine.set_fair_crew_strength(control.data);
                }
                self.engine.clear_fair_crew_physicals();
            }
            // C4CVT_None and unknown raw enum values assert only in native
            // debug builds; release playback leaves state untouched.
            _ => {}
        }
    }

    fn remove_replay_player(
        &mut self,
        player_id: i32,
        disconnected: bool,
        frame: u64,
    ) -> Result<(), String> {
        if self.engine.player(player_id).is_none() {
            return Ok(());
        }
        let player = self
            .engine
            .remove_player(player_id)
            .map_err(|error| error.to_string())?;
        self.mark_replay_player_removed(&player, disconnected, frame);
        Ok(())
    }

    fn mark_replay_player_removed(
        &mut self,
        player: &crate::Player,
        disconnected: bool,
        frame: u64,
    ) {
        let info_id = player.player_info_id();
        if info_id != 0 {
            if let Some(info) = self.player_infos.get_mut(&info_id) {
                info.flags |= crate::PLAYER_INFO_FLAG_JOINED | crate::PLAYER_INFO_FLAG_REMOVED;
                if disconnected {
                    info.flags |= crate::PLAYER_INFO_FLAG_DISCONNECTED;
                }
                info.game_part_frame = i32::try_from(frame).unwrap_or(i32::MAX);
            }
        }
    }

    fn remove_replay_players_at_client(
        &mut self,
        client_id: i32,
        frame: u64,
    ) -> Result<(), String> {
        let players = self
            .engine
            .players()
            .filter(|player| player.at_client().get() == client_id)
            .map(crate::Player::id)
            .collect::<Vec<_>>();
        for player in players {
            self.remove_replay_player(player, true, frame)?;
        }
        Ok(())
    }

    fn mark_replay_player_infos_voted_out(&mut self, client_id: Option<i32>) {
        if self.engine.is_game_over() {
            return;
        }
        match client_id {
            Some(client_id) => {
                let ids = self
                    .player_info_clients
                    .get(&client_id)
                    .cloned()
                    .unwrap_or_default();
                for id in ids {
                    if let Some(info) = self.player_infos.get_mut(&id) {
                        if info.flags & crate::PLAYER_INFO_FLAG_REMOVED == 0 {
                            info.flags |= crate::PLAYER_INFO_FLAG_VOTED_OUT;
                        }
                    }
                }
            }
            None => {
                for info in self.player_infos.values_mut() {
                    if info.flags & crate::PLAYER_INFO_FLAG_REMOVED == 0 {
                        info.flags |= crate::PLAYER_INFO_FLAG_VOTED_OUT;
                    }
                }
            }
        }
    }

    fn prune_unjoined_replay_player_infos(&mut self, client_id: i32) {
        let Some(ids) = self.player_info_clients.get(&client_id).cloned() else {
            // C4Network2Players::OnClientPart returns before team recheck when
            // the departing client has no C4ClientPlayerInfos packet.
            return;
        };
        for id in &ids {
            let never_joined = self
                .player_infos
                .get(id)
                .is_some_and(|info| info.flags & crate::PLAYER_INFO_FLAG_JOINED == 0);
            if never_joined {
                self.player_infos.remove(id);
            }
        }
        let retained = ids
            .into_iter()
            .filter(|id| self.player_infos.contains_key(id))
            .collect::<Vec<_>>();
        if retained.is_empty() {
            self.player_info_clients.remove(&client_id);
        } else {
            self.player_info_clients.insert(client_id, retained);
        }

        let mut memberships = self
            .player_infos
            .values()
            .filter(|entry| entry.id > 0 && entry.flags & crate::PLAYER_INFO_FLAG_REMOVED == 0)
            .map(|entry| (entry.id, entry.team))
            .collect::<Vec<_>>();
        memberships.sort_unstable_by_key(|(id, _)| *id);
        self.engine
            .recheck_team_player_info_memberships(&memberships);
    }

    fn apply_replay_player_info(&mut self, info: crate::PlayerInfoControlData) {
        // C4ControlPlayerInfo::Execute adds or replaces one synchronized
        // C4ClientPlayerInfos packet in Game.PlayerInfos.
        let add = info.flags & crate::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS != 0;
        if !add {
            if let Some(previous) = self.player_info_clients.insert(info.client_id, Vec::new()) {
                for id in previous {
                    self.player_infos.remove(&id);
                }
            }
        }
        let client_entries = self.player_info_clients.entry(info.client_id).or_default();
        for entry in info.players {
            let id = entry.id;
            if add && self.player_infos.contains_key(&id) {
                continue;
            }
            self.player_infos.insert(id, entry);
            client_entries.push(id);
        }
        let mut memberships = self
            .player_infos
            .values()
            .filter(|entry| entry.id > 0 && entry.flags & crate::PLAYER_INFO_FLAG_REMOVED == 0)
            .map(|entry| (entry.id, entry.team))
            .collect::<Vec<_>>();
        memberships.sort_unstable_by_key(|(id, _)| *id);
        self.engine
            .recheck_team_player_info_memberships(&memberships);
        self.sync_engine_player_info_parameters();
    }

    fn replace_replay_player_infos(&mut self, infos: Vec<crate::PlayerInfoControlData>) {
        self.player_infos.clear();
        self.player_info_clients.clear();
        for info in infos {
            self.apply_replay_player_info(info);
        }
        // The empty-list case must clear stale script-visible league data as
        // well; a non-empty list is already synchronized after its last row.
        if self.player_infos.is_empty() {
            self.sync_engine_player_info_parameters();
        }
    }

    fn sync_engine_player_info_parameters(&mut self) {
        let progress = self
            .player_infos
            .values()
            .map(|info| {
                (
                    info.id,
                    (!info.league_progress_data_is_null)
                        .then(|| info.league_progress_data.as_bytes().to_vec()),
                )
            })
            .collect::<Vec<_>>();
        let scores = self
            .player_infos
            .values()
            .map(|info| (info.id, info.league_score))
            .collect::<Vec<_>>();
        self.engine
            .replace_player_info_league_progress_data(progress);
        self.engine.replace_player_info_league_scores(scores);
    }

    fn apply_control_packets_for_frame(&mut self, frame: u64) -> Result<(), String> {
        let packets = match self.control_packets.remove(&frame) {
            Some(packets) => packets,
            None => return Ok(()),
        };
        tracing::debug!(frame, count = packets.len(), "applying control packets");
        for packet in packets {
            match packet {
                ControlPacket::Set(data) => self.apply_set_control(data),
                ControlPacket::ClientJoin(data) => {
                    self.control_clients.apply_join(&data);
                }
                ControlPacket::ClientUpdate(data) => {
                    let enters_observer = data.by_client == 0
                        && data.update_type == crate::CLIENT_UPDATE_SET_OBSERVER
                        && self
                            .control_clients
                            .state(data.client_id)
                            .is_some_and(|client| !client.observer);
                    self.control_clients.apply_update(&data);
                    if enters_observer {
                        self.remove_replay_players_at_client(data.client_id, frame)?;
                    }
                }
                ControlPacket::ClientRemove(data) => {
                    if data.by_client == 0 && data.client_id != 0 {
                        let known_client = self.control_clients.apply_remove(&data);
                        // Native replay removes players even when its client
                        // list did not contain the recorded client.
                        self.remove_replay_players_at_client(data.client_id, frame)?;
                        if known_client {
                            // Network.Players.OnClientPart removes only infos
                            // which never joined; joined evaluation history is
                            // retained after the disconnected player removal.
                            self.prune_unjoined_replay_player_infos(data.client_id);
                        }
                    }
                }
                ControlPacket::Vote(_) => {
                    // C4ControlVote stores ballots only while Game.Network
                    // is enabled. Native replay runs CM_Replay without a
                    // network session and therefore has no simulation-side
                    // vote mutation; the recorded VoteEnd remains decisive.
                }
                ControlPacket::VoteEnd(data) => {
                    if data.by_client != 0 || !data.approve {
                        continue;
                    }
                    match data.vote_type {
                        crate::VOTE_TYPE_CANCEL => {
                            self.mark_replay_player_infos_voted_out(None);
                            let players = self
                                .engine
                                .abort_players_without_callbacks(self.local_client_id.unwrap_or(-1))
                                .map_err(|error| error.to_string())?;
                            for player in players {
                                self.mark_replay_player_removed(&player, true, frame);
                            }
                            // C++ proceeds into Application.QuitGame after
                            // removing its local/remote players. The headless
                            // comparator has no application process to quit.
                        }
                        crate::VOTE_TYPE_KICK => {
                            self.mark_replay_player_infos_voted_out(Some(data.data));
                            // A replay is never the control host, so it does
                            // not synthesize CtrlRemove; the recorded client
                            // removal packet performs that later in order.
                            if data.data == self.local_client_id.unwrap_or(-1) {
                                let action = if data.data == data.by_client {
                                    "leave the game".to_string()
                                } else {
                                    let target = self
                                        .control_clients
                                        .state(data.data)
                                        .map(|client| {
                                            clonk_script::c4_string_from_bytes(
                                                client.name.as_bytes(),
                                            )
                                        })
                                        .filter(|name| !name.is_empty())
                                        .unwrap_or_else(|| "???".to_string());
                                    format!("kick client {target}")
                                };
                                let message = format!(
                                    "You have been removed by vote. (It was decided to {action}.)"
                                );
                                self.engine.evaluate_network_round_results(
                                    crate::RoundResultsNetworkResult::NetworkError,
                                    Some(message.into_bytes()),
                                );
                                self.engine
                                    .request_game_over_from_control()
                                    .map_err(|error| error.to_string())?;
                            }
                        }
                        crate::VOTE_TYPE_PAUSE | crate::VOTE_TYPE_NONE => {}
                        _ => {}
                    }
                }
                ControlPacket::PlayerControl(data) => {
                    self.apply_player_control(&data)
                        .map_err(|error| format!("{error} (player {})", data.player))?;
                }
                ControlPacket::PlayerCommand(data) => {
                    self.apply_player_command(&data)
                        .map_err(|error| format!("{error} (player {})", data.player))?;
                }
                ControlPacket::PlayerSelect(data) => {
                    self.engine
                        .execute_player_select(&data)
                        .map_err(|error| format!("{error} (player {})", data.player))?;
                }
                ControlPacket::EmMoveObject(data) => {
                    self.engine
                        .execute_em_move_object_control(&data, ScriptControlPolicy::replay(false))
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::EmDrawTool(data) => {
                    self.engine.execute_em_draw_tool_control(&data);
                }
                ControlPacket::EmDropDef(data) => {
                    self.engine
                        .execute_em_drop_def_control(&data)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::Script(data) => {
                    self.engine
                        .execute_script_control(&data, ScriptControlPolicy::replay(false))
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::Message(_) => {
                    // C4ControlMessage is non-synchronized presentation state;
                    // replay simulation deliberately has no world-side effect.
                }
                ControlPacket::MessageBoardAnswer(data) => {
                    self.engine
                        .execute_message_board_answer_control(&data)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::CustomCommand(data) => {
                    self.engine
                        .execute_custom_command_control(&data, true)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::ActivateGameGoalMenu(data) => {
                    self.engine
                        .execute_activate_game_goal_menu_control(&data)
                        .map_err(|error| error.to_string())?;
                    // Replay evaluates goals but never opens presentation UI.
                    self.engine.take_game_goal_menu_requests();
                }
                ControlPacket::ToggleHostility(data) => {
                    self.engine
                        .execute_toggle_hostility_control(&data)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::ActivateGameGoalRule(data) => {
                    self.engine
                        .execute_activate_game_goal_rule_control(&data)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::SetPlayerTeam(data) => {
                    self.engine
                        .execute_set_player_team_control(&data)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::EliminatePlayer(data) => {
                    self.engine
                        .execute_eliminate_player_control(&data)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::Synchronize(data) => {
                    // C4Control::Execute preserves packet order. Execute the
                    // re-fix inline so later packets in this same recorded
                    // frame observe the fresh RNG ledger.
                    self.engine
                        .execute_synchronize_control(data.save_player_files, data.sync_clearance)
                        .map_err(|error| error.to_string())?;
                }
                ControlPacket::SyncCheck(_) => {
                    // The FFI comparator checks the live snapshot and RNG
                    // ledger after each frame; the recorded digest itself
                    // has no simulation side effects.
                }
                ControlPacket::PlayerInfo(info) => {
                    self.apply_replay_player_info(info);
                }
                ControlPacket::JoinPlayer(join) => {
                    self.handle_join_player(&join)?;
                }
                ControlPacket::RemovePlayer(remove) => {
                    if remove.by_client != 0 {
                        continue;
                    }
                    self.remove_replay_player(remove.player, remove.disconnected, frame)?;
                }
                ControlPacket::InitScenarioPlayer(data) => {
                    let allowed = data.player == -1
                        || self
                            .engine
                            .player(data.player)
                            .is_some_and(|player| player.at_client().get() == data.by_client);
                    if allowed {
                        self.engine
                            .initialize_scenario_player(data.player, data.team)
                            .map_err(|error| error.to_string())?;
                    }
                }
                ControlPacket::SurrenderPlayer(data) => {
                    self.engine.execute_surrender_player_control(data);
                }
                ControlPacket::DebugRecord(_) => {
                    // C4ControlDebugRec::Execute is intentionally empty.
                }
                ControlPacket::Unknown { id, name, .. } => {
                    let name = name.unwrap_or_else(|| "unnamed".to_string());
                    return Err(format!(
                        "unsupported replay control packet 0x{:02x} ({}, {name}) at frame {frame}",
                        id.0, id.0
                    ));
                }
            }
        }
        Ok(())
    }

    /// `C4ControlJoinPlayer::Execute` (C4Control.cpp:710-775), local
    /// branch: resolve the info, load the player file (the local filename
    /// first, the embedded PlrData bytes otherwise) and run the join.
    fn handle_join_player(&mut self, join: &JoinPlayerControlData) -> Result<(), String> {
        // Native execution gates on the synchronized client before looking
        // up PlayerInfos. In particular, an initial PlayerInfos.txt row does
        // not synthesize a missing Parameters.txt client.
        let Some(client) = self.control_clients.state(join.at_client) else {
            tracing::warn!(
                info_id = join.info_id,
                at_client = join.at_client,
                "ignoring replay join for missing controlling client"
            );
            return Ok(());
        };
        let client_name = if !self.scenario_is_replay && client.name.is_empty() {
            "Local".to_string()
        } else {
            clonk_script::c4_string_from_bytes(client.name.as_bytes())
        };

        // Local games install INITIAL infos directly before control starts;
        // those rows do not traverse this ABI. Preserve that one live-shadow
        // reconstruction. A native replay, however, must already have the
        // referenced info and treats a ghost join as a packet-local no-op.
        let info = self.player_infos.get(&join.info_id).cloned();
        if self.scenario_is_replay && info.is_none() {
            tracing::warn!(
                info_id = join.info_id,
                "ignoring replay join for missing player info"
            );
            return Ok(());
        }

        let local_control = self.local_client_id == Some(join.by_client);

        let file = match &join.source {
            JoinPlayerSource::Resource(core) if self.scenario_is_replay => self
                .scenario_path
                .as_ref()
                .and_then(|scenario| {
                    let resource_path = legacy_filename_path(&core.filename);
                    let basename = resource_path.file_name()?;
                    let mut recorded_name = std::ffi::OsString::from(format!("{}-", core.id));
                    recorded_name.push(basename);
                    Some(scenario.join(recorded_name))
                })
                .and_then(|path| match crate::player_file::PlayerFile::load_from_path(&path) {
                    Ok(file) => Some(file),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "recorded player resource failed to parse");
                        None
                    }
                }),
            JoinPlayerSource::Resource(_) => None,
            JoinPlayerSource::Embedded(player_data) => {
                let path = legacy_filename_path(&join.filename);
                if local_control && !join.filename.is_empty() {
                    match crate::player_file::PlayerFile::load_from_path(&path) {
                        Ok(file) => Some(file),
                        Err(error) => {
                            tracing::warn!(path = %path.display(), %error, "local player file failed to parse");
                            None
                        }
                    }
                } else if !player_data.is_empty() {
                    clonk_resources::Group::from_memory(path, player_data.clone())
                        .and_then(|group| {
                            crate::player_file::PlayerFile::load(&group).map_err(|error| {
                                clonk_resources::GroupError::InvalidGroup(error.to_string())
                            })
                        })
                        .map_err(|error| {
                            tracing::warn!(%error, "embedded PlrData failed to parse");
                            error
                        })
                        .ok()
                } else {
                    None
                }
            }
        };
        let fileless_script = info.as_ref().is_some_and(|info| info.is_script_player())
            && matches!(&join.source, JoinPlayerSource::Embedded(data) if data.is_empty())
            && join.filename.is_empty();
        if file.is_none() && !fileless_script {
            tracing::warn!(
                info_id = join.info_id,
                at_client = join.at_client,
                "ignoring replay join whose player data is unavailable"
            );
            return Ok(());
        }
        let retained_player_info_core = file
            .as_ref()
            .map(crate::player_file::PlayerFile::exact_info_core)
            .unwrap_or_default();
        // C4Game freezes Parameters.StartupPlayerCount before control/replay
        // initialization. Replay packages seed that value during scenario
        // loading; synthetic/non-replay runners fall back once to the exact
        // nonremoved registry visible at their first join.
        let observed_startup_player_count = i32::try_from(
            self.player_infos
                .values()
                .filter(|entry| entry.flags & crate::PLAYER_INFO_FLAG_REMOVED == 0)
                .count(),
        )
        .unwrap_or(i32::MAX);
        let startup_player_count = self
            .engine
            .freeze_startup_player_count(observed_startup_player_count);
        let config = if let Some(info) = info.as_ref() {
            match crate::prepare_join_player_config(crate::JoinPlayerPreparation {
                join,
                info,
                player_file: file.as_ref(),
                startup_player_count,
            }) {
                Ok(config) => config,
                Err(error) => {
                    tracing::warn!(info_id = join.info_id, %error, "player join preparation failed");
                    return Ok(());
                }
            }
        } else {
            let (
                name,
                score,
                rounds,
                rounds_won,
                rounds_lost,
                total_playing_time,
                color_dw,
                pref_color,
                pref_position,
                control_style,
                auto_context_menu,
                crew,
            ) = file
                .map(|file| {
                    (
                        file.name,
                        file.score,
                        file.rounds,
                        file.rounds_won,
                        file.rounds_lost,
                        file.total_playing_time,
                        file.pref_color_dw & 0xffffff,
                        file.pref_color,
                        file.pref_position,
                        file.pref_control_style,
                        file.pref_auto_context_menu,
                        file.crew,
                    )
                })
                .unwrap_or_else(|| {
                    let control_style = false;
                    (
                        "Neuling".to_string(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        0xff,
                        0,
                        0,
                        control_style,
                        control_style,
                        Vec::new(),
                    )
                });
            crate::JoinPlayerConfig {
                name,
                player_info_id: 0,
                score,
                rounds,
                rounds_won,
                rounds_lost,
                total_playing_time,
                team: None,
                color_dw,
                pref_color,
                pref_position,
                crew,
                startup_player_count,
                control_style,
                auto_context_menu,
            }
        };
        let outcome = match self.engine.join_player_with_profile_core(
            config,
            crate::PlayerAtClient::new(join.at_client),
            client_name,
            info.as_ref(),
            crate::PlayerRuntimeControl::NONE,
            retained_player_info_core,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(info_id = join.info_id, %error, "player join failed");
                return Ok(());
            }
        };
        if let Some(info) = self.player_infos.get_mut(&join.info_id) {
            // C4PlayerInfo::SetJoined only ORs Joined; a malformed/reused
            // Removed history entry keeps its other synchronized flags.
            info.flags |= crate::PLAYER_INFO_FLAG_JOINED;
            info.game_number = outcome.number();
            info.game_join_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        }
        if let Some(joined) = outcome.initialized() {
            tracing::info!(
                number = joined.number,
                start_x = joined.start_x,
                start_y = joined.start_y,
                "player joined via control"
            );
        } else {
            tracing::info!(
                number = outcome.number(),
                "player awaiting team selection via control"
            );
        }
        // Creation-order forensics for the numbering-skew epic (visible
        // with LC_RUST_ENGINE_LOG): dump the join-cascade id -> definition
        // table so live runs can be diffed against the headless order.
        for (id, definition) in self.engine.spawn_dump_from(1419) {
            tracing::info!(id, definition, "SPAWNDUMP");
        }
        Ok(())
    }

    fn apply_player_control(&mut self, data: &PlayerControlData) -> Result<(), String> {
        // C4ControlPlayerControl::Execute counts non-release input first,
        // then forwards the packet fields to C4Player::InCom
        // (C4Control.cpp:386-395).
        self.engine
            .execute_player_control(data.player, data.command, data.data)
            .map_err(|error| error.to_string())
    }

    fn apply_player_command(&mut self, data: &PlayerCommandControlData) -> Result<(), String> {
        self.engine
            .execute_player_command(
                data.player,
                data.command,
                data.x,
                data.y,
                data.target,
                data.target2,
                data.data,
                data.add_mode,
            )
            .map_err(|error| error.to_string())
    }

    fn find_path(
        &self,
        from: Vector2,
        to: Vector2,
        transfer_zones_enabled: bool,
        level: i32,
    ) -> Option<Path> {
        self.engine
            .find_path(from, to, level, transfer_zones_enabled)
    }

    fn advance_to_frame(&mut self, frame: u64) -> Result<(), String> {
        let current = self.engine.frame();
        tracing::debug!(
            frame,
            current,
            pending = self.control_packets.len(),
            "advance_to_frame"
        );
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
            // Control recorded under FrameCounter N executes at the START
            // of frame N, before that frame's object tick
            // (C4Game::Execute: Control.Execute precedes ExecObjects,
            // C4Game.cpp:776-854) — the tick below moves N to N+1.
            let executing = self.engine.frame();
            self.apply_control_packets_for_frame(executing)?;
            self.engine
                .tick_without_snapshot()
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
    base_definition_ids: Vec<Option<CString>>,
    base_graphics_names: Vec<Option<CString>>,
}

fn snapshot_fixed_position(object: &ObjectSnapshot) -> FixedVec2 {
    object
        .fixed_position
        .unwrap_or_else(|| FixedVec2::from_ints(object.position.x, object.position.y))
}

fn snapshot_fixed_velocity(object: &ObjectSnapshot) -> FixedVec2 {
    object
        .fixed_velocity
        .unwrap_or_else(|| FixedVec2::from_ints(object.velocity.x, object.velocity.y))
}

fn optional_fixed_vec(raw_x: i32, raw_y: i32, pixels: Vector2) -> Option<FixedVec2> {
    let fixed = FixedVec2::new(C4Fixed::from_raw(raw_x), C4Fixed::from_raw(raw_y));
    if fixed == FixedVec2::from_ints(pixels.x, pixels.y) {
        None
    } else {
        Some(fixed)
    }
}

/// Neutralises effect state the bridge ABI cannot carry.
/// `LcEngineEffectSnapshot` transports name/priority/interval/timer only, so
/// C++ reports everything else as a default; comparing those raw invents a
/// divergence. Shared so the per-object and global comparisons agree on what
/// is actually comparable.
fn comparable_effects(effects: &[EffectState]) -> Vec<EffectState> {
    effects
        .iter()
        .cloned()
        .map(|mut effect| {
            effect.number = 0;
            effect.start_dispatched = false;
            effect.vars.clear();
            effect.command_target = None;
            effect.command_id = None;
            effect
        })
        .collect()
}

/// Renders an effect list for a divergence message. Shared so the per-object
/// and global comparisons describe effects the same way.
fn describe_effects(effects: &[EffectState]) -> String {
    effects
        .iter()
        .map(|effect| {
            format!(
                "{}(prio {} int {} t {})",
                effect.name, effect.priority, effect.interval, effect.timer
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn optional_fixed(raw: i32, pixel: i32) -> Option<C4Fixed> {
    let fixed = C4Fixed::from_raw(raw);
    if fixed == itofix(pixel) {
        None
    } else {
        Some(fixed)
    }
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
            base_definition_ids: Vec::with_capacity(snapshot.objects.len()),
            base_graphics_names: Vec::with_capacity(snapshot.objects.len()),
        };

        for object in &snapshot.objects {
            let definition_id = CString::new(object.definition_id.clone()).map_err(|_| {
                format!("definition id for object {} contains null byte", object.id)
            })?;
            let action_name = CString::new(object.action.compiled_name())
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

            // The ABI field is Action.Time (RustEngineBridge.cpp:397,1173).
            let action_ticks = object.action.time;

            let has_container = object.container.is_some();
            let container_id = object.container.map(|id| id.as_u64()).unwrap_or_default();
            let fixed_position = snapshot_fixed_position(object);
            let fixed_velocity = snapshot_fixed_velocity(object);
            let fixed_rotation = object
                .fixed_rotation
                .unwrap_or_else(|| itofix(object.rotation));
            let rotation_velocity = object.rotation_velocity.unwrap_or(C4Fixed::ZERO);

            let (has_base_graphics, base_definition_ptr, base_graphics_ptr, base_blit_mode) =
                if let Some(base) = object.base_graphics.as_ref() {
                    let definition = CString::new(base.definition.clone()).map_err(|_| {
                        format!(
                            "base definition id for object {} contains null byte",
                            object.id
                        )
                    })?;
                    let name_cstring = if let Some(name) = base.graphics_name.as_ref() {
                        Some(CString::new(name.clone()).map_err(|_| {
                            format!(
                                "base graphics name for object {} contains null byte",
                                object.id
                            )
                        })?)
                    } else {
                        None
                    };
                    buffer.base_definition_ids.push(Some(definition));
                    buffer.base_graphics_names.push(name_cstring);
                    let def_ptr = buffer
                        .base_definition_ids
                        .last()
                        .and_then(|value| value.as_ref())
                        .map(|cstr| cstr.as_ptr())
                        .unwrap_or(ptr::null());
                    let name_ptr = buffer
                        .base_graphics_names
                        .last()
                        .and_then(|value| value.as_ref())
                        .map(|cstr| cstr.as_ptr())
                        .unwrap_or(ptr::null());
                    (true, def_ptr, name_ptr, base.blit_mode)
                } else {
                    buffer.base_definition_ids.push(None);
                    buffer.base_graphics_names.push(None);
                    (false, ptr::null(), ptr::null(), 0)
                };

            buffer.objects.push(LcEngineRuntimeObjectState {
                id: object.id.as_u64(),
                definition_id: buffer.definition_ids.last().unwrap().as_ptr(),
                position_x: object.position.x,
                position_y: object.position.y,
                velocity_x: object.velocity.x,
                velocity_y: object.velocity.y,
                rotation: object.rotation,
                fixed_position_x: fixed_position.x.val(),
                fixed_position_y: fixed_position.y.val(),
                fixed_velocity_x: fixed_velocity.x.val(),
                fixed_velocity_y: fixed_velocity.y.val(),
                fixed_rotation: fixed_rotation.val(),
                mobile: object.mobile,
                in_liquid: object.in_liquid,
                object_timer: object.timer,
                rotation_velocity: rotation_velocity.val(),
                energy: object.energy,
                construction: object.construction,
                damage: object.damage,
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
                has_base_graphics,
                base_definition_id: base_definition_ptr,
                base_graphics_name: base_graphics_ptr,
                base_blit_mode,
                has_draw_transform: object.draw_transform.is_some(),
                draw_scale_x: object
                    .draw_transform
                    .map(|transform| transform.scale_x)
                    .unwrap_or(1.0),
                draw_scale_y: object
                    .draw_transform
                    .map(|transform| transform.scale_y)
                    .unwrap_or(1.0),
                draw_offset_x: object
                    .draw_transform
                    .map(|transform| transform.offset_x)
                    .unwrap_or(0.0),
                draw_offset_y: object
                    .draw_transform
                    .map(|transform| transform.offset_y)
                    .unwrap_or(0.0),
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
        // C++ ActIdle is the EMPTY action name; the runtime's idle
        // sentinel is "Idle" (SetActionByName treats them identically,
        // C4Object.cpp:4214-4215) — map the representation so idle
        // compares as idle.
        let action_name = if action_name.is_empty() {
            String::from("Idle")
        } else {
            action_name
        };
        let mut action = ActionState::new(action_name);
        action.phase = entry.action_phase;
        action.time = entry.action_ticks;
        action.data = entry.action_data;

        let direction = Direction::from_raw(entry.direction);
        let command_direction = CommandDirection::from_raw(entry.command_direction);

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

        let base_graphics = if entry.has_base_graphics {
            let definition_ptr = entry.base_definition_id;
            if definition_ptr.is_null() {
                return None;
            }
            let definition = match CStr::from_ptr(definition_ptr).to_str() {
                Ok(value) => value.to_string(),
                Err(_) => CStr::from_ptr(definition_ptr)
                    .to_string_lossy()
                    .into_owned(),
            };
            let graphics_name = if entry.base_graphics_name.is_null() {
                None
            } else {
                Some(
                    CStr::from_ptr(entry.base_graphics_name)
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            Some(ObjectBaseGraphics {
                definition,
                graphics_name,
                blit_mode: entry.base_blit_mode,
            })
        } else {
            None
        };

        snapshots.push(ObjectSnapshot {
            id: ObjectId::new(entry.id),
            definition_id,
            custom_name: None,
            position: Vector2::new(entry.position_x, entry.position_y),
            velocity: Vector2::new(entry.velocity_x, entry.velocity_y),
            // The bridge exports C4Object::r verbatim. Negative rotations are
            // valid after DoMovement's circle bound and must survive restore.
            rotation: entry.rotation,
            energy: entry.energy,
            // The current bridge ABI does not expose C4Object::NeedEnergy.
            need_energy: false,
            construction: entry.construction,
            damage: entry.damage,
            magic_energy: entry.magic_energy,
            magic_capacity: entry.magic_capacity,
            action,
            direction,
            command_direction,
            action_procedure: None,
            effects,
            vertices,
            current_shape: None,
            current_fire_top: None,
            // The bridge ABI does not expose the live C4Shape field yet.
            contact_density: crate::CONTACT_DENSITY_SOLID,
            own_vertices: None,
            vertex_contacts: Vec::new(),
            solid_mask_override: None,
            container,
            // The bridge ABI does not expose C4Object::pLayer yet.
            layer: None,
            // The bridge ABI does not expose C4Object::Visibility yet.
            visibility: 0,
            // `base_blit_mode` is SetGraphics state, not C4Object::BlitMode;
            // the bridge ABI does not expose the latter yet.
            blit_mode: 0,
            // The bridge ABI does not expose object picture state yet.
            color: 0,
            color_modulation: 0,
            picture_rect: Default::default(),
            contents,
            // Was a HashMap when the pinned bridge was written; the port now
            // models C4IDList's ordering with ComponentList.
            components: crate::ComponentList::new(),
            // The bridge ABI does not expose C4Object::Component yet.
            component_order: Vec::new(),
            status: ObjectStatus::Normal,
            // The bridge ABI does not carry Base or Controller yet. The C++
            // expected-side snapshot mirrors the owner-inherit default
            // for Controller (C4Object.cpp:162); the comparator checks neither.
            base: crate::OWNER_NONE,
            controller: entry.owner,
            owner: entry.owner,
            category: entry.category,
            crew_member: entry.crew_member,
            // Projected from the existing crew-selection ABI below; adding
            // a field to LcEngineObjectSnapshot would break the C++ layout.
            selected: false,
            alive: entry.alive,
            base_graphics,
            graphics_overlays: Vec::new(),
            draw_transform: if entry.has_draw_transform {
                Some(DrawTransform::from_components(
                    entry.draw_scale_x,
                    entry.draw_scale_y,
                    entry.draw_offset_x,
                    entry.draw_offset_y,
                ))
            } else {
                None
            },
            command_queue: Vec::new(),
            command_stack: CommandStackSnapshot::default(),
            local_vars: HashMap::new(),
            in_liquid: entry.in_liquid,
            mobile: entry.mobile,
            // The C++ side recomputes OCF continuously; the compare does not
            // cover it — import as stored.
            ocf: 0,
            timer: entry.object_timer.max(0),
            own_mass: 0,
            on_fire: false,
            fire_phase: 0,
            fire_caused_by: crate::OWNER_NONE,
            info_physical: None,
            temporary_physical: None,
            physical_changes: Vec::new(),
            breath: 0,
            // The current bridge ABI does not expose C4Object::PlrViewRange.
            plr_view_range: 0,
            last_energy_loss_cause: crate::OWNER_NONE,
            fixed_position: optional_fixed_vec(
                entry.fixed_position_x,
                entry.fixed_position_y,
                Vector2::new(entry.position_x, entry.position_y),
            ),
            fixed_velocity: optional_fixed_vec(
                entry.fixed_velocity_x,
                entry.fixed_velocity_y,
                Vector2::new(entry.velocity_x, entry.velocity_y),
            ),
            rotation_velocity: if entry.rotation_velocity == 0 {
                None
            } else {
                Some(C4Fixed::from_raw(entry.rotation_velocity))
            },
            fixed_rotation: optional_fixed(entry.fixed_rotation, entry.rotation),
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
            // LcEngineParticleSnapshot has no PXS fields, so a bridged
            // particle carries neither the exact C4Fixed pair nor the chunk
            // slot. Comparing PXS through this ABI needs the header extended
            // first (parity/bridge/lc_engine_ffi.h:22-33).
            pxs_fixed: None,
            pxs_slot: None,
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
    // The bridge already exports C4Object::Select through each player's
    // selected-id list. Rehydrate the canonical per-object bit without an
    // ABI change, while retaining the legacy projection for comparisons.
    for (&owner, selection) in &crew_selection_map {
        for &id in &selection.selected {
            if let Some(object) = snapshots
                .iter_mut()
                .find(|object| object.id == id && object.owner == owner)
            {
                object.selected = true;
            }
        }
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
            wealth: entry.wealth,
            score: entry.score,
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
        game_time: 0,
        game_over: false,
        round_results: Default::default(),
        league_name: Vec::new(),
        player_info_league_progress_data: Default::default(),
        player_info_league_scores: Default::default(),
        physics: None,
        objects: snapshots,
        render_order: Vec::new(),
        environment: EnvironmentFrame::default(),
        sky: None,
        weather_events: Vec::new(),
        global_effects: global_effects_vec,
        script_globals: Default::default(),
        particles: particle_snapshots,
        players: Vec::new(),
        fow_players: Default::default(),
        crew_selection: crew_selection_map,
        crew_roles: crew_role_map,
        known_crew_owners,
        eliminated_crew_owners,
        landscape: None,
        rng: LcgRng::seed_from_u64(frame),
        hud: HudSnapshot {
            players: hud_players_vec,
            messages: Vec::new(),
            scoreboard: Default::default(),
            scoreboard_presentations: Vec::new(),
            local_players: Vec::new(),
        },
        surfaces: surface_snapshots,
        controls: control_entries,
        network_packets: network_snapshots,
        definition_categories: HashMap::new(),
        definition_closed_containers: Default::default(),
        definition_lines: HashMap::new(),
        transfer_zones: Vec::new(),
        pathfinder_debug: Default::default(),
        menu_requests: Vec::new(),
        audio: Vec::new(),
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
            "frame rust {}, cpp {}",
            expected.frame, actual.frame
        ));
    }

    if expected.objects.len() != actual.objects.len() {
        // Name the difference: a per-definition histogram diff turns
        // "count mismatch" into an actionable worklist.
        let histogram = |objects: &[ObjectSnapshot]| {
            let mut counts: BTreeMap<String, i64> = BTreeMap::new();
            for object in objects {
                *counts.entry(object.definition_id.clone()).or_default() += 1;
            }
            counts
        };
        let ours = histogram(&expected.objects);
        let theirs = histogram(&actual.objects);
        let mut missing: Vec<String> = Vec::new();
        let mut extra: Vec<String> = Vec::new();
        let ids: std::collections::BTreeSet<&String> = ours.keys().chain(theirs.keys()).collect();
        for id in ids {
            let have = ours.get(id).copied().unwrap_or(0);
            let want = theirs.get(id).copied().unwrap_or(0);
            match have.cmp(&want) {
                std::cmp::Ordering::Less => missing.push(format!("{}x {id}", want - have)),
                std::cmp::Ordering::Greater => extra.push(format!("{}x {id}", have - want)),
                std::cmp::Ordering::Equal => {}
            }
        }
        return Some(format!(
            "object count mismatch (rust {}, cpp {}; runtime missing: [{}]; runtime extra: [{}])",
            expected.objects.len(),
            actual.objects.len(),
            missing.join(", "),
            extra.join(", "),
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
                // Different definitions under the same Number = the
                // late-spawn NUMBERING SKEW (creation order diverged);
                // reporting it first stops the skew from masquerading as
                // alive/energy/effects field noise.
                if expected_object.definition_id != actual_object.definition_id {
                    problems.push(format!(
                        "object {} definition rust {}, cpp {}",
                        id, expected_object.definition_id, actual_object.definition_id
                    ));
                }
                if expected_object.position != actual_object.position {
                    problems.push(format!(
                        "object {} position rust {:?}, cpp {:?}",
                        id, expected_object.position, actual_object.position
                    ));
                }
                // Sub-pixel 16.16 state: the integer compare masks drift
                // until a pixel boundary crossing — compare the raw fixed
                // values (None ⇒ itofix(position), the lossless case).
                let expected_fix = expected_object.fixed_position.unwrap_or_else(|| {
                    FixedVec2::from_ints(expected_object.position.x, expected_object.position.y)
                });
                let actual_fix = actual_object.fixed_position.unwrap_or_else(|| {
                    FixedVec2::from_ints(actual_object.position.x, actual_object.position.y)
                });
                if expected_fix != actual_fix {
                    problems.push(format!(
                        "object {} subpix position rust ({},{}), cpp ({},{})",
                        id,
                        expected_fix.x.val(),
                        expected_fix.y.val(),
                        actual_fix.x.val(),
                        actual_fix.y.val()
                    ));
                }
                if expected_object.mobile != actual_object.mobile {
                    problems.push(format!(
                        "object {} mobile rust {}, cpp {}",
                        id, expected_object.mobile, actual_object.mobile
                    ));
                }
                if expected_object.timer != actual_object.timer {
                    problems.push(format!(
                        "object {} def-timer rust {}, cpp {}",
                        id, expected_object.timer, actual_object.timer
                    ));
                }
                let expected_fixv = expected_object.fixed_velocity.unwrap_or_else(|| {
                    FixedVec2::from_ints(expected_object.velocity.x, expected_object.velocity.y)
                });
                let actual_fixv = actual_object.fixed_velocity.unwrap_or_else(|| {
                    FixedVec2::from_ints(actual_object.velocity.x, actual_object.velocity.y)
                });
                if expected_fixv != actual_fixv {
                    problems.push(format!(
                        "object {} subpix velocity rust ({},{}), cpp ({},{})",
                        id,
                        expected_fixv.x.val(),
                        expected_fixv.y.val(),
                        actual_fixv.x.val(),
                        actual_fixv.y.val()
                    ));
                }
                if expected_object.velocity != actual_object.velocity {
                    problems.push(format!(
                        "object {} velocity rust {:?}, cpp {:?}",
                        id, expected_object.velocity, actual_object.velocity
                    ));
                }
                if expected_object.rotation != actual_object.rotation {
                    problems.push(format!(
                        "object {} rotation rust {}, cpp {}",
                        id, expected_object.rotation, actual_object.rotation
                    ));
                }
                let expected_fixr = expected_object
                    .fixed_rotation
                    .unwrap_or_else(|| itofix(expected_object.rotation));
                let actual_fixr = actual_object
                    .fixed_rotation
                    .unwrap_or_else(|| itofix(actual_object.rotation));
                if expected_fixr != actual_fixr {
                    problems.push(format!(
                        "object {} subdegree rotation rust {}, cpp {}",
                        id,
                        expected_fixr.val(),
                        actual_fixr.val()
                    ));
                }
                let expected_rdir = expected_object.rotation_velocity.unwrap_or(C4Fixed::ZERO);
                let actual_rdir = actual_object.rotation_velocity.unwrap_or(C4Fixed::ZERO);
                if expected_rdir != actual_rdir {
                    problems.push(format!(
                        "object {} rotation velocity rust {}, cpp {}",
                        id,
                        expected_rdir.val(),
                        actual_rdir.val()
                    ));
                }
                if expected_object.energy != actual_object.energy {
                    problems.push(format!(
                        "object {} energy rust {}, cpp {}",
                        id, expected_object.energy, actual_object.energy
                    ));
                }
                if expected_object.owner != actual_object.owner {
                    problems.push(format!(
                        "object {} owner rust {}, cpp {}",
                        id, expected_object.owner, actual_object.owner
                    ));
                }
                if expected_object.crew_member != actual_object.crew_member {
                    problems.push(format!(
                        "object {} crew member rust {}, cpp {}",
                        id, expected_object.crew_member, actual_object.crew_member
                    ));
                }
                if expected_object.alive != actual_object.alive {
                    problems.push(format!(
                        "object {} alive rust {}, cpp {}",
                        id, expected_object.alive, actual_object.alive
                    ));
                }
                if expected_object.action.name != actual_object.action.name {
                    problems.push(format!(
                        "object {} action rust {}, cpp {}",
                        id, expected_object.action.name, actual_object.action.name
                    ));
                }
                if expected_object.action.phase != actual_object.action.phase {
                    problems.push(format!(
                        "object {} action phase rust {}, cpp {}",
                        id, expected_object.action.phase, actual_object.action.phase
                    ));
                }
                if expected_object.direction != actual_object.direction {
                    problems.push(format!(
                        "object {} direction rust {:?}, cpp {:?}",
                        id, expected_object.direction, actual_object.direction
                    ));
                }
                if expected_object.command_direction != actual_object.command_direction {
                    problems.push(format!(
                        "object {} command direction rust {:?}, cpp {:?}",
                        id, expected_object.command_direction, actual_object.command_direction
                    ));
                }
                if comparable_effects(&expected_object.effects)
                    != comparable_effects(&actual_object.effects)
                {
                    problems.push(format!(
                        "object {} effects rust [{}], cpp [{}]",
                        id,
                        describe_effects(&expected_object.effects),
                        describe_effects(&actual_object.effects)
                    ));
                }
                if expected_object.vertices != actual_object.vertices {
                    problems.push(format!(
                        "object {} vertices mismatch (rust {:?}, cpp {:?})",
                        id, expected_object.vertices, actual_object.vertices
                    ));
                }
            }
            None => problems.push(format!("object {} only on the RUST side", id)),
        }
    }

    for id in actual_objects.keys() {
        if !expected_objects.contains_key(id) {
            problems.push(format!("object {} only on the CPP side", id));
        }
    }

    if comparable_effects(&expected.global_effects) != comparable_effects(&actual.global_effects) {
        problems.push(format!(
            "global effects rust [{}], cpp [{}]",
            describe_effects(&expected.global_effects),
            describe_effects(&actual.global_effects)
        ));
    }

    // Particles are NOT C++ sync state (C4ControlSyncCheck hashes frame/
    // control/rng/player data only; C4Particle uses SafeRandom) — the
    // strict equality only ever passed while both sides were empty.
    // Opt back in with LC_RUST_ENGINE_COMPARE_PARTICLES=1.
    if std::env::var("LC_RUST_ENGINE_COMPARE_PARTICLES").is_ok()
        && expected.particles != actual.particles
    {
        problems.push(format!(
            "particle state mismatch (expected {} entries, got {})",
            expected.particles.len(),
            actual.particles.len()
        ));
    }

    if expected.crew_selection != actual.crew_selection {
        problems.push(format!(
            "crew selection mismatch (rust {:?}, cpp {:?})",
            expected.crew_selection, actual.crew_selection
        ));
    }

    if expected.crew_roles != actual.crew_roles {
        problems.push(format!(
            "crew roles mismatch (rust {:?}, cpp {:?})",
            expected.crew_roles, actual.crew_roles
        ));
    }

    if expected.known_crew_owners != actual.known_crew_owners {
        problems.push(format!(
            "known crew owners mismatch (rust {:?}, cpp {:?})",
            expected.known_crew_owners, actual.known_crew_owners
        ));
    }

    if expected.eliminated_crew_owners != actual.eliminated_crew_owners {
        problems.push(format!(
            "eliminated crew owners mismatch (rust {:?}, cpp {:?})",
            expected.eliminated_crew_owners, actual.eliminated_crew_owners
        ));
    }

    if expected.controls != actual.controls {
        problems.push(format!(
            "controls mismatch (rust {:?}, cpp {:?})",
            expected.controls, actual.controls
        ));
    }

    // The bridge does not export C4GameMessageList or C4Player::LocalControl.
    // Empty cpp fields against populated Rust state are comparator
    // asymmetries, not simulation divergences. Compare the transported HUD
    // fields only (RustEngineBridge.cpp:1343-1399).
    {
        let mut expected_hud = expected.hud.clone();
        let mut actual_hud = actual.hud.clone();
        if actual_hud.messages.is_empty() || expected_hud.messages.is_empty() {
            expected_hud.messages.clear();
            actual_hud.messages.clear();
        }
        expected_hud.local_players.clear();
        actual_hud.local_players.clear();
        if expected_hud != actual_hud {
            problems.push(format!(
                "hud mismatch (rust {expected_hud:?}, cpp {actual_hud:?})"
            ));
        }
    }

    // The rust runtime exports no render surfaces; the cpp side always
    // does. Only compare when BOTH sides carry hashes.
    if !expected.surfaces.is_empty()
        && !actual.surfaces.is_empty()
        && expected.surfaces != actual.surfaces
    {
        problems.push(format!(
            "surface hash mismatch (rust {:?}, cpp {:?})",
            expected.surfaces, actual.surfaces
        ));
    }

    if expected.network_packets != actual.network_packets {
        problems.push(format!(
            "network packets mismatch (rust {:?}, cpp {:?})",
            expected.network_packets, actual.network_packets
        ));
    }

    if problems.is_empty() {
        None
    } else {
        Some(problems.join(", "))
    }
}

/// Env-gated diagnostics for the embedded runtime: `LC_RUST_ENGINE_LOG`
/// installs a stderr tracing subscriber (filter syntax like
/// `LC_RUST_ENGINE_LOG=info`). The host process has no other way to see
/// the runtime's warnings.
fn init_runtime_tracing() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Ok(filter) = std::env::var("LC_RUST_ENGINE_LOG") {
            if !filter.is_empty() {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_writer(std::io::stderr)
                    .try_init();
            }
        }
    });
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_new() -> *mut RuntimeHandle {
    init_runtime_tracing();
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

/// Definition search for runtime-loaded real content. Explicit
/// DefinitionFilenames use executable-data roots; folder/scenario-local
/// definitions are loaded separately by Scenario (C4Game.cpp:81-103,
/// 184-213).
struct RuntimeDefinitionResolver {
    roots: Vec<PathBuf>,
    language_packs: clonk_resources::LanguagePacks,
}

impl crate::scenario::LegacyDefinitionResolver for RuntimeDefinitionResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &clonk_resources::Group,
        identifier: &str,
    ) -> Result<Vec<clonk_resources::Group>, crate::ScenarioError> {
        let normalized = identifier.replace('\\', "/");
        let relative = std::path::Path::new(&normalized);
        let mut groups: Vec<clonk_resources::Group> = Vec::new();
        for root in &self.roots {
            let candidate = root.join(relative);
            if !candidate.exists() {
                continue;
            }
            let group = clonk_resources::Group::open(&candidate)?;
            if groups
                .iter()
                .all(|existing| existing.root() != group.root())
            {
                groups.push(group);
            }
        }
        if groups.is_empty() {
            return Err(crate::ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            });
        }
        Ok(groups)
    }

    fn resolve_material_groups(
        &self,
        scenario: &clonk_resources::Group,
    ) -> Result<Vec<clonk_resources::Group>, crate::ScenarioError> {
        let mut groups = Vec::new();
        let mut candidates = scenario
            .root()
            .ancestors()
            .map(|root| root.join("Material.c4g"))
            .collect::<Vec<_>>();
        candidates.extend(self.roots.iter().map(|root| root.join("Material.c4g")));
        for candidate in candidates {
            let Ok(group) = clonk_resources::Group::open(&candidate) else {
                continue;
            };
            if groups
                .iter()
                .all(|existing: &clonk_resources::Group| existing.root() != group.root())
            {
                groups.push(group);
            }
        }
        Ok(groups)
    }

    fn resolve_language_packs(
        &self,
        _scenario: &clonk_resources::Group,
    ) -> Result<clonk_resources::LanguagePacks, crate::ScenarioError> {
        Ok(self.language_packs.clone())
    }
}

fn runtime_language_packs(install_root: &std::path::Path) -> clonk_resources::LanguagePacks {
    let planet = install_root.join("planet");
    clonk_resources::LanguagePacks::discover(
        &[planet.join("Language.c4g")],
        &[
            install_root.join("content"),
            planet,
            install_root.to_path_buf(),
        ],
    )
}

fn load_runtime_system_scripts(
    group: &clonk_resources::Group,
    language_packs: &clonk_resources::LanguagePacks,
) -> Result<Vec<(String, String)>, crate::ScenarioError> {
    let components = language_packs.component_groups(group, None, None);
    crate::scenario::load_system_scripts_with_components(group, &components, &["US", "DE"])
}

fn load_scenario_into_runtime(
    runtime: &mut RuntimeHandle,
    path: &PathBuf,
    seed: u64,
) -> Result<(), String> {
    let mut roots = path
        .ancestors()
        .skip(1)
        .map(std::path::Path::to_path_buf)
        .collect::<Vec<_>>();
    roots.reverse();
    let install_root = path
        .ancestors()
        .find(|ancestor| ancestor.join("planet/System.c4g").exists())
        .map(std::path::Path::to_path_buf);
    let language_packs = install_root
        .as_deref()
        .map(runtime_language_packs)
        .unwrap_or_default();
    let resolver = RuntimeDefinitionResolver {
        roots,
        language_packs: language_packs.clone(),
    };
    let scenario_group = clonk_resources::Group::open(path)
        .map_err(|error| format!("failed to load scenario: {error}"))?;
    let replay_startup = if scenario_group.exists("Scenario.json") {
        None
    } else {
        Scenario::preflight_replay_startup_from_group(&scenario_group)
            .map_err(|error| format!("failed to load scenario: {error}"))?
    };
    let scenario = match replay_startup {
        Some(startup) => {
            Scenario::load_from_group_with_languages_and_seed_and_startup_player_count(
                &scenario_group,
                &resolver,
                &["US", "DE"],
                seed,
                startup.startup_player_count,
            )
        }
        None => Scenario::load_from_group_with_seed(&scenario_group, &resolver, seed),
    }
    .map_err(|error| format!("failed to load scenario: {error}"))?;
    let scenario_is_replay = scenario
        .lobby_metadata()
        .is_some_and(|metadata| metadata.head().is_replay());
    let replay_player_infos = if scenario_is_replay {
        let group = clonk_resources::Group::open(path)
            .map_err(|error| format!("failed to open replay group: {error}"))?;
        if group.exists("PlayerInfos.txt") {
            let bytes = group
                .read_file("PlayerInfos.txt")
                .map_err(|error| format!("failed to read replay PlayerInfos.txt: {error}"))?;
            Some(
                crate::control::parse_replay_player_infos_ini(&bytes)
                    .map_err(|error| format!("failed to parse replay PlayerInfos.txt: {error}"))?,
            )
        } else {
            Some(crate::ReplayPlayerInfosDocument::default())
        }
    } else {
        None
    };
    let replay_clients = scenario
        .lobby_metadata()
        .and_then(|metadata| metadata.embedded_game_parameter_values())
        .map(|parameters| {
            parameters
                .clients()
                .iter()
                .map(|client| crate::ClientCoreControlData {
                    client_id: client.id(),
                    activated: client.is_activated(),
                    observer: client.is_observer(),
                    name: LegacyCString::from_bytes(client.name().as_bytes().to_vec())
                        .unwrap_or_default(),
                    nick: LegacyCString::from_bytes(client.nick().as_bytes().to_vec())
                        .unwrap_or_default(),
                    lobby_ready: client.is_lobby_ready(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    runtime.engine = Engine::with_seed(seed);
    runtime.engine.set_control_host(false);
    runtime.engine.set_replay_control(true);
    if let Some(startup) = replay_startup {
        runtime
            .engine
            .freeze_startup_player_count(startup.startup_player_count);
    }
    // The clonk-app boot sequence: materials, then the engine-global
    // System.c4g scripts, then the scenario (which adds its own System.c4g).
    {
        // C4Game::InitMaterialTexture (C4Game.cpp:882-960): the
        // scenario-local Material.c4g loads FIRST, the global one after
        // when the local TexMap.txt says OverloadMaterials; each load
        // prepends new names (C4Material.cpp:263-299).
        let open_library = |root: &std::path::Path| {
            clonk_resources::Group::open(root.join("Material.c4g"))
                .ok()
                .and_then(|group| clonk_resources::MaterialLibrary::from_group(&group).ok())
        };
        let local_root = path
            .join("Material.c4g")
            .exists()
            .then(|| path.to_path_buf());
        let local = local_root.as_deref().and_then(open_library);
        let overload_materials = local_root
            .as_deref()
            .map(|root| {
                std::fs::read(root.join("Material.c4g").join("TexMap.txt"))
                    .map(|bytes| {
                        clonk_resources::texmap::TextureMap::parse_bytes(&bytes).overload_materials
                    })
                    .unwrap_or(true)
            })
            .unwrap_or(true);
        let global = overload_materials
            .then(|| {
                path.ancestors()
                    .skip(1)
                    .find(|ancestor| ancestor.join("Material.c4g").exists())
                    .and_then(open_library)
            })
            .flatten();
        let loads: Vec<&clonk_resources::MaterialLibrary> = [local.as_ref(), global.as_ref()]
            .into_iter()
            .flatten()
            .collect();
        if !loads.is_empty() {
            let merged = clonk_resources::MaterialLibrary::from_overloaded_loads(&loads)
                .map_err(|error| format!("failed to merge material libraries: {error}"))?;
            runtime.engine.configure_materials_from_library(&merged);
        }
    }
    if let Some(install_root) = install_root {
        if let Ok(group) = clonk_resources::Group::open(install_root.join("planet/System.c4g")) {
            if let Ok(sources) = load_runtime_system_scripts(&group, &language_packs) {
                runtime.engine.install_global_scripts(&sources);
            }
            // Game.Names: the standard clonk names live next to the
            // System.c4g scripts (C4Game.cpp:2772); the scenario's own
            // Names.txt overrides them at apply.
            runtime.engine.set_standard_names(
                group
                    .read_file("Names.txt")
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            );
        }
    }
    scenario
        .apply(&mut runtime.engine)
        .map_err(|error| format!("failed to apply scenario: {error}"))?;
    runtime.seed = seed;
    runtime.last_frame = runtime.engine.frame();
    runtime.scenario_path = Some(path.clone());
    runtime.scenario_is_replay = scenario_is_replay;
    runtime.local_client_id = (!scenario_is_replay).then_some(0);
    runtime.control_log_strings.clear();
    runtime.control_packets.clear();
    runtime.control_clients = ControlClientRegistry::default();
    if scenario_is_replay {
        // C4ClientList::CompileFunc restores exactly Parameters.Clients and
        // marks every compiled entry remote. PlayerInfos never invent a
        // corresponding client (`C4GameParameters.cpp:374-377`).
        runtime.control_clients.replace_snapshot(replay_clients);
    } else {
        // Offline/host startup clears the compiled list and installs one
        // activated local host before control initialization.
        runtime
            .control_clients
            .replace_snapshot([crate::ClientCoreControlData {
                client_id: 0,
                activated: true,
                observer: false,
                name: LegacyCString::from_bytes(b"Local".to_vec()).unwrap_or_default(),
                nick: LegacyCString::from_bytes(b"Local".to_vec()).unwrap_or_default(),
                lobby_ready: false,
            }]);
    }
    if let Some(player_infos) = replay_player_infos {
        runtime.replace_replay_player_infos(player_infos.clients);
        // Row projection repairs the counter from observed IDs; the named
        // list compiler instead preserves its exact persisted header value.
        runtime
            .engine
            .set_last_player_info_id(player_infos.last_player_id);
    } else {
        runtime.player_infos.clear();
        runtime.player_info_clients.clear();
    }
    runtime.rng_mismatch_reported = false;
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
        Ok(()) => {
            // Landscape parity forensics: dump the runtime's pixel plane
            // (headless xtask has its own dump; this one runs with the
            // LIVE MapSeed the bridge handed over).
            if let Ok(dump) = std::env::var("LC_RUST_ENGINE_DUMP_LANDSCAPE") {
                if let Some((width, height, bytes)) = runtime.engine.debug_landscape_plane() {
                    let mut out = Vec::with_capacity(8 + bytes.len());
                    out.extend_from_slice(&(width as i32).to_le_bytes());
                    out.extend_from_slice(&(height as i32).to_le_bytes());
                    out.extend_from_slice(&bytes);
                    let _ = std::fs::write(&dump, out);
                }
            }
            true
        }
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
            tracing::debug!(
                frame,
                count = packets.len(),
                kinds = ?packets
                    .iter()
                    .map(|packet| match packet {
                        ControlPacket::PlayerControl(_) => "PlayerControl",
                        ControlPacket::PlayerCommand(_) => "PlayerCommand",
                        ControlPacket::PlayerSelect(_) => "PlayerSelect",
                        ControlPacket::EmMoveObject(_) => "EMMoveObject",
                        ControlPacket::EmDrawTool(_) => "EMDrawTool",
                        ControlPacket::EmDropDef(_) => "EMDropDef",
                        ControlPacket::Script(_) => "Script",
                        ControlPacket::Message(_) => "Message",
                        ControlPacket::MessageBoardAnswer(_) => "MessageBoardAnswer",
                        ControlPacket::CustomCommand(_) => "CustomCommand",
                        ControlPacket::ActivateGameGoalMenu(_) => "ActivateGameGoalMenu",
                        ControlPacket::ToggleHostility(_) => "ToggleHostility",
                        ControlPacket::ActivateGameGoalRule(_) => "ActivateGameGoalRule",
                        ControlPacket::SetPlayerTeam(_) => "SetPlayerTeam",
                        ControlPacket::EliminatePlayer(_) => "EliminatePlayer",
                        ControlPacket::InitScenarioPlayer(_) => "InitScenarioPlayer",
                        ControlPacket::SurrenderPlayer(_) => "SurrenderPlayer",
                        ControlPacket::SyncCheck(_) => "SyncCheck",
                        ControlPacket::Synchronize(_) => "Synchronize",
                        ControlPacket::JoinPlayer(_) => "JoinPlayer",
                        ControlPacket::RemovePlayer(_) => "RemovePlayer",
                        ControlPacket::PlayerInfo(_) => "PlayerInfo",
                        ControlPacket::ClientJoin(_) => "ClientJoin",
                        ControlPacket::ClientUpdate(_) => "ClientUpdate",
                        ControlPacket::ClientRemove(_) => "ClientRemove",
                        ControlPacket::Vote(_) => "Vote",
                        ControlPacket::VoteEnd(_) => "VoteEnd",
                        ControlPacket::Set(_) => "Set",
                        ControlPacket::DebugRecord(_) => "DebugRecord",
                        ControlPacket::Unknown { .. } => "Unknown",
                    })
                    .collect::<Vec<_>>(),
                "control packets recorded"
            );
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
        runtime.engine.set_control_host(false);
        runtime.engine.set_replay_control(true);
        runtime.last_frame = runtime.engine.frame();
        runtime.scenario_is_replay = true;
        runtime.local_client_id = None;
        runtime.control_log_strings.clear();
        runtime.control_packets.clear();
        runtime.player_infos.clear();
        runtime.player_info_clients.clear();
        runtime.control_clients = ControlClientRegistry::default();
        runtime.rng_mismatch_reported = false;
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
pub extern "C" fn lc_engine_runtime_find_path(
    handle: *const RuntimeHandle,
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    transfer_zones_enabled: bool,
    level: i32,
    error_out: *mut *mut c_char,
) -> *mut LcEngineRuntimePathResult {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let Some(runtime) = (unsafe { handle.as_ref() }) else {
        set_error(error_out, "runtime handle is null".into());
        return ptr::null_mut();
    };

    if runtime.engine.landscape().is_none() {
        set_error(error_out, "runtime landscape unavailable".into());
        return ptr::null_mut();
    }

    let from = Vector2::new(from_x, from_y);
    let to = Vector2::new(to_x, to_y);
    let path = runtime.find_path(from, to, transfer_zones_enabled, level);
    Box::into_raw(Box::new(LcEngineRuntimePathResult::from_path(path)))
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_path_slice(
    buffer: *const LcEngineRuntimePathResult,
) -> LcEnginePathSlice {
    if buffer.is_null() {
        return LcEnginePathSlice::default();
    }

    unsafe { (*buffer).slice() }
}

#[no_mangle]
pub extern "C" fn lc_engine_runtime_path_free(buffer: *mut LcEngineRuntimePathResult) {
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
    rng_hold: u32,
    rng_count: i32,
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
            // The synced-RNG registers must match once both engines completed the
            // frame (C4Random.h:29-30) — report the FIRST divergence; a ledger slip
            // precedes and explains most downstream state diffs.
            {
                let rng = runtime.engine.debug_rng_clone();
                if std::env::var("LC_RUST_RNG_TRACE").is_ok() {
                    eprintln!(
                        "RNGMARK frame={frame} rust_count={} cpp_count={rng_count}",
                        rng.count
                    );
                }
                if (rng.hold != rng_hold || rng.count != rng_count)
                    && !runtime.rng_mismatch_reported
                {
                    runtime.rng_mismatch_reported = true;
                    tracing::error!(
                        frame,
                        rust_hold = rng.hold,
                        rust_count = rng.count,
                        cpp_hold = rng_hold,
                        cpp_count = rng_count,
                        "synced RNG ledger diverged"
                    );
                }
            }
        }
        if let Some(detail) = runtime_snapshot_mismatch(&expected, &snapshot) {
            let detail = format!("frame {frame}: {detail}");
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
        let detail = format!("frame {frame}: {detail}");
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
    // EngineState does not own C4GameParameters::Clients or
    // C4Game::PlayerInfos. Keep the synchronized lifecycle registries seeded
    // by scenario/replay loading (or earlier controls) across this optional
    // world-state import, exactly as the C++ game does.
    runtime.rng_mismatch_reported = false;

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
        control::{COM_MENU_RIGHT, COM_MENU_SELECT, COM_RELEASE_OFFSET, COM_RIGHT},
        ActionSpec, Definition, EnvironmentSettings, ObjectMenuItem, ObjectMenuState, ObjectUpdate,
        PlayerConfig, PlayerSelectControlData, RgbColor, SpawnConfig, Vector2,
    };
    use serde_json::Value;
    use std::{ffi::CString, ptr};

    fn write_definition_graphics(path: &std::path::Path) {
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(path.join("Graphics.png"))
            .expect("write definition graphics");
    }

    #[test]
    fn runtime_cross_loads_language_pack_tables_for_all_script_scopes() {
        let install = tempfile::tempdir().expect("temporary FFI install");
        let content = install.path().join("content");
        let definition = content.join("Defs.c4d/Good.c4d");
        let scenario = content.join("Probe.c4s");
        let system = install.path().join("planet/System.c4g");
        std::fs::create_dir_all(&definition).expect("definition directory");
        std::fs::create_dir_all(&scenario).expect("scenario directory");
        std::fs::create_dir_all(&system).expect("System.c4g directory");
        std::fs::write(
            definition.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\n",
        )
        .expect("definition core");
        std::fs::write(
            definition.join("Script.c"),
            "global func FfiDefinitionPackValue() { return \"$DefinitionValue$\"; }\n",
        )
        .expect("definition script");
        write_definition_graphics(&definition);
        std::fs::write(
            scenario.join("Scenario.txt"),
            "[Head]\nTitle=Probe\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("scenario core");
        std::fs::write(
            scenario.join("Script.c"),
            "static ffi_scenario_value, ffi_definition_value, ffi_system_value;\n\
             global func Initialize() {\n\
               ffi_scenario_value = \"$ScenarioValue$\";\n\
               ffi_definition_value = FfiDefinitionPackValue();\n\
               ffi_system_value = FfiSystemPackValue();\n\
             }\n",
        )
        .expect("scenario script");
        std::fs::write(
            system.join("Probe.c"),
            "global func FfiSystemPackValue() { return \"$SystemValue$\"; }\n",
        )
        .expect("system script");

        let language = install.path().join("planet/Language.c4g/Finnish.c4g");
        let pack_definition = language.join("Defs.c4d/Good.c4d");
        let pack_scenario = language.join("Probe.c4s");
        let pack_system = language.join("System.c4g");
        std::fs::create_dir_all(&pack_definition).expect("pack definition path");
        std::fs::create_dir_all(&pack_scenario).expect("pack scenario path");
        std::fs::create_dir_all(&pack_system).expect("pack system path");
        std::fs::write(
            pack_definition.join("StringTblUS.txt"),
            "DefinitionValue=definition pack\n",
        )
        .expect("definition pack table");
        std::fs::write(
            pack_scenario.join("StringTblUS.txt"),
            "ScenarioValue=scenario pack\n",
        )
        .expect("scenario pack table");
        std::fs::write(
            pack_system.join("StringTblUS.txt"),
            "SystemValue=system pack\n",
        )
        .expect("system pack table");

        let mut runtime = RuntimeHandle::new();
        load_scenario_into_runtime(&mut runtime, &scenario, 7)
            .expect("FFI scenario loads with language packs");
        let globals = runtime.engine.script_globals.borrow();
        for (name, expected) in [
            ("ffi_scenario_value", "scenario pack"),
            ("ffi_definition_value", "definition pack"),
            ("ffi_system_value", "system pack"),
        ] {
            assert_eq!(
                globals.get(name).map(|cell| cell.borrow().clone()),
                Some(clonk_script::Value::String(expected.to_string().into())),
                "{name} must use its mirrored pack StringTblUS.txt"
            );
        }
    }

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
#strict 3
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
        let selected = [42u64];
        let crew_selection = LcEngineCrewSelectionSnapshot {
            owner: -1,
            selected: selected.as_ptr(),
            selected_count: selected.len(),
            has_cursor: false,
            cursor: 0,
        };

        let object = LcEngineObjectSnapshot {
            id: 42,
            definition_id: definition.as_ptr(),
            position_x: 10,
            position_y: 20,
            velocity_x: -1,
            velocity_y: 2,
            rotation: 0,
            fixed_position_x: itofix(10).val(),
            fixed_position_y: itofix(20).val(),
            fixed_velocity_x: itofix(-1).val(),
            fixed_velocity_y: itofix(2).val(),
            fixed_rotation: itofix(0).val(),
            mobile: false,
            in_liquid: false,
            object_timer: 0,
            rotation_velocity: C4Fixed::ZERO.val(),
            energy: 95,
            construction: crate::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
            owner: -1,
            category: crate::DEFAULT_CATEGORY,
            crew_member: true,
            alive: true,
            action_name: action.as_ptr(),
            action_phase: 3,
            action_ticks: -2,
            action_data: 0,
            direction: 13,
            command_direction: 200,
            effects: &effect_snapshot,
            effect_count: 1,
            vertices: ptr::null(),
            vertex_count: 0,
            has_container: false,
            container_id: 0,
            contents: ptr::null(),
            contents_len: 0,
            has_base_graphics: false,
            base_definition_id: ptr::null(),
            base_graphics_name: ptr::null(),
            base_blit_mode: 0,
            has_draw_transform: false,
            draw_scale_x: 1.0,
            draw_scale_y: 1.0,
            draw_offset_x: 0.0,
            draw_offset_y: 0.0,
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
                &crew_selection,
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
        };

        assert_eq!(snapshot.objects.len(), 1);
        let recorded = &snapshot.objects[0];
        assert_eq!(recorded.effects.len(), 1);
        assert_eq!(recorded.owner, -1);
        assert!(recorded.crew_member);
        assert!(
            recorded.selected,
            "the existing selected-id ABI projects onto C4Object::Select"
        );
        // C4Action::CompileFunc transports Action.Dir as an unrestricted
        // int32 (C4Action.cpp:45-54); the bridge must not collapse it to 0/1.
        assert_eq!(recorded.direction.to_script_value(), 13);
        // ComDir is the adjacent unrestricted int32 field in the same
        // C4Action::CompileFunc block (C4Action.cpp:45-54).
        assert_eq!(recorded.command_direction.to_script_value(), 200);
        // The ABI action_ticks slot carries C4Object Action.Time (the
        // bridge exports obj->Action.Time); the phase-delay counter is
        // not transported.
        assert_eq!(recorded.action.time, -2);

        let effect = &recorded.effects[0];
        assert_eq!(effect.name, "FxFire");
        assert_eq!(effect.priority, 100);
        assert_eq!(effect.interval, 2);
        assert_eq!(effect.timer, 1);
    }

    #[test]
    fn make_snapshot_preserves_raw_fixed_object_state() {
        let definition = CString::new("Clonk").unwrap();
        let action = CString::new("Tumble").unwrap();
        let object = LcEngineObjectSnapshot {
            id: 77,
            definition_id: definition.as_ptr(),
            position_x: 2,
            position_y: -3,
            velocity_x: 0,
            velocity_y: 1,
            rotation: -5,
            fixed_position_x: itofix(2).val() + 123,
            fixed_position_y: itofix(-3).val() - 456,
            fixed_velocity_x: 300,
            fixed_velocity_y: itofix(1).val() + 789,
            fixed_rotation: itofix(-5).val() - 42,
            mobile: false,
            in_liquid: false,
            object_timer: 0,
            rotation_velocity: itofix(1).val(),
            energy: 0,
            construction: crate::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
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
            vertices: ptr::null(),
            vertex_count: 0,
            has_container: false,
            container_id: 0,
            contents: ptr::null(),
            contents_len: 0,
            has_base_graphics: false,
            base_definition_id: ptr::null(),
            base_graphics_name: ptr::null(),
            base_blit_mode: 0,
            has_draw_transform: false,
            draw_scale_x: 1.0,
            draw_scale_y: 1.0,
            draw_offset_x: 0.0,
            draw_offset_y: 0.0,
        };

        let snapshot = unsafe {
            call_make_snapshot(
                9,
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

        let recorded = &snapshot.objects[0];
        assert_eq!(recorded.position, Vector2::new(2, -3));
        assert_eq!(recorded.rotation, -5, "raw signed r is preserved");
        assert_eq!(
            recorded
                .fixed_position
                .expect("raw fixed position preserved")
                .x
                .val(),
            itofix(2).val() + 123
        );
        assert_eq!(
            recorded
                .fixed_velocity
                .expect("raw fixed velocity preserved")
                .x
                .val(),
            300
        );
        assert_eq!(
            recorded
                .rotation_velocity
                .expect("raw rdir preserved")
                .val(),
            itofix(1).val()
        );
        assert_eq!(
            recorded.fixed_rotation.expect("raw fix_r preserved").val(),
            itofix(-5).val() - 42
        );
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
            rotation: 0,
            fixed_position_x: itofix(0).val(),
            fixed_position_y: itofix(0).val(),
            fixed_velocity_x: itofix(0).val(),
            fixed_velocity_y: itofix(0).val(),
            fixed_rotation: itofix(0).val(),
            mobile: false,
            in_liquid: false,
            object_timer: 0,
            rotation_velocity: C4Fixed::ZERO.val(),
            energy: 0,
            construction: crate::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
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
            has_base_graphics: false,
            base_definition_id: ptr::null(),
            base_graphics_name: ptr::null(),
            base_blit_mode: 0,
            has_draw_transform: false,
            draw_scale_x: 1.0,
            draw_scale_y: 1.0,
            draw_offset_x: 0.0,
            draw_offset_y: 0.0,
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
            rotation: 0,
            fixed_position_x: itofix(0).val(),
            fixed_position_y: itofix(0).val(),
            fixed_velocity_x: itofix(0).val(),
            fixed_velocity_y: itofix(0).val(),
            fixed_rotation: itofix(0).val(),
            mobile: false,
            in_liquid: false,
            object_timer: 0,
            rotation_velocity: C4Fixed::ZERO.val(),
            energy: 0,
            construction: crate::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
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
            has_base_graphics: false,
            base_definition_id: ptr::null(),
            base_graphics_name: ptr::null(),
            base_blit_mode: 0,
            has_draw_transform: false,
            draw_scale_x: 1.0,
            draw_scale_y: 1.0,
            draw_offset_x: 0.0,
            draw_offset_y: 0.0,
        };

        let child_snapshot = LcEngineObjectSnapshot {
            id: 2,
            definition_id: ptr::null(),
            position_x: 0,
            position_y: 0,
            velocity_x: 0,
            velocity_y: 0,
            rotation: 0,
            fixed_position_x: itofix(0).val(),
            fixed_position_y: itofix(0).val(),
            fixed_velocity_x: itofix(0).val(),
            fixed_velocity_y: itofix(0).val(),
            fixed_rotation: itofix(0).val(),
            mobile: false,
            in_liquid: false,
            object_timer: 0,
            rotation_velocity: C4Fixed::ZERO.val(),
            energy: 0,
            construction: crate::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
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
            has_base_graphics: false,
            base_definition_id: ptr::null(),
            base_graphics_name: ptr::null(),
            base_blit_mode: 0,
            has_draw_transform: false,
            draw_scale_x: 1.0,
            draw_scale_y: 1.0,
            draw_offset_x: 0.0,
            draw_offset_y: 0.0,
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
            wealth: 125,
            score: 42,
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
        assert_eq!(player.wealth, 125);
        assert_eq!(player.score, 42);
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
    fn runtime_mismatch_count_branch_names_per_definition_diff() {
        // A bare "object count mismatch" is not actionable; the histogram
        // diff names which definitions diverge. Keys are C4ID text — the
        // bridge sends C4IdText(Def->id) (RustEngineBridge.cpp:1141), the
        // runtime keys objects by definition_id.
        let object = |id: u64, definition: &str| -> ObjectSnapshot {
            serde_json::from_value(serde_json::json!({
                "id": id,
                "definition_id": definition,
                "position": {"x": 0, "y": 0},
                "velocity": {"x": 0, "y": 0},
                "energy": 0,
            }))
            .expect("object snapshot deserializes")
        };

        let baseline = unsafe {
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

        let mut runtime = baseline.clone();
        runtime.objects = vec![object(1, "ROCK"), object(2, "GOLD"), object(6, "TREE")];
        let mut cpp = baseline;
        cpp.objects = vec![
            object(1, "ROCK"),
            object(3, "GOLD"),
            object(4, "GOLD"),
            object(5, "BNDT"),
        ];

        let detail = runtime_snapshot_mismatch(&runtime, &cpp).expect("should report mismatch");
        assert!(
            detail.contains("runtime missing: [1x BNDT, 1x GOLD]"),
            "detail did not name the missing definitions: {detail}"
        );
        assert!(
            detail.contains("runtime extra: [1x TREE]"),
            "detail did not name the extra definitions: {detail}"
        );
    }

    #[test]
    fn runtime_mismatch_reports_raw_rotation_state() {
        let object = || -> ObjectSnapshot {
            serde_json::from_value(serde_json::json!({
                "id": 1,
                "definition_id": "ROCK",
                "position": {"x": 0, "y": 0},
                "velocity": {"x": 0, "y": 0},
                "rotation": -9,
                "energy": 0,
                "fixed_rotation": -589524,
                "rotation_velocity": 32768,
            }))
            .expect("object snapshot deserializes")
        };
        let baseline = unsafe {
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
        let mut expected = baseline;
        expected.objects = vec![object()];

        let cases: [(&str, fn(&mut ObjectSnapshot)); 3] = [
            ("rotation rust", |snapshot: &mut ObjectSnapshot| {
                snapshot.rotation += 1
            }),
            ("subdegree rotation", |snapshot: &mut ObjectSnapshot| {
                snapshot.fixed_rotation = Some(C4Fixed::from_raw(-589523));
            }),
            ("rotation velocity", |snapshot: &mut ObjectSnapshot| {
                snapshot.rotation_velocity = Some(C4Fixed::from_raw(32767));
            }),
        ];
        for (label, mutate) in cases {
            let mut actual = expected.clone();
            mutate(&mut actual.objects[0]);
            let detail = runtime_snapshot_mismatch(&expected, &actual)
                .expect("rotation difference is reported");
            assert!(detail.contains(label), "missing {label} in {detail}");
        }
    }

    /// A bare "global effects mismatch" gives a reader nothing to act on —
    /// the per-object comparison prints both lists, and this one has to as
    /// well, or a divergence here cannot be triaged without a debugger.
    #[test]
    fn runtime_mismatch_names_the_global_effects_that_differ() {
        let effect_name = CString::new("FxWeather").unwrap();
        let effect = LcEngineEffectSnapshot {
            name: effect_name.as_ptr(),
            priority: 100,
            interval: 2,
            timer: 7,
        };

        let actual = unsafe {
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
        let expected = unsafe {
            call_make_snapshot(
                1,
                ptr::null(),
                0,
                &effect,
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

        let detail = runtime_snapshot_mismatch(&expected, &actual)
            .expect("a global effect only the port carries is a mismatch");
        assert!(
            detail.contains("FxWeather"),
            "the message must name the differing effect: {detail}"
        );
        assert!(
            detail.contains("cpp []"),
            "the message must show the C++ side was empty: {detail}"
        );
    }

    /// `LcEngineEffectSnapshot` carries only name/priority/interval/timer, so
    /// C++ reports every other field as a default and comparing them raw makes
    /// a divergence out of nothing. The per-object comparison already
    /// neutralises them; the global one has to agree, or every scenario that
    /// runs a global effect reports a false positive.
    #[test]
    fn runtime_mismatch_ignores_global_effect_state_the_abi_cannot_carry() {
        let effect_name = CString::new("IntSchedule").unwrap();
        let effect = LcEngineEffectSnapshot {
            name: effect_name.as_ptr(),
            priority: 1,
            interval: 100,
            timer: 1,
        };
        let actual = unsafe {
            call_make_snapshot(
                1,
                ptr::null(),
                0,
                &effect,
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

        let mut expected = actual.clone();
        let carried = &mut expected.global_effects[0];
        carried.number = 1;
        carried.start_dispatched = true;
        carried.command_target = Some(7);

        assert_eq!(runtime_snapshot_mismatch(&expected, &actual), None);
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
            wealth: 0,
            score: 0,
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
    fn runtime_mismatch_ignores_cpp_unexported_local_players() {
        // C4Player::LocalControl is client-local/no-save state
        // (C4Player.h:81), but RustEngineBridge's HUD ABI exports no local
        // flag (RustEngineBridge.cpp:1343-1399; lc_engine_ffi.h:113-122).
        let actual = unsafe {
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
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };
        let mut expected = actual.clone();
        expected.hud.local_players.push(0);

        assert_eq!(runtime_snapshot_mismatch(&expected, &actual), None);
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
#strict 3
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

        runtime
            .engine
            .tick_without_snapshot()
            .expect("tick succeeds");
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
    fn runtime_object_state_export_preserves_raw_fixed_state() {
        let mut runtime = RuntimeHandle::new();
        runtime
            .engine
            .register_definition(
                Definition::from_script(
                    "Mover",
                    "Mover",
                    "global func Initialize(state, random) { return 0; }",
                )
                .unwrap(),
            )
            .expect("definition registers");
        let id = runtime
            .engine
            .spawn_object(SpawnConfig::new("Mover").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = runtime.engine.find_object_index(id).expect("object exists");
        let object = &mut runtime.engine.objects[idx];
        object.fixed_position = FixedVec2::new(C4Fixed::from_raw(12345), C4Fixed::from_raw(-23456));
        object.set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(300),
            C4Fixed::from_raw(70000),
        ));
        object.state.rotation = 5;
        object.fixed_rotation = C4Fixed::from_raw(itofix(5).val() + 42);
        object.rotation_velocity = itofix(1);

        let snapshot = runtime.engine.snapshot();
        let buffer =
            LcEngineRuntimeObjectStateArray::from_snapshot(&snapshot).expect("snapshot exports");
        assert_eq!(buffer.objects.len(), 1);
        let state = &buffer.objects[0];
        assert_eq!(state.position_x, 0);
        assert_eq!(state.fixed_position_x, 12345);
        assert_eq!(state.fixed_position_y, -23456);
        assert_eq!(state.velocity_x, 0);
        assert_eq!(state.fixed_velocity_x, 300);
        assert_eq!(state.fixed_velocity_y, 70000);
        assert_eq!(state.rotation, 5);
        assert_eq!(state.fixed_rotation, itofix(5).val() + 42);
        assert_eq!(state.rotation_velocity, itofix(1).val());
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
    fn join_control_packets_run_the_join_pipeline() {
        // CID_PlrInfo registers the C4PlayerInfo, CID_JoinPlr executes
        // C4Game::JoinPlayer with the named player file
        // (C4Control.cpp:710-775, local-control branch) — both at frame 0
        // before the first compare (Control.Execute runs before
        // ExecObjects, C4Game.cpp:776-854).
        let dir = tempfile::tempdir().expect("tempdir");

        // A minimal legacy scenario with one crew def.
        let defs = dir.path().join("Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&defs).expect("def dir");
        std::fs::write(
            defs.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=1\n",
        )
        .expect("defcore");
        std::fs::write(defs.join("Script.c"), "// crew\n").expect("script");
        write_definition_graphics(&defs);
        let scenario_dir = dir.path().join("Join.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Join\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n\n\
             [Player1]\nCrew=GOOD=2\n",
        )
        .expect("scenario core");

        // A directory-format player file with one crew info.
        let player_dir = dir.path().join("Tester.c4p");
        std::fs::create_dir_all(&player_dir).expect("player dir");
        std::fs::write(
            player_dir.join("Player.txt"),
            "[Player]\nName=Tyler\n\n[Preferences]\nColor=2\nColorDw=15997440\n",
        )
        .expect("player core");

        let mut runtime = RuntimeHandle::new();
        load_scenario_into_runtime(&mut runtime, &scenario_dir.to_path_buf(), 7)
            .expect("scenario loads");
        assert!(runtime.engine.player(0).is_none());

        let control = format!(
            "[Control]\n\
              [IDPacket]\n\
                ID=144\n\
                [Player Info]\n\
                  ID=0\n\
                  Flags=Initial\n\
                  [Player]\n\
                    Name=\"Tyler\"\n\
                    ID=1\n\
                    Type=User\n\
                    Color=15997440\n\
                    Team=0\n\
                ByClient=0\n\
              [IDPacket]\n\
                ID=145\n\
                [Join Player]\n\
                  Filename=\"{}\"\n\
                  AtClient=0\n\
                  InfoID=1\n\
                  ByRes=false\n\
                  ByClient=0\n",
            player_dir.display()
        );
        let packets = parse_control_ini(&control).expect("control parses");
        runtime.control_packets.insert(0, packets);
        runtime
            .apply_control_packets_for_frame(0)
            .expect("join control applies");

        let player = runtime.engine.player(0).expect("player joined");
        assert_eq!(player.name(), "Tyler");
        let crew_count = runtime
            .engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.owner == 0 && object.crew_member)
            .count();
        assert_eq!(crew_count, 2, "ready crew placed by the join");
    }

    #[test]
    fn replay_late_join_uses_frozen_startup_player_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&defs).expect("def dir");
        std::fs::write(
            defs.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\n",
        )
        .expect("defcore");
        std::fs::write(defs.join("Script.c"), "// definition\n").expect("script");
        write_definition_graphics(&defs);

        let scenario_dir = dir.path().join("Replay.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Replay\nReplay=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n",
        )
        .expect("scenario core");
        std::fs::write(
            scenario_dir.join("Parameters.txt"),
            "[Parameters]\nRandomSeed=7\nStartupPlayerCount=4\n  [Client]\n  ID=0\n  Activated=true\n  Name=Host\n  Nick=Host\n",
        )
        .expect("replay parameters");
        std::fs::write(
            scenario_dir.join("PlayerInfos.txt"),
            concat!(
                "[PlayerInfoList]\n",
                "LastPlayerID=2\n",
                "  [Client]\n",
                "  ID=0\n",
                "  Flags=Initial\n",
                "    [Player]\n",
                "    Name=Player 1\n",
                "    ID=1\n",
                "    Type=User\n",
                "    [Player]\n",
                "    Name=Player 2\n",
                "    ID=2\n",
                "    Type=User\n",
            ),
        )
        .expect("initial replay player infos");

        let player_dir = scenario_dir.join("17-Replay.c4p");
        std::fs::create_dir_all(&player_dir).expect("player dir");
        std::fs::write(
            player_dir.join("Player.txt"),
            "[Player]\nName=Replay Player\n\n[Preferences]\nPosition=3\nColorDw=255\n",
        )
        .expect("player core");
        let player_filename =
            LegacyCString::from_bytes(b"Replay.c4p".to_vec()).expect("player filename");
        let player_resource = crate::NetworkResourceCore {
            resource_type: 3,
            id: 17,
            filename: player_filename.clone(),
            ..crate::NetworkResourceCore::default()
        };
        let info = |id, flags| ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(format!("Player {id}").into_bytes())
                .expect("player name"),
            id,
            flags,
            ..ControlPlayerInfoEntry::default()
        };
        let join = |info_id| {
            ControlPacket::JoinPlayer(JoinPlayerControlData {
                filename: player_filename.clone(),
                at_client: 0,
                info_id,
                source: JoinPlayerSource::Resource(player_resource.clone()),
                by_client: 0,
            })
        };

        let mut runtime = RuntimeHandle::new();
        load_scenario_into_runtime(&mut runtime, &scenario_dir, 7).expect("replay loads");
        assert_eq!(runtime.engine.startup_player_count(), Some(2));
        runtime.control_packets.insert(
            0,
            vec![
                ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                    client_id: 0,
                    flags: crate::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![info(1, 0), info(2, 0)],
                    by_client: 0,
                }),
                join(1),
                join(2),
            ],
        );
        runtime
            .apply_control_packets_for_frame(0)
            .expect("initial replay joins apply");
        assert_eq!(runtime.engine.player(0).unwrap().position_index(), 1);
        assert_eq!(runtime.engine.player(1).unwrap().position_index(), 0);

        let mut export_error: *mut c_char = ptr::null_mut();
        let json_ptr = lc_engine_runtime_export_state_json(&mut runtime, &mut export_error);
        assert!(export_error.is_null());
        assert!(!json_ptr.is_null());
        let state_json = unsafe {
            let json = CStr::from_ptr(json_ptr)
                .to_str()
                .expect("state JSON is UTF-8")
                .to_owned();
            lc_engine_string_free(json_ptr);
            json
        };
        let state_value: Value = serde_json::from_str(&state_json).expect("state JSON parses");
        assert_eq!(state_value["startup_player_count"], Value::from(2));

        let mut restored = RuntimeHandle::new();
        load_scenario_into_runtime(&mut restored, &scenario_dir, 7)
            .expect("restored replay loads before state import");
        let state_json = CString::new(state_json).expect("state CString");
        let mut import_error: *mut c_char = ptr::null_mut();
        assert!(lc_engine_runtime_import_state_json(
            &mut restored,
            state_json.as_ptr(),
            &mut import_error,
        ));
        assert!(import_error.is_null());
        assert_eq!(restored.engine.startup_player_count(), Some(2));

        restored.control_packets.insert(
            1,
            vec![
                ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                    client_id: 0,
                    flags: 0,
                    players: vec![
                        info(1, crate::PLAYER_INFO_FLAG_JOINED),
                        info(2, crate::PLAYER_INFO_FLAG_JOINED),
                        info(3, 0),
                        info(4, 0),
                    ],
                    by_client: 0,
                }),
                ControlPacket::RemovePlayer(crate::RemovePlayerControlData {
                    player: 0,
                    disconnected: false,
                    by_client: 0,
                }),
                join(3),
            ],
        );
        restored
            .apply_control_packets_for_frame(1)
            .expect("late replay join applies");

        assert_ne!(
            restored.player_infos[&1].flags & crate::PLAYER_INFO_FLAG_REMOVED,
            0,
            "the mutable roster changed after startup"
        );
        assert_eq!(restored.player_infos.len(), 4, "late infos were retained");
        let late = restored
            .engine
            .players()
            .find(|player| player.player_info_id() == 3)
            .expect("late replay player joined");
        assert_eq!(
            late.position_index(),
            1,
            "late join reused the original two-slot startup distribution"
        );
        assert_eq!(restored.engine.startup_player_count(), Some(2));

        let mut removal_only = RuntimeHandle::new();
        load_scenario_into_runtime(&mut removal_only, &scenario_dir, 7)
            .expect("removal replay loads before state import");
        let mut removal_import_error: *mut c_char = ptr::null_mut();
        assert!(lc_engine_runtime_import_state_json(
            &mut removal_only,
            state_json.as_ptr(),
            &mut removal_import_error,
        ));
        assert!(removal_import_error.is_null());
        removal_only.control_packets.insert(
            1,
            vec![
                ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                    client_id: 0,
                    flags: 0,
                    players: vec![
                        info(1, crate::PLAYER_INFO_FLAG_JOINED),
                        info(2, crate::PLAYER_INFO_FLAG_JOINED),
                        info(3, 0),
                    ],
                    by_client: 0,
                }),
                ControlPacket::RemovePlayer(crate::RemovePlayerControlData {
                    player: 0,
                    disconnected: false,
                    by_client: 0,
                }),
                ControlPacket::RemovePlayer(crate::RemovePlayerControlData {
                    player: 1,
                    disconnected: false,
                    by_client: 0,
                }),
                join(3),
            ],
        );
        removal_only
            .apply_control_packets_for_frame(1)
            .expect("post-removal replay join applies");
        assert_eq!(
            removal_only
                .player_infos
                .values()
                .filter(|entry| entry.flags & crate::PLAYER_INFO_FLAG_REMOVED == 0)
                .count(),
            1,
            "only the delayed player remains in the mutable roster"
        );
        let post_removal = removal_only
            .engine
            .players()
            .find(|player| player.player_info_id() == 3)
            .expect("post-removal replay player joined");
        assert_eq!(
            post_removal.position_index(),
            1,
            "roster shrinkage does not change the original two-slot distribution"
        );
        assert_eq!(removal_only.engine.startup_player_count(), Some(2));
    }

    #[test]
    fn replay_load_seeds_lifecycle_registries_across_import_and_reset() {
        let dir = tempfile::tempdir().expect("replay directory");
        let replay = dir.path().join("Lifecycle.c4r");
        let definition = dir.path().join("Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&definition).expect("definition directory");
        std::fs::write(
            definition.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\n",
        )
        .expect("definition core");
        std::fs::write(definition.join("Script.c"), "// fixture\n").expect("definition script");
        write_definition_graphics(&definition);
        std::fs::create_dir_all(&replay).expect("replay group");
        std::fs::write(
            replay.join("Scenario.txt"),
            "[Head]\nTitle=Lifecycle\nReplay=1\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("scenario core");
        std::fs::write(
            replay.join("Parameters.txt"),
            "[Parameters]\n  [Client]\n  ID=3\n  Activated=1\n  Observer=0\n  Name=Alice\n  Nick=Ally\n  LobbyReady=1\n",
        )
        .expect("replay parameters");
        std::fs::write(
            replay.join("PlayerInfos.txt"),
            "[PlayerInfoList]\nLastPlayerID=91\n  [Client]\n  ID=3\n  Flags=Initial\n    [Player]\n    Name=Alice\n    Flags=Joined\n    ID=31\n    Team=2\n    GameNumber=10\n    GameJoinFrame=4\n",
        )
        .expect("initial replay player infos");

        let mut runtime = RuntimeHandle::new();
        load_scenario_into_runtime(&mut runtime, &replay, 7).expect("replay loads");

        let client = runtime.control_clients.state(3).expect("client seeded");
        assert!(client.activated);
        assert!(!client.observer);
        assert_eq!(client.name.as_bytes(), b"Alice");
        assert_eq!(client.nick.as_bytes(), b"Ally");
        assert!(client.lobby_ready);
        assert_eq!(runtime.player_info_clients.get(&3), Some(&vec![31]));
        let info = runtime.player_infos.get(&31).expect("player info seeded");
        assert_eq!(info.name.as_bytes(), b"Alice");
        assert_eq!(info.team, 2);
        assert_ne!(info.flags & crate::PLAYER_INFO_FLAG_JOINED, 0);
        assert_eq!((info.game_number, info.game_join_frame), (10, 4));
        assert_eq!(runtime.engine.last_player_info_id(), 91);

        let state = CString::new(
            serde_json::to_string(&runtime.engine.capture_state()).expect("state serializes"),
        )
        .expect("state has no NUL");
        let mut error: *mut c_char = ptr::null_mut();
        assert!(lc_engine_runtime_import_state_json(
            &mut runtime,
            state.as_ptr(),
            &mut error,
        ));
        assert!(error.is_null());
        assert!(runtime.control_clients.contains(3));
        assert!(runtime.player_infos.contains_key(&31));
        assert_eq!(runtime.player_info_clients.get(&3), Some(&vec![31]));
        assert_eq!(runtime.engine.last_player_info_id(), 91);

        assert!(lc_engine_runtime_reset(&mut runtime, &mut error));
        assert!(error.is_null());
        assert!(runtime.control_clients.contains(3));
        assert!(runtime.player_infos.contains_key(&31));
        assert_eq!(runtime.player_info_clients.get(&3), Some(&vec![31]));
        assert_eq!(runtime.engine.last_player_info_id(), 91);
    }

    #[test]
    fn control_recorded_at_frame_n_executes_before_that_frames_tick() {
        // C4Game::Execute runs Control.Execute BEFORE ExecObjects within
        // the same FrameCounter (C4Game.cpp:776-854), and the bridge
        // records control under that FrameCounter. Advancing past frame N
        // must therefore apply frame-N packets before the tick that moves
        // N -> N+1 — the first OnFrame compare arrives at FrameCounter 1,
        // and frame-0 control (the player join!) must not be skipped.
        let dir = tempfile::tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&defs).expect("def dir");
        std::fs::write(
            defs.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=1\n",
        )
        .expect("defcore");
        std::fs::write(defs.join("Script.c"), "// crew\n").expect("script");
        write_definition_graphics(&defs);
        let scenario_dir = dir.path().join("Join.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Join\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n\n\
             [Player1]\nCrew=GOOD=1\n",
        )
        .expect("scenario core");
        let player_dir = dir.path().join("Tester.c4p");
        std::fs::create_dir_all(&player_dir).expect("player dir");
        std::fs::write(
            player_dir.join("Player.txt"),
            "[Player]\nName=Tyler\n\n[Preferences]\nColorDw=255\n",
        )
        .expect("player core");

        let mut runtime = RuntimeHandle::new();
        load_scenario_into_runtime(&mut runtime, &scenario_dir.to_path_buf(), 7)
            .expect("scenario loads");
        let control = format!(
            "[Control]\n\
              [IDPacket]\n\
                ID=145\n\
                [Join Player]\n\
                  Filename=\"{}\"\n\
                  AtClient=0\n\
                  InfoID=1\n\
                  ByClient=0\n",
            player_dir.display()
        );
        let mut error_ptr: *mut c_char = ptr::null_mut();
        let ok = lc_engine_runtime_record_control_ini(
            &mut runtime,
            0,
            std::ffi::CString::new(control).expect("cstring").as_ptr(),
            &mut error_ptr,
        );
        assert!(ok, "control records");

        // The first compare in a live run advances straight to frame 1.
        runtime.advance_to_frame(1).expect("advance succeeds");
        assert!(
            runtime.engine.player(0).is_some(),
            "frame-0 join executed before the 0 -> 1 tick"
        );
    }

    #[test]
    fn replay_waits_for_recorded_remove_player_control() {
        let mut runtime = RuntimeHandle::new();
        runtime
            .engine
            .register_player(PlayerConfig::new(0, "Replay").with_player_info_id(7))
            .expect("register replay player");
        runtime.player_infos.insert(
            7,
            ControlPlayerInfoEntry {
                id: 7,
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                ..Default::default()
            },
        );
        runtime.control_packets.insert(
            0,
            vec![ControlPacket::Script(crate::ScriptControlData {
                target_object: crate::SCRIPT_SCOPE_GLOBAL,
                strictness: crate::ScriptStrictness::Strict3,
                script: LegacyCString::from_bytes(b"EliminatePlayer(0, true)".to_vec())
                    .expect("script has no NUL"),
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(0)
            .expect("recorded script executes");
        assert!(runtime.engine.player(0).is_some());
        assert!(runtime
            .engine
            .take_pending_remove_player_controls()
            .is_empty());

        runtime.control_packets.insert(
            1,
            vec![ControlPacket::RemovePlayer(
                crate::RemovePlayerControlData {
                    player: 0,
                    disconnected: true,
                    by_client: 0,
                },
            )],
        );
        runtime
            .apply_control_packets_for_frame(1)
            .expect("recorded RemovePlr executes");

        assert!(runtime.engine.player(0).is_none());
        let info = runtime.player_infos.get(&7).expect("history remains");
        assert_ne!(info.flags & crate::PLAYER_INFO_FLAG_REMOVED, 0);
        assert_ne!(info.flags & crate::PLAYER_INFO_FLAG_DISCONNECTED, 0);
        assert_eq!(info.game_part_frame, 1);
    }

    #[test]
    fn replay_synchronize_refixes_rng_at_its_frame_and_preserves_packet_order() {
        let script_random = || {
            ControlPacket::Script(crate::ScriptControlData {
                target_object: crate::SCRIPT_SCOPE_GLOBAL,
                strictness: crate::ScriptStrictness::Strict3,
                script: LegacyCString::from_bytes(b"Random(100)".to_vec())
                    .expect("script has no NUL"),
                by_client: 0,
            })
        };
        let mut runtime = RuntimeHandle::new();
        runtime
            .control_packets
            .insert(1, vec![script_random(), script_random()]);
        runtime
            .advance_to_frame(1)
            .expect("advance stops before frame-one control");
        assert_eq!(runtime.engine.debug_rng_clone().count, 500);
        runtime
            .advance_to_frame(2)
            .expect("frame-one controls execute at frame one");
        assert_eq!(runtime.engine.debug_rng_clone().count, 502);

        runtime.control_packets.insert(
            2,
            vec![
                ControlPacket::Synchronize(crate::SynchronizeControlData {
                    save_player_files: false,
                    sync_clearance: false,
                    by_client: 91,
                }),
                script_random(),
            ],
        );
        assert_eq!(
            runtime.engine.debug_rng_clone().count,
            502,
            "frame-two Synchronize does not execute before advancing past frame two"
        );
        runtime
            .advance_to_frame(3)
            .expect("recorded frame-two controls execute");

        let mut expected = crate::rng::LcgRng::seed_from_u64(0);
        expected.random(100);
        assert_eq!(runtime.engine.debug_rng_clone(), expected);
    }

    #[test]
    fn replay_executes_internal_player_script_packets_in_recorded_order() {
        let mut runtime = RuntimeHandle::new();
        for (player, client) in [(1, 7), (2, 9)] {
            runtime
                .engine
                .register_player(PlayerConfig::new(player, format!("Player {player}")))
                .expect("player registers");
            runtime
                .engine
                .player_mut(player)
                .expect("player remains")
                .set_at_client(crate::PlayerAtClient::new(client));
        }
        runtime
            .engine
            .set_teams(vec![crate::TeamInfo::new(4, "Four", 0x44)]);
        runtime
            .engine
            .register_definition(
                Definition::from_script(
                    "RULE",
                    "Rule",
                    "#strict 3\nlocal Marker; func Activate(player) { Marker = player; return true; } func ReadMarker() { return Marker; }",
                )
                .expect("rule compiles"),
            )
            .expect("rule registers");
        let rule = runtime
            .engine
            .spawn_object(SpawnConfig::new("RULE"))
            .expect("rule spawns");
        let rule_number = i32::try_from(rule.as_u64()).expect("fixture id fits i32");

        let controls = format!(
            "[Control]\n\
             [IDPacket]\nID=211\n[Activate Game Goal Menu]\nPlr=1\nByClient=7\n\
             [IDPacket]\nID=212\n[Toggle Hostility]\nOpponent=2\nPlr=1\nByClient=7\n\
             [IDPacket]\nID=214\n[Activate Game Goal/Rule]\nObject={rule_number}\nPlr=1\nByClient=7\n\
             [IDPacket]\nID=215\n[Set Player Team]\nTeam=4\nPlr=1\nByClient=7\n\
             [IDPacket]\nID=216\n[Eliminate Player]\nPlr=2\nByClient=0\n"
        );
        runtime.control_packets.insert(
            0,
            parse_control_ini(&controls).expect("internal controls parse"),
        );
        runtime
            .apply_control_packets_for_frame(0)
            .expect("internal controls replay");

        assert!(runtime.engine.player(1).unwrap().is_hostile_towards(2));
        assert_eq!(runtime.engine.player(1).unwrap().team(), Some(4));
        assert_eq!(
            runtime.engine.player(2).unwrap().status(),
            crate::PlayerStatus::Eliminated
        );
        let rule_index = runtime.engine.find_object_index(rule).unwrap();
        assert_eq!(
            runtime
                .engine
                .call_object_function(rule_index, "ReadMarker", Vec::new())
                .expect("rule marker reads"),
            clonk_script::Value::Int(1)
        );
    }

    #[test]
    fn replay_executes_em_move_object_before_the_frame_tick() {
        let mut runtime = RuntimeHandle::new();
        runtime
            .engine
            .register_definition(
                Definition::from_script("MOVE", "Move", "").expect("definition compiles"),
            )
            .expect("definition registers");
        let object = runtime
            .engine
            .spawn_object(
                SpawnConfig::new("MOVE")
                    .with_position(crate::Vector2::new(7, 11))
                    .with_velocity(crate::Vector2::new(4, -3))
                    .with_mobile(true),
            )
            .expect("object spawns");
        let object_number = i32::try_from(object.as_u64()).expect("fixture id fits i32");
        let controls = format!(
            "[Control]\n\
             [IDPacket]\nID=176\n[EM Move Obj]\nAction=0\ntx=3\nty=-2\nObjectNum=1\nStrict=3\nObjs={object_number}\nByClient=0\n"
        );
        runtime.control_packets.insert(
            0,
            parse_control_ini(&controls).expect("editor control parses"),
        );
        runtime
            .apply_control_packets_for_frame(0)
            .expect("editor control replays");

        let index = runtime.engine.find_object_index(object).unwrap();
        let object = &runtime.engine.objects[index];
        assert_eq!(object.state.position, crate::Vector2::new(10, 9));
        assert_eq!(object.state.velocity, crate::Vector2::ZERO);
        assert!(!object.state.mobile);
    }

    #[test]
    fn replay_executes_em_draw_tool_before_the_frame_tick() {
        let mut runtime = RuntimeHandle::new();
        runtime.engine.set_landscape(crate::Landscape::flat(8, 8));
        let controls = "[Control]\n\
                        [IDPacket]\n\
                        ID=177\n\
                        [EM Draw Tool]\n\
                        Action=0\n\
                        Mode=3\n\
                        ByClient=0\n";
        runtime.control_packets.insert(
            0,
            parse_control_ini(controls).expect("editor draw control parses"),
        );

        runtime
            .apply_control_packets_for_frame(0)
            .expect("editor draw control replays");

        assert_eq!(
            runtime
                .engine
                .landscape()
                .expect("fixture landscape remains")
                .mode(),
            crate::LANDSCAPE_MODE_EXACT
        );
    }

    #[test]
    fn replay_executes_em_drop_def_before_the_frame_tick() {
        let mut runtime = RuntimeHandle::new();
        let mut definition =
            Definition::from_script("DROP", "Drop", "#strict\n").expect("definition compiles");
        definition.set_category(crate::CATEGORY_OBJECT);
        runtime
            .engine
            .register_definition(definition)
            .expect("definition registers");
        let controls = "[Control]\n\
                        [IDPacket]\n\
                        ID=178\n\
                        [EM Drop Def]\n\
                        ID=DROP\n\
                        X=23\n\
                        Y=17\n\
                        ByClient=0\n";
        runtime.control_packets.insert(
            0,
            parse_control_ini(controls).expect("editor drop control parses"),
        );

        runtime
            .apply_control_packets_for_frame(0)
            .expect("editor drop control replays");

        let object = runtime
            .engine
            .first_active_object_for_definition("DROP")
            .and_then(|id| runtime.engine.object_snapshot(id))
            .expect("dropped object exists");
        assert_eq!(object.position, crate::Vector2::new(23, 17));
        assert_eq!(object.owner, crate::OWNER_NONE);
    }

    #[test]
    fn ffi_record_replay_executes_all_known_control_packet_variants() {
        fn record_and_apply(runtime: &mut RuntimeHandle, frame: u64, controls: &str) {
            let controls = CString::new(controls).expect("control INI has no NUL");
            let mut error_ptr: *mut c_char = ptr::null_mut();
            let recorded = lc_engine_runtime_record_control_ini(
                runtime,
                frame,
                controls.as_ptr(),
                &mut error_ptr,
            );
            let error = if error_ptr.is_null() {
                None
            } else {
                let message = unsafe { CStr::from_ptr(error_ptr) }
                    .to_string_lossy()
                    .into_owned();
                lc_engine_string_free(error_ptr);
                Some(message)
            };
            assert!(recorded, "frame {frame} control rejected: {error:?}");
            runtime
                .apply_control_packets_for_frame(frame)
                .unwrap_or_else(|message| panic!("frame {frame} control failed: {message}"));
        }

        let mut runtime = RuntimeHandle::new();
        runtime.engine.set_landscape(Landscape::flat(64, 40));
        runtime
            .engine
            .set_teams(vec![crate::TeamInfo::new(2, "Blue", 0x0000_c800)]);
        runtime.engine.set_allow_debug(true);
        runtime.engine.set_debug_mode(true);

        let chooser = runtime
            .engine
            .join_player_for_team_selection(crate::JoinPlayerConfig {
                name: "Chooser".to_string(),
                player_info_id: 21,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff_0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("team chooser registers");
        runtime
            .engine
            .mark_team_selection_pending(chooser)
            .expect("team choice is pending");
        assert_eq!(chooser, 0);

        runtime
            .engine
            .register_player(PlayerConfig::new(10, "Remote").with_player_info_id(31))
            .expect("remote player registers");
        runtime
            .engine
            .player_mut(10)
            .expect("remote player remains")
            .set_at_client(crate::PlayerAtClient::new(3));
        runtime.player_infos.insert(
            31,
            ControlPlayerInfoEntry {
                id: 31,
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                game_number: 10,
                game_join_frame: 0,
                ..Default::default()
            },
        );
        runtime.player_info_clients.insert(3, vec![31]);

        runtime
            .engine
            .register_player(PlayerConfig::new(11, "Surrender"))
            .expect("surrender player registers");
        runtime
            .engine
            .player_mut(11)
            .expect("surrender player remains")
            .set_at_client(crate::PlayerAtClient::new(7));

        record_and_apply(
            &mut runtime,
            0,
            r#"[Control]
[IDPacket]
ID=135
[Set]
Type=0
Data=50
ByClient=0
[IDPacket]
ID=135
[Set]
Type=1
Data=0
ByClient=7
[IDPacket]
ID=135
[Set]
Type=2
Data=8
ByClient=0
[IDPacket]
ID=135
[Set]
Type=3
Data=4
ByClient=0
[IDPacket]
ID=135
[Set]
Type=4
Data=1
ByClient=0
[IDPacket]
ID=135
[Set]
Type=5
Data=777
ByClient=0
[IDPacket]
ID=135
[Set]
Type=0
Data=-19
ByClient=7
[IDPacket]
ID=135
[Set]
Type=2
Data=99
ByClient=7
[IDPacket]
ID=135
[Set]
Type=3
Data=0
ByClient=7
[IDPacket]
ID=135
[Set]
Type=4
Data=0
ByClient=7
[IDPacket]
ID=135
[Set]
Type=5
Data=-1
ByClient=7
[IDPacket]
ID=135
[Set]
Type=99
Data=123
ByClient=0
[IDPacket]
ID=128
[Client Join]
[ClientCore]
ID=3
Activated=false
Observer=false
Name=Alice
Nick=Ally
LobbyReady=true
ByClient=0
[IDPacket]
ID=129
[Client Update]
Type=0
ClientID=3
Data=1
ByClient=7
[IDPacket]
ID=131
[Voting]
Type=1
Approve=true
Data=3
ByClient=3
[IDPacket]
ID=132
[Voting End]
Type=1
Approve=true
Data=3
ByClient=7
[IDPacket]
ID=210
[Init Scenario Player]
Team=2
Plr=0
ByClient=7
[IDPacket]
ID=213
[Surrender Player]
Plr=11
ByClient=8
[IDPacket]
ID=192
Debug Rec=5:"\000\377@C4"
"#,
        );

        assert_eq!(runtime.engine.control_rate(), 20);
        assert_eq!(runtime.engine.max_players(), Some(8));
        assert_eq!(runtime.engine.team_distribution(), 4);
        assert!(runtime.engine.team_colors());
        assert!(runtime.engine.use_fair_crew());
        assert_eq!(runtime.engine.fair_crew_strength(), 777);
        assert!(!runtime.engine.allow_debug());
        assert!(!runtime.engine.debug_mode());
        let client = runtime.control_clients.state(3).expect("client joined");
        assert!(!client.activated);
        assert!(!client.observer);
        assert_eq!(client.name.as_bytes(), b"Alice");
        assert_eq!(client.nick.as_bytes(), b"Ally");
        assert!(client.lobby_ready);
        assert_eq!(
            runtime.engine.player(chooser).and_then(crate::Player::team),
            None
        );
        assert!(!runtime
            .engine
            .player(11)
            .expect("player remains")
            .surrendered());
        assert_eq!(
            runtime.player_infos[&31].flags & crate::PLAYER_INFO_FLAG_VOTED_OUT,
            0
        );

        runtime.engine.set_fair_crew_forced(true);
        runtime.engine.league_game = true;

        record_and_apply(
            &mut runtime,
            1,
            r#"[Control]
[IDPacket]
ID=129
[Client Update]
Type=0
ClientID=3
Data=1
ByClient=0
[IDPacket]
ID=132
[Voting End]
Type=1
Approve=true
Data=3
ByClient=0
[IDPacket]
ID=210
[Init Scenario Player]
Team=2
Plr=0
ByClient=0
[IDPacket]
ID=213
[Surrender Player]
Plr=11
ByClient=7
[IDPacket]
ID=135
[Set]
Type=2
Data=77
ByClient=0
[IDPacket]
ID=135
[Set]
Type=5
Data=999
ByClient=0
[IDPacket]
ID=132
[Voting End]
Type=1
Approve=false
Data=3
ByClient=0
[IDPacket]
ID=132
[Voting End]
Type=2
Approve=true
Data=1
ByClient=0
[IDPacket]
ID=132
[Voting End]
Type=255
Approve=true
Data=3
ByClient=0
"#,
        );

        assert!(
            runtime
                .control_clients
                .state(3)
                .expect("client remains")
                .activated
        );
        assert_ne!(
            runtime.player_infos[&31].flags & crate::PLAYER_INFO_FLAG_VOTED_OUT,
            0
        );
        assert_eq!(
            runtime.engine.player(chooser).and_then(crate::Player::team),
            Some(2)
        );
        assert!(runtime
            .engine
            .player(11)
            .expect("player remains")
            .surrendered());
        assert_eq!(runtime.engine.max_players(), Some(8));
        assert_eq!(runtime.engine.fair_crew_strength(), 777);

        record_and_apply(
            &mut runtime,
            2,
            r#"[Control]
[IDPacket]
ID=129
[Client Update]
Type=1
ClientID=3
ByClient=0
"#,
        );
        let client = runtime.control_clients.state(3).expect("observer remains");
        assert!(!client.activated);
        assert!(client.observer);
        assert!(runtime.engine.player(10).is_none());
        let remote_info = runtime.player_infos.get(&31).expect("history remains");
        assert_ne!(remote_info.flags & crate::PLAYER_INFO_FLAG_REMOVED, 0);
        assert_ne!(remote_info.flags & crate::PLAYER_INFO_FLAG_DISCONNECTED, 0);
        assert_eq!(remote_info.game_part_frame, 2);

        runtime
            .engine
            .register_player(PlayerConfig::new(12, "Late remote").with_player_info_id(32))
            .expect("late remote player registers");
        runtime
            .engine
            .player_mut(12)
            .expect("late remote remains")
            .set_at_client(crate::PlayerAtClient::new(3));
        runtime.player_infos.insert(
            32,
            ControlPlayerInfoEntry {
                id: 32,
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                game_number: 12,
                game_join_frame: 2,
                ..Default::default()
            },
        );
        runtime.player_infos.insert(
            33,
            ControlPlayerInfoEntry {
                id: 33,
                ..Default::default()
            },
        );
        runtime.player_info_clients.insert(3, vec![31, 32, 33]);

        record_and_apply(
            &mut runtime,
            3,
            r#"[Control]
[IDPacket]
ID=129
[Client Update]
Type=1
ClientID=3
ByClient=0
"#,
        );
        assert!(
            runtime.engine.player(12).is_some(),
            "repeated SetObserver is a full native no-op"
        );

        record_and_apply(
            &mut runtime,
            4,
            r#"[Control]
[IDPacket]
ID=130
[Client Remove]
ClientID=3
Reason=left
ByClient=0
"#,
        );
        assert!(!runtime.control_clients.contains(3));
        assert!(runtime.player_infos.contains_key(&31));
        assert!(runtime.engine.player(12).is_none());
        let late_info = runtime
            .player_infos
            .get(&32)
            .expect("joined history remains");
        assert_ne!(late_info.flags & crate::PLAYER_INFO_FLAG_REMOVED, 0);
        assert_ne!(late_info.flags & crate::PLAYER_INFO_FLAG_DISCONNECTED, 0);
        assert_eq!(late_info.game_part_frame, 4);
        assert!(!runtime.player_infos.contains_key(&33));

        runtime
            .engine
            .register_player(PlayerConfig::new(13, "Unlisted client").with_player_info_id(34))
            .expect("unlisted client player registers");
        runtime
            .engine
            .player_mut(13)
            .expect("unlisted client player remains")
            .set_at_client(crate::PlayerAtClient::new(9));
        runtime.player_infos.insert(
            34,
            ControlPlayerInfoEntry {
                id: 34,
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                game_number: 13,
                game_join_frame: 4,
                ..Default::default()
            },
        );
        runtime.player_info_clients.insert(9, vec![34]);
        record_and_apply(
            &mut runtime,
            5,
            r#"[Control]
[IDPacket]
ID=130
[Client Remove]
ClientID=9
Reason=missing core
ByClient=0
"#,
        );
        assert!(runtime.engine.player(13).is_none());
        assert!(runtime.player_info_clients.contains_key(&9));
        let unlisted_info = runtime.player_infos.get(&34).expect("history remains");
        assert_ne!(unlisted_info.flags & crate::PLAYER_INFO_FLAG_REMOVED, 0);
        assert_ne!(
            unlisted_info.flags & crate::PLAYER_INFO_FLAG_DISCONNECTED,
            0
        );
    }

    #[test]
    fn replay_cancel_vote_uses_cpp_callback_free_abort_selection() {
        let mut runtime = RuntimeHandle::new();
        for (number, client, info_id, name) in [
            (1, -1, 41, "Local"),
            (2, 7, 42, "Remote"),
            (3, -1, 43, "Unknown nonlocal"),
        ] {
            runtime
                .engine
                .register_player(PlayerConfig::new(number, name).with_player_info_id(info_id))
                .expect("player registers");
            runtime
                .engine
                .player_mut(number)
                .expect("player remains")
                .set_at_client(crate::PlayerAtClient::new(client));
            runtime.player_infos.insert(
                info_id,
                ControlPlayerInfoEntry {
                    id: info_id,
                    flags: crate::PLAYER_INFO_FLAG_JOINED,
                    game_number: number,
                    game_join_frame: 0,
                    ..Default::default()
                },
            );
            runtime
                .player_info_clients
                .entry(client)
                .or_default()
                .push(info_id);
        }
        runtime.engine.set_local_players([1]);
        runtime.control_packets.insert(
            0,
            vec![ControlPacket::VoteEnd(crate::VoteControlData {
                vote_type: crate::VOTE_TYPE_CANCEL,
                approve: true,
                data: 0,
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(0)
            .expect("approved cancel replays");

        assert!(
            runtime.engine.player(1).is_none(),
            "local player removed first"
        );
        assert!(
            runtime.engine.player(2).is_none(),
            "remote client player removed"
        );
        assert!(
            runtime.engine.player(3).is_some(),
            "nonlocal player at the replay client ID survives native Abort"
        );
        assert!(
            !runtime.engine.is_game_over(),
            "Abort quits without DoGameOver"
        );
        for info_id in [41, 42, 43] {
            let info = runtime.player_infos.get(&info_id).expect("history remains");
            assert_ne!(info.flags & crate::PLAYER_INFO_FLAG_VOTED_OUT, 0);
            if info_id == 43 {
                assert_eq!(info.flags & crate::PLAYER_INFO_FLAG_REMOVED, 0);
            } else {
                assert_ne!(info.flags & crate::PLAYER_INFO_FLAG_REMOVED, 0);
                assert_ne!(info.flags & crate::PLAYER_INFO_FLAG_DISCONNECTED, 0);
            }
        }

        runtime.control_packets.insert(
            1,
            vec![ControlPacket::VoteEnd(crate::VoteControlData {
                vote_type: crate::VOTE_TYPE_KICK,
                approve: true,
                data: -1,
                by_client: 0,
            })],
        );
        runtime
            .apply_control_packets_for_frame(1)
            .expect("local-client kick replays");
        assert!(
            runtime.engine.is_game_over(),
            "a kick targeting C4ClientIDUnknown invokes DoGameOver in replay"
        );
        let results = runtime.engine.capture_state().round_results;
        assert_eq!(
            results.network_result,
            Some(crate::RoundResultsNetworkResult::NetworkError)
        );
        assert_eq!(
            results.network_result_message,
            b"You have been removed by vote. (It was decided to kick client ???.)"
        );
    }

    #[test]
    fn unknown_replay_control_fails_loudly() {
        let packets =
            parse_control_ini("[Control]\n  [IDPacket]\n    ID=137\n    [Mystery]\n      Data=1\n")
                .expect("unknown packet still parses structurally");
        let mut runtime = RuntimeHandle::new();
        runtime.control_packets.insert(9, packets);

        let error = runtime
            .apply_control_packets_for_frame(9)
            .expect_err("unsupported simulation control must not be dropped");
        assert!(error.contains("0x89"), "missing hex CID: {error}");
        assert!(error.contains("137"), "missing decimal CID: {error}");
        assert!(error.contains("Mystery"), "missing packet name: {error}");
        assert!(error.contains("frame 9"), "missing frame: {error}");
    }

    #[test]
    fn fileless_no_scenario_init_script_player_replays_join() {
        let packets = parse_control_ini(
            "[Control]\n\
             [IDPacket]\n\
             ID=144\n\
             [Player Info]\n\
             ID=7\n\
             [Player]\n\
             Name=Script Bot\n\
             Flags=NoScenarioInit|NoEliminationCheck\n\
             ID=42\n\
             Type=Script\n\
             Color=1193046\n\
             ByClient=0\n\
             [IDPacket]\n\
             ID=145\n\
             [Join Player]\n\
             AtClient=7\n\
             InfoID=42\n\
             ByClient=0\n",
        )
        .expect("script-player record controls parse");
        let mut runtime = RuntimeHandle::new();
        runtime.control_clients.register(7, true, false);
        runtime.control_packets.insert(0, packets);

        runtime
            .apply_control_packets_for_frame(0)
            .expect("fileless script player joins");

        let player = runtime.engine.player(0).expect("script player exists");
        assert_eq!(player.name(), "Script Bot");
        assert_eq!(player.player_info_id(), 42);
        assert_eq!(player.at_client(), crate::PlayerAtClient::new(7));
        assert!(player.is_script_player());
        assert!(player.no_elimination_check());
        let info = runtime.player_infos.get(&42).expect("player info retained");
        assert_ne!(info.flags & crate::PLAYER_INFO_FLAG_JOINED, 0);
        assert_eq!((info.game_number, info.game_join_frame), (0, 0));
    }

    #[test]
    fn replay_join_requires_cpp_client_and_info_and_preserves_client_name() {
        let info = ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Script Bot".to_vec()).expect("player name"),
            id: 42,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            flags: crate::PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                | crate::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
            ..ControlPlayerInfoEntry::default()
        };
        let join = |info_id| {
            ControlPacket::JoinPlayer(JoinPlayerControlData {
                filename: LegacyCString::default(),
                at_client: 7,
                info_id,
                source: JoinPlayerSource::Embedded(Vec::new()),
                by_client: 0,
            })
        };
        let mut runtime = RuntimeHandle::new();
        runtime.control_packets.insert(
            0,
            vec![
                ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                    client_id: 7,
                    players: vec![info.clone()],
                    by_client: 0,
                    ..crate::PlayerInfoControlData::default()
                }),
                join(42),
            ],
        );
        runtime
            .apply_control_packets_for_frame(0)
            .expect("missing-client join is consumed");
        assert!(runtime.engine.players().next().is_none());

        let client_name = LegacyCString::from_bytes(b"Host\xfc".to_vec()).expect("client name");
        runtime.control_packets.insert(
            1,
            vec![
                ControlPacket::ClientJoin(crate::ClientJoinControlData {
                    core: crate::ClientCoreControlData {
                        client_id: 7,
                        activated: true,
                        observer: false,
                        name: client_name.clone(),
                        nick: client_name,
                        lobby_ready: false,
                    },
                    by_client: 0,
                }),
                join(99),
            ],
        );
        runtime
            .apply_control_packets_for_frame(1)
            .expect("missing-info join is consumed");
        assert!(runtime.engine.players().next().is_none());

        runtime.control_packets.insert(2, vec![join(42)]);
        runtime
            .apply_control_packets_for_frame(2)
            .expect("valid fileless script join applies");
        let player = runtime.engine.player(0).expect("script player joined");
        assert_eq!(
            player.at_client_name(),
            clonk_script::c4_string_from_bytes(b"Host\xfc")
        );

        runtime.control_packets.insert(
            3,
            vec![
                ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                    client_id: 7,
                    flags: crate::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        id: 43,
                        ..ControlPlayerInfoEntry::default()
                    }],
                    by_client: 0,
                }),
                join(43),
            ],
        );
        runtime
            .apply_control_packets_for_frame(3)
            .expect("missing user player data is a packet-local no-op");
        assert_eq!(runtime.engine.players().count(), 1);
    }

    #[test]
    fn replay_player_info_replaces_or_appends_per_client() {
        let packet = |client_id, flags, ids: &[i32]| {
            ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                client_id,
                flags,
                players: ids
                    .iter()
                    .map(|id| ControlPlayerInfoEntry {
                        id: *id,
                        ..ControlPlayerInfoEntry::default()
                    })
                    .collect(),
                by_client: 0,
            })
        };
        let mut runtime = RuntimeHandle::new();
        runtime
            .control_packets
            .insert(0, vec![packet(7, 0, &[1]), packet(8, 0, &[9])]);
        runtime
            .apply_control_packets_for_frame(0)
            .expect("initial player infos apply");
        runtime.control_packets.insert(
            1,
            vec![packet(7, crate::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS, &[2])],
        );
        runtime
            .apply_control_packets_for_frame(1)
            .expect("add packet appends");
        assert!(runtime.player_infos.contains_key(&1));
        assert!(runtime.player_infos.contains_key(&2));

        runtime.control_packets.insert(2, vec![packet(7, 0, &[3])]);
        runtime
            .apply_control_packets_for_frame(2)
            .expect("replacement packet replaces one client");
        assert!(!runtime.player_infos.contains_key(&1));
        assert!(!runtime.player_infos.contains_key(&2));
        assert!(runtime.player_infos.contains_key(&3));
        assert!(
            runtime.player_infos.contains_key(&9),
            "another client's list survives"
        );
    }

    #[test]
    fn replay_player_info_team_change_rechecks_both_team_lists() {
        let entry = |id, team, flags| ControlPlayerInfoEntry {
            id,
            team,
            flags,
            ..ControlPlayerInfoEntry::default()
        };
        let packet = |players| {
            ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                client_id: 7,
                players,
                by_client: 0,
                ..crate::PlayerInfoControlData::default()
            })
        };
        let mut runtime = RuntimeHandle::new();
        runtime.control_packets.insert(
            0,
            vec![packet(vec![
                entry(10, 1, 0),
                entry(20, 1, 0),
                entry(30, 2, 0),
                entry(40, 2, 0),
                entry(50, 1, 0),
                entry(60, 2, 0),
            ])],
        );
        runtime
            .apply_control_packets_for_frame(0)
            .expect("initial player infos apply");
        runtime.engine.set_teams(vec![
            crate::TeamInfo::new(1, "One", 0).with_player_ids(vec![50, 20, 99, 50]),
            crate::TeamInfo::new(2, "Two", 0).with_player_ids(vec![30, 40, 60, 77]),
        ]);

        runtime.control_packets.insert(
            1,
            vec![packet(vec![
                entry(10, 1, 0),
                entry(20, 2, 0),
                entry(30, 2, 0),
                entry(40, 1, 0),
                entry(50, 1, 0),
                entry(60, 2, crate::PLAYER_INFO_FLAG_REMOVED),
                entry(0, 1, 0),
            ])],
        );
        runtime
            .apply_control_packets_for_frame(1)
            .expect("runtime team-change info applies");

        assert_eq!(runtime.engine.teams()[0].player_ids, vec![50, 50, 10, 40]);
        assert_eq!(runtime.engine.teams()[1].player_ids, vec![30, 20]);
        assert!(
            runtime.engine.players().next().is_none(),
            "replay PlayerInfo must not synthesize a join"
        );
    }

    #[test]
    fn client_remove_without_player_infos_skips_native_team_recheck() {
        let mut runtime = RuntimeHandle::new();
        runtime.control_clients.register(7, true, false);
        runtime.engine.set_teams(vec![
            crate::TeamInfo::new(1, "One", 0).with_player_ids(vec![99])
        ]);
        runtime.control_packets.insert(
            0,
            vec![ControlPacket::ClientRemove(
                crate::ClientRemoveControlData {
                    client_id: 7,
                    reason: LegacyCString::default(),
                    by_client: 0,
                },
            )],
        );

        runtime
            .apply_control_packets_for_frame(0)
            .expect("client removal replays");

        assert!(!runtime.control_clients.contains(7));
        assert_eq!(runtime.engine.teams()[0].player_ids, vec![99]);
    }

    #[test]
    fn replay_resource_join_prefers_recorded_scenario_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let raw_player = dir.path().join("Raw.c4p");
        let scenario = dir.path().join("Replay.c4s");
        let recorded_player = scenario.join("17-Tyler.c4p");
        std::fs::create_dir_all(&raw_player).expect("raw player dir");
        std::fs::create_dir_all(&recorded_player).expect("recorded player dir");
        std::fs::write(
            raw_player.join("Player.txt"),
            "[Player]\nName=Raw\nScore=1\n",
        )
        .expect("raw player core");
        std::fs::write(
            recorded_player.join("Player.txt"),
            "[Player]\nName=Recorded\nScore=99\n",
        )
        .expect("recorded player core");

        let resource = crate::NetworkResourceCore {
            resource_type: 3,
            id: 17,
            filename: LegacyCString::from_bytes(b"Players/Tyler.c4p".to_vec())
                .expect("resource filename"),
            ..crate::NetworkResourceCore::default()
        };
        let mut runtime = RuntimeHandle::new();
        runtime.control_clients.register(7, true, false);
        runtime.scenario_path = Some(scenario);
        runtime.control_packets.insert(
            0,
            vec![
                ControlPacket::PlayerInfo(crate::PlayerInfoControlData {
                    client_id: 7,
                    flags: 0,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Info".to_vec()).expect("info name"),
                        id: 42,
                        flags: crate::PLAYER_INFO_FLAG_HAS_RESOURCE
                            | crate::PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                            | crate::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
                        resource: Some(resource.clone()),
                        ..ControlPlayerInfoEntry::default()
                    }],
                    by_client: 0,
                }),
                ControlPacket::JoinPlayer(JoinPlayerControlData {
                    filename: LegacyCString::from_bytes(
                        raw_player.as_os_str().to_string_lossy().as_bytes().to_vec(),
                    )
                    .expect("raw filename"),
                    at_client: 7,
                    info_id: 42,
                    source: JoinPlayerSource::Resource(resource),
                    by_client: 0,
                }),
            ],
        );
        runtime
            .apply_control_packets_for_frame(0)
            .expect("resource join applies");

        assert_eq!(runtime.engine.player(0).expect("player joined").score(), 99);
    }

    #[test]
    fn join_without_info_falls_back_to_the_player_file() {
        // Local games install the INITIAL player's C4PlayerInfo directly
        // during InitPlayers (C4PlayerInfoList::LocalJoinUnjoinedPlayersInQueue
        // only queues CID_JoinPlr, C4PlayerInfo.cpp:1292-1323) — the info
        // never traverses the control queue, so the shadow runtime joins
        // from the player file core instead of dropping the join.
        let dir = tempfile::tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&defs).expect("def dir");
        std::fs::write(
            defs.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=1\n",
        )
        .expect("defcore");
        std::fs::write(defs.join("Script.c"), "// crew\n").expect("script");
        write_definition_graphics(&defs);
        let scenario_dir = dir.path().join("Join.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Join\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n\n\
             [Player1]\nCrew=GOOD=1\n",
        )
        .expect("scenario core");
        let player_dir = dir.path().join("Tester.c4p");
        std::fs::create_dir_all(&player_dir).expect("player dir");
        std::fs::write(
            player_dir.join("Player.txt"),
            "[Player]\nName=Tyler\n\n[Preferences]\nColorDw=15997440\n",
        )
        .expect("player core");

        let mut runtime = RuntimeHandle::new();
        load_scenario_into_runtime(&mut runtime, &scenario_dir.to_path_buf(), 7)
            .expect("scenario loads");

        let control = format!(
            "[Control]\n\
              [IDPacket]\n\
                ID=145\n\
                [Join Player]\n\
                  Filename=\"{}\"\n\
                  AtClient=0\n\
                  InfoID=1\n\
                  ByRes=false\n\
                  ByClient=0\n",
            player_dir.display()
        );
        let packets = parse_control_ini(&control).expect("control parses");
        runtime.control_packets.insert(0, packets);
        runtime
            .apply_control_packets_for_frame(0)
            .expect("join control applies");

        let player = runtime.engine.player(0).expect("player joined");
        assert_eq!(player.name(), "Tyler");
    }

    fn runtime_with_cursor_menu() -> (RuntimeHandle, ObjectId) {
        let mut runtime = RuntimeHandle::new();
        let mut definition = Definition::from_script("Test", "Test", "").expect("script compiles");
        definition.set_crew_member(true);
        runtime
            .engine
            .register_definition(definition)
            .expect("definition registers");
        runtime
            .engine
            .register_player(PlayerConfig::new(0, "Test"))
            .expect("player registers");

        let crew = runtime
            .engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        runtime
            .engine
            .select_crew(0, vec![crew])
            .expect("select crew");
        runtime
            .engine
            .set_crew_cursor(0, Some(crew))
            .expect("set cursor");
        let items = ["One", "Two", "Three"]
            .into_iter()
            .map(|caption| ObjectMenuItem {
                caption: caption.to_string(),
                info_caption: String::new(),
                command: "return 1".to_string(),
                command2: String::new(),
                count: 12_345_678,
                item_id: "NONE".to_string(),
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: None,
                components: Vec::new(),
                selectable: true,
                value: None,
                text_display_progress: -1,
            })
            .collect();
        runtime
            .engine
            .apply_object_update(
                crew,
                ObjectUpdate {
                    menu: Some(Some(ObjectMenuState {
                        caption: "Choose".to_string(),
                        symbol_id: "NONE".to_string(),
                        title_symbol: crate::ObjectMenuSymbol::default(),
                        identification: crate::Value::Int(0),
                        style: 0,
                        equal_item_height: false,
                        permanent: false,
                        location: None,
                        location_reset_generation: 0,
                        runtime_id: 0,
                        extra: crate::ObjectMenuExtra::default(),
                        extra_data: 0,
                        internal_refill_token: 0,
                        selection: 0,
                        user_menu: false,
                        command_object: Some(crew),
                        scenario_callbacks: false,
                        refill_object: None,
                        refill_object_contents_count: 0,
                        items,
                        columns: 5,
                        lines: 0,
                        text_progressing: false,
                        decoration: None,
                    })),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install menu");
        (runtime, crew)
    }

    #[test]
    fn player_control_count_ffi_packet_routes_through_count_control() {
        let (mut runtime, _) = runtime_with_cursor_menu();
        runtime.control_packets.insert(
            1,
            vec![ControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                // Raw 273 is countable even though InCom narrows it to the
                // release-range byte 17.
                command: 273,
                data: 4,
                by_client: 0,
            })],
        );
        runtime
            .apply_control_packets_for_frame(1)
            .expect("control applies");

        let player = runtime.engine.player(0).expect("player exists");
        assert_eq!((player.control_count(), player.action_count()), (1, 1));
    }

    #[test]
    fn player_command_ffi_packet_reaches_the_object_command_seam() {
        let (mut runtime, crew) = runtime_with_cursor_menu();
        runtime.control_packets.insert(
            1,
            vec![ControlPacket::PlayerCommand(PlayerCommandControlData {
                player: 0,
                command: crate::command::CommandId::Wait as i32,
                x: 12,
                y: -7,
                target: 999_999,
                target2: 0,
                data: 23,
                add_mode: 1,
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(1)
            .expect("player command applies");

        let commands = runtime
            .engine
            .object_snapshot(crew)
            .expect("crew exists")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "Wait");
        assert_eq!(commands[0].target, None);
        assert_eq!(commands[0].tx, Some(12));
        assert_eq!(commands[0].ty, Some(-7));
        assert_eq!(commands[0].data, crate::command::CommandData::Integer(23));
        let player = runtime.engine.player(0).expect("player exists");
        assert_eq!((player.control_count(), player.action_count()), (1, 1));
    }

    #[test]
    fn player_select_ffi_packet_reaches_the_synchronized_execution_seam() {
        let (mut runtime, crew) = runtime_with_cursor_menu();
        runtime.control_packets.insert(
            1,
            vec![ControlPacket::PlayerSelect(PlayerSelectControlData {
                player: 0,
                objects: vec![crew.as_u64() as i32],
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(1)
            .expect("player selection applies");

        assert_eq!(runtime.engine.selected_crew(0), vec![crew]);
        let player = runtime.engine.player(0).expect("player exists");
        assert_eq!((player.control_count(), player.action_count()), (1, 1));
    }

    #[test]
    fn host_script_ffi_packet_mutates_the_replay_engine() {
        let mut runtime = RuntimeHandle::new();
        assert_ne!(runtime.engine.physics().gravity, 77);
        runtime.control_packets.insert(
            1,
            vec![ControlPacket::Script(crate::ScriptControlData {
                target_object: crate::SCRIPT_SCOPE_GLOBAL,
                strictness: crate::ScriptStrictness::Strict3,
                script: LegacyCString::from_bytes(b"SetGravity(77)".to_vec())
                    .expect("script is NUL-free"),
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(1)
            .expect("script control applies");

        assert_eq!(runtime.engine.physics().gravity, 77);
    }

    #[test]
    fn recorded_menu_right_reaches_the_cursor_menu() {
        // C4ControlPlayerControl::Execute forwards the recorded raw command
        // and data to C4Player::InCom (C4Control.cpp:386-395).
        let (mut runtime, crew) = runtime_with_cursor_menu();
        runtime.control_packets.insert(
            1,
            vec![ControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                command: i32::from(COM_MENU_RIGHT),
                data: 0,
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(1)
            .expect("control applies");

        assert_eq!(
            runtime
                .engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("menu stays open")
                .selection,
            1
        );
    }

    #[test]
    fn recorded_menu_select_forwards_its_data_to_the_cursor_menu() {
        // C4ControlPlayerControl::Execute preserves iData
        // (C4Control.cpp:386-395); C4Menu::Control masks the adjustment bit
        // before selecting that item (C4Menu.cpp:474-476).
        let (mut runtime, crew) = runtime_with_cursor_menu();
        runtime.control_packets.insert(
            1,
            vec![ControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                command: i32::from(COM_MENU_SELECT),
                data: i32::MIN | 2,
                by_client: 0,
            })],
        );

        runtime
            .apply_control_packets_for_frame(1)
            .expect("control applies");

        assert_eq!(
            runtime
                .engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("menu stays open")
                .selection,
            2
        );
    }

    #[test]
    fn runtime_control_application_updates_direction() {
        let mut runtime = RuntimeHandle::new();
        let mut definition = Definition::from_script("Test", "Test", "").expect("script compiles");
        definition.set_crew_member(true);
        definition.configure_actions(
            Some("Walk".to_string()),
            [(
                "Walk".to_string(),
                ActionSpec::default().with_procedure("walk"),
            )]
            .into(),
        );
        runtime
            .engine
            .register_definition(definition)
            .expect("definition registers");
        runtime
            .engine
            .register_player(PlayerConfig::new(0, "Test"))
            .expect("player registers");
        runtime
            .engine
            .player_mut(0)
            .expect("player exists")
            .control
            .control_style = true;

        let crew = runtime
            .engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_owner(0)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("spawn succeeds");
        runtime
            .engine
            .select_crew(0, vec![crew])
            .expect("select crew");
        runtime
            .engine
            .set_crew_cursor(0, Some(crew))
            .expect("set cursor");

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
