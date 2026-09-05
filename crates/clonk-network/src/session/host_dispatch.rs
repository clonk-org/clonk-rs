//! Host packet dispatch: client messages, forwarding, control ingest/resync, broadcasts, membership.
//!
//! This child module shares the parent session's private protocol machinery;
//! `session.rs` re-exports its crate-facing surface under the original paths.

use super::*;

#[cfg(test)]
struct HostCapabilityDispatchPause {
    token: Arc<()>,
    reached: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

#[cfg(test)]
pub(crate) struct HostCapabilityDispatchPauseGuard {
    client_name: Vec<u8>,
    token: Arc<()>,
}

#[cfg(test)]
fn host_capability_dispatch_pauses(
) -> &'static Mutex<BTreeMap<Vec<u8>, HostCapabilityDispatchPause>> {
    static PAUSES: std::sync::OnceLock<Mutex<BTreeMap<Vec<u8>, HostCapabilityDispatchPause>>> =
        std::sync::OnceLock::new();
    PAUSES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
pub(crate) fn pause_host_capability_dispatch(
    client_name: &[u8],
) -> (
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    HostCapabilityDispatchPauseGuard,
) {
    let (reached_tx, reached_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    let client_name = client_name.to_vec();
    let token = Arc::new(());
    let mut pauses = host_capability_dispatch_pauses()
        .lock()
        .expect("host capability dispatch pause lock poisoned");
    let already_installed = pauses.contains_key(&client_name);
    if already_installed {
        drop(pauses);
        panic!("host capability dispatch pause already installed for this client");
    }
    pauses.insert(
        client_name.clone(),
        HostCapabilityDispatchPause {
            token: Arc::clone(&token),
            reached: reached_tx,
            resume: resume_rx,
        },
    );
    (
        reached_rx,
        resume_tx,
        HostCapabilityDispatchPauseGuard { client_name, token },
    )
}

#[cfg(test)]
impl Drop for HostCapabilityDispatchPauseGuard {
    fn drop(&mut self) {
        let mut pauses = host_capability_dispatch_pauses()
            .lock()
            .expect("host capability dispatch pause lock poisoned");
        if pauses
            .get(&self.client_name)
            .is_some_and(|pause| Arc::ptr_eq(&pause.token, &self.token))
        {
            pauses.remove(&self.client_name);
        }
    }
}

#[cfg(test)]
async fn wait_at_host_capability_dispatch_pause(state: &HostState, client_id: ClientId) {
    let Some(client_name) = state
        .client_cores
        .get(&(client_id as i32))
        .map(|core| core.name.as_bytes().to_vec())
    else {
        return;
    };
    let pause = host_capability_dispatch_pauses()
        .lock()
        .expect("host capability dispatch pause lock poisoned")
        .remove(&client_name);
    let Some(HostCapabilityDispatchPause {
        reached, resume, ..
    }) = pause
    else {
        return;
    };
    let _ = reached.send(());
    let _ = resume.await;
}

#[cfg(test)]
pub(crate) fn notify_peer_capability_waiters(state: &mut HostState) {
    let waiters = std::mem::take(&mut state.peer_capability_waiters);
    for waiter in waiters {
        if state
            .peer_capabilities
            .peer_supports(waiter.client_id as i32, waiter.capability)
        {
            let _ = waiter.completion.send(());
        } else {
            state.peer_capability_waiters.push(waiter);
        }
    }
}

pub(crate) async fn handle_client_message_with_restart_fence(
    connection_id: u32,
    client_id: ClientId,
    message: ControlMessage,
    ping_ms: i32,
    state: &mut HostState,
) {
    if let Some(expected_nonce) = state.round_restart_pending_clients.get(&client_id).copied() {
        match message {
            ControlMessage::RoundRestartAck { restart_nonce }
                if restart_nonce == expected_nonce =>
            {
                if state
                    .round_restart_routes
                    .get(&client_id)
                    .is_some_and(|expected_route| *expected_route != connection_id)
                {
                    return;
                }
                state.round_restart_pending_clients.remove(&client_id);
                state.round_restart_routes.remove(&client_id);
                let now_seconds = state.resource_epoch.elapsed().as_secs();
                if let Some(backend) = state.resource_backend.as_mut() {
                    let mut random = resource_safe_random;
                    match backend.on_peer_connected(client_id as i32, now_seconds, &mut random) {
                        Ok(events) => dispatch_host_resource_events(events, false, state).await,
                        Err(error) => report_host_resource_error(error, state).await,
                    }
                } else {
                    let actions = state.resource_catalog.on_peer_connected(client_id as i32);
                    dispatch_host_resource_actions(actions, state).await;
                }
            }
            // Route liveness is not round-scoped. Keep answering it while the
            // application installs the new lobby; everything that can mutate
            // synchronized or resource state remains behind the fence.
            ControlMessage::Ping(_) | ControlMessage::Pong(_) => {
                handle_client_message(connection_id, client_id, message, ping_ms, state).await;
            }
            _ => {}
        }
        return;
    }
    handle_client_message(connection_id, client_id, message, ping_ms, state).await;
}

pub(crate) async fn handle_client_message(
    connection_id: u32,
    client_id: ClientId,
    message: ControlMessage,
    ping_ms: i32,
    state: &mut HostState,
) {
    // The per-message `ping_ms` mirrors `getPingTime()` at receive time and
    // stays with the message for activation requests
    // (src/C4Network2.cpp:1564); route ping state is maintained by the
    // transport task's `ConnectionPing` messages instead.
    match message {
        ControlMessage::PortCapabilities(capabilities) => {
            // A port peer that announced a different compatibility profile
            // cannot share this session: the two engines would diverge as soon
            // as any profile-gated behaviour ran. Refuse it here, before lobby
            // or game state exists. Silence is the legacy case and is admitted
            // by `compat_profile_admits` (clonk-org/clonk-rs#583).
            if !host_profile_announcement(state).compat_profile_admits(capabilities) {
                handle_client_disconnected(
                    connection_id,
                    client_id,
                    0,
                    0,
                    None,
                    Some("incompatible compatibility profile".to_string()),
                    state,
                )
                .await;
                return;
            }
            #[cfg(test)]
            wait_at_host_capability_dispatch_pause(state, client_id).await;
            // Record what this peer can do, and answer so it learns the same
            // about us. A stock C++ peer never sends this and never replies, so
            // it simply stays absent from the registry and keeps the
            // compatible path.
            state
                .peer_capabilities
                .record(client_id as i32, capabilities);
            #[cfg(test)]
            notify_peer_capability_waiters(state);
            if let Some(route) = state.accepted_routes.get_mut(&connection_id) {
                route.peer_is_port = true;
                if state.config.voice_enabled && route.protocol == crate::NetworkProtocol::Udp {
                    route.voice_auth.record_peer_capabilities(capabilities);
                }
                let announcement = route
                    .voice_auth
                    .announcement()
                    .unwrap_or_else(crate::PortCapabilities::supported_without_voice);
                let _ = route
                    .outbound
                    .try_send(ControlMessage::PortCapabilities(announcement));
            }
        }
        // Only the host restarts a session. A client claiming to is either
        // confused or hostile; either way there is nothing to act on.
        ControlMessage::HostRestarting { .. }
        | ControlMessage::HostRestartLobby { .. }
        | ControlMessage::RoundRestartAck { .. }
        | ControlMessage::ControlWaitAttribution(_) => {}
        ControlMessage::Ping(packet) => {
            if let Some(route) = state.accepted_routes.get(&connection_id) {
                let _ = route.outbound.try_send(ControlMessage::Pong(packet));
            }
        }
        ControlMessage::Pong(_) => {}
        ControlMessage::ConnectionRequest(_) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: "accepted client sent a duplicate connection request".to_string(),
                })
                .await;
        }
        ControlMessage::ConnectionReply(_) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: "accepted client sent a duplicate connection reply".to_string(),
                })
                .await;
        }
        ControlMessage::ForwardRequest(packet) => {
            handle_forward_request(connection_id, client_id, packet, ping_ms, state).await;
        }
        ControlMessage::Forward(packet) => {
            handle_forwarded_packet_for_host(connection_id, client_id, packet, ping_ms, state)
                .await;
        }
        ControlMessage::PostMortem(packet) => {
            handle_post_mortem_recovery(client_id, packet, ping_ms, state).await;
        }
        // PID_JoinData is host-to-client only; C++ silently ignores it on a
        // host (src/C4Network2.cpp:938-946).
        ControlMessage::JoinData(_) => {}
        // League results are host-authored. C++ recognizes the packet on a
        // host but accepts it only when it arrived from the host client
        // (src/C4Network2Players.cpp:392-419).
        ControlMessage::LeagueRoundResults(_) => {}
        ControlMessage::Address(packet) => {
            handle_received_host_address(client_id, packet, state).await;
        }
        ControlMessage::TcpSimOpen(_) => {
            // Host-side simultaneous-open socket ownership is not part of the
            // client mesh loop. The typed packet remains accepted/nonfatal.
        }
        ControlMessage::Resource(packet) => {
            let now_seconds = state.resource_epoch.elapsed().as_secs();
            if let Some(backend) = state.resource_backend.as_mut() {
                let mut random = resource_safe_random;
                match backend.on_packet(client_id as i32, &packet, now_seconds, &mut random) {
                    Ok(events) => {
                        if matches!(&packet, ResourcePacket::Derive(_)) {
                            let _ = state.resource_catalog.on_packet(client_id as i32, &packet);
                        }
                        update_derived_resource_sources_with_paths(
                            &mut state.published_player_sources,
                            Some(&mut state.published_player_local_paths),
                            &events,
                        );
                        dispatch_host_resource_events(events, false, state).await;
                    }
                    Err(error) => report_host_resource_error(error, state).await,
                }
            } else {
                let actions = state.resource_catalog.on_packet(client_id as i32, &packet);
                dispatch_host_resource_actions(actions, state).await;
            }
        }
        ControlMessage::Status(_) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: "client attempted to originate host Status".to_string(),
                })
                .await;
        }
        ControlMessage::StatusAck(status) => {
            let accepted = state
                .status_barrier
                .remote_ack_changes_state(client_id, status);
            if accepted {
                let _ = state
                    .event_tx
                    .send(HostEvent::StatusAck { client_id, status })
                    .await;
            }
            let effects = state.status_barrier.remote_ack(client_id, status);
            apply_barrier_effects(effects, state).await;
        }
        ControlMessage::LobbyCountdown(packet) => {
            let _ = state
                .event_tx
                .send(HostEvent::LobbyCountdown { packet })
                .await;
        }
        // A Request is host-authored. C++ rejects every network-origin
        // Request while running as the host, regardless of packet.Client
        // (src/C4Network2.cpp:1642-1654).
        ControlMessage::ReadyCheck(packet) if packet.data.vote_requested() => {}
        ControlMessage::ReadyCheck(packet) => {
            apply_ready_check_to_host_state(packet, state);
            let _ = state.event_tx.send(HostEvent::ReadyCheck { packet }).await;
        }
        ControlMessage::ActivationRequest { tick } => {
            let waited_for = matches!(
                state.status_barrier.remotes.get(&client_id),
                Some(RemoteBarrierState::NotReady | RemoteBarrierState::Ready)
            );
            let _ = state
                .event_tx
                .send(HostEvent::ActivationRequest {
                    client_id,
                    tick,
                    waited_for,
                    ping_ms,
                })
                .await;
        }
        ControlMessage::PlayerInfoUpdate(request) => {
            let _ = state
                .event_tx
                .send(HostEvent::PlayerInfoUpdate { client_id, request })
                .await;
        }
        ControlMessage::Control(packet) => {
            // C4GameControlNetwork::HandleControl receives the source client
            // ID but deliberately does not authenticate the packet envelope
            // against it. Only PID_ControlPkt checks its embedded ByClient.
            ingest_control(packet, ControlIngress::Network, state).await;
        }
        ControlMessage::Request { from_tick } => {
            fulfill_resync_request(client_id, from_tick, state).await;
        }
        ControlMessage::Packet { delivery, data } => {
            broadcast_packet(delivery, data, Some(client_id), state).await;
        }
        ControlMessage::ExecSync { control_tick } => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: format!(
                        "client attempted to release synchronized controls at tick {control_tick}"
                    ),
                })
                .await;
        }
    }
}

