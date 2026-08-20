use clonk_engine::ClientCoreControlData;

use crate::legacy::{
    append_c_string, append_raw_i32, append_uint32, LegacyControlError, LegacyEncodeError, Reader,
};
use crate::name_validation::validate_name_no_empty;

/// A `C4ClientList` value from `C4GameParameters`, plus the local-only marker
/// which `C4Client::CompileFunc` deliberately does not serialize
/// (`src/C4Client.cpp:130-136,353-376`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JoinClientRegistrySnapshot {
    pub clients: Vec<ClientCoreControlData>,
    pub local_client_id: Option<i32>,
}

impl JoinClientRegistrySnapshot {
    pub fn new(clients: Vec<ClientCoreControlData>) -> Self {
        Self {
            clients: canonical_clients(clients),
            local_client_id: None,
        }
    }
}

/// Reads the packed client count followed by exact `C4ClientCore` fields
/// (`src/C4Client.cpp:68-76,353-376`). The decoded list is ordered through the
/// same client-ID comparison as `C4ClientList::Add` (`src/C4Client.cpp:159-176`).
pub(crate) fn decode_join_client_registry(
    reader: &mut Reader<'_>,
) -> Result<JoinClientRegistrySnapshot, LegacyControlError> {
    let count = reader.read_uint32()?;
    let mut clients = Vec::new();
    for _ in 0..count {
        clients.push(decode_client_core(reader)?);
    }
    Ok(JoinClientRegistrySnapshot::new(clients))
}

/// Writes a binary `C4ClientList`. The local marker is local-only state and is
/// therefore intentionally absent from the wire (`src/C4Client.cpp:130-136`).
pub(crate) fn encode_join_client_registry(
    buffer: &mut Vec<u8>,
    snapshot: &JoinClientRegistrySnapshot,
) -> Result<(), LegacyEncodeError> {
    let clients = canonical_clients(snapshot.clients.clone());
    let count = u32::try_from(clients.len())
        .map_err(|_| LegacyEncodeError::JoinDataClientCountTooLarge(clients.len()))?;
    append_uint32(buffer, count);
    for client in &clients {
        encode_client_core(buffer, client);
    }
    Ok(())
}

/// Models the `HandleJoinData` sequence which first rebinds the existing local
/// object to the assigned ID and then updates ID-matched objects from the
/// incoming list (`src/C4Network2.cpp:1574-1604; src/C4Client.cpp:284-290,321-350`).
///
/// `None` is the C++ "Could not find local client in join data" failure path.
pub fn reconcile_join_client_registry(
    existing: &JoinClientRegistrySnapshot,
    incoming: JoinClientRegistrySnapshot,
    assigned_local_client_id: i32,
) -> Option<JoinClientRegistrySnapshot> {
    let mut incoming = JoinClientRegistrySnapshot::new(incoming.clients);
    (existing.local_client_id.is_some()
        && assigned_local_client_id != -1
        && incoming
            .clients
            .iter()
            .any(|client| client.client_id == assigned_local_client_id))
    .then(|| {
        incoming.local_client_id = Some(assigned_local_client_id);
        incoming
    })
}

fn decode_client_core(
    reader: &mut Reader<'_>,
) -> Result<ClientCoreControlData, LegacyControlError> {
    Ok(ClientCoreControlData {
        client_id: reader.read_raw_i32()?,
        activated: reader.read_u8()? != 0,
        observer: reader.read_u8()? != 0,
        name: validate_name_no_empty(reader.read_c_string()?),
        nick: validate_name_no_empty(reader.read_c_string()?),
        lobby_ready: reader.read_u8()? != 0,
    })
}

fn encode_client_core(buffer: &mut Vec<u8>, client: &ClientCoreControlData) {
    append_raw_i32(buffer, client.client_id);
    buffer.push(u8::from(client.activated));
    buffer.push(u8::from(client.observer));
    append_c_string(buffer, &client.name);
    append_c_string(buffer, &client.nick);
    buffer.push(u8::from(client.lobby_ready));
}

