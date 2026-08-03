use anyhow::{bail, Result};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LegacyRegistryConfig {
    pub(crate) keys: Vec<LegacyRegistryKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyRegistryKey {
    /// Registry subkey components relative to the LegacyClonk config root.
    pub(crate) path: Vec<String>,
    pub(crate) values: Vec<LegacyRegistryValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyRegistryValue {
    pub(crate) name: String,
    pub(crate) data: LegacyRegistryData,
}

#[derive(Clone, Debug, Eq, PartialEq)]
// Production values are constructed only by the Windows registry reader. Keep the
// serializer available on other platforms for migration tests without warning in
// non-test release builds.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) enum LegacyRegistryData {
    Dword(Vec<u8>),
    Qword(Vec<u8>),
    String(Vec<u8>),
    Unsupported { value_type: u32, data: Vec<u8> },
}

#[derive(Clone, Copy)]
enum FieldKind {
    Bool,
    SignedDword,
    UnsignedDword,
    SignedQword,
    UnsignedQword,
    String,
    Enum(&'static [&'static str]),
}

#[derive(Clone, Copy)]
struct FieldSchema {
    name: &'static str,
    kind: FieldKind,
}

struct KeySchema {
    path: &'static [&'static str],
    fields: &'static [FieldSchema],
}

const fn field(name: &'static str, kind: FieldKind) -> FieldSchema {
    FieldSchema { name, kind }
}

const GENERAL_FIELDS: &[FieldSchema] = &[
    field("Version", FieldKind::UnsignedDword),
    field("Name", FieldKind::String),
    field("Language", FieldKind::String),
    field("LanguageEx", FieldKind::String),
    field("LanguageCharset", FieldKind::String),
    field("Definitions", FieldKind::String),
    field("Participants", FieldKind::String),
    field("LogPath", FieldKind::String),
    field("PlayerPath", FieldKind::String),
    field("DefinitionPath", FieldKind::String),
    field("UserPath", FieldKind::String),
    field("SaveGameFolder", FieldKind::String),
    field("SaveDemoFolder", FieldKind::String),
    field("MissionAccess", FieldKind::String),
    field("FPS", FieldKind::Bool),
    field("Record", FieldKind::Bool),
    field("ScreenshotFolder", FieldKind::String),
    field("NoCrew", FieldKind::Bool),
    field("DefCrewStrength", FieldKind::SignedDword),
    field("ScrollSmooth", FieldKind::SignedDword),
    field("DebugMode", FieldKind::Bool),
    field("AllowScriptingInReplays", FieldKind::Bool),
    field("FontName", FieldKind::String),
    field("FontSize", FieldKind::SignedDword),
    field("GamepadEnabled", FieldKind::Bool),
    field("FirstStart", FieldKind::Bool),
    field("UserPortraitsWritten", FieldKind::Bool),
    field("ConfigResetSafety", FieldKind::SignedDword),
    field("UseWhiteIngameChat", FieldKind::Bool),
    field("UseWhiteLobbyChat", FieldKind::Bool),
    field("ShowLogTimestamps", FieldKind::Bool),
    field("Preloading", FieldKind::Bool),
];

