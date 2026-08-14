//! Typed projection of `C4Config` for the classic advanced-settings dialog.
//!
//! `clonk_core::std_config::Config` deliberately preserves an INI document as
//! strings.  The C++ dialog, however, is built while `C4Config::CompileFunc`
//! walks the typed config object.  This module is that policy bridge: it owns
//! the C++ field order/default/type table and hands an inert owned model to
//! `clonk-frontend`. Unknown INI extensions never enter the editor and therefore
//! survive a Save unchanged.

use clonk_core::std_config::Config;
use clonk_frontend::startup_options_advanced::{
    AdvancedConfigChange, AdvancedConfigRow, AdvancedConfigSection, AdvancedConfigValue,
};

use crate::input::advanced_config_default_raw_keyboard_keys;

const I32_MIN: i128 = i32::MIN as i128;
const I32_MAX: i128 = i32::MAX as i128;
const I64_MIN: i128 = i64::MIN as i128;
const I64_MAX: i128 = i64::MAX as i128;
const U32_MAX: i128 = u32::MAX as i128;
const U64_MAX: i128 = u64::MAX as i128;
const C4_MAX_NAME: usize = 30;
const C4_MAX_COMMENT: usize = 256;

const UPPER_BOARD_VALUES: &[(&str, i32)] = &[("Hide", 0), ("Full", 1), ("Small", 2), ("Mini", 3)];
const DISPLAY_MODE_VALUES: &[(&str, i32)] = &[("Fullscreen", 0), ("Window", 1)];
const SCRIPT_STRICTNESS_VALUES: &[(&str, i32)] = &[
    ("NonStrict", 0),
    ("Strict1", 1),
    ("Strict2", 2),
    ("Strict3", 3),
    ("MaxStrict", 255),
];
const LOG_LEVEL_VALUES: &[(&str, i32)] = &[
    ("trace", 0),
    ("debug", 1),
    ("info", 2),
    ("warn", 3),
    ("error", 4),
    ("critical", 5),
    ("off", 6),
];

fn configured_value<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config.get_in(Some(section), key).or_else(|| {
        // Older Rust-written fixtures used the root section for General.
        (section == "General").then(|| config.get(key)).flatten()
    })
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

