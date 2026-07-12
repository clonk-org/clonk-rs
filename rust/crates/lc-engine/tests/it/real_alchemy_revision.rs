use crate::support::real_scenario::{join_local_player, load_installed_scenario};

#[test]
fn alchemy_idle_window_keeps_full_raster_cache_revision_stable() {
    // The frontend rebuilds its complete width*height*4 landscape cache
    // whenever PixelGrid::revision changes. A deterministic 1,200-frame
    // probe of installed Alchemy changed it on zero frames; keep a shorter
    // window here to catch accidental per-frame/no-op invalidation without
    // making the integration suite pay for the full diagnostic run.
    const TICKS: usize = 120;

    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    join_local_player(&mut engine, "Alchemy raster revision probe");
    let initial_grid = engine
        .landscape()
        .and_then(lc_engine::Landscape::pixel_grid)
        .expect("Alchemy has its generated raster landscape");
    assert_eq!(
        (initial_grid.width(), initial_grid.height()),
        (1_488, 1_536)
    );
    let initial_revision = initial_grid.revision();
    let mut previous_revision = initial_revision;
    let mut changed_frames = 0;

    for _ in 0..TICKS {
        let snapshot = engine.tick().expect("Alchemy idle frame executes");
        let revision = snapshot
            .landscape
            .as_ref()
            .and_then(lc_engine::Landscape::pixel_grid)
            .expect("Alchemy snapshot retains its raster landscape")
            .revision();
        changed_frames += usize::from(revision != previous_revision);
        previous_revision = revision;
    }

    assert_eq!(
        changed_frames, 0,
        "idle Alchemy must not rebuild its 8.72 MiB RGBA landscape cache"
    );
    assert_eq!(previous_revision, initial_revision);
}
