//! User-facing metadata for the shared settings catalog. The existing native
//! schema remains the source of values and defaults.
use clonk_core::std_config::Config;
use clonk_frontend::settings_overlay::{ApplyPolicy, Setting, SettingId, SettingsCategory};
use clonk_frontend::startup_options_advanced::{AdvancedConfigChoice, AdvancedConfigValue};

pub(crate) fn catalog(config: &Config) -> Vec<Setting> {
    let defaults = crate::advanced_config::sections(&Config::new());
    let mut settings: Vec<_> = crate::advanced_config::sections(config)
        .into_iter()
        .flat_map(|section| {
            let defaults = &defaults;
            section.rows.into_iter().map(move |row| {
                let default = defaults
                    .iter()
                    .find(|s| s.name == section.name)
                    .and_then(|s| s.rows.iter().find(|r| r.name == row.name))
                    .map(|row| row.value.clone())
                    .unwrap_or_else(|| row.value.clone());
                Setting {
                    id: SettingId::new(&section.name, &row.name),
                    label: row.name,
                    category: SettingsCategory::System,
                    keywords: section.name.clone(),
                    advanced: true,
                    value: row.value,
                    default,
                    details: Default::default(),
                }
            })
        })
        .collect();
    for setting in &mut settings {
        describe(setting);
    }
    for setting in presentation_settings(config) {
        if let Some(existing) = settings
            .iter_mut()
            .find(|existing| existing.id == setting.id)
        {
            *existing = setting;
        } else {
            settings.push(setting);
        }
    }
    settings.sort_by_key(|setting| {
        (
            setting.advanced,
            setting.category as usize,
            match (setting.id.section.as_str(), setting.id.key.as_str()) {
                ("Sound", "SoundVolume") | ("Graphics", "DisplayMode") => 0,
                ("Sound", "MusicVolume") | ("Graphics", "Scale") => 1,
                ("Voice", "Enabled") | ("Graphics", "ResolutionX") => 2,
                ("Voice", "Volume") | ("Graphics", "ResolutionY") => 3,
                ("Voice", "InputDevice") => 4,
                ("Voice", "OutputDevice") => 5,
                ("Voice", "ActivationMode") => 6,
                ("Voice", "PushToTalkKey") => 7,
                ("Voice", "ActivationThreshold") => 8,
                ("Voice", "ActivationHangover") => 9,
                _ => binding_rank(&setting.id).unwrap_or(10),
            },
            setting.label.clone(),
        )
    });
    settings
}

pub(crate) fn shortcut_key(paths: Option<&clonk_platform::AppPaths>) -> winit::keyboard::KeyCode {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("Settings"), "OpenKey")
                .and_then(|value| value.parse().ok())
        })
        .and_then(crate::input::decode_platform_key_code)
        .filter(|key| *key != winit::keyboard::KeyCode::Escape)
        .unwrap_or(winit::keyboard::KeyCode::Comma)
}

