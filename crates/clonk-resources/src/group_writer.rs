//! Mutable, C++-faithful foundation for writing stock C4Group files.

use std::borrow::Cow;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
const MAX_ENTRY_NAME_BYTES: usize = 255;
#[cfg(windows)]
const MAX_ENTRY_NAME_BYTES: usize = 256;
#[cfg(not(any(unix, windows)))]
const MAX_ENTRY_NAME_BYTES: usize = 255;
const GROUP_HEADER_SIZE: usize = 204;
const GROUP_ENTRY_SIZE: usize = 316;
const GROUP_FILE_ID: &[u8] = b"RedWolf Design GrpFolder";
const GROUP_MAKER_MAX_BYTES: usize = 30;
const GROUP_MAKER_FIELD_BYTES: usize = 32;
const C4FLS_SYSTEM: &str = "*.hlp|*.cnt|Language*.txt|*.fon|*.fnt|*.ttf|*.ttc|*.fot|*.otf|Fonts.txt|Alchem.c|StringTbl*.txt|*.c|Names.txt";
const C4FLS_MOUSE: &str =
    "*.txt|*.rtf|Title.bmp|Title.png|Icon.bmp|Tutorial01.c4s|Tutorial02.c4s|Tutorial03.c4s|Objects.c4d";
const C4FLS_KEYBOARD: &str = "*.txt|*.rtf|Title.bmp|Title.png|Icon.bmp|Tutorial01.c4s|Tutorial02.c4s|Tutorial03.c4s|Tutorial04.c4s|Tutorial05.c4s|Tutorial06.c4s|Tutorial07.c4s|Tutorial08.c4s|Tutorial09.c4s|Tutorial10.c4s";
const C4FLS_EASY: &str = "*.txt|*.rtf|Title.bmp|Title.png|Icon.bmp|Goldmine.c4s|Monsterkill.c4s|Economy.c4s|Melee.c4s|Lake.c4s|Castle.c4s";
const C4FLS_MATERIAL: &str = "TexMap.txt|*.bmp|*.png|*.c4m";
const C4FLS_GRAPHICS: &str = concat!(
    "Loader*.bmp|Loader*.png|Loader*.jpeg|Loader*.jpg|FontEndeavour12.png|FontEndeavour24.png|FontEndeavour16.png|FontEndeavour10.png|Font*.png",
    "|*.pal|Control.png|Fire.png|Background.png|Flag.png|Crew.png|Score.png|Wealth.png|Player.png|Rank.png|Entry.png|Captain.png|Cursor.png|CursorSmall.png|CursorMedium.png|CursorLarge.png|CursorXLarge.png|CursorXXLarge.png|CursorXXXLarge.png|CursorXXXXLarge.png|CursorXXXXXLarge.png|SelectMark.png|MenuSymbol.png|Menu.png|Logo.png|Construction.png|Energy.png|Magic.png|Options.png|UpperBoard.png|Arrow.png|Exit.png|Hand.png|Gamepad.png|Build.png|EnergyBars.png|Liquid.png",
    "|GUICaption.png|GUIButton.png|GUIButtonDown.png|GUIButtonHighlight.png|GUIIcons.png|GUIIcons2.png|GUIScroll.png|GUIContext.png|GUISubmenu.png|GUICheckBox.png|GUIBigArrows.png|GUIProgress.png",
    "|StartupScenSelBG.*|StartupPlrSelBG.*|StartupPlrPropBG.*|StartupNetworkBG.*|StartupAboutBG.*|StartupBigButton.png|StartupBigButtonDown.png|StartupBookScroll.png|StartupContext.png|StartupScenSelIcons.png|StartupScenSelTitleOv.png|StartupPlrCtrlType.png|StartupDlgPaper.png|StartupOptionIcons.png|StartupTabClip.png|StartupNetGetRef.png",
);
const C4FLS_FOLDER: &str = "Folder.txt|Title*.txt|Info.txt|Desc*.rtf|Title.png|Title.bmp|Icon.png|Icon.bmp|Author.txt|Version.txt|*.c4s|*.c4f|Loader*.bmp|Loader*.png|Loader*.jpeg|Loader*.jpg|FolderMap.txt|FolderMap.png|*.png";
const C4FLS_WESTERN: &str = concat!(
    "Folder.txt|Title*.txt|Info.txt|Desc*.rtf|Title.png|Title.bmp|Icon.png|Icon.bmp|Author.txt|Version.txt|*.c4s|*.c4f|Loader*.bmp|Loader*.png|Loader*.jpeg|Loader*.jpg|FolderMap.txt|FolderMap.png|*.png",
    "|ScenGCBase.png|ScenGC.png|ScenDMVBase.png|ScenDMV.png|ScenFSBase.png|ScenFS.png|ScenCTFBase.png|ScenCTF.png|ScenLHBase.png|ScenLH.png|ScenMCBase.png|ScenMC.png|ScenMWBase.png|ScenMW.png|ScenBRBase.png|ScenBR.png|ScenTHBase.png|ScenTH.png|ScenGRBase.png|ScenGR.png|ScenSTSBase.png|ScenSTS.png|ScenNWBase.png|ScenNW.png|AccLH.png|AccFS.png|AccGC.png|AccGR.png|AccMW.png|AccNW.png",
);
const C4FLS_DEFINITION: &str = "Particle.txt|DefCore.txt|Graphics.bmp|Graphics.png|Overlay.png|Graphics*.png|Overlay*.png|Portrait*.png|Portrait*.bmp|ActMap.txt|Script.c|Script*.c|C4Script.c|StringTbl*.txt|Names*.txt|Title*.txt|ClonkNames.txt|Rank*.txt|Rank.bmp|Rank.png|Desc*.txt|Overlay.png|Title.bmp|Title.png|Icon.bmp|Author.txt|Version.txt|*.wav|*.ogg|*.mp3|*.c4d";
const C4FLS_PLAYER: &str = "Player.txt|Portrait.png|Portrait.bmp|*.c4i";
const C4FLS_OBJECT: &str = "ObjectInfo.txt|Portrait.png|Portrait.bmp";
const C4FLS_SCENARIO: &str = "Loader*.bmp|Loader*.png|Loader*.jpeg|Loader*.jpg|Fonts.txt|Scenario.txt|Title*.txt|Info.txt|Desc*.rtf|Icon.png|Icon.bmp|Game.txt|StringTbl*.txt|Teams.txt|Parameters.txt|Info.txt|Sect*.c4g|Music.c4g|*.mid|*.wav|Desc*.rtf|Title.bmp|Title.png|*.c4d|Material.c4g|MatMap.txt|Landscape.bmp|Landscape.png|DiffLandscape.bmp|Sky.bmp|Sky.png|Sky.jpeg|Sky.jpg|PXS.c4b|MassMover.c4b|CtrlRec.c4b|Strings.txt|Objects.txt|RoundResults.txt|Author.txt|Version.txt|Names.txt|*.c4d|Script.c|Script*.c|System.c4g";
const C4FLS_SECTION: &str = "Scenario.txt|Game.txt|Landscape.bmp|Landscape.png|Sky.bmp|Sky.png|Sky.jpeg|Sky.jpg|PXS.c4b|MassMover.c4b|CtrlRec.c4b|Strings.txt|Objects.txt";
const C4FLS_MUSIC: &str = "Frontend.*|Credits.*";
const C4CFN_FLS: &[(&str, &str)] = &[
    ("System.c4g", C4FLS_SYSTEM),
    ("Mouse.c4f", C4FLS_MOUSE),
    ("Keyboard.c4f", C4FLS_KEYBOARD),
    ("Easy.c4f", C4FLS_EASY),
    ("Material.c4g", C4FLS_MATERIAL),
    ("Graphics.c4g", C4FLS_GRAPHICS),
    ("Western.c4f", C4FLS_WESTERN),
    ("*.c4d", C4FLS_DEFINITION),
    ("*.c4p", C4FLS_PLAYER),
    ("*.c4i", C4FLS_OBJECT),
    ("*.c4s", C4FLS_SCENARIO),
    ("*.c4f", C4FLS_FOLDER),
    ("Sect*.c4g", C4FLS_SECTION),
    ("Music.c4g", C4FLS_MUSIC),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutableGroupError {
    SourceGroup(String),
    EmptyEntryName,
    EntryNameContainsNul,
    EntryNameTooLong(usize),
    EntryAlreadyExists(String),
    TooManyEntries(usize),
    EntryDataTooLarge(usize),
    GroupDataTooLarge,
    CompressionFailed(String),
}

pub(crate) struct ImportedPackedChildCoreMetadata {
    pub(crate) crc_state: u8,
    pub(crate) stored_crc: u32,
    pub(crate) child_contents_crc: Option<u32>,
    pub(crate) time: u32,
    pub(crate) executable: bool,
}

impl fmt::Display for MutableGroupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceGroup(message) => {
                write!(formatter, "failed to read source C4Group: {message}")
            }
            Self::EmptyEntryName => formatter.write_str("C4Group entry name is empty"),
            Self::EntryNameContainsNul => {
                formatter.write_str("C4Group entry name contains a NUL byte")
            }
            Self::EntryNameTooLong(length) => write!(
                formatter,
                "C4Group entry name has {length} bytes; maximum is {MAX_ENTRY_NAME_BYTES}"
            ),
            Self::EntryAlreadyExists(name) => {
                write!(formatter, "C4Group entry already exists: {name}")
            }
            Self::TooManyEntries(count) => {
                write!(formatter, "C4Group has {count} entries; maximum is int32")
            }
            Self::EntryDataTooLarge(size) => {
                write!(
                    formatter,
                    "C4Group entry has {size} bytes; maximum is int32"
                )
            }
            Self::GroupDataTooLarge => formatter.write_str("C4Group entry offsets exceed int32"),
            Self::CompressionFailed(message) => {
                write!(formatter, "C4Group gzip compression failed: {message}")
            }
        }
    }
}

