//! Which screens a compatibility session promises to render like the oracle,
//! and on what terms a capture pair may be compared.
//!
//! `compat/profile.json` names the landing presentation-capture test as held
//! evidence; this is the detail behind that entry. Keeping the list here rather
//! than in a test means the screens, the geometry and the tolerances are one
//! artifact that the contract gate reads, so a screen cannot quietly stop being
//! compared.
//!
//! A screen counts as `captured` only once it names a same-resolution C++ and
//! Rust capture pair. Until then it is `pending` and the profile may not claim
//! the presentation promise. All thirteen promised screens retain their first
//! audited pair, and the landing gate reruns the comparison.

use serde::Deserialize;
#[cfg(test)]
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
    /// Which term this screen is compared on — `pixel` or `layout`.
    ///
    /// A screen that renders port-authored assets cannot meet a pixel term at
    /// any resolution, so it is compared on layout instead
    /// (clonk-org/clonk-rs#1298). Defaults to `pixel`, so a screen has to opt
    /// into the weaker term explicitly.
    #[serde(default = "default_comparison")]
    pub comparison: ComparisonTerm,
    /// The `port_assets` classes this screen renders, when it is on the
    /// `layout` term. Naming them is what keeps the weaker term from being a
    /// blanket excuse.
    #[serde(default)]
    pub port_assets: Vec<String>,
    /// The exact C++ and Rust comparison artifacts, once there is a pair.
    #[serde(default)]
    pub evidence: Option<CaptureEvidence>,
    /// Why a comparison that has already been run cannot pass.
    ///
    /// A screen with no blocker is pending because nobody has captured it. A
    /// screen with one has been captured and measured, and the reason it still
    /// cannot be `captured` is written down rather than left to be
    /// rediscovered.
    #[serde(default)]
    pub blocker: Option<CaptureBlocker>,
}

/// One screen-bound pair of artifacts retained from the first audited run.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CaptureEvidence {
    /// The C++ reference artifact used by the declared comparison term.
    pub cpp: String,
    /// The Rust artifact compared to [`Self::cpp`].
    pub rust: String,
}

fn default_comparison() -> ComparisonTerm {
    ComparisonTerm::Pixel
}

/// The evidence a screen promises to compare.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComparisonTerm {
    /// Pixel equality within the manifest's surface tolerance.
    Pixel,
    /// Ordered control geometry, captions and resolved wrapping.
    Layout,
}

/// A class of port-authored asset that puts a screen on the `layout` term.
///
/// Declaring these is the difference between "this screen is compared more
/// weakly" and "this screen is compared more weakly *because* it renders the
/// port's own logo" — the second can be argued with, the first cannot.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PortAsset {
    /// Stable id a screen references.
    pub id: String,
    /// What the asset is.
    pub summary: String,
    /// What approved treating it as port-authored.
    pub authority: String,
}

/// A measured pixel result for a screen, and where the remaining work is
/// tracked.
///
/// For a screen on the `layout` term the recorded result is expected rather
/// than a defect: it is kept so the measurement is not rediscovered, and so a
/// reader can see the renderer itself was not what failed.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CaptureBlocker {
    /// What was measured and what explains the difference.
    pub summary: String,
    /// Where the decision that would unblock it is tracked.
    pub issue: String,
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
    pub pointer: CapturePointer,
}

/// Ambient mouse state normalized as producer input, never hidden from output.
#[derive(Clone, Debug, Deserialize)]
pub struct CapturePointer {
    pub position: [u32; 2],
    pub button: String,
    pub modifiers: Vec<String>,
    pub help: bool,
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
    #[serde(default)]
    port_assets: Vec<PortAsset>,
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

/// The port-authored asset classes that put a screen on the `layout` term.
pub fn port_assets() -> &'static [PortAsset] {
    &manifest().port_assets
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

/// The screens whose comparison has been run and cannot pass as specified.
///
/// These are a subset of [`pending_screens`] — a measured blocker never lets a
/// screen count towards the presentation promise, it only says why.
pub fn blocked_screens() -> Vec<&'static CaptureScreen> {
    screens()
        .iter()
        .filter(|screen| screen.blocker.is_some())
        .collect()
}