fn presentation_settings(config: &Config) -> Vec<Setting> {
    use clonk_frontend::settings_overlay::SettingDetails;
    let make =
        |section, key, label: &str, value, default, category, advanced, description: &str| {
            Setting {
                id: SettingId::new(section, key),
                label: label.into(),
                value,
                default,
                category,
                advanced,
                keywords: format!("{key} {label} presentation enhanced accessibility"),
                details: SettingDetails {
                    description: description.into(),
                    scope: "This computer".into(),
                    policy: ApplyPolicy::Restart,
                    step: 1,
                    ..Default::default()
                },
            }
        };
    let enabled = config
        .get_in(Some("Graphics"), "Remaster")
        .is_some_and(|value| matches!(value, "1" | "true"));
    let mut rows = vec![make("Graphics", "Remaster", "Enhanced graphics preset", AdvancedConfigValue::Bool(enabled), AdvancedConfigValue::Bool(false), SettingsCategory::Display, false,
        "Enable presentation enhancements. Individual settings can follow this preset or override it.")];
    for (key, label) in [
        ("HighDpiCursor", "High resolution pointer"),
        ("Mipmaps", "Texture mipmaps"),
        ("SmoothLandscape", "Smooth landscape"),
        ("FineFogOfWar", "Smooth fog of war"),
        ("HDExactBlits", "High resolution artwork"),
        ("ShaderLandscape", "Landscape shaders"),
        ("LoaderAspect", "Preserve loading image proportions"),
        ("SnapTextToPixels", "Crisp interface text"),
        ("SkyDither", "Smooth sky gradients"),
        ("SmoothPresentation", "Smooth menu animation"),
    ] {
        let choice = |value: &str| AdvancedConfigValue::Choice {
            value: value.into(),
            choices: [("Default", "Follow preset"), ("1", "On"), ("0", "Off")]
                .into_iter()
                .map(|(value, label)| AdvancedConfigChoice {
                    value: value.into(),
                    label: label.into(),
                })
                .collect(),
        };
        let value = match config.get_in(Some("Graphics"), key) {
            Some("1" | "true") => "1",
            Some("0" | "false") => "0",
            _ => "Default",
        };
        rows.push(make("Graphics", key, label, choice(value), choice("Default"), SettingsCategory::Display, true,
            "Follow the enhanced graphics preset, or override it for this computer. Takes effect after restarting Clonk."));
    }
    let detail = |value| AdvancedConfigValue::Integer {
        value,
        min: 1,
        max: i128::from(clonk_app_render::gpu_renderer::MAX_LANDSCAPE_DETAIL),
    };
    rows.push(make("Graphics", "LandscapeDetail", "Landscape detail", detail(config.get_in(Some("Graphics"), "LandscapeDetail").and_then(|value| value.parse().ok()).unwrap_or(1)), detail(1), SettingsCategory::Display, true,
        "Landscape shader detail. Higher values preserve more artwork detail and use more graphics memory."));
    let default =
        crate::input::encode_virtual_key_code(winit::keyboard::KeyCode::Comma).unwrap_or(188);
    let key = |value| AdvancedConfigValue::Integer {
        value: i128::from(value),
        min: 0,
        max: i128::from(i32::MAX),
    };
    let mut shortcut = make("Settings", "OpenKey", "Open settings shortcut", key(config.get_in(Some("Settings"), "OpenKey").and_then(|value| value.parse::<i32>().ok()).unwrap_or(default)), key(default), SettingsCategory::Controls, false,
        "Choose the key used with Ctrl (or Command on macOS) to open settings from any screen. Esc cancels capture.");
    shortcut.details.policy = ApplyPolicy::Live;
    shortcut.details.binding = true;
    rows.push(shortcut);
    rows
}

/// Keyboard sets before controllers, each set's bindings in the game's own
/// control order, after the category's other preferences.
fn binding_rank(id: &SettingId) -> Option<usize> {
    let rank = |base: usize, (set, binding): (usize, crate::input::ControlBindingId)| {
        base + set * crate::input::ControlBindingId::ALL.len() + binding as usize
    };
    keyboard_binding(id)
        .map(|found| rank(100, found))
        .or_else(|| gamepad_binding(id).map(|found| rank(200, found)))
}

pub(crate) fn keyboard_binding(id: &SettingId) -> Option<(usize, crate::input::ControlBindingId)> {
    if id.section != "Controls" {
        return None;
    }
    let (set, key) = id.key.strip_prefix("Kbd")?.split_once("Key")?;
    let set = set
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
        .filter(|s| *s < 4)?;
    let key = key.parse::<usize>().ok()?.checked_sub(1)?;
    Some((set, *crate::input::ControlBindingId::ALL.get(key)?))
}

pub(crate) fn gamepad_binding(id: &SettingId) -> Option<(usize, crate::input::ControlBindingId)> {
    let set = id
        .section
        .strip_prefix("Gamepad")?
        .parse::<usize>()
        .ok()
        .filter(|s| *s < 4)?;
    let key = id
        .key
        .strip_prefix("Button")?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)?;
    Some((set, *crate::input::ControlBindingId::ALL.get(key)?))
}