impl std::error::Error for MutableGroupError {}

#[derive(Debug)]
pub enum MutableGroupChildMut<'a> {
    Missing,
    File,
    Child(&'a mut MutableGroup),
}

/// Non-mutating classification of an entry in a writable group image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutableGroupEntryKind {
    File,
    ChildGroup,
    /// The source core marks this entry as a child group, but its complete
    /// payload cannot be opened as one. C4Group::OpenAsChild fails for it.
    UnopenableChildGroup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutableGroup {
    pub(crate) filename: Vec<u8>,
    rewrite_header_template: Option<[u8; GROUP_HEADER_SIZE]>,
    maker: [u8; GROUP_MAKER_FIELD_BYTES],
    original: i32,
    pub(crate) entries: Vec<MutableGroupEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutableGroupEntry {
    name: String,
    pub(crate) name_bytes: Vec<u8>,
    pub(crate) data: MutableGroupEntryData,
    time: u32,
    executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MutableGroupEntryData {
    File(Vec<u8>),
    ExistingFile {
        data: Vec<u8>,
        crc_state: u8,
        stored_crc: u32,
    },
    Child(Box<MutableGroup>),
    PackedChild {
        data: Vec<u8>,
        crc_state: u8,
        stored_crc: u32,
        child_contents_crc: Option<u32>,
    },
}

impl MutableGroup {
    /// Creates a group whose logical filename selects the stock C4CFN_FLS sort list.
    pub fn new(filename: impl Into<String>) -> Self {
        Self::new_bytes(filename.into().into_bytes())
    }

    /// Creates a group with an exact legacy byte-string filename. The logical
    /// filename selects the stock C4CFN_FLS sort list.
    pub fn new_bytes(filename: impl Into<Vec<u8>>) -> Self {
        let mut maker = [0; GROUP_MAKER_FIELD_BYTES];
        maker[..b"New C4Group".len()].copy_from_slice(b"New C4Group");
        Self {
            filename: filename.into(),
            rewrite_header_template: None,
            maker,
            original: 0,
            entries: Vec::new(),
        }
    }

    pub fn add_file(
        &mut self,
        name: impl Into<String>,
        data: Vec<u8>,
    ) -> Result<(), MutableGroupError> {
        self.add_entry(
            name.into(),
            MutableGroupEntryData::File(data),
            unix_time_now(),
            false,
        )
    }

    /// Byte-preserving form of [`Self::add_file`] for legacy entry names that
    /// are not valid UTF-8.
    pub fn add_file_bytes(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
    ) -> Result<(), MutableGroupError> {
        self.add_entry_bytes(
            name.into(),
            MutableGroupEntryData::File(data),
            unix_time_now(),
            false,
        )
    }

    pub fn add_file_with_metadata(
        &mut self,
        name: impl Into<String>,
        data: Vec<u8>,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_entry(
            name.into(),
            MutableGroupEntryData::File(data),
            entry_time_or_now(time),
            executable,
        )
    }

    pub fn add_file_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_entry_bytes(
            name.into(),
            MutableGroupEntryData::File(data),
            entry_time_or_now(time),
            executable,
        )
    }

    /// Adds bytes read from an on-disk file while retaining the literal
    /// metadata supplied by C++'s `AddEntryOnDisk` path, including a zero
    /// timestamp that the regular mutable-group metadata helpers normalize.
    pub fn add_disk_file_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_entry_bytes(
            name.into(),
            MutableGroupEntryData::File(data),
            time,
            executable,
        )
    }

    /// Imports an existing file whose supplied CRC is already a trusted
    /// `C4GECS_New` entry checksum. The otherwise-special zero timestamp is
    /// retained verbatim.
    pub fn add_existing_file_with_metadata(
        &mut self,
        name: impl Into<String>,
        data: Vec<u8>,
        contents_crc: u32,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_existing_file_bytes_with_metadata(
            name.into().into_bytes(),
            data,
            contents_crc,
            time,
            executable,
        )
    }

    pub fn add_existing_file_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
        contents_crc: u32,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_imported_file_core_bytes_with_metadata(
            name,
            data,
            2,
            contents_crc,
            time,
            executable,
        )
    }

    /// Imports the raw CRC state from an existing ordinary-file core. C++
    /// retains `C4GECS_New` verbatim, while `None`/`Old` are resolved only
    /// when the group closes, after any intervening rename.
    pub(crate) fn add_imported_file_core_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
        crc_state: u8,
        stored_crc: u32,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_entry_bytes(
            name.into(),
            MutableGroupEntryData::ExistingFile {
                data,
                crc_state,
                stored_crc,
            },
            time,
            executable,
        )
    }

    pub fn add_child(
        &mut self,
        name: impl Into<String>,
        mut child: MutableGroup,
    ) -> Result<(), MutableGroupError> {
        let name = name.into();
        child.filename = name.as_bytes().to_vec();
        self.add_entry(
            name,
            MutableGroupEntryData::Child(Box::new(child)),
            unix_time_now(),
            false,
        )
    }

    pub fn add_child_bytes(
        &mut self,
        name: impl Into<Vec<u8>>,
        mut child: MutableGroup,
    ) -> Result<(), MutableGroupError> {
        let name = name.into();
        child.filename = name.clone();
        self.add_entry_bytes(
            name,
            MutableGroupEntryData::Child(Box::new(child)),
            unix_time_now(),
            false,
        )
    }

    pub fn add_child_with_metadata(
        &mut self,
        name: impl Into<String>,
        mut child: MutableGroup,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        let name = name.into();
        child.filename = name.as_bytes().to_vec();
        self.add_entry(
            name,
            MutableGroupEntryData::Child(Box::new(child)),
            entry_time_or_now(time),
            executable,
        )
    }

    pub fn add_child_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        mut child: MutableGroup,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        let name = name.into();
        child.filename = name.clone();
        self.add_entry_bytes(
            name,
            MutableGroupEntryData::Child(Box::new(child)),
            entry_time_or_now(time),
            executable,
        )
    }

    /// Imports an existing child core without treating timestamp zero as the
    /// sentinel used by C4Group's public Add overloads.
    pub(crate) fn add_existing_child_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        mut child: MutableGroup,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        let name = name.into();
        child.filename = name.clone();
        self.add_entry_bytes(
            name,
            MutableGroupEntryData::Child(Box::new(child)),
            time,
            executable,
        )
    }

    /// Imports a raw uncompressed child image with its trusted, already-new
    /// contents CRC. C4Group copies such unchanged payloads as opaque bytes
    /// when rewriting their parent (`C4Group::AppendEntry2StdFile`).
    pub fn add_packed_child_with_metadata(
        &mut self,
        name: impl Into<String>,
        data: Vec<u8>,
        contents_crc: u32,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_packed_child_bytes_with_metadata(
            name.into().into_bytes(),
            data,
            contents_crc,
            time,
            executable,
        )
    }

    /// Imports a freshly moved standalone group using the timestamp and
    /// executable defaults of `C4Group::AddEntryOnDisk`.
    pub fn add_packed_child_bytes(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
        contents_crc: u32,
    ) -> Result<(), MutableGroupError> {
        self.add_packed_child_bytes_with_metadata(name, data, contents_crc, unix_time_now(), false)
    }

    pub fn add_packed_child_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
        contents_crc: u32,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_entry_bytes(
            name.into(),
            MutableGroupEntryData::PackedChild {
                data,
                crc_state: 2,
                stored_crc: contents_crc,
                child_contents_crc: Some(contents_crc),
            },
            time,
            executable,
        )
    }

    /// Imports a packed child's original CRC core plus the result of the
    /// Close-time calculation, if the complete child image can be opened.
    /// Keeping both lets the ordered CRC pass retain this core when an earlier
    /// entry fails, exactly like `C4Group::EntryCRC32`.
    pub(crate) fn add_imported_packed_child_core_bytes_with_metadata(
        &mut self,
        name: impl Into<Vec<u8>>,
        data: Vec<u8>,
        metadata: ImportedPackedChildCoreMetadata,
    ) -> Result<(), MutableGroupError> {
        self.add_entry_bytes(
            name.into(),
            MutableGroupEntryData::PackedChild {
                data,
                crc_state: metadata.crc_state,
                stored_crc: metadata.stored_crc,
                child_contents_crc: metadata.child_contents_crc,
            },
            metadata.time,
            metadata.executable,
        )
    }

    pub fn set_maker(&mut self, maker: &str) {
        self.set_maker_bytes(maker.as_bytes());
    }

    pub fn set_maker_bytes(&mut self, bytes: &[u8]) {
        let length = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len())
            .min(GROUP_MAKER_MAX_BYTES);
        self.maker[0] = 0;
        self.maker[..length].copy_from_slice(&bytes[..length]);
        self.maker[length] = 0;
    }

    /// Apply C4Group's process-global maker to a group created from a physical
    /// directory and every recursively materialized directory child. Opaque
    /// packed children retain their original headers because native does not
    /// reopen them while packing the parent.
    pub fn set_maker_bytes_recursively(&mut self, bytes: &[u8]) {
        self.set_maker_bytes(bytes);
        for entry in &mut self.entries {
            if let MutableGroupEntryData::Child(child) = &mut entry.data {
                child.set_maker_bytes_recursively(bytes);
            }
        }
    }

    pub fn set_maker_field(&mut self, field: &[u8; GROUP_MAKER_FIELD_BYTES]) {
        self.maker = *field;
    }

    /// The NUL-terminated maker body [`MutableGroup::pack`] will write, which is
    /// what [`crate::Group::maker_bytes`] reads back and what a network resource
    /// core serializes (`C4Network2Res::SetByGroup`). Callers that stamp the
    /// process maker conditionally need this rather than their own input: a
    /// group created here carries the native `New C4Group` default until it is
    /// overwritten.
    pub fn maker_bytes(&self) -> &[u8] {
        let length = self
            .maker
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.maker.len());
        &self.maker[..length]
    }

    pub fn set_rewrite_header_template(&mut self, header: &[u8; GROUP_HEADER_SIZE]) {
        self.rewrite_header_template = Some(*header);
        self.maker.copy_from_slice(&header[40..72]);
    }

    pub fn make_original(&mut self, original: bool) {
        self.original = if original { 1_234_567 } else { 0 };
    }

    pub fn entry_crc(&self, name: &str) -> Option<u32> {
        let entries = self.ordered_entries();
        entries
            .iter()
            .find(|entry| entry.name_bytes.eq_ignore_ascii_case(name.as_bytes()))
            .map(|entry| entry.calculated_crc(&entries).unwrap_or(0))
    }

    pub fn contents_crc(&self) -> u32 {
        let entries = self.ordered_entries();
        entries
            .iter()
            .try_fold(0, |crc, entry| {
                entry
                    .calculated_crc(&entries)
                    .map(|entry_crc| crc ^ entry_crc)
            })
            .unwrap_or(0)
    }

    pub fn pack_raw(&self) -> Result<Vec<u8>, MutableGroupError> {
        let creation = unix_time_now() as i32;
        let entries = self.ordered_entries();
        let entry_count = i32::try_from(entries.len())
            .map_err(|_| MutableGroupError::TooManyEntries(entries.len()))?;
        let packed_entries = entries
            .iter()
            .map(|entry| PackedEntry::from_entry(entry))
            .collect::<Result<Vec<_>, MutableGroupError>>()?;
        let crc_cores = close_crc_cores(&entries);

        let mut offset = 0_i32;
        let entry_cores = packed_entries
            .iter()
            .zip(&entries)
            .zip(&crc_cores)
            .map(|((packed, entry), crc_core)| {
                let core = encode_entry_core(entry, packed, *crc_core, offset)?;
                offset = offset
                    .checked_add(packed.size)
                    .ok_or(MutableGroupError::GroupDataTooLarge)?;
                Ok(core)
            })
            .collect::<Result<Vec<_>, MutableGroupError>>()?;

        let payload_size = packed_entries.iter().try_fold(0_usize, |size, entry| {
            size.checked_add(entry.data.len())
                .ok_or(MutableGroupError::GroupDataTooLarge)
        })?;
        let mut image = Vec::with_capacity(
            GROUP_HEADER_SIZE
                .checked_add(
                    GROUP_ENTRY_SIZE
                        .checked_mul(entries.len())
                        .ok_or(MutableGroupError::GroupDataTooLarge)?,
                )
                .and_then(|size| size.checked_add(payload_size))
                .ok_or(MutableGroupError::GroupDataTooLarge)?,
        );
        image.extend_from_slice(&encode_header(
            self.rewrite_header_template.as_ref(),
            &self.maker,
            creation,
            self.original,
            entry_count,
        ));
        entry_cores
            .iter()
            .for_each(|core| image.extend_from_slice(core));
        packed_entries
            .iter()
            .for_each(|entry| image.extend_from_slice(&entry.data));
        Ok(image)
    }

    pub fn pack(&self) -> Result<Vec<u8>, MutableGroupError> {
        let image = self.pack_raw()?;
        compress_c4group_image(&image)
    }

    pub fn entry_names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    /// Classifies an entry without opening or marking an imported packed
    /// child as rewritten.
    pub fn entry_kind(&self, name: &str) -> Option<MutableGroupEntryKind> {
        self.entries
            .iter()
            .find(|entry| entry.name_bytes.eq_ignore_ascii_case(name.as_bytes()))
            .map(|entry| match &entry.data {
                MutableGroupEntryData::File(_) | MutableGroupEntryData::ExistingFile { .. } => {
                    MutableGroupEntryKind::File
                }
                MutableGroupEntryData::Child(_) => MutableGroupEntryKind::ChildGroup,
                MutableGroupEntryData::PackedChild {
                    child_contents_crc: Some(_),
                    ..
                } => MutableGroupEntryKind::ChildGroup,
                MutableGroupEntryData::PackedChild {
                    child_contents_crc: None,
                    ..
                } => MutableGroupEntryKind::UnopenableChildGroup,
            })
    }

    /// Deletes every entry with the same ASCII-case-insensitive name, like
    /// C4Group::DeleteEntry during a scenario rewrite.
    pub fn remove_entry(&mut self, name: &str) -> bool {
        self.remove_entry_bytes(name.as_bytes())
    }

    /// Byte-preserving form of [`Self::remove_entry`] for legacy C4Group
    /// filenames that are not valid UTF-8.
    pub fn remove_entry_bytes(&mut self, name: &[u8]) -> bool {
        let previous = self.entries.len();
        self.entries
            .retain(|entry| !entry.name_bytes.eq_ignore_ascii_case(name));
        self.entries.len() != previous
    }

    /// Renames one entry with C4Group's ASCII-case-insensitive lookup rules.
    /// A missing source, invalid name, or distinct entry already occupying
    /// `new` leaves the group unchanged and returns `false`.
    pub fn rename_entry(&mut self, old: &str, new: &str) -> bool {
        let old_name_bytes = old.as_bytes();
        let new_name_bytes = new.as_bytes();
        if old_name_bytes.is_empty()
            || new_name_bytes.is_empty()
            || old_name_bytes.contains(&0)
            || new_name_bytes.contains(&0)
        {
            return false;
        }
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.name_bytes.eq_ignore_ascii_case(old_name_bytes))
        else {
            return false;
        };
        if self.entries.iter().enumerate().any(|(candidate, entry)| {
            candidate != index && entry.name_bytes.eq_ignore_ascii_case(new_name_bytes)
        }) {
            return false;
        }

        // The startup-player caller needs C4Group's public SCopy truncation
        // semantics, while scenario mutation uses the checked API below.
        let stored_name = new_name_bytes[..new_name_bytes.len().min(MAX_ENTRY_NAME_BYTES)].to_vec();
        self.rename_entry_at(index, stored_name);
        true
    }

    /// Renames one entry in place without replacing a case-insensitive
    /// destination collision. The entry's payload, timestamp, executable bit,
    /// and position are retained; a materialized child also receives the new
    /// logical filename so its standard C4Group sort list stays correct.
    pub fn rename_entry_checked(
        &mut self,
        old_name: &str,
        new_name: &str,
    ) -> Result<bool, MutableGroupError> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.name_bytes.eq_ignore_ascii_case(old_name.as_bytes()))
        else {
            return Ok(false);
        };

        let new_name_bytes = new_name.as_bytes();
        if new_name_bytes.is_empty() {
            return Err(MutableGroupError::EmptyEntryName);
        }
        validate_entry_name(new_name_bytes)?;
        if self.entries[index].name_bytes == new_name_bytes {
            return Ok(true);
        }
        if self.entries.iter().enumerate().any(|(candidate, entry)| {
            candidate != index && entry.name_bytes.eq_ignore_ascii_case(new_name_bytes)
        }) {
            return Err(MutableGroupError::EntryAlreadyExists(new_name.to_string()));
        }

        self.rename_entry_at(index, new_name_bytes.to_vec());
        Ok(true)
    }

    fn rename_entry_at(&mut self, index: usize, new_name_bytes: Vec<u8>) {
        let entry = &mut self.entries[index];
        entry.name = String::from_utf8_lossy(&new_name_bytes).into_owned();
        entry.name_bytes = new_name_bytes;
        if let MutableGroupEntryData::Child(child) = &mut entry.data {
            child.filename = entry.name_bytes.clone();
        }
    }

    pub fn sort(&mut self, sort_list: &str) -> bool {
        if sort_list.is_empty() {
            return false;
        }
        let patterns = sort_list.split('|').collect::<Vec<_>>();
        self.entries.sort_by(|left, right| {
            let left_rank = sort_rank_bytes(&left.name_bytes, &patterns);
            let right_rank = sort_rank_bytes(&right.name_bytes, &patterns);
            right_rank.cmp(&left_rank).then_with(|| {
                left.name_bytes
                    .to_ascii_lowercase()
                    .cmp(&right.name_bytes.to_ascii_lowercase())
            })
        });
        true
    }

    /// Retargets a standalone group and applies the stock sort list selected
    /// by its destination filename. The return value reports whether native
    /// `C4Group::Sort` would mark the group modified and rewrite its header.
    pub fn resort_for_filename_bytes(&mut self, filename: impl Into<Vec<u8>>) -> bool {
        self.filename = filename.into();
        let Some(sort_list) = standard_sort_list_for_filename(&self.filename) else {
            return false;
        };
        let patterns = sort_list.split('|').collect::<Vec<_>>();
        let before = self
            .entries
            .iter()
            .map(|entry| entry.name_bytes.clone())
            .collect::<Vec<_>>();
        self.entries
            .sort_by(|left, right| entry_sort_order(left, right, &patterns));
        self.entries
            .iter()
            .map(|entry| &entry.name_bytes)
            .ne(before.iter())
    }

    fn add_entry(
        &mut self,
        name: String,
        data: MutableGroupEntryData,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        self.add_entry_bytes(name.into_bytes(), data, time, executable)
    }

    fn add_entry_bytes(
        &mut self,
        mut name_bytes: Vec<u8>,
        mut data: MutableGroupEntryData,
        time: u32,
        executable: bool,
    ) -> Result<(), MutableGroupError> {
        // C4Group::AddEntry performs its replacement lookup before SCopy
        // truncates the new core name; both operations still see a C string.
        let c_name_length = name_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_bytes.len());
        let lookup_name = &name_bytes[..c_name_length];
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.name_bytes.eq_ignore_ascii_case(lookup_name))
        {
            self.entries.remove(index);
        }
        // Child sorting uses the source name before the _MAX_FNAME copy.
        if let MutableGroupEntryData::Child(child) = &mut data {
            child.filename = lookup_name.to_vec();
        }
        name_bytes.truncate(c_name_length.min(MAX_ENTRY_NAME_BYTES));
        validate_entry_name(&name_bytes)?;
        let name = String::from_utf8_lossy(&name_bytes).into_owned();
        self.entries.push(MutableGroupEntry {
            name,
            name_bytes,
            data,
            time,
            executable,
        });
        Ok(())
    }

    fn ordered_entries(&self) -> Vec<&MutableGroupEntry> {
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        if let Some(sort_list) = standard_sort_list_for_filename(&self.filename) {
            let patterns = sort_list.split('|').collect::<Vec<_>>();
            entries.sort_by(|left, right| entry_sort_order(left, right, &patterns));
        }
        entries
    }
}

