//! Network voice chat: source authorization, speaking state and playout.
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
//! 48 kHz mono Opus frames travel on a bounded, droppable UDP media lane
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
//! stream and the receiving route supplies the source client ID. In a running
//! game, [`authenticated_selected_voice_crew`] additionally revalidates that
//! the claimed player belongs to that client before resolving the live
//! selected `PlayerState.cursor`. A lobby instead authorizes synchronized
//! client membership and the reserved [`LOBBY_VOICE_PLAYER_ID`] scope, so an
//! observer or a client with zero or several player profiles still has exactly
//! one voice identity. That lobby policy deliberately does not broaden the
//! unresolved in-game observer/multiple-local-player policy
//! (clonk-org/clonk-rs#419).
//!
//! Lobby playback is centered and non-positional. Running-game playback uses
//! the existing linear 700-pixel positional mix; the speaker glyph additionally
//! obeys per-viewport object/FoW visibility. Several speakers at once would
//! otherwise sum straight into the output clamp, so the audio mixer limits the
//! summed voice bus to its own ceiling — voice is the one source it may
//! attenuate, because the sound and music paths owe SDL_mixer's arithmetic.
//! Landscape openness and obstacles deliberately do not occlude speech
//! (clonk-org/clonk-rs#418).
//!
//! The media lane is encrypted (clonk-org/clonk-rs#426): each route agrees its
//! own key over the reliable control stream and seals every frame under it, so
//! a passive observer recovers nothing, and a peer that cannot agree a key gets
//! no lane rather than a cleartext one. That exchange is bound to no identity,
//! so it does not defend against an attacker who can rewrite the cleartext
//! control stream — the limitation the rest of the protocol already carries.
//! The network seal intentionally only authenticates and decrypts each
//! datagram; it keeps no replay window, since that would put per-connection
//! state on a lane whose droppability is the point. After that network step,
//! this app layer's [`VoiceActivityTracker`] owns duplicate/late suppression
//! through its shared epoch and sequence window.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clonk_audio::{
    EncodedVoiceFrame, VoiceCapture, VoiceCaptureControl, VoiceCaptureError, VoiceCaptureOptions,
    VoiceEchoReference, VoiceInputDeviceId, VoiceInputFrame, VoiceProcessingConfig,
    VoiceProcessingSwitches,
};
use clonk_engine::{ObjectSnapshot, PlayerStatus, SimulationSnapshot};

use crate::settings::VoiceActivation;

pub(crate) const SPEAKING_HANGOVER: Duration = Duration::from_millis(250);
/// A lobby speaker belongs to an authenticated network client, not to one of
/// its zero or more synchronized player profiles.
pub(crate) const LOBBY_VOICE_PLAYER_ID: i32 = clonk_engine::OWNER_NONE;
const VOICE_FRAME_DURATION: Duration = Duration::from_millis(20);
const INITIAL_VOICE_JITTER_FRAMES: usize = 4;
const MIN_VOICE_JITTER_FRAMES: usize = 2;
const MAX_VOICE_JITTER_FRAMES: usize = 6;
const MAX_PENDING_VOICE_FRAMES: usize = 8;
const VOICE_SEQUENCE_WINDOW_FRAMES: usize = u64::BITS as usize;
const MIN_VOICE_JITTER_OBSERVATIONS: usize = 3;
// The worker runs every 5 ms. One remaining (possibly partial) frame gives
// it time to conceal before underrun; concealing with two frames still queued
// needlessly replaces packets that can arrive in the next 20 ms.
const VOICE_PLAYOUT_GUARD_FRAMES: usize = 1;
const MAX_CONSECUTIVE_VOICE_PLC_FRAMES: u16 = 3;
const VOICE_CAPTURE_RETRY_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VoiceChatContext {
    Running,
    Lobby,
}

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
    requires_new_epoch: bool,
    last_frame_at: Instant,
    visually_active: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct VoiceActivityTracker {
    speakers: BTreeMap<(i32, i32), SpeakerActivity>,
    local_speaker: Option<((i32, i32), Instant)>,
}

pub(crate) struct CapturedVoiceFrame {
    pub(crate) stream_epoch: u32,
    pub(crate) sequence: u16,
    pub(crate) captured_at: Instant,
    pub(crate) payload: Option<EncodedVoiceFrame>,
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

pub(crate) struct AcceptedRemoteVoicePacket {
    pub(crate) stream_id: u64,
    pub(crate) reset_stream: bool,
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

    fn finish_at(&self, _at: Instant) {}

    fn is_finished(&self) -> bool {
        true
    }

    fn stream_generation(&self) -> u64 {
        0
    }
}

impl VoiceFrameSource for VoiceCapture {
    fn drain_frames(&self) -> Vec<VoiceInputFrame> {
        self.drain_frames()
    }

    fn stream_generation(&self) -> u64 {
        self.stream_generation()
    }

    fn finish_at(&self, at: Instant) {
        self.finish_at(at);
    }

