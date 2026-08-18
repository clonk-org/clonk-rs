//! The window surface itself: a drawable, a CPU frame buffer, and the blit
//! that puts one onto the other.

use crate::acquire::{acquire_drawable, AcquireError, Acquisition};
use crate::blit::{BlitTransform, Blitter};
use std::time::{Duration, Instant};

/// The CPU frame buffer's format. Four bytes per pixel, always: the buffer is
/// filled by the software rasterizer, which has no other layout.
const BUFFER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const BUFFER_BYTES_PER_PIXEL: u32 = 4;

/// Create the process's `wgpu::Instance` for a backend set.
///
/// Exposed separately because an instance must outlive every window that
/// borrows it — see `clonk-app`'s instance registry for why destroying one
/// while another window still presents takes the process down.
pub fn create_instance(backends: wgpu::Backends) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle().with_env()
    })
}

/// Everything that can go wrong presenting to a window.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SurfaceError {
    /// No usable GPU adapter for the requested backends.
    #[error("no usable GPU adapter was found")]
    AdapterNotFound,
    /// The device could not be created.
    #[error("failed to create the GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    /// The window handle could not be turned into a surface.
    #[error("failed to create a surface for the window: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    /// An extent was zero or beyond what the device supports.
    #[error("{0}")]
    Extent(#[from] ExtentError),
    /// The adapter does not meet the interactive graphics floor. Carries every
    /// unmet requirement, not just the first one found.
    #[error("{0}")]
    BelowGraphicsFloor(crate::capability::CapabilityReport),
    /// The surface is gone and must be rebuilt by its owner, not reconfigured.
    /// This is the one presentation failure callers can recover from.
    #[error("the window surface was lost")]
    SurfaceLost,
    /// The driver rejected the surface.
    #[error("the window surface failed validation")]
    Validation,
    /// The render callback failed.
    #[error("the render callback failed: {0}")]
    Callback(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl From<AcquireError> for SurfaceError {
    fn from(error: AcquireError) -> Self {
        match error {
            AcquireError::SurfaceLost => Self::SurfaceLost,
            AcquireError::Validation => Self::Validation,
        }
    }
}

/// A requested extent the device cannot back.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ExtentError {
    /// Width is zero or above `max_texture_dimension_2d`.
    #[error("texture width is invalid: {0}")]
    Width(u32),
    /// Height is zero or above `max_texture_dimension_2d`.
    #[error("texture height is invalid: {0}")]
    Height(u32),
}

/// Reject an extent the device cannot back, before wgpu turns it into a
/// validation error with no way to recover.
const fn check_extent(width: u32, height: u32, max: u32) -> Result<(), ExtentError> {
    if width == 0 || width > max {
        return Err(ExtentError::Width(width));
    }
    if height == 0 || height > max {
        return Err(ExtentError::Height(height));
    }
    Ok(())
}

/// Whether a redraw actually reached the compositor.
///
/// An occluded, timed-out or persistently outdated surface yields no drawable,
/// and the render callback never runs — so `Ok` alone does not mean a frame was
/// presented, and a caller that treats it that way reports frames it never drew.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presentation {
    Presented,
    Skipped,
}

/// Host-side wall-clock intervals around the surface API calls.
///
/// Submission and presentation are CPU call durations, not GPU completion.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowSurfaceCpuStages {
    pub drawable_acquisition: Duration,
    /// CPU time spent finalizing the command encoder before submission.
    pub command_encoder_finalization: Duration,
    pub queue_submission: Duration,
    pub presentation: Duration,
}

