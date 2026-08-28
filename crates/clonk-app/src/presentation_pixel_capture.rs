use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::presentation_captures::ComparisonTerm;

const CAPTURE_ENABLED_ENV: &str = "CLONK_PRESENTATION_CAPTURE";
const CAPTURE_RUN_ID_ENV: &str = "CLONK_PRESENTATION_RUN_ID";
const CAPTURE_NONCE_ENV: &str = "CLONK_PRESENTATION_NONCE";
const CAPTURE_CASE_ENV: &str = "CLONK_PRESENTATION_CASE";
const CAPTURE_OUTPUT_DIR_ENV: &str = "CLONK_PRESENTATION_OUTPUT_DIR";
const CAPTURE_RECEIPT_ENV: &str = "CLONK_PRESENTATION_RECEIPT";
const CAPTURE_DISCOVER_ENV: &str = "CLONK_PRESENTATION_DISCOVER";
const CAPTURE_SOURCE_IDENTITY_ENV: &str = "CLONK_PRESENTATION_SOURCE_IDENTITY";
const CAPABILITIES_ENV: &str = "CLONK_PRESENTATION_CAPABILITIES";
const COMPARE_ENABLED_ENV: &str = "CLONK_PRESENTATION_COMPARE";
const COMPARE_REFERENCE_ENV: &str = "CLONK_PRESENTATION_REFERENCE";
const COMPARE_ACTUAL_ENV: &str = "CLONK_PRESENTATION_ACTUAL";
const CANONICAL_CONFIG_SHA256: &str =
    "8e7351443514744d638c5af2c4d534b85f2791ad1b0759d72822aa257c21e1bb";
const CANONICAL_PLAYER_SHA256: &str =
    "8dcaf794355d1f8d7e8dfa3efa76b8f601a8a911561d161a3da8ead2a40cd5c0";
const CANONICAL_NETWORK_REFERENCES_SHA256: &str =
    "922c7ccf941069bafd38a18e3ed71a747eadaa0c2a037b4e34422ce7312c8bf4";
