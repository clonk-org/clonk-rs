//! Receiver-local attribution for a host-routed control wait.
//!
//! This packet never enters the synchronized control queue. It only tells a
//! Rust client whether the host waited for its contribution, another client's
//! contribution, or both before publishing one aggregate control tick.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Tick;

const CONTROL_WAIT_ATTRIBUTION_LIMIT: usize = 256;

/// Packet ID for host-to-client control-wait attribution.
///
/// This is in the port-only `0x7x` range, above every packet ID dispatched by
/// the pinned C++ oracle. A stock peer would close the connection on it (release
/// builds), so the host sends it only to peers that announced
/// [`crate::PortCapabilities::CONTROL_WAIT_ATTRIBUTION`] — a stock peer never
/// receives one. See [`crate::capabilities`].
pub const PID_PORT_CONTROL_WAIT_ATTRIBUTION: u8 = 0x73;

/// Host-side classification of the participants missing at one control tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlWaitAttribution {
    /// Control tick whose first host-side wait this describes.
    pub tick: Tick,
    /// Whether this packet's recipient was missing.
    pub waited_for_recipient: bool,
    /// Whether at least one different participant was missing.
    pub waited_for_other: bool,
    /// Whether the async deadline expired with this recipient still missing, so
    /// the host packed the tick without its control and the input is gone.
    ///
    /// `force_expired_async_control` mirrors `PackCompleteCtrl`
    /// (C4GameControlNetwork.cpp:741-784): the absent client's control is
    /// dropped, not deferred, and a packet arriving afterwards is rejected as
    /// stale. Without this flag that loss is silent, and a player cannot tell a
    /// vanished keypress from an engine bug.
    pub discarded_recipient_control: bool,
}

/// Thread-safe receiver snapshot keyed by control tick.
#[derive(Clone, Debug, Default)]
pub struct ControlWaitAttributionSnapshot {
    by_tick: Arc<RwLock<BTreeMap<Tick, ControlWaitAttribution>>>,
}

impl ControlWaitAttributionSnapshot {
    /// Builds a snapshot from already-received attributions.
    pub fn from_attributions(
        attributions: impl IntoIterator<Item = ControlWaitAttribution>,
    ) -> Self {
        let snapshot = Self::default();
        for attribution in attributions {
            snapshot.publish(attribution);
        }
        snapshot
    }

    pub(crate) fn publish(&self, attribution: ControlWaitAttribution) {
        let mut by_tick = self
            .by_tick
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        by_tick.insert(attribution.tick, attribution);
        while by_tick.len() > CONTROL_WAIT_ATTRIBUTION_LIMIT {
            by_tick.pop_first();
        }
    }

    /// Samples attribution for exactly `tick` without consuming it.
    pub fn sample(&self, tick: Tick) -> Option<ControlWaitAttribution> {
        self.by_tick
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&tick)
            .copied()
    }
}

pub(crate) fn encode_control_wait_attribution(attribution: ControlWaitAttribution) -> Vec<u8> {
    let mut wire = vec![PID_PORT_CONTROL_WAIT_ATTRIBUTION];
    wire.extend_from_slice(&attribution.tick.to_le_bytes());
    // The flags byte grows by bit, and the decoder masks each bit it knows, so
    // a build that predates the discard flag reads the packet as the wait-only
    // attribution it already understood rather than rejecting it. That keeps
    // the existing `CONTROL_WAIT_ATTRIBUTION` bit honest: every peer that
    // announces it can still decode every packet it is sent.
    wire.push(
        u8::from(attribution.waited_for_recipient)
            | (u8::from(attribution.waited_for_other) << 1)
            | (u8::from(attribution.discarded_recipient_control) << 2),
    );
    wire
}

pub(crate) fn decode_control_wait_attribution(wire: &[u8]) -> Option<ControlWaitAttribution> {
    if wire.first().copied()? != PID_PORT_CONTROL_WAIT_ATTRIBUTION {
        return None;
    }
    let tick = Tick::from_le_bytes(wire.get(1..5)?.try_into().ok()?);
    let flags = *wire.get(5)?;
    Some(ControlWaitAttribution {
        tick,
        waited_for_recipient: flags & 1 != 0,
        waited_for_other: flags & 2 != 0,
        discarded_recipient_control: flags & 4 != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_attribution_snapshot_is_keyed_to_its_control_tick() {
        let snapshot = ControlWaitAttributionSnapshot::default();
        let attribution = ControlWaitAttribution {
            tick: 73,
            waited_for_recipient: false,
            waited_for_other: true,
            discarded_recipient_control: false,
        };

        snapshot.publish(attribution);

        assert_eq!(snapshot.sample(72), None);
        assert_eq!(snapshot.sample(73), Some(attribution));
        assert_eq!(snapshot.sample(74), None);
    }

    /// The discard outcome rides the same packet as the wait it resolves, and
    /// a peer built before the flag existed still decodes that packet — it
    /// masks the bits it knows, so the unknown bit reads as "not discarded"
    /// rather than closing the connection.
    #[test]
    fn a_discarded_control_survives_the_wire_and_is_ignored_by_an_older_decoder() {
        let discarded = ControlWaitAttribution {
            tick: 4242,
            waited_for_recipient: true,
            waited_for_other: false,
            discarded_recipient_control: true,
        };

        let wire = encode_control_wait_attribution(discarded);
        assert_eq!(decode_control_wait_attribution(&wire), Some(discarded));

        // What a build that only knows bits 0 and 1 reads out of the same
        // bytes: the wait it always understood, with no discard.
        let flags = wire[5];
        assert_eq!(flags & 1 != 0, discarded.waited_for_recipient);
        assert_eq!(flags & 2 != 0, discarded.waited_for_other);

        // A wait that did not end in a discard keeps the old encoding exactly.
        let waited_only = ControlWaitAttribution {
            discarded_recipient_control: false,
            ..discarded
        };
        assert_eq!(encode_control_wait_attribution(waited_only)[5], 1);
    }

    /// The signal must never become a control. Determinism here rests on the
    /// host alone deciding the timeout and broadcasting one authoritative
    /// aggregate that every client executes identically; a notice inside that
    /// stream would make one client's loss a synchronized event, and a client
    /// that missed the notice would execute a different tick.
    #[test]
    fn the_discard_signal_never_travels_as_control() {
        // C++ dispatches control on 0x40-0x43. This packet is in the port-only
        // 0x7x range, above every ID the pinned oracle dispatches, so no
        // control decoder will ever see it.
        const { assert!(PID_PORT_CONTROL_WAIT_ATTRIBUTION >= 0x70) };
        for control_pid in [0x40_u8, 0x41, 0x42, 0x43] {
            assert_ne!(PID_PORT_CONTROL_WAIT_ATTRIBUTION, control_pid);
        }

        // And it carries no executable payload to smuggle one in: a tick and a
        // flags byte, nothing a control queue could run.
        let wire = encode_control_wait_attribution(ControlWaitAttribution {
            tick: 1,
            waited_for_recipient: true,
            waited_for_other: true,
            discarded_recipient_control: true,
        });
        assert_eq!(wire.len(), 6);
    }
}
