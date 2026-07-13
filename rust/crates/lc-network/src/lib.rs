mod address_packet;
mod admission;
mod advertise;
mod connection_handshake;
mod connection_liveness;
mod client_bootstrap;
mod client_player_resource;
mod control;
mod forward_packet;
mod host_resource_core;
mod host_game_reference;
mod host_initial_resources;
mod initial_network_dynamic;
mod initial_network_metadata;
mod initial_network_parameters;
mod join_client_registry;
mod join_player_registry;
mod join_team_registry;
mod league;
mod legacy;
mod lobby;
mod local_resource_resolution;
mod name_validation;
mod post_mortem;
mod puncher;
mod resource_catalog;
mod resource_file_store;
mod resource_packet;
mod resource_transfer_backend;
mod resync;
mod search;
mod session;
mod status;
mod transport;
mod udp;

pub use address_packet::{
    append_received_address, decode_address_packet_payload, encode_address_packet_payload,
    AddressInsertion, AddressPacket, AddressPacketDecodeError, NetworkAddress, NetworkProtocol,
    PID_ADDR,
};
pub use admission::{
    AdmissionDecision, ClientAdmission, ConnectionAction, ConnectionStatus, HostAdmission,
    KnownPeerAdmission, LegacyConnection,
};
pub use advertise::{
    discovery_reply_for_packet, encode_host_game_reference_response, encode_reference_response,
    HostGameAdvertiserError, NetworkGameAdvertiser, NetworkGameAdvertiserConfig,
};
pub use join_client_registry::{reconcile_join_client_registry, JoinClientRegistrySnapshot};
pub use join_player_registry::{ClientPlayerInfosSnapshot, PlayerInfoListSnapshot};
pub use join_team_registry::{JoinTeamListSnapshot, JoinTeamSnapshot};
pub use lc_engine::{InitScenarioPlayerControlData, PlayerInfoUpdateRequest};

