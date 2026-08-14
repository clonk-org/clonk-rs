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
//! glyph additionally obeys per-viewport object/FoW visibility. Several
//! speakers at once would otherwise sum straight into the output clamp, so the
//! audio mixer limits the summed voice bus to its own ceiling — voice is the
//! one source it may attenuate, because the sound and music paths owe
//! SDL_mixer's arithmetic. Landscape
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
const VOICE_FRAME_DURATION: Duration = Duration::from_millis(20);
const INITIAL_VOICE_JITTER_FRAMES: usize = 4;
const MIN_VOICE_JITTER_FRAMES: usize = 2;
const MAX_VOICE_JITTER_FRAMES: usize = 6;
const MAX_PENDING_VOICE_FRAMES: usize = 8;
const VOICE_SEQUENCE_WINDOW_FRAMES: usize = u64::BITS as usize;
const MIN_VOICE_JITTER_OBSERVATIONS: usize = 3;
const VOICE_PLAYOUT_GUARD_FRAMES: usize = 2;

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
    seen_sequences: u64,
    playout_floor: Option<u16>,
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
    pub(crate) sequence: u16,
    pub(crate) samples: [i16; clonk_audio::VOICE_FRAME_SAMPLES],
    pub(crate) concealed: bool,
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
    jitter: RemoteVoiceJitterBuffer,
}

#[derive(Debug)]
struct BufferedRemoteVoiceFrame {
    sequence: u16,
    samples: [i16; clonk_audio::VOICE_FRAME_SAMPLES],
}

#[derive(Debug)]
struct RemoteVoicePlayoutFrame {
    sequence: u16,
    samples: [i16; clonk_audio::VOICE_FRAME_SAMPLES],
    concealed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RemoteVoicePlayoutStats {
    pub(crate) target_frames: usize,
    pub(crate) reordered_frames: u64,
    pub(crate) concealed_frames: u64,
}

#[derive(Debug)]
struct RemoteVoiceJitterBuffer {
    pending: Vec<BufferedRemoteVoiceFrame>,
    next_playout_sequence: Option<u16>,
    first_arrival_at: Option<Instant>,
    arrival_origin: Option<(u16, Instant)>,
    transit_bounds_ns: Option<(i128, i128)>,
    arrival_observations: usize,
    target_frames: usize,
    started: bool,
    previous_output: Option<[i16; clonk_audio::VOICE_FRAME_SAMPLES]>,
    highest_arrival_sequence: Option<u16>,
    reordered_frames: u64,
    concealed_frames: u64,
}

impl Default for RemoteVoiceJitterBuffer {
    fn default() -> Self {
        Self {
            pending: Vec::with_capacity(MAX_PENDING_VOICE_FRAMES),
            next_playout_sequence: None,
            first_arrival_at: None,
            arrival_origin: None,
            transit_bounds_ns: None,
            arrival_observations: 0,
            target_frames: INITIAL_VOICE_JITTER_FRAMES,
            started: false,
            previous_output: None,
            highest_arrival_sequence: None,
            reordered_frames: 0,
            concealed_frames: 0,
        }
    }
}

impl RemoteVoiceJitterBuffer {
    fn can_insert(&self, sequence: u16) -> bool {
        if self
            .pending
            .iter()
            .any(|pending| pending.sequence == sequence)
        {
            return false;
        }
        let Some(anchor) = self.insertion_anchor(sequence) else {
            return false;
        };
        let incoming_offset = sequence.wrapping_sub(anchor);
        let mut retained_count = 0;
        let mut farthest_offset = None;
        for offset in self
            .pending
            .iter()
            .map(|pending| pending.sequence.wrapping_sub(anchor))
            .filter(|&offset| usize::from(offset) < VOICE_SEQUENCE_WINDOW_FRAMES)
        {
            retained_count += 1;
            farthest_offset = farthest_offset.max(Some(offset));
        }
        if retained_count < MAX_PENDING_VOICE_FRAMES {
            return true;
        }
        farthest_offset.is_some_and(|farthest_offset| incoming_offset < farthest_offset)
    }

    fn insertion_anchor(&self, sequence: u16) -> Option<u16> {
        let Some(next_playout_sequence) = self.next_playout_sequence else {
            return Some(sequence);
        };
        let forward = sequence.wrapping_sub(next_playout_sequence);
        if usize::from(forward) < VOICE_SEQUENCE_WINDOW_FRAMES {
            return Some(next_playout_sequence);
        }
        let rewind = next_playout_sequence.wrapping_sub(sequence);
        (!self.started && usize::from(rewind) < VOICE_SEQUENCE_WINDOW_FRAMES).then_some(sequence)
    }

