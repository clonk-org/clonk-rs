use crate::SimulationSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    frames: Vec<SimulationSnapshot>,
}

impl Recording {
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    pub fn frames(&self) -> &[SimulationSnapshot] {
        &self.frames
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn push(&mut self, snapshot: SimulationSnapshot) {
        self.frames.push(snapshot);
    }

    pub fn into_frames(self) -> Vec<SimulationSnapshot> {
        self.frames
    }

    pub fn to_writer<W: Write>(&self, mut writer: W) -> serde_json::Result<()> {
        serde_json::to_writer_pretty(&mut writer, &self.frames)
    }

    pub fn from_reader<R: Read>(reader: R) -> serde_json::Result<Self> {
        let frames: Vec<SimulationSnapshot> = serde_json::from_reader(reader)?;
        Ok(Self { frames })
    }

    pub fn to_string(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(&self.frames)
    }

    pub fn from_str(data: &str) -> serde_json::Result<Self> {
        let frames: Vec<SimulationSnapshot> = serde_json::from_str(data)?;
        Ok(Self { frames })
    }
}

impl Default for Recording {
    fn default() -> Self {
        Self::new()
    }
}

/// C4RecordChunkType values (C4Record.h:58-61).
pub const RCT_CTRL: u8 = 0x00;
pub const RCT_CTRL_PKT: u8 = 0x01;
pub const RCT_FRAME: u8 = 0x02;
pub const RCT_END: u8 = 0x10;

/// Binary control record stream mirroring `C4Record::Rec`
/// (C4Record.cpp:243-264): 2-byte chunk heads `{frame_diff: u8, type: u8}`
/// (C4RecordChunkHead, C4Record.h:109-113) followed by the raw payload, with
/// empty RCT_Frame filler chunks whenever the frame difference exceeds 0xff,
/// and an RCT_End head at `frame + 37` on close (C4Record.cpp:195-197 — the
/// u8 head field truncates the sum).
#[derive(Debug, Clone, Default)]
pub struct BinaryControlRecord {
    bytes: Vec<u8>,
    last_frame: u32,
}

impl BinaryControlRecord {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rec(&mut self, frame: u32, payload: &[u8], chunk_type: u8) {
        // filler chunks (C4Record.cpp:245-247)
        while frame.saturating_sub(self.last_frame) > 0xff {
            // The difference check guarantees this addition cannot overflow.
            let filler_frame = self.last_frame + 0xff;
            self.rec(filler_frame, &[], RCT_FRAME);
        }
        // get frame difference (C4Record.cpp:248-250)
        let frame_diff = frame.saturating_sub(self.last_frame);
        self.last_frame += frame_diff;
        // head + payload (C4Record.cpp:252-255)
        self.bytes.push(frame_diff as u8);
        self.bytes.push(chunk_type);
        self.bytes.extend_from_slice(payload);
    }

