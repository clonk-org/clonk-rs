use lc_engine::{
    BinaryControlRecord, ControlPacket as EngineControlPacket, RCT_CTRL, RCT_CTRL_PKT, RCT_END,
    RCT_FRAME,
};
use std::collections::VecDeque;
use thiserror::Error;

use crate::legacy::{
    decode_control_entry_prefix, decode_control_list_prefix, encode_control_entry_payload,
    encode_control_list_payload, LegacyControlError, LegacyEncodeError, Reader,
};

/// One typed chunk from a C++ `CtrlRec.c4b` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRecordChunk {
    /// One queued/executed `C4Control` list (`RCT_Ctrl`).
    Controls {
        frame: u32,
        controls: Vec<EngineControlPacket>,
    },
    /// One immediately executed `C4IDPacket` (`RCT_CtrlPkt`).
    ControlPacket {
        frame: u32,
        control: EngineControlPacket,
    },
    /// Empty frame-distance filler (`RCT_Frame`).
    Frame { frame: u32 },
    /// One diagnostic `C4PktDebugRec` chunk. The outer record type and the
    /// embedded raw enum are retained independently because C++ does not
    /// require them to match.
    DebugRecord {
        frame: u32,
        chunk_type: u8,
        debug_type: i32,
        data: Vec<u8>,
    },
    /// End-of-record marker (`RCT_End`).
    End { frame: u32 },
}

impl ControlRecordChunk {
    pub fn frame(&self) -> u32 {
        match self {
            Self::Controls { frame, .. }
            | Self::ControlPacket { frame, .. }
            | Self::Frame { frame }
            | Self::DebugRecord { frame, .. }
            | Self::End { frame } => *frame,
        }
    }
}

fn decode_debug_record_prefix(payload: &[u8]) -> Result<(i32, Vec<u8>, usize), LegacyControlError> {
    let mut reader = Reader::new(payload);
    // C4PktDebugRec::eType is compiled through mkIntAdapt, whose default
    // storage type is a native-endian int32. Its StdBuf then carries a packed
    // uint32 byte count and exactly that many opaque bytes.
    let debug_type = reader.read_raw_i32()?;
    let size = reader.read_uint32()? as usize;
    let data = reader.read_bytes(size)?.to_vec();
    let consumed = payload.len() - reader.remaining();
    Ok((debug_type, data, consumed))
}

/// Typed writer for the control-bearing subset of `CtrlRec.c4b`.
///
/// The raw chunk writer remains in `lc-engine`; this layer supplies the exact
/// `C4Control`/`C4IDPacket` payload codecs and prevents client/tick transport
/// metadata from being written into a record.
#[derive(Debug, Clone, Default)]
pub struct ControlRecordWriter {
    record: BinaryControlRecord,
}

