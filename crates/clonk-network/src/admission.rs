use crate::{ConnectionReply, ConnectionRequest};
use clonk_engine::{ClientCoreControlData, ClientJoinControlData, LegacyCString};
use std::collections::BTreeSet;

/// One C4Network2IO connection's acceptance phase
/// (`src/C4Network2IO.h:209-217,287-315`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    HalfAccepted,
    Accepted,
    Closed,
}

/// Side effects emitted by the pure connection acceptance reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionAction {
    SendRequest(ConnectionRequest),
    SendReply(ConnectionReply),
    EmitDirectClientJoin(ClientJoinControlData),
    RegisterHost(clonk_engine::ClientCoreControlData),
    AssociatePeer(ClientCoreControlData),
    Close {
        message: clonk_engine::LegacyCString,
        wrong_password: bool,
    },
}

/// One admission policy decision. The reducer owns the single ConnRe send;
/// policy supplies only the accepted core, actions which precede that reply,
/// and its exact message/flag data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionDecision {
    Accept {
        peer_core: clonk_engine::ClientCoreControlData,
        before_reply: Vec<ConnectionAction>,
        message: LegacyCString,
    },
    Reject {
        message: LegacyCString,
        wrong_password: bool,
    },
}

/// Client-owned policy for its initial host connection during `GS_Init`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientAdmission;

impl ClientAdmission {
    pub fn admit_host(request: &ConnectionRequest) -> AdmissionDecision {
        Self::admit_host_for_build(request, crate::CURRENT_GAME_BUILD)
    }

    /// Applies initial-host admission using the build selected for this
    /// client session.
    ///
    /// Reference-backed joins use the host's advertised build for their
    /// outbound `PID_Conn`; the reciprocal host request must be checked
    /// against that same value rather than this executable's native build.
    pub fn admit_host_for_build(
        request: &ConnectionRequest,
        compatibility_build: i32,
    ) -> AdmissionDecision {
        if request.build != compatibility_build {
            return rejection(
                format!(
                    "wrong engine ({}, I have {compatibility_build})",
                    request.build
                )
                .into_bytes(),
                false,
            );
        }
        if request.core.client_id != 0 {
            return rejection(b"not host".to_vec(), false);
        }
        AdmissionDecision::Accept {
            peer_core: request.core.clone(),
            before_reply: vec![ConnectionAction::RegisterHost(request.core.clone())],
            message: wire_string(b"host connection accepted"),
        }
    }
}

/// Policy for a connection whose client already exists in the synchronized
/// registry (`C4Network2::CheckConn`). Status-only differences are accepted;
/// ID, name or nick changes are not.
#[derive(Debug, Clone, Copy, Default)]
pub struct KnownPeerAdmission;

impl KnownPeerAdmission {
    pub fn admit(
        request: &ConnectionRequest,
        canonical_core: &ClientCoreControlData,
        already_connected: bool,
    ) -> AdmissionDecision {
        Self::admit_for_build(
            request,
            canonical_core,
            already_connected,
            crate::CURRENT_GAME_BUILD,
        )
    }

    /// Applies existing-peer admission using the build selected for this
    /// client session. Rust hosts continue to call [`Self::admit`], retaining
    /// their native build requirement.
    pub fn admit_for_build(
        request: &ConnectionRequest,
        canonical_core: &ClientCoreControlData,
        already_connected: bool,
        compatibility_build: i32,
    ) -> AdmissionDecision {
        if request.build != compatibility_build {
            return rejection(
                format!(
                    "wrong engine ({}, I have {compatibility_build})",
                    request.build
                )
                .into_bytes(),
                false,
            );
        }
        if already_connected {
            return AdmissionDecision::Accept {
                peer_core: canonical_core.clone(),
                before_reply: Vec::new(),
                message: wire_string(b"already connected"),
            };
        }
        if request.core.client_id != canonical_core.client_id
            || request.core.name != canonical_core.name
            || request.core.nick != canonical_core.nick
        {
            return rejection(b"wrong client core".to_vec(), false);
        }
        AdmissionDecision::Accept {
            peer_core: canonical_core.clone(),
            before_reply: Vec::new(),
            message: wire_string(b"connection accepted"),
        }
    }
}

