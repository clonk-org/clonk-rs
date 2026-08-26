//! The graphics floor: what an adapter must offer before retained GPU
//! presentation can start, checked in one pass. The primary window may still
//! start through the wgpu-free software presenter when this probe fails.
//!
//! Discovery previously failed at the first unmet requirement and, for the
//! surface format, did not fail at all — it fell back to `Bgra8UnormSrgb`
//! whether or not the surface offered it, turning a missing capability into a
//! later error somewhere else. A machine below the floor is worth one
//! diagnostic naming everything it is missing, not a sequence of them.
//!
//! The probe is a pure function over capability *data* rather than a live
//! adapter, so every requirement is unit-testable without a GPU. That matters
//! because the tiers this is meant to police — GLES 3 and software adapters —
//! are exactly the ones CI runners do not have.

/// Optional wgpu features interactive play requires: none.
///
/// Timestamp queries are opt-in (`WindowSurfaceBuildOptions`) and the renderer
/// runs without them, so nothing here may become non-empty without a matching
/// change to the support matrix in `docs/GRAPHICS_SUPPORT.md`.
pub const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::empty();

/// What the frame-buffer texture is created with: it is uploaded to and
/// sampled, and nothing else.
pub const REQUIRED_BUFFER_USAGES: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST);

/// The 2D texture dimension the floor promises, which is what GLES 3.0 and
/// WebGL2 both guarantee.
///
/// An adapter offering more is used to the full — the device asks for
/// `adapter.limits()` — so this bounds what the port may *require*, not what it
/// may use.
pub const MINIMUM_TEXTURE_DIMENSION_2D: u32 = 2048;

/// One unmet requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingCapability {
    /// The adapter cannot both upload to and sample the frame-buffer format.
    BufferUsages {
        needed: wgpu::TextureUsages,
        available: wgpu::TextureUsages,
    },
    /// Presentation composites in byte space and relies on the surface encode
    /// to restore those bytes, so an sRGB surface format is required.
    SrgbSurfaceFormat,
    /// The requested buffer does not fit `max_texture_dimension_2d`.
    TextureDimension { needed: u32, available: u32 },
    /// A zero extent, which no adapter can satisfy and which the caller should
    /// never have asked for.
    ZeroExtent,
}

impl std::fmt::Display for MissingCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SrgbSurfaceFormat => {
                formatter.write_str("the surface offers no sRGB texture format")
            }
            Self::TextureDimension { needed, available } => write!(
                formatter,
                "the requested {needed}px buffer exceeds the adapter's \
                 max_texture_dimension_2d of {available}px"
            ),
            Self::BufferUsages { needed, available } => write!(
                formatter,
                "the frame buffer format supports {available:?}, and presentation needs {needed:?}"
            ),
            Self::ZeroExtent => formatter.write_str("the requested buffer extent is zero"),
        }
    }
}

/// Everything the adapter fails to provide, in the order it is checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityReport {
    pub missing: Vec<MissingCapability>,
}

impl CapabilityReport {
    pub fn is_supported(&self) -> bool {
        self.missing.is_empty()
    }
}

impl std::fmt::Display for CapabilityReport {
    /// One line naming every missing requirement, so a below-floor machine
    /// reports its whole gap at once.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.missing.is_empty() {
            return formatter.write_str("the adapter meets the graphics floor");
        }
        formatter.write_str("this GPU cannot run retained GPU presentation: ")?;
        for (index, missing) in self.missing.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{missing}")?;
        }
        Ok(())
    }
}

/// Checks an adapter's capabilities against the renderer's requirements.
///
/// Takes the capability data rather than the adapter so the floor can be
/// exercised for tiers no test machine has.
pub fn probe_capabilities(
    surface_formats: &[wgpu::TextureFormat],
    max_texture_dimension_2d: u32,
    buffer_format_usages: wgpu::TextureUsages,
    buffer_extent: (u32, u32),
) -> CapabilityReport {
    let mut missing = Vec::new();

    if !surface_formats.iter().any(|format| format.is_srgb()) {
        missing.push(MissingCapability::SrgbSurfaceFormat);
    }

    if !buffer_format_usages.contains(REQUIRED_BUFFER_USAGES) {
        missing.push(MissingCapability::BufferUsages {
            needed: REQUIRED_BUFFER_USAGES,
            available: buffer_format_usages,
        });
    }

    let (width, height) = buffer_extent;
    if width == 0 || height == 0 {
        missing.push(MissingCapability::ZeroExtent);
    } else {
        // Report the larger side: naming one axis when both are over is a
        // second round trip for the same machine.
        let needed = width.max(height);
        if needed > max_texture_dimension_2d {
            missing.push(MissingCapability::TextureDimension {
                needed,
                available: max_texture_dimension_2d,
            });
        }
    }

    CapabilityReport { missing }
}

