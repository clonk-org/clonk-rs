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

/// The ordered `NRT_Material` groups a host publishes for `scenario_path`:
/// every physical- and Origin-path parent folder's `Material.c4g` in GroupSet
/// priority order, then the installed one (C4GameParameters.cpp:214-222).
///
/// Inputs that carry no chain — a scenario outside every install root, an
/// installation without `Material.c4g` — resolve to the shorter chain they
/// produce rather than to an error. `resolve_host_game_resource_sources`
/// remains the authority that rejects them, so reading this chain never
/// changes which failure a host bootstrap reports first.
pub fn resolve_host_material_groups(
    scenario_path: &Path,
    scenario_origin: Option<&str>,
    install_roots: &[PathBuf],
    executable_root: &Path,
) -> Result<Vec<Group>, GroupError> {
    let mut groups = ordered_folder_material_groups(
        scenario_path,
        scenario_origin,
        install_roots,
        executable_root,
    )
    .map_err(|(_, source)| source)?
    .unwrap_or_default()
    .into_iter()
    .map(|folder| folder.material)
    .collect::<Vec<_>>();
    let folder_roots = groups
        .iter()
        .map(|group| group.root().to_path_buf())
        .collect::<Vec<_>>();
    match open_installed_group(
        HostGameResourceSourceKind::Material,
        "Material.c4g",
        install_roots,
        &folder_roots,
    ) {
        Ok((_, installed)) => groups.push(installed),
        Err(HostGameResourceSourceError::ResourceGroup { source, .. }) => return Err(source),
        Err(_) => {}
    }
    Ok(groups)
}

/// Whether a candidate group was already contributed by the folder chain.
fn contributed(already_contributed: &[PathBuf], candidate: &Path) -> bool {
    already_contributed
        .iter()
        .any(|root| root == candidate || same_existing_path(root, candidate))
}

