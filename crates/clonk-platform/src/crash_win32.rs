//! Windows unhandled-exception diagnostics.
//!
//! `C4CrashHandlerWin32.cpp:360-470` installs a one-shot unhandled-exception
//! filter that writes a human-readable report to the log descriptor, writes a
//! timestamped minidump beside the user path, shows a native message box naming
//! both artifacts, and then returns `EXCEPTION_CONTINUE_SEARCH` so the OS keeps
//! its own exception processing. `C4WinMain.cpp:68-70` installs it before the
//! application initializes.
//!
//! The report text and dump path are built by host-independent helpers here so
//! they can be pinned on any platform; only the handler itself is Win32-gated.

/// `C4ENGINENAME` — the dump and dialog both name the engine, not the port.
const ENGINE_NAME: &str = "LegacyClonk";

/// Builds the dump path for `user_path`, mirroring the
/// `"%s-crash-%04d-%02d-%02d-%02d-%02d-%02d.dmp"` template at
/// `C4CrashHandlerWin32.cpp:390,410`. `time` is UTC, matching the `GetSystemTime`
/// call at :406. Returns `None` when the path is empty, which is how C++ treats
/// a corrupted config (:381-384).
pub fn crash_dump_filename(user_path: &str, time: (u16, u8, u8, u8, u8, u8)) -> Option<String> {
    (!user_path.is_empty()).then(|| {
        let (year, month, day, hour, minute, second) = time;
        // "Make sure the path ends in a backslash" (:400-404).
        let separator = if user_path.ends_with('\\') { "" } else { "\\" };
        format!(
            "{user_path}{separator}{ENGINE_NAME}-crash-\
             {year:04}-{month:02}-{day:02}-{hour:02}-{minute:02}-{second:02}.dmp"
        )
    })
}

/// The exception codes `SafeTextDump` names individually
/// (`C4CrashHandlerWin32.cpp:97-113`). `STATUS_ASSERTION_FAILURE` is the
/// locally defined `0xC0000420` from :82-84.
pub const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const EXCEPTION_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const EXCEPTION_IN_PAGE_ERROR: u32 = 0xC000_0006;
pub const EXCEPTION_NONCONTINUABLE_EXCEPTION: u32 = 0xC000_0025;
pub const EXCEPTION_PRIV_INSTRUCTION: u32 = 0xC000_0096;
pub const EXCEPTION_STACK_OVERFLOW: u32 = 0xC000_00FD;
pub const EXCEPTION_GUARD_PAGE: u32 = 0x8000_0001;
pub const STATUS_ASSERTION_FAILURE: u32 = 0xC000_0420;

/// `EXCEPTION_NONCONTINUABLE` — the flag :115 compares against.
pub const EXCEPTION_NONCONTINUABLE: u32 = 0x1;

/// The `LOG_EXCEPTION(code, text)` table at `C4CrashHandlerWin32.cpp:99-108`,
/// falling back to the `%#08x` unknown-exception line at :111.
pub fn exception_description(code: u32) -> String {
    let known = match code {
        EXCEPTION_ACCESS_VIOLATION => Some((
            "EXCEPTION_ACCESS_VIOLATION",
            "The thread tried to read from or write to a virtual address for which it does not \
             have the appropriate access.",
        )),
        EXCEPTION_ILLEGAL_INSTRUCTION => Some((
            "EXCEPTION_ILLEGAL_INSTRUCTION",
            "The thread tried to execute an invalid instruction.",
        )),
        EXCEPTION_IN_PAGE_ERROR => Some((
            "EXCEPTION_IN_PAGE_ERROR",
            "The thread tried to access a page that was not present, and the system was unable to \
             load the page.",
        )),
        EXCEPTION_NONCONTINUABLE_EXCEPTION => Some((
            "EXCEPTION_NONCONTINUABLE_EXCEPTION",
            "The thread tried to continue execution after a noncontinuable exception occurred.",
        )),
        EXCEPTION_PRIV_INSTRUCTION => Some((
            "EXCEPTION_PRIV_INSTRUCTION",
            "The thread tried to execute an instruction whose operation is not allowed in the \
             current machine mode.",
        )),
        EXCEPTION_STACK_OVERFLOW => {
            Some(("EXCEPTION_STACK_OVERFLOW", "The thread used up its stack."))
        }
        EXCEPTION_GUARD_PAGE => Some((
            "EXCEPTION_GUARD_PAGE",
            "The thread accessed memory allocated with the PAGE_GUARD modifier.",
        )),
        STATUS_ASSERTION_FAILURE => Some((
            "STATUS_ASSERTION_FAILURE",
            "The thread specified a pre- or postcondition that did not hold.",
        )),
        _ => None,
    };
    known.map_or_else(
        || format!("{code:#08x}: The thread raised an unknown exception.\n"),
        |(name, text)| format!("{name}: {text}\n"),
    )
}