async fn handle_post_mortem_recovery(
    source_client_id: ClientId,
    packet: crate::PostMortemPacket,
    ping_ms: i32,
    state: &mut HostState,
) {
    state.closed_routes.expire();
    if let Some(expected_client_id) = state.closed_routes.client_id(packet.connection_id) {
        if expected_client_id != source_client_id {
            return;
        }
    }
    let Some(replay) = state.closed_routes.recover(&packet) else {
        let retiring_route = state
            .accepted_routes
            .get(&packet.connection_id)
            .is_some_and(|route| route.client_id == source_client_id)
            .then(|| {
                state
                    .accepted_routes
                    .get(&packet.connection_id)
                    .expect("checked accepted route exists")
                    .outbound
                    .clone()
            });
        if let Some(outbound) = retiring_route {
            state
                .pending_post_mortems
                .insert(packet.connection_id, (source_client_id, packet, ping_ms));
            // C++ retires even a locally-live route as soon as its peer sends
            // PID_PostMortem. The independent signal avoids waiting behind
            // that route's outbound queue or its liveness timeout.
            outbound.retire();
        }
        return;
    };
    for nested_packet in replay.packets {
        match crate::transport::parse_complete_packet(&nested_packet) {
            Ok(Some(message)) => {
                Box::pin(handle_client_message(
                    replay.connection_id,
                    replay.client_id,
                    message,
                    ping_ms,
                    state,
                ))
                .await;
            }
            Ok(None) => {
                report_unhandled_forwarded_packet(replay.client_id, &nested_packet, state).await;
            }
            Err(error) => {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: Some(replay.client_id),
                        error: format!(
                            "invalid post-mortem packet for closed connection {}: {error}",
                            replay.connection_id
                        ),
                    })
                    .await;
            }
        }
    }
}

pub(crate) fn forward_selects(packet: &crate::ForwardPacket, client_id: i32) -> bool {
    let listed = packet.clients.contains(&client_id);
    if listed {
        !packet.negative_list
    } else {
        packet.negative_list
    }
}

pub(crate) fn decentral_control_message(
    packet: &ControlPacket,
) -> Result<ControlMessage, TransportError> {
    decentral_control_message_to_unconnected(packet, std::iter::empty())
}

pub(crate) fn decentral_control_message_to_unconnected(
    packet: &ControlPacket,
    directly_reached: impl IntoIterator<Item = ClientId>,
) -> Result<ControlMessage, TransportError> {
    Ok(ControlMessage::ForwardRequest(crate::ForwardPacket {
        negative_list: true,
        clients: directly_reached
            .into_iter()
            .filter_map(|client_id| i32::try_from(client_id).ok())
            .collect(),
        nested_packet: crate::transport::encode_complete_control_packet(packet)?,
    }))
}

async fn report_forward_error(source: ClientId, error: String, state: &HostState) {
    let _ = state
        .event_tx
        .send(HostEvent::TransportError {
            client_id: Some(source),
            error,
        })
        .await;
}

async fn report_unhandled_forwarded_packet(
    source: ClientId,
    nested_packet: &[u8],
    state: &HostState,
) {
    let Some(&packet_type) = nested_packet.first() else {
        return;
    };
    let _ = state
        .event_tx
        .send(HostEvent::UnhandledPacket {
            client_id: Some(source),
            packet_type,
        })
        .await;
}

async fn handle_forward_request(
    connection_id: u32,
    source: ClientId,
    packet: crate::ForwardPacket,
    ping_ms: i32,
    state: &mut HostState,
) {
    // A relayed packet reaches its target on the host's own route and is
    // therefore indistinguishable from one the host authored. C++ has no
    // packet whose meaning depends on coming from the host, so it relays
    // anything (src/C4Network2IO.cpp:1066-1082); the port's own IDs do, and a
    // client has no business speaking for the host in that range.
    if packet
        .nested_packet
        .first()
        .is_some_and(|packet_type| *packet_type & 0xf0 == 0x70)
    {
        let _ = state
            .event_tx
            .send(HostEvent::UnhandledPacket {
                client_id: Some(source),
                packet_type: packet.nested_packet[0],
            })
            .await;
        return;
    }
    // C4Network2IO keeps connection-list order, excludes the requester's
    // client ID, and deduplicates targets into a positive list. Rust assigns
    // monotonically increasing IDs, so reverse ID order mirrors the current
    // head-inserted C++ connection list (src/C4Network2IO.cpp:1066-1082).
    let target_ids = state
        .clients
        .keys()
        .rev()
        .copied()
        .filter(|client_id| *client_id != source)
        .filter(|client_id| {
            i32::try_from(*client_id).is_ok_and(|client_id| forward_selects(&packet, client_id))
        })
        .collect::<Vec<_>>();
    if target_ids.len() <= 2 {
        for client_id in &target_ids {
            let _ = send_host_raw(
                state,
                *client_id,
                ConnectionTrafficClass::Message,
                packet.nested_packet.clone(),
            )
            .await;
        }
    } else {
        let forwarded = ControlMessage::Forward(crate::ForwardPacket {
            negative_list: false,
            clients: target_ids
                .iter()
                .filter_map(|client_id| i32::try_from(*client_id).ok())
                .collect(),
            nested_packet: packet.nested_packet.clone(),
        });
        for client_id in &target_ids {
            let _ = send_host_message(
                state,
                *client_id,
                ConnectionTrafficClass::Message,
                forwarded.clone(),
            )
            .await;
        }
    }
    if forward_selects(&packet, HOST_CLIENT_ID as i32) {
        dispatch_forwarded_packet_for_host(
            connection_id,
            source,
            &packet.nested_packet,
            ping_ms,
            state,
        )
        .await;
    }
}