    /// Write the end marker (C4Record.cpp:194-197): an RCT_End head whose u8
    /// frame field carries `frame + 37` truncated.
    pub fn finish(&mut self, frame: u32) {
        self.bytes.push(frame.wrapping_add(37) as u8);
        self.bytes.push(RCT_END);
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod binary_record_tests {
    use super::*;

    #[test]
    fn binary_record_chunk_stream_matches_cpp_format() {
        // C4Record::Rec (C4Record.cpp:243-264) + Stop end marker
        // (C4Record.cpp:194-197).
        let mut record = BinaryControlRecord::new();
        record.rec(5, b"AB", RCT_CTRL);
        record.rec(5, b"C", RCT_CTRL_PKT);
        record.rec(600, b"", RCT_FRAME);
        record.finish(600);
        let bytes = record.into_bytes();
        // chunk 1: diff 5 from frame 0, control payload
        assert_eq!(&bytes[0..4], &[5, RCT_CTRL, b'A', b'B']);
        // chunk 2: same frame → diff 0
        assert_eq!(&bytes[4..7], &[0, RCT_CTRL_PKT, b'C']);
        // frame 600 needs fillers at 5+255=260 and 260+255=515, then diff 85
        assert_eq!(&bytes[7..9], &[255, RCT_FRAME]);
        assert_eq!(&bytes[9..11], &[255, RCT_FRAME]);
        assert_eq!(&bytes[11..13], &[85, RCT_FRAME]);
        // end head: (600 + 37) & 0xff = 125
        assert_eq!(&bytes[13..15], &[125, RCT_END]);
        assert_eq!(bytes.len(), 15);
    }

    #[test]
    fn binary_record_earlier_frame_clamps_diff_to_zero() {
        // C4Record.cpp:249: iLastFrame > iFrame → diff 0 (no rewind).
        let mut record = BinaryControlRecord::new();
        record.rec(10, b"", RCT_CTRL);
        record.rec(3, b"", RCT_CTRL);
        let bytes = record.into_bytes();
        assert_eq!(&bytes[0..2], &[10, RCT_CTRL]);
        assert_eq!(&bytes[2..4], &[0, RCT_CTRL]);
    }

    #[test]
    fn binary_record_handles_uint32_end_frames_like_cpp() {
        let mut record = BinaryControlRecord::new();
        record.finish(u32::MAX);
        let bytes = record.into_bytes();
        assert_eq!(bytes, [36, RCT_END]);
    }
}

#[derive(Debug, Default)]
pub struct Recorder {
    recording: Recording,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, snapshot: &SimulationSnapshot) {
        self.recording.push(snapshot.clone());
    }

    pub fn into_recording(self) -> Recording {
        self.recording
    }

    pub fn frames(&self) -> &[SimulationSnapshot] {
        self.recording.frames()
    }
}

#[derive(Debug)]
pub struct Playback {
    recording: Recording,
    cursor: usize,
}

impl Playback {
    pub fn from_recording(recording: Recording) -> Self {
        Self {
            recording,
            cursor: 0,
        }
    }

    pub fn from_reader<R: Read>(reader: R) -> Result<Self, PlaybackError> {
        Ok(Self::from_recording(Recording::from_reader(reader)?))
    }

    pub fn from_str(data: &str) -> Result<Self, PlaybackError> {
        Ok(Self::from_recording(Recording::from_str(data)?))
    }

    pub fn frames_remaining(&self) -> usize {
        self.recording.frames.len().saturating_sub(self.cursor)
    }

    pub fn validate_snapshot(
        &mut self,
        snapshot: &SimulationSnapshot,
    ) -> Result<(), PlaybackError> {
        let expected = self.recording.frames.get(self.cursor).ok_or({
            PlaybackError::UnexpectedFrame {
                frame: snapshot.frame,
            }
        })?;
        if expected == snapshot {
            self.cursor += 1;
            return Ok(());
        }
        Err(PlaybackError::FrameMismatch {
            frame: expected.frame,
            detail: describe_snapshot_mismatch(expected, snapshot),
        })
    }

    pub fn finish(self) -> Result<(), PlaybackError> {
        if self.cursor == self.recording.frames.len() {
            Ok(())
        } else {
            let frame = self.recording.frames[self.cursor].frame;
            Err(PlaybackError::MissingFrame { frame })
        }
    }

    pub fn validate_sequence<I>(mut self, snapshots: I) -> Result<(), PlaybackError>
    where
        I: IntoIterator<Item = SimulationSnapshot>,
    {
        for snapshot in snapshots {
            self.validate_snapshot(&snapshot)?;
        }
        self.finish()
    }

