use lc_engine::{
    ControlPacket as EngineControlPacket, JoinPlayerControlData, JoinPlayerSource, LegacyCString,
    PlayerControlData, SyncCheckPacket,
};

use crate::{ClientId, ControlPacket, ReadyBatch, Tick, BROADCAST_CLIENT_ID};

// C4PacketType::PID_None (src/C4PacketBase.h). Binary C4PacketList values
// terminate with a default C4IDPacket carrying this byte.
const PID_NONE: u8 = 0xff;
const CID_JOIN_PLR: u8 = 0x80 | 0x11;
const CID_PLR_CONTROL: u8 = 0x80 | 0x21;
const CID_SYNC_CHECK: u8 = 0x80 | 0x05;
const MAX_VARINT_BYTES: usize = 5;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LegacyControlError {
    #[error("control payload is empty")]
    EmptyPayload,
    #[error("control packet is truncated")]
    UnexpectedEof,
    #[error("packed integer exceeds supported size")]
    VarintOverflow,
    #[error("control packet {0:#x} is not supported yet")]
    UnsupportedPacket(u8),
    #[error("resource-backed JoinPlayer controls are not supported yet")]
    UnsupportedResourceJoin,
    #[error("control payload contained negative client id {0}")]
    NegativeClientId(i32),
    #[error("control payload contained negative tick {0}")]
    NegativeTick(i32),
    #[error("control payload reported client {payload_id} but header contained {header_id}")]
    ClientIdMismatch {
        header_id: ClientId,
        payload_id: ClientId,
    },
    #[error("control payload reported tick {payload_tick} but header contained {header_tick}")]
    TickMismatch {
        header_tick: Tick,
        payload_tick: Tick,
    },
    #[error("control payload contained trailing data")]
    TrailingData,
    #[error("control payload does not end with a PID_NONE list terminator")]
    MissingListTerminator,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LegacyEncodeError {
    #[error("control packet variant is not supported yet")]
    UnsupportedPacket,
    #[error("client id {0} exceeds supported range")]
    ClientIdOutOfRange(ClientId),
    #[error("control tick {0} exceeds supported range")]
    TickOutOfRange(Tick),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LegacyAggregateError {
    #[error("control packet for client {client_id} could not be decoded: {source}")]
    Decode {
        client_id: ClientId,
        source: LegacyControlError,
    },
    #[error("control packet for client {client_id} has tick {packet_tick}, expected {tick}")]
    TickMismatch {
        client_id: ClientId,
        tick: Tick,
        packet_tick: Tick,
    },
    #[error("complete control packet could not be encoded: {0}")]
    Encode(#[from] LegacyEncodeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyControlFrame {
    pub client_id: ClientId,
    pub tick: Tick,
    pub timestamp_ms: u64,
    pub controls: Vec<EngineControlPacket>,
}

/// The validated outer fields and opaque C4Control list body of one legacy
/// packet. The body excludes its final `PID_NONE`, so callers can concatenate
/// multiple source lists exactly like `C4GameControlPacket::Add` without
/// needing to understand every control ID.
pub(crate) struct LegacyControlEnvelope<'a> {
    pub(crate) client_id: ClientId,
    pub(crate) tick: Tick,
    pub(crate) control_body: &'a [u8],
}

/// Validate the packet/payload header agreement and split off the one final
/// C4Control list terminator. Control entries deliberately stay opaque: C++
/// `PackCompleteCtrl` appends packet lists rather than decoding individual
/// control variants (src/C4GameControlNetwork.cpp:759-769).
pub(crate) fn validate_control_envelope(
    packet: &ControlPacket,
) -> Result<LegacyControlEnvelope<'_>, LegacyControlError> {
    let payload = packet.payload();
    if payload.is_empty() {
        return Err(LegacyControlError::EmptyPayload);
    }

    let mut reader = Reader::new(payload);
    let client_id = decode_client_id(reader.read_int32()?)?;
    let tick_raw = reader.read_int32()?;
    if tick_raw < 0 {
        return Err(LegacyControlError::NegativeTick(tick_raw));
    }
    let tick = tick_raw as Tick;

    if client_id != packet.client_id() {
        return Err(LegacyControlError::ClientIdMismatch {
            header_id: packet.client_id(),
            payload_id: client_id,
        });
    }
    if tick != packet.tick() {
        return Err(LegacyControlError::TickMismatch {
            header_tick: packet.tick(),
            payload_tick: tick,
        });
    }

    let list = &payload[reader.offset..];
    let Some((&terminator, control_body)) = list.split_last() else {
        return Err(LegacyControlError::MissingListTerminator);
    };
    if terminator != PID_NONE {
        return Err(LegacyControlError::MissingListTerminator);
    }

    Ok(LegacyControlEnvelope {
        client_id,
        tick,
        control_body,
    })
}

fn decode_client_id(client_id_raw: i32) -> Result<ClientId, LegacyControlError> {
    if client_id_raw == -1 {
        // C4ClientIDAll aliases C4ClientIDUnknown (-1) for a complete control
        // packet (C4GameControlNetwork.h:25-27).
        Ok(BROADCAST_CLIENT_ID)
    } else if client_id_raw < 0 {
        Err(LegacyControlError::NegativeClientId(client_id_raw))
    } else {
        Ok(client_id_raw as ClientId)
    }
}

pub fn decode_control_packet(
    packet: &ControlPacket,
) -> Result<LegacyControlFrame, LegacyControlError> {
    let mut frame = decode_control_payload(packet.payload())?;
    let header_client = packet.client_id();
    let header_tick = packet.tick();
    if frame.client_id != header_client {
        return Err(LegacyControlError::ClientIdMismatch {
            header_id: header_client,
            payload_id: frame.client_id,
        });
    }
    if frame.tick != header_tick {
        return Err(LegacyControlError::TickMismatch {
            header_tick,
            payload_tick: frame.tick,
        });
    }
    frame.timestamp_ms = packet.timestamp_ms();
    Ok(frame)
}

pub fn decode_control_payload(payload: &[u8]) -> Result<LegacyControlFrame, LegacyControlError> {
    if payload.is_empty() {
        return Err(LegacyControlError::EmptyPayload);
    }
    let mut reader = Reader::new(payload);
    let client_id = decode_client_id(reader.read_int32()?)?;
    let tick_raw = reader.read_int32()?;
    if tick_raw < 0 {
        return Err(LegacyControlError::NegativeTick(tick_raw));
    }
    let tick = tick_raw as Tick;

    let controls = decode_control_list(&mut reader)?;

    if reader.remaining() != 0 {
        return Err(LegacyControlError::TrailingData);
    }

    Ok(LegacyControlFrame {
        client_id,
        tick,
        timestamp_ms: 0,
        controls,
    })
}

fn decode_control_list(
    reader: &mut Reader<'_>,
) -> Result<Vec<EngineControlPacket>, LegacyControlError> {
    let mut controls = Vec::new();
    loop {
        let id = reader.read_u8()?;
        if id == PID_NONE {
            break;
        }
        match id {
            CID_JOIN_PLR => controls.push(decode_join_player(reader)?),
            CID_PLR_CONTROL => controls.push(decode_player_control(reader)?),
            CID_SYNC_CHECK => controls.push(decode_sync_check(reader)?),
            other => return Err(LegacyControlError::UnsupportedPacket(other)),
        }
    }
    Ok(controls)
}

fn decode_join_player(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let filename = reader.read_c_string()?;
    let at_client = reader.read_int32()?;
    let info_id = reader.read_int32()?;
    let by_resource = reader.read_u8()? != 0;
    if by_resource {
        return Err(LegacyControlError::UnsupportedResourceJoin);
    }
    let player_data_len = reader.read_uint32()? as usize;
    let player_data = reader.read_bytes(player_data_len)?.to_vec();
    let by_client = reader.read_int32()?;

    Ok(EngineControlPacket::JoinPlayer(JoinPlayerControlData {
        filename,
        at_client,
        info_id,
        source: JoinPlayerSource::Embedded(player_data),
        by_client,
    }))
}

fn decode_player_control(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let player = reader.read_int32()?;
    let command = reader.read_int32()?;
    let data = reader.read_int32()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::PlayerControl(PlayerControlData {
        player,
        command,
        data,
        by_client,
    }))
}

