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

use crate::update_entries::UpdateEntry;

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
