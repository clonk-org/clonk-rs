use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ClientId;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LobbyError {
    #[error("client {0} is already part of the lobby")]
    DuplicateClient(ClientId),
    #[error("lobby is full")]
    LobbyFull,
    #[error("client {0} not found")]
    UnknownClient(ClientId),
    #[error("cannot promote non-member {0} to host")]
    InvalidHost(ClientId),
    #[error("lobby has no participants to elect a host")]
    NoParticipants,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbySettings {
    pub max_players: usize,
    pub scenario: Option<String>,
    pub script_hash: Option<String>,
}

impl LobbySettings {
    pub fn new(max_players: usize) -> Self {
        Self {
            max_players,
            scenario: None,
            script_hash: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantKind {
    Player,
    Observer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyParticipant {
    pub name: String,
    pub ready: bool,
    pub kind: ParticipantKind,
}

impl LobbyParticipant {
    fn new(name: impl Into<String>, kind: ParticipantKind) -> Self {
        Self {
            name: name.into(),
            ready: matches!(kind, ParticipantKind::Observer),
            kind,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Lobby {
    host: Option<ClientId>,
    participants: BTreeMap<ClientId, LobbyParticipant>,
    settings: LobbySettings,
}

impl Lobby {
    pub fn new(max_players: usize) -> Self {
        Self {
            host: None,
            participants: BTreeMap::new(),
            settings: LobbySettings::new(max_players),
        }
    }

    pub fn settings(&self) -> &LobbySettings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut LobbySettings {
        &mut self.settings
    }

    pub fn host(&self) -> Option<ClientId> {
        self.host
    }

    pub fn participants(&self) -> impl Iterator<Item = (ClientId, &LobbyParticipant)> {
        self.participants
            .iter()
            .map(|(&id, participant)| (id, participant))
    }

    pub fn join_player(
        &mut self,
        client_id: ClientId,
        name: impl Into<String>,
    ) -> Result<(), LobbyError> {
        self.insert_participant(client_id, name, ParticipantKind::Player)
    }

    pub fn join_observer(
        &mut self,
        client_id: ClientId,
        name: impl Into<String>,
    ) -> Result<(), LobbyError> {
        self.insert_participant(client_id, name, ParticipantKind::Observer)
    }

    fn insert_participant(
        &mut self,
        client_id: ClientId,
        name: impl Into<String>,
        kind: ParticipantKind,
    ) -> Result<(), LobbyError> {
        if self.participants.contains_key(&client_id) {
            return Err(LobbyError::DuplicateClient(client_id));
        }
        if matches!(kind, ParticipantKind::Player)
            && self.player_count() >= self.settings.max_players
        {
            return Err(LobbyError::LobbyFull);
        }

        self.participants
            .insert(client_id, LobbyParticipant::new(name, kind));

        if self.host.is_none() {
            self.host = Some(client_id);
        }
        Ok(())
    }

    pub fn set_ready(&mut self, client_id: ClientId, ready: bool) -> Result<(), LobbyError> {
        let participant = self
            .participants
            .get_mut(&client_id)
            .ok_or(LobbyError::UnknownClient(client_id))?;
        participant.ready = ready || matches!(participant.kind, ParticipantKind::Observer);
        Ok(())
    }

    pub fn set_participant_kind(
        &mut self,
        client_id: ClientId,
        kind: ParticipantKind,
    ) -> Result<(), LobbyError> {
        let requires_slot = {
            let participant = self
                .participants
                .get(&client_id)
                .ok_or(LobbyError::UnknownClient(client_id))?;
            matches!(kind, ParticipantKind::Player)
                && matches!(participant.kind, ParticipantKind::Observer)
        };

        if requires_slot && self.player_count() >= self.settings.max_players {
            return Err(LobbyError::LobbyFull);
        }

        let participant = self
            .participants
            .get_mut(&client_id)
            .expect("participant known to exist");
        participant.kind = kind;
        if matches!(kind, ParticipantKind::Observer) {
            participant.ready = true;
        }
        Ok(())
    }

    pub fn set_name(
        &mut self,
        client_id: ClientId,
        name: impl Into<String>,
    ) -> Result<(), LobbyError> {
        let participant = self
            .participants
            .get_mut(&client_id)
            .ok_or(LobbyError::UnknownClient(client_id))?;
        participant.name = name.into();
        Ok(())
    }

    pub fn remove(&mut self, client_id: ClientId) -> Result<Option<ClientId>, LobbyError> {
        if self.participants.remove(&client_id).is_none() {
            return Err(LobbyError::UnknownClient(client_id));
        }
        if self.host == Some(client_id) {
            self.host = self.select_new_host();
        }
        Ok(self.host)
    }

    pub fn promote_host(&mut self, client_id: ClientId) -> Result<(), LobbyError> {
        if !self.participants.contains_key(&client_id) {
            return Err(LobbyError::InvalidHost(client_id));
        }
        self.host = Some(client_id);
        Ok(())
    }

    pub fn everybody_ready(&self) -> bool {
        self.participants
            .values()
            .filter(|p| matches!(p.kind, ParticipantKind::Player))
            .all(|p| p.ready)
    }

    pub fn player_count(&self) -> usize {
        self.participants
            .values()
            .filter(|p| matches!(p.kind, ParticipantKind::Player))
            .count()
    }

    fn select_new_host(&self) -> Option<ClientId> {
        let mut players = self
            .participants
            .iter()
            .filter(|(_, p)| matches!(p.kind, ParticipantKind::Player))
            .map(|(&id, _)| id);
        if let Some(player) = players.next() {
            return Some(player);
        }
        self.participants.keys().next().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_joined_becomes_host() {
        let mut lobby = Lobby::new(2);
        lobby.join_player(1, "Alpha").unwrap();
        lobby.join_player(2, "Beta").unwrap();
        assert_eq!(lobby.host(), Some(1));
        lobby.promote_host(2).unwrap();
        assert_eq!(lobby.host(), Some(2));
    }

    #[test]
    fn host_switches_when_current_leaves() {
        let mut lobby = Lobby::new(2);
        lobby.join_player(1, "Alpha").unwrap();
        lobby.join_player(3, "Gamma").unwrap();
        lobby.join_observer(8, "Obs").unwrap();

        lobby.remove(1).unwrap();
        assert_eq!(lobby.host(), Some(3));
        lobby.remove(3).unwrap();
        assert_eq!(lobby.host(), Some(8));
    }

    #[test]
    fn observers_do_not_consume_player_slots() {
        let mut lobby = Lobby::new(1);
        lobby.join_player(1, "Alpha").unwrap();
        lobby.join_observer(2, "Bravo").unwrap();
        assert!(lobby.join_player(3, "Charlie").is_err());
        assert!(lobby.join_observer(4, "Delta").is_ok());
    }

    #[test]
    fn everybody_ready_ignores_observers() {
        let mut lobby = Lobby::new(2);
        lobby.join_player(1, "Alpha").unwrap();
        lobby.join_player(2, "Beta").unwrap();
        lobby.join_observer(3, "Gamma").unwrap();

        assert!(!lobby.everybody_ready());
        lobby.set_ready(1, true).unwrap();
        lobby.set_ready(2, true).unwrap();
        assert!(lobby.everybody_ready());
    }

    #[test]
    fn promoting_unknown_client_is_error() {
        let mut lobby = Lobby::new(2);
        lobby.join_player(1, "Alpha").unwrap();
        assert_eq!(lobby.promote_host(99), Err(LobbyError::InvalidHost(99)));
    }
}
