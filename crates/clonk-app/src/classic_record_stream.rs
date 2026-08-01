use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clonk_engine::scenario::ScenarioLoaderHead;
use clonk_resources::{compress_c4group_image, merge_extracted_group_entries, Group, MutableGroup};

pub(crate) fn convert_classic_record_stream(
    stream_path: &Path,
    process_group_maker: &[u8],
) -> Result<PathBuf> {
    let compressed = std::fs::read(stream_path)
        .with_context(|| format!("read classic record stream {}", stream_path.display()))?;
    let stream = clonk_network::decode_classic_record_stream(&compressed)
        .with_context(|| format!("decode classic record stream {}", stream_path.display()))?;
    let initial = Group::from_top_level_memory(PathBuf::from("~initial.tmp"), stream.initial_group)
        .context("open initial streamed record save")?;
    let initial_head = ScenarioLoaderHead::load_from_group_for_resource_registration(&initial)
        .context("load Scenario.txt from initial streamed record save")?;
    let origin = initial_head
        .origin()
        .filter(|origin| !origin.is_empty())
        .ok_or_else(|| anyhow!("initial streamed record save has no Scenario.Head.Origin"))?;
    let origin_path = PathBuf::from(origin.replace('\\', std::path::MAIN_SEPARATOR_STR));
    let origin_group = crate::open_group_path_for_folder_map(&origin_path)
        .with_context(|| format!("open record origin scenario {}", origin_path.display()))?;
    let output_path = classic_record_output_path(stream_path);
    if origin_group.is_directory() {
        persist_directory_record(
            origin_group.root(),
            &initial,
            stream.files,
            stream.control_record,
            &output_path,
        )?;
        return Ok(output_path);
    }

    let mut record = MutableGroup::from_group(&origin_group)
        .with_context(|| format!("copy record origin scenario {}", origin_path.display()))?;

    merge_extracted_group_entries(&mut record, &initial)
        .context("merge initial streamed record save")?;
    for file in stream.files {
        insert_streamed_file(
            &mut record,
            file.filename.as_bytes(),
            file.data,
            process_group_maker,
        )
        .context("insert streamed record file")?;
    }
    record
        .add_file("CtrlRec.c4b", stream.control_record)
        .context("install converted CtrlRec.c4b")?;
    record.resort_for_filename_bytes(crate::path_to_legacy_bytes(output_path.as_path()));
    if !process_group_maker.is_empty() {
        record.set_maker_bytes(process_group_maker);
    }

    crate::persist_console_save_group(&record, &output_path, false)
        .with_context(|| format!("write converted record {}", output_path.display()))?;
    Ok(output_path)
}

