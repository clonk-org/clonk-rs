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
