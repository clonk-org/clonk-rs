//! Deterministic startup-state checkpoints for presentation pixel captures.
//!
//! These helpers stop the real lobby and loader pipelines at the exact state
//! the capture driver renders. They do not construct look-alike frontend
//! models: the retained [`GameApp`] state remains the renderer's input.

use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::{FrontendScenario, GameApp, ScenarioLoadingEvent};

const CHECKPOINT_TIMEOUT: Duration = Duration::from_secs(30);
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StartupPixelCheckpoint {
    pub(crate) simulation_seed: u64,
    pub(crate) random_count: u64,
    pub(crate) render_ordinal: u32,
}

pub(crate) fn stage_network_lobby_checkpoint(
    app: &mut GameApp,
    scenario: FrontendScenario,
) -> Result<StartupPixelCheckpoint> {
    anyhow::ensure!(
        app.startup_network_connection.is_none() && app.network.is_none(),
        "network-lobby capture requires an idle network session"
    );
    let definition_load = app.scenario_seed_definition_load();
    app.stage_network_host_scenario(scenario, definition_load)
        .map_err(anyhow::Error::from)
        .context("staging the real network host for presentation capture")?;

    let deadline = Instant::now() + CHECKPOINT_TIMEOUT;
    while app.startup_network_connection.is_some() || app.pending_network_host_preparation.is_some()
    {
        app.poll_startup_network_connection()
            .map_err(anyhow::Error::from)
            .context("polling the presentation capture's network host")?;
        app.poll_pending_network_host_preparation()
            .map_err(anyhow::Error::from)
            .context("publishing the presentation capture's network host resources")?;
        anyhow::ensure!(
            Instant::now() < deadline,
            "network host did not reach its live lobby capture checkpoint"
        );
        if app.startup_network_connection.is_some()
            || app.pending_network_host_preparation.is_some()
        {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    let dialogs = app
        .dialogs
        .messages
        .iter()
        .map(|dialog| format!("{}: {}", dialog.state.caption(), dialog.state.message()))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        app.startup.view == crate::StartupView::NetworkLobby
            && app.network.is_some()
            && matches!(app.network_mode, Some(crate::NetworkMode::Host(_)))
            && (app.classic_host_lobby.is_some() || app.network_lobby.is_some()),
        "network host did not leave a live lobby ready for capture: view={:?}, network={}, mode={:?}, classic_lobby={}, fallback_lobby={}, status={:?}, dialogs={dialogs:?}",
        app.startup.view,
        app.network.is_some(),
        app.network_mode.as_ref().map(|mode| match mode {
            crate::NetworkMode::Host(_) => "host",
            crate::NetworkMode::Client(_) => "client",
        }),
        app.classic_host_lobby.is_some(),
        app.network_lobby.is_some(),
        app.status_text,
    );
    let notice_count = app
        .dialogs
        .messages
        .iter()
        .filter(|dialog| {
            matches!(
                dialog.continuation,
                crate::MessageDialogContinuation::CompatProfileLobbyNotice { .. }
            )
        })
        .count();
    anyhow::ensure!(
        notice_count == 1 && app.dialogs.messages.len() == 1,
        "network lobby did not present exactly one compatibility notice"
    );
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .map_err(anyhow::Error::from)
        .context("acknowledging the compatibility notice over the live lobby")?;
    anyhow::ensure!(
        app.startup.view == crate::StartupView::NetworkLobby
            && app.dialogs.messages.is_empty()
            && app.status_text.is_empty(),
        "acknowledging the compatibility notice displaced the live lobby"
    );

    let simulation_seed = app
        .host_join_snapshot
        .as_ref()
        .map(|snapshot| u64::from(snapshot.parameters.random_seed as u32))
        .context("the live host lobby has no synchronized Parameters.RandomSeed")?;
    // DoLobby precedes InitGameSecondPart::FixRandom, so native's process
    // ledger is still zero here (C4Network2.cpp:451-485;
    // C4Game.cpp:2642-2652). The retained Engine is only a pre-round
    // placeholder and its prebuilt Rnd3 table is not the live native state.
    Ok(StartupPixelCheckpoint {
        simulation_seed,
        random_count: 0,
        render_ordinal: 2,
    })
}

pub(crate) fn stage_loader_checkpoint(
    app: &mut GameApp,
    scenario: FrontendScenario,
) -> Result<StartupPixelCheckpoint> {
    anyhow::ensure!(
        app.loading_state.is_none(),
        "loader capture requires an idle scenario loader"
    );
    app.start_scenario(scenario)
        .map_err(anyhow::Error::from)
        .context("starting the real scenario loader for presentation capture")?;

    let deadline = Instant::now() + CHECKPOINT_TIMEOUT;
    loop {
        let now = Instant::now();
        anyhow::ensure!(
            now < deadline,
            "scenario loader did not reach the exact 60% capture checkpoint"
        );
        let wait = deadline
            .saturating_duration_since(now)
            .min(WORKER_POLL_INTERVAL);
        let event = app
            .loading_state
            .as_ref()
            .context("the scenario loader disappeared before the capture checkpoint")?
            .receiver
            .recv_timeout(wait);
        let event = match event {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                bail!("scenario loader disconnected before the capture checkpoint")
            }
        };
        match event {
            ScenarioLoadingEvent::LoaderFrame { progress, log } => {
                anyhow::ensure!(
                    progress <= 60,
                    "scenario loader skipped the exact 60% capture checkpoint and reached {progress}%"
                );
                app.apply_scenario_loader_frame(progress, log);
                if progress == 60 {
                    break;
                }
            }
            ScenarioLoadingEvent::RefreshResources => {
                bail!("scenario loader refreshed resources before its 60% capture checkpoint")
            }
            ScenarioLoadingEvent::AcceptedRandomSeed(_) => {
                bail!("scenario loader replaced its seed before its 60% capture checkpoint")
            }
            ScenarioLoadingEvent::Finished(_) => {
                bail!("scenario loader finished before its 60% capture checkpoint")
            }
        }
    }

    let loading = app
        .loading_state
        .as_ref()
        .context("the scenario loader disappeared at its capture checkpoint")?;
    let expected_log_tail = [
        "C4AulScriptEngine linked - 24442 lines, 0 warnings, 0 errors",
        "Texture table holds 48 entries.",
        "21 textures loaded.",
        "21 materials loaded.",
    ];
    anyhow::ensure!(
        loading.last_progress == 60
            && loading.log.len() >= expected_log_tail.len()
            && loading.log[loading.log.len() - expected_log_tail.len()..] == expected_log_tail,
        "scenario loader's 60% frame did not carry the native link/material log tail: {:?}",
        loading
            .log
            .iter()
            .rev()
            .take(expected_log_tail.len())
            .rev()
            .collect::<Vec<_>>()
    );
    let simulation_seed = loading
        .offline_random_seed
        .context("the loader capture has no frozen simulation seed")?;
    // The retained Engine is only GameApp's pre-round placeholder here.
    // Native has selected Parameters.RandomSeed but does not call FixRandom
    // and spend Randomize3's 500 draws until InitGameSecondPart, after the 60%
    // frame (C4Game.cpp:2635,2642-2652; C4Random.cpp:24-33). Report the live
    // pre-init ledger, not the placeholder engine's already-built Rnd3 table.
    let random_count = 0;
    Ok(StartupPixelCheckpoint {
        simulation_seed,
        random_count,
        render_ordinal: 2,
    })
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;

    use super::*;
    use crate::{AppMode, AudioOptions, RuntimeConfig};

    struct TestEnvironment {
        _lock: parking_lot::ReentrantMutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl TestEnvironment {
        fn install(repository: &Path, user_data: &Path) -> Self {
            let lock = crate::tests::env_lock().lock();
            crate::reset_cached_app_paths();
            let values = [
                ("LC_INSTALL_ROOT", Some(repository.as_os_str().to_owned())),
                (
                    "LC_CONTENT_DIR",
                    Some(repository.join("content").into_os_string()),
                ),
                ("LC_USER_DATA_DIR", Some(user_data.as_os_str().to_owned())),
                ("LC_PIN_SEED", Some(OsString::from("587"))),
            ];
            let saved = values
                .iter()
                .map(|(name, _)| (*name, env::var_os(name)))
                .collect();
            for (name, value) in values {
                env::set_var(name, value.expect("test environment value"));
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            for (name, value) in self.saved.drain(..) {
                match value {
                    Some(value) => env::set_var(name, value),
                    None => env::remove_var(name),
                }
            }
            crate::reset_cached_app_paths();
        }
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("clonk-app belongs to the workspace")
            .to_path_buf()
    }

    fn capture_app() -> Result<(
        TestEnvironment,
        tempfile::TempDir,
        GameApp,
        FrontendScenario,
    )> {
        let repository = repository_root();
        let user_data = tempfile::Builder::new()
            .prefix("lc-presentation-startup-")
            .tempdir()?;
        let environment = TestEnvironment::install(&repository, user_data.path());
        let config_path = user_data.path().join("presentation.config");
        let config = fs::read_to_string(repository.join("compat/presentation/rust.config"))?
            .replace("PortUDP=41118", "PortUDP=0")
            .replace("PortRefServer=41119", "PortRefServer=0");
        fs::write(&config_path, config)?;
        let player_path = user_data.path().join("Presentation.c4p");
        fs::copy(
            repository.join("compat/presentation/player.c4p"),
            &player_path,
        )?;
        let paths = crate::AppPaths::discover_with_config_file(Some(&config_path))?;
        paths.ensure_user_dirs()?;
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = "Tutorial.c4f/Tutorial01.c4s".to_owned();
        scenario.title = "Tutorial 01".to_owned();
        scenario.path = Some(repository.join("content/Tutorial.c4f/Tutorial01.c4s"));
        let mut app = GameApp::new_with_frontend_scenarios_for_profile(
            1_280,
            720,
            AudioOptions {
                sound_enabled: false,
                music_enabled: false,
                menu_music_enabled: false,
                menu_sound_enabled: false,
                ..AudioOptions::default()
            },
            Some(&paths),
            RuntimeConfig {
                player_owner: 0,
                player_name: "Presentation Host".to_owned(),
                network: None,
                record_enabled: false,
            },
            Some(vec![scenario.clone()]),
            crate::settings::CompatProfile::LegacyClonk,
        )?;
        // Keep TCP enabled while giving the test host an isolated local port.
        // Native's selected lobby state does not depend on the concrete local
        // transport address.
        let tcp_probe = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let tcp_port = tcp_probe.local_addr()?.port();
        drop(tcp_probe);
        app.classic_command_line.tcp_port = Some(tcp_port);
        app.classic_command_line.player_files = vec![player_path];
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while app.mode != AppMode::Menu {
            app.update()?;
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "capture fixture did not reach the startup menu"
            );
            thread::sleep(Duration::from_millis(1));
        }
        Ok((environment, user_data, app, scenario))
    }

    #[test]
    fn loader_checkpoint_stops_on_the_real_sixty_percent_frame() -> Result<()> {
        // Pinned C++ oracle: src/C4AulLink.cpp:299-303 emits the link summary,
        // src/C4Game.cpp:945,980-981 emits the three material/texture totals,
        // and src/C4Game.cpp:4094-4106 publishes the exact InitProgress value
        // rendered by src/C4LoaderScreen.cpp:281-324.
        let (_environment, _user_data, mut app, scenario) = capture_app()?;

        let checkpoint = stage_loader_checkpoint(&mut app, scenario)?;

        assert_eq!(checkpoint.render_ordinal, 2);
        assert_eq!(checkpoint.simulation_seed, 587);
        assert_eq!(checkpoint.random_count, 0);
        assert_eq!(app.mode, AppMode::Loading);
        let loading = app
            .loading_state
            .as_ref()
            .expect("the sixty-percent worker remains live");
        assert_eq!(loading.last_progress, 60);
        assert_eq!(
            loading.log[loading.log.len() - 4..],
            [
                "C4AulScriptEngine linked - 24442 lines, 0 warnings, 0 errors",
                "Texture table holds 48 entries.",
                "21 textures loaded.",
                "21 materials loaded.",
            ]
        );
        let loader = app
            .loader_screen
            .as_ref()
            .expect("the real scenario loader remains installed");
        assert_eq!(loader.state().progress(), 60);
        Ok(())
    }

    #[test]
    fn network_lobby_checkpoint_acknowledges_the_notice_over_the_live_host() -> Result<()> {
        // Pinned C++ oracle: src/C4Network2.cpp:451-485 admits and
        // acknowledges GS_Lobby before C4GameLobby.cpp:269-281 constructs the
        // native log surface that the second render presents.
        let (_environment, _user_data, mut app, scenario) = capture_app()?;

        let checkpoint = stage_network_lobby_checkpoint(&mut app, scenario)?;

        assert_eq!(checkpoint.render_ordinal, 2);
        assert_eq!(checkpoint.simulation_seed, 587);
        assert_eq!(checkpoint.random_count, 0);
        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup.view, crate::StartupView::NetworkLobby);
        assert!(matches!(
            app.network_mode,
            Some(crate::NetworkMode::Host(_))
        ));
        assert!(app.classic_host_lobby.is_some() || app.network_lobby.is_some());
        assert!(app.dialogs.messages.is_empty());
        assert!(app.status_text.is_empty());
        Ok(())
    }
}