impl WindowSurfaceCpuStages {
    pub fn total(self) -> Duration {
        self.drawable_acquisition
            .saturating_add(self.command_encoder_finalization)
            .saturating_add(self.queue_submission)
            .saturating_add(self.presentation)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfiledPresentation {
    pub presentation: Presentation,
    pub cpu_stages: WindowSurfaceCpuStages,
}

/// What the render callback is handed alongside the encoder and drawable view.
pub struct FrameContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowSurfaceBuildOptions {
    pub timestamp_queries: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimestampQueryStatus {
    pub requested: bool,
    pub supported: bool,
    pub enabled: bool,
}

impl TimestampQueryStatus {
    pub const fn required_features(self) -> wgpu::Features {
        if self.enabled {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        }
    }
}

fn timestamp_query_status(
    requested: bool,
    adapter_features: wgpu::Features,
) -> TimestampQueryStatus {
    let supported = adapter_features.contains(wgpu::Features::TIMESTAMP_QUERY);
    TimestampQueryStatus {
        requested,
        supported,
        enabled: requested && supported,
    }
}

/// A window's drawable, its CPU frame buffer, and the blit between them.
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_dimension_2d: u32,
    adapter_features: wgpu::Features,
    timestamp_query_status: TimestampQueryStatus,

    surface_format: wgpu::TextureFormat,
    surface_extent: (u32, u32),
    present_mode: wgpu::PresentMode,
    alpha_mode: wgpu::CompositeAlphaMode,

    buffer: Vec<u8>,
    buffer_extent: (u32, u32),
    texture: wgpu::Texture,
    blitter: Blitter,
}

impl WindowSurface {
    /// Build a surface for `window` against a caller-owned instance.
    ///
    /// The instance is borrowed rather than created here because it must
    /// outlive this surface and every other window's.
    pub fn build<W>(
        instance: &wgpu::Instance,
        window: W,
        buffer_extent: (u32, u32),
        surface_extent: (u32, u32),
        present_mode: wgpu::PresentMode,
    ) -> Result<Self, SurfaceError>
    where
        W: wgpu::WindowHandle + raw_window_handle::HasDisplayHandle + 'static,
    {
        Self::build_with_options(
            instance,
            window,
            buffer_extent,
            surface_extent,
            present_mode,
            WindowSurfaceBuildOptions::default(),
        )
    }

    pub fn build_with_options<W>(
        instance: &wgpu::Instance,
        window: W,
        buffer_extent: (u32, u32),
        surface_extent: (u32, u32),
        present_mode: wgpu::PresentMode,
        options: WindowSurfaceBuildOptions,
    ) -> Result<Self, SurfaceError>
    where
        W: wgpu::WindowHandle + raw_window_handle::HasDisplayHandle + 'static,
    {
        pollster::block_on(Self::build_async(
            instance,
            window,
            buffer_extent,
            surface_extent,
            present_mode,
            options,
        ))
    }

    async fn build_async<W>(
        instance: &wgpu::Instance,
        window: W,
        buffer_extent: (u32, u32),
        surface_extent: (u32, u32),
        present_mode: wgpu::PresentMode,
        options: WindowSurfaceBuildOptions,
    ) -> Result<Self, SurfaceError>
    where
        W: wgpu::WindowHandle + raw_window_handle::HasDisplayHandle + 'static,
    {
        let surface = instance.create_surface(window)?;
        let adapter = adapter_for(instance, &surface).await?;
        let adapter_features = adapter.features();
        let timestamp_query_status =
            timestamp_query_status(options.timestamp_queries, adapter_features);

        // Ask for everything the adapter offers: the renderer reads
        // `max_texture_dimension_2d` back off the device to decide whether a
        // presentation extent fits, and a defaulted limit would cap it far
        // below what the hardware can do.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: timestamp_query_status.required_features(),
                required_limits: adapter.limits(),
                ..wgpu::DeviceDescriptor::default()
            })
            .await?;

        let capabilities = surface.get_capabilities(&adapter);
        let present_mode = match present_mode {
            wgpu::PresentMode::AutoVsync | wgpu::PresentMode::AutoNoVsync => present_mode,
            requested if capabilities.present_modes.contains(&requested) => requested,
            _ => wgpu::PresentMode::AutoVsync,
        };

        // Check the whole graphics floor before deriving anything from it, so a
        // machine below it gets one diagnostic naming every missing
        // requirement rather than discovering them one failure at a time.
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let report = crate::capability::probe_capabilities(
            &capabilities.formats,
            max_texture_dimension_2d,
            buffer_extent,
        );
        if !report.is_supported() {
            return Err(SurfaceError::BelowGraphicsFloor(report));
        }

        // Presentation composites in byte space and relies on the surface
        // encode to restore those bytes, so an sRGB surface is required. The
        // probe above has already established there is one.
        let surface_format = capabilities
            .formats
            .iter()
            .find(|format| format.is_srgb())
            .copied()
            .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);

        let texture = create_buffer_texture(&device, buffer_extent);
        let blitter = Blitter::new(
            &device,
            &texture,
            surface_format,
            BlitTransform::pixel_perfect(buffer_extent, surface_extent),
        );