const PRESENTATION_RNG_ALGORITHM: &str = "darwin-libc-rand-park-miller-v1";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub(crate) fn capture_or_discovery_requested() -> bool {
    std::env::var_os(CAPTURE_ENABLED_ENV).is_some()
        || std::env::var_os(CAPTURE_DISCOVER_ENV).is_some()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PresentationComparisonRequest {
    case_id: String,
    reference_path: PathBuf,
    actual_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PresentationSourceIdentity {
    schema: String,
    commit: String,
    tree: String,
    content_tree: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PresentationCaptureRequest {
    run_id: String,
    launcher_nonce: String,
    case_id: String,
    candidate_root: PathBuf,
    output_dir: PathBuf,
    receipt_path: PathBuf,
    source_identity_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PresentationLaunchRequest {
    Capture(PresentationCaptureRequest),
    Discover(PresentationCaptureCase),
}

const CANONICAL_CASE_IDS: [&str; 13] = [
    "startup-main",
    "startup-scenario-selection",
    "startup-network-browser",
    "startup-player-selection",
    "startup-options",
    "startup-about",
    "network-lobby",
    "loader",
    "hud",
    "ingame-menu",
    "object-menu",
    "gameplay",
    "evaluation",
];
const PIXEL_CASES: [PixelCaptureCase; 7] = [
    PixelCaptureCase::NetworkLobby,
    PixelCaptureCase::Loader,
    PixelCaptureCase::Hud,
    PixelCaptureCase::IngameMenu,
    PixelCaptureCase::ObjectMenu,
    PixelCaptureCase::Gameplay,
    PixelCaptureCase::Evaluation,
];

const LAYOUT_CASES: [LayoutCaptureCase; 6] = [
    LayoutCaptureCase::Main,
    LayoutCaptureCase::ScenarioSelection,
    LayoutCaptureCase::NetworkBrowser,
    LayoutCaptureCase::PlayerSelection,
    LayoutCaptureCase::Options,
    LayoutCaptureCase::About,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PresentationCaptureCase {
    Layout(LayoutCaptureCase),
    Pixel(PixelCaptureCase),
}

impl PresentationCaptureCase {
    fn from_id(id: &str) -> Option<Self> {
        LayoutCaptureCase::from_id(id)
            .map(Self::Layout)
            .or_else(|| PixelCaptureCase::from_id(id).map(Self::Pixel))
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Layout(case) => case.id(),
            Self::Pixel(case) => case.id(),
        }
    }
}

impl From<LayoutCaptureCase> for PresentationCaptureCase {
    fn from(value: LayoutCaptureCase) -> Self {
        Self::Layout(value)
    }
}

impl From<PixelCaptureCase> for PresentationCaptureCase {
    fn from(value: PixelCaptureCase) -> Self {
        Self::Pixel(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutCaptureCase {
    Main,
    ScenarioSelection,
    NetworkBrowser,
    PlayerSelection,
    Options,
    About,
}

impl LayoutCaptureCase {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "startup-main" => Some(Self::Main),
            "startup-scenario-selection" => Some(Self::ScenarioSelection),
            "startup-network-browser" => Some(Self::NetworkBrowser),
            "startup-player-selection" => Some(Self::PlayerSelection),
            "startup-options" => Some(Self::Options),
            "startup-about" => Some(Self::About),
            _ => None,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Main => "startup-main",
            Self::ScenarioSelection => "startup-scenario-selection",
            Self::NetworkBrowser => "startup-network-browser",
            Self::PlayerSelection => "startup-player-selection",
            Self::Options => "startup-options",
            Self::About => "startup-about",
        }
    }

    const fn startup_screen(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::ScenarioSelection => "scen",
            Self::NetworkBrowser => "net",
            Self::PlayerSelection => "plrsel",
            Self::Options => "options",
            Self::About => "about",
        }
    }

    fn checkpoint(self) -> String {
        format!("{}/fade-complete", self.id())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PixelCaptureCase {
    NetworkLobby,
    Loader,
    Hud,
    IngameMenu,
    ObjectMenu,
    Gameplay,
    Evaluation,
}

impl PixelCaptureCase {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "network-lobby" => Some(Self::NetworkLobby),
            "loader" => Some(Self::Loader),
            "hud" => Some(Self::Hud),
            "ingame-menu" => Some(Self::IngameMenu),
            "object-menu" => Some(Self::ObjectMenu),
            "gameplay" => Some(Self::Gameplay),
            "evaluation" => Some(Self::Evaluation),
            _ => None,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::NetworkLobby => "network-lobby",
            Self::Loader => "loader",
            Self::Hud => "hud",
            Self::IngameMenu => "ingame-menu",
            Self::ObjectMenu => "object-menu",
            Self::Gameplay => "gameplay",
            Self::Evaluation => "evaluation",
        }
    }

    const fn trigger(self) -> &'static str {
        match self {
            Self::NetworkLobby => "network-host-lobby-v1",
            Self::Loader => "scenario-loader-v1",
            Self::Hud => "tutorial01-frame-v1",
            Self::IngameMenu => "tutorial01-activate-menu-main-v1",
            Self::ObjectMenu => "tutorial03-object-menu-input-v1",
            Self::Gameplay => "tutorial02-frame-v1",
            Self::Evaluation => "tutorial01-game-over-v1",
        }
    }

    const fn checkpoint(self) -> &'static str {
        match self {
            Self::NetworkLobby => "lobby-notice-acknowledged/render-ordinal-2",
            Self::Loader => "loader-progress-60-fixed-log/render-ordinal-2",
            Self::Hud => "state-frame-90/render-ordinal-1",
            Self::IngameMenu => "state-frame-90/render-ordinal-2",
            Self::ObjectMenu => "state-frame-410/tooltip-render-ordinal-90",
            Self::Gameplay => "state-frame-180/render-ordinal-1",
            Self::Evaluation => "state-frame-90/evaluation-open/render-ordinal-2",
        }
    }

    const fn frame(self) -> u64 {
        match self {
            Self::NetworkLobby | Self::Loader => 2,
            Self::Hud | Self::IngameMenu | Self::Evaluation => 90,
            Self::ObjectMenu => 410,
            Self::Gameplay => 180,
        }
    }

    const fn scenario(self) -> &'static str {
        match self {
            Self::ObjectMenu => "Tutorial.c4f/Tutorial03.c4s",
            Self::Gameplay => "Tutorial.c4f/Tutorial02.c4s",
            Self::NetworkLobby | Self::Loader | Self::Hud | Self::IngameMenu | Self::Evaluation => {
                "Tutorial.c4f/Tutorial01.c4s"
            }
        }
    }

    const fn render_ordinal(self) -> u32 {
        match self {
            Self::NetworkLobby | Self::Loader | Self::IngameMenu | Self::Evaluation => 2,
            Self::Hud | Self::Gameplay => 1,
            Self::ObjectMenu => 90,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct EngineConfigHashes {
    cpp: String,
    rust: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureLocale {
    language: String,
    charset: String,
    lang: String,
    lc_all: String,
    tz: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureSimulationSeedState {
    seed: u64,
    calls: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CapturePresentationSeedState {
    algorithm: String,
    seed: u64,
    calls: u64,
    trace_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureSeeds {
    simulation: CaptureSimulationSeedState,
    presentation: CapturePresentationSeedState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureTrigger {
    id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureScenario {
    path: Option<String>,
    content_tree: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureFrame {
    checkpoint: String,
    number: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RuntimeResourceIdentity {
    tree: String,
    manifest_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct EngineRuntimeResources {
    graphics: RuntimeResourceIdentity,
    system: RuntimeResourceIdentity,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaseRuntimeResources {
    cpp: EngineRuntimeResources,
    rust: EngineRuntimeResources,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkReferencesFixture {
    schema: String,
    references: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureCaseSpec {
    id: String,
    comparison: String,
    port_asset_exemptions: BTreeMap<String, String>,
    config_sha256: EngineConfigHashes,
    player_sha256: String,
    locale: CaptureLocale,
    seeds: CaptureSeeds,
    trigger: CaptureTrigger,
    scenario: CaptureScenario,
    frame: CaptureFrame,
    runtime_resources: CaseRuntimeResources,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct CaptureArtifactRecord {
    path: String,
    sha256: String,
    size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct PresentationEngineReceipt {
    schema: &'static str,
    run_id: String,
    launcher_nonce: String,
    engine: &'static str,
    producer: &'static str,
    case_id: String,
    binary_sha256: String,
    source_tree: String,
    content_tree: String,
    profile: &'static str,
    config_sha256: String,
    player_sha256: String,
    network_references_sha256: String,
    locale: CaptureLocale,
    seeds: CaptureSeeds,
    trigger: CaptureTrigger,
    scenario: CaptureScenario,
    frame: CaptureFrame,
    runtime_resources: EngineRuntimeResources,
    artifacts: BTreeMap<String, CaptureArtifactRecord>,
}

fn build_engine_receipt(
    request: &PresentationCaptureRequest,
    spec: &CaptureCaseSpec,
    identity: &PresentationSourceIdentity,
    binary_sha256: String,
    artifacts: BTreeMap<String, CaptureArtifactRecord>,
    checkpoint: crate::presentation_pixel_startup::StartupPixelCheckpoint,
    presentation_report: clonk_engine::particles::PresentationSafeRandomCaptureReport,
    runtime_resources: EngineRuntimeResources,
) -> Result<PresentationEngineReceipt> {
    let case = PresentationCaptureCase::from_id(&request.case_id).ok_or_else(|| {
        anyhow::anyhow!("unknown presentation capture case {:?}", request.case_id)
    })?;
    anyhow::ensure!(
        spec.id == request.case_id,
        "case spec does not match capture request"
    );
    anyhow::ensure!(
        identity.content_tree == spec.scenario.content_tree,
        "source identity content tree does not match the trusted case spec"
    );
    anyhow::ensure!(
        checkpoint.simulation_seed == spec.seeds.simulation.seed
            && checkpoint.random_count == spec.seeds.simulation.calls,
        "observed simulation seed/RandomCount does not match the trusted case spec"
    );
    anyhow::ensure!(
        spec.seeds.presentation.algorithm == PRESENTATION_RNG_ALGORITHM
            && spec.seeds.presentation.seed == 587
            && presentation_report.calls == spec.seeds.presentation.calls
            && sha256_bytes(&presentation_report.trace)? == spec.seeds.presentation.trace_sha256,
        "observed presentation RNG identity does not match the trusted case spec"
    );
    anyhow::ensure!(
        presentation_report.raw_calls == 0,
        "presentation capture reached an unaudited direct raw random call"
    );
    anyhow::ensure!(
        runtime_resources == spec.runtime_resources.rust,
        "observed runtime resources do not match the trusted case spec"
    );
    anyhow::ensure!(
        checkpoint.render_ordinal
            == match case {
                PresentationCaptureCase::Layout(_) => 2,
                PresentationCaptureCase::Pixel(case) => case.render_ordinal(),
            },
        "render ordinal does not match the typed case checkpoint"
    );
    anyhow::ensure!(
        is_lower_hex(&binary_sha256, 64),
        "capture binary SHA-256 is invalid"
    );
    anyhow::ensure!(
        artifacts.keys().map(String::as_str).collect::<Vec<_>>()
            == match case {
                PresentationCaptureCase::Layout(_) => vec!["layout", "png"],
                PresentationCaptureCase::Pixel(_) => vec!["png"],
            },
        "capture artifact set does not match the typed case"
    );
    for (kind, artifact) in &artifacts {
        let suffix = if kind == "layout" {
            ".layout.json"
        } else {
            ".png"
        };
        anyhow::ensure!(
            artifact.path
                == format!(
                    "{}/rust/artifacts/{}{suffix}",
                    request.run_id, request.case_id
                )
                && is_lower_hex(&artifact.sha256, 64),
            "capture {kind} artifact record is not bound to the requested run/case"
        );
    }
    Ok(PresentationEngineReceipt {
        schema: "clonk-rs/presentation-engine-receipt/v2",
        run_id: request.run_id.clone(),
        launcher_nonce: request.launcher_nonce.clone(),
        engine: "rust",
        producer: "clonk-rs-capture-driver-v1",
        case_id: request.case_id.clone(),
        binary_sha256,
        source_tree: identity.tree.clone(),
        content_tree: spec.scenario.content_tree.clone(),
        profile: "legacy-clonk",
        config_sha256: spec.config_sha256.rust.clone(),
        player_sha256: spec.player_sha256.clone(),
        network_references_sha256: CANONICAL_NETWORK_REFERENCES_SHA256.to_owned(),
        locale: spec.locale.clone(),
        seeds: spec.seeds.clone(),
        trigger: spec.trigger.clone(),
        scenario: spec.scenario.clone(),
        frame: spec.frame.clone(),
        runtime_resources,
        artifacts,
    })
}

fn trusted_case_spec_from_bytes(
    bytes: &[u8],
    case: impl Into<PresentationCaptureCase>,
) -> Result<CaptureCaseSpec> {
    let case = case.into();
    let specs: Vec<CaptureCaseSpec> = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        specs.len() == CANONICAL_CASE_IDS.len(),
        "trusted case contract must contain exactly {} cases",
        CANONICAL_CASE_IDS.len()
    );
    for ((spec, expected_id), screen) in specs
        .iter()
        .zip(CANONICAL_CASE_IDS)
        .zip(crate::presentation_captures::screens())
    {
        anyhow::ensure!(
            spec.id == expected_id && spec.id == screen.id,
            "trusted case contract order/identity drift at {:?}",
            expected_id
        );
        let expected_comparison = match screen.comparison {
            ComparisonTerm::Layout => "layout",
            ComparisonTerm::Pixel => "pixel",
        };
        anyhow::ensure!(
            spec.comparison == expected_comparison,
            "{} comparison term differs from the capture manifest",
            spec.id
        );
        let expected_port_asset_exemptions =
            crate::presentation_layout::expected_port_asset_exemptions(&spec.id)
                .ok_or_else(|| anyhow::anyhow!("unknown presentation screen {:?}", spec.id))?;
        anyhow::ensure!(
            spec.port_asset_exemptions == expected_port_asset_exemptions,
            "{} port asset exemptions differ from the exact path/type contract",
            spec.id
        );
        anyhow::ensure!(
            spec.port_asset_exemptions
                .values()
                .all(|asset| screen.port_assets.iter().any(|allowed| allowed == asset)),
            "{} port asset exemption class differs from the capture manifest",
            spec.id
        );
        anyhow::ensure!(
            spec.config_sha256.cpp == CANONICAL_CONFIG_SHA256
                && spec.config_sha256.rust == CANONICAL_CONFIG_SHA256,
            "{} config SHA-256 does not bind the canonical native bytes",
            spec.id
        );
        anyhow::ensure!(
            spec.player_sha256 == CANONICAL_PLAYER_SHA256,
            "{} player SHA-256 does not bind Presentation.c4p",
            spec.id
        );
        anyhow::ensure!(
            spec.locale.language == "US"
                && spec.locale.charset == "Windows-1252"
                && spec.locale.lang == "C"
                && spec.locale.lc_all == "C"
                && spec.locale.tz == "UTC",
            "{} locale is not the canonical presentation locale",
            spec.id
        );
        anyhow::ensure!(
            spec.seeds.simulation.seed == 587 && spec.seeds.presentation.seed == 587,
            "{} seeds are not pinned to 587",
            spec.id
        );
        anyhow::ensure!(
            spec.seeds.presentation.algorithm == PRESENTATION_RNG_ALGORITHM
                && is_lower_hex(&spec.seeds.presentation.trace_sha256, 64)
                && (spec.seeds.presentation.calls != 0
                    || spec.seeds.presentation.trace_sha256 == EMPTY_SHA256),
            "{} presentation RNG contract is invalid",
            spec.id
        );
        for (engine, resources) in [
            ("cpp", &spec.runtime_resources.cpp),
            ("rust", &spec.runtime_resources.rust),
        ] {
            for (group, identity) in [
                ("graphics", &resources.graphics),
                ("system", &resources.system),
            ] {
                anyhow::ensure!(
                    is_lower_hex(&identity.tree, 40) && is_lower_hex(&identity.manifest_sha256, 64),
                    "{} {engine} {group} runtime resource identity is invalid",
                    spec.id
                );
            }
        }
        anyhow::ensure!(
            is_lower_hex(&spec.scenario.content_tree, 40),
            "{} content tree must be 40 lowercase hexadecimal characters",
            spec.id
        );
        anyhow::ensure!(
            !spec.trigger.id.trim().is_empty() && !spec.frame.checkpoint.trim().is_empty(),
            "{} trigger and checkpoint must be nonempty",
            spec.id
        );
    }
    let spec = specs
        .into_iter()
        .find(|spec| spec.id == case.id())
        .ok_or_else(|| anyhow::anyhow!("trusted case contract does not contain {}", case.id()))?;
    match case {
        PresentationCaptureCase::Layout(case) => {
            let checkpoint = case.checkpoint();
            anyhow::ensure!(
                spec.comparison == "layout",
                "{} must use the strict layout term",
                case.id()
            );
            anyhow::ensure!(
                spec.trigger.id == "direct-startup-dialog",
                "{} trigger is {:?}, expected direct-startup-dialog",
                case.id(),
                spec.trigger.id
            );
            anyhow::ensure!(
                spec.scenario.path.is_none(),
                "{} startup capture must not name a scenario",
                case.id()
            );
            anyhow::ensure!(
                spec.frame.checkpoint == checkpoint && spec.frame.number == 2,
                "{} frame/checkpoint differs from the direct startup contract",
                case.id()
            );
        }
        PresentationCaptureCase::Pixel(case) => {
            anyhow::ensure!(
                spec.comparison == "pixel" && spec.port_asset_exemptions.is_empty(),
                "{} must use the strict pixel term without port assets",
                case.id()
            );
            anyhow::ensure!(
                spec.trigger.id == case.trigger(),
                "{} trigger is {:?}, expected {:?}",
                case.id(),
                spec.trigger.id,
                case.trigger()
            );
            anyhow::ensure!(
                spec.scenario.path.as_deref() == Some(case.scenario()),
                "{} scenario is {:?}, expected {:?}",
                case.id(),
                spec.scenario.path,
                case.scenario()
            );
            anyhow::ensure!(
                spec.frame.checkpoint == case.checkpoint(),
                "{} checkpoint is {:?}, expected {:?}",
                case.id(),
                spec.frame.checkpoint,
                case.checkpoint()
            );
            anyhow::ensure!(
                spec.frame.number == case.frame(),
                "{} frame is {:?}, expected {:?}",
                case.id(),
                spec.frame.number,
                case.frame()
            );
        }
    }
    Ok(spec)
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn source_identity_from_bytes(bytes: &[u8]) -> Result<PresentationSourceIdentity> {
    let identity: PresentationSourceIdentity = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        identity.schema == "clonk-rs/presentation-source-identity/v1",
        "unsupported presentation source identity schema {:?}",
        identity.schema
    );
    for (field, value) in [
        ("commit", identity.commit.as_str()),
        ("tree", identity.tree.as_str()),
        ("content_tree", identity.content_tree.as_str()),
    ] {
        anyhow::ensure!(
            is_lower_hex(value, 40),
            "presentation source identity {field} must be 40 lowercase hexadecimal characters"
        );
    }
    Ok(identity)
}

fn stage_tutorial_checkpoint(
    app: &mut crate::GameApp,
    scenario: crate::FrontendScenario,
    case: PixelCaptureCase,
) -> Result<crate::presentation_pixel_startup::StartupPixelCheckpoint> {
    anyhow::ensure!(
        !matches!(
            case,
            PixelCaptureCase::NetworkLobby | PixelCaptureCase::Loader
        ),
        "{} is not a tutorial runtime checkpoint",
        case.id()
    );
    app.start_scenario(scenario)
        .map_err(|error| anyhow::anyhow!("start {}: {error}", case.scenario()))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while app.mode != crate::AppMode::Running {
        app.update()
            .map_err(|error| anyhow::anyhow!("load {}: {error}", case.scenario()))?;
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "{} did not enter the running state",
            case.scenario()
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    app.discard_terminal_loader_frame_for_headless_render();

    while app.engine.frame() < case.frame() {
        if case == PixelCaptureCase::ObjectMenu {
            use clonk_engine::{ControlButton, ControlEvent};
            let events: &[ControlEvent] = match app.engine.frame() {
                360 => &[ControlEvent::Press(ControlButton::Right)],
                365 => &[
                    ControlEvent::Release(ControlButton::Right),
                    ControlEvent::Press(ControlButton::Up),
                ],
                366 => &[ControlEvent::Release(ControlButton::Up)],
                _ => &[],
            };
            for event in events {
                app.dispatch_control_event_for_owner(app.players.local_owner, *event)
                    .map_err(|error| anyhow::anyhow!("dispatch object-menu input: {error}"))?;
            }
        }
        let before = app.engine.frame();
        app.update().map_err(|error| {
            anyhow::anyhow!("advance {} from frame {before}: {error}", case.id())
        })?;
        anyhow::ensure!(
            app.mode == crate::AppMode::Running && app.engine.frame() == before + 1,
            "{} did not advance exactly one real simulation frame from {before}",
            case.id()
        );
    }
    anyhow::ensure!(
        app.engine.frame() == case.frame(),
        "{} overshot its exact frame checkpoint",
        case.id()
    );

    match case {
        PixelCaptureCase::IngameMenu => {
            app.activate_ingame_main_menu_for_player(app.players.local_owner)
                .map_err(|error| anyhow::anyhow!("activate real in-game main menu: {error}"))?;
            anyhow::ensure!(
                app.ingame_menu.contains(app.players.local_owner),
                "real in-game main menu did not remain active"
            );
        }
        PixelCaptureCase::ObjectMenu => {
            let owner = app.players.local_owner;
            let player = app.engine.player(owner);
            let preferences = player.map(|player| player.control_style_preferences());
            let cursor = player.and_then(|player| player.cursor());
            let container = cursor
                .and_then(|cursor| app.snapshot.object(cursor))
                .and_then(|object| object.container);
            anyhow::ensure!(
                app.engine.cursor_object_menu(owner).is_some(),
                "Tutorial03 inputs did not open the real engine object menu: owner={owner}, preferences={preferences:?}, cursor={cursor:?}, container={container:?}"
            );
        }
        PixelCaptureCase::Evaluation => {
            anyhow::ensure!(
                app.engine.request_game_over_from_control()?,
                "Tutorial01 did not enter the real game-over path"
            );
            app.snapshot = app.engine.snapshot();
            app.handle_game_over()
                .map_err(|error| anyhow::anyhow!("open real evaluation dialog: {error}"))?;
            anyhow::ensure!(
                app.game_over_dialog.is_some(),
                "real game-over path did not open evaluation"
            );
        }
        PixelCaptureCase::Hud | PixelCaptureCase::Gameplay => {}
        PixelCaptureCase::NetworkLobby | PixelCaptureCase::Loader => unreachable!(),
    }

    let random_count = u64::try_from(app.engine.sync_check(0).random_count)
        .map_err(|_| anyhow::anyhow!("simulation RandomCount became negative"))?;
    Ok(crate::presentation_pixel_startup::StartupPixelCheckpoint {
        simulation_seed: app.engine.random_seed(),
        random_count,
        render_ordinal: case.render_ordinal(),
    })
}

fn render_checkpoint_png(app: &mut crate::GameApp, render_ordinal: u32) -> Result<Vec<u8>> {
    anyhow::ensure!(render_ordinal > 0, "render ordinal must be positive");
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    anyhow::ensure!(
        (width, height) == (1280, 720),
        "presentation render surface is {width}x{height}, expected 1280x720"
    );
    let mut frame = vec![0; width as usize * height as usize * 4];
    for ordinal in 1..=render_ordinal {
        app.render(&mut frame)
            .map_err(|error| anyhow::anyhow!("render presentation ordinal {ordinal}: {error}"))?;
    }
    crate::encode_rgba_png(width, height, &frame)
        .map_err(|error| anyhow::anyhow!("encode presentation PNG: {error}"))
}

fn stage_pixel_checkpoint(
    app: &mut crate::GameApp,
    case: PixelCaptureCase,
) -> Result<crate::presentation_pixel_startup::StartupPixelCheckpoint> {
    let scenario = crate::resolve_next_mission_scenario(&app.scensel.catalog, case.scenario())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "trusted presentation scenario is unavailable: {}",
                case.scenario()
            )
        })?;
    let content_root = app
        .app_paths
        .as_ref()
        .and_then(|paths| paths.content_dir())
        .ok_or_else(|| {
            anyhow::anyhow!("{} requires an explicit canonical content root", case.id())
        })?;
    let resolved_path = scenario
        .path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{} resolved scenario has no physical source", case.id()))?;
    validate_canonical_scenario_source(
        content_root,
        case.scenario(),
        resolved_path,
        &scenario.source_paths,
    )?;
    match case {
        PixelCaptureCase::NetworkLobby => {
            crate::presentation_pixel_startup::stage_network_lobby_checkpoint(app, scenario)
        }
        PixelCaptureCase::Loader => {
            crate::presentation_pixel_startup::stage_loader_checkpoint(app, scenario)
        }
        PixelCaptureCase::Hud
        | PixelCaptureCase::IngameMenu
        | PixelCaptureCase::ObjectMenu
        | PixelCaptureCase::Gameplay
        | PixelCaptureCase::Evaluation => stage_tutorial_checkpoint(app, scenario, case),
    }
}

fn validate_canonical_scenario_source(
    content_root: &Path,
    expected_relative: &str,
    resolved_path: &Path,
    source_paths: &[PathBuf],
) -> Result<()> {
    anyhow::ensure!(
        content_root.is_absolute(),
        "canonical content root must be absolute"
    );
    let root_metadata = std::fs::symlink_metadata(content_root).map_err(|error| {
        anyhow::anyhow!(
            "cannot inspect canonical content root {}: {error}",
            content_root.display()
        )
    })?;
    anyhow::ensure!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "canonical content root must be a regular non-symlink directory"
    );
    let relative = Path::new(expected_relative);
    anyhow::ensure!(
        !expected_relative.contains('\\')
            && !relative.is_absolute()
            && relative
                .components()
                .all(|component| { matches!(component, std::path::Component::Normal(_)) }),
        "canonical scenario path must be an exact normalized relative path"
    );

    let canonical_root = std::fs::canonicalize(content_root)?;
    anyhow::ensure!(
        canonical_root == content_root,
        "canonical content root must already be normalized"
    );
    let expected_path = content_root.join(relative);
    let mut component_path = content_root.to_path_buf();
    for component in relative.components() {
        component_path.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&component_path).map_err(|error| {
            anyhow::anyhow!(
                "canonical scenario component {} is unavailable: {error}",
                component_path.display()
            )
        })?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "canonical scenario source contains a symlink at {}",
            component_path.display()
        );
    }
    let expected_path = std::fs::canonicalize(expected_path)?;
    anyhow::ensure!(
        expected_path.starts_with(&canonical_root),
        "canonical scenario escaped the canonical content root"
    );
    let resolved_path = std::fs::canonicalize(resolved_path)?;
    anyhow::ensure!(
        resolved_path == expected_path,
        "resolved scenario did not come from the canonical content root"
    );
    anyhow::ensure!(
        source_paths.len() == 1,
        "resolved scenario has alternate catalog contributors"
    );
    let source_path = std::fs::canonicalize(&source_paths[0])?;
    anyhow::ensure!(
        source_path == expected_path,
        "resolved scenario has an alternate catalog contributor outside the canonical content root"
    );
    Ok(())
}

struct LayoutCaptureOutput {
    png: Vec<u8>,
    layout: Vec<u8>,
    checkpoint: crate::presentation_pixel_startup::StartupPixelCheckpoint,
}

fn validate_canonical_player_selection(app: &crate::GameApp) -> Result<()> {
    let paths = app
        .app_paths
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("startup-player-selection has no application paths"))?;
    let expected_path = paths.install_root().join("Presentation.c4p");
    let player = read_regular_non_symlink(&expected_path, "canonical Presentation player")?;
    anyhow::ensure!(
        player.len() == 235 && sha256_bytes(&player)? == CANONICAL_PLAYER_SHA256,
        "canonical Presentation player bytes differ from the audited packed fixture"
    );
    anyhow::ensure!(
        app.startup.player_files.len() == 1
            && app.startup.player_models.len() == 1
            && app.startup.player_files[0].path == expected_path
            && app.startup.player_files[0].file_name == "Presentation.c4p"
            && app.startup.player_models[0].name == "Presentation Host"
            && app.startup.player_models[0].activated,
        "startup-player-selection must contain exactly one active canonical Presentation player"
    );
    let controller = app
        .startup
        .player_dialog
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("startup-player-selection has no live controller"))?;
    anyhow::ensure!(
        controller.player_activations() == [true] && controller.selected_index() == Some(0),
        "startup-player-selection must select its one active canonical Presentation player"
    );
    Ok(())
}

fn stage_layout_checkpoint(
    app: &mut crate::GameApp,
    case: LayoutCaptureCase,
    network_references: &[u8],
) -> Result<crate::presentation_pixel_startup::StartupPixelCheckpoint> {
    app.apply_classic_startup_screen(case.startup_screen());
    let expected_view = match case {
        LayoutCaptureCase::Main => crate::StartupView::MainMenu,
        LayoutCaptureCase::ScenarioSelection => crate::StartupView::ScenarioBrowser,
        LayoutCaptureCase::NetworkBrowser => crate::StartupView::NetworkGame,
        LayoutCaptureCase::PlayerSelection => crate::StartupView::PlayerSelection,
        LayoutCaptureCase::Options => crate::StartupView::Options,
        LayoutCaptureCase::About => crate::StartupView::About,
    };
    anyhow::ensure!(
        app.mode == crate::AppMode::Menu && app.startup.view == expected_view,
        "{} did not open its real startup dialog",
        case.id()
    );
    if case == LayoutCaptureCase::ScenarioSelection {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.scensel.discovery.is_some() {
            app.poll_scenario_selector_discovery().map_err(|error| {
                anyhow::anyhow!("poll startup-scenario-selection discovery: {error}")
            })?;
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "startup-scenario-selection discovery did not finish"
            );
            std::thread::yield_now();
        }
    }
    if app.startup_dialog_fade_active() {
        let surface = app.graphics.surface();
        let mut frame = vec![0; surface.width() as usize * surface.height() as usize * 4];
        let mut steps = 0_u8;
        while app.startup_dialog_fade_active() {
            app.render(&mut frame)
                .map_err(|error| anyhow::anyhow!("settle {} fade: {error}", case.id()))?;
            steps = steps.saturating_add(1);
            anyhow::ensure!(
                steps <= crate::STARTUP_DIALOG_FADE_STEPS,
                "{} transition fade did not settle",
                case.id()
            );
        }
    }
    anyhow::ensure!(
        !app.startup_dialog_fade_active(),
        "{} direct startup route unexpectedly entered a transition fade",
        case.id()
    );
    if case == LayoutCaptureCase::NetworkBrowser {
        let fixture: NetworkReferencesFixture = serde_json::from_slice(network_references)?;
        anyhow::ensure!(
            fixture.schema == "clonk-rs/presentation-network-references/v1"
                && fixture.references.is_empty(),
            "startup-network-browser requires the canonical completed-empty reference fixture"
        );
        app.startup_game_search = None;
        app.apply_startup_game_search_event(
            clonk_network::StartupGameSearchEvent::ReferencesUpdated(Vec::new()),
        )?;
        anyhow::ensure!(
            app.startup_network_dialog.is_some(),
            "startup-network-browser has no live network dialog controller"
        );
    }
    if case == LayoutCaptureCase::PlayerSelection {
        validate_canonical_player_selection(app)?;
    }
    // The capture fork performs this at the start of every
    // C4Startup::Execute, after dialog construction and before the stable
    // render. Native changes RandomHold/RandomCount but not FRndBuf3.
    app.engine.pin_presentation_startup_random_state(587);
    let random_count = u64::try_from(app.engine.sync_check(0).random_count)
        .map_err(|_| anyhow::anyhow!("simulation RandomCount became negative"))?;
    Ok(crate::presentation_pixel_startup::StartupPixelCheckpoint {
        simulation_seed: app.engine.random_seed(),
        random_count,
        render_ordinal: 2,
    })
}

fn render_layout_capture(
    app: &mut crate::GameApp,
    case: LayoutCaptureCase,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    anyhow::ensure!(
        (width, height) == (1280, 720),
        "presentation render surface is {width}x{height}, expected 1280x720"
    );
    let mut frame = vec![0; width as usize * height as usize * 4];
    app.render(&mut frame)
        .map_err(|error| anyhow::anyhow!("render {} ordinal 1: {error}", case.id()))?;
    let mut presenter = clonk_scaling::FramePresenter::new(1.0, width, height);
    let refreshed = presenter
        .present(&mut frame, |logical| {
            app.render_ordered_native_base(logical)
        })
        .map_err(|error| anyhow::anyhow!("render {} ordinal 2: {error}", case.id()))?;
    anyhow::ensure!(refreshed, "{} ordinal 2 did not refresh", case.id());
    let commands = app
        .pending_native_presentation
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("{} semantic render produced no native plan", case.id()))?
        .batches
        .iter()
        .flat_map(|batch| batch.text.iter().cloned())
        .collect::<Vec<_>>();
    {
        let mut composer = presenter.ordered_composer(&mut frame);
        app.replay_pending_native_presentation(&mut composer)
            .map_err(|error| anyhow::anyhow!("replay {} ordinal 2: {error}", case.id()))?;
    }
    // The semantic commands and encoded pixels now come from the same second
    // full render, after its native-resolution layers have been composited.
    let png = crate::encode_rgba_png(width, height, &frame)
        .map_err(|error| anyhow::anyhow!("encode {} PNG: {error}", case.id()))?;
    let gui_fonts = app
        .assets
        .clonk_fonts
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{} has no live GUI fonts", case.id()))?;
    let trace =
        match case {
            LayoutCaptureCase::Main => {
                let logo = app
                    .assets
                    .logo()
                    .ok_or_else(|| anyhow::anyhow!("startup-main has no live logo"))?;
                crate::presentation_layout_producers::startup_main_trace(
                    &app.main_menu_state.menu,
                    &app.main_menu_state.participants_label,
                    (logo.width(), logo.height()),
                    &commands,
                    gui_fonts,
                )?
            }
            LayoutCaptureCase::ScenarioSelection => {
                let book_fonts = app.assets.book_fonts.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("startup-scenario-selection has no book fonts")
                })?;
                let layout = clonk_frontend::startup_scensel::scen_sel_layout(
                    width as i32,
                    height as i32,
                    gui_fonts,
                );
                let list_scrollbar_visible =
                    crate::scenario_list_scrollbar_visible(&app.menu_state, &layout, book_fonts);
                crate::presentation_layout_producers::startup_scenario_selection_trace(
                    list_scrollbar_visible,
                    &commands,
                    gui_fonts,
                    book_fonts,
                )?
            }
            LayoutCaptureCase::NetworkBrowser => {
                let controller = app.startup_network_dialog.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("startup-network-browser has no live controller")
                })?;
                crate::presentation_layout_producers::startup_network_browser_trace(
                    controller, &commands, gui_fonts,
                )?
            }
            LayoutCaptureCase::PlayerSelection => {
                let controller = app.startup.player_dialog.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("startup-player-selection has no live controller")
                })?;
                let book_fonts =
                    app.assets.plrsel_book_fonts.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("startup-player-selection has no book fonts")
                    })?;
                crate::presentation_layout_producers::startup_player_selection_trace(
                    controller, &commands, gui_fonts, book_fonts,
                )?
            }
            LayoutCaptureCase::Options => {
                let state =
                    app.startup.options_dialog.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("startup-options has no live dialog state")
                    })?;
                let book_fonts = app
                    .assets
                    .options_book_fonts
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("startup-options has no book fonts"))?;
                crate::presentation_layout_producers::startup_options_trace(
                    state, &commands, gui_fonts, book_fonts,
                )?
            }
            LayoutCaptureCase::About => {
                let state = app
                    .startup
                    .about_dialog
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("startup-about has no live dialog state"))?;
                crate::presentation_layout_producers::startup_about_trace(
                    state, &commands, gui_fonts,
                )?
            }
        };
    Ok((
        png,
        crate::presentation_layout_producers::serialize_layout_trace(&trace)?.into_bytes(),
    ))
}

