//! The classic `c4group` command line.
//!
//! `c4group_ng.cpp:530-580` scans leading options, then treats every remaining
//! argument up to the first command as a group path, then walks the commands
//! (`:136-400`). Commands are `-<letter>` with their own arguments; an argument
//! beginning with `-` terminates the preceding command's argument list, which
//! is how `-a file1 file2 -e other` parses.
//!
//! Parsing is separated from execution so the whole matrix can be pinned
//! without touching the filesystem.

/// The leading options (`c4group_ng.cpp:545-576`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Options {
    /// `-q` / `-v`. C++ starts quiet and `-v` clears it (:549-555).
    pub quiet: bool,
    /// `-r`.
    pub recursive: bool,
    /// `-i` — Windows shell registration.
    pub register_shell: bool,
    /// `-u` — Windows shell unregistration.
    pub unregister_shell: bool,
    /// `-p` — wait for a keypress after the last command.
    pub prompt_at_end: bool,
    /// `-x:<command>` — run this after the last command. C++ copies from
    /// `argv[i] + 3`, i.e. everything after `-x:` (:571).
    pub execute_at_end: Option<String>,
    /// Options C++ reports on stderr and otherwise ignores (:573-576).
    pub unknown: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            // `fQuiet` starts true in the native tool.
            quiet: true,
            recursive: false,
            register_shell: false,
            unregister_shell: false,
            prompt_at_end: false,
            execute_at_end: None,
            unknown: Vec::new(),
        }
    }
}

/// One parsed command (`c4group_ng.cpp:146-400`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// `-a <file>...` — add each file under its own name (:148-180).
    Add { files: Vec<String> },
    /// `-as <file> <as>` — add one file under a different name (:153-158).
    AddAs { file: String, stored_as: String },
    /// `-m <file>...` — add and delete the source (:181-200).
    Move { files: Vec<String> },
    /// `-e <file>...` — extract (:201-227).
    Extract { files: Vec<String> },
    /// `-et <file> <to>` — extract one entry to a given path.
    ExtractTo { file: String, target: String },
    /// `-d <file>...` (:228-239).
    Delete { files: Vec<String> },
    /// `-s <list>` — override the sort list (:240-256).
    Sort { list: String },
    /// `-r <old> <new>` (:257-269).
    Rename { from: String, to: String },
    /// `-l` / `-v` — list, optionally filtered by wildcards (:270-284).
    List { wildcards: Vec<String> },
    /// `-o` — mark original (:285-288).
    MakeOriginal,
    /// `-p` — pack (:289-307).
    Pack,
    /// `-u` — unpack (:308-326).
    Unpack,
    /// `-x` — explode (:327-345).
    Explode,
    /// `-k` — print the maker (:346-349).
    PrintMaker,
    /// `-g <source> <target> <title>` — generate an update group (:350-379).
    GenerateUpdate {
        source: String,
        target: String,
        title: String,
    },
    /// `-y` — apply an update (:380-389).
    ApplyUpdate,
    /// `-z` — print internal group structures (:390-396).
    PrintInternals,
    /// `-w <milliseconds>` — wait (:397-400).
    Wait { milliseconds: String },
    /// A `-` argument whose letter is not a command.
    Unknown { argument: String },
}

/// A parsed command line: options, the group paths, and the commands to run
/// against each of them. No commands at all means "list the contents"
/// (`c4group_ng.cpp:130-134`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLine {
    pub options: Options,
    pub groups: Vec<String>,
    pub commands: Vec<Command>,
}

/// Errors C++ reports on stderr before skipping the command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingArgument {
    pub command: &'static str,
}

fn is_flag(argument: &str) -> bool {
    // C++ also accepts `/` on Windows for options, but command detection is
    // `-` only (`c4group_ng.cpp:143`).
    argument.starts_with('-')
}

