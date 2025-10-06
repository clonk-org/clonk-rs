use crate::{ClientId, ControlPacket, Tick};
use std::convert::TryFrom;
use std::io;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const TCP_FRAME_PREFIX: u8 = 0xFF;
const MAX_PACKET_SIZE: usize = 2 * 1024 * 1024; // 2 MiB cap to guard against bogus frames.
const PID_CONTROL: u8 = 0x40;
const PID_CONTROL_REQ: u8 = 0x41;
const PID_CONTROL_PKT: u8 = 0x42;
const PID_EXEC_SYNC_CTRL: u8 = 0x43;

/// Errors raised while parsing or emitting LegacyClonk network frames.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("packet type 0x{0:02x} is not supported")]
    UnsupportedPacket(u8),
    #[error("malformed packet: {0}")]
    Malformed(&'static str),
    #[error("invalid control delivery value {0}")]
    InvalidDelivery(u8),
    #[error("unexpected end of data while decoding varint")]
    UnexpectedEof,
    #[error("varint exceeds 32-bit range")]
    VarintOverflow,
}

/// Delivery semantics mirrored from `C4ControlDeliveryType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlDelivery {
    Queue = 0,
    Sync = 1,
    Direct = 2,
    Private = 3,
    Decide = 4,
}

impl TryFrom<u8> for ControlDelivery {
    type Error = TransportError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ControlDelivery::Queue),
            1 => Ok(ControlDelivery::Sync),
            2 => Ok(ControlDelivery::Direct),
            3 => Ok(ControlDelivery::Private),
            4 => Ok(ControlDelivery::Decide),
            other => Err(TransportError::InvalidDelivery(other)),
        }
    }
}

impl From<ControlDelivery> for u8 {
    fn from(value: ControlDelivery) -> Self {
        value as u8
    }
}

/// Logical messages reconstructed from LegacyClonk network frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMessage {
    Control(ControlPacket),
    Request {
        from_tick: Tick,
    },
    Packet {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
    },
}

/// Tokio-powered transport that understands LegacyClonk TCP framing and control packets.
#[derive(Debug)]
pub struct ControlTransport<S> {
    stream: S,
}

impl<S> ControlTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub fn into_inner(self) -> S {
        self.stream
    }

    pub async fn read_message(&mut self) -> Result<ControlMessage, TransportError> {
        let mut header = [0u8; 5];
        self.stream.read_exact(&mut header).await?;
        if header[0] != TCP_FRAME_PREFIX {
            return Err(TransportError::Malformed("invalid TCP frame prefix"));
        }
        let size = u32::from_le_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if size > MAX_PACKET_SIZE {
            return Err(TransportError::Malformed("packet exceeds allowed size"));
        }
        let mut body = vec![0u8; size];
        if size > 0 {
            self.stream.read_exact(&mut body).await?;
        }
        parse_body(&body)
    }

    pub async fn send_message(&mut self, message: ControlMessage) -> Result<(), TransportError> {
        let mut payload = Vec::new();
        match message {
            ControlMessage::Control(packet) => {
                payload.push(PID_CONTROL);
                encode_varint(packet.client_id(), &mut payload);
                encode_varint(packet.tick(), &mut payload);
                payload.extend_from_slice(packet.payload());
            }
            ControlMessage::Request { from_tick } => {
                payload.push(PID_CONTROL_REQ);
                encode_varint(from_tick, &mut payload);
            }
            ControlMessage::Packet { delivery, data } => {
                payload.push(PID_CONTROL_PKT);
                payload.push(u8::from(delivery));
                payload.extend_from_slice(&data);
            }
            ControlMessage::ExecSync { control_tick } => {
                payload.push(PID_EXEC_SYNC_CTRL);
                encode_varint(control_tick, &mut payload);
            }
        }

        let size = payload.len() as u32;
        self.stream.write_all(&[TCP_FRAME_PREFIX]).await?;
        self.stream.write_all(&size.to_le_bytes()).await?;
        if !payload.is_empty() {
            self.stream.write_all(&payload).await?;
        }
        self.stream.flush().await?;
        Ok(())
    }
}

fn parse_body(body: &[u8]) -> Result<ControlMessage, TransportError> {
    if body.is_empty() {
        return Err(TransportError::Malformed("missing packet payload"));
    }
    match body[0] {
        PID_CONTROL => parse_control(&body[1..]),
        PID_CONTROL_REQ => parse_request(&body[1..]),
        PID_CONTROL_PKT => parse_packet(&body[1..]),
        PID_EXEC_SYNC_CTRL => parse_exec_sync(&body[1..]),
        other => Err(TransportError::UnsupportedPacket(other)),
    }
}

fn parse_control(data: &[u8]) -> Result<ControlMessage, TransportError> {
    let (client_id, consumed_a) = decode_varint(data)?;
    let (tick, consumed_b) = decode_varint(&data[consumed_a..])?;
    let payload = data[consumed_a + consumed_b..].to_vec();
    Ok(ControlMessage::Control(
        ControlPacket::builder(client_id as ClientId, tick as Tick)
            .timestamp_ms(0)
            .payload(payload),
    ))
}