/// Wraps an uncompressed nested-group image in the stock on-disk C4Group
/// gzip envelope without rewriting its header or entries.
pub fn compress_c4group_image(image: &[u8]) -> Result<Vec<u8>, MutableGroupError> {
    let input_length =
        u32::try_from(image.len()).map_err(|_| MutableGroupError::GroupDataTooLarge)?;
    // zlib requires null allocator hooks on input and installs non-null defaults
    // during initialization. libz-sys models those C hooks as non-null function
    // pointers, so keep the zeroed bytes in MaybeUninit until zlib replaces them.
    let mut uninitialized_stream = Box::new(std::mem::MaybeUninit::<libz_sys::z_stream>::zeroed());
    let stream_pointer = uninitialized_stream.as_mut_ptr();
    // SAFETY: zlib accepts a zero-filled z_stream at this boundary. The version
    // and structure size come from the same linked libz-sys build, and Rust does
    // not materialize the invalid null function-pointer fields before the call.
    let status = unsafe {
        libz_sys::deflateInit2_(
            stream_pointer,
            9,
            libz_sys::Z_DEFLATED,
            15 + 16,
            2,
            libz_sys::Z_DEFAULT_STRATEGY,
            libz_sys::zlibVersion(),
            std::mem::size_of::<libz_sys::z_stream>() as i32,
        )
    };
    if status != libz_sys::Z_OK {
        return Err(MutableGroupError::CompressionFailed(format!(
            "deflateInit2 failed with zlib status {status}"
        )));
    }
    // SAFETY: Z_OK guarantees deflateInit2_ initialized the complete z_stream,
    // including replacing both null allocator hooks with zlib defaults. Keep it
    // at its boxed address because zlib's internal state retains this pointer.
    let stream = unsafe { &mut *stream_pointer };
    let _guard = DeflateEndGuard(stream_pointer);

    // SAFETY: the initialized stream remains live through the guard, and zlib
    // accepts the complete input length used for the following one-shot call.
    let output_bound = unsafe { libz_sys::deflateBound(stream, image.len() as _) };
    // zlib's `uLong` is 32-bit on Windows and 64-bit elsewhere, so this is a
    // widening check on one target and a no-op on the other.
    #[allow(clippy::useless_conversion)]
    let output_length =
        u32::try_from(output_bound).map_err(|_| MutableGroupError::GroupDataTooLarge)?;
    let mut compressed = vec![0_u8; output_length as usize];
    stream.next_in = image.as_ptr() as *mut libz_sys::Bytef;
    stream.avail_in = input_length;
    stream.next_out = compressed.as_mut_ptr();
    stream.avail_out = output_length;

    // SAFETY: both buffers remain allocated for the call and their lengths are
    // recorded in the z_stream fields using zlib's unsigned-int representation.
    let status = unsafe { libz_sys::deflate(stream, libz_sys::Z_FINISH) };
    if status != libz_sys::Z_STREAM_END {
        return Err(MutableGroupError::CompressionFailed(format!(
            "deflate failed with zlib status {status}"
        )));
    }
    compressed.truncate(stream.total_out as usize);
    if compressed.len() < 2 {
        return Err(MutableGroupError::CompressionFailed(
            "gzip output is missing its header".to_owned(),
        ));
    }
    compressed[..2].copy_from_slice(&[0x1e, 0x8c]);
    Ok(compressed)
}

