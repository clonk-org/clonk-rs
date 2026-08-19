//! Real-window probe for the presenter that uses no GPU adapter.
//!
//! `headed_surface_smoke` validates GPU adapter and driver teardown and quotes
//! `adapter_info()` in its report, so it cannot speak for a presentation path
//! that has no adapter to report (clonk-org/clonk-rs#299). This is the
//! equivalent for that path: open the shell, paint and present a frame, resize
//! the drawable, present again, and exit — reporting what actually happened.
//!
//! Deliberately a separate probe with its own report rather than a mode of the
//! existing one. The two validate different things, and the existing report
//! schema is checked by a script in `scripts/`; widening it to carry
//! "sometimes there is no adapter" would make both harder to read.
//!
//! Resize is the interesting phase. A drawable that is not resized with the
//! window presents a stale or wrongly-sized frame, and unlike the GPU path
//! there is no surface reconfiguration underneath to catch it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, ensure, Context, Result};
use serde::Serialize;
use winit::event_loop::ActiveEventLoop;

use crate::cpu_target::CpuTarget;
use crate::developer_host::DeveloperHost;
use crate::developer_windows::{DeveloperWindows, SHELL_WINDOW};

const SMOKE_TIMEOUT: Duration = Duration::from_secs(15);
const SMOKE_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// How much the probe shrinks the drawable by, in physical pixels.
///
/// A shrink rather than a grow: growing can be silently clamped by the window
/// manager, which would make the resize phase pass without resizing anything.
const RESIZE_DELTA: u32 = 40;