const CONTROLS_FIELDS: &[FieldSchema] = &[
    field("Kbd1Key1", FieldKind::SignedDword),
    field("Kbd1Key2", FieldKind::SignedDword),
    field("Kbd1Key3", FieldKind::SignedDword),
    field("Kbd1Key4", FieldKind::SignedDword),
    field("Kbd1Key5", FieldKind::SignedDword),
    field("Kbd1Key6", FieldKind::SignedDword),
    field("Kbd1Key7", FieldKind::SignedDword),
    field("Kbd1Key8", FieldKind::SignedDword),
    field("Kbd1Key9", FieldKind::SignedDword),
    field("Kbd1Key10", FieldKind::SignedDword),
    field("Kbd1Key11", FieldKind::SignedDword),
    field("Kbd1Key12", FieldKind::SignedDword),
    field("Kbd2Key1", FieldKind::SignedDword),
    field("Kbd2Key2", FieldKind::SignedDword),
    field("Kbd2Key3", FieldKind::SignedDword),
    field("Kbd2Key4", FieldKind::SignedDword),
    field("Kbd2Key5", FieldKind::SignedDword),
    field("Kbd2Key6", FieldKind::SignedDword),
    field("Kbd2Key7", FieldKind::SignedDword),
    field("Kbd2Key8", FieldKind::SignedDword),
    field("Kbd2Key9", FieldKind::SignedDword),
    field("Kbd2Key10", FieldKind::SignedDword),
    field("Kbd2Key11", FieldKind::SignedDword),
    field("Kbd2Key12", FieldKind::SignedDword),
    field("Kbd3Key1", FieldKind::SignedDword),
    field("Kbd3Key2", FieldKind::SignedDword),
    field("Kbd3Key3", FieldKind::SignedDword),
    field("Kbd3Key4", FieldKind::SignedDword),
    field("Kbd3Key5", FieldKind::SignedDword),
    field("Kbd3Key6", FieldKind::SignedDword),
    field("Kbd3Key7", FieldKind::SignedDword),
    field("Kbd3Key8", FieldKind::SignedDword),
    field("Kbd3Key9", FieldKind::SignedDword),
    field("Kbd3Key10", FieldKind::SignedDword),
    field("Kbd3Key11", FieldKind::SignedDword),
    field("Kbd3Key12", FieldKind::SignedDword),
    field("Kbd4Key1", FieldKind::SignedDword),
    field("Kbd4Key2", FieldKind::SignedDword),
    field("Kbd4Key3", FieldKind::SignedDword),
    field("Kbd4Key4", FieldKind::SignedDword),
    field("Kbd4Key5", FieldKind::SignedDword),
    field("Kbd4Key6", FieldKind::SignedDword),
    field("Kbd4Key7", FieldKind::SignedDword),
    field("Kbd4Key8", FieldKind::SignedDword),
    field("Kbd4Key9", FieldKind::SignedDword),
    field("Kbd4Key10", FieldKind::SignedDword),
    field("Kbd4Key11", FieldKind::SignedDword),
    field("Kbd4Key12", FieldKind::SignedDword),
    field("MouseAutoScroll", FieldKind::SignedDword),
    field("GamepadGuiControl", FieldKind::SignedDword),
];

const GAMEPAD_FIELDS: &[FieldSchema] = &[
    field("Axis0Min", FieldKind::UnsignedDword),
    field("Axis0Max", FieldKind::UnsignedDword),
    field("Axis0Calibrated", FieldKind::Bool),
    field("Axis1Min", FieldKind::UnsignedDword),
    field("Axis1Max", FieldKind::UnsignedDword),
    field("Axis1Calibrated", FieldKind::Bool),
    field("Axis2Min", FieldKind::UnsignedDword),
    field("Axis2Max", FieldKind::UnsignedDword),
    field("Axis2Calibrated", FieldKind::Bool),
    field("Axis3Min", FieldKind::UnsignedDword),
    field("Axis3Max", FieldKind::UnsignedDword),
    field("Axis3Calibrated", FieldKind::Bool),
    field("Axis4Min", FieldKind::UnsignedDword),
    field("Axis4Max", FieldKind::UnsignedDword),
    field("Axis4Calibrated", FieldKind::Bool),
    field("Axis5Min", FieldKind::UnsignedDword),
    field("Axis5Max", FieldKind::UnsignedDword),
    field("Axis5Calibrated", FieldKind::Bool),
    field("Button1", FieldKind::SignedDword),
    field("Button2", FieldKind::SignedDword),
    field("Button3", FieldKind::SignedDword),
    field("Button4", FieldKind::SignedDword),
    field("Button5", FieldKind::SignedDword),
    field("Button6", FieldKind::SignedDword),
    field("Button7", FieldKind::SignedDword),
    field("Button8", FieldKind::SignedDword),
    field("Button9", FieldKind::SignedDword),
    field("Button10", FieldKind::SignedDword),
    field("Button11", FieldKind::SignedDword),
    field("Button12", FieldKind::SignedDword),
];

const UPPER_BOARD_VALUES: &[&str] = &["Hide", "Full", "Small", "Mini"];
const DISPLAY_MODE_VALUES: &[&str] = &["Fullscreen", "Window"];
const STRICTNESS_VALUES: &[&str] = &["NonStrict", "Strict1", "Strict2", "Strict3", "MaxStrict"];
const LOG_LEVEL_VALUES: &[&str] = &["trace", "debug", "info", "warn", "error", "critical", "off"];