fn capture_layout_case_with_app(
    app: &mut crate::GameApp,
    case: LayoutCaptureCase,
    network_references: &[u8],
) -> Result<LayoutCaptureOutput> {
    let checkpoint = stage_layout_checkpoint(app, case, network_references)?;
    let (png, layout) = render_layout_capture(app, case)?;
    Ok(LayoutCaptureOutput {
        png,
        layout,
        checkpoint,
    })
}

#[derive(Serialize)]
struct PresentationDiscovery {
    case: &'static str,
    simulation_seed: u64,
    random_count: u64,
    presentation_algorithm: &'static str,
    presentation_seed: u64,
    safe_random_calls: u64,
    safe_random_trace: String,
    safe_random_trace_sha256: String,
    direct_rand_calls: u64,
}

fn presentation_discovery(
    case: &'static str,
    checkpoint: crate::presentation_pixel_startup::StartupPixelCheckpoint,
) -> Result<PresentationDiscovery> {
    let report = clonk_engine::particles::presentation_safe_random_capture_report();
    let safe_random_trace_sha256 = sha256_bytes(&report.trace)?;
    let safe_random_trace = String::from_utf8(report.trace)
        .map_err(|error| anyhow::anyhow!("presentation RNG trace is not UTF-8: {error}"))?;
    Ok(PresentationDiscovery {
        case,
        simulation_seed: checkpoint.simulation_seed,
        random_count: checkpoint.random_count,
        presentation_algorithm: PRESENTATION_RNG_ALGORITHM,
        presentation_seed: 587,
        safe_random_calls: report.calls,
        safe_random_trace,
        safe_random_trace_sha256,
        direct_rand_calls: report.raw_calls,
    })
}

fn discover_pixel_case_with_app(
    app: &mut crate::GameApp,
    case: PixelCaptureCase,
    mut output: impl Write,
) -> Result<()> {
    let checkpoint = stage_pixel_checkpoint(app, case)?;
    let _png = render_checkpoint_png(app, checkpoint.render_ordinal)?;
    serde_json::to_writer(&mut output, &presentation_discovery(case.id(), checkpoint)?)?;
    writeln!(output)?;
    Ok(())
}

fn discover_capture_case_with_app(
    app: &mut crate::GameApp,
    case: PresentationCaptureCase,
    network_references: &[u8],
    mut output: impl Write,
) -> Result<()> {
    let checkpoint = match case {
        PresentationCaptureCase::Layout(case) => {
            capture_layout_case_with_app(app, case, network_references)?.checkpoint
        }
        PresentationCaptureCase::Pixel(case) => {
            let checkpoint = stage_pixel_checkpoint(app, case)?;
            let _png = render_checkpoint_png(app, checkpoint.render_ordinal)?;
            checkpoint
        }
    };
    serde_json::to_writer(&mut output, &presentation_discovery(case.id(), checkpoint)?)?;
    writeln!(output)?;
    Ok(())
}

fn discovery_network_references(
    app_paths: Option<&Arc<crate::AppPaths>>,
    case: PresentationCaptureCase,
) -> Result<Vec<u8>> {
    if case != PresentationCaptureCase::Layout(LayoutCaptureCase::NetworkBrowser) {
        return Ok(Vec::new());
    }
    let config_path = app_paths
        .ok_or_else(|| anyhow::anyhow!("presentation discovery requires an explicit config"))?
        .config_file();
    let inputs = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("presentation config has no input directory"))?;
    read_regular_non_symlink(
        &inputs.join("network-references.json"),
        "network references fixture",
    )
}

struct ValidatedCaptureInputs {
    spec: CaptureCaseSpec,
    identity: PresentationSourceIdentity,
    binary_sha256: String,
    network_references: Vec<u8>,
    runtime_resources: EngineRuntimeResources,
}

fn sha256_bytes(bytes: &[u8]) -> Result<String> {
    clonk_update::sha256_reader(Cursor::new(bytes))
        .map_err(|error| anyhow::anyhow!("compute SHA-256: {error}"))
}

fn lower_hex_bytes(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn git_object_id(kind: &str, contents: &[u8]) -> [u8; 20] {
    let mut framed = format!("{kind} {}\0", contents.len()).into_bytes();
    framed.extend_from_slice(contents);
    Sha1::digest(framed).into()
}

#[cfg(unix)]
fn runtime_resource_file_mode(metadata: &std::fs::Metadata) -> &'static [u8] {
    if metadata.permissions().mode() & 0o111 == 0 {
        b"100644"
    } else {
        b"100755"
    }
}

#[cfg(not(unix))]
fn runtime_resource_file_mode(_metadata: &std::fs::Metadata) -> &'static [u8] {
    b"100644"
}

