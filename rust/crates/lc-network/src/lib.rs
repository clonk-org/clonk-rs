mod control;
mod league;
mod legacy;
mod lobby;
mod resync;
mod session;
mod status;
mod transport;

pub use lc_engine::PlayerInfoUpdateRequest;

pub use control::{
    ControlCoordinator, ControlError, ControlOutcome, ControlPacket, ControlPacketBuilder,
    InsertStatus, MissingRange, ReadyBatch,
};
pub use league::LeagueFbidRegistry;
pub use legacy::{
    aggregate_ready_batch, decode_control_entry_payload, decode_control_packet,
    decode_control_payload, decode_player_info_update_payload, encode_control_entry_payload,
    encode_control_packet, encode_control_payload, encode_player_info_update_payload,
    LegacyAggregateError, LegacyControlError, LegacyControlFrame, LegacyControlSet,
    LegacyEncodeError,
};
pub use lobby::{Lobby, LobbyError, LobbyParticipant, LobbySettings, ParticipantKind};
pub use resync::{ControlBacklog, ResyncRequest, ResyncScheduler};
pub use session::{
    connect_client, start_host, ClientCommand, ClientConfig, ClientError, ClientEvent,
    ClientHandle, HostCommand, HostConfig, HostError, HostEvent, HostHandle, BROADCAST_CLIENT_ID,
};
pub use status::{BarrierEffect, BarrierPhase, RemoteBarrierState, StatusBarrier};
pub use transport::{
    ControlDelivery, ControlMessage, ControlTransport, LobbyCountdown, NetworkStatus, ReadyCheck,
    ReadyCheckData, TransportError, NETWORK_STATE_GO, NETWORK_STATE_INIT, NETWORK_STATE_LOBBY,
    NETWORK_STATE_NONE, NETWORK_STATE_PAUSE,
};

pub type ClientId = u32;
pub type Tick = u32;
