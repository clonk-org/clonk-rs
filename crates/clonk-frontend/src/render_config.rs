use super::*;

/// Device/resource snapshot of the advanced CStdGL raster switches.
///
/// The native renderer reads these values while restoring its device and
/// then applies the immutable snapshot to later draw submissions. The Rust
/// backend keeps the same ownership model even though its renderer is CPU
/// based and can install the snapshot without recreating an OS GL context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvancedRendererConfig {
    pub no_alpha_add: bool,
    pub no_box_fades: bool,
    pub tex_indent: i32,
    pub blit_offset: i32,
    pub allowed_blit_modes: u32,
    pub shader: bool,
    pub use_shader_gamma: bool,
    pub disable_gamma: bool,
}

impl AdvancedRendererConfig {
    pub const DEFAULT: Self = Self {
        no_alpha_add: false,
        no_box_fades: false,
        tex_indent: 0,
        blit_offset: 0,
        allowed_blit_modes: 15,
        shader: false,
        use_shader_gamma: true,
        disable_gamma: false,
    };

    pub(crate) fn masked_blit_mode(self, mode: u32) -> u32 {
        mode & self.allowed_blit_modes
    }

    pub(crate) fn texture_indent(self) -> f32 {
        self.tex_indent as f32 / 1000.0
    }

    pub(crate) fn destination_offset(self) -> f32 {
        self.blit_offset as f32 / 100.0
    }

    pub(crate) fn has_adjusted_quad_geometry(self) -> bool {
        self.tex_indent != 0 || self.blit_offset != 0
    }

    pub(crate) fn changes_generic_textured_blit(
        self,
        requested_mode: u32,
        modulated: bool,
    ) -> bool {
        self.has_adjusted_quad_geometry()
            || self.masked_blit_mode(requested_mode) != requested_mode
            || (modulated && !self.shader && self.no_alpha_add)
    }

    pub(crate) fn changes_generic_color_quad(self) -> bool {
        self.blit_offset != 0 || self.no_box_fades
    }

    pub(crate) fn uses_fragment_gamma(self) -> bool {
        !self.disable_gamma && self.shader && self.use_shader_gamma
    }

    pub(crate) fn uses_monitor_gamma(self) -> bool {
        !self.disable_gamma && !self.uses_fragment_gamma()
    }
}

impl Default for AdvancedRendererConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

thread_local! {
    /// CStdDDraw owns one renderer/device on the presentation thread. Keep
    /// generic GUI helpers source-compatible while allowing a production
    /// frame to expose that immutable device snapshot to every nested draw.
    static ACTIVE_ADVANCED_RENDERER_CONFIG: Cell<Option<AdvancedRendererConfig>> =
        const { Cell::new(None) };
}

/// Restores the previous generic-draw renderer snapshot when a nested render
/// scope completes. The marker deliberately keeps the guard on the thread
/// whose TLS value it replaced.
#[must_use = "the renderer configuration remains active only while the guard is alive"]
pub struct AdvancedRendererConfigGuard {
    previous: Option<AdvancedRendererConfig>,
    _clonk_text_blit: clonk_graphics::clonk_font::ClonkTextBlitConfigGuard,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Drop for AdvancedRendererConfigGuard {
    fn drop(&mut self) {
        ACTIVE_ADVANCED_RENDERER_CONFIG.with(|active| active.set(self.previous));
    }
}

/// Activates one immutable CStdGL device snapshot for generic GUI/HUD draw
/// submissions on the current presentation thread. Scopes are nest-safe and
/// callers outside a scope retain the historical compatibility path.
pub fn activate_advanced_renderer_config(
    config: AdvancedRendererConfig,
) -> AdvancedRendererConfigGuard {
    let clonk_text_blit = clonk_graphics::clonk_font::activate_clonk_text_blit_config(
        clonk_graphics::clonk_font::ClonkTextBlitConfig {
            no_alpha_add: config.no_alpha_add,
            tex_indent: config.tex_indent,
            blit_offset: config.blit_offset,
            allowed_blit_modes: config.allowed_blit_modes,
            shader: config.shader,
        },
    );
    let previous = ACTIVE_ADVANCED_RENDERER_CONFIG.with(|active| active.replace(Some(config)));
    AdvancedRendererConfigGuard {
        previous,
        _clonk_text_blit: clonk_text_blit,
        _not_send_or_sync: PhantomData,
    }
}

pub(crate) fn active_advanced_renderer_config() -> Option<AdvancedRendererConfig> {
    ACTIVE_ADVANCED_RENDERER_CONFIG.with(Cell::get)
}