/// The banner at `C4CrashHandlerWin32.cpp:86-95`. The dump lines are omitted for
/// assertion failures and when no dump was written (:89).
pub fn report_header(code: u32, dump_filename: Option<&str>) -> String {
    const RULE: &str = "**********************************************************************\n";
    let dump = dump_filename
        .filter(|name| !name.is_empty() && code != STATUS_ASSERTION_FAILURE)
        .map(|name| {
            format!(
                "* A crash dump may have been written to {name}\n\
                 * If this file exists, please send it to a developer for investigation.\n"
            )
        })
        .unwrap_or_default();
    format!("{RULE}* UNHANDLED EXCEPTION\n{dump}{RULE}")
}

/// Formats a pointer the way `POINTER_FORMAT` does on x64 (:75-76).
fn pointer(value: usize) -> String {
    format!("0x{value:016x}")
}

/// The `EXCEPTION_ACCESS_VIOLATION`/`EXCEPTION_IN_PAGE_ERROR` arm of the
/// exception-information switch (`C4CrashHandlerWin32.cpp:120-150`). Other codes
/// contribute no detail line.
pub fn access_violation_detail(code: u32, parameters: &[usize]) -> String {
    if !matches!(code, EXCEPTION_ACCESS_VIOLATION | EXCEPTION_IN_PAGE_ERROR) {
        return String::new();
    }
    if parameters.len() < 2 {
        return "Additional information for the exception was not provided.\n".to_owned();
    }
    // EXCEPTION_READ_FAULT/WRITE_FAULT/EXECUTE_FAULT (:132-141).
    let access = match parameters[0] {
        0 => "tried to read from memory".to_owned(),
        1 => "tried to write to memory".to_owned(),
        8 => "caused an user-mode DEP violation".to_owned(),
        other => format!("tried to access ({other:#x}) memory"),
    };
    let address = pointer(parameters[1]);
    // The page-error NTSTATUS tail (:143-148).
    let page_error = if code == EXCEPTION_IN_PAGE_ERROR {
        parameters.get(2).map_or_else(
            || "The NTSTATUS code that resulted in this exception was not provided.\n".to_owned(),
            |status| {
                format!(
                    "The NTSTATUS code that resulted in this exception was {}.\n",
                    pointer(*status)
                )
            },
        )
    } else {
        String::new()
    };
    format!(
        "Additional information for the exception: The thread {access} at address {address}.\n\
         {page_error}"
    )
}

/// The `CONTEXT` fields the x86_64 register block reads
/// (`C4CrashHandlerWin32.cpp:168-181`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct X64Registers {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rbp: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rsp: usize,
    pub rip: usize,
}

