use crate::support::real_scenario::{join_local_player_on_team, load_installed_scenario};
use clonk_engine::{Engine, ObjectId, SpawnConfig};
use clonk_script::Value;

/// Queron drives its relaunches from script: `Cycle` reacts to `PlrClonkDied`
/// by creating the next class and fading the fog out on the corpse, and the
/// `FoWFadeOut`/`IntWait2Launch` effect chain runs the countdown before
/// `StartToPlay` re-enables the crew and fades the fog back in
/// (content Melees.c4f/Queron3.c4s/Script.c:566-601,663-700,723-746,843-865,
/// 946-950).
///
/// The probe only observes; every state change comes from the shipped scripts.
const PROBE: &str = r#"#strict
public func Options() { return GameCall("OnGameOptionsDone"); }

public func Slay(object target, int killer)
{
    SetKiller(killer, target);
    return Kill(target, true);
}

public func CrewOf(int plr) { return GetCrew(plr, 0); }
public func CrewId(int plr) { return GetID(GetCrew(plr, 0)); }
public func Waiting() { return GetEffect("IntWait2Launch"); }

// IntWait2Launch is global and carries its player in EffectVar(0)
// (Queron3.c4s/Script.c:580,845), so a per-player probe has to scan.
public func WaitingFor(int plr)
{
    var index, number;
    while (number = GetEffect("IntWait2Launch", 0, index++))
        if (EffectVar(0, 0, number) == plr)
            return number;
    return 0;
}
public func Inactive(object target) { return GetEffect("IntInactive", target); }

public func VanishOnly(object target) { return RemoveObject(target); }
"#;

fn ask(engine: &mut Engine, probe: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(probe));
    crate::support::TestValueExt::test_value(engine.call_object_function(index, function, args))
}

fn tick(engine: &mut Engine, frames: usize) {
    for _ in 0..frames {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
}

/// Run one full relaunch and return whether the countdown was ever observed.
fn relaunch(engine: &mut Engine, probe: ObjectId, victim_owner: i32, killer: i32) -> bool {
    let victim = ask(engine, probe, "CrewOf", vec![Value::Int(victim_owner)]);
    assert!(
        !matches!(victim, Value::Nil),
        "the player owns a live clonk before the kill"
    );
    ask(engine, probe, "Slay", vec![victim, Value::Int(killer)]);

    // The fade-out runs 3 seconds and the relaunch countdown 60 ticks of 5
    // frames, so one cycle needs well under 900 frames.
    let mut saw_countdown = false;
    for _ in 0..900 {
        tick(engine, 1);
        if !matches!(ask(engine, probe, "Waiting", vec![]), Value::Nil) {
            saw_countdown = true;
        }
    }
    saw_countdown
}

/// The reported failure is a player who dies as the assassin and never reaches
/// the paladin life: no relaunch countdown, and a screen that stays black
/// because the replacement clonk keeps the `SetPlrViewRange(-1)` darkness it
/// spawns with (clonk-org/clonk-rs#590).
///
/// Pin the whole chain: both relaunches complete, the countdown runs, the
/// paladin becomes controllable again, and the player's fog view reopens on a
/// live object.
#[test]
fn queron_assassin_death_relaunches_a_visible_paladin() {
    let mut engine = load_installed_scenario("Melees.c4f/Queron3.c4s", 4);
    let host = join_local_player_on_team(&mut engine, "Host", 1);
    let guest = join_local_player_on_team(&mut engine, "Guest", 2);
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "QPRB",
        "Queron relaunch probe",
        PROBE,
    ));
    let probe =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("QPRB")));

    // The host menu configures the round; confirming it launches both players
    // (Queron3.c4s/Script.c:653-661).
    ask(&mut engine, probe, "Options", vec![]);
    tick(&mut engine, 120);
    assert_eq!(
        ask(&mut engine, probe, "CrewId", vec![Value::Int(host)]),
        Value::C4Id("KNIG".into()),
        "the first life is the knight"
    );

    assert!(
        relaunch(&mut engine, probe, host, guest),
        "the knight's death runs the relaunch countdown"
    );
    assert_eq!(
        ask(&mut engine, probe, "CrewId", vec![Value::Int(host)]),
        Value::C4Id("ASAS".into()),
        "the second life is the assassin"
    );

    assert!(
        relaunch(&mut engine, probe, host, guest),
        "the assassin's death runs the relaunch countdown"
    );
    assert_eq!(
        ask(&mut engine, probe, "CrewId", vec![Value::Int(host)]),
        Value::C4Id("PLDN".into()),
        "the third life is the paladin"
    );

    let paladin = ask(&mut engine, probe, "CrewOf", vec![Value::Int(host)]);
    assert!(
        matches!(
            ask(&mut engine, probe, "Inactive", vec![paladin.clone()]),
            Value::Nil
        ),
        "StartToPlay removes IntInactive, so the paladin is controllable"
    );

    let snapshot = engine.snapshot();
    let paladin_id = match paladin {
        Value::Object(id) => ObjectId::new(id),
        other => panic!("the paladin is a live object, got {other:?}"),
    };
    let view_range = snapshot
        .objects
        .iter()
        .find(|object| object.id == paladin_id)
        .map(|object| object.plr_view_range);
    assert!(
        matches!(view_range, Some(range) if range > 0),
        "FoWFadeIn reopens the fog around the relaunched clonk, got {view_range:?}"
    );

    let player = snapshot
        .players
        .iter()
        .find(|player| player.id == host)
        .expect("the host player is live");
    let focus = player
        .view_cursor
        .or(player.cursor)
        .expect("C4Player::UpdateView needs a ViewCursor or Cursor to center on");
    assert!(
        snapshot.object(focus).is_some(),
        "the player's view centers on a live object rather than a stale pointer"
    );
}