    fn insert(
        &mut self,
        sequence: u16,
        received_at: Instant,
        samples: [i16; clonk_audio::VOICE_FRAME_SAMPLES],
    ) -> bool {
        if !self.can_insert(sequence) {
            return false;
        }
        let insertion_anchor = self.insertion_anchor(sequence).unwrap_or(sequence);
        self.pending.retain(|pending| {
            usize::from(pending.sequence.wrapping_sub(insertion_anchor))
                < VOICE_SEQUENCE_WINDOW_FRAMES
        });
        if self.pending.len() >= MAX_PENDING_VOICE_FRAMES {
            let Some((farthest_position, _)) = self
                .pending
                .iter()
                .enumerate()
                .max_by_key(|(_, pending)| pending.sequence.wrapping_sub(insertion_anchor))
            else {
                return false;
            };
            self.pending.remove(farthest_position);
        }
        match self.highest_arrival_sequence.as_mut() {
            Some(highest) => {
                let advance = sequence.wrapping_sub(*highest);
                if advance <= u16::MAX / 2 {
                    *highest = sequence;
                } else {
                    self.reordered_frames = self.reordered_frames.saturating_add(1);
                }
            }
            None => self.highest_arrival_sequence = Some(sequence),
        }
        let mut next_playout_sequence = *self.next_playout_sequence.get_or_insert(sequence);
        self.first_arrival_at.get_or_insert(received_at);
        let offset = sequence.wrapping_sub(next_playout_sequence);
        if offset > u16::MAX / 2 {
            let rewind = next_playout_sequence.wrapping_sub(sequence);
            if self.started || usize::from(rewind) >= VOICE_SEQUENCE_WINDOW_FRAMES {
                return false;
            }
            next_playout_sequence = sequence;
            self.next_playout_sequence = Some(sequence);
        }
        self.pending
            .push(BufferedRemoteVoiceFrame { sequence, samples });
        self.pending
            .sort_unstable_by_key(|pending| pending.sequence.wrapping_sub(next_playout_sequence));
        self.observe_arrival(sequence, received_at);
        true
    }

    fn observe_arrival(&mut self, sequence: u16, received_at: Instant) {
        let (origin_sequence, origin_at) =
            *self.arrival_origin.get_or_insert((sequence, received_at));
        let arrival_offset_ns = received_at
            .checked_duration_since(origin_at)
            .map(duration_nanos)
            .unwrap_or_else(|| -duration_nanos(origin_at.duration_since(received_at)));
        let sequence_offset = i128::from(sequence.wrapping_sub(origin_sequence) as i16);
        let residual_ns = arrival_offset_ns
            .saturating_sub(sequence_offset.saturating_mul(duration_nanos(VOICE_FRAME_DURATION)));
        match self.transit_bounds_ns.as_mut() {
            Some((minimum, maximum)) => {
                *minimum = (*minimum).min(residual_ns);
                *maximum = (*maximum).max(residual_ns);
            }
            None => self.transit_bounds_ns = Some((residual_ns, residual_ns)),
        }
        self.arrival_observations = self.arrival_observations.saturating_add(1);
        // Resize only the startup prebuffer. Growing a live delay without
        // time-stretching would create the very gap this buffer prevents.
        if self.started || self.arrival_observations < MIN_VOICE_JITTER_OBSERVATIONS {
            return;
        }
        let (minimum, maximum) = self
            .transit_bounds_ns
            .expect("an observed voice arrival has transit bounds");
        let frame_ns = duration_nanos(VOICE_FRAME_DURATION);
        let spread_ns = maximum.saturating_sub(minimum);
        let spread_frames = spread_ns.saturating_add(frame_ns - 1) / frame_ns;
        self.target_frames = usize::try_from(spread_frames.saturating_add(1))
            .unwrap_or(MAX_VOICE_JITTER_FRAMES)
            .clamp(MIN_VOICE_JITTER_FRAMES, MAX_VOICE_JITTER_FRAMES);
    }

    #[cfg(test)]
    fn target_frames(&self) -> usize {
        self.target_frames
    }

    fn drain_ready(&mut self, now: Instant, max_frames: usize) -> Vec<RemoteVoicePlayoutFrame> {
        self.drain_ready_with_headroom(now, max_frames, 0)
    }

