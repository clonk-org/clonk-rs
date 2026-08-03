//! Windows file associations and the `clonk:` URL protocol.
//!
//! `C4Application.cpp:219-223` registers these best-effort during graphical
//! startup; a failure never stops the engine. `C4FileClasses.cpp:28-72` lists
//! the eleven classes, the protocol, the update verb and the AppUserModelId,
//! and `StdRegistry.cpp:224-279` defines the key and value shapes.
//!
//! Every entry lands under `HKEY_CLASSES_ROOT` (`SetRegClassesRoot`). The
//! composition below is host-independent so it can be asserted anywhere; only
//! the write is Windows-gated.

/// `C4FileClassContentType` (`C4FileClasses.cpp:26`).
const GROUP_CONTENT_TYPE: &str = "application/vnd.clonk.c4group";

/// `STD_APPUSERMODELID` (`CMakeLists.txt:731`).
pub const APP_USER_MODEL_ID: &str = "LegacyClonkTeam.LegacyClonk";

/// One `HKEY_CLASSES_ROOT` string value to write. `name` is `None` for a key's
/// default value, which is what `SetRegClassesRoot` writes with a null name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryStringValue {
    pub key: String,
    pub name: Option<&'static str>,
    pub data: String,
}

impl RegistryStringValue {
    fn default_value(key: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: None,
            data: data.into(),
        }
    }

    fn named(key: impl Into<String>, name: &'static str, data: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: Some(name),
            data: data.into(),
        }
    }
}

/// One row of the `SetC4FileClasses` table (`C4FileClasses.cpp:47-58`).
struct FileClass {
    prog_id: &'static str,
    extension: &'static str,
    description: &'static str,
    icon_index: i32,
    content_type: &'static str,
}

/// The eleven classes in `SetC4FileClasses` order. The icon indices are the
/// engine's own resource ordinals, so they are reproduced exactly.
const FILE_CLASSES: [FileClass; 11] = [
    FileClass {
        prog_id: "Clonk4.Scenario",
        extension: "c4s",
        description: "Clonk 4 Scenario",
        icon_index: 1,
        content_type: GROUP_CONTENT_TYPE,
    },
    FileClass {
        prog_id: "Clonk4.Group",
        extension: "c4g",
        description: "Clonk 4 Group",
        icon_index: 2,
        content_type: GROUP_CONTENT_TYPE,
    },
    FileClass {
        prog_id: "Clonk4.Folder",
        extension: "c4f",
        description: "Clonk 4 Folder",
        icon_index: 3,
        content_type: GROUP_CONTENT_TYPE,
    },
    FileClass {
        prog_id: "Clonk4.Player",
        extension: "c4p",
        description: "Clonk 4 Player",
        icon_index: 4,
        content_type: GROUP_CONTENT_TYPE,
    },
    FileClass {
        prog_id: "Clonk4.Definition",
        extension: "c4d",
        description: "Clonk 4 Object Definition",
        icon_index: 6,
        content_type: GROUP_CONTENT_TYPE,
    },
    FileClass {
        prog_id: "Clonk4.Object",
        extension: "c4i",
        description: "Clonk 4 Object Info",
        icon_index: 7,
        content_type: GROUP_CONTENT_TYPE,
    },
    FileClass {
        prog_id: "Clonk4.Material",
        extension: "c4m",
        description: "Clonk 4 Material",
        icon_index: 8,
        content_type: "text/plain",
    },
    FileClass {
        prog_id: "Clonk4.Binary",
        extension: "c4b",
        description: "Clonk 4 Binary",
        icon_index: 9,
        content_type: "application/octet-stream",
    },
    FileClass {
        prog_id: "Clonk4.Video",
        extension: "c4v",
        description: "Clonk 4 Video",
        icon_index: 10,
        content_type: "video/avi",
    },
    FileClass {
        prog_id: "Clonk4.Weblink",
        extension: "c4l",
        description: "Clonk 4 Weblink",
        icon_index: 11,
        content_type: GROUP_CONTENT_TYPE,
    },
    FileClass {
        prog_id: "Clonk4.Update",
        extension: "c4u",
        description: "Clonk 4 Update",
        icon_index: 13,
        content_type: GROUP_CONTENT_TYPE,
    },
];

