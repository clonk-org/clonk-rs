use clonk_engine::SimulationSnapshot;
use clonk_frontend::{CursorAtlas, DefinitionSprite, GraphicsSystem, HudGraphics, ViewportInput};
use clonk_graphics::BitmapFont;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const CACHED_SAMPLES: usize = 10;

#[test]
#[ignore = "developer feedback timing probe; invoked by cargo dev-check"]
fn dev_feedback_render() -> Result<(), Box<dyn Error>> {
    let snapshot_path = snapshot_path()?;
    let snapshot: SimulationSnapshot = serde_json::from_reader(fs::File::open(&snapshot_path)?)?;
    let frame_path = output_path("LC_DEV_CHECK_FRAME_PNG", &snapshot_path, "frame-final.png");
    let metrics_path = output_path(
        "LC_DEV_CHECK_RENDER_METRICS",
        &snapshot_path,
        "render-metrics.json",
    );

    ensure_parent(&frame_path)?;
    ensure_parent(&metrics_path)?;

    let sprites: Arc<HashMap<String, DefinitionSprite>> = Arc::new(HashMap::new());
    let mut graphics = GraphicsSystem::new(
        WIDTH,
        HEIGHT,
        HEIGHT as i32 * 3 / 4,
        "Developer feedback render",
        Arc::new(BitmapFont::new()),
        sprites,
        Arc::new(CursorAtlas::empty()),
        Arc::new(HudGraphics::default()),
    );
    let viewports = snapshot
        .objects
        .first()
        .map(|focus| vec![ViewportInput::from_focus(focus)])
        .unwrap_or_default();

    let cold_started = Instant::now();
    graphics.render_frame(&snapshot, &viewports);
    let cold_render = cold_started.elapsed();
    let cold_checksum = graphics.surface().snapshot().checksum();

    // The first repeat settles any renderer lifecycle state (camera/gamma).
    // Every timed cached repeat after that must produce the same pixels.
    graphics.render_frame(&snapshot, &viewports);
    let repeat_checksum = graphics.surface().snapshot().checksum();
    let mut cached_renders = Vec::with_capacity(CACHED_SAMPLES);
    for _ in 0..CACHED_SAMPLES {
        let started = Instant::now();
        graphics.render_frame(&snapshot, &viewports);
        cached_renders.push(started.elapsed());
        assert_eq!(
            graphics.surface().snapshot().checksum(),
            repeat_checksum,
            "repeated rendering of one simulation snapshot must be pixel-deterministic"
        );
    }

    image::save_buffer_with_format(
        &frame_path,
        graphics.surface().pixels(),
        WIDTH,
        HEIGHT,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;

    let cached_render_ns: Vec<u64> = cached_renders.iter().copied().map(duration_ns).collect();
    let metrics = serde_json::json!({
        "schema_version": 1,
        "snapshot": snapshot_path,
        "width": WIDTH,
        "height": HEIGHT,
        "cold_render_ns": duration_ns(cold_render),
        "cold_checksum": format!("{cold_checksum:08x}"),
        "cached_render_ns": cached_render_ns,
        "cached_checksum": format!("{repeat_checksum:08x}"),
        "cached_samples": CACHED_SAMPLES,
    });
    fs::write(metrics_path, serde_json::to_vec_pretty(&metrics)?)?;
    Ok(())
}

fn snapshot_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = env::var_os("LC_DEV_CHECK_SNAPSHOT") {
        return Ok(PathBuf::from(path));
    }

    let mut candidates = Vec::new();
    for variable in ["LC_DEV_CHECK_ARTIFACT_DIR", "LC_TEST_ARTIFACT_DIR"] {
        if let Some(root) = env::var_os(variable) {
            let root = Path::new(&root);
            if root.is_dir() {
                collect_snapshot_candidates(root, &mut candidates)?;
            }
        }
    }
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    candidates
        .into_iter()
        .map(|(path, _)| path)
        .next()
        .ok_or_else(|| {
            "set LC_DEV_CHECK_SNAPSHOT or place snapshot-final.json under \
             LC_DEV_CHECK_ARTIFACT_DIR/LC_TEST_ARTIFACT_DIR"
                .into()
        })
}

fn collect_snapshot_candidates(
    directory: &Path,
    candidates: &mut Vec<(PathBuf, SystemTime)>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_snapshot_candidates(&path, candidates)?;
        } else if path
            .file_name()
            .is_some_and(|name| name == "snapshot-final.json")
        {
            let modified = entry
                .metadata()?
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            candidates.push((path, modified));
        }
    }
    Ok(())
}

fn output_path(variable: &str, snapshot: &Path, default_name: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| snapshot.with_file_name(default_name))
}

fn ensure_parent(path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().try_into().unwrap_or(u64::MAX)
}
