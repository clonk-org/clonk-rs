//! `impl GameApp` — savegames & slots methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;
use crate::game_app_scenario::remove_unassociated_savegame_player_objects_with_logs;

type OfflineSavegameRestore = (Vec<i32>, Vec<PathBuf>, Vec<(i32, Vec<u8>)>);

impl GameApp {
    pub(crate) fn developer_console_player_save_options(&self) -> (bool, bool, String) {
        let graphics = load_options_graphics_state(self.app_paths.as_ref());
        let rank_name = self
            .save_description_language_table
            .as_ref()
            .and_then(|table| table.entries.get("IDS_MSG_RANK"))
            .map(|value| clonk_script::c4_string_from_bytes(value))
            .unwrap_or_else(|| "Rank".to_string());
        (
            graphics.add_new_crew_portraits,
            graphics.save_default_portraits,
            rank_name,
        )
    }

    fn developer_console_save_parameters(&self) -> Result<Vec<u8>> {
        let seed = self
            .live_save_seed
            .as_ref()
            .ok_or_else(|| anyhow!("live save parameter seed is unavailable"))?;
        let mut parameters = seed.parameters.clone();
        parameters.random_seed = (self.engine.random_seed() as u32) as i32;
        parameters.startup_player_count = self.engine.startup_player_count().unwrap_or_else(|| {
            i32::try_from(self.control_player_infos.nonremoved_player_count()).unwrap_or(i32::MAX)
        });
        parameters.max_players = self
            .engine
            .max_players()
            .unwrap_or(seed.scenario_defaults.max_players);
        parameters.use_fair_crew = self.engine.use_fair_crew();
        parameters.fair_crew_forced = self.engine.fair_crew_forced();
        parameters.fair_crew_strength = self.engine.fair_crew_strength();
        parameters.allow_debug = self.engine.allow_debug();
        parameters.is_network_game = self.network.is_some();
        parameters.control_rate = self.engine.control_rate();
        parameters.auto_frame_skip = self.auto_frame_skip;
        parameters.player_infos = self.recording_player_info_snapshot();
        parameters.clients =
            clonk_network::JoinClientRegistrySnapshot::new(self.control_clients.snapshot());
        clonk_network::serialize_initial_network_parameters(&parameters, &seed.scenario_defaults)
            .context("serialize live save Parameters.txt")
    }

    pub(crate) fn classic_save_description(
        &self,
        title: &[u8],
        definition_modules: &[Vec<u8>],
        kind: ClassicSaveDescriptionKind,
    ) -> (Vec<u8>, Vec<u8>) {
        let table = self.save_description_language_table.as_ref();
        let resource_bytes = |key: &str, _fallback: &str| {
            let Some(table) = table else {
                return b"Language string table not loaded.".to_vec();
            };
            let Some(value) = table.entries.get(key) else {
                return format!("[Undefined: {key}]").into_bytes();
            };
            value.clone()
        };

        // C4Language::Init overwrites Config.General.LanguageCharset from the
        // active resource table. Keep legacy code-page bytes opaque; only the
        // ASCII charset identifier is interpreted here.
        let charset_name = table
            .and_then(|table| table.entries.get("IDS_LANG_CHARSET"))
            .and_then(|value| std::str::from_utf8(value).ok())
            .unwrap_or_default();
        let charset_code = classic_rtf_charset_code(charset_name);
        let language = self.save_description_language.clone();
        let mut title = clonk_script::c4_string_from_bytes(title);
        Markup::strip_markup(&mut title);
        let title = clonk_script::c4_string_bytes(&title);
        let now = classic_calendar_time_now();
        let (date_key, date_fallback) = match kind {
            ClassicSaveDescriptionKind::Record => {
                ("IDS_DESC_DATEREC", "Recording from %i.%i.%i %02d:%02d.")
            }
            ClassicSaveDescriptionKind::Savegame if self.network.is_some() => {
                ("IDS_DESC_DATENET", "Network game from %i.%i.%i %02d:%02d.")
            }
            ClassicSaveDescriptionKind::Savegame => {
                ("IDS_DESC_DATE", "Game saved %i.%i.%i %02d:%02d.")
            }
        };
        let date_template = resource_bytes(date_key, date_fallback);
        let mut lines = vec![(
            developer_console_save::format_resource_integers(
                &date_template,
                &[now.day, now.month, now.year, now.hour, now.minute],
            ),
            true,
        )];

        let game_time = self.engine.game_time();
        if game_time != 0 {
            let duration = resource_bytes("IDS_DESC_DURATION", "Playing time: %02d:%02d:%02d.");
            lines.push((
                developer_console_save::format_resource_integers(
                    &duration,
                    &[game_time / 3_600, (game_time % 3_600) / 60, game_time % 60],
                ),
                true,
            ));
        }

        if matches!(kind, ClassicSaveDescriptionKind::Record) {
            let build = format!("{CLASSIC_ENGINE_BUILD:03}");
            let version = resource_bytes("IDS_DESC_VERSION", "Engine version: %s");
            lines.push((
                developer_console_save::format_resource_strings(&version, &[build.as_bytes()]),
                true,
            ));
        }

        if !definition_modules.is_empty() {
            let mut definitions = resource_bytes("IDS_DESC_DEFSPECS", "Object definitions: ");
            for (index, module) in definition_modules.iter().enumerate() {
                if index != 0 {
                    definitions.extend_from_slice(b", ");
                }
                let relative =
                    developer_console_definition_description_path(module, self.app_paths.as_ref());
                for byte in relative {
                    if byte == b'\\' {
                        definitions.push(b'\\');
                    }
                    definitions.push(byte);
                }
            }
            lines.push((definitions, true));
        }

        if matches!(kind, ClassicSaveDescriptionKind::Record) && self.network_is_league {
            let league = resource_bytes("IDS_PRC_LEAGUE", "Using league '%s'");
            lines.push((
                developer_console_save::format_resource_strings(
                    &league,
                    &[self.network_league_name.as_slice()],
                ),
                true,
            ));
        }

        if self.network.is_some() {
            let mut clients = resource_bytes("IDS_DESC_CLIENTS", "Clients: ");
            for (index, client) in self.control_clients.snapshot().iter().enumerate() {
                if index != 0 {
                    clients.extend_from_slice(b", ");
                }
                clients.extend_from_slice(client.name.as_bytes());
            }
            lines.push((clients, true));
        }

        let (_, packets) = self.control_player_infos.retained_rows_snapshot();
        let has_retained_players = packets.iter().any(|(_, _, players)| !players.is_empty());
        let players = packets
            .iter()
            .flat_map(|(_, _, players)| players)
            .filter(|player| {
                player.is_joined() && player.flags & clonk_engine::PLAYER_INFO_FLAG_INVISIBLE == 0
            })
            .collect::<Vec<_>>();
        if has_retained_players {
            let label = resource_bytes("IDS_DESC_PLRS", "Players: ");
            let team_configuration = self.engine.team_configuration();
            if team_configuration.active && !team_configuration.auto_generate_teams {
                lines.push((label, true));
                let mut known_team_ids = HashSet::new();
                for team in self.engine.teams() {
                    known_team_ids.insert(team.id);
                    let members = players
                        .iter()
                        .copied()
                        .filter(|player| player.team == team.id)
                        .collect::<Vec<_>>();
                    if members.is_empty() {
                        continue;
                    }
                    let mut line = clonk_script::c4_string_bytes(&team.name);
                    line.extend_from_slice(b": ");
                    append_description_player_names(&mut line, &members);
                    lines.push((line, true));
                }
                let unassigned = players
                    .iter()
                    .copied()
                    .filter(|player| !known_team_ids.contains(&player.team))
                    .collect::<Vec<_>>();
                if !unassigned.is_empty() {
                    let mut line = Vec::new();
                    append_description_player_names(&mut line, &unassigned);
                    lines.push((line, true));
                }
            } else {
                let mut line = label;
                append_description_player_names(&mut line, &players);
                lines.push((line, !players.is_empty()));
            }
        }

        let mut filename = b"Desc".to_vec();
        filename.extend_from_slice(&language);
        filename.extend_from_slice(b".rtf");
        (
            filename,
            developer_console_save::serialize_savegame_description(&title, charset_code, &lines),
        )
    }

