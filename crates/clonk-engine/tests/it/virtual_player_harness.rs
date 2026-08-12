use crate::support::virtual_player::{VirtualPlayer, VirtualPlayerError};
use clonk_engine::{
    AudioCommand, Definition, Engine, MenuRequest, MenuRequestKind, ObjectId, PlayerConfig,
    SpawnConfig, COM_DIG, COM_DOWN, COM_RIGHT, COM_SPECIAL,
};
use clonk_script::Value;
use std::error::Error;

const CONTROL_PROBE_SCRIPT: &str = r#"
#strict 2

local right_press, right_release, down_double, dig_single, chosen;

protected func ControlRight()         { right_press = 1; return(1); }
protected func ControlRightReleased() { right_release = 1; return(1); }
protected func ControlDownDouble()    { down_double = 1; return(1); }
protected func ControlDigSingle()     { dig_single = 1; return(1); }

protected func ControlSpecial()
{
	CreateMenu(MENU, this(), this(), 0, "Harness");
	AddMenuItem("First",  "Pick(1)", MENU, this());
	AddMenuItem("Second", "Pick(2)", MENU, this());
	AddMenuItem("Third",  "Pick(3)", MENU, this());
	AddMenuItem("Fourth", "Pick(4)", MENU, this());
	SetMenuSize(2, 2, this());
	SelectMenuItem(0, this());
	return(1);
}

public func Pick(value) { chosen = value; return(1); }
"#;

fn fixture() -> Result<(Engine, ObjectId), Box<dyn Error>> {
    let mut engine = Engine::with_seed(0);
    let mut definition =
        Definition::from_script("CLNK", "Virtual player probe", CONTROL_PROBE_SCRIPT)?;
    definition.set_crew_member(true);
    engine.register_definition(definition)?;
    engine.register_player(PlayerConfig::new(1, "Virtual player"))?;
    let crew = engine.spawn_object(
        SpawnConfig::new("CLNK")
            .with_owner(1)
            .with_crew_member(true),
    )?;
    engine.select_crew(1, [crew])?;
    engine.set_crew_cursor(1, Some(crew))?;
    Ok((engine, crew))
}

fn local(engine: &Engine, object: ObjectId, name: &str) -> Option<Value> {
    engine
        .object_snapshot(object)
        .and_then(|snapshot| snapshot.local_vars.get(name).cloned())
}

#[test]
fn snapshotless_tick_matches_a_discarded_snapshot_and_drains_frame_output(
) -> Result<(), Box<dyn Error>> {
    let (mut snapshot_engine, snapshot_crew) = fixture()?;
    let (mut snapshotless_engine, snapshotless_crew) = fixture()?;
    let audio = |target| AudioCommand::PlaySound {
        name: "SnapshotlessTick".to_owned(),
        target: Some(target),
        volume: 73,
        looped: false,
        multiple: false,
        custom_falloff: None,
    };
    let menu = |crew_id| MenuRequest {
        crew_id,
        owner: 1,
        kind: MenuRequestKind::Construction,
    };
    snapshot_engine.pending_audio.push(audio(snapshot_crew));
    snapshotless_engine
        .pending_audio
        .push(audio(snapshotless_crew));
    snapshot_engine
        .pending_menu_requests
        .push(menu(snapshot_crew));
    snapshotless_engine
        .pending_menu_requests
        .push(menu(snapshotless_crew));

    let emitted = snapshot_engine.tick()?;
    snapshotless_engine.tick_without_snapshot()?;

    assert_eq!(emitted.audio, vec![audio(snapshot_crew)]);
    assert_eq!(emitted.menu_requests, vec![menu(snapshot_crew)]);
    assert_eq!(snapshotless_engine.snapshot(), snapshot_engine.snapshot());

    let snapshot_frame = snapshot_engine.tick()?;
    let snapshotless_frame = snapshotless_engine.tick()?;
    assert_eq!(snapshotless_frame, snapshot_frame);
    assert!(snapshotless_frame.audio.is_empty());
    assert!(snapshotless_frame.menu_requests.is_empty());
    Ok(())
}

#[test]
fn physical_controls_preserve_cpp_press_release_double_and_single_timing(
) -> Result<(), Box<dyn Error>> {
    let (mut engine, crew) = fixture()?;
    let mut player = VirtualPlayer::new(&mut engine, 1);

    player.press(COM_RIGHT)?;
    player.release(COM_RIGHT)?;
    assert_eq!(
        local(player.engine(), crew, "right_press"),
        Some(Value::Int(1))
    );
    assert_eq!(
        local(player.engine(), crew, "right_release"),
        Some(Value::Int(1))
    );

    player.double_tap(COM_DOWN)?;
    assert_eq!(
        local(player.engine(), crew, "down_double"),
        Some(Value::Int(1))
    );

    player.tap(COM_DIG)?;
    // C4Player::ExecuteControl increments once per tick and flushes only for
    // `LastComDelay > C4DoubleClick` (src/C4Player.cpp:1215-1232), while
    // C4DoubleClick is exactly 10 (src/C4Constants.h:156).
    player.ticks(10)?;
    assert_eq!(
        local(player.engine(), crew, "dig_single"),
        Some(Value::Nil),
        "C4Player::ExecuteControl waits while LastComDelay == C4DoubleClick"
    );
    player.wait_out_double_click()?;
    assert_eq!(
        local(player.engine(), crew, "dig_single"),
        Some(Value::Int(1)),
        "the buffered single fires only when LastComDelay > C4DoubleClick"
    );
    Ok(())
}

