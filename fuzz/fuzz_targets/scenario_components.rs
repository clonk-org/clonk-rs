//! Legacy scenario and saved-state components through the production loader
//! (clonk-org/clonk-rs#961).
//!
//! Scenario.txt, Game.txt, Objects.txt, Teams.txt and the rest arrive from
//! downloaded scenarios, saves, records and peers, so these text grammars read
//! attacker-shaped bytes. The amplification is in the *counts and nesting*:
//! count-prefixed value lists, nested arrays and maps in `Locals=`, long object
//! lists and repeated sections all let a small component name a large amount of
//! work.
//!
//! The bytes are composed into a real in-memory group and handed to the same
//! `Scenario::load_from_group_with` the game uses, so the target exercises
//! component parsing, cross-component references and the loader's own defaulting
//! rather than a parser called in isolation. Archive structure itself belongs to
//! clonk-org/clonk-rs#959.

#![no_main]

use std::path::PathBuf;

use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::Scenario;
use clonk_resources::{Group, MutableGroup};
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's cap, so a finding here reproduces there.
const MAX_INPUT: usize = 16 * 1024;

/// The components the loader consults. A case splits its input across these, so
/// one mutation can produce inconsistent counts and references *between*
/// components, which is where the cross-component assumptions live.
const COMPONENTS: [&str; 8] = [
    "Scenario.txt",
    "Game.txt",
    "Objects.txt",
    "Teams.txt",
    "PlayerInfos.txt",
    "SavePlayerInfos.txt",
    "Strings.txt",
    "Parameters.txt",
];

/// The loader resolves definition references; a fuzz case has no definitions to
/// resolve, which is itself the "dangling reference" shape worth exercising.
struct NoDefinitions;

impl LegacyDefinitionResolver for NoDefinitions {
    fn resolve_definition_groups(
        &self,
        _group: &Group,
        _definitions: &str,
    ) -> Result<Vec<Group>, clonk_engine::ScenarioError> {
        Ok(Vec::new())
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    // First byte selects how many components the rest is split across, so the
    // mutator can reach both "one big component" and "many inconsistent ones".
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    let parts = usize::from(selector % COMPONENTS.len() as u8) + 1;
    let chunk = body.len().div_ceil(parts).max(1);

    let mut group = MutableGroup::new("Fuzz.c4s");
    for (name, bytes) in COMPONENTS.iter().zip(body.chunks(chunk)) {
        if group.add_file(*name, bytes.to_vec()).is_err() {
            return;
        }
    }
    let Ok(image) = group.pack() else {
        return;
    };
    let Ok(group) = Group::from_memory(PathBuf::from("Fuzz.c4s"), image) else {
        return;
    };

    // The contract is a typed result: malformed components are rejected, they
    // do not panic, overflow the stack, or run unbounded work.
    let _ = Scenario::load_from_group_with(&group, &NoDefinitions);
});
