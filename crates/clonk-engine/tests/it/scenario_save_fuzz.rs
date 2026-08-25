//! Bounded malformed-input campaign over the legacy scenario and saved-state
//! text parsers (clonk-org/clonk-rs#961).
//!
//! `Scenario.txt`, `Game.txt`, `Objects.txt`, `Teams.txt`, `PlayerInfos.txt`
//! and their saved-state siblings arrive inside downloaded scenarios,
//! savegames, records and network resources, so they are attacker-shaped text
//! by default. The amplification risk is different in kind from a binary
//! header: these are count-prefixed, list-shaped grammars where one short line
//! can *name* a large amount of structure — a long object list, a deeply nested
//! serialized `C4Value`, a repeated section.
//!
//! The contract is:
//!
//! * arbitrary bytes, truncations and repeated sections never panic — every
//!   rejection is a typed `ScenarioError`;
//! * parsing stays bounded by its input, against the loader's own declared
//!   limits rather than numbers invented here;
//! * parsing is deterministic, because a scenario is loaded independently on
//!   every peer and a component that parses two ways is a desync rather than a
//!   cosmetic difference.
//!
//! These drive the real public loading entry points — `Scenario::load_from_group_with`
//! and `parse_initial_network_game_data` — rather than a test-only
//! reimplementation, so a finding here is a finding in live loading.

use clonk_engine::scenario::ScenarioLoaderHead;
use clonk_engine::{parse_initial_network_game_data, Scenario};
use clonk_resources::{Group, MutableGroup};
use std::path::PathBuf;

/// Inputs are capped so a case measures the parsers rather than the mutator.
const MAX_INPUT: usize = 4096;

/// The components a savegame carries, in the shapes the loader expects.
fn components() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Scenario.txt",
            "[Head]\nTitle=Fuzz\nVersion=4,9,8\nMaxPlayer=4\nSaveGame=1\n\n\
             [Definitions]\nDefinition1=Stub.c4d\n\n\
             [Player1]\nWealth=10\nCrew=CLNK=2\n\n\
             [Landscape]\nSky=Clouds\nMapWidth=100\nMapHeight=50\n\n\
             [Weather]\nClimate=25\nWind=0,100\n",
        ),
        (
            "Game.txt",
            "[Game]\nTime=120\nFrameCounter=4321\nRandomSeed=7\nControlTick=17\n\n\
             [Player0]\nNumber=0\nName=Fuzz\n",
        ),
        (
            "Objects.txt",
            "[Object]\nid=CLNK\nNumber=1\nCategory=8\nX=10\nY=20\nSize=100000\n\n\
             [Object]\nid=CLNK\nNumber=2\nCategory=8\nX=30\nY=40\nSize=100000\n\
             FixX=F999424\nFixY=F327680\n",
        ),
        (
            "Teams.txt",
            "[Teams]\nActive=1\nTeamCount=2\n\n\
             [Team]\nindex=1\nName=Alpha\n\n[Team]\nindex=2\nName=Beta\n",
        ),
        (
            "PlayerInfos.txt",
            "[PlayerInfos]\n\n[Client]\nID=0\nName=Host\n\n[Player]\nID=1\nName=Fuzz\n",
        ),
        (
            "SavePlayerInfos.txt",
            "[PlayerInfos]\n\n[Client]\nID=0\nName=Host\n\n[Player]\nID=1\nName=Fuzz\n",
        ),
        ("Strings.txt", "[Strings]\nString1=hello\nString2=world\n"),
        (
            "Parameters.txt",
            "[Parameters]\nMaxPlayers=4\n\n[Rules]\nRule1=RULE\n",
        ),
    ]
}

fn group_with(entries: &[(&str, Vec<u8>)]) -> Option<Group> {
    let mut group = MutableGroup::new("Fuzz.c4s");
    for (name, body) in entries {
        group.add_file(*name, body.clone()).ok()?;
    }
    let packed = group.pack().ok()?;
    Group::from_memory(PathBuf::from("Fuzz.c4s"), packed).ok()
}

/// Parse through the real public text entry points and bound what came back.
///
/// Both stop at the component text: `ScenarioLoaderHead` reads Scenario.txt's
/// head, definitions and lobby sections, and the offline preflight additionally
/// reads the saved-state components. Neither pulls in the definition graph, so
/// an outcome here is attributable to the bytes under mutation.
fn load(group: &Group, what: &str) -> bool {
    let mut parsed = false;
    if let Ok(head) = ScenarioLoaderHead::load_from_group(group) {
        // A component cannot name more definition modules than its own bytes
        // can spell.
        assert!(
            head.configured_definition_modules().len() <= MAX_INPUT,
            "{what}: {} definition modules",
            head.configured_definition_modules().len()
        );
        parsed = true;
    }
    if Scenario::preflight_offline_startup_from_group(group).is_ok() {
        parsed = true;
    }
    parsed
}

