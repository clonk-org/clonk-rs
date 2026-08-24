//! Capability negotiation with a peer that may or may not be this port.
//!
//! The port wants protocol improvements a stock C++ peer cannot parse — a
//! separate logical channel for control, redundancy carried in-band, elided
//! empty control. None of those may break the ability to join an old
//! LegacyClonk server, so each is announced and used only when *both* ends
//! support it.
//!
//! The announcement rides a PID outside every ID C++ dispatches, but "outside
//! C++'s range" is not "invisible to C++". `C4Network2IO::HandlePacket`
//! unpacks before dispatching, and `C4IDPacket::CompileFunc` `excCorrupt`s on
//! an ID with no `FnUnpack` (oracle-src-pinned src/C4Packet2.cpp:210-217); the
//! handler catches that, logs it, and in a release build closes the connection
//! (src/C4Network2IO.cpp:820-834). A stock peer therefore drops the link on any
//! port-only ID it receives — it never ignores one. The exemption is narrow:
//! such an ID may be sent only to a peer already known to be this port, or
//! immediately before a connection would close anyway (the host restart notices
//! in [`crate::host_restart`]). It is not a licence for new port-only IDs on a
//! live session.
//!
//! The positive peer marker is carried in a fixed extension after the known
//! `PID_Conn`/`PID_ConnRe` fields. The pinned C++ compiler reads those fields
//! without requiring EOF (`src/C4Packet2.cpp:145-149;
//! src/StdCompiler.cpp:228-244`), so stock peers ignore the marker while this
//! port can know the peer before sending any port-only ID. A missing marker
//! means stock C++ and keeps the C++-compatible path; capabilities are never
//! assumed from a version number or from the absence of a parse error.

use std::collections::BTreeMap;

/// Packet ID for the capability announcement.
///
/// Chosen above every ID C++ dispatches (its highest is `PID_ExecSyncCtrl`,
/// 0x43, with the resource and league IDs below that) so it can never collide
/// with a packet a C++ peer would otherwise act on.
pub const PID_PORT_CAPABILITIES: u8 = 0x70;

/// Version of the capability vocabulary itself.
///
/// Bumped only when the *meaning* of a bit changes. New capabilities take a new
/// bit and leave this alone, so an older port build simply reports fewer.
pub const PORT_CAPABILITY_VERSION: u16 = 1;

/// What a peer running this port can do beyond the C++ protocol.
///
/// A bitset rather than a version number on purpose: capabilities land
/// independently, and two builds of the port may support overlapping but
/// non-identical sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortCapabilities {
    bits: u32,
    voice_cookie: Option<crate::voice::VoiceRouteCookie>,
    voice_public_key: Option<[u8; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES]>,
}

impl PortCapabilities {
    /// Control carried on its own logical channel, so bulk resource fragments
    /// can never sit ahead of it in an ordered stream.
    pub const CONTROL_CHANNEL: u32 = 1 << 0;
    /// Redundancy carried inside the control packet as the last few ticks,
    /// rather than as duplicate datagrams.
    pub const INBAND_REDUNDANCY: u32 = 1 << 1;
    /// A tick with no input may be omitted rather than sent empty.
    pub const ELIDED_EMPTY_CONTROL: u32 = 1 << 2;
    /// Bit 3 announced the earlier *cleartext* voice media lane. Retired, never
    /// reused, and never announced — see [`Self::VOICE_CHAT`].
    pub const RETIRED_CLEARTEXT_VOICE_CHAT: u32 = 1 << 3;
    /// Host-routed control waits identify whether this client or a different
    /// participant held up the aggregate tick.
    pub const CONTROL_WAIT_ATTRIBUTION: u32 = 1 << 4;
    /// Best-effort voice media carried outside reliable packet accounting and
    /// sealed under the route's own key exchange.
    ///
    /// This takes a fresh bit rather than reusing bit 3 because the bit is what
    /// an older build acts on. That build reads bit 3 as "this peer accepts my
    /// voice", marks the route negotiated on the cookie alone, and opens its
    /// microphone — putting *its* audio on the wire in the clear, for a lane
    /// this build can no longer even receive. Retiring the bit means such a
    /// peer sees no voice offer at all, which is the only honest answer: the
    /// two builds cannot carry voice between them.
    pub const VOICE_CHAT: u32 = 1 << 5;
    /// The host can replace the round bootstrap in-place, fencing retained
    /// client ingress with a nonce until each client installs fresh JoinData.
    ///
    /// The original lobby-restart packet was shipped without negotiation.
    /// This second-generation contract is explicit because a peer that does
    /// not understand the marker would keep sending old-round traffic on a
    /// connection the host has already repurposed.
    pub const ROUND_RESTART_V2: u32 = 1 << 6;