/// Which renderer produced a capture, and therefore how far it may differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSurface {
    /// The software renderer, which is the exact oracle.
    Cpu,
    /// GPU composition, which carries the documented one-byte cross-driver
    /// tolerance (`docs/RENDERING_PARITY.md`).
    Gpu,
}

impl CaptureSurface {
    fn max_channel_delta(self) -> u8 {
        match self {
            Self::Cpu => tolerance().cpu_max_channel_delta,
            Self::Gpu => tolerance().gpu_max_channel_delta,
        }
    }
}

/// Why a capture pair is not a match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureMismatch {
    /// The manifest does not list the screen, so no terms exist to compare on.
    UnknownScreen { screen: String },
    /// The caller chose a comparator that does not implement this screen's term.
    ComparisonTerm {
        screen: String,
        expected: ComparisonTerm,
        actual: ComparisonTerm,
    },
    /// A capture is not the geometry the manifest fixes for every screen.
    Geometry {
        screen: String,
        expected: String,
        actual: String,
    },
    /// A buffer is not `width * height * 4` bytes of RGBA.
    Length {
        screen: String,
        expected: usize,
        actual: usize,
    },
    /// The captures differ outside every approved mask.
    Pixels {
        screen: String,
        /// The first differing pixel in row-major order, so a report names one
        /// place to look rather than a count.
        x: u32,
        y: u32,
        /// The largest single-channel difference anywhere outside the masks.
        max_channel_delta: u8,
        /// How many pixels differ, so a one-pixel slip reads differently from a
        /// wholesale divergence.
        differing_pixels: usize,
    },
}

/// Whether `x,y,width,height` covers this pixel.
///
/// Start-inclusive and end-exclusive, so adjacent regions do not overlap. A
/// region that does not parse covers **nothing**: the manifest test rejects a
/// malformed one, and failing open here would let a typo silently widen a
/// comparison into passing.
fn region_contains(region: &str, x: u32, y: u32) -> bool {
    let mut fields = region.split(',').map(|field| field.trim().parse::<u32>());
    match (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) {
        (Some(Ok(mx)), Some(Ok(my)), Some(Ok(mw)), Some(Ok(mh)), None) => {
            x >= mx && x < mx.saturating_add(mw) && y >= my && y < my.saturating_add(mh)
        }
        _ => false,
    }
}

/// Whether a pixel falls inside a region approved for this screen.
fn masked(screen: &str, x: u32, y: u32) -> bool {
    masks()
        .iter()
        .filter(|mask| mask.screen == screen)
        .any(|mask| region_contains(&mask.region, x, y))
}

