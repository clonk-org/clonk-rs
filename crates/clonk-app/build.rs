//! Records the target triple this binary was built for, and embeds the Windows
//! icon resources.
//!
//! The updater has to find its own entry in a manifest keyed by target triple,
//! and nothing in `std` reports the triple a build was produced for:
//! `std::env::consts` describes only OS and architecture, which cannot tell a
//! `-gnu` Windows build from an `-msvc` one. Cargo passes it to build scripts,
//! so this is the only place it can be captured.
//!
//! The icons are here for the same reason: a Win32 `RT_GROUP_ICON` resource is a
//! link-time artefact, so no amount of run-time work can give the `.exe` file an
//! icon. This is the port's `src/res/engine.rc`, which CMake appends to the
//! `clonk` target alone (`cmake/filelists/EngineWin32.cmake`).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rustc-env=CLONK_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap_or_default()
    );
    clonk_icon::build::embed_engine_icons();
}