fn describe(setting: &mut Setting) {
    use ApplyPolicy::*;
    use SettingsCategory::*;
    let section = setting.id.section.as_str();
    let key = setting.id.key.as_str();
    setting.details.exclude_from_category_reset = section == "General"
        && matches!(
            key,
            "MissionAccess"
                | "Participants"
                | "FirstStart"
                | "UserPortraitsWritten"
                | "ConfigResetSafety"
                | "Version"
        );
    setting.category = match section {
        "Sound" | "Voice" => Audio,
        "Controls" => Controls,
        name if name.starts_with("Gamepad") => Controls,
        "Chat" | "Toasts" | "IRC" => Interface,
        "Graphics"
            if matches!(
                key,
                "ShowPortraits"
                    | "ShowCrewNames"
                    | "ShowCrewCNames"
                    | "ShowCommands"
                    | "ShowCommandKeys"
                    | "ShowPlayerHUDAlways"
                    | "UpperBoard"
                    | "ShowClock"
                    | "ShowStats"
                    | "MsgBoard"
                    | "SplitscreenDividers"
            ) =>
        {
            Interface
        }
        "Graphics" => Display,
        "Network" | "Lobby" => Game,
        "General"
            if matches!(
                key,
                "FPS"
                    | "Language"
                    | "LanguageEx"
                    | "FontName"
                    | "FontSize"
                    | "UseWhiteIngameChat"
                    | "UseWhiteLobbyChat"
                    | "ShowLogTimestamps"
            ) =>
        {
            Interface
        }
        "General"
            if matches!(
                key,
                "NoCrew" | "DefCrewStrength" | "Record" | "DebugMode" | "AllowScriptingInReplays"
            ) =>
        {
            Game
        }
        "General" if matches!(key, "GamepadEnabled" | "ScrollSmooth") => Controls,
        _ => System,
    };
    setting.details.policy = match (section, key) {
        ("Voice", _) | ("Chat", _) => Live,
        (
            "Sound",
            "Sound" | "Music" | "MenuMusic" | "MenuSound" | "MusicVolume" | "SoundVolume"
            | "MuteSoundCommand",
        ) => Live,
        ("Controls", "GamepadGuiControl") => Live,
        ("Graphics", "Scale" | "DisplayMode" | "ResolutionX" | "ResolutionY") => DisplayPreview,
        ("Graphics", "SmokeLevel" | "FireParticles") => NextGame,
        (
            "Graphics",
            "ShowPortraits"
            | "ShowCrewNames"
            | "ShowCrewCNames"
            | "ShowCommands"
            | "ShowCommandKeys"
            | "ShowPlayerHUDAlways"
            | "UpperBoard"
            | "ShowClock"
            | "ShowStats"
            | "MsgBoard"
            | "SplitscreenDividers"
            | "PXSGfx"
            | "PointFiltering"
            | "ShowFolderMaps"
            | "NoAlphaAdd"
            | "AllowedBlitModes"
            | "TexIndent"
            | "BlitOffset",
        ) => Live,
        (
            "General",
            "FPS" | "UseWhiteIngameChat" | "UseWhiteLobbyChat" | "ShowLogTimestamps"
            | "ScrollSmooth",
        )
        | ("Toasts", _) => Live,
        (
            "General",
            "NoCrew" | "DefCrewStrength" | "Record" | "DebugMode" | "AllowScriptingInReplays",
        )
        | ("Lobby", _) => NextGame,
        ("Network", _) | ("IRC", _) => NextConnection,
        _ => Restart,
    };
    setting.label = humanize(key);
    setting.keywords = format!("{section} {key}");
    setting.details.scope = match setting.details.policy {
        NextGame => "Defaults for your next game",
        NextConnection => "Your next connection",
        _ => "This computer",
    }
    .into();
    setting.details.step = 1;
    let metadata = match (section, key) {
        ("Sound", "MusicVolume") => Some((
            "Music volume",
            "Adjust the music without changing sound effects or voices.",
            "volume loud quiet soundtrack",
        )),
        ("Sound", "SoundVolume") => Some((
            "Sound effects volume",
            "Adjust game sound effects independently of music and voice chat.",
            "volume loud quiet sfx",
        )),
        ("Sound", "Music") => Some((
            "Game music",
            "Play music during a game.",
            "sound soundtrack mute",
        )),
        ("Sound", "Sound") => Some((
            "Game sound effects",
            "Play sound effects during a game.",
            "sfx mute",
        )),
        ("Sound", "MenuMusic") => Some((
            "Menu music",
            "Play music in menus and the lobby.",
            "startup lobby audio",
        )),
        ("Sound", "MenuSound") => Some((
            "Menu sound effects",
            "Play feedback when using menus.",
            "startup clicks audio",
        )),
        ("Voice", "Enabled") => Some((
            "Enable voice chat",
            "Hear nearby players and talk to them. Changes apply immediately.",
            "microphone mic talk mute",
        )),
        ("Voice", "Volume") => Some((
            "Voice volume",
            "Adjust other players' voices. Above 100% boosts quiet speakers.",
            "microphone mic speech loud quiet",
        )),
        ("Voice", "InputDevice") => Some((
            "Microphone",
            "Choose an input device, or follow the system default.",
            "mic input headset device",
        )),
        ("Voice", "OutputDevice") => Some((
            "Game & voice output",
            "Game audio and voice chat use the same output device.",
            "speakers headphones headset device",
        )),
        ("Voice", "ActivationMode") => Some((
            "Microphone mode",
            "Choose push to talk or voice activation. Tests are heard only on this computer.",
            "ptt mic threshold sensitivity",
        )),
        ("Voice", "PushToTalkKey") => Some((
            "Push-to-talk key",
            "Press Enter, then the key you want to use. Esc cancels.",
            "ptt mic keybind hotkey",
        )),
        ("Voice", "ActivationThreshold") => Some((
            "Activation threshold",
            "Raise this to ignore quieter sounds. Lower it if words are not detected.",
            "mic sensitivity noise gate",
        )),
        ("Voice", "ActivationHangover") => Some((
            "Release delay",
            "Milliseconds to keep transmitting after speech ends, to preserve word endings.",
            "mic hangover tail",
        )),
        ("Voice", "EchoCancellation") => Some((
            "Echo cancellation",
            "Reduce speaker audio picked up by your microphone.",
            "mic feedback",
        )),
        ("Voice", "NoiseSuppression") => Some((
            "Noise suppression",
            "Reduce steady background noise from your microphone.",
            "mic fan hum",
        )),
        ("Voice", "AutomaticGainControl") => Some((
            "Automatic mic level",
            "Keep speech at a more consistent loudness.",
            "mic agc gain",
        )),
        ("Graphics", "Scale") => Some((
            "Interface scale",
            "Preview the size of menus and game presentation; unconfirmed changes revert.",
            "ui zoom size dpi display",
        )),
        ("Graphics", "DisplayMode") => Some((
            "Window mode",
            "Switch between fullscreen and a window. Confirm to keep the change.",
            "fullscreen windowed display",
        )),
        ("Graphics", "ResolutionX") => Some((
            "Window width",
            "Requested window width in pixels. Confirm to keep the change.",
            "resolution display size",
        )),
        ("Graphics", "ResolutionY") => Some((
            "Window height",
            "Requested window height in pixels. Confirm to keep the change.",
            "resolution display size",
        )),
        ("General", "FPS") => Some((
            "Show frame rate",
            "Display the frame counter.",
            "fps performance",
        )),
        ("Graphics", "ShowStats") => Some((
            "Performance statistics",
            "Display detailed local rendering statistics.",
            "fps debug performance",
        )),
        ("Chat", "TextSize") => Some((
            "Chat text size",
            "Change the size of enhanced chat text.",
            "font accessibility reading",
        )),
        ("Chat", "Enhanced") => Some((
            "Enhanced chat",
            "Use the enhanced chat presentation.",
            "messages accessibility",
        )),
        ("Chat", "Opacity") => Some((
            "Chat opacity",
            "Change the background opacity behind chat text.",
            "contrast accessibility",
        )),
        ("Chat", "Duration") => Some((
            "Chat message duration",
            "Seconds that recent chat messages remain visible.",
            "reading accessibility timeout",
        )),
        ("General", "NoCrew") => Some((
            "Fair crew for new games",
            "Set the starting preference for games you create; the current match is unchanged.",
            "rules strength host",
        )),
        ("General", "Record") => Some((
            "Record new games",
            "Record games that start after this change. An ongoing recording is unchanged.",
            "recording replay demo save",
        )),
        ("General", "DefCrewStrength") => Some((
            "Fair crew strength",
            "Default crew strength for future games you create.",
            "rules host",
        )),
        ("General", "CompatProfile") => Some((
            "Compatibility profile",
            "Select the profile for the next application launch.",
            "legacy normal restart",
        )),
        _ => None,
    };
    if let Some((label, description, keywords)) = metadata {
        setting.label = label.into();
        setting.details.description = description.into();
        setting.keywords.push(' ');
        setting.keywords.push_str(keywords);
        setting.advanced = false;
    }
    if (section == "Voice"
        && matches!(
            key,
            "EchoCancellation" | "NoiseSuppression" | "AutomaticGainControl" | "ActivationHangover"
        ))
        || (section == "Graphics" && matches!(key, "ResolutionX" | "ResolutionY"))
    {
        setting.advanced = true;
    }
    if let Some((set, id)) = keyboard_binding(&setting.id) {
        setting.label = format!(
            "Keyboard {}: {}",
            set + 1,
            clonk_frontend::startup_options_controls::CONTROL_KEY_LABELS[id as usize]
        );
        setting.details.binding = true;
        setting.details.policy = Live;
        setting.advanced = false;
        setting.details.scope = format!("Keyboard control set {}", set + 1);
        setting.details.group = Some(format!("Keyboard {}", set + 1));
    }
    if let Some((set, id)) = gamepad_binding(&setting.id) {
        setting.label = format!(
            "Controller {}: {}",
            set + 1,
            clonk_frontend::startup_options_controls::CONTROL_KEY_LABELS[id as usize]
        );
        setting.details.binding = true;
        setting.details.policy = Live;
        setting.advanced = false;
        setting.details.scope = format!("Controller control set {}", set + 1);
        setting.details.group = Some(format!("Controller {}", set + 1));
    }
    if section == "Voice" && key == "PushToTalkKey" {
        setting.details.binding = true;
    }
    if setting.details.binding {
        if section != "Voice" {
            setting.details.description = format!(
                "{}: Enter to bind, Esc to cancel. Other control sets are independent.",
                setting.details.scope
            );
        }
        setting
            .keywords
            .push_str(" keyboard controller bind key button remap");
    }
    setting.details.unit = match (section, key) {
        ("Chat", "Duration") => " s",
        ("Chat", "Opacity") | ("Sound", "MusicVolume" | "SoundVolume") | ("Voice", "Volume") => "%",
        ("Voice", "ActivationHangover") => " ms",
        _ => "",
    }
    .into();
    let bounds = match (section, key) {
        ("Sound", "MusicVolume" | "SoundVolume") => Some((0, 100)),
        ("Graphics", "Scale") => Some((100, 400)),
        ("Graphics", "ResolutionX") => Some((640, 7680)),
        ("Graphics", "ResolutionY") => Some((480, 4320)),
        ("Graphics", "SmokeLevel") => Some((0, 300)),
        ("Network", name) if name.starts_with("Port") => Some((-1, 65535)),
        _ => None,
    };
    if let Some((low, high)) = bounds {
        for value in [&mut setting.value, &mut setting.default] {
            if let AdvancedConfigValue::Integer { min, max, .. } = value {
                *min = low;
                *max = high;
            }
        }
    }
    let choices: &[(&str, &str)] = match (section, key) {
        ("Voice", "ActivationMode") => &[
            ("PushToTalk", "Push to talk"),
            ("VoiceActivated", "Voice activation"),
        ],
        ("Graphics", "DisplayMode") => &[("Fullscreen", "Fullscreen"), ("Window", "Windowed")],
        ("Graphics", "UpperBoard") => &[
            ("Full", "Full"),
            ("Small", "Small"),
            ("Mini", "Mini"),
            ("Hide", "Hidden"),
        ],
        ("General", "CompatProfile") => &[("Normal", "Normal"), ("legacy-clonk", "LegacyClonk")],
        ("Chat", "TextSize") => &[("0", "Small"), ("1", "Medium"), ("2", "Large")],
        ("Network", "ControlMode") => {
            &[("0", "Decentral"), ("1", "Central"), ("2", "Asynchronous")]
        }
        _ => &[],
    };
    if !choices.is_empty() {
        for value in [&mut setting.value, &mut setting.default] {
            *value = AdvancedConfigValue::Choice {
                value: value.serialized(),
                choices: choices
                    .iter()
                    .map(|(value, label)| AdvancedConfigChoice {
                        value: (*value).into(),
                        label: (*label).into(),
                    })
                    .collect(),
            };
        }
    }
    if (section, key) == ("Graphics", "Scale") {
        offer_scale_steps(setting);
    }
    if !setting.value.is_editable() {
        setting.details.policy = ReadOnly;
    }
    if setting.details.description.is_empty() {
        setting.details.description = format!(
            "{} · {}. {}.",
            setting.category.label(),
            setting.label,
            setting.details.policy.label()
        );
    }
}

