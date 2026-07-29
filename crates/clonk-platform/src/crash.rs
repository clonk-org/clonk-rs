//! Unix fatal-signal diagnostics.
//!
//! `C4WinMain.cpp:179-213` installs one handler for the classic signal set. It
//! writes the signal name to stderr and to the current log descriptor, dumps up
//! to 100 backtrace frames to both, then restores the default action and
//! reraises so the process keeps its original signal exit status and core-dump
//! behaviour. Everything here runs in signal context, so it uses only
//! async-signal-safe calls: `write(2)`, `backtrace`/`backtrace_symbols_fd`,
//! `signal(2)` and `raise(2)`.

use std::ffi::c_void;

/// The set `C4WinMain` installs (`C4WinMain.cpp:257-264`), in that order.
const HANDLED_SIGNALS: [(i32, &str); 8] = [
    (SIGBUS, "SIGBUS"),
    (SIGILL, "SIGILL"),
    (SIGSEGV, "SIGSEGV"),
    (SIGABRT, "SIGABRT"),
    (SIGINT, "SIGINT"),
    (SIGQUIT, "SIGQUIT"),
    (SIGFPE, "SIGFPE"),
    (SIGTERM, "SIGTERM"),
];

#[cfg(target_os = "macos")]
const SIGBUS: i32 = 10;
#[cfg(target_os = "macos")]
const SIGSEGV: i32 = 11;
#[cfg(target_os = "macos")]
const SIGQUIT: i32 = 3;
#[cfg(not(target_os = "macos"))]
const SIGBUS: i32 = 7;
#[cfg(not(target_os = "macos"))]
const SIGSEGV: i32 = 11;
#[cfg(not(target_os = "macos"))]
const SIGQUIT: i32 = 3;
const SIGILL: i32 = 4;
const SIGABRT: i32 = 6;
const SIGINT: i32 = 2;
const SIGFPE: i32 = 8;
const SIGTERM: i32 = 15;

const STDERR_FILENO: i32 = 2;
const SIG_DFL: usize = 0;
/// `void *stack[100]` (C4WinMain.cpp:203).
const MAX_FRAMES: usize = 100;

type SignalHandler = extern "C" fn(i32);

extern "C" {
    fn write(fd: i32, buf: *const c_void, count: usize) -> isize;
    fn signal(sig: i32, handler: usize) -> usize;
    fn raise(sig: i32) -> i32;
    fn backtrace(buffer: *mut *mut c_void, size: i32) -> i32;
    fn backtrace_symbols_fd(buffer: *const *mut c_void, size: i32, fd: i32);
}

/// Resolves the active session log's descriptor, or a negative sentinel.
/// Set by [`install`] so this crate does not depend on the logging crate.
static LOG_DESCRIPTOR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

fn write_all(fd: i32, bytes: &[u8]) {
    if fd < 0 {
        return;
    }
    let mut written = 0;
    while written < bytes.len() {
        // SAFETY: the slice outlives the call and the length is in range.
        let result = unsafe { write(fd, bytes[written..].as_ptr().cast(), bytes.len() - written) };
        if result <= 0 {
            return;
        }
        written += result as usize;
    }
}

extern "C" fn crash_handler(signo: i32) {
    let name = HANDLED_SIGNALS
        .iter()
        .find(|(candidate, _)| *candidate == signo)
        .map(|(_, name)| *name)
        .unwrap_or("");

    // C++ writes the banner to stderr, then to the log descriptor, then stops.
    let log_fd = LOG_DESCRIPTOR.load(std::sync::atomic::Ordering::Acquire);
    for fd in [STDERR_FILENO, log_fd] {
        if fd < 0 {
            continue;
        }
        write_all(fd, crate::PRODUCT_NAME.as_bytes());
        write_all(fd, b": Caught signal ");
        write_all(fd, name.as_bytes());
        write_all(fd, b"\n");
    }

    let mut frames = [std::ptr::null_mut::<c_void>(); MAX_FRAMES];
    // SAFETY: `frames` is a live buffer of exactly MAX_FRAMES entries, and
    // `backtrace`/`backtrace_symbols_fd` are async-signal-safe.
    unsafe {
        let count = backtrace(frames.as_mut_ptr(), MAX_FRAMES as i32);
        if count > 0 {
            backtrace_symbols_fd(frames.as_ptr(), count, STDERR_FILENO);
            if log_fd >= 0 {
                backtrace_symbols_fd(frames.as_ptr(), count, log_fd);
            }
        }
        // Restore the default action and reraise, so the process exits with the
        // original signal status and dumps core if it would have
        // (C4WinMain.cpp:210-211).
        signal(signo, SIG_DFL);
        raise(signo);
    }
}

