//! Embeds the Windows icon resource.
//!
//! A Win32 `RT_GROUP_ICON` resource is a link-time artefact, so nothing the
//! launcher does at run time can give its `.exe` an icon — and this is the
//! binary a Windows user actually sees: `scripts/windows-installer.nsi` points
//! the Start Menu shortcut and the Add/Remove Programs `DisplayIcon` at it, both
//! of which read the icon out of the executable itself.
//!
//! Only the application icon, not the file-class set: C++ has no launcher, and
//! `clonk-platform` registers its `DefaultIcon` ordinals against the engine
//! binary, so the thirteen file-class icons belong there alone.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    clonk_icon::build::embed_app_icon();
}