    /// Everything this build knows how to do.
    pub fn supported() -> Self {
        Self::from_bits(Self::ROUND_RESTART_V2 | Self::VOICE_CHAT | Self::CONTROL_WAIT_ATTRIBUTION)
    }

    /// Capabilities every port route can advertise without a voice media
    /// cookie or key exchange.
    pub fn supported_without_voice() -> Self {
        Self::from_bits(Self::supported().bits() & !Self::VOICE_CHAT)
    }

    pub fn from_bits(bits: u32) -> Self {
        Self {
            bits,
            voice_cookie: None,
            voice_public_key: None,
        }
    }

    pub fn bits(self) -> u32 {
        self.bits
    }

    pub fn has(self, capability: u32) -> bool {
        self.bits & capability == capability
    }

    /// What both ends can do — the only set that may actually be used.
    pub fn agreed_with(self, peer: Self) -> Self {
        Self::from_bits(self.bits & peer.bits)
    }

    pub(crate) fn with_voice_cookie(
        mut self,
        voice_cookie: crate::voice::VoiceRouteCookie,
    ) -> Self {
        self.voice_cookie = Some(voice_cookie);
        self
    }

    pub(crate) fn voice_cookie(self) -> Option<crate::voice::VoiceRouteCookie> {
        self.voice_cookie
    }

    pub(crate) fn with_voice_public_key(
        mut self,
        voice_public_key: [u8; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES],
    ) -> Self {
        self.voice_public_key = Some(voice_public_key);
        self
    }

    pub(crate) fn voice_public_key(
        self,
    ) -> Option<[u8; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES]> {
        self.voice_public_key
    }
}

/// Where the media lane's per-route key exchange rides. It follows the cookie
/// so the version-and-bit prefix every earlier build parses stays byte-exact.
const VOICE_COOKIE_OFFSET: usize = 7;
const VOICE_PUBLIC_KEY_OFFSET: usize = VOICE_COOKIE_OFFSET + crate::voice::VOICE_ROUTE_COOKIE_BYTES;

/// Encodes the announcement. Body is the vocabulary version then the bitset,
/// both little-endian, so an older peer can read the version and stop.
pub fn encode_port_capabilities(capabilities: PortCapabilities) -> Vec<u8> {
    let mut wire = vec![PID_PORT_CAPABILITIES];
    wire.extend_from_slice(&PORT_CAPABILITY_VERSION.to_le_bytes());
    wire.extend_from_slice(&capabilities.bits().to_le_bytes());
    if let Some(cookie) = capabilities.voice_cookie() {
        wire.extend_from_slice(&cookie.into_bytes());
    }
    // Only ever alongside the cookie: the media lane needs both halves, and a
    // key without the cookie that labels its direction cannot derive anything.
    if let Some((_, public_key)) = capabilities
        .voice_cookie()
        .zip(capabilities.voice_public_key())
    {
        wire.extend_from_slice(&public_key);
    }
    wire
}

/// Decodes an announcement, or `None` if this is not one or is truncated.
///
/// A future vocabulary version is read for its bits anyway: unknown bits are
/// capabilities this build does not have, and `agreed_with` masks them off.
pub fn decode_port_capabilities(wire: &[u8]) -> Option<PortCapabilities> {
    if wire.first().copied()? != PID_PORT_CAPABILITIES || wire.len() < VOICE_COOKIE_OFFSET {
        return None;
    }
    let bits = u32::from_le_bytes(wire.get(3..VOICE_COOKIE_OFFSET)?.try_into().ok()?);
    let voice_cookie = wire
        .get(VOICE_COOKIE_OFFSET..VOICE_PUBLIC_KEY_OFFSET)
        .and_then(|bytes| bytes.try_into().ok())
        .map(crate::voice::VoiceRouteCookie::from_bytes);
    let voice_public_key = wire
        .get(
            VOICE_PUBLIC_KEY_OFFSET
                ..VOICE_PUBLIC_KEY_OFFSET + crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES,
        )
        .and_then(|bytes| bytes.try_into().ok());
    Some(PortCapabilities {
        bits,
        voice_cookie,
        voice_public_key,
    })
}

