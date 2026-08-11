//! Resolves the physical game resources published by a new network host.

use std::fs;
use std::path::{Path, PathBuf};

use clonk_engine::LegacyCString;
use clonk_network::HostInitialResourceSource;
use clonk_resources::{compress_c4group_image, Group, GroupError, MutableGroupError};
use thiserror::Error;

pub(crate) use crate::resource_path_identity::{
    executable_relative_group_name, open_group_path, opened_physical_group_name,
};
use crate::resource_path_identity::{
    executable_relative_wire_name, opened_group_name, path_wire_bytes,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGameResourceSources {
    pub definitions: Vec<HostInitialResourceSource>,
    pub system: HostInitialResourceSource,
    pub materials: Vec<HostInitialResourceSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGameResourceSourceKind {
    Scenario,
    Definition,
    System,
    Material,
    Player,
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
    #[error("resource wire path contains an interior NUL: {path}")]
    InvalidWirePath { path: PathBuf },
    #[error(
        "effective definition vector has {actual} resources, fewer than the {expected} selected external entries"
    )]
    DefinitionResourceCountMismatch { actual: usize, expected: usize },
    #[error("could not recover the selected folder-local spelling for {}", path.display())]
    FolderDefinitionSpellingMissing { path: PathBuf },
    #[error("{kind:?} resource group could not be snapshotted at {}: {source}", path.display())]
    ResourceGroupSnapshot {
        kind: HostGameResourceSourceKind,
        path: PathBuf,
        #[source]
        source: MutableGroupError,
    },
}

/// Freezes the staged physical definition vector together with the filename
/// spelling C++ publishes. Explicit entries retain their selected module
/// spelling; folder-local entries use their executable-relative physical path.
pub fn freeze_host_definition_resource_sources(
    definition_resource_paths: &[PathBuf],
    scenario_path: &Path,
    effective_modules: &[String],
    definition_root_applied: bool,
    definition_executable_root: &Path,
    definition_path: &str,
) -> Result<Vec<HostInitialResourceSource>, HostGameResourceSourceError> {
    let block_count = 1 + usize::from(definition_root_applied);
    let external_count = effective_modules.len().saturating_mul(block_count);
    if definition_resource_paths.len() < external_count {
        return Err(
            HostGameResourceSourceError::DefinitionResourceCountMismatch {
                actual: definition_resource_paths.len(),
                expected: external_count,
            },
        );
    }

    let mut lookup_names = Vec::with_capacity(definition_resource_paths.len());
    if definition_root_applied {
        let definition_path = clonk_script::c4_string_bytes(definition_path);
        lookup_names.extend(effective_modules.iter().map(|module| {
            let mut lookup_name = definition_path.clone();
            lookup_name.extend(clonk_script::c4_string_bytes(module));
            lookup_name
        }));
    }
    lookup_names.extend(
        effective_modules
            .iter()
            .map(|module| clonk_script::c4_string_bytes(module)),
    );
    lookup_names.extend(folder_local_lookup_names(
        scenario_path,
        &definition_resource_paths[external_count..],
        definition_executable_root,
    )?);

    definition_resource_paths
        .iter()
        .zip(lookup_names)
        .map(|(path, lookup_name)| {
            let opened_name = opened_group_name(path, &lookup_name, definition_executable_root);
            let wire_name =
                executable_relative_wire_name(lookup_name.clone(), definition_executable_root);
            source_from_names(path.clone(), lookup_name, opened_name, wire_name, path)
        })
        .collect()
}

fn folder_local_lookup_names(
    scenario_path: &Path,
    folder_resources: &[PathBuf],
    executable_root: &Path,
) -> Result<Vec<Vec<u8>>, HostGameResourceSourceError> {
    let mut candidates = scenario_path
        .ancestors()
        .skip(1)
        .filter(|path| has_extension(path, "c4f"))
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    candidates.reverse();
    let mut candidate_start = 0;

    folder_resources
        .iter()
        .map(|resource| {
            let matched =
                candidates[candidate_start..]
                    .iter()
                    .enumerate()
                    .find_map(|(offset, candidate)| {
                        open_group_path(candidate)
                            .ok()
                            .filter(|group| group.root() == resource)
                            .map(|_| (candidate_start + offset, candidate))
                    });
            if let Some((index, candidate)) = matched {
                candidate_start = index + 1;
                Ok(executable_relative_wire_name(
                    path_wire_bytes(candidate),
                    executable_root,
                ))
            } else {
                Err(
                    HostGameResourceSourceError::FolderDefinitionSpellingMissing {
                        path: resource.clone(),
                    },
                )
            }
        })
        .collect()
}