/// A crew clonk can leave the world without dying — removed by script or by
/// the world rather than killed — and `Cycle` only ever reacts to
/// `PlrClonkDied` (Queron3.c4s/Script.c:566-601). That asymmetry is the shape
/// of the black-screen report in clonk-org/clonk-rs#590: a player whose clonk
/// goes away without the death callback would get no replacement, no
/// countdown, and a fog that never reopens.
///
/// It does not happen — the relaunch runs anyway — and this pins that, because
/// the failure mode it rules out is invisible from the existing
/// kill-driven subcase.
#[test]
fn queron_removed_crew_still_reaches_the_relaunch_countdown() {
    let mut engine = load_installed_scenario("Melees.c4f/Queron3.c4s", 4);
    let host = join_local_player_on_team(&mut engine, "Host", 1);
    let _guest = join_local_player_on_team(&mut engine, "Guest", 2);
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "QPRB",
        "Queron relaunch probe",
        PROBE,
    ));
    let probe =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("QPRB")));
    ask(&mut engine, probe, "Options", vec![]);
    tick(&mut engine, 120);

    let victim = ask(&mut engine, probe, "CrewOf", vec![Value::Int(host)]);
    assert!(
        !matches!(victim, Value::Nil),
        "the host owns a live clonk before it is removed"
    );
    ask(&mut engine, probe, "VanishOnly", vec![victim]);

    let mut saw_countdown = false;
    for _ in 0..900 {
        tick(&mut engine, 1);
        if !matches!(ask(&mut engine, probe, "Waiting", vec![]), Value::Nil) {
            saw_countdown = true;
        }
    }
    assert!(
        saw_countdown,
        "a removed crew member still runs the relaunch countdown"
    );

    let replacement = ask(&mut engine, probe, "CrewOf", vec![Value::Int(host)]);
    assert!(
        !matches!(replacement, Value::Nil),
        "and the player is given a live clonk again rather than left empty"
    );
}

