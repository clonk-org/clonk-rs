//! Native semantics for the recording and screenshot output roots.
//!
//! `C4ConfigGeneral::CreateSaveFolder` (`C4Config.cpp:1397-1412`) prepares a
//! save/record root: one directory creation, then a `Title.txt` naming the root
//! in the active language — written only when absent, so a user's own title
//! survives. `C4Record.cpp:118-145` runs it for the configured record root.
//!
//! The screenshot root is composed differently: `C4Config.cpp:1326-1332`
//! appends the configured `ScreenshotFolder` to `ExePath` *raw*, and
//! `C4Config::AtScreenshotPath` (`:1378-1392`) attempts a single directory
//! creation, falling back to `ExePath` when that fails rather than creating a
//! tree.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `C4CFN_WriteTitle` (`C4Components.h:68`).
pub(crate) const WRITE_TITLE_FILE: &str = "Title.txt";

/// The `"<lang>:<title>"` line `CreateSaveFolder` writes
/// (`C4Config.cpp:1404-1405`). C++ takes the first two characters of
/// `Config.General.Language`, which holds a list like `"DE,US"`.
pub(crate) fn localized_title_component(language: &str, language_title: &str) -> String {
    let language_code: String = language.chars().take(2).collect();
    format!("{language_code}:{language_title}")
}

/// `C4ConfigGeneral::CreateSaveFolder` (`C4Config.cpp:1397-1412`).
///
/// Creates `directory` with a single-level creation when it does not exist,
/// then writes `Title.txt` **only if absent** — an existing title is never
/// overwritten (:1407-1409).
pub(crate) fn create_save_folder(
    directory: &Path,
    language_title: &str,
    language: &str,
) -> io::Result<()> {
    if !directory.is_dir() {
        // `MakeDirectory`, not a recursive create: C++ creates one level.
        fs::create_dir(directory)?;
    }
    let title_file = directory.join(WRITE_TITLE_FILE);
    if !title_file.exists() {
        fs::write(
            &title_file,
            localized_title_component(language, language_title),
        )?;
    }
    Ok(())
}

/// `C4Config::Load`'s screenshot-path composition (`C4Config.cpp:1326-1332`):
/// `ScreenshotFolder` is appended to `ExePath` verbatim, with no trimming or
/// normalization. An absolute or `..`-bearing configured value therefore
/// composes exactly as C++ leaves it.
pub(crate) fn compose_screenshot_path(exe_path: &Path, screenshot_folder: &str) -> PathBuf {
    if screenshot_folder.is_empty() {
        return exe_path.to_path_buf();
    }
    let mut composed = exe_path.as_os_str().to_owned();
    if !exe_path.as_os_str().is_empty() {
        composed.push(std::path::MAIN_SEPARATOR_STR);
    }
    composed.push(screenshot_folder);
    PathBuf::from(composed)
}

/// `C4Config::AtScreenshotPath`'s directory handling (`C4Config.cpp:1381-1390`).
///
/// One trailing separator is stripped, then a single directory creation is
/// attempted. Failure falls back to `ExePath` — C++ does not create a tree and
/// does not error out.
pub(crate) fn resolve_screenshot_directory(composed: &Path, exe_path: &Path) -> PathBuf {
    let trimmed = strip_one_trailing_separator(composed);
    if trimmed.is_dir() {
        return trimmed;
    }
    // `MakeDirectory`, one level only (:1384).
    match fs::create_dir(&trimmed) {
        Ok(()) => trimmed,
        Err(_) => exe_path.to_path_buf(),
    }
}

/// `AtScreenshotPath` removes exactly one trailing separator (:1381-1383).
fn strip_one_trailing_separator(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    text.strip_suffix(std::path::MAIN_SEPARATOR)
        .map_or_else(|| path.to_path_buf(), PathBuf::from)
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    // C4Config.cpp:1397-1412; C4Record.cpp:118-145 — the record root gains a
    // language-prefixed Title.txt, and an existing one is left alone.
    #[test]
    fn recording_root_writes_localized_title_component() {
        let root = tempfile::tempdir().expect("temp dir");
        let records = root.path().join("Records");

        create_save_folder(&records, "Records", "DE,US").expect("create the record root");
        assert!(records.is_dir());
        // Only the first two characters of the language list are used (:1403).
        assert_eq!(
            fs::read_to_string(records.join(WRITE_TITLE_FILE)).expect("title"),
            "DE:Records"
        );

        // A second pass must not overwrite a title the user edited (:1407-1409).
        fs::write(records.join(WRITE_TITLE_FILE), "DE:Meine Aufzeichnungen").expect("user title");
        create_save_folder(&records, "Records", "DE,US").expect("second pass");
        assert_eq!(
            fs::read_to_string(records.join(WRITE_TITLE_FILE)).expect("title"),
            "DE:Meine Aufzeichnungen",
            "an existing title component must be preserved"
        );

        assert_eq!(localized_title_component("US", "Records"), "US:Records");
        // A short or empty language yields whatever is there, as SCopy does.
        assert_eq!(localized_title_component("", "Records"), ":Records");

        // One level only: a missing parent is an error, not a recursive create.
        let nested = root.path().join("Missing").join("Records");
        assert!(create_save_folder(&nested, "Records", "US").is_err());
    }

    // C4Config.cpp:1326-1332,1378-1392 — raw composition, one mkdir, ExePath
    // fallback.
    #[test]
    fn screenshot_folder_matches_native_raw_single_mkdir_fallback() {
        let root = tempfile::tempdir().expect("temp dir");
        let exe_path = root.path();

        // The configured value is appended verbatim (:1330).
        let composed = compose_screenshot_path(exe_path, "Screenshots");
        assert_eq!(composed, exe_path.join("Screenshots"));
        // An empty configured folder leaves ExePath itself (:1327-1332).
        assert_eq!(compose_screenshot_path(exe_path, ""), exe_path);

        // The directory is created one level deep and then reused.
        let resolved = resolve_screenshot_directory(&composed, exe_path);
        assert_eq!(resolved, exe_path.join("Screenshots"));
        assert!(resolved.is_dir());
        assert_eq!(
            resolve_screenshot_directory(&composed, exe_path),
            exe_path.join("Screenshots"),
            "an existing directory is reused rather than recreated"
        );

        // A trailing separator is stripped before the existence check (:1381-1383).
        let with_separator = PathBuf::from(format!(
            "{}{}",
            composed.display(),
            std::path::MAIN_SEPARATOR
        ));
        assert_eq!(
            resolve_screenshot_directory(&with_separator, exe_path),
            exe_path.join("Screenshots")
        );

        // A missing parent makes the single creation fail, so C++ falls back to
        // ExePath instead of building the tree (:1386-1389).
        let unreachable = compose_screenshot_path(exe_path, "Missing/Deeper");
        assert_eq!(
            resolve_screenshot_directory(&unreachable, exe_path),
            exe_path,
            "a failed single creation falls back to ExePath"
        );
        assert!(
            !exe_path.join("Missing").exists(),
            "the fallback must not create intermediate directories"
        );
    }
}
