//! Late arrival of the transfer identity for a resource announced without it.
//!
//! A host that publishes before its exact deflates have run announces every
//! directory-backed resource with `C4Network2ResCore::Set`'s non-loadable
//! sentinels (`src/C4Network2Res.cpp:83-92`). That is enough for a peer which
//! already has the content — `SetByCore` matches on the contents CRC alone
//! (`src/C4Network2Res.cpp:448`) — but it leaves the core without the size and
//! CRC that identify the chunk stream, and C++ has no way to supply them
//! afterwards: `SendJoinData` snapshots the parameters once
//! (`src/C4Network2.cpp:1839`).
//!
//! This packet supplies them. It is port-only, so it may be sent only to a peer
//! that announced [`crate::PortCapabilities::DEFERRED_RESOURCE_CORES`]; a stock
//! peer is never given a deferred core in the first place.
//!
//! The count's high bit marks a pending announcement before JoinData. Clearing
//! that bit supplies the completed cores, including resources whose final size
//! leaves them non-loadable. Missing local content waits only for announced
//! pending cores; ordinary non-loadable resources still fail admission.

use clonk_protocol::NetworkResourceCore;

use crate::resource_packet::{
    decode_resource_core_payload, encode_resource_core_payload, ResourcePacketCodecError,
};

/// Packet ID for host-to-client resource-core upgrades.
///
/// In the port-only `0x7x` range, above every packet ID the pinned C++ oracle
/// dispatches. A stock peer would close the connection on it, which is why it
/// is gated on the capability. See [`crate::capabilities`].
pub const PID_PORT_RESOURCE_UPGRADE: u8 = 0x75;

/// Upper bound on the cores one upgrade may carry.
///
/// A publication announces one resource per scenario, definition, material and
/// player, so a real upgrade is a handful of cores. The bound exists so a
/// hostile count cannot make the decoder reserve unbounded memory.
pub const MAX_RESOURCE_UPGRADE_CORES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceUpgradePacket {
    /// True announces the cores still awaiting packing before JoinData.
    /// False supplies their final transfer identities.
    pub pending: bool,
    /// Cores keep the same content identity and ID in both states.
    pub cores: Vec<NetworkResourceCore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceUpgradeCodecError {
    Truncated,
    TrailingData,
    CoreCountOutOfRange(u32),
    Core(ResourcePacketCodecError),
}

impl std::fmt::Display for ResourceUpgradeCodecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(formatter, "resource upgrade payload ended early"),
            Self::TrailingData => write!(formatter, "resource upgrade payload has trailing data"),
            Self::CoreCountOutOfRange(count) => write!(
                formatter,
                "resource upgrade announces {count} cores, above the {MAX_RESOURCE_UPGRADE_CORES} limit"
            ),
            Self::Core(error) => write!(formatter, "resource upgrade core: {error}"),
        }
    }
}

impl std::error::Error for ResourceUpgradeCodecError {}

/// Frames the packet with its ID, as it goes on the wire.
pub(crate) fn encode_resource_upgrade(
    packet: &ResourceUpgradePacket,
) -> Result<Vec<u8>, ResourceUpgradeCodecError> {
    let mut wire = vec![PID_PORT_RESOURCE_UPGRADE];
    wire.extend(encode_resource_upgrade_payload(packet)?);
    Ok(wire)
}

pub(crate) fn decode_resource_upgrade(wire: &[u8]) -> Option<ResourceUpgradePacket> {
    if wire.first().copied()? != PID_PORT_RESOURCE_UPGRADE {
        return None;
    }
    decode_resource_upgrade_payload(wire.get(1..)?).ok()
}

pub fn encode_resource_upgrade_payload(
    packet: &ResourceUpgradePacket,
) -> Result<Vec<u8>, ResourceUpgradeCodecError> {
    let count = u32::try_from(packet.cores.len())
        .ok()
        .filter(|count| *count as usize <= MAX_RESOURCE_UPGRADE_CORES)
        .ok_or(ResourceUpgradeCodecError::CoreCountOutOfRange(u32::MAX))?;
    let count = count | if packet.pending { 1 << 31 } else { 0 };
    let mut payload = count.to_ne_bytes().to_vec();
    for core in &packet.cores {
        let core = encode_resource_core_payload(core).map_err(ResourceUpgradeCodecError::Core)?;
        // Each core is length-prefixed because the C4Network2ResCore layout is
        // variable and this packet carries several back to back.
        let length = u32::try_from(core.len())
            .map_err(|_| ResourceUpgradeCodecError::CoreCountOutOfRange(u32::MAX))?;
        payload.extend_from_slice(&length.to_ne_bytes());
        payload.extend_from_slice(&core);
    }
    Ok(payload)
}