/// Two players waiting to relaunch at once (clonk-org/clonk-rs#590).
///
/// Queron installs the countdown as a **global** effect —
/// `AddEffect("IntWait2Launch",, 1, 5, clonk,, iPlr, …)` with an empty target
/// (`Queron3.c4s/Script.c:580,601`) — so concurrent relaunches mean several
/// same-named global effects distinguished only by `EffectVar(0)`. The report
/// is from a host with two clients and says the black screen happens
/// "sometimes", which is the shape of one player's wait being resolved against
/// another player's effect.
///
/// Priority 1 is what should make that safe: C4Effect keeps priority-1 effects
/// out of the `Fx*Effect` call chain entirely (`C4Effect.cpp:97`), so a second
/// countdown neither merges with nor displaces the first, and
/// `FxIntWait2LaunchStop` cycles `EffectVar(0, pTarget, iNr)` — its own player,
/// not a looked-up one (`Script.c:866-869`).
///
/// This pins that reasoning against the shipped scripts rather than leaving it
/// as an argument: both players must reach their own countdown and both must
/// advance a class.
#[test]
fn queron_overlapping_relaunches_each_keep_their_own_countdown() {
    let mut engine = load_installed_scenario("Melees.c4f/Queron3.c4s", 4);
    let host = join_local_player_on_team(&mut engine, "Host", 1);
    let guest = join_local_player_on_team(&mut engine, "Guest", 2);
    let third = join_local_player_on_team(&mut engine, "Third", 2);
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "QPRB",
        "Queron relaunch probe",
        PROBE,
    ));
    let probe =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("QPRB")));

    ask(&mut engine, probe, "Options", vec![]);
    tick(&mut engine, 120);
    for player in [host, guest, third] {
        assert_eq!(
            ask(&mut engine, probe, "CrewId", vec![Value::Int(player)]),
            Value::C4Id("KNIG".into()),
            "every player starts on the knight"
        );
    }

    // Kill the host, let its countdown start, then kill the guest while that
    // first wait is still running. Overlapping rather than simultaneous is the
    // harder case: it is when a second effect joins a list that already has one.
    let host_crew = ask(&mut engine, probe, "CrewOf", vec![Value::Int(host)]);
    ask(
        &mut engine,
        probe,
        "Slay",
        vec![host_crew, Value::Int(third)],
    );
    let mut host_waiting = false;
    for _ in 0..400 {
        tick(&mut engine, 1);
        if !matches!(
            ask(&mut engine, probe, "WaitingFor", vec![Value::Int(host)]),
            Value::Nil | Value::Int(0)
        ) {
            host_waiting = true;
            break;
        }
    }
    assert!(host_waiting, "the host reaches its own relaunch countdown");

    let guest_crew = ask(&mut engine, probe, "CrewOf", vec![Value::Int(guest)]);
    ask(
        &mut engine,
        probe,
        "Slay",
        vec![guest_crew, Value::Int(third)],
    );
    let mut guest_waiting = false;
    for _ in 0..400 {
        tick(&mut engine, 1);
        if !matches!(
            ask(&mut engine, probe, "WaitingFor", vec![Value::Int(guest)]),
            Value::Nil | Value::Int(0)
        ) {
            guest_waiting = true;
            break;
        }
    }
    assert!(
        guest_waiting,
        "a second player's countdown starts while the first is still running"
    );

    // The crux: two *distinct* global effects coexist, one per player, rather
    // than the second replacing or merging with the first. Equal numbers here
    // would mean one countdown serving two players, which is how a player ends
    // up stranded on the black screen.
    let host_wait = ask(&mut engine, probe, "WaitingFor", vec![Value::Int(host)]);
    let guest_wait = ask(&mut engine, probe, "WaitingFor", vec![Value::Int(guest)]);
    assert!(
        !matches!(host_wait, Value::Nil | Value::Int(0)),
        "the first countdown survives the second player joining the wait"
    );
    assert_ne!(
        host_wait, guest_wait,
        "each player owns its own IntWait2Launch effect"
    );

    // Both must finish. A wait resolved against the wrong player would strand
    // one of them with no crew and the fog never reopening — the reported
    // black screen.
    for _ in 0..1200 {
        tick(&mut engine, 1);
    }
    for (label, player) in [("host", host), ("guest", guest)] {
        let crew = ask(&mut engine, probe, "CrewId", vec![Value::Int(player)]);
        assert_eq!(
            crew,
            Value::C4Id("ASAS".into()),
            "{label} advanced to its own next life instead of being stranded"
        );
    }
}
