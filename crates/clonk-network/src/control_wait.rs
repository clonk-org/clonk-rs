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
/// the pinned C++ oracle. A stock peer silently ignores it.
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
    wire.push(
        u8::from(attribution.waited_for_recipient) | (u8::from(attribution.waited_for_other) << 1),
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
        };

        snapshot.publish(attribution);

        assert_eq!(snapshot.sample(72), None);
        assert_eq!(snapshot.sample(73), Some(attribution));
        assert_eq!(snapshot.sample(74), None);
    }
}