/// The x86_64 register dump at `C4CrashHandlerWin32.cpp:168-181`. The stray
/// leading spaces before `R8` and `R9` are the C++ format strings' alignment.
pub fn x64_register_lines(registers: &X64Registers) -> String {
    let p = pointer;
    format!(
        "\nProcessor registers (x86_64):\n\
         RAX: {}, RBX: {}, RCX: {}, RDX: {}\n\
         RBP: {}, RSI: {}, RDI: {},  R8: {}\n\
         \x20R9: {}, R10: {}, R11: {}, R12: {}\n\
         R13: {}, R14: {}, R15: {}\n\
         RSP: {}, RIP: {}\n",
        p(registers.rax),
        p(registers.rbx),
        p(registers.rcx),
        p(registers.rdx),
        p(registers.rbp),
        p(registers.rsi),
        p(registers.rdi),
        p(registers.r8),
        p(registers.r9),
        p(registers.r10),
        p(registers.r11),
        p(registers.r12),
        p(registers.r13),
        p(registers.r14),
        p(registers.r15),
        p(registers.rsp),
        p(registers.rip),
    )
}

/// The EFLAGS line at `C4CrashHandlerWin32.cpp:196-202`: overflow, direction,
/// sign, zero, auxiliary carry, parity, carry.
pub fn eflags_line(eflags: u32) -> String {
    const FLAGS: [(u32, char); 7] = [
        (0x800, 'O'),
        (0x400, 'D'),
        (0x80, 'S'),
        (0x40, 'Z'),
        (0x10, 'A'),
        (0x4, 'P'),
        (0x1, 'C'),
    ];
    let letters: String = FLAGS
        .iter()
        .map(|&(bit, letter)| if eflags & bit != 0 { letter } else { '.' })
        .collect();
    format!("EFLAGS: {eflags:#010x} ({letters})\n")
}

/// The message-body assembled at `C4CrashHandlerWin32.cpp:427-447`.
pub fn crash_dialog_text(log_path: Option<&str>, dump_path: Option<&str>) -> String {
    let mut text = "LegacyClonk crashed. Please report this crash ".to_owned();
    if log_path.is_none() && dump_path.is_none() {
        text.push_str("to the developers.");
        return text;
    }
    text.push_str("together with the following information to the developers:\n");
    if let Some(log) = log_path {
        text.push_str(&format!("\nYou can find detailed information in {log}."));
    }
    if let Some(dump) = dump_path {
        text.push_str(&format!("\nA crash dump has been generated at {dump}."));
    }
    text
}

/// `C4CrashHandlerWin32.cpp:115-118`.
pub fn continuable_line(exception_flags: u32) -> &'static str {
    if exception_flags == EXCEPTION_NONCONTINUABLE {
        "This is a non-continuable exception.\n"
    } else {
        "This is a continuable exception.\n"
    }
}

/// The `EXCEPTION_RECORD`/`CONTEXT` fields the report draws on
/// (`C4CrashHandlerWin32.cpp:86-202`), lifted out of the raw Win32 structures so
/// the report can be composed and asserted on any host.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExceptionSummary {
    /// `ExceptionRecord->ExceptionCode`.
    pub code: u32,
    /// `ExceptionRecord->ExceptionFlags`.
    pub exception_flags: u32,
    /// `ExceptionRecord->ExceptionInformation`, truncated to `NumberParameters`.
    pub parameters: Vec<usize>,
    /// The `ContextRecord` general-purpose registers.
    pub registers: X64Registers,
    /// `ContextRecord->EFlags`.
    pub eflags: u32,
}

/// Assembles the human-readable report `SafeTextDump` writes to the log
/// descriptor (`C4CrashHandlerWin32.cpp:86-202`) for one exception.
pub fn compose_report(exception: &ExceptionSummary, dump_filename: Option<&str>) -> String {
    format!(
        "{}{}{}{}{}{}",
        report_header(exception.code, dump_filename),
        exception_description(exception.code),
        continuable_line(exception.exception_flags),
        access_violation_detail(exception.code, &exception.parameters),
        x64_register_lines(&exception.registers),
        eflags_line(exception.eflags),
    )
}

#[cfg(windows)]
pub use windows_impl::{
    crash_artifacts_for, install, set_log_descriptor, set_user_path, CrashArtifacts,
};

