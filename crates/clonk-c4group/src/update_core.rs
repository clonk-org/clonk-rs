//! `AutoUpdate.txt` — a `.c4u` package's `[Update]` core.
//!
//! `C4UpdatePackageCore::CompileFunc` (`C4Update.cpp`) writes these keys in
//! this order, each omitted when it equals its default:
//!
//! ```ini
//! [Update]
//! RequireVersion=...      ; array, omitted when all zero
//! RequireOSVersion=...    ; omitted when unset
//! Name=...
//! DestPath=...
//! GrpUpdate=0
//! TargetCount=0           ; serialised from `UpGrpCnt`, note the rename
//! AllowMissingTarget=0
//! GrpChks1=...            ; array
//! GrpChks2=0
//! GrpContentsCRC1=...     ; array
//! GrpContentsCRC2=0
//! ```
//!
//! Two things a port must get right, both established by generating real
//! packages with the pinned oracle's own `c4group`:
//!
//! - **`TargetCount` is `UpGrpCnt`.** The key and the member have different
//!   names; reading `UpGrpCnt` back from a key called `UpGrpCnt` finds nothing
//!   and silently yields a zero-target package.
//! - **`GrpContentsCRC1` is uninitialised in real C++ output.**
//!   `C4UpdatePackageCore`'s constructor initialises `GrpChks1` but not
//!   `GrpContentsCRC1`/`GrpContentsCRC2`, while `CompileFunc` serialises all
//!   fifty array slots — so a genuine package carries fifty words of stack
//!   garbage. `Check` then compares against them and only works because it
//!   falls through to its `GrpChks1` comparison. Reading must tolerate that;
//!   writing must not reproduce it, which is also why C++ `-g` output is not
//!   byte-reproducible between runs.
//!
//! Only the first `TargetCount` entries of either array are meaningful.

/// `C4Group_GetFileCRC` (`C4Group.cpp`).
///
/// Despite the name it is a plain CRC-32 over the **whole packed file's
/// bytes**, seeded at zero — not over entry contents, and not over the group
/// structure. `MakeUpdate` stores it as `GrpChks1`/`GrpChks2`, and `Check`
/// compares a candidate target against it.
///
/// This matters for the port's scope: `GrpChks2` is the CRC of the *target
/// group*, so a port that produces the same target reproduces it by
/// construction. It is not a checksum of the update package, and never
/// constrained how the package itself is built.
pub(crate) fn group_file_crc(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    crc ^ u32::MAX
}

/// `C4Group::CalcCRC32` for one entry (`C4Group.cpp`).
///
/// The entry's CRC is **not** a CRC of its data alone: C++ computes
/// `crc32(0, data)` and then *continues* the same CRC over the entry's
/// **filename** bytes. Omitting the name gives a plausible-looking value that
/// never matches a real package.
pub(crate) fn entry_crc(name: &str, data: &[u8]) -> u32 {
    crc32_continue(crc32_continue(0, data), name.as_bytes())
}

/// `C4Group::EntryCRC32(nullptr)` (`C4Group.cpp`) — the **XOR** of every
/// entry's CRC, which `C4Group_GetFileContentsCRC` returns.
///
/// Being an XOR makes it order- and packing-independent, and that is exactly
/// why it exists: `C4UpdatePackage::Execute`'s verdict accepts a result whose
/// *contents* CRC matches `GrpContentsCRC2` **or** whose *file* CRC matches
/// `GrpChks2`. Without the contents CRC an update can only succeed by
/// reproducing the target byte for byte; with it, an equivalent repack passes.
pub(crate) fn group_contents_crc<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> u32 {
    entries
        .into_iter()
        .fold(0, |crc, (name, data)| crc ^ entry_crc(name, data))
}

fn crc32_continue(initial: u32, data: &[u8]) -> u32 {
    let mut crc = initial ^ u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    crc ^ u32::MAX
}

/// `C4UP_MaxUpGrpCnt` (`C4Update.h:25`).
pub(crate) const MAX_UPDATE_GROUP_COUNT: usize = 50;