/// Resolves the C4GameRes sources without consulting process-global app state.
pub fn resolve_host_game_resource_sources(
    scenario_path: impl AsRef<Path>,
    install_roots: &[PathBuf],
    definition_resources: &[HostInitialResourceSource],
    executable_root: &Path,
) -> Result<HostGameResourceSources, HostGameResourceSourceError> {
    let scenario_path = scenario_path.as_ref();
    validate_install_roots(install_roots)?;
    open_group_path(scenario_path).map_err(|source| {
        HostGameResourceSourceError::ScenarioGroup {
            path: scenario_path.to_path_buf(),
            source,
        }
    })?;
    let (scenario_root, scenario_relative) = scenario_install_root(scenario_path, install_roots)?;

    let definitions = definition_resources
        .iter()
        .map(resolve_effective_definition)
        .collect::<Result<Vec<_>, _>>()?;

    let system = resolve_installed_group(
        HostGameResourceSourceKind::System,
        "System.c4g",
        install_roots,
        executable_root,
    )?;
    let mut materials = folder_materials(scenario_root, scenario_relative, executable_root)?;
    materials.push(resolve_installed_group(
        HostGameResourceSourceKind::Material,
        "Material.c4g",
        install_roots,
        executable_root,
    )?);

    Ok(HostGameResourceSources {
        definitions,
        system,
        materials,
    })
}

fn resolve_effective_definition(
    resource: &HostInitialResourceSource,
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    validate_host_group_resource_source(HostGameResourceSourceKind::Definition, resource.clone())
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

/// The first install root holding `logical_name`, opened as a group.
fn open_installed_group(
    kind: HostGameResourceSourceKind,
    normalized_name: &str,
    install_roots: &[PathBuf],
) -> Result<(PathBuf, Group), HostGameResourceSourceError> {
    let path = install_roots
        .iter()
        .map(|root| root.join(normalized_name))
        .find(|candidate| candidate.exists())
        .ok_or_else(|| HostGameResourceSourceError::ResourceMissing {
            kind,
            wire_name: normalized_name.to_owned(),
        })?;
    let group =
        open_group_path(&path).map_err(|source| group_source_error(kind, path.clone(), source))?;
    Ok((path, group))
}

fn resolve_installed_group(
    kind: HostGameResourceSourceKind,
    logical_name: &str,
    install_roots: &[PathBuf],
    executable_root: &Path,
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    let normalized_name = logical_name.replace('\\', "/");
    let logical_path = Path::new(&normalized_name);
    let (path, group) = open_installed_group(kind, &normalized_name, install_roots)?;
    let lookup_name = clonk_script::c4_string_bytes(&normalized_name);
    let opened_name = opened_group_name(group.root(), &lookup_name, executable_root);
    let resource = source_from_names(
        path,
        lookup_name.clone(),
        opened_name,
        lookup_name,
        logical_path,
    )?;
    validate_open_group_resource_source(kind, resource, group)
}

/// One registered parent folder's `Material.c4g`, kept beside the folder group
/// whose spelling `C4GameParameters::Load` passes to `AddByFile`.
struct FolderMaterialGroup {
    folder: Group,
    material: Group,
    material_path: PathBuf,
}

/// Every registered parent folder's `Material.c4g`, innermost folder first, in
/// the order `C4GameParameters::Load` publishes them as `NRT_Material`
/// (C4GameParameters.cpp:214-220). A failure carries the path it happened at
/// so both callers can name it in their own error type.
fn folder_material_groups(
    scenario_root: &Path,
    scenario_relative: &Path,
) -> Result<Vec<FolderMaterialGroup>, (PathBuf, GroupError)> {
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
        let folder =
            open_group_path(&folder_path).map_err(|source| (folder_path.clone(), source))?;
        let material_entry = folder
            .entries()
            .map_err(|source| (folder_path.clone(), source))?
            .into_iter()
            .find(|entry| matches_ascii_name(&entry.relative_path, b"Material.c4g"));
        let Some(material_entry) = material_entry else {
            continue;
        };
        let material_path = folder_path.join(&material_entry.relative_path);
        let material = folder
            .open_child_entry_exact(&material_entry)
            .map_err(|source| (material_path.clone(), source))?;
        materials.push(FolderMaterialGroup {
            folder,
            material,
            material_path,
        });
    }
    Ok(materials)
}