/// Why an interactive window is presenting in software.
///
/// Carried rather than collapsed into a bool because startup diagnostics have
/// to tell these apart: "this machine has no adapter" and "you asked for the
/// software path" are very different things to read in a log
/// (clonk-org/clonk-rs#299).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftwareReason {
    /// The operator asked for it. Available on capable hardware too, because
    /// a fallback nobody can reproduce on their own machine is a fallback
    /// nobody can debug.
    Forced,
    /// No adapter could be created on any backend — the GLES-2-only case the
    /// issue exists for.
    NoAdapter,
    /// An adapter exists but does not meet the floor.
    BelowFloor,
}

impl std::fmt::Display for SoftwareReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Forced => formatter.write_str("software presentation was requested"),
            Self::NoAdapter => formatter.write_str("no GPU adapter could be created"),
            Self::BelowFloor => {
                formatter.write_str("the GPU adapter does not meet the graphics floor")
            }
        }
    }
}

/// How an interactive window should present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationChoice {
    /// Retained GPU presentation. The normal path, and the only one that
    /// carries the optional retained effects.
    Gpu,
    /// The wgpu-free software presenter.
    Software(SoftwareReason),
}

impl PresentationChoice {
    /// True when this choice needs no wgpu instance, adapter or device.
    pub const fn is_software(self) -> bool {
        matches!(self, Self::Software(_))
    }
}

