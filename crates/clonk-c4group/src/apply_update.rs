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

#[cfg(test)]
mod tests {
    use super::{apply_update, ApplyOutcome};
    use clonk_resources::MutableGroup;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn scratch() -> std::path::PathBuf {
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "clonk-rust-apply-update-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_group(path: &std::path::Path, name: &str, payload: &[u8]) {
        let mut group = MutableGroup::new(name);
        group.set_maker("OracleHost");
        group
            .add_file_with_metadata("Data.bin", payload.to_vec(), 1, false)
            .unwrap();
        std::fs::write(path, group.pack().unwrap()).unwrap();
    }

    /// Re-applying a `.c4u` whose contents the target already carries is a
    /// **success**, not a refusal (`C4UpdatePackage::Execute`, `C4Update.cpp`).
    ///
    /// This is the first check in the sequence and the easy one to get
    /// backwards: an updater that treats "already at this version" as an error
    /// turns an idempotent operation into a hard failure, which is what a
    /// resumed or retried update looks like.
    ///
    /// `apply_update` had no test at all — the module implements the whole
    /// `Execute` sequence, including a garbage-guard that exists to reproduce a
    /// C++ uninitialised-memory read, and nothing exercised any of it.
    #[test]
    fn applying_an_update_twice_reports_already_updated_rather_than_failing() {
        let directory = scratch();
        let old_path = directory.join("old.c4g");
        let new_path = directory.join("new.c4g");
        let package_path = directory.join("update.c4u");

        write_group(&old_path, "old.c4g", b"version one");
        write_group(&new_path, "new.c4g", b"version two");

        // `dest_path` is the *source* path, so the package rewrites `old.c4g`
        // in place to carry `new.c4g`'s contents (make_update.rs:217).
        let generated = crate::make_update::generate_update(
            old_path.to_str().unwrap(),
            new_path.to_str().unwrap(),
            package_path.to_str().unwrap(),
            "round trip",
            false,
        )
        .expect("the update package generates");
        assert!(generated, "the two versions differ, so there is an update");

        assert_eq!(
            apply_update(package_path.to_str().unwrap()).expect("the first application succeeds"),
            ApplyOutcome::Applied
        );
        // Contents, not bytes: the module's own verdict rule accepts a result
        // whose contents CRC matches even when the repack is not byte-identical,
        // and a repack legitimately differs (header filename, entry order).
        let updated = clonk_resources::Group::open(&old_path).expect("the updated target opens");
        let entry = updated
            .entries()
            .expect("entries")
            .into_iter()
            .find(|entry| entry.name_bytes == b"Data.bin")
            .expect("the payload survives the update");
        assert_eq!(
            updated
                .read_entry_bytes_exact(&entry)
                .expect("payload reads"),
            b"version two".to_vec(),
            "the target now carries the new version's contents"
        );

        // The second application is the case under test: the contents CRC
        // already matches, so `Execute` returns success without touching it.
        assert_eq!(
            apply_update(package_path.to_str().unwrap())
                .expect("re-applying an already-applied update is not an error"),
            ApplyOutcome::AlreadyUpdated
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A package refuses a target that is not one of the source versions it
    /// knows, rather than applying over it.
    ///
    /// This is the two-step source check whose first comparison guards against
    /// C++'s uninitialised `GrpContentsCRC1[i]`: skipping it would accept
    /// packages C++ rejects, and skipping the `GrpChks1` fallback would reject
    /// packages C++ accepts.
    #[test]
    fn applying_an_update_to_an_unknown_version_is_refused() {
        let directory = scratch();
        let old_path = directory.join("old.c4g");
        let new_path = directory.join("new.c4g");
        let package_path = directory.join("update.c4u");

        write_group(&old_path, "old.c4g", b"version one");
        write_group(&new_path, "new.c4g", b"version two");
        crate::make_update::generate_update(
            old_path.to_str().unwrap(),
            new_path.to_str().unwrap(),
            package_path.to_str().unwrap(),
            "round trip",
            false,
        )
        .expect("the update package generates");

        // Replace the target with a version the package has never seen.
        write_group(&old_path, "old.c4g", b"version three");

        let error = apply_update(package_path.to_str().unwrap())
            .expect_err("an unknown source version must be refused");
        assert!(
            error.contains("does not match any source version"),
            "unexpected refusal: {error}"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }
}