        let surface = Self {
            surface,
            device,
            queue,
            max_texture_dimension_2d,
            adapter_features,
            timestamp_query_status,
            surface_format,
            surface_extent,
            present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            buffer: vec![0; buffer_len(buffer_extent)],
            buffer_extent,
            texture,
            blitter,
        };
        surface.configure();
        Ok(surface)
    }

    fn configure(&self) {
        self.surface.configure(
            &self.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.surface_format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width: self.surface_extent.0,
                height: self.surface_extent.1,
                present_mode: self.present_mode,
                desired_maximum_frame_latency: 2,
                alpha_mode: self.alpha_mode,
                view_formats: vec![],
            },
        );
    }

    /// The CPU frame buffer. Not cleared between frames.
    pub fn frame(&self) -> &[u8] {
        &self.buffer
    }

    /// The CPU frame buffer, to rasterize into.
    pub fn frame_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    /// The frame buffer's extent, which is not always the drawable's.
    pub const fn buffer_extent(&self) -> (u32, u32) {
        self.buffer_extent
    }

    /// The compositor drawable's configured extent.
    pub const fn surface_extent(&self) -> (u32, u32) {
        self.surface_extent
    }

    pub const fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub const fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub const fn timestamp_query_status(&self) -> TimestampQueryStatus {
        self.timestamp_query_status
    }

    /// Optional features advertised by the exact adapter selected for this surface.
    pub const fn adapter_features(&self) -> wgpu::Features {
        self.adapter_features
    }

    /// The drawable's format, which the scene renderer must target.
    pub const fn surface_texture_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    /// The presentation mode selected from the surface's advertised modes.
    pub const fn present_mode(&self) -> wgpu::PresentMode {
        self.present_mode
    }

    pub const fn alpha_mode(&self) -> wgpu::CompositeAlphaMode {
        self.alpha_mode
    }

    /// The largest 2D texture this device can back.
    pub const fn max_texture_dimension_2d(&self) -> u32 {
        self.max_texture_dimension_2d
    }

    /// Resize the CPU frame buffer, rebuilding its texture.
    pub fn resize_buffer(&mut self, width: u32, height: u32) -> Result<(), ExtentError> {
        check_extent(width, height, self.max_texture_dimension_2d)?;
        self.buffer_extent = (width, height);
        self.buffer = vec![0; buffer_len(self.buffer_extent)];
        self.texture = create_buffer_texture(&self.device, self.buffer_extent);
        self.blitter.rebind(&self.device, &self.texture);
        self.retransform();
        Ok(())
    }

    /// Resize the drawable and reconfigure the surface.
    pub fn resize_surface(&mut self, width: u32, height: u32) -> Result<(), ExtentError> {
        check_extent(width, height, self.max_texture_dimension_2d)?;
        self.surface_extent = (width, height);
        self.retransform();
        self.configure();
        Ok(())
    }

    fn retransform(&mut self) {
        self.blitter.set_transform(
            &self.queue,
            BlitTransform::pixel_perfect(self.buffer_extent, self.surface_extent),
        );
    }

    /// Upload the CPU frame buffer and blit it to the drawable.
    pub fn present_frame(&self) -> Result<Presentation, SurfaceError> {
        self.render_with(|encoder, view, _| {
            self.blitter.blit(encoder, view);
            Ok(())
        })
    }

    /// Acquire a drawable, upload the CPU frame buffer, and hand the encoder to
    /// `render`.
    ///
    /// `render` does not run when no drawable was available; the returned
    /// [`Presentation`] says which happened.
    pub fn render_with<F>(&self, render: F) -> Result<Presentation, SurfaceError>
    where
        F: FnOnce(
            &mut wgpu::CommandEncoder,
            &wgpu::TextureView,
            &FrameContext<'_>,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        self.render_with_profiled(render)
            .map(|profiled| profiled.presentation)
    }

    pub fn render_with_profiled<F>(&self, render: F) -> Result<ProfiledPresentation, SurfaceError>
    where
        F: FnOnce(
            &mut wgpu::CommandEncoder,
            &wgpu::TextureView,
            &FrameContext<'_>,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        let acquisition_started = Instant::now();
        let Some(frame) = acquire_drawable(
            || match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => Acquisition::Success(frame),
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Acquisition::Suboptimal(frame),
                wgpu::CurrentSurfaceTexture::Outdated => Acquisition::Outdated,
                wgpu::CurrentSurfaceTexture::Lost => Acquisition::Lost,
                wgpu::CurrentSurfaceTexture::Occluded => Acquisition::Occluded,
                wgpu::CurrentSurfaceTexture::Timeout => Acquisition::Timeout,
                wgpu::CurrentSurfaceTexture::Validation => Acquisition::Validation,
            },
            || self.configure(),
        )?
        else {
            return Ok(ProfiledPresentation {
                presentation: Presentation::Skipped,
                cpu_stages: WindowSurfaceCpuStages {
                    drawable_acquisition: acquisition_started.elapsed(),
                    ..WindowSurfaceCpuStages::default()
                },
            });
        };
        let drawable_acquisition = acquisition_started.elapsed();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clonk_surface_encoder"),
            });
        self.upload_buffer();
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        render(&mut encoder, &view, &self.frame_context()).map_err(SurfaceError::Callback)?;

        let finalization_started = Instant::now();
        let command_buffer = encoder.finish();
        let submission_started = Instant::now();
        let command_encoder_finalization = submission_started.duration_since(finalization_started);
        self.queue.submit(Some(command_buffer));
        let presentation_started = Instant::now();
        let queue_submission = presentation_started.duration_since(submission_started);
        self.queue.present(frame);
        let presented_at = Instant::now();
        let presentation = presented_at.duration_since(presentation_started);
        Ok(ProfiledPresentation {
            presentation: Presentation::Presented,
            cpu_stages: WindowSurfaceCpuStages {
                drawable_acquisition,
                command_encoder_finalization,
                queue_submission,
                presentation,
            },
        })
    }

    const fn frame_context(&self) -> FrameContext<'_> {
        FrameContext {
            device: &self.device,
            queue: &self.queue,
        }
    }

    fn upload_buffer(&self) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.buffer,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.buffer_extent.0 * BUFFER_BYTES_PER_PIXEL),
                rows_per_image: Some(self.buffer_extent.1),
            },
            extent_3d(self.buffer_extent),
        );
    }
}