    fn developer_console_savegame_description(
        &self,
        title: &str,
        definition_modules: &[Vec<u8>],
    ) -> (Vec<u8>, Vec<u8>) {
        self.classic_save_description(
            &clonk_script::c4_string_bytes(title),
            definition_modules,
            ClassicSaveDescriptionKind::Savegame,
        )
    }

    pub(crate) fn save_developer_console_game(
        &mut self,
        kind: ConsoleSaveKind,
        requested_target: Option<&Path>,
    ) -> Result<bool> {
        self.save_native_c4_game(kind, requested_target, true, None)
    }

    pub(crate) fn save_native_c4_game(
        &mut self,
        kind: ConsoleSaveKind,
        requested_target: Option<&Path>,
        retarget_active_scenario: bool,
        title_png: Option<&[u8]>,
    ) -> Result<bool> {
        anyhow::ensure!(
            self.mode == AppMode::Running,
            "cannot save while no game is running"
        );
        let active = self
            .active_scenario
            .clone()
            .ok_or_else(|| anyhow!("active scenario metadata is unavailable"))?;
        let validation_policy = match kind {
            ConsoleSaveKind::Scenario => clonk_engine::LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            ConsoleSaveKind::Savegame => clonk_engine::LiveC4SavePolicy::Savegame {
                target_group_name: "",
            },
        };
        let restore_plan = runtime_join_save::set_as_live_save_restore_infos(
            &self.control_clients.snapshot(),
            &self.recording_player_info_snapshot(),
            self.network.is_some(),
            validation_policy.player_policy(),
        );
        restore_plan.validate_for_live_save(
            validation_policy,
            self.engine.players().map(|player| player.player_info_id()),
        )?;
        let mut source_path = if retarget_active_scenario {
            active.path.clone()
        } else {
            self.live_save_seed
                .as_ref()
                .map(|seed| seed.scenario_source_path.clone())
                .or_else(|| active.path.clone())
        }
        .ok_or_else(|| anyhow!("active scenario has no filesystem path"))?;
        let retained_origin = (!retarget_active_scenario).then(|| {
            self.live_save_seed
                .as_ref()
                .map(|seed| seed.scenario_origin.clone())
                .unwrap_or_else(|| {
                    record_scenario_origin(
                        &source_path,
                        self.app_paths.as_ref(),
                        &active.identifier,
                    )
                })
        });

        // FileSave's overwrite guard precedes SaveGame's host/child guards;
        // FileSaveAs deliberately bypasses it by copying to a fresh target.
        if retarget_active_scenario
            && kind == ConsoleSaveKind::Savegame
            && requested_target.is_none()
        {
            let source = open_group_path_for_folder_map(&source_path)
                .with_context(|| format!("open {}", source_path.display()))?;
            if !ScenarioLoaderHead::load_from_group(&source)?.is_save_game() {
                self.show_developer_console_message(
                    self.runtime_resource_text(
                        "IDS_CNS_NOGAMEOVERSCEN",
                        "You should not overwrite the original scenario file with a save game.",
                    ),
                    None,
                )?;
                return Ok(false);
            }
        }

        let mut destination = requested_target
            .map(Path::to_path_buf)
            .unwrap_or_else(|| source_path.clone());
        if requested_target.is_some() && destination.extension().is_none() {
            destination.set_extension("c4s");
        }

        if requested_target.is_some() {
            if !retarget_active_scenario && cpp_loader_items_identical(&source_path, &destination)?
            {
                anyhow::bail!(
                    "cannot save the running scenario over itself: {}",
                    destination.display()
                );
            }
            if !retarget_active_scenario {
                match fs::symlink_metadata(&destination) {
                    Ok(_) => remove_file_or_directory(&destination).with_context(|| {
                        format!("erase previous quick-save slot {}", destination.display())
                    })?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("inspect quick-save slot {}", destination.display())
                        });
                    }
                }
            }
            // FileSaveAs closes the current group, copies the complete source
            // with C4Group_CopyItem, changes ScenarioFilename/caption, and
            // reopens the copy *before* SaveGame applies its host guard.
            // Preserve unpacked directories just as CopyDirectory does.
            if retarget_active_scenario {
                if let Some(active) = self.active_scenario.as_mut() {
                    active.identifier = destination.to_string_lossy().into_owned();
                    active.path = Some(destination.clone());
                    active.source_paths = vec![destination.clone()];
                }
                if let Some(seed) = self.live_save_seed.as_mut() {
                    seed.scenario_source_path = destination.clone();
                    seed.scenario_origin = record_scenario_origin(
                        &destination,
                        self.app_paths.as_ref(),
                        &active.identifier,
                    );
                }
                self.classic_command_line.scenario = Some(destination.clone());
            }
            let copy_result = (|| -> Result<()> {
                let source = open_group_path_for_folder_map(&source_path)
                    .with_context(|| format!("open source scenario {}", source_path.display()))?;
                let copy = MutableGroup::from_group(&source)
                    .with_context(|| format!("copy source scenario {}", source_path.display()))?;
                persist_console_save_group(
                    &copy,
                    &destination,
                    retarget_active_scenario && source_path.is_dir(),
                )
                .with_context(|| format!("copy scenario to {}", destination.display()))
            })();
            if let Err(error) = copy_result {
                tracing::error!(%error, target = %destination.display(), "native C4Group save copy failed");
                if !retarget_active_scenario {
                    return Err(error);
                }
                let target = destination.to_string_lossy();
                let message = format_resource_string(
                    self.runtime_resource_text(
                        "IDS_CNS_SAVEASERROR",
                        "Error while saving the scenario to %s.",
                    ),
                    &[&target],
                );
                self.show_developer_console_message(message, None)?;
                return Ok(false);
            }
            source_path = destination.clone();
        }

        if self.network.is_some() && !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            self.show_developer_console_message(
                self.runtime_resource_text(
                    "IDS_GAME_NOCLIENTSAVE",
                    "Network games may be saved by the host only.",
                ),
                None,
            )?;
            return Ok(false);
        }
        if requested_target.is_none() {
            let (_, children) = scenario_logical_storage(&source_path)?;
            if !children.is_empty() {
                let filename = source_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| source_path.to_string_lossy().into_owned());
                let message = format_resource_string(
                    self.runtime_resource_text(
                        "IDS_CNS_NOCHILDSAVE",
                        "%s is located in a group folder.\nScenarios cannot be saved in closed group folders.",
                    ),
                    &[&filename],
                );
                self.show_developer_console_message(message, None)?;
                return Ok(false);
            }
        }
        let source = open_group_path_for_folder_map(&source_path)
            .with_context(|| format!("open source scenario {}", source_path.display()))?;
        let save_title_components = if kind == ConsoleSaveKind::Savegame {
            if self.engine.frame() == 0 {
                ["Title.bmp", "Title.png"]
                    .into_iter()
                    .filter_map(|name| source.read_file(name).ok().map(|payload| (name, payload)))
                    .collect::<Vec<_>>()
            } else {
                title_png
                    .map(|payload| vec![("Title.png", payload.to_vec())])
                    .unwrap_or_default()
            }
        } else {
            Vec::new()
        };
        let mut group = MutableGroup::from_group(&source)
            .with_context(|| format!("copy source scenario {}", source_path.display()))?;
        let preserve_folder_group = source_path.is_dir();
        let mut folder_save_journal = if preserve_folder_group {
            developer_console_save::FolderSaveJournal::default()
        } else {
            developer_console_save::FolderSaveJournal::disabled()
        };
        if !self.process_group_maker.is_empty() {
            // Closing an owned C4Group stamps the process-global maker on
            // the root header even when the save later reports failure.
            group.set_maker_bytes(self.process_group_maker.as_bytes());
        }
        let copied_material_group_is_file = matches!(
            group.entry_kind("Material.c4g"),
            Some(MutableGroupEntryKind::File | MutableGroupEntryKind::UnopenableChildGroup)
        );

        if kind == ConsoleSaveKind::Savegame {
            if let Some(network) = self.network.as_ref() {
                if let Err(error) = network.submit_queued_synchronize(
                    self.local_control_submission_tick(),
                    true,
                    false,
                ) {
                    // Native unconditionally continues after adding the
                    // synchronization control to its queue.
                    tracing::warn!(%error, "failed to queue savegame player synchronization");
                }
            } else {
                // C4GameSaveSavegame::OnSaving synchronizes offline players
                // even when the running game is a replay.
                self.engine.checkpoint_local_player_files_for_save();
                // Native ignores this aggregate result and proceeds with the
                // savegame even if one profile could not be synchronized.
                let _ = self.persist_synchronized_local_player_files();
            }
        }

        let definition_modules = match self.active_definition_load.as_ref() {
            Some(ScenarioDefinitionLoad::Seed { modules, .. })
            | Some(ScenarioDefinitionLoad::Fixed { modules, .. }) => modules.clone(),
            None => Vec::new(),
        };
        let description_definition_modules =
            if self.active_description_definition_modules.is_empty()
                && !definition_modules.is_empty()
            {
                // State-only embedders may seed the historical String vector
                // directly. Its C4 byte projection remains the exact fallback
                // whenever no filesystem-derived byte cache exists.
                definition_modules
                    .iter()
                    .map(|module| clonk_script::c4_string_bytes(module))
                    .collect()
            } else {
                self.active_description_definition_modules.clone()
            };
        let native_config = load_native_config_bytes(self.app_paths.as_ref());
        let (definition_executable_path, definition_path) =
            game_save_definition_paths(self.app_paths.as_ref(), &native_config);
        let title = self
            .host_join_snapshot
            .as_ref()
            .map(|snapshot| native_bytes_as_legacy_text(snapshot.parameters.title.as_bytes()))
            .or_else(|| {
                self.live_save_seed
                    .as_ref()
                    .map(|seed| native_bytes_as_legacy_text(seed.scenario_title.as_bytes()))
            })
            .unwrap_or_else(|| active.title.clone());
        let origin = retained_origin.unwrap_or_else(|| {
            record_scenario_origin(&destination, self.app_paths.as_ref(), &active.identifier)
        });
        let destination_name = destination.to_string_lossy().into_owned();
        let force_exact_landscape = self
            .engine
            .landscape()
            .is_some_and(|landscape| landscape.mode() == clonk_engine::LANDSCAPE_MODE_EXACT);
        let landscape_is_static = self
            .engine
            .landscape()
            .is_some_and(|landscape| landscape.mode() == clonk_engine::LANDSCAPE_MODE_STATIC);
        let policy = match kind {
            ConsoleSaveKind::Scenario => clonk_engine::LiveC4SavePolicy::Scenario {
                force_exact_landscape,
            },
            ConsoleSaveKind::Savegame => clonk_engine::LiveC4SavePolicy::Savegame {
                target_group_name: &destination_name,
            },
        };
        // Exact SaveCore writes Parameters.txt before Scenario.txt.
        if kind == ConsoleSaveKind::Savegame {
            let parameters = self.developer_console_save_parameters()?;
            folder_save_journal.put_file(
                "Parameters.txt",
                &parameters,
                developer_console_save::FolderSaveAddFailure::Fatal,
            );
            group
                .add_file("Parameters.txt", parameters)
                .context("write live save Parameters.txt")?;
        }
        let save = match self.engine.serialize_live_c4_save_with_policy(
            clonk_engine::LiveC4SaveSpec {
                title: &title,
                definition_modules: &definition_modules,
                definition_executable_path: &definition_executable_path,
                definition_path: &definition_path,
                origin: &origin,
                music_enabled: self.runtime_music_enabled,
                copied_material_group_is_file,
                title_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                info_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                script_component: clonk_engine::LiveC4ComponentHost::Unmodified,
            },
            policy,
        ) {
            Ok(save) => save,
            Err(error) => {
                if let Some(partial) = error.pre_landscape_components() {
                    let apply_result =
                        developer_console_save::apply_live_save_pre_landscape_to_group_recorded(
                            &mut group,
                            policy,
                            partial,
                            &mut folder_save_journal,
                        );
                    if !self.process_group_maker.is_empty() {
                        group.set_maker_bytes_recursively(self.process_group_maker.as_bytes());
                    }
                    let persist_result = persist_live_console_save_group(
                        &group,
                        &destination,
                        preserve_folder_group,
                        &folder_save_journal,
                        self.process_group_maker.as_bytes(),
                    );
                    apply_result?;
                    persist_result?;
                }
                return Err(error).context("serialize live C4 scenario state");
            }
        };

        let mutation_result = (|| -> Result<()> {
            developer_console_save::apply_live_save_runtime_components_to_group_recorded(
                &mut group,
                policy,
                &save,
                landscape_is_static,
                &mut folder_save_journal,
            )?;

            // C4PlayerInfoList::Save deletes the old entry before compiling.
            group.remove_entry("SavePlayerInfos.txt");
            folder_save_journal.delete_entry("SavePlayerInfos.txt");
            if !restore_plan.restore_infos.clients.is_empty() {
                let save_player_infos =
                    clonk_network::encode_player_info_list_ini(&restore_plan.restore_infos)
                        .context("serialize SavePlayerInfos.txt")?;
                developer_console_save::add_live_save_player_infos_after_delete_to_group_recorded(
                    &mut group,
                    &save_player_infos,
                    &mut folder_save_journal,
                )?;
            }

            let maker = self.process_group_maker.as_bytes().to_vec();
            let (add_new_crew_portraits, save_default_portraits, player_rank_name_default) =
                self.developer_console_player_save_options();
            let player_options = clonk_engine::LiveC4PlayerSaveOptions {
                savegame: true,
                // C4PlayerList converts GetCreateSmallFile into
                // fStoreOnOriginal before calling C4Player::Save. Every
                // embedded player type therefore passes false.
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
            for (game_number, player_info_id) in runtime_players {
                let Some(index) = remaining_targets
                    .iter()
                    .position(|target| target.player_info_id == player_info_id)
                else {
                    continue;
                };
                let target = remaining_targets.remove(index);
                let player_group =
                    clonk_engine::serialize_live_c4_player_with_options_and_enumeration(
                        &self.engine,
                        game_number,
                        target.filename.as_bytes(),
                        &maker,
                        player_options,
                        &save.value_enumeration,
                    )
                    .with_context(|| {
                        format!(
                            "serialize player info {} (game player {})",
                            target.player_info_id, game_number
                        )
                    })?;
                developer_console_save::add_live_save_player_group_recorded(
                    &mut group,
                    runtime_join_save::SerializedRuntimeJoinPlayerGroup {
                        filename: target.filename,
                        group: player_group,
                    },
                    &mut folder_save_journal,
                )?;
            }
            // C4PlayerList::Save ignores stale restore rows without a live
            // player, then SaveDesc runs after all player children.
            if kind == ConsoleSaveKind::Savegame {
                let (description_name, description) = self.developer_console_savegame_description(
                    &title,
                    &description_definition_modules,
                );
                folder_save_journal.put_file(
                    &description_name,
                    &description,
                    developer_console_save::FolderSaveAddFailure::Ignore,
                );
                if let Err(error) = group.add_file_bytes(description_name, description) {
                    // C4GameSave::Save deliberately ignores SaveDesc failure
                    // and continues with the remaining save components.
                    tracing::warn!(%error, "failed to write live save description");
                }
            }
            for (name, payload) in save_title_components {
                folder_save_journal.put_file(
                    name,
                    &payload,
                    developer_console_save::FolderSaveAddFailure::Ignore,
                );
                if let Err(error) = group.add_file(name, payload) {
                    // SaveGameTitle failure is logged but never makes the
                    // enclosing C4GameSaveSavegame fail.
                    tracing::warn!(%error, component = name, "failed to write live save title");
                }
            }
            // Components the user edited this round. `C4ComponentHost::Save`
            // skips an unmodified host entirely and *deletes* an emptied one
            // rather than writing zero bytes (`C4ComponentHost.cpp:231-236`),
            // both of which `component_save_mutations` already decides.
            for mutation in developer_console_save::component_save_mutations(
                self.developer_component_hosts
                    .iter()
                    .map(clonk_engine::developer_components::ComponentHost::save_action),
            ) {
                match mutation {
                    developer_console_save::FolderSaveMutation::DeleteEntry { name } => {
                        let name = String::from_utf8_lossy(&name).into_owned();
                        folder_save_journal.delete_entry(&name);
                        group.remove_entry(&name);
                    }
                    developer_console_save::FolderSaveMutation::PutFile {
                        name, payload, ..
                    } => {
                        let name = String::from_utf8_lossy(&name).into_owned();
                        folder_save_journal.put_file(
                            &name,
                            &payload,
                            // Silently dropping a component the user just
                            // edited would lose their edit.
                            developer_console_save::FolderSaveAddFailure::Fatal,
                        );
                        group
                            .add_file_bytes(name.as_str(), payload)
                            .with_context(|| format!("write edited component {name}"))?;
                    }
                    // `component_save_mutations` produces only those two.
                    _ => {}
                }
            }
            Ok(())
        })();
        if let Err(error) = mutation_result {
            if !self.process_group_maker.is_empty() {
                group.set_maker_bytes_recursively(self.process_group_maker.as_bytes());
            }
            persist_live_console_save_group(
                &group,
                &destination,
                preserve_folder_group,
                &folder_save_journal,
                self.process_group_maker.as_bytes(),
            )
            .context("persist partially completed live save")?;
            return Err(error);
        }
        if !self.process_group_maker.is_empty() {
            group.set_maker_bytes_recursively(self.process_group_maker.as_bytes());
        }
        persist_live_console_save_group(
            &group,
            &destination,
            preserve_folder_group,
            &folder_save_journal,
            self.process_group_maker.as_bytes(),
        )?;
        if retarget_active_scenario {
            let success = match kind {
                ConsoleSaveKind::Scenario => {
                    self.runtime_resource_text("IDS_CNS_SCENARIOSAVED", "Scenario saved.")
                }
                ConsoleSaveKind::Savegame => {
                    self.runtime_resource_text("IDS_CNS_GAMESAVED", "Game saved.")
                }
            };
            self.developer_console.out(&success);
        }
        Ok(true)
    }

    /// `C4Game::CanQuickSave` (C4Game.cpp:2205-2223): network hosts only, and
    /// running league rounds only when they are replays.
    pub(crate) fn can_quick_save(&self) -> bool {
        self.network.is_none()
            || (matches!(self.network_mode, Some(NetworkMode::Host(_)))
                && (!self.network_is_league || self.engine.replay()))
    }

    /// The ten savegame slots (C4MainMenu.cpp:474-494).
    pub(crate) fn savegame_slots(&self) -> [SaveSlotState; 10] {
        let root = configured_savegame_directory(self.app_paths.as_ref());
        let scenario_name = self.savegame_slot_base();
        let mut slots = [SaveSlotState { free: true }; 10];
        for (index, slot) in slots.iter_mut().enumerate() {
            let slot_number = (index + 1) as u8;
            slot.free = !c4group_is_group(&classic_savegame_slot_path(
                &root,
                &scenario_name,
                slot_number,
            ));
        }
        slots
    }

    fn savegame_slot_base(&self) -> String {
        self.active_scenario
            .as_ref()
            .map(classic_savegame_scenario_name)
            .unwrap_or_else(|| sanitize_save_label(&self.scenario_label))
    }

    pub(crate) fn savegame_slot_path(&self, slot: u8) -> PathBuf {
        classic_savegame_slot_path(
            &configured_savegame_directory(self.app_paths.as_ref()),
            &self.savegame_slot_base(),
            slot,
        )
    }

    /// `Game.QuickSave(strFilename, strTitle)` for a menu slot
    /// (C4MainMenu.cpp:797-804). Unlike the Rust-only quick/custom save flow,
    /// this writes the copied scenario as a native C4Group.
    pub(crate) fn save_to_slot(&mut self, slot: u8) {
        // `C4Game::SaveGameTitle` copies the scenario's own title before the
        // first frame and otherwise screenshots under `isFullScreen && Active`
        // (C4Game.cpp:2102-2115) — the pair spelled here as
        // `!console_mode && window_active`.
        //
        // Not reachable from `run_headless_server`: the only caller is
        // `MenuAction::SaveSlot`, which arrives through
        // `dispatch_control_event_for_local_player`, and a dedicated server has
        // no local input device — `process_console_command` accepts `/quit`,
        // `/close`, `/start`, `/open` and chat, none of which open a savegame
        // menu. Should a headless path ever reach it, `window_active` has to
        // start tracking `CStdApp::Active{false}` (StdApp.h:257) first: a
        // headless `GameApp` still defaults it to `true`, so C++ would decline
        // the screenshot where the port would take one.
        let capture_title = self.engine.frame() != 0 && !self.console_mode && self.window_active;
        let title_png = if capture_title && !self.retained_gpu_presentation_active {
            let surface = self.graphics.surface();
            match encode_presented_save_thumbnail(
                surface.width(),
                surface.height(),
                surface.pixels(),
            ) {
                Ok(encoded) => Some(encoded),
                Err(error) => {
                    tracing::warn!(?error, "failed to encode native savegame Title.png");
                    None
                }
            }
        } else {
            None
        };
        let saved_path = self.save_to_slot_with_title_png(slot, title_png.as_deref());
        if capture_title && self.retained_gpu_presentation_active {
            if let Some(path) = saved_path {
                match fs::read(&path) {
                    Ok(packed_group) => {
                        self.pending_native_save_thumbnails
                            .retain(|request| request.path != path);
                        self.pending_native_save_thumbnails
                            .push_back(PendingNativeSaveThumbnail { path, packed_group });
                    }
                    Err(error) => {
                        tracing::warn!(
                            path = %path.display(),
                            ?error,
                            "failed to retain native save generation for GPU thumbnail"
                        );
                    }
                }
            }
        }
    }

    fn generate_default_save_label(&self) -> String {
        let base = self
            .active_scenario
            .as_ref()
            .map(|scenario| scenario.title.clone())
            .unwrap_or_else(|| self.scenario_label.clone());
        format!("{} {}", base, current_unix_timestamp())
    }

    pub(crate) fn finish_pending_native_save_thumbnails(&mut self, title_png: Option<&[u8]>) {
        while let Some(request) = self.pending_native_save_thumbnails.pop_front() {
            let Some(title_png) = title_png else {
                continue;
            };
            match replace_native_save_title_png_if_unchanged(
                &request,
                title_png,
                self.process_group_maker.as_bytes(),
            ) {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(
                        path = %request.path.display(),
                        "skipped stale retained GPU native-save thumbnail"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        path = %request.path.display(),
                        ?error,
                        "failed to persist retained GPU native-save thumbnail"
                    );
                }
            }
        }
    }

    fn save_to_slot_with_title_png(
        &mut self,
        slot: u8,
        title_png: Option<&[u8]>,
    ) -> Option<PathBuf> {
        let result = (|| -> Result<PathBuf> {
            anyhow::ensure!((1..=10).contains(&slot), "invalid savegame slot {slot}");
            anyhow::ensure!(self.can_quick_save(), "quick saving is not allowed");

            // QuickSave receives Game.Parameters.ScenarioTitle, which remains
            // stable even if later UI metadata changes.
            let label = self
                .host_join_snapshot
                .as_ref()
                .map(|snapshot| snapshot.parameters.title.as_bytes().to_vec())
                .or_else(|| {
                    self.live_save_seed
                        .as_ref()
                        .map(|seed| seed.scenario_title.as_bytes().to_vec())
                })
                .or_else(|| {
                    self.active_scenario
                        .as_ref()
                        .map(|scenario| clonk_script::c4_string_bytes(&scenario.title))
                })
                .unwrap_or_else(|| clonk_script::c4_string_bytes(&self.scenario_label));
            let status_label = clonk_resources::decode_legacy_script_text(&label);
            let root = configured_savegame_directory(self.app_paths.as_ref());
            let language = classic_save_folder_language(self.app_paths.as_ref());
            let root_title =
                self.runtime_resource_bytes_with_fallback("IDS_GAME_SAVEGAMESTITLE", "Savegames");
            ensure_classic_save_folder(&root, &language, &root_title)?;

            let path = self.savegame_slot_path(slot);
            let scenario_folder = path
                .parent()
                .context("classic savegame slot has no scenario folder")?;
            ensure_classic_save_folder(scenario_folder, &language, &label)?;

            let saved = self.save_main_menu_slot_game(&path, title_png)?;
            anyhow::ensure!(saved, "native savegame write was rejected");
            self.status_text = format!("Saved {status_label}");
            Ok(path)
        })();

        if let Err(err) = &result {
            tracing::error!(error = ?err, slot, "slot save failed");
            self.status_text = format!("Save failed: {err:#}");
        }
        result.ok()
    }

    fn perform_named_save(&mut self, label: &str, target: Option<PathBuf>) -> Result<PathBuf> {
        self.perform_named_save_with_label_policy(label, target, false)
    }

    fn perform_named_save_with_label_policy(
        &mut self,
        label: &str,
        target: Option<PathBuf>,
        preserve_label: bool,
    ) -> Result<PathBuf> {
        if self.mode != AppMode::Running {
            anyhow::bail!("cannot save while not running a scenario");
        }

        let scenario = self
            .active_scenario
            .clone()
            .unwrap_or_else(FrontendScenario::fallback);
        let savegame_policy = clonk_engine::LiveC4SavePolicy::Savegame {
            target_group_name: "",
        };
        let source_restore_plan = runtime_join_save::set_as_live_save_restore_infos(
            &self.control_clients.snapshot(),
            &self.recording_player_info_snapshot(),
            self.network.is_some(),
            savegame_policy.player_policy(),
        );
        source_restore_plan.validate_for_live_save(
            savegame_policy,
            self.engine.players().map(|player| player.player_info_id()),
        )?;
        let source_save_player_infos =
            Some(if source_restore_plan.restore_infos.clients.is_empty() {
                Vec::new()
            } else {
                clonk_network::encode_player_info_list_ini(&source_restore_plan.restore_infos)
                    .context("serialize saved source SavePlayerInfos.txt")?
            });
        let source_string_table = Some(self.engine.enumerate_live_c4_string_table_for_save());
        let engine_state = self.engine.capture_state();
        let stored_label = if preserve_label {
            label.to_string()
        } else if label.trim().is_empty() {
            self.generate_default_save_label()
        } else {
            label.trim().to_string()
        };

        let saved = SavedGameFile {
            version: SAVE_FILE_VERSION,
            saved_at_seconds: current_unix_timestamp(),
            scenario: SavedScenarioInfo::from_frontend(
                &scenario,
                &self.scenario_label,
                self.fallback_ground,
            ),
            definition_load: self.active_definition_load.clone(),
            focus_id: self.focus_id,
            user_label: Some(stored_label.clone()),
            runtime_music_enabled: Some(self.runtime_music_enabled),
            source_save_player_infos,
            source_string_table,
            source_title_png: None,
            engine_state,
        };

        let path = match target {
            Some(path) => path,
            None => {
                let dir = ensure_save_directory()?;
                let base = sanitize_save_label(&stored_label);
                unique_save_path(&dir, &base)
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create save directory at {}", parent.display())
            })?;
        }

        let mut file = File::create(&path)
            .with_context(|| format!("failed to create save file at {}", path.display()))?;
        serde_json::to_writer_pretty(&mut file, &saved).context("failed to serialise save data")?;
        file.flush().context("failed to flush save data")?;

        self.write_save_thumbnail(&path)?;
        self.last_save_path = Some(path.clone());
        self.status_text = format!("Saved {}", saved.scenario.title);
        Ok(path)
    }

    pub(crate) fn write_save_thumbnail(&mut self, path: &Path) -> Result<()> {
        let target = path.with_extension("png");
        if self.retained_gpu_presentation_active {
            self.pending_gpu_thumbnail_paths.push_back(target);
            return Ok(());
        }
        let surface = self.graphics.surface();
        let encoded =
            encode_surface_to_png(surface).context("failed to encode save thumbnail image")?;
        let mut file = File::create(&target)
            .with_context(|| format!("failed to create thumbnail at {}", target.display()))?;
        file.write_all(&encoded)
            .context("failed to write save thumbnail")?;
        file.flush()
            .context("failed to flush save thumbnail to disk")?;
        Ok(())
    }

    pub(crate) fn save_next_screenshot(
        &mut self,
        presented_frame: Option<&[u8]>,
        physical_width: u32,
        physical_height: u32,
        scale: f32,
    ) -> Option<ScreenshotSaveOutcome> {
        let request = self.pending_screenshots.pop_front()?;
        let kind = request.kind;
        let (path, directory_result) = prepare_numbered_screenshot_path(self.app_paths.as_ref());
        let result = (|| -> Result<()> {
            directory_result?;
            // C4Application::isFullScreen distinguishes the graphical client
            // from `/console`, not an OS fullscreen window. Rust has no
            // console frontend, so only lpBack needs an explicit equivalent.
            let presented_frame = presented_frame
                .context("screenshot capture requires an initialized presentation back buffer")?;
            match kind {
                ScreenshotKind::PresentedFrame => {
                    write_screenshot(&path, physical_width, physical_height, presented_frame)
                }
                ScreenshotKind::FullLandscape => {
                    let surface = self
                        .graphics
                        .render_full_landscape_with_gamma(&self.snapshot, &request.gamma)
                        .context("full-landscape screenshot requires an active viewport")?;
                    let width = scaled_screenshot_extent(surface.width(), scale)?;
                    let height = scaled_screenshot_extent(surface.height(), scale)?;
                    let frame_len = (width as usize)
                        .checked_mul(height as usize)
                        .and_then(|pixels| pixels.checked_mul(4))
                        .context("full-landscape screenshot dimensions overflow")?;
                    let mut frame = vec![0_u8; frame_len];
                    clonk_scaling::upscale_frame(
                        surface.pixels(),
                        surface.width(),
                        surface.height(),
                        &mut frame,
                        width,
                        height,
                    );
                    write_screenshot(&path, width, height, &frame)
                }
            }
        })();

        // Both Rust paths already contain the gamma used for presentation:
        // F9 copies the physical frame and the full-map world pass encodes
        // fragments with the request-time installed ramp. Applying it again
        // would double it.
        Some(ScreenshotSaveOutcome { kind, path, result })
    }

    pub(crate) fn prepare_network_savegame_recreation(
        &mut self,
        save_game: bool,
    ) -> Result<(), EngineError> {
        let restore_player_infos = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| prepared.restore_player_infos.clone())
            .unwrap_or_default();
        if !restore_player_infos
            .iter()
            .any(|restore| restore.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
        {
            self.deferred_network_savegame_recreation.clear();
            return Ok(());
        }
        self.deferred_network_savegame_recreation = route_network_savegame_recreation(
            &mut self.control_player_infos,
            &restore_player_infos,
        );
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        remove_unassociated_savegame_player_objects_with_logs(
            &mut self.engine,
            &self.control_player_infos,
            &restore_player_infos,
            save_game,
            &self.startup_tooltip_resources,
        )?;

        let memberships = ordered_control_player_team_memberships(&self.control_player_infos);
        let exact_teams = self.network_team_assignment.as_mut().map(|assignment| {
            self.control_player_infos
                .recheck_team_players(assignment.teams_mut());
            let metadata = assignment.teams().clone();
            (
                runtime_teams_from_initial_metadata(&metadata),
                clonk_network::join_team_list_snapshot(metadata),
            )
        });
        let runtime_teams = if let Some((runtime_teams, snapshot)) = exact_teams {
            if let Some(host_snapshot) = self.host_join_snapshot.as_mut() {
                host_snapshot.parameters.teams = snapshot;
            }
            runtime_teams
        } else {
            let mut runtime_teams = self.engine.teams().to_vec();
            recheck_runtime_team_memberships_from_infos(&mut runtime_teams, &memberships);
            if let Some(host_snapshot) = self.host_join_snapshot.as_mut() {
                recheck_join_team_memberships_from_infos(
                    &mut host_snapshot.parameters.teams.teams,
                    &memberships,
                );
            }
            runtime_teams
        };
        self.engine.set_teams(runtime_teams.clone());
        if let Some(prepared) = self
            .loading_state
            .as_mut()
            .and_then(|loading| loading.prepared_go.as_mut())
        {
            prepared.team_registry = runtime_teams;
        }
        self.publish_current_host_player_infos();
        if !self.deferred_network_savegame_recreation.is_empty() {
            tracing::debug!(
                players = ?self.deferred_network_savegame_recreation,
                "routed joined savegame infos to deferred recreation"
            );
        }
        Ok(())
    }

    pub(crate) fn save_options_advanced_changes(
        &self,
        changes: &[clonk_frontend::startup_options_advanced::AdvancedConfigChange],
    ) -> io::Result<()> {
        let Some(paths) = self.app_paths.as_ref() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "application paths are unavailable",
            ));
        };
        let path = paths.config_file();
        let mut config = match Config::load(&path) {
            Ok(config) => config,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
            Err(error) => return Err(error),
        };
        if self.apply_open_options_config(&mut config).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the Options dialog is unavailable",
            ));
        }
        // C4StartupOptionsDlg::SaveConfig calls Config.Save after applying
        // the open dialog (`C4StartupOptionsDlg.cpp:1182-1183`). Keep the
        // in-memory Display values in that explicit save just as the C++
        // Config object does; the Display menu's regular path remains
        // deferred until C4Application::Quit (`C4Application.cpp:351-367`).
        self.apply_display_flags_to_config(&mut config);
        advanced_config::canonicalize_existing(&mut config);
        advanced_config::apply_changes(&mut config, changes);
        let fair_crew = changes
            .iter()
            .rev()
            .find(|change| change.section == "General" && change.key == "NoCrew")
            .map(|change| parse_config_bool(&change.value));
        let always_debug = changes
            .iter()
            .rev()
            .find(|change| change.section == "General" && change.key == "DebugMode")
            .map(|change| parse_config_bool(&change.value));
        let gamepads_enabled = config
            .get_in(Some("General"), "GamepadEnabled")
            .map(parse_config_bool);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        save_config_preserving_native_general_booleans(
            &config,
            &path,
            gamepads_enabled,
            always_debug,
        )?;
        if let Some(enabled) = fair_crew {
            persist_native_config_values(
                paths,
                "General",
                &[(
                    "NoCrew",
                    clonk_app_netplay::NativeConfigValue::RawAscii(if enabled {
                        "true"
                    } else {
                        "false"
                    }),
                )],
            )?;
        }
        Ok(())
    }

    /// `C4PlayerInfoList::RecreatePlayers`' engine half: the restored players
    /// exist from here on, which `InitGameFinal`'s script calls rely on
    /// (C4Game.cpp:479 runs `InitPlayers` before :484).
    pub(crate) fn restore_offline_savegame_engine_players(
        &mut self,
        engine: &mut Engine,
        scenario_path: &Path,
        savegame: &OfflineSavegameStartup,
    ) -> Result<OfflineSavegameRestore, EngineError> {
        if savegame.runtime_players.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        let mut local_players = Vec::new();
        let mut joined_player_files = Vec::new();
        let mut prejoin_recorded_player_files = Vec::new();
        let mut filename_ledger = clonk_engine::RuntimeJoinPlayerFilenameLedger::default();
        for source in &savegame.runtime_players {
            if self.recording_enabled || self.network_is_league {
                if let Some(path) = savegame.recreation_record_paths.get(&source.info.id) {
                    match packed_group_bytes(path, self.process_group_maker.as_bytes()) {
                        Ok(bytes) => prejoin_recorded_player_files.push((source.info.id, bytes)),
                        Err(error) => tracing::warn!(
                            info_id = source.info.id,
                            path = %path.display(),
                            %error,
                            "failed to capture offline player file before recreation"
                        ),
                    }
                }
            }
            let result = engine.restore_offline_savegame_players_from_path_with_filename_ledger(
                scenario_path,
                std::slice::from_ref(source),
                &savegame.external_player_paths,
                savegame.save_game,
                &mut filename_ledger,
            );
            if savegame.embedded_player_info_ids.contains(&source.info.id) {
                self.control_player_infos
                    .clear_recreated_temporary_player_file(source.info.id, false);
            }
            match result {
                Ok(mut bindings) => {
                    if let Some(binding) = bindings.pop() {
                        let (
                            saved_mouse_control,
                            preferred_control_set,
                            prefers_mouse,
                            saved_pref_control_style,
                            saved_pref_auto_context_menu,
                            saved_player_name,
                        ) = {
                            let player = engine
                                .player(binding.number)
                                .ok_or(EngineError::UnknownPlayer(binding.number))?;
                            let (preferred_control_set, prefers_mouse) =
                                player.control_preferences();
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
                        let player_name = if control_player_effective_name(&current_info).is_empty()
                        {
                            saved_player_name
                        } else {
                            clonk_script::c4_string_from_bytes(control_player_effective_name(
                                &current_info,
                            ))
                        };
                        let control_init = LocalControlInit {
                            owner: binding.number,
                            preferred_set: preferred_control_set,
                            prefers_mouse,
                            gamepads_enabled: self.gamepads_enabled,
                            replay: false,
                            disable_mouse: !self.mouse_control_allowed,
                        };
                        let control = if script_player {
                            self.local_controls.resolve(control_init)
                        } else {
                            let control = self
                                .local_controls
                                .initialize_after_restore(control_init, saved_mouse_control != 0);
                            local_players.push(binding.number);
                            control
                        };
                        engine.reinitialize_player_after_restore(
                            binding.number,
                            clonk_engine::PlayerAtClient::HOST,
                            "Local",
                            player_name,
                            control.runtime_control(),
                            script_player,
                            current_info.no_elimination_check(),
                            saved_pref_control_style,
                            saved_pref_auto_context_menu,
                        )?;
                        if let Some(player_path) =
                            savegame.external_player_paths.get(&binding.player_info_id)
                        {
                            self.local_player_profile_paths
                                .insert(binding.player_info_id, player_path.clone());
                            let icon = load_local_player_big_icon(player_path);
                            self.cache_joined_player_big_icon(
                                binding.player_info_id,
                                icon.as_ref(),
                            );
                            if let Ok(real_path) = offline_player_real_path(player_path) {
                                joined_player_files.push(real_path);
                            }
                        }
                    }
                }
                Err(error @ clonk_engine::RuntimeJoinPlayerRestoreError::ProvisionalRemoval(_)) => {
                    return Err(EngineError::from(error));
                }
                Err(error) => {
                    tracing::warn!(
                        info_id = source.info.id,
                        %error,
                        "failed to recreate one offline savegame player; continuing"
                    );
                    if !matches!(
                        error,
                        clonk_engine::RuntimeJoinPlayerRestoreError::ZeroPlayerInfoId(_)
                    ) {
                        self.control_player_infos.mark_removed(
                            source.info.id,
                            false,
                            i32::try_from(engine.frame()).unwrap_or(i32::MAX),
                        );
                    }
                }
            }
        }
        self.local_controls.finalize_restored_mouse_owner(
            engine
                .players()
                .map(|player| (player.id(), player.status())),
        );
        engine.set_local_players(local_players.iter().copied());
        self.mouse_control = self.local_controls.mouse_owner().is_some();
        Ok((
            local_players,
            joined_player_files,
            prejoin_recorded_player_files,
        ))
    }

    pub(crate) fn quick_save(&mut self) -> Result<()> {
        let dir = ensure_save_directory()?;
        let path = dir.join(QUICK_SAVE_FILE);
        self.perform_named_save("Quick Save", Some(path))?;
        Ok(())
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod save_thumbnail_tests {
    use super::*;
    use png::Decoder;

    /// Vertical stripes with a period of four: one white column followed by
    /// three black ones.
    fn striped_frame(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|_| 0..width)
            .flat_map(|x| {
                let level = if x % 4 == 0 { 255 } else { 0 };
                [level, level, level, 255]
            })
            .collect()
    }

    #[test]
    fn save_thumbnail_averages_every_source_pixel_of_the_frame() {
        // 800x600 -> 200x150 is a 4x reduction, so every destination cell
        // covers exactly one white and three black columns and must come out
        // at 255/4. A two-tap bilinear reduction samples only the two middle
        // columns of each cell — both black — and loses the stripes entirely.
        let encoded = encode_presented_save_thumbnail(800, 600, &striped_frame(800, 600))
            .expect("encode area-reduced save thumbnail");
        let mut reader = Decoder::new(io::Cursor::new(encoded))
            .read_info()
            .expect("read save thumbnail header");
        let mut buffer = vec![
            0;
            reader
                .output_buffer_size()
                .expect("save thumbnail buffer size fits usize")
        ];
        let info = reader
            .next_frame(&mut buffer)
            .expect("decode save thumbnail");

        assert_eq!(
            (info.width, info.height),
            (SAVE_THUMBNAIL_WIDTH, SAVE_THUMBNAIL_HEIGHT)
        );
        assert!(
            buffer
                .chunks_exact(4)
                .all(|pixel| pixel == [64, 64, 64, 255]),
            "every thumbnail cell must average its whole 4x4 source block"
        );
    }

    #[test]
    fn save_thumbnail_rejects_a_frame_whose_length_disagrees_with_its_extent() {
        assert!(encode_presented_save_thumbnail(4, 4, &[0; 60]).is_err());
        assert!(encode_presented_save_thumbnail(0, 0, &[]).is_err());
    }
}
