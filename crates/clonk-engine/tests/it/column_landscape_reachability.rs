//! Which landscapes can reach the column-only rendering fallback.
//!
//! `GraphicsSystem::draw_ground` tries the Surface8 material-texture path
//! first and only falls back to `capture_column_landscape_fallback` when
//! `Landscape::pixel_grid()` is `None`. That fallback allocates two
//! viewport-sized RGBA planes, clears and fills both every frame, and can
//! upload them — 63.3 MiB per plane at 4K, from a source that is one ground
//! height plus a short liquid-segment list per world column.
//!
//! clonk-org/clonk-rs#285 asks, before any procedural renderer is written, for
//! an inventory of what actually reaches that path. This is the shipped-content
//! half of it: a pixel grid means Surface8 handles the landscape and the
//! fallback is never entered.

use crate::support::real_scenario::load_installed_scenario;

/// Shipped scenarios all carry a pixel grid, so none of them renders through
/// the column-only fallback.
///
/// One per content pack rather than one per scenario: the grid comes from how
/// a pack's landscape is authored (`Landscape.png`/`Map.bmp` plus a material
/// map), so a pack is the unit that could differ. `ClonkMars.c4f/03_Chaos.c4s`
/// was checked the same way and also carries one; it is left out of the
/// committed test because its activation alone runs about ten seconds.
///
/// If this ever fails, the inventory in clonk-org/clonk-rs#285 is stale and the
/// fallback's cost has become reachable from real gameplay — which is the
/// condition that issue's go/no-go turns on.
#[test]
fn shipped_scenarios_never_reach_the_column_landscape_fallback() {
    for path in [
        "Tutorial.c4f/Tutorial01.c4s",
        "Hazard.c4f/Tutorial.c4s",
        "Knights.c4f/Camp.c4s",
        "Races.c4f/Skyrace.c4s",
    ] {
        let engine = load_installed_scenario(path, 0);
        let landscape = engine
            .landscape()
            .unwrap_or_else(|| panic!("`{path}` loads a landscape"));
        assert!(
            landscape.pixel_grid().is_some(),
            "`{path}` has no pixel grid, so it would render through the \
             column-only fallback that clonk-org/clonk-rs#285 measures",
        );
    }
}