/// `SetRegFileClass` (`StdRegistry.cpp:257-279`): the class name, its icon, the
/// extension mapping and the extension's content type.
fn file_class_values(class: &FileClass, engine_path: &str) -> Vec<RegistryStringValue> {
    let extension_key = format!(".{}", class.extension);
    vec![
        RegistryStringValue::default_value(class.prog_id, class.description),
        RegistryStringValue::default_value(
            format!("{}\\DefaultIcon", class.prog_id),
            format!("{engine_path},{}", class.icon_index),
        ),
        RegistryStringValue::default_value(extension_key.clone(), class.prog_id),
        RegistryStringValue::named(extension_key, "Content Type", class.content_type),
    ]
}

/// `SetProtocol` (`C4FileClasses.cpp:28-44`). The command is `"{module} %1"`
/// and the icon is the module's second resource (`,1`).
fn protocol_values(protocol: &str, engine_path: &str) -> Vec<RegistryStringValue> {
    vec![
        RegistryStringValue::default_value(protocol, "URL: Protocol"),
        RegistryStringValue::named(protocol, "URL Protocol", ""),
        RegistryStringValue::default_value(
            format!("{protocol}\\shell\\open\\command"),
            format!("{engine_path} %1"),
        ),
        RegistryStringValue::default_value(
            format!("{protocol}\\DefaultIcon"),
            format!("{engine_path},1"),
        ),
    ]
}

/// `SetRegShell(..., fMakeDefault = true)` (`StdRegistry.cpp:224-243`).
fn shell_verb_values(
    prog_id: &str,
    verb: &'static str,
    caption: &'static str,
    command: String,
) -> Vec<RegistryStringValue> {
    vec![
        RegistryStringValue::default_value(format!("{prog_id}\\Shell\\{verb}"), caption),
        RegistryStringValue::default_value(format!("{prog_id}\\Shell\\{verb}\\Command"), command),
        // fMakeDefault: the class's Shell default names the verb (:240-241).
        RegistryStringValue::default_value(format!("{prog_id}\\Shell"), verb),
    ]
}

/// Every `HKEY_CLASSES_ROOT` value `SetC4FileClasses` writes, in C++'s order
/// (`C4FileClasses.cpp:46-71`). `engine_path` is the executable's full path.
pub fn file_class_registry_values(engine_path: &str) -> Vec<RegistryStringValue> {
    let mut values: Vec<RegistryStringValue> = FILE_CLASSES
        .iter()
        .flat_map(|class| file_class_values(class, engine_path))
        .collect();
    values.extend(protocol_values("clonk", engine_path));
    // The c4u application verb, quoted because it carries a path (:63-65).
    values.extend(shell_verb_values(
        "Clonk4.Update",
        "Update",
        "Update",
        format!("\"{engine_path}\" \"%1\""),
    ));
    values.push(RegistryStringValue::named(
        format!("AppUserModelId\\{APP_USER_MODEL_ID}"),
        "DisplayName",
        crate::paths::ENGINE_CAPTION,
    ));
    values
}

/// The `HKEY_CLASSES_ROOT` keys the registration creates, deepest first so a
/// parent is only removed once its children are gone. `RegDeleteKey` does not
/// delete a key that still has subkeys.
pub fn file_class_registry_keys_for_removal(engine_path: &str) -> Vec<String> {
    let mut keys: Vec<String> = file_class_registry_values(engine_path)
        .into_iter()
        .map(|value| value.key)
        .collect();
    keys.sort();
    keys.dedup();
    // Deepest first: more separators means deeper. `sort_by_key` is stable, so
    // the alphabetical order established above survives within each depth.
    keys.sort_by_key(|key| std::cmp::Reverse(key.matches('\\').count()));
    keys
}