pub use connection_handshake::{
    run_client_connection_handshake, run_host_connection_handshake, ClientConnectionHandshake,
    ConnectionHandshakeError, ConnectionLivenessState, HostAdmissionRequest,
    HostConnectionHandshake,
};
pub use connection_liveness::{
    ConnectionLiveness, ConnectionTimeout, LivenessClock, LivenessPhase, PingProbe, PingSchedule,
    ACCEPT_TIMEOUT_SECONDS, NETWORK_TIMER_INTERVAL_MS, PACKET_LOG_START, PING_FREQUENCY_MS,
    PING_TIMEOUT_MS,
};
pub use client_bootstrap::{
    plan_client_bootstrap, ClientBootstrapLocalCandidates, ClientBootstrapPlan,
    ClientBootstrapPlanError, ClientBootstrapResourcePlan, ClientBootstrapResourceRole,
    ClientBootstrapResourceSource,
};
pub use client_player_resource::{
    publish_client_player_resource, ClientPlayerResourcePublication, ClientPlayerResourceRequest,
    ClientPlayerResourcePublicationError, ClientPlayerResourcePublicationSpec,
};
pub use control::{
    ControlCoordinator, ControlError, ControlOutcome, ControlPacket, ControlPacketBuilder,
    InsertStatus, MissingRange, ReadyBatch,
};
pub use forward_packet::{
    decode_forward_packet_payload, encode_forward_packet_payload, ForwardPacket,
    ForwardPacketCodecError, MAX_FORWARD_CLIENTS, PID_FORWARD, PID_FORWARD_REQUEST,
};
pub use host_resource_core::{
    build_host_resource_core, HostResourceCoreError, HostResourceCoreSpec,
    HostResourcePublication, HostResourceType,
};
pub use host_game_reference::{
    HostGameReference, HostGameReferenceError, HostGameReferenceMetadata,
};
pub use host_initial_resources::{
    publish_host_initial_resources, HostInitialResourcePublication,
    HostInitialResourcePublicationError, HostInitialResourcePublicationSpec,
    HostInitialResourceSource,
};
pub use initial_network_dynamic::{
    compose_initial_network_dynamic, InitialNetworkDynamic, InitialNetworkDynamicEntry,
    InitialNetworkDynamicError, InitialNetworkDynamicSpec,
};
pub use initial_network_metadata::{
    fill_scenario_derived_join_parameters, initial_network_scenario_defaults,
    join_team_list_snapshot, InitialNetworkMetadataError,
};
pub use initial_network_parameters::{
    serialize_initial_network_parameters, InitialNetworkParametersError,
    InitialNetworkScenarioDefaults,
};
pub use league::{
    decode_league_auth_response, decode_league_join_response, encode_league_auth_request,
    encode_league_auth_request_head, encode_league_join_request, encode_league_join_request_head,
    encode_league_player_info_section, solve_league_checksum, LeagueAuthRequestHead,
    LeagueAuthResponse, LeagueChecksumError, LeagueFbidRegistry, LeagueHttpPostTransport,
    LeagueHttpTransportConfig, LeagueHttpTransportError, LeagueJoinRequestHead, LeagueJoinResponse,
    LeaguePlayerInfoEncodeError, LEAGUE_HTTP_TIMEOUT, LEAGUE_HTTP_USER_AGENT, MAX_LEAGUES,
};
pub use legacy::{
    aggregate_ready_batch, decode_control_entry_payload, decode_control_packet,
    decode_control_payload, decode_init_scenario_player_control_entry_payload,
    decode_join_data_envelope, decode_join_game_parameters_envelope,
    decode_player_info_update_payload, encode_control_entry_payload, encode_control_packet,
    encode_control_payload, encode_init_scenario_player_control_entry_payload,
    encode_join_data_envelope, encode_join_game_parameters_envelope,
    encode_player_info_update_payload, JoinDataC4Id, JoinDataEnvelope, JoinDataIdListEntry,
    JoinGameParametersEnvelope, LegacyAggregateError, LegacyControlError, LegacyControlFrame,
    LegacyEncodeError,
};
pub use lobby::{Lobby, LobbyError, LobbyParticipant, LobbySettings, ParticipantKind};
pub use local_resource_resolution::{
    resolve_local_resource, LocalResourceMatch, LocalResourceResolution,
    LocalResourceResolutionError, NonLoadableResourceMismatch,
};
pub use post_mortem::{PostMortemPacket, RecoverablePacketLog};
pub use puncher::{
    decode_netpuncher_packet, encode_netpuncher_packet, reduce_puncher_connect,
    NetpuncherAddressFamily, NetpuncherGameIds, NetpuncherPacket, NetpuncherPacketDecodeError,
    NetpuncherRole, NetpuncherRuntimeState, NETPUNCHER_PROTOCOL_VERSION,
};
pub use resource_catalog::{
    ChunkSet, ChunkStoreOutcome, OutstandingLoad, PeerStatusOutcome, ResourceCatalog,
    ResourceCatalogAction, ResourceLoadPoll, ResourceRegistration,
    RESOURCE_DISCOVER_INTERVAL_SECONDS, RESOURCE_DISCOVER_TIMEOUT_SECONDS,
    RESOURCE_LOAD_TIMEOUT_SECONDS, RESOURCE_MAX_LOADS, RESOURCE_MAX_LOAD_PER_PEER_PER_FILE,
    RESOURCE_STATUS_INTERVAL_SECONDS,
};
pub use resource_file_store::{
    ChunkWriteOutcome, ResourceFileOwnership, ResourceFileStore, ResourceFileStoreError,
};
pub use resource_packet::{
    decode_resource_core_payload, decode_resource_data_payload, decode_resource_discover_payload,
    decode_resource_packet, decode_resource_request_payload, decode_resource_status_payload,
    encode_resource_core_payload, encode_resource_data_payload, encode_resource_discover_payload,
    encode_resource_packet, encode_resource_request_payload, encode_resource_status_payload,
    ResourceChunkAvailability, ResourceChunkRange, ResourceDataPacket, ResourceDiscoverPacket,
    ResourcePacket, ResourcePacketCodecError, ResourceRequestPacket, ResourceStatusPacket,
    DISCOVER_RESOURCE_ID_CAPACITY, MAX_STOCK_DISCOVER_RESOURCE_IDS, MAX_STOCK_RESOURCE_DATA_BYTES,
    PID_NET_RES_DATA, PID_NET_RES_DERIVE, PID_NET_RES_DISCOVER, PID_NET_RES_REQUEST,
    PID_NET_RES_STATUS,
};
pub use resource_transfer_backend::{
    ResourceTransferBackend, ResourceTransferError, ResourceTransferEvent,
};
pub use resync::{ControlBacklog, ResyncRequest, ResyncScheduler};
pub use search::{
    fetch_reference_endpoint, fetch_reference_endpoint_with_config, parse_reference_response,
    LanProbeTrigger, NetworkGameReference, NetworkGameSearch, NetworkGameSearchConfig,
    ReferenceEndpoint, ReferenceFetchError, ReferenceParseError, ReferenceQueryConfig,
    ReferenceQuerySource, SearchCommand, StartupGameSearch, StartupGameSearchEvent,
    CURRENT_GAME_BUILD, CURRENT_GAME_VERSION, DEFAULT_DISCOVERY_PORT, DEFAULT_MASTER_SERVER_URL,
    DEFAULT_REFERENCE_PORT, GAME_SEARCH_INTERVAL, MAX_LAN_DISCOVERS, REFERENCE_QUERY_TIMEOUT,
};
pub use session::{
    connect_client, start_host, ClientCommand, ClientConfig, ClientError, ClientEvent,
    ClientHandle, HostCommand, HostConfig, HostError, HostEvent, HostHandle, HostJoinSnapshot,
    HostedResourceFile, BROADCAST_CLIENT_ID,
};
pub use status::{BarrierEffect, BarrierPhase, RemoteBarrierState, StatusBarrier};
pub use transport::{
    decode_connection_reply_payload, decode_connection_request_payload,
    encode_connection_reply_payload, encode_connection_request_payload, ConnectionReply,
    ConnectionRequest, ControlDelivery, ControlMessage, ControlTransport, LobbyCountdownPacket,
    NetworkStatus, PingPacket, ReadyCheckData, ReadyCheckPacket, TransportError, NETWORK_STATE_GO,
    NETWORK_STATE_INIT, NETWORK_STATE_LOBBY, NETWORK_STATE_NONE, NETWORK_STATE_PAUSE,
};
pub use udp::{
    decode_reliable_udp_check, decode_reliable_udp_connect_ok, decode_reliable_udp_data_fragment,
    encode_reliable_udp_check, encode_reliable_udp_connect, encode_reliable_udp_data_fragments,
    ReliableUdpCheck, ReliableUdpConnect, ReliableUdpConnectOk, ReliableUdpDataFragment,
    ReliableUdpDecodeError, ReliableUdpEncodeError, ReliableUdpMulticastMode,
    RELIABLE_UDP_DATA_PAYLOAD_LIMIT, RELIABLE_UDP_PROTOCOL_VERSION,
};

pub type ClientId = u32;
pub type Tick = u32;
