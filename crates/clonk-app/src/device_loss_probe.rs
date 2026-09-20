//! Real-window device-loss recovery probe (clonk-org/clonk-rs#1241).
//!
//! Unit tests inject `Lost`/`Outdated` surface states and call
//! `RetainedGpuRenderer::recreate` with replacement devices; none of them
//! loses a live device under the shipped event loop. This probe stays inside
//! that loop: after a configured number of retained presentations it destroys
//! the live `wgpu::Device` (a backend-authoritative loss, reported to the
//! device-lost callback as `Destroyed`), then watches the ordinary recovery
//! path rebuild the device and surface, recreate the renderer at a new
//! generation, and present again. It writes its report when it has seen the
//! presentations it wanted, when it gives up, or when the loop exits early.

use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use crate::gpu_renderer::{GpuReadbackFrame, GpuRendererStats, RetainedGpuRenderer};
use crate::headed_surface_smoke::AdapterEvidence;
use clonk_surface::WindowSurface;

/// Retained presentations to let through before the device is destroyed.
pub(crate) const DEFAULT_INJECT_AFTER_FRAMES: u32 = 30;
/// Presentations on the replacement generation that count as recovery.
const PRESENTATIONS_AFTER_RECOVERY: u32 = 3;
/// How long the loop may take from the injected loss to those presentations.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn prepare(report_path: &Path) -> Result<()> {
    ensure!(
        !report_path
            .try_exists()
            .with_context(|| format!("could not inspect {}", report_path.display()))?,
        "device-loss probe report already exists: {}",
        report_path.display()
    );
    let requested = wgpu::Backends::from_env().context(
        "the device-loss probe requires one explicit WGPU_BACKEND, so the report names the backend it qualified",
    )?;
    ensure!(
        requested.bits().count_ones() == 1,
        "the device-loss probe requires exactly one WGPU_BACKEND, got {requested:?}"
    );
    Ok(())
}

/// What the event loop must do after reporting a presentation to the probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProbeStep {
    /// Destroy the live device now; the next present must fail into recovery.
    Inject,
    /// The replacement generation presented enough; write the report and exit.
    Recovered,
    /// The loss was mishandled or timed out; write the report and exit.
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Armed,
    Injected,
    Recovered,
    Failed,
}

pub(crate) struct DeviceLossProbe {
    report_path: PathBuf,
    inject_after: u32,
    phase: Phase,
    presented_before: u32,
    injected_at: Option<Instant>,
    started_at: Instant,
    generation_before: Option<u64>,
    generation_after: Option<u64>,
    presented_after: u32,
    callback_diagnosis: Option<String>,
    rebuild_ok: Option<bool>,
    recovered_after: Option<Duration>,
    adapter: Option<AdapterEvidence>,
    failure: Option<String>,
    reported: bool,
    reference_frame: Option<GpuReadbackFrame>,
    reference_textures: Vec<clonk_graphics::GpuTextureId>,
    reference_resident_textures: usize,
    resource_recovery: Option<ResourceRecoveryEvidence>,
    surface_counts_at_drop: Option<[usize; 2]>,
}

#[derive(Clone, Debug, Serialize)]
struct ResourceRecoveryEvidence {
    extent: [u32; 2],
    texture_ids_before: Vec<u64>,
    texture_ids_after: Vec<u64>,
    resident_textures_before: usize,
    textures_before: usize,
    textures_after: usize,
    recreated_textures: usize,
    full_upload_calls: usize,
    full_upload_bytes: u64,
    pixels_identical: bool,
}

fn validate_recovered_frame(
    before: &GpuReadbackFrame,
    after: &GpuReadbackFrame,
    textures_before: usize,
    stats: &GpuRendererStats,
) -> Result<()> {
    ensure!(before == after, "the first recovered frame differs from the pre-loss frame; use an idle, static screen for qualification");
    ensure!(
        textures_before > 0
            && stats.resident_source_textures == textures_before
            && stats.created_source_textures == textures_before
            && stats.full_upload_calls >= textures_before
            && stats.full_upload_bytes > 0,
        "the replacement device did not recreate and upload every presented source texture: \
         expected {textures_before}, resident {}, created {}, full uploads {}, bytes {}",
        stats.resident_source_textures,
        stats.created_source_textures,
        stats.full_upload_calls,
        stats.full_upload_bytes,
    );
    Ok(())
}

