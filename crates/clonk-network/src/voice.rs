use ring::rand::{SecureRandom as _, SystemRandom};
use ring::{aead, agreement, hkdf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use std::{collections::BTreeMap, collections::BTreeSet};
use thiserror::Error;
use tokio::sync::mpsc;
use zeroize::{Zeroize as _, Zeroizing};

use crate::ClientId;

/// Duration represented by every encoded voice payload. Keeping this fixed
/// avoids trusting a sender-controlled sample count during decoder allocation.
pub const VOICE_FRAME_DURATION_MS: u16 = 20;
/// This wire version's exact independently decodable IMA ADPCM payload size. A
/// different codec or frame shape must use a new wire signature rather than
/// making this version sender-sized.
pub const VOICE_PAYLOAD_BYTES: usize = 164;
pub const MAX_VOICE_PAYLOAD_BYTES: usize = VOICE_PAYLOAD_BYTES;
/// How many peers one talking client will address directly before it stops and
/// leans on the host relay.
///
/// # Why the mesh, and not a relay (clonk-org/clonk-rs#425)
///
/// Send bandwidth in a mesh grows with the number of listeners, which is worth
/// measuring rather than assuming. Measured off the real encoder by
/// `the_voice_mesh_costs_one_sealed_datagram_per_listener_per_frame`: a sealed
/// direct datagram is 231 bytes, 259 with IPv4 and UDP headers, sent 50 times a
/// second — **103.6 kbit/s of the speaker's uplink per listener**.
///
/// Two things bound what that can reach:
///
/// - **Push-to-talk.** A peer that is not holding its key sends nothing at all,
///   so this is a cost per *speaker*, not per participant. The mesh's total is
///   set by how many people talk at once, which in practice is one or two.
/// - **This cap.** A speaker addresses at most 32 peers directly, so the worst
///   case the code permits is about **3.3 Mbit/s** of uplink while the key is
///   held. Beyond 32 the host relay carries the rest.
///
/// A relay that *replaced* direct fanout would move that load onto the host —
/// which pays it for every speaker at once — and add a hop of latency to every
/// listener, on a lane whose whole design is bounded and droppable. At the
/// session sizes this project targets that is a worse trade, so the mesh
/// stands and the relay stays what it already is: the path for peers direct
/// fanout cannot reach, and the valve for a saturated media queue.
///
/// Revisit if the codec, the frame rate or this cap change — the test above
/// fails rather than letting the figure drift silently.
pub(crate) const MAX_VOICE_DIRECT_RECIPIENTS: usize = 32;
pub(crate) const VOICE_ROUTE_COOKIE_BYTES: usize = 16;

/// The Rust-only media datagram family, shared by every version of the lane.
///
/// The transport diverts on *this*, not on the exact version, and does so
/// before the reliable-UDP packet header is inspected. That is what keeps media
/// bytes out of the reliable receive window: an unrecognized version is still
/// recognizably media, so it is dropped rather than read as a packet number.
/// Divert by family, admit by version.
pub(crate) const VOICE_MEDIA_FAMILY: &[u8; 4] = b"\x7fC4V";

/// This build's exact wire version, and the only one it will encode or open.
///
/// V2 seals everything after the route cookie. The version is bumped rather
/// than reused because a V1 build reads the cookie out of a V2 announcement
/// happily — it simply ignores the trailing agreement key — and would then
/// parse ciphertext as a cleartext packet. Bumping makes that mismatch fail at
/// the cheapest possible check instead of deep inside a length-driven parse.
pub(crate) const VOICE_MEDIA_PREFIX: &[u8; 5] = b"\x7fC4V2";

/// X25519 public value exchanged in the capability announcement.
pub(crate) const VOICE_KEY_AGREEMENT_PUBLIC_BYTES: usize = 32;
/// ChaCha20-Poly1305 key derived per route and per direction.
pub(crate) const VOICE_MEDIA_KEY_BYTES: usize = 32;
const VOICE_MEDIA_NONCE_BYTES: usize = aead::NONCE_LEN;
const VOICE_MEDIA_TAG_BYTES: usize = 16;

/// Domain separation for the media key schedule. The salt names the wire
/// version so a future frame shape cannot derive the same key from the same
/// exchange; the info string is qualified by the *receiving* route cookie,
/// which is what makes the two directions independent (see
/// [`derive_route_media_keys`]).
const VOICE_MEDIA_KEY_SALT: &[u8] = b"clonk-rs voice media v2";
const VOICE_MEDIA_KEY_INFO: &[u8] = b"media key";

const VOICE_PACKET_DIRECT: u8 = 0;
const VOICE_PACKET_RELAY_REQUEST: u8 = 1;
const VOICE_PACKET_RELAYED: u8 = 2;
const VOICE_PACKET_FIXED_HEADER: usize = 18;
pub(crate) const MAX_VOICE_WIRE_BYTES: usize = VOICE_MEDIA_PREFIX.len()
    + VOICE_ROUTE_COOKIE_BYTES
    + VOICE_MEDIA_NONCE_BYTES
    + VOICE_PACKET_FIXED_HEADER
    + MAX_VOICE_DIRECT_RECIPIENTS * std::mem::size_of::<ClientId>()
    + MAX_VOICE_PAYLOAD_BYTES
    + VOICE_MEDIA_TAG_BYTES;

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
/// reliable-UDP route. It names the route a datagram belongs to and lets the
/// transport drop a foreign one before any parsing; confidentiality is the
/// separate job of the sealed body ([`VoiceMediaCipher`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VoiceRouteCookie([u8; VOICE_ROUTE_COOKIE_BYTES]);

impl VoiceRouteCookie {
    pub(crate) fn generate() -> Option<Self> {
        let mut bytes = [0_u8; VOICE_ROUTE_COOKIE_BYTES];
        SystemRandom::new().fill(&mut bytes).ok()?;
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

/// One direction of one route's media protection: the cookie that names the
/// direction on the wire, and the key that seals everything behind it.
/// The codec is intentionally stateless: each call authenticates one datagram
/// and does not retain `(stream_epoch, sequence)` replay state. Once a frame is
/// decoded, duplicate/late suppression belongs to the application layer's
/// `VoiceActivityTracker`.
/// Deliberately not `Copy`: a `Copy` type cannot have a destructor, and every
/// implicit copy would be one more unerasable image of the key
/// (clonk-org/clonk-rs#470). `Clone` stays, so a route that genuinely needs a
/// second owned cipher can still make one -- and that clone erases itself too.
#[derive(Clone)]
pub(crate) struct VoiceMediaCipher {
    cookie: VoiceRouteCookie,
    key: [u8; VOICE_MEDIA_KEY_BYTES],
}

/// Erases the key when the cipher goes away, so a route's key material is not
/// left readable in memory the allocator is about to reuse.
///
/// This bounds honestly what it buys. A Rust move is a `memcpy` that does not
/// clear the source, so a key that has been moved may still have images the
/// destructor never sees; the guarantee is that no *live* owner leaks its copy
/// on the way out, not that the key was never duplicated. The reason it is
/// still worth having is that the long-lived owners -- the route tables in
/// both session loops -- are exactly the ones that would otherwise hold key
/// material for the whole match and release it unerased at teardown.
impl Drop for VoiceMediaCipher {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

// Derived `Debug` would print the key into any log that formats a route.
impl std::fmt::Debug for VoiceMediaCipher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceMediaCipher")
            .field("cookie", &self.cookie)
            .finish_non_exhaustive()
    }
}

impl VoiceMediaCipher {
    #[cfg(test)]
    pub(crate) const fn from_parts(
        cookie: VoiceRouteCookie,
        key: [u8; VOICE_MEDIA_KEY_BYTES],
    ) -> Self {
        Self { cookie, key }
    }

    pub(crate) const fn cookie(&self) -> VoiceRouteCookie {
        self.cookie
    }

    fn sealing_key(&self) -> Result<aead::LessSafeKey, VoiceCodecError> {
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &self.key)
            .map(aead::LessSafeKey::new)
            .map_err(|_| VoiceCodecError::MediaKeyUnusable)
    }

    /// Bytes bound to the ciphertext but sent in the clear. Covering the
    /// signature and the cookie means a sealed body cannot be lifted onto a
    /// different route or reinterpreted under a different wire version.
    fn associated_data(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(VOICE_MEDIA_PREFIX.len() + VOICE_ROUTE_COOKIE_BYTES);
        aad.extend_from_slice(VOICE_MEDIA_PREFIX);
        aad.extend_from_slice(&self.cookie.into_bytes());
        aad
    }
}

/// One route's ephemeral X25519 contribution.
///
/// The two halves are stored apart because they have different lifetimes: ring
/// consumes the secret to agree, so it exists for exactly one derivation, while
/// the public value keeps being announced. The host answers every announcement
/// it receives with its own, and that reply lands after agreement has already
/// consumed the secret.
fn generate_voice_key_agreement() -> Option<(
    agreement::EphemeralPrivateKey,
    [u8; VOICE_KEY_AGREEMENT_PUBLIC_BYTES],
)> {
    let secret =
        agreement::EphemeralPrivateKey::generate(&agreement::X25519, &SystemRandom::new()).ok()?;
    let public = secret.compute_public_key().ok()?.as_ref().try_into().ok()?;
    Some((secret, public))
}

/// Turns one completed exchange into the route's two independent directional
/// keys.
///
/// Both ends run this with mirrored arguments and must land on the same pair,
/// so the direction cannot be labelled by "host" or "client" — a mesh route
/// has neither. It is labelled by the *receiving* side's cookie instead: the
/// key I open with is qualified by my own cookie, the key I seal with by my
/// peer's, and my peer's mirrored call assigns the same two keys the opposite
/// way round.
fn derive_route_media_keys(
    local_secret: agreement::EphemeralPrivateKey,
    local_public: [u8; VOICE_KEY_AGREEMENT_PUBLIC_BYTES],
    local_cookie: VoiceRouteCookie,
    peer_cookie: VoiceRouteCookie,
    peer_public: [u8; VOICE_KEY_AGREEMENT_PUBLIC_BYTES],
) -> Option<(VoiceMediaCipher, VoiceMediaCipher)> {
    // An announcement echoed back verbatim would agree with itself, collapse
    // both directions onto one key, and let the echoer replay our own audio at
    // us. Neither half of our own announcement may come back as the peer's.
    if peer_public == local_public || peer_cookie == local_cookie {
        return None;
    }
    let peer_public_key = agreement::UnparsedPublicKey::new(&agreement::X25519, peer_public);
    agreement::agree_ephemeral(local_secret, &peer_public_key, |shared_secret| {
        let secret =
            hkdf::Salt::new(hkdf::HKDF_SHA256, VOICE_MEDIA_KEY_SALT).extract(shared_secret);
        expand_media_key(&secret, local_cookie)
            .zip(expand_media_key(&secret, peer_cookie))
            .map(|(receive, send)| {
                (
                    VoiceMediaCipher {
                        cookie: local_cookie,
                        key: *receive,
                    },
                    VoiceMediaCipher {
                        cookie: peer_cookie,
                        key: *send,
                    },
                )
            })
    })
    .ok()
    .flatten()
}

/// Expands one directional key.
///
/// The buffer is `Zeroizing` because it outlives its usefulness by exactly one
/// move: the caller copies it into the cipher, and without this the derivation
/// frame keeps a second readable image of the key for as long as that stack
/// depth goes unreused (clonk-org/clonk-rs#470).
fn expand_media_key(
    secret: &hkdf::Prk,
    receiver_cookie: VoiceRouteCookie,
) -> Option<Zeroizing<[u8; VOICE_MEDIA_KEY_BYTES]>> {
    let receiver_cookie = receiver_cookie.into_bytes();
    let mut key = Zeroizing::new([0_u8; VOICE_MEDIA_KEY_BYTES]);
    secret
        .expand(&[VOICE_MEDIA_KEY_INFO, &receiver_cookie], hkdf::HKDF_SHA256)
        .ok()?
        .fill(&mut *key)
        .ok()?;
    Some(key)
}

/// Directional media protection for one admitted transport route.
///
/// The route announces a locally generated cookie and an ephemeral X25519
/// public value on its own reliable control stream, and learns the peer's from
/// the matching announcement. Agreement is what buys confidentiality: the
/// control stream is cleartext, so a key *sent* over it would be readable by
/// the same observer the media is being hidden from, while neither side's
/// ephemeral secret ever leaves the process.
///
/// That bounds the guarantee honestly. A passive observer cannot recover
/// audio. An attacker who can *rewrite* the reliable control stream can
/// substitute its own announcement, which the whole cleartext LegacyClonk
/// control protocol already concedes — there is no peer identity here to bind
/// an exchange to, and inventing one would not survive a C++ peer.
#[derive(Default)]
pub(crate) struct VoiceRouteAuthentication {
    local_receive_cookie: Option<VoiceRouteCookie>,
    local_public_key: Option<[u8; VOICE_KEY_AGREEMENT_PUBLIC_BYTES]>,
    local_secret: Option<agreement::EphemeralPrivateKey>,
    receive: Option<VoiceMediaCipher>,
    send: Option<VoiceMediaCipher>,
}

// `EphemeralPrivateKey` is not `Debug`, and the derived form on the rest would
// print a route's bearer cookie into any log that formats session state.
impl std::fmt::Debug for VoiceRouteAuthentication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceRouteAuthentication")
            .field("negotiated", &self.is_negotiated())
            .finish_non_exhaustive()
    }
}

