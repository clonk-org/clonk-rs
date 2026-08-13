use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use std::{collections::BTreeMap, collections::BTreeSet};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::ClientId;

/// Duration represented by every encoded voice payload. Keeping this fixed
/// avoids trusting a sender-controlled sample count during decoder allocation.
pub const VOICE_FRAME_DURATION_MS: u16 = 20;
/// V1's exact independently decodable IMA ADPCM payload size. A different
/// codec or frame shape must use a new wire signature rather than making this
/// version sender-sized.
pub const VOICE_PAYLOAD_BYTES: usize = 164;
pub const MAX_VOICE_PAYLOAD_BYTES: usize = VOICE_PAYLOAD_BYTES;
pub(crate) const MAX_VOICE_DIRECT_RECIPIENTS: usize = 32;
pub(crate) const VOICE_ROUTE_COOKIE_BYTES: usize = 16;

/// A Rust-only datagram signature. It is recognized before the reliable-UDP
/// packet header is inspected, so its following bytes can never advance the
/// reliable receive window or enter PostMortem recovery.
pub(crate) const VOICE_MEDIA_PREFIX: &[u8; 5] = b"\x7fC4V1";

const VOICE_PACKET_DIRECT: u8 = 0;
const VOICE_PACKET_RELAY_REQUEST: u8 = 1;
const VOICE_PACKET_RELAYED: u8 = 2;
const VOICE_PACKET_FIXED_HEADER: usize = 18;
pub(crate) const MAX_VOICE_WIRE_BYTES: usize = VOICE_MEDIA_PREFIX.len()
    + VOICE_ROUTE_COOKIE_BYTES
    + VOICE_PACKET_FIXED_HEADER
    + MAX_VOICE_DIRECT_RECIPIENTS * std::mem::size_of::<ClientId>()
    + MAX_VOICE_PAYLOAD_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceFrame {
    /// Authenticated network client. Outbound constructors initialize this to
    /// the broadcast sentinel; session ingress always overwrites it from the
    /// established route before publishing the frame.
    pub client_id: ClientId,
    pub player_id: i32,
    pub stream_epoch: u32,
    pub sequence: u16,
    pub payload: Vec<u8>,
}

/// An unpredictable bearer cookie generated independently for each admitted
/// reliable-UDP route. It authenticates best-effort media to that route but
/// does not encrypt the media or protect an on-path observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VoiceRouteCookie([u8; VOICE_ROUTE_COOKIE_BYTES]);

impl VoiceRouteCookie {
    pub(crate) fn generate() -> Option<Self> {
        let mut bytes = [0_u8; VOICE_ROUTE_COOKIE_BYTES];
        rustls::crypto::ring::default_provider()
            .secure_random
            .fill(&mut bytes)
            .ok()?;
        Some(Self(bytes))
    }

    pub(crate) const fn from_bytes(bytes: [u8; VOICE_ROUTE_COOKIE_BYTES]) -> Self {
        Self(bytes)
    }

    pub(crate) const fn into_bytes(self) -> [u8; VOICE_ROUTE_COOKIE_BYTES] {
        self.0
    }

    fn matches(self, candidate: &[u8]) -> bool {
        candidate.len() == VOICE_ROUTE_COOKIE_BYTES
            && self
                .0
                .iter()
                .zip(candidate)
                .fold(0_u8, |different, (expected, actual)| {
                    different | (expected ^ actual)
                })
                == 0
    }
}

/// Directional authentication state for one admitted transport route. The
/// local receive cookie is sent through that route's reliable stream; the peer
/// receive cookie is learned from the corresponding reliable announcement and
/// is placed on outbound best-effort datagrams.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VoiceRouteAuthentication {
    local_receive_cookie: Option<VoiceRouteCookie>,
    peer_receive_cookie: Option<VoiceRouteCookie>,
}