#[derive(Debug, Serialize)]
struct DeviceLossProbeReport {
    schema_version: u32,
    kind: &'static str,
    success: bool,
    failure: Option<String>,
    port_version: &'static str,
    os: &'static str,
    arch: &'static str,
    adapter: Option<AdapterEvidence>,
    inject_after_presentations: u32,
    presented_before_loss: u32,
    generation_before: Option<u64>,
    generation_after: Option<u64>,
    callback_diagnosis: Option<String>,
    rebuild_ok: Option<bool>,
    presented_after_recovery: u32,
    recovery_ms: Option<u128>,
    resource_recovery: Option<ResourceRecoveryEvidence>,
    surface_counts_at_drop: Option<[usize; 2]>,
}

impl DeviceLossProbe {
    pub(crate) fn new(report_path: PathBuf, inject_after: u32) -> Self {
        Self {
            report_path,
            inject_after: inject_after.max(1),
            phase: Phase::Armed,
            presented_before: 0,
            injected_at: None,
            started_at: Instant::now(),
            generation_before: None,
            generation_after: None,
            presented_after: 0,
            callback_diagnosis: None,
            rebuild_ok: None,
            recovered_after: None,
            adapter: None,
            failure: None,
            reported: false,
            reference_frame: None,
            reference_textures: Vec::new(),
            reference_resident_textures: 0,
            resource_recovery: None,
            surface_counts_at_drop: None,
        }
    }

    /// Inspect the composition actually presented, without requesting another
    /// scene or changing the normal renderer's resource-restoration path.
    pub(crate) fn observe_retained_presentation(
        &mut self,
        renderer: &RetainedGpuRenderer,
        surface: &WindowSurface,
        now: Instant,
    ) -> Option<ProbeStep> {
        if self.phase == Phase::Injected
            && renderer.generation() <= self.generation_before.unwrap_or(0)
        {
            return self.note_retained_presentation(renderer.generation(), now);
        }
        if (self.phase == Phase::Armed && self.presented_before + 1 >= self.inject_after)
            || (self.phase == Phase::Injected && self.presented_after == 0)
        {
            if let Err(error) = self.capture_presented_frame(renderer, surface) {
                return Some(
                    self.fail(format!("resource recovery verification failed: {error:#}")),
                );
            }
        }
        self.note_retained_presentation(renderer.generation(), now)
    }

