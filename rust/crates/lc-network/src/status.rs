use std::collections::BTreeMap;

use crate::{ClientId, NetworkStatus};

/// Host-side phase corresponding to `fStatusReached`/`fStatusAck` in
/// `C4Network2` (`src/C4Network2.cpp:1994-2110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierPhase {
    Stable,
    Waiting { local_reached: bool },
}

/// The C++ network-client states relevant to `AllClientsReady`
/// (`src/C4Network2Client.h:37-45,110-115`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteBarrierState {
    Joining,
    Chasing,
    NotReady,
    Ready,
    Removing,
}

/// Ordered side effects produced by a status transition. The consumer must
/// preserve this order because pending activation controls execute before the
/// Go player-info sweep (`src/C4Network2.cpp:2062-2110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierEffect {
    InvalidateReference,
    BroadcastStatus(NetworkStatus),
    DriveControlTo(i32),
    StopControl,
    ExecutePendingSyncControls,
    BroadcastStatusAck(NetworkStatus),
    SendStatusAck {
        client_id: ClientId,
        status: NetworkStatus,
    },
    SetControlMode(i32),
    SweepUnjoinedPlayers,
    StartControl,
}

/// Deterministic host-side projection of the C++ status barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBarrier {
    pub status: NetworkStatus,
    pub phase: BarrierPhase,
    pub remotes: BTreeMap<ClientId, RemoteBarrierState>,
}
