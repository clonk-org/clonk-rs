//! Shared settings navigation. Values belong to the application; every view
//! (categories and search) addresses the same stable IDs.

use crate::startup_options_advanced::AdvancedConfigValue;
pub use crate::startup_options_controls::ControlDevice;

mod view;
pub use view::{SettingsAction, SettingsFocus, SettingsLayout};

/// What a binding with no key or button assigned shows.
pub const NOT_BOUND: &str = "Not bound";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SettingId {
    pub section: String,
    pub key: String,
}

impl SettingId {
    pub fn new(section: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            section: section.into(),
            key: key.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsCategory {
    Audio,
    Controls,
    Display,
    /// General: the interface, games you start, the current match and the
    /// program itself.
    Interface,
}

impl SettingsCategory {
    pub const ALL: [Self; 4] = [Self::Interface, Self::Display, Self::Audio, Self::Controls];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Controls => "Controls",
            Self::Display => "Graphics",
            Self::Interface => "General",
        }
    }

    fn book_icon(self) -> usize {
        match self {
            Self::Interface => 0,
            Self::Display => 1,
            Self::Audio => 2,
            Self::Controls => 3,
        }
    }

    /// The tab `step` places on from this one, wrapping around.
    fn step(self, step: isize) -> Self {
        let index = Self::ALL.iter().position(|c| *c == self).unwrap_or(0) as isize;
        Self::ALL[(index + step).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioPage {
    #[default]
    Sound,
    Voice,
}

impl AudioPage {
    pub const ALL: [Self; 2] = [Self::Sound, Self::Voice];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sound => "Sound",
            Self::Voice => "Voice chat",
        }
    }

    fn contains(self, setting: &Setting) -> bool {
        match self {
            Self::Sound => {
                setting.id.section != "Voice"
                    || matches!(setting.id.key.as_str(), "Volume" | "OutputDevice")
            }
            Self::Voice => setting.id.section == "Voice",
        }
    }
}

/// The Controls category's tabs: one for each kind of control set, and one
/// for its other preferences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlsPage {
    #[default]
    Keyboard,
    Controller,
    General,
}

impl ControlsPage {
    pub const ALL: [Self; 3] = [Self::Keyboard, Self::Controller, Self::General];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "Keyboard",
            Self::Controller => "Controller",
            Self::General => "General",
        }
    }

    /// The tab that shows `device`'s control sets.
    pub const fn for_device(device: ControlDevice) -> Self {
        match device {
            ControlDevice::Keyboard => Self::Keyboard,
            ControlDevice::Gamepad => Self::Controller,
        }
    }

    /// The device whose control sets the tab shows.
    pub const fn device(self) -> Option<ControlDevice> {
        match self {
            Self::Keyboard => Some(ControlDevice::Keyboard),
            Self::Controller => Some(ControlDevice::Gamepad),
            Self::General => None,
        }
    }

    fn contains(self, setting: &Setting) -> bool {
        setting.details.control.map(|binding| binding.set.device) == self.device()
    }
}

/// One of the keyboard or controller control sets players choose between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlSet {
    pub device: ControlDevice,
    pub index: usize,
}

impl ControlSet {
    pub fn label(self) -> String {
        let device = match self.device {
            ControlDevice::Keyboard => "Keyboard",
            ControlDevice::Gamepad => "Controller",
        };
        format!("{device} {}", self.index + 1)
    }
}

/// The key or button a control set assigns to one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlBinding {
    pub set: ControlSet,
    /// The command's index in `CONTROL_KEY_LABELS`.
    pub command: usize,
}

/// A slider whose positions are not the value it stores, such as fair crew
/// strength: positions `0..=positions` are converted to and from the value,
/// and `label` names a value for the player.
#[derive(Clone, Copy, Debug)]
pub struct SliderScale {
    pub positions: i128,
    pub position: fn(i128) -> i128,
    pub value: fn(i128) -> i128,
    pub label: fn(i128) -> String,
}