/// The stale `App Paths` key `SetC4FileClasses` deletes under
/// `HKEY_LOCAL_MACHINE` (`C4FileClasses.cpp:68`).
pub const STALE_APP_PATHS_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\Clonk.exe";

#[cfg(windows)]
pub use windows_impl::{register_file_classes, unregister_file_classes};

#[cfg(windows)]
mod windows_impl {
    use super::{file_class_registry_values, RegistryStringValue};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExA, RegDeleteKeyA, RegSetValueExA, HKEY, HKEY_CLASSES_ROOT,
        KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    /// `SetC4FileClasses` (`C4FileClasses.cpp:46-72`), called best-effort during
    /// startup (`C4Application.cpp:219-223`). Returns whether every value was
    /// written; a failure is reported, never fatal.
    pub fn register_file_classes(engine_path: &str) -> bool {
        file_class_registry_values(engine_path)
            .iter()
            .all(write_classes_root_value)
    }

    /// Removes the registration, deepest key first. A key that is already
    /// absent is not a failure — the operation is idempotent, which is what a
    /// `c4group -u` run over a partly-registered machine needs.
    pub fn unregister_file_classes(engine_path: &str) -> bool {
        super::file_class_registry_keys_for_removal(engine_path)
            .iter()
            .map(|key| delete_classes_root_key(key))
            .collect::<Vec<bool>>()
            .into_iter()
            .all(|removed| removed)
    }

    fn delete_classes_root_key(key: &str) -> bool {
        let Ok(key_name) = std::ffi::CString::new(key) else {
            return false;
        };
        // SAFETY: `key_name` outlives the call.
        let removed = unsafe { RegDeleteKeyA(HKEY_CLASSES_ROOT, key_name.as_ptr().cast()) };
        // ERROR_FILE_NOT_FOUND means it was never there, which is success here.
        removed == ERROR_SUCCESS || removed == ERROR_FILE_NOT_FOUND
    }

