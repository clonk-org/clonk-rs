//! End-to-end `c4group` behaviour against a native-format fixture.

use std::path::Path;
use std::process::Command;

use clonk_resources::group_writer::MutableGroup;

/// Builds a packed group the native tool would accept.
fn packed_fixture(directory: &Path) -> std::path::PathBuf {
    let mut group = MutableGroup::new("Fixture.c4g");
    group.set_maker("Round Trip");
    group
        .add_file("Alpha.txt", b"alpha".to_vec())
        .expect("add Alpha.txt");
    group
        .add_file("Beta.png", b"beta".to_vec())
        .expect("add Beta.png");
    let path = directory.join("Fixture.c4g");
    std::fs::write(&path, group.pack().expect("pack fixture")).expect("write fixture");
    path
}

fn c4group(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_c4group"))
        .args(arguments)
        .output()
        .expect("run c4group")
}

// c4group_ng.cpp:110-134,270-284,346-348 — the default listing, the explicit
// listing with wildcards, the maker, and extraction.
#[test]
fn c4group_cli_round_trips_native_command_matrix() {
    let directory = tempfile::tempdir().expect("temp dir");
    let group = packed_fixture(directory.path());
    let group = group.to_str().expect("utf-8 path");

    // No commands: display contents (:120-124).
    let listing = c4group(&[group]);
    assert!(listing.status.success());
    let listed = String::from_utf8_lossy(&listing.stdout);
    assert!(listed.contains("Alpha.txt"), "listing was {listed:?}");
    assert!(listed.contains("Beta.png"), "listing was {listed:?}");

    // `-l` with a wildcard filters the listing (:270-284).
    let filtered = c4group(&[group, "-l", "*.png"]);
    assert!(filtered.status.success());
    let filtered = String::from_utf8_lossy(&filtered.stdout);
    assert!(filtered.contains("Beta.png"));
    assert!(!filtered.contains("Alpha.txt"), "wildcard did not filter");

    // `-k` prints the maker (:346-348).
    let maker = c4group(&[group, "-k"]);
    assert!(maker.status.success());
    assert_eq!(String::from_utf8_lossy(&maker.stdout).trim(), "Round Trip");

    // `-et` extracts one entry to a chosen path (:206).
    let target = directory.path().join("extracted.txt");
    let extracted = c4group(&[
        group,
        "-et",
        "Alpha.txt",
        target.to_str().expect("utf-8 target"),
    ]);
    assert!(
        extracted.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    assert_eq!(std::fs::read(&target).expect("read extracted"), b"alpha");

    // A missing argument is reported and fails, rather than being ignored.
    let incomplete = c4group(&[group, "-as", "only.txt"]);
    assert!(!incomplete.status.success());
    assert!(String::from_utf8_lossy(&incomplete.stderr).contains("Missing argument for add as"));

    // An unimplemented command says so and fails rather than silently passing.
    let unsupported = c4group(&[group, "-p"]);
    assert!(!unsupported.status.success());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("not implemented"));

    // An unreadable group is an error, not a panic.
    let missing = c4group(&[directory
        .path()
        .join("Absent.c4g")
        .to_str()
        .expect("utf-8 path")]);
    assert!(!missing.status.success());
}
