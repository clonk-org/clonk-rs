use std::collections::BTreeMap;

use crate::{
    ClientId, NetworkStatus, NETWORK_STATE_GO, NETWORK_STATE_LOBBY, NETWORK_STATE_PAUSE,
};

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

impl StatusBarrier {
    pub fn stable(status: NetworkStatus) -> Self {
        Self {
            status,
            phase: BarrierPhase::Stable,
            remotes: BTreeMap::new(),
        }
    }

    pub fn set_remote_state(&mut self, client_id: ClientId, state: RemoteBarrierState) {
        self.remotes.insert(client_id, state);
    }

    pub fn remove_remote(&mut self, client_id: ClientId) -> Vec<BarrierEffect> {
        self.remotes.remove(&client_id);
        self.try_commit()
    }

    pub fn change_status(&mut self, status: NetworkStatus) -> Vec<BarrierEffect> {
        self.status = status;
        for state in self.remotes.values_mut() {
            if matches!(
                *state,
                RemoteBarrierState::Ready | RemoteBarrierState::NotReady
            ) {
                *state = RemoteBarrierState::NotReady;
            }
        }
        self.phase = BarrierPhase::Waiting {
            local_reached: false,
        };
        let mut effects = vec![
            BarrierEffect::InvalidateReference,
            BarrierEffect::BroadcastStatus(status),
        ];
        if matches!(status.state, NETWORK_STATE_GO | NETWORK_STATE_PAUSE) {
            effects.push(BarrierEffect::DriveControlTo(status.target_tick));
        }
        effects
    }

    pub fn local_reached(&mut self) -> Vec<BarrierEffect> {
        let BarrierPhase::Waiting { local_reached } = &mut self.phase else {
            return Vec::new();
        };
        if *local_reached {
            return self.try_commit();
        }
        *local_reached = true;
        let mut effects = vec![BarrierEffect::StopControl];
        effects.extend(self.try_commit());
        effects
    }

    pub fn remote_ack(
        &mut self,
        client_id: ClientId,
        acknowledgement: NetworkStatus,
    ) -> Vec<BarrierEffect> {
        let Some(state) = self.remotes.get(&client_id).copied() else {
            return Vec::new();
        };
        if matches!(
            state,
            RemoteBarrierState::Joining | RemoteBarrierState::Removing
        ) || acknowledgement.state != self.status.state
            || acknowledgement.target_tick < self.status.target_tick
        {
            return Vec::new();
        }

        if self.phase == BarrierPhase::Stable {
            self.remotes
                .insert(client_id, RemoteBarrierState::Ready);
            return vec![BarrierEffect::SendStatusAck {
                client_id,
                status: acknowledgement,
            }];
        }

        if acknowledgement.target_tick > self.status.target_tick {
            let status = NetworkStatus {
                target_tick: acknowledgement.target_tick,
                ..self.status
            };
            let effects = self.change_status(status);
            self.remotes
                .insert(client_id, RemoteBarrierState::Ready);
            return effects;
        }

        self.remotes
            .insert(client_id, RemoteBarrierState::Ready);
        self.try_commit()
    }

    pub fn sync(&mut self, next_control_tick: i32) -> Vec<BarrierEffect> {
        if matches!(self.phase, BarrierPhase::Waiting { .. }) {
            return self.try_commit();
        }
        if self.status.state == NETWORK_STATE_LOBBY
            || self.status.state == NETWORK_STATE_PAUSE
        {
            return Vec::new();
        }
        self.change_status(NetworkStatus {
            target_tick: next_control_tick,
            ..self.status
        })
    }

    pub fn is_running(&self) -> bool {
        self.status.state == NETWORK_STATE_GO && self.phase == BarrierPhase::Stable
    }