fn runtime_resource_identity(directory: &Path) -> Result<RuntimeResourceIdentity> {
    struct TreeEntry {
        sort_name: Vec<u8>,
        mode: &'static [u8],
        name: Vec<u8>,
        object_id: [u8; 20],
    }

    fn visit(
        directory: &Path,
        relative: &str,
        manifest: &mut Vec<(String, String)>,
    ) -> Result<[u8; 20]> {
        require_directory(directory, "runtime resource directory")?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(directory).map_err(|error| {
            anyhow::anyhow!(
                "cannot enumerate runtime resource directory {}: {error}",
                directory.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                anyhow::anyhow!(
                    "cannot enumerate runtime resource directory {}: {error}",
                    directory.display()
                )
            })?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|name| anyhow::anyhow!("runtime resource name is not UTF-8: {name:?}"))?;
            anyhow::ensure!(
                !name.is_empty() && !name.contains(['/', '\0']),
                "runtime resource name is invalid: {name:?}"
            );
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                anyhow::anyhow!(
                    "cannot inspect runtime resource {}: {error}",
                    path.display()
                )
            })?;
            let relative_name = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let name_bytes = name.into_bytes();
            let (sort_name, mode, object_id) = if metadata.file_type().is_dir() {
                let mut sort_name = name_bytes.clone();
                sort_name.push(b'/');
                (
                    sort_name,
                    b"40000" as &'static [u8],
                    visit(&path, &relative_name, manifest)?,
                )
            } else if metadata.file_type().is_file() {
                let contents = std::fs::read(&path).map_err(|error| {
                    anyhow::anyhow!("cannot read runtime resource {}: {error}", path.display())
                })?;
                manifest.push((relative_name, sha256_bytes(&contents)?));
                (
                    name_bytes.clone(),
                    runtime_resource_file_mode(&metadata),
                    git_object_id("blob", &contents),
                )
            } else {
                anyhow::bail!(
                    "runtime resource contains a symlink or special entry: {}",
                    path.display()
                );
            };
            entries.push(TreeEntry {
                sort_name,
                mode,
                name: name_bytes,
                object_id,
            });
        }
        entries.sort_by(|left, right| left.sort_name.cmp(&right.sort_name));
        let mut payload = Vec::new();
        for entry in entries {
            payload.extend_from_slice(entry.mode);
            payload.push(b' ');
            payload.extend_from_slice(&entry.name);
            payload.push(0);
            payload.extend_from_slice(&entry.object_id);
        }
        Ok(git_object_id("tree", &payload))
    }

    let mut manifest = Vec::new();
    let tree = visit(directory, "", &mut manifest)?;
    anyhow::ensure!(
        !manifest.is_empty(),
        "runtime resource directory is empty: {}",
        directory.display()
    );
    manifest.sort_by(|left, right| left.0.cmp(&right.0));
    let mut canonical_manifest = Vec::new();
    for (path, digest) in manifest {
        canonical_manifest.extend_from_slice(format!("{digest}  {path}\n").as_bytes());
    }
    Ok(RuntimeResourceIdentity {
        tree: lower_hex_bytes(&tree),
        manifest_sha256: sha256_bytes(&canonical_manifest)?,
    })
}

fn require_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| anyhow::anyhow!("cannot inspect {label} {}: {error}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        "{label} must be a non-symlink directory: {}",
        path.display()
    );
    Ok(())
}

fn validate_fixed_runtime_inputs(
    classic: &crate::ClassicCommandLine,
    app_paths: Option<&Arc<crate::AppPaths>>,
    candidate_root: Option<&Path>,
) -> Result<()> {
    let app_paths = app_paths.ok_or_else(|| {
        anyhow::anyhow!("presentation capture requires the explicit canonical config")
    })?;
    let config_path = app_paths.config_file();
    anyhow::ensure!(
        config_path.is_absolute(),
        "presentation config path must be absolute"
    );
    let expected_config = candidate_root.map(|root| root.join("inputs/rust.config"));
    if let Some(expected) = expected_config.as_ref() {
        anyhow::ensure!(
            config_path == *expected,
            "presentation process did not consume staged inputs/rust.config"
        );
    }
    let config = read_regular_non_symlink(&config_path, "presentation config")?;
    anyhow::ensure!(
        sha256_bytes(&config)? == CANONICAL_CONFIG_SHA256,
        "presentation config SHA-256 differs from the canonical native bytes"
    );

    anyhow::ensure!(
        classic.player_files.len() == 1,
        "presentation capture requires exactly one player .c4p"
    );
    let player_path = &classic.player_files[0];
    anyhow::ensure!(
        player_path.is_absolute()
            && player_path.extension().and_then(|value| value.to_str()) == Some("c4p"),
        "presentation player must be one absolute lowercase .c4p path"
    );
    if let Some(root) = candidate_root {
        anyhow::ensure!(
            player_path == &root.join("inputs/Presentation.c4p"),
            "presentation player path differs from staged inputs/Presentation.c4p"
        );
    }
    let player = read_regular_non_symlink(player_path, "presentation player")?;
    anyhow::ensure!(
        player.len() == 235 && sha256_bytes(&player)? == CANONICAL_PLAYER_SHA256,
        "presentation player bytes differ from the canonical packed fixture"
    );
    if let Some(root) = candidate_root {
        let expected_install_root = root.join("work/rust-source");
        anyhow::ensure!(
            app_paths.install_root() == expected_install_root,
            "presentation install root differs from staged work/rust-source"
        );
        require_directory(app_paths.install_root(), "presentation install root")?;
        let discovery_path = app_paths.install_root().join("Presentation.c4p");
        let discovery_player =
            read_regular_non_symlink(&discovery_path, "presentation discovery player")?;
        anyhow::ensure!(
            discovery_player == player,
            "install-root Presentation.c4p differs from the exact CLI player bytes"
        );
    }
    let expected_classic = crate::ClassicCommandLine {
        player_files: vec![player_path.clone()],
        compat_profile: Some(crate::settings::CompatProfile::LegacyClonk),
        ..crate::ClassicCommandLine::default()
    };
    anyhow::ensure!(
        *classic == expected_classic,
        "presentation command line contains fields outside the fixed player/profile contract"
    );
    for (name, expected) in [
        ("LC_LANGUAGE", "US"),
        ("LANG", "C"),
        ("LC_ALL", "C"),
        ("TZ", "UTC"),
    ] {
        anyhow::ensure!(
            std::env::var_os(name).as_deref() == Some(std::ffi::OsStr::new(expected)),
            "{name} must be exactly {expected:?} for presentation capture"
        );
    }
    Ok(())
}

fn validate_capture_inputs(
    request: &PresentationCaptureRequest,
    classic: &crate::ClassicCommandLine,
    app_paths: Option<&Arc<crate::AppPaths>>,
) -> Result<ValidatedCaptureInputs> {
    validate_fixed_runtime_inputs(classic, app_paths, Some(&request.candidate_root))?;
    require_directory(&request.output_dir, "presentation artifact directory")?;
    let receipt_directory = request
        .receipt_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("presentation receipt has no parent directory"))?;
    require_directory(receipt_directory, "presentation receipt directory")?;
    let artifact_path = request.output_dir.join(format!("{}.png", request.case_id));
    let layout_path = request
        .output_dir
        .join(format!("{}.layout.json", request.case_id));
    anyhow::ensure!(
        !artifact_path.exists()
            && !request.receipt_path.exists()
            && (LayoutCaptureCase::from_id(&request.case_id).is_none() || !layout_path.exists()),
        "presentation artifact and receipt targets must be fresh"
    );

    let network_path = request
        .candidate_root
        .join("inputs/network-references.json");
    let network = read_regular_non_symlink(&network_path, "network references fixture")?;
    anyhow::ensure!(
        sha256_bytes(&network)? == CANONICAL_NETWORK_REFERENCES_SHA256,
        "network references fixture SHA-256 differs from the canonical bytes"
    );

    let identity_bytes = read_regular_non_symlink(
        &request.source_identity_path,
        "presentation source identity",
    )?;
    let identity = source_identity_from_bytes(&identity_bytes)?;

    let case_specs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("clonk-app belongs to the workspace")
        .join("compat/presentation/case_specs.json");
    anyhow::ensure!(
        case_specs_path.is_file() && !case_specs_path.is_symlink(),
        "presentation case contract not implemented: compat/presentation/case_specs.json"
    );
    let case_specs = std::fs::read(&case_specs_path).map_err(|error| {
        anyhow::anyhow!(
            "cannot read trusted case contract {}: {error}",
            case_specs_path.display()
        )
    })?;
    let case = PresentationCaptureCase::from_id(&request.case_id).ok_or_else(|| {
        anyhow::anyhow!("unknown presentation capture case {:?}", request.case_id)
    })?;
    let spec = trusted_case_spec_from_bytes(&case_specs, case)?;
    anyhow::ensure!(
        identity.content_tree == spec.scenario.content_tree,
        "source identity content tree differs from the trusted case contract"
    );

    let binary_path = std::env::current_exe()?;
    let binary = read_regular_non_symlink(&binary_path, "running capture executable")?;
    let binary_sha256 = sha256_bytes(&binary)?;
    let planet_dir = app_paths
        .ok_or_else(|| anyhow::anyhow!("presentation capture requires application paths"))?
        .planet_dir();
    let runtime_resources = EngineRuntimeResources {
        graphics: runtime_resource_identity(&planet_dir.join("Graphics.c4g"))?,
        system: runtime_resource_identity(&planet_dir.join("System.c4g"))?,
    };
    anyhow::ensure!(
        runtime_resources == spec.runtime_resources.rust,
        "running Rust runtime resources differ from the trusted case contract"
    );
    Ok(ValidatedCaptureInputs {
        spec,
        identity,
        binary_sha256,
        network_references: network,
        runtime_resources,
    })
}

struct PresentationRandomGuard;

impl PresentationRandomGuard {
    fn install() -> Self {
        crate::seed_classic_safe_random(587);
        clonk_engine::particles::install_presentation_safe_random_seed(587);
        clonk_engine::particles::begin_presentation_safe_random_capture();
        Self
    }
}

impl Drop for PresentationRandomGuard {
    fn drop(&mut self) {
        clonk_engine::particles::end_presentation_safe_random_capture();
        clonk_engine::particles::clear_presentation_safe_random_seed();
    }
}

fn boot_capture_app_to_menu(
    classic: &crate::ClassicCommandLine,
    app_paths: Option<&Arc<crate::AppPaths>>,
) -> Result<crate::GameApp> {
    let mut app = crate::build_capture_app(
        classic,
        app_paths,
        crate::RuntimeConfig {
            player_owner: 1,
            player_name: "Presentation Host".to_owned(),
            network: None,
            record_enabled: false,
        },
    )?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while app.mode != crate::AppMode::Menu {
        app.update()
            .map_err(|error| anyhow::anyhow!("boot presentation app: {error}"))?;
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "presentation app did not reach the startup menu"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(app)
}

fn write_new_file(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| anyhow::anyhow!("cannot create {label} {}: {error}", path.display()))?;
    output
        .write_all(bytes)
        .map_err(|error| anyhow::anyhow!("cannot write {label} {}: {error}", path.display()))?;
    output
        .sync_all()
        .map_err(|error| anyhow::anyhow!("cannot sync {label} {}: {error}", path.display()))
}

fn run_capture_request(
    request: &PresentationCaptureRequest,
    classic: &crate::ClassicCommandLine,
    app_paths: Option<&Arc<crate::AppPaths>>,
) -> Result<()> {
    let inputs = validate_capture_inputs(request, classic, app_paths)?;
    std::env::set_var("LC_PIN_SEED", "587");
    let _random = PresentationRandomGuard::install();
    let mut app = boot_capture_app_to_menu(classic, app_paths)?;
    let case = PresentationCaptureCase::from_id(&request.case_id).ok_or_else(|| {
        anyhow::anyhow!("unknown presentation capture case {:?}", request.case_id)
    })?;
    let (checkpoint, png, layout) = match case {
        PresentationCaptureCase::Layout(case) => {
            let output = capture_layout_case_with_app(&mut app, case, &inputs.network_references)?;
            (output.checkpoint, output.png, Some(output.layout))
        }
        PresentationCaptureCase::Pixel(case) => {
            let checkpoint = stage_pixel_checkpoint(&mut app, case)?;
            let png = render_checkpoint_png(&mut app, checkpoint.render_ordinal)?;
            (checkpoint, png, None)
        }
    };
    let presentation_report = clonk_engine::particles::presentation_safe_random_capture_report();
    let mut artifacts = BTreeMap::from([(
        "png".to_owned(),
        CaptureArtifactRecord {
            path: format!("{}/rust/artifacts/{}.png", request.run_id, request.case_id),
            sha256: sha256_bytes(&png)?,
            size_bytes: u64::try_from(png.len())?,
        },
    )]);
    if let Some(layout) = layout.as_ref() {
        artifacts.insert(
            "layout".to_owned(),
            CaptureArtifactRecord {
                path: format!(
                    "{}/rust/artifacts/{}.layout.json",
                    request.run_id, request.case_id
                ),
                sha256: sha256_bytes(layout)?,
                size_bytes: u64::try_from(layout.len())?,
            },
        );
    }
    let receipt = build_engine_receipt(
        request,
        &inputs.spec,
        &inputs.identity,
        inputs.binary_sha256,
        artifacts,
        checkpoint,
        presentation_report,
        inputs.runtime_resources,
    )?;
    let mut receipt_bytes = serde_json::to_vec(&receipt)?;
    receipt_bytes.push(b'\n');
    let artifact_path = request.output_dir.join(format!("{}.png", request.case_id));
    write_new_file(&artifact_path, &png, "presentation artifact")?;
    if let Some(layout) = layout {
        let layout_path = request
            .output_dir
            .join(format!("{}.layout.json", request.case_id));
        write_new_file(&layout_path, &layout, "presentation layout artifact")?;
    }
    write_new_file(
        &request.receipt_path,
        &receipt_bytes,
        "presentation engine receipt",
    )?;
    Ok(())
}

pub(crate) fn run_capture_or_discovery_from_environment(
    classic: &crate::ClassicCommandLine,
    app_paths: Option<&Arc<crate::AppPaths>>,
) -> Result<bool> {
    let environment = std::env::vars_os().collect::<BTreeMap<_, _>>();
    let Some(launch) = launch_from_values(&environment)? else {
        return Ok(false);
    };
    for incompatible in [
        CAPABILITIES_ENV,
        COMPARE_ENABLED_ENV,
        COMPARE_REFERENCE_ENV,
        COMPARE_ACTUAL_ENV,
    ] {
        anyhow::ensure!(
            !environment.contains_key(&OsString::from(incompatible)),
            "presentation capture/discovery cannot be combined with present {incompatible}"
        );
    }
    match launch {
        PresentationLaunchRequest::Capture(request) => {
            run_capture_request(&request, classic, app_paths)?;
        }
        PresentationLaunchRequest::Discover(case) => {
            for capture_only in [
                CAPTURE_RUN_ID_ENV,
                CAPTURE_NONCE_ENV,
                CAPTURE_CASE_ENV,
                CAPTURE_OUTPUT_DIR_ENV,
                CAPTURE_RECEIPT_ENV,
                CAPTURE_SOURCE_IDENTITY_ENV,
            ] {
                anyhow::ensure!(
                    !environment.contains_key(&OsString::from(capture_only)),
                    "{CAPTURE_DISCOVER_ENV} cannot be combined with present {capture_only}"
                );
            }
            validate_fixed_runtime_inputs(classic, app_paths, None)?;
            let network_references = discovery_network_references(app_paths, case)?;
            std::env::set_var("LC_PIN_SEED", "587");
            let _random = PresentationRandomGuard::install();
            let mut app = boot_capture_app_to_menu(classic, app_paths)?;
            discover_capture_case_with_app(
                &mut app,
                case,
                &network_references,
                std::io::stdout().lock(),
            )?;
        }
    }
    Ok(true)
}

