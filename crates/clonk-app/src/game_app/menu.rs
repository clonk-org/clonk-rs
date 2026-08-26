//! `impl GameApp` — runtime menus & dialogs methods.
//!
//! This remains an `impl GameApp` module so it can share private application
//! state. Extracting independently owned runtime menu and dialog state is
//! tracked by clonk-org/clonk-rs#1236.

use super::*;

impl GameApp {
    pub(crate) fn open_league_signup_dialog(
        &mut self,
        mode: clonk_frontend::league_signup::LeagueSignupMode,
        auth: clonk_network::LeagueAuthRequestHead,
        continuation: LeaguePlayerAuthContinuation,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "C4LeagueSignupDialog",
            self.assets
                .league_signup_resources()
                .context("exact C4LeagueSignupDialog resource set is absent")
                .and_then(|resources| resources.validate()),
        )?;
        let player_name = Self::league_auth_continuation_player_name(&continuation);
        let server_name = Self::league_auth_continuation_server_name(&continuation).to_owned();
        let account_preference = match mode {
            clonk_frontend::league_signup::LeagueSignupMode::Login => {
                legacy_presentation_text(auth.account.as_bytes())
            }
            clonk_frontend::league_signup::LeagueSignupMode::Registration => {
                legacy_presentation_text(load_network_nick(self.app_paths.as_ref()).as_bytes())
            }
        };
        let password_preference = match mode {
            clonk_frontend::league_signup::LeagueSignupMode::Login => {
                legacy_presentation_text(auth.password.as_bytes())
            }
            clonk_frontend::league_signup::LeagueSignupMode::Registration => String::new(),
        };
        let config =
            clonk_frontend::league_signup::LeagueSignupConfig::new(player_name, server_name, mode)
                .with_preferences(account_preference, password_preference);
        let pointer_position = self.running_pointer_position;
        self.close_context_menu_silently();
        self.cancel_underlying_interaction();
        self.league_signup_consumed_keys.clear();
        self.league_signup_pointer_capture = false;
        // MouseInput carries no coordinates. Retain the application's last
        // pointer so a stationary cursor can click the newly opened dialog.
        self.league_signup_pointer_position = pointer_position;
        self.league_signup_dialog = Some(PendingLeagueSignupDialog {
            controller: clonk_frontend::league_signup::LeagueSignupController::new(
                config,
                self.league_signup_strings(),
            ),
            auth,
            continuation,
        });
        Ok(())
    }

    pub(crate) fn open_scenario_mission_access_dialog(&mut self) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "Mission Access input dialog",
            self.assets.input_dialog_resources().map(|_| ()),
        )?;
        self.close_context_menu_silently();
        self.scenario_game_options.cancel_interaction();
        let controller = InputDialogController::new(
            "Enter mission password:",
            "Mission Access",
            InputDialogIcon::OPTIONS,
        );
        self.startup_tooltip.pointer_left();
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::ScenarioMissionAccess,
            controller,
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        Ok(())
    }

    pub(crate) fn open_scenario_delete_dialog(&mut self) -> Result<(), EngineError> {
        self.menu_state.abort_renaming();
        let Some(selected) = self.menu_state.selected_scenario().cloned() else {
            return Ok(());
        };
        let distinct_sources = selected
            .source_paths
            .iter()
            .map(|source| scenario_root_key(source))
            .collect::<HashSet<_>>();
        if distinct_sources.len() > 1 {
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Delete failure.",
                    "Delete",
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                ),
                MessageDialogContinuation::None,
            )?;
            return Ok(());
        }
        let Some(path) = selected.path.clone() else {
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Delete failure.",
                    "Delete",
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                ),
                MessageDialogContinuation::None,
            )?;
            return Ok(());
        };
        let next_identifier = self
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == selected.identifier)
            .and_then(|index| self.menu_state.visible_entries().get(index + 1))
            .map(|entry| entry.identifier.clone());
        let type_name = match selected.kind {
            ScenarioKind::Scenario => "Scenario",
            ScenarioKind::Folder
                if path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f")) =>
            {
                "Scenario folder"
            }
            _ => "Directory",
        };
        let subject = format!("{type_name} {}", selected.title);
        let warning = if scenario_storage_is_original(&path) {
            format!("{subject} is an original file. Are your sure you want to delete it?")
        } else {
            format!("Delete {subject}?")
        };
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                warning,
                "Delete",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::DeleteScenario {
                path,
                next_identifier,
            },
        )
    }

    pub(crate) fn refresh_network_chart_dialog(&mut self) {
        let Some(aliases) = self.network_chart_dialog.as_ref().map(|dialog| {
            dialog
                .tab_names()
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        }) else {
            return;
        };
        let network_input_title = self.runtime_resource_text("IDS_NET_INPUT", "Network Input");
        let network_output_title = self.runtime_resource_text("IDS_NET_OUTPUT", "Network Output");
        let Some(stats) = self.network_stats.as_mut() else {
            return;
        };
        stats.update();
        let snapshots = aliases
            .iter()
            .filter_map(|alias| {
                stats.graph_by_name(alias).map(|graph| {
                    (
                        alias.clone(),
                        Self::network_chart_graph_snapshot(
                            graph,
                            &network_input_title,
                            &network_output_title,
                        ),
                    )
                })
            })
            .collect::<Vec<_>>();
        if let Some(dialog) = self.network_chart_dialog.as_mut() {
            for (alias, graph) in snapshots {
                dialog.set_graph_snapshot(&alias, graph);
            }
        }
    }

    pub(crate) fn runtime_default_dialog_visible(&self, dialog: RuntimeDefaultDialog) -> bool {
        match dialog {
            RuntimeDefaultDialog::Scoreboard => self.scoreboard_dialog.is_some(),
            RuntimeDefaultDialog::NetworkChart => self.network_chart_dialog.is_some(),
            RuntimeDefaultDialog::ClientList => self.runtime_client_list.is_some(),
            RuntimeDefaultDialog::GameOver => self.game_over_dialog.is_some(),
            RuntimeDefaultDialog::ExternalIrc => self.external_irc_dialog_visible,
        }
    }

    /// Returns a complete bottom-to-top projection. The fallback entries keep
    /// older direct-state fixtures deterministic; production show/close paths
    /// populate the retained order explicitly.
    pub(crate) fn runtime_default_dialog_order_snapshot(&self) -> Vec<RuntimeDefaultDialog> {
        let mut order = self
            .runtime_default_dialog_order
            .iter()
            .copied()
            .filter(|dialog| self.runtime_default_dialog_visible(*dialog))
            .collect::<Vec<_>>();
        let fallback = if self.runtime_client_list_above_game_over {
            [
                RuntimeDefaultDialog::Scoreboard,
                RuntimeDefaultDialog::NetworkChart,
                RuntimeDefaultDialog::GameOver,
                RuntimeDefaultDialog::ClientList,
                RuntimeDefaultDialog::ExternalIrc,
            ]
        } else {
            [
                RuntimeDefaultDialog::Scoreboard,
                RuntimeDefaultDialog::NetworkChart,
                RuntimeDefaultDialog::ClientList,
                RuntimeDefaultDialog::GameOver,
                RuntimeDefaultDialog::ExternalIrc,
            ]
        };
        for dialog in fallback {
            if self.runtime_default_dialog_visible(dialog) && !order.contains(&dialog) {
                order.push(dialog);
            }
        }
        order
    }

    fn sync_runtime_default_dialog_compatibility(&mut self) {
        let order = self.runtime_default_dialog_order_snapshot();
        let client = order
            .iter()
            .position(|dialog| *dialog == RuntimeDefaultDialog::ClientList);
        let game_over = order
            .iter()
            .position(|dialog| *dialog == RuntimeDefaultDialog::GameOver);
        self.runtime_client_list_above_game_over = client
            .zip(game_over)
            .is_some_and(|(client, game_over)| client > game_over);
    }

    pub(crate) fn show_or_raise_runtime_default_dialog(&mut self, dialog: RuntimeDefaultDialog) {
        let mut order = self.runtime_default_dialog_order_snapshot();
        order.retain(|candidate| *candidate != dialog);
        order.push(dialog);
        if self.network_chart_elevated
            && dialog != RuntimeDefaultDialog::NetworkChart
            && self.network_chart_dialog.is_some()
        {
            order.retain(|candidate| *candidate != RuntimeDefaultDialog::NetworkChart);
            order.push(RuntimeDefaultDialog::NetworkChart);
        }
        self.runtime_default_dialog_order = order;
        self.sync_runtime_default_dialog_compatibility();
    }

    pub(crate) fn activate_runtime_default_dialog(&mut self, dialog: RuntimeDefaultDialog) {
        match dialog {
            RuntimeDefaultDialog::Scoreboard => {
                self.activate_running_dialog_stack_only(RunningDialogStackEntry::Scoreboard);
            }
            RuntimeDefaultDialog::ClientList => {
                self.activate_running_dialog_stack_only(RunningDialogStackEntry::RuntimeClientList);
            }
            RuntimeDefaultDialog::NetworkChart
            | RuntimeDefaultDialog::GameOver
            | RuntimeDefaultDialog::ExternalIrc => {}
        }
        self.show_or_raise_runtime_default_dialog(dialog);
        if dialog == RuntimeDefaultDialog::NetworkChart {
            self.network_chart_elevated =
                !self.message_dialogs.is_empty() || self.running_chat_controller().is_some();
            if self.network_chart_elevated {
                self.message_dialog_active_index = None;
                self.set_running_chat_active(false);
            }
        } else {
            if self.network_chart_elevated {
                let mut order = self.runtime_default_dialog_order_snapshot();
                order.retain(|candidate| *candidate != dialog);
                order.push(dialog);
                self.runtime_default_dialog_order = order;
            }
            self.network_chart_elevated = false;
            self.sync_runtime_default_dialog_compatibility();
        }
    }

    pub(crate) fn hide_runtime_default_dialog(&mut self, dialog: RuntimeDefaultDialog) {
        let chart_was_elevated =
            dialog == RuntimeDefaultDialog::NetworkChart && self.network_chart_elevated;
        let chart_owned_input = chart_was_elevated && self.network_chart_elevated_owns_input();
        let mut order = self.runtime_default_dialog_order_snapshot();
        order.retain(|candidate| *candidate != dialog);
        self.runtime_default_dialog_order = order;
        if dialog == RuntimeDefaultDialog::NetworkChart {
            self.network_chart_elevated = false;
            if chart_owned_input {
                match self.running_active_dialog {
                    Some(RunningDialogStackEntry::Chat) if self.running_chat.is_some() => {
                        self.set_running_chat_active(true);
                    }
                    Some(RunningDialogStackEntry::Message(stack_id)) => {
                        self.message_dialog_active_index = self.running_message_index(stack_id);
                    }
                    _ => {
                        self.message_dialog_active_index = None;
                    }
                }
            }
        }
        self.sync_runtime_default_dialog_compatibility();
    }

    pub(crate) fn reset_runtime_default_dialog_order(&mut self) {
        self.runtime_default_dialog_order.clear();
        self.network_chart_elevated = false;
        if self.external_irc_dialog_visible {
            self.runtime_default_dialog_order
                .push(RuntimeDefaultDialog::ExternalIrc);
        }
        self.runtime_client_list_above_game_over = false;
    }

    pub(crate) fn runtime_default_dialog_is_top(&self, dialog: RuntimeDefaultDialog) -> bool {
        self.runtime_default_dialog_order_snapshot().last().copied() == Some(dialog)
    }

    pub(crate) fn runtime_top_default_dialog_is_exclusive(&self) -> bool {
        matches!(
            self.runtime_default_dialog_order_snapshot().last(),
            Some(RuntimeDefaultDialog::GameOver | RuntimeDefaultDialog::ExternalIrc)
        )
    }

    pub(crate) fn runtime_default_dialog_is_above(
        &self,
        upper: RuntimeDefaultDialog,
        lower: RuntimeDefaultDialog,
    ) -> bool {
        let order = self.runtime_default_dialog_order_snapshot();
        order
            .iter()
            .position(|dialog| *dialog == upper)
            .zip(order.iter().position(|dialog| *dialog == lower))
            .is_some_and(|(upper, lower)| upper > lower)
    }

    pub(crate) fn network_chart_is_active_dialog(&self) -> bool {
        matches!(self.mode, AppMode::Running)
            && self.network_chart_dialog.is_some()
            && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart)
            && self.context_menu.is_none()
            && !self.runtime_modal_above_network_chart()
            && (self.network_chart_elevated_owns_input()
                || (self.message_dialogs.is_empty() && self.game_option_input_dialog.is_none()))
    }

    fn set_running_active_dialog(&mut self, entry: Option<RunningDialogStackEntry>) {
        self.running_active_dialog = entry;
        if let Some(chat) = self.running_chat.as_mut() {
            chat.active = entry == Some(RunningDialogStackEntry::Chat);
        }
    }

    pub(crate) fn show_running_dialog(&mut self, entry: RunningDialogStackEntry) {
        if self.mode != AppMode::Running {
            return;
        }
        self.running_dialog_stack
            .retain(|current| *current != entry);
        let z_order = entry.z_order();
        let previous_len = self.running_dialog_stack.len();
        let index = self
            .running_dialog_stack
            .iter()
            .position(|current| current.z_order() > z_order)
            .unwrap_or(self.running_dialog_stack.len());
        self.running_dialog_stack.insert(index, entry);
        if index == previous_len {
            self.set_running_active_dialog(Some(entry));
        }
    }

    pub(crate) fn remove_running_dialog(&mut self, entry: RunningDialogStackEntry) {
        let was_named_active = self.running_active_dialog == Some(entry);
        let chart_owned_input = self.network_chart_elevated_owns_input();
        let was_active = was_named_active && !chart_owned_input;
        if was_active {
            // Screen::CloseDialog releases the one screen-global drag element
            // and aborts its context menu before selecting the next active
            // dialog. The element may belong to a lower dialog that began a
            // drag before this one was shown asynchronously.
            self.release_all_running_pointer_elements();
        }
        self.running_dialog_stack
            .retain(|current| *current != entry);
        if was_named_active {
            let next = self.running_dialog_stack.last().copied();
            if self.network_chart_elevated {
                // Keep the shared Screen projection current without granting
                // its successor input through the visually higher chart.
                self.running_active_dialog = next;
            } else {
                self.set_running_active_dialog(next);
            }
        }
    }

    fn activate_running_dialog_stack_only(&mut self, entry: RunningDialogStackEntry) {
        if !self.running_dialog_stack.contains(&entry) {
            return;
        }
        if entry.z_order() != 0 {
            self.set_running_active_dialog(Some(entry));
            return;
        }
        // Screen::ActivateDialog uses MakeLastElement for default-z dialogs,
        // even when this carries them past specially ordered input/chat
        // dialogs. Subsequent ShowDialog insertion observes that exact order.
        self.running_dialog_stack
            .retain(|current| *current != entry);
        self.running_dialog_stack.push(entry);
        self.set_running_active_dialog(Some(entry));
    }

    pub(crate) fn activate_running_dialog(&mut self, entry: RunningDialogStackEntry) {
        match entry {
            RunningDialogStackEntry::Scoreboard => {
                self.activate_runtime_default_dialog(RuntimeDefaultDialog::Scoreboard);
            }
            RunningDialogStackEntry::RuntimeClientList => {
                self.activate_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
            }
            RunningDialogStackEntry::Message(_) | RunningDialogStackEntry::Chat => {
                self.activate_running_dialog_stack_only(entry);
            }
        }
    }

    pub(crate) fn running_dialog_is_above(
        &self,
        candidate: RunningDialogStackEntry,
        other: RunningDialogStackEntry,
    ) -> bool {
        let candidate = self
            .running_dialog_stack
            .iter()
            .rposition(|entry| *entry == candidate);
        let other = self
            .running_dialog_stack
            .iter()
            .rposition(|entry| *entry == other);
        matches!((candidate, other), (Some(candidate), Some(other)) if candidate > other)
    }

    pub(crate) fn top_runtime_default_dialog_at(
        &mut self,
        point: GuiPoint,
    ) -> Result<Option<RuntimeDefaultDialog>, EngineError> {
        for dialog in self
            .runtime_default_dialog_order_snapshot()
            .into_iter()
            .rev()
        {
            if self.runtime_default_dialog_contains_point(dialog, point)? {
                return Ok(Some(dialog));
            }
        }
        Ok(None)
    }

    pub(crate) fn open_scoreboard_dialog(&mut self, request: ScoreboardPresentationRequest) {
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        let layout_revision = request.layout_revision;
        self.scoreboard_dialog = Some(request);
        self.scoreboard_close_pointer_capture = false;
        self.scoreboard_runtime = ScoreboardDialogRuntime {
            layout_revision,
            preferred: Some(preferred),
            ..ScoreboardDialogRuntime::default()
        };
        self.show_running_dialog(RunningDialogStackEntry::Scoreboard);
        self.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::Scoreboard);
    }

    pub(crate) fn close_scoreboard_dialog(&mut self) -> bool {
        let closed = self.scoreboard_dialog.take().is_some();
        self.scoreboard_pointer_left();
        self.remove_running_dialog(RunningDialogStackEntry::Scoreboard);
        self.hide_runtime_default_dialog(RuntimeDefaultDialog::Scoreboard);
        self.scoreboard_runtime = ScoreboardDialogRuntime::default();
        closed
    }

    pub(crate) fn game_over_dialog_contains_point(&self, point: GuiPoint) -> bool {
        let surface = self.graphics.surface();
        self.game_over_dialog.as_ref().is_some_and(|dialog| {
            dialog.classic_dialog_contains_point(
                point.x,
                point.y,
                surface.width(),
                surface.height(),
            )
        })
    }

    pub(crate) fn runtime_default_dialog_contains_point(
        &mut self,
        dialog: RuntimeDefaultDialog,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        Ok(match dialog {
            RuntimeDefaultDialog::Scoreboard => self.scoreboard_pointer_target(point)?.is_some(),
            RuntimeDefaultDialog::NetworkChart => self.network_chart_contains_point(point),
            RuntimeDefaultDialog::ClientList => self.runtime_client_list_contains_point(point),
            RuntimeDefaultDialog::GameOver => self.game_over_dialog_contains_point(point),
            RuntimeDefaultDialog::ExternalIrc => self.external_irc_dialog_contains_point(point),
        })
    }

    pub(crate) fn game_over_dialog_is_active(&self) -> bool {
        self.game_over_dialog_is_mouse_active() && self.context_menu.is_none()
    }

    /// Menu commands on the key-input path share the control fail-safe:
    /// a script error inside a menu action logs, becomes a status line and
    /// counts as menu-consumed — C++ shows the error and keeps the session
    /// alive (C4AulExec.cpp:1345-1361); only engine-model errors stay fatal.
    pub(crate) fn handle_menu_command_failsafe(
        &mut self,
        owner: i32,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Result<bool, EngineError> {
        self.handle_menu_command(owner, command, kind)
            .or_else(|err| {
                let status = control_script_error_to_status(err)?;
                tracing::error!(status, "control script error (non-fatal like C++)");
                self.status_text = status;
                Ok(true)
            })
    }

    pub(crate) fn ingame_menu_belongs_to(&self, owner: i32) -> bool {
        self.ingame_menu.contains(owner)
    }

    pub(crate) fn menu_controls_active_for(&self, owner: i32) -> bool {
        matches!(self.mode, AppMode::Running)
            && (self.ingame_menu_belongs_to(owner)
                || (owner == self.local_owner && self.object_menu.is_some()))
    }

    /// Opens the player menu (`C4Player::ActivateMenuMain` ->
    /// `C4MainMenu::ActivateMain`, C4Player.cpp:2327 + C4MainMenu.cpp:643).
    pub(crate) fn open_ingame_menu(&mut self) -> Result<(), EngineError> {
        self.open_ingame_menu_for_player(self.local_owner)
    }

    pub(crate) fn open_ingame_menu_for_player(&mut self, player: i32) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running) || self.ingame_menu.contains(player) {
            return Ok(());
        }
        self.activate_ingame_main_menu_for_player(player)
    }

    fn activate_hostility_menu_for_player(&mut self, player: i32) {
        let menu = self
            .hostility_entries_for_player(player)
            .map(|entries| IngameMenuState::hostility_menu(&entries, &self.ingame_menu_labels()));
        self.ingame_menu.replace(player, menu);
    }

    /// Resolve every in-game menu string through the active language table,
    /// the way `C4MainMenu`'s `LoadResStr` calls do at page-construction time
    /// (C4MainMenu.cpp:59-732; C4Player.cpp:1801). A key missing from the
    /// table falls back to its shipped `LanguageUS.txt` value, matching
    /// C4ResStrTable.
    /// The portrait selector's visible strings, resolved like every other
    /// startup caption (`C4FileSelDlg.cpp:142,439,535,568-571`). The title is
    /// `IDS_MSG_SELECT` ("Select %s") filled with `IDS_TYPE_PORTRAIT`.
    pub(crate) fn portrait_sel_labels(
        &self,
    ) -> clonk_frontend::startup_portraitsel::PortraitSelLabels {
        use clonk_frontend::startup_portraitsel::PortraitSelLabels;
        let defaults = PortraitSelLabels::default();
        let portrait = self.runtime_resource_text("IDS_TYPE_PORTRAIT", "Portrait");
        let select = self.runtime_resource_text("IDS_MSG_SELECT", "Select %s");
        PortraitSelLabels {
            select_portrait: select.replacen("%s", &portrait, 1),
            location: self.runtime_resource_text("IDS_TEXT_LOCATION", &defaults.location),
            import_image_as: self
                .runtime_resource_text("IDS_CTL_IMPORTIMAGEAS", &defaults.import_image_as),
            player_image: self
                .runtime_resource_text("IDS_TEXT_PLAYERIMAGE", &defaults.player_image),
            lobby_icon: self.runtime_resource_text("IDS_TEXT_LOBBYICON", &defaults.lobby_icon),
            no_portrait: self.runtime_resource_text("IDS_MSG_NOPORTRAIT", &defaults.no_portrait),
            loading: self.runtime_resource_text("IDS_PRC_INITIALIZE", &defaults.loading),
            ok: self.runtime_resource_text("IDS_DLG_OK", &defaults.ok),
            cancel: self.runtime_resource_text("IDS_DLG_CANCEL", &defaults.cancel),
        }
    }

    pub(crate) fn ingame_menu_labels(&self) -> IngameMenuLabels {
        let defaults = IngameMenuLabels::default();
        IngameMenuLabels {
            main_caption: self.runtime_resource_text("IDS_MENU_CPMAIN", &defaults.main_caption),
            observer_caption: self
                .runtime_resource_text("IDS_MENU_OBSERVER", &defaults.observer_caption),
            goals: self.runtime_resource_text("IDS_MENU_CPGOALS", &defaults.goals),
            goals_info: self.runtime_resource_text("IDS_MENU_CPGOALSINFO", &defaults.goals_info),
            rules: self.runtime_resource_text("IDS_MENU_CPRULES", &defaults.rules),
            rules_info: self.runtime_resource_text("IDS_MENU_CPRULESINFO", &defaults.rules_info),
            view: self.runtime_resource_text("IDS_TEXT_VIEW", &defaults.view),
            view_info: self
                .runtime_resource_text("IDS_TEXT_DETERMINEPLAYERVIEWTOFOLL", &defaults.view_info),
            attack_page: self.runtime_resource_text("IDS_MENU_CPATTACK", &defaults.attack_page),
            attack_page_info: self
                .runtime_resource_text("IDS_MENU_CPATTACKINFO", &defaults.attack_page_info),
            select_team: self.runtime_resource_text("IDS_MSG_SELTEAM", &defaults.select_team),
            select_team_info: self.runtime_resource_text(
                "IDS_MSG_ALLOWSYOUTOJOINADIFFERENT",
                &defaults.select_team_info,
            ),
            join_team: self.runtime_resource_text("IDS_MSG_JOINTEAM", &defaults.join_team),
            new_player: self.runtime_resource_text("IDS_MENU_CPNEWPLAYER", &defaults.new_player),
            new_player_info: self
                .runtime_resource_text("IDS_MENU_CPNEWPLAYERINFO", &defaults.new_player_info),
            new_player_item: self
                .runtime_resource_text("IDS_MENU_NEWPLAYER", &defaults.new_player_item),
            no_player_files: self
                .runtime_resource_text("IDS_MENU_NOPLRFILES", &defaults.no_player_files),
            savegame: self.runtime_resource_text("IDS_MENU_CPSAVEGAME", &defaults.savegame),
            savegame_info: self
                .runtime_resource_text("IDS_MENU_CPSAVEGAMEINFO", &defaults.savegame_info),
            options: self.runtime_resource_text("IDS_MNU_OPTIONS", &defaults.options),
            options_info: self.runtime_resource_text("IDS_MNU_OPTIONSINFO", &defaults.options_info),
            disconnect: self.runtime_resource_text("IDS_MENU_DISCONNECT", &defaults.disconnect),
            disconnect_host_info: self.runtime_resource_text(
                "IDS_TEXT_KICKCERTAINCLIENTSFROMTHE",
                &defaults.disconnect_host_info,
            ),
            disconnect_client_info: self.runtime_resource_text(
                "IDS_TEXT_DISCONNECTTHEGAMEFROMTHES",
                &defaults.disconnect_client_info,
            ),
            disconnect_client_caption: self.runtime_resource_text(
                "IDS_MENU_DISCONNECTCLIENT",
                &defaults.disconnect_client_caption,
            ),
            disconnect_server_caption: self.runtime_resource_text(
                "IDS_MENU_DISCONNECTFROMSERVER",
                &defaults.disconnect_server_caption,
            ),
            surrender: self.runtime_resource_text("IDS_MENU_CPSURRENDER", &defaults.surrender),
            surrender_info: self
                .runtime_resource_text("IDS_MENU_CPSURRENDERINFO", &defaults.surrender_info),
            surrender_caption: self
                .runtime_resource_text("IDS_MENU_SURRENDER", &defaults.surrender_caption),
            abort: self.runtime_resource_text("IDS_MENU_ABORT", &defaults.abort),
            abort_info: self.runtime_resource_text("IDS_MENU_ABORT_DESC", &defaults.abort_info),
            attack: self.runtime_resource_text("IDS_MENU_ATTACK", &defaults.attack),
            no_attack: self.runtime_resource_text("IDS_MENU_NOATTACK", &defaults.no_attack),
            attack_hostile: self
                .runtime_resource_text("IDS_MENU_ATTACKHOSTILE", &defaults.attack_hostile),
            attack_friendly: self
                .runtime_resource_text("IDS_MENU_ATTACKFRIENDLY", &defaults.attack_friendly),
            attack_not: self.runtime_resource_text("IDS_MENU_ATTACKNOT", &defaults.attack_not),
            attack_info: self.runtime_resource_text("IDS_MENU_ATTACKINFO", &defaults.attack_info),
            free_view: self.runtime_resource_text("IDS_MSG_FREEVIEW", &defaults.free_view),
            free_view_info: self.runtime_resource_text(
                "IDS_MSG_FREELYSCROLLAROUNDTHEMAP",
                &defaults.free_view_info,
            ),
            follow_view: self
                .runtime_resource_text("IDS_TEXT_FOLLOWVIEWOFPLAYER", &defaults.follow_view),
            sound: self.runtime_resource_text("IDS_DLG_SOUND", &defaults.sound),
            music: self.runtime_resource_text("IDS_MNU_MUSIC", &defaults.music),
            mouse_control: self
                .runtime_resource_text("IDS_MNU_MOUSECONTROL", &defaults.mouse_control),
            display: self.runtime_resource_text("IDS_MENU_DISPLAY", &defaults.display),
            player_names: self.runtime_resource_text("IDS_MNU_PLAYERNAMES", &defaults.player_names),
            player_names_info: self
                .runtime_resource_text("IDS_MENU_PLAYERNAMES_DESC", &defaults.player_names_info),
            clonk_names: self.runtime_resource_text("IDS_MNU_CLONKNAMES", &defaults.clonk_names),
            clonk_names_info: self
                .runtime_resource_text("IDS_MENU_CLONKNAMES_DESC", &defaults.clonk_names_info),
            portraits: self.runtime_resource_text("IDS_MNU_PORTRAITS", &defaults.portraits),
            show_commands: self
                .runtime_resource_text("IDS_MENU_SHOWCOMMANDS", &defaults.show_commands),
            show_command_keys: self
                .runtime_resource_text("IDS_MENU_SHOWCOMMANDKEYS", &defaults.show_command_keys),
            upper_board: self.runtime_resource_text("IDS_MNU_UPPERBOARD", &defaults.upper_board),
            upper_board_off: self
                .runtime_resource_text("IDS_MNU_UPPERBOARD_OFF", &defaults.upper_board_off),
            upper_board_normal: self
                .runtime_resource_text("IDS_MNU_UPPERBOARD_NORMAL", &defaults.upper_board_normal),
            upper_board_small: self
                .runtime_resource_text("IDS_MNU_UPPERBOARD_SMALL", &defaults.upper_board_small),
            upper_board_mini: self
                .runtime_resource_text("IDS_MNU_UPPERBOARD_MINI", &defaults.upper_board_mini),
            fps: self.runtime_resource_text("IDS_MNU_FPS", &defaults.fps),
            clock: self.runtime_resource_text("IDS_MNU_CLOCK", &defaults.clock),
            white_chat: self.runtime_resource_text("IDS_MNU_WHITECHAT", &defaults.white_chat),
            white_chat_info: self
                .runtime_resource_text("IDS_DESC_WHITECHAT_INGAME", &defaults.white_chat_info),
            yes: self.runtime_resource_text("IDS_BTN_YES", &defaults.yes),
            no: self.runtime_resource_text("IDS_BTN_NO", &defaults.no),
        }
    }

    /// `C4Menu::Execute` refills *every* active menu when `Game.iTick35`
    /// wraps, not just the hostility page (C4Menu.cpp:990-1000), so an open
    /// team page follows live joins, switches and the generated-team row.
    pub(crate) fn refresh_team_menus(&mut self) {
        let players = self
            .ingame_menu
            .iter()
            .filter_map(|(player, menu)| {
                (menu.page() == ingame_menu::MenuPage::TeamSelection)
                    .then_some((player, menu.is_team_switch()))
            })
            .collect::<Vec<_>>();
        if players.is_empty() {
            return;
        }
        let entries = self.team_selection_entries();
        self.cache_team_selection_icons(&entries);
        let labels = self.ingame_menu_labels();
        for (player, switching) in players {
            if let Some(menu) = self.ingame_menu.get_mut(player) {
                menu.refill_team(&entries, switching, &labels);
            }
        }
    }

    pub(crate) fn refresh_hostility_menus(&mut self) {
        let players = self
            .ingame_menu
            .iter()
            .filter_map(|(player, menu)| {
                (menu.page() == ingame_menu::MenuPage::Hostility).then_some(player)
            })
            .collect::<Vec<_>>();
        let labels = self.ingame_menu_labels();
        for player in players {
            let Some(entries) = self.hostility_entries_for_player(player) else {
                self.close_ingame_menu_for_player(player);
                continue;
            };
            if let Some(menu) = self.ingame_menu.get_mut(player) {
                menu.refill_hostility(&entries, &labels);
            }
        }
    }

    /// `C4FullScreen::ShowAbortDlg`: construct the standalone, exclusive
    /// screen dialog and capture its one offline `HaltCount` lease.
    pub(crate) fn show_abort_dialog(&mut self, _player: i32) -> bool {
        if !matches!(self.mode, AppMode::Running)
            || self.game_over_dialog.is_some()
            || self.message_dialogs.iter().any(|dialog| {
                matches!(
                    dialog.continuation,
                    MessageDialogContinuation::AbortGame { .. }
                )
            })
        {
            return false;
        }
        let show_restart = self.engine.is_control_host() || self.engine.cinematic_film();
        let buttons = if show_restart {
            clonk_frontend::message_dialog::MessageDialogButtons::YES_RESTART_NO
        } else {
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
        };
        let size = if show_restart {
            clonk_frontend::message_dialog::MessageDialogSize::Fixed(400)
        } else {
            clonk_frontend::message_dialog::MessageDialogSize::Small
        };
        let halted_offline = self.network.is_none();
        let state = clonk_frontend::message_dialog::MessageDialogState::new(
            self.runtime_resource_text("IDS_HOLD_ABORT", "Abort round?"),
            self.runtime_resource_text("IDS_DLG_ABORT", "Abort"),
            buttons,
            clonk_frontend::message_dialog::MessageDialogIcon::Standard(33),
            size,
            false,
        )
        .with_centered_message();
        if let Err(error) = self.push_message_dialog(
            state,
            MessageDialogContinuation::AbortGame { halted_offline },
        ) {
            tracing::error!(%error, "failed to show abort confirmation");
            return false;
        }
        if halted_offline {
            self.offline_halt_count += 1;
        }
        true
    }

    pub(crate) fn close_ingame_menu(&mut self) {
        self.ingame_menu.clear();
        self.ingame_menu_close_pointer_capture = None;
        if matches!(self.menu_title_drag, Some(MenuTitleDrag::Ingame { .. })) {
            self.menu_title_drag = None;
        }
    }

    pub(crate) fn close_ingame_menu_for_player(&mut self, player: i32) {
        self.ingame_menu.remove(player);
        if self.ingame_menu_close_pointer_capture == Some(player) {
            self.ingame_menu_close_pointer_capture = None;
        }
        if matches!(
            self.menu_title_drag,
            Some(MenuTitleDrag::Ingame {
                player: dragged,
                ..
            }) if dragged == player
        ) {
            self.menu_title_drag = None;
        }
    }

    pub(crate) fn close_ingame_menu_by_user(&mut self) -> Result<(), EngineError> {
        self.close_ingame_menu_by_user_for_player(self.local_owner)
    }

    fn close_ingame_menu_by_user_for_player(&mut self, player: i32) -> Result<(), EngineError> {
        if self.ingame_menu.remove(player).is_some() {
            if self.ingame_menu_close_pointer_capture == Some(player) {
                self.ingame_menu_close_pointer_capture = None;
            }
            if matches!(
                self.menu_title_drag,
                Some(MenuTitleDrag::Ingame {
                    player: dragged,
                    ..
                }) if dragged == player
            ) {
                self.menu_title_drag = None;
            }
            if player != OWNER_NONE {
                // Player-owned C4MainMenu::OnClosed queues exactly one
                // synchronized clear; C4FullScreen's NO_OWNER menu and
                // silent teardown/reset paths do not
                // (src/C4MainMenu.cpp:319-329; src/C4Player.cpp:1392-1395).
                self.clear_local_control(player)?;
            }
        }
        Ok(())
    }

    pub(crate) fn open_object_menu(&mut self) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) || self.object_menu.is_some() {
            return Ok(false);
        }
        match ObjectMenuState::for_player(self.local_owner, &mut self.engine, &self.snapshot) {
            Some(menu) => {
                self.clear_local_control(self.local_owner)?;
                self.object_menu = Some(menu);
                self.close_ingame_menu_for_player(self.local_owner);
                if self.status_text.is_empty() {
                    self.status_text = "Inventory open".to_string();
                }
                Ok(true)
            }
            None => {
                if self.status_text.is_empty() {
                    self.status_text = "No crew inventory available".to_string();
                }
                Ok(false)
            }
        }
    }

    pub(crate) fn close_object_menu(&mut self) {
        if self.object_menu.is_some() {
            self.object_menu = None;
            if self.status_text == "Inventory open" {
                self.status_text.clear();
            }
        }
    }

    pub(crate) fn handle_menu_command(
        &mut self,
        owner: i32,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(false);
        }

        let menu_command = matches!(
            command,
            ControlCommand::MenuEnter
                | ControlCommand::MenuEnterAll
                | ControlCommand::MenuClose
                | ControlCommand::MenuDown
                | ControlCommand::MenuLeft
                | ControlCommand::MenuRight
                | ControlCommand::MenuSelect
                | ControlCommand::MenuShowText
                | ControlCommand::MenuUp
        );

        let owns_object_menu = owner == self.local_owner && self.object_menu.is_some();
        if menu_command && !owns_object_menu && !self.ingame_menu_belongs_to(owner) {
            return Ok(false);
        }

        if matches!(command, ControlCommand::PlayerMenu) {
            if matches!(
                kind,
                CommandKind::Press | CommandKind::Single | CommandKind::Double
            ) {
                if self.ingame_menu_belongs_to(owner) {
                    self.close_ingame_menu_by_user_for_player(owner)?;
                } else {
                    self.open_ingame_menu_for_player(owner)?;
                }
            }
            return Ok(true);
        }

        if owns_object_menu {
            let Some(menu) = self.object_menu.as_mut() else {
                return Ok(false);
            };
            if let Some(action) = menu.handle_command(command, kind) {
                self.execute_object_menu_action(action)?;
            }
            return Ok(true);
        }

        if !self.ingame_menu_belongs_to(owner) {
            return Ok(false);
        }
        let (outcome, preview_target) = {
            let Some(menu) = self.ingame_menu.get_mut(owner) else {
                return Ok(menu_command);
            };
            let previous_target = menu.selected_observer_target();
            let outcome = menu.handle_command(command, kind);
            let selected_target = menu.selected_observer_target();
            let preview_target = (selected_target != previous_target)
                .then_some(selected_target)
                .flatten();
            (outcome, preview_target)
        };
        if let Some(target) = preview_target {
            let _ = self.apply_observer_target(target);
        }
        if let Some(outcome) = outcome {
            self.execute_ingame_menu_outcome_for_player(owner, outcome)?;
        }
        Ok(true)
    }

    fn execute_object_menu_action(&mut self, action: ObjectMenuAction) -> Result<(), EngineError> {
        match action {
            ObjectMenuAction::Close => {
                self.close_object_menu();
            }
            ObjectMenuAction::Execute { command, selection } => match command {
                ObjectMenuCommand::Focus => {
                    let menu_selection = MenuCommandSelection {
                        primary_id: selection.primary_id,
                        instances: selection.instances.clone(),
                        definition_id: selection.definition_id.clone(),
                        label: selection.label.clone(),
                    };
                    let handled = self.engine.menu_command(
                        selection.crew_id,
                        MenuCommandKind::Focus,
                        menu_selection,
                    )?;
                    self.snapshot = self.engine.snapshot();
                    if handled {
                        self.refresh_object_menu();
                        self.refresh_focus();
                        self.object_menu = None;
                        self.status_text = format!("Executed {} via script", selection.label);
                        return Ok(());
                    }
                    self.object_menu = None;
                    self.focus_id = Some(selection.primary_id);
                    self.focus_snapshot = self.snapshot.object(selection.primary_id).cloned();
                    self.status_text =
                        format!("Selected {} (x{})", selection.label, selection.count());
                }
                ObjectMenuCommand::DropAll => {
                    let menu_selection = MenuCommandSelection {
                        primary_id: selection.primary_id,
                        instances: selection.instances.clone(),
                        definition_id: selection.definition_id.clone(),
                        label: selection.label.clone(),
                    };
                    let handled = self.engine.menu_command(
                        selection.crew_id,
                        MenuCommandKind::DropAll,
                        menu_selection,
                    )?;
                    self.snapshot = self.engine.snapshot();
                    if handled {
                        self.refresh_object_menu();
                        self.refresh_focus();
                        self.status_text = format!(
                            "Executed {} (x{}) via script",
                            selection.label,
                            selection.count()
                        );
                        return Ok(());
                    }
                    self.drop_inventory_selection(&selection)?;
                }
                ObjectMenuCommand::Take => {
                    self.take_from_container(&selection, 1)?;
                }
                ObjectMenuCommand::TakeAll => {
                    let amount = selection.instances.len().max(1);
                    self.take_from_container(&selection, amount)?;
                }
            },
            ObjectMenuAction::Context { selection } => {
                let handled = self
                    .engine
                    .execute_context_menu(selection.crew_id, &selection.function)?;
                self.snapshot = self.engine.snapshot();
                self.refresh_object_menu();
                self.refresh_focus();
                if handled {
                    self.object_menu = None;
                    self.status_text = format!("Executed {}", selection.label);
                } else if let Some(description) = selection.description.as_deref() {
                    self.status_text = description.to_string();
                } else if self.status_text.is_empty() {
                    self.status_text = format!("No scripted action for {}", selection.label);
                }
            }
            ObjectMenuAction::Build { selection, amount } => {
                if selection.owner == OWNER_NONE {
                    self.status_text = "Cannot construct without a player owner".to_string();
                    return Ok(());
                }
                let Some(crew_snapshot) = self.snapshot.object(selection.crew_id).cloned() else {
                    self.status_text = "Crew no longer available".to_string();
                    self.object_menu = None;
                    return Ok(());
                };
                let available = self
                    .engine
                    .player(selection.owner)
                    .and_then(|player| {
                        player
                            .home_base_material()
                            .get(&selection.definition_id)
                            .copied()
                    })
                    .unwrap_or(0);
                if available == 0 {
                    self.status_text = format!("No {} available", selection.label);
                    self.refresh_object_menu();
                    return Ok(());
                }
                let requested = amount.min(available);
                if requested == 0 {
                    self.status_text = format!("No {} available", selection.label);
                    self.refresh_object_menu();
                    return Ok(());
                }

                let definition_id = selection.definition_id.clone();
                let label = selection.label.clone();
                let owner = selection.owner;
                let crew_id = selection.crew_id;
                let mut delivered = 0u32;

                for _ in 0..requested {
                    self.engine.adjust_player_home_base_material(
                        owner,
                        definition_id.clone(),
                        -1,
                    )?;
                    match self.engine.spawn_object(
                        SpawnConfig::new(definition_id.clone())
                            .with_owner(owner)
                            .with_position(crew_snapshot.position)
                            .with_container(crew_id),
                    ) {
                        Ok(_) => delivered += 1,
                        Err(err) => {
                            self.engine.adjust_player_home_base_material(
                                owner,
                                definition_id.clone(),
                                1,
                            )?;
                            self.status_text = format!("Failed to deliver {}: {}", label, err);
                            break;
                        }
                    }
                }

                self.snapshot = self.engine.snapshot();
                self.refresh_object_menu();
                self.refresh_focus();

                if delivered > 0 {
                    let remaining = self
                        .engine
                        .player(owner)
                        .and_then(|player| player.home_base_material().get(&definition_id).copied())
                        .unwrap_or(0);
                    self.status_text = if remaining > 0 {
                        format!(
                            "Received {} (x{}), {} remaining",
                            label, delivered, remaining
                        )
                    } else {
                        format!("Received {} (x{})", label, delivered)
                    };
                } else if self.status_text.is_empty() {
                    self.status_text = format!("Unable to deliver {}", label);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn refresh_object_menu(&mut self) {
        if let Some(menu) = self.object_menu.as_mut() {
            if !menu.refresh(&mut self.engine, &self.snapshot) {
                self.object_menu = None;
            }
        }
    }

    /// Applies a menu outcome: `C4Menu::Enter` closes non-permanent menus
    /// before the command runs (C4Menu.cpp:512-518); `C4Menu::TryClose`
    /// executes the close command after closing (C4Menu.cpp:317-334).
    pub(crate) fn execute_ingame_menu_outcome(
        &mut self,
        outcome: MenuOutcome,
    ) -> Result<(), EngineError> {
        self.execute_ingame_menu_outcome_for_player(self.local_owner, outcome)
    }

    pub(crate) fn execute_ingame_menu_outcome_for_player(
        &mut self,
        player: i32,
        outcome: MenuOutcome,
    ) -> Result<(), EngineError> {
        match outcome {
            MenuOutcome::Action { action, close_menu } => {
                if close_menu {
                    self.close_ingame_menu_by_user_for_player(player)?;
                }
                self.apply_ingame_menu_action_for_player(player, action)?;
            }
            MenuOutcome::Closed { close_action } => {
                self.close_ingame_menu_by_user_for_player(player)?;
                if let Some(action) = close_action {
                    self.apply_ingame_menu_action_for_player(player, action)?;
                }
            }
        }
        Ok(())
    }

    /// `C4MainMenu::MenuCommand` (C4MainMenu.cpp:734-948).
    pub(crate) fn apply_ingame_menu_action(
        &mut self,
        action: MenuAction,
    ) -> Result<(), EngineError> {
        self.apply_ingame_menu_action_for_player(self.local_owner, action)
    }

    /// `C4MainMenu::MenuCommand("Host:Kick:<id>")` (C4MainMenu.cpp:805-819).
    /// Host ID zero and disabled networking are no-ops. League clients with
    /// a live player are voted out without closing the permanent page; all
    /// other nonzero IDs take the direct remove path and close it.
    fn kick_ingame_menu_client(&mut self, player: i32, client_id: i32) -> Result<(), EngineError> {
        if client_id == 0 || self.network.is_none() {
            return Ok(());
        }
        if self.network_is_league && self.runtime_client_has_players(client_id) {
            self.submit_own_league_vote(
                LeagueVoteSubject {
                    vote_type: clonk_engine::VOTE_TYPE_KICK,
                    data: client_id,
                },
                true,
            );
            return Ok(());
        }
        if self.control_clients.contains(client_id) {
            let reason = self.runtime_resource_string("IDS_MSG_KICKBYMENU");
            let remove = clonk_engine::ClientRemoveControlData {
                client_id,
                reason: clonk_engine::LegacyCString::from_bytes(clonk_script::c4_string_bytes(
                    &reason,
                ))
                .unwrap_or_default(),
                by_client: 0,
            };
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_client_remove(remove))
            {
                tracing::error!(%client_id, %error, "failed to submit host-menu kick");
            }
        }
        self.close_ingame_menu_by_user_for_player(player)
    }

    pub(crate) fn apply_ingame_menu_action_for_player(
        &mut self,
        player: i32,
        action: MenuAction,
    ) -> Result<(), EngineError> {
        match action {
            MenuAction::ActivateMain => {
                self.ingame_menu.replace(
                    player,
                    IngameMenuState::main_menu(
                        &self.main_menu_conditions_for(player),
                        &self.ingame_menu_labels(),
                    ),
                );
            }
            MenuAction::ActivateGoals => {
                // Goal callbacks are synchronized by
                // CID_ActivateGameGoalMenu. Only packet execution may open
                // the local menu and expose its fulfilled markers.
                if self.network.is_some() {
                    let tick = self.local_control_submission_tick();
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_activate_game_goal_menu(tick, player))
                    {
                        tracing::warn!(player, %error, "failed to queue goal menu activation");
                    }
                } else {
                    let by_client = self
                        .engine
                        .player(player)
                        .map(|player| player.at_client().get())
                        .unwrap_or(-1);
                    let control =
                        clonk_engine::ActivateGameGoalMenuControlData { player, by_client };
                    self.record_control_batch(std::slice::from_ref(
                        &clonk_engine::ControlPacket::ActivateGameGoalMenu(control),
                    ));
                    self.engine
                        .execute_activate_game_goal_menu_control(&control)?;
                    self.apply_game_goal_menu_requests()?;
                }
            }
            MenuAction::ActivateRules => {
                let rules = self.goal_rule_entries(C4D_RULE);
                self.cache_definition_icons(&rules)?;
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::rules_menu(
                        &rules,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::ActivateNewPlayer => {
                let conditions = self.main_menu_conditions_for(player);
                if conditions.is_league || conditions.player_count >= conditions.max_players {
                    return Ok(());
                }
                let players = self.available_runtime_player_files();
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::new_player_menu(
                        &players,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::ActivateOptions => {
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::options_menu(
                        &self.option_flags(player),
                        0,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::ActivateDisplay => {
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::display_menu(
                        &self.display_flags,
                        0,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::ActivateSavegame => {
                // Game.CanQuickSave: network clients may not save
                // (C4Game.cpp:2205-2223) — the menu simply stays closed.
                if self.can_quick_save() {
                    let slots = self.savegame_slots();
                    self.ingame_menu.replace(
                        player,
                        Some(IngameMenuState::savegame_menu(
                            &slots,
                            &self.ingame_menu_labels(),
                        )),
                    );
                }
            }
            MenuAction::ActivateSurrender => {
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::surrender_menu(&self.ingame_menu_labels())),
                );
            }
            MenuAction::ActivateClientDisconnect => {
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::client_disconnect_menu(
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::ActivateHostility => {
                self.activate_hostility_menu_for_player(player);
            }
            MenuAction::ToggleHostility(opponent) => {
                if !self.engine.team_configuration().allow_hostility_change
                    || !self.hostility_opponent_is_user(opponent)
                {
                    return Ok(());
                }
                if self.network.is_some() {
                    let tick = self.local_control_submission_tick();
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_toggle_hostility(tick, player, opponent))
                    {
                        tracing::warn!(player, opponent, %error, "failed to queue hostility toggle");
                    }
                } else {
                    let by_client = self
                        .engine
                        .player(player)
                        .map(|player| player.at_client().get())
                        .unwrap_or(-1);
                    let control = clonk_engine::ToggleHostilityControlData {
                        opponent,
                        player,
                        by_client,
                    };
                    self.record_control_batch(std::slice::from_ref(
                        &clonk_engine::ControlPacket::ToggleHostility(control),
                    ));
                    let _ = self.engine.execute_toggle_hostility_control(&control)?;
                }
            }
            MenuAction::ActivateHostDisconnect => {
                let clients = self
                    .control_clients
                    .snapshot()
                    .into_iter()
                    .map(|client| HostDisconnectClientEntry {
                        client_id: client.client_id,
                        caption: format!(
                            "{} ({})",
                            legacy_presentation_text(client.name.as_bytes()),
                            legacy_presentation_text(client.nick.as_bytes())
                        ),
                        activated: client.activated,
                    })
                    .collect::<Vec<_>>();
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::host_disconnect_menu(
                        &clients,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::KickClient(client_id) => {
                self.kick_ingame_menu_client(player, client_id)?;
            }
            MenuAction::ActivateObserver => {
                let Some(current_player) = self.observer_viewport_player() else {
                    return Ok(());
                };
                let players = self.observer_player_entries();
                let menu = IngameMenuState::observer_menu(
                    &players,
                    if current_player == OWNER_NONE {
                        ObserverTarget::Free
                    } else {
                        ObserverTarget::Player(current_player)
                    },
                    &self.ingame_menu_labels(),
                );
                let selected = menu
                    .selected_observer_target()
                    .unwrap_or(ObserverTarget::Free);
                self.ingame_menu.replace(OWNER_NONE, Some(menu));
                let _ = self.apply_observer_target(selected);
            }
            MenuAction::ActivateTeamSelection => {
                if let Some(status) = self.engine.player(player).map(clonk_engine::Player::status) {
                    let entries = self.team_selection_entries();
                    self.cache_team_selection_icons(&entries);
                    let menu = if status == clonk_engine::PlayerStatus::TeamSelection {
                        IngameMenuState::team_selection_menu_from_main(
                            &entries,
                            &self.ingame_menu_labels(),
                        )
                    } else {
                        IngameMenuState::team_switch_menu(&entries, &self.ingame_menu_labels())
                    };
                    self.ingame_menu.replace(player, Some(menu));
                }
            }
            MenuAction::Abort => {
                self.show_abort_dialog(player);
            }
            MenuAction::Surrender => {
                // CID_SurrenderPlayer -> player surrenders with evaluation
                // (C4MainMenu.cpp:791-795); the engine's game-over check
                // treats surrendered players as inactive. Network games route
                // this through the next complete control tick.
                if self.network.is_some() {
                    let tick = self.local_control_submission_tick();
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_surrender_player(tick, player))
                    {
                        tracing::warn!(player, %error, "failed to queue player surrender");
                    }
                } else {
                    let by_client = self
                        .engine
                        .player(player)
                        .map(|player| player.at_client().get())
                        .unwrap_or(-1);
                    self.record_control_batch(std::slice::from_ref(
                        &clonk_engine::ControlPacket::SurrenderPlayer(
                            clonk_engine::SurrenderPlayerControlData { player, by_client },
                        ),
                    ));
                    if let Err(err) = self.engine.set_player_surrendered(player, true) {
                        tracing::error!(error = ?err, "surrender failed");
                    }
                }
            }
            MenuAction::Part => {
                // Non-league Part clears C4Network2, which changes the live
                // round to local control instead of aborting it
                // (C4MainMenu.cpp:820-831; C4GameControl.cpp:93-127).
                if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
                    if let Some(local_client_id) = self
                        .network
                        .as_ref()
                        .and_then(|network| i32::try_from(network.local_client_id()).ok())
                    {
                        let league_self_kick = self.network_is_league
                            && self
                                .engine
                                .players()
                                .any(|player| player.at_client().get() == local_client_id);
                        if league_self_kick {
                            self.submit_own_league_vote(
                                LeagueVoteSubject {
                                    vote_type: clonk_engine::VOTE_TYPE_KICK,
                                    data: local_client_id,
                                },
                                true,
                            );
                        } else {
                            let result_message =
                                self.runtime_resource_bytes("IDS_ERR_GAMELEFTVIAPLAYERMENU");
                            self.engine.evaluate_network_round_results(
                                clonk_engine::RoundResultsNetworkResult::NetworkError,
                                Some(result_message),
                            );
                            self.snapshot.round_results = self.engine.snapshot().round_results;
                            if let Some(Err(error)) =
                                self.network.as_ref().map(NetworkManager::graceful_part)
                            {
                                tracing::warn!(%error, "failed to notify host before parting");
                            }
                            self.change_network_control_to_local(local_client_id);
                        }
                    }
                }
            }
            MenuAction::SaveSlot(slot) => {
                // "Save:Game:<file>:<title>" -> Game.QuickSave + reopen the
                // savegame menu (C4MainMenu.cpp:797-804).
                self.save_to_slot(slot);
                if self.ingame_menu.contains(player) {
                    let slots = self.savegame_slots();
                    self.ingame_menu.replace(
                        player,
                        Some(IngameMenuState::savegame_menu(
                            &slots,
                            &self.ingame_menu_labels(),
                        )),
                    );
                }
            }
            MenuAction::ToggleSound => {
                // Application.SoundSystem->ToggleOnOff() + reopen with the
                // previous selection (C4MainMenu.cpp:842-852).
                let selection = self.ingame_menu_selection(player);
                self.toggle_sound_option()?;
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::options_menu(
                        &self.option_flags(player),
                        selection,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::ToggleMusic => {
                let selection = self.ingame_menu_selection(player);
                self.toggle_music_option()?;
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::options_menu(
                        &self.option_flags(player),
                        selection,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::ToggleMouseControl => {
                let selection = self.ingame_menu_selection(player);
                if self.mouse_control_allowed {
                    if let Some(control) = self.local_controls.toggle_mouse(player) {
                        self.engine
                            .set_player_mouse_control(player, control.mouse)?;
                        self.mouse_control = self.local_controls.mouse_owner().is_some();
                        // C4Player::ToggleMouseControl clears and defaults
                        // C4MouseControl whenever ownership is reinitialized.
                        self.reset_ingame_mouse_control();
                    }
                }
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::options_menu(
                        &self.option_flags(player),
                        selection,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::Display(toggle) => {
                // Toggle + reopen with the previous selection
                // (C4MainMenu.cpp:855-884).
                let selection = self.ingame_menu_selection(player);
                self.display_flags.toggle(toggle);
                self.defer_display_toggle(toggle);
                if toggle == DisplayToggle::UpperBoard {
                    let game_time_seconds = self.game_time_seconds();
                    self.graphics.set_upper_board_mode(
                        frontend_upper_board_mode(self.display_flags.upper_board),
                        game_time_seconds,
                    );
                }
                self.ingame_menu.replace(
                    player,
                    Some(IngameMenuState::display_menu(
                        &self.display_flags,
                        selection,
                        &self.ingame_menu_labels(),
                    )),
                );
            }
            MenuAction::GoalInfo(id) => {
                self.submit_game_goal_rule_activation(
                    player,
                    ClassicIngameMenuChild::GoalInfo(id),
                )?;
            }
            MenuAction::RuleInfo(id) => {
                self.submit_game_goal_rule_activation(
                    player,
                    ClassicIngameMenuChild::RuleInfo(id),
                )?;
            }
            MenuAction::JoinPlayer(file) => {
                if self.network_is_league || self.engine.replay() {
                    return Ok(());
                }
                if self.network.is_some() {
                    match self.submit_runtime_network_player(&file) {
                        Ok(()) => {
                            self.status_text = format!("Joining player {file}");
                        }
                        Err(error) => {
                            tracing::warn!(%file, %error, "runtime network player join failed");
                            self.status_text = format!("Unable to join player: {error}");
                        }
                    }
                } else if let Err(error) = self.submit_runtime_offline_player(&file) {
                    tracing::warn!(%file, %error, "runtime offline player join failed");
                    return Err(classic_ingame_menu_child_error(
                        ClassicIngameMenuChild::JoinPlayer {
                            file,
                            detail: error,
                        },
                    ));
                }
            }
            MenuAction::SelectTeam(team) => {
                self.engine.mark_team_selection_pending(player)?;
                if self.network.is_some() {
                    let tick = self.local_control_submission_tick();
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_init_scenario_player(tick, player, team))
                    {
                        tracing::warn!(player, team, %error, "failed to queue team selection");
                    }
                } else {
                    self.record_control_batch(std::slice::from_ref(
                        &clonk_engine::ControlPacket::InitScenarioPlayer(
                            clonk_engine::InitScenarioPlayerControlData {
                                team,
                                player,
                                by_client: 0,
                            },
                        ),
                    ));
                    self.execute_init_scenario_player_control(player, team)?;
                }
            }
            MenuAction::SwitchTeam(team) => {
                // `TeamSwitch:<id>` closes its page before this dispatch and
                // rechecks the live scenario flag. The queued control itself
                // deliberately does not recheck it (C4MainMenu.cpp:909-918).
                if !self.engine.team_configuration().allow_team_switch {
                    return Ok(());
                }
                if self.network.is_some() {
                    let tick = self.local_control_submission_tick();
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_set_player_team(tick, player, team))
                    {
                        tracing::warn!(player, team, %error, "failed to queue team switch");
                    }
                } else {
                    let by_client = self
                        .engine
                        .player(player)
                        .map(|player| player.at_client().get())
                        .unwrap_or(-1);
                    let control = clonk_engine::SetPlayerTeamControlData {
                        team,
                        player,
                        by_client,
                    };
                    self.record_control_batch(std::slice::from_ref(
                        &clonk_engine::ControlPacket::SetPlayerTeam(control),
                    ));
                    let _ = self.engine.execute_set_player_team_control(&control)?;
                }
            }
            MenuAction::Observe(target) => {
                let _ = self.apply_observer_target(target);
            }
            MenuAction::NoOp => {}
        }
        Ok(())
    }

    fn ingame_menu_selection(&self, player: i32) -> usize {
        self.ingame_menu
            .get(player)
            .map(IngameMenuState::selection)
            .unwrap_or(0)
    }

    pub(crate) fn apply_game_goal_menu_requests(&mut self) -> Result<(), EngineError> {
        for request in self.engine.take_game_goal_menu_requests() {
            if !request.open_menu {
                continue;
            }
            let entries = request
                .goals
                .into_iter()
                .map(|definition_id| GoalRuleEntry {
                    name: self
                        .engine
                        .definition_name(&definition_id)
                        .unwrap_or(&definition_id)
                        .to_string(),
                    description: self
                        .engine
                        .definition_description(&definition_id)
                        .map(str::to_string),
                    fulfilled: request.fulfilled_goals.contains(&definition_id),
                    definition_id,
                })
                .collect::<Vec<_>>();
            self.cache_definition_icons(&entries)?;
            self.ingame_menu.replace(
                request.player,
                Some(IngameMenuState::goals_menu(
                    &entries,
                    &self.ingame_menu_labels(),
                )),
            );
        }
        Ok(())
    }

    pub(crate) fn update_blocking_resource_wait_dialog(
        &mut self,
        scope: BlockingResourceScope,
        resource_id: i32,
        present_percent: u8,
    ) {
        if let Some(dialog) = self.message_dialogs.iter_mut().rfind(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::BlockingResourceWait {
                    scope: candidate_scope,
                    resource_id: candidate_id,
                } if candidate_scope == scope && candidate_id == resource_id
            )
        }) {
            dialog.state.set_progress(present_percent);
        }
    }

    pub(crate) fn dismiss_blocking_resource_wait_dialog(
        &mut self,
        scope: BlockingResourceScope,
        resource_id: i32,
    ) {
        let Some(index) = self.message_dialogs.iter().rposition(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::BlockingResourceWait {
                    scope: candidate_scope,
                    resource_id: candidate_id,
                } if candidate_scope == scope && candidate_id == resource_id
            )
        }) else {
            return;
        };
        self.remove_message_dialog_at(index);
    }

    pub(crate) fn handle_menu_cancel_action(
        &mut self,
        state: ElementState,
    ) -> Result<(), EngineError> {
        if self.game_over_dialog.is_some() {
            if state == ElementState::Pressed {
                self.handle_game_over_action(GameOverAction::End)?;
            }
            return Ok(());
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::PlayerSelection
            && self.startup_crew_rename.is_some()
        {
            if state == ElementState::Pressed {
                self.abort_startup_crew_rename();
            }
            return Ok(());
        }
        if self.handle_startup_dialog_key(KeyCode::Escape, state)? {
            return Ok(());
        }
        if self.startup_view == StartupView::ScenarioBrowser
            && self.menu_state.rename_edit.is_some()
        {
            if state == ElementState::Pressed {
                self.abort_scenario_rename();
            }
            return Ok(());
        }
        match self.startup_view {
            StartupView::ScenarioBrowser => match state {
                ElementState::Pressed => self.close_scenario_browser(),
                ElementState::Released => {}
            },
            StartupView::NetworkGame | StartupView::PlayerSelection => {}
            // Escape does not quit here, diverging from C++ deliberately.
            // `C4StartupMainDlg::OnClosed` exits whenever the dialog closes
            // without OK - "if dlg got aborted (by user), quit startup"
            // (src/C4StartupMainDlg.cpp:202-206). But Escape is also how a
            // player leaves a running game, so it arrives in bursts while a
            // scenario unloads, and the presses that outlast the transition
            // would kill the process with no confirmation
            // (clonk-org/clonk-rs#943). Quitting stays with the explicit Quit
            // item, which is the same affordance C++ offers here.
            StartupView::MainMenu => {}
            StartupView::NetworkLobby => {
                if state == ElementState::Pressed {
                    self.show_main_menu();
                }
            }
            StartupView::Options | StartupView::About => {}
        }
        Ok(())
    }

    /// `Element::DoDragging`: retain the original title-local pointer and
    /// apply its screen-space delta one-for-one, even outside the dialog.
    pub(crate) fn update_menu_title_drag(&mut self, point: GuiPoint) -> bool {
        let Some(drag) = self.menu_title_drag else {
            return false;
        };
        let moved = |start_pointer: GuiPoint, start_location: (i32, i32)| {
            (
                start_location
                    .0
                    .saturating_add((point.x - start_pointer.x).round() as i32),
                start_location
                    .1
                    .saturating_add((point.y - start_pointer.y).round() as i32),
            )
        };
        match drag {
            MenuTitleDrag::Ingame {
                player,
                start_pointer,
                start_location,
            } => {
                let Some(menu) = self.ingame_menu.get_mut(player) else {
                    self.menu_title_drag = None;
                    return false;
                };
                menu.set_location(moved(start_pointer, start_location));
            }
            MenuTitleDrag::Script {
                owner,
                target,
                start_pointer,
                start_location,
            } => {
                let valid = self
                    .engine
                    .cursor_object_menu(owner)
                    .is_some_and(|(current, menu)| {
                        current == target
                            && self
                                .script_menu_presentations
                                .get(&owner)
                                .is_some_and(|state| {
                                    same_script_menu_presentation(state, target, menu)
                                })
                    });
                if !valid {
                    self.menu_title_drag = None;
                    return false;
                }
                if let Some(state) = self.script_menu_presentations.get_mut(&owner) {
                    state.location = Some(moved(start_pointer, start_location));
                    state.location_needs_initialization = false;
                }
            }
        }
        true
    }

    pub(crate) fn finish_menu_title_drag(&mut self, point: Option<GuiPoint>) -> bool {
        if self.menu_title_drag.is_none() {
            return false;
        }
        if let Some(point) = point {
            self.update_menu_title_drag(point);
        }
        self.menu_title_drag = None;
        self.cancel_ingame_mouse_gestures();
        true
    }

    pub(crate) fn construction_menu_drag_captured(&self) -> bool {
        self.construction_menu_drag.is_some()
    }

    pub(crate) fn arm_construction_menu_drag(
        &mut self,
        owner: i32,
        item_index: usize,
        down: GuiPoint,
    ) {
        let Some(drag) = self.engine.object_menu_construction_drag(owner, item_index) else {
            self.construction_menu_drag = None;
            return;
        };
        self.construction_menu_drag = Some(ConstructionMenuDrag::Candidate {
            owner,
            menu_object_id: drag.menu_object_id,
            item_index,
            definition_id: drag.definition_id,
            definition_c4id: drag.definition_c4id,
            down,
        });
    }

    /// Advance C4Menu's GUI-space drag element before normal GUI hit-testing.
    /// `true` means the gesture is now C4MouseControl-owned and this move must
    /// not be delivered to a menu or another modal overlay.
    pub(crate) fn update_construction_menu_drag(
        &mut self,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        let Some(drag) = self.construction_menu_drag.clone() else {
            return Ok(false);
        };
        match drag {
            ConstructionMenuDrag::Candidate {
                owner,
                menu_object_id,
                item_index,
                definition_id,
                definition_c4id,
                down,
            } => {
                let distance = (point.x - down.x).abs().max((point.y - down.y).abs());
                if distance < MENU_DRAG_THRESHOLD {
                    return Ok(false);
                }
                let still_same_item = self
                    .engine
                    .object_menu_construction_drag(owner, item_index)
                    .is_some_and(|current| {
                        current.menu_object_id == menu_object_id
                            && current.definition_id == definition_id
                            && current.definition_c4id == definition_c4id
                    });
                if !still_same_item {
                    self.construction_menu_drag = None;
                    return Ok(false);
                }

                // C4GUI::CMouse::ReleaseButtons drops its menu capture before
                // C4MouseControl starts receiving the construction drag.
                self.clear_ingame_world_mouse_gestures();
                self.construction_menu_drag = Some(ConstructionMenuDrag::Active {
                    owner,
                    definition_id,
                    definition_c4id,
                    viewport_index: None,
                    pointer: None,
                    site_valid: false,
                });
                self.update_ingame_pointer(point)?;
                self.refresh_construction_menu_drag();
                Ok(true)
            }
            ConstructionMenuDrag::Active { .. } => {
                self.update_ingame_pointer(point)?;
                self.refresh_construction_menu_drag();
                Ok(true)
            }
        }
    }

    pub(crate) fn refresh_construction_menu_drag(&mut self) {
        let Some(ConstructionMenuDrag::Active {
            owner,
            definition_id,
            ..
        }) = self.construction_menu_drag.as_ref()
        else {
            return;
        };
        let owner = *owner;
        let definition_id = definition_id.clone();
        // MouseControl::Execute repeats Move(VpX, VpY), so a stationary OS
        // cursor follows the viewport's current ViewX/ViewY instead of
        // retaining the world coordinate from the last platform event.
        let retained_viewport = self
            .ingame_viewport_mouse
            .filter(|retained| !retained.observer && retained.owner == owner);
        let pointer = match retained_viewport {
            Some(retained) => self
                .graphics
                .active_viewport_projections()
                .into_iter()
                .find(|viewport| {
                    viewport.index == retained.viewport_index && viewport.owner == owner
                })
                .and_then(|viewport| {
                    let screen = GuiPoint::new(
                        viewport.rect.x.saturating_add(retained.position.x) as f32,
                        viewport.rect.y.saturating_add(retained.position.y) as f32,
                    );
                    self.graphics
                        .viewport_output_point_for_index(viewport.index, screen)
                }),
            None => self.ingame_pointer.filter(|pointer| pointer.owner == owner),
        };
        self.ingame_pointer = pointer;
        let site_valid = pointer.is_some_and(|pointer| {
            let site = ingame_pointer_world_pixel(pointer);
            self.ingame_viewport_region(owner, pointer.screen).is_none()
                && self.engine.construction_site_visible(owner, site)
                && self.engine.construction_site_valid(&definition_id, site)
        });
        if let Some(ConstructionMenuDrag::Active {
            viewport_index: stored_viewport_index,
            pointer: stored_pointer,
            site_valid: stored_valid,
            ..
        }) = self.construction_menu_drag.as_mut()
        {
            *stored_viewport_index =
                pointer.and(retained_viewport.map(|mouse| mouse.viewport_index));
            *stored_pointer = pointer;
            *stored_valid = site_valid;
        }
    }

    pub(crate) fn finish_construction_menu_drag(&mut self) -> Result<bool, EngineError> {
        let Some(drag) = self.construction_menu_drag.take() else {
            return Ok(false);
        };
        let ConstructionMenuDrag::Active {
            owner,
            definition_c4id,
            pointer,
            site_valid,
            ..
        } = drag
        else {
            return Ok(false);
        };
        if self.mouse_control
            && self.local_controls.mouse_owner() == Some(owner)
            && site_valid
            && self.engine.player(owner).is_some()
            && !self.engine.is_owner_eliminated(owner)
        {
            if let Some(pointer) = pointer.filter(|pointer| pointer.owner == owner) {
                let site = ingame_pointer_world_pixel(pointer);
                self.show_startup_hint = false;
                self.submit_or_execute_player_command(PlayerCommandControlData {
                    player: owner,
                    command: CommandId::Construct as i32,
                    x: site.x,
                    y: site.y,
                    target: 0,
                    target2: 0,
                    data: definition_c4id,
                    add_mode: 1 | if self.keyboard_modifiers.shift_key() {
                        4
                    } else {
                        0
                    },
                    by_client: -1,
                })?;
                self.snapshot = self.engine.snapshot();
                self.refresh_object_menu();
                self.refresh_focus();
            }
        }
        Ok(true)
    }

    pub(crate) fn running_external_menu_is_shown(&self) -> bool {
        self.ingame_menu.is_some() || self.engine.has_active_object_menu()
    }

    pub(crate) fn ingame_menu_area(&self, player: i32) -> Option<Rect> {
        if player == OWNER_NONE {
            let surface = self.graphics.surface();
            Some(Rect::new(0, 0, surface.width(), surface.height()))
        } else {
            self.graphics.viewport_rect(player)
        }
    }

    pub(crate) fn arm_ingame_menu_title_drag(&mut self, player: i32, point: GuiPoint) -> bool {
        let Some(area) = self.ingame_menu_area(player) else {
            return false;
        };
        let fallback = self.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            self.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let gfx = IngameMenuGraphics {
            show_commands: self.display_flags.show_commands,
            show_close_button: true,
            ..IngameMenuGraphics::default()
        };
        let Some(bounds) = self
            .ingame_menu
            .get(player)
            .map(|menu| menu.bounds(area, &font, &gfx))
        else {
            return false;
        };
        let start_location = (bounds.x, bounds.y);
        if let Some(menu) = self.ingame_menu.get_mut(player) {
            menu.set_location(start_location);
        }
        self.menu_title_drag = Some(MenuTitleDrag::Ingame {
            player,
            start_pointer: point,
            start_location,
        });
        true
    }

    pub(crate) fn handle_runtime_default_dialog_primary_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(false);
        }
        for dialog_kind in self
            .runtime_default_dialog_order_snapshot()
            .into_iter()
            .rev()
        {
            let handled = match dialog_kind {
                RuntimeDefaultDialog::ExternalIrc => {
                    self.handle_runtime_external_irc_pointer_button(button_state)?
                }
                RuntimeDefaultDialog::GameOver => {
                    let hit = self
                        .running_pointer_position
                        .is_some_and(|point| self.game_over_pointer_route_hit(point))
                        || self
                            .game_over_dialog
                            .as_ref()
                            .is_some_and(GameOverState::has_pointer_capture);
                    if !hit {
                        false
                    } else {
                        let (width, height) = {
                            let surface = self.graphics.surface();
                            (surface.width(), surface.height())
                        };
                        let action =
                            self.game_over_dialog
                                .as_mut()
                                .and_then(|dialog| match button_state {
                                    ElementState::Pressed => {
                                        dialog.handle_pointer_down(width, height);
                                        None
                                    }
                                    ElementState::Released => {
                                        dialog.handle_pointer_up(width, height)
                                    }
                                });
                        let sounds = self
                            .game_over_dialog
                            .as_mut()
                            .map(GameOverState::take_sound_events)
                            .unwrap_or_default();
                        self.play_game_over_sound_events(sounds);
                        if let Some(action) = action {
                            self.handle_game_over_action(action)?;
                        }
                        true
                    }
                }
                RuntimeDefaultDialog::ClientList => {
                    self.handle_runtime_client_list_pointer_button(button_state)?
                }
                RuntimeDefaultDialog::NetworkChart => {
                    self.handle_network_chart_pointer_button(button_state)
                }
                RuntimeDefaultDialog::Scoreboard => {
                    self.handle_scoreboard_pointer_button(button_state)?
                }
            };
            if handled {
                if button_state == ElementState::Pressed
                    && self.runtime_default_dialog_visible(dialog_kind)
                {
                    self.activate_runtime_default_dialog(dialog_kind);
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Input can arrive between `CreateMenu` and the next frame. Seed the
    /// presentation state here as well as during render so the first wheel or
    /// title gesture is retained instead of merely being consumed.
    pub(crate) fn ensure_script_menu_presentation_for_owner(&mut self, owner: i32) -> bool {
        let Some((target, menu)) = self
            .engine
            .cursor_object_menu(owner)
            .map(|(target, menu)| (target, menu.clone()))
        else {
            return false;
        };
        let initial_location = self.script_menu_free_location(owner, &menu);
        let key = ScriptMenuPresentationKey {
            target,
            runtime_id: menu.runtime_id,
            symbol_id: menu.symbol_id.clone(),
            caption: menu.caption.clone(),
            selection: menu.selection,
            location: menu.location,
        };
        let mut next = match self.script_menu_presentations.remove(&owner) {
            Some(state) if state.key == key => state,
            Some(mut state) if same_script_menu_presentation(&state, target, &menu) => {
                state.key = key;
                state.time_on_selection = 0;
                if state.location.is_none() {
                    state.location = initial_location;
                    state.location_needs_initialization = initial_location.is_some();
                    state.free_aligned = initial_location.is_some();
                }
                state.selection_needs_adjustment |= state.scroll_selection != menu.selection;
                state
            }
            _ => ScriptMenuPresentationState {
                key,
                time_on_selection: 0,
                location: initial_location,
                location_needs_initialization: initial_location.is_some(),
                free_aligned: initial_location.is_some(),
                scroll_y: 0,
                scroll_selection: menu.selection,
                selection_needs_adjustment: true,
                // A row count set before the first draw is discarded by that
                // draw's InitLocation (C4Menu.cpp:713-721,796-797).
                explicit_lines: None,
                applied_menu_lines: menu.lines,
                applied_location_reset_generation: menu.location_reset_generation,
                location_reset_pending: false,
            },
        };
        // C4Menu::SetSize assigns Lines without clearing LocationSet, so a
        // SetMenuSize on an already-displayed menu keeps its explicit row
        // count (C4Menu.cpp:635-640).
        if !next.location_reset_pending && next.applied_menu_lines != menu.lines {
            next.explicit_lines = (menu.lines > 0).then_some(menu.lines);
            next.applied_menu_lines = menu.lines;
        }
        sync_script_menu_presentation_location_reset(&mut next, &menu);
        self.script_menu_presentations.insert(owner, next);
        true
    }

    pub(crate) fn script_menu_layout_for_owner(
        &self,
        owner: i32,
        adjust_selection: bool,
    ) -> Result<Option<(ObjectId, EngineScriptMenuLayout)>, EngineError> {
        if !self.mouse_control {
            return Ok(None);
        }
        let Some((target, menu)) = self.engine.cursor_object_menu(owner) else {
            return Ok(None);
        };
        if !matches!(menu.style, 0..=2) {
            return Ok(None);
        }
        self.assets
            .require_classic_ingame_menu_resources()
            .map_err(|error| classic_parity_engine_error(report_classic_parity_boundary(error)))?;
        let fallback = self.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            self.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let area = self.graphics.viewport_rect(owner).unwrap_or_else(|| {
            let surface = self.graphics.surface();
            Rect::new(0, 0, surface.width(), surface.height())
        });
        let font_images =
            resolve_script_menu_font_images(&self.engine, menu, self.script_text_spec_resources())
                .map_err(|error| {
                    classic_parity_engine_error(report_classic_parity_boundary(
                        ClassicParityBoundary::ScriptMenuPointerResources {
                            detail: error.to_string(),
                        },
                    ))
                })?;
        let presentation = self
            .script_menu_presentations
            .get(&owner)
            .filter(|state| same_script_menu_presentation(state, target, menu));
        let location = presentation
            .and_then(|state| state.location)
            .or_else(|| self.script_menu_free_location(owner, menu));
        let scroll_y = presentation.map_or(0, |state| state.scroll_y);
        let selection_changed =
            presentation.is_none_or(|state| state.scroll_selection != menu.selection);
        let adjust_selection = adjust_selection || selection_changed;
        let use_free_anchor = presentation.map_or(location.is_some(), |state| {
            state.location_needs_initialization
        });
        let explicit_lines = presentation.and_then(|state| state.explicit_lines);
        let layout = if use_free_anchor {
            engine_script_menu_layout_with_free_anchor(
                area,
                &font,
                menu,
                self.display_flags.show_commands,
                &font_images,
                location.expect("free anchor has a location"),
                scroll_y,
                adjust_selection,
                explicit_lines,
            )
        } else {
            engine_script_menu_layout_with_presentation(
                area,
                &font,
                menu,
                self.display_flags.show_commands,
                &font_images,
                location,
                scroll_y,
                adjust_selection,
                explicit_lines,
            )
        };
        Ok(Some((target, layout)))
    }

    pub(crate) fn script_menu_geometry_for_owner(
        &self,
        owner: i32,
    ) -> Result<Option<(ObjectId, EngineScriptMenuPresentationGeometry)>, EngineError> {
        if !self.mouse_control {
            return Ok(None);
        }
        let Some((target, menu)) = self.engine.cursor_object_menu(owner) else {
            return Ok(None);
        };
        self.assets
            .require_classic_ingame_menu_resources()
            .map_err(|error| classic_parity_engine_error(report_classic_parity_boundary(error)))?;
        let fallback = self.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            self.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let area = self.graphics.viewport_rect(owner).unwrap_or_else(|| {
            let surface = self.graphics.surface();
            Rect::new(0, 0, surface.width(), surface.height())
        });
        let font_images =
            resolve_script_menu_font_images(&self.engine, menu, self.script_text_spec_resources())
                .map_err(|error| {
                    classic_parity_engine_error(report_classic_parity_boundary(
                        ClassicParityBoundary::ScriptMenuPointerResources {
                            detail: error.to_string(),
                        },
                    ))
                })?;
        let item_icons = if menu.style == 3 {
            self.script_menu_item_icons(menu)
        } else {
            Default::default()
        };
        let presentation = self
            .script_menu_presentations
            .get(&owner)
            .filter(|state| same_script_menu_presentation(state, target, menu));
        let location = presentation
            .and_then(|state| state.location)
            .or_else(|| self.script_menu_free_location(owner, menu));
        let scroll_y = presentation.map_or(0, |state| state.scroll_y);
        let explicit_lines = presentation.and_then(|state| state.explicit_lines);
        if presentation.map_or(location.is_some(), |state| {
            state.location_needs_initialization
        }) {
            let geometry = engine_script_menu_presentation_geometry_with_free_anchor(
                area,
                &font,
                menu,
                &item_icons,
                self.display_flags.show_commands,
                &font_images,
                location.expect("free anchor has a location"),
                scroll_y,
                false,
                explicit_lines,
            );
            return Ok(geometry.map(|geometry| (target, geometry)));
        }
        Ok(engine_script_menu_presentation_geometry(
            area,
            &font,
            menu,
            &item_icons,
            self.display_flags.show_commands,
            &font_images,
            location,
            scroll_y,
            explicit_lines,
        )
        .map(|geometry| (target, geometry)))
    }

    pub(crate) fn arm_script_menu_title_drag(
        &mut self,
        owner: i32,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        if !self.ensure_script_menu_presentation_for_owner(owner) {
            return Ok(false);
        }
        let Some((target, geometry)) = self.script_menu_geometry_for_owner(owner)? else {
            return Ok(false);
        };
        let start_location = (geometry.bounds.x, geometry.bounds.y);
        let Some(state) = self
            .script_menu_presentations
            .get_mut(&owner)
            .filter(|state| state.key.target == target)
        else {
            return Ok(false);
        };
        state.location = Some(start_location);
        state.location_needs_initialization = false;
        self.menu_title_drag = Some(MenuTitleDrag::Script {
            owner,
            target,
            start_pointer: point,
            start_location,
        });
        Ok(true)
    }

    pub(crate) fn script_menu_free_location(
        &self,
        owner: i32,
        menu: &clonk_engine::ObjectMenuState,
    ) -> Option<(i32, i32)> {
        if let Some(location) = menu.location {
            let area = self.graphics.viewport_rect(owner).unwrap_or_else(|| {
                let surface = self.graphics.surface();
                Rect::new(0, 0, surface.width(), surface.height())
            });
            return Some((
                area.x.saturating_add(location.x),
                area.y.saturating_add(location.y),
            ));
        }
        if menu.style != 2 || menu.user_menu {
            return None;
        }
        let target_id = menu.items.first()?.picture_object?;
        let target = self.snapshot.object(target_id)?;
        let shape = self.engine.object_current_shape_rect(target_id)?;
        let anchor = Vector2::new(
            target
                .position
                .x
                .saturating_add(shape.x)
                .saturating_add(shape.width)
                .saturating_add(10),
            target.position.y.saturating_add(shape.y),
        );
        self.graphics
            .world_to_screen(owner, anchor)
            .map(|(x, y)| (x.floor() as i32, y.floor() as i32))
    }

    #[inline(never)]
    pub(crate) fn handle_menu_actions(
        &mut self,
        actions: Vec<StartupMenuAction>,
    ) -> Result<(), EngineError> {
        let (start_identifier, updated_label) = self
            .process_menu_actions(actions)
            .map_err(classic_parity_engine_error)?;

        if let Some(label) = updated_label {
            self.scenario_label = label;
        }

        if let Some(identifier) = start_identifier {
            if let Some(scenario) = self.scenario_catalog.get(&identifier).cloned() {
                if self.startup_view == StartupView::ScenarioBrowser {
                    if let Some(message) = self
                        .scenario_selector_open_error(&scenario, self.scenario_selector_mode)
                        .map_err(classic_parity_engine_error)?
                    {
                        let caption = self.runtime_resource_string("IDS_MSG_CANNOTSTARTSCENARIO");
                        self.status_text.clear();
                        self.push_message_dialog(
                            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                                message,
                                caption,
                                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                            ),
                            MessageDialogContinuation::None,
                        )?;
                        return Ok(());
                    }
                    if self.scenario_selector_mode == ScenarioSelectorMode::NetworkHost {
                        match self
                            .network_scenario_open_decision(&scenario)
                            .map_err(classic_parity_engine_error)?
                        {
                            NetworkScenarioOpenDecision::Proceed => {}
                            NetworkScenarioOpenDecision::Error { message, caption } => {
                                self.status_text.clear();
                                self.push_message_dialog(
                                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                                        message,
                                        caption,
                                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                                    ),
                                    MessageDialogContinuation::None,
                                )?;
                                return Ok(());
                            }
                            NetworkScenarioOpenDecision::Warning { message, caption }
                                if !self.startup_message_hidden("HideMsgStartDedicated") =>
                            {
                                self.status_text.clear();
                                let checkbox = self.runtime_resource_text(
                                    "IDS_MSG_DONTSHOW",
                                    "&Don't display this message in the future.",
                                );
                                self.push_message_dialog(
                                    clonk_frontend::message_dialog::MessageDialogState::new(
                                        message,
                                        caption,
                                        clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL,
                                        clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                                        clonk_frontend::message_dialog::MessageDialogSize::Regular,
                                        false,
                                    )
                                    .with_checkbox(checkbox, false),
                                    MessageDialogContinuation::NetworkScenarioPlayerCountWarning {
                                        scenario,
                                    },
                                )?;
                                return Ok(());
                            }
                            NetworkScenarioOpenDecision::Warning { .. } => {}
                        }
                    }
                }
                self.continue_scenario_from_selector(scenario)?;
            } else {
                tracing::warn!(
                    scenario = %identifier,
                    "selected scenario is not available in Rust catalog"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn process_menu_actions(
        &mut self,
        actions: Vec<StartupMenuAction>,
    ) -> std::result::Result<(Option<String>, Option<String>), ClassicParityBoundary> {
        let mut start_identifier: Option<String> = None;
        let mut updated_label: Option<String> = None;
        let mut pending: VecDeque<StartupMenuAction> = actions.into();

        while let Some(action) = pending.pop_front() {
            match action {
                StartupMenuAction::SelectionChanged(_) => {
                    self.menu_state.sync_definition_checkbox_to_selection();
                    self.sync_scenario_game_option_constraint();
                    self.play_ui_sound("Command");
                }
                StartupMenuAction::StartScenario(summary) => {
                    if summary.kind == ScenarioKind::Editor {
                        return Err(report_classic_parity_boundary(
                            ClassicParityBoundary::EditorScenario {
                                identifier: summary.identifier,
                            },
                        ));
                    }
                    let entry_kind = self
                        .menu_state
                        .require_supported_activation(&summary.identifier)
                        .map_err(report_classic_parity_boundary)?;
                    if matches!(entry_kind, Some(ScenarioKind::Editor)) {
                        return Err(report_classic_parity_boundary(
                            ClassicParityBoundary::EditorScenario {
                                identifier: summary.identifier,
                            },
                        ));
                    }
                    self.play_ui_sound("Click");
                    if matches!(self.startup_view, StartupView::NetworkLobby) {
                        if self.select_network_lobby_scenario(&summary.identifier, &summary.title) {
                            self.status_text = format!("Selected {}", summary.title);
                        }
                    } else {
                        start_identifier = Some(summary.identifier);
                    }
                }
                StartupMenuAction::OpenEntry(summary) => {
                    if summary.identifier == BACK_ENTRY_IDENTIFIER {
                        self.play_ui_sound("DoorClose");
                        if self.menu_state.stack.len() <= 1 {
                            self.close_scenario_browser();
                        } else {
                            self.menu_state.leave_folder();
                            self.configure_current_folder_map();
                            self.refresh_scenario_entry_enabled();
                            updated_label = Some(self.menu_state.label_path());
                            pending.extend(self.menu_state.select_default_entry());
                        }
                        continue;
                    }

                    if summary.kind == ScenarioKind::Editor {
                        return Err(report_classic_parity_boundary(
                            ClassicParityBoundary::EditorScenario {
                                identifier: summary.identifier,
                            },
                        ));
                    }

                    let entry_kind = self
                        .menu_state
                        .require_supported_activation(&summary.identifier)
                        .map_err(report_classic_parity_boundary)?;

                    match entry_kind {
                        Some(ScenarioKind::Folder) => {
                            self.play_ui_sound("DoorOpen");
                            self.enter_scenario_folder(&summary.identifier);
                            self.scenario_game_options.set_focused_button(None);
                            updated_label = Some(self.menu_state.label_path());
                            pending.extend(self.menu_state.select_default_entry());
                        }
                        Some(ScenarioKind::Scenario) => {
                            self.play_ui_sound("Click");
                            if matches!(self.startup_view, StartupView::NetworkLobby) {
                                if self.select_network_lobby_scenario(
                                    &summary.identifier,
                                    &summary.title,
                                ) {
                                    self.status_text = format!("Selected {}", summary.title);
                                }
                            } else {
                                start_identifier = Some(summary.identifier);
                            }
                        }
                        Some(ScenarioKind::Editor) => {
                            return Err(report_classic_parity_boundary(
                                ClassicParityBoundary::EditorScenario {
                                    identifier: summary.identifier,
                                },
                            ));
                        }
                        None => {
                            self.play_ui_sound("DoorOpen");
                            self.enter_scenario_folder(&summary.identifier);
                            self.scenario_game_options.set_focused_button(None);
                            updated_label = Some(self.menu_state.label_path());
                            pending.extend(self.menu_state.select_default_entry());
                        }
                    }
                }
                StartupMenuAction::EditEntry(summary) => {
                    return Err(report_classic_parity_boundary(
                        ClassicParityBoundary::EditScenario {
                            identifier: summary.identifier,
                        },
                    ));
                }
            }
        }
        Ok((start_identifier, updated_label))
    }

    pub(crate) fn process_network_dialog_actions(
        &mut self,
        actions: Vec<clonk_frontend::startup_netdlg::NetDlgAction>,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_netdlg::NetDlgAction;

        if self.startup_network_transition_active() {
            return Ok(());
        }

        for action in actions {
            match action {
                NetDlgAction::FocusChanged(_)
                | NetDlgAction::ModeChanged(clonk_frontend::startup_netdlg::NetDlgMode::GameList)
                | NetDlgAction::JoinAddressChanged(_) => {}
                NetDlgAction::ModeChanged(clonk_frontend::startup_netdlg::NetDlgMode::Chat) => {
                    self.sync_startup_irc_snapshot();
                }
                NetDlgAction::OpenJoinAddressContextMenu(request) => {
                    let entries = request
                        .items
                        .into_iter()
                        .map(|item| {
                            let (label_key, label, tooltip_key, tooltip) = match item.command {
                                clonk_frontend::startup_netdlg::NetDlgEditContextCommand::Cut => (
                                    "IDS_DLG_CUT",
                                    item.label.as_str(),
                                    "IDS_DLGTIP_CUT",
                                    item.tooltip.as_str(),
                                ),
                                clonk_frontend::startup_netdlg::NetDlgEditContextCommand::Copy => (
                                    "IDS_DLG_COPY",
                                    item.label.as_str(),
                                    "IDS_DLGTIP_COPY",
                                    item.tooltip.as_str(),
                                ),
                                clonk_frontend::startup_netdlg::NetDlgEditContextCommand::Paste => (
                                    "IDS_DLG_PASTE",
                                    item.label.as_str(),
                                    "IDS_DLGTIP_PASTE",
                                    item.tooltip.as_str(),
                                ),
                                clonk_frontend::startup_netdlg::NetDlgEditContextCommand::Clear => (
                                    "IDS_DLG_CLEAR",
                                    item.label.as_str(),
                                    "IDS_DLGTIP_CLEAR",
                                    item.tooltip.as_str(),
                                ),
                                clonk_frontend::startup_netdlg::NetDlgEditContextCommand::SelectAll => (
                                    "IDS_DLG_SELALL",
                                    item.label.as_str(),
                                    "IDS_DLGTIP_SELALL",
                                    item.tooltip.as_str(),
                                ),
                            };
                            ContextMenuEntry::new(
                                self.runtime_resource_text(label_key, label),
                            )
                            .with_tooltip(
                                self.runtime_resource_text(tooltip_key, tooltip),
                            )
                            .with_icon(ContextMenuIcon::None)
                            .with_action(AppContextMenuCommand::NetworkJoinEdit(item.command))
                        })
                        .collect();
                    self.open_context_menu_at(entries, request.anchor)?;
                }
                NetDlgAction::ClipboardTransfer { text, cut } => {
                    match arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(text))
                    {
                        Ok(()) if cut => {
                            let fonts = self.assets.clonk_fonts.clone();
                            let follow_up = fonts
                                .as_deref()
                                .and_then(|fonts| {
                                    self.input_network_dialog_mut()
                                        .map(|dialog| dialog.confirm_clipboard_cut(&fonts.text))
                                })
                                .unwrap_or_default();
                            self.process_network_dialog_actions(follow_up)?;
                        }
                        Ok(()) => {}
                        Err(error) => {
                            tracing::warn!(%error, "failed to copy join-address edit text");
                        }
                    }
                }
                NetDlgAction::GuiSound(sound) => self.play_ui_sound(match sound {
                    clonk_frontend::startup_netdlg::NetDlgSound::ArrowHit => "ArrowHit",
                    clonk_frontend::startup_netdlg::NetDlgSound::Command => "Command",
                    clonk_frontend::startup_netdlg::NetDlgSound::Error => "Error",
                }),
                NetDlgAction::ChatConnect(login) => {
                    self.request_startup_irc_connection(login)?;
                }
                NetDlgAction::ChatValidationFailed(error) => {
                    self.show_startup_irc_validation_error(error.field())?;
                }
                NetDlgAction::ChatCommand(command) => self.dispatch_startup_irc_command(command),
                NetDlgAction::ChatDisconnectConfirmationRequested => {
                    self.request_startup_irc_disconnect_confirmation()?;
                }
                NetDlgAction::ChatDisconnect => {
                    self.disconnect_startup_irc();
                    self.show_irc_login_on_all_controllers();
                }
                NetDlgAction::ChatHistoryStored(text) => {
                    self.store_message_input_history(&text);
                }
                NetDlgAction::ChatDialogCloseRequested => self.hide_external_irc_dialog(),
                NetDlgAction::ChatSelectSheet { .. } => {
                    self.sync_startup_irc_snapshot();
                }
                NetDlgAction::Back => {
                    self.begin_startup_dialog_fade(StartupDialog::MainMenu);
                    self.show_main_menu();
                }
                NetDlgAction::Refresh => {
                    self.request_startup_network_refresh()?;
                }
                NetDlgAction::QueryAddress { address } => {
                    self.begin_startup_direct_reference_query(address);
                }
                NetDlgAction::OpenUrl(url) => {
                    if let Err(error) = open_external_http_url(&url) {
                        tracing::warn!(%url, %error, "failed to open network-dialog hyperlink");
                        self.status_text = format!("Unable to open link: {error}");
                    }
                }
                NetDlgAction::JoinGame { .. } => {
                    let selected_index = self
                        .startup_network_dialog
                        .as_ref()
                        .and_then(|dialog| dialog.selected_game());
                    let target =
                        selected_index.and_then(|index| self.startup_network_join_target(index));
                    let Some(target) = target else {
                        self.status_text.clear();
                        let message = self.runtime_resource_text(
                            "IDS_NET_NOJOIN_NOREF",
                            "No reference selected. Select a game from the list or enter a direct join address below!",
                        );
                        let caption =
                            self.runtime_resource_text("IDS_NET_NOJOIN", "Cannot join game");
                        self.push_message_dialog(
                            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                                message,
                                caption,
                                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                            ),
                            MessageDialogContinuation::None,
                        )?;
                        continue;
                    };
                    match target {
                        StartupNetworkJoinTarget::Reference(reference) => {
                            self.request_network_reference_join(reference)?;
                        }
                        StartupNetworkJoinTarget::DirectAddress(address) => {
                            self.activate_network_join(address)?;
                        }
                        StartupNetworkJoinTarget::QueryError(error) => {
                            let message = format_resource_string(
                                self.runtime_resource_text(
                                    "IDS_NET_NOJOIN_BADREF",
                                    "Cannot join selected game: %s",
                                ),
                                &[&error],
                            );
                            let caption =
                                self.runtime_resource_text("IDS_NET_NOJOIN", "Cannot join game");
                            self.push_message_dialog(
                                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                                    message,
                                    caption,
                                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                                ),
                                MessageDialogContinuation::None,
                            )?;
                        }
                    }
                }
                NetDlgAction::CreateGame => {
                    // C4StartupNetDlg opens C4StartupScenSelDlg(true) before
                    // any host socket or NetworkManager exists.
                    self.begin_startup_dialog_fade(StartupDialog::ScenarioBrowser(
                        ScenarioSelectorMode::NetworkHost,
                    ));
                    self.open_network_host_scenario_browser();
                }
                NetDlgAction::MasterserverSignupChanged(enabled) => {
                    // UpdateMasterserver creates the query object it is about
                    // to enable, so the icon can never read "on" with nothing
                    // behind it (src/C4StartupNetDlg.cpp:851-866).
                    self.restore_startup_game_search();
                    if let Some(search) = self.startup_game_search.as_ref() {
                        let _ = search.set_internet_enabled(enabled);
                    }
                    if enabled {
                        let now = Instant::now();
                        self.startup_network_last_refresh = Some(now);
                        self.startup_masterserver_next_query_at =
                            now.checked_add(clonk_network::GAME_SEARCH_INTERVAL);
                        self.reset_startup_masterserver_entry_at(now);
                    } else {
                        self.startup_masterserver_next_query_at = None;
                        self.startup_masterserver_request_timeout_at = None;
                    }
                    // `OnBtnInternet` flips the flag in memory only; the file
                    // is written once at shutdown (C4StartupNetDlg.cpp:840-845).
                    self.deferred_config.set(
                        "Network",
                        "MasterServerSignUp",
                        i32::from(enabled).to_string(),
                    );
                }
                NetDlgAction::RecordingChanged(record) => {
                    self.startup_view_flags.record = record;
                    self.recording_enabled = record && self.recordings_dir.is_some();
                    // `OnBtnRecord` likewise mutates memory only (:847-850).
                    self.deferred_config
                        .set("General", "Record", i32::from(record).to_string());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn open_network_join_password_dialog(&mut self) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "Network join password input dialog",
            self.assets.input_dialog_resources().map(|_| ()),
        )?;
        self.close_context_menu_silently();
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.cancel_interaction();
        }
        let controller = InputDialogController::new(
            "Enter password:",
            "Enter password:",
            InputDialogIcon::LOCKED,
        );
        self.startup_tooltip.pointer_left();
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::NetworkJoinPassword,
            controller,
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        self.status_text.clear();
        Ok(())
    }

    pub(crate) fn open_context_menu_at(
        &mut self,
        entries: Vec<ContextMenuEntry<AppContextMenuCommand>>,
        anchor: GuiPoint,
    ) -> Result<bool, EngineError> {
        self.open_context_menu_at_with_minimum_width(entries, anchor, 0, None)
    }

    pub(crate) fn open_context_menu_at_with_minimum_width(
        &mut self,
        entries: Vec<ContextMenuEntry<AppContextMenuCommand>>,
        anchor: GuiPoint,
        minimum_width: i32,
        lobby_team_player: Option<i32>,
    ) -> Result<bool, EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        let resources = self
            .assets
            .context_menu_resources()
            .map_err(|error| Self::gui_overlay_engine_error("C4GUI context menu", error))?;
        let surface = self.graphics.surface();
        let screen = clonk_frontend::classic_gui::IntRect::new(
            0,
            0,
            surface.width() as i32,
            surface.height() as i32,
        );
        let (menu, outcome) = ClassicContextMenu::open_with_minimum_width(
            entries,
            anchor,
            screen,
            resources,
            minimum_width,
        );
        self.startup_tooltip.pointer_left();
        self.note_classic_lobby_non_pointer_input();
        self.context_menu = Some(menu);
        self.context_menu_lobby_kick_client = None;
        self.context_menu_lobby_player = None;
        self.set_context_menu_lobby_option(None);
        self.set_context_menu_lobby_team_player(lobby_team_player);
        self.process_context_menu_outcome(outcome)?;
        Ok(true)
    }

    pub(crate) fn open_scenario_search_context_menu(
        &mut self,
        keyboard_trigger: bool,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::ScenarioBrowser
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let Some(fonts) = self.assets.clonk_fonts.as_deref() else {
            return Err(Self::gui_overlay_engine_error(
                "scenario-search context menu",
                "classic GUI fonts are unavailable",
            ));
        };
        let search = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            fonts,
        )
        .search_edit;
        let anchor = if keyboard_trigger {
            if !self.menu_state.search_focused() {
                return Ok(false);
            }
            GuiPoint::new(
                (search.x + search.w / 2) as f32,
                (search.y + search.h / 2) as f32,
            )
        } else {
            let Some(point) = self.menu_state.pointer_position().filter(|point| {
                point.x >= search.x as f32
                    && point.x < (search.x + search.w) as f32
                    && point.y >= search.y as f32
                    && point.y < (search.y + search.h) as f32
            }) else {
                return Ok(false);
            };
            point
        };
        let entries = scensel_search_context_entries(
            &self.menu_state.search_edit,
            clipboard_text_available(),
        );
        self.open_context_menu_at(entries, anchor)
    }

    pub(crate) fn process_context_menu_outcome(
        &mut self,
        outcome: ContextMenuOutcome<AppContextMenuCommand>,
    ) -> Result<(), EngineError> {
        for event in outcome.events {
            match event {
                ContextMenuEvent::Sound(sound) => self.play_ui_sound(match sound {
                    ContextMenuSound::DoorOpen => "DoorOpen",
                    ContextMenuSound::DoorClose => "DoorClose",
                    ContextMenuSound::Command => "Command",
                    ContextMenuSound::Click => "Click",
                }),
                ContextMenuEvent::Closed => {
                    self.startup_tooltip.pointer_left();
                    self.note_classic_lobby_non_pointer_input();
                    self.context_menu = None;
                    self.set_context_menu_lobby_team_player(None);
                    self.set_context_menu_lobby_option(None);
                    self.context_menu_lobby_kick_client = None;
                    self.context_menu_lobby_player = None;
                }
                ContextMenuEvent::Activated(command) => match command {
                    AppContextMenuCommand::StartupPlayer(
                        PlrSelPlayerContextCommand::PlayerProperties(index),
                    ) => {
                        self.open_existing_startup_player_properties(index);
                    }
                    AppContextMenuCommand::StartupPlayer(
                        PlrSelPlayerContextCommand::DeletePlayer(index),
                    ) => {
                        // ContextMenu already emitted the activation Click.
                        self.open_startup_player_delete_dialog(index)?;
                    }
                    AppContextMenuCommand::StartupCrew(PlrSelCrewContextCommand::RenameCrew(
                        index,
                    )) => {
                        self.abort_startup_crew_rename();
                        self.start_startup_crew_rename(index)?;
                    }
                    AppContextMenuCommand::StartupCrew(PlrSelCrewContextCommand::DeleteCrew(
                        index,
                    )) => {
                        self.abort_startup_crew_rename();
                        self.open_startup_crew_delete_dialog(index)?;
                    }
                    AppContextMenuCommand::StartupCrew(
                        PlrSelCrewContextCommand::SetCrewDeathMessage(index),
                    ) => {
                        self.open_startup_crew_death_message_dialog(index)?;
                    }
                    AppContextMenuCommand::AddStartupParticipant(reference) => {
                        self.set_startup_participant(&reference, true);
                    }
                    AppContextMenuCommand::RemoveStartupParticipant(index) => {
                        self.remove_startup_participant(index);
                    }
                    AppContextMenuCommand::OptionsLanguage(code) => {
                        let selected = self
                            .startup_options_dialog
                            .as_mut()
                            .is_some_and(|dialog| dialog.select_language(&code));
                        if !selected {
                            tracing::error!(code, "language combo selected a stale entry");
                            continue;
                        }
                        match self.persist_open_options_config() {
                            Some(Ok(())) => {
                                match self.reload_application_language_resources() {
                                    Ok(charset) => {
                                        // Stays eager. `C4Language::InitInfos`
                                        // overwrites
                                        // `Config.General.LanguageCharset` from
                                        // the loaded table (C4Language.cpp:311)
                                        // and the Options dialog then writes the
                                        // whole config —
                                        // `C4StartupOptionsDlg::SaveConfig` ends
                                        // in an outright `Config.Save()`
                                        // "in case the game crashes later on"
                                        // (C4StartupOptionsDlg.cpp:1183) — so
                                        // the charset reaches the file with the
                                        // options that changed it.
                                        if let Some(paths) = self.app_paths.as_ref() {
                                            if let Err(error) = persist_config_value(
                                                paths,
                                                "General",
                                                "LanguageCharset",
                                                charset,
                                            ) {
                                                tracing::warn!(
                                                    error = %error,
                                                    "failed to save selected language charset"
                                                );
                                            }
                                        }
                                    }
                                    Err(error) => tracing::error!(
                                        error = %error,
                                        "failed to reload selected application language"
                                    ),
                                }
                                self.begin_startup_dialog_fade(StartupDialog::Options);
                                self.open_options_menu();
                            }
                            Some(Err(error)) => tracing::warn!(
                                error = %error,
                                "failed to save selected options language"
                            ),
                            None => {
                                let _ = self.reload_application_language_resources();
                                self.begin_startup_dialog_fade(StartupDialog::Options);
                                self.open_options_menu();
                            }
                        }
                    }
                    AppContextMenuCommand::OptionsFontFace(face) => {
                        self.apply_options_font_selection(Some(face), None)?;
                    }
                    AppContextMenuCommand::OptionsFontSize(size) => {
                        self.apply_options_font_selection(None, Some(size))?;
                    }
                    AppContextMenuCommand::OptionsDisplayMode(mode) => {
                        let changed = self
                            .startup_options_dialog
                            .as_mut()
                            .and_then(|dialog| dialog.graphics_mut().set_display_mode(mode))
                            .is_some();
                        if changed {
                            self.queue_options_display_request(OptionsDisplayRequest::SetMode(
                                mode,
                            ));
                        }
                    }
                    AppContextMenuCommand::LobbyTeam { player_id, team_id } => {
                        self.submit_classic_lobby_team_selection(player_id, team_id);
                    }
                    AppContextMenuCommand::LobbyControlRate(control_rate) => {
                        self.submit_classic_lobby_control_rate(control_rate);
                    }
                    AppContextMenuCommand::LobbyRuntimeJoin(allowed) => {
                        self.set_classic_lobby_runtime_join(allowed);
                    }
                    AppContextMenuCommand::RuntimeClientOption { option, value } => {
                        self.apply_runtime_client_list_option(option, value)?;
                    }
                    AppContextMenuCommand::LobbyTeamDistribution(distribution) => {
                        self.submit_classic_lobby_team_setting(
                            LobbyOptionKind::TeamDistribution,
                            distribution,
                        );
                    }
                    AppContextMenuCommand::LobbyTeamColors(enabled) => {
                        self.submit_classic_lobby_team_setting(
                            LobbyOptionKind::TeamColors,
                            i32::from(enabled),
                        );
                    }
                    AppContextMenuCommand::LobbyRandomTeamCount(count) => {
                        self.set_classic_lobby_random_team_count(count);
                    }
                    AppContextMenuCommand::LobbyPlayerTakeOver {
                        savegame_player_id,
                        player_id,
                    } => {
                        self.take_over_classic_lobby_savegame_player(savegame_player_id, player_id);
                    }
                    AppContextMenuCommand::LobbyPlayerTakeOverSubmenu { .. } => {
                        // The deferred parent entry carries no menu handler;
                        // this request only arrives via SubmenuRequested.
                    }
                    AppContextMenuCommand::LobbyPlayerRemove {
                        client_id,
                        player_id,
                    } => {
                        self.remove_classic_lobby_player(client_id, player_id);
                    }
                    AppContextMenuCommand::LobbyPlayerNewColor {
                        client_id,
                        player_id,
                    } => {
                        self.reset_classic_lobby_player_color(client_id, player_id);
                    }
                    AppContextMenuCommand::LobbyClientToggleMute(client_id) => {
                        self.toggle_classic_lobby_client_mute(client_id);
                    }
                    AppContextMenuCommand::LobbyClientToggleActivate(client_id) => {
                        self.toggle_classic_lobby_client_activation(client_id);
                    }
                    AppContextMenuCommand::LobbyClientInfo(client_id) => {
                        self.open_classic_lobby_client_info(client_id)?;
                    }
                    AppContextMenuCommand::LobbyKick(client_id) => {
                        self.kick_classic_lobby_client(client_id);
                    }
                    AppContextMenuCommand::LobbySheet(sheet) => {
                        if self.classic_host_lobby_active() {
                            if !self.select_classic_lobby_sheet(sheet) {
                                return Err(classic_game_lobby_child_error(
                                    ClassicGameLobbyChild::Sheet(sheet),
                                ));
                            }
                        } else {
                            self.process_lobby_action(LobbyAction::SelectSheet(sheet))?;
                        }
                    }
                    AppContextMenuCommand::NetworkJoinEdit(command) => {
                        self.apply_network_join_edit_context_command(command)?;
                    }
                    AppContextMenuCommand::LeagueSignupEdit { field, command } => {
                        self.apply_league_signup_edit_context_command(field, command)?;
                    }
                    AppContextMenuCommand::LobbyChat(command) => {
                        self.process_classic_lobby_chat_request(LobbyChatRequest::ContextCommand(
                            command,
                        ))?;
                    }
                    AppContextMenuCommand::ScenarioSearch(command) => {
                        self.execute_scenario_search_context_command(command)?;
                    }
                    AppContextMenuCommand::StartupCrewRename(command) => {
                        self.execute_startup_crew_rename_context_command(command);
                    }
                    AppContextMenuCommand::InputDialog(command) => {
                        self.apply_input_dialog_context_command(command)?;
                    }
                },
                // C4GUI fills a submenu when it opens: CheckOpenSubmenu runs
                // the entry's OnSubcontext callback and opens the returned
                // menu in the same dispatch (src/C4GuiMenu.cpp:469-506).
                ContextMenuEvent::SubmenuRequested(command) => {
                    let entries = match command {
                        AppContextMenuCommand::LobbyPlayerTakeOverSubmenu {
                            savegame_player_id,
                        } => self.classic_lobby_takeover_entries(savegame_player_id),
                        other => {
                            tracing::error!(
                                ?other,
                                "context submenu request without a live provider"
                            );
                            Vec::new()
                        }
                    };
                    if let Some(outcome) = self
                        .context_menu
                        .as_mut()
                        .map(|menu| menu.fill_requested_submenu(entries))
                    {
                        self.process_context_menu_outcome(outcome)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn close_context_menu_silently(&mut self) {
        let Some(mut menu) = self.context_menu.take() else {
            self.set_context_menu_lobby_team_player(None);
            self.set_context_menu_lobby_option(None);
            self.context_menu_lobby_kick_client = None;
            self.context_menu_lobby_player = None;
            self.context_menu_pointer_dismissed_lobby_team_player = None;
            self.context_menu_pointer_dismissed_lobby_option = None;
            return;
        };
        let _ = menu.dismiss(false);
        self.startup_tooltip.pointer_left();
        self.note_classic_lobby_non_pointer_input();
        self.set_context_menu_lobby_team_player(None);
        self.set_context_menu_lobby_option(None);
        self.context_menu_lobby_kick_client = None;
        self.context_menu_lobby_player = None;
        self.context_menu_pointer_dismissed_lobby_team_player = None;
        self.context_menu_pointer_dismissed_lobby_option = None;
        self.context_menu_pointer_capture = None;
    }

    pub(crate) fn process_player_dialog_actions(
        &mut self,
        actions: Vec<clonk_frontend::startup_plrsel::PlrSelAction>,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_plrsel::{PlrSelAction, PlrSelSound};

        let sounds = self
            .startup_player_dialog
            .as_mut()
            .map(|dialog| dialog.take_sound_events())
            .unwrap_or_default();
        for sound in sounds {
            self.play_ui_sound(match sound {
                PlrSelSound::Command => "Command",
                PlrSelSound::ArrowHit => "ArrowHit",
                PlrSelSound::Click => "Click",
            });
        }

        for action in actions {
            match action {
                PlrSelAction::SelectionChanged(_) => {
                    self.abort_startup_crew_rename();
                }
                PlrSelAction::FocusChanged(_) => {}
                PlrSelAction::Back => {
                    self.begin_startup_dialog_fade(StartupDialog::MainMenu);
                    self.show_main_menu();
                }
                PlrSelAction::NewPlayer => {
                    self.open_new_startup_player_properties();
                }
                PlrSelAction::ActivationChanged { index, activated } => {
                    let selected_before = self
                        .startup_player_dialog
                        .as_ref()
                        .and_then(|dialog| dialog.selected_index());
                    let old_activated = self
                        .startup_player_models
                        .get(index)
                        .map(|player| player.activated);
                    if let Some(player) = self.startup_player_files.get_mut(index) {
                        player.set_activated(activated);
                    }
                    if let Some(player) = self.startup_player_models.get_mut(index) {
                        player.activated = activated;
                    }
                    let persisted = self
                        .app_paths
                        .as_ref()
                        .ok_or_else(|| "application paths are unavailable".to_string())
                        .and_then(|paths| {
                            persist_activations(
                                &paths.config_file(),
                                &mut self.startup_player_files,
                            )
                            .map_err(|error| error.to_string())
                        });
                    match persisted {
                        Ok(refusals) => {
                            for refusal in &refusals {
                                if let Some(player) =
                                    self.startup_player_models.get_mut(refusal.index)
                                {
                                    player.activated = false;
                                }
                                if let Some(dialog) = self.startup_player_dialog.as_mut() {
                                    dialog.set_player_activation(refusal.index, false);
                                }
                            }
                            self.selected_player_file = self
                                .startup_player_files
                                .iter()
                                .find(|player| player.render_model.activated)
                                .map(|player| player.player_file.clone());
                            self.refresh_participants_label();
                            self.status_text.clear();
                            self.show_startup_player_activation_refusals(&refusals)?;
                        }
                        Err(error) => {
                            if let Some(old_activated) = old_activated {
                                if let Some(player) = self.startup_player_files.get_mut(index) {
                                    player.set_activated(old_activated);
                                }
                                if let Some(player) = self.startup_player_models.get_mut(index) {
                                    player.activated = old_activated;
                                }
                                if let Some(dialog) = self.startup_player_dialog.as_mut() {
                                    dialog.set_player_activations(
                                        self.startup_player_models
                                            .iter()
                                            .map(|player| player.activated)
                                            .collect(),
                                    );
                                    dialog.set_selected_index(selected_before);
                                }
                            }
                            self.status_text = format!("Unable to save player selection: {error}");
                        }
                    }
                }
                PlrSelAction::DeletePlayer(index) => {
                    self.open_startup_player_delete_dialog(index)?;
                }
                PlrSelAction::PlayerProperties(index) => {
                    self.open_existing_startup_player_properties(index);
                }
                PlrSelAction::ShowCrew(index) => {
                    self.enter_startup_crew_mode(index)?;
                }
                PlrSelAction::LeaveCrew => {
                    self.leave_startup_crew_mode();
                }
                PlrSelAction::CrewParticipationChanged {
                    index,
                    participating,
                } => {
                    self.set_startup_crew_participation(index, participating)?;
                    self.abort_startup_crew_rename();
                }
                PlrSelAction::DeleteCrew(index) => {
                    self.abort_startup_crew_rename();
                    self.open_startup_crew_delete_dialog(index)?;
                }
                PlrSelAction::RenameCrew(index) => {
                    self.abort_startup_crew_rename();
                    self.start_startup_crew_rename(index)?;
                }
                PlrSelAction::SetCrewDeathMessage(index) => {
                    self.open_startup_crew_death_message_dialog(index)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn process_about_dialog_actions(
        &mut self,
        actions: Vec<clonk_frontend::startup_about_dlg::AboutDlgAction>,
    ) -> Result<(), EngineError> {
        self.process_about_dialog_actions_with_sound(actions, true)
    }

    /// `C4Startup::SetStartScreen` (C4Startup.cpp:389-408) maps the seven
    /// case-insensitive `/startup:` names onto the dialog it opens first.
    /// `netscen` is the scenario selector in network-host mode, distinct from
    /// both `scen` and the `net` game browser. An unknown name leaves the
    /// remembered/default view alone — C++ returns false and changes nothing.
    pub(crate) fn apply_classic_startup_screen(&mut self, screen: &str) {
        let screen = screen.trim();
        if screen.eq_ignore_ascii_case("main") {
            self.show_main_menu();
        } else if screen.eq_ignore_ascii_case("scen") {
            self.open_scenario_browser();
        } else if screen.eq_ignore_ascii_case("netscen") {
            self.open_network_host_scenario_browser();
        } else if screen.eq_ignore_ascii_case("net") {
            self.open_network_game_dialog();
        } else if screen.eq_ignore_ascii_case("options") {
            self.open_options_menu();
        } else if screen.eq_ignore_ascii_case("plrsel") {
            self.open_player_selection_dialog();
        } else if screen.eq_ignore_ascii_case("about") {
            self.open_about_dialog();
        } else {
            tracing::warn!(
                screen,
                "unknown classic /startup screen; keeping the default view"
            );
        }
    }

    /// `C4StartupMainDlg::SwitchToEditor` (C4StartupMainDlg.cpp:313-325). On
    /// Windows it refuses when the configured editor is absent — returning
    /// false so the key is not consumed — and otherwise flags the launch and
    /// exits startup; `~C4Application` spawns it after teardown
    /// (C4Application.cpp:58-74). The `#ifdef _WIN32` body is skipped
    /// elsewhere, so every other platform consumes the key and does nothing.
    pub(crate) fn switch_to_editor(&mut self) -> bool {
        if !cfg!(windows) {
            return true;
        }
        let Some(editor) = self.classic_editor_executable() else {
            return false;
        };
        self.pending_editor_launch = Some(editor);
        self.request_exit("the classic editor was launched");
        true
    }

    /// `Config.AtExePath(C4CFN_Editor)` — "Editor.exe" beside the engine
    /// (C4Components.h:23; C4StartupMainDlg.cpp:317).
    pub(crate) fn classic_editor_executable(&self) -> Option<PathBuf> {
        let editor = self.app_paths.as_ref()?.install_root().join("Editor.exe");
        editor.is_file().then_some(editor)
    }

    /// This session's `Network.MasterServerSignUp`. `OnBtnInternet` flips the
    /// process-wide `Config` and leaves the file to `C4Application::Clear`, and
    /// every reader — `UpdateMasterserver`, `OnShown`, the btnInternet icon —
    /// reads that in-memory value, so an unflushed toggle outranks the file
    /// (src/C4StartupNetDlg.cpp:710,771-777,838-845,851-866).
    pub(crate) fn masterserver_signup_setting(&self) -> bool {
        self.deferred_config
            .get("Network", "MasterServerSignUp")
            .and_then(parse_native_config_bool)
            .unwrap_or_else(|| load_network_startup_settings(self.app_paths.as_ref()).0)
    }

    /// Spawns the worker that stands in for `C4StartupNetDlg`'s DiscoverClient
    /// and pMasterserverClient and issues their first query
    /// (src/C4StartupNetDlg.cpp:737-738,864-865).
    fn start_startup_game_search(&mut self, search_config: clonk_network::NetworkGameSearchConfig) {
        let reference_config = load_reference_query_settings(self.app_paths.as_ref());
        self.startup_game_search =
            match clonk_network::StartupGameSearch::start_with_reference_config(
                search_config,
                reference_config,
            ) {
                Ok(search) => {
                    if search.initial_refresh().is_err() {
                        self.status_text = "Unable to start network game search".to_string();
                    }
                    Some(search)
                }
                Err(error) => {
                    self.status_text = format!("Unable to start network game search: {error}");
                    None
                }
            };
    }

    /// `C4StartupNetDlg` cannot be on screen without the clients that search
    /// for games: DiscoverClient is constructed with the dialog and closed only
    /// by its destructor, and OnShown -> UpdateMasterserver recreates
    /// pMasterserverClient whenever MasterServerSignUp is set and it is missing
    /// (src/C4StartupNetDlg.cpp:737-738,752,771-777,851-866). Every C++ route
    /// that stops the search also destroys the dialog, so a port path that
    /// keeps the dialog has to bring the worker back with it.
    pub(crate) fn restore_startup_game_search(&mut self) {
        let Some(masterserver_enabled) = self
            .startup_network_dialog
            .as_ref()
            .map(|dialog| dialog.config().masterserver_signup)
        else {
            return;
        };
        if self.startup_game_search.is_some() {
            return;
        }
        // The dialog's own flag is the in-memory `Config.Network.MasterServerSignUp`
        // that OnBtnInternet writes; the file only catches up at shutdown
        // (src/C4StartupNetDlg.cpp:838-845).
        let mut search_config = load_network_search_settings(self.app_paths.as_ref());
        search_config.internet_enabled = masterserver_enabled;
        self.startup_network_refresh_waiting_for_clear = false;
        self.start_startup_game_search(search_config);
        let now = Instant::now();
        self.startup_network_last_refresh = Some(now);
        self.startup_masterserver_next_query_at = masterserver_enabled
            .then(|| now.checked_add(clonk_network::GAME_SEARCH_INTERVAL))
            .flatten();
        self.reset_startup_masterserver_entry_at(now);
    }

    pub(crate) fn open_network_game_dialog(&mut self) {
        self.external_irc_dialog_visible = false;
        self.external_irc_dialog = None;
        self.hide_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
        self.irc_dialog_last_click = None;
        self.external_irc_pointer_capture = false;
        self.close_context_menu_silently();
        self.status_text.clear();
        self.startup_network_refresh_waiting_for_clear = false;
        self.startup_network_ignore_redirect = false;
        self.startup_game_references.clear();
        self.startup_discovery_reference_queries.clear();
        self.startup_direct_reference_queries.clear();
        self.netdlg_last_click = None;
        self.netdlg_join_edit_last_click = None;
        self.netdlg_edit_consumed_keys.clear();
        self.pending_network_join = None;
        let mut dialog = self.new_network_dialog_controller();
        let mut search_config = load_network_search_settings(self.app_paths.as_ref());
        search_config.internet_enabled = self.masterserver_signup_setting();
        let masterserver_enabled = search_config.internet_enabled;
        dialog.set_masterserver_entry(Self::startup_masterserver_query_entry(
            &self.startup_tooltip_resources,
            &search_config.master_server_url,
        ));
        self.start_startup_game_search(search_config);
        self.startup_network_dialog = Some(dialog);
        self.replace_startup_dialog(StartupView::NetworkGame, StartupDialog::NetworkGame);
        self.sync_startup_irc_snapshot();
        let now = Instant::now();
        self.startup_network_last_refresh = Some(now);
        self.startup_masterserver_next_query_at = if masterserver_enabled {
            now.checked_add(clonk_network::GAME_SEARCH_INTERVAL)
        } else {
            None
        };
        self.startup_masterserver_request_timeout_at = masterserver_enabled
            .then(|| now.checked_add(clonk_network::REFERENCE_QUERY_TIMEOUT))
            .flatten();
    }

    /// C4StartupNetDlg::OnShown refreshes the Internet icon and query-row
    /// presence from Config; its Record icon intentionally retains the
    /// controller's older value.
    pub(crate) fn refresh_retained_network_dialog_internet(&mut self) {
        // OnShown runs UpdateMasterserver before the first OnSec1Timer, so a
        // re-shown dialog always searches (src/C4StartupNetDlg.cpp:771-777).
        self.restore_startup_game_search();
        let masterserver_signup = self.masterserver_signup_setting();
        let recreate_masterserver = self.startup_network_dialog.as_mut().is_some_and(|dialog| {
            let recreate = !dialog.config().masterserver_signup && masterserver_signup;
            dialog.sync_masterserver_signup_from_config(masterserver_signup);
            recreate
        });
        if let Some(search) = self.startup_game_search.as_ref() {
            let _ = search.set_internet_enabled(masterserver_signup);
        }
        if !masterserver_signup {
            self.startup_masterserver_next_query_at = None;
            self.startup_masterserver_request_timeout_at = None;
        }
        if recreate_masterserver {
            let now = Instant::now();
            self.startup_network_last_refresh = Some(now);
            self.startup_masterserver_next_query_at =
                now.checked_add(clonk_network::GAME_SEARCH_INTERVAL);
            self.reset_startup_masterserver_entry_at(now);
        }
    }

    pub(crate) fn open_player_selection_dialog(&mut self) {
        self.abort_startup_crew_rename();
        self.close_context_menu_silently();
        self.startup_player_properties_dialog = None;
        self.startup_crew_files.clear();
        self.startup_crew_models.clear();
        self.startup_crew_player_index = None;
        let activation_refusals = self
            .app_paths
            .as_ref()
            .map(AppPaths::config_file)
            .map(|config_path| persist_activations(&config_path, &mut self.startup_player_files))
            .transpose();
        let activation_refusals = match activation_refusals {
            Ok(Some(refusals)) => refusals,
            Ok(None) => Vec::new(),
            Err(error) => {
                tracing::warn!(%error, "failed to validate participants while opening player selection");
                Vec::new()
            }
        };
        for refusal in &activation_refusals {
            if let Some(player) = self.startup_player_models.get_mut(refusal.index) {
                player.activated = false;
            }
        }
        if !activation_refusals.is_empty() {
            self.selected_player_file = self
                .startup_player_files
                .iter()
                .find(|player| player.render_model.activated)
                .map(|player| player.player_file.clone());
        }
        let mut dialog =
            clonk_frontend::startup_plrsel::PlrSelController::new(self.startup_player_models.len());
        dialog.set_player_activations(
            self.startup_player_models
                .iter()
                .map(|player| player.activated)
                .collect(),
        );
        let width = self.graphics.surface().width() as i32;
        let height = self.graphics.surface().height() as i32;
        if let (Some(fonts), Some(book)) = (
            self.assets.clonk_fonts.as_deref(),
            self.assets.plrsel_book_fonts.as_deref(),
        ) {
            dialog.resize_with_fonts(width, height, fonts, book);
        } else {
            dialog.resize(width, height);
        }
        self.startup_player_dialog = Some(dialog);
        self.plrsel_last_click = None;
        self.replace_startup_dialog(StartupView::PlayerSelection, StartupDialog::PlayerSelection);
        self.status_text.clear();
        if let Err(error) = self.show_startup_player_activation_refusals(&activation_refusals) {
            tracing::error!(%error, "failed to show participant overflow while opening player selection");
        }
    }

    pub(crate) fn open_about_dialog(&mut self) {
        self.close_context_menu_silently();
        let mut dialog = clonk_frontend::startup_about_dlg::AboutDlgState::new();
        // C4StartupAboutDlg's constructor resolves its bottom-line captions
        // through LoadResStr (C4StartupAboutDlg.cpp:279-284).
        dialog.set_labels(clonk_frontend::startup_about_dlg::AboutLabels {
            buttons: [
                self.runtime_resource_text("IDS_BTN_BACK", "Back"),
                self.runtime_resource_text("IDS_BTN_CHECKFORUPDATES", "Check for &updates"),
                self.runtime_resource_text("IDS_BTN_LICENSES", "&Licenses"),
            ],
        });
        if let Some(fonts) = self.assets.clonk_fonts.as_deref() {
            dialog.resize(
                self.graphics.surface().width() as i32,
                self.graphics.surface().height() as i32,
                fonts,
            );
        }
        self.startup_about_dialog = Some(dialog);
        self.replace_startup_dialog(StartupView::About, StartupDialog::About);
        self.status_text.clear();
    }

    pub(crate) fn open_next_league_vote_dialog(&mut self) -> Result<(), EngineError> {
        let already_open = self.message_dialogs.iter().any(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::LeagueVote { .. }
                    | MessageDialogContinuation::LeagueSurrender
            )
        });
        if already_open {
            return Ok(());
        }
        let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            return Ok(());
        };
        let has_joined_local_player = self
            .engine
            .players()
            .any(|player| player.at_client().get() == local_client_id);
        if !has_joined_local_player {
            return Ok(());
        }
        let Some(origin) = self
            .league_votes
            .first_subject_needing_vote(local_client_id)
        else {
            return Ok(());
        };
        let subject = LeagueVoteSubject::from(origin);
        let origin_name = self.league_vote_client_name(origin.by_client);
        let description = self.league_vote_description(origin);
        let warning = match origin.vote_type {
            clonk_engine::VOTE_TYPE_CANCEL => {
                "Notice: if the game is cancelled, no league score will be awarded."
            }
            clonk_engine::VOTE_TYPE_KICK => {
                "Notice: if a player leaves without being defeated, the opposing players will gain less league score in case of a win."
            }
            _ => "",
        };
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                format!("{origin_name} wants to {description}. Allow?|{warning}"),
                "Voting",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                true,
            ),
            MessageDialogContinuation::LeagueVote { subject },
        )
    }

    pub(crate) fn open_league_surrender_dialog(&mut self) -> Result<(), EngineError> {
        if self.message_dialogs.iter().any(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::LeagueVote { .. }
                    | MessageDialogContinuation::LeagueSurrender
            )
        }) {
            return Ok(());
        }
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                "It was decided that you cannot leave the game. However, you can forfeit the game instead.||Do you want to surrender?",
                "Voting",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                true,
            ),
            MessageDialogContinuation::LeagueSurrender,
        )
    }

    pub(crate) fn handle_menu_requests(&mut self) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(());
        }
        let local_owner = self.local_owner;
        for request in &self.snapshot.menu_requests {
            if request.owner != local_owner {
                continue;
            }
            match &request.kind {
                // Every `C4ObjectMenu` request is opened by the engine while
                // the command event is applied, so nothing here is a live
                // request: what reaches this loop is a stale serialized
                // record, and the app has no menu of its own to build from
                // one. Ignore them all rather than resurrecting a non-C++
                // pane (clonk-org/clonk-rs#1205, clonk-org/clonk-rs#1206).
                MenuRequestKind::Activate
                | MenuRequestKind::ActivateTarget { .. }
                | MenuRequestKind::Construction
                | MenuRequestKind::Get { .. }
                | MenuRequestKind::Context { .. }
                | MenuRequestKind::Buy { .. }
                | MenuRequestKind::Sell { .. }
                | MenuRequestKind::Contents { .. }
                | MenuRequestKind::Info { .. } => {}
            }
        }
        Ok(())
    }

    pub(crate) fn dismiss_game_over_dialog(&mut self) {
        if self.game_over_dialog.take().is_some() {
            self.hide_runtime_default_dialog(RuntimeDefaultDialog::GameOver);
            self.play_ui_sound("DoorClose");
            self.pointer_left_unchecked();
        }
    }

    pub(crate) fn push_league_end_error_dialog(
        &mut self,
        message: String,
        buttons: clonk_frontend::message_dialog::MessageDialogButtons,
        continuation: MessageDialogContinuation,
    ) -> Result<(), EngineError> {
        let caption = self.runtime_resource_text("IDS_NET_ERR_LEAGUE", "League error");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                buttons,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            continuation,
        )
    }

    pub(crate) fn open_live_masterserver_signup_dialog(
        &mut self,
        server_name: &str,
    ) -> Result<(), EngineError> {
        let message = self
            .runtime_resource_text("IDS_NET_LEAGUE_REGGAME", "Registering game at %s...")
            .replacen("%s", server_name, 1);
        let caption = self.runtime_resource_text("IDS_NET_LEAGUE_STARTGAME", "Starting game...");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(3),
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::LiveMasterserverSignup,
        )?;
        let abort = self.runtime_resource_text("IDS_DLG_ABORT", "Abort");
        if let Some(dialog) = self.message_dialogs.last_mut() {
            dialog.state.set_button_label(
                clonk_frontend::message_dialog::MessageDialogButton::Cancel,
                abort,
            );
        }
        Ok(())
    }

    pub(crate) fn close_live_masterserver_signup_dialog(&mut self) -> Result<(), EngineError> {
        let Some(index) = self.message_dialogs.iter().position(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::LiveMasterserverSignup
            )
        }) else {
            return Ok(());
        };
        self.finish_message_dialog_at(
            index,
            clonk_frontend::message_dialog::MessageDialogResult::Dismissed,
        )
    }

    pub(crate) fn push_message_dialog(
        &mut self,
        mut state: clonk_frontend::message_dialog::MessageDialogState,
        continuation: MessageDialogContinuation,
    ) -> Result<(), EngineError> {
        use clonk_frontend::message_dialog::MessageDialogButton;

        for (button, key, fallback) in [
            (MessageDialogButton::Ok, "IDS_DLG_OK", "&OK"),
            (MessageDialogButton::Retry, "IDS_BTN_RETRY", "Retry"),
            (MessageDialogButton::Cancel, "IDS_DLG_CANCEL", "Cancel"),
            (MessageDialogButton::Yes, "IDS_DLG_YES", "&Yes"),
            (MessageDialogButton::Restart, "IDS_BTN_RESTART", "&Restart"),
            (MessageDialogButton::No, "IDS_DLG_NO", "&No"),
        ] {
            state.set_button_label(button, self.runtime_resource_text(key, fallback));
        }
        if matches!(
            continuation,
            MessageDialogContinuation::StartupIrcDisconnectConfirm
                | MessageDialogContinuation::LeaguePlayerAuthWait
                | MessageDialogContinuation::LeaguePlayerAuthWelcome
                | MessageDialogContinuation::LeagueEndRetry
                | MessageDialogContinuation::LeagueEndRejected
                // `C4UpdateDlg.cpp:277` opens the wait with btnAbort.
                | MessageDialogContinuation::UpdateCheckWait
                | MessageDialogContinuation::UpdateDownloadWait
        ) {
            state.set_button_label(
                MessageDialogButton::Cancel,
                self.runtime_resource_text("IDS_DLG_ABORT", "Abort"),
            );
        }
        state.set_close_tooltip(self.runtime_resource_text("IDS_MNU_CLOSE", "Close"));
        state.set_progress_tooltip(
            self.runtime_resource_text("IDS_DLGTIP_PROGRESS", "Progress bar"),
        );
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "C4GUI::MessageDialog",
            self.assets
                .message_dialog_resources()
                .context("exact C4GUI::MessageDialog resource set is absent")
                .and_then(|resources| resources.validate()),
        )?;
        let chat_above = self.running_chat_controller().is_some();
        let chart_above = self.network_chart_elevated;
        let chart_stays_above = chart_above && chat_above;
        if chart_above && !chart_stays_above {
            self.network_chart_elevated = false;
        }
        if !chat_above && !chart_stays_above && self.mode != AppMode::Running {
            self.close_context_menu_silently();
        }
        if self.message_dialogs.is_empty()
            && !chat_above
            && !chart_stays_above
            && self.mode != AppMode::Running
        {
            // Release the underlying screen's hover/drag capture before
            // the C4GUI input-z dialog takes over. Chat has z=+2, so a
            // newly inserted default-z message remains underneath it.
            self.cancel_underlying_interaction();
        }
        if !chat_above && !chart_stays_above && self.mode != AppMode::Running {
            self.pressed_engine_keys.clear();
        }
        let running_stack_id = self.next_running_message_stack_id;
        self.next_running_message_stack_id = self.next_running_message_stack_id.wrapping_add(1);
        self.message_dialogs.push(PendingMessageDialog {
            running_stack_id,
            state,
            continuation,
        });
        self.show_running_dialog(RunningDialogStackEntry::Message(running_stack_id));
        if !chat_above && !chart_stays_above {
            self.message_dialog_active_index = self.message_dialogs.len().checked_sub(1);
        }
        Ok(())
    }

    pub(crate) fn persist_top_message_dialog_checkbox_changes(&mut self) {
        let Some(index) = self.message_dialogs.len().checked_sub(1) else {
            return;
        };
        self.persist_message_dialog_checkbox_changes(index);
    }

    pub(crate) fn persist_message_dialog_checkbox_changes(&mut self, index: usize) {
        let Some((key, description, native_irc_preference, changes)) =
            self.message_dialogs.get_mut(index).and_then(|dialog| {
                let (key, description, native_irc_preference) = match &dialog.continuation {
                    MessageDialogContinuation::ClassicLobbyStart { .. } => (
                        "HideMsgPlrNoTakeOver",
                        "unassociated savegame-player warning preference",
                        false,
                    ),
                    MessageDialogContinuation::NetworkScenarioPlayerCountWarning { .. } => (
                        "HideMsgStartDedicated",
                        "scenario-start warning preference",
                        false,
                    ),
                    MessageDialogContinuation::StartupIrcConnectWarning { .. } => {
                        ("HideMsgIRCDangerous", "IRC disclaimer preference", true)
                    }
                    MessageDialogContinuation::SavegamePlayerTakeoverWarning => (
                        "HideMsgPlrTakeOver",
                        "savegame player-takeover warning preference",
                        false,
                    ),
                    _ => return None,
                };
                Some((
                    key,
                    description,
                    native_irc_preference,
                    dialog.state.take_checkbox_changes(),
                ))
            })
        else {
            return;
        };
        for checked in changes {
            // `ShowMessageModal` takes the `Config.Startup.HideMsg*` flag by
            // pointer and writes it in memory; no call site saves, and neither
            // `C4Gui.cpp`, `C4GuiDialogs.cpp` nor `C4ChatDlg.cpp` contains a
            // `Config.Save()` (e.g. C4ChatDlg.cpp:624). The file is written at
            // the next save surface.
            if native_irc_preference {
                let Some(paths) = self.app_paths.as_ref() else {
                    continue;
                };
                if let Err(error) = persist_irc_warning_preference(paths, checked) {
                    tracing::warn!(%error, preference = description, "failed to persist warning preference");
                }
            } else {
                self.deferred_config
                    .set("Startup", key, i32::from(checked).to_string());
            }
        }
    }

    pub(crate) fn finish_message_dialog(
        &mut self,
        result: clonk_frontend::message_dialog::MessageDialogResult,
    ) -> Result<(), EngineError> {
        let Some(index) = self.message_dialogs.len().checked_sub(1) else {
            return Ok(());
        };
        self.finish_message_dialog_at(index, result)
    }

    pub(crate) fn remove_message_dialog_at(
        &mut self,
        index: usize,
    ) -> Option<(PendingMessageDialog, bool)> {
        if index >= self.message_dialogs.len() {
            return None;
        }
        let removed_entry =
            RunningDialogStackEntry::Message(self.message_dialogs[index].running_stack_id);
        let was_active = if self.mode == AppMode::Running {
            !self.network_chart_elevated_owns_input()
                && self.running_active_dialog == Some(removed_entry)
        } else {
            self.message_dialog_active_index == Some(index)
        };
        if was_active {
            self.release_message_dialog_pointer_elements();
            self.release_game_option_input_pointer_elements();
        }
        let releases_abort_halt = matches!(
            &self.message_dialogs[index].continuation,
            MessageDialogContinuation::AbortGame {
                halted_offline: true
            }
        );
        if releases_abort_halt {
            debug_assert!(self.offline_halt_count > 0);
            self.offline_halt_count -= 1;
        }
        let pending = self.message_dialogs.remove(index);
        self.remove_running_dialog(removed_entry);
        self.message_dialog_active_index = match self.message_dialog_active_index {
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => None,
            active => active,
        };
        self.message_dialog_pointer_capture_index = match self.message_dialog_pointer_capture_index
        {
            Some(captured) if captured > index => Some(captured - 1),
            Some(captured) if captured == index => None,
            captured => captured,
        };
        if was_active {
            if self.mode == AppMode::Running {
                if self.network_chart_elevated {
                    self.message_dialog_active_index = None;
                    if let Some(chat) = self.running_chat.as_mut() {
                        chat.active = false;
                    }
                } else {
                    self.message_dialog_active_index = match self.running_active_dialog {
                        Some(RunningDialogStackEntry::Message(stack_id)) => {
                            self.running_message_index(stack_id)
                        }
                        _ => None,
                    };
                    if self.running_active_dialog == Some(RunningDialogStackEntry::Chat) {
                        if let Some(chat) = self.running_chat.as_mut() {
                            chat.active = true;
                        }
                    }
                }
            } else if self.running_chat.is_some() {
                self.set_running_chat_active(true);
            } else {
                self.message_dialog_active_index = self.message_dialogs.len().checked_sub(1);
            }
        }
        if self.message_dialogs.is_empty() && self.running_chat_controller().is_none() {
            self.network_chart_elevated = false;
        }
        Some((pending, was_active))
    }

    pub(crate) fn finish_message_dialog_at(
        &mut self,
        index: usize,
        result: clonk_frontend::message_dialog::MessageDialogResult,
    ) -> Result<(), EngineError> {
        let Some((pending, _)) = self.remove_message_dialog_at(index) else {
            return Ok(());
        };
        self.startup_tooltip.pointer_left();
        let checkbox_checked = pending.state.checkbox_checked();
        match pending.continuation {
            MessageDialogContinuation::None => {}
            MessageDialogContinuation::AbortGame { .. } => match result {
                clonk_frontend::message_dialog::MessageDialogResult::Yes => {
                    self.restart_restore_infos = RestartRestoreInfos::default();
                    self.route_abort_confirmation()?;
                }
                clonk_frontend::message_dialog::MessageDialogResult::Restart => {
                    self.retain_restart_restore_mask_for_restart();
                    self.abort_restart_pending = true;
                    self.route_abort_confirmation()?;
                }
                clonk_frontend::message_dialog::MessageDialogResult::Ok
                | clonk_frontend::message_dialog::MessageDialogResult::Retry
                | clonk_frontend::message_dialog::MessageDialogResult::Cancel
                | clonk_frontend::message_dialog::MessageDialogResult::No
                | clonk_frontend::message_dialog::MessageDialogResult::Dismissed => {
                    self.clear_local_controls()?;
                }
            },
            MessageDialogContinuation::DeveloperConsoleNotice { follow_up } => {
                if let Some(message) = follow_up {
                    self.show_developer_console_message(message, None)?;
                }
            }
            // The warning is informational: `RestoreSavegameInfos` has already
            // made the assignment before showing it (C4PlayerInfo.cpp:1383-1390).
            MessageDialogContinuation::SavegamePlayerTakeoverWarning => {}
            MessageDialogContinuation::StartupNetworkConnectProgress => {
                if self
                    .startup_network_connection
                    .as_ref()
                    .is_some_and(|connection| connection.purpose == StartupNetworkPurpose::Join)
                {
                    // The attempt owns its cancellation signal and launcher.
                    // Drop it before restoring NetDlg so its receiver closes,
                    // every pending transport is interrupted, and both the
                    // network worker and launcher have joined synchronously.
                    let connection = self.startup_network_connection.take();
                    drop(connection);
                    self.pending_network_join = None;
                    // Cancelling a reconnect to a restarting host abandons the
                    // whole rejoin. Without this the retry loop would reopen
                    // this very dialog on the next frame and Cancel would do
                    // nothing until the window expired.
                    self.pending_host_rejoin = None;
                    self.status_text.clear();
                    self.resume_startup_music_after_failed_open_game();
                    self.restore_startup_game_search();
                }
            }
            MessageDialogContinuation::StartupIrcConnectWarning { login } => {
                if let (Some(paths), Some(checked)) = (self.app_paths.as_ref(), checkbox_checked) {
                    if let Err(error) = persist_irc_warning_preference(paths, checked) {
                        tracing::warn!(%error, "failed to persist IRC disclaimer preference");
                    }
                }
                if result == clonk_frontend::message_dialog::MessageDialogResult::Ok {
                    self.connect_startup_irc(login)?;
                }
            }
            MessageDialogContinuation::StartupIrcDisconnectConfirm => {
                if result == clonk_frontend::message_dialog::MessageDialogResult::Ok {
                    self.disconnect_startup_irc();
                    self.show_irc_login_on_all_controllers();
                }
            }
            MessageDialogContinuation::NetworkClientStartWait => {
                self.return_to_menu();
            }
            MessageDialogContinuation::BlockingResourceWait { scope, resource_id } => {
                self.cancel_blocking_resource_wait(scope, resource_id)?;
            }
            MessageDialogContinuation::NetworkRuntimeJoin { reference }
                if result == clonk_frontend::message_dialog::MessageDialogResult::Yes =>
            {
                self.activate_network_reference_join(reference)?;
            }
            MessageDialogContinuation::NetworkRuntimeJoin { .. } => {}
            MessageDialogContinuation::NetworkServerRedirect { address }
                if result == clonk_frontend::message_dialog::MessageDialogResult::Yes =>
            {
                // C4StartupNetDlg writes Config.Network.ServerAddress and
                // immediately calls Config.Save (`C4StartupNetDlg.cpp:312-315`).
                // Carry the in-memory Display fields into that complete save
                // before the deferred shutdown flush gets a chance to run.
                let persisted =
                    self.persist_config_value_with_display("Network", "ServerAddress", address);
                match persisted {
                    Ok(()) => {
                        let message = self.runtime_resource_text(
                            "IDS_NET_SERVERREDIRECTDONE",
                            "Server redirection has been applied.",
                        );
                        let caption = self
                            .runtime_resource_text("IDS_NET_SERVERREDIRECT", "Server Redirection");
                        self.push_message_dialog(
                            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                                message,
                                caption,
                                clonk_frontend::message_dialog::MessageDialogIcon::Standard(44),
                            ),
                            MessageDialogContinuation::None,
                        )?;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to persist masterserver redirect");
                        self.status_text = format!("Unable to save server redirection: {error}");
                    }
                }
            }
            MessageDialogContinuation::NetworkServerRedirect { .. } => {
                self.startup_network_ignore_redirect = true;
            }
            MessageDialogContinuation::ClassicLobbyStart { countdown_seconds } => {
                // `C4GameLobby` hands `Config.Startup.HideMsgPlrNoTakeOver` to
                // `ShowMessageModal` by pointer (C4GameLobby.cpp:462), which
                // writes it in memory; that file contains no `Config.Save()`.
                if let Some(checked) = checkbox_checked {
                    self.deferred_config.set(
                        "Startup",
                        "HideMsgPlrNoTakeOver",
                        i32::from(checked).to_string(),
                    );
                }
                if result == clonk_frontend::message_dialog::MessageDialogResult::Yes
                    && self.classic_host_lobby_active()
                {
                    self.start_network_lobby_countdown_with(countdown_seconds)?;
                }
            }
            MessageDialogContinuation::LobbyResourceOverwrite { resource_id }
                if result == clonk_frontend::message_dialog::MessageDialogResult::Yes =>
            {
                self.request_lobby_resource_save(resource_id, true)?;
            }
            MessageDialogContinuation::LobbyResourceOverwrite { .. } => {}
            MessageDialogContinuation::NetworkScenarioPlayerCountWarning { scenario } => {
                // `C4StartupScenSelDlg` likewise passes
                // `Config.Startup.HideMsgStartDedicated` to `ShowMessageModal`
                // by pointer (C4StartupScenSelDlg.cpp:1697) and never saves.
                if let Some(checked) = checkbox_checked {
                    self.deferred_config.set(
                        "Startup",
                        "HideMsgStartDedicated",
                        i32::from(checked).to_string(),
                    );
                }
                if result == clonk_frontend::message_dialog::MessageDialogResult::Ok {
                    self.continue_scenario_from_selector(scenario)?;
                }
            }
            MessageDialogContinuation::LobbyReadyCheck { .. } => {
                self.complete_lobby_ready_check_response(
                    result == clonk_frontend::message_dialog::MessageDialogResult::Yes,
                )?;
            }
            MessageDialogContinuation::LiveMasterserverSignup => {
                self.abort_live_masterserver_signup();
            }
            MessageDialogContinuation::LeaguePlayerAuthWait => {
                if let Some(mut pending) = self.pending_league_player_auth.take() {
                    Self::reject_league_auth_continuation_player(&mut pending.continuation);
                    let _ = self.continue_league_player_auth(pending.continuation)?;
                }
            }
            MessageDialogContinuation::LeaguePlayerAuthWelcome => {
                if result == clonk_frontend::message_dialog::MessageDialogResult::Ok {
                    if let Some(mut pending) = self.pending_league_player_auth.take() {
                        Self::advance_league_auth_continuation(&mut pending.continuation);
                        let _ = self.continue_league_player_auth(pending.continuation)?;
                    }
                } else if let Some(pending) = self.pending_league_player_auth.as_ref() {
                    let player = Self::league_auth_continuation_player_name(&pending.continuation);
                    let message = format_resource_string(
                        self.runtime_resource_text(
                            "IDS_MSG_LEAGUESIGNUPCANCELLED",
                            "League login for player %s cancelled. Without login this player can not take part in this round!",
                        ),
                        &[&player],
                    );
                    self.push_message_dialog(
                        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                            message,
                            self.runtime_resource_text("IDS_DLG_LEAGUESIGNUP", "League Login"),
                            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                        ),
                        MessageDialogContinuation::LeaguePlayerAuthCancelled,
                    )?;
                }
            }
            MessageDialogContinuation::LeaguePlayerAuthError => {
                if let Some(pending) = self.pending_league_player_auth.take() {
                    self.reopen_league_player_auth_form(pending)?;
                }
            }
            MessageDialogContinuation::LeaguePlayerAuthCancelled => {
                if let Some(pending) = self.pending_league_player_auth.take() {
                    self.reopen_league_player_auth_form(pending)?;
                }
            }
            MessageDialogContinuation::LeagueEndRetry => {
                let retry = result == clonk_frontend::message_dialog::MessageDialogResult::Retry
                    && self
                        .pending_league_end
                        .as_ref()
                        .is_some_and(|pending| pending.attempts < LEAGUE_END_MAX_ATTEMPTS);
                if retry {
                    self.run_pending_league_end_attempt()?;
                } else {
                    self.finalize_pending_league_end_failure()?;
                }
            }
            MessageDialogContinuation::LeagueEndRejected => {
                self.finish_pending_league_end_terminal()?;
            }
            MessageDialogContinuation::LeagueStartRefused { message } => {
                // `*pCancel = !fResult`: only Abort fails InitHost, and every
                // other answer falls through to `NetIO.SetAcceptMode(true)`
                // (src/C4Network2.cpp:265-274,2379-2384). An aborted host still
                // unwinds through QuitGame carrying the message LeagueStart
                // already logged.
                if result != clonk_frontend::message_dialog::MessageDialogResult::Ok {
                    self.finish_startup_network_failure(
                        StartupNetworkPurpose::StagedHost,
                        message,
                    )?;
                }
            }
            MessageDialogContinuation::LeagueSignupCancelled => {
                if let Some(mut continuation) = self.cancelled_league_signup_continuation.take() {
                    Self::reject_league_auth_continuation_player(&mut continuation);
                    let _ = self.continue_league_player_auth(continuation)?;
                }
            }
            MessageDialogContinuation::LeagueVote { subject } => {
                self.complete_league_vote_response(
                    subject,
                    result == clonk_frontend::message_dialog::MessageDialogResult::Yes,
                );
            }
            MessageDialogContinuation::LeagueSurrender
                if result == clonk_frontend::message_dialog::MessageDialogResult::Yes =>
            {
                self.record_league_surrender_round_result();
                #[cfg(test)]
                {
                    self.league_surrender_pre_abort_results = Some((
                        self.engine.snapshot().round_results,
                        self.snapshot.round_results.clone(),
                        self.network.is_some(),
                    ));
                }
                if let Some(local_client_id) = self
                    .network
                    .as_ref()
                    .and_then(|network| i32::try_from(network.local_client_id()).ok())
                {
                    self.change_network_control_to_local(local_client_id);
                }
                self.return_to_menu();
            }
            MessageDialogContinuation::LeagueSurrender => {}
            MessageDialogContinuation::DeleteStartupPlayer { path }
                if result == clonk_frontend::message_dialog::MessageDialogResult::Yes =>
            {
                self.delete_startup_player_and_refresh(&path)?;
            }
            MessageDialogContinuation::DeleteStartupPlayer { .. } => {}
            MessageDialogContinuation::DeleteStartupCrew {
                player_path,
                file_name,
            } if result == clonk_frontend::message_dialog::MessageDialogResult::Yes => {
                self.delete_startup_crew_and_refresh(&player_path, &file_name)?;
            }
            MessageDialogContinuation::DeleteStartupCrew { .. } => {}
            MessageDialogContinuation::DeleteScenario {
                path,
                next_identifier,
            } if result == clonk_frontend::message_dialog::MessageDialogResult::Yes => {
                self.delete_scenario_and_refresh(&path, next_identifier.as_deref())?;
            }
            MessageDialogContinuation::DeleteScenario { .. } => {}
            MessageDialogContinuation::OptionsScaleTest {
                old_percent,
                new_percent,
                ..
            } => {
                let accepted = result == clonk_frontend::message_dialog::MessageDialogResult::Yes;
                if let Some(dialog) = self.startup_options_dialog.as_mut() {
                    if accepted {
                        dialog.graphics_mut().commit_scale_test();
                    } else {
                        dialog.graphics_mut().revert_scale_test();
                    }
                }
                self.queue_options_display_request(OptionsDisplayRequest::SetScale {
                    percent: if accepted { new_percent } else { old_percent },
                    persist: accepted,
                });
            }
            // Both capture modals apply their binding from the key event
            // itself; closing them has nothing left to do.
            MessageDialogContinuation::OptionsControlCapture(_)
            | MessageDialogContinuation::OptionsVoicePushToTalkCapture => {}
            MessageDialogContinuation::OptionsAlternateServerNotice => {
                if checkbox_checked == Some(true) {
                    if let Some(dialog) = self.startup_options_dialog.as_mut() {
                        dialog.network_mut().hide_no_official_league_notice = true;
                    }
                }
            }
            MessageDialogContinuation::OptionsResetConfiguration
                if result == clonk_frontend::message_dialog::MessageDialogResult::Yes =>
            {
                if let Some(paths) = self.app_paths.as_ref() {
                    let path = paths.config_file();
                    let reset = path
                        .parent()
                        .map_or(Ok(()), fs::create_dir_all)
                        .and_then(|()| Config::new().save(&path));
                    if let Err(error) = reset {
                        tracing::warn!(%error, path = %path.display(), "failed to save reset configuration");
                    }
                }
                self.pending_options_display_requests.clear();
                self.configuration_reset_requested = true;
                self.request_exit("the configuration was reset");
            }
            MessageDialogContinuation::OptionsResetConfiguration => {}
            MessageDialogContinuation::OptionsAdvancedWarning
                if result == clonk_frontend::message_dialog::MessageDialogResult::Ok =>
            {
                self.open_options_advanced_dialog()?;
            }
            MessageDialogContinuation::OptionsAdvancedWarning => {}
            // Closing the wait dialog by any route is C++'s abort
            // (`C4UpdateDlg.cpp:294-296`), which reports nothing.
            MessageDialogContinuation::UpdateCheckWait => self.abort_update_check(),
            MessageDialogContinuation::UpdatePrompt {
                manifest_base_url,
                version,
                components,
            } if result == clonk_frontend::message_dialog::MessageDialogResult::Yes => {
                // C++ downloads and applies here (`C4UpdateDlg.cpp:386-394`).
                self.start_update_download(manifest_base_url, version, components)?;
            }
            MessageDialogContinuation::UpdatePrompt { .. } => {}
            MessageDialogContinuation::UpdateDownloadWait => self.abort_update_download(),
            MessageDialogContinuation::UpdateNotice => {}
        }
        Ok(())
    }

    pub(crate) fn message_dialog_layout_at(
        &self,
        index: usize,
    ) -> Option<clonk_frontend::message_dialog::MessageDialogLayout> {
        let dialog = self.message_dialogs.get(index)?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.graphics.surface();
        Some(
            dialog
                .state
                .layout(surface.width() as i32, surface.height() as i32, &fonts.text),
        )
    }

    pub(crate) fn top_message_dialog_layout(
        &self,
    ) -> Option<clonk_frontend::message_dialog::MessageDialogLayout> {
        self.message_dialogs
            .len()
            .checked_sub(1)
            .and_then(|index| self.message_dialog_layout_at(index))
    }

    pub(crate) fn top_message_dialog_hit_index(&self, point: GuiPoint) -> Option<usize> {
        (0..self.message_dialogs.len()).rev().find(|index| {
            self.message_dialog_layout_at(*index)
                .is_some_and(|layout| Self::point_in_message_dialog_bounds(point, &layout))
        })
    }

    pub(crate) fn top_message_dialog_is_exclusive(&self) -> bool {
        self.message_dialogs.last().is_some_and(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::AbortGame { .. }
                    | MessageDialogContinuation::LeagueVote { .. }
                    | MessageDialogContinuation::LeagueSurrender
            )
        })
    }

    pub(crate) fn active_message_dialog_index(&self) -> Option<usize> {
        if self.mode == AppMode::Running {
            if self.network_chart_elevated_owns_input() {
                return None;
            }
            let RunningDialogStackEntry::Message(stack_id) = self.running_active_dialog? else {
                return None;
            };
            return self.running_message_index(stack_id);
        }
        self.message_dialog_active_index
            .filter(|index| *index < self.message_dialogs.len())
    }

    pub(crate) fn captured_message_dialog_index(&self) -> Option<usize> {
        self.message_dialog_pointer_capture_index.filter(|index| {
            self.message_dialogs
                .get(*index)
                .is_some_and(|dialog| dialog.state.has_pointer_capture())
        })
    }

    pub(crate) fn point_in_message_dialog_bounds(
        point: GuiPoint,
        layout: &clonk_frontend::message_dialog::MessageDialogLayout,
    ) -> bool {
        let bounds = layout.bounds;
        point.x >= bounds.x as f32
            && point.x < (bounds.x + bounds.w) as f32
            && point.y >= bounds.y as f32
            && point.y < (bounds.y + bounds.h) as f32
    }

    /// Returns the topmost ordinary `C4GUI::Dialog::SetTitle` tooltip target.
    /// The process-global tracker owns the 500ms delay; the individual dialog
    /// controllers only own their current title/close hit geometry.
    pub(crate) fn classic_dialog_title_tooltip_target_at(
        &self,
        point: GuiPoint,
    ) -> Option<StartupTooltip> {
        if self.context_menu.is_some() {
            return None;
        }
        let scoreboard_owns_point = |app: &Self| {
            app.scoreboard_tooltip_target_cached(point).or_else(|| {
                app.scoreboard_pointer_target_cached(point)
                    .map(|_| StartupTooltip::text(String::new()))
            })
        };
        let top_default_target = self
            .runtime_default_dialog_order_snapshot()
            .into_iter()
            .rev()
            .find(|dialog| match dialog {
                RuntimeDefaultDialog::Scoreboard => {
                    self.scoreboard_pointer_target_cached(point).is_some()
                }
                RuntimeDefaultDialog::NetworkChart => self.network_chart_contains_point(point),
                RuntimeDefaultDialog::ClientList => self.runtime_client_list_contains_point(point),
                RuntimeDefaultDialog::GameOver => self.game_over_dialog_contains_point(point),
                RuntimeDefaultDialog::ExternalIrc => self.external_irc_dialog_contains_point(point),
            });
        if matches!(
            top_default_target,
            Some(RuntimeDefaultDialog::NetworkChart | RuntimeDefaultDialog::ExternalIrc)
        ) {
            return None;
        }
        if self.mode == AppMode::Running
            && self.game_over_dialog.is_none()
            && !self.running_dialog_stack.is_empty()
        {
            return match self.top_scoreboard_message_pointer_target_cached(point) {
                Some(RunningDialogStackEntry::Scoreboard) => {
                    self.scoreboard_tooltip_target_cached(point)
                }
                Some(RunningDialogStackEntry::RuntimeClientList) => {
                    let dialog = self.runtime_client_list.as_ref()?;
                    let line_height = self.assets.clonk_fonts.as_deref()?.text.line_height;
                    let preferred = scoreboard_preferred_rect(
                        self.graphics
                            .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
                    );
                    dialog.tooltip_at(point, preferred, line_height)
                }
                Some(RunningDialogStackEntry::Message(_))
                | Some(RunningDialogStackEntry::Chat)
                | None => None,
            };
        }

        // The lobby's standalone client-info window is composited after the
        // standalone IRC window, while the running F4 list is below it.
        if self.mode != AppMode::Running {
            if let Some(dialog) = self
                .runtime_client_list
                .as_ref()
                .filter(|dialog| dialog.is_info_only())
            {
                let line_height = self.assets.clonk_fonts.as_deref()?.text.line_height;
                let preferred = scoreboard_preferred_rect(
                    self.graphics
                        .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
                );
                return dialog.tooltip_at(point, preferred, line_height);
            }
        }
        let scoreboard_above_chat = self.running_chat_controller().is_some()
            && self.running_dialog_is_above(
                RunningDialogStackEntry::Scoreboard,
                RunningDialogStackEntry::Chat,
            );
        if scoreboard_above_chat {
            if let Some(target) = scoreboard_owns_point(self) {
                return match target {
                    StartupTooltip::Text(text) if text.is_empty() => None,
                    target => Some(target),
                };
            }
        }
        if self.external_irc_dialog_visible || self.game_option_input_dialog.is_some() {
            return None;
        }
        if self.mode == AppMode::Running {
            if !self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ClientList) {
                return None;
            }
            let scoreboard_above_client = self.runtime_client_list.is_none()
                || self.scoreboard_is_above_runtime_client_list();
            if scoreboard_above_client {
                if let Some(target) = scoreboard_owns_point(self) {
                    return match target {
                        StartupTooltip::Text(text) if text.is_empty() => None,
                        target => Some(target),
                    };
                }
            }
            if let Some(dialog) = self.runtime_client_list.as_ref() {
                let line_height = self.assets.clonk_fonts.as_deref()?.text.line_height;
                let preferred = scoreboard_preferred_rect(
                    self.graphics
                        .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
                );
                if let Some(target) = dialog.tooltip_at(point, preferred, line_height) {
                    return Some(target);
                }
            }
            if !scoreboard_above_client {
                return self.scoreboard_tooltip_target_cached(point);
            }
        }
        if self.external_irc_dialog_visible {
            return None;
        }
        if let Some(controller) = self.definition_selector.as_ref() {
            let layout = self.definition_selector_layout()?;
            return controller.tooltip_at(point, &layout);
        }
        self.startup_options_advanced_dialog
            .as_ref()
            .and_then(|pending| pending.controller.tooltip_at(point))
    }

    /// Whether an app-owned `C4MainMenu` may use its owning viewport.
    ///
    /// An eliminated player still receives the local `COM_PlayerMenu` path:
    /// C++ draws the eliminated notice, retains the PlayerMenu control, and
    /// lets it activate the capacity-gated New Player page
    /// (src/C4Viewport.cpp:836-880,965-976,1511-1525;
    /// src/C4MouseControl.cpp:1056-1064; src/C4MainMenu.cpp:643-687).
    /// Keep that one UI surface available without relaxing the eliminated
    /// viewport suppression used for script/object menus or world controls.
    pub(crate) fn ingame_menu_owner_has_visible_surface(&self, menu_owner: i32) -> bool {
        if self.menu_owner_has_unsuppressed_viewport(menu_owner) {
            return true;
        }
        menu_owner != OWNER_NONE
            && self.ingame_menu.contains(menu_owner)
            && self
                .snapshot
                .players
                .iter()
                .any(|player| player.id == menu_owner)
            && self
                .physical_viewports
                .iter()
                .any(|viewport| viewport.displayed_player == menu_owner)
    }

    pub(crate) fn ingame_menu_has_visible_surface(&self, menu_owner: i32) -> bool {
        if self.ingame_menu_owner_has_visible_surface(menu_owner) {
            return true;
        }
        // Preserve the generic whole-surface compatibility path only for an
        // unresolved synthetic menu key. Real C4Player menus require a
        // physical viewport that currently displays their player.
        menu_owner != OWNER_NONE
            && !self
                .snapshot
                .players
                .iter()
                .any(|player| player.id == menu_owner)
            && self
                .physical_viewports
                .iter()
                .copied()
                .any(|viewport| self.physical_viewport_is_unsuppressed(viewport))
    }

    pub(crate) fn return_to_menu(&mut self) {
        if std::mem::take(&mut self.abort_restart_pending) {
            if let Err(error) = self.restart_current_scenario() {
                tracing::error!(%error, "failed to consume scheduled abort-dialog restart");
            }
            return;
        }
        self.restart_restore_infos = RestartRestoreInfos::default();
        self.return_to_menu_with_dialog_restore(true, NetworkSessionTeardown::Clear);
    }

    pub(crate) fn return_to_menu_for_relaunch(&mut self) {
        self.return_to_menu_with_dialog_restore(false, NetworkSessionTeardown::Clear);
    }

    /// Tears the round down while the network session it ran on stays up.
    ///
    /// Only for a host restart that keeps every client connected
    /// (`clonk_network::host_restart`); the caller is responsible for putting
    /// the retained session into a lobby immediately afterwards, because this
    /// leaves it attached to a round that no longer exists.
    pub(crate) fn return_to_menu_retaining_network_session(&mut self) {
        self.return_to_menu_with_dialog_restore(false, NetworkSessionTeardown::Retain);
    }

    fn return_to_menu_with_dialog_restore(
        &mut self,
        restore_dialog: bool,
        session: NetworkSessionTeardown,
    ) {
        // The save itself is already durable. If teardown wins the screenshot
        // readback race, discard its guarded thumbnail update so a later round
        // can never mutate this save.
        self.finish_pending_native_save_thumbnails(None);
        let last_startup_dialog = self.last_startup_dialog;
        self.abort_restart_pending = false;
        self.finalize_pending_league_end_for_teardown();
        self.clear_lobby_preload();
        self.restart_restore_roster_items.clear();
        // Leaving the round abandons any host restart this client was going to
        // follow. `begin_pending_host_rejoin` re-arms it across this teardown
        // precisely because the default is to drop it.
        self.pending_host_rejoin = None;
        // C4Game::Clear starts the fade before tearing down game state.
        self.fade_out_game_music();
        if let Some(audio) = self.audio.as_ref() {
            let mut audio = audio.borrow_mut();
            audio.stop_lobby_elevator();
        }
        self.active_game_graphics = None;
        self.ingame_menu_gfx = None;
        self.runtime_player_big_icons.clear();
        self.runtime_player_big_icon_misses.clear();
        self.restore_startup_gui_sheets();
        self.active_global_gui_failures.clear();
        self.close_context_menu_silently();
        self.network_start_wait = None;
        self.host_lobby_countdown = None;
        self.pending_local_lobby_countdown_echoes.clear();
        self.finish_recording();
        // C4Game::Clear ends the network session for every round it tears
        // down, after the evaluation and record work that still needs it and
        // before the rest of the game state goes (src/C4Game.cpp:544-582).
        // Leaving it to the lobby view below would strand a session torn down
        // from inside a running round. A retained session is the deliberate
        // exception: the host restarting the round is keeping every client
        // connected across it.
        if session == NetworkSessionTeardown::Clear {
            self.clear_live_network_session();
        }
        self.live_save_seed = None;
        self.recording_template = None;
        self.control_playback = None;
        self.deferred_network_savegame_recreation.clear();
        self.network_savegame_recreation_progress = None;
        self.message_dialogs.clear();
        self.message_dialog_active_index = None;
        self.message_dialog_pointer_capture_index = None;
        self.league_signup_dialog = None;
        self.cancelled_league_signup_continuation = None;
        self.league_signup_consumed_keys.clear();
        self.league_signup_pointer_capture = false;
        self.league_signup_pointer_position = None;
        self.primary_pointer_left_down = false;
        self.message_dialog_consumed_keys.clear();
        self.definition_selector = None;
        self.pending_definition_selection = None;
        self.pending_lobby_player_selection = None;
        self.definition_selector_last_click = None;
        self.definition_selector_consumed_keys.clear();
        self.definition_selector_pointer_capture = false;
        self.close_ingame_menu();
        self.object_menu = None;
        self.script_menu_presentations.clear();
        self.game_over_dialog = None;
        self.pending_league_end = None;
        self.pending_league_player_auth = None;
        self.runtime_help_visible = false;
        self.ingame_mouse_help = false;
        self.ingame_mouse_help_caption = None;
        self.runtime_flash_message = None;
        self.film_view_player = None;
        self.clear_physical_viewport_states();
        self.physical_viewports_authoritative = false;
        self.runtime_client_list = None;
        self.running_dialog_stack.clear();
        self.running_active_dialog = None;
        self.runtime_client_list_consumed_keys.clear();
        self.runtime_client_list_above_game_over = false;
        self.scoreboard_dialog = None;
        self.scoreboard_initial_reconcile_pending = false;
        self.scoreboard_close_pointer_capture = false;
        self.scoreboard_runtime = ScoreboardDialogRuntime::default();
        self.network_stats = None;
        self.network_stats_clients.clear();
        self.network_stats_players.clear();
        self.network_chart_dialog = None;
        self.network_chart_consumed_keys.clear();
        self.network_chart_pointer_capture = false;
        self.reset_runtime_default_dialog_order();
        // C4Application::QuitGame runs Game.Default and enters PreInit before
        // showing startup again. This is when a language selected in the
        // previous startup session finally replaces Game.Rank.
        self.default_rank_names = self.loaded_default_rank_names.clone();
        self.engine = Engine::new();
        reconnect_audio_context(&mut self.engine, self.audio.as_ref());
        self.engine.set_smoke_level(self.graphics_smoke_level);
        self.engine
            .set_fire_particles(self.display_flags.fire_particles);
        self.engine.set_local_players([self.local_owner]);
        self.engine
            .set_max_players(i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.pressed_engine_keys.clear();
        self.scoreboard_tab_raw_pressed = false;
        self.ingame_gui_pointer = None;
        self.ingame_pointer = None;
        self.ingame_mouse_help = false;
        self.ingame_mouse_init_centered = false;
        self.ingame_viewport_mouse = None;
        self.ingame_edge_scroll = None;
        self.ingame_mouse_caption = IngameMouseCaptionState::default();
        self.ingame_mouse_target = None;
        self.running_pointer_position = None;
        self.mouse_state = None;
        self.ingame_right_mouse_state = None;
        self.construction_menu_drag = None;
        self.ingame_dragged_objects.clear();
        self.ingame_last_left_down = None;
        self.ingame_ignore_left_up = false;
        self.sky = None;
        self.snapshot = self.engine.snapshot();
        self.sync_checks.clear();
        self.network_ticks.clear();
        self.network_sync.clear();
        self.offline_control_input.clear();
        self.offline_halt_count = 0;
        self.network_control_running = self.network.is_none();
        self.runtime_network_status_barrier = None;
        self.league_votes.clear();
        self.frames_per_second = 0;
        self.frames_since_second = 0;
        self.presentation_stats = PresentationStats::default();
        self.script_created_objects = false;
        self.full_speed = false;
        self.frame_skip = 1;
        if session == NetworkSessionTeardown::Clear {
            self.control_clients =
                initial_control_clients(self.network.as_ref(), self.network_mode.as_ref());
        } else {
            // The retained manager still owns exactly these connections. A
            // local-only rebuild would keep their sockets while erasing the
            // peers from both sides of the lobby they are returning to.
            self.control_clients.clear_nonhost_lobby_ready();
        }
        self.network_client_activity.clear();
        if session == NetworkSessionTeardown::Clear {
            self.control_player_infos = ControlPlayerInfoRegistry::default();
            self.local_player_profile_paths.clear();
        }
        self.network_team_assignment = None;
        self.clear_blocking_resource_wait();
        if session == NetworkSessionTeardown::Clear {
            self.admission_resources.clear();
            self.host_local_alternate_colors_by_resource.clear();
            self.host_local_player_info_ids.clear();
        }
        self.pending_runtime_dynamic_request = None;
        self.pending_network_join_data = None;
        self.pending_round_restart_join_data = false;
        self.initial_lobby_status_ack_pending = false;
        self.network_is_league = false;
        self.network_league_name.clear();
        self.network_stream_address = LegacyCString::default();
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        self.client_start_barrier = ClientStartBarrier::default();
        self.pending_client_start_status = None;
        self.client_combined_scenario_path = None;
        self.client_combined_preload_file.clear();
        self.network_material_resource_groups = None;
        self.refresh_object_menu();
        self.focus_id = None;
        self.focus_snapshot = None;
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.active_scenario = None;
        self.active_definition_load = None;
        self.active_description_definition_modules.clear();
        self.loading_state = None;
        self.runtime_music_enabled = false;
        self.reconstruct_music_system_at_preinit();
        if let Some(audio) = self.audio.as_ref() {
            let mut audio = audio.borrow_mut();
            // Game.Clear has already requested its 2s fade above. On the next
            // C4AS_PreInit, MusicSystem.emplace() replaces the still-engaged
            // C4MusicSystem; its destructor performs Stop(0), so startup never
            // waits for that fade to finish. Keep the audio backend, but cancel
            // both live playback and any stale asynchronous Rust decode now.
            // The same PreInit reconstructs the process sound system.
            audio.reset_sound_system_generation();
            audio.configure_scenario(None);
        }
        self.resume_frontend_music_after_fade = false;

        self.fallback_ground = DEFAULT_GROUND_HEIGHT;
        self.scenario_label = self.menu_state.label_path();
        self.object_sprites = self.assets.base_sprite_map().clone();
        self.sprite_cache = Arc::new(self.object_sprites.clone());

        let width = self.graphics.surface().width();
        let height = self.graphics.surface().height();
        let mut graphics = GraphicsSystem::new(
            width,
            height,
            self.fallback_ground,
            &self.scenario_label,
            self.assets.font_arc(),
            Arc::clone(&self.sprite_cache),
            self.assets.cursor_atlas(),
            self.assets.hud_graphics(),
        );
        graphics.inherit_liquid_animation_cycle(&self.graphics);
        graphics.inherit_runtime_sprite_filtering(&self.graphics);
        graphics.inherit_advanced_renderer_config(&self.graphics);
        graphics.inherit_cursor_tiers(&self.graphics);
        self.graphics = graphics;
        self.graphics
            .set_clonk_fonts(self.assets.clonk_fonts.clone());
        self.graphics.set_game_palette(self.assets.game_palette());
        self.graphics
            .set_liquid_animation(self.assets.liquid_animation());
        self.graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics.set_sky(self.sky.clone());
        self.graphics
            .set_material_texture_surfaces(Arc::clone(&self.material_texture_images));
        self.graphics
            .set_material_render_info(Arc::clone(&self.material_render_info));

        self.menu_state.set_pointer_position(None);
        self.menu_state.refresh_menu_entries();
        let width_f = width as f32;
        let height_f = height as f32;
        self.menu_state.menu().resize(width_f, height_f);
        self.main_menu_state.resize(width_f, height_f);

        self.mode = AppMode::Menu;
        if session == NetworkSessionTeardown::Retain
            && self.startup_view == StartupView::NetworkLobby
        {
            // `show_main_menu` treats departure from NetworkLobby as the
            // session teardown hub. A round restart is only borrowing its
            // presentation reset; the manager and its routes are the state
            // this caller explicitly retains.
            self.startup_view = StartupView::MainMenu;
        }
        // `show_main_menu` also clears host discovery/reference state. Keep
        // the live advertiser out of that generic teardown so existing TCP
        // reference requests survive the same round boundary as game routes.
        let retained_advertiser = if session == NetworkSessionTeardown::Retain {
            self.network_game_advertiser.take()
        } else {
            None
        };
        let retained_reference = if session == NetworkSessionTeardown::Retain {
            self.advertised_game_reference.take()
        } else {
            None
        };
        let retained_reference_paused = self.host_reference_paused;
        self.show_main_menu();
        if session == NetworkSessionTeardown::Retain {
            self.network_game_advertiser = retained_advertiser;
            self.advertised_game_reference = retained_reference;
            self.host_reference_paused = retained_reference_paused;
            // `show_main_menu` normally enforces first-run profile creation.
            // This presentation reset is immediately replaced by the retained
            // network lobby, where an observer may intentionally have no
            // local player profile.
            self.startup_player_properties_dialog = None;
        }
        if restore_dialog {
            self.restore_startup_dialog(last_startup_dialog);
            self.begin_startup_dialog_fade_in();
            // C4Startup::DoStartup follows successful PreInit and requests one
            // non-looping Frontend.* song without polling the old fade.
            self.begin_frontend_music_entry();
        } else {
            // Restart/Next Mission immediately opens another game. Retain the
            // eventual startup destination without constructing a dialog,
            // starting network discovery, or playing frontend music behind
            // that new round (PreInit proceeds directly to C4AS_StartGame).
            self.last_startup_dialog = last_startup_dialog;
        }
    }
}