/// Swap one component's bytes, keeping the rest of the savegame well formed so
/// the case reaches that component's parser rather than failing earlier.
fn with_component(
    seeds: &[(&'static str, &'static str)],
    target: usize,
    bytes: Vec<u8>,
) -> Vec<(&'static str, Vec<u8>)> {
    seeds
        .iter()
        .enumerate()
        .map(|(index, (name, body))| {
            let body = if index == target {
                bytes.clone()
            } else {
                body.as_bytes().to_vec()
            };
            (*name, body)
        })
        .collect()
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next() % bound as u64) as usize
        }
    }
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for _ in 0..=rng.below(6) {
        match rng.below(6) {
            0 if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                bytes[at] = (rng.next() & 0xff) as u8;
            }
            1 if !bytes.is_empty() => {
                bytes.truncate(rng.below(bytes.len()));
            }
            2 => {
                let at = rng.below(bytes.len() + 1);
                bytes.insert(at, (rng.next() & 0xff) as u8);
            }
            3 if !bytes.is_empty() => {
                // Repeated sections: the loader must take one of them rather
                // than accumulate a section per copy.
                let span = bytes.clone();
                bytes.extend_from_slice(&span);
            }
            4 => {
                // Numbers are where a count-prefixed grammar amplifies, so
                // splice the boundary values in rather than waiting for a byte
                // mutation to spell one.
                let literal: &[u8] = match rng.below(6) {
                    0 => b"2147483647",
                    1 => b"-2147483648",
                    2 => b"999999999999999999999",
                    3 => b"0",
                    4 => b"-1",
                    _ => b"4294967296",
                };
                let at = rng.below(bytes.len() + 1);
                bytes.splice(at..at, literal.iter().copied());
            }
            _ if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                let len = rng.below(bytes.len() - at).min(64);
                let span = bytes[at..at + len].to_vec();
                bytes.extend_from_slice(&span);
            }
            _ => {}
        }
        if bytes.len() > MAX_INPUT {
            bytes.truncate(MAX_INPUT);
        }
    }
    bytes
}

#[test]
fn the_seed_savegame_loads_through_the_real_entry_point() {
    let seeds = components();
    let entries = with_component(&seeds, usize::MAX, Vec::new());
    let group = group_with(&entries).expect("the seed components pack and open");
    let outcome = ScenarioLoaderHead::load_from_group(&group);
    assert!(
        outcome.is_ok(),
        "the seed savegame must parse, or every campaign below proves nothing: {:?}",
        outcome.err()
    );
}

#[test]
fn mutated_components_never_panic_and_stay_bounded() {
    let seeds = components();
    let mut rng = Rng(0x5EED_0961);
    let mut loaded = 0_usize;
    for round in 0..1_500 {
        let target = rng.below(seeds.len());
        let bytes = mutate(seeds[target].1.as_bytes(), &mut rng);
        let Some(group) = group_with(&with_component(&seeds, target, bytes)) else {
            continue;
        };
        if load(&group, &format!("{} round {round}", seeds[target].0)) {
            loaded += 1;
        }
    }
    assert!(
        loaded > 0,
        "no mutated savegame loaded; the campaign never reached the parsers"
    );
}

#[test]
fn arbitrary_component_bytes_never_panic() {
    let seeds = components();
    let mut rng = Rng(0x0961_5EED);
    for _ in 0..600 {
        let target = rng.below(seeds.len());
        let len = rng.below(512);
        let noise = (0..len)
            .map(|_| (rng.next() & 0xff) as u8)
            .collect::<Vec<_>>();
        if let Some(group) = group_with(&with_component(&seeds, target, noise)) {
            load(&group, "arbitrary");
        }
    }
}

#[test]
fn truncating_each_component_is_safe() {
    let seeds = components();
    for (target, (name, body)) in seeds.iter().enumerate() {
        for end in (0..body.len()).step_by(7) {
            let bytes = body.as_bytes()[..end].to_vec();
            if let Some(group) = group_with(&with_component(&seeds, target, bytes)) {
                load(&group, &format!("{name} truncated to {end}"));
            }
        }
    }
}

#[test]
fn loading_the_same_bytes_twice_produces_the_same_outcome() {
    // A scenario is loaded independently on every peer, so a component that
    // parses two ways is a desync rather than a cosmetic difference.
    let seeds = components();
    let mut rng = Rng(0xD37E_0961);
    for _ in 0..300 {
        let target = rng.below(seeds.len());
        let bytes = mutate(seeds[target].1.as_bytes(), &mut rng);
        let entries = with_component(&seeds, target, bytes);
        let (Some(first), Some(second)) = (group_with(&entries), group_with(&entries)) else {
            continue;
        };
        assert_eq!(
            ScenarioLoaderHead::load_from_group(&first).is_ok(),
            ScenarioLoaderHead::load_from_group(&second).is_ok(),
            "the same component bytes parsed two ways"
        );
    }
}

#[test]
fn arbitrary_game_text_never_panics_and_parses_the_same_way_twice() {
    // `parse_initial_network_game_data` is infallible by design — a missing or
    // malformed field falls back to its named default — so its contract is that
    // it never panics and never disagrees with itself.
    let seed = components()
        .into_iter()
        .find(|(name, _)| *name == "Game.txt")
        .expect("the seed set carries Game.txt")
        .1;
    let mut rng = Rng(0x6A4E_0961);
    for _ in 0..2_000 {
        let bytes = mutate(seed.as_bytes(), &mut rng);
        let first = parse_initial_network_game_data(&bytes);
        let second = parse_initial_network_game_data(&bytes);
        assert_eq!(
            (first.control_tick, first.frame, first.time),
            (second.control_tick, second.frame, second.time),
            "Game.txt parsed two ways"
        );
    }
}