/// Host-owned policy for admitting a new unknown-ID client.
#[derive(Debug, Clone)]
pub struct HostAdmission {
    next_client_id: i32,
    allow_join: bool,
    password: Option<LegacyCString>,
    used_names: BTreeSet<Vec<u8>>,
}

impl HostAdmission {
    pub fn new(
        next_client_id: i32,
        allow_join: bool,
        password: Option<LegacyCString>,
        used_names: impl IntoIterator<Item = LegacyCString>,
    ) -> Self {
        Self {
            next_client_id,
            allow_join,
            password,
            used_names: used_names
                .into_iter()
                .map(|name| name.as_bytes().to_vec())
                .collect(),
        }
    }

    pub fn next_client_id(&self) -> i32 {
        self.next_client_id
    }

    pub fn set_allow_join(&mut self, allow_join: bool) {
        self.allow_join = allow_join;
    }

    pub fn set_password(&mut self, password: Option<LegacyCString>) {
        self.password = password;
    }

    pub fn register_client_name(&mut self, name: &LegacyCString) {
        self.used_names.insert(name.as_bytes().to_vec());
    }

    pub fn remove_client_name(&mut self, name: &LegacyCString) {
        self.used_names.remove(name.as_bytes());
    }

    pub fn admit_new_peer(&mut self, request: &ConnectionRequest) -> AdmissionDecision {
        if request.build != 362 {
            return rejection(
                format!("wrong engine ({}, I have 362)", request.build).into_bytes(),
                false,
            );
        }
        if self
            .password
            .as_ref()
            .is_some_and(|password| password != &request.password)
        {
            return rejection(b"wrong password".to_vec(), true);
        }
        if !self.allow_join {
            return rejection(b"join denied".to_vec(), false);
        }
        if request.core.client_id != -1 {
            return rejection(b"join with set id not allowed".to_vec(), false);
        }

        let mut core = request.core.clone();
        core.client_id = self.next_client_id;
        self.next_client_id += 1;
        core.activated = false;
        core.observer = false;
        core.name = self.unique_name(&core.name);
        self.used_names.insert(core.name.as_bytes().to_vec());

        AdmissionDecision::Accept {
            peer_core: core.clone(),
            before_reply: vec![ConnectionAction::EmitDirectClientJoin(
                ClientJoinControlData { core, by_client: 0 },
            )],
            message: wire_string(b"join accepted"),
        }
    }

    fn unique_name(&self, name: &LegacyCString) -> LegacyCString {
        if !self.used_names.contains(name.as_bytes()) {
            return name.clone();
        }
        let mut suffix = 2i32;
        loop {
            let digits = suffix.to_string();
            let prefix_length = name
                .as_bytes()
                .len()
                .min(30usize.saturating_sub(digits.len()));
            let mut candidate = name.as_bytes()[..prefix_length].to_vec();
            candidate.extend_from_slice(digits.as_bytes());
            if !self.used_names.contains(&candidate) {
                return LegacyCString::from_bytes(candidate).unwrap_or_default();
            }
            suffix += 1;
        }
    }
}

fn wire_string(message: &[u8]) -> LegacyCString {
    LegacyCString::from_bytes(message.to_vec()).unwrap_or_default()
}

fn rejection(message: Vec<u8>, wrong_password: bool) -> AdmissionDecision {
    AdmissionDecision::Reject {
        message: LegacyCString::from_bytes(message).unwrap_or_default(),
        wrong_password,
    }
}

