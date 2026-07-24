//! Console attachment for the Windows GUI subsystem.
//!
//! Both binaries are linked as `windows_subsystem = "windows"` so launching
//! them from Explorer does not flash a console window behind the game. That
//! subsystem also detaches stdio when the binary *is* started from a terminal,
//! which would silently swallow `--help` and the automation reports, so the
//! process reattaches to its parent console when one exists.

/// Reattach stdio to the parent console, if this process was started from one.
///
/// Does nothing on platforms without the Windows console model, and nothing
/// when the process has no parent console (the Explorer double-click case).
pub fn attach_parent_console() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

        // SAFETY: `AttachConsole` takes only a process id and touches no memory
        // owned by this crate. It returns zero when the parent has no console,
        // which is the ordinary GUI-launch case and needs no handling.
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
