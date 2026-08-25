use super::*;

#[test]
fn discarding_the_map_creator_spends_the_two_template_default_draws() {
    // ~C4MapCreatorS2 is not just deallocation. It runs Clear(), which calls
    // Default(), which re-defaults the template map -- and C4MCMap::Default
    // evaluates MapWdt and MapHgt (oracle-src-pinned
    // src/C4MapCreatorS2.cpp:633-644,717-740). Both are ordinary
    // C4SVal::Evaluate calls, so each spends one draw even for a fixed value
    // (src/C4Scenario.cpp:43-46), and C4Landscape::PostInitMap performs that
    // delete on the live synced ledger (src/C4Landscape.cpp:554-556).
    //
    // Omitting them leaves the port two draws behind native for the rest of
    // initialisation, which is what desynced a stock C++ client
    // (clonk-org/clonk-rs#1050). The results are discarded; only the ledger
    // movement is observable, so this pins the count and the exact values the
    // two evaluations consume rather than anything downstream.
    let mut engine = Engine::with_seed(0x1050);
    let map_width = crate::scenario::LegacyC4SVal::new(104, 0, 64, 250);
    let map_height = crate::scenario::LegacyC4SVal::new(77, 0, 40, 250);

    let before = engine.rng.count;
    let mut oracle = engine.rng.clone();
    engine.spend_map_creator_discard_draws(map_width, map_height);

    assert_eq!(
        engine.rng.count - before,
        2,
        "one draw per axis, exactly as C4MCMap::Default spends them"
    );

    // And they are the C4SVal evaluations themselves, not two bare Random(1)
    // calls that happen to advance the ledger by the same amount.
    let expected_width = map_width.evaluate(&mut oracle);
    let expected_height = map_height.evaluate(&mut oracle);
    assert_eq!(
        expected_width, 104,
        "a fixed value still evaluates to itself"
    );
    assert_eq!(expected_height, 77);
    assert_eq!(
        engine.rng.hold, oracle.hold,
        "the ledger must land where the two evaluations leave it"
    );
}
