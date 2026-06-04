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
        let expected = self.recording.frames.get(self.cursor).ok_or_else(|| {
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
        ActionState, CommandDirection, CommandStackSnapshot, Direction, EnvironmentFrame,
        HudSnapshot, ObjectSnapshot, ObjectStatus, SimulationSnapshot, Vector2, OWNER_NONE,
    };
    use std::collections::HashMap;

    fn make_snapshot(frame: u64, energy: i32) -> SimulationSnapshot {
        SimulationSnapshot {
            frame,
            game_over: false,
            physics: None,
            objects: vec![ObjectSnapshot {
                id: crate::ObjectId::new(1),
                definition_id: "Test".into(),
                position: Vector2::new(1, 2),
                velocity: Vector2::new(0, 0),
                rotation: 0,
                energy,
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
                container: None,
                contents: Vec::new(),
                components: HashMap::new(),
                status: ObjectStatus::Normal,
                owner: OWNER_NONE,
                category: crate::DEFAULT_CATEGORY,
                crew_member: false,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
                fixed_position: None,
                fixed_velocity: None,
                rotation_velocity: None,
                fixed_rotation: None,
            }],
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            particles: Vec::new(),
            players: Vec::new(),
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
            transfer_zones: Vec::new(),
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