fn decode_sync_check(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let frame = reader.read_int32()?;
    let control_tick = reader.read_int32()?;
    let random3 = reader.read_int32()?;
    let random_count = reader.read_raw_i32()?;
    let crew_positions_sum = reader.read_int32()?;
    let pxs_count = reader.read_int32()?;
    let mass_mover_index = reader.read_raw_i32()?;
    let object_count = reader.read_int32()?;
    let object_enumeration_index = reader.read_int32()?;
    let sector_shape_sum = reader.read_int32()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::SyncCheck(SyncCheckPacket {
        frame,
        control_tick,
        random3,
        random_count,
        crew_positions_sum,
        pxs_count,
        mass_mover_index,
        object_count,
        object_enumeration_index,
        sector_shape_sum,
        by_client,
    }))
}

struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    fn read_u8(&mut self) -> Result<u8, LegacyControlError> {
        if self.offset >= self.data.len() {
            return Err(LegacyControlError::UnexpectedEof);
        }
        let byte = self.data[self.offset];
        self.offset += 1;
        Ok(byte)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], LegacyControlError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_c_string(&mut self) -> Result<LegacyCString, LegacyControlError> {
        let remaining = self
            .data
            .get(self.offset..)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let len = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let bytes = self.read_bytes(len)?.to_vec();
        self.read_u8()?;
        LegacyCString::from_bytes(bytes).ok_or(LegacyControlError::UnexpectedEof)
    }

    fn read_uint32(&mut self) -> Result<u32, LegacyControlError> {
        let mut value = 0u32;
        for shift in (0..32).step_by(7).take(MAX_VARINT_BYTES) {
            let byte = self.read_u8()?;
            value |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(LegacyControlError::VarintOverflow)
    }

    fn read_int32(&mut self) -> Result<i32, LegacyControlError> {
        let mut tmp = self.read_u8()? as i32;
        let mut bytes_read = 1;
        let mut val = clear_upper_i32(tmp);
        let mut data = val;
        let mut shift = 7;

        while (data as u8) != tmp as u8 {
            if bytes_read >= MAX_VARINT_BYTES {
                return Err(LegacyControlError::VarintOverflow);
            }
            tmp = self.read_u8()? as i32;
            bytes_read += 1;
            data = clear_upper_i32(tmp);
            let lower_mask = if shift >= 63 {
                -1i64
            } else {
                (1i64 << shift) - 1
            };
            let preserved = (val as i64) & lower_mask;
            let combined = ((data as i64) << shift) | preserved;
            val = combined as i32;
            shift += 7;
        }

        Ok(val)
    }

    fn read_raw_i32(&mut self) -> Result<i32, LegacyControlError> {
        let end = self
            .offset
            .checked_add(size_of::<i32>())
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(LegacyControlError::UnexpectedEof)?;
        self.offset = end;
        Ok(i32::from_ne_bytes(bytes))
    }
}