fn request_from_values(
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Option<PresentationCaptureRequest>> {
    let value = |name: &str| {
        environment
            .get(&OsString::from(name))
            .and_then(|value| value.to_str())
    };
    match value(CAPTURE_ENABLED_ENV) {
        None => return Ok(None),
        Some("1") => {}
        Some(_) => anyhow::bail!("{CAPTURE_ENABLED_ENV} must be exactly 1"),
    }

    let required = |name: &str| {
        value(name)
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("{name} is required for presentation capture"))
    };
    let run_id = required(CAPTURE_RUN_ID_ENV)?;
    anyhow::ensure!(
        matches!(run_id.as_str(), "run-1" | "run-2"),
        "{CAPTURE_RUN_ID_ENV} must be run-1 or run-2"
    );
    let launcher_nonce = required(CAPTURE_NONCE_ENV)?;
    anyhow::ensure!(
        launcher_nonce.len() == 64
            && launcher_nonce
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{CAPTURE_NONCE_ENV} must be 64 lowercase hexadecimal characters"
    );
    let case_id = required(CAPTURE_CASE_ENV)?;
    anyhow::ensure!(
        PresentationCaptureCase::from_id(&case_id).is_some(),
        "{CAPTURE_CASE_ENV} is not a presentation capture case: {case_id:?}"
    );
    let output_dir = PathBuf::from(required(CAPTURE_OUTPUT_DIR_ENV)?);
    anyhow::ensure!(
        lexically_canonical_absolute(&output_dir),
        "{CAPTURE_OUTPUT_DIR_ENV} must be an absolute path without dot components"
    );
    let candidate_root = output_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| anyhow::anyhow!("{CAPTURE_OUTPUT_DIR_ENV} has no candidate root"))?
        .to_path_buf();
    anyhow::ensure!(
        output_dir == candidate_root.join(&run_id).join("rust").join("artifacts"),
        "{CAPTURE_OUTPUT_DIR_ENV} does not match the run artifact topology"
    );
    let receipt_path = PathBuf::from(required(CAPTURE_RECEIPT_ENV)?);
    anyhow::ensure!(
        receipt_path
            == candidate_root
                .join(&run_id)
                .join("rust")
                .join("receipts")
                .join(format!("{case_id}.json")),
        "{CAPTURE_RECEIPT_ENV} does not match the case receipt topology"
    );
    let source_identity_path = PathBuf::from(required(CAPTURE_SOURCE_IDENTITY_ENV)?);
    anyhow::ensure!(
        source_identity_path == candidate_root.join("inputs/rust-source-identity.json"),
        "{CAPTURE_SOURCE_IDENTITY_ENV} does not match the staged identity topology"
    );
    Ok(Some(PresentationCaptureRequest {
        run_id,
        launcher_nonce,
        case_id,
        candidate_root,
        output_dir,
        receipt_path,
        source_identity_path,
    }))
}

fn lexically_canonical_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

fn launch_from_values(
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Option<PresentationLaunchRequest>> {
    if let Some(case_id) = environment
        .get(&OsString::from(CAPTURE_DISCOVER_ENV))
        .and_then(|value| value.to_str())
    {
        anyhow::ensure!(
            !environment.contains_key(&OsString::from(CAPTURE_ENABLED_ENV)),
            "{CAPTURE_DISCOVER_ENV} cannot be combined with present {CAPTURE_ENABLED_ENV}"
        );
        let case = PresentationCaptureCase::from_id(case_id)
            .ok_or_else(|| anyhow::anyhow!("unknown presentation discovery case {case_id:?}"))?;
        return Ok(Some(PresentationLaunchRequest::Discover(case)));
    }
    request_from_values(environment).map(|request| request.map(PresentationLaunchRequest::Capture))
}

fn comparison_request_from_values(
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Option<PresentationComparisonRequest>> {
    let value = |name: &str| {
        environment
            .get(&OsString::from(name))
            .and_then(|value| value.to_str())
    };
    let Some(enabled) = value(COMPARE_ENABLED_ENV) else {
        return Ok(None);
    };
    anyhow::ensure!(enabled == "1", "{COMPARE_ENABLED_ENV} must be exactly 1");
    let required = |name: &str| {
        value(name)
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("{name} is required for presentation comparison"))
    };
    let case_id = required(CAPTURE_CASE_ENV)?;
    anyhow::ensure!(
        crate::presentation_captures::screens()
            .iter()
            .any(|screen| screen.id == case_id),
        "unknown presentation comparison case {case_id:?}"
    );
    let reference_path = PathBuf::from(required(COMPARE_REFERENCE_ENV)?);
    let actual_path = PathBuf::from(required(COMPARE_ACTUAL_ENV)?);
    anyhow::ensure!(
        reference_path.is_absolute() && actual_path.is_absolute(),
        "presentation comparison artifact paths must be absolute"
    );
    Ok(Some(PresentationComparisonRequest {
        case_id,
        reference_path,
        actual_path,
    }))
}

fn compare_artifact_bytes(case_id: &str, reference: &[u8], actual: &[u8]) -> Result<&'static str> {
    let case = PresentationCaptureCase::from_id(case_id)
        .ok_or_else(|| anyhow::anyhow!("unknown presentation comparison case {case_id:?}"))?;
    trusted_case_spec_from_bytes(
        include_bytes!("../../../compat/presentation/case_specs.json"),
        case,
    )?;
    let screen = crate::presentation_captures::screens()
        .iter()
        .find(|screen| screen.id == case_id)
        .ok_or_else(|| anyhow::anyhow!("unknown presentation comparison case {case_id:?}"))?;
    match screen.comparison {
        ComparisonTerm::Layout => {
            let reference = std::str::from_utf8(reference)
                .map_err(|error| anyhow::anyhow!("reference layout is not UTF-8: {error}"))?;
            let actual = std::str::from_utf8(actual)
                .map_err(|error| anyhow::anyhow!("actual layout is not UTF-8: {error}"))?;
            crate::presentation_layout::compare_layout_traces(case_id, reference, actual)
                .map_err(|mismatch| anyhow::anyhow!("layout mismatch: {mismatch:?}"))?;
            Ok("layout")
        }
        ComparisonTerm::Pixel => {
            let (reference_width, reference_height, reference) =
                decode_capture_png("reference", reference)?;
            let (actual_width, actual_height, actual) = decode_capture_png("actual", actual)?;
            anyhow::ensure!(
                (reference_width, reference_height) == (actual_width, actual_height),
                "capture PNG geometry differs: reference is {reference_width}x{reference_height}, actual is {actual_width}x{actual_height}"
            );
            crate::presentation_captures::compare_capture(
                case_id,
                // The reference side is LegacyClonk's native OpenGL
                // framebuffer readback. Select the manifest's GPU term for
                // the comparison pair even though Rust encodes its composed
                // software surface directly.
                crate::presentation_captures::CaptureSurface::Gpu,
                reference_width,
                reference_height,
                &reference,
                &actual,
            )
            .map_err(|mismatch| anyhow::anyhow!("pixel mismatch: {mismatch:?}"))?;
            Ok("pixel")
        }
    }
}

fn decode_capture_png(label: &str, bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| anyhow::anyhow!("cannot decode {label} PNG header: {error}"))?;
    anyhow::ensure!(
        reader.info().bit_depth == png::BitDepth::Eight,
        "{label} PNG must use 8-bit channels"
    );
    anyhow::ensure!(
        matches!(
            reader.info().color_type,
            png::ColorType::Rgb | png::ColorType::Rgba
        ),
        "{label} PNG must be RGB or RGBA"
    );
    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("{label} PNG output is too large"))?;
    let mut pixels = vec![0; output_size];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| anyhow::anyhow!("cannot decode {label} PNG pixels: {error}"))?;
    pixels.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        _ => anyhow::bail!("{label} PNG decoded to an unsupported color type"),
    };
    Ok((info.width, info.height, rgba))
}

fn read_regular_non_symlink(path: &Path, label: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(path.is_absolute(), "{label} path must be absolute");
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| anyhow::anyhow!("cannot inspect {label} {}: {error}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "{label} must be a regular non-symlink file: {}",
        path.display()
    );
    std::fs::read(path)
        .map_err(|error| anyhow::anyhow!("cannot read {label} {}: {error}", path.display()))
}

#[derive(Serialize)]
struct PresentationComparisonSuccess<'a> {
    schema: &'static str,
    case_id: &'a str,
    comparison: &'static str,
    status: &'static str,
}

fn run_comparison_request(
    request: &PresentationComparisonRequest,
    mut output: impl Write,
) -> Result<()> {
    let screen = crate::presentation_captures::screens()
        .iter()
        .find(|screen| screen.id == request.case_id)
        .ok_or_else(|| {
            anyhow::anyhow!("unknown presentation comparison case {:?}", request.case_id)
        })?;
    let suffix = match screen.comparison {
        ComparisonTerm::Layout => ".layout.json",
        ComparisonTerm::Pixel => ".png",
    };
    for (path, label) in [
        (&request.reference_path, "reference artifact"),
        (&request.actual_path, "actual artifact"),
    ] {
        anyhow::ensure!(
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix)),
            "{label} must end in {suffix} for {}",
            request.case_id
        );
    }
    let reference = read_regular_non_symlink(&request.reference_path, "reference artifact")?;
    let actual = read_regular_non_symlink(&request.actual_path, "actual artifact")?;
    let comparison = compare_artifact_bytes(&request.case_id, &reference, &actual)?;
    serde_json::to_writer(
        &mut output,
        &PresentationComparisonSuccess {
            schema: "clonk-rs/presentation-comparison/v1",
            case_id: &request.case_id,
            comparison,
            status: "match",
        },
    )?;
    writeln!(output)?;
    Ok(())
}

#[derive(Serialize)]
struct PresentationCapabilityCase {
    id: &'static str,
    artifacts: Vec<&'static str>,
}

#[derive(Serialize)]
struct PresentationCapabilities {
    schema: &'static str,
    producer: &'static str,
    cases: Vec<PresentationCapabilityCase>,
}

fn write_pixel_capabilities(mut output: impl Write) -> Result<()> {
    serde_json::to_writer(
        &mut output,
        &PresentationCapabilities {
            schema: "clonk-rs/presentation-capabilities/v1",
            producer: "clonk-rs-capture-driver-v1",
            cases: LAYOUT_CASES
                .into_iter()
                .map(|case| PresentationCapabilityCase {
                    id: case.id(),
                    artifacts: vec!["png", "layout"],
                })
                .chain(
                    PIXEL_CASES
                        .into_iter()
                        .map(|case| PresentationCapabilityCase {
                            id: case.id(),
                            artifacts: vec!["png"],
                        }),
                )
                .collect(),
        },
    )?;
    writeln!(output)?;
    Ok(())
}