/// Interface scales a player picks from; a saved value off these steps
/// stays selectable so opening settings never changes it.
const SCALE_STEPS: [i128; 9] = [100, 125, 150, 175, 200, 250, 300, 350, 400];

fn offer_scale_steps(setting: &mut Setting) {
    let mut steps: Vec<i128> = [&setting.value, &setting.default]
        .into_iter()
        .filter_map(|value| value.serialized().parse().ok())
        .chain(SCALE_STEPS)
        .collect();
    steps.sort_unstable();
    steps.dedup();
    let choices: Vec<_> = steps
        .iter()
        .map(|step| AdvancedConfigChoice {
            value: step.to_string(),
            label: format!("{step}%"),
        })
        .collect();
    for value in [&mut setting.value, &mut setting.default] {
        *value = AdvancedConfigValue::Choice {
            value: value.serialized(),
            choices: choices.clone(),
        };
    }
}

fn humanize(key: &str) -> String {
    let mut result = String::new();
    let chars: Vec<_> = key.chars().collect();
    for (i, c) in chars.iter().copied().enumerate() {
        if i > 0
            && c.is_uppercase()
            && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|c| c.is_lowercase()))
        {
            result.push(' ');
        }
        result.push(c);
    }
    result
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5")
))]
mod tests {
    use super::*;
    #[test]
    fn unified_catalog_exposes_presentation_preferences_without_materializing_inherited_defaults() {
        let mut config = Config::new();
        config.set_in(Some("Graphics"), "Remaster", "1");
        let rows = catalog(&config);
        for key in [
            "Mipmaps",
            "ShaderLandscape",
            "SnapTextToPixels",
            "SmoothPresentation",
            "HighDpiCursor",
        ] {
            let row = rows
                .iter()
                .find(|row| row.id.section == "Graphics" && row.id.key == key)
                .expect("presentation preference");
            assert_eq!(row.value.serialized(), "Default");
            assert_eq!(row.details.policy, ApplyPolicy::Restart);
            assert!(config.get_in(Some("Graphics"), key).is_none());
        }
        assert!(rows.iter().any(|row| row.id.section == "Settings"
            && row.id.key == "OpenKey"
            && row.details.binding));
    }
    #[test]
    fn unified_catalog_distinguishes_live_preferences_from_session_and_restart_changes() {
        use clonk_frontend::settings_overlay::ApplyPolicy;
        let rows = catalog(&Config::new());
        for (section, key, category, policy) in [
            (
                "Voice",
                "Volume",
                SettingsCategory::Audio,
                ApplyPolicy::Live,
            ),
            (
                "Graphics",
                "Scale",
                SettingsCategory::Display,
                ApplyPolicy::DisplayPreview,
            ),
            (
                "Graphics",
                "SmokeLevel",
                SettingsCategory::Display,
                ApplyPolicy::NextGame,
            ),
            (
                "Network",
                "PortTCP",
                SettingsCategory::Game,
                ApplyPolicy::NextConnection,
            ),
            (
                "General",
                "CompatProfile",
                SettingsCategory::System,
                ApplyPolicy::Restart,
            ),
        ] {
            let setting = rows
                .iter()
                .find(|s| s.id.section == section && s.id.key == key)
                .unwrap();
            assert_eq!(
                (setting.category, setting.details.policy),
                (category, policy),
                "{section}.{key}"
            );
        }
        let binding = rows.iter().find(|s| s.id.key == "Kbd1Key1").unwrap();
        assert!(binding.details.binding);
        let volume = rows.iter().find(|s| s.id.key == "MusicVolume").unwrap();
        assert!(!volume.advanced);
        assert!(matches!(
            volume.value,
            clonk_frontend::startup_options_advanced::AdvancedConfigValue::Integer {
                min: 0,
                max: 100,
                ..
            }
        ));
    }
    #[test]
    fn unified_catalog_offers_interface_scale_in_percentage_steps() {
        let mut config = Config::new();
        config.set_in(Some("Graphics"), "Scale", "110");
        let rows = catalog(&config);
        let scale = rows
            .iter()
            .find(|row| row.id.section == "Graphics" && row.id.key == "Scale")
            .unwrap();
        let AdvancedConfigValue::Choice { value, choices } = &scale.value else {
            panic!("scale is offered as steps: {:?}", scale.value);
        };
        assert_eq!(value, "110", "an unlisted saved scale stays selectable");
        assert_eq!(
            choices
                .iter()
                .map(|choice| choice.label.as_str())
                .collect::<Vec<_>>(),
            ["100%", "110%", "125%", "150%", "175%", "200%", "250%", "300%", "350%", "400%"]
        );
        assert_eq!(scale.default.serialized(), "100");
        assert_eq!(scale.details.policy, ApplyPolicy::DisplayPreview);
        for key in ["ResolutionX", "ResolutionY"] {
            let row = rows
                .iter()
                .find(|row| row.id.section == "Graphics" && row.id.key == key)
                .unwrap();
            assert!(row.advanced, "raw window dimensions sit under Advanced");
        }
    }