fn parse_request(data: &[u8]) -> Result<ControlMessage, TransportError> {
    let (tick, consumed) = decode_varint(data)?;
    if consumed != data.len() {
        return Err(TransportError::Malformed(
            "unexpected trailing bytes in control request",
        ));
    }
    Ok(ControlMessage::Request {
        from_tick: tick as Tick,
    })
}

fn parse_packet(data: &[u8]) -> Result<ControlMessage, TransportError> {
    if data.is_empty() {
        return Err(TransportError::Malformed(
            "missing delivery byte for control packet",
        ));
    }
    let delivery = ControlDelivery::try_from(data[0])?;
    Ok(ControlMessage::Packet {
        delivery,
        data: data[1..].to_vec(),
    })
}

fn parse_exec_sync(data: &[u8]) -> Result<ControlMessage, TransportError> {
    let (tick, consumed) = decode_varint(data)?;
    if consumed != data.len() {
        return Err(TransportError::Malformed(
            "unexpected trailing bytes in exec sync packet",
        ));
    }
    Ok(ControlMessage::ExecSync {
        control_tick: tick as Tick,
    })
}

fn encode_varint(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

fn decode_varint(data: &[u8]) -> Result<(u32, usize), TransportError> {
    let mut value: u32 = 0;
    let mut shift = 0;
    for (idx, byte) in data.iter().enumerate() {
        let chunk = (byte & 0x7F) as u32;
        if shift >= 32 && chunk != 0 {
            return Err(TransportError::VarintOverflow);
        }
        value |= chunk << shift;
        if (byte & 0x80) == 0 {
            return Ok((value, idx + 1));
        }
        shift += 7;
        if shift > 28 {
            return Err(TransportError::VarintOverflow);
        }
    }
    Err(TransportError::UnexpectedEof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt as _, AsyncWriteExt as _};

    fn expect_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(5 + payload.len());
        frame.push(TCP_FRAME_PREFIX);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_control_packet() {
        let payload = [PID_CONTROL, 0x0C, 0x22, 0x00];
        let frame = expect_frame(&payload);
        let (mut client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        match transport.read_message().await.unwrap() {
            ControlMessage::Control(packet) => {
                assert_eq!(packet.client_id(), 12);
                assert_eq!(packet.tick(), 34);
                assert_eq!(packet.payload(), &[0x00]);
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_multibyte_varints() {
        // client 300 (0x12C) -> bytes [0xAC, 0x02]; tick 2000 -> [0xD0, 0x0F]
        let payload = [PID_CONTROL, 0xAC, 0x02, 0xD0, 0x0F, 0x00, 0x01, 0x02];
        let frame = expect_frame(&payload);
        let (mut client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        match transport.read_message().await.unwrap() {
            ControlMessage::Control(packet) => {
                assert_eq!(packet.client_id(), 300);
                assert_eq!(packet.tick(), 2000);
                assert_eq!(packet.payload(), &[0x00, 0x01, 0x02]);
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_control_request() {
        let payload = [PID_CONTROL_REQ, 0x96, 0x01]; // tick 150
        let frame = expect_frame(&payload);
        let (mut client, mut server) = duplex(32);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        match transport.read_message().await.unwrap() {
            ControlMessage::Request { from_tick } => assert_eq!(from_tick, 150),
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_exec_sync() {
        let payload = [PID_EXEC_SYNC_CTRL, 0x9B, 0xFB, 0x0B]; // tick 195995
        let frame = expect_frame(&payload);
        let (mut client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        match transport.read_message().await.unwrap() {
            ControlMessage::ExecSync { control_tick } => assert_eq!(control_tick, 195_995),
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_control_packet_matches_protocol() {
        let (client, mut server) = duplex(128);
        let mut transport = ControlTransport::new(client);
        let packet = ControlPacket::builder(12, 34)
            .timestamp_ms(123)
            .payload(vec![0xAA, 0xBB]);
        transport
            .send_message(ControlMessage::Control(packet))
            .await
            .unwrap();
        drop(transport);

        let mut buf = Vec::new();
        server.read_to_end(&mut buf).await.unwrap();
        let expected = expect_frame(&[PID_CONTROL, 0x0C, 0x22, 0xAA, 0xBB]);
        assert_eq!(buf, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_control_request_matches_protocol() {
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);
        transport
            .send_message(ControlMessage::Request { from_tick: 150 })
            .await
            .unwrap();
        drop(transport);

        let mut buf = Vec::new();
        server.read_to_end(&mut buf).await.unwrap();
        let expected = expect_frame(&[PID_CONTROL_REQ, 0x96, 0x01]);
        assert_eq!(buf, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_control_pkt_matches_protocol() {
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);
        transport
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: vec![0x80, 0x01, 0x02, 0x03],
            })
            .await
            .unwrap();
        drop(transport);

        let mut buf = Vec::new();
        server.read_to_end(&mut buf).await.unwrap();
        let expected = expect_frame(&[PID_CONTROL_PKT, 0x02, 0x80, 0x01, 0x02, 0x03]);
        assert_eq!(buf, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_invalid_prefix() {
        let mut frame = expect_frame(&[PID_CONTROL, 0x00]);
        frame[0] = 0xAA;
        let (mut client, mut server) = duplex(16);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        let err = transport.read_message().await.unwrap_err();
        match err {
            TransportError::Malformed(_) => {}
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
