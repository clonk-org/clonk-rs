mod advertise;
mod admission;
mod control;
mod league;
mod legacy;
mod lobby;
mod resync;
mod search;
mod session;
mod status;
mod transport;

pub use advertise::{
    discovery_reply_for_packet, encode_reference_response, NetworkGameAdvertiser,
    NetworkGameAdvertiserConfig,
};
pub use admission::{
    AdmissionDecision, ClientAdmission, ConnectionAction, ConnectionStatus, HostAdmission,
    KnownPeerAdmission, LegacyConnection,
};
pub use lc_engine::PlayerInfoUpdateRequest;

pub use control::{
    ControlCoordinator, ControlError, ControlOutcome, ControlPacket, ControlPacketBuilder,
    InsertStatus, MissingRange, ReadyBatch,
};
pub use league::LeagueFbidRegistry;
pub use legacy::{
    aggregate_ready_batch, decode_control_entry_payload, decode_control_packet,
    decode_control_payload, decode_join_data_envelope, decode_join_game_parameters_envelope,
    decode_player_info_update_payload, encode_control_entry_payload, encode_control_packet,
    encode_control_payload, encode_join_data_envelope, encode_join_game_parameters_envelope,
    encode_player_info_update_payload, JoinDataC4Id, JoinDataEnvelope, JoinDataIdListEntry,
    JoinGameParametersEnvelope, LegacyAggregateError, LegacyControlError, LegacyControlFrame,
    LegacyEncodeError,
};
pub use lobby::{Lobby, LobbyError, LobbyParticipant, LobbySettings, ParticipantKind};
pub use resync::{ControlBacklog, ResyncRequest, ResyncScheduler};
pub use search::{
    fetch_reference_endpoint, parse_reference_response, NetworkGameReference, NetworkGameSearch,
    NetworkGameSearchConfig, ReferenceEndpoint, ReferenceFetchError, ReferenceParseError,
    ReferenceQuerySource, SearchCommand, StartupGameSearch, StartupGameSearchEvent,
    CURRENT_GAME_BUILD, CURRENT_GAME_VERSION, DEFAULT_DISCOVERY_PORT, DEFAULT_MASTER_SERVER_URL,
    DEFAULT_REFERENCE_PORT, GAME_SEARCH_INTERVAL, MAX_LAN_DISCOVERS, REFERENCE_QUERY_TIMEOUT,
};
pub use session::{
    connect_client, start_host, ClientCommand, ClientConfig, ClientError, ClientEvent,
    ClientHandle, HostCommand, HostConfig, HostError, HostEvent, HostHandle, BROADCAST_CLIENT_ID,
};
pub use status::{BarrierEffect, BarrierPhase, RemoteBarrierState, StatusBarrier};
pub use transport::{
    decode_connection_reply_payload, decode_connection_request_payload,
    encode_connection_reply_payload, encode_connection_request_payload, ConnectionReply,
    ConnectionRequest, ControlDelivery, ControlMessage, ControlTransport, NetworkStatus,
    TransportError, NETWORK_STATE_GO, NETWORK_STATE_INIT, NETWORK_STATE_LOBBY, NETWORK_STATE_NONE,
    NETWORK_STATE_PAUSE,
};

pub type ClientId = u32;
pub type Tick = u32;