/// Whether two paths name the same existing directory.
///
/// The folder chain and the install roots reach the same group by different
/// spellings often enough — a symlinked data directory, a relative root — that
/// comparing the literal paths alone would let the overlay through.
fn same_existing_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Resolves the C4GameRes sources without consulting process-global app state.
pub fn resolve_host_game_resource_sources(
    scenario_path: impl AsRef<Path>,
    scenario_origin: Option<&str>,
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
    scenario_install_root(scenario_path, install_roots)?;

    let definitions = definition_resources
        .iter()
        .map(resolve_effective_definition)
        .collect::<Result<Vec<_>, _>>()?;

    let system = resolve_installed_group(
        HostGameResourceSourceKind::System,
        "System.c4g",
        install_roots,
        executable_root,
        &[],
    )?;
    let registered_materials = ordered_folder_material_groups(
        scenario_path,
        scenario_origin,
        install_roots,
        executable_root,
    )
    .map_err(folder_material_group_error)?
    .unwrap_or_default();
    let published_material_roots = registered_materials
        .iter()
        .map(|folder| folder.material.root().to_path_buf())
        .collect::<Vec<_>>();
    let mut materials = folder_materials(registered_materials, executable_root)?;
    materials.push(resolve_installed_group(
        HostGameResourceSourceKind::Material,
        "Material.c4g",
        install_roots,
        executable_root,
        &published_material_roots,
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
    already_contributed: &[PathBuf],
) -> Result<(PathBuf, Group), HostGameResourceSourceError> {
    let path = install_roots
        .iter()
        .map(|root| root.join(normalized_name))
        .find(|candidate| candidate.exists() && !contributed(already_contributed, candidate))
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
    already_contributed: &[PathBuf],
) -> Result<HostInitialResourceSource, HostGameResourceSourceError> {
    let normalized_name = logical_name.replace('\\', "/");
    let logical_path = Path::new(&normalized_name);
    let (path, group) =
        open_installed_group(kind, &normalized_name, install_roots, already_contributed)?;
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
    folder_priority: usize,
    folder: Group,
    material: Group,
    material_path: PathBuf,
}

fn ordered_folder_material_groups(
    scenario_path: &Path,
    scenario_origin: Option<&str>,
    install_roots: &[PathBuf],
    executable_root: &Path,
) -> Result<Option<Vec<FolderMaterialGroup>>, (PathBuf, GroupError)> {
    let actual = scenario_install_root(scenario_path, install_roots)
        .ok()
        .map(|(scenario_root, scenario_relative)| {
            folder_material_groups(scenario_root, scenario_relative)
        })
        .transpose()?;
    let origin = scenario_origin
        .and_then(|origin| {
            resolve_origin_location(origin, install_roots, executable_root).filter(
                |(root, relative)| !loader_items_identical(&root.join(relative), scenario_path),
            )
        })
        .map(|(root, relative)| origin_folder_material_groups(&root, &relative))
        .transpose()?;

    if actual.is_none() && origin.is_none() {
        return Ok(None);
    }

    let mut registered = Vec::new();
    for (registration_order, groups) in [actual, origin].into_iter().flatten().enumerate() {
        registered.extend(
            groups
                .into_iter()
                .map(|group| (group.folder_priority, registration_order, group)),
        );
    }
    registered.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    Ok(Some(
        registered.into_iter().map(|(_, _, group)| group).collect(),
    ))
}

/// `ItemIdentical` compares the `RealPath` of both loader spellings. On POSIX,
/// that resolves the longest existing prefix and retains any logical suffix,
/// which also works for children nested below packed groups
/// (src/StdFile.cpp:114-150,696-708).
fn loader_items_identical(left: &Path, right: &Path) -> bool {
    let left = loader_real_path(left);
    let right = loader_real_path(right);
    if cfg!(windows) {
        path_wire_bytes(&left).eq_ignore_ascii_case(&path_wire_bytes(&right))
    } else {
        left == right
    }
}

fn loader_real_path(logical: &Path) -> PathBuf {
    let logical = if logical.is_absolute() {
        logical.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(logical))
            .unwrap_or_else(|_| logical.to_path_buf())
    };

    #[cfg(windows)]
    {
        // `_fullpath` is lexical and does not require the target to exist.
        let mut normalized = PathBuf::new();
        for component in logical.components() {
            match component {
                std::path::Component::Prefix(_)
                | std::path::Component::RootDir
                | std::path::Component::Normal(_) => normalized.push(component.as_os_str()),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if normalized.file_name().is_some() {
                        normalized.pop();
                    }
                }
            }
        }
        normalized
    }

    #[cfg(not(windows))]
    {
        let mut prefix = logical.clone();
        let mut suffix = Vec::new();
        loop {
            if let Ok(mut resolved) = fs::canonicalize(&prefix) {
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                return resolved;
            }
            let Some(component) = prefix.file_name().map(|name| name.to_os_string()) else {
                return logical;
            };
            suffix.push(component);
            if !prefix.pop() {
                return logical;
            }
        }
    }
}

fn resolve_origin_location(
    origin: &str,
    install_roots: &[PathBuf],
    executable_root: &Path,
) -> Option<(PathBuf, PathBuf)> {
    let origin = PathBuf::from(origin.replace('\\', "/"));
    if origin.is_absolute() {
        let outer = c4f_parent_paths(&origin).last()?.clone();
        let root = outer.parent()?.to_path_buf();
        let relative = origin.strip_prefix(&root).ok()?.to_path_buf();
        return folder_chain_item_is_present(&root, &relative).then_some((root, relative));
    }

    std::iter::once(executable_root)
        .chain(install_roots.iter().map(PathBuf::as_path))
        .filter(|root| !root.as_os_str().is_empty())
        .flat_map(|root| {
            let mut relatives = vec![origin.clone()];
            // Old saves retained `content/...` while current saves use paths
            // relative to the executable data root, which is itself content
            // in a split source checkout.
            if let Some(stripped) = strip_redundant_root_component(&origin, root) {
                relatives.push(stripped);
            }
            relatives
                .into_iter()
                .map(move |relative| (root.to_path_buf(), relative))
        })
        .find(|(root, relative)| folder_chain_item_is_present(root, relative))
}

