//! `C4UpdatePackage::MkUp`'s diff — which entries a `.c4u` must carry.
//!
//! `MkUp` (`C4Update.cpp`) walks the **target** group's entries and decides two
//! separate things, which are easy to conflate:
//!
//! - **Which entries to copy** into the update group. An entry is changed when
//!   its `EntrySize` *or* its `EntryCRC32` differs from the source's entry of
//!   the same name — the comparison is size-then-CRC, never a byte compare —
//!   or when there is no source group at all.
//! - **Whether the group is written at all** (`includeInUpdate`). That is set
//!   by a copied entry, but *also* by a header difference or an **entry-order**
//!   difference, independently of whether any content changed. Two groups with
//!   identical entries in a different order still produce an update.
//!
//! `AllowMissingTarget` short-circuits both: everything is treated as changed.
//!
//! The manifest is written **always**, and lists **every** target entry — not
//! just the changed ones — because `DoGrpUpdate` deletes whatever the manifest
//! does not name. See [`crate::update_entries`].

use crate::update_core::{group_file_crc, UpdateCore};
use crate::update_entries::{format_entry_list, UpdateEntry};
use std::path::Path;

/// `C4CFN_UpdateCore` / `C4CFN_UpdateEntries`.
pub(crate) const UPDATE_CORE_ENTRY: &str = "AutoUpdate.txt";
pub(crate) const UPDATE_ENTRIES_ENTRY: &str = "GRPUP_Entries.txt";

/// One entry as `MkUp` compares it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UpdateEntrySource {
    pub(crate) name: String,
    /// `C4Group::EntrySize`.
    pub(crate) size: u64,
    /// `C4Group::EntryCRC32`.
    pub(crate) crc32: u32,
    /// `C4Group::EntryTime`, carried into the manifest.
    pub(crate) time: i64,
}

/// What `MkUp` decided for one group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UpdatePlan {
    /// Entries to copy into the update, in target order.
    pub(crate) changed: Vec<String>,
    /// `GRPUP_Entries.txt` — every target entry, always.
    pub(crate) manifest: Vec<UpdateEntry>,
    /// `includeInUpdate`.
    pub(crate) include_in_update: bool,
}

/// `C4UpdatePackage::MkUp`'s per-group decision.
///
/// `source` is `None` when the target group has no counterpart, which C++
/// signals with a null `pGrp1` and treats as "everything changed".
/// `headers_differ` covers `MkUp`'s creation/original/maker/password check.
pub(crate) fn plan_update(
    source: Option<&[UpdateEntrySource]>,
    target: &[UpdateEntrySource],
    allow_missing_target: bool,
    headers_differ: bool,
) -> UpdatePlan {
    let mut include_in_update = headers_differ || source.is_none();

    // The order check walks both lists in step: C++ advances its own
    // `FindNextEntry` cursor over the source once per target entry, so a
    // missing or differently-named entry at the same position is a difference
    // even when every name is present somewhere.
    if let Some(source) = source {
        if !allow_missing_target {
            let ordered = source.len() >= target.len()
                && source
                    .iter()
                    .zip(target)
                    .all(|(source, target)| source.name == target.name);
            if !ordered {
                include_in_update = true;
            }
        }
    }

    let changed: Vec<String> = target
        .iter()
        .filter(|entry| {
            if allow_missing_target {
                return true;
            }
            match source.and_then(|source| source.iter().find(|held| held.name == entry.name)) {
                // Size then CRC32 — never a byte comparison.
                Some(previous) => previous.size != entry.size || previous.crc32 != entry.crc32,
                None => true,
            }
        })
        .map(|entry| entry.name.clone())
        .collect();

    UpdatePlan {
        include_in_update: include_in_update || !changed.is_empty(),
        changed,
        manifest: target
            .iter()
            .map(|entry| UpdateEntry {
                name: entry.name.clone(),
                time: entry.time,
            })
            .collect(),
    }
}