fn persist_directory_record(
    origin_path: &Path,
    initial: &Group,
    files: Vec<clonk_network::ClassicRecordStreamFile>,
    control_record: Vec<u8>,
    output_path: &Path,
) -> Result<()> {
    let parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create record parent {}", parent.display()))?;
    let filename = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("record");
    let staged = crate::create_sibling_rewrite_directory(parent, filename)?;
    let prepared = (|| -> Result<()> {
        crate::copy_directory_contents(origin_path, &staged)
            .context("copy folder-backed record origin")?;
        overlay_group_entries(&staged, initial).context("merge initial streamed record save")?;
        for file in files {
            overlay_directory_file(&staged, file.filename.as_bytes(), &file.data)
                .context("insert streamed record file")?;
        }
        overlay_directory_file(&staged, b"CtrlRec.c4b", &control_record)
            .context("install converted CtrlRec.c4b")?;
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = crate::remove_file_or_directory(&staged);
        return Err(error);
    }
    let committed = crate::commit_staged_path_with_backup(&staged, output_path, || Ok(()));
    if committed.is_err() {
        let _ = crate::remove_file_or_directory(&staged);
    }
    committed.with_context(|| format!("write converted record {}", output_path.display()))
}

fn overlay_group_entries(destination: &Path, source: &Group) -> Result<()> {
    for entry in source
        .entries()
        .context("enumerate streamed initial save")?
    {
        let data = source.read_entry_bytes_exact(&entry).with_context(|| {
            format!(
                "read streamed initial entry {}",
                entry.relative_path.display()
            )
        })?;
        let data = if entry.is_directory {
            // C4Group_UnpackDirectory wraps a child's existing raw image in a
            // standalone gzip envelope without reopening or resorting it.
            compress_c4group_image(&data).with_context(|| {
                format!(
                    "extract streamed initial child {}",
                    entry.relative_path.display()
                )
            })?
        } else {
            data
        };
        overlay_directory_file(destination, &entry.name_bytes, &data)?;
    }
    Ok(())
}

fn overlay_directory_file(destination: &Path, name: &[u8], data: &[u8]) -> Result<()> {
    crate::write_folder_save_entry(destination, name, data)
}

fn insert_streamed_file(
    record: &mut MutableGroup,
    filename: &[u8],
    data: Vec<u8>,
    process_group_maker: &[u8],
) -> Result<()> {
    let label = path_from_legacy_bytes(filename);
    if let Ok(child) = Group::from_top_level_memory(label, data.clone()) {
        let mut raw_image = child.raw_image().context("read streamed child group")?;
        let mut contents_crc = child.contents_crc_or_zero();
        let requires_rewrite = child.requires_rewrite();
        let mut rewritten =
            MutableGroup::from_group(&child).context("copy streamed child group")?;
        let reordered = rewritten.resort_for_filename_bytes(filename.to_vec());
        if requires_rewrite || reordered {
            if !process_group_maker.is_empty() {
                rewritten.set_maker_bytes(process_group_maker);
            }
            contents_crc = rewritten.contents_crc();
            raw_image = rewritten
                .pack_raw()
                .context("rewrite streamed child group")?;
        }
        record
            .add_packed_child_bytes(filename.to_vec(), raw_image, contents_crc)
            .context("add streamed child group")?;
    } else {
        record
            .add_file_bytes(filename.to_vec(), data)
            .context("add streamed file")?;
    }
    Ok(())
}

#[cfg(unix)]
fn path_from_legacy_bytes(bytes: &[u8]) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_legacy_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(unix)]
fn classic_record_output_path(stream_path: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let mut bytes = stream_path.as_os_str().as_bytes().to_vec();
    let filename_start = bytes
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(0, |index| index + 1);
    if let Some(dot) = bytes[filename_start..]
        .iter()
        .rposition(|byte| *byte == b'.')
        .map(|index| filename_start + index)
    {
        bytes.truncate(dot);
    } else {
        bytes.pop();
    }
    bytes.extend_from_slice(b".c4s");
    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn classic_record_output_path(stream_path: &Path) -> PathBuf {
    if stream_path.extension().is_some() {
        return stream_path.with_extension("c4s");
    }
    let mut path = stream_path.to_string_lossy().into_owned();
    path.pop();
    path.push_str(".c4s");
    PathBuf::from(path)
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use clonk_engine::{LegacyCString, RCT_FRAME};
    use clonk_resources::{Group, MutableGroup};

    use super::{classic_record_output_path, convert_classic_record_stream};

    #[test]
    fn classic_output_path_matches_native_extension_handling() {
        assert_eq!(
            classic_record_output_path(Path::new("League.Round.c4r")),
            PathBuf::from("League.Round.c4s")
        );
        assert_eq!(
            classic_record_output_path(Path::new("extensionless")),
            PathBuf::from("extensionles.c4s")
        );
    }

    #[test]
    fn classic_record_stream_rewrites_a_packed_origin_like_cpp() {
        let working_directory = std::env::current_dir().expect("record-stream working directory");
        let fixture = tempfile::Builder::new()
            .prefix("lc-record-packed-")
            .tempdir_in(&working_directory)
            .expect("packed record-stream fixture");
        let origin_path = fixture.path().join("Origin.c4s");
        let origin_reference = origin_path
            .strip_prefix(&working_directory)
            .expect("origin is below the working directory")
            .to_string_lossy()
            .replace('\\', "/");

        let mut preserved_child = MutableGroup::new("Preserved.c4g");
        preserved_child.set_maker("Origin Child Maker");
        preserved_child
            .add_file("Preserved.txt", b"origin child".to_vec())
            .expect("add preserved origin child file");
        let mut origin = MutableGroup::new("Origin.c4s");
        origin.set_maker("Origin Parent Maker");
        origin
            .add_file(
                "Scenario.txt",
                b"[Head]\nTitle=Packed origin\nIcon=2\nMaxPlayer=1\nNoInitialize=1\n".to_vec(),
            )
            .expect("add packed origin scenario core");
        origin
            .add_file("Layer.txt", b"origin".to_vec())
            .expect("add packed origin layer");
        origin
            .add_child("Preserved.c4g", preserved_child)
            .expect("add preserved origin child");
        std::fs::write(&origin_path, origin.pack().expect("pack origin scenario"))
            .expect("write packed origin scenario");

        let mut initial_child = MutableGroup::new("InitialChild.c4g");
        initial_child.set_maker("Initial Child Maker");
        initial_child
            .add_file("InitialChild.txt", b"initial child".to_vec())
            .expect("add initial child file");
        let mut initial = MutableGroup::new("Initial.c4s");
        initial
            .add_file(
                "Scenario.txt",
                format!(
                    "[Head]\nTitle=Packed conversion\nIcon=2\nMaxPlayer=1\nSaveGame=1\nNoInitialize=1\nReplay=1\nOrigin={origin_reference}\n"
                )
                .into_bytes(),
            )
            .expect("add streamed initial scenario core");
        initial
            .add_file("Layer.txt", b"initial".to_vec())
            .expect("add streamed initial layer");
        initial
            .add_child("InitialChild.c4g", initial_child)
            .expect("add streamed initial child");
        let initial = initial.pack().expect("pack streamed initial save");
        let expected_initial_child =
            Group::from_top_level_memory(PathBuf::from("Initial.c4s"), initial.clone())
                .expect("reopen streamed initial save")
                .open_child("InitialChild.c4g")
                .expect("open streamed initial child")
                .raw_image()
                .expect("read streamed initial child image");

        let mut generic_child = MutableGroup::new("WrongGeneric.tmp");
        generic_child.set_maker("Generic Stream Maker");
        generic_child
            .add_file("Z.txt", b"z".to_vec())
            .expect("add generic child file");
        generic_child
            .add_file("A.txt", b"a".to_vec())
            .expect("add second generic child file");
        let generic_child = generic_child.pack().expect("pack generic stream child");
        let expected_generic_child =
            Group::from_top_level_memory(PathBuf::from("WrongGeneric.tmp"), generic_child.clone())
                .expect("open generic stream child")
                .raw_image()
                .expect("read generic stream child image");

        let mut section_child = MutableGroup::new("WrongSection.tmp");
        section_child.set_maker("Section Stream Maker");
        section_child
            .add_file("Objects.txt", b"objects".to_vec())
            .expect("add out-of-order section object file");
        section_child
            .add_file("Scenario.txt", b"scenario".to_vec())
            .expect("add out-of-order section core");
        let section_child = section_child.pack().expect("pack section stream child");

        let initial_name = LegacyCString::from_bytes(b"ignored.tmp".to_vec()).unwrap();
        let mut raw = clonk_network::encode_league_stream_file_chunk(&initial_name, &initial)
            .expect("encode initial file chunk");
        for (delta, name, data) in [
            (2, b"Generic.c4g".as_slice(), generic_child.as_slice()),
            (3, b"SectLater.c4g".as_slice(), section_child.as_slice()),
        ] {
            let name = LegacyCString::from_bytes(name.to_vec()).unwrap();
            let mut chunk = clonk_network::encode_league_stream_file_chunk(&name, data)
                .expect("encode later file chunk");
            chunk[0] = delta;
            raw.extend_from_slice(&chunk);
        }
        raw.extend_from_slice(&[4, RCT_FRAME]);

        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&raw).expect("compress record stream");
        let stream_path = fixture.path().join("Packed.c4r");
        std::fs::write(
            &stream_path,
            encoder.finish().expect("finish record-stream compression"),
        )
        .expect("write record stream");

        let output_path =
            convert_classic_record_stream(&stream_path, b"Process Maker").expect("convert stream");
        assert!(output_path.is_file());
        let output = Group::open(&output_path).expect("open packed converted record");
        assert_eq!(output.maker(), Some("Process Maker"));
        assert_eq!(output.read_file("Layer.txt").unwrap(), b"initial");
        assert_eq!(output.read_file("CtrlRec.c4b").unwrap(), [9, RCT_FRAME]);
        assert_eq!(
            output.open_child("Preserved.c4g").unwrap().maker(),
            Some("Origin Child Maker")
        );
        let output_initial_child = output.open_child("InitialChild.c4g").unwrap();
        assert_eq!(output_initial_child.maker(), Some("Initial Child Maker"));
        assert_eq!(
            output_initial_child.raw_image().unwrap(),
            expected_initial_child
        );
        let output_generic_child = output.open_child("Generic.c4g").unwrap();
        assert_eq!(output_generic_child.maker(), Some("Generic Stream Maker"));
        assert_eq!(
            output_generic_child.raw_image().unwrap(),
            expected_generic_child,
            "a destination without a native sort list is copied opaquely"
        );
        let output_section_child = output.open_child("SectLater.c4g").unwrap();
        assert_eq!(output_section_child.maker(), Some("Process Maker"));
        assert_eq!(
            output_section_child
                .entries()
                .unwrap()
                .into_iter()
                .map(|entry| entry.name_bytes)
                .collect::<Vec<_>>(),
            [b"Scenario.txt".to_vec(), b"Objects.txt".to_vec()],
            "renaming a streamed child selects the destination sort list"
        );
    }
}
