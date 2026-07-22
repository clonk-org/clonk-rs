use clonk_engine::ActionProcedure;

#[test]
fn maps_walkto_to_walk_procedure() {
    assert_eq!(ActionProcedure::from_name("WalkTo"), ActionProcedure::Walk);
}

#[test]
fn maps_dive_to_swim_procedure() {
    assert_eq!(ActionProcedure::from_name("Dive"), ActionProcedure::Swim);
}

#[test]
fn maps_tumble_and_dead_to_flight() {
    assert_eq!(
        ActionProcedure::from_name("Tumble"),
        ActionProcedure::Flight
    );
    assert_eq!(ActionProcedure::from_name("Dead"), ActionProcedure::Flight);
    assert_eq!(ActionProcedure::from_name("Dead2"), ActionProcedure::Flight);
}