async fn handle_forwarded_packet_for_host(
    connection_id: u32,
    source: ClientId,
    packet: crate::ForwardPacket,
    ping_ms: i32,
    state: &mut HostState,
) {
    if !forward_selects(&packet, HOST_CLIENT_ID as i32) {
        return;
    }
    dispatch_forwarded_packet_for_host(
        connection_id,
        source,
        &packet.nested_packet,
        ping_ms,
        state,
    )
    .await;
}

async fn dispatch_forwarded_packet_for_host(
    connection_id: u32,
    source: ClientId,
    nested_packet: &[u8],
    ping_ms: i32,
    state: &mut HostState,
) {
    let message = match crate::transport::parse_complete_packet(nested_packet) {
        Ok(Some(message)) => message,
        Ok(None) => {
            report_unhandled_forwarded_packet(source, nested_packet, state).await;
            return;
        }
        Err(error) => {
            report_forward_error(source, format!("invalid forwarded packet: {error}"), state).await;
            if let Some(route) = state
                .accepted_routes
                .get(&connection_id)
                .filter(|route| route.client_id == source)
            {
                route.outbound.retire();
            }
            return;
        }
    };
    match message {
        ControlMessage::Packet { delivery, data }
            if matches!(delivery, ControlDelivery::Direct | ControlDelivery::Private) =>
        {
            dispatch_packet(delivery, data, Some(source), false, state).await;
        }
        message => {
            Box::pin(handle_client_message(
                connection_id,
                source,
                message,
                ping_ms,
                state,
            ))
            .await;
        }
    }
}

pub(crate) async fn dispatch_host_resource_actions(
    actions: Vec<crate::ResourceCatalogAction>,
    state: &mut HostState,
) {
    for action in actions {
        match action {
            crate::ResourceCatalogAction::SendToPeer { peer_id, packet } => {
                let Ok(client_id) = ClientId::try_from(peer_id) else {
                    continue;
                };
                let traffic = resource_traffic_class(&packet);
                let _ =
                    send_host_message(state, client_id, traffic, ControlMessage::Resource(packet))
                        .await;
            }
            crate::ResourceCatalogAction::Broadcast { packet } => {
                let traffic = resource_traffic_class(&packet);
                let _ =
                    broadcast_host_message(state, traffic, ControlMessage::Resource(packet), None);
            }
            external => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceAction(external))
                    .await;
            }
        }
    }
}

pub(crate) async fn dispatch_host_resource_events(
    events: Vec<crate::ResourceTransferEvent>,
    completion_local: bool,
    state: &mut HostState,
) {
    for event in events {
        match event {
            crate::ResourceTransferEvent::Transport(action) => {
                dispatch_host_resource_actions(vec![action], state).await;
            }
            crate::ResourceTransferEvent::Progress {
                resource_id,
                present_percent,
            } => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceProgress {
                        resource_id,
                        present_percent,
                    })
                    .await;
            }
            crate::ResourceTransferEvent::Completed {
                resource_id,
                core,
                path,
            } => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceComplete {
                        resource_id,
                        core,
                        path,
                        local: completion_local,
                    })
                    .await;
            }
            crate::ResourceTransferEvent::LoadFailed { resource_id } => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceLoadFailed { resource_id })
                    .await;
            }
        }
    }
}

pub(crate) async fn report_host_resource_error(
    error: crate::ResourceTransferError,
    state: &HostState,
) {
    let _ = state
        .event_tx
        .send(HostEvent::TransportError {
            client_id: None,
            error: format!("resource transfer failed: {error}"),
        })
        .await;
}

async fn handle_received_host_address(
    source_client_id: ClientId,
    packet: crate::AddressPacket,
    state: &mut HostState,
) {
    if !state.client_cores.contains_key(&packet.client_id) {
        return;
    }
    let Some(peer_addr) = state
        .clients
        .get(&source_client_id)
        .map(|client| client.peer_addr)
    else {
        return;
    };
    let packet = packet.announcement_for_peer(peer_addr);
    let insertion = crate::append_received_address(
        state.client_addresses.entry(packet.client_id).or_default(),
        packet.address,
    );
    if !matches!(insertion, crate::AddressInsertion::Added { .. }) {
        return;
    }

    // AddAddr(..., true) re-announces a newly learned address to every
    // connected client, including the source connection. The source then
    // suppresses the duplicate on receipt (src/C4Network2Client.cpp:259-278,
    // 581-597).
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::Address(packet),
        None,
    );
}

/// What this host announces about its own compatibility profile.
///
/// Always carries [`crate::PortCapabilities::COMPAT_PROFILE_ANNOUNCED`]: this
/// build knows about profiles, so its silence would otherwise be read as the
/// legacy case and admit a peer it should refuse.
pub(crate) fn host_profile_announcement(state: &HostState) -> crate::PortCapabilities {
    let mut bits = crate::PortCapabilities::COMPAT_PROFILE_ANNOUNCED;
    if state.config.compat_profile_legacy {
        bits |= crate::PortCapabilities::COMPAT_PROFILE_LEGACY_CLONK;
    }
    crate::PortCapabilities::from_bits(bits)
}

pub(crate) async fn handle_client_disconnected(
    connection_id: u32,
    client_id: ClientId,
    next_inbound_packet: u32,
    mut next_outbound_packet: u32,
    mut post_mortem: Option<crate::PostMortemPacket>,
    reason: Option<String>,
    state: &mut HostState,
) {
    let disconnected_route = state.accepted_routes.remove(&connection_id);
    if disconnected_route.is_none() {
        return;
    }
    if disconnected_route
        .as_ref()
        .is_some_and(|route| route.protocol == crate::NetworkProtocol::Udp)
        && !state.accepted_routes.values().any(|route| {
            route.client_id == client_id && route.protocol == crate::NetworkProtocol::Udp
        })
    {
        state
            .peer_capabilities
            .clear(client_id as i32, crate::PortCapabilities::VOICE_CHAT);
    }
    state.invalidate_control_send_time();
    #[cfg(test)]
    notify_accepted_route_waiters(state);
    if let Some(route) = &disconnected_route {
        state
            .closed_routes
            .retain(connection_id, route.client_id, next_inbound_packet);
        for message in route.outbound.retire_and_take_post_failure() {
            let packet = match message {
                HostOutboundMessage::Message(message) => {
                    crate::transport::encode_complete_message(message).ok()
                }
                HostOutboundMessage::Raw(packet) => Some(packet),
            };
            if let Some(packet) = packet {
                crate::post_mortem::retain_post_failure_packet(
                    &mut post_mortem,
                    route.remote_connection_id,
                    &mut next_outbound_packet,
                    packet,
                );
            }
        }
    }
    if let Some((source_client_id, packet, ping_ms)) =
        state.pending_post_mortems.remove(&connection_id)
    {
        handle_post_mortem_recovery(source_client_id, packet, ping_ms, state).await;
    }
    let is_secondary_route = disconnected_route.as_ref().is_some_and(|route| {
        state
            .clients
            .get(&client_id)
            .is_some_and(|client| !route.outbound.same_channel(&client.outbound))
    });
    let promoted_route = disconnected_route
        .as_ref()
        .filter(|route| {
            state
                .clients
                .get(&client_id)
                .is_some_and(|client| route.outbound.same_channel(&client.outbound))
        })
        .and_then(|_| preferred_host_route(state, client_id, ConnectionTrafficClass::Message))
        .map(|route| (route.outbound.clone(), route.peer_addr));
    if let Some(AcceptedConnectionRoute {
        client_id: route_client_id,
        remote_connection_id: _remote_connection_id,
        peer_addr: _peer_addr,
        protocol: _protocol,
        outbound: _outbound,
        ping: _,
        voice_auth: _,
        peer_is_port: _,
    }) = disconnected_route
    {
        debug_assert_eq!(route_client_id, client_id);
    }
    if is_secondary_route {
        if let Some(post_mortem) = post_mortem {
            let _ = try_send_host_message(
                state,
                client_id,
                ConnectionTrafficClass::Message,
                ControlMessage::PostMortem(post_mortem),
            );
        }
        if let Some(reason) = reason {
            let _ = state
                .event_tx
                .send(HostEvent::RecoverableRouteDiagnostic {
                    client_id: Some(client_id),
                    error: reason,
                })
                .await;
        }
        return;
    }
    if let Some((outbound, peer_addr)) = promoted_route {
        if let Some(client) = state.clients.get_mut(&client_id) {
            client.outbound = outbound.clone();
            client.peer_addr = peer_addr;
        }
        if let Some(post_mortem) = post_mortem {
            let _ = try_send_host_message(
                state,
                client_id,
                ConnectionTrafficClass::Message,
                ControlMessage::PostMortem(post_mortem),
            );
        }
        if let Some(reason) = reason {
            let _ = state
                .event_tx
                .send(HostEvent::RecoverableRouteDiagnostic {
                    client_id: Some(client_id),
                    error: reason,
                })
                .await;
        }
        return;
    }
    state.peer_capabilities.forget(client_id as i32);
    mark_client_removing(client_id, state);
    let disconnected = state.clients.remove(&client_id);
    let removed_logical_client = disconnected.is_some();
    if removed_logical_client {
        state.round_restart_pending_clients.remove(&client_id);
        state.round_restart_routes.remove(&client_id);
    }
    if let Some(client) = &disconnected {
        state.pending_kinds.remove(&client.core.client_id);
        if let Some(remote) = state.status_barrier.remotes.get_mut(&client_id) {
            *remote = RemoteBarrierState::Removing;
        }
    }

    if removed_logical_client {
        // C4Network2::OnClientDisconnect reports the league disconnect before
        // scheduling the synchronized client removal. This event is separate
        // from ClientLeft so controlled removal and host teardown cannot be
        // mistaken for a failed connection.
        let _ = state
            .event_tx
            .send(HostEvent::ClientConnectionFailed { client_id })
            .await;
        let _ = state
            .event_tx
            .send(HostEvent::ClientLeft { client_id })
            .await;
    }

    if let Some(client) = disconnected {
        // Socket loss stops waiting for this peer's status acknowledgement,
        // but the running control client remains active until the synchronized
        // ClientRemove executes. C4ClientList::CtrlRemove only flags the net
        // client before queuing CDT_Sync; C4GameControlNetwork refreshes its
        // active-client copy at that synchronization boundary
        // (src/C4Client.cpp:293-303;
        // src/C4GameControlNetwork.cpp:181-220,260-297,329-345).
        queue_disconnected_client_remove(&client.core, state).await;
    }
    let barrier_effects = state.status_barrier.remove_remote(client_id);
    apply_barrier_effects(barrier_effects, state).await;
    if removed_logical_client {
        let retry_effects = retry_unreached_status_after_disconnect(
            &mut state.status_barrier,
            state.coordinator.current_tick(),
        );
        apply_barrier_effects(retry_effects, state).await;
    }

    if let Some(reason) = reason {
        let _ = state
            .event_tx
            .send(HostEvent::RecoverableRouteDiagnostic {
                client_id: Some(client_id),
                error: reason,
            })
            .await;
    }
}

