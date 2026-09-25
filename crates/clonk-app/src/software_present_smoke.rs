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
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use serde::Serialize;
use winit::event_loop::{ActiveEventLoop, ControlFlow};

use crate::cpu_target::CpuTarget;
use crate::developer_host::DeveloperHost;
use crate::developer_windows::{DeveloperWindows, SHELL_WINDOW};

const SMOKE_TIMEOUT: Duration = Duration::from_secs(15);
const SMOKE_RETRY_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn prepare(report_path: &Path) -> Result<()> {
    ensure!(
        !report_path
            .try_exists()
            .with_context(|| format!("could not inspect {}", report_path.display()))?,
        "software presentation smoke report already exists: {}",
        report_path.display()
    );
    ensure!(
        crate::main_audio::software_presentation_requested()
            || wgpu::Backends::from_env().is_some_and(|backends| backends.is_empty()),
        "the software presentation probe needs LC_SOFTWARE_PRESENTATION set or an \
         explicitly empty WGPU_BACKEND to exercise automatic fallback"
    );
    crate::gpu_instance::begin_retained_instance_evidence_capture();
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmokePhase {
    PresentInitial,
    AwaitWindowResize,
    PresentAfterResize,
    AwaitPointer,
    BeginLetterboxed,
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

#[derive(Debug, Clone, Serialize)]
struct PointerMapping {
    window_position: [f64; 2],
    gui_position: [f32; 2],
    scale: f32,
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
    input_mapping: Option<PointerMapping>,
    software_reason: &'static str,
    gpu_attempt_backends: Vec<Vec<&'static str>>,
    display_backend: &'static str,
    target_os: &'static str,
    target_arch: &'static str,
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
    check_input: bool,
    previous_scale: f32,
    pointer_position: Option<[f64; 2]>,
    input_mapping: Option<PointerMapping>,
    software_reason: &'static str,
    gpu_attempt_backends: Vec<Vec<&'static str>>,
    display_backend: &'static str,
}

impl SoftwarePresentSmoke {
    pub(crate) fn start(
        report_path: PathBuf,
        windows: &mut DeveloperWindows<DeveloperHost>,
        check_input: bool,
        choice: clonk_surface::capability::PresentationChoice,
    ) -> Result<Self> {
        tracing::info!(check_input, "starting software presentation probe");
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
        let display_backend = match shell.window.display_handle()?.as_raw() {
            RawDisplayHandle::Windows(_) => "windows",
            RawDisplayHandle::AppKit(_) => "appkit",
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => "x11",
            RawDisplayHandle::Wayland(_) => "wayland",
            _ => "unknown",
        };
        let now = Instant::now();
        let software_reason = match choice {
            clonk_surface::capability::PresentationChoice::Software(reason) => match reason {
                clonk_surface::capability::SoftwareReason::Forced => "forced",
                clonk_surface::capability::SoftwareReason::NoAdapter => "no-adapter",
                clonk_surface::capability::SoftwareReason::BelowFloor => "below-floor",
            },
            clonk_surface::capability::PresentationChoice::Gpu => {
                return Err(anyhow!("the software probe selected GPU presentation"));
            }
        };
        let gpu_attempt_backends = crate::gpu_instance::retained_instance_registry_evidence()
            .acquisitions
            .into_iter()
            .map(|attempt| {
                wgpu::Backend::ALL
                    .into_iter()
                    .filter(|backend| attempt.backends.contains((*backend).into()))
                    .map(wgpu::Backend::to_str)
                    .collect()
            })
            .collect();
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
            check_input,
            previous_scale: shell.presenter.scale(),
            pointer_position: None,
            input_mapping: None,
            software_reason,
            gpu_attempt_backends,
            display_backend,
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
                "the software presentation probe timed out in phase {:?}; native pointer: {:?}",
                self.phase,
                self.pointer_position
            ));
        }
        if now >= self.next_retry {
            windows.request_redraw_visible();
            self.next_retry = now + SMOKE_RETRY_INTERVAL;
        }
        // This probe consumes AboutToWait before the normal frame scheduler.
        // Without a wakeup, an idle Windows loop never retries or times out.
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_retry.min(self.deadline)));
        Ok(())
    }

    pub(crate) fn redraw(
        &mut self,
        os_window_id: winit::window::WindowId,
        event_loop: &ActiveEventLoop,
        windows: &mut DeveloperWindows<DeveloperHost>,
        app: &mut crate::GameApp,
    ) -> Result<()> {
        if self.phase == SmokePhase::Failed {
            event_loop.exit();
            return Ok(());
        }
        if os_window_id != self.shell_os_window_id {
            return Ok(());
        }
        let previous_phase = self.phase;
        match self.phase {
            SmokePhase::PresentInitial => {
                self.presented_before_resize |= present_shell(windows, [0x2f, 0x6f, 0xa8, 0xff])?;
                if self.presented_before_resize {
                    maximize_shell(windows)?;
                    self.phase = SmokePhase::AwaitWindowResize;
                    windows.request_redraw(SHELL_WINDOW);
                }
            }
            SmokePhase::AwaitWindowResize => {
                if let Some(extent) = shell_resize_followed(windows, self.initial_extent)? {
                    self.resized_extent = extent;
                    self.phase = SmokePhase::PresentAfterResize;
                    windows.request_redraw(SHELL_WINDOW);
                }
            }
            SmokePhase::PresentAfterResize => {
                self.presented_after_resize |= present_shell(windows, [0xa8, 0x6f, 0x2f, 0xff])?;
                if self.presented_after_resize {
                    let recorded = record_phase(windows, "windowed", true)?;
                    self.phases.push(recorded);
                    self.phase = if self.check_input {
                        set_input_scale(windows, app, 2.0)?;
                        app.input_routing.live.window_pointer = None;
                        let shell = windows
                            .shell_mut()
                            .and_then(DeveloperHost::as_shell_mut)
                            .context("the software probe lost its shell before pointer input")?;
                        self.phase = SmokePhase::AwaitPointer;
                        shell.window.focus_window();
                        shell
                            .window
                            .set_cursor_position(winit::dpi::PhysicalPosition::new(128.0, 96.0))?;
                        SmokePhase::AwaitPointer
                    } else {
                        SmokePhase::BeginLetterboxed
                    };
                    windows.request_redraw(SHELL_WINDOW);
                }
            }
            SmokePhase::AwaitPointer => {
                if let (Some(position), Some(point)) =
                    (self.pointer_position, app.input_routing.live.window_pointer)
                {
                    if position == [128.0, 96.0] && point.x == 64.0 && point.y == 48.0 {
                        self.input_mapping = Some(PointerMapping {
                            window_position: position,
                            gui_position: [point.x, point.y],
                            scale: 2.0,
                        });
                        set_input_scale(windows, app, self.previous_scale)?;
                        self.phase = SmokePhase::BeginLetterboxed;
                        windows.request_redraw(SHELL_WINDOW);
                    }
                }
            }
            SmokePhase::BeginLetterboxed => {
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
                    save_captures(&self.report_path, windows, app)?;
                    self.phase = SmokePhase::AwaitLoopExit;
                    event_loop.exit();
                }
            }
            SmokePhase::AwaitLoopExit => event_loop.exit(),
            SmokePhase::Failed => unreachable!("failed probes exit above"),
        }
        if self.phase != previous_phase {
            tracing::info!(?previous_phase, phase = ?self.phase, "software presentation probe advanced");
        }
        Ok(())
    }

    pub(crate) fn fail(&mut self, error: &anyhow::Error) {
        self.phase = SmokePhase::Failed;
        self.failure = Some(format!("{error:#}"));
    }

    pub(crate) fn note_pointer(
        &mut self,
        window_id: winit::window::WindowId,
        position: winit::dpi::PhysicalPosition<f64>,
    ) {
        if window_id == self.shell_os_window_id && self.phase == SmokePhase::AwaitPointer {
            self.pointer_position = Some([position.x, position.y]);
        }
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
            && (!self.check_input || self.input_mapping.is_some())
            && registry_empty;
        let report = SmokeReport {
            schema_version: 3,
            kind: "clonk_software_present_smoke",
            success,
            failure: self.failure.clone(),
            initial_extent: self.initial_extent,
            resized_extent: self.resized_extent,
            presented_before_resize: self.presented_before_resize,
            presented_after_resize: self.presented_after_resize,
            phases: self.phases.clone(),
            registry_empty_at_exit: registry_empty,
            input_mapping: self.input_mapping.clone(),
            software_reason: self.software_reason,
            gpu_attempt_backends: self.gpu_attempt_backends.clone(),
            display_backend: self.display_backend,
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
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

fn set_input_scale(
    windows: &mut DeveloperWindows<DeveloperHost>,
    app: &mut crate::GameApp,
    scale: f32,
) -> Result<()> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software probe lost its shell before scaling input")?;
    shell.presenter.set_scale(scale);
    let (width, height) = shell.presenter.logical_size();
    app.resize(width, height)?;
    Ok(())
}

fn save_captures(
    report_path: &Path,
    windows: &mut DeveloperWindows<DeveloperHost>,
    app: &mut crate::GameApp,
) -> Result<()> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software probe lost its shell before capture")?;
    let presenter = shell
        .software
        .as_mut()
        .context("the software probe lost its presenter before capture")?;
    let (width, height) = presenter.frame_extent();
    // Exercise the ordinary F9 request and save path against the frame that
    // reached the software presenter, including its screenshot directory.
    app.handle_key(
        crate::VirtualKeyCode::F9,
        winit::event::ElementState::Pressed,
    )?;
    app.handle_key(
        crate::VirtualKeyCode::F9,
        winit::event::ElementState::Released,
    )?;
    let screenshot = app
        .save_next_screenshot(
            Some(presenter.frame_mut()),
            width,
            height,
            shell.presenter.scale(),
        )
        .context("the software probe's F9 request produced no screenshot")?;
    screenshot
        .result
        .context("the software probe's screenshot failed")?;
    std::fs::copy(
        &screenshot.path,
        report_path.with_extension("screenshot.png"),
    )?;
    let thumbnail = crate::main_resources::encode_presented_save_thumbnail(
        width,
        height,
        presenter.frame_mut(),
    )?;
    std::fs::write(report_path.with_extension("thumbnail.png"), thumbnail)?;
    Ok(())
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

/// Ask the window system to maximize the shell; its ordinary resize event must
/// resize the presenter.
///
/// A maximize rather than a requested size: on Wayland winit applies a
/// client's own `request_inner_size` at once and sends no resize event
/// (winit 0.30 `platform_impl/linux/wayland/window/state.rs`), so the
/// production handler would never run and the phase could not complete.
fn maximize_shell(windows: &mut DeveloperWindows<DeveloperHost>) -> Result<()> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software presentation probe's shell disappeared before resize")?;
    shell.window.set_maximized(true);
    Ok(())
}

/// The shell's new extent once the drawable, the frame and the input
/// presenter have all followed its resize.
fn shell_resize_followed(
    windows: &mut DeveloperWindows<DeveloperHost>,
    initial: [u32; 2],
) -> Result<Option<[u32; 2]>> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the software presentation probe's shell disappeared during resize")?;
    let presenter = shell
        .software
        .as_ref()
        .context("the software presentation probe's presenter disappeared during resize")?;
    let size = shell.window.inner_size();
    let window = [size.width, size.height];
    let followers = [
        presenter.drawable_extent(),
        presenter.frame_extent(),
        shell.presenter.physical_size(),
    ];
    Ok(crate::headed_surface_smoke::resize_followed(initial, window, &followers).then_some(window))
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5")
))]
mod tests {
    #[test]
    fn the_probe_accepts_automatic_fallback_with_no_enabled_gpu_backend() {
        let directory = tempfile::tempdir().unwrap();
        std::env::remove_var(crate::main_audio::SOFTWARE_PRESENTATION_ENV);
        std::env::set_var("WGPU_BACKEND", "");
        super::prepare(&directory.path().join("report.json")).unwrap();
    }
}
