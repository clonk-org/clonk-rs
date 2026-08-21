use clonk_core::std_config::Config;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

#[test]
fn parses_language_pack_fixture() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("repository root");
    let language_path = repo_root.join("planet/System.c4g/LanguageUS.txt");
    assert!(
        language_path.exists(),
        "missing language fixture: {}",
        language_path.display()
    );

    let file = File::open(&language_path).expect("open language file");
    let mut reader = BufReader::new(file);
    let config = Config::from_reader(&mut reader).expect("parse language config");

    // Sanity check: ensure we loaded comparable data volume to the legacy Catch2 fixture.
    assert!(
        config.iter().count() > 1300,
        "unexpectedly few entries parsed from language pack"
    );

    assert_eq!(config.get("IDS_LANG_NAME"), Some("English"));
    assert_eq!(config.get("IDS_BTN_ACCEPT"), Some("Accept"));
    assert_eq!(config.get("IDS_BTN_CHAT"), Some("&Chat"));
    assert_eq!(config.get("IDS_BTN_SIMPLE"), Some("<- Basic"));
    assert_eq!(config.get("IDS_BTN_EXTENDED"), Some("Extended ->"));
}

/// The shipped English table must stay valid UTF-8, because
/// `decode_legacy_script_text` picks its encoding **per file**: it takes the
/// UTF-8 branch when the whole file parses, and falls back to Windows-1252
/// otherwise (`clonk-resources/src/script_strings.rs:10-22`).
///
/// That fallback is deliberate — `LanguageDE.txt` really is Windows-1252 and
/// relies on it — but it makes the encoding an all-or-nothing property of the
/// file. The port's own `IDS_LAUNCHER_UI_*` strings are written in UTF-8 and
/// use U+2026, so a single Windows-1252 byte added anywhere in this file would
/// flip every one of them at once: `Copy…` silently becomes `Copyâ€¦` across
/// the launcher, with nothing else failing.
///
/// C++ has no such branch — `C4Language` treats string-table bytes as
/// Windows-1252 throughout — so this only guards strings C++ never reads.
#[test]
fn the_shipped_english_table_stays_utf8_so_launcher_ellipses_survive() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("repository root");
    let bytes = std::fs::read(repo_root.join("planet/System.c4g/LanguageUS.txt"))
        .expect("read the shipped English table");

    let text = std::str::from_utf8(&bytes).unwrap_or_else(|error| {
        panic!(
            "LanguageUS.txt stopped being valid UTF-8 at byte {}: every \
             IDS_LAUNCHER_UI_* ellipsis now decodes as Windows-1252 mojibake",
            error.valid_up_to()
        )
    });

    let copy = text
        .lines()
        .find_map(|line| line.strip_prefix("IDS_LAUNCHER_UI_BUTTON_COPY="))
        .expect("the launcher copy button caption");
    assert!(
        copy.ends_with('\u{2026}'),
        "expected a real ellipsis, got {copy:?}"
    );
    assert!(
        !copy.contains('\u{fffd}') && !copy.contains("â€"),
        "the caption decoded as mojibake: {copy:?}"
    );
}