const GRAPHICS_FIELDS: &[FieldSchema] = &[
    field("ResolutionX", FieldKind::SignedDword),
    field("ResolutionY", FieldKind::SignedDword),
    field("Scale", FieldKind::SignedDword),
    field("SplitscreenDividers", FieldKind::SignedDword),
    field("ShowPlayerHUDAlways", FieldKind::Bool),
    field("ShowPortraits", FieldKind::Bool),
    field("AddNewCrewPortraits", FieldKind::Bool),
    field("SaveDefaultPortraits", FieldKind::Bool),
    field("ShowCommands", FieldKind::Bool),
    field("ShowCommandKeys", FieldKind::Bool),
    field("ColorAnimation", FieldKind::Bool),
    field("SmokeLevel", FieldKind::SignedDword),
    field("VerboseObjectLoading", FieldKind::SignedDword),
    field("UpperBoard", FieldKind::Enum(UPPER_BOARD_VALUES)),
    field("ShowClock", FieldKind::Bool),
    field("ShowCrewNames", FieldKind::Bool),
    field("ShowCrewCNames", FieldKind::Bool),
    field("MsgBoard", FieldKind::Bool),
    field("PXSGfx", FieldKind::Bool),
    field("Engine", FieldKind::SignedDword),
    field("NoAlphaAdd", FieldKind::Bool),
    field("PointFiltering", FieldKind::Bool),
    field("NoBoxFades", FieldKind::Bool),
    field("NoAcceleration", FieldKind::Bool),
    field("TexIndent", FieldKind::SignedDword),
    field("BlitOffset", FieldKind::SignedDword),
    field("AllowedBlitModes", FieldKind::UnsignedDword),
    field("Gamma1", FieldKind::SignedDword),
    field("Gamma2", FieldKind::SignedDword),
    field("Gamma3", FieldKind::SignedDword),
    field("RenderInactive", FieldKind::UnsignedDword),
    field("DisableGamma", FieldKind::Bool),
    field("Monitor", FieldKind::SignedDword),
    field("FireParticles", FieldKind::Bool),
    field("MaxRefreshDelay", FieldKind::SignedDword),
    field("Shader", FieldKind::Bool),
    field("AutoFrameSkip", FieldKind::Bool),
    field("CacheTexturesInRAM", FieldKind::SignedDword),
    field("DisplayMode", FieldKind::Enum(DISPLAY_MODE_VALUES)),
    field("Maximized", FieldKind::Bool),
    field("PositionX", FieldKind::SignedDword),
    field("PositionY", FieldKind::SignedDword),
    field("ShowFolderMaps", FieldKind::Bool),
    field("UseShaderGamma", FieldKind::Bool),
];

const SOUND_FIELDS: &[FieldSchema] = &[
    field("Sound", FieldKind::Bool),
    field("Music", FieldKind::Bool),
    field("MenuMusic", FieldKind::Bool),
    field("MenuSound", FieldKind::Bool),
    field("MusicVolume", FieldKind::SignedDword),
    field("SoundVolume", FieldKind::SignedDword),
    field("MaxChannels", FieldKind::SignedDword),
    field("PreferLinearResampling", FieldKind::Bool),
    field("MuteSoundCommand", FieldKind::Bool),
];

const NETWORK_FIELDS: &[FieldSchema] = &[
    field("ControlRate", FieldKind::SignedDword),
    field("WorkPath", FieldKind::String),
    field("NoRuntimeJoin", FieldKind::Bool),
    field("MaxResSearchRecursion", FieldKind::SignedDword),
    field("Comment", FieldKind::String),
    field("PortTCP", FieldKind::SignedDword),
    field("PortUDP", FieldKind::SignedDword),
    field("PortDiscovery", FieldKind::SignedDword),
    field("PortRefServer", FieldKind::SignedDword),
    field("ControlMode", FieldKind::SignedDword),
    field("LocalName", FieldKind::String),
    field("Nick", FieldKind::String),
    field("MaxLoadFileSize", FieldKind::SignedDword),
    field("MasterServerSignUp", FieldKind::Bool),
    field("MasterReferencePeriod", FieldKind::SignedDword),
    field("LeagueServerSignUp", FieldKind::Bool),
    field("ServerAddress", FieldKind::String),
    field("UseAlternateServer", FieldKind::Bool),
    field("AlternateServerAddress", FieldKind::String),
    field("UpdateServerAddress", FieldKind::String),
    field("LastPassword", FieldKind::String),
    field("EnableAutomaticUpdate", FieldKind::Bool),
    field("LastUpdateTime", FieldKind::UnsignedQword),
    field("AsyncMaxWait", FieldKind::SignedDword),
    field("PuncherAddress", FieldKind::String),
    field("LeagueNick", FieldKind::String),
    field("LeagueAutoLogin", FieldKind::Bool),
    field("UseCurl", FieldKind::Bool),
    field("EnableUPnP", FieldKind::Bool),
];

