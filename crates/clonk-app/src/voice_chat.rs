//! Proximity voice chat: source authentication, speaking state and the
//! positional mix.
//!
//! This is a deliberate Rust-only extension (clonk-org/clonk-rs#301), **not a
//! parity claim** — there is no C++ oracle for any of it. It is opt-in: nothing
//! here opens a microphone unless `Voice.Enabled` is set. An opted-in player
//! then chooses how it opens, and push-to-talk — the microphone open only for a
//! held configured key — is and stays the default. Voice activation
//! (clonk-org/clonk-rs#422) is the alternative: the capture is open while the
//! player is eligible to speak, and [`VoiceActivationGate`] decides per frame
//! whether what it hears is transmitted. Neither mode weakens the other, and a
//! player who has taken neither opt-in is never recorded at all.
//!
//! **The determinism boundary is the invariant to protect.** Fixed 20 ms,
//! 16 kHz mono IMA ADPCM frames travel on a bounded, droppable UDP media lane
//! after positive Rust-to-Rust capability negotiation. They never enter
//! lockstep controls, snapshots, savegames, records/replays, sync checks or
//! PostMortem recovery, and nothing here may change that: a peer that cannot
//! decode voice, or drops every frame, must still stay in perfect lockstep,
//! which is also what keeps cross-play against a stock LegacyClonk client
//! working. The lane's droppability is load-bearing — do not add
//! retransmission or ordering requirements to it.
//!
//! Source identity is authenticated rather than trusted: each admitted UDP
//! route exchanges an unpredictable media cookie over its reliable control
//! stream, the receiving route supplies the source client ID, and
//! [`authenticated_selected_voice_crew`] revalidates that the claimed player
//! belongs to that client before resolving the live selected
//! `PlayerState.cursor`.
//!
//! Playback uses the existing linear 700-pixel positional mix; the speaker
//! glyph additionally obeys per-viewport object/FoW visibility. Landscape
//! openness and obstacles deliberately do not occlude speech
//! (clonk-org/clonk-rs#418), and the media lane is not encrypted
//! (clonk-org/clonk-rs#426).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use clonk_audio::{
    decode_voice_frame, EncodedVoiceFrame, VoiceCapture, VoiceCaptureError, VoiceInputFrame,
};
use clonk_engine::{ObjectSnapshot, PlayerStatus, SimulationSnapshot};

use crate::settings::VoiceActivation;

pub(crate) const SPEAKING_HANGOVER: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PushToTalkAction {
    Ignore,
    Consume,
    Start,
    Stop,
}

