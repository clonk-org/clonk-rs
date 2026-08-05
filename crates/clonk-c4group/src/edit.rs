//! Mutating a packed group.
//!
//! C++'s `C4Group` edits in place and rewrites on `Close`. This port's writer
//! builds groups rather than opening them for mutation, so a mutating command
//! reads the group into a fresh [`MutableGroup`] and repacks it.
//!
//! The rebuild is metadata-preserving: each entry keeps its timestamp and
//! executable bit, and a nested group is re-added as an already-packed child
//! with its stored CRC, so it is never unpacked and repacked. An untouched
//! rebuild therefore reproduces every entry core and payload byte, changing
//! only the header creation stamp that `C4Group::Close` restamps on any save.

use clonk_resources::group_writer::MutableGroup;
use clonk_resources::Group;

/// Why a mutating command could not run.
#[derive(Debug)]
pub enum EditError {
    Read(String),
    Write(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(detail) | Self::Write(detail) => formatter.write_str(detail),
        }
    }
}

/// Loads `group` into a writable copy, preserving per-entry metadata and
/// leaving nested groups packed.
pub fn to_mutable(group: &Group, filename: &str) -> Result<MutableGroup, EditError> {
    let mut mutable = MutableGroup::new_bytes(filename.as_bytes().to_vec());
    if let Some(maker) = group.maker_bytes() {
        mutable.set_maker_bytes(maker);
    }
    let entries = group
        .entries()
        .map_err(|error| EditError::Read(error.to_string()))?;
    for entry in entries {
        let data = group
            .read_entry_bytes_exact(&entry)
            .map_err(|error| EditError::Read(error.to_string()))?;
        let result = if entry.is_directory {
            // Re-added already packed, so the child's bytes and CRC survive.
            mutable.add_packed_child_bytes_with_metadata(
                entry.name_bytes.clone(),
                data,
                entry.stored_crc,
                entry.time,
                entry.executable,
            )
        } else {
            mutable.add_file_bytes_with_metadata(
                entry.name_bytes.clone(),
                data,
                entry.time,
                entry.executable,
            )
        };
        result.map_err(|error| EditError::Write(error.to_string()))?;
    }
    Ok(mutable)
}

/// `C4Group::SortRank` (`C4Group.cpp:2290-2303`): the first `|`-separated
/// segment matching `name` gives rank `(segments) - index`; no match is 0.
/// A higher rank sorts earlier.
pub fn sort_rank(name: &str, sort_list: &str) -> usize {
    let segments: Vec<&str> = sort_list.split('|').collect();
    segments
        .iter()
        .position(|segment| crate::wildcard::matches(segment, name))
        .map_or(0, |index| segments.len() - index)
}

/// `C4Group::Sort` (`C4Group.cpp:2306-2340`): primary key is the sort rank
/// descending, secondary is the case-insensitive filename ascending. C++
/// bubble-sorts with those comparisons; a stable sort on the same key matches.
pub fn sorted_entry_order(names: &[String], sort_list: &str) -> Vec<usize> {
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by(|left, right| {
        let ranks = sort_rank(&names[*right], sort_list).cmp(&sort_rank(&names[*left], sort_list));
        ranks.then_with(|| {
            names[*left]
                .to_lowercase()
                .cmp(&names[*right].to_lowercase())
        })
    });
    order
}

/// Rebuilds `group` with its entries in `order`, preserving metadata exactly as
/// [`to_mutable`] does.
pub fn to_mutable_ordered(
    group: &Group,
    filename: &str,
    order: &[usize],
) -> Result<MutableGroup, EditError> {
    let mut mutable = MutableGroup::new_bytes(filename.as_bytes().to_vec());
    if let Some(maker) = group.maker_bytes() {
        mutable.set_maker_bytes(maker);
    }
    let entries = group
        .entries()
        .map_err(|error| EditError::Read(error.to_string()))?;
    for index in order {
        let Some(entry) = entries.get(*index) else {
            continue;
        };
        let data = group
            .read_entry_bytes_exact(entry)
            .map_err(|error| EditError::Read(error.to_string()))?;
        let result = if entry.is_directory {
            mutable.add_packed_child_bytes_with_metadata(
                entry.name_bytes.clone(),
                data,
                entry.stored_crc,
                entry.time,
                entry.executable,
            )
        } else {
            mutable.add_file_bytes_with_metadata(
                entry.name_bytes.clone(),
                data,
                entry.time,
                entry.executable,
            )
        };
        result.map_err(|error| EditError::Write(error.to_string()))?;
    }
    Ok(mutable)
}