    fn is_finished(&self) -> bool {
        self.is_finished()
    }
}

type VoiceCaptureOpener =
    Box<dyn FnMut(VoiceCaptureOptions) -> Result<Box<dyn VoiceFrameSource>, VoiceCaptureError>>;

#[derive(Debug, Default)]
pub(crate) struct RemoteVoiceStream {
    pub(crate) stream_epoch: u32,
    pub(crate) last_frame_at: Option<Instant>,
    jitter: RemoteVoiceJitterBuffer,
}

#[derive(Debug)]
struct BufferedRemoteVoiceFrame {
    sequence: u16,
    payload: EncodedVoiceFrame,
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
    pub(crate) rate_adjustment_ppm: i32,
    pub(crate) late_frames: u64,
}

#[derive(Debug, Default)]
struct VoicePlayoutClock {
    last_update: Option<Instant>,
    smoothed_error_frames: f64,
    rate_adjustment: f64,
}

impl VoicePlayoutClock {
    fn update(&mut self, now: Instant, buffered: Duration, target: Duration) -> i32 {
        let previous = self.last_update.replace(now).unwrap_or(now);
        let elapsed = now
            .saturating_duration_since(previous)
            .as_secs_f64()
            .min(0.1);
        let error_frames =
            (buffered.as_secs_f64() - target.as_secs_f64()) / VOICE_FRAME_DURATION.as_secs_f64();
        let smoothing = 1.0 - (-elapsed / 0.5).exp();
        self.smoothed_error_frames += (error_frames - self.smoothed_error_frames) * smoothing;
        // Small rate changes correct both persistent device skew and the
        // desired jitter delay. Limit their size and slew to avoid jumps.
        let desired = (self.smoothed_error_frames * 5_000.0).clamp(-10_000.0, 10_000.0);
        let step = 40_000.0 * elapsed;
        self.rate_adjustment += (desired - self.rate_adjustment).clamp(-step, step);
        self.rate_adjustment.round() as i32
    }
}

#[derive(Debug)]
struct RemoteVoiceJitterBuffer {
    pending: Vec<BufferedRemoteVoiceFrame>,
    next_playout_sequence: Option<u16>,
    first_arrival_at: Option<Instant>,
    arrival_origin: Option<Instant>,
    arrival_sequence: Option<(u16, i64)>,
    recent_transits: VecDeque<(u16, Instant, i128)>,
    arrival_observations: usize,
    target_frames: usize,
    started: bool,
    next_playout_at: Option<Instant>,
    end_sequence: Option<u16>,
    consecutive_concealed_frames: u16,
    previous_output: Option<[i16; clonk_audio::VOICE_FRAME_SAMPLES]>,
    highest_arrival_sequence: Option<u16>,
    reordered_frames: u64,
    concealed_frames: u64,
    clock: VoicePlayoutClock,
    late_frames: u64,
    decoder: Result<clonk_audio::VoiceDecoder, clonk_audio::VoiceCodecError>,
}

impl Default for RemoteVoiceJitterBuffer {
    fn default() -> Self {
        Self {
            pending: Vec::with_capacity(MAX_PENDING_VOICE_FRAMES),
            next_playout_sequence: None,
            first_arrival_at: None,
            arrival_origin: None,
            arrival_sequence: None,
            recent_transits: VecDeque::with_capacity(128),
            arrival_observations: 0,
            target_frames: INITIAL_VOICE_JITTER_FRAMES,
            started: false,
            next_playout_at: None,
            end_sequence: None,
            consecutive_concealed_frames: 0,
            previous_output: None,
            highest_arrival_sequence: None,
            reordered_frames: 0,
            concealed_frames: 0,
            clock: VoicePlayoutClock::default(),
            late_frames: 0,
            decoder: clonk_audio::VoiceDecoder::new(),
        }
    }
}

impl RemoteVoiceJitterBuffer {
    fn can_insert(&self, sequence: u16) -> bool {
        if self
            .end_sequence
            .is_some_and(|end| sequence.wrapping_sub(end) <= u16::MAX / 2)
        {
            return false;
        }
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

    fn replaces_concealed_tail(&self, sequence: u16) -> bool {
        self.started
            && self.next_playout_sequence.is_some_and(|next| {
                (1..=self
                    .consecutive_concealed_frames
                    .min(MAX_CONSECUTIVE_VOICE_PLC_FRAMES))
                    .contains(&next.wrapping_sub(sequence))
            })
    }

    fn can_end(&self, sequence: u16) -> bool {
        self.end_sequence.is_none()
            && (self.insertion_anchor(sequence).is_some() || self.replaces_concealed_tail(sequence))
    }

    fn end(&mut self, sequence: u16, received_at: Instant) -> bool {
        if !self.can_end(sequence) {
            return false;
        }
        self.end_sequence = Some(sequence);
        self.next_playout_sequence.get_or_insert(sequence);
        self.first_arrival_at.get_or_insert(received_at);
        self.pending
            .retain(|frame| frame.sequence.wrapping_sub(sequence) > u16::MAX / 2);
        true
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

    fn insert(&mut self, sequence: u16, received_at: Instant, payload: EncodedVoiceFrame) -> bool {
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
            .push(BufferedRemoteVoiceFrame { sequence, payload });
        self.pending
            .sort_unstable_by_key(|pending| pending.sequence.wrapping_sub(next_playout_sequence));
        self.observe_arrival(sequence, received_at);
        true
    }

    fn observe_arrival(&mut self, sequence: u16, received_at: Instant) -> bool {
        if self
            .recent_transits
            .iter()
            .any(|(seen, _, _)| *seen == sequence)
        {
            return false;
        }
        let origin_at = *self.arrival_origin.get_or_insert(received_at);
        let (previous, position) = *self.arrival_sequence.get_or_insert((sequence, 0));
        let advance = i64::from(sequence.wrapping_sub(previous) as i16);
        let position = position.saturating_add(advance);
        if advance > 0 {
            self.arrival_sequence = Some((sequence, position));
        }
        let arrival_offset_ns = received_at
            .checked_duration_since(origin_at)
            .map(duration_nanos)
            .unwrap_or_else(|| -duration_nanos(origin_at.duration_since(received_at)));
        let residual_ns = arrival_offset_ns.saturating_sub(
            i128::from(position).saturating_mul(duration_nanos(VOICE_FRAME_DURATION)),
        );
        while self.recent_transits.len() >= 128
            || self.recent_transits.front().is_some_and(|(_, at, _)| {
                received_at.saturating_duration_since(*at) > Duration::from_secs(3)
            })
        {
            self.recent_transits.pop_front();
        }
        self.recent_transits
            .push_back((sequence, received_at, residual_ns));
        self.arrival_observations = self.arrival_observations.saturating_add(1);
        if self.arrival_observations < MIN_VOICE_JITTER_OBSERVATIONS {
            return true;
        }
        // A bounded recent window forgets route changes and temporary bursts.
        // Unwrapped positions prevent a long call from looking like a huge
        // delay jump each time the wire sequence counter wraps.
        let (minimum, maximum) = self.recent_transits.iter().fold(
            (residual_ns, residual_ns),
            |(minimum, maximum), &(_, _, transit)| (minimum.min(transit), maximum.max(transit)),
        );
        let frame_ns = duration_nanos(VOICE_FRAME_DURATION);
        let spread_ns = maximum.saturating_sub(minimum);
        let spread_frames = spread_ns.saturating_add(frame_ns - 1) / frame_ns;
        self.target_frames = usize::try_from(spread_frames.saturating_add(1))
            .unwrap_or(MAX_VOICE_JITTER_FRAMES)
            .clamp(MIN_VOICE_JITTER_FRAMES, MAX_VOICE_JITTER_FRAMES);
        true
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
        let frame_duration = Duration::from_secs_f64(
            VOICE_FRAME_DURATION.as_secs_f64() / (1.0 + self.clock.rate_adjustment / 1_000_000.0),
        );
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
            self.next_playout_at = Some(now);
        }

        let mut ready = Vec::with_capacity(max_frames.min(self.pending.len()));
        while ready.len() < max_frames {
            let expected = self
                .next_playout_sequence
                .expect("a started voice jitter buffer has a playout sequence");
            if self
                .end_sequence
                .is_some_and(|end| expected.wrapping_sub(end) <= u16::MAX / 2)
            {
                break;
            }
            let Some(position) = self
                .pending
                .iter()
                .position(|pending| pending.sequence == expected)
            else {
                let successor = self
                    .pending
                    .iter()
                    .min_by_key(|pending| pending.sequence.wrapping_sub(expected))
                    .map(|pending| (pending.sequence, pending.payload));
                let Some((successor_sequence, successor_payload)) = successor else {
                    if self.previous_output.is_none()
                        || self.consecutive_concealed_frames >= MAX_CONSECUTIVE_VOICE_PLC_FRAMES
                        || self.next_playout_at.is_none_or(|deadline| now < deadline)
                        || buffered_playout_frames.saturating_add(ready.len())
                            > VOICE_PLAYOUT_GUARD_FRAMES
                    {
                        break;
                    }
                    let samples = self.decode_for_playout(None, false);
                    self.previous_output = Some(samples);
                    ready.push(RemoteVoicePlayoutFrame {
                        sequence: expected,
                        samples,
                        concealed: true,
                    });
                    self.concealed_frames = self.concealed_frames.saturating_add(1);
                    self.consecutive_concealed_frames += 1;
                    self.next_playout_sequence = Some(expected.wrapping_add(1));
                    self.next_playout_at = self
                        .next_playout_at
                        .and_then(|at| at.checked_add(frame_duration));
                    continue;
                };
                let Some(_) = self.previous_output else {
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
                // Opus FEC describes only the immediately preceding interval.
                // Earlier missing intervals need independent PLC calls so the
                // decoder clock advances by exactly the missing duration.
                let samples = self.decode_for_playout(
                    (successor_distance == 1).then_some(successor_payload),
                    successor_distance == 1,
                );
                self.previous_output = Some(samples);
                ready.push(RemoteVoicePlayoutFrame {
                    sequence: expected,
                    samples,
                    concealed: true,
                });
                self.concealed_frames = self.concealed_frames.saturating_add(1);
                self.consecutive_concealed_frames =
                    self.consecutive_concealed_frames.saturating_add(1);
                self.next_playout_at = self
                    .next_playout_at
                    .and_then(|at| at.checked_add(frame_duration));
                self.next_playout_sequence =
                    Some(if successor_distance <= MAX_CONSECUTIVE_VOICE_PLC_FRAMES {
                        expected.wrapping_add(1)
                    } else {
                        successor_sequence
                    });
                continue;
            };
            let frame = self.pending.remove(position);
            self.consecutive_concealed_frames = 0;
            self.next_playout_at = self
                .next_playout_at
                .and_then(|at| at.checked_add(frame_duration));
            let samples = self.decode_for_playout(Some(frame.payload), false);
            self.previous_output = Some(samples);
            ready.push(RemoteVoicePlayoutFrame {
                sequence: frame.sequence,
                samples,
                concealed: false,
            });
            self.next_playout_sequence = Some(expected.wrapping_add(1));
        }
        ready
    }

    fn decode_for_playout(
        &mut self,
        packet: Option<EncodedVoiceFrame>,
        fec: bool,
    ) -> [i16; clonk_audio::VOICE_FRAME_SAMPLES] {
        let Ok(decoder) = self.decoder.as_mut() else {
            return [0; clonk_audio::VOICE_FRAME_SAMPLES];
        };
        let decoded = if let Some(packet) = packet {
            decoder.decode(&packet, fec)
        } else {
            decoder.conceal()
        };
        decoded
            .or_else(|_| decoder.conceal())
            .unwrap_or([0; clonk_audio::VOICE_FRAME_SAMPLES])
    }

    fn stats(&self) -> RemoteVoicePlayoutStats {
        RemoteVoicePlayoutStats {
            target_frames: self.target_frames,
            reordered_frames: self.reordered_frames,
            concealed_frames: self.concealed_frames,
            rate_adjustment_ppm: self.clock.rate_adjustment.round() as i32,
            late_frames: self.late_frames,
        }
    }
}

fn duration_nanos(duration: Duration) -> i128 {
    i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX)
}

pub(crate) struct VoiceChatState {
    context: Option<VoiceChatContext>,
    activity: VoiceActivityTracker,
    capture: Option<Box<dyn VoiceFrameSource>>,
    capture_control: Option<VoiceCaptureControl>,
    finishing_capture: Option<Instant>,
    capture_opener: VoiceCaptureOpener,
    /// Shared with the live capture, so a settings change reaches the
    /// microphone thread on its next frame instead of waiting for the
    /// microphone to close (clonk-org/clonk-rs#421).
    processing: Arc<VoiceProcessingSwitches>,
    capture_key: Option<winit::keyboard::KeyCode>,
    capture_input_device: Option<VoiceInputDeviceId>,
    next_capture_retry_at: Option<Instant>,
    capture_stream_generation: u64,
    activation_gate: VoiceActivationGate,
    activation_preroll: VecDeque<VoiceInputFrame>,
    activation_open_failed: bool,
    stream_epoch: u32,
    next_sequence: u16,
    capture_sample_origin: Option<u64>,
    last_capture_sample_offset: Option<u64>,
    transmitted_in_epoch: bool,
    pub(crate) remote_streams: BTreeMap<(i32, i32), RemoteVoiceStream>,
    /// Clients this player has silenced, mirroring the runtime client list's
    /// existing per-client mute so one control silences a participant rather
    /// than text and voice needing separate ones. A client is muted as a whole,
    /// which also covers every local player speaking from it.
    ///
    /// Muting is purely local and purely presentational: it discards frames
    /// after they arrive, tells the muted peer nothing, and never reaches the
    /// control stream — the same boundary the whole feature sits behind
    /// (clonk-org/clonk-rs#301). It outlives any one stream, so a peer that
    /// reconnects or restarts its stream stays muted for the session.
    muted_clients: BTreeSet<i32>,
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
        Self::with_source_opener(|_| Err::<VoiceCapture, _>(VoiceCaptureError::Unavailable))
    }
}

impl VoiceChatState {
    pub(crate) fn with_source_opener<F, S>(mut opener: F) -> Self
    where
        F: FnMut(VoiceCaptureOptions) -> Result<S, VoiceCaptureError> + 'static,
        S: VoiceFrameSource + 'static,
    {
        Self {
            context: None,
            activity: VoiceActivityTracker::default(),
            capture: None,
            capture_control: None,
            finishing_capture: None,
            capture_opener: Box::new(move |options| {
                opener(options).map(|source| Box::new(source) as Box<dyn VoiceFrameSource>)
            }),
            processing: VoiceProcessingSwitches::new(VoiceProcessingConfig::default()),
            capture_key: None,
            capture_input_device: None,
            next_capture_retry_at: None,
            capture_stream_generation: 0,
            activation_gate: VoiceActivationGate::default(),
            activation_preroll: VecDeque::with_capacity(3),
            activation_open_failed: false,
            stream_epoch: 0,
            next_sequence: 0,
            capture_sample_origin: None,
            last_capture_sample_offset: None,
            transmitted_in_epoch: false,
            remote_streams: BTreeMap::new(),
            muted_clients: BTreeSet::new(),
        }
    }