    #[test]
    fn unified_catalog_groups_bindings_by_control_set_in_control_order() {
        let rows = catalog(&Config::new());
        let group = |name: &str| -> Vec<String> {
            rows.iter()
                .filter(|row| row.details.group.as_deref() == Some(name))
                .map(|row| format!("{}.{}", row.id.section, row.id.key))
                .collect()
        };
        let count = crate::input::ControlBindingId::ALL.len();
        for set in 1..=4 {
            assert_eq!(
                group(&format!("Keyboard {set}")),
                (1..=count)
                    .map(|key| format!("Controls.Kbd{set}Key{key}"))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                group(&format!("Controller {set}")),
                (1..=count)
                    .map(|key| format!("Gamepad{}.Button{key}", set - 1))
                    .collect::<Vec<_>>()
            );
        }
        let first_binding = rows
            .iter()
            .position(|row| row.details.group.is_some())
            .unwrap();
        assert_eq!(
            rows[first_binding].details.group.as_deref(),
            Some("Keyboard 1")
        );
    }

    #[test]
    fn unified_catalog_names_the_unit_of_every_measured_preference() {
        let rows = catalog(&Config::new());
        for (section, key, unit) in [
            ("Chat", "Duration", " s"),
            ("Chat", "Opacity", "%"),
            ("Sound", "MusicVolume", "%"),
            ("Sound", "SoundVolume", "%"),
            ("Voice", "Volume", "%"),
            ("Voice", "ActivationHangover", " ms"),
        ] {
            let row = rows
                .iter()
                .find(|s| s.id.section == section && s.id.key == key)
                .unwrap();
            assert_eq!(row.details.unit, unit, "{section}.{key}");
        }
    }

    #[test]
    fn unified_catalog_includes_every_advanced_setting_once() {
        let config = Config::new();
        let rows = catalog(&config);
        for section in crate::advanced_config::sections(&config) {
            for row in section.rows {
                assert_eq!(
                    rows.iter()
                        .filter(|s| s.id.section == section.name && s.id.key == row.name)
                        .count(),
                    1,
                    "{}.{}",
                    section.name,
                    row.name
                );
            }
        }
    }
}
