use lc_app::game::{DemoGame, GameResult, GameSummary};

fn main() {
    if let Err(error) = run() {
        eprintln!("lc-app: {error}");
        std::process::exit(1);
    }
}

fn run() -> GameResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut game = DemoGame::new()?;
    let ticks = parse_ticks(&args).unwrap_or_else(|| game.configured_ticks());
    let summary = game.run(ticks)?;
    report(&summary);
    Ok(())
}

fn parse_ticks(args: &[String]) -> Option<u32> {
    for arg in args.iter().skip(1) {
        if let Some(value) = arg.strip_prefix("--ticks=") {
            if let Ok(parsed) = value.parse::<u32>() {
                return Some(parsed);
            }
            continue;
        }
        if arg.chars().all(|ch| ch.is_ascii_digit()) {
            if let Ok(parsed) = arg.parse::<u32>() {
                return Some(parsed);
            }
        }
    }
    None
}

fn report(summary: &GameSummary) {
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
