//! `GRPUP_Entries.txt` — the entry manifest inside a `.c4u` update package.
//!
//! `C4UpdatePackage::MkUp` writes one `name=mtime` record per entry of the
//! target group, `|`-separated (`C4Update.cpp`, the `entryList` in `MkUp`), and
//! `DoGrpUpdate` reads it back to do two things: **delete every entry of the
//! target group that is not in the list**, and then set each listed entry's
//! time and sort order.
//!
//! # A proven C++ defect this module deliberately does not reproduce
//!
//! In the pinned source `MkUp` builds each record with
//!
//! ```cpp
//! char strItemName[_MAX_PATH];
//! strItemName[0] = strItemName2[0] = 0;
//! ...
//! entryList += std::format("{}={}", strItemName, pGrp2->EntryTime(strItemName));
//! ```
//!
//! On the pinned arm64 macOS build that `std::format` writes the **whole
//! `_MAX_PATH` array**, not the NUL-terminated name, so every record carries
//! about a kilobyte of uninitialised stack memory between the name and its `=`.
//! It is observable three ways, all reproduced against
//! `build-arm64-native/c4group`:
//!
//! 1. `GRPUP_Entries.txt` in a generated `.c4u` contains the garbage.
//! 2. `Update.log`'s `"{}\\{}: update"` lines contain it too — the same buffer.
//! 3. It is **not cosmetic**. `DoGrpUpdate` matches list names against real
//!    entry names with `SEqual`, so a garbage-suffixed name matches nothing,
//!    *every* entry is deleted, and applying the package fails. Generating a
//!    package with `c4group -g` and applying it with `c4group -y` fails on the
//!    same machine that produced it.
//!
//! The same omission appears in `C4UpdatePackageCore`'s constructor, which
//! initialises `GrpChks1` but **not** `GrpContentsCRC1`/`GrpContentsCRC2`, so
//! `AutoUpdate.txt` carries fifty uninitialised words in `GrpContentsCRC1`.
//! `Check` then compares against them, and only works because it falls through
//! to the `GrpChks1` comparison.
//!
//! Consequences for the port, all of which follow from the above:
//!
//! - **Byte-identical output is not an achievable acceptance criterion.** Three
//!   runs of C++ `c4group -g` over identical inputs produce three different
//!   files, because the garbage differs per run.
//! - Writing a correct manifest is a **fix**, not a divergence to justify: an
//!   update package is not simulation state, so this cannot affect determinism.
//! - Reading must stay **tolerant**, because packages produced by the C++ tool
//!   exist in the wild. [`parse_entry_list`] therefore truncates a record's
//!   name at the first NUL or control byte.
//!
//! Caveat: this was reproduced on the pinned arm64 macOS build. `std::format`
//! over `char[N]` may behave differently under another standard library, so the
//! defect may be toolchain-specific rather than universal.

/// One record of the manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UpdateEntry {
    pub(crate) name: String,
    /// `C4Group::EntryTime`, a Unix timestamp.
    pub(crate) time: i64,
}

/// Writes the manifest the way `MkUp` intends to: `name=time`, `|`-separated,
/// with no separator before the first record.
pub(crate) fn format_entry_list(entries: &[UpdateEntry]) -> String {
    entries
        .iter()
        .map(|entry| format!("{}={}", entry.name, entry.time))
        .collect::<Vec<_>>()
        .join("|")
}

/// Reads a manifest, tolerating C++-produced records whose name is followed by
/// uninitialised bytes.
///
/// `DoGrpUpdate` splits on `|` and cuts each segment at the first `=`; a record
/// with no `=` keeps its whole segment as the name and gets no time, which is
/// what `if (pTime)` guards. Empty segments are skipped rather than becoming
/// nameless entries.
pub(crate) fn parse_entry_list(raw: &str) -> Vec<UpdateEntry> {
    raw.split('|')
        .filter_map(|segment| {
            let (name, time) = match segment.split_once('=') {
                Some((name, time)) => (name, time.trim().parse().ok().unwrap_or(0)),
                None => (segment, 0),
            };
            // A C++-written record carries uninitialised bytes between the name
            // and its `=`. Real entry names are printable and NUL-free, so cut
            // at the first byte that is not.
            let name: String = name
                .chars()
                .take_while(|character| !character.is_control() && *character != '\u{0}')
                .collect();
            (!name.is_empty()).then_some(UpdateEntry { name, time })
        })
        .collect()
}

/// `DoGrpUpdate`'s first pass: every entry of the target group that the
/// manifest does not name is deleted.
pub(crate) fn entries_to_delete<'a>(
    target_entries: &'a [String],
    manifest: &[UpdateEntry],
) -> Vec<&'a String> {
    target_entries
        .iter()
        .filter(|entry| !manifest.iter().any(|listed| listed.name == **entry))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, time: i64) -> UpdateEntry {
        UpdateEntry {
            name: name.to_owned(),
            time,
        }
    }

    // C4Update.cpp MkUp/DoGrpUpdate — the manifest round-trip, tolerant reading
    // of C++'s corrupted records, and the delete-what-is-not-listed pass.
    #[test]
    fn update_entry_manifest_round_trips_and_tolerates_cpp_uninitialised_names() {
        let entries = vec![entry("a.txt", 1785343178), entry("added.txt", 1785343179)];
        let raw = format_entry_list(&entries);
        assert_eq!(raw, "a.txt=1785343178|added.txt=1785343179");
        assert_eq!(parse_entry_list(&raw), entries);

        // No leading separator, and an empty manifest is the empty string
        // rather than a stray `|`.
        assert_eq!(format_entry_list(&[]), "");
        assert!(parse_entry_list("").is_empty());

        // A record produced by the pinned C++ build carries uninitialised bytes
        // between the name and its `=`. Reading must recover the real name, or
        // DoGrpUpdate deletes every entry it cannot match.
        let corrupted = "a.txt\u{1}\u{0}\u{7f}=1785343178|added.txt\u{2}=1785343179";
        assert_eq!(
            parse_entry_list(corrupted),
            entries,
            "a garbage suffix must not become part of the entry name"
        );

        // A record with no `=` keeps its whole segment and no time, which is
        // what `if (pTime)` guards against in DoGrpUpdate.
        assert_eq!(parse_entry_list("lonely"), vec![entry("lonely", 0)]);
        // Empty segments are skipped rather than becoming nameless entries.
        assert_eq!(parse_entry_list("a.txt=1||b.txt=2").len(), 2);

        // The delete pass: anything the manifest does not name goes.
        let target = vec![
            "a.txt".to_owned(),
            "removed.txt".to_owned(),
            "added.txt".to_owned(),
        ];
        assert_eq!(
            entries_to_delete(&target, &entries),
            vec![&"removed.txt".to_owned()]
        );
        // With the corrupted manifest read tolerantly, nothing is lost...
        assert_eq!(
            entries_to_delete(&target, &parse_entry_list(corrupted)),
            vec![&"removed.txt".to_owned()]
        );
        // ...whereas taking those names literally would delete the whole group,
        // which is exactly the C++ failure this module exists to avoid.
        let literal: Vec<UpdateEntry> = corrupted
            .split('|')
            .map(|segment| {
                let (name, _) = segment.split_once('=').unwrap_or((segment, ""));
                entry(name, 0)
            })
            .collect();
        assert_eq!(
            entries_to_delete(&target, &literal).len(),
            target.len(),
            "literal parsing deletes every entry — the observed C++ behaviour"
        );
    }
}
