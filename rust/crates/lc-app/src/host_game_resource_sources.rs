//! Resolves the physical game resources published by a new network host.

use std::fs;
use std::path::{Path, PathBuf};

use lc_engine::{InitialNetworkScenarioMetadata, LegacyCString};
use lc_network::HostInitialResourceSource;
use lc_resources::{Group, GroupError};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGameResourceSources {
    pub definitions: Vec<HostInitialResourceSource>,
    pub system: HostInitialResourceSource,
    pub materials: Vec<HostInitialResourceSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGameResourceSourceKind {
    Definition,
    System,
    Material,
}

#[derive(Debug, Error)]
pub enum HostGameResourceSourceError {
    #[error("at least one explicit install root is required")]
    MissingInstallRoots,
    #[error("install root does not exist: {path}")]
    InstallRootMissing { path: PathBuf },
    #[error("install root is not a directory: {path}")]
    InstallRootNotDirectory { path: PathBuf },
    #[error("install root metadata could not be read at {}: {source}", path.display())]
    InstallRootIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("scenario is not contained by an explicit install root: {path}")]
    ScenarioOutsideInstallRoots { path: PathBuf },
    #[error("scenario group could not be opened at {}: {source}", path.display())]
    ScenarioGroup {
        path: PathBuf,
        #[source]
        source: GroupError,
    },
    #[error("{kind:?} resource `{wire_name}` was not found in the explicit install roots")]
    ResourceMissing {
        kind: HostGameResourceSourceKind,
        wire_name: String,
    },
    #[error("{kind:?} resource group could not be opened at {}: {source}", path.display())]
    ResourceGroup {
        kind: HostGameResourceSourceKind,
        path: PathBuf,
        #[source]
        source: GroupError,
    },
    #[error("resource wire path is not UTF-8: {path}")]
    NonUtf8WirePath { path: PathBuf },
    #[error("resource wire path contains an interior NUL: {path}")]
    InvalidWirePath { path: PathBuf },
    #[error("packed ancestor material cannot yet be represented as a physical source: {path}")]
    PackedAncestorMaterialUnsupported { path: PathBuf },
}

/// Resolves the C4GameRes sources without consulting process-global app state.
pub fn resolve_host_game_resource_sources(
    scenario_path: impl AsRef<Path>,
    install_roots: &[PathBuf],
    metadata: &InitialNetworkScenarioMetadata,
) -> Result<HostGameResourceSources, HostGameResourceSourceError> {
    let scenario_path = scenario_path.as_ref();
    validate_install_roots(install_roots)?;
    Group::open(scenario_path).map_err(|source| HostGameResourceSourceError::ScenarioGroup {
        path: scenario_path.to_path_buf(),
        source,
    })?;
    let (scenario_root, scenario_relative) = scenario_install_root(scenario_path, install_roots)?;

    let mut definitions = metadata
        .definition_modules
        .iter()
        .map(|module| {
            resolve_installed_group(
                HostGameResourceSourceKind::Definition,
                module,
                install_roots,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    definitions.extend(folder_local_definitions(scenario_root, scenario_relative)?);

    let system = resolve_installed_group(
        HostGameResourceSourceKind::System,
        "System.c4g",
        install_roots,
    )?;
    let mut materials = folder_materials(scenario_root, scenario_relative)?;
    materials.push(resolve_installed_group(
        HostGameResourceSourceKind::Material,
        "Material.c4g",
        install_roots,
    )?);

    Ok(HostGameResourceSources {
        definitions,
        system,
        materials,
    })
}

fn validate_install_roots(install_roots: &[PathBuf]) -> Result<(), HostGameResourceSourceError> {
    if install_roots.is_empty() {
        return Err(HostGameResourceSourceError::MissingInstallRoots);
    }
    install_roots.iter().try_for_each(|root| {
        let metadata = fs::metadata(root).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                HostGameResourceSourceError::InstallRootMissing { path: root.clone() }
            } else {
                HostGameResourceSourceError::InstallRootIo {
                    path: root.clone(),
                    source: error,
                }
            }
        })?;
        if metadata.is_dir() {
            Ok(())
        } else {
            Err(HostGameResourceSourceError::InstallRootNotDirectory { path: root.clone() })
        }
    })
}

fn scenario_install_root<'a>(
    scenario_path: &'a Path,
    install_roots: &'a [PathBuf],
) -> Result<(&'a Path, &'a Path), HostGameResourceSourceError> {
    install_roots
        .iter()
        .find_map(|root| {
            scenario_path
                .strip_prefix(root)
                .ok()
                .map(|relative| (root.as_path(), relative))
        })
        .ok_or_else(
            || HostGameResourceSourceError::ScenarioOutsideInstallRoots {
                path: scenario_path.to_path_buf(),
            },
        )
}

