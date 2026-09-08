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

use crate::headed_surface_smoke::AdapterEvidence;

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
    generation_before: Option<u64>,
    generation_after: Option<u64>,
    presented_after: u32,
    callback_diagnosis: Option<String>,
    rebuild_ok: Option<bool>,
    recovered_after: Option<Duration>,
    adapter: Option<AdapterEvidence>,
    failure: Option<String>,
    reported: bool,
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
}

impl DeviceLossProbe {
    pub(crate) fn new(report_path: PathBuf, inject_after: u32) -> Self {
        Self {
            report_path,
            inject_after: inject_after.max(1),
            phase: Phase::Armed,
            presented_before: 0,
            injected_at: None,
            generation_before: None,
            generation_after: None,
            presented_after: 0,
            callback_diagnosis: None,
            rebuild_ok: None,
            recovered_after: None,
            adapter: None,
            failure: None,
            reported: false,
        }
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

    /// The software presenter presented a frame.
    pub(crate) fn note_software_presentation(&mut self) -> Option<ProbeStep> {
        (self.phase == Phase::Injected).then(|| {
            self.fail("a software presenter presented after the loss instead of the rebuilt retained renderer")
        })
    }

    /// Called every loop iteration so a loss that never recovers still ends.
    pub(crate) fn check_deadline(&mut self, now: Instant) -> Option<ProbeStep> {
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
        if self.phase != Phase::Recovered {
            let phase = self.phase;
            self.fail(format!(
                "the probe ended in phase {phase:?} without recovering"
            ));
        }
        let report = DeviceLossProbeReport {
            schema_version: 1,
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

#[cfg(test)]
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
    fn a_loss_that_never_recovers_times_out() {
        let mut probe = probe();
        let start = Instant::now();
        assert_eq!(
            probe.check_deadline(start + Duration::from_secs(60)),
            None,
            "nothing is pending before the loss"
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
