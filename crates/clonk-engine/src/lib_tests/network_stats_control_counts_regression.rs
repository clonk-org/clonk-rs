use super::*;
use crate::player::CountedControlType;

#[test]
fn control_counts_drain_without_resetting_action_deduplication() {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_player(PlayerConfig::new(5, "First")));
    crate::TestValueExt::test_value(engine.register_player(PlayerConfig::new(2, "Second")));

    engine.count_player_control(5, CountedControlType::Command, 77, 1);
    engine.count_player_control(5, CountedControlType::Command, 77, 1);
    engine.count_player_control(2, CountedControlType::DirectCom, 9, 1);

    assert_eq!(
        engine.players().map(Player::id).collect::<Vec<_>>(),
        vec![5, 2],
        "the fixture exercises native link order rather than map order"
    );
    assert_eq!(
        engine.take_player_control_counts(),
        vec![(5, 2, 1), (2, 1, 1)]
    );
    assert_eq!(
        engine.take_player_control_counts(),
        vec![(5, 0, 0), (2, 0, 0)],
        "each statistics sample drains only the counters"
    );

    engine.count_player_control(5, CountedControlType::Command, 77, 1);
    assert_eq!(
        engine.take_player_control_counts(),
        vec![(5, 1, 0), (2, 0, 0)],
        "LastControl survives the drain and suppresses the repeated action"
    );
}
