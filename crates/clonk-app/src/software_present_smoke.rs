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
    PresentLetterboxed,
    PresentRestored,
    AwaitLoopExit,
    Failed,
}

/// One presented phase, with everything needed to tell a correct present
/// from one that used the extent before the transition.
#[derive(Debug, Clone, Serialize)]
struct PresentedPhase {
    name: &'static str,
    /// What the renderer draws at, and what the window presents into. A
    /// transition changes the second without changing the first, which is the
    /// only case that produces a scale above one and a letterbox.
    frame_extent: [u32; 2],
    drawable_extent: [u32; 2],
    scale: u32,
    /// Where the scaled frame lands: x, y, width, height.
    clip_rect: [u32; 4],
    presented: bool,
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
    /// Every phase the probe presented, in order.
    phases: Vec<PresentedPhase>,
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
    phases: Vec<PresentedPhase>,
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
            phases: Vec::new(),
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
                    let recorded = record_phase(windows, "windowed", true)?;
                    self.phases.push(recorded);
                    // The presenter-visible shape of going fullscreen: the
                    // drawable grows and the frame does not, so the frame is
                    // scaled up and letterboxed rather than stretched.
                    set_drawable_holding_frame(
                        windows,
                        [
                            self.resized_extent[0].saturating_mul(2).max(1),
                            self.resized_extent[1].saturating_mul(2).max(1),
                        ],
                    )?;
                    self.phase = SmokePhase::PresentLetterboxed;
                    windows.request_redraw(SHELL_WINDOW);
                }
            }
            SmokePhase::PresentLetterboxed => {
                let presented = present_shell(windows, [0x2f, 0xa8, 0x6f, 0xff])?;
                if presented {
                    let recorded = record_phase(windows, "fullscreen", true)?;
                    self.phases.push(recorded);
                    // And back to a window. A presenter that kept the previous
                    // transform would now scale and crop for a drawable twice
                    // the size of the one it is presenting into.
                    set_drawable_holding_frame(windows, self.resized_extent)?;
                    self.phase = SmokePhase::PresentRestored;
                    windows.request_redraw(SHELL_WINDOW);
                }
            }
            SmokePhase::PresentRestored => {
                let presented = present_shell(windows, [0x6f, 0x2f, 0xa8, 0xff])?;
                if presented {
                    let recorded = record_phase(windows, "windowed-again", true)?;
                    self.phases.push(recorded);
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
        // Each phase must have presented through its own drawable: the
        // recorded clip has to fit the drawable it was computed for, and the
        // fullscreen phase has to have actually scaled, or the sequence proved
        // nothing about a transition.
        let phases_fit = self.phases.iter().all(|phase| {
            phase.presented
                && phase.clip_rect[0] + phase.clip_rect[2] <= phase.drawable_extent[0]
                && phase.clip_rect[1] + phase.clip_rect[3] <= phase.drawable_extent[1]
        });
        let letterboxed_scaled = self
            .phases
            .iter()
            .find(|phase| phase.name == "fullscreen")
            .is_some_and(|phase| phase.scale > 1);
        let success = self.failure.is_none()
            && self.presented_before_resize
            && self.presented_after_resize
            && self.resized_extent != self.initial_extent
            && self.phases.len() == 3
            && phases_fit
            && letterboxed_scaled
            && registry_empty;
        let report = SmokeReport {
            schema_version: 2,
            kind: "clonk_software_present_smoke",
            success,
            failure: self.failure.clone(),
            initial_extent: self.initial_extent,
            resized_extent: self.resized_extent,
            presented_before_resize: self.presented_before_resize,
            presented_after_resize: self.presented_after_resize,
            phases: self.phases.clone(),
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

/// Grow the drawable while holding the frame, which is what a windowed to
/// fullscreen transition does to the presenter.
///
/// The existing resize moves both together, so the scale stays one and nothing
/// is ever letterboxed -- it cannot catch a wrong scale or a crop computed from
/// the extent before the transition. Only a drawable that changes on its own
/// produces those.
fn set_drawable_holding_frame(
    windows: &mut DeveloperWindows<DeveloperHost>,
    drawable: [u32; 2],
) -> Result<()> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software presentation probe's shell disappeared before a transition")?;
    let presenter = shell
        .software
        .as_mut()
        .context("the software presentation probe's presenter disappeared before a transition")?;
    presenter
        .resize_drawable((drawable[0], drawable[1]))
        .context("failed to resize the software presentation drawable for a transition")
}

/// What the presenter would put on screen right now.
fn record_phase(
    windows: &mut DeveloperWindows<DeveloperHost>,
    name: &'static str,
    presented: bool,
) -> Result<PresentedPhase> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software presentation probe's shell disappeared before recording")?;
    let presenter = shell
        .software
        .as_ref()
        .context("the software presentation probe's presenter disappeared before recording")?;
    let frame = presenter.frame_extent();
    let drawable = presenter.drawable_extent();
    let transform = clonk_surface::BlitTransform::pixel_perfect(frame, drawable);
    let (clip_x, clip_y, clip_width, clip_height) = transform.clip_rect();
    Ok(PresentedPhase {
        name,
        frame_extent: [frame.0, frame.1],
        drawable_extent: [drawable.0, drawable.1],
        scale: transform.scale(),
        clip_rect: [clip_x, clip_y, clip_width, clip_height],
        presented,
    })
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
