use crate::legacy::{
    decode_player_info_update_payload, encode_player_info_update_payload, LegacyControlError,
    LegacyEncodeError,
};
use crate::{ClientId, ControlPacket, Tick};
use lc_engine::PlayerInfoUpdateRequest;
use std::convert::TryFrom;
use std::io;
use std::mem::size_of;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const TCP_FRAME_PREFIX: u8 = 0xFF;
const MAX_PACKET_SIZE: usize = 2 * 1024 * 1024; // 2 MiB cap to guard against bogus frames.
const PID_STATUS: u8 = 0x10;
const PID_STATUS_ACK: u8 = 0x11;
const PID_CLIENT_ACT_REQ: u8 = 0x13;
const PID_PLAYER_INFO_UPDATE_REQ: u8 = 0x16;
const PID_LOBBY_COUNTDOWN: u8 = 0x20;
const PID_READY_CHECK: u8 = 0x21;
const PID_CONTROL: u8 = 0x40;
const PID_CONTROL_REQ: u8 = 0x41;
const PID_CONTROL_PKT: u8 = 0x42;
const PID_EXEC_SYNC_CTRL: u8 = 0x43;

pub const NETWORK_STATE_NONE: u8 = 0;
pub const NETWORK_STATE_INIT: u8 = 1;
pub const NETWORK_STATE_LOBBY: u8 = 2;
pub const NETWORK_STATE_PAUSE: u8 = 3;
pub const NETWORK_STATE_GO: u8 = 4;

/// Exact `C4Network2Status` payload shared by `PID_Status` and
/// `PID_StatusAck` (`src/C4Network2.cpp:103-123`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkStatus {
    pub state: u8,
    pub control_mode: i32,
    pub target_tick: i32,
}

/// Exact `C4GameLobby::C4PacketCountdown` payload carried by
/// `PID_LobbyCountdown` (`src/C4GameLobby.h:49-63`).
///
/// `seconds == -1` aborts a countdown. Other values are kept verbatim because
/// the C++ binary compiler serializes the signed `int32_t` field without an
/// integer-packing adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyCountdown {
    pub seconds: i32,
}

impl LobbyCountdown {
    pub const ABORT_SECONDS: i32 = -1;
    pub const ALMOST_START_SECONDS: i32 = 10;

    pub const fn new(seconds: i32) -> Self {
        Self { seconds }
    }

    pub const fn abort() -> Self {
        Self::new(Self::ABORT_SECONDS)
    }

    pub const fn is_abort(self) -> bool {
        self.seconds == Self::ABORT_SECONDS
    }

    /// Whether the C++ one-second timer broadcasts this countdown value.
    ///
    /// The initial value and abort are sent unconditionally by their callers;
    /// this predicate is for subsequent timer ticks only
    /// (`src/C4GameLobby.cpp:1138-1161`).
    pub const fn timer_tick_is_broadcast(self) -> bool {
        self.seconds >= 0
            && (self.seconds <= Self::ALMOST_START_SECONDS
                || (self.seconds <= 600 && self.seconds % 10 == 0)
                || self.seconds % 60 == 0)
    }
}

/// The signed `Data` field of `C4PacketReadyCheck`.
///
/// Unknown values are retained instead of rejected: the C++ receiver treats
/// every value other than `Request` and `Ready` as a not-ready reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyCheckData {
    Request,
    NotReady,
    Ready,
    Unknown(i32),
}

impl ReadyCheckData {
    pub const fn from_wire(value: i32) -> Self {
        match value {
            -1 => Self::Request,
            0 => Self::NotReady,
            1 => Self::Ready,
            other => Self::Unknown(other),
        }
    }

    pub const fn wire_value(self) -> i32 {
        match self {
            Self::Request => -1,
            Self::NotReady => 0,
            Self::Ready => 1,
            Self::Unknown(value) => value,
        }
    }