    #[cfg(test)]
    fn with_capture_opener<F, S>(opener: F) -> Self
    where
        F: FnMut(VoiceCaptureOptions) -> Result<S, VoiceCaptureError> + 'static,
        S: VoiceFrameSource + 'static,
    {
        Self::with_source_opener(opener)
    }

    /// Which capture-processing stages run. Takes effect on the microphone's
    /// next frame, whether or not one is open.
    pub(crate) fn activity_snapshot(&self) -> VoiceActivityTracker {
        self.activity.clone()
    }

    pub(crate) fn set_processing(&self, config: VoiceProcessingConfig) {
        self.processing.set(config);
    }

    /// Ends capture and queued playout when speech crosses a lobby/game/
    /// inactive boundary. A retained network session keeps its replay
    /// tombstones, so delayed datagrams cannot reopen speech in the next
    /// context; [`Self::clear`] remains the full session teardown.
    pub(crate) fn reconcile_context(
        &mut self,
        context: Option<VoiceChatContext>,
    ) -> Vec<(i32, i32)> {
        if self.context == context {
            return Vec::new();
        }
        let previous = self.context;
        self.context = context;
        if previous.is_none() {
            return Vec::new();
        }

        self.stop_capture();
        self.activity.seal_context();
        let removed = self.remote_streams.keys().copied().collect::<Vec<_>>();
        for &speaker in &removed {
            self.advance_replay_floor_past_accepted(speaker);
        }
        self.remote_streams.clear();
        removed
    }

    /// `key` is the push-to-talk key whose release closes this capture again;
    /// `None` opens a capture no key owns. `echo_reference` is what the mixer
    /// is playing, which the canceller needs and nothing else does.
    pub(crate) fn start_capture(
        &mut self,
        key: Option<winit::keyboard::KeyCode>,
        echo_reference: Option<VoiceEchoReference>,
    ) -> Result<(), VoiceCaptureError> {
        self.start_capture_on_device(key, echo_reference, None)
    }

    pub(crate) fn start_capture_on_device(
        &mut self,
        key: Option<winit::keyboard::KeyCode>,
        echo_reference: Option<VoiceEchoReference>,
        input_device: Option<VoiceInputDeviceId>,
    ) -> Result<(), VoiceCaptureError> {
        self.start_capture_on_device_at(key, echo_reference, input_device, Instant::now())
    }

    pub(crate) fn capture_control(&self) -> Option<VoiceCaptureControl> {
        self.capture_control.clone()
    }

    pub(crate) fn set_capture_control(&mut self, control: VoiceCaptureControl) {
        if self.capture.is_none() {
            self.capture_control = Some(control);
        }
    }

    fn start_capture_on_device_at(
        &mut self,
        key: Option<winit::keyboard::KeyCode>,
        echo_reference: Option<VoiceEchoReference>,
        input_device: Option<VoiceInputDeviceId>,
        now: Instant,
    ) -> Result<(), VoiceCaptureError> {
        if self.finishing_capture.is_some() {
            self.stop_capture();
        }
        if self.capture.is_some() {
            return Ok(());
        }
        // Intent must outlive the physical stream. If the selected microphone
        // is absent, the release of a held push-to-talk key still owns and
        // cancels this pending capture instead of falling through to gameplay.
        self.capture_key = key;
        self.capture_input_device = input_device.clone();
        let mut options = VoiceCaptureOptions::new(self.processing.clone());
        options.control = self
            .capture_control
            .get_or_insert_with(VoiceCaptureControl::default)
            .clone();
        options.input_device = input_device;
        options.echo_reference = echo_reference;
        let capture = match (self.capture_opener)(options) {
            Ok(capture) => capture,
            Err(error) => {
                self.next_capture_retry_at = now.checked_add(VOICE_CAPTURE_RETRY_INTERVAL);
                return Err(error);
            }
        };
        self.capture_stream_generation = capture.stream_generation();
        self.capture = Some(capture);
        self.next_capture_retry_at = None;
        self.activation_gate.close();
        self.stream_epoch = self.stream_epoch.wrapping_add(1).max(1);
        self.activation_preroll.clear();
        self.next_sequence = 0;
        self.capture_sample_origin = None;
        self.last_capture_sample_offset = None;
        self.transmitted_in_epoch = false;
        Ok(())
    }

    pub(crate) fn start_voice_activated_capture_on_device_at(
        &mut self,
        echo_reference: Option<VoiceEchoReference>,
        input_device: Option<VoiceInputDeviceId>,
        now: Instant,
    ) -> Result<(), VoiceCaptureError> {
        if self.capture.is_some()
            || (self.activation_open_failed
                && self
                    .next_capture_retry_at
                    .is_some_and(|retry_at| now < retry_at))
        {
            return Ok(());
        }
        let opened = self.start_capture_on_device_at(None, echo_reference, input_device, now);
        self.activation_open_failed = opened.is_err();
        opened
    }

    /// Applies a settings change without discarding who currently owns the
    /// capture. Replacing the physical stream starts a fresh media epoch, but
    /// leaves the network and every remote playout stream untouched.
    pub(crate) fn reconcile_capture_device(
        &mut self,
        input_device: Option<VoiceInputDeviceId>,
        echo_reference: Option<VoiceEchoReference>,
    ) -> Result<(), VoiceCaptureError> {
        self.reconcile_capture_device_at(input_device, echo_reference, Instant::now())
    }

    pub(crate) fn reconcile_capture_device_at(
        &mut self,
        input_device: Option<VoiceInputDeviceId>,
        echo_reference: Option<VoiceEchoReference>,
        now: Instant,
    ) -> Result<(), VoiceCaptureError> {
        if self.finishing_capture.is_some() {
            if self.capture_input_device != input_device {
                self.stop_capture();
            }
            return Ok(());
        }
        if self.capture_input_device == input_device {
            if self.capture.is_none()
                && self.capture_key.is_some()
                && self
                    .next_capture_retry_at
                    .is_none_or(|retry_at| now >= retry_at)
            {
                return self.start_capture_on_device_at(
                    self.capture_key,
                    echo_reference,
                    input_device,
                    now,
                );
            }
            return Ok(());
        }
        let requested =
            self.capture.is_some() || self.capture_key.is_some() || self.activation_open_failed;
        let key = self.capture_key;
        if let Some(control) = self.capture_control.take() {
            control.abort();
        }
        self.capture = None;
        self.capture_input_device = input_device.clone();
        self.next_capture_retry_at = None;
        self.activation_open_failed = false;
        if !requested {
            return Ok(());
        }
        let opened = self.start_capture_on_device_at(key, echo_reference, input_device, now);
        if key.is_none() {
            self.activation_open_failed = opened.is_err();
        }
        opened
    }

    pub(crate) fn stop_capture(&mut self) {
        if let Some(control) = self.capture_control.take() {
            control.abort();
        }
        self.capture = None;
        self.finishing_capture = None;
        self.capture_key = None;
        self.capture_input_device = None;
        self.next_capture_retry_at = None;
        self.capture_stream_generation = 0;
        self.activation_open_failed = false;
        self.activation_gate.close();
        self.activation_preroll.clear();
    }

    pub(crate) fn finish_capture_at(&mut self, at: Instant) {
        self.capture_key = None;
        self.next_capture_retry_at = None;
        if let Some(control) = &self.capture_control {
            control.finish_at(at);
        }
        if let Some(capture) = &self.capture {
            self.finishing_capture.get_or_insert(at);
            capture.finish_at(at);
        }
    }

    pub(crate) fn capture_active(&self) -> bool {
        self.capture.is_some() && self.finishing_capture.is_none()
    }

    pub(crate) fn capture_key(&self) -> Option<winit::keyboard::KeyCode> {
        self.capture_key
    }