fn strip_redundant_root_component(origin: &Path, root: &Path) -> Option<PathBuf> {
    let root_name = root.file_name()?;
    let mut components = origin.components();
    let first = components.next()?;
    if !path_wire_bytes(Path::new(first.as_os_str()))
        .eq_ignore_ascii_case(&path_wire_bytes(Path::new(root_name)))
    {
        return None;
    }
    Some(components.map(|component| component.as_os_str()).collect())
}

fn folder_chain_item_is_present(root: &Path, relative: &Path) -> bool {
    let Some(outer) = folder_group_paths(root, relative).into_iter().last() else {
        return false;
    };
    // Only a true lookup miss advances to another mapped executable-data
    // root. A corrupt, unreadable, or dangling selected entry shadows every
    // later root just as the single C++ ExePath does.
    match open_group_path(&outer) {
        Err(GroupError::Missing(_) | GroupError::EntryNotFound(_)) => false,
        Ok(_) | Err(_) => true,
    }
}

fn c4f_parent_paths(path: &Path) -> Vec<PathBuf> {
    let mut parents = Vec::new();
    let mut current = if has_extension(path, "c4f") {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(parent) = current.filter(|parent| has_extension(parent, "c4f")) {
        parents.push(parent.to_path_buf());
        current = parent.parent();
    }
    parents
}

/// Every registered parent folder's `Material.c4g`, innermost folder first, in
/// the order `C4GameParameters::Load` publishes them as `NRT_Material`
/// (C4GameParameters.cpp:214-220). A failure carries the path it happened at
/// so both callers can name it in their own error type.
fn folder_material_groups(
    scenario_root: &Path,
    scenario_relative: &Path,
) -> Result<Vec<FolderMaterialGroup>, (PathBuf, GroupError)> {
    let folder_paths = folder_group_paths(scenario_root, scenario_relative);
    let folder_count = folder_paths.len();
    folder_paths
        .into_iter()
        .enumerate()
        .map(|(index, folder_path)| {
            let folder =
                open_group_path(&folder_path).map_err(|source| (folder_path.clone(), source))?;
            folder_material_group(folder_path, folder, folder_count.saturating_sub(index + 1))
        })
        .filter_map(Result::transpose)
        .collect()
}

fn origin_folder_material_groups(
    scenario_root: &Path,
    scenario_relative: &Path,
) -> Result<Vec<FolderMaterialGroup>, (PathBuf, GroupError)> {
    let mut materials = Vec::new();
    for (folder_priority, folder_path) in folder_group_paths(scenario_root, scenario_relative)
        .into_iter()
        .rev()
        .enumerate()
    {
        let folder = match open_group_path(&folder_path) {
            Ok(folder) => folder,
            Err(error) => {
                tracing::warn!(
                    %error,
                    parent = %folder_path.display(),
                    "prepared host stopped at an unavailable Origin parent"
                );
                break;
            }
        };
        if let Some(material) = folder_material_group(folder_path, folder, folder_priority)? {
            materials.push(material);
        }
    }
    materials.sort_by_key(|group| std::cmp::Reverse(group.folder_priority));
    Ok(materials)
}

fn folder_material_group(
    folder_path: PathBuf,
    folder: Group,
    folder_priority: usize,
) -> Result<Option<FolderMaterialGroup>, (PathBuf, GroupError)> {
    let material_entry = folder
        .entries()
        .map_err(|source| (folder_path.clone(), source))?
        .into_iter()
        .find(|entry| matches_ascii_name(&entry.relative_path, b"Material.c4g"));
    let Some(material_entry) = material_entry else {
        return Ok(None);
    };
    let material_path = folder_path.join(&material_entry.relative_path);
    let material = folder
        .open_child_entry_exact(&material_entry)
        .map_err(|source| (material_path.clone(), source))?;
    Ok(Some(FolderMaterialGroup {
        folder_priority,
        folder,
        material,
        material_path,
    }))
}

fn folder_group_paths(scenario_root: &Path, scenario_relative: &Path) -> Vec<PathBuf> {
    c4f_parent_paths(&scenario_root.join(scenario_relative))
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
    folder_materials: Vec<FolderMaterialGroup>,
    executable_root: &Path,
) -> Result<Vec<HostInitialResourceSource>, HostGameResourceSourceError> {
    folder_materials
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

#[cfg(test)]
mod material_chain_tests {
    use super::*;
    use std::fs;

    /// A scenario folder that is itself an install root must not stand in for
    /// the global material group.
    ///
    /// `C4GameParameters::Load` publishes every registered parent folder's
    /// `Material.c4g` and then the global one, and `C4Game::InitMaterialTexture`
    /// walks that chain (`C4GameParameters.cpp:214-222`; `C4Game.cpp:901-977`) —
    /// two distinct groups, because the global comes from the installed data
    /// root rather than from the folder chain. Resolving the same group twice
    /// truncates the overload chain instead: a folder-local
    /// `TexMap.txt` that declares `OverloadTextures` and names a texture only
    /// the global group ships then loses it, and the host renders a partial map.
    #[test]
    fn the_installed_group_is_not_the_scenario_folders_own_overlay() {
        let root = std::env::temp_dir().join(format!(
            "lc-material-chain-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let folder = root.join("Scenarios.c4f");
        let installed = root.join("content");
        fs::create_dir_all(folder.join("Round.c4s")).unwrap();
        fs::create_dir_all(folder.join("Material.c4g")).unwrap();
        fs::create_dir_all(installed.join("Material.c4g")).unwrap();
        fs::write(
            folder.join("Material.c4g").join("TexMap.txt"),
            b"OverloadTextures\n",
        )
        .unwrap();
        fs::write(
            installed.join("Material.c4g").join("TexMap.txt"),
            b"# global\n",
        )
        .unwrap();

        // The scenario's own folder is an install root, exactly as it is when a
        // scenario folder is opened from outside the installation.
        let install_roots = vec![folder.clone(), installed.clone()];
        let groups =
            resolve_host_material_groups(&folder.join("Round.c4s"), None, &install_roots, &root)
                .expect("the material chain resolves");

        let roots = groups
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect::<Vec<_>>();
        assert!(
            roots.contains(&installed.join("Material.c4g")),
            "the global material group is missing from the chain: {roots:?}"
        );
        let mut unique = roots.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            roots.len(),
            "the same material group appears twice in the chain: {roots:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// The same rule on the publication side.
    ///
    /// `resolve_host_material_groups` is only the host's pre-publication load;
    /// what a client reloads afterwards is the published `NRT_Material` set. If
    /// that set carries the folder overlay twice, the client rebuilds the same
    /// truncated chain from the resources it was sent, so both have to skip a
    /// root the folder chain already contributed.
    #[test]
    fn the_published_material_resources_are_distinct_groups() {
        let root = std::env::temp_dir().join(format!(
            "lc-material-publish-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let folder = root.join("Scenarios.c4f");
        let installed = root.join("content");
        fs::create_dir_all(folder.join("Round.c4s")).unwrap();
        fs::create_dir_all(folder.join("Material.c4g")).unwrap();
        fs::create_dir_all(installed.join("Material.c4g")).unwrap();
        fs::create_dir_all(installed.join("System.c4g")).unwrap();
        fs::write(
            folder.join("Material.c4g").join("TexMap.txt"),
            b"OverloadTextures\n",
        )
        .unwrap();
        fs::write(
            installed.join("Material.c4g").join("TexMap.txt"),
            b"# global\n",
        )
        .unwrap();

        let install_roots = vec![folder.clone(), installed.clone()];
        let sources = resolve_host_game_resource_sources(
            folder.join("Round.c4s"),
            None,
            &install_roots,
            &[],
            &root,
        )
        .expect("the host resource set resolves");

        let names = sources
            .materials
            .iter()
            .map(|resource| resource.path.clone())
            .collect::<Vec<_>>();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            names.len(),
            "the same material group is published twice: {names:?}"
        );
        assert_eq!(
            sources.materials.len(),
            2,
            "the folder overlay and the global group are both published: {names:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
