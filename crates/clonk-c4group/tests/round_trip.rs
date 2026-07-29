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

    // `-a` adds a file from disk; `-as` stores it under another name (:148-158).
    let source = directory.path().join("Gamma.txt");
    std::fs::write(&source, b"gamma").expect("write source");
    let source = source.to_str().expect("utf-8 source");
    let added = c4group(&[group, "-a", source]);
    assert!(
        added.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    let renamed = c4group(&[group, "-as", source, "Delta.txt"]);
    assert!(renamed.status.success());
    let listed = String::from_utf8_lossy(&c4group(&[group]).stdout).into_owned();
    assert!(listed.contains("Gamma.txt"), "listing was {listed:?}");
    assert!(listed.contains("Delta.txt"), "listing was {listed:?}");

    // `-r` renames in place and `-d` deletes (:228-269).
    assert!(c4group(&[group, "-r", "Gamma.txt", "Epsilon.txt"])
        .status
        .success());
    assert!(c4group(&[group, "-d", "Delta.txt"]).status.success());
    let listed = String::from_utf8_lossy(&c4group(&[group]).stdout).into_owned();
    assert!(listed.contains("Epsilon.txt"), "listing was {listed:?}");
    assert!(!listed.contains("Delta.txt"), "listing was {listed:?}");
    // The untouched entries survived every rewrite.
    assert!(listed.contains("Alpha.txt"));
    assert!(listed.contains("Beta.png"));

    // Deleting an absent entry reports and fails rather than passing silently.
    let absent = c4group(&[group, "-d", "NotThere.txt"]);
    assert!(!absent.status.success());
    assert!(String::from_utf8_lossy(&absent.stderr).contains("no such entry"));

    // `-m` adds and then removes the source file (:181-200).
    let moved = directory.path().join("Moved.txt");
    std::fs::write(&moved, b"moved").expect("write moved");
    assert!(c4group(&[group, "-m", moved.to_str().expect("utf-8")])
        .status
        .success());
    assert!(!moved.exists(), "-m must delete the source");

    // `-u` then `-p` round-trips in place: the file becomes a directory of the
    // same name and back again (:289-326).
    assert!(c4group(&[group, "-u"]).status.success());
    let path = Path::new(group);
    assert!(path.is_dir(), "-u must replace the file with a directory");
    assert_eq!(
        std::fs::read(path.join("Alpha.txt")).expect("unpacked Alpha"),
        b"alpha"
    );
    let repacked = c4group(&[group, "-p"]);
    assert!(
        repacked.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&repacked.stderr)
    );
    assert!(path.is_file(), "-p must replace the directory with a file");
    let listed = String::from_utf8_lossy(&c4group(&[group]).stdout).into_owned();
    assert!(listed.contains("Alpha.txt"), "after repack: {listed:?}");
    assert!(listed.contains("Epsilon.txt"), "after repack: {listed:?}");

    // `-s` reorders entries: the sort list ranks earlier segments first, then
    // case-insensitive filename (:240-256).
    assert!(c4group(&[group, "-s", "Beta.png|*.txt"]).status.success());
    let sorted = String::from_utf8_lossy(&c4group(&[group]).stdout).into_owned();
    let order: Vec<&str> = sorted.lines().collect();
    assert_eq!(order.first(), Some(&"Beta.png"), "sorted: {order:?}");

    // `-z` reports the entry table, `-w` waits.
    let internals = c4group(&[group, "-z"]);
    assert!(internals.status.success());
    assert!(String::from_utf8_lossy(&internals.stdout).contains("entries"));
    assert!(c4group(&[group, "-w", "1"]).status.success());
    let bad_wait = c4group(&[group, "-w", "soon"]);
    assert!(!bad_wait.status.success());

    // An unimplemented command says so and fails rather than silently passing.
    let unsupported = c4group(&[group, "-g", "a", "b", "Title"]);
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