pub(crate) async fn handle_admission_failed(
    connection_id: u32,
    error: String,
    state: &mut HostState,
) {
    state.pending_route_peers.remove(&connection_id);
    let route_client_id = state.pending_route_clients.remove(&connection_id);
    let provisional_client_id = state.pending_admissions.remove(&connection_id);
    let provisional_client_id = provisional_client_id.and_then(|client_id| {
        ClientId::try_from(client_id)
            .ok()
            .map(|network_client_id| (client_id, network_client_id))
    });
    if provisional_client_id.is_some_and(|(_, client_id)| {
        state.clients.contains_key(&client_id)
            || state
                .accepted_routes
                .values()
                .any(|route| route.client_id == client_id)
    }) {
        let _ = state
            .event_tx
            .send(HostEvent::RecoverableRouteDiagnostic {
                client_id: provisional_client_id.map(|(_, client_id)| client_id),
                error,
            })
            .await;
        return;
    }
    if let Some(core) =
        provisional_client_id.and_then(|(client_id, _)| state.client_cores.get(&client_id).cloned())
    {
        let client_id =
            ClientId::try_from(core.client_id).expect("provisional client id was validated above");
        mark_client_removing(client_id, state);
        // C4Network2::OnConnectFail performs the same logical-client
        // disconnect notification before CtrlRemove as an accepted route
        // failure. The failed socket remains peer-local; it does not abort
        // the host network loop (src/C4Network2.cpp:1761-1771,1802-1824).
        let _ = state
            .event_tx
            .send(HostEvent::ClientConnectionFailed { client_id })
            .await;
        queue_disconnected_client_remove(&core, state).await;
        let retry_effects = retry_unreached_status_after_disconnect(
            &mut state.status_barrier,
            state.coordinator.current_tick(),
        );
        apply_barrier_effects(retry_effects, state).await;
    }
    let Some(client_id) = provisional_client_id
        .map(|(_, client_id)| client_id)
        .or(route_client_id)
    else {
        // A socket that never named a client is C4Network2IO::OnDisconn /
        // C4Network2::OnConnectFail, and a refusal is HandleConn's "connection
        // by X blocked" — all at info, under the warn its GUI sink defaults to.
        // MainDlg::OnLog does not receive it; the log file still does, which is
        // where a host reads why a join failed (src/C4NetIO.cpp:749;
        // src/C4Network2IO.cpp:533-566; src/C4Network2.cpp:1361,1745-1747;
        // src/C4Log.cpp:307).
        let _ = state
            .event_tx
            .send(HostEvent::UnassociatedConnectionFailed { error })
            .await;
        return;
    };
    let event = HostEvent::RecoverableRouteDiagnostic {
        client_id: Some(client_id),
        error,
    };
    let _ = state.event_tx.send(event).await;
}

pub(crate) fn retry_unreached_status_after_disconnect(
    barrier: &mut StatusBarrier,
    current_tick: Tick,
) -> Vec<BarrierEffect> {
    if !matches!(
        barrier.phase,
        BarrierPhase::Waiting {
            local_reached: false
        }
    ) || !matches!(barrier.status.state, NETWORK_STATE_GO | NETWORK_STATE_PAUSE)
    {
        return Vec::new();
    }

    let status = barrier
        .status
        .with_target_tick(i32::try_from(current_tick).unwrap_or(i32::MAX));
    // ChangeGameStatus immediately asks the runtime to CheckStatusReached.
    // Keep local arrival app-owned even when this target equals the session
    // coordinator's tick; otherwise a one-shot commit can outrun GameApp.
    barrier.change_status(status)
}

async fn queue_disconnected_client_remove(
    core: &clonk_engine::ClientCoreControlData,
    state: &mut HostState,
) {
    let reason =
        clonk_engine::LegacyCString::from_bytes(b"disconnected".to_vec()).unwrap_or_default();
    queue_host_client_remove(core, reason, state).await;
}

pub(crate) async fn fail_host_pending_join_data(
    reason: clonk_engine::LegacyCString,
    state: &mut HostState,
) -> usize {
    let pending = pending_join_data_client_ids(&state.clients, &state.removing_clients)
        .into_iter()
        .filter_map(|client_id| {
            state
                .clients
                .get(&client_id)
                .map(|client| client.core.clone())
        })
        .collect::<Vec<_>>();
    let removed = pending.len();
    for core in pending {
        queue_host_client_remove(&core, reason.clone(), state).await;
    }
    removed
}