/// What each connected peer announced.
///
/// A peer with no entry is assumed to be stock C++, which is the only safe
/// default: silence is exactly what a C++ peer produces.
#[derive(Debug, Default)]
pub struct PeerCapabilityRegistry {
    peers: BTreeMap<i32, PortCapabilities>,
}

impl PeerCapabilityRegistry {
    pub fn record(&mut self, client_id: i32, capabilities: PortCapabilities) {
        self.peers
            .insert(client_id, PortCapabilities::from_bits(capabilities.bits()));
    }

    pub fn forget(&mut self, client_id: i32) {
        self.peers.remove(&client_id);
    }

    pub fn clear(&mut self, client_id: i32, capability: u32) {
        if let Some(capabilities) = self.peers.get_mut(&client_id) {
            capabilities.bits &= !capability;
        }
    }

    pub fn of(&self, client_id: i32) -> PortCapabilities {
        self.peers.get(&client_id).copied().unwrap_or_default()
    }

    /// Whether a capability may be used on the link to one peer. Per-connection
    /// scope: transport-level choices like the control channel are settled link
    /// by link, so a mixed session is fine.
    pub fn peer_supports(&self, client_id: i32, capability: u32) -> bool {
        PortCapabilities::supported()
            .agreed_with(self.of(client_id))
            .has(capability)
    }