impl VoiceRouteAuthentication {
    pub(crate) fn new_udp() -> Self {
        VoiceRouteCookie::generate()
            .zip(generate_voice_key_agreement())
            .map(
                |(local_receive_cookie, (local_secret, local_public_key))| Self {
                    local_receive_cookie: Some(local_receive_cookie),
                    local_public_key: Some(local_public_key),
                    local_secret: Some(local_secret),
                    receive: None,
                    send: None,
                },
            )
            .unwrap_or_default()
    }

    pub(crate) fn announcement(&self) -> Option<crate::PortCapabilities> {
        self.local_receive_cookie
            .zip(self.local_public_key)
            .map(|(cookie, public)| {
                crate::PortCapabilities::supported()
                    .with_voice_cookie(cookie)
                    .with_voice_public_key(public)
            })
    }

    pub(crate) fn record_peer_capabilities(&mut self, capabilities: crate::PortCapabilities) {
        if self.is_negotiated() || !capabilities.has(crate::PortCapabilities::VOICE_CHAT) {
            return;
        }
        // A peer that announces voice without an agreement key is an older
        // port build. There is no cleartext fallback to drop to: leaving the
        // route unnegotiated is the whole point.
        let Some((((local_cookie, local_public), peer_cookie), peer_public)) = self
            .local_receive_cookie
            .zip(self.local_public_key)
            .zip(capabilities.voice_cookie())
            .zip(capabilities.voice_public_key())
        else {
            return;
        };
        // Taken only once the announcement is complete: agreement consumes the
        // secret, so a truncated one must not strand the route. Consuming it on
        // a *failed* agreement is deliberate — the route then stays closed.
        let derived = self.local_secret.take().and_then(|local_secret| {
            derive_route_media_keys(
                local_secret,
                local_public,
                local_cookie,
                peer_cookie,
                peer_public,
            )
        });
        if let Some((receive, send)) = derived {
            self.receive = Some(receive);
            self.send = Some(send);
        }
    }

