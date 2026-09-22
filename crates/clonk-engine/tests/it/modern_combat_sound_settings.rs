//! Modern Combat's Sound Setting (`ModernCombat.c4d/Environment.c4d/
//! SoundEffects.c4d`) keeps `local ... global` and assigns it in `Set`. C4Aul
//! resolves a declared value before it considers `global` an access directive
//! (C4AulParse.cpp:1976-2011 precede 2168-2171), so that is an ordinary
//! assignment. clonk-rs took it for the start of a declaration and failed to
//! parse `Set`, and CMC_Train's `Initialize`, which places its sound settings
//! in `CreateInterior`, stopped there, before its equipment.

use crate::support::real_scenario::load_installed_scenario;

#[test]
fn the_cmc_train_initializes_past_its_sound_settings() {
    let engine = load_installed_scenario("Collection.c4f/ModernCombat.c4f/CMC_Train.c4s", 0);

    let machines = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "WPVM")
        .count();
    assert!(machines > 0, "CreateEquipment placed the vending machine");
}