async fn queue_host_client_remove(
    core: &clonk_engine::ClientCoreControlData,
    reason: clonk_engine::LegacyCString,
    state: &mut HostState,
) {
    let Ok(data) = crate::encode_control_entry_payload(&clonk_engine::ControlPacket::ClientRemove(
        clonk_engine::ClientRemoveControlData {
            client_id: core.client_id,
            reason,
            by_client: 0,
        },
    )) else {
        return;
    };
    broadcast_packet(ControlDelivery::Sync, data, None, state).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlIngress {
    Local,
    Network,
}

pub(crate) async fn ingest_control(
    packet: ControlPacket,
    ingress: ControlIngress,
    state: &mut HostState,
) {
    let client_id = packet.client_id();
    // Validate everything PackCompleteCtrl needs before the coordinator
    // consumes contributions and advances its tick. A malformed frame must
    // not create a permanent hole in the lockstep stream.
    if let Err(error) = validate_control_envelope(&packet) {
        let _ = state
            .event_tx
            .send(HostEvent::TransportError {
                client_id: Some(client_id),
                error: format!("invalid control packet: {error}"),
            })
            .await;
        return;
    }
    if let Err(error) = validate_queued_control_authors(&packet) {
        let _ = state
            .event_tx
            .send(HostEvent::TransportError {
                client_id: Some(client_id),
                error,
            })
            .await;
        return;
    }
    if !state.backlog.contains_packet(client_id, packet.tick()) {
        state.client_performance.record_arrival(
            client_id,
            packet.tick(),
            tokio::time::Instant::now(),
        );
    }
    if ingress == ControlIngress::Local {
        state.local_control_backlog.record_packet(&packet);
    }
    state.backlog.record_packet(&packet);
    // A host's own DoInput broadcasts before AddCtrl in CNM_Decentral. A raw
    // network PID_Control goes straight to HandleControl and is only stored;
    // client fallback fanout belongs to PID_FwdReq instead (pristine C++
    // src/C4GameControlNetwork.cpp:156-179,517-529).
    if ingress == ControlIngress::Local && state.control_mode == 0 {
        broadcast_control(&packet, state).await;
    }
    if client_id == BROADCAST_CLIENT_ID {
        ingest_complete_control(packet, state).await;
        return;
    }
    match state.coordinator.ingest(packet) {
        Ok(ControlOutcome { ready, missing, .. }) => {
            resolve_host_ready(ready, state).await;
            if !missing.is_empty() {
                schedule_missing(missing, state);
            }
        }
        Err(crate::ControlError::UnknownClient(_)) if ingress == ControlIngress::Network => {
            // HandleControl accepts network control independently of the
            // activated client list. Rust cannot use that contribution in the
            // active coordinator yet, but a valid early packet is not a
            // transport error and must not create per-tick error noise.
        }
        Err(error) => {
            let _ = state
                .event_tx
                .send(HostEvent::FatalError {
                    error: format!(
                        "authoritative host control for client {client_id} was rejected: {error}"
                    ),
                })
                .await;
        }
    }
}

async fn ingest_complete_control(packet: ControlPacket, state: &mut HostState) {
    if packet.tick() < state.coordinator.current_tick() {
        return;
    }
    state
        .pending_complete
        .entry(packet.tick())
        .or_insert(packet);
    resolve_host_ready(Vec::new(), state).await;
}

// Borrow the lobby TextWindow's numeric 100/4096 ceilings as conservative
// raw-packet cache bounds (src/C4GameLobby.cpp:277-280).
const LOBBY_CHAT_HISTORY_MAX_MESSAGES: usize = 100;
const LOBBY_CHAT_HISTORY_MAX_BYTES: usize = 4096;

/// Retain only public, presentation-only lobby conversation in its authenticated
/// C++ wire form. Reusing the original control keeps rendering and sender
/// lookup on the ordinary client execution path without touching lockstep.
fn retain_lobby_chat_message(
    delivery: ControlDelivery,
    control: &clonk_engine::ControlPacket,
    data: &[u8],
    state: &mut HostState,
) {
    let retain = delivery == ControlDelivery::Private
        && state.status_barrier.status.state == NETWORK_STATE_LOBBY
        && matches!(
            control,
            clonk_engine::ControlPacket::Message(message)
                if matches!(
                    message.message_type,
                    clonk_engine::MESSAGE_TYPE_NORMAL | clonk_engine::MESSAGE_TYPE_ME
                )
        );
    if !retain {
        return;
    }

    state.lobby_chat_history.push_back(data.to_vec());
    while state.lobby_chat_history.len() > LOBBY_CHAT_HISTORY_MAX_MESSAGES
        || state.lobby_chat_history.iter().map(Vec::len).sum::<usize>()
            > LOBBY_CHAT_HISTORY_MAX_BYTES
    {
        state.lobby_chat_history.pop_front();
    }
}

/// Authenticate security-sensitive inner authors in a queued contribution.
///
/// C++ typed packet unpack rejects unknown control IDs, so every queued frame
/// reaching the coordinator is fully decoded and checked here.
pub(crate) fn validate_queued_control_authors(packet: &ControlPacket) -> Result<(), String> {
    let (controls, _) = packet
        .decoded_control_list()
        .map_err(|error| format!("invalid control packet: {error}"))?;
    // PackCompleteCtrl keeps each contribution's embedded ByClient while the
    // merged envelope uses C4ClientIDAll (src/C4GameControlNetwork.cpp:741-777).
    if packet.client_id() == BROADCAST_CLIENT_ID {
        return Ok(());
    }
    let expected_author = i32::try_from(packet.client_id()).map_err(|_| {
        format!(
            "queued control packet has unsupported author id {}",
            packet.client_id()
        )
    })?;
    for control in controls {
        let (name, author) = match control {
            clonk_engine::ControlPacket::ClientJoin(control) => {
                ("CID_ClientJoin", control.by_client)
            }
            clonk_engine::ControlPacket::ClientUpdate(control) => {
                ("CID_ClientUpdate", control.by_client)
            }
            clonk_engine::ControlPacket::ClientRemove(control) => {
                ("CID_ClientRemove", control.by_client)
            }
            clonk_engine::ControlPacket::PlayerInfo(control) => ("CID_PlrInfo", control.by_client),
            clonk_engine::ControlPacket::JoinPlayer(control) => ("CID_JoinPlr", control.by_client),
            clonk_engine::ControlPacket::PlayerSelect(control) => {
                ("CID_PlrSelect", control.by_client)
            }
            clonk_engine::ControlPacket::PlayerControl(control) => {
                ("CID_PlrControl", control.by_client)
            }
            clonk_engine::ControlPacket::PlayerCommand(control) => {
                ("CID_PlrCommand", control.by_client)
            }
            clonk_engine::ControlPacket::Script(script) => ("CID_Script", script.by_client),
            clonk_engine::ControlPacket::MessageBoardAnswer(answer) => {
                ("CID_MessageBoardAnswer", answer.by_client)
            }
            clonk_engine::ControlPacket::Message(message) => ("CID_Message", message.by_client),
            clonk_engine::ControlPacket::CustomCommand(command) => {
                ("CID_CustomCommand", command.by_client)
            }
            clonk_engine::ControlPacket::EmMoveObject(control) => {
                ("CID_EMMoveObj", control.by_client)
            }
            clonk_engine::ControlPacket::EmDrawTool(control) => {
                ("CID_EMDrawTool", control.by_client)
            }
            clonk_engine::ControlPacket::EmDropDef(control) => ("CID_EMDropDef", control.by_client),
            clonk_engine::ControlPacket::ActivateGameGoalMenu(control) => {
                ("CID_ActivateGameGoalMenu", control.by_client)
            }
            clonk_engine::ControlPacket::ToggleHostility(control) => {
                ("CID_ToggleHostility", control.by_client)
            }
            clonk_engine::ControlPacket::ActivateGameGoalRule(control) => {
                ("CID_ActivateGameGoalRule", control.by_client)
            }
            clonk_engine::ControlPacket::SetPlayerTeam(control) => {
                ("CID_SetPlayerTeam", control.by_client)
            }
            clonk_engine::ControlPacket::EliminatePlayer(control) => {
                ("CID_EliminatePlayer", control.by_client)
            }
            clonk_engine::ControlPacket::RemovePlayer(remove) => {
                ("CID_RemovePlr", remove.by_client)
            }
            clonk_engine::ControlPacket::Set(set) => ("CID_Set", set.by_client),
            clonk_engine::ControlPacket::Vote(vote) => ("CID_Vote", vote.by_client),
            clonk_engine::ControlPacket::VoteEnd(vote) => ("CID_VoteEnd", vote.by_client),
            clonk_engine::ControlPacket::InitScenarioPlayer(control) => {
                ("CID_InitScenarioPlayer", control.by_client)
            }
            clonk_engine::ControlPacket::SurrenderPlayer(control) => {
                ("CID_SurrenderPlayer", control.by_client)
            }
            clonk_engine::ControlPacket::Synchronize(control) => {
                ("CID_Synchronize", control.by_client)
            }
            clonk_engine::ControlPacket::SyncCheck(control) => ("CID_SyncCheck", control.by_client),
            // DebugRec has no inherited C4ControlPacket body, so the outer
            // authenticated contribution is its only author identity.
            clonk_engine::ControlPacket::DebugRecord(_) => continue,
            _ => continue,
        };
        if author != expected_author {
            return Err(format!(
                "queued {name} claimed author {author}, but authenticated author is {expected_author}"
            ));
        }
    }
    Ok(())
}

fn schedule_missing(missing: Vec<MissingRange>, state: &mut HostState) {
    let requests = state.scheduler.schedule(missing.iter(), Instant::now());
    for request in requests {
        if !state
            .clients
            .get(&request.client_id)
            .is_some_and(|client| client.join_data_sent)
        {
            continue;
        }
        let _ = try_send_host_message(
            state,
            request.client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Request {
                from_tick: request.from_tick,
            },
        );
    }
}

pub(crate) async fn request_missing_controls(state: &mut HostState) {
    let missing = state.coordinator.missing_ranges();
    if missing.is_empty() {
        return;
    }
    schedule_missing(missing, state);
}

async fn fulfill_resync_request(client_id: ClientId, from_tick: Tick, state: &mut HostState) {
    let resend = state.backlog.fulfill_request(from_tick);
    for packet in resend {
        if !send_host_message(
            state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Control(packet),
        )
        .await
        {
            return;
        }
    }
}

fn control_wait_attribution_for(
    tick: Tick,
    recipient: ClientId,
    waiting: &BTreeSet<ClientId>,
    discarded: &BTreeSet<ClientId>,
) -> Option<crate::ControlWaitAttribution> {
    (!waiting.is_empty()).then(|| crate::ControlWaitAttribution {
        tick,
        waited_for_recipient: waiting.contains(&recipient),
        waited_for_other: waiting.iter().any(|client_id| *client_id != recipient),
        discarded_recipient_control: discarded.contains(&recipient),
    })
}

pub(crate) async fn publish_ready_batch(batch: ReadyBatch, state: &mut HostState) {
    let aggregated = match aggregate_ready_batch(&batch) {
        Ok(packet) => packet,
        Err(error) => {
            let _ = state
                .event_tx
                .send(HostEvent::FatalError {
                    error: format!("failed to aggregate ready tick {}: {error}", batch.tick()),
                })
                .await;
            return;
        }
    };

    state.backlog.record_ready_batch(&batch);
    state.backlog.record_packet(&aggregated);
    // Only central/async hosts transmit C4ClientIDAll. Decentralized peers
    // already received each contribution and pack this packet themselves
    // (src/C4GameControlNetwork.cpp:763-777).
    if state.control_mode != 0 {
        let waiting = state
            .control_waiting_clients
            .remove(&batch.tick())
            .unwrap_or_default();
        let discarded = state
            .control_discarded_clients
            .remove(&batch.tick())
            .unwrap_or_default();
        for client_id in state
            .coordinator
            .client_ids()
            .filter(|client_id| *client_id != HOST_CLIENT_ID)
        {
            if !state.peer_capabilities.peer_supports(
                client_id as i32,
                crate::PortCapabilities::CONTROL_WAIT_ATTRIBUTION,
            ) {
                continue;
            }
            if let Some(attribution) =
                control_wait_attribution_for(batch.tick(), client_id, &waiting, &discarded)
            {
                let _ = try_send_host_message(
                    state,
                    client_id,
                    ConnectionTrafficClass::Message,
                    ControlMessage::ControlWaitAttribution(attribution),
                );
            }
        }
        broadcast_control(&aggregated, state).await;
    }
    let _ = state
        .event_tx
        .send(HostEvent::Ready { packet: aggregated })
        .await;
}

/// Resolve locally completed batches against received C4ClientIDAll packets.
///
/// CheckCompleteCtrl always consumes a stored complete packet before packing
/// the same tick from partial contributions, and walks only contiguous ticks
/// (src/C4GameControlNetwork.cpp:679-719).
pub(crate) async fn resolve_host_ready(ready: Vec<ReadyBatch>, state: &mut HostState) {
    let mut batches = VecDeque::from(ready);
    loop {
        while let Some(batch) = batches.pop_front() {
            let complete = state.pending_complete.remove(&batch.tick());
            if let Some(packet) = complete {
                let _ = state.event_tx.send(HostEvent::Ready { packet }).await;
            } else {
                publish_ready_batch(batch, state).await;
            }
        }

        let tick = state.coordinator.current_tick();
        let Some(packet) = state.pending_complete.remove(&tick) else {
            break;
        };
        let next_tick = tick.saturating_add(1);
        if next_tick != tick {
            batches.extend(state.coordinator.advance_to(next_tick));
        }
        let _ = state.event_tx.send(HostEvent::Ready { packet }).await;
        if next_tick == tick {
            break;
        }
    }

    let current_tick = state.coordinator.current_tick();
    state
        .pending_complete
        .retain(|tick, _| *tick >= current_tick);
}

async fn broadcast_control(packet: &ControlPacket, state: &mut HostState) {
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::Control(packet.clone()),
        None,
    );
}