const LOBBY_FIELDS: &[FieldSchema] = &[
    field("AllowPlayerSave", FieldKind::Bool),
    field("CountdownTime", FieldKind::SignedDword),
];

const IRC_FIELDS: &[FieldSchema] = &[
    field("Server2", FieldKind::String),
    field("Nick", FieldKind::String),
    field("RealName", FieldKind::String),
    field("Channel", FieldKind::String),
];

const DEVELOPER_FIELDS: &[FieldSchema] = &[
    field("AutoFileReload", FieldKind::Bool),
    field(
        "ConsoleScriptStrictness",
        FieldKind::Enum(STRICTNESS_VALUES),
    ),
];

const STARTUP_FIELDS: &[FieldSchema] = &[
    field("HideMsgStartDedicated", FieldKind::Bool),
    field("HideMsgPlrTakeOver", FieldKind::Bool),
    field("HideMsgPlrNoTakeOver", FieldKind::Bool),
    field("HideMsgNoOfficialLeague", FieldKind::Bool),
    field("HideMsgIRCDangerous", FieldKind::Bool),
    field("AlphabeticalSorting", FieldKind::Bool),
    field("LastPortraitFolderIdx", FieldKind::SignedDword),
];

const COOLDOWN_FIELDS: &[FieldSchema] = &[
    field("SoundCommand", FieldKind::SignedQword),
    field("ReadyCheck", FieldKind::SignedQword),
];

const TOAST_FIELDS: &[FieldSchema] = &[field("ReadyCheck", FieldKind::Bool)];

const LOGGING_FIELDS: &[FieldSchema] =
    &[field("LogLevelStdout", FieldKind::Enum(LOG_LEVEL_VALUES))];

const LOGGER_FIELDS: &[FieldSchema] = &[
    field("LogLevel", FieldKind::Enum(LOG_LEVEL_VALUES)),
    field("GuiLogLevel", FieldKind::Enum(LOG_LEVEL_VALUES)),
    field("ShowLoggerNameInGui", FieldKind::Bool),
];

const SCHEMA: &[KeySchema] = &[
    KeySchema {
        path: &["General"],
        fields: GENERAL_FIELDS,
    },
    KeySchema {
        path: &["Controls"],
        fields: CONTROLS_FIELDS,
    },
    KeySchema {
        path: &["Gamepad0"],
        fields: GAMEPAD_FIELDS,
    },
    KeySchema {
        path: &["Gamepad1"],
        fields: GAMEPAD_FIELDS,
    },
    KeySchema {
        path: &["Gamepad2"],
        fields: GAMEPAD_FIELDS,
    },
    KeySchema {
        path: &["Gamepad3"],
        fields: GAMEPAD_FIELDS,
    },
    KeySchema {
        path: &["Graphics"],
        fields: GRAPHICS_FIELDS,
    },
    KeySchema {
        path: &["Sound"],
        fields: SOUND_FIELDS,
    },
    KeySchema {
        path: &["Network"],
        fields: NETWORK_FIELDS,
    },
    KeySchema {
        path: &["Lobby"],
        fields: LOBBY_FIELDS,
    },
    KeySchema {
        path: &["IRC"],
        fields: IRC_FIELDS,
    },
    KeySchema {
        path: &["Developer"],
        fields: DEVELOPER_FIELDS,
    },
    KeySchema {
        path: &["Startup"],
        fields: STARTUP_FIELDS,
    },
    KeySchema {
        path: &["Cooldowns"],
        fields: COOLDOWN_FIELDS,
    },
    KeySchema {
        path: &["Toasts"],
        fields: TOAST_FIELDS,
    },
    KeySchema {
        path: &["Logging"],
        fields: LOGGING_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4AudioSystem"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4AulExec"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4AulProfiler"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "CStdDDraw"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4GameControl"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "Network"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4Network2IO"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4Network2HTTPClient"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4Network2UPnP"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "C4Playback"],
        fields: LOGGER_FIELDS,
    },
    KeySchema {
        path: &["Logging", "CPNGFile"],
        fields: LOGGER_FIELDS,
    },
    // Present only in legacy builds compiled WITH_GLIB, but harmless to recognize when found.
    KeySchema {
        path: &["Logging", "GLib"],
        fields: LOGGER_FIELDS,
    },
];

