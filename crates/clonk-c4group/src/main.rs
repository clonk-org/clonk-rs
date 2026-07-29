//! The classic `c4group` command-line utility.
//!
//! `c4group_ng.cpp` opens each group in turn and runs the whole command list
//! against it, printing the contents when no command was given (:110-134).
//!
//! Commands whose group primitives this port does not expose yet report
//! themselves on stderr and set a failing exit status rather than being
//! silently ignored — see the `c4group` row in `PORT_STATUS.md`.

mod cli;

use std::path::Path;
use std::process::ExitCode;

use cli::{Command, CommandLine};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() {
        print_usage();
        return ExitCode::SUCCESS;
    }
    let (line, missing) = cli::parse(&arguments);
    for report in &missing {
        eprintln!("Missing argument for {} command", report.command);
    }
    let mut failed = !missing.is_empty();
    for option in &line.options.unknown {
        eprintln!("Unknown option {option}");
    }
    if line.groups.is_empty() {
        print_usage();
        return ExitCode::FAILURE;
    }
    for group in &line.groups {
        if !run_group(group, &line) {
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Runs every command against one group, reporting whether all succeeded.
fn run_group(path: &str, line: &CommandLine) -> bool {
    // `hGroup.Open(szFilename, true)` (:118).
    let group = match clonk_resources::Group::open(Path::new(path)) {
        Ok(group) => group,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    // "No commands: display contents" (:120-124).
    if line.commands.is_empty() {
        return list_entries(&group, path, &[]);
    }
    // Collected before testing: every command runs, as C++ walks the whole
    // list regardless of an earlier failure. `all` alone would short-circuit.
    let results: Vec<bool> = line
        .commands
        .iter()
        .map(|command| run_command(&group, path, command))
        .collect();
    results.into_iter().all(|succeeded| succeeded)
}

fn run_command(group: &clonk_resources::Group, path: &str, command: &Command) -> bool {
    match command {
        Command::List { wildcards } => list_entries(group, path, wildcards),
        Command::PrintMaker => {
            // `std::println("{}", hGroup.GetMaker())` (:346-348).
            println!("{}", group.maker().unwrap_or_default());
            true
        }
        Command::Extract { files } => {
            // Every named file is extracted even if an earlier one failed.
            let results: Vec<bool> = files
                .iter()
                .map(|file| extract_entry(group, file, Path::new(file)))
                .collect();
            results.into_iter().all(|succeeded| succeeded)
        }
        Command::ExtractTo { file, target } => extract_entry(group, file, Path::new(target)),
        Command::Unknown { argument } => {
            eprintln!("Unknown command {argument}");
            false
        }
        unsupported => {
            eprintln!(
                "c4group: {} is not implemented yet",
                command_name(unsupported)
            );
            false
        }
    }
}

/// The native listing: one line per entry, filtered by the given wildcards
/// (`c4group_ng.cpp:270-284`).
fn list_entries(group: &clonk_resources::Group, path: &str, wildcards: &[String]) -> bool {
    let entries = match group.entries() {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    for entry in entries {
        // `name_bytes` carries the exact legacy filename; the listing is text,
        // so it is shown lossily rather than dropped.
        let name = String::from_utf8_lossy(&entry.name_bytes).into_owned();
        if wildcards.is_empty()
            || wildcards
                .iter()
                .any(|pattern| matches_wildcard(pattern, &name))
        {
            println!("{name}");
        }
    }
    true
}

fn extract_entry(group: &clonk_resources::Group, entry: &str, target: &Path) -> bool {
    match group.read_file(entry) {
        Ok(bytes) => match std::fs::write(target, bytes) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("Error: {}: {error}", target.display());
                false
            }
        },
        Err(error) => {
            eprintln!("Error: {entry}: {error}");
            false
        }
    }
}

/// `WildcardMatch`-style matching for the listing filter: `*` spans any run and
/// `?` one character.
fn matches_wildcard(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some(p);
                resume = n;
                p += 1;
            }
            Some('?') => {
                p += 1;
                n += 1;
            }
            Some(character) if character.eq_ignore_ascii_case(&name[n]) => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some(index) => {
                    p = index + 1;
                    resume += 1;
                    n = resume;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|character| *character == '*')
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Add { .. } => "add",
        Command::AddAs { .. } => "add as",
        Command::Move { .. } => "move",
        Command::Extract { .. } => "extract",
        Command::ExtractTo { .. } => "extract to",
        Command::Delete { .. } => "delete",
        Command::Sort { .. } => "sort",
        Command::Rename { .. } => "rename",
        Command::List { .. } => "list",
        Command::MakeOriginal => "original",
        Command::Pack => "pack",
        Command::Unpack => "unpack",
        Command::Explode => "explode",
        Command::PrintMaker => "maker",
        Command::GenerateUpdate { .. } => "generate update",
        Command::ApplyUpdate => "apply update",
        Command::PrintInternals => "print internals",
        Command::Wait { .. } => "wait",
        Command::Unknown { .. } => "unknown",
    }
}

fn print_usage() {
    println!("c4group [options] group(s) [command(s)]");
}