fn canonical_clients(clients: Vec<ClientCoreControlData>) -> Vec<ClientCoreControlData> {
    let mut clients = clients
        .into_iter()
        .map(|mut client| {
            client.name = validate_name_no_empty(client.name);
            client.nick = validate_name_no_empty(client.nick);
            client
        })
        .collect::<Vec<_>>();
    clients.sort_by_key(|client| client.client_id);
    clients
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_engine::LegacyCString;

    fn string(value: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(value.to_vec()).unwrap()
    }

    fn core(
        client_id: i32,
        activated: bool,
        observer: bool,
        name: &[u8],
        nick: &[u8],
        lobby_ready: bool,
    ) -> ClientCoreControlData {
        ClientCoreControlData {
            client_id,
            activated,
            observer,
            name: string(name),
            nick: string(nick),
            lobby_ready,
        }
    }

    fn append_wire_core(buffer: &mut Vec<u8>, core: &ClientCoreControlData) {
        buffer.extend_from_slice(&core.client_id.to_ne_bytes());
        buffer.push(u8::from(core.activated));
        buffer.push(u8::from(core.observer));
        buffer.extend_from_slice(core.name.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(core.nick.as_bytes());
        buffer.push(0);
        buffer.push(u8::from(core.lobby_ready));
    }

    #[test]
    fn cpp_client_list_wire_vector_round_trips() {
        // C4ClientList::CompileFunc and C4ClientCore::CompileFunc
        // (src/C4Client.cpp:68-76,353-376).
        let host = core(0, true, false, b"Host", b"HostNick", true);
        let joiner = core(7, false, true, b"Alice", b"Ali", false);
        let mut cpp_wire = vec![2];
        append_wire_core(&mut cpp_wire, &host);
        append_wire_core(&mut cpp_wire, &joiner);

        let mut reader = Reader::new(&cpp_wire);
        let decoded = decode_join_client_registry(&mut reader).unwrap();

        assert_eq!(reader.remaining(), 0);
        assert_eq!(decoded.clients, vec![host, joiner]);
        assert_eq!(decoded.local_client_id, None);
        let mut encoded = Vec::new();
        encode_join_client_registry(&mut encoded, &decoded).unwrap();
        assert_eq!(encoded, cpp_wire);
    }

    #[test]
    fn decode_applies_cpp_name_no_empty_validation() {
        // C4ClientCore uses ValidatedStdStrBuf<VAL_NameNoEmpty>
        // (src/C4Client.h:40-45), normalized after compilation by
        // C4InputValidation.cpp:39-55,97-118.
        let dirty = core(3, false, false, b"{   ", b" {<i>Alice</i>{ ", false);
        let empty = core(4, false, false, b"", b"", false);
        let mut wire = vec![2];
        append_wire_core(&mut wire, &dirty);
        append_wire_core(&mut wire, &empty);

        let mut reader = Reader::new(&wire);
        let decoded = decode_join_client_registry(&mut reader).unwrap();

        assert_eq!(decoded.clients[0].name.as_bytes(), b"Unknown");
        assert_eq!(decoded.clients[0].nick.as_bytes(), b"Alice");
        assert_eq!(decoded.clients[1].name.as_bytes(), b"empty");
        assert_eq!(decoded.clients[1].nick.as_bytes(), b"empty");
    }

    #[test]
    fn decode_sorts_clients_by_id_like_cpp_add() {
        // C4ClientList::Add inserts before the first greater client ID
        // (src/C4Client.cpp:159-176).
        let high = core(19, false, false, b"High", b"High", false);
        let low = core(0, true, false, b"Host", b"Host", true);
        let middle = core(7, false, false, b"Middle", b"Middle", false);
        let mut wire = vec![3];
        append_wire_core(&mut wire, &high);
        append_wire_core(&mut wire, &low);
        append_wire_core(&mut wire, &middle);

        let mut reader = Reader::new(&wire);
        let decoded = decode_join_client_registry(&mut reader).unwrap();

        assert_eq!(
            decoded
                .clients
                .iter()
                .map(|client| client.client_id)
                .collect::<Vec<_>>(),
            vec![0, 7, 19]
        );
    }

    #[test]
    fn reconciliation_preserves_local_identity_at_assigned_id() {
        // HandleJoinData rebinds the existing local object before assigning the
        // incoming list; C4ClientList::operator= then updates that ID-matched
        // object without clearing its non-serialized local flag
        // (src/C4Network2.cpp:1574-1604; src/C4Client.cpp:284-290,321-350).
        let existing = JoinClientRegistrySnapshot {
            clients: vec![core(-1, false, false, b"Local", b"Local", false)],
            local_client_id: Some(-1),
        };
        let incoming = JoinClientRegistrySnapshot::new(vec![
            core(0, true, false, b"Host", b"Host", true),
            core(7, false, false, b"Canonical", b"Local", false),
        ]);

        let reconciled = reconcile_join_client_registry(&existing, incoming.clone(), 7).unwrap();

        assert_eq!(reconciled.local_client_id, Some(7));
        assert_eq!(reconciled.clients, incoming.clients);
        assert!(reconcile_join_client_registry(&existing, incoming, 8).is_none());

        let unknown = JoinClientRegistrySnapshot::new(vec![core(
            -1, false, false, b"Local", b"Local", false,
        )]);
        assert!(reconcile_join_client_registry(&existing, unknown, -1).is_none());
    }
}
