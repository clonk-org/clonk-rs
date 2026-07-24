use std::collections::BTreeMap;

use crate::{ClientId, NetworkStatus, NETWORK_STATE_GO, NETWORK_STATE_LOBBY, NETWORK_STATE_PAUSE};

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
    ExecutePendingSyncControls(i32),
    BroadcastStatusAck(NetworkStatus),
    SendStatusAck {
        client_id: ClientId,
        status: NetworkStatus,
    },
    SetControlMode {
        mode: i32,
        from_tick: i32,
    },
    SweepUnjoinedPlayers,
    StartControl,
}

/// Deterministic host-side projection of the C++ status barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBarrier {
    pub status: NetworkStatus,
    pub phase: BarrierPhase,
    /// The host's own client-list state. `ResetReady` includes the local
    /// client, but `AllClientsReady` skips it, so a status change leaves this
    /// `NotReady` even after the host reaches and commits the barrier.
    pub local: RemoteBarrierState,
    pub remotes: BTreeMap<ClientId, RemoteBarrierState>,
    local_reached_tick: Option<i32>,
}

impl StatusBarrier {
    pub fn stable(status: NetworkStatus) -> Self {
        Self {
            status,
            phase: BarrierPhase::Stable,
            local: RemoteBarrierState::Ready,
            remotes: BTreeMap::new(),
            local_reached_tick: None,
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
        if matches!(
            self.local,
            RemoteBarrierState::Ready | RemoteBarrierState::NotReady
        ) {
            self.local = RemoteBarrierState::NotReady;
        }
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
        self.local_reached_tick = None;
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
        self.local_reached_at(self.status.target_tick)
    }

    fn local_reached_at(&mut self, actual_control_tick: i32) -> Vec<BarrierEffect> {
        let BarrierPhase::Waiting { local_reached } = &mut self.phase else {
            return Vec::new();
        };
        if *local_reached {
            return self.try_commit();
        }
        *local_reached = true;
        self.local_reached_tick = Some(actual_control_tick);
        let mut effects = vec![BarrierEffect::StopControl];
        effects.extend(self.try_commit());
        effects
    }

    /// Applies a runtime-owned local arrival only if the host barrier has not
    /// been retargeted since that runtime began driving toward it.
    pub fn local_reached_for(
        &mut self,
        expected: NetworkStatus,
        actual_control_tick: i32,
    ) -> Vec<BarrierEffect> {
        if expected.state != self.status.state || expected.target_tick != self.status.target_tick {
            return Vec::new();
        }
        self.local_reached_at(actual_control_tick)
    }

    pub fn remote_ack(
        &mut self,
        client_id: ClientId,
        acknowledgement: NetworkStatus,
    ) -> Vec<BarrierEffect> {
        if !self.remote_ack_is_acceptable(client_id, acknowledgement) {
            return Vec::new();
        }

        if self.phase == BarrierPhase::Stable {
            self.remotes.insert(client_id, RemoteBarrierState::Ready);
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
            self.remotes.insert(client_id, RemoteBarrierState::Ready);
            return effects;
        }

        self.remotes.insert(client_id, RemoteBarrierState::Ready);
        self.try_commit()
    }

    /// Mirrors the security/matching guard in
    /// `C4Network2::HandleStatusAck` before any remote is marked ready.
    pub fn remote_ack_is_acceptable(
        &self,
        client_id: ClientId,
        acknowledgement: NetworkStatus,
    ) -> bool {
        let Some(state) = self.remotes.get(&client_id).copied() else {
            return false;
        };
        !matches!(
            state,
            RemoteBarrierState::Joining | RemoteBarrierState::Removing
        ) && acknowledgement.state == self.status.state
            && acknowledgement.target_tick >= self.status.target_tick
    }

    /// Whether an acceptable acknowledgement advances authoritative host
    /// state. Duplicate ready acknowledgements still receive the C++ wire ACK
    /// response, but need not amplify into repeated application events.
    pub fn remote_ack_changes_state(
        &self,
        client_id: ClientId,
        acknowledgement: NetworkStatus,
    ) -> bool {
        self.remote_ack_is_acceptable(client_id, acknowledgement)
            && (acknowledgement.target_tick > self.status.target_tick
                || self.remotes.get(&client_id) != Some(&RemoteBarrierState::Ready))
    }

    pub fn sync(&mut self, next_control_tick: i32) -> Vec<BarrierEffect> {
        if matches!(self.phase, BarrierPhase::Waiting { .. }) {
            return self.try_commit();
        }
        if self.status.state == NETWORK_STATE_LOBBY || self.status.state == NETWORK_STATE_PAUSE {
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

    pub fn is_frozen(&self) -> bool {
        self.status.state == NETWORK_STATE_LOBBY
            || self.status.state == NETWORK_STATE_PAUSE && self.phase == BarrierPhase::Stable
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
        let sync_control_tick = self.local_reached_tick.unwrap_or(self.status.target_tick);
        let mut effects = vec![
            BarrierEffect::ExecutePendingSyncControls(sync_control_tick),
            BarrierEffect::BroadcastStatusAck(self.status),
        ];
        if self.status.state == NETWORK_STATE_GO {
            effects.extend([
                BarrierEffect::SetControlMode {
                    mode: self.status.control_mode,
                    from_tick: sync_control_tick,
                },
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
        // ChangeGameStatus resets the local client and every waited-for
        // remote. CheckStatusAck may commit only after the host reaches the
        // target and all such remotes acknowledge; AllClientsReady skips the
        // local list entry, so it deliberately stays NotReady afterward
        // (src/C4Network2.cpp:1994-2014,2062-2110).
        let mut barrier = StatusBarrier::stable(NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: -1,
        });
        barrier.set_remote_state(1, RemoteBarrierState::Ready);
        barrier.set_remote_state(2, RemoteBarrierState::Ready);
        assert_eq!(barrier.local, RemoteBarrierState::Ready);
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
        assert_eq!(barrier.local, RemoteBarrierState::NotReady);
        assert_eq!(barrier.remote_ack(1, go), Vec::new());
        assert_eq!(barrier.local_reached(), vec![BarrierEffect::StopControl]);
        assert!(!barrier.is_running());
        assert_eq!(
            barrier.remote_ack(2, go),
            vec![
                BarrierEffect::ExecutePendingSyncControls(10),
                BarrierEffect::BroadcastStatusAck(go),
                BarrierEffect::SetControlMode {
                    mode: 1,
                    from_tick: 10,
                },
                BarrierEffect::SweepUnjoinedPlayers,
                BarrierEffect::StartControl,
            ]
        );
        assert!(barrier.is_running());
        assert_eq!(barrier.phase, BarrierPhase::Stable);
        assert_eq!(barrier.local, RemoteBarrierState::NotReady);
    }

    #[test]
    fn go_mode_switch_replays_from_the_hosts_actual_reached_tick() {
        let lobby = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: -1,
        };
        let go = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 10,
        };
        let mut barrier = StatusBarrier::stable(lobby);
        barrier.change_status(go);

        assert_eq!(
            barrier.local_reached_for(go, 12),
            vec![
                BarrierEffect::StopControl,
                BarrierEffect::ExecutePendingSyncControls(12),
                BarrierEffect::BroadcastStatusAck(go),
                BarrierEffect::SetControlMode {
                    mode: 1,
                    from_tick: 12,
                },
                BarrierEffect::SweepUnjoinedPlayers,
                BarrierEffect::StartControl,
            ]
        );
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
        assert_eq!(barrier.remotes.get(&3), Some(&RemoteBarrierState::NotReady));
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
        assert_eq!(barrier.remotes.get(&1), Some(&RemoteBarrierState::Ready));
        assert_eq!(barrier.remotes.get(&2), Some(&RemoteBarrierState::NotReady));
        assert_eq!(
            barrier.phase,
            BarrierPhase::Waiting {
                local_reached: false
            }
        );
    }

    #[test]
    fn stale_local_reach_cannot_complete_a_retargeted_barrier() {
        let mut barrier = StatusBarrier::stable(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 3,
        });
        let original = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 10,
        };
        barrier.change_status(original);
        let retargeted = NetworkStatus {
            target_tick: 12,
            ..original
        };
        barrier.change_status(retargeted);

        assert!(barrier.local_reached_for(original, 10).is_empty());
        assert_eq!(
            barrier.phase,
            BarrierPhase::Waiting {
                local_reached: false
            }
        );
        assert_eq!(
            barrier.local_reached_for(retargeted, 14),
            vec![
                BarrierEffect::StopControl,
                BarrierEffect::ExecutePendingSyncControls(14),
                BarrierEffect::BroadcastStatusAck(retargeted),
            ]
        );
    }

    #[test]
    fn stale_or_duplicate_status_ack_is_not_an_authoritative_transition() {
        let lobby = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: -1,
        };
        let go = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 20,
        };
        let mut barrier = StatusBarrier::stable(lobby);
        barrier.set_remote_state(7, RemoteBarrierState::Ready);
        barrier.change_status(go);

        let wrong_state = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            ..go
        };
        let stale_tick = NetworkStatus {
            target_tick: 19,
            ..go
        };
        assert!(!barrier.remote_ack_is_acceptable(7, wrong_state));
        assert!(!barrier.remote_ack_is_acceptable(7, stale_tick));
        assert!(barrier.remote_ack(7, wrong_state).is_empty());
        assert!(barrier.remote_ack(7, stale_tick).is_empty());
        assert_eq!(barrier.remotes.get(&7), Some(&RemoteBarrierState::NotReady));

        assert!(barrier.remote_ack_changes_state(7, go));
        barrier.remote_ack(7, go);
        assert!(!barrier.remote_ack_changes_state(7, go));
    }
}