    pub fn into_recording(self) -> Recording {
        self.recording
    }
}

#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("unexpected frame {frame} (recording exhausted)")]
    UnexpectedFrame { frame: u64 },
    #[error("missing snapshot for frame {frame}")]
    MissingFrame { frame: u64 },
    #[error("frame {frame} mismatch: {detail}")]
    FrameMismatch { frame: u64, detail: String },
    #[error("unable to parse recording: {0}")]
    Parse(#[from] serde_json::Error),
}

fn describe_snapshot_mismatch(
    expected: &SimulationSnapshot,
    actual: &SimulationSnapshot,
) -> String {
    if expected.frame != actual.frame {
        return format!("expected frame {}, got {}", expected.frame, actual.frame);
    }

    if expected.objects.len() != actual.objects.len() {
        return format!(
            "object count mismatch (expected {}, got {})",
            expected.objects.len(),
            actual.objects.len()
        );
    }

    let expected_objects: HashMap<_, _> =
        expected.objects.iter().map(|obj| (obj.id, obj)).collect();
    let actual_objects: HashMap<_, _> = actual.objects.iter().map(|obj| (obj.id, obj)).collect();

    let mut problems = Vec::new();

    for (&id, expected_obj) in &expected_objects {
        match actual_objects.get(&id) {
            Some(actual_obj) => {
                if expected_obj.position != actual_obj.position {
                    problems.push(format!(
                        "object {} position expected {:?}, got {:?}",
                        id, expected_obj.position, actual_obj.position
                    ));
                }
                if expected_obj.velocity != actual_obj.velocity {
                    problems.push(format!(
                        "object {} velocity expected {:?}, got {:?}",
                        id, expected_obj.velocity, actual_obj.velocity
                    ));
                }
                if expected_obj.energy != actual_obj.energy {
                    problems.push(format!(
                        "object {} energy expected {}, got {}",
                        id, expected_obj.energy, actual_obj.energy
                    ));
                }
                if expected_obj.owner != actual_obj.owner {
                    problems.push(format!(
                        "object {} owner expected {}, got {}",
                        id, expected_obj.owner, actual_obj.owner
                    ));
                }
                if expected_obj.crew_member != actual_obj.crew_member {
                    problems.push(format!(
                        "object {} crew_member expected {}, got {}",
                        id, expected_obj.crew_member, actual_obj.crew_member
                    ));
                }
                if expected_obj.action.name != actual_obj.action.name {
                    problems.push(format!(
                        "object {} action expected {}, got {}",
                        id, expected_obj.action.name, actual_obj.action.name
                    ));
                }
                if expected_obj.action.phase != actual_obj.action.phase {
                    problems.push(format!(
                        "object {} action phase expected {}, got {}",
                        id, expected_obj.action.phase, actual_obj.action.phase
                    ));
                }
                if expected_obj.effects != actual_obj.effects {
                    problems.push(format!("object {} effects differed", id));
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

    if expected.landscape != actual.landscape {
        problems.push("landscape mismatch".into());
    }

    if expected.environment.gamma != actual.environment.gamma {
        problems.push("gamma controls mismatch".into());
    }

    if expected.rng != actual.rng {
        problems.push("rng mismatch".into());
    }

    if problems.is_empty() {
        "unknown mismatch".into()
    } else {
        problems.join(", ")
    }
}

impl fmt::Display for Recording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} frames", self.frames.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::LcgRng;
    use crate::{
        ActionState, CommandDirection, CommandStackSnapshot, ComponentList, Direction,
        EnvironmentFrame, HudSnapshot, ObjectSnapshot, ObjectStatus, SimulationSnapshot, Vector2,
        OWNER_NONE,
    };
    use std::collections::HashMap;

    fn make_snapshot(frame: u64, energy: i32) -> SimulationSnapshot {
        SimulationSnapshot {
            frame,
            game_time: 0,
            game_over: false,
            round_results: Default::default(),
            league_name: Vec::new(),
            player_info_league_progress_data: Default::default(),
            player_info_league_scores: Default::default(),
            physics: None,
            objects: vec![ObjectSnapshot {
                id: crate::ObjectId::new(1),
                definition_id: "Test".into(),
                custom_name: None,
                position: Vector2::new(1, 2),
                velocity: Vector2::new(0, 0),
                rotation: 0,
                energy,
                need_energy: false,
                construction: crate::FULL_CON,
                damage: 0,
                magic_energy: 0,
                magic_capacity: 0,
                action: ActionState::default(),
                direction: Direction::default(),
                command_direction: CommandDirection::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                current_shape: None,
                current_fire_top: None,
                contact_density: 50,
                own_vertices: None,
                vertex_contacts: Vec::new(),
                solid_mask_override: None,
                container: None,
                layer: None,
                visibility: 0,
                blit_mode: 0,
                color: 0,
                color_modulation: 0,
                picture_rect: Default::default(),
                contents: Vec::new(),
                components: ComponentList::new(),
                component_order: Vec::new(),
                status: ObjectStatus::Normal,
                owner: OWNER_NONE,
                controller: OWNER_NONE,
                category: crate::DEFAULT_CATEGORY,
                crew_member: false,
                plr_view_range: 0,
                selected: false,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
                in_liquid: false,
                mobile: false,
                ocf: 0,
                timer: 0,
                own_mass: 0,
                on_fire: false,
                fire_phase: 0,
                fire_caused_by: -1,
                info_physical: None,
                temporary_physical: None,
                physical_changes: Vec::new(),
                breath: 0,
                last_energy_loss_cause: -1,
                base: -1,
                fixed_position: None,
                fixed_velocity: None,
                rotation_velocity: None,
                fixed_rotation: None,
            }],
            render_order: Vec::new(),
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            script_globals: Default::default(),
            particles: Vec::new(),
            players: Vec::new(),
            fow_players: Default::default(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners: Vec::new(),
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: LcgRng::seed_from_u64(frame),
            hud: HudSnapshot::default(),
            surfaces: Vec::new(),
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            definition_closed_containers: Default::default(),
            definition_lines: HashMap::new(),
            transfer_zones: Vec::new(),
            pathfinder_debug: Default::default(),
            menu_requests: Vec::new(),
            audio: Vec::new(),
        }
    }

    #[test]
    fn roundtrip_recording() {
        let mut recorder = Recorder::new();
        recorder.record(&make_snapshot(1, 5));
        recorder.record(&make_snapshot(2, 6));
        let recording = recorder.into_recording();
        let serialized = recording.to_string().expect("serializes");
        let parsed = Recording::from_str(&serialized).expect("parses");
        assert_eq!(parsed.frames().len(), 2);
    }

    #[test]
    fn recording_preserves_live_shape_render_state() {
        let mut snapshot = make_snapshot(1, 5);
        snapshot.objects[0].current_shape = Some(crate::DefinitionRect::new(2, 3, 4, 5));
        snapshot.objects[0].current_fire_top = Some(6);
        let recording = Recording {
            frames: vec![snapshot.clone()],
        };

        let serialized = recording.to_string().expect("serializes");
        assert!(serialized.contains("current_shape"));
        assert!(serialized.contains("current_fire_top"));

        let parsed = Recording::from_str(&serialized).expect("parses");
        assert_eq!(parsed.frames(), &[snapshot.clone()]);
        Playback::from_recording(parsed)
            .validate_snapshot(&snapshot)
            .expect("render state round-trips into playback validation");
    }

    #[test]
    fn detects_mismatch() {
        let recording = Recording {
            frames: vec![make_snapshot(1, 5)],
        };
        let mut playback = Playback::from_recording(recording);
        let result = playback.validate_snapshot(&make_snapshot(1, 7));
        assert!(matches!(result, Err(PlaybackError::FrameMismatch { .. })));
    }

    #[test]
    fn detects_control_mismatch() {
        let mut expected_snapshot = make_snapshot(1, 5);
        expected_snapshot.controls = vec!["[Control]\nPlayer=1\n".to_string()];
        let recording = Recording {
            frames: vec![expected_snapshot],
        };
        let mut playback = Playback::from_recording(recording);
        let mut actual_snapshot = make_snapshot(1, 5);
        actual_snapshot.controls = vec!["[Control]\nPlayer=2\n".to_string()];

        let err = playback
            .validate_snapshot(&actual_snapshot)
            .expect_err("mismatch expected");

        match err {
            PlaybackError::FrameMismatch { detail, .. } => {
                assert!(
                    detail.contains("controls mismatch"),
                    "expected controls mismatch detail, got {detail}"
                );
            }
            other => panic!("unexpected error {other:?}"),
        }
    }
}
