use crate::decoder::AudioDecodeError;
use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

const DEFAULT_TEMPO_MICROS_PER_QUARTER: u64 = 500_000;
const MICROS_PER_SECOND: u128 = 1_000_000;
const TAIL_SECONDS: u64 = 2;
// These limits bound parsing and the owned event schedule, independently of how
// long the MIDI takes to play. PCM is synthesized incrementally by `MidiStream`.
// Track count bounds merge/sort work, input bytes bound the parser's borrowed
// structure, event count bounds the owned schedule, and SysEx bounds one copy.
const MAX_TRACKS: usize = 128;
const MAX_MIDI_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCHEDULED_EVENTS: usize = 1_000_000;
const MAX_SYSEX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MidiTimeline {
    pub events: Vec<TimedMidiEvent>,
    pub body_end_frame: usize,
    pub end_frame: usize,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimedMidiEvent {
    pub frame: usize,
    pub command: MidiCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MidiCommand {
    NoteOff {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    Aftertouch {
        channel: u8,
        key: u8,
        pressure: u8,
    },
    Controller {
        channel: u8,
        controller: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    ChannelAftertouch {
        channel: u8,
        pressure: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    SysEx(Vec<u8>),
}

#[derive(Clone, Copy)]
struct OrderedEvent<'a> {
    tick: u64,
    track_index: usize,
    event_index: usize,
    kind: TrackEventKind<'a>,
}

pub(crate) fn parse_timeline(
    data: &[u8],
    sample_rate: u32,
) -> Result<MidiTimeline, AudioDecodeError> {
    if sample_rate == 0 {
        return Err(invalid("MIDI output sample rate is zero"));
    }
    validate_input_len(data.len())?;

    let smf = Smf::parse(data).map_err(|_| invalid("invalid MIDI file"))?;
    match smf.header.format {
        Format::SingleTrack | Format::Parallel => {}
        Format::Sequential => return Err(invalid("MIDI format 2 is not supported")),
    }
    if smf.tracks.len() > MAX_TRACKS {
        return Err(invalid("MIDI has too many tracks"));
    }
    let ticks_per_quarter = match smf.header.timing {
        Timing::Metrical(value) if value.as_int() != 0 => u64::from(value.as_int()),
        Timing::Metrical(_) => return Err(invalid("MIDI ticks per quarter is zero")),
        Timing::Timecode(_, _) => return Err(invalid("SMPTE MIDI timing is not supported")),
    };

    let mut ordered = collect_events(&smf)?;
    ordered.sort_by_key(|event| (event.tick, event.track_index, event.event_index));
    schedule_events(&ordered, ticks_per_quarter, sample_rate)
}

fn validate_input_len(len: usize) -> Result<(), AudioDecodeError> {
    if len > MAX_MIDI_BYTES {
        return Err(invalid("MIDI exceeds 8 MiB parser input limit"));
    }
    Ok(())
}

fn collect_events<'a>(smf: &'a Smf<'a>) -> Result<Vec<OrderedEvent<'a>>, AudioDecodeError> {
    let mut ordered = Vec::new();
    let event_count = smf.tracks.iter().try_fold(0_usize, |count, track| {
        let track_count = track
            .iter()
            .position(|event| matches!(event.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)))
            .map_or(track.len(), |index| index + 1);
        checked_event_total(count, track_count)
    })?;
    ordered
        .try_reserve_exact(event_count)
        .map_err(|_| invalid("MIDI event schedule is too large"))?;
    for (track_index, track) in smf.tracks.iter().enumerate() {
        let mut tick = 0_u64;
        for (event_index, event) in track.iter().enumerate() {
            tick = checked_tick_position(tick, u64::from(event.delta.as_int()))?;
            ordered.push(OrderedEvent {
                tick,
                track_index,
                event_index,
                kind: event.kind,
            });
            if matches!(event.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)) {
                break;
            }
        }
    }
    Ok(ordered)
}

fn checked_event_total(current: usize, additional: usize) -> Result<usize, AudioDecodeError> {
    current
        .checked_add(additional)
        .filter(|total| *total <= MAX_SCHEDULED_EVENTS)
        .ok_or_else(|| invalid("MIDI has too many events to schedule"))
}

fn schedule_events(
    ordered: &[OrderedEvent<'_>],
    ticks_per_quarter: u64,
    sample_rate: u32,
) -> Result<MidiTimeline, AudioDecodeError> {
    let denominator = u128::from(ticks_per_quarter)
        .checked_mul(MICROS_PER_SECOND)
        .ok_or_else(|| invalid("MIDI timing denominator overflow"))?;
    let tail_frames = u64::from(sample_rate)
        .checked_mul(TAIL_SECONDS)
        .ok_or_else(|| invalid("MIDI tail duration overflow"))?;
    let mut events = Vec::new();
    events
        .try_reserve_exact(ordered.len())
        .map_err(|_| invalid("MIDI event output is too large"))?;
    let mut last_tick = 0_u64;
    let mut elapsed_numerator = 0_u128;
    let mut tempo = DEFAULT_TEMPO_MICROS_PER_QUARTER;
    let mut event_end_frame = 0_u64;

    for event in ordered {
        let delta = event
            .tick
            .checked_sub(last_tick)
            .ok_or_else(|| invalid("MIDI events are out of order"))?;
        elapsed_numerator =
            checked_elapsed_numerator(elapsed_numerator, delta, tempo, sample_rate)?;
        event_end_frame = rounded_frame(elapsed_numerator, denominator)?;
        last_tick = event.tick;

        match event.kind {
            TrackEventKind::Midi { channel, message } => events.push(TimedMidiEvent {
                frame: usize::try_from(event_end_frame)
                    .map_err(|_| invalid("MIDI frame index exceeds platform limits"))?,
                command: channel_command(channel.as_int(), message)?,
            }),
            TrackEventKind::Meta(MetaMessage::Tempo(value)) => {
                tempo = u64::from(value.as_int());
                if tempo == 0 {
                    return Err(invalid("MIDI tempo is zero"));
                }
            }
            TrackEventKind::SysEx(data) => events.push(TimedMidiEvent {
                frame: usize::try_from(event_end_frame)
                    .map_err(|_| invalid("MIDI frame index exceeds platform limits"))?,
                command: sysex_command(data)?,
            }),
            TrackEventKind::Escape(_) => {
                return Err(invalid("MIDI escape events are not supported"))
            }
            TrackEventKind::Meta(_) => {}
        }
    }

    let end_frame = event_end_frame
        .checked_add(tail_frames)
        .ok_or_else(|| invalid("MIDI output duration overflow"))?;
    Ok(MidiTimeline {
        events,
        body_end_frame: usize::try_from(event_end_frame)
            .map_err(|_| invalid("MIDI duration exceeds platform limits"))?,
        end_frame: usize::try_from(end_frame)
            .map_err(|_| invalid("MIDI duration exceeds platform limits"))?,
        sample_rate,
    })
}

fn rounded_frame(numerator: u128, denominator: u128) -> Result<u64, AudioDecodeError> {
    let rounded = numerator
        .checked_add(denominator / 2)
        .ok_or_else(|| invalid("MIDI frame rounding overflow"))?
        / denominator;
    u64::try_from(rounded).map_err(|_| invalid("MIDI frame position overflow"))
}

fn checked_tick_position(current: u64, delta: u64) -> Result<u64, AudioDecodeError> {
    current
        .checked_add(delta)
        .ok_or_else(|| invalid("MIDI tick position overflow"))
}

fn checked_elapsed_numerator(
    elapsed: u128,
    delta: u64,
    tempo: u64,
    sample_rate: u32,
) -> Result<u128, AudioDecodeError> {
    let interval = u128::from(delta)
        .checked_mul(u128::from(tempo))
        .and_then(|value| value.checked_mul(u128::from(sample_rate)))
        .ok_or_else(|| invalid("MIDI frame calculation overflow"))?;
    elapsed
        .checked_add(interval)
        .ok_or_else(|| invalid("MIDI elapsed time overflow"))
}

fn channel_command(channel: u8, message: MidiMessage) -> Result<MidiCommand, AudioDecodeError> {
    match message {
        MidiMessage::NoteOff { key, vel } => Ok(MidiCommand::NoteOff {
            channel,
            key: key.as_int(),
            velocity: vel.as_int(),
        }),
        MidiMessage::NoteOn { key, vel } if vel.as_int() == 0 => Ok(MidiCommand::NoteOff {
            channel,
            key: key.as_int(),
            velocity: 0,
        }),
        MidiMessage::NoteOn { key, vel } => Ok(MidiCommand::NoteOn {
            channel,
            key: key.as_int(),
            velocity: vel.as_int(),
        }),
        MidiMessage::Aftertouch { key, vel } => Ok(MidiCommand::Aftertouch {
            channel,
            key: key.as_int(),
            pressure: vel.as_int(),
        }),
        MidiMessage::Controller { controller, value } => Ok(MidiCommand::Controller {
            channel,
            controller: controller.as_int(),
            value: value.as_int(),
        }),
        MidiMessage::ProgramChange { program } => Ok(MidiCommand::ProgramChange {
            channel,
            program: program.as_int(),
        }),
        MidiMessage::ChannelAftertouch { vel } => Ok(MidiCommand::ChannelAftertouch {
            channel,
            pressure: vel.as_int(),
        }),
        MidiMessage::PitchBend { bend } => Ok(MidiCommand::PitchBend {
            channel,
            value: bend.0.as_int(),
        }),
    }
}

fn sysex_command(data: &[u8]) -> Result<MidiCommand, AudioDecodeError> {
    let (terminator, payload) = data
        .split_last()
        .ok_or_else(|| invalid("MIDI SysEx event is empty or split"))?;
    if *terminator != 0xF7 {
        return Err(invalid("split MIDI SysEx is not supported"));
    }
    if payload.iter().any(|byte| byte & 0x80 != 0) {
        return Err(invalid("MIDI SysEx payload contains a status byte"));
    }
    if payload.len() > MAX_SYSEX_BYTES {
        return Err(invalid("MIDI SysEx payload is too large to schedule"));
    }
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(payload.len())
        .map_err(|_| invalid("MIDI SysEx payload is too large"))?;
    owned.extend_from_slice(payload);
    Ok(MidiCommand::SysEx(owned))
}

fn invalid(message: &'static str) -> AudioDecodeError {
    AudioDecodeError::InvalidData(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smf(format: u16, division: u16, tracks: &[&[u8]]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6_u32.to_be_bytes());
        bytes.extend_from_slice(&format.to_be_bytes());
        bytes.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&division.to_be_bytes());
        for track in tracks {
            bytes.extend_from_slice(b"MTrk");
            bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
            bytes.extend_from_slice(track);
        }
        bytes
    }

    fn rmid(midi: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(12_u32 + midi.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"RMIDdata");
        bytes.extend_from_slice(&(midi.len() as u32).to_le_bytes());
        bytes.extend_from_slice(midi);
        bytes
    }

    #[test]
    fn default_tempo_schedules_one_quarter_note_and_tail() {
        // C++ parity oracle: C4MusicSystem.cpp:31-32,101-113 accepts `.mid` and passes
        // its bytes through; C4AudioSystemSdl.cpp:280-282 delegates them to SDL_mixer.
        let midi = smf(
            0,
            480,
            &[&[
                0x00, 0x90, 60, 64, // tick 0: note on
                0x83, 0x60, 0x80, 60, 64, // tick 480: note off
                0x00, 0xFF, 0x2F, 0x00, // tick 480: end of track
            ]],
        );

        let timeline = parse_timeline(&midi, 44_100).expect("valid test MIDI");

        assert_eq!(timeline.events[0].frame, 0);
        assert_eq!(timeline.events[1].frame, 22_050);
        assert_eq!(timeline.body_end_frame, 22_050);
        assert_eq!(timeline.end_frame, 22_050 + 88_200);
    }

    #[test]
    fn parses_riff_wrapped_midi() {
        // C4AudioSystemSdl.cpp:280-282 delegates RMID parsing to SDL_mixer.
        let midi = smf(0, 480, &[&[0x00, 0xFF, 0x2F, 0x00]]);

        let timeline = parse_timeline(&rmid(&midi), 1_000).expect("valid RMID");

        assert!(timeline.events.is_empty());
        assert_eq!(timeline.end_frame, 2_000);
    }

    #[test]
    fn maps_every_channel_command_to_owned_values() {
        let midi = smf(
            0,
            480,
            &[&[
                0x00, 0xA2, 61, 62, // polyphonic aftertouch
                0x00, 0xB3, 7, 100, // controller
                0x00, 0xC4, 10, // program change
                0x00, 0xD5, 44, // channel aftertouch
                0x00, 0xE6, 0, 64, // centered pitch bend (8192)
                0x00, 0x97, 64, 0, // velocity-zero note on is note off
                0x00, 0xFF, 0x2F, 0x00,
            ]],
        );

        let timeline = parse_timeline(&midi, 44_100).expect("valid test MIDI");

        assert_eq!(
            timeline
                .events
                .into_iter()
                .map(|event| event.command)
                .collect::<Vec<_>>(),
            vec![
                MidiCommand::Aftertouch {
                    channel: 2,
                    key: 61,
                    pressure: 62,
                },
                MidiCommand::Controller {
                    channel: 3,
                    controller: 7,
                    value: 100,
                },
                MidiCommand::ProgramChange {
                    channel: 4,
                    program: 10,
                },
                MidiCommand::ChannelAftertouch {
                    channel: 5,
                    pressure: 44,
                },
                MidiCommand::PitchBend {
                    channel: 6,
                    value: 8_192,
                },
                MidiCommand::NoteOff {
                    channel: 7,
                    key: 64,
                    velocity: 0,
                },
            ]
        );
    }

    #[test]
    fn maps_complete_sysex_to_owned_payload_without_terminator() {
        let midi = smf(
            0,
            480,
            &[&[
                0x00, 0xF0, 0x04, 0x7E, 0x7F, 0x09, 0xF7, // complete GM-system SysEx
                0x00, 0xFF, 0x2F, 0x00,
            ]],
        );

        let timeline = parse_timeline(&midi, 44_100).expect("valid test MIDI");

        assert_eq!(
            timeline.events[0].command,
            MidiCommand::SysEx(vec![0x7E, 0x7F, 0x09])
        );
    }

    #[test]
    fn tempo_change_affects_only_following_tick_intervals() {
        let tempo_track = [
            0x83, 0x60, 0xFF, 0x51, 0x03, 0x03, 0xD0, 0x90, // tick 480: 250,000 us/qn
            0x83, 0x60, 0xFF, 0x2F, 0x00, // tick 960: end
        ];
        let note_track = [
            0x00, 0x90, 60, 64, // tick 0
            0x83, 0x60, 0x90, 61, 64, // tick 480
            0x83, 0x60, 0x90, 62, 64, // tick 960
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let midi = smf(1, 480, &[&tempo_track, &note_track]);

        let timeline = parse_timeline(&midi, 44_100).expect("valid test MIDI");
        let frames = timeline
            .events
            .iter()
            .map(|event| event.frame)
            .collect::<Vec<_>>();

        assert_eq!(frames, vec![0, 22_050, 33_075]);
    }

    #[test]
    fn fractional_tick_intervals_do_not_accumulate_rounding_drift() {
        let midi = smf(
            0,
            3,
            &[&[
                0x01, 0x90, 60, 64, // tick 1
                0x01, 0x90, 61, 64, // tick 2
                0x01, 0x90, 62, 64, // tick 3
                0x00, 0xFF, 0x2F, 0x00,
            ]],
        );

        let timeline = parse_timeline(&midi, 10).expect("valid test MIDI");
        let frames = timeline
            .events
            .iter()
            .map(|event| event.frame)
            .collect::<Vec<_>>();

        assert_eq!(frames, vec![2, 3, 5]);
    }

    #[test]
    fn same_tick_events_merge_by_track_then_event_order() {
        let track_zero = [
            0x0A, 0xC0, 1, // tick 10, event 0
            0x00, 0xC0, 2, // tick 10, event 1
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let track_one = [
            0x0A, 0xC1, 3, // tick 10, event 0
            0x00, 0xC1, 4, // tick 10, event 1
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let midi = smf(1, 480, &[&track_zero, &track_one]);

        let timeline = parse_timeline(&midi, 44_100).expect("valid test MIDI");
        let programs = timeline
            .events
            .iter()
            .map(|event| match event.command {
                MidiCommand::ProgramChange { channel, program } => (channel, program),
                _ => unreachable!("test emits only program changes"),
            })
            .collect::<Vec<_>>();

        assert_eq!(programs, vec![(0, 1), (0, 2), (1, 3), (1, 4)]);
        assert!(timeline
            .events
            .windows(2)
            .all(|events| events[0].frame == events[1].frame));
    }

    #[test]
    fn track_stops_at_first_end_of_track_event() {
        let midi = smf(
            0,
            100,
            &[&[
                0x00, 0x90, 60, 64, 0x0A, 0xFF, 0x2F, 0x00, // tick 10: first EOT
                0x64, 0x90, 61, 64, // tick 110: must be ignored
            ]],
        );

        let timeline = parse_timeline(&midi, 1_000).expect("valid test MIDI");

        assert_eq!(timeline.events.len(), 1);
        assert_eq!(timeline.end_frame, 50 + 2_000);
    }

    #[test]
    fn rejects_split_sysex_and_escape_events() {
        let split = smf(
            0,
            480,
            &[&[0x00, 0xF0, 0x02, 0x01, 0x02, 0x00, 0xFF, 0x2F, 0x00]],
        );
        let escaped = smf(
            0,
            480,
            &[&[0x00, 0xF7, 0x02, 0x01, 0x02, 0x00, 0xFF, 0x2F, 0x00]],
        );

        assert!(parse_timeline(&split, 44_100).is_err());
        assert!(parse_timeline(&escaped, 44_100).is_err());
    }

    #[test]
    fn rejects_format_two_smpte_and_zero_ppqn() {
        let eot = [0x00, 0xFF, 0x2F, 0x00];
        let format_two = smf(2, 480, &[&eot]);
        let smpte = smf(0, 0xE7_28, &[&eot]);
        let zero_ppqn = smf(0, 0, &[&eot]);

        assert!(parse_timeline(&format_two, 44_100).is_err());
        assert!(parse_timeline(&smpte, 44_100).is_err());
        assert!(parse_timeline(&zero_ppqn, 44_100).is_err());
    }

    #[test]
    fn rejects_more_than_128_tracks() {
        let eot = [0x00, 0xFF, 0x2F, 0x00];
        let tracks = vec![eot.as_slice(); 129];
        let midi = smf(1, 480, &tracks);

        assert!(parse_timeline(&midi, 44_100).is_err());
    }

    #[test]
    fn rejects_zero_tempo_and_output_sample_rate() {
        let zero_tempo = smf(
            0,
            480,
            &[&[
                0x00, 0xFF, 0x51, 0x03, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x2F, 0x00,
            ]],
        );
        let ordinary = smf(0, 480, &[&[0x00, 0xFF, 0x2F, 0x00]]);

        assert!(parse_timeline(&zero_tempo, 44_100).is_err());
        assert!(parse_timeline(&ordinary, 0).is_err());
    }

    #[test]
    fn schedules_midi_beyond_the_former_fifteen_minute_limit() {
        // C4AudioSystemSdl.cpp:280-282 streams through SDL_mixer, so playback
        // duration is not a parser or event-schedule allocation limit.
        let long_midi = smf(
            0,
            1,
            &[&[0x8E, 0x09, 0xFF, 0x2F, 0x00]], // tick 1801: 900.5 s + tail
        );

        let timeline = parse_timeline(&long_midi, 1_000).expect("long MIDI timeline");

        assert_eq!(timeline.body_end_frame, 900_500);
        assert_eq!(timeline.end_frame, 902_500);
    }

    #[test]
    fn caps_owned_midi_event_scheduling() {
        // Streaming still owns a compact command for every event, so this bound
        // protects the schedule allocation rather than the output duration.
        assert_eq!(
            checked_event_total(MAX_SCHEDULED_EVENTS - 1, 1).unwrap(),
            MAX_SCHEDULED_EVENTS
        );
        assert!(checked_event_total(MAX_SCHEDULED_EVENTS, 1).is_err());
    }

    #[test]
    fn caps_midi_input_parsing() {
        // Streaming still parses the complete file and bounds that untrusted
        // input allocation surface independently of output duration.
        assert!(validate_input_len(MAX_MIDI_BYTES).is_ok());
        assert!(validate_input_len(MAX_MIDI_BYTES + 1).is_err());
    }

    #[test]
    fn checked_tick_and_frame_math_rejects_overflow() {
        assert!(checked_tick_position(u64::MAX, 1).is_err());
        assert!(checked_elapsed_numerator(u128::MAX, 1, 1, 1).is_err());
        assert!(checked_elapsed_numerator(0, u64::MAX, u64::MAX, u32::MAX).is_err());
    }
}