pub(crate) fn push_to_talk_action(
    capture_key: Option<winit::keyboard::KeyCode>,
    configured_key: winit::keyboard::KeyCode,
    enabled: bool,
    eligible: bool,
    repeated: bool,
    key: winit::keyboard::KeyCode,
    state: winit::event::ElementState,
) -> PushToTalkAction {
    if state == winit::event::ElementState::Released && capture_key == Some(key) {
        return PushToTalkAction::Stop;
    }
    if !enabled || key != configured_key {
        return PushToTalkAction::Ignore;
    }
    match state {
        winit::event::ElementState::Released => PushToTalkAction::Consume,
        winit::event::ElementState::Pressed if eligible && !repeated => PushToTalkAction::Start,
        winit::event::ElementState::Pressed => PushToTalkAction::Consume,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VoiceFrameDisposition {
    Accepted,
    AcceptedNewEpoch,
    UnknownPlayer,
    OwnershipMismatch,
    DuplicateOrLate,
}

#[derive(Clone, Copy, Debug)]
struct SpeakerActivity {
    stream_epoch: u32,
    latest_sequence: u16,
    last_frame_at: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct VoiceActivityTracker {
    speakers: BTreeMap<(i32, i32), SpeakerActivity>,
    local_speaker: Option<((i32, i32), Instant)>,
}

pub(crate) struct CapturedVoiceFrame {
    pub(crate) stream_epoch: u32,
    pub(crate) sequence: u16,
    pub(crate) payload: EncodedVoiceFrame,
}

/// Decides which captured frames a voice-activated capture actually transmits.
///
/// Push-to-talk has no gate: a held key is the player saying "send this". Voice
/// activation replaces that decision with the measured input level, so the gate
/// opens on a frame at or above the configured threshold and stays open for a
/// configured tail of frames afterwards — without one, every pause between
/// words would clip the end of the last one.
#[derive(Debug, Default)]
struct VoiceActivationGate {
    open: bool,
    hangover_remaining: u32,
}

impl VoiceActivationGate {
    /// `Some(reopened)` to transmit this frame, where `reopened` marks the
    /// frame that broke a silence and therefore starts a new stream. `None`
    /// suppresses the frame.
    fn admit(&mut self, level: f32, activation: &VoiceActivation) -> Option<bool> {
        let transmit = if level >= activation.threshold {
            self.hangover_remaining = activation.hangover_frames;
            true
        } else if self.hangover_remaining > 0 {
            self.hangover_remaining -= 1;
            true
        } else {
            false
        };
        let reopened = transmit && !self.open;
        self.open = transmit;
        transmit.then_some(reopened)
    }

    fn close(&mut self) {
        self.open = false;
        self.hangover_remaining = 0;
    }
}

pub(crate) struct AcceptedRemoteVoiceFrame {
    pub(crate) stream_id: u64,
    pub(crate) samples: [i16; clonk_audio::VOICE_FRAME_SAMPLES],
    pub(crate) reset_stream: bool,
}

pub(crate) trait VoiceFrameSource {
    fn drain_frames(&self) -> Vec<VoiceInputFrame>;
}

impl VoiceFrameSource for VoiceCapture {
    fn drain_frames(&self) -> Vec<VoiceInputFrame> {
        self.drain_frames()
    }
}

type VoiceCaptureOpener = Box<dyn FnMut() -> Result<Box<dyn VoiceFrameSource>, VoiceCaptureError>>;

#[derive(Debug, Default)]
pub(crate) struct RemoteVoiceStream {
    pub(crate) stream_epoch: u32,
    pub(crate) last_frame_at: Option<Instant>,
}

pub(crate) struct VoiceChatState {
    activity: VoiceActivityTracker,
    capture: Option<Box<dyn VoiceFrameSource>>,
    capture_opener: VoiceCaptureOpener,
    capture_key: Option<winit::keyboard::KeyCode>,
    activation_gate: VoiceActivationGate,
    activation_open_failed: bool,
    stream_epoch: u32,
    next_sequence: u16,
    pub(crate) remote_streams: BTreeMap<(i32, i32), RemoteVoiceStream>,
}

impl Default for VoiceChatState {
    fn default() -> Self {
        Self::with_source_opener(VoiceCapture::open)
    }
}

impl VoiceChatState {
    pub(crate) fn with_source_opener<F, S>(mut opener: F) -> Self
    where
        F: FnMut() -> Result<S, VoiceCaptureError> + 'static,
        S: VoiceFrameSource + 'static,
    {
        Self {
            activity: VoiceActivityTracker::default(),
            capture: None,
            capture_opener: Box::new(move || {
                opener().map(|source| Box::new(source) as Box<dyn VoiceFrameSource>)
            }),
            capture_key: None,
            activation_gate: VoiceActivationGate::default(),
            activation_open_failed: false,
            stream_epoch: 0,
            next_sequence: 0,
            remote_streams: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    fn with_capture_opener<F, S>(opener: F) -> Self
    where
        F: FnMut() -> Result<S, VoiceCaptureError> + 'static,
        S: VoiceFrameSource + 'static,
    {
        Self::with_source_opener(opener)
    }

    /// `key` is the push-to-talk key whose release closes this capture again;
    /// `None` opens a capture no key owns.
    pub(crate) fn start_capture(
        &mut self,
        key: Option<winit::keyboard::KeyCode>,
    ) -> Result<(), VoiceCaptureError> {
        if self.capture.is_some() {
            return Ok(());
        }
        self.capture = Some((self.capture_opener)()?);
        self.capture_key = key;
        self.activation_gate.close();
        self.stream_epoch = self.stream_epoch.wrapping_add(1).max(1);
        self.next_sequence = 0;
        Ok(())
    }

    /// Opens a capture no push-to-talk key owns, for a player who has chosen
    /// voice activation.
    ///
    /// A failed open latches. Voice activation has no key press to rate-limit
    /// it, so without the latch a missing or busy microphone would be reopened
    /// on every tick for as long as the player stays in the game. The latch
    /// clears the next time the capture stops — which is what a player who has
    /// just plugged a microphone in will cause by leaving and re-entering.
    pub(crate) fn start_voice_activated_capture(&mut self) -> Result<(), VoiceCaptureError> {
        if self.capture.is_some() || self.activation_open_failed {
            return Ok(());
        }
        let opened = self.start_capture(None);
        self.activation_open_failed = opened.is_err();
        opened
    }

    pub(crate) fn stop_capture(&mut self) {
        self.capture = None;
        self.capture_key = None;
        self.activation_open_failed = false;
        self.activation_gate.close();
    }

    pub(crate) fn capture_active(&self) -> bool {
        self.capture.is_some()
    }

    pub(crate) fn capture_key(&self) -> Option<winit::keyboard::KeyCode> {
        self.capture_key
    }

    /// `activation` is `Some` only in voice-activated mode, where it decides
    /// per frame whether the microphone's output is transmitted at all. On the
    /// push-to-talk default it is `None` and every captured frame goes out.
    pub(crate) fn drain_captured_frames(
        &mut self,
        activation: Option<&VoiceActivation>,
    ) -> Vec<CapturedVoiceFrame> {
        let Some(capture) = self.capture.as_ref() else {
            return Vec::new();
        };
        capture
            .drain_frames()
            .into_iter()
            .filter_map(|frame| match activation {
                None => Some((frame.payload, false)),
                Some(activation) => self
                    .activation_gate
                    .admit(frame.level, activation)
                    .map(|reopened| (frame.payload, reopened)),
            })
            // The gate borrows `self` mutably, so the stamping pass cannot be
            // fused into it.
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(payload, reopened)| self.stamp_captured_frame(payload, reopened))
            .collect()
    }

    fn stamp_captured_frame(
        &mut self,
        payload: EncodedVoiceFrame,
        starts_new_stream: bool,
    ) -> CapturedVoiceFrame {
        // Nothing has gone out on this epoch yet when the sequence is still 0,
        // so the first frame after a capture opens keeps the epoch
        // `start_capture` already allocated.
        if starts_new_stream && self.next_sequence != 0 {
            self.stream_epoch = self.stream_epoch.wrapping_add(1).max(1);
            self.next_sequence = 0;
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        CapturedVoiceFrame {
            stream_epoch: self.stream_epoch,
            sequence,
            payload,
        }
    }

    pub(crate) fn note_remote_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        client_id: i32,
        player_id: i32,
        stream_epoch: u32,
        sequence: u16,
        received_at: Instant,
    ) -> VoiceFrameDisposition {
        let disposition = self.activity.note_frame(
            snapshot,
            client_id,
            player_id,
            stream_epoch,
            sequence,
            received_at,
        );
        if matches!(
            disposition,
            VoiceFrameDisposition::Accepted | VoiceFrameDisposition::AcceptedNewEpoch
        ) {
            self.remote_streams.insert(
                (client_id, player_id),
                RemoteVoiceStream {
                    stream_epoch,
                    last_frame_at: Some(received_at),
                },
            );
        }
        disposition
    }

    pub(crate) fn accept_remote_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        frame: &clonk_network::VoiceFrame,
        received_at: Instant,
    ) -> Option<AcceptedRemoteVoiceFrame> {
        let samples = decode_voice_frame(&frame.payload).ok()?;
        let client_id = i32::try_from(frame.client_id).ok()?;
        let disposition = self.note_remote_frame(
            snapshot,
            client_id,
            frame.player_id,
            frame.stream_epoch,
            frame.sequence,
            received_at,
        );
        match disposition {
            VoiceFrameDisposition::Accepted | VoiceFrameDisposition::AcceptedNewEpoch => {
                Some(AcceptedRemoteVoiceFrame {
                    stream_id: voice_stream_id(client_id, frame.player_id),
                    samples,
                    reset_stream: disposition == VoiceFrameDisposition::AcceptedNewEpoch,
                })
            }
            VoiceFrameDisposition::UnknownPlayer
            | VoiceFrameDisposition::OwnershipMismatch
            | VoiceFrameDisposition::DuplicateOrLate => None,
        }
    }

    pub(crate) fn note_local_frame(&mut self, client_id: i32, player_id: i32, now: Instant) {
        self.activity.note_local_frame(client_id, player_id, now);
    }

    pub(crate) fn active_speakers(&self, now: Instant) -> Vec<(i32, i32)> {
        self.activity.active_speakers(now)
    }

    pub(crate) fn expire_playback(&mut self, now: Instant) -> Vec<(i32, i32)> {
        self.activity.expire_visual_activity(now);
        let mut expired = Vec::new();
        self.remote_streams.retain(|&speaker, stream| {
            let active = stream.last_frame_at.is_some_and(|last_frame_at| {
                now.saturating_duration_since(last_frame_at) < SPEAKING_HANGOVER
            });
            if !active {
                expired.push(speaker);
            }
            active
        });
        expired
    }

    pub(crate) fn forget_client(&mut self, client_id: i32) -> Vec<(i32, i32)> {
        self.activity.forget_client(client_id);
        let removed = self
            .remote_streams
            .keys()
            .copied()
            .filter(|(speaker_client_id, _)| *speaker_client_id == client_id)
            .collect::<Vec<_>>();
        self.remote_streams
            .retain(|(speaker_client_id, _), _| *speaker_client_id != client_id);
        removed
    }

    pub(crate) fn clear(&mut self) -> Vec<(i32, i32)> {
        self.stop_capture();
        self.activity.clear();
        let removed = self.remote_streams.keys().copied().collect();
        self.remote_streams.clear();
        removed
    }
}

pub(crate) fn voice_stream_id(client_id: i32, player_id: i32) -> u64 {
    (u64::from(client_id as u32) << 32) | u64::from(player_id as u32)
}

pub(crate) fn authenticated_selected_voice_crew(
    snapshot: &SimulationSnapshot,
    client_id: i32,
    player_id: i32,
) -> Option<&ObjectSnapshot> {
    let player = snapshot.players.iter().find(|player| {
        player.id == player_id
            && player.status == PlayerStatus::Active
            && player.at_client.get() == client_id
    })?;
    let object = snapshot.object(player.cursor?)?;
    (object.status == clonk_engine::ObjectStatus::Normal
        && object.ocf & clonk_engine::ocf::CREW_MEMBER != 0)
        .then_some(object)
}

impl VoiceActivityTracker {
    pub(crate) fn note_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        client_id: i32,
        player_id: i32,
        stream_epoch: u32,
        sequence: u16,
        received_at: Instant,
    ) -> VoiceFrameDisposition {
        let Some(player) = snapshot
            .players
            .iter()
            .find(|player| player.id == player_id)
        else {
            return VoiceFrameDisposition::UnknownPlayer;
        };
        if player.at_client.get() != client_id {
            return VoiceFrameDisposition::OwnershipMismatch;
        }
        if player.status != PlayerStatus::Active || player.cursor.is_none() {
            return VoiceFrameDisposition::UnknownPlayer;
        }

        let key = (client_id, player_id);
        let mut disposition = VoiceFrameDisposition::Accepted;
        if let Some(activity) = self.speakers.get_mut(&key) {
            if activity.stream_epoch == stream_epoch {
                let advance = sequence.wrapping_sub(activity.latest_sequence);
                if advance == 0 || advance > u16::MAX / 2 {
                    return VoiceFrameDisposition::DuplicateOrLate;
                }
            } else {
                let advance = stream_epoch.wrapping_sub(activity.stream_epoch);
                if advance == 0 || advance > u32::MAX / 2 {
                    return VoiceFrameDisposition::DuplicateOrLate;
                }
                disposition = VoiceFrameDisposition::AcceptedNewEpoch;
            }
            activity.stream_epoch = stream_epoch;
            activity.latest_sequence = sequence;
            activity.last_frame_at = received_at;
        } else {
            self.speakers.insert(
                key,
                SpeakerActivity {
                    stream_epoch,
                    latest_sequence: sequence,
                    last_frame_at: received_at,
                },
            );
        }
        disposition
    }

    pub(crate) fn active_speakers(&self, now: Instant) -> Vec<(i32, i32)> {
        let mut speakers = self
            .speakers
            .iter()
            .filter_map(|(&speaker, activity)| {
                now.saturating_duration_since(activity.last_frame_at)
                    .lt(&SPEAKING_HANGOVER)
                    .then_some(speaker)
            })
            .collect::<Vec<_>>();
        if let Some((speaker, started_at)) = self.local_speaker {
            if now.saturating_duration_since(started_at) < SPEAKING_HANGOVER
                && !speakers.contains(&speaker)
            {
                speakers.push(speaker);
                speakers.sort_unstable();
            }
        }
        speakers
    }

    pub(crate) fn expire_visual_activity(&mut self, now: Instant) {
        if self.local_speaker.is_some_and(|(_, started_at)| {
            now.saturating_duration_since(started_at) >= SPEAKING_HANGOVER
        }) {
            self.local_speaker = None;
        }
    }

    pub(crate) fn forget_client(&mut self, client_id: i32) {
        self.speakers
            .retain(|(speaker_client_id, _), _| *speaker_client_id != client_id);
        if self
            .local_speaker
            .is_some_and(|((speaker_client_id, _), _)| speaker_client_id == client_id)
        {
            self.local_speaker = None;
        }
    }

    pub(crate) fn note_local_frame(&mut self, client_id: i32, player_id: i32, now: Instant) {
        self.local_speaker = Some(((client_id, player_id), now));
    }

    pub(crate) fn clear(&mut self) {
        self.speakers.clear();
        self.local_speaker = None;
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use clonk_engine::{Engine, PlayerAtClient, PlayerState};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn snapshot_with_player(player_id: i32, client_id: i32) -> SimulationSnapshot {
        let mut snapshot = Engine::new().snapshot();
        snapshot.players = vec![PlayerState {
            id: player_id,
            at_client: PlayerAtClient::new(client_id),
            status: PlayerStatus::Active,
            cursor: Some(clonk_engine::ObjectId::new(3)),
            ..PlayerState::default()
        }];
        snapshot
    }

    struct TestVoiceSource {
        frames: RefCell<Vec<VoiceInputFrame>>,
    }

    impl TestVoiceSource {
        fn with_frame(payload: EncodedVoiceFrame) -> Self {
            Self {
                frames: RefCell::new(vec![VoiceInputFrame {
                    payload,
                    level: 1.0,
                }]),
            }
        }

        fn with_levels(levels: &[f32]) -> Self {
            Self {
                frames: RefCell::new(
                    levels
                        .iter()
                        .map(|&level| VoiceInputFrame {
                            payload: [0; clonk_audio::VOICE_ENCODED_FRAME_BYTES],
                            level,
                        })
                        .collect(),
                ),
            }
        }
    }

    impl VoiceFrameSource for TestVoiceSource {
        fn drain_frames(&self) -> Vec<VoiceInputFrame> {
            std::mem::take(&mut *self.frames.borrow_mut())
        }
    }

    #[test]
    fn voice_activity_authenticates_ownership_rejects_replays_and_expires() {
        let snapshot = snapshot_with_player(17, 7);
        let start = Instant::now();
        let mut activity = VoiceActivityTracker::default();

        assert_eq!(
            activity.note_frame(&snapshot, 8, 17, 1, 0, start),
            VoiceFrameDisposition::OwnershipMismatch,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 1, 0, start),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 1, 0, start),
            VoiceFrameDisposition::DuplicateOrLate,
        );
        assert_eq!(activity.active_speakers(start), vec![(7, 17)]);
        assert_eq!(
            activity.active_speakers(start + SPEAKING_HANGOVER),
            Vec::<(i32, i32)>::new(),
        );
    }

    #[test]
    fn voice_activity_requires_an_active_player_with_a_selected_clonk() {
        let mut snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut activity = VoiceActivityTracker::default();

        snapshot.players[0].cursor = None;
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 1, 0, now),
            VoiceFrameDisposition::UnknownPlayer,
        );
        snapshot.players[0].cursor = Some(clonk_engine::ObjectId::new(3));
        snapshot.players[0].status = PlayerStatus::Eliminated;
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 1, 0, now),
            VoiceFrameDisposition::UnknownPlayer,
        );
    }

    #[test]
    fn voice_activity_accepts_sequence_wrap_and_a_new_stream_epoch() {
        let snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut activity = VoiceActivityTracker::default();

        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 4, u16::MAX, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 4, 0, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 5, 0, now),
            VoiceFrameDisposition::AcceptedNewEpoch,
        );
    }

    #[test]
    fn voice_activity_rejects_a_delayed_packet_from_an_old_stream_epoch() {
        let snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut activity = VoiceActivityTracker::default();

        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 5, 9, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 6, 0, now),
            VoiceFrameDisposition::AcceptedNewEpoch,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 5, 10, now),
            VoiceFrameDisposition::DuplicateOrLate,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, u32::MAX, 0, now),
            VoiceFrameDisposition::DuplicateOrLate,
        );

        let mut wrapping = VoiceActivityTracker::default();
        assert_eq!(
            wrapping.note_frame(&snapshot, 7, 17, u32::MAX, 0, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            wrapping.note_frame(&snapshot, 7, 17, 1, 0, now),
            VoiceFrameDisposition::AcceptedNewEpoch,
        );
    }

    #[test]
    fn expiring_voice_playback_preserves_the_replay_tombstone() {
        let snapshot = snapshot_with_player(17, 7);
        let start = Instant::now();
        let mut voice = VoiceChatState::default();

        assert_eq!(
            voice.note_remote_frame(&snapshot, 7, 17, 5, 9, start),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            voice.expire_playback(start + SPEAKING_HANGOVER),
            vec![(7, 17)],
        );
        assert!(voice.active_speakers(start + SPEAKING_HANGOVER).is_empty());
        assert_eq!(
            voice.note_remote_frame(&snapshot, 7, 17, 5, 9, start + SPEAKING_HANGOVER,),
            VoiceFrameDisposition::DuplicateOrLate,
        );
    }

    #[test]
    fn disconnect_and_clear_return_each_playback_stream_for_removal_once() {
        let mut snapshot = snapshot_with_player(17, 7);
        snapshot.players.push(PlayerState {
            id: 23,
            at_client: PlayerAtClient::new(8),
            status: PlayerStatus::Active,
            cursor: Some(clonk_engine::ObjectId::new(4)),
            ..PlayerState::default()
        });
        let now = Instant::now();
        let mut voice = VoiceChatState::default();
        assert_eq!(
            voice.note_remote_frame(&snapshot, 7, 17, 1, 0, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            voice.note_remote_frame(&snapshot, 8, 23, 1, 0, now),
            VoiceFrameDisposition::Accepted,
        );

        assert_eq!(voice.forget_client(7), vec![(7, 17)]);
        assert!(voice.forget_client(7).is_empty());
        assert_eq!(voice.clear(), vec![(8, 23)]);
        assert!(voice.clear().is_empty());
    }

    #[test]
    fn microphone_opens_only_for_an_explicit_capture_start() {
        let opens = Rc::new(Cell::new(0));
        let observed_opens = opens.clone();
        let mut voice = VoiceChatState::with_capture_opener(move || {
            observed_opens.set(observed_opens.get() + 1);
            Ok(TestVoiceSource::with_frame([0; 164]))
        });

        assert_eq!(opens.get(), 0);
        assert!(!voice.capture_active());

        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote))
            .unwrap();
        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote))
            .unwrap();
        assert_eq!(opens.get(), 1);
        assert_eq!(
            voice
                .drain_captured_frames(None)
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>(),
            vec![(1, 0)],
        );

        voice.stop_capture();
        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote))
            .unwrap();
        assert_eq!(opens.get(), 2);
        assert_eq!(
            voice
                .drain_captured_frames(None)
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>(),
            vec![(2, 0)],
        );
    }

    #[test]
    fn voice_activation_transmits_speech_with_a_release_tail_on_a_fresh_stream() {
        let levels = [0.1, 0.9, 0.1, 0.1, 0.1, 0.9];
        let mut voice =
            VoiceChatState::with_capture_opener(move || Ok(TestVoiceSource::with_levels(&levels)));
        let activation = VoiceActivation {
            threshold: 0.5,
            hangover_frames: 2,
        };

        voice.start_capture(None).unwrap();

        assert_eq!(
            voice
                .drain_captured_frames(Some(&activation))
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>(),
            vec![(1, 0), (1, 1), (1, 2), (2, 0)],
            "silence before speech is dropped, two frames of tail follow it, \
             and speaking again after the tail expires starts a new stream",
        );
    }

    #[test]
    fn push_to_talk_transmits_every_captured_frame_including_silence() {
        let levels = [0.0, 0.0];
        let mut voice =
            VoiceChatState::with_capture_opener(move || Ok(TestVoiceSource::with_levels(&levels)));

        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote))
            .unwrap();

        assert_eq!(
            voice
                .drain_captured_frames(None)
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>(),
            vec![(1, 0), (1, 1)],
            "a held key is the player's decision to transmit; the level is not consulted",
        );
    }

    #[test]
    fn a_zero_threshold_keeps_a_voice_activated_capture_permanently_open() {
        let levels = [0.0, 0.0];
        let mut voice =
            VoiceChatState::with_capture_opener(move || Ok(TestVoiceSource::with_levels(&levels)));
        let activation = VoiceActivation {
            threshold: 0.0,
            hangover_frames: 0,
        };

        voice.start_capture(None).unwrap();

        assert_eq!(voice.drain_captured_frames(Some(&activation)).len(), 2);
    }

    #[test]
    fn stopping_a_capture_closes_the_activation_gate_behind_it() {
        let opens = Cell::new(0);
        let mut voice = VoiceChatState::with_capture_opener(move || {
            opens.set(opens.get() + 1);
            // The reopened capture hears nothing but a quiet room.
            Ok(TestVoiceSource::with_levels(if opens.get() == 1 {
                &[0.9]
            } else {
                &[0.1]
            }))
        });
        let activation = VoiceActivation {
            threshold: 0.5,
            hangover_frames: 8,
        };

        voice.start_capture(None).unwrap();
        assert_eq!(voice.drain_captured_frames(Some(&activation)).len(), 1);
        voice.stop_capture();
        voice.start_capture(None).unwrap();

        assert!(
            voice.drain_captured_frames(Some(&activation)).is_empty(),
            "the reopened capture must not inherit the previous frame's release tail",
        );
    }

    #[test]
    fn malformed_remote_audio_cannot_advance_activity_or_show_a_speaker() {
        let snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut voice = VoiceChatState::default();
        let malformed = clonk_network::VoiceFrame {
            client_id: 7,
            player_id: 17,
            stream_epoch: 5,
            sequence: 9,
            payload: vec![0; 3],
        };

        assert!(voice
            .accept_remote_frame(&snapshot, &malformed, now)
            .is_none());
        assert!(voice.active_speakers(now).is_empty());

        let valid = clonk_network::VoiceFrame {
            payload: clonk_audio::encode_voice_frame(&[1_000; 320]).to_vec(),
            ..malformed
        };
        let accepted = voice
            .accept_remote_frame(&snapshot, &valid, now)
            .expect("the malformed frame did not consume its sequence");
        assert_eq!(accepted.stream_id, voice_stream_id(7, 17));
        assert!(!accepted.reset_stream);
        assert_eq!(accepted.samples, [1_000; 320]);
        assert_eq!(voice.active_speakers(now), vec![(7, 17)]);
    }

    #[test]
    fn push_to_talk_requires_opt_in_and_releases_the_key_that_opened_capture() {
        use winit::event::ElementState::{Pressed, Released};
        use winit::keyboard::KeyCode::{Backquote, KeyT};

        assert_eq!(
            push_to_talk_action(None, Backquote, false, true, false, Backquote, Pressed),
            PushToTalkAction::Ignore,
        );
        assert_eq!(
            push_to_talk_action(None, Backquote, true, false, false, Backquote, Pressed),
            PushToTalkAction::Consume,
        );
        assert_eq!(
            push_to_talk_action(None, Backquote, true, true, false, Backquote, Pressed),
            PushToTalkAction::Start,
        );
        assert_eq!(
            push_to_talk_action(
                Some(Backquote),
                KeyT,
                false,
                false,
                false,
                Backquote,
                Released,
            ),
            PushToTalkAction::Stop,
            "release still closes capture after settings change or disable",
        );
        assert_eq!(
            push_to_talk_action(None, Backquote, true, true, true, Backquote, Pressed),
            PushToTalkAction::Consume,
            "key repeat cannot retry a failed microphone open while held",
        );
    }

    #[test]
    fn selected_voice_crew_accepts_containment_without_reprojecting_world_position() {
        use clonk_engine::{Definition, PlayerConfig, SpawnConfig, Vector2};

        let mut engine = Engine::new();
        let mut crew_definition = Definition::from_script("CLNK", "Clonk", "#strict\n").unwrap();
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition).unwrap();
        engine
            .register_definition(Definition::from_script("CONT", "Container", "#strict\n").unwrap())
            .unwrap();
        engine
            .register_player(PlayerConfig::new(17, "Speaker"))
            .unwrap();
        engine
            .player_mut(17)
            .unwrap()
            .set_at_client(PlayerAtClient::new(7));
        let outer = engine
            .spawn_object(SpawnConfig::new("CONT").with_position(Vector2::new(700, 500)))
            .unwrap();
        let inner = engine
            .spawn_object(
                SpawnConfig::new("CONT")
                    .with_position(Vector2::new(600, 400))
                    .with_container(outer),
            )
            .unwrap();
        let crew = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(17)
                    .with_position(Vector2::new(500, 300))
                    .with_alive(true)
                    .with_container(inner),
            )
            .unwrap();
        engine.player_mut(17).unwrap().set_cursor(Some(crew));
        let mut snapshot = engine.snapshot();
        snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == crew)
            .unwrap()
            .position = Vector2::new(41, 37);
        snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == inner)
            .unwrap()
            .position = Vector2::new(600, 400);
        snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == outer)
            .unwrap()
            .position = Vector2::new(700, 500);

        assert_eq!(
            authenticated_selected_voice_crew(&snapshot, 7, 17).map(|object| object.position),
            Some(Vector2::new(41, 37)),
            "the selected crew's native world position remains authoritative while contained",
        );
        assert!(authenticated_selected_voice_crew(&snapshot, 8, 17).is_none());
        snapshot.players[0].status = PlayerStatus::Eliminated;
        assert!(authenticated_selected_voice_crew(&snapshot, 7, 17).is_none());
    }
}
