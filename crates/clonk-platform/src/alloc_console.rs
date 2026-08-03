//! The classic GUI-build console policy.
//!
//! `C4WinMain.cpp:72-93` allocates a console before normal initialization: for
//! debug GUI builds unconditionally, for release GUI builds only when
//! `/allocconsole` appears in `argv`. A failed allocation returns
//! `C4XRV_Failure`; on success stdin is reopened on `CONIN$` and stdout/stderr
//! on `CONOUT$`.
//!
//! C++ reattaches the CRT's `FILE` streams with `freopen`. Rust's `std::io` does
//! not go through the CRT — it reads the process standard handles — so the port
//! reaches the same observable state by opening the console devices and
//! publishing them with `SetStdHandle`.

/// Whether the classic policy allocates a console for this build and command
/// line (`C4WinMain.cpp:73-82`). `debug_build` is the inverse of `NDEBUG`.
pub fn console_is_required<S: AsRef<str>>(debug_build: bool, arguments: &[S]) -> bool {
    debug_build
        || arguments
            .iter()
            .any(|argument| argument.as_ref() == "/allocconsole")
}

#[cfg(windows)]
pub use windows_impl::{allocate_console, ConsoleError};

#[cfg(windows)]
mod windows_impl {
    use windows_sys::Win32::Foundation::{GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileA, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AllocConsole, SetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    /// Why the classic console bootstrap failed. C++ maps both arms to
    /// `C4XRV_Failure` (`C4WinMain.cpp:85-87`).
    #[derive(Debug, thiserror::Error)]
    pub enum ConsoleError {
        #[error("could not allocate a console (error {0})")]
        Allocate(u32),
        #[error("could not attach {device} to the console (error {code})")]
        AttachStream { device: &'static str, code: u32 },
    }

    /// `AllocConsole` plus the three standard streams (`C4WinMain.cpp:84-92`).
    ///
    /// Like C++, any `AllocConsole` failure returns before the standard streams
    /// are attached (`C4WinMain.cpp:84-87`).
    pub fn allocate_console() -> Result<(), ConsoleError> {
        // SAFETY: `AllocConsole` takes no caller-owned memory.
        if unsafe { AllocConsole() } == 0 {
            // SAFETY: reads this thread's last-error slot.
            return Err(ConsoleError::Allocate(unsafe { GetLastError() }));
        }
        attach_stream("CONIN$", STD_INPUT_HANDLE, "stdin")?;
        attach_stream("CONOUT$", STD_OUTPUT_HANDLE, "stdout")?;
        attach_stream("CONOUT$", STD_ERROR_HANDLE, "stderr")
    }

    /// Opens a console device and publishes it as one standard handle, the
    /// handle-model equivalent of `freopen(device, ..., stream)` (:89-91).
    fn attach_stream(
        device: &str,
        standard_handle: u32,
        name: &'static str,
    ) -> Result<(), ConsoleError> {
        let device_c = std::ffi::CString::new(device).map_err(|_| ConsoleError::AttachStream {
            device: name,
            code: 0,
        })?;
        let access = if standard_handle == STD_INPUT_HANDLE {
            FILE_GENERIC_READ
        } else {
            FILE_GENERIC_WRITE
        };
        // SAFETY: `device_c` outlives the call; the console devices are opened
        // with OPEN_EXISTING so nothing is created.
        let handle: HANDLE = unsafe {
            CreateFileA(
                device_c.as_ptr().cast(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            // SAFETY: reads this thread's last-error slot.
            return Err(ConsoleError::AttachStream {
                device: name,
                code: unsafe { GetLastError() },
            });
        }
        // SAFETY: `handle` is a live console handle owned by the process for
        // the remainder of its life, which is what a standard handle requires.
        let published = unsafe { SetStdHandle(standard_handle, handle) } != 0;
        published.then_some(()).ok_or(ConsoleError::AttachStream {
            device: name,
            // SAFETY: reads this thread's last-error slot.
            code: unsafe { GetLastError() },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4WinMain.cpp:73-82 — release GUI builds need the switch; debug builds
    // allocate unconditionally.
    #[test]
    fn console_policy_matches_the_cpp_build_gates() {
        assert!(!console_is_required(false, &["clonk", "--fullscreen"]));
        assert!(console_is_required(false, &["clonk", "/allocconsole"]));
        // The debug build ignores the command line entirely (:74-82).
        assert!(console_is_required(true, &["clonk"]));
        // The switch is matched whole, the way `std::ranges::find` compares.
        assert!(!console_is_required(false, &["clonk", "/allocconsole=1"]));
        assert!(!console_is_required(false, &["clonk", "allocconsole"]));
    }

    // C4WinMain.cpp:84-92 — a successful request leaves all three standard
    // streams attached to console devices, while a second request fails at
    // `AllocConsole` instead of continuing into the `freopen` calls.
    #[cfg(windows)]
    #[test]
    fn windows_release_allocconsole_attaches_standard_streams() {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_CHAR};
        use windows_sys::Win32::System::Console::{
            FreeConsole, GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
            STD_OUTPUT_HANDLE,
        };

        const STREAMS: [(u32, &str); 3] = [
            (STD_INPUT_HANDLE, "stdin"),
            (STD_OUTPUT_HANDLE, "stdout"),
            (STD_ERROR_HANDLE, "stderr"),
        ];
        const CHILD_MARKER: &str = "LC_ALLOC_CONSOLE_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_some() {
            assert!(console_is_required(false, &["clonk", "/allocconsole"]));
            // SAFETY: reads the three standard-handle slots inherited from the
            // parent test process.
            let saved = STREAMS.map(|(id, _)| unsafe { GetStdHandle(id) });
            // The test harness is a console-subsystem binary and may inherit
            // the runner's console, unlike the shipped GUI executable.
            // SAFETY: the child owns only its association with the console;
            // after this call it is unattached whether or not it had one.
            unsafe { FreeConsole() };
            let attached = super::allocate_console();
            let observed = attached.as_ref().ok().map(|_| {
                STREAMS.map(|(id, _)| {
                    // SAFETY: both calls only inspect handles owned by this
                    // process.
                    unsafe {
                        let handle = GetStdHandle(id);
                        (handle, GetFileType(handle))
                    }
                })
            });
            let duplicate_allocation = attached.as_ref().ok().map(|_| super::allocate_console());

            // SAFETY: restores the standard-handle slots inherited on entry so
            // assertion failures remain visible to the parent test process.
            STREAMS
                .iter()
                .zip(saved)
                .for_each(|((id, _), handle)| unsafe {
                    SetStdHandle(*id, handle);
                });

            attached.expect("console bootstrap");
            assert!(
                matches!(duplicate_allocation, Some(Err(ConsoleError::Allocate(_)))),
                "a second AllocConsole call must return the C++ failure"
            );
            STREAMS
                .iter()
                .zip(observed.expect("attached console handles"))
                .for_each(|((_, name), (handle, file_type))| {
                    assert!(handle != INVALID_HANDLE_VALUE, "{name} handle is invalid");
                    assert!(!handle.is_null(), "{name} handle is null");
                    assert_eq!(
                        file_type, FILE_TYPE_CHAR,
                        "{name} is not attached to a console device"
                    );
                });
            return;
        }

        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "windows_release_allocconsole_attaches_standard_streams",
                "--nocapture",
            ])
            .env(CHILD_MARKER, "1")
            .status()
            .expect("launch console-less child");
        assert!(status.success(), "console-less child failed with {status}");
    }
}