    fn capture_presented_frame(
        &mut self,
        renderer: &RetainedGpuRenderer,
        surface: &WindowSurface,
    ) -> Result<()> {
        let mut encoder =
            surface
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("device_loss_probe_readback"),
                });
        let ticket = renderer
            .readback_last_presentation(surface.device(), &mut encoder)?
            .context("the renderer has no presented composition")?;
        surface.queue().submit([encoder.finish()]);
        let frame = ticket.read(surface.device())?;
        let stats = renderer.last_stats();
        let texture_ids = renderer.last_scene_texture_ids();
        let suffix = if self.phase == Phase::Armed {
            "before.png"
        } else {
            "after.png"
        };
        let path = self.report_path.with_extension(suffix);
        let png =
            crate::main_resources::encode_rgba_png(frame.extent[0], frame.extent[1], &frame.rgba)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?
            .write_all(&png)?;
        if self.phase == Phase::Armed {
            ensure!(
                !texture_ids.is_empty(),
                "the pre-loss frame used no source textures"
            );
            self.reference_textures = texture_ids;
            self.reference_resident_textures = stats.resident_source_textures;
            self.reference_frame = Some(frame);
        } else {
            let reference = self
                .reference_frame
                .as_ref()
                .context("no pre-loss frame was captured")?;
            self.generation_after = Some(renderer.generation());
            self.resource_recovery = Some(ResourceRecoveryEvidence {
                extent: frame.extent,
                texture_ids_before: self.reference_textures.iter().map(|id| id.get()).collect(),
                texture_ids_after: texture_ids.iter().map(|id| id.get()).collect(),
                resident_textures_before: self.reference_resident_textures,
                textures_before: self.reference_textures.len(),
                textures_after: stats.resident_source_textures,
                recreated_textures: stats.created_source_textures,
                full_upload_calls: stats.full_upload_calls,
                full_upload_bytes: stats.full_upload_bytes,
                pixels_identical: reference == &frame,
            });
            ensure!(
                texture_ids == self.reference_textures,
                "the first recovered scene uses different source texture identities"
            );
            validate_recovered_frame(reference, &frame, self.reference_textures.len(), &stats)?;
        }
        Ok(())
    }

    /// A retained presentation completed on renderer `generation`.
    pub(crate) fn note_retained_presentation(
        &mut self,
        generation: u64,
        now: Instant,
    ) -> Option<ProbeStep> {
        match self.phase {
            Phase::Armed => {
                self.presented_before += 1;
                (self.presented_before >= self.inject_after).then(|| {
                    self.phase = Phase::Injected;
                    self.injected_at = Some(now);
                    self.generation_before = Some(generation);
                    ProbeStep::Inject
                })
            }
            Phase::Injected => {
                let before = self.generation_before.unwrap_or(0);
                if generation <= before {
                    return Some(self.fail(format!(
                        "a frame presented on generation {generation} after the device was destroyed: the loss was ignored or the old swapchain was retained"
                    )));
                }
                if self.rebuild_ok != Some(true)
                    || !self
                        .callback_diagnosis
                        .as_deref()
                        .is_some_and(|diagnosis| diagnosis.contains("Destroyed"))
                {
                    return Some(self.fail("a replacement presented without an observed device loss and successful rebuild"));
                }
                self.generation_after = Some(generation);
                self.presented_after += 1;
                (self.presented_after >= PRESENTATIONS_AFTER_RECOVERY).then(|| {
                    self.phase = Phase::Recovered;
                    self.recovered_after = self.injected_at.map(|at| now.duration_since(at));
                    ProbeStep::Recovered
                })
            }
            Phase::Recovered | Phase::Failed => None,
        }
    }

    /// The loop rebuilt the device after the loss: what the callback said and
    /// whether recreation succeeded.
    pub(crate) fn note_rebuild(&mut self, diagnosis: String, ok: bool) {
        if self.phase != Phase::Injected {
            return;
        }
        self.callback_diagnosis.get_or_insert(diagnosis);
        self.rebuild_ok = Some(ok);
        if !ok {
            self.fail("the device rebuild after the loss failed");
        }
    }

    pub(crate) fn note_surface_drop(
        &mut self,
        before: Option<usize>,
        after: Option<usize>,
    ) -> Result<()> {
        let before = before.context("wgpu cannot report the old surface lifetime")?;
        let after = after.context("wgpu cannot report surface destruction")?;
        self.surface_counts_at_drop = Some([before, after]);
        ensure!(
            before > 0 && after == before - 1,
            "the old configured surface is still live before replacement: {before} -> {after}"
        );
        Ok(())
    }

    /// The software presenter presented a frame.
    pub(crate) fn note_software_presentation(&mut self) -> Option<ProbeStep> {
        (self.phase == Phase::Injected).then(|| {
            self.fail("a software presenter presented after the loss instead of the rebuilt retained renderer")
        })
    }

    /// Called every loop iteration so a loss that never recovers still ends.
    pub(crate) fn check_deadline(&mut self, now: Instant) -> Option<ProbeStep> {
        if self.phase == Phase::Armed && now.duration_since(self.started_at) > RECOVERY_TIMEOUT {
            return Some(self.fail(format!(
                "only {} retained presentations before injection within {} s; the window must be visible",
                self.presented_before, RECOVERY_TIMEOUT.as_secs()
            )));
        }
        let waited = self.injected_at.filter(|_| self.phase == Phase::Injected)?;
        (now.duration_since(waited) > RECOVERY_TIMEOUT).then(|| {
            self.fail(format!(
                "no presentation on a replacement generation within {} s of the injected loss",
                RECOVERY_TIMEOUT.as_secs()
            ))
        })
    }

    pub(crate) fn record_adapter(&mut self, info: &wgpu::AdapterInfo) {
        self.adapter = Some(AdapterEvidence::from_info(info));
    }

    pub(crate) fn succeeded(&self) -> bool {
        self.phase == Phase::Recovered && self.failure.is_none()
    }

    fn fail(&mut self, message: impl Into<String>) -> ProbeStep {
        self.failure.get_or_insert_with(|| message.into());
        self.phase = Phase::Failed;
        ProbeStep::Failed
    }

    /// Write the report and set the process exit code: 0 for a recovery, 2 for
    /// a mishandled loss, 1 when the report itself could not be written.
    pub(crate) fn conclude(&mut self, exit_code: &AtomicI32) {
        match self.finish() {
            Ok(()) => exit_code.store(if self.succeeded() { 0 } else { 2 }, Ordering::Relaxed),
            Err(error) => {
                tracing::error!(%error, "device-loss probe report failed");
                exit_code.store(1, Ordering::Relaxed);
            }
        }
    }

    /// Write the report once and print the one-line verdict.
    pub(crate) fn finish(&mut self) -> Result<()> {
        if self.reported {
            return Ok(());
        }
        self.reported = true;
        if self.phase == Phase::Recovered
            && (self.resource_recovery.is_none() || self.surface_counts_at_drop.is_none())
        {
            self.fail("recovery lacked surface destruction, resource uploads, or matching presented pixels");
        }
        if self.phase != Phase::Recovered {
            let phase = self.phase;
            self.fail(format!(
                "the probe ended in phase {phase:?} without recovering"
            ));
        }
        let report = DeviceLossProbeReport {
            schema_version: 2,
            kind: "clonk_device_loss_probe",
            success: self.succeeded(),
            failure: self.failure.clone(),
            port_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            adapter: self.adapter.clone(),
            inject_after_presentations: self.inject_after,
            presented_before_loss: self.presented_before,
            generation_before: self.generation_before,
            generation_after: self.generation_after,
            callback_diagnosis: self.callback_diagnosis.clone(),
            rebuild_ok: self.rebuild_ok,
            presented_after_recovery: self.presented_after,
            recovery_ms: self.recovered_after.map(|elapsed| elapsed.as_millis()),
            resource_recovery: self.resource_recovery.clone(),
            surface_counts_at_drop: self.surface_counts_at_drop,
        };
        let bytes = serde_json::to_vec_pretty(&report)
            .context("failed to serialize the device-loss probe report")?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.report_path)
            .with_context(|| {
                format!(
                    "failed to create device-loss probe report {}",
                    self.report_path.display()
                )
            })?;
        file.write_all(&bytes).with_context(|| {
            format!(
                "failed to write device-loss probe report {}",
                self.report_path.display()
            )
        })?;
        println!(
            "LC_APP_DEVICE_LOSS_PROBE result={} backend={} generation_before={} generation_after={} presented_after={} recovery_ms={} report={}",
            if report.success { "recovered" } else { "fail" },
            report.adapter.as_ref().map_or("unknown", AdapterEvidence::backend),
            report.generation_before.map_or("-".to_owned(), |g| g.to_string()),
            report.generation_after.map_or("-".to_owned(), |g| g.to_string()),
            report.presented_after_recovery,
            report.recovery_ms.map_or("-".to_owned(), |ms| ms.to_string()),
            self.report_path.display(),
        );
        Ok(())
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    fn probe() -> DeviceLossProbe {
        DeviceLossProbe::new(PathBuf::from("unused.json"), 2)
    }

    #[test]
    fn the_device_is_destroyed_on_the_configured_presentation() {
        let mut probe = probe();
        let start = Instant::now();
        assert_eq!(probe.note_retained_presentation(1, start), None);
        assert_eq!(
            probe.note_retained_presentation(1, start),
            Some(ProbeStep::Inject)
        );
        assert_eq!(probe.generation_before, Some(1));
        assert_eq!(probe.presented_before, 2);
    }

    #[test]
    fn three_presentations_on_a_newer_generation_are_a_recovery() {
        let mut probe = probe();
        let start = Instant::now();
        probe.note_retained_presentation(1, start);
        probe.note_retained_presentation(1, start);
        probe.note_rebuild("Destroyed".to_owned(), true);
        let later = start + Duration::from_millis(120);
        assert_eq!(probe.note_retained_presentation(2, later), None);
        assert_eq!(probe.note_retained_presentation(2, later), None);
        assert_eq!(
            probe.note_retained_presentation(2, later),
            Some(ProbeStep::Recovered)
        );
        assert!(probe.succeeded());
        assert_eq!(probe.generation_after, Some(2));
        assert_eq!(probe.presented_after, 3);
        assert_eq!(probe.recovered_after, Some(Duration::from_millis(120)));
        assert_eq!(probe.callback_diagnosis.as_deref(), Some("Destroyed"));
        assert_eq!(probe.rebuild_ok, Some(true));
    }

    #[test]
    fn unchanged_pixels_without_restored_textures_do_not_qualify() {
        let frame = crate::gpu_renderer::GpuReadbackFrame {
            extent: [1, 1],
            rgba: vec![12, 34, 56, 255],
        };
        assert!(validate_recovered_frame(
            &frame,
            &frame,
            1,
            &crate::gpu_renderer::GpuRendererStats::default(),
        )
        .is_err());
    }

    #[test]
    fn a_new_generation_without_observing_the_loss_is_not_recovery() {
        let mut probe = probe();
        let start = Instant::now();
        probe.note_retained_presentation(1, start);
        probe.note_retained_presentation(1, start);
        assert_eq!(
            probe.note_retained_presentation(2, start),
            Some(ProbeStep::Failed)
        );
        assert!(!probe.succeeded());
    }

    #[test]
    fn a_surface_error_without_the_destroyed_callback_is_not_recovery() {
        let mut probe = probe();
        let start = Instant::now();
        probe.note_retained_presentation(1, start);
        probe.note_retained_presentation(1, start);
        probe.note_rebuild("surface was lost".to_owned(), true);
        assert_eq!(
            probe.note_retained_presentation(2, start),
            Some(ProbeStep::Failed)
        );
    }

    #[test]
    fn a_presentation_on_the_destroyed_generation_is_an_ignored_loss() {
        let mut probe = probe();
        let start = Instant::now();
        probe.note_retained_presentation(1, start);
        probe.note_retained_presentation(1, start);
        assert_eq!(
            probe.note_retained_presentation(1, start),
            Some(ProbeStep::Failed)
        );
        assert!(!probe.succeeded());
        assert!(probe
            .failure
            .as_deref()
            .is_some_and(|f| f.contains("generation 1")));
    }

    #[test]
    fn a_software_presentation_after_the_loss_is_an_unrequested_fallback() {
        let mut probe = probe();
        let start = Instant::now();
        assert_eq!(
            probe.note_software_presentation(),
            None,
            "before the loss it is not the probe's concern"
        );
        probe.note_retained_presentation(1, start);
        probe.note_retained_presentation(1, start);
        assert_eq!(probe.note_software_presentation(), Some(ProbeStep::Failed));
        assert!(probe
            .failure
            .as_deref()
            .is_some_and(|f| f.contains("software")));
    }

    #[test]
    fn a_window_that_never_presents_times_out_before_injection() {
        let mut probe = probe();
        assert_eq!(
            probe.check_deadline(Instant::now() + Duration::from_secs(16)),
            Some(ProbeStep::Failed)
        );
        assert_eq!(probe.presented_before, 0);
    }

    #[test]
    fn a_loss_that_never_recovers_times_out() {
        let mut probe = probe();
        let start = Instant::now();
        assert_eq!(
            probe.check_deadline(start + Duration::from_secs(1)),
            None,
            "the startup deadline has not elapsed"
        );
        probe.note_retained_presentation(1, start);
        probe.note_retained_presentation(1, start);
        assert_eq!(probe.check_deadline(start + Duration::from_secs(14)), None);
        assert_eq!(
            probe.check_deadline(start + Duration::from_secs(16)),
            Some(ProbeStep::Failed)
        );
        assert!(probe.failure.as_deref().is_some_and(|f| f.contains("15")));
    }
}
