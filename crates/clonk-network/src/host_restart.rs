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

/// Packet ID for the restart notice.
///
/// Sits directly above [`crate::PID_PORT_CAPABILITIES`], in the same range
/// chosen to be above every ID C++ dispatches.
pub const PID_PORT_HOST_RESTARTING: u8 = 0x71;

/// How long a client should keep trying to reach the restarted host before it
/// gives up, in seconds. Carried on the wire so the host — the only side that
/// knows how long its own teardown and re-host take — sets the budget.
pub const DEFAULT_HOST_RESTART_REJOIN_SECONDS: u16 = 30;

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
}