    /// `SetRegClassesRoot`: create the key under `HKEY_CLASSES_ROOT` and write
    /// one `REG_SZ` value (`StdRegistry.cpp`).
    fn write_classes_root_value(value: &RegistryStringValue) -> bool {
        let (Ok(key_name), Ok(data)) = (
            std::ffi::CString::new(value.key.as_str()),
            std::ffi::CString::new(value.data.as_str()),
        ) else {
            return false;
        };
        let name = value.name.map(std::ffi::CString::new).transpose();
        let Ok(name) = name else {
            return false;
        };
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `key_name` outlives the call and `key` is a stack out-param.
        let opened = unsafe {
            RegCreateKeyExA(
                HKEY_CLASSES_ROOT,
                key_name.as_ptr().cast(),
                0,
                std::ptr::null_mut(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if opened != ERROR_SUCCESS {
            return false;
        }
        // The written length includes the terminating NUL, as REG_SZ requires.
        let bytes = data.as_bytes_with_nul();
        // SAFETY: `key` is live until closed below; `bytes` outlives the call.
        let written = unsafe {
            RegSetValueExA(
                key,
                name.as_ref()
                    .map_or(std::ptr::null(), |name| name.as_ptr().cast()),
                0,
                REG_SZ,
                bytes.as_ptr(),
                bytes.len() as u32,
            )
        };
        // SAFETY: `key` came from RegCreateKeyExA and is not used afterwards.
        unsafe { RegCloseKey(key) };
        written == ERROR_SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4FileClasses.cpp:46-71; StdRegistry.cpp:224-279 — the exact keys and
    // values C++ writes under HKEY_CLASSES_ROOT.
    #[test]
    fn windows_file_classes_match_the_native_registry_entries() {
        let values = file_class_registry_values("C:\\Games\\Clonk\\clonk.exe");
        let find = |key: &str, name: Option<&str>| {
            values
                .iter()
                .find(|value| value.key == key && value.name == name)
                .map(|value| value.data.as_str())
        };

        // A class writes its name, icon, extension mapping and content type.
        assert_eq!(find("Clonk4.Scenario", None), Some("Clonk 4 Scenario"));
        assert_eq!(
            find("Clonk4.Scenario\\DefaultIcon", None),
            Some("C:\\Games\\Clonk\\clonk.exe,1")
        );
        assert_eq!(find(".c4s", None), Some("Clonk4.Scenario"));
        assert_eq!(
            find(".c4s", Some("Content Type")),
            Some("application/vnd.clonk.c4group")
        );

        // The three classes whose content type is not the group type (:53-55).
        assert_eq!(find(".c4m", Some("Content Type")), Some("text/plain"));
        assert_eq!(
            find(".c4b", Some("Content Type")),
            Some("application/octet-stream")
        );
        assert_eq!(find(".c4v", Some("Content Type")), Some("video/avi"));

        // Icon ordinals 5 and 12 are deliberately skipped by C++ (:47-58).
        assert_eq!(
            find("Clonk4.Definition\\DefaultIcon", None),
            Some("C:\\Games\\Clonk\\clonk.exe,6")
        );
        assert_eq!(
            find("Clonk4.Update\\DefaultIcon", None),
            Some("C:\\Games\\Clonk\\clonk.exe,13")
        );

        // All eleven extensions are mapped (:47-58).
        for extension in [
            ".c4s", ".c4g", ".c4f", ".c4p", ".c4d", ".c4i", ".c4m", ".c4b", ".c4v", ".c4l", ".c4u",
        ] {
            assert!(
                find(extension, None).is_some(),
                "{extension} has no class mapping"
            );
        }

        // The clonk: protocol (:28-44,:60).
        assert_eq!(find("clonk", None), Some("URL: Protocol"));
        assert_eq!(find("clonk", Some("URL Protocol")), Some(""));
        assert_eq!(
            find("clonk\\shell\\open\\command", None),
            Some("C:\\Games\\Clonk\\clonk.exe %1")
        );
        assert_eq!(
            find("clonk\\DefaultIcon", None),
            Some("C:\\Games\\Clonk\\clonk.exe,1")
        );

        // The c4u update verb, made default (:62-65; StdRegistry.cpp:238-241).
        assert_eq!(find("Clonk4.Update\\Shell\\Update", None), Some("Update"));
        assert_eq!(
            find("Clonk4.Update\\Shell\\Update\\Command", None),
            Some("\"C:\\Games\\Clonk\\clonk.exe\" \"%1\"")
        );
        assert_eq!(find("Clonk4.Update\\Shell", None), Some("Update"));

        // The AppUserModelId display name (:70).
        assert_eq!(
            find(
                "AppUserModelId\\LegacyClonkTeam.LegacyClonk",
                Some("DisplayName")
            ),
            Some("LegacyClonk")
        );

        // 11 classes * 4 values + 4 protocol + 3 verb + 1 AppUserModelId.
        assert_eq!(values.len(), 11 * 4 + 4 + 3 + 1);
    }

    // Removal must visit children before their parents: `RegDeleteKey` refuses
    // a key that still has subkeys.
    #[test]
    fn removal_order_visits_child_keys_before_their_parents() {
        let keys = file_class_registry_keys_for_removal("C:\\clonk.exe");
        let position = |key: &str| {
            keys.iter()
                .position(|candidate| candidate == key)
                .unwrap_or_else(|| panic!("{key} is not scheduled for removal"))
        };
        assert!(
            position("Clonk4.Update\\Shell\\Update\\Command")
                < position("Clonk4.Update\\Shell\\Update")
        );
        assert!(position("Clonk4.Update\\Shell\\Update") < position("Clonk4.Update\\Shell"));
        assert!(position("Clonk4.Update\\Shell") < position("Clonk4.Update"));
        assert!(position("clonk\\shell\\open\\command") < position("clonk"));
        assert!(position("Clonk4.Scenario\\DefaultIcon") < position("Clonk4.Scenario"));
        // Every registered key is scheduled exactly once.
        let mut deduped = keys.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(deduped.len(), keys.len());
    }
}