pub(crate) fn run_presentation_utility_from_environment() -> Result<bool> {
    let environment = std::env::vars_os().collect::<BTreeMap<_, _>>();
    if let Some(value) = environment.get(&OsString::from(CAPABILITIES_ENV)) {
        anyhow::ensure!(
            value == std::ffi::OsStr::new("1"),
            "{CAPABILITIES_ENV} must be exactly 1"
        );
        for incompatible in [
            CAPTURE_ENABLED_ENV,
            CAPTURE_RUN_ID_ENV,
            CAPTURE_NONCE_ENV,
            CAPTURE_CASE_ENV,
            CAPTURE_OUTPUT_DIR_ENV,
            CAPTURE_RECEIPT_ENV,
            CAPTURE_DISCOVER_ENV,
            CAPTURE_SOURCE_IDENTITY_ENV,
            COMPARE_ENABLED_ENV,
            COMPARE_REFERENCE_ENV,
            COMPARE_ACTUAL_ENV,
        ] {
            anyhow::ensure!(
                !environment.contains_key(&OsString::from(incompatible)),
                "{CAPABILITIES_ENV} cannot be combined with present {incompatible}"
            );
        }
        anyhow::ensure!(
            std::env::args_os().count() == 1,
            "{CAPABILITIES_ENV} cannot be combined with command-line arguments"
        );
        write_pixel_capabilities(std::io::stdout().lock())?;
        return Ok(true);
    }
    let Some(request) = comparison_request_from_values(&environment)? else {
        return Ok(false);
    };
    for incompatible in [
        CAPTURE_ENABLED_ENV,
        CAPTURE_RUN_ID_ENV,
        CAPTURE_NONCE_ENV,
        CAPTURE_OUTPUT_DIR_ENV,
        CAPTURE_RECEIPT_ENV,
        CAPTURE_DISCOVER_ENV,
        CAPTURE_SOURCE_IDENTITY_ENV,
        CAPABILITIES_ENV,
    ] {
        anyhow::ensure!(
            !environment.contains_key(&OsString::from(incompatible)),
            "{COMPARE_ENABLED_ENV} cannot be combined with present {incompatible}"
        );
    }
    anyhow::ensure!(
        std::env::args_os().count() == 1,
        "{COMPARE_ENABLED_ENV} cannot be combined with command-line arguments"
    );
    run_comparison_request(&request, std::io::stdout().lock())?;
    Ok(true)
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    struct ScopedCaptureEnvironment {
        _lock: parking_lot::ReentrantMutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl ScopedCaptureEnvironment {
        fn install(repository: &Path, user_data: &Path) -> Self {
            Self::install_for_roots(repository, &repository.join("content"), user_data)
        }

        fn install_for_roots(install_root: &Path, content_root: &Path, user_data: &Path) -> Self {
            let lock = crate::tests::env_lock().lock();
            crate::reset_cached_app_paths();
            let values = [
                ("LC_INSTALL_ROOT", install_root.as_os_str().to_owned()),
                ("LC_CONTENT_DIR", content_root.as_os_str().to_owned()),
                ("LC_USER_DATA_DIR", user_data.as_os_str().to_owned()),
                ("LC_PIN_SEED", OsString::from("587")),
                (CAPTURE_DISCOVER_ENV, OsString::from("startup-main")),
            ];
            let saved = values
                .iter()
                .map(|(name, _)| (*name, std::env::var_os(name)))
                .collect();
            for (name, value) in values {
                std::env::set_var(name, value);
            }
            Self { _lock: lock, saved }
        }
    }

    struct IsolatedCaptureAppEnvironment {
        _environment: ScopedCaptureEnvironment,
        _install_root: tempfile::TempDir,
        _user_data: tempfile::TempDir,
    }

    fn copy_directory_tree(source: &Path, destination: &Path) -> Result<()> {
        std::fs::create_dir(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry.file_type()?;
            anyhow::ensure!(!file_type.is_symlink(), "test fixture contains a symlink");
            if file_type.is_dir() {
                copy_directory_tree(&source_path, &destination_path)?;
            } else {
                std::fs::copy(source_path, destination_path)?;
            }
        }
        Ok(())
    }

    impl Drop for ScopedCaptureEnvironment {
        fn drop(&mut self) {
            for (name, value) in self.saved.drain(..) {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
            crate::reset_cached_app_paths();
        }
    }

    fn real_capture_app() -> Result<(ScopedCaptureEnvironment, tempfile::TempDir, crate::GameApp)> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("clonk-app belongs to the workspace");
        let user_data = tempfile::Builder::new()
            .prefix("lc-presentation-pixel-")
            .tempdir()?;
        let environment = ScopedCaptureEnvironment::install(repository, user_data.path());
        let config_path = user_data.path().join("rust.config");
        std::fs::copy(
            repository.join("compat/presentation/rust.config"),
            &config_path,
        )?;
        let player_path = user_data.path().join("Presentation.c4p");
        std::fs::copy(
            repository.join("compat/presentation/player.c4p"),
            &player_path,
        )?;
        let paths = std::sync::Arc::new(crate::AppPaths::discover_with_config_file(Some(
            &config_path,
        ))?);
        paths.ensure_user_dirs()?;
        let classic = crate::ClassicCommandLine {
            player_files: vec![player_path],
            compat_profile: Some(crate::settings::CompatProfile::LegacyClonk),
            ..crate::ClassicCommandLine::default()
        };
        let mut app = crate::build_capture_app(
            &classic,
            Some(&paths),
            crate::RuntimeConfig {
                player_owner: 1,
                player_name: "Presentation Host".to_owned(),
                network: None,
                record_enabled: false,
            },
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.mode != crate::AppMode::Menu {
            app.update()?;
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "capture fixture did not reach the startup menu"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok((environment, user_data, app))
    }

    fn canonical_install_root_capture_app(
    ) -> Result<(IsolatedCaptureAppEnvironment, crate::GameApp)> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("clonk-app belongs to the workspace");
        let install_root = tempfile::Builder::new()
            .prefix("lc-presentation-install-")
            .tempdir()?;
        let user_data = tempfile::Builder::new()
            .prefix("lc-presentation-user-")
            .tempdir()?;
        let planet = install_root.path().join("planet");
        std::fs::create_dir(&planet)?;
        for group in ["Graphics.c4g", "System.c4g"] {
            copy_directory_tree(&repository.join("planet").join(group), &planet.join(group))?;
        }
        let player_path = install_root.path().join("Presentation.c4p");
        std::fs::copy(
            repository.join("compat/presentation/player.c4p"),
            &player_path,
        )?;
        let environment = ScopedCaptureEnvironment::install_for_roots(
            install_root.path(),
            &repository.join("content"),
            user_data.path(),
        );
        let config_path = user_data.path().join("rust.config");
        std::fs::copy(
            repository.join("compat/presentation/rust.config"),
            &config_path,
        )?;
        let paths = std::sync::Arc::new(crate::AppPaths::discover_with_config_file(Some(
            &config_path,
        ))?);
        paths.ensure_user_dirs()?;
        let classic = crate::ClassicCommandLine {
            player_files: vec![player_path],
            compat_profile: Some(crate::settings::CompatProfile::LegacyClonk),
            ..crate::ClassicCommandLine::default()
        };
        let mut app = crate::build_capture_app(
            &classic,
            Some(&paths),
            crate::RuntimeConfig {
                player_owner: 1,
                player_name: "Presentation Host".to_owned(),
                network: None,
                record_enabled: false,
            },
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.mode != crate::AppMode::Menu {
            app.update()?;
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "capture fixture did not reach the startup menu"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok((
            IsolatedCaptureAppEnvironment {
                _environment: environment,
                _install_root: install_root,
                _user_data: user_data,
            },
            app,
        ))
    }

    #[test]
    fn canonical_capture_player_enables_object_context_menu() -> Result<()> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("clonk-app belongs to the workspace");
        let player = std::fs::read(repository.join("compat/presentation/player.c4p"))?;

        assert_eq!(
            sha256_bytes(&player)?,
            "8dcaf794355d1f8d7e8dfa3efa76b8f601a8a911561d161a3da8ead2a40cd5c0"
        );
        assert_eq!(
            CANONICAL_PLAYER_SHA256,
            "8dcaf794355d1f8d7e8dfa3efa76b8f601a8a911561d161a3da8ead2a40cd5c0"
        );
        Ok(())
    }

    #[test]
    fn capture_boot_preserves_the_canonical_config_bytes() -> Result<()> {
        // The capture fork suppresses both native Config.Save sites so the
        // audited input stays byte-exact (src/C4Application.cpp:95-98,364-367).
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("clonk-app belongs to the workspace");
        let (_environment, user_data, _app) = real_capture_app()?;

        assert_eq!(
            std::fs::read(user_data.path().join("rust.config"))?,
            std::fs::read(repository.join("compat/presentation/rust.config"))?,
        );
        Ok(())
    }

    #[test]
    fn player_selection_capture_preserves_the_canonical_config_bytes() -> Result<()> {
        // Native player-list construction only rebuilds Participants in memory;
        // it does not save the config (src/C4StartupPlrSelDlg.cpp:662-729,824-833).
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("clonk-app belongs to the workspace");
        let (environment, mut app) = canonical_install_root_capture_app()?;

        stage_layout_checkpoint(&mut app, LayoutCaptureCase::PlayerSelection, b"")?;

        assert_eq!(
            std::fs::read(environment._user_data.path().join("rust.config"))?,
            std::fs::read(repository.join("compat/presentation/rust.config"))?,
        );
        Ok(())
    }

    #[test]
    fn runtime_resource_identity_recomputes_git_tree_and_flat_manifest() -> Result<()> {
        let directory = tempfile::tempdir()?;
        std::fs::write(directory.path().join("a.txt"), b"alpha\n")?;
        std::fs::create_dir(directory.path().join("nested"))?;
        std::fs::write(directory.path().join("nested/b.txt"), b"beta\n")?;

        let observed = runtime_resource_identity(directory.path())?;

        assert_eq!(
            observed,
            RuntimeResourceIdentity {
                tree: "770f5cd37e40d84f39ab7c168c1cf4cd75bfb59b".to_owned(),
                manifest_sha256: "ab2b347f82d1884f8fc505404de113b0efc4b1258b1949131eb7b113cf907786"
                    .to_owned(),
            }
        );
        Ok(())
    }

    #[test]
    fn live_startup_main_capture_emits_png_and_semantic_layout() -> Result<()> {
        // C4StartupMainDlg paints the live logo, version, buttons, participant
        // label and footer in this exact screen (src/C4StartupMainDlg.cpp:42-74,111-121).
        let (_environment, _user_data, mut app) = real_capture_app()?;

        let output = capture_layout_case_with_app(&mut app, LayoutCaptureCase::Main, &[])?;

        let (width, height, _) = decode_capture_png("startup-main", &output.png)?;
        let trace: crate::presentation_layout::LayoutTrace =
            serde_json::from_slice(&output.layout)?;
        assert_eq!((width, height), (1280, 720));
        assert_eq!(trace.screen, "startup-main");
        assert!(!trace.elements.is_empty());
        assert_eq!(output.checkpoint.render_ordinal, 2);
        Ok(())
    }

    #[test]
    fn live_startup_dialog_captures_emit_png_and_semantic_layout() -> Result<()> {
        // These are the real initial controller states mirrored from
        // C4StartupScenSelDlg.cpp:1302-1382, C4StartupNetDlg.cpp:631-728,
        // C4StartupPlrSelDlg.cpp:545-583, C4StartupOptionsDlg.cpp:609-792,
        // and C4StartupAboutDlg.cpp:262-350.
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("clonk-app belongs to the workspace");
        let network_references =
            std::fs::read(repository.join("compat/presentation/network_references.json"))?;
        for case in [
            LayoutCaptureCase::ScenarioSelection,
            LayoutCaptureCase::NetworkBrowser,
            LayoutCaptureCase::Options,
            LayoutCaptureCase::About,
        ] {
            let (_environment, _user_data, mut app) = real_capture_app()?;

            let output = capture_layout_case_with_app(&mut app, case, &network_references)?;

            let (width, height, _) = decode_capture_png(case.id(), &output.png)?;
            let trace: crate::presentation_layout::LayoutTrace =
                serde_json::from_slice(&output.layout)?;
            assert_eq!((width, height), (1280, 720), "{}", case.id());
            assert_eq!(trace.screen, case.id());
            assert!(!trace.elements.is_empty(), "{}", case.id());
            assert_eq!(output.checkpoint.render_ordinal, 2, "{}", case.id());
        }
        Ok(())
    }

    #[test]
    fn player_selection_rejects_noncanonical_install_root_rows() -> Result<()> {
        // C4StartupPlrSelDlg::UpdatePlayerList discovers every ExePath player,
        // applies Config.General.Participants, then selects the first active row
        // (src/C4StartupPlrSelDlg.cpp:662-729).
        let (_environment, _user_data, mut app) = real_capture_app()?;

        let error = stage_layout_checkpoint(&mut app, LayoutCaptureCase::PlayerSelection, b"")
            .expect_err("capture must reject ambient install-root player rows");

        assert!(error.to_string().contains("canonical Presentation player"));
        Ok(())
    }

    #[test]
    fn player_selection_uses_one_active_selected_install_root_player() -> Result<()> {
        // C4StartupPlrSelDlg::UpdatePlayerList discovers ExePath players,
        // applies Config.General.Participants, and selects the active row
        // (src/C4StartupPlrSelDlg.cpp:662-729).
        let (_environment, mut app) = canonical_install_root_capture_app()?;

        let output =
            capture_layout_case_with_app(&mut app, LayoutCaptureCase::PlayerSelection, b"")?;

        assert_eq!(app.startup.player_models.len(), 1);
        assert_eq!(app.startup.player_models[0].name, "Presentation Host");
        assert!(app.startup.player_models[0].activated);
        let controller = app.startup.player_dialog.as_ref().expect("live dialog");
        assert_eq!(controller.player_activations(), [true]);
        assert_eq!(controller.selected_index(), Some(0));
        let trace: crate::presentation_layout::LayoutTrace =
            serde_json::from_slice(&output.layout)?;
        assert_eq!(trace.screen, "startup-player-selection");
        assert_eq!(output.checkpoint.render_ordinal, 2);
        Ok(())
    }

    fn synthetic_case_specs() -> Vec<serde_json::Value> {
        crate::presentation_captures::screens()
            .iter()
            .map(|screen| {
                let pixel = PixelCaptureCase::from_id(&screen.id);
                let layout = LayoutCaptureCase::from_id(&screen.id);
                let comparison = match screen.comparison {
                    crate::presentation_captures::ComparisonTerm::Pixel => "pixel",
                    crate::presentation_captures::ComparisonTerm::Layout => "layout",
                };
                serde_json::json!({
                    "id": screen.id,
                    "comparison": comparison,
                    "port_asset_exemptions": match screen.id.as_str() {
                        "startup-main" => serde_json::json!({
                            "startup/main/branding/logo": "branding",
                            "startup/main/branding/version": "branding",
                            "startup/main/branding/fan-project": "branding"
                        }),
                        "startup-scenario-selection" => serde_json::json!({
                            "startup/scenario-selection/background": "super-resolved-startup-art"
                        }),
                        "startup-network-browser" => serde_json::json!({
                            "startup/network-browser/background": "super-resolved-startup-art"
                        }),
                        "startup-player-selection" => serde_json::json!({
                            "startup/player-selection/background": "super-resolved-startup-art"
                        }),
                        "startup-options" => serde_json::json!({
                            "startup/options/tabs/paper": "super-resolved-startup-art"
                        }),
                        "startup-about" => serde_json::json!({
                            "startup/about/branding/fan-project": "branding"
                        }),
                        _ => serde_json::json!({}),
                    },
                    "config_sha256": {
                        "cpp": CANONICAL_CONFIG_SHA256,
                        "rust": CANONICAL_CONFIG_SHA256
                    },
                    "player_sha256": CANONICAL_PLAYER_SHA256,
                    "locale": {
                        "language": "US",
                        "charset": "Windows-1252",
                        "lang": "C",
                        "lc_all": "C",
                        "tz": "UTC"
                    },
                    "seeds": {
                        "simulation": {"seed": 587, "calls": 0},
                        "presentation": {
                            "algorithm": PRESENTATION_RNG_ALGORITHM,
                            "seed": 587,
                            "calls": 0,
                            "trace_sha256": EMPTY_SHA256
                        }
                    },
                    "trigger": {"id": pixel.map_or("direct-startup-dialog", PixelCaptureCase::trigger)},
                    "scenario": {
                        "path": pixel.map(PixelCaptureCase::scenario),
                        "content_tree": "4".repeat(40)
                    },
                    "frame": {
                        "checkpoint": pixel.map_or_else(
                            || layout.expect("every synthetic screen is typed").checkpoint(),
                            |case| case.checkpoint().to_owned(),
                        ),
                        "number": pixel.map_or(2, PixelCaptureCase::frame)
                    },
                    "runtime_resources": {
                        "cpp": {
                            "graphics": {
                                "tree": "7".repeat(40),
                                "manifest_sha256": "8".repeat(64)
                            },
                            "system": {
                                "tree": "9".repeat(40),
                                "manifest_sha256": "a".repeat(64)
                            }
                        },
                        "rust": {
                            "graphics": {
                                "tree": "b".repeat(40),
                                "manifest_sha256": "c".repeat(64)
                            },
                            "system": {
                                "tree": "d".repeat(40),
                                "manifest_sha256": "e".repeat(64)
                            }
                        }
                    }
                })
            })
            .collect()
    }

    #[test]
    fn trusted_layout_contract_accepts_only_exact_path_typed_port_asset_exemptions() -> Result<()> {
        let specs = serde_json::to_vec(&synthetic_case_specs())?;

        let spec = trusted_case_spec_from_bytes(&specs, LayoutCaptureCase::Main)?;

        assert_eq!(
            spec.port_asset_exemptions,
            BTreeMap::from([
                (
                    "startup/main/branding/fan-project".to_owned(),
                    "branding".to_owned(),
                ),
                (
                    "startup/main/branding/logo".to_owned(),
                    "branding".to_owned(),
                ),
                (
                    "startup/main/branding/version".to_owned(),
                    "branding".to_owned(),
                ),
            ])
        );
        Ok(())
    }

    #[test]
    fn complete_nonce_bound_environment_selects_one_rust_case() {
        let environment = BTreeMap::from([
            (OsString::from(CAPTURE_ENABLED_ENV), OsString::from("1")),
            (OsString::from(CAPTURE_RUN_ID_ENV), OsString::from("run-1")),
            (
                OsString::from(CAPTURE_NONCE_ENV),
                OsString::from("a".repeat(64)),
            ),
            (OsString::from(CAPTURE_CASE_ENV), OsString::from("gameplay")),
            (
                OsString::from(CAPTURE_OUTPUT_DIR_ENV),
                OsString::from("/candidate/run-1/rust/artifacts"),
            ),
            (
                OsString::from(CAPTURE_RECEIPT_ENV),
                OsString::from("/candidate/run-1/rust/receipts/gameplay.json"),
            ),
            (
                OsString::from(CAPTURE_SOURCE_IDENTITY_ENV),
                OsString::from("/candidate/inputs/rust-source-identity.json"),
            ),
        ]);

        let request = request_from_values(&environment)
            .expect("valid capture environment")
            .expect("capture enabled");

        assert_eq!(request.run_id, "run-1");
        assert_eq!(request.launcher_nonce, "a".repeat(64));
        assert_eq!(request.case_id, "gameplay");
        assert_eq!(
            request.output_dir,
            PathBuf::from("/candidate/run-1/rust/artifacts")
        );
        assert_eq!(
            request.receipt_path,
            PathBuf::from("/candidate/run-1/rust/receipts/gameplay.json")
        );
        assert_eq!(
            request.source_identity_path,
            PathBuf::from("/candidate/inputs/rust-source-identity.json")
        );
    }

    #[test]
    fn capture_environment_rejects_a_noncanonical_launcher_nonce() {
        let environment = BTreeMap::from([
            (OsString::from(CAPTURE_ENABLED_ENV), OsString::from("1")),
            (OsString::from(CAPTURE_RUN_ID_ENV), OsString::from("run-1")),
            (
                OsString::from(CAPTURE_NONCE_ENV),
                OsString::from("launcher-chosen"),
            ),
            (OsString::from(CAPTURE_CASE_ENV), OsString::from("gameplay")),
            (
                OsString::from(CAPTURE_OUTPUT_DIR_ENV),
                OsString::from("/candidate/run-1/rust/artifacts"),
            ),
            (
                OsString::from(CAPTURE_RECEIPT_ENV),
                OsString::from("/candidate/run-1/rust/receipts/gameplay.json"),
            ),
            (
                OsString::from(CAPTURE_SOURCE_IDENTITY_ENV),
                OsString::from("/candidate/inputs/rust-source-identity.json"),
            ),
        ]);

        let error = request_from_values(&environment).expect_err("nonce must be 64 lowercase hex");

        assert!(error.to_string().contains(CAPTURE_NONCE_ENV));
    }

    #[test]
    fn capture_environment_rejects_a_receipt_outside_its_run_topology() {
        let environment = BTreeMap::from([
            (OsString::from(CAPTURE_ENABLED_ENV), OsString::from("1")),
            (OsString::from(CAPTURE_RUN_ID_ENV), OsString::from("run-1")),
            (
                OsString::from(CAPTURE_NONCE_ENV),
                OsString::from("a".repeat(64)),
            ),
            (OsString::from(CAPTURE_CASE_ENV), OsString::from("gameplay")),
            (
                OsString::from(CAPTURE_OUTPUT_DIR_ENV),
                OsString::from("/candidate/run-1/rust/artifacts"),
            ),
            (
                OsString::from(CAPTURE_RECEIPT_ENV),
                OsString::from("/candidate/run-2/rust/receipts/gameplay.json"),
            ),
            (
                OsString::from(CAPTURE_SOURCE_IDENTITY_ENV),
                OsString::from("/candidate/inputs/rust-source-identity.json"),
            ),
        ]);

        let error = request_from_values(&environment)
            .expect_err("receipt must be inside the nonce-bound run directory");

        assert!(error.to_string().contains("receipt"));
    }

    #[test]
    fn trusted_pixel_contract_rejects_a_substituted_trigger() {
        let mut specs = synthetic_case_specs();
        specs[11]["trigger"]["id"] = serde_json::json!("caller-selected-trigger");
        let bytes = serde_json::to_vec(&specs).expect("serialize test contract");

        let error = trusted_case_spec_from_bytes(&bytes, PixelCaptureCase::Gameplay)
            .expect_err("gameplay trigger must be fixed by the trusted contract");

        assert!(error.to_string().contains("trigger"));
    }

    #[test]
    fn trusted_pixel_contract_rejects_a_substituted_scenario() {
        let mut specs = synthetic_case_specs();
        specs[11]["scenario"]["path"] = serde_json::json!("Tutorial.c4f/Tutorial01.c4s");
        let bytes = serde_json::to_vec(&specs).expect("serialize test contract");

        let error = trusted_case_spec_from_bytes(&bytes, PixelCaptureCase::Gameplay)
            .expect_err("gameplay scenario must be fixed by the trusted contract");

        assert!(error.to_string().contains("scenario"));
    }

    #[test]
    fn capture_rejects_a_catalog_winner_outside_the_canonical_content_root() -> Result<()> {
        let content = tempfile::tempdir()?;
        let expected = content.path().join("Tutorial.c4f/Tutorial02.c4s");
        std::fs::create_dir_all(&expected)?;
        let alternate = tempfile::tempdir()?;
        let alternate_scenario = alternate.path().join("Tutorial.c4f/Tutorial02.c4s");
        std::fs::create_dir_all(&alternate_scenario)?;

        let error = validate_canonical_scenario_source(
            content.path(),
            "Tutorial.c4f/Tutorial02.c4s",
            &alternate_scenario,
            std::slice::from_ref(&alternate_scenario),
        )
        .expect_err("an alternate catalog root must not win production capture");

        assert!(error.to_string().contains("canonical content root"));
        Ok(())
    }

    #[test]
    fn trusted_pixel_contract_rejects_a_substituted_checkpoint() {
        let mut specs = synthetic_case_specs();
        specs[11]["frame"]["checkpoint"] = serde_json::json!("state-frame-179");
        let bytes = serde_json::to_vec(&specs).expect("serialize test contract");

        let error = trusted_case_spec_from_bytes(&bytes, PixelCaptureCase::Gameplay)
            .expect_err("gameplay checkpoint must include exact state and render ordinal");

        assert!(error.to_string().contains("checkpoint"));
    }

    #[test]
    fn trusted_pixel_contract_rejects_a_stale_config_digest() {
        let mut specs = synthetic_case_specs();
        specs[11]["config_sha256"]["rust"] =
            serde_json::json!("80be362cafeec5d2f0301451801b43507b32f36b818f23d466083e0727b15315");
        let bytes = serde_json::to_vec(&specs).expect("serialize test contract");

        let error = trusted_case_spec_from_bytes(&bytes, PixelCaptureCase::Gameplay)
            .expect_err("capture must bind the exact native config bytes it consumes");

        assert!(error.to_string().contains("config"));
    }

    #[test]
    fn source_identity_rejects_an_unbound_extra_field() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema": "clonk-rs/presentation-source-identity/v1",
            "commit": "1".repeat(40),
            "tree": "2".repeat(40),
            "content_tree": "3".repeat(40),
            "caller_claim": true
        }))
        .expect("serialize source identity");

        let error = source_identity_from_bytes(&bytes)
            .expect_err("source identity must have an exact schema");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn gameplay_checkpoint_runs_the_real_tutorial_to_frame_180() -> Result<()> {
        // Pinned C++ oracle: src/C4Game.cpp:776-858 executes one real game
        // frame, while src/C4Game.cpp:1902 advances Game.FrameCounter.
        let (_environment, _user_data, mut app) = real_capture_app()?;
        let scenario = crate::resolve_next_mission_scenario(
            &app.scensel.catalog,
            PixelCaptureCase::Gameplay.scenario(),
        )
        .expect("tracked Tutorial02 scenario");

        let checkpoint = stage_tutorial_checkpoint(&mut app, scenario, PixelCaptureCase::Gameplay)?;

        assert_eq!(app.engine.frame(), 180);
        assert_eq!(checkpoint.simulation_seed, 587);
        assert_eq!(checkpoint.render_ordinal, 1);
        Ok(())
    }

    #[test]
    fn gameplay_checkpoint_renders_through_the_production_cpu_surface() -> Result<()> {
        // Pinned C++ oracle: src/C4Game.cpp:776-858 fixes the state consumed
        // by C4Viewport::Draw; the capture takes that real presented frame.
        let (_environment, _user_data, mut app) = real_capture_app()?;
        let scenario = crate::resolve_next_mission_scenario(
            &app.scensel.catalog,
            PixelCaptureCase::Gameplay.scenario(),
        )
        .expect("tracked Tutorial02 scenario");
        let checkpoint = stage_tutorial_checkpoint(&mut app, scenario, PixelCaptureCase::Gameplay)?;

        let png = render_checkpoint_png(&mut app, checkpoint.render_ordinal)?;
        let (width, height, _) = decode_capture_png("rendered", &png)?;

        assert_eq!((width, height), (1280, 720));
        Ok(())
    }

    #[test]
    fn loader_capture_encodes_the_final_ordinal_two_render_frame() -> Result<()> {
        // C4LoaderScreen::Draw draws the current progress/log into lpBack,
        // and the oracle saves that completed second frame
        // (src/C4LoaderScreen.cpp:281-324;
        // parity/oracle/presentation_capture.patch:2035).
        let (_environment, _user_data, mut app) = real_capture_app()?;
        let scenario = crate::resolve_next_mission_scenario(
            &app.scensel.catalog,
            PixelCaptureCase::Loader.scenario(),
        )
        .expect("tracked Tutorial01 loader scenario");
        let checkpoint =
            crate::presentation_pixel_startup::stage_loader_checkpoint(&mut app, scenario)?;
        let stale_surface = crate::encode_surface_to_png(app.graphics.surface())?;

        let png = render_checkpoint_png(&mut app, checkpoint.render_ordinal)?;

        assert_ne!(
            png, stale_surface,
            "loader capture must contain chrome/text written to the final render frame"
        );
        Ok(())
    }

    #[test]
    fn object_menu_checkpoint_uses_the_real_auto_context_menu_route() -> Result<()> {
        // Player AutoContextMenu is loaded by C4InfoCore.cpp:171 and object
        // definitions bind their automatic context-menu mode at C4Def.cpp:416.
        let (_environment, _user_data, mut app) = real_capture_app()?;
        let scenario = crate::resolve_next_mission_scenario(
            &app.scensel.catalog,
            PixelCaptureCase::ObjectMenu.scenario(),
        )
        .expect("tracked Tutorial03 scenario");

        let checkpoint =
            stage_tutorial_checkpoint(&mut app, scenario, PixelCaptureCase::ObjectMenu)?;
        let png = render_checkpoint_png(&mut app, checkpoint.render_ordinal)?;
        let (width, height, _) = decode_capture_png("object-menu", &png)?;

        assert_eq!(app.engine.frame(), 410);
        assert!(app
            .engine
            .cursor_object_menu(app.players.local_owner)
            .is_some());
        assert_eq!(checkpoint.render_ordinal, 90);
        assert_eq!((width, height), (1280, 720));
        Ok(())
    }

    #[test]
    fn engine_receipt_serializes_every_nonce_bound_v2_field() -> Result<()> {
        let specs = serde_json::to_vec(&synthetic_case_specs())?;
        let spec = trusted_case_spec_from_bytes(&specs, PixelCaptureCase::Gameplay)?;
        let request = PresentationCaptureRequest {
            run_id: "run-1".to_owned(),
            launcher_nonce: "a".repeat(64),
            case_id: "gameplay".to_owned(),
            candidate_root: PathBuf::from("/candidate"),
            output_dir: PathBuf::from("/candidate/run-1/rust/artifacts"),
            receipt_path: PathBuf::from("/candidate/run-1/rust/receipts/gameplay.json"),
            source_identity_path: PathBuf::from("/candidate/inputs/rust-source-identity.json"),
        };
        let identity = PresentationSourceIdentity {
            schema: "clonk-rs/presentation-source-identity/v1".to_owned(),
            commit: "1".repeat(40),
            tree: "2".repeat(40),
            content_tree: "4".repeat(40),
        };
        let artifact = CaptureArtifactRecord {
            path: "run-1/rust/artifacts/gameplay.png".to_owned(),
            sha256: "5".repeat(64),
            size_bytes: 123,
        };
        let artifacts = BTreeMap::from([("png".to_owned(), artifact)]);
        let checkpoint = crate::presentation_pixel_startup::StartupPixelCheckpoint {
            simulation_seed: 587,
            random_count: 0,
            render_ordinal: 1,
        };
        let presentation_report = clonk_engine::particles::PresentationSafeRandomCaptureReport {
            calls: 0,
            raw_calls: 0,
            trace: Vec::new(),
        };
        let runtime_resources = spec.runtime_resources.rust.clone();

        let receipt = build_engine_receipt(
            &request,
            &spec,
            &identity,
            "6".repeat(64),
            artifacts.clone(),
            checkpoint,
            presentation_report,
            runtime_resources.clone(),
        )?;
        let value = serde_json::to_value(receipt)?;
        assert_eq!(value["schema"], "clonk-rs/presentation-engine-receipt/v2");
        let keys = value
            .as_object()
            .expect("receipt object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            keys,
            [
                "artifacts",
                "binary_sha256",
                "case_id",
                "config_sha256",
                "content_tree",
                "engine",
                "frame",
                "launcher_nonce",
                "locale",
                "network_references_sha256",
                "player_sha256",
                "producer",
                "profile",
                "run_id",
                "runtime_resources",
                "scenario",
                "schema",
                "seeds",
                "source_tree",
                "trigger",
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(value["config_sha256"], CANONICAL_CONFIG_SHA256);
        assert_eq!(
            value["network_references_sha256"],
            CANONICAL_NETWORK_REFERENCES_SHA256
        );
        assert_eq!(value["source_tree"], identity.tree);
        assert_eq!(
            value["runtime_resources"],
            serde_json::to_value(&runtime_resources)?
        );
        assert_eq!(
            value["seeds"]["presentation"],
            serde_json::json!({
                "algorithm": PRESENTATION_RNG_ALGORITHM,
                "seed": 587,
                "calls": 0,
                "trace_sha256": EMPTY_SHA256
            })
        );

        let stale_trace = build_engine_receipt(
            &request,
            &spec,
            &identity,
            "6".repeat(64),
            artifacts.clone(),
            checkpoint,
            clonk_engine::particles::PresentationSafeRandomCaptureReport {
                calls: 0,
                raw_calls: 0,
                trace: b"1:0\n".to_vec(),
            },
            runtime_resources.clone(),
        )
        .expect_err("receipt must recompute and bind the logical trace digest");
        assert!(stale_trace.to_string().contains("RNG identity"));

        let raw_draw = build_engine_receipt(
            &request,
            &spec,
            &identity,
            "6".repeat(64),
            artifacts.clone(),
            checkpoint,
            clonk_engine::particles::PresentationSafeRandomCaptureReport {
                calls: 0,
                raw_calls: 1,
                trace: Vec::new(),
            },
            runtime_resources.clone(),
        )
        .expect_err("an unaudited direct raw draw must fail closed");
        assert!(raw_draw.to_string().contains("direct raw"));

        let mut substituted_resources = runtime_resources;
        substituted_resources.graphics.tree = "f".repeat(40);
        let resource_error = build_engine_receipt(
            &request,
            &spec,
            &identity,
            "6".repeat(64),
            artifacts,
            checkpoint,
            clonk_engine::particles::PresentationSafeRandomCaptureReport {
                calls: 0,
                raw_calls: 0,
                trace: Vec::new(),
            },
            substituted_resources,
        )
        .expect_err("receipt must bind the observed runtime-resource identity");
        assert!(resource_error.to_string().contains("runtime resources"));
        Ok(())
    }

    #[test]
    fn discovery_runs_the_same_real_gameplay_checkpoint_without_files() -> Result<()> {
        // C4Game::InitGame plays scenario music before InitGameFinal invokes
        // Script.Initialize (C4Game.cpp:2544,2733). With Music=false, the
        // first post-FixRandom presentation draw is therefore the five-way
        // crew portrait selected at C4ObjectInfo.cpp:424.
        clonk_engine::particles::install_presentation_safe_random_seed(587);
        crate::seed_classic_safe_random(587);
        clonk_engine::particles::begin_presentation_safe_random_capture();
        let result = (|| {
            let (_environment, _user_data, mut app) = real_capture_app()?;
            let mut output = Vec::new();

            discover_pixel_case_with_app(&mut app, PixelCaptureCase::Gameplay, &mut output)?;

            let value: serde_json::Value = serde_json::from_slice(&output)?;
            assert_eq!(value["case"], "gameplay");
            assert_eq!(value["simulation_seed"], 587);
            assert_eq!(value["presentation_seed"], 587);
            assert_eq!(value["presentation_algorithm"], PRESENTATION_RNG_ALGORITHM);
            assert_eq!(value["random_count"], 581);
            assert_eq!(value["safe_random_calls"], 1);
            assert_eq!(value["safe_random_trace"], "5:4\n");
            assert_eq!(
                value["safe_random_trace_sha256"],
                "190bdc4eb42013ce38e48c7295c2a6119142cc0af03172a9e393e49131fddcda"
            );
            assert_eq!(value["direct_rand_calls"], 0);
            Ok(())
        })();
        clonk_engine::particles::end_presentation_safe_random_capture();
        clonk_engine::particles::clear_presentation_safe_random_seed();
        result
    }

    #[test]
    fn layout_png_contains_the_second_render_native_presentation() -> Result<()> {
        // The oracle writes lpBack only after the stable capture render has
        // completed (parity/oracle/presentation_capture.patch:2035).
        let (_environment, _user_data, mut app) = real_capture_app()?;
        let checkpoint = stage_layout_checkpoint(&mut app, LayoutCaptureCase::Main, b"")?;
        assert_eq!(checkpoint.render_ordinal, 2);

        let (png, _layout) = render_layout_capture(&mut app, LayoutCaptureCase::Main)?;
        let uncomposed = crate::encode_surface_to_png(app.graphics.surface())?;

        assert_ne!(
            png, uncomposed,
            "native text/chrome must be replayed before encoding the ordinal-2 PNG"
        );
        Ok(())
    }

    #[test]
    fn startup_checkpoint_reseeds_both_random_streams_at_the_execute_seam() -> Result<()> {
        // The instrumented C4Startup::Execute pins RandomHold/RandomCount and
        // SafeRandom before each stable render (src/C4Startup.cpp:370-378).
        let _random = PresentationRandomGuard::install();
        let (_environment, _user_data, mut app) = real_capture_app()?;

        let checkpoint = stage_layout_checkpoint(&mut app, LayoutCaptureCase::Main, b"")?;
        let report = clonk_engine::particles::presentation_safe_random_capture_report();

        assert_eq!(checkpoint.simulation_seed, 587);
        assert_eq!(checkpoint.random_count, 0);
        assert_eq!(report.calls, 0);
        assert_eq!(report.raw_calls, 0);
        assert!(report.trace.is_empty());
        Ok(())
    }

    #[test]
    fn classic_capture_uses_the_native_sound_options_tab() -> Result<()> {
        // C4StartupOptionsDlg constructs its third sheet from IDS_DLG_SOUND
        // (C4StartupOptionsDlg.cpp:686-700).
        let (_environment, _user_data, mut app) = real_capture_app()?;
        stage_layout_checkpoint(&mut app, LayoutCaptureCase::Options, b"")?;

        let labels = app
            .startup
            .options_dialog
            .as_ref()
            .expect("options dialog is live")
            .labels();

        assert_eq!(labels.sheets[2], "Sound");
        Ok(())
    }

    #[test]
    fn one_row_scenario_selection_auto_hides_its_native_scrollbar() -> Result<()> {
        // C4GUI::ListBox::UpdateElementPositions updates its ScrollWindow
        // client height after ScenListItem insertion, and the auto scrollbar
        // hides when that one row does not overflow (src/gui/C4Gui.cpp:2280-2301,
        // src/gui/C4GuiContainers.cpp:493-541).
        let (_environment, _user_data, mut app) = real_capture_app()?;
        stage_layout_checkpoint(&mut app, LayoutCaptureCase::ScenarioSelection, b"")?;
        let row = app
            .menu_state
            .visible_entries()
            .first()
            .cloned()
            .expect("live scenario catalog has a row");
        app.menu_state
            .replace_discovered_entries(vec![row], None, true, false);
        app.menu_state.set_include_back(false);
        app.menu_backdrop_cache = crate::StartupBackdropCache::default();

        let (_png, layout) = render_layout_capture(&mut app, LayoutCaptureCase::ScenarioSelection)?;
        let trace: crate::presentation_layout::LayoutTrace = serde_json::from_slice(&layout)?;
        let scrollbar = trace
            .elements
            .iter()
            .find(|element| element.path == "startup/scenario-selection/list/scrollbar")
            .expect("scenario list scrollbar element");

        assert_eq!(app.menu_state.visible_entries().len(), 1);
        assert!(!scrollbar.visible);
        Ok(())
    }

    #[test]
    fn capabilities_advertise_all_thirteen_compiled_capture_routes() -> Result<()> {
        let mut output = Vec::new();

        write_pixel_capabilities(&mut output)?;

        let value: serde_json::Value = serde_json::from_slice(&output)?;
        assert_eq!(
            value,
            serde_json::json!({
                "schema": "clonk-rs/presentation-capabilities/v1",
                "producer": "clonk-rs-capture-driver-v1",
                "cases": [
                    {"id": "startup-main", "artifacts": ["png", "layout"]},
                    {"id": "startup-scenario-selection", "artifacts": ["png", "layout"]},
                    {"id": "startup-network-browser", "artifacts": ["png", "layout"]},
                    {"id": "startup-player-selection", "artifacts": ["png", "layout"]},
                    {"id": "startup-options", "artifacts": ["png", "layout"]},
                    {"id": "startup-about", "artifacts": ["png", "layout"]},
                    {"id": "network-lobby", "artifacts": ["png"]},
                    {"id": "loader", "artifacts": ["png"]},
                    {"id": "hud", "artifacts": ["png"]},
                    {"id": "ingame-menu", "artifacts": ["png"]},
                    {"id": "object-menu", "artifacts": ["png"]},
                    {"id": "gameplay", "artifacts": ["png"]},
                    {"id": "evaluation", "artifacts": ["png"]}
                ]
            })
        );
        Ok(())
    }

    #[test]
    fn libc_safe_random_route_joins_the_capture_call_counter() {
        // The native `SafeRandom` wrapper is one logical call even when its
        // range-zero branch skips `rand` (C4Random.h:71-75).
        clonk_engine::particles::begin_presentation_safe_random_capture();

        assert_eq!(crate::classic_safe_random_unlocked(0), 0);

        assert_eq!(
            clonk_engine::particles::presentation_safe_random_capture_count(),
            1
        );
        clonk_engine::particles::end_presentation_safe_random_capture();
    }

    #[test]
    fn discovery_rejects_any_present_capture_enable_variable() {
        let environment = BTreeMap::from([
            (
                OsString::from(CAPTURE_DISCOVER_ENV),
                OsString::from("gameplay"),
            ),
            (OsString::from(CAPTURE_ENABLED_ENV), OsString::from("0")),
        ]);

        let error = launch_from_values(&environment)
            .expect_err("discovery and evidence capture must be disjoint");

        assert!(error.to_string().contains(CAPTURE_ENABLED_ENV));
    }

    #[test]
    fn comparator_dispatches_identical_layout_artifacts_by_manifest_term() {
        let trace = serde_json::json!({
            "schema": crate::presentation_layout::LAYOUT_TRACE_SCHEMA,
            "screen": "startup-main",
            "resolution": "1280x720",
            "scale": 100,
            "elements": [{
                "path": "startup/main/background",
                "role": "image",
                "rect": {"x": 0, "y": 0, "width": 1280, "height": 720},
                "visible": true,
                "caption": "",
                "lines": []
            }, {
                "path": "startup/main/branding/logo",
                "role": "image",
                "rect": {"x": 854, "y": 29, "width": 384, "height": 128},
                "visible": true,
                "port_asset": "branding",
                "caption": "",
                "lines": []
            }, {
                "path": "startup/main/branding/version",
                "role": "label",
                "rect": {"x": 854, "y": 168, "width": 394, "height": 22},
                "visible": true,
                "port_asset": "branding",
                "caption": "Version",
                "lines": []
            }, {
                "path": "startup/main/branding/fan-project",
                "role": "label",
                "rect": {"x": 0, "y": 711, "width": 1280, "height": 18},
                "visible": true,
                "port_asset": "branding",
                "caption": "Fan project",
                "lines": []
            }]
        });
        let bytes = serde_json::to_vec(&trace).expect("serialize test layout");

        let comparison = compare_artifact_bytes("startup-main", &bytes, &bytes)
            .expect("identical semantic layouts match");

        assert_eq!(comparison, "layout");
    }

    #[test]
    fn comparator_dispatches_identical_png_artifacts_by_manifest_term() {
        let surface =
            clonk_graphics::Surface::new(1280, 720, clonk_graphics::PixelFormat::Rgba8888);
        let png = crate::encode_surface_to_png(&surface).expect("encode test capture");

        let comparison =
            compare_artifact_bytes("gameplay", &png, &png).expect("identical CPU captures match");

        assert_eq!(comparison, "pixel");
    }

    #[test]
    fn comparator_uses_gpu_tolerance_for_native_opengl_reference() {
        let mut reference =
            clonk_graphics::Surface::new(1280, 720, clonk_graphics::PixelFormat::Rgba8888);
        let mut actual = reference.clone();
        reference
            .set_pixel(0, 0, clonk_graphics::Color::opaque(10, 20, 30))
            .expect("set reference pixel");
        actual
            .set_pixel(0, 0, clonk_graphics::Color::opaque(11, 20, 30))
            .expect("set actual pixel");
        let reference = crate::encode_surface_to_png(&reference).expect("encode reference");
        let actual = crate::encode_surface_to_png(&actual).expect("encode actual");

        let comparison = compare_artifact_bytes("gameplay", &reference, &actual)
            .expect("one-channel delta is accepted for the native OpenGL reference");

        assert_eq!(comparison, "pixel");
    }

    #[test]
    fn complete_comparator_environment_selects_one_manifest_case() {
        let environment = BTreeMap::from([
            (OsString::from(COMPARE_ENABLED_ENV), OsString::from("1")),
            (OsString::from(CAPTURE_CASE_ENV), OsString::from("gameplay")),
            (
                OsString::from(COMPARE_REFERENCE_ENV),
                OsString::from("/accepted/gameplay.png"),
            ),
            (
                OsString::from(COMPARE_ACTUAL_ENV),
                OsString::from("/fresh/gameplay.png"),
            ),
        ]);

        let request = comparison_request_from_values(&environment)
            .expect("valid comparison environment")
            .expect("comparison enabled");

        assert_eq!(request.case_id, "gameplay");
        assert_eq!(
            request.reference_path,
            PathBuf::from("/accepted/gameplay.png")
        );
        assert_eq!(request.actual_path, PathBuf::from("/fresh/gameplay.png"));
    }

    #[test]
    fn comparator_emits_a_bound_success_receipt_after_real_file_comparison() {
        let directory = tempfile::tempdir().expect("create comparison fixture directory");
        let reference_path = directory.path().join("startup-main.reference.layout.json");
        let actual_path = directory.path().join("startup-main.actual.layout.json");
        let trace = serde_json::json!({
            "schema": crate::presentation_layout::LAYOUT_TRACE_SCHEMA,
            "screen": "startup-main",
            "resolution": "1280x720",
            "scale": 100,
            "elements": [{
                "path": "startup/main/background",
                "role": "image",
                "rect": {"x": 0, "y": 0, "width": 1280, "height": 720},
                "visible": true,
                "caption": "",
                "lines": []
            }, {
                "path": "startup/main/branding/logo",
                "role": "image",
                "rect": {"x": 854, "y": 29, "width": 384, "height": 128},
                "visible": true,
                "port_asset": "branding",
                "caption": "",
                "lines": []
            }, {
                "path": "startup/main/branding/version",
                "role": "label",
                "rect": {"x": 854, "y": 168, "width": 394, "height": 22},
                "visible": true,
                "port_asset": "branding",
                "caption": "Version",
                "lines": []
            }, {
                "path": "startup/main/branding/fan-project",
                "role": "label",
                "rect": {"x": 0, "y": 711, "width": 1280, "height": 18},
                "visible": true,
                "port_asset": "branding",
                "caption": "Fan project",
                "lines": []
            }]
        });
        let bytes = serde_json::to_vec(&trace).expect("serialize test layout");
        std::fs::write(&reference_path, &bytes).expect("write reference layout");
        std::fs::write(&actual_path, &bytes).expect("write actual layout");
        let request = PresentationComparisonRequest {
            case_id: "startup-main".to_owned(),
            reference_path,
            actual_path,
        };
        let mut output = Vec::new();

        run_comparison_request(&request, &mut output).expect("compare artifact files");

        assert_eq!(
            String::from_utf8(output).expect("comparison output is UTF-8"),
            "{\"schema\":\"clonk-rs/presentation-comparison/v1\",\"case_id\":\"startup-main\",\"comparison\":\"layout\",\"status\":\"match\"}\n"
        );
    }
}
