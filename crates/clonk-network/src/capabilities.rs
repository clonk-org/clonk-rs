//! Capability negotiation with a peer that may or may not be this port.
//!
//! The port wants protocol improvements a stock C++ peer cannot parse — a
//! separate logical channel for control, redundancy carried in-band, elided
//! empty control. None of those may break the ability to join an old
//! LegacyClonk server, so each is announced and used only when *both* ends
//! support it.
//!
//! The announcement is safe to send unconditionally because C++'s
//! `C4Network2IO::HandlePacket` switch has **no `default:` case**
//! (oracle-src-pinned src/C4Network2IO.cpp): a packet ID it does not know is
//! silently ignored, not rejected and not fatal. So this rides a PID outside
//! C++'s range, a C++ peer drops it on the floor, and the absence of a reply is
//! itself the signal that we are talking to C++.
//!
//! The default is therefore always the C++-compatible path. A capability is
//! enabled only on positive evidence, never assumed from a version number.

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
    /// Best-effort voice media carried outside reliable packet accounting.
    pub const VOICE_CHAT: u32 = 1 << 3;

    /// Everything this build knows how to do.
    pub fn supported() -> Self {
        Self::from_bits(Self::VOICE_CHAT)
    }

    pub fn from_bits(bits: u32) -> Self {
        Self {
            bits,
            voice_cookie: None,
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
}

/// Encodes the announcement. Body is the vocabulary version then the bitset,
/// both little-endian, so an older peer can read the version and stop.
pub fn encode_port_capabilities(capabilities: PortCapabilities) -> Vec<u8> {
    let mut wire = vec![PID_PORT_CAPABILITIES];
    wire.extend_from_slice(&PORT_CAPABILITY_VERSION.to_le_bytes());
    wire.extend_from_slice(&capabilities.bits().to_le_bytes());
    if let Some(cookie) = capabilities.voice_cookie() {
        wire.extend_from_slice(&cookie.into_bytes());
    }
    wire
}

/// Decodes an announcement, or `None` if this is not one or is truncated.
///
/// A future vocabulary version is read for its bits anyway: unknown bits are
/// capabilities this build does not have, and `agreed_with` masks them off.
pub fn decode_port_capabilities(wire: &[u8]) -> Option<PortCapabilities> {
    if wire.first().copied()? != PID_PORT_CAPABILITIES || wire.len() < 7 {
        return None;
    }
    let bits = u32::from_le_bytes(wire.get(3..7)?.try_into().ok()?);
    let voice_cookie = wire
        .get(7..7 + crate::voice::VOICE_ROUTE_COOKIE_BYTES)
        .and_then(|bytes| bytes.try_into().ok())
        .map(crate::voice::VoiceRouteCookie::from_bytes);
    Some(PortCapabilities { bits, voice_cookie })
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

    #[test]
    fn a_silent_peer_is_treated_as_stock_cpp() {
        // The whole safety argument: C++ ignores the announcement, so it never
        // replies, so it lands here as "supports nothing".
        let registry = PeerCapabilityRegistry::default();

        assert_eq!(registry.of(7), PortCapabilities::default());
        assert!(!registry.peer_supports(7, PortCapabilities::CONTROL_CHANNEL));
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
            PortCapabilities::VOICE_CHAT,
            "voice media is the only implemented extension"
        );
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
}
