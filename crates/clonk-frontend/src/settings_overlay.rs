//! Shared settings navigation. Values belong to the application; every view
//! (categories, search and pinned settings) addresses the same stable IDs.

use crate::startup_options_advanced::AdvancedConfigValue;

mod view;
pub use view::{SettingsAction, SettingsFocus, SettingsLayout};

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
    Quick,
    Audio,
    Controls,
    Display,
    Interface,
    Game,
    System,
}

impl SettingsCategory {
    pub const ALL: [Self; 7] = [
        Self::Quick,
        Self::Audio,
        Self::Controls,
        Self::Display,
        Self::Interface,
        Self::Game,
        Self::System,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quick => "Quick settings",
            Self::Audio => "Audio",
            Self::Controls => "Controls",
            Self::Display => "Display",
            Self::Interface => "Interface",
            Self::Game => "Game",
            Self::System => "System",
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
    pub active_value: Option<String>,
    pub display_value: Option<String>,
    pub binding: bool,
    pub unavailable: Option<String>,
    pub step: i128,
    pub exclude_from_category_reset: bool,
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

pub struct SettingsController {
    pub settings: Vec<Setting>,
    pub category: SettingsCategory,
    pub query: String,
    pub show_advanced: bool,
    pub pinned: Vec<SettingId>,
    pub modified_only: bool,
    pub view: view::SettingsViewState,
}

impl SettingsController {
    pub fn new(settings: Vec<Setting>) -> Self {
        Self {
            settings,
            category: SettingsCategory::Quick,
            query: String::new(),
            show_advanced: false,
            pinned: Vec::new(),
            modified_only: false,
            view: Default::default(),
        }
    }

    pub fn toggle_pin(&mut self, index: usize) {
        if let Some(setting) = self.settings.get(index) {
            if let Some(position) = self.pinned.iter().position(|id| *id == setting.id) {
                self.pinned.remove(position);
            } else {
                self.pinned.push(setting.id.clone());
            }
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
        let mut indices: Vec<_> = self
            .settings
            .iter()
            .enumerate()
            .filter_map(|(index, setting)| {
                let matches = if query.is_empty() {
                    if self.category == SettingsCategory::Quick {
                        self.pinned.contains(&setting.id)
                    } else {
                        setting.category == self.category
                            && (!setting.advanced || self.show_advanced)
                    }
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
                (matches
                    && (!self.modified_only
                        || setting.value.serialized() != setting.default.serialized()))
                .then_some(index)
            })
            .collect();
        if query.is_empty() && self.category == SettingsCategory::Quick {
            indices.sort_by_key(|index| {
                self.pinned
                    .iter()
                    .position(|id| *id == self.settings[*index].id)
            });
        }
        indices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn quick_settings_share_values_and_preserve_personal_order() {
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
        controller.toggle_pin(1);
        controller.toggle_pin(0);
        assert_eq!(controller.visible_indices(), vec![1, 0]);
        controller.settings[0].value = AdvancedConfigValue::Bool(false);
        controller.modified_only = true;
        assert_eq!(controller.visible_indices(), vec![0]);
        controller.toggle_pin(0);
        assert!(controller.visible_indices().is_empty());
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
