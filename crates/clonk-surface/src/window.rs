//! The window surface itself: a drawable, a CPU frame buffer, and the blit
//! that puts one onto the other.

use crate::acquire::{acquire_drawable, AcquireError, Acquisition};
use crate::blit::{BlitTransform, Blitter};

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

/// What the render callback is handed alongside the encoder and drawable view.
pub struct FrameContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

/// A window's drawable, its CPU frame buffer, and the blit between them.
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_dimension_2d: u32,

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
        pollster::block_on(Self::build_async(
            instance,
            window,
            buffer_extent,
            surface_extent,
            present_mode,
        ))
    }

    async fn build_async<W>(
        instance: &wgpu::Instance,
        window: W,
        buffer_extent: (u32, u32),
        surface_extent: (u32, u32),
        present_mode: wgpu::PresentMode,
    ) -> Result<Self, SurfaceError>
    where
        W: wgpu::WindowHandle + raw_window_handle::HasDisplayHandle + 'static,
    {
        let surface = instance.create_surface(window)?;
        // `WGPU_ADAPTER_NAME` picks an adapter by name when it is set; without
        // it fall back to the ordinary request, which honours
        // `WGPU_POWER_PREF`.
        let adapter = match wgpu::util::initialize_adapter_from_env(instance, Some(&surface)).await
        {
            Ok(adapter) => Ok(adapter),
            Err(_) => {
                instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        compatible_surface: Some(&surface),
                        force_fallback_adapter: false,
                        power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(),
                    })
                    .await
            }
        }
        .map_err(|_| SurfaceError::AdapterNotFound)?;

        // Ask for everything the adapter offers: the renderer reads
        // `max_texture_dimension_2d` back off the device to decide whether a
        // presentation extent fits, and a defaulted limit would cap it far
        // below what the hardware can do.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
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
        // Presentation composites in byte space and relies on the surface
        // encode to restore those bytes, so an sRGB surface is required; the
        // fallback matches what every desktop backend actually offers.
        let surface_format = capabilities
            .formats
            .iter()
            .find(|format| format.is_srgb())
            .copied()
            .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);

        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        check_extent(buffer_extent.0, buffer_extent.1, max_texture_dimension_2d)?;

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

    pub const fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub const fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The drawable's format, which the scene renderer must target.
    pub const fn surface_texture_format(&self) -> wgpu::TextureFormat {
        self.surface_format
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
            return Ok(Presentation::Skipped);
        };

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

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(Presentation::Presented)
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

    // A zero extent is what a minimized window reports, and the device rejects
    // it with a validation error that cannot be recovered from. Catching it
    // here is what lets a resize to nothing be ignored instead of fatal.
    #[test]
    fn a_zero_extent_is_rejected_before_it_reaches_the_device() {
        assert_eq!(check_extent(0, 480, 8192), Err(ExtentError::Width(0)));
        assert_eq!(check_extent(640, 0, 8192), Err(ExtentError::Height(0)));
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