struct DeflateEndGuard(*mut libz_sys::z_stream);

impl Drop for DeflateEndGuard {
    fn drop(&mut self) {
        // SAFETY: the guard is created only after successful deflateInit2_ and
        // owns the single corresponding deflateEnd call.
        unsafe {
            libz_sys::deflateEnd(self.0);
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn compress_c4group_for_test(image: &[u8]) -> Result<Vec<u8>, MutableGroupError> {
    compress_c4group_image(image)
}

struct PackedEntry<'a> {
    data: Cow<'a, [u8]>,
    size: i32,
    child: bool,
}

#[derive(Clone, Copy)]
struct EntryCoreCrc {
    state: u8,
    value: u32,
}

impl<'a> PackedEntry<'a> {
    fn from_entry(entry: &'a MutableGroupEntry) -> Result<Self, MutableGroupError> {
        let (data, child) = match &entry.data {
            MutableGroupEntryData::File(data) => (Cow::Borrowed(data.as_slice()), false),
            MutableGroupEntryData::ExistingFile { data, .. } => {
                (Cow::Borrowed(data.as_slice()), false)
            }
            MutableGroupEntryData::Child(child) => (Cow::Owned(child.pack_raw()?), true),
            MutableGroupEntryData::PackedChild { data, .. } => {
                (Cow::Borrowed(data.as_slice()), true)
            }
        };
        let size = i32::try_from(data.len())
            .map_err(|_| MutableGroupError::EntryDataTooLarge(data.len()))?;
        Ok(Self { data, size, child })
    }
}

fn encode_header(
    template: Option<&[u8; GROUP_HEADER_SIZE]>,
    maker: &[u8; GROUP_MAKER_FIELD_BYTES],
    creation: i32,
    original: i32,
    entry_count: i32,
) -> [u8; 204] {
    let mut header = template.copied().unwrap_or([0_u8; GROUP_HEADER_SIZE]);
    if template.is_none() {
        header[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
    }
    header[28..32].copy_from_slice(&1_i32.to_le_bytes());
    header[32..36].copy_from_slice(&2_i32.to_le_bytes());
    header[36..40].copy_from_slice(&entry_count.to_le_bytes());
    header[40..40 + GROUP_MAKER_FIELD_BYTES].copy_from_slice(maker);
    header[104..108].copy_from_slice(&creation.to_le_bytes());
    header[108..112].copy_from_slice(&original.to_le_bytes());
    mem_scramble(&mut header);
    header
}

fn encode_entry_core(
    entry: &MutableGroupEntry,
    packed: &PackedEntry<'_>,
    crc: EntryCoreCrc,
    offset: i32,
) -> Result<[u8; GROUP_ENTRY_SIZE], MutableGroupError> {
    validate_entry_name(&entry.name_bytes)?;
    let mut core = [0_u8; GROUP_ENTRY_SIZE];
    core[..entry.name_bytes.len()].copy_from_slice(&entry.name_bytes);
    core[264..268].copy_from_slice(&i32::from(packed.child).to_le_bytes());
    core[268..272].copy_from_slice(&packed.size.to_le_bytes());
    core[276..280].copy_from_slice(&offset.to_le_bytes());
    core[280..284].copy_from_slice(&entry.time.to_le_bytes());
    core[284] = crc.state;
    core[285..289].copy_from_slice(&crc.value.to_le_bytes());
    core[289] = u8::from(entry.executable);
    Ok(core)
}

/// Simulates the one ordered `EntryCRC32(nullptr)` pass performed by
/// `C4Group::Close`. A direct calculation failure leaves that entry and every
/// later entry at their pre-pass CRC state/value; earlier successes stay new.
fn close_crc_cores(entries: &[&MutableGroupEntry]) -> Vec<EntryCoreCrc> {
    let mut calculation_failed = false;
    entries
        .iter()
        .map(|entry| {
            let original = entry.original_crc_core();
            if calculation_failed || original.state == 2 {
                return original;
            }
            match entry.calculated_crc(entries) {
                Some(value) => EntryCoreCrc { state: 2, value },
                None => {
                    calculation_failed = true;
                    original
                }
            }
        })
        .collect()
}

fn mem_scramble(buffer: &mut [u8]) {
    buffer.iter_mut().for_each(|byte| *byte ^= 237);
    for index in (0..buffer.len().saturating_sub(2)).step_by(3) {
        buffer.swap(index, index + 2);
    }
}

fn sort_rank_bytes(name: &[u8], patterns: &[&str]) -> usize {
    patterns
        .iter()
        .position(|pattern| wildcard_match(pattern.as_bytes(), name))
        .map(|index| patterns.len() - index)
        .unwrap_or(0)
}

fn entry_sort_order(
    left: &MutableGroupEntry,
    right: &MutableGroupEntry,
    patterns: &[&str],
) -> std::cmp::Ordering {
    let left_rank = sort_rank_bytes(&left.name_bytes, patterns);
    let right_rank = sort_rank_bytes(&right.name_bytes, patterns);
    right_rank.cmp(&left_rank).then_with(|| {
        left.name_bytes
            .to_ascii_lowercase()
            .cmp(&right.name_bytes.to_ascii_lowercase())
    })
}

/// The stock sort list native `C4Group::Sort` selects for a group of this
/// filename, mirroring `C4GameSave::GetSortOrder` picking `C4FLS_Scenario`
/// for a saved scenario (`C4GameSave.h:63`).
pub fn standard_sort_list_for_filename(filename: &[u8]) -> Option<&'static str> {
    let filename = filename
        .rsplit(|byte| matches!(byte, b'/' | b'\\'))
        .next()
        .unwrap_or(filename);
    C4CFN_FLS
        .iter()
        .find(|(pattern, _)| wildcard_match(pattern.as_bytes(), filename))
        .map(|(_, sort_list)| *sort_list)
}

fn wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut backtrack_pattern, mut backtrack_value) = (None, None);
    while pattern_index < pattern.len() || backtrack_pattern.is_some() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            backtrack_pattern = Some(pattern_index);
            backtrack_value = Some(value_index);
        } else if value_index >= value.len() {
            break;
        } else if pattern.get(pattern_index) == Some(&b'?')
            || pattern
                .get(pattern_index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if let (Some(saved_pattern), Some(saved_value)) =
            (backtrack_pattern, backtrack_value)
        {
            let next_value = saved_value + 1;
            pattern_index = saved_pattern;
            value_index = next_value;
            backtrack_value = Some(next_value);
        } else {
            return false;
        }
    }
    pattern_index == pattern.len() && value_index == value.len()
}