impl SliderScale {
    /// The value `steps` positions away from `value`, kept on the slider. A
    /// press that cannot move the slider keeps the value, even one beyond
    /// the slider's end.
    fn step(self, value: i128, steps: i128) -> i128 {
        let current = (self.position)(value).clamp(0, self.positions);
        let next = (current + steps).clamp(0, self.positions);
        if next == current {
            value
        } else {
            (self.value)(next)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplyPolicy {
    #[default]
    Live,
    DisplayPreview,
    NextGame,
    NextConnection,
    Restart,
    ReadOnly,
}

impl ApplyPolicy {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Live => "Applies now",
            Self::DisplayPreview => "Preview before keeping",
            Self::NextGame => "Next game",
            Self::NextConnection => "Next connection",
            Self::Restart => "After restart",
            Self::ReadOnly => "Read only",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SettingDetails {
    pub description: String,
    pub scope: String,
    pub policy: ApplyPolicy,
    /// The value the running game keeps using until the change applies.
    pub active_value: Option<AdvancedConfigValue>,
    pub display_value: Option<String>,
    /// The application's label for the default, when `display_value` shows
    /// the value in a form the overlay cannot derive (key and button names).
    pub default_display: Option<String>,
    pub binding: bool,
    pub unavailable: Option<String>,
    pub step: i128,
    pub exclude_from_category_reset: bool,
    /// Appended to whole-number values, such as `"%"` or `" s"`.
    pub unit: String,
    /// A control set's binding; the category lists one control set at a
    /// time.
    pub control: Option<ControlBinding>,
    /// How the setting's slider maps positions to values, when not linearly.
    pub slider: Option<SliderScale>,
}

#[derive(Clone, Debug)]
pub struct Setting {
    pub id: SettingId,
    pub label: String,
    pub keywords: String,
    pub category: SettingsCategory,
    pub advanced: bool,
    pub value: AdvancedConfigValue,
    pub default: AdvancedConfigValue,
    pub details: SettingDetails,
}

impl Setting {
    pub fn is_modified(&self) -> bool {
        self.value.serialized() != self.default.serialized()
    }
}

pub struct SettingsController {
    pub settings: Vec<Setting>,
    pub category: SettingsCategory,
    pub audio_page: AudioPage,
    pub query: String,
    pub show_advanced: bool,
    pub modified_only: bool,
    pub controls_page: ControlsPage,
    /// The control sets the Keyboard and Controller tabs show.
    pub keyboard_set: usize,
    pub controller_set: usize,
    /// The local players' names by the control set each one plays with.
    pub set_users: Vec<(ControlSet, String)>,
    pub view: view::SettingsViewState,
}

impl SettingsController {
    pub fn new(settings: Vec<Setting>) -> Self {
        Self {
            settings,
            category: SettingsCategory::Interface,
            audio_page: AudioPage::Sound,
            query: String::new(),
            show_advanced: false,
            modified_only: false,
            controls_page: ControlsPage::Keyboard,
            keyboard_set: 0,
            controller_set: 0,
            set_users: Vec::new(),
            view: Default::default(),
        }
    }

    /// Whether the setting differs from the value Reset would restore.
    pub fn is_modified(&self, index: usize) -> bool {
        self.settings.get(index).is_some_and(Setting::is_modified)
    }

    /// The control sets on the current page, in catalog order.
    pub fn control_sets(&self) -> Vec<ControlSet> {
        self.settings
            .iter()
            .filter(|setting| setting.category == self.category && self.page_contains(setting))
            .filter_map(|setting| setting.details.control.map(|binding| binding.set))
            .fold(Vec::new(), |mut sets, set| {
                if !sets.contains(&set) {
                    sets.push(set);
                }
                sets
            })
    }

    /// The control set whose bindings the page shows: the one chosen for
    /// its device while the page has it, otherwise the page's first.
    pub fn current_control_set(&self) -> Option<ControlSet> {
        let sets = self.control_sets();
        self.controls_page
            .device()
            .map(|device| ControlSet {
                device,
                index: match device {
                    ControlDevice::Keyboard => self.keyboard_set,
                    ControlDevice::Gamepad => self.controller_set,
                },
            })
            .filter(|set| sets.contains(set))
            .or_else(|| sets.into_iter().next())
    }

    /// Shows `set` on its device's tab from now on.
    pub fn choose_set(&mut self, set: ControlSet) {
        match set.device {
            ControlDevice::Keyboard => self.keyboard_set = set.index,
            ControlDevice::Gamepad => self.controller_set = set.index,
        }
    }

    /// Whether a setting of the current category sits on its current tab,
    /// whatever the filters.
    fn page_contains(&self, setting: &Setting) -> bool {
        match self.category {
            SettingsCategory::Audio => self.audio_page.contains(setting),
            SettingsCategory::Controls => self.controls_page.contains(setting),
            _ => true,
        }
    }

    pub fn parse_value(&self, index: usize, text: &str) -> Result<AdvancedConfigValue, String> {
        let setting = self.settings.get(index).ok_or("Setting unavailable")?;
        match &setting.value {
            AdvancedConfigValue::Integer { min, max, .. } => {
                let value = text
                    .trim()
                    .parse::<i128>()
                    .map_err(|_| "Enter a whole number")?;
                if (*min..=*max).contains(&value) {
                    Ok(AdvancedConfigValue::Integer {
                        value,
                        min: *min,
                        max: *max,
                    })
                } else {
                    Err(format!("Enter a value between {min} and {max}"))
                }
            }
            AdvancedConfigValue::Text(_)
                if text.len() <= 254 && !text.contains(['\n', '\r', '\0']) =>
            {
                Ok(AdvancedConfigValue::Text(text.into()))
            }
            _ => Err("This setting cannot be edited as text".into()),
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        let shown_set = self.current_control_set();
        // A control set's grid keeps every key in its place.
        let changed_only = self.modified_only && self.control_set_page().is_none();
        self.settings
            .iter()
            .enumerate()
            .filter_map(|(index, setting)| {
                let matches = if query.is_empty() {
                    setting.category == self.category
                        && (!setting.advanced || self.show_advanced)
                        && (self.category != SettingsCategory::Audio
                            || self.audio_setting_visible(setting))
                        && (self.category != SettingsCategory::Controls
                            || self.controls_page.contains(setting))
                        && setting
                            .details
                            .control
                            .is_none_or(|binding| Some(binding.set) == shown_set)
                } else {
                    let searchable = format!(
                        "{} {} {} {}",
                        setting.label, setting.keywords, setting.id.section, setting.id.key
                    )
                    .to_lowercase();
                    query
                        .split_whitespace()
                        .all(|word| searchable.contains(word))
                };
                (matches && (!changed_only || setting.is_modified())).then_some(index)
            })
            .collect()
    }

    fn audio_setting_visible(&self, setting: &Setting) -> bool {
        if !self.audio_page.contains(setting) {
            return false;
        }
        if self.audio_page == AudioPage::Sound || self.show_advanced {
            return true;
        }
        let activated = self
            .settings
            .iter()
            .find(|setting| setting.id.section == "Voice" && setting.id.key == "ActivationMode")
            .is_some_and(|setting| {
                matches!(setting.value.serialized().as_str(), "VoiceActivated" | "1")
            });
        match setting.id.key.as_str() {
            "PushToTalkKey" => !activated,
            "ActivationThreshold" | "ActivationHangover" => activated,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_tabs_hold_every_setting_under_the_names_players_expect() {
        assert_eq!(
            SettingsCategory::ALL.map(SettingsCategory::label),
            ["General", "Graphics", "Audio", "Controls"]
        );
    }

    #[test]
    fn voice_page_shows_relevant_activation_controls_without_hiding_them_from_search() {
        let mut controller = SettingsController::new(
            [
                ("ActivationMode", "PushToTalk"),
                ("PushToTalkKey", "86"),
                ("ActivationThreshold", "40"),
            ]
            .into_iter()
            .map(|(key, value)| Setting {
                id: SettingId::new("Voice", key),
                label: key.into(),
                keywords: String::new(),
                category: SettingsCategory::Audio,
                advanced: false,
                value: AdvancedConfigValue::Text(value.into()),
                default: AdvancedConfigValue::Text(value.into()),
                details: Default::default(),
            })
            .collect(),
        );
        controller.select_audio_page(AudioPage::Voice);
        assert_eq!(controller.visible_indices(), vec![0, 1]);
        controller.settings[0].value = AdvancedConfigValue::Text("VoiceActivated".into());
        assert_eq!(controller.visible_indices(), vec![0, 2]);
        controller.query = "PushToTalkKey".into();
        assert_eq!(controller.visible_indices(), vec![1]);
    }

    #[test]
    fn audio_opens_the_sound_mix_without_microphone_setup() {
        let mut controller = SettingsController::new(
            [
                ("Sound", "MusicVolume"),
                ("Voice", "Enabled"),
                ("Voice", "Volume"),
                ("Voice", "OutputDevice"),
            ]
            .into_iter()
            .map(|(section, key)| Setting {
                id: SettingId::new(section, key),
                label: key.into(),
                keywords: String::new(),
                category: SettingsCategory::Audio,
                advanced: false,
                value: AdvancedConfigValue::Bool(true),
                default: AdvancedConfigValue::Bool(true),
                details: Default::default(),
            })
            .collect(),
        );
        controller.select_category(SettingsCategory::Audio);
        assert_eq!(controller.visible_indices(), vec![0, 2, 3]);
    }

    #[test]
    fn numeric_edits_validate_bounds_without_changing_the_live_value() {
        let value = AdvancedConfigValue::Integer {
            value: 40,
            min: 0,
            max: 100,
        };
        let controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Voice", "ActivationThreshold"),
            label: "Threshold".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value.clone(),
            details: Default::default(),
        }]);
        assert_eq!(controller.parse_value(0, "75").unwrap().serialized(), "75");
        assert!(controller.parse_value(0, "101").is_err());
        assert!(controller.parse_value(0, "NaN").is_err());
        assert_eq!(controller.settings[0].value, value);
    }

    #[test]
    fn only_changed_keeps_the_settings_that_differ_from_their_defaults() {
        let value = AdvancedConfigValue::Bool(true);
        let setting = |key: &str| Setting {
            id: SettingId::new("Sound", key),
            label: key.into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value.clone(),
            details: Default::default(),
        };
        let mut controller = SettingsController::new(vec![setting("Music"), setting("Sound")]);
        controller.select_category(SettingsCategory::Audio);
        assert_eq!(controller.visible_indices(), vec![0, 1]);
        controller.settings[0].value = AdvancedConfigValue::Bool(false);
        controller.modified_only = true;
        assert_eq!(controller.visible_indices(), vec![0]);
    }

    #[test]
    fn search_finds_advanced_settings_across_categories_by_alias() {
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Voice", "ActivationThreshold"),
            label: "Microphone activation threshold".into(),
            keywords: "mic sensitivity".into(),
            category: SettingsCategory::Audio,
            advanced: true,
            value: AdvancedConfigValue::Integer {
                value: 40,
                min: 0,
                max: 100,
            },
            default: AdvancedConfigValue::Integer {
                value: 40,
                min: 0,
                max: 100,
            },
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Display;
        controller.query = "MIC sensitivity".into();
        assert_eq!(controller.visible_indices(), vec![0]);
    }
}