struct RenderedKey<'a> {
    path: &'a [&'a str],
    values: Vec<(&'a str, Vec<u8>)>,
}

pub(crate) fn serialize_legacy_registry_config(
    config: &LegacyRegistryConfig,
) -> Result<Option<Vec<u8>>> {
    let mut rendered = Vec::new();

    for key_schema in SCHEMA {
        let Some(key) = find_key(config, key_schema.path) else {
            continue;
        };
        let mut values = Vec::new();
        for field_schema in key_schema.fields {
            let Some(value) = key
                .values
                .iter()
                .find(|value| value.name.eq_ignore_ascii_case(field_schema.name))
            else {
                continue;
            };
            if let Some(serialized) = serialize_field(key_schema.path, *field_schema, value)? {
                values.push((field_schema.name, serialized));
            }
        }
        if !values.is_empty() {
            rendered.push(RenderedKey {
                path: key_schema.path,
                values,
            });
        }
    }

    if rendered.is_empty() {
        return Ok(None);
    }

    let mut output = Vec::new();
    let mut current_top: Option<&str> = None;
    for key in rendered {
        let top = key.path[0];
        if key.path.len() == 1 {
            start_top_level_section(&mut output, top);
            current_top = Some(top);
        } else {
            if current_top != Some(top) {
                start_top_level_section(&mut output, top);
                current_top = Some(top);
            }
            output.extend_from_slice(b"\r\n");
            let indent = (key.path.len() - 1) * 2;
            output.extend(std::iter::repeat_n(b' ', indent));
            output.push(b'[');
            output.extend_from_slice(key.path[key.path.len() - 1].as_bytes());
            output.extend_from_slice(b"]\r\n");
        }

        let indent = (key.path.len() - 1) * 2;
        for (name, serialized) in key.values {
            output.extend(std::iter::repeat_n(b' ', indent));
            output.extend_from_slice(name.as_bytes());
            output.push(b'=');
            output.extend_from_slice(&serialized);
            output.extend_from_slice(b"\r\n");
        }
    }

    Ok(Some(output))
}

fn find_key<'a>(config: &'a LegacyRegistryConfig, path: &[&str]) -> Option<&'a LegacyRegistryKey> {
    config.keys.iter().find(|key| {
        key.path.len() == path.len()
            && key
                .path
                .iter()
                .zip(path)
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    })
}

fn serialize_field(
    path: &[&str],
    schema: FieldSchema,
    value: &LegacyRegistryValue,
) -> Result<Option<Vec<u8>>> {
    let path = path.join("\\");
    let bytes = match (schema.kind, &value.data) {
        (FieldKind::Bool, LegacyRegistryData::Dword(bytes)) => {
            let value = read_dword(bytes, &path, schema.name)?;
            if value == 0 {
                b"false".to_vec()
            } else {
                b"true".to_vec()
            }
        }
        (FieldKind::SignedDword, LegacyRegistryData::Dword(bytes)) => {
            let value = i32::from_le_bytes(read_dword_bytes(bytes, &path, schema.name)?);
            value.to_string().into_bytes()
        }
        (FieldKind::UnsignedDword, LegacyRegistryData::Dword(bytes)) => {
            read_dword(bytes, &path, schema.name)?
                .to_string()
                .into_bytes()
        }
        (FieldKind::SignedQword, LegacyRegistryData::Qword(bytes)) => {
            let value = i64::from_le_bytes(read_qword_bytes(bytes, &path, schema.name)?);
            value.to_string().into_bytes()
        }
        (FieldKind::UnsignedQword, LegacyRegistryData::Qword(bytes)) => {
            u64::from_le_bytes(read_qword_bytes(bytes, &path, schema.name)?)
                .to_string()
                .into_bytes()
        }
        (FieldKind::String, LegacyRegistryData::String(bytes)) => escape_cpp_string(bytes),
        (FieldKind::Enum(accepted), LegacyRegistryData::String(bytes)) => {
            let bytes = registry_string_bytes(bytes);
            if !accepted
                .iter()
                .any(|candidate| candidate.as_bytes() == bytes)
            {
                return Ok(None);
            }
            bytes.to_vec()
        }
        (FieldKind::Enum(_), LegacyRegistryData::Dword(bytes)) => {
            let value = i32::from_le_bytes(read_dword_bytes(bytes, &path, schema.name)?);
            value.to_string().into_bytes()
        }
        _ => return Ok(None),
    };
    Ok(Some(bytes))
}