impl MutableGroupEntry {
    pub(crate) fn mark_child_rewritten(&mut self) {
        // C4Group::Save rewrites a modified child to a temporary file and
        // moves that file back through Mother->Move. AddEntryOnDisk therefore
        // gives the replacement child the temp file's current timestamp and
        // executable bit (normally false), rather than retaining the old core.
        self.time = unix_time_now();
        self.executable = false;
    }

    fn original_crc_core(&self) -> EntryCoreCrc {
        match &self.data {
            MutableGroupEntryData::File(_) | MutableGroupEntryData::Child(_) => {
                EntryCoreCrc { state: 0, value: 0 }
            }
            MutableGroupEntryData::ExistingFile {
                crc_state,
                stored_crc,
                ..
            }
            | MutableGroupEntryData::PackedChild {
                crc_state,
                stored_crc,
                ..
            } => EntryCoreCrc {
                state: *crc_state,
                value: *stored_crc,
            },
        }
    }

    /// Returns the value native CalcCRC32 would produce when this entry is
    /// visited. `None` is the direct packed-child open failure that aborts the
    /// containing group's CRC traversal; nested failures are already projected
    /// to a successful numeric zero by the importer.
    fn calculated_crc(&self, entries: &[&MutableGroupEntry]) -> Option<u32> {
        let original = self.original_crc_core();
        if original.state == 2 {
            return Some(original.value);
        }
        match &self.data {
            MutableGroupEntryData::File(data) => Some(c4group_entry_crc(data, &self.name_bytes)),
            MutableGroupEntryData::ExistingFile {
                data,
                crc_state,
                stored_crc,
            } => Some(imported_file_entry_crc(
                data,
                &self.name_bytes,
                *crc_state,
                *stored_crc,
            )),
            MutableGroupEntryData::Child(_) | MutableGroupEntryData::PackedChild { .. } => {
                calculated_child_crc(&self.name_bytes, entries)
            }
        }
    }
}

