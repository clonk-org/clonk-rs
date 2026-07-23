use super::*;

/// Opaque continuation for the ordered HUD half of one [`GraphicsSystem`]
/// frame. It owns the snapshot and gamma state captured when the world phase
/// began, so later layers cannot accidentally use a newer frame.
#[must_use = "a base frame must be completed with GraphicsSystem::render_frame_hud"]
pub struct PendingHudFrame<'snapshot> {
    snapshot: &'snapshot SimulationSnapshot,
    frame: u64,
    gamma: clonk_graphics::GammaRamp,
    fragment_gamma: bool,
    monitor_gamma: bool,
    pending_gamma_control_points: [u32; 3],
    graphics_system_identity: Arc<()>,
    generation: u64,
}

impl PendingHudFrame<'_> {
    fn draw_gamma(&self) -> Option<&clonk_graphics::GammaRamp> {
        self.fragment_gamma.then_some(&self.gamma)
    }
}

/// Continuation after per-viewport HUD controls and before the fullscreen
/// message/upper-board chrome.
#[must_use = "HUD player overlays must be completed with GraphicsSystem::render_frame_hud_chrome"]
pub struct PendingHudChromeFrame<'snapshot>(PendingHudFrame<'snapshot>);

struct PendingViewportForeground {
    surface: Surface,
    destination: SurfacePoint,
}

const MAX_TILED_UNDERLAY_CACHE_ENTRIES: usize = 8;

pub(crate) struct TiledUnderlayCacheEntry {
    width: u32,
    height: u32,
    format: PixelFormat,
    origin_x: i32,
    origin_y: i32,
    surface: Surface,
}

/// Gamma-transformed `Background.png` tiles are presentation-invariant until
/// the output geometry, image, gamma ramp, or tile origin changes. Retain the
/// completed backing surfaces so ordinary frames only copy their bytes rather
/// than running the fragment gamma lookup for every background pixel again.
///
/// The small fixed entry bound covers the fullscreen backing and the stable
/// split-screen viewport rectangles without letting transient capture sizes or
/// origins retain unbounded frame-sized allocations.
pub(crate) struct TiledUnderlayCache {
    frame_surface: Option<(u32, u32, PixelFormat)>,
    background: Option<ImageData>,
    gamma: Option<clonk_graphics::GammaRamp>,
    renderer_config: Option<AdvancedRendererConfig>,
    pub(crate) entries: Vec<TiledUnderlayCacheEntry>,
    #[cfg(test)]
    pub(crate) rasterizations: usize,
}

impl Default for TiledUnderlayCache {
    fn default() -> Self {
        Self {
            frame_surface: None,
            background: None,
            gamma: None,
            renderer_config: None,
            entries: Vec::new(),
            #[cfg(test)]
            rasterizations: 0,
        }
    }
}

impl TiledUnderlayCache {
    pub(crate) fn begin_frame(
        &mut self,
        surface: &Surface,
        background: Option<&ImageData>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let frame_surface = Some((surface.width(), surface.height(), surface.format()));
        if self.frame_surface != frame_surface {
            self.entries.clear();
            self.frame_surface = frame_surface;
        }
        self.prepare_source(background, gamma);
    }

    fn prepare_source(
        &mut self,
        background: Option<&ImageData>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let renderer_config = active_advanced_renderer_config();
        if self.background.as_ref() == background
            && self.gamma.as_ref() == gamma
            && self.renderer_config == renderer_config
        {
            return;
        }
        self.entries.clear();
        self.background = background.cloned();
        self.gamma = gamma.cloned();
        self.renderer_config = renderer_config;
    }

    pub(crate) fn draw(
        &mut self,
        surface: &mut Surface,
        background: &ImageData,
        origin_x: i32,
        origin_y: i32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.prepare_source(Some(background), gamma);

        // A recording surface is presented by flattening its command stream;
        // CPU cache bytes are intentionally not part of that stream. Emit the
        // retained image tiles directly so scroll borders and translucent FoW
        // keep their underlay on the GPU path.
        if surface.is_gpu_scene_capture_active() {
            tile_image_on_surface(surface, background, origin_x, origin_y, gamma);
            return;
        }

        let width = surface.width();
        let height = surface.height();
        let format = surface.format();
        if let Some(entry) = self.entries.iter().find(|entry| {
            entry.width == width
                && entry.height == height
                && entry.format == format
                && entry.origin_x == origin_x
                && entry.origin_y == origin_y
        }) {
            surface.pixels_mut().copy_from_slice(entry.surface.pixels());
            return;
        }

        // Malformed or empty images make the uncached operation a no-op.
        // Preserve that behavior instead of copying a zeroed cache surface.
        let source_stride = (background.width() as usize).saturating_mul(4);
        if background.width() == 0
            || background.height() == 0
            || width == 0
            || height == 0
            || background.pixels().len()
                < source_stride.saturating_mul(background.height() as usize)
            || surface.stride() < (width as usize).saturating_mul(4)
        {
            tile_image_on_surface(surface, background, origin_x, origin_y, gamma);
            return;
        }

        let mut cached = Surface::new(width, height, format);
        tile_image_on_surface(&mut cached, background, origin_x, origin_y, gamma);
        surface.pixels_mut().copy_from_slice(cached.pixels());
        if self.entries.len() >= MAX_TILED_UNDERLAY_CACHE_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(TiledUnderlayCacheEntry {
            width,
            height,
            format,
            origin_x,
            origin_y,
            surface: cached,
        });
        #[cfg(test)]
        {
            self.rasterizations += 1;
        }
    }
}

pub struct GraphicsSystem {
    pub(crate) surface: Surface,
    tiled_underlay_cache: TiledUnderlayCache,
    font: Arc<dyn TextFont>,
    /// The CStdFont-faithful fonts; the HUD's FontRegular when present
    /// (C4GraphicsResource::InitFonts, src/C4GraphicsResource.cpp:144-169).
    clonk_fonts: Option<Arc<ClonkFontSet>>,
    scenario_label_text: String,
    /// Per-player HUD state fed by [`Self::update_overlay`].
    pub(crate) hud_players: Vec<PlayerOverlay>,
    crew_name_labels: Vec<CrewNameOverlay>,
    /// The two native film-replay gates in `C4Viewport::Draw` and
    /// `C4Viewport::DrawOverlay` (src/C4Viewport.cpp:838-881,1088).
    pub(crate) viewport_overlays_visible: bool,
    game_time_seconds: u64,
    pub(crate) message_board: MessageBoardOverlay,
    clock_text: Option<String>,
    frames_per_second: Option<i32>,
    pub(crate) upper_board_mode: hud::UpperBoardMode,
    /// `C4UpperBoard::TextWidth`, fixed by `Init` until fullscreen chrome is
    /// reinitialized (src/C4UpperBoard.cpp:102-121).
    upper_board_text_width: Option<i32>,
    /// `Config.Graphics.ShowPlayerHUDAlways` (src/C4Viewport.cpp:1287-1321).
    show_player_hud_always: bool,
    /// `Config.Graphics.SplitscreenDividers` (src/C4GraphicsSystem.cpp:389-394).
    splitscreen_dividers: bool,
    /// `Config.Graphics.FireParticles` gates C++'s extended Fire/Fire2 pass.
    /// The Rust renderer currently has only the simple object fire facet,
    /// which remains unconditional as C++'s fallback does.
    fire_particles: bool,
    /// `Config.Graphics.ShowPortraits` / `ShowCommands` / `ShowCommandKeys`
    /// (src/C4Config.cpp:448-450, default true).
    pub(crate) show_portraits: bool,
    show_commands: bool,
    show_command_keys: bool,
    /// Debug FRAME/STATUS lines; `None` hides them (default HUD).
    debug_hud_text: Option<(String, String)>,
    debug_draw_flags: DebugDrawFlags,
    definition_debug_geometry: HashMap<DefinitionId, DefinitionDebugGeometry>,
    network_status_text: Option<String>,
    pub(crate) viewport_x: f32,
    pub(crate) viewport_y: f32,
    pub(crate) viewport_zoom: f32,
    /// Global logical `Config.Graphics.ResX`. Per-viewport rendering
    /// temporarily replaces `surface_width`, but cursor-sheet selection is
    /// resolution-global in C++.
    logical_resolution_width: u32,
    /// `Application.GetScale()` / `Config.Graphics.Scale / 100` supplied by
    /// the frame presenter before composition.
    presentation_scale: f32,
    /// Live `Config.Graphics.PointFiltering`. At scale one this forces point
    /// sampling for non-exact runtime blits; non-100% application scale still
    /// forces linear filtering in StdGL.
    point_filtering: bool,
    /// Immutable CStdGL device/resource options installed by the application.
    advanced_renderer_config: AdvancedRendererConfig,
    surface_width: u32,
    surface_height: u32,
    fallback_ground_height: i32,
    world_width: i32,
    pub(crate) world_height: i32,
    object_sprites: Arc<HashMap<String, DefinitionSprite>>,
    particle_sprites: Arc<HashMap<String, ParticleRenderDefinition>>,
    rotateable_definitions: HashSet<DefinitionId>,
    cursor_atlas: Arc<CursorAtlas>,
    pub(crate) hud_graphics: Arc<HudGraphics>,
    owner_colored_crew_icons: HashMap<(GpuTextureId, Color), ImageData>,
    pub(crate) game_palette: Arc<GamePalette>,
    pub(crate) active_viewports: Vec<ActiveViewport>,
    rendered_object_audibility_calls: RenderedObjectAudibilityCalls,
    pub(crate) content_audibility_facet: Option<AudibilityFacet>,
    pub(crate) full_audibility_facet: Option<AudibilityFacet>,
    pub(crate) current_audibility_facet: Option<AudibilityFacet>,
    pub(crate) camera_states: HashMap<CameraKey, CameraState>,
    /// FreeView input may arrive after a graphics rebuild but before the
    /// first physical viewport is projected. Retain that primary-camera
    /// displacement so the key press is not lost before the next render.
    pub(crate) pending_primary_observer_scroll: Vector2,
    /// Gamma currently installed in CStdDDraw. A runtime SetGamma mutates the
    /// snapshot controls during the game tick, but C4GraphicsSystem applies
    /// them only after drawing that render pass; a fresh graphics system has
    /// already received InitGame's explicit ApplyGamma.
    pub(crate) active_gamma_control_points: Option<[u32; 3]>,
    render_phase_identity: Arc<()>,
    render_phase_generation: u64,
    pending_viewport_foregrounds: Vec<PendingViewportForeground>,
    /// C4ConfigGeneral::ScrollSmooth. Config plumbing lives above the
    /// frontend; retain the exact C++ default and clamp at use meanwhile.
    scroll_smooth: i32,
    sky: Option<SkyRenderState>,
    /// Advanced quad geometry bakes lighting into the sky source. Keep one
    /// mutable retained texture and advance its revision when those bytes
    /// change instead of allocating a new GPU identity every frame.
    retained_lit_sky: Option<RetainedLitSkyTexture>,
    /// Native material texture surfaces by byte-folded texture name. Both
    /// Surface32 PNGs and indexed Surface8 BMPs participate in landscape
    /// patterns; only Surface32 is eligible for graphical PXS.
    material_textures: Arc<HashMap<String, MaterialTextureSurface>>,
    /// C4MaterialCore presentation fields by lowercase material name.
    material_render_info: Arc<HashMap<String, MaterialRenderInfo>>,
    /// Persistent C++-style Surface32 counterpart. The retained PixelGrid
    /// clone anchors COW ancestry, allowing changed rectangles to patch the
    /// RGBA bytes without rebuilding the complete landscape.
    pub(crate) landscape_cache: Option<LandscapeRenderCache>,
    /// Retained screen-space source layers for legacy column-only worlds.
    /// Ground and liquid remain separate because native applies landscape
    /// modulation, fog, gamma and alpha blending to each painter-order pass.
    column_ground_cache: Option<Surface>,
    column_liquid_cache: Option<Surface>,
    /// Optional Graphics.c4g/Liquid.png shader input. The density plane
    /// supplies C++'s separate alpha mask.
    liquid_animation_image: Option<ImageData>,
    /// CStdGL::BlitLandscape keeps this cycle in a function-static array, so
    /// renderer rebuilds and graphics-resource swaps must not reset it.
    pub(crate) liquid_animation_cycle: LiquidAnimationCycle,
    /// Presentation-only `SafeRandom` stream. C++ deliberately keeps this
    /// outside the synchronized game RNG; DrawBolt consumes it while drawing.
    pub(crate) presentation_rng: SafeRng,
    /// Active viewport `CClrModAddMap`. It is installed for world drawing and
    /// removed before parallax GUI/overlay rendering.
    pub(crate) active_fog_map: Option<Arc<ClrModMap>>,
    /// `C4D_IgnoreFoW` temporarily disables the map around an object's base
    /// draw without affecting the surrounding viewport pass.
    fog_suppression_depth: u32,
}

impl GraphicsSystem {
    pub fn new(
        surface_width: u32,
        surface_height: u32,
        fallback_ground_height: i32,
        scenario_label: &str,
        font: Arc<dyn TextFont>,
        object_sprites: Arc<HashMap<String, DefinitionSprite>>,
        cursor_atlas: Arc<CursorAtlas>,
        hud_graphics: Arc<HudGraphics>,
    ) -> Self {
        let mut surface = Surface::new(
            surface_width,
            surface_height,
            clonk_graphics::PixelFormat::Rgba8888,
        );
        surface.fill(Color::opaque(8, 12, 24));

        Self {
            surface,
            tiled_underlay_cache: TiledUnderlayCache::default(),
            font,
            clonk_fonts: None,
            scenario_label_text: scenario_label.to_string(),
            hud_players: Vec::new(),
            crew_name_labels: Vec::new(),
            viewport_overlays_visible: true,
            game_time_seconds: 0,
            message_board: MessageBoardOverlay::default(),
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: hud::UpperBoardMode::Full,
            upper_board_text_width: None,
            show_player_hud_always: true,
            splitscreen_dividers: true,
            fire_particles: true,
            show_portraits: true,
            show_commands: true,
            show_command_keys: true,
            debug_hud_text: None,
            debug_draw_flags: DebugDrawFlags::default(),
            definition_debug_geometry: HashMap::new(),
            network_status_text: None,
            viewport_x: 0.0,
            viewport_y: 0.0,
            viewport_zoom: 1.0,
            logical_resolution_width: surface_width,
            presentation_scale: 1.0,
            point_filtering: false,
            advanced_renderer_config: AdvancedRendererConfig::DEFAULT,
            surface_width,
            surface_height,
            fallback_ground_height,
            world_width: surface_width as i32,
            world_height: fallback_ground_height.max(surface_height as i32).max(0),
            object_sprites,
            particle_sprites: Arc::new(HashMap::new()),
            rotateable_definitions: HashSet::new(),
            cursor_atlas,
            hud_graphics,
            owner_colored_crew_icons: HashMap::new(),
            game_palette: Arc::new(GamePalette::default()),
            active_viewports: Vec::new(),
            rendered_object_audibility_calls: HashMap::new(),
            content_audibility_facet: None,
            full_audibility_facet: None,
            current_audibility_facet: None,
            camera_states: HashMap::new(),
            pending_primary_observer_scroll: Vector2::ZERO,
            active_gamma_control_points: None,
            render_phase_identity: Arc::new(()),
            render_phase_generation: 0,
            pending_viewport_foregrounds: Vec::new(),
            scroll_smooth: DEFAULT_SCROLL_SMOOTH,
            sky: None,
            retained_lit_sky: None,
            material_textures: Arc::new(HashMap::new()),
            material_render_info: Arc::new(HashMap::new()),
            landscape_cache: None,
            column_ground_cache: None,
            column_liquid_cache: None,
            liquid_animation_image: None,
            liquid_animation_cycle: LiquidAnimationCycle::default(),
            presentation_rng: SafeRng::default(),
            active_fog_map: None,
            fog_suppression_depth: 0,
        }
    }

    pub fn set_object_sprites(&mut self, sprites: Arc<HashMap<String, DefinitionSprite>>) {
        self.object_sprites = sprites;
    }

    /// Look up one installed definition sprite by its `sprite_map_key`.
    pub fn object_sprite(&self, key: &str) -> Option<&DefinitionSprite> {
        self.object_sprites.get(key)
    }

    /// Install the final exact-case, post-overload particle render catalog.
    pub fn set_particle_sprites(
        &mut self,
        sprites: Arc<HashMap<String, ParticleRenderDefinition>>,
    ) {
        self.particle_sprites = sprites;
    }

    pub fn set_presentation_scale(&mut self, scale: f32) {
        self.presentation_scale = if scale.is_finite() {
            scale.max(f32::EPSILON)
        } else {
            1.0
        };
    }

    pub fn set_runtime_sprite_filtering(&mut self, scale: f32, point_filtering: bool) {
        self.set_presentation_scale(scale);
        self.set_point_filtering(point_filtering);
    }

    pub fn set_point_filtering(&mut self, point_filtering: bool) {
        self.point_filtering = point_filtering;
    }

    pub fn point_filtering(&self) -> bool {
        self.point_filtering
    }

    /// Installs one complete renderer snapshot. Callers replace the value as
    /// a unit, mirroring a native device/resource restore rather than
    /// exposing independently mutable draw flags.
    pub fn set_advanced_renderer_config(&mut self, config: AdvancedRendererConfig) {
        self.advanced_renderer_config = config;
    }

    pub fn advanced_renderer_config(&self) -> AdvancedRendererConfig {
        self.advanced_renderer_config
    }

    pub fn inherit_advanced_renderer_config(&mut self, previous: &Self) {
        self.advanced_renderer_config = previous.advanced_renderer_config;
    }

    pub fn fragment_gamma_enabled(&self) -> bool {
        self.advanced_renderer_config.uses_fragment_gamma()
    }

    pub fn monitor_gamma_enabled(&self) -> bool {
        self.advanced_renderer_config.uses_monitor_gamma()
    }

    pub fn apply_monitor_gamma(&mut self, gamma: &clonk_graphics::GammaRamp) {
        if self.monitor_gamma_enabled() {
            gamma.apply_to_surface(&mut self.surface);
        }
    }

    fn configured_blit(&self, blit: SpriteBlitState) -> SpriteBlitState {
        blit.with_renderer_config(self.advanced_renderer_config)
    }

    pub fn inherit_runtime_sprite_filtering(&mut self, previous: &Self) {
        self.presentation_scale = previous.presentation_scale;
        self.point_filtering = previous.point_filtering;
    }

    pub(crate) fn runtime_sprite_blit(
        &self,
        source: FloatSourceRect,
        destination_size: (f32, f32),
        transformed: bool,
    ) -> (FloatSourceRect, BlitSampling) {
        let source = source.with_scaling_correction(self.presentation_scale != 1.0);
        let exact = !transformed
            && source.width == destination_size.0
            && source.height == destination_size.1;
        (
            source,
            stdgl_blit_sampling(self.presentation_scale, self.point_filtering, exact),
        )
    }

    pub fn set_rotateable_definitions(&mut self, definitions: HashSet<DefinitionId>) {
        self.rotateable_definitions = definitions;
    }

    pub fn set_definition_debug_geometry(
        &mut self,
        geometry: HashMap<DefinitionId, DefinitionDebugGeometry>,
    ) {
        self.definition_debug_geometry = geometry;
    }

    pub fn set_debug_draw_flags(&mut self, flags: DebugDrawFlags) {
        self.debug_draw_flags = flags;
    }

    pub fn debug_draw_flags(&self) -> DebugDrawFlags {
        self.debug_draw_flags
    }

    /// Pipe-delimited CStdFont markup produced by the network runtime.
    pub fn set_network_status_text(&mut self, text: Option<String>) {
        self.network_status_text = text;
    }

    pub fn network_status_text(&self) -> Option<&str> {
        self.network_status_text.as_deref()
    }

    pub fn set_world_width(&mut self, world_width: i32) {
        self.world_width = world_width.max(self.surface_width as i32);
    }

    pub fn set_world_height(&mut self, world_height: i32) {
        self.world_height = world_height.max(self.surface_height as i32);
    }

    pub fn set_world_dimensions(&mut self, world_width: i32, world_height: i32) {
        self.set_world_width(world_width);
        self.set_world_height(world_height);
    }

    pub fn set_material_textures(&mut self, textures: Arc<HashMap<String, ImageData>>) {
        self.material_textures = Arc::new(
            textures
                .iter()
                .map(|(name, image)| {
                    (
                        name.clone(),
                        MaterialTextureSurface::surface32(image.clone()),
                    )
                })
                .collect(),
        );
        self.landscape_cache = None;
    }

    pub fn set_material_texture_surfaces(
        &mut self,
        textures: Arc<HashMap<String, MaterialTextureSurface>>,
    ) {
        self.material_textures = textures;
        self.landscape_cache = None;
    }

    pub fn set_material_render_info(
        &mut self,
        render_info: Arc<HashMap<String, MaterialRenderInfo>>,
    ) {
        self.material_render_info = render_info;
        self.landscape_cache = None;
    }

    /// Installs the opt-in Graphics.c4g liquid-animation noise texture.
    pub fn set_liquid_animation(&mut self, image: Option<ImageData>) {
        self.liquid_animation_image = image;
    }

    /// Carries CStdGL's process-presentation cycle across an app-owned
    /// [`GraphicsSystem`] rebuild without exposing or synchronizing it.
    pub fn inherit_liquid_animation_cycle(&mut self, previous: &Self) {
        self.liquid_animation_cycle = previous.liquid_animation_cycle;
    }

    /// Preserve FreeView input across consecutive output rebuilds that occur
    /// before any physical viewport has been projected.
    pub fn inherit_pending_observer_scroll(&mut self, previous: &Self) {
        self.pending_primary_observer_scroll = previous.pending_primary_observer_scroll;
    }

    /// Preserve process-presentation debug state across an output-only
    /// rebuild such as a window resize. A game reset constructs a fresh
    /// [`GraphicsSystem`] and therefore still clears these native toggles.
    pub fn inherit_debug_draw_state(&mut self, previous: &Self) {
        self.debug_draw_flags = previous.debug_draw_flags;
        self.definition_debug_geometry = previous.definition_debug_geometry.clone();
        self.network_status_text = previous.network_status_text.clone();
    }

    pub fn set_sky(&mut self, sky: Option<SkyRenderState>) {
        self.sky = sky;
    }

    /// Set `Config.General.ScrollSmooth` for subsequent viewport renders.
    /// C++ stores the raw value and clamps it to 1..=50 in AdjustPosition.
    pub fn set_scroll_smooth(&mut self, scroll_smooth: i32) {
        self.scroll_smooth = scroll_smooth;
    }

    pub fn hud_graphics(&self) -> Arc<HudGraphics> {
        Arc::clone(&self.hud_graphics)
    }

    pub fn set_game_palette(&mut self, palette: Arc<GamePalette>) {
        self.game_palette = palette;
    }