    /// Whether a capability may be used for the *session*.
    ///
    /// Session scope is stricter than per-connection because the host packs one
    /// merged control packet for everyone: anything that changes that format is
    /// legal only when every participant can read it. One stock C++ client is
    /// enough to hold the whole session on the compatible path.
    pub fn session_supports<I>(&self, participants: I, capability: u32) -> bool
    where
        I: IntoIterator<Item = i32>,
    {
        let mut any = false;
        for client_id in participants {
            any = true;
            if !self.peer_supports(client_id, capability) {
                return false;
            }
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_announcement_round_trips() {
        let announced = PortCapabilities::from_bits(
            PortCapabilities::CONTROL_CHANNEL | PortCapabilities::INBAND_REDUNDANCY,
        );
        let wire = encode_port_capabilities(announced);

        assert_eq!(wire[0], PID_PORT_CAPABILITIES);
        assert_eq!(decode_port_capabilities(&wire), Some(announced));
    }

    #[test]
    fn a_packet_that_is_not_an_announcement_is_not_decoded_as_one() {
        assert_eq!(decode_port_capabilities(&[]), None);
        assert_eq!(decode_port_capabilities(&[0x40, 1, 2, 3, 4, 5, 6]), None);
        // Truncated: a short body must not be read as an empty capability set,
        // which would look like a successful "supports nothing" announcement.
        assert_eq!(
            decode_port_capabilities(&[PID_PORT_CAPABILITIES, 1, 0, 0]),
            None
        );
    }

    /// What a *previously released* build concludes from this build's
    /// announcement (clonk-org/clonk-rs#471).
    ///
    /// Every other test in the tree pairs this revision with itself. The two
    /// defects that reached a green gate set in clonk-org/clonk-rs#465 were both
    /// about the older peer's behaviour, which nothing could express: bumping
    /// the media wire signature did not stop a released build transmitting,
    /// because that build acts on the capability *bit*, not on the datagram
    /// signature.
    ///
    /// So the property under test is the one that matters to a peer we cannot
    /// build here: a released decoder reads the bitset out of our announcement
    /// and acts on each bit it recognises. Reading it is trivial — the field is
    /// at a fixed offset and every release has had it there — which is what
    /// makes the cheap harness possible without compiling a second revision.
    ///
    /// A retired bit must therefore read as *clear* to that build, for every
    /// announcement this build can produce. Bit 3 is the concrete case: an
    /// older build reads it as "this peer accepts my voice", marks the route
    /// negotiated on the cookie alone, and opens its microphone onto a
    /// cleartext lane this build can no longer receive.
    #[test]
    fn a_released_build_reads_no_retired_capability_from_this_builds_announcement() {
        /// The one field a released build is guaranteed to read the same way:
        /// `bits` as little-endian at offset 3. Deliberately not
        /// `decode_port_capabilities` — the point is to read the wire the way
        /// another build would, not the way this one does.
        fn bits_as_a_released_build_reads_them(wire: &[u8]) -> u32 {
            assert_eq!(wire[0], PID_PORT_CAPABILITIES, "not an announcement");
            u32::from_le_bytes(wire[3..7].try_into().expect("announcement carries bits"))
        }

        // Every retired bit, and what a released build would do with it.
        const RETIRED: [(u32, &str); 1] = [(
            PortCapabilities::RETIRED_CLEARTEXT_VOICE_CHAT,
            "opens its microphone onto a cleartext lane this build cannot receive",
        )];

        // Both shapes this build announces: bare, and with a voice route
        // offered. The cookie and key ride *after* the bitset, so neither can
        // change what a released build reads out of it — assert that rather
        // than assume it.
        let bare = PortCapabilities::supported();
        let announcements = [
            ("bare", encode_port_capabilities(bare)),
            (
                "with a voice route",
                encode_port_capabilities(
                    bare.with_voice_cookie(crate::voice::VoiceRouteCookie::from_bytes(
                        [0x5a; crate::voice::VOICE_ROUTE_COOKIE_BYTES],
                    ))
                    .with_voice_public_key([0x27; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES]),
                ),
            ),
        ];

        for (label, wire) in &announcements {
            let seen = bits_as_a_released_build_reads_them(wire);
            for (bit, consequence) in RETIRED {
                assert_eq!(
                    seen & bit,
                    0,
                    "the {label} announcement sets retired bit {bit:#x}, so a released \
                     build {consequence}"
                );
            }
        }

        // The bitset a released build reads must not depend on what rides after
        // it, or the check above would only cover the shape it happened to see.
        let [(_, bare_wire), (_, voice_wire)] = &announcements;
        assert_eq!(
            bits_as_a_released_build_reads_them(bare_wire),
            bits_as_a_released_build_reads_them(voice_wire),
            "trailing fields must not move or alter the bitset"
        );
    }

    /// The announcement's byte layout, pinned so a change to it cannot land
    /// without a reviewer seeing the wire move (clonk-org/clonk-rs#471).
    ///
    /// A released build has no `decode_port_capabilities` of ours to call — it
    /// has its own, compiled against the layout that shipped. So the layout is
    /// the interface, and a golden is the cheapest way to make a change to it
    /// visible without compiling a second revision. Anything this diff shows
    /// moving is something an older peer will read at the wrong offset.
    ///
    /// `decode_port_capabilities` ignores trailing bytes by design, which is
    /// what makes the format extensible — and is exactly why *appending* is
    /// safe while *moving* or *reusing* is not.
    #[test]
    fn the_announcement_wire_shape_is_pinned_byte_for_byte() {
        assert_eq!(
            encode_port_capabilities(PortCapabilities::supported()),
            vec![
                // Packet ID, above every ID C++ dispatches.
                0x70, // Vocabulary version 1, little-endian u16.
                0x01, 0x00,
                // Bits, little-endian u32: ROUND_RESTART_V2 (1 << 6) |
                // VOICE_CHAT (1 << 5) | CONTROL_WAIT_ATTRIBUTION (1 << 4).
                // Bit 3 is retired and stays clear — see the retired-capability
                // test above.
                0x70, 0x00, 0x00, 0x00,
            ],
            "the bare announcement moved; an older peer reads these offsets",
        );

        // With a voice route offered, the cookie and then the public key are
        // appended — in that order, and only together.
        let cookie = [0x5a; crate::voice::VOICE_ROUTE_COOKIE_BYTES];
        let public_key = [0x27; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES];
        let offered = PortCapabilities::supported()
            .with_voice_cookie(crate::voice::VoiceRouteCookie::from_bytes(cookie))
            .with_voice_public_key(public_key);

        let mut expected = vec![0x70, 0x01, 0x00, 0x70, 0x00, 0x00, 0x00];
        expected.extend_from_slice(&cookie);
        expected.extend_from_slice(&public_key);
        assert_eq!(
            encode_port_capabilities(offered),
            expected,
            "the voice route rides after the bitset, cookie before key",
        );

        // A key with no cookie derives nothing, so it is never sent alone —
        // and a released build must not find one where it expects the cookie.
        assert_eq!(
            encode_port_capabilities(
                PortCapabilities::supported().with_voice_public_key(public_key)
            ),
            vec![0x70, 0x01, 0x00, 0x70, 0x00, 0x00, 0x00],
            "a public key without its cookie must not reach the wire",
        );
    }

    #[test]
    fn a_silent_peer_is_treated_as_stock_cpp() {
        // The whole safety argument: a stock peer cannot produce this ID — it
        // does not know it — so it never announces and lands here as "supports
        // nothing".
        let registry = PeerCapabilityRegistry::default();

        assert_eq!(registry.of(7), PortCapabilities::default());
        assert!(!registry.peer_supports(7, PortCapabilities::CONTROL_CHANNEL));
    }

    #[test]
    fn round_restart_requires_every_retained_peer_to_announce_v2() {
        let mut registry = PeerCapabilityRegistry::default();
        registry.record(7, PortCapabilities::supported_without_voice());
        registry.record(
            8,
            PortCapabilities::from_bits(PortCapabilities::CONTROL_WAIT_ATTRIBUTION),
        );

        assert!(registry.peer_supports(7, PortCapabilities::ROUND_RESTART_V2));
        assert!(!registry.peer_supports(8, PortCapabilities::ROUND_RESTART_V2));
        assert!(!registry.session_supports([7, 8], PortCapabilities::ROUND_RESTART_V2));
        assert!(registry.session_supports([7], PortCapabilities::ROUND_RESTART_V2));
    }

    #[test]
    fn non_voice_announcement_keeps_every_supported_session_capability() {
        let capabilities = PortCapabilities::supported_without_voice();

        assert!(capabilities.has(PortCapabilities::CONTROL_WAIT_ATTRIBUTION));
        assert!(capabilities.has(PortCapabilities::ROUND_RESTART_V2));
        assert!(!capabilities.has(PortCapabilities::VOICE_CHAT));
    }

    #[test]
    fn a_capability_needs_both_ends() {
        let mut registry = PeerCapabilityRegistry::default();
        // A peer claiming a capability this build does not have gets masked off
        // rather than believed.
        registry.record(
            7,
            PortCapabilities::from_bits(PortCapabilities::CONTROL_CHANNEL),
        );

        assert_eq!(
            registry.peer_supports(7, PortCapabilities::CONTROL_CHANNEL),
            PortCapabilities::supported().has(PortCapabilities::CONTROL_CHANNEL),
            "agreement is the intersection, never the peer's claim alone"
        );
    }

    #[test]
    fn control_wait_attribution_requires_positive_peer_evidence() {
        let mut registry = PeerCapabilityRegistry::default();

        assert!(!registry.peer_supports(7, PortCapabilities::CONTROL_WAIT_ATTRIBUTION));

        registry.record(7, PortCapabilities::supported());

        assert!(registry.peer_supports(7, PortCapabilities::CONTROL_WAIT_ATTRIBUTION));
    }

    #[test]
    fn one_cpp_participant_holds_the_whole_session_on_the_compatible_path() {
        // Session-scoped capabilities change the merged control packet every
        // participant must read, so a single silent peer disables them.
        let mut registry = PeerCapabilityRegistry::default();
        let everything = PortCapabilities::from_bits(u32::MAX);
        registry.record(1, everything);
        registry.record(2, everything);

        assert!(registry.session_supports(
            [1, 2],
            PortCapabilities::supported().bits() & PortCapabilities::ELIDED_EMPTY_CONTROL
        ));
        assert!(
            !registry.session_supports([1, 2, 3], PortCapabilities::ELIDED_EMPTY_CONTROL),
            "client 3 never announced, so it must be assumed to be C++"
        );
    }

    #[test]
    fn an_empty_session_supports_nothing() {
        // Guards against a vacuous `all()`: with no participants there is nobody
        // to have agreed, and enabling a format change would be unfounded.
        let registry = PeerCapabilityRegistry::default();

        assert!(!registry.session_supports([], PortCapabilities::CONTROL_CHANNEL));
    }

    #[test]
    fn this_build_announces_only_what_it_implements() {
        // Announcing a capability before it exists is a promise to a peer that
        // this build cannot keep, and the peer would encode for it.
        assert_eq!(
            PortCapabilities::supported().bits(),
            PortCapabilities::VOICE_CHAT
                | PortCapabilities::CONTROL_WAIT_ATTRIBUTION
                | PortCapabilities::ROUND_RESTART_V2,
            "the advertised mask must name exactly the implemented extensions"
        );
    }

    #[test]
    fn the_retired_cleartext_voice_bit_is_never_announced() {
        // A build from before the media lane was sealed treats bit 3 as "this
        // peer accepts my voice" and opens its microphone on it, transmitting
        // in the clear. Announcing that bit would make this build the reason
        // that audio reaches the wire, for a lane it cannot even receive.
        assert_ne!(
            PortCapabilities::VOICE_CHAT,
            PortCapabilities::RETIRED_CLEARTEXT_VOICE_CHAT
        );
        assert_eq!(
            PortCapabilities::supported().bits() & PortCapabilities::RETIRED_CLEARTEXT_VOICE_CHAT,
            0
        );
    }

    #[test]
    fn a_peer_offering_only_the_retired_cleartext_lane_gets_no_voice() {
        let mut registry = PeerCapabilityRegistry::default();

        registry.record(
            7,
            PortCapabilities::from_bits(PortCapabilities::RETIRED_CLEARTEXT_VOICE_CHAT),
        );

        assert!(!registry.peer_supports(7, PortCapabilities::VOICE_CHAT));
    }

    #[test]
    fn this_build_negotiates_voice_chat_only_with_an_announcing_peer() {
        let mut peers = PeerCapabilityRegistry::default();
        peers.record(4, PortCapabilities::from_bits(PortCapabilities::VOICE_CHAT));

        assert!(peers.peer_supports(4, PortCapabilities::VOICE_CHAT));
        assert!(!peers.peer_supports(5, PortCapabilities::VOICE_CHAT));

        peers.clear(4, PortCapabilities::VOICE_CHAT);
        assert!(!peers.peer_supports(4, PortCapabilities::VOICE_CHAT));
    }

    #[test]
    fn capability_announcement_carries_a_route_local_voice_cookie() {
        let cookie = crate::voice::VoiceRouteCookie::from_bytes(
            [0x6d; crate::voice::VOICE_ROUTE_COOKIE_BYTES],
        );
        let announcement = PortCapabilities::supported().with_voice_cookie(cookie);
        let wire = encode_port_capabilities(announcement);

        assert_eq!(
            decode_port_capabilities(&wire).and_then(PortCapabilities::voice_cookie),
            Some(cookie)
        );
        assert_eq!(
            wire.len(),
            7 + crate::voice::VOICE_ROUTE_COOKIE_BYTES,
            "the legacy version-and-bit prefix stays byte-compatible"
        );
    }

    #[test]
    fn capability_announcement_carries_the_media_lane_key_exchange() {
        let cookie = crate::voice::VoiceRouteCookie::from_bytes(
            [0x6d; crate::voice::VOICE_ROUTE_COOKIE_BYTES],
        );
        let public_key = [0x2f; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES];
        let announcement = PortCapabilities::supported()
            .with_voice_cookie(cookie)
            .with_voice_public_key(public_key);
        let wire = encode_port_capabilities(announcement);

        assert_eq!(decode_port_capabilities(&wire), Some(announcement));
        assert_eq!(
            wire.len(),
            7 + crate::voice::VOICE_ROUTE_COOKIE_BYTES
                + crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES,
            "the exchange follows the cookie, leaving every earlier field in place"
        );
        assert_eq!(
            &wire[..7 + crate::voice::VOICE_ROUTE_COOKIE_BYTES],
            &encode_port_capabilities(PortCapabilities::supported().with_voice_cookie(cookie))[..],
            "a build that stops after the cookie reads the same bytes it always did"
        );
    }

    #[test]
    fn a_key_exchange_without_its_cookie_is_not_announced() {
        // The media key schedule labels each direction by the receiving side's
        // cookie, so a key with no cookie beside it can derive nothing and
        // would only invite a peer to try.
        let announcement = PortCapabilities::supported()
            .with_voice_public_key([0x2f; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES]);

        assert_eq!(encode_port_capabilities(announcement).len(), 7);
    }

    #[test]
    fn the_peer_registry_keeps_bits_and_never_route_key_material() {
        // Registry entries are session-wide and outlive the route the cookie
        // and exchange belong to; only the bitset is meaningful beyond it.
        let announced = PortCapabilities::supported()
            .with_voice_cookie(crate::voice::VoiceRouteCookie::from_bytes(
                [0x6d; crate::voice::VOICE_ROUTE_COOKIE_BYTES],
            ))
            .with_voice_public_key([0x2f; crate::voice::VOICE_KEY_AGREEMENT_PUBLIC_BYTES]);
        let mut registry = PeerCapabilityRegistry::default();

        registry.record(7, announced);

        assert_eq!(registry.of(7), PortCapabilities::supported());
        assert_eq!(registry.of(7).voice_cookie(), None);
        assert_eq!(registry.of(7).voice_public_key(), None);
    }
}