    pub const fn is_request(self) -> bool {
        matches!(self, Self::Request)
    }

    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Exact `C4PacketReadyCheck` payload carried by `PID_ReadyCheck`
/// (`src/C4Network2.h:480-502`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadyCheck {
    /// Signed because the legacy field is a raw C++ `int32_t`.
    pub client_id: i32,
    pub data: ReadyCheckData,
}

impl ReadyCheck {
    pub const fn request(client_id: i32) -> Self {
        Self {
            client_id,
            data: ReadyCheckData::Request,
        }
    }

    pub const fn reply(client_id: i32, ready: bool) -> Self {
        Self {
            client_id,
            data: if ready {
                ReadyCheckData::Ready
            } else {
                ReadyCheckData::NotReady
            },
        }
    }
}

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
    #[error("execute-sync packet contained negative control tick {0}")]
    NegativeControlTick(i32),
    #[error("execute-sync control tick {0} exceeds C++ int32 range")]
    ControlTickOutOfRange(Tick),
    #[error("invalid player-info update request: {0}")]
    PlayerInfoUpdateDecode(#[source] LegacyControlError),
    #[error("failed to encode player-info update request: {0}")]
    PlayerInfoUpdateEncode(#[source] LegacyEncodeError),
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
    Status(NetworkStatus),
    StatusAck(NetworkStatus),
    LobbyCountdown(LobbyCountdown),
    ReadyCheck(ReadyCheck),
    ActivationRequest {
        tick: i32,
    },
    PlayerInfoUpdate(PlayerInfoUpdateRequest),
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

/// Length of the frame header: prefix byte plus little-endian u32 size.
const FRAME_HEADER_LEN: usize = 5;

/// Tokio-powered transport that understands LegacyClonk TCP framing and control packets.
#[derive(Debug)]
pub struct ControlTransport<S> {
    stream: S,
    /// Accumulated inbound bytes; a partial frame stays buffered here so a
    /// dropped `read_message` future never loses stream position. Mirrors
    /// `C4NetIOTCP::Peer::IBuf` (src/C4NetIO.cpp:1415): incomplete frames are
    /// retained until more bytes arrive.
    read_buf: Vec<u8>,
}

impl<S> ControlTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            read_buf: Vec::new(),
        }
    }

    /// Returns the underlying stream, discarding any buffered partial frame.
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Reads the next complete frame.
    ///
    /// Cancel-safe: this future may be dropped mid-frame (e.g. by
    /// `tokio::select!`) without corrupting the stream — partial frames are
    /// kept in the transport's buffer and completed by the next call.
    pub async fn read_message(&mut self) -> Result<ControlMessage, TransportError> {
        loop {
            if let Some(message) = self.extract_frame()? {
                return Ok(message);
            }
            let mut chunk = [0u8; 4096];
            let read = self.stream.read(&mut chunk).await?;
            if read == 0 {
                return Err(TransportError::Io(io::ErrorKind::UnexpectedEof.into()));
            }
            self.read_buf.extend_from_slice(&chunk[..read]);
        }
    }

    /// Extracts one complete frame from the accumulated buffer, mirroring
    /// `C4NetIOTCP::UnpackPacket` (src/C4NetIO.cpp:1304). Returns `Ok(None)`
    /// while the frame is still incomplete.
    fn extract_frame(&mut self) -> Result<Option<ControlMessage>, TransportError> {
        if self.read_buf.len() < FRAME_HEADER_LEN {
            return Ok(None);
        }
        if self.read_buf[0] != TCP_FRAME_PREFIX {
            return Err(TransportError::Malformed("invalid TCP frame prefix"));
        }
        let size = u32::from_le_bytes([
            self.read_buf[1],
            self.read_buf[2],
            self.read_buf[3],
            self.read_buf[4],
        ]) as usize;
        if size > MAX_PACKET_SIZE {
            return Err(TransportError::Malformed("packet exceeds allowed size"));
        }
        if self.read_buf.len() < FRAME_HEADER_LEN + size {
            return Ok(None);
        }
        let message = parse_body(&self.read_buf[FRAME_HEADER_LEN..FRAME_HEADER_LEN + size])?;
        self.read_buf.drain(..FRAME_HEADER_LEN + size);
        Ok(Some(message))
    }

    /// Sends one message as a single contiguous frame, mirroring
    /// `C4NetIOTCP::PackPacket` (src/C4NetIO.cpp:1286) which writes prefix,
    /// size and payload into one output buffer.
    pub async fn send_message(&mut self, message: ControlMessage) -> Result<(), TransportError> {
        let mut frame = vec![TCP_FRAME_PREFIX, 0, 0, 0, 0];
        match message {
            ControlMessage::Status(status) => {
                frame.push(PID_STATUS);
                encode_network_status(status, &mut frame);
            }
            ControlMessage::StatusAck(status) => {
                frame.push(PID_STATUS_ACK);
                encode_network_status(status, &mut frame);
            }
            ControlMessage::LobbyCountdown(countdown) => {
                frame.push(PID_LOBBY_COUNTDOWN);
                frame.extend_from_slice(&countdown.seconds.to_ne_bytes());
            }
            ControlMessage::ReadyCheck(ready_check) => {
                frame.push(PID_READY_CHECK);
                frame.extend_from_slice(&ready_check.client_id.to_ne_bytes());
                frame.extend_from_slice(&ready_check.data.wire_value().to_ne_bytes());
            }
            ControlMessage::ActivationRequest { tick } => {
                frame.push(PID_CLIENT_ACT_REQ);
                encode_packed_i32(tick, &mut frame);
            }
            ControlMessage::PlayerInfoUpdate(request) => {
                frame.push(PID_PLAYER_INFO_UPDATE_REQ);
                frame.extend(
                    encode_player_info_update_payload(&request)
                        .map_err(TransportError::PlayerInfoUpdateEncode)?,
                );
            }
            ControlMessage::Control(packet) => {
                frame.push(PID_CONTROL);
                encode_varint(packet.client_id(), &mut frame);
                encode_varint(packet.tick(), &mut frame);
                frame.extend_from_slice(packet.payload());
            }
            ControlMessage::Request { from_tick } => {
                frame.push(PID_CONTROL_REQ);
                encode_varint(from_tick, &mut frame);
            }
            ControlMessage::Packet { delivery, data } => {
                frame.push(PID_CONTROL_PKT);
                frame.push(u8::from(delivery));
                frame.extend_from_slice(&data);
            }
            ControlMessage::ExecSync { control_tick } => {
                frame.push(PID_EXEC_SYNC_CTRL);
                let control_tick = i32::try_from(control_tick)
                    .map_err(|_| TransportError::ControlTickOutOfRange(control_tick))?;
                frame.extend_from_slice(&control_tick.to_ne_bytes());
            }
        }

        let size = (frame.len() - FRAME_HEADER_LEN) as u32;
        frame[1..FRAME_HEADER_LEN].copy_from_slice(&size.to_le_bytes());
        self.stream.write_all(&frame).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

fn parse_body(body: &[u8]) -> Result<ControlMessage, TransportError> {
    if body.is_empty() {
        return Err(TransportError::Malformed("missing packet payload"));
    }
    match body[0] {
        PID_STATUS => parse_network_status(&body[1..]).map(ControlMessage::Status),
        PID_STATUS_ACK => parse_network_status(&body[1..]).map(ControlMessage::StatusAck),
        PID_LOBBY_COUNTDOWN => {
            parse_lobby_countdown(&body[1..]).map(ControlMessage::LobbyCountdown)
        }
        PID_READY_CHECK => parse_ready_check(&body[1..]).map(ControlMessage::ReadyCheck),
        PID_CLIENT_ACT_REQ => parse_activation_request(&body[1..]),
        PID_PLAYER_INFO_UPDATE_REQ => parse_player_info_update(&body[1..]),
        PID_CONTROL => parse_control(&body[1..]),
        PID_CONTROL_REQ => parse_request(&body[1..]),
        PID_CONTROL_PKT => parse_packet(&body[1..]),
        PID_EXEC_SYNC_CTRL => parse_exec_sync(&body[1..]),
        other => Err(TransportError::UnsupportedPacket(other)),
    }
}

fn parse_network_status(data: &[u8]) -> Result<NetworkStatus, TransportError> {
    let (&state, fields) = data
        .split_first()
        .ok_or(TransportError::Malformed("status packet is missing state"))?;
    let (control_mode, mode_len) = decode_packed_i32(fields)?;
    let (target_tick, tick_len) = decode_packed_i32(&fields[mode_len..])?;
    if mode_len + tick_len != fields.len() {
        return Err(TransportError::Malformed(
            "unexpected trailing bytes in status packet",
        ));
    }
    Ok(NetworkStatus {
        state,
        control_mode,
        target_tick,
    })
}

fn parse_lobby_countdown(data: &[u8]) -> Result<LobbyCountdown, TransportError> {
    let bytes: [u8; size_of::<i32>()] = data.try_into().map_err(|_| {
        TransportError::Malformed("lobby countdown packet must contain one raw int32")
    })?;
    Ok(LobbyCountdown::new(i32::from_ne_bytes(bytes)))
}

fn parse_ready_check(data: &[u8]) -> Result<ReadyCheck, TransportError> {
    if data.len() != size_of::<i32>() * 2 {
        return Err(TransportError::Malformed(
            "ready-check packet must contain two raw int32 values",
        ));
    }
    let client_id = i32::from_ne_bytes(
        data[..size_of::<i32>()]
            .try_into()
            .expect("ready-check client slice has exact int32 length"),
    );
    let wire_data = i32::from_ne_bytes(
        data[size_of::<i32>()..]
            .try_into()
            .expect("ready-check data slice has exact int32 length"),
    );
    Ok(ReadyCheck {
        client_id,
        data: ReadyCheckData::from_wire(wire_data),
    })
}

fn parse_player_info_update(data: &[u8]) -> Result<ControlMessage, TransportError> {
    decode_player_info_update_payload(data)
        .map(ControlMessage::PlayerInfoUpdate)
        .map_err(TransportError::PlayerInfoUpdateDecode)
}

fn parse_activation_request(data: &[u8]) -> Result<ControlMessage, TransportError> {
    let (tick, consumed) = decode_packed_i32(data)?;
    if consumed != data.len() {
        return Err(TransportError::Malformed(
            "unexpected trailing bytes in activation request",
        ));
    }
    Ok(ControlMessage::ActivationRequest { tick })
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
    let bytes: [u8; size_of::<i32>()] = data.try_into().map_err(|_| {
        TransportError::Malformed("execute-sync packet must contain one raw int32")
    })?;
    let tick = i32::from_ne_bytes(bytes);
    if tick < 0 {
        return Err(TransportError::NegativeControlTick(tick));
    }
    Ok(ControlMessage::ExecSync {
        control_tick: tick as Tick,
    })
}

fn encode_network_status(status: NetworkStatus, out: &mut Vec<u8>) {
    out.push(status.state);
    encode_packed_i32(status.control_mode, out);
    encode_packed_i32(status.target_tick, out);
}

fn encode_packed_i32(mut value: i32, out: &mut Vec<u8>) {
    loop {
        let chunk = (value << 25) >> 25;
        if chunk == value {
            out.push(chunk as u8);
            break;
        }
        out.push((chunk ^ 0x80) as u8);
        value >>= 7;
    }
}

fn decode_packed_i32(data: &[u8]) -> Result<(i32, usize), TransportError> {
    let first = *data.first().ok_or(TransportError::UnexpectedEof)?;
    let mut current = first;
    let mut signed = (i32::from(current) << 25) >> 25;
    let mut value = signed;
    let mut bytes_read = 1usize;
    let mut shift = 7u32;

    while signed as u8 != current {
        if bytes_read >= 5 {
            return Err(TransportError::VarintOverflow);
        }
        current = *data
            .get(bytes_read)
            .ok_or(TransportError::UnexpectedEof)?;
        signed = (i32::from(current) << 25) >> 25;
        let lower_mask = (1i64 << shift) - 1;
        value = (((i64::from(signed)) << shift) | (i64::from(value) & lower_mask)) as i32;
        bytes_read += 1;
        shift += 7;
    }

    Ok((value, bytes_read))
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
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

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
        let (client, mut server) = duplex(64);
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
        let (client, mut server) = duplex(64);
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
        let (client, mut server) = duplex(32);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        match transport.read_message().await.unwrap() {
            ControlMessage::Request { from_tick } => assert_eq!(from_tick, 150),
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_player_info_update_request() {
        // C4PacketBase::pack prefixes PID_PlayerInfoUpdReq (0x16) before the
        // C4ClientPlayerInfos body (src/C4Packet2.cpp:140-143;
        // src/C4PlayerInfo.cpp:601-630,1800-1803). These bytes come from the
        // live C++ player-info-update codec oracle fixture.
        let payload = [
            0x16, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, b'P', 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x33, 0x22, 0x11, 0x00, 0x33,
            0x22, 0x11, 0x00, 0x00, 0x00, 0x00, b'N', b'O', b'N', b'E', 0x00, 0x00, 0x00,
            0x00, 0x00, 0xff, 0x00, 0x00, 0x00,
        ];
        let frame = expect_frame(&payload);
        let (client, mut server) = duplex(128);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        let ControlMessage::PlayerInfoUpdate(request) = transport.read_message().await.unwrap()
        else {
            panic!("expected PlayerInfo update request");
        };
        assert_eq!((request.client_id, request.flags), (3, 1));
        let [player] = request.players.as_slice() else {
            panic!("expected one player info");
        };
        assert_eq!((player.name.as_bytes(), player.id), (b"P".as_slice(), 0));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_cpp_activation_request_signed_tick() {
        // C4PacketActivateReq is PID_ClientActReq followed by one signed
        // packed int32 tick (src/C4PacketBase.h:104-114;
        // src/C4Network2IO.cpp:1780-1785). This is C++ tick 195995.
        let frame = expect_frame(&[0x13, 0x9b, 0x7b, 0x0b]);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::ActivationRequest { tick: 195_995 }
        );

        let negative_frame = expect_frame(&[0x13, 0xff]);
        let (negative_client, mut negative_server) = duplex(16);
        negative_server.write_all(&negative_frame).await.unwrap();
        let mut negative_transport = ControlTransport::new(negative_client);
        assert_eq!(
            negative_transport.read_message().await.unwrap(),
            ControlMessage::ActivationRequest { tick: -1 }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_activation_request_trailing_bytes() {
        let frame = expect_frame(&[0x13, 0x25, 0x00]);
        let (client, mut server) = duplex(16);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert!(matches!(
            transport.read_message().await,
            Err(TransportError::Malformed(
                "unexpected trailing bytes in activation request"
            ))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sends_cpp_activation_request_signed_tick() {
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);
        transport
            .send_message(ControlMessage::ActivationRequest { tick: 195_995 })
            .await
            .unwrap();
        drop(transport);

        let mut buf = Vec::new();
        server.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, expect_frame(&[0x13, 0x9b, 0x7b, 0x0b]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_cpp_go_status_with_signed_packed_fields() {
        // C4Network2Status is raw state followed by signed-packed CtrlMode and
        // TargetTick (src/C4Network2.cpp:103-123). These bytes are the C++
        // encoding of Go, default mode -1, target tick 195995.
        let payload = [PID_STATUS, NETWORK_STATE_GO, 0xff, 0x9b, 0x7b, 0x0b];
        let frame = expect_frame(&payload);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::Status(NetworkStatus {
                state: NETWORK_STATE_GO,
                control_mode: -1,
                target_tick: 195_995,
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_exec_sync() {
        // C4PacketExecSyncCtrl uses raw native int32, not StdCompiler's packed
        // integer adapter (src/C4GameControlNetwork.h:284-295).
        let mut payload = vec![PID_EXEC_SYNC_CTRL];
        payload.extend_from_slice(&195_995i32.to_ne_bytes());
        let frame = expect_frame(&payload);
        let (client, mut server) = duplex(64);
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
    async fn sends_cpp_status_and_ack_with_signed_packed_fields() {
        // PID_Status/PID_StatusAck share the C4Network2Status body
        // (src/C4PacketBase.h:104-113; src/C4Network2.cpp:103-123).
        let (client, mut server) = duplex(128);
        let mut transport = ControlTransport::new(client);
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 195_995,
        };
        transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        transport
            .send_message(ControlMessage::StatusAck(status))
            .await
            .unwrap();
        drop(transport);

        let mut buf = Vec::new();
        server.read_to_end(&mut buf).await.unwrap();
        let mut expected = expect_frame(&[PID_STATUS, 0x04, 0x01, 0x9b, 0x7b, 0x0b]);
        expected.extend(expect_frame(&[
            PID_STATUS_ACK,
            0x04,
            0x01,
            0x9b,
            0x7b,
            0x0b,
        ]));
        assert_eq!(buf, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lobby_packets_match_cpp_raw_signed_int32_layout() {
        // Neither C4PacketCountdown nor C4PacketReadyCheck uses
        // mkIntPackAdapt. StdCompilerBinWrite therefore copies each int32_t
        // verbatim after the PID (src/C4GameLobby.cpp:45-48;
        // src/C4Network2IO.cpp:1694-1700).
        let (client, mut server) = duplex(128);
        let mut transport = ControlTransport::new(client);
        transport
            .send_message(ControlMessage::LobbyCountdown(LobbyCountdown::abort()))
            .await
            .unwrap();
        transport
            .send_message(ControlMessage::ReadyCheck(ReadyCheck::request(0)))
            .await
            .unwrap();
        transport
            .send_message(ControlMessage::ReadyCheck(ReadyCheck::reply(7, true)))
            .await
            .unwrap();
        drop(transport);

        let mut actual = Vec::new();
        server.read_to_end(&mut actual).await.unwrap();
        let mut expected_countdown = vec![PID_LOBBY_COUNTDOWN];
        expected_countdown.extend_from_slice(&(-1i32).to_ne_bytes());
        let mut expected_request = vec![PID_READY_CHECK];
        expected_request.extend_from_slice(&0i32.to_ne_bytes());
        expected_request.extend_from_slice(&(-1i32).to_ne_bytes());
        let mut expected_reply = vec![PID_READY_CHECK];
        expected_reply.extend_from_slice(&7i32.to_ne_bytes());
        expected_reply.extend_from_slice(&1i32.to_ne_bytes());
        let mut expected = expect_frame(&expected_countdown);
        expected.extend(expect_frame(&expected_request));
        expected.extend(expect_frame(&expected_reply));
        assert_eq!(actual, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_lobby_packets_and_preserves_unknown_ready_value() {
        let mut countdown = vec![PID_LOBBY_COUNTDOWN];
        countdown.extend_from_slice(&195_995i32.to_ne_bytes());
        let mut unknown_ready = vec![PID_READY_CHECK];
        unknown_ready.extend_from_slice(&7i32.to_ne_bytes());
        unknown_ready.extend_from_slice(&23i32.to_ne_bytes());
        let mut frames = expect_frame(&countdown);
        frames.extend(expect_frame(&unknown_ready));
        let (client, mut server) = duplex(128);
        server.write_all(&frames).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::LobbyCountdown(LobbyCountdown::new(195_995))
        );
        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::ReadyCheck(ReadyCheck {
                client_id: 7,
                data: ReadyCheckData::Unknown(23),
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_malformed_lobby_packet_lengths() {
        let malformed = [
            expect_frame(&[PID_LOBBY_COUNTDOWN, 0, 0, 0]),
            expect_frame(&[PID_LOBBY_COUNTDOWN, 0, 0, 0, 0, 0]),
            expect_frame(&[PID_READY_CHECK, 0, 0, 0, 0, 0, 0, 0]),
            expect_frame(&[PID_READY_CHECK, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        ];

        for frame in malformed {
            let (client, mut server) = duplex(32);
            server.write_all(&frame).await.unwrap();
            let mut transport = ControlTransport::new(client);
            assert!(matches!(
                transport.read_message().await,
                Err(TransportError::Malformed(_))
            ));
        }
    }

    #[test]
    fn lobby_countdown_timer_cadence_matches_cpp() {
        for seconds in [601, 599, 59, 11] {
            assert!(!LobbyCountdown::new(seconds).timer_tick_is_broadcast());
        }
        for seconds in [660, 600, 590, 60, 10, 9, 1, 0] {
            assert!(LobbyCountdown::new(seconds).timer_tick_is_broadcast());
        }
        assert!(!LobbyCountdown::abort().timer_tick_is_broadcast());
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
    async fn send_exec_sync_matches_cpp_raw_int32() {
        // C4PacketExecSyncCtrl::CompileFunc serializes ControlTick through
        // mkIntAdapt (src/C4GameControlNetwork.h:284-295).
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);
        transport
            .send_message(ControlMessage::ExecSync {
                control_tick: 195_995,
            })
            .await
            .unwrap();
        drop(transport);

        let mut buf = Vec::new();
        server.read_to_end(&mut buf).await.unwrap();
        let mut payload = vec![PID_EXEC_SYNC_CTRL];
        payload.extend_from_slice(&195_995i32.to_ne_bytes());
        assert_eq!(buf, expect_frame(&payload));
    }

    // `read_message` is polled inside `tokio::select!` loops (session.rs), so a
    // partially received frame must survive the read future being dropped.
    // Mirrors C4NetIOTCP::Peer::OnRecv / UnpackPacket (src/C4NetIO.cpp:1415,
    // :1304): incomplete frames stay in the peer's IBuf until more bytes arrive.
    #[tokio::test(flavor = "current_thread")]
    async fn read_message_survives_cancellation_mid_frame() {
        let frame = expect_frame(&[PID_CONTROL, 0x0C, 0x22, 0xAB]);
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);

        // Deliver only part of the frame header, then poll `read_message` and
        // drop it mid-frame, exactly as `tokio::select!` does when another
        // branch wins.
        server.write_all(&frame[..3]).await.unwrap();
        tokio::select! {
            biased;
            result = transport.read_message() => {
                panic!("read completed on a partial frame: {result:?}")
            }
            _ = tokio::task::yield_now() => {}
        }

        // The rest of the frame arrives; the retried read must still parse it.
        server.write_all(&frame[3..]).await.unwrap();
        match transport.read_message().await.unwrap() {
            ControlMessage::Control(packet) => {
                assert_eq!(packet.client_id(), 12);
                assert_eq!(packet.tick(), 34);
                assert_eq!(packet.payload(), &[0xAB]);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_invalid_prefix() {
        let mut frame = expect_frame(&[PID_CONTROL, 0x00]);
        frame[0] = 0xAA;
        let (client, mut server) = duplex(16);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        let err = transport.read_message().await.unwrap_err();
        match err {
            TransportError::Malformed(_) => {}
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
