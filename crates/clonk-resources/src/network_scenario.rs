//! Client-side scenario/dynamic composition used by C++ network bootstrap.

use std::collections::HashSet;

use thiserror::Error;

use crate::group_writer::{c4group_entry_crc, compress_c4group_image};
use crate::{Group, GroupEntry, GroupError, MutableGroup, MutableGroupError};

const MATERIAL_GROUP: &[u8] = b"Material.c4g";

#[derive(Debug, Error)]
pub enum NetworkScenarioError {
    #[error(transparent)]
    Group(#[from] GroupError),
    #[error(transparent)]
    Writer(#[from] MutableGroupError),
    #[error("both network resources contain Material.c4g, but one is not a child group")]
    InvalidMaterialGroup,
}

/// Builds the locally packed scenario that C++ creates in
/// `C4Network2::RetrieveScenario`.
///
/// Dynamic top-level entries replace scenario entries by legacy byte name.
/// Ordinary child groups retain their exact nested image. When both inputs
/// contain `Material.c4g`, C++ first unpacks both children; its folder-target
/// `C4Group::Merge` then retains and rebuilds the scenario material directory.
/// This perhaps surprising behavior is intentional here because network
/// parity follows the executable, including that edge case.
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
        (Some(scenario_entry), Some(dynamic_entry)) => {
            open_child_entry_exact(scenario, scenario_entry)
                .map_err(|_| NetworkScenarioError::InvalidMaterialGroup)?;
            open_child_entry_exact(dynamic, dynamic_entry)
                .map_err(|_| NetworkScenarioError::InvalidMaterialGroup)?;
            true
        }
        _ => false,
    };

    let overwritten = dynamic_entries
        .iter()
        .filter(|entry| !(retain_scenario_material && is_material_entry(entry)))
        .map(|entry| case_fold(&entry.name_bytes))
        .collect::<HashSet<_>>();
    let mut combined = MutableGroup::new(output_filename);
    combined.set_maker(maker);
    for entry in scenario_entries
        .iter()
        .filter(|entry| !overwritten.contains(&case_fold(&entry.name_bytes)))
    {
        if retain_scenario_material && is_material_entry(entry) {
            rebuild_material_entry(&mut combined, scenario, entry, maker)?;
        } else {
            copy_entry(&mut combined, scenario, entry)?;
        }
    }
    for entry in dynamic_entries
        .iter()
        .filter(|entry| !(retain_scenario_material && is_material_entry(entry)))
    {
        copy_entry(&mut combined, dynamic, entry)?;
    }
    combined.pack().map_err(Into::into)
}

/// Overlay every top-level source entry as if a packed group had first been
/// extracted and then passed to `C4Group::Merge`.
///
/// In particular, an ordinary entry whose bytes form a standalone C4Group is
/// promoted to a child group during the destination rewrite. Matching names
/// replace earlier entries in source order.
pub fn merge_extracted_group_entries(
    target: &mut MutableGroup,
    source: &Group,
) -> Result<(), NetworkScenarioError> {
    for entry in source.entries()? {
        copy_entry(target, source, &entry)?;
    }
    Ok(())
}

fn copy_entry(
    target: &mut MutableGroup,
    source: &Group,
    entry: &GroupEntry,
) -> Result<(), NetworkScenarioError> {
    if source.is_directory() && entry.is_directory {
        let child = source.open_child(&entry.relative_path)?;
        let child = MutableGroup::from_group(&child)?;
        target.add_existing_child_bytes_with_metadata(
            entry.name_bytes.clone(),
            child,
            entry.time,
            false,
        )?;
        return Ok(());
    }

    // Directory-backed groups do not carry a child flag for packed group
    // files. Detect those exactly as C4Group_IsGroup does during the final
    // directory pack, then retain their uncompressed image opaquely.
    if source.is_directory() {
        let path = source.root().join(&entry.relative_path);
        if let Ok(child) = Group::open(path) {
            let data = child.raw_image()?;
            let contents_crc = child.contents_crc()?;
            target.add_packed_child_bytes_with_metadata(
                entry.name_bytes.clone(),
                data,
                contents_crc,
                entry.time,
                cfg!(target_os = "linux") && entry.executable,
            )?;
            return Ok(());
        }
    }

    let data = source.read_entry_bytes_exact(entry)?;
    if entry.is_directory {
        if let Ok(child) = Group::from_raw_memory(entry.relative_path.clone(), data.clone()) {
            target.add_packed_child_bytes_with_metadata(
                entry.name_bytes.clone(),
                data,
                child.contents_crc_or_zero(),
                entry.time,
                cfg!(target_os = "linux") && entry.executable,
            )?;
        } else {
            // UnpackDirectory still writes a child-marked payload to a
            // standalone gzip file. Merge then runs C4Group_IsGroup on that
            // file and demotes an invalid image to an ordinary entry.
            let data = compress_c4group_image(&data)?;
            let contents_crc = c4group_entry_crc(&data, &entry.name_bytes);
            target.add_existing_file_bytes_with_metadata(
                entry.name_bytes.clone(),
                data,
                contents_crc,
                entry.time,
                cfg!(target_os = "linux") && entry.executable,
            )?;
        }
    } else if let Ok(child) =
        Group::from_top_level_memory(entry.relative_path.clone(), data.clone())
    {
        // Root extraction discards the original child-core flag. During the
        // final folder pack C4Group_IsGroup promotes any valid standalone
        // group file and stores its uncompressed image as an opaque child.
        let data = child.raw_image()?;
        let contents_crc = child.contents_crc_or_zero();
        target.add_packed_child_bytes_with_metadata(
            entry.name_bytes.clone(),
            data,
            contents_crc,
            entry.time,
            cfg!(target_os = "linux") && entry.executable,
        )?;
    } else {
        let contents_crc = c4group_entry_crc(&data, &entry.name_bytes);
        target.add_existing_file_bytes_with_metadata(
            entry.name_bytes.clone(),
            data,
            contents_crc,
            entry.time,
            cfg!(target_os = "linux") && entry.executable,
        )?;
    }
    Ok(())
}