/// Applies the same direct-child lookup CalcCRC32 reaches through
/// OpenAsChild: `*` rejects the request, `?` scans final entry order, and the
/// first name match is terminal even when it is not an openable child.
fn calculated_child_crc(pattern: &[u8], entries: &[&MutableGroupEntry]) -> Option<u32> {
    if pattern.contains(&b'*') {
        return None;
    }
    let selected = entries
        .iter()
        .copied()
        .find(|entry| wildcard_match(pattern, &entry.name_bytes))?;
    match &selected.data {
        MutableGroupEntryData::Child(child) => Some(child.contents_crc()),
        MutableGroupEntryData::PackedChild {
            child_contents_crc, ..
        } => *child_contents_crc,
        MutableGroupEntryData::File(_) | MutableGroupEntryData::ExistingFile { .. } => None,
    }
}

fn validate_entry_name(name: &[u8]) -> Result<(), MutableGroupError> {
    if name.contains(&0) {
        return Err(MutableGroupError::EntryNameContainsNul);
    }
    if name.len() > MAX_ENTRY_NAME_BYTES {
        return Err(MutableGroupError::EntryNameTooLong(name.len()));
    }
    Ok(())
}

/// zlib-compatible CRC-32 update, including support for chained calls.
pub fn c4group_file_crc(data: &[u8]) -> u32 {
    crc32(0, data)
}