    pub(crate) fn voice_activated_capture_requested(&self) -> bool {
        self.finishing_capture.is_none()
            && self.capture_key.is_none()
            && (self.capture.is_some() || self.activation_open_failed)
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
        // Completion is sampled before draining: the last DSP enqueue must
        // precede the end marker, even when completion races this pump.
        let finished = capture.is_finished();
        let frames = capture.drain_frames();
        let generation = capture.stream_generation();
        if generation != self.capture_stream_generation {
            self.capture_stream_generation = generation;
            self.stream_epoch = self.stream_epoch.wrapping_add(1).max(1);
            self.next_sequence = 0;
            self.capture_sample_origin = None;
            self.last_capture_sample_offset = None;
            self.transmitted_in_epoch = false;
            self.activation_gate.close();
            self.activation_preroll.clear();
        }
        let now = Instant::now();
        let mut result = Vec::new();
        for frame in frames.into_iter().filter(|frame| frame.is_fresh_at(now)) {
            let Some(activation) = activation else {
                result.push(self.stamp_captured_frame(frame, false));
                continue;
            };
            let was_open = self.activation_gate.open;
            if let Some(mut reopened) = self.activation_gate.admit(frame.level, activation) {
                while let Some(previous) = self.activation_preroll.pop_front() {
                    let nearby = previous
                        .capture_timing()
                        .zip(frame.capture_timing())
                        .is_none_or(|(before, current)| {
                            current
                                .captured_at
                                .saturating_duration_since(before.captured_at)
                                <= Duration::from_millis(60)
                        });
                    if previous.is_fresh_at(now) && nearby {
                        result.push(self.stamp_captured_frame(previous, reopened));
                        reopened = false;
                    }
                }
                result.push(self.stamp_captured_frame(frame, reopened));
            } else if was_open {
                // One quiet packet releases Opus lookahead, including when
                // the user selects zero hangover. Never replay it as preroll.
                let tail = self.stamp_captured_frame(frame, false);
                let captured_at = tail.captured_at;
                result.push(tail);
                result.push(CapturedVoiceFrame {
                    stream_epoch: self.stream_epoch,
                    sequence: self.next_sequence,
                    captured_at,
                    payload: None,
                });
            } else {
                if self.activation_preroll.len() == 3 {
                    self.activation_preroll.pop_front();
                }
                self.activation_preroll.push_back(frame);
            }
        }
        if let Some(at) = self.finishing_capture.filter(|at| {
            finished || now.saturating_duration_since(*at) >= Duration::from_millis(160)
        }) {
            if self.transmitted_in_epoch {
                result.push(CapturedVoiceFrame {
                    stream_epoch: self.stream_epoch,
                    sequence: self.next_sequence,
                    captured_at: at,
                    payload: None,
                });
            }
            self.stop_capture();
        }
        result
    }

