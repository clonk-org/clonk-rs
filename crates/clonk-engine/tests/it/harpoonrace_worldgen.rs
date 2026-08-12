use crate::support::real_scenario::prepare_installed_scenario;
use clonk_engine::Engine;

const HARPOON_RACE: &str = "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s";
const SKY_RACE: &str = "Races.c4f/Skyrace.c4s";

#[test]
fn generated_harpoonrace_detects_water_exposed_by_sky_overlays() {
    // C4MapCreatorS2.cpp:448-553 renders the scenario's final operator chain
    // exactly; this seed leaves Water movable at load time in both engines.
    let prepared = prepare_installed_scenario(HARPOON_RACE, 1_784_903_470);
    assert!(
        prepared.generated_landscape_requires_seed_retry(),
        "known-bad SkyParcour seeds must be rejected before publication"
    );
    let engine = prepared.instantiate();
    let (_, movable_water) = water_pixels_after_activation(&engine);
    assert_eq!(
        movable_water.len(),
        88,
        "the Rust Surface8 must retain the 88 movable pixels observed in the C++ oracle before the authority rejects this seed"
    );
}

#[test]
fn generated_harpoonrace_accepts_adjacent_contained_water_seed() {
    let prepared = prepare_installed_scenario(HARPOON_RACE, 1_784_903_471);
    assert!(
        !prepared.generated_landscape_requires_seed_retry(),
        "contained SkyParcour Water must not trigger another generation"
    );
}

#[test]
fn accepted_harpoonrace_seed_stays_contained_after_final_s2_rerender() {
    let prepared = prepare_installed_scenario(HARPOON_RACE, 1_784_903_471);
    assert!(
        !prepared.generated_landscape_requires_seed_retry(),
        "the eager authoritative preview must accept the adjacent seed"
    );

    // Scenario::apply crosses the final C4Game::InitGame boundary: scripts
    // have linked, MapCreatorS2 rerenders onto the live Engine landscape, and
    // post-init map callbacks plus ScenarioInit have had their opportunity to
    // alter it. Validate that final activated Surface8, not just the preview.
    let engine = prepared.instantiate();
    let (water_pixels, movable_water) = water_pixels_after_activation(&engine);
    assert!(
        water_pixels > 0,
        "final rerender must retain the authored contained Water"
    );
    assert!(
        movable_water.is_empty(),
        "accepted seed exposed movable Water after final rerender/activation at {movable_water:?}"
    );
}

#[test]
fn generated_skyrace_uses_the_same_skyparcour_guard() {
    let prepared = prepare_installed_scenario(SKY_RACE, 1_784_903_470);
    assert!(
        prepared.generated_landscape_requires_seed_retry(),
        "the original Sky Race copy of SkyParcour needs the same guard"
    );
}

fn water_pixels_after_activation(engine: &Engine) -> (usize, Vec<(i32, i32)>) {
    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
    let grid = crate::support::TestValueExt::test_value(landscape.pixel_grid());
    let materials = engine.materials();
    let water = crate::support::TestValueExt::test_value(
        materials.get("Water").filter(|water| water.instable()),
    );
    let width = crate::support::TestValueExt::test_value(i32::try_from(grid.width()));
    let height = crate::support::TestValueExt::test_value(i32::try_from(grid.height()));
    let mut water_pixels = 0;
    let mut movable = Vec::new();

    // HarpoonRace deliberately uses BottomOpen=1. Like the production guard,
    // exclude that authored final row and detect only an interior path that
    // C4MassMover can take immediately.
    for y in 0..height.saturating_sub(1) {
        for x in 0..width {
            if landscape.material_at(x, y) != Some(water.id()) {
                continue;
            }
            water_pixels += 1;
            let mut target_x = x;
            let mut target_y = y;
            if landscape.find_mat_path(
                &mut target_x,
                &mut target_y,
                1,
                water.density(),
                water.max_slide(),
                materials,
            ) {
                movable.push((x, y));
            }
        }
    }
    (water_pixels, movable)
}
