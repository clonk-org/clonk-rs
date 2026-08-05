//! Unix fatal-signal diagnostics.
//!
//! `C4WinMain.cpp:179-213` installs one handler for the classic signal set. It
//! writes the signal name to stderr and to the current log descriptor, dumps up
//! to 100 backtrace frames to both, then restores the default action and
//! reraises so the process keeps its original signal exit status and core-dump
//! behaviour. Everything here runs in signal context, so it uses only
//! async-signal-safe calls: `write(2)`, `backtrace`/`backtrace_symbols_fd`,
//! `signal(2)` and `raise(2)`. `backtrace` earns that description only after
//! [`install`] has resolved it once — glibc loads it from libgcc on first use.
//!
//! The handlers themselves go in with `sigaction(2)` and `SA_ONSTACK` rather
//! than C++'s `signal(2)`, so that a stack overflow can still be reported; see
//! [`install`].

use std::ffi::c_void;

/// The set `C4WinMain` installs (`C4WinMain.cpp:257-264`), in that order.
const HANDLED_SIGNALS: [(i32, &str); 8] = [
    (libc::SIGBUS, "SIGBUS"),
    (libc::SIGILL, "SIGILL"),
    (libc::SIGSEGV, "SIGSEGV"),
    (libc::SIGABRT, "SIGABRT"),
    (libc::SIGINT, "SIGINT"),
    (libc::SIGQUIT, "SIGQUIT"),
    (libc::SIGFPE, "SIGFPE"),
    (libc::SIGTERM, "SIGTERM"),
];

/// `void *stack[100]` (C4WinMain.cpp:203).
const MAX_FRAMES: usize = 100;

// `backtrace(3)` lives in glibc's `execinfo.h`, which the libc crate declares
// only for Apple and the BSDs, so these two stay hand-written.
extern "C" {
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
        let result =
            unsafe { libc::write(fd, bytes[written..].as_ptr().cast(), bytes.len() - written) };
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
    for fd in [libc::STDERR_FILENO, log_fd] {
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
            backtrace_symbols_fd(frames.as_ptr(), count, libc::STDERR_FILENO);
            if log_fd >= 0 {
                backtrace_symbols_fd(frames.as_ptr(), count, log_fd);
            }
        }
        // Restore the default action and reraise, so the process exits with the
        // original signal status and dumps core if it would have
        // (C4WinMain.cpp:210-211).
        libc::signal(signo, libc::SIG_DFL);
        libc::raise(signo);
    }
}

/// Room for the handler to run when the ordinary stack is gone. The banner
/// plus a 100-frame `backtrace_symbols_fd` measured under 5 KiB; the margin is
/// for the deeper symbol resolution a release build with more shared objects
/// can need. Linux x86-64's `SIGSTKSZ` is only 8 KiB, so it is not enough on
/// its own, and macOS refuses anything under its 32 KiB `MINSIGSTKSZ`.
const ALTERNATE_STACK_SIZE: usize = 128 * 1024;

/// Give this thread a stack the handler can run on when its own is exhausted.
///
/// Leaked deliberately: it has to outlive every frame in the process, and it
/// is allocated exactly once.
///
/// `sigaltstack(2)` is per-thread, so this covers the thread that installs —
/// the one running the engine and the script VM, and so the one an overflow
/// is most likely to happen on. Worker threads keep the alternate stack the
/// Rust runtime gives them, which is smaller but still several times the
/// handler's measured high-water mark; it keeps installing one per thread
/// because it latches that decision before `main`, ahead of us. Threads Rust
/// did not create — an audio or GPU-driver callback — have no alternate stack
/// from anyone and stay mute, exactly as they are under stock Rust.
fn install_alternate_signal_stack() {
    let size = ALTERNATE_STACK_SIZE.max(libc::MINSIGSTKSZ);
    let stack = Box::leak(vec![0u8; size].into_boxed_slice());
    let descriptor = libc::stack_t {
        ss_sp: stack.as_mut_ptr().cast(),
        ss_size: size,
        ss_flags: 0,
    };
    // SAFETY: `descriptor` borrows a leaked allocation of exactly `ss_size`
    // bytes, so the kernel's reference to it stays valid for the process.
    unsafe {
        libc::sigaltstack(&descriptor, std::ptr::null_mut());
    }
}

/// Resolve `backtrace` before a fault needs it.
///
/// glibc keeps it in libgcc and loads that on first use, which allocates — so
/// the first call is the one call that is *not* async-signal-safe. Making it
/// here leaves only resolved code on the crash path.
fn prewarm_backtrace() {
    let mut frames = [std::ptr::null_mut::<c_void>(); 8];
    // SAFETY: `frames` is a live buffer of exactly the length passed.
    unsafe {
        backtrace(frames.as_mut_ptr(), frames.len() as i32);
    }
}

