use crate::component_natives::{
    dead_fish_embowel_uses_the_trappers_custom_components,
    get_component_definition_branch_uses_custom_recipe_and_builder,
};
use crate::get_material_color::get_material_color_reads_earth_palette_and_system_rgb_wrapper;
use crate::load_scenario_section::gold_rush_do_change_section_loads_ash_city_landscape;
use crate::set_landscape_pixel::{
    set_landscape_pixel_accepts_rgb_and_only_changes_the_relative_surface32_pixel,
    shipped_volcano_draw_x_gradient_runs_through_set_landscape_pixel,
};
use crate::set_mat_adjust::{
    get_mat_adjust_tracks_default_and_same_call_set_value,
    gold_rush_global_fade_timer_reaches_its_completion_check,
    western_global_fade_restores_pre_fade_material_modulation,
};
use crate::set_material_color::set_material_color_matches_native_modulation_formula_and_invalid_gate;
use crate::support::real_scenario::{prepare_installed_scenario, PreparedInstalledScenario};

type GoldrushSubcase = (&'static str, fn(&PreparedInstalledScenario));

#[test]
fn goldrush_shared_scenario_subcases_batch_1() {
    run_goldrush_batch(&[
        (
            "global_fade_timer_reaches_its_completion_check",
            gold_rush_global_fade_timer_reaches_its_completion_check,
        ),
        (
            "get_material_color_reads_earth_palette_and_system_rgb_wrapper",
            get_material_color_reads_earth_palette_and_system_rgb_wrapper,
        ),
        (
            "set_material_color_matches_native_modulation_formula_and_invalid_gate",
            set_material_color_matches_native_modulation_formula_and_invalid_gate,
        ),
    ]);
}

#[test]
fn goldrush_shared_scenario_subcases_batch_2() {
    run_goldrush_batch(&[
        (
            "western_global_fade_restores_pre_fade_material_modulation",
            western_global_fade_restores_pre_fade_material_modulation,
        ),
        (
            "get_mat_adjust_tracks_default_and_same_call_set_value",
            get_mat_adjust_tracks_default_and_same_call_set_value,
        ),
        (
            "do_change_section_loads_ash_city_landscape",
            gold_rush_do_change_section_loads_ash_city_landscape,
        ),
        (
            "set_landscape_pixel_accepts_rgb_and_only_changes_the_relative_surface32_pixel",
            set_landscape_pixel_accepts_rgb_and_only_changes_the_relative_surface32_pixel,
        ),
    ]);
}

#[test]
fn goldrush_shared_scenario_subcases_batch_3() {
    run_goldrush_batch(&[
        (
            "volcano_draw_x_gradient_runs_through_set_landscape_pixel",
            shipped_volcano_draw_x_gradient_runs_through_set_landscape_pixel,
        ),
        (
            "get_component_definition_branch_uses_custom_recipe_and_builder",
            get_component_definition_branch_uses_custom_recipe_and_builder,
        ),
        (
            "dead_fish_embowel_uses_the_trappers_custom_components",
            dead_fish_embowel_uses_the_trappers_custom_components,
        ),
    ]);
}

fn run_goldrush_batch(subcases: &[GoldrushSubcase]) {
    let prepared = prepare_installed_scenario("Western.c4f/Goldrush.c4s", 0);
    let mut failures = Vec::new();

    for &(name, subcase) in subcases {
        eprintln!("running shared Goldrush subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subcase(&prepared))).is_err() {
            eprintln!("shared Goldrush subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} shared Goldrush subcase(s) failed: {}",
            failures.len(),
            failures.join(", ")
        );
    }
}
