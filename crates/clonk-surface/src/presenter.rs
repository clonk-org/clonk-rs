//! A window presenter that never touches wgpu.
//!
//! [`WindowSurface`](crate::WindowSurface) needs an adapter and a device even
//! when the frame it presents was composed entirely on the CPU, so a machine
//! with no usable adapter cannot open an interactive window at all
//! (clonk-org/clonk-rs#299). This presenter is the alternative: it owns a
//! `softbuffer` surface, takes the same CPU frame, and puts it on the window
//! with the geometry [`crate::software::present_pixel_perfect`] derives from
//! the GPU blit's own fit.
//!
//! Not to be confused with `clonk-app`'s `SoftwareWindow`, which is a
//! developer window whose *contents* are software-drawn but which still
//! presents through wgpu. The distinction here is the presenter, not the
//! renderer: what changes is how pixels reach the screen, never how the
//! simulation runs or what it draws.
//!
//! The window handle is erased behind trait objects for the same reason
//! `WindowSurface` erases it into a `'static` surface — a generic parameter
//! here would propagate into every caller that merely holds a window.

use std::num::NonZeroU32;
use std::sync::Arc;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::software::{present_pixel_perfect, SoftwarePresentError};

/// A window this presenter can draw to.
///
/// `Send + Sync` because the application shares its window as an `Arc` across
/// the places that need it; the presenter itself is used from one thread.
pub trait PresentTarget: HasWindowHandle + HasDisplayHandle + Send + Sync {}

impl<T> PresentTarget for T where T: HasWindowHandle + HasDisplayHandle + Send + Sync {}

type ErasedTarget = Arc<dyn PresentTarget>;

/// Why a software presenter could not be built or could not present.
#[derive(Debug, thiserror::Error)]
pub enum PresenterError {
    /// `softbuffer` could not reach the platform's window system.
    #[error("software presenter could not attach to the window: {0}")]
    Attach(String),
    /// The drawable extent has a zero side, so there is nothing to present to.
    #[error("software presenter needs a non-zero drawable, got {width}x{height}")]
    EmptyDrawable { width: u32, height: u32 },
    /// The frame extent needs more memory than this target can address or
    /// the allocator can serve.
    #[error("software presenter cannot allocate a {width}x{height} frame")]
    FrameTooLarge { width: u32, height: u32 },
    /// Compositing the frame failed.
    #[error(transparent)]
    Composite(#[from] SoftwarePresentError),
}

/// The CPU frame the renderer draws into.
///
/// Split out from the presenter so its sizing — the part that decides how many
/// bytes the renderer is handed — is testable on a machine with no display.
#[derive(Debug, Default)]
struct CpuFrame {
    bytes: Vec<u8>,
    extent: (u32, u32),
}

impl CpuFrame {
    /// Resize and clear. Resizing is rare (a resolution change), so the frame
    /// is not preserved across it: keeping stale pixels of a different extent
    /// would present garbage for one frame.
    ///
    /// An extent whose byte count does not fit `usize` is refused rather than
    /// saturated. Saturating would hand `Vec::resize` a length no allocator
    /// can serve, turning a bad extent into an abort instead of an error the
    /// caller can report.
    fn resize(&mut self, extent: (u32, u32)) -> Result<(), PresenterError> {
        let (width, height) = extent;
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(PresenterError::FrameTooLarge { width, height })?;
        self.bytes.clear();
        self.bytes
            .try_reserve_exact(len)
            .map_err(|_| PresenterError::FrameTooLarge { width, height })?;
        self.bytes.resize(len, 0);
        self.extent = extent;
        Ok(())
    }
}

/// The drawable's extent as `softbuffer` needs it, or why it cannot be used.
///
/// A minimised window reports a zero side, which `softbuffer` cannot resize
/// to; refusing here is what keeps that from reaching it.
fn drawable_dimensions(extent: (u32, u32)) -> Result<(NonZeroU32, NonZeroU32), PresenterError> {
    let (width, height) = extent;
    NonZeroU32::new(width)
        .zip(NonZeroU32::new(height))
        .ok_or(PresenterError::EmptyDrawable { width, height })
}

/// A CPU frame buffer and the window it is presented to, with no wgpu in
/// sight.
pub struct SoftwarePresenter {
    surface: softbuffer::Surface<ErasedTarget, ErasedTarget>,
    frame: CpuFrame,
    drawable_extent: (u32, u32),
}

impl SoftwarePresenter {
    /// Attach to `window`, with a CPU frame of `frame_extent` presented into a
    /// drawable of `drawable_extent`.
    pub fn build(
        window: Arc<dyn PresentTarget>,
        frame_extent: (u32, u32),
        drawable_extent: (u32, u32),
    ) -> Result<Self, PresenterError> {
        let context = softbuffer::Context::new(Arc::clone(&window))
            .map_err(|error| PresenterError::Attach(error.to_string()))?;
        let surface = softbuffer::Surface::new(&context, window)
            .map_err(|error| PresenterError::Attach(error.to_string()))?;
        let mut presenter = Self {
            surface,
            frame: CpuFrame::default(),
            drawable_extent: (0, 0),
        };
        presenter.resize_frame(frame_extent)?;
        presenter.resize_drawable(drawable_extent)?;
        Ok(presenter)
    }