    /// The cookie the transport matches inbound datagrams against before any
    /// parsing. Cleartext by necessity — it is the pre-parse drop check.
    pub(crate) fn receive_cookie(&self) -> Option<VoiceRouteCookie> {
        self.local_receive_cookie
    }

    pub(crate) fn receive_cipher(&self) -> Option<&VoiceMediaCipher> {
        self.receive.as_ref()
    }

    pub(crate) fn send_cipher(&self) -> Option<&VoiceMediaCipher> {
        self.send.as_ref()
    }

    pub(crate) const fn is_negotiated(&self) -> bool {
        self.receive.is_some() && self.send.is_some()
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
    #[error("voice payload has {actual} bytes; this wire version requires exactly {expected}")]
    InvalidPayloadLength { actual: usize, expected: usize },
    #[error("voice relay names {0} direct recipients; at most {MAX_VOICE_DIRECT_RECIPIENTS} are allowed")]
    TooManyDirectRecipients(usize),
    #[error("voice media packet signature is missing")]
    MissingSignature,
    #[error("voice media route cookie is missing or invalid")]
    InvalidRouteCookie,
    #[error("voice media key could not be used")]
    MediaKeyUnusable,
    #[error("voice media packet did not authenticate under this route's key")]
    MediaNotAuthentic,
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

/// Whether the transport must keep this datagram off the reliable path.
///
/// Deliberately matches the family rather than this build's version: a peer
/// speaking an older or newer media version is still speaking media, and the
/// one thing that must not happen is its bytes reaching `receive_at`, where the
/// four bytes after the signature would be observed as a reliable packet
/// number. The codec then refuses anything that is not exactly this version.
pub(crate) fn is_voice_media_datagram(wire: &[u8]) -> bool {
    wire.starts_with(VOICE_MEDIA_FAMILY)
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

/// Seals one packet for one route direction.
///
/// The nonce is drawn fresh per datagram rather than derived from the frame
/// header, because the header is not unique under a single key: `sequence` is
/// a `u16` that wraps after about 22 minutes of continuous speech without
/// advancing `stream_epoch`, and a host relays many sources' frames under one
/// key, so two speakers routinely share a header. A random 96-bit nonce keeps
/// the lane stateless — nothing to resynchronize, so nothing that would make a
/// dropped or reordered datagram anyone's problem.
pub(crate) fn encode_authenticated_voice_packet(
    cipher: &VoiceMediaCipher,
    packet: &VoicePacket,
) -> Result<Vec<u8>, VoiceCodecError> {
    let packet = encode_voice_packet(packet)?;
    let mut nonce = [0_u8; VOICE_MEDIA_NONCE_BYTES];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| VoiceCodecError::MediaKeyUnusable)?;
    let mut sealed = packet[VOICE_MEDIA_PREFIX.len()..].to_vec();
    cipher
        .sealing_key()?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(cipher.associated_data()),
            &mut sealed,
        )
        .map_err(|_| VoiceCodecError::MediaKeyUnusable)?;

    let mut wire = Vec::with_capacity(
        VOICE_MEDIA_PREFIX.len() + VOICE_ROUTE_COOKIE_BYTES + nonce.len() + sealed.len(),
    );
    wire.extend_from_slice(VOICE_MEDIA_PREFIX);
    wire.extend_from_slice(&cipher.cookie.into_bytes());
    wire.extend_from_slice(&nonce);
    wire.extend_from_slice(&sealed);
    Ok(wire)
}

pub(crate) fn voice_datagram_has_cookie(wire: &[u8], expected: VoiceRouteCookie) -> bool {
    wire.strip_prefix(VOICE_MEDIA_PREFIX)
        .and_then(|body| body.get(..VOICE_ROUTE_COOKIE_BYTES))
        .is_some_and(|candidate| expected.matches(candidate))
}

pub(crate) fn decode_authenticated_voice_packet(
    wire: &[u8],
    cipher: &VoiceMediaCipher,
) -> Result<VoicePacket, VoiceCodecError> {
    let body = wire
        .strip_prefix(VOICE_MEDIA_PREFIX)
        .ok_or(VoiceCodecError::MissingSignature)?;
    let cookie = body
        .get(..VOICE_ROUTE_COOKIE_BYTES)
        .ok_or(VoiceCodecError::InvalidRouteCookie)?;
    if !cipher.cookie.matches(cookie) {
        return Err(VoiceCodecError::InvalidRouteCookie);
    }
    let nonce: [u8; VOICE_MEDIA_NONCE_BYTES] = body
        .get(VOICE_ROUTE_COOKIE_BYTES..VOICE_ROUTE_COOKIE_BYTES + VOICE_MEDIA_NONCE_BYTES)
        .and_then(|nonce| nonce.try_into().ok())
        .ok_or(VoiceCodecError::Truncated)?;
    let mut sealed = body
        .get(VOICE_ROUTE_COOKIE_BYTES + VOICE_MEDIA_NONCE_BYTES..)
        .filter(|sealed| sealed.len() > VOICE_MEDIA_TAG_BYTES)
        .ok_or(VoiceCodecError::Truncated)?
        .to_vec();
    let opened = cipher
        .sealing_key()?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(cipher.associated_data()),
            &mut sealed,
        )
        .map_err(|_| VoiceCodecError::MediaNotAuthentic)?;