fn read_dword(bytes: &[u8], path: &str, name: &str) -> Result<u32> {
    Ok(u32::from_le_bytes(read_dword_bytes(bytes, path, name)?))
}

fn read_dword_bytes(bytes: &[u8], path: &str, name: &str) -> Result<[u8; 4]> {
    if bytes.len() != 4 {
        bail!(
            "legacy registry value {path}\\{name} is a DWORD with {} bytes, expected 4",
            bytes.len()
        );
    }
    let mut value = [0; 4];
    value.copy_from_slice(bytes);
    Ok(value)
}

fn read_qword_bytes(bytes: &[u8], path: &str, name: &str) -> Result<[u8; 8]> {
    if bytes.len() != 8 {
        bail!(
            "legacy registry value {path}\\{name} is a QWORD with {} bytes, expected 8",
            bytes.len()
        );
    }
    let mut value = [0; 8];
    value.copy_from_slice(bytes);
    Ok(value)
}

fn registry_string_bytes(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    &bytes[..end]
}

fn escape_cpp_string(bytes: &[u8]) -> Vec<u8> {
    let mut output = vec![b'"'];
    let mut last_was_numeric_escape = false;
    for &byte in registry_string_bytes(bytes) {
        if (b' '..=b'~').contains(&byte)
            && byte != b'\\'
            && byte != b'"'
            && !(last_was_numeric_escape && byte.is_ascii_digit())
        {
            output.push(byte);
            last_was_numeric_escape = false;
            continue;
        }

        let named_escape = match byte {
            0x07 => Some(b'a'),
            0x08 => Some(b'b'),
            0x0c => Some(b'f'),
            b'\n' => Some(b'n'),
            b'\r' => Some(b'r'),
            b'\t' => Some(b't'),
            0x0b => Some(b'v'),
            b'"' => Some(b'"'),
            b'\\' => Some(b'\\'),
            _ => None,
        };
        output.push(b'\\');
        if let Some(escaped) = named_escape {
            output.push(escaped);
            last_was_numeric_escape = false;
        } else {
            output.extend_from_slice(format!("{byte:o}").as_bytes());
            last_was_numeric_escape = true;
        }
    }
    output.push(b'"');
    output
}

fn start_top_level_section(output: &mut Vec<u8>, name: &str) {
    if !output.is_empty() {
        output.extend_from_slice(b"\r\n");
    }
    output.push(b'[');
    output.extend_from_slice(name.as_bytes());
    output.extend_from_slice(b"]\r\n");
}

#[cfg(not(windows))]
pub(crate) fn read_legacy_windows_registry() -> Result<Option<LegacyRegistryConfig>> {
    Ok(None)
}

#[cfg(windows)]
pub(crate) fn read_legacy_windows_registry() -> Result<Option<LegacyRegistryConfig>> {
    windows_reader::read()
}