    pub fn game_palette(&self) -> Arc<GamePalette> {
        Arc::clone(&self.game_palette)
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn surface_mut(&mut self) -> &mut Surface {
        &mut self.surface
    }

    pub fn begin_gpu_scene_capture(&mut self) {
        self.surface.begin_gpu_scene_capture();
    }

    pub fn finish_gpu_scene_capture(
        &mut self,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Option<clonk_graphics::GpuScene> {
        let extent = [self.surface.width(), self.surface.height()];
        self.surface
            .take_gpu_scene_capture()
            .map(|recorder| recorder.into_scene(extent, Color::opaque(8, 12, 24), gamma))
    }

    pub(crate) fn fog_draw_context(&self) -> Option<FogDrawContext> {
        if self.fog_suppression_depth != 0 {
            return None;
        }
        Some(FogDrawContext {
            map: Arc::clone(self.active_fog_map.as_ref()?),
            zoom: self.viewport_zoom.max(MIN_VIEWPORT_ZOOM),
        })
    }

    fn fog_box_sampler(
        &self,
        world_aligned: bool,
    ) -> Option<(FogDrawContext, Option<FogSpriteSampler>)> {
        let fog = self.fog_draw_context()?;
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let source_origin = if world_aligned {
            (self.viewport_x, self.viewport_y)
        } else {
            (0.0, 0.0)
        };
        let chunk_size = (
            fog.map.resolution_x as f32,
            fog.map.resolution_y as f32,
        );
        let sampler = FogSpriteSampler::new_with_chunks(
            &fog,
            (
                0.0,
                0.0,
                self.surface_width as f32,
                self.surface_height as f32,
            ),
            (
                source_origin.0,
                source_origin.1,
                self.surface_width as f32 / zoom,
                self.surface_height as f32 / zoom,
            ),
            chunk_size,
            false,
            |x, y| (x, y),
        );
        Some((fog, sampler))
    }

    fn draw_world_color_pixel(
        &mut self,
        x: u32,
        y: u32,
        color: Color,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let color = self
            .fog_draw_context()
            .map_or(color, |fog| fog.color_at(color, x as i32, y as i32));
        self.draw_prepared_world_color_pixel(x, y, color, gamma);
    }

    fn draw_prepared_world_color_pixel(
        &mut self,
        x: u32,
        y: u32,
        color: Color,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if color.a == 0 {
            return;
        }
        blend_prepared_sprite_fragment(
            &mut self.surface,
            x,
            y,
            PreparedSpriteFragment::Legacy(color),
            SpriteBlitState::normal(),
            gamma,
        );
    }

    pub(crate) fn fill_world_color(
        &mut self,
        mut color: Color,
        world_aligned: bool,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if self.surface_width == 0 || self.surface_height == 0 {
            return;
        }
        let offset = self.advanced_renderer_config.destination_offset();
        if self.active_fog_map.is_none() && self.advanced_renderer_config.no_box_fades {
            color = normalize_quad_colors([color; 4]);
        }
        let x_start = ((offset - 0.5).ceil() as i32).clamp(0, self.surface_width as i32);
        let y_start = ((offset - 0.5).ceil() as i32).clamp(0, self.surface_height as i32);
        let x_end = ((offset + self.surface_width as f32 - 0.5).ceil() as i32)
            .clamp(0, self.surface_width as i32);
        let y_end = ((offset + self.surface_height as f32 - 0.5).ceil() as i32)
            .clamp(0, self.surface_height as i32);
        let fog = self.fog_box_sampler(world_aligned);
        if self.surface.is_gpu_scene_capture_active()
            && fog.as_ref().is_none_or(|(_, sampler)| sampler.is_some())
        {
            if let Some((_, Some(sampler))) = fog.as_ref() {
                for quad in &sampler.quads {
                    let left = offset + quad.x.0 / sampler.source_width * self.surface_width as f32;
                    let right =
                        offset + quad.x.1 / sampler.source_width * self.surface_width as f32;
                    let top =
                        offset + quad.y.0 / sampler.source_height * self.surface_height as f32;
                    let bottom =
                        offset + quad.y.1 / sampler.source_height * self.surface_height as f32;
                    let mut colors = quad
                        .modulation
                        .map(|modulation| modulate_surface_color(color, modulation));
                    if self.advanced_renderer_config.no_box_fades {
                        colors = [normalize_quad_colors(colors); 4];
                    }
                    record_gpu_solid_quad(
                        &mut self.surface,
                        (left, top, right, bottom),
                        colors,
                        GpuBlend::Normal,
                        gamma.is_some_and(|gamma| !gamma.is_passthrough()),
                    );
                }
            } else {
                record_gpu_solid_quad(
                    &mut self.surface,
                    (
                        offset,
                        offset,
                        offset + self.surface_width as f32,
                        offset + self.surface_height as f32,
                    ),
                    [color; 4],
                    GpuBlend::Normal,
                    gamma.is_some_and(|gamma| !gamma.is_passthrough()),
                );
            }
            return;
        }
        if fog.is_none() {
            if offset == 0.0 {
                self.surface
                    .fill(gamma.map_or(color, |gamma| gamma_encode_fragment(color, gamma)));
            } else {
                for y in y_start..y_end {
                    for x in x_start..x_end {
                        self.draw_prepared_world_color_pixel(x as u32, y as u32, color, gamma);
                    }
                }
            }
            return;
        }
        let Some((fog, sampler)) = fog else {
            unreachable!("the no-fog path returned above");
        };
        let raster_axes = sampler.as_ref().map(|sampler| {
            sampler.raster_axes_with_destination_offset(
                self.surface_width,
                self.surface_height,
                offset,
                offset,
            )
        });
        for y in y_start as u32..y_end as u32 {
            for x in x_start as u32..x_end as u32 {
                let color = match (sampler.as_ref(), raster_axes.as_ref()) {
                    (Some(sampler), Some((x_samples, y_samples)))
                        if self.advanced_renderer_config.no_box_fades =>
                    {
                        sampler.normalized_vertical_color_at_axes(
                            x_samples[x as usize],
                            y_samples[y as usize],
                            |_| color,
                        )
                    }
                    (Some(sampler), Some((x_samples, y_samples))) => {
                        sampler.color_at_axes(color, x_samples[x as usize], y_samples[y as usize])
                    }
                    _ => fog.color_at(color, x as i32, y as i32),
                };
                self.draw_prepared_world_color_pixel(x, y, color, gamma);
            }
        }
    }

    /// Output rectangle of the player's active viewport. Viewport-owned GUI
    /// such as C4ObjectMenu aligns inside this area, not the full backbuffer
    /// (C4Viewport::DrawMenu, C4Viewport.cpp:967-1014).
    pub fn viewport_rect(&self, owner: i32) -> Option<SurfaceRect> {
        self.active_viewports
            .iter()
            .find(|viewport| viewport.owner == owner)
            .map(|viewport| viewport.rect)
    }

    /// Preferred rectangle used by fullscreen `C4GUI::Dialog` placement.
    /// Ordinarily this is the viewport area between the upper and message
    /// boards. A mouse-controlled player's viewport replaces it when that
    /// viewport is laid out (`C4Viewport::SetOutputSize`).
    pub fn preferred_dialog_rect(&self, mouse_owner: Option<i32>) -> SurfaceRect {
        self.preferred_dialog_rect_for_upper_board_mode(mouse_owner, self.upper_board_mode)
    }

    /// Mode-explicit form used when app configuration has changed before the
    /// next frame's [`GraphicsOverlay`] has synchronized renderer state.
    pub fn preferred_dialog_rect_for_upper_board_mode(
        &self,
        mouse_owner: Option<i32>,
        upper_board_mode: hud::UpperBoardMode,
    ) -> SurfaceRect {
        if let Some(rect) = mouse_owner.and_then(|owner| self.viewport_rect(owner)) {
            return rect;
        }

        if !self.hud_chrome_active() {
            return SurfaceRect::new(0, 0, self.surface_width, self.surface_height);
        }

        let top = hud::upper_board_reserved_height(upper_board_mode)
            .clamp(0, self.surface_height as i32);
        let bottom = self
            .message_board_height()
            .clamp(0, self.surface_height as i32);
        let height = (self.surface_height as i32)
            .saturating_sub(top)
            .saturating_sub(bottom);
        if height <= 0 {
            return SurfaceRect::new(0, 0, self.surface_width, self.surface_height);
        }
        SurfaceRect::new(0, top, self.surface_width, height as u32)
    }

    /// Projection state for every active viewport, in rendered layout order.
    /// The index identifies the viewport record even when owners repeat.
    pub fn active_viewport_projections(&self) -> Vec<ActiveViewportProjection> {
        self.active_viewports
            .iter()
            .enumerate()
            .map(|(index, viewport)| ActiveViewportProjection {
                index,
                owner: viewport.owner,
                is_no_owner_viewport: viewport.is_no_owner_viewport,
                rect: viewport.rect,
                content_rect: viewport.content_rect,
                target_x: viewport.target_x,
                target_y: viewport.target_y,
                logical_width: viewport.logical_width,
                logical_height: viewport.logical_height,
                content_origin_x: viewport.viewport_x,
                content_origin_y: viewport.viewport_y,
                zoom: viewport.zoom,
            })
            .collect()
    }

    /// Ordered special-object audibility calls produced by the most recent
    /// completed world render. A skipped render deliberately leaves this map
    /// untouched, matching the lifetime of C4Object::Audible/AudiblePan.
    pub fn rendered_object_audibility_calls(&self) -> &RenderedObjectAudibilityCalls {
        &self.rendered_object_audibility_calls
    }

    /// Delete smoothing state owned by one destroyed physical C4Viewport.
    /// All of its Rust-only expanded camera slots share the same identity.
    pub fn drop_physical_camera(&mut self, identity: u64) {
        self.camera_states.retain(|key, _| {
            !matches!(
                key,
                CameraKey::Physical {
                    identity: candidate,
                    ..
                } if *candidate == identity
            )
        });
    }

    /// Apply one direct C4MouseControl scroll step to an unassigned
    /// fullscreen observer viewport. Temporary film-view player assignment
    /// does not change this physical classification. A primary scroll queued
    /// before the first post-rebuild projection is applied by that render.
    /// Returns false for any other missing or owned camera; a clamped scroll
    /// returns true.
    pub fn scroll_observer_viewport(&mut self, index: usize, delta: Vector2) -> bool {
        let Some(viewport) = self.active_viewports.get(index) else {
            if index == 0 && self.active_viewports.is_empty() {
                self.queue_primary_observer_scroll(delta);
                return true;
            }
            return false;
        };
        if !viewport.is_no_owner_viewport {
            return false;
        }
        let key = viewport.camera_key;
        let view_width = viewport.logical_width;
        let view_height = viewport.logical_height;
        let world_width = viewport.world_width;
        let world_height = viewport.world_height;
        let Some(state) = self.camera_states.get_mut(&key) else {
            return false;
        };

        state.view_x = state.view_x.saturating_add(delta.x);
        state.view_y = state.view_y.saturating_add(delta.y);
        let (view_x, view_y) = state.no_owner_position(
            view_width,
            view_height,
            world_width,
            world_height,
        );

        // C4Viewport::UpdateViewPosition changes the live projection in the
        // same call. Keep pointer routing and another edge tick observable
        // without waiting for the next world render.
        let viewport = &mut self.active_viewports[index];
        let border_left = (-view_x).max(0).min(view_width);
        let border_top = (-view_y).max(0).min(view_height);
        let border_right = (view_width - world_width + view_x)
            .max(0)
            .min(view_width - border_left);
        let border_bottom = (view_height - world_height + view_y)
            .max(0)
            .min(view_height - border_top);
        let offset_x =
            scaled_camera_border(border_left, viewport.zoom, viewport.rect.width) as i32;
        let offset_y =
            scaled_camera_border(border_top, viewport.zoom, viewport.rect.height) as i32;
        let right_pixels = scaled_camera_border(border_right, viewport.zoom, viewport.rect.width);
        let bottom_pixels = scaled_camera_border(border_bottom, viewport.zoom, viewport.rect.height);
        let content_width = viewport
            .rect
            .width
            .saturating_sub(offset_x as u32)
            .saturating_sub(right_pixels)
            .max(1);
        let content_height = viewport
            .rect
            .height
            .saturating_sub(offset_y as u32)
            .saturating_sub(bottom_pixels)
            .max(1);
        viewport.target_x = view_x;
        viewport.target_y = view_y;
        viewport.content_rect = SurfaceRect::new(
            viewport.rect.x + offset_x,
            viewport.rect.y + offset_y,
            content_width,
            content_height,
        );
        viewport.viewport_x = (view_x + border_left) as f32;
        viewport.viewport_y = (view_y + border_top) as f32;
        true
    }

    /// Queue a primary FreeView displacement after the app-level physical
    /// NO_OWNER check when the live projection is absent or stale.
    pub fn queue_primary_observer_scroll(&mut self, delta: Vector2) {
        self.pending_primary_observer_scroll += delta;
    }

    /// Draw one selected C4MouseControl cursor from the resolution-selected
    /// cursor sheet. `screen` is an absolute logical output point; the
    /// native hotspot and inverse presentation scale are applied here.
    pub fn draw_mouse_cursor(
        &mut self,
        phase: MouseCursorPhase,
        screen: GuiPoint,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        self.draw_mouse_cursor_with_native_offset(phase, phase, screen, (0, 0), gamma)
    }

    fn draw_mouse_cursor_with_native_offset(
        &mut self,
        phase: MouseCursorPhase,
        hotspot_phase: MouseCursorPhase,
        screen: GuiPoint,
        native_offset: (i32, i32),
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        Self::draw_mouse_cursor_to_surface(
            &mut self.surface,
            self.cursor_atlas.as_ref(),
            self.logical_resolution_width,
            self.presentation_scale,
            self.advanced_renderer_config,
            phase,
            hotspot_phase,
            screen,
            native_offset,
            gamma,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_mouse_cursor_to_surface(
        surface: &mut Surface,
        cursor_atlas: &CursorAtlas,
        logical_resolution_width: u32,
        presentation_scale: f32,
        renderer_config: AdvancedRendererConfig,
        phase: MouseCursorPhase,
        hotspot_phase: MouseCursorPhase,
        screen: GuiPoint,
        native_offset: (i32, i32),
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some(image) = cursor_atlas.image_for_scaled_resolution(
            logical_resolution_width,
            presentation_scale,
        ) else {
            return false;
        };
        let cell = image.height() as i32;
        let source = SourceRect::new(phase.atlas_phase().saturating_mul(cell), 0, cell, cell);
        if !Self::source_within_image(&image, &source) {
            return false;
        }

        let scale = presentation_scale;
        let inverse_scale = scale.recip();
        let (offset_x, offset_y) = hotspot_phase.hotspot(cell);
        let destination = GuiRect::from_origin_size(
            GuiPoint::new(
                (screen.x * scale - offset_x as f32 + native_offset.0 as f32).trunc()
                    * inverse_scale,
                (screen.y * scale - offset_y as f32 + native_offset.1 as f32).trunc()
                    * inverse_scale,
            ),
            GuiSize::new(cell as f32 * inverse_scale, cell as f32 * inverse_scale),
        );
        draw_image_region(
            surface,
            &destination,
            &image,
            None,
            &source,
            false,
            None,
            SpriteBlitState::normal().with_renderer_config(renderer_config),
            gamma,
            None,
        );
        true
    }

    /// Draw a C4MouseControl cursor while retaining the viewport clip that
    /// encloses `C4MouseControl::Draw` in the native renderer.
    pub fn draw_mouse_cursor_clipped(
        &mut self,
        phase: MouseCursorPhase,
        viewport_clip: SurfaceRect,
        screen: GuiPoint,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let previous_clip = self.surface.clip();
        let clip = previous_clip
            .and_then(|clip| clip.intersection(viewport_clip))
            .unwrap_or_else(|| {
                if previous_clip.is_some() {
                    SurfaceRect::new(0, 0, 0, 0)
                } else {
                    viewport_clip
                }
            });
        self.surface.set_clip(clip);
        let drawn = self.draw_mouse_cursor(phase, screen, gamma);
        match previous_clip {
            Some(clip) => self.surface.set_clip(clip),
            None => self.surface.clear_clip(),
        }
        drawn
    }

    /// Draw C4GUI::CMouse: the base Region cell and, while mouse Help owns
    /// the GUI, its second Help cell at the native (+5,-5) source-pixel offset.
    pub fn draw_gui_mouse_cursor(
        &mut self,
        screen: GuiPoint,
        help: bool,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let drawn = self.draw_mouse_cursor(MouseCursorPhase::Region, screen, gamma);
        if help {
            self.draw_mouse_cursor_with_native_offset(
                MouseCursorPhase::Help,
                MouseCursorPhase::Region,
                screen,
                (5, -5),
                gamma,
            );
        }
        drawn
    }

    /// Draw C4GUI::CMouse into a caller-owned logical surface. Loading uses
    /// this path because its non-native compositor intentionally builds the
    /// loader and dialog stack in a temporary surface before presentation.
    pub fn draw_gui_mouse_cursor_to_surface(
        &self,
        surface: &mut Surface,
        screen: GuiPoint,
        help: bool,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let drawn = Self::draw_mouse_cursor_to_surface(
            surface,
            self.cursor_atlas.as_ref(),
            self.logical_resolution_width,
            self.presentation_scale,
            self.advanced_renderer_config,
            MouseCursorPhase::Region,
            MouseCursorPhase::Region,
            screen,
            (0, 0),
            gamma,
        );
        if help {
            Self::draw_mouse_cursor_to_surface(
                surface,
                self.cursor_atlas.as_ref(),
                self.logical_resolution_width,
                self.presentation_scale,
                self.advanced_renderer_config,
                MouseCursorPhase::Help,
                MouseCursorPhase::Region,
                screen,
                (5, -5),
                gamma,
            );
        }
        drawn
    }

    /// Draw the missing-image construction fallback without allowing its
    /// centered cursor cell to escape the originating split-screen viewport.
    pub fn draw_construction_cursor_fallback(
        &mut self,
        viewport_clip: SurfaceRect,
        screen: GuiPoint,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        self.draw_mouse_cursor_clipped(MouseCursorPhase::Construct, viewport_clip, screen, gamma)
    }

    /// Returns one selected cursor cell's native source-pixel hotspot.
    pub fn mouse_cursor_primary_offset(&self, phase: MouseCursorPhase) -> Option<GuiPoint> {
        let image = self
            .cursor_atlas
            .image_for_scaled_resolution(self.logical_resolution_width, self.presentation_scale)?;
        let cell = i32::try_from(image.height()).ok()?;
        let source = SourceRect::new(
            phase.atlas_phase().saturating_mul(cell),
            0,
            cell,
            cell,
        );
        if !Self::source_within_image(&image, &source) {
            return None;
        }
        let (x, y) = phase.hotspot(cell);
        Some(GuiPoint::new(x as f32, y as f32))
    }

    /// Returns the selected construction cursor cell's centered native
    /// hotspot. The offset is in source pixels and is applied before the
    /// inverse presentation transform, just like C4MouseControl's `iOffset`.
    pub fn construction_cursor_primary_offset(&self) -> Option<GuiPoint> {
        self.mouse_cursor_primary_offset(MouseCursorPhase::Construct)
    }

    /// Draws C4MouseControl's Shift add marker relative to a construction
    /// cursor or drag image. `primary_offset` is the native source-pixel
    /// `iOffset` used for the primary draw; the marker's `(8, 8)` adjustment
    /// is applied in the same physical space before inverse presentation
    /// scaling (src/C4MouseControl.cpp:366-403).
    pub fn draw_construction_add_marker(
        &mut self,
        viewport_clip: SurfaceRect,
        screen: GuiPoint,
        primary_offset: GuiPoint,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some(image) = self
            .cursor_atlas
            .image_for_scaled_resolution(self.logical_resolution_width, self.presentation_scale)
        else {
            return false;
        };
        let Ok(cell) = i32::try_from(image.height()) else {
            return false;
        };
        let source = SourceRect::new(
            MouseCursorPhase::Add.atlas_phase().saturating_mul(cell),
            0,
            cell,
            cell,
        );
        if !Self::source_within_image(&image, &source) {
            return false;
        }

        let scale = self.presentation_scale;
        let inverse_scale = scale.recip();
        let destination = GuiRect::from_origin_size(
            GuiPoint::new(
                (screen.x * scale - primary_offset.x + 8.0).trunc() * inverse_scale,
                (screen.y * scale - primary_offset.y + 8.0).trunc() * inverse_scale,
            ),
            GuiSize::new(cell as f32 * inverse_scale, cell as f32 * inverse_scale),
        );
        let previous_clip = self.surface.clip();
        let clip = previous_clip
            .and_then(|clip| clip.intersection(viewport_clip))
            .unwrap_or_else(|| {
                if previous_clip.is_some() {
                    SurfaceRect::new(0, 0, 0, 0)
                } else {
                    viewport_clip
                }
            });
        self.surface.set_clip(clip);
        draw_image_region(
            &mut self.surface,
            &destination,
            &image,
            None,
            &source,
            false,
            None,
            SpriteBlitState::normal().with_renderer_config(self.advanced_renderer_config),
            gamma,
            None,
        );
        match previous_clip {
            Some(clip) => self.surface.set_clip(clip),
            None => self.surface.clear_clip(),
        }
        true
    }

    /// Draws C4MouseControl's construction drag image at its native logical
    /// size. `bottom_center` is the cursor hotspot; `valid` selects the native
    /// green or red MOD2 modulation (src/C4MouseControl.cpp:379-385).
    pub fn draw_construction_drag_preview(
        &mut self,
        image: &ImageData,
        viewport_clip: SurfaceRect,
        bottom_center: GuiPoint,
        valid: bool,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Ok(width) = i32::try_from(image.width()) else {
            return false;
        };
        let Ok(height) = i32::try_from(image.height()) else {
            return false;
        };
        let source = SourceRect::new(0, 0, width, height);
        if !Self::source_within_image(image, &source) {
            return false;
        }

        // C4MouseControl uses integer Wdt/2 and the full image height for the
        // construction cursor hotspot.
        let destination = GuiRect::from_origin_size(
            GuiPoint::new(
                bottom_center.x - (width / 2) as f32,
                bottom_center.y - height as f32,
            ),
            GuiSize::new(width as f32, height as f32),
        );
        let previous_clip = self.surface.clip();
        let clip = previous_clip
            .and_then(|clip| clip.intersection(viewport_clip))
            .unwrap_or_else(|| {
                if previous_clip.is_some() {
                    SurfaceRect::new(0, 0, 0, 0)
                } else {
                    viewport_clip
                }
            });
        self.surface.set_clip(clip);
        draw_image_region(
            &mut self.surface,
            &destination,
            image,
            None,
            &source,
            false,
            None,
            SpriteBlitState {
                mode: self
                    .advanced_renderer_config
                    .masked_blit_mode(C4GFXBLIT_MOD2),
                modulation: Some(if valid {
                    CONSTRUCTION_DRAG_VALID_MODULATION
                } else {
                    CONSTRUCTION_DRAG_INVALID_MODULATION
                }),
                fog_modulation: None,
                renderer_config: self.advanced_renderer_config,
            },
            gamma,
            None,
        );
        match previous_clip {
            Some(clip) => self.surface.set_clip(clip),
            None => self.surface.clear_clip(),
        }
        true
    }

    pub fn world_to_screen(&self, owner: i32, position: Vector2) -> Option<(f32, f32)> {
        self.active_viewports
            .iter()
            .find(|viewport| viewport.owner == owner)
            .map(|viewport| {
                let screen_x = (position.x as f32 - viewport.viewport_x) * viewport.zoom
                    + viewport.content_rect.x as f32;
                let screen_y = (position.y as f32 - viewport.viewport_y) * viewport.zoom
                    + viewport.content_rect.y as f32;
                (screen_x, screen_y)
            })
    }

    /// Draw C4MouseControl's landscape-selection rectangle over the owning
    /// viewport. `down_world` stays fixed in world space while the camera
    /// scrolls; `current_screen` is the live clamped viewport cursor
    /// (src/C4MouseControl.cpp:203-316,406-414).
    pub fn draw_mouse_selection_frame(
        &mut self,
        owner: i32,
        down_world: Vector2,
        current_screen: GuiPoint,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some(viewport) = self
            .active_viewports
            .iter()
            .find(|viewport| viewport.owner == owner)
            .cloned()
        else {
            return false;
        };
        if viewport.rect.width == 0 || viewport.rect.height == 0 {
            return false;
        }

        let down = (
            ((down_world.x as f32 - viewport.viewport_x) * viewport.zoom
                + viewport.content_rect.x as f32)
                .round() as i32,
            ((down_world.y as f32 - viewport.viewport_y) * viewport.zoom
                + viewport.content_rect.y as f32)
                .round() as i32,
        );
        let right = viewport.rect.x + viewport.rect.width as i32 - 1;
        let bottom = viewport.rect.y + viewport.rect.height as i32 - 1;
        let current = (
            (current_screen.x.round() as i32).clamp(viewport.rect.x, right),
            (current_screen.y.round() as i32).clamp(viewport.rect.y, bottom),
        );
        draw_mouse_selection_frame_raster(
            &mut self.surface,
            viewport.rect,
            current,
            down,
            self.game_palette.color(10),
            gamma,
        );
        true
    }

    /// Draw the transient `C4MouseControl::Selection` marks. These are a
    /// mouse-local presentation list and deliberately bypass the player's
    /// `SelectFlash` timer (src/C4MouseControl.cpp:317-327;
    /// src/C4ObjectList.cpp:698-703).
    pub fn draw_mouse_selection_marks(
        &mut self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        selection: &[ObjectId],
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some(viewport) = self
            .active_viewports
            .iter()
            .find(|viewport| viewport.owner == owner)
            .cloned()
        else {
            return false;
        };
        let Some(image) = self.hud_graphics.select_mark.clone() else {
            return false;
        };
        if viewport.rect.width == 0 || viewport.rect.height == 0 {
            return false;
        }

        let previous_clip = self.surface.clip();
        let clip = previous_clip
            .and_then(|clip| clip.intersection(viewport.rect))
            .unwrap_or_else(|| {
                if previous_clip.is_some() {
                    SurfaceRect::new(0, 0, 0, 0)
                } else {
                    viewport.rect
                }
            });
        self.surface.set_clip(clip);

        let cell = image.height() as i32;
        let right = viewport.rect.x + viewport.rect.width as i32 - 1;
        let bottom = viewport.rect.y + viewport.rect.height as i32 - 1;
        for id in selection {
            let Some(object) = snapshot
                .object(*id)
                .filter(|object| object.status.is_active())
            else {
                continue;
            };
            let screen_x = (object.position.x as f32 - viewport.viewport_x) * viewport.zoom
                + viewport.content_rect.x as f32;
            let screen_y = (object.position.y as f32 - viewport.viewport_y) * viewport.zoom
                + viewport.content_rect.y as f32;
            if screen_x < viewport.rect.x as f32
                || screen_x > right as f32
                || screen_y < viewport.rect.y as f32
                || screen_y > bottom as f32
            {
                continue;
            }

            let shape = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .map(Self::sprite_def_shape)
                .filter(|shape| shape.width > 0 && shape.height > 0)
                .unwrap_or_else(|| DefinitionRect::new(-6, -6, 12, 12));
            let cox = screen_x + shape.x as f32 * viewport.zoom - 2.0;
            let coy = screen_y + shape.y as f32 * viewport.zoom - 2.0;
            let shape_width = shape.width as f32 * viewport.zoom;
            let shape_height = shape.height as f32 * viewport.zoom;
            for (px, py, phase) in [
                (cox, coy, 0),
                (cox + shape_width, coy, 1),
                (cox, coy + shape_height, 2),
                (cox + shape_width, coy + shape_height, 3),
            ] {
                let source = SourceRect::new(phase * cell, 0, cell, cell);
                if !Self::source_within_image(&image, &source) {
                    continue;
                }
                draw_image_region(
                    &mut self.surface,
                    &GuiRect::from_origin_size(
                        GuiPoint::new(px, py),
                        GuiSize::new(cell as f32, cell as f32),
                    ),
                    &image,
                    None,
                    &source,
                    false,
                    None,
                    SpriteBlitState::normal().with_renderer_config(self.advanced_renderer_config),
                    gamma,
                    None,
                );
            }
        }

        match previous_clip {
            Some(clip) => self.surface.set_clip(clip),
            None => self.surface.clear_clip(),
        }
        true
    }

    pub fn viewport_point_at(&self, point: GuiPoint) -> Option<ViewportPointer> {
        let viewport = self.viewport_for_point(point)?;
        Some(Self::pointer_for_viewport(viewport, point))
    }

    /// C4MouseControl owns the viewport's whole output rectangle, including
    /// HUD/command regions outside a letterboxed world content rectangle.
    /// World coordinates continue from ViewX/ViewY through those border
    /// pixels just like `X = ViewX + VpX` (C4MouseControl.cpp:207-269).
    pub fn viewport_output_point_at(&self, point: GuiPoint) -> Option<ViewportPointer> {
        let viewport = self.active_viewports.iter().rev().find(|viewport| {
            let rect = viewport.rect;
            let left = rect.x as f32;
            let top = rect.y as f32;
            let right = left + rect.width as f32;
            let bottom = top + rect.height as f32;
            point.x >= left && point.x < right && point.y >= top && point.y < bottom
        })?;
        Some(Self::pointer_for_viewport(viewport, point))
    }

    /// Projects a physical point through the requested owner's first viewport,
    /// clamping to its inclusive output bounds like C4MouseControl.
    pub fn viewport_output_point_for_owner(
        &self,
        owner: i32,
        point: GuiPoint,
    ) -> Option<ViewportPointer> {
        let index = self
            .active_viewports
            .iter()
            .position(|viewport| viewport.owner == owner)?;
        self.viewport_output_point_for_index(index, point)
    }

    /// Projects a physical point through one exact active viewport record.
    /// This preserves physical viewport identity when owners repeat or a
    /// no-owner viewport is temporarily assigned to a film-view player.
    pub fn viewport_output_point_for_index(
        &self,
        index: usize,
        point: GuiPoint,
    ) -> Option<ViewportPointer> {
        let viewport = self.active_viewports.get(index)?;
        let rect = viewport.rect;
        if rect.width == 0 || rect.height == 0 {
            return None;
        }
        let right = rect.x.saturating_add(
            i32::try_from(rect.width.saturating_sub(1)).unwrap_or(i32::MAX),
        );
        let bottom = rect.y.saturating_add(
            i32::try_from(rect.height.saturating_sub(1)).unwrap_or(i32::MAX),
        );
        let point = GuiPoint::new(
            point.x.clamp(rect.x as f32, right as f32),
            point.y.clamp(rect.y as f32, bottom as f32),
        );
        Some(Self::pointer_for_viewport(viewport, point))
    }

    fn pointer_for_viewport(viewport: &ActiveViewport, point: GuiPoint) -> ViewportPointer {
        let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);
        let base_x = viewport.content_rect.x as f32;
        let base_y = viewport.content_rect.y as f32;
        let world_x = (point.x - base_x) / zoom + viewport.viewport_x;
        let world_y = (point.y - base_y) / zoom + viewport.viewport_y;
        ViewportPointer {
            owner: viewport.owner,
            world: FloatVector2::new(world_x, world_y),
            screen: point,
        }
    }

    pub fn crew_at_point(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
    ) -> Option<ObjectId> {
        let viewport = self.viewport_for_point(point)?;
        if viewport.owner != owner {
            return None;
        }

        let mut best: Option<(ObjectId, f32)> = None;
        for object in &snapshot.objects {
            if object.owner != owner
                || !object.crew_member
                || !object.status.is_active()
                || !object.alive
                || !Self::object_is_visible(
                    &snapshot.objects,
                    &snapshot.players,
                    object,
                    owner,
                    false,
                )
            {
                continue;
            }
            if let Some(rect) = self.object_screen_rect_for_viewport(object, viewport) {
                if rect_contains(rect, point, PICK_TOLERANCE) {
                    let center_x = rect.x as f32 + rect.width as f32 * 0.5;
                    let center_y = rect.y as f32 + rect.height as f32 * 0.5;
                    let dx = point.x - center_x;
                    let dy = point.y - center_y;
                    let distance_sq = dx * dx + dy * dy;
                    match best {
                        Some((_, best_dist)) if distance_sq >= best_dist => {}
                        _ => best = Some((object.id, distance_sq)),
                    }
                }
            }
        }
        best.map(|(id, _)| id)
    }

    /// Returns the frontmost world object under a viewport pointer using the
    /// same front-to-back order as `C4Game::FindVisObject`: C++ searches
    /// `Objects.First -> Next`, while drawing uses `Last -> Prev`
    /// (`C4Game.cpp:1426-1492`; `C4ObjectList.cpp:387-396`).
    pub fn object_at_point(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
    ) -> Option<ObjectId> {
        self.object_at_point_with_ocf(snapshot, owner, point, u32::MAX)
    }

    /// Returns the frontmost world object other than `excluded`, matching
    /// `C4Game::FindVisObject`'s single `pExclude` pointer.
    pub fn object_at_point_excluding(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
        excluded: ObjectId,
    ) -> Option<ObjectId> {
        self.object_at_point_with_ocf_excluding(
            snapshot,
            owner,
            point,
            u32::MAX,
            Some(excluded),
        )
    }

    /// The OCF-filtered form of [`Self::object_at_point`], matching the mask
    /// passed to `C4Game::FindVisObject` by `C4MouseControl::GetTargetObject`.
    /// A nonmatching front object does not hide a matching object behind it
    /// (C4Game.cpp:1426-1492; C4MouseControl.cpp:1318-1325).
    pub fn object_at_point_with_ocf(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
        ocf: u32,
    ) -> Option<ObjectId> {
        self.object_at_point_with_ocf_excluding(snapshot, owner, point, ocf, None)
    }

    fn object_at_point_with_ocf_excluding(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
        ocf: u32,
        excluded: Option<ObjectId>,
    ) -> Option<ObjectId> {
        let viewport = self.viewport_for_point(point)?;
        if viewport.owner != owner {
            return None;
        }

        // Reconstruct the renderer's effective back-to-front list first.
        // A partial sidecar is legal and draw_objects appends omitted objects
        // canonically, so those omitted objects are the frontmost group.
        let mut back_to_front = Vec::with_capacity(snapshot.objects.len());
        let mut seen = HashSet::with_capacity(snapshot.objects.len());
        if !snapshot.render_order.is_empty() {
            for id in &snapshot.render_order {
                if seen.insert(*id) {
                    if let Some(object) = snapshot.object(*id) {
                        back_to_front.push(object);
                    }
                }
            }
        }
        back_to_front.extend(
            snapshot
                .objects
                .iter()
                .filter(|object| seen.insert(object.id)),
        );
        // A valid C++ player with no cursor cannot see a target through this
        // search: FindVisObject rejects every candidate before the shape
        // check, so right-up falls through to select-next.
        let player_cursor = snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .map(|player| player.cursor);
        let cursor_object = match player_cursor {
            Some(Some(cursor)) => Some(snapshot.object(cursor)?),
            Some(None) => return None,
            None => snapshot
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor)
                .and_then(|cursor| snapshot.object(cursor)),
        };
        let cursor_layer = cursor_object.map(|cursor| cursor.layer);

        back_to_front.into_iter().rev().find_map(|object| {
            if excluded == Some(object.id)
                || object.status != ObjectStatus::Normal
                || object.container.is_some()
                || object.ocf & ocf == 0
                || object.category & CATEGORY_MOUSE_IGNORE_FLAG != 0
                || cursor_layer.is_some_and(|layer| object.layer != layer)
                || !Self::object_is_visible(
                    &snapshot.objects,
                    &snapshot.players,
                    object,
                    owner,
                    false,
                )
            {
                return None;
            }
            self.object_pick_rect_for_viewport(object, viewport)
                .filter(|rect| rect_contains(*rect, point, 0.0))
                .map(|_| object.id)
        })
    }

    fn viewport_for_point(&self, point: GuiPoint) -> Option<&ActiveViewport> {
        self.active_viewports.iter().rev().find(|viewport| {
            let rect = viewport.content_rect;
            let left = rect.x as f32;
            let top = rect.y as f32;
            let right = left + rect.width as f32;
            let bottom = top + rect.height as f32;
            point.x >= left && point.x < right && point.y >= top && point.y < bottom
        })
    }

    /// Applies the renderer-owned Graphics configuration that is independent
    /// of the per-frame simulation overlay.
    pub fn set_renderer_config(
        &mut self,
        show_player_hud_always: bool,
        splitscreen_dividers: bool,
        fire_particles: bool,
    ) {
        self.show_player_hud_always = show_player_hud_always;
        self.fire_particles = fire_particles;
        if self.splitscreen_dividers != splitscreen_dividers {
            self.splitscreen_dividers = splitscreen_dividers;
            self.relayout_active_viewports();
        }
    }

    /// Whether the optional extended Fire/Fire2 presentation pass is enabled.
    pub fn fire_particles_enabled(&self) -> bool {
        self.fire_particles
    }

    /// Stores the HUD state drawn by [`Self::render_frame`] — the Rust
    /// counterpart of the per-frame data reads in `C4Viewport::DrawOverlay`
    /// (src/C4Viewport.cpp:835-882) and `C4UpperBoard::Execute`
    /// (src/C4UpperBoard.cpp:37-44).
    pub fn update_overlay(&mut self, overlay: &GraphicsOverlay<'_>) {
        self.hud_players = overlay.players.clone();
        self.crew_name_labels = overlay.crew_name_labels.clone();
        self.viewport_overlays_visible = overlay.viewport_overlays_visible;
        self.game_time_seconds = overlay.game_time_seconds;
        self.message_board = overlay.message_board.clone();
        self.clock_text = overlay.clock_text.clone();
        self.frames_per_second = overlay.frames_per_second;
        self.set_upper_board_mode(overlay.upper_board_mode, overlay.game_time_seconds);
        self.show_portraits = overlay.show_portraits;
        self.show_commands = overlay.show_commands;
        self.show_command_keys = overlay.show_command_keys;
        self.debug_hud_text = overlay
            .debug_hud
            .then(|| (overlay.frame_text.to_string(), overlay.status_text.to_string()));
    }

    /// Installs the CStdFont-faithful HUD fonts (FontRegular et al).
    pub fn set_clonk_fonts(&mut self, fonts: Option<Arc<ClonkFontSet>>) {
        self.clonk_fonts = fonts;
        if self.upper_board_text_width.is_some() {
            self.initialize_upper_board_text_width();
        }
    }

    /// Reinitializes fullscreen upper-board geometry immediately, matching
    /// `Game.InitFullscreenComponents(true)` in Display:UpperBoard.
    pub fn set_upper_board_mode(&mut self, mode: hud::UpperBoardMode, game_time_seconds: u64) {
        self.game_time_seconds = game_time_seconds;
        let mode_changed = self.upper_board_mode != mode;
        if mode_changed || self.upper_board_text_width.is_none() {
            self.upper_board_mode = mode;
            self.initialize_upper_board_text_width();
            if mode_changed {
                self.relayout_active_viewports();
            }
        }
    }

    /// Breaks a newly appended message using the current initialized
    /// `C4MessageBoard::LogBuffer` width. The returned strings are physical
    /// history lines and must not be reflowed after a later mode change.
    pub fn prepare_message_board_lines(&self, line: &str) -> Vec<String> {
        let font = self.hud_font();
        let width = hud::message_board_available_width_for_text_width(
            self.surface_width as i32,
            self.upper_board_mode,
            self.initialized_upper_board_text_width(),
        );
        hud::message_board_physical_lines(&font, line, width)
    }

    pub fn upper_board_text_strip_width(&self) -> i32 {
        hud::upper_board_text_strip_width_for_text_width(
            self.initialized_upper_board_text_width(),
        )
    }

    fn initialize_upper_board_text_width(&mut self) {
        let width = self
            .hud_font()
            .text_width(&hud::format_game_time(self.game_time_seconds));
        self.upper_board_text_width = Some(width.max(0));
    }

    fn initialized_upper_board_text_width(&self) -> i32 {
        self.upper_board_text_width.unwrap_or_else(|| {
            self.hud_font()
                .text_width(&hud::format_game_time(self.game_time_seconds))
                .max(0)
        })
    }

    fn relayout_active_viewports(&mut self) {
        let rects = self
            .layout_viewports(self.active_viewports.len())
            .into_iter()
            .zip(&self.active_viewports)
            .map(|(rect, viewport)| {
                Self::centered_viewport_rect_for_world(
                    rect,
                    viewport.world_width,
                    viewport.world_height,
                )
            })
            .collect::<Vec<_>>();

        for (viewport, rect) in self.active_viewports.iter_mut().zip(rects) {
            let logical_width = ((rect.width as f32 / viewport.zoom).ceil() as i32).max(1);
            let logical_height = ((rect.height as f32 / viewport.zoom).ceil() as i32).max(1);
            let shifted_x = viewport
                .target_x
                .saturating_add((viewport.logical_width - logical_width) / 2);
            let shifted_y = viewport
                .target_y
                .saturating_add((viewport.logical_height - logical_height) / 2);
            viewport.logical_width = logical_width;
            viewport.logical_height = logical_height;
            if let Some(state) = self.camera_states.get_mut(&viewport.camera_key) {
                if viewport.is_no_owner_viewport {
                    (viewport.target_x, viewport.target_y) = state.no_owner_position(
                        logical_width,
                        logical_height,
                        viewport.world_width,
                        viewport.world_height,
                    );
                } else {
                    state.resize_output(logical_width, logical_height);
                    viewport.target_x = shifted_x;
                    viewport.target_y = shifted_y;
                }
            } else if viewport.is_no_owner_viewport {
                viewport.target_x = if viewport.world_width < logical_width {
                    (viewport.world_width - logical_width) / 2
                } else {
                    shifted_x.clamp(0, viewport.world_width - logical_width)
                };
                viewport.target_y = if viewport.world_height < logical_height {
                    (viewport.world_height - logical_height) / 2
                } else {
                    shifted_y.clamp(0, viewport.world_height - logical_height)
                };
            } else {
                viewport.target_x = shifted_x;
                viewport.target_y = shifted_y;
            }

            let border_left = (-viewport.target_x).max(0).min(logical_width);
            let border_top = (-viewport.target_y).max(0).min(logical_height);
            let border_right = (logical_width - viewport.world_width + viewport.target_x)
                .max(0)
                .min(logical_width - border_left);
            let border_bottom = (logical_height - viewport.world_height + viewport.target_y)
                .max(0)
                .min(logical_height - border_top);
            let offset_x = scaled_camera_border(border_left, viewport.zoom, rect.width) as i32;
            let offset_y = scaled_camera_border(border_top, viewport.zoom, rect.height) as i32;
            let right_pixels = scaled_camera_border(border_right, viewport.zoom, rect.width);
            let bottom_pixels = scaled_camera_border(border_bottom, viewport.zoom, rect.height);
            let content_width = rect
                .width
                .saturating_sub(offset_x as u32)
                .saturating_sub(right_pixels)
                .max(1);
            let content_height = rect
                .height
                .saturating_sub(offset_y as u32)
                .saturating_sub(bottom_pixels)
                .max(1);

            viewport.rect = rect;
            viewport.content_rect = SurfaceRect::new(
                rect.x + offset_x,
                rect.y + offset_y,
                content_width,
                content_height,
            );
            viewport.viewport_x = (viewport.target_x + border_left) as f32;
            viewport.viewport_y = (viewport.target_y + border_top) as f32;
        }
    }

    /// Installs the current scenario controls immediately, matching the
    /// explicit `ApplyGamma` at the end of `C4Game::Init` (C4Game.cpp:490).
    /// Runtime `SetGamma` changes continue through [`Self::render_frame`]'s
    /// draw-then-apply lifecycle.
    pub fn apply_gamma_now(&mut self, gamma: &GammaControlState) {
        self.active_gamma_control_points = Some(gamma.combined_control_points());
    }

    /// Returns the gamma ramp installed while the current frame is drawn.
    /// Callers that append GUI after [`Self::render_frame`] must capture this
    /// before rendering, because `render_frame` latches `pending` for the next
    /// pass at its tail just like `C4GraphicsSystem::Execute`
    /// (`src/C4GraphicsSystem.cpp:167-199`).
    pub fn active_gamma_ramp(&self, pending: &GammaControlState) -> clonk_graphics::GammaRamp {
        let configured = clonk_graphics::GammaRamp::from_control_points(
            self.active_gamma_control_points
                .unwrap_or_else(|| pending.combined_control_points()),
        );
        if self.advanced_renderer_config.disable_gamma {
            return clonk_graphics::GammaRamp::identity();
        }
        configured
    }

    pub fn render_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
    ) -> Vec<EngineSurfaceSnapshot> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let pending = self.render_frame_base(snapshot, viewports);
        self.render_frame_foreground(&pending);
        let pending = self.render_frame_hud_players(pending);
        self.render_frame_hud_chrome(pending)
    }

    /// Render a complete frame without materializing the diagnostic sprite
    /// atlas. Production callers that present [`Self::surface`] directly do
    /// not need the per-surface snapshots returned by [`Self::render_frame`].
    pub fn render_frame_without_atlas(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
    ) {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let pending = self.render_frame_base(snapshot, viewports);
        self.render_frame_foreground(&pending);
        let pending = self.render_frame_hud_players(pending);
        self.render_frame_hud_chrome_without_atlas(pending);
    }

    /// Render all renderer-owned layers while leaving a monitor-style gamma
    /// curve for the caller to apply after its later GUI layers. This is the
    /// in-process counterpart of C++ composing the framebuffer before the OS
    /// monitor ramp becomes visible.
    pub fn render_frame_without_atlas_deferred_monitor_gamma(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
    ) {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let pending = self.render_frame_base(snapshot, viewports);
        self.render_frame_foreground(&pending);
        let pending = self.render_frame_hud_players(pending);
        self.render_frame_hud_chrome_without_atlas_deferred_monitor_gamma(pending);
    }