/// Compares one C++/Rust capture pair on the manifest's terms.
///
/// Both buffers are RGBA8 at the geometry the manifest fixes. Alpha is compared
/// like any other channel: a capture that differs only in alpha still differs.
///
/// This is deliberately given raw buffers rather than paths — decoding belongs
/// to whatever produces the captures, and keeping it out means the terms of the
/// comparison can be tested without any capture existing yet
/// (clonk-org/clonk-rs#587).
pub fn compare_capture(
    screen: &str,
    surface: CaptureSurface,
    width: u32,
    height: u32,
    reference: &[u8],
    actual: &[u8],
) -> Result<(), CaptureMismatch> {
    let terms = screens()
        .iter()
        .find(|entry| entry.id == screen)
        .ok_or_else(|| CaptureMismatch::UnknownScreen {
            screen: screen.to_owned(),
        })?;
    if terms.comparison != ComparisonTerm::Pixel {
        return Err(CaptureMismatch::ComparisonTerm {
            screen: screen.to_owned(),
            expected: terms.comparison,
            actual: ComparisonTerm::Pixel,
        });
    }

    let expected_geometry = &geometry().resolution;
    let actual_geometry = format!("{width}x{height}");
    if actual_geometry != *expected_geometry {
        return Err(CaptureMismatch::Geometry {
            screen: screen.to_owned(),
            expected: expected_geometry.clone(),
            actual: actual_geometry,
        });
    }

    let expected_len = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    for buffer in [reference, actual] {
        if buffer.len() != expected_len {
            return Err(CaptureMismatch::Length {
                screen: screen.to_owned(),
                expected: expected_len,
                actual: buffer.len(),
            });
        }
    }

    let limit = surface.max_channel_delta();
    let mut first: Option<(u32, u32)> = None;
    let mut worst = 0_u8;
    let mut differing = 0_usize;
    for index in 0..(width as usize * height as usize) {
        let x = (index % width as usize) as u32;
        let y = (index / width as usize) as u32;
        if masked(screen, x, y) {
            continue;
        }
        let offset = index * 4;
        let delta = (0..4)
            .map(|channel| reference[offset + channel].abs_diff(actual[offset + channel]))
            .max()
            .unwrap_or(0);
        if delta > limit {
            differing += 1;
            worst = worst.max(delta);
            first.get_or_insert((x, y));
        }
    }

    match first {
        None => Ok(()),
        Some((x, y)) => Err(CaptureMismatch::Pixels {
            screen: screen.to_owned(),
            x,
            y,
            max_channel_delta: worst,
            differing_pixels: differing,
        }),
    }
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
        for screen in screens() {
            if screen.status == "pending" {
                assert!(
                    screen.evidence.is_none(),
                    "{}: pending screen already claims capture evidence",
                    screen.id
                );
                continue;
            }
            let evidence = screen
                .evidence
                .as_ref()
                .unwrap_or_else(|| panic!("{}: captured screen names no pair", screen.id));
            let suffix = match screen.comparison {
                ComparisonTerm::Pixel => "png",
                ComparisonTerm::Layout => "layout.json",
            };
            assert_eq!(
                evidence.cpp,
                format!(
                    "compat/presentation/oracle/v1/run-1/cpp/artifacts/{}.{}",
                    screen.id, suffix
                ),
                "{}: C++ evidence is detached from its compared artifact",
                screen.id
            );
            assert_eq!(
                evidence.rust,
                format!(
                    "compat/presentation/oracle/v1/run-1/rust/artifacts/{}.{}",
                    screen.id, suffix
                ),
                "{}: Rust evidence is detached from its compared artifact",
                screen.id
            );
        }
    }

    /// A blocker is the record of a comparison that was run and did not pass.
    /// Without a measurement and a place the decision lives it degrades into
    /// an excuse, which is worse than a bare `pending`.
    #[test]
    fn a_blocked_screen_says_what_was_measured_and_where_it_is_decided() {
        for screen in blocked_screens() {
            let blocker = screen.blocker.as_ref().expect("filtered on Some");
            assert!(
                blocker.summary.contains('%'),
                "{}: blocker states no measured result",
                screen.id
            );
            assert!(
                blocker.issue.starts_with("clonk-org/clonk-rs#"),
                "{}: blocker issue `{}` is not a qualified reference",
                screen.id,
                blocker.issue
            );
            assert_eq!(
                screen.status, "pending",
                "{}: a measured blocker cannot sit on a screen that claims to be captured",
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

    /// The weaker term is opt-in and has to be justified: a screen may only be
    /// compared on layout if it names a declared port-authored asset class.
    /// Without that, "compare this one more weakly" would be unfalsifiable
    /// (clonk-org/clonk-rs#1298).
    #[test]
    fn a_layout_screen_names_the_port_assets_that_put_it_there() {
        let declared = port_assets()
            .iter()
            .map(|asset| asset.id.as_str())
            .collect::<BTreeSet<_>>();
        for screen in screens() {
            assert!(
                matches!(
                    screen.comparison,
                    ComparisonTerm::Pixel | ComparisonTerm::Layout
                ),
                "{}: unknown comparison term",
                screen.id
            );
            if screen.comparison == ComparisonTerm::Layout {
                assert!(
                    !screen.port_assets.is_empty(),
                    "{}: compared on layout without naming a port asset class",
                    screen.id
                );
                for asset in &screen.port_assets {
                    assert!(
                        declared.contains(asset.as_str()),
                        "{}: names undeclared port asset class `{asset}`",
                        screen.id
                    );
                }
            } else {
                assert!(
                    screen.port_assets.is_empty(),
                    "{}: names port assets but is still compared on pixels",
                    screen.id
                );
            }
        }
    }

    #[test]
    fn in_game_product_branding_uses_the_layout_term() {
        for id in [
            "hud",
            "ingame-menu",
            "object-menu",
            "gameplay",
            "evaluation",
        ] {
            let screen = screens()
                .iter()
                .find(|screen| screen.id == id)
                .unwrap_or_else(|| panic!("missing capture screen {id}"));
            assert_eq!(
                screen.comparison,
                ComparisonTerm::Layout,
                "{id} renders the Clonk Rust logo"
            );
            assert_eq!(screen.port_assets, ["branding"]);
        }
    }

    /// Every declared class has to say what it is and what approved it, for the
    /// same reason a mask does.
    #[test]
    fn every_port_asset_class_says_what_it_is_and_who_approved_it() {
        for asset in port_assets() {
            assert!(
                !asset.summary.trim().is_empty(),
                "port asset `{}` declares no summary",
                asset.id
            );
            assert!(
                asset.authority.contains("clonk-org/clonk-rs#"),
                "port asset `{}` names no qualified authority",
                asset.id
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

    #[test]
    fn every_capture_pins_the_same_engine_rendered_pointer_input() {
        assert_eq!(geometry().pointer.position, [32, 32]);
        assert_eq!(geometry().pointer.button, "none");
        assert!(geometry().pointer.modifiers.is_empty());
        assert!(!geometry().pointer.help);
    }

    /// A capture the size the manifest fixes, filled with one colour.
    fn plane(value: u8) -> Vec<u8> {
        let (width, height) = (1_280_usize, 720_usize);
        vec![value; width * height * 4]
    }

    #[test]
    fn an_identical_pair_matches_on_either_surface() {
        for surface in [CaptureSurface::Cpu, CaptureSurface::Gpu] {
            assert_eq!(
                compare_capture("network-lobby", surface, 1_280, 720, &plane(9), &plane(9)),
                Ok(())
            );
        }
    }

    #[test]
    fn a_layout_screen_cannot_be_verified_by_the_pixel_comparator() {
        assert_eq!(
            compare_capture(
                "startup-main",
                CaptureSurface::Cpu,
                1_280,
                720,
                &plane(9),
                &plane(9),
            ),
            Err(CaptureMismatch::ComparisonTerm {
                screen: "startup-main".to_owned(),
                expected: ComparisonTerm::Layout,
                actual: ComparisonTerm::Pixel,
            })
        );
    }

    /// The software renderer is the exact oracle, so nothing is forgiven there;
    /// GPU composition carries the documented one byte and no more.
    #[test]
    fn the_surface_decides_how_far_a_channel_may_drift() {
        let reference = plane(10);
        let mut off_by_one = plane(10);
        off_by_one[0] = 11;
        let mut off_by_two = plane(10);
        off_by_two[0] = 12;

        assert!(matches!(
            compare_capture(
                "network-lobby",
                CaptureSurface::Cpu,
                1_280,
                720,
                &reference,
                &off_by_one
            ),
            Err(CaptureMismatch::Pixels {
                max_channel_delta: 1,
                differing_pixels: 1,
                ..
            })
        ));
        assert_eq!(
            compare_capture(
                "network-lobby",
                CaptureSurface::Gpu,
                1_280,
                720,
                &reference,
                &off_by_one
            ),
            Ok(())
        );
        assert!(matches!(
            compare_capture(
                "network-lobby",
                CaptureSurface::Gpu,
                1_280,
                720,
                &reference,
                &off_by_two
            ),
            Err(CaptureMismatch::Pixels {
                max_channel_delta: 2,
                ..
            })
        ));
    }

    /// A report has to name one place to look, not just a count.
    #[test]
    fn a_mismatch_names_the_first_differing_pixel_in_row_major_order() {
        let reference = plane(0);
        let mut actual = plane(0);
        // (5, 2) comes first in row-major order; (400, 1) is earlier by y.
        let at = |x: usize, y: usize| (y * 1_280 + x) * 4;
        actual[at(5, 2)] = 40;
        actual[at(400, 1)] = 40;

        let Err(CaptureMismatch::Pixels {
            x,
            y,
            differing_pixels,
            ..
        }) = compare_capture(
            "network-lobby",
            CaptureSurface::Cpu,
            1_280,
            720,
            &reference,
            &actual,
        )
        else {
            panic!("differing planes do not match");
        };
        assert_eq!((x, y), (400, 1));
        assert_eq!(differing_pixels, 2);
    }

    /// Geometry and buffer length are rejected before any pixel is read: a
    /// comparison across different shapes proves nothing, and silently
    /// comparing a prefix would look like a pass.
    #[test]
    fn a_capture_that_is_not_the_fixed_geometry_is_rejected() {
        assert!(matches!(
            compare_capture(
                "network-lobby",
                CaptureSurface::Cpu,
                640,
                480,
                &plane(0),
                &plane(0)
            ),
            Err(CaptureMismatch::Geometry { .. })
        ));
        assert!(matches!(
            compare_capture(
                "network-lobby",
                CaptureSurface::Cpu,
                1_280,
                720,
                &plane(0),
                &[0; 16]
            ),
            Err(CaptureMismatch::Length { .. })
        ));
    }

    /// Masking is how a comparison lies, so its bounds are worth pinning even
    /// while the manifest approves none. Start-inclusive and end-exclusive, and
    /// a region that does not parse covers nothing rather than everything.
    #[test]
    fn an_approved_region_covers_exactly_its_own_pixels() {
        for (x, y) in [(10, 20), (10, 39), (29, 20), (29, 39)] {
            assert!(region_contains("10,20,20,20", x, y), "({x},{y}) is inside");
        }
        for (x, y) in [(9, 20), (10, 19), (30, 20), (10, 40)] {
            assert!(
                !region_contains("10,20,20,20", x, y),
                "({x},{y}) is outside"
            );
        }
        assert!(region_contains(" 10 , 20 , 20 , 20 ", 15, 25));
        assert!(
            !region_contains("10,20,0,20", 10, 20),
            "a zero-width region covers nothing"
        );
        for malformed in ["", "10,20,20", "10,20,20,20,20", "a,b,c,d", "-1,0,5,5"] {
            assert!(
                !region_contains(malformed, 0, 0),
                "`{malformed}` must not mask anything"
            );
        }
    }

    /// Comparing a screen the manifest does not list would be a comparison with
    /// no stated terms — no geometry, no tolerance, no approved masks.
    #[test]
    fn a_screen_outside_the_manifest_has_no_terms_to_compare_on() {
        assert!(matches!(
            compare_capture(
                "not-a-screen",
                CaptureSurface::Cpu,
                1_280,
                720,
                &plane(0),
                &plane(0)
            ),
            Err(CaptureMismatch::UnknownScreen { .. })
        ));
    }

    /// The presentation promise is held only when every required screen has
    /// retained comparison evidence and the landing gate keeps rerunning it.
    #[test]
    fn every_presentation_capture_is_held_by_the_landing_gate() {
        assert_eq!(
            screens().len(),
            13,
            "the promised screen set must stay complete"
        );
        assert!(
            pending_screens().is_empty(),
            "every screen must be captured"
        );
        assert!(
            blocked_screens().is_empty(),
            "no captured screen may remain blocked"
        );

        let profile: serde_json::Value =
            serde_json::from_str(crate::compat_readiness::profile_manifest_for_tests())
                .expect("the embedded compatibility profile is valid JSON");
        let evidence = profile["promise"]["presentation"]["evidence"]
            .as_array()
            .expect("the presentation promise has an evidence array");
        assert!(evidence.iter().any(|entry| {
            entry["kind"] == "test"
                && entry["value"] == ".github/workflows/landing.yml (presentation captures)"
                && entry["status"] == "held"
        }));
        assert!(!evidence
            .iter()
            .any(|entry| entry["value"] == "clonk-org/clonk-rs#587"));
    }
}