    fn stamp_captured_frame(
        &mut self,
        frame: VoiceInputFrame,
        starts_new_stream: bool,
    ) -> CapturedVoiceFrame {
        let capture_gap = frame
            .capture_timing()
            .zip(self.last_capture_sample_offset)
            .is_some_and(|(timing, previous)| {
                timing.sample_offset.saturating_sub(previous)
                    > VOICE_SEQUENCE_WINDOW_FRAMES as u64 * clonk_audio::VOICE_FRAME_SAMPLES as u64
            });
        // An explicit flag survives sequence wrap during long utterances.
        if (starts_new_stream || capture_gap) && self.transmitted_in_epoch {
            self.stream_epoch = self.stream_epoch.wrapping_add(1).max(1);
            self.next_sequence = 0;
            self.capture_sample_origin = None;
            self.transmitted_in_epoch = false;
        }
        let sequence = frame.capture_timing().map_or(self.next_sequence, |timing| {
            let origin = *self
                .capture_sample_origin
                .get_or_insert(timing.sample_offset);
            (timing.sample_offset.saturating_sub(origin) / clonk_audio::VOICE_FRAME_SAMPLES as u64)
                as u16
        });
        self.next_sequence = sequence.wrapping_add(1);
        self.last_capture_sample_offset = frame.capture_timing().map(|timing| timing.sample_offset);
        self.transmitted_in_epoch = true;
        CapturedVoiceFrame {
            stream_epoch: self.stream_epoch,
            sequence,
            captured_at: frame
                .capture_timing()
                .map_or_else(Instant::now, |timing| timing.captured_at),
            payload: Some(frame.payload),
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

    fn note_authenticated_remote_frame(
        &mut self,
        client_id: i32,
        player_id: i32,
        stream_epoch: u32,
        sequence: u16,
        received_at: Instant,
    ) -> VoiceFrameDisposition {
        let disposition = self.activity.note_authenticated_frame(
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

    /// Whether this player has silenced a participant's voice.
    pub(crate) fn is_client_muted(&self, client_id: i32) -> bool {
        self.muted_clients.contains(&client_id)
    }

    /// Silence or unsilence one participant, returning the streams that must
    /// stop playing.
    ///
    /// Muting drops the peer's live streams so the buffered tail stops too,
    /// rather than letting the last few frames play out after the player asked
    /// for silence. Nothing is sent: the muted peer keeps transmitting and is
    /// never told, which is what keeps this local and out of the control
    /// stream.
    pub(crate) fn set_client_muted(&mut self, client_id: i32, muted: bool) -> Vec<(i32, i32)> {
        if !muted {
            self.muted_clients.remove(&client_id);
            return Vec::new();
        }
        self.muted_clients.insert(client_id);
        let silenced = self
            .remote_streams
            .keys()
            .copied()
            .filter(|(stream_client, _)| *stream_client == client_id)
            .collect::<Vec<_>>();
        for key in &silenced {
            self.remote_streams.remove(key);
        }
        silenced
    }

    pub(crate) fn accept_remote_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        frame: &clonk_network::VoiceFrame,
        received_at: Instant,
    ) -> Option<AcceptedRemoteVoicePacket> {
        let (client_id, payload) = self.prepare_remote_frame(frame, received_at)?;
        let disposition = self.note_remote_frame(
            snapshot,
            client_id,
            frame.player_id,
            frame.stream_epoch,
            frame.sequence,
            received_at,
        );
        self.finish_remote_frame(frame, received_at, client_id, payload, disposition)
    }

    /// Admit a decoded media frame after the caller has authorized its
    /// client-scoped identity. Transport authenticates the client ID; the
    /// caller still owns the context-specific scope check. The sequence/epoch
    /// replay window remains here exactly as it is for positional speech.
    pub(crate) fn accept_authorized_remote_frame(
        &mut self,
        frame: &clonk_network::VoiceFrame,
        received_at: Instant,
    ) -> Option<AcceptedRemoteVoicePacket> {
        let (client_id, payload) = self.prepare_remote_frame(frame, received_at)?;
        let disposition = self.note_authenticated_remote_frame(
            client_id,
            frame.player_id,
            frame.stream_epoch,
            frame.sequence,
            received_at,
        );
        self.finish_remote_frame(frame, received_at, client_id, payload, disposition)
    }

    fn prepare_remote_frame(
        &mut self,
        frame: &clonk_network::VoiceFrame,
        received_at: Instant,
    ) -> Option<(i32, Option<EncodedVoiceFrame>)> {
        let client_id = i32::try_from(frame.client_id).ok()?;
        // Before decoding: a muted peer costs nothing beyond the bytes the
        // transport already read.
        if self.is_client_muted(client_id) {
            return None;
        }
        let payload = if frame.payload.is_empty() {
            None // V3 reserves the empty authenticated payload for end-of-stream.
        } else {
            Some(EncodedVoiceFrame::from_packet(&frame.payload).ok()?)
        };
        if let Some(stream) = self
            .remote_streams
            .get_mut(&(client_id, frame.player_id))
            .filter(|stream| stream.stream_epoch == frame.stream_epoch)
        {
            let admitted = if payload.is_some() {
                stream.jitter.can_insert(frame.sequence)
            } else {
                stream.jitter.can_end(frame.sequence)
            };
            if !admitted {
                let late = stream.jitter.next_playout_sequence.is_some_and(|next| {
                    (1..=VOICE_SEQUENCE_WINDOW_FRAMES as u16)
                        .contains(&next.wrapping_sub(frame.sequence))
                });
                if payload.is_some()
                    && stream.jitter.started
                    && late
                    && stream.jitter.observe_arrival(frame.sequence, received_at)
                {
                    stream.jitter.late_frames = stream.jitter.late_frames.saturating_add(1);
                }
                return None;
            }
        }
        Some((client_id, payload))
    }

    fn finish_remote_frame(
        &mut self,
        frame: &clonk_network::VoiceFrame,
        received_at: Instant,
        client_id: i32,
        payload: Option<EncodedVoiceFrame>,
        disposition: VoiceFrameDisposition,
    ) -> Option<AcceptedRemoteVoicePacket> {
        // Ownership was checked before producing this disposition. A late
        // end marker can replace only an already-concealed tail in the same
        // epoch; it must never bypass the floor for actual audio or revive an
        // expired stream. The jitter buffer still rejects a repeated end.
        let disposition = if disposition == VoiceFrameDisposition::DuplicateOrLate
            && payload.is_none()
            && self
                .remote_streams
                .get(&(client_id, frame.player_id))
                .is_some_and(|stream| {
                    stream.stream_epoch == frame.stream_epoch
                        && stream.jitter.replaces_concealed_tail(frame.sequence)
                        && stream.jitter.can_end(frame.sequence)
                }) {
            VoiceFrameDisposition::Accepted
        } else {
            disposition
        };
        match disposition {
            VoiceFrameDisposition::Accepted | VoiceFrameDisposition::AcceptedNewEpoch => {
                let inserted = self
                    .remote_streams
                    .get_mut(&(client_id, frame.player_id))
                    .is_some_and(|stream| match payload {
                        Some(payload) => stream.jitter.insert(frame.sequence, received_at, payload),
                        None => stream.jitter.end(frame.sequence, received_at),
                    });
                if !inserted {
                    return None;
                }
                Some(AcceptedRemoteVoicePacket {
                    stream_id: voice_stream_id(client_id, frame.player_id),
                    reset_stream: disposition == VoiceFrameDisposition::AcceptedNewEpoch,
                })
            }
            VoiceFrameDisposition::UnknownPlayer
            | VoiceFrameDisposition::OwnershipMismatch
            | VoiceFrameDisposition::DuplicateOrLate => None,
        }
    }

    pub(crate) fn update_playout_clock(
        &mut self,
        client: i32,
        player: i32,
        now: Instant,
        queued: Duration,
    ) -> i32 {
        let Some(stream) = self.remote_streams.get_mut(&(client, player)) else {
            return 0;
        };
        if !stream.jitter.started {
            return 0;
        }
        let pending = VOICE_FRAME_DURATION.saturating_mul(stream.jitter.pending.len() as u32);
        let target = VOICE_FRAME_DURATION.saturating_mul(stream.jitter.target_frames as u32);
        stream
            .jitter
            .clock
            .update(now, queued.saturating_add(pending), target)
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
        self.context = None;
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

/// The voice source policy for a running round.
///
/// Proximity voice is a Rust-only extension with no C++ oracle, so this is a
/// decision rather than a port: **a voice source is one active player's selected
/// crew, and nothing else.** Two cases follow from that rather than from an
/// accident of the cursor lookup:
///
/// - **An observer is not a voice source.** A participant with no active player
///   at this client, or an active player with no selected crew, has no position
///   to mix from. It is silent in both directions: `local_voice_identity`
///   returns `None`, so the microphone is never opened
///   (see [`crate::game_app`]'s `update_voice_chat`), and an inbound frame
///   claiming such a player resolves to no source and is dropped. Playing it
///   unpositioned instead would hand observers a broadcast channel that
///   positional players do not have.
/// - **A client with several local players speaks as its selected one.** The
///   sender stamps `local_owner`, the single local player whose crew the client
///   is controlling, so a client contributes at most one stream and the choice
///   never depends on player order. Receivers key streams on
///   `(client_id, player_id)` and mix each from *that* player's own cursor, so
///   two local players would be two independently positioned sources rather
///   than one ambiguous one — the receiving side needs no tie-break at all.
///
/// The determinism boundary is untouched either way: this only decides which
/// presentation-only frames are mixed, and voice never enters controls,
/// snapshots, savegames, records or sync checks (clonk-org/clonk-rs#301).
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

        self.note_authenticated_frame(client_id, player_id, stream_epoch, sequence, received_at)
    }

    fn note_authenticated_frame(
        &mut self,
        client_id: i32,
        player_id: i32,
        stream_epoch: u32,
        sequence: u16,
        received_at: Instant,
    ) -> VoiceFrameDisposition {
        let key = (client_id, player_id);
        let mut disposition = VoiceFrameDisposition::Accepted;
        if let Some(activity) = self.speakers.get_mut(&key) {
            if activity.stream_epoch == stream_epoch {
                if activity.requires_new_epoch {
                    return VoiceFrameDisposition::DuplicateOrLate;
                }
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
                activity.requires_new_epoch = false;
            }
            activity.stream_epoch = stream_epoch;
            activity.last_frame_at = received_at;
            activity.visually_active = true;
        } else {
            self.speakers.insert(
                key,
                SpeakerActivity {
                    stream_epoch,
                    latest_sequence: sequence,
                    seen_sequences: 1,
                    playout_floor: None,
                    requires_new_epoch: false,
                    last_frame_at: received_at,
                    visually_active: true,
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
                (activity.visually_active
                    && now.saturating_duration_since(activity.last_frame_at) < SPEAKING_HANGOVER)
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

    pub(crate) fn seal_context(&mut self) {
        self.local_speaker = None;
        for activity in self.speakers.values_mut() {
            activity.visually_active = false;
            activity.requires_new_epoch = true;
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

    fn speech_voice_frame(sequence: u16) -> [i16; clonk_audio::VOICE_FRAME_SAMPLES] {
        std::array::from_fn(|sample| {
            let position = usize::from(sequence) * clonk_audio::VOICE_FRAME_SAMPLES + sample;
            (12_000.0
                * (std::f64::consts::TAU * 200.0 * position as f64
                    / f64::from(clonk_audio::VOICE_SAMPLE_RATE))
                .sin()) as i16
        })
    }

    fn bipolar_voice_frame() -> [i16; clonk_audio::VOICE_FRAME_SAMPLES] {
        speech_voice_frame(0)
    }

    struct TestVoiceSource {
        frames: RefCell<Vec<VoiceInputFrame>>,
    }

    impl TestVoiceSource {
        fn with_frame(payload: EncodedVoiceFrame) -> Self {
            Self {
                frames: RefCell::new(vec![VoiceInputFrame::test_frame(payload, 1.0)]),
            }
        }

        fn with_levels(levels: &[f32]) -> Self {
            Self {
                frames: RefCell::new(
                    levels
                        .iter()
                        .map(|&level| {
                            VoiceInputFrame::test_frame(
                                clonk_audio::test_encode_voice_frame(
                                    &[0; clonk_audio::VOICE_FRAME_SAMPLES],
                                )
                                .unwrap(),
                                level,
                            )
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
    fn voice_activity_tracker_rejects_newest_late_and_old_epoch_replays() {
        let snapshot = snapshot_with_player(17, 7);
        let now = Instant::now();
        let mut activity = VoiceActivityTracker::default();

        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 5, 100, now),
            VoiceFrameDisposition::Accepted,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 5, 100, now),
            VoiceFrameDisposition::DuplicateOrLate,
            "the network seal authenticates a duplicate; the activity tracker refuses it",
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 5, 36, now),
            VoiceFrameDisposition::DuplicateOrLate,
            "a frame outside the activity tracker's late window must not reopen speech",
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 6, 0, now),
            VoiceFrameDisposition::AcceptedNewEpoch,
        );
        assert_eq!(
            activity.note_frame(&snapshot, 7, 17, 5, 101, now),
            VoiceFrameDisposition::DuplicateOrLate,
            "a delayed frame from the previous epoch must remain rejected",
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

        assert!(jitter.insert(
            40,
            start,
            clonk_audio::test_encode_voice_frame(&[40; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap()
        ));
        assert!(jitter.insert(
            42,
            start + Duration::from_millis(20),
            clonk_audio::test_encode_voice_frame(&[42; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap(),
        ));
        assert!(jitter.insert(
            41,
            start + Duration::from_millis(35),
            clonk_audio::test_encode_voice_frame(&[41; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap(),
        ));
        assert!(jitter.insert(
            43,
            start + Duration::from_millis(60),
            clonk_audio::test_encode_voice_frame(&[43; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap(),
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
                clonk_audio::test_encode_voice_frame(
                    &[sequence as i16; clonk_audio::VOICE_FRAME_SAMPLES]
                )
                .unwrap(),
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

        assert!(jitter.insert(
            13,
            start,
            clonk_audio::test_encode_voice_frame(&bipolar_voice_frame()).unwrap()
        ));
        assert!(jitter.insert(
            0,
            start + Duration::from_millis(10),
            clonk_audio::test_encode_voice_frame(&bipolar_voice_frame()).unwrap(),
        ));
    }

    #[test]
    fn remote_voice_jitter_buffer_orders_sequence_wrap() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for (sequence, arrival_ms) in [(u16::MAX - 1, 0), (0, 20), (u16::MAX, 35), (1, 60)] {
            assert!(jitter.insert(
                sequence,
                start + Duration::from_millis(arrival_ms),
                clonk_audio::test_encode_voice_frame(&bipolar_voice_frame()).unwrap(),
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
                clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence)).unwrap(),
            ));
        }

        assert!(jitter.insert(
            1,
            start + Duration::from_millis(170),
            clonk_audio::test_encode_voice_frame(&speech_voice_frame(1)).unwrap(),
        ));
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

        assert!(jitter.insert(
            0,
            start,
            clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap()
        ));
        assert!(jitter.insert(
            1,
            start + VOICE_FRAME_DURATION,
            clonk_audio::test_encode_voice_frame(&[1; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap(),
        ));
        assert!(jitter.insert(
            2,
            start + VOICE_FRAME_DURATION.saturating_mul(2),
            clonk_audio::test_encode_voice_frame(&[2; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap(),
        ));
        assert_eq!(jitter.target_frames(), 2);

        assert!(jitter.insert(
            3,
            start + VOICE_FRAME_DURATION.saturating_mul(6),
            clonk_audio::test_encode_voice_frame(&[3; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap(),
        ));
        assert_eq!(jitter.target_frames(), 4);
    }

    #[test]
    fn remote_voice_jitter_buffer_adapts_live_delay_within_the_latency_budget() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in 0..3 {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence)).unwrap(),
            ));
        }
        assert_eq!(jitter.target_frames(), 2);
        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(40), usize::MAX)
                .len(),
            3,
        );

        assert!(jitter.insert(
            3,
            start + Duration::from_millis(120),
            clonk_audio::test_encode_voice_frame(&speech_voice_frame(3)).unwrap(),
        ));
        assert_eq!(
            jitter.target_frames(),
            4,
            "changing network delay must update the live playout target",
        );
        for sequence in 4..200 {
            jitter.observe_arrival(
                sequence,
                start + Duration::from_millis(u64::from(sequence) * 20 + 60),
            );
        }
        assert_eq!(
            jitter.target_frames(),
            2,
            "a stable route must recover its low latency target"
        );
    }

    #[test]
    fn voice_playout_clock_keeps_latency_bounded_over_eight_hours_of_device_drift() {
        let start = Instant::now();
        for drift in [-0.0003, 0.0003] {
            let mut clock = VoicePlayoutClock::default();
            let interval = 0.02 * (1.0 + drift);
            let mut queued = 0.08_f64;
            let mut rate = 0;
            for frame in 0..1_440_000 {
                queued += 0.02 - interval * (1.0 + f64::from(rate) / 1_000_000.0);
                assert!(
                    queued > 0.0 && queued < 0.16,
                    "clock drift exhausted the latency budget: {queued}"
                );
                rate = clock.update(
                    start + Duration::from_secs_f64(f64::from(frame) * interval),
                    Duration::from_secs_f64(queued),
                    Duration::from_millis(80),
                );
            }
            assert!((queued - 0.08).abs() < 0.005, "latency drifted to {queued}");
            assert!((f64::from(rate) + drift * 1_000_000.0).abs() < 10.0);
        }
    }

    #[test]
    fn voice_delay_changes_use_bounded_gradual_clock_corrections() {
        let start = Instant::now();
        let mut clock = VoicePlayoutClock::default();
        let mut queued = 0.04_f64;
        let mut rate = 0;
        for frame in 0..1_000 {
            let target = if frame < 500 { 120 } else { 40 };
            queued += 0.02 - 0.02 * (1.0 + f64::from(rate) / 1_000_000.0);
            let next = clock.update(
                start + Duration::from_millis(frame * 20),
                Duration::from_secs_f64(queued),
                Duration::from_millis(target),
            );
            assert!(next.abs() <= 10_000);
            assert!((next - rate).abs() <= 800, "playout rate changed abruptly");
            assert!(
                (0.03..0.14).contains(&queued),
                "delay adjustment created a gap or excessive backlog"
            );
            rate = next;
            if frame == 499 {
                assert!(queued > 0.105, "delay did not grow smoothly: {queued}");
            }
        }
        assert!(
            queued < 0.055,
            "delay did not return toward its low latency target: {queued}"
        );
    }

    #[test]
    fn jitter_estimates_survive_eight_hours_of_wire_sequence_wrap() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for frame in 0..1_440_000_u64 {
            jitter.observe_arrival(frame as u16, start + Duration::from_micros(frame * 20_004));
            if frame > 2 {
                assert_eq!(
                    jitter.target_frames(),
                    2,
                    "a wrapped sequence looked like a network delay spike at {frame}"
                );
            }
        }
    }

    #[test]
    fn late_authenticated_voice_adapts_delay_without_replaying_concealed_audio() {
        let start = Instant::now();
        let payload =
            clonk_audio::test_encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES])
                .unwrap();
        let mut voice = VoiceChatState::default();
        let packet = |sequence| clonk_network::VoiceFrame {
            client_id: 7,
            player_id: 17,
            stream_epoch: 1,
            sequence,
            payload: payload.to_vec(),
        };
        for sequence in 0..3 {
            assert!(voice
                .accept_authorized_remote_frame(
                    &packet(sequence),
                    start + Duration::from_millis(u64::from(sequence) * 20)
                )
                .is_some());
        }
        assert_eq!(
            voice
                .drain_remote_playout(7, 17, start + Duration::from_millis(40), 8, 0)
                .len(),
            3
        );
        assert_eq!(
            voice.drain_remote_playout(7, 17, start + Duration::from_millis(100), 8, 0)[0].sequence,
            3
        );
        assert!(voice
            .accept_authorized_remote_frame(&packet(3), start + Duration::from_millis(120))
            .is_none());
        let stats = voice.remote_playout_stats(7, 17);
        assert_eq!(
            stats.target_frames, 4,
            "late packets must still inform the delay estimator"
        );
        assert_eq!(stats.late_frames, 1);
        assert!(voice
            .accept_authorized_remote_frame(&packet(3), start + Duration::from_millis(140))
            .is_none());
        assert_eq!(
            voice.remote_playout_stats(7, 17).late_frames,
            1,
            "a repeated late packet is not another loss"
        );
    }

    #[test]
    fn stateful_voice_decode_follows_playout_order_and_recovers_a_lost_interval() {
        let start = Instant::now();
        let mut encoder = clonk_audio::VoiceEncoder::new().unwrap();
        let packets = (0..6)
            .map(|sequence| encoder.encode(&speech_voice_frame(sequence)).unwrap())
            .collect::<Vec<_>>();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for (sequence, arrival_ms) in [(0, 0), (2, 21), (1, 52), (3, 65), (5, 100)] {
            assert!(jitter.insert(
                sequence,
                start + Duration::from_millis(arrival_ms),
                packets[usize::from(sequence)]
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
            ]
        );
        let mut reference = clonk_audio::VoiceDecoder::new().unwrap();
        for (sequence, frame) in ready.iter().enumerate() {
            let packet = packets[if sequence == 4 { 5 } else { sequence }];
            assert_eq!(
                frame.samples,
                reference.decode(&packet, sequence == 4).unwrap()
            );
        }
    }

    #[test]
    fn concealed_voice_preserves_energy_across_zero_crossing_boundaries() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        let mut encoder = clonk_audio::VoiceEncoder::new().unwrap();
        for sequence in 0..6 {
            let packet = encoder.encode(&speech_voice_frame(sequence)).unwrap();
            if sequence == 4 {
                continue;
            }
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                packet,
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
        assert!(concealed
            .windows(2)
            .all(|pair| i32::from(pair[1]).abs_diff(i32::from(pair[0])) <= 1_000));
    }

    #[test]
    fn isolated_voice_loss_is_concealed_before_buffered_playout_runs_dry() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for (sequence, arrival_ms) in [(0, 0), (1, 20), (2, 40), (3, 60), (5, 80)] {
            assert!(jitter.insert(
                sequence,
                start + Duration::from_millis(arrival_ms),
                clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence)).unwrap(),
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
        assert!(
            jitter
                .drain_ready_with_headroom(start + Duration::from_millis(100), usize::MAX, 2)
                .is_empty(),
            "two buffered frames leave time for a reordered packet"
        );
        assert_eq!(
            jitter
                .drain_ready_with_headroom(start + Duration::from_millis(120), usize::MAX, 1)
                .into_iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            vec![(4, true), (5, false)],
        );
    }

    #[test]
    fn authenticated_voice_end_stops_concealment_and_rejects_audio_past_the_end() {
        let start = Instant::now();
        let mut state = VoiceChatState::default();
        for sequence in 0..3 {
            let mut frame = clonk_network::VoiceFrame::outbound(
                17,
                1,
                sequence,
                clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence))
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            frame.client_id = 0;
            assert!(state
                .accept_authorized_remote_frame(
                    &frame,
                    start + VOICE_FRAME_DURATION * u32::from(sequence)
                )
                .is_some());
        }
        let mut end = clonk_network::VoiceFrame::outbound(17, 1, 3, Vec::new()).unwrap();
        end.client_id = 0;
        assert!(state
            .accept_authorized_remote_frame(&end, start + Duration::from_millis(60))
            .is_some());
        assert_eq!(
            state
                .drain_remote_playout(0, 17, start + Duration::from_millis(60), 8, 0)
                .len(),
            3
        );
        assert!(state
            .drain_remote_playout(0, 17, start + Duration::from_millis(200), 8, 0)
            .is_empty());
        let mut late = clonk_network::VoiceFrame::outbound(
            17,
            1,
            4,
            clonk_audio::test_encode_voice_frame(&speech_voice_frame(4))
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        late.client_id = 0;
        assert!(state
            .accept_authorized_remote_frame(&late, start + Duration::from_millis(200))
            .is_none());
    }

    #[test]
    fn missing_voice_tail_uses_a_bounded_playout_clock_without_a_successor() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in 0..3 {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION * u32::from(sequence),
                clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence)).unwrap(),
            ));
        }
        assert_eq!(
            jitter
                .drain_ready(start + Duration::from_millis(40), 8)
                .len(),
            3
        );
        assert!(jitter
            .drain_ready(start + Duration::from_millis(99), 8)
            .is_empty());
        for (sequence, millis) in [(3, 100), (4, 120), (5, 140)] {
            let ready = jitter.drain_ready(start + Duration::from_millis(millis), 8);
            assert_eq!(
                ready
                    .iter()
                    .map(|f| (f.sequence, f.concealed))
                    .collect::<Vec<_>>(),
                vec![(sequence, true)]
            );
            assert!(jitter
                .drain_ready(start + Duration::from_millis(millis), 8)
                .is_empty());
        }
        assert!(
            jitter
                .drain_ready(start + Duration::from_secs(1), 8)
                .is_empty(),
            "an ended or disconnected speaker cannot generate unbounded artificial audio"
        );
    }

    #[test]
    fn consecutive_voice_loss_preserves_each_missing_interval() {
        let start = Instant::now();
        let mut jitter = RemoteVoiceJitterBuffer::default();
        for sequence in [0, 1, 2, 3, 6] {
            assert!(jitter.insert(
                sequence,
                start + VOICE_FRAME_DURATION.saturating_mul(u32::from(sequence)),
                clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence)).unwrap(),
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
                    VOICE_PLAYOUT_GUARD_FRAMES - 1,
                )
                .into_iter()
                .map(|frame| (frame.sequence, frame.concealed))
                .collect::<Vec<_>>(),
            vec![(4, true), (5, true), (6, false)],
        );
        assert!(jitter.insert(
            7,
            start + Duration::from_millis(140),
            clonk_audio::test_encode_voice_frame(&speech_voice_frame(7)).unwrap(),
        ));
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
                clonk_audio::test_encode_voice_frame(&bipolar_voice_frame()).unwrap(),
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
            clonk_audio::test_encode_voice_frame(&bipolar_voice_frame()).unwrap(),
        ));
        assert!(!jitter.insert(
            5,
            start + Duration::from_millis(280),
            clonk_audio::test_encode_voice_frame(&bipolar_voice_frame()).unwrap(),
        ));
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
                clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence)).unwrap(),
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
    fn voice_context_changes_discard_playback_but_preserve_replay_tombstones() {
        let start = Instant::now();
        let mut voice = VoiceChatState::default();
        let frame = |stream_epoch, sequence| clonk_network::VoiceFrame {
            client_id: 7,
            player_id: LOBBY_VOICE_PLAYER_ID,
            stream_epoch,
            sequence,
            payload: clonk_audio::test_encode_voice_frame(
                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .unwrap()
            .to_vec(),
        };

        assert!(voice
            .reconcile_context(Some(VoiceChatContext::Lobby))
            .is_empty());
        assert!(voice
            .accept_authorized_remote_frame(&frame(5, 100), start)
            .is_some());
        assert_eq!(
            voice.reconcile_context(Some(VoiceChatContext::Running)),
            vec![(7, LOBBY_VOICE_PLAYER_ID)],
        );
        assert!(voice.active_speakers(start).is_empty());
        assert!(voice
            .reconcile_context(Some(VoiceChatContext::Lobby))
            .is_empty());
        assert!(voice
            .accept_authorized_remote_frame(&frame(5, 100), start)
            .is_none());
        assert!(voice
            .accept_authorized_remote_frame(&frame(5, 101), start)
            .is_none());
        assert!(voice
            .accept_authorized_remote_frame(&frame(6, 0), start)
            .is_some());
    }

    #[test]
    fn inactive_voice_context_discards_playback_but_preserves_replay_tombstones() {
        let start = Instant::now();
        let mut voice = VoiceChatState::default();
        let frame = |stream_epoch, sequence| clonk_network::VoiceFrame {
            client_id: 7,
            player_id: LOBBY_VOICE_PLAYER_ID,
            stream_epoch,
            sequence,
            payload: clonk_audio::test_encode_voice_frame(
                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .unwrap()
            .to_vec(),
        };

        assert!(voice
            .reconcile_context(Some(VoiceChatContext::Lobby))
            .is_empty());
        assert!(voice
            .accept_authorized_remote_frame(&frame(5, 100), start)
            .is_some());
        assert_eq!(
            voice.reconcile_context(None),
            vec![(7, LOBBY_VOICE_PLAYER_ID)],
        );
        assert!(voice
            .reconcile_context(Some(VoiceChatContext::Lobby))
            .is_empty());
        assert!(voice
            .accept_authorized_remote_frame(&frame(5, 100), start)
            .is_none());
        assert!(voice
            .accept_authorized_remote_frame(&frame(5, 101), start)
            .is_none());
        assert!(voice
            .accept_authorized_remote_frame(&frame(6, 0), start)
            .is_some());
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
            payload: clonk_audio::test_encode_voice_frame(
                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .unwrap()
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
            payload: clonk_audio::test_encode_voice_frame(
                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .unwrap()
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
            payload: clonk_audio::test_encode_voice_frame(
                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .unwrap()
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
    fn capture_overflow_preserves_missing_sample_intervals_on_the_wire() {
        let now = Instant::now();
        let mut voice = VoiceChatState::with_capture_opener(move |_| {
            let payload =
                clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES])
                    .unwrap();
            Ok(TestVoiceSource {
                frames: RefCell::new(
                    [0, 3]
                        .map(|index| {
                            VoiceInputFrame::test_frame(payload, 1.0).with_test_timing(
                                clonk_audio::VoiceCaptureTiming {
                                    captured_at: now,
                                    sample_offset: index * clonk_audio::VOICE_FRAME_SAMPLES as u64,
                                },
                            )
                        })
                        .to_vec(),
                ),
            })
        });
        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
            .unwrap();
        assert_eq!(
            voice
                .drain_captured_frames(None)
                .into_iter()
                .map(|frame| (frame.sequence, frame.captured_at))
                .collect::<Vec<_>>(),
            vec![(0, now), (3, now)]
        );
    }

    #[test]
    fn capture_recovers_with_a_new_epoch_after_a_gap_beyond_the_receive_window() {
        let payload =
            clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap();
        let mut voice =
            VoiceChatState::with_capture_opener(|_| Ok(TestVoiceSource::with_levels(&[])));
        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
            .unwrap();
        let stamps = [0, 100, 101, 65_637].map(|index| {
            let frame = voice.stamp_captured_frame(
                VoiceInputFrame::test_frame(payload, 1.0).with_test_timing(
                    clonk_audio::VoiceCaptureTiming {
                        captured_at: Instant::now(),
                        sample_offset: index * clonk_audio::VOICE_FRAME_SAMPLES as u64,
                    },
                ),
                false,
            );
            (frame.stream_epoch, frame.sequence)
        });
        assert_eq!(
            stamps,
            [(1, 0), (2, 0), (2, 1), (3, 0)],
            "a local capture stall must not leave the receiver rejecting the rest of the utterance"
        );
    }

    #[test]
    fn push_to_talk_release_sends_captured_speech_then_one_end_marker() {
        let mut voice =
            VoiceChatState::with_capture_opener(|_| Ok(TestVoiceSource::with_levels(&[0.8, 0.7])));
        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
            .unwrap();
        voice.finish_capture_at(Instant::now());
        assert!(!voice.capture_active());
        assert!(voice.capture_key().is_none());
        let frames = voice.drain_captured_frames(None);
        assert_eq!(
            frames
                .iter()
                .map(|f| (f.stream_epoch, f.sequence, f.payload.is_some()))
                .collect::<Vec<_>>(),
            vec![(1, 0, true), (1, 1, true), (1, 2, false)]
        );
        assert!(voice.drain_captured_frames(None).is_empty());
        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
            .unwrap();
        voice.finish_capture_at(Instant::now());
        voice.stop_capture();
        assert!(
            voice.drain_captured_frames(None).is_empty(),
            "privacy cancellation overrides graceful release"
        );
    }

    #[test]
    fn microphone_opens_only_for_an_explicit_capture_start() {
        let opens = Rc::new(Cell::new(0));
        let observed_opens = opens.clone();
        let mut voice = VoiceChatState::with_capture_opener(move |_| {
            observed_opens.set(observed_opens.get() + 1);
            Ok(TestVoiceSource::with_frame(
                clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES])
                    .unwrap(),
            ))
        });

        assert_eq!(opens.get(), 0);
        assert!(!voice.capture_active());

        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
            .unwrap();
        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
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
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
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
    fn failed_push_to_talk_open_keeps_the_key_owned_until_release() {
        let mut voice = VoiceChatState::with_capture_opener(
            |_| -> Result<TestVoiceSource, VoiceCaptureError> {
                Err(VoiceCaptureError::NoInputDevice)
            },
        );

        assert!(matches!(
            voice.start_capture(Some(winit::keyboard::KeyCode::Backquote), None),
            Err(VoiceCaptureError::NoInputDevice),
        ));
        assert_eq!(
            voice.capture_key(),
            Some(winit::keyboard::KeyCode::Backquote),
            "an unplugged microphone must not make a held push-to-talk key forget its release",
        );

        voice.stop_capture();
        assert_eq!(voice.capture_key(), None);
    }

    #[test]
    fn capture_opens_with_the_exact_selected_input_device() {
        let observed = Rc::new(RefCell::new(None));
        let observed_options = observed.clone();
        let mut voice = VoiceChatState::with_capture_opener(move |options| {
            *observed_options.borrow_mut() = options.input_device;
            Ok(TestVoiceSource::with_frame(
                clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES])
                    .unwrap(),
            ))
        });
        let selected = r#"coreaudio:USB #1 — \"room\""#
            .parse::<clonk_audio::VoiceInputDeviceId>()
            .expect("a persisted CPAL input device identity");

        voice
            .start_capture_on_device(
                Some(winit::keyboard::KeyCode::Backquote),
                None,
                Some(selected.clone()),
            )
            .expect("the injected microphone opens");

        assert_eq!(*observed.borrow(), Some(selected));
    }

    #[test]
    fn selecting_a_different_input_replaces_capture_without_forgetting_its_key() {
        struct DroppingVoiceSource {
            frame: RefCell<Option<VoiceInputFrame>>,
            drops: Rc<Cell<usize>>,
        }

        impl VoiceFrameSource for DroppingVoiceSource {
            fn drain_frames(&self) -> Vec<VoiceInputFrame> {
                self.frame.borrow_mut().take().into_iter().collect()
            }
        }

        impl Drop for DroppingVoiceSource {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }

        let first = "coreaudio:first"
            .parse::<VoiceInputDeviceId>()
            .expect("the first device identity");
        let second = "coreaudio:second"
            .parse::<VoiceInputDeviceId>()
            .expect("the second device identity");
        let opens = Rc::new(RefCell::new(Vec::new()));
        let observed_opens = opens.clone();
        let drops = Rc::new(Cell::new(0));
        let observed_drops = drops.clone();
        let mut voice = VoiceChatState::with_capture_opener(move |options| {
            observed_opens.borrow_mut().push(options.input_device);
            Ok(DroppingVoiceSource {
                frame: RefCell::new(Some(VoiceInputFrame::test_frame(
                    clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES])
                        .unwrap(),
                    1.0,
                ))),
                drops: observed_drops.clone(),
            })
        });

        voice
            .start_capture_on_device(
                Some(winit::keyboard::KeyCode::Backquote),
                None,
                Some(first.clone()),
            )
            .expect("first input opens");
        assert_eq!(
            voice
                .drain_captured_frames(None)
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>(),
            vec![(1, 0)],
        );

        voice
            .reconcile_capture_device(Some(second.clone()), None)
            .expect("replacement input opens");

        assert_eq!(opens.borrow().as_slice(), &[Some(first), Some(second)],);
        assert_eq!(
            drops.get(),
            1,
            "the replaced stream is dropped exactly once"
        );
        assert_eq!(
            voice.capture_key(),
            Some(winit::keyboard::KeyCode::Backquote),
        );
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
    fn voice_activation_retries_a_missing_input_only_after_the_bounded_delay() {
        let attempts = Rc::new(Cell::new(0));
        let observed_attempts = attempts.clone();
        let mut voice = VoiceChatState::with_capture_opener(move |_| {
            observed_attempts.set(observed_attempts.get() + 1);
            if observed_attempts.get() == 1 {
                Err(VoiceCaptureError::NoInputDevice)
            } else {
                Ok(TestVoiceSource::with_frame(
                    clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES])
                        .unwrap(),
                ))
            }
        });
        let start = Instant::now();

        assert!(matches!(
            voice.start_voice_activated_capture_on_device_at(None, None, start),
            Err(VoiceCaptureError::NoInputDevice),
        ));
        for elapsed in [
            Duration::from_millis(1),
            Duration::from_millis(100),
            Duration::from_millis(999),
        ] {
            voice
                .start_voice_activated_capture_on_device_at(None, None, start + elapsed)
                .expect("the retry remains latched before its deadline");
        }
        assert_eq!(attempts.get(), 1, "the microphone is not opened per tick");
        assert!(!voice.capture_active());

        voice
            .start_voice_activated_capture_on_device_at(None, None, start + Duration::from_secs(1))
            .expect("the bounded retry opens a reappeared microphone");
        assert_eq!(attempts.get(), 2);
        assert_eq!(
            voice
                .drain_captured_frames(None)
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>(),
            vec![(1, 0)],
            "failed attempts do not consume a media epoch",
        );
    }

    #[test]
    fn a_hotplug_reopen_starts_one_new_media_epoch() {
        struct ReopeningVoiceSource {
            generation: Rc<Cell<u64>>,
            frames: Rc<RefCell<Vec<VoiceInputFrame>>>,
        }

        impl VoiceFrameSource for ReopeningVoiceSource {
            fn drain_frames(&self) -> Vec<VoiceInputFrame> {
                std::mem::take(&mut *self.frames.borrow_mut())
            }

            fn stream_generation(&self) -> u64 {
                self.generation.get()
            }
        }

        let generation = Rc::new(Cell::new(1));
        let frames = Rc::new(RefCell::new(Vec::new()));
        let source_generation = generation.clone();
        let source_frames = frames.clone();
        let mut voice = VoiceChatState::with_capture_opener(move |_| {
            Ok(ReopeningVoiceSource {
                generation: source_generation.clone(),
                frames: source_frames.clone(),
            })
        });
        voice
            .start_capture(None, None)
            .expect("initial input opens");

        let mut capture_one = || {
            frames.borrow_mut().push(VoiceInputFrame::test_frame(
                clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES])
                    .unwrap(),
                1.0,
            ));
            voice
                .drain_captured_frames(None)
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>()
        };
        assert_eq!(capture_one(), vec![(1, 0)]);

        generation.set(2);
        assert_eq!(capture_one(), vec![(2, 0)]);
        assert_eq!(capture_one(), vec![(2, 1)]);
    }

    #[test]
    fn a_capture_opens_with_the_processing_the_player_has_configured() {
        let opened = Rc::new(RefCell::new(None));
        let observed = opened.clone();
        let mut voice = VoiceChatState::with_capture_opener(move |options| {
            *observed.borrow_mut() = Some(options.processing.clone());
            Ok(TestVoiceSource::with_frame(
                clonk_audio::test_encode_voice_frame(&[0; clonk_audio::VOICE_FRAME_SAMPLES])
                    .unwrap(),
            ))
        });
        let quiet_room = VoiceProcessingConfig {
            noise_suppression: false,
            ..VoiceProcessingConfig::default()
        };

        voice.set_processing(quiet_room);
        voice.start_capture(None, None).unwrap();

        let switches = opened.borrow().clone().expect("the capture opened");
        assert_eq!(switches.get(), quiet_room);

        voice.set_processing(VoiceProcessingConfig::DISABLED);
        assert_eq!(
            switches.get(),
            VoiceProcessingConfig::DISABLED,
            "the open capture reads the same switches, so a change reaches it",
        );
    }

    #[test]
    fn voice_activation_transmits_speech_with_a_release_tail_on_a_fresh_stream() {
        let levels = [0.1, 0.9, 0.1, 0.1, 0.1, 0.9];
        let mut voice =
            VoiceChatState::with_capture_opener(move |_| Ok(TestVoiceSource::with_levels(&levels)));
        let activation = VoiceActivation {
            threshold: 0.5,
            hangover_frames: 2,
        };

        voice.start_capture(None, None).unwrap();

        assert_eq!(
            voice
                .drain_captured_frames(Some(&activation))
                .into_iter()
                .map(|frame| (frame.stream_epoch, frame.sequence))
                .collect::<Vec<_>>(),
            vec![(1, 0), (1, 1), (1, 2), (1, 3), (1, 4), (1, 5), (2, 0)],
            "preroll precedes speech, hangover and codec tail precede the end marker, \
             and speaking again starts a new stream",
        );
    }

    #[test]
    fn voice_activation_keeps_sixty_milliseconds_of_preroll_and_ends_the_utterance() {
        let payload =
            clonk_audio::test_encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES])
                .unwrap();
        let start = Instant::now() - Duration::from_millis(120);
        let mut voice = VoiceChatState::with_capture_opener(move |_| {
            Ok(TestVoiceSource {
                frames: RefCell::new(
                    [0.0, 0.1, 0.2, 0.3, 0.9, 0.1, 0.1]
                        .into_iter()
                        .enumerate()
                        .map(|(i, level)| {
                            VoiceInputFrame::test_frame(payload, level).with_test_timing(
                                clonk_audio::VoiceCaptureTiming {
                                    captured_at: start + Duration::from_millis(i as u64 * 20),
                                    sample_offset: i as u64
                                        * clonk_audio::VOICE_FRAME_SAMPLES as u64,
                                },
                            )
                        })
                        .collect(),
                ),
            })
        });
        voice.start_capture(None, None).unwrap();
        let activation = VoiceActivation {
            threshold: 0.5,
            hangover_frames: 0,
        };
        let frames = voice.drain_captured_frames(Some(&activation));
        assert_eq!(
            frames
                .iter()
                .map(|f| (f.sequence, f.payload.is_some()))
                .collect::<Vec<_>>(),
            vec![
                (0, true),
                (1, true),
                (2, true),
                (3, true),
                (4, true),
                (5, false)
            ]
        );
        assert_eq!(frames[0].captured_at, start + Duration::from_millis(20));
        assert_eq!(frames[4].captured_at, start + Duration::from_millis(100));
        assert!(voice.drain_captured_frames(Some(&activation)).is_empty());
        voice.stop_capture();
        voice.start_capture(None, None).unwrap();
        assert!(
            voice
                .drain_captured_frames(Some(&VoiceActivation {
                    threshold: 1.0,
                    hangover_frames: 0
                }))
                .is_empty(),
            "silence alone never opens a stream"
        );
    }

