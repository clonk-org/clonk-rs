use clonk_engine::{
    parse_control_ini, BinaryControlRecord, ControlPacket as EngineControlPacket,
    ControlParseError, RCT_CTRL, RCT_CTRL_PKT, RCT_END, RCT_FRAME,
};
use std::collections::VecDeque;
use thiserror::Error;

use crate::legacy::{
    append_raw_i32, append_uint32, decode_control_entry_prefix, decode_control_list_prefix,
    encode_control_entry_payload, encode_control_list_payload, LegacyControlError,
    LegacyEncodeError, Reader,
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

    fn signed_frame(&self) -> i32 {
        self.frame() as i32
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
/// The raw chunk writer remains in `clonk-engine`; this layer supplies the exact
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
    #[error("text control record payload is invalid: {0}")]
    TextControl(#[from] ControlParseError),
    #[error("text control packet envelope is invalid: {detail}")]
    InvalidTextControlEnvelope { detail: String },
    #[error("text direct-control chunk decoded {count} packets instead of exactly one")]
    TextControlPacketCount { count: usize },
    #[error("control record chunk type {0:#x} is unsupported")]
    UnsupportedChunkType(u8),
    #[error("control record ends in a partial chunk")]
    Truncated,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlRecordRewriteError {
    #[error("control record could not be decoded: {0}")]
    Decode(#[from] ControlRecordDecodeError),
    #[error("control record could not be encoded: {0}")]
    Encode(#[from] LegacyEncodeError),
    #[error("control record could not be written as classic text: {0}")]
    Text(#[from] clonk_engine::ControlIniEncodeError),
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

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn legacy_text_lines(input: &[u8]) -> Vec<&[u8]> {
    let end = input
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(input.len());
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < end {
        if input[index] != b'\r' && input[index] != b'\n' {
            index += 1;
            continue;
        }
        lines.push(&input[start..index]);
        if input[index] == b'\r' && input.get(index + 1) == Some(&b'\n') {
            index += 1;
        }
        index += 1;
        start = index;
    }
    lines.push(&input[start..end]);
    lines
}

fn parse_control_ini_bytes(input: &[u8]) -> Result<Vec<EngineControlPacket>, ControlParseError> {
    let end = input
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(input.len());
    let input = &input[..end];
    let mut normalized = String::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'\r' => {
                normalized.push('\n');
                index += usize::from(input.get(index + 1) == Some(&b'\n')) + 1;
            }
            b'\n' => {
                normalized.push('\n');
                index += 1;
            }
            byte => {
                normalized.push(char::from(byte));
                index += 1;
            }
        }
    }
    parse_control_ini(&normalized)
}

fn parse_text_controls(
    input: &[u8],
) -> Result<Option<Vec<EngineControlPacket>>, ControlRecordDecodeError> {
    match parse_control_ini_bytes(input) {
        Ok(mut controls) => {
            validate_text_control_names(&mut controls, input);
            Ok(Some(controls))
        }
        Err(error)
            if matches!(
                &error,
                ControlParseError::MissingField { .. }
                    | ControlParseError::InvalidBooleanField { .. }
            ) || matches!(
                &error,
                ControlParseError::InvalidIntegerField { value, .. }
                    if parse_cpp_integer_prefix(value.as_bytes()).is_none()
            ) =>
        {
            // These are StdCompiler::NotFoundException paths in C++. The
            // repeated Rec container catches them and keeps only the already
            // decoded prefix.
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn text_section_name(line: &[u8]) -> Option<&[u8]> {
    let line = trim_ascii(line);
    line.strip_prefix(b"[")?.strip_suffix(b"]")
}

fn text_field<'a>(line: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let line = trim_ascii(line);
    let equals = line.iter().position(|byte| *byte == b'=')?;
    (trim_ascii(&line[..equals]) == name).then(|| trim_ascii(&line[equals + 1..]))
}

fn has_text_field(lines: &[&[u8]], name: &[u8]) -> bool {
    lines.iter().any(|line| text_field(line, name).is_some())
}

fn validate_text_control_names(controls: &mut [EngineControlPacket], input: &[u8]) {
    let lines = legacy_text_lines(input);
    let packet_blocks = lines
        .split(|line| text_section_name(line) == Some(b"IDPacket"))
        .skip(1);

    for (control, block) in controls.iter_mut().zip(packet_blocks) {
        match control {
            EngineControlPacket::ClientJoin(data) => {
                if has_text_field(block, b"Name") {
                    data.core.name =
                        crate::validate_name_no_empty(std::mem::take(&mut data.core.name));
                }
                if has_text_field(block, b"Nick") {
                    data.core.nick =
                        crate::validate_name_no_empty(std::mem::take(&mut data.core.nick));
                }
            }
            EngineControlPacket::PlayerInfo(data) => {
                let player_blocks = block
                    .split(|line| text_section_name(line) == Some(b"Player"))
                    .skip(1);
                for (player, player_block) in data.players.iter_mut().zip(player_blocks) {
                    if has_text_field(player_block, b"Name") {
                        player.name =
                            crate::validate_name_no_empty(std::mem::take(&mut player.name));
                    }
                    if has_text_field(player_block, b"ForcedName") {
                        player.forced_name = crate::validate_name_allow_empty(std::mem::take(
                            &mut player.forced_name,
                        ));
                    }
                    if has_text_field(player_block, b"LeagueAccount") {
                        player.league_account = crate::validate_name_allow_empty(std::mem::take(
                            &mut player.league_account,
                        ));
                    }
                    if has_text_field(player_block, b"ClanTag") {
                        player.clan_tag =
                            crate::validate_name_allow_empty(std::mem::take(&mut player.clan_tag));
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_cpp_integer_prefix_with_len(value: &[u8]) -> Option<(i64, usize)> {
    let value = trim_ascii(value);
    let mut index = 0;
    let negative = match value.first() {
        Some(b'-') => {
            index = 1;
            true
        }
        Some(b'+') => {
            index = 1;
            false
        }
        _ => false,
    };
    let radix = if value[index..].starts_with(b"0x") || value[index..].starts_with(b"0X") {
        index += 2;
        16u8
    } else {
        10u8
    };
    let start = index;
    let limit = if negative {
        i64::MAX as u64 + 1
    } else {
        i64::MAX as u64
    };
    let mut magnitude = 0u64;
    while let Some(byte) = value.get(index) {
        let digit = match *byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' if radix == 16 => byte - b'a' + 10,
            b'A'..=b'F' if radix == 16 => byte - b'A' + 10,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        magnitude = magnitude
            .saturating_mul(u64::from(radix))
            .saturating_add(u64::from(digit))
            .min(limit);
        index += 1;
    }
    if index == start {
        return None;
    }
    let value = if negative {
        if magnitude == i64::MAX as u64 + 1 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    };
    Some((value, index))
}

fn parse_cpp_integer_prefix(value: &[u8]) -> Option<i64> {
    parse_cpp_integer_prefix_with_len(value).map(|(value, _)| value)
}

fn parse_text_packet_id(value: &[u8]) -> Option<u8> {
    parse_cpp_integer_prefix(value).map(|value| {
        if value < 0 {
            u8::MAX
        } else {
            value.min(i64::from(u8::MAX)) as u8
        }
    })
}

fn control_packet_section(id: u8) -> Option<&'static [u8]> {
    Some(match id {
        0x80 => b"Client Join",
        0x81 => b"Client Update",
        0x82 => b"Client Remove",
        0x83 => b"Voting",
        0x84 => b"Voting End",
        0x85 => b"Sync Check",
        0x86 => b"Synchronize",
        0x87 => b"Set",
        0x88 => b"Script",
        0x90 => b"Player Info",
        0x91 => b"Join Player",
        0x92 => b"Remove Player",
        0xa0 => b"Player Select",
        0xa1 => b"Player Control",
        0xa2 => b"Player Command",
        0xa3 => b"Message",
        0xb0 => b"EM Move Obj",
        0xb1 => b"EM Draw Tool",
        0xb2 => b"EM Drop Def",
        0xc0 => b"Debug Rec",
        0xd0 => b"Message Board Answer",
        0xd1 => b"Custom Command",
        0xd2 => b"Init Scenario Player",
        0xd3 => b"Activate Game Goal Menu",
        0xd4 => b"Toggle Hostility",
        0xd5 => b"Surrender Player",
        // The native INI reader cannot scan the slash in this packet's
        // registered name, so C++ text playback cannot read CID 0xd6 either.
        0xd7 => b"Set Player Team",
        0xd8 => b"Eliminate Player",
        _ => return None,
    })
}

fn validate_text_packet(lines: &[&[u8]]) -> Result<(), ControlRecordDecodeError> {
    let id = lines
        .iter()
        .find_map(|line| text_field(line, b"ID"))
        .and_then(parse_text_packet_id)
        .ok_or_else(|| ControlRecordDecodeError::InvalidTextControlEnvelope {
            detail: "packet ID is absent or unreadable".to_string(),
        })?;
    // C4ControlDebugRec compiles its StdBuf directly through the named
    // `Debug Rec` value instead of opening a packet-body section.
    if id == 0xc0 {
        if lines
            .iter()
            .any(|line| text_field(line, b"Debug Rec").is_some())
        {
            return Ok(());
        }
        return Err(ControlRecordDecodeError::InvalidTextControlEnvelope {
            detail: "packet ID 0xc0 is missing Debug Rec=<StdBuf>".to_string(),
        });
    }
    let expected = control_packet_section(id).ok_or_else(|| {
        ControlRecordDecodeError::InvalidTextControlEnvelope {
            detail: format!("packet ID {id:#x} has no native control payload"),
        }
    })?;
    if lines
        .iter()
        .filter_map(|line| text_section_name(line))
        .any(|section| section == expected)
    {
        return Ok(());
    }
    Err(ControlRecordDecodeError::InvalidTextControlEnvelope {
        detail: format!(
            "packet ID {id:#x} is missing [{}]",
            String::from_utf8_lossy(expected)
        ),
    })
}

fn validate_text_control_payload(
    payload: &[u8],
    direct: bool,
) -> Result<(), ControlRecordDecodeError> {
    let lines = legacy_text_lines(payload);
    if direct {
        return validate_text_packet(&lines);
    }
    let mut packet_start = None;
    for (index, line) in lines.iter().enumerate() {
        if text_section_name(line) != Some(b"IDPacket") {
            continue;
        }
        if let Some(start) = packet_start.replace(index) {
            validate_text_packet(&lines[start..index])?;
        }
    }
    if let Some(start) = packet_start {
        validate_text_packet(&lines[start..])?;
    }
    Ok(())
}

fn canonicalize_text_packet_ids(payload: &[u8], direct: bool) -> Vec<u8> {
    let mut output = Vec::with_capacity(payload.len());
    let mut expect_id = direct;
    for line in legacy_text_lines(payload) {
        if !direct && text_section_name(line) == Some(b"IDPacket") {
            expect_id = true;
        }
        if expect_id {
            if let Some(id) = text_field(line, b"ID").and_then(parse_text_packet_id) {
                let indent = line
                    .iter()
                    .take_while(|byte| byte.is_ascii_whitespace())
                    .count();
                output.extend_from_slice(&line[..indent]);
                output.extend_from_slice(format!("ID={id}").as_bytes());
                output.push(b'\n');
                expect_id = false;
                continue;
            }
        }
        output.extend_from_slice(line);
        output.push(b'\n');
    }
    output
}

fn record_root_field<'a>(line: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    if line.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        return None;
    }
    let line = trim_ascii(line);
    let equals = line.iter().position(|byte| *byte == b'=')?;
    (trim_ascii(&line[..equals]) == name).then(|| trim_ascii(&line[equals + 1..]))
}

fn parse_text_integer(value: &[u8]) -> Option<i32> {
    parse_cpp_integer_prefix(value).map(|value| value as i32)
}

fn parse_text_std_buf(value: &[u8]) -> Option<Vec<u8>> {
    let colon = value.iter().position(|byte| *byte == b':')?;
    let size_field = trim_ascii(&value[..colon]);
    let (size, consumed) = parse_cpp_integer_prefix_with_len(size_field)?;
    if consumed != size_field.len() {
        return None;
    }
    let size = usize::try_from(size).ok()?;
    let (mut data, _, quoted) = crate::league::parse_escaped_value(trim_ascii(&value[colon + 1..]));
    if !quoted || data.len() < size {
        return None;
    }
    data.truncate(size);
    Some(data)
}

fn decode_text_debug_record(
    payload: &[u8],
    frame: u32,
    chunk_type: u8,
) -> Option<ControlRecordChunk> {
    let lines = legacy_text_lines(payload);
    let debug_type = lines
        .iter()
        .find_map(|line| text_field(line, b"Type"))
        .and_then(parse_text_integer)?;
    let data = lines
        .iter()
        .find_map(|line| text_field(line, b"Data"))
        .and_then(parse_text_std_buf)?;
    Some(ControlRecordChunk::DebugRecord {
        frame,
        chunk_type,
        debug_type,
        data,
    })
}

fn decode_text_record_block(
    lines: &[&[u8]],
) -> Result<Option<ControlRecordChunk>, ControlRecordDecodeError> {
    let mut frame = None;
    let mut chunk_type = None;
    let mut payload = Vec::new();
    for line in lines {
        if let Some(value) = record_root_field(line, b"Frame") {
            if frame.is_none() {
                let Some(value) = parse_text_integer(value) else {
                    return Ok(None);
                };
                frame = Some(value);
                continue;
            }
        }
        if let Some(value) = record_root_field(line, b"Type") {
            if chunk_type.is_none() {
                let Some(value) = parse_text_integer(value) else {
                    return Ok(None);
                };
                chunk_type = Some(value as u8);
                continue;
            }
        }
        payload.extend_from_slice(line);
        payload.push(b'\n');
    }

    // mkSTLContainerAdapt stops at the first Rec whose required named values
    // are absent or unreadable. Preserve the successfully decoded prefix
    // instead of turning that native NotFound boundary into a corruption
    // error.
    let (Some(frame), Some(chunk_type)) = (frame, chunk_type) else {
        return Ok(None);
    };
    // The binary path holds the native int32 frame's bit pattern in this
    // unsigned accumulator. Preserve that same representation for text so
    // rewriting casts it back exactly and binary output retains its low byte.
    let frame = frame as u32;

    let chunk = match chunk_type {
        RCT_CTRL => {
            validate_text_control_payload(&payload, false)?;
            let mut control = b"[Control]\n".to_vec();
            control.extend_from_slice(&canonicalize_text_packet_ids(&payload, false));
            let Some(controls) = parse_text_controls(&control)? else {
                return Ok(None);
            };
            ControlRecordChunk::Controls { frame, controls }
        }
        RCT_CTRL_PKT => {
            validate_text_control_payload(&payload, true)?;
            let mut control = b"[Control]\n[IDPacket]\n".to_vec();
            control.extend_from_slice(&canonicalize_text_packet_ids(&payload, true));
            let Some(mut controls) = parse_text_controls(&control)? else {
                return Ok(None);
            };
            if controls.len() != 1 {
                return Err(ControlRecordDecodeError::TextControlPacketCount {
                    count: controls.len(),
                });
            }
            ControlRecordChunk::ControlPacket {
                frame,
                control: controls.pop().expect("one direct control was checked"),
            }
        }
        RCT_FRAME => ControlRecordChunk::Frame { frame },
        RCT_END => ControlRecordChunk::End { frame },
        // RCT_File compiles an unnamed filename/buffer pair. INI has no value
        // position for it, so native raises NotFound and retains the already
        // decoded repeated-Rec prefix.
        0x30 => return Ok(None),
        other => {
            let Some(debug_record) = decode_text_debug_record(&payload, frame, other) else {
                return Ok(None);
            };
            debug_record
        }
    };
    Ok(Some(chunk))
}

/// Decode the control-bearing subset of C++'s repeated `[Rec]` text grammar.
///
/// `Frame` is absolute in this representation. Type 0 embeds a `C4Control`
/// list as repeated `[IDPacket]` children, while type 1 embeds one
/// `C4IDPacket` directly. Other readable chunk types retain their native
/// `C4PktDebugRec` type/data envelope; `RCT_File` has no usable INI form.
pub fn decode_control_record_text(
    bytes: &[u8],
) -> Result<Vec<ControlRecordChunk>, ControlRecordDecodeError> {
    let mut chunks = Vec::new();
    let mut block = None::<Vec<&[u8]>>;
    for line in legacy_text_lines(bytes) {
        if trim_ascii(line) == b"[Rec]" {
            if let Some(previous) = block.take() {
                let Some(chunk) = decode_text_record_block(&previous)? else {
                    return Ok(chunks);
                };
                chunks.push(chunk);
            }
            block = Some(Vec::new());
        } else if let Some(block) = block.as_mut() {
            block.push(line);
        }
    }
    if let Some(block) = block {
        if let Some(chunk) = decode_text_record_block(&block)? {
            chunks.push(chunk);
        }
    }
    Ok(chunks)
}

/// Encode already-decoded control-record chunks using C++'s canonical binary
/// rewrite rules. This entry point lets stream conversion remove `RCT_File`
/// chunks before sharing the ordinary `/recdump` writer.
pub fn encode_control_record_binary(
    chunks: &[ControlRecordChunk],
) -> Result<Vec<u8>, LegacyEncodeError> {
    let mut output = Vec::new();
    let mut previous_frame = 0u32;

    for chunk in chunks {
        let mut payload = Vec::new();
        let (chunk_type, finished) = match chunk {
            ControlRecordChunk::Controls { controls, .. } => {
                payload = encode_control_list_payload(controls)?;
                (RCT_CTRL, false)
            }
            ControlRecordChunk::ControlPacket { control, .. } => {
                payload = encode_control_entry_payload(control)?;
                (RCT_CTRL_PKT, false)
            }
            ControlRecordChunk::Frame { .. } => (RCT_FRAME, false),
            ControlRecordChunk::DebugRecord {
                chunk_type,
                debug_type,
                data,
                ..
            } => {
                append_raw_i32(&mut payload, *debug_type);
                let size = u32::try_from(data.len())
                    .map_err(|_| LegacyEncodeError::DebugRecordTooLarge(data.len()))?;
                append_uint32(&mut payload, size);
                payload.extend_from_slice(data);
                (*chunk_type, false)
            }
            ControlRecordChunk::End { .. } => (RCT_END, true),
        };

        let frame = chunk.frame();
        // C4Playback::ReWriteBinary diagnoses a delta outside 0..=255 but
        // still assigns it to the uint8 chunk header, retaining its low byte.
        output.push(frame.wrapping_sub(previous_frame) as u8);
        output.push(chunk_type);
        output.extend_from_slice(&payload);
        previous_frame = frame;

        // ReWriteBinary stops at the first RCT_End even if its in-memory
        // chunk list happens to contain later entries.
        if finished {
            break;
        }
    }

    Ok(output)
}

/// Rewrite a complete C++ `CtrlRec.c4b` stream in canonical binary form.
///
/// This mirrors `C4Playback::ReWriteBinary`: chunk payloads are decompiled
/// through their typed codecs, frame deltas are recomputed from absolute
/// frames and truncated to the native uint8 header, and output stops after
/// the first `RCT_End`. No missing end marker is synthesized.
pub fn rewrite_control_record_binary(bytes: &[u8]) -> Result<Vec<u8>, ControlRecordRewriteError> {
    let chunks = decode_control_record(bytes)?;
    Ok(encode_control_record_binary(&chunks)?)
}

fn append_record_text_value(output: &mut Vec<u8>, name: &str, value: impl std::fmt::Display) {
    output.extend_from_slice(name.as_bytes());
    output.push(b'=');
    output.extend_from_slice(value.to_string().as_bytes());
    output.extend_from_slice(b"\r\n");
}

fn append_record_text_escaped(output: &mut Vec<u8>, value: &[u8]) {
    output.push(b'"');
    let mut previous_was_numeric_escape = false;
    for &byte in value {
        let printable = byte.is_ascii_graphic() || byte == b' ';
        let needs_escape = !printable
            || matches!(byte, b'\\' | b'"')
            || (previous_was_numeric_escape && byte.is_ascii_digit());
        if !needs_escape {
            output.push(byte);
            previous_was_numeric_escape = false;
            continue;
        }

        previous_was_numeric_escape = false;
        match byte {
            b'\x07' => output.extend_from_slice(b"\\a"),
            b'\x08' => output.extend_from_slice(b"\\b"),
            b'\x0c' => output.extend_from_slice(b"\\f"),
            b'\n' => output.extend_from_slice(b"\\n"),
            b'\r' => output.extend_from_slice(b"\\r"),
            b'\t' => output.extend_from_slice(b"\\t"),
            b'\x0b' => output.extend_from_slice(b"\\v"),
            b'"' => output.extend_from_slice(b"\\\""),
            b'\\' => output.extend_from_slice(b"\\\\"),
            other => {
                output.push(b'\\');
                output.extend_from_slice(format!("{other:o}").as_bytes());
                previous_was_numeric_escape = true;
            }
        }
    }
    output.push(b'"');
}

fn append_record_text_buffer(output: &mut Vec<u8>, name: &str, value: &[u8]) {
    output.extend_from_slice(name.as_bytes());
    output.push(b'=');
    output.extend_from_slice(value.len().to_string().as_bytes());
    output.push(b':');
    append_record_text_escaped(output, value);
    output.extend_from_slice(b"\r\n");
}

/// Encode already-decoded chunks as C++ `CtrlRec.txt` data.
pub fn encode_control_record_text(
    chunks: &[ControlRecordChunk],
) -> Result<Vec<u8>, clonk_engine::ControlIniEncodeError> {
    let mut output = Vec::new();
    for chunk in chunks {
        output.extend_from_slice(b"[Rec]\r\n");
        append_record_text_value(&mut output, "Frame", chunk.frame() as i32);
        match chunk {
            ControlRecordChunk::Controls { controls, .. } => {
                append_record_text_value(&mut output, "Type", RCT_CTRL);
                for control in controls {
                    clonk_engine::append_control_packet_ini(
                        &mut output,
                        control,
                        2,
                        clonk_engine::ControlIniPacketMode::IdPacketSection,
                    )?;
                }
            }
            ControlRecordChunk::ControlPacket { control, .. } => {
                append_record_text_value(&mut output, "Type", RCT_CTRL_PKT);
                clonk_engine::append_control_packet_ini(
                    &mut output,
                    control,
                    0,
                    clonk_engine::ControlIniPacketMode::Inline,
                )?;
            }
            ControlRecordChunk::Frame { .. } => {
                append_record_text_value(&mut output, "Type", RCT_FRAME);
            }
            ControlRecordChunk::DebugRecord {
                chunk_type,
                debug_type,
                data,
                ..
            } => {
                append_record_text_value(&mut output, "Type", chunk_type);
                append_record_text_value(&mut output, "Type", debug_type);
                append_record_text_buffer(&mut output, "Data", data);
            }
            ControlRecordChunk::End { .. } => {
                append_record_text_value(&mut output, "Type", RCT_END);
            }
        }
        // C4Playback::ReWriteText appends two literal LF bytes after each
        // independently generated INI chunk, whose final field already ends
        // in CRLF.
        output.extend_from_slice(b"\n\n");
    }
    Ok(output)
}

/// Rewrite a complete C++ `CtrlRec.c4b` stream as canonical `CtrlRec.txt`.
pub fn rewrite_control_record_text(bytes: &[u8]) -> Result<Vec<u8>, ControlRecordRewriteError> {
    let chunks = decode_control_record(bytes)?;
    Ok(encode_control_record_text(&chunks)?)
}

/// Frame-ordered playback view of a decoded control-record stream.
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
        Ok(Self::from_chunks(decode_control_record(bytes)?))
    }

    pub fn from_text_bytes(bytes: &[u8]) -> Result<Self, ControlRecordDecodeError> {
        Ok(Self::from_chunks(decode_control_record_text(bytes)?))
    }

    pub fn from_chunks(chunks: Vec<ControlRecordChunk>) -> Self {
        Self {
            chunks: chunks.into(),
            finished: false,
        }
    }

    pub fn take_controls(&mut self, frame: u32) -> Vec<EngineControlPacket> {
        if self.finished {
            return Vec::new();
        }

        let mut controls = Vec::new();
        while self
            .chunks
            .front()
            .is_some_and(|chunk| i64::from(chunk.signed_frame()) <= i64::from(frame))
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
    use clonk_engine::{
        DebugRecordControlData, LegacyCString, PlayerControlData, ScriptControlData,
        ScriptStrictness, SynchronizeControlData, COM_RIGHT,
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
        writer
            .record_controls(5, std::slice::from_ref(&control))
            .unwrap();
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
        writer
            .record_controls(7, std::slice::from_ref(&queued))
            .unwrap();
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
        writer
            .record_controls(5, std::slice::from_ref(&control))
            .unwrap();
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
    fn classic_recdump_binary_rewrite_matches_cpp_chunk_grammar() {
        let mut input = vec![
            5,
            RCT_CTRL,
            0x86,
            1,
            1,
            0x80,
            0,
            0xff,
            0,
            RCT_CTRL_PKT,
            0x86,
            0,
            0,
            0x80,
            0,
            250,
            0xa1,
        ];
        input.extend_from_slice(&(-1_234_i32).to_ne_bytes());
        // Both signed zero and the one-byte debug size use valid but
        // noncanonical two-byte packed representations in the source.
        input.extend_from_slice(&[0x81, 0, 0xee, 1, RCT_FRAME, 2, RCT_END]);
        input.extend_from_slice(&[9, RCT_FRAME]);

        let mut expected = vec![
            5,
            RCT_CTRL,
            0x86,
            1,
            1,
            0,
            0xff,
            0,
            RCT_CTRL_PKT,
            0x86,
            0,
            0,
            0,
            250,
            0xa1,
        ];
        expected.extend_from_slice(&(-1_234_i32).to_ne_bytes());
        expected.extend_from_slice(&[1, 0xee, 1, RCT_FRAME, 2, RCT_END]);

        assert_eq!(rewrite_control_record_binary(&input), Ok(expected));
    }

    #[test]
    fn cpp_binary_rewrite_truncates_frame_deltas_and_stops_at_end() {
        let chunks = vec![
            ControlRecordChunk::Frame { frame: 300 },
            ControlRecordChunk::DebugRecord {
                frame: 20,
                chunk_type: 0xff,
                debug_type: 0x1234_5678,
                data: vec![0xab; 128],
            },
            ControlRecordChunk::End { frame: 700 },
            ControlRecordChunk::Frame { frame: 701 },
        ];

        let rewritten = encode_control_record_binary(&chunks).unwrap();
        let mut expected = vec![44, RCT_FRAME, 232, 0xff];
        expected.extend_from_slice(&0x1234_5678_i32.to_ne_bytes());
        expected.extend_from_slice(&[0x80, 0x01]);
        expected.extend_from_slice(&[0xab; 128]);
        expected.extend_from_slice(&[168, RCT_END]);
        assert_eq!(rewritten, expected);
    }

    #[test]
    fn binary_rewrite_propagates_incomplete_record_errors() {
        assert_eq!(
            rewrite_control_record_binary(&[5, RCT_CTRL, 0x86]),
            Err(ControlRecordRewriteError::Decode(
                ControlRecordDecodeError::Truncated
            ))
        );
    }

    #[test]
    fn classic_recdump_text_rewrite_matches_cpp_ini_grammar() {
        let chunks = vec![
            ControlRecordChunk::Controls {
                frame: 5,
                controls: vec![synchronize()],
            },
            ControlRecordChunk::ControlPacket {
                frame: 5,
                control: script(),
            },
            ControlRecordChunk::DebugRecord {
                frame: 6,
                chunk_type: 0x80,
                debug_type: -1_234,
                data: vec![0, b'7', b'"', b'\\', 0xff],
            },
            ControlRecordChunk::Frame { frame: 7 },
            ControlRecordChunk::End { frame: 9 },
        ];

        let expected = concat!(
            "[Rec]\r\n",
            "Frame=5\r\n",
            "Type=0\r\n",
            "\r\n",
            "  [IDPacket]\r\n",
            "  ID=134\r\n",
            "\r\n",
            "    [Synchronize]\r\n",
            "    SavePlrs=true\r\n",
            "    SyncClear=true\r\n",
            "    ByClient=0\r\n",
            "\n\n",
            "[Rec]\r\n",
            "Frame=5\r\n",
            "Type=1\r\n",
            "ID=136\r\n",
            "\r\n",
            "  [Script]\r\n",
            "  Script=\"A\\1B\"\r\n",
            "  ByClient=2\r\n",
            "\n\n",
            "[Rec]\r\n",
            "Frame=6\r\n",
            "Type=128\r\n",
            "Type=-1234\r\n",
            "Data=5:\"\\0\\67\\\"\\\\\\377\"\r\n",
            "\n\n",
            "[Rec]\r\n",
            "Frame=7\r\n",
            "Type=2\r\n",
            "\n\n",
            "[Rec]\r\n",
            "Frame=9\r\n",
            "Type=16\r\n",
            "\n\n",
        );

        assert_eq!(
            encode_control_record_text(&chunks).unwrap(),
            expected.as_bytes()
        );
    }

    #[test]
    fn cpp_text_record_preserves_signed_frames_and_debugrec_value_envelopes() {
        let queued = EngineControlPacket::DebugRecord(DebugRecordControlData {
            data: vec![0x00, 0xff],
        });
        let direct = EngineControlPacket::DebugRecord(DebugRecordControlData {
            data: b"direct".to_vec(),
        });
        let chunks = vec![
            ControlRecordChunk::Controls {
                frame: u32::MAX,
                controls: vec![queued.clone()],
            },
            ControlRecordChunk::ControlPacket {
                frame: 0,
                control: direct.clone(),
            },
            ControlRecordChunk::End { frame: 1 },
        ];

        let text = encode_control_record_text(&chunks).unwrap();
        let rendered = std::str::from_utf8(&text).unwrap();
        assert!(rendered.contains("Frame=-1\r\n"));
        assert!(rendered.contains("Debug Rec=2:\"\\0\\377\"\r\n"));
        assert!(rendered.contains("Debug Rec=6:\"direct\"\r\n"));
        assert!(!rendered.contains("[Debug Rec]"));
        assert_eq!(decode_control_record_text(&text).unwrap(), chunks);

        let binary = encode_control_record_binary(&chunks).unwrap();
        assert_eq!(binary[0], u8::MAX, "-1 rewrites through its low byte");

        let mut playback = ControlRecordPlayback::from_text_bytes(&text).unwrap();
        assert_eq!(playback.take_controls(0), vec![queued, direct]);
    }

    #[test]
    fn classic_recdump_text_input_normalizes_cpp_validated_names() {
        let input = concat!(
            "[Rec]\n",
            "Frame=1\n",
            "Type=1\n",
            "ID=128\n",
            "  [Client Join]\n",
            "    [ClientCore]\n",
            "    Name=\"\"\n",
            "    Nick=\" {<i> </i>{ \"\n",
            "[Rec]\n",
            "Frame=2\n",
            "Type=1\n",
            "ID=144\n",
            "  [Player Info]\n",
            "    [Player]\n",
            "    Name=\"\"\n",
            "    ForcedName=\" <i>Alice</i> \"\n",
            "    LeagueAccount=\" { \"\n",
            "    ClanTag=\"\\t<i>Clan</i>\\r\"\n",
            "[Rec]\n",
            "Frame=3\n",
            "Type=1\n",
            "ID=128\n",
            "  [Client Join]\n",
            "  ByClient=0\n",
        );

        let chunks = decode_control_record_text(input.as_bytes()).unwrap();
        let ControlRecordChunk::ControlPacket {
            control: EngineControlPacket::ClientJoin(join),
            ..
        } = &chunks[0]
        else {
            panic!("expected validated ClientJoin")
        };
        assert_eq!(join.core.name.as_bytes(), b"empty");
        assert_eq!(join.core.nick.as_bytes(), b"Unknown");

        let ControlRecordChunk::ControlPacket {
            control: EngineControlPacket::PlayerInfo(info),
            ..
        } = &chunks[1]
        else {
            panic!("expected validated PlayerInfo")
        };
        let player = &info.players[0];
        assert_eq!(player.name.as_bytes(), b"empty");
        assert_eq!(player.forced_name.as_bytes(), b"Alice");
        assert!(player.league_account.is_empty());
        assert_eq!(player.clan_tag.as_bytes(), b"Clan");

        let ControlRecordChunk::ControlPacket {
            control: EngineControlPacket::ClientJoin(defaulted),
            ..
        } = &chunks[2]
        else {
            panic!("expected defaulted ClientJoin")
        };
        assert!(defaulted.core.name.is_empty());
        assert!(defaulted.core.nick.is_empty());

        let rewritten = encode_control_record_text(&chunks).unwrap();
        let rewritten = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten.contains("    Name=\"empty\"\r\n"));
        assert!(rewritten.contains("    Nick=\"Unknown\"\r\n"));
        assert!(rewritten.contains("    ForcedName=\"Alice\"\r\n"));
        assert!(!rewritten.contains("LeagueAccount="));
        assert!(rewritten.contains("    ClanTag=Clan\r\n"));
    }

    #[test]
    fn playback_returns_all_due_controls_in_file_order_before_the_frame_tick() {
        let first = script();
        let second = synchronize();
        let third = script();
        let mut writer = ControlRecordWriter::new();
        writer.record_packet(2, &first).unwrap();
        writer
            .record_controls(2, std::slice::from_ref(&second))
            .unwrap();
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

        let mut list = encode_control_list_payload(std::slice::from_ref(&control)).unwrap();
        let list_len = list.len();
        list.extend_from_slice(&[0, RCT_END]);
        assert_eq!(
            decode_control_list_prefix(&list),
            Ok((vec![control], list_len))
        );
    }

    #[test]
    fn cpp_text_record_decodes_list_and_direct_control_envelopes() {
        let text = concat!(
            "[Rec]\r\n",
            "Frame=4\r\n",
            "Type=0\r\n",
            "\r\n",
            "  [IDPacket]\r\n",
            "  ID=134\r\n",
            "\r\n",
            "    [Synchronize]\r\n",
            "    SavePlrs=true\r\n",
            "    SyncClear=true\r\n",
            "    ByClient=0\r\n",
            "\n",
            "[Rec]\n",
            "Frame=7\n",
            "Type=1\n",
            "ID=0xA1 trailing text\n",
            "\n",
            "  [Player Control]\n",
            "  Player=3\n",
            "  Com=2\n",
            "  ByClient=4\n",
            "\n",
            "[Rec]\n",
            "Frame=8\n",
            "Type=129\n",
            "Type=130\n",
            "Data=3:\"\\000\\377A\"\n",
            "\n",
            "[Rec]\n",
            "Frame=9\n",
            "Type=2\n",
            "\n",
            "[Rec]\n",
            "Frame=10\n",
            "Type=16\n",
        )
        .as_bytes();

        assert_eq!(
            decode_control_record_text(text).unwrap(),
            vec![
                ControlRecordChunk::Controls {
                    frame: 4,
                    controls: vec![synchronize()],
                },
                ControlRecordChunk::ControlPacket {
                    frame: 7,
                    control: EngineControlPacket::PlayerControl(PlayerControlData {
                        player: 3,
                        command: i32::from(COM_RIGHT),
                        data: 0,
                        by_client: 4,
                    }),
                },
                ControlRecordChunk::DebugRecord {
                    frame: 8,
                    chunk_type: 0x81,
                    debug_type: 0x82,
                    data: vec![0x00, 0xff, b'A'],
                },
                ControlRecordChunk::Frame { frame: 9 },
                ControlRecordChunk::End { frame: 10 },
            ]
        );

        let mut unterminated = ControlRecordPlayback::from_text_bytes(
            concat!("[Rec]\n", "Frame=3\n", "Type=2\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(unterminated.next_frame(), Some(3));
        assert!(unterminated.take_controls(3).is_empty());
        assert_eq!(unterminated.next_frame(), None);
        assert!(
            !unterminated.is_finished(),
            "C++ does not synthesize an End chunk when text input is exhausted"
        );

        let truncated = concat!(
            "[Rec]\n",
            "Frame=2\n",
            "Type=2\n",
            "[Rec]\n",
            "Frame=not-a-number\n",
            "Type=16\n",
        );
        assert_eq!(
            decode_control_record_text(truncated.as_bytes()).unwrap(),
            vec![ControlRecordChunk::Frame { frame: 2 }],
            "C++ treats an unreadable required value as the named-list boundary"
        );

        let file_boundary = concat!(
            "[Rec]\n",
            "Frame=2\n",
            "Type=2\n",
            "[Rec]\n",
            "Frame=3\n",
            "Type=48\n",
            "[Rec]\n",
            "Frame=4\n",
            "Type=2\n",
        );
        assert_eq!(
            decode_control_record_text(file_boundary.as_bytes()).unwrap(),
            vec![ControlRecordChunk::Frame { frame: 2 }],
            "RCT_File has no readable INI payload and ends the native Rec list"
        );

        let malformed_debug_buffer = concat!(
            "[Rec]\n",
            "Frame=2\n",
            "Type=2\n",
            "[Rec]\n",
            "Frame=3\n",
            "Type=129\n",
            "Type=130\n",
            "Data=3junk:\"abc\"\n",
        );
        assert_eq!(
            decode_control_record_text(malformed_debug_buffer.as_bytes()).unwrap(),
            vec![ControlRecordChunk::Frame { frame: 2 }],
            "StdBuf requires its separator immediately after the byte count"
        );

        let invalid_payload_value = concat!(
            "[Rec]\n",
            "Frame=2\n",
            "Type=2\n",
            "[Rec]\n",
            "Frame=3\n",
            "Type=1\n",
            "ID=161\n",
            "  [Player Control]\n",
            "  Player=not-a-number\n",
        );
        assert_eq!(
            decode_control_record_text(invalid_payload_value.as_bytes()).unwrap(),
            vec![ControlRecordChunk::Frame { frame: 2 }],
            "packet NotFound also terminates the repeated native Rec list"
        );

        let prefixed_frame = concat!("[Rec]\n", "Frame=0x10 trailing text\n", "Type=2\n",);
        assert_eq!(
            decode_control_record_text(prefixed_frame.as_bytes()).unwrap(),
            vec![ControlRecordChunk::Frame { frame: 16 }]
        );

        let missing_payload_section = concat!("[Rec]\n", "Frame=16\n", "Type=1\n", "ID=161\n",);
        assert!(matches!(
            decode_control_record_text(missing_payload_section.as_bytes()),
            Err(ControlRecordDecodeError::InvalidTextControlEnvelope { .. })
        ));
    }
}