/// zlib-compatible CRC-32 update, accelerated by the same library used for
/// C4Group compression and preserving chained-update semantics.
pub fn c4group_crc32(initial: u32, data: &[u8]) -> u32 {
    data.chunks(u32::MAX as usize).fold(initial, |crc, chunk| {
        // SAFETY: `chunk` remains live for the call and zlib reads exactly its
        // represented `uInt` length. The return value is the updated CRC only.
        unsafe { libz_sys::crc32(crc as _, chunk.as_ptr(), chunk.len() as _) as u32 }
    })
}

/// Computes the CRC C4Group writes for a regular entry after reading it back
/// from disk. Empty files are the one exception: their entry CRC is always
/// zero and does not include the filename.
pub(crate) fn c4group_entry_crc(data: &[u8], name: &[u8]) -> u32 {
    if data.is_empty() {
        0
    } else {
        crc32(crc32(0, data), name)
    }
}

/// Resolves one imported ordinary-file core at C4Group close time. New CRCs
/// precede both the empty-file and child checks in native `CalcCRC32`; this
/// helper handles ordinary files, so state two remains an unconditional cache
/// hit. State one is the legacy data-only CRC seed, and every other state is
/// recalculated like `C4GECS_None`.
fn imported_file_entry_crc(data: &[u8], name: &[u8], crc_state: u8, stored_crc: u32) -> u32 {
    if crc_state == 2 {
        return stored_crc;
    }
    if data.is_empty() {
        return 0;
    }
    let data_crc = if crc_state == 1 {
        stored_crc
    } else {
        crc32(0, data)
    };
    crc32(data_crc, name)
}

