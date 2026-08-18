//! Real-window lifecycle probe for headed GPU/driver validation.
//!
//! Unit tests can prove the registry rules, but cannot construct winit's
//! `ActiveEventLoop`. This probe stays inside the shipped event handler: it
//! opens a second real window through the ordinary framebuffer builder,
//! presents both, destroys the child, presents the survivor, requests exit,
//! and writes its report only after the production `LoopExiting` teardown.

use crate::developer_host::DeveloperHost;
use crate::developer_windows::{
    CloseOutcome, DeveloperWindows, HostPurpose, WindowId, SHELL_WINDOW,
};
use crate::main_audio::{present_pixels_frame, RetainedGpuPresentOutcome};
use anyhow::{anyhow, ensure, Context, Result};
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::Window;

const SMOKE_TIMEOUT: Duration = Duration::from_secs(15);
const SMOKE_RETRY_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn prepare(report_path: &Path) -> Result<()> {
    ensure!(
        !report_path
            .try_exists()
            .with_context(|| format!("could not inspect {}", report_path.display()))?,
        "headed surface smoke report already exists: {}",
        report_path.display()
    );
    let requested = wgpu::Backends::from_env().context(
        "headed surface smoke requires one explicit WGPU_BACKEND (vulkan on the reporting hardware)",
    )?;
    ensure!(
        requested.bits().count_ones() == 1,
        "headed surface smoke requires exactly one WGPU_BACKEND, got {requested:?}"
    );
    crate::gpu_instance::begin_retained_instance_evidence_capture();
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct AdapterEvidence {
    name: String,
    vendor_id: u32,
    device_id: u32,
    device_type: &'static str,
    pci_bus_id: String,
    driver: String,
    driver_info: String,
    backend: &'static str,
    subgroup_min_size: u32,
    subgroup_max_size: u32,
    /// `None` where the adapter does not report it — the GL backend never does.
    transient_saves_memory: Option<bool>,
}

impl AdapterEvidence {
    fn from_info(info: &wgpu::AdapterInfo) -> Self {
        let wgpu::AdapterInfo {
            name,
            vendor,
            device,
            device_type,
            device_pci_bus_id,
            driver,
            driver_info,
            backend,
            subgroup_min_size,
            subgroup_max_size,
            transient_saves_memory,
            // Bucketing is never requested, so this is always `None` and is no
            // evidence about the adapter.
            limit_bucket: _,
        } = info.clone();
        let device_type = match device_type {
            wgpu::DeviceType::Other => "other",
            wgpu::DeviceType::IntegratedGpu => "integrated-gpu",
            wgpu::DeviceType::DiscreteGpu => "discrete-gpu",
            wgpu::DeviceType::VirtualGpu => "virtual-gpu",
            wgpu::DeviceType::Cpu => "cpu",
        };
        Self {
            name,
            vendor_id: vendor,
            device_id: device,
            device_type,
            pci_bus_id: device_pci_bus_id,
            driver,
            driver_info,
            backend: backend.to_str(),
            subgroup_min_size,
            subgroup_max_size,
            transient_saves_memory,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct InstanceAcquisitionReport {
    sequence: u64,
    entry_id: u64,
    requested_backends: Vec<&'static str>,
    created: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RetainedInstanceReport {
    entry_id: u64,
    requested_backends: Vec<&'static str>,
    acquisitions: u64,
    resident_at_loop_exit: bool,
}

#[derive(Debug, Serialize)]
struct HeadedSurfaceSmokeReport {
    schema_version: u32,
    kind: &'static str,
    success: bool,
    failure: Option<String>,
    display_backend: &'static str,
    wayland_display: Option<String>,
    xdg_session_type: Option<String>,
    surface_windows: Vec<SurfaceWindowReport>,
    instance_acquisitions: Vec<InstanceAcquisitionReport>,
    retained_registry: Vec<RetainedInstanceReport>,
    shell_adapter: AdapterEvidence,
    child_adapter: AdapterEvidence,
    shell_presented_before_close: bool,
    child_presented_before_close: bool,
    child_closed_while_shell_survived: bool,
    child_released_after_close: bool,
    shell_presented_after_child_close: bool,
    loop_exiting_release_order: Vec<u64>,
    registry_empty_on_loop_exiting: bool,
    shell_released_on_loop_exiting: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SurfaceWindowReport {
    role: &'static str,
    window_id: String,
    instance_entry_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmokePhase {
    PresentBoth,
    PresentSurvivor,
    AwaitLoopExit,
    Failed,
}

pub(crate) struct HeadedSurfaceSmoke {
    report_path: PathBuf,
    child_key: WindowId,
    shell_window: Weak<Window>,
    child_window: Weak<Window>,
    display_backend: &'static str,
    shell_os_window_id: winit::window::WindowId,
    child_os_window_id: winit::window::WindowId,
    shell_window_id: String,
    child_window_id: String,
    shell_adapter: AdapterEvidence,
    child_adapter: AdapterEvidence,
    instance_entry_id: u64,
    acquisitions: Vec<InstanceAcquisitionReport>,
    deadline: Instant,
    next_retry: Instant,
    phase: SmokePhase,
    shell_presented_before_close: bool,
    child_presented_before_close: bool,
    child_closed_while_shell_survived: bool,
    child_released_after_close: bool,
    shell_presented_after_child_close: bool,
    failure: Option<String>,
}

impl HeadedSurfaceSmoke {
    pub(crate) fn start(
        report_path: PathBuf,
        event_loop: &ActiveEventLoop,
        windows: &mut DeveloperWindows<DeveloperHost>,
        next_window_key: &mut u64,
    ) -> Result<Self> {
        let (shell_window, shell_os_window_id, shell_window_id, display_backend, shell_adapter) = {
            let shell = windows
                .shell_mut()
                .and_then(DeveloperHost::as_shell_mut)
                .context("the headed surface probe needs the live shell")?;
            let pixels = shell
                .pixels
                .as_ref()
                .context("the headed surface probe needs the shell framebuffer")?;
            shell.window.set_visible(true);
            shell.window.focus_window();
            (
                Arc::downgrade(&shell.window),
                shell.window.id(),
                format!("{:?}", shell.window.id()),
                display_backend(&shell.window)?,
                AdapterEvidence::from_info(&pixels.device().adapter_info()),
            )
        };
        let after_shell = crate::gpu_instance::retained_instance_registry_evidence();
        let shell_acquisition = exact_shell_acquisition(&after_shell)?;

        let child = crate::viewport_window_host::build_viewport_window(
            event_loop,
            "Clonk headed GPU teardown probe",
            96,
            96,
            u64::MAX,
            1.0,
        )
        .context("failed to build the headed surface probe's viewport window")?;
        child.window.set_visible(true);
        child.window.focus_window();
        let child_window = Arc::downgrade(&child.window);
        let child_os_window_id = child.window.id();
        let child_window_id = format!("{:?}", child.window.id());
        ensure!(
            child_window_id != shell_window_id,
            "the headed surface probe did not create a distinct child window"
        );
        let child_adapter = AdapterEvidence::from_info(
            &child
                .pixels
                .as_ref()
                .context("the headed surface probe needs the child framebuffer")?
                .device()
                .adapter_info(),
        );
        let after_child = crate::gpu_instance::retained_instance_registry_evidence();
        let acquisitions = exact_child_acquisition(&after_child, shell_acquisition)?;
        if child_adapter != shell_adapter {
            return Err(anyhow!(
                "the shell and child selected different adapters: {shell_adapter:?} != {child_adapter:?}"
            ));
        }

        let child_key = WindowId(*next_window_key);
        *next_window_key = next_window_key.saturating_add(1);
        windows.insert(
            child_key,
            HostPurpose::Viewport { viewport: u32::MAX },
            DeveloperHost::Viewport(child),
        );
        windows.request_redraw_visible();

        Ok(Self {
            report_path,
            child_key,
            shell_window,
            child_window,
            display_backend,
            shell_os_window_id,
            child_os_window_id,
            shell_window_id,
            child_window_id,
            shell_adapter,
            child_adapter,
            instance_entry_id: shell_acquisition.entry_id,
            acquisitions,
            deadline: Instant::now() + SMOKE_TIMEOUT,
            next_retry: Instant::now(),
            phase: SmokePhase::PresentBoth,
            shell_presented_before_close: false,
            child_presented_before_close: false,
            child_closed_while_shell_survived: false,
            child_released_after_close: false,
            shell_presented_after_child_close: false,
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
                "the headed surface probe timed out in phase {:?}",
                self.phase
            ));
        }
        if now >= self.next_retry {
            windows.request_redraw_visible();
            self.next_retry = now + SMOKE_RETRY_INTERVAL;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_retry.min(self.deadline)));
        if self.phase == SmokePhase::AwaitLoopExit {
            event_loop.exit();
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
        if Instant::now() >= self.deadline {
            return Err(anyhow!(
                "the headed surface probe timed out in phase {:?}",
                self.phase
            ));
        }
        match self.phase {
            SmokePhase::PresentBoth => {
                if os_window_id == self.child_os_window_id {
                    self.child_presented_before_close |= present_child(windows, self.child_key)?;
                } else if os_window_id == self.shell_os_window_id {
                    self.shell_presented_before_close |= present_shell(windows)?;
                }
                if self.child_presented_before_close && self.shell_presented_before_close {
                    if windows.close(self.child_key) != CloseOutcome::Destroyed {
                        return Err(anyhow!(
                            "the headed surface probe's child did not close as a real child window"
                        ));
                    }
                    self.child_closed_while_shell_survived = self.shell_window.upgrade().is_some();
                    self.child_released_after_close = self.child_window.upgrade().is_none();
                    if !self.child_closed_while_shell_survived || !self.child_released_after_close {
                        return Err(anyhow!(
                            "closing the child did not release only that window and surface"
                        ));
                    }
                    self.phase = SmokePhase::PresentSurvivor;
                    windows.request_redraw(SHELL_WINDOW);
                }
            }
            SmokePhase::PresentSurvivor => {
                if os_window_id == self.shell_os_window_id {
                    self.shell_presented_after_child_close |= present_shell(windows)?;
                    if self.shell_presented_after_child_close {
                        self.phase = SmokePhase::AwaitLoopExit;
                        event_loop.exit();
                    }
                }
            }
            SmokePhase::AwaitLoopExit => event_loop.exit(),
            SmokePhase::Failed => unreachable!("failed probes exit above"),
        }
        Ok(())
    }

    pub(crate) fn fail(&mut self, error: impl std::fmt::Display) {
        self.failure.get_or_insert_with(|| error.to_string());
        self.phase = SmokePhase::Failed;
    }

    pub(crate) fn finish(&mut self, released: &[WindowId], registry_empty: bool) -> Result<()> {
        let release_order = released.iter().map(|id| id.0).collect::<Vec<_>>();
        let shell_released = self.shell_window.upgrade().is_none();
        if self.phase != SmokePhase::AwaitLoopExit {
            self.failure.get_or_insert_with(|| {
                format!(
                    "the event loop exited while the probe was in phase {:?}",
                    self.phase
                )
            });
        }
        if released != [SHELL_WINDOW] {
            self.failure.get_or_insert_with(|| {
                format!("LoopExiting released {released:?}; expected only the surviving shell")
            });
        }
        if !registry_empty || !shell_released {
            self.failure.get_or_insert_with(|| {
                "LoopExiting returned before every remaining window and surface was released"
                    .to_owned()
            });
        }

        let registry = crate::gpu_instance::retained_instance_registry_evidence();
        if registry.acquisitions.len() != self.acquisitions.len()
            || registry.entries.len() != 1
            || registry.entries[0].entry_id != self.instance_entry_id
            || registry.entries[0].acquisitions != 2
        {
            self.failure.get_or_insert_with(|| {
                format!("the retained instance registry drifted before LoopExiting: {registry:?}")
            });
        }
        let retained_registry = registry
            .entries
            .iter()
            .map(|entry| RetainedInstanceReport {
                entry_id: entry.entry_id,
                requested_backends: backend_names(entry.backends),
                acquisitions: entry.acquisitions,
                resident_at_loop_exit: true,
            })
            .collect();
        let report = HeadedSurfaceSmokeReport {
            schema_version: 1,
            kind: "clonk_headed_surface_smoke",
            success: self.failure.is_none(),
            failure: self.failure.clone(),
            display_backend: self.display_backend,
            wayland_display: std::env::var("WAYLAND_DISPLAY").ok(),
            xdg_session_type: std::env::var("XDG_SESSION_TYPE").ok(),
            surface_windows: vec![
                SurfaceWindowReport {
                    role: "shell",
                    window_id: self.shell_window_id.clone(),
                    instance_entry_id: self.instance_entry_id,
                },
                SurfaceWindowReport {
                    role: "viewport",
                    window_id: self.child_window_id.clone(),
                    instance_entry_id: self.instance_entry_id,
                },
            ],
            instance_acquisitions: self.acquisitions.clone(),
            retained_registry,
            shell_adapter: self.shell_adapter.clone(),
            child_adapter: self.child_adapter.clone(),
            shell_presented_before_close: self.shell_presented_before_close,
            child_presented_before_close: self.child_presented_before_close,
            child_closed_while_shell_survived: self.child_closed_while_shell_survived,
            child_released_after_close: self.child_released_after_close,
            shell_presented_after_child_close: self.shell_presented_after_child_close,
            loop_exiting_release_order: release_order,
            registry_empty_on_loop_exiting: registry_empty,
            shell_released_on_loop_exiting: shell_released,
        };
        let bytes = serde_json::to_vec_pretty(&report)
            .context("failed to encode the headed surface smoke report")?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.report_path)
            .with_context(|| {
                format!(
                    "headed surface smoke report path stopped being fresh: {}",
                    self.report_path.display()
                )
            })?;
        file.write_all(&bytes).with_context(|| {
            format!(
                "failed to write headed surface smoke report {}",
                self.report_path.display()
            )
        })?;
        file.sync_all().with_context(|| {
            format!(
                "failed to flush headed surface smoke report {}",
                self.report_path.display()
            )
        })?;
        report.success.then_some(()).ok_or_else(|| {
            anyhow!(
                "headed surface smoke failed: {}",
                report.failure.as_deref().unwrap_or("unknown failure")
            )
        })
    }
}

fn present_child(
    windows: &mut DeveloperWindows<DeveloperHost>,
    child_key: WindowId,
) -> Result<bool> {
    let pixels = match windows.host_mut(child_key) {
        Some(DeveloperHost::Viewport(child)) => child.pixels.as_mut(),
        Some(
            DeveloperHost::Shell(_)
            | DeveloperHost::Toolbox(_)
            | DeveloperHost::ObjectList(_)
            | DeveloperHost::ComponentEditor(_),
        )
        | None => None,
    }
    .context("the headed surface probe's viewport framebuffer disappeared before close")?;
    paint_probe_frame(pixels, [0x3d, 0xb7, 0x83, 0xff]);
    present_pixels_frame(pixels)
        .map(|outcome| outcome == RetainedGpuPresentOutcome::Presented)
        .context("failed to present the headed surface probe's child window")
}

fn present_shell(windows: &mut DeveloperWindows<DeveloperHost>) -> Result<bool> {
    let shell = windows
        .shell_mut()
        .and_then(DeveloperHost::as_shell_mut)
        .context("the headed surface probe's shell disappeared before LoopExiting")?;
    let pixels = shell
        .pixels
        .as_mut()
        .context("the headed surface probe's shell framebuffer disappeared before LoopExiting")?;
    paint_probe_frame(pixels, [0x36, 0x74, 0xa7, 0xff]);
    present_pixels_frame(pixels)
        .map(|outcome| outcome == RetainedGpuPresentOutcome::Presented)
        .context("failed to present the headed surface probe's shell window")
}

fn paint_probe_frame(surface: &mut clonk_surface::WindowSurface, color: [u8; 4]) {
    for pixel in surface.frame_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&color);
    }
}

fn exact_shell_acquisition(
    registry: &crate::gpu_instance::InstanceRegistryEvidence,
) -> Result<crate::gpu_instance::InstanceAcquisitionEvidence> {
    ensure!(
        registry.entries.len() == 1 && registry.acquisitions.len() == 1,
        "the shell must make exactly one retained-instance acquisition under a forced backend: {registry:?}"
    );
    let acquisition = registry.acquisitions[0];
    ensure!(
        acquisition.sequence == 1
            && acquisition.created
            && acquisition.entry_id == registry.entries[0].entry_id
            && acquisition.backends.bits().count_ones() == 1
            && registry.entries[0].acquisitions == 1,
        "the shell did not create exactly one linked retained instance: {registry:?}"
    );
    Ok(acquisition)
}

fn exact_child_acquisition(
    registry: &crate::gpu_instance::InstanceRegistryEvidence,
    shell: crate::gpu_instance::InstanceAcquisitionEvidence,
) -> Result<Vec<InstanceAcquisitionReport>> {
    ensure!(
        registry.entries.len() == 1 && registry.acquisitions.len() == 2,
        "the viewport must add exactly one retained-instance acquisition: {registry:?}"
    );
    let child = registry.acquisitions[1];
    ensure!(
        child.sequence == 2
            && !child.created
            && child.entry_id == shell.entry_id
            && child.backends == shell.backends
            && registry.entries[0].entry_id == shell.entry_id
            && registry.entries[0].backends == shell.backends
            && registry.entries[0].acquisitions == 2,
        "the viewport did not reuse the shell's retained instance: {registry:?}"
    );
    Ok(registry
        .acquisitions
        .iter()
        .map(|acquisition| InstanceAcquisitionReport {
            sequence: acquisition.sequence,
            entry_id: acquisition.entry_id,
            requested_backends: backend_names(acquisition.backends),
            created: acquisition.created,
        })
        .collect())
}

fn backend_names(backends: wgpu::Backends) -> Vec<&'static str> {
    wgpu::Backend::ALL
        .into_iter()
        .filter(|backend| backends.contains((*backend).into()))
        .map(wgpu::Backend::to_str)
        .collect()
}

fn display_backend(window: &Window) -> Result<&'static str> {
    let display = window
        .display_handle()
        .context("the headed surface probe could not read the shell display handle")?;
    Ok(match display.as_raw() {
        RawDisplayHandle::Wayland(_) => "wayland",
        RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => "x11",
        RawDisplayHandle::AppKit(_) => "appkit",
        RawDisplayHandle::Windows(_) => "windows",
        RawDisplayHandle::UiKit(_) => "uikit",
        RawDisplayHandle::Orbital(_) => "orbital",
        RawDisplayHandle::Ohos(_) => "ohos",
        RawDisplayHandle::Drm(_) => "drm",
        RawDisplayHandle::Gbm(_) => "gbm",
        RawDisplayHandle::Web(_) => "web",
        RawDisplayHandle::Android(_) => "android",
        RawDisplayHandle::Haiku(_) => "haiku",
        _ => "unknown",
    })
}