#[test]
fn route_checkpoint_reset_changes_only_the_physical_input_ledger() -> Result<(), Box<dyn Error>> {
    let (mut engine, _) = fixture()?;
    {
        let control = &mut engine.player_mut(1)?.control;
        control.last_com = i32::from(COM_RIGHT);
        control.last_com_delay = 7;
        control.last_com_down_double = 3;
        control.pressed_coms = 1 << COM_RIGHT;
        control.control_style = false;
        control.auto_context_menu = true;
        control.cursor_flash = 11;
        control.select_flash = 12;
        control.cursor_selection = 13;
        control.cursor_toggled = 14;
    }

    let mut player = VirtualPlayer::new(&mut engine, 1);
    player.reset_input_ledger_with_control_style(true)?;

    let control = crate::support::TestValueExt::test_value(player.engine().player(1)).control;
    assert_eq!(
        (
            control.last_com,
            control.last_com_delay,
            control.last_com_down_double,
            control.pressed_coms,
            control.control_style,
        ),
        (0, 0, 0, 0, true)
    );
    assert_eq!(
        (
            control.auto_context_menu,
            control.cursor_flash,
            control.select_flash,
            control.cursor_selection,
            control.cursor_toggled,
        ),
        (true, 11, 12, 13, 14),
        "the checkpoint must not reset menu or cursor/selection state"
    );
    Ok(())
}

#[test]
fn menu_helpers_use_real_player_navigation_and_enter_controls() -> Result<(), Box<dyn Error>> {
    let (mut engine, crew) = fixture()?;
    let mut player = VirtualPlayer::new(&mut engine, 1);

    player.tap(COM_SPECIAL)?;
    player.assert_milestone("script menu opened", |engine| {
        engine.cursor_object_menu(1).is_some()
    })?;

    // Ordinary directional controls are converted to menu controls before
    // single/double processing (src/C4Player.cpp:1502-1513), then follow
    // C4Menu::Control's column and wrap rules (src/C4Menu.cpp:433-480).
    player.menu_right()?;
    player.menu_down()?;
    player.menu_left()?;
    player.menu_up()?;
    assert_eq!(
        player
            .engine()
            .cursor_object_menu(1)
            .expect("menu remains open")
            .1
            .selection,
        0,
        "right/down/left/up traverses the same two-column loop as C4Menu::Control"
    );

    player.menu_navigate_to_caption("Fourth")?;
    assert_eq!(
        player
            .engine()
            .cursor_object_menu(1)
            .expect("menu remains open")
            .1
            .selection,
        3
    );
    player.menu_enter()?;
    player.assert_milestone("fourth menu command executed", |engine| {
        local(engine, crew, "chosen") == Some(Value::Int(4))
    })?;
    assert!(player.engine().cursor_object_menu(1).is_none());

    player.ticks(11)?;
    player.tap(COM_SPECIAL)?;
    player.menu_close()?;
    assert!(player.engine().cursor_object_menu(1).is_none());
    Ok(())
}

#[test]
fn wait_until_reports_elapsed_ticks_and_a_labeled_timeout() -> Result<(), Box<dyn Error>> {
    let (mut engine, crew) = fixture()?;
    let mut player = VirtualPlayer::new(&mut engine, 1);
    let target_frame = player.engine().frame() + 3;

    assert_eq!(
        player.wait_until("three engine frames", 3, |engine| {
            engine.frame() == target_frame
        })?,
        3
    );

    let error = player
        .wait_until("impossible milestone", 2, |_| false)
        .expect_err("an unmet milestone must time out");
    let diagnostics = match &error {
        VirtualPlayerError::Timeout { diagnostics, .. } => diagnostics,
        other => panic!("expected timeout diagnostics, got {other:?}"),
    };
    assert!(diagnostics.contains("recent=[frame=3"));
    assert!(diagnostics.contains("frame=5"));
    assert!(diagnostics.contains("cursor=1:CLNK"));
    assert!(diagnostics.contains("pos=(0,0)"));
    assert!(diagnostics.contains("action=Idle"));
    assert!(diagnostics.contains("comdir=0"));
    assert!(diagnostics.contains("container=-"));
    assert!(diagnostics.contains("contents=[]"));
    assert!(diagnostics.contains("menu=closed"));
    assert!(diagnostics.contains("hud={owner=1"));
    assert!(diagnostics.contains("global-effects=[]"));
    assert!(error.to_string().contains(diagnostics));
    assert!(matches!(
        error,
        VirtualPlayerError::Timeout {
            ref milestone,
            max_ticks: 2,
            ..
        } if milestone == "impossible milestone"
    ));

    let error = player
        .wait_until("long impossible milestone", 8, |_| false)
        .expect_err("a longer unmet milestone must time out");
    let diagnostics = match &error {
        VirtualPlayerError::Timeout { diagnostics, .. } => diagnostics,
        other => panic!("expected timeout diagnostics, got {other:?}"),
    };
    assert!(diagnostics.starts_with("recent=[frame=8 "));
    assert!(diagnostics.contains("frame=13 "));
    assert_eq!(diagnostics.matches(" | ").count(), 5);

    let error = player
        .hold_until(COM_RIGHT, "unreachable while walking", 2, |_| false)
        .expect_err("an unmet held-control milestone must time out");
    assert!(matches!(error, VirtualPlayerError::Timeout { .. }));
    assert_eq!(
        local(player.engine(), crew, "right_release"),
        Some(Value::Int(1)),
        "hold_until releases the physical key after a timeout"
    );
    Ok(())
}