impl VoiceRouteAuthentication {
    pub(crate) fn new_udp() -> Self {
        Self {
            local_receive_cookie: VoiceRouteCookie::generate(),
            peer_receive_cookie: None,
        }
    }

    pub(crate) fn announcement(self) -> Option<crate::PortCapabilities> {
        self.local_receive_cookie
            .map(|cookie| crate::PortCapabilities::supported().with_voice_cookie(cookie))
    }

    pub(crate) fn record_peer_capabilities(&mut self, capabilities: crate::PortCapabilities) {
        if self.peer_receive_cookie.is_none()
            && capabilities.has(crate::PortCapabilities::VOICE_CHAT)
        {
            self.peer_receive_cookie = capabilities.voice_cookie();
        }
    }

    pub(crate) fn receive_cookie(self) -> Option<VoiceRouteCookie> {
        self.local_receive_cookie
    }

    pub(crate) fn send_cookie(self) -> Option<VoiceRouteCookie> {
        self.peer_receive_cookie
    }

    pub(crate) const fn is_negotiated(self) -> bool {
        self.local_receive_cookie.is_some() && self.peer_receive_cookie.is_some()
    }
}

/// Cloneable, nonblocking application side of a session's bounded media
/// queue. It can be detached before the owning session handle moves into a
/// network-manager worker.
#[derive(Clone, Debug)]
pub struct VoiceSender {
    pub(crate) tx: mpsc::Sender<VoiceFrame>,
    available: Arc<AtomicBool>,
}

const VOICE_FRAME_CREDIT: Duration = Duration::from_millis(20);
const VOICE_BURST_CREDIT: Duration = Duration::from_millis(1_500);

#[derive(Clone, Copy, Debug)]
struct VoiceIngressBudget {
    credit: Duration,
    updated_at: Instant,
}

/// Per-authenticated-client token bucket. A conforming 20 ms stream consumes
/// exactly the 50 credits replenished each second; the bounded burst absorbs
/// scheduler stalls without allowing a peer to amplify unbounded traffic.
#[derive(Debug, Default)]
pub(crate) struct VoiceIngressLimiter {
    sources: BTreeMap<ClientId, VoiceIngressBudget>,
}

impl VoiceIngressLimiter {
    pub(crate) fn allow(&mut self, source: ClientId, now: Instant) -> bool {
        let budget = self.sources.entry(source).or_insert(VoiceIngressBudget {
            credit: VOICE_BURST_CREDIT,
            updated_at: now,
        });
        let elapsed = now.saturating_duration_since(budget.updated_at);
        budget.credit = budget
            .credit
            .saturating_add(elapsed)
            .min(VOICE_BURST_CREDIT);
        budget.updated_at = now;
        if budget.credit < VOICE_FRAME_CREDIT {
            return false;
        }
        budget.credit -= VOICE_FRAME_CREDIT;
        true
    }

    pub(crate) fn retain_sources(&mut self, sources: impl IntoIterator<Item = ClientId>) {
        let sources = sources.into_iter().collect::<BTreeSet<_>>();
        self.sources.retain(|source, _| sources.contains(source));
    }
}

pub(crate) const fn voice_media_may_run(
    control_pending: bool,
    reliable_input_pending: bool,
) -> bool {
    !control_pending && !reliable_input_pending
}

impl VoiceSender {
    pub(crate) fn new(tx: mpsc::Sender<VoiceFrame>) -> Self {
        Self {
            tx,
            available: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn try_send(&self, frame: VoiceFrame) -> Result<(), VoiceSendError> {
        validate_voice_payload(&frame.payload).map_err(VoiceSendError::Invalid)?;
        self.tx.try_send(frame).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => VoiceSendError::Full,
            mpsc::error::TrySendError::Closed(_) => VoiceSendError::Closed,
        })
    }

    /// Whether at least one established UDP link has positively negotiated
    /// voice support. This remains false for an all-C++ session.
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    pub(crate) fn availability(&self) -> Arc<AtomicBool> {
        self.available.clone()
    }
}

