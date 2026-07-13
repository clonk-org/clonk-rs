mod control;
mod league;
mod legacy;
mod lobby;
mod resync;
mod session;
mod transport;

pub use control::{
    ControlCoordinator, ControlError, ControlOutcome, ControlPacket, ControlPacketBuilder,
    InsertStatus, MissingRange, ReadyBatch,
};
pub use league::LeagueFbidRegistry;
pub use legacy::{
    aggregate_ready_batch, decode_control_entry_payload, decode_control_packet,
    decode_control_payload, decode_player_info_update_payload, encode_control_entry_payload,
    encode_control_packet, encode_control_payload, encode_player_info_update_payload,
    LegacyAggregateError, LegacyControlError, LegacyControlFrame, LegacyEncodeError,
    PlayerInfoUpdateRequest,
};
pub use lobby::{Lobby, LobbyError, LobbyParticipant, LobbySettings, ParticipantKind};
pub use resync::{ControlBacklog, ResyncRequest, ResyncScheduler};
pub use session::{
    connect_client, start_host, ClientCommand, ClientConfig, ClientError, ClientEvent,
    ClientHandle, HostCommand, HostConfig, HostError, HostEvent, HostHandle, BROADCAST_CLIENT_ID,
};
pub use transport::{ControlDelivery, ControlMessage, ControlTransport, TransportError};

pub type ClientId = u32;
pub type Tick = u32;
