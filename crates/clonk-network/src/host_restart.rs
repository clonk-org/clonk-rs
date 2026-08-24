//! The host announcing that it is restarting the round, not dying.
//!
//! Restarting a round tears the host's whole network session down and re-hosts
//! from scratch — that is what C++ does (`C4Application::QuitGame` restores
//! `Game.fLobby`/`NetworkActive` for the next mission,
//! src/C4Application.cpp:373-405, after `C4Network2::Clear` has closed `NetIO`,
//! src/C4Network2.cpp:748-796). A client therefore observes exactly the same
//! thing it observes when the host crashes: its connection closes. Native reads
//! that as a dead host and drops the round to local control
//! (src/C4Network2.cpp:1826-1832), so nobody comes back.
//!
//! This notice is the missing intent. The host sends it before tearing down, so
//! a port client can distinguish "back in a moment, same address" from "gone",
//! and follow the host into the new lobby.
//!
//! Port-only, and safe **here specifically** — but not because C++ ignores it.
//! `C4Network2IO::HandlePacket` unpacks before dispatching, and
//! `C4IDPacket::CompileFunc` `excCorrupt`s on an ID with no `FnUnpack`
//! (oracle-src-pinned src/C4Network2IO.cpp:820-834, src/C4Packet2.cpp:210-217);
//! the handler catches that, logs it, and in a release build closes the
//! connection. So a C++ peer that receives this drops the connection.
//!
//! That is harmless for this packet and only this packet: the host sends it as
//! the last thing it does before tearing the session down anyway, so a C++
//! client's connection closing a few milliseconds early is indistinguishable
//! from the close it was about to observe, and it falls back to the native
//! dead-host path either way. Do not copy this reasoning to a packet sent
//! during a session that is meant to survive.
//!
//! # Two notices, two costs
//!
//! [`PID_PORT_HOST_RESTARTING`] above is the *reconnect* notice: it tells a
//! client the address will answer again, and the client pays for a fresh
//! connection — handshake, admission and resource negotiation — to get back in.
//!
//! [`PID_PORT_HOST_RESTART_LOBBY`] is the cheaper one. The host keeps its
//! session, sockets and client list up across the restart and only rebuilds the
//! round, so the client keeps the connection it already has and re-enters the
//! lobby in place. One FIFO host route survives as the round boundary; any
//! retired auxiliary TCP/UDP route may reconnect internally without repeating
//! client admission. Fresh round resources are reconciled on the retained
//! resource session, so only content that changed is loaded.
//!
//! Unlike the reconnect notice, the session-preserving packet is never sent to
//! a stock or older port peer. Before it mutates any round state or queues this
//! marker, the host requires every retained client to have advertised
//! [`crate::PortCapabilities::ROUND_RESTART_V2`]. If even one client lacks the
//! capability, the atomic restart rejects and the application falls back to
//! the ordinary teardown/re-host path (and its reconnect notice). This keeps a
//! port-only ID off every connection whose peer cannot parse it, while still
//! allowing a host with no retained clients to restart locally in place.

/// Packet ID for the restart notice.
///
/// Sits directly above [`crate::PID_PORT_CAPABILITIES`], in the same range
/// chosen to be above every ID C++ dispatches.
pub const PID_PORT_HOST_RESTARTING: u8 = 0x71;

/// Packet ID for the notice that keeps the session up across the restart.
///
/// Sits directly above [`PID_PORT_HOST_RESTARTING`], inside the same `0x7x`
/// port-only range the host refuses to relay.
pub const PID_PORT_HOST_RESTART_LOBBY: u8 = 0x72;

/// Packet ID for a retained client releasing the next-round ingress fence.
pub const PID_PORT_ROUND_RESTART_ACK: u8 = 0x74;

/// How long a client should keep trying to reach the restarted host before it
/// gives up, in seconds. Carried on the wire so the host — the only side that
/// knows how long its own teardown and re-host take — sets the budget.
pub const DEFAULT_HOST_RESTART_REJOIN_SECONDS: u16 = 30;

/// Encodes the session-preserving restart notice.
///
/// Unlike [`encode_host_restart_notice`] this carries no rejoin window: the
/// connection it arrives on is the one the client keeps, so there is no
/// re-dial for the host to budget.
pub fn encode_host_restart_lobby_notice(restart_nonce: u64) -> Vec<u8> {
    let mut wire = vec![PID_PORT_HOST_RESTART_LOBBY];
    wire.extend_from_slice(&restart_nonce.to_le_bytes());
    wire
}

