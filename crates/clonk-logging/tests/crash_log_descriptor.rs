#![cfg(windows)]

use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

const RAW_MARKER: &[u8] = b"windows crash descriptor marker\n";
const TRACING_MARKER: &str = "tracing writer survived closing the crash descriptor";

// C4Log.cpp:140-183; C4CrashHandlerWin32.cpp:450-452 — GetLogFD is a binary
// CRT descriptor for the live log, while the FILE stream retains independent
// ownership and remains usable until the log sink is destroyed.
#[test]
fn windows_crash_descriptor_is_binary_and_owns_a_duplicate_handle() {
    let directory = unique_temp_dir();
    let log_path = directory.join("Clonk.log");

    assert_eq!(clonk_logging::crash_log_descriptor(), -1);
    clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the session log");

    let descriptor = clonk_logging::crash_log_descriptor();
    assert!(
        descriptor >= 0,
        "session log did not publish a CRT descriptor"
    );
    let written = unsafe {
        libc::write(
            descriptor,
            RAW_MARKER.as_ptr().cast(),
            RAW_MARKER.len() as u32,
        )
    };
    assert_eq!(written, RAW_MARKER.len() as i32);

    // Closing the crash descriptor must close only its duplicated HANDLE. If
    // `_open_osfhandle` took the tracing File's original HANDLE, this event
    // would disappear because the normal writer would now be invalid.
    assert_eq!(unsafe { libc::close(descriptor) }, 0);
    tracing::info!(TRACING_MARKER);

    let bytes = fs::read(&log_path).expect("read session log");
    assert!(
        bytes
            .windows(RAW_MARKER.len())
            .any(|window| window == RAW_MARKER),
        "CRT descriptor did not preserve binary newlines: {}",
        String::from_utf8_lossy(&bytes)
    );
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(TRACING_MARKER),
        "closing the crash descriptor also closed the tracing writer: {text}"
    );

    let _ = fs::remove_dir_all(directory);
}

fn unique_temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let directory = env::temp_dir().join(format!(
        "clonk-logging-crash-descriptor-{}-{nonce}",
        process::id()
    ));
    fs::create_dir_all(&directory).expect("create test directory");
    directory
}