#[cfg(windows)]
mod windows_reader {
    use super::{
        FieldSchema, KeySchema, LegacyRegistryConfig, LegacyRegistryData, LegacyRegistryKey,
        LegacyRegistryValue, SCHEMA,
    };
    use anyhow::{bail, Result};
    use std::ffi::CString;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND,
        ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumValueA, RegOpenKeyExA, RegQueryInfoKeyA, HKEY, HKEY_CURRENT_USER,
        KEY_READ, REG_DWORD, REG_QWORD, REG_SZ,
    };

    const ROOT_PATH: &[u8] = b"Software\\LegacyClonk Team\\LegacyClonk\0";

    // Keep the ANSI API deliberately: the C++ registry compiler also uses
    // Reg*ValueA, and the serializer below preserves those observed bytes.

    struct OwnedKey(HKEY);

    impl Drop for OwnedKey {
        fn drop(&mut self) {
            // SAFETY: OwnedKey is created only from a successful RegOpenKeyExA call and owns
            // exactly one close of that handle.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    pub(super) fn read() -> Result<Option<LegacyRegistryConfig>> {
        let Some(root) = open_key(HKEY_CURRENT_USER, ROOT_PATH.as_ptr())? else {
            return Ok(None);
        };

        let mut config = LegacyRegistryConfig::default();
        for key_schema in SCHEMA {
            let path = key_schema.path.join("\\");
            let path = CString::new(path).expect("registry schema paths never contain NUL");
            let Some(key) = open_key(root.0, path.as_ptr().cast())? else {
                continue;
            };
            let mut values = enumerate_values(&key)?;
            values.retain(|value| is_recognized_value(key_schema, &value.name));
            if !values.is_empty() {
                config.keys.push(LegacyRegistryKey {
                    path: key_schema
                        .path
                        .iter()
                        .map(|part| (*part).to_owned())
                        .collect(),
                    values,
                });
            }
        }

        Ok(Some(config))
    }

    fn is_recognized_value(schema: &KeySchema, name: &str) -> bool {
        schema
            .fields
            .iter()
            .map(|FieldSchema { name, .. }| name)
            .any(|expected| name.eq_ignore_ascii_case(expected))
    }

    fn open_key(parent: HKEY, path: *const u8) -> Result<Option<OwnedKey>> {
        let mut key: HKEY = null_mut();
        // SAFETY: path points to a NUL-terminated byte string for the duration of the call,
        // and key is a valid out pointer. KEY_READ intentionally uses the process-default
        // registry view, matching the legacy C++ oracle.
        let status = unsafe { RegOpenKeyExA(parent, path, 0, KEY_READ, &mut key) };
        match status {
            ERROR_SUCCESS => Ok(Some(OwnedKey(key))),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
            _ => bail!("RegOpenKeyExA failed with Windows error {status}"),
        }
    }

    fn enumerate_values(key: &OwnedKey) -> Result<Vec<LegacyRegistryValue>> {
        let mut value_count = 0;
        let mut max_name_len = 0;
        let mut max_data_len = 0;
        // SAFETY: all supplied output pointers are valid and optional outputs are null.
        let status = unsafe {
            RegQueryInfoKeyA(
                key.0,
                null_mut(),
                null_mut(),
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut value_count,
                &mut max_name_len,
                &mut max_data_len,
                null_mut(),
                null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            bail!("RegQueryInfoKeyA failed with Windows error {status}");
        }

        let mut values = Vec::with_capacity(value_count as usize);
        let mut index = 0;
        loop {
            let mut name = vec![0; (max_name_len as usize).saturating_add(1).max(1)];
            let mut data = vec![0; (max_data_len as usize).max(1)];
            loop {
                let mut name_len = u32::try_from(name.len()).unwrap_or(u32::MAX);
                let mut data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
                let mut value_type = 0;
                // SAFETY: both buffers and all size/type out pointers remain valid for the
                // call. The same index is retried after ERROR_MORE_DATA.
                let status = unsafe {
                    RegEnumValueA(
                        key.0,
                        index,
                        name.as_mut_ptr(),
                        &mut name_len,
                        null(),
                        &mut value_type,
                        data.as_mut_ptr(),
                        &mut data_len,
                    )
                };
                if status == ERROR_NO_MORE_ITEMS {
                    return Ok(values);
                }
                if status == ERROR_MORE_DATA {
                    let requested_name = (name_len as usize).saturating_add(1);
                    let requested_data = data_len as usize;
                    let next_name = requested_name.max(name.len().saturating_mul(2)).max(1);
                    let next_data = requested_data.max(data.len().saturating_mul(2)).max(1);
                    name.resize(next_name, 0);
                    data.resize(next_data, 0);
                    continue;
                }
                if status != ERROR_SUCCESS {
                    bail!("RegEnumValueA failed with Windows error {status}");
                }

                name.truncate(name_len as usize);
                data.truncate(data_len as usize);
                let Ok(name) = String::from_utf8(name) else {
                    // All recognized LegacyClonk value names are ASCII.
                    index += 1;
                    break;
                };
                let data = match value_type {
                    REG_DWORD => LegacyRegistryData::Dword(data),
                    REG_QWORD => LegacyRegistryData::Qword(data),
                    REG_SZ => LegacyRegistryData::String(data),
                    value_type => LegacyRegistryData::Unsupported { value_type, data },
                };
                values.push(LegacyRegistryValue { name, data });
                index += 1;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(path: &[&str], values: Vec<LegacyRegistryValue>) -> LegacyRegistryKey {
        LegacyRegistryKey {
            path: path.iter().map(|part| (*part).to_owned()).collect(),
            values,
        }
    }

    fn value(name: &str, data: LegacyRegistryData) -> LegacyRegistryValue {
        LegacyRegistryValue {
            name: name.to_owned(),
            data,
        }
    }

    #[test]
    fn l025_serializes_typed_values_and_nested_logging_like_cpp() {
        let config = LegacyRegistryConfig {
            keys: vec![
                key(
                    &["Logging", "C4AudioSystem"],
                    vec![
                        value(
                            "ShowLoggerNameInGui",
                            LegacyRegistryData::Dword(1u32.to_le_bytes().to_vec()),
                        ),
                        value("LogLevel", LegacyRegistryData::String(b"warn\0".to_vec())),
                    ],
                ),
                key(
                    &["Graphics"],
                    vec![
                        value(
                            "DisplayMode",
                            LegacyRegistryData::String(b"Window\0".to_vec()),
                        ),
                        value(
                            "UpperBoard",
                            LegacyRegistryData::String(b"Bogus\0".to_vec()),
                        ),
                    ],
                ),
                key(
                    &["General"],
                    vec![
                        value(
                            "GamepadEnabled",
                            LegacyRegistryData::Dword(0u32.to_le_bytes().to_vec()),
                        ),
                        value(
                            "Name",
                            LegacyRegistryData::String(b"A \"B\\\x01\x31\xc3\0".to_vec()),
                        ),
                    ],
                ),
                key(
                    &["Gamepad0"],
                    vec![value(
                        "Button1",
                        LegacyRegistryData::Dword((-1i32).to_le_bytes().to_vec()),
                    )],
                ),
                key(
                    &["Logging"],
                    vec![value(
                        "LogLevelStdout",
                        LegacyRegistryData::String(b"debug\0".to_vec()),
                    )],
                ),
            ],
        };

        let serialized = serialize_legacy_registry_config(&config)
            .expect("snapshot should serialize")
            .expect("recognized values should produce output");
        assert_eq!(
            serialized,
            b"[General]\r\nName=\"A \\\"B\\\\\\1\\61\\303\"\r\nGamepadEnabled=false\r\n\r\n[Gamepad0]\r\nButton1=-1\r\n\r\n[Graphics]\r\nDisplayMode=Window\r\n\r\n[Logging]\r\nLogLevelStdout=debug\r\n\r\n  [C4AudioSystem]\r\n  LogLevel=warn\r\n  ShowLoggerNameInGui=true\r\n"
        );
    }

    #[test]
    fn l025_preserves_signed_and_unsigned_registry_bit_patterns() {
        let all_ones_dword = u32::MAX.to_le_bytes().to_vec();
        let all_ones_qword = u64::MAX.to_le_bytes().to_vec();
        let config = LegacyRegistryConfig {
            keys: vec![
                key(
                    &["Gamepad0"],
                    vec![
                        value(
                            "Axis0Max",
                            LegacyRegistryData::Dword(all_ones_dword.clone()),
                        ),
                        value("Button1", LegacyRegistryData::Dword(all_ones_dword)),
                    ],
                ),
                key(
                    &["Network"],
                    vec![value(
                        "LastUpdateTime",
                        LegacyRegistryData::Qword(all_ones_qword.clone()),
                    )],
                ),
                key(
                    &["Cooldowns"],
                    vec![value(
                        "SoundCommand",
                        LegacyRegistryData::Qword(all_ones_qword),
                    )],
                ),
            ],
        };

        let serialized = serialize_legacy_registry_config(&config)
            .expect("snapshot should serialize")
            .expect("recognized values should produce output");
        assert_eq!(
            serialized,
            b"[Gamepad0]\r\nAxis0Max=4294967295\r\nButton1=-1\r\n\r\n[Network]\r\nLastUpdateTime=18446744073709551615\r\n\r\n[Cooldowns]\r\nSoundCommand=-1\r\n"
        );
    }

    #[test]
    fn l025_rejects_malformed_expected_integer_sizes_and_ignores_wrong_types() {
        let malformed = LegacyRegistryConfig {
            keys: vec![key(
                &["General"],
                vec![value(
                    "GamepadEnabled",
                    LegacyRegistryData::Dword(vec![0; 3]),
                )],
            )],
        };
        assert!(serialize_legacy_registry_config(&malformed).is_err());

        let wrong_type = LegacyRegistryConfig {
            keys: vec![key(
                &["General"],
                vec![
                    value(
                        "GamepadEnabled",
                        LegacyRegistryData::String(b"0\0".to_vec()),
                    ),
                    value(
                        "Name",
                        LegacyRegistryData::Unsupported {
                            value_type: 2,
                            data: b"ignored\0".to_vec(),
                        },
                    ),
                ],
            )],
        };
        assert_eq!(serialize_legacy_registry_config(&wrong_type).unwrap(), None);
    }
}