fn clear_upper_i32(value: i32) -> i32 {
    (value << 25) >> 25
}

fn encode_int32(mut value: i32) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let chunk = clear_upper_i32(value);
        if chunk == value {
            bytes.push(chunk as u8);
            break;
        } else {
            bytes.push((chunk ^ 0x80) as u8);
            value >>= 7;
        }
    }
    bytes
}

fn append_int32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend(encode_int32(value));
}

fn append_raw_i32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend(value.to_ne_bytes());
}

fn encode_player_control(buffer: &mut Vec<u8>, data: &PlayerControlData) {
    buffer.push(CID_PLR_CONTROL);
    append_int32(buffer, data.player);
    append_int32(buffer, data.command);
    append_int32(buffer, data.data);
    append_int32(buffer, data.by_client);
}

fn encode_sync_check(buffer: &mut Vec<u8>, data: &SyncCheckPacket) {
    buffer.push(CID_SYNC_CHECK);
    append_int32(buffer, data.frame);
    append_int32(buffer, data.control_tick);
    append_int32(buffer, data.random3);
    append_raw_i32(buffer, data.random_count);
    append_int32(buffer, data.crew_positions_sum);
    append_int32(buffer, data.pxs_count);
    append_raw_i32(buffer, data.mass_mover_index);
    append_int32(buffer, data.object_count);
    append_int32(buffer, data.object_enumeration_index);
    append_int32(buffer, data.sector_shape_sum);
    append_int32(buffer, data.by_client);
}

