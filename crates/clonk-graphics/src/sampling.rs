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

/// `Graphics.HDExactBlits`: the application scale [`stdgl_blit_sampling`]
/// should judge a blit by.
///
/// `pApp->GetScale() != 1.f` is C++'s proxy for "this blit magnifies and
/// therefore needs filtering", which holds only because it assumes every facet
/// is authored at DefCore `Scale=100`. High-resolution art breaks the proxy: a
/// `Scale=200` sheet drawn at a 200% presentation scale covers exactly one
/// authored texel per device pixel. A caller that has established that 1:1
/// physical mapping passes `texel_exact`, which drops StdGL's
/// application-scale arm (src/StdGL.cpp:527) and leaves C++'s own exactness
/// rule to select the filter.
///
/// With `texel_exact` false this is the identity, so the default path stays
/// bit-exact with C++.
pub const fn hd_filter_scale(application_scale: f32, texel_exact: bool) -> f32 {
    if texel_exact {
        1.0
    } else {
        application_scale
    }
}
