//! `impl GameApp` — netplay, league & sync methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    pub(crate) fn launch_classic_command_line_join(&mut self) -> Result<(), EngineError> {
        let Some(address) = self.classic_command_line.direct_join.clone() else {
            return Ok(());
        };
        // Explicit Rust `--host`/`--join` modes keep their existing precedence.
        if self.network_mode.is_some() {
            return Ok(());
        }
        if self.startup_network_connection.is_some()
            || self.classic_direct_reference_query.is_some()
        {
            self.status_text = "A network connection is already in progress".to_string();
            return Ok(());
        }
        if let Err(error) = self.freeze_configured_client_players_for_game() {
            self.status_text = format!("Unable to load configured players: {error}");
            return Ok(());
        }
        self.prepare_network_join_game_state();
        self.startup_game_search = None;

        let (sender, receiver) = mpsc::channel();
        let player_name = self.player_name.clone();
        let app_paths = self.app_paths.clone();
        let group_maker = self
            .configured_client_player_selection
            .as_ref()
            .map(|selection| selection.group_maker().clone());
        let classic = self.classic_command_line.clone();
        let reference_config = load_reference_query_settings(app_paths.as_ref());
        let connect_target = address.clone();
        let spawn = thread::Builder::new()
            .name("lc-classic-direct-join".to_string())
            .spawn(move || {
                let result = (|| {
                    let endpoint = classic_direct_reference_endpoint(&address, app_paths.as_ref())
                        .map_err(|error| {
                            NetworkStartError::Other(format!(
                                "invalid reference-server address: {error:#}"
                            ))
                        })?;
                    let reference = query_first_classic_reference(endpoint, &reference_config)?;
                    let settings = classic_client_settings_for_reference(
                        &reference,
                        player_name,
                        app_paths.as_ref(),
                        group_maker,
                        &classic,
                    )
                    .map_err(|error| {
                        NetworkStartError::Other(format!(
                            "unable to apply direct-join settings: {error:#}"
                        ))
                    })?;
                    Ok(ClassicDirectReferenceQueryResult {
                        settings,
                        password_needed: reference.password_needed,
                    })
                })();
                let _ = sender.send(result);
            });
        match spawn {
            Ok(_) => {
                self.classic_direct_reference_query =
                    Some(ClassicDirectReferenceQuery { receiver });
                self.status_text = format!("Querying network game at {connect_target}...");
            }
            Err(error) => {
                self.status_text = format!("Unable to start reference query: {error}");
            }
        }
        Ok(())
    }

    pub(crate) fn poll_classic_direct_reference_query(&mut self) -> Result<(), EngineError> {
        // The query can finish before async boot resources. Keep its result
        // queued until boot relinquishes `Loading`; opening a password prompt
        // or an error dialog earlier would strand the boot worker forever.
        if self.boot_loading.is_some() {
            return Ok(());
        }
        let Some(query) = self.classic_direct_reference_query.as_ref() else {
            return Ok(());
        };
        let result = match query.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => Err(NetworkStartError::Other(
                "reference query worker disconnected before reporting a game".to_string(),
            )),
        };
        self.classic_direct_reference_query = None;
        self.status_text.clear();
        match result {
            Ok(result) => {
                self.pending_network_join = Some(result.settings);
                if result.password_needed && self.classic_command_line.password.is_none() {
                    self.mode = AppMode::Menu;
                    self.open_network_join_password_dialog()?;
                } else {
                    self.launch_pending_network_join()?;
                }
            }
            Err(error) => {
                self.finish_startup_network_failure(
                    StartupNetworkPurpose::Join,
                    format!("Unable to query network game: {error}"),
                )?;
            }
        }
        Ok(())
    }

    fn current_league_server_name(&self) -> String {
        if let Some(NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        })) = self.network_mode.as_ref()
        {
            return prepared
                .league_config()
                .map(|league| league_server_name(&league.endpoint))
                .unwrap_or_default();
        }
        if let Some(join_data) = self.pending_network_join_data.as_ref() {
            return league_server_name(&legacy_presentation_text(
                join_data.parameters.league_address.as_bytes(),
            ));
        }
        retained_client_league_server_name(self.network_mode.as_ref())
    }

    pub(crate) fn league_login_prompt_required(&self) -> bool {
        self.league_player_auth_settings().password.is_empty()
            || !load_league_auto_login(self.app_paths.as_ref())
    }

    pub(crate) fn league_signup_strings(
        &self,
    ) -> clonk_frontend::league_signup::LeagueSignupStrings {
        clonk_frontend::league_signup::LeagueSignupStrings {
            caption_on_server: self.runtime_resource_text(
                "IDS_DLG_LEAGUESIGNUPON",
                "League Login on %s",
            ),
            login_message: self.runtime_resource_text(
                "IDS_MSG_PASSWORDFORPLAYER",
                "League login for player %s:",
            ),
            registration_message: self.runtime_resource_text(
                "IDS_MSG_LEAGUE_REGISTRATION",
                "Player %s: This is your first login at the league. Your can specify your desired league user name and league password below.",
            ),
            account_label: self.runtime_resource_text(
                "IDS_CTL_LEAGUE_ACCOUNT",
                "League user name:",
            ),
            password_checkbox: self.runtime_resource_text(
                "IDS_CTL_LEAGUE_CHK_PLRPW",
                "Specify league password",
            ),
            password_checkbox_tooltip: self.runtime_resource_text(
                "IDS_DESC_LEAGUECHECKPASSWORD",
                "Enable to enter your own password. If you do not enter a password of your own, the personal WebCode will be used which is already stored on this system.",
            ),
            password_label: self.runtime_resource_text(
                "IDS_CTL_LEAGUE_PLRPW",
                "League password:",
            ),
            password_confirmation_label: self.runtime_resource_text(
                "IDS_CTL_LEAGUE_PLRPW2",
                "League password (repeat):",
            ),
            ok: self.runtime_resource_text("IDS_DLG_OK", "&OK"),
            cancel: self.runtime_resource_text("IDS_DLG_CANCEL", "Cancel"),
            close_tooltip: self.runtime_resource_text("IDS_MNU_CLOSE", "Close"),
            invalid_entry_caption: self
                .runtime_resource_text("IDS_DLG_INVALIDENTRY", "Invalid Entry"),
            missing_account: self.runtime_resource_text(
                "IDS_MSG_LEAGUEMISSINGUSERNAME",
                "Please enter a user name!",
            ),
            invalid_account: self.runtime_resource_text(
                "IDS_MSG_LEAGUEINVALIDUSERNAME",
                "The user name contains invalid characters.",
            ),
            account_too_short: self.runtime_resource_text(
                "IDS_MSG_LEAGUEUSERNAMETOOSHORT",
                "The user name is too short.",
            ),
            missing_password: self.runtime_resource_text(
                "IDS_MSG_LEAGUEMISSINGPASSWORD",
                "Please enter a password!",
            ),
            password_mismatch: self.runtime_resource_text(
                "IDS_MSG_LEAGUEMISMATCHPASSWORD",
                "Repeated password mismatch. Please re-enter password!",
            ),
            cancelled_caption: self
                .runtime_resource_text("IDS_DLG_LEAGUESIGNUP", "League Login"),
            cancelled_message: self.runtime_resource_text(
                "IDS_MSG_LEAGUESIGNUPCANCELLED",
                "League login for player %s cancelled. Without login this player can not take part in this round!",
            ),
        }
    }

    pub(crate) fn process_league_signup_actions(
        &mut self,
        actions: Vec<clonk_frontend::league_signup::LeagueSignupAction>,
    ) -> Result<(), EngineError> {
        use clonk_frontend::league_signup::{LeagueSignupAction, LeagueSignupMode};

        let sounds = self
            .league_signup_dialog
            .as_mut()
            .map(|dialog| dialog.controller.take_sound_events())
            .unwrap_or_default();
        for sound in sounds {
            self.play_ui_sound(match sound {
                clonk_frontend::league_signup::LeagueSignupSound::ArrowHit => "ArrowHit",
                clonk_frontend::league_signup::LeagueSignupSound::Click => "Click",
            });
        }
        if !actions.is_empty() {
            self.mark_menu_dirty();
        }
        for action in actions {
            match action {
                LeagueSignupAction::FocusChanged(_)
                | LeagueSignupAction::TextChanged { .. }
                | LeagueSignupAction::PasswordEnabledChanged(_) => {}
                LeagueSignupAction::OpenEditContextMenu(request) => {
                    let field = request.field;
                    let entries = request
                        .items
                        .into_iter()
                        .map(|item| {
                            let (label_key, tooltip_key) = match item.command {
                                clonk_frontend::league_signup::LeagueSignupEditContextCommand::Cut => {
                                    ("IDS_DLG_CUT", "IDS_DLGTIP_CUT")
                                }
                                clonk_frontend::league_signup::LeagueSignupEditContextCommand::Copy => {
                                    ("IDS_DLG_COPY", "IDS_DLGTIP_COPY")
                                }
                                clonk_frontend::league_signup::LeagueSignupEditContextCommand::Paste => {
                                    ("IDS_DLG_PASTE", "IDS_DLGTIP_PASTE")
                                }
                                clonk_frontend::league_signup::LeagueSignupEditContextCommand::Clear => {
                                    ("IDS_DLG_CLEAR", "IDS_DLGTIP_CLEAR")
                                }
                                clonk_frontend::league_signup::LeagueSignupEditContextCommand::SelectAll => {
                                    ("IDS_DLG_SELALL", "IDS_DLGTIP_SELALL")
                                }
                            };
                            ContextMenuEntry::new(
                                self.runtime_resource_text(label_key, &item.label),
                            )
                            .with_tooltip(
                                self.runtime_resource_text(tooltip_key, &item.tooltip),
                            )
                            .with_icon(ContextMenuIcon::None)
                            .with_action(AppContextMenuCommand::LeagueSignupEdit {
                                field,
                                command: item.command,
                            })
                        })
                        .collect();
                    self.open_context_menu_at(entries, request.anchor)?;
                }
                LeagueSignupAction::ClipboardTransfer { field, text, cut } => {
                    match arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(text))
                    {
                        Ok(()) if cut => {
                            let layout = self.league_signup_layout();
                            let fonts = self.assets.clonk_fonts.clone();
                            let follow_up = layout
                                .as_ref()
                                .zip(fonts.as_deref())
                                .and_then(|(layout, fonts)| {
                                    self.league_signup_dialog.as_mut().map(|dialog| {
                                        dialog.controller.confirm_clipboard_cut(
                                            field,
                                            layout,
                                            &fonts.text,
                                        )
                                    })
                                })
                                .unwrap_or_default();
                            self.process_league_signup_actions(follow_up)?;
                        }
                        Ok(()) => {}
                        Err(error) => {
                            tracing::warn!(%error, "failed to copy league-signup edit text");
                        }
                    }
                }
                LeagueSignupAction::ValidationFailed(failure) => {
                    self.league_signup_pointer_capture = false;
                    if let Some(dialog) = self.league_signup_dialog.as_mut() {
                        dialog.controller.cancel_interaction();
                    }
                    self.push_message_dialog(
                        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                            failure.message,
                            failure.caption,
                            clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                        ),
                        MessageDialogContinuation::None,
                    )?;
                }
                LeagueSignupAction::Submitted(submission) => {
                    let Some(pending) = self.league_signup_dialog.take() else {
                        break;
                    };
                    self.league_signup_pointer_capture = false;
                    let mut auth = pending.auth;
                    let mode = pending.controller.mode();
                    match mode {
                        LeagueSignupMode::Login => {
                            auth.account = LegacyCString::from_bytes(submission.account)
                                .expect("validated league account cannot contain NUL");
                            auth.password =
                                LegacyCString::from_bytes(submission.password.unwrap_or_default())
                                    .expect("validated league password cannot contain NUL");
                            auth.new_account = LegacyCString::default();
                            auth.new_password = LegacyCString::default();
                            self.set_league_player_auth_settings(auth.clone());
                        }
                        LeagueSignupMode::Registration => {
                            auth.new_account = LegacyCString::from_bytes(submission.account)
                                .expect("validated league account cannot contain NUL");
                            auth.new_password = submission.password.map_or_else(
                                || auth.password.clone(),
                                |password| {
                                    LegacyCString::from_bytes(password)
                                        .expect("validated league password cannot contain NUL")
                                },
                            );
                        }
                    }
                    let _ =
                        self.begin_league_player_auth_exchange(pending.continuation, auth, mode)?;
                    break;
                }
                LeagueSignupAction::Aborted { caption, message } => {
                    let Some(pending) = self.league_signup_dialog.take() else {
                        break;
                    };
                    self.league_signup_pointer_capture = false;
                    self.cancelled_league_signup_continuation = Some(pending.continuation);
                    self.push_message_dialog(
                        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                            message,
                            caption,
                            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                        ),
                        MessageDialogContinuation::LeagueSignupCancelled,
                    )?;
                    break;
                }
            }
        }
        Ok(())
    }

    fn league_auth_continuation_has_current(continuation: &LeaguePlayerAuthContinuation) -> bool {
        match continuation {
            LeaguePlayerAuthContinuation::InitialClient { request, index, .. }
            | LeaguePlayerAuthContinuation::RuntimePlayer { request, index, .. } => {
                *index < request.players.len()
            }
            LeaguePlayerAuthContinuation::StartupHost { players, index, .. } => {
                *index < players.len()
            }
        }
    }

    pub(crate) fn league_auth_continuation_player_name(
        continuation: &LeaguePlayerAuthContinuation,
    ) -> String {
        let player = match continuation {
            LeaguePlayerAuthContinuation::InitialClient { request, index, .. }
            | LeaguePlayerAuthContinuation::RuntimePlayer { request, index, .. } => {
                &request.players[*index]
            }
            LeaguePlayerAuthContinuation::StartupHost { players, index, .. } => &players[*index],
        };
        legacy_presentation_text(control_player_effective_name(player))
    }

    pub(crate) fn league_auth_continuation_server_name(
        continuation: &LeaguePlayerAuthContinuation,
    ) -> &str {
        match continuation {
            LeaguePlayerAuthContinuation::InitialClient { server_name, .. }
            | LeaguePlayerAuthContinuation::RuntimePlayer { server_name, .. }
            | LeaguePlayerAuthContinuation::StartupHost { server_name, .. } => server_name,
        }
    }

    pub(crate) fn advance_league_auth_continuation(
        continuation: &mut LeaguePlayerAuthContinuation,
    ) {
        match continuation {
            LeaguePlayerAuthContinuation::InitialClient { index, .. }
            | LeaguePlayerAuthContinuation::RuntimePlayer { index, .. }
            | LeaguePlayerAuthContinuation::StartupHost { index, .. } => *index += 1,
        }
    }

    pub(crate) fn reject_league_auth_continuation_player(
        continuation: &mut LeaguePlayerAuthContinuation,
    ) {
        match continuation {
            LeaguePlayerAuthContinuation::InitialClient { request, index, .. }
            | LeaguePlayerAuthContinuation::RuntimePlayer { request, index, .. } => {
                request.players.swap_remove(*index);
            }
            LeaguePlayerAuthContinuation::StartupHost { players, index, .. } => {
                players.swap_remove(*index);
            }
        }
    }

    fn apply_league_auth_response(
        continuation: &mut LeaguePlayerAuthContinuation,
        response: &clonk_network::LeagueAuthResponse,
    ) -> bool {
        match continuation {
            LeaguePlayerAuthContinuation::InitialClient { request, index, .. }
            | LeaguePlayerAuthContinuation::RuntimePlayer { request, index, .. } => {
                response.apply_player_auth(&mut request.players[*index])
            }
            LeaguePlayerAuthContinuation::StartupHost { players, index, .. } => {
                response.apply_player_auth(&mut players[*index])
            }
        }
    }

    fn finish_league_auth_continuation(
        &mut self,
        continuation: LeaguePlayerAuthContinuation,
    ) -> LeaguePlayerAuthStatus {
        match continuation {
            LeaguePlayerAuthContinuation::InitialClient { request, .. } => {
                let submitted = self.network.as_ref().is_some_and(|network| {
                    network
                        .submit_player_info_update(request)
                        .inspect_err(
                            |error| tracing::error!(%error, "failed to submit initial PlayerInfo"),
                        )
                        .is_ok()
                });
                if self.pending_network_join_data.is_some() {
                    self.initial_lobby_status_ack_pending = submitted
                        && self
                            .pending_network_join_data
                            .as_ref()
                            .is_some_and(|join_data| {
                                join_data.status.state == clonk_network::NETWORK_STATE_LOBBY
                            });
                    self.acknowledge_initial_lobby_status_if_ready();
                }
                LeaguePlayerAuthStatus::Completed(submitted)
            }
            LeaguePlayerAuthContinuation::StartupHost {
                mode,
                manager,
                selected_scenario,
                purpose,
                players,
                ..
            } => {
                let (sender, receiver) = mpsc::channel();
                if sender.send(Ok((mode, manager))).is_err() {
                    return LeaguePlayerAuthStatus::Completed(false);
                }
                let mut connection =
                    StartupNetworkConnection::new(receiver, selected_scenario, purpose);
                connection.authenticated_league_players = Some(players);
                self.startup_network_connection = Some(connection);
                LeaguePlayerAuthStatus::Completed(true)
            }
            LeaguePlayerAuthContinuation::RuntimePlayer {
                request,
                host,
                alternate_resource_id,
                alternate_color,
                ..
            } => {
                if request.players.is_empty() {
                    return LeaguePlayerAuthStatus::Completed(false);
                }
                let completed = self
                    .finish_runtime_network_player_add(
                        request,
                        host,
                        alternate_resource_id,
                        alternate_color,
                    )
                    .inspect_err(|error| {
                        tracing::error!(%error, "failed to finish authenticated runtime player add")
                    })
                    .is_ok();
                LeaguePlayerAuthStatus::Completed(completed)
            }
        }
    }

    pub(crate) fn begin_league_player_auth_exchange(
        &mut self,
        mut continuation: LeaguePlayerAuthContinuation,
        auth: clonk_network::LeagueAuthRequestHead,
        mode: clonk_frontend::league_signup::LeagueSignupMode,
    ) -> Result<LeaguePlayerAuthStatus, EngineError> {
        if self.pending_league_player_auth.is_some() {
            tracing::warn!("refusing to replace an in-flight league player authentication");
            return Ok(LeaguePlayerAuthStatus::Completed(false));
        }
        if !Self::league_auth_continuation_has_current(&continuation) {
            return Ok(self.finish_league_auth_continuation(continuation));
        }
        let exchange = match &continuation {
            LeaguePlayerAuthContinuation::InitialClient { request, index, .. }
            | LeaguePlayerAuthContinuation::RuntimePlayer { request, index, .. } => {
                match self.network.as_ref() {
                    Some(network) => network
                        .begin_authenticate_league_player(auth.clone(), &request.players[*index]),
                    None => Ok(None),
                }
            }
            LeaguePlayerAuthContinuation::StartupHost {
                manager,
                players,
                index,
                ..
            } => manager.begin_authenticate_league_player(auth.clone(), &players[*index]),
        };
        let exchange = match exchange {
            Ok(Some(exchange)) => exchange,
            Ok(None) => {
                Self::reject_league_auth_continuation_player(&mut continuation);
                return self.continue_league_player_auth(continuation);
            }
            Err(error) => {
                tracing::warn!(%error, "failed to begin league player authentication");
                Self::reject_league_auth_continuation_player(&mut continuation);
                return self.continue_league_player_auth(continuation);
            }
        };
        let player = Self::league_auth_continuation_player_name(&continuation);
        let server = Self::league_auth_continuation_server_name(&continuation).to_string();
        let message = format_resource_string(
            self.runtime_resource_text(
                "IDS_MSG_TRYLEAGUESIGNUP",
                "League login for player %s on %s...",
            ),
            &[&player, &server],
        );
        let caption = self.runtime_resource_text("IDS_DLG_LEAGUESIGNUP", "League Login");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(3),
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::LeaguePlayerAuthWait,
        )?;
        self.pending_league_player_auth = Some(PendingLeaguePlayerAuth {
            continuation,
            stage: PendingLeaguePlayerAuthStage::Waiting(exchange),
            auth,
            mode,
        });
        Ok(LeaguePlayerAuthStatus::Pending)
    }

    pub(crate) fn continue_league_player_auth(
        &mut self,
        continuation: LeaguePlayerAuthContinuation,
    ) -> Result<LeaguePlayerAuthStatus, EngineError> {
        if self.pending_league_player_auth.is_some() || self.league_signup_dialog.is_some() {
            tracing::warn!("refusing to replace an in-flight league player authentication");
            return Ok(LeaguePlayerAuthStatus::Completed(false));
        }
        if !Self::league_auth_continuation_has_current(&continuation) {
            return Ok(self.finish_league_auth_continuation(continuation));
        }
        let auth = self.league_player_auth_settings();
        if self.league_login_prompt_required() {
            self.open_league_signup_dialog(
                clonk_frontend::league_signup::LeagueSignupMode::Login,
                auth,
                continuation,
            )?;
            return Ok(LeaguePlayerAuthStatus::Pending);
        }
        self.begin_league_player_auth_exchange(
            continuation,
            auth,
            clonk_frontend::league_signup::LeagueSignupMode::Login,
        )
    }

    fn dismiss_league_player_auth_wait(&mut self) {
        let Some(index) = self.message_dialogs.iter().rposition(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::LeaguePlayerAuthWait
            )
        }) else {
            return;
        };
        self.remove_message_dialog_at(index);
        self.mark_menu_dirty();
    }

    pub(crate) fn clear_pending_league_player_auth(&mut self) {
        self.pending_league_player_auth = None;
        let mut removed = false;
        while let Some(index) = self.message_dialogs.iter().rposition(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::LeaguePlayerAuthWait
                    | MessageDialogContinuation::LeaguePlayerAuthWelcome
                    | MessageDialogContinuation::LeaguePlayerAuthError
                    | MessageDialogContinuation::LeaguePlayerAuthCancelled
            )
        }) {
            self.remove_message_dialog_at(index);
            removed = true;
        }
        if removed {
            self.mark_menu_dirty();
        }
    }

    pub(crate) fn reopen_league_player_auth_form(
        &mut self,
        mut pending: PendingLeaguePlayerAuth,
    ) -> Result<(), EngineError> {
        // C4Network2::LeaguePlrAuth clears only the process-global password
        // after the error/cancellation modal closes. Its loop keeps the
        // local Password value for registration's no-custom-password
        // fallback, while a login retry starts with an empty password edit.
        self.clear_remembered_league_password();
        pending.auth.new_account = LegacyCString::default();
        pending.auth.new_password = LegacyCString::default();
        if pending.mode == clonk_frontend::league_signup::LeagueSignupMode::Login {
            pending.auth.password = LegacyCString::default();
        }
        self.open_league_signup_dialog(pending.mode, pending.auth, pending.continuation)
    }

    fn push_league_player_check_error(&mut self, message: String) -> Result<(), EngineError> {
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message,
                self.runtime_resource_text("IDS_DLG_ERROR", "Error"),
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        )
    }

    pub(crate) fn poll_league_player_auth(&mut self) -> Result<(), EngineError> {
        let Some(pending) = self.pending_league_player_auth.as_ref() else {
            return Ok(());
        };
        let PendingLeaguePlayerAuthStage::Waiting(exchange) = &pending.stage else {
            return Ok(());
        };
        let Some(result) = exchange.try_complete() else {
            return Ok(());
        };
        let mut pending = self
            .pending_league_player_auth
            .take()
            .expect("polled league Auth state exists");
        self.dismiss_league_player_auth_wait();
        match result {
            Ok(response)
                if response.is_register()
                    && pending.mode == clonk_frontend::league_signup::LeagueSignupMode::Login =>
            {
                if !response.account.is_empty() {
                    pending.auth.account = response.account;
                }
                pending.auth.new_account = LegacyCString::default();
                pending.auth.new_password = LegacyCString::default();
                self.open_league_signup_dialog(
                    clonk_frontend::league_signup::LeagueSignupMode::Registration,
                    pending.auth,
                    pending.continuation,
                )?;
            }
            Ok(response)
                if Self::apply_league_auth_response(&mut pending.continuation, &response) =>
            {
                if pending.mode == clonk_frontend::league_signup::LeagueSignupMode::Registration
                    && !response.account.is_empty()
                {
                    pending.auth.account.clone_from(&response.account);
                }
                if load_league_auto_login(self.app_paths.as_ref()) {
                    Self::advance_league_auth_continuation(&mut pending.continuation);
                    let _ = self.continue_league_player_auth(pending.continuation)?;
                    return Ok(());
                }
                let player = Self::league_auth_continuation_player_name(&pending.continuation);
                let server =
                    Self::league_auth_continuation_server_name(&pending.continuation).to_string();
                let message = if response.message.is_empty() {
                    if response.account.is_empty() {
                        format_resource_string(
                            self.runtime_resource_text(
                                "IDS_MSG_LEAGUEPLAYERSIGNUP",
                                "Player: %s|Server: %s",
                            ),
                            &[&player, &server],
                        )
                    } else {
                        let account = legacy_presentation_text(response.account.as_bytes());
                        format_resource_string(
                            self.runtime_resource_text(
                                "IDS_MSG_LEAGUEPLAYERSIGNUPAS",
                                "Player: %s|League user name: %s|Server: %s",
                            ),
                            &[&player, &account, &server],
                        )
                    }
                } else {
                    legacy_presentation_text(response.message.as_bytes())
                };
                pending.stage = PendingLeaguePlayerAuthStage::Decision;
                self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::new(
                        message,
                        self.runtime_resource_text(
                            "IDS_DLG_LEAGUESIGNUPCONFIRM",
                            "Confirm League Login",
                        ),
                        clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL,
                        clonk_frontend::message_dialog::MessageDialogIcon::Extended(8),
                        clonk_frontend::message_dialog::MessageDialogSize::Regular,
                        false,
                    ),
                    MessageDialogContinuation::LeaguePlayerAuthWelcome,
                )?;
                self.pending_league_player_auth = Some(pending);
            }
            Ok(response) => {
                if pending.mode == clonk_frontend::league_signup::LeagueSignupMode::Registration
                    && !response.account.is_empty()
                {
                    pending.auth.account.clone_from(&response.account);
                }
                pending.auth.new_account = LegacyCString::default();
                pending.auth.new_password = LegacyCString::default();
                let server_message = if response.is_success() && response.auid.is_empty() {
                    self.runtime_resource_text(
                        "IDS_MSG_LEAGUESERVERREPLYWITHOUTA",
                        "League server reply without authentication-id!",
                    )
                } else {
                    legacy_presentation_text(response.message.as_bytes())
                };
                let message = format_resource_string(
                    self.runtime_resource_text(
                        "IDS_MSG_LEAGUESERVERMSG",
                        "League server reply: %s",
                    ),
                    &[&server_message],
                );
                pending.stage = PendingLeaguePlayerAuthStage::Decision;
                self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                        message,
                        self.runtime_resource_text(
                            "IDS_DLG_LEAGUESIGNUPFAILED",
                            "League Login Failed",
                        ),
                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    ),
                    MessageDialogContinuation::LeaguePlayerAuthError,
                )?;
                self.pending_league_player_auth = Some(pending);
            }
            Err(error) => {
                let message = format_resource_string(
                    self.runtime_resource_text(
                        "IDS_MSG_LEAGUESERVERMSG",
                        "League server reply: %s",
                    ),
                    &[&error.to_string()],
                );
                pending.stage = PendingLeaguePlayerAuthStage::Decision;
                self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                        message,
                        self.runtime_resource_text(
                            "IDS_DLG_LEAGUESIGNUPFAILED",
                            "League Login Failed",
                        ),
                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    ),
                    MessageDialogContinuation::LeaguePlayerAuthError,
                )?;
                self.pending_league_player_auth = Some(pending);
            }
        }
        Ok(())
    }

    pub(crate) fn finalize_prepared_host_players(
        &mut self,
        manager: &NetworkManager,
        prepared: &mut prepared_host_bootstrap::PreparedHostBootstrap,
        is_league: bool,
        authenticated_league_players: Option<Vec<clonk_engine::ControlPlayerInfoEntry>>,
    ) -> Result<(), String> {
        let auth = self.league_player_auth_settings();
        let league = prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .map(|snapshot| synchronized_league_name(&snapshot.parameters))
            .and_then(clonk_engine::LegacyCString::from_bytes)
            .unwrap_or_default();
        let already_authenticated = authenticated_league_players;
        let mut players = already_authenticated.clone().unwrap_or_else(|| {
            prepared
                .pending_initial_league_players()
                .unwrap_or_default()
                .to_vec()
        });
        // JoinLocalPlayer obtains AUIDs before ID allocation, team assignment,
        // or attribute conflict resolution.
        if is_league && already_authenticated.is_none() {
            retain_player_infos_with_cpp_swap_remove(&mut players, |player| {
                match manager.authenticate_league_player(auth.clone(), player) {
                    Ok(true) => true,
                    Ok(false) => false,
                    Err(error) => {
                        tracing::warn!(name = ?player.name, %error, "host league player authentication failed");
                        false
                    }
                }
            });
        }
        let mut oracle =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template.clone());
        let refusal_template = self.runtime_resource_text(
            "IDS_MSG_LEAGUEJOINREFUSED",
            "League server has refused the join of player %s: %s",
        );
        let mut check_errors = Vec::new();
        let finalized = prepared
            .finalize_initial_league_players(
                players,
                &mut oracle,
                // The callback runs after full host normalization. Restore
                // script rows are appended only after it returns.
                |player| {
                    if !is_league {
                        return true;
                    }
                    match manager.check_league_player(&league, player) {
                        Ok(network::LeaguePlayerCheck::Accepted) => true,
                        Ok(network::LeaguePlayerCheck::Unavailable) => false,
                        Ok(network::LeaguePlayerCheck::Rejected(message)) => {
                            let player_name = legacy_presentation_text(player.name.as_bytes());
                            let message = legacy_presentation_text(message.as_bytes());
                            check_errors.push(format_resource_string(
                                refusal_template.clone(),
                                &[&player_name, &message],
                            ));
                            false
                        }
                        Err(error) => {
                            tracing::warn!(player_id = player.id, %error, "host league player check failed");
                            check_errors.push(error.to_string());
                            false
                        }
                    }
                },
            );
        for message in check_errors {
            self.push_league_player_check_error(message)
                .map_err(|error| error.to_string())?;
        }
        finalized.map_err(|error| error.to_string())?;
        Ok(())
    }

    /// `C4Game::JoinPlayer` for the standalone app's local player
    /// (C4Game.cpp:3511-3534 -> C4PlayerList::Join, C4PlayerList.cpp:
    /// 271-318): joins through the engine's C4Player::Init/ScenarioInit
    /// port, so the scenario's [PlayerN] ready crew is placed
    /// (PlaceReadyCrew, C4Player.cpp:481-570), InitializePlayer is
    /// broadcast (C4Player.cpp:769-775) and the cursor lands on the
    /// hi-rank crew member (FinalInit -> AdjustCursorCommand,
    /// C4Player.cpp:794). The joined number becomes the local owner
    /// (C4PlayerList::GetFreeNumber, C4PlayerList.cpp:189-201).
    pub(crate) fn join_local_player(&mut self) -> Result<(), EngineError> {
        if self.engine.player(self.local_owner).is_some() {
            return Ok(());
        }
        let retained_player_info_core = self
            .selected_player_file
            .as_ref()
            .map(|player| player.exact_info_core());
        let (
            name,
            color_dw,
            pref_color,
            pref_position,
            crew,
            control_style,
            auto_context_menu,
            preferred_control,
            prefers_mouse,
            score,
            rounds,
            rounds_won,
            rounds_lost,
            total_playing_time,
        ) = self
            .selected_player_file
            .as_ref()
            .map(|player| {
                let name = if self.player_name == "Player" {
                    player.name.clone()
                } else {
                    self.player_name.clone()
                };
                (
                    name,
                    player.pref_color_dw & 0x00ff_ffff,
                    player.pref_color,
                    player.pref_position,
                    player.crew.clone(),
                    player.pref_control_style,
                    player.pref_auto_context_menu,
                    player.pref_control,
                    player.pref_mouse,
                    player.score,
                    player.rounds,
                    player.rounds_won,
                    player.rounds_lost,
                    player.total_playing_time,
                )
            })
            .unwrap_or_else(|| {
                // The C++ new-player dialog opts fresh players into
                // Jump'n'Run controls (C4StartupPlrSelDlg.cpp:1103-1113).
                (
                    self.player_name.clone(),
                    0xff,
                    0,
                    0,
                    Vec::new(),
                    true,
                    true,
                    0,
                    true,
                    0,
                    0,
                    0,
                    0,
                    0,
                )
            });
        let predicted_owner = self.engine.next_player_number();
        let control = self.local_controls.initialize(LocalControlInit {
            owner: predicted_owner,
            preferred_set: preferred_control,
            prefers_mouse,
            gamepads_enabled: self.gamepads_enabled,
            replay: false,
            disable_mouse: !self.mouse_control_allowed,
        });
        let config = JoinPlayerConfig {
            name,
            player_info_id: 0,
            score,
            rounds,
            rounds_won,
            rounds_lost,
            total_playing_time,
            team: None,
            color_dw,
            pref_color,
            pref_position,
            crew,
            startup_player_count: 1,
            control_style,
            auto_context_menu,
        };
        let join_result = if let Some(core) = retained_player_info_core {
            self.engine.join_player_with_profile_core(
                config,
                clonk_engine::PlayerAtClient::HOST,
                "Local",
                None,
                control.runtime_control(),
                core,
            )
        } else {
            self.engine
                .join_player_with_runtime_control(config, control.runtime_control())
        };
        let joined = match join_result {
            Ok(joined) => joined,
            Err(error) => {
                self.remove_local_control_assignment(predicted_owner);
                return Err(error);
            }
        };
        debug_assert_eq!(joined.number(), predicted_owner);
        self.local_owner = joined.number();
        self.mouse_control = self.local_controls.mouse_owner().is_some();
        if matches!(
            joined,
            clonk_engine::JoinPlayerOutcome::AwaitingTeamSelection { .. }
        ) {
            self.open_initial_team_selection(joined.number());
        }
        Ok(())
    }

    pub(crate) fn reconcile_network_stats_series(&mut self) {
        if self.network_stats.is_none() {
            return;
        }

        let clients = self
            .control_clients
            .snapshot()
            .into_iter()
            .filter_map(|client| {
                ClientId::try_from(client.client_id)
                    .ok()
                    .map(|client_id| (client_id, legacy_presentation_text(client.name.as_bytes())))
            })
            .collect::<Vec<_>>();
        let client_ids = clients
            .iter()
            .map(|(client_id, _)| *client_id)
            .collect::<HashSet<_>>();
        let players = self
            .engine
            .players()
            .map(|player| {
                let color = player.color().map_or(0x00ff_ffff, |color| {
                    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
                });
                (player.id(), player.name().to_string(), color)
            })
            .collect::<Vec<_>>();
        let player_ids = players
            .iter()
            .map(|(player_id, _, _)| *player_id)
            .collect::<HashSet<_>>();
        let removed_clients = self
            .network_stats_clients
            .difference(&client_ids)
            .copied()
            .collect::<Vec<_>>();
        let removed_players = self
            .network_stats_players
            .difference(&player_ids)
            .copied()
            .collect::<Vec<_>>();

        let stats = self
            .network_stats
            .as_mut()
            .expect("network statistics presence checked above");
        for client_id in removed_clients {
            stats.remove_client(client_id);
        }
        for (client_id, name) in clients {
            if !self.network_stats_clients.contains(&client_id) {
                stats.register_client(client_id, name);
            }
        }
        for player_id in removed_players {
            stats.remove_player(player_id);
        }
        for (player_id, name, color) in players {
            if !self.network_stats_players.contains(&player_id) {
                stats.register_player(player_id, name, color);
            }
        }
        self.network_stats_clients = client_ids;
        self.network_stats_players = player_ids;
    }

    pub(crate) fn network_chart_graph_snapshot(
        graph: clonk_network::NetworkStatsGraph<'_>,
        network_input_title: &str,
        network_output_title: &str,
    ) -> clonk_frontend::network_chart::NetworkChartGraphSnapshot {
        let series = (0..graph.series_count())
            .filter_map(|index| graph.series(index))
            .map(|series| {
                let start_time = series.start_time();
                let values = (start_time..series.end_time())
                    .map(|time| series.value(time))
                    .collect::<Vec<_>>();
                let title = match series.title() {
                    "Network input" => network_input_title,
                    "Network output" => network_output_title,
                    title => title,
                };
                clonk_frontend::network_chart::NetworkChartSeriesSnapshot::new(
                    title,
                    series.color(),
                    start_time,
                    values,
                )
            })
            .collect();
        clonk_frontend::network_chart::NetworkChartGraphSnapshot::new(graph.title(), series)
    }

    pub(crate) fn toggle_network_chart(&mut self) {
        self.cancel_network_chart_pointer_capture();
        if self.network_chart_dialog.take().is_some() {
            self.hide_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
            return;
        }
        self.reconcile_network_stats_series();
        let caption = self.runtime_resource_text("IDS_NET_STATISTICS", "Statistics");
        self.network_chart_dialog = Some(
            clonk_frontend::network_chart::NetworkChartDialog::new_with_caption(
                self.network.is_some(),
                caption,
            ),
        );
        self.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
        self.refresh_network_chart_dialog();
    }

    /// The chart is a non-exclusive shared-screen dialog. Only its explicit
    /// fullscreen Escape binding is reachable during play, and the callback
    /// is conditional on the chart still being the active top dialog.
    pub(crate) fn runtime_modal_above_network_chart(&self) -> bool {
        self.league_signup_dialog.is_some()
            || self.definition_selector.is_some()
            || self.startup_options_advanced_dialog.is_some()
            || self.startup_player_properties_dialog.is_some()
            || self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
    }

    pub(crate) fn network_chart_renders_elevated(&self) -> bool {
        matches!(self.mode, AppMode::Running)
            && self.network_chart_elevated
            && self.network_chart_dialog.is_some()
            && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::NetworkChart)
            && !self.runtime_modal_above_network_chart()
    }

    pub(crate) fn network_chart_owns_stronger_escape(&self) -> bool {
        self.network_chart_is_active_dialog()
            && !self.local_player_key_binding_in_scope(VirtualKeyCode::Escape)
    }

    pub(crate) fn network_chart_contains_point(&self, point: GuiPoint) -> bool {
        let (Some(dialog), Some(resources)) = (
            self.network_chart_dialog.as_ref(),
            self.assets.network_chart_resources(),
        ) else {
            return false;
        };
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        let bounds = dialog.layout(preferred, resources).bounds;
        point.x >= bounds.x as f32
            && point.x < bounds.x.saturating_add(bounds.w) as f32
            && point.y >= bounds.y as f32
            && point.y < bounds.y.saturating_add(bounds.h) as f32
    }

    pub(crate) fn runtime_network_role(&self) -> RuntimeNetworkRole {
        let Some(network) = self.network.as_ref() else {
            return RuntimeNetworkRole::Offline;
        };
        match (self.network_mode.as_ref(), network.local_client_id()) {
            (Some(NetworkMode::Host(_)), 0) => RuntimeNetworkRole::Host,
            (Some(NetworkMode::Client(_)), local_client_id) if local_client_id != 0 => {
                RuntimeNetworkRole::Client
            }
            _ => RuntimeNetworkRole::Ambiguous,
        }
    }

    pub(crate) fn runtime_network_is_paused(&self) -> bool {
        debug_assert!(self.network.is_some());
        // C4Network2::isRunning requires both GS_Go and a fully acknowledged
        // status. Either an active PAUSE/GO barrier or stopped control is
        // therefore paused for TogglePause's immediate routing decision.
        self.runtime_network_status_barrier.is_some() || !self.network_control_running
    }

    pub(crate) fn request_host_runtime_pause(&mut self, paused: bool) {
        let target_tick = if paused {
            // C4Network2::Pause targets Control.getNextControlTick().
            self.next_network_control_tick()
        } else {
            // C4Network2::Start targets the current ControlTick, which Rust's
            // already-advanced cadence clock exposes through this projection.
            self.displayed_network_control_tick()
        };
        let status = clonk_network::NetworkStatus {
            state: if paused {
                clonk_network::NETWORK_STATE_PAUSE
            } else {
                clonk_network::NETWORK_STATE_GO
            },
            control_mode: self.league_vote_control_mode(),
            target_tick,
        };
        match self.change_runtime_network_status(status) {
            Ok(()) => {
                // ChangeGameStatus changes the advertised Status before the
                // synchronized halt is reached, just as C++ IsPaused does.
                self.host_reference_paused = paused;
                self.publish_running_host_reference();
            }
            Err(error) => tracing::error!(%error, paused, "failed to toggle host pause status"),
        }
    }

    /// How long the game may sit on an unarrived control tick before saying so.
    ///
    /// Long enough that ordinary jitter never trips it, short enough that a
    /// player does not conclude the game has crashed. LegacyClonk issue #28's
    /// reporter proposed 100 ms; that is well inside normal stall duration on a
    /// bad link and would flash constantly.
    const NETWORK_STALL_NOTICE_AFTER: Duration = Duration::from_millis(1_500);

    /// Says once, per stall, that the session is waiting on the network.
    ///
    /// Deliberately not a modal: the game is still running, it is simply not
    /// advancing, and a dialog would be a worse lie than silence. The per-client
    /// detail — who is behind and by how much — is already in the F7 client list
    /// as "(wait N ms, behind M)".
    pub(crate) fn announce_network_stall(&mut self, now: Instant) -> Result<(), EngineError> {
        let (since, announced) = *self.network_stall_since.get_or_insert((now, false));
        if announced || now.saturating_duration_since(since) < Self::NETWORK_STALL_NOTICE_AFTER {
            return Ok(());
        }
        self.network_stall_since = Some((since, true));
        let text = self.runtime_resource_text("IDS_NET_WAITFORTHER", "Waiting for network...");
        self.set_network_pacing_flash(&text)
    }

    pub(crate) fn set_network_pacing_flash(&mut self, text: &str) -> Result<(), EngineError> {
        self.set_runtime_flash_message(text, RuntimeHelpCharset::Windows1252)
            .map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeFlashResources {
                        detail: error.to_string(),
                    },
                ))
            })
    }

    /// Apply process-local SetPreSend effects after their synchronized script
    /// call. C++ matches only this peer's local name; mismatches are successful
    /// no-ops and must not disturb the current target or flash.
    pub(crate) fn apply_engine_network_target_fps_requests(&mut self) -> Result<(), EngineError> {
        let requests = self.engine.take_network_target_fps_requests();
        if requests.is_empty() {
            return Ok(());
        }
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        let local_name = local_client_id
            .and_then(|client_id| self.control_clients.state(client_id))
            .map(|client| client.name.as_bytes().to_vec())
            .unwrap_or_else(|| b"???".to_vec());

        for request in requests {
            let matches = request.client_pattern.as_ref().is_none_or(|pattern| {
                let pattern = clonk_script::c4_string_bytes(pattern);
                pattern.is_empty() || classic_raw_wildcard_match(&pattern, &local_name)
            });
            if !matches {
                continue;
            }
            let Some(clock) = self.network_control_clock.as_mut() else {
                return Err(classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::NetworkControlPacing {
                        detail: "SetPreSend executed without a live network control clock"
                            .to_string(),
                    },
                )));
            };
            clock.set_target_fps(request.target_fps);
            self.set_network_pacing_flash(&format!("TargetFPS: {}", request.target_fps))?;
        }
        Ok(())
    }

    pub(crate) fn scoreboard_is_above_runtime_client_list(&self) -> bool {
        self.running_dialog_is_above(
            RunningDialogStackEntry::Scoreboard,
            RunningDialogStackEntry::RuntimeClientList,
        )
    }

    /// Pulls presentation work produced by synchronous script callbacks that
    /// ran between simulation ticks. Keep the visible boundary payload on the
    /// live matrix even when SetCell itself emitted no lifecycle request.
    pub(crate) fn sync_scoreboard_presentation(&mut self) {
        self.snapshot.hud.scoreboard = self.engine.scoreboard_snapshot();
        self.snapshot
            .hud
            .scoreboard_presentations
            .extend(self.engine.take_scoreboard_presentations());
        self.apply_scoreboard_presentation_requests();
    }

    pub(crate) fn runtime_client_row_network_state(
        local: bool,
        state: Option<&network::RuntimeNetworkClientState>,
    ) -> (
        clonk_frontend::runtime_client_list::RuntimeClientStatusIcon,
        Option<i32>,
    ) {
        use clonk_frontend::runtime_client_list::RuntimeClientStatusIcon;

        let Some(state) = state else {
            return (RuntimeClientStatusIcon::Ready, None);
        };
        let icon = match state.status {
            clonk_network::RemoteBarrierState::Joining
            | clonk_network::RemoteBarrierState::Chasing
            | clonk_network::RemoteBarrierState::NotReady => RuntimeClientStatusIcon::Loading,
            clonk_network::RemoteBarrierState::Ready if !state.control_ready => {
                RuntimeClientStatusIcon::NetWait
            }
            clonk_network::RemoteBarrierState::Ready => RuntimeClientStatusIcon::Ready,
            clonk_network::RemoteBarrierState::Removing => RuntimeClientStatusIcon::Kick,
        };
        (icon, (!local).then_some(state.wait_ms))
    }

    /// `Game.Network.isHost() && pNetClient && !pNetClient->isReady()`
    /// (src/C4Network2Dialogs.cpp:71). Only a network host sees the marker;
    /// the local row has no `C4Network2Client` at all (`:62`), and every
    /// status other than `NCS_Ready` counts as unacknowledged
    /// (src/C4Network2Client.h:113).
    pub(crate) fn runtime_client_row_unacknowledged(
        network_host: bool,
        state: Option<&network::RuntimeNetworkClientState>,
    ) -> bool {
        network_host
            && state.is_some_and(|state| state.status != clonk_network::RemoteBarrierState::Ready)
    }

    /// Compose the detailed `C4Network2::DrawStatus` text from live runtime
    /// diagnostics. Collection is skipped while its renderer flag is off so
    /// the normal frame path never blocks on worker inspection.
    pub(crate) fn update_network_status_overlay(&mut self) {
        if !self.graphics.debug_draw_flags().show_net_status {
            self.graphics.set_network_status_text(None);
            return;
        }
        let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            self.graphics.set_network_status_text(None);
            return;
        };

        let clients = self.control_clients.snapshot();
        let local = clients
            .iter()
            .find(|client| client.client_id == local_client_id);
        let activity = |client: &clonk_engine::ClientCoreControlData| {
            if client.observer {
                "Observing"
            } else if client.activated {
                "Active"
            } else {
                "Inactive"
            }
        };
        let local_name = local
            .map(|client| legacy_presentation_text(client.name.as_bytes()))
            .unwrap_or_else(|| "???".to_string());
        let local_activity = local.map(activity).unwrap_or("Inactive");
        let local_role = if local_client_id == 0 {
            "host"
        } else {
            "client"
        };

        let barrier = self.runtime_network_status_barrier;
        let displayed_status = barrier
            .map(|barrier| barrier.status)
            .or(self.runtime_network_committed_status);
        let status_state = displayed_status
            .map(|status| status.state)
            .unwrap_or_else(|| {
                if self.network_control_running {
                    clonk_network::NETWORK_STATE_GO
                } else {
                    clonk_network::NETWORK_STATE_PAUSE
                }
            });
        let status_name = match status_state {
            clonk_network::NETWORK_STATE_NONE => "none",
            clonk_network::NETWORK_STATE_INIT => "init",
            clonk_network::NETWORK_STATE_LOBBY => "lobby",
            clonk_network::NETWORK_STATE_PAUSE => "pause",
            clonk_network::NETWORK_STATE_GO => "go",
            _ => "???",
        };
        let status_tick = displayed_status.map_or_else(
            || i32::try_from(self.expected_network_control_tick()).unwrap_or(i32::MAX),
            |status| status.target_tick,
        );
        let reached = barrier.map_or_else(
            || self.runtime_network_committed_status.is_some(),
            |barrier| barrier.local_reached,
        );
        let ack = barrier.is_none() && self.runtime_network_committed_status.is_some();

        let pacing = self.network_control_pacing();
        let frame = self.engine.frame();
        let (control_tick, control_rate, presend, average_control_time) = self
            .network_control_clock
            .map_or((status_tick, self.engine.control_rate(), 0, 0), |clock| {
                (
                    clock.display_control_tick_for_frame(frame),
                    clock.control_rate(),
                    clock.control_presend(),
                    clock.avg_control_send_time(),
                )
            });
        let control_mode = match displayed_status
            .map(|status| status.control_mode)
            .or(self.runtime_network_committed_control_mode)
            .unwrap_or(2)
        {
            0 => "Decentral",
            1 => "Central",
            _ => "Async",
        };

        let (addresses, connections, client_states) = self.network.as_ref().map_or_else(
            || (Vec::new(), Vec::new(), Vec::new()),
            |network| {
                let addresses = network.local_addresses();
                let connections = network.runtime_connections().unwrap_or_default();
                let states = u32::try_from(control_tick)
                    .ok()
                    .and_then(|tick| network.runtime_client_states(tick).ok())
                    .unwrap_or_default();
                (addresses, connections, states)
            },
        );
        let protocol_name = |protocol: clonk_network::NetworkProtocol| match protocol {
            clonk_network::NetworkProtocol::Tcp => "TCP".to_string(),
            clonk_network::NetworkProtocol::Udp => "UDP".to_string(),
            clonk_network::NetworkProtocol::Unknown(value) => format!("Protocol {value}"),
            _ => "Unknown".to_string(),
        };

        let mut lines = vec![
            format!("Local: {local_activity} {local_role} {local_name} (ID {local_client_id})"),
            format!(
                "Game Status: {status_name} (tick {status_tick}){}{}",
                if reached { " reached" } else { "" },
                if ack { " ack" } else { "" },
            ),
        ];
        let tcp = addresses
            .iter()
            .find(|address| address.protocol == clonk_network::NetworkProtocol::Tcp);
        let udp = addresses
            .iter()
            .find(|address| address.protocol == clonk_network::NetworkProtocol::Udp);
        let message_io = udp.or(tcp);
        let data_io = tcp.or(udp);
        if let (Some(message_io), Some(data_io)) = (message_io, data_io) {
            // L066 supplies the C++ rate accumulator; the current session
            // transport does not yet publish its snapshot to the app, so an
            // unsampled bucket has the same zero values as native startup.
            let rates = |_protocol| (0_u64, 0_u64, 0_u64);
            let (msg_in, msg_out, msg_broadcast) = rates(message_io.protocol);
            let message_label = if message_io.protocol == data_io.protocol {
                "Msg/Data"
            } else {
                "Msg"
            };
            let mut protocols = format!(
                "Protocols: {message_label}: {} ({} i{msg_in} o{msg_out} bc{msg_broadcast})",
                protocol_name(message_io.protocol),
                message_io.endpoint.port(),
            );
            if message_io.protocol != data_io.protocol {
                let (data_in, data_out, _data_broadcast) = rates(data_io.protocol);
                protocols.push_str(&format!(
                    ", Data: {} ({} i{data_in} o{data_out} bcv)",
                    protocol_name(data_io.protocol),
                    data_io.endpoint.port(),
                ));
            }
            lines.push(protocols);
        } else {
            lines.push("Protocols: none".to_string());
        }
        lines.push(format!(
            "Control: {control_mode}, Tick {control_tick}, Behind {}, Rate {control_rate}, PreSend {presend}, ACT: {average_control_time}",
            pacing.behind,
        ));
        let stream = self.league_record_stream_status();
        if stream.is_streaming() {
            lines.push(format!(
                "Streaming: {} waiting, {} in, {} out, {} sent",
                stream.waiting_raw_bytes(),
                stream.input_position(),
                stream.pending_compressed_bytes(),
                stream.sent_position(),
            ));
        }
        lines.push("Clients:".to_string());
        for client in clients
            .iter()
            .filter(|client| client.client_id != local_client_id)
        {
            let state = client_states
                .iter()
                .find(|state| i32::try_from(state.client_id).ok() == Some(client.client_id));
            let suffix = state.map_or("", |state| match state.status {
                clonk_network::RemoteBarrierState::Joining => " (joining)",
                clonk_network::RemoteBarrierState::Chasing => " (chasing)",
                clonk_network::RemoteBarrierState::NotReady => " (!rdy)",
                clonk_network::RemoteBarrierState::Removing => " (removed)",
                clonk_network::RemoteBarrierState::Ready => " (ready to start)",
            });
            let wait = state.map_or(0, |state| state.wait_ms);
            let control_suffix = state
                .filter(|state| client.activated && !state.control_ready)
                .map_or("", |_| " (!ctrl)");
            let name = legacy_presentation_text(client.name.as_bytes());
            let client_next_control = self
                .network_client_next_control_ticks
                .get(&client.client_id)
                .copied()
                .unwrap_or(0);
            let client_behind = control_tick.wrapping_sub(client_next_control);
            lines.push(format!(
                "- {} {} {} (ID {}) (wait {wait} ms, behind {client_behind}){suffix}{control_suffix}",
                activity(client),
                if client.client_id == 0 {
                    "host"
                } else {
                    "client"
                },
                name,
                client.client_id,
            ));
            let mut routes = connections
                .iter()
                .filter(|connection| {
                    i32::try_from(connection.client_id).ok() == Some(client.client_id)
                })
                .collect::<Vec<_>>();
            if routes.is_empty() {
                lines.push("   Not connected".to_string());
            } else {
                routes.sort_by_key(|connection| match connection.usage.as_str() {
                    "Data/Msg" | "Msg" => 0,
                    "Data" => 1,
                    _ => 2,
                });
                let routes = routes
                    .into_iter()
                    .map(|connection| {
                        let usage = if connection.usage == "Data/Msg" {
                            "Msg/Data"
                        } else {
                            connection.usage.as_str()
                        };
                        let peer = connection
                            .peer_address
                            .map(|address| address.to_string())
                            .unwrap_or_else(|| "???".to_string());
                        // DrawStatus prints getPingTime(), not getLag()
                        // (src/C4Network2.cpp:1207-1219).
                        format!(
                            "{usage}: {} ({peer} p{} l{})",
                            protocol_name(connection.protocol),
                            connection.ping_ms,
                            connection.packet_loss,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("   Connections: {routes}"));
            }
        }
        if clients.is_empty() {
            lines.push(" - none -".to_string());
        }
        self.graphics.set_network_status_text(Some(lines.join("|")));
    }

    pub(crate) fn runtime_client_list_snapshot(
        &mut self,
    ) -> (
        Vec<LobbyOptionRow>,
        Vec<clonk_frontend::runtime_client_list::RuntimeClientRow>,
        clonk_frontend::runtime_client_list::RuntimeClientListStatus,
    ) {
        let behind = self.network_control_pacing().behind;
        let frame = self.engine.frame();
        let status = self.network_control_clock.map_or_else(
            || clonk_frontend::runtime_client_list::RuntimeClientListStatus {
                tick: i32::try_from(self.expected_network_control_tick()).unwrap_or(i32::MAX),
                behind,
                rate: self.engine.control_rate(),
                presend: 0,
                average_control_time: 0,
            },
            |clock| clonk_frontend::runtime_client_list::RuntimeClientListStatus {
                tick: clock.display_control_tick_for_frame(frame),
                behind,
                rate: clock.control_rate(),
                presend: clock.control_presend(),
                average_control_time: clock.avg_control_send_time(),
            },
        );
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        let local_addresses = self
            .network
            .as_ref()
            .map(NetworkManager::local_addresses)
            .unwrap_or_default();
        let runtime_connections = self
            .network
            .as_ref()
            .map(NetworkManager::runtime_connections)
            .transpose()
            .unwrap_or_else(|error| {
                tracing::debug!(%error, "runtime connection details are not available");
                None
            })
            .unwrap_or_default();
        let runtime_client_states = u32::try_from(status.tick)
            .ok()
            .and_then(|tick| {
                self.network
                    .as_ref()
                    .map(|network| network.runtime_client_states(tick))
            })
            .transpose()
            .unwrap_or_else(|error| {
                tracing::debug!(%error, "runtime client states are not available");
                None
            })
            .unwrap_or_default();
        let can_moderate = matches!(self.network_mode, Some(NetworkMode::Host(_)));
        let (_, retained_player_infos) = self.control_player_infos.retained_rows_snapshot();
        let active_player_names = retained_player_infos
            .into_iter()
            .map(|(client_id, _, players)| {
                let names = players
                    .into_iter()
                    .filter(|player| {
                        player.flags
                            & (clonk_engine::PLAYER_INFO_FLAG_REMOVED
                                | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE)
                            == 0
                    })
                    .map(|player| legacy_presentation_text(control_player_effective_name(&player)))
                    .collect::<Vec<_>>();
                (client_id, names)
            })
            .collect::<BTreeMap<_, _>>();
        let rows = self
            .control_clients
            .snapshot()
            .into_iter()
            .map(|client| {
                let player_names = active_player_names
                    .get(&client.client_id)
                    .cloned()
                    .unwrap_or_default();
                let local = local_client_id == Some(client.client_id);
                let protocol_name = |protocol| match protocol {
                    clonk_network::NetworkProtocol::Tcp => "TCP".to_string(),
                    clonk_network::NetworkProtocol::Udp => "UDP".to_string(),
                    clonk_network::NetworkProtocol::Unknown(value) => {
                        format!("Protocol {value}")
                    }
                    _ => "Unknown protocol".to_string(),
                };
                let connections = runtime_connections
                    .iter()
                    .filter(|connection| {
                        i32::try_from(connection.client_id).ok() == Some(client.client_id)
                    })
                    .map(
                        |connection| clonk_frontend::runtime_client_list::RuntimeConnectionRow {
                            connection_id: connection.connection_id,
                            usage: connection.usage.clone(),
                            protocol: protocol_name(connection.protocol),
                            peer_address: connection
                                .peer_address
                                .map(|address| address.to_string())
                                .unwrap_or_else(|| "???".to_string()),
                            packet_loss: connection.packet_loss,
                            ping_ms: connection.ping_ms,
                            lag_ms: connection.lag_ms,
                            can_disconnect: !self.network_is_league,
                        },
                    )
                    .collect::<Vec<_>>();
                let mut addresses = if local {
                    local_addresses
                        .iter()
                        .map(|address| {
                            let protocol = protocol_name(address.protocol);
                            format!("{protocol}: {}", address.endpoint)
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                addresses.extend(
                    connections
                        .iter()
                        .filter(|connection| connection.peer_address != "???")
                        .map(|connection| {
                            format!("{}: {}", connection.protocol, connection.peer_address)
                        }),
                );
                addresses.sort();
                addresses.dedup();
                let client_state = runtime_client_states
                    .iter()
                    .find(|state| i32::try_from(state.client_id).ok() == Some(client.client_id));
                let (status, wait_ms) = Self::runtime_client_row_network_state(local, client_state);
                clonk_frontend::runtime_client_list::RuntimeClientRow {
                    client_id: client.client_id,
                    name: legacy_presentation_text(client.name.as_bytes()),
                    nick: legacy_presentation_text(client.nick.as_bytes()),
                    host: client.client_id == 0,
                    local,
                    activated: client.activated,
                    observer: client.observer,
                    muted: self.control_messages.is_muted(client.client_id),
                    has_players: !player_names.is_empty(),
                    player_names,
                    addresses,
                    // Lifecycle, raw per-client control readiness, and wait
                    // come from the live network worker rather than the
                    // aggregate buffered-control count.
                    status,
                    wait_ms,
                    connections,
                    can_moderate: can_moderate && client.client_id != 0,
                    unacknowledged: Self::runtime_client_row_unacknowledged(
                        can_moderate,
                        client_state,
                    ),
                }
            })
            .collect::<Vec<_>>();
        let network_host = matches!(self.runtime_network_role(), RuntimeNetworkRole::Host);
        let runtime_join_allowed = self
            .runtime_network_join_allowed
            .or_else(|| match self.network_mode.as_ref() {
                Some(NetworkMode::Host(HostSettings {
                    prepared: Some(prepared),
                    ..
                })) => Some(prepared.admission().runtime_join_allowed()),
                _ => None,
            })
            .unwrap_or_else(|| {
                !native_config_text(
                    &load_native_config_bytes(self.app_paths.as_ref()),
                    "Network",
                    "NoRuntimeJoin",
                )
                .as_deref()
                .map(parse_config_bool)
                .unwrap_or(true)
            });
        let options = core_runtime_option_rows(
            self.engine.is_control_host(),
            network_host,
            self.network_is_league,
            &self.classic_lobby_option_labels(),
            self.runtime_client_list_control_mode(),
            status.rate,
            runtime_join_allowed,
        );
        (options, rows, status)
    }

    pub(crate) fn refresh_runtime_client_list(&mut self) -> bool {
        self.refresh_runtime_client_list_inner(false)
    }

    pub(crate) fn refresh_runtime_client_list_on_sec1(&mut self) -> bool {
        self.refresh_runtime_client_list_inner(true)
    }

    fn refresh_runtime_client_list_inner(&mut self, sec1_timer: bool) -> bool {
        if self.runtime_client_list.is_none()
            || self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.is_static_info_only())
        {
            return false;
        }
        let info_was_open = self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.info_is_open());
        let (options, rows, status) = self.runtime_client_list_snapshot();
        let close_info_only = self.runtime_client_list.as_mut().is_some_and(|dialog| {
            if sec1_timer {
                dialog.replace_snapshot_on_sec1(options, rows, status);
            } else {
                dialog.replace_snapshot(options, rows, status);
            }
            dialog.is_info_only() && !dialog.info_is_open()
        });
        if info_was_open
            && self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| !dialog.info_is_open())
        {
            self.startup_tooltip.pointer_left();
        }
        if close_info_only {
            self.runtime_client_list = None;
            self.remove_running_dialog(RunningDialogStackEntry::RuntimeClientList);
            self.runtime_client_list_above_game_over = false;
            self.hide_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
        }
        true
    }

    pub(crate) fn toggle_runtime_client_list(&mut self) -> Result<(), EngineError> {
        if self.runtime_client_list.take().is_some() {
            self.remove_running_dialog(RunningDialogStackEntry::RuntimeClientList);
            self.startup_tooltip.pointer_left();
            if self.context_menu_lobby_option.is_some() {
                self.close_context_menu_silently();
            }
            self.runtime_client_list_consumed_keys.clear();
            self.hide_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
            return Ok(());
        }
        if !matches!(
            self.runtime_network_role(),
            RuntimeNetworkRole::Host | RuntimeNetworkRole::Client
        ) {
            return Ok(());
        }
        Self::guard_gui_overlay_result(
            "C4Network2ClientListDlg",
            self.assets
                .runtime_client_list_resources()
                .context("exact C4Network2ClientListDlg resource set is absent")
                .and_then(|resources| resources.validate()),
        )?;
        let (options, rows, status) = self.runtime_client_list_snapshot();
        let option_caption_reference = self.classic_lobby_option_labels().runtime_join;
        self.runtime_client_list_consumed_keys.clear();
        self.runtime_client_list = Some(
            clonk_frontend::runtime_client_list::RuntimeClientListDialog::new(
                self.runtime_resource_string("IDS_NET_CAPTION"),
                options,
                rows,
                status,
            )
            .with_option_caption_reference(option_caption_reference)
            .with_info_caption(self.runtime_resource_string("IDS_NET_CLIENT_INFO")),
        );
        self.show_running_dialog(RunningDialogStackEntry::RuntimeClientList);
        self.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
        Ok(())
    }

    pub(crate) fn runtime_client_has_players(&self, client_id: i32) -> bool {
        self.engine
            .players()
            .any(|player| player.at_client().get() == client_id)
    }

    pub(crate) fn handle_runtime_client_list_action(
        &mut self,
        action: clonk_frontend::runtime_client_list::RuntimeClientListAction,
    ) -> Result<(), EngineError> {
        use clonk_frontend::runtime_client_list::RuntimeClientListAction;
        match action {
            RuntimeClientListAction::Close => {
                self.startup_tooltip.pointer_left();
                self.runtime_client_list = None;
                self.remove_running_dialog(RunningDialogStackEntry::RuntimeClientList);
                self.runtime_client_list_above_game_over = false;
                self.hide_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
                return Ok(());
            }
            RuntimeClientListAction::OpenInfo(_) => {
                self.startup_tooltip.pointer_left();
                return Ok(());
            }
            RuntimeClientListAction::CloseInfo => {
                self.startup_tooltip.pointer_left();
                if self
                    .runtime_client_list
                    .as_ref()
                    .is_some_and(|dialog| dialog.is_info_only())
                {
                    self.runtime_client_list = None;
                    self.remove_running_dialog(RunningDialogStackEntry::RuntimeClientList);
                    self.runtime_client_list_above_game_over = false;
                    self.hide_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
                }
                return Ok(());
            }
            RuntimeClientListAction::OptionSelectionRequested {
                option,
                anchor,
                minimum_width,
            } => {
                self.open_runtime_client_list_option_combo(option, anchor, minimum_width)?;
                return Ok(());
            }
            RuntimeClientListAction::ToggleMute(client_id) => {
                if self.control_clients.contains(client_id) {
                    let muted = !self.control_messages.is_muted(client_id);
                    self.control_messages.set_muted(client_id, muted);
                }
            }
            RuntimeClientListAction::ToggleActivate(client_id) => {
                if !matches!(self.network_mode, Some(NetworkMode::Host(_)))
                    || client_id == 0
                    || !self.control_clients.contains(client_id)
                {
                    return Ok(());
                }
                if self.network_is_league && self.runtime_client_has_players(client_id) {
                    self.append_control_message_log(
                        self.runtime_resource_string("IDS_LOG_COMMANDNOTALLOWEDINLEAGUE"),
                        CONTROL_LOG_COLOR,
                        None,
                    );
                    return Ok(());
                }
                let update = clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    client_id,
                    data: i32::from(!self.control_clients.is_activated(client_id)),
                    by_client: 0,
                };
                if let Some(Err(error)) = self
                    .network
                    .as_ref()
                    .map(|network| network.submit_client_update(update))
                {
                    tracing::error!(%client_id, %error, "failed to submit client-list activation");
                }
            }
            RuntimeClientListAction::Kick(client_id) => {
                if !matches!(self.network_mode, Some(NetworkMode::Host(_)))
                    || client_id == 0
                    || !self.control_clients.contains(client_id)
                {
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
                let reason = self.runtime_resource_string("IDS_MSG_KICKFROMCLIENTLIST");
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
                    tracing::error!(%client_id, %error, "failed to submit client-list kick");
                }
            }
            RuntimeClientListAction::Disconnect {
                client_id,
                connection_id,
            } => {
                if self.network_is_league {
                    return Ok(());
                }
                if let Some(Err(error)) = self
                    .network
                    .as_ref()
                    .map(|network| network.disconnect_runtime_connection(connection_id))
                {
                    tracing::warn!(
                        %client_id,
                        %connection_id,
                        %error,
                        "failed to disconnect runtime client route"
                    );
                }
            }
        }
        self.refresh_runtime_client_list();
        Ok(())
    }

    pub(crate) fn runtime_client_list_contains_point(&self, point: GuiPoint) -> bool {
        let (Some(dialog), Some((preferred, line_height))) = (
            self.runtime_client_list.as_ref(),
            self.runtime_client_list_input_geometry(),
        ) else {
            return false;
        };
        let bounds = dialog.layout(preferred, line_height).bounds;
        point.x >= bounds.x as f32
            && point.x < bounds.x.saturating_add(bounds.w) as f32
            && point.y >= bounds.y as f32
            && point.y < bounds.y.saturating_add(bounds.h) as f32
    }

    pub(crate) fn runtime_client_list_owns_game_over(&self) -> bool {
        self.runtime_client_list.is_some()
            && self.game_over_dialog.is_some()
            && self.runtime_default_dialog_is_above(
                RuntimeDefaultDialog::ClientList,
                RuntimeDefaultDialog::GameOver,
            )
    }

    pub(crate) fn runtime_client_list_is_active(&self) -> bool {
        self.runtime_client_list.is_some()
            && (!matches!(self.mode, AppMode::Running)
                || self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ClientList))
    }

    pub(crate) fn stop_runtime_client_list_title_drag_at_current_position(&mut self) {
        if !self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.has_positional_pointer_drag())
        {
            return;
        }
        let Some(point) = self.running_pointer_position else {
            if let Some(dialog) = self.runtime_client_list.as_mut() {
                dialog.pointer_left();
            }
            return;
        };
        let Some((preferred, line_height)) = self.runtime_client_list_input_geometry() else {
            if let Some(dialog) = self.runtime_client_list.as_mut() {
                dialog.pointer_left();
            }
            return;
        };
        if let Some(dialog) = self.runtime_client_list.as_mut() {
            let action = dialog.handle_pointer_up(point, preferred, line_height);
            debug_assert!(action.is_none(), "a title drag cannot activate a control");
        }
    }

    pub(crate) fn expected_network_control_tick(&self) -> Tick {
        self.network_control_clock
            .and_then(|clock| Tick::try_from(clock.current_tick()).ok())
            .unwrap_or_else(|| u32::try_from(self.engine.frame()).unwrap_or(u32::MAX))
    }

    /// Rust advances this clock as soon as a cadence control is consumed,
    /// while C++ advances ControlTick at the following frame boundary.
    /// Consequently Rust's current tick is already C++ getNextControlTick.
    pub(crate) fn next_network_control_tick(&self) -> i32 {
        self.network_control_clock
            .map(NetworkControlClock::current_tick)
            .unwrap_or_else(|| {
                i32::try_from(self.local_control_submission_tick()).unwrap_or(i32::MAX)
            })
    }

    pub(crate) fn displayed_network_control_tick(&self) -> i32 {
        self.network_control_clock
            .map(|clock| clock.display_control_tick_for_frame(self.engine.frame()))
            .unwrap_or_else(|| {
                i32::try_from(self.expected_network_control_tick()).unwrap_or(i32::MAX)
            })
    }

    /// `C4GameControlNetwork::CopyClientList` clears the private control
    /// roster and re-adds every activated client at one shared ControlTick.
    pub(crate) fn refresh_network_client_next_control_ticks(&mut self) {
        let control_tick = self.displayed_network_control_tick();
        self.network_client_next_control_ticks = self
            .control_clients
            .activated_client_ids()
            .into_iter()
            .map(|client_id| (client_id, control_tick))
            .collect();
    }

    fn arm_runtime_network_status_barrier(&mut self, status: clonk_network::NetworkStatus) -> bool {
        if self.mode != AppMode::Running
            || self.network.is_none()
            || status.target_tick < 0
            || !matches!(
                status.state,
                clonk_network::NETWORK_STATE_GO | clonk_network::NETWORK_STATE_PAUSE
            )
        {
            return false;
        }
        self.runtime_network_control_mode = Some(status.control_mode);
        if self
            .runtime_network_status_barrier
            .is_some_and(|pending| same_runtime_network_status_barrier(pending.status, status))
        {
            return false;
        }
        self.runtime_network_status_barrier = Some(RuntimeNetworkStatusBarrier {
            status,
            local_reached: false,
            actual_control_tick: None,
        });
        // CheckStatusReached tests the target before SetRunning(true). This is
        // observable for Go requested from a committed Pause: current-tick
        // packets received while stopped are not promoted before the node
        // reaches and acknowledges the Go barrier.
        if self.check_runtime_network_status_reached() == RuntimeStatusReachOutcome::NotReached {
            if let Some(network) = self.network.as_ref() {
                network.reset_client_performance();
            }
            self.network_control_running = true;
            self.refresh_network_client_next_control_ticks();
        }
        true
    }

    fn change_runtime_network_status(
        &mut self,
        status: clonk_network::NetworkStatus,
    ) -> Result<()> {
        self.network
            .as_ref()
            .ok_or_else(|| anyhow!("runtime network is unavailable"))?
            .change_status(status)
            .map_err(|error| anyhow!(error))?;
        if let Some(clock) = self.network_control_clock.as_mut() {
            clock.set_target_tick(Some(status.target_tick));
        }
        // The host session also echoes every authoritative status request.
        // Arm synchronously so a sync control that opens a follow-up barrier
        // cannot be overwritten by the old commit's tail.
        self.arm_runtime_network_status_barrier(status);
        Ok(())
    }

    pub(crate) fn check_runtime_network_status_reached(&mut self) -> RuntimeStatusReachOutcome {
        let Some(pending) = self
            .runtime_network_status_barrier
            .filter(|pending| !pending.local_reached)
        else {
            return RuntimeStatusReachOutcome::NotReached;
        };
        let Some(clock) = self.network_control_clock else {
            return RuntimeStatusReachOutcome::NotReached;
        };
        let current_control_tick = clock.current_tick();
        if current_control_tick < pending.status.target_tick
            || !self
                .engine
                .frame()
                .is_multiple_of(clock.control_rate() as u64)
        {
            return RuntimeStatusReachOutcome::NotReached;
        }
        let Ok(expected_tick) = Tick::try_from(current_control_tick) else {
            return RuntimeStatusReachOutcome::NotReached;
        };
        // Native reaches only after the current *executable* control queue is
        // empty. CheckCompleteCtrl advances readiness only after PreExecute;
        // raw packets blocked on a resource do not keep the barrier running.
        // While committed Pause is stopped, newly received controls likewise
        // have not been promoted to CtrlReady yet.
        let current_control_ready = if self.network_control_running {
            let network_ticks = &self.network_ticks;
            let admission_resources = &mut self.admission_resources;
            let control_clients = &self.control_clients;
            let aborted_player_joins = &self.aborted_player_resource_joins;
            network_ticks.exact_is_ready_if(expected_tick, |controls| {
                preflight_admission_resources(
                    admission_resources,
                    control_clients,
                    controls,
                    aborted_player_joins,
                )
            })
        } else {
            false
        };
        if current_control_ready {
            return RuntimeStatusReachOutcome::NotReached;
        }

        // OnStatusReached stops control before the host reports local reach
        // or the client sends PID_StatusAck.
        self.network_control_running = false;
        let current_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        let role = self.runtime_network_role();
        let reached = match role {
            RuntimeNetworkRole::Host => self
                .network
                .as_ref()
                .map(|network| network.status_reached(pending.status, current_control_tick)),
            RuntimeNetworkRole::Client => self.network.as_mut().map(|network| {
                network.acknowledge_expected_status_at_frame(
                    pending.status,
                    current_control_tick,
                    current_frame,
                )
            }),
            RuntimeNetworkRole::Offline | RuntimeNetworkRole::Ambiguous => None,
        };
        match reached {
            Some(Ok(())) => {
                if let Some(active) = self.runtime_network_status_barrier.as_mut() {
                    if same_runtime_network_status_barrier(active.status, pending.status) {
                        active.local_reached = true;
                        active.actual_control_tick = Some(current_control_tick);
                        if role == RuntimeNetworkRole::Client {
                            // A client acknowledges the ControlTick it actually
                            // reached, which may be later than the request.
                            active.status.target_tick = current_control_tick;
                        }
                    }
                }
                RuntimeStatusReachOutcome::Reported
            }
            Some(Err(error)) => {
                tracing::error!(%error, "failed to report runtime network status arrival");
                RuntimeStatusReachOutcome::ReportFailed
            }
            None => {
                tracing::error!("cannot report runtime network status for ambiguous role");
                RuntimeStatusReachOutcome::ReportFailed
            }
        }
    }

    pub(crate) fn network_control_pacing(&mut self) -> NetworkControlPacing {
        if self.mode != AppMode::Running
            || self.network.is_none()
            || !self.network_control_running
            || self.game_over_dialog.is_some()
        {
            return NetworkControlPacing::default();
        }

        let expected_tick = self.expected_network_control_tick();
        let behind = {
            let network_ticks = &self.network_ticks;
            let admission_resources = &mut self.admission_resources;
            let control_clients = &self.control_clients;
            let aborted_player_joins = &self.aborted_player_resource_joins;
            network_ticks.contiguous_ready_behind_if(expected_tick, |controls| {
                preflight_admission_resources(
                    admission_resources,
                    control_clients,
                    controls,
                    aborted_player_joins,
                )
            })
        };
        // NetworkControlClock advances as soon as a cadence-frame control is
        // consumed. C++ retains that ControlTick until the new FrameCounter
        // reaches the next rate boundary, so its inclusive GetBehind still
        // counts the just-executed tick during the intervening frames.
        let behind = match self.network_control_clock {
            Some(clock)
                if Tick::try_from(clock.current_tick()).is_ok()
                    && !self
                        .engine
                        .frame()
                        .is_multiple_of(clock.control_rate() as u64) =>
            {
                behind.saturating_add(1)
            }
            _ => behind,
        };
        let overflow = behind > NETWORK_CONTROL_OVERFLOW_LIMIT;
        let skip_render = if overflow && behind >= NETWORK_RENDER_SKIP_BEHIND {
            let divisor = behind.saturating_add(15) / 20;
            !self.engine.frame().is_multiple_of(u64::from(divisor))
        } else {
            false
        };
        NetworkControlPacing {
            behind,
            overflow,
            skip_render,
        }
    }

    pub(crate) fn issue_unjoined_joins_for_client(&mut self, client_id: i32) {
        let resources = &self.admission_resources;
        let joins = self
            .control_player_infos
            .issue_unjoined_players(client_id, |core| {
                resources.complete_path(core.id).and_then(|path| {
                    clonk_engine::LegacyCString::from_bytes(path_to_legacy_bytes(path))
                })
            });
        let tick = self.local_control_submission_tick();
        for join in joins {
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_join_player(tick, join))
            {
                tracing::error!(%error, "failed to submit synchronized JoinPlayer");
            }
        }
    }

    fn client_start_resource_display_name(&self, pending: &PendingClientStartResource) -> String {
        match pending.role {
            ClientStartResourceRole::Scenario => {
                self.runtime_resource_text("IDS_NET_RES_SCENARIO", "Scenario")
            }
            ClientStartResourceRole::Dynamic => {
                self.runtime_resource_text("IDS_NET_RES_DYNAMIC", "Dynamic")
            }
            ClientStartResourceRole::GameResource { .. } => {
                let filename = pending.core.filename.to_string_lossy().into_owned();
                let basename = filename
                    .rsplit(['/', '\\'])
                    .next()
                    .filter(|basename| !basename.is_empty())
                    .unwrap_or(filename.as_str());
                format!(
                    "{}: {basename}",
                    self.runtime_resource_text("IDS_DLG_DEFINITION", "Object Definition")
                )
            }
        }
    }

    pub(crate) fn wait_for_client_start_resource(
        &mut self,
        pending: PendingClientStartResource,
    ) -> Result<(), String> {
        let display_name = self.client_start_resource_display_name(&pending);
        if matches!(
            self.admission_resources.status(pending.core.id),
            Some(AdmissionResourceState::Unavailable(_))
        ) {
            return self
                .finish_startup_network_failure(
                    StartupNetworkPurpose::Join,
                    format!("Unable to retrieve {display_name}."),
                )
                .map_err(|error| error.to_string());
        }
        self.begin_blocking_resource_wait_at(
            BlockingResourceScope::ClientStart,
            pending.core.id,
            None,
            display_name,
            Instant::now(),
        )
        .map_err(|error| error.to_string())
    }

    fn blocking_resource_wait_message(&self, display_name: &str) -> String {
        let template = self.runtime_resource_text("IDS_NET_WAITFORRES", "Waiting for %s...");
        format_resource_string(template, &[display_name])
    }

    pub(crate) fn begin_blocking_resource_wait_at(
        &mut self,
        scope: BlockingResourceScope,
        resource_id: i32,
        player_info_id: Option<i32>,
        display_name: String,
        now: Instant,
    ) -> Result<(), EngineError> {
        let present_percent = self
            .admission_resources
            .present_percent
            .get(&resource_id)
            .copied()
            .unwrap_or_default()
            .min(100);
        let same_wait = self
            .blocking_resource_wait
            .as_ref()
            .is_some_and(|wait| wait.scope == scope && wait.resource_id == resource_id);
        if !same_wait {
            if let Some(previous) = self.blocking_resource_wait.take() {
                self.dismiss_blocking_resource_wait_dialog(previous.scope, previous.resource_id);
            }
            let wait = BlockingResourceWait::new_at(
                scope,
                resource_id,
                player_info_id,
                display_name.clone(),
                present_percent,
                now,
            );
            let dialog = clonk_frontend::progress_dialog::ProgressDialogState::new(
                self.blocking_resource_wait_message(&display_name),
                self.runtime_resource_text("IDS_NET_CAPTION", "Network"),
                present_percent,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(3),
            )
            .into_message_dialog();
            self.push_message_dialog(
                dialog,
                MessageDialogContinuation::BlockingResourceWait { scope, resource_id },
            )?;
            self.blocking_resource_wait = Some(wait);
        } else {
            if let Some(wait) = self.blocking_resource_wait.as_mut() {
                wait.display_name = display_name;
            }
            self.update_blocking_resource_wait_dialog(scope, resource_id, present_percent);
        }
        Ok(())
    }

    fn finish_blocking_resource_wait(&mut self, resource_id: i32) {
        let Some(wait) = self
            .blocking_resource_wait
            .take_if(|wait| wait.resource_id == resource_id)
        else {
            return;
        };
        self.dismiss_blocking_resource_wait_dialog(wait.scope, wait.resource_id);
    }

    pub(crate) fn clear_blocking_resource_wait(&mut self) {
        self.aborted_player_resource_joins.clear();
        let Some(wait) = self.blocking_resource_wait.take() else {
            return;
        };
        self.dismiss_blocking_resource_wait_dialog(wait.scope, wait.resource_id);
    }

    pub(crate) fn cancel_blocking_resource_wait(
        &mut self,
        scope: BlockingResourceScope,
        resource_id: i32,
    ) -> Result<(), EngineError> {
        let Some(wait) = self
            .blocking_resource_wait
            .take_if(|wait| wait.scope == scope && wait.resource_id == resource_id)
        else {
            return Ok(());
        };
        match wait.scope {
            BlockingResourceScope::ClientStart => self.finish_startup_network_failure(
                StartupNetworkPurpose::Join,
                format!("Waiting for {} was aborted.", wait.display_name),
            ),
            BlockingResourceScope::PlayerJoin => {
                // RetrieveRes cancellation fails this one caller, not the
                // backend transfer. Bypass this synchronized JoinPlayer once;
                // later callers for the same loading resource still wait.
                if let Some(info_id) = wait.player_info_id {
                    self.aborted_player_resource_joins
                        .insert((wait.resource_id, info_id));
                }
                Ok(())
            }
        }
    }

    pub(crate) fn poll_blocking_resource_wait_at(
        &mut self,
        now: Instant,
    ) -> Result<(), EngineError> {
        let Some((scope, resource_id, previous_percent)) = self
            .blocking_resource_wait
            .as_ref()
            .map(|wait| (wait.scope, wait.resource_id, wait.present_percent()))
        else {
            return Ok(());
        };
        let present_percent = self
            .admission_resources
            .present_percent
            .get(&resource_id)
            .copied()
            .unwrap_or(previous_percent)
            .min(100);
        let timed_out = self
            .blocking_resource_wait
            .as_mut()
            .is_some_and(|wait| wait.observe_at(present_percent, now));
        self.update_blocking_resource_wait_dialog(scope, resource_id, present_percent);
        if !timed_out {
            return Ok(());
        }

        let Some(wait) = self.blocking_resource_wait.take() else {
            return Ok(());
        };
        self.dismiss_blocking_resource_wait_dialog(wait.scope, wait.resource_id);
        let template =
            self.runtime_resource_text("IDS_NET_ERR_RESTIMEOUT", "Waiting for %s: Timeout!");
        let message = format_resource_string(template, &[&wait.display_name]);
        tracing::error!(
            resource_id = wait.resource_id,
            resource = %wait.display_name,
            "blocking network resource retrieval timed out"
        );
        match wait.scope {
            BlockingResourceScope::ClientStart => {
                self.finish_startup_network_failure(StartupNetworkPurpose::Join, message)
            }
            BlockingResourceScope::PlayerJoin => {
                if let Some(info_id) = wait.player_info_id {
                    self.aborted_player_resource_joins
                        .insert((wait.resource_id, info_id));
                }
                let caption = self.runtime_resource_text("IDS_DLG_LOG", "Error Log");
                self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                        message,
                        caption,
                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    ),
                    MessageDialogContinuation::None,
                )
            }
        }
    }

    pub(crate) fn prepare_client_network_scenario_if_ready(&mut self) -> Result<(), EngineError> {
        if let Err(error) = self.try_prepare_client_network_scenario() {
            tracing::error!(%error, "failed to prepare client network scenario");
            return self.finish_scenario_loading_failure(
                format!("Unable to prepare network scenario: {error}"),
                true,
            );
        }
        Ok(())
    }

    pub(crate) fn try_prepare_client_network_scenario(&mut self) -> Result<(), String> {
        let Some(status) = self.pending_client_start_status else {
            return Ok(());
        };
        let Some(join_data) = self.pending_network_join_data.clone() else {
            return Ok(());
        };
        if self.lobby_preload_task.is_some() {
            return Ok(());
        }
        let Some(NetworkMode::Client(settings)) = self.network_mode.as_ref() else {
            return Ok(());
        };
        let resource_directory = settings.resource_directory.clone();
        let maker = settings.player_name.clone();
        if self.client_combined_scenario_path.is_none() {
            let resources = match resolve_client_scenario_resources(&join_data, |core| {
                self.admission_resources
                    .complete_path(core.id)
                    .map(Path::to_path_buf)
            }) {
                Ok(resources) => resources,
                Err(pending) => {
                    self.wait_for_client_start_resource(pending)?;
                    return Ok(());
                }
            };
            let filename = format!("Combined{}.c4s", join_data.client_id);
            let packed = compose_client_network_scenario(&resources, &filename, &maker)
                .map_err(|error| error.to_string())?;
            fs::create_dir_all(&resource_directory).map_err(|error| {
                format!("failed to create {}: {error}", resource_directory.display())
            })?;
            let combined_path = resource_directory.join(filename);
            fs::write(&combined_path, packed)
                .map_err(|error| format!("failed to write {}: {error}", combined_path.display()))?;
            self.network
                .as_ref()
                .ok_or_else(|| "client network disappeared during scenario merge".to_string())?
                .remove_client_resource(join_data.dynamic.id)
                .map_err(|error| {
                    format!(
                        "failed to retire merged dynamic resource {}: {error}",
                        join_data.dynamic.id
                    )
                })?;
            self.client_combined_preload_file.clear();
            self.client_combined_scenario_path = Some(combined_path);
        }
        if self.loading_state.is_some() {
            return Ok(());
        }
        let combined_path = self
            .client_combined_scenario_path
            .clone()
            .expect("combined path was installed above");
        // C4Game::InitGameFirstPart blocks on Parameters.GameRes.RetrieveFiles
        // before any InitGame work proceeds (C4Game.cpp:2576-2580); the
        // GraphicsResource refresh below registers exactly these synchronized
        // definition roots, so a preloaded scenario waits here as well.
        let game_resources = match resolve_client_game_resources(&join_data, |core| {
            self.admission_resources
                .complete_path(core.id)
                .map(Path::to_path_buf)
        }) {
            Ok(resources) => resources,
            Err(pending) => {
                self.wait_for_client_start_resource(pending)?;
                return Ok(());
            }
        };
        let mut definition_groups = Vec::new();
        let mut material_groups = Vec::new();
        for resource in &game_resources {
            let target = match resource.core.resource_type {
                value if value == clonk_network::HostResourceType::Definitions as u8 => {
                    &mut definition_groups
                }
                value if value == clonk_network::HostResourceType::Material as u8 => {
                    &mut material_groups
                }
                _ => continue,
            };
            target.push(Group::open(&resource.path).map_err(|error| {
                format!(
                    "failed to open synchronized game resource {} at {}: {error}",
                    resource.core.id,
                    resource.path.display()
                )
            })?);
        }
        let preloaded_scenario = self
            .lobby_preload_artifact
            .as_mut()
            .filter(|artifact| artifact.scenario_path == combined_path)
            .and_then(|artifact| artifact.client.as_mut())
            .filter(|client| {
                client.client_id == join_data.client_id
                    && client.dynamic_resource_id == join_data.dynamic.id
                    && client.random_seed == u64::from(join_data.parameters.random_seed as u32)
            })
            .and_then(|client| client.scenario.take());
        let preloaded_first_part = preloaded_scenario.is_some();
        if let Some(scenario_data) =
            preloaded_scenario.filter(|scenario| !scenario.uses_map_player_extend())
        {
            return self.install_prepared_client_network_scenario(
                status,
                join_data,
                combined_path,
                scenario_data,
                None,
                definition_groups,
                true,
            );
        }
        let resolver_paths = cached_app_paths().ok();
        let scenario_group = Group::open(&combined_path).map_err(|error| {
            format!(
                "failed to open combined scenario {} for graphics lookup: {error}",
                combined_path.display()
            )
        })?;
        let graphics_groups = InstallDefinitionResolver::new(resolver_paths.clone())
            .resolve_graphics_groups_with_definition_roots(&scenario_group, &definition_groups)
            .map_err(|error| format!("failed to resolve client graphics resources: {error}"))?;
        let languages = startup_language_sequence(resolver_paths.as_deref());
        let language_packs = resolver_paths
            .as_deref()
            .map(classic_language_packs)
            .unwrap_or_default();
        let random_seed = u64::from(join_data.parameters.random_seed as u32);
        let initial_game_source = read_optional_initial_network_game_source(&scenario_group)
            .map_err(|error| {
                format!(
                    "network scenario {} has an unreadable Game.txt: {error}",
                    combined_path.display()
                )
            })?;
        let initial_game_state = initial_game_source
            .as_deref()
            .map(clonk_engine::parse_initial_network_game_data)
            .unwrap_or_else(|| clonk_engine::InitialNetworkGameData {
                control_tick: join_data.start_control_tick,
                ..clonk_engine::InitialNetworkGameData::default()
            });
        let startup_player_count = startup_player_count_for_init(
            initial_game_state.frame,
            Some(join_data.parameters.startup_player_count),
            Some(
                i32::try_from(self.control_player_infos.nonremoved_player_count())
                    .unwrap_or(i32::MAX),
            ),
        )
        .unwrap_or(join_data.parameters.startup_player_count);
        let worker_path = combined_path.clone();
        let worker_definition_groups = definition_groups.clone();
        let worker_material_groups = material_groups.clone();
        let scenario_title = legacy_presentation_text(join_data.parameters.title.as_bytes());
        let spawn_failure_title = scenario_title.clone();
        let (sender, receiver) = mpsc::channel();
        self.begin_client_network_scenario_loading(
            status,
            join_data,
            combined_path,
            receiver,
            Some(material_groups),
            definition_groups,
            preloaded_first_part,
        )?;
        let spawn_failure_sender = sender.clone();
        thread::Builder::new()
            .name("NetworkScenarioLoad".to_string())
            .spawn(move || {
                let mut reporter = ScenarioLoadingReporter::new(sender);
                let result =
                    Scenario::load_network_from_path_with_languages_and_seed_and_packs_and_startup_player_count_and_progress(
                        &worker_path,
                        &worker_definition_groups,
                        &worker_material_groups,
                        &graphics_groups,
                        &languages,
                        random_seed,
                        &language_packs,
                        startup_player_count,
                        |progress, line| {
                            let visible = if preloaded_first_part {
                                progress >= 88
                            } else {
                                progress >= 8
                            };
                            if visible {
                                reporter.report(progress, line);
                            }
                        },
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|scenario| {
                        validate_client_network_scenario(&scenario)?;
                        scenario
                            .validate_initial_network_game_data(&initial_game_state)
                            .map_err(|error| format!("invalid network Game.txt: {error}"))?;
                        Ok(scenario)
                    })
                    .map_err(|error| format!("Failed to load {scenario_title}: {error}"));
                reporter.send(ScenarioLoadingEvent::Finished(result));
            })
            .map(|_| ())
            .or_else(|error| {
                let message = format!("Failed to launch {spawn_failure_title} loader: {error}");
                spawn_failure_sender
                    .send(ScenarioLoadingEvent::Finished(Err(message)))
                    .map_err(|send_error| {
                        format!("failed to report network loader launch failure: {send_error}")
                    })
            })
    }

    fn install_prepared_client_network_scenario(
        &mut self,
        status: clonk_network::NetworkStatus,
        join_data: clonk_network::JoinDataEnvelope,
        combined_path: PathBuf,
        scenario_data: Scenario,
        material_groups: Option<Vec<Group>>,
        definition_groups: Vec<Group>,
        preloaded_first_part: bool,
    ) -> Result<(), String> {
        validate_client_network_scenario(&scenario_data)?;
        let (sender, receiver) = mpsc::channel();
        let _ = sender.send(ScenarioLoadingEvent::Finished(Ok(scenario_data)));
        self.begin_client_network_scenario_loading(
            status,
            join_data,
            combined_path,
            receiver,
            material_groups,
            definition_groups,
            preloaded_first_part,
        )
    }

    fn begin_client_network_scenario_loading(
        &mut self,
        status: clonk_network::NetworkStatus,
        join_data: clonk_network::JoinDataEnvelope,
        combined_path: PathBuf,
        receiver: Receiver<ScenarioLoadingEvent>,
        material_groups: Option<Vec<Group>>,
        definition_groups: Vec<Group>,
        preloaded_first_part: bool,
    ) -> Result<(), String> {
        let scenario_group = Group::open(&combined_path).map_err(|error| {
            format!(
                "failed to open combined scenario {} for runtime data: {error}",
                combined_path.display()
            )
        })?;
        let initial_game_source = read_optional_initial_network_game_source(&scenario_group)
            .map_err(|error| {
                format!(
                    "network scenario {} has an unreadable Game.txt: {error}",
                    combined_path.display()
                )
            })?;
        // A missing component is not the same as compiling an empty named
        // tree: C4Game::Compile simply returns and retains the live state.
        // HandleJoinData has already initialized ControlTick from the host,
        // so seed that one post-InitSystem value into the otherwise-default
        // runtime snapshot before staging it (C4Network2.cpp:1605-1609).
        let initial_game_state = initial_game_source
            .as_deref()
            .map(clonk_engine::parse_initial_network_game_data)
            .unwrap_or_else(|| clonk_engine::InitialNetworkGameData {
                control_tick: join_data.start_control_tick,
                ..clonk_engine::InitialNetworkGameData::default()
            });
        initial_game_state
            .validate_runtime_application()
            .map_err(|error| format!("invalid network Game.txt: {error}"))?;
        if let Some(material_groups) = material_groups {
            self.network_material_resource_groups = Some(material_groups);
        }
        self.fade_out_game_music();
        let random_seed = u64::from(join_data.parameters.random_seed as u32);
        let title = legacy_presentation_text(join_data.parameters.title.as_bytes());
        let scenario = FrontendScenario {
            identifier: combined_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("Combined{}.c4s", join_data.client_id)),
            title: if title.is_empty() {
                "Network game".to_string()
            } else {
                title
            },
            description: None,
            kind: ScenarioKind::Scenario,
            is_editable: false,
            is_playable: true,
            mission_access: None,
            path: Some(combined_path.clone()),
            source_paths: vec![combined_path],
            root_label: None,
            preview: None,
            title_picture: None,
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
            author: None,
            version: None,
            local_only: None,
            allow_user_change: None,
            definition_modules: Vec::new(),
        };
        // A joining client runs the same InitGame → GraphicsResource::Init
        // pass over the join-synchronized group set (C4Game.cpp:2432-2450;
        // C4GraphicsResource.cpp:278-292). Stage that resolution so the
        // loading refresh applies it through the identical typed failure
        // gate local loading uses. The classic resource environment exists
        // only with discovered application paths; path-less state-only
        // fixtures keep their startup resources.
        let refresh = match self.app_paths.as_ref() {
            Some(refresh_paths) => Some(
                resolve_client_network_loading_refresh(
                    &self.assets,
                    refresh_paths,
                    &scenario,
                    &scenario_group,
                    &definition_groups,
                )
                .map_err(|error| {
                    format!("failed to resolve the client GraphicsResource refresh: {error:#}")
                })?,
            ),
            None => None,
        };
        let team_registry = runtime_teams_from_join_snapshot(&join_data.parameters.teams);
        let mut loading_state = ScenarioLoadingState::from_network_receiver(
            scenario,
            receiver,
            status,
            // Keep Game.Parameters.RestorePlayerInfos distinct from the
            // dynamic-local SavePlayerInfos consumed exclusively by the
            // NetworkRuntimeJoin branch in C4Game::InitPlayers.
            player_info_list_entries(&join_data.parameters.restore_player_infos).collect(),
            Some(initial_game_state),
            random_seed,
            join_data.parameters.use_fair_crew,
            join_data.parameters.fair_crew_strength,
            join_data.parameters.fair_crew_forced,
            join_data.parameters.allow_debug,
            join_data.parameters.auto_frame_skip,
            synchronized_rule_goal_lists(&join_data.parameters),
            synchronized_team_configuration(&join_data.parameters),
            team_registry,
        );
        loading_state
            .prepared_go
            .as_mut()
            .expect("client loading retains its Go boundary")
            .pending_client_runtime_join = Some(PendingClientRuntimeJoinLoading {
            local_client_id: join_data.client_id,
            packet_restore_player_infos: join_data.parameters.restore_player_infos.clone(),
        });
        if let Some(refresh) = refresh {
            loading_state.refreshed_resources = refresh.resources;
            loading_state.refreshed_tooltip_font = refresh.tooltip_font;
            loading_state.refreshed_native_font_source = refresh.native_font_source;
            loading_state.refreshed_global_gui_failures = Some(refresh.failures);
            loading_state.refreshed_gui_sheet_overrides = Some(refresh.overrides);
            loading_state.refresh_requested = true;
        }
        self.loading_state = Some(loading_state);
        if preloaded_first_part {
            // Successful client preloading makes InitGameFirstPart return
            // before its RetrieveScenario 6/7 branch. InitGame then resumes
            // after GraphicsResource::Init at 10
            // (src/C4Game.cpp:2414-2452,2551-2556).
            self.apply_scenario_loader_frame(10, None);
        } else {
            // RetrieveScenario and synchronized GameRes retrieval have
            // completed; C++ publishes 7 before InitScriptEngine
            // (src/C4Game.cpp:2575-2598).
            self.apply_scenario_loader_frame(7, None);
        }
        retain_client_league_server_name(
            self.network_mode.as_mut(),
            &join_data.parameters.league_address,
        );
        self.pending_network_join_data = None;
        self.mode = AppMode::Loading;
        Ok(())
    }

    pub(crate) fn finalize_client_network_scenario_loading(
        &mut self,
        scenario_data: &Scenario,
        combined_path: &Path,
    ) -> Result<(), String> {
        if !matches!(self.network_mode, Some(NetworkMode::Client(_))) {
            return Ok(());
        }
        validate_client_network_scenario(scenario_data)?;
        if let Some(initial_game_state) = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .and_then(|prepared| prepared.initial_game_data.as_ref())
        {
            scenario_data
                .validate_initial_network_game_data(initial_game_state)
                .map_err(|error| format!("invalid network Game.txt: {error}"))?;
        }
        let Some(runtime_join) = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .and_then(|prepared| prepared.pending_client_runtime_join.clone())
        else {
            return Ok(());
        };
        let scenario_group = Group::open(combined_path).map_err(|error| {
            format!(
                "failed to open combined scenario {} for runtime data: {error}",
                combined_path.display()
            )
        })?;
        let network_runtime_join = scenario_data
            .lobby_metadata()
            .is_some_and(|metadata| metadata.head().allows_network_runtime_join());
        // HandleJoinData has copied Game.Parameters already. Runtime joins
        // instead consume the combined scenario's freshly saved local list
        // after loading identifies that exclusive branch
        // (src/C4Game.cpp:2805-2850).
        let resolver_paths = cached_app_paths().ok();
        let languages = startup_language_sequence(resolver_paths.as_deref());
        let language_packs = resolver_paths
            .as_deref()
            .map(classic_language_packs)
            .unwrap_or_default();
        let restore_player_infos = client_network_restore_player_infos(
            network_runtime_join,
            &scenario_group,
            &runtime_join.packet_restore_player_infos,
            &languages,
            &language_packs,
        );
        let runtime_join_players = if network_runtime_join {
            restore_player_infos
                .clients
                .iter()
                .flat_map(|client| {
                    client
                        .players
                        .iter()
                        .filter(|info| info.is_joined())
                        .map(|info| clonk_engine::RuntimeJoinPlayerSource {
                            client_id: client.client_id,
                            info: info.clone(),
                            load_unnamed_portraits: client.client_id
                                == runtime_join.local_client_id,
                        })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if let Some(prepared) = self
            .loading_state
            .as_mut()
            .and_then(|loading| loading.prepared_go.as_mut())
        {
            prepared.network_runtime_join = network_runtime_join;
            prepared.restore_player_infos =
                player_info_list_entries(&restore_player_infos).collect();
            prepared.runtime_join_players = runtime_join_players;
            prepared.pending_client_runtime_join = None;
        }
        Ok(())
    }

    pub(crate) fn freeze_configured_client_players_for_game(&mut self) -> Result<()> {
        self.configured_client_player_selection = self
            .app_paths
            .as_ref()
            .map(|paths| {
                snapshot_effective_client_player_selection(paths, &self.classic_command_line)
            })
            .transpose()?;
        Ok(())
    }

    pub(crate) fn submit_initial_client_player_info(
        &mut self,
        client_id: i32,
        league_server_name: String,
    ) -> LeaguePlayerAuthStatus {
        let authenticate_players = self.network_is_league;
        let Some(network) = self.network.as_ref() else {
            return LeaguePlayerAuthStatus::Completed(false);
        };
        let mut completed_resources = Vec::new();
        let empty_request = || clonk_network::PlayerInfoUpdateRequest {
            client_id,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: Vec::new(),
        };
        let request = self
            .app_paths
            .as_ref()
            .zip(self.configured_client_player_selection.as_ref())
            .map(|(paths, selection)| {
                let configured = load_snapshotted_client_players(paths, selection);
                publish_initial_configured_client_players(client_id, &configured, |publication| {
                    let source_path = publication.source_path.clone();
                    network
                        .publish_client_player_resource(
                            clonk_network::ClientPlayerResourceRequest {
                                source_path: publication.source_path,
                                wire_name: publication.wire_name,
                                group_maker: publication.group_maker,
                            },
                        )
                        .inspect(|core| {
                            completed_resources.push((core.clone(), source_path.clone()));
                        })
                        .inspect_err(|error| {
                            tracing::warn!(
                                path = %source_path.display(),
                                %error,
                                "failed to publish configured network player"
                            );
                        })
                })
            })
            .unwrap_or_else(empty_request);
        for (core, path) in completed_resources {
            self.admission_resources.ensure_by_core(&core);
            self.admission_resources.mark_complete(core.id, path);
        }
        if authenticate_players {
            let continuation = LeaguePlayerAuthContinuation::InitialClient {
                request,
                index: 0,
                server_name: league_server_name,
            };
            return match self.continue_league_player_auth(continuation) {
                Ok(status) => status,
                Err(error) => {
                    tracing::error!(%error, "failed to open league player authentication");
                    LeaguePlayerAuthStatus::Completed(false)
                }
            };
        }
        LeaguePlayerAuthStatus::Completed(match network.submit_player_info_update(request) {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(%error, "failed to submit initial PlayerInfo");
                false
            }
        })
    }

    fn commit_host_player_info_admission(
        &mut self,
        admission: clonk_engine::PlayerInfoAdmission,
    ) -> Result<()> {
        let clonk_engine::PlayerInfoAdmission {
            updated_existing,
            admitted,
            joined_player_team_updates,
        } = admission;
        for update in &joined_player_team_updates {
            self.engine.apply_admitted_player_team_update(
                update.info_id,
                update.team,
                update.color,
            )?;
        }
        for info in updated_existing {
            self.broadcast_and_preexecute_player_info(info, true, false)?;
        }
        self.broadcast_and_preexecute_player_info(admitted, false, true)
    }

    fn finalize_host_player_info_admission(
        &mut self,
        mut admission: clonk_engine::PlayerInfoAdmission,
    ) -> Option<clonk_engine::PlayerInfoAdmission> {
        // Native resets gains after ID/team/attribute normalization for every
        // request, before its league/lobby branch.
        self.control_player_infos
            .reset_projected_gains_for_admission(&mut admission);
        if !self.network_is_league {
            return Some(admission);
        }
        // League player-info requests outside GS_Lobby are rejected in full;
        // they are not admitted without authentication.
        if !self.league_player_auth_lobby_active() {
            return None;
        }
        let network = self.network.as_ref()?;
        let league = clonk_engine::LegacyCString::from_bytes(self.network_league_name.clone())
            .unwrap_or_default();
        let refusal_template = self.runtime_resource_text(
            "IDS_MSG_LEAGUEJOINREFUSED",
            "League server has refused the join of player %s: %s",
        );
        let mut check_errors = Vec::new();
        retain_player_infos_with_cpp_swap_remove(&mut admission.admitted.players, |player| {
            if self.control_player_infos.get(player.id).is_some() {
                return true;
            }
            match network.check_league_player(&league, player) {
                Ok(network::LeaguePlayerCheck::Accepted) => true,
                Ok(network::LeaguePlayerCheck::Unavailable) => false,
                Ok(network::LeaguePlayerCheck::Rejected(message)) => {
                    let player_name = legacy_presentation_text(player.name.as_bytes());
                    let message = legacy_presentation_text(message.as_bytes());
                    check_errors.push(format_resource_string(
                        refusal_template.clone(),
                        &[&player_name, &message],
                    ));
                    false
                }
                Err(error) => {
                    tracing::warn!(player_id = player.id, %error, "league player check failed");
                    check_errors.push(error.to_string());
                    false
                }
            }
        });
        for message in check_errors {
            if let Err(error) = self.push_league_player_check_error(message) {
                tracing::error!(%error, "failed to show league player-check error");
            }
        }
        Some(admission)
    }

    pub(crate) fn submit_runtime_network_player(&mut self, file: &str) -> Result<(), String> {
        self.submit_network_player_path(Path::new(file), file, true)
    }

    pub(crate) fn offline_local_client_id(&self) -> i32 {
        self.engine
            .player(self.local_owner)
            .map(|player| player.at_client().get())
            .filter(|client_id| self.control_clients.contains(*client_id))
            .or_else(|| {
                self.control_clients
                    .snapshot()
                    .into_iter()
                    .find(|client| client.activated && !client.observer)
                    .map(|client| client.client_id)
            })
            .unwrap_or(0)
    }

    pub(crate) fn submit_runtime_network_player_path(
        &mut self,
        source_path: &Path,
        wire_filename: &str,
    ) -> Result<(), String> {
        self.submit_network_player_path(source_path, wire_filename, true)
    }

    pub(crate) fn submit_network_player_path(
        &mut self,
        source_path: &Path,
        wire_filename: &str,
        require_activated_client: bool,
    ) -> Result<(), String> {
        let fallback_group_maker = || {
            self.configured_client_player_selection
                .as_ref()
                .map(|selection| selection.group_maker().clone())
                .or_else(|| {
                    clonk_engine::LegacyCString::from_bytes(self.player_name.as_bytes().to_vec())
                })
                .ok_or_else(|| "network player name contains an interior NUL".to_string())
        };
        let (host, group_maker) = match self.network_mode.as_ref() {
            Some(NetworkMode::Host(settings)) => (
                true,
                match settings.prepared.as_ref() {
                    Some(prepared) => prepared.host_config().group_maker.clone(),
                    None => fallback_group_maker()?,
                },
            ),
            Some(NetworkMode::Client(settings)) => (false, settings.group_maker.clone()),
            None => return Err("runtime joining requires a network session".to_string()),
        };
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| "network session is unavailable".to_string())?;
        let client_id = i32::try_from(network.local_client_id())
            .map_err(|_| "local client ID exceeds the PlayerInfo wire field".to_string())?;
        if require_activated_client && !self.control_clients.is_activated(client_id) {
            return Err("network client is not active".to_string());
        }

        let source_path = source_path.to_path_buf();
        let player_file = PlayerFile::load_from_path(&source_path)
            .map_err(|error| format!("failed to load {}: {error}", source_path.display()))?;
        let wire_name =
            clonk_engine::LegacyCString::from_bytes(clonk_script::c4_string_bytes(wire_filename))
                .ok_or_else(|| "player filename contains an interior NUL".to_string())?;
        let selected =
            SelectedClientPlayer::new(source_path.clone(), wire_name.clone(), player_file);
        let alternate_color = selected.alternate_color();

        // LoadFromLocalFile publishes/reuses NRT_Player before JoinLocalPlayer
        // handles its CIF_AddPlayers request. Hosts process that request
        // directly; clients send it to the host (src/C4PlayerInfo.cpp:70-104;
        // src/C4Network2Players.cpp:78-137).
        let publication = clonk_network::ClientPlayerResourceRequest {
            source_path: source_path.clone(),
            wire_name,
            group_maker,
        };
        let resource = if host {
            network.publish_host_player_resource(publication)
        } else {
            network.publish_client_player_resource(publication)
        }
        .map_err(|error| error.to_string())?;
        let request = selected
            .runtime_add_player_info_update(client_id, resource)
            .map_err(|error| error.to_string())?;
        // A locally published resource keeps its original file for this
        // process's JoinPlayer while the backend serves the optimized
        // standalone to peers
        // (src/C4Network2Res.cpp:409-424,1168-1189;
        // src/C4Network2Players.cpp:353-382).
        let resource_core = request
            .players
            .first()
            .and_then(|player| player.resource.as_ref())
            .cloned()
            .ok_or_else(|| "runtime player request has no resource".to_string())?;
        let alternate_resource_id = resource_core.id;
        self.admission_resources
            .register_lobby_resource(&resource_core);
        self.admission_resources
            .mark_complete(resource_core.id, source_path);
        if self.network_is_league {
            if request.players.is_empty() {
                return Err("runtime player request has no player".to_string());
            }
            let server_name = self.current_league_server_name();
            return match self.continue_league_player_auth(
                LeaguePlayerAuthContinuation::RuntimePlayer {
                    request,
                    index: 0,
                    server_name,
                    host,
                    alternate_resource_id,
                    alternate_color,
                },
            ) {
                Ok(LeaguePlayerAuthStatus::Pending)
                | Ok(LeaguePlayerAuthStatus::Completed(true)) => Ok(()),
                Ok(LeaguePlayerAuthStatus::Completed(false)) => {
                    Err("league player authentication was rejected".to_string())
                }
                Err(error) => Err(format!("league player authentication failed: {error}")),
            };
        }
        self.finish_runtime_network_player_add(
            request,
            host,
            alternate_resource_id,
            alternate_color,
        )
    }

    fn finish_runtime_network_player_add(
        &mut self,
        request: clonk_network::PlayerInfoUpdateRequest,
        host: bool,
        alternate_resource_id: i32,
        alternate_color: u32,
    ) -> Result<(), String> {
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| "network session is unavailable".to_string())?;
        if !host {
            return network
                .submit_player_info_update(request)
                .map_err(|error| error.to_string());
        }

        let restore_players = host_restore_player_info_entries(self.host_join_snapshot.as_ref());
        let generated_team_name_template = self.generated_team_name_template.clone();
        let has_or_will_have_lobby = self.has_or_will_have_network_lobby();
        let alternate_colors = &self.host_local_alternate_colors_by_resource;
        let local_player_info_ids = &self.host_local_player_info_ids;
        let admission = match self.network_team_assignment.as_mut() {
            Some(team_assignment) => team_assignment.admit_request_with_alternate_colors(
                &mut self.control_player_infos,
                request,
                self.network_max_players,
                true,
                has_or_will_have_lobby,
                &restore_players,
                |player| {
                    player
                        .resource
                        .as_ref()
                        .filter(|resource| resource.id == alternate_resource_id)
                        .map(|_| alternate_color)
                        .or_else(|| {
                            host_runtime_alternate_color(
                                alternate_colors,
                                local_player_info_ids,
                                player,
                            )
                        })
                },
            ),
            None => {
                let mut oracle =
                    ProcessInitialHostTeamAssignmentOracle::new(generated_team_name_template);
                self.control_player_infos
                    .admit_request_with_attributes_and_alternate_colors(
                        request,
                        self.network_max_players,
                        None,
                        &restore_players,
                        &mut oracle,
                        |player| {
                            player
                                .resource
                                .as_ref()
                                .filter(|resource| resource.id == alternate_resource_id)
                                .map(|_| alternate_color)
                                .or_else(|| {
                                    host_runtime_alternate_color(
                                        alternate_colors,
                                        local_player_info_ids,
                                        player,
                                    )
                                })
                        },
                    )
                    .map_err(NetworkTeamControlError::from)
            }
        }
        .map_err(|error| format!("host rejected the runtime player attributes: {error}"))?
        .ok_or_else(|| "host rejected the runtime player request".to_string())?;
        let admission = self
            .finalize_host_player_info_admission(admission)
            .ok_or_else(|| "host rejected the runtime league player request".to_string())?;
        let admitted_local_ids = admission
            .admitted
            .players
            .iter()
            .filter_map(|player| {
                player
                    .resource
                    .as_ref()
                    .filter(|resource| resource.id == alternate_resource_id)
                    .map(|_| player.id)
            })
            .filter(|id| *id > 0)
            .collect::<Vec<_>>();
        self.commit_host_player_info_admission(admission)
            .map_err(|error| error.to_string())?;
        if !admitted_local_ids.is_empty() {
            self.host_local_alternate_colors_by_resource
                .insert(alternate_resource_id, alternate_color);
            self.host_local_player_info_ids.extend(admitted_local_ids);
        }
        Ok(())
    }

    pub(crate) fn append_running_command_resource(&mut self, key: &str, fallback: &str) {
        let message = self.runtime_resource_text(key, fallback);
        self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
    }

    /// `C4GameControlNetwork::DecideControlDelivery`: clients always queue;
    /// the host uses Sync only while the network is frozen or its local
    /// client is inactive. Every other running command is tick-stamped.
    pub(crate) fn running_control_prefers_sync(&self) -> bool {
        if !matches!(self.runtime_network_role(), RuntimeNetworkRole::Host) {
            return false;
        }
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(0);
        let frozen = self.host_reference_paused && self.runtime_network_status_barrier.is_none();
        frozen || !self.control_clients.is_activated(local_client_id)
    }

    pub(crate) fn set_running_network_comment(&mut self, value: &[u8]) {
        let value = &value[..value
            .len()
            .min(clonk_frontend::game_option_buttons::COMMENT_MAX_TEXT)];
        let Some(comment) = clonk_engine::LegacyCString::from_bytes(value.to_vec()) else {
            return;
        };
        let projected = clonk_script::c4_string_from_bytes(value);
        self.persist_game_option_value("Network", "Comment", projected);
        let password_needed = self
            .advertised_game_reference
            .as_ref()
            .is_some_and(|reference| reference.summary().password_needed);
        self.publish_lobby_game_option_reference(password_needed, comment);
        let message = self.runtime_resource_text(
            "IDS_NET_COMMENTCHANGED",
            clonk_frontend::game_option_buttons::COMMENT_CHANGED_LOG,
        );
        self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
    }

    pub(crate) fn set_running_network_password(&mut self, value: &[u8]) {
        let Some(password) = clonk_engine::LegacyCString::from_bytes(value.to_vec()) else {
            return;
        };
        let Some(network) = self.network.as_ref() else {
            return;
        };
        if let Err(error) = network.set_host_password(password) {
            tracing::error!(%error, "failed to update chat-command host password");
            return;
        }
        let comment = self
            .advertised_game_reference
            .as_ref()
            .map(|reference| reference.metadata().comment.clone())
            .unwrap_or_default();
        self.publish_lobby_game_option_reference(!value.is_empty(), comment);
    }

    pub(crate) fn change_running_network_control_mode(&mut self, mode: i32) {
        if self.runtime_network_control_mode == Some(mode) {
            return;
        }
        let status = clonk_network::NetworkStatus {
            state: if self.host_reference_paused {
                clonk_network::NETWORK_STATE_PAUSE
            } else {
                clonk_network::NETWORK_STATE_GO
            },
            control_mode: mode,
            // C4Network2::SetCtrlMode deliberately uses ControlTick rather
            // than the next tick used by Pause/Sync.
            target_tick: self.displayed_network_control_tick(),
        };
        match self.change_runtime_network_status(status) {
            Ok(()) => {
                self.runtime_network_control_mode = Some(mode);
                self.persist_game_option_value("Network", "ControlMode", mode.to_string());
                self.publish_running_host_reference();
            }
            Err(error) => tracing::error!(%error, mode, "failed to change chat control mode"),
        }
    }

    pub(crate) fn record_network_error_round_result(&mut self, message: &str) {
        let encoded = match self.runtime_language_charset {
            RuntimeHelpCharset::Windows1252 => message
                .chars()
                .map(runtime_cp1252_byte)
                .collect::<Result<Vec<_>>>(),
            RuntimeHelpCharset::Utf8 => Ok(message.as_bytes().to_vec()),
        };
        let mut result_message = encoded.unwrap_or_else(|error| {
            tracing::warn!(
                %error,
                "network result message was not representable in the process language charset"
            );
            message.as_bytes().to_vec()
        });
        if let Some(nul) = result_message.iter().position(|byte| *byte == 0) {
            result_message.truncate(nul);
        }
        self.engine.evaluate_network_round_results(
            clonk_engine::RoundResultsNetworkResult::NetworkError,
            Some(result_message),
        );
        self.snapshot.round_results = self.engine.snapshot().round_results;
    }

    pub(crate) fn process_network_events(&mut self) -> Result<(), EngineError> {
        let events = self
            .network
            .as_mut()
            .map(NetworkManager::poll_events)
            .unwrap_or_default();
        let had_events = !events.is_empty();
        if had_events {
            self.mark_menu_dirty();
        }
        {
            for event in events {
                // C4GameControlNetwork::HandleControlPkt executes synchronized
                // controls immediately while network control is frozen in the
                // lobby (src/C4GameControlNetwork.cpp:558-588).
                let frozen_lobby = self.joined_network_lobby_active()
                    || self.classic_host_lobby_active()
                    || (self.mode == AppMode::Loading
                        && self
                            .loading_state
                            .as_ref()
                            .is_some_and(|loading| loading.prepared_go.is_some()));
                let frozen_runtime = self.mode == AppMode::Running
                    && self.host_reference_paused
                    && !self.network_control_running
                    && self.runtime_network_status_barrier.is_none();
                if (frozen_lobby || frozen_runtime)
                    && matches!(&event, NetworkEvent::ScheduledSync { .. })
                {
                    let NetworkEvent::ScheduledSync { tick, controls } = event else {
                        unreachable!("ScheduledSync was matched above");
                    };
                    self.apply_synchronized_controls(tick, controls)?;
                    continue;
                }
                if let NetworkEvent::FatalError(message) = &event {
                    self.record_network_error_round_result(message);
                    let purpose = if matches!(self.network_mode, Some(NetworkMode::Host(_))) {
                        StartupNetworkPurpose::StagedHost
                    } else {
                        StartupNetworkPurpose::Join
                    };
                    let network_control_active = self.network_control_clock.is_some()
                        && (self.mode == AppMode::Running
                            || (self.mode == AppMode::Loading
                                && self.network_lobby.is_none()
                                && self.classic_host_lobby.is_none()));
                    if network_control_active {
                        // Clear on loss of the live network invokes
                        // ChangeToLocal whenever Game.Control is already in
                        // CM_Network, including post-lobby Game::Init. It
                        // keeps the current frame/control tick and removes
                        // remote clients instead of leaving the dead lockstep
                        // clock installed (src/C4Network2.cpp:748-775;
                        // src/C4GameControl.cpp:93-127).
                        tracing::error!(
                            message = %message,
                            "fatal network worker error; changing network control to local"
                        );
                        if self.network.is_some() {
                            let local_client_id = self
                                .network
                                .as_ref()
                                .and_then(|network| i32::try_from(network.local_client_id()).ok())
                                .unwrap_or_else(|| self.offline_local_client_id());
                            self.change_network_control_to_local(local_client_id);
                        }
                        break;
                    }
                    // A failed application message loop before GO makes
                    // native DoLobby clear the network and return false.
                    // Game::Init then returns through QuitGame to the
                    // remembered startup dialog with its error flag retained
                    // (src/C4Network2.cpp:475-510;
                    // src/C4Game.cpp:408-411;
                    // src/C4Application.cpp:373-400,438-449).
                    self.finish_startup_network_failure(
                        purpose,
                        format!("Unable to start network session: {message}"),
                    )?;
                    break;
                }
                let lobby_diagnostic_active =
                    self.classic_host_lobby_active() || self.joined_network_lobby_active();
                if lobby_diagnostic_active {
                    let diagnostic = match &event {
                        NetworkEvent::RecoverableRouteDiagnostic { client_id, error } => {
                            Some((*client_id, error.clone(), false))
                        }
                        NetworkEvent::TransportDiagnostic { client_id, error } => {
                            Some((*client_id, error.clone(), true))
                        }
                        NetworkEvent::Error(error) => Some((None, error.clone(), true)),
                        _ => None,
                    };
                    if let Some((client_id, error, severe)) = diagnostic {
                        // Both host and client fullscreen lobbies own
                        // C4GameLobby::MainDlg. The GUI log sink forwards
                        // ordinary network warnings/errors to whichever
                        // lobby is live, without a host-role gate
                        // (src/C4Log.cpp:227-239;
                        // src/C4GameLobby.cpp:738-753).
                        let message = client_id
                            .map(|client_id| format!("client {client_id}: {error}"))
                            .unwrap_or(error);
                        if severe {
                            tracing::error!(message = %message, "lobby network diagnostic");
                        } else {
                            tracing::warn!(message = %message, "recoverable lobby route diagnostic");
                        }
                        self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
                        continue;
                    }
                }
                if self.classic_host_lobby_active() {
                    if let NetworkEvent::LeagueUpdate(response) = &event {
                        self.apply_league_update_response(response.clone());
                        continue;
                    }
                    if let NetworkEvent::NetpuncherStateChanged {
                        game_ids,
                        local_addresses,
                    } = &event
                    {
                        self.update_host_netpuncher_reference(*game_ids, local_addresses.clone());
                        continue;
                    }
                    if matches!(&event, NetworkEvent::PeerConnected { client_id: 0, .. }) {
                        continue;
                    }
                    let classic_lobby_state_event = matches!(
                        &event,
                        NetworkEvent::ActivationRequest { .. }
                            | NetworkEvent::StatusCommitted(_)
                            | NetworkEvent::JoinDataNeeded { .. }
                            | NetworkEvent::PlayerInfoUpdateRequest { .. }
                            | NetworkEvent::PreexecutedPlayerInfoEcho { .. }
                            | NetworkEvent::DirectControl(NetworkControl::PlayerInfo(_))
                            | NetworkEvent::DirectControl(NetworkControl::ClientJoin(_))
                            | NetworkEvent::DirectControl(NetworkControl::ClientUpdate(_))
                            | NetworkEvent::DirectControl(NetworkControl::ClientRemove(_))
                            | NetworkEvent::DirectControl(NetworkControl::DebugRecord(_))
                            | NetworkEvent::PeerConnected { .. }
                            | NetworkEvent::PeerDisconnected { .. }
                            | NetworkEvent::PeerConnectionFailed { .. }
                            | NetworkEvent::ResourceProgress { .. }
                            | NetworkEvent::ResourceComplete { .. }
                            | NetworkEvent::ResourceLoadFailed { .. }
                            | NetworkEvent::ReadyCheck(_)
                            | NetworkEvent::HostStatusAck { .. }
                            | NetworkEvent::LobbyCountdown(_)
                            | NetworkEvent::ResourceAction(_)
                            | NetworkEvent::ResourceDeriveUnsupported { .. }
                    );
                    if let NetworkEvent::DirectControl(NetworkControl::Message(control)) = &event {
                        self.execute_message_control(control.clone());
                        continue;
                    }
                    let boundary = match &event {
                        NetworkEvent::HostPingMeasured { .. } => None,
                        NetworkEvent::HostStatusChanged(_) => None,
                        NetworkEvent::JoinData(_) => Some("join data"),
                        NetworkEvent::LeagueRoundResults(_) => Some("league round results"),
                        NetworkEvent::LeagueUpdate(_) => Some("league update"),
                        NetworkEvent::ReadyCheck(_) => None,
                        NetworkEvent::HostStatusAck { .. } => None,
                        NetworkEvent::StatusRequested(_) => Some("status request"),
                        NetworkEvent::StatusCommitted(_) => None,
                        NetworkEvent::LobbyCountdown(_) => None,
                        NetworkEvent::ActivationRequest { .. } => None,
                        NetworkEvent::JoinDataNeeded { .. } => None,
                        NetworkEvent::PlayerInfoUpdateRequest { .. } => None,
                        NetworkEvent::PreexecutedPlayerInfoEcho { .. } => None,
                        NetworkEvent::ReadyTick { .. } => Some("ready control tick"),
                        NetworkEvent::ScheduledSync { .. } => Some("scheduled control"),
                        NetworkEvent::DirectControl(
                            NetworkControl::PlayerInfo(_)
                            | NetworkControl::ClientJoin(_)
                            | NetworkControl::ClientUpdate(_)
                            | NetworkControl::ClientRemove(_)
                            | NetworkControl::DebugRecord(_),
                        ) => None,
                        NetworkEvent::DirectControl(_) => Some("direct player/resource control"),
                        NetworkEvent::PeerConnected { .. } => None,
                        NetworkEvent::PeerDisconnected { .. } => None,
                        NetworkEvent::PeerConnectionFailed { .. } => None,
                        NetworkEvent::NetpuncherStateChanged { .. } => unreachable!(
                            "netpuncher state is applied before the classic-lobby boundary"
                        ),
                        NetworkEvent::ResourceAction(_) => None,
                        NetworkEvent::ResourceProgress { .. } => None,
                        NetworkEvent::ResourceComplete { .. } => None,
                        NetworkEvent::ResourceLoadFailed { .. } => None,
                        NetworkEvent::ResourceDeriveUnsupported { .. } => None,
                        NetworkEvent::RecoverableRouteDiagnostic { .. } => unreachable!(
                            "recoverable route diagnostics are logged before the classic-lobby boundary"
                        ),
                        NetworkEvent::TransportDiagnostic { .. } => unreachable!(
                            "transport diagnostics are logged before the classic-lobby boundary"
                        ),
                        NetworkEvent::Error(_) => unreachable!(
                            "network diagnostics are logged before the classic-lobby boundary"
                        ),
                        NetworkEvent::FatalError(_) => unreachable!(
                            "fatal worker failures restore startup before the classic-lobby boundary"
                        ),
                    };
                    if let Some(boundary) = boundary {
                        return Err(classic_game_lobby_child_error(
                            ClassicGameLobbyChild::NetworkEvent(boundary),
                        ));
                    }
                    if !classic_lobby_state_event {
                        continue;
                    }
                }
                match event {
                    // Route Ping/Pong remains available for presentation, but
                    // C++ CalcPerformance paces from the full preferred-route
                    // topology sampled at the consumed-control boundary.
                    NetworkEvent::HostPingMeasured { .. } => {}
                    NetworkEvent::HostStatusChanged(status) => {
                        self.retarget_network_start_wait(status);
                        if let Some(clock) = self.network_control_clock.as_mut() {
                            clock.set_target_tick(Some(status.target_tick));
                        }
                        let rereach_prepared_host = self
                            .loading_state
                            .as_mut()
                            .and_then(|loading| loading.prepared_go.as_mut())
                            .is_some_and(|pending| {
                                let rereach = pending.local_reached
                                    && matches!(self.network_mode, Some(NetworkMode::Host(_)));
                                pending.local_reached = false;
                                pending.status = status;
                                rereach
                            });
                        if rereach_prepared_host {
                            match self
                                .network
                                .as_ref()
                                .map(NetworkManager::status_reached_current)
                            {
                                Some(Ok(())) => {
                                    if let Some(pending) = self
                                        .loading_state
                                        .as_mut()
                                        .and_then(|loading| loading.prepared_go.as_mut())
                                    {
                                        pending.local_reached = true;
                                    }
                                }
                                Some(Err(error)) => {
                                    self.status_text = format!(
                                        "Unable to reach retargeted network Go barrier: {error}"
                                    );
                                }
                                None => {}
                            }
                        } else if let Some(pending) = self
                            .loading_state
                            .as_mut()
                            .and_then(|loading| loading.prepared_go.as_mut())
                        {
                            pending.status = status;
                        }
                        if self.mode == AppMode::Running {
                            self.arm_runtime_network_status_barrier(status);
                        }
                    }
                    NetworkEvent::HostStatusAck { client_id, status } => {
                        self.update_network_start_wait_ack(client_id, status);
                    }
                    NetworkEvent::JoinData(join_data) => {
                        // Game.Parameters is the authoritative client/player
                        // snapshot. Scenario and dynamic resource application
                        // remains deferred until the game leaves the lobby
                        // (src/C4Network2.cpp:1574-1620,619-671).
                        self.admission_resources
                            .register_join_data_resources(&join_data);
                        let lobby_resource_rows = joined_classic_lobby_resource_rows(
                            &join_data,
                            &self.admission_resources.present_percent,
                        );
                        self.network_max_players =
                            usize::try_from(join_data.parameters.max_players).unwrap_or(0);
                        self.engine
                            .set_max_players(join_data.parameters.max_players);
                        self.engine
                            .set_use_fair_crew(join_data.parameters.use_fair_crew);
                        self.engine
                            .set_fair_crew_strength(join_data.parameters.fair_crew_strength);
                        self.engine
                            .set_fair_crew_forced(join_data.parameters.fair_crew_forced);
                        self.engine
                            .set_allow_debug(join_data.parameters.allow_debug);
                        self.network_is_league =
                            synchronized_parameters_are_league(&join_data.parameters);
                        self.network_league_name = synchronized_league_name(&join_data.parameters);
                        self.network_control_clock = Some(NetworkControlClock::new(
                            join_data.start_control_tick,
                            join_data.parameters.control_rate,
                        ));
                        self.control_clients
                            .replace_snapshot(join_data.parameters.clients.clients.iter().cloned());
                        self.refresh_network_client_next_control_ticks();
                        self.network_client_activity.replace_clients(
                            join_data
                                .parameters
                                .clients
                                .clients
                                .iter()
                                .map(|client| client.client_id),
                        );
                        self.control_player_infos.replace_snapshot(
                            join_data.parameters.player_infos.last_player_id,
                            join_data
                                .parameters
                                .player_infos
                                .clients
                                .iter()
                                .cloned()
                                .map(|client| clonk_engine::PlayerInfoControlData {
                                    client_id: client.client_id,
                                    flags: client.flags,
                                    players: client.players,
                                    by_client: 0,
                                }),
                        );
                        seed_engine_player_info_parameters(
                            &mut self.engine,
                            &self.network_league_name,
                            &self.control_player_infos,
                        );
                        let scenario_title =
                            legacy_presentation_text(join_data.parameters.title.as_bytes());
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            lobby.replace_participants_from_clients(
                                &join_data.parameters.clients.clients,
                            );
                            lobby.set_scenario_title(&scenario_title);
                            lobby.resource_rows = lobby_resource_rows;
                        }
                        if !scenario_title.is_empty() {
                            self.scenario_label = scenario_title;
                        }
                        let joined_league_server_name = retain_client_league_server_name(
                            self.network_mode.as_mut(),
                            &join_data.parameters.league_address,
                        );
                        let is_client =
                            matches!(self.network_mode.as_ref(), Some(NetworkMode::Client(_)));
                        let local_is_observer =
                            self.control_clients.is_observer(join_data.client_id);
                        let initial_player_info_ready = if is_client && !local_is_observer {
                            match self.submit_initial_client_player_info(
                                join_data.client_id,
                                joined_league_server_name,
                            ) {
                                LeaguePlayerAuthStatus::Completed(submitted) => submitted,
                                LeaguePlayerAuthStatus::Pending => false,
                            }
                        } else {
                            is_client && local_is_observer
                        };
                        self.initial_lobby_status_ack_pending = initial_player_info_ready
                            && join_data.status.state == clonk_network::NETWORK_STATE_LOBBY;
                        self.client_start_barrier =
                            ClientStartBarrier::from_join_data_status(join_data.status);
                        self.pending_client_start_status = None;
                        self.clear_lobby_preload();
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            lobby.preload.reset_for_context();
                        }
                        self.pending_network_join_data = Some(join_data);
                        self.sync_network_lobby_game_option_state();
                        self.sync_classic_lobby_roster();
                        self.sync_classic_lobby_resource_ready();
                        self.acknowledge_initial_lobby_status_if_ready();
                    }
                    NetworkEvent::LeagueRoundResults(packet) => {
                        self.apply_league_round_results_packet(&packet);
                    }
                    NetworkEvent::LeagueUpdate(response) => {
                        self.apply_league_update_response(response);
                    }
                    NetworkEvent::ReadyCheck(packet) => {
                        if packet.data.vote_requested() {
                            if self.control_clients.clear_nonhost_lobby_ready() {
                                self.publish_updated_host_join_snapshot();
                            }
                            self.sync_classic_lobby_roster();
                            self.handle_lobby_ready_check_request(packet)?;
                        } else {
                            self.append_remote_lobby_ready_log(packet);
                            let ready_state_changed = self
                                .control_clients
                                .set_lobby_ready(packet.client_id, packet.data.is_ready());
                            let lobby_changed_client_id = self
                                .network_lobby
                                .as_mut()
                                .and_then(|lobby| lobby.apply_ready_check(packet));
                            if ready_state_changed {
                                self.publish_updated_host_join_snapshot();
                            }
                            self.sync_classic_lobby_roster();
                            let changed_client_id = ready_state_changed
                                .then(|| ClientId::try_from(packet.client_id).ok())
                                .flatten()
                                .or(lobby_changed_client_id);
                            if let Some(changed_client_id) = changed_client_id {
                                self.on_lobby_client_ready_state_change(changed_client_id)?;
                            }
                        }
                    }
                    NetworkEvent::LobbyCountdown(packet) => {
                        let local_echo = matches!(self.network_mode, Some(NetworkMode::Host(_)))
                            && self
                                .pending_local_lobby_countdown_echoes
                                .front()
                                .is_some_and(|pending| *pending == packet);
                        if local_echo {
                            self.pending_local_lobby_countdown_echoes.pop_front();
                        } else {
                            self.apply_lobby_countdown_presentation(packet);
                        }
                    }
                    NetworkEvent::StatusRequested(status) => {
                        if let Some(clock) = self.network_control_clock.as_mut() {
                            clock.set_target_tick(Some(status.target_tick));
                        }
                        if status.state != clonk_network::NETWORK_STATE_LOBBY
                            && self.joined_network_lobby_active()
                        {
                            // HandleStatus installs GS_Go before the status
                            // acknowledgement or scenario preparation
                            // finishes. That makes DoLobby close and delete
                            // pLobby immediately (src/C4Network2.cpp:475-515,
                            // 2010-2029).
                            self.close_lobby_child_dialogs_silently();
                            self.network_lobby = None;
                            self.host_lobby_countdown = None;
                            self.pending_local_lobby_countdown_echoes.clear();
                            self.mode = AppMode::Loading;
                        }
                        if self.mode == AppMode::Running {
                            // CheckStatusReached keeps control running until
                            // the requested target is reached at an empty
                            // cadence boundary. Receipt alone never pauses.
                            self.arm_runtime_network_status_barrier(status);
                        } else {
                            // Before InitGameFinal, local scenario preparation
                            // is the reach condition and control stays stopped.
                            self.network_control_running = false;
                            if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
                                let first_part_preloaded = self.lobby_preload_task.is_some()
                                    || self
                                        .lobby_preload_artifact
                                        .as_ref()
                                        .and_then(|artifact| artifact.client.as_ref())
                                        .is_some_and(|client| client.scenario.is_some());
                                if let Some(requested) =
                                    self.client_start_barrier.status_requested(status)
                                {
                                    self.pending_client_start_status = Some(requested);
                                    // InitGameFirstPart publishes 6 before
                                    // RetrieveScenario may block on either
                                    // synchronized scenario resource
                                    // (src/C4Game.cpp:2558-2568).
                                    if !first_part_preloaded {
                                        if let Some(loader) = self.loader_screen.as_mut() {
                                            loader.update(LoaderUpdate::SetProgress(6));
                                        }
                                    }
                                }
                                self.prepare_client_network_scenario_if_ready()?;
                                if self.network.is_none() {
                                    break;
                                }
                            }
                        }
                        tracing::debug!(
                            state = status.state,
                            control_mode = status.control_mode,
                            target_tick = status.target_tick,
                            "network status is waiting for local preparation"
                        );
                    }
                    NetworkEvent::StatusCommitted(status) => {
                        let closes_start_wait = self
                            .network_start_wait
                            .as_ref()
                            .is_some_and(|wait| wait.expected_status == status);
                        self.handle_status_committed(status)?;
                        if self.mode == AppMode::Running {
                            if closes_start_wait {
                                self.network_start_wait = None;
                            }
                            self.dismiss_network_client_start_wait();
                        }
                    }
                    NetworkEvent::ActivationRequest {
                        client_id,
                        tick,
                        waited_for,
                        ping_ms,
                    } => {
                        let client_id = i32::try_from(client_id).unwrap_or(i32::MAX);
                        let host_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
                        let running =
                            matches!(self.mode, AppMode::Running) && self.network_control_running;
                        if let Some(update) = self.control_clients.activation_update_for_request(
                            client_id,
                            tick,
                            host_frame,
                            running,
                            waited_for,
                            ping_ms,
                            self.frames_per_second,
                        ) {
                            if let Some(Err(error)) = self
                                .network
                                .as_ref()
                                .map(|network| network.submit_client_update(update))
                            {
                                tracing::error!(%error, "failed to submit client activation");
                            }
                        }
                    }
                    NetworkEvent::JoinDataNeeded {
                        client_id,
                        current_control_tick,
                    } => {
                        self.request_runtime_join_dynamic(client_id, current_control_tick);
                    }
                    NetworkEvent::PlayerInfoUpdateRequest {
                        origin,
                        request,
                        by_host,
                    } => {
                        tracing::debug!(%origin, by_host, "processing PlayerInfo update request");
                        let restore_players =
                            host_restore_player_info_entries(self.host_join_snapshot.as_ref());
                        let generated_team_name_template =
                            self.generated_team_name_template.clone();
                        let has_or_will_have_lobby = self.has_or_will_have_network_lobby();
                        let alternate_colors = &self.host_local_alternate_colors_by_resource;
                        let local_player_info_ids = &self.host_local_player_info_ids;
                        let admission = match self.network_team_assignment.as_mut() {
                            Some(team_assignment) => team_assignment
                                .admit_request_with_alternate_colors(
                                    &mut self.control_player_infos,
                                    request,
                                    self.network_max_players,
                                    by_host,
                                    has_or_will_have_lobby,
                                    &restore_players,
                                    |player| {
                                        host_runtime_alternate_color(
                                            alternate_colors,
                                            local_player_info_ids,
                                            player,
                                        )
                                    },
                                ),
                            None => {
                                let mut oracle = ProcessInitialHostTeamAssignmentOracle::new(
                                    generated_team_name_template,
                                );
                                self.control_player_infos
                                    .admit_request_with_attributes_and_alternate_colors(
                                        request,
                                        self.network_max_players,
                                        None,
                                        &restore_players,
                                        &mut oracle,
                                        |player| {
                                            host_runtime_alternate_color(
                                                alternate_colors,
                                                local_player_info_ids,
                                                player,
                                            )
                                        },
                                    )
                                    .map_err(NetworkTeamControlError::from)
                            }
                        };
                        match admission {
                            Ok(Some(admission)) => {
                                let Some(admission) =
                                    self.finalize_host_player_info_admission(admission)
                                else {
                                    continue;
                                };
                                if let Err(error) =
                                    self.commit_host_player_info_admission(admission)
                                {
                                    tracing::error!(%error, "failed to broadcast authoritative PlayerInfo");
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                tracing::warn!(
                                    %origin,
                                    %error,
                                    "rejected PlayerInfo update with unavailable attribute-conflict state"
                                );
                            }
                        }
                    }
                    NetworkEvent::PreexecutedPlayerInfoEcho {
                        original,
                        info,
                        join_players_on_echo,
                    } => {
                        self.apply_preexecuted_player_info_echo(
                            original,
                            info,
                            join_players_on_echo,
                        );
                    }
                    NetworkEvent::ReadyTick { tick, controls } => {
                        if self.mode == AppMode::Running {
                            let expected_tick = self.expected_network_control_tick();
                            self.network_ticks.queue(expected_tick, tick, controls);
                        }
                    }
                    NetworkEvent::ScheduledSync { tick, controls } => {
                        let expected_tick = self.expected_network_control_tick();
                        self.network_sync.queue(expected_tick, tick, controls);
                    }
                    NetworkEvent::DirectControl(control) => {
                        if let Some(packet) = control.clone().into_packet() {
                            self.record_control_packet(&packet);
                        }
                        match control {
                            NetworkControl::ClientJoin(join) => {
                                if self.control_clients.apply_join(&join) {
                                    self.append_network_client_join_log(join.core.name.as_bytes());
                                    self.network_client_activity
                                        .reset_client(join.core.client_id);
                                    self.publish_updated_host_join_snapshot();
                                    self.sync_classic_lobby_roster();
                                }
                            }
                            NetworkControl::ClientUpdate(update) => {
                                self.control_clients.apply_update(&update);
                                if update.by_client == 0 {
                                    self.publish_updated_host_join_snapshot();
                                }
                                self.sync_classic_lobby_roster();
                            }
                            NetworkControl::ClientRemove(remove) => {
                                if self.control_clients.apply_remove(&remove) {
                                    self.remove_classic_lobby_resources_at_client(remove.client_id);
                                    self.network_client_activity.remove_client(remove.client_id);
                                    self.control_messages.remove_client(remove.client_id);
                                    self.control_player_infos.on_client_part(remove.client_id);
                                    self.publish_current_host_player_infos();
                                    self.sync_classic_lobby_roster();
                                }
                            }
                            NetworkControl::SyncCheck(packet) => {
                                self.handle_sync_check(packet);
                            }
                            NetworkControl::PlayerInfo(info) => {
                                let follow_ups = self.apply_direct_player_info_control(info, true);
                                for follow_up in follow_ups {
                                    if let Err(error) = self.broadcast_and_preexecute_player_info(
                                        follow_up, true, false,
                                    ) {
                                        tracing::error!(%error, "failed to broadcast updated PlayerInfo follow-up");
                                    }
                                }
                            }
                            NetworkControl::Vote(vote) => self.execute_league_vote(vote)?,
                            NetworkControl::Set(set) => self.execute_control_set(set),
                            NetworkControl::DebugRecord(_) => {
                                // C4ControlDebugRec::Execute is intentionally empty.
                            }
                            NetworkControl::Message(message) => {
                                self.execute_message_control(message);
                            }
                            control => {
                                tracing::warn!(?control, "ignoring unsupported direct control");
                            }
                        }
                    }
                    NetworkEvent::PeerConnected {
                        client_id,
                        name,
                        kind,
                    } => {
                        tracing::info!(%client_id, %name, ?kind, "network client connected");
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            lobby.register_peer(client_id, name.clone(), kind);
                        }
                        if let (Ok(client_id), Some(wait)) =
                            (i32::try_from(client_id), self.network_start_wait.as_mut())
                        {
                            wait.controller.update_client(
                                clonk_frontend::network_start_wait::NetworkStartWaitClient::new(
                                    client_id,
                                    name.clone(),
                                    clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Loading,
                                ),
                            );
                        }
                        // The transport callback updates connection state only.
                        // C++ MainDlg::OnClientConnect is empty; the accepted
                        // C4ControlClientJoin owns the visible lobby log
                        // (src/C4GameLobby.cpp:669-675;
                        // src/C4Control.cpp:554-565).
                    }
                    NetworkEvent::PeerDisconnected { client_id, reason } => {
                        self.forget_pending_runtime_join_client(client_id);
                        let abort_countdown_for_disconnected_client =
                            matches!(self.network_mode, Some(NetworkMode::Host(_)))
                                && (self.network_lobby.as_ref().is_some_and(|lobby| {
                                    lobby.countdown.is_some_and(|remaining| {
                                        remaining <= ALMOST_START_LOBBY_COUNTDOWN_SECONDS
                                    })
                                }) || self
                                    .classic_host_lobby
                                    .as_ref()
                                    .is_some_and(|lobby| lobby.controller.countdown().is_locked()))
                                && i32::try_from(client_id).ok().is_some_and(|client_id| {
                                    !self
                                        .control_player_infos
                                        .client_info_ids(client_id)
                                        .is_empty()
                                });
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            lobby.unregister_peer(client_id);
                        }
                        self.mark_network_start_wait_client_kick(client_id);
                        if abort_countdown_for_disconnected_client {
                            self.abort_network_lobby_countdown();
                        }
                        let local_client_id = (client_id == 0
                            && matches!(self.network_mode.as_ref(), Some(NetworkMode::Client(_))))
                        .then(|| {
                            self.network
                                .as_ref()
                                .and_then(|network| i32::try_from(network.local_client_id()).ok())
                        })
                        .flatten();
                        if let Some(local_client_id) = local_client_id {
                            let host_lost_in_lobby = self.mode == AppMode::Menu
                                && self.startup_view == StartupView::NetworkLobby;
                            let host_lost_during_final_init = self.mode == AppMode::Loading
                                && (self
                                    .loading_state
                                    .as_ref()
                                    .and_then(|loading| loading.prepared_go.as_ref())
                                    .is_some_and(|prepared| prepared.local_reached)
                                    || self.message_dialogs.iter().any(|dialog| {
                                        matches!(
                                            dialog.continuation,
                                            MessageDialogContinuation::NetworkClientStartWait
                                        )
                                    }));
                            let host_name = self
                                .control_clients
                                .state(0)
                                .map(|host| legacy_presentation_text(host.name.as_bytes()))
                                .filter(|name| !name.is_empty())
                                .unwrap_or_else(|| "Host".to_string());
                            // A lost host cannot receive a graceful ConnRe;
                            // C4Network2 clears directly into ChangeToLocal
                            // (C4Network2.cpp:1786-1817).
                            self.report_league_disconnect(
                                local_client_id,
                                clonk_network::LeagueDisconnectReason::ConnectionFailed,
                            );
                            let message = format_resource_string(
                                self.runtime_resource_text(
                                    "IDS_NET_HOSTDISCONNECTED",
                                    "Network: host %s disconnected!",
                                ),
                                &[&host_name],
                            );
                            // OnClientDisconnect evaluates the host-loss
                            // verdict before C4Network2::Clear. The same
                            // result is retained whether Clear exits DoLobby
                            // or changes an active round to local control
                            // (src/C4Network2.cpp:1825-1833).
                            self.record_network_error_round_result(&message);
                            if host_lost_in_lobby {
                                // Clear makes an active DoLobby return false,
                                // aborting C4Game::Init back through the
                                // remembered startup dialog (C4Network2.cpp:
                                // 477-515,1809-1833; C4Game.cpp:405-411).
                                self.finish_startup_network_failure(
                                    StartupNetworkPurpose::Join,
                                    message,
                                )?;
                                break;
                            }
                            if host_lost_during_final_init {
                                // Clear releases FinalInit's wait, whose final
                                // isEnabled() check then fails. Dismiss its
                                // modal before unwinding the failed Game::Init
                                // through the appropriate startup lineage
                                // (C4Network2.cpp:558-616,1809-1833;
                                // C4Game.cpp:459-466).
                                self.dismiss_network_client_start_wait();
                                let final_init_error = self.runtime_resource_text(
                                    "IDS_ERR_NETWORKFINALINIT",
                                    "Error on final network init.",
                                );
                                self.finish_scenario_loading_failure(final_init_error, true)?;
                                break;
                            }
                            self.change_network_control_to_local(local_client_id);
                        }
                        if let Some(reason) = reason {
                            tracing::info!(
                                %client_id,
                                reason = %reason,
                                "network client disconnected"
                            );
                        } else {
                            tracing::info!(%client_id, "network client disconnected");
                        }
                        // An ordinary remote-client transport loss is likewise
                        // presentation-silent. C++ waits for the authoritative
                        // ClientRemove control to write the localized lobby log
                        // (src/C4Network2.cpp:1774-1833;
                        // src/C4Control.cpp:637-670).
                    }
                    NetworkEvent::PeerConnectionFailed { client_id } => {
                        self.forget_pending_runtime_join_client(client_id);
                        if let Ok(client_id) = i32::try_from(client_id) {
                            self.report_league_disconnect(
                                client_id,
                                clonk_network::LeagueDisconnectReason::ConnectionFailed,
                            );
                        }
                    }
                    NetworkEvent::NetpuncherStateChanged {
                        game_ids,
                        local_addresses,
                    } => {
                        self.update_host_netpuncher_reference(game_ids, local_addresses);
                    }
                    NetworkEvent::ResourceAction(action) => {
                        // Socket/protocol state is already retained by
                        // clonk-network. Filesystem-backed serving and chunk
                        // persistence are completed by the resource backend
                        // rather than silently treating loadable cores as
                        // available.
                        tracing::debug!(?action, "network resource backend action pending");
                        self.sync_classic_lobby_resource_ready();
                    }
                    NetworkEvent::ResourceProgress {
                        resource_id,
                        present_percent,
                    } => {
                        self.admission_resources
                            .mark_progress(resource_id, present_percent);
                        self.update_classic_lobby_resource_progress(resource_id, present_percent);
                        self.sync_classic_lobby_resource_ready();
                    }
                    NetworkEvent::ResourceComplete {
                        resource_id,
                        core,
                        path,
                        local,
                    } => {
                        // The control host registers FinishDerive's returned
                        // core synchronously so a second save can derive from
                        // it before this queued event is drained. Retain that
                        // resource's mutable getFile()-equivalent path and
                        // ownership instead of replacing them with the
                        // backend's serving standalone.
                        let (path, local) = match self.admission_resources.status(resource_id) {
                            Some(AdmissionResourceState::Complete {
                                path: mutable_path,
                                local: mutable_local,
                                ..
                            }) if core.derived_id >= 0 => (mutable_path.clone(), *mutable_local),
                            _ => (path, local),
                        };
                        self.admission_resources.register_lobby_resource(&core);
                        self.admission_resources.mark_complete_with_locality(
                            resource_id,
                            path.clone(),
                            local,
                        );
                        self.register_classic_lobby_resource(&core, 100);
                        // A completed player resource may replace the lobby's
                        // fallback icon with BigIcon.png. This is an explicit
                        // PlayerInfo-list invalidation; ordinary packet batches
                        // must not rebuild these raster rows.
                        if core.resource_type == clonk_network::HostResourceType::Player as u8 {
                            self.sync_classic_lobby_roster();
                        }
                        tracing::info!(
                            resource_id,
                            resource = %core.filename.to_string_lossy(),
                            path = %path.display(),
                            "network resource received"
                        );
                        self.finish_blocking_resource_wait(resource_id);
                        // A Graphics-bearing group arriving mid-round is the
                        // network overloading C4GraphicsResource::Init stays
                        // re-callable for (C4GraphicsResource.cpp:285-291).
                        self.refresh_network_overloaded_gui_resources(&core)?;
                        self.prepare_client_network_scenario_if_ready()?;
                        if self.network.is_none() {
                            break;
                        }
                        self.sync_classic_lobby_resource_ready();
                    }
                    NetworkEvent::ResourceLoadFailed { resource_id } => {
                        let failed_client_start = self
                            .blocking_resource_wait
                            .as_ref()
                            .filter(|wait| {
                                wait.resource_id == resource_id
                                    && wait.scope == BlockingResourceScope::ClientStart
                            })
                            .map(|wait| wait.display_name.clone());
                        self.admission_resources.mark_failed(resource_id);
                        self.finish_blocking_resource_wait(resource_id);
                        self.remove_classic_lobby_resource(resource_id);
                        // A failed player transfer may restore the fallback
                        // icon, so reproject at this lifecycle edge only.
                        if self
                            .admission_resources
                            .resource_cores
                            .get(&resource_id)
                            .is_some_and(|core| {
                                core.resource_type == clonk_network::HostResourceType::Player as u8
                            })
                        {
                            self.sync_classic_lobby_roster();
                        }
                        tracing::warn!(resource_id, "network resource load failed");
                        if let Some(display_name) = failed_client_start {
                            self.finish_startup_network_failure(
                                StartupNetworkPurpose::Join,
                                format!("Unable to retrieve {display_name}."),
                            )?;
                            break;
                        }
                        self.sync_classic_lobby_resource_ready();
                    }
                    NetworkEvent::ResourceDeriveUnsupported { core } => {
                        tracing::warn!(
                            resource_id = core.id,
                            parent_resource_id = core.derived_id,
                            "network resource derivation is not implemented"
                        );
                        self.sync_classic_lobby_resource_ready();
                    }
                    NetworkEvent::RecoverableRouteDiagnostic { client_id, error } => {
                        tracing::warn!(
                            ?client_id,
                            message = %error,
                            "recoverable network route diagnostic"
                        );
                    }
                    NetworkEvent::TransportDiagnostic { client_id, error } => {
                        tracing::error!(
                            ?client_id,
                            message = %error,
                            "network transport diagnostic"
                        );
                    }
                    NetworkEvent::Error(message) => {
                        tracing::error!(message = %message, "network error");
                    }
                    NetworkEvent::FatalError(message) => {
                        tracing::error!(message = %message, "fatal network worker error");
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_league_round_results_packet(
        &mut self,
        packet: &clonk_network::LeagueRoundResultsPacket,
    ) {
        self.engine.evaluate_league_round_results(
            packet.success,
            packet.result_string.as_bytes().to_vec(),
            packet
                .players
                .iter()
                .map(|result| clonk_engine::LeagueRoundResultUpdate {
                    player_info_id: result.player_info_id,
                    league_score_new: result.league_score_new,
                    league_score_gain: result.league_score_gain,
                    league_rank_new: result.league_rank_new,
                    league_rank_symbol_new: result.league_rank_symbol_new,
                    league_progress_data: result.league_progress_data.as_bytes().to_vec(),
                }),
        );
        self.snapshot.round_results = self.engine.snapshot().round_results;
        tracing::info!(
            success = packet.success,
            player_count = packet.players.len(),
            result = %packet.result_string.to_string_lossy(),
            "applied league round results"
        );
    }

    fn apply_league_update_response(&mut self, response: clonk_network::LeagueUpdateResponse) {
        let updates = self.control_player_infos.apply_league_projected_gains(
            response
                .player_infos
                .players
                .iter()
                .map(|player| (player.id, player.league_projected_gain)),
        );
        if updates.is_empty() {
            return;
        }
        if let Some(network) = self.network.as_ref() {
            for update in updates {
                if let Err(error) = network.broadcast_player_info(update) {
                    tracing::error!(%error, "failed to broadcast league projected gains");
                }
            }
        }
        self.publish_current_host_player_infos();
        self.sync_classic_lobby_roster();
    }

    pub(crate) fn report_league_disconnect(
        &self,
        client_id: i32,
        reason: clonk_network::LeagueDisconnectReason,
    ) {
        if !self.network_is_league || self.mode != AppMode::Running || self.snapshot.game_over {
            return;
        }
        let (_, clients) = self.control_player_infos.retained_rows_snapshot();
        let Some((_, flags, players)) = clients
            .into_iter()
            .find(|(id, _, players)| *id == client_id && players.iter().any(|p| p.is_joined()))
        else {
            return;
        };
        let Some(network) = self.network.as_ref() else {
            return;
        };
        if let Err(error) = network.report_league_disconnect(
            reason,
            clonk_network::ClientPlayerInfosSnapshot {
                client_id,
                flags,
                players,
            },
            clonk_network::LeagueFbidRegistry::new(),
        ) {
            tracing::error!(%error, client_id, "failed to queue league disconnect report");
        }
    }

    pub(crate) fn handle_sync_check(&mut self, packet: SyncCheckPacket) {
        if matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        if let Some((local, remote)) = self.sync_checks.record_remote(packet) {
            self.evaluate_sync_checks(local, remote);
        }
    }

    fn evaluate_sync_checks(&mut self, local: SyncCheckPacket, remote: SyncCheckPacket) {
        if local.matches(&remote) {
            return;
        }
        self.handle_desync(local, remote);
    }

    pub(crate) fn league_signup_layout(
        &self,
    ) -> Option<clonk_frontend::league_signup::LeagueSignupLayout> {
        let dialog = self.league_signup_dialog.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.graphics.surface();
        Some(
            dialog
                .controller
                .layout(surface.width() as i32, surface.height() as i32, &fonts.text),
        )
    }

    pub(crate) fn request_network_reference_join(
        &mut self,
        reference: clonk_network::NetworkGameReference,
    ) -> Result<(), EngineError> {
        if !reference.join_allowed {
            let message = self.runtime_resource_text(
                "IDS_NET_NOJOIN_NORUNTIME",
                "The game has started already and runtime join is not allowed! Try joining anyway?",
            );
            let caption = self.runtime_resource_text("IDS_NET_NOJOIN", "Cannot join game");
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::new(
                    message,
                    caption,
                    clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    clonk_frontend::message_dialog::MessageDialogSize::Regular,
                    false,
                ),
                MessageDialogContinuation::NetworkRuntimeJoin { reference },
            )?;
            return Ok(());
        }
        self.activate_network_reference_join(reference)
    }

    pub(crate) fn apply_network_join_edit_context_command(
        &mut self,
        command: clonk_frontend::startup_netdlg::NetDlgEditContextCommand,
    ) -> Result<(), EngineError> {
        if !self.external_irc_dialog_visible
            && (self.mode != AppMode::Menu || self.startup_view != StartupView::NetworkGame)
        {
            tracing::error!(?command, "stale join-address context command");
            return Ok(());
        }
        let clipboard = matches!(
            command,
            clonk_frontend::startup_netdlg::NetDlgEditContextCommand::Paste
        )
        .then(|| {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.get_text())
                .ok()
        })
        .flatten();
        let fonts = self.assets.clonk_fonts.clone();
        let actions = fonts
            .as_deref()
            .and_then(|fonts| {
                self.input_network_dialog_mut().map(|dialog| {
                    dialog.apply_context_command(command, clipboard.as_deref(), &fonts.text)
                })
            })
            .unwrap_or_default();
        self.process_network_dialog_actions(actions)?;
        self.mark_menu_dirty();
        Ok(())
    }

    pub(crate) fn apply_league_signup_edit_context_command(
        &mut self,
        field: clonk_frontend::league_signup::LeagueSignupField,
        command: clonk_frontend::league_signup::LeagueSignupEditContextCommand,
    ) -> Result<(), EngineError> {
        if self.league_signup_dialog.is_none() {
            tracing::error!(?field, ?command, "stale league-signup context command");
            return Ok(());
        }
        let clipboard = matches!(
            command,
            clonk_frontend::league_signup::LeagueSignupEditContextCommand::Paste
        )
        .then(|| {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.get_text())
                .ok()
        })
        .flatten();
        let layout = self.league_signup_layout();
        let fonts = self.assets.clonk_fonts.clone();
        let actions = layout
            .as_ref()
            .zip(fonts.as_deref())
            .and_then(|(layout, fonts)| {
                self.league_signup_dialog.as_mut().map(|dialog| {
                    dialog.controller.apply_edit_context_command(
                        field,
                        command,
                        clipboard.as_deref(),
                        layout,
                        &fonts.text,
                    )
                })
            })
            .unwrap_or_default();
        self.process_league_signup_actions(actions)?;
        self.mark_menu_dirty();
        Ok(())
    }

    pub(crate) fn activate_prepared_network_host(
        &mut self,
        scenario: FrontendScenario,
        bind_addr: SocketAddr,
    ) {
        if self.startup_network_connection.is_some() {
            self.status_text = "A network connection is already in progress".to_string();
            return;
        }
        let staged = self.staged_network_host_scenario.as_ref().filter(|staged| {
            staged.frontend.identifier == scenario.identifier
                && staged.frontend.path == scenario.path
        });
        let Some(staged) = staged else {
            self.status_text =
                "Unable to prepare network game: staged scenario resources are unavailable"
                    .to_string();
            return;
        };
        let staged_identity = Some((staged.lobby.local_name.as_str(), staged.lobby.nick.as_str()));
        let preparation = match build_network_host_preparation(
            self,
            &scenario,
            &staged.definition_load,
            &staged.effective_definition_modules,
            &staged.definition_resources,
            Some((&staged.definition_executable_path, &staged.definition_path)),
            staged_identity,
        ) {
            Ok(preparation) => preparation,
            Err(error) => {
                self.status_text = format!("Unable to prepare network game: {error}");
                return;
            }
        };
        let selected_scenario = Some((scenario.identifier.clone(), scenario.title.clone()));
        self.startup_game_search = None;
        let (sender, receiver) = mpsc::channel();
        let local_owner = self.local_owner;
        let spawn = thread::Builder::new()
            .name("lc-prepare-network-host".to_string())
            .spawn(move || {
                let result = preparation
                    .prepare()
                    .map_err(|error| {
                        NetworkStartError::Other(format!("host preparation failed: {error}"))
                    })
                    .and_then(|prepared| {
                        let mode = NetworkMode::Host(HostSettings {
                            bind_addr,
                            player_name: native_bytes_as_legacy_text(
                                prepared.host_config().local_core.name.as_bytes(),
                            ),
                            prepared: Some(prepared),
                        });
                        NetworkManager::for_mode(mode.clone(), local_owner)
                            .map(|manager| (mode, manager))
                    });
                let _ = sender.send(result);
            });
        match spawn {
            Ok(_) => {
                match self.begin_startup_network_connection(
                    receiver,
                    StartupNetworkPurpose::StagedHost,
                    selected_scenario,
                    None,
                ) {
                    Ok(()) => {
                        self.status_text = "Preparing network game…".to_string();
                    }
                    Err(error) => {
                        self.status_text = format!("Unable to start network preparation: {error}");
                    }
                }
            }
            Err(error) => {
                self.status_text = format!("Unable to start network preparation: {error}");
            }
        }
    }

    pub(crate) fn activate_network_join(&mut self, address: String) -> Result<(), EngineError> {
        if self.startup_network_connection.is_some() {
            self.status_text = "A network connection is already in progress".to_string();
            return Ok(());
        }
        if let Err(error) = self.freeze_configured_client_players_for_game() {
            self.status_text = format!("Unable to load configured players: {error}");
            return Ok(());
        }
        self.prepare_network_join_game_state();
        self.startup_game_search = None;
        let local_owner = self.local_owner;
        let player_name = self.player_name.clone();
        let app_paths = self.app_paths.clone();
        let group_maker = self
            .configured_client_player_selection
            .as_ref()
            .map(|selection| selection.group_maker().clone());
        let (_, default_port) = load_network_startup_settings(self.app_paths.as_ref());
        let connect_target = address.clone();
        let spawn = spawn_startup_network_attempt("lc-startup-network", move |cancellation| {
            let server_addr = resolve_join_socket(&address, default_port).map_err(|error| {
                NetworkStartError::Other(format!("invalid network address: {error:#}"))
            })?;
            if cancellation.is_cancelled() {
                return Err(NetworkStartError::Cancelled);
            }
            let mut settings =
                client_settings_for_paths(server_addr, player_name, app_paths.as_ref());
            if let Some(group_maker) = group_maker {
                settings.group_maker = group_maker;
            }
            let mode = NetworkMode::Client(settings.clone());
            NetworkManager::for_client_cancellable(settings, local_owner, cancellation)
                .map(|manager| (mode, manager))
        });
        match spawn {
            Ok((receiver, attempt)) => {
                self.begin_cancellable_startup_network_connection(
                    receiver,
                    attempt,
                    StartupNetworkPurpose::Join,
                    None,
                    Some(connect_target),
                )?;
            }
            Err(error) => {
                self.status_text = format!("Unable to start network worker: {error}");
            }
        }
        Ok(())
    }

    pub(crate) fn activate_network_reference_join(
        &mut self,
        reference: clonk_network::NetworkGameReference,
    ) -> Result<(), EngineError> {
        if self.startup_network_connection.is_some() {
            self.status_text = "A network connection is already in progress".to_string();
            return Ok(());
        }
        if let Err(error) = self.freeze_configured_client_players_for_game() {
            self.status_text = format!("Unable to load configured players: {error}");
            return Ok(());
        }
        self.prepare_network_join_game_state();
        let route_plan = reference.join_route_plan_for_local_host();
        let netpuncher_address = reference.netpuncher_address.clone();
        let netpuncher_game_ids = clonk_network::NetpuncherGameIds {
            ipv4: reference.netpuncher_ipv4,
            ipv6: reference.netpuncher_ipv6,
        };
        let mut settings = client_settings_for_paths(
            reference.source_address,
            self.player_name.clone(),
            self.app_paths.as_ref(),
        );
        if let Some(selection) = self.configured_client_player_selection.as_ref() {
            settings.group_maker = selection.group_maker().clone();
        }
        let settings = settings
            .with_compatibility_build(reference.build)
            .with_join_route_plan(route_plan)
            .with_netpuncher(netpuncher_address, netpuncher_game_ids);
        self.startup_game_search = None;
        self.pending_network_join = Some(settings);
        if reference.password_needed {
            self.open_network_join_password_dialog()?;
        } else {
            self.launch_pending_network_join()?;
        }
        Ok(())
    }

    /// Mirrors the game-state reset immediately before C++ starts either a
    /// reference-backed or unresolved direct join. The synchronized client
    /// load replaces this seed with the host's exact fixed resources later.
    fn prepare_network_join_game_state(&mut self) {
        self.startup_restart_diagnostics.begin_game_init();
        self.clear_lobby_preload();
        self.active_scenario = None;
        let definition_load = self.take_scenario_seed_definition_load();
        self.active_definition_load = Some(definition_load);
        self.active_description_definition_modules.clear();
    }

    pub(crate) fn launch_pending_network_join(&mut self) -> Result<(), EngineError> {
        let Some(settings) = self.pending_network_join.clone() else {
            self.status_text = "Network join settings are unavailable".to_string();
            return Ok(());
        };
        let connect_targets = startup_network_connect_targets(&settings);
        let local_owner = self.local_owner;
        let spawn = spawn_startup_network_attempt("lc-startup-network", move |cancellation| {
            let mode = NetworkMode::Client(settings.clone());
            NetworkManager::for_client_cancellable(settings, local_owner, cancellation)
                .map(|manager| (mode, manager))
        });
        match spawn {
            Ok((receiver, attempt)) => {
                if let Err(error) = self.begin_cancellable_startup_network_connection(
                    receiver,
                    attempt,
                    StartupNetworkPurpose::Join,
                    None,
                    Some(connect_targets),
                ) {
                    self.pending_network_join = None;
                    return Err(error);
                }
            }
            Err(error) => {
                self.pending_network_join = None;
                self.status_text = format!("Unable to start network worker: {error}");
            }
        }
        Ok(())
    }

    pub(crate) fn start_prepared_network_game_advertiser(
        &mut self,
        prepared: &prepared_host_bootstrap::PreparedHostBootstrap,
        network: &NetworkManager,
    ) {
        // InitLocal snapshots the canonical parameters and live admission only
        // after Players.Init/AllowJoin, then the reference server exposes that
        // complete value (src/C4Network2Reference.cpp:49-109;
        // src/C4Game.cpp:3869-3876).
        let (game_ids, addresses) = network.netpuncher_state();
        let reference = match prepared
            .initial_host_game_reference(true, &addresses)
            .map_err(|error| error.to_string())
            .and_then(|reference| {
                reference
                    .replacing_netpuncher_state(game_ids, addresses)
                    .map_err(|error| error.to_string())
            }) {
            Ok(reference) => reference,
            Err(error) => {
                tracing::warn!(%error, "exact network game reference unavailable");
                self.network_game_advertiser = None;
                self.advertised_game_reference = None;
                self.host_reference_paused = false;
                return;
            }
        };
        let config = load_network_advertiser_settings(self.app_paths.as_ref());
        self.start_network_game_advertiser_with_reference(config, reference);
    }

    pub(crate) fn start_network_game_advertiser_with_reference(
        &mut self,
        config: clonk_network::NetworkGameAdvertiserConfig,
        reference: clonk_network::HostGameReference,
    ) {
        // The validated InitLocal value is game state, not socket state. Keep
        // it even when the optional LAN/reference listener cannot bind so a
        // later runtime or game-over rebuild never falls back to stale data.
        self.advertised_game_reference = Some(reference.clone());
        self.host_reference_paused = false;
        match clonk_network::NetworkGameAdvertiser::start_exact(config, reference.clone()) {
            Ok(advertiser) => {
                self.network_game_advertiser = Some(advertiser);
            }
            Err(error) => {
                tracing::warn!(%error, "network game advertising unavailable");
                self.network_game_advertiser = None;
            }
        }
    }

    fn update_host_netpuncher_reference(
        &mut self,
        game_ids: clonk_network::NetpuncherGameIds,
        addresses: Vec<clonk_network::NetworkAddress>,
    ) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        if let Some(network) = self.network.as_ref() {
            if let Err(error) = network.invalidate_league_reference() {
                tracing::error!(%error, "failed to invalidate netpuncher league reference");
            }
        }
        let Some(reference) = self.advertised_game_reference.clone() else {
            return;
        };
        let updated = match reference.replacing_netpuncher_state(game_ids, addresses) {
            Ok(updated) => updated,
            Err(error) => {
                tracing::error!(%error, "failed to rebuild netpuncher host reference");
                return;
            }
        };
        self.advertised_game_reference = Some(updated.clone());
        if let Some(advertiser) = self.network_game_advertiser.as_ref() {
            if let Err(error) = advertiser.update_exact(&updated) {
                tracing::error!(%error, "failed to publish netpuncher host reference");
            }
        }
    }

    pub(crate) fn publish_running_host_reference(&mut self) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        let Some(template) = self.advertised_game_reference.clone() else {
            return;
        };
        let parameters = self
            .host_join_snapshot
            .as_ref()
            .map(|snapshot| snapshot.parameters.clone())
            .unwrap_or_else(|| template.parameters().clone());
        let max_players = self
            .engine
            .max_players()
            .unwrap_or_else(|| i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
        let join_allowed = self
            .network_mode
            .as_ref()
            .and_then(|mode| match mode {
                NetworkMode::Host(HostSettings {
                    prepared: Some(prepared),
                    ..
                }) => Some(prepared.admission().runtime_join_allowed()),
                NetworkMode::Host(_) | NetworkMode::Client(_) => None,
            })
            .unwrap_or(template.summary().join_allowed);
        let control_mode = self.runtime_network_control_mode;
        let updated = match running_host_reference(
            &template,
            parameters,
            &self.control_clients,
            &self.control_player_infos,
            self.engine.teams(),
            max_players,
            if self.host_reference_paused {
                "Paused"
            } else {
                "Running"
            },
            join_allowed,
            &self.snapshot,
        )
        .and_then(|reference| match control_mode {
            Some(control_mode) => reference.replacing_control_mode(control_mode),
            None => Ok(reference),
        }) {
            Ok(reference) => reference,
            Err(error) => {
                tracing::error!(%error, "failed to rebuild running host reference");
                return;
            }
        };

        // Commit the validated state independently of advertiser I/O.
        self.advertised_game_reference = Some(updated.clone());
        if let Some(advertiser) = self.network_game_advertiser.as_ref() {
            if let Err(error) = advertiser.update_exact(&updated) {
                tracing::error!(%error, "failed to update running host reference");
            }
        }
    }

    pub(crate) fn publish_game_over_host_reference(&mut self) {
        let config = load_network_advertiser_settings(self.app_paths.as_ref());
        self.publish_game_over_host_reference_with_config(config);
    }

    pub(crate) fn clear_remembered_league_password(&mut self) {
        if let Some(auth) = self.league_auth_session.as_mut() {
            auth.password = LegacyCString::default();
        }
        if let Some(NetworkMode::Client(settings)) = self.network_mode.as_mut() {
            settings.league_auth.password = LegacyCString::default();
        }
    }

    pub(crate) fn install_prepared_host_material_resources(
        &mut self,
        prepared: &PreparedHostBootstrap,
    ) {
        self.network_material_resource_groups = Some(prepared.material_resource_groups().to_vec());
    }

    pub(crate) fn start_network_game_now(&mut self) -> Result<(), EngineError> {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            self.status_text = "Only the host can start the game".to_string();
            return Ok(());
        }
        if self.pending_lobby_internet_signup.is_some() {
            if self.status_text.is_empty() {
                self.status_text =
                    "Unable to start network game while Internet signup is changing".to_string();
            }
            return Ok(());
        }
        if let Some(task) = self.lobby_preload_task.as_mut() {
            // C++ blocks InitGame on PreloadMutex. Keep the lobby responsive
            // while the worker runs, then resume this exact start request.
            task.start_host_when_ready = true;
            return Ok(());
        }
        let classic_start = self.classic_host_lobby.is_some();
        let prepared = self.network_mode.as_ref().and_then(|mode| match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => Some(prepared.clone()),
            NetworkMode::Host(_) | NetworkMode::Client(_) => None,
        });
        if let Some(prepared) = prepared {
            let Some((
                restore_player_infos,
                random_seed,
                serialized_startup_player_count,
                use_fair_crew,
                fair_crew_strength,
                fair_crew_forced,
                allow_debug,
                auto_frame_skip,
                synchronized_rule_goal_lists,
                team_configuration,
                team_registry,
            )) = self
                .host_join_snapshot
                .as_ref()
                .or_else(|| prepared.host_config().initial_join_snapshot.as_ref())
                .map(|snapshot| {
                    (
                        player_info_list_entries(&snapshot.parameters.restore_player_infos)
                            .collect::<Vec<_>>(),
                        u64::from(snapshot.parameters.random_seed as u32),
                        snapshot.parameters.startup_player_count,
                        snapshot.parameters.use_fair_crew,
                        snapshot.parameters.fair_crew_strength,
                        snapshot.parameters.fair_crew_forced,
                        snapshot.parameters.allow_debug,
                        snapshot.parameters.auto_frame_skip,
                        synchronized_rule_goal_lists(&snapshot.parameters),
                        synchronized_team_configuration(&snapshot.parameters),
                        runtime_teams_from_join_snapshot(&snapshot.parameters.teams),
                    )
                })
            else {
                self.status_text =
                    "Unable to start prepared host: initial JoinData is missing".to_string();
                return Ok(());
            };
            // C++ leaves the lobby through Network.Start: request GS_Go,
            // then apply NoRuntimeJoin admission during DoLobby teardown and
            // initialize the already-opened scenario
            // (src/C4Network2.cpp:510-530;
            // src/C4GameLobby.cpp:442-472). Reopening the source here
            // would diverge from the JoinData already sent to peers.
            let Some(scenario) = self.network_start_scenario() else {
                return Ok(());
            };
            let scenario_load = match prepared.claim_scenario_load() {
                Ok(load) => load,
                Err(error) => {
                    self.status_text = format!("Unable to start prepared host: {error}");
                    return Ok(());
                }
            };
            let initial_game_data = prepared.initial_game_data().clone();
            let startup_player_count = startup_player_count_for_init(
                initial_game_data.frame,
                Some(serialized_startup_player_count),
                Some(
                    i32::try_from(self.control_player_infos.nonremoved_player_count())
                        .unwrap_or(i32::MAX),
                ),
            )
            .unwrap_or(serialized_startup_player_count);
            let host_first_part_preloaded =
                self.lobby_preload_artifact
                    .as_ref()
                    .is_some_and(|artifact| {
                        artifact.catalog_host.is_none() && artifact.client.is_none()
                    });
            let use_lobby_preload =
                host_first_part_preloaded && !scenario_load.retained().uses_map_player_extend();
            let target_tick =
                i32::try_from(self.local_control_submission_tick()).unwrap_or(i32::MAX);
            let status = clonk_network::NetworkStatus {
                state: clonk_network::NETWORK_STATE_GO,
                control_mode: prepared.host_config().initial_status.control_mode,
                target_tick,
            };
            let Some(network) = self.network.as_ref() else {
                self.status_text = "Prepared host network is unavailable".to_string();
                return Ok(());
            };
            // One host-loop command owns both the Go barrier and the policy
            // installed as the lobby closes. Separate FIFO commands leave an
            // admission branch able to accept a late join between them.
            if let Err(error) =
                network.begin_go(status, prepared.admission().runtime_join_allowed())
            {
                self.status_text = format!("Unable to start prepared host: {error}");
                return Ok(());
            }
            if let Some(clock) = self.network_control_clock.as_mut() {
                clock.set_target_tick(Some(target_tick));
            }
            if classic_start {
                // Deleting the native lobby also deletes any ComboBox-owned
                // recursive menu. Do this before dropping the controller so
                // no invisible popup can retain input during Loading.
                self.close_context_menu_silently();
            }
            if !classic_start {
                self.play_ui_sound("Click");
            }
            self.fade_out_game_music();
            self.status_text.clear();
            let (sender, receiver) = mpsc::channel();
            if use_lobby_preload {
                let mut reporter = ScenarioLoadingReporter::new(sender);
                reporter.report(10, "Graphics resources initialized");
                reporter.send(ScenarioLoadingEvent::Finished(Ok(
                    scenario_load.into_retained()
                )));
            } else {
                let scenario_title = scenario.title.clone();
                let spawn_failure_sender = sender.clone();
                let spawn_failure_title = scenario_title.clone();
                if let Err(error) = thread::Builder::new()
                    .name("NetworkScenarioLoad".to_string())
                    .spawn(move || {
                        let mut reporter = ScenarioLoadingReporter::new(sender);
                        if host_first_part_preloaded {
                            reporter.report(10, "Graphics resources initialized");
                        }
                        let result = scenario_load
                            .load_with_progress(
                                random_seed,
                                startup_player_count,
                                |progress, line| {
                                    // OpenScenario's 4 belongs before the
                                    // lobby. A completed preload already ran
                                    // the first part, but MapPlayerExtend
                                    // resumes at the landscape.
                                    let visible = if host_first_part_preloaded {
                                        progress >= 88
                                    } else {
                                        progress >= 8
                                    };
                                    if visible {
                                        reporter.report(progress, line);
                                    }
                                },
                            )
                            .map_err(|error| format!("Failed to load {scenario_title}: {error}"));
                        reporter.send(ScenarioLoadingEvent::Finished(result));
                    })
                {
                    let message = format!("Failed to launch {spawn_failure_title} loader: {error}");
                    let _ = spawn_failure_sender.send(ScenarioLoadingEvent::Finished(Err(message)));
                }
            }
            let mut loading = ScenarioLoadingState::from_network_receiver(
                scenario,
                receiver,
                status,
                restore_player_infos,
                Some(initial_game_data),
                random_seed,
                use_fair_crew,
                fair_crew_strength,
                fair_crew_forced,
                allow_debug,
                auto_frame_skip,
                synchronized_rule_goal_lists,
                team_configuration,
                team_registry,
            );
            loading
                .prepared_go
                .as_mut()
                .expect("network loading always stages the Go boundary")
                .definition_modules = Some(prepared.definition_modules().to_vec());
            self.install_prepared_host_material_resources(&prepared);
            if let Some(staged) = self.staged_network_host_scenario.take() {
                loading.refreshed_resources = Some(staged.loader_refreshed_resources);
                loading.refreshed_tooltip_font = staged.loader_refreshed_tooltip_font;
                loading.refreshed_native_font_source = staged.loader_refreshed_native_font_source;
                loading.refreshed_global_gui_failures = Some(staged.pending_global_gui_failures);
                loading.refreshed_gui_sheet_overrides = Some(staged.pending_gui_sheet_overrides);
                loading.refresh_requested = true;
            }
            self.loading_state = Some(loading);
            // InitNetworkHost returns from DoLobby at 7, immediately before
            // InitGame begins its staged work (src/C4Game.cpp:438-457).
            self.apply_scenario_loader_frame(7, None);
            self.begin_network_start_wait(status);
            self.host_lobby_countdown = None;
            self.pending_local_lobby_countdown_echoes.clear();
            self.classic_host_lobby = None;
            self.network_lobby = None;
            self.mode = AppMode::Loading;
            return Ok(());
        }
        let Some(lobby) = self.network_lobby.as_ref() else {
            return Ok(());
        };
        let Some(identifier) = lobby.selected_identifier() else {
            self.status_text = "Select a scenario before starting".to_string();
            return Ok(());
        };
        let scenario = match self.scenario_catalog.get(identifier).cloned() {
            Some(scenario) => scenario,
            None => {
                self.status_text =
                    format!("Scenario `{}` is not available in the catalog", identifier);
                return Ok(());
            }
        };
        if !classic_start {
            self.play_ui_sound("Click");
        }
        self.start_scenario(scenario)?;
        Ok(())
    }

    fn network_start_scenario(&mut self) -> Option<FrontendScenario> {
        if let Some(staged) = self.staged_network_host_scenario.as_ref() {
            return Some(staged.frontend.clone());
        }
        let Some(lobby) = self.network_lobby.as_ref() else {
            self.status_text = "Network lobby state is unavailable".to_string();
            return None;
        };
        let Some(identifier) = lobby.selected_identifier() else {
            self.status_text = "Select a scenario before starting".to_string();
            return None;
        };
        match self.scenario_catalog.get(identifier).cloned() {
            Some(scenario) => Some(scenario),
            None => {
                self.status_text =
                    format!("Scenario `{identifier}` is not available in the catalog");
                None
            }
        }
    }

    pub(crate) fn begin_network_start_wait(&mut self, status: clonk_network::NetworkStatus) {
        let clients = self
            .control_clients
            .snapshot()
            .into_iter()
            .filter_map(|client| {
                (client.client_id != 0).then(|| {
                    let name = client.name.to_string_lossy().into_owned();
                    clonk_frontend::network_start_wait::NetworkStartWaitClient::new(
                        client.client_id,
                        if name.is_empty() {
                            format!("Client {}", client.client_id)
                        } else {
                            name
                        },
                        clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Loading,
                    )
                })
            });
        self.network_start_wait = Some(NetworkStartWaitDialogState {
            controller: clonk_frontend::network_start_wait::NetworkStartWaitState::with_clients(
                clients,
            ),
            expected_status: status,
            visible: false,
            pointer: None,
        });
    }

    pub(crate) fn show_reached_network_start_wait(&mut self) -> Result<(), EngineError> {
        if matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            if let Some(wait) = self.network_start_wait.as_mut() {
                wait.visible = true;
            }
            self.mark_menu_dirty();
            return Ok(());
        }

        let dialog = clonk_frontend::message_dialog::MessageDialogState::new(
            self.runtime_resource_text("IDS_NET_WAITFORSTART", "Waiting for start..."),
            self.runtime_resource_text("IDS_NET_CAPTION", "Network"),
            clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
            clonk_frontend::message_dialog::MessageDialogIcon::Standard(3),
            clonk_frontend::message_dialog::MessageDialogSize::Small,
            false,
        )
        .without_focus();
        self.push_message_dialog(dialog, MessageDialogContinuation::NetworkClientStartWait)
    }

    fn dismiss_network_client_start_wait(&mut self) {
        let Some(index) = self.message_dialogs.iter().rposition(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::NetworkClientStartWait
            )
        }) else {
            return;
        };
        if self.remove_message_dialog_at(index).is_some() {
            self.startup_tooltip.pointer_left();
            self.mark_menu_dirty();
        }
    }

    fn retarget_network_start_wait(&mut self, status: clonk_network::NetworkStatus) {
        let Some(wait) = self.network_start_wait.as_mut() else {
            return;
        };
        if wait.expected_status == status {
            return;
        }
        wait.expected_status = status;
        let clients = wait.controller.clients().to_vec();
        for client in clients {
            if client.status
                != clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Kick
            {
                wait.controller.update_client_status(
                    client.client_id,
                    clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Loading,
                );
            }
        }
        self.mark_menu_dirty();
    }

    pub(crate) fn update_network_start_wait_ack(
        &mut self,
        client_id: ClientId,
        status: clonk_network::NetworkStatus,
    ) {
        let Ok(client_id) = i32::try_from(client_id) else {
            return;
        };
        let Some(wait) = self.network_start_wait.as_mut() else {
            return;
        };
        if status.state != wait.expected_status.state
            || status.target_tick < wait.expected_status.target_tick
        {
            return;
        }
        if status.target_tick > wait.expected_status.target_tick {
            wait.expected_status.target_tick = status.target_tick;
            let clients = wait.controller.clients().to_vec();
            for client in clients {
                if client.status
                    != clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Kick
                {
                    wait.controller.update_client_status(
                        client.client_id,
                        clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Loading,
                    );
                }
            }
            if let Some(pending) = self
                .loading_state
                .as_mut()
                .and_then(|loading| loading.prepared_go.as_mut())
            {
                pending.status.target_tick = status.target_tick;
            }
        }
        if wait.controller.update_client_status(
            client_id,
            clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Ready,
        ) {
            self.mark_menu_dirty();
        }
    }

    fn mark_network_start_wait_client_kick(&mut self, client_id: ClientId) {
        let Ok(client_id) = i32::try_from(client_id) else {
            return;
        };
        if self.network_start_wait.as_mut().is_some_and(|wait| {
            wait.controller.update_client_status(
                client_id,
                clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Kick,
            )
        }) {
            self.mark_menu_dirty();
        }
    }

    pub(crate) fn network_game_start_guard_passes(&mut self) -> bool {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            self.status_text = "Only the host can start the game".to_string();
            return false;
        }
        if self.pending_lobby_internet_signup.is_some() {
            if self.status_text.is_empty() {
                self.status_text =
                    "Unable to start network game while Internet signup is changing".to_string();
            }
            return false;
        }
        if self.network_mode.as_ref().is_some_and(|mode| match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => prepared.host_config().initial_join_snapshot.is_none(),
            NetworkMode::Host(_) | NetworkMode::Client(_) => false,
        }) {
            self.status_text =
                "Unable to start prepared host: initial JoinData is missing".to_string();
            return false;
        }
        if self.staged_network_host_scenario.is_some() || self.classic_host_lobby.is_some() {
            return true;
        }
        let Some(lobby) = self.network_lobby.as_ref() else {
            self.status_text = "Network lobby state is unavailable".to_string();
            return false;
        };
        let Some(identifier) = lobby.selected_identifier() else {
            self.status_text = "Select a scenario before starting".to_string();
            return false;
        };
        if !self.scenario_catalog.contains_key(identifier) {
            self.status_text = format!("Scenario `{identifier}` is not available in the catalog");
            return false;
        }
        true
    }

    pub(crate) fn finish_classic_command_line_host_entry(&mut self) -> Result<(), EngineError> {
        if self.classic_command_line.scenario.is_some()
            && self.classic_command_line.network_active == Some(true)
            && self.classic_command_line.lobby_timeout.is_none()
        {
            // C4Game::InitNetworkHost calls Network.Start immediately unless
            // `/lobby` set fLobby. The prepared Rust host is installed through
            // its lobby state first, then requests start in this same update.
            // Preloading may defer the state transition, but never presents a
            // staging-lobby frame.
            self.start_network_game_now()?;
            if self
                .lobby_preload_task
                .as_ref()
                .is_some_and(|task| task.start_host_when_ready)
                && self.mode == AppMode::Menu
            {
                self.replace_startup_view(StartupView::NetworkGame);
                self.mode = AppMode::Loading;
            }
            Ok(())
        } else {
            self.start_classic_command_line_lobby_timeout()
        }
    }

    pub(crate) fn network_start_wait_layout(
        &self,
    ) -> Option<clonk_frontend::network_start_wait::NetworkStartWaitLayout> {
        let wait = self
            .network_start_wait
            .as_ref()
            .filter(|wait| wait.visible)?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.graphics.surface();
        Some(
            wait.controller
                .layout(surface.width() as i32, surface.height() as i32, fonts),
        )
    }

    pub(crate) fn play_network_start_wait_sounds(&mut self) {
        let sounds = self
            .network_start_wait
            .as_mut()
            .map(|wait| wait.controller.take_sound_events())
            .unwrap_or_default();
        for sound in sounds {
            self.play_ui_sound(match sound {
                clonk_frontend::network_start_wait::NetworkStartWaitSound::ArrowHit => "ArrowHit",
                clonk_frontend::network_start_wait::NetworkStartWaitSound::Click => "Click",
            });
        }
    }

    pub(crate) fn process_network_start_wait_actions(
        &mut self,
        actions: Vec<clonk_frontend::network_start_wait::NetworkStartWaitAction>,
    ) -> Result<(), EngineError> {
        self.play_network_start_wait_sounds();
        let Some(action) = actions.into_iter().next() else {
            return Ok(());
        };
        match action {
            clonk_frontend::network_start_wait::NetworkStartWaitAction::Restart => {
                self.network_start_wait = None;
                self.restart_current_network_scenario();
            }
            clonk_frontend::network_start_wait::NetworkStartWaitAction::Cancel => {
                self.network_start_wait = None;
                self.return_to_menu();
            }
            clonk_frontend::network_start_wait::NetworkStartWaitAction::Kick(client_id) => {
                self.kick_classic_lobby_client(client_id);
            }
        }
        Ok(())
    }

    pub(crate) fn restart_current_network_scenario(&mut self) {
        let Some(scenario) = self.active_scenario.clone() else {
            self.return_to_menu();
            return;
        };
        let definition_load = self
            .active_definition_load
            .clone()
            .unwrap_or_else(|| self.scenario_seed_definition_load());
        let mut values = self.scenario_game_options.values().clone();
        values.countdown = false;
        values.lobby_is_league = false;
        self.retain_restart_restore_mask_for_restart();
        self.return_to_menu_for_relaunch();
        self.scenario_game_options =
            GameOptionButtons::new(GameOptionContext::NetworkHostSelector, values);
        self.scenario_selector_mode = ScenarioSelectorMode::NetworkHost;
        self.stage_network_host_scenario(scenario, definition_load);
    }

    pub(crate) fn catalog_host_preload_scenario(&self) -> Option<&FrontendScenario> {
        if !matches!(self.network_mode.as_ref(), Some(NetworkMode::Host(_))) {
            return None;
        }
        let lobby = self.network_lobby.as_ref().filter(|lobby| lobby.is_host)?;
        self.scenario_catalog.get(lobby.selected_identifier()?)
    }

    pub(crate) fn clear_client_preload_projection(&mut self) {
        self.client_combined_preload_file.clear();
        self.client_combined_scenario_path = None;
        self.network_material_resource_groups = None;
    }

    pub(crate) fn open_network_host_scenario_browser(&mut self) {
        self.open_scenario_browser_with_mode(ScenarioSelectorMode::NetworkHost);
    }

    pub(crate) fn prune_host_local_alternate_colors(&mut self) {
        let mut retained_ids = HashSet::new();
        let mut retained_resources = HashSet::new();
        for &info_id in &self.host_local_player_info_ids {
            let Some(player) = self.control_player_infos.get(info_id) else {
                continue;
            };
            if player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0 {
                continue;
            }
            let Some(resource_id) = player.resource.as_ref().map(|resource| resource.id) else {
                continue;
            };
            if self
                .host_local_alternate_colors_by_resource
                .contains_key(&resource_id)
            {
                retained_ids.insert(info_id);
                retained_resources.insert(resource_id);
            }
        }
        self.host_local_player_info_ids = retained_ids;
        self.host_local_alternate_colors_by_resource
            .retain(|resource_id, _| retained_resources.contains(resource_id));
    }

    pub(crate) fn remove_runtime_players_at_client(&mut self, client_id: i32, disconnected: bool) {
        let info_ids: HashSet<i32> = self
            .control_player_infos
            .client_info_ids(client_id)
            .into_iter()
            .collect();
        let runtime_players: Vec<(i32, i32)> = self
            .engine
            .snapshot()
            .players
            .into_iter()
            .filter(|player| info_ids.contains(&player.player_info_id))
            .map(|player| (player.id, player.player_info_id))
            .collect();
        let game_part_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        for (player_id, info_id) in runtime_players {
            match self.remove_runtime_player_with_viewport_feedback(player_id) {
                Ok(()) => {
                    self.control_player_infos
                        .mark_removed(info_id, disconnected, game_part_frame);
                }
                Err(error) => {
                    tracing::warn!(%player_id, %info_id, %error, "failed to remove client player");
                }
            }
        }
        self.prune_host_local_alternate_colors();
    }

    pub(crate) fn change_network_control_to_local(&mut self, local_client_id: i32) {
        // C4GameControl::ChangeToLocal preserves FrameCounter, ControlTick and
        // Game.Parameters while changing only the cadence to ControlRate=1
        // (C4GameControl.cpp:93-127).
        let game_over_dialog_shown = self.game_over_dialog.is_some();
        self.finalize_pending_league_end_for_teardown();
        self.clear_lobby_preload();
        let control_tick = self.engine.sync_check(local_client_id).control_tick;
        self.remove_remote_runtime_players(local_client_id);
        self.snapshot.round_results = self.engine.snapshot().round_results;
        // RemoveRemote callbacks still run while C4GameControl is in network
        // mode. Apply any SetPreSend effects they produced before clearing the
        // live clock and client-name registry below.
        if let Err(error) = self.apply_engine_network_target_fps_requests() {
            tracing::error!(%error, "failed to apply network pacing before ChangeToLocal");
        }
        if let Ok(timing) = clonk_engine::NetworkControlTiming::new(control_tick, 1) {
            self.engine.initialize_network_control_timing(timing);
        }
        self.abandon_live_masterserver_signup();
        self.network = None;
        self.network_mode = None;
        self.runtime_client_list = None;
        self.remove_running_dialog(RunningDialogStackEntry::RuntimeClientList);
        self.runtime_client_list_consumed_keys.clear();
        self.hide_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
        self.control_messages.clear_clients();
        self.network_game_advertiser = None;
        self.advertised_game_reference = None;
        self.host_reference_paused = false;
        self.runtime_network_control_mode = None;
        self.runtime_network_committed_control_mode = None;
        self.runtime_network_committed_status = None;
        self.runtime_network_join_allowed = None;
        self.host_join_snapshot = None;
        self.pending_runtime_dynamic_request = None;
        self.network_lobby = None;
        self.network_start_wait = None;
        self.host_lobby_countdown = None;
        self.pending_local_lobby_countdown_echoes.clear();
        self.network_control_clock = None;
        self.network_ticks.clear();
        self.network_sync.clear();
        self.offline_control_input.clear();
        self.sync_checks.clear();
        self.offline_halt_count = i32::from(game_over_dialog_shown);
        // Native does not clear HaltCount while C4GameOverDlg is shown;
        // otherwise a client starts simulating when its host disconnects
        // beneath the evaluation dialog (src/C4GameControl.cpp:121-127).
        self.network_control_running = !game_over_dialog_shown;
        self.runtime_network_status_barrier = None;
        self.league_votes.clear();
        self.clear_blocking_resource_wait();
        self.admission_resources.clear();
        self.host_local_alternate_colors_by_resource.clear();
        self.host_local_player_info_ids.clear();
        self.pending_network_join_data = None;
        self.initial_lobby_status_ack_pending = false;
        self.client_start_barrier = ClientStartBarrier::default();
        self.pending_client_start_status = None;
        self.client_combined_scenario_path = None;
        self.client_combined_preload_file.clear();
        self.network_material_resource_groups = None;
        self.control_clients = ControlClientRegistry::default();
        self.network_client_activity.clear();
        self.control_clients.register(local_client_id, true, false);
        self.engine.set_control_host(true);
        self.engine.set_network_control_mode(false);
    }

    fn request_runtime_join_dynamic(&mut self, client_id: ClientId, current_control_tick: Tick) {
        if !matches!(self.runtime_network_role(), RuntimeNetworkRole::Host) {
            tracing::warn!(
                %client_id,
                current_control_tick,
                "ignoring runtime JoinData request outside an authoritative host session"
            );
            return;
        }

        if self.host_join_snapshot.as_ref().is_some_and(|snapshot| {
            published_runtime_dynamic_covers_request(
                &snapshot.dynamic,
                snapshot.dynamic_tick,
                current_control_tick,
            )
        }) {
            // publish_runtime_dynamic wakes every client that was waiting at
            // publication time. A JoinDataNeeded event already queued behind
            // that synchronization is therefore stale; C++'s second queued
            // Synchronize likewise sees fDynamicNeeded=false and does not
            // create another C4GameSaveNetwork.
            tracing::debug!(
                %client_id,
                current_control_tick,
                "ignoring runtime JoinData request covered by the published dynamic"
            );
            return;
        }

        let should_queue = match self.pending_runtime_dynamic_request.as_mut() {
            Some(pending) => {
                pending.include(client_id, current_control_tick);
                pending.needs_synchronize()
            }
            None => {
                self.pending_runtime_dynamic_request = Some(PendingRuntimeDynamicRequest::new(
                    client_id,
                    current_control_tick,
                ));
                true
            }
        };
        if !should_queue {
            return;
        }

        // C4Network2::SendJoinData sets fDynamicNeeded and queues
        // C4ControlSynchronize(false, true) through CDT_Sync. Any successful
        // Game.Synchronize callback may satisfy the coalesced request.
        let submission = self
            .network
            .as_ref()
            .ok_or_else(|| anyhow!("network manager is unavailable"))
            .and_then(|network| network.submit_synchronize(current_control_tick, false, true));
        match submission {
            Ok(()) => {
                if let Some(pending) = self.pending_runtime_dynamic_request.as_mut() {
                    pending.synchronize_queued = true;
                }
            }
            Err(error) => {
                tracing::error!(
                    %client_id,
                    current_control_tick,
                    %error,
                    "failed to queue runtime JoinData synchronization"
                );
                self.fail_pending_runtime_dynamic_request(error.to_string());
            }
        }
    }

    pub(crate) fn forget_pending_runtime_join_client(&mut self, client_id: ClientId) {
        let remove_request = self
            .pending_runtime_dynamic_request
            .as_mut()
            .is_some_and(|pending| {
                pending.client_ids.remove(&client_id);
                pending.client_ids.is_empty()
            });
        if remove_request {
            self.pending_runtime_dynamic_request = None;
        }
    }

    /// Application-owned counterpart of C4GameSaveNetwork(false) at the exact
    /// C4Network2::OnGameSynchronized seam. The live serializer/resource
    /// publisher plugs in here; `None` keeps the coalesced request attached to
    /// this synchronization tick without republishing the stale dynamic.
    fn capture_runtime_join_snapshot_at_synchronized_boundary(
        &mut self,
        synchronized_control_tick: Tick,
    ) -> std::result::Result<Option<clonk_network::HostJoinSnapshot>, String> {
        tracing::debug!(
            synchronized_control_tick,
            "runtime JoinData serializer/publication boundary reached"
        );

        let prepared = match self.network_mode.as_ref() {
            Some(NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            })) => prepared.clone(),
            Some(NetworkMode::Host(_)) => {
                return Err("runtime JoinData requires a scenario-first prepared host".to_string());
            }
            Some(NetworkMode::Client(_)) | None => {
                return Err("runtime JoinData capture requires the network host".to_string());
            }
        };
        let definition_modules = match self.active_definition_load.as_ref() {
            Some(ScenarioDefinitionLoad::Seed { modules, .. })
            | Some(ScenarioDefinitionLoad::Fixed { modules, .. }) => modules.clone(),
            None => {
                return Err(
                    "runtime JoinData capture has no active definition module order".to_string(),
                );
            }
        };
        let mut parameters = self
            .host_join_snapshot
            .as_ref()
            .map(|snapshot| snapshot.parameters.clone())
            .ok_or_else(|| "runtime JoinData capture has no host parameters".to_string())?;
        let max_players = self
            .engine
            .max_players()
            .unwrap_or_else(|| i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
        parameters = live_host_reference_parameters(
            parameters,
            &self.control_clients,
            &self.control_player_infos,
            self.engine.teams(),
            max_players,
            None,
        );
        let restore_plan = runtime_join_save::set_as_runtime_join_restore_infos(
            &parameters.clients.clients,
            &parameters.player_infos,
        );
        // C4GameSave owns this temporary list only for SavePlayerInfos.txt and
        // embedded player groups. SendJoinData still copies the unchanged
        // Game.Parameters.RestorePlayerInfos into the packet.

        // Prepared host metadata and the synchronized parameter title are the
        // byte-exact values used by C++; the frontend title is presentation
        // Unicode and cannot safely recreate every native byte sequence.
        let title = native_bytes_as_legacy_text(parameters.title.as_bytes());
        let origin = prepared.scenario_origin().to_string();
        let maker = prepared.host_config().group_maker.as_bytes().to_vec();
        let group_filename = prepared.dynamic_filename_seed().to_owned();
        let scenario_defaults = prepared.scenario_defaults().clone();
        let dynamic_tick = self.next_network_control_tick();
        let (definition_executable_path, definition_path) = prepared.definition_save_paths();

        let save = self
            .engine
            .serialize_live_c4_save(clonk_engine::LiveC4SaveSpec {
                title: &title,
                definition_modules: &definition_modules,
                definition_executable_path,
                definition_path,
                origin: &origin,
                music_enabled: self.runtime_music_enabled,
                copied_material_group_is_file: false,
                // The app has no mutable C4ComponentHost counterparts yet;
                // native Save is a no-op while these hosts are unmodified.
                title_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                info_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                script_component: clonk_engine::LiveC4ComponentHost::Unmodified,
            })
            .map_err(|error| format!("serialize synchronized runtime game: {error}"))?;

        // C4PlayerList::Save walks live players in link order and looks each
        // one up in RestoreInfos by stable PlayerInfo ID. Do not inherit the
        // client/player-info list order used to build RestoreInfos.
        let (add_new_crew_portraits, save_default_portraits, player_rank_name_default) =
            self.developer_console_player_save_options();
        let player_save_options = clonk_engine::LiveC4PlayerSaveOptions {
            savegame: true,
            store_tiny: false,
            add_new_crew_portraits,
            save_default_portraits,
            player_rank_name_default: &player_rank_name_default,
        };
        let runtime_players = self
            .engine
            .players()
            .map(|player| (player.id(), player.player_info_id()))
            .collect::<Vec<_>>();
        let mut remaining_targets = restore_plan.player_groups;
        let mut player_groups = Vec::with_capacity(remaining_targets.len());
        for (game_number, player_info_id) in runtime_players {
            let Some(index) = remaining_targets
                .iter()
                .position(|target| target.player_info_id == player_info_id)
            else {
                continue;
            };
            let target = remaining_targets.remove(index);
            let group = clonk_engine::serialize_live_c4_player_with_options_and_enumeration(
                &self.engine,
                game_number,
                target.filename.as_bytes(),
                &maker,
                player_save_options,
                &save.value_enumeration,
            )
            .map_err(|error| {
                format!(
                    "serialize runtime player info {} (game player {}): {error}",
                    target.player_info_id, game_number
                )
            })?;
            player_groups.push(runtime_join_save::SerializedRuntimeJoinPlayerGroup {
                filename: target.filename,
                group,
            });
        }
        let parameter_bytes =
            clonk_network::serialize_initial_network_parameters(&parameters, &scenario_defaults)
                .map_err(|error| format!("serialize synchronized runtime parameters: {error}"))?;
        let dynamic = runtime_join_save::compose_runtime_join_dynamic(
            group_filename,
            maker,
            parameter_bytes,
            save,
            &restore_plan.restore_infos,
            player_groups,
        )
        .map_err(|error| error.to_string())?;
        let dynamic = self
            .network
            .as_ref()
            .ok_or_else(|| "runtime JoinData capture has no network manager".to_string())?
            .publish_runtime_dynamic(dynamic, dynamic_tick, parameters.clone())
            .map_err(|error| format!("publish synchronized runtime dynamic: {error}"))?;

        Ok(Some(clonk_network::HostJoinSnapshot {
            dynamic,
            dynamic_tick,
            parameters,
        }))
    }

    pub(crate) fn on_runtime_join_synchronized(&mut self, synchronized_control_tick: Tick) {
        let requested_control_tick = {
            let Some(pending) = self.pending_runtime_dynamic_request.as_mut() else {
                return;
            };
            pending.synchronize_queued = false;
            pending.synchronized_control_tick = Some(synchronized_control_tick);
            pending.requested_control_tick
        };
        let required_tick = i32::try_from(requested_control_tick).unwrap_or(i32::MAX);
        let dynamic_tick = self.next_network_control_tick();
        if dynamic_tick < required_tick {
            self.fail_pending_runtime_dynamic_request(format!(
                "runtime dynamic tick {dynamic_tick} precedes requested control tick {required_tick}"
            ));
            return;
        }

        match self.capture_runtime_join_snapshot_at_synchronized_boundary(synchronized_control_tick)
        {
            Ok(Some(snapshot)) => {
                self.pending_runtime_dynamic_request = None;
                self.host_join_snapshot = Some(snapshot);
                // publish_runtime_dynamic already installed this exact core
                // and woke every waiting client. Refresh references without
                // queueing a redundant generic JoinData publication.
                self.refresh_published_host_join_snapshot_views();
            }
            Ok(None) => {}
            Err(error) => self.fail_pending_runtime_dynamic_request(error),
        }
    }

    pub(crate) fn publish_updated_host_join_snapshot(&mut self) {
        self.publish_updated_host_join_snapshot_with_network(true);
    }

    fn refresh_published_host_join_snapshot_views(&mut self) {
        self.publish_updated_host_join_snapshot_with_network(false);
    }

    fn publish_updated_host_join_snapshot_with_network(&mut self, publish_to_network: bool) {
        let project_runtime_teams = matches!(self.mode, AppMode::Running);
        let clients =
            clonk_network::JoinClientRegistrySnapshot::new(self.control_clients.snapshot());
        let teams = project_runtime_teams.then(|| self.engine.teams().to_vec());
        if let Some(snapshot) = self.host_join_snapshot.as_mut() {
            // Game.Clients and Game.Teams are live aliases of their
            // C4GameParameters counterparts in native. Materialize those
            // aliases before any later-join or reference serialization.
            snapshot.parameters.clients = clients;
            if let Some(teams) = teams.as_deref() {
                project_live_team_memberships(&mut snapshot.parameters.teams, teams);
            }
        }
        let Some(snapshot) = self.host_join_snapshot.clone() else {
            return;
        };
        if publish_to_network {
            if let Some(network) = self.network.as_ref() {
                if let Err(error) = network.publish_join_snapshot(snapshot.clone()) {
                    tracing::error!(%error, "failed to publish updated host JoinData");
                }
            }
        }
        if let Some(reference) = self.advertised_game_reference.clone() {
            let rebuilt = if self.snapshot.game_over {
                let max_players = self
                    .engine
                    .max_players()
                    .unwrap_or_else(|| i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
                game_over_host_reference(
                    &reference,
                    snapshot.parameters,
                    &self.control_clients,
                    &self.control_player_infos,
                    self.engine.teams(),
                    max_players,
                    &self.snapshot,
                )
            } else {
                reference.replacing_parameters(snapshot.parameters)
            };
            match rebuilt {
                Ok(updated) => {
                    if let Some(advertiser) = self.network_game_advertiser.as_ref() {
                        if let Err(error) = advertiser.update_exact(&updated) {
                            tracing::error!(%error, "failed to publish updated host reference");
                        }
                    }
                    self.advertised_game_reference = Some(updated);
                }
                Err(error) => {
                    tracing::error!(%error, "failed to rebuild updated host reference");
                }
            }
        }
        if let Some(network) = self.network.as_ref() {
            if let Err(error) = network.invalidate_league_reference() {
                tracing::error!(%error, "failed to invalidate league reference");
            }
        }
    }

    /// C++ stores `Game.PlayerInfos` as a reference to
    /// `Game.Parameters.PlayerInfos`, so every authoritative registry mutation
    /// is automatically visible to later JoinData consumers
    /// (src/C4Game.cpp:65-71; src/C4Network2.cpp:1820-1844).
    pub(crate) fn refresh_current_host_player_infos(&mut self) -> bool {
        if !matches!(self.runtime_network_role(), RuntimeNetworkRole::Host) {
            return false;
        }
        let Some(snapshot) = self.host_join_snapshot.as_mut() else {
            return false;
        };
        let (last_player_id, clients) = self.control_player_infos.retained_rows_snapshot();
        snapshot.parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
            last_player_id,
            clients: clients
                .into_iter()
                .map(
                    |(client_id, flags, players)| clonk_network::ClientPlayerInfosSnapshot {
                        client_id,
                        flags,
                        players,
                    },
                )
                .collect(),
        };
        true
    }

    pub(crate) fn publish_current_host_player_infos(&mut self) {
        if self.refresh_current_host_player_infos() {
            self.publish_updated_host_join_snapshot();
        }
    }

    pub(crate) fn execute_league_vote(
        &mut self,
        vote: clonk_engine::VoteControlData,
    ) -> Result<(), EngineError> {
        if !self.control_clients.contains(vote.by_client) {
            return Ok(());
        }
        let subject = LeagueVoteSubject::from(vote);
        self.league_votes.add(vote);
        self.pause_host_for_league_vote();
        self.open_next_league_vote_dialog()?;
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return Ok(());
        }
        let Some(approve) = self.league_vote_decision(subject) else {
            return Ok(());
        };
        if let Some(Err(error)) = self
            .network
            .as_ref()
            .map(|network| network.submit_vote_end(subject.vote_type, approve, subject.data))
        {
            tracing::error!(%error, "failed to submit authoritative league vote result");
        }
        Ok(())
    }

    pub(crate) fn league_vote_description(&self, vote: clonk_engine::VoteControlData) -> String {
        match vote.vote_type {
            clonk_engine::VOTE_TYPE_CANCEL => "abort the round".to_string(),
            clonk_engine::VOTE_TYPE_KICK if vote.data == vote.by_client => {
                "leave the game".to_string()
            }
            clonk_engine::VOTE_TYPE_KICK => {
                format!("kick client {}", self.league_vote_client_name(vote.data))
            }
            clonk_engine::VOTE_TYPE_PAUSE if vote.data != 0 => "pause the game".to_string(),
            clonk_engine::VOTE_TYPE_PAUSE => "continue the game".to_string(),
            _ => "perform some mysterious action".to_string(),
        }
    }

    pub(crate) fn league_vote_client_name(&self, client_id: i32) -> String {
        self.control_clients
            .state(client_id)
            .map(|client| legacy_presentation_text(client.name.as_bytes()))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "???".to_string())
    }

    pub(crate) fn execute_league_vote_end(&mut self, result: clonk_engine::VoteControlData) {
        if result.by_client != 0 {
            return;
        }
        let subject = LeagueVoteSubject::from(result);
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        let origin = self
            .league_votes
            .end(subject, result.approve, local_client_id);
        if let Some(index) = self.message_dialogs.iter().position(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::LeagueVote {
                    subject: active_subject
                } if active_subject == subject
            )
        }) {
            self.remove_message_dialog_at(index);
            self.mark_menu_dirty();
        }
        let rejected_own_cancel = !result.approve
            && origin == local_client_id
            && (result.vote_type == clonk_engine::VOTE_TYPE_CANCEL
                || result.vote_type == clonk_engine::VOTE_TYPE_KICK
                    && result.data == local_client_id.unwrap_or(-1));
        if rejected_own_cancel {
            if let Err(error) = self.open_league_surrender_dialog() {
                tracing::error!(%error, "failed to open league surrender dialog");
            }
        }
        if let Err(error) = self.open_next_league_vote_dialog() {
            tracing::error!(%error, "failed to open next league vote dialog");
        }
        self.finish_host_vote_pause(result);
        if !result.approve {
            return;
        }
        if !self.engine.is_game_over() {
            match result.vote_type {
                clonk_engine::VOTE_TYPE_CANCEL => {
                    self.control_player_infos.mark_voted_out(None);
                }
                clonk_engine::VOTE_TYPE_KICK => {
                    self.control_player_infos.mark_voted_out(Some(result.data));
                }
                _ => {}
            }
        }
        if result.vote_type == clonk_engine::VOTE_TYPE_CANCEL {
            if let Err(error) = self.hard_abort_running_game() {
                tracing::error!(%error, "failed to hard-abort an approved league cancellation");
            }
            return;
        }
        if result.vote_type != clonk_engine::VOTE_TYPE_KICK {
            return;
        }
        let host_removes_target = matches!(self.network_mode, Some(NetworkMode::Host(_)))
            && self.control_clients.contains(result.data);
        if host_removes_target {
            let remove = clonk_engine::ClientRemoveControlData {
                client_id: result.data,
                reason: clonk_engine::LegacyCString::from_bytes(b"voted out".to_vec())
                    .unwrap_or_default(),
                by_client: 0,
            };
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_client_remove(remove))
            {
                tracing::error!(%error, "failed to remove client approved by league vote");
            }
        }
        if local_client_id != Some(result.data) {
            return;
        }
        let local_players = self
            .engine
            .players()
            .filter(|player| player.at_client().get() == result.data)
            .map(|player| player.id())
            .collect::<Vec<_>>();
        self.change_network_control_to_local(result.data);
        for player in local_players {
            if let Err(error) = self.engine.set_player_surrendered(player, true) {
                tracing::error!(player, %error, "failed to end voted-out local player");
            }
        }
        self.status_text = "You have been removed by vote.".to_string();
    }

    pub(crate) fn pause_host_for_league_vote(&mut self) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || self.runtime_network_is_paused()
            || self.league_votes.paused_for_vote
        {
            return;
        }
        let target_tick = self.next_network_control_tick();
        let status = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_PAUSE,
            control_mode: self.league_vote_control_mode(),
            target_tick,
        };
        if let Err(error) = self.change_runtime_network_status(status) {
            tracing::error!(%error, "failed to pause host for league vote");
        }
        self.league_votes.paused_for_vote = true;
        self.host_reference_paused = true;
        self.publish_running_host_reference();
    }

    pub(crate) fn finish_host_vote_pause(&mut self, result: clonk_engine::VoteControlData) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        if result.approve && result.vote_type == clonk_engine::VOTE_TYPE_PAUSE {
            self.league_votes.paused_for_vote = result.data == 0;
        }
        if !self.league_votes.ballots.is_empty() || !self.league_votes.paused_for_vote {
            return;
        }
        let current_tick = self
            .executing_ready_tick
            .unwrap_or_else(|| self.expected_network_control_tick());
        let status = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: self.league_vote_control_mode(),
            target_tick: i32::try_from(current_tick).unwrap_or(i32::MAX),
        };
        if let Err(error) = self.change_runtime_network_status(status) {
            tracing::error!(%error, "failed to restore host after league vote");
        }
        self.league_votes.paused_for_vote = false;
        self.host_reference_paused = false;
        self.publish_running_host_reference();
    }

    fn league_vote_control_mode(&self) -> i32 {
        if let Some(mode) = self.runtime_network_control_mode {
            return mode;
        }
        match self.network_mode.as_ref() {
            Some(NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            })) => prepared.host_config().initial_status.control_mode,
            Some(NetworkMode::Host(_)) | Some(NetworkMode::Client(_)) | None => 0,
        }
    }

    fn runtime_client_list_control_mode(&self) -> i32 {
        if let Some(mode) = self.runtime_network_committed_control_mode {
            return mode;
        }
        match self.network_mode.as_ref() {
            Some(NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            })) => prepared.host_config().initial_status.control_mode,
            Some(NetworkMode::Host(_)) | Some(NetworkMode::Client(_)) | None => 0,
        }
    }

    fn league_vote_decision(&self, subject: LeagueVoteSubject) -> Option<bool> {
        let eligible_players = self
            .engine
            .players()
            .filter_map(|player| {
                let client_id = player.at_client().get();
                (client_id >= 0 && self.control_clients.contains(client_id))
                    .then_some((client_id, player.team()))
            })
            .collect::<Vec<_>>();
        let team_ids = if self.engine.teams().is_empty() {
            vec![None]
        } else {
            self.engine
                .teams()
                .iter()
                .map(|team| Some(team.id))
                .collect::<Vec<_>>()
        };
        let mut positive_teams = 0usize;
        let mut negative_teams = 0usize;
        let mut voting_teams = 0usize;
        for team_id in team_ids {
            let team_players = eligible_players
                .iter()
                .filter(|(_, player_team)| team_id.is_none() || *player_team == team_id)
                .collect::<Vec<_>>();
            if team_players.is_empty() {
                continue;
            }
            voting_teams += 1;
            let (positive, negative) = team_players.iter().fold(
                (0usize, 0usize),
                |(positive, negative), (client_id, _)| match self
                    .league_votes
                    .first_ballot(*client_id, subject)
                {
                    Some(true) => (positive + 1, negative),
                    Some(false) => (positive, negative + 1),
                    None => (positive, negative),
                },
            );
            if positive * 2 > team_players.len() {
                positive_teams += 1;
            } else if negative * 2 >= team_players.len() {
                negative_teams += 1;
            }
        }
        if positive_teams * 2 > voting_teams {
            Some(true)
        } else if negative_teams * 2 >= voting_teams {
            Some(false)
        } else {
            None
        }
    }

    pub(crate) fn apply_join_player_control(
        &mut self,
        join: clonk_engine::JoinPlayerControlData,
    ) -> Result<(), EngineError> {
        let Some(info) = self.control_player_infos.get(join.info_id).cloned() else {
            tracing::warn!(
                info_id = join.info_id,
                "ignoring join for missing player info"
            );
            return Ok(());
        };
        let Some(at_client) = self.control_clients.state(join.at_client) else {
            tracing::warn!(
                info_id = join.info_id,
                at_client = join.at_client,
                "ignoring join for missing controlling client"
            );
            return Ok(());
        };
        let at_client_name = if self.network.is_none() && at_client.name.is_empty() {
            "Local".to_string()
        } else {
            clonk_script::c4_string_from_bytes(at_client.name.as_bytes())
        };
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        let offline_local = self.network.is_none()
            && self.control_playback.is_none()
            && join.at_client == self.offline_local_client_id();
        let locally_controlled =
            !info.is_script_player() && (local_client_id == Some(join.at_client) || offline_local);
        let pending_player_big_icon = match &join.source {
            clonk_engine::JoinPlayerSource::Resource(core) => self
                .admission_resources
                .complete_path(core.id)
                .and_then(|path| {
                    if locally_controlled {
                        load_local_player_big_icon(path)
                    } else {
                        load_network_player_big_icon(path)
                    }
                })
                .or_else(|| {
                    self.control_playback
                        .is_some()
                        .then(|| self.replay_record_player_group(core).ok())
                        .flatten()
                        .and_then(|group| load_player_big_icon_from_group(&group))
                }),
            clonk_engine::JoinPlayerSource::Embedded(_)
                if info.is_script_player() && join.filename.is_empty() =>
            {
                None
            }
            clonk_engine::JoinPlayerSource::Embedded(_)
                if local_client_id == Some(join.by_client)
                    || (offline_local && join.by_client == join.at_client) =>
            {
                let path = PathBuf::from(join.filename.to_string_lossy().into_owned());
                load_local_player_big_icon(&path)
            }
            clonk_engine::JoinPlayerSource::Embedded(data) => load_packed_network_player_big_icon(
                PathBuf::from(join.filename.to_string_lossy().into_owned()),
                data,
            ),
        };
        let player_file = match &join.source {
            clonk_engine::JoinPlayerSource::Resource(core) => {
                // Rust has no stable local temp path to serialize while this
                // resource is loading. Resolve the completed registry entry by
                // ID on the authoring host too, after PreExecute releases it.
                if let Some(path) = self.admission_resources.complete_path(core.id) {
                    match PlayerFile::load_from_path(path) {
                        Ok(file) => Some(file),
                        Err(error) => {
                            tracing::warn!(info_id = join.info_id, path = %path.display(), %error, "failed to load completed player resource");
                            return Ok(());
                        }
                    }
                } else if self.control_playback.is_some() {
                    match self.replay_record_player_file(core) {
                        Ok(file) => Some(file),
                        Err(error) => {
                            tracing::warn!(info_id = join.info_id, resource_id = core.id, %error, "failed to load player resource from replay group");
                            return Ok(());
                        }
                    }
                } else {
                    return Ok(());
                }
            }
            clonk_engine::JoinPlayerSource::Embedded(_)
                if info.is_script_player() && join.filename.is_empty() =>
            {
                // Script players have no .c4p file even on the host that issued
                // their fileless JoinPlayer control (C4Control.cpp:745-749).
                None
            }
            clonk_engine::JoinPlayerSource::Embedded(_)
                if local_client_id == Some(join.by_client)
                    || (offline_local && join.by_client == join.at_client) =>
            {
                let path = PathBuf::from(join.filename.to_string_lossy().into_owned());
                match PlayerFile::load_from_path(&path) {
                    Ok(file) => Some(file),
                    Err(error) => {
                        tracing::warn!(info_id = join.info_id, path = %path.display(), %error, "failed to load local player file");
                        return Ok(());
                    }
                }
            }
            clonk_engine::JoinPlayerSource::Embedded(_) => {
                match clonk_engine::resolve_remote_embedded_player_data_with_engine(
                    &self.engine,
                    &join,
                    &info,
                ) {
                    Ok(clonk_engine::RemoteEmbeddedPlayerData::PlayerFile(file)) => Some(file),
                    Ok(clonk_engine::RemoteEmbeddedPlayerData::ScriptWithoutFile) => None,
                    Err(error) => {
                        tracing::warn!(info_id = join.info_id, %error, "failed to resolve embedded join");
                        return Ok(());
                    }
                }
            }
        };
        // C4Player derives these from its loaded player core on every client;
        // fileless script players retain C4PlayerInfoCore's defaults.
        let (preferred_set, prefers_mouse) = player_file
            .as_ref()
            .map(|file| (file.pref_control, file.pref_mouse))
            .unwrap_or((0, true));
        let retained_player_info_core = player_file
            .as_ref()
            .map(PlayerFile::exact_info_core)
            .unwrap_or_default();
        let observed_startup_player_count =
            i32::try_from(self.control_player_infos.nonremoved_player_count()).unwrap_or(i32::MAX);
        let startup_player_count = self
            .engine
            .freeze_startup_player_count(observed_startup_player_count);
        let config =
            match clonk_engine::prepare_join_player_config(clonk_engine::JoinPlayerPreparation {
                join: &join,
                info: &info,
                player_file: player_file.as_ref(),
                startup_player_count,
            }) {
                Ok(config) => config,
                Err(error) => {
                    tracing::warn!(info_id = join.info_id, %error, "failed to prepare player join");
                    return Ok(());
                }
            };
        self.refresh_non_authoritative_physical_viewports();
        self.apply_direct_film_view_projection();
        let _ = self.apply_pending_viewport_presentation_requests();
        let predicted_owner = self.engine.next_player_number();
        let control_init = LocalControlInit {
            owner: predicted_owner,
            preferred_set,
            prefers_mouse,
            gamepads_enabled: self.gamepads_enabled,
            replay: false,
            disable_mouse: !self.mouse_control_allowed,
        };
        let previous_mouse_owner = self.local_controls.mouse_owner();
        let control = if locally_controlled {
            self.local_controls.initialize(control_init)
        } else {
            self.local_controls.resolve(control_init)
        };
        match self.engine.join_player_with_profile_core(
            config,
            clonk_engine::PlayerAtClient::new(join.at_client),
            at_client_name,
            Some(&info),
            control.runtime_control(),
            retained_player_info_core,
        ) {
            Ok(joined) if locally_controlled => {
                self.cache_joined_player_big_icon(join.info_id, pending_player_big_icon.as_ref());
                // InitializePlayer callbacks run before JoinPlayer creates
                // the new local viewport. Apply their physical mutations to
                // the pre-existing list before CreateViewport sorts it.
                let _ = self.apply_pending_viewport_presentation_requests();
                debug_assert_eq!(joined.number(), predicted_owner);
                let player_info_changed = self.control_player_infos.mark_joined(
                    join.info_id,
                    joined.number(),
                    i32::try_from(self.engine.frame()).unwrap_or(i32::MAX),
                );
                if player_info_changed {
                    self.publish_current_host_player_infos();
                }
                if self.local_controls.mouse_owner() != previous_mouse_owner {
                    self.reset_ingame_mouse_control();
                }
                self.mouse_control = self.local_controls.mouse_owner().is_some();
                let mut local_players = self.engine.snapshot().hud.local_players;
                if !local_players.contains(&joined.number()) {
                    local_players.push(joined.number());
                    self.engine.set_local_players(local_players);
                }
                if matches!(
                    joined,
                    clonk_engine::JoinPlayerOutcome::AwaitingTeamSelection { .. }
                ) {
                    self.open_initial_team_selection(joined.number());
                }
                let game_running = matches!(self.mode, AppMode::Running);
                let _ = self.create_physical_viewport(joined.number(), false, game_running, true);
                self.check_fullscreen_physical_viewports(game_running);
            }
            Ok(joined) => {
                self.cache_joined_player_big_icon(join.info_id, pending_player_big_icon.as_ref());
                let _ = self.apply_pending_viewport_presentation_requests();
                let player_info_changed = self.control_player_infos.mark_joined(
                    join.info_id,
                    joined.number(),
                    i32::try_from(self.engine.frame()).unwrap_or(i32::MAX),
                );
                if player_info_changed {
                    self.publish_current_host_player_infos();
                }
                // JoinPlayer calls ViewportCheck even for remote/script
                // players. In replay film mode this silently retargets an
                // existing ownerless viewport to the first live player.
                self.check_fullscreen_physical_viewports(matches!(self.mode, AppMode::Running));
            }
            Err(error) => {
                if locally_controlled {
                    self.remove_local_control_assignment(predicted_owner);
                }
                tracing::warn!(info_id = join.info_id, %error, "player join failed");
            }
        }
        Ok(())
    }

    pub(crate) fn deactivate_inactive_network_clients(&mut self) {
        if !matches!(self.network_mode.as_ref(), Some(NetworkMode::Host(_))) {
            return;
        }
        let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            return;
        };
        let current_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        let activated_client_ids = self.control_clients.activated_client_ids();
        let player_client_ids = self
            .engine
            .players()
            .map(|player| player.at_client().get())
            .collect::<Vec<_>>();
        let candidates = self.network_client_activity.deactivation_candidates(
            activated_client_ids,
            player_client_ids,
            local_client_id,
            current_frame,
        );
        let Some(network) = self.network.as_ref() else {
            return;
        };
        for client_id in candidates {
            let update = clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id,
                data: 0,
                by_client: 0,
            };
            if let Err(error) = network.submit_client_update(update) {
                tracing::error!(%client_id, %error, "failed to deactivate inactive client");
            }
        }
    }

    pub(crate) fn tick_league_update_at(&self, now: i64) {
        if self.pending_league_end.is_some()
            || !matches!(self.network_mode, Some(NetworkMode::Host(_)))
        {
            return;
        }
        let (Some(network), Some(reference)) = (
            self.network.as_ref(),
            self.advertised_game_reference.clone(),
        ) else {
            return;
        };
        if let Err(error) = network.update_league_reference(now, reference) {
            tracing::error!(%error, "failed to queue league reference update");
        }
    }

    pub(crate) fn refresh_game_over_network_result(&mut self) -> bool {
        let result = self.snapshot.round_results.network_result;
        let result_text =
            legacy_presentation_text(&self.snapshot.round_results.network_result_message);
        let stream = self.league_record_stream_status();
        let is_host = matches!(self.network_mode, Some(NetworkMode::Host(_)));
        self.game_over_dialog.as_mut().is_some_and(|dialog| {
            dialog.update_network_result(
                is_host,
                &result_text,
                result,
                stream.pending_compressed_bytes(),
                stream.is_streaming(),
            )
        })
    }

    pub(crate) fn tick_host_league_vote_timeout_at(&mut self, now: i64) -> bool {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return false;
        }
        let Some(network) = self.network.as_ref() else {
            return false;
        };
        let Some(subject) = self.league_votes.take_timed_out_subject_at(now) else {
            return false;
        };
        if let Err(error) = network.submit_vote_end(subject.vote_type, false, subject.data) {
            tracing::error!(%error, "failed to reject timed-out league vote");
        }
        true
    }

    pub(crate) fn run_pending_league_end_attempt(&mut self) -> Result<(), EngineError> {
        let Some(pending) = self.pending_league_end.as_mut() else {
            return Ok(());
        };
        pending.attempts = pending.attempts.saturating_add(1);
        let reference = pending.reference.clone();
        let record = pending.record.clone();
        let attempt = match self
            .network
            .as_ref()
            .map(|network| network.end_league(reference, record))
        {
            Some(Ok(attempt)) => attempt,
            Some(Err(error)) => LeagueEndAttempt::Retryable {
                phase: LeagueEndFailurePhase::Send,
                error: error.to_string(),
            },
            None => LeagueEndAttempt::Finished(None),
        };
        match attempt {
            LeagueEndAttempt::Finished(None) => {
                self.pending_league_end = None;
                self.finish_game_over_after_league()
            }
            LeagueEndAttempt::Finished(Some(mut packet)) if packet.success => {
                packet.result_string = league_result_message(&self.runtime_resource_text(
                    "IDS_MSG_LEAGUEEVALUATIONSUCCESSFU",
                    "League: evaluation successful.",
                ));
                self.pending_league_end = None;
                self.apply_and_broadcast_league_result(packet);
                self.finish_game_over_after_league()
            }
            LeagueEndAttempt::Rejected(mut packet)
            | LeagueEndAttempt::Finished(Some(mut packet)) => {
                let message = self.league_end_error_message(
                    LeagueEndFailurePhase::Send,
                    &legacy_presentation_text(packet.result_string.as_bytes()),
                );
                packet.result_string = league_result_message(&message);
                if let Some(pending) = self.pending_league_end.as_mut() {
                    pending.last_failure = Some(message.clone());
                    pending.terminal_packet = Some(packet);
                }
                tracing::error!(%message, "league server rejected the round result");
                self.push_league_end_error_dialog(
                    message,
                    clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                    MessageDialogContinuation::LeagueEndRejected,
                )
            }
            LeagueEndAttempt::Retryable { phase, error } => {
                let message = self.league_end_error_message(phase, &error);
                if let Some(pending) = self.pending_league_end.as_mut() {
                    pending.last_failure = Some(message.clone());
                }
                tracing::error!(%message, "league round-result request failed");
                self.push_league_end_error_dialog(
                    message,
                    clonk_frontend::message_dialog::MessageDialogButtons::RETRY_CANCEL,
                    MessageDialogContinuation::LeagueEndRetry,
                )
            }
        }
    }

    fn league_end_error_message(&self, phase: LeagueEndFailurePhase, error: &str) -> String {
        let error = if error.is_empty() {
            self.runtime_resource_text("IDS_NET_ERR_LEAGUE_EMPTYREPLY", "Empty reply")
        } else {
            error.to_string()
        };
        let (key, fallback) = match phase {
            LeagueEndFailurePhase::Start => {
                ("IDS_NET_ERR_LEAGUE_FINISHGAME", "Could not finish game: %s")
            }
            LeagueEndFailurePhase::Send => (
                "IDS_NET_ERR_LEAGUE_SENDRESULT",
                "Could not send game result: %s",
            ),
        };
        format_resource_string(self.runtime_resource_text(key, fallback), &[&error])
    }

    pub(crate) fn finalize_pending_league_end_failure(&mut self) -> Result<(), EngineError> {
        let Some(message) = self
            .pending_league_end
            .as_ref()
            .and_then(|pending| pending.last_failure.clone())
        else {
            return Ok(());
        };
        let fallback = clonk_network::LeagueRoundResultsPacket {
            success: false,
            result_string: league_result_message(&message),
            players: Vec::new(),
        };
        let packet = self
            .network
            .as_ref()
            .and_then(
                |network| match network.finalize_league_end_failure(fallback.clone()) {
                    Ok(packet) => packet,
                    Err(error) => {
                        tracing::error!(%error, "failed to finalize league round-result failure");
                        None
                    }
                },
            )
            .unwrap_or(fallback);
        self.pending_league_end = None;
        self.apply_and_broadcast_league_result(packet);
        self.finish_game_over_after_league()
    }

    pub(crate) fn finish_pending_league_end_terminal(&mut self) -> Result<(), EngineError> {
        let packet = self
            .pending_league_end
            .take()
            .and_then(|pending| pending.terminal_packet);
        if let Some(fallback) = packet {
            let packet = self
                .network
                .as_ref()
                .and_then(
                    |network| match network.finalize_league_end_failure(fallback.clone()) {
                        Ok(packet) => packet,
                        Err(error) => {
                            tracing::error!(%error, "failed to finalize rejected league result");
                            None
                        }
                    },
                )
                .unwrap_or(fallback);
            self.apply_and_broadcast_league_result(packet);
        }
        self.finish_game_over_after_league()
    }

    pub(crate) fn finalize_pending_league_end_for_teardown(&mut self) {
        let Some(pending) = self.pending_league_end.take() else {
            return;
        };
        let fallback = pending.terminal_packet.unwrap_or_else(|| {
            let message = pending
                .last_failure
                .unwrap_or_else(|| self.league_end_error_message(LeagueEndFailurePhase::Send, ""));
            clonk_network::LeagueRoundResultsPacket {
                success: false,
                result_string: league_result_message(&message),
                players: Vec::new(),
            }
        });
        let packet = self
            .network
            .as_ref()
            .and_then(
                |network| match network.finalize_league_end_failure(fallback.clone()) {
                    Ok(packet) => packet,
                    Err(error) => {
                        tracing::error!(%error, "failed to finalize league result during teardown");
                        None
                    }
                },
            )
            .unwrap_or(fallback);
        self.apply_and_broadcast_league_result(packet);
    }

    fn apply_and_broadcast_league_result(
        &mut self,
        packet: clonk_network::LeagueRoundResultsPacket,
    ) {
        self.apply_league_round_results_packet(&packet);
        if let Some(network) = self.network.as_ref() {
            if let Err(error) = network.broadcast_league_round_results(packet) {
                tracing::error!(%error, "host league-result broadcast failed");
            }
        }
    }

    pub(crate) fn finish_game_over_after_league(&mut self) -> Result<(), EngineError> {
        // C4GameOverDlg::OnShown hides the scoreboard and closes each
        // player's fullscreen C4MainMenu before evaluation becomes
        // interactive. The synchronized object/cursor menu survives
        // C4Player::CloseMenu; save-browser UI is a descendant of the app's
        // fullscreen menu. The scoreboard refcount is untouched.
        self.close_scoreboard_dialog();
        let fullscreen_menu_open = self.ingame_menu.is_some() || self.save_browser.is_some();
        self.close_ingame_menu();
        self.save_browser = None;
        self.save_browser_return_to_menu = false;
        if fullscreen_menu_open {
            // C4MainMenu::OnClosed synchronizes exactly one
            // ClearPressedComs when game-over closes the player menu.
            self.clear_local_controls()?;
        }
        self.hydrate_runtime_player_big_icons_for_evaluation();
        let fulfilled_goal_tooltip =
            self.runtime_resource_text("IDS_DESC_GOALFULFILLED", "Goal %s fulfilled: %s");
        let unfulfilled_goal_tooltip =
            self.runtime_resource_text("IDS_DESC_GOALNOTFULFILLED", "Goal %s not fulfilled: %s");
        let scenario_title = self
            .active_scenario
            .as_ref()
            .map(|scenario| scenario.title.clone())
            .unwrap_or_else(|| "Scenario".to_string());
        let host_or_cinematic_film = self.engine.is_control_host() || self.engine.cinematic_film();
        let next_mission = self.engine.next_mission();
        let mut dialog = build_game_over_dialog(
            &self.snapshot,
            self.engine.teams(),
            self.engine.auto_generate_teams(),
            self.local_owner,
            self.graphics.surface().width(),
            host_or_cinematic_film,
            scenario_title.clone(),
            next_mission,
            |definition_id, fulfilled| {
                // `C4GoalDisplay::GoalPicture` looks the goal up as a live
                // object first and hands it to `C4Def::Draw(.., pGoalObj)`, so
                // the picture carries that object's current graphics rather
                // than the bare definition picture
                // (src/C4GameOverDlg.cpp:52-59; src/C4GameObjects.cpp:264-268
                // -> C4ObjectList::Find, which takes the first live entry with
                // that id).
                let goal_object = self
                    .snapshot
                    .objects
                    .iter()
                    .find(|object| object.definition_id.as_str() == definition_id);
                let picture = goal_object
                    .and_then(|object| self.engine.object_picture_image(object))
                    .or_else(|| self.engine.definition_picture_image(definition_id))
                    .map(definition_menu_picture);
                let name = self
                    .engine
                    .definition_name(definition_id)
                    .map(c4_presentation_text)
                    .unwrap_or_default();
                let description = self
                    .engine
                    .definition_description(definition_id)
                    .map(c4_presentation_text)
                    .unwrap_or_default();
                let template = if fulfilled {
                    fulfilled_goal_tooltip.clone()
                } else {
                    unfulfilled_goal_tooltip.clone()
                };
                (
                    picture,
                    format_resource_string_with_opaque_arguments(template, &[&name, &description]),
                )
            },
            |player_info_id| self.runtime_player_big_icons.get(&player_info_id).cloned(),
            |player_info_id| {
                self.control_player_infos
                    .get(player_info_id)
                    .map(|info| info.league_score)
            },
            |player_info_id| {
                // A row that *is* a free savegame player joins itself;
                // otherwise the association names a RestorePlayerInfos entry
                // (src/C4PlayerInfoListBox.cpp:701-716).
                let info = self.control_player_infos.get(player_info_id)?;
                let joined = if info.savegame_player == 0 {
                    return None;
                } else {
                    self.classic_lobby_restore_player(info.savegame_player)?
                };
                Some(joined.color)
            },
            |icon_spec, color| {
                let resources = self.script_text_spec_resources();
                resolve_script_font_image(&self.engine, icon_spec, color, resources)
            },
            |player_info_id| {
                self.control_player_infos
                    .get(player_info_id)
                    .map(|info| info.league_rank_symbol)
            },
            self.network_is_league,
        );
        let network_result = self.snapshot.round_results.network_result;
        let network_result_text =
            legacy_presentation_text(&self.snapshot.round_results.network_result_message);
        let stream = self.league_record_stream_status();
        dialog.initialize_network_result(
            self.network_is_league || network_result.is_some(),
            matches!(self.network_mode, Some(NetworkMode::Host(_))),
            &network_result_text,
            network_result,
            stream.pending_compressed_bytes(),
            stream.is_streaming(),
        );
        for (action, label_key, label, description_key, description) in [
            (
                GameOverAction::End,
                "IDS_BTN_ENDROUND",
                "&End game",
                "IDS_DESC_ENDTHEROUND",
                "End the round.",
            ),
            (
                GameOverAction::Continue,
                "IDS_BTN_CONTINUEGAME",
                "&Continue playing",
                "IDS_DESC_CONTINUETHEROUNDWITHNOFUR",
                "Continue playing this round (with no further evaluation).",
            ),
            (
                GameOverAction::Restart,
                "IDS_BTN_RESTART",
                "&Restart",
                "IDS_DESC_RESTART",
                "Play this scenario again.",
            ),
        ] {
            dialog.set_button_content(
                action,
                self.runtime_resource_text(label_key, label),
                self.runtime_resource_text(description_key, description),
            );
        }
        dialog.configure_classic_fonts(self.assets.clonk_fonts.as_deref());
        let status_message = if dialog.subtitle().is_empty() {
            format!("{scenario_title} complete")
        } else {
            format!("{scenario_title}: {}", dialog.subtitle())
        };
        self.status_text = status_message;
        self.game_over_dialog = Some(dialog);
        self.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::GameOver);
        // C4GameOverDlg::OnShown delegates to C4Game::Pause after closing the
        // scoreboard and player fullscreen menus. That routes through the
        // synchronized GS_Pause barrier for a network host, is governed by
        // the host for a client, and directly acquires the offline halt.
        self.set_runtime_pause(true);
        Ok(())
    }

    pub(crate) fn maybe_emit_sync_check(&mut self) {
        let Some(local_client_id) = self
            .network
            .as_ref()
            .map(|network| network.local_client_id())
        else {
            return;
        };
        if !matches!(self.mode, AppMode::Running) {
            return;
        }
        let Ok(frame_i32) = i32::try_from(self.snapshot.frame) else {
            return;
        };
        if frame_i32 < 0 {
            return;
        }
        if frame_i32 % SYNC_CHECK_RATE as i32 != 0 {
            self.sync_checks
                .prune_before(frame_i32.saturating_sub(SYNC_CHECK_HISTORY));
            return;
        }
        let client_id = i32::try_from(local_client_id).unwrap_or(0);
        let check = self.engine.sync_check(client_id);
        if let Some((local, remote)) = self.sync_checks.record_local(check.clone()) {
            self.evaluate_sync_checks(local, remote);
        }
        // C4GameControl::DoSyncCheck stores a client's local check for later
        // comparison; only the host queues a C4ControlSyncCheck into network
        // control (src/C4GameControl.cpp:441-468).
        if matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            let tick = self.local_control_submission_tick();
            if let Some(network) = self.network.as_ref() {
                network.submit_sync_check(tick, check);
            }
        }
        self.sync_checks
            .prune_before(frame_i32.saturating_sub(SYNC_CHECK_HISTORY));
    }

    fn apply_live_league_start_response(
        &mut self,
        response: &clonk_network::LeagueStartResponse,
    ) -> Result<()> {
        let mut parameters = self
            .host_join_snapshot
            .as_ref()
            .map(|snapshot| snapshot.parameters.clone())
            .or_else(|| {
                self.advertised_game_reference
                    .as_ref()
                    .map(|reference| reference.parameters().clone())
            })
            .ok_or_else(|| anyhow!("live host game parameters are unavailable"))?;
        let max_players =
            network::apply_league_start_response_to_parameters(&mut parameters, response)
                .map_err(|message| anyhow!(message))?;

        // Validate every local representation before mutating any of them.
        // The server-side registration is already live at this point, so a
        // partially applied response must never be painted as signup-off.
        let updated_reference = self
            .advertised_game_reference
            .as_ref()
            .map(|reference| {
                reference
                    .replacing_parameters(parameters.clone())
                    .map_err(|error| anyhow!("cannot rebuild live league reference: {error}"))
            })
            .transpose()?;
        // PreparedHostBootstrap clones share the single retained Scenario.
        // Validate the independently cloned reference first so the only
        // remaining fallible operation may update that shared scenario as the
        // final step of this transaction.
        let updated_prepared = self
            .network_mode
            .as_ref()
            .and_then(|mode| match mode {
                NetworkMode::Host(HostSettings { prepared, .. }) => prepared.as_ref(),
                NetworkMode::Client(_) => None,
            })
            .map(|prepared| {
                let mut updated = prepared.clone();
                updated
                    .apply_league_start_response(response)
                    .map_err(|error| anyhow!("cannot apply live league Start settings: {error}"))?;
                Ok::<_, anyhow::Error>(updated)
            })
            .transpose()?;

        if let Some(updated) = updated_prepared {
            if let Some(prepared) = self.network_mode.as_mut().and_then(|mode| match mode {
                NetworkMode::Host(HostSettings { prepared, .. }) => prepared.as_mut(),
                NetworkMode::Client(_) => None,
            }) {
                *prepared = updated;
            }
        }

        if let Some(snapshot) = self.host_join_snapshot.as_mut() {
            snapshot.parameters = parameters.clone();
        }
        if let Some(max_players) = max_players {
            self.network_max_players = max_players;
            self.engine.set_max_players(response.max_players);
            if let Some(staged) = self.staged_network_host_scenario.as_mut() {
                staged.lobby.max_players = response.max_players;
            }
        }
        self.network_league_name = parameters.league.as_bytes().to_vec();
        self.network_stream_address = if response.league.is_empty() {
            LegacyCString::default()
        } else {
            response.stream_to.clone()
        };
        self.network_is_league = synchronized_parameters_are_league(&parameters);
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        self.scenario_game_options
            .set_lobby_league(self.network_is_league);
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.set_league_mode(self.network_is_league);
        }
        self.sync_classic_lobby_roster();

        if let Some(snapshot) = self.host_join_snapshot.clone() {
            if let Some(network) = self.network.as_ref() {
                if let Err(error) = network.publish_join_snapshot(snapshot) {
                    tracing::error!(
                        %error,
                        "failed to publish live league Start parameters"
                    );
                }
            }
        }
        if let Some(updated) = updated_reference {
            if let Some(advertiser) = self.network_game_advertiser.as_ref() {
                if let Err(error) = advertiser.update_exact(&updated) {
                    tracing::error!(
                        %error,
                        "failed to publish live league Start reference"
                    );
                }
            }
            self.advertised_game_reference = Some(updated);
        }
        Ok(())
    }

    fn clear_live_league_registration(&mut self) -> Result<()> {
        let mut parameters = self
            .host_join_snapshot
            .as_ref()
            .map(|snapshot| snapshot.parameters.clone())
            .or_else(|| {
                self.advertised_game_reference
                    .as_ref()
                    .map(|reference| reference.parameters().clone())
            })
            .ok_or_else(|| anyhow!("live host game parameters are unavailable"))?;
        parameters.league = LegacyCString::default();
        parameters.league_address = LegacyCString::default();

        let updated_prepared = self
            .network_mode
            .as_ref()
            .and_then(|mode| match mode {
                NetworkMode::Host(HostSettings { prepared, .. }) => prepared.as_ref(),
                NetworkMode::Client(_) => None,
            })
            .map(|prepared| {
                let mut updated = prepared.clone();
                updated
                    .clear_live_league_registration()
                    .map_err(|error| anyhow!("cannot clear live league settings: {error}"))?;
                Ok::<_, anyhow::Error>(updated)
            })
            .transpose()?;
        let updated_reference = self
            .advertised_game_reference
            .as_ref()
            .map(|reference| {
                reference
                    .replacing_parameters(parameters.clone())
                    .map_err(|error| anyhow!("cannot rebuild non-league reference: {error}"))
            })
            .transpose()?;

        if let Some(updated) = updated_prepared {
            if let Some(prepared) = self.network_mode.as_mut().and_then(|mode| match mode {
                NetworkMode::Host(HostSettings { prepared, .. }) => prepared.as_mut(),
                NetworkMode::Client(_) => None,
            }) {
                *prepared = updated;
            }
        }
        if let Some(snapshot) = self.host_join_snapshot.as_mut() {
            snapshot.parameters = parameters;
        }
        self.network_league_name.clear();
        self.network_is_league = false;
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        self.scenario_game_options.set_lobby_league(false);
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.set_league_mode(false);
        }
        self.sync_classic_lobby_roster();

        if let Some(snapshot) = self.host_join_snapshot.clone() {
            if let Some(network) = self.network.as_ref() {
                if let Err(error) = network.publish_join_snapshot(snapshot) {
                    tracing::error!(%error, "failed to publish cleared live league parameters");
                }
            }
        }
        if let Some(updated) = updated_reference {
            if let Some(advertiser) = self.network_game_advertiser.as_ref() {
                if let Err(error) = advertiser.update_exact(&updated) {
                    tracing::error!(%error, "failed to publish non-league reference");
                }
            }
            self.advertised_game_reference = Some(updated);
        }
        Ok(())
    }

    fn begin_live_masterserver_signup_rollback(
        &self,
    ) -> Result<network::PendingMasterserverSignup> {
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| anyhow!("live host network manager is unavailable"))?;
        let reference = self
            .advertised_game_reference
            .clone()
            .ok_or_else(|| anyhow!("live host game reference is unavailable"))?;
        network.begin_masterserver_signup(
            false,
            load_prepared_league_host_config(self.app_paths.as_ref(), false),
            reference,
        )
    }

    pub(crate) fn poll_live_masterserver_signup(&mut self) -> Result<(), EngineError> {
        let result = match (
            self.network.as_ref(),
            self.pending_lobby_internet_signup.as_mut(),
        ) {
            (Some(network), Some(pending)) => network.poll_masterserver_signup(pending),
            _ => None,
        };
        let Some(result) = result else {
            return Ok(());
        };
        let pending = self
            .pending_lobby_internet_signup
            .take()
            .expect("polled live masterserver request is retained");
        if let Err(error) = self.close_live_masterserver_signup_dialog() {
            tracing::error!(%error, "failed to close live masterserver wait dialog");
        }

        let enabled = pending.enabled();
        let previous_enabled = pending.previous_enabled();
        let (effective, failure) = match result {
            Ok(response) => {
                if enabled {
                    match response
                        .as_ref()
                        .map(|response| self.apply_live_league_start_response(response))
                        .transpose()
                    {
                        Ok(_) => (true, None),
                        Err(error) => match self.begin_live_masterserver_signup_rollback() {
                            Ok(rollback) => {
                                // Start has committed on the worker. Keep its
                                // live state visible and block launch until
                                // the compensating End is confirmed.
                                self.pending_lobby_internet_signup = Some(rollback);
                                (true, Some(error))
                            }
                            Err(rollback_error) => {
                                // Dropping the manager makes the worker send
                                // End with its retained Start-updated
                                // reference. Tear the staged host down so a
                                // locally rejected seed can never launch.
                                let message = format!(
                                    "{error}; could not begin compensating Internet signup cleanup: {rollback_error}"
                                );
                                tracing::error!(error = %message, "tearing down rejected live signup");
                                return self.finish_startup_network_failure(
                                    StartupNetworkPurpose::StagedHost,
                                    format!("Unable to change Internet game signup: {message}"),
                                );
                            }
                        },
                    }
                } else {
                    match self.clear_live_league_registration() {
                        Ok(()) => (false, None),
                        Err(error) => (false, Some(error)),
                    }
                }
            }
            Err(error) if !enabled && previous_enabled => {
                return self.finish_startup_network_failure(
                    StartupNetworkPurpose::StagedHost,
                    format!("Unable to confirm cleanup of the live Internet registration: {error}"),
                );
            }
            Err(error) => (if enabled { previous_enabled } else { false }, Some(error)),
        };
        self.scenario_game_options
            .apply_lobby_internet_result(effective);
        self.persist_game_option_value(
            "Network",
            "MasterServerSignUp",
            i32::from(effective).to_string(),
        );
        if let Some(error) = failure {
            tracing::error!(%error, enabled, "failed to change live masterserver signup");
            self.status_text = format!("Unable to change Internet game signup: {error}");
        }
        Ok(())
    }

    pub(crate) fn abort_live_masterserver_signup(&mut self) {
        let Some(mut pending) = self.pending_lobby_internet_signup.take() else {
            return;
        };
        if !pending.cancel() {
            self.pending_lobby_internet_signup = Some(pending);
            return;
        }
        let previous_enabled = pending.previous_enabled();
        self.scenario_game_options
            .apply_lobby_internet_result(previous_enabled);
        self.persist_game_option_value(
            "Network",
            "MasterServerSignUp",
            i32::from(previous_enabled).to_string(),
        );
        self.status_text = "Internet game signup cancelled.".to_string();
    }

    pub(crate) fn abandon_live_masterserver_signup(&mut self) {
        if let Some(mut pending) = self.pending_lobby_internet_signup.take() {
            if !pending.finish_committed_cleanup_on_worker_shutdown() {
                let _ = pending.cancel();
            }
        }
    }

    pub(crate) fn complete_league_vote_response(
        &mut self,
        subject: LeagueVoteSubject,
        approve: bool,
    ) {
        if !self.league_votes.subject_active(subject) {
            return;
        }
        self.submit_own_league_vote(subject, approve);
    }

    pub(crate) fn submit_own_league_vote(
        &mut self,
        subject: LeagueVoteSubject,
        approve: bool,
    ) -> bool {
        let now = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
        self.submit_own_league_vote_at(subject, approve, now)
    }

    pub(crate) fn submit_own_league_vote_at(
        &mut self,
        subject: LeagueVoteSubject,
        approve: bool,
        now: i64,
    ) -> bool {
        let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            return false;
        };
        if self
            .league_votes
            .first_ballot(local_client_id, subject)
            .is_some()
        {
            return false;
        }
        if !self.league_votes.try_submit_own_vote_at(subject, now) {
            let message = self.runtime_resource_string("IDS_TEXT_YOUCANONLYSTARTONEVOTINGE");
            self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
            let opens_surrender = subject.vote_type == clonk_engine::VOTE_TYPE_CANCEL
                || subject.vote_type == clonk_engine::VOTE_TYPE_KICK
                    && subject.data == local_client_id;
            if opens_surrender {
                if let Err(error) = self.open_league_surrender_dialog() {
                    tracing::error!(%error, "failed to open league surrender dialog");
                }
            }
            return false;
        }
        // C4Network2::Vote broadcasts the Pause status before it queues its
        // own direct vote, so peers cannot observe the ballot while still
        // running past the host's chosen control boundary.
        self.pause_host_for_league_vote();
        let Some(network) = self.network.as_ref() else {
            return false;
        };
        if let Err(error) = network.submit_vote(subject.vote_type, approve, subject.data) {
            tracing::error!(%error, "failed to submit league vote");
            return false;
        }
        true
    }

    pub(crate) fn league_signup_tooltip(
        &self,
        width: i32,
        height: i32,
    ) -> Option<(GuiPoint, String)> {
        if !self.message_dialogs.is_empty() || self.context_menu.is_some() {
            return None;
        }
        let pointer = self.startup_tooltip.eligible_pointer()?;
        let dialog = self.league_signup_dialog.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let layout = dialog.controller.layout(width, height, &fonts.text);
        let text = dialog.controller.tooltip_at(pointer, &layout)?.to_owned();
        (!text.is_empty()).then_some((pointer, text))
    }

    pub(crate) fn network_game_tooltip_target_at(&self, point: GuiPoint) -> Option<StartupTooltip> {
        let dialog = self.startup_network_dialog.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.graphics.surface();
        let layout = clonk_frontend::startup_netdlg::net_dlg_layout(
            surface.width() as i32,
            surface.height() as i32,
            &clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(fonts),
        );
        let title = self.startup_tooltip_resource_string("IDS_DLG_NETSTART");
        let (display_title, _) = clonk_frontend::expand_hotkey_markup(&title);
        clonk_frontend::centered_label_tooltip_at(
            point,
            layout.title_anchor,
            fonts.title.measure(&display_title, true),
            StartupTooltip::text(title),
        )
        .or_else(|| dialog.tooltip_at(point))
    }

    /// `C4Record::AddFile` strips a temporary `.c4p` copy before streaming;
    /// the local replay group retains the complete original player resource.
    pub(crate) fn pack_stripped_stream_player(
        &self,
        source: &Group,
        target: &[u8],
    ) -> std::result::Result<Vec<u8>, String> {
        // C4Player::Strip loads crew with fLoadPortraits=false. An unnamed
        // embedded image must not turn into a persisted `PortraitFile=custom`
        // while the temporary stream copy is rebuilt.
        let player =
            PlayerFile::load_with_portraits(source, false).map_err(|error| error.to_string())?;
        if count_direct_stream_player_crew_files(source)? == 0 {
            return Err("player group contains no loadable direct crew info".to_string());
        }
        let (_, _, player_rank_name_default) = self.developer_console_player_save_options();
        let stripped = clonk_engine::serialize_aggressively_stripped_c4_player(
            &self.engine,
            &player,
            target,
            self.process_group_maker.as_bytes(),
            &player_rank_name_default,
        )
        .map_err(|error| error.to_string())?;
        stripped.pack().map_err(|error| error.to_string())
    }

    /// Network branch of `C4ScenarioListLoader::Scenario::CanOpen`.
    /// Replays fail before player counting. Savegames use the minimum-player
    /// lift only for this upper-bound check; the later restore-row floor is a
    /// separate `C4Game::OpenScenario` transition.
    pub(crate) fn network_scenario_open_decision(
        &self,
        scenario: &FrontendScenario,
    ) -> std::result::Result<NetworkScenarioOpenDecision, ClassicParityBoundary> {
        let Some(path) = scenario.path.as_deref() else {
            return Ok(NetworkScenarioOpenDecision::Proceed);
        };
        let Some(paths) = self.app_paths.as_ref() else {
            return Ok(NetworkScenarioOpenDecision::Proceed);
        };
        let inspect_error = |error: &dyn fmt::Display| {
            report_classic_parity_boundary(ClassicParityBoundary::ScenarioStartInspection {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })
        };
        let group = Group::open(path).map_err(|error| inspect_error(&error))?;
        let head = load_classic_scenario_loader_head(&group, paths)
            .map_err(|error| inspect_error(&error))?;
        let cannot_start =
            || self.runtime_resource_text("IDS_MSG_CANNOTSTARTSCENARIO", "Cannot start scenario.");
        if !head.mission_access().is_empty() {
            let granted = load_configured_mission_access(paths)
                .map_err(|error| inspect_error(&error))?
                .split(';')
                .map(str::trim)
                .any(|access| access.eq_ignore_ascii_case(head.mission_access()));
            if !granted {
                return Ok(NetworkScenarioOpenDecision::Error {
                    message: self.runtime_resource_text(
                        "IDS_PRC_NOMISSIONACCESS",
                        "Access to this mission not yet granted.",
                    ),
                    caption: cannot_start(),
                });
            }
        }
        if head.is_replay() {
            return Ok(NetworkScenarioOpenDecision::Error {
                message: self.runtime_resource_text(
                    "IDS_PRC_NONETREPLAY",
                    "Cannot play back records while in network mode.",
                ),
                caption: cannot_start(),
            });
        }

        let player_count =
            startup_participant_module_count(paths).map_err(|error| inspect_error(&error))?;
        let minimum = head.min_players();
        let warning = (player_count < minimum).then(|| {
            let template = self.runtime_resource_text(
                "IDS_MSG_TOOFEWPLAYERSNET",
                "This scenario is designed for a minimum of %i players. On start, you will have to wait for additional players to join from the network.",
            );
            let caption = self.runtime_resource_text("IDS_DLG_STARTGAME", "&Start Game");
            NetworkScenarioOpenDecision::Warning {
                message: template.replacen("%i", &minimum.to_string(), 1),
                caption: caption.replace('&', ""),
            }
        });
        let maximum = if head.is_save_game() {
            head.max_players().max(minimum)
        } else {
            head.max_players()
        };
        if player_count > maximum {
            let template = self.runtime_resource_text(
                "IDS_MSG_TOOMANYPLAYERS",
                "This scenario is designed for a maximum of %i players.",
            );
            return Ok(NetworkScenarioOpenDecision::Error {
                // Native intentionally formats the raw scenario maximum even
                // when a savegame used the minimum-player lift above.
                message: template.replacen("%i", &head.max_players().to_string(), 1),
                caption: cannot_start(),
            });
        }
        Ok(warning.unwrap_or(NetworkScenarioOpenDecision::Proceed))
    }

    pub(crate) fn stage_network_host_scenario(
        &mut self,
        frontend: FrontendScenario,
        definition_load: ScenarioDefinitionLoad,
    ) {
        self.initial_definition_seed = None;
        self.startup_restart_diagnostics.begin_game_init();
        self.staged_network_host_scenario = None;
        self.clear_lobby_preload();
        let title = frontend.title.clone();
        let path = frontend.path.clone();
        let staged = match self.prepare_network_host_scenario(frontend, definition_load) {
            Ok(staged) => staged,
            Err(error) => {
                tracing::error!(
                    scenario = %title,
                    path = ?path,
                    %error,
                    "network host scenario validation failed before socket creation"
                );
                self.status_text = format!("Cannot host {title}: {error}");
                return;
            }
        };
        let selected = staged.frontend.clone();
        self.staged_network_host_scenario = Some(staged);
        let (_, configured_port) = load_network_startup_settings(self.app_paths.as_ref());
        let port = self
            .classic_command_line
            .tcp_port
            .unwrap_or(configured_port);
        self.activate_prepared_network_host(selected, SocketAddr::from(([0, 0, 0, 0], port)));
        if self.startup_network_connection.is_none() {
            self.staged_network_host_scenario = None;
        }
    }

    pub(crate) fn prepare_network_host_scenario(
        &self,
        mut frontend: FrontendScenario,
        definition_load: ScenarioDefinitionLoad,
    ) -> Result<StagedNetworkHostScenario> {
        let paths = self.app_paths.as_ref().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: "application paths are unavailable".to_string(),
            })
        })?;
        if let Some(detail) = self.loader_render_error.as_deref() {
            return Err(classic_game_lobby_error(
                ClassicGameLobbyBoundary::Resources {
                    detail: format!("loader render configuration is invalid: {detail}"),
                },
            ));
        }
        self.loader_render_config.ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: "loader render configuration is unavailable".to_string(),
            })
        })?;
        let path = frontend.path.as_deref().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "no transferable scenario group".to_string(),
            })
        })?;
        let resolver = InstallDefinitionResolver::new(self.app_paths.clone().map(Arc::new));
        let languages = startup_language_sequence(self.app_paths.as_ref());
        let scenario =
            load_scenario_with_definition_load(path, &resolver, &languages, &definition_load)
                .map_err(|error| {
                    classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                        detail: format!("scenario validation failed: {error}"),
                    })
                })?;
        let metadata = scenario.lobby_metadata().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "legacy Scenario.txt lobby metadata is unavailable".to_string(),
            })
        })?;
        let effective_definition_modules = metadata
            .definitions()
            .effective_modules()
            .unwrap_or_default()
            .to_vec();
        let effective_definition_spellings = metadata
            .definitions()
            .effective_modules()
            .map(|_| metadata.definitions().requested_module_spellings().to_vec())
            .unwrap_or_default();
        let native_config = load_native_config_bytes(Some(paths));
        let (definition_executable_path, definition_path) =
            game_save_definition_paths(Some(paths), &native_config);
        let definition_executable_root =
            path_from_group_name_bytes(&clonk_script::c4_string_bytes(&definition_executable_path));
        let definition_resources =
            host_game_resource_sources::freeze_host_definition_resource_sources(
                scenario.definition_resource_paths(),
                path,
                &effective_definition_spellings,
                metadata.definitions().definition_root_applied(),
                &definition_executable_root,
                &definition_path,
            )
            .map_err(|error| {
                classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                    detail: format!("definition publication freeze failed: {error}"),
                })
            })?;
        if metadata.head().is_replay() {
            return Err(classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "network replay selection must be rejected by CanOpen".to_string(),
            }));
        }
        let embedded = metadata.embedded_game_parameter_values();
        let parameters = embedded
            .as_ref()
            .unwrap_or_else(|| metadata.game_parameter_defaults());
        let options = self.scenario_game_options.values().clone();
        let (local_name, nick, countdown_seconds) =
            load_classic_lobby_identity(paths).map_err(|error| {
                classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                    detail: error.to_string(),
                })
            })?;
        self.assets.game_lobby_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        self.assets.game_option_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        let loader_setup =
            build_scenario_loader(&frontend, &definition_load, paths, self.assets.as_ref())
                .map_err(|error| {
                    classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                        detail: format!("scenario loader backdrop is unavailable: {error}"),
                    })
                })?;
        let loader_initial_tooltip_font = loader_setup
            .initial_tooltip_font
            .clone()
            .context("scenario loader did not provide its pre-definition tooltip font")?;
        let default_player_icon = loader_setup
            .refreshed_player_icon
            .clone()
            .context("scenario loader did not resolve the post-definition Player graphic")?;
        let default_crew_icon = loader_setup
            .refreshed_crew_icon
            .clone()
            .context("scenario loader did not resolve the post-definition Crew graphic")?;
        retain_selected_scenario_title(&mut frontend, loader_setup.scenario_title.as_deref());
        let (fair_crew, fair_crew_strength) =
            resolve_scenario_fair_crew_parameters(metadata, &options);
        let lobby = ClassicHostLobbyProjection {
            local_name,
            nick,
            countdown_seconds,
            max_players: parameters.max_players(),
            has_teams: metadata.teams().is_active(),
            fair_crew,
            fair_crew_forced: parameters.fair_crew_forced(),
            fair_crew_strength,
        };
        Ok(StagedNetworkHostScenario {
            frontend,
            definition_load,
            effective_definition_modules,
            definition_resources,
            definition_executable_path,
            definition_path,
            scenario,
            loader_screen: Some(loader_setup.screen),
            loader_initial_tooltip_font,
            loader_initial_native_font_source: loader_setup.initial_native_font_source,
            loader_refreshed_resources: loader_setup.refreshed_resources,
            loader_refreshed_tooltip_font: loader_setup.refreshed_tooltip_font,
            loader_refreshed_native_font_source: loader_setup.refreshed_native_font_source,
            pending_global_gui_failures: loader_setup.refreshed_global_gui_failures,
            pending_gui_sheet_overrides: loader_setup.refreshed_gui_sheet_overrides,
            default_player_icon,
            default_crew_icon,
            options,
            lobby,
        })
    }

    fn recreate_runtime_join_players(&mut self) -> Result<(), EngineError> {
        let staged_sources = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| prepared.runtime_join_players.clone())
            .unwrap_or_default();
        if staged_sources.is_empty() {
            return Ok(());
        }

        // RecreatePlayers skips a whole SavePlayerInfos packet when its
        // original client is no longer present. Do this before opening any
        // embedded player group, so a departed client's stale filename is
        // never treated as a load failure.
        let sources = staged_sources
            .into_iter()
            .filter(|source| self.control_clients.contains(source.client_id))
            .collect::<Vec<_>>();
        if sources.is_empty() {
            self.deferred_network_savegame_recreation.clear();
            return Ok(());
        }
        let scenario_path = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.scenario.path.clone())
            .ok_or_else(|| {
                EngineError::from(clonk_engine::RuntimeJoinPlayerRestoreError::MissingScenarioPath)
            })?;
        let restored = self
            .engine
            .restore_runtime_join_players_from_path(&scenario_path, &sources)
            .map_err(EngineError::from)?;

        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(0);
        let mut rebound_local_controls = LocalControlRegistry::default();
        let mut local_players = Vec::new();

        for (source, binding) in sources.iter().zip(restored) {
            let (
                saved_mouse_control,
                preferred_control_set,
                prefers_mouse,
                saved_pref_control_style,
                saved_pref_auto_context_menu,
                saved_player_name,
            ) = {
                let player = self
                    .engine
                    .player(binding.number)
                    .ok_or(EngineError::UnknownPlayer(binding.number))?;
                let (preferred_control_set, prefers_mouse) = player.control_preferences();
                let (pref_control_style, pref_auto_context_menu) =
                    player.control_style_preferences();
                (
                    player.mouse_control(),
                    preferred_control_set,
                    prefers_mouse,
                    pref_control_style,
                    pref_auto_context_menu,
                    player.name().to_string(),
                )
            };
            let current_info = self
                .control_player_infos
                .get(binding.player_info_id)
                .cloned()
                .unwrap_or_else(|| source.info.clone());
            let script_player = current_info.is_script_player();
            let no_elimination_check = current_info.no_elimination_check();
            let player_name = if control_player_effective_name(&current_info).is_empty() {
                saved_player_name
            } else {
                clonk_script::c4_string_from_bytes(control_player_effective_name(&current_info))
            };
            let client_name = self
                .control_clients
                .state(source.client_id)
                .expect("missing SavePlayerInfos clients were filtered before recreation")
                .name
                .as_bytes();
            let at_client_name = clonk_script::c4_string_from_bytes(client_name);
            let locally_controlled = !script_player && source.client_id == local_client_id;
            let control_init = LocalControlInit {
                owner: binding.number,
                preferred_set: preferred_control_set,
                prefers_mouse,
                gamepads_enabled: self.gamepads_enabled,
                replay: false,
                disable_mouse: !self.mouse_control_allowed,
            };
            let control = if locally_controlled {
                let control = rebound_local_controls
                    .initialize_after_restore(control_init, saved_mouse_control != 0);
                local_players.push(binding.number);
                control
            } else {
                rebound_local_controls.resolve(control_init)
            };
            self.engine.reinitialize_player_after_restore(
                binding.number,
                clonk_engine::PlayerAtClient::new(source.client_id),
                at_client_name,
                player_name,
                control.runtime_control(),
                script_player,
                no_elimination_check,
                saved_pref_control_style,
                saved_pref_auto_context_menu,
            )?;
        }

        rebound_local_controls.finalize_restored_mouse_owner(
            self.engine
                .players()
                .map(|player| (player.id(), player.status())),
        );
        self.local_controls = rebound_local_controls;
        if let Some(owner) = local_players.first().copied() {
            self.local_owner = owner;
        }
        self.engine.set_local_players(local_players);
        self.engine.finalize_restored_players()?;
        self.mouse_control = self.local_controls.mouse_owner().is_some();
        self.deferred_network_savegame_recreation.clear();
        Ok(())
    }

    pub(crate) fn finalize_network_loaded_scenario(
        &mut self,
        network_savegame: bool,
    ) -> Result<(), EngineError> {
        // Network.FinalInit runs after InitGame but before InitPlayers and
        // InitGameFinal. Ordinary network player joins remain host-issued
        // controls; scenario Initialize runs only after the status barrier
        // (pristine 9ffa0a5d src/C4Game.cpp:455-482;
        // src/C4Network2.cpp:558-615, src/C4Game.cpp:2699-2736).
        self.engine.game_start_synchronize()?;
        let network_runtime_join = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .is_some_and(|prepared| prepared.network_runtime_join);
        if network_runtime_join {
            // C4Game::InitPlayers handles NetworkRuntimeJoin in an exclusive
            // first branch. Parameters.RestorePlayerInfos must not run the
            // ordinary RestoreSavegameInfos association path first.
            self.deferred_network_savegame_recreation.clear();
        } else {
            self.prepare_network_savegame_recreation();
        }
        self.recreate_runtime_join_players()?;
        // C4Game::InitGameFinal runs Script.Initialize only for a fresh
        // scenario. A savegame already contains the initialized script and
        // object state, so invoking it again would duplicate mutations
        // (pristine 9ffa0a5d src/C4Game.cpp:2724-2734).
        if !network_savegame {
            let objects_before_initialize = self.engine.active_object_count();
            self.engine.initialize_scenario_script()?;
            self.script_created_objects =
                self.engine.active_object_count() != objects_before_initialize;
        } else {
            self.script_created_objects = false;
        }
        self.snapshot = self.engine.snapshot();
        self.rebuild_definition_sprites();
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        self.initialize_physical_viewports(false);
        self.graphics
            .apply_gamma_now(&self.snapshot.environment.gamma);
        self.refresh_object_menu();
        self.refresh_focus();
        self.advance_scenario_loader(98, "Network final initialization complete");
        self.advance_scenario_loader(99, "Runtime presentation initialized");
        Ok(())
    }

    /// `C4GraphicsResource::Init` remains re-callable while a network round
    /// runs so newly arrived Graphics-bearing groups can overload the
    /// registered set (C4GraphicsResource.cpp:278-292): RegisterMainGroups
    /// appends only groups newer than idRegisteredMainGroupSetFiles
    /// (:376-382) and LoadFile reloads a sheet only when its winning group
    /// id changed (:418-470). Mirror that pass when a Definitions resource
    /// completes mid-round: rebuild the active registered set, append every
    /// completed network definition root that is not registered yet, and
    /// rebind through the id-cached apply path under the same typed gate.
    fn refresh_network_overloaded_gui_resources(
        &mut self,
        core: &clonk_engine::NetworkResourceCore,
    ) -> Result<(), EngineError> {
        if core.resource_type != clonk_network::HostResourceType::Definitions as u8
            || self.loading_state.is_some()
        {
            return Ok(());
        }
        let Some(frontend) = self.active_scenario.clone() else {
            return Ok(());
        };
        let resolution = match self.resolve_network_overloaded_gui_resolution(&frontend) {
            Ok(Some(resolution)) => resolution,
            Ok(None) => return Ok(()),
            Err(error) => {
                // C++ registers nothing mid-round on its own; when the active
                // set cannot even be rebuilt the running round keeps its
                // current resources instead of aborting.
                tracing::error!(
                    resource_id = core.id,
                    error = format!("{error:#}"),
                    "cannot rebuild the active GUI group set for a network overloading"
                );
                return Ok(());
            }
        };
        self.assets
            .require_classic_global_gui_bootstrap_resources(&resolution.failures)
            .map_err(report_classic_parity_boundary)
            .map_err(classic_parity_engine_error)?;
        self.install_active_gui_sheet_overrides(&resolution.overrides);
        self.active_global_gui_failures = resolution.failures;
        if let Some(bundle) = resolution.font_bundle {
            self.install_active_classic_fonts(
                bundle.fonts,
                Some(bundle.tooltip),
                bundle.native_source,
            );
        }
        Ok(())
    }
}