/// Decodes the nonce that fences one session-preserving restart.
pub fn decode_host_restart_lobby_notice(wire: &[u8]) -> Option<u64> {
    if wire.len() != 9 || wire.first().copied()? != PID_PORT_HOST_RESTART_LOBBY {
        return None;
    }
    wire.get(1..9)
        .and_then(|nonce| nonce.try_into().ok())
        .map(u64::from_le_bytes)
}

pub fn encode_round_restart_ack(restart_nonce: u64) -> Vec<u8> {
    let mut wire = vec![PID_PORT_ROUND_RESTART_ACK];
    wire.extend_from_slice(&restart_nonce.to_le_bytes());
    wire
}

pub fn decode_round_restart_ack(wire: &[u8]) -> Option<u64> {
    if wire.len() != 9 || wire.first().copied()? != PID_PORT_ROUND_RESTART_ACK {
        return None;
    }
    wire.get(1..9)
        .and_then(|nonce| nonce.try_into().ok())
        .map(u64::from_le_bytes)
}

/// Encodes the notice. Body is the rejoin window in seconds, little-endian, to
/// match [`crate::encode_port_capabilities`].
pub fn encode_host_restart_notice(rejoin_seconds: u16) -> Vec<u8> {
    let mut wire = vec![PID_PORT_HOST_RESTARTING];
    wire.extend_from_slice(&rejoin_seconds.to_le_bytes());
    wire
}

/// Decodes a notice, or `None` if this is not one or is truncated.
pub fn decode_host_restart_notice(wire: &[u8]) -> Option<u16> {
    if wire.first().copied()? != PID_PORT_HOST_RESTARTING {
        return None;
    }
    wire.get(1..3)
        .and_then(|window| window.try_into().ok())
        .map(u16::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_extension_packet_ids_are_unique() {
        let packet_ids = [
            crate::PID_PORT_CAPABILITIES,
            PID_PORT_HOST_RESTARTING,
            PID_PORT_HOST_RESTART_LOBBY,
            crate::PID_PORT_CONTROL_WAIT_ATTRIBUTION,
            PID_PORT_ROUND_RESTART_ACK,
        ];
        let unique = packet_ids
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(unique.len(), packet_ids.len());
    }

    #[test]
    fn a_notice_encodes_its_rejoin_window() {
        assert_eq!(
            encode_host_restart_notice(30),
            vec![PID_PORT_HOST_RESTARTING, 30, 0]
        );
    }

    #[test]
    fn a_truncated_notice_is_not_a_notice() {
        assert_eq!(
            decode_host_restart_notice(&[PID_PORT_HOST_RESTARTING]),
            None
        );
        assert_eq!(
            decode_host_restart_notice(&[PID_PORT_HOST_RESTARTING, 30]),
            None
        );
    }

    #[test]
    fn another_packet_id_is_not_a_notice() {
        assert_eq!(
            decode_host_restart_notice(&[crate::PID_PORT_CAPABILITIES, 30, 0]),
            None
        );
    }

    #[test]
    fn a_lobby_notice_carries_its_restart_nonce_little_endian() {
        assert_eq!(
            encode_host_restart_lobby_notice(0x0102_0304_0506_0708),
            vec![PID_PORT_HOST_RESTART_LOBBY, 8, 7, 6, 5, 4, 3, 2, 1]
        );
        assert_eq!(
            decode_host_restart_lobby_notice(&[
                PID_PORT_HOST_RESTART_LOBBY,
                8,
                7,
                6,
                5,
                4,
                3,
                2,
                1,
            ]),
            Some(0x0102_0304_0506_0708)
        );
    }

    #[test]
    fn the_reconnect_notice_is_not_a_lobby_notice() {
        assert_eq!(
            decode_host_restart_lobby_notice(&[PID_PORT_HOST_RESTARTING, 30, 0]),
            None
        );
        assert_eq!(decode_host_restart_lobby_notice(&[]), None);
    }

    #[test]
    fn a_retained_round_ack_is_an_exact_port_packet() {
        let nonce = 0xfedc_ba98_7654_3210;
        assert_eq!(
            encode_round_restart_ack(nonce),
            vec![
                PID_PORT_ROUND_RESTART_ACK,
                0x10,
                0x32,
                0x54,
                0x76,
                0x98,
                0xba,
                0xdc,
                0xfe
            ]
        );
        assert_eq!(
            decode_round_restart_ack(&encode_round_restart_ack(nonce)),
            Some(nonce)
        );
        assert_eq!(
            decode_round_restart_ack(&[PID_PORT_ROUND_RESTART_ACK]),
            None
        );
        assert_eq!(decode_round_restart_ack(&[]), None);
    }
}
