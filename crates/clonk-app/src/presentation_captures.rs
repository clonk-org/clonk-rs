//! Which screens a compatibility session promises to render like the oracle,
//! and on what terms a capture pair may be compared.
//!
//! `compat/profile.json` names clonk-org/clonk-rs#587 as the presentation
//! promise's pending evidence; this is the detail behind that entry. Keeping
//! the list here rather than in a test means the screens, the geometry and the
//! tolerances are one artifact that the contract gate reads, so a screen cannot
//! quietly stop being compared.
//!
//! A screen counts as `captured` only once it names a same-resolution C++ and
//! Rust capture pair. Until then it is `pending` and the profile may not claim
//! the presentation promise — which is already the case, through #587.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// The manifest, embedded so the runtime answer and the gated file are the same
/// artifact.
const CAPTURE_MANIFEST: &str = include_str!("../../../compat/presentation_captures.json");

/// A capture the profile owes, or has.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CaptureScreen {
    /// Stable id, quotable in a report.
    pub id: String,
    /// Which part of the session it belongs to.
    pub area: String,
    /// What the screen is.
    pub description: String,
    /// `pending` until a capture pair exists, then `captured`.
    pub status: String,
    /// The C++ and Rust capture pair, once there is one.
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// A region a comparison is allowed to ignore.
///
/// Masking is how a comparison lies, so each one has to say what platform
/// artifact it covers and who approved it. A mask may never hide a difference
/// the profile is supposed to be proving absent.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CaptureMask {
    /// Which screen it applies to.
    pub screen: String,
    /// The masked region, `x,y,width,height`.
    pub region: String,
    /// The platform artifact it covers.
    pub reason: String,
    /// What approved it — a contract section or an issue.
    pub authority: String,
}

/// The geometry every capture shares.
#[derive(Clone, Debug, Deserialize)]
pub struct CaptureGeometry {
    pub resolution: String,
    pub scale: u32,
}

/// How far two captures may differ and still match.
#[derive(Clone, Debug, Deserialize)]
pub struct CaptureTolerance {
    pub cpu_max_channel_delta: u8,
    pub gpu_max_channel_delta: u8,
}

#[derive(Deserialize)]
struct Manifest {
    capture: CaptureGeometry,
    tolerance: CaptureTolerance,
    screens: Vec<CaptureScreen>,
    #[serde(default)]
    masks: Vec<CaptureMask>,
}

fn manifest() -> &'static Manifest {
    static PARSED: OnceLock<Manifest> = OnceLock::new();
    PARSED.get_or_init(|| {
        serde_json::from_str(CAPTURE_MANIFEST)
            .expect("the embedded presentation capture manifest is gated by the app test suite")
    })
}

/// Every screen the profile owes a capture comparison for.
pub fn screens() -> &'static [CaptureScreen] {
    &manifest().screens
}

/// The regions a comparison may ignore.
pub fn masks() -> &'static [CaptureMask] {
    &manifest().masks
}

/// The geometry every capture shares.
pub fn geometry() -> &'static CaptureGeometry {
    &manifest().capture
}

/// How far two captures may differ and still match.
pub fn tolerance() -> &'static CaptureTolerance {
    &manifest().tolerance
}

/// The screens that still have no capture pair.
pub fn pending_screens() -> Vec<&'static CaptureScreen> {
    screens()
        .iter()
        .filter(|screen| screen.status == "pending")
        .collect()
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    /// The scope clonk-org/clonk-rs#587 names, screen by screen.
    ///
    /// Listing them here rather than counting entries is deliberate: a screen
    /// dropped from the manifest is exactly the failure this is for, and a
    /// count would not notice a rename.
    const REQUIRED_SCREENS: &[&str] = &[
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

    #[test]
    fn the_manifest_covers_every_screen_the_profile_promises() {
        let present = screens()
            .iter()
            .map(|screen| screen.id.as_str())
            .collect::<BTreeSet<_>>();
        let missing = REQUIRED_SCREENS
            .iter()
            .filter(|required| !present.contains(*required))
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "the presentation promise covers screens the manifest omits: {missing:?}"
        );
    }

    #[test]
    fn every_screen_declares_a_status_the_gate_understands() {
        for screen in screens() {
            assert!(
                matches!(screen.status.as_str(), "pending" | "captured"),
                "{}: status `{}` is neither pending nor captured",
                screen.id,
                screen.status
            );
            assert!(
                !screen.description.trim().is_empty(),
                "{}: says nothing about what the screen is",
                screen.id
            );
        }
    }

    /// A `captured` screen with no capture pair would assert a comparison that
    /// never runs, which is the one failure mode this manifest exists to stop.
    #[test]
    fn a_captured_screen_names_the_pair_it_was_compared_against() {
        for screen in screens().iter().filter(|s| s.status == "captured") {
            assert!(
                screen.evidence.len() >= 2,
                "{}: claims a capture without naming a C++ and a Rust capture",
                screen.id
            );
        }
    }

    #[test]
    fn every_mask_names_a_real_screen_a_reason_and_an_authority() {
        let known = screens()
            .iter()
            .map(|screen| screen.id.as_str())
            .collect::<BTreeSet<_>>();
        for mask in masks() {
            assert!(
                known.contains(mask.screen.as_str()),
                "mask covers unknown screen `{}`",
                mask.screen
            );
            assert!(
                !mask.reason.trim().is_empty(),
                "mask on `{}` hides pixels without saying what artifact it covers",
                mask.screen
            );
            assert!(
                !mask.authority.trim().is_empty(),
                "mask on `{}` hides pixels without naming what approved it",
                mask.screen
            );
            let fields = mask.region.split(',').count();
            assert_eq!(
                fields, 4,
                "mask on `{}` has region `{}`, not `x,y,width,height`",
                mask.screen, mask.region
            );
        }
    }

    /// The software renderer is the exact oracle, so a CPU comparison that
    /// tolerated anything would stop being one.
    #[test]
    fn the_cpu_comparison_admits_no_difference_at_all() {
        assert_eq!(tolerance().cpu_max_channel_delta, 0);
        assert!(
            tolerance().gpu_max_channel_delta <= 1,
            "the GPU tolerance is the documented one byte, not {}",
            tolerance().gpu_max_channel_delta
        );
    }

    /// Comparing captures taken at different geometry proves nothing, so the
    /// manifest fixes one and the gate reads it rather than each harness
    /// choosing its own.
    #[test]
    fn every_capture_shares_one_resolution_and_scale() {
        assert_eq!(geometry().resolution, "1280x720");
        assert_eq!(geometry().scale, 100);
    }

    /// The profile may not claim presentation while a screen is uncompared.
    /// This is the same fail-closed reading `compat_readiness` applies, stated
    /// here so the two cannot drift apart silently.
    #[test]
    fn an_uncaptured_screen_keeps_the_presentation_promise_pending() {
        let pending = pending_screens();
        if pending.is_empty() {
            return;
        }
        let manifest = crate::compat_readiness::profile_manifest_for_tests();
        assert!(
            manifest.contains("clonk-org/clonk-rs#587"),
            "screens are still uncaptured but the profile records no pending \
             presentation evidence for them"
        );
    }
}
