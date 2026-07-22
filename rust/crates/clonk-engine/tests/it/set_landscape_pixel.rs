use clonk_engine::{Definition, SpawnConfig, Vector2};
use clonk_script::Value;

use crate::support::real_scenario::PreparedInstalledScenario;

const PIXEL_COLOR: u32 = 0x0011_2233;

pub(super) fn set_landscape_pixel_accepts_rgb_and_only_changes_the_relative_surface32_pixel(
    prepared: &PreparedInstalledScenario,
) {
    // The installed System.c4g layer supplies RGB, matching the shipped
    // Volcano and MapScreen callers rather than a test-only packing helper.
    let mut engine = prepared.instantiate();
    engine
        .register_definition(
            Definition::from_script(
                "LSPX",
                "SetLandscapePixel probe",
                r#"#strict
local iBefore, iAfter;

public func Paint()
{
    iBefore = 1;
    SetLandscapePixel(3, 4, RGB(17, 34, 51));
    iAfter = 2;
    return(iAfter);
}
"#,
            )
            .expect("the SetLandscapePixel probe compiles"),
        )
        .expect("the SetLandscapePixel probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("LSPX").with_position(Vector2::new(40, 40)))
        .expect("the SetLandscapePixel probe spawns");
    let probe_position = engine
        .object_snapshot(probe)
        .expect("the probe remains active")
        .position;
    let target = Vector2::new(probe_position.x + 3, probe_position.y + 4);
    let index = engine.find_object_index(probe).expect("the probe exists");

    let (before_byte, before_material, before_grid_revision, before_unoffset_color) = {
        let landscape = engine.landscape().expect("Goldrush has a landscape");
        let grid = landscape
            .pixel_grid()
            .expect("Goldrush has an authoritative Surface8 grid");
        (
            landscape.grid_byte_at(target.x, target.y),
            landscape.material_at(target.x, target.y),
            grid.revision(),
            landscape.surface32_pixel_at(3, 4),
        )
    };

    assert_eq!(
        engine
            .call_object_function(index, "Paint", Vec::new())
            .expect("SetLandscapePixel(3, 4, RGB(17, 34, 51)) executes"),
        Value::Int(2)
    );

    let landscape = engine
        .landscape()
        .expect("the probe landscape remains installed");
    assert_eq!(
        landscape.surface32_pixel_at(target.x, target.y),
        Some(PIXEL_COLOR),
        "SetLandscapePixel offsets by the calling object's live position"
    );
    assert_eq!(
        landscape.surface32_pixel_at(3, 4),
        before_unoffset_color,
        "the unoffset script coordinates are not the write target"
    );
    assert_eq!(landscape.grid_byte_at(target.x, target.y), before_byte);
    assert_eq!(
        landscape.material_at(target.x, target.y),
        before_material,
        "the Surface32-only write must not change the material map"
    );
    assert_eq!(
        landscape
            .pixel_grid()
            .expect("Surface8 remains installed")
            .revision(),
        before_grid_revision,
        "the Surface32-only write must not dirty Surface8"
    );
    let locals = &engine
        .object_snapshot(probe)
        .expect("the probe remains active")
        .local_vars;
    assert_eq!(locals.get("iBefore"), Some(&Value::Int(1)));
    assert_eq!(
        locals.get("iAfter"),
        Some(&Value::Int(2)),
        "SetLandscapePixel must not abort the calling script"
    );
}

pub(super) fn shipped_volcano_draw_x_gradient_runs_through_set_landscape_pixel(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert_eq!(
        engine.debug_definition_has_function("FXV1", "DrawXGradient"),
        Some(true),
        "Goldrush loads the shipped Objects.c4d Volcano definition"
    );
    let volcano = engine
        .spawn_object(SpawnConfig::new("FXV1").with_position(Vector2::new(80, 80)))
        .expect("the shipped Volcano object spawns");
    let volcano_position = engine
        .object_snapshot(volcano)
        .expect("the shipped Volcano remains active")
        .position;
    // DrawXGradient changes dir=1 to +1, then its one loop iteration writes
    // at local (1, 0). max=0 keeps the supplied color unchanged through the
    // shipped DarkenRGB/LightenRGB helpers.
    let target = Vector2::new(volcano_position.x + 1, volcano_position.y);
    let (before_byte, before_material, before_grid_revision) = {
        let landscape = engine.landscape().expect("Goldrush has a landscape");
        (
            landscape.grid_byte_at(target.x, target.y),
            landscape.material_at(target.x, target.y),
            landscape
                .pixel_grid()
                .expect("Goldrush has an authoritative Surface8 grid")
                .revision(),
        )
    };
    let index = engine
        .find_object_index(volcano)
        .expect("the shipped Volcano exists");

    assert_eq!(
        engine
            .call_object_function(
                index,
                "DrawXGradient",
                vec![
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(PIXEL_COLOR as i32),
                    Value::Int(1),
                    Value::Int(0),
                    Value::Int(1),
                ],
            )
            .expect("the shipped Volcano gradient has no unknown-function error"),
        Value::Nil
    );

    let landscape = engine
        .landscape()
        .expect("the Volcano landscape remains installed");
    assert_eq!(
        landscape.surface32_pixel_at(target.x, target.y),
        Some(PIXEL_COLOR),
        "the real Volcano helper reaches SetLandscapePixel"
    );
    assert_eq!(landscape.grid_byte_at(target.x, target.y), before_byte);
    assert_eq!(landscape.material_at(target.x, target.y), before_material);
    assert_eq!(
        landscape
            .pixel_grid()
            .expect("Surface8 remains installed")
            .revision(),
        before_grid_revision
    );
}
