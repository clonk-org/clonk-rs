//! C4MapCreator — the basic dynamic-map generator (src/C4Map.cpp).
//!
//! Scenarios without Map.bmp/Landscape.bmp/Landscape.txt build their 8-bit
//! map from the `[Landscape]` keys: a sine+noise surface line of the base
//! material, bottom fill, a liquid level, and random material layers
//! (C4Landscape::CreateMap, src/C4Landscape.cpp:512-528 →
//! C4MapCreator::Create, src/C4Map.cpp:73-167). All draws come from the
//! synced game RNG between the FixRandom brackets of C4Landscape::Init
//! (src/C4Landscape.cpp:578,734), so the map is a pure function of the
//! game seed, the scenario keys and the startup player count.

use crate::rng::LcgRng;
use crate::scenario::{LegacyC4SVal, MapPixelClassifier};

/// `C4S_MaxMapPlayerExtend` (src/C4Constants.h via C4Scenario.h:192).
const MAX_MAP_PLAYER_EXTEND: i32 = 4;
/// `C4S_MaxPlayer` (src/C4Constants.h:65).
const MAX_PLAYER: i32 = 4;
/// `MapIFT` (C4MapCreator::Reset, src/C4Map.cpp:37).
const MAP_IFT: u8 = 128;

/// The `[Landscape]` inputs of C4MapCreator::Create (C4SLandscape).
pub(crate) struct BasicMapParams {
    pub map_width: LegacyC4SVal,
    pub map_height: LegacyC4SVal,
    pub map_player_extend: bool,
    pub amplitude: LegacyC4SVal,
    pub phase: LegacyC4SVal,
    pub period: LegacyC4SVal,
    pub random: LegacyC4SVal,
    pub material: String,
    pub liquid: String,
    pub liquid_level: LegacyC4SVal,
    /// `Layers` C4NameList: (material name, count), file order.
    pub layers: Vec<(String, i32)>,
}

/// The map surface under construction (CSurface8 stand-in): row-major
/// texmap-index bytes, 0 = sky.
struct MapBuf {
    wdt: i32,
    hgt: i32,
    bytes: Vec<u8>,
    /// `Exclusive`: when >= 0, SetPix only replaces that byte
    /// (src/C4Map.cpp:39,47).
    exclusive: i32,
}

impl MapBuf {
    fn get_pix(&self, x: i32, y: i32) -> u8 {
        // Safety (src/C4Map.cpp:65-71): out of bounds reads 0.
        if x < 0 || y < 0 || x >= self.wdt || y >= self.hgt {
            return 0;
        }
        self.bytes[(y * self.wdt + x) as usize]
    }

    fn set_pix(&mut self, x: i32, y: i32, col: u8) {
        // Safety + Exclusive (src/C4Map.cpp:42-50).
        if x < 0 || y < 0 || x >= self.wdt || y >= self.hgt {
            return;
        }
        if self.exclusive > -1 && i32::from(self.get_pix(x, y)) != self.exclusive {
            return;
        }
        self.bytes[(y * self.wdt + x) as usize] = col;
    }

    /// DrawLayer (src/C4Map.cpp:52-63).
    fn draw_layer(&mut self, mut x: i32, mut y: i32, size: i32, col: u8, rng: &mut LcgRng) {
        for _ in 0..size {
            x += rng.random(9) - 4;
            y += rng.random(3) - 1;
            for cnt2 in rng.random(3)..5 {
                self.set_pix(x + cnt2, y, col);
                self.set_pix(x + cnt2 + 1, y + 1, col);
            }
        }
    }
}

/// C4SLandscape::GetMapSize (src/C4Scenario.cpp:327-334) with the Init-time
/// `MapWdt.Max = MapHgt.Max = 10000` widening (src/C4Landscape.cpp:568-569).
pub(crate) fn evaluate_map_size(
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_player_extend: bool,
    player_count: i32,
    rng: &mut LcgRng,
) -> (i32, i32) {
    let width_val = LegacyC4SVal::new(map_width.std, map_width.rnd, map_width.min, 10000);
    let height_val = LegacyC4SVal::new(map_height.std, map_height.rnd, map_height.min, 10000);
    let mut wdt = width_val.evaluate(rng);
    let hgt = height_val.evaluate(rng);
    let players = player_count.max(1);
    if map_player_extend {
        wdt = (wdt * players.min(MAX_MAP_PLAYER_EXTEND)).min(width_val.max);
    }
    (wdt, hgt)
}