/// Installs the classic handlers. Call before application initialization
/// (C4WinMain.cpp:256-265). `log_descriptor` is the session log's raw fd, or a
/// negative value when there is no log yet.
///
/// C++ installs these with plain `signal(2)` (C4WinMain.cpp:257-264) and this
/// deliberately does not. A handler with no alternate stack cannot be entered
/// once the stack is exhausted, so `signal(2)` makes a stack overflow — the
/// one fault a deeply recursive engine can actually hit — kill the process
/// having written nothing at all. Worse than inheriting C++'s behaviour: it
/// also *replaces* the `SA_ONSTACK` handler the Rust runtime installs before
/// `main`, so the port loses the `has overflowed its stack` line stock Rust
/// would have printed (clonk-org/clonk-rs#40). `SA_ONSTACK` restores the
/// banner without changing which signals are handled or what is written.
pub fn install(log_descriptor: i32) {
    LOG_DESCRIPTOR.store(log_descriptor, std::sync::atomic::Ordering::Release);
    prewarm_backtrace();
    install_alternate_signal_stack();
    let handler: extern "C" fn(i32) = crash_handler;
    for (signo, _) in HANDLED_SIGNALS {
        // SAFETY: `crash_handler` has the C signal-handler signature and uses
        // only async-signal-safe calls. The mask mirrors what `signal(2)`
        // would have set, so the only change is where the handler runs.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handler as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaddset(&mut action.sa_mask, signo);
            action.sa_flags = libc::SA_ONSTACK | libc::SA_RESTART;
            libc::sigaction(signo, &action, std::ptr::null_mut());
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

    /// Consume stack until the guard page faults. `black_box` and the live
    /// padding keep the optimizer from turning this into a loop or eliding the
    /// frames, which it otherwise does at the profile the tests build with.
    #[allow(
        unconditional_recursion,
        reason = "overflowing the stack is the fixture"
    )]
    fn exhaust_the_stack(depth: u64) -> u64 {
        let padding = std::hint::black_box([depth; 512]);
        std::hint::black_box(exhaust_the_stack(depth + 1)) + padding[511]
    }

    /// A stack overflow must reach the banner like any other fatal signal.
    ///
    /// It is the one fault the handler cannot report from the exhausted stack
    /// itself, so without an alternate stack the kernel cannot enter the
    /// handler at all and the process dies having written nothing — no banner,
    /// no backtrace, and not even the `has overflowed its stack` line the Rust
    /// runtime would have printed had we left its handler in place. That is
    /// exactly the "it just vanished, and the log simply stops" signature of
    /// clonk-org/clonk-rs#40.
    #[test]
    fn unix_stack_overflow_still_reaches_the_crash_banner() {
        // Re-exec this test binary as the overflowing child.
        if std::env::var("LC_CRASH_OVERFLOW_CHILD").is_ok() {
            install(1); // stdout doubles as the "log" descriptor for the child
            println!("overflowed to {}", exhaust_the_stack(0));
            unreachable!("the recursion must fault before it returns");
        }

        let output = Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "crash::tests::unix_stack_overflow_still_reaches_the_crash_banner",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("LC_CRASH_OVERFLOW_CHILD", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("run the overflowing child");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // The guard-page fault is SIGSEGV on the main thread; macOS reports
        // some thread-stack guard faults as SIGBUS. Either is a handled signal
        // and either must be named.
        let banner = format!("{}: Caught signal ", crate::PRODUCT_NAME);
        assert!(
            stderr.contains(&banner),
            "a stack overflow wrote no banner to stderr.\n\
             status={:?}\nstdout={stdout}\nstderr={stderr}",
            output.status
        );
        assert!(
            stdout.contains(&banner),
            "a stack overflow wrote no banner to the log descriptor: {stdout}"
        );
        assert!(
            stderr.contains("SIGSEGV") || stderr.contains("SIGBUS"),
            "the banner must name the fault: {stderr}"
        );
        // The backtrace is the diagnosis here: the repeated frame names the
        // runaway recursion.
        assert!(
            stderr.lines().count() > 1,
            "no backtrace frames on stderr: {stderr}"
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
                libc::raise(libc::SIGABRT);
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
            Some(libc::SIGABRT),
            "child must die from the original signal, got {status:?}"
        );
        assert!(status.code().is_none());
    }
}