fn folder_material_group_error(
    (path, source): (PathBuf, GroupError),
) -> HostGameResourceSourceError {
    HostGameResourceSourceError::ResourceGroup {
        kind: HostGameResourceSourceKind::Material,
        path,
        source,
    }
}

fn folder_materials(
    scenario_root: &Path,
    scenario_relative: &Path,
    executable_root: &Path,
) -> Result<Vec<HostInitialResourceSource>, HostGameResourceSourceError> {
    folder_material_groups(scenario_root, scenario_relative)
        .map_err(folder_material_group_error)?
        .into_iter()
        .map(|folder| {
            // C4GameParameters passes the already-opened parent group's full
            // name plus the literal child name to AddByFile. SetByFile freezes
            // AtExeRelativePath of that lookup spelling before C4Group corrects
            // the retained opened filename used only for later reuse searches.
            let lookup_path = folder.folder.root().join("Material.c4g");
            let lookup_name = path_wire_bytes(&lookup_path);
            let opened_name =
                opened_group_name(folder.material.root(), &lookup_name, executable_root);
            let wire_name = executable_relative_wire_name(lookup_name.clone(), executable_root);
            let resource = source_from_names(
                folder.material_path,
                lookup_name,
                opened_name,
                wire_name,
                &lookup_path,
            )?;
            validate_open_group_resource_source(
                HostGameResourceSourceKind::Material,
                resource,
                folder.material,
            )
        })
        .collect()
}

/// Validates an initial group source and freezes a standalone packed image
/// when its stable source path exists only virtually inside a packed parent.
pub(crate) fn validate_host_group_resource_source(
    kind: HostGameResourceSourceKind,
    resource: HostInitialResourceSource,
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    let group = match resource.virtual_group_bytes.as_deref() {
        Some(bytes) => Group::from_memory(resource.path.clone(), bytes.to_vec()),
        None => open_group_path(&resource.path),
    }
    .map_err(|source| group_source_error(kind, resource.path.clone(), source))?;
    validate_open_group_resource_source(kind, resource, group)
}

fn validate_open_group_resource_source(
    kind: HostGameResourceSourceKind,
    mut resource: HostInitialResourceSource,
    group: Group,
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    resource.path = group.root().to_path_buf();
    if resource.path.exists() || resource.virtual_group_bytes.is_some() {
        return Ok(resource);
    }

    let raw_image = group
        .raw_image()
        .map_err(|source| group_source_error(kind, resource.path.clone(), source))?;
    let packed = compress_c4group_image(&raw_image).map_err(|source| {
        HostGameResourceSourceError::ResourceGroupSnapshot {
            kind,
            path: resource.path.clone(),
            source,
        }
    })?;
    resource.virtual_group_bytes = Some(packed);
    Ok(resource)
}

fn group_source_error(
    kind: HostGameResourceSourceKind,
    path: PathBuf,
    source: GroupError,
) -> HostGameResourceSourceError {
    if kind == HostGameResourceSourceKind::Scenario {
        HostGameResourceSourceError::ScenarioGroup { path, source }
    } else {
        HostGameResourceSourceError::ResourceGroup { kind, path, source }
    }
}

fn source_from_names(
    path: PathBuf,
    lookup_name: Vec<u8>,
    opened_name: Vec<u8>,
    wire_name: Vec<u8>,
    wire_path: &Path,
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    let lookup_name = legacy_wire_name(lookup_name, wire_path)?;
    let opened_name = legacy_wire_name(opened_name, wire_path)?;
    let wire_name = legacy_wire_name(wire_name, wire_path)?;
    Ok(HostInitialResourceSource {
        path,
        lookup_name,
        opened_name,
        wire_name,
        virtual_group_bytes: None,
    })
}

fn legacy_wire_name(
    bytes: Vec<u8>,
    path: &Path,
) -> Result<LegacyCString, HostGameResourceSourceError> {
    LegacyCString::from_bytes(bytes).ok_or_else(|| HostGameResourceSourceError::InvalidWirePath {
        path: path.to_path_buf(),
    })
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|value| {
        path_wire_bytes(Path::new(value)).eq_ignore_ascii_case(extension.as_bytes())
    })
}

fn matches_ascii_name(path: &Path, expected: &[u8]) -> bool {
    path.file_name()
        .is_some_and(|name| path_wire_bytes(Path::new(name)).eq_ignore_ascii_case(expected))
}