pub(crate) async fn broadcast_packet(
    delivery: ControlDelivery,
    data: Vec<u8>,
    origin: Option<ClientId>,
    state: &mut HostState,
) {
    dispatch_packet(delivery, data, origin, true, state).await;
}

async fn dispatch_packet(
    delivery: ControlDelivery,
    data: Vec<u8>,
    origin: Option<ClientId>,
    relay_to_clients: bool,
    state: &mut HostState,
) {
    match delivery {
        ControlDelivery::Sync => {
            let expected_author = origin
                .and_then(|client_id| i32::try_from(client_id).ok())
                .unwrap_or(0);
            let control = match authenticated_single_control(&data, expected_author) {
                Ok(control) => control,
                Err(error) => {
                    let _ = state
                        .event_tx
                        .send(HostEvent::TransportError {
                            client_id: origin,
                            error,
                        })
                        .await;
                    return;
                }
            };
            if expected_author == HOST_CLIENT_ID as i32 {
                if let clonk_engine::ControlPacket::ClientRemove(remove) = &control {
                    if remove.by_client == HOST_CLIENT_ID as i32 {
                        if let Ok(client_id) = ClientId::try_from(remove.client_id) {
                            if state.client_cores.contains_key(&remove.client_id)
                                || state.clients.contains_key(&client_id)
                                || state.status_barrier.remotes.contains_key(&client_id)
                            {
                                mark_client_removing(client_id, state);
                            }
                        }
                    }
                }
            }
            // The client that originated a Sync packet deleted its local copy
            // and waits for the host echo, so include every client here
            // (src/C4GameControlNetwork.cpp:181-220,568-572).
            if relay_to_clients {
                let _ = broadcast_host_message(
                    state,
                    ConnectionTrafficClass::Message,
                    ControlMessage::Packet {
                        delivery,
                        data: data.clone(),
                    },
                    None,
                );
            }
            state.pending_sync.push(control);
            if state.status_barrier.is_frozen() {
                execute_frozen_sync(state.coordinator.current_tick(), state).await;
            } else if let Ok(next_control_tick) = i32::try_from(state.coordinator.current_tick()) {
                let effects = state.status_barrier.sync(next_control_tick);
                apply_barrier_effects(effects, state).await;
            }
        }
        ControlDelivery::Queue | ControlDelivery::Decide => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: origin,
                    error: format!("single control packet cannot use {delivery:?} delivery"),
                })
                .await;
        }
        ControlDelivery::Direct | ControlDelivery::Private => {
            let expected_author = origin
                .and_then(|client_id| i32::try_from(client_id).ok())
                .unwrap_or(0);
            let mut control = match authenticated_single_control(&data, expected_author) {
                Ok(control) => control,
                Err(error) => {
                    let _ = state
                        .event_tx
                        .send(HostEvent::TransportError {
                            client_id: origin,
                            error,
                        })
                        .await;
                    return;
                }
            };
            let mut local_data = data.clone();
            let mut prompt_player_resource_discovery = Vec::new();
            retain_lobby_chat_message(delivery, &control, &data, state);
            if let clonk_engine::ControlPacket::PlayerInfo(info) = &mut control {
                let resource_owner = info.client_id;
                let resource_owner_client_id = ClientId::try_from(resource_owner).ok();
                let resource_owner_is_authorized =
                    origin.is_none() || origin == resource_owner_client_id;
                let loaded = load_authoritative_player_resources(
                    &state.resource_resolver,
                    &mut state.resource_catalog,
                    state.resource_backend.as_mut(),
                    info,
                );
                for (path, core) in &loaded.local_sources {
                    let _ = state
                        .event_tx
                        .send(HostEvent::ResourceComplete {
                            resource_id: core.id,
                            core: core.clone(),
                            path: path.clone(),
                            local: true,
                        })
                        .await;
                }
                state
                    .published_player_sources
                    .extend(loaded.local_sources.iter().cloned());
                state.published_player_local_paths.extend(
                    loaded
                        .local_sources
                        .iter()
                        .map(|(path, _)| (path.clone(), path.clone())),
                );
                if relay_to_clients
                    && state.status_barrier.status.state == NETWORK_STATE_LOBBY
                    && resource_owner != HOST_CLIENT_ID as i32
                    && !loaded.newly_loading_resource_ids.is_empty()
                    && resource_owner_client_id
                        .is_some_and(|client_id| state.clients.contains_key(&client_id))
                    && resource_owner_is_authorized
                    && loaded
                        .newly_loading_resource_ids
                        .iter()
                        .all(|resource_id| resource_id >> 16 == resource_owner)
                {
                    // OnClientConnect uses this same targeted stock discovery
                    // packet. Reusing it here keeps the 15-ID wire cap and
                    // puts the just-added resource first.
                    let catalog = state
                        .resource_backend
                        .as_ref()
                        .map(|backend| backend.catalog())
                        .unwrap_or(&state.resource_catalog);
                    prompt_player_resource_discovery = catalog.on_peer_connected(resource_owner);
                }
                if let Ok(normalized) = crate::encode_control_entry_payload(&control) {
                    local_data = normalized;
                }
            }
            if relay_to_clients {
                let _ = broadcast_host_message(
                    state,
                    ConnectionTrafficClass::Message,
                    ControlMessage::Packet {
                        delivery,
                        data: data.clone(),
                    },
                    origin,
                );
            }
            // Queue the authoritative PlayerInfo for every other participant
            // before asking its owner for resource status. The owner's answer
            // can then only be relayed after peers know what the ID denotes.
            dispatch_host_resource_actions(prompt_player_resource_discovery, state).await;
            let _ = state
                .event_tx
                .send(HostEvent::Direct {
                    client_id: origin.unwrap_or(BROADCAST_CLIENT_ID),
                    delivery,
                    data: local_data,
                })
                .await;
        }
    }
}