impl VoiceFrame {
    pub fn outbound(
        player_id: i32,
        stream_epoch: u32,
        sequence: u16,
        payload: Vec<u8>,
    ) -> Result<Self, VoiceCodecError> {
        validate_voice_payload(&payload)?;
        Ok(Self {
            client_id: crate::BROADCAST_CLIENT_ID,
            player_id,
            stream_epoch,
            sequence,
            payload,
        })
    }

    pub(crate) fn with_authenticated_source(mut self, source_client_id: ClientId) -> Self {
        self.client_id = source_client_id;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VoicePacket {
    Direct(VoiceFrame),
    RelayRequest {
        frame: VoiceFrame,
        direct_recipients: Vec<ClientId>,
    },
    Relayed(VoiceFrame),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VoiceCodecError {
    #[error("voice payload has {actual} bytes; V1 requires exactly {expected}")]
    InvalidPayloadLength { actual: usize, expected: usize },
    #[error("voice relay names {0} direct recipients; at most {MAX_VOICE_DIRECT_RECIPIENTS} are allowed")]
    TooManyDirectRecipients(usize),
    #[error("voice media packet signature is missing")]
    MissingSignature,
    #[error("voice media route cookie is missing or invalid")]
    InvalidRouteCookie,
    #[error("voice media packet is truncated")]
    Truncated,
    #[error("voice media packet kind {0} is unknown")]
    UnknownKind(u8),
    #[error("voice media packet has trailing bytes")]
    TrailingBytes,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VoiceSendError {
    #[error("voice frame is invalid: {0}")]
    Invalid(VoiceCodecError),
    #[error("voice send queue is full; frame dropped")]
    Full,
    #[error("voice session is closed")]
    Closed,
}

pub(crate) fn validate_voice_payload(payload: &[u8]) -> Result<(), VoiceCodecError> {
    if payload.len() != VOICE_PAYLOAD_BYTES {
        return Err(VoiceCodecError::InvalidPayloadLength {
            actual: payload.len(),
            expected: VOICE_PAYLOAD_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn is_voice_media_datagram(wire: &[u8]) -> bool {
    wire.starts_with(VOICE_MEDIA_PREFIX)
}

pub(crate) fn encode_voice_packet(packet: &VoicePacket) -> Result<Vec<u8>, VoiceCodecError> {
    let (kind, frame, direct_recipients): (u8, &VoiceFrame, &[ClientId]) = match packet {
        VoicePacket::Direct(frame) => (VOICE_PACKET_DIRECT, frame, &[]),
        VoicePacket::RelayRequest {
            frame,
            direct_recipients,
        } => (VOICE_PACKET_RELAY_REQUEST, frame, direct_recipients),
        VoicePacket::Relayed(frame) => (VOICE_PACKET_RELAYED, frame, &[]),
    };
    validate_voice_payload(&frame.payload)?;
    if direct_recipients.len() > MAX_VOICE_DIRECT_RECIPIENTS {
        return Err(VoiceCodecError::TooManyDirectRecipients(
            direct_recipients.len(),
        ));
    }

    let mut wire = Vec::with_capacity(
        VOICE_MEDIA_PREFIX.len()
            + VOICE_PACKET_FIXED_HEADER
            + std::mem::size_of_val(direct_recipients)
            + frame.payload.len(),
    );
    wire.extend_from_slice(VOICE_MEDIA_PREFIX);
    wire.push(kind);
    wire.extend_from_slice(&frame.client_id.to_le_bytes());
    wire.extend_from_slice(&frame.player_id.to_le_bytes());
    wire.extend_from_slice(&frame.stream_epoch.to_le_bytes());
    wire.extend_from_slice(&frame.sequence.to_le_bytes());
    wire.push(direct_recipients.len() as u8);
    wire.extend_from_slice(&(frame.payload.len() as u16).to_le_bytes());
    for client_id in direct_recipients {
        wire.extend_from_slice(&client_id.to_le_bytes());
    }
    wire.extend_from_slice(&frame.payload);
    Ok(wire)
}

pub(crate) fn encode_authenticated_voice_packet(
    cookie: VoiceRouteCookie,
    packet: &VoicePacket,
) -> Result<Vec<u8>, VoiceCodecError> {
    let packet = encode_voice_packet(packet)?;
    let mut wire = Vec::with_capacity(packet.len() + VOICE_ROUTE_COOKIE_BYTES);
    wire.extend_from_slice(VOICE_MEDIA_PREFIX);
    wire.extend_from_slice(&cookie.into_bytes());
    wire.extend_from_slice(&packet[VOICE_MEDIA_PREFIX.len()..]);
    Ok(wire)
}

pub(crate) fn voice_datagram_has_cookie(wire: &[u8], expected: VoiceRouteCookie) -> bool {
    wire.strip_prefix(VOICE_MEDIA_PREFIX)
        .and_then(|body| body.get(..VOICE_ROUTE_COOKIE_BYTES))
        .is_some_and(|candidate| expected.matches(candidate))
}

pub(crate) fn decode_authenticated_voice_packet(
    wire: &[u8],
    expected: VoiceRouteCookie,
) -> Result<VoicePacket, VoiceCodecError> {
    let body = wire
        .strip_prefix(VOICE_MEDIA_PREFIX)
        .ok_or(VoiceCodecError::MissingSignature)?;
    let cookie = body
        .get(..VOICE_ROUTE_COOKIE_BYTES)
        .ok_or(VoiceCodecError::InvalidRouteCookie)?;
    if !expected.matches(cookie) {
        return Err(VoiceCodecError::InvalidRouteCookie);
    }
    let mut packet = Vec::with_capacity(wire.len() - VOICE_ROUTE_COOKIE_BYTES);
    packet.extend_from_slice(VOICE_MEDIA_PREFIX);
    packet.extend_from_slice(&body[VOICE_ROUTE_COOKIE_BYTES..]);
    decode_voice_packet(&packet)
}

pub(crate) fn admit_voice_ingress(
    wire: &[u8],
    expected_cookie: VoiceRouteCookie,
    authenticated_source: ClientId,
    limiter: &mut VoiceIngressLimiter,
    now: Instant,
) -> Option<VoicePacket> {
    if !voice_datagram_has_cookie(wire, expected_cookie)
        || !limiter.allow(authenticated_source, now)
    {
        return None;
    }
    decode_authenticated_voice_packet(wire, expected_cookie).ok()
}

pub(crate) fn decode_voice_packet(wire: &[u8]) -> Result<VoicePacket, VoiceCodecError> {
    let body = wire
        .strip_prefix(VOICE_MEDIA_PREFIX)
        .ok_or(VoiceCodecError::MissingSignature)?;
    if body.len() < VOICE_PACKET_FIXED_HEADER {
        return Err(VoiceCodecError::Truncated);
    }
    let kind = body[0];
    let source_client_id = u32::from_le_bytes(
        body[1..5]
            .try_into()
            .map_err(|_| VoiceCodecError::Truncated)?,
    );
    let player_id = i32::from_le_bytes(
        body[5..9]
            .try_into()
            .map_err(|_| VoiceCodecError::Truncated)?,
    );
    let stream_epoch = u32::from_le_bytes(
        body[9..13]
            .try_into()
            .map_err(|_| VoiceCodecError::Truncated)?,
    );
    let sequence = u16::from_le_bytes(
        body[13..15]
            .try_into()
            .map_err(|_| VoiceCodecError::Truncated)?,
    );
    let recipient_count = usize::from(body[15]);
    let payload_len = usize::from(u16::from_le_bytes(
        body[16..18]
            .try_into()
            .map_err(|_| VoiceCodecError::Truncated)?,
    ));
    if recipient_count > MAX_VOICE_DIRECT_RECIPIENTS {
        return Err(VoiceCodecError::TooManyDirectRecipients(recipient_count));
    }
    if payload_len != VOICE_PAYLOAD_BYTES {
        return Err(VoiceCodecError::InvalidPayloadLength {
            actual: payload_len,
            expected: VOICE_PAYLOAD_BYTES,
        });
    }
    let recipients_len = recipient_count
        .checked_mul(std::mem::size_of::<ClientId>())
        .ok_or(VoiceCodecError::Truncated)?;
    let payload_start = VOICE_PACKET_FIXED_HEADER
        .checked_add(recipients_len)
        .ok_or(VoiceCodecError::Truncated)?;
    let packet_end = payload_start
        .checked_add(payload_len)
        .ok_or(VoiceCodecError::Truncated)?;
    if body.len() < packet_end {
        return Err(VoiceCodecError::Truncated);
    }
    if body.len() != packet_end {
        return Err(VoiceCodecError::TrailingBytes);
    }
    let direct_recipients = body[VOICE_PACKET_FIXED_HEADER..payload_start]
        .chunks_exact(std::mem::size_of::<ClientId>())
        .map(|bytes| {
            u32::from_le_bytes(
                bytes
                    .try_into()
                    .expect("voice recipient chunk has ClientId width"),
            )
        })
        .collect::<Vec<_>>();
    let frame = VoiceFrame {
        client_id: source_client_id,
        player_id,
        stream_epoch,
        sequence,
        payload: body[payload_start..packet_end].to_vec(),
    };
    match kind {
        VOICE_PACKET_DIRECT if direct_recipients.is_empty() => Ok(VoicePacket::Direct(frame)),
        VOICE_PACKET_RELAY_REQUEST => Ok(VoicePacket::RelayRequest {
            frame,
            direct_recipients,
        }),
        VOICE_PACKET_RELAYED if direct_recipients.is_empty() => Ok(VoicePacket::Relayed(frame)),
        VOICE_PACKET_DIRECT | VOICE_PACKET_RELAYED => Err(VoiceCodecError::TrailingBytes),
        kind => Err(VoiceCodecError::UnknownKind(kind)),
    }
}

/// Authenticates a client-originated packet against its established host
/// route. The sender-controlled client field is never trusted.
pub(crate) fn authenticate_host_ingress(
    ingress_client_id: ClientId,
    packet: VoicePacket,
) -> Option<(VoiceFrame, Vec<ClientId>)> {
    match packet {
        VoicePacket::Direct(frame) => Some((
            frame.with_authenticated_source(ingress_client_id),
            Vec::new(),
        )),
        VoicePacket::RelayRequest {
            frame,
            direct_recipients,
        } => Some((
            frame.with_authenticated_source(ingress_client_id),
            direct_recipients,
        )),
        VoicePacket::Relayed(_) => None,
    }
}

/// Enforces directionality at a client. Direct mesh packets are attributed to
/// their route; relayed packets retain the source already authenticated by the
/// host.
pub(crate) fn authenticate_client_ingress(
    ingress_peer_id: ClientId,
    ingress_is_host: bool,
    packet: VoicePacket,
) -> Option<VoiceFrame> {
    match (ingress_is_host, packet) {
        (true, VoicePacket::Relayed(frame)) => Some(frame),
        (false, VoicePacket::Direct(frame)) => {
            Some(frame.with_authenticated_source(ingress_peer_id))
        }
        _ => None,
    }
}

pub(crate) fn host_relay_selects(
    target_client_id: ClientId,
    source_client_id: ClientId,
    direct_recipients: &[ClientId],
) -> bool {
    target_client_id != source_client_id && !direct_recipients.contains(&target_client_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_frame_wire_round_trip_preserves_fixed_twenty_millisecond_metadata() {
        let frame = VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap();
        let authenticated = frame.with_authenticated_source(3);

        let wire = encode_voice_packet(&VoicePacket::Direct(authenticated.clone())).unwrap();
        let decoded = decode_voice_packet(&wire).unwrap();

        assert_eq!(decoded, VoicePacket::Direct(authenticated));
        assert_eq!(VOICE_FRAME_DURATION_MS, 20);
    }

    #[test]
    fn voice_wire_rejects_a_cookie_from_another_admitted_route() {
        let expected = VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]);
        let other_route = VoiceRouteCookie::from_bytes([0x22; VOICE_ROUTE_COOKIE_BYTES]);
        let frame = VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap();
        let wire =
            encode_authenticated_voice_packet(other_route, &VoicePacket::Direct(frame.clone()))
                .unwrap();

        assert!(!voice_datagram_has_cookie(&wire, expected));
        assert_eq!(
            decode_authenticated_voice_packet(&wire, expected),
            Err(VoiceCodecError::InvalidRouteCookie)
        );
        assert_eq!(
            decode_authenticated_voice_packet(&wire, other_route),
            Ok(VoicePacket::Direct(frame))
        );
    }

    #[test]
    fn route_media_authentication_requires_both_directional_cookies() {
        let local = VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]);
        let peer = VoiceRouteCookie::from_bytes([0x22; VOICE_ROUTE_COOKIE_BYTES]);
        let mut authentication = VoiceRouteAuthentication {
            local_receive_cookie: Some(local),
            peer_receive_cookie: None,
        };

        assert!(!authentication.is_negotiated());
        authentication
            .record_peer_capabilities(crate::PortCapabilities::supported().with_voice_cookie(peer));
        assert!(authentication.is_negotiated());
    }

    #[test]
    fn route_voice_negotiation_requires_capability_and_cookie_on_the_same_link() {
        let local = VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]);
        let peer = VoiceRouteCookie::from_bytes([0x22; VOICE_ROUTE_COOKIE_BYTES]);
        let mut authentication = VoiceRouteAuthentication {
            local_receive_cookie: Some(local),
            peer_receive_cookie: None,
        };

        authentication.record_peer_capabilities(crate::PortCapabilities::from_bits(
            crate::PortCapabilities::VOICE_CHAT,
        ));
        assert!(!authentication.is_negotiated());
        authentication
            .record_peer_capabilities(crate::PortCapabilities::default().with_voice_cookie(peer));
        assert!(!authentication.is_negotiated());
        authentication
            .record_peer_capabilities(crate::PortCapabilities::supported().with_voice_cookie(peer));
        assert!(authentication.is_negotiated());
    }

    #[test]
    fn host_ingress_overwrites_spoofed_source_and_preserves_relay_filter() {
        let spoofed = VoiceFrame {
            client_id: 99,
            player_id: 7,
            stream_epoch: 11,
            sequence: 29,
            payload: vec![0x5a; 164],
        };

        let (authenticated, direct) = authenticate_host_ingress(
            3,
            VoicePacket::RelayRequest {
                frame: spoofed,
                direct_recipients: vec![4, 5],
            },
        )
        .unwrap();

        assert_eq!(authenticated.client_id, 3);
        assert_eq!(direct, vec![4, 5]);
        assert!(authenticate_host_ingress(3, VoicePacket::Relayed(authenticated)).is_none());
    }

    #[test]
    fn client_ingress_accepts_direct_from_mesh_and_relay_only_from_host() {
        let spoofed = VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap();

        assert_eq!(
            authenticate_client_ingress(3, false, VoicePacket::Direct(spoofed.clone()))
                .unwrap()
                .client_id,
            3
        );
        assert!(
            authenticate_client_ingress(3, false, VoicePacket::Relayed(spoofed.clone())).is_none()
        );
        assert_eq!(
            authenticate_client_ingress(0, true, VoicePacket::Relayed(spoofed))
                .unwrap()
                .client_id,
            crate::BROADCAST_CLIENT_ID,
            "the host already authenticated the relayed source"
        );
    }

    #[test]
    fn host_relay_excludes_source_and_successful_direct_recipients() {
        assert!(!host_relay_selects(3, 3, &[4, 5]));
        assert!(!host_relay_selects(4, 3, &[4, 5]));
        assert!(host_relay_selects(6, 3, &[4, 5]));
    }

    #[test]
    fn application_sender_is_nonblocking_and_drops_at_its_bound() {
        let (tx, mut rx) = mpsc::channel(1);
        let sender = VoiceSender::new(tx);
        let first = VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap();
        let second = VoiceFrame::outbound(7, 11, 30, vec![0x5a; 164]).unwrap();

        assert!(!sender.is_available());
        assert_eq!(sender.try_send(first.clone()), Ok(()));
        assert_eq!(sender.try_send(second), Err(VoiceSendError::Full));
        assert_eq!(rx.try_recv(), Ok(first));
        drop(rx);
        assert_eq!(
            sender.try_send(VoiceFrame::outbound(7, 11, 31, vec![0x5a; 164]).unwrap()),
            Err(VoiceSendError::Closed)
        );
    }

    #[test]
    fn version_one_accepts_only_the_fixed_codec_payload_size() {
        assert!(VoiceFrame::outbound(7, 11, 29, vec![0; 163]).is_err());
        assert!(VoiceFrame::outbound(7, 11, 29, vec![0; 164]).is_ok());
        assert!(VoiceFrame::outbound(7, 11, 29, vec![0; 165]).is_err());
    }

    #[test]
    fn ingress_rate_limit_allows_fifty_frames_per_second_with_a_bounded_burst() {
        let start = std::time::Instant::now();
        let mut limiter = VoiceIngressLimiter::default();

        for _ in 0..75 {
            assert!(limiter.allow(7, start));
        }
        assert!(!limiter.allow(7, start));
        assert!(limiter.allow(8, start), "sources have independent budgets");
        assert!(limiter.allow(7, start + std::time::Duration::from_millis(21)));
        assert!(!limiter.allow(7, start + std::time::Duration::from_millis(21)));

        limiter.retain_sources([8]);
        assert!(
            limiter.allow(7, start),
            "forgotten peers receive a fresh budget"
        );
    }

    #[test]
    fn ingress_admission_checks_route_cookie_before_consuming_source_budget() {
        let start = Instant::now();
        let expected = VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]);
        let forged = VoiceRouteCookie::from_bytes([0x22; VOICE_ROUTE_COOKIE_BYTES]);
        let packet = VoicePacket::Direct(
            VoiceFrame::outbound(7, 11, 29, vec![0x5a; VOICE_PAYLOAD_BYTES]).unwrap(),
        );
        let valid_wire = encode_authenticated_voice_packet(expected, &packet).unwrap();
        let forged_wire = encode_authenticated_voice_packet(forged, &packet).unwrap();
        let mut limiter = VoiceIngressLimiter::default();

        for _ in 0..100 {
            assert_eq!(
                admit_voice_ingress(&forged_wire, expected, 7, &mut limiter, start),
                None
            );
        }
        for _ in 0..75 {
            assert_eq!(
                admit_voice_ingress(&valid_wire, expected, 7, &mut limiter, start),
                Some(packet.clone())
            );
        }
        assert_eq!(
            admit_voice_ingress(&valid_wire, expected, 7, &mut limiter, start),
            None,
            "the authenticated source cannot publish or relay above its burst"
        );
    }

    #[test]
    fn queued_control_or_reliable_input_preempts_media_work() {
        assert!(voice_media_may_run(false, false));
        assert!(!voice_media_may_run(true, false));
        assert!(!voice_media_may_run(false, true));
        assert!(!voice_media_may_run(true, true));
    }
}