/// The index of the first adapter whose name satisfies a `WGPU_ADAPTER_NAME`
/// request, matched case-insensitively as a substring.
fn adapter_matching_name(adapter_names: &[impl AsRef<str>], desired: &str) -> Option<usize> {
    let desired = desired.to_lowercase();
    adapter_names
        .iter()
        .position(|name| name.as_ref().to_lowercase().contains(&desired))
}

/// Pick the adapter to present through.
///
/// `WGPU_ADAPTER_NAME` selects one by name when it is set. wgpu's own
/// `initialize_adapter_from_env` panics when that name matches nothing
/// (wgpu-29.0.4 src/util/init.rs:44); a panic here would escape the caller's
/// backend-widening loop and unwind through a winit callback, so an unmatched
/// name is logged and falls through to the ordinary request instead.
async fn adapter_for(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, SurfaceError> {
    if let Some(desired) = std::env::var("WGPU_ADAPTER_NAME")
        .ok()
        .filter(|name| !name.is_empty())
    {
        let mut compatible = instance
            .enumerate_adapters(wgpu::Backends::all())
            .await
            .into_iter()
            .filter(|adapter| adapter.is_surface_supported(surface))
            .collect::<Vec<_>>();
        let names = compatible
            .iter()
            .map(|adapter| adapter.get_info().name)
            .collect::<Vec<_>>();
        match adapter_matching_name(&names, &desired) {
            Some(index) => return Ok(compatible.swap_remove(index)),
            None => tracing::warn!(
                %desired,
                available = ?names,
                "WGPU_ADAPTER_NAME matched no adapter for this surface; requesting the default"
            ),
        }
    }

    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(surface),
            force_fallback_adapter: false,
            power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(),
            apply_limit_buckets: false,
        })
        .await
        .map_err(|_| SurfaceError::AdapterNotFound)
}

const fn buffer_len(extent: (u32, u32)) -> usize {
    (extent.0 * extent.1 * BUFFER_BYTES_PER_PIXEL) as usize
}

const fn extent_3d(extent: (u32, u32)) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: extent.0,
        height: extent.1,
        depth_or_array_layers: 1,
    }
}

