use lc_engine::{ControlPacket as EngineControlPacket, PlayerControlData, SyncCheckPacket};

use crate::{ClientId, ControlPacket, Tick};

const PID_NONE: u8 = 0x00;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyControlFrame {
    pub client_id: ClientId,
    pub tick: Tick,
    pub timestamp_ms: u64,
    pub controls: Vec<EngineControlPacket>,
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
    let client_id_raw = reader.read_int32()?;
    if client_id_raw < 0 {
        return Err(LegacyControlError::NegativeClientId(client_id_raw));
    }
    let client_id = client_id_raw as ClientId;
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
            CID_PLR_CONTROL => controls.push(decode_player_control(reader)?),
            CID_SYNC_CHECK => controls.push(decode_sync_check(reader)?),
            other => return Err(LegacyControlError::UnsupportedPacket(other)),
        }
    }
    Ok(controls)
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
    let random_count = reader.read_int32()?;
    let crew_positions_sum = reader.read_int32()?;
    let pxs_count = reader.read_int32()?;
    let mass_mover_index = reader.read_int32()?;
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
    append_int32(buffer, data.random_count);
    append_int32(buffer, data.crew_positions_sum);
    append_int32(buffer, data.pxs_count);
    append_int32(buffer, data.mass_mover_index);
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
    let client_id = i32::try_from(frame.client_id)
        .map_err(|_| LegacyEncodeError::ClientIdOutOfRange(frame.client_id))?;
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
        let packet = ControlPacket::builder(3, 11).payload(payload);
        let error = decode_control_packet(&packet).unwrap_err();
        assert!(matches!(
            error,
            LegacyControlError::TickMismatch {
                header_tick: 11,
                payload_tick: 10
            }
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
        let mut payload = build_payload(-1, 5, &[]);
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::NegativeClientId(-1)));
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
}