/// Collects a command's arguments: everything up to the next `-` argument.
fn take_arguments(arguments: &[String], index: &mut usize) -> Vec<String> {
    let mut collected = Vec::new();
    while *index + 1 < arguments.len() && !is_flag(&arguments[*index + 1]) {
        *index += 1;
        collected.push(arguments[*index].clone());
    }
    collected
}

/// Parses the classic command line from `argv[1..]`.
///
/// Returns the parse plus the missing-argument diagnostics C++ prints; a
/// command with too few arguments is reported and dropped, matching :150-152.
pub fn parse(arguments: &[String]) -> (CommandLine, Vec<MissingArgument>) {
    let mut options = Options::default();
    let mut index = 0;

    // Leading options (:536-580). Scanning stops at the first non-option.
    while index < arguments.len() {
        let argument = &arguments[index];
        if !is_flag(argument) && !(cfg!(windows) && argument.starts_with('/')) {
            break;
        }
        match argument.chars().nth(1) {
            Some('q') => options.quiet = true,
            Some('v') => options.quiet = false,
            Some('r') => options.recursive = true,
            Some('i') => options.register_shell = true,
            Some('u') => options.unregister_shell = true,
            Some('p') => options.prompt_at_end = true,
            // `argv[i] + 3` — everything past `-x:`.
            Some('x') => options.execute_at_end = Some(argument.chars().skip(3).collect()),
            _ => options.unknown.push(argument.clone()),
        }
        index += 1;
    }

    // Group paths run until the first command (:583-590).
    let mut groups = Vec::new();
    while index < arguments.len() && !is_flag(&arguments[index]) {
        groups.push(arguments[index].clone());
        index += 1;
    }

    let mut commands = Vec::new();
    let mut missing = Vec::new();
    while index < arguments.len() {
        let argument = arguments[index].clone();
        if !is_flag(&argument) {
            index += 1;
            continue;
        }
        let letter = argument.chars().nth(1);
        // `-as` and `-et` are the two two-letter forms (:153,:206).
        let qualifier = argument.chars().nth(2);
        match letter {
            Some('a') => {
                let files = take_arguments(arguments, &mut index);
                match (qualifier, files.as_slice()) {
                    (Some('s'), [file, stored_as]) => commands.push(Command::AddAs {
                        file: file.clone(),
                        stored_as: stored_as.clone(),
                    }),
                    (Some('s'), _) => missing.push(MissingArgument { command: "add as" }),
                    (_, []) => missing.push(MissingArgument { command: "add" }),
                    _ => commands.push(Command::Add { files }),
                }
            }
            Some('m') => {
                let files = take_arguments(arguments, &mut index);
                if files.is_empty() {
                    missing.push(MissingArgument { command: "move" });
                } else {
                    commands.push(Command::Move { files });
                }
            }
            Some('e') => {
                let files = take_arguments(arguments, &mut index);
                match (qualifier, files.as_slice()) {
                    (Some('t'), [file, target]) => commands.push(Command::ExtractTo {
                        file: file.clone(),
                        target: target.clone(),
                    }),
                    (Some('t'), _) => missing.push(MissingArgument {
                        command: "extract to",
                    }),
                    (_, []) => missing.push(MissingArgument { command: "extract" }),
                    _ => commands.push(Command::Extract { files }),
                }
            }
            Some('d') => {
                let files = take_arguments(arguments, &mut index);
                if files.is_empty() {
                    missing.push(MissingArgument { command: "delete" });
                } else {
                    commands.push(Command::Delete { files });
                }
            }
            Some('s') => {
                let arguments = take_arguments(arguments, &mut index);
                match arguments.first() {
                    Some(list) => commands.push(Command::Sort { list: list.clone() }),
                    None => missing.push(MissingArgument { command: "sort" }),
                }
            }
            Some('r') => {
                let arguments = take_arguments(arguments, &mut index);
                match arguments.as_slice() {
                    [from, to] => commands.push(Command::Rename {
                        from: from.clone(),
                        to: to.clone(),
                    }),
                    _ => missing.push(MissingArgument { command: "rename" }),
                }
            }
            // `-l` and `-v` are the same listing command (:270-271).
            Some('l') | Some('v') => {
                let wildcards = take_arguments(arguments, &mut index);
                commands.push(Command::List { wildcards });
            }
            Some('o') => commands.push(Command::MakeOriginal),
            Some('p') => commands.push(Command::Pack),
            Some('u') => commands.push(Command::Unpack),
            Some('x') => commands.push(Command::Explode),
            Some('k') => commands.push(Command::PrintMaker),
            Some('g') => {
                let arguments = take_arguments(arguments, &mut index);
                match arguments.as_slice() {
                    [source, target, title] => commands.push(Command::GenerateUpdate {
                        source: source.clone(),
                        target: target.clone(),
                        title: title.clone(),
                    }),
                    _ => missing.push(MissingArgument {
                        command: "generate update",
                    }),
                }
            }
            Some('y') => commands.push(Command::ApplyUpdate),
            Some('z') => commands.push(Command::PrintInternals),
            Some('w') => {
                let arguments = take_arguments(arguments, &mut index);
                match arguments.first() {
                    Some(milliseconds) => commands.push(Command::Wait {
                        milliseconds: milliseconds.clone(),
                    }),
                    None => missing.push(MissingArgument { command: "wait" }),
                }
            }
            _ => commands.push(Command::Unknown { argument }),
        }
        index += 1;
    }

    (
        CommandLine {
            options,
            groups,
            commands,
        },
        missing,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &[&str]) -> (CommandLine, Vec<MissingArgument>) {
        let arguments: Vec<String> = line.iter().map(|part| (*part).to_owned()).collect();
        parse(&arguments)
    }

    // c4group_ng.cpp:130-134 — no commands means "display contents".
    #[test]
    fn c4group_cli_parses_bare_group_as_a_listing() {
        let (line, missing) = parse_line(&["Objects.c4d"]);
        assert_eq!(line.groups, vec!["Objects.c4d".to_owned()]);
        assert!(line.commands.is_empty());
        assert!(missing.is_empty());
        // The native tool starts quiet (:549-555).
        assert!(line.options.quiet);
    }

    // c4group_ng.cpp:545-576.
    #[test]
    fn c4group_cli_parses_leading_options() {
        let (line, _) = parse_line(&["-v", "-r", "-p", "-x:echo done", "Pack.c4g", "-l"]);
        assert!(!line.options.quiet, "-v clears quiet mode");
        assert!(line.options.recursive);
        assert!(line.options.prompt_at_end);
        assert_eq!(line.options.execute_at_end.as_deref(), Some("echo done"));
        assert_eq!(line.groups, vec!["Pack.c4g".to_owned()]);
        assert_eq!(line.commands, vec![Command::List { wildcards: vec![] }]);

        // -q wins where it appears last, and shell registration is recognised.
        let (line, _) = parse_line(&["-v", "-q", "-i", "-u", "Pack.c4g"]);
        assert!(line.options.quiet);
        assert!(line.options.register_shell);
        assert!(line.options.unregister_shell);

        // An unrecognised option is reported and skipped, not fatal (:573-576).
        let (line, _) = parse_line(&["-Q", "Pack.c4g"]);
        assert_eq!(line.options.unknown, vec!["-Q".to_owned()]);
        assert_eq!(line.groups, vec!["Pack.c4g".to_owned()]);
    }

    // c4group_ng.cpp:146-400 — the full command matrix, including the two
    // two-letter forms and multi-argument commands.
    #[test]
    fn c4group_cli_parses_the_native_command_matrix() {
        let (line, missing) = parse_line(&[
            "Pack.c4g",
            "-a",
            "one.txt",
            "two.txt",
            "-as",
            "src.txt",
            "stored.txt",
            "-m",
            "gone.txt",
            "-e",
            "out.txt",
            "-et",
            "in.txt",
            "there.txt",
            "-d",
            "old.txt",
            "-s",
            "*.png|*.txt",
            "-r",
            "before",
            "after",
            "-l",
            "*.c4d",
            "-o",
            "-p",
            "-u",
            "-x",
            "-k",
            "-g",
            "a",
            "b",
            "Title",
            "-y",
            "-z",
            "-w",
            "500",
        ]);
        assert!(missing.is_empty(), "every command had its arguments");
        assert_eq!(line.groups, vec!["Pack.c4g".to_owned()]);
        assert_eq!(
            line.commands,
            vec![
                // `-a` takes every following non-flag argument (:167-176).
                Command::Add {
                    files: vec!["one.txt".to_owned(), "two.txt".to_owned()]
                },
                Command::AddAs {
                    file: "src.txt".to_owned(),
                    stored_as: "stored.txt".to_owned()
                },
                Command::Move {
                    files: vec!["gone.txt".to_owned()]
                },
                Command::Extract {
                    files: vec!["out.txt".to_owned()]
                },
                Command::ExtractTo {
                    file: "in.txt".to_owned(),
                    target: "there.txt".to_owned()
                },
                Command::Delete {
                    files: vec!["old.txt".to_owned()]
                },
                Command::Sort {
                    list: "*.png|*.txt".to_owned()
                },
                Command::Rename {
                    from: "before".to_owned(),
                    to: "after".to_owned()
                },
                Command::List {
                    wildcards: vec!["*.c4d".to_owned()]
                },
                Command::MakeOriginal,
                Command::Pack,
                Command::Unpack,
                Command::Explode,
                Command::PrintMaker,
                Command::GenerateUpdate {
                    source: "a".to_owned(),
                    target: "b".to_owned(),
                    title: "Title".to_owned()
                },
                Command::ApplyUpdate,
                Command::PrintInternals,
                Command::Wait {
                    milliseconds: "500".to_owned()
                },
            ]
        );
    }

    // A command whose arguments are missing is reported and dropped (:150-152).
    #[test]
    fn c4group_cli_reports_missing_command_arguments() {
        let (line, missing) = parse_line(&["Pack.c4g", "-a", "-e"]);
        assert!(line.commands.is_empty());
        assert_eq!(
            missing,
            vec![
                MissingArgument { command: "add" },
                MissingArgument { command: "extract" },
            ]
        );

        // The two-letter forms need exactly two arguments.
        let (_, missing) = parse_line(&["Pack.c4g", "-as", "only.txt"]);
        assert_eq!(missing, vec![MissingArgument { command: "add as" }]);
        let (_, missing) = parse_line(&["Pack.c4g", "-et", "only.txt"]);
        assert_eq!(
            missing,
            vec![MissingArgument {
                command: "extract to"
            }]
        );
        let (_, missing) = parse_line(&["Pack.c4g", "-r", "one"]);
        assert_eq!(missing, vec![MissingArgument { command: "rename" }]);
    }

    // `:583-590` — several groups may precede the commands, and each runs the
    // whole command list.
    #[test]
    fn c4group_cli_parses_multiple_groups() {
        let (line, _) = parse_line(&["-v", "One.c4g", "Two.c4g", "Three.c4g", "-k"]);
        assert_eq!(
            line.groups,
            vec![
                "One.c4g".to_owned(),
                "Two.c4g".to_owned(),
                "Three.c4g".to_owned()
            ]
        );
        assert_eq!(line.commands, vec![Command::PrintMaker]);
    }

    // An unrecognised command letter is preserved so execution can report it.
    #[test]
    fn c4group_cli_preserves_unknown_commands() {
        let (line, missing) = parse_line(&["Pack.c4g", "-Q"]);
        assert!(missing.is_empty());
        assert_eq!(
            line.commands,
            vec![Command::Unknown {
                argument: "-Q".to_owned()
            }]
        );
    }
}
