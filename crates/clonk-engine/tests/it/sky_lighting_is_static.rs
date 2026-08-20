//! Whether shipped content ever animates the sky lighting factor.
//!
//! `GraphicsSystem::retained_lit_sky_texture` caches its lit plane on
//! `(source identity, lighting bits)`, so a *changing* lighting value is what
//! makes it rebuild and re-upload the whole sky. clonk-org/clonk-rs#287 gates
//! moving that transform to the GPU on "a measured win for realistic animated
//! lighting; otherwise retain the current cache and close with evidence".
//!
//! The lighting factor comes from `lighting_factor(environment.time_of_day)`,
//! and `time_of_day` advances only in `advance_time_of_day`, which returns
//! immediately when `time_speed == 0`.

use crate::support::real_scenario::load_installed_scenario;

/// No shipped scenario runs the engine clock, so its sky lighting factor is a
/// constant and the lit-sky cache never misses after the first frame.
///
/// This is a behavioural check rather than a scan of `Scenario.txt` for
/// `Time=`/`TimeSpeed`, because the clock has two independent-looking drivers
/// and only one of them is the engine's. `Objects.c4d/Environment.c4d/Time.c4d`
/// and ClonkMars's `TIME.c` both define a script function *named* `SetTime` and
/// ClonkMars scenarios really do place `TIME` objects — but that is the object's
/// own function driving `RestoreSkyColors`, a script-side sky *colour*
/// modulation. It never reaches `EnvironmentSettings::time_speed`, and the
/// engine registers no `SetTime` host function for it to reach through.
///
/// Reading the scenario files alone would miss that distinction in one
/// direction and reading the content scripts alone would miss it in the other,
/// so assert the engine state the lighting factor actually depends on.
#[test]
fn shipped_scenarios_do_not_animate_the_sky_lighting_factor() {
    for path in [
        "Tutorial.c4f/Tutorial01.c4s",
        "ClonkMars.c4f/03_Chaos.c4s",
        "Hazard.c4f/Tutorial.c4s",
        "Knights.c4f/Camp.c4s",
    ] {
        let engine = load_installed_scenario(path, 0);
        let settings = engine.snapshot().environment.settings;
        assert_eq!(
            settings.time_speed(),
            0,
            "`{path}` runs the engine clock, so its sky lighting animates and \
             clonk-org/clonk-rs#287's premise holds after all",
        );
        assert_eq!(
            settings.time_of_day(),
            0,
            "`{path}` starts at a non-noon time, so its lighting factor is not 1.0",
        );
    }
}