fn parse_native_integer(value: &str) -> Option<i128> {
    let value = value.trim();
    let (negative, value) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (radix, digits) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or((10, value), |digits| (16, digits));
    if digits.is_empty() {
        return None;
    }
    let magnitude = i128::from_str_radix(digits, radix).ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

fn truncate_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn is_native_markup_tag(tag: &str) -> bool {
    if tag.len() > 49 {
        return false;
    }
    if tag == "i" || (tag.starts_with('/') && !tag.contains(' ')) {
        return true;
    }
    tag.strip_prefix("c ").is_some_and(|color| {
        color.len() <= 8
            && color
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn strip_native_name_markup(value: &str) -> String {
    // C4InVal first removes every opening brace, then CMarkup strips its
    // recognized tags and any now-unpaired closing inline-image marker.
    let without_open_braces: String = value
        .chars()
        .filter(|character| *character != '{')
        .collect();
    let mut stripped = String::with_capacity(without_open_braces.len());
    let mut remaining = without_open_braces.as_str();
    while !remaining.is_empty() {
        if let Some(after_marker) = remaining.strip_prefix("}}") {
            remaining = after_marker;
            continue;
        }
        if let Some(after_open) = remaining.strip_prefix('<') {
            if let Some(close) = after_open.find('>') {
                if is_native_markup_tag(&after_open[..close]) {
                    remaining = &after_open[close + 1..];
                    continue;
                }
            }
        }
        let character = remaining.chars().next().expect("non-empty string");
        stripped.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    stripped
}

fn trim_ascii_whitespace(value: &str) -> &str {
    value.trim_matches(|character: char| character.is_ascii_whitespace())
}

fn validate_network_comment(value: &str) -> String {
    truncate_bytes(value, C4_MAX_COMMENT)
}

fn validate_network_name(value: &str, allow_empty: bool) -> String {
    let stripped = strip_native_name_markup(value);
    let trimmed = trim_ascii_whitespace(&stripped);
    if trimmed.is_empty() && !allow_empty {
        return "Unknown".to_string();
    }
    truncate_bytes(trimmed, C4_MAX_NAME)
}

fn enum_values(section: &str, key: &str) -> Option<&'static [(&'static str, i32)]> {
    match (section, key) {
        ("Graphics", "UpperBoard") => Some(UPPER_BOARD_VALUES),
        ("Graphics", "DisplayMode") => Some(DISPLAY_MODE_VALUES),
        ("Developer", "ConsoleScriptStrictness") => Some(SCRIPT_STRICTNESS_VALUES),
        ("Logging", "LogLevelStdout") => Some(LOG_LEVEL_VALUES),
        _ => None,
    }
}

fn text_row(config: &Config, section: &str, key: &str, default: &str) -> AdvancedConfigRow {
    AdvancedConfigRow::new(
        key,
        AdvancedConfigValue::Text(
            configured_value(config, section, key)
                .unwrap_or(default)
                .to_string(),
        ),
    )
}

fn enum_row(
    config: &Config,
    section: &str,
    key: &str,
    default: &str,
    values: &[(&str, i32)],
) -> AdvancedConfigRow {
    let configured = configured_value(config, section, key);
    let value = configured
        .and_then(|configured| {
            values
                .iter()
                .find_map(|(name, _)| (*name == configured.trim()).then_some(*name))
                .or_else(|| {
                    let number = parse_native_integer(configured)?;
                    let number = i32::try_from(number).ok()?;
                    values
                        .iter()
                        .find_map(|(name, value)| (*value == number).then_some(*name))
                })
        })
        .unwrap_or(default);
    AdvancedConfigRow::new(key, AdvancedConfigValue::Text(value.to_string()))
}

fn validated_text_row(
    config: &Config,
    section: &str,
    key: &str,
    default: &str,
    validate: impl FnOnce(&str) -> String,
) -> AdvancedConfigRow {
    AdvancedConfigRow::new(
        key,
        AdvancedConfigValue::Text(validate(
            configured_value(config, section, key).unwrap_or(default),
        )),
    )
}

fn bool_row(config: &Config, section: &str, key: &str, default: bool) -> AdvancedConfigRow {
    AdvancedConfigRow::new(
        key,
        AdvancedConfigValue::Bool(
            configured_value(config, section, key)
                .and_then(parse_bool)
                .unwrap_or(default),
        ),
    )
}

fn int_row(
    config: &Config,
    section: &str,
    key: &str,
    default: i128,
    min: i128,
    max: i128,
) -> AdvancedConfigRow {
    AdvancedConfigRow::new(
        key,
        AdvancedConfigValue::Integer {
            value: configured_value(config, section, key)
                .and_then(parse_native_integer)
                .unwrap_or(default)
                .clamp(min, max),
            min,
            max,
        },
    )
}

fn i32_row(config: &Config, section: &str, key: &str, default: i32) -> AdvancedConfigRow {
    int_row(config, section, key, i128::from(default), I32_MIN, I32_MAX)
}

fn readonly_row(config: &Config, key: &str, default: &str) -> AdvancedConfigRow {
    AdvancedConfigRow::new(
        key,
        AdvancedConfigValue::ReadOnly(
            configured_value(config, "General", key)
                .unwrap_or(default)
                .to_string(),
        ),
    )
}

#[cfg(target_os = "windows")]
const DEFAULT_USER_PATH: &str = "%APPDATA%\\Clonk Rust";
#[cfg(target_os = "linux")]
const DEFAULT_USER_PATH: &str = "$HOME/.local/share/clonk-rust";
#[cfg(target_os = "macos")]
const DEFAULT_USER_PATH: &str = "$HOME/Library/Application Support/Clonk Rust";

fn general(config: &Config) -> AdvancedConfigSection {
    let section = "General";
    let mut rows = vec![
        readonly_row(config, "Version", "347"),
        text_row(config, section, "Name", ""),
        text_row(config, section, "Language", ""),
        text_row(config, section, "LanguageEx", ""),
        text_row(config, section, "LanguageCharset", ""),
        text_row(config, section, "Definitions", ""),
        text_row(config, section, "Participants", ""),
        text_row(config, section, "LogPath", ""),
        text_row(config, section, "PlayerPath", ""),
        text_row(config, section, "DefinitionPath", ""),
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        text_row(config, section, "UserPath", DEFAULT_USER_PATH),
        text_row(config, section, "SaveGameFolder", "Savegames.c4f"),
        text_row(config, section, "SaveDemoFolder", "Records.c4f"),
        text_row(config, section, "MissionAccess", ""),
        bool_row(config, section, "FPS", false),
        bool_row(config, section, "Record", false),
        text_row(config, section, "ScreenshotFolder", "Screenshots"),
        bool_row(config, section, "NoCrew", false),
        i32_row(config, section, "DefCrewStrength", 1_000),
        i32_row(config, section, "ScrollSmooth", 4),
        bool_row(config, section, "DebugMode", false),
        bool_row(config, section, "AllowScriptingInReplays", false),
        text_row(config, section, "FontName", "Endeavour"),
        i32_row(config, section, "FontSize", 14),
        bool_row(config, section, "GamepadEnabled", true),
        bool_row(config, section, "FirstStart", true),
        bool_row(config, section, "UserPortraitsWritten", false),
        readonly_row(config, "ConfigResetSafety", "42"),
        bool_row(config, section, "UseWhiteIngameChat", false),
        bool_row(config, section, "UseWhiteLobbyChat", false),
        bool_row(config, section, "ShowLogTimestamps", false),
        bool_row(
            config,
            section,
            "Preloading",
            cfg!(not(target_os = "macos")),
        ),
    ];
    #[cfg(not(target_os = "windows"))]
    rows.push(int_row(
        config,
        section,
        "ThreadPoolThreadCount",
        8,
        0,
        U32_MAX,
    ));
    AdvancedConfigSection::new(section, rows)
}

fn controls(config: &Config) -> AdvancedConfigSection {
    let section = "Controls";
    let mut rows = Vec::with_capacity(50);
    let defaults = advanced_config_default_raw_keyboard_keys();
    for keyboard in 1..=4 {
        for key in 1..=12 {
            rows.push(i32_row(
                config,
                section,
                &format!("Kbd{keyboard}Key{key}"),
                defaults[keyboard - 1][key - 1],
            ));
        }
    }
    rows.push(i32_row(config, section, "MouseAutoScroll", 0));
    rows.push(i32_row(config, section, "GamepadGuiControl", 0));
    AdvancedConfigSection::new(section, rows)
}

fn gamepad(config: &Config, index: usize) -> AdvancedConfigSection {
    let section = format!("Gamepad{index}");
    let mut rows = Vec::with_capacity(30);
    for axis in 0..6 {
        rows.push(int_row(
            config,
            &section,
            &format!("Axis{axis}Min"),
            0,
            0,
            U32_MAX,
        ));
        rows.push(int_row(
            config,
            &section,
            &format!("Axis{axis}Max"),
            0,
            0,
            U32_MAX,
        ));
        rows.push(bool_row(
            config,
            &section,
            &format!("Axis{axis}Calibrated"),
            false,
        ));
    }
    for button in 1..=12 {
        rows.push(i32_row(config, &section, &format!("Button{button}"), -1));
    }
    AdvancedConfigSection::new(section, rows)
}

fn graphics(config: &Config) -> AdvancedConfigSection {
    let section = "Graphics";
    #[allow(unused_mut)]
    let mut rows = vec![
        i32_row(config, section, "ResolutionX", 800),
        i32_row(config, section, "ResolutionY", 600),
        i32_row(config, section, "Scale", 100),
        i32_row(config, section, "SplitscreenDividers", 1),
        bool_row(config, section, "ShowPlayerHUDAlways", true),
        bool_row(config, section, "ShowPortraits", true),
        bool_row(config, section, "AddNewCrewPortraits", true),
        bool_row(config, section, "SaveDefaultPortraits", true),
        bool_row(config, section, "ShowCommands", true),
        bool_row(config, section, "ShowCommandKeys", true),
        bool_row(config, section, "ColorAnimation", false),
        i32_row(config, section, "SmokeLevel", 200),
        i32_row(config, section, "VerboseObjectLoading", 0),
        enum_row(config, section, "UpperBoard", "Full", UPPER_BOARD_VALUES),
        bool_row(config, section, "ShowClock", false),
        bool_row(config, section, "ShowStats", false),
        bool_row(config, section, "ShowCrewNames", true),
        bool_row(config, section, "ShowCrewCNames", true),
        bool_row(config, section, "MsgBoard", true),
        bool_row(config, section, "PXSGfx", true),
        i32_row(config, section, "Engine", 0),
        bool_row(config, section, "NoAlphaAdd", false),
        bool_row(config, section, "PointFiltering", false),
        bool_row(config, section, "Remaster", false),
        bool_row(config, section, "HighDpiCursor", false),
        bool_row(config, section, "SkyDither", false),
        bool_row(config, section, "Mipmaps", false),
        bool_row(config, section, "SmoothLandscape", false),
        bool_row(config, section, "FineFogOfWar", false),
        bool_row(config, section, "HDExactBlits", false),
        bool_row(config, section, "LoaderAspect", false),
        bool_row(config, section, "NoBoxFades", false),
        bool_row(config, section, "NoAcceleration", false),
        i32_row(config, section, "TexIndent", 0),
        i32_row(config, section, "BlitOffset", 0),
        int_row(config, section, "AllowedBlitModes", 15, 0, U32_MAX),
        i32_row(config, section, "Gamma1", 0),
        i32_row(config, section, "Gamma2", 0x80_80_80),
        i32_row(config, section, "Gamma3", 0xff_ff_ff),
        // The editor materializes the shipped default, both bits, rather than
        // C++'s Console alone (C4Config.cpp:481): the divergence is the default
        // itself, so showing the oracle value here would write the frozen
        // Alt-Tab behaviour back in as an explicit key on the next Save.
        int_row(
            config,
            section,
            "RenderInactive",
            i128::from(crate::RENDER_INACTIVE_FULLSCREEN | crate::RENDER_INACTIVE_CONSOLE),
            0,
            U32_MAX,
        ),
        bool_row(config, section, "DisableGamma", false),
        i32_row(config, section, "Monitor", 0),
        bool_row(config, section, "FireParticles", true),
        // The editor materializes the oracle's own default (C4Config.cpp:485),
        // not the faster presentation cadence: that is opt-in through
        // `Graphics.SmoothPresentation`, which deliberately leaves this key
        // absent so an explicit value written here still wins.
        i32_row(
            config,
            section,
            "MaxRefreshDelay",
            crate::DEFAULT_MAX_REFRESH_DELAY_MS as i32,
        ),
        bool_row(config, section, "Shader", false),
        bool_row(config, section, "AutoFrameSkip", true),
        i32_row(config, section, "CacheTexturesInRAM", 100),
        enum_row(
            config,
            section,
            "DisplayMode",
            "Fullscreen",
            DISPLAY_MODE_VALUES,
        ),
        bool_row(config, section, "ShowFolderMaps", true),
        bool_row(config, section, "UseShaderGamma", true),
    ];
    #[cfg(target_os = "windows")]
    rows.splice(
        rows.len() - 2..rows.len() - 2,
        [
            bool_row(config, section, "Maximized", false),
            i32_row(config, section, "PositionX", 0),
            i32_row(config, section, "PositionY", 0),
        ],
    );
    AdvancedConfigSection::new(section, rows)
}

fn sound(config: &Config) -> AdvancedConfigSection {
    let section = "Sound";
    AdvancedConfigSection::new(
        section,
        vec![
            bool_row(config, section, "Sound", true),
            bool_row(config, section, "Music", true),
            bool_row(config, section, "MenuMusic", true),
            bool_row(config, section, "MenuSound", true),
            i32_row(config, section, "MusicVolume", 100),
            i32_row(config, section, "SoundVolume", 100),
            int_row(config, section, "MaxChannels", 1_024, 1, 1_024),
            bool_row(config, section, "PreferLinearResampling", false),
            bool_row(config, section, "MuteSoundCommand", false),
        ],
    )
}

fn voice(config: &Config) -> AdvancedConfigSection {
    let section = "Voice";
    let default_push_to_talk =
        crate::input::encode_virtual_key_code(winit::keyboard::KeyCode::Backquote)
            .expect("backquote has a native key code");
    AdvancedConfigSection::new(
        section,
        vec![
            // Port-only extension: LegacyClonk has no microphone or voice
            // settings, so this section has no C4Config counterpart.
            //
            // These three are also on the Options dialog's Audio sheet
            // (clonk-org/clonk-rs#452), in a group placed in the vertical slack
            // C++'s own grid leaves unused. They stay here as well because the
            // Audio group is omitted where that slack is too small -- 640x480
            // leaves 50px -- and because this editor is the only surface for
            // the remaining Voice keys.
            bool_row(config, section, "Enabled", false),
            int_row(config, section, "Volume", 100, 0, 100),
            i32_row(config, section, "PushToTalkKey", default_push_to_talk),
        ],
    )
}

fn network(config: &Config) -> AdvancedConfigSection {
    let section = "Network";
    AdvancedConfigSection::new(
        section,
        vec![
            i32_row(config, section, "ControlRate", 2),
            text_row(config, section, "WorkPath", "Network"),
            bool_row(config, section, "NoRuntimeJoin", true),
            // clonk-rs extension, absent from C4Config: a host that sets this
            // refuses to readmit a profile its player was eliminated with
            // (clonk-org/clonk-rs#240).
            bool_row(config, section, "NoRejoinAfterElimination", false),
            i32_row(config, section, "MaxResSearchRecursion", 1),
            validated_text_row(config, section, "Comment", "", validate_network_comment),
            i32_row(config, section, "PortTCP", 11_112),
            i32_row(config, section, "PortUDP", 11_113),
            i32_row(config, section, "PortDiscovery", 11_114),
            i32_row(config, section, "PortRefServer", 11_111),
            // 2 = `CNM_Async`, where C++ ships 0 (`CNM_Decentral`) in
            // `C4Config.cpp` and labels async experimental
            // (`C4GameOptions.cpp:93`). Only the default differs: the
            // mechanism is a faithful port of `PackCompleteCtrl`
            // (C4GameControlNetwork.cpp:741-784, deadline mirroring :754).
            // In lockstep the host cannot publish a tick until every client's
            // control for it arrives, so the slowest link paces the session and
            // one bad peer stalls everyone; async bounds that wait at
            // `ControlRate * AsyncMaxWait * 1000 / TargetFPS` (106 ms at
            // defaults) and drops the absent input rather than deferring it.
            // Determinism is unaffected — only the host decides the timeout and
            // it still broadcasts one authoritative aggregate that every client
            // executes identically. The drop is silent: a player whose input
            // misses the deadline gets no client-side signal that it was
            // discarded. Measured over 16 seeds x 400 ticks with
            // PreSend active, p99/max shared-tick lateness against a 250 ms
            // peer fell 232/281 ms -> 190/206 ms for 32 dropped packets, and a
            // 60 ms/10%-loss peer was unchanged at 93/106 ms with 0 drops.
            // Do not tune the budget down to chase the tail: it must stay above
            // ordinary delivery time, or a peer that is consistently slow is
            // dropped on nearly every tick (`AsyncMaxWait` 1 dropped 6490 of
            // 6400 ticks against a 250 ms peer without PreSend).
            i32_row(config, section, "ControlMode", 2),
            validated_text_row(config, section, "LocalName", "Unknown", |value| {
                validate_network_name(value, false)
            }),
            validated_text_row(config, section, "Nick", "", |value| {
                validate_network_name(value, true)
            }),
            i32_row(config, section, "MaxLoadFileSize", 100 * 1024 * 1024),
            bool_row(config, section, "MasterServerSignUp", true),
            i32_row(config, section, "MasterReferencePeriod", 120),
            bool_row(config, section, "LeagueServerSignUp", false),
            text_row(
                config,
                section,
                "ServerAddress",
                "https://league.clonkspot.org",
            ),
            bool_row(config, section, "UseAlternateServer", false),
            text_row(
                config,
                section,
                "AlternateServerAddress",
                "https://league.clonkspot.org",
            ),
            text_row(
                config,
                section,
                "UpdateServerAddress",
                "https://update.clonkspot.org/lc/update",
            ),
            text_row(config, section, "LastPassword", "Wipf"),
            bool_row(config, section, "EnableAutomaticUpdate", true),
            int_row(config, section, "LastUpdateTime", 0, 0, U64_MAX),
            i32_row(config, section, "AsyncMaxWait", 2),
            text_row(
                config,
                section,
                "PuncherAddress",
                "netpuncher.openclonk.org:11115",
            ),
            text_row(config, section, "LeagueNick", ""),
            bool_row(config, section, "LeagueAutoLogin", true),
            bool_row(config, section, "UseCurl", true),
            bool_row(config, section, "EnableUPnP", true),
        ],
    )
}

fn simple_sections(config: &Config) -> Vec<AdvancedConfigSection> {
    vec![
        AdvancedConfigSection::new(
            "Lobby",
            vec![
                bool_row(config, "Lobby", "AllowPlayerSave", false),
                i32_row(config, "Lobby", "CountdownTime", 5),
            ],
        ),
        AdvancedConfigSection::new(
            "IRC",
            vec![
                text_row(config, "IRC", "Server2", "irc.euirc.net"),
                text_row(config, "IRC", "Nick", ""),
                text_row(config, "IRC", "RealName", ""),
                text_row(config, "IRC", "Channel", "#clonken,#legacyclonk"),
            ],
        ),
        AdvancedConfigSection::new(
            "Developer",
            vec![
                bool_row(config, "Developer", "AutoFileReload", true),
                enum_row(
                    config,
                    "Developer",
                    "ConsoleScriptStrictness",
                    "MaxStrict",
                    SCRIPT_STRICTNESS_VALUES,
                ),
            ],
        ),
        AdvancedConfigSection::new(
            "Startup",
            vec![
                bool_row(config, "Startup", "HideMsgStartDedicated", false),
                bool_row(config, "Startup", "HideMsgPlrTakeOver", false),
                bool_row(config, "Startup", "HideMsgPlrNoTakeOver", false),
                bool_row(config, "Startup", "HideMsgNoOfficialLeague", false),
                bool_row(config, "Startup", "HideMsgIRCDangerous", false),
                bool_row(config, "Startup", "AlphabeticalSorting", false),
                i32_row(config, "Startup", "LastPortraitFolderIdx", 0),
            ],
        ),
        AdvancedConfigSection::new(
            "Cooldowns",
            vec![
                int_row(config, "Cooldowns", "SoundCommand", 0, I64_MIN, I64_MAX),
                int_row(config, "Cooldowns", "ReadyCheck", 10, 5, I64_MAX),
            ],
        ),
        AdvancedConfigSection::new(
            "Toasts",
            vec![bool_row(config, "Toasts", "ReadyCheck", true)],
        ),
        AdvancedConfigSection::new(
            "Logging",
            vec![enum_row(
                config,
                "Logging",
                "LogLevelStdout",
                "info",
                LOG_LEVEL_VALUES,
            )],
        ),
    ]
}

/// Builds the same top-level section order traversed by
/// `C4Config::CompileFunc`. Nested logger fields are deliberately absent: the
/// native GUI compiler ignores names below depth two as well.
pub fn sections(config: &Config) -> Vec<AdvancedConfigSection> {
    let mut sections = vec![general(config), controls(config)];
    sections.extend((0..4).map(|index| gamepad(config, index)));
    sections.push(graphics(config));
    sections.push(sound(config));
    sections.push(voice(config));
    sections.push(network(config));
    sections.extend(simple_sections(config));
    sections
}

/// Materializes the typed defaults traversed by `C4Config::Default` into a
/// fresh document. Startup corruption recovery must discard every value from
/// the damaged file, including unknown extensions, rather than canonicalize
/// that file in place.
pub fn default_config() -> Config {
    let empty = Config::new();
    let mut defaults = Config::new();
    for section in sections(&empty) {
        let section_name = section.name;
        for row in section.rows {
            defaults.set_in(
                Some(section_name.as_str()),
                row.name,
                row.value.serialized(),
            );
        }
    }
    defaults
}

/// Replays the native typed config load/save normalization for fields that
/// are already present. Missing defaults stay missing, protected rows remain
/// untouched, and unknown extension keys never enter the projection.
pub fn canonicalize_existing(config: &mut Config) {
    for section in sections(config) {
        for row in section.rows {
            if !row.value.is_editable() {
                continue;
            }
            let value = row.value.serialized();
            if config
                .get_in(Some(section.name.as_str()), row.name.as_str())
                .is_some()
            {
                config.set_in(Some(section.name.as_str()), row.name.as_str(), value);
            } else if section.name == "General" && config.get(row.name.as_str()).is_some() {
                // Retain the location used by older Rust-generated fixtures.
                config.set(row.name.as_str(), value);
            }
        }
    }
}

/// Applies only dirty editor rows. Unknown INI nodes remain untouched, and
/// the two integrity fields blocked by the C++ dialog are rejected again at
/// this persistence boundary.
pub fn apply_changes(config: &mut Config, changes: &[AdvancedConfigChange]) {
    for change in changes {
        if change.section == "General"
            && matches!(change.key.as_str(), "Version" | "ConfigResetSafety")
        {
            continue;
        }
        let value = if let Some(values) = enum_values(&change.section, &change.key) {
            let Some((name, _)) = values
                .iter()
                .find(|(name, _)| *name == change.value.as_str())
            else {
                // The native enum adaptor warns and leaves the prior typed
                // value untouched when an edit is not a known identifier.
                continue;
            };
            (*name).to_string()
        } else if change.section == "Network" && change.key == "Comment" {
            validate_network_comment(&change.value)
        } else if change.section == "Network" && change.key == "LocalName" {
            validate_network_name(&change.value, false)
        } else if change.section == "Network" && change.key == "Nick" {
            validate_network_name(&change.value, true)
        } else {
            change.value.clone()
        };
        config.set_in(Some(change.section.as_str()), change.key.as_str(), value);
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use clonk_frontend::startup_options_advanced::AdvancedConfigController;

    fn change(section: &str, key: &str, value: impl Into<String>) -> AdvancedConfigChange {
        AdvancedConfigChange {
            section: section.to_string(),
            key: key.to_string(),
            value: value.into(),
        }
    }

    #[test]
    fn schema_keeps_string_bool_integer_and_integrity_types_distinct() {
        let mut config = Config::new();
        config.set_in(Some("General"), "Name", "1");
        config.set_in(Some("General"), "FPS", "1");
        config.set_in(Some("Graphics"), "SmokeLevel", "1");
        config.set_in(Some("General"), "Version", "999");

        let controller = AdvancedConfigController::new(sections(&config));
        assert!(matches!(
            controller.value("General", "Name"),
            Some(AdvancedConfigValue::Text(value)) if value == "1"
        ));
        assert_eq!(
            controller.value("General", "FPS"),
            Some(&AdvancedConfigValue::Bool(true))
        );
        assert!(matches!(
            controller.value("Graphics", "SmokeLevel"),
            Some(AdvancedConfigValue::Integer { value: 1, .. })
        ));
        assert_eq!(
            controller.value("General", "Version"),
            Some(&AdvancedConfigValue::ReadOnly("999".to_string()))
        );
    }

    #[test]
    fn schema_exposes_voice_as_a_port_only_section() {
        let controller = AdvancedConfigController::new(sections(&Config::new()));

        assert_eq!(
            controller.value("Voice", "Enabled"),
            Some(&AdvancedConfigValue::Bool(false))
        );
        assert!(matches!(
            controller.value("Voice", "Volume"),
            Some(AdvancedConfigValue::Integer {
                value: 100,
                min: 0,
                max: 100,
            })
        ));
        assert!(matches!(
            controller.value("Voice", "PushToTalkKey"),
            Some(AdvancedConfigValue::Integer { value, .. })
                if *value == i128::from(
                    crate::input::encode_virtual_key_code(winit::keyboard::KeyCode::Backquote)
                        .expect("backquote has a native key code")
                )
        ));
    }

    #[test]
    fn applying_dirty_rows_preserves_vendor_extensions_and_blocks_integrity_fields() {
        let mut config = Config::new();
        config.set_in(Some("General"), "Name", "Old");
        config.set_in(Some("General"), "VendorExtension", "keep");
        config.set_in(Some("General"), "Version", "347");
        apply_changes(
            &mut config,
            &[
                AdvancedConfigChange {
                    section: "General".to_string(),
                    key: "Name".to_string(),
                    value: "New".to_string(),
                },
                AdvancedConfigChange {
                    section: "General".to_string(),
                    key: "Version".to_string(),
                    value: "999".to_string(),
                },
            ],
        );
        assert_eq!(config.get_in(Some("General"), "Name"), Some("New"));
        assert_eq!(
            config.get_in(Some("General"), "VendorExtension"),
            Some("keep")
        );
        assert_eq!(config.get_in(Some("General"), "Version"), Some("347"));
    }

    #[test]
    fn native_bool_and_hex_integer_forms_are_projected() {
        let mut config = Config::new();
        config.set_in(Some("General"), "FPS", "yes");
        config.set_in(Some("General"), "GamepadEnabled", "off");
        config.set_in(Some("General"), "Record", "TRUE");
        config.set_in(Some("General"), "DebugMode", "true");
        config.set_in(Some("General"), "FirstStart", "0");
        config.set_in(Some("General"), "UserPortraitsWritten", "1");
        config.set_in(Some("Graphics"), "Gamma2", "0x808080");
        config.set_in(Some("Graphics"), "AllowedBlitModes", "0Xff");
        config.set_in(Some("Network"), "LastUpdateTime", "0x10");

        let controller = AdvancedConfigController::new(sections(&config));
        assert_eq!(
            controller.value("General", "FPS"),
            Some(&AdvancedConfigValue::Bool(false))
        );
        assert_eq!(
            controller.value("General", "GamepadEnabled"),
            Some(&AdvancedConfigValue::Bool(true))
        );
        assert_eq!(
            controller.value("General", "Record"),
            Some(&AdvancedConfigValue::Bool(false))
        );
        assert_eq!(
            controller.value("General", "DebugMode"),
            Some(&AdvancedConfigValue::Bool(true))
        );
        assert_eq!(
            controller.value("General", "FirstStart"),
            Some(&AdvancedConfigValue::Bool(false))
        );
        assert_eq!(
            controller.value("General", "UserPortraitsWritten"),
            Some(&AdvancedConfigValue::Bool(true))
        );
        assert!(matches!(
            controller.value("Graphics", "Gamma2"),
            Some(AdvancedConfigValue::Integer {
                value: 0x80_80_80,
                ..
            })
        ));
        assert!(matches!(
            controller.value("Graphics", "AllowedBlitModes"),
            Some(AdvancedConfigValue::Integer { value: 0xff, .. })
        ));
        assert!(matches!(
            controller.value("Network", "LastUpdateTime"),
            Some(AdvancedConfigValue::Integer { value: 16, .. })
        ));
    }

    #[test]
    fn native_enums_use_canonical_tokens_and_retain_values_on_invalid_edits() {
        let mut config = Config::new();
        config.set_in(Some("Graphics"), "UpperBoard", "3");
        config.set_in(Some("Graphics"), "DisplayMode", "Window");
        config.set_in(Some("Developer"), "ConsoleScriptStrictness", "Strict2");
        config.set_in(Some("Logging"), "LogLevelStdout", "4");

        let controller = AdvancedConfigController::new(sections(&config));
        assert_eq!(
            controller.value("Graphics", "UpperBoard"),
            Some(&AdvancedConfigValue::Text("Mini".to_string()))
        );
        assert_eq!(
            controller.value("Graphics", "DisplayMode"),
            Some(&AdvancedConfigValue::Text("Window".to_string()))
        );
        assert_eq!(
            controller.value("Developer", "ConsoleScriptStrictness"),
            Some(&AdvancedConfigValue::Text("Strict2".to_string()))
        );
        assert_eq!(
            controller.value("Logging", "LogLevelStdout"),
            Some(&AdvancedConfigValue::Text("error".to_string()))
        );

        apply_changes(
            &mut config,
            &[
                change("Graphics", "UpperBoard", "mini"),
                change("Graphics", "DisplayMode", "Borderless"),
                change("Developer", "ConsoleScriptStrictness", "Strict4"),
                change("Logging", "LogLevelStdout", "warning"),
            ],
        );
        assert_eq!(config.get_in(Some("Graphics"), "UpperBoard"), Some("3"));
        assert_eq!(
            config.get_in(Some("Graphics"), "DisplayMode"),
            Some("Window")
        );
        assert_eq!(
            config.get_in(Some("Developer"), "ConsoleScriptStrictness"),
            Some("Strict2")
        );
        assert_eq!(config.get_in(Some("Logging"), "LogLevelStdout"), Some("4"));

        apply_changes(
            &mut config,
            &[
                change("Graphics", "UpperBoard", "Small"),
                change("Graphics", "DisplayMode", "Fullscreen"),
                change("Developer", "ConsoleScriptStrictness", "MaxStrict"),
                change("Logging", "LogLevelStdout", "critical"),
            ],
        );
        assert_eq!(config.get_in(Some("Graphics"), "UpperBoard"), Some("Small"));
        assert_eq!(
            config.get_in(Some("Graphics"), "DisplayMode"),
            Some("Fullscreen")
        );
        assert_eq!(
            config.get_in(Some("Developer"), "ConsoleScriptStrictness"),
            Some("MaxStrict")
        );
        assert_eq!(
            config.get_in(Some("Logging"), "LogLevelStdout"),
            Some("critical")
        );
    }

    #[test]
    fn native_network_validators_normalize_model_and_persisted_edits() {
        let mut config = Config::new();
        config.set_in(Some("Network"), "Comment", "C".repeat(300));
        config.set_in(Some("Network"), "LocalName", "  {<i>Alice</i>}}  ");
        config.set_in(
            Some("Network"),
            "Nick",
            format!("  <c ff0000>{}</c>  ", "N".repeat(40)),
        );

        let controller = AdvancedConfigController::new(sections(&config));
        assert!(matches!(
            controller.value("Network", "Comment"),
            Some(AdvancedConfigValue::Text(value)) if value.len() == C4_MAX_COMMENT
        ));
        assert_eq!(
            controller.value("Network", "LocalName"),
            Some(&AdvancedConfigValue::Text("Alice".to_string()))
        );
        assert!(matches!(
            controller.value("Network", "Nick"),
            Some(AdvancedConfigValue::Text(value)) if value == &"N".repeat(C4_MAX_NAME)
        ));

        apply_changes(
            &mut config,
            &[
                change("Network", "Comment", "D".repeat(300)),
                change("Network", "LocalName", " { <i></i> }} "),
                change("Network", "Nick", "  {<i>Bob</i>}}  "),
            ],
        );
        assert_eq!(
            config
                .get_in(Some("Network"), "Comment")
                .expect("comment")
                .len(),
            C4_MAX_COMMENT
        );
        assert_eq!(config.get_in(Some("Network"), "LocalName"), Some("Unknown"));
        assert_eq!(config.get_in(Some("Network"), "Nick"), Some("Bob"));
    }

    #[test]
    fn canonicalization_rewrites_only_existing_editable_known_fields() {
        let mut config = Config::new();
        config.set("DebugMode", "true");
        config.set_in(Some("General"), "FPS", "true");
        config.set_in(Some("General"), "Version", "999");
        config.set_in(Some("General"), "VendorExtension", "keep");
        config.set_in(Some("Graphics"), "Gamma2", "0x808080");
        config.set_in(Some("Graphics"), "UpperBoard", "3");
        config.set_in(Some("Network"), "Comment", "C".repeat(300));
        config.set_in(Some("Network"), "LocalName", " {<i>Alice</i>}} ");

        canonicalize_existing(&mut config);

        assert_eq!(config.get("DebugMode"), Some("1"));
        assert_eq!(config.get_in(Some("General"), "DebugMode"), None);
        assert_eq!(config.get_in(Some("General"), "FPS"), Some("1"));
        assert_eq!(config.get_in(Some("General"), "Record"), None);
        assert_eq!(config.get_in(Some("General"), "Version"), Some("999"));
        assert_eq!(
            config.get_in(Some("General"), "VendorExtension"),
            Some("keep")
        );
        assert_eq!(config.get_in(Some("Graphics"), "Gamma2"), Some("8421504"));
        assert_eq!(config.get_in(Some("Graphics"), "UpperBoard"), Some("Mini"));
        assert_eq!(
            config
                .get_in(Some("Network"), "Comment")
                .expect("comment")
                .len(),
            C4_MAX_COMMENT
        );
        assert_eq!(config.get_in(Some("Network"), "LocalName"), Some("Alice"));
    }
}
