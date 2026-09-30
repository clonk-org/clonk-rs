use super::*;
use clonk_frontend::settings_overlay::{ApplyPolicy, Setting, SettingDetails};
use clonk_frontend::startup_options_advanced::{AdvancedConfigChoice, AdvancedConfigValue};

impl GameApp {
    pub(crate) fn refresh_unified_session_settings(&mut self) {
        let Some(owner) = self
            .unified_settings
            .as_ref()
            .map(|settings| settings.owner)
        else {
            return;
        };
        let rows = if self.mode == AppMode::Running {
            self.settings_runtime_option_rows()
        } else if self.mode == AppMode::Menu && self.startup.view == StartupView::NetworkLobby {
            self.current_classic_lobby_option_rows().unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut current: Vec<Setting> = rows
            .into_iter()
            .map(|row| {
                let selected = row
                    .choices
                    .iter()
                    .find(|choice| choice.label == row.value)
                    .map(|choice| choice.id.to_string())
                    .unwrap_or_else(|| row.value.clone());
                let value = AdvancedConfigValue::Choice {
                    value: selected,
                    choices: row
                        .choices
                        .into_iter()
                        .map(|choice| AdvancedConfigChoice {
                            value: choice.id.to_string(),
                            label: choice.label,
                        })
                        .collect(),
                };
                Setting {
                    id: SettingId::new("Session", format!("{:?}", row.kind)),
                    label: format!("This match: {}", row.caption.trim_end_matches(':')),
                    keywords: "current session host lobby server game network rules".into(),
                    category: SettingsCategory::Game,
                    advanced: false,
                    default: value.clone(),
                    value,
                    details: SettingDetails {
                        description: row.tooltip,
                        scope: "This match · synchronized host control".into(),
                        policy: ApplyPolicy::Live,
                        exclude_from_category_reset: true,
                        display_value: Some(row.value),
                        unavailable: (!row.editable).then(|| {
                            "Only the host can change this, when allowed by the match rules.".into()
                        }),
                        ..Default::default()
                    },
                }
            })
            .collect();
        if self.mode == AppMode::Running {
            let flags = self.option_flags(owner);
            let value = AdvancedConfigValue::Bool(flags.mouse);
            current.push(Setting {
                id: SettingId::new("Session", "MouseControl"), label: "Mouse control for this player".into(),
                keywords: "mouse local player input controls".into(), category: SettingsCategory::Controls,
                advanced: false, default: value.clone(), value,
                details: SettingDetails {
                    description: "Use the mouse to control this local player. Only one local player can own it.".into(),
                    scope: format!("Local player {owner}"), policy: ApplyPolicy::Live,
                    unavailable: (!flags.mouse_shown || !self.ingame_mouse.control_allowed)
                        .then(|| "This player cannot take mouse control in the current game.".into()),
                    ..Default::default()
                },
            });
        }
        if let Some(settings) = self.unified_settings.as_mut() {
            for row in &mut settings.controller.settings {
                if row.id.section == "Session" {
                    row.details.unavailable = Some("Unavailable in the current screen.".into());
                }
            }
            for row in current {
                if let Some(existing) = settings
                    .controller
                    .settings
                    .iter_mut()
                    .find(|existing| existing.id == row.id)
                {
                    *existing = row;
                } else {
                    settings.controller.settings.push(row);
                }
            }
        }
    }

    pub(crate) fn change_unified_session_setting(
        &mut self,
        key: &str,
        value: &AdvancedConfigValue,
    ) -> Result<(), EngineError> {
        if key == "MouseControl" {
            let Some(owner) = self
                .unified_settings
                .as_ref()
                .map(|settings| settings.owner)
            else {
                return Ok(());
            };
            let flags = self.option_flags(owner);
            if self.mode == AppMode::Running
                && flags.mouse_shown
                && self.ingame_mouse.control_allowed
                && matches!(value, AdvancedConfigValue::Bool(enabled) if *enabled != flags.mouse)
            {
                self.apply_ingame_menu_action_for_player(owner, MenuAction::ToggleMouseControl)?;
            }
        } else {
            let Some(option) = [
                LobbyOptionKind::ControlMode,
                LobbyOptionKind::ControlRate,
                LobbyOptionKind::RuntimeJoin,
                LobbyOptionKind::TeamDistribution,
                LobbyOptionKind::TeamColors,
                LobbyOptionKind::RandomTeamCount,
            ]
            .into_iter()
            .find(|kind| format!("{kind:?}") == key) else {
                return Ok(());
            };
            let Ok(selected) = value.serialized().parse::<i32>() else {
                return Ok(());
            };
            if self.mode == AppMode::Running {
                self.apply_runtime_client_list_option(option, selected)?;
            } else if self.mode == AppMode::Menu && self.startup.view == StartupView::NetworkLobby {
                match option {
                    LobbyOptionKind::ControlRate => {
                        self.submit_classic_lobby_control_rate(selected)
                    }
                    LobbyOptionKind::RuntimeJoin => {
                        self.set_classic_lobby_runtime_join(selected != 0)
                    }
                    LobbyOptionKind::TeamDistribution | LobbyOptionKind::TeamColors => {
                        self.submit_classic_lobby_team_setting(option, selected)
                    }
                    LobbyOptionKind::RandomTeamCount => {
                        self.set_classic_lobby_random_team_count(selected)
                    }
                    LobbyOptionKind::ControlMode => {}
                }
            }
        }
        self.refresh_unified_session_settings();
        if let Some(settings) = self.unified_settings.as_mut() {
            settings.controller.view.message = "Change requested through the game controls.".into();
        }
        Ok(())
    }
}