/// Models the two independently accepted halves of one LegacyClonk socket.
#[derive(Debug, Clone)]
pub struct LegacyConnection {
    local_request: ConnectionRequest,
    peer_core: Option<ClientCoreControlData>,
    remote_connection_id: Option<u32>,
    request_pending: bool,
    conn_sent: bool,
    pending_reply: Option<ConnectionReply>,
    status: ConnectionStatus,
}

impl LegacyConnection {
    pub fn new(local_request: ConnectionRequest) -> Self {
        Self {
            local_request,
            peer_core: None,
            remote_connection_id: None,
            request_pending: false,
            conn_sent: false,
            pending_reply: None,
            status: ConnectionStatus::Connected,
        }
    }

    pub fn with_known_peer(
        local_request: ConnectionRequest,
        peer_core: ClientCoreControlData,
    ) -> Self {
        let mut connection = Self::new(local_request);
        connection.peer_core = Some(peer_core);
        connection
    }

    pub fn status(&self) -> ConnectionStatus {
        self.status
    }

    pub fn remote_connection_id(&self) -> Option<u32> {
        self.remote_connection_id
    }

    pub fn on_socket_open(&mut self) -> Vec<ConnectionAction> {
        if self.request_pending || self.conn_sent || self.status == ConnectionStatus::Closed {
            return Vec::new();
        }
        self.request_pending = true;
        vec![ConnectionAction::SendRequest(self.local_request.clone())]
    }

    pub fn on_request_sent(&mut self, succeeded: bool) -> Vec<ConnectionAction> {
        if !self.request_pending || self.status == ConnectionStatus::Closed {
            return Vec::new();
        }
        self.request_pending = false;
        if succeeded {
            self.conn_sent = true;
            Vec::new()
        } else {
            self.status = ConnectionStatus::Closed;
            vec![ConnectionAction::Close {
                message: LegacyCString::default(),
                wrong_password: false,
            }]
        }
    }

    pub fn accept_peer_request<F>(
        &mut self,
        mut request: ConnectionRequest,
        decide: F,
    ) -> Vec<ConnectionAction>
    where
        F: FnOnce(&ConnectionRequest) -> AdmissionDecision,
    {
        if self.status == ConnectionStatus::Closed {
            return Vec::new();
        }
        self.remote_connection_id = Some(request.connection_id);
        if self.pending_reply.is_some() {
            return Vec::new();
        }
        let decision = decide(&request);
        let (reply, mut actions) = match decision {
            AdmissionDecision::Accept {
                peer_core,
                before_reply,
                message,
            } => {
                request.core = peer_core;
                self.peer_core = Some(request.core);
                if matches!(
                    self.status,
                    ConnectionStatus::HalfAccepted | ConnectionStatus::Accepted
                ) {
                    return Vec::new();
                }
                (
                    ConnectionReply {
                        ok: true,
                        message,
                        wrong_password: false,
                        port_protocol: true,
                    },
                    before_reply,
                )
            }
            AdmissionDecision::Reject {
                message,
                wrong_password,
            } => (
                ConnectionReply {
                    ok: false,
                    message,
                    wrong_password,
                    port_protocol: false,
                },
                Vec::new(),
            ),
        };
        self.pending_reply = Some(reply.clone());
        actions.push(ConnectionAction::SendReply(reply));
        actions
    }

    pub fn on_reply_sent(&mut self, succeeded: bool) -> Vec<ConnectionAction> {
        if self.status == ConnectionStatus::Closed {
            self.pending_reply = None;
            return Vec::new();
        }
        let Some(reply) = self.pending_reply.take() else {
            return Vec::new();
        };
        if !succeeded {
            return Vec::new();
        }
        if reply.ok {
            if self.status != ConnectionStatus::Accepted {
                self.status = ConnectionStatus::HalfAccepted;
            }
            Vec::new()
        } else {
            self.status = ConnectionStatus::Closed;
            vec![ConnectionAction::Close {
                message: reply.message,
                wrong_password: reply.wrong_password,
            }]
        }
    }