fn resolve_installed_group(
    kind: HostGameResourceSourceKind,
    logical_name: &str,
    install_roots: &[PathBuf],
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    let normalized_name = logical_name.replace('\\', "/");
    let logical_path = Path::new(&normalized_name);
    let path = install_roots
        .iter()
        .map(|root| root.join(logical_path))
        .find(|candidate| candidate.exists())
        .ok_or_else(|| HostGameResourceSourceError::ResourceMissing {
            kind,
            wire_name: normalized_name.clone(),
        })?;
    Group::open(&path).map_err(|source| HostGameResourceSourceError::ResourceGroup {
        kind,
        path: path.clone(),
        source,
    })?;
    source(path, logical_path)
}

fn folder_local_definitions(
    scenario_root: &Path,
    scenario_relative: &Path,
) -> Result<Vec<HostInitialResourceSource>, HostGameResourceSourceError> {
    let mut relative = PathBuf::new();
    let mut definitions = Vec::new();
    for component in scenario_relative
        .parent()
        .into_iter()
        .flat_map(Path::components)
    {
        relative.push(component.as_os_str());
        if !has_extension(&relative, "c4f") {
            continue;
        }
        let path = scenario_root.join(&relative);
        let group =
            Group::open(&path).map_err(|source| HostGameResourceSourceError::ResourceGroup {
                kind: HostGameResourceSourceKind::Definition,
                path: path.clone(),
                source,
            })?;
        let contains_definitions = group
            .entries()
            .map_err(|source| HostGameResourceSourceError::ResourceGroup {
                kind: HostGameResourceSourceKind::Definition,
                path: path.clone(),
                source,
            })?
            .iter()
            .any(|entry| matches_definition_entry(&entry.relative_path));
        if contains_definitions {
            definitions.push(source(path, &relative)?);
        }
    }
    Ok(definitions)
}

fn folder_materials(
    scenario_root: &Path,
    scenario_relative: &Path,
) -> Result<Vec<HostInitialResourceSource>, HostGameResourceSourceError> {
    let mut relative = PathBuf::new();
    let mut folder_prefixes = Vec::new();
    for component in scenario_relative
        .parent()
        .into_iter()
        .flat_map(Path::components)
    {
        relative.push(component.as_os_str());
        folder_prefixes.push(relative.clone());
    }

    let mut materials = Vec::new();
    for relative in folder_prefixes
        .into_iter()
        .rev()
        .take_while(|relative| has_extension(relative, "c4f"))
    {
        let folder_path = scenario_root.join(&relative);
        let group = Group::open(&folder_path).map_err(|source| {
            HostGameResourceSourceError::ResourceGroup {
                kind: HostGameResourceSourceKind::Material,
                path: folder_path.clone(),
                source,
            }
        })?;
        let material_entry = group
            .entries()
            .map_err(|source| HostGameResourceSourceError::ResourceGroup {
                kind: HostGameResourceSourceKind::Material,
                path: folder_path.clone(),
                source,
            })?
            .into_iter()
            .find(|entry| matches_ascii_name(&entry.relative_path, b"Material.c4g"));
        let Some(material_entry) = material_entry else {
            continue;
        };
        let material_path = folder_path.join(&material_entry.relative_path);
        if !group.is_directory() {
            return Err(
                HostGameResourceSourceError::PackedAncestorMaterialUnsupported {
                    path: material_path,
                },
            );
        }
        Group::open(&material_path).map_err(|source| {
            HostGameResourceSourceError::ResourceGroup {
                kind: HostGameResourceSourceKind::Material,
                path: material_path.clone(),
                source,
            }
        })?;
        let wire_path = relative.join("Material.c4g");
        materials.push(source(material_path, &wire_path)?);
    }
    Ok(materials)
}

fn source(
    path: PathBuf,
    wire_path: &Path,
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    let wire_name = wire_path
        .to_str()
        .ok_or_else(|| HostGameResourceSourceError::NonUtf8WirePath {
            path: wire_path.to_path_buf(),
        })?
        .replace('\\', "/");
    let wire_name = LegacyCString::from_bytes(wire_name.into_bytes()).ok_or_else(|| {
        HostGameResourceSourceError::InvalidWirePath {
            path: wire_path.to_path_buf(),
        }
    })?;
    Ok(HostInitialResourceSource { path, wire_name })
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|value| {
        value
            .as_encoded_bytes()
            .eq_ignore_ascii_case(extension.as_bytes())
    })
}

fn matches_definition_entry(path: &Path) -> bool {
    // C4GroupSet::CheckGroupContents calls FindEntry(C4CFN_DefFiles), where
    // C4CFN_DefFiles is the case-insensitive direct-entry wildcard `*.c4d`
    // (src/C4GroupSet.cpp:112-132; src/C4Components.h:125;
    // src/StdFile.cpp:337-367). Group::entries is direct-only for both packed
    // and directory groups.
    path.file_name().is_some_and(|name| {
        let bytes = name.as_encoded_bytes();
        bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".c4d")
    })
}

fn matches_ascii_name(path: &Path, expected: &[u8]) -> bool {
    path.file_name()
        .is_some_and(|name| name.as_encoded_bytes().eq_ignore_ascii_case(expected))
}
