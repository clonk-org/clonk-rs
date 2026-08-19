//! Where a CPU-composed frame goes.
//!
//! The primary window has always had a CPU presentation branch — the menus,
//! the loader and the console all reach the drawable that way. Until now its
//! destination was necessarily a [`WindowSurface`], which needs a wgpu device
//! even though nothing about the frame does, so a machine with no usable
//! adapter could not open a window at all (clonk-org/clonk-rs#299).
//!
//! This is the two destinations that branch can have. It is an enum rather
//! than a trait because the two are not interchangeable everywhere: a lost
//! surface can ask for a GPU device rebuild, and a software presenter has no
//! device to rebuild. Keeping the distinction in the type means that recovery
//! path cannot be written for a target that has no answer to it.

use clonk_surface::{SoftwarePresenter, WindowSurface};

/// The destination for a CPU-composed frame.
pub(crate) enum CpuTarget<'a> {
    /// The ordinary path: a wgpu surface that blits the frame.
    Gpu(&'a mut WindowSurface),
    /// The wgpu-free path.
    Software(&'a mut SoftwarePresenter),
}

/// Why a CPU frame could not be sized or presented.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CpuTargetError {
    #[error(transparent)]
    Extent(#[from] clonk_surface::ExtentError),
    #[error(transparent)]
    Surface(#[from] clonk_surface::SurfaceError),
    #[error(transparent)]
    Presenter(#[from] clonk_surface::PresenterError),
}

impl CpuTarget<'_> {
    /// The frame the renderer composes into, tightly packed `Rgba8888`.
    pub(crate) fn frame_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Gpu(surface) => surface.frame_mut(),
            Self::Software(presenter) => presenter.frame_mut(),
        }
    }

    /// The frame's extent, which is what the renderer lays out against.
    pub(crate) fn buffer_extent(&self) -> (u32, u32) {
        match self {
            Self::Gpu(surface) => surface.buffer_extent(),
            Self::Software(presenter) => presenter.frame_extent(),
        }
    }

    /// Resize the frame the renderer composes into.
    pub(crate) fn resize_buffer(&mut self, width: u32, height: u32) -> Result<(), CpuTargetError> {
        match self {
            Self::Gpu(surface) => surface.resize_buffer(width, height).map_err(Into::into),
            Self::Software(presenter) => {
                presenter.resize_frame((width, height)).map_err(Into::into)
            }
        }
    }

    /// Present the composed frame, reporting whether it actually reached the
    /// window.
    ///
    /// A GPU surface can decline: acquiring a drawable may report that none is
    /// available this frame, which the caller treats as a skipped presentation
    /// rather than a failure. The software presenter owns its buffer outright,
    /// so it either presents or errors.
    pub(crate) fn present(&mut self) -> Result<bool, CpuTargetError> {
        match self {
            Self::Gpu(surface) => surface
                .present_frame()
                .map(|presentation| presentation == clonk_surface::Presentation::Presented)
                .map_err(Into::into),
            Self::Software(presenter) => presenter.present().map(|()| true).map_err(Into::into),
        }
    }

    /// The wgpu surface behind this target, if it has one.
    ///
    /// Recovery from a lost surface rebuilds a device, which only exists on
    /// the GPU path; a `None` here is what tells that path there is nothing to
    /// rebuild rather than leaving it to guess.
    pub(crate) fn gpu_surface(&self) -> Option<&WindowSurface> {
        match self {
            Self::Gpu(surface) => Some(surface),
            Self::Software(_) => None,
        }
    }
}
