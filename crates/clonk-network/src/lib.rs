mod address_packet;
mod admission;
mod advertise;
mod capabilities;
mod client_bootstrap;
mod client_mesh;
mod client_player_resource;
mod connection_handshake;
mod connection_liveness;
mod control;
mod control_latency;
mod control_record;
mod forward_packet;
mod host_game_reference;
mod host_initial_resources;
mod host_resource_core;
mod http_backend;
mod initial_network_dynamic;
mod initial_network_metadata;
mod initial_network_parameters;
mod irc;
mod join_client_registry;
mod join_player_registry;
mod join_team_registry;
mod league;
mod league_round_results_packet;
mod league_stream;
mod legacy;
mod live_network_dynamic;
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
mod sim;
mod sim_session;
mod statistics;
mod stats;
mod status;
mod transport;
mod udp;
mod udp_runtime;
mod udp_session;
mod upnp;

pub use address_packet::{
    append_received_address, decode_address_packet_payload, decode_tcp_sim_open_packet_payload,
    encode_address_packet_payload, encode_tcp_sim_open_packet_payload, AddressInsertion,
    AddressPacket, AddressPacketDecodeError, NetworkAddress, NetworkProtocol, TcpSimOpenPacket,
    PID_ADDR, PID_TCP_SIM_OPEN,
};
pub use admission::{
    AdmissionDecision, ClientAdmission, ConnectionAction, ConnectionStatus, HostAdmission,
    KnownPeerAdmission, LegacyConnection,
};
pub use advertise::{
    discovery_reply_for_packet, encode_host_game_reference_response, encode_reference_response,
    HostGameAdvertiserError, NetworkGameAdvertiser, NetworkGameAdvertiserConfig,
};
pub use capabilities::{
    decode_port_capabilities, encode_port_capabilities, PeerCapabilityRegistry, PortCapabilities,
    PID_PORT_CAPABILITIES, PORT_CAPABILITY_VERSION,
};
pub use clonk_engine::{InitScenarioPlayerControlData, PlayerInfoUpdateRequest};
pub use control_latency::ControlLatencyEstimator;
pub use join_client_registry::{reconcile_join_client_registry, JoinClientRegistrySnapshot};
pub use join_player_registry::{ClientPlayerInfosSnapshot, PlayerInfoListSnapshot};
pub use join_team_registry::{JoinTeamListSnapshot, JoinTeamSnapshot};

