//! `C4UpdatePackage::Execute` — applying a `.c4u` to its target group.
//!
//! The order of the checks matters, and so does one thing that looks like a
//! bug and must be reproduced rather than fixed
//! (`C4Update.cpp`):
//!
//! 1. **Already updated?** If `GrpContentsCRC2` is set and the target's
//!    contents CRC already equals it, there is nothing to do and that is a
//!    *success*, not a refusal.
//! 2. **Is this a known source?** C++ first looks for a match in
//!    `GrpContentsCRC1[i]` — which in a C++-produced package is *uninitialised
//!    memory* — and only then falls back to comparing the target's file CRC
//!    against `GrpChks1[i]`. The `GrpContentsCRC1[i] &&` guard is what stops
//!    the garbage matching by accident, and the fallback is what makes real
//!    packages work at all. Skipping the first comparison would accept
//!    packages C++ rejects.
//! 3. **Apply**: copy every package entry except the two metadata files, then
//!    delete every target entry the manifest does not name.
//! 4. **Verdict**:
//!    `(!GrpContentsCRC2 || GrpContentsCRC2 != resultContents) && resultFile != GrpChks2`
//!    is failure — so a result whose *contents* match passes even when the
//!    repack is not byte-identical.

use crate::make_update::{UPDATE_CORE_ENTRY, UPDATE_ENTRIES_ENTRY};
use crate::update_core::{entry_crc, group_file_crc, UpdateCore};
use crate::update_entries::{entries_to_delete, parse_entry_list};
use std::path::Path;

/// What applying a package did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplyOutcome {
    Applied,
    /// The target already carries the update's contents.
    AlreadyUpdated,
}

/// `C4Group_GetFileContentsCRC` — the XOR of every entry's CRC.
fn contents_crc(group: &clonk_resources::Group) -> Result<u32, String> {
    let entries = group.entries().map_err(|error| error.to_string())?;
    let mut crc = 0;
    for entry in entries.iter().filter(|entry| !entry.is_directory) {
        let bytes = group
            .read_entry_bytes_exact(entry)
            .map_err(|error| error.to_string())?;
        crc ^= entry_crc(&String::from_utf8_lossy(&entry.name_bytes), &bytes);
    }
    Ok(crc)
}

fn read_text(group: &clonk_resources::Group, name: &str) -> Result<String, String> {
    group
        .read_entry_bytes(name)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| format!("{name}: {error}"))
}

/// `C4UpdatePackage::Execute` for a flat group update.
pub(crate) fn apply_update(package_path: &str) -> Result<ApplyOutcome, String> {
    let package = clonk_resources::Group::open(Path::new(package_path))
        .map_err(|error| format!("{package_path}: {error}"))?;
    let core = UpdateCore::from_ini(&read_text(&package, UPDATE_CORE_ENTRY)?);
    let manifest = parse_entry_list(&read_text(&package, UPDATE_ENTRIES_ENTRY)?);
    if !core.group_update {
        return Err("only group updates are supported".to_owned());
    }

    let target_path = core.dest_path.clone();
    let target_bytes =
        std::fs::read(&target_path).map_err(|error| format!("{target_path}: {error}"))?;
    let target = clonk_resources::Group::open(Path::new(&target_path))
        .map_err(|error| format!("{target_path}: {error}"))?;

    let target_contents = contents_crc(&target)?;
    if core.target_contents_crc != 0 && core.target_contents_crc == target_contents {
        return Ok(ApplyOutcome::AlreadyUpdated);
    }

    // The two-step source check, garbage-guard included.
    let known_source = core
        .source_contents_crcs
        .iter()
        .any(|crc| *crc != 0 && *crc == target_contents)
        || core
            .source_checksums
            .contains(&group_file_crc(&target_bytes));
    if !known_source {
        return Err(format!(
            "{target_path}: does not match any source version this package updates"
        ));
    }

    let filename = Path::new(&target_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| target_path.clone());
    let mut mutable =
        crate::edit::to_mutable(&target, &filename).map_err(|error| error.to_string())?;

    // Copy every package entry except the metadata pair.
    for entry in package.entries().map_err(|error| error.to_string())? {
        let name = String::from_utf8_lossy(&entry.name_bytes).into_owned();
        if name == UPDATE_CORE_ENTRY || name == UPDATE_ENTRIES_ENTRY || entry.is_directory {
            continue;
        }
        let bytes = package
            .read_entry_bytes_exact(&entry)
            .map_err(|error| format!("{name}: {error}"))?;
        mutable.remove_entry(&name);
        mutable
            .add_file_bytes(name.clone(), bytes)
            .map_err(|error| format!("{name}: {error}"))?;
    }

    // Then drop whatever the manifest does not name.
    let present: Vec<String> = target
        .entries()
        .map_err(|error| error.to_string())?
        .iter()
        .filter(|entry| !entry.is_directory)
        .map(|entry| String::from_utf8_lossy(&entry.name_bytes).into_owned())
        .collect();
    for stale in entries_to_delete(&present, &manifest) {
        mutable.remove_entry(stale);
    }

    crate::edit::write_back(&mutable, Path::new(&target_path))
        .map_err(|error| format!("{target_path}: {error}"))?;

    // The verdict: contents CRC *or* file CRC.
    let updated_bytes =
        std::fs::read(&target_path).map_err(|error| format!("{target_path}: {error}"))?;
    let updated = clonk_resources::Group::open(Path::new(&target_path))
        .map_err(|error| format!("{target_path}: {error}"))?;
    let result_contents = contents_crc(&updated)?;
    let contents_ok = core.target_contents_crc != 0 && core.target_contents_crc == result_contents;
    if contents_ok || group_file_crc(&updated_bytes) == core.target_checksum {
        Ok(ApplyOutcome::Applied)
    } else {
        Err(format!(
            "{target_path}: update result does not match the target"
        ))
    }
}