pub(crate) fn authenticated_single_control(
    data: &[u8],
    expected_author: i32,
) -> Result<clonk_engine::ControlPacket, String> {
    let control = decode_control_entry_payload(data)
        .map_err(|error| format!("invalid single control packet: {error}"))?;
    let author = match &control {
        clonk_engine::ControlPacket::ClientJoin(data) => data.by_client,
        clonk_engine::ControlPacket::ClientUpdate(data) => data.by_client,
        clonk_engine::ControlPacket::ClientRemove(data) => data.by_client,
        clonk_engine::ControlPacket::PlayerSelect(data) => data.by_client,
        clonk_engine::ControlPacket::PlayerControl(data) => data.by_client,
        clonk_engine::ControlPacket::PlayerCommand(data) => data.by_client,
        clonk_engine::ControlPacket::Script(data) => data.by_client,
        clonk_engine::ControlPacket::MessageBoardAnswer(data) => data.by_client,
        clonk_engine::ControlPacket::Message(data) => data.by_client,
        clonk_engine::ControlPacket::CustomCommand(data) => data.by_client,
        clonk_engine::ControlPacket::EmMoveObject(data) => data.by_client,
        clonk_engine::ControlPacket::EmDrawTool(data) => data.by_client,
        clonk_engine::ControlPacket::EmDropDef(data) => data.by_client,
        clonk_engine::ControlPacket::ActivateGameGoalMenu(data) => data.by_client,
        clonk_engine::ControlPacket::ToggleHostility(data) => data.by_client,
        clonk_engine::ControlPacket::ActivateGameGoalRule(data) => data.by_client,
        clonk_engine::ControlPacket::SetPlayerTeam(data) => data.by_client,
        clonk_engine::ControlPacket::EliminatePlayer(data) => data.by_client,
        clonk_engine::ControlPacket::InitScenarioPlayer(data) => data.by_client,
        clonk_engine::ControlPacket::SurrenderPlayer(data) => data.by_client,
        clonk_engine::ControlPacket::Synchronize(data) => data.by_client,
        clonk_engine::ControlPacket::SyncCheck(data) => data.by_client,
        clonk_engine::ControlPacket::JoinPlayer(data) => data.by_client,
        clonk_engine::ControlPacket::RemovePlayer(data) => data.by_client,
        clonk_engine::ControlPacket::PlayerInfo(data) => data.by_client,
        clonk_engine::ControlPacket::Vote(data) | clonk_engine::ControlPacket::VoteEnd(data) => {
            data.by_client
        }
        clonk_engine::ControlPacket::Set(data) => data.by_client,
        // C4ControlDebugRec contains only its opaque StdBuf. The authenticated
        // control envelope is therefore its sole author identity.
        clonk_engine::ControlPacket::DebugRecord(_) => expected_author,
        clonk_engine::ControlPacket::Unknown { .. } => {
            return Err("unsupported single control packet".to_string());
        }
    };
    if author != expected_author {
        return Err(format!(
            "single control claimed author {author}, but authenticated author is {expected_author}"
        ));
    }
    Ok(control)
}

pub(crate) fn control_requires_host_ingress(control: &clonk_engine::ControlPacket) -> bool {
    matches!(
        control,
        clonk_engine::ControlPacket::ClientJoin(_)
            | clonk_engine::ControlPacket::ClientUpdate(_)
            | clonk_engine::ControlPacket::ClientRemove(_)
            | clonk_engine::ControlPacket::VoteEnd(_)
            | clonk_engine::ControlPacket::EliminatePlayer(_)
            | clonk_engine::ControlPacket::Synchronize(_)
            | clonk_engine::ControlPacket::RemovePlayer(_)
            | clonk_engine::ControlPacket::PlayerInfo(_)
    )
}

pub(crate) fn validate_peer_control_packet(
    packet: &ControlPacket,
    peer_id: ClientId,
) -> Result<(), String> {
    if packet.client_id() != peer_id {
        return Err(format!(
            "peer {peer_id} sent a control contribution for client {}",
            packet.client_id()
        ));
    }
    validate_control_envelope(packet)
        .map_err(|error| format!("invalid control packet: {error}"))?;
    validate_queued_control_authors(packet)?;
    let (controls, _) = packet
        .decoded_control_list()
        .map_err(|error| format!("invalid control packet: {error}"))?;
    if controls.iter().any(control_requires_host_ingress) {
        return Err("peer control contribution contains a host-authority control".to_string());
    }
    Ok(())
}

pub(crate) fn validate_peer_control_or_recovery(
    packet: &ControlPacket,
    peer_id: ClientId,
    recovery_from_tick: Option<Tick>,
) -> Result<(), String> {
    if recovery_from_tick.is_some_and(|from_tick| packet.tick() >= from_tick) {
        return validate_control_envelope(packet)
            .map(|_| ())
            .map_err(|error| format!("invalid recovery control packet: {error}"));
    }
    validate_peer_control_packet(packet, peer_id)
}

pub(crate) fn extend_peer_recovery_window(recovery_from_tick: &mut Option<Tick>, from_tick: Tick) {
    *recovery_from_tick =
        Some(recovery_from_tick.map_or(from_tick, |outstanding| outstanding.min(from_tick)));
}

pub(crate) async fn broadcast_exec_sync(control_tick: Tick, state: &mut HostState) {
    if state.pending_sync.is_empty() {
        return;
    }
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::ExecSync { control_tick },
        None,
    );
    let controls = std::mem::take(&mut state.pending_sync);
    apply_host_membership_controls(&controls, state).await;
    let _ = state
        .event_tx
        .send(HostEvent::SyncScheduled {
            control_tick,
            controls,
        })
        .await;
}

async fn execute_frozen_sync(control_tick: Tick, state: &mut HostState) {
    if state.pending_sync.is_empty() {
        return;
    }
    let controls = std::mem::take(&mut state.pending_sync);
    apply_host_membership_controls(&controls, state).await;
    let _ = state
        .event_tx
        .send(HostEvent::SyncScheduled {
            control_tick,
            controls,
        })
        .await;
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::ExecSync { control_tick },
        None,
    );
}

async fn apply_host_membership_controls(
    controls: &[clonk_engine::ControlPacket],
    state: &mut HostState,
) {
    for control in controls {
        match control {
            clonk_engine::ControlPacket::ClientUpdate(update)
                if update.by_client == HOST_CLIENT_ID as i32 =>
            {
                let Ok(client_id) = ClientId::try_from(update.client_id) else {
                    continue;
                };
                match update.update_type {
                    clonk_engine::CLIENT_UPDATE_ACTIVATE => {
                        let activated = update.data != 0;
                        if let Some(core) = state.client_cores.get_mut(&update.client_id) {
                            core.activated = activated;
                            core.observer = false;
                        } else {
                            continue;
                        }
                        if let Some(client) = state.clients.get_mut(&client_id) {
                            client.core.activated = activated;
                            client.core.observer = false;
                        }
                        if activated {
                            let _ = state.coordination_register(client_id);
                        } else {
                            coordination_unregister(client_id, state).await;
                        }
                    }
                    clonk_engine::CLIENT_UPDATE_SET_OBSERVER => {
                        if let Some(core) = state.client_cores.get_mut(&update.client_id) {
                            core.activated = false;
                            core.observer = true;
                        } else {
                            continue;
                        }
                        if let Some(client) = state.clients.get_mut(&client_id) {
                            client.core.activated = false;
                            client.core.observer = true;
                        }
                        coordination_unregister(client_id, state).await;
                    }
                    _ => {}
                }
            }
            clonk_engine::ControlPacket::ClientRemove(remove)
                if remove.by_client == HOST_CLIENT_ID as i32 =>
            {
                apply_host_client_remove(remove.client_id, state).await;
            }
            _ => {}
        }
    }
}

async fn apply_host_client_remove(client_id: i32, state: &mut HostState) {
    if let Ok(client_id) = ClientId::try_from(client_id) {
        close_removed_client_connections(client_id, state).await;
        coordination_unregister(client_id, state).await;
    }
    if let Some(core) = state.client_cores.remove(&client_id) {
        state.admission.remove_client_name(&core.name);
        state.invalidate_control_send_time();
    }
    state.client_addresses.remove(&client_id);
    state.resource_catalog.remove_at_client(client_id);
    if let Some(backend) = state.resource_backend.as_mut() {
        backend.remove_at_client(client_id);
    }
    state.pending_kinds.remove(&client_id);
}

pub(crate) async fn finish_host_restart_removals(state: &mut HostState) {
    let removing = state.removing_clients.iter().copied().collect::<Vec<_>>();
    for client_id in removing {
        if let Ok(client_id) = i32::try_from(client_id) {
            apply_host_client_remove(client_id, state).await;
        }
    }
}

async fn close_removed_client_connections(client_id: ClientId, state: &mut HostState) {
    state.peer_capabilities.forget(client_id as i32);
    let routes = state
        .accepted_routes
        .iter()
        .filter(|(_, route)| route.client_id == client_id)
        .map(|(connection_id, route)| (*connection_id, route.outbound.clone()))
        .collect::<Vec<_>>();
    for (connection_id, _) in &routes {
        state.accepted_routes.remove(connection_id);
    }
    if !routes.is_empty() {
        state.invalidate_control_send_time();
    }
    invalidate_pending_client_routes(client_id, state);
    state
        .pending_post_mortems
        .retain(|_, (pending_client_id, _, _)| *pending_client_id != client_id);
    state.closed_routes.remove_client(client_id);
    let removed_client = state.clients.remove(&client_id);
    let barrier_effects = state.status_barrier.remove_remote(client_id);
    let reply = crate::ConnectionReply {
        ok: false,
        message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec())
            .unwrap_or_default(),
        wrong_password: false,
        port_protocol: false,
    };
    for (_, outbound) in routes {
        if outbound.try_close(reply.clone()).is_err() {
            outbound.retire();
        }
    }
    if removed_client.is_some() {
        let _ = state
            .event_tx
            .send(HostEvent::ClientLeft { client_id })
            .await;
    }
    Box::pin(apply_barrier_effects(barrier_effects, state)).await;
    state.removing_clients.remove(&client_id);
}