pub use client_bootstrap::{
    plan_client_bootstrap, plan_client_bootstrap_with_group_maker, ClientBootstrapLocalCandidates,
    ClientBootstrapPlan, ClientBootstrapPlanError, ClientBootstrapResourcePlan,
    ClientBootstrapResourceRole, ClientBootstrapResourceSource,
};
pub use client_mesh::{
    client_mesh_local_addresses, client_mesh_puncher_variants, client_mesh_tcp_sim_open_eligible,
    ClientMeshAddressState, ClientMeshConnectDecision, ClientMeshConnectivity,
    ClientMeshDialAttempt, ClientMeshPeerState, ClientMeshPuncherUpdate,
    CLIENT_MESH_CONNECT_ATTEMPTS, CLIENT_MESH_CONNECT_BACKOFF, CLIENT_MESH_CONNECT_INTERVAL,
};
pub use client_player_resource::{
    publish_client_player_resource, ClientPlayerResourcePublication,
    ClientPlayerResourcePublicationError, ClientPlayerResourcePublicationSpec,
    ClientPlayerResourceRequest,
};
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
pub use control::{
    ControlCoordinator, ControlError, ControlOutcome, ControlPacket, ControlPacketBuilder,
    InsertStatus, MissingRange, ReadyBatch,
};
pub use control_record::{
    decode_control_record, decode_control_record_text, encode_control_record_binary,
    encode_control_record_text, rewrite_control_record_binary, rewrite_control_record_text,
    ControlRecordChunk, ControlRecordDecodeError, ControlRecordParser, ControlRecordPlayback,
    ControlRecordRewriteError, ControlRecordWriter,
};
pub use forward_packet::{
    decode_forward_packet_payload, encode_forward_packet_payload, ForwardPacket,
    ForwardPacketCodecError, MAX_FORWARD_CLIENTS, PID_FORWARD, PID_FORWARD_REQUEST,
};
pub use host_game_reference::{
    encode_player_info_list_ini, HostGameReference, HostGameReferenceError,
    HostGameReferenceMetadata,
};
pub use host_initial_resources::{
    publish_host_initial_resources, HostInitialResourcePublication,
    HostInitialResourcePublicationError, HostInitialResourcePublicationSpec,
    HostInitialResourceSource,
};
pub use host_resource_core::{
    build_host_resource_core, HostResourceCoreError, HostResourceCoreSpec, HostResourcePublication,
    HostResourceType, MAX_PLAYER_BIG_ICON_SIZE,
};
pub use http_backend::{HttpBackend, NETIO_HAPPY_EYEBALLS_TIMEOUT, NETIO_QUERY_TIMEOUT};
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
pub use irc::{
    resolve_irc_server, IrcChannel, IrcClientError, IrcClientEvent, IrcClientHandle,
    IrcClientSnapshot, IrcClientState, IrcCommand, IrcConnectConfig, IrcConnectionState,
    IrcLineDecoder, IrcMessage, IrcMessageType, IrcReduceResult, IrcStatusTemplates, IrcUser,
    IRC_DEFAULT_PORT, IRC_MAX_LOG_LENGTH, IRC_MAX_READ_LOG_LENGTH,
};
pub use league::{
    decode_league_auth_response, decode_league_end_response, decode_league_join_response,
    decode_league_report_disconnect_response, decode_league_start_response,
    decode_league_update_response, decode_player_info_list_ini, encode_league_auth_request,
    encode_league_auth_request_head, encode_league_end_request, encode_league_join_request,
    encode_league_join_request_head, encode_league_player_info_section,
    encode_league_report_disconnect_request, encode_league_start_request,
    encode_league_update_request, solve_league_checksum, LeagueAuthRequestHead, LeagueAuthResponse,
    LeagueChecksumError, LeagueDisconnectReason, LeagueEndRecord, LeagueEndResponse,
    LeagueFbidRegistry, LeagueHeartbeat, LeagueHostSession, LeagueHttpPostTransport,
    LeagueHttpTransportConfig, LeagueHttpTransportError, LeagueJoinRequestHead, LeagueJoinResponse,
    LeaguePlayerInfoEncodeError, LeagueReferenceRequestEncodeError, LeagueResponseDecodeError,
    LeagueStartResponse, LeagueUpdateResponse, PlayerInfoListIniError, LEAGUE_HTTP_TIMEOUT,
    LEAGUE_HTTP_USER_AGENT, LEAGUE_MIN_UPDATE_INTERVAL_SECONDS, MAX_LEAGUES,
};
pub use league_round_results_packet::{
    decode_league_round_results_payload, encode_league_round_results_payload,
    LeagueRoundPlayerStatus, LeagueRoundResultsDecodeError, LeagueRoundResultsEncodeError,
    LeagueRoundResultsPacket, LeagueRoundResultsPlayer, PID_LEAGUE_ROUND_RESULTS,
};
pub use league_stream::{
    decode_classic_record_stream, encode_league_stream_file_chunk, ClassicRecordStream,
    ClassicRecordStreamDecodeError, ClassicRecordStreamFile, LeagueRecordStream,
    LeagueRecordStreamError, LeagueRecordUpload, LEAGUE_STREAM_FILE_CHUNK_TYPE,
    LEAGUE_STREAM_INTERVAL_SECONDS, LEAGUE_STREAM_MAX_BLOCK_SIZE, LEAGUE_STREAM_MIN_BLOCK_SIZE,
};
pub use legacy::{
    aggregate_ready_batch, decode_control_entry_payload, decode_control_entry_prefix,
    decode_control_list_prefix, decode_control_packet, decode_control_payload,
    decode_init_scenario_player_control_entry_payload, decode_join_data_envelope,
    decode_join_game_parameters_envelope, decode_player_info_update_payload,
    encode_control_entry_payload, encode_control_list_payload, encode_control_packet,
    encode_control_payload, encode_init_scenario_player_control_entry_payload,
    encode_join_data_envelope, encode_join_game_parameters_envelope,
    encode_player_info_update_payload, JoinDataC4Id, JoinDataEnvelope, JoinDataIdListEntry,
    JoinGameParametersEnvelope, LegacyAggregateError, LegacyControlError, LegacyControlFrame,
    LegacyControlSet, LegacyEncodeError,
};
pub use live_network_dynamic::{
    compose_live_network_dynamic, LiveNetworkDynamic, LiveNetworkDynamicComponent,
    LiveNetworkDynamicEntry, LiveNetworkDynamicError, LiveNetworkDynamicSpec,
};
pub use lobby::{Lobby, LobbyError, LobbyParticipant, LobbySettings, ParticipantKind};
pub use local_resource_resolution::{
    resolve_local_resource, resolve_local_resource_with_group_maker, LocalResourceMatch,
    LocalResourceResolution, LocalResourceResolutionError, NonLoadableResourceMismatch,
};
pub use name_validation::{validate_name_allow_empty, validate_name_no_empty};
pub use post_mortem::{PostMortemPacket, RecoverablePacketLog};
pub use puncher::{
    decode_netpuncher_packet, encode_netpuncher_packet, encode_netpuncher_punch,
    reduce_puncher_connect, NetpuncherAddressFamily, NetpuncherGameIds, NetpuncherIoEvent,
    NetpuncherPacket, NetpuncherPacketDecodeError, NetpuncherRole, NetpuncherRuntimeState,
    NETPUNCHER_PROTOCOL_VERSION,
};
pub use resource_catalog::{
    ChunkSet, ChunkStoreOutcome, OutstandingLoad, PeerStatusOutcome, ResourceCatalog,
    ResourceCatalogAction, ResourceLoadPoll, ResourceRegistration,
    RESOURCE_DISCOVER_INTERVAL_SECONDS, RESOURCE_DISCOVER_TIMEOUT_SECONDS,
    RESOURCE_LOAD_TIMEOUT_SECONDS, RESOURCE_MAX_LOADS, RESOURCE_MAX_LOAD_PER_PEER_IN_GAME,
    RESOURCE_MAX_LOAD_PER_PEER_PER_FILE, RESOURCE_STATUS_INTERVAL_SECONDS,
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
    ResourceDerivation, ResourceTransferBackend, ResourceTransferError, ResourceTransferEvent,
};
pub use resync::{ControlBacklog, ResyncRequest, ResyncScheduler};
pub use search::{
    direct_reference_endpoint, fetch_reference_endpoint, fetch_reference_endpoint_with_config,
    fetch_reference_query_endpoint, fetch_reference_query_endpoint_with_config,
    parse_reference_query_response, parse_reference_query_response_with_config,
    parse_reference_response, LanProbeTrigger, MasterserverReplyInfo, MasterserverVersion,
    NetworkGameReference, NetworkGameSearch, NetworkGameSearchConfig, NetworkJoinRoutePlan,
    ReferenceEndpoint, ReferenceFetchError, ReferenceParseError, ReferenceQueryConfig,
    ReferenceQueryResponse, ReferenceQuerySource, SearchCommand, StartupGameSearch,
    StartupGameSearchEvent, CURRENT_GAME_BUILD, CURRENT_GAME_VERSION, DEFAULT_DISCOVERY_PORT,
    DEFAULT_MASTER_SERVER_URL, DEFAULT_REFERENCE_PORT, GAME_SEARCH_INTERVAL, MAX_LAN_DISCOVERS,
    REFERENCE_QUERY_TIMEOUT,
};
pub use session::{
    connect_client, connect_client_addresses, connect_dual_client, connect_udp_client, start_host,
    start_host_with_bindings, start_host_with_udp_binding, ClientCommand, ClientConfig,
    ClientError, ClientEvent, ClientHandle, ClientMeshPuncherConfig, ControlSendTimeSnapshot,
    HostCommand, HostConfig, HostError, HostEvent, HostHandle, HostJoinSnapshot, HostUdpBinding,
    HostedResourceFile, RuntimeLobbyClientTelemetry, RuntimeNetworkClientState,
    RuntimeNetworkConnection, BROADCAST_CLIENT_ID,
};
pub use sim::{
    mean, percentile, replay_lockstep, run_control_delivery, ControlDeliveryConfig, InFlight, Link,
    LinkConditions, LinkReport, LockstepPlayout, Lookahead, SimRng, CONTROL_PERIOD, STEP,
};
pub use sim_session::{
    run_session, ClientOutcome, ClientProfile, CpuProfile, PresendSource, SessionConfig,
    SessionReport, FRAME_INTERVAL,
};
pub use statistics::{
    ConnectionRateStatistics, ConnectionStatisticsKey, ConnectionStatisticsRecorder,
    NetworkIoStatistics, NetworkIoStatisticsSnapshot, ProtocolRateStatistics,
    NETWORK_STATISTICS_INTERVAL_MS, TCP_STATISTICS_HEADER_BYTES, UDP_STATISTICS_HEADER_BYTES,
};
pub use stats::{
    ClientPingSample, NetworkStats, NetworkStatsGraph, PlayerControlSample, ProtocolRateSample,
    TableGraph, CONTROL_GRAPH_AVERAGE, DEFAULT_GRAPH_BACKLOG, FORWARD_AVERAGE_FACTOR,
    PLAYER_GRAPH_BACKLOG,
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
    decode_reliable_udp_add_address, decode_reliable_udp_check, decode_reliable_udp_close,
    decode_reliable_udp_connect, decode_reliable_udp_connect_ok, decode_reliable_udp_data_fragment,
    encode_reliable_udp_add_address, encode_reliable_udp_check, encode_reliable_udp_close,
    encode_reliable_udp_connect, encode_reliable_udp_connect_ok,
    encode_reliable_udp_data_fragments, encode_reliable_udp_ping_response,
    reliable_udp_packet_kind, ReliableUdpAddAddress, ReliableUdpChannel, ReliableUdpCheck,
    ReliableUdpClose, ReliableUdpConnect, ReliableUdpConnectOk, ReliableUdpDataFragment,
    ReliableUdpDecodeError, ReliableUdpEncodeError, ReliableUdpMulticastMode,
    ReliableUdpPacketKind, ReliableUdpReassembledPacket, ReliableUdpReassemblyError,
    ReliableUdpReceiveWindow, RELIABLE_UDP_DATA_PAYLOAD_LIMIT, RELIABLE_UDP_PROTOCOL_VERSION,
    RELIABLE_UDP_RECHECK_INTERVAL,
};
pub use udp_runtime::{
    canonical_reliable_udp_peer_address, reliable_udp_send_address, ReliableUdpDatagram,
    ReliableUdpDisconnectReason, ReliableUdpDriverError, ReliableUdpEndpointCore, ReliableUdpEvent,
    ReliableUdpPeerStatus, ReliableUdpRuntimeError, ReliableUdpSocketDriver, ReliableUdpStep,
    RELIABLE_UDP_CHECK_INTERVAL, RELIABLE_UDP_CONNECT_RETRIES, RELIABLE_UDP_CONNECT_TIMEOUT,
    RELIABLE_UDP_OUTGOING_PACKET_CAPACITY,
};
pub use udp_session::{
    ReliableUdpOwnedPeerStream, ReliableUdpPeerStream, ReliableUdpSessionHandle,
    ReliableUdpSessionHub,
};

pub type ClientId = u32;
pub type Tick = u32;