fn crc32(initial: u32, data: &[u8]) -> u32 {
    c4group_crc32(initial, data)
}

fn unix_time_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

fn entry_time_or_now(time: u32) -> u32 {
    if time == 0 {
        unix_time_now()
    } else {
        time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerated_crc_preserves_cpp_chained_update_semantics() {
        // C4Group_GetFileCRC starts at zero and chains zlib crc32 over every
        // file chunk (src/C4Group.cpp:429-469).
        let first = c4group_crc32(0, b"1234");

        assert_eq!(c4group_crc32(first, b"56789"), 0xcbf4_3926);
        assert_eq!(c4group_crc32(first, b""), first);
    }

    #[test]
    fn packed_entry_borrows_unchanged_payload_until_final_image_copy() {
        let entry = MutableGroupEntry {
            name: "marker.txt".to_owned(),
            name_bytes: b"marker.txt".to_vec(),
            data: MutableGroupEntryData::File(b"unchanged payload".to_vec()),
            time: 0,
            executable: false,
        };
        let source = match &entry.data {
            MutableGroupEntryData::File(data) => data.as_ptr(),
            _ => unreachable!(),
        };

        let packed = PackedEntry::from_entry(&entry).unwrap();

        assert_eq!(packed.data.as_ptr(), source);
    }
}
