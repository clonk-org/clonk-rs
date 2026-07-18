use crate::address_packet::{
    decode_address_packet_payload, encode_address_packet_payload, AddressPacket,
    AddressPacketDecodeError, PID_ADDR,
};
use crate::forward_packet::{
    decode_forward_packet_payload, encode_forward_packet_payload, ForwardPacket,
    ForwardPacketCodecError, PID_FORWARD, PID_FORWARD_REQUEST,
};
use crate::legacy::{
    decode_join_data_envelope, decode_player_info_update_payload, encode_join_data_envelope,
    encode_player_info_update_payload, JoinDataEnvelope, LegacyControlError, LegacyEncodeError,
};
use crate::league_round_results_packet::{
    decode_league_round_results_payload, encode_league_round_results_payload,
    LeagueRoundResultsDecodeError, LeagueRoundResultsEncodeError, LeagueRoundResultsPacket,
    PID_LEAGUE_ROUND_RESULTS,
};
use crate::name_validation::validate_name_no_empty;
use crate::resource_packet::{
    decode_resource_packet, encode_resource_packet, ResourcePacket, ResourcePacketCodecError,
    PID_NET_RES_DATA, PID_NET_RES_DERIVE, PID_NET_RES_DISCOVER, PID_NET_RES_REQUEST,
    PID_NET_RES_STATUS,
};
use crate::{ClientId, ControlPacket, Tick};
use lc_engine::{ClientCoreControlData, LegacyCString, PlayerInfoUpdateRequest};
use std::convert::TryFrom;
use std::io;
use std::mem::size_of;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const TCP_FRAME_PREFIX: u8 = 0xFF;
const MAX_PACKET_SIZE: usize = 2 * 1024 * 1024; // 2 MiB cap to guard against bogus frames.
const PID_PING: u8 = 0x00;
const PID_PONG: u8 = 0x01;
const PID_CONN: u8 = 0x02;
const PID_CONN_RE: u8 = 0x03;
const PID_POST_MORTEM: u8 = 0x06;
const PID_STATUS: u8 = 0x10;
const PID_STATUS_ACK: u8 = 0x11;
const PID_CLIENT_ACT_REQ: u8 = 0x13;
const PID_TCP_SIM_OPEN: u8 = 0x14;
const PID_JOIN_DATA: u8 = 0x15;
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

/// Exact payload of `C4PacketConn` (`src/C4Network2IO.cpp:1611-1626`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub core: ClientCoreControlData,
    pub build: i32,
    pub password: LegacyCString,
    pub connection_id: u32,
}

/// Exact payload of `C4PacketConnRe` (`src/C4Network2IO.cpp:1630-1642`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReply {
    pub ok: bool,
    pub message: LegacyCString,
    pub wrong_password: bool,
}

/// Exact `C4PacketPing` body shared by `PID_Ping` and `PID_Pong`
/// (`src/C4Network2IO.cpp:1702-1718`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingPacket {
    pub sent_at: u32,
    pub packet_counter: u32,
}

/// Exact body of `PID_LobbyCountdown` (`src/C4GameLobby.h:47-65`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyCountdownPacket {
    countdown: i32,
}

impl LobbyCountdownPacket {
    pub const ABORT: i32 = -1;

    pub const fn new(countdown: i32) -> Self {
        Self { countdown }
    }

    pub const fn countdown(self) -> i32 {
        self.countdown
    }

    pub const fn is_abort(self) -> bool {
        self.countdown == Self::ABORT
    }
}

/// Exact underlying values of `C4PacketReadyCheck::Data`
/// (`src/C4Network2.h:480-502`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyCheckData {
    Request,
    NotReady,
    Ready,
    /// C++ retains arbitrary underlying `int32_t` values and treats them as
    /// not-ready (`src/C4Network2.h:494-497`).
    Other(i32),
}