    #[test]
    fn voice_activation_preroll_cannot_cross_a_privacy_cancellation() {
        let opens = Cell::new(0);
        let mut voice = VoiceChatState::with_capture_opener(move |_| {
            opens.set(opens.get() + 1);
            Ok(TestVoiceSource::with_levels(if opens.get() == 1 {
                &[0.1, 0.2]
            } else {
                &[0.9]
            }))
        });
        let activation = VoiceActivation {
            threshold: 0.5,
            hangover_frames: 0,
        };
        voice.start_capture(None, None).unwrap();
        assert!(voice.drain_captured_frames(Some(&activation)).is_empty());
        voice.stop_capture();
        voice.start_capture(None, None).unwrap();
        let frames = voice.drain_captured_frames(Some(&activation));
        assert_eq!(
            frames.len(),
            1,
            "audio retained before privacy revocation must be gone"
        );
        assert_eq!((frames[0].stream_epoch, frames[0].sequence), (2, 0));
    }

    #[test]
    fn push_to_talk_transmits_every_captured_frame_including_silence() {
        let levels = [0.0, 0.0];
        let mut voice =
            VoiceChatState::with_capture_opener(move |_| Ok(TestVoiceSource::with_levels(&levels)));

        voice
            .start_capture(Some(winit::keyboard::KeyCode::Backquote), None)
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
            VoiceChatState::with_capture_opener(move |_| Ok(TestVoiceSource::with_levels(&levels)));
        let activation = VoiceActivation {
            threshold: 0.0,
            hangover_frames: 0,
        };

        voice.start_capture(None, None).unwrap();

        assert_eq!(voice.drain_captured_frames(Some(&activation)).len(), 2);
    }