    fn drain_ready_with_headroom(
        &mut self,
        now: Instant,
        max_frames: usize,
        buffered_playout_frames: usize,
    ) -> Vec<RemoteVoicePlayoutFrame> {
        let Some(mut next_sequence) = self.next_playout_sequence else {
            return Vec::new();
        };
        if !self.started {
            let contiguous = self
                .pending
                .iter()
                .take_while(|frame| {
                    let matches = frame.sequence == next_sequence;
                    next_sequence = next_sequence.wrapping_add(u16::from(matches));
                    matches
                })
                .count();
            let prebuffer_elapsed = self.first_arrival_at.is_some_and(|first_arrival_at| {
                now.saturating_duration_since(first_arrival_at)
                    >= VOICE_FRAME_DURATION.saturating_mul(self.target_frames as u32)
            });
            if contiguous < self.target_frames && !prebuffer_elapsed {
                return Vec::new();
            }
            self.started = true;
        }

        let mut ready = Vec::with_capacity(max_frames.min(self.pending.len()));
        while ready.len() < max_frames {
            let expected = self
                .next_playout_sequence
                .expect("a started voice jitter buffer has a playout sequence");
            let Some(position) = self
                .pending
                .iter()
                .position(|pending| pending.sequence == expected)
            else {
                let successor = self
                    .pending
                    .iter()
                    .min_by_key(|pending| pending.sequence.wrapping_sub(expected))
                    .map(|pending| (pending.sequence, pending.samples[0]));
                let Some((successor_sequence, successor_first)) = successor else {
                    break;
                };
                let Some(previous_output) = self.previous_output else {
                    break;
                };
                let successor_distance = successor_sequence.wrapping_sub(expected);
                if successor_distance == 0 || successor_distance > u16::MAX / 2 {
                    break;
                }
                let buffered_headroom = buffered_playout_frames.saturating_add(ready.len());
                if buffered_headroom > VOICE_PLAYOUT_GUARD_FRAMES {
                    break;
                }
                let samples = concealed_voice_frame(&previous_output, successor_first);
                self.previous_output = Some(samples);
                ready.push(RemoteVoicePlayoutFrame {
                    sequence: expected,
                    samples,
                    concealed: true,
                });
                self.concealed_frames = self.concealed_frames.saturating_add(1);
                self.next_playout_sequence = Some(if successor_distance == 1 {
                    expected.wrapping_add(1)
                } else {
                    successor_sequence
                });
                continue;
            };
            let frame = self.pending.remove(position);
            self.previous_output = Some(frame.samples);
            ready.push(RemoteVoicePlayoutFrame {
                sequence: frame.sequence,
                samples: frame.samples,
                concealed: false,
            });
            self.next_playout_sequence = Some(expected.wrapping_add(1));
        }
        ready
    }

    fn stats(&self) -> RemoteVoicePlayoutStats {
        RemoteVoicePlayoutStats {
            target_frames: self.target_frames,
            reordered_frames: self.reordered_frames,
            concealed_frames: self.concealed_frames,
        }
    }
}

fn duration_nanos(duration: Duration) -> i128 {
    i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX)
}