impl ReadyCheckData {
    pub const fn vote_requested(self) -> bool {
        matches!(self, Self::Request)
    }

    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

impl From<i32> for ReadyCheckData {
    fn from(value: i32) -> Self {
        match value {
            -1 => Self::Request,
            0 => Self::NotReady,
            1 => Self::Ready,
            other => Self::Other(other),
        }
    }
}

impl From<ReadyCheckData> for i32 {
    fn from(value: ReadyCheckData) -> Self {
        match value {
            ReadyCheckData::Request => -1,
            ReadyCheckData::NotReady => 0,
            ReadyCheckData::Ready => 1,
            ReadyCheckData::Other(other) => other,
        }
    }
}

/// Exact body of `PID_ReadyCheck` (`src/C4Network2.h:480-502`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadyCheckPacket {
    pub client_id: i32,
    pub data: ReadyCheckData,
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
    #[error("control packet contained negative control tick {0}")]
    NegativeControlTick(i32),
    #[error("control packet contained invalid negative client id {0}")]
    NegativeControlClientId(i32),
    #[error("control tick {0} exceeds C++ int32 range")]
    ControlTickOutOfRange(Tick),
    #[error("control client id {0} exceeds C++ int32 range")]
    ControlClientIdOutOfRange(ClientId),
    #[error("post-mortem packet count {0} exceeds C++ uint32 range")]
    PostMortemPacketCountOutOfRange(usize),
    #[error("post-mortem nested packet length {0} exceeds C++ uint32 range")]
    PostMortemPacketLengthOutOfRange(usize),
    #[error("invalid player-info update request: {0}")]
    PlayerInfoUpdateDecode(#[source] LegacyControlError),
    #[error("failed to encode player-info update request: {0}")]
    PlayerInfoUpdateEncode(#[source] LegacyEncodeError),
    #[error("invalid join-data packet: {0}")]
    JoinDataDecode(#[source] LegacyControlError),
    #[error("failed to encode join-data packet: {0}")]
    JoinDataEncode(#[source] LegacyEncodeError),
    #[error("invalid league round-results packet: {0}")]
    LeagueRoundResultsDecode(#[source] LeagueRoundResultsDecodeError),
    #[error("failed to encode league round-results packet: {0}")]
    LeagueRoundResultsEncode(#[source] LeagueRoundResultsEncodeError),
    #[error("invalid client-address packet: {0}")]
    AddressDecode(#[source] AddressPacketDecodeError),
    #[error("invalid resource packet: {0}")]
    ResourceDecode(#[source] ResourcePacketCodecError),
    #[error("failed to encode resource packet: {0}")]
    ResourceEncode(#[source] ResourcePacketCodecError),
    #[error("invalid forward packet: {0}")]
    ForwardDecode(#[source] ForwardPacketCodecError),
    #[error("failed to encode forward packet: {0}")]
    ForwardEncode(#[source] ForwardPacketCodecError),
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
    Ping(PingPacket),
    Pong(PingPacket),
    ConnectionRequest(ConnectionRequest),
    ConnectionReply(ConnectionReply),
    ForwardRequest(ForwardPacket),
    Forward(ForwardPacket),
    PostMortem(crate::PostMortemPacket),
    JoinData(Box<JoinDataEnvelope>),
    LeagueRoundResults(LeagueRoundResultsPacket),
    Address(AddressPacket),
    Resource(ResourcePacket),
    Status(NetworkStatus),
    StatusAck(NetworkStatus),
    LobbyCountdown(LobbyCountdownPacket),
    ReadyCheck(ReadyCheckPacket),
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

#[derive(Debug)]
pub(crate) enum InboundPacket {
    Message(ControlMessage),
    Ignored(u8),
    Empty,
    Invalid {
        packet_type: u8,
        error: TransportError,
    },
}

/// Length of the frame header: prefix byte plus native-endian u32 size.
const FRAME_HEADER_LEN: usize = 5;

/// Tokio-powered transport that understands LegacyClonk TCP framing and control packets.
#[derive(Debug)]
pub struct ControlTransport<S> {
    stream: S,
    outbound_packet_log: crate::RecoverablePacketLog,
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
            outbound_packet_log: crate::RecoverablePacketLog::default(),
            read_buf: Vec::new(),
        }
    }

    /// Returns the underlying stream, discarding any buffered partial frame.
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Builds the one C++ recovery envelope permitted for this connection.
    pub fn create_post_mortem(
        &mut self,
        remote_connection_id: u32,
    ) -> Option<crate::PostMortemPacket> {
        self.outbound_packet_log
            .create_post_mortem(remote_connection_id)
    }

    /// Reads the next supported logical message, transparently consuming any
    /// known C++ packet types whose handlers have not been ported yet.
    ///
    /// Cancel-safe: this future may be dropped mid-frame (e.g. by
    /// `tokio::select!`) without corrupting the stream — partial frames are
    /// kept in the transport's buffer and completed by the next call.
    pub async fn read_message(&mut self) -> Result<ControlMessage, TransportError> {
        loop {
            match self.read_packet().await? {
                InboundPacket::Message(message) => return Ok(message),
                InboundPacket::Ignored(_) | InboundPacket::Empty => {}
                InboundPacket::Invalid { error, .. } => return Err(error),
            }
        }
    }

    /// Reads one framed packet, including known C++ packet types whose
    /// handlers have not been ported yet. Session owners use this to preserve
    /// C++ packet-counter accounting before ignoring those packets.
    pub(crate) async fn read_packet(&mut self) -> Result<InboundPacket, TransportError> {
        loop {
            if let Some(packet) = self.extract_frame()? {
                if let InboundPacket::Message(ControlMessage::Ping(ping)) = &packet {
                    self.outbound_packet_log
                        .acknowledge_received(ping.packet_counter);
                }
                return Ok(packet);
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
    fn extract_frame(&mut self) -> Result<Option<InboundPacket>, TransportError> {
        if self.read_buf.len() < FRAME_HEADER_LEN {
            return Ok(None);
        }
        if self.read_buf[0] != TCP_FRAME_PREFIX {
            return Err(TransportError::Malformed("invalid TCP frame prefix"));
        }
        let size = u32::from_ne_bytes([
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
        let packet = if size == 0 {
            InboundPacket::Empty
        } else {
            let body = &self.read_buf[FRAME_HEADER_LEN..FRAME_HEADER_LEN + size];
            let packet_type = body[0];
            match parse_complete_packet(body) {
                Ok(Some(message)) => InboundPacket::Message(message),
                Ok(None) => InboundPacket::Ignored(packet_type),
                Err(error) => InboundPacket::Invalid { packet_type, error },
            }
        };
        self.read_buf.drain(..FRAME_HEADER_LEN + size);
        Ok(Some(packet))
    }

    fn encode_message_frame(message: ControlMessage) -> Result<Vec<u8>, TransportError> {
        let mut frame = vec![TCP_FRAME_PREFIX, 0, 0, 0, 0];
        match message {
            ControlMessage::Ping(packet) => {
                frame.push(PID_PING);
                encode_ping(packet, &mut frame);
            }
            ControlMessage::Pong(packet) => {
                frame.push(PID_PONG);
                encode_ping(packet, &mut frame);
            }
            ControlMessage::ConnectionRequest(request) => {
                frame.push(PID_CONN);
                frame.extend(encode_connection_request_payload(&request)?);
            }
            ControlMessage::ConnectionReply(reply) => {
                frame.push(PID_CONN_RE);
                frame.extend(encode_connection_reply_payload(&reply)?);
            }
            ControlMessage::ForwardRequest(packet) => {
                frame.push(PID_FORWARD_REQUEST);
                frame.extend(
                    encode_forward_packet_payload(&packet)
                        .map_err(TransportError::ForwardEncode)?,
                );
            }
            ControlMessage::Forward(packet) => {
                frame.push(PID_FORWARD);
                frame.extend(
                    encode_forward_packet_payload(&packet)
                        .map_err(TransportError::ForwardEncode)?,
                );
            }
            ControlMessage::PostMortem(packet) => {
                frame.push(PID_POST_MORTEM);
                frame.extend_from_slice(&packet.connection_id.to_ne_bytes());
                frame.extend_from_slice(&packet.packet_counter.to_ne_bytes());
                let packet_count = u32::try_from(packet.packets.len()).map_err(|_| {
                    TransportError::PostMortemPacketCountOutOfRange(packet.packets.len())
                })?;
                frame.extend_from_slice(&packet_count.to_ne_bytes());
                for nested in packet.packets {
                    let length = u32::try_from(nested.len()).map_err(|_| {
                        TransportError::PostMortemPacketLengthOutOfRange(nested.len())
                    })?;
                    encode_varint(length, &mut frame);
                    frame.extend_from_slice(&nested);
                }
            }
            ControlMessage::JoinData(envelope) => {
                frame.push(PID_JOIN_DATA);
                frame.extend(
                    encode_join_data_envelope(&envelope).map_err(TransportError::JoinDataEncode)?,
                );
            }
            ControlMessage::LeagueRoundResults(packet) => {
                frame.push(PID_LEAGUE_ROUND_RESULTS);
                frame.extend(
                    encode_league_round_results_payload(&packet)
                        .map_err(TransportError::LeagueRoundResultsEncode)?,
                );
            }
            ControlMessage::Address(packet) => {
                frame.push(PID_ADDR);
                frame.extend(encode_address_packet_payload(&packet));
            }
            ControlMessage::Resource(packet) => {
                frame.extend(
                    encode_resource_packet(&packet).map_err(TransportError::ResourceEncode)?,
                );
            }
            ControlMessage::Status(status) => {
                frame.push(PID_STATUS);
                encode_network_status(status, &mut frame);
            }
            ControlMessage::StatusAck(status) => {
                frame.push(PID_STATUS_ACK);
                encode_network_status(status, &mut frame);
            }
            ControlMessage::LobbyCountdown(packet) => {
                frame.push(PID_LOBBY_COUNTDOWN);
                frame.extend_from_slice(&packet.countdown().to_ne_bytes());
            }
            ControlMessage::ReadyCheck(packet) => {
                frame.extend(encode_complete_ready_check_packet(packet));
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
                frame.extend(encode_complete_control_packet(&packet)?);
            }
            ControlMessage::Request { from_tick } => {
                frame.push(PID_CONTROL_REQ);
                let from_tick = i32::try_from(from_tick)
                    .map_err(|_| TransportError::ControlTickOutOfRange(from_tick))?;
                encode_packed_i32(from_tick, &mut frame);
            }
            ControlMessage::Packet { delivery, data } => {
                frame.extend(encode_complete_control_delivery_packet(delivery, &data));
            }
            ControlMessage::ExecSync { control_tick } => {
                frame.push(PID_EXEC_SYNC_CTRL);
                let control_tick = i32::try_from(control_tick)
                    .map_err(|_| TransportError::ControlTickOutOfRange(control_tick))?;
                frame.extend_from_slice(&control_tick.to_ne_bytes());
            }
        }

        let size = (frame.len() - FRAME_HEADER_LEN) as u32;
        frame[1..FRAME_HEADER_LEN].copy_from_slice(&size.to_ne_bytes());
        Ok(frame)
    }

    /// Retains an already-accepted logical send for rerouting after this
    /// transport fails before its per-route queue can write it.
    pub(crate) fn retain_unsent_message(
        &mut self,
        message: ControlMessage,
    ) -> Result<(), TransportError> {
        let frame = Self::encode_message_frame(message)?;
        self.outbound_packet_log
            .record_outbound(frame[FRAME_HEADER_LEN..].to_vec());
        Ok(())
    }

    pub(crate) fn retain_unsent_complete_packet(
        &mut self,
        packet: Vec<u8>,
    ) -> Result<(), TransportError> {
        if packet.len() > MAX_PACKET_SIZE {
            return Err(TransportError::Malformed("packet exceeds allowed size"));
        }
        self.outbound_packet_log.record_outbound(packet);
        Ok(())
    }

    /// Sends one message as a single contiguous frame, mirroring
    /// `C4NetIOTCP::PackPacket` (src/C4NetIO.cpp:1286) which writes prefix,
    /// size and payload into one output buffer.
    pub async fn send_message(&mut self, message: ControlMessage) -> Result<(), TransportError> {
        let frame = Self::encode_message_frame(message)?;
        self.outbound_packet_log
            .record_outbound(frame[FRAME_HEADER_LEN..].to_vec());
        self.stream.write_all(&frame).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Sends one already-encoded complete packet body without normalizing its
    /// payload. C++ uses this path when a host directly relays a `PID_FwdReq`
    /// nested packet to at most two clients.
    pub(crate) async fn send_complete_packet_bytes(
        &mut self,
        packet: &[u8],
    ) -> Result<(), TransportError> {
        if packet.len() > MAX_PACKET_SIZE {
            return Err(TransportError::Malformed("packet exceeds allowed size"));
        }
        let size = u32::try_from(packet.len())
            .map_err(|_| TransportError::Malformed("packet exceeds allowed size"))?;
        let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + packet.len());
        frame.push(TCP_FRAME_PREFIX);
        frame.extend_from_slice(&size.to_ne_bytes());
        frame.extend_from_slice(packet);
        self.outbound_packet_log.record_outbound(packet.to_vec());
        self.stream.write_all(&frame).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

pub(crate) fn parse_complete_packet(body: &[u8]) -> Result<Option<ControlMessage>, TransportError> {
    if body.is_empty() {
        return Err(TransportError::Malformed("missing packet payload"));
    }
    // Both IDs are present in C++'s typed packet table. C++ unpacks them
    // before checking whether a handler is enabled, so malformed bodies
    // remain fatal even when a valid packet would be ignored.
    match body[0] {
        PID_TCP_SIM_OPEN => {
            decode_address_packet_payload(&body[1..]).map_err(|_| {
                TransportError::Malformed("invalid TCP simultaneous-open packet")
            })?;
            return Ok(None);
        }
        PID_LEAGUE_ROUND_RESULTS => {
            return decode_league_round_results_payload(&body[1..])
                .map(ControlMessage::LeagueRoundResults)
                .map(Some)
                .map_err(TransportError::LeagueRoundResultsDecode);
        }
        _ => {}
    }
    parse_control_message(body).map(Some)
}

fn parse_control_message(body: &[u8]) -> Result<ControlMessage, TransportError> {
    match body[0] {
        PID_PING => parse_ping(&body[1..]).map(ControlMessage::Ping),
        PID_PONG => parse_ping(&body[1..]).map(ControlMessage::Pong),
        PID_CONN => {
            decode_connection_request_payload(&body[1..]).map(ControlMessage::ConnectionRequest)
        }
        PID_CONN_RE => {
            decode_connection_reply_payload(&body[1..]).map(ControlMessage::ConnectionReply)
        }
        PID_FORWARD_REQUEST => decode_forward_packet_payload(&body[1..])
            .map(ControlMessage::ForwardRequest)
            .map_err(TransportError::ForwardDecode),
        PID_FORWARD => decode_forward_packet_payload(&body[1..])
            .map(ControlMessage::Forward)
            .map_err(TransportError::ForwardDecode),
        PID_POST_MORTEM => parse_post_mortem(&body[1..]),
        PID_STATUS => parse_network_status(&body[1..]).map(ControlMessage::Status),
        PID_STATUS_ACK => parse_network_status(&body[1..]).map(ControlMessage::StatusAck),
        PID_LOBBY_COUNTDOWN => {
            parse_lobby_countdown(&body[1..]).map(ControlMessage::LobbyCountdown)
        }
        PID_READY_CHECK => parse_ready_check(&body[1..]).map(ControlMessage::ReadyCheck),
        PID_CLIENT_ACT_REQ => parse_activation_request(&body[1..]),
        PID_JOIN_DATA => decode_join_data_envelope(&body[1..])
            .map(Box::new)
            .map(ControlMessage::JoinData)
            .map_err(TransportError::JoinDataDecode),
        PID_ADDR => decode_address_packet_payload(&body[1..])
            .map(ControlMessage::Address)
            .map_err(TransportError::AddressDecode),
        PID_NET_RES_DISCOVER | PID_NET_RES_STATUS | PID_NET_RES_DERIVE | PID_NET_RES_REQUEST
        | PID_NET_RES_DATA => decode_resource_packet(body)
            .map(ControlMessage::Resource)
            .map_err(TransportError::ResourceDecode),
        PID_PLAYER_INFO_UPDATE_REQ => parse_player_info_update(&body[1..]),
        PID_CONTROL => parse_control(&body[1..]),
        PID_CONTROL_REQ => parse_request(&body[1..]),
        PID_CONTROL_PKT => parse_packet(&body[1..]),
        PID_EXEC_SYNC_CTRL => parse_exec_sync(&body[1..]),
        other => Err(TransportError::UnsupportedPacket(other)),
    }
}

pub(crate) fn encode_complete_control_packet(
    packet: &ControlPacket,
) -> Result<Vec<u8>, TransportError> {
    let client_id = if packet.client_id() == crate::BROADCAST_CLIENT_ID {
        -1
    } else {
        i32::try_from(packet.client_id())
            .map_err(|_| TransportError::ControlClientIdOutOfRange(packet.client_id()))?
    };
    let tick = i32::try_from(packet.tick())
        .map_err(|_| TransportError::ControlTickOutOfRange(packet.tick()))?;
    let mut body = vec![PID_CONTROL];
    encode_packed_i32(client_id, &mut body);
    encode_packed_i32(tick, &mut body);
    body.extend_from_slice(packet.payload());
    Ok(body)
}

pub(crate) fn encode_complete_ready_check_packet(packet: ReadyCheckPacket) -> Vec<u8> {
    let mut body = vec![PID_READY_CHECK];
    encode_ready_check(packet, &mut body);
    body
}

pub(crate) fn encode_complete_control_delivery_packet(
    delivery: ControlDelivery,
    data: &[u8],
) -> Vec<u8> {
    let mut body = vec![PID_CONTROL_PKT, u8::from(delivery)];
    body.extend_from_slice(data);
    body
}

fn parse_ping(data: &[u8]) -> Result<PingPacket, TransportError> {
    if data.len() < 8 {
        return Err(TransportError::Malformed("ping packet is truncated"));
    }
    Ok(PingPacket {
        sent_at: u32::from_ne_bytes(data[..4].try_into().expect("checked ping time length")),
        packet_counter: u32::from_ne_bytes(
            data[4..8].try_into().expect("checked ping counter length"),
        ),
    })
}

fn parse_post_mortem(data: &[u8]) -> Result<ControlMessage, TransportError> {
    let header = data.get(..12).ok_or(TransportError::Malformed(
        "post-mortem packet header is truncated",
    ))?;
    let connection_id = u32::from_ne_bytes(
        header[..4]
            .try_into()
            .expect("post-mortem connection ID length checked above"),
    );
    let packet_counter = u32::from_ne_bytes(
        header[4..8]
            .try_into()
            .expect("post-mortem packet counter length checked above"),
    );
    let packet_count = u32::from_ne_bytes(
        header[8..12]
            .try_into()
            .expect("post-mortem packet count length checked above"),
    );
    let mut offset = header.len();
    let mut packets = Vec::new();
    for _ in 0..packet_count {
        let (length, consumed) = decode_varint(data.get(offset..).ok_or(
            TransportError::Malformed("post-mortem packet list is truncated"),
        )?)?;
        offset = offset
            .checked_add(consumed)
            .ok_or(TransportError::Malformed("post-mortem packet length overflow"))?;
        let end = offset
            .checked_add(length as usize)
            .ok_or(TransportError::Malformed("post-mortem packet length overflow"))?;
        let packet = data
            .get(offset..end)
            .ok_or(TransportError::Malformed(
                "post-mortem packet data is truncated",
            ))?;
        packets.push(packet.to_vec());
        offset = end;
    }
    if offset != data.len() {
        return Err(TransportError::Malformed(
            "unexpected trailing bytes in post-mortem packet",
        ));
    }
    Ok(ControlMessage::PostMortem(crate::PostMortemPacket {
        connection_id,
        packet_counter,
        packets,
    }))
}

fn encode_ping(packet: PingPacket, output: &mut Vec<u8>) {
    output.extend_from_slice(&packet.sent_at.to_ne_bytes());
    output.extend_from_slice(&packet.packet_counter.to_ne_bytes());
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

fn parse_ready_check(data: &[u8]) -> Result<ReadyCheckPacket, TransportError> {
    if data.len() < 8 {
        return Err(TransportError::Malformed("ready-check packet is truncated"));
    }
    let client_id = i32::from_ne_bytes(
        data[..4]
            .try_into()
            .expect("ready-check client length checked above"),
    );
    let data = i32::from_ne_bytes(
        data[4..8]
            .try_into()
            .expect("ready-check data length checked above"),
    );
    Ok(ReadyCheckPacket {
        client_id,
        data: ReadyCheckData::from(data),
    })
}

fn parse_lobby_countdown(data: &[u8]) -> Result<LobbyCountdownPacket, TransportError> {
    let countdown = i32::from_ne_bytes(
        data.get(..size_of::<i32>())
            .ok_or(TransportError::Malformed(
                "lobby-countdown packet is truncated",
            ))?
            .try_into()
            .expect("lobby-countdown length checked above"),
    );
    Ok(LobbyCountdownPacket::new(countdown))
}

fn encode_ready_check(packet: ReadyCheckPacket, output: &mut Vec<u8>) {
    output.extend_from_slice(&packet.client_id.to_ne_bytes());
    output.extend_from_slice(&i32::from(packet.data).to_ne_bytes());
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
    let (client_id, consumed_a) = decode_packed_i32(data)?;
    let client_id = match client_id {
        -1 => crate::BROADCAST_CLIENT_ID,
        value if value < 0 => return Err(TransportError::NegativeControlClientId(value)),
        value => value as ClientId,
    };
    let (tick, consumed_b) = decode_packed_i32(&data[consumed_a..])?;
    if tick < 0 {
        return Err(TransportError::NegativeControlTick(tick));
    }
    let payload = data[consumed_a + consumed_b..].to_vec();
    Ok(ControlMessage::Control(
        ControlPacket::builder(client_id, tick as Tick)
            .timestamp_ms(0)
            .payload(payload),
    ))
}

fn parse_request(data: &[u8]) -> Result<ControlMessage, TransportError> {
    let (tick, _) = decode_packed_i32(data)?;
    if tick < 0 {
        return Err(TransportError::NegativeControlTick(tick));
    }
    // CompileFromBuf does not require the C++ packet reader to consume the
    // complete buffer, so PID_ControlReq tolerates bytes after CtrlTick.
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
    let bytes: [u8; size_of::<i32>()] = data
        .try_into()
        .map_err(|_| TransportError::Malformed("execute-sync packet must contain one raw int32"))?;
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
        current = *data.get(bytes_read).ok_or(TransportError::UnexpectedEof)?;
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

/// Decodes the body following `PID_Conn` using the exact C++ field order.
pub fn decode_connection_request_payload(data: &[u8]) -> Result<ConnectionRequest, TransportError> {
    let mut reader = ConnectionPayloadReader::new(data);
    let core = ClientCoreControlData {
        client_id: reader.read_raw_i32()?,
        activated: reader.read_bool()?,
        observer: reader.read_bool()?,
        name: reader.read_validated_client_name()?,
        nick: reader.read_validated_client_name()?,
        lobby_ready: reader.read_bool()?,
    };
    let build = reader.read_packed_i32()?;
    let password = reader.read_c_string()?;
    let connection_id = reader.read_packed_u32()?;
    Ok(ConnectionRequest {
        core,
        build,
        password,
        connection_id,
    })
}

/// Encodes the body following `PID_Conn` using the exact C++ field order.
pub fn encode_connection_request_payload(
    request: &ConnectionRequest,
) -> Result<Vec<u8>, TransportError> {
    let name = validate_name_no_empty(request.core.name.clone());
    let nick = validate_name_no_empty(request.core.nick.clone());
    let mut data = Vec::new();
    data.extend_from_slice(&request.core.client_id.to_ne_bytes());
    data.push(u8::from(request.core.activated));
    data.push(u8::from(request.core.observer));
    append_c_string(&mut data, &name);
    append_c_string(&mut data, &nick);
    data.push(u8::from(request.core.lobby_ready));
    encode_packed_i32(request.build, &mut data);
    append_c_string(&mut data, &request.password);
    encode_varint(request.connection_id, &mut data);
    Ok(data)
}

/// Decodes the body following `PID_ConnRe` using the exact C++ field order.
pub fn decode_connection_reply_payload(data: &[u8]) -> Result<ConnectionReply, TransportError> {
    let mut reader = ConnectionPayloadReader::new(data);
    let reply = ConnectionReply {
        ok: reader.read_bool()?,
        message: reader.read_c_string()?,
        wrong_password: reader.read_bool()?,
    };
    Ok(reply)
}

/// Encodes the body following `PID_ConnRe` using the exact C++ field order.
pub fn encode_connection_reply_payload(reply: &ConnectionReply) -> Result<Vec<u8>, TransportError> {
    let mut data = Vec::new();
    data.push(u8::from(reply.ok));
    append_c_string(&mut data, &reply.message);
    data.push(u8::from(reply.wrong_password));
    Ok(data)
}

fn append_c_string(data: &mut Vec<u8>, value: &LegacyCString) {
    data.extend_from_slice(value.as_bytes());
    data.push(0);
}

struct ConnectionPayloadReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> ConnectionPayloadReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, TransportError> {
        let value = self
            .data
            .get(self.offset)
            .copied()
            .ok_or(TransportError::UnexpectedEof)?;
        self.offset += 1;
        Ok(value)
    }

    fn read_bool(&mut self) -> Result<bool, TransportError> {
        self.read_u8().map(|value| value != 0)
    }

    fn read_raw_i32(&mut self) -> Result<i32, TransportError> {
        let end = self
            .offset
            .checked_add(size_of::<i32>())
            .ok_or(TransportError::UnexpectedEof)?;
        let bytes: [u8; size_of::<i32>()] = self
            .data
            .get(self.offset..end)
            .ok_or(TransportError::UnexpectedEof)?
            .try_into()
            .map_err(|_| TransportError::UnexpectedEof)?;
        self.offset = end;
        Ok(i32::from_ne_bytes(bytes))
    }

    fn read_c_string(&mut self) -> Result<LegacyCString, TransportError> {
        let remaining = self
            .data
            .get(self.offset..)
            .ok_or(TransportError::UnexpectedEof)?;
        let length = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(TransportError::UnexpectedEof)?;
        let end = self
            .offset
            .checked_add(length)
            .ok_or(TransportError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .ok_or(TransportError::UnexpectedEof)?
            .to_vec();
        self.offset = end + 1;
        LegacyCString::from_bytes(bytes).ok_or(TransportError::Malformed(
            "connection string contains an interior NUL",
        ))
    }

    fn read_validated_client_name(&mut self) -> Result<LegacyCString, TransportError> {
        let value = self.read_c_string()?;
        Ok(validate_name_no_empty(value))
    }

    fn read_packed_i32(&mut self) -> Result<i32, TransportError> {
        let (value, consumed) = decode_packed_i32(
            self.data
                .get(self.offset..)
                .ok_or(TransportError::UnexpectedEof)?,
        )?;
        self.offset += consumed;
        Ok(value)
    }

    fn read_packed_u32(&mut self) -> Result<u32, TransportError> {
        let (value, consumed) = decode_varint(
            self.data
                .get(self.offset..)
                .ok_or(TransportError::UnexpectedEof)?,
        )?;
        self.offset += consumed;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_packet::{
        ResourceChunkAvailability, ResourceChunkRange, ResourceDataPacket, ResourceDiscoverPacket,
        ResourcePacket, ResourcePacketCodecError, ResourceRequestPacket, ResourceStatusPacket,
    };
    use crate::{AddressPacket, NetworkAddress, NetworkProtocol};
    use lc_engine::{ClientCoreControlData, LegacyCString, NetworkResourceCore};
    use std::net::SocketAddr;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    fn expect_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(5 + payload.len());
        frame.push(TCP_FRAME_PREFIX);
        frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[tokio::test(flavor = "current_thread")]
    async fn known_cpp_packets_decode_or_skip_without_losing_following_tcp_frame() {
        // Real C++ bodies: packed client 7 + TCP IPv6 address for
        // PID_TCPSimOpen, then a successful zero-player league result.
        let tcp_sim_open = [
            PID_TCP_SIM_OPEN,
            0x07,
            0x01,
            b'[',
            b'2',
            b'0',
            b'0',
            b'1',
            b':',
            b'd',
            b'b',
            b'8',
            b':',
            b':',
            b'7',
            b']',
            b':',
            b'1',
            b'1',
            b'1',
            b'1',
            b'2',
            0x00,
        ];
        let league_results = [PID_LEAGUE_ROUND_RESULTS, 0x01, b'O', b'K', 0x00, 0x00];
        let ping = PingPacket {
            sent_at: 0x1122_3344,
            packet_counter: 7,
        };
        let mut ping_payload = vec![PID_PING];
        encode_ping(ping, &mut ping_payload);

        let mut frames = expect_frame(&tcp_sim_open);
        frames.extend(expect_frame(&league_results));
        frames.extend(expect_frame(&ping_payload));
        let (client, mut server) = duplex(256);
        server.write_all(&frames).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert!(matches!(
            transport.read_packet().await.unwrap(),
            InboundPacket::Ignored(PID_TCP_SIM_OPEN)
        ));
        let results = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            transport.read_message(),
        )
        .await
        .expect("league results blocked the following frame")
        .expect("valid league results disconnected the transport");
        assert_eq!(
            results,
            ControlMessage::LeagueRoundResults(LeagueRoundResultsPacket {
                success: true,
                result_string: LegacyCString::from_bytes(b"OK".to_vec()).unwrap(),
                players: Vec::new(),
            })
        );
        assert_eq!(transport.read_message().await.unwrap(), ControlMessage::Ping(ping));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn known_cpp_packet_exposes_a_skippable_outcome_before_the_next_message() {
        let ping = PingPacket {
            sent_at: 0x1234_5678,
            packet_counter: 9,
        };
        let mut ping_payload = vec![PID_PING];
        encode_ping(ping, &mut ping_payload);
        let tcp_sim_open = [
            PID_TCP_SIM_OPEN,
            0x07,
            0x01,
            b'[',
            b'2',
            b'0',
            b'0',
            b'1',
            b':',
            b'd',
            b'b',
            b'8',
            b':',
            b':',
            b'7',
            b']',
            b':',
            b'1',
            b'1',
            b'1',
            b'1',
            b'2',
            0x00,
        ];
        let mut frames = expect_frame(&tcp_sim_open);
        frames.extend(expect_frame(&ping_payload));
        let (client, mut server) = duplex(64);
        server.write_all(&frames).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert!(matches!(
            transport.read_packet().await.unwrap(),
            InboundPacket::Ignored(PID_TCP_SIM_OPEN)
        ));
        assert!(matches!(
            transport.read_packet().await.unwrap(),
            InboundPacket::Message(ControlMessage::Ping(received)) if received == ping
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn zero_length_frame_is_ignored_before_a_buffered_valid_message() {
        let ping = PingPacket {
            sent_at: 17,
            packet_counter: 3,
        };
        let mut ping_payload = vec![PID_PING];
        encode_ping(ping, &mut ping_payload);
        let mut frames = expect_frame(&[]);
        frames.extend(expect_frame(&ping_payload));
        let (client, mut server) = duplex(64);
        server.write_all(&frames).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(transport.read_message().await.unwrap(), ControlMessage::Ping(ping));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_known_cpp_packet_is_fatal_without_sticking_in_the_buffer() {
        let ping = PingPacket {
            sent_at: 29,
            packet_counter: 4,
        };
        let mut ping_payload = vec![PID_PING];
        encode_ping(ping, &mut ping_payload);
        let mut frames = expect_frame(&[PID_TCP_SIM_OPEN]);
        frames.extend(expect_frame(&ping_payload));
        let (client, mut server) = duplex(64);
        server.write_all(&frames).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert!(matches!(
            transport.read_packet().await.unwrap(),
            InboundPacket::Invalid {
                packet_type: PID_TCP_SIM_OPEN,
                error: TransportError::Malformed("invalid TCP simultaneous-open packet"),
            }
        ));
        assert!(matches!(
            transport.read_packet().await.unwrap(),
            InboundPacket::Message(ControlMessage::Ping(received)) if received == ping
        ));
        assert!(matches!(
            parse_complete_packet(&[PID_LEAGUE_ROUND_RESULTS, 1, b'O', b'K']),
            Err(TransportError::LeagueRoundResultsDecode(
                LeagueRoundResultsDecodeError::Legacy(LegacyControlError::UnexpectedEof)
            ))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn truly_unknown_cpp_packet_id_still_errors() {
        let frame = expect_frame(&[0x7e, 0xde, 0xad]);
        let (client, mut server) = duplex(32);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert!(matches!(
            transport.read_message().await,
            Err(TransportError::UnsupportedPacket(0x7e))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn opaque_complete_packet_send_preserves_every_nested_byte() {
        let packet = [PID_READY_CHECK, 7, 0, 0, 0, 1, 0, 0, 0, 0xde, 0xad];
        let recoverable = [PID_CONTROL, 0xaa];
        let mut expected = expect_frame(&packet);
        expected.extend(expect_frame(&recoverable));
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);

        transport
            .send_complete_packet_bytes(&packet)
            .await
            .unwrap();
        transport
            .send_complete_packet_bytes(&recoverable)
            .await
            .unwrap();
        assert_eq!(
            transport
                .create_post_mortem(9)
                .expect("raw relay enters the recovery log")
                .packets,
            vec![packet.to_vec(), recoverable.to_vec()]
        );
        drop(transport);
        let mut received = Vec::new();
        server.read_to_end(&mut received).await.unwrap();

        assert_eq!(received, expected);
    }

    async fn assert_resource_frame_round_trip(packet: ResourcePacket, payload: &[u8]) {
        let frame = expect_frame(payload);
        let (client, mut server) = duplex(512);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::Resource(packet.clone())
        );

        transport
            .send_message(ControlMessage::Resource(packet))
            .await
            .unwrap();
        drop(transport);
        let mut response = Vec::new();
        server.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, frame);
    }

    fn minimal_join_game_parameters() -> crate::JoinGameParametersEnvelope {
        let empty_players = crate::PlayerInfoListSnapshot {
            last_player_id: 0,
            clients: Vec::new(),
        };
        crate::JoinGameParametersEnvelope {
            random_seed: 0,
            startup_player_count: 0,
            max_players: 8,
            use_fair_crew: false,
            fair_crew_forced: false,
            fair_crew_strength: 0,
            allow_debug: true,
            is_network_game: true,
            control_rate: 1,
            auto_frame_skip: false,
            rules: Vec::new(),
            goals: Vec::new(),
            league: LegacyCString::default(),
            league_address: LegacyCString::default(),
            title: LegacyCString::from_bytes(b"No title".to_vec()).unwrap(),
            scenario: lc_engine::NetworkResourceCore::default(),
            game_resources: Vec::new(),
            player_infos: empty_players.clone(),
            restore_player_infos: empty_players,
            teams: crate::JoinTeamListSnapshot {
                active: 1,
                custom: 0,
                allow_hostility_change: 1,
                allow_team_switch: 0,
                auto_generate_teams: 1,
                last_team_id: 0,
                team_distribution: 0,
                team_colors: 0,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 0,
                teams: Vec::new(),
            },
            clients: crate::JoinClientRegistrySnapshot {
                clients: Vec::new(),
                local_client_id: None,
            },
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ping_and_pong_match_cpp_raw_dword_packets() {
        // C4PacketPing writes raw uint32 Time then PacketCounter; PID_Pong
        // echoes the exact body (src/C4Network2IO.cpp:1007-1028,1702-1718).
        let packet = PingPacket {
            sent_at: 0x0102_0304,
            packet_counter: 0x1122_3344,
        };
        let mut body = vec![PID_PING];
        body.extend_from_slice(&packet.sent_at.to_ne_bytes());
        body.extend_from_slice(&packet.packet_counter.to_ne_bytes());
        let (client, mut server) = duplex(64);
        server.write_all(&expect_frame(&body)).await.unwrap();
        let mut transport = ControlTransport::new(client);
        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::Ping(packet)
        );

        transport
            .send_message(ControlMessage::Pong(packet))
            .await
            .unwrap();
        drop(transport);
        let mut response = Vec::new();
        server.read_to_end(&mut response).await.unwrap();
        body[0] = PID_PONG;
        assert_eq!(response, expect_frame(&body));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_cpp_post_mortem_recovery_packet() {
        // C4PacketPostMortem writes three raw uint32 fields followed by each
        // complete C4NetIOPacket as a packed-length StdBuf, oldest first
        // (src/C4Network2IO.cpp:1379-1395,1497-1586; src/StdBuf.cpp:86-100).
        let frame = expect_frame(&[
            0x06, 0x44, 0x33, 0x22, 0x11, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04,
            0x10, 0x02, 0x00, 0xff, 0x04, 0x40, 0x01, 0x00, 0xff,
        ]);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::PostMortem(crate::PostMortemPacket {
                connection_id: 0x1122_3344,
                packet_counter: 7,
                packets: vec![
                    vec![0x10, 0x02, 0x00, 0xff],
                    vec![0x40, 0x01, 0x00, 0xff],
                ],
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sends_cpp_post_mortem_recovery_packet() {
        let packet = crate::PostMortemPacket {
            connection_id: 0x1122_3344,
            packet_counter: 7,
            packets: vec![
                vec![0x10, 0x02, 0x00, 0xff],
                vec![0x40, 0x01, 0x00, 0xff],
            ],
        };
        let expected = expect_frame(&[
            0x06, 0x44, 0x33, 0x22, 0x11, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04,
            0x10, 0x02, 0x00, 0xff, 0x04, 0x40, 0x01, 0x00, 0xff,
        ]);
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);

        transport
            .send_message(ControlMessage::PostMortem(packet))
            .await
            .unwrap();
        drop(transport);
        let mut response = Vec::new();
        server.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sent_recoverable_packet_enters_cpp_post_mortem_backlog() {
        // C4Network2IOConnection::Send records the complete packet before the
        // underlying transport attempt, and CreatePostMortem later retains that
        // exact body (src/C4Network2IO.cpp:1379-1395,1426-1457).
        let (client, _server) = duplex(64);
        let mut transport = ControlTransport::new(client);
        transport
            .send_message(ControlMessage::Status(NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 0,
                target_tick: -1,
            }))
            .await
            .unwrap();

        assert_eq!(
            transport.create_post_mortem(77),
            Some(crate::PostMortemPacket {
                connection_id: 77,
                packet_counter: 1,
                packets: vec![vec![0x10, 0x02, 0x00, 0xff]],
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn incoming_ping_acknowledges_the_recoverable_backlog() {
        // HandlePacket(PID_Ping) clears every logged packet below the peer's
        // reported next inbound counter after echoing its Pong
        // (src/C4Network2IO.cpp:1000-1007,1358-1377).
        let (client, mut server) = duplex(128);
        let mut transport = ControlTransport::new(client);
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: -1,
        };
        transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        transport
            .send_message(ControlMessage::StatusAck(status))
            .await
            .unwrap();
        let ping = PingPacket {
            sent_at: 123,
            packet_counter: 2,
        };
        let mut body = vec![PID_PING];
        encode_ping(ping, &mut body);
        server.write_all(&expect_frame(&body)).await.unwrap();

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::Ping(ping)
        );
        assert_eq!(transport.create_post_mortem(77), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn forward_request_matches_cpp_empty_negative_list_frame() {
        // C4PacketFwd writes Negative, packed ClientCnt, packed Clients, then
        // the length-prefixed complete nested packet (src/C4Network2IO.cpp:
        // 1644-1681; src/StdBuf.cpp:86-100).
        let packet = crate::ForwardPacket {
            negative_list: true,
            clients: Vec::new(),
            nested_packet: vec![0x40, 0x01, 0x00, 0xff],
        };
        let frame = vec![
            0xff, 0x08, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x04, 0x40, 0x01, 0x00, 0xff,
        ];
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::ForwardRequest(packet.clone())
        );

        transport
            .send_message(ControlMessage::ForwardRequest(packet))
            .await
            .unwrap();
        drop(transport);
        let mut response = Vec::new();
        server.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, frame);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn forwarded_packet_matches_cpp_empty_negative_list_frame() {
        // PID_Fwd uses the same C4PacketFwd body as PID_FwdReq and differs
        // only in its outer packet ID (src/C4PacketBase.h:95-96;
        // src/C4Packet2.cpp:58-59).
        let packet = crate::ForwardPacket {
            negative_list: true,
            clients: Vec::new(),
            nested_packet: vec![0x40, 0x01, 0x00, 0xff],
        };
        let frame = vec![
            0xff, 0x08, 0x00, 0x00, 0x00, 0x05, 0x01, 0x00, 0x04, 0x40, 0x01, 0x00, 0xff,
        ];
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::Forward(packet.clone())
        );

        transport
            .send_message(ControlMessage::Forward(packet))
            .await
            .unwrap();
        drop(transport);
        let mut response = Vec::new();
        server.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, frame);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_forwarding_frame_keeps_its_typed_network_error() {
        // StdBuf declares four nested bytes here but receives only three;
        // C++ raises EOF while unpacking C4PacketFwd
        // (src/C4Network2IO.cpp:1644-1681; src/StdBuf.cpp:86-100).
        let frame = expect_frame(&[PID_FORWARD_REQUEST, 0x00, 0x00, 0x04, 0x40, 0x01, 0x00]);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert!(matches!(
            transport.read_message().await,
            Err(TransportError::ForwardDecode(
                ForwardPacketCodecError::UnexpectedEof
            ))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lobby_countdown_matches_captured_cpp_frame_and_preserves_raw_i32() {
        // C4PacketCountdown writes its countdown as a native int32 and keeps
        // arbitrary values; -1 is the distinguished abort value
        // (src/C4GameLobby.h:47-65; src/C4GameLobby.cpp:43-48).
        let cases = [
            (
                LobbyCountdownPacket::new(5),
                vec![0xff, 0x05, 0x00, 0x00, 0x00, 0x20, 0x05, 0x00, 0x00, 0x00],
            ),
            (
                LobbyCountdownPacket::new(LobbyCountdownPacket::ABORT),
                expect_frame(&[0x20, 0xff, 0xff, 0xff, 0xff]),
            ),
            (
                LobbyCountdownPacket::new(i32::MIN),
                expect_frame(&[0x20, 0x00, 0x00, 0x00, 0x80]),
            ),
        ];

        for (packet, frame) in cases {
            let (client, mut server) = duplex(64);
            server.write_all(&frame).await.unwrap();
            let mut transport = ControlTransport::new(client);

            assert_eq!(
                transport.read_message().await.unwrap(),
                ControlMessage::LobbyCountdown(packet)
            );

            transport
                .send_message(ControlMessage::LobbyCountdown(packet))
                .await
                .unwrap();
            drop(transport);
            let mut response = Vec::new();
            server.read_to_end(&mut response).await.unwrap();
            assert_eq!(response, frame);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ready_check_matches_cpp_raw_dword_packet_in_both_directions() {
        // C4PacketBase::pack prefixes PID_ReadyCheck, and C4PacketReadyCheck
        // writes Client then Data as native int32 values
        // (src/C4PacketBase.h:127-130; src/C4Network2.h:480-502;
        // src/C4Network2IO.cpp:1674-1680; src/StdCompiler.cpp:104-107,125-132).
        let packet = ReadyCheckPacket {
            client_id: 7,
            data: ReadyCheckData::Request,
        };
        let frame = expect_frame(&[0x21, 0x07, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff]);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::ReadyCheck(packet)
        );

        transport
            .send_message(ControlMessage::ReadyCheck(packet))
            .await
            .unwrap();
        drop(transport);
        let mut response = Vec::new();
        server.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, frame);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ready_check_keeps_cpp_unknown_data_and_ignores_trailing_bytes() {
        // StdCompilerBinRead stops after the two fields without requiring EOF,
        // and GetData/IsReady retain arbitrary int32 values while treating
        // anything other than Ready as false (src/StdCompiler.h:380-387;
        // src/StdCompiler.cpp:228-239; src/C4Network2.h:494-497).
        let frame = expect_frame(&[
            0x21, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0xde, 0xad,
        ]);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        let ControlMessage::ReadyCheck(packet) = transport.read_message().await.unwrap() else {
            panic!("expected ready-check packet");
        };
        assert_eq!(packet.data, ReadyCheckData::Other(2));
        assert!(!packet.data.vote_requested());
        assert!(!packet.data.is_ready());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pid_addr_matches_cpp_tcp_frame_in_both_directions() {
        // C4PacketAddr is PID_Addr (0x12) followed by packed ClientID and
        // C4Network2Address; the latter writes protocol then NUL-terminated
        // endpoint text (src/C4PacketBase.h:109-110;
        // src/C4Network2Client.cpp:656-662; src/C4Network2Address.cpp:486-505).
        let packet = AddressPacket {
            client_id: 42,
            address: NetworkAddress::new(
                NetworkProtocol::Tcp,
                "203.0.113.7:11112".parse::<SocketAddr>().unwrap(),
            ),
        };
        let frame = expect_frame(&[
            0x12, 0x2a, 0x01, b'2', b'0', b'3', b'.', b'0', b'.', b'1', b'1', b'3', b'.', b'7',
            b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ]);
        let (client, mut server) = duplex(128);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::Address(packet)
        );

        transport
            .send_message(ControlMessage::Address(packet))
            .await
            .unwrap();
        drop(transport);
        let mut response = Vec::new();
        server.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, frame);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pid_net_res_discover_matches_cpp_tcp_frame_in_both_directions() {
        // C4PacketResDiscover is PID_NetResDis (0x30), followed by a packed
        // int32 count and native int32 IDs (src/C4PacketBase.h:131-136;
        // src/C4Network2IO.cpp:1753-1757).
        let packet = ResourcePacket::Discover(ResourceDiscoverPacket {
            resource_ids: vec![0x0102_0304, -1, 128],
        });
        assert_resource_frame_round_trip(
            packet,
            &[
                0x30, 0x03, 0x04, 0x03, 0x02, 0x01, 0xff, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00,
            ],
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pid_net_res_status_matches_cpp_tcp_frame_in_both_directions() {
        // C4PacketResStatus is PID_NetResStat (0x31), followed by native ResID
        // and the packed chunk-count/range pairs (src/C4PacketBase.h:131-136;
        // src/C4Network2IO.cpp:1726-1730; src/C4Network2Res.cpp:321-350).
        let packet = ResourcePacket::Status(ResourceStatusPacket {
            resource_id: 0x0102_0304,
            chunks: ResourceChunkAvailability {
                chunk_count: 300,
                ranges: vec![
                    ResourceChunkRange {
                        start: 0,
                        length: 2,
                    },
                    ResourceChunkRange {
                        start: 128,
                        length: 172,
                    },
                ],
            },
        });
        assert_resource_frame_round_trip(
            packet,
            &[
                0x31, 0x04, 0x03, 0x02, 0x01, 0xac, 0x02, 0x02, 0x00, 0x02, 0x80, 0x01, 0xac, 0x01,
            ],
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pid_net_res_derive_matches_cpp_tcp_frame_in_both_directions() {
        // PID_NetResDerive (0x32) directly compiles C4Network2ResCore
        // (src/C4Packet2.cpp:90; src/C4Network2Res.cpp:114-143).
        let packet = ResourcePacket::Derive(NetworkResourceCore {
            resource_type: 2,
            id: -1,
            derived_id: 0x0102_0304,
            loadable: false,
            contents_crc: 0x1122_3344,
            filename: LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
            author: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
            ..NetworkResourceCore::default()
        });
        assert_resource_frame_round_trip(
            packet,
            &[
                0x32, 0x02, 0xff, 0xff, 0xff, 0xff, 0x04, 0x03, 0x02, 0x01, 0x00, 0x44, 0x33, 0x22,
                0x11, 0x00, b'S', b'c', b'e', b'n', b'a', b'r', b'i', b'o', b'.', b'c', b'4', b's',
                0x00, b'A', b'l', b'i', b'c', b'e', 0x00,
            ],
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pid_net_res_request_matches_cpp_tcp_frame_in_both_directions() {
        // C4PacketResRequest is PID_NetResReq (0x33), followed by native ResID
        // and packed Chunk (src/C4PacketBase.h:131-136;
        // src/C4Network2IO.cpp:1764-1768).
        let packet = ResourcePacket::Request(ResourceRequestPacket {
            resource_id: -2,
            chunk: 128,
        });
        assert_resource_frame_round_trip(packet, &[0x33, 0xfe, 0xff, 0xff, 0xff, 0x80, 0x01]).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pid_net_res_data_matches_cpp_tcp_frame_in_both_directions() {
        // C4Network2ResChunk is PID_NetResData (0x34), followed by native
        // ResID/Chunk and StdBuf's packed length + bytes
        // (src/C4PacketBase.h:131-136; src/C4Network2Res.cpp:1321-1328).
        let packet = ResourcePacket::Data(ResourceDataPacket {
            resource_id: 0x0102_0304,
            chunk: 0xa0b0_c0d0,
            data: vec![0xde, 0xad, 0xbe],
        });
        assert_resource_frame_round_trip(
            packet,
            &[
                0x34, 0x04, 0x03, 0x02, 0x01, 0xd0, 0xc0, 0xb0, 0xa0, 0x03, 0xde, 0xad, 0xbe,
            ],
        )
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resource_codec_errors_remain_typed_at_the_transport_boundary() {
        // C4PacketResDiscover's fixed array rejects counts above 16
        // (src/C4Network2Res.h:420; src/C4Network2IO.cpp:1745-1757).
        let over_capacity = ResourcePacket::Discover(ResourceDiscoverPacket {
            resource_ids: (0..17).collect(),
        });
        let (client, _server) = duplex(64);
        let mut transport = ControlTransport::new(client);
        assert!(matches!(
            transport
                .send_message(ControlMessage::Resource(over_capacity))
                .await,
            Err(TransportError::ResourceEncode(
                ResourcePacketCodecError::DiscoverCountOutOfRange(17)
            ))
        ));

        let (client, mut server) = duplex(64);
        server
            .write_all(&expect_frame(&[0x30, 0x11]))
            .await
            .unwrap();
        let mut transport = ControlTransport::new(client);
        assert!(matches!(
            transport.read_message().await,
            Err(TransportError::ResourceDecode(
                ResourcePacketCodecError::DiscoverCountOutOfRange(17)
            ))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sends_cpp_connection_request_frame() {
        // C4Network2IO::SendConnPackets sends a packed PID_Conn over the
        // normal C4NetIOTCP frame (src/C4Network2IO.cpp:1223-1252;
        // src/C4NetIO.cpp:1287-1323).
        let request = ConnectionRequest {
            core: ClientCoreControlData {
                client_id: -1,
                activated: false,
                observer: true,
                name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                nick: LegacyCString::from_bytes(b"Ali".to_vec()).unwrap(),
                lobby_ready: false,
            },
            build: 362,
            password: LegacyCString::from_bytes(b"s3cret".to_vec()).unwrap(),
            connection_id: 0x0102_0304,
        };
        let (client, mut server) = duplex(128);
        let mut transport = ControlTransport::new(client);

        transport
            .send_message(ControlMessage::ConnectionRequest(request))
            .await
            .unwrap();
        drop(transport);

        let mut bytes = Vec::new();
        server.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(
            bytes,
            expect_frame(&[
                0x02, 0xff, 0xff, 0xff, 0xff, 0x00, 0x01, b'A', b'l', b'i', b'c', b'e', 0x00, b'A',
                b'l', b'i', 0x00, 0x00, 0x6a, 0x02, b's', b'3', b'c', b'r', b'e', b't', 0x00, 0x84,
                0x86, 0x88, 0x08,
            ])
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sends_cpp_connection_reply_frame() {
        // C4Network2IO::HandlePacket answers PID_Conn with PID_ConnRe through
        // the same C4NetIOTCP frame (src/C4Network2IO.cpp:965-1005;
        // src/C4Network2IO.cpp:1630-1642).
        let reply = ConnectionReply {
            ok: false,
            message: LegacyCString::from_bytes(b"wrong password".to_vec()).unwrap(),
            wrong_password: true,
        };
        let (client, mut server) = duplex(128);
        let mut transport = ControlTransport::new(client);

        transport
            .send_message(ControlMessage::ConnectionReply(reply))
            .await
            .unwrap();
        drop(transport);

        let mut bytes = Vec::new();
        server.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(
            bytes,
            expect_frame(&[
                0x03, 0x00, b'w', b'r', b'o', b'n', b'g', b' ', b'p', b'a', b's', b's', b'w', b'o',
                b'r', b'd', 0x00, 0x01,
            ])
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_cpp_connection_request_frame() {
        // PID_Conn is accepted before a connection reaches half-accepted state
        // (src/C4Network2IO.cpp:802-813,954-1005).
        let frame = expect_frame(&[
            0x02, 0xff, 0xff, 0xff, 0xff, 0x00, 0x01, b'A', b'l', b'i', b'c', b'e', 0x00, b'A',
            b'l', b'i', 0x00, 0x00, 0x6a, 0x02, b's', b'3', b'c', b'r', b'e', b't', 0x00, 0x84,
            0x86, 0x88, 0x08,
        ]);
        let (client, mut server) = duplex(128);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        let ControlMessage::ConnectionRequest(request) = transport.read_message().await.unwrap()
        else {
            panic!("expected connection request");
        };
        assert_eq!(request.core.client_id, -1);
        assert_eq!(request.core.name.as_bytes(), b"Alice");
        assert_eq!(request.build, 362);
        assert_eq!(request.connection_id, 0x0102_0304);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_cpp_connection_reply_frame() {
        // PID_ConnRe completes or rejects one half of the mutual connection
        // acceptance (src/C4Network2IO.cpp:987-1005).
        let frame = expect_frame(&[
            0x03, 0x00, b'w', b'r', b'o', b'n', b'g', b' ', b'p', b'a', b's', b's', b'w', b'o',
            b'r', b'd', 0x00, 0x01,
        ]);
        let (client, mut server) = duplex(128);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        let ControlMessage::ConnectionReply(reply) = transport.read_message().await.unwrap() else {
            panic!("expected connection reply");
        };
        assert!(!reply.ok);
        assert_eq!(reply.message.as_bytes(), b"wrong password");
        assert!(reply.wrong_password);
    }

    #[test]
    fn cpp_connection_decoder_ignores_trailing_payload_bytes() {
        // CompileFromBuf does not require StdCompilerBinRead to be at EOF
        // (src/StdCompiler.h:372-385; src/StdCompiler.cpp:241-244).
        let reply = decode_connection_reply_payload(&[0x01, 0x00, 0x00, 0xaa])
            .expect("C++ ignores data after the compiled ConnRe fields");

        assert!(reply.ok);
        assert!(reply.message.is_empty());
        assert!(!reply.wrong_password);
    }

    #[test]
    fn outgoing_connection_request_enforces_cpp_client_name_invariant() {
        // Locally constructed C4ClientCore names pass through the same
        // VAL_NameNoEmpty type invariant before C4PacketConn serialization
        // (src/C4Client.h:43-51; src/C4InputValidation.h:87-113).
        let request = ConnectionRequest {
            core: ClientCoreControlData {
                client_id: -1,
                activated: false,
                observer: false,
                name: LegacyCString::from_bytes(b" {<i>Alice</i>{ ".to_vec()).unwrap(),
                nick: LegacyCString::default(),
                lobby_ready: false,
            },
            build: 362,
            password: LegacyCString::default(),
            connection_id: 0,
        };

        assert_eq!(
            encode_connection_request_payload(&request).unwrap(),
            [
                0xff, 0xff, 0xff, 0xff, 0x00, 0x00, b'A', b'l', b'i', b'c', b'e', 0x00, b'e', b'm',
                b'p', b't', b'y', 0x00, 0x00, 0x6a, 0x02, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn incoming_connection_request_applies_cpp_client_name_validation() {
        // Binary C4ClientCore compilation uses VAL_NameNoEmpty for Name and
        // Nick (src/C4Client.cpp:75-83; src/C4InputValidation.cpp:97-118).
        let dirty_name = b"  {<c ff0000>ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890</c>{  ";
        let mut payload = (-1_i32).to_ne_bytes().to_vec();
        payload.extend_from_slice(&[0x00, 0x00]);
        payload.extend_from_slice(dirty_name);
        payload.push(0x00);
        payload.push(0x00);
        payload.push(0x00);
        payload.extend_from_slice(&[0x6a, 0x02, 0x00, 0x00]);

        let request = decode_connection_request_payload(&payload)
            .expect("source-grounded C4PacketConn bytes decode");

        assert_eq!(
            request.core.name.as_bytes(),
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZ1234"
        );
        assert_eq!(request.core.nick.as_bytes(), b"empty");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sends_cpp_join_data_frame() {
        // SendJoinData uses the ordinary PID_JoinData packet after the first
        // connection becomes accepted (src/C4Network2.cpp:1768-1784,1820-1849).
        let envelope = crate::JoinDataEnvelope {
            client_id: 3,
            start_control_tick: 17,
            status: NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 1,
                target_tick: -1,
            },
            dynamic: lc_engine::NetworkResourceCore::default(),
            parameters: minimal_join_game_parameters(),
        };
        let expected_payload = crate::encode_join_data_envelope(&envelope).unwrap();
        let (client, mut server) = duplex(128);
        let mut transport = ControlTransport::new(client);

        transport
            .send_message(ControlMessage::JoinData(Box::new(envelope)))
            .await
            .unwrap();
        drop(transport);

        let mut bytes = Vec::new();
        server.read_to_end(&mut bytes).await.unwrap();
        let mut packet = vec![0x15];
        packet.extend(expected_payload);
        assert_eq!(bytes, expect_frame(&packet));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_cpp_join_data_frame() {
        // PID_JoinData is host-to-client only and is processed after connection
        // acceptance while the client is still GS_Init
        // (src/C4Network2.cpp:938-946,1574-1623).
        let envelope = crate::JoinDataEnvelope {
            client_id: 3,
            start_control_tick: 17,
            status: NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 1,
                target_tick: -1,
            },
            dynamic: lc_engine::NetworkResourceCore::default(),
            parameters: minimal_join_game_parameters(),
        };
        let mut packet = vec![0x15];
        packet.extend(crate::encode_join_data_envelope(&envelope).unwrap());
        let (client, mut server) = duplex(128);
        server.write_all(&expect_frame(&packet)).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::JoinData(Box::new(envelope))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_control_packet() {
        let payload = [PID_CONTROL, 0x0C, 0x22, 0xff];
        let frame = expect_frame(&payload);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        match transport.read_message().await.unwrap() {
            ControlMessage::Control(packet) => {
                assert_eq!(packet.client_id(), 12);
                assert_eq!(packet.tick(), 34);
                assert_eq!(packet.payload(), &[0xff]);
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_multibyte_signed_packed_ints() {
        // mkIntPackAdapt writes client 300 as [0xAC, 0x02] and tick 2000 as
        // [0x50, 0x0F] (src/C4GameControlNetwork.cpp:867-872).
        let payload = [PID_CONTROL, 0xAC, 0x02, 0x50, 0x0F, 0xff];
        let frame = expect_frame(&payload);
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);
        match transport.read_message().await.unwrap() {
            ControlMessage::Control(packet) => {
                assert_eq!(packet.client_id(), 300);
                assert_eq!(packet.tick(), 2000);
                assert_eq!(packet.payload(), &[0xff]);
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn parses_cpp_control_request_signed_ticks() {
        for (payload, from_tick) in [
            (&[PID_CONTROL_REQ, 0x40, 0x00][..], 64),
            (&[PID_CONTROL_REQ, 0x96, 0x01][..], 150),
            (&[PID_CONTROL_REQ, 0x40, 0x00, 0xaa][..], 64),
        ] {
            assert_eq!(
                parse_complete_packet(payload).unwrap(),
                Some(ControlMessage::Request { from_tick })
            );
        }

        assert!(matches!(
            parse_complete_packet(&[PID_CONTROL_REQ, 0xff]),
            Err(TransportError::NegativeControlTick(-1))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_player_info_update_request() {
        // C4PacketBase::pack prefixes PID_PlayerInfoUpdReq (0x16) before the
        // C4ClientPlayerInfos body (src/C4Packet2.cpp:140-143;
        // src/C4PlayerInfo.cpp:601-630,1800-1803). These bytes come from the
        // live C++ player-info-update codec oracle fixture.
        let payload = [
            0x16, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, b'P', 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x33, 0x22, 0x11, 0x00, 0x33, 0x22, 0x11,
            0x00, 0x00, 0x00, 0x00, b'N', b'O', b'N', b'E', 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
            0x00, 0x00, 0x00,
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
            .payload(vec![0xff]);
        transport
            .send_message(ControlMessage::Control(packet))
            .await
            .unwrap();
        drop(transport);

        let mut buf = Vec::new();
        server.read_to_end(&mut buf).await.unwrap();
        let expected = expect_frame(&[PID_CONTROL, 0x0C, 0x22, 0xff]);
        assert_eq!(buf, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_empty_control_packet_matches_cpp_single_envelope() {
        // C4GameControlPacket::CompileFunc writes packed ClientID, packed
        // CtrlTick, and then C4Control exactly once
        // (src/C4GameControlNetwork.cpp:867-872). C4NetIOTCP::PackPacket's
        // size includes the PID, so this four-byte body has size 4
        // (src/C4NetIO.cpp:1287-1301).
        let packet = crate::encode_control_packet(&crate::LegacyControlFrame {
            client_id: 1,
            tick: 0,
            timestamp_ms: 0,
            controls: Vec::new(),
        })
        .expect("empty control frame encodes");
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);

        transport
            .send_message(ControlMessage::Control(packet))
            .await
            .unwrap();
        drop(transport);

        let mut actual = Vec::new();
        server.read_to_end(&mut actual).await.unwrap();
        assert_eq!(actual, [0xff, 0x04, 0, 0, 0, PID_CONTROL, 1, 0, 0xff]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_complete_control_packet_uses_cpp_negative_one_client_id() {
        // PackCompleteCtrl assigns C4ClientIDAll (-1), and CompileFunc writes
        // it through mkIntPackAdapt (src/C4GameControlNetwork.cpp:759-769,
        // 867-872; src/C4GameControlNetwork.h:25-27).
        let packet = crate::encode_control_packet(&crate::LegacyControlFrame {
            client_id: crate::BROADCAST_CLIENT_ID,
            tick: 7,
            timestamp_ms: 0,
            controls: Vec::new(),
        })
        .expect("complete control frame encodes");
        let (client, mut server) = duplex(64);
        let mut transport = ControlTransport::new(client);

        transport
            .send_message(ControlMessage::Control(packet))
            .await
            .unwrap();
        drop(transport);

        let mut actual = Vec::new();
        server.read_to_end(&mut actual).await.unwrap();
        assert_eq!(actual, [0xff, 0x04, 0, 0, 0, PID_CONTROL, 0xff, 7, 0xff]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_complete_control_packet_negative_one_client_id() {
        // C4ClientIDAll is the sole accepted negative client ID in a complete
        // C4GameControlPacket (src/C4GameControlNetwork.h:25-27).
        let frame = [0xff, 0x04, 0, 0, 0, PID_CONTROL, 0xff, 7, 0xff];
        let (client, mut server) = duplex(64);
        server.write_all(&frame).await.unwrap();
        let mut transport = ControlTransport::new(client);

        assert_eq!(
            transport.read_message().await.unwrap(),
            ControlMessage::Control(
                ControlPacket::builder(crate::BROADCAST_CLIENT_ID, 7).payload(vec![0xff])
            )
        );
    }

    #[test]
    fn rejects_control_client_id_below_cpp_all_sentinel() {
        // C4ClientIDAll aliases the sole C4ClientIDUnknown sentinel (-1)
        // (src/C4GameControlNetwork.h:25-27; src/C4Client.h:25-28).
        assert!(matches!(
            parse_complete_packet(&[PID_CONTROL, 0xfe, 0, 0xff]),
            Err(TransportError::NegativeControlClientId(-2))
        ));
    }

    #[test]
    fn rejects_negative_control_tick() {
        // Runtime C4GameControlPacket ticks are serialized signed but queued
        // from non-negative control ticks (src/C4GameControlNetwork.cpp:156-163).
        assert!(matches!(
            parse_complete_packet(&[PID_CONTROL, 1, 0xff, 0xff]),
            Err(TransportError::NegativeControlTick(-1))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_request_round_trip_matches_cpp_signed_pack_reference() {
        let cases: &[(Tick, &[u8])] = &[
            (0, &[0x00]),
            (63, &[0x3f]),
            (64, &[0x40, 0x00]),
            (127, &[0x7f, 0x00]),
            (128, &[0x80, 0x01]),
            (191, &[0xbf, 0x01]),
            (192, &[0x40, 0x01]),
            (8191, &[0x7f, 0x3f]),
            (8192, &[0x80, 0x40, 0x00]),
            (16383, &[0x7f, 0x7f, 0x00]),
            (16384, &[0x80, 0x80, 0x01]),
            (100000, &[0xa0, 0x8d, 0x06]),
        ];

        for &(from_tick, encoded) in cases {
            let mut payload = vec![PID_CONTROL_REQ];
            payload.extend_from_slice(encoded);

            let (client, mut server) = duplex(64);
            let mut transport = ControlTransport::new(client);
            transport
                .send_message(ControlMessage::Request { from_tick })
                .await
                .unwrap();
            drop(transport);

            let mut frame = Vec::new();
            server.read_to_end(&mut frame).await.unwrap();
            assert_eq!(frame, expect_frame(&payload), "from_tick={from_tick}");
            assert_eq!(
                parse_complete_packet(&payload).unwrap(),
                Some(ControlMessage::Request { from_tick }),
                "from_tick={from_tick}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_control_request_tick_above_cpp_i32_range() {
        let (client, _server) = duplex(16);
        let mut transport = ControlTransport::new(client);
        let from_tick = i32::MAX as Tick + 1;

        assert!(matches!(
            transport
                .send_message(ControlMessage::Request { from_tick })
                .await,
            Err(TransportError::ControlTickOutOfRange(tick)) if tick == from_tick
        ));
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