#[cfg(windows)]
mod windows_impl {
    use super::{
        compose_report, crash_dialog_text, crash_dump_filename, ExceptionSummary, X64Registers,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, SYSTEMTIME};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileA, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
        FILE_SHARE_READ,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        MiniDumpNormal, MiniDumpWriteDump, SetUnhandledExceptionFilter, EXCEPTION_POINTERS,
        MINIDUMP_EXCEPTION_INFORMATION,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTime;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxA, MB_ICONERROR};

    /// `EXCEPTION_CONTINUE_SEARCH` — "Call native exception handler" (:468).
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    /// `static bool FirstCrash = true` (:367-368): the filter reports once.
    static FIRST_CRASH: AtomicBool = AtomicBool::new(true);

    /// `Config.General.UserPath`, captured at install time because the filter
    /// must not walk live config state (:374-375).
    static USER_PATH: Mutex<String> = Mutex::new(String::new());

    /// The log descriptor `GetLogFD()` returns (:432,450), or a negative
    /// sentinel when no session log is open.
    static LOG_DESCRIPTOR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

    /// What one crash produced, for the dialog and for tests.
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub struct CrashArtifacts {
        /// The composed report text, as written to the log descriptor.
        pub report: String,
        /// The dump path, when `CREATE_NEW` succeeded (:417-424).
        pub dump_path: Option<String>,
        /// The message-box body (:427-447).
        pub dialog: String,
    }

    extern "C" {
        #[link_name = "_write"]
        fn crt_write(fd: i32, buf: *const std::ffi::c_void, count: u32) -> i32;
    }

    /// Installs the one-shot unhandled-exception filter
    /// (`C4CrashHandlerWin32.cpp:644`), which `C4WinMain.cpp:68-70` does before
    /// the application initializes. The user path and log descriptor are
    /// published separately as they become known, mirroring the way C++ reads
    /// `Config.General.UserPath` and `GetLogFD()` from inside the filter
    /// (:374-375,:432).
    pub fn install() {
        // SAFETY: registering a process-wide filter; the callback below only
        // touches statics it owns and OS APIs valid in exception context.
        unsafe { SetUnhandledExceptionFilter(Some(generate_dump)) };
    }

    /// Publishes `Config.General.UserPath`, the directory the dump goes in
    /// (`C4CrashHandlerWin32.cpp:374-375`). Until this is set no dump is written,
    /// which is how C++ treats an unusable path (:381-384).
    pub fn set_user_path(user_path: &str) {
        if let Ok(mut path) = USER_PATH.lock() {
            path.clear();
            path.push_str(user_path);
        }
    }

    /// Publishes the session log descriptor `GetLogFD()` returns (:432,450).
    pub fn set_log_descriptor(log_descriptor: i32) {
        LOG_DESCRIPTOR.store(log_descriptor, Ordering::SeqCst);
    }

    /// Builds the report and writes the minidump for one exception, returning
    /// what was produced. Split out from [`generate_dump`] so the artifacts can
    /// be asserted without raising a real unhandled exception.
    pub fn crash_artifacts_for(
        user_path: &str,
        log_path: Option<&str>,
        exception: &ExceptionSummary,
        exception_pointers: Option<*const EXCEPTION_POINTERS>,
    ) -> CrashArtifacts {
        // SAFETY: GetSystemTime fills the whole struct; SYSTEMTIME is plain data.
        let system_time = unsafe {
            let mut time: SYSTEMTIME = std::mem::zeroed();
            GetSystemTime(&mut time);
            time
        };
        let candidate = crash_dump_filename(
            user_path,
            (
                system_time.wYear,
                system_time.wMonth as u8,
                system_time.wDay as u8,
                system_time.wHour as u8,
                system_time.wMinute as u8,
                system_time.wSecond as u8,
            ),
        );
        // "If we can't create a *new* file to dump into, don't dump at all" (:417-424).
        let dump_path =
            candidate.and_then(|path| write_minidump(&path, exception_pointers).then_some(path));
        let report = compose_report(exception, dump_path.as_deref());
        CrashArtifacts {
            dialog: crash_dialog_text(log_path, dump_path.as_deref()),
            report,
            dump_path,
        }
    }

    /// `CreateFileA` + `MiniDumpWriteDump` (:417,455-464). Reports whether the
    /// dump file was created.
    fn write_minidump(path: &str, exception_pointers: Option<*const EXCEPTION_POINTERS>) -> bool {
        let Ok(path_c) = std::ffi::CString::new(path) else {
            return false;
        };
        // SAFETY: `path_c` outlives the call; CREATE_NEW never clobbers.
        let file: HANDLE = unsafe {
            CreateFileA(
                path_c.as_ptr().cast(),
                FILE_GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                std::ptr::null(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                0,
            )
        };
        if file == INVALID_HANDLE_VALUE {
            return false;
        }
        // SAFETY: `file` is a live handle we just opened and close below.
        unsafe {
            let parameters = MINIDUMP_EXCEPTION_INFORMATION {
                ThreadId: GetCurrentThreadId(),
                ExceptionPointers: exception_pointers.unwrap_or(std::ptr::null()).cast_mut(),
                ClientPointers: 1,
            };
            MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                file,
                MiniDumpNormal,
                if exception_pointers.is_some() {
                    &parameters
                } else {
                    std::ptr::null()
                },
                std::ptr::null(),
                std::ptr::null(),
            );
            CloseHandle(file);
        }
        true
    }

    /// The `SetUnhandledExceptionFilter` callback (`C4CrashHandlerWin32.cpp:360-470`).
    unsafe extern "system" fn generate_dump(exception_pointers: *const EXCEPTION_POINTERS) -> i32 {
        // "if (!FirstCrash) return EXCEPTION_CONTINUE_SEARCH" (:367-368).
        if !FIRST_CRASH.swap(false, Ordering::SeqCst) {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let mut exception = exception_pointers
            .as_ref()
            .and_then(|pointers| pointers.ExceptionRecord.as_ref())
            .map(|record| {
                let count = (record.NumberParameters as usize).min(15);
                ExceptionSummary {
                    code: record.ExceptionCode as u32,
                    exception_flags: record.ExceptionFlags,
                    parameters: record.ExceptionInformation[..count].to_vec(),
                    ..ExceptionSummary::default()
                }
            })
            .unwrap_or_default();
        if let Some(context) = exception_pointers
            .as_ref()
            .and_then(|pointers| pointers.ContextRecord.as_ref())
        {
            exception.registers = registers_from_context(context);
            exception.eflags = context.EFlags;
        }
        let user_path = USER_PATH
            .lock()
            .map(|path| path.clone())
            .unwrap_or_default();
        let artifacts = crash_artifacts_for(&user_path, None, &exception, Some(exception_pointers));
        let descriptor = LOG_DESCRIPTOR.load(Ordering::SeqCst);
        if descriptor >= 0 {
            crt_write(
                descriptor,
                artifacts.report.as_ptr().cast(),
                artifacts.report.len() as u32,
            );
        }
        if let Ok(body) = std::ffi::CString::new(artifacts.dialog) {
            MessageBoxA(
                0,
                body.as_ptr().cast(),
                c"LegacyClonk crashed".as_ptr().cast(),
                MB_ICONERROR,
            );
        }
        // "Call native exception handler" (:467-468).
        EXCEPTION_CONTINUE_SEARCH
    }

    #[cfg(target_arch = "x86_64")]
    fn registers_from_context(
        context: &windows_sys::Win32::System::Diagnostics::Debug::CONTEXT,
    ) -> X64Registers {
        X64Registers {
            rax: context.Rax as usize,
            rbx: context.Rbx as usize,
            rcx: context.Rcx as usize,
            rdx: context.Rdx as usize,
            rbp: context.Rbp as usize,
            rsi: context.Rsi as usize,
            rdi: context.Rdi as usize,
            r8: context.R8 as usize,
            r9: context.R9 as usize,
            r10: context.R10 as usize,
            r11: context.R11 as usize,
            r12: context.R12 as usize,
            r13: context.R13 as usize,
            r14: context.R14 as usize,
            r15: context.R15 as usize,
            rsp: context.Rsp as usize,
            rip: context.Rip as usize,
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn registers_from_context(
        _context: &windows_sys::Win32::System::Diagnostics::Debug::CONTEXT,
    ) -> X64Registers {
        X64Registers::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4CrashHandlerWin32.cpp:390,410 — the C4ENGINENAME-crash-<UTC>.dmp template.
    #[test]
    fn crash_dump_filename_uses_the_cpp_template() {
        assert_eq!(
            crash_dump_filename("C:\\Users\\a\\Clonk", (2026, 7, 29, 4, 5, 6)).as_deref(),
            Some("C:\\Users\\a\\Clonk\\LegacyClonk-crash-2026-07-29-04-05-06.dmp")
        );
    }

    // :400-404 — a path that already ends in a backslash does not get another.
    #[test]
    fn crash_dump_filename_does_not_double_the_separator() {
        assert_eq!(
            crash_dump_filename("C:\\Clonk\\", (2026, 12, 31, 23, 59, 58)).as_deref(),
            Some("C:\\Clonk\\LegacyClonk-crash-2026-12-31-23-59-58.dmp")
        );
    }

    // :381-384 — no usable user path means no dump at all.
    #[test]
    fn crash_dump_filename_is_absent_without_a_user_path() {
        assert_eq!(crash_dump_filename("", (2026, 7, 29, 0, 0, 0)), None);
    }

    // :97-113 — each known code logs "<NAME>: <sentence>"; unknown codes fall
    // through to the `%#08x` line at :111.
    #[test]
    fn exception_description_names_the_cpp_code_and_sentence() {
        assert_eq!(
            exception_description(EXCEPTION_ACCESS_VIOLATION),
            "EXCEPTION_ACCESS_VIOLATION: The thread tried to read from or write to a virtual \
             address for which it does not have the appropriate access.\n"
        );
        assert_eq!(
            exception_description(EXCEPTION_STACK_OVERFLOW),
            "EXCEPTION_STACK_OVERFLOW: The thread used up its stack.\n"
        );
        assert_eq!(
            exception_description(STATUS_ASSERTION_FAILURE),
            "STATUS_ASSERTION_FAILURE: The thread specified a pre- or postcondition that did not \
             hold.\n"
        );
        assert_eq!(
            exception_description(0x1234_5678),
            "0x12345678: The thread raised an unknown exception.\n"
        );
    }

    // :115-118 — the continuable line keys off EXCEPTION_NONCONTINUABLE alone.
    #[test]
    fn continuable_line_reports_the_exception_flags() {
        assert_eq!(continuable_line(0), "This is a continuable exception.\n");
        assert_eq!(
            continuable_line(EXCEPTION_NONCONTINUABLE),
            "This is a non-continuable exception.\n"
        );
    }

    // :86-95 — the banner names the dump only for non-assertion exceptions that
    // actually produced one.
    #[test]
    fn report_header_names_the_dump_only_when_one_was_written() {
        assert_eq!(
            report_header(EXCEPTION_ACCESS_VIOLATION, Some("C:\\a.dmp")),
            "**********************************************************************\n\
             * UNHANDLED EXCEPTION\n\
             * A crash dump may have been written to C:\\a.dmp\n\
             * If this file exists, please send it to a developer for investigation.\n\
             **********************************************************************\n"
        );
        assert_eq!(
            report_header(EXCEPTION_ACCESS_VIOLATION, None),
            "**********************************************************************\n\
             * UNHANDLED EXCEPTION\n\
             **********************************************************************\n"
        );
        // :89 excludes assertion failures even when a dump exists.
        assert_eq!(
            report_header(STATUS_ASSERTION_FAILURE, Some("C:\\a.dmp")),
            "**********************************************************************\n\
             * UNHANDLED EXCEPTION\n\
             **********************************************************************\n"
        );
    }

    // :126-150 — read/write/DEP wording, the x64 %016zx address, and the
    // EXCEPTION_IN_PAGE_ERROR NTSTATUS tail.
    #[test]
    fn access_violation_detail_matches_the_cpp_wording() {
        assert_eq!(
            access_violation_detail(EXCEPTION_ACCESS_VIOLATION, &[0, 0xDEAD_BEEF]),
            "Additional information for the exception: The thread tried to read from memory at \
             address 0x00000000deadbeef.\n"
        );
        assert_eq!(
            access_violation_detail(EXCEPTION_ACCESS_VIOLATION, &[1, 0x10]),
            "Additional information for the exception: The thread tried to write to memory at \
             address 0x0000000000000010.\n"
        );
        assert_eq!(
            access_violation_detail(EXCEPTION_ACCESS_VIOLATION, &[8, 0x10]),
            "Additional information for the exception: The thread caused an user-mode DEP \
             violation at address 0x0000000000000010.\n"
        );
        assert_eq!(
            access_violation_detail(EXCEPTION_ACCESS_VIOLATION, &[3, 0x10]),
            "Additional information for the exception: The thread tried to access (0x3) memory at \
             address 0x0000000000000010.\n"
        );
        // :127-129 — fewer than two parameters means no detail at all.
        assert_eq!(
            access_violation_detail(EXCEPTION_ACCESS_VIOLATION, &[0]),
            "Additional information for the exception was not provided.\n"
        );
        // :143-148 — the page-error NTSTATUS tail, present and absent.
        assert_eq!(
            access_violation_detail(EXCEPTION_IN_PAGE_ERROR, &[0, 0x10, 0xC0000185]),
            "Additional information for the exception: The thread tried to read from memory at \
             address 0x0000000000000010.\nThe NTSTATUS code that resulted in this exception was \
             0x00000000c0000185.\n"
        );
        assert_eq!(
            access_violation_detail(EXCEPTION_IN_PAGE_ERROR, &[0, 0x10]),
            "Additional information for the exception: The thread tried to read from memory at \
             address 0x0000000000000010.\nThe NTSTATUS code that resulted in this exception was \
             not provided.\n"
        );
        // Codes outside the two switch arms contribute nothing (:120-125,:170).
        assert_eq!(
            access_violation_detail(EXCEPTION_STACK_OVERFLOW, &[0, 0]),
            ""
        );
    }

    // :196-202 — the seven EFLAGS bits, each a letter or a dot.
    #[test]
    fn eflags_line_spells_the_cpp_flag_letters() {
        assert_eq!(eflags_line(0), "EFLAGS: 0x00000000 (.......)\n");
        assert_eq!(
            eflags_line(0x800 | 0x400 | 0x80 | 0x40 | 0x10 | 0x4 | 0x1),
            "EFLAGS: 0x00000cd5 (ODSZAPC)\n"
        );
        // Zero and carry only, in their fixed column positions.
        assert_eq!(eflags_line(0x41), "EFLAGS: 0x00000041 (...Z..C)\n");
    }

    // :168-181 — the x86_64 register block, including the leading-space
    // alignment on " R8" and " R9".
    #[test]
    fn x64_register_lines_match_the_cpp_layout() {
        let registers = X64Registers {
            rax: 1,
            rbx: 2,
            rcx: 3,
            rdx: 4,
            rbp: 5,
            rsi: 6,
            rdi: 7,
            r8: 8,
            r9: 9,
            r10: 10,
            r11: 11,
            r12: 12,
            r13: 13,
            r14: 14,
            r15: 15,
            rsp: 16,
            rip: 17,
        };
        assert_eq!(
            x64_register_lines(&registers),
            "\nProcessor registers (x86_64):\n\
             RAX: 0x0000000000000001, RBX: 0x0000000000000002, RCX: 0x0000000000000003, RDX: 0x0000000000000004\n\
             RBP: 0x0000000000000005, RSI: 0x0000000000000006, RDI: 0x0000000000000007,  R8: 0x0000000000000008\n\
             \x20R9: 0x0000000000000009, R10: 0x000000000000000a, R11: 0x000000000000000b, R12: 0x000000000000000c\n\
             R13: 0x000000000000000d, R14: 0x000000000000000e, R15: 0x000000000000000f\n\
             RSP: 0x0000000000000010, RIP: 0x0000000000000011\n"
        );
    }

    // :427-447 — the message box names whichever artifacts exist.
    #[test]
    fn crash_dialog_text_names_the_generated_artifacts() {
        assert_eq!(
            crash_dialog_text(Some("C:\\Clonk.log"), Some("C:\\a.dmp")),
            "LegacyClonk crashed. Please report this crash together with the following \
             information to the developers:\n\nYou can find detailed information in \
             C:\\Clonk.log.\nA crash dump has been generated at C:\\a.dmp."
        );
        assert_eq!(
            crash_dialog_text(None, None),
            "LegacyClonk crashed. Please report this crash to the developers."
        );
    }

    // C4CrashHandlerWin32.cpp:360-470 — one exception produces the log report,
    // a timestamped minidump under the user path, and the dialog body naming
    // both. The OS-invoked filter itself cannot run in-process without killing
    // the harness, so this drives the same artifact path directly.
    #[cfg(windows)]
    #[test]
    fn windows_unhandled_exception_writes_log_minidump_and_dialog() {
        let user_directory = tempfile::tempdir().expect("temp dir");
        let user_path = user_directory.path().to_str().expect("utf-8 path");
        let registers = X64Registers {
            rip: 0x1234,
            ..X64Registers::default()
        };

        let exception = ExceptionSummary {
            code: EXCEPTION_ACCESS_VIOLATION,
            exception_flags: 0,
            parameters: vec![1, 0xDEAD_BEEF],
            registers,
            eflags: 0x40,
        };
        let artifacts =
            super::crash_artifacts_for(user_path, Some("C:\\Clonk.log"), &exception, None);

        // A timestamped dump was created beneath the user path (:390,410,417).
        let dump_path = artifacts.dump_path.as_deref().expect("dump path");
        let dump = std::path::Path::new(dump_path);
        assert!(dump.is_file(), "no dump written at {dump_path}");
        let name = dump
            .file_name()
            .and_then(|n| n.to_str())
            .expect("dump name");
        assert!(
            name.starts_with("LegacyClonk-crash-") && name.ends_with(".dmp"),
            "unexpected dump name {name}"
        );
        assert_eq!(dump.parent(), Some(user_directory.path()));

        // The report carries banner, code, continuability, fault detail and
        // registers (:86-202), and names the dump (:91).
        assert!(artifacts.report.contains("* UNHANDLED EXCEPTION\n"));
        assert!(artifacts.report.contains(dump_path));
        assert!(artifacts
            .report
            .contains("EXCEPTION_ACCESS_VIOLATION: The thread tried to read from or write to"));
        assert!(artifacts
            .report
            .contains("This is a continuable exception.\n"));
        assert!(artifacts
            .report
            .contains("The thread tried to write to memory at address 0x00000000deadbeef.\n"));
        assert!(artifacts
            .report
            .contains("RSP: 0x0000000000000000, RIP: 0x0000000000001234\n"));
        assert!(artifacts.report.contains("EFLAGS: 0x00000040 (...Z...)\n"));

        // The dialog names both artifacts (:427-447).
        assert_eq!(
            artifacts.dialog,
            format!(
                "LegacyClonk crashed. Please report this crash together with the following \
                 information to the developers:\n\nYou can find detailed information in \
                 C:\\Clonk.log.\nA crash dump has been generated at {dump_path}."
            )
        );
    }
}
