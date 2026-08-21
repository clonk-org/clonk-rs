use clonk_network::{NetworkStatus, NETWORK_STATE_GO, NETWORK_STATE_PAUSE};

/// One client-side C++ game-status barrier awaiting local initialization.
#[derive(Debug, Default)]
pub struct ClientStartBarrier {
    pending: Option<PendingClientStart>,
}

#[derive(Debug)]
struct PendingClientStart {
    status: NetworkStatus,
    local_initialized: bool,
    runtime_join_chasing: bool,
}

fn same_barrier(left: NetworkStatus, right: NetworkStatus) -> bool {
    left.state == right.state && left.target_tick == right.target_tick
}

impl ClientStartBarrier {
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn is_runtime_join_chasing(&self, status: NetworkStatus) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.runtime_join_chasing && same_barrier(pending.status, status)
        })
    }

    /// Installs the JoinData status exactly as `HandleJoinData` does: it hands
    /// the status to `HandleStatus`, which clears both flags and leaves the
    /// client owing an acknowledgement. The host may replace it with a later
    /// chase-target `PID_Status`, but the client starts loading immediately
    /// from JoinData instead of waiting for that five-second timer. The
    /// reference form omits TargetTick, leaving the -1 default that lets
    /// `FinalInit`'s `CheckStatusReached(true)` reach an otherwise unchanged
    /// barrier while `Game.IsRunning` is still false
    /// (src/C4Network2.cpp:558-561,1501-1510,1574-1592,2017-2057,2161-2183).
    ///
    /// A lobby status opens nothing: `CheckStatusReached` gates GS_Lobby on
    /// `fLobbyRunning`, and the host's later Go request is the real barrier.
    pub fn from_join_data_status(status: NetworkStatus) -> Self {
        Self {
            pending: matches!(status.state, NETWORK_STATE_GO | NETWORK_STATE_PAUSE).then(|| {
                PendingClientStart {
                    status,
                    local_initialized: false,
                    runtime_join_chasing: true,
                }
            }),
        }
    }

    /// Opens preparation for a complete ordinary status request once.
    pub fn status_requested(&mut self, status: NetworkStatus) -> Option<NetworkStatus> {
        if status.target_tick < 0 || !matches!(status.state, NETWORK_STATE_GO | NETWORK_STATE_PAUSE)
        {
            return None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| same_barrier(pending.status, status))
        {
            return None;
        }
        let runtime_join_chasing = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.runtime_join_chasing);
        self.pending = Some(PendingClientStart {
            status,
            local_initialized: false,
            runtime_join_chasing,
        });
        Some(status)
    }

    /// Reports local initialization only when an ordinary status request has
    /// opened a barrier.
    pub fn local_initialized_at(&mut self, current_control_tick: i32) -> Option<NetworkStatus> {
        self.pending
            .as_mut()
            .filter(|pending| !pending.local_initialized)
            .map(|pending| {
                pending.local_initialized = true;
                // CheckStatusReached replaces the requested target with the
                // control tick the client actually reached before sending
                // PID_StatusAck (C4Network2.cpp:2041-2052). Keep the app-side
                // exact commit barrier on that same retargeted status.
                pending.status.target_tick = current_control_tick;
                pending.status
            })
    }

    /// Accepts a host commit only when an ordinary status request has opened a
    /// matching barrier and local initialization has reached it.
    pub fn status_committed(&mut self, status: NetworkStatus) -> Option<NetworkStatus> {
        self.pending
            .as_ref()
            .is_some_and(|pending| {
                pending.local_initialized && same_barrier(pending.status, status)
            })
            .then(|| {
                self.pending = None;
                status
            })
    }
}

#[cfg(test)]
mod tests {
    use clonk_network::{
        NetworkStatus, NETWORK_STATE_GO, NETWORK_STATE_LOBBY, NETWORK_STATE_PAUSE,
    };

    fn status(state: u8, control_mode: i32, target_tick: i32) -> NetworkStatus {
        NetworkStatus::new(state, control_mode, target_tick)
    }

    #[test]
    fn lobby_join_data_status_opens_no_start_barrier() {
        // HandleJoinData installs a lobby status too, but CheckStatusReached
        // gates GS_Lobby on fLobbyRunning rather than on a control target, so
        // a client joining into the lobby owes no acknowledgement until the
        // host's ChangeGameStatus requests Go (pristine 9ffa0a5d
        // src/C4Network2.cpp:1574-1592,2017-2040).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 2, -1));

        assert_eq!(barrier.local_initialized_at(0), None);
        assert_eq!(
            barrier.status_committed(status(NETWORK_STATE_LOBBY, 2, -1)),
            None
        );
    }

    #[test]
    fn runtime_join_go_status_opens_a_start_barrier() {
        // HandleJoinData installs the JoinData status itself and clears both
        // flags. If loading finishes before UpdateChaseTarget's five-second
        // replacement, FinalInit reaches this initial barrier while
        // Game.IsRunning is still false. The reference form's absent
        // TargetTick decompiles to -1, making CtrlTickReached(-1) trivially
        // true (7d43b47b src/C4Network2.cpp:558-561,1501-1510,1574-1592,
        // 2017-2057,2161-2183).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_GO, 2, -1));

        // CheckStatusReached retargets to the tick the client actually reached
        // before sending PID_StatusAck (src/C4Network2.cpp:2050-2052).
        assert_eq!(
            barrier.local_initialized_at(41),
            Some(status(NETWORK_STATE_GO, 2, 41)),
            "final init must reach the barrier the JoinData status opened"
        );
        assert_eq!(
            barrier.local_initialized_at(41),
            None,
            "and fStatusReached suppresses a repeat acknowledgement"
        );

        // The host echoes the client's own retargeted status back as the ack.
        assert_eq!(
            barrier.status_committed(status(NETWORK_STATE_GO, 2, 41)),
            Some(status(NETWORK_STATE_GO, 2, 41)),
            "the host ack for that retargeted status starts the joined client"
        );
    }