fn encode_controls(
    controls: &[EngineControlPacket],
    buffer: &mut Vec<u8>,
) -> Result<(), LegacyEncodeError> {
    for control in controls {
        match control {
            EngineControlPacket::PlayerControl(data) => encode_player_control(buffer, data),
            EngineControlPacket::SyncCheck(data) => encode_sync_check(buffer, data),
            _ => return Err(LegacyEncodeError::UnsupportedPacket),
        }
    }
    Ok(())
}

pub fn encode_control_payload(frame: &LegacyControlFrame) -> Result<Vec<u8>, LegacyEncodeError> {
    let client_id = if frame.client_id == BROADCAST_CLIENT_ID {
        // C4GameControlNetwork::PackCompleteCtrl writes C4ClientIDAll (-1) for
        // the merged packet (src/C4GameControlNetwork.cpp:759-768).
        -1
    } else {
        i32::try_from(frame.client_id)
            .map_err(|_| LegacyEncodeError::ClientIdOutOfRange(frame.client_id))?
    };
    let tick =
        i32::try_from(frame.tick).map_err(|_| LegacyEncodeError::TickOutOfRange(frame.tick))?;
    let mut payload = Vec::new();
    append_int32(&mut payload, client_id);
    append_int32(&mut payload, tick);
    encode_controls(&frame.controls, &mut payload)?;
    payload.push(PID_NONE);
    Ok(payload)
}

pub fn encode_control_packet(
    frame: &LegacyControlFrame,
) -> Result<ControlPacket, LegacyEncodeError> {
    let payload = encode_control_payload(frame)?;
    Ok(ControlPacket::builder(frame.client_id, frame.tick)
        .timestamp_ms(frame.timestamp_ms)
        .payload(payload))
}

/// Merge a coordinator-ready batch into the one complete control packet used
/// by the legacy lockstep protocol.
///
/// `C4GameControlNetwork::PackCompleteCtrl` waits for every client, marks the
/// result as `C4ClientIDAll`, and appends each client's control list in client
/// ID order (src/C4GameControlNetwork.cpp:741-777). Envelope validation strips
/// only per-client headers and final list terminators; individual control
/// entries remain opaque so all C++ control IDs are preserved verbatim.
pub fn aggregate_ready_batch(batch: &ReadyBatch) -> Result<ControlPacket, LegacyAggregateError> {
    aggregate_control_packets_for_tick(batch.tick(), batch.packets())
}