pub(crate) fn prepare(report_path: &Path) -> Result<()> {
    ensure!(
        !report_path
            .try_exists()
            .with_context(|| format!("could not inspect {}", report_path.display()))?,
        "software presentation smoke report already exists: {}",
        report_path.display()
    );
    ensure!(
        crate::main_audio::software_presentation_requested(),
        "the software presentation probe needs LC_SOFTWARE_PRESENTATION set, or it would \
         validate the GPU presenter instead"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmokePhase {
    PresentInitial,
    PresentAfterResize,
    AwaitLoopExit,
    Failed,
}

#[derive(Debug, Serialize)]
struct SmokeReport {
    schema_version: u32,
    kind: &'static str,
    success: bool,
    failure: Option<String>,
    /// The extent presented before the resize, and after it.
    initial_extent: [u32; 2],
    resized_extent: [u32; 2],
    presented_before_resize: bool,
    presented_after_resize: bool,
    /// The shell must still be the only registry entry at teardown: a software
    /// presenter that leaked a window would show up here.
    registry_empty_at_exit: bool,
}

pub(crate) struct SoftwarePresentSmoke {
    report_path: PathBuf,
    phase: SmokePhase,
    deadline: Instant,
    next_retry: Instant,
    shell_os_window_id: winit::window::WindowId,
    initial_extent: [u32; 2],
    resized_extent: [u32; 2],
    presented_before_resize: bool,
    presented_after_resize: bool,
    failure: Option<String>,
}

impl SoftwarePresentSmoke {
    pub(crate) fn start(
        report_path: PathBuf,
        windows: &mut DeveloperWindows<DeveloperHost>,
    ) -> Result<Self> {
        let shell = windows
            .shell_mut()
            .and_then(DeveloperHost::as_shell_mut)
            .context("the software presentation probe needs the live shell")?;
        ensure!(
            shell.software.is_some(),
            "the shell is not presenting in software, so this probe would prove nothing"
        );
        shell.window.set_visible(true);
        let size = shell.window.inner_size();
        let now = Instant::now();
        Ok(Self {
            report_path,
            phase: SmokePhase::PresentInitial,
            deadline: now + SMOKE_TIMEOUT,
            next_retry: now,
            shell_os_window_id: shell.window.id(),
            initial_extent: [size.width, size.height],
            resized_extent: [0, 0],
            presented_before_resize: false,
            presented_after_resize: false,
            failure: None,
        })
    }

    pub(crate) fn about_to_wait(
        &mut self,
        event_loop: &ActiveEventLoop,
        windows: &mut DeveloperWindows<DeveloperHost>,
    ) -> Result<()> {
        if self.phase == SmokePhase::Failed {
            event_loop.exit();
            return Ok(());
        }
        let now = Instant::now();
        if now >= self.deadline {
            return Err(anyhow!(
                "the software presentation probe timed out in phase {:?}",
                self.phase
            ));
        }
        if now >= self.next_retry {
            windows.request_redraw_visible();
            self.next_retry = now + SMOKE_RETRY_INTERVAL;
        }
        Ok(())
    }

    pub(crate) fn redraw(
        &mut self,
        os_window_id: winit::window::WindowId,
        event_loop: &ActiveEventLoop,
        windows: &mut DeveloperWindows<DeveloperHost>,
    ) -> Result<()> {
        if self.phase == SmokePhase::Failed {
            event_loop.exit();
            return Ok(());
        }
        if os_window_id != self.shell_os_window_id {
            return Ok(());
        }
        match self.phase {
            SmokePhase::PresentInitial => {
                self.presented_before_resize |= present_shell(windows, [0x2f, 0x6f, 0xa8, 0xff])?;
                if self.presented_before_resize {
                    self.resized_extent = resize_shell(windows, self.initial_extent)?;
                    self.phase = SmokePhase::PresentAfterResize;
                    windows.request_redraw(SHELL_WINDOW);
                }
            }
            SmokePhase::PresentAfterResize => {
                self.presented_after_resize |= present_shell(windows, [0xa8, 0x6f, 0x2f, 0xff])?;
                if self.presented_after_resize {
                    self.phase = SmokePhase::AwaitLoopExit;
                    event_loop.exit();
                }
            }
            SmokePhase::AwaitLoopExit => event_loop.exit(),
            SmokePhase::Failed => unreachable!("failed probes exit above"),
        }
        Ok(())
    }

    pub(crate) fn fail(&mut self, error: &anyhow::Error) {
        self.phase = SmokePhase::Failed;
        self.failure = Some(format!("{error:#}"));
    }

    pub(crate) fn finish(&mut self, registry_empty: bool) -> Result<()> {
        let success = self.failure.is_none()
            && self.presented_before_resize
            && self.presented_after_resize
            && self.resized_extent != self.initial_extent
            && registry_empty;
        let report = SmokeReport {
            schema_version: 1,
            kind: "clonk_software_present_smoke",
            success,
            failure: self.failure.clone(),
            initial_extent: self.initial_extent,
            resized_extent: self.resized_extent,
            presented_before_resize: self.presented_before_resize,
            presented_after_resize: self.presented_after_resize,
            registry_empty_at_exit: registry_empty,
        };
        let encoded =
            serde_json::to_vec_pretty(&report).context("serialize software presentation report")?;
        let mut file = std::fs::File::create(&self.report_path)
            .with_context(|| format!("create {}", self.report_path.display()))?;
        file.write_all(&encoded)
            .and_then(|()| file.flush())
            .with_context(|| format!("write {}", self.report_path.display()))?;
        ensure!(
            success,
            "software presentation smoke failed: {}",
            self.failure
                .clone()
                .unwrap_or_else(|| "the probe did not complete every phase".to_string())
        );
        Ok(())
    }
}

/// Paint the whole frame one colour and present it.
///
/// A flat fill is deliberate: the probe is asking whether pixels reach the
/// window at all, and a uniform frame makes a partial or stale present obvious
/// to anyone looking at the screen while it runs.
fn present_shell(windows: &mut DeveloperWindows<DeveloperHost>, color: [u8; 4]) -> Result<bool> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software presentation probe's shell disappeared")?;
    let presenter = shell
        .software
        .as_mut()
        .context("the software presentation probe's presenter disappeared")?;
    let mut target = CpuTarget::Software(presenter);
    for pixel in target.frame_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&color);
    }
    target
        .present()
        .context("failed to present the software presentation probe's frame")
}

/// Shrink the drawable and report the extent that took effect.
fn resize_shell(windows: &mut DeveloperWindows<DeveloperHost>, from: [u32; 2]) -> Result<[u32; 2]> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software presentation probe's shell disappeared before resize")?;
    let resized = [
        from[0].saturating_sub(RESIZE_DELTA).max(1),
        from[1].saturating_sub(RESIZE_DELTA).max(1),
    ];
    let presenter = shell
        .software
        .as_mut()
        .context("the software presentation probe's presenter disappeared before resize")?;
    presenter
        .resize_drawable((resized[0], resized[1]))
        .context("failed to resize the software presentation drawable")?;
    presenter.resize_frame((resized[0], resized[1]))?;
    Ok(resized)
}
