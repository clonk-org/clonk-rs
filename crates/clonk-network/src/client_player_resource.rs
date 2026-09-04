//! Publication of a client's local player file after the host assigns its ID.

use std::path::PathBuf;

use clonk_engine::{LegacyCString, NetworkResourceCore};
use thiserror::Error;

use crate::{
    build_host_resource_core, HostResourceCoreError, HostResourceCoreSpec, HostResourceType,
    HostedResourceFile, ResourceRegistration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPlayerResourcePublicationSpec {
    pub resource_id: i32,
    pub source_path: PathBuf,
    pub wire_name: LegacyCString,
    pub network_directory: PathBuf,
    pub group_maker: LegacyCString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPlayerResourceRequest {
    pub source_path: PathBuf,
    pub wire_name: LegacyCString,
    pub group_maker: LegacyCString,
}

/// The resource core and the path the publishing process must use for its
/// local `JoinPlayer` load. Directory player profiles use the packed
/// standalone here, matching C++'s `GetStandalone` rewrite of `szFile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedPlayerResource {
    pub core: NetworkResourceCore,
    pub local_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ClientPlayerResourcePublication {
    pub core: NetworkResourceCore,
    pub registration: ResourceRegistration,
    pub resource_file: HostedResourceFile,
}

#[derive(Debug, Error)]
pub enum ClientPlayerResourcePublicationError {
    #[error("could not publish client player resource: {0}")]
    ResourceCore(#[from] HostResourceCoreError),
    #[error("NRT_Player publication did not produce a loadable standalone")]
    MissingStandalone,
}

/// Publishes a local player at the resource ID allocated by the client's
/// `ResourceCatalog`.
///
/// C++ `nextResID` selects the free ID before `AddByFile` publishes the
/// selected file as `NRT_Player` (`src/C4Network2Res.cpp:1376-1385,1443-1471`;
/// `src/C4PlayerInfo.cpp:70-104`).
pub fn publish_client_player_resource(
    spec: ClientPlayerResourcePublicationSpec,
) -> Result<ClientPlayerResourcePublication, ClientPlayerResourcePublicationError> {
    let publication = build_host_resource_core(
        &spec.source_path,
        &spec.network_directory,
        HostResourceCoreSpec::new_with_raw_group_maker(
            HostResourceType::Player,
            spec.resource_id,
            spec.wire_name,
            spec.group_maker,
        ),
    )?;
    let path = publication
        .standalone_path
        .ok_or(ClientPlayerResourcePublicationError::MissingStandalone)?;
    let ownership = publication
        .standalone_ownership
        .ok_or(ClientPlayerResourcePublicationError::MissingStandalone)?;
    if !publication.core.loadable {
        return Err(ClientPlayerResourcePublicationError::MissingStandalone);
    }

    let core = publication.core;
    let registration = ResourceRegistration::from_core(&core, true, false);
    let resource_file = HostedResourceFile {
        core: core.clone(),
        path,
        ownership,
        binary_compatible: true,
    };
    Ok(ClientPlayerResourcePublication {
        core,
        registration,
        resource_file,
    })
}