/// The `[Update]` core.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct UpdateCore {
    pub(crate) name: String,
    pub(crate) dest_path: String,
    /// `GrpUpdate` — non-zero for a group update, which is the only kind
    /// `Check` inspects further.
    pub(crate) group_update: bool,
    pub(crate) allow_missing_target: bool,
    /// Source-group CRCs, one per supported source version. Serialised as
    /// `GrpChks1`, counted by `TargetCount`.
    pub(crate) source_checksums: Vec<u32>,
    /// `GrpChks2` — the CRC of the **target** group file, not of the package.
    pub(crate) target_checksum: u32,
    /// `GrpContentsCRC1`, aligned with `source_checksums`.
    pub(crate) source_contents_crcs: Vec<u32>,
    pub(crate) target_contents_crc: u32,
}

impl UpdateCore {
    /// `UpGrpCnt`, written as `TargetCount`.
    pub(crate) fn target_count(&self) -> usize {
        self.source_checksums.len().min(MAX_UPDATE_GROUP_COUNT)
    }

    /// Serialises the core. Arrays are written to `target_count()` entries and
    /// deliberately **not** padded to fifty with uninitialised memory.
    pub(crate) fn to_ini(&self) -> String {
        let count = self.target_count();
        let list = |values: &[u32]| {
            values
                .iter()
                .take(count)
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut out = String::from("[Update]\n");
        out.push_str(&format!("Name={}\n", self.name));
        out.push_str(&format!("DestPath={}\n", self.dest_path));
        out.push_str(&format!("GrpUpdate={}\n", u8::from(self.group_update)));
        out.push_str(&format!("TargetCount={count}\n"));
        if self.allow_missing_target {
            out.push_str("AllowMissingTarget=1\n");
        }
        out.push_str(&format!("GrpChks1={}\n", list(&self.source_checksums)));
        out.push_str(&format!("GrpChks2={}\n", self.target_checksum));
        out.push_str(&format!(
            "GrpContentsCRC1={}\n",
            list(&self.source_contents_crcs)
        ));
        out.push_str(&format!("GrpContentsCRC2={}\n", self.target_contents_crc));
        out
    }

    /// Reads a core, keeping only the first `TargetCount` array entries — which
    /// is what discards C++'s uninitialised `GrpContentsCRC1` tail.
    pub(crate) fn from_ini(raw: &str) -> Self {
        let value = |key: &str| {
            raw.lines()
                .filter_map(|line| line.split_once('='))
                .find(|(name, _)| name.trim() == key)
                .map(|(_, value)| value.trim().to_owned())
        };
        let number = |key: &str| {
            value(key)
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0)
        };
        let count = number("TargetCount") as usize;
        let list = |key: &str| {
            value(key)
                .map(|value| {
                    value
                        .split(',')
                        .filter_map(|entry| entry.trim().parse::<u32>().ok())
                        .take(count)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        Self {
            name: value("Name").unwrap_or_default(),
            dest_path: value("DestPath").unwrap_or_default(),
            group_update: number("GrpUpdate") != 0,
            allow_missing_target: number("AllowMissingTarget") != 0,
            source_checksums: list("GrpChks1"),
            target_checksum: number("GrpChks2"),
            source_contents_crcs: list("GrpContentsCRC1"),
            target_contents_crc: number("GrpContentsCRC2"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AutoUpdate.txt` from a package generated by the pinned oracle's own
    /// `build-arm64-native/c4group -g`, verbatim. The `GrpContentsCRC1` tail is
    /// the uninitialised memory described in the module docs.
    const REAL_CPP_CORE: &str = "[Update]\n\
        Name=Test Update\n\
        DestPath=g1.c4f\n\
        GrpUpdate=1\n\
        TargetCount=1\n\
        GrpChks1=1686362931\n\
        GrpChks2=1194512086\n\
        GrpContentsCRC1=178201084,1,2,0,0,2,27858744,1,3992821344,1,1863871408,\
1,2176677316,0,3992834048,1,0,0,32,0,28000648,1,32,0,27787088,1,27641280,1,\
27639808,1,1863871296,1,2176809360,1,32,0,28000648,1,32,0,14850096,1,27639808,\
1,214011808,12,1863871392,1,2176773728,1\n\
        GrpContentsCRC2=3949291798\n";

    // C4Update.cpp C4UpdatePackageCore::CompileFunc, checked against a package
    // the oracle's own c4group produced.
    #[test]
    fn update_core_reads_real_cpp_output_and_writes_without_uninitialised_tails() {
        let core = UpdateCore::from_ini(REAL_CPP_CORE);
        assert_eq!(core.name, "Test Update");
        assert_eq!(core.dest_path, "g1.c4f");
        assert!(core.group_update);
        assert!(!core.allow_missing_target);
        assert_eq!(core.target_count(), 1, "TargetCount is UpGrpCnt, renamed");
        assert_eq!(core.source_checksums, vec![1_686_362_931]);
        assert_eq!(core.target_checksum, 1_194_512_086);
        assert_eq!(core.target_contents_crc, 3_949_291_798);

        // The uninitialised tail is discarded: only TargetCount entries are
        // meaningful, and C++ itself only ever indexes `[0, UpGrpCnt)`.
        assert_eq!(
            core.source_contents_crcs,
            vec![178_201_084],
            "the other 49 words are stack garbage and must not be retained"
        );

        // Writing produces a deterministic core with no padding, so unlike C++
        // the same inputs give the same bytes every time.
        let written = core.to_ini();
        assert_eq!(
            written,
            "[Update]\n\
             Name=Test Update\n\
             DestPath=g1.c4f\n\
             GrpUpdate=1\n\
             TargetCount=1\n\
             GrpChks1=1686362931\n\
             GrpChks2=1194512086\n\
             GrpContentsCRC1=178201084\n\
             GrpContentsCRC2=3949291798\n"
        );
        assert_eq!(written, core.to_ini(), "writing is reproducible");
        assert_eq!(UpdateCore::from_ini(&written), core, "and round-trips");

        // A multi-source package keeps its arrays aligned and counted.
        let multi = UpdateCore {
            name: "Multi".into(),
            dest_path: "game.c4f".into(),
            group_update: true,
            allow_missing_target: true,
            source_checksums: vec![10, 20, 30],
            target_checksum: 99,
            source_contents_crcs: vec![11, 21, 31],
            target_contents_crc: 98,
        };
        let raw = multi.to_ini();
        assert!(raw.contains("TargetCount=3"));
        assert!(raw.contains("GrpChks1=10,20,30"));
        assert!(raw.contains("GrpContentsCRC1=11,21,31"));
        assert!(
            raw.contains("AllowMissingTarget=1"),
            "written only when set, like every other defaulted key"
        );
        assert_eq!(UpdateCore::from_ini(&raw), multi);

        // C4Group_GetFileCRC is a plain CRC-32 of the whole file. Checked
        // against the two groups the fixture package was generated from: the
        // oracle wrote GrpChks1=1686362931 for g1.c4f and GrpChks2=1194512086
        // for g2.c4f, and those are exactly their file CRCs.
        // C4Group::EntryCRC32 XORs entry CRCs, and each entry's CRC continues
        // over its filename. Checked against the fixture: the oracle wrote
        // GrpContentsCRC2=3949291798 for g2.c4f's three entries.
        assert_eq!(
            group_contents_crc([
                ("a.txt", b"alpha two\n".as_slice()),
                ("added.txt", b"new file\n".as_slice()),
                ("keep.txt", b"shared\n".as_slice()),
            ]),
            3_949_291_798
        );
        // Order-independent, which is what lets an equivalent repack pass.
        assert_eq!(
            group_contents_crc([
                ("keep.txt", b"shared\n".as_slice()),
                ("a.txt", b"alpha two\n".as_slice()),
                ("added.txt", b"new file\n".as_slice()),
            ]),
            3_949_291_798
        );
        // Dropping the filename from the entry CRC gives a different, wrong
        // value — the trap this pins.
        assert_ne!(
            entry_crc("a.txt", b"alpha two\n"),
            group_file_crc(b"alpha two\n")
        );

        assert_eq!(group_file_crc(b""), 0);
        assert_eq!(
            group_file_crc(b"123456789"),
            0xCBF4_3926,
            "CRC-32 check value"
        );
        // Seeded at zero and reflected, so an incremental split matches.
        assert_eq!(group_file_crc(b"hello world"), 0x0D4A_1185);

        // An absent or empty core reads as the all-defaults package rather than
        // failing, matching StdCompiler's defaulted values.
        let empty = UpdateCore::from_ini("[Update]\n");
        assert_eq!(empty, UpdateCore::default());
        assert_eq!(empty.target_count(), 0);
    }
}
