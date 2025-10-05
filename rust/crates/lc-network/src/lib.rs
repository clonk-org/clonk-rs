mod control;
mod lobby;

pub use control::{
    ControlCoordinator, ControlError, ControlOutcome, ControlPacket, ControlPacketBuilder,
    InsertStatus, MissingRange, ReadyBatch,
};
pub use lobby::{Lobby, LobbyError, LobbyParticipant, LobbySettings, ParticipantKind};

pub type ClientId = u32;
pub type Tick = u32;