fn create_buffer_texture(device: &wgpu::Device, extent: (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("clonk_surface_frame_buffer"),
        size: extent_3d(extent),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: BUFFER_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_feature_is_not_requested_when_disabled() {
        let status = timestamp_query_status(false, wgpu::Features::TIMESTAMP_QUERY);

        assert!(!status.requested);
        assert!(status.supported);
        assert!(!status.enabled);
        assert_eq!(status.required_features(), wgpu::Features::empty());
    }

    #[test]
    fn unsupported_timestamp_request_keeps_features_empty() {
        let status = timestamp_query_status(true, wgpu::Features::empty());

        assert!(status.requested);
        assert!(!status.supported);
        assert!(!status.enabled);
        assert_eq!(status.required_features(), wgpu::Features::empty());
    }

    #[test]
    fn supported_timestamp_request_enables_only_timestamp_query() {
        let status = timestamp_query_status(
            true,
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TEXTURE_COMPRESSION_BC,
        );

        assert!(status.enabled);
        assert_eq!(status.required_features(), wgpu::Features::TIMESTAMP_QUERY);
    }

    #[test]
    fn surface_cpu_stage_total_reconciles_host_api_intervals() {
        let stages = WindowSurfaceCpuStages {
            drawable_acquisition: std::time::Duration::from_nanos(1),
            command_encoder_finalization: std::time::Duration::from_nanos(2),
            queue_submission: std::time::Duration::from_nanos(3),
            presentation: std::time::Duration::from_nanos(4),
        };

        assert_eq!(stages.total(), std::time::Duration::from_nanos(10));
    }

    // A zero extent is what a minimized window reports, and the device rejects
    // it with a validation error that cannot be recovered from. Catching it
    // here is what lets a resize to nothing be ignored instead of fatal.
    #[test]
    fn a_zero_extent_is_rejected_before_it_reaches_the_device() {
        assert_eq!(check_extent(0, 480, 8192), Err(ExtentError::Width(0)));
        assert_eq!(check_extent(640, 0, 8192), Err(ExtentError::Height(0)));
    }

    // `WGPU_ADAPTER_NAME` is a substring match, case-insensitively, against the
    // adapter's reported name — the same rule wgpu's own helper applies.
    #[test]
    fn an_adapter_is_selected_by_a_case_insensitive_substring_of_its_name() {
        let adapters = ["Intel(R) UHD Graphics 630", "NVIDIA GeForce RTX 4090"];

        assert_eq!(adapter_matching_name(&adapters, "nvidia"), Some(1));
        assert_eq!(adapter_matching_name(&adapters, "UHD"), Some(0));
        assert_eq!(adapter_matching_name(&adapters, "geforce rtx"), Some(1));
    }

    // wgpu's `initialize_adapter_from_env` *panics* here (wgpu-29.0.4
    // src/util/init.rs:44). A panic out of surface construction escapes the
    // backend-widening loop that is supposed to try the next backend set, and
    // unwinds through a winit callback — which aborts on macOS. Reporting no
    // match lets the caller fall back to an ordinary adapter request instead.
    #[test]
    fn an_adapter_name_matching_nothing_reports_no_match_rather_than_panicking() {
        let adapters = ["Intel(R) UHD Graphics 630"];

        assert_eq!(adapter_matching_name(&adapters, "nvidia"), None);
        assert_eq!(adapter_matching_name(&[] as &[&str], "nvidia"), None);
    }

    // The renderer sizes presentation against the device limit, so an extent
    // above it has to be refused rather than clamped silently.
    #[test]
    fn an_extent_beyond_the_device_limit_is_rejected() {
        assert_eq!(check_extent(8193, 480, 8192), Err(ExtentError::Width(8193)));
        assert_eq!(
            check_extent(640, 8193, 8192),
            Err(ExtentError::Height(8193))
        );
        assert_eq!(check_extent(8192, 8192, 8192), Ok(()));
    }

    // Four bytes per pixel, and the multiplication has to happen in a width
    // that holds it: a 4K buffer is 33 MB, which overflows nothing here, but
    // the expression is the one place a narrowing would go unnoticed.
    #[test]
    fn the_frame_buffer_is_four_bytes_for_every_pixel() {
        assert_eq!(buffer_len((1, 1)), 4);
        assert_eq!(buffer_len((640, 480)), 640 * 480 * 4);
        assert_eq!(buffer_len((3840, 2160)), 33_177_600);
    }
}