    /// The CPU frame the renderer draws into, tightly packed `Rgba8888`.
    pub fn frame_mut(&mut self) -> &mut [u8] {
        &mut self.frame.bytes
    }

    /// The CPU frame, for a screenshot or save thumbnail to read.
    pub fn frame(&self) -> &[u8] {
        &self.frame.bytes
    }

    /// The frame's extent, which is the logical size the renderer draws at.
    pub const fn frame_extent(&self) -> (u32, u32) {
        self.frame.extent
    }

    /// Resize the CPU frame, clearing it.
    ///
    /// Kept separate from [`Self::resize_drawable`] because the two change for
    /// different reasons: the frame follows the application's logical
    /// resolution, the drawable follows the window.
    pub fn resize_frame(&mut self, frame_extent: (u32, u32)) -> Result<(), PresenterError> {
        self.frame.resize(frame_extent)
    }

    /// Resize the drawable to follow the window.
    pub fn resize_drawable(&mut self, drawable_extent: (u32, u32)) -> Result<(), PresenterError> {
        let (width, height) = drawable_dimensions(drawable_extent)?;
        self.surface
            .resize(width, height)
            .map_err(|error| PresenterError::Attach(error.to_string()))?;
        self.drawable_extent = drawable_extent;
        Ok(())
    }

    /// Composite the CPU frame onto the window and show it.
    pub fn present(&mut self) -> Result<(), PresenterError> {
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| PresenterError::Attach(error.to_string()))?;
        present_pixel_perfect(
            &self.frame.bytes,
            self.frame.extent,
            self.drawable_extent,
            &mut buffer,
        )?;
        buffer
            .present()
            .map_err(|error| PresenterError::Attach(error.to_string()))
    }
}

impl std::fmt::Debug for SoftwarePresenter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `softbuffer::Surface` is not `Debug`, and the frame is megabytes.
        formatter
            .debug_struct("SoftwarePresenter")
            .field("frame_extent", &self.frame.extent)
            .field("drawable_extent", &self.drawable_extent)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizing_the_frame_allocates_exactly_one_rgba_pixel_per_cell() {
        let mut frame = CpuFrame::default();
        frame
            .resize((320, 200))
            .expect("an ordinary extent allocates");
        assert_eq!(frame.bytes.len(), 320 * 200 * 4);
        assert_eq!(frame.extent, (320, 200));
        assert!(frame.bytes.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn a_resized_frame_keeps_no_pixels_from_the_old_extent() {
        // Stale pixels of a different extent would present as garbage for one
        // frame after a resolution change.
        let mut frame = CpuFrame::default();
        frame.resize((4, 4)).expect("first extent allocates");
        frame.bytes.fill(0xab);
        frame.resize((2, 2)).expect("second extent allocates");
        assert_eq!(frame.bytes.len(), 2 * 2 * 4);
        assert!(frame.bytes.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn an_unallocatable_frame_extent_is_refused_rather_than_attempted() {
        // Saturating this would ask the allocator for `usize::MAX` bytes and
        // abort the process; the caller needs an error it can report instead.
        let mut frame = CpuFrame::default();
        assert!(matches!(
            frame.resize((u32::MAX, u32::MAX)),
            Err(PresenterError::FrameTooLarge { .. })
        ));
        assert_eq!(frame.extent, (0, 0), "a refused resize changes nothing");
    }

    #[test]
    fn a_zero_sided_drawable_is_refused_rather_than_presented_into() {
        // A minimised window reports a zero extent, which `softbuffer` cannot
        // resize to.
        assert!(matches!(
            drawable_dimensions((0, 480)),
            Err(PresenterError::EmptyDrawable {
                width: 0,
                height: 480
            })
        ));
        assert!(matches!(
            drawable_dimensions((640, 0)),
            Err(PresenterError::EmptyDrawable {
                width: 640,
                height: 0
            })
        ));
        let (width, height) = drawable_dimensions((640, 480)).expect("a real window presents");
        assert_eq!((width.get(), height.get()), (640, 480));
    }
}
