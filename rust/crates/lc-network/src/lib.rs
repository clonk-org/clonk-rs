mod control;
mod legacy;
mod lobby;
mod resync;
mod transport;

pub use control::{
    ControlCoordinator, ControlError, ControlOutcome, ControlPacket, ControlPacketBuilder,
    InsertStatus, MissingRange, ReadyBatch,
};
pub use legacy::{
    decode_control_packet, decode_control_payload, LegacyControlError, LegacyControlFrame,
};
pub use lobby::{Lobby, LobbyError, LobbyParticipant, LobbySettings, ParticipantKind};
pub use resync::{ControlBacklog, ResyncRequest, ResyncScheduler};
pub use transport::{ControlDelivery, ControlMessage, ControlTransport, TransportError};

pub type ClientId = u32;
pub type Tick = u32;
