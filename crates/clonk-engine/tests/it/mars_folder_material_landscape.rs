//! `ClonkMars.c4f/Material.c4g` carries the `Earth`/`Rock2` textures that
//! `01_Fossae.c4s`'s Landscape.txt names, and the installed `Material.c4g`
//! does not. C4GameParameters publishes every parent folder's Material.c4g as
//! NRT_Material ahead of the installed one, so C4MapCreatorS2 resolves those
//! `tex=` names and paints a solid map; without the folder in the chain the
//! parse stops at the first overlay and the world comes out empty
//! (C4GameParameters.cpp:214-222; C4Game.cpp:901-977;
//! C4MapCreatorS2.cpp:341-350).

use crate::support::real_scenario::load_installed_scenario;

#[test]
fn fossae_scenario_folder_textures_fill_its_generated_landscape() {
    let engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let earth = crate::support::TestValueExt::test_value(engine.materials().id_of("Earth"));
    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
    let width = crate::support::TestValueExt::test_value(i32::try_from(landscape.width()));
    let height = landscape.estimated_height();

    let (solid, earth_pixels) = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .fold((0usize, 0usize), |(solid, earth_pixels), (x, y)| {
            (
                solid + usize::from(landscape.is_solid_at(x, y)),
                earth_pixels + usize::from(landscape.material_at(x, y) == Some(earth)),
            )
        });

    assert!(
        solid > 0 && earth_pixels > 0,
        "Fossae's `mat=Earth;tex=Earth` base overlay must paint the map; \
         {width}x{height} landscape has solid={solid} earth={earth_pixels}"
    );
}
