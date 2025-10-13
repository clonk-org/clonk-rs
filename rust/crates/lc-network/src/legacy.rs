use lc_engine::{ControlPacket as EngineControlPacket, PlayerControlData};

use crate::{ClientId, ControlPacket, Tick};

const PID_NONE: u8 = 0x00;
const CID_PLR_CONTROL: u8 = 0x80 | 0x21;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyControlFrame {
    pub client_id: ClientId,
    pub tick: Tick,
    pub timestamp_ms: u64,
    pub controls: Vec<EngineControlPacket>,
}

pub fn decode_control_packet(packet: &ControlPacket) -> Result<LegacyControlFrame, LegacyControlError> {
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

fn decode_control_list(reader: &mut Reader<'_>) -> Result<Vec<EngineControlPacket>, LegacyControlError> {
    let mut controls = Vec::new();
    loop {
        let id = reader.read_u8()?;
        if id == PID_NONE {
            break;
        }
        match id {
            CID_PLR_CONTROL => controls.push(decode_player_control(reader)?),
            other => return Err(LegacyControlError::UnsupportedPacket(other)),
        }
    }
    Ok(controls)
}

fn decode_player_control(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn encode_player_control(player: i32, command: i32, data: i32, by_client: i32) -> Vec<u8> {
        let mut bytes = vec![CID_PLR_CONTROL];
        bytes.extend(encode_int32(player));
        bytes.extend(encode_int32(command));
        bytes.extend(encode_int32(data));
        bytes.extend(encode_int32(by_client));
        bytes
    }

    fn build_payload(client: i32, tick: i32, controls: &[[i32; 4]]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend(encode_int32(client));
        payload.extend(encode_int32(tick));
        for control in controls {
            payload.extend(encode_player_control(control[0], control[1], control[2], control[3]));
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
            LegacyControlError::ClientIdMismatch { header_id: 4, payload_id: 3 }
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
}