pub(crate) fn aggregate_control_packets_for_tick(
    tick: Tick,
    packets: &[ControlPacket],
) -> Result<ControlPacket, LegacyAggregateError> {
    let mut packets = packets.iter().collect::<Vec<_>>();
    packets.sort_by_key(|packet| packet.client_id());

    let mut timestamp_ms = 0;
    let mut control_body = Vec::new();
    for packet in packets {
        if packet.tick() != tick {
            return Err(LegacyAggregateError::TickMismatch {
                client_id: packet.client_id(),
                tick,
                packet_tick: packet.tick(),
            });
        }
        let envelope =
            validate_control_envelope(packet).map_err(|source| LegacyAggregateError::Decode {
                client_id: packet.client_id(),
                source,
            })?;
        if envelope.tick != tick {
            return Err(LegacyAggregateError::TickMismatch {
                client_id: envelope.client_id,
                tick,
                packet_tick: envelope.tick,
            });
        }
        timestamp_ms = timestamp_ms.max(packet.timestamp_ms());
        control_body.extend_from_slice(envelope.control_body);
    }

    let tick_raw = i32::try_from(tick)
        .map_err(|_| LegacyAggregateError::Encode(LegacyEncodeError::TickOutOfRange(tick)))?;
    let mut payload = Vec::new();
    append_int32(&mut payload, -1);
    append_int32(&mut payload, tick_raw);
    payload.extend(control_body);
    payload.push(PID_NONE);
    Ok(ControlPacket::builder(BROADCAST_CLIENT_ID, tick)
        .timestamp_ms(timestamp_ms)
        .payload(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_payload(client: i32, tick: i32, controls: &[[i32; 4]]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend(super::encode_int32(client));
        payload.extend(super::encode_int32(tick));
        for control in controls {
            let data = PlayerControlData {
                player: control[0],
                command: control[1],
                data: control[2],
                by_client: control[3],
            };
            let mut encoded = Vec::new();
            super::encode_player_control(&mut encoded, &data);
            payload.extend(encoded);
        }
        payload.push(PID_NONE);
        payload
    }

    #[test]
    fn decodes_single_player_control() {
        let payload = build_payload(2, 42, &[[1, 5, 0, 2]]);
        let frame = decode_control_payload(&payload).expect("decode succeeds");
        assert_eq!(frame.client_id, 2);
        assert_eq!(frame.tick, 42);
        assert_eq!(frame.controls.len(), 1);
        match &frame.controls[0] {
            EngineControlPacket::PlayerControl(data) => {
                assert_eq!(data.player, 1);
                assert_eq!(data.command, 5);
                assert_eq!(data.data, 0);
                assert_eq!(data.by_client, 2);
            }
            other => panic!("unexpected control packet: {other:?}"),
        }
    }

    #[test]
    fn decodes_cpp_embedded_join_player_bytes() {
        // C4ControlJoinPlayer writes Filename, packed AtClient/InfoID, the
        // raw ByRes bool, embedded StdBuf, and packed ByClient in that order;
        // the containing C4PacketList then writes PID_None (0xff)
        // (src/C4Control.cpp:852-863; src/StdBuf.cpp:86-100;
        // src/C4Packet2.cpp:193-220,298-335).
        let payload = vec![
            0x04, 0x40, 0x00, 0x91, b'P', 0x80, 0x00, 0xff, 0x40, 0x00, 0x00, 0x03, 0xaa, 0x00,
            0xcc, 0x04, 0xff,
        ];

        let frame = decode_control_payload(&payload).expect("C++ JoinPlayer bytes decode");

        assert_eq!(frame.client_id, 4);
        assert_eq!(frame.tick, 64);
        assert_eq!(frame.controls.len(), 1);
        match &frame.controls[0] {
            EngineControlPacket::JoinPlayer(join) => {
                assert_eq!(join.filename.as_bytes(), b"P\x80");
                assert_eq!(join.at_client, -1);
                assert_eq!(join.info_id, 64);
                assert_eq!(
                    join.source,
                    lc_engine::JoinPlayerSource::Embedded(vec![0xaa, 0x00, 0xcc])
                );
                assert_eq!(join.by_client, 4);
            }
            other => panic!("expected JoinPlayer, got {other:?}"),
        }
    }

    #[test]
    fn cpp_control_lists_use_ff_as_pid_none() {
        // C4PacketType::PID_None is 0xff (src/C4PacketBase.h), and the
        // binary C4PacketList writer appends that default C4IDPacket as the
        // list terminator (src/C4Packet2.cpp).
        let frame = LegacyControlFrame {
            client_id: 2,
            tick: 42,
            timestamp_ms: 0,
            controls: Vec::new(),
        };
        let encoded = encode_control_payload(&frame).expect("empty control list encodes");
        assert_eq!(encoded.last(), Some(&0xff));
        assert!(decode_control_payload(&encoded).is_ok());

        let mut zero_terminated = encoded;
        *zero_terminated.last_mut().expect("payload has terminator") = 0x00;
        assert!(matches!(
            decode_control_payload(&zero_terminated),
            Err(LegacyControlError::UnsupportedPacket(0x00))
        ));
    }

    #[test]
    fn rejects_unsupported_packet() {
        let mut payload = build_payload(1, 1, &[]);
        let insert_at = payload
            .len()
            .checked_sub(1)
            .expect("payload includes terminator");
        payload.insert(insert_at, CID_PLR_CONTROL + 1);
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::UnsupportedPacket(_)));
    }

    #[test]
    fn decode_matches_header_validation() {
        let payload = build_payload(5, 77, &[[5, 64, 0, 5]]);
        let packet = ControlPacket::builder(5, 77)
            .timestamp_ms(1234)
            .payload(payload);
        let frame = decode_control_packet(&packet).expect("decode with header succeeds");
        assert_eq!(frame.timestamp_ms, 1234);
        assert_eq!(frame.controls.len(), 1);
    }

    #[test]
    fn detects_mismatched_header() {
        let payload = build_payload(3, 10, &[]);
        let packet = ControlPacket::builder(4, 10).payload(payload.clone());
        let error = decode_control_packet(&packet).unwrap_err();
        assert!(matches!(
            error,
            LegacyControlError::ClientIdMismatch {
                header_id: 4,
                payload_id: 3
            }
        ));
        assert!(matches!(
            validate_control_envelope(&ControlPacket::builder(4, 10).payload(payload.clone())),
            Err(LegacyControlError::ClientIdMismatch {
                header_id: 4,
                payload_id: 3
            })
        ));
        let packet = ControlPacket::builder(3, 11).payload(payload);
        let error = decode_control_packet(&packet).unwrap_err();
        assert!(matches!(
            error,
            LegacyControlError::TickMismatch {
                header_tick: 11,
                payload_tick: 10
            }
        ));
        assert!(matches!(
            validate_control_envelope(&packet),
            Err(LegacyControlError::TickMismatch {
                header_tick: 11,
                payload_tick: 10
            })
        ));
    }

    #[test]
    fn detects_truncated_payload() {
        let mut payload = build_payload(1, 2, &[[1, 2, 3, 4]]);
        payload.pop();
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::UnexpectedEof));
    }

    #[test]
    fn rejects_negative_client_or_tick() {
        let mut payload = build_payload(-2, 5, &[]);
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::NegativeClientId(-2)));
        payload = build_payload(1, -2, &[]);
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::NegativeTick(-2)));
    }

    #[test]
    fn encode_and_decode_roundtrip() {
        let frame = LegacyControlFrame {
            client_id: 3,
            tick: 12,
            timestamp_ms: 77,
            controls: vec![EngineControlPacket::PlayerControl(PlayerControlData {
                player: 1,
                command: 5,
                data: 0,
                by_client: 3,
            })],
        };
        let packet = encode_control_packet(&frame).expect("encoding succeeds");
        let decoded = decode_control_packet(&packet).expect("decoding succeeds");
        assert_eq!(decoded.client_id, frame.client_id);
        assert_eq!(decoded.tick, frame.tick);
        assert_eq!(decoded.timestamp_ms, frame.timestamp_ms);
        assert_eq!(decoded.controls, frame.controls);
    }

    #[test]
    fn encode_and_decode_sync_check() {
        let sync = SyncCheckPacket {
            frame: 120,
            control_tick: 118,
            random3: 33,
            random_count: 77,
            crew_positions_sum: 1024,
            pxs_count: 45,
            mass_mover_index: 12,
            object_count: 256,
            object_enumeration_index: 512,
            sector_shape_sum: 2048,
            by_client: 5,
        };
        let frame = LegacyControlFrame {
            client_id: 5,
            tick: 120,
            timestamp_ms: 55,
            controls: vec![EngineControlPacket::SyncCheck(sync.clone())],
        };
        let packet = encode_control_packet(&frame).expect("encode succeeds");
        let decoded = decode_control_packet(&packet).expect("decode succeeds");
        assert_eq!(decoded.client_id, frame.client_id);
        assert_eq!(decoded.tick, frame.tick);
        assert_eq!(decoded.timestamp_ms, frame.timestamp_ms);
        assert_eq!(decoded.controls.len(), 1);
        match &decoded.controls[0] {
            EngineControlPacket::SyncCheck(decoded_sync) => {
                assert_eq!(decoded_sync, &sync);
            }
            other => panic!("expected sync check control, got {other:?}"),
        }
    }

    #[test]
    fn sync_check_raw_dwords_match_cpp_binary_compiler() {
        // C++ oracle: C4ControlSyncCheck::CompileFunc packs most fields but
        // writes RandomCount and MassMoverIndex as plain int32 values
        // (src/C4Control.cpp:522-534). StdCompilerBinWrite::DWord writes those
        // four native bytes verbatim (src/StdCompiler.cpp:104-112,125-132).
        let sync = SyncCheckPacket {
            frame: 7,
            control_tick: 8,
            random3: 9,
            random_count: 0x0102_0304,
            crew_positions_sum: 10,
            pxs_count: 11,
            mass_mover_index: 0x1213_1415,
            object_count: 12,
            object_enumeration_index: 13,
            sector_shape_sum: 14,
            by_client: 5,
        };
        let frame = LegacyControlFrame {
            client_id: 5,
            tick: 6,
            timestamp_ms: 0,
            controls: vec![EngineControlPacket::SyncCheck(sync.clone())],
        };

        let mut expected = vec![5, 6, CID_SYNC_CHECK, 7, 8, 9];
        expected.extend(sync.random_count.to_ne_bytes());
        expected.extend([10, 11]);
        expected.extend(sync.mass_mover_index.to_ne_bytes());
        expected.extend([12, 13, 14, 5, PID_NONE]);

        assert_eq!(
            encode_control_payload(&frame).expect("sync check encodes"),
            expected,
            "raw DWord fields must not use packed-int encoding"
        );
        assert_eq!(
            decode_control_payload(&expected)
                .expect("C++ sync-check bytes decode")
                .controls,
            frame.controls
        );
    }

    #[test]
    fn aggregate_complete_control_is_ordered_and_has_one_list_terminator() {
        let host_control = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 2,
            data: 10,
            by_client: 0,
        });
        let client_control = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 1,
            command: 5,
            data: 20,
            by_client: 1,
        });
        let packet = |client_id, timestamp_ms, control| {
            encode_control_packet(&LegacyControlFrame {
                client_id,
                tick: 7,
                timestamp_ms,
                controls: vec![control],
            })
            .expect("per-client control encodes")
        };

        // Deliberately supply client 1 before host 0. PackCompleteCtrl appends
        // controls in client-ID order (src/C4GameControlNetwork.cpp:760-769).
        let complete = aggregate_control_packets_for_tick(
            7,
            &[
                packet(1, 20, client_control.clone()),
                packet(0, 10, host_control.clone()),
            ],
        )
        .expect("ready controls aggregate");

        assert_eq!(complete.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(complete.tick(), 7);
        assert_eq!(complete.timestamp_ms(), 20);
        assert_eq!(complete.payload().last(), Some(&0xff));
        let decoded = decode_control_packet(&complete)
            .expect("one merged list decodes without trailing per-client data");
        assert_eq!(decoded.client_id, BROADCAST_CLIENT_ID);
        assert_eq!(decoded.controls, vec![host_control, client_control]);
    }

    #[test]
    fn aggregate_preserves_opaque_unsupported_controls_in_client_order() {
        let opaque_packet = |client_id: ClientId, body: &[u8]| {
            let mut payload = Vec::new();
            append_int32(&mut payload, client_id as i32);
            append_int32(&mut payload, 9);
            payload.extend_from_slice(body);
            payload.push(PID_NONE);
            ControlPacket::builder(client_id, 9).payload(payload)
        };
        let host_body = [0x89, 0x31];
        let client_body = [0x88, 0x41, 0x42];

        let complete = aggregate_control_packets_for_tick(
            9,
            &[opaque_packet(1, &client_body), opaque_packet(0, &host_body)],
        )
        .expect("opaque legacy controls aggregate");
        let envelope = validate_control_envelope(&complete).expect("complete envelope validates");

        assert_eq!(complete.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(envelope.client_id, BROADCAST_CLIENT_ID);
        assert_eq!(envelope.tick, 9);
        assert_eq!(
            envelope.control_body,
            [host_body.as_slice(), client_body.as_slice()].concat(),
            "PackCompleteCtrl preserves opaque lists and orders host before client"
        );
        assert_eq!(complete.payload().last(), Some(&0xff));
        assert!(matches!(
            decode_control_packet(&complete),
            Err(LegacyControlError::UnsupportedPacket(0x89))
        ));
    }

    #[test]
    fn envelope_validator_rejects_a_missing_final_list_terminator() {
        let mut payload = Vec::new();
        append_int32(&mut payload, 3);
        append_int32(&mut payload, 4);
        payload.extend([0x88, 0x7f]);
        let packet = ControlPacket::builder(3, 4).payload(payload);

        assert!(matches!(
            validate_control_envelope(&packet),
            Err(LegacyControlError::MissingListTerminator)
        ));
        assert!(matches!(
            aggregate_control_packets_for_tick(4, &[packet]),
            Err(LegacyAggregateError::Decode {
                client_id: 3,
                source: LegacyControlError::MissingListTerminator,
            })
        ));
    }
}