/// Installs the classic handlers. Call before application initialization
/// (C4WinMain.cpp:256-265). `log_descriptor` is the session log's raw fd, or a
/// negative value when there is no log yet.
pub fn install(log_descriptor: i32) {
    LOG_DESCRIPTOR.store(log_descriptor, std::sync::atomic::Ordering::Release);
    let handler: SignalHandler = crash_handler;
    for (signo, _) in HANDLED_SIGNALS {
        // SAFETY: `crash_handler` has the C signal-handler signature and uses
        // only async-signal-safe calls.
        unsafe {
            signal(signo, handler as usize);
        }
    }
}

/// Updates the descriptor the handler writes to, once the session log exists.
pub fn set_log_descriptor(log_descriptor: i32) {
    LOG_DESCRIPTOR.store(log_descriptor, std::sync::atomic::Ordering::Release);
}

/// The signal numbers this platform installs, for tests and diagnostics.
pub fn handled_signals() -> impl Iterator<Item = (i32, &'static str)> {
    HANDLED_SIGNALS.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::process::{Command, Stdio};

    /// The exact set and order `C4WinMain` installs (C4WinMain.cpp:257-264).
    #[test]
    fn handled_signal_set_matches_cpp() {
        assert_eq!(
            handled_signals().map(|(_, name)| name).collect::<Vec<_>>(),
            vec![
                "SIGBUS", "SIGILL", "SIGSEGV", "SIGABRT", "SIGINT", "SIGQUIT", "SIGFPE", "SIGTERM"
            ]
        );
    }

    /// A handled fatal signal writes its name and a backtrace to stderr and to
    /// the log descriptor, then restores the default action and reraises so the
    /// process still dies *from that signal* rather than exiting normally
    /// (C4WinMain.cpp:179-213).
    #[test]
    fn unix_fatal_signal_writes_diagnostics_then_reraises() {
        // Re-exec this test binary as the crashing child.
        if std::env::var("LC_CRASH_HANDLER_CHILD").is_ok() {
            install(1); // stdout doubles as the "log" descriptor for the child
                        // SAFETY: deliberately raising a handled signal.
            unsafe {
                raise(SIGABRT);
            }
            unreachable!("the handler must reraise and terminate the process");
        }

        let mut child = Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "crash::tests::unix_fatal_signal_writes_diagnostics_then_reraises",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("LC_CRASH_HANDLER_CHILD", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the crashing child");
        let mut stdout = String::new();
        let mut stderr = String::new();
        child
            .stdout
            .take()
            .expect("child stdout")
            .read_to_string(&mut stdout)
            .expect("read child stdout");
        child
            .stderr
            .take()
            .expect("child stderr")
            .read_to_string(&mut stderr)
            .expect("read child stderr");
        let status = child.wait().expect("await the crashing child");

        // The banner names the product and the signal, on both destinations.
        let banner = format!("{}: Caught signal SIGABRT", crate::PRODUCT_NAME);
        assert!(
            stderr.contains(&banner),
            "stderr banner missing.\nstatus={status:?}\nstdout={stdout}\nstderr={stderr}"
        );
        assert!(
            stdout.contains(&banner),
            "log-descriptor banner missing: {stdout}"
        );
        // A best-effort backtrace follows it on stderr. `backtrace_symbols_fd`
        // emits one frame per line; require more than the banner alone.
        assert!(
            stderr.lines().count() > 1,
            "no backtrace frames on stderr: {stderr}"
        );

        // The default action was restored and the signal reraised, so the
        // child died from SIGABRT rather than exiting.
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(SIGABRT),
            "child must die from the original signal, got {status:?}"
        );
        assert!(status.code().is_none());
    }
}