/// Evaluate the `float` surface locals from `C4MapCreator::Create`
/// (src/C4Map.cpp:94-98), promoting them only after all native-precision
/// arithmetic is complete.
fn evaluate_surface_parameters(
    params: &BasicMapParams,
    player_num: i32,
    rng: &mut LcgRng,
) -> (f64, f64, f64, f64) {
    let amplitude = params.amplitude.evaluate(rng) as f32;
    let phase = params.phase.evaluate(rng) as f32;
    let mut period = params.period.evaluate(rng) as f32;
    if params.map_player_extend {
        period *= player_num.min(MAX_MAP_PLAYER_EXTEND) as f32;
    }
    let natural = params.random.evaluate(rng) as f32;
    (
        f64::from(amplitude),
        f64::from(phase),
        f64::from(period),
        f64::from(natural),
    )
}

/// C4MapCreator::Create (src/C4Map.cpp:73-167) with `fLayers = true`
/// (C4Landscape::CreateMap always passes true, src/C4Landscape.cpp:523).
/// Returns row-major map bytes.
pub(crate) fn create_basic_map(
    params: &BasicMapParams,
    classifier: &mut MapPixelClassifier,
    player_count: i32,
    rng: &mut LcgRng,
) -> clonk_resources::bitmap::IndexedBitmap {
    // C4Landscape::CreateMap sizes the surface first
    // (src/C4Landscape.cpp:518).
    let (map_wdt, map_hgt) = evaluate_map_size(
        params.map_width,
        params.map_height,
        params.map_player_extend,
        player_count,
        rng,
    );
    // CSurface8's constructor starts at 0x0 and Create returns before
    // assigning either dimension when either requested size is zero
    // (StdSurface8.cpp:33-47,90-95). Keep the existing negative-size
    // hardening, but preserve that native failed-surface state exactly.
    let (map_wdt, map_hgt) = if map_wdt == 0 || map_hgt == 0 {
        (0, 0)
    } else {
        (map_wdt.max(1), map_hgt.max(1))
    };
    let mut map = MapBuf {
        wdt: map_wdt,
        hgt: map_hgt,
        bytes: vec![0; (map_wdt * map_hgt) as usize],
        exclusive: -1,
    };

    let full_period = 20.0 * std::f64::consts::PI;
    let player_num = player_count.clamp(1, MAX_PLAYER);

    // Surface (src/C4Map.cpp:92-124).
    let ccol = classifier
        .get_index_mat_tex(&params.material, Some("Smooth"))
        .wrapping_add(MAP_IFT);
    let (amplitude, phase, period, natural) = evaluate_surface_parameters(params, player_num, rng);
    let level0 = map_wdt.min(map_hgt) / 2;
    let maxrange = level0 * 3 / 4;

    let mut rnd_cy = f64::from(rng.random(2000 + 1) - 1000) / 1000.0;
    let mut rnd_tend = f64::from(rng.random(200 + 1) - 100) / 20000.0;

    for cx in 0..map_wdt {
        rnd_cy += rnd_tend;
        rnd_tend += f64::from(rng.random(100 + 1) - 50) / 10000.0;
        rnd_tend = rnd_tend.clamp(-0.05, 0.05);
        if rnd_cy < -0.5 {
            rnd_tend += 0.01;
        }
        if rnd_cy > 0.5 {
            rnd_tend -= 0.01;
        }

        let cy_natural = rnd_cy * natural / 100.0;
        let cy_curve = (full_period * period / 100.0 * f64::from(cx) / f64::from(map_wdt)
            + 2.0 * std::f64::consts::PI * phase / 100.0)
            .sin()
            * amplitude
            / 100.0;

        let cy = level0
            + ((f64::from(maxrange) * (cy_curve + cy_natural)) as i32).clamp(-maxrange, maxrange);

        map.set_pix(cx, cy, ccol);
    }

    // Raise bottom to surface (src/C4Map.cpp:126-129).
    for cx in 0..map_wdt {
        let mut cy = map_hgt - 1;
        while cy >= 0 && map.get_pix(cx, cy) == 0 {
            map.set_pix(cx, cy, ccol);
            cy -= 1;
        }
    }

    // Raise liquid level (src/C4Map.cpp:130-137).
    map.exclusive = 0;
    let liquid_col = classifier.get_index_mat_tex(&params.liquid, Some("Smooth"));
    let wtr_level = params.liquid_level.evaluate(rng);
    for cx in 0..map_wdt {
        for cy in (map_hgt * (100 - wtr_level) / 100)..map_hgt {
            map.set_pix(cx, cy, liquid_col);
        }
    }
    map.exclusive = -1;

    // Layers (src/C4Map.cpp:139-166): only into the base material.
    map.exclusive = i32::from(
        classifier
            .get_index_mat_tex(&params.material, Some("Smooth"))
            .wrapping_add(MAP_IFT),
    );
    for (layer_name, layer_count) in params.layers.iter().take(10) {
        if layer_name.is_empty() {
            continue;
        }
        let ccol = classifier
            .get_index_mat_tex(layer_name, Some("Rough"))
            .wrapping_add(MAP_IFT);
        let layer_num = layer_count * map_wdt * map_hgt / 15000;
        for _ in 0..layer_num {
            let sptx = rng.random(map_wdt);
            let mut spty = 0;
            while spty < map_hgt && i32::from(map.get_pix(sptx, spty)) != map.exclusive {
                spty += 1;
            }
            spty += 5 + rng.random((map_hgt - spty) - 10);
            let size = rng.random(15);
            map.draw_layer(sptx, spty, size, ccol, rng);
        }
    }
    map.exclusive = -1;

    clonk_resources::bitmap::IndexedBitmap {
        width: map_wdt as u32,
        height: map_hgt as u32,
        indices: map.bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_classifier() -> MapPixelClassifier {
        let mut densities = [0i32; 128];
        let mut names: Vec<Option<String>> = vec![None; 128];
        let mut textures: Vec<Option<String>> = vec![None; 128];
        // TexMap slots the C++ scenario would carry: Earth-Smooth solid,
        // Water-Smooth liquid, Gold-Rough solid.
        densities[30] = 100;
        names[30] = Some("Earth".into());
        textures[30] = Some("Smooth".into());
        densities[20] = 25;
        names[20] = Some("Water".into());
        textures[20] = Some("Smooth".into());
        densities[40] = 100;
        names[40] = Some("Gold".into());
        textures[40] = Some("Rough".into());
        MapPixelClassifier::from_slots(densities, names, textures, vec![None; 128])
    }

    fn flat_params() -> BasicMapParams {
        BasicMapParams {
            // Min stays authoritative (only Max is widened at Init):
            // pick mins below the Std so the fixture size is exact.
            map_width: LegacyC4SVal::new(40, 0, 20, 250),
            map_height: LegacyC4SVal::new(30, 0, 20, 250),
            map_player_extend: false,
            amplitude: LegacyC4SVal::new(0, 0, 0, 100),
            phase: LegacyC4SVal::new(50, 0, 0, 100),
            period: LegacyC4SVal::new(15, 0, 0, 100),
            random: LegacyC4SVal::new(0, 0, 0, 100),
            material: "Earth".into(),
            liquid: "Water".into(),
            liquid_level: LegacyC4SVal::new(0, 0, 0, 100),
            layers: Vec::new(),
        }
    }

    #[test]
    fn basic_map_quantizes_surface_parameters_through_f32_like_cpp() {
        fn exact(value: i32) -> LegacyC4SVal {
            LegacyC4SVal::new(value, 0, i32::MIN, i32::MAX)
        }

        // C4MapCreator stores all four evaluated values in `float` locals
        // before their later promotion into the double surface formula.
        let mut params = flat_params();
        params.amplitude = exact(16_777_217);
        params.phase = exact(16_777_217);
        params.period = exact(16_777_217);
        params.random = exact(16_777_217);
        let mut rng = LcgRng::seed_from_u64(7);
        let (amplitude, phase, period, natural) = evaluate_surface_parameters(&params, 1, &mut rng);
        assert_eq!(amplitude, 16_777_216.0, "Amplitude narrows to float");
        assert_eq!(phase, 16_777_216.0, "Phase narrows to float");
        assert_eq!(period, 16_777_216.0, "Period narrows to float");
        assert_eq!(natural, 16_777_216.0, "Random narrows to float");

        // The quantized Phase is observable at a surface boundary, so the
        // production map path must render the adjacent integers identically.
        params.map_width = exact(100);
        params.map_height = exact(100);
        params.amplitude = exact(100);
        params.period = exact(0);
        params.random = exact(0);
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(7);
        let quantized = create_basic_map(&params, &mut classifier, 1, &mut rng);
        params.phase = exact(16_777_216);
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(7);
        let rounded = create_basic_map(&params, &mut classifier, 1, &mut rng);
        assert_eq!(quantized, rounded);

        // The compound MapPlayerExtend multiplication also happens while
        // Period is still a float: the exact product 50_331_645 rounds down.
        params.map_player_extend = true;
        params.period = exact(16_777_215);
        let mut rng = LcgRng::seed_from_u64(7);
        let (_, _, period, _) = evaluate_surface_parameters(&params, 3, &mut rng);
        assert_eq!(period, 50_331_644.0);
    }

    #[test]
    fn map_size_ignores_the_compile_time_max_like_init_widening() {
        // C4Landscape::Init raises MapWdt.Max/MapHgt.Max to 10000 BEFORE
        // CreateMap (src/C4Landscape.cpp:568-569), so a scenario MapWidth
        // beyond the C4SVal default max of 250 keeps its value.
        let mut rng = LcgRng::seed_from_u64(0);
        let (wdt, hgt) = evaluate_map_size(
            LegacyC4SVal::new(400, 0, 64, 250),
            LegacyC4SVal::new(300, 0, 40, 250),
            false,
            1,
            &mut rng,
        );
        assert_eq!((wdt, hgt), (400, 300));
    }

    #[test]
    fn map_player_extend_multiplies_width_by_the_clamped_player_count() {
        // GetMapSize (src/C4Scenario.cpp:331-333): wdt *= min(players,
        // C4S_MaxMapPlayerExtend), clamped to the widened Max.
        let mut rng = LcgRng::seed_from_u64(0);
        let (wdt, _) = evaluate_map_size(
            LegacyC4SVal::new(100, 0, 64, 250),
            LegacyC4SVal::new(50, 0, 40, 250),
            true,
            6,
            &mut rng,
        );
        assert_eq!(wdt, 400, "six players clamp to the 4x extend");
    }

    #[test]
    fn flat_map_fills_surface_material_below_the_level_line() {
        // Amplitude=0, Random=0: cy = level0 for every column
        // (src/C4Map.cpp:107-124), bottom raised to the surface
        // (src/C4Map.cpp:126-129). level0 = min(40,30)/2 = 15.
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(7);
        let map = create_basic_map(&flat_params(), &mut classifier, 1, &mut rng);
        assert_eq!((map.width, map.height), (40, 30));
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        for x in 0..40 {
            for y in 0..30u32 {
                let expected = if y >= 15 { 30 | 0x80 } else { 0 };
                assert_eq!(at(x, y), expected, "column {x} row {y}");
            }
        }
    }

    #[test]
    fn liquid_level_fills_sky_rows_only() {
        // Exclusive=0 while raising the liquid level (src/C4Map.cpp:
        // 130-137): water replaces sky below MapHgt*(100-level)/100,
        // never the surface material.
        let mut params = flat_params();
        params.liquid_level = LegacyC4SVal::new(60, 0, 0, 100);
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(7);
        let map = create_basic_map(&params, &mut classifier, 1, &mut rng);
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        // Liquid from row 30*(100-60)/100 = 12 downwards; earth from 15.
        assert_eq!(at(5, 11), 0, "above the liquid level stays sky");
        assert_eq!(at(5, 12), 20, "sky below the level becomes Water");
        assert_eq!(at(5, 14), 20);
        assert_eq!(at(5, 15), 30 | 0x80, "earth is not replaced");
    }

    #[test]
    fn create_draws_the_cpp_ledger_sequence() {
        // Draw order (src/C4Landscape.cpp:518 + src/C4Map.cpp:73-166):
        // MapWdt/MapHgt evaluates, Amplitude, Phase, Period, Random,
        // rnd_cy Random(2001), rnd_tend Random(201), one Random(101) per
        // column, LiquidLevel — layer-less: 9 + MapWdt draws.
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(7);
        let base = rng.count; // FixRandom leaves Randomize3's 500 draws
        create_basic_map(&flat_params(), &mut classifier, 1, &mut rng);
        assert_eq!(rng.count - base, 9 + 40);
    }

    #[test]
    fn basic_map_zero_dimension_preserves_native_empty_surface_and_rng_ledger() {
        let advance_native_empty_ledger = |params: &BasicMapParams, rng: &mut LcgRng| {
            LegacyC4SVal::new(
                params.map_width.std,
                params.map_width.rnd,
                params.map_width.min,
                10000,
            )
            .evaluate(rng);
            LegacyC4SVal::new(
                params.map_height.std,
                params.map_height.rnd,
                params.map_height.min,
                10000,
            )
            .evaluate(rng);
            params.amplitude.evaluate(rng);
            params.phase.evaluate(rng);
            params.period.evaluate(rng);
            params.random.evaluate(rng);
            rng.random(2001);
            rng.random(201);
            params.liquid_level.evaluate(rng);
        };

        for (requested_width, requested_height) in [(0, 7), (7, 0), (0, 0)] {
            let mut params = flat_params();
            params.map_width = LegacyC4SVal::new(requested_width, 0, 0, 250);
            params.map_height = LegacyC4SVal::new(requested_height, 0, 0, 250);
            params.map_player_extend = true;
            params.amplitude = LegacyC4SVal::new(20, 3, 0, 100);
            params.phase = LegacyC4SVal::new(40, 4, 0, 100);
            params.period = LegacyC4SVal::new(10, 2, 0, 100);
            params.random = LegacyC4SVal::new(30, 5, 0, 100);
            params.liquid_level = LegacyC4SVal::new(25, 6, 0, 100);
            params.layers = vec![("Gold".into(), 20)];

            let mut expected_rng = LcgRng::seed_from_u64(73);
            let base_count = expected_rng.count;
            advance_native_empty_ledger(&params, &mut expected_rng);

            let mut classifier = test_classifier();
            let mut rng = LcgRng::seed_from_u64(73);
            let map = create_basic_map(&params, &mut classifier, 3, &mut rng);
            assert_eq!(
                (map.width, map.height),
                (0, 0),
                "requested {requested_width}x{requested_height}"
            );
            assert!(map.indices.is_empty());
            assert_eq!(rng.count - base_count, 9, "no forced-column draws");
            assert_eq!(rng, expected_rng, "exact C4SVal/noise draw order");

            let landscape = crate::scenario::classified_landscape(&map, &classifier, 10, 0)
                .expect("the empty source map still builds the minimum world");
            let grid = landscape
                .pixel_grid()
                .expect("dynamic maps retain a Surface8 plane");
            assert_eq!((grid.width(), grid.height()), (100, 100));
            assert!(grid.bytes().iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn layers_scatter_material_into_the_base_only() {
        // Layers (src/C4Map.cpp:139-166): count*W*H/15000 spots of the
        // layer material drawn with Exclusive = surface color — layer
        // pixels only ever replace Earth.
        let mut params = flat_params();
        params.layers = vec![("Gold".into(), 20)];
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(7);
        let map = create_basic_map(&params, &mut classifier, 1, &mut rng);
        let gold = 40u8 | 0x80;
        let earth = 30u8 | 0x80;
        let mut gold_count = 0;
        for (index, &byte) in map.indices.iter().enumerate() {
            let y = index as u32 / map.width;
            assert!(
                byte == 0 || byte == earth || byte == gold,
                "only sky/earth/gold appear"
            );
            if byte == gold {
                gold_count += 1;
                assert!(y >= 15, "gold only below the surface line");
            }
        }
        assert!(gold_count > 0, "layer spots drawn (20*40*30/15000 = 1)");
    }
}