    fn try_commit(&mut self) -> Vec<BarrierEffect> {
        if !matches!(
            self.phase,
            BarrierPhase::Waiting {
                local_reached: true
            }
        ) || self
            .remotes
            .values()
            .any(|state| *state == RemoteBarrierState::NotReady)
        {
            return Vec::new();
        }

        self.phase = BarrierPhase::Stable;
        let mut effects = vec![
            BarrierEffect::ExecutePendingSyncControls,
            BarrierEffect::BroadcastStatusAck(self.status),
        ];
        if self.status.state == NETWORK_STATE_GO {
            effects.extend([
                BarrierEffect::SetControlMode(self.status.control_mode),
                BarrierEffect::SweepUnjoinedPlayers,
                BarrierEffect::StartControl,
            ]);
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NETWORK_STATE_GO, NETWORK_STATE_LOBBY};

    #[test]
    fn go_requires_local_and_all_waited_remotes() {
        // ChangeGameStatus resets every waited-for remote. CheckStatusAck may
        // commit only after the host reaches the target and all such remotes
        // acknowledge (src/C4Network2.cpp:1994-2014,2062-2110).
        let mut barrier = StatusBarrier::stable(NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: -1,
        });
        barrier.set_remote_state(1, RemoteBarrierState::Ready);
        barrier.set_remote_state(2, RemoteBarrierState::Ready);
        let go = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 10,
        };

        assert_eq!(
            barrier.change_status(go),
            vec![
                BarrierEffect::InvalidateReference,
                BarrierEffect::BroadcastStatus(go),
                BarrierEffect::DriveControlTo(10),
            ]
        );
        assert_eq!(barrier.remote_ack(1, go), Vec::new());
        assert_eq!(barrier.local_reached(), vec![BarrierEffect::StopControl]);
        assert!(!barrier.is_running());
        assert_eq!(
            barrier.remote_ack(2, go),
            vec![
                BarrierEffect::ExecutePendingSyncControls,
                BarrierEffect::BroadcastStatusAck(go),
                BarrierEffect::SetControlMode(1),
                BarrierEffect::SweepUnjoinedPlayers,
                BarrierEffect::StartControl,
            ]
        );
        assert!(barrier.is_running());
        assert_eq!(barrier.phase, BarrierPhase::Stable);
    }

    #[test]
    fn same_state_go_sync_reopens_the_barrier() {
        // Sync changes to the already-active state at getNextControlTick when
        // Go is acknowledged and therefore not frozen
        // (src/C4Network2.cpp:541-555,1982-1991).
        let mut barrier = StatusBarrier::stable(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 10,
        });
        barrier.set_remote_state(3, RemoteBarrierState::Ready);

        let next = NetworkStatus {
            target_tick: 11,
            ..barrier.status
        };
        assert_eq!(
            barrier.sync(11),
            vec![
                BarrierEffect::InvalidateReference,
                BarrierEffect::BroadcastStatus(next),
                BarrierEffect::DriveControlTo(11),
            ]
        );
        assert_eq!(barrier.status, next);
        assert_eq!(
            barrier.remotes.get(&3),
            Some(&RemoteBarrierState::NotReady)
        );
        assert!(!barrier.is_running());
    }

    #[test]
    fn higher_remote_target_restarts_the_barrier_before_marking_sender_ready() {
        // A client that reaches a later tick forces the host to rebroadcast
        // the same state at that tick; the ACK's CtrlMode is ignored
        // (src/C4Network2.cpp:1513-1534).
        let mut barrier = StatusBarrier::stable(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 9,
        });
        barrier.set_remote_state(1, RemoteBarrierState::Ready);
        barrier.set_remote_state(2, RemoteBarrierState::Ready);
        let current = NetworkStatus {
            target_tick: 10,
            ..barrier.status
        };
        barrier.change_status(current);
        let later_ack = NetworkStatus {
            control_mode: 99,
            target_tick: 12,
            ..current
        };
        let retargeted = NetworkStatus {
            control_mode: 1,
            target_tick: 12,
            ..current
        };

        assert_eq!(
            barrier.remote_ack(1, later_ack),
            vec![
                BarrierEffect::InvalidateReference,
                BarrierEffect::BroadcastStatus(retargeted),
                BarrierEffect::DriveControlTo(12),
            ]
        );
        assert_eq!(barrier.status, retargeted);
        assert_eq!(
            barrier.remotes.get(&1),
            Some(&RemoteBarrierState::Ready)
        );
        assert_eq!(
            barrier.remotes.get(&2),
            Some(&RemoteBarrierState::NotReady)
        );
        assert_eq!(
            barrier.phase,
            BarrierPhase::Waiting {
                local_reached: false
            }
        );
    }
}