    #[test]
    fn ordinary_go_status_begins_preparation_only_once() {
        // HandleStatus installs the complete ordinary PID_Status target and
        // CheckStatusReached drives the client toward that target before an
        // acknowledgement is possible (pristine 9ffa0a5d
        // src/C4Network2.cpp:1501-1510,2017-2057).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));
        let requested = status(NETWORK_STATE_GO, 2, 41);

        assert_eq!(barrier.status_requested(requested), Some(requested));
        assert_eq!(barrier.status_requested(requested), None);
    }

    #[test]
    fn initialized_pause_barrier_emits_one_acknowledgement() {
        // Once CheckStatusReached reaches either Pause or Go, OnStatusReached
        // stops control and the client sends exactly one PID_StatusAck; its
        // fStatusReached guard suppresses repeats (pristine 9ffa0a5d
        // src/C4Network2.cpp:2017-2057,2085-2100).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));
        let requested = status(NETWORK_STATE_PAUSE, 2, 73);
        assert_eq!(barrier.status_requested(requested), Some(requested));

        assert_eq!(
            barrier.local_initialized_at(requested.target_tick),
            Some(requested)
        );
        assert_eq!(barrier.local_initialized_at(requested.target_tick), None);
    }

    #[test]
    fn exact_commit_waits_for_local_initialization() {
        // Client-side HandleStatusAck accepts the host commit only when the
        // target matches and fStatusReached is already set; an early commit is
        // deliberately ignored (pristine 9ffa0a5d
        // src/C4Network2.cpp:1536-1548).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));
        let requested = status(NETWORK_STATE_GO, 2, 73);
        assert_eq!(barrier.status_requested(requested), Some(requested));

        assert_eq!(barrier.status_committed(requested), None);
        assert_eq!(
            barrier.local_initialized_at(requested.target_tick),
            Some(requested)
        );
        assert_eq!(barrier.status_committed(requested), Some(requested));
    }

    #[test]
    fn commit_identity_is_state_and_target_not_control_mode() {
        // Client HandleStatusAck rejects a different state or target, but does
        // not compare CtrlMode before accepting the host commit (pristine
        // 9ffa0a5d src/C4Network2.cpp:1513-1519,1536-1548).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));
        let requested = status(NETWORK_STATE_GO, 2, 73);
        assert_eq!(barrier.status_requested(requested), Some(requested));
        assert_eq!(
            barrier.local_initialized_at(requested.target_tick),
            Some(requested)
        );

        assert_eq!(
            barrier.status_committed(status(NETWORK_STATE_PAUSE, 2, 73)),
            None
        );
        let host_commit = status(NETWORK_STATE_GO, 9, 73);
        assert_eq!(barrier.status_committed(host_commit), Some(host_commit));
    }

    #[test]
    fn exact_commit_consumes_the_pending_barrier() {
        // OnStatusAck sets fStatusAck and advances the client once; subsequent
        // packets do not re-enter that completed status transition (pristine
        // 9ffa0a5d src/C4Network2.cpp:1536-1548,2085-2110).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));
        let requested = status(NETWORK_STATE_GO, 2, 73);
        assert_eq!(barrier.status_requested(requested), Some(requested));
        assert_eq!(
            barrier.local_initialized_at(requested.target_tick),
            Some(requested)
        );

        assert_eq!(barrier.status_committed(requested), Some(requested));
        assert_eq!(barrier.status_committed(requested), None);
        assert_eq!(barrier.local_initialized_at(0), None);
    }

    #[test]
    fn omitted_reference_target_cannot_begin_preparation() {
        // A reference-form JoinData status has no TargetTick field and keeps
        // -1; only the later full PID_Status can provide a control target
        // (pristine 9ffa0a5d src/C4Network2.cpp:54-55,108-123,1501-1510).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));

        assert_eq!(
            barrier.status_requested(status(NETWORK_STATE_GO, 2, -1)),
            None
        );
        assert_eq!(barrier.local_initialized_at(0), None);
    }

    #[test]
    fn lobby_status_does_not_begin_game_preparation() {
        // CheckStatusReached treats Lobby as a running-lobby readiness check;
        // only Pause and Go drive the initialized game control toward a target
        // (pristine 9ffa0a5d src/C4Network2.cpp:2017-2040).
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));

        assert_eq!(
            barrier.status_requested(status(NETWORK_STATE_LOBBY, 2, 73)),
            None
        );
        assert_eq!(barrier.local_initialized_at(73), None);
    }

    #[test]
    fn local_initialization_retargets_the_exact_commit_barrier() {
        // A chasing client may have advanced beyond the host's requested
        // target. C++ sends its actual ControlTick and subsequently accepts
        // only a PID_StatusAck for that retargeted status.
        let mut barrier =
            super::ClientStartBarrier::from_join_data_status(status(NETWORK_STATE_LOBBY, 0, -1));
        let requested = status(NETWORK_STATE_GO, 2, 41);
        let reached = status(NETWORK_STATE_GO, 2, 44);
        assert_eq!(barrier.status_requested(requested), Some(requested));

        assert_eq!(barrier.local_initialized_at(44), Some(reached));
        assert_eq!(barrier.status_committed(requested), None);
        assert_eq!(barrier.status_committed(reached), Some(reached));
    }
}
