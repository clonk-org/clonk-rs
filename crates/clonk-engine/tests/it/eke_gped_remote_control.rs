use crate::support::real_scenario::prepare_installed_scenario;
use clonk_engine::{
    CommandDirection, Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOUBLE, COM_LEFT,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_SPECIAL2, COM_WHEEL_DOWN,
};
use clonk_script::Value;

const SCENARIO: &str = "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/AirbikeFight.c4s";
const APPEND: &str = "EkeGpedRemoteControl.c";

fn join_pilot(engine: &mut Engine, auto_stop: bool) -> i32 {
    engine
        .join_player(JoinPlayerConfig {
            name: "Eke remote control".into(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: auto_stop,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("the local virtual player joins")
        .initialized()
        .expect("AirbikeFight needs no runtime team selection")
        .number
}

fn local(engine: &Engine, object: ObjectId, name: &str) -> Value {
    engine
        .object_snapshot(object)
        .unwrap_or_else(|| panic!("object {object:?} remains live"))
        .local_vars
        .get(name)
        .cloned()
        .unwrap_or(Value::Nil)
}

fn action_name(engine: &Engine, object: ObjectId) -> String {
    engine
        .object_snapshot(object)
        .unwrap_or_else(|| panic!("object {object:?} remains live"))
        .action
        .name
}

fn com_dir(engine: &Engine, object: ObjectId) -> CommandDirection {
    engine
        .object_snapshot(object)
        .unwrap_or_else(|| panic!("object {object:?} remains live"))
        .command_direction
}

fn tick(engine: &mut Engine, frames: u32) {
    for _ in 0..frames {
        engine.tick_without_snapshot().expect("the frame executes");
    }
}

struct RemoteControl {
    owner: i32,
    sft: ObjectId,
    gped: ObjectId,
    airbike: ObjectId,
}

/// AirbikeFight hands every joined SFT a GPED, a jetpack and its own airbike,
/// and seats the SFT on that airbike
/// (EkeReloaded.c4f/InterplanetaryCivilwar.c4f/AirbikeFight.c4s/Script.c:28-63,
/// reached through Deathmatch::InitializePlayer's
/// `GameCall("InitializeClonk", ...)`).
///
/// Walks the issue's reproduction up to the point where the GPED steers the
/// airbike: dismount, select the GPED, switch it to Control mode and activate
/// it on the bike.
fn steering_by_remote_control(engine: &mut Engine, auto_stop: bool) -> RemoteControl {
    let owner = join_pilot(engine, auto_stop);
    let sft = engine
        .crew_cursor(owner)
        .expect("AirbikeFight joins with a selected SFT");
    assert_eq!(
        action_name(engine, sft),
        "AirbikeFly",
        "InitializeClonk seats the fresh SFT on its airbike"
    );

    let airbike = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "AB5B" && object.owner == owner)
        .map(|object| object.id)
        .expect("InitializeClonk creates one airbike per player");

    // "Land/exit the bike": Airbike::Entrance is the shipped dismount body
    // (Airbike.c4d/Script.c:452-461) that Airbike::ControlDown also runs once
    // the bike is grounded.
    let airbike_index = engine
        .find_object_index(airbike)
        .expect("the airbike has an index");
    engine
        .call_object_function(airbike_index, "Entrance", Vec::new())
        .expect("the shipped dismount runs");
    // SFT::CheckArmed renames Walk after the selected item, so the dismounted
    // SFT lands in JetpackWalk (SFT.c4d/Script.c:441-445).
    let dismounted = action_name(engine, sft);
    assert!(
        dismounted.ends_with("Walk"),
        "the dismounted SFT walks again, not {dismounted}"
    );

    let gped = engine
        .object_snapshot(sft)
        .expect("the SFT remains live")
        .contents
        .into_iter()
        .find(|&object| {
            engine
                .object_snapshot(object)
                .is_some_and(|snapshot| snapshot.definition_id == "GP5B")
        })
        .expect("InitializeClonk equips one GPED");

    // Select the GPED through the engine's own inventory shift: the SFT only
    // forwards controls to Contents() (SFT.c4d/Script.c:288-295).
    for _ in 0..8 {
        if engine
            .object_snapshot(sft)
            .expect("the SFT remains live")
            .contents
            .first()
            == Some(&gped)
        {
            break;
        }
        engine
            .player_in_com(owner, COM_WHEEL_DOWN, 0)
            .expect("shifting the inventory succeeds");
    }
    assert_eq!(
        engine
            .object_snapshot(sft)
            .expect("the SFT remains live")
            .contents
            .first(),
        Some(&gped),
        "the GPED is the selected item"
    );

    // [Special2] rotates the GPED from Blaster to Control mode
    // (GPED.c4d/Script.c:129-150).
    engine
        .player_in_com(owner, COM_SPECIAL2, 0)
        .expect("the mode switch reaches the GPED");
    assert_eq!(
        local(engine, gped, "mode"),
        Value::String("Control".into()),
        "Special2 selects the GPED's remote-control mode"
    );

    // [Dig] double activates the selected item, and GPED::Activate runs
    // ControlDig, which picks the nearest matching airbike and hands the GPED
    // to Airbike::ControlRequest (GPED.c4d/Script.c:90-156,
    // Airbike.c4d/Script.c:180-199,477-489).
    engine
        .player_in_com(owner, COM_DIG | COM_DOUBLE, 0)
        .expect("activating the GPED succeeds");
    engine
        .player_in_com(owner, COM_DIG + COM_RELEASE_OFFSET, 0)
        .expect("releasing Dig succeeds");
    assert_eq!(
        local(engine, gped, "target"),
        Value::Object(airbike.as_u64()),
        "the GPED latches onto the airbike"
    );
    assert_eq!(
        action_name(engine, gped),
        "AirbikeFly",
        "Airbike::ControlRequest puts the GPED into its remote-control action"
    );
    assert_eq!(
        com_dir(engine, sft),
        CommandDirection::Stop,
        "GPED::ControlDig stops the pilot before handing over control"
    );

    RemoteControl {
        owner,
        sft,
        gped,
        airbike,
    }
}

/// clonk-org/clonk-rs#202: while the GPED remote-controls an airbike, the
/// pilot's own Clonk must stand still.
///
/// GPED::ControlLeft returns 1 once its action is "AirbikeFly"
/// (GPED.c4d/Script.c:15-23), SFT::ControlLeft passes that through
/// (SFT.c4d/Script.c:40-59), and `C4Object::DirectCom`'s object script override
/// returns before any `ObjectComMovement` (oracle C4Object.cpp:3399-3403).
/// Under Jump'n'Run control the stale `LastCom | COM_Single` coms are the hole
/// that `planet/System.c4g/EkeGpedRemoteControl.c` closes.
#[test]
fn eke_gped_remote_control_steers_the_airbike_without_walking_its_pilot() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for auto_stop in [false, true] {
        let mut engine = prepared.instantiate();
        let remote = steering_by_remote_control(&mut engine, auto_stop);
        let parked = engine
            .object_snapshot(remote.sft)
            .expect("the SFT remains live")
            .position;

        // [Left] steers the airbike.
        engine
            .player_in_com(remote.owner, COM_LEFT, 0)
            .expect("the turn reaches the GPED");
        assert_eq!(
            com_dir(&engine, remote.airbike),
            CommandDirection::Left,
            "the remote-controlled airbike turns left (auto_stop={auto_stop})"
        );
        assert_eq!(
            com_dir(&engine, remote.sft),
            CommandDirection::Stop,
            "the pilot must not walk off while remote-controlling the airbike \
             (auto_stop={auto_stop})"
        );

        // Holding the key past the double-click window makes
        // C4Player::ExecuteControl flush `LastCom | COM_Single` on its own
        // (oracle C4Player.cpp:1212-1228, C4Constants.h:156).
        tick(&mut engine, 12);
        assert_eq!(
            com_dir(&engine, remote.sft),
            CommandDirection::Stop,
            "the flushed single com must not walk the pilot (auto_stop={auto_stop})"
        );

        // Steering the other way is the ordinary next input: release [Left],
        // press [Right]. C4Player::InCom flushes the stale
        // `COM_Left | COM_Single` first (oracle C4Player.cpp:1522-1531).
        engine
            .player_in_com(remote.owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .expect("releasing Left succeeds");
        engine
            .player_in_com(remote.owner, COM_RIGHT, 0)
            .expect("the reverse turn reaches the GPED");
        assert_eq!(
            com_dir(&engine, remote.airbike),
            CommandDirection::Right,
            "the remote-controlled airbike turns back (auto_stop={auto_stop})"
        );
        assert_eq!(
            com_dir(&engine, remote.sft),
            CommandDirection::Stop,
            "changing the steering direction must not walk the pilot \
             (auto_stop={auto_stop})"
        );

        tick(&mut engine, 12);
        assert_eq!(
            com_dir(&engine, remote.sft),
            CommandDirection::Stop,
            "the pilot is still parked a moment later (auto_stop={auto_stop})"
        );
        assert_eq!(
            engine
                .object_snapshot(remote.sft)
                .expect("the SFT remains live")
                .position
                .x,
            parked.x,
            "the pilot stays put while the airbike flies (auto_stop={auto_stop})"
        );
    }
}

/// Pins the divergence itself: shipped Eke content really does walk the pilot,
/// and only under Jump'n'Run control. Without that A/B the append above reads
/// as a port bugfix rather than a deliberate departure from the oracle.
#[test]
fn shipped_eke_content_walks_the_remote_control_pilot_only_under_jump_and_run() {
    let prepared = prepare_installed_scenario(SCENARIO, 0);
    for (auto_stop, expected) in [
        (false, CommandDirection::Stop),
        (true, CommandDirection::Right),
    ] {
        let mut engine = prepared.instantiate_without_system_script(APPEND);
        let remote = steering_by_remote_control(&mut engine, auto_stop);

        engine
            .player_in_com(remote.owner, COM_LEFT, 0)
            .expect("the turn reaches the GPED");
        engine
            .player_in_com(remote.owner, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .expect("releasing Left succeeds");
        engine
            .player_in_com(remote.owner, COM_RIGHT, 0)
            .expect("the reverse turn reaches the GPED");

        assert_eq!(
            com_dir(&engine, remote.airbike),
            CommandDirection::Right,
            "the shipped GPED still steers the airbike (auto_stop={auto_stop})"
        );
        assert_eq!(
            com_dir(&engine, remote.sft),
            expected,
            "shipped AutoStopUpdateComDir behaviour for the stale \
             `COM_Left | COM_Single` (auto_stop={auto_stop})"
        );
        assert_eq!(
            action_name(&engine, remote.gped),
            "AirbikeFly",
            "the GPED keeps steering either way (auto_stop={auto_stop})"
        );
    }
}