    pub fn receive_reply(&mut self, reply: ConnectionReply) -> Vec<ConnectionAction> {
        if self.status == ConnectionStatus::Closed {
            return Vec::new();
        }
        if !self.conn_sent {
            self.status = ConnectionStatus::Closed;
            return vec![ConnectionAction::Close {
                message: clonk_engine::LegacyCString::default(),
                wrong_password: false,
            }];
        }
        let Some(peer_core) = self.peer_core.clone() else {
            self.status = ConnectionStatus::Closed;
            return vec![ConnectionAction::Close {
                message: LegacyCString::default(),
                wrong_password: false,
            }];
        };
        if !reply.ok {
            self.status = ConnectionStatus::Closed;
            return vec![ConnectionAction::Close {
                message: reply.message,
                wrong_password: reply.wrong_password,
            }];
        }
        if self.status == ConnectionStatus::Accepted {
            return Vec::new();
        }
        self.status = ConnectionStatus::Accepted;
        vec![ConnectionAction::AssociatePeer(peer_core)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_engine::{ClientCoreControlData, LegacyCString};

    const CPP_COMPATIBILITY_BUILD: i32 = crate::CURRENT_GAME_BUILD + 2;

    fn request(client_id: i32, connection_id: u32) -> ConnectionRequest {
        ConnectionRequest {
            core: ClientCoreControlData {
                client_id,
                name: LegacyCString::from_bytes(b"Peer".to_vec()).unwrap(),
                nick: LegacyCString::from_bytes(b"Peer".to_vec()).unwrap(),
                ..Default::default()
            },
            build: 362,
            password: LegacyCString::default(),
            connection_id,
            port_protocol: false,
        }
    }

    fn accepted(message: &[u8]) -> ConnectionReply {
        ConnectionReply {
            ok: true,
            message: LegacyCString::from_bytes(message.to_vec()).unwrap(),
            wrong_password: false,
            port_protocol: true,
        }
    }

    fn accepted_decision(request: &ConnectionRequest, message: &[u8]) -> AdmissionDecision {
        AdmissionDecision::Accept {
            peer_core: request.core.clone(),
            before_reply: Vec::new(),
            message: LegacyCString::from_bytes(message.to_vec()).unwrap(),
        }
    }

    #[test]
    fn mutual_connection_reaches_accepted_after_positive_reply() {
        // Both endpoints send PID_Conn on socket-open. Accepting the peer's
        // Conn produces HalfAccepted; a positive reply associates the known
        // canonical peer and produces Accepted
        // (src/C4Network2IO.cpp:478-525,965-1006,1223-1254;
        // src/C4Network2.cpp:1448-1499).
        let local = request(0, 7);
        let peer = request(-1, 11);
        let mut connection = LegacyConnection::new(local.clone());

        assert_eq!(connection.status(), ConnectionStatus::Connected);
        assert_eq!(
            connection.on_socket_open(),
            vec![ConnectionAction::SendRequest(local)]
        );
        assert_eq!(connection.status(), ConnectionStatus::Connected);
        assert!(connection.on_request_sent(true).is_empty());

        let reply = accepted(b"join accepted");
        let decision = accepted_decision(&peer, b"join accepted");
        assert_eq!(
            connection.accept_peer_request(peer.clone(), |_| decision),
            vec![ConnectionAction::SendReply(reply)]
        );
        assert_eq!(connection.status(), ConnectionStatus::Connected);
        assert!(connection.on_reply_sent(true).is_empty());
        assert_eq!(connection.status(), ConnectionStatus::HalfAccepted);
        assert_eq!(connection.remote_connection_id(), Some(11));

        assert_eq!(
            connection.receive_reply(accepted(b"host connection accepted")),
            vec![ConnectionAction::AssociatePeer(peer.core)]
        );
        assert_eq!(connection.status(), ConnectionStatus::Accepted);
    }

    #[test]
    fn reply_before_local_connection_request_closes_as_fishy() {
        // PID_ConnRe before ConnSent is treated as fishy and closes without
        // processing the reply payload (src/C4Network2IO.cpp:987-998).
        let mut connection = LegacyConnection::new(request(0, 7));

        assert_eq!(
            connection.receive_reply(accepted(b"premature")),
            vec![ConnectionAction::Close {
                message: LegacyCString::default(),
                wrong_password: false,
            }]
        );
        assert_eq!(connection.status(), ConnectionStatus::Closed);
        assert!(connection
            .receive_reply(accepted(b"late duplicate"))
            .is_empty());
        let peer = request(-1, 11);
        assert!(connection
            .accept_peer_request(peer, |_| panic!("closed connection invoked policy"))
            .is_empty());
        assert_eq!(connection.status(), ConnectionStatus::Closed);
    }

    #[test]
    fn host_assigns_and_renames_before_sending_positive_reply() {
        // C4Network2::Join assigns the ID, forces inactive/non-observer state,
        // uniquifies the name and executes direct ClientJoin before HandleConn
        // sends "join accepted" (src/C4Network2.cpp:1316-1352,1395-1445).
        let mut incoming = request(-1, 11);
        incoming.core.activated = true;
        incoming.core.observer = true;
        let mut host = HostAdmission::new(
            3,
            true,
            None,
            [LegacyCString::from_bytes(b"Peer".to_vec()).unwrap()],
        );

        let AdmissionDecision::Accept {
            peer_core,
            before_reply,
            message,
        } = host.admit_new_peer(&incoming)
        else {
            panic!("expected accepted host admission");
        };
        let [ConnectionAction::EmitDirectClientJoin(join)] = before_reply.as_slice() else {
            panic!("expected one pre-reply ClientJoin, got {before_reply:?}");
        };
        assert_eq!((join.core.client_id, join.by_client), (3, 0));
        assert!(!join.core.activated);
        assert!(!join.core.observer);
        assert_eq!(join.core.name.as_bytes(), b"Peer2");
        assert_eq!(peer_core, join.core);
        assert_eq!(message.as_bytes(), b"join accepted");
        assert_eq!(host.next_client_id(), 4);
    }

    #[test]
    fn host_rejections_match_cpp_messages_flags_and_do_not_consume_id() {
        // HandleConn checks build, then password, then Join's allow/id gates;
        // rejected joins never increment iNextClientID
        // (src/C4Network2.cpp:1282-1363,1395-1405).
        let secret = LegacyCString::from_bytes(b"secret".to_vec()).unwrap();
        let mut host = HostAdmission::new(3, true, Some(secret.clone()), []);

        let mut wrong_build = request(-1, 11);
        wrong_build.build = 361;
        assert_reply(
            &host.admit_new_peer(&wrong_build),
            false,
            b"wrong engine (361, I have 362)",
            false,
        );

        assert_reply(
            &host.admit_new_peer(&request(-1, 12)),
            false,
            b"wrong password",
            true,
        );

        let mut set_id = request(9, 13);
        set_id.password = secret.clone();
        assert_reply(
            &host.admit_new_peer(&set_id),
            false,
            b"join with set id not allowed",
            false,
        );
        assert_eq!(host.next_client_id(), 3);

        let mut denied_host = HostAdmission::new(8, false, Some(secret), []);
        let mut denied = request(-1, 14);
        denied.password = LegacyCString::from_bytes(b"secret".to_vec()).unwrap();
        assert_reply(
            &denied_host.admit_new_peer(&denied),
            false,
            b"join denied",
            false,
        );
        assert_eq!(denied_host.next_client_id(), 8);
    }

    #[test]
    fn host_password_changes_apply_to_subsequent_admission_attempts() {
        let mut host = HostAdmission::new(3, true, None, []);

        assert!(matches!(
            host.admit_new_peer(&request(-1, 11)),
            AdmissionDecision::Accept { .. }
        ));

        let secret = LegacyCString::from_bytes(b"secret".to_vec()).unwrap();
        host.set_password(Some(secret.clone()));
        assert_reply(
            &host.admit_new_peer(&request(-1, 12)),
            false,
            b"wrong password",
            true,
        );
        let mut authenticated = request(-1, 13);
        authenticated.password = secret;
        assert!(matches!(
            host.admit_new_peer(&authenticated),
            AdmissionDecision::Accept { .. }
        ));

        host.set_password(None);
        assert!(matches!(
            host.admit_new_peer(&request(-1, 14)),
            AdmissionDecision::Accept { .. }
        ));
    }

    #[test]
    fn positive_duplicates_are_ignored_after_acceptance() {
        // HandleConn skips an already half-accepted successful request and
        // HandleConnRe ignores a positive reply on a fully accepted normal
        // connection (src/C4Network2.cpp:1337-1340,1472-1474).
        let local = request(0, 7);
        let peer = request(-1, 11);
        let mut connection = LegacyConnection::new(local);
        connection.on_socket_open();
        connection.on_request_sent(true);
        let decision = accepted_decision(&peer, b"join accepted");
        connection.accept_peer_request(peer.clone(), |_| decision);
        connection.on_reply_sent(true);
        connection.receive_reply(accepted(b"host connection accepted"));

        let duplicate_decision = accepted_decision(&peer, b"already connected");
        assert!(connection
            .accept_peer_request(peer, |_| duplicate_decision)
            .is_empty());
        assert!(connection
            .receive_reply(accepted(b"already connected"))
            .is_empty());
        assert_eq!(connection.status(), ConnectionStatus::Accepted);
    }

    #[test]
    fn client_registers_only_id_zero_host_before_positive_reply() {
        // During GS_Init the client accepts its first peer only when that
        // C4ClientCore is the host (ID 0), creates the provisional host client,
        // then replies "host connection accepted"
        // (src/C4Network2.cpp:1305-1315,1383-1392).
        let host_request = request(0, 7);
        let AdmissionDecision::Accept {
            peer_core,
            before_reply,
            message,
        } = ClientAdmission::admit_host(&host_request)
        else {
            panic!("expected host acceptance");
        };
        assert_eq!(peer_core, host_request.core);
        assert_eq!(
            before_reply,
            vec![ConnectionAction::RegisterHost(host_request.core.clone())]
        );
        assert_eq!(message.as_bytes(), b"host connection accepted");

        let non_host = request(2, 8);
        assert_reply(
            &ClientAdmission::admit_host(&non_host),
            false,
            b"not host",
            false,
        );
    }

    #[test]
    fn client_admission_uses_the_selected_compatibility_build() {
        // A reciprocal C++ PID_Conn carries C4XVERBUILD and is checked by the
        // same exact-build branch (oracle-src-pinned
        // src/C4Network2IO.cpp:1611-1618; src/C4Network2.cpp:1291-1299).
        let mut matching_host = request(0, 7);
        matching_host.build = CPP_COMPATIBILITY_BUILD;
        assert!(matches!(
            ClientAdmission::admit_host_for_build(&matching_host, CPP_COMPATIBILITY_BUILD),
            AdmissionDecision::Accept { .. }
        ));

        let mut stale_host = matching_host;
        stale_host.build = CPP_COMPATIBILITY_BUILD - 1;
        let expected = format!(
            "wrong engine ({}, I have {CPP_COMPATIBILITY_BUILD})",
            CPP_COMPATIBILITY_BUILD - 1
        );
        assert_reply(
            &ClientAdmission::admit_host_for_build(&stale_host, CPP_COMPATIBILITY_BUILD),
            false,
            expected.as_bytes(),
            false,
        );
    }

    #[test]
    fn composed_host_admission_associates_assigned_core_and_confirms_sends() {
        // Join replaces the unknown core with its assigned core before
        // HandleConnRe associates the socket. ConnSent and HalfAccepted are
        // updated only after their respective sends succeed
        // (src/C4Network2.cpp:1327-1355,1483-1488;
        // src/C4Network2IO.cpp:1241-1249).
        let local = request(0, 7);
        let incoming = request(-1, 11);
        let mut host = HostAdmission::new(3, true, None, []);
        let mut connection = LegacyConnection::new(local.clone());

        assert_eq!(
            connection.on_socket_open(),
            vec![ConnectionAction::SendRequest(local)]
        );
        assert_eq!(connection.status(), ConnectionStatus::Connected);
        assert!(connection.on_request_sent(true).is_empty());

        let actions =
            connection.accept_peer_request(incoming, |request| host.admit_new_peer(request));
        assert!(matches!(
            actions.as_slice(),
            [
                ConnectionAction::EmitDirectClientJoin(_),
                ConnectionAction::SendReply(_)
            ]
        ));
        assert_eq!(connection.status(), ConnectionStatus::Connected);
        assert!(connection.on_reply_sent(true).is_empty());
        assert_eq!(connection.status(), ConnectionStatus::HalfAccepted);

        let association = connection.receive_reply(accepted(b"host connection accepted"));
        let [ConnectionAction::AssociatePeer(assigned)] = association.as_slice() else {
            panic!("expected assigned peer association");
        };
        assert_eq!(assigned.client_id, 3);
        assert_eq!(connection.status(), ConnectionStatus::Accepted);
    }

    #[test]
    fn positive_peer_reply_accepts_as_soon_as_canonical_peer_is_known() {
        // HandleConnRe requires a registered pClient, not HalfAccepted. It may
        // therefore associate a known peer while our positive ConnRe send is
        // still pending (src/C4Network2.cpp:1448-1499).
        let local = request(0, 7);
        let peer = request(-1, 11);
        let mut connection = LegacyConnection::new(local);
        connection.on_socket_open();
        connection.on_request_sent(true);
        let decision = accepted_decision(&peer, b"join accepted");
        connection.accept_peer_request(peer.clone(), |_| decision);

        assert_eq!(
            connection.receive_reply(accepted(b"host connection accepted")),
            vec![ConnectionAction::AssociatePeer(peer.core)]
        );
        assert_eq!(connection.status(), ConnectionStatus::Accepted);
        assert!(connection.on_reply_sent(true).is_empty());
        assert_eq!(connection.status(), ConnectionStatus::Accepted);
    }

    #[test]
    fn known_peer_can_accept_reply_before_receiving_peer_conn() {
        // Outbound connections already carry a registry-owned client. C++
        // accepts their positive ConnRe without first receiving that socket's
        // Conn or entering HalfAccepted (src/C4Network2.cpp:1448-1499).
        let local = request(0, 7);
        let peer = request(3, 11);
        let mut connection = LegacyConnection::with_known_peer(local, peer.core.clone());
        connection.on_socket_open();
        connection.on_request_sent(true);

        assert_eq!(
            connection.receive_reply(accepted(b"connection accepted")),
            vec![ConnectionAction::AssociatePeer(peer.core)]
        );
        assert_eq!(connection.status(), ConnectionStatus::Accepted);
    }

    #[test]
    fn known_peer_policy_matches_cpp_core_diff_and_already_connected_order() {
        let canonical = request(3, 11).core;
        let mut status_only = request(3, 12);
        status_only.core.activated = true;
        let AdmissionDecision::Accept { message, .. } =
            KnownPeerAdmission::admit(&status_only, &canonical, false)
        else {
            panic!("status-only differences should be accepted");
        };
        assert_eq!(message.as_bytes(), b"connection accepted");

        let mut wrong_core = status_only;
        wrong_core.core.name = LegacyCString::from_bytes(b"Impostor".to_vec()).unwrap();
        assert_reply(
            &KnownPeerAdmission::admit(&wrong_core, &canonical, false),
            false,
            b"wrong client core",
            false,
        );

        let AdmissionDecision::Accept { message, .. } =
            KnownPeerAdmission::admit(&wrong_core, &canonical, true)
        else {
            panic!("already-associated socket is accepted before core comparison");
        };
        assert_eq!(message.as_bytes(), b"already connected");
    }

    #[test]
    fn known_peer_admission_uses_the_selected_compatibility_build() {
        // CheckConn applies the version branch before existing-client identity
        // checks (oracle-src-pinned src/C4Network2.cpp:1286-1303).
        let canonical = request(3, 11).core;
        let mut matching_peer = request(3, 12);
        matching_peer.build = CPP_COMPATIBILITY_BUILD;
        assert!(matches!(
            KnownPeerAdmission::admit_for_build(
                &matching_peer,
                &canonical,
                false,
                CPP_COMPATIBILITY_BUILD,
            ),
            AdmissionDecision::Accept { .. }
        ));

        let mut stale_peer = matching_peer;
        stale_peer.build = CPP_COMPATIBILITY_BUILD - 1;
        let expected = format!(
            "wrong engine ({}, I have {CPP_COMPATIBILITY_BUILD})",
            CPP_COMPATIBILITY_BUILD - 1
        );
        assert_reply(
            &KnownPeerAdmission::admit_for_build(
                &stale_peer,
                &canonical,
                false,
                CPP_COMPATIBILITY_BUILD,
            ),
            false,
            expected.as_bytes(),
            false,
        );
    }

    #[test]
    fn changed_core_duplicate_is_rejected_even_after_half_acceptance() {
        // HandleConn reruns CheckConn. Only a successful duplicate is skipped;
        // a changed identity receives a negative reply and closes the socket
        // (src/C4Network2.cpp:1282-1363).
        let local = request(0, 7);
        let peer = request(3, 11);
        let canonical = peer.core.clone();
        let mut connection = LegacyConnection::with_known_peer(local, canonical.clone());
        connection.on_socket_open();
        connection.on_request_sent(true);
        connection.accept_peer_request(peer, |request| {
            KnownPeerAdmission::admit(request, &canonical, false)
        });
        connection.on_reply_sent(true);

        let mut changed = request(3, 12);
        changed.core.nick = LegacyCString::from_bytes(b"Changed".to_vec()).unwrap();
        let actions = connection.accept_peer_request(changed, |request| {
            KnownPeerAdmission::admit(request, &canonical, false)
        });
        assert!(matches!(
            actions.as_slice(),
            [ConnectionAction::SendReply(ConnectionReply {
                ok: false,
                ..
            })]
        ));
        assert!(matches!(
            connection.on_reply_sent(true).as_slice(),
            [ConnectionAction::Close { .. }]
        ));
        assert_eq!(connection.status(), ConnectionStatus::Closed);
    }

    #[test]
    fn removed_client_name_becomes_available_for_a_later_join() {
        // Join queries the live C4ClientList for every candidate; a removed
        // client's name is therefore reusable (src/C4Network2.cpp:1406-1432;
        // src/C4Client.cpp:186-192,226-255).
        let peer = LegacyCString::from_bytes(b"Peer".to_vec()).unwrap();
        let mut host = HostAdmission::new(3, true, None, [peer.clone()]);
        host.remove_client_name(&peer);

        let AdmissionDecision::Accept { peer_core, .. } = host.admit_new_peer(&request(-1, 11))
        else {
            panic!("expected accepted join");
        };
        assert_eq!(peer_core.name.as_bytes(), b"Peer");
    }

    fn assert_reply(decision: &AdmissionDecision, ok: bool, message: &[u8], wrong_password: bool) {
        let AdmissionDecision::Reject {
            message: actual,
            wrong_password: actual_wrong_password,
        } = decision
        else {
            panic!("expected rejection, got {decision:?}");
        };
        assert_eq!(
            (false, actual.as_bytes(), *actual_wrong_password),
            (ok, message, wrong_password),
        );
    }
}
