/// Texture filtering selected for a blit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlitSampling {
    Nearest,
    Linear,
}

/// Reproduce `CStdGL::PerformBlt`'s texture-filter choice.
///
/// A non-default application scale always enables linear filtering. At scale
/// one, non-exact blits are linear unless `PointFiltering` is enabled; exact
/// blits remain nearest-neighbour.
pub const fn stdgl_blit_sampling(
    application_scale: f32,
    point_filtering: bool,
    exact_blit: bool,
) -> BlitSampling {
    if application_scale != 1.0 || (!exact_blit && !point_filtering) {
        BlitSampling::Linear
    } else {
        BlitSampling::Nearest
    }
}
