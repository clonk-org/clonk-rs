use crate::{ControlPlayerInfoEntry, PlayerInfoControlData};

/// `C4PlayerInfoList`'s synchronized per-client player-info registry.
#[derive(Debug, Default)]
pub struct ControlPlayerInfoRegistry {
    clients: Vec<ClientPlayerInfos>,
}

#[derive(Debug)]
struct ClientPlayerInfos {
    client_id: i32,
    players: Vec<ControlPlayerInfoEntry>,
}

impl ControlPlayerInfoRegistry {
    pub fn apply(&mut self, info: PlayerInfoControlData) {
        let PlayerInfoControlData {
            client_id, players, ..
        } = info;
        if let Some(existing) = self
            .clients
            .iter_mut()
            .find(|client| client.client_id == client_id)
        {
            existing.players = players;
        } else {
            self.clients.push(ClientPlayerInfos { client_id, players });
        }
    }

    pub fn get(&self, info_id: i32) -> Option<&ControlPlayerInfoEntry> {
        self.clients
            .iter()
            .flat_map(|client| &client.players)
            .find(|player| player.id == info_id)
    }

    pub fn player_count(&self) -> usize {
        self.clients.iter().map(|client| client.players.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: i32) -> ControlPlayerInfoEntry {
        ControlPlayerInfoEntry {
            id,
            ..Default::default()
        }
    }

    #[test]
    fn non_add_packet_replaces_the_clients_player_list() {
        // C4PlayerInfoList::AddInfo replaces an existing client's entire
        // C4ClientPlayerInfos unless CIF_AddPlayers is set
        // (src/C4PlayerInfo.cpp:834-880).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(7)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(8)],
            ..Default::default()
        });

        assert!(registry.get(7).is_none());
        assert_eq!(registry.get(8).map(|entry| entry.id), Some(8));
        assert_eq!(registry.player_count(), 1);
    }
}