/// Reads one group's entries in the form `MkUp` compares them.
///
/// `EntryCRC32` is the stored CRC when the entry carries one and a computed
/// CRC otherwise, which is what C++'s lazy `EntryCRC32` does.
fn entries_for_diff(
    group: &clonk_resources::Group,
) -> Result<Vec<UpdateEntrySource>, clonk_resources::GroupError> {
    group
        .entries()?
        .into_iter()
        .filter(|entry| !entry.is_directory)
        .map(|entry| {
            let crc32 = if entry.crc_state != 0 {
                entry.stored_crc
            } else {
                group_file_crc(&group.read_entry_bytes_exact(&entry)?)
            };
            Ok(UpdateEntrySource {
                name: String::from_utf8_lossy(&entry.name_bytes).into_owned(),
                size: entry.size,
                crc32,
                time: i64::from(entry.time),
            })
        })
        .collect()
}

/// `C4UpdatePackage::MakeUpdate` for a single (non-recursive) group pair.
///
/// Writes `output`: the `[Update]` core, the full entry manifest, and every
/// changed entry. `GrpChks1`/`GrpChks2` are the **file** CRCs of the two
/// groups, which is what `Check` compares a candidate target against.
///
/// Child groups are not descended into yet; `MkUp` recurses, and a nested
/// difference is currently reported rather than packed.
pub(crate) fn generate_update(
    source_path: &str,
    target_path: &str,
    output_path: &str,
    title: &str,
    allow_missing_target: bool,
) -> Result<bool, String> {
    let read = |path: &str| std::fs::read(path).map_err(|error| format!("{path}: {error}"));
    let open = |path: &str| {
        clonk_resources::Group::open(Path::new(path)).map_err(|error| format!("{path}: {error}"))
    };

    let source_bytes = read(source_path)?;
    let target_bytes = read(target_path)?;
    let source = open(source_path)?;
    let target = open(target_path)?;

    let source_entries = entries_for_diff(&source).map_err(|error| error.to_string())?;
    let target_entries = entries_for_diff(&target).map_err(|error| error.to_string())?;
    if source
        .entries()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|e| e.is_directory)
        || target
            .entries()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|e| e.is_directory)
    {
        return Err(format!(
            "{output_path}: child groups are not supported by this update generator yet"
        ));
    }

    let plan = plan_update(
        Some(&source_entries),
        &target_entries,
        allow_missing_target,
        false,
    );

    let core = UpdateCore {
        // `FormatWithNull(Name, "{} Update", GetFilename(strFile1))` when no
        // title is given (`C4Update.cpp`).
        name: if title.is_empty() {
            format!("{} Update", file_name(source_path))
        } else {
            title.to_owned()
        },
        dest_path: source_path.to_owned(),
        group_update: true,
        allow_missing_target,
        source_checksums: vec![group_file_crc(&source_bytes)],
        target_checksum: group_file_crc(&target_bytes),
        source_contents_crcs: vec![0],
        target_contents_crc: 0,
    };

    let mut update = clonk_resources::MutableGroup::new(file_name(output_path));
    let put = |update: &mut clonk_resources::MutableGroup,
               name: &str,
               bytes: Vec<u8>|
     -> Result<(), String> {
        update
            .add_file_bytes(name, bytes)
            .map_err(|error| format!("{name}: {error}"))
    };
    put(&mut update, UPDATE_CORE_ENTRY, core.to_ini().into_bytes())?;
    put(
        &mut update,
        UPDATE_ENTRIES_ENTRY,
        format_entry_list(&plan.manifest).into_bytes(),
    )?;
    for name in &plan.changed {
        let bytes = target
            .read_entry_bytes(name)
            .map_err(|error| format!("{name}: {error}"))?;
        put(&mut update, name, bytes)?;
    }
    crate::edit::write_back(&update, Path::new(output_path))
        .map_err(|error| format!("{output_path}: {error}"))?;
    Ok(plan.include_in_update)
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64, crc32: u32) -> UpdateEntrySource {
        UpdateEntrySource {
            name: name.to_owned(),
            size,
            crc32,
            time: 1_785_343_178,
        }
    }

    // C4Update.cpp MkUp — the change rule, the order rule, and the manifest.
    #[test]
    fn update_plan_copies_changed_entries_and_lists_every_target_entry() {
        // The shape of the fixture used to study the C++ tool: a.txt edited,
        // keep.txt untouched, removed.txt dropped, added.txt new. The package
        // the oracle's own `c4group -g` produced for it contained exactly
        // `a.txt` and `added.txt` beside the two metadata files.
        let source = [
            entry("a.txt", 10, 1),
            entry("keep.txt", 5, 2),
            entry("removed.txt", 5, 9),
        ];
        let target = [
            entry("a.txt", 11, 3),
            entry("keep.txt", 5, 2),
            entry("added.txt", 8, 4),
        ];

        let plan = plan_update(Some(&source), &target, false, false);
        // Changed content and a new entry are copied; an identical one is not.
        assert_eq!(plan.changed, vec!["a.txt", "added.txt"]);
        assert!(plan.include_in_update);

        // The manifest lists EVERY target entry, not just the changed ones —
        // DoGrpUpdate deletes whatever it does not name, which is how
        // removed.txt disappears without the package carrying anything for it.
        assert_eq!(
            plan.manifest
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt", "keep.txt", "added.txt"]
        );
        assert!(
            !plan
                .manifest
                .iter()
                .any(|entry| entry.name == "removed.txt"),
            "a dropped entry is deleted by its absence from the manifest"
        );

        // A same-size, same-CRC entry is unchanged even with a different time,
        // because the comparison is size-then-CRC only.
        let retimed = [UpdateEntrySource {
            time: 999,
            ..entry("keep.txt", 5, 2)
        }];
        let unchanged = plan_update(Some(&[entry("keep.txt", 5, 2)]), &retimed, false, false);
        assert!(unchanged.changed.is_empty());
        assert!(!unchanged.include_in_update);
        assert_eq!(unchanged.manifest[0].time, 999, "the manifest keeps it");

        // Same size, different CRC is changed; same CRC, different size too.
        assert_eq!(
            plan_update(
                Some(&[entry("a", 10, 1)]),
                &[entry("a", 10, 2)],
                false,
                false
            )
            .changed,
            vec!["a"]
        );
        assert_eq!(
            plan_update(
                Some(&[entry("a", 10, 1)]),
                &[entry("a", 11, 1)],
                false,
                false
            )
            .changed,
            vec!["a"]
        );

        // No source group at all: everything is changed.
        let fresh = plan_update(None, &target, false, false);
        assert_eq!(fresh.changed.len(), 3);
        assert!(fresh.include_in_update);

        // AllowMissingTarget short-circuits the comparison entirely.
        let permissive = plan_update(Some(&source), &target, true, false);
        assert_eq!(permissive.changed.len(), 3);

        // A header difference forces the group into the update even when no
        // entry changed.
        let headers = plan_update(
            Some(&[entry("keep.txt", 5, 2)]),
            &[entry("keep.txt", 5, 2)],
            false,
            true,
        );
        assert!(headers.changed.is_empty());
        assert!(
            headers.include_in_update,
            "creation/maker/password differences alone force an update"
        );

        // So does a pure **reordering**, with identical content.
        let reordered = plan_update(
            Some(&[entry("a", 1, 1), entry("b", 2, 2)]),
            &[entry("b", 2, 2), entry("a", 1, 1)],
            false,
            false,
        );
        assert!(reordered.changed.is_empty());
        assert!(
            reordered.include_in_update,
            "the same entries in a different order still produce an update"
        );

        // A source with fewer entries counts as an order difference too, which
        // is how C++'s FindNextEntry running out is observed.
        let shorter = plan_update(
            Some(&[entry("a", 1, 1)]),
            &[entry("a", 1, 1), entry("b", 2, 2)],
            false,
            false,
        );
        assert_eq!(shorter.changed, vec!["b"]);
        assert!(shorter.include_in_update);

        // An empty target writes an empty manifest and nothing else.
        let empty = plan_update(Some(&source), &[], false, false);
        assert!(empty.changed.is_empty());
        assert!(empty.manifest.is_empty());
    }
}
