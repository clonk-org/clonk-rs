//! The window surface the application owns.
//!
//! This is the presentation half of what the vendored `pixels` 0.17.2 crate
//! used to provide: create a surface for a winit window, keep a CPU frame
//! buffer, and blit it to the drawable once per redraw.
//!
//! It lives here rather than in a patched dependency because the application
//! needs to own the two things `pixels` insists on owning — the `wgpu::Instance`
//! lifetime across several windows, and the surface-acquisition retry policy —
//! and neither is reachable from outside the crate.

pub use wgpu;

mod acquire;
mod blit;
pub mod capability;
pub mod software;
mod window;

pub use acquire::AcquireError;
pub use blit::BlitTransform;
pub use software::{present_pixel_perfect, SoftwarePresentError};
pub use window::{
    create_instance, ExtentError, FrameContext, Presentation, ProfiledPresentation, SurfaceError,
    TimestampQueryStatus, WindowSurface, WindowSurfaceBuildOptions, WindowSurfaceCpuStages,
};