/// Decide how to present, given what the operator asked for and what the
/// adapter can do.
///
/// `adapter` is `None` when no adapter could be created at all, which is a
/// different condition from an adapter that exists and falls short: the first
/// has no capabilities to report, the second has a [`CapabilityReport`]
/// naming exactly what it is missing.
///
/// A forced request wins over a capable adapter on purpose. The issue asks for
/// a diagnostic mode precisely so the software path can be exercised where
/// there is a GPU to compare it against.
pub fn choose_presentation(
    force_software: bool,
    adapter: Option<&CapabilityReport>,
) -> PresentationChoice {
    if force_software {
        return PresentationChoice::Software(SoftwareReason::Forced);
    }
    match adapter {
        None => PresentationChoice::Software(SoftwareReason::NoAdapter),
        Some(report) if !report.is_supported() => {
            PresentationChoice::Software(SoftwareReason::BelowFloor)
        }
        Some(_) => PresentationChoice::Gpu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
    const LINEAR: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

    #[test]
    fn an_adapter_meeting_the_floor_reports_nothing_missing() {
        let report =
            probe_capabilities(&[LINEAR, SRGB], 8192, REQUIRED_BUFFER_USAGES, (1920, 1080));
        assert!(report.is_supported());
        assert_eq!(report.to_string(), "the adapter meets the graphics floor");
    }

    /// The old path picked `Bgra8UnormSrgb` when no sRGB format was offered,
    /// which is not a fallback — it names a format the surface just said it
    /// does not have.
    #[test]
    fn a_surface_without_an_srgb_format_is_below_the_floor() {
        let report = probe_capabilities(&[LINEAR], 8192, REQUIRED_BUFFER_USAGES, (640, 480));
        assert_eq!(report.missing, vec![MissingCapability::SrgbSurfaceFormat]);
    }

    #[test]
    fn a_buffer_larger_than_the_texture_limit_names_both_numbers() {
        let report = probe_capabilities(&[SRGB], 2048, REQUIRED_BUFFER_USAGES, (4096, 1080));
        assert_eq!(
            report.missing,
            vec![MissingCapability::TextureDimension {
                needed: 4096,
                available: 2048,
            }]
        );
        assert!(report.to_string().contains("4096px"));
        assert!(report.to_string().contains("2048px"));
    }

    /// The point of the probe: a machine short of two requirements learns both
    /// from one diagnostic instead of fixing one and being told about the next.
    #[test]
    fn every_missing_requirement_is_reported_together() {
        let report = probe_capabilities(&[LINEAR], 1024, REQUIRED_BUFFER_USAGES, (4096, 4096));
        assert_eq!(
            report.missing,
            vec![
                MissingCapability::SrgbSurfaceFormat,
                MissingCapability::TextureDimension {
                    needed: 4096,
                    available: 1024,
                },
            ]
        );
        let rendered = report.to_string();
        assert!(rendered.contains("no sRGB"), "{rendered}");
        assert!(rendered.contains("max_texture_dimension_2d"), "{rendered}");
    }

    /// The floor is *declared*, not inferred from whatever the renderer
    /// happens to ask for today. This is the gate the issue asks for: a
    /// dependency or renderer change that needs an optional feature, a wider
    /// usage set or a larger texture than GLES 3.0 guarantees fails here
    /// instead of silently raising the bar for every user.
    #[test]
    fn the_declared_floor_is_what_the_renderer_actually_requires() {
        assert_eq!(
            REQUIRED_FEATURES,
            wgpu::Features::empty(),
            "interactive play requires no optional wgpu feature; timestamp \
             queries are opt-in and degrade when absent"
        );
        assert_eq!(
            REQUIRED_BUFFER_USAGES,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            "the frame buffer is uploaded to and sampled, and nothing else"
        );
        assert_eq!(
            MINIMUM_TEXTURE_DIMENSION_2D, 2048,
            "GLES 3.0 and WebGL2 both guarantee 2048, which is what the floor \
             promises and no more"
        );
    }

    /// An adapter whose format features do not cover what the frame buffer is
    /// created with cannot present at all, and says so in the same diagnostic
    /// as everything else it is missing.
    #[test]
    fn a_format_without_the_buffer_usages_is_below_the_floor() {
        let report = probe_capabilities(
            &[SRGB],
            8192,
            wgpu::TextureUsages::TEXTURE_BINDING,
            (640, 480),
        );

        assert_eq!(
            report.missing,
            vec![MissingCapability::BufferUsages {
                needed: REQUIRED_BUFFER_USAGES,
                available: wgpu::TextureUsages::TEXTURE_BINDING,
            }]
        );
        assert!(report.to_string().contains("COPY_DST"), "{report}");
    }

    #[test]
    fn a_zero_extent_is_reported_without_a_dimension_comparison() {
        let report = probe_capabilities(&[SRGB], 8192, REQUIRED_BUFFER_USAGES, (0, 480));
        assert_eq!(report.missing, vec![MissingCapability::ZeroExtent]);
    }

    fn capable() -> CapabilityReport {
        probe_capabilities(&[SRGB], 8192, REQUIRED_BUFFER_USAGES, (640, 480))
    }

    fn below_floor() -> CapabilityReport {
        probe_capabilities(&[LINEAR], 1024, REQUIRED_BUFFER_USAGES, (2048, 480))
    }

    #[test]
    fn a_capable_adapter_presents_on_the_gpu() {
        assert!(capable().is_supported());
        assert_eq!(
            choose_presentation(false, Some(&capable())),
            PresentationChoice::Gpu
        );
        assert!(!PresentationChoice::Gpu.is_software());
    }

    #[test]
    fn no_adapter_and_a_short_adapter_are_reported_as_different_reasons() {
        // Both end in software, but a log that cannot tell them apart cannot
        // tell an operator whether to look at their driver or their hardware.
        assert_eq!(
            choose_presentation(false, None),
            PresentationChoice::Software(SoftwareReason::NoAdapter)
        );
        assert!(!below_floor().is_supported());
        assert_eq!(
            choose_presentation(false, Some(&below_floor())),
            PresentationChoice::Software(SoftwareReason::BelowFloor)
        );
    }

    #[test]
    fn a_forced_request_wins_over_a_capable_adapter() {
        // The diagnostic mode exists to run the software path where there is a
        // GPU to compare it against, so a capable adapter must not override it.
        assert_eq!(
            choose_presentation(true, Some(&capable())),
            PresentationChoice::Software(SoftwareReason::Forced)
        );
        assert_eq!(
            choose_presentation(true, None),
            PresentationChoice::Software(SoftwareReason::Forced)
        );
    }

    #[test]
    fn every_software_reason_reads_as_its_own_sentence() {
        let sentences: Vec<String> = [
            SoftwareReason::Forced,
            SoftwareReason::NoAdapter,
            SoftwareReason::BelowFloor,
        ]
        .into_iter()
        .map(|reason| reason.to_string())
        .collect();
        assert_eq!(
            sentences.len(),
            sentences
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            "a reason that reads the same as another cannot be diagnosed: {sentences:?}"
        );
        assert!(sentences.iter().all(|sentence| !sentence.is_empty()));
    }
}