    let mut packet = Vec::with_capacity(VOICE_MEDIA_PREFIX.len() + opened.len());
    packet.extend_from_slice(VOICE_MEDIA_PREFIX);
    packet.extend_from_slice(opened);
    decode_voice_packet(&packet)
}

pub(crate) fn admit_voice_ingress(
    wire: &[u8],
    cipher: &VoiceMediaCipher,
    authenticated_source: ClientId,
    limiter: &mut VoiceIngressLimiter,
    now: Instant,
) -> Option<VoicePacket> {
    // The cookie travels in the clear, so matching it proves only that the
    // sender read one earlier datagram. Spend the source's budget after the
    // seal has proved the datagram really came from that route: otherwise an
    // on-path forger drains the bucket and silences the peer it is imitating.
    if !voice_datagram_has_cookie(wire, cipher.cookie()) {
        return None;
    }
    let packet = decode_authenticated_voice_packet(wire, cipher).ok()?;
    limiter.allow(authenticated_source, now).then_some(packet)
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

    /// What the mesh actually costs, measured off the real encoder rather than
    /// added up from header constants (clonk-org/clonk-rs#425).
    ///
    /// A talking peer sends one sealed datagram per listener per 20 ms frame,
    /// so its uplink carries `listeners x 50 x datagram` for as long as the
    /// key is held. Pinning it here means a codec, header or sealing change
    /// cannot move that number without saying so.
    #[test]
    fn the_voice_mesh_costs_one_sealed_datagram_per_listener_per_frame() {
        /// IPv4 20 + UDP 8. The lane is UDP, and this is the smaller of the
        /// two IP header sizes, so the figure is a floor rather than a
        /// flattering estimate.
        const IP_AND_UDP_HEADER_BYTES: usize = 28;
        const FRAMES_PER_SECOND: usize = 1000 / VOICE_FRAME_DURATION_MS as usize;

        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );
        let packet = VoicePacket::Direct(
            VoiceFrame::outbound(7, 11, 29, vec![0x5a; VOICE_PAYLOAD_BYTES]).unwrap(),
        );
        let wire = encode_authenticated_voice_packet(&cipher, &packet).unwrap();
        let datagram = wire.len() + IP_AND_UDP_HEADER_BYTES;

        let per_listener_bits = datagram * FRAMES_PER_SECOND * 8;
        assert!(
            (95_000..115_000).contains(&per_listener_bits),
            "one listener costs {per_listener_bits} bit/s of a speaker's uplink"
        );

        // The worst case this code permits: one peer talking with the direct
        // fanout saturated. Beyond this the sender stops adding direct
        // recipients and leans on the host relay.
        let saturated = per_listener_bits * MAX_VOICE_DIRECT_RECIPIENTS;
        assert!(
            saturated < 4_000_000,
            "a saturated direct fanout must stay inside a few Mbit/s: {saturated} bit/s"
        );
    }

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
    fn sealed_media_round_trips_and_leaves_no_plaintext_on_the_wire() {
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );
        let payload = vec![0x5a; VOICE_PAYLOAD_BYTES];
        let packet = VoicePacket::Direct(VoiceFrame::outbound(7, 11, 29, payload.clone()).unwrap());

        let wire = encode_authenticated_voice_packet(&cipher, &packet).unwrap();

        assert!(
            !wire.windows(payload.len()).any(|window| window == payload),
            "an on-path observer must not read the encoded audio off the wire"
        );
        assert_eq!(
            decode_authenticated_voice_packet(&wire, &cipher),
            Ok(packet)
        );
    }

    #[test]
    fn sealed_media_leaves_replay_suppression_to_the_application_tracker() {
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );
        let packet = VoicePacket::Direct(
            VoiceFrame::outbound(7, 11, 29, vec![0x5a; VOICE_PAYLOAD_BYTES]).unwrap(),
        );
        let wire = encode_authenticated_voice_packet(&cipher, &packet).unwrap();

        assert_eq!(
            decode_authenticated_voice_packet(&wire, &cipher),
            Ok(packet.clone())
        );
        assert_eq!(
            decode_authenticated_voice_packet(&wire, &cipher),
            Ok(packet),
            "the network seal authenticates both deliveries; the app tracker owns replay suppression",
        );
    }

    /// Drives both halves of one route's announcement exchange the way the
    /// session loops do: each side announces on its own reliable stream, and
    /// each records what the other announced.
    fn negotiated_route_pair() -> (VoiceRouteAuthentication, VoiceRouteAuthentication) {
        let mut local = VoiceRouteAuthentication::new_udp();
        let mut peer = VoiceRouteAuthentication::new_udp();
        let (local_announcement, peer_announcement) = (
            local.announcement().expect("local route announces"),
            peer.announcement().expect("peer route announces"),
        );

        local.record_peer_capabilities(peer_announcement);
        peer.record_peer_capabilities(local_announcement);
        (local, peer)
    }

    #[test]
    fn an_older_media_version_is_still_diverted_off_the_reliable_path() {
        // The transport routes on the family, so a peer speaking V1 is kept
        // away from `receive_at` — where "C4V1" would be observed as a reliable
        // packet number and poison the receive window — and is then refused by
        // the codec for not being this version.
        let mut v1 = Vec::from(*b"\x7fC4V1");
        v1.extend_from_slice(&[0x11; VOICE_ROUTE_COOKIE_BYTES]);
        v1.extend_from_slice(&[0x5a; VOICE_PACKET_FIXED_HEADER + VOICE_PAYLOAD_BYTES]);
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );

        assert!(is_voice_media_datagram(&v1), "still recognized as media");
        assert!(!voice_datagram_has_cookie(&v1, cipher.cookie()));
        assert_eq!(
            decode_authenticated_voice_packet(&v1, &cipher),
            Err(VoiceCodecError::MissingSignature),
            "but never opened as this version"
        );
        assert!(v1.len() <= MAX_VOICE_WIRE_BYTES, "and still inside the cap");
    }

    #[test]
    fn the_largest_sealed_packet_still_fits_the_transport_datagram_cap() {
        // The transport drops anything over this cap before parsing it, so a
        // cap that forgot the nonce or the tag would silently discard exactly
        // the fullest relay requests rather than fail loudly.
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );
        let largest = VoicePacket::RelayRequest {
            frame: VoiceFrame::outbound(7, 11, 29, vec![0x5a; VOICE_PAYLOAD_BYTES]).unwrap(),
            direct_recipients: (0..MAX_VOICE_DIRECT_RECIPIENTS as ClientId).collect(),
        };

        let wire = encode_authenticated_voice_packet(&cipher, &largest).unwrap();

        assert_eq!(wire.len(), MAX_VOICE_WIRE_BYTES);
        assert_eq!(
            decode_authenticated_voice_packet(&wire, &cipher),
            Ok(largest)
        );
    }

    #[test]
    fn sealing_leaves_the_lane_droppable_and_reorderable() {
        // The lane stays off the lockstep path precisely because losing or
        // reordering a datagram costs nothing. A seal carrying per-connection
        // state — a rolling nonce, a replay window, a rekey schedule — would
        // quietly turn a dropped frame into a stalled stream.
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );
        let stream = (0..8)
            .map(|sequence| {
                VoicePacket::Direct(
                    VoiceFrame::outbound(7, 11, sequence, vec![0x5a; VOICE_PAYLOAD_BYTES]).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let wire = stream
            .iter()
            .map(|packet| encode_authenticated_voice_packet(&cipher, packet).unwrap())
            .collect::<Vec<_>>();

        // Delivered backwards, with every third datagram lost.
        let delivered = wire
            .iter()
            .enumerate()
            .rev()
            .filter(|(index, _)| index % 3 != 0)
            .map(|(index, wire)| {
                (
                    index,
                    decode_authenticated_voice_packet(wire, &cipher).unwrap(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(delivered.len(), 5);
        for (index, packet) in delivered {
            assert_eq!(packet, stream[index], "datagram {index} decoded alone");
        }
    }

    #[test]
    fn a_tampered_sealed_packet_never_reaches_the_decoder() {
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );
        let packet = VoicePacket::Direct(VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap());
        let wire = encode_authenticated_voice_packet(&cipher, &packet).unwrap();

        // Every byte the cookie does not already cover is under the tag.
        for index in VOICE_MEDIA_PREFIX.len() + VOICE_ROUTE_COOKIE_BYTES..wire.len() {
            let mut tampered = wire.clone();
            tampered[index] ^= 0x01;
            assert_eq!(
                decode_authenticated_voice_packet(&tampered, &cipher),
                Err(VoiceCodecError::MediaNotAuthentic),
                "byte {index} is not covered by the seal"
            );
        }
        assert_eq!(
            decode_authenticated_voice_packet(&wire[..wire.len() - 1], &cipher),
            Err(VoiceCodecError::MediaNotAuthentic),
            "a truncated seal must not open"
        );
    }

    #[test]
    fn two_seals_of_one_packet_never_repeat_a_nonce() {
        // A repeated nonce under one key is what breaks ChaCha20-Poly1305, and
        // the frame header cannot supply a unique one: `sequence` is a `u16`
        // that wraps, and a host seals many speakers' frames under one key.
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0x42; VOICE_MEDIA_KEY_BYTES],
        );
        let packet = VoicePacket::Direct(VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap());
        let nonce_range = VOICE_MEDIA_PREFIX.len() + VOICE_ROUTE_COOKIE_BYTES
            ..VOICE_MEDIA_PREFIX.len() + VOICE_ROUTE_COOKIE_BYTES + VOICE_MEDIA_NONCE_BYTES;

        let nonces = (0..32)
            .map(|_| {
                encode_authenticated_voice_packet(&cipher, &packet).unwrap()[nonce_range.clone()]
                    .to_vec()
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            nonces.len(),
            32,
            "the same frame must never seal identically"
        );
    }

    #[test]
    fn voice_wire_rejects_a_cookie_from_another_admitted_route() {
        let (local, _) = negotiated_route_pair();
        let (other_route, _) = negotiated_route_pair();
        let expected = local.receive_cipher().unwrap();
        let other_route = other_route.receive_cipher().unwrap();
        let frame = VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap();
        let wire =
            encode_authenticated_voice_packet(other_route, &VoicePacket::Direct(frame.clone()))
                .unwrap();

        assert!(!voice_datagram_has_cookie(&wire, expected.cookie()));
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
    fn route_media_keys_agree_across_the_link_and_differ_per_direction() {
        let (local, peer) = negotiated_route_pair();

        assert!(local.is_negotiated() && peer.is_negotiated());
        let frame = VoiceFrame::outbound(7, 11, 29, vec![0x5a; VOICE_PAYLOAD_BYTES]).unwrap();
        let packet = VoicePacket::Direct(frame);
        // What one side seals to send, the other opens on receive.
        let outbound =
            encode_authenticated_voice_packet(local.send_cipher().unwrap(), &packet).unwrap();
        assert_eq!(
            decode_authenticated_voice_packet(&outbound, peer.receive_cipher().unwrap()),
            Ok(packet.clone())
        );
        // The reverse direction is a different key, so our own sealed frame
        // fed back to us is not ours to open.
        assert_eq!(
            decode_authenticated_voice_packet(&outbound, local.receive_cipher().unwrap()),
            Err(VoiceCodecError::InvalidRouteCookie)
        );
        let reflected = encode_authenticated_voice_packet(
            &VoiceMediaCipher::from_parts(
                local.receive_cipher().unwrap().cookie(),
                peer.receive_cipher().unwrap().key,
            ),
            &packet,
        )
        .unwrap();
        assert_eq!(
            decode_authenticated_voice_packet(&reflected, local.receive_cipher().unwrap()),
            Err(VoiceCodecError::MediaNotAuthentic),
            "relabelling a send-direction frame with the receive cookie must not open it"
        );
    }

    #[test]
    fn route_voice_negotiation_requires_capability_cookie_and_agreement_key_together() {
        let mut authentication = VoiceRouteAuthentication::new_udp();
        let peer = VoiceRouteAuthentication::new_udp();
        let announcement = peer.announcement().unwrap();
        let (peer_cookie, peer_public) = (
            announcement.voice_cookie().unwrap(),
            announcement.voice_public_key().unwrap(),
        );

        authentication.record_peer_capabilities(crate::PortCapabilities::from_bits(
            crate::PortCapabilities::VOICE_CHAT,
        ));
        assert!(!authentication.is_negotiated());
        authentication.record_peer_capabilities(
            crate::PortCapabilities::default()
                .with_voice_cookie(peer_cookie)
                .with_voice_public_key(peer_public),
        );
        assert!(!authentication.is_negotiated());
        // A peer that announces voice and a cookie but no agreement key is an
        // older port build. There is no cleartext fallback to drop back to.
        authentication.record_peer_capabilities(
            crate::PortCapabilities::supported().with_voice_cookie(peer_cookie),
        );
        assert!(
            !authentication.is_negotiated(),
            "a peer that cannot agree a key gets no media lane at all"
        );
        authentication.record_peer_capabilities(announcement);
        assert!(authentication.is_negotiated());
    }

    #[test]
    fn an_echoed_announcement_cannot_reflect_our_own_audio_back_at_us() {
        let mut authentication = VoiceRouteAuthentication::new_udp();
        let echoed = authentication.announcement().unwrap();

        authentication.record_peer_capabilities(echoed);

        assert!(
            !authentication.is_negotiated(),
            "agreeing with our own announcement would collapse both directions onto one key"
        );
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
    fn the_wire_version_accepts_only_the_fixed_codec_payload_size() {
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
    fn only_an_authentic_datagram_consumes_the_source_budget() {
        let start = Instant::now();
        let (local, _) = negotiated_route_pair();
        let (other_route, _) = negotiated_route_pair();
        let expected = local.receive_cipher().unwrap();
        let forged = other_route.receive_cipher().unwrap();
        // The cookie is public, so this is what an on-path forger can actually
        // build: the right route label over a body it cannot seal.
        let unsealable =
            VoiceMediaCipher::from_parts(expected.cookie(), [0xaa; VOICE_MEDIA_KEY_BYTES]);
        let packet = VoicePacket::Direct(
            VoiceFrame::outbound(7, 11, 29, vec![0x5a; VOICE_PAYLOAD_BYTES]).unwrap(),
        );
        let valid_wire = encode_authenticated_voice_packet(expected, &packet).unwrap();
        let forged_wire = encode_authenticated_voice_packet(forged, &packet).unwrap();
        let unsealable_wire = encode_authenticated_voice_packet(&unsealable, &packet).unwrap();
        let mut limiter = VoiceIngressLimiter::default();

        for _ in 0..100 {
            assert_eq!(
                admit_voice_ingress(&forged_wire, expected, 7, &mut limiter, start),
                None
            );
            assert_eq!(
                admit_voice_ingress(&unsealable_wire, expected, 7, &mut limiter, start),
                None,
                "a forger who copies the public cookie must not drain the real peer's budget"
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

    /// clonk-org/clonk-rs#470: a route's media key must not outlive the cipher
    /// that held it.
    ///
    /// The destructor is run by hand so the key's storage can be read back
    /// afterwards. `slot` still owns that storage — this is not a read of
    /// freed memory — so what it observes is exactly the residue a real drop
    /// leaves behind in the stack slot or heap block the allocator is then
    /// free to hand to someone else.
    #[test]
    fn dropping_a_media_cipher_erases_its_key() {
        let cipher = VoiceMediaCipher::from_parts(
            VoiceRouteCookie::from_bytes([0x11; VOICE_ROUTE_COOKIE_BYTES]),
            [0xa7; VOICE_MEDIA_KEY_BYTES],
        );
        let mut slot = std::mem::ManuallyDrop::new(cipher);
        let key_storage = std::ptr::addr_of!(slot.key);
        // SAFETY: `slot` is a live local this test owns, and running its
        // destructor exactly once is what `ManuallyDrop` exists to allow.
        unsafe { std::mem::ManuallyDrop::drop(&mut slot) };
        // SAFETY: the storage above is still owned and still initialised as
        // far as `[u8; N]` is concerned — every byte was written, and a drop
        // that zeroes them writes bytes rather than invalidating them. The
        // read is volatile so it cannot be optimised away as dead.
        let residue = unsafe { std::ptr::read_volatile(key_storage) };
        assert_eq!(
            residue, [0; VOICE_MEDIA_KEY_BYTES],
            "the media key survived the cipher that held it"
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