pub fn decode_resource_upgrade_payload(
    payload: &[u8],
) -> Result<ResourceUpgradePacket, ResourceUpgradeCodecError> {
    let mut rest = payload;
    let header = read_u32(&mut rest)?;
    let pending = header & (1 << 31) != 0;
    let count = header & !(1 << 31);
    if count as usize > MAX_RESOURCE_UPGRADE_CORES {
        return Err(ResourceUpgradeCodecError::CoreCountOutOfRange(count));
    }
    let mut cores = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let length = read_u32(&mut rest)? as usize;
        let (core, remainder) = rest
            .split_at_checked(length)
            .ok_or(ResourceUpgradeCodecError::Truncated)?;
        cores.push(decode_resource_core_payload(core).map_err(ResourceUpgradeCodecError::Core)?);
        rest = remainder;
    }
    if !rest.is_empty() {
        return Err(ResourceUpgradeCodecError::TrailingData);
    }
    Ok(ResourceUpgradePacket { pending, cores })
}

fn read_u32(rest: &mut &[u8]) -> Result<u32, ResourceUpgradeCodecError> {
    let (head, remainder) = rest
        .split_at_checked(4)
        .ok_or(ResourceUpgradeCodecError::Truncated)?;
    *rest = remainder;
    Ok(u32::from_ne_bytes(
        head.try_into().expect("split_at_checked yields four bytes"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PID_PORT_CAPABILITIES;

    fn loadable_core(id: i32) -> NetworkResourceCore {
        NetworkResourceCore {
            resource_type: crate::HostResourceType::Definitions as u8,
            id,
            loadable: true,
            file_size: 4096,
            file_crc: 0xdead_beef,
            chunk_size: crate::STOCK_CHUNK_SIZE,
            contents_crc: 0x0bad_f00d,
            filename: clonk_protocol::LegacyCString::from_bytes(b"Objects.c4d".to_vec()).unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn a_framed_resource_upgrade_round_trips_through_its_packet_id() {
        let packet = ResourceUpgradePacket {
            pending: false,
            cores: vec![loadable_core(1)],
        };
        let wire = encode_resource_upgrade(&packet).unwrap();
        assert_eq!(wire.first(), Some(&PID_PORT_RESOURCE_UPGRADE));
        assert_eq!(decode_resource_upgrade(&wire), Some(packet));
        assert_eq!(decode_resource_upgrade(&[PID_PORT_CAPABILITIES]), None);
    }

    #[test]
    fn resource_upgrade_round_trips_every_announced_core() {
        let packet = ResourceUpgradePacket {
            pending: false,
            cores: vec![loadable_core(0), loadable_core(3)],
        };
        let payload = encode_resource_upgrade_payload(&packet).unwrap();
        assert_eq!(decode_resource_upgrade_payload(&payload).unwrap(), packet);
    }

    #[test]
    fn resource_upgrade_rejects_a_count_no_payload_could_satisfy() {
        let mut payload = u32::MAX.to_ne_bytes().to_vec();
        payload.extend_from_slice(&0_u32.to_ne_bytes());
        assert_eq!(
            decode_resource_upgrade_payload(&payload),
            Err(ResourceUpgradeCodecError::CoreCountOutOfRange(0x7fff_ffff))
        );
    }

    #[test]
    fn resource_upgrade_rejects_a_core_running_past_the_payload() {
        let mut payload = 1_u32.to_ne_bytes().to_vec();
        payload.extend_from_slice(&64_u32.to_ne_bytes());
        payload.extend_from_slice(b"short");
        assert_eq!(
            decode_resource_upgrade_payload(&payload),
            Err(ResourceUpgradeCodecError::Truncated)
        );
    }

    #[test]
    fn resource_upgrade_rejects_trailing_data() {
        let mut payload =
            encode_resource_upgrade_payload(&ResourceUpgradePacket::default()).unwrap();
        payload.push(1);
        assert!(decode_resource_upgrade_payload(&payload).is_err());
    }

    #[test]
    fn resource_upgrade_decodes_a_pending_publication() {
        let packet = ResourceUpgradePacket {
            pending: false,
            cores: vec![loadable_core(1)],
        };
        let mut payload = encode_resource_upgrade_payload(&packet).unwrap();
        payload[..4].copy_from_slice(&(0x8000_0000_u32 | 1).to_ne_bytes());
        let decoded = decode_resource_upgrade_payload(&payload).unwrap();
        assert!(decoded.pending);
        assert_eq!(decoded.cores, packet.cores);
    }
}
