//! Client-side scenario/dynamic composition used by C++ network bootstrap.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{Group, GroupEntry, GroupError, MutableGroup, MutableGroupError};

const MATERIAL_GROUP: &str = "Material.c4g";

#[derive(Debug, Error)]
pub enum NetworkScenarioError {
    #[error(transparent)]
    Group(#[from] GroupError),
    #[error(transparent)]
    Writer(#[from] MutableGroupError),
    #[error("network scenario entry name is not valid UTF-8: {0}")]
    NonUtf8Entry(PathBuf),
    #[error("both network resources contain Material.c4g, but one is not a child group")]
    InvalidMaterialGroup,
}

/// Builds the locally packed scenario that C++ creates in
/// `C4Network2::RetrieveScenario`.
///
/// Top-level files from the dynamic replace scenario files. When both inputs
/// contain `Material.c4g`, C++ first unpacks both children; its folder-target
/// `C4Group::Merge` then retains the scenario material directory. This perhaps
/// surprising behavior is intentional here because network parity follows the
/// executable, including that edge case.
pub fn combine_network_scenario(
    scenario: &Group,
    dynamic: &Group,
    output_filename: &str,
    maker: &str,
) -> Result<Vec<u8>, NetworkScenarioError> {
    let scenario_entries = scenario.entries()?;
    let dynamic_entries = dynamic.entries()?;
    let scenario_material = material_entry(&scenario_entries);
    let dynamic_material = material_entry(&dynamic_entries);
    let retain_scenario_material = match (scenario_material, dynamic_material) {
        (Some(scenario), Some(dynamic)) if scenario.is_directory && dynamic.is_directory => true,
        (Some(_), Some(_)) => return Err(NetworkScenarioError::InvalidMaterialGroup),
        _ => false,
    };

    let overwritten = dynamic_entries
        .iter()
        .filter(|entry| !(retain_scenario_material && is_material_entry(entry)))
        .map(|entry| case_fold(&entry.relative_path))
        .collect::<HashSet<_>>();
    let mut combined = MutableGroup::new(output_filename);
    combined.set_maker(maker);
    for entry in scenario_entries
        .iter()
        .filter(|entry| !overwritten.contains(&case_fold(&entry.relative_path)))
    {
        copy_entry(&mut combined, scenario, entry, maker)?;
    }
    for entry in dynamic_entries
        .iter()
        .filter(|entry| !(retain_scenario_material && is_material_entry(entry)))
    {
        copy_entry(&mut combined, dynamic, entry, maker)?;
    }
    combined.pack().map_err(Into::into)
}

fn copy_entry(
    target: &mut MutableGroup,
    source: &Group,
    entry: &GroupEntry,
    maker: &str,
) -> Result<(), NetworkScenarioError> {
    let name = entry_name(entry)?;
    if entry.is_directory {
        let child = source.open_child(&entry.relative_path)?;
        let child = copy_group(&child, &name, maker)?;
        target.add_child(name, child)?;
    } else {
        target.add_file(name, source.read_file(&entry.relative_path)?)?;
    }
    Ok(())
}

fn copy_group(
    source: &Group,
    filename: &str,
    maker: &str,
) -> Result<MutableGroup, NetworkScenarioError> {
    let mut target = MutableGroup::new(filename);
    target.set_maker(maker);
    for entry in source.entries()? {
        copy_entry(&mut target, source, &entry, maker)?;
    }
    Ok(target)
}

fn material_entry(entries: &[GroupEntry]) -> Option<&GroupEntry> {
    entries.iter().find(|entry| is_material_entry(entry))
}

fn is_material_entry(entry: &GroupEntry) -> bool {
    entry
        .relative_path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(MATERIAL_GROUP))
}

fn entry_name(entry: &GroupEntry) -> Result<String, NetworkScenarioError> {
    entry
        .relative_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| NetworkScenarioError::NonUtf8Entry(entry.relative_path.clone()))
}

fn case_fold(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}