    #[test]
    fn stopping_a_capture_closes_the_activation_gate_behind_it() {
        let opens = Cell::new(0);
        let mut voice = VoiceChatState::with_capture_opener(move |_| {
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

        voice.start_capture(None, None).unwrap();
        assert_eq!(voice.drain_captured_frames(Some(&activation)).len(), 1);
        voice.stop_capture();
        voice.start_capture(None, None).unwrap();

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
            payload: clonk_audio::test_encode_voice_frame(
                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .unwrap()
            .to_vec(),
            ..malformed
        };
        let accepted = voice
            .accept_remote_frame(&snapshot, &valid, now)
            .expect("the malformed frame did not consume its sequence");
        assert_eq!(accepted.stream_id, voice_stream_id(7, 17));
        assert!(!accepted.reset_stream);
        let playout = voice.drain_remote_playout(7, 17, now + Duration::from_millis(100), 1, 0);
        assert_eq!(playout.len(), 1);
        assert!(playout[0]
            .samples
            .iter()
            .any(|sample| sample.unsigned_abs() > 100));
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
            payload: clonk_audio::test_encode_voice_frame(
                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
            )
            .unwrap()
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
                payload: clonk_audio::test_encode_voice_frame(
                    &[sequence as i16; clonk_audio::VOICE_FRAME_SAMPLES],
                )
                .unwrap()
                .to_vec(),
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
                payload: clonk_audio::test_encode_voice_frame(&speech_voice_frame(sequence))
                    .unwrap()
                    .to_vec(),
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
        assert!(voice
            .drain_remote_playout(7, 17, start + Duration::from_millis(120), 2, 2)
            .is_empty());
        drained.extend(voice.drain_remote_playout(7, 17, start + Duration::from_millis(140), 2, 1));
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
            payload: clonk_audio::test_encode_voice_frame(&speech_voice_frame(4))
                .unwrap()
                .to_vec(),
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

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5"),
))]
#[path = "voice_network_qualification.rs"]
mod network_qualification;