/// Repacks `mutable` over `path`.
pub fn write_back(mutable: &MutableGroup, path: &std::path::Path) -> Result<(), EditError> {
    let packed = mutable
        .pack()
        .map_err(|error| EditError::Write(error.to_string()))?;
    std::fs::write(path, packed).map_err(|error| EditError::Write(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_resources::compress_c4group_image;

    const GROUP_HEADER_SIZE: usize = 204;
    /// `C4GroupHeader::Creation` (`C4Group.h:87-95`), the only field whose value
    /// a rewrite cannot carry over from the group it read.
    const CREATION_FIELD: std::ops::Range<usize> = 104..108;
    /// A creation stamp no rebuild can produce, so the fixture and its rebuild
    /// always disagree there whatever second the test runs in.
    const FIXTURE_CREATION: i32 = 0x0bad_f00d;

    /// `MemScramble` (`C4Group.cpp:529-542`), which is its own inverse.
    fn mem_scramble(buffer: &mut [u8]) {
        buffer.iter_mut().for_each(|byte| *byte ^= 237);
        for index in (0..buffer.len().saturating_sub(2)).step_by(3) {
            buffer.swap(index, index + 2);
        }
    }

    /// A packed image carrying `creation`. The header is stored scrambled, so
    /// the field is reached by unscrambling it and scrambling it back.
    fn with_creation_stamp(image: &[u8], creation: i32) -> Vec<u8> {
        let mut image = image.to_vec();
        mem_scramble(&mut image[..GROUP_HEADER_SIZE]);
        image[CREATION_FIELD].copy_from_slice(&creation.to_le_bytes());
        mem_scramble(&mut image[..GROUP_HEADER_SIZE]);
        image
    }

    fn creation_stamp(image: &[u8]) -> i32 {
        let mut header = [0_u8; GROUP_HEADER_SIZE];
        header.copy_from_slice(&image[..GROUP_HEADER_SIZE]);
        mem_scramble(&mut header);
        i32::from_le_bytes(header[CREATION_FIELD].try_into().expect("creation stamp"))
    }

    /// Returns the fixture's path alongside the uncompressed image written to
    /// it, which is what a rebuild has to reproduce.
    fn fixture(directory: &std::path::Path) -> (std::path::PathBuf, Vec<u8>) {
        let mut child = MutableGroup::new("Child.c4g");
        child
            .add_file("Inner.txt", b"inner".to_vec())
            .expect("add inner");
        let mut group = MutableGroup::new("Fixture.c4g");
        group.set_maker("Round Trip");
        group
            .add_file("Alpha.txt", b"alpha".to_vec())
            .expect("add alpha");
        group
            .add_child("Child.c4g", child)
            .expect("add child group");
        let image = with_creation_stamp(&group.pack_raw().expect("pack"), FIXTURE_CREATION);
        let path = directory.join("Fixture.c4g");
        std::fs::write(&path, compress_c4group_image(&image).expect("compress")).expect("write");
        (path, image)
    }

    // A rebuild that changes nothing must reproduce every entry core and every
    // payload byte, or a mutating command would silently rewrite unrelated
    // entries. The creation stamp is the one exception: `C4Group::Close` sets
    // `Head.Creation` to the current time on every save (`C4Group.cpp:937-939`),
    // so comparing it would only pin which second the test ran in.
    #[test]
    fn an_untouched_rebuild_changes_nothing_but_the_creation_stamp() {
        let directory = tempfile::tempdir().expect("temp dir");
        let (path, original) = fixture(directory.path());

        let group = Group::open(&path).expect("open");
        let mutable = to_mutable(&group, "Fixture.c4g").expect("rebuild");
        drop(group);
        write_back(&mutable, &path).expect("write back");
        let rebuilt = mutable.pack_raw().expect("repack");

        assert_ne!(
            creation_stamp(&rebuilt),
            FIXTURE_CREATION,
            "a save stamps the current time rather than carrying the old one over"
        );
        assert_eq!(
            with_creation_stamp(&rebuilt, 0),
            with_creation_stamp(&original, 0),
            "an untouched rebuild must not alter any other packed byte"
        );
    }

    // Deleting one entry leaves the rest — including the nested group — intact.
    #[test]
    fn delete_preserves_the_remaining_entries() {
        let directory = tempfile::tempdir().expect("temp dir");
        let (path, _) = fixture(directory.path());

        let group = Group::open(&path).expect("open");
        let mut mutable = to_mutable(&group, "Fixture.c4g").expect("rebuild");
        drop(group);
        assert!(mutable.remove_entry("Alpha.txt"));
        write_back(&mutable, &path).expect("write back");

        let group = Group::open(&path).expect("reopen");
        let names: Vec<String> = group
            .entries()
            .expect("entries")
            .iter()
            .map(|entry| String::from_utf8_lossy(&entry.name_bytes).into_owned())
            .collect();
        assert_eq!(names, vec!["Child.c4g".to_owned()]);
        assert_eq!(group.maker(), Some("Round Trip"));
    }

    // C4Group.cpp:2290-2340 — rank descending, then case-insensitive name.
    #[test]
    fn sort_ranks_and_orders_like_the_native_bubble_sort() {
        // Earlier segments rank higher, so they sort first.
        assert_eq!(sort_rank("Scenario.txt", "Scenario.txt|*.png"), 2);
        assert_eq!(sort_rank("Title.png", "Scenario.txt|*.png"), 1);
        assert_eq!(sort_rank("Other.dat", "Scenario.txt|*.png"), 0);

        let names: Vec<String> = ["b.png", "Scenario.txt", "a.png", "zz.dat", "AA.dat"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        let order = sorted_entry_order(&names, "Scenario.txt|*.png");
        let sorted: Vec<&str> = order.iter().map(|index| names[*index].as_str()).collect();
        assert_eq!(
            sorted,
            // rank 2 first, then the rank-1 pngs alphabetically, then the
            // unranked rest alphabetically and case-insensitively.
            vec!["Scenario.txt", "a.png", "b.png", "AA.dat", "zz.dat"]
        );
    }

    // Renaming keeps the payload and leaves other entries alone.
    #[test]
    fn rename_keeps_the_payload() {
        let directory = tempfile::tempdir().expect("temp dir");
        let (path, _) = fixture(directory.path());

        let group = Group::open(&path).expect("open");
        let mut mutable = to_mutable(&group, "Fixture.c4g").expect("rebuild");
        drop(group);
        assert!(mutable.rename_entry("Alpha.txt", "Renamed.txt"));
        write_back(&mutable, &path).expect("write back");

        let group = Group::open(&path).expect("reopen");
        assert_eq!(
            group.read_file("Renamed.txt").expect("read renamed"),
            b"alpha"
        );
        assert!(!group.exists("Alpha.txt"));
    }
}