fn rebuild_material_entry(
    target: &mut MutableGroup,
    source: &Group,
    entry: &GroupEntry,
    maker: &str,
) -> Result<(), NetworkScenarioError> {
    let source = open_child_entry_exact(source, entry)
        .map_err(|_| NetworkScenarioError::InvalidMaterialGroup)?;
    let mut rebuilt = MutableGroup::new_bytes(entry.name_bytes.clone());
    rebuilt.set_maker(maker);
    for child_entry in source.entries()? {
        copy_entry(&mut rebuilt, &source, &child_entry)?;
    }
    target.add_child_bytes(entry.name_bytes.clone(), rebuilt)?;
    Ok(())
}

fn open_child_entry_exact(source: &Group, entry: &GroupEntry) -> Result<Group, GroupError> {
    if source.is_directory() {
        Group::open(source.root().join(&entry.relative_path))
    } else {
        let data = source.read_entry_bytes_exact(entry)?;
        if entry.is_directory {
            Group::from_raw_memory(entry.relative_path.clone(), data)
        } else {
            // RetrieveScenario extracts the entry before opening the native
            // Material.c4g path as a top-level group. A valid wrapped group
            // therefore opens even if its outer core did not mark it as a
            // child, while a raw unwrapped image does not.
            Group::from_top_level_memory(entry.relative_path.clone(), data)
        }
    }
}

fn material_entry(entries: &[GroupEntry]) -> Option<&GroupEntry> {
    entries.iter().find(|entry| is_material_entry(entry))
}

fn is_material_entry(entry: &GroupEntry) -> bool {
    entry.name_bytes.eq_ignore_ascii_case(MATERIAL_GROUP)
}

fn case_fold(name: &[u8]) -> Vec<u8> {
    let mut folded = name.to_vec();
    folded.make_ascii_lowercase();
    folded
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::group_writer::ImportedPackedChildCoreMetadata;
    use crate::MutableGroupEntryKind;

    use super::*;

    #[test]
    fn classic_record_merge_demotes_an_invalid_child_image_after_extraction() {
        let mut initial = MutableGroup::new("Initial.c4s");
        initial
            .add_imported_packed_child_core_bytes_with_metadata(
                b"Broken.c4g".to_vec(),
                b"not a raw group image".to_vec(),
                ImportedPackedChildCoreMetadata {
                    crc_state: 0,
                    stored_crc: 0,
                    child_contents_crc: None,
                    time: 123,
                    executable: false,
                },
            )
            .unwrap();
        let initial =
            Group::from_top_level_memory(PathBuf::from("Initial.c4s"), initial.pack().unwrap())
                .unwrap();
        let mut record = MutableGroup::new("Record.c4s");

        merge_extracted_group_entries(&mut record, &initial).unwrap();

        assert_eq!(
            record.entry_kind("Broken.c4g"),
            Some(MutableGroupEntryKind::File)
        );
        let record =
            Group::from_top_level_memory(PathBuf::from("Record.c4s"), record.pack().unwrap())
                .unwrap();
        let extracted = record.read_file("Broken.c4g").unwrap();
        assert!(Group::from_top_level_memory(PathBuf::from("Broken.c4g"), extracted).is_err());
    }
}
