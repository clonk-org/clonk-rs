//! Stock `Parameters.txt` serialization for the initial network save.

use std::fmt::Write as _;

use clonk_engine::{ClientCoreControlData, LegacyCString};
use thiserror::Error;

use crate::legacy::{JoinDataIdListEntry, JoinGameParametersEnvelope};

/// Scenario values used by `C4GameParameters::CompileFunc` as text defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitialNetworkScenarioDefaults {
    pub random_seed: i32,
    pub max_players: i32,
    pub use_fair_crew: bool,
    pub fair_crew_forced: bool,
    pub fair_crew_strength: i32,
    pub rules: Vec<JoinDataIdListEntry>,
    pub goals: Vec<JoinDataIdListEntry>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InitialNetworkParametersError {
    #[error("client ID {0} occurs more than once")]
    DuplicateClientId(i32),
}

/// Serializes the `Save(group, pScenario)` form of `C4GameParameters`.
///
/// Supplying scenario defaults is intentional: with non-null `pScenario`, C++
/// default-elides scenario-derived values and omits the league-address,
/// title/resource, player-info, and team block entirely
/// (`src/C4GameParameters.cpp:528-587`).
pub fn serialize_initial_network_parameters(
    parameters: &JoinGameParametersEnvelope,
    defaults: &InitialNetworkScenarioDefaults,
) -> Result<Vec<u8>, InitialNetworkParametersError> {
    let mut parameter_lines = Vec::new();
    push_i32_if_different(
        &mut parameter_lines,
        "RandomSeed",
        parameters.random_seed,
        defaults.random_seed,
    );
    push_i32_if_different(
        &mut parameter_lines,
        "StartupPlayerCount",
        parameters.startup_player_count,
        0,
    );
    push_i32_if_different(
        &mut parameter_lines,
        "MaxPlayers",
        parameters.max_players,
        defaults.max_players,
    );
    push_bool_if_different(
        &mut parameter_lines,
        "UseFairCrew",
        parameters.use_fair_crew,
        defaults.use_fair_crew,
    );
    push_bool_if_different(
        &mut parameter_lines,
        "FairCrewForced",
        parameters.fair_crew_forced,
        defaults.fair_crew_forced,
    );
    push_i32_if_different(
        &mut parameter_lines,
        "FairCrewStrength",
        parameters.fair_crew_strength,
        defaults.fair_crew_strength,
    );
    push_bool_if_different(
        &mut parameter_lines,
        "AllowDebug",
        parameters.allow_debug,
        true,
    );
    push_bool_if_different(
        &mut parameter_lines,
        "IsNetworkGame",
        parameters.is_network_game,
        false,
    );
    push_i32_if_different(
        &mut parameter_lines,
        "ControlRate",
        parameters.control_rate,
        -1,
    );
    push_bool_if_different(
        &mut parameter_lines,
        "AutoFrameSkip",
        parameters.auto_frame_skip,
        false,
    );
    if parameters.rules != defaults.rules && !parameters.rules.is_empty() {
        parameter_lines.push(format!("Rules={}", encode_id_list(&parameters.rules)));
    }
    if parameters.goals != defaults.goals && !parameters.goals.is_empty() {
        parameter_lines.push(format!("Goals={}", encode_id_list(&parameters.goals)));
    }
    if !parameters.league.is_empty() {
        parameter_lines.push(format!("League={}", quote_text(&parameters.league)));
    }

    let mut clients = parameters.clients.clients.iter().collect::<Vec<_>>();
    // C4ClientList::Add maintains ascending client-ID order before CompileFunc
    // walks the linked list (src/C4Client.cpp:150-176,353-371).
    clients.sort_by_key(|client| client.client_id);
    if let Some(client_id) = clients
        .windows(2)
        .find(|pair| pair[0].client_id == pair[1].client_id)
        .map(|pair| pair[0].client_id)
    {
        return Err(InitialNetworkParametersError::DuplicateClientId(client_id));
    }
    let client_sections = clients
        .into_iter()
        .map(client_lines)
        .filter(|lines| !lines.is_empty())
        .collect::<Vec<_>>();

    if parameter_lines.is_empty() && client_sections.is_empty() {
        return Ok(Vec::new());
    }

    let mut output = String::from("[Parameters]\r\n");
    for line in parameter_lines {
        output.push_str(&line);
        output.push_str("\r\n");
    }
    for lines in client_sections {
        output.push_str("\r\n  [Client]\r\n");
        for line in lines {
            output.push_str("  ");
            output.push_str(&line);
            output.push_str("\r\n");
        }
    }
    Ok(output.into_bytes())
}

fn push_i32_if_different(lines: &mut Vec<String>, name: &str, value: i32, default: i32) {
    if value != default {
        lines.push(format!("{name}={value}"));
    }
}

fn push_bool_if_different(lines: &mut Vec<String>, name: &str, value: bool, default: bool) {
    if value != default {
        lines.push(format!("{name}={}", if value { "true" } else { "false" }));
    }
}

fn encode_id_list(entries: &[JoinDataIdListEntry]) -> String {
    entries
        .iter()
        .map(|entry| {
            let id = entry
                .id
                .as_bytes()
                .iter()
                .copied()
                .map(char::from)
                .collect::<String>();
            format!("{id}={}", entry.count)
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn client_lines(client: &ClientCoreControlData) -> Vec<String> {
    let mut lines = Vec::new();
    push_i32_if_different(&mut lines, "ID", client.client_id, -1);
    push_bool_if_different(&mut lines, "Activated", client.activated, false);
    push_bool_if_different(&mut lines, "Observer", client.observer, false);
    if !client.name.is_empty() {
        lines.push(format!("Name={}", quote_text(&client.name)));
    }
    if !client.nick.is_empty() {
        lines.push(format!("Nick={}", quote_text(&client.nick)));
    }
    push_bool_if_different(&mut lines, "LobbyReady", client.lobby_ready, false);
    lines
}

fn quote_text(value: &LegacyCString) -> String {
    let mut output = String::with_capacity(value.as_bytes().len() + 2);
    output.push('"');
    let mut last_was_numeric_escape = false;
    for byte in value.as_bytes() {
        let numeric_escape = last_was_numeric_escape && byte.is_ascii_digit();
        last_was_numeric_escape = false;
        if (!byte.is_ascii_graphic() && *byte != b' ')
            || *byte == b'\\'
            || *byte == b'"'
            || numeric_escape
        {
            match byte {
                b'\x07' => output.push_str("\\a"),
                b'\x08' => output.push_str("\\b"),
                b'\x0c' => output.push_str("\\f"),
                b'\n' => output.push_str("\\n"),
                b'\r' => output.push_str("\\r"),
                b'\t' => output.push_str("\\t"),
                b'\x0b' => output.push_str("\\v"),
                b'"' => output.push_str("\\\""),
                b'\\' => output.push_str("\\\\"),
                _ => {
                    let _ = write!(output, "\\{byte:o}");
                    last_was_numeric_escape = true;
                }
            }
        } else {
            output.push(char::from(*byte));
        }
    }
    output.push('"');
    output
}