fn concealed_voice_frame(
    previous_frame: &[i16; clonk_audio::VOICE_FRAME_SAMPLES],
    next_sample: i16,
) -> [i16; clonk_audio::VOICE_FRAME_SAMPLES] {
    let previous_first = i64::from(previous_frame[0]);
    let previous_last = i64::from(previous_frame[clonk_audio::VOICE_FRAME_SAMPLES - 1]);
    let bridge_difference = i64::from(next_sample).saturating_sub(previous_last);
    let bridge_denominator =
        i64::try_from(clonk_audio::VOICE_FRAME_SAMPLES + 1).unwrap_or(i64::MAX);
    let texture_denominator =
        i64::try_from(clonk_audio::VOICE_FRAME_SAMPLES - 1).unwrap_or(i64::MAX);
    let previous_difference = previous_last.saturating_sub(previous_first);
    std::array::from_fn(|index| {
        let texture_numerator = i64::try_from(index).unwrap_or(i64::MAX);
        let previous_baseline = previous_first.saturating_add(
            previous_difference.saturating_mul(texture_numerator) / texture_denominator,
        );
        let texture = i64::from(previous_frame[index]).saturating_sub(previous_baseline);
        let bridge_numerator = i64::try_from(index + 1).unwrap_or(i64::MAX);
        previous_last
            .saturating_add(bridge_difference.saturating_mul(bridge_numerator) / bridge_denominator)
            .saturating_add(texture)
            .clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
    })
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
    #[cfg(not(test))]
    fn default() -> Self {
        Self::with_source_opener(VoiceCapture::open)
    }

    /// Under test the default state can never reach a real device. A test that
    /// forgets to inject a source would otherwise open the *developer's*
    /// microphone, pass locally, and then fail on a CI runner that has no input
    /// device — which is exactly how this arrived. Tests that exercise capture
    /// pass their own source to [`VoiceChatState::with_source_opener`].
    #[cfg(test)]
    fn default() -> Self {
        Self::with_source_opener(|| Err::<VoiceCapture, _>(VoiceCaptureError::Unavailable))
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
            let stream = self
                .remote_streams
                .entry((client_id, player_id))
                .or_default();
            if disposition == VoiceFrameDisposition::AcceptedNewEpoch {
                stream.jitter = RemoteVoiceJitterBuffer::default();
            }
            stream.stream_epoch = stream_epoch;
            stream.last_frame_at = Some(received_at);
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
        if self
            .remote_streams
            .get(&(client_id, frame.player_id))
            .is_some_and(|stream| {
                stream.stream_epoch == frame.stream_epoch
                    && !stream.jitter.can_insert(frame.sequence)
            })
        {
            return None;
        }
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
                let inserted = self
                    .remote_streams
                    .get_mut(&(client_id, frame.player_id))
                    .is_some_and(|stream| {
                        stream.jitter.insert(frame.sequence, received_at, samples)
                    });
                if !inserted {
                    return None;
                }
                Some(AcceptedRemoteVoiceFrame {
                    stream_id: voice_stream_id(client_id, frame.player_id),
                    sequence: frame.sequence,
                    samples,
                    concealed: false,
                    reset_stream: disposition == VoiceFrameDisposition::AcceptedNewEpoch,
                })
            }
            VoiceFrameDisposition::UnknownPlayer
            | VoiceFrameDisposition::OwnershipMismatch
            | VoiceFrameDisposition::DuplicateOrLate => None,
        }
    }

    pub(crate) fn drain_remote_playout(
        &mut self,
        client_id: i32,
        player_id: i32,
        now: Instant,
        max_frames: usize,
        buffered_playout_frames: usize,
    ) -> Vec<AcceptedRemoteVoiceFrame> {
        let Some(stream) = self.remote_streams.get_mut(&(client_id, player_id)) else {
            return Vec::new();
        };
        let stream_epoch = stream.stream_epoch;
        let frames =
            stream
                .jitter
                .drain_ready_with_headroom(now, max_frames, buffered_playout_frames);
        if !frames.is_empty() {
            stream.last_frame_at = Some(now);
        }
        let playout_floor = if frames.is_empty() {
            None
        } else {
            stream.jitter.next_playout_sequence
        };
        if let Some(playout_floor) = playout_floor {
            if let Some(activity) = self
                .activity
                .speakers
                .get_mut(&(client_id, player_id))
                .filter(|activity| activity.stream_epoch == stream_epoch)
            {
                activity.playout_floor = Some(playout_floor);
            }
        }
        frames
            .into_iter()
            .map(|frame| AcceptedRemoteVoiceFrame {
                stream_id: voice_stream_id(client_id, player_id),
                sequence: frame.sequence,
                samples: frame.samples,
                concealed: frame.concealed,
                reset_stream: false,
            })
            .collect()
    }

    pub(crate) fn remote_playout_stats(
        &self,
        client_id: i32,
        player_id: i32,
    ) -> RemoteVoicePlayoutStats {
        self.remote_streams
            .get(&(client_id, player_id))
            .map(|stream| stream.jitter.stats())
            .unwrap_or_default()
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
        for speaker in &expired {
            self.advance_replay_floor_past_accepted(*speaker);
        }
        expired
    }

    pub(crate) fn discard_remote_playback(&mut self, client_id: i32, player_id: i32) -> bool {
        let speaker = (client_id, player_id);
        let removed = self.remote_streams.remove(&speaker).is_some();
        if removed {
            self.advance_replay_floor_past_accepted(speaker);
        }
        removed
    }

    fn advance_replay_floor_past_accepted(&mut self, speaker: (i32, i32)) {
        if let Some(activity) = self.activity.speakers.get_mut(&speaker) {
            activity.playout_floor = Some(activity.latest_sequence.wrapping_add(1));
        }
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
                if activity.playout_floor.is_some_and(|playout_floor| {
                    sequence.wrapping_sub(playout_floor) > u16::MAX / 2
                }) {
                    return VoiceFrameDisposition::DuplicateOrLate;
                }
                let advance = sequence.wrapping_sub(activity.latest_sequence);
                if advance == 0 {
                    return VoiceFrameDisposition::DuplicateOrLate;
                }
                if advance <= u16::MAX / 2 {
                    activity.seen_sequences = if u32::from(advance) >= u64::BITS {
                        1
                    } else {
                        (activity.seen_sequences << advance) | 1
                    };
                    activity.latest_sequence = sequence;
                } else {
                    let rewind = activity.latest_sequence.wrapping_sub(sequence);
                    if u32::from(rewind) >= u64::BITS {
                        return VoiceFrameDisposition::DuplicateOrLate;
                    }
                    let seen_bit = 1_u64 << rewind;
                    if activity.seen_sequences & seen_bit != 0 {
                        return VoiceFrameDisposition::DuplicateOrLate;
                    }
                    activity.seen_sequences |= seen_bit;
                }
            } else {
                let advance = stream_epoch.wrapping_sub(activity.stream_epoch);
                if advance == 0 || advance > u32::MAX / 2 {
                    return VoiceFrameDisposition::DuplicateOrLate;
                }
                disposition = VoiceFrameDisposition::AcceptedNewEpoch;
                activity.latest_sequence = sequence;
                activity.seen_sequences = 1;
                activity.playout_floor = None;
            }
            activity.stream_epoch = stream_epoch;
            activity.last_frame_at = received_at;
        } else {
            self.speakers.insert(
                key,
                SpeakerActivity {
                    stream_epoch,
                    latest_sequence: sequence,
                    seen_sequences: 1,
                    playout_floor: None,
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

    fn ramp_voice_frame(sequence: u16) -> [i16; clonk_audio::VOICE_FRAME_SAMPLES] {
        std::array::from_fn(|sample| {
            let absolute_sample = usize::from(sequence) * clonk_audio::VOICE_FRAME_SAMPLES + sample;
            4_000 + i16::try_from(absolute_sample * 8).unwrap_or(i16::MAX)
        })
    }

    fn bipolar_voice_frame() -> [i16; clonk_audio::VOICE_FRAME_SAMPLES] {
        std::array::from_fn(|sample| match sample {
            0..=79 => i16::try_from(sample * 100).unwrap_or(i16::MAX),
            80..=159 => i16::try_from((159 - sample) * 100).unwrap_or(i16::MAX),
            160..=239 => -i16::try_from((sample - 160) * 100).unwrap_or(i16::MAX),
            _ => -i16::try_from((319 - sample) * 100).unwrap_or(i16::MAX),
        })
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
    fn remote_voice_accepts_one_out_of_order_frame_within_the_playout_window_once() {
        let snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut activity = VoiceActivityTracker::default();

        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 4, 40, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 4, 42, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 4, 41, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 4, 41, now),
            VoiceFrameDisposition::DuplicateOrLate,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 4, 43, now),
            VoiceFrameDisposition::Accepted,
        );
    }

    #[test]
    fn remote_voice_jitter_buffer_reorders_frames_before_playout() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();

        assert!(jitter.insert(40, start, [40; clonk_audio::VOICE_FRAME_SAMPLES]));
        assert!(jitter.insert(
            42,
            start + Duration::from_millis(20),
            [42; clonk_audio::VOICE_FRAME_SAMPLES],
        ));
        assert!(jitter.insert(
            41,
            start + Duration::from_millis(35),
            [41; clonk_audio::VOICE_FRAME_SAMPLES],
        ));
        assert!(jitter.insert(
            43,
            start + Duration::from_millis(60),
            [43; clonk_audio::VOICE_FRAME_SAMPLES],
        ));

        let ready = jitter.drain_ready(start + Duration::from_millis(60), usize::MAX);
        assert_eq!(
            ready.iter().map(|frame| frame.sequence).collect::<Vec<_>>(),
            vec![40, 41, 42, 43],
        );
        assert!(ready.iter().all(|frame| !frame.concealed));
    }

    #[test]
    fn remote_voice_jitter_buffer_rewinds_an_unstarted_first_arrival() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();

        for (sequence, arrival_ms) in [(2, 0), (0, 10), (1, 20), (3, 30)] {
            assert!(jitter.insert(
                sequence,
                start + Duration::from_millis(arrival_ms),
                [sequence as i16; clonk_audio::VOICE_FRAME_SAMPLES],
            ));
        }

        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(30), usize::MAX)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
        );
    }

    #[test]
    fn remote_voice_jitter_buffer_rewinds_within_the_replay_window() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();

        assert!(jitter.insert(13, start, bipolar_voice_frame()));
        assert!(jitter.insert(0, start + Duration::from_millis(10), bipolar_voice_frame(),));
    }

    #[test]
    fn remote_voice_jitter_buffer_orders_sequence_wrap() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for (sequence, arrival_ms) in [(u16::MAX - 1, 0), (0, 20), (u16::MAX, 35), (1, 60)] {
            assert!(jitter.insert(
                sequence,
                start + Duration::from_millis(arrival_ms),
                bipolar_voice_frame(),
            ));
        }

        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(60), usize::MAX)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![u16::MAX - 1, u16::MAX, 0, 1],
        );
    }

    #[test]
    fn full_voice_jitter_buffer_keeps_a_late_frame_closer_to_playout() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in [0, 2, 3, 4, 5, 6, 7, 8] {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                ramp_voice_frame(sequence),
            ));
        }

        assert!(jitter.insert(1, start + Duration::from_millis(170), ramp_voice_frame(1),));
        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(170), usize::MAX)
                .into_iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            (0..8).map(|sequence| (sequence, false)).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn remote_voice_jitter_buffer_sizes_prebuffer_from_observed_arrival_spread() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();

        assert!(jitter.insert(0, start, [0; clonk_audio::VOICE_FRAME_SAMPLES]));
        assert!(jitter.insert(
            1,
            start + VOICE_FRAME_DURATION,
            [1; clonk_audio::VOICE_FRAME_SAMPLES],
        ));
        assert!(jitter.insert(
            2,
            start + VOICE_FRAME_DURATION.saturating_mul(2),
            [2; clonk_audio::VOICE_FRAME_SAMPLES],
        ));
        assert_eq!(jitter.target_frames(), 2);

        assert!(jitter.insert(
            3,
            start + VOICE_FRAME_DURATION.saturating_mul(6),
            [3; clonk_audio::VOICE_FRAME_SAMPLES],
        ));
        assert_eq!(jitter.target_frames(), 4);
    }

    #[test]
    fn remote_voice_jitter_buffer_freezes_delay_after_playout_starts() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in 0..3 {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                ramp_voice_frame(sequence),
            ));
        }
        assert_eq!(jitter.target_frames(), 2);
        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(40), usize::MAX)
                .len(),
            3,
        );

        assert!(jitter.insert(3, start + Duration::from_millis(120), ramp_voice_frame(3),));
        assert_eq!(
            jitter.target_frames(),
            2,
            "growing live playout delay would itself introduce an audio gap",
        );
    }

    #[test]
    fn remote_voice_jitter_buffer_conceals_one_lost_frame_without_a_gap_or_click() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for (sequence, arrival_ms) in [(0, 0), (2, 21), (1, 52), (3, 65), (5, 100)] {
            assert!(jitter.insert(
                sequence,
                start + Duration::from_millis(arrival_ms),
                ramp_voice_frame(sequence),
            ));
        }

        let mut ready = jitter.drain_ready(start + Duration::from_millis(100), usize::MAX);
        ready.extend(jitter.drain_ready(start + Duration::from_millis(180), usize::MAX));

        assert_eq!(
            ready
                .iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            vec![
                (0, false),
                (1, false),
                (2, false),
                (3, false),
                (4, true),
                (5, false)
            ],
        );
        let played_samples = ready
            .iter()
            .flat_map(|frame| frame.samples)
            .collect::<Vec<_>>();
        let expected_samples = (0..6).flat_map(ramp_voice_frame).collect::<Vec<_>>();
        assert_eq!(played_samples, expected_samples);
        assert!(played_samples.windows(2).all(|pair| pair[1] - pair[0] == 8));
    }

    #[test]
    fn concealed_voice_preserves_energy_across_zero_crossing_boundaries() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in [0, 1, 2, 3, 5] {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                bipolar_voice_frame(),
            ));
        }

        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(100), usize::MAX)
                .len(),
            4,
        );
        let ready = jitter.drain_ready_with_headroom(
            start + Duration::from_millis(100),
            usize::MAX,
            VOICE_PLAYOUT_GUARD_FRAMES,
        );
        assert_eq!(
            ready
                .iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            vec![(4, true), (5, false)],
        );
        let concealed = &ready[0].samples;
        assert!(
            concealed
                .iter()
                .map(|sample| i64::from(sample.abs()))
                .sum::<i64>()
                > 500_000
        );
        assert_eq!(concealed[0], 0);
        assert_eq!(concealed[clonk_audio::VOICE_FRAME_SAMPLES - 1], 0);
        assert!(concealed
            .windows(2)
            .all(|pair| (pair[1] - pair[0]).abs() <= 100));
    }

    #[test]
    fn isolated_voice_loss_is_concealed_before_buffered_playout_runs_dry() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for (sequence, arrival_ms) in [(0, 0), (1, 20), (2, 40), (3, 60), (5, 80)] {
            assert!(jitter.insert(
                sequence,
                start + Duration::from_millis(arrival_ms),
                ramp_voice_frame(sequence),
            ));
        }

        assert_eq!(
            jitter
                .drain_ready_with_headroom(start + Duration::from_millis(80), usize::MAX, 0)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
        );
        assert_eq!(
            jitter
                .drain_ready_with_headroom(start + Duration::from_millis(100), usize::MAX, 2)
                .into_iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            vec![(4, true), (5, false)],
        );
    }

    #[test]
    fn consecutive_voice_loss_reanchors_after_one_concealed_frame() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in [0, 1, 2, 3, 6] {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                ramp_voice_frame(sequence),
            ));
        }

        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(120), usize::MAX)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
        );
        assert_eq!(
            jitter
                .drain_ready_with_headroom(
                    start + Duration::from_millis(120),
                    usize::MAX,
                    VOICE_PLAYOUT_GUARD_FRAMES,
                )
                .into_iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            vec![(4, true), (6, false)],
        );
        assert!(jitter.insert(7, start + Duration::from_millis(140), ramp_voice_frame(7),));
        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(140), usize::MAX)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![7],
        );
    }

    #[test]
    fn large_bounded_voice_sequence_jump_reanchors_without_wedging_playout() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in [0, 1, 2, 3, 13] {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                bipolar_voice_frame(),
            ));
        }

        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(260), usize::MAX)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
        );
        assert_eq!(
            jitter
                .drain_ready_with_headroom(
                    start + Duration::from_millis(260),
                    usize::MAX,
                    VOICE_PLAYOUT_GUARD_FRAMES,
                )
                .into_iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            vec![(4, true), (13, false)],
        );
        assert!(jitter.insert(
            14,
            start + Duration::from_millis(280),
            bipolar_voice_frame(),
        ));
        assert!(!jitter.insert(5, start + Duration::from_millis(280), bipolar_voice_frame(),));
        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(280), usize::MAX)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![14],
        );
    }

    #[test]
    fn paused_voice_playout_keeps_reordering_while_buffered_headroom_remains() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in [0, 1, 2, 3, 5] {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                ramp_voice_frame(sequence),
            ));
        }

        assert_eq!(
            jitter
                .drain_ready_with_headroom(start + Duration::from_millis(100), usize::MAX, 0)
                .len(),
            4,
        );
        assert!(jitter
            .drain_ready_with_headroom(start + Duration::from_secs(1), usize::MAX, 4)
            .is_empty());
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
    fn expired_voice_playback_rejects_an_unseen_frame_behind_the_playout_floor() {
        let snapshot = snapshot_with_player(17, 7);
        let start = Instant::now();
        let mut voice = VoiceChatState::default();
        let frame = |sequence| clonk_network::VoiceFrame {
            client_id: 7,
            player_id: 17,
            stream_epoch: 5,
            sequence,
            payload: clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES])
                .to_vec(),
        };

        assert!(voice
            .accept_remote_frame(&snapshot, &frame(100), start)
            .is_some());
        let playout_at = start + Duration::from_millis(80);
        assert_eq!(
            voice
                .drain_remote_playout(7, 17, playout_at, usize::MAX, 0)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![100],
        );
        let expires_at = playout_at + SPEAKING_HANGOVER;
        assert_eq!(voice.expire_playback(expires_at), vec![(7, 17)],);

        assert!(voice
            .accept_remote_frame(&snapshot, &frame(99), expires_at)
            .is_none());
        assert!(voice.active_speakers(expires_at).is_empty());
        assert!(voice
            .accept_remote_frame(&snapshot, &frame(101), expires_at)
            .is_some());
    }

    #[test]
    fn expiring_unplayed_voice_advances_the_replay_floor_past_discarded_packets() {
        let snapshot = snapshot_with_player(17, 7);
        let start = Instant::now();
        let mut voice = VoiceChatState::default();
        let frame = |sequence| clonk_network::VoiceFrame {
            client_id: 7,
            player_id: 17,
            stream_epoch: 5,
            sequence,
            payload: clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES])
                .to_vec(),
        };

        assert!(voice
            .accept_remote_frame(&snapshot, &frame(100), start)
            .is_some());
        assert_eq!(
            voice.expire_playback(start + SPEAKING_HANGOVER),
            vec![(7, 17)],
        );
        assert!(voice
            .accept_remote_frame(&snapshot, &frame(99), start + SPEAKING_HANGOVER,)
            .is_none());
        assert!(voice
            .accept_remote_frame(&snapshot, &frame(101), start + SPEAKING_HANGOVER,)
            .is_some());
    }

    #[test]
    fn buffered_voice_playback_expires_after_the_last_drain_not_the_last_packet() {
        let snapshot = snapshot_with_player(17, 7);
        let start = Instant::now();
        let mut voice = VoiceChatState::default();
        let frame = |sequence| clonk_network::VoiceFrame {
            client_id: 7,
            player_id: 17,
            stream_epoch: 5,
            sequence,
            payload: clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES])
                .to_vec(),
        };

        for sequence in 0..5 {
            assert!(voice
                .accept_remote_frame(&snapshot, &frame(sequence), start)
                .is_some());
        }
        assert_eq!(
            voice
                .drain_remote_playout(7, 17, start, 5, 0)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            (0..5).collect::<Vec<_>>(),
        );
        for sequence in 5..13 {
            assert!(voice
                .accept_remote_frame(&snapshot, &frame(sequence), start)
                .is_some());
        }

        assert_eq!(
            voice
                .drain_remote_playout(7, 17, start + Duration::from_millis(240), 3, 2)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![5, 6, 7],
        );
        assert!(voice.active_speakers(start + SPEAKING_HANGOVER).is_empty());
        assert!(voice.expire_playback(start + SPEAKING_HANGOVER).is_empty());
        assert_eq!(
            voice
                .drain_remote_playout(7, 17, start + Duration::from_millis(300), 3, 2)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![8, 9, 10],
        );
        assert_eq!(
            voice
                .drain_remote_playout(7, 17, start + Duration::from_millis(360), 3, 2)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![11, 12],
        );
        assert!(voice
            .expire_playback(
                start + Duration::from_millis(360) + SPEAKING_HANGOVER - Duration::from_millis(1),
            )
            .is_empty());
        assert_eq!(
            voice.expire_playback(start + Duration::from_millis(360) + SPEAKING_HANGOVER),
            vec![(7, 17)],
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
    fn discarding_remote_playback_keeps_the_replay_tombstone() {
        let snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut voice = VoiceChatState::default();
        assert_eq!(
            voice.note_remote_frame(&snapshot, 7, 17, 4, 9, now),
            VoiceFrameDisposition::Accepted,
        );

        assert!(voice.discard_remote_playback(7, 17));
        assert!(!voice.remote_streams.contains_key(&(7, 17)));
        assert_eq!(
            voice.note_remote_frame(&snapshot, 7, 17, 4, 9, now),
            VoiceFrameDisposition::DuplicateOrLate,
        );
        assert_eq!(
            voice.note_remote_frame(&snapshot, 7, 17, 4, 8, now),
            VoiceFrameDisposition::DuplicateOrLate,
        );
        assert_eq!(
            voice.note_remote_frame(&snapshot, 7, 17, 4, 10, now),
            VoiceFrameDisposition::Accepted,
        );
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
    fn remote_voice_rejects_a_sequence_outside_the_replay_window_without_refreshing_activity() {
        let snapshot = snapshot_with_player(17, 7);
        let start = Instant::now();
        let mut voice = VoiceChatState::default();
        let frame = |sequence| clonk_network::VoiceFrame {
            client_id: 7,
            player_id: 17,
            stream_epoch: 5,
            sequence,
            payload: clonk_audio::encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES])
                .to_vec(),
        };

        assert!(voice
            .accept_remote_frame(&snapshot, &frame(0), start)
            .is_some());
        assert!(voice
            .accept_remote_frame(
                &snapshot,
                &frame(VOICE_SEQUENCE_WINDOW_FRAMES as u16),
                start + SPEAKING_HANGOVER - Duration::from_millis(1),
            )
            .is_none());
        assert!(voice.active_speakers(start + SPEAKING_HANGOVER).is_empty());
        assert!(voice
            .accept_remote_frame(&snapshot, &frame(1), start + SPEAKING_HANGOVER,)
            .is_some());
    }

    #[test]
    fn accepted_remote_voice_drains_in_sequence_order_after_poll_batching() {
        let snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut voice = VoiceChatState::default();

        for sequence in [0, 2, 1, 3] {
            let frame = clonk_network::VoiceFrame {
                client_id: 7,
                player_id: 17,
                stream_epoch: 5,
                sequence,
                payload: clonk_audio::encode_voice_frame(&[sequence as i16; 320]).to_vec(),
            };
            assert!(voice.accept_remote_frame(&snapshot, &frame, now).is_some());
        }

        assert_eq!(
            voice
                .drain_remote_playout(7, 17, now, clonk_audio::DEFAULT_VOICE_BUFFERED_FRAMES, 0,)
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
        );
    }

    #[test]
    fn remote_voice_rejects_a_missing_frame_after_playout_concealed_it() {
        let snapshot = snapshot_with_player(17, 7);
        let start = Instant::now();
        let mut voice = VoiceChatState::default();

        for sequence in [0, 1, 2, 3, 5] {
            let frame = clonk_network::VoiceFrame {
                client_id: 7,
                player_id: 17,
                stream_epoch: 5,
                sequence,
                payload: clonk_audio::encode_voice_frame(&ramp_voice_frame(sequence)).to_vec(),
            };
            assert!(voice
                .accept_remote_frame(&snapshot, &frame, start)
                .is_some());
        }
        let mut drained = voice.drain_remote_playout(
            7,
            17,
            start + Duration::from_millis(120),
            clonk_audio::DEFAULT_VOICE_BUFFERED_FRAMES,
            0,
        );
        drained.extend(voice.drain_remote_playout(7, 17, start + Duration::from_millis(120), 2, 2));
        assert_eq!(
            drained
                .into_iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5],
        );

        let late = clonk_network::VoiceFrame {
            client_id: 7,
            player_id: 17,
            stream_epoch: 5,
            sequence: 4,
            payload: clonk_audio::encode_voice_frame(&ramp_voice_frame(4)).to_vec(),
        };
        assert!(voice
            .accept_remote_frame(&snapshot, &late, start + Duration::from_millis(200))
            .is_none());
        assert!(voice.active_speakers(start + SPEAKING_HANGOVER).is_empty());
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
