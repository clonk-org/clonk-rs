//! Embeds the product icon into a Windows executable, from a build script.
//!
//! An `.exe` shows an icon in Explorer, on a shortcut, on a pinned taskbar
//! button and in Add/Remove Programs only if it carries a Win32 `RT_GROUP_ICON`
//! resource. That is a link-time artefact: nothing a running program does can
//! reach it, which is why the window icon `startup_window_builder` attaches
//! covers the title bar and the taskbar *button* while leaving the file itself
//! iconless.
//!
//! Behind the `build-script` feature so the thirteen file-class `.ico` files are
//! embedded in the build script rather than in the shipped binary. Cargo's v2
//! resolver keeps build-dependency features separate from normal ones, so a
//! crate that also depends on `clonk-icon` at run time does not pay for them.

use std::path::{Path, PathBuf};

/// The file-class icons, recovered from the C++ `src/res` at the pinned
/// snapshot. Embedded because a build script cannot assume where the workspace
/// root is relative to the crate it is building.
const FILE_CLASS_ICONS: [(&str, &[u8]); 13] = [
    ("c4s.ico", include_bytes!("../res/windows/c4s.ico")),
    ("c4g.ico", include_bytes!("../res/windows/c4g.ico")),
    ("c4f.ico", include_bytes!("../res/windows/c4f.ico")),
    ("c4p.ico", include_bytes!("../res/windows/c4p.ico")),
    ("c4x.ico", include_bytes!("../res/windows/c4x.ico")),
    ("c4d.ico", include_bytes!("../res/windows/c4d.ico")),
    ("c4i.ico", include_bytes!("../res/windows/c4i.ico")),
    ("c4m.ico", include_bytes!("../res/windows/c4m.ico")),
    ("c4b.ico", include_bytes!("../res/windows/c4b.ico")),
    ("c4v.ico", include_bytes!("../res/windows/c4v.ico")),
    ("c4l.ico", include_bytes!("../res/windows/c4l.ico")),
    ("c4k.ico", include_bytes!("../res/windows/c4k.ico")),
    ("c4u.ico", include_bytes!("../res/windows/c4u.ico")),
];

/// The generated application icon's name inside `OUT_DIR`.
const APP_ICON_FILE: &str = "clonk-rust.ico";

/// Embeds the application icon alone.
///
/// For the binaries with no C++ counterpart — the launcher has no `clonk.exe`
/// equivalent — which still need Explorer and the Start Menu shortcut to show
/// the product mark.
pub fn embed_app_icon() {
    embed(&crate::WINDOWS_ICON_RESOURCES[..1]);
}

/// Embeds the application icon plus the thirteen file-class icons, in
/// [`crate::WINDOWS_ICON_RESOURCES`] order.
///
/// This mirrors `src/res/engine.rc`, which CMake appends to the `clonk` target
/// only (`cmake/filelists/EngineWin32.cmake`), so it belongs to the engine
/// binary alone. The order is what makes the `DefaultIcon` ordinals
/// `clonk-platform` registers resolve to the right pictures.
pub fn embed_engine_icons() {
    embed(&crate::WINDOWS_ICON_RESOURCES);
}

fn embed(resources: &[crate::WindowsIconResource]) {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }

    let out_dir = PathBuf::from(
        std::env::var_os("OUT_DIR").expect("cargo always sets OUT_DIR for a build script"),
    );
    let script = out_dir.join("icons.rc");
    write_icons(&out_dir, resources);
    std::fs::write(&script, resource_script(resources))
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", script.display()));

    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile the Windows icon resources");
}

/// Stages every `.ico` beside the script that names them. `rc.exe` and `windres`
/// both resolve a relative `ICON` path against the script's own directory, and a
/// path with a space or a backslash in it is a portability hazard in an `.rc`.
fn write_icons(out_dir: &Path, resources: &[crate::WindowsIconResource]) {
    resources.iter().for_each(|resource| match resource.file {
        Some(name) => {
            let bytes = FILE_CLASS_ICONS
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, bytes)| *bytes)
                .unwrap_or_else(|| panic!("{name} is in the resource table but not shipped"));
            write(out_dir.join(name), bytes);
        }
        None => {
            let ico = crate::app_ico_bytes().expect("the product logo encodes as an .ico");
            write(out_dir.join(APP_ICON_FILE), &ico);
        }
    });
}

fn write(path: PathBuf, bytes: &[u8]) {
    std::fs::write(&path, bytes)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

/// The `.rc` source, mirroring `src/res/engine.rc:30-46` — including its comment,
/// which is the reason slot 0 comes first.
fn resource_script(resources: &[crate::WindowsIconResource]) -> String {
    let entries = resources
        .iter()
        .map(|resource| {
            format!(
                "{} ICON DISCARDABLE \"{}\"\n",
                resource.id,
                resource.file.unwrap_or(APP_ICON_FILE)
            )
        })
        .collect::<String>();
    format!(
        "// Generated by clonk-icon. Mirrors src/res/engine.rc.\n\
         // Icon with lowest ID value placed first to ensure application icon\n\
         // remains consistent on all systems.\n\
         {entries}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The script is what `rc.exe` reads, so its text is the real contract.
    #[test]
    fn the_generated_script_lists_every_resource_in_table_order() {
        let script = resource_script(&crate::WINDOWS_ICON_RESOURCES);

        let ids: Vec<&str> = script
            .lines()
            .filter(|line| line.contains(" ICON DISCARDABLE "))
            .map(|line| line.split_whitespace().next().unwrap_or_default())
            .collect();
        assert_eq!(ids.first(), Some(&"4000"), "the app icon must come first");
        assert_eq!(ids.len(), crate::WINDOWS_ICON_RESOURCES.len());
        assert!(
            ids.windows(2).all(|pair| pair[0] < pair[1]),
            "resource ids must ascend, or the DefaultIcon ordinals shift"
        );
        assert!(
            script.contains(&format!("4000 ICON DISCARDABLE \"{APP_ICON_FILE}\"")),
            "slot 0 must name the generated icon, not a shipped file"
        );
    }

    // A table entry naming a file that is not embedded would panic mid-build on
    // Windows only, which is the worst place to find out.
    #[test]
    fn every_table_entry_has_bytes_behind_it() {
        crate::WINDOWS_ICON_RESOURCES
            .iter()
            .filter_map(|resource| resource.file)
            .for_each(|name| {
                assert!(
                    FILE_CLASS_ICONS
                        .iter()
                        .any(|(candidate, bytes)| *candidate == name && !bytes.is_empty()),
                    "{name} is in the resource table but not shipped"
                );
            });
    }

    // Every recovered file must be a real ICONDIR, or `rc.exe` rejects it.
    #[test]
    fn every_shipped_file_class_icon_is_a_valid_icon_container() {
        FILE_CLASS_ICONS.iter().for_each(|(name, bytes)| {
            assert_eq!(&bytes[..4], &[0, 0, 1, 0], "{name} is not an ICONDIR");
            let entries = u16::from_le_bytes([bytes[4], bytes[5]]);
            assert!(entries > 0, "{name} carries no images");
            (0..usize::from(entries)).for_each(|index| {
                let entry = &bytes[6 + index * 16..6 + (index + 1) * 16];
                let length =
                    u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
                let offset =
                    u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
                assert!(
                    offset + length <= bytes.len(),
                    "{name} entry {index} points past the end of the file"
                );
            });
        });
    }
}