    /// Render the back buffer and viewports through world cursor labels. In
    /// native-capture mode, foreground-parallax objects are retained for the
    /// next ordered layer so they can still occlude those labels.
    pub fn render_frame_base<'snapshot>(
        &mut self,
        snapshot: &'snapshot SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
    ) -> PendingHudFrame<'snapshot> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let pending = snapshot.environment.gamma.combined_control_points();
        // C4Game::Init applies the initialization controls before the first
        // render (C4Game.cpp:490). Later SetGamma calls set fSetGamma during
        // simulation and C4GraphicsSystem::Execute applies them only after it
        // has drawn the current pass (C4GraphicsSystem.cpp:195-199).
        let gamma = self.active_gamma_ramp(&snapshot.environment.gamma);
        let fragment_gamma = self.advanced_renderer_config.uses_fragment_gamma();
        let monitor_gamma = self.advanced_renderer_config.uses_monitor_gamma();
        self.render_frame_base_with_gamma(snapshot, viewports, fragment_gamma.then_some(&gamma));
        self.render_phase_generation = self.render_phase_generation.wrapping_add(1);
        PendingHudFrame {
            snapshot,
            frame: snapshot.frame,
            gamma,
            fragment_gamma,
            monitor_gamma,
            pending_gamma_control_points: pending,
            graphics_system_identity: Arc::clone(&self.render_phase_identity),
            generation: self.render_phase_generation,
        }
    }

    fn assert_pending_frame(&self, pending: &PendingHudFrame<'_>) {
        assert!(
            Arc::ptr_eq(
                &self.render_phase_identity,
                &pending.graphics_system_identity
            ),
            "pending frame belongs to a different graphics system"
        );
        assert_eq!(
            self.render_phase_generation, pending.generation,
            "pending frame was superseded by a newer base phase"
        );
    }

    /// Draw foreground-parallax objects retained by [`Self::render_frame_base`]
    /// onto the caller's current (normally transparent) logical layer.
    pub fn render_frame_foreground(&mut self, pending: &PendingHudFrame<'_>) {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        self.assert_pending_frame(pending);
        self.draw_pending_viewport_foregrounds();
    }

    /// Draw per-viewport HUD controls, leaving the later fullscreen boards for
    /// a separate ordered layer.
    pub fn render_frame_hud_players<'snapshot>(
        &mut self,
        pending: PendingHudFrame<'snapshot>,
    ) -> PendingHudChromeFrame<'snapshot> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        self.assert_pending_frame(&pending);
        assert!(
            self.pending_viewport_foregrounds.is_empty(),
            "foreground phase must be rendered before HUD players"
        );
        self.draw_hud_players(pending.frame, pending.draw_gamma());
        PendingHudChromeFrame(pending)
    }

    /// Draw message/upper-board chrome and complete the frame's gamma/atlas
    /// lifecycle.
    pub fn render_frame_hud_chrome(
        &mut self,
        pending: PendingHudChromeFrame<'_>,
    ) -> Vec<EngineSurfaceSnapshot> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let pending = pending.0;
        self.assert_pending_frame(&pending);
        self.draw_hud_chrome(pending.draw_gamma());
        let snapshots = self.collect_sprite_atlas(pending.snapshot);
        if pending.monitor_gamma {
            pending.gamma.apply_to_surface(&mut self.surface);
        }
        self.active_gamma_control_points = Some(pending.pending_gamma_control_points);
        snapshots
    }

    /// Complete the ordered HUD chrome and gamma lifecycle without
    /// materializing the diagnostic sprite atlas.
    pub fn render_frame_hud_chrome_without_atlas(
        &mut self,
        pending: PendingHudChromeFrame<'_>,
    ) {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let pending = pending.0;
        self.assert_pending_frame(&pending);
        self.draw_hud_chrome(pending.draw_gamma());
        if pending.monitor_gamma {
            pending.gamma.apply_to_surface(&mut self.surface);
        }
        self.active_gamma_control_points = Some(pending.pending_gamma_control_points);
    }

    /// Complete renderer chrome without applying a monitor-style postpass;
    /// the application still has GUI layers to composite onto this frame.
    pub fn render_frame_hud_chrome_without_atlas_deferred_monitor_gamma(
        &mut self,
        pending: PendingHudChromeFrame<'_>,
    ) {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let pending = pending.0;
        self.assert_pending_frame(&pending);
        self.draw_hud_chrome(pending.draw_gamma());
        self.active_gamma_control_points = Some(pending.pending_gamma_control_points);
    }

    /// Render the complete landscape through the first active viewport's
    /// world pass, matching `C4GraphicsSystem::DoSaveScreenshot(true)`.
    /// Borders and HUD/menu overlays are deliberately omitted; cursor marks
    /// and parallax foreground objects remain part of `C4Viewport::Draw`.
    pub fn render_full_landscape(&mut self, snapshot: &SimulationSnapshot) -> Option<Surface> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let gamma = self.active_gamma_ramp(&snapshot.environment.gamma);
        self.render_full_landscape_with_gamma(snapshot, &gamma)
    }

    /// Full-landscape capture with the gamma ramp installed when the request
    /// was made. Queued screenshots must not observe a later SetGamma latch.
    pub fn render_full_landscape_with_gamma(
        &mut self,
        snapshot: &SimulationSnapshot,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Option<Surface> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        let landscape = snapshot.landscape.as_ref()?;
        let active = self.active_viewports.first()?.clone();
        let world_width = i32::try_from(landscape.width()).ok()?.max(1);
        let world_height = landscape.estimated_height().max(1);

        // Native full-map capture temporarily clears sky parallax and retargets
        // the first physical viewport to NO_OWNER. Clone the COW snapshot so
        // the live simulation state is not mutated; like C++, the extra draw
        // may still advance presentation-only randomness and caches.
        let mut capture = snapshot.clone();
        if let Some(sky) = capture.sky.as_mut() {
            sky.settings.parallax_x = 10;
            sky.settings.parallax_y = 10;
        }
        let focus = active.focus.and_then(|focus| capture.object(focus));
        let camera_key = CameraKey::Player {
            owner: OWNER_NONE,
            slot: usize::MAX,
        };
        let input = ViewportInput {
            owner: OWNER_NONE,
            center: Vector2::new(world_width / 2, world_height / 2),
            offset: Vector2::ZERO,
            zoom: 1.0,
            focus,
            camera_identity: Some(camera_key),
            // C++ changes Player on the existing viewport without changing
            // its fIsNoOwnerViewport classification.
            is_no_owner_viewport: active.is_no_owner_viewport,
            scrolling: false,
        };
        let owner_colors = Self::collect_owner_colors(&capture);

        let saved_surface = std::mem::replace(
            &mut self.surface,
            Surface::new(
                world_width as u32,
                world_height as u32,
                PixelFormat::Rgba8888,
            ),
        );
        let saved_surface_width = self.surface_width;
        let saved_surface_height = self.surface_height;
        let saved_world_width = self.world_width;
        let saved_world_height = self.world_height;
        let saved_viewports = std::mem::take(&mut self.active_viewports);
        let saved_camera = self.camera_states.remove(&camera_key);
        let saved_fog_map = self.active_fog_map.take();
        let saved_fog_suppression_depth = self.fog_suppression_depth;
        let fragment_gamma = self
            .advanced_renderer_config
            .uses_fragment_gamma()
            .then_some(gamma);

        self.surface_width = world_width as u32;
        self.surface_height = world_height as u32;
        self.world_width = world_width;
        self.world_height = world_height;
        self.render_viewport(
            &capture,
            &input,
            usize::MAX,
            SurfaceRect::new(0, 0, world_width as u32, world_height as u32),
            &owner_colors,
            fragment_gamma,
        );
        let mut rendered = std::mem::replace(&mut self.surface, saved_surface);
        if self.advanced_renderer_config.uses_monitor_gamma() {
            gamma.apply_to_surface(&mut rendered);
        }

        self.surface_width = saved_surface_width;
        self.surface_height = saved_surface_height;
        self.world_width = saved_world_width;
        self.world_height = saved_world_height;
        self.active_viewports = saved_viewports;
        self.camera_states.remove(&camera_key);
        if let Some(camera) = saved_camera {
            self.camera_states.insert(camera_key, camera);
        }
        self.active_fog_map = saved_fog_map;
        self.fog_suppression_depth = saved_fog_suppression_depth;

        Some(rendered)
    }

    /// Compatibility completion for callers that do not need ordered seams.
    pub fn render_frame_hud(&mut self, pending: PendingHudFrame<'_>) -> Vec<EngineSurfaceSnapshot> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        self.render_frame_foreground(&pending);
        let pending = self.render_frame_hud_players(pending);
        self.render_frame_hud_chrome(pending)
    }

    /// Internal seam for C++ per-fragment gamma rendering and exact isolated
    /// fragment tests. Public rendering drives its active/pending lifecycle.
    pub(crate) fn render_frame_with_gamma(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Vec<EngineSurfaceSnapshot> {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        self.render_frame_base_with_gamma(snapshot, viewports, gamma);
        self.draw_pending_viewport_foregrounds();
        self.draw_hud_players(snapshot.frame, gamma);
        self.draw_hud_chrome(gamma);
        self.collect_sprite_atlas(snapshot)
    }

    fn render_frame_base_with_gamma(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.active_viewports.clear();
        self.rendered_object_audibility_calls.clear();
        self.pending_viewport_foregrounds.clear();
        let background = self.hud_graphics.background.clone();
        self.tiled_underlay_cache
            .begin_frame(&self.surface, background.as_ref(), gamma);
        if let Some(background) = background.as_ref() {
            self.tiled_underlay_cache
                .draw(&mut self.surface, background, 0, 0, gamma);
        } else {
            self.surface.fill(Color::opaque(8, 12, 24));
        }

        let owner_colors = Self::collect_owner_colors(snapshot);
        self.render_viewports(snapshot, viewports, &owner_colors, gamma);
    }

    fn draw_pending_viewport_foregrounds(&mut self) {
        for mut pending in self.pending_viewport_foregrounds.drain(..) {
            blit_surface(
                &mut self.surface,
                &pending.surface,
                pending.destination.x,
                pending.destination.y,
            );
            let _ = self
                .surface
                .extend_clonk_text_capture_from(&mut pending.surface, pending.destination);
        }
    }

    fn render_viewports(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
        owner_colors: &HashMap<i32, Color>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if viewports.is_empty() {
            if let Some(object) = snapshot.objects.first() {
                let default = ViewportInput::from_focus(object);
                self.render_viewport(
                    snapshot,
                    &default,
                    0,
                    SurfaceRect::new(0, 0, self.surface_width, self.surface_height),
                    owner_colors,
                    gamma,
                );
            }
            return;
        }

        let layout = self.layout_viewports(viewports.len());
        let mut owner_slots = HashMap::<i32, usize>::new();
        for (input, rect) in viewports.iter().zip(layout.into_iter()) {
            let slot = owner_slots.entry(input.owner).or_default();
            let camera_slot = *slot;
            *slot += 1;
            self.render_viewport(snapshot, input, camera_slot, rect, owner_colors, gamma);
        }
    }

    fn render_viewport(
        &mut self,
        snapshot: &SimulationSnapshot,
        input: &ViewportInput<'_>,
        camera_slot: usize,
        rect: SurfaceRect,
        owner_colors: &HashMap<i32, Color>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }

        let saved_surface_width = self.surface_width;
        let saved_surface_height = self.surface_height;
        let saved_viewport_x = self.viewport_x;
        let saved_viewport_y = self.viewport_y;
        let saved_viewport_zoom = self.viewport_zoom;
        let saved_world_width = self.world_width;
        let saved_world_height = self.world_height;
        let saved_content_audibility_facet = self.content_audibility_facet;
        let saved_full_audibility_facet = self.full_audibility_facet;
        let saved_current_audibility_facet = self.current_audibility_facet;

        self.surface_width = rect.width;
        self.surface_height = rect.height;
        self.update_world_dimensions(snapshot.landscape.as_ref());

        let rect = self.centered_viewport_rect(rect);
        let format = self.surface.format();
        self.surface_width = rect.width;
        self.surface_height = rect.height;

        let zoom = input.zoom.clamp(MIN_VIEWPORT_ZOOM, MAX_VIEWPORT_ZOOM);
        let world_width = self.world_width.max(1);
        let world_height = self.world_height.max(1);
        // C4Application::SetResolution converts physical output to the
        // logical viewport with ceilf(physical/scale) before SetOutputSize.
        let view_width = ((rect.width as f32 / zoom).ceil() as i32).max(1);
        let view_height = ((rect.height as f32 / zoom).ceil() as i32).max(1);

        let key = CameraKey::Player {
            owner: input.owner,
            slot: camera_slot,
        };
        let key = input.camera_identity.unwrap_or(key);
        let pending_observer_scroll = if self.active_viewports.is_empty() {
            let pending = std::mem::take(&mut self.pending_primary_observer_scroll);
            if input.is_no_owner_viewport {
                pending
            } else {
                Vector2::ZERO
            }
        } else {
            Vector2::ZERO
        };

        let state = self.camera_states.entry(key).or_insert_with(|| {
            CameraState::new(world_width, world_height, view_width, view_height)
        });
        let (mut view_x, mut view_y) = if input.owner == OWNER_NONE {
            if input.is_no_owner_viewport {
                state.no_owner_position(view_width, view_height, world_width, world_height)
            } else {
                state.stationary_position(view_width, view_height)
            }
        } else {
            let position = state.update(
                input.center.x,
                input.center.y,
                view_width,
                view_height,
                world_width,
                world_height,
                if input.is_no_owner_viewport {
                    0
                } else {
                    VIEWPORT_SCROLL_BORDER
                },
                input.scrolling,
                self.scroll_smooth,
            );
            position
        };
        if pending_observer_scroll != Vector2::ZERO {
            state.view_x = state.view_x.saturating_add(pending_observer_scroll.x);
            state.view_y = state.view_y.saturating_add(pending_observer_scroll.y);
            (view_x, view_y) = state.no_owner_position(
                view_width,
                view_height,
                world_width,
                world_height,
            );
        }
        let offset = if input.owner == OWNER_NONE {
            Vector2::ZERO
        } else {
            input.offset
        };
        let view_x = view_x.saturating_add(offset.x);
        let view_y = view_y.saturating_add(offset.y);
        // C4Viewport keeps the full ViewWdt/Hgt and clips landscape drawing
        // around any out-of-map portion. Preserve the existing Rust
        // letterbox representation by turning those portions into tiled
        // margins and drawing only the in-world content surface.
        let border_left = (-view_x).max(0).min(view_width);
        let border_top = (-view_y).max(0).min(view_height);
        let border_right = (view_width - world_width + view_x)
            .max(0)
            .min(view_width - border_left);
        let border_bottom = (view_height - world_height + view_y)
            .max(0)
            .min(view_height - border_top);

        self.content_audibility_facet = Some(AudibilityFacet {
            target_x: view_x + border_left,
            target_y: view_y + border_top,
            width: view_width - border_left - border_right,
            height: view_height - border_top - border_bottom,
        });
        self.full_audibility_facet = Some(AudibilityFacet {
            target_x: view_x,
            target_y: view_y,
            width: view_width,
            height: view_height,
        });
        self.current_audibility_facet = None;

        let offset_x = scaled_camera_border(border_left, zoom, rect.width) as i32;
        let offset_y = scaled_camera_border(border_top, zoom, rect.height) as i32;
        let right_pixels = scaled_camera_border(border_right, zoom, rect.width);
        let bottom_pixels = scaled_camera_border(border_bottom, zoom, rect.height);
        let content_width = rect
            .width
            .saturating_sub(offset_x as u32)
            .saturating_sub(right_pixels)
            .max(1);
        let content_height = rect
            .height
            .saturating_sub(offset_y as u32)
            .saturating_sub(bottom_pixels)
            .max(1);
        let origin_x = (view_x + border_left) as f32;
        let origin_y = (view_y + border_top) as f32;

        self.viewport_x = origin_x;
        self.viewport_y = origin_y;
        self.viewport_zoom = zoom;

        self.surface_width = content_width;
        self.surface_height = content_height;

        let fog_map = build_fog_modulation_map(
            snapshot,
            input.owner,
            origin_x as i32,
            origin_y as i32,
            view_width,
            view_height,
        );
        let fade_transparent = fog_map
            .as_ref()
            .is_some_and(|map| map.fade_transparent);
        let has_scroll_borders = offset_x != 0
            || offset_y != 0
            || content_width != rect.width
            || content_height != rect.height;
        let background = self.hud_graphics.background.clone();
        let capture_gpu_scene = self.surface.is_gpu_scene_capture_active();
        let mut viewport_surface = has_scroll_borders.then(|| {
            let mut surface = Surface::new(rect.width, rect.height, format);
            if capture_gpu_scene {
                surface.begin_gpu_scene_capture();
            }
            draw_viewport_underlay(
                &mut self.tiled_underlay_cache,
                &mut surface,
                background.as_ref(),
                rect.x,
                rect.y,
                gamma,
            );
            surface
        });

        let capture_native_text = self.surface.is_clonk_text_capture_active();
        let mut content_surface = Surface::new(content_width.max(1), content_height.max(1), format);
        if capture_gpu_scene {
            content_surface.begin_gpu_scene_capture();
        }
        if capture_native_text {
            content_surface.begin_clonk_text_capture();
        }
        if fade_transparent && viewport_surface.is_none() {
            // With no scroll borders the world content replaces the complete
            // viewport. Seed that content directly from the viewport underlay
            // for translucent FoW instead of allocating an equally-sized
            // scratch surface, copying it into content, then copying it back.
            draw_viewport_underlay(
                &mut self.tiled_underlay_cache,
                &mut content_surface,
                background.as_ref(),
                rect.x,
                rect.y,
                gamma,
            );
        }
        let main_surface = std::mem::replace(&mut self.surface, content_surface);

        if fade_transparent {
            // Reset draws FoWColor onto the viewport before enabling the map.
            // Preserve the tiled viewport underlay for translucent colors.
            if let Some(viewport_surface) = viewport_surface.as_ref() {
                if capture_gpu_scene {
                    blit_surface(&mut self.surface, viewport_surface, -offset_x, -offset_y);
                } else {
                    for y in 0..content_height {
                        for x in 0..content_width {
                            if let Some(color) = viewport_surface.get_pixel(
                                (offset_x + x as i32) as u32,
                                (offset_y + y as i32) as u32,
                            ) {
                                let _ = self.surface.set_pixel(x, y, color);
                            }
                        }
                    }
                }
            }
            draw_color_rect(
                &mut self.surface,
                SurfaceRect::new(0, 0, content_width, content_height),
                c4_color_to_surface(snapshot.environment.fow_color),
                gamma,
            );
        }
        self.active_fog_map = fog_map.map(Arc::new);
        self.fog_suppression_depth = 0;

        let environment = &snapshot.environment;
        let events = &snapshot.weather_events;
        let lighting = Self::lighting_factor(environment.settings.time_of_day);

        self.draw_sky(snapshot.sky.as_ref(), environment, events, lighting, gamma);
        // C4D_Background objects live in Game.BackObjects and draw between
        // sky and landscape (C4Viewport.cpp:1051-1063).
        self.draw_objects_at_frame(
            snapshot.frame,
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.particles,
            &snapshot.players,
            input.owner,
            lighting,
            owner_colors,
            ObjectRenderPass::Background,
            gamma,
        );
        let textured_landscape = self.draw_ground(
            environment.ambient_temperature,
            snapshot.landscape.as_ref(),
            lighting,
            gamma,
        );
        // C4Landscape::Draw presents the material-colored Surface32 once and
        // supplies a separate alpha-only liquid-animation mask to
        // BlitLandscape (C4Landscape.cpp:261-270,2599-2616). The scalar
        // repaint below predates the raster renderer and remains only for
        // column-only fixture worlds that have no Surface8 equivalent.
        if !textured_landscape {
            self.draw_liquids(
                environment.ambient_temperature,
                snapshot.landscape.as_ref(),
                lighting,
                gamma,
            );
        }
        // C4Viewport draws sync-relevant C4PXS after the landscape and before
        // objects. Weather precipitation reaches this same path after the
        // simulation creates rain/snow PXS; there is no procedural viewport
        // rain layer (C4Viewport.cpp:1056-1078; C4PXS.cpp:242-307).
        self.draw_pxs(&snapshot.particles, lighting, gamma);
        self.draw_objects_at_frame(
            snapshot.frame,
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.particles,
            &snapshot.players,
            input.owner,
            lighting,
            owner_colors,
            ObjectRenderPass::Normal,
            gamma,
        );
        self.draw_definition_particles(
            &snapshot.particles,
            &ParticleLayer::Global,
            None,
            gamma,
        );
        self.draw_objects_at_frame(
            snapshot.frame,
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.particles,
            &snapshot.players,
            input.owner,
            lighting,
            owner_colors,
            ObjectRenderPass::ForegroundNonParallax,
            gamma,
        );
        if self.debug_draw_flags.show_pathfinder {
            self.draw_pathfinder_debug(&snapshot.pathfinder_debug, gamma);
        }
        // NeedEnergy bolts are emitted inside each object's base pass so
        // background/foreground category layering matches C4Object::Draw.
        if self.viewport_overlays_visible {
            if input.owner != OWNER_NONE {
                if let Some(focus) = input.focus {
                    let highlight_ids = Self::collect_highlight_ids(snapshot, input.owner, focus.id);
                    self.draw_selection_marks(
                        snapshot,
                        &highlight_ids,
                        input.owner,
                        zoom,
                        gamma,
                    );
                }
                self.draw_player_cursors(snapshot, input.owner, origin_x, origin_y, zoom, gamma);
            } else {
                // C4Game::DrawCursors(NO_OWNER) emits every player's active
                // cursor flash, while C4Object::DrawSelectMark requires a valid
                // `iByPlayer` and therefore emits no per-object select marks.
                for player in &snapshot.players {
                    self.draw_player_cursors(
                        snapshot,
                        player.id,
                        origin_x,
                        origin_y,
                        zoom,
                        gamma,
                    );
                }
            }
        }
        // C4Viewport disables ClrModMap after world cursors and before the
        // custom parallax GUI/overlay pass.
        self.active_fog_map = None;
        let pending_foreground = if capture_native_text {
            let mut foreground = Surface::new(content_width.max(1), content_height.max(1), format);
            foreground.begin_clonk_text_capture();
            let base_surface = std::mem::replace(&mut self.surface, foreground);
            self.draw_objects_at_frame(
                snapshot.frame,
                &snapshot.objects,
                &snapshot.render_order,
                &snapshot.definition_lines,
                &snapshot.particles,
                &snapshot.players,
                input.owner,
                lighting,
                owner_colors,
                ObjectRenderPass::ForegroundParallax,
                gamma,
            );
            let foreground = std::mem::replace(&mut self.surface, base_surface);
            Some(PendingViewportForeground {
                surface: foreground,
                destination: SurfacePoint::new(rect.x + offset_x, rect.y + offset_y),
            })
        } else {
            self.draw_objects_at_frame(
                snapshot.frame,
                &snapshot.objects,
                &snapshot.render_order,
                &snapshot.definition_lines,
                &snapshot.particles,
                &snapshot.players,
                input.owner,
                lighting,
                owner_colors,
                ObjectRenderPass::ForegroundParallax,
                gamma,
            );
            None
        };

        let mut content_surface = std::mem::replace(&mut self.surface, main_surface);

        self.surface_width = saved_surface_width;
        self.surface_height = saved_surface_height;
        self.viewport_x = saved_viewport_x;
        self.viewport_y = saved_viewport_y;
        self.viewport_zoom = saved_viewport_zoom;
        self.world_width = saved_world_width;
        self.world_height = saved_world_height;
        self.content_audibility_facet = saved_content_audibility_facet;
        self.full_audibility_facet = saved_full_audibility_facet;
        self.current_audibility_facet = saved_current_audibility_facet;

        present_viewport_content(
            &mut self.surface,
            viewport_surface.as_mut(),
            &content_surface,
            rect,
            offset_x,
            offset_y,
        );
        if capture_native_text {
            let _ = self.surface.extend_clonk_text_capture_from(
                &mut content_surface,
                SurfacePoint::new(rect.x + offset_x, rect.y + offset_y),
            );
        }
        if let Some(foreground) = pending_foreground {
            self.pending_viewport_foregrounds.push(foreground);
        }

        self.active_viewports.push(ActiveViewport {
            owner: input.owner,
            focus: input.focus.map(|focus| focus.id),
            rect,
            content_rect: SurfaceRect::new(
                rect.x + offset_x,
                rect.y + offset_y,
                content_width,
                content_height,
            ),
            target_x: view_x,
            target_y: view_y,
            logical_width: view_width,
            logical_height: view_height,
            world_width,
            world_height,
            viewport_x: origin_x,
            viewport_y: origin_y,
            zoom,
            camera_key: key,
            is_no_owner_viewport: input.is_no_owner_viewport,
        });
    }

    /// `C4GraphicsSystem::RecalculateViewports` caps fullscreen viewport
    /// output to the landscape plus the two scroll borders and centers the
    /// result inside its layout cell (src/C4GraphicsSystem.cpp:384-396).
    fn centered_viewport_rect(&self, area: SurfaceRect) -> SurfaceRect {
        Self::centered_viewport_rect_for_world(area, self.world_width, self.world_height)
    }

    pub(crate) fn centered_viewport_rect_for_world(
        area: SurfaceRect,
        world_width: i32,
        world_height: i32,
    ) -> SurfaceRect {
        let border = VIEWPORT_SCROLL_BORDER.saturating_mul(2);
        let max_width = world_width.max(1).saturating_add(border) as u32;
        let max_height = world_height.max(1).saturating_add(border) as u32;
        let width = area.width.min(max_width);
        let height = area.height.min(max_height);
        SurfaceRect::new(
            area.x + area.width.saturating_sub(width) as i32 / 2,
            area.y + area.height.saturating_sub(height) as i32 / 2,
            width,
            height,
        )
    }

    fn collect_highlight_ids(
        snapshot: &SimulationSnapshot,
        owner: i32,
        focus: ObjectId,
    ) -> HashSet<ObjectId> {
        let mut highlights: HashSet<ObjectId> = HashSet::new();
        highlights.insert(focus);
        if let Some(selection) = snapshot.crew_selection.get(&owner) {
            if let Some(cursor) = selection.cursor {
                highlights.insert(cursor);
            }
            highlights.extend(selection.selected.iter().copied());
        }
        if let Some(state) = snapshot.players.iter().find(|state| state.id == owner) {
            if let Some(cursor) = state.cursor {
                highlights.insert(cursor);
            }
        }
        for player in &snapshot.hud.players {
            if player.owner == owner {
                if let Some(focus_id) = player.focus {
                    highlights.insert(focus_id);
                }
            }
        }
        highlights
    }

    /// `C4Object::DrawSelectMark` (src/C4Object.cpp:3839-3857): the four
    /// PHASES of fctSelectMark (square cells of sheet height) sit at the
    /// shape corners offset by -2. Gated on the owning player's SelectFlash
    /// (src/C4Object.cpp:2497-2502).
    fn draw_selection_marks(
        &mut self,
        snapshot: &SimulationSnapshot,
        highlights: &HashSet<ObjectId>,
        owner: i32,
        zoom: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let Some(image) = self.hud_graphics.select_mark.clone() else {
            return;
        };
        // `Game.Players.Get(Owner)->SelectFlash` (src/C4Object.cpp:2501);
        // fixture snapshots without player entries keep the marks visible.
        if snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .map(|player| player.control.select_flash <= 0)
            .unwrap_or(false)
        {
            return;
        }
        let cell = image.height() as i32;
        let fog = self.fog_draw_context();
        let surface_width = self.surface_width as f32;
        let surface_height = self.surface_height as f32;
        let margin = (cell as f32).max(16.0);
        for id in highlights {
            let Some(object) = snapshot.object(*id) else {
                continue;
            };
            // C4Object::Draw returns from the ShowSolidMask branch before
            // DrawSelectMark is reached.
            if self.debug_draw_flags.show_solid_mask && self.object_has_debug_solid_mask(object) {
                continue;
            }
            // DrawSelectMark resolves its origin through TargetPos just like
            // C4Object::Draw, so marks stay locked to a pinned C4D_Parallax
            // object (src/C4Object.cpp:3887-3893).
            let (target_x, target_y) = self.object_target_position(object);
            let screen_x = (object.position.x as f32 - target_x) * zoom;
            let screen_y = (object.position.y as f32 - target_y) * zoom;
            if screen_x < -margin
                || screen_x > surface_width + margin
                || screen_y < -margin
                || screen_y > surface_height + margin
            {
                continue;
            }

            let shape = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .map(Self::sprite_def_shape)
                .filter(|shape| shape.width > 0 && shape.height > 0)
                .unwrap_or_else(|| DefinitionRect::new(-6, -6, 12, 12));
            // cox/coy = x + Shape.x - 2 (src/C4Object.cpp:3850-3856).
            let cox = screen_x + (shape.x as f32) * zoom - 2.0;
            let coy = screen_y + (shape.y as f32) * zoom - 2.0;
            let shape_width = shape.width as f32 * zoom;
            let shape_height = shape.height as f32 * zoom;
            let corners = [
                (cox, coy, 0),
                (cox + shape_width, coy, 1),
                (cox, coy + shape_height, 2),
                (cox + shape_width, coy + shape_height, 3),
            ];
            // This mark is normally emitted inside C4Object::Draw, while
            // C4D_IgnoreFoW has the modulation map temporarily disabled.
            let fog_disabled_for_parallax = object.category & CATEGORY_FOREGROUND_FLAG != 0
                && object.category & CATEGORY_PARALLAX_FLAG != 0;
            let object_fog = (object.category & CATEGORY_IGNORE_FOW_FLAG == 0
                && !fog_disabled_for_parallax)
                .then_some(())
                .and(fog.as_ref());
            for (px, py, phase) in corners {
                let source = SourceRect::new(phase * cell, 0, cell, cell);
                if !Self::source_within_image(&image, &source) {
                    continue;
                }
                let rect = GuiRect::from_origin_size(
                    GuiPoint::new(px, py),
                    GuiSize::new(cell as f32, cell as f32),
                );
                draw_image_region(
                    &mut self.surface,
                    &rect,
                    &image,
                    None,
                    &source,
                    false,
                    None,
                    SpriteBlitState::normal().with_renderer_config(self.advanced_renderer_config),
                    gamma,
                    object_fog,
                );
            }
        }
    }

    pub(crate) fn layout_viewports(&self, count: usize) -> Vec<SurfaceRect> {
        if count == 0 {
            return Vec::new();
        }

        // Viewport area between the upper board and the message board
        // (C4GraphicsSystem::RecalculateViewports,
        // src/C4GraphicsSystem.cpp:343-348).
        let chrome = self.hud_chrome_active();
        let mut overlay_height = if chrome {
            hud::upper_board_reserved_height(self.upper_board_mode)
                .clamp(0, self.surface_height as i32)
        } else {
            0
        };
        let board_height = if chrome {
            self.message_board_height()
                .clamp(0, self.surface_height as i32)
        } else {
            0
        };
        let mut available_height = (self.surface_height as i32)
            .saturating_sub(overlay_height)
            .saturating_sub(board_height);
        if available_height <= 0 {
            // Surface too small to host the overlay and a viewport. Give the
            // entire surface to the viewport and suppress the overlay instead
            // of producing a zero-height viewport that won't render anything.
            overlay_height = 0;
            available_height = self.surface_height as i32;
        }
        if available_height <= 0 {
            return vec![SurfaceRect::new(
                0,
                overlay_height,
                self.surface_width,
                available_height.max(0) as u32,
            )];
        }

        // C4GraphicsSystem::RecalculateViewports uses floor(sqrt(count)) rows.
        // Any remainder adds one column to the first rows; pixel remainders
        // from the integer cell divisions stay available for the background.
        let rows = ((count as f32).sqrt() as usize).max(1);
        let base_columns = count / rows;
        let longer_rows = count % rows;
        let available_width = self.surface_width;
        let row_height = available_height as u32 / rows as u32;
        let mut rects = Vec::with_capacity(count);
        for row in 0..rows {
            let columns = base_columns + usize::from(row < longer_rows);
            let column_width = available_width / columns as u32;
            for col in 0..columns {
                // Graphics.SplitscreenDividers defaults to enabled. C++ takes
                // four pixels only from non-last cells, leaving no outer inset.
                let divider_width = if self.splitscreen_dividers && col + 1 < columns {
                    4
                } else {
                    0
                };
                let divider_height = if self.splitscreen_dividers && row + 1 < rows {
                    4
                } else {
                    0
                };
                rects.push(SurfaceRect::new(
                    (col as u32 * column_width) as i32,
                    overlay_height + (row as i32 * row_height as i32),
                    column_width.saturating_sub(divider_width),
                    row_height.saturating_sub(divider_height),
                ));
            }
        }

        rects
    }

    pub fn ground_height_at(&self, landscape: Option<&Landscape>, x: i32) -> i32 {
        let clamped_x = if self.world_width > 0 {
            x.clamp(0, self.world_width.saturating_sub(1))
        } else {
            x
        };
        self.surface_height_at(landscape, clamped_x)
            .unwrap_or(self.fallback_ground_height)
    }

    fn update_world_dimensions(&mut self, landscape: Option<&Landscape>) {
        if let Some(landscape) = landscape {
            let width = landscape.width() as i32;
            if width > 0 {
                self.world_width = width;
            }

            self.world_height = landscape.estimated_height().max(1);
        } else {
            if self.world_width <= 0 {
                self.world_width = self.surface_width as i32;
            }
            if self.world_height <= 0 {
                self.world_height = self
                    .fallback_ground_height
                    .max(self.surface_height as i32)
                    .max(1);
            }
        }
    }

    pub(crate) fn draw_sky(
        &mut self,
        frame: Option<&SkyFrame>,
        environment: &EnvironmentFrame,
        events: &[WeatherEvent],
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if let Some(state) = self.sky.clone() {
            self.render_configured_sky(&state, frame, events, lighting, gamma);
        } else {
            let base = environment
                .sky_color
                .map(|color| Color::opaque(color.r, color.g, color.b))
                .unwrap_or_else(|| {
                    Self::sky_color_for_temperature(environment.ambient_temperature)
                });
            let tinted = Self::apply_lighting(base, lighting);
            self.fill_world_color(tinted, true, gamma);
        }
    }

    fn render_configured_sky(
        &mut self,
        state: &SkyRenderState,
        frame: Option<&SkyFrame>,
        _events: &[WeatherEvent],
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let settings = frame
            .map(|frame| &frame.settings)
            .unwrap_or(&state.settings);

        if let Some(color) = settings.back_color {
            let base = Self::bgr_to_color(color);
            let tinted = Self::apply_lighting(base, lighting);
            self.fill_world_color(tinted, false, gamma);
        } else if settings.has_surface
            && !self
                .active_fog_map
                .as_ref()
                .is_some_and(|map| map.fade_transparent)
            && !(state.image_is_fully_opaque()
                && settings
                    .modulation
                    .is_none_or(|modulation| modulation >> 24 == 0))
        {
            // The legacy Rust surface starts transparent, while the native
            // render target already has an opaque backing. A complete opaque
            // sky tile with opaque modulation overwrites every destination,
            // so materializing that otherwise-dead backing would repaint the
            // whole viewport for no visible result. Do not synthesize it when
            // Reset has explicitly painted FoWColor either: every world layer
            // must fade independently onto that color.
            self.fill_world_color(Color::opaque(0, 0, 0), false, gamma);
        }

        if settings.has_surface {
            if let Some(image) = state.image() {
                self.tile_sky_image(image, settings, frame, lighting, gamma);
            } else {
                self.fill_sky_gradient(settings, lighting, gamma);
            }
        } else if settings.back_color.is_none() {
            self.fill_sky_gradient(settings, lighting, gamma);
        }
    }

    pub(crate) fn fill_sky_gradient(
        &mut self,
        settings: &SkySettings,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        // C4Sky::Draw without a surface fades from GetSkyFadeClr(TargetY)
        // to GetSkyFadeClr(TargetY+Hgt) (C4Sky.cpp:219-225): the fade spans
        // the landscape height in world coordinates, offset by the
        // viewport origin — not merely the visible window.
        let zoom = if self.viewport_zoom > 0.0 {
            self.viewport_zoom
        } else {
            1.0
        };
        let view_top = self.viewport_y.round() as i32;
        let view_bottom = (self.viewport_y + self.surface_height as f32 / zoom).round() as i32;
        let top = Self::sky_fade_color(settings, view_top, self.world_height);
        let bottom = Self::sky_fade_color(settings, view_bottom, self.world_height);
        let top = Color::opaque(top.r, top.g, top.b);
        let bottom = Color::opaque(bottom.r, bottom.g, bottom.b);
        self.fill_vertical_gradient_modulated(
            top,
            bottom,
            lighting,
            settings.modulation,
            gamma,
        );
    }

    /// C4Sky::GetSkyFadeClr (C4Sky.cpp:230-236): integer fade between
    /// FadeClr1 (world top) and FadeClr2 across the landscape height —
    /// iPos2 = iY*256/GBackHgt, channel = (c1*iPos1 + c2*iPos2) >> 8.
    /// C++ never sees out-of-landscape Y (the viewport is clamped); the
    /// clamp here keeps stray coordinates from wrapping the fixed-point mix.
    pub(crate) fn sky_fade_color(settings: &SkySettings, world_y: i32, world_height: i32) -> RgbColor {
        let height = world_height.max(1);
        let pos2 = (world_y * 256 / height).clamp(0, 256);
        let pos1 = 256 - pos2;
        let channel =
            |c1: u8, c2: u8| ((i32::from(c1) * pos1 + i32::from(c2) * pos2) >> 8).clamp(0, 255) as u8;
        RgbColor::new(
            channel(settings.fade_top.r, settings.fade_bottom.r),
            channel(settings.fade_top.g, settings.fade_bottom.g),
            channel(settings.fade_top.b, settings.fade_bottom.b),
        )
    }

    pub(crate) fn fill_vertical_gradient(
        &mut self,
        top: Color,
        bottom: Color,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.fill_vertical_gradient_modulated(top, bottom, lighting, None, gamma);
    }

    pub(crate) fn fill_vertical_gradient_modulated(
        &mut self,
        top: Color,
        bottom: Color,
        lighting: f32,
        modulation: Option<u32>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if self.surface_width == 0 || self.surface_height == 0 {
            return;
        }
        let fog = self.fog_box_sampler(true);
        let height = self.surface_height.saturating_sub(1).max(1);
        // DrawBoxFade applies the active Sky.Modulation to each color vertex
        // before DrawQuadDw adds ClrModMap modulation at that same vertex.
        let color_at_y = |t: f32| {
            let color = Self::apply_lighting(Self::lerp_color(top, bottom, t), lighting);
            modulation.map_or(color, |modulation| {
                modulate_surface_color(color, modulation)
            })
        };
        let offset = self.advanced_renderer_config.destination_offset();
        let x_start = ((offset - 0.5).ceil() as i32).clamp(0, self.surface_width as i32);
        let y_start = ((offset - 0.5).ceil() as i32).clamp(0, self.surface_height as i32);
        let x_end = ((offset + self.surface_width as f32 - 0.5).ceil() as i32)
            .clamp(0, self.surface_width as i32);
        let y_end = ((offset + self.surface_height as f32 - 0.5).ceil() as i32)
            .clamp(0, self.surface_height as i32);
        if self.surface.is_gpu_scene_capture_active()
            && fog.as_ref().is_none_or(|(_, sampler)| sampler.is_some())
        {
            let mut emit =
                |left: f32, top: f32, right: f32, bottom: f32, fog_modulation: Option<[u32; 4]>| {
                    let base_top = color_at_y((top / height as f32).clamp(0.0, 1.0));
                    let base_bottom = color_at_y((bottom / height as f32).clamp(0.0, 1.0));
                    let mut colors = fog_modulation.map_or(
                        [base_top, base_top, base_bottom, base_bottom],
                        |fog| {
                            [
                                modulate_surface_color(base_top, fog[0]),
                                modulate_surface_color(base_top, fog[1]),
                                modulate_surface_color(base_bottom, fog[2]),
                                modulate_surface_color(base_bottom, fog[3]),
                            ]
                        },
                    );
                    if self.advanced_renderer_config.no_box_fades {
                        let normalized =
                            normalize_quad_colors([colors[0], colors[2], colors[3], colors[1]]);
                        colors = [normalized; 4];
                    }
                    record_gpu_solid_quad(
                        &mut self.surface,
                        (left + offset, top + offset, right + offset, bottom + offset),
                        colors,
                        GpuBlend::Normal,
                        gamma.is_some_and(|gamma| !gamma.is_passthrough()),
                    );
                };
            if let Some((_, Some(sampler))) = fog.as_ref() {
                for quad in &sampler.quads {
                    let left = quad.x.0 / sampler.source_width * self.surface_width as f32;
                    let right = quad.x.1 / sampler.source_width * self.surface_width as f32;
                    let top = quad.y.0 / sampler.source_height * self.surface_height as f32;
                    let bottom = quad.y.1 / sampler.source_height * self.surface_height as f32;
                    emit(left, top, right, bottom, Some(quad.modulation));
                }
            } else {
                emit(
                    0.0,
                    0.0,
                    self.surface_width as f32,
                    self.surface_height as f32,
                    None,
                );
            }
            return;
        }
        let fog_raster_axes = fog.as_ref().and_then(|(_, sampler)| {
            sampler.as_ref().map(|sampler| {
                sampler.raster_axes_with_destination_offset(
                    self.surface_width,
                    self.surface_height,
                    offset,
                    offset,
                )
            })
        });
        let normalized = self.advanced_renderer_config.no_box_fades.then(|| {
            normalize_quad_colors([
                color_at_y(0.0),
                color_at_y(1.0),
                color_at_y(1.0),
                color_at_y(0.0),
            ])
        });
        for y in y_start as u32..y_end as u32 {
            let logical_y = y as f32 - offset;
            let t = logical_y / height as f32;
            let tinted = normalized.unwrap_or_else(|| color_at_y(t));
            for x in x_start as u32..x_end as u32 {
                let tinted = fog.as_ref().map_or(tinted, |(fog, sampler)| {
                    sampler.as_ref().map_or_else(
                        || fog.color_at(tinted, x as i32, y as i32),
                        |sampler| match fog_raster_axes.as_ref() {
                            Some((x_samples, y_samples))
                                if self.advanced_renderer_config.no_box_fades =>
                            {
                                sampler.normalized_vertical_color_at_axes(
                                    x_samples[x as usize],
                                    y_samples[y as usize],
                                    |vertex_y| {
                                        let t = (vertex_y * self.surface_height as f32
                                            / height as f32)
                                            .clamp(0.0, 1.0);
                                        color_at_y(t)
                                    },
                                )
                            }
                            Some((x_samples, y_samples)) => sampler.vertical_color_at_axes(
                                x_samples[x as usize],
                                y_samples[y as usize],
                                |vertex_y| {
                                    let t = (vertex_y * self.surface_height as f32
                                        / height as f32)
                                        .clamp(0.0, 1.0);
                                    color_at_y(t)
                                },
                            ),
                            None => unreachable!("sampler axes accompany a fog sampler"),
                        },
                    )
                });
                self.draw_prepared_world_color_pixel(x, y, tinted, gamma);
            }
        }
    }

    fn tile_sky_image(
        &mut self,
        image: &ImageData,
        settings: &SkySettings,
        frame: Option<&SkyFrame>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let visible_pixels = (self.surface_width as usize)
            .saturating_mul(self.surface_height as usize);
        self.tile_sky_image_with_parallel_rows(
            image,
            settings,
            frame,
            lighting,
            gamma,
            visible_pixels >= PARALLEL_SKY_MIN_PIXELS,
        );
    }

    pub(crate) fn tile_sky_image_with_parallel_rows(
        &mut self,
        image: &ImageData,
        settings: &SkySettings,
        frame: Option<&SkyFrame>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
        parallel_rows: bool,
    ) {
        let width = image.width();
        let height = image.height();
        if width == 0 || height == 0 {
            return;
        }
        let width_f = width as f32;
        let height_f = height as f32;
        let runtime_x = frame.map(|frame| frame.offset_x).unwrap_or(0.0);
        let runtime_y = frame.map(|frame| frame.offset_y).unwrap_or(0.0);
        let parallax_x = if settings.parallax_x == 0 {
            10
        } else {
            settings.parallax_x
        };
        let parallax_y = if settings.parallax_y == 0 {
            10
        } else {
            settings.parallax_y
        };
        let source_x = (self.viewport_x * 10.0 / parallax_x as f32) - runtime_x;
        let source_y = (self.viewport_y * 10.0 / parallax_y as f32) - runtime_y;
        let offset_x = Self::normalize_offset(source_x, width_f);
        let offset_y = Self::normalize_offset(source_y, height_f);
        let modulation = settings.modulation;

        let mut positions = Vec::new();
        let mut y = -offset_y;
        while y < self.surface_height as f32 {
            let mut x = -offset_x;
            while x < self.surface_width as f32 {
                positions.push((x.round() as i32, y.round() as i32));
                x += width_f;
            }
            y += height_f;
        }
        self.draw_sky_tile_positions_with_parallel_rows(
            image,
            &positions,
            modulation,
            lighting,
            gamma,
            parallel_rows,
        );
    }

    pub(crate) fn blit_sky_tile(
        &mut self,
        image: &ImageData,
        dest_x: i32,
        dest_y: i32,
        modulation: Option<u32>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let visible_pixels = SkyTileBounds::visible(
            self.surface_width,
            self.surface_height,
            image.width(),
            image.height(),
            dest_x,
            dest_y,
        )
        .map_or(0, SkyTileBounds::pixel_count);
        self.blit_sky_tile_with_parallel_rows(
            image,
            dest_x,
            dest_y,
            modulation,
            lighting,
            gamma,
            visible_pixels >= PARALLEL_SKY_MIN_PIXELS,
        );
    }

    fn blit_sky_tile_with_parallel_rows(
        &mut self,
        image: &ImageData,
        dest_x: i32,
        dest_y: i32,
        modulation: Option<u32>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
        parallel_rows: bool,
    ) {
        self.draw_sky_tile_positions_with_parallel_rows(
            image,
            &[(dest_x, dest_y)],
            modulation,
            lighting,
            gamma,
            parallel_rows,
        );
    }

    pub(crate) fn retained_lit_sky_texture(
        &mut self,
        source: &ImageData,
        lighting: f32,
    ) -> (ImageData, GpuTextureResource) {
        let source_id = source.gpu_texture_id();
        let lighting_bits = lighting.to_bits();
        if let Some(cached) = self.retained_lit_sky.as_ref() {
            if cached.source == source_id && cached.lighting == lighting_bits {
                return (
                    cached.image.clone(),
                    GpuTextureResource {
                        id: cached.texture,
                        extent: [cached.image.width(), cached.image.height()],
                        revision: cached.revision,
                        base_revision: None,
                        format: clonk_graphics::GpuTextureFormat::Rgba8,
                        pixels: cached.image.pixels_arc(),
                        dirty: Vec::new(),
                    },
                );
            }
        }

        let pixels: Arc<[u8]> = Arc::from(
            lit_sky_texels(source, lighting)
                .into_iter()
                .flat_map(|color| [color.r, color.g, color.b, color.a])
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let mut base_revision = None;
        let mut dirty = Vec::new();
        match self.retained_lit_sky.as_mut() {
            Some(cached) => {
                cached.source = source_id;
                cached.lighting = lighting_bits;
                let same_extent = cached.image.width() == source.width()
                    && cached.image.height() == source.height();
                if !same_extent || cached.image.pixels() != pixels.as_ref() {
                    let previous_revision = cached.revision;
                    cached.revision = cached.revision.wrapping_add(1);
                    cached.image = ImageData::transient_from_arc(
                        source.width(),
                        source.height(),
                        Arc::clone(&pixels),
                    );
                    if same_extent {
                        base_revision = Some(previous_revision);
                        dirty.push(clonk_graphics::Rect::new(
                            0,
                            0,
                            source.width(),
                            source.height(),
                        ));
                    }
                }
            }
            None => {
                self.retained_lit_sky = Some(RetainedLitSkyTexture {
                    source: source_id,
                    lighting: lighting_bits,
                    image: ImageData::transient_from_arc(
                        source.width(),
                        source.height(),
                        Arc::clone(&pixels),
                    ),
                    texture: GpuTextureId::fresh(),
                    revision: 0,
                });
            }
        }
        let cached = self
            .retained_lit_sky
            .as_ref()
            .expect("retained lit sky was initialized");
        (
            cached.image.clone(),
            GpuTextureResource {
                id: cached.texture,
                extent: [cached.image.width(), cached.image.height()],
                revision: cached.revision,
                base_revision,
                format: clonk_graphics::GpuTextureFormat::Rgba8,
                pixels: cached.image.pixels_arc(),
                dirty,
            },
        )
    }

    pub(crate) fn draw_sky_tile_positions_with_parallel_rows(
        &mut self,
        image: &ImageData,
        positions: &[(i32, i32)],
        modulation: Option<u32>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
        parallel_rows: bool,
    ) {
        let width = image.width();
        let height = image.height();
        if width == 0 || height == 0 || positions.is_empty() {
            return;
        }
        let surface_width = self.surface_width;
        let surface_height = self.surface_height;
        if self.advanced_renderer_config.has_adjusted_quad_geometry() {
            let (lit_image, lit_resource) = self.retained_lit_sky_texture(image, lighting);
            let fog = self.fog_draw_context();
            let blit = SpriteBlitState {
                mode: 0,
                modulation,
                fog_modulation: None,
                renderer_config: self.advanced_renderer_config,
            };
            for &(dest_x, dest_y) in positions {
                let Some(bounds) = SkyTileBounds::visible(
                    surface_width,
                    surface_height,
                    width,
                    height,
                    dest_x,
                    dest_y,
                ) else {
                    continue;
                };
                let source = FloatSourceRect {
                    x: bounds.source_left as f32,
                    y: bounds.source_top as f32,
                    width: bounds.width() as f32,
                    height: bounds.height() as f32,
                };
                let destination = (
                    bounds.target_left() as f32,
                    bounds.target_top() as f32,
                    bounds.width() as f32,
                    bounds.height() as f32,
                );
                if !capture_gpu_sprite_with_resource(
                    &mut self.surface,
                    destination,
                    destination,
                    &GraphicsTransform::identity(),
                    &lit_image,
                    None,
                    source,
                    false,
                    None,
                    blit,
                    gamma,
                    fog.as_ref(),
                    GpuSampler::Nearest,
                    false,
                    Some(lit_resource.clone()),
                ) {
                    draw_image_region_float_source(
                        &mut self.surface,
                        &GuiRect::new(
                            bounds.target_left() as f32,
                            bounds.target_top() as f32,
                            bounds.width() as f32,
                            bounds.height() as f32,
                        ),
                        &lit_image,
                        None,
                        &source,
                        BlitSampling::Nearest,
                        false,
                        None,
                        blit,
                        gamma,
                        fog.as_ref(),
                    );
                }
            }
            return;
        }
        let fog = self.fog_draw_context();
        // BlitSurfaceTile2 trims each edge tile before handing it to Blit.
        // Build one sampler per cropped tile so those new crop edges remain
        // the ClrModMap vertices even though all tiles now share one row pass.
        let regions = positions
            .iter()
            .filter_map(|&(dest_x, dest_y)| {
                SkyTileBounds::visible(surface_width, surface_height, width, height, dest_x, dest_y)
            })
            .map(|bounds| SkyTileRegion::new(bounds, fog.as_ref(), width, height))
            .collect::<Vec<_>>();
        if regions.is_empty() {
            return;
        }
        let base_blit = SpriteBlitState {
            mode: 0,
            modulation,
            fog_modulation: None,
            renderer_config: self.advanced_renderer_config,
        };
        if self.surface.is_gpu_scene_capture_active() {
            let gpu_blit = if lighting == 1.0 {
                base_blit
            } else {
                let channel = (lighting.max(0.0) * 255.0).round().clamp(0.0, 255.0) as u32;
                let lighting_modulation = (channel << 16) | (channel << 8) | channel;
                SpriteBlitState {
                    modulation: Some(
                        base_blit
                            .modulation
                            .map(|modulation| modulate_c4_colors(modulation, lighting_modulation))
                            .unwrap_or(lighting_modulation),
                    ),
                    ..base_blit
                }
            };
            for region in &regions {
                let bounds = region.bounds;
                let target = GuiRect::from_origin_size(
                    GuiPoint::new(bounds.target_left() as f32, bounds.target_top() as f32),
                    GuiSize::new(bounds.width() as f32, bounds.height() as f32),
                );
                let source = SourceRect::new(
                    bounds.source_left,
                    bounds.source_top,
                    bounds.width(),
                    bounds.height(),
                );
                draw_image_region(
                    &mut self.surface,
                    &target,
                    image,
                    None,
                    &source,
                    false,
                    None,
                    gpu_blit,
                    gamma,
                    fog.as_ref(),
                );
            }
            return;
        }
        let mut region_indices_by_row = vec![Vec::new(); surface_height as usize];
        for (region_index, region) in regions.iter().enumerate() {
            let top = region.bounds.target_top() as usize;
            let bottom = (region.bounds.dest_y + region.bounds.source_bottom) as usize;
            for row in &mut region_indices_by_row[top..bottom] {
                row.push(region_index);
            }
        }
        // Lighting is constant for the complete tiled draw. C++ applies it
        // before per-tile modulation/fog, so caching these exact u8 texels
        // removes repeated work without changing shader ordering.
        let lit_texels = lit_sky_texels(image, lighting);
        // C4Sky leaves this packed modulation active while PerformBlt folds
        // the ClrModMap into every vertex. Keeping the two values in the blit
        // state preserves native `ModulateClr` ordering and transparency.
        let uses_blit_modulation = fog.is_some() || modulation.is_some();
        let row_context = SkyTileRowRenderContext {
            lit_texels: &lit_texels,
            image_width: width as usize,
            surface_width,
            regions: &regions,
            region_indices_by_row: &region_indices_by_row,
            base_blit,
            uses_blit_modulation,
            fog: fog.as_ref(),
            gamma,
            clip: self.surface.clip(),
        };
        draw_sky_tile_rows(
            &row_context,
            self.surface.pixels_mut(),
            surface_height,
            parallel_rows,
        );
    }

    fn normalize_offset(offset: f32, dimension: f32) -> f32 {
        if dimension <= 0.0 {
            return 0.0;
        }
        let mut wrapped = offset % dimension;
        if wrapped < 0.0 {
            wrapped += dimension;
        }
        wrapped
    }

    fn lerp_color(a: Color, b: Color, t: f32) -> Color {
        let clamped = t.clamp(0.0, 1.0);
        let lerp_channel = |start: u8, end: u8| -> u8 {
            let start = start as f32;
            let end = end as f32;
            (start + (end - start) * clamped).round().clamp(0.0, 255.0) as u8
        };
        Color::new(
            lerp_channel(a.r, b.r),
            lerp_channel(a.g, b.g),
            lerp_channel(a.b, b.b),
            255,
        )
    }

    fn bgr_to_color(value: u32) -> Color {
        let r = ((value >> 16) & 0xff) as u8;
        let g = ((value >> 8) & 0xff) as u8;
        let b = (value & 0xff) as u8;
        Color::opaque(r, g, b)
    }

    /// Draw one native particle list. Snapshot order is creation order,
    /// whereas C++ prepends new particles, so reverse iteration is native
    /// newest-first traversal.
    pub(crate) fn draw_definition_particles(
        &mut self,
        particles: &[ParticleSnapshot],
        layer: &ParticleLayer,
        target: Option<&ObjectSnapshot>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        for particle in particles.iter().rev() {
            if &particle.layer != layer {
                continue;
            }
            let Some(definition) = self
                .particle_sprites
                .get(&particle.definition_id)
                .cloned()
            else {
                continue;
            };
            match definition.draw_proc {
                ParticleDrawProc::Smoke => {
                    self.draw_smoke_particle(particle, &definition, gamma)
                }
                ParticleDrawProc::Std => {
                    self.draw_std_particle(particle, &definition, target, gamma)
                }
            }
        }
    }

    fn particle_parallax_target(&self, core: &ParticleDefCore) -> (i32, i32) {
        let target_x = self.viewport_x as i32;
        let target_y = self.viewport_y as i32;
        (
            target_x.wrapping_mul(core.parallaxity[0]) / 100,
            target_y.wrapping_mul(core.parallaxity[1]) / 100,
        )
    }

    fn particle_visible(&self, x: f32, y: f32, radius: f32, tx: i32, ty: i32) -> bool {
        if !(x.is_finite() && y.is_finite() && radius.is_finite()) {
            return false;
        }
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let width = (self.surface_width as f32 / zoom).ceil();
        let height = (self.surface_height as f32 / zoom).ceil();
        x >= tx as f32 - radius
            && x <= tx as f32 + width + radius
            && y >= ty as f32 - radius
            && y <= ty as f32 + height + radius
    }

    fn draw_smoke_particle(
        &mut self,
        particle: &ParticleSnapshot,
        definition: &ParticleRenderDefinition,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let (tx, ty) = self.particle_parallax_target(&definition.core);
        if !self.particle_visible(
            particle.position.x,
            particle.position.y,
            particle.parameter_a,
            tx,
            ty,
        ) {
            return;
        }
        let cx = particle.position.x as i32 - tx;
        let cy = particle.position.y as i32 - ty;
        let kind = particle.velocity.y as i32;
        let source = definition.facet.phase(kind / 4, kind % 4);
        let left = (cx as f32 - particle.parameter_a) as i32;
        let top = (cy as f32 - particle.parameter_a) as i32;
        let size = (particle.parameter_a * 2.0) as i32;
        if size <= 0 {
            return;
        }
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let destination = GuiRect::new(
            left as f32 * zoom,
            top as f32 * zoom,
            size as f32 * zoom,
            size as f32 * zoom,
        );
        let fog = self.fog_draw_context();
        draw_image_region(
            &mut self.surface,
            &destination,
            &definition.image,
            None,
            &source,
            false,
            None,
            SpriteBlitState {
                mode: 0,
                modulation: Some(particle.parameter_b as u32),
                fog_modulation: None,
                renderer_config: self.advanced_renderer_config,
            },
            gamma,
            fog.as_ref(),
        );
    }

    fn draw_std_particle(
        &mut self,
        particle: &ParticleSnapshot,
        definition: &ParticleRenderDefinition,
        target: Option<&ObjectSnapshot>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let (tx, ty) = self.particle_parallax_target(&definition.core);
        let mut x = particle.position.x;
        let mut y = particle.position.y;
        let mut xdir = particle.velocity.x;
        let mut ydir = particle.velocity.y;
        if definition.core.attach != 0 {
            if let Some(target) = target {
                x += target.position.x as f32;
                y += target.position.y as f32;
                if let Some(velocity) = target.fixed_velocity {
                    xdir += velocity.x.to_float();
                    ydir += velocity.y.to_float();
                } else {
                    xdir += target.velocity.x as f32;
                    ydir += target.velocity.y as f32;
                }
            }
        }
        if !self.particle_visible(x, y, particle.parameter_a, tx, ty) {
            return;
        }

        let mut phase = particle.life;
        if definition.core.delay != 0 {
            if phase >= 0 {
                phase /= definition.core.delay;
                if definition.core.reverse != 0 {
                    let length = definition.length - 1;
                    let cycle = length.saturating_mul(2);
                    if cycle == 0 {
                        return;
                    }
                    phase %= cycle;
                    if phase > length {
                        phase = length.saturating_mul(2).saturating_add(1) - phase;
                    }
                } else {
                    if definition.length == 0 {
                        return;
                    }
                    phase %= definition.length;
                }
            } else {
                if definition.core.fade_out_delay == 0 {
                    return;
                }
                phase = (phase + 1) / -definition.core.fade_out_delay + definition.length;
            }
        }

        let rotation = match definition.core.r_by_v {
            1 | 2 => c4_particle_angle((xdir * 10.0) as i32, (ydir * 10.0) as i32),
            3 => ((particle.position.x * 23.0 + particle.position.y * 12.0) as i32) % 360,
            _ => 0,
        };
        let half_width = particle.parameter_a as i32;
        let half_height = (definition.aspect * half_width as f32) as i32;
        if half_width <= 0 || half_height <= 0 {
            return;
        }
        let cgox = -tx;
        let cgoy = -ty;
        let cx = (x + cgox as f32) as i32;
        let cy = (y + cgoy as f32) as i32;
        let width = half_width.saturating_mul(2);
        let height = half_height.saturating_mul(2);
        let source = definition.facet.phase(phase, 0);
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let blit = SpriteBlitState {
            mode: self.advanced_renderer_config.masked_blit_mode(
                if definition.core.additive != 0 {
                    C4GFXBLIT_ADDITIVE
                } else {
                    0
                },
            ),
            modulation: Some(particle.parameter_b as u32),
            fog_modulation: None,
            renderer_config: self.advanced_renderer_config,
        };
        let fog = self.fog_draw_context();

        let clip_left = (cgox as f32 * zoom).round() as i32;
        let clip_top = ((cgoy + definition.core.y_off) as f32 * zoom).round() as i32;
        let lower_clip = lower_bounded_surface_clip(&self.surface, clip_left, clip_top);
        let previous_clip = self.surface.clip();
        let clip = previous_clip
            .and_then(|clip| clip.intersection(lower_clip))
            .unwrap_or_else(|| {
                if previous_clip.is_some() {
                    SurfaceRect::new(0, 0, 0, 0)
                } else {
                    lower_clip
                }
            });
        self.surface.set_clip(clip);

        if rotation != 0 {
            draw_image_region_rotated(
                &mut self.surface,
                cx as f32 * zoom,
                cy as f32 * zoom,
                width as f32 * zoom,
                height as f32 * zoom,
                &definition.image,
                None,
                &source,
                false,
                None,
                rotation as f32,
                blit,
                gamma,
                fog.as_ref(),
            );
        } else {
            draw_image_region(
                &mut self.surface,
                &GuiRect::new(
                    (cx - half_width) as f32 * zoom,
                    (cy - half_height) as f32 * zoom,
                    width as f32 * zoom,
                    height as f32 * zoom,
                ),
                &definition.image,
                None,
                &source,
                false,
                None,
                blit,
                gamma,
                fog.as_ref(),
            );
        }
        match previous_clip {
            Some(clip) => self.surface.set_clip(clip),
            None => self.surface.clear_clip(),
        }
    }

    pub(crate) fn draw_pxs(
        &mut self,
        particles: &[ParticleSnapshot],
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        // C4PXSSystem::Draw is deliberately two-pass: every old-style
        // pixel/velocity line first, then every material sprite. Thus a
        // graphical PXS overlays every old-style PXS regardless of slot
        // order (C4PXS.cpp:248-307).
        for particle in particles {
            if !self.pxs_visible(particle) {
                continue;
            }
            let Some(material_name) = particle
                .definition_id
                .strip_prefix("material/pxs/")
                .map(clonk_resources::material::c4_name_key)
            else {
                continue;
            };
            let Some(material) = self.material_render_info.get(&material_name) else {
                continue;
            };
            if self.pxs_graphics(material).is_some() {
                continue;
            }
            let material = material.clone();
            self.draw_old_style_pxs(particle, &material, lighting, gamma);
        }

        let mut compacted_slot = 0u32;
        for particle in particles {
            let Some(material_name) = particle
                .definition_id
                .strip_prefix("material/pxs/")
                .map(clonk_resources::material::c4_name_key)
            else {
                continue;
            };
            let fallback_slot = compacted_slot;
            compacted_slot = compacted_slot.wrapping_add(1);
            if !self.pxs_visible(particle) {
                continue;
            }
            let Some(material) = self.material_render_info.get(&material_name).cloned() else {
                continue;
            };
            let Some((texture, rect)) = self
                .pxs_graphics(&material)
                .map(|(texture, rect)| (texture.clone(), rect))
            else {
                continue;
            };
            let slot = particle.pxs_slot.unwrap_or(fallback_slot) as usize % 500;
            self.draw_graphical_pxs(particle, &material, &texture, rect, slot, lighting, gamma);
        }
    }

    fn pxs_visible(&self, particle: &ParticleSnapshot) -> bool {
        // VisibleRect is the world target rectangle enlarged by 20 and tests
        // the CURRENT fixtoi position before either pass. It intentionally
        // does not draw a long velocity line merely because that line crosses
        // the viewport (C4PXS.cpp:245-259,283-288).
        let [x, y, _, _] = Self::pxs_fixed(particle);
        let x = clonk_engine::math::fixtoi(x);
        let y = clonk_engine::math::fixtoi(y);
        let zoom = self.viewport_zoom.max(f32::EPSILON);
        let left = self.viewport_x.floor() as i32 - 20;
        let top = self.viewport_y.floor() as i32 - 20;
        let width = (self.surface.width() as f32 / zoom).ceil() as i32 + 40;
        let height = (self.surface.height() as f32 / zoom).ceil() as i32 + 40;
        x >= left && x < left + width && y >= top && y < top + height
    }

    fn pxs_graphics(
        &self,
        material: &MaterialRenderInfo,
    ) -> Option<(&ImageData, [i32; 6])> {
        let rect = material.pxs_gfx_rect;
        if rect[2] <= 0 || rect[3] <= 0 || material.pxs_gfx_size <= 0 {
            return None;
        }
        material
            .pxs_gfx
            .as_deref()
            .and_then(|name| {
                self.material_textures
                    .get(&clonk_resources::material::c4_name_key(name))
                    .and_then(MaterialTextureSurface::surface32_image)
                    // A failed native ReadPNG leaves a non-null 0x0 surface;
                    // PXS phase arithmetic then divides by zero. Contain that
                    // undefined case as the old-style fallback.
                    .filter(|image| image.width() != 0 && image.height() != 0)
            })
            .map(|texture| (texture, rect))
    }

    fn pxs_fixed(particle: &ParticleSnapshot) -> [clonk_engine::math::C4Fixed; 4] {
        particle.pxs_fixed.map_or_else(
            || {
                [
                    clonk_engine::math::ftofix(particle.position.x),
                    clonk_engine::math::ftofix(particle.position.y),
                    clonk_engine::math::ftofix(particle.velocity.x),
                    clonk_engine::math::ftofix(particle.velocity.y),
                ]
            },
            |raw| raw.map(clonk_engine::math::C4Fixed::from_raw),
        )
    }

    fn draw_old_style_pxs(
        &mut self,
        particle: &ParticleSnapshot,
        material: &MaterialRenderInfo,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let [x, y, xdir, ydir] = Self::pxs_fixed(particle);
        let moving = clonk_engine::math::fixtoi(xdir) != 0 || clonk_engine::math::fixtoi(ydir) != 0;
        let mut transparency = i32::from(material.alpha[0]);
        if moving {
            let len = clonk_engine::math::fixtoi(xdir.abs() + ydir.abs()).max(1);
            transparency = transparency.max(195 - (195 - transparency) / len);
        }
        let color = Color::new(
            material.color[0],
            material.color[1],
            material.color[2],
            255u8.saturating_sub(transparency as u8),
        )
        .modulate(lighting);
        let screen = |wx: f32, wy: f32| {
            (
                (wx - self.viewport_x) * self.viewport_zoom,
                (wy - self.viewport_y) * self.viewport_zoom,
            )
        };
        let end = screen(x.to_float(), y.to_float());
        let fog = self.fog_draw_context();
        if moving {
            let start = screen((x - xdir).to_float(), (y - ydir).to_float());
            draw_pxs_line(&mut self.surface, start, end, color, gamma, fog.as_ref());
        } else {
            // DrawPix samples ClrModMap at the original float vertex (cast
            // toward zero) before the raster coordinate is rounded.
            let color = fog
                .as_ref()
                .map_or(color, |fog| fog.color_at_point(color, end.0, end.1));
            draw_pxs_pixel(
                &mut self.surface,
                end.0,
                end.1,
                color,
                gamma,
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_graphical_pxs(
        &mut self,
        particle: &ParticleSnapshot,
        material: &MaterialRenderInfo,
        texture: &ImageData,
        rect: [i32; 6],
        slot: usize,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let [x, y, _, _] = Self::pxs_fixed(particle);
        let facet_width = rect[2];
        let facet_height = rect[3];
        let phases_x = texture.width() as i32 / facet_width;
        let phases_y = texture.height() as i32 / facet_height;
        if phases_x <= 0 || phases_y <= 0 {
            self.draw_old_style_pxs(particle, material, lighting, gamma);
            return;
        }
        let phase_count = (phases_x * phases_y).max(1) as usize;
        let z = 1
            + (((slot / phase_count) ^ 341) % material.pxs_gfx_size as usize) as i32;
        let phase_x = (slot % phases_x as usize) as i32;
        let phase_y = ((slot / phases_x as usize) % phases_y as usize) as i32;
        let world_x = clonk_engine::math::fixtoi(x) + z * rect[4] / facet_width;
        let world_y = clonk_engine::math::fixtoi(y) + z * rect[5] / facet_width;
        let draw_height = z * facet_height / facet_width;
        if draw_height <= 0 {
            return;
        }
        let target = GuiRect::from_origin_size(
            GuiPoint::new(
                (world_x as f32 - self.viewport_x) * self.viewport_zoom,
                (world_y as f32 - self.viewport_y) * self.viewport_zoom,
            ),
            GuiSize::new(
                z as f32 * self.viewport_zoom,
                draw_height as f32 * self.viewport_zoom,
            ),
        );
        let source = SourceRect::new(
            rect[0] + facet_width * phase_x,
            rect[1] + facet_height * phase_y,
            facet_width,
            facet_height,
        );
        let facet_third = (facet_width / 3).max(1);
        // C++ stores transparency in the high byte. The signed expression
        // intentionally narrows to that byte after its <=255 cap
        // (C4PXS.cpp:300-304; StdGL.cpp:437-469).
        let modulation_transparency = ((facet_third - z) * 16).min(255) as u8;
        let fog = self.fog_draw_context();
        let renderer_config = self.advanced_renderer_config;
        draw_pxs_image_region(
            &mut self.surface,
            &target,
            texture,
            &source,
            modulation_transparency,
            lighting,
            renderer_config,
            gamma,
            fog.as_ref(),
        );
    }

    /// Per-pixel landscape rendering from the sim plane: every pixel
    /// byte samples its texmap texture png tiled by WORLD coordinates —
    /// the same composition C4Landscape::MapToSurface bakes into
    /// Surface32. Returns false when no plane/textures exist (legacy
    /// column painter takes over).
    pub(crate) fn draw_ground_textured(
        &mut self,
        landscape: Option<&Landscape>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let visible_pixels = (self.surface_width as usize)
            .saturating_mul(self.surface_height as usize);
        self.draw_ground_textured_with_parallel_rows(
            landscape,
            gamma,
            visible_pixels >= PARALLEL_LANDSCAPE_MIN_PIXELS,
        )
    }

    pub(crate) fn draw_ground_textured_with_parallel_rows(
        &mut self,
        landscape: Option<&Landscape>,
        gamma: Option<&clonk_graphics::GammaRamp>,
        parallel_rows: bool,
    ) -> bool {
        let Some(landscape) = landscape else {
            return false;
        };
        let Some(grid) = landscape.pixel_grid() else {
            return false;
        };
        let blit = SpriteBlitState {
            mode: 0,
            modulation: (landscape.modulation() != 0).then_some(landscape.modulation()),
            fog_modulation: None,
            renderer_config: self.advanced_renderer_config,
        };
        if !grid.has_surface32_pixels()
            && (self.material_textures.is_empty() || self.material_render_info.is_empty())
        {
            return false;
        }
        enum CacheUpdate {
            Reuse,
            Patch {
                rects: Vec<clonk_engine::landscape::PixelGridDirtyRect>,
                surface8_changed: bool,
            },
            Rebuild,
        }
        let width = grid.width();
        let height = grid.height();
        let shade_materials = landscape.shade_materials();
        let border_state = (
            landscape.left_open(),
            landscape.right_open(),
            landscape.top_open(),
            landscape.bottom_open(),
            landscape.grid_vehicle_byte(),
        );
        let expected_bytes = width as usize * height as usize * 4;
        let update = match self.landscape_cache.as_ref() {
            None => CacheUpdate::Rebuild,
            Some(cache)
                if (cache.width, cache.height) != (width, height)
                    || cache.pixels.len() != expected_bytes
                    || cache.shade_materials != shade_materials
                    || cache.border_state != border_state =>
            {
                CacheUpdate::Rebuild
            }
            Some(cache) => match grid.render_dirty_rects_since(&cache.grid) {
                Some(rects) if rects.is_empty() => CacheUpdate::Reuse,
                Some(rects) => CacheUpdate::Patch {
                    rects,
                    surface8_changed: grid.bytes().as_ptr() != cache.grid.bytes().as_ptr(),
                },
                None => CacheUpdate::Rebuild,
            },
        };
        if !matches!(&update, CacheUpdate::Reuse) {
            let regions = match update {
                CacheUpdate::Reuse => unreachable!(),
                CacheUpdate::Patch {
                    rects,
                    surface8_changed,
                } => rects
                    .into_iter()
                    .map(|rect| {
                        if !shade_materials || !surface8_changed {
                            return (rect.x(), rect.y(), rect.width(), rect.height());
                        }
                        let x = rect.x().saturating_sub(1);
                        let y = rect.y().saturating_sub(8);
                        let right = rect
                            .x()
                            .saturating_add(rect.width())
                            .saturating_add(1)
                            .min(width);
                        let bottom = rect
                            .y()
                            .saturating_add(rect.height())
                            .saturating_add(8)
                            .min(height);
                        (x, y, right.saturating_sub(x), bottom.saturating_sub(y))
                    })
                    .collect(),
                CacheUpdate::Rebuild => {
                    self.landscape_cache = Some(LandscapeRenderCache::new(
                        grid.clone(),
                        width,
                        height,
                        shade_materials,
                        border_state,
                    ));
                    vec![(0, 0, width, height)]
                }
            };
            let bytes = grid.bytes();
            let has_surface32_pixels = grid.has_surface32_pixels();
            let textures = grid.texture_names();
            let materials = grid.material_names();
            let material_textures = Arc::clone(&self.material_textures);
            let material_render_info = Arc::clone(&self.material_render_info);
            let mut placements: Vec<i32> = (0..128usize)
                .map(|index| {
                    materials
                        .get(index)
                        .and_then(|name| name.as_deref())
                        .and_then(|name| {
                            material_render_info.get(&clonk_resources::material::c4_name_key(name))
                        })
                        .map_or(0, |material| material.placement)
                })
                .collect();
            // UpdatePixMaps forces sky to zero even if slot zero happens to
            // carry stale material metadata.
            placements[0] = 0;
            let liquid_slots = std::array::from_fn::<_, 128, _>(|index| {
                (25..50).contains(&grid.density_of_byte(index as u8))
            });
            // Per texmap slot: C4TexMapEntry's primary pattern plus the
            // material's secondary pattern.
            enum Slot<'a> {
                Empty,
                Patterns {
                    material: &'a MaterialRenderInfo,
                    texture: &'a MaterialTextureSurface,
                    overlay: Option<&'a MaterialTextureSurface>,
                },
            }
            let slots: Vec<Slot> = (0..128usize)
                .map(|index| {
                    let Some(material) = materials
                        .get(index)
                        .and_then(|name| name.as_deref())
                        .and_then(|name| {
                            material_render_info.get(&clonk_resources::material::c4_name_key(name))
                        })
                    else {
                        return Slot::Empty;
                    };
                    let resolve_texture = |name: &str| {
                        let name = if (25..50).contains(&material.density)
                            && clonk_resources::material::c4_names_equal(name, "Smooth")
                        {
                            clonk_resources::material::c4_name_key("Liquid")
                        } else {
                            clonk_resources::material::c4_name_key(name)
                        };
                        material_textures.get(&name)
                    };
                    let Some(texture) = textures
                        .get(index)
                        .and_then(|name| name.as_deref())
                        .and_then(resolve_texture)
                    else {
                        return Slot::Empty;
                    };
                    let overlay_name = material
                        .texture_overlay
                        .as_deref()
                        .filter(|name| {
                            material_textures
                                .contains_key(&clonk_resources::material::c4_name_key(name))
                        })
                        .unwrap_or("Smooth");
                    Slot::Patterns {
                        material,
                        texture,
                        overlay: resolve_texture(overlay_name),
                    }
                })
                .collect();
            // C4Landscape::GetPix/GetPlacement are inline array lookups in
            // the native relight loop. Keep the same border rules local to
            // this hot composition pass instead of crossing the crate
            // boundary six times for every shaded pixel.
            let byte_with_border = |x: i32, y: i32| {
                let (left_open, right_open, top_open, bottom_open, vehicle) = border_state;
                let border = |is_open: bool| is_open.then_some(0).or(vehicle);
                if x < 0 {
                    return border(y < left_open);
                }
                if x as u32 >= width {
                    return border(y < right_open);
                }
                if y < 0 {
                    return border(top_open);
                }
                if y as u32 >= height {
                    return border(bottom_open);
                }
                Some(bytes[y as usize * width as usize + x as usize])
            };
            let placement_at = |x: i32, y: i32| {
                byte_with_border(x, y).map_or(0, |byte| placements[usize::from(byte & 0x7f)])
            };
            let cache = self
                .landscape_cache
                .as_mut()
                .expect("rebuild installs cache and patch retains it");
            let cache_pixels = Arc::make_mut(&mut cache.pixels);
            let liquid_mask = Arc::make_mut(&mut cache.liquid_mask);
            for &(region_x, region_y, region_width, region_height) in &regions {
                let region_width = region_width as usize;
                let region_height = region_height as usize;
                let column_bytes = region_height.saturating_mul(5);
                let patch_bytes = region_width.saturating_mul(column_bytes);
                cache.composition_scratch.resize(patch_bytes, 0);
                let patch = cache.composition_scratch.as_mut_slice();
                let compose_column = |column_index: usize, column: &mut [u8]| {
                    let x = region_x as i32 + column_index as i32;
                    let first_y = region_y as i32;
                    let mut above_density = 0;
                    let mut below_density = 0;
                    if shade_materials {
                        for offset in 1..=8 {
                            above_density += placement_at(x, first_y - offset - 1);
                            below_density += placement_at(x, first_y + offset - 1);
                        }
                    }
                    for (row_index, pixel) in column.chunks_exact_mut(5).enumerate() {
                        let y = region_y as i32 + row_index as i32;
                        if shade_materials {
                            // Slide to the eight rows immediately above and
                            // below this pixel before testing sky, exactly as
                            // C4Landscape::ApplyLighting does.
                            above_density -= placement_at(x, y - 9);
                            above_density += placement_at(x, y - 1);
                            below_density -= placement_at(x, y);
                            below_density += placement_at(x, y + 8);
                        }
                        let (output, liquid) = pixel.split_at_mut(4);
                        output.fill(0);
                        let byte = bytes[y as usize * width as usize + x as usize];
                        liquid[0] =
                            u8::from(liquid_slots[usize::from(byte & 0x7f)]).saturating_mul(255);
                        if has_surface32_pixels {
                            if let Some(color) = grid.surface32_pixel_at(x, y) {
                                let [red, green, blue, transparency] = split_c4_color(color);
                                output.copy_from_slice(&[
                                    red,
                                    green,
                                    blue,
                                    255u8.saturating_sub(transparency),
                                ]);
                                continue;
                            }
                        }
                        // Pixel zero is sky. C4Landscape::GetClrByTex only
                        // applies material patterns when `pix` is nonzero
                        // (C4Landscape.cpp:2622-2632).
                        if byte == 0 {
                            continue;
                        }
                        let index = (byte & 0x7f) as usize;
                        match &slots[index] {
                            Slot::Empty => {}
                            Slot::Patterns {
                                material,
                                texture,
                                overlay,
                            } => {
                                let mut color = compose_material_surface_pixel(
                                    material,
                                    byte,
                                    x,
                                    y,
                                    (*texture).into(),
                                    (*overlay).map(Into::into),
                                );
                                if shade_materials {
                                    let mut own_density = placements[index];
                                    if own_density == 0 {
                                        continue;
                                    }
                                    own_density = (2 * own_density
                                        + placement_at(x - 1, y)
                                        + placement_at(x + 1, y))
                                        / 4;
                                    let compare_density = above_density / 8;
                                    if own_density > compare_density {
                                        lighten_material_color(
                                            &mut color,
                                            (2 * (own_density - compare_density)).min(30),
                                        );
                                    } else if own_density < compare_density && own_density < 30 {
                                        darken_material_color(
                                            &mut color,
                                            (2 * (compare_density - own_density)).min(30),
                                        );
                                    }
                                    let compare_density = below_density / 8;
                                    if own_density > compare_density {
                                        darken_material_color(
                                            &mut color,
                                            (2 * (own_density - compare_density)).min(30),
                                        );
                                    }
                                }
                                output.copy_from_slice(&[color.r, color.g, color.b, color.a]);
                            }
                        }
                    }
                };
                let parallel_region = parallel_rows
                    && region_width.saturating_mul(region_height) >= PARALLEL_LANDSCAPE_MIN_PIXELS;
                if parallel_region {
                    patch
                        .par_chunks_mut(column_bytes)
                        .enumerate()
                        .for_each(|(column_index, column)| compose_column(column_index, column));
                } else {
                    patch
                        .chunks_mut(column_bytes)
                        .enumerate()
                        .for_each(|(column_index, column)| compose_column(column_index, column));
                }

                let row_bytes = width as usize * 4;
                let mask_row_bytes = width as usize;
                let copy_row = |row_index: usize, output: &mut [u8], mask: &mut [u8]| {
                    for column_index in 0..region_width {
                        let source = column_index * column_bytes + row_index * 5;
                        let destination = (region_x as usize + column_index) * 4;
                        output[destination..destination + 4]
                            .copy_from_slice(&patch[source..source + 4]);
                        mask[region_x as usize + column_index] = patch[source + 4];
                    }
                };
                if parallel_region {
                    cache_pixels
                        .par_chunks_mut(row_bytes)
                        .zip(liquid_mask.par_chunks_mut(mask_row_bytes))
                        .skip(region_y as usize)
                        .take(region_height)
                        .enumerate()
                        .for_each(|(row_index, (output, mask))| copy_row(row_index, output, mask));
                } else {
                    cache_pixels
                        .chunks_mut(row_bytes)
                        .zip(liquid_mask.chunks_mut(mask_row_bytes))
                        .skip(region_y as usize)
                        .take(region_height)
                        .enumerate()
                        .for_each(|(row_index, (output, mask))| copy_row(row_index, output, mask));
                }
            }
            cache.record_gpu_update(&regions);
        }
        let fog = self.fog_draw_context();
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let fog_sampler = fog.as_ref().and_then(|fog| {
            FogSpriteSampler::new(
                fog,
                (
                    0.0,
                    0.0,
                    self.surface_width as f32,
                    self.surface_height as f32,
                ),
                (
                    self.viewport_x,
                    self.viewport_y,
                    self.surface_width as f32 / zoom,
                    self.surface_height as f32 / zoom,
                ),
                (width, height),
                false,
                |x, y| (x, y),
            )
        });
        let fog_raster_axes = fog_sampler.as_ref().map(|sampler| {
            let offset = self.advanced_renderer_config.destination_offset();
            sampler.raster_axes_with_destination_offset(
                self.surface_width,
                self.surface_height,
                offset,
                offset,
            )
        });
        let liquid_animation = self.liquid_animation_image.as_ref().map(|image| {
            let modulation = self.liquid_animation_cycle.advance();
            (image.clone(), modulation)
        });
        let Some(cache) = self.landscape_cache.as_mut() else {
            return false;
        };
        // Anchor the exact byte-plane generation presented by this snapshot.
        // The next engine mutation then starts a new COW dirty generation.
        cache.grid = grid.clone();
        if record_gpu_landscape(
            &mut self.surface,
            cache,
            self.surface_width,
            self.surface_height,
            self.viewport_x,
            self.viewport_y,
            zoom,
            blit,
            fog.as_ref(),
            fog_sampler.as_ref(),
            liquid_animation
                .as_ref()
                .map(|(image, phase)| (image, *phase)),
            gamma.is_some_and(|gamma| !gamma.is_passthrough()),
        ) {
            return true;
        }
        if self.surface.is_gpu_scene_capture_active() {
            // A retained landscape can reject pathological fog chunking.
            // Let draw_ground select its retained column-source fallback;
            // never precompose the row renderer against stale CPU bytes.
            return false;
        }
        let cache_width = cache.width as i32;
        let cache_height = cache.height as i32;
        let cache_pixels = &cache.pixels;
        let destination_offset = self.advanced_renderer_config.destination_offset();
        let texture_size = cpp_tex_size(cache.width, cache.height) as i32;
        let x_samples = (0..self.surface_width)
            .map(|screen_x| {
                let destination_x = screen_x as f32 + 0.5 - destination_offset;
                let raw = if destination_x < 0.0 || destination_x >= self.surface_width as f32 {
                    f32::NAN
                } else {
                    self.viewport_x + destination_x / zoom
                };
                LandscapeXSample::new(raw, texture_size)
            })
            .collect::<Vec<_>>();
        let row_context = LandscapeRowRenderContext {
            grid,
            cache_pixels,
            cache_width,
            cache_height,
            screen_width: self.surface_width,
            screen_height: self.surface_height,
            viewport_y: self.viewport_y,
            zoom,
            texture_size,
            x_samples: &x_samples,
            blit,
            fog: fog.as_ref(),
            fog_sampler: fog_sampler.as_ref(),
            fog_axes: fog_raster_axes
                .as_ref()
                .map(|(x_samples, y_samples)| (x_samples.as_slice(), y_samples.as_slice())),
            liquid_animation: liquid_animation
                .as_ref()
                .map(|(image, modulation)| (image, *modulation)),
            gamma,
            clip: self.surface.clip(),
            #[cfg(test)]
            destination_samples: LANDSCAPE_DESTINATION_SAMPLES
                .with(|samples| Arc::clone(samples)),
        };
        draw_ground_textured_rows(
            &row_context,
            self.surface.pixels_mut(),
            parallel_rows,
        );
        true
    }

    /// Column-only fixtures predate Surface8 but remain reachable in normal
    /// gameplay (including the built-in sandbox). Build retained ground and
    /// liquid source layers, then let the ordinary GPU sprite path apply the
    /// same landscape modulation, FoW, gamma, clipping and alpha composition
    /// as the scalar oracle. This uploads source layers rather than a completed
    /// framebuffer and preserves their texture identities across frames.
    fn capture_column_landscape_fallback(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        if !self.surface.is_gpu_scene_capture_active()
            || self.surface_width == 0
            || self.surface_height == 0
        {
            return false;
        }

        let width = self.surface_width;
        let height = self.surface_height;
        let format = PixelFormat::Rgba8888;
        let reuse_layer = |layer: Option<Surface>| {
            layer
                .filter(|surface| {
                    surface.width() == width
                        && surface.height() == height
                        && surface.format() == format
                })
                .unwrap_or_else(|| Surface::new(width, height, format))
        };
        let mut ground = reuse_layer(self.column_ground_cache.take());
        let mut liquid = reuse_layer(self.column_liquid_cache.take());
        ground.pixels_mut().fill(0);
        liquid.pixels_mut().fill(0);

        let ground_color = Self::apply_lighting(
            Self::ground_color_for_temperature(ambient_temperature),
            lighting,
        );
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let surface_height = height as i32;
        let max_world_x = self.world_width.saturating_sub(1).max(0);
        let ground_pixels = ground.pixels_mut();
        for screen_x in 0..width {
            let world_x = self.viewport_x + (screen_x as f32 + 0.5) / zoom;
            let world_x = (world_x.floor() as i32).clamp(0, max_world_x);
            let ground_world = self.ground_height_at(landscape, world_x);
            let ground_screen = ((ground_world as f32 - self.viewport_y) * zoom)
                .round()
                .clamp(0.0, surface_height as f32) as u32;
            for screen_y in ground_screen..height {
                let offset = (screen_y as usize * width as usize + screen_x as usize) * 4;
                ground_pixels[offset..offset + 4].copy_from_slice(&[
                    ground_color.r,
                    ground_color.g,
                    ground_color.b,
                    ground_color.a,
                ]);
            }
        }

        let mut has_liquid = false;
        if let Some(landscape) = landscape {
            let liquid_color = Self::apply_lighting(
                Self::liquid_color_for_temperature(ambient_temperature),
                lighting,
            );
            let liquid_pixels = liquid.pixels_mut();
            for (world_x, column) in landscape.liquids().iter().enumerate() {
                let screen_x = ((world_x as f32 - self.viewport_x) * zoom).round() as i32;
                if screen_x < 0 || screen_x >= width as i32 {
                    continue;
                }
                for segment in column.segments() {
                    let mut start =
                        ((segment.top as f32 - self.viewport_y) * zoom).round() as i32;
                    let mut end =
                        ((segment.bottom as f32 - self.viewport_y) * zoom).round() as i32;
                    if start > end {
                        std::mem::swap(&mut start, &mut end);
                    }
                    if end < 0 || start >= surface_height {
                        continue;
                    }
                    start = start.max(0);
                    end = end.min(surface_height - 1);
                    has_liquid = true;
                    for screen_y in start..=end {
                        let offset =
                            (screen_y as usize * width as usize + screen_x as usize) * 4;
                        liquid_pixels[offset..offset + 4].copy_from_slice(&[
                            liquid_color.r,
                            liquid_color.g,
                            liquid_color.b,
                            liquid_color.a,
                        ]);
                    }
                }
            }
        }

        let blit = SpriteBlitState {
            mode: 0,
            modulation: landscape
                .map(Landscape::modulation)
                .filter(|modulation| *modulation != 0),
            fog_modulation: None,
            renderer_config: self.advanced_renderer_config,
        };
        let fog = self.fog_draw_context();
        let dest = (0.0, 0.0, width as f32, height as f32);
        let source = FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
        };
        let transform = GraphicsTransform::identity();
        let mut capture_layer = |layer: &Surface| {
            let resource = layer.gpu_texture_resource();
            let image = ImageData::transient_from_arc(
                width,
                height,
                Arc::clone(&resource.pixels),
            );
            capture_gpu_sprite_with_resource(
                &mut self.surface,
                dest,
                dest,
                &transform,
                &image,
                None,
                source,
                false,
                None,
                blit,
                gamma,
                fog.as_ref(),
                GpuSampler::Nearest,
                false,
                Some(resource),
            )
        };
        let captured = capture_layer(&ground) && (!has_liquid || capture_layer(&liquid));
        self.column_ground_cache = Some(ground);
        self.column_liquid_cache = Some(liquid);
        captured
    }

    pub(crate) fn draw_ground(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        if self.debug_draw_flags.show_solid_mask && self.draw_ground_surface8(landscape, gamma) {
            return true;
        }
        if self.draw_ground_textured(landscape, gamma) {
            return true;
        }
        if self.capture_column_landscape_fallback(
            ambient_temperature,
            landscape,
            lighting,
            gamma,
        ) {
            return true;
        }
        let ground_color = Self::apply_lighting(
            Self::ground_color_for_temperature(ambient_temperature),
            lighting,
        );
        let blit = SpriteBlitState {
            mode: 0,
            modulation: landscape
                .map(Landscape::modulation)
                .filter(|modulation| *modulation != 0),
            fog_modulation: None,
            renderer_config: self.advanced_renderer_config,
        };
        let source = prepare_sprite_fragment(ground_color, None, None, blit);
        if source.alpha() == 0.0 {
            return false;
        }
        let fog = self.fog_box_sampler(true);
        let opaque_output = (fog.is_none() && source.alpha() == 255.0)
            .then(|| composite_sprite_fragment(source, Color::transparent(), blit, gamma));
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let surface_height = self.surface_height as i32;
        let max_world_x = self.world_width.saturating_sub(1).max(0);
        for screen_x in 0..self.surface_width {
            let pixel_center = screen_x as f32 + 0.5;
            let world_x = self.viewport_x + pixel_center / zoom;
            let world_x_index = world_x.floor() as i32;
            let world_x_index = world_x_index.clamp(0, max_world_x);
            let ground_world = self.ground_height_at(landscape, world_x_index);
            let mut ground_screen = ((ground_world as f32 - self.viewport_y) * zoom).round() as i32;
            if ground_screen < 0 {
                ground_screen = 0;
            }
            if ground_screen >= surface_height {
                continue;
            }
            for y in ground_screen..surface_height {
                if let Some(output) = opaque_output {
                    let _ = self.surface.set_pixel(screen_x, y as u32, output);
                } else {
                    let pixel_blit = fog.as_ref().map_or(blit, |(fog, sampler)| {
                        fog_sprite_blit_at(
                            sampler.as_ref(),
                            Some(fog),
                            blit,
                            (screen_x as f32 + 0.5) / self.surface_width as f32,
                            (y as f32 + 0.5) / self.surface_height as f32,
                            screen_x as i32,
                            y,
                        )
                    });
                    let source = prepare_sprite_fragment(ground_color, None, None, pixel_blit);
                    blend_prepared_sprite_fragment(
                        &mut self.surface,
                        screen_x,
                        y as u32,
                        source,
                        pixel_blit,
                        gamma,
                    );
                }
            }
        }
        false
    }

    /// `C4Landscape::Draw`'s ShowSolidMask branch blits the live Surface8
    /// byte plane through its live Mat2Pal palette instead of the
    /// material-textured Surface32. Blit8Fast bypasses landscape modulation
    /// and ClrModMap; object masks are already baked into this plane.
    pub(crate) fn draw_ground_surface8(
        &mut self,
        landscape: Option<&Landscape>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some(landscape) = landscape else {
            return false;
        };
        let Some(grid) = landscape.pixel_grid() else {
            return false;
        };
        let width = grid.width() as i32;
        let height = grid.height() as i32;
        let bytes = grid.bytes();
        let mut palette = [Color::opaque(0, 0, 0); 256];
        for (index, color) in palette.iter_mut().enumerate() {
            let (source, source_transparency, material) =
                landscape.surface8_palette_entry(index as u8);
            let source_color = if landscape.mode() == clonk_engine::landscape::LANDSCAPE_MODE_EXACT {
                Color::new(
                    source[0],
                    source[1],
                    source[2],
                    255_u8.saturating_sub(source_transparency),
                )
            } else {
                // Generated Surface8 copies the active loader-resolved DDraw
                // palette; Exact mode instead owns the BMP palette retained
                // by LandscapeRasterState.
                self.game_palette.color(index as u8)
            };
            *color = material
                .and_then(|name| {
                    self.material_render_info
                        .get(&clonk_resources::material::c4_name_key(name))
                })
                .map_or(source_color, |material| {
                    Color::new(
                        material.color[0],
                        material.color[1],
                        material.color[2],
                        255_u8.saturating_sub(material.alpha[if index >= 128 { 3 } else { 0 }]),
                    )
                });
        }
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        for screen_y in 0..self.surface_height {
            let world_y = (self.viewport_y + (screen_y as f32 + 0.5) / zoom).floor() as i32;
            if world_y < 0 || world_y >= height {
                continue;
            }
            for screen_x in 0..self.surface_width {
                let world_x = (self.viewport_x + (screen_x as f32 + 0.5) / zoom).floor() as i32;
                if world_x < 0 || world_x >= width {
                    continue;
                }
                let byte = bytes[(world_y * width + world_x) as usize];
                if byte == 0 {
                    continue;
                }
                draw_pxs_pixel(
                    &mut self.surface,
                    screen_x as f32,
                    screen_y as f32,
                    palette[usize::from(byte)],
                    gamma,
                    None,
                );
            }
        }
        true
    }

    pub(crate) fn draw_liquids(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let Some(landscape) = landscape else {
            return;
        };
        if landscape.liquids().is_empty() {
            return;
        }

        let base_color = Self::apply_lighting(
            Self::liquid_color_for_temperature(ambient_temperature),
            lighting,
        );
        let blit = SpriteBlitState {
            mode: 0,
            modulation: (landscape.modulation() != 0).then_some(landscape.modulation()),
            fog_modulation: None,
            renderer_config: self.advanced_renderer_config,
        };
        let source = prepare_sprite_fragment(base_color, None, None, blit);
        if source.alpha() == 0.0 {
            return;
        }

        let fog = self.fog_box_sampler(true);
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let surface_width = self.surface_width as i32;
        let surface_height = self.surface_height as i32;

        for (world_x, column) in landscape.liquids().iter().enumerate() {
            if column.segments().is_empty() {
                continue;
            }

            let screen_x = ((world_x as f32 - self.viewport_x) * zoom).round() as i32;
            if screen_x < 0 || screen_x >= surface_width {
                continue;
            }

            for segment in column.segments() {
                let mut start = ((segment.top as f32 - self.viewport_y) * zoom).round() as i32;
                let mut end = ((segment.bottom as f32 - self.viewport_y) * zoom).round() as i32;
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                }
                if end < 0 || start >= surface_height {
                    continue;
                }
                start = start.max(0);
                end = end.min(surface_height - 1);

                for screen_y in start..=end {
                    let x = screen_x as u32;
                    let y = screen_y as u32;
                    let pixel_blit = fog.as_ref().map_or(blit, |(fog, sampler)| {
                        fog_sprite_blit_at(
                            sampler.as_ref(),
                            Some(fog),
                            blit,
                            (screen_x as f32 + 0.5) / self.surface_width as f32,
                            (screen_y as f32 + 0.5) / self.surface_height as f32,
                            screen_x,
                            screen_y,
                        )
                    });
                    let source = prepare_sprite_fragment(base_color, None, None, pixel_blit);
                    blend_prepared_sprite_fragment(
                        &mut self.surface,
                        x,
                        y,
                        source,
                        pixel_blit,
                        gamma,
                    );
                }
            }
        }
    }

    /// C4Object::IsVisible (src/C4Object.cpp:5600-5629). This is shared by
    /// rendering and FindVisObject-style mouse picking so hidden HUD helpers,
    /// spell targets, and layer-gated objects cannot leak through either path.
    pub(crate) fn object_is_visible(
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        object: &ObjectSnapshot,
        for_player: i32,
        as_overlay: bool,
    ) -> bool {
        object_visible_for_player(objects, players, object, for_player, as_overlay)
    }

    /// C4Object::TargetPos / ApplyParallaxity
    /// (src/C4Object.h:377-380; C4Object.cpp:5800-5814). The viewport target
    /// and extent are logical pixels even when the output surface is scaled.
    pub(crate) fn object_target_position(&self, object: &ObjectSnapshot) -> (f32, f32) {
        if object.category & CATEGORY_PARALLAX_FLAG == 0 {
            return (self.viewport_x, self.viewport_y);
        }
        let local = |name| {
            object
                .local_vars
                .get(name)
                .and_then(|value| value.as_c4_int())
                .unwrap_or(0)
        };
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let width = ((self.surface_width as f32 / zoom).ceil() as i32).max(1);
        let height = ((self.surface_height as f32 / zoom).ceil() as i32).max(1);
        let apply = |target: f32, parallax: i32, position: i32, extent: i32| {
            if parallax == 0 && position < 0 {
                -extent
            } else {
                (target as i32).wrapping_mul(parallax) / 100
            }
        };
        (
            apply(
                self.viewport_x,
                local("__local_0"),
                object.position.x,
                width,
            ) as f32,
            apply(
                self.viewport_y,
                local("__local_1"),
                object.position.y,
                height,
            ) as f32,
        )
    }

    fn audibility_facet_for_pass(&self, pass: ObjectRenderPass) -> Option<AudibilityFacet> {
        match pass {
            ObjectRenderPass::ForegroundParallax => self.full_audibility_facet,
            ObjectRenderPass::Background
            | ObjectRenderPass::Normal
            | ObjectRenderPass::ForegroundNonParallax => self.content_audibility_facet,
        }
    }

    /// Integer `C4Object::TargetPos` for the exact facet that produced an
    /// audibility call. This remains separate from the float output transform:
    /// native sound uses logical facet coordinates and integer division.
    fn object_audibility_target_position(
        object: &ObjectSnapshot,
        facet: AudibilityFacet,
    ) -> Vector2 {
        if object.category & CATEGORY_PARALLAX_FLAG == 0 {
            return Vector2::new(facet.target_x, facet.target_y);
        }
        let local = |name| {
            object
                .local_vars
                .get(name)
                .and_then(|value| value.as_c4_int())
                .unwrap_or(0)
        };
        let apply = |target: i32, parallax: i32, position: i32, extent: i32| {
            if parallax == 0 && position < 0 {
                -extent
            } else {
                target.wrapping_mul(parallax) / 100
            }
        };
        Vector2::new(
            apply(
                facet.target_x,
                local("__local_0"),
                object.position.x,
                facet.width,
            ),
            apply(
                facet.target_y,
                local("__local_1"),
                object.position.y,
                facet.height,
            ),
        )
    }

    fn record_audibility_at(&mut self, object: &ObjectSnapshot, point: Vector2) {
        let Some(facet) = self.current_audibility_facet else {
            return;
        };
        let call = if object.category & CATEGORY_PARALLAX_FLAG != 0 {
            let target = Self::object_audibility_target_position(object, facet);
            RenderedAudibilityCall::Parallax {
                point,
                rendered_center: Vector2::new(
                    target.x.wrapping_add(facet.width / 2),
                    target.y.wrapping_add(facet.height / 2),
                ),
            }
        } else {
            RenderedAudibilityCall::World { point }
        };
        self.rendered_object_audibility_calls
            .entry(object.id)
            .or_default()
            .push(call);
    }

    fn record_line_audibility_calls(&mut self, object: &ObjectSnapshot) {
        let Some(first) = object.vertices.first() else {
            return;
        };
        let last = object
            .vertices
            .last()
            .expect("a first line vertex implies a last line vertex");
        self.record_audibility_at(object, Vector2::new(first.x, first.y));
        self.record_audibility_at(object, Vector2::new(last.x, last.y));
    }

    fn record_direct_audibility_calls(&mut self, object: &ObjectSnapshot, line: i32) {
        if line != 0 {
            self.record_line_audibility_calls(object);
        } else if object.category & CATEGORY_PARALLAX_FLAG != 0 {
            self.record_audibility_at(object, object.position);
        }
    }

    /// Native output-boundary gate, which runs after the back list but before
    /// command debug, ShowSolidMask, containment and IgnoreFoW suppression.
    fn object_output_bounds_culled(&self, object: &ObjectSnapshot) -> bool {
        let (base_definition_id, base_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(
                &base_definition_id,
                base_graphics_name.as_deref(),
            ))
            .cloned();
        if sprite.is_none() && base_graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&base_definition_id, None))
                .cloned();
        }
        if sprite.is_none() && base_definition_id != object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let geometry_sprite = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
            .cloned()
            .or(sprite);
        if let Some(geometry_sprite) = geometry_sprite.as_ref() {
            let shape = self.live_object_shape(geometry_sprite, object);
            return !self.object_reaches_post_face_draw(object, geometry_sprite, shape);
        }

        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let screen_x = (object.position.x as f32 - self.viewport_x) * zoom;
        let screen_y = (object.position.y as f32 - self.viewport_y) * zoom;
        screen_x < -10.0
            || screen_y < -10.0
            || screen_x > self.surface_width as f32 + 10.0
            || screen_y > self.surface_height as f32 + 10.0
    }

    #[cfg(test)]
    pub(crate) fn draw_objects(
        &mut self,
        objects: &[ObjectSnapshot],
        render_order: &[ObjectId],
        definition_lines: &HashMap<DefinitionId, DefinitionLineMetadata>,
        players: &[PlayerState],
        for_player: i32,
        lighting: f32,
        owner_colors: &HashMap<i32, Color>,
        pass: ObjectRenderPass,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.draw_objects_at_frame(
            0,
            objects,
            render_order,
            definition_lines,
            &[],
            players,
            for_player,
            lighting,
            owner_colors,
            pass,
            gamma,
        );
    }

    pub(crate) fn draw_objects_at_frame(
        &mut self,
        frame: u64,
        objects: &[ObjectSnapshot],
        render_order: &[ObjectId],
        definition_lines: &HashMap<DefinitionId, DefinitionLineMetadata>,
        particles: &[ParticleSnapshot],
        players: &[PlayerState],
        for_player: i32,
        lighting: f32,
        owner_colors: &HashMap<i32, Color>,
        pass: ObjectRenderPass,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let saved_current_audibility_facet = self.current_audibility_facet;
        self.current_audibility_facet = self.audibility_facet_for_pass(pass);

        // Engine snapshots keep object payloads in canonical ID order, while
        // C4ObjectList draws Last -> Prev in its mutable master-list order
        // (src/C4ObjectList.cpp:387-396). Empty is the legacy snapshot
        // fallback; a partial sidecar appends omitted objects canonically.
        let mut ordered = Vec::with_capacity(objects.len());
        let mut seen = HashSet::with_capacity(objects.len());
        if render_order.is_empty() {
            ordered.extend(objects);
        } else {
            let by_id: HashMap<_, _> = objects.iter().map(|object| (object.id, object)).collect();
            ordered.extend(
                render_order
                    .iter()
                    .filter(|id| seen.insert(**id))
                    .filter_map(|id| by_id.get(id).copied()),
            );
            ordered.extend(objects.iter().filter(|object| seen.insert(object.id)));
        }
        let mut selected = Vec::new();

        for object in ordered {
            if object.status != ObjectStatus::Normal {
                continue;
            }
            if !Self::object_is_visible(objects, players, object, for_player, false) {
                continue;
            }
            match pass {
                ObjectRenderPass::Background => {
                    if object.category & CATEGORY_BACKGROUND_FLAG != 0 {
                        selected.push(object);
                    }
                }
                ObjectRenderPass::Normal => {
                    if object.category & (CATEGORY_BACKGROUND_FLAG | CATEGORY_FOREGROUND_FLAG) == 0
                    {
                        selected.push(object);
                    }
                }
                ObjectRenderPass::ForegroundNonParallax => {
                    if object.category & CATEGORY_FOREGROUND_FLAG != 0
                        && object.category & CATEGORY_PARALLAX_FLAG == 0
                    {
                        selected.push(object);
                    }
                }
                ObjectRenderPass::ForegroundParallax => {
                    if object.category & CATEGORY_FOREGROUND_FLAG != 0
                        && object.category & CATEGORY_PARALLAX_FLAG != 0
                    {
                        selected.push(object);
                    }
                }
            }
        }

        for object in &selected {
            let line = definition_lines
                .get(&object.definition_id)
                .map(|metadata| metadata.line)
                .unwrap_or(0);
            // C4Object::Draw dispatches DrawLine before particles, output
            // bounds and containment. Non-line parallax objects likewise call
            // SetAudibilityAt before their output-boundary gate.
            self.record_direct_audibility_calls(object, line);
            if line == 0 && object.container.is_none() {
                self.draw_definition_particles(
                    particles,
                    &ParticleLayer::ObjectBack(object.id),
                    Some(object),
                    gamma,
                );
            }
            if line == 0 && self.object_output_bounds_culled(object) {
                if object.container.is_none() {
                    self.draw_definition_particles(
                        particles,
                        &ParticleLayer::ObjectFront(object.id),
                        Some(object),
                        gamma,
                    );
                }
                continue;
            }
            if line == 0 && self.debug_draw_flags.show_command {
                self.paint_command_debug(object, objects, gamma);
            }
            if line == 0
                && self.debug_draw_flags.show_solid_mask
                && self.object_has_debug_solid_mask(object)
            {
                continue;
            }
            if line == 0 && object.container.is_some() {
                continue;
            }
            // `C4D_IgnoreFoW` disables the modulation map only around the
            // object's base `Draw` body. Line definitions return before that
            // switch and `DrawTopFace` runs later with the map restored.
            let suppress_fog = line == 0
                && object.category & CATEGORY_IGNORE_FOW_FLAG != 0
                && self.active_fog_map.is_some();
            if suppress_fog {
                self.fog_suppression_depth += 1;
            }
            let reaches_post_face_draw = self.paint_object_with_particles(
                object,
                objects,
                particles,
                players,
                for_player,
                lighting,
                owner_colors,
                line,
                gamma,
            );
            if line == 0 && reaches_post_face_draw {
                self.paint_need_energy_bolt(object, frame, gamma);
                self.paint_object_debug_tail(object, gamma);
            }
            if suppress_fog {
                self.fog_suppression_depth -= 1;
            }
        }
        for object in &selected {
            if !definition_lines
                .get(&object.definition_id)
                .is_some_and(|metadata| metadata.line != 0)
                && object.container.is_some()
            {
                continue;
            }
            // The crew-name block is the first drawing work in
            // C4Object::DrawTopFace, before the construction/TopFace facet.
            // Keeping it in this outer list pass prevents graphics-overlay
            // recursion from producing duplicate labels.
            self.paint_crew_name_label(object, for_player, owner_colors, gamma);
            // Native DrawTopFace emits the crew label before the solid-mask
            // early return suppresses the actual TopFace.
            if self.debug_draw_flags.show_solid_mask && self.object_has_debug_solid_mask(object) {
                continue;
            }
            if !definition_lines
                .get(&object.definition_id)
                .is_some_and(|metadata| metadata.line != 0)
            {
                let blit = self.configured_blit(SpriteBlitState::for_object(object));
                self.paint_object_top_face(object, blit, gamma);
            }
        }

        self.current_audibility_facet = saved_current_audibility_facet;
    }

    fn object_has_debug_solid_mask(&self, object: &ObjectSnapshot) -> bool {
        match object.solid_mask_override {
            Some(mask) => mask.width > 0 && mask.height > 0,
            None => self
                .definition_debug_geometry
                .get(&object.definition_id)
                .and_then(|geometry| geometry.solid_mask)
                .is_some_and(|mask| mask.width > 0 && mask.height > 0),
        }
    }

    fn draw_pathfinder_debug(
        &mut self,
        graph: &clonk_engine::PathfinderDebugSnapshot,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let viewport_x = self.viewport_x;
        let viewport_y = self.viewport_y;
        let screen = |point: Vector2| {
            (
                (point.x as f32 - viewport_x) * zoom,
                (point.y as f32 - viewport_y) * zoom,
            )
        };
        for zone in &graph.zones {
            if zone.width <= 0 || zone.height <= 0 {
                continue;
            }
            let top_left = screen(Vector2::new(zone.x, zone.y));
            self.draw_debug_frame(
                top_left.0,
                top_left.1,
                top_left.0 + (zone.width - 1) as f32 * zoom,
                top_left.1 + (zone.height - 1) as f32 * zoom,
                self.game_palette.color(if zone.used { 11 } else { 10 }),
                gamma,
            );
        }
        for ray in &graph.rays {
            let color_index = if ray.uses_transfer_zone {
                14
            } else {
                match ray.status {
                    clonk_engine::PathfinderDebugRayStatus::Launch
                    | clonk_engine::PathfinderDebugRayStatus::Crawl => 10,
                    clonk_engine::PathfinderDebugRayStatus::Still => 7,
                    clonk_engine::PathfinderDebugRayStatus::Failure => 13,
                    clonk_engine::PathfinderDebugRayStatus::Deleted => 2,
                }
            };
            let start = screen(ray.start);
            let end = screen(ray.end);
            if ray.status == clonk_engine::PathfinderDebugRayStatus::Crawl {
                let (dx, dy) = match ray.crawl_attach {
                    1 => (0.0, -7.0),
                    2 => (7.0, 0.0),
                    3 => (0.0, 7.0),
                    4 => (-7.0, 0.0),
                    _ => (0.0, 0.0),
                };
                self.draw_debug_line(
                    end,
                    (end.0 + dx * zoom, end.1 + dy * zoom),
                    self.game_palette.color(10),
                    gamma,
                );
            }
            let color = self.game_palette.color(color_index);
            self.draw_debug_line(start, end, color, gamma);
            let crawler_color = if ray.status == clonk_engine::PathfinderDebugRayStatus::Crawl {
                self.game_palette
                    .color(if ray.direction < 0 { 11 } else { 14 })
            } else {
                color
            };
            self.draw_debug_frame(
                end.0 - zoom,
                end.1 - zoom,
                end.0 + zoom,
                end.1 + zoom,
                crawler_color,
                gamma,
            );
            let target = screen(ray.target);
            self.draw_debug_frame(
                target.0 - 2.0 * zoom,
                target.1 - 2.0 * zoom,
                target.0 + 2.0 * zoom,
                target.1 + 2.0 * zoom,
                self.game_palette.color(13),
                gamma,
            );
        }
    }

    fn draw_debug_line(
        &mut self,
        start: (f32, f32),
        end: (f32, f32),
        color: Color,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let fog = self.fog_draw_context();
        draw_pxs_line(&mut self.surface, start, end, color, gamma, fog.as_ref());
    }

    fn draw_debug_frame(
        &mut self,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
        color: Color,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.draw_debug_line((left, top), (right, top), color, gamma);
        self.draw_debug_line((right, top), (right, bottom), color, gamma);
        self.draw_debug_line((right, bottom), (left, bottom), color, gamma);
        self.draw_debug_line((left, bottom), (left, top), color, gamma);
    }

    fn object_debug_screen_position(&self, object: &ObjectSnapshot, x: i32, y: i32) -> (f32, f32) {
        let (target_x, target_y) = self.object_target_position(object);
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        ((x as f32 - target_x) * zoom, (y as f32 - target_y) * zoom)
    }

    fn object_debug_name(&self, objects: &[ObjectSnapshot], id: ObjectId) -> String {
        objects
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|candidate| {
                candidate
                    .custom_name
                    .clone()
                    .or_else(|| {
                        self.definition_debug_geometry
                            .get(&candidate.definition_id)
                            .and_then(|geometry| geometry.name.clone())
                    })
                    .unwrap_or_else(|| candidate.definition_id.clone())
            })
            .unwrap_or_else(|| id.as_u64().to_string())
    }

    fn paint_command_debug(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let Some(sprite) = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
        else {
            return;
        };
        let shape = self.live_object_shape(sprite, object);
        if !self.object_reaches_post_face_draw(object, sprite, shape) {
            return;
        }
        let views = object.command_stack.command_views();
        if views.is_empty() {
            return;
        }
        let mut cursor = object.position;
        let mut move_tos = 0usize;
        let mut lines = Vec::new();
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        for command in views {
            let is_move = command.name == "MoveTo";
            let is_transfer = command.name == "Transfer";
            if is_move || is_transfer {
                if let (Some(x), Some(y)) = (command.tx, command.ty) {
                    let start = self.object_debug_screen_position(object, cursor.x, cursor.y);
                    let end = self.object_debug_screen_position(object, x, y);
                    let color = self.game_palette.color(if is_transfer { 11 } else { 10 });
                    self.draw_debug_line(start, end, color, gamma);
                    self.draw_debug_frame(
                        end.0 - zoom,
                        end.1 - zoom,
                        end.0 + zoom,
                        end.1 + zoom,
                        color,
                        gamma,
                    );
                    cursor = Vector2::new(x, y);
                }
                if is_move {
                    move_tos += 1;
                    continue;
                }
            }
            if move_tos != 0 {
                lines.push(format!("{move_tos}x MoveTo"));
                move_tos = 0;
            }
            let target = command
                .target
                .map(|id| self.object_debug_name(objects, id))
                .unwrap_or_default();
            let data_definition_id = match &command.data {
                clonk_engine::command::CommandData::Integer(raw) if *raw != 0 => {
                    Some(clonk_script::c4_id_from_raw(*raw as u32 as usize))
                }
                _ => None,
            };
            let data_id = match &command.data {
                clonk_engine::command::CommandData::Integer(_) => data_definition_id
                    .as_deref()
                    .map(clonk_script::c4_id_text)
                    .unwrap_or_default(),
                clonk_engine::command::CommandData::Text(text) => c4_presentation_text(text),
                _ => String::new(),
            };
            let text = match command.name.as_str() {
                "None" => String::new(),
                "Put" => {
                    let item = command
                        .target2
                        .map(|id| self.object_debug_name(objects, id))
                        .filter(|name| !name.is_empty())
                        .or_else(|| (!data_id.is_empty()).then(|| data_id.clone()))
                        .unwrap_or_else(|| "Content".to_string());
                    format!("Put {item} to {target}")
                }
                "Buy" | "Sell" => {
                    let base = if target.is_empty() {
                        "closest base"
                    } else {
                        target.as_str()
                    };
                    format!("{} {data_id} at {base}", command.name)
                }
                "Acquire" => format!("Acquire {target}"),
                "Call" => {
                    let call = match &command.data {
                        clonk_engine::command::CommandData::Text(text) => c4_presentation_text(text),
                        _ => String::new(),
                    };
                    let target = if target.is_empty() {
                        "(null)"
                    } else {
                        target.as_str()
                    };
                    format!("Call {call} in {target}")
                }
                "Construct" => {
                    let definition_id = data_definition_id
                        .as_deref()
                        .or(command.tx_definition.as_deref());
                    let definition = definition_id
                        .and_then(|definition_id| {
                            self.definition_debug_geometry
                                .get(definition_id)
                                .and_then(|geometry| geometry.name.clone())
                        })
                        .unwrap_or_default();
                    format!("Construct {definition}")
                }
                _ if target.is_empty() => command.name,
                _ => format!("{} {target}", command.name),
            };
            if !text.is_empty() {
                lines.push(if command.finished {
                    format!("<i>{text}</i>")
                } else {
                    text
                });
            }
        }
        if move_tos != 0 {
            lines.push(format!("{move_tos}x MoveTo"));
        }
        if lines.is_empty() {
            return;
        }
        let text = format!("|{}", lines.join("|"));
        let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
        let (_, height) = font.text_extent_markup(&text);
        let anchor = self.object_debug_screen_position(
            object,
            object.position.x,
            object.position.y + shape.y,
        );
        let x = anchor.0.round() as i32;
        let y = anchor.1.round() as i32 - 10 - height;
        let fog = self.fog_draw_context();
        if let Some(fog) = fog.as_ref() {
            draw_fogged_markup_text(
                &mut self.surface,
                &font,
                x,
                y,
                &text,
                Color::opaque(255, 255, 255),
                gamma,
                self.advanced_renderer_config,
                fog,
            );
        } else {
            font.draw_markup_with_gamma(
                &mut self.surface,
                x,
                y,
                &text,
                Color::opaque(255, 255, 255),
                clonk_graphics::clonk_font::TextAlign::Center,
                gamma,
            );
        }
    }

    fn paint_object_debug_tail(
        &mut self,
        object: &ObjectSnapshot,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        if self.debug_draw_flags.show_vertices && object.vertices.len() > 1 {
            for (index, vertex) in object.vertices.iter().enumerate() {
                let center = self.object_debug_screen_position(
                    object,
                    object.position.x + vertex.x,
                    object.position.y + vertex.y,
                );
                if center.0 < zoom
                    || center.1 < zoom
                    || center.0 > self.surface_width as f32 - 2.0 * zoom
                    || center.1 > self.surface_height as f32 - 2.0 * zoom
                {
                    continue;
                }
                let color_index = if vertex.cnat & clonk_engine::CNAT_NO_COLLISION != 0 {
                    14
                } else if object.mobile {
                    10
                } else {
                    13
                };
                let color = self.game_palette.color(color_index);
                self.draw_debug_line(
                    (center.0 - zoom, center.1),
                    (center.0 + zoom, center.1),
                    color,
                    gamma,
                );
                self.draw_debug_line(
                    (center.0, center.1 - zoom),
                    (center.0, center.1 + zoom),
                    color,
                    gamma,
                );
                if object.vertex_contacts.get(index).copied().unwrap_or(0) != 0 {
                    self.draw_debug_frame(
                        center.0 - 2.0 * zoom,
                        center.1 - 2.0 * zoom,
                        center.0 + 2.0 * zoom,
                        center.1 + 2.0 * zoom,
                        self.game_palette.color(6),
                        gamma,
                    );
                }
            }
        }

        if self.debug_draw_flags.show_entrance {
            let geometry = self
                .definition_debug_geometry
                .get(&object.definition_id)
                .cloned()
                .unwrap_or_default();
            for (enabled, rect, color_index) in [
                (
                    object.ocf & clonk_engine::ocf::ENTRANCE != 0,
                    geometry.entrance,
                    14,
                ),
                (
                    object.ocf & clonk_engine::ocf::COLLECTION != 0,
                    geometry.collection,
                    10,
                ),
            ] {
                let Some(rect) = enabled.then_some(rect).flatten() else {
                    continue;
                };
                if rect.width <= 0 || rect.height <= 0 {
                    continue;
                }
                let top_left = self.object_debug_screen_position(
                    object,
                    object.position.x + rect.x,
                    object.position.y + rect.y,
                );
                self.draw_debug_frame(
                    top_left.0,
                    top_left.1,
                    top_left.0 + (rect.width - 1) as f32 * zoom,
                    top_left.1 + (rect.height - 1) as f32 * zoom,
                    self.game_palette.color(color_index),
                    gamma,
                );
            }
        }

        // A physical ActMap slot is exactly native `Action.Act > ActIdle`;
        // unresolved names remain on the built-in idle sentinel.
        let active_action = object.action.act_map_index.is_some();
        if self.debug_draw_flags.show_action && active_action {
            let text = format!("{} ({})", object.action.name, object.action.phase);
            let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
            let (_, height) = font.text_extent_markup(&text);
            let shape_y = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .map(|sprite| self.live_object_shape(sprite, object).y)
                .unwrap_or(0);
            let anchor = self.object_debug_screen_position(
                object,
                object.position.x,
                object.position.y + shape_y,
            );
            let color = if object.in_liquid {
                c4_color_to_surface(0xfa00_00ff)
            } else {
                Color::opaque(255, 255, 255)
            };
            let x = anchor.0.round() as i32;
            let y = anchor.1.round() as i32 - height;
            let fog = self.fog_draw_context();
            if let Some(fog) = fog.as_ref() {
                draw_fogged_markup_text(
                    &mut self.surface,
                    &font,
                    x,
                    y,
                    &text,
                    color,
                    gamma,
                    self.advanced_renderer_config,
                    fog,
                );
            } else {
                font.draw_markup_with_gamma(
                    &mut self.surface,
                    x,
                    y,
                    &text,
                    color,
                    clonk_graphics::clonk_font::TextAlign::Center,
                    gamma,
                );
            }
        }
    }

    /// `C4Object::DrawTopFace`'s crew label
    /// (src/C4Object.cpp:2582-2612). Player existence, invisibility,
    /// hostility and the two display-toggle text variants have already been
    /// projected into [`CrewNameOverlay`]; world/object gates stay here.
    fn paint_crew_name_label(
        &mut self,
        object: &ObjectSnapshot,
        for_player: i32,
        owner_colors: &HashMap<i32, Color>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if !self.viewport_overlays_visible
            || object.ocf & clonk_engine::ocf::CREW_MEMBER == 0
            || object.container.is_some()
        {
            return;
        }
        let Some(label) = self
            .crew_name_labels
            .iter()
            .find(|label| {
                label.object_id == object.id && label.visible_to.contains(&for_player)
            })
            .cloned()
        else {
            return;
        };
        if label.text.is_empty() {
            return;
        }

        // The range gate reads the live object shape; the vertical anchor
        // retains the definition shape even after SetShape/Con changes.
        let Some(definition_sprite) = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
            .cloned()
        else {
            return;
        };
        let shape = self.live_object_shape(&definition_sprite, object);
        let definition_shape = Self::sprite_def_shape(&definition_sprite);
        let (target_x, target_y) = self.object_target_position(object);
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let view_width = ((self.surface_width as f32 / zoom).ceil() as i32).max(1);
        let view_height = ((self.surface_height as f32 / zoom).ceil() as i32).max(1);
        let shape_x = object.position.x + shape.x - target_x as i32;
        let shape_y = object.position.y + shape.y - target_y as i32;
        let inside = |value: i32, minimum: i32, maximum: i32| {
            value >= minimum && value <= maximum
        };
        if !inside(shape_x, 1 - shape.width, view_width)
            || !inside(shape_y, 1 - shape.height, view_height)
        {
            return;
        }

        let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
        let max_line = (view_width / font.text_width("m").max(1)).max(20);
        let text = c4_word_wrap(&label.text, max_line as usize);
        let (text_width, text_height) = font.text_extent_markup(&text);
        let output_width = self.surface_width as i32;
        let output_height = self.surface_height as i32;
        let unclamped_x = ((object.position.x as f32 - target_x) * zoom).round() as i32;
        let half_width = text_width / 2;
        let text_x = if half_width <= output_width - half_width {
            unclamped_x.clamp(half_width, output_width - half_width)
        } else {
            output_width / 2
        };
        let unclamped_y = ((object.position.y - definition_shape.height / 2 - 20) as f32
            - target_y)
            * zoom;
        let unclamped_y = unclamped_y.round() as i32 - text_height;
        let text_y = if text_height <= output_height {
            unclamped_y.clamp(0, output_height - text_height)
        } else {
            0
        };

        let owner_color = owner_colors
            .get(&object.owner)
            .copied()
            .unwrap_or_else(|| default_owner_color(object.owner));
        // C4 colors store transparency in the high byte. `| 0x7f000000`
        // therefore becomes source alpha 255-127 = 128.
        let color = Color::new(owner_color.r, owner_color.g, owner_color.b, 128);
        let fog = self.fog_draw_context();
        if let Some(fog) = fog.as_ref() {
            draw_fogged_markup_text(
                &mut self.surface,
                &font,
                text_x,
                text_y,
                &text,
                color,
                gamma,
                self.advanced_renderer_config,
                fog,
            );
        } else {
            font.draw_markup_with_gamma(
                &mut self.surface,
                text_x,
                text_y,
                &text,
                color,
                clonk_graphics::clonk_font::TextAlign::Center,
                gamma,
            );
        }
    }

    /// C4Object::Draw emits the global fctEnergy facet after the object's
    /// face/overlays and before the list-wide TopFace pass. It is centered on
    /// the live Con-scaled definition Shape. The facet itself ignores object
    /// blit state, owner tint, rotation, and draw transforms
    /// (src/C4Object.cpp:2518-2524).
    fn paint_need_energy_bolt(
        &mut self,
        object: &ObjectSnapshot,
        frame: u64,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if !object.need_energy || frame % 35 <= 12 {
            return;
        }
        let Some(energy) = self.hud_graphics.energy.clone() else {
            return;
        };
        let Some(definition_sprite) = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
        else {
            return;
        };
        let shape = self.live_object_shape(definition_sprite, object);
        if !self.object_reaches_post_face_draw(object, definition_sprite, shape) {
            return;
        }
        let width = energy.width() as i32;
        let height = energy.height() as i32;
        let x = object.position.x + shape.x + shape.width / 2 - width / 2;
        let y = object.position.y + shape.y - height - 5;
        let (target_x, target_y) = self.object_target_position(object);
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let fog = self.fog_draw_context();
        draw_image_region_float_source(
            &mut self.surface,
            &GuiRect::new(
                (x as f32 - target_x) * zoom,
                (y as f32 - target_y) * zoom,
                width as f32 * zoom,
                height as f32 * zoom,
            ),
            &energy,
            None,
            &FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: energy.width() as f32,
                height: energy.height() as f32,
            },
            BlitSampling::Linear,
            false,
            None,
            SpriteBlitState::normal().with_renderer_config(self.advanced_renderer_config),
            gamma,
            fog.as_ref(),
        );
    }

    /// The output-boundary return precedes overlays, selection and fctEnergy
    /// in C4Object::Draw (src/C4Object.cpp:2266-2283). Active non-rotated
    /// facets use their own rectangle; stretched facets bypass the gate.
    fn object_reaches_post_face_draw(
        &self,
        object: &ObjectSnapshot,
        definition_sprite: &DefinitionSprite,
        shape: DefinitionRect,
    ) -> bool {
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let output_width = ((self.surface_width as f32 / zoom).ceil() as i32).max(1);
        let output_height = ((self.surface_height as f32 / zoom).ceil() as i32).max(1);
        let (target_x, target_y) = self.object_target_position(object);
        let cox = object.position.x + shape.x - target_x as i32;
        let coy = object.position.y + shape.y - target_y as i32;
        let inside = |position: i32, size: i32, extent: i32| {
            position >= 1 - size && position <= extent
        };

        if let Some(graphics) = Self::live_action_graphics(
            &definition_sprite.actions,
            &object.action,
        ) {
            if graphics.facet_target_stretch {
                return true;
            }
            if object.rotation == 0
                && !graphics.facet_base
                && object.construction <= FULL_CON
            {
                let (x, y, width, height) = graphics
                    .facet
                    .as_ref()
                    .map_or((0, 0, 0, 0), |facet| {
                        (
                            facet.target_x,
                            facet.target_y,
                            facet.width,
                            facet.height,
                        )
                    });
                return inside(cox + x, width, output_width)
                    && inside(coy + y, height, output_height);
            }
        }

        inside(cox, shape.width, output_width) && inside(coy, shape.height, output_height)
    }

    /// C4ObjectList draws every base before any TopFace
    /// (src/C4ObjectList.cpp:390-396). The pass also owns the construction
    /// sign, even when the definition has no TopFace. An active
    /// `FacetTopFace` action relocates only the source rectangle; the target
    /// remains the definition TopFace target (src/C4Object.cpp:2639-2668).
    pub(crate) fn paint_object_top_face(
        &mut self,
        object: &ObjectSnapshot,
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let (base_definition_id, base_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        // SetGraphics swaps only GetGraphics()/the bitmap source. DefCore
        // TopFace, Shape/GrowthType and the live ActMap remain owned by
        // object->Def (C4Object.cpp:357-376,404-425,2639-2667).
        let mut bitmap_sprite = self
            .object_sprites
            .get(&sprite_map_key(
                &base_definition_id,
                base_graphics_name.as_deref(),
            ))
            .cloned();
        if bitmap_sprite.is_none() && base_graphics_name.is_some() {
            bitmap_sprite = self
                .object_sprites
                .get(&sprite_map_key(&base_definition_id, None))
                .cloned();
        }
        if bitmap_sprite.is_none() && base_definition_id != object.definition_id {
            bitmap_sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let Some(bitmap_sprite) = bitmap_sprite else {
            return;
        };
        let definition_sprite = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
            .cloned()
            // A same-definition named sheet carries identical definition
            // metadata in imported atlases. Preserve that legacy fallback
            // when the default atlas entry itself is unavailable.
            .or_else(|| {
                (base_definition_id == object.definition_id).then(|| bitmap_sprite.clone())
            });
        let Some(definition_sprite) = definition_sprite else {
            return;
        };

        // C4Object::DrawTopFace draws fctConstruction at the bottom-left of
        // the CURRENT Con-scaled Shape, after every object's base pass. It is
        // a plain global-resource facet: no owner tint, object transform,
        // ColorMod, rotation or object blit mode (C4Object.cpp:2617-2638).
        if object.ocf & clonk_engine::ocf::CONSTRUCT != 0 && object.rotation == 0 {
            if let Some(construction) = self.hud_graphics.construction.clone() {
                let fog = self.fog_draw_context();
                let shape = Self::con_scaled_shape(
                    Self::sprite_def_shape(&definition_sprite),
                    object.construction.clamp(0, FULL_CON),
                    definition_sprite.stretch_growth,
                );
                let cox = object.position.x + shape.x;
                let coy = object.position.y + shape.y;
                let width = construction.width() as i32;
                let height = construction.height() as i32;
                let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
                let rect = GuiRect::new(
                    (cox as f32 - self.viewport_x) * zoom,
                    ((coy + shape.height - height) as f32 - self.viewport_y) * zoom,
                    width as f32 * zoom,
                    height as f32 * zoom,
                );
                let (source, sampling) = self.runtime_sprite_blit(
                    FloatSourceRect::scaled(SourceRect::new(0, 0, width, height), 1.0),
                    (rect.size.width, rect.size.height),
                    false,
                );
                draw_image_region_float_source(
                    &mut self.surface,
                    &rect,
                    &construction,
                    None,
                    &source,
                    sampling,
                    false,
                    None,
                    SpriteBlitState::normal().with_renderer_config(self.advanced_renderer_config),
                    gamma,
                    fog.as_ref(),
                );
            }
        }

        let construction = object.construction.max(0);
        if (construction < FULL_CON && !definition_sprite.stretch_growth)
            || object.rotation.rem_euclid(360) != 0
        {
            return;
        }
        let Some(top_face) = definition_sprite.top_face else {
            return;
        };
        let shape = Self::con_scaled_shape(
            Self::sprite_def_shape(&definition_sprite),
            construction,
            definition_sprite.stretch_growth,
        );
        let cox = object.position.x + shape.x;
        let coy = object.position.y + shape.y;
        // UpdateFlipDir installs the object's horizontal draw transform for
        // every active action, independently of FacetTopFace. DrawTopFace
        // therefore mirrors a plain definition TopFace too
        // (src/C4Object.cpp:404-430,2639-2668).
        let action_graphics =
            Self::live_action_graphics(&definition_sprite.actions, &object.action);
        let (draw_dir, flipped) = action_graphics
            .map(|graphics| Self::resolve_draw_direction(graphics, object.direction))
            .unwrap_or((object.direction.to_script_value(), false));

        let mut source_x = top_face.x;
        let mut source_y = top_face.y;
        if let Some(graphics) = action_graphics.filter(|graphics| graphics.facet_top_face) {
            // C4ActionDef::Facet is a zeroed C4TargetRect when omitted. Only
            // its source x/y/size participate in the TopFace override.
            let (facet_x, facet_y, facet_width, facet_height) = graphics
                .facet
                .as_ref()
                .map(|facet| (facet.x, facet.y, facet.width, facet.height))
                .unwrap_or((0, 0, 0, 0));
            let mut phase = object.action.phase;
            if graphics.reverse {
                phase = graphics
                    .length
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .saturating_sub(phase);
            }
            source_x = facet_x
                .saturating_add(top_face.x)
                .saturating_add(facet_width.saturating_mul(phase));
            source_y = facet_y
                .saturating_add(top_face.y)
                .saturating_add(facet_height.saturating_mul(draw_dir));
        }
        let growth_scale = if definition_sprite.stretch_growth && construction != FULL_CON {
            Some(construction)
        } else {
            None
        };
        let target_x = growth_scale.map_or(top_face.target_x, |con| {
            top_face.target_x.saturating_mul(con) / FULL_CON
        });
        let target_y = growth_scale.map_or(top_face.target_y, |con| {
            top_face.target_y.saturating_mul(con) / FULL_CON
        });
        let target_width = growth_scale.map_or(top_face.width, |con| {
            top_face.width.saturating_mul(con) / FULL_CON
        });
        let target_height = growth_scale.map_or(top_face.height, |con| {
            top_face.height.saturating_mul(con) / FULL_CON
        });
        self.blit_face(
            &bitmap_sprite,
            SourceRect::new(source_x, source_y, top_face.width, top_face.height),
            (
                (cox + target_x) as f32,
                (coy + target_y) as f32,
                target_width as f32,
                target_height as f32,
            ),
            (
                cox as f32 + shape.width as f32 / 2.0,
                coy as f32 + shape.height as f32 / 2.0,
            ),
            flipped,
            Some(object_color_by_owner_tint(object)),
            self.viewport_zoom.max(MIN_VIEWPORT_ZOOM),
            0.0,
            object.draw_transform,
            blit,
            gamma,
        );
    }

    pub(crate) fn paint_object(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        for_player: i32,
        lighting: f32,
        owner_colors: &HashMap<i32, Color>,
        line: i32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        self.paint_object_with_particles(
            object,
            objects,
            &[],
            players,
            for_player,
            lighting,
            owner_colors,
            line,
            gamma,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_object_with_particles(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        particles: &[ParticleSnapshot],
        players: &[PlayerState],
        for_player: i32,
        lighting: f32,
        _owner_colors: &HashMap<i32, Color>,
        line: i32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        // C4Object::Draw dispatches every nonzero Def->Line before bounds,
        // containment, TargetPos/parallax, transforms, particles, and faces
        // (src/C4Object.cpp:2249-2254). Even presently unsupported types must
        // return here so a sprite never stands in for the line.
        if line != 0 {
            self.paint_typed_line(object, line, gamma);
            return false;
        }
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let content_width = self.surface_width as f32;
        let content_height = self.surface_height as f32;
        let color = object_color(object).modulate(lighting);
        let owner_color = Some(object_color_by_owner_tint(object));
        let rotation_degrees = (object.rotation.rem_euclid(360)) as f32;

        // C4Object::Draw resolves the face origin through TargetPos, which
        // applies parallaxity for C4D_Parallax objects (src/C4Object.cpp:2271).
        // It holds that in the locals cotx/coty and never writes back into
        // cgo.TargetX/Y, so every C4GraphicsOverlay::Draw re-derives its own
        // target from the untouched viewport scroll
        // (src/C4DefGraphics.cpp:763-765). Keep self.viewport_x/y as that raw
        // scroll and hand the resolved origin down explicitly.
        let (target_x, target_y) = self.object_target_position(object);
        let screen_x = (object.position.x as f32 - target_x) * zoom;
        let screen_y = (object.position.y as f32 - target_y) * zoom;

        let base_transform = object.draw_transform;
        let (base_definition_id, base_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(
                &base_definition_id,
                base_graphics_name.as_deref(),
            ))
            .cloned();
        if sprite.is_none() && base_graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&base_definition_id, None))
                .cloned();
        }
        if sprite.is_none() && base_definition_id != object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let geometry_sprite = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
            .cloned()
            .or_else(|| sprite.clone());
        if let Some(geometry_sprite) = geometry_sprite.as_ref() {
            let target_position = (target_x, target_y);
            let shape = self.live_object_shape(geometry_sprite, object);
            if !self.object_reaches_post_face_draw(object, geometry_sprite, shape) {
                self.draw_definition_particles(
                    particles,
                    &ParticleLayer::ObjectFront(object.id),
                    Some(object),
                    gamma,
                );
                return false;
            }
            // C4Object draws the fire facet before PrepareDrawing and before
            // its base/action face (src/C4Object.cpp:2388-2418), so the
            // object's ColorMod, BlitMode, rotation and draw transform do not
            // affect this normal rendering path.
            self.draw_object_fire(
                object,
                geometry_sprite,
                target_position,
                SpriteBlitState::normal().with_renderer_config(self.advanced_renderer_config),
                gamma,
            );
        }
        if let Some(sprite) = sprite {
            // The face blit consumes the resolved origin (cox/coy are measured
            // from cotx/coty), so point the draw origin at it for exactly that
            // call. It is restored before the overlay walk, which resolves its
            // own target from the raw scroll (src/C4DefGraphics.cpp:763).
            let saved_viewport_x = self.viewport_x;
            let saved_viewport_y = self.viewport_y;
            self.viewport_x = target_x;
            self.viewport_y = target_y;
            self.draw_object_face(
                object,
                objects,
                &sprite,
                owner_color,
                zoom,
                rotation_degrees,
                base_transform,
                SpriteBlitState::for_object(object)
                    .with_renderer_config(self.advanced_renderer_config),
                gamma,
            );
            self.viewport_x = saved_viewport_x;
            self.viewport_y = saved_viewport_y;
            self.draw_object_overlays_with_particles(
                object,
                objects,
                particles,
                players,
                for_player,
                owner_color,
                screen_x,
                screen_y,
                zoom,
                rotation_degrees,
                base_transform,
                gamma,
            );
            self.draw_definition_particles(
                particles,
                &ParticleLayer::ObjectFront(object.id),
                Some(object),
                gamma,
            );
            return true;
        }

        if screen_x < -10.0
            || screen_y < -10.0
            || screen_x > content_width + 10.0
            || screen_y > content_height + 10.0
        {
            self.draw_definition_particles(
                particles,
                &ParticleLayer::ObjectFront(object.id),
                Some(object),
                gamma,
            );
            return false;
        }

        // No sprite available: debug fallbacks only (C++ objects always
        // have a graphics facet, so these paths have no oracle) — the
        // vertex polygon, then a plain dot.
        let fog = self.fog_draw_context();
        if object.vertices.len() >= 3 {
            let mut points = Vec::with_capacity(object.vertices.len());
            let mut min_x = f32::MAX;
            let mut max_x = f32::MIN;
            let mut min_y = f32::MAX;
            let mut max_y = f32::MIN;
            for vertex in &object.vertices {
                let world_x = (object.position.x + vertex.x) as f32;
                let world_y = (object.position.y + vertex.y) as f32;
                let x = (world_x - self.viewport_x) * zoom;
                let y = (world_y - self.viewport_y) * zoom;
                points.push((x.round() as i32, y.round() as i32));
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }

            if max_x >= -zoom
                && min_x <= content_width + zoom
                && max_y >= -zoom
                && min_y <= content_height + zoom
                && if let Some(fog) = fog.as_ref() {
                    fill_polygon_impl(&mut self.surface, &points, color, Some(fog), gamma)
                } else {
                    fill_polygon(&mut self.surface, &points, color)
                }
            {
                self.draw_definition_particles(
                    particles,
                    &ParticleLayer::ObjectFront(object.id),
                    Some(object),
                    gamma,
                );
                return true;
            }
        }

        let size = (6.0 * zoom).max(3.0);
        let rect = GuiRect::from_origin_size(
            GuiPoint::new(
                (screen_x - size / 2.0).max(0.0),
                (screen_y - size / 2.0).max(0.0),
            ),
            GuiSize::new(size, size),
        );
        if let Some(fog) = fog.as_ref() {
            fill_rect_impl(&mut self.surface, &rect, color, Some(fog), gamma);
        } else {
            fill_rect(&mut self.surface, &rect, color);
        }
        self.draw_object_overlays_with_particles(
            object,
            objects,
            particles,
            players,
            for_player,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            base_transform,
            gamma,
        );
        self.draw_definition_particles(
            particles,
            &ParticleLayer::ObjectFront(object.id),
            Some(object),
            gamma,
        );
        true
    }

    fn paint_typed_line(
        &mut self,
        object: &ObjectSnapshot,
        line: i32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let local_palette = |name| {
            object
                .local_vars
                .get(name)
                .and_then(|value| value.as_c4_int())
                .unwrap_or(0) as u8
        };
        let colors = match line {
            1 => Some((68, 26)),
            2 | 3 => Some((23, 26)),
            4 => Some((6, 6)),
            6 => Some((65, 65)),
            7 | 8 => Some((local_palette("__local_0"), local_palette("__local_1"))),
            // Volcano has no C++ DrawLine switch arm.
            5 => None,
            _ => None,
        };
        let Some((primary, marker)) = colors else {
            return;
        };
        let object_blit = self.configured_blit(SpriteBlitState::for_object(object));
        // DrawLineDw applies the active ColorMod through
        // ClrByCurrentBlitMod, but only the ADDITIVE bit changes its GL blend
        // function. Texture-only MOD2 modes never alter line RGB. Preserve
        // the modulation activation (MOD2 + zero ColorMod intentionally
        // modulates the line to black), while masking those mode bits out.
        let primary = modulate_line_palette_color(
            self.game_palette.color(primary),
            object_blit.modulation,
        );
        let marker =
            modulate_line_palette_color(self.game_palette.color(marker), object_blit.modulation);
        let blit = SpriteBlitState {
            mode: object_blit.mode & C4GFXBLIT_ADDITIVE,
            modulation: None,
            fog_modulation: None,
            renderer_config: self.advanced_renderer_config,
        };
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let target_x = self.viewport_x as i32;
        let target_y = self.viewport_y as i32;
        let logical_width = ((self.surface.width() as f32 / zoom).ceil() as i32).max(1);
        let logical_height = ((self.surface.height() as f32 / zoom).ceil() as i32).max(1);
        let fog = self.fog_draw_context();
        for vertices in object.vertices.windows(2) {
            // CONNECT owns absolute live C4Shape vertices. DrawLine is called
            // before TargetPos, so object position/parallax/transform never
            // participates (src/C4Object.cpp:2249; C4FacetEx.cpp:46-54).
            let start = (vertices[0].x - target_x, vertices[0].y - target_y);
            let end = (vertices[1].x - target_x, vertices[1].y - target_y);
            if line == 4 {
                draw_object_bolt_segment(
                    &mut self.surface,
                    start,
                    end,
                    logical_width,
                    logical_height,
                    zoom,
                    primary,
                    marker,
                    blit,
                    gamma,
                    fog.as_ref(),
                    &mut self.presentation_rng,
                );
            } else {
                draw_object_line_segment(
                    &mut self.surface,
                    (
                        (vertices[0].x as f32 - self.viewport_x) * zoom,
                        (vertices[0].y as f32 - self.viewport_y) * zoom,
                    ),
                    (
                        (vertices[1].x as f32 - self.viewport_x) * zoom,
                        (vertices[1].y as f32 - self.viewport_y) * zoom,
                    ),
                    primary,
                    marker,
                    blit,
                    gamma,
                    fog.as_ref(),
                );
            }
        }
    }

    /// C4Shape con-scaling for drawing (C4Object::UpdateShape,
    /// src/C4Object.cpp:325-333): GrowthType stretches x/y/Wdt/Hgt
    /// (C4Shape::Stretch, src/C4Shape.cpp:103-116), otherwise only
    /// y/Hgt shrink (C4Shape::Jolt, src/C4Shape.cpp:119-128).
    fn con_scaled_shape(shape: DefinitionRect, con: i32, stretch_growth: bool) -> DefinitionRect {
        if con == FULL_CON {
            return shape;
        }
        let percent = ((i64::from(con.max(0)) * 100) / i64::from(FULL_CON)) as i32;
        let scale = |value: i32| {
            (i64::from(value) * i64::from(percent) / 100)
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        let mut scaled = shape;
        if stretch_growth {
            scaled.x = scale(scaled.x);
            scaled.width = scale(scaled.width);
        }
        scaled.y = scale(scaled.y);
        scaled.height = scale(scaled.height);
        scaled
    }

    /// Reconstruct the live C4Shape rect used for presentation. Shape::Rotate
    /// replaces the Con-scaled rect with its legacy radius square whenever
    /// the definition is Rotateable and the raw saved angle is nonzero
    /// (src/C4Object.cpp:320-343; src/C4Shape.cpp:41-83).
    fn live_object_shape(
        &self,
        sprite: &DefinitionSprite,
        object: &ObjectSnapshot,
    ) -> DefinitionRect {
        if let Some(shape) = object.current_shape {
            return shape;
        }
        let mut shape = Self::sprite_def_shape(sprite);
        if sprite.line == 0 {
            shape = Self::con_scaled_shape(
                shape,
                object.construction.max(0),
                sprite.stretch_growth,
            );
        }
        // UpdateShape tests raw r, so a loaded r=360 still enlarges the
        // rectangle even though its vertices retain their orientation.
        let rotateable = sprite.rotateable != 0
            || self
                .rotateable_definitions
                .contains(&object.definition_id)
            || object.ocf & clonk_engine::ocf::ROTATE != 0;
        if sprite.line == 0 && rotateable && object.rotation != 0 {
            let radius = (((i64::from(shape.x) * i64::from(shape.x)
                + i64::from(shape.y) * i64::from(shape.y)) as f64)
                .sqrt() as i32)
                .saturating_add(2);
            shape.x = -radius;
            shape.y = -radius;
            shape.width = radius.saturating_mul(2);
            shape.height = radius.saturating_mul(2);
        }
        shape
    }
    /// The def Shape rect used for drawing; loader sprites without a def
    /// shape fall back to the whole image centered on the position.
    fn sprite_def_shape(sprite: &DefinitionSprite) -> DefinitionRect {
        sprite.shape.unwrap_or_else(|| {
            let width = sprite.image.width() as i32;
            let height = sprite.image.height() as i32;
            DefinitionRect::new(-width / 2, -height / 2, width, height)
        })
    }

    /// C4Object::Draw's fctFire pass (src/C4Object.cpp:2388-2408). The
    /// horizontal FirePhase cell is stretched without aspect preservation;
    /// even rotated objects receive an axis-aligned flame rectangle.
    pub(crate) fn draw_object_fire(
        &mut self,
        object: &ObjectSnapshot,
        definition_sprite: &DefinitionSprite,
        target_position: (f32, f32),
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        if !object.on_fire {
            return;
        }
        let Some(fire) = self.hud_graphics.fire.clone() else {
            return;
        };
        let cell = fire.height() as i32;
        if cell <= 0 {
            return;
        }

        // Oversize definitions may legitimately carry Con > FullCon; C++
        // scales their Shape and FireTop above 100% (C4Object.cpp:322-340).
        let con = object.construction.max(0);
        let shape = self.live_object_shape(definition_sprite, object);
        let target = if object.rotation == 0 {
            let percent = if con == FULL_CON {
                100
            } else {
                ((i64::from(con) * 100) / i64::from(FULL_CON)) as i32
            };
            let fire_top = object.current_fire_top.unwrap_or_else(|| {
                if definition_sprite.line != 0 {
                    definition_sprite.fire_top
                } else {
                    (i64::from(definition_sprite.fire_top) * i64::from(percent) / 100)
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
                }
            });
            DefinitionRect::new(
                object.position.x + shape.x,
                object.position.y + shape.y,
                shape.width,
                shape.height - fire_top,
            )
        } else {
            // GetVertexOutline includes the origin and every live vertex.
            // Its final y is forced to the already-live Shape.y while
            // retaining the previous lower edge (src/C4Shape.cpp:130-163).
            // This also preserves SetShape and the non-rotateable/raw-r case.
            // FireTop is intentionally ignored in this branch.
            let left = object
                .vertices
                .iter()
                .map(|vertex| vertex.x)
                .min()
                .unwrap_or(0)
                .min(0);
            let right = object
                .vertices
                .iter()
                .map(|vertex| vertex.x)
                .max()
                .unwrap_or(0)
                .max(0);
            let bottom = object
                .vertices
                .iter()
                .map(|vertex| vertex.y)
                .max()
                .unwrap_or(0)
                .max(0);
            DefinitionRect::new(
                object.position.x + left,
                object.position.y + shape.y,
                right - left,
                bottom - shape.y,
            )
        };
        if target.width <= 0 || target.height <= 0 {
            return;
        }

        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let (target_x, target_y) = target_position;
        let rect = GuiRect::from_origin_size(
            GuiPoint::new(
                (target.x as f32 - target_x) * zoom,
                (target.y as f32 - target_y) * zoom,
            ),
            GuiSize::new(target.width as f32 * zoom, target.height as f32 * zoom),
        );
        let source_x = i64::from(object.fire_phase) * i64::from(cell);
        if source_x < 0 || source_x + i64::from(cell) > i64::from(fire.width()) {
            return;
        }
        let source = SourceRect::new(source_x as i32, 0, cell, cell);
        let fog = self.fog_draw_context();
        let (source, sampling) = self.runtime_sprite_blit(
            FloatSourceRect::scaled(source, 1.0),
            (rect.size.width, rect.size.height),
            false,
        );
        draw_image_region_float_source(
            &mut self.surface,
            &rect,
            &fire,
            None,
            &source,
            sampling,
            false,
            None,
            blit,
            gamma,
            fog.as_ref(),
        );
    }

    pub(crate) fn live_action_graphics<'a>(
        actions: &'a HashMap<String, DefinitionActionGraphics>,
        action: &clonk_engine::ActionState,
    ) -> Option<&'a DefinitionActionGraphics> {
        match action.act_map_index {
            Some(index) => actions.get(&physical_action_graphics_key(index)),
            None if actions.contains_key(PHYSICAL_ACTION_GRAPHICS_MARKER) => None,
            None => actions.get(action.name.as_str()),
        }
    }

    /// C4Object::Draw facet selection (src/C4Object.cpp:2388-2468):
    /// idle draws the base face only; active actions draw the optional
    /// FacetBase face plus the action facet — an active action with
    /// neither draws nothing (src/C4Object.cpp:2402).
    fn draw_object_face(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        sprite: &DefinitionSprite,
        owner_color: Option<u32>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let con = object.construction.clamp(0, FULL_CON);
        let def_shape = Self::sprite_def_shape(sprite);
        let inst_shape = Self::con_scaled_shape(def_shape, con, sprite.stretch_growth);
        let graphics = Self::live_action_graphics(&sprite.actions, &object.action);
        let Some(graphics) = graphics else {
            // Idle: BaseFace only, phase (0,0) (src/C4Object.cpp:2388-2392).
            self.draw_base_face(
                object,
                sprite,
                con,
                def_shape,
                inst_shape,
                0,
                0,
                false,
                owner_color,
                zoom,
                rotation_degrees,
                transform,
                blit,
                gamma,
            );
            return;
        };
        let (draw_dir, flipped) = Self::resolve_draw_direction(graphics, object.direction);
        // FacetBase face underneath, phase (0, DrawDir)
        // (src/C4Object.cpp:2397-2399).
        if graphics.facet_base {
            self.draw_base_face(
                object,
                sprite,
                con,
                def_shape,
                inst_shape,
                0,
                draw_dir,
                flipped,
                owner_color,
                zoom,
                rotation_degrees,
                transform,
                blit,
                gamma,
            );
        }
        let Some(facet) = &graphics.facet else {
            return;
        };
        if facet.width <= 0 || facet.height <= 0 {
            return;
        }
        let cox = (object.position.x + inst_shape.x) as f32;
        let coy = (object.position.y + inst_shape.y) as f32;
        // FacetTargetStretch bypasses action phase/direction and object
        // transforms: DrawX scales the declared source from FacetY exactly
        // to Target->y + Target->Shape.y (src/C4Object.cpp:2426-2438).
        if graphics.facet_target_stretch {
            let Some(target) = object
                .action
                .target
                .and_then(|target| objects.iter().find(|object| object.id == target))
            else {
                return;
            };
            let Some(target_sprite) = self
                .object_sprites
                .get(&sprite_map_key(&target.definition_id, None))
            else {
                return;
            };
            let target_shape = Self::con_scaled_shape(
                Self::sprite_def_shape(target_sprite),
                target.construction.clamp(0, FULL_CON),
                target_sprite.stretch_growth,
            );
            let dest_y = coy + facet.target_y as f32;
            let dest_height = (target.position.y + target_shape.y) as f32 - dest_y;
            self.blit_face(
                sprite,
                SourceRect::new(facet.x, facet.y, facet.width, facet.height),
                (
                    cox + facet.target_x as f32,
                    dest_y,
                    facet.width as f32,
                    dest_height,
                ),
                (
                    cox + inst_shape.width as f32 / 2.0,
                    coy + inst_shape.height as f32 / 2.0,
                ),
                false,
                owner_color,
                zoom,
                0.0,
                None,
                blit,
                gamma,
            );
            return;
        }
        // Drawing phase; Reverse mirrors it (src/C4Object.cpp:2419-2420).
        let length = graphics.length.unwrap_or(1);
        let mut phase = object.action.phase;
        if graphics.reverse {
            phase = length.saturating_sub(1).saturating_sub(phase);
        }
        let source = SourceRect::new(
            facet.x + facet.width.saturating_mul(phase),
            facet.y + facet.height.saturating_mul(draw_dir),
            facet.width,
            facet.height,
        );
        // Full con: the facet at cox+FacetX/coy+FacetY; growing: the
        // con-scaled shape rect at cox/coy (src/C4Object.cpp:2450-2467).
        let dest = if con == FULL_CON {
            (
                cox + facet.target_x as f32,
                coy + facet.target_y as f32,
                facet.width as f32,
                facet.height as f32,
            )
        } else {
            (cox, coy, inst_shape.width as f32, inst_shape.height as f32)
        };
        self.blit_face(
            sprite,
            source,
            dest,
            (
                cox + inst_shape.width as f32 / 2.0,
                coy + inst_shape.height as f32 / 2.0,
            ),
            flipped,
            owner_color,
            zoom,
            rotation_degrees,
            transform,
            blit,
            gamma,
        );
    }

    /// C4Object::DrawFace (src/C4Object.cpp:438-467): the base face is
    /// the def Shape.Wdt x Shape.Hgt crop at phase (iPhaseX, iPhaseY),
    /// stretched by Con — GrowthType shrinks both axes toward the shape
    /// center, otherwise the width stays and the bottom source slice is
    /// shown (construction display).
    #[allow(clippy::too_many_arguments)]
    fn draw_base_face(
        &mut self,
        object: &ObjectSnapshot,
        sprite: &DefinitionSprite,
        con: i32,
        def_shape: DefinitionRect,
        inst_shape: DefinitionRect,
        phase_x: i32,
        phase_y: i32,
        flipped: bool,
        owner_color: Option<u32>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let swdt = def_shape.width;
        let shgt = def_shape.height;
        let fx = swdt * phase_x;
        let mut fy = shgt * phase_y;
        let fwdt = swdt;
        let mut fhgt = shgt;

        let cox = object.position.x + inst_shape.x;
        let coy = object.position.y + inst_shape.y;

        // Grow-type display (src/C4Object.cpp:448-451).
        let mut tx = (cox + (inst_shape.width - swdt * con / FULL_CON) / 2) as f32;
        let ty = (coy + (inst_shape.height - shgt * con / FULL_CON) / 2) as f32;
        let mut twdt = (swdt * con / FULL_CON) as f32;
        let thgt = (shgt * con / FULL_CON) as f32;

        // Construction-type display (src/C4Object.cpp:453-460).
        if !sprite.stretch_growth {
            tx = cox as f32 + (inst_shape.width - swdt) as f32 / 2.0;
            twdt = swdt as f32;
            fy += shgt * (FULL_CON - con).max(0) / FULL_CON;
            fhgt = (shgt * con / FULL_CON).min(shgt);
        }

        self.blit_face(
            sprite,
            SourceRect::new(fx, fy, fwdt, fhgt),
            (tx, ty, twdt, thgt),
            (
                cox as f32 + inst_shape.width as f32 / 2.0,
                coy as f32 + inst_shape.height as f32 / 2.0,
            ),
            flipped,
            owner_color,
            zoom,
            rotation_degrees,
            transform,
            blit,
            gamma,
        );
    }

    /// Blit one object face: clamps the source to the sheet (ActMap
    /// facets may nominally exceed it — Tree1 Still is 73x73 on a
    /// 71px-tall Graphics.png; GL clamps), mirrors flipped faces around
    /// the shape center (C4DrawTransform flipdir, C4Object::UpdateFlipDir
    /// src/C4Object.cpp:415-418, applied at src/C4Object.cpp:2458),
    /// applies the script draw transform at the shape center
    /// (SetTransformAt, src/C4Object.cpp:2431) and rotates around it
    /// (src/C4Object.cpp:483-488, 2428-2435).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_face(
        &mut self,
        sprite: &DefinitionSprite,
        source: SourceRect,
        dest: (f32, f32, f32, f32),
        shape_center: (f32, f32),
        flipped: bool,
        owner_color: Option<u32>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let (dest_x, dest_y, mut dest_w, mut dest_h) = dest;
        if dest_w <= 0.0 || dest_h <= 0.0 || source.width <= 0 || source.height <= 0 {
            return;
        }
        // Every C4Object face path passes GetGraphics()->pDef->Scale to the
        // facet blit. SetGraphics may select another definition, so scale
        // the source rectangle with the selected bitmap's metadata while
        // leaving the live definition's destination/shape untouched.
        let source = FloatSourceRect::scaled(source, sprite.graphics_scale);
        if !source.is_valid() {
            return;
        }
        let transformed = transform.is_some() || flipped || rotation_degrees.abs() > f32::EPSILON;
        let (mut source, sampling) =
            self.runtime_sprite_blit(source, (dest_w * zoom, dest_h * zoom), transformed);
        let image_w = sprite.image.width() as f32;
        let image_h = sprite.image.height() as f32;
        if source.x < 0.0 || source.y < 0.0 {
            return;
        }
        let clamped_w = source.width.min(image_w - source.x);
        let clamped_h = source.height.min(image_h - source.y);
        if !clamped_w.is_finite()
            || !clamped_h.is_finite()
            || clamped_w <= 0.0
            || clamped_h <= 0.0
        {
            return;
        }
        dest_w *= clamped_w / source.width;
        dest_h *= clamped_h / source.height;
        source.width = clamped_w;
        source.height = clamped_h;

        let viewport_x = self.viewport_x;
        let viewport_y = self.viewport_y;
        let fog = self.fog_draw_context();
        if !transformed {
            let rect = GuiRect::from_origin_size(
                GuiPoint::new((dest_x - viewport_x) * zoom, (dest_y - viewport_y) * zoom),
                GuiSize::new(dest_w * zoom, dest_h * zoom),
            );
            draw_image_region_float_source(
                &mut self.surface,
                &rect,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                sampling,
                false,
                owner_color,
                blit,
                gamma,
                fog.as_ref(),
            );
        } else {
            let mut matrix = transform
                .map(|transform| transform.matrix())
                .unwrap_or(GraphicsTransform::identity().mat);
            // C4DrawTransform::SetFlipDir changes only matrix a. Direction
            // mirroring must therefore happen before SetTransformAt, not as a
            // second texture flip after the script matrix.
            if flipped {
                matrix[0] = -matrix[0];
            }
            let mut matrix = draw_transform_at(matrix, shape_center.0, shape_center.1);
            if rotation_degrees.abs() > f32::EPSILON {
                matrix = matrix.multiply(&GraphicsTransform::set_rotate(
                    (rotation_degrees * 100.0).round() as i32,
                    shape_center.0,
                    shape_center.1,
                ));
            }
            matrix = matrix.multiply(&GraphicsTransform::set_move_scale(
                -viewport_x * zoom,
                -viewport_y * zoom,
                zoom,
                zoom,
            ));
            // Rotation is composed into this matrix too, so rotated object
            // faces retain the same fractional source rectangle.
            draw_image_region_transformed_float_source(
                &mut self.surface,
                (dest_x, dest_y, dest_w, dest_h),
                &matrix,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                sampling,
                false,
                owner_color,
                blit,
                gamma,
                fog.as_ref(),
            );
        }
    }

    fn draw_action_graphic(
        &mut self,
        sprite: &DefinitionSprite,
        action_name: &str,
        phase: i32,
        direction: Direction,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some(graphics) = sprite.actions.get(action_name) else {
            return false;
        };
        let Some(facet) = &graphics.facet else {
            return false;
        };

        if facet.width <= 0 || facet.height <= 0 {
            return false;
        }

        let frame_count_i32 = graphics.length.unwrap_or(1).max(1);
        if frame_count_i32 <= 0 {
            return false;
        }

        let frame_index = if graphics.reverse && frame_count_i32 > 1 {
            let cycle = frame_count_i32.saturating_mul(2).saturating_sub(2);
            if cycle <= 0 {
                0
            } else {
                let cycle_i64 = i64::from(cycle);
                let phase_i64 = i64::from(phase);
                let pos = ((phase_i64 % cycle_i64) + cycle_i64) % cycle_i64;
                let pos_i32 = pos as i32;
                if pos_i32 >= frame_count_i32 {
                    cycle - pos_i32
                } else {
                    pos_i32
                }
            }
        } else {
            phase.rem_euclid(frame_count_i32)
        };

        let (draw_dir, flipped) = Self::resolve_draw_direction(graphics, direction);

        let source_rect = SourceRect::new(
            facet.x + facet.width.saturating_mul(frame_index),
            facet.y + facet.height.saturating_mul(draw_dir),
            facet.width,
            facet.height,
        );

        if !Self::source_within_image(&sprite.image, &source_rect) {
            return false;
        }
        let fog = self.fog_draw_context();
        // C4GraphicsOverlay always passes a local C4DrawTransform pointer,
        // including for a logically identity overlay. It is therefore always
        // non-exact in PerformBlt. Retain the established straight rasterizer
        // for a logical identity; its half-pixel geometry is tracked by L030.
        let source = FloatSourceRect::scaled(source_rect, 1.0);
        let (source, sampling) = self.runtime_sprite_blit(
            source,
            (facet.width as f32 * zoom, facet.height as f32 * zoom),
            true,
        );
        if transform.is_none() && rotation_degrees.abs() <= f32::EPSILON {
            let dest_width = facet.width as f32 * zoom;
            let dest_height = facet.height as f32 * zoom;
            let dest_rect = GuiRect::from_origin_size(
                GuiPoint::new(
                    screen_x - dest_width / 2.0,
                    screen_y - dest_height / 2.0,
                ),
                GuiSize::new(dest_width, dest_height),
            );
            draw_image_region_float_source(
                &mut self.surface,
                &dest_rect,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                sampling,
                flipped,
                owner_color,
                blit,
                gamma,
                fog.as_ref(),
            );
        } else {
            let center = (screen_x / zoom, screen_y / zoom);
            let dest = (
                center.0 - facet.width as f32 / 2.0,
                center.1 - facet.height as f32 / 2.0,
                facet.width as f32,
                facet.height as f32,
            );
            let mut matrix = draw_transform_at(
                transform
                    .map(|transform| transform.matrix())
                    .unwrap_or(GraphicsTransform::identity().mat),
                center.0,
                center.1,
            );
            if rotation_degrees.abs() > f32::EPSILON {
                matrix = matrix.multiply(&GraphicsTransform::set_rotate(
                    (rotation_degrees * 100.0).round() as i32,
                    center.0,
                    center.1,
                ));
            }
            matrix = matrix.multiply(&GraphicsTransform::set_move_scale(
                0.0, 0.0, zoom, zoom,
            ));
            draw_image_region_transformed_float_source(
                &mut self.surface,
                dest,
                &matrix,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                sampling,
                flipped,
                owner_color,
                blit,
                gamma,
                fog.as_ref(),
            );
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_object_overlays(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        for_player: i32,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        base_transform: Option<DrawTransform>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.draw_object_overlays_with_particles(
            object,
            objects,
            &[],
            players,
            for_player,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            base_transform,
            gamma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_object_overlays_with_particles(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        particles: &[ParticleSnapshot],
        players: &[PlayerState],
        for_player: i32,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        base_transform: Option<DrawTransform>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let mut object_ancestry = HashSet::from([object.id]);
        self.draw_object_overlays_inner(
            object,
            objects,
            particles,
            players,
            for_player,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            base_transform,
            gamma,
            &mut object_ancestry,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_object_overlays_inner(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        particles: &[ParticleSnapshot],
        players: &[PlayerState],
        for_player: i32,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        _rotation_degrees: f32,
        _base_transform: Option<DrawTransform>,
        gamma: Option<&clonk_graphics::GammaRamp>,
        object_ancestry: &mut HashSet<ObjectId>,
    ) {
        if object.graphics_overlays.is_empty() {
            return;
        }
        for overlay in &object.graphics_overlays {
            match overlay.mode {
                GraphicsOverlayMode::Action | GraphicsOverlayMode::Base => {
                    // Ordinary C++ overlays draw through their own Transform;
                    // the host's base draw transform and rotation are not
                    // inherited (C4DefGraphics.cpp:808-821).
                    let blit = self.configured_blit(SpriteBlitState::for_overlay(object, overlay));
                    if overlay.mode == GraphicsOverlayMode::Action {
                        self.draw_overlay_action(
                            object,
                            overlay,
                            owner_color,
                            screen_x,
                            screen_y,
                            zoom,
                            0.0,
                            overlay.transform,
                            blit,
                            gamma,
                        );
                    } else {
                        self.draw_overlay_base(
                            object,
                            overlay,
                            owner_color,
                            screen_x,
                            screen_y,
                            zoom,
                            0.0,
                            overlay.transform,
                            blit,
                            gamma,
                        );
                    }
                }
                GraphicsOverlayMode::Object => self.draw_overlay_object(
                    object,
                    overlay,
                    objects,
                    particles,
                    players,
                    for_player,
                    zoom,
                    gamma,
                    object_ancestry,
                ),
                // C4Object::Draw skips exactly one mode — IsPicture(), which is
                // `eMode == MODE_Picture` (src/C4Object.cpp:2526-2529;
                // src/C4DefGraphics.h:247), and C4GraphicsOverlay::Draw asserts
                // it never arrives (src/C4DefGraphics.cpp:758). MODE_None
                // survives that filter but IsValid rejects it, because
                // UpdateFacet leaves fctBlit defaulted (src/C4DefGraphics.cpp:
                // 638-639, :709-710).
                GraphicsOverlayMode::Picture | GraphicsOverlayMode::None => {}
                // TODO: MODE_ExtraGraphics redraws the host from the overlay's
                // graphics (src/C4DefGraphics.cpp:788-811); tracked in
                // PORT_STATUS.md. Listed explicitly so a new mode cannot be lost
                // to a catch-all the way these two were.
                GraphicsOverlayMode::ExtraGraphics => {}
                GraphicsOverlayMode::IngamePicture => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_overlay_object(
        &mut self,
        host: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        objects: &[ObjectSnapshot],
        particles: &[ParticleSnapshot],
        players: &[PlayerState],
        for_player: i32,
        zoom: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
        object_ancestry: &mut HashSet<ObjectId>,
    ) {
        let Some(target) = overlay
            .overlay_object
            .and_then(|id| objects.iter().find(|object| object.id == id))
        else {
            return;
        };
        // C4GraphicsOverlay::IsValid rejects missing/deleted targets and
        // overlay-object recursion. Imported snapshots can still be malformed,
        // so keep the draw walk bounded as well (C4DefGraphics.cpp:692-706).
        if target.status == ObjectStatus::Deleted
            || !Self::object_is_visible(objects, players, target, for_player, true)
            || !object_ancestry.insert(target.id)
        {
            return;
        }

        let saved_viewport_x = self.viewport_x;
        let saved_viewport_y = self.viewport_y;
        let saved_current_audibility_facet = self.current_audibility_facet;
        let offset_x = overlay.transform.map_or(0, |transform| transform.offset_x as i32);
        let offset_y = overlay.transform.map_or(0, |transform| transform.offset_y as i32);
        // C++ mutates cgo.TargetX/Y rather than the object's position. Keeping
        // the simulation coordinates intact is important for stretched action
        // facets that inspect their action target while the referenced object
        // is painted at the host's output position.
        let (host_target_x, host_target_y) = self.object_target_position(host);
        self.viewport_x = host_target_x - host.position.x as f32 + target.position.x as f32
            - offset_x as f32;
        self.viewport_y = host_target_y - host.position.y as f32 + target.position.y as f32
            - offset_y as f32;
        self.current_audibility_facet = saved_current_audibility_facet.map(|facet| {
            let host_target = Self::object_audibility_target_position(host, facet);
            AudibilityFacet {
                target_x: host_target
                    .x
                    .wrapping_sub(host.position.x)
                    .wrapping_add(target.position.x)
                    .wrapping_sub(offset_x),
                target_y: host_target
                    .y
                    .wrapping_sub(host.position.y)
                    .wrapping_add(target.position.y)
                    .wrapping_sub(offset_y),
                width: facet.width,
                height: facet.height,
            }
        });

        let (base_definition_id, base_graphics_name) =
            if let Some(base) = target.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (target.definition_id.clone(), None)
            };
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(
                &base_definition_id,
                base_graphics_name.as_deref(),
            ))
            .cloned();
        if sprite.is_none() && base_graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&base_definition_id, None))
                .cloned();
        }
        if sprite.is_none() && base_definition_id != target.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&target.definition_id, None))
                .cloned();
        }

        if let Some(sprite) = sprite {
            // MODE_Object's exact Parent sentinel inherits the referenced
            // object's state, not the host object's state
            // (C4DefGraphics.cpp:761-768).
            let blit = if overlay.blit_mode == C4GFXBLIT_PARENT {
                SpriteBlitState::for_object(target)
            } else {
                SpriteBlitState::for_overlay(host, overlay)
            }
            .with_renderer_config(self.advanced_renderer_config);
            let owner_color = Some(object_color_by_owner_tint(target));
            let rotation_degrees = (target.rotation.rem_euclid(360)) as f32;
            let geometry_sprite = self
                .object_sprites
                .get(&sprite_map_key(&target.definition_id, None))
                .cloned()
                .unwrap_or_else(|| sprite.clone());
            if geometry_sprite.line != 0 {
                // C4Object::Draw dispatches lines before every draw-mode
                // branch, including ODM_Overlay.
                self.record_line_audibility_calls(target);
                self.paint_typed_line(target, geometry_sprite.line, gamma);
            } else {
                if target.container.is_none() {
                    self.draw_definition_particles(
                        particles,
                        &ParticleLayer::ObjectBack(target.id),
                        Some(target),
                        gamma,
                    );
                }
                // C4Object::Draw(ODM_Overlay) reaches ShowSolidMask after
                // the back list and returns before fire, face, recursive
                // overlays, front particles, and the separate TopFace call.
                if self.debug_draw_flags.show_solid_mask
                    && self.object_has_debug_solid_mask(target)
                {
                    self.viewport_x = saved_viewport_x;
                    self.viewport_y = saved_viewport_y;
                    self.current_audibility_facet = saved_current_audibility_facet;
                    object_ancestry.remove(&target.id);
                    return;
                }
                // MODE_Object calls C4Object::Draw with ODM_Overlay. The fire
                // pass still precedes the face, but inherits the overlay's
                // already-established blit state (C4DefGraphics.cpp:769-780).
                // The referenced object's IgnoreFoW flag covers this Draw
                // body, including fire and recursive overlays, but C++
                // restores FoW before the separate DrawTopFace call.
                let suppress_fog = target.category & CATEGORY_IGNORE_FOW_FLAG != 0
                    && self.active_fog_map.is_some();
                if suppress_fog {
                    self.fog_suppression_depth += 1;
                }
                self.draw_object_fire(
                    target,
                    &geometry_sprite,
                    (self.viewport_x, self.viewport_y),
                    blit,
                    gamma,
                );
                self.draw_object_face(
                    target,
                    objects,
                    &sprite,
                    owner_color,
                    zoom,
                    rotation_degrees,
                    target.draw_transform,
                    blit,
                    gamma,
                );
                let screen_x = (target.position.x as f32 - self.viewport_x) * zoom;
                let screen_y = (target.position.y as f32 - self.viewport_y) * zoom;
                self.draw_object_overlays_inner(
                    target,
                    objects,
                    particles,
                    players,
                    for_player,
                    owner_color,
                    screen_x,
                    screen_y,
                    zoom,
                    rotation_degrees,
                    target.draw_transform,
                    gamma,
                    object_ancestry,
                );
                self.draw_definition_particles(
                    particles,
                    &ParticleLayer::ObjectFront(target.id),
                    Some(target),
                    gamma,
                );
                if suppress_fog {
                    self.fog_suppression_depth -= 1;
                }
                self.paint_object_top_face(target, blit, gamma);
            }
        }

        self.viewport_x = saved_viewport_x;
        self.viewport_y = saved_viewport_y;
        self.current_audibility_facet = saved_current_audibility_facet;
        object_ancestry.remove(&target.id);
    }

    fn draw_overlay_action(
        &mut self,
        object: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let definition_id = overlay
            .definition
            .as_deref()
            .unwrap_or(&object.definition_id);
        let graphics_name = overlay.graphics_name.as_deref();
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(definition_id, graphics_name))
            .cloned();
        if sprite.is_none() && graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(definition_id, None))
                .cloned();
        }
        if sprite.is_none() && definition_id != &object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let Some(sprite) = sprite else {
            return;
        };
        let action_name = overlay
            .action
            .as_deref()
            .unwrap_or(object.action.name.as_str());
        let phase = if overlay.phase != 0 {
            overlay.phase
        } else {
            object.action.phase
        };
        let _ = self.draw_action_graphic(
            &sprite,
            action_name,
            phase,
            object.direction,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            transform,
            blit,
            gamma,
        );
    }

    fn draw_overlay_base(
        &mut self,
        object: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let definition_id = overlay
            .definition
            .as_deref()
            .unwrap_or(&object.definition_id);
        let graphics_name = overlay.graphics_name.as_deref();
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(definition_id, graphics_name))
            .cloned();
        if sprite.is_none() && graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(definition_id, None))
                .cloned();
        }
        if sprite.is_none() && definition_id != &object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let Some(sprite) = sprite else {
            return;
        };
        let sprite_width = (sprite.image.width() as f32 * zoom).max(1.0);
        let sprite_height = (sprite.image.height() as f32 * zoom).max(1.0);
        let source_rect = SourceRect::new(
            0,
            0,
            sprite.image.width() as i32,
            sprite.image.height() as i32,
        );
        if !Self::source_within_image(&sprite.image, &source_rect) {
            return;
        }
        let fog = self.fog_draw_context();
        let source = FloatSourceRect::scaled(source_rect, 1.0);
        let (source, sampling) =
            self.runtime_sprite_blit(source, (sprite_width, sprite_height), true);
        if transform.is_none() && rotation_degrees.abs() <= f32::EPSILON {
            let rect = GuiRect::from_origin_size(
                GuiPoint::new(
                    screen_x - sprite_width / 2.0,
                    screen_y - sprite_height / 2.0,
                ),
                GuiSize::new(sprite_width, sprite_height),
            );
            draw_image_region_float_source(
                &mut self.surface,
                &rect,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                sampling,
                false,
                owner_color,
                blit,
                gamma,
                fog.as_ref(),
            );
        } else {
            let center = (screen_x / zoom, screen_y / zoom);
            let dest = (
                center.0 - sprite.image.width() as f32 / 2.0,
                center.1 - sprite.image.height() as f32 / 2.0,
                sprite.image.width() as f32,
                sprite.image.height() as f32,
            );
            let mut matrix = draw_transform_at(
                transform
                    .map(|transform| transform.matrix())
                    .unwrap_or(GraphicsTransform::identity().mat),
                center.0,
                center.1,
            );
            if rotation_degrees.abs() > f32::EPSILON {
                matrix = matrix.multiply(&GraphicsTransform::set_rotate(
                    (rotation_degrees * 100.0).round() as i32,
                    center.0,
                    center.1,
                ));
            }
            matrix = matrix.multiply(&GraphicsTransform::set_move_scale(
                0.0, 0.0, zoom, zoom,
            ));
            draw_image_region_transformed_float_source(
                &mut self.surface,
                dest,
                &matrix,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                sampling,
                false,
                owner_color,
                blit,
                gamma,
                fog.as_ref(),
            );
        }
    }

    pub(crate) fn resolve_draw_direction(
        graphics: &DefinitionActionGraphics,
        direction: Direction,
    ) -> (i32, bool) {
        let direction = direction.to_script_value();
        if let Some(flip_dir) = graphics.flip_dir {
            if flip_dir != 0 && direction >= flip_dir {
                return (
                    flip_dir
                        .saturating_mul(2)
                        .saturating_sub(1)
                        .saturating_sub(direction),
                    true,
                );
            }
        }
        (direction, false)
    }

    fn source_within_image(image: &ImageData, rect: &SourceRect) -> bool {
        let width = image.width() as i32;
        let height = image.height() as i32;
        if width <= 0 || height <= 0 {
            return false;
        }
        rect.x >= 0
            && rect.y >= 0
            && rect.width > 0
            && rect.height > 0
            && rect.x + rect.width <= width
            && rect.y + rect.height <= height
    }

    pub(crate) fn cursor_mark_rect(
        screen_x: f32,
        screen_y: f32,
        shape_height: f32,
        cell: i32,
        scale: f32,
    ) -> GuiRect {
        let inverse_scale = scale.recip();
        // C++ casts cox/coy*scale before subtracting the unscaled physical
        // facet offsets; Wdt/2 and Shape.Hgt/2 are integer divisions.
        let x = ((screen_x * scale).trunc() - (cell / 2) as f32) * inverse_scale;
        let y = ((screen_y * scale).trunc() - (shape_height / 2.0).trunc() - cell as f32)
            * inverse_scale;
        let size = cell as f32 * inverse_scale;
        GuiRect::from_origin_size(GuiPoint::new(x, y), GuiSize::new(size, size))
    }

    /// `C4Game::DrawCursors` (src/C4Game.cpp:1852-1874): while a player's
    /// CursorFlash/SelectFlash timer runs, the fctCursor mark — the 35th
    /// square cell of the mouse-cursor sheet, phase +1 when the crew is
    /// contained (src/C4GraphicsResource.cpp:328-336, src/C4Game.cpp:1868)
    /// — is drawn centered above the cursor clonk's def shape.
    fn draw_player_cursors(
        &mut self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        origin_x: f32,
        origin_y: f32,
        zoom: f32,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let Some(image) = self.cursor_atlas.image_for_scaled_resolution(
            self.logical_resolution_width,
            self.presentation_scale,
        ) else {
            return;
        };

        let player = snapshot.players.iter().find(|player| player.id == owner);
        // `if (pPlr->CursorFlash || pPlr->SelectFlash)` (src/C4Game.cpp:1863).
        if player
            .map(|player| player.control.cursor_flash <= 0 && player.control.select_flash <= 0)
            .unwrap_or(false)
        {
            return;
        }
        let cursor_id = player.and_then(|player| player.cursor).or_else(|| {
            snapshot
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor)
        });
        let Some(cursor_id) = cursor_id else {
            return;
        };
        let Some(object) = snapshot.object(cursor_id) else {
            return;
        };

        // fctCursor: cell size = sheet height; phase 1 while contained.
        let cell = image.height() as i32;
        let phase = i32::from(object.container.is_some());
        let source = SourceRect::new((35 + phase) * cell, 0, cell, cell);
        if !Self::source_within_image(&image, &source) {
            return;
        }

        let content_width = self.surface_width as f32;
        let content_height = self.surface_height as f32;
        let screen_x = (object.position.x as f32 - origin_x) * zoom;
        let screen_y = (object.position.y as f32 - origin_y) * zoom;
        let margin = 16.0;
        if screen_x < -margin
            || screen_y < -margin
            || screen_x > content_width + margin
            || screen_y > content_height + margin
        {
            return;
        }

        // `coy - cursor->Def->Shape.Hgt / 2 - fctCursor.Hgt`
        // (src/C4Game.cpp:1872): offset by the def shape height, not the
        // sprite-sheet image height.
        let (cursor_definition_id, cursor_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        let shape_height = {
            let mut sprite = self.object_sprites.get(&sprite_map_key(
                &cursor_definition_id,
                cursor_graphics_name.as_deref(),
            ));
            if sprite.is_none() && cursor_graphics_name.is_some() {
                sprite = self
                    .object_sprites
                    .get(&sprite_map_key(&cursor_definition_id, None));
            }
            if sprite.is_none() && cursor_definition_id != object.definition_id {
                sprite = self
                    .object_sprites
                    .get(&sprite_map_key(&object.definition_id, None));
            }
            sprite
                .map(|sprite| (Self::sprite_def_shape(sprite).height as f32 * zoom).max(1.0))
                .unwrap_or(12.0 * zoom)
        };
        // DrawT applies an inverse Application.GetScale() transform after
        // choosing a physically sized cursor sheet (src/C4Game.cpp:1859-1880).
        // The frame presenter supplies the matching forward scale later.
        let inverse_scale = self.presentation_scale.recip();
        let fog = self.fog_draw_context();

        let rect = Self::cursor_mark_rect(
            screen_x,
            screen_y,
            shape_height,
            cell,
            self.presentation_scale,
        );
        draw_image_region(
            &mut self.surface,
            &rect,
            &image,
            None,
            &source,
            false,
            None,
            SpriteBlitState::normal().with_renderer_config(self.advanced_renderer_config),
            gamma,
            fog.as_ref(),
        );

        // Cursor name label (src/C4Game.cpp:1873-1887): with cursor->Info,
        // the crew name — prefixed by a `sRankName` line when Rank > 0 —
        // is drawn in FontRegular, red 0xffff0000, centered above the mark
        // (`coy - Shape.Hgt/2 - fctCursor.Hgt - 2 - texthgt`). TextOut
        // splits the C++ "rank|name" on '|' into stacked centered lines
        // (src/StdDDraw2.cpp:1039).
        let label = self
            .hud_players
            .iter()
            .find(|player| player.owner == owner)
            .and_then(|player| player.crew.iter().find(|crew| crew.object_id == cursor_id))
            .and_then(|crew| {
                crew.info_name
                    .as_ref()
                    .map(|name| (name.clone(), crew.rank, crew.rank_name.clone()))
            });
        if let Some((name, rank, rank_name)) = label {
            let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
            let line_height = font.line_height();
            // `texthgt = GetLineHeight(); if (Rank > 0) texthgt += texthgt`
            // (src/C4Game.cpp:1876-1880).
            let lines: Vec<String> = rank_name
                .filter(|_| rank > 0)
                .map(|rank_name| vec![rank_name, name.clone()])
                .unwrap_or_else(|| vec![name]);
            let text_height = line_height * lines.len() as i32;
            let text_x = screen_x.round() as i32;
            // TextOut is not under DrawT's transform. C++ offsets it by the
            // ordinary logical shape height and trunc(fctCursor.Hgt / scale).
            let label_mark_top = screen_y
                - shape_height / 2.0
                - (cell as f32 * inverse_scale).trunc();
            let mut text_y = label_mark_top.round() as i32 - 2 - text_height;
            for line in &lines {
                let color = Color::opaque(0xff, 0x00, 0x00);
                if let Some(fog) = fog.as_ref() {
                    draw_fogged_cursor_text_line(
                        &mut self.surface,
                        &font,
                        text_x,
                        text_y,
                        line,
                        color,
                        gamma,
                        self.advanced_renderer_config,
                        fog,
                    );
                } else {
                    font.draw_with_gamma(
                        &mut self.surface,
                        text_x,
                        text_y,
                        line,
                        color,
                        clonk_graphics::clonk_font::TextAlign::Center,
                        gamma,
                    );
                }
                text_y += line_height;
            }
        }
    }

    /// `Game.GraphicsResource.FontRegular` for HUD text.
    pub(crate) fn hud_font(&self) -> hud::HudFont<'_> {
        hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref())
    }

    /// Current `Game.GraphicsResource.FontRegular.GetLineHeight()` used by
    /// `C4MessageBoard::Init` and `Execute`.
    pub fn message_board_line_height(&self) -> i32 {
        self.hud_font().line_height()
    }

    /// The bottom border occupied by `MessageBoard.Output.Hgt` after the
    /// current `ChangeMode` (src/C4MessageBoard.cpp:65-125,223-241).
    pub(crate) fn message_board_height(&self) -> i32 {
        self.message_board
            .output_height(self.message_board_line_height())
    }

    /// Whether the fullscreen chrome (upper board + message board) is
    /// active. C++ only sets the boards up when their Graphics.c4g facets
    /// loaded (`C4UpperBoard::Init` bails without `fctUpperBoard.Surface`,
    /// src/C4UpperBoard.cpp:114-118); asset-less test setups render bare
    /// viewports.
    fn hud_chrome_active(&self) -> bool {
        self.hud_graphics.upper_board.is_some()
    }

    /// The drawn top-board raster height. Small uses half the texture height;
    /// Hide/Mini draw no top board (src/C4UpperBoard.cpp:38-99).
    fn upper_board_pixel_height(&self) -> i32 {
        hud::upper_board_output_height(self.upper_board_mode, &self.hud_graphics)
    }

    /// `C4UpperBoard::Output`, including Mini's bottom-right message strip.
    pub(crate) fn upper_board_output_rect(&self) -> Option<SurfaceRect> {
        let surface_width = self.surface_width as i32;
        let surface_height = self.surface_height as i32;
        match self.upper_board_mode {
            // Hide has zero reserved/drawn height, but Init still takes the
            // non-Mini branch and expands its raw Output facet to the full
            // upper-board texture height. Execute alone suppresses drawing.
            hud::UpperBoardMode::Hide => {
                let height = self
                    .hud_graphics
                    .upper_board
                    .as_ref()
                    .map_or(0, |board| board.height() as i32)
                    .clamp(0, surface_height) as u32;
                (height > 0).then(|| SurfaceRect::new(0, 0, self.surface_width, height))
            }
            hud::UpperBoardMode::Full | hud::UpperBoardMode::Small => {
                let height = self
                    .upper_board_pixel_height()
                    .clamp(0, surface_height) as u32;
                (height > 0).then(|| SurfaceRect::new(0, 0, self.surface_width, height))
            }
            hud::UpperBoardMode::Mini => {
                let width = hud::upper_board_text_strip_width_for_text_width(
                    self.initialized_upper_board_text_width(),
                )
                .clamp(0, surface_width);
                let height = self.message_board_height().clamp(0, surface_height);
                (width > 0 && height > 0).then(|| {
                    SurfaceRect::new(
                        surface_width - width,
                        surface_height - height,
                        width as u32,
                        height as u32,
                    )
                })
            }
        }
    }

    /// `C4MessageBoard::Output`, shortened on the right by Mini mode.
    pub(crate) fn message_board_output_rect(&self) -> Option<SurfaceRect> {
        let width = hud::message_board_available_width_for_text_width(
            self.surface_width as i32,
            self.upper_board_mode,
            self.initialized_upper_board_text_width(),
        );
        let height = self
            .message_board_height()
            .clamp(0, self.surface_height as i32);
        (width > 0 && height > 0).then(|| {
            SurfaceRect::new(
                0,
                self.surface_height as i32 - height,
                width as u32,
                height as u32,
            )
        })
    }

    /// Per-viewport player HUD, which precedes the fullscreen boards in
    /// C4GraphicsSystem::Execute (src/C4GraphicsSystem.cpp:352-365).
    fn draw_hud_players(&mut self, frame: u64, gamma: Option<&clonk_graphics::GammaRamp>) {
        if !self.viewport_overlays_visible {
            return;
        }
        // Per-viewport player info (C4Viewport::DrawOverlay,
        // src/C4Viewport.cpp:835-848).
        let viewports = self.active_viewports.clone();
        for viewport in &viewports {
            let Some(player) = self
                .hud_players
                .iter()
                .find(|player| player.owner == viewport.owner)
            else {
                continue;
            };
            let player = player.clone();
            let rect = viewport.rect;
            let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());

            // Cursor info: C++ draws nothing without a cursor crew member
            // (src/C4Viewport.cpp:891-897) — the faithful "no crew"
            // presentation is an empty corner.
            let cursor_crew = player
                .cursor
                .and_then(|id| player.crew.iter().find(|crew| crew.object_id == id));
            if let Some(crew) = cursor_crew {
                // `cursor->Info` gates only C4ObjectInfo::Draw. Inventory
                // and bars below remain visible for a non-crew ViewCursor.
                if crew.info_name.is_some() {
                    hud::draw_cursor_info_with_gamma(
                        &mut self.surface,
                        &font,
                        &self.hud_graphics,
                        rect,
                        &crew.label,
                        crew.rank,
                        crew.rank_name.as_deref(),
                        crew.portrait.as_ref(),
                        crew.portrait_owner_overlay.as_ref(),
                        crew.portrait_owner_color,
                        crew.rank_symbols.as_ref(),
                        crew.rank_symbol_count,
                        player.captain == Some(crew.object_id),
                        crew.hide_hud_elements,
                        gamma,
                    );
                }
                if crew.hide_hud_elements & clonk_engine::HIDE_HUD_ELEMENT_INVENTORY == 0 {
                    hud::draw_inventory_with_gamma(
                        &mut self.surface,
                        &font,
                        rect,
                        &crew.inventory,
                        gamma,
                    );
                }
                let mut bar_slot = 0;
                if crew.hide_hud_bars & clonk_engine::HIDE_HUD_BAR_ENERGY == 0 {
                    hud::draw_level_bar_with_gamma(
                        &mut self.surface,
                        &self.hud_graphics,
                        rect,
                        hud::HudBarKind::Energy,
                        0,
                        crew.energy,
                        crew.energy_capacity,
                        self.show_portraits,
                        gamma,
                    );
                    bar_slot += 1;
                }
                if crew.magic_energy != 0
                    && crew.hide_hud_bars & clonk_engine::HIDE_HUD_BAR_MAGIC_ENERGY == 0
                {
                    hud::draw_level_bar_with_gamma(
                        &mut self.surface,
                        &self.hud_graphics,
                        rect,
                        hud::HudBarKind::Magic,
                        bar_slot,
                        crew.magic_energy / MAGIC_PHYSICAL_FACTOR,
                        crew.magic_capacity / MAGIC_PHYSICAL_FACTOR,
                        self.show_portraits,
                        gamma,
                    );
                    bar_slot += 1;
                }
                if crew.breath != 0
                    && crew.breath < crew.breath_capacity
                    && crew.hide_hud_bars & clonk_engine::HIDE_HUD_BAR_BREATH == 0
                {
                    hud::draw_level_bar_with_gamma(
                        &mut self.surface,
                        &self.hud_graphics,
                        rect,
                        hud::HudBarKind::Breath,
                        bar_slot,
                        crew.breath,
                        crew.breath_capacity,
                        self.show_portraits,
                        gamma,
                    );
                }
            }

            // Command rows (src/C4Viewport.cpp:947-961), gated on
            // Config.Graphics.ShowCommands; 23px key caps pick FontTiny
            // (`cgo.Hgt <= C4MN_SymbolSize`, src/C4ObjectCom.cpp:940).
            if self.show_commands && !player.commands.is_empty() {
                let tiny = self
                    .clonk_fonts
                    .as_deref()
                    .map(|set| hud::HudFont::Clonk(&set.mini))
                    .unwrap_or(hud::HudFont::Fallback(self.font.as_ref()));
                hud::draw_commands_with_gamma(
                    &mut self.surface,
                    &tiny,
                    &self.hud_graphics,
                    rect,
                    &player.commands,
                    self.show_command_keys,
                    player.flash_command,
                    frame,
                    gamma,
                );
            }

            let (show_wealth, show_score, show_crew) = player_fixed_item_visibility(
                self.show_player_hud_always,
                player.view_wealth,
                player.view_value,
            );
            let crew_icon = if show_crew {
                self.hud_graphics.crew.as_ref().map(|source| {
                    let key = (source.gpu_texture_id(), player.owner_color);
                    self.owner_colored_crew_icons
                        .entry(key)
                        .or_insert_with(|| hud::colorize_by_owner(source, player.owner_color))
                        .clone()
                })
            } else {
                None
            };
            hud::draw_player_fixed_items_with_colored_crew_gamma(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                rect,
                player.wealth,
                player.score,
                player.select_count,
                player.crew_count,
                crew_icon.as_ref(),
                show_wealth,
                show_score,
                show_crew,
                gamma,
            );

            let tiny = self
                .clonk_fonts
                .as_deref()
                .map(|set| hud::HudFont::Clonk(&set.mini))
                .unwrap_or(hud::HudFont::Fallback(self.font.as_ref()));
            hud::draw_player_controls_with_gamma(
                &mut self.surface,
                &font,
                &tiny,
                &self.hud_graphics,
                rect,
                player.show_control,
                player.show_control_position,
                player.last_com,
                &player.control_key_labels,
                frame,
                gamma,
            );

            if player.show_startup {
                let player_name = c4_presentation_text(&player.name);
                hud::draw_player_startup_with_gamma(
                    &mut self.surface,
                    &font,
                    &self.hud_graphics,
                    rect,
                    &player_name,
                    player.owner_color,
                    player.control_set,
                    player.mouse_control,
                    gamma,
                );
            }
        }
    }

    /// `C4Network2::DrawStatus` is a per-viewport, pipe-delimited FontRegular
    /// overlay at (+20,+50), after the player's viewport overlay.
    pub fn draw_network_status(&mut self, gamma: Option<&clonk_graphics::GammaRamp>) -> bool {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        if !self.debug_draw_flags.show_net_status {
            return false;
        }
        let Some(text) = self.network_status_text.clone() else {
            return false;
        };
        let viewports = self.active_viewports.clone();
        if viewports.is_empty() {
            return false;
        }
        let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
        for viewport in viewports {
            let previous_clip = self.surface.clip();
            let clip = previous_clip
                .and_then(|clip| clip.intersection(viewport.rect))
                .unwrap_or_else(|| {
                    if previous_clip.is_some() {
                        SurfaceRect::new(0, 0, 0, 0)
                    } else {
                        viewport.rect
                    }
                });
            self.surface.set_clip(clip);
            font.draw_markup_with_gamma(
                &mut self.surface,
                viewport.rect.x + 20,
                viewport.rect.y + 50,
                &text,
                Color::opaque(255, 255, 255),
                clonk_graphics::clonk_font::TextAlign::Left,
                gamma,
            );
            match previous_clip {
                Some(clip) => self.surface.set_clip(clip),
                None => self.surface.clear_clip(),
            }
        }
        true
    }

    /// Draw the final per-viewport Help/PlayerMenu/Chat controls after
    /// menus and game messages, immediately before the mouse presentation
    /// (`C4Viewport::DrawOverlay`, src/C4Viewport.cpp:863-880).
    pub fn draw_viewport_control_overlays(
        &mut self,
        mouse_viewport_index: Option<usize>,
        chat_active: bool,
        gui_icons2: Option<&ImageData>,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let _renderer_config = activate_advanced_renderer_config(self.advanced_renderer_config);
        if !self.viewport_overlays_visible {
            return;
        }
        let viewports = self.active_viewports.clone();
        for (viewport_index, viewport) in viewports.iter().enumerate() {
            let player = self
                .hud_players
                .iter()
                .find(|player| player.owner == viewport.owner);
            let menu_key_label = player
                .and_then(|player| player.control_key_labels.get(9))
                .map(String::as_str)
                .unwrap_or("");
            // DrawCommandKey selects FontRegular when the 23px target cell
            // is larger than C4MN_SymbolSize (16).
            let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
            hud::draw_viewport_buttons_with_gamma(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                gui_icons2,
                viewport.rect,
                self.show_commands,
                self.show_command_keys,
                mouse_viewport_index == Some(viewport_index),
                chat_active,
                menu_key_label,
                gamma,
            );
        }
    }

    /// Message board, upper board and optional debug text. Keeping this phase
    /// separate lets its raster chrome occlude earlier scale-native HUD text.
    fn draw_hud_chrome(&mut self, gamma: Option<&clonk_graphics::GammaRamp>) {
        let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
        if self.hud_chrome_active() {
            let text_width = self.initialized_upper_board_text_width();
            hud::draw_message_board_with_gamma(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                &self.message_board,
                gamma,
            );
            hud::draw_upper_board_with_initialized_text_width(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                self.upper_board_mode,
                &self.scenario_label_text,
                self.game_time_seconds,
                text_width,
                self.clock_text.as_deref(),
                self.frames_per_second,
                gamma,
            );
        }

        // Opt-in debug lines (replaces the old debug bar; off by default).
        if let Some((frame_text, status_text)) = self.debug_hud_text.clone() {
            let line_height = font.line_height();
            let base_y = if self.hud_chrome_active() {
                self.upper_board_pixel_height() + 2
            } else {
                2
            };
            font.draw_with_gamma(
                &mut self.surface,
                hud::SYMBOL_BORDER,
                base_y,
                &frame_text,
                Color::opaque(255, 255, 255),
                clonk_graphics::clonk_font::TextAlign::Left,
                gamma,
            );
            font.draw_with_gamma(
                &mut self.surface,
                hud::SYMBOL_BORDER,
                base_y + line_height,
                &status_text,
                Color::opaque(255, 255, 255),
                clonk_graphics::clonk_font::TextAlign::Left,
                gamma,
            );
        }
    }

    #[cfg(test)]
    pub fn viewport(&self) -> (i32, i32) {
        self.active_viewports
            .first()
            .map(|viewport| {
                let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);
                let offset_x = viewport.content_rect.x as f32 / zoom;
                let offset_y = viewport.content_rect.y as f32 / zoom;
                let adjusted_x = (viewport.viewport_x - offset_x).max(0.0);
                let adjusted_y = (viewport.viewport_y - offset_y).max(0.0);
                (adjusted_x.round() as i32, adjusted_y.round() as i32)
            })
            .unwrap_or((
                self.viewport_x.round() as i32,
                self.viewport_y.round() as i32,
            ))
    }

    fn surface_height_at(&self, landscape: Option<&Landscape>, x: i32) -> Option<i32> {
        landscape.and_then(|landscape| landscape.surface_height(x))
    }

    pub(crate) fn lighting_factor(time_of_day: u16) -> f32 {
        // C++ CR has no ambient day/night dimming for standard scenarios
        // (C4Weather adjusts the SKY gamma only) — an unset time-of-day
        // must render at full brightness, not as midnight. The cycle
        // stays for sandbox worlds that drive the clock.
        if time_of_day == 0 {
            return 1.0;
        }
        let cycle = EnvironmentSettings::TIME_CYCLE as f32;
        if cycle <= 0.0 {
            return 1.0;
        }
        let half_cycle = cycle / 2.0;
        let time = (time_of_day as u32 % EnvironmentSettings::TIME_CYCLE as u32) as f32;
        let mut distance = (time - half_cycle).abs();
        if distance > half_cycle {
            distance = cycle - distance;
        }
        let normalized = 1.0 - distance / half_cycle;
        let normalized = normalized.clamp(0.0, 1.0);
        let min = 0.35f32;
        let max = 1.0f32;
        min + normalized * (max - min)
    }

    pub(crate) fn apply_lighting(color: Color, lighting: f32) -> Color {
        color.modulate(lighting)
    }

    fn collect_owner_colors(snapshot: &SimulationSnapshot) -> HashMap<i32, Color> {
        let mut colors: HashMap<i32, Color> = HashMap::new();
        for player in &snapshot.players {
            if let Some(rgb) = player.color {
                colors.insert(player.id, Color::opaque(rgb.r, rgb.g, rgb.b));
            }
        }

        let mut owners: HashSet<i32> = snapshot.players.iter().map(|state| state.id).collect();
        owners.extend(snapshot.known_crew_owners.iter().copied());
        owners.extend(snapshot.eliminated_crew_owners.iter().copied());
        for object in &snapshot.objects {
            if object.owner != OWNER_NONE {
                owners.insert(object.owner);
            }
        }

        for owner in owners {
            if owner == OWNER_NONE {
                continue;
            }
            colors
                .entry(owner)
                .or_insert_with(|| default_owner_color(owner));
        }

        colors
    }

    fn collect_sprite_atlas(&self, snapshot: &SimulationSnapshot) -> Vec<EngineSurfaceSnapshot> {
        let mut atlas = Vec::with_capacity(
            3 + snapshot
                .objects
                .len()
                .saturating_add(self.active_viewports.len()),
        );

        let full_snapshot = self.surface.snapshot();
        atlas.push(Self::make_engine_surface(
            "back_buffer".to_string(),
            full_snapshot,
        ));

        for (index, viewport) in self.active_viewports.iter().enumerate() {
            if let Some(region) = self.surface.snapshot_region(viewport.rect) {
                let owner_label = if viewport.owner < 0 {
                    "none".to_string()
                } else {
                    viewport.owner.to_string()
                };
                let label = format!("viewport#{index}:player={owner_label}");
                atlas.push(Self::make_engine_surface(label, region));
            }
        }

        if self.hud_chrome_active() {
            if let Some(snapshot) = self
                .upper_board_output_rect()
                .and_then(|rect| self.surface.snapshot_region(rect))
            {
                atlas.push(Self::make_engine_surface(
                    "upper_board".to_string(),
                    snapshot,
                ));
            }
            if let Some(snapshot) = self
                .message_board_output_rect()
                .and_then(|rect| self.surface.snapshot_region(rect))
            {
                atlas.push(Self::make_engine_surface(
                    "message_board".to_string(),
                    snapshot,
                ));
            }
        }

        for viewport in &self.active_viewports {
            if let Some(object) = viewport.focus.and_then(|focus| snapshot.object(focus)) {
                if let Some(rect) = self.object_screen_rect_for_viewport(object, viewport) {
                    if let Some(snap) = self.surface.snapshot_region(rect) {
                        let label =
                            format!("focus#{}:player={}", object.id.as_u64(), viewport.owner);
                        atlas.push(Self::make_engine_surface(label, snap));
                    }
                }
            }
        }

        for object in &snapshot.objects {
            if let Some(rect) = self
                .active_viewports
                .iter()
                .find_map(|viewport| self.object_screen_rect_for_viewport(object, viewport))
            {
                if let Some(snap) = self.surface.snapshot_region(rect) {
                    let label =
                        format!("object#{}:def={}", object.id.as_u64(), object.definition_id);
                    atlas.push(Self::make_engine_surface(label, snap));
                }
            }
        }

        atlas
    }

    fn make_engine_surface(
        label: String,
        snapshot: GraphicsSurfaceSnapshot,
    ) -> EngineSurfaceSnapshot {
        let width = i32::try_from(snapshot.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(snapshot.height()).unwrap_or(i32::MAX);
        EngineSurfaceSnapshot {
            label,
            width,
            height,
            hash: u64::from(snapshot.checksum()),
        }
    }

    fn object_screen_rect_for_viewport(
        &self,
        object: &ObjectSnapshot,
        viewport: &ActiveViewport,
    ) -> Option<SurfaceRect> {
        if !object.status.is_active() || !object.alive {
            return None;
        }

        let base_x = viewport.content_rect.x as f32;
        let base_y = viewport.content_rect.y as f32;
        let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);

        if object.vertices.is_empty() {
            let screen_x = (object.position.x as f32 - viewport.viewport_x) * zoom + base_x;
            let screen_y = (object.position.y as f32 - viewport.viewport_y) * zoom + base_y;
            let size = (6.0 * zoom).max(3.0);
            let half = size / 2.0;
            let rect = SurfaceRect::new(
                (screen_x - half).floor() as i32,
                (screen_y - half).floor() as i32,
                size.ceil() as u32,
                size.ceil() as u32,
            );
            return rect.intersection(viewport.rect);
        }

        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for vertex in &object.vertices {
            let world_x = (object.position.x + vertex.x) as f32;
            let world_y = (object.position.y + vertex.y) as f32;
            let x = (world_x - viewport.viewport_x) * zoom + base_x;
            let y = (world_y - viewport.viewport_y) * zoom + base_y;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }

        if !(min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite()) {
            return None;
        }
        if min_x > max_x || min_y > max_y {
            return None;
        }

        let padding = (2.0 * zoom).max(1.0);
        let left = (min_x - padding).floor() as i32;
        let top = (min_y - padding).floor() as i32;
        let right = (max_x + padding).ceil() as i32;
        let bottom = (max_y + padding).ceil() as i32;

        if right < left || bottom < top {
            return None;
        }

        let width = (right - left + 1).max(1) as u32;
        let height = (bottom - top + 1).max(1) as u32;
        SurfaceRect::new(left, top, width, height).intersection(viewport.rect)
    }

    /// C4Game::FindVisObject point searches the current C4Shape rectangle,
    /// including structures and carryables whose `Alive` flag is false
    /// (C4Game.cpp:1469-1476). This intentionally differs from the
    /// crew-focused atlas/selection bounds above.
    fn object_pick_rect_for_viewport(
        &self,
        object: &ObjectSnapshot,
        viewport: &ActiveViewport,
    ) -> Option<SurfaceRect> {
        let base_x = viewport.content_rect.x as f32;
        let base_y = viewport.content_rect.y as f32;
        let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);
        let shape = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
            .map(|sprite| {
                Self::con_scaled_shape(
                    Self::sprite_def_shape(sprite),
                    object.construction.clamp(0, FULL_CON),
                    sprite.stretch_growth,
                )
            })
            .filter(|shape| shape.width > 0 && shape.height > 0);

        let (world_left, world_top, world_right, world_bottom) = if let Some(shape) = shape {
            (
                object.position.x + shape.x,
                object.position.y + shape.y,
                object.position.x + shape.x + shape.width,
                object.position.y + shape.y + shape.height,
            )
        } else if object.vertices.is_empty() {
            (
                object.position.x - 3,
                object.position.y - 3,
                object.position.x + 3,
                object.position.y + 3,
            )
        } else {
            let min_x = object.vertices.iter().map(|vertex| vertex.x).min()?;
            let max_x = object.vertices.iter().map(|vertex| vertex.x).max()?;
            let min_y = object.vertices.iter().map(|vertex| vertex.y).min()?;
            let max_y = object.vertices.iter().map(|vertex| vertex.y).max()?;
            (
                object.position.x + min_x,
                object.position.y + min_y,
                object.position.x + max_x + 1,
                object.position.y + max_y + 1,
            )
        };

        let left = ((world_left as f32 - viewport.viewport_x) * zoom + base_x).floor() as i32;
        let top = ((world_top as f32 - viewport.viewport_y) * zoom + base_y).floor() as i32;
        let right = ((world_right as f32 - viewport.viewport_x) * zoom + base_x).ceil() as i32;
        let bottom = ((world_bottom as f32 - viewport.viewport_y) * zoom + base_y).ceil() as i32;
        let width = (right - left).max(1) as u32;
        let height = (bottom - top).max(1) as u32;
        SurfaceRect::new(left, top, width, height).intersection(viewport.rect)
    }

    pub(crate) fn sky_color_for_temperature(temperature: i32) -> Color {
        let factor = Self::temperature_factor(temperature);
        let cold = (10, 16, 32);
        let warm = (84, 52, 16);
        Color::opaque(
            Self::blend_channel(cold.0, warm.0, factor),
            Self::blend_channel(cold.1, warm.1, factor),
            Self::blend_channel(cold.2, warm.2, factor),
        )
    }

    pub(crate) fn ground_color_for_temperature(temperature: i32) -> Color {
        let factor = Self::temperature_factor(temperature);
        let cold = (28, 84, 44);
        let warm = (108, 90, 32);
        Color::opaque(
            Self::blend_channel(cold.0, warm.0, factor),
            Self::blend_channel(cold.1, warm.1, factor),
            Self::blend_channel(cold.2, warm.2, factor),
        )
    }

    pub(crate) fn liquid_color_for_temperature(temperature: i32) -> Color {
        let factor = Self::temperature_factor(temperature);
        let cold = (36, 112, 200);
        let warm = (48, 132, 160);
        Color::new(
            Self::blend_channel(cold.0, warm.0, factor),
            Self::blend_channel(cold.1, warm.1, factor),
            Self::blend_channel(cold.2, warm.2, factor),
            192,
        )
    }

    fn temperature_factor(temperature: i32) -> f32 {
        let clamped = temperature.clamp(-50, 50);
        (clamped as f32 + 50.0) / 100.0
    }

    fn blend_channel(cold: u8, warm: u8, factor: f32) -> u8 {
        let factor = factor.clamp(0.0, 1.0);
        let cold = cold as f32;
        let warm = warm as f32;
        let value = cold + (warm - cold) * factor;
        value.round().clamp(0.0, 255.0) as u8
    }
}