impl ControlRecordWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one executed control list. C++ omits empty lists.
    pub fn record_controls(
        &mut self,
        frame: u32,
        controls: &[EngineControlPacket],
    ) -> Result<(), LegacyEncodeError> {
        if controls.is_empty() {
            return Ok(());
        }
        // Encode before touching the stream so an unsupported packet leaves
        // the writer unchanged.
        let payload = encode_control_list_payload(controls)?;
        self.record.rec(frame, &payload, RCT_CTRL);
        Ok(())
    }

    /// Record one immediately executed control packet. The payload excludes
    /// the live `PID_ControlPkt` delivery byte.
    pub fn record_packet(
        &mut self,
        frame: u32,
        control: &EngineControlPacket,
    ) -> Result<(), LegacyEncodeError> {
        let payload = encode_control_entry_payload(control)?;
        self.record.rec(frame, &payload, RCT_CTRL_PKT);
        Ok(())
    }

    pub fn bytes(&self) -> &[u8] {
        self.record.bytes()
    }

    /// Append C++'s end marker and consume the writer, preventing data from
    /// being appended after the logical end of the record.
    pub fn finish(mut self, frame: u32) -> Vec<u8> {
        self.record.finish(frame);
        self.record.into_bytes()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlRecordDecodeError {
    #[error("control record payload is invalid: {0}")]
    Control(#[from] LegacyControlError),
    #[error("control record chunk type {0:#x} is unsupported")]
    UnsupportedChunkType(u8),
    #[error("control record ends in a partial chunk")]
    Truncated,
}

/// Incremental parser for a C++ `CtrlRec.c4b` stream.
///
/// Chunk payloads have no lengths. The parser uses the control codec's exact
/// consumed position and retains an incomplete header/payload until more bytes
/// arrive, matching `C4Playback::ReadBinary`'s sequential-buffer behavior.
#[derive(Debug, Clone, Default)]
pub struct ControlRecordParser {
    pending: Vec<u8>,
    frame: u32,
    ended: bool,
}

impl ControlRecordParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_finished(&self) -> bool {
        self.ended
    }

    pub fn push(
        &mut self,
        bytes: &[u8],
    ) -> Result<Vec<ControlRecordChunk>, ControlRecordDecodeError> {
        if self.ended {
            return Ok(Vec::new());
        }
        self.pending.extend_from_slice(bytes);

        let mut cursor = 0usize;
        let mut frame = self.frame;
        let mut ended = false;
        let mut chunks = Vec::new();

        loop {
            let available = &self.pending[cursor..];
            if available.len() < 2 {
                break;
            }
            let delta = available[0];
            let chunk_type = available[1];
            let payload = &available[2..];

            let parsed = match chunk_type {
                RCT_CTRL => match decode_control_list_prefix(payload) {
                    Ok((controls, consumed)) => Some((
                        ControlRecordChunk::Controls { frame: 0, controls },
                        consumed,
                    )),
                    Err(LegacyControlError::UnexpectedEof) => None,
                    Err(error) => return Err(error.into()),
                },
                RCT_CTRL_PKT => match decode_control_entry_prefix(payload) {
                    Ok((control, consumed)) => Some((
                        ControlRecordChunk::ControlPacket { frame: 0, control },
                        consumed,
                    )),
                    Err(LegacyControlError::UnexpectedEof) => None,
                    Err(error) => return Err(error.into()),
                },
                RCT_FRAME => Some((ControlRecordChunk::Frame { frame: 0 }, 0)),
                RCT_END => Some((ControlRecordChunk::End { frame: 0 }, 0)),
                other if other >= 0x80 => match decode_debug_record_prefix(payload) {
                    Ok((debug_type, data, consumed)) => Some((
                        ControlRecordChunk::DebugRecord {
                            frame: 0,
                            chunk_type: other,
                            debug_type,
                            data,
                        },
                        consumed,
                    )),
                    Err(LegacyControlError::UnexpectedEof) => None,
                    Err(error) => return Err(error.into()),
                },
                other => return Err(ControlRecordDecodeError::UnsupportedChunkType(other)),
            };

            let Some((mut chunk, payload_len)) = parsed else {
                break;
            };
            // C4Playback accumulates the uint8 delta into a uint32 frame and
            // therefore wraps at the native unsigned boundary.
            let next_frame = frame.wrapping_add(u32::from(delta));
            match &mut chunk {
                ControlRecordChunk::Controls { frame, .. }
                | ControlRecordChunk::ControlPacket { frame, .. }
                | ControlRecordChunk::Frame { frame }
                | ControlRecordChunk::DebugRecord { frame, .. }
                | ControlRecordChunk::End { frame } => *frame = next_frame,
            }

            frame = next_frame;
            cursor += 2 + payload_len;
            ended = matches!(chunk, ControlRecordChunk::End { .. });
            chunks.push(chunk);
            if ended {
                break;
            }
        }

        self.frame = frame;
        self.ended = ended;
        if ended {
            // C4Playback::ReadBinary stops at the first RCT_End without
            // inspecting any bytes that follow it.
            self.pending.clear();
        } else if cursor != 0 {
            self.pending.drain(..cursor);
        }
        Ok(chunks)
    }

    /// Validate the remaining non-sequential input at physical EOF.
    ///
    /// C++ does not require `RCT_End` and also ignores a lone byte that cannot
    /// form a complete chunk header. A complete header whose known payload is
    /// partial remains a truncation error.
    pub fn finish(&self) -> Result<(), ControlRecordDecodeError> {
        if self.ended || self.pending.len() < 2 {
            Ok(())
        } else {
            Err(ControlRecordDecodeError::Truncated)
        }
    }
}

/// Decode a complete `CtrlRec.c4b` byte stream.
pub fn decode_control_record(
    bytes: &[u8],
) -> Result<Vec<ControlRecordChunk>, ControlRecordDecodeError> {
    let mut parser = ControlRecordParser::new();
    let chunks = parser.push(bytes)?;
    parser.finish()?;
    Ok(chunks)
}

/// Frame-ordered playback view of a complete `CtrlRec.c4b` stream.
///
/// [`take_controls`](Self::take_controls) mirrors
/// `C4Playback::ExecuteControl`: every list or individual packet whose
/// recorded frame is less than or equal to the requested frame is returned in
/// file order. Callers execute the returned controls before simulating that
/// frame.
#[derive(Debug, Clone)]
pub struct ControlRecordPlayback {
    chunks: VecDeque<ControlRecordChunk>,
    finished: bool,
}

impl ControlRecordPlayback {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ControlRecordDecodeError> {
        Ok(Self {
            chunks: decode_control_record(bytes)?.into(),
            finished: false,
        })
    }

    pub fn take_controls(&mut self, frame: u32) -> Vec<EngineControlPacket> {
        if self.finished {
            return Vec::new();
        }

        let mut controls = Vec::new();
        while self
            .chunks
            .front()
            .is_some_and(|chunk| chunk.frame() <= frame)
        {
            match self.chunks.pop_front().expect("front chunk exists") {
                ControlRecordChunk::Controls {
                    controls: recorded, ..
                } => controls.extend(recorded),
                ControlRecordChunk::ControlPacket { control, .. } => controls.push(control),
                ControlRecordChunk::Frame { .. } => {}
                ControlRecordChunk::DebugRecord { .. } => {
                    // Normal C++ builds parse diagnostic chunks so their
                    // boundary is known, then advance without executing them.
                }
                ControlRecordChunk::End { .. } => {
                    self.finished = true;
                    break;
                }
            }
        }
        controls
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn next_frame(&self) -> Option<u32> {
        self.chunks.front().map(ControlRecordChunk::frame)
    }
}

#[cfg(test)]
mod tests {
    use lc_engine::{
        DebugRecordControlData, LegacyCString, ScriptControlData, ScriptStrictness,
        SynchronizeControlData,
    };

    use super::*;

    fn synchronize() -> EngineControlPacket {
        EngineControlPacket::Synchronize(SynchronizeControlData {
            save_player_files: true,
            sync_clearance: true,
            by_client: 0,
        })
    }

    fn script() -> EngineControlPacket {
        EngineControlPacket::Script(ScriptControlData {
            target_object: -1,
            strictness: ScriptStrictness::Strict3,
            script: LegacyCString::from_bytes(b"A\x01B".to_vec()).unwrap(),
            by_client: 2,
        })
    }

    #[test]
    fn ctrl_rec_writer_matches_known_cpp_control_list_bytes() {
        let control = synchronize();
        let mut writer = ControlRecordWriter::new();
        writer.record_controls(5, &[control.clone()]).unwrap();
        let bytes = writer.finish(5);

        // RCT_Ctrl head, CID_Synchronize body, PID_None, then C++'s raw
        // `(frame + 37) & 0xff` RCT_End head.
        assert_eq!(bytes, [5, RCT_CTRL, 0x86, 1, 1, 0, 0xff, 42, RCT_END]);
        assert_eq!(
            decode_control_record(&bytes).unwrap(),
            vec![
                ControlRecordChunk::Controls {
                    frame: 5,
                    controls: vec![control],
                },
                // C++ writes 42 into the delta field after the frame-5 chunk.
                ControlRecordChunk::End { frame: 47 },
            ]
        );
    }

    #[test]
    fn adjacent_unlengthened_packet_and_list_decode_at_their_schema_boundaries() {
        let direct = script();
        let queued = synchronize();
        let mut writer = ControlRecordWriter::new();
        writer.record_packet(7, &direct).unwrap();
        writer.record_controls(7, &[queued.clone()]).unwrap();
        let bytes = writer.finish(7);

        assert_eq!(
            decode_control_record(&bytes).unwrap(),
            vec![
                ControlRecordChunk::ControlPacket {
                    frame: 7,
                    control: direct,
                },
                ControlRecordChunk::Controls {
                    frame: 7,
                    controls: vec![queued],
                },
                ControlRecordChunk::End { frame: 51 },
            ]
        );
    }

    #[test]
    fn incremental_parser_retains_every_partial_header_and_payload() {
        let mut writer = ControlRecordWriter::new();
        writer.record_packet(9, &script()).unwrap();
        writer.record_controls(10, &[synchronize()]).unwrap();
        let bytes = writer.finish(10);
        let expected = decode_control_record(&bytes).unwrap();

        let mut parser = ControlRecordParser::new();
        let mut actual = Vec::new();
        for byte in bytes {
            actual.extend(parser.push(&[byte]).unwrap());
        }
        parser.finish().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn cpp_debug_chunks_do_not_block_later_record_controls() {
        let later_control = synchronize();
        let mut bytes = Vec::new();

        // ReadBinary uses one generic C4PktDebugRec grammar for every outer
        // type >= 0x80, including gaps and RCT_Undefined. Deliberately make
        // the embedded raw i32 differ from the outer byte: C++ retains both
        // and does not validate equality while loading the record.
        for (index, chunk_type) in (0x80_u8..=0xff).enumerate() {
            bytes.extend([if index == 0 { 7 } else { 0 }, chunk_type]);
            let debug_type = 0x1234_0000 | i32::from(chunk_type);
            bytes.extend(debug_type.to_ne_bytes());
            let data = match chunk_type {
                0x80 => Vec::new(),
                0x81 => vec![chunk_type; 128],
                _ => vec![chunk_type, 0x00, 0xff],
            };
            if data.len() == 128 {
                bytes.extend([0x80, 0x01]);
            } else {
                bytes.push(data.len() as u8);
            }
            bytes.extend(data);
        }

        bytes.extend([0, RCT_CTRL_PKT]);
        bytes.extend(encode_control_entry_payload(&later_control).unwrap());
        bytes.extend([44, RCT_END]);

        let expected = decode_control_record(&bytes).expect("C++ debug chunks decode");
        assert_eq!(expected.len(), 130);
        for (index, chunk) in expected.iter().take(128).enumerate() {
            let chunk_type = (0x80_u16 + index as u16) as u8;
            let data = match chunk_type {
                0x80 => Vec::new(),
                0x81 => vec![chunk_type; 128],
                _ => vec![chunk_type, 0x00, 0xff],
            };
            assert_eq!(
                chunk,
                &ControlRecordChunk::DebugRecord {
                    frame: 7,
                    chunk_type,
                    debug_type: 0x1234_0000 | i32::from(chunk_type),
                    data,
                }
            );
        }
        assert_eq!(
            expected[128],
            ControlRecordChunk::ControlPacket {
                frame: 7,
                control: later_control.clone(),
            }
        );

        let mut parser = ControlRecordParser::new();
        let mut incremental = Vec::new();
        for byte in &bytes {
            incremental.extend(parser.push(std::slice::from_ref(byte)).unwrap());
        }
        parser.finish().unwrap();
        assert_eq!(incremental, expected);

        let mut playback = ControlRecordPlayback::from_bytes(&bytes).unwrap();
        assert!(playback.take_controls(6).is_empty());
        assert_eq!(playback.take_controls(7), vec![later_control]);
        assert!(!playback.is_finished());
        assert!(playback.take_controls(u32::MAX).is_empty());
        assert!(playback.is_finished());
    }

    #[test]
    fn cpp_debugrec_control_round_trips_as_noop() {
        let control = EngineControlPacket::DebugRecord(DebugRecordControlData {
            data: vec![0x00, 0xff, 0x40, b'C', b'4'],
        });
        let payload = encode_control_entry_payload(&control).unwrap();
        assert_eq!(payload, [0xc0, 5, 0x00, 0xff, 0x40, b'C', b'4']);

        let mut writer = ControlRecordWriter::new();
        writer.record_packet(4, &control).unwrap();
        let bytes = writer.finish(4);
        assert_eq!(
            decode_control_record(&bytes).unwrap(),
            vec![
                ControlRecordChunk::ControlPacket {
                    frame: 4,
                    control: control.clone(),
                },
                ControlRecordChunk::End { frame: 45 },
            ]
        );

        // The runtime's exhaustive control-replay test executes this variant
        // through the intentional C4ControlDebugRec no-op arm. This layer
        // proves that its opaque bytes survive the C++ binary record codec.
        let mut playback = ControlRecordPlayback::from_bytes(&bytes).unwrap();
        assert_eq!(playback.take_controls(4), vec![control]);
    }

    #[test]
    fn large_frame_gap_decodes_the_cpp_filler_chunks() {
        let mut writer = ControlRecordWriter::new();
        writer.record_controls(600, &[synchronize()]).unwrap();
        let bytes = writer.finish(600);
        let chunks = decode_control_record(&bytes).unwrap();

        assert_eq!(
            chunks
                .iter()
                .map(ControlRecordChunk::frame)
                .collect::<Vec<_>>(),
            vec![255, 510, 600, 725]
        );
        assert!(matches!(chunks[0], ControlRecordChunk::Frame { .. }));
        assert!(matches!(chunks[1], ControlRecordChunk::Frame { .. }));
        assert!(matches!(chunks[2], ControlRecordChunk::Controls { .. }));
        assert!(matches!(chunks[3], ControlRecordChunk::End { .. }));
    }

    #[test]
    fn empty_control_list_is_not_written() {
        let writer = {
            let mut writer = ControlRecordWriter::new();
            writer.record_controls(123, &[]).unwrap();
            writer
        };
        assert!(writer.bytes().is_empty());
        assert_eq!(writer.finish(0), [37, RCT_END]);
    }

    #[test]
    fn parser_distinguishes_truncation_and_unsupported_chunks() {
        let mut truncated = ControlRecordParser::new();
        assert!(truncated.push(&[5, RCT_CTRL, 0x86]).unwrap().is_empty());
        assert_eq!(truncated.finish(), Err(ControlRecordDecodeError::Truncated));

        let mut truncated_debug = ControlRecordParser::new();
        assert!(truncated_debug
            .push(&[0, 0x81, 0x81, 0, 0, 0, 2, 0xaa])
            .unwrap()
            .is_empty());
        assert_eq!(
            truncated_debug.finish(),
            Err(ControlRecordDecodeError::Truncated)
        );
        assert_eq!(
            decode_control_record(&[0, 0x55]),
            Err(ControlRecordDecodeError::UnsupportedChunkType(0x55))
        );
    }

    #[test]
    fn cpp_record_parser_accepts_clean_eof_and_ignores_after_end() {
        let control = synchronize();
        let mut writer = ControlRecordWriter::new();
        writer.record_controls(5, &[control.clone()]).unwrap();
        let without_end = writer.bytes().to_vec();
        let expected_without_end = vec![ControlRecordChunk::Controls {
            frame: 5,
            controls: vec![control.clone()],
        }];

        assert_eq!(
            decode_control_record(&without_end).unwrap(),
            expected_without_end
        );
        assert!(decode_control_record(&[]).unwrap().is_empty());

        let mut with_partial_header = without_end.clone();
        with_partial_header.push(0xff);
        assert_eq!(
            decode_control_record(&with_partial_header).unwrap(),
            expected_without_end
        );

        let mut playback = ControlRecordPlayback::from_bytes(&without_end).unwrap();
        assert_eq!(playback.take_controls(5), vec![control.clone()]);
        assert_eq!(playback.next_frame(), None);
        assert!(!playback.is_finished());

        let mut ended_with_suffix = writer.finish(5);
        ended_with_suffix.extend_from_slice(&[0, 0x55, 0xff]);
        let expected_ended = vec![
            ControlRecordChunk::Controls {
                frame: 5,
                controls: vec![control],
            },
            ControlRecordChunk::End { frame: 47 },
        ];
        let mut parser = ControlRecordParser::new();
        assert_eq!(parser.push(&ended_with_suffix).unwrap(), expected_ended);
        assert!(parser.is_finished());
        assert!(parser.push(&[0, RCT_CTRL, 0x86]).unwrap().is_empty());
        parser.finish().unwrap();

        assert_eq!(
            decode_control_record(&[5, RCT_CTRL, 0x86]),
            Err(ControlRecordDecodeError::Truncated)
        );
    }

    #[test]
    fn playback_returns_all_due_controls_in_file_order_before_the_frame_tick() {
        let first = script();
        let second = synchronize();
        let third = script();
        let mut writer = ControlRecordWriter::new();
        writer.record_packet(2, &first).unwrap();
        writer.record_controls(2, &[second.clone()]).unwrap();
        writer.record_packet(4, &third).unwrap();
        let bytes = writer.finish(4);

        let mut playback = ControlRecordPlayback::from_bytes(&bytes).unwrap();
        assert!(playback.take_controls(1).is_empty());
        assert_eq!(playback.take_controls(2), vec![first, second]);
        assert!(playback.take_controls(3).is_empty());
        assert_eq!(playback.take_controls(4), vec![third]);
        assert!(!playback.is_finished());
        assert!(playback.take_controls(u32::MAX).is_empty());
        assert!(playback.is_finished());
    }

    #[test]
    fn prefix_decoders_report_the_exact_consumed_boundary() {
        let control = synchronize();
        let mut entry = encode_control_entry_payload(&control).unwrap();
        let entry_len = entry.len();
        entry.extend_from_slice(&[0, RCT_END]);
        assert_eq!(
            decode_control_entry_prefix(&entry),
            Ok((control.clone(), entry_len))
        );

        let mut list = encode_control_list_payload(&[control.clone()]).unwrap();
        let list_len = list.len();
        list.extend_from_slice(&[0, RCT_END]);
        assert_eq!(
            decode_control_list_prefix(&list),
            Ok((vec![control], list_len))
        );
    }
}
