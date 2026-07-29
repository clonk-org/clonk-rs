//! The classic `c4group` command-line utility.
//!
//! `c4group_ng.cpp` opens each group in turn and runs the whole command list
//! against it, printing the contents when no command was given (:110-134).
//!
//! Commands whose group primitives this port does not expose yet report
//! themselves on stderr and set a failing exit status rather than being
//! silently ignored — see the `c4group` row in `PORT_STATUS.md`.

mod cli;
mod edit;
mod wildcard;

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
    // `-i`/`-u` register or unregister the shell integration and run without a
    // group (`c4group_ng.cpp:561-568`).
    if line.options.register_shell || line.options.unregister_shell {
        failed |= !apply_shell_registration(&line);
    }
    for group in &line.groups {
        if !run_group(group, &line) {
            failed = true;
        }
    }
    // "Done. Press any key to continue." (:680-684).
    if line.options.prompt_at_end {
        println!("\nDone. Press any key to continue.");
        let mut discard = String::new();
        let _ = std::io::stdin().read_line(&mut discard);
    }
    // Execute when done (:686-704).
    if let Some(command) = line
        .options
        .execute_at_end
        .as_deref()
        .filter(|c| !c.is_empty())
    {
        println!("Executing: {command}");
        if let Err(error) = spawn_detached(command) {
            eprintln!("Error: {command}: {error}");
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `-i` / `-u`: install or remove the classic file associations. C++ only has
/// these on Windows (`c4group_ng.cpp:470-540`); elsewhere they are reported as
/// unsupported rather than silently ignored.
fn apply_shell_registration(line: &CommandLine) -> bool {
    #[cfg(windows)]
    {
        let Ok(module) = std::env::current_exe() else {
            eprintln!("c4group: could not resolve the executable path");
            return false;
        };
        let module = module.to_string_lossy();
        if line.options.unregister_shell {
            return clonk_platform::file_classes::unregister_file_classes(&module);
        }
        return clonk_platform::file_classes::register_file_classes(&module);
    }
    #[cfg(not(windows))]
    {
        let _ = line;
        eprintln!("c4group: shell registration is a Windows-only command");
        false
    }
}

/// Starts `command` without waiting, as C++ does with `CreateProcess`/`fork`
/// (`c4group_ng.cpp:690-704`).
fn spawn_detached(command: &str) -> std::io::Result<()> {
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return Ok(());
    };
    std::process::Command::new(program)
        .args(parts)
        .spawn()
        .map(|_| ())
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
    let mut group = Some(group);
    let results: Vec<bool> = line
        .commands
        .iter()
        .map(|command| run_command(&mut group, path, command))
        .collect();
    results.into_iter().all(|succeeded| succeeded)
}

/// Runs one command. `group` is taken by `Option` because a mutating command
/// must close the read handle before repacking over the same path.
fn run_command(group: &mut Option<clonk_resources::Group>, path: &str, command: &Command) -> bool {
    let Some(open) = group.as_ref() else {
        eprintln!("Error: {path}: group is no longer open");
        return false;
    };
    match command {
        Command::List { wildcards } => list_entries(open, path, wildcards),
        Command::PrintMaker => {
            // `std::println("{}", hGroup.GetMaker())` (:346-348).
            println!("{}", open.maker().unwrap_or_default());
            true
        }
        Command::Extract { files } => {
            // Every named file is extracted even if an earlier one failed.
            let results: Vec<bool> = files
                .iter()
                .map(|file| extract_entry(open, file, Path::new(file)))
                .collect();
            results.into_iter().all(|succeeded| succeeded)
        }
        Command::ExtractTo { file, target } => extract_entry(open, file, Path::new(target)),
        Command::Unpack => unpack(group, path),
        Command::Pack => pack(group, path),
        Command::Explode => explode(group, path),
        Command::Sort { list } => sort_entries(group, path, list),
        Command::PrintInternals => print_internals(open, path),
        Command::Wait { milliseconds } => wait(milliseconds),
        Command::Add { .. }
        | Command::AddAs { .. }
        | Command::Move { .. }
        | Command::Delete { .. }
        | Command::Rename { .. }
        | Command::MakeOriginal => mutate(group, path, command),
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
                .any(|pattern| wildcard::matches(pattern, &name))
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

/// Applies a mutating command by rebuilding the group and repacking it
/// (see `edit`). The read handle is dropped first so the write cannot race it.
fn mutate(group: &mut Option<clonk_resources::Group>, path: &str, command: &Command) -> bool {
    let Some(open) = group.as_ref() else {
        return false;
    };
    let filename = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    let mut mutable = match edit::to_mutable(open, &filename) {
        Ok(mutable) => mutable,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    // The rebuild copied everything; release the reader before rewriting.
    *group = None;

    let mut moved_sources: Vec<&String> = Vec::new();
    let applied = match command {
        Command::Add { files } => add_files(&mut mutable, files, None),
        Command::AddAs { file, stored_as } => {
            add_files(&mut mutable, std::slice::from_ref(file), Some(stored_as))
        }
        Command::Move { files } => {
            moved_sources = files.iter().collect();
            add_files(&mut mutable, files, None)
        }
        Command::Delete { files } => {
            let removed: Vec<bool> = files
                .iter()
                .map(|file| {
                    mutable.remove_entry(file) || {
                        eprintln!("Error: {file}: no such entry");
                        false
                    }
                })
                .collect();
            removed.into_iter().all(|one| one)
        }
        Command::Rename { from, to } => {
            mutable.rename_entry(from, to) || {
                eprintln!("Error: {from}: no such entry");
                false
            }
        }
        Command::MakeOriginal => {
            mutable.make_original(true);
            true
        }
        _ => false,
    };

    if let Err(error) = edit::write_back(&mutable, Path::new(path)) {
        eprintln!("Error: {path}: {error}");
        return false;
    }
    // `-m` deletes the sources only once the group has been rewritten (:181-200).
    if applied {
        for source in moved_sources {
            if let Err(error) = std::fs::remove_file(source) {
                eprintln!("Error: {source}: {error}");
            }
        }
    }
    // Reopen so later commands in the same run see the new contents.
    match clonk_resources::Group::open(Path::new(path)) {
        Ok(reopened) => *group = Some(reopened),
        Err(error) => eprintln!("Error: {path}: {error}"),
    }
    applied
}

/// Reads each source from disk and stores it, optionally under another name.
fn add_files(
    mutable: &mut clonk_resources::group_writer::MutableGroup,
    files: &[String],
    stored_as: Option<&String>,
) -> bool {
    let results: Vec<bool> = files
        .iter()
        .map(|file| {
            let source = Path::new(file);
            let name = stored_as.map(String::as_str).unwrap_or_else(|| {
                source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(file)
            });
            match std::fs::read(source) {
                Ok(bytes) => {
                    // Replace an existing entry rather than duplicating it.
                    mutable.remove_entry(name);
                    match mutable.add_file_bytes(name, bytes) {
                        Ok(()) => true,
                        Err(error) => {
                            eprintln!("Error: {file}: {error}");
                            false
                        }
                    }
                }
                Err(error) => {
                    eprintln!("Error: {file}: {error}");
                    false
                }
            }
        })
        .collect();
    results.into_iter().all(|added| added)
}

/// `-s <list>` — reorder the entries and save (`c4group_ng.cpp:240-256`).
fn sort_entries(group: &mut Option<clonk_resources::Group>, path: &str, list: &str) -> bool {
    let Some(open) = group.as_ref() else {
        return false;
    };
    let names: Vec<String> = match open.entries() {
        Ok(entries) => entries
            .iter()
            .map(|entry| String::from_utf8_lossy(&entry.name_bytes).into_owned())
            .collect(),
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    let order = edit::sorted_entry_order(&names, list);
    let filename = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    let mutable = match edit::to_mutable_ordered(open, &filename, &order) {
        Ok(mutable) => mutable,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    *group = None;
    if let Err(error) = edit::write_back(&mutable, Path::new(path)) {
        eprintln!("Error: {path}: {error}");
        return false;
    }
    reopen(group, path);
    true
}

/// `-x` — unpack, then unpack every child group in turn
/// (`c4group_ng.cpp:327-345`).
fn explode(group: &mut Option<clonk_resources::Group>, path: &str) -> bool {
    if !unpack(group, path) {
        return false;
    }
    // After unpacking, `path` is a directory; explode each packed child in it.
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    let mut all = true;
    for entry in entries.flatten() {
        let child = entry.path();
        if !child.is_file() {
            continue;
        }
        // Only a readable group explodes further; anything else is a plain file.
        let Ok(opened) = clonk_resources::Group::open(&child) else {
            continue;
        };
        let mut opened = Some(opened);
        let Some(child_path) = child.to_str() else {
            continue;
        };
        if !explode(&mut opened, child_path) {
            all = false;
        }
    }
    all
}

/// `-z` — the entry table C++ prints from `PrintInternals`
/// (`c4group_ng.cpp:390-396`).
fn print_internals(group: &clonk_resources::Group, path: &str) -> bool {
    let entries = match group.entries() {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    println!("{path}: {} entries", entries.len());
    for entry in entries {
        println!(
            "  {} size {} time {} crc {:08x}{}",
            String::from_utf8_lossy(&entry.name_bytes),
            entry.size,
            entry.time,
            entry.stored_crc,
            if entry.is_directory { " (group)" } else { "" }
        );
    }
    true
}

/// `-w <milliseconds>` (`c4group_ng.cpp:397-400`).
fn wait(milliseconds: &str) -> bool {
    match milliseconds.parse::<u64>() {
        Ok(delay) => {
            std::thread::sleep(std::time::Duration::from_millis(delay));
            true
        }
        Err(error) => {
            eprintln!("Error: {milliseconds}: {error}");
            false
        }
    }
}

/// `-p` — pack an unpacked group in place (`c4group_ng.cpp:289-307`). The
/// directory is replaced by a packed file of the same name.
fn pack(group: &mut Option<clonk_resources::Group>, path: &str) -> bool {
    let Some(open) = group.as_ref() else {
        return false;
    };
    let target = Path::new(path);
    if target.is_file() {
        // Already packed: C++ repacks, which for this port is the same rebuild.
        return mutate_noop(group, path);
    }
    let filename = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    let mutable = match edit::to_mutable(open, &filename) {
        Ok(mutable) => mutable,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    *group = None;
    let packed = match mutable.pack() {
        Ok(packed) => packed,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    // Replace the directory with the packed file of the same name.
    let staged = target.with_extension("c4group-packing");
    if let Err(error) = std::fs::write(&staged, packed) {
        eprintln!("Error: {}: {error}", staged.display());
        return false;
    }
    if let Err(error) = std::fs::remove_dir_all(target) {
        eprintln!("Error: {path}: {error}");
        let _ = std::fs::remove_file(&staged);
        return false;
    }
    if let Err(error) = std::fs::rename(&staged, target) {
        eprintln!("Error: {path}: {error}");
        return false;
    }
    reopen(group, path);
    true
}

/// A repack that changes nothing, used when `-p` targets an already-packed
/// group. The rebuild is byte-identical (see `edit`).
fn mutate_noop(group: &mut Option<clonk_resources::Group>, path: &str) -> bool {
    mutate(group, path, &Command::Delete { files: Vec::new() })
}

fn reopen(group: &mut Option<clonk_resources::Group>, path: &str) {
    match clonk_resources::Group::open(Path::new(path)) {
        Ok(reopened) => *group = Some(reopened),
        Err(error) => eprintln!("Error: {path}: {error}"),
    }
}

/// `-u` — unpack in place: the group file is replaced by a directory of the
/// same name holding its entries (`c4group_ng.cpp:308-326`).
fn unpack(group: &mut Option<clonk_resources::Group>, path: &str) -> bool {
    let Some(open) = group.as_ref() else {
        return false;
    };
    let target = Path::new(path);
    if target.is_dir() {
        // Already unpacked; nothing to do, as C++'s UnpackDirectory reports.
        return true;
    }
    let staged = target.with_extension("c4group-unpacking");
    if let Err(error) = std::fs::create_dir_all(&staged) {
        eprintln!("Error: {}: {error}", staged.display());
        return false;
    }
    let entries = match open.entries() {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Error: {path}: {error}");
            return false;
        }
    };
    let results: Vec<bool> = entries
        .iter()
        .map(|entry| {
            let name = String::from_utf8_lossy(&entry.name_bytes).into_owned();
            match open.read_entry_bytes_exact(entry) {
                Ok(bytes) => match std::fs::write(staged.join(&name), bytes) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("Error: {name}: {error}");
                        false
                    }
                },
                Err(error) => {
                    eprintln!("Error: {name}: {error}");
                    false
                }
            }
        })
        .collect();
    let written = results.into_iter().all(|one| one);
    if !written {
        let _ = std::fs::remove_dir_all(&staged);
        return false;
    }
    // Release the reader before replacing the file it was opened from.
    *group = None;
    if let Err(error) = std::fs::remove_file(target) {
        eprintln!("Error: {path}: {error}");
        let _ = std::fs::remove_dir_all(&staged);
        return false;
    }
    if let Err(error) = std::fs::rename(&staged, target) {
        eprintln!("Error: {path}: {error}");
        return false;
    }
    reopen(group, path);
    true
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