async fn coordination_unregister(client_id: ClientId, state: &mut HostState) {
    let ready_batches = state
        .coordinator
        .remove_client(client_id)
        .unwrap_or_default();
    state.scheduler.remove_client(client_id);
    resolve_host_ready(ready_batches, state).await;
}

pub(crate) async fn broadcast_status(
    status: NetworkStatus,
    acknowledgement: bool,
    state: &mut HostState,
) {
    let message = if acknowledgement {
        ControlMessage::StatusAck(status)
    } else {
        ControlMessage::Status(status)
    };
    let _ = broadcast_host_message(state, ConnectionTrafficClass::Message, message, None);
}

pub(crate) async fn broadcast_ready_check(
    packet: ReadyCheckPacket,
    except_client_id: Option<ClientId>,
    state: &mut HostState,
) {
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::ReadyCheck(packet),
        except_client_id,
    );
}

pub(crate) async fn broadcast_lobby_countdown(packet: LobbyCountdownPacket, state: &mut HostState) {
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::LobbyCountdown(packet),
        None,
    );
}

pub(crate) async fn broadcast_league_round_results(
    packet: crate::LeagueRoundResultsPacket,
    state: &mut HostState,
) {
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::LeagueRoundResults(packet),
        None,
    );
}

pub(crate) async fn broadcast_host_restarting(rejoin_seconds: u16, state: &mut HostState) {
    let _ = broadcast_host_message(
        state,
        ConnectionTrafficClass::Message,
        ControlMessage::HostRestarting { rejoin_seconds },
        None,
    );
}

pub(crate) fn queue_host_restart_lobby(
    retained_routes: &BTreeMap<ClientId, u32>,
    state: &mut HostState,
) -> Result<(), String> {
    let marker = ControlMessage::HostRestartLobby {
        restart_nonce: state.round_restart_nonce,
    };
    for (client_id, connection_id) in retained_routes {
        let route = state
            .accepted_routes
            .get(connection_id)
            .filter(|route| route.client_id == *client_id)
            .ok_or_else(|| format!("retained restart route {connection_id} disappeared"))?;
        route
            .outbound
            .try_send(marker.clone())
            .map_err(|_| format!("retained restart route {connection_id} closed"))?;
    }
    Ok(())
}

pub(crate) fn apply_ready_check_to_host_state(packet: ReadyCheckPacket, state: &mut HostState) {
    if packet.data.vote_requested() {
        return;
    }
    let ready = packet.data.is_ready();
    if let Some(core) = state.client_cores.get_mut(&packet.client_id) {
        core.lobby_ready = ready;
    }
    if let Ok(client_id) = ClientId::try_from(packet.client_id) {
        if let Some(client) = state.clients.get_mut(&client_id) {
            client.core.lobby_ready = ready;
        }
    }
}

pub(crate) fn contiguous_client_controls(
    backlog: &ControlBacklog,
    client_id: ClientId,
    from_tick: Tick,
) -> Vec<ControlPacket> {
    let mut expected_tick = from_tick;
    let mut contiguous = Vec::new();
    for (tick, packets) in backlog.packets_from(from_tick) {
        if tick != expected_tick {
            break;
        }
        let Some(packet) = packets
            .into_iter()
            .find(|packet| packet.client_id() == client_id)
        else {
            break;
        };
        contiguous.push(packet);
        expected_tick = expected_tick.saturating_add(1);
    }
    contiguous
}

pub(crate) fn contiguous_complete_controls(
    backlog: &ControlBacklog,
    from_tick: Tick,
) -> Result<Vec<ControlPacket>, String> {
    let mut expected_tick = from_tick;
    let mut contiguous = Vec::new();
    for (tick, packets) in backlog.packets_from(from_tick) {
        if tick != expected_tick {
            break;
        }
        let Some(complete) = packets
            .into_iter()
            .find(|packet| packet.client_id() == BROADCAST_CLIENT_ID)
        else {
            break;
        };
        contiguous.push(complete);
        expected_tick = expected_tick.saturating_add(1);
    }
    Ok(contiguous)
}

async fn apply_host_control_mode(mode: i32, from_tick: i32, state: &mut HostState) {
    if state.control_mode == mode {
        return;
    }
    state.control_mode = mode;
    state.invalidate_control_send_time();
    let Ok(from_tick) = Tick::try_from(from_tick) else {
        return;
    };
    let packets = match mode {
        0 => contiguous_client_controls(&state.local_control_backlog, HOST_CLIENT_ID, from_tick),
        1 => match contiguous_complete_controls(&state.backlog, from_tick) {
            Ok(packets) => packets,
            Err(error) => {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: None,
                        error,
                    })
                    .await;
                return;
            }
        },
        _ => Vec::new(),
    };
    for packet in packets {
        broadcast_control(&packet, state).await;
    }
}

pub(crate) async fn apply_barrier_effects(effects: Vec<BarrierEffect>, state: &mut HostState) {
    let mut committed = false;
    for effect in effects {
        match effect {
            BarrierEffect::InvalidateReference
            | BarrierEffect::DriveControlTo(_)
            | BarrierEffect::StopControl
            | BarrierEffect::SweepUnjoinedPlayers
            | BarrierEffect::StartControl => {}
            BarrierEffect::SetControlMode { mode, from_tick } => {
                apply_host_control_mode(mode, from_tick, state).await
            }
            BarrierEffect::BroadcastStatus(status) => {
                if status.state != NETWORK_STATE_LOBBY {
                    state.lobby_chat_history.clear();
                }
                broadcast_status(status, false, state).await;
                let _ = state.event_tx.send(HostEvent::StatusChanged(status)).await;
            }
            BarrierEffect::ExecutePendingSyncControls(actual_control_tick) => {
                if let Ok(control_tick) = Tick::try_from(actual_control_tick) {
                    broadcast_exec_sync(control_tick, state).await;
                }
            }
            BarrierEffect::BroadcastStatusAck(status) => {
                broadcast_status(status, true, state).await;
                if status.state == NETWORK_STATE_GO {
                    state.game_started = true;
                    // Once control is flowing, bulk sitting ahead of it in the
                    // shared ordered stream is what freezes everybody, so the
                    // per-peer window narrows. In the lobby the opposite is
                    // true: nothing is being blocked and a fast join is what the
                    // player is waiting for.
                    // Same as the client side: the backend's catalog is what
                    // schedules whenever there is a backend.
                    state
                        .resource_catalog
                        .set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME);
                    if let Some(backend) = state.resource_backend.as_mut() {
                        backend.set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME);
                    }
                }
                committed = true;
            }
            BarrierEffect::SendStatusAck { client_id, status } => {
                let _ = send_host_message(
                    state,
                    client_id,
                    ConnectionTrafficClass::Message,
                    ControlMessage::StatusAck(status),
                )
                .await;
            }
        }
    }
    if committed {
        let _ = state
            .event_tx
            .send(HostEvent::StatusCommitted(state.status_barrier.status))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_wait_attribution_distinguishes_the_late_recipient_from_healthy_peers() {
        let waiting = BTreeSet::from([7]);
        let none = BTreeSet::new();

        assert_eq!(
            control_wait_attribution_for(73, 7, &waiting, &none),
            Some(crate::ControlWaitAttribution {
                tick: 73,
                waited_for_recipient: true,
                waited_for_other: false,
                discarded_recipient_control: false,
            })
        );
        assert_eq!(
            control_wait_attribution_for(73, 8, &waiting, &none),
            Some(crate::ControlWaitAttribution {
                tick: 73,
                waited_for_recipient: false,
                waited_for_other: true,
                discarded_recipient_control: false,
            })
        );
        assert_eq!(
            control_wait_attribution_for(73, 8, &BTreeSet::new(), &none),
            None
        );
    }

    /// Only the client the deadline actually gave up on is told its input was
    /// dropped. A peer that was merely waited for — and delivered before the
    /// budget ran out — is not, or every healthy client in a session with one
    /// bad peer would report losing input it never lost.
    #[test]
    fn only_the_client_the_deadline_gave_up_on_is_told_its_control_was_dropped() {
        let waiting = BTreeSet::from([7, 8]);
        let discarded = BTreeSet::from([7]);

        let dropped = control_wait_attribution_for(73, 7, &waiting, &discarded)
            .expect("the tick had a wait to attribute");
        assert!(dropped.waited_for_recipient);
        assert!(dropped.discarded_recipient_control);

        let waited_only = control_wait_attribution_for(73, 8, &waiting, &discarded)
            .expect("the tick had a wait to attribute");
        assert!(waited_only.waited_for_recipient);
        assert!(
            !waited_only.discarded_recipient_control,
            "8 was waited for but delivered, so it lost nothing"
        );

        let bystander = control_wait_attribution_for(73, 9, &waiting, &discarded)
            .expect("the tick had a wait to attribute");
        assert!(!bystander.waited_for_recipient);
        assert!(!bystander.discarded_recipient_control);
    }
}
