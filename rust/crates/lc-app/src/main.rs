use std::fs::File;
use std::path::{Path, PathBuf};

use clap::Parser;
use lc_app::game::{DemoGame, DemoGameOptions, GameError, GameResult, GameSummary};

#[derive(Debug, Parser)]
#[command(
    name = "lc-app",
    author,
    version,
    about = "LegacyClonk Rust demo runner",
    long_about = None
)]
struct Cli {
    /// Number of simulation ticks to run
    #[arg(short = 't', long = "ticks")]
    ticks: Option<u32>,

    /// Override the bundled demo configuration with a file on disk
    #[arg(long = "config", value_name = "PATH")]
    config: Option<PathBuf>,

    /// Load a scenario directory or packed group for the demo
    #[arg(long = "scenario", value_name = "PATH")]
    scenario: Option<PathBuf>,

    /// Write the run summary as JSON to the given path
    #[arg(long = "summary-json", value_name = "PATH")]
    summary_json: Option<PathBuf>,

    /// Suppress the human-readable summary on stdout
    #[arg(long = "quiet")]
    quiet: bool,

    /// Enable live keyboard controls during the demo run
    #[arg(long = "interactive")]
    interactive: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("lc-app: {error}");
        std::process::exit(1);
    }
}

fn run() -> GameResult<()> {
    let cli = Cli::parse();
    run_with_cli(cli)
}

fn run_with_cli(cli: Cli) -> GameResult<()> {
    let mut game = DemoGame::new(DemoGameOptions {
        config_path: cli.config.clone(),
        scenario_path: cli.scenario.clone(),
        interactive: cli.interactive,
    })?;
    let ticks = cli.ticks.unwrap_or_else(|| game.configured_ticks());
    let summary = game.run(ticks)?;

    if let Some(path) = cli.summary_json.as_deref() {
        write_summary_json(path, &summary)?;
    }

    if !cli.quiet {
        print_summary(&summary);
    }

    Ok(())
}

fn write_summary_json(path: &Path, summary: &GameSummary) -> GameResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(GameError::SummaryOutput)?;
        }
    }
    let file = File::create(path).map_err(GameError::SummaryOutput)?;
    serde_json::to_writer_pretty(file, summary).map_err(GameError::SummarySerialize)?;
    Ok(())
}

fn print_summary(summary: &GameSummary) {
    println!("LegacyClonk Rust demo complete.");
    println!("Scenario: {}", summary.scenario_name);
    println!(
        "System version: {} ({} entries)",
        summary.system_version, summary.system_entry_count
    );
    println!("Install root: {}", summary.install_root.display());
    println!("User data dir: {}", summary.user_data_dir.display());
    println!("Logs dir: {}", summary.logs_dir.display());
    println!("Cache dir: {}", summary.cache_dir.display());
    println!(
        "Physics: gravity {} max_fall {} max_rise {} max_horizontal {}",
        summary.gravity,
        summary.max_fall_speed,
        summary.max_rise_speed,
        summary.max_horizontal_speed
    );
    println!(
        "Environment: wind {} (base {} +/- {} period {} temperature {} time {} speed {})",
        summary.environment.current_wind,
        summary.environment.base_wind,
        summary.environment.wind_variation,
        summary.environment.wind_period,
        summary.environment.temperature,
        summary.environment.time_of_day,
        summary.environment.time_speed
    );
    println!("Frames simulated: {}", summary.ticks);
    println!("Ground contacts: {}", summary.ground_hits);
    println!("Control ready batches: {}", summary.ready_batches);
    println!("Final engine frame: {}", summary.final_snapshot.frame);
    if let Some(object) = summary.final_snapshot.objects.first() {
        println!(
            "Final object {} pos ({}, {}) vel ({}, {}) energy {}",
            object.definition_id,
            object.position.x,
            object.position.y,
            object.velocity.x,
            object.velocity.y,
            object.energy
        );
    } else {
        println!("Final snapshot did not contain any objects.");
    }
    println!("Surface hash: 0x{:016x}", summary.surface_hash);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn cli_accepts_tick_override() {
        let cli = Cli::parse_from(["lc-app", "--ticks", "42", "--quiet"]);
        assert_eq!(cli.ticks, Some(42));
        assert!(cli.quiet);
        assert!(cli.scenario.is_none());
        assert!(!cli.interactive);
    }

    #[test]
    fn cli_accepts_scenario_path() {
        let cli = Cli::parse_from(["lc-app", "--scenario", "test_scenario", "--quiet"]);
        assert_eq!(cli.scenario, Some(PathBuf::from("test_scenario")));
        assert!(cli.quiet);
        assert!(!cli.interactive);
    }

    #[test]
    fn run_with_summary_json_writes_file() {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        path.push(format!("lc_app_summary_{nanos}.json"));

        let cli = Cli {
            ticks: Some(4),
            config: None,
            scenario: None,
            summary_json: Some(path.clone()),
            quiet: true,
            interactive: false,
        };

        run_with_cli(cli).expect("cli run succeeds");

        let data = std::fs::read_to_string(&path).expect("summary file readable");
        assert!(data.contains("\"ticks\": 4"));

        let _ = std::fs::remove_file(path);
    }
}
